//! A console, as the processes attached to it have it: the handles `GetStdHandle` gives, `CONIN$` and `CONOUT$`.
//! Where its input comes from and its output goes is given ([`Stdio::host`] for the host's own stdin and stdout, or
//! another, as `carbon exec` gives one).
//!
//! It's either a console (`docker run -t`: `GetConsoleMode` works, and input is read a line at a time, as conhost's
//! cooked mode gives it), or pipes (without `-t`: `GetConsoleMode` fails and `GetFileType` says a pipe, so programs
//! read and write it as a file, as they do a redirected one). A console on a terminal in raw mode does conhost's line
//! editing itself: echo, Backspace, the arrows and history, Home, End, Delete, Esc to clear, Ctrl+C, and Ctrl+Z then
//! Enter for the end.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use host::{Host, Stream};
use kernels_core::lock;

/// `ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_INSERT_MODE | ENABLE_QUICK_EDIT_MODE |
/// ENABLE_EXTENDED_FLAGS | ENABLE_AUTO_POSITION`, conhost's for input.
pub const INPUT_MODE: u32 = 0x1f7;
pub const LINE_INPUT: u32 = 0x2;
pub const ECHO_INPUT: u32 = 0x4;
/// `ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT`.
pub const OUTPUT_MODE: u32 = 0x3;

type Read = Box<dyn Fn(&mut [u8]) -> usize + Send + Sync>;
type Write = Box<dyn Fn(Stream, &[u8]) + Send + Sync>;

pub struct Stdio {
    /// A console, rather than pipes.
    pub console: bool,
    /// It edits lines itself: its terminal is raw.
    edit: bool,
    read: Read,
    write: Write,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// What's been read and not taken: a line already finished, or raw input not yet edited.
    lines: VecDeque<u8>,
    raw: VecDeque<u8>,
    history: Vec<String>,
    input_mode: Option<u32>,
    output_mode: Option<u32>,
    ended: bool,
    /// The last byte written was `\r`, for turning a bare `\n` into `\r\n` on a raw terminal.
    after_cr: bool,
}

impl Stdio {
    pub fn new(console: bool, edit: bool, read: Read, write: Write) -> Arc<Stdio> {
        Arc::new(Stdio { console, edit: console && edit, read, write, state: Mutex::new(State::default()) })
    }

    /// The host's stdin and stdout (and stderr): a console that edits its own lines if `console` and the host's is a
    /// terminal, which it then puts in raw mode for as long as this is kept; else pipes.
    pub fn host(host: Arc<dyn Host>, console: bool) -> Arc<Stdio> {
        let edit = console && host.is_terminal();
        if edit {
            host.set_raw(true);
        }
        let (r, w) = (host.clone(), host);
        Stdio::new(console, edit, Box::new(move |buffer| r.read(buffer)), Box::new(move |stream, data| w.write(stream, data)))
    }

    /// Whether it edits lines itself, its terminal raw.
    pub fn edits(&self) -> bool {
        self.edit
    }

    pub fn input_mode(&self) -> u32 {
        lock(&self.state).input_mode.unwrap_or(INPUT_MODE)
    }

    pub fn output_mode(&self) -> u32 {
        lock(&self.state).output_mode.unwrap_or(OUTPUT_MODE)
    }

    pub fn set_mode(&self, input: bool, mode: u32) {
        let mut state = lock(&self.state);
        if input {
            state.input_mode = Some(mode);
        } else {
            state.output_mode = Some(mode);
        }
    }

    /// Output, to its stdout or stderr: on a raw terminal, a bare `\n` as `\r\n`.
    pub fn write(&self, stream: Stream, data: &[u8]) {
        if !self.edit {
            (self.write)(stream, data);
            return;
        }
        let mut out = Vec::with_capacity(data.len() + 8);
        let mut after_cr = lock(&self.state).after_cr;
        for &b in data {
            if b == b'\n' && !after_cr {
                out.push(b'\r');
            }
            out.push(b);
            after_cr = b == b'\r';
        }
        lock(&self.state).after_cr = after_cr;
        (self.write)(stream, &out);
    }

    fn echo(&self, text: &str) {
        (self.write)(Stream::Out, text.as_bytes());
    }

    /// Reads more from where its input comes from, into `raw`; false at its end.
    fn fill(&self) -> bool {
        if lock(&self.state).ended {
            return false;
        }
        let mut buffer = [0u8; 4096];
        let n = (self.read)(&mut buffer);
        let mut state = lock(&self.state);
        if n == 0 {
            state.ended = true;
            return false;
        }
        state.raw.extend(&buffer[..n]);
        true
    }

    /// A read of its input, up to `len` bytes: for pipes, what there is; for a console in line mode, a line at a time
    /// (with its `\r\n`), edited here if its terminal is raw; with line mode off, the keys as they come. Empty at the
    /// end.
    pub fn read(&self, len: usize) -> Vec<u8> {
        let len = len.max(1);
        let line_mode = self.console && self.input_mode() & LINE_INPUT != 0;
        if !line_mode {
            {
                let mut state = lock(&self.state);
                if !state.lines.is_empty() {
                    let n = len.min(state.lines.len());
                    return state.lines.drain(..n).collect();
                }
            }
            if lock(&self.state).raw.is_empty() && !self.fill() {
                return Vec::new();
            }
            let mut state = lock(&self.state);
            let n = len.min(state.raw.len());
            return state.raw.drain(..n).collect();
        }
        loop {
            {
                let mut state = lock(&self.state);
                if !state.lines.is_empty() {
                    let n = len.min(state.lines.len());
                    return state.lines.drain(..n).collect();
                }
            }
            let line = if self.edit { self.edit_line() } else { self.cooked_line() };
            match line {
                Some(line) => lock(&self.state).lines.extend(line),
                None => return Vec::new(),
            }
        }
    }

    /// A line from input that's edited already (a cooked terminal, or a pipe given a console): up to its `\n`, which
    /// becomes `\r\n`.
    fn cooked_line(&self) -> Option<Vec<u8>> {
        loop {
            {
                let mut state = lock(&self.state);
                if let Some(at) = state.raw.iter().position(|&b| b == b'\n') {
                    let mut line: Vec<u8> = state.raw.drain(..=at).collect();
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    line.extend(b"\r\n");
                    return Some(line);
                }
                if state.ended {
                    if state.raw.is_empty() {
                        return None;
                    }
                    return Some(state.raw.drain(..).collect());
                }
            }
            if !self.fill() && lock(&self.state).raw.is_empty() {
                return None;
            }
        }
    }

    /// The next byte typed, waiting for it; None at the end.
    fn key(&self) -> Option<u8> {
        loop {
            if let Some(b) = lock(&self.state).raw.pop_front() {
                return Some(b);
            }
            if !self.fill() {
                return None;
            }
        }
    }

    /// A byte that's already arrived, for the rest of an escape sequence; None if nothing's waiting.
    fn waiting(&self) -> Option<u8> {
        lock(&self.state).raw.pop_front()
    }

    /// conhost's line editing, on a raw terminal: a line as typed, with its `\r\n`; None at the end (Ctrl+Z, Enter).
    fn edit_line(&self) -> Option<Vec<u8>> {
        let echo = self.input_mode() & ECHO_INPUT != 0;
        let mut line: Vec<char> = Vec::new();
        let mut cursor = 0usize;
        let mut recalled = lock(&self.state).history.len();
        let mut pending_utf8: Vec<u8> = Vec::new();
        // Draws the line from the cursor on, then puts the cursor back.
        let redraw_from = |line: &[char], cursor: usize, clear: usize| {
            if !echo {
                return;
            }
            let rest: String = line[cursor..].iter().collect();
            let mut s = rest.clone();
            s.push_str(&" ".repeat(clear));
            s.push_str(&"\x08".repeat(rest.chars().count() + clear));
            self.echo(&s);
        };
        let replace = |line: &mut Vec<char>, cursor: &mut usize, with: &str| {
            if echo {
                let mut s = "\x08".repeat(*cursor);
                let new: Vec<char> = with.chars().collect();
                s.extend(new.iter());
                let extra = line.len().saturating_sub(new.len());
                s.push_str(&" ".repeat(extra));
                s.push_str(&"\x08".repeat(extra));
                self.echo(&s);
            }
            *line = with.chars().collect();
            *cursor = line.len();
        };
        loop {
            let b = self.key()?;
            if !pending_utf8.is_empty() || b >= 0x80 {
                pending_utf8.push(b);
                if let Ok(s) = std::str::from_utf8(&pending_utf8) {
                    for c in s.chars() {
                        line.insert(cursor, c);
                        cursor += 1;
                        if echo {
                            self.echo(&c.to_string());
                        }
                        redraw_from(&line, cursor, 0);
                    }
                    pending_utf8.clear();
                } else if pending_utf8.len() >= 4 {
                    pending_utf8.clear();
                }
                continue;
            }
            match b {
                b'\r' | b'\n' => {
                    if echo {
                        self.echo("\r\n");
                    }
                    let text: String = line.iter().collect();
                    if text == "\x1a" || text.starts_with('\x1a') {
                        return None;
                    }
                    if !text.is_empty() {
                        let mut state = lock(&self.state);
                        if state.history.last() != Some(&text) {
                            state.history.push(text.clone());
                        }
                    }
                    let mut bytes = text.into_bytes();
                    bytes.extend(b"\r\n");
                    return Some(bytes);
                }
                // Backspace.
                0x7f | 0x08 => {
                    if cursor > 0 {
                        cursor -= 1;
                        line.remove(cursor);
                        if echo {
                            self.echo("\x08");
                        }
                        redraw_from(&line, cursor, 1);
                    }
                }
                // Ctrl+C: the line is dropped, as conhost does, and an empty one read.
                0x03 => {
                    if echo {
                        self.echo("^C\r\n");
                    }
                    return Some(b"\r\n".to_vec());
                }
                // Ctrl+Z: shown, and as the line's first character, the end.
                0x1a => {
                    line.insert(cursor, '\x1a');
                    cursor += 1;
                    if echo {
                        self.echo("^Z");
                    }
                }
                0x1b => {
                    // An escape sequence, or Esc alone, which clears the line.
                    match self.waiting() {
                        Some(b'[') | Some(b'O') => {
                            let mut seq = Vec::new();
                            while let Some(c) = self.waiting() {
                                seq.push(c);
                                if c.is_ascii_alphabetic() || c == b'~' {
                                    break;
                                }
                            }
                            match seq.as_slice() {
                                b"A" => {
                                    let history = lock(&self.state).history.clone();
                                    if recalled > 0 {
                                        recalled -= 1;
                                        replace(&mut line, &mut cursor, &history[recalled]);
                                    }
                                }
                                b"B" => {
                                    let history = lock(&self.state).history.clone();
                                    if recalled + 1 < history.len() {
                                        recalled += 1;
                                        replace(&mut line, &mut cursor, &history[recalled]);
                                    } else {
                                        recalled = history.len();
                                        replace(&mut line, &mut cursor, "");
                                    }
                                }
                                b"C" if cursor < line.len() => {
                                    if echo {
                                        self.echo(&line[cursor].to_string());
                                    }
                                    cursor += 1;
                                }
                                b"D" if cursor > 0 => {
                                    cursor -= 1;
                                    if echo {
                                        self.echo("\x08");
                                    }
                                }
                                b"H" | b"1~" | b"7~" => {
                                    if echo {
                                        self.echo(&"\x08".repeat(cursor));
                                    }
                                    cursor = 0;
                                }
                                b"F" | b"4~" | b"8~" => {
                                    if echo {
                                        let rest: String = line[cursor..].iter().collect();
                                        self.echo(&rest);
                                    }
                                    cursor = line.len();
                                }
                                b"3~" if cursor < line.len() => {
                                    line.remove(cursor);
                                    redraw_from(&line, cursor, 1);
                                }
                                _ => {}
                            }
                        }
                        Some(other) => lock(&self.state).raw.push_front(other),
                        None => replace(&mut line, &mut cursor, ""),
                    }
                }
                // Tab: conhost would complete a file's name; it's a character here.
                b'\t' => {}
                c if c >= 0x20 => {
                    line.insert(cursor, c as char);
                    cursor += 1;
                    if echo {
                        self.echo(&(c as char).to_string());
                    }
                    redraw_from(&line, cursor, 0);
                }
                _ => {}
            }
        }
    }
}
