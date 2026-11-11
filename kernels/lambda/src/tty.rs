//! Terminals: one line discipline for the console (the host's terminal, in raw mode, as the machine's) and for
//! pseudo-terminal pairs (`/dev/ptmx` and `/dev/pts/N`). It edits lines, echoes, and turns ^C, ^\ and ^Z into signals
//! for the terminal's foreground process group, which the caller sends.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use host::Host;
use kernels_core::lock;
use kernels_core::wait::{Waiter, Watchers, Woke};

use crate::abi::*;
use crate::signal::{SIGINT, SIGQUIT, SIGTSTP, SIGWINCH};

// termios's flags.
const ICRNL: u32 = 0o400;
const INLCR: u32 = 0o100;
const IGNCR: u32 = 0o200;
const IXON: u32 = 0o2000;
const IUTF8: u32 = 0o40000;
const OPOST: u32 = 1;
const ONLCR: u32 = 4;
const ISIG: u32 = 1;
const ICANON: u32 = 2;
const ECHO: u32 = 0o10;
const ECHOE: u32 = 0o20;
const ECHOK: u32 = 0o40;
const ECHONL: u32 = 0o100;
const NOFLSH: u32 = 0o200;
const ECHOCTL: u32 = 0o1000;
const ECHOKE: u32 = 0o4000;
const IEXTEN: u32 = 0o100000;
// Where each control character is.
const VINTR: usize = 0;
const VQUIT: usize = 1;
const VERASE: usize = 2;
const VKILL: usize = 3;
const VEOF: usize = 4;
const VTIME: usize = 5;
const VMIN: usize = 6;
const VSUSP: usize = 10;
const VEOL: usize = 11;
const VWERASE: usize = 14;

/// The kernel's `struct termios`, the same on ARM64 and x86-64: 36 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; 19],
}

impl Default for Termios {
    /// What a fresh terminal has: `stty sane`.
    fn default() -> Termios {
        Termios {
            iflag: ICRNL | IXON | IUTF8,
            oflag: OPOST | ONLCR,
            cflag: 0o277, // B38400 | CS8 | CREAD
            lflag: ISIG | ICANON | ECHO | ECHOE | ECHOK | ECHOCTL | ECHOKE | IEXTEN,
            line: 0,
            cc: [3, 28, 127, 21, 4, 0, 1, 0, 17, 19, 26, 0, 18, 15, 23, 22, 0, 0, 0],
        }
    }
}

impl Termios {
    pub fn encode(&self) -> [u8; 36] {
        let mut b = [0; 36];
        for (i, v) in [self.iflag, self.oflag, self.cflag, self.lflag].into_iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b[16] = self.line;
        b[17..36].copy_from_slice(&self.cc);
        b
    }

    pub fn decode(b: &[u8]) -> Termios {
        let at = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or_default());
        let mut cc = [0; 19];
        cc.copy_from_slice(&b[17..36]);
        Termios { iflag: at(0), oflag: at(1), cflag: at(2), lflag: at(3), line: b[16], cc }
    }
}

/// Where a terminal's output goes.
enum Sink {
    /// The host's terminal.
    Host(Arc<dyn Host>),
    /// A pty's master, which reads it from `output`.
    Master,
}

struct State {
    termios: Termios,
    /// Rows, columns, and pixel width and height.
    winsize: [u16; 4],
    /// The line being edited, in canonical mode.
    line: Vec<u8>,
    /// Lines ready to read, in canonical mode. An empty one is end-of-file (^D on an empty line).
    lines: VecDeque<Vec<u8>>,
    /// Bytes ready to read, in raw mode.
    raw: VecDeque<u8>,
    /// What the master hasn't read yet (a pty).
    output: VecDeque<u8>,
    /// The session it controls, and its foreground process group.
    session: Option<i32>,
    foreground: i32,
    waiting: Vec<Arc<Waiter>>,
    /// Open slave sides, and whether the master is open (a pty).
    slaves: usize,
    master: bool,
    locked: bool,
}

pub(crate) struct Tty {
    /// Its number under `/dev/pts`, or None for the console.
    pub index: Option<u32>,
    state: Mutex<State>,
    sink: Sink,
    /// Told when there's more to read on either side, or a side closes.
    pub watchers: Watchers,
}

/// A signal for a process group, which the terminal's input asked for.
pub(crate) type Signal = (i32, i32);

impl Tty {
    fn new(index: Option<u32>, sink: Sink, winsize: [u16; 4]) -> Arc<Tty> {
        Arc::new(Tty {
            index,
            sink,
            watchers: Watchers::default(),
            state: Mutex::new(State {
                termios: Termios::default(),
                winsize,
                line: Vec::new(),
                lines: VecDeque::new(),
                raw: VecDeque::new(),
                output: VecDeque::new(),
                session: None,
                foreground: 0,
                waiting: Vec::new(),
                slaves: 0,
                master: index.is_some(),
                locked: index.is_some(),
            }),
        })
    }

    /// The machine's console, on the host's terminal.
    pub fn console(host: Arc<dyn Host>) -> Arc<Tty> {
        let (rows, cols) = host.terminal_size().unwrap_or((24, 80));
        Tty::new(None, Sink::Host(host), [rows, cols, 0, 0])
    }

    /// A new pty pair, numbered `index`, with its master open.
    pub fn pty(index: u32) -> Arc<Tty> {
        Tty::new(Some(index), Sink::Master, [24, 80, 0, 0])
    }

    /// Something changed: wake whoever's waiting to read, and tell its watchers.
    fn wake(&self, state: &mut State) {
        for w in state.waiting.drain(..) {
            w.wake();
        }
        self.watchers.notify();
    }

    /// Output, as the terminal's settings change it, to its sink.
    fn emit(&self, state: &mut State, data: &[u8]) {
        let cooked: Vec<u8>;
        let data = if state.termios.oflag & (OPOST | ONLCR) == OPOST | ONLCR && data.contains(&b'\n') {
            cooked = data.iter().flat_map(|&b| if b == b'\n' { vec![b'\r', b'\n'] } else { vec![b] }).collect();
            &cooked
        } else {
            data
        };
        match &self.sink {
            Sink::Host(host) => host.write(host::Stream::Out, data),
            Sink::Master => {
                state.output.extend(data);
                self.wake(state);
            }
        }
    }

    fn echo(&self, state: &mut State, byte: u8) {
        let lflag = state.termios.lflag;
        if lflag & ECHO == 0 {
            if byte == b'\n' && lflag & (ECHONL | ICANON) == ECHONL | ICANON {
                self.emit(state, b"\n");
            }
            return;
        }
        if lflag & ECHOCTL != 0 && (byte < 32 && byte != b'\t' && byte != b'\n' || byte == 127) {
            self.emit(state, &[b'^', byte ^ 0x40]);
        } else {
            self.emit(state, &[byte]);
        }
    }

    /// Takes back the last character of the line being edited, on screen too.
    fn erase(&self, state: &mut State) -> bool {
        let Some(last) = state.line.last().copied() else { return false };
        // A whole UTF-8 character.
        let mut start = state.line.len() - 1;
        while start > 0 && state.line[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        state.line.truncate(start);
        if state.termios.lflag & (ECHO | ECHOE) == ECHO | ECHOE {
            let width = if state.termios.lflag & ECHOCTL != 0 && (last < 32 || last == 127) { 2 } else { 1 };
            for _ in 0..width {
                self.emit(state, b"\x08 \x08");
            }
        }
        true
    }

    /// Input, from the host's keyboard or a pty's master. Gives the signals it asks for.
    pub fn input(&self, data: &[u8]) -> Vec<Signal> {
        let mut state = lock(&self.state);
        let mut signals = Vec::new();
        for &byte in data {
            let t = state.termios;
            let byte = match byte {
                b'\r' if t.iflag & IGNCR != 0 => continue,
                b'\r' if t.iflag & ICRNL != 0 => b'\n',
                b'\n' if t.iflag & INLCR != 0 => b'\r',
                b => b,
            };
            let is = |i: usize| t.cc[i] != 0 && byte == t.cc[i];
            if t.lflag & ISIG != 0 && (is(VINTR) || is(VQUIT) || is(VSUSP)) {
                let signal = if is(VINTR) {
                    SIGINT
                } else if is(VQUIT) {
                    SIGQUIT
                } else {
                    SIGTSTP
                };
                if t.lflag & NOFLSH == 0 {
                    state.line.clear();
                    state.lines.clear();
                    state.raw.clear();
                }
                self.echo(&mut state, byte);
                if state.session.is_some() && state.foreground > 0 {
                    signals.push((state.foreground, signal));
                }
                continue;
            }
            if t.lflag & ICANON == 0 {
                state.raw.push_back(byte);
                self.echo(&mut state, byte);
                self.wake(&mut state);
                continue;
            }
            if is(VERASE) || byte == 8 && t.cc[VERASE] == 127 {
                self.erase(&mut state);
            } else if is(VWERASE) && t.lflag & IEXTEN != 0 {
                while state.line.last() == Some(&b' ') && self.erase(&mut state) {}
                while state.line.last().is_some_and(|&b| b != b' ') && self.erase(&mut state) {}
            } else if is(VKILL) {
                if t.lflag & ECHOKE != 0 {
                    while self.erase(&mut state) {}
                } else {
                    state.line.clear();
                    if t.lflag & ECHOK != 0 {
                        self.emit(&mut state, b"\n");
                    }
                }
            } else if is(VEOF) {
                let line = std::mem::take(&mut state.line);
                state.lines.push_back(line);
                self.wake(&mut state);
            } else if byte == b'\n' || is(VEOL) {
                state.line.push(byte);
                let line = std::mem::take(&mut state.line);
                state.lines.push_back(line);
                self.echo(&mut state, b'\n');
                self.wake(&mut state);
            } else {
                state.line.push(byte);
                self.echo(&mut state, byte);
            }
        }
        signals
    }

    /// Whether a read wouldn't wait.
    pub fn readable(&self) -> bool {
        let state = lock(&self.state);
        !state.lines.is_empty() || !state.raw.is_empty() || self.hung_up(&state)
    }

    /// How much there is to read now (`FIONREAD`).
    pub fn available(&self) -> usize {
        let state = lock(&self.state);
        state.lines.iter().map(Vec::len).sum::<usize>() + state.raw.len()
    }

    fn hung_up(&self, state: &State) -> bool {
        self.index.is_some() && !state.master
    }

    /// A read of the terminal: a line in canonical mode, as `VMIN` and `VTIME` say otherwise.
    pub fn read(&self, me: &Arc<Waiter>, len: usize, nonblock: bool) -> Result<Vec<u8>, Errno> {
        let started = Instant::now();
        loop {
            let deadline;
            {
                let mut state = lock(&self.state);
                let t = state.termios;
                if t.lflag & ICANON != 0 {
                    if let Some(line) = state.lines.front_mut() {
                        let n = len.min(line.len());
                        let out: Vec<u8> = line.drain(..n).collect();
                        if line.is_empty() {
                            state.lines.pop_front();
                        }
                        return Ok(out);
                    }
                    deadline = None;
                } else {
                    let (min, time) = (t.cc[VMIN] as usize, t.cc[VTIME] as u64);
                    let wanted = min.clamp(1, len.max(1));
                    let timer = (time > 0).then(|| started + Duration::from_millis(time * 100));
                    if state.raw.len() >= wanted || min == 0 && time == 0 || timer.is_some_and(|d| Instant::now() >= d) {
                        let n = len.min(state.raw.len());
                        return Ok(state.raw.drain(..n).collect());
                    }
                    deadline = timer;
                }
                if self.hung_up(&state) {
                    return Ok(Vec::new());
                }
                if nonblock {
                    return Err(EAGAIN);
                }
                me.reset();
                state.waiting.push(me.clone());
            }
            if me.wait(deadline) == Woke::Interrupted {
                let mut state = lock(&self.state);
                state.waiting.retain(|w| !Arc::ptr_eq(w, me));
                return Err(EINTR);
            }
        }
    }

    /// A program's write to the terminal.
    pub fn write(&self, data: &[u8]) -> Result<usize, Errno> {
        let mut state = lock(&self.state);
        if self.hung_up(&state) {
            return Err(EIO);
        }
        self.emit(&mut state, data);
        Ok(data.len())
    }

    /// A read of a pty's master: what the slave side wrote.
    pub fn read_master(&self, me: &Arc<Waiter>, len: usize, nonblock: bool) -> Result<Vec<u8>, Errno> {
        loop {
            {
                let mut state = lock(&self.state);
                if !state.output.is_empty() {
                    let n = len.min(state.output.len());
                    return Ok(state.output.drain(..n).collect());
                }
                if state.slaves == 0 && !state.locked {
                    return Err(EIO);
                }
                if nonblock {
                    return Err(EAGAIN);
                }
                me.reset();
                state.waiting.push(me.clone());
            }
            if me.wait(None) == Woke::Interrupted {
                return Err(EINTR);
            }
        }
    }

    pub fn master_readable(&self) -> bool {
        let state = lock(&self.state);
        !state.output.is_empty() || state.slaves == 0 && !state.locked
    }

    pub fn open_slave(&self) -> Result<(), Errno> {
        let mut state = lock(&self.state);
        if state.locked || !state.master {
            return Err(EIO);
        }
        state.slaves += 1;
        Ok(())
    }

    pub fn close_slave(&self) {
        let mut state = lock(&self.state);
        state.slaves = state.slaves.saturating_sub(1);
        self.wake(&mut state);
    }

    pub fn close_master(&self) {
        let mut state = lock(&self.state);
        state.master = false;
        self.wake(&mut state);
    }

    pub fn set_locked(&self, locked: bool) {
        lock(&self.state).locked = locked;
    }

    pub fn locked(&self) -> bool {
        lock(&self.state).locked
    }

    pub fn termios(&self) -> Termios {
        lock(&self.state).termios
    }

    /// New settings. Leaving canonical mode makes what's been typed readable as it is.
    pub fn set_termios(&self, termios: Termios, flush: bool) {
        let mut state = lock(&self.state);
        let was_canonical = state.termios.lflag & ICANON != 0;
        state.termios = termios;
        if flush {
            state.line.clear();
            state.lines.clear();
            state.raw.clear();
        } else if was_canonical && termios.lflag & ICANON == 0 {
            let lines: Vec<u8> = state.lines.drain(..).flatten().collect();
            let line = std::mem::take(&mut state.line);
            state.raw.extend(lines);
            state.raw.extend(line);
        } else if !was_canonical && termios.lflag & ICANON != 0 {
            let raw: Vec<u8> = state.raw.drain(..).collect();
            state.line.extend(raw);
        }
        self.wake(&mut state);
    }

    /// Throws away input (`which` 0 or 2) and output (1 or 2) not yet read.
    pub fn flush(&self, which: u64) {
        let mut state = lock(&self.state);
        if which != 1 {
            state.line.clear();
            state.lines.clear();
            state.raw.clear();
        }
        if which != 0 {
            state.output.clear();
        }
    }

    pub fn winsize(&self) -> [u16; 4] {
        lock(&self.state).winsize
    }

    /// A new size: if it changed, the foreground process group gets SIGWINCH.
    pub fn set_winsize(&self, size: [u16; 4]) -> Option<Signal> {
        let mut state = lock(&self.state);
        if state.winsize == size {
            return None;
        }
        state.winsize = size;
        (state.session.is_some() && state.foreground > 0).then_some((state.foreground, SIGWINCH))
    }

    pub fn session(&self) -> Option<i32> {
        lock(&self.state).session
    }

    pub fn foreground(&self) -> i32 {
        lock(&self.state).foreground
    }

    pub fn set_foreground(&self, pgid: i32) {
        lock(&self.state).foreground = pgid;
    }

    /// Makes this terminal `session`'s, with `pgid` in the foreground.
    pub fn set_session(&self, session: Option<i32>, pgid: i32) {
        let mut state = lock(&self.state);
        state.session = session;
        state.foreground = pgid;
    }
}
