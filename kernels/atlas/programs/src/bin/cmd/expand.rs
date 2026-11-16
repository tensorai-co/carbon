//! `%` expansion, as cmd does it to a line (or block) before parsing it: variables, `%VAR:a=b%` and `%VAR:~n,m%`, and
//! in a batch file its arguments (`%1`, `%*`, `%~dp0`) and `%%` for `%`. And what's left for when a command runs: FOR's
//! variables with their `~` modifiers, and delayed `!VAR!`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use programs::*;

/// A dynamic variable's value, when no variable of its name is set.
pub fn dynamic(name: &str, errorlevel: i32) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "cd" => current_dir(),
        "errorlevel" => format!("{errorlevel}"),
        "date" => date_text(),
        "time" => time_text(true),
        "random" => format!("{}", (unsafe { GetTickCount64() }.wrapping_mul(1103515245).wrapping_add(12345) >> 8) % 32768),
        "cmdcmdline" => command_line(),
        "cmdextversion" => String::from("2"),
        _ => return None,
    })
}

pub fn variable(name: &str, errorlevel: i32) -> Option<String> {
    get_env(name).or_else(|| dynamic(name, errorlevel))
}

pub fn now() -> SYSTEMTIME {
    let mut t = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut t) };
    t
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// `%DATE%`, as en-US Windows writes it: `Mon 11/16/2026`.
pub fn date_text() -> String {
    let t = now();
    format!("{} {:02}/{:02}/{:04}", DAYS[t.day_of_week as usize % 7], t.month, t.day, t.year)
}

/// `%TIME%` (` 3:04:05.06`), or TIME /T's `03:04 PM`.
pub fn time_text(full: bool) -> String {
    let t = now();
    if full {
        format!("{:>2}:{:02}:{:02}.{:02}", t.hour, t.minute, t.second, t.milliseconds / 10)
    } else {
        let (h, pm) = match t.hour {
            0 => (12, false),
            1..=11 => (t.hour, false),
            12 => (12, true),
            h => (h - 12, true),
        };
        format!("{h:02}:{:02} {}", t.minute, if pm { "PM" } else { "AM" })
    }
}

/// A value with a variable's `:` modifier applied: `~start[,length]` or `find=replace` (`*find=` from the start).
fn modify(value: &str, modifier: &str) -> String {
    if let Some(range) = modifier.strip_prefix('~') {
        let chars: Vec<char> = value.chars().collect();
        let len = chars.len() as i64;
        let mut parts = range.splitn(2, ',');
        let start: i64 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let start = if start < 0 { (len + start).max(0) } else { start.min(len) };
        let end = match parts.next().and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(n) if n < 0 => (len + n).max(start),
            Some(n) => (start + n).min(len),
            None => len,
        };
        return chars[start as usize..end.max(start) as usize].iter().collect();
    }
    let Some((find, replace)) = modifier.split_once('=') else { return value.to_string() };
    if find.is_empty() {
        return value.to_string();
    }
    if let Some(find) = find.strip_prefix('*') {
        // Everything up to and including the first match.
        let lower = value.to_ascii_lowercase();
        return match lower.find(&find.to_ascii_lowercase()) {
            Some(at) => format!("{replace}{}", &value[at + find.len()..]),
            None => value.to_string(),
        };
    }
    replace_folded(value, find, replace)
}

/// Every match of `find` (case folded) replaced.
fn replace_folded(value: &str, find: &str, replace: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let find_lower = find.to_ascii_lowercase();
    let mut out = String::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find(&find_lower) {
        out.push_str(&value[at..at + i]);
        out.push_str(replace);
        at += i + find.len();
    }
    out.push_str(&value[at..]);
    out
}

/// `%` expansion of a line. `args` is a batch file's arguments (`%0` first), or None at the prompt, where a variable
/// that isn't set stays as written.
pub fn percent(text: &str, args: Option<&[String]>, errorlevel: i32) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        if let Some(args) = args {
            match chars.get(i + 1) {
                Some('%') => {
                    out.push('%');
                    i += 2;
                    continue;
                }
                Some(d) if d.is_ascii_digit() => {
                    out.push_str(args.get(d.to_digit(10).unwrap_or(0) as usize).map_or("", |s| s.as_str()));
                    i += 2;
                    continue;
                }
                Some('*') => {
                    out.push_str(&args.iter().skip(1).cloned().collect::<Vec<_>>().join(" "));
                    i += 2;
                    continue;
                }
                Some('~') => {
                    // %~[modifiers]n
                    let mut j = i + 2;
                    while j < chars.len() && chars[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if let Some(d) = chars.get(j).filter(|d| d.is_ascii_digit()) {
                        let mods: String = chars[i + 2..j].iter().collect();
                        let value = args.get(d.to_digit(10).unwrap_or(0) as usize).cloned().unwrap_or_default();
                        out.push_str(&tilde(&value, &mods));
                        i = j + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        // %NAME% or %NAME:modifier%
        if let Some(end) = chars[i + 1..].iter().position(|&c| c == '%') {
            let inner: String = chars[i + 1..i + 1 + end].iter().collect();
            if !inner.is_empty() && !inner.contains(['\n']) {
                let (name, modifier) = match inner.split_once(':') {
                    Some((n, m)) => (n, Some(m)),
                    None => (inner.as_str(), None),
                };
                match variable(name, errorlevel) {
                    Some(value) => {
                        out.push_str(&modifier.map_or(value.clone(), |m| modify(&value, m)));
                        i += end + 2;
                        continue;
                    }
                    None if args.is_some() => {
                        // In a batch file, a variable that isn't set is nothing.
                        i += end + 2;
                        continue;
                    }
                    None => {}
                }
            }
        }
        // A `%` alone: kept at the prompt; dropped in a batch file.
        if args.is_none() {
            out.push('%');
        }
        i += 1;
    }
    out
}

/// `!NAME!` expansion, with delayed expansion on.
pub fn delayed(text: &str, errorlevel: i32) -> String {
    if !text.contains('!') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('!') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('!') {
            Some(end) => {
                let inner = &after[..end];
                let (name, modifier) = match inner.split_once(':') {
                    Some((n, m)) => (n, Some(m)),
                    None => (inner, None),
                };
                if let Some(value) = variable(name, errorlevel) {
                    out.push_str(&modifier.map_or(value.clone(), |m| modify(&value, m)));
                }
                rest = &after[end + 1..];
            }
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// FOR's variables, `%v` and `%~[modifiers]v`, put in.
pub fn for_vars(text: &str, vars: &[(char, String)]) -> String {
    if vars.is_empty() || !text.contains('%') {
        return text.to_string();
    }
    let value = |v: char| vars.iter().rev().find(|(name, _)| *name == v).map(|(_, value)| value.clone());
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(&v) = chars.get(i + 1)
                && let Some(val) = value(v)
            {
                out.push_str(&val);
                i += 2;
                continue;
            }
            if chars.get(i + 1) == Some(&'~') {
                // The longest run of modifier letters whose next letter is a variable.
                let mut j = i + 2;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let mut found = None;
                for end in (i + 2..j).rev() {
                    if let Some(val) = value(chars[end]) {
                        found = Some((end, val));
                        break;
                    }
                }
                if let Some((end, val)) = found {
                    let mods: String = chars[i + 2..end].iter().collect();
                    out.push_str(&tilde(&val, &mods));
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A `~` modifier on a path: quotes taken off, then `f` (full path), `d` (drive), `p` (folder), `n` (name), `x`
/// (extension), `s` (short name, which is the name), `a` (attributes), `t` (time), `z` (size).
pub fn tilde(value: &str, mods: &str) -> String {
    let plain = value.trim_matches('"');
    if mods.is_empty() {
        return plain.to_string();
    }
    let lower = mods.to_ascii_lowercase();
    let full = full_path(plain);
    let (dir, file) = match full.rfind('\\') {
        Some(at) => (&full[..=at], &full[at + 1..]),
        None => ("", full.as_str()),
    };
    let (name, ext) = match file.rfind('.') {
        Some(at) if at > 0 => (&file[..at], &file[at..]),
        _ => (file, ""),
    };
    let drive = if full.len() >= 2 && full.as_bytes()[1] == b':' { &full[..2] } else { "" };
    let path = dir.strip_prefix(drive).unwrap_or(dir);
    let mut out = String::new();
    let mut info = None;
    let attrs = |info: &mut Option<WIN32_FIND_DATAW>| {
        if info.is_none() {
            let mut found = WIN32_FIND_DATAW::default();
            let w = wide(&full);
            let h = unsafe { FindFirstFileW(w.as_ptr(), &mut found) };
            if h != INVALID_HANDLE_VALUE {
                unsafe { FindClose(h) };
                *info = Some(found);
            }
        }
        *info
    };
    if lower.contains('a') {
        if let Some(f) = attrs(&mut info) {
            let a = f.attributes;
            let flag = |bit: u32, c: char| if a & bit != 0 { c } else { '-' };
            out.push_str(&format!("{}{}{}{}{}--------", flag(FILE_ATTRIBUTE_DIRECTORY, 'd'), flag(FILE_ATTRIBUTE_READONLY, 'r'), flag(FILE_ATTRIBUTE_ARCHIVE, 'a'), flag(FILE_ATTRIBUTE_HIDDEN, 'h'), flag(FILE_ATTRIBUTE_SYSTEM, 's')));
            out.push(' ');
        }
    }
    if lower.contains('t') {
        if let Some(f) = attrs(&mut info) {
            out.push_str(&crate::shell::file_time(f.written));
            out.push(' ');
        }
    }
    if lower.contains('z') {
        if let Some(f) = attrs(&mut info) {
            out.push_str(&format!("{} ", (u64::from(f.size_high) << 32) | u64::from(f.size_low)));
        }
    }
    if lower.contains('f') && !lower.contains(['d', 'p', 'n', 'x']) {
        out.push_str(&full);
        return String::from(out.trim_end());
    }
    if lower.contains('d') {
        out.push_str(drive);
    }
    if lower.contains('p') {
        out.push_str(path);
    }
    if lower.contains('n') {
        out.push_str(name);
    }
    if lower.contains('x') {
        out.push_str(ext);
    }
    String::from(out.trim_end())
}

/// A path made full, as GetFullPathName makes it.
pub fn full_path(path: &str) -> String {
    let w = wide(path);
    fill(|b, n| unsafe { GetFullPathNameW(w.as_ptr(), n, b, core::ptr::null_mut()) }).unwrap_or_else(|| path.to_string())
}
