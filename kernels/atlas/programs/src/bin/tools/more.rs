//! more.com: a file (or stdin) a screen at a time on a console, waiting for a key at `-- More --`; straight through
//! to a file or pipe. `+n` starts at line n.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use alloc::vec::Vec;
use programs::*;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let args = arguments();
    let mut skip = 0usize;
    let mut files = Vec::new();
    for a in &args {
        if let Some(n) = a.strip_prefix('+') {
            skip = n.parse().unwrap_or(0);
        } else if !a.starts_with('/') {
            files.extend(glob(a));
        }
    }
    let data = if files.is_empty() {
        read_stdin()
    } else {
        let mut d = Vec::new();
        for f in &files {
            match read_file(f) {
                Some(bytes) => d.extend(bytes),
                None => {
                    err(&format!("Cannot access file {f}\r\n"));
                    exit(1);
                }
            }
        }
        d
    };
    let all = lines(&data);
    let console = is_console(unsafe { GetStdHandle(STD_OUTPUT) });
    // A screen's worth before each prompt; the prompt's keys come from the console, not stdin (which may be the text).
    let page = 24usize;
    let conin = wide("CONIN$");
    let keys = unsafe { CreateFileW(conin.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, core::ptr::null_mut(), OPEN_EXISTING, 0, core::ptr::null_mut()) };
    for (n, line) in all.iter().skip(skip).enumerate() {
        out(&format!("{line}\r\n"));
        if console && keys != INVALID_HANDLE_VALUE && n % page == page - 1 && n + 1 < all.len() - skip {
            out("-- More --");
            let mut buffer = [0u16; 64];
            let mut read = 0u32;
            unsafe { ReadConsoleW(keys, buffer.as_mut_ptr(), 64, &mut read, core::ptr::null_mut()) };
            out("\r          \r");
            if read > 0 && (buffer[0] == b'q' as u16 || buffer[0] == b'Q' as u16) {
                break;
            }
        }
    }
    exit(0)
}
