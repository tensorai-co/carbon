//! where.exe: where programs (or any files) a pattern names are: in the current folder and each of `PATH`'s, with
//! `PATHEXT`'s extensions tried, or under a folder with `/R`. `/Q` says nothing and only sets the exit code.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use programs::*;

fn matches_in(dir: &str, pattern: &str, exts: &[String], found: &mut Vec<String>) {
    let dir = dir.trim_end_matches('\\');
    let has_ext = pattern.contains('.');
    let mut candidates = Vec::new();
    candidates.push(String::from(pattern));
    if !has_ext {
        for e in exts {
            candidates.push(format!("{pattern}{e}"));
        }
    }
    for c in candidates {
        for f in glob(&format!("{dir}\\{c}")) {
            // A name without wildcards is itself whether it's there or not: only one that's a file counts.
            let w = wide(&f);
            let a = unsafe { GetFileAttributesW(w.as_ptr()) };
            if a == INVALID_FILE_ATTRIBUTES || a & FILE_ATTRIBUTE_DIRECTORY != 0 {
                continue;
            }
            if !found.iter().any(|x| eq_ignore_case(x, &f)) {
                found.push(f);
            }
        }
    }
}

fn tree(dir: &str, pattern: &str, exts: &[String], found: &mut Vec<String>) {
    matches_in(dir, pattern, exts, found);
    let w = wide(&format!("{}\\*", dir.trim_end_matches('\\')));
    let mut data = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(w.as_ptr(), &mut data) };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let mut dirs = Vec::new();
    loop {
        let name = narrow(&data.name);
        if data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && name != "." && name != ".." {
            dirs.push(format!("{}\\{name}", dir.trim_end_matches('\\')));
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    for d in dirs {
        tree(&d, pattern, exts, found);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let args = arguments();
    let mut quiet = false;
    let mut root = None;
    let mut patterns = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].to_ascii_lowercase().as_str() {
            "/q" => quiet = true,
            "/r" => {
                i += 1;
                root = args.get(i).cloned();
            }
            "/?" => {
                out("WHERE [/R dir] [/Q] pattern...\r\n");
                exit(0);
            }
            _ => patterns.push(args[i].clone()),
        }
        i += 1;
    }
    if patterns.is_empty() {
        err("ERROR: A search pattern must be specified.\r\nType \"WHERE /?\" for usage.\r\n");
        exit(2);
    }
    let exts: Vec<String> = get_env("PATHEXT").unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD")).split(';').filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()).collect();
    let mut found = Vec::new();
    for pattern in &patterns {
        match &root {
            Some(r) => tree(r, pattern, &exts, &mut found),
            None => {
                matches_in(&current_dir(), pattern, &exts, &mut found);
                for dir in get_env("PATH").unwrap_or_default().split(';').filter(|d| !d.is_empty()) {
                    matches_in(&dir.replace('"', ""), pattern, &exts, &mut found);
                }
            }
        }
    }
    if found.is_empty() {
        if !quiet {
            err("INFO: Could not find files for the given pattern(s).\r\n");
        }
        exit(1);
    }
    if !quiet {
        for f in found {
            out(&format!("{f}\r\n"));
        }
    }
    exit(0)
}
