//! Running what's parsed: commands joined and nested, redirections, pipes, programs and batch files, and cmd's own
//! commands.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use programs::*;

use crate::expand::{self, full_path};
use crate::parse::{Cond, ForKind, Node, Parser, Redir, unbalanced};

/// What a command does to what runs after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    Next,
    /// GOTO: the rest of the line is skipped, and the batch file goes on at the label.
    Goto(String),
    /// EXIT /B, or GOTO :EOF: the batch file (or CALLed label) ends.
    EndBatch,
    /// EXIT: cmd ends, with this code.
    Exit(i32),
}

/// What SETLOCAL keeps, for ENDLOCAL to put back.
struct Local {
    env: Vec<String>,
    cwd: String,
    delayed: bool,
    extensions: bool,
}

/// A batch file being run (or a label in one, CALLed).
pub struct Batch {
    lines: Vec<String>,
    pos: usize,
    /// `%0` first.
    args: Vec<String>,
    locals: Vec<Local>,
}

pub struct Shell {
    pub errorlevel: i32,
    /// Whether the last command worked, for `&&` and `||`.
    ok: bool,
    pub echo: bool,
    pub delayed: bool,
    pub extensions: bool,
    batches: Vec<Batch>,
    dirs: Vec<String>,
    for_vars: Vec<(char, String)>,
    /// This program, for running parts of a pipe in a cmd of their own.
    exe: String,
    pub interactive: bool,
}

const NOT_FOUND: &str = "The system cannot find the file specified.\r\n";
const PATH_NOT_FOUND: &str = "The system cannot find the path specified.\r\n";
pub const VERSION: &str = "Microsoft Windows [Version 10.0.19045.0]";

/// Cmd's own commands.
const INTERNAL: &[&str] = &[
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "echo", "endlocal", "erase", "exit", "ftype", "goto", "md", "mkdir", "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "ren", "rename", "rmdir", "set", "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
];

fn std_handle(fd: u8) -> u32 {
    match fd {
        0 => STD_INPUT,
        1 => STD_OUTPUT,
        _ => STD_ERROR,
    }
}

/// A file time, local, as DIR shows it: `11/16/2026  03:04 PM`.
pub fn file_time(t: FILETIME) -> String {
    let mut local = FILETIME::default();
    let mut st = SYSTEMTIME::default();
    unsafe {
        FileTimeToLocalFileTime(&t, &mut local);
        FileTimeToSystemTime(&local, &mut st);
    }
    let (h, pm) = match st.hour {
        0 => (12, false),
        1..=11 => (st.hour, false),
        12 => (12, true),
        h => (h - 12, true),
    };
    format!("{:02}/{:02}/{:04}  {:02}:{:02} {}", st.month, st.day, st.year, h, st.minute, if pm { "PM" } else { "AM" })
}

/// Files and folders matching a pattern: each one's find data.
pub fn find(pattern: &str) -> Vec<WIN32_FIND_DATAW> {
    let mut found = Vec::new();
    let w = wide(pattern);
    let mut data = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(w.as_ptr(), &mut data) };
    if h == INVALID_HANDLE_VALUE {
        return found;
    }
    loop {
        found.push(data);
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    found
}

pub fn attributes(path: &str) -> Option<u32> {
    let w = wide(path);
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    (a != INVALID_FILE_ATTRIBUTES).then_some(a)
}

fn is_dir(path: &str) -> bool {
    attributes(path).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0)
}

/// `dir\name`, without doubling a `\`.
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "." {
        return name.to_string();
    }
    if dir.ends_with(['\\', '/', ':']) { format!("{dir}{name}") } else { format!("{dir}\\{name}") }
}

/// The folder part of a path, as written (`C:\x\` from `C:\x\*.txt`), or empty.
fn dir_part(path: &str) -> &str {
    match path.rfind(['\\', '/', ':']) {
        Some(at) => &path[..=at],
        None => "",
    }
}

fn unquote(s: &str) -> String {
    s.replace('"', "")
}

/// Arguments as cmd's own commands take them: split on spaces (and `,`, `;`, `=`), quotes kept together.
fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut w = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
            w.push(c);
        } else if !quoted && matches!(c, ' ' | '\t' | ',' | ';' | '=') {
            if !w.is_empty() {
                words.push(core::mem::take(&mut w));
            }
        } else {
            w.push(c);
        }
    }
    if !w.is_empty() {
        words.push(w);
    }
    words
}

/// Switches (`/x`) and the rest.
fn switches(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut sw = Vec::new();
    let mut rest = Vec::new();
    for a in args {
        if a.starts_with('/') && a.len() > 1 {
            // `/a/b` is two.
            for part in a[1..].split('/') {
                if !part.is_empty() {
                    sw.push(part.to_ascii_lowercase());
                }
            }
        } else {
            rest.push(a.clone());
        }
    }
    (sw, rest)
}

fn ask_yes(question: &str) -> bool {
    out(question);
    let answer = Input::stdin().line().unwrap_or_default();
    answer.trim().to_ascii_lowercase().starts_with('y')
}

impl Shell {
    pub fn new() -> Shell {
        let exe = fill(|b, n| unsafe { GetModuleFileNameW(0, b, n) }).unwrap_or_else(|| String::from("C:\\Windows\\System32\\cmd.exe"));
        Shell { errorlevel: 0, ok: true, echo: true, delayed: false, extensions: true, batches: Vec::new(), dirs: Vec::new(), for_vars: Vec::new(), exe, interactive: false }
    }

    fn fail(&mut self, message: &str) {
        err(message);
        self.errorlevel = 1;
        self.ok = false;
    }

    /// What's left to put in when a command runs: FOR's variables, then `!` if delayed expansion is on.
    fn late(&self, text: &str) -> String {
        let t = expand::for_vars(text, &self.for_vars);
        if self.delayed { expand::delayed(&t, self.errorlevel) } else { t }
    }

    pub fn in_batch(&self) -> bool {
        !self.batches.is_empty()
    }

    /// A line (or block) as typed or read from a batch file, run.
    pub fn run_line(&mut self, line: &str) -> Flow {
        let args = self.batches.last().map(|b| b.args.clone());
        let expanded = expand::percent(line, args.as_deref(), self.errorlevel);
        let node = Parser::parse(&expanded);
        self.run(&node)
    }

    pub fn run(&mut self, node: &Node) -> Flow {
        match node {
            Node::Empty => Flow::Next,
            Node::Seq(a, b) => match self.run(a) {
                Flow::Next => self.run(b),
                other => other,
            },
            Node::And(a, b) => match self.run(a) {
                Flow::Next if self.ok => self.run(b),
                other => other,
            },
            Node::Or(a, b) => match self.run(a) {
                Flow::Next if !self.ok => self.run(b),
                other => other,
            },
            Node::Block { body, redirs } => {
                let Some(saved) = self.redirect(redirs) else { return Flow::Next };
                let flow = self.run(body);
                restore(saved);
                flow
            }
            Node::If { not, fold, cond, then, other } => {
                let truth = self.condition(cond, *fold) != *not;
                if truth {
                    self.run(then)
                } else if let Some(other) = other {
                    self.run(other)
                } else {
                    Flow::Next
                }
            }
            Node::For { kind, var, set, body } => self.run_for(kind, *var, set, body),
            Node::Pipe(parts) => self.pipe(parts),
            Node::Simple { text, redirs, .. } => {
                let text = self.late(text);
                let Some(saved) = self.redirect(redirs) else { return Flow::Next };
                let flow = self.simple(&text);
                restore(saved);
                flow
            }
        }
    }

    // ---- Redirections ----

    /// Opens a node's redirections and makes them the standard handles; what to put back, or None if one couldn't
    /// be opened (said so).
    fn redirect(&mut self, redirs: &[Redir]) -> Option<Vec<(u32, HANDLE, bool)>> {
        let mut saved: Vec<(u32, HANDLE, bool)> = Vec::new();
        for r in redirs {
            let (fd, handle, opened) = match r {
                Redir::Out { fd, path, append } => {
                    let path = unquote(&self.late(path));
                    let w = wide(&path);
                    let mut sa = SECURITY_ATTRIBUTES::inherited();
                    let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &mut sa, if *append { OPEN_ALWAYS } else { CREATE_ALWAYS }, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
                    if h == INVALID_HANDLE_VALUE {
                        restore(saved);
                        self.fail(if last_error() == ERROR_PATH_NOT_FOUND { PATH_NOT_FOUND } else { "Access is denied.\r\n" });
                        return None;
                    }
                    if *append {
                        unsafe { SetFilePointerEx(h, 0, core::ptr::null_mut(), FILE_END) };
                    }
                    (*fd, h, true)
                }
                Redir::In(path) => {
                    let path = unquote(&self.late(path));
                    let w = wide(&path);
                    let mut sa = SECURITY_ATTRIBUTES::inherited();
                    let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &mut sa, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
                    if h == INVALID_HANDLE_VALUE {
                        restore(saved);
                        self.fail(NOT_FOUND);
                        return None;
                    }
                    (0, h, true)
                }
                Redir::Dup { fd, to } => (*fd, unsafe { GetStdHandle(std_handle(*to)) }, false),
            };
            let which = std_handle(fd);
            saved.push((which, unsafe { GetStdHandle(which) }, false));
            unsafe { SetStdHandle(which, handle) };
            if opened {
                // Closed when put back.
                if let Some(last) = saved.last_mut() {
                    last.2 = true;
                }
            }
        }
        Some(saved)
    }

    // ---- Conditions ----

    fn condition(&mut self, cond: &Cond, fold: bool) -> bool {
        match cond {
            Cond::Errorlevel(n) => self.errorlevel >= self.late(n).trim().parse::<i32>().unwrap_or(0),
            Cond::CmdExtVersion(n) => 2 >= self.late(n).trim().parse::<i32>().unwrap_or(0),
            Cond::Defined(name) => get_env(&self.late(name)).is_some(),
            Cond::Exist(path) => {
                let path = unquote(&self.late(path));
                if path.contains(['*', '?']) { !find(&path).is_empty() } else { attributes(&path).is_some() }
            }
            Cond::Compare(a, op, b) => {
                let (a, b) = (self.late(a), self.late(b));
                let numbers = (parse_number(&a), parse_number(&b));
                let order = match numbers {
                    (Some(x), Some(y)) if op != "==" => x.cmp(&y),
                    _ if fold => a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()),
                    _ => a.cmp(&b),
                };
                use core::cmp::Ordering::*;
                match op.to_ascii_lowercase().as_str() {
                    "==" | "equ" => order == Equal,
                    "neq" => order != Equal,
                    "lss" => order == Less,
                    "leq" => order != Greater,
                    "gtr" => order == Greater,
                    "geq" => order != Less,
                    _ => {
                        self.fail(&format!("{op} was unexpected at this time.\r\n"));
                        false
                    }
                }
            }
        }
    }

    // ---- FOR ----

    fn with_var(&mut self, var: char, value: String, body: &Node) -> Flow {
        self.for_vars.push((var, value));
        let flow = self.run(body);
        self.for_vars.pop();
        flow
    }

    fn run_for(&mut self, kind: &ForKind, var: char, set: &str, body: &Node) -> Flow {
        let set = self.late(set);
        match kind {
            ForKind::Count => {
                let n: Vec<i64> = words(&set).iter().map(|w| parse_number(w).unwrap_or(0)).collect();
                let (start, step, end) = (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(1), n.get(2).copied().unwrap_or(0));
                let mut i = start;
                while step != 0 && ((step > 0 && i <= end) || (step < 0 && i >= end)) {
                    match self.with_var(var, format!("{i}"), body) {
                        Flow::Next => {}
                        other => return other,
                    }
                    i += step;
                }
                Flow::Next
            }
            ForKind::Plain | ForKind::Dirs => {
                let dirs = matches!(kind, ForKind::Dirs);
                for item in words(&set) {
                    let items: Vec<String> = if item.contains(['*', '?']) {
                        let dir = dir_part(&unquote(&item)).to_string();
                        find(&unquote(&item))
                            .iter()
                            .filter(|f| (f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0) == dirs)
                            .map(|f| narrow(&f.name))
                            .filter(|n| n != "." && n != "..")
                            .map(|n| format!("{dir}{n}"))
                            .collect()
                    } else if dirs {
                        Vec::new()
                    } else {
                        vec![item.clone()]
                    };
                    for value in items {
                        match self.with_var(var, value, body) {
                            Flow::Next => {}
                            other => return other,
                        }
                    }
                }
                Flow::Next
            }
            ForKind::Recursive(root) => {
                let root = root.as_ref().map(|r| full_path(&unquote(&self.late(r)))).unwrap_or_else(current_dir);
                let mut folders = vec![root];
                let mut i = 0;
                while i < folders.len() {
                    let dir = folders[i].clone();
                    for f in find(&join(&dir, "*")) {
                        let name = narrow(&f.name);
                        if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && name != "." && name != ".." {
                            folders.push(join(&dir, &name));
                        }
                    }
                    i += 1;
                }
                for dir in folders {
                    for item in words(&set) {
                        let item = unquote(&item);
                        let values: Vec<String> = if item.contains(['*', '?']) {
                            find(&join(&dir, &item)).iter().filter(|f| f.attributes & FILE_ATTRIBUTE_DIRECTORY == 0).map(|f| join(&dir, &narrow(&f.name))).collect()
                        } else if item == "." {
                            vec![dir.clone()]
                        } else {
                            vec![join(&dir, &item)]
                        };
                        for value in values {
                            match self.with_var(var, value, body) {
                                Flow::Next => {}
                                other => return other,
                            }
                        }
                    }
                }
                Flow::Next
            }
            ForKind::Lines(options) => self.for_lines(&self.late(options), var, &set, body),
        }
    }

    fn for_lines(&mut self, options: &str, var: char, set: &str, body: &Node) -> Flow {
        let mut delims: Vec<char> = vec![' ', '\t'];
        let mut tokens: Vec<(usize, usize)> = vec![(1, 1)];
        let mut rest_token = false;
        let mut skip = 0usize;
        let mut eol = Some(';');
        let mut backq = false;
        // Options: each `key=value`, where `delims=` takes the rest of the string.
        let mut opts = options.trim();
        while !opts.is_empty() {
            let lower = opts.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("delims=") {
                let n = opts.len() - v.len();
                // Up to the next key, or the end: delims may hold spaces only last.
                let value = &opts[n..];
                let end = ["tokens=", "skip=", "eol=", "usebackq"].iter().filter_map(|k| value.to_ascii_lowercase().find(k)).min().unwrap_or(value.len());
                let mut d = &value[..end];
                if end < value.len() {
                    d = d.trim_end();
                }
                delims = d.chars().collect();
                opts = value[end..].trim_start();
            } else if let Some(v) = lower.strip_prefix("tokens=") {
                let n = opts.len() - v.len();
                let value = opts[n..].split(' ').next().unwrap_or("");
                tokens.clear();
                for part in value.split(',') {
                    let part = part.trim();
                    if part == "*" {
                        rest_token = true;
                        continue;
                    }
                    let (part, star) = match part.strip_suffix('*') {
                        Some(p) => (p, true),
                        None => (part, false),
                    };
                    if let Some((a, b)) = part.split_once('-') {
                        tokens.push((a.parse().unwrap_or(1), b.parse().unwrap_or(1)));
                    } else if let Ok(n) = part.parse() {
                        tokens.push((n, n));
                    }
                    rest_token |= star;
                }
                opts = opts[n + value.len()..].trim_start();
            } else if let Some(v) = lower.strip_prefix("skip=") {
                let n = opts.len() - v.len();
                let value = opts[n..].split(' ').next().unwrap_or("");
                skip = value.parse().unwrap_or(0);
                opts = opts[n + value.len()..].trim_start();
            } else if let Some(v) = lower.strip_prefix("eol=") {
                let n = opts.len() - v.len();
                eol = opts[n..].chars().next();
                opts = opts[n + eol.map_or(0, char::len_utf8)..].trim_start();
            } else if lower.starts_with("usebackq") {
                backq = true;
                opts = opts[8..].trim_start();
            } else {
                opts = opts[1..].trim_start();
            }
        }
        // Where the lines come from: a command's output, a string, or files.
        let set = set.trim();
        let (command_quote, string_quote) = if backq { ('`', '\'') } else { ('\'', '"') };
        let mut text = String::new();
        if set.starts_with(command_quote) && set.ends_with(command_quote) && set.len() >= 2 {
            text = self.capture(&set[1..set.len() - 1]);
        } else if set.starts_with(string_quote) && set.ends_with(string_quote) && set.len() >= 2 {
            text = set[1..set.len() - 1].to_string();
        } else {
            let names: Vec<String> = if backq { split_arguments(set) } else { words(set).iter().map(|w| unquote(w)).collect() };
            for name in names {
                match read_file(&name) {
                    Some(bytes) => text.push_str(&String::from_utf8_lossy(&bytes)),
                    None => {
                        self.fail(&format!("The system cannot find the file {name}.\r\n"));
                        return Flow::Next;
                    }
                }
                text.push('\n');
            }
        }
        let wanted: Vec<usize> = {
            let mut w: Vec<usize> = tokens.iter().flat_map(|&(a, b)| a..=b).collect();
            w.sort_unstable();
            w.dedup();
            w
        };
        for line in text.lines().skip(skip) {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || eol.is_some_and(|e| line.starts_with(e)) {
                continue;
            }
            // Its tokens, and where each starts, for `*`'s rest.
            let mut found: Vec<(usize, &str)> = Vec::new();
            let mut start = None;
            for (i, c) in line.char_indices() {
                if delims.contains(&c) {
                    if let Some(s) = start.take() {
                        found.push((s, &line[s..i]));
                    }
                } else if start.is_none() {
                    start = Some(i);
                }
            }
            if let Some(s) = start {
                found.push((s, &line[s..]));
            }
            if found.is_empty() {
                continue;
            }
            let mut values: Vec<(char, String)> = Vec::new();
            let mut letter = var;
            for &n in &wanted {
                values.push((letter, found.get(n - 1).map_or(String::new(), |(_, t)| t.to_string())));
                letter = (letter as u8 + 1) as char;
            }
            if rest_token {
                let after = wanted.last().copied().unwrap_or(0);
                let rest = found.get(after).map_or(String::new(), |&(s, _)| line[s..].to_string());
                values.push((letter, rest));
            }
            let count = values.len();
            self.for_vars.extend(values);
            let flow = self.run(body);
            for _ in 0..count {
                self.for_vars.pop();
            }
            if flow != Flow::Next {
                return flow;
            }
        }
        Flow::Next
    }

    /// What a command writes, run by `cmd /c` with its output into a pipe (FOR /F's 'command').
    fn capture(&mut self, command: &str) -> String {
        let (mut read, mut write) = (core::ptr::null_mut(), core::ptr::null_mut());
        let mut sa = SECURITY_ATTRIBUTES::inherited();
        if unsafe { CreatePipe(&mut read, &mut write, &mut sa, 0) } == 0 {
            return String::new();
        }
        unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };
        let line = format!("\"{}\" /D /S /C \"{command}\"", self.exe);
        let child = spawn(&self.exe, &line, unsafe { GetStdHandle(STD_INPUT) }, write, unsafe { GetStdHandle(STD_ERROR) });
        unsafe { CloseHandle(write) };
        let mut text = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut n = 0u32;
            if unsafe { ReadFile(read, buffer.as_mut_ptr(), buffer.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
                break;
            }
            text.extend_from_slice(&buffer[..n as usize]);
        }
        unsafe { CloseHandle(read) };
        if let Some(child) = child {
            wait(&child);
        }
        String::from_utf8_lossy(&text).into_owned()
    }

    // ---- Pipes ----

    fn pipe(&mut self, parts: &[(Node, String)]) -> Flow {
        let mut children = Vec::new();
        let mut input = unsafe { GetStdHandle(STD_INPUT) };
        let mut input_ours = false;
        for (i, (node, text)) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            let (mut read, mut write) = (core::ptr::null_mut(), core::ptr::null_mut());
            let output = if last {
                unsafe { GetStdHandle(STD_OUTPUT) }
            } else {
                let mut sa = SECURITY_ATTRIBUTES::inherited();
                unsafe { CreatePipe(&mut read, &mut write, &mut sa, 0) };
                // Ours, not the child's.
                unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };
                write
            };
            // A program runs as itself; anything else (cmd's own commands, blocks), in a cmd of its own.
            let program = match node {
                Node::Simple { text, redirs, .. } if redirs.is_empty() => self.program_for(&self.late(text)),
                _ => None,
            };
            let child = match program {
                Some((path, line)) => spawn(&path, &line, input, output, unsafe { GetStdHandle(STD_ERROR) }),
                None => spawn(&self.exe, &format!("\"{}\" /S /D /C \"{}\"", self.exe, self.late(text)), input, output, unsafe { GetStdHandle(STD_ERROR) }),
            };
            if input_ours {
                unsafe { CloseHandle(input) };
            }
            if !last {
                unsafe { CloseHandle(write) };
                unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
                input = read;
                input_ours = true;
            }
            children.push(child);
        }
        let mut code = 0;
        for child in children.iter().flatten() {
            code = wait(child);
        }
        self.errorlevel = code as i32;
        self.ok = code == 0;
        Flow::Next
    }

    // ---- Simple commands ----

    /// A command's name and the rest: `echo.` is ECHO and `.`; `cd..` is CD and `..`.
    fn split_name(text: &str) -> (String, String) {
        let text = text.trim_start_matches([' ', '\t', '@']);
        let end = text.find(|c: char| matches!(c, ' ' | '\t' | ',' | ';' | '=' | '(' | '/' | '.' | '\\' | '+' | ':' | '[' | ']' | '"')).unwrap_or(text.len());
        (text[..end].to_string(), text[end..].to_string())
    }

    fn simple(&mut self, text: &str) -> Flow {
        let (name, rest) = Shell::split_name(text);
        let lower = name.to_ascii_lowercase();
        // A drive alone (`C:`) changes to it.
        if lower.len() == 1 && rest.trim() == ":" {
            return Flow::Next;
        }
        if !lower.is_empty() && INTERNAL.contains(&lower.as_str()) {
            self.ok = true;
            return self.internal(&lower, &rest);
        }
        self.external(text)
    }

    /// A program the command line names, found as cmd finds one, and the command line to start it with. None for a
    /// batch file or one that isn't there.
    fn program_for(&self, text: &str) -> Option<(String, String)> {
        let (name, _) = Shell::split_name(text);
        if INTERNAL.contains(&name.to_ascii_lowercase().as_str()) {
            return None;
        }
        let first = first_word(text);
        let path = search(&unquote(&first))?;
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            return None;
        }
        Some((path, text.trim_start().to_string()))
    }

    fn external(&mut self, text: &str) -> Flow {
        let first = first_word(text);
        let name = unquote(&first);
        let Some(path) = search(&name) else {
            err(&format!("'{name}' is not recognized as an internal or external command,\r\noperable program or batch file.\r\n"));
            self.errorlevel = 9009;
            self.ok = false;
            return Flow::Next;
        };
        let lower = path.to_ascii_lowercase();
        let args_text = text.trim_start()[first.len()..].to_string();
        if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            // A batch file named without CALL takes over from the one running, as cmd has it.
            let mut args = vec![first.clone()];
            args.extend(words(&args_text));
            if let Some(b) = self.batches.pop() {
                let mut locals = b.locals;
                let flow = self.batch(&path, args);
                self.end_locals(&mut locals);
                return if matches!(flow, Flow::Exit(_)) { flow } else { Flow::EndBatch };
            }
            return self.batch(&path, args);
        }
        match spawn(&path, text.trim_start(), unsafe { GetStdHandle(STD_INPUT) }, unsafe { GetStdHandle(STD_OUTPUT) }, unsafe { GetStdHandle(STD_ERROR) }) {
            Some(child) => {
                let code = wait(&child);
                self.errorlevel = code as i32;
                self.ok = code == 0;
            }
            None => {
                err(&format!("'{name}' could not be started.\r\n"));
                self.errorlevel = 9009;
                self.ok = false;
            }
        }
        Flow::Next
    }

    // ---- Batch files ----

    /// Runs a batch file: its lines one by one, with its arguments (`%0` first).
    pub fn batch(&mut self, path: &str, args: Vec<String>) -> Flow {
        let Some(bytes) = read_file(path) else {
            self.fail(NOT_FOUND);
            return Flow::Next;
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let lines: Vec<String> = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect();
        self.batches.push(Batch { lines, pos: 0, args, locals: Vec::new() });
        self.run_batch()
    }

    /// Runs the batch on top until it ends; it's taken off then.
    fn run_batch(&mut self) -> Flow {
        let mut result = Flow::Next;
        loop {
            let Some(line) = self.next_batch_line() else { break };
            let trimmed = line.trim_start_matches([' ', '\t', '@']);
            if trimmed.is_empty() || trimmed.starts_with(':') {
                continue;
            }
            if self.echo && !line.trim_start().starts_with('@') {
                out(&format!("\r\n{}{}\r\n", self.prompt(), line.trim()));
            }
            match self.run_line(&line) {
                Flow::Next => {}
                Flow::Goto(label) => {
                    if !self.goto(&label) {
                        err(&format!("The system cannot find the batch label specified - {label}\r\n"));
                        break;
                    }
                }
                Flow::EndBatch => break,
                Flow::Exit(code) => {
                    result = Flow::Exit(code);
                    break;
                }
            }
        }
        if let Some(mut b) = self.batches.pop() {
            self.end_locals(&mut b.locals);
        }
        result
    }

    /// The batch file's next line: with `^` at the end joined to the one after, and a block left open joined to the
    /// lines that close it.
    fn next_batch_line(&mut self) -> Option<String> {
        let b = self.batches.last_mut()?;
        if b.pos >= b.lines.len() {
            return None;
        }
        let mut line = b.lines[b.pos].clone();
        b.pos += 1;
        loop {
            if line.ends_with('^') && !line.ends_with("^^") && b.pos < b.lines.len() {
                line.pop();
                line.push_str(&b.lines[b.pos]);
                b.pos += 1;
            } else if unbalanced(&line) && b.pos < b.lines.len() {
                line.push('\n');
                line.push_str(&b.lines[b.pos]);
                b.pos += 1;
            } else {
                return Some(line);
            }
        }
    }

    /// Goes to a label in the batch file running: after the line it's on, then from the top. Whether it's there.
    fn goto(&mut self, label: &str) -> bool {
        let label = label.trim().trim_start_matches(':');
        let label = label.split([' ', '\t']).next().unwrap_or("");
        let Some(b) = self.batches.last_mut() else { return false };
        let at = (b.pos..b.lines.len()).chain(0..b.pos).find(|&i| {
            let l = b.lines[i].trim_start();
            l.strip_prefix(':').is_some_and(|name| eq_ignore_case(name.split([' ', '\t', '+', ':']).next().unwrap_or(""), label))
        });
        match at {
            Some(i) => {
                b.pos = i + 1;
                true
            }
            None => false,
        }
    }

    fn end_locals(&mut self, locals: &mut Vec<Local>) {
        while let Some(local) = locals.pop() {
            self.restore_local(local);
        }
    }

    fn restore_local(&mut self, local: Local) {
        let now = all_env();
        for var in &now {
            if let Some((name, _)) = var.split_once('=')
                && !name.is_empty()
                && !local.env.iter().any(|v| v.split_once('=').is_some_and(|(n, _)| eq_ignore_case(n, name)))
            {
                set_env(name, None);
            }
        }
        for var in &local.env {
            if let Some((name, value)) = var.split_once('=')
                && !name.is_empty()
            {
                set_env(name, Some(value));
            }
        }
        let w = wide(&local.cwd);
        unsafe { SetCurrentDirectoryW(w.as_ptr()) };
        self.delayed = local.delayed;
        self.extensions = local.extensions;
    }

    // ---- The prompt ----

    pub fn prompt(&self) -> String {
        let template = get_env("PROMPT").filter(|p| !p.is_empty()).unwrap_or_else(|| String::from("$P$G"));
        let mut out = String::new();
        let mut chars = template.chars();
        while let Some(c) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            match chars.next().map(|c| c.to_ascii_uppercase()) {
                Some('P') => out.push_str(&current_dir()),
                Some('G') => out.push('>'),
                Some('L') => out.push('<'),
                Some('B') => out.push('|'),
                Some('Q') => out.push('='),
                Some('S') => out.push(' '),
                Some('$') => out.push('$'),
                Some('_') => out.push_str("\r\n"),
                Some('N') => out.push_str(&current_dir()[..1]),
                Some('D') => out.push_str(&expand::date_text()),
                Some('T') => out.push_str(&expand::time_text(true)),
                Some('V') => out.push_str(VERSION),
                Some('E') => out.push('\x1b'),
                Some('A') => out.push('&'),
                Some('C') => out.push('('),
                Some('F') => out.push(')'),
                Some('H') => out.push('\x08'),
                Some('+') => out.push_str(&"+".repeat(self.dirs.len())),
                _ => {}
            }
        }
        out
    }

    // ---- Cmd's own commands ----

    fn internal(&mut self, name: &str, rest: &str) -> Flow {
        let args = words(rest);
        match name {
            "echo" => self.echo_command(rest),
            "rem" => {}
            "cls" => out("\x1b[2J\x1b[3J\x1b[H"),
            "ver" => out(&format!("\r\n{VERSION}\r\n")),
            "vol" => self.vol(),
            "cd" | "chdir" => self.cd(rest),
            "pushd" => {
                let target = unquote(rest.trim());
                if target.is_empty() {
                    for d in self.dirs.iter().rev() {
                        out(&format!("{d}\r\n"));
                    }
                } else {
                    let here = current_dir();
                    if self.change_dir(&target) {
                        self.dirs.push(here);
                    }
                }
            }
            "popd" => {
                if let Some(d) = self.dirs.pop() {
                    self.change_dir(&d);
                }
            }
            "set" => self.set(rest),
            "setlocal" => {
                if let Some(b) = self.batches.last_mut() {
                    b.locals.push(Local { env: all_env(), cwd: current_dir(), delayed: self.delayed, extensions: self.extensions });
                }
                for a in &args {
                    match a.to_ascii_lowercase().as_str() {
                        "enabledelayedexpansion" => self.delayed = true,
                        "disabledelayedexpansion" => self.delayed = false,
                        "enableextensions" => self.extensions = true,
                        "disableextensions" => self.extensions = false,
                        _ => {}
                    }
                }
                self.errorlevel = 0;
            }
            "endlocal" => {
                if let Some(local) = self.batches.last_mut().and_then(|b| b.locals.pop()) {
                    self.restore_local(local);
                }
            }
            "exit" => {
                let (sw, rest) = switches(&args);
                let code = rest.first().and_then(|c| parse_number(c)).map(|c| c as i32);
                if sw.iter().any(|s| s == "b") {
                    if let Some(c) = code {
                        self.errorlevel = c;
                    }
                    if self.in_batch() {
                        return Flow::EndBatch;
                    }
                    return Flow::Next;
                }
                return Flow::Exit(code.unwrap_or(self.errorlevel));
            }
            "goto" => {
                let label = rest.trim().trim_start_matches(':').to_string();
                if eq_ignore_case(label.split(' ').next().unwrap_or(""), "eof") {
                    return Flow::EndBatch;
                }
                if !self.in_batch() {
                    return Flow::Next;
                }
                return Flow::Goto(label);
            }
            "call" => return self.call(rest.trim_start()),
            "shift" => {
                let from = args.first().and_then(|a| a.strip_prefix('/')).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                if let Some(b) = self.batches.last_mut()
                    && from < b.args.len()
                {
                    b.args.remove(from);
                }
            }
            "title" => {
                let w = wide(rest.trim());
                unsafe { SetConsoleTitleW(w.as_ptr()) };
            }
            "color" | "break" | "verify" => {
                if name == "verify" && args.is_empty() {
                    out("VERIFY is off.\r\n");
                }
            }
            "path" => {
                let value = rest.trim_start_matches([' ', '=']);
                if value.is_empty() {
                    out(&format!("PATH={}\r\n", get_env("PATH").unwrap_or_else(|| String::from("(null)"))));
                } else if value == ";" {
                    set_env("PATH", None);
                } else {
                    set_env("PATH", Some(value));
                }
            }
            "prompt" => {
                let value = rest.trim_start();
                set_env("PROMPT", if value.is_empty() { None } else { Some(value) });
            }
            "pause" => {
                out("Press any key to continue . . . ");
                let _ = Input::stdin().line();
                out("\r\n");
            }
            "date" | "time" => {
                let (sw, _) = switches(&args);
                let value = if name == "date" { expand::date_text() } else { expand::time_text(!sw.iter().any(|s| s == "t")) };
                if sw.iter().any(|s| s == "t") {
                    out(&format!("{value}\r\n"));
                } else {
                    out(&format!("The current {name} is: {value}\r\n"));
                    out(if name == "date" { "Enter the new date: (mm-dd-yy) " } else { "Enter the new time: " });
                    let _ = Input::stdin().line();
                }
            }
            "type" => self.type_files(&args),
            "dir" => crate::dir::dir(self, &args),
            "md" | "mkdir" => self.mkdir(&args),
            "rd" | "rmdir" => self.rmdir(&args),
            "del" | "erase" => self.del(&args),
            "copy" => self.copy(&args),
            "move" => self.move_files(&args),
            "ren" | "rename" => self.rename(&args),
            "mklink" => self.mklink(&args),
            "start" => self.start(rest),
            "assoc" | "ftype" => {
                if let Some(a) = args.first() {
                    self.fail(&format!("File association not found for extension {a}\r\n"));
                }
            }
            _ => {}
        }
        Flow::Next
    }

    fn echo_command(&mut self, rest: &str) {
        // `echo.`, `echo:` and the like: what's after is printed, even nothing.
        if let Some(text) = rest.strip_prefix(['.', ':', '(', ';', ',', '=', '/', '[', ']', '+', '\\']) {
            out(&format!("{text}\r\n"));
            return;
        }
        let text = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
        match text.trim().to_ascii_lowercase().as_str() {
            "" => out(&format!("ECHO is {}.\r\n", if self.echo { "on" } else { "off" })),
            "on" => self.echo = true,
            "off" => self.echo = false,
            _ => out(&format!("{text}\r\n")),
        }
    }

    fn vol(&mut self) {
        let root = wide("C:\\");
        let mut serial = 0u32;
        let mut label = [0u16; 64];
        unsafe { GetVolumeInformationW(root.as_ptr(), label.as_mut_ptr(), 64, &mut serial, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), 0) };
        let label = narrow(&label);
        if label.is_empty() {
            out(" Volume in drive C has no label.\r\n");
        } else {
            out(&format!(" Volume in drive C is {label}\r\n"));
        }
        out(&format!(" Volume Serial Number is {:04X}-{:04X}\r\n", serial >> 16, serial & 0xffff));
    }

    fn change_dir(&mut self, target: &str) -> bool {
        let w = wide(target);
        if unsafe { SetCurrentDirectoryW(w.as_ptr()) } == 0 {
            self.fail(if is_dir(target) || attributes(target).is_none() { PATH_NOT_FOUND } else { "The directory name is invalid.\r\n" });
            return false;
        }
        true
    }

    fn cd(&mut self, rest: &str) {
        let mut target = rest.trim().to_string();
        if let Some(t) = target.strip_prefix("/d").or_else(|| target.strip_prefix("/D")) {
            target = t.trim().to_string();
        }
        let target = unquote(&target);
        let target = target.trim_end();
        if target.is_empty() {
            out(&format!("{}\r\n", current_dir()));
            return;
        }
        // A drive alone is where that drive is now.
        if target.len() == 2 && target.ends_with(':') {
            out(&format!("{}\r\n", current_dir()));
            return;
        }
        self.change_dir(target);
    }

    fn set(&mut self, rest: &str) {
        let text = rest.trim_start();
        let lower = text.to_ascii_lowercase();
        if let Some(expr) = lower.strip_prefix("/a") {
            let expr = &text[text.len() - expr.len()..];
            let expr = unquote(expr.trim());
            match crate::arith::eval(&expr) {
                Ok(value) => {
                    if !self.in_batch() {
                        out(&format!("{value}"));
                    }
                    self.errorlevel = 0;
                }
                Err(why) => self.fail(&format!("{why}\r\n")),
            }
            return;
        }
        if let Some(spec) = lower.strip_prefix("/p") {
            let spec = text[text.len() - spec.len()..].trim_start();
            let spec = if spec.starts_with('"') && spec.ends_with('"') && spec.len() >= 2 { &spec[1..spec.len() - 1] } else { spec };
            let Some((name, prompt)) = spec.split_once('=') else {
                self.fail("The syntax of the command is incorrect.\r\n");
                return;
            };
            out(prompt);
            match Input::stdin().line() {
                Some(line) if !line.is_empty() => {
                    set_env(name, Some(&line));
                    self.errorlevel = 0;
                }
                _ => self.errorlevel = 1,
            }
            return;
        }
        // `set "name=value"`: the quotes go, and anything after the last one.
        let spec = if let Some(inner) = text.strip_prefix('"') { inner.rsplit_once('"').map_or(inner, |(v, _)| v) } else { text };
        match spec.split_once('=') {
            None => {
                let prefix = spec.trim();
                let mut vars: Vec<String> = all_env().into_iter().filter(|v| !v.starts_with('=')).filter(|v| v.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())).collect();
                vars.sort_by_key(|v| v.to_ascii_lowercase());
                if vars.is_empty() {
                    self.fail(&format!("Environment variable {prefix} not defined\r\n"));
                    return;
                }
                for v in vars {
                    out(&format!("{v}\r\n"));
                }
            }
            Some((name, value)) => {
                if name.is_empty() {
                    self.fail("The syntax of the command is incorrect.\r\n");
                    return;
                }
                set_env(name, if value.is_empty() { None } else { Some(value) });
                self.errorlevel = 0;
            }
        }
    }

    fn call(&mut self, rest: &str) -> Flow {
        if let Some(label) = rest.strip_prefix(':') {
            // A label in this batch file, as a subroutine with arguments of its own.
            let Some(current) = self.batches.last() else { return Flow::Next };
            let mut args = words(label);
            if args.is_empty() {
                return Flow::Next;
            }
            let name = args.remove(0);
            let mut all = vec![format!(":{name}")];
            all.extend(args);
            let lines = current.lines.clone();
            self.batches.push(Batch { lines, pos: 0, args: all, locals: Vec::new() });
            if !self.goto(&name) {
                self.batches.pop();
                err(&format!("The system cannot find the batch label specified - {name}\r\n"));
                return Flow::Next;
            }
            return match self.run_batch() {
                Flow::Exit(code) => Flow::Exit(code),
                _ => Flow::Next,
            };
        }
        let first = first_word(rest);
        if let Some(path) = search(&unquote(&first)).filter(|p| p.to_ascii_lowercase().ends_with(".bat") || p.to_ascii_lowercase().ends_with(".cmd")) {
            let mut args = vec![first.clone()];
            args.extend(words(&rest[first.len()..]));
            return match self.batch(&path, args) {
                Flow::Exit(code) => Flow::Exit(code),
                _ => Flow::Next,
            };
        }
        // Anything else runs as itself, its `%` expanded a second time, as CALL does.
        let again = expand::percent(rest, None, self.errorlevel);
        let node = Parser::parse(&again);
        self.run(&node)
    }

    fn type_files(&mut self, args: &[String]) {
        let names: Vec<String> = args.iter().map(|a| unquote(a)).collect();
        if names.is_empty() {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        }
        let many = names.len() > 1 || names.iter().any(|n| n.contains(['*', '?']));
        for name in names {
            let files: Vec<String> = if name.contains(['*', '?']) {
                let dir = dir_part(&name).to_string();
                find(&name).iter().filter(|f| f.attributes & FILE_ATTRIBUTE_DIRECTORY == 0).map(|f| format!("{dir}{}", narrow(&f.name))).collect()
            } else {
                vec![name.clone()]
            };
            for file in files {
                match read_file(&file) {
                    Some(bytes) => {
                        if many {
                            err(&format!("\r\n{file}\r\n\r\n\r\n"));
                        }
                        write_bytes(&bytes);
                    }
                    None => self.fail(if is_dir(&file) { "Access is denied.\r\n" } else { NOT_FOUND }),
                }
            }
        }
    }

    fn mkdir(&mut self, args: &[String]) {
        if args.is_empty() {
            self.fail("The syntax of the command is incorrect.\r\n");
        }
        for a in args {
            let path = unquote(a);
            if attributes(&path).is_some() {
                self.fail(&format!("A subdirectory or file {path} already exists.\r\n"));
                continue;
            }
            // Each folder on the way, as extensions do.
            let mut so_far = String::new();
            for (i, part) in path.split(['\\', '/']).enumerate() {
                if i > 0 {
                    so_far.push('\\');
                }
                so_far.push_str(part);
                if part.is_empty() || part.ends_with(':') || attributes(&so_far).is_some() {
                    continue;
                }
                let w = wide(&so_far);
                if unsafe { CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) } == 0 {
                    self.fail("The system cannot find the path specified.\r\n");
                    break;
                }
            }
        }
    }

    fn rmdir(&mut self, args: &[String]) {
        let (sw, names) = switches(args);
        let tree = sw.iter().any(|s| s == "s");
        let quiet = sw.iter().any(|s| s == "q");
        for name in names {
            let path = unquote(&name);
            if !is_dir(&path) {
                self.fail(if attributes(&path).is_some() { "The directory name is invalid.\r\n" } else { NOT_FOUND });
                continue;
            }
            if tree {
                if !quiet && !ask_yes(&format!("{path}, Are you sure (Y/N)? ")) {
                    continue;
                }
                remove_tree(&path);
            }
            let w = wide(&path);
            if unsafe { RemoveDirectoryW(w.as_ptr()) } == 0 {
                let e = last_error();
                self.fail(if e == ERROR_DIR_NOT_EMPTY { "The directory is not empty.\r\n" } else { "Access is denied.\r\n" });
            }
        }
    }

    fn del(&mut self, args: &[String]) {
        let (sw, names) = switches(args);
        let quiet = sw.iter().any(|s| s == "q");
        let tree = sw.iter().any(|s| s == "s");
        if names.is_empty() {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        }
        for name in names {
            let mut pattern = unquote(&name);
            if is_dir(&pattern) {
                pattern = join(&pattern, "*");
            }
            let all = pattern.ends_with("*") && (pattern.ends_with("\\*") || pattern.ends_with("*.*") || pattern == "*");
            if all && !quiet && !ask_yes(&format!("{}, Are you sure (Y/N)? ", full_path(&pattern))) {
                continue;
            }
            let dirs: Vec<String> = if tree { tree_dirs(dir_part(&pattern)) } else { vec![dir_part(&pattern).to_string()] };
            let file_pattern = pattern[dir_part(&pattern).len()..].to_string();
            let mut deleted = 0;
            for dir in dirs {
                for f in find(&join(&dir, &file_pattern)) {
                    if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        continue;
                    }
                    let path = join(&dir, &narrow(&f.name));
                    let w = wide(&path);
                    if unsafe { DeleteFileW(w.as_ptr()) } != 0 {
                        deleted += 1;
                        if tree {
                            out(&format!("Deleted file - {}\r\n", full_path(&path)));
                        }
                    } else {
                        self.fail(&format!("{}\r\nAccess is denied.\r\n", full_path(&path)));
                    }
                }
            }
            if deleted == 0 && !all {
                err(&format!("Could Not Find {}\r\n", full_path(&pattern)));
            }
        }
    }

    fn copy(&mut self, args: &[String]) {
        let (_, names) = switches(args);
        let Some(source) = names.first().map(|s| unquote(s)) else {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        };
        let dest = names.get(1).map(|d| unquote(d)).unwrap_or_else(|| String::from("."));
        let sources: Vec<String> = if source.contains(['*', '?']) || is_dir(&source) {
            let pattern = if is_dir(&source) { join(&source, "*") } else { source.clone() };
            let dir = dir_part(&pattern).to_string();
            find(&pattern).iter().filter(|f| f.attributes & FILE_ATTRIBUTE_DIRECTORY == 0).map(|f| format!("{dir}{}", narrow(&f.name))).collect()
        } else {
            vec![source.clone()]
        };
        let many = sources.len() > 1 || source.contains(['*', '?']);
        let mut copied = 0;
        for s in &sources {
            let name = s.rsplit(['\\', '/']).next().unwrap_or(s);
            let target = if is_dir(&dest) { join(&dest, name) } else { dest.clone() };
            if many {
                out(&format!("{s}\r\n"));
            }
            let (from, to) = (wide(s), wide(&target));
            if unsafe { CopyFileW(from.as_ptr(), to.as_ptr(), 0) } != 0 {
                copied += 1;
            } else {
                self.fail(if attributes(s).is_none() { NOT_FOUND } else { "Access is denied.\r\n" });
            }
        }
        out(&format!("{:>9} file(s) copied.\r\n", copied));
    }

    fn move_files(&mut self, args: &[String]) {
        let (_, names) = switches(args);
        let (Some(source), Some(dest)) = (names.first().map(|s| unquote(s)), names.get(1).map(|d| unquote(d))) else {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        };
        let sources: Vec<String> = if source.contains(['*', '?']) {
            let dir = dir_part(&source).to_string();
            find(&source).iter().map(|f| format!("{dir}{}", narrow(&f.name))).filter(|n| !n.ends_with("\\.") && !n.ends_with("..")).collect()
        } else {
            vec![source.clone()]
        };
        let mut moved = 0;
        for s in &sources {
            let name = s.rsplit(['\\', '/']).next().unwrap_or(s);
            let target = if is_dir(&dest) { join(&dest, name) } else { dest.clone() };
            let (from, to) = (wide(s), wide(&target));
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED) } != 0 {
                moved += 1;
            } else {
                self.fail(if attributes(s).is_none() { NOT_FOUND } else { "Access is denied.\r\n" });
            }
        }
        if moved > 0 {
            out(&format!("{:>9} {}(s) moved.\r\n", moved, if sources.len() == 1 && is_dir(&dest) == false && attributes(&dest).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) { "dir" } else { "file" }));
        }
    }

    fn rename(&mut self, args: &[String]) {
        let (Some(source), Some(new)) = (args.first().map(|s| unquote(s)), args.get(1).map(|d| unquote(d))) else {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        };
        if new.contains(['\\', '/']) {
            self.fail("The syntax of the command is incorrect.\r\n");
            return;
        }
        let target = format!("{}{new}", dir_part(&source));
        let (from, to) = (wide(&source), wide(&target));
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            let e = last_error();
            self.fail(match e {
                ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS => "A duplicate file name exists, or the file\r\ncannot be found.\r\n",
                _ => NOT_FOUND,
            });
        }
    }

    fn mklink(&mut self, args: &[String]) {
        let (sw, names) = switches(args);
        let (Some(link), Some(target)) = (names.first().map(|s| unquote(s)), names.get(1).map(|d| unquote(d))) else {
            out("Creates a symbolic link.\r\n\r\nMKLINK [[/D] | [/H] | [/J]] Link Target\r\n");
            return;
        };
        let (l, t) = (wide(&link), wide(&target));
        let (ok, kind) = if sw.iter().any(|s| s == "h") {
            (unsafe { CreateHardLinkW(l.as_ptr(), t.as_ptr(), core::ptr::null_mut()) } != 0, "Hardlink")
        } else if sw.iter().any(|s| s == "j") {
            (unsafe { CreateSymbolicLinkW(l.as_ptr(), t.as_ptr(), SYMBOLIC_LINK_FLAG_DIRECTORY) } != 0, "Junction")
        } else {
            let flags = if sw.iter().any(|s| s == "d") { SYMBOLIC_LINK_FLAG_DIRECTORY } else { 0 };
            (unsafe { CreateSymbolicLinkW(l.as_ptr(), t.as_ptr(), flags) } != 0, "symbolic link")
        };
        if ok {
            out(&format!("{kind} created for {link} <<===>> {target}\r\n"));
        } else {
            self.fail(if last_error() == ERROR_ALREADY_EXISTS { "Cannot create a file when that file already exists.\r\n" } else { "Access is denied.\r\n" });
        }
    }

    fn start(&mut self, rest: &str) {
        let mut text = rest.trim_start().to_string();
        // A title first, in quotes.
        if text.starts_with('"') {
            if let Some(end) = text[1..].find('"') {
                text = text[end + 2..].trim_start().to_string();
            }
        }
        let mut wait_for = false;
        let mut dir = None;
        loop {
            let lower = text.to_ascii_lowercase();
            if lower.starts_with("/wait") {
                wait_for = true;
                text = text[5..].trim_start().to_string();
            } else if lower.starts_with("/d") {
                let after = text[2..].trim_start();
                let w = first_word(after);
                dir = Some(unquote(&w));
                text = after[w.len()..].trim_start().to_string();
            } else if lower.starts_with("/b") || lower.starts_with("/i") || lower.starts_with("/min") || lower.starts_with("/max") {
                let n = text.find([' ', '\t']).unwrap_or(text.len());
                text = text[n..].trim_start().to_string();
            } else {
                break;
            }
        }
        if text.is_empty() {
            text = format!("\"{}\"", self.exe);
        }
        let (path, line) = match self.program_for(&text) {
            Some(found) => found,
            None => (self.exe.clone(), format!("\"{}\" /K {text}", self.exe)),
        };
        let here = dir.map(|d| wide(&d));
        let child = spawn_in(&path, &line, unsafe { GetStdHandle(STD_INPUT) }, unsafe { GetStdHandle(STD_OUTPUT) }, unsafe { GetStdHandle(STD_ERROR) }, here.as_ref().map_or(core::ptr::null(), |w| w.as_ptr()));
        match child {
            Some(child) if wait_for => {
                self.errorlevel = wait(&child) as i32;
            }
            Some(_) => {}
            None => self.fail(&format!("The system cannot find the file {}.\r\n", first_word(&text))),
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Shell::new()
    }
}

/// The handles a redirection replaced, put back (and the ones it opened, closed).
fn restore(saved: Vec<(u32, HANDLE, bool)>) {
    for (which, old, opened) in saved.into_iter().rev() {
        let now = unsafe { GetStdHandle(which) };
        unsafe { SetStdHandle(which, old) };
        if opened {
            unsafe { CloseHandle(now) };
        }
    }
}

fn parse_number(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else if s.len() > 1 && s.starts_with('0') {
        i64::from_str_radix(&s[1..], 8).ok()?
    } else {
        s.parse().ok()?
    };
    Some(if neg { -n } else { n })
}

/// The first word of a command line: quoted, or up to a space or one of cmd's delimiters.
fn first_word(text: &str) -> String {
    let text = text.trim_start();
    if let Some(rest) = text.strip_prefix('"') {
        return match rest.find('"') {
            Some(end) => text[..end + 2].to_string(),
            None => text.to_string(),
        };
    }
    let end = text.find([' ', '\t', ',', ';', '=']).unwrap_or(text.len());
    text[..end].to_string()
}

/// A program as cmd finds it: as named if it has a path, else in the current folder then `PATH`'s; with `PATHEXT`'s
/// extensions tried when it has none.
pub fn search(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let pathext = get_env("PATHEXT").unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD"));
    let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let has_ext = file.contains('.') && exts.iter().any(|e| file.to_ascii_lowercase().ends_with(&e.to_ascii_lowercase()));
    let try_at = |base: &str| -> Option<String> {
        if has_ext || (file.contains('.') && attributes(base).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0)) {
            if attributes(base).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0) {
                return Some(full_path(base));
            }
            if has_ext {
                return None;
            }
        }
        exts.iter().map(|e| format!("{base}{}", e.to_ascii_lowercase())).find(|p| attributes(p).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0)).map(|p| full_path(&p))
    };
    if name.contains(['\\', '/', ':']) {
        return try_at(name);
    }
    if let Some(found) = try_at(&join(&current_dir(), name)) {
        return Some(found);
    }
    let path = get_env("PATH").unwrap_or_default();
    path.split(';').filter(|d| !d.is_empty()).find_map(|dir| try_at(&join(&unquote(dir), name)))
}

/// A started program: its process.
pub struct Child(PROCESS_INFORMATION);

/// Starts `path` with the command line `line` and these standard handles; the handles marked inheritable go with it.
pub fn spawn(path: &str, line: &str, stdin: HANDLE, stdout: HANDLE, stderr: HANDLE) -> Option<Child> {
    spawn_in(path, line, stdin, stdout, stderr, core::ptr::null())
}

fn spawn_in(path: &str, line: &str, stdin: HANDLE, stdout: HANDLE, stderr: HANDLE, dir: *const u16) -> Option<Child> {
    let app = wide(path);
    let mut line = wide(line);
    let mut si = STARTUPINFOW { flags: STARTF_USESTDHANDLES, stdin, stdout, stderr, ..STARTUPINFOW::default() };
    let mut pi = PROCESS_INFORMATION::default();
    let ok = unsafe { CreateProcessW(app.as_ptr(), line.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), TRUE, 0, core::ptr::null_mut(), dir, &mut si, &mut pi) };
    (ok != 0).then_some(Child(pi))
}

/// Waits for a started program to end; its exit code.
pub fn wait(child: &Child) -> u32 {
    unsafe { WaitForSingleObject(child.0.process, INFINITE) };
    let mut code = 0;
    unsafe { GetExitCodeProcess(child.0.process, &mut code) };
    unsafe {
        CloseHandle(child.0.process as HANDLE);
        CloseHandle(child.0.thread as HANDLE);
    }
    code
}

pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let w = wide(path);
    let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut data = Vec::new();
    let mut buffer = vec![0u8; 64 << 10];
    loop {
        let mut n = 0u32;
        if unsafe { ReadFile(h, buffer.as_mut_ptr(), buffer.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..n as usize]);
    }
    unsafe { CloseHandle(h) };
    Some(data)
}

/// A file's bytes to stdout: to the console as text, to a file or pipe as they are.
fn write_bytes(bytes: &[u8]) {
    let h = unsafe { GetStdHandle(STD_OUTPUT) };
    if is_console(h) {
        write_to(h, &String::from_utf8_lossy(bytes));
    } else {
        let mut done = 0;
        unsafe { WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut done, core::ptr::null_mut()) };
    }
}

/// A folder and every folder under it.
fn tree_dirs(root: &str) -> Vec<String> {
    let root = if root.is_empty() { String::from(".") } else { root.trim_end_matches(['\\', '/']).to_string() };
    let mut dirs = vec![root];
    let mut i = 0;
    while i < dirs.len() {
        let dir = dirs[i].clone();
        for f in find(&join(&dir, "*")) {
            let name = narrow(&f.name);
            if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && name != "." && name != ".." {
                dirs.push(join(&dir, &name));
            }
        }
        i += 1;
    }
    dirs
}

/// Everything under a folder, deleted (not the folder itself).
fn remove_tree(dir: &str) {
    for f in find(&join(dir, "*")) {
        let name = narrow(&f.name);
        if name == "." || name == ".." {
            continue;
        }
        let path = join(dir, &name);
        let w = wide(&path);
        if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && f.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            remove_tree(&path);
            unsafe { RemoveDirectoryW(w.as_ptr()) };
        } else if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            unsafe { RemoveDirectoryW(w.as_ptr()) };
        } else {
            unsafe {
                SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL);
                DeleteFileW(w.as_ptr());
            }
        }
    }
}
