//! findstr.exe: lines matching any of its search strings (space separated, or one with `/C:`), in files or stdin.
//! `/I` ignores case, `/V` inverts, `/N` numbers, `/B` and `/E` anchor, `/X` matches whole lines, `/M` lists only the
//! files, `/S` searches folders under the current one, `/R` reads the strings as findstr's regular expressions (`.`,
//! `*`, `^`, `$`, `[class]`, `\x` literals), `/L` literally.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use programs::*;

/// findstr's regular expressions: a match anywhere (or anchored by ^ and $).
fn regex(pattern: &[char], text: &[char]) -> bool {
    if let Some(rest) = pattern.strip_prefix(&['^']) {
        return here(rest, text);
    }
    (0..=text.len()).any(|i| here(pattern, &text[i..]))
}

/// One item: a character, `.`, a class, or `\\x`; and how many pattern characters it took.
fn item(p: &[char]) -> Option<(usize, fn(char, &[char]) -> bool, Vec<char>)> {
    match p.first()? {
        '.' => Some((1, |_, _| true, Vec::new())),
        '\\' if p.len() > 1 => Some((2, |c, set| c == set[0], alloc::vec![p[1]])),
        '[' => {
            let end = p.iter().position(|&c| c == ']')?;
            Some((end + 1, |c, set| {
                let (negate, set) = match set.first() {
                    Some('^') => (true, &set[1..]),
                    _ => (false, set),
                };
                let mut i = 0;
                let mut hit = false;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        hit |= set[i] <= c && c <= set[i + 2];
                        i += 3;
                    } else {
                        hit |= set[i] == c;
                        i += 1;
                    }
                }
                hit != negate
            }, p[1..end].to_vec()))
        }
        &c => Some((1, |x, set| x == set[0], alloc::vec![c])),
    }
}

fn here(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return true;
    }
    if p == ['$'] {
        return t.is_empty();
    }
    let Some((len, test, set)) = item(p) else { return false };
    if p.get(len) == Some(&'*') {
        let rest = &p[len + 1..];
        let mut i = 0;
        loop {
            if here(rest, &t[i..]) {
                return true;
            }
            if i < t.len() && test(t[i], &set) {
                i += 1;
            } else {
                return false;
            }
        }
    }
    !t.is_empty() && test(t[0], &set) && here(&p[len..], &t[1..])
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    let args = arguments();
    let (mut fold, mut invert, mut number, mut begin, mut end, mut whole, mut names, mut tree, mut re) = (false, false, false, false, false, false, false, false, false);
    let mut strings: Vec<String> = Vec::new();
    let mut given_string = false;
    let mut files = Vec::new();
    for a in &args {
        let lower = a.to_ascii_lowercase();
        if let Some(s) = a.strip_prefix("/C:").or_else(|| a.strip_prefix("/c:")) {
            strings.push(String::from(s));
            given_string = true;
            continue;
        }
        if lower.starts_with('/') && lower.len() >= 2 {
            for c in lower[1..].chars() {
                match c {
                    'i' => fold = true,
                    'v' => invert = true,
                    'n' => number = true,
                    'b' => begin = true,
                    'e' => end = true,
                    'x' => whole = true,
                    'm' => names = true,
                    's' => tree = true,
                    'r' => re = true,
                    'l' => re = false,
                    '?' => {
                        out("FINDSTR [/B] [/E] [/L] [/R] [/S] [/I] [/X] [/V] [/N] [/M] [/C:string] strings [files]\r\n");
                        exit(0);
                    }
                    _ => {}
                }
            }
            continue;
        }
        if !given_string && strings.is_empty() {
            strings.extend(a.split(' ').filter(|s| !s.is_empty()).map(String::from));
            given_string = true;
        } else if tree {
            files.push(a.clone());
        } else {
            files.extend(glob(a));
        }
    }
    if strings.is_empty() {
        err("FINDSTR: Bad command line\r\n");
        exit(2);
    }
    if tree {
        // Each pattern in the current folder and every one under it.
        let patterns = core::mem::take(&mut files);
        let mut dirs = alloc::vec![String::from(".")];
        let mut i = 0;
        while i < dirs.len() {
            let d = dirs[i].clone();
            for p in &patterns {
                for f in glob(&format!("{d}\\{p}")) {
                    files.push(String::from(f.trim_start_matches(".\\")));
                }
            }
            let w = wide(&format!("{d}\\*"));
            let mut data = WIN32_FIND_DATAW::default();
            let h = unsafe { FindFirstFileW(w.as_ptr(), &mut data) };
            if h != INVALID_HANDLE_VALUE {
                loop {
                    let n = narrow(&data.name);
                    if data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && n != "." && n != ".." {
                        dirs.push(format!("{d}\\{n}"));
                    }
                    if unsafe { FindNextFileW(h, &mut data) } == 0 {
                        break;
                    }
                }
                unsafe { FindClose(h) };
            }
            i += 1;
        }
    }
    let prepared: Vec<Vec<char>> = strings.iter().map(|s| if fold { s.to_ascii_lowercase() } else { s.clone() }.chars().collect()).collect();
    let matches = |line: &str| -> bool {
        let hay: Vec<char> = if fold { line.to_ascii_lowercase() } else { String::from(line) }.chars().collect();
        prepared.iter().any(|p| {
            if re {
                let mut pat = p.clone();
                if begin && pat.first() != Some(&'^') {
                    pat.insert(0, '^');
                }
                if (end || whole) && pat.last() != Some(&'$') {
                    pat.push('$');
                }
                if whole && pat.first() != Some(&'^') {
                    pat.insert(0, '^');
                }
                regex(&pat, &hay)
            } else if whole {
                &hay == p
            } else if begin && end {
                &hay == p
            } else if begin {
                hay.starts_with(p)
            } else if end {
                hay.ends_with(p)
            } else {
                hay.windows(p.len().max(1)).any(|w| w == p.as_slice()) || p.is_empty()
            }
        }) != invert
    };
    let many = files.len() > 1 || tree;
    let mut any = false;
    if files.is_empty() {
        for (i, line) in lines(&read_stdin()).iter().enumerate() {
            if matches(line) {
                any = true;
                if number {
                    out(&format!("{}:{line}\r\n", i + 1));
                } else {
                    out(&format!("{line}\r\n"));
                }
            }
        }
    }
    for f in &files {
        let Some(data) = read_file(f) else {
            err(&format!("FINDSTR: Cannot open {f}\r\n"));
            continue;
        };
        for (i, line) in lines(&data).iter().enumerate() {
            if matches(line) {
                any = true;
                if names {
                    out(&format!("{f}\r\n"));
                    break;
                }
                let prefix = match (many, number) {
                    (true, true) => format!("{f}:{}:", i + 1),
                    (true, false) => format!("{f}:"),
                    (false, true) => format!("{}:", i + 1),
                    (false, false) => String::new(),
                };
                out(&format!("{prefix}{line}\r\n"));
            }
        }
    }
    exit(if any { 0 } else { 1 })
}
