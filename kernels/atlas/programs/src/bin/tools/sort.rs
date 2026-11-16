//! sort.exe: lines sorted, case folded as Windows' sort does; `/R` reversed; `/+n` from the nth character; `/O file`
//! to a file.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use programs::*;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let args = arguments();
    let mut reverse = false;
    let mut column = 0usize;
    let mut output: Option<String> = None;
    let mut input = None;
    let mut i = 0;
    while i < args.len() {
        let lower = args[i].to_ascii_lowercase();
        if lower == "/r" || lower == "/reverse" {
            reverse = true;
        } else if let Some(n) = lower.strip_prefix("/+") {
            column = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
        } else if lower == "/o" || lower == "/output" {
            i += 1;
            output = args.get(i).cloned();
        } else if !lower.starts_with('/') {
            input = Some(args[i].clone());
        }
        i += 1;
    }
    let data = match &input {
        Some(f) => read_file(f).unwrap_or_else(|| {
            err("The system cannot find the file specified.\r\n");
            exit(1)
        }),
        None => read_stdin(),
    };
    let mut all: Vec<String> = lines(&data);
    all.sort_by_key(|l| l.chars().skip(column).collect::<String>().to_ascii_lowercase());
    if reverse {
        all.reverse();
    }
    let mut text = String::new();
    for l in all {
        text.push_str(&format!("{l}\r\n"));
    }
    match output {
        Some(path) => {
            let w = wide(&path);
            let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
            if h == INVALID_HANDLE_VALUE {
                err("Access is denied.\r\n");
                exit(1);
            }
            write_to(h, &text);
            unsafe { CloseHandle(h) };
        }
        None => out(&text),
    }
    exit(0)
}
