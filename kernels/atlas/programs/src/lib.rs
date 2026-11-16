//! What every program here shares: the Win32 calls they make (imported from kernel32 by name, as any Windows program
//! imports them), memory from the process heap, UTF-16 and back, the console, and the command line. They're `no_std`
//! Rust, built by atlas's `build.rs` for ARM64, x86-64 and x86, and are ordinary Windows programs: atlas's system
//! layer has them in System32, and they run on atlas as any program does.

#![no_std]
#![allow(non_snake_case, clippy::missing_safety_doc)]

extern crate alloc;

mod crt;
pub mod sys;

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

pub use sys::*;

/// Memory from the process heap.
struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // The heap's blocks are 16-byte aligned on 64-bit Windows, 8 on 32-bit; more than that is over-allocated.
        if layout.align() <= 8 {
            unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
        } else {
            let raw = unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size() + layout.align()) };
            if raw.is_null() {
                return raw;
            }
            let aligned = ((raw as usize + core::mem::size_of::<usize>()).next_multiple_of(layout.align())) as *mut u8;
            unsafe { (aligned as *mut usize).sub(1).write(raw as usize) };
            aligned
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let raw = if layout.align() <= 8 { ptr } else { unsafe { (ptr as *mut usize).sub(1).read() as *mut u8 } };
        unsafe { HeapFree(GetProcessHeap(), 0, raw) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if layout.align() <= 8 {
            return unsafe { HeapReAlloc(GetProcessHeap(), 0, ptr, size.max(1)) };
        }
        let new = unsafe { self.alloc(Layout::from_size_align_unchecked(size, layout.align())) };
        if !new.is_null() {
            unsafe { core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(size)) };
            unsafe { self.dealloc(ptr, layout) };
        }
        new
    }
}

#[global_allocator]
static HEAP: Heap = Heap;

/// What MSVC's code generation looks for when floating point is used.
#[unsafe(no_mangle)]
pub static _fltused: i32 = 0;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut message = String::from("internal error: ");
    if let Some(location) = info.location() {
        message.push_str(location.file());
        message.push(':');
        message.push_str(&alloc::format!("{}", location.line()));
    }
    message.push_str("\r\n");
    err(&message);
    unsafe { ExitProcess(255) }
}

/// Ends the process.
pub fn exit(code: i32) -> ! {
    unsafe { ExitProcess(code as u32) }
}

// ---- UTF-16 ----

/// A string as Windows wants it: UTF-16, ended with a 0.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// UTF-16 to a string, up to its first 0.
pub fn narrow(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// A 0-ended UTF-16 string at `p`.
pub unsafe fn narrow_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) })
}

/// Calls a Win32 function that fills a UTF-16 buffer and gives its length (or the size it needs, if it's too small).
pub fn fill(mut call: impl FnMut(*mut u16, u32) -> u32) -> Option<String> {
    let mut size = 260u32;
    loop {
        let mut buffer = alloc::vec![0u16; size as usize];
        let n = call(buffer.as_mut_ptr(), size);
        if n == 0 {
            return None;
        }
        if n < size {
            buffer.truncate(n as usize);
            return Some(String::from_utf16_lossy(&buffer));
        }
        size = n + 1;
    }
}

// ---- The console and standard handles ----

pub const STD_INPUT: u32 = -10i32 as u32;
pub const STD_OUTPUT: u32 = -11i32 as u32;
pub const STD_ERROR: u32 = -12i32 as u32;

/// Whether `handle` is the console's (and not a file or pipe).
pub fn is_console(handle: HANDLE) -> bool {
    let mut mode = 0;
    unsafe { GetConsoleMode(handle, &mut mode) != 0 }
}

/// Writes `text` to a handle: through the console's own call if it's the console's, else as UTF-8.
pub fn write_to(handle: HANDLE, text: &str) {
    if is_console(handle) {
        let w: Vec<u16> = text.encode_utf16().collect();
        let mut done = 0;
        unsafe { WriteConsoleW(handle, w.as_ptr(), w.len() as u32, &mut done, core::ptr::null_mut()) };
    } else {
        let mut done = 0;
        unsafe { WriteFile(handle, text.as_ptr(), text.len() as u32, &mut done, core::ptr::null_mut()) };
    }
}

/// To stdout.
pub fn out(text: &str) {
    write_to(unsafe { GetStdHandle(STD_OUTPUT) }, text);
}

/// To stderr.
pub fn err(text: &str) {
    write_to(unsafe { GetStdHandle(STD_ERROR) }, text);
}

/// Where a line is read from: the console, a line at a time as it edits it, or a file or pipe, buffered.
pub struct Input {
    handle: HANDLE,
    pending: Vec<u8>,
    ended: bool,
}

impl Input {
    pub fn stdin() -> Input {
        Input { handle: unsafe { GetStdHandle(STD_INPUT) }, pending: Vec::new(), ended: false }
    }

    pub fn is_console(&self) -> bool {
        is_console(self.handle)
    }

    /// The next line, without its end; None at the input's end.
    pub fn line(&mut self) -> Option<String> {
        if self.is_console() {
            let mut buffer = [0u16; 4096];
            let mut read = 0u32;
            let ok = unsafe { ReadConsoleW(self.handle, buffer.as_mut_ptr(), buffer.len() as u32, &mut read, core::ptr::null_mut()) };
            if ok == 0 || read == 0 {
                return None;
            }
            let text = String::from_utf16_lossy(&buffer[..read as usize]);
            return Some(String::from(text.trim_end_matches(['\r', '\n'])));
        }
        loop {
            if let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=at).collect();
                let text = String::from_utf8_lossy(&line);
                return Some(String::from(text.trim_end_matches(['\r', '\n'])));
            }
            if self.ended {
                if self.pending.is_empty() {
                    return None;
                }
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                return Some(text);
            }
            let mut buffer = [0u8; 4096];
            let mut read = 0u32;
            let ok = unsafe { ReadFile(self.handle, buffer.as_mut_ptr(), buffer.len() as u32, &mut read, core::ptr::null_mut()) };
            if ok == 0 || read == 0 {
                self.ended = true;
            } else {
                self.pending.extend_from_slice(&buffer[..read as usize]);
            }
        }
    }
}

// ---- The command line ----

/// The process's command line, as it was given.
pub fn command_line() -> String {
    unsafe { narrow_ptr(GetCommandLineW()) }
}

/// The command line after the program's own name, as cmd and others read it: everything past the first argument
/// (quoted or not) and the spaces after it.
pub fn arguments_text() -> String {
    let line = command_line();
    let rest = line.trim_start();
    let rest = match rest.strip_prefix('"') {
        Some(r) => r.split_once('"').map_or("", |(_, after)| after),
        None => rest.find([' ', '\t']).map_or("", |i| &rest[i..]),
    };
    String::from(rest.trim_start_matches([' ', '\t']))
}

/// The arguments, split as the C runtime splits them (quotes, and backslashes before a quote).
pub fn arguments() -> Vec<String> {
    split_arguments(&arguments_text())
}

pub fn split_arguments(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
            chars.next();
        }
        if chars.peek().is_none() {
            return args;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if !quoted && (c == ' ' || c == '\t') {
                break;
            }
            chars.next();
            match c {
                '\\' => {
                    let mut slashes = 1;
                    while chars.peek() == Some(&'\\') {
                        chars.next();
                        slashes += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        arg.extend(core::iter::repeat_n('\\', slashes / 2));
                        if slashes % 2 == 1 {
                            chars.next();
                            arg.push('"');
                        }
                    } else {
                        arg.extend(core::iter::repeat_n('\\', slashes));
                    }
                }
                '"' => {
                    if quoted && chars.peek() == Some(&'"') {
                        chars.next();
                        arg.push('"');
                    } else {
                        quoted = !quoted;
                    }
                }
                c => arg.push(c),
            }
        }
        args.push(arg);
    }
}

// ---- Environment ----

pub fn get_env(name: &str) -> Option<String> {
    let w = wide(name);
    let mut buffer = alloc::vec![0u16; 1024];
    loop {
        unsafe { SetLastError(0) };
        let n = unsafe { GetEnvironmentVariableW(w.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
        if n == 0 {
            return if unsafe { GetLastError() } == 0 { Some(String::new()) } else { None };
        }
        if (n as usize) < buffer.len() {
            return Some(String::from_utf16_lossy(&buffer[..n as usize]));
        }
        buffer.resize(n as usize + 1, 0);
    }
}

pub fn set_env(name: &str, value: Option<&str>) -> bool {
    let n = wide(name);
    match value {
        Some(v) => {
            let v = wide(v);
            unsafe { SetEnvironmentVariableW(n.as_ptr(), v.as_ptr()) != 0 }
        }
        None => unsafe { SetEnvironmentVariableW(n.as_ptr(), core::ptr::null()) != 0 },
    }
}

/// Every variable, `NAME=value`, as the process has them (the `=C:` ones too).
pub fn all_env() -> Vec<String> {
    let block = unsafe { GetEnvironmentStringsW() };
    let mut vars = Vec::new();
    if block.is_null() {
        return vars;
    }
    let mut at = block;
    loop {
        let var = unsafe { narrow_ptr(at) };
        if var.is_empty() {
            break;
        }
        at = unsafe { at.add(var.encode_utf16().count() + 1) };
        vars.push(var);
    }
    unsafe { FreeEnvironmentStringsW(block) };
    vars
}

pub fn current_dir() -> String {
    fill(|b, n| unsafe { GetCurrentDirectoryW(n, b) }).unwrap_or_else(|| String::from("C:\\"))
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

// ---- Strings ----

pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.chars().zip(b.chars()).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// A number with thousands separators: 1,234,567.
pub fn grouped(n: u64) -> String {
    let digits = alloc::format!("{n}");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Whether `name` matches a Windows wildcard pattern (`*` and `?`, case folded).
pub fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
            // `?` at the end, or before a dot, can match nothing, as Windows' does.
            (Some('?'), None) => go(&p[1..], n),
            (Some(a), Some(b)) => a.to_ascii_lowercase() == b.to_ascii_lowercase() && go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // `*.*` is everything, with or without a dot.
    if pattern == "*.*" {
        return true;
    }
    go(&p, &n)
}

// ---- What the tools share ----

/// Everything stdin has, to its end.
pub fn read_stdin() -> Vec<u8> {
    let h = unsafe { GetStdHandle(STD_INPUT) };
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    if is_console(h) {
        let mut input = Input::stdin();
        while let Some(line) = input.line() {
            data.extend_from_slice(line.as_bytes());
            data.extend_from_slice(b"\r\n");
        }
        return data;
    }
    loop {
        let mut n = 0u32;
        if unsafe { ReadFile(h, buffer.as_mut_ptr(), buffer.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            return data;
        }
        data.extend_from_slice(&buffer[..n as usize]);
    }
}

/// A file's bytes.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let w = wide(path);
    let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut data = Vec::new();
    let mut buffer = alloc::vec![0u8; 64 << 10];
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

/// Text's lines, without their ends.
pub fn lines(data: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(data);
    let mut lines: Vec<String> = text.split('\n').map(|l| String::from(l.trim_end_matches('\r'))).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// The files a pattern names (`*` and `?` in its last part), with their folder as written.
pub fn glob(pattern: &str) -> Vec<String> {
    if !pattern.contains(['*', '?']) {
        return alloc::vec![String::from(pattern)];
    }
    let dir = match pattern.rfind(['\\', '/']) {
        Some(at) => &pattern[..=at],
        None => "",
    };
    let mut found = Vec::new();
    let w = wide(pattern);
    let mut data = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(w.as_ptr(), &mut data) };
    if h == INVALID_HANDLE_VALUE {
        return found;
    }
    loop {
        if data.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            found.push(alloc::format!("{dir}{}", narrow(&data.name)));
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    found
}

/// The computer's name.
pub fn computer_name() -> String {
    let mut buffer = [0u16; 256];
    let mut size = 256u32;
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return String::from("CARBON");
    }
    narrow(&buffer[..size as usize])
}

/// The user's name.
pub fn user_name() -> String {
    let mut buffer = [0u16; 256];
    let mut size = 256u32;
    if unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return String::from("user");
    }
    narrow(&buffer)
}
