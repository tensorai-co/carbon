//! find.exe: lines with (or, `/V`, without) a string, in files or stdin; `/C` counts them, `/N` numbers them, `/I`
//! ignores case.

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
    let (mut invert, mut count, mut number, mut fold) = (false, false, false, false);
    let mut needle: Option<String> = None;
    let mut files = Vec::new();
    for a in &args {
        match a.to_ascii_lowercase().as_str() {
            "/v" => invert = true,
            "/c" => count = true,
            "/n" => number = true,
            "/i" => fold = true,
            "/?" => {
                out("FIND [/V] [/C] [/N] [/I] \"string\" [[drive:][path]filename[ ...]]\r\n");
                exit(0);
            }
            _ if needle.is_none() => needle = Some(a.clone()),
            _ => files.extend(glob(a)),
        }
    }
    let Some(needle) = needle else {
        err("FIND: Parameter format not correct\r\n");
        exit(2);
    };
    let wanted = if fold { needle.to_ascii_lowercase() } else { needle.clone() };
    let mut any = false;
    let mut search = |name: Option<&str>, data: &[u8]| {
        if let Some(n) = name {
            out(&format!("\r\n---------- {}", n.to_ascii_uppercase()));
            if !count {
                out("\r\n");
            }
        }
        let mut hits = 0;
        for (i, line) in lines(data).iter().enumerate() {
            let hay = if fold { line.to_ascii_lowercase() } else { line.clone() };
            if hay.contains(&wanted) != invert {
                hits += 1;
                any = true;
                if !count {
                    if number {
                        out(&format!("[{}]{line}\r\n", i + 1));
                    } else {
                        out(&format!("{line}\r\n"));
                    }
                }
            }
        }
        if count {
            out(&format!("{}{hits}\r\n", if name.is_some() { ": " } else { "" }));
        }
    };
    if files.is_empty() {
        search(None, &read_stdin());
    } else {
        for f in &files {
            match read_file(f) {
                Some(data) => search(Some(f), &data),
                None => err(&format!("File not found - {}\r\n", f.to_ascii_uppercase())),
            }
        }
    }
    exit(if any { 0 } else { 1 })
}
