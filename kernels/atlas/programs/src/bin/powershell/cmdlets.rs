//! The cmdlets, by their names, and Windows PowerShell's aliases for them; binding a command's arguments to their
//! parameters (a parameter may be named by any prefix that's unambiguous, as PowerShell allows); and programs: found on
//! `PATH` with `PATHEXT`'s extensions, run with their output going to the console, or captured as lines when a pipe,
//! an assignment or `$(...)` takes it.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use programs::*;

use crate::eval::{Flow, Out, Shell, error};
use crate::format as fmt;
use crate::parse::Parser;
use crate::value::{self, Value, compare, equal, like};

/// Each cmdlet's name as it's shown.
const CMDLETS: &[&str] = &[
    "Get-ChildItem", "Get-Item", "Set-Location", "Get-Location", "Push-Location", "Pop-Location", "Get-Content", "Set-Content", "Add-Content", "Clear-Content", "Out-File", "New-Item", "Remove-Item", "Copy-Item", "Move-Item", "Rename-Item",
    "Test-Path", "Resolve-Path", "Join-Path", "Split-Path", "Set-Item", "Write-Output", "Write-Host", "Write-Error", "Write-Warning", "Write-Verbose", "Write-Debug", "Write-Information", "Clear-Host", "ForEach-Object", "Where-Object",
    "Select-Object", "Sort-Object", "Measure-Object", "Group-Object", "Format-Table", "Format-List", "Format-Wide", "Out-String", "Out-Host", "Out-Null", "Out-Default", "Get-Date", "Start-Sleep", "Get-Command", "Get-Alias", "Set-Alias",
    "New-Alias", "Get-Variable", "Set-Variable", "New-Variable", "Remove-Variable", "Clear-Variable", "Get-Help", "Invoke-Expression", "Invoke-Command", "Start-Process", "Select-String", "Get-Member", "Read-Host", "Get-Random",
    "New-Object", "Tee-Object", "Get-Unique", "ConvertTo-Json", "Get-Host", "Get-Process", "Stop-Process", "mkdir",
];

pub fn display_name(lower: &str) -> String {
    CMDLETS.iter().find(|c| c.eq_ignore_ascii_case(lower)).map_or_else(|| lower.to_string(), |c| c.to_string())
}

pub fn exists(lower: &str) -> bool {
    CMDLETS.iter().any(|c| c.eq_ignore_ascii_case(lower))
}

/// Windows PowerShell 5.1's aliases.
pub fn aliases(a: &mut BTreeMap<String, String>) {
    let table: &[(&str, &str)] = &[
        ("dir", "get-childitem"), ("ls", "get-childitem"), ("gci", "get-childitem"), ("gi", "get-item"), ("cd", "set-location"), ("chdir", "set-location"), ("sl", "set-location"), ("pwd", "get-location"), ("gl", "get-location"),
        ("pushd", "push-location"), ("popd", "pop-location"), ("cat", "get-content"), ("type", "get-content"), ("gc", "get-content"), ("sc", "set-content"), ("ac", "add-content"), ("clc", "clear-content"), ("ni", "new-item"),
        ("md", "mkdir"), ("rm", "remove-item"), ("del", "remove-item"), ("erase", "remove-item"), ("rd", "remove-item"), ("rmdir", "remove-item"), ("ri", "remove-item"), ("cp", "copy-item"), ("copy", "copy-item"), ("cpi", "copy-item"),
        ("mv", "move-item"), ("move", "move-item"), ("mi", "move-item"), ("ren", "rename-item"), ("rni", "rename-item"), ("rvpa", "resolve-path"), ("si", "set-item"), ("echo", "write-output"), ("write", "write-output"), ("cls", "clear-host"),
        ("clear", "clear-host"), ("%", "foreach-object"), ("foreach", "foreach-object"), ("?", "where-object"), ("where", "where-object"), ("select", "select-object"), ("sort", "sort-object"), ("measure", "measure-object"),
        ("group", "group-object"), ("ft", "format-table"), ("fl", "format-list"), ("fw", "format-wide"), ("oh", "out-host"), ("sleep", "start-sleep"), ("gcm", "get-command"), ("gal", "get-alias"), ("sal", "set-alias"),
        ("nal", "new-alias"), ("gv", "get-variable"), ("set", "set-variable"), ("sv", "set-variable"), ("nv", "new-variable"), ("rv", "remove-variable"), ("clv", "clear-variable"), ("help", "get-help"), ("man", "get-help"),
        ("iex", "invoke-expression"), ("icm", "invoke-command"), ("start", "start-process"), ("saps", "start-process"), ("sls", "select-string"), ("gm", "get-member"), ("tee", "tee-object"), ("gu", "get-unique"), ("gps", "get-process"),
        ("ps", "get-process"), ("kill", "stop-process"), ("spps", "stop-process"),
    ];
    for (k, v) in table {
        a.insert(k.to_string(), v.to_string());
    }
}

/// A command's arguments, bound: named ones by any unambiguous prefix of their names, positional ones in order.
struct Bound {
    named: Vec<(String, Value)>,
    positional: Vec<Value>,
}

impl Bound {
    /// `switches` take no value; any other named parameter takes the value after it.
    fn new(args: Vec<(Option<String>, Option<Value>)>, switches: &[&str]) -> Bound {
        let mut named = Vec::new();
        let mut positional = Vec::new();
        let mut it = args.into_iter().peekable();
        while let Some((n, v)) = it.next() {
            match (n, v) {
                (Some(n), Some(v)) => named.push((n.to_ascii_lowercase(), v)),
                (Some(n), None) => {
                    let lower = n.to_ascii_lowercase();
                    let is_switch = switches.iter().any(|s| s.starts_with(&lower));
                    if !is_switch && matches!(it.peek(), Some((None, Some(_)))) {
                        let v = it.next().and_then(|(_, v)| v).unwrap_or(Value::Null);
                        named.push((lower, v));
                    } else {
                        named.push((lower, Value::Bool(true)));
                    }
                }
                (None, Some(v)) => positional.push(v),
                (None, None) => {}
            }
        }
        Bound { named, positional }
    }

    /// A named parameter's value, by any of its names (a given name that begins one counts).
    fn get(&self, names: &[&str]) -> Option<Value> {
        self.named.iter().find(|(n, _)| names.iter().any(|name| name.starts_with(n.as_str()) && !n.is_empty())).map(|(_, v)| v.clone())
    }

    fn has(&self, names: &[&str]) -> bool {
        self.get(names).is_some_and(|v| v.truthy())
    }

    fn pos(&self, n: usize) -> Option<Value> {
        self.positional.get(n).cloned()
    }

    /// A parameter by name, or else the `n`th positional value.
    fn arg(&self, names: &[&str], n: usize) -> Option<Value> {
        self.get(names).or_else(|| self.pos(n))
    }

    fn text(&self, names: &[&str], n: usize) -> Option<String> {
        self.arg(names, n).map(|v| v.to_string())
    }

}

pub fn attributes(path: &str) -> Option<u32> {
    let w = wide(path);
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    (a != INVALID_FILE_ATTRIBUTES).then_some(a)
}

pub fn full_path(path: &str) -> String {
    let path = if path == "~" || path.starts_with("~\\") || path.starts_with("~/") { format!("{}{}", get_env("USERPROFILE").unwrap_or_else(|| String::from("C:\\")), &path[1..]) } else { path.to_string() };
    let w = wide(&path);
    fill(|b, n| unsafe { GetFullPathNameW(w.as_ptr(), n, b, core::ptr::null_mut()) }).unwrap_or(path)
}

pub fn write_file(path: &str, data: &[u8], append: bool) -> Result<(), String> {
    let full = full_path(path);
    let w = wide(&full);
    let h = unsafe { CreateFileW(w.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ, core::ptr::null_mut(), if append { OPEN_ALWAYS } else { CREATE_ALWAYS }, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return Err(format!("Could not find a part of the path '{full}'."));
    }
    if append {
        unsafe { SetFilePointerEx(h, 0, core::ptr::null_mut(), FILE_END) };
    }
    let mut done = 0;
    unsafe { WriteFile(h, data.as_ptr(), data.len() as u32, &mut done, core::ptr::null_mut()) };
    unsafe { CloseHandle(h) };
    Ok(())
}

fn local_time(t: FILETIME) -> SYSTEMTIME {
    let mut local = FILETIME::default();
    let mut st = SYSTEMTIME::default();
    unsafe {
        FileTimeToLocalFileTime(&t, &mut local);
        FileTimeToSystemTime(&local, &mut st);
    }
    st
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A date in a .NET custom format: `yyyy MM dd HH mm ss tt` and their shorter and longer forms.
pub fn format_date(st: &SYSTEMTIME, f: &str) -> String {
    match f {
        "d" => return format!("{}/{}/{}", st.month, st.day, st.year),
        "D" => return format_date(st, "dddd, MMMM d, yyyy"),
        "t" => return format_date(st, "h:mm tt"),
        "T" => return format_date(st, "h:mm:ss tt"),
        "g" => return format_date(st, "M/d/yyyy h:mm tt"),
        "G" => return format_date(st, "M/d/yyyy h:mm:ss tt"),
        "s" => return format_date(st, "yyyy-MM-ddTHH:mm:ss"),
        "o" | "O" => return format_date(st, "yyyy-MM-ddTHH:mm:ss.fff0000"),
        "u" => return format_date(st, "yyyy-MM-dd HH:mm:ssZ"),
        _ => {}
    }
    let c: Vec<char> = f.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let h12 = match st.hour % 12 {
        0 => 12,
        h => h,
    };
    while i < c.len() {
        let ch = c[i];
        let mut n = 1;
        while i + n < c.len() && c[i + n] == ch {
            n += 1;
        }
        let piece = match (ch, n) {
            ('y', 1 | 2) => format!("{:02}", st.year % 100),
            ('y', _) => format!("{:04}", st.year),
            ('M', 1) => format!("{}", st.month),
            ('M', 2) => format!("{:02}", st.month),
            ('M', 3) => MONTHS[(st.month as usize + 11) % 12][..3].to_string(),
            ('M', _) => MONTHS[(st.month as usize + 11) % 12].to_string(),
            ('d', 1) => format!("{}", st.day),
            ('d', 2) => format!("{:02}", st.day),
            ('d', 3) => DAYS[st.day_of_week as usize % 7][..3].to_string(),
            ('d', _) => DAYS[st.day_of_week as usize % 7].to_string(),
            ('H', 1) => format!("{}", st.hour),
            ('H', _) => format!("{:02}", st.hour),
            ('h', 1) => format!("{h12}"),
            ('h', _) => format!("{h12:02}"),
            ('m', 1) => format!("{}", st.minute),
            ('m', _) => format!("{:02}", st.minute),
            ('s', 1) => format!("{}", st.second),
            ('s', _) => format!("{:02}", st.second),
            ('f', n) => format!("{:03}", st.milliseconds)[..n.min(3)].to_string(),
            ('t', 1) => String::from(if st.hour < 12 { "A" } else { "P" }),
            ('t', _) => String::from(if st.hour < 12 { "AM" } else { "PM" }),
            ('\\', _) if i + 1 < c.len() => {
                out.push(c[i + 1]);
                i += 2;
                continue;
            }
            (q @ ('\'' | '"'), _) => {
                let end = c[i + 1..].iter().position(|&x| x == q).map_or(c.len(), |e| i + 1 + e);
                out.extend(&c[i + 1..end]);
                i = end + 1;
                continue;
            }
            (other, n) => core::iter::repeat_n(other, n).collect(),
        };
        out.push_str(&piece);
        i += n;
    }
    out
}

fn date_value(st: SYSTEMTIME) -> Value {
    let text = format_date(&st, "dddd, MMMM d, yyyy h:mm:ss tt");
    Value::Object(Rc::new(value::Object {
        type_name: String::from("System.DateTime"),
        props: vec![
            (String::from("Year"), Value::Int(i64::from(st.year))),
            (String::from("Month"), Value::Int(i64::from(st.month))),
            (String::from("Day"), Value::Int(i64::from(st.day))),
            (String::from("Hour"), Value::Int(i64::from(st.hour))),
            (String::from("Minute"), Value::Int(i64::from(st.minute))),
            (String::from("Second"), Value::Int(i64::from(st.second))),
            (String::from("Millisecond"), Value::Int(i64::from(st.milliseconds))),
            (String::from("DayOfWeek"), Value::str(DAYS[st.day_of_week as usize % 7])),
            (String::from("DateTime"), Value::str(text.clone())),
        ],
        text: Some(text),
    }))
}

pub fn now_value() -> Value {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    date_value(st)
}

/// A file or folder, as Get-ChildItem gives one.
fn item(dir: &str, f: &WIN32_FIND_DATAW) -> Value {
    let name = narrow(&f.name);
    let full = if dir.ends_with('\\') { format!("{dir}{name}") } else { format!("{dir}\\{name}") };
    let is_dir = f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let a = f.attributes;
    let flag = |bit: u32, c: char| if a & bit != 0 { c } else { '-' };
    let mode = format!("{}{}{}{}{}{}", if is_dir { 'd' } else { '-' }, flag(FILE_ATTRIBUTE_ARCHIVE, 'a'), flag(FILE_ATTRIBUTE_READONLY, 'r'), flag(FILE_ATTRIBUTE_HIDDEN, 'h'), flag(FILE_ATTRIBUTE_SYSTEM, 's'), flag(FILE_ATTRIBUTE_REPARSE_POINT, 'l'));
    let st = local_time(f.written);
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    let mut props = vec![
        (String::from("PSPath"), Value::str(format!("Microsoft.PowerShell.Core\\FileSystem::{full}"))),
        (String::from("PSParentPath"), Value::str(dir.to_string())),
        (String::from("Mode"), Value::str(mode)),
        (String::from("LastWriteTime"), date_value(st)),
    ];
    if !is_dir {
        props.push((String::from("Length"), Value::Int(((u64::from(f.size_high) << 32) | u64::from(f.size_low)) as i64)));
    }
    props.extend([
        (String::from("Name"), Value::str(name.clone())),
        (String::from("FullName"), Value::str(full)),
        (String::from("BaseName"), Value::str(base)),
        (String::from("Extension"), Value::str(ext)),
        (String::from("DirectoryName"), Value::str(dir.to_string())),
        (String::from("PSIsContainer"), Value::Bool(is_dir)),
        (String::from("Attributes"), Value::str(if is_dir { "Directory" } else { "Archive" })),
    ]);
    Value::Object(Rc::new(value::Object { type_name: String::from(if is_dir { "System.IO.DirectoryInfo" } else { "System.IO.FileInfo" }), props, text: None }))
}

fn find_all(pattern: &str) -> Vec<WIN32_FIND_DATAW> {
    let mut found = Vec::new();
    let w = wide(pattern);
    let mut data = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(w.as_ptr(), &mut data) };
    if h == INVALID_HANDLE_VALUE {
        return found;
    }
    loop {
        let name = narrow(&data.name);
        if name != "." && name != ".." {
            found.push(data);
        }
        if unsafe { FindNextFileW(h, &mut data) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    found.sort_by_key(|f| (f.attributes & FILE_ATTRIBUTE_DIRECTORY == 0, narrow(&f.name).to_ascii_lowercase()));
    found
}

/// The paths a path names: itself, or with wildcards in its last part, each match.
fn resolve(path: &str) -> Vec<String> {
    let full = full_path(path);
    if !full.contains(['*', '?']) {
        return if attributes(&full).is_some() { vec![full] } else { Vec::new() };
    }
    let dir = full.rsplit_once('\\').map_or(String::new(), |(d, _)| d.to_string());
    find_all(&full).iter().map(|f| format!("{dir}\\{}", narrow(&f.name))).collect()
}

fn not_found(path: &str) -> Flow {
    error(&format!("Cannot find path '{}' because it does not exist.", full_path(path)))
}

fn env_entries(filter: &str) -> Vec<Value> {
    let mut vars: Vec<(String, String)> = all_env().into_iter().filter(|v| !v.starts_with('=')).filter_map(|v| v.split_once('=').map(|(k, x)| (k.to_string(), x.to_string()))).collect();
    vars.sort_by_key(|(k, _)| k.to_ascii_lowercase());
    vars.into_iter().filter(|(k, _)| filter.is_empty() || like(k, filter, false)).map(|(k, v)| Value::object("System.Collections.DictionaryEntry", vec![("Name", Value::str(k.clone())), ("Key", Value::str(k)), ("Value", Value::str(v))])).collect()
}

fn env_path(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    lower.strip_prefix("env:").map(|rest| path[path.len() - rest.len()..].trim_start_matches(['\\', '/']).to_string())
}

/// Lines of text, as Set-Content writes them.
fn text_of(values: &[Value], newline: bool) -> String {
    let mut s = String::new();
    for v in values {
        s.push_str(&v.to_string());
        if newline {
            s.push_str("\r\n");
        }
    }
    s
}

pub fn find_program(name: &str) -> Option<String> {
    let pathext = get_env("PATHEXT").unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD"));
    let mut exts: Vec<String> = pathext.split(';').filter(|e| !e.is_empty()).map(|e| e.to_ascii_lowercase()).collect();
    exts.push(String::from(".ps1"));
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let has_ext = file.contains('.');
    let try_at = |base: &str| -> Option<String> {
        if has_ext && attributes(base).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0) {
            return Some(full_path(base));
        }
        exts.iter().map(|e| format!("{base}{e}")).find(|p| attributes(p).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0)).map(|p| full_path(&p))
    };
    if name.contains(['\\', '/', ':']) {
        return try_at(name);
    }
    get_env("PATH").unwrap_or_default().split(';').filter(|d| !d.is_empty()).find_map(|dir| try_at(&format!("{}\\{name}", dir.trim_end_matches('\\'))))
}

/// A program, run: its arguments quoted as Windows programs read them; its output to the console, or captured as lines
/// when something takes it; input given to its stdin as lines.
pub fn run_program(sh: &mut Shell, path: &str, name: &str, words: &[String], input: Option<Vec<Value>>, merge: bool) -> Out {
    let quote = |w: &str| if w.is_empty() || w.contains([' ', '\t', '"']) { format!("\"{}\"", w.replace('"', "\\\"")) } else { w.to_string() };
    let mut line = quote(name);
    for w in words {
        line.push(' ');
        line.push_str(&quote(w));
    }
    let lower = path.to_ascii_lowercase();
    let (app, line) = if lower.ends_with(".bat") || lower.ends_with(".cmd") {
        let comspec = get_env("ComSpec").unwrap_or_else(|| String::from("C:\\Windows\\System32\\cmd.exe"));
        (comspec.clone(), format!("\"{comspec}\" /c \"{line}\""))
    } else {
        (path.to_string(), line)
    };
    let capture = sh.capturing > 0;
    let mut sa = SECURITY_ATTRIBUTES::inherited();
    let (mut out_read, mut out_write) = (core::ptr::null_mut(), core::ptr::null_mut());
    if capture {
        unsafe { CreatePipe(&mut out_read, &mut out_write, &mut sa, 0) };
        unsafe { SetHandleInformation(out_read, HANDLE_FLAG_INHERIT, 0) };
    }
    let input = input.filter(|i| !i.is_empty());
    let (mut in_read, mut in_write) = (core::ptr::null_mut(), core::ptr::null_mut());
    if input.is_some() {
        unsafe { CreatePipe(&mut in_read, &mut in_write, &mut sa, 0) };
        unsafe { SetHandleInformation(in_write, HANDLE_FLAG_INHERIT, 0) };
    }
    let stdout = if capture { out_write } else { unsafe { GetStdHandle(STD_OUTPUT) } };
    let stderr = if merge && capture { out_write } else { unsafe { GetStdHandle(STD_ERROR) } };
    let stdin = if input.is_some() { in_read } else { unsafe { GetStdHandle(STD_INPUT) } };
    let app_w = wide(&app);
    let mut line_w = wide(&line);
    let mut si = STARTUPINFOW { flags: STARTF_USESTDHANDLES, stdin, stdout, stderr, ..STARTUPINFOW::default() };
    let mut pi = PROCESS_INFORMATION::default();
    let ok = unsafe { CreateProcessW(app_w.as_ptr(), line_w.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), TRUE, 0, core::ptr::null_mut(), core::ptr::null(), &mut si, &mut pi) };
    if capture {
        unsafe { CloseHandle(out_write) };
    }
    if input.is_some() {
        unsafe { CloseHandle(in_read) };
    }
    if ok == 0 {
        if capture {
            unsafe { CloseHandle(out_read) };
        }
        return Err(error(&format!("Program '{name}' failed to run.")));
    }
    if let Some(input) = input {
        let text = text_of(&input, true);
        let mut done = 0;
        unsafe { WriteFile(in_write, text.as_ptr(), text.len() as u32, &mut done, core::ptr::null_mut()) };
        unsafe { CloseHandle(in_write) };
    }
    let mut out = Vec::new();
    if capture {
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut n = 0u32;
            if unsafe { ReadFile(out_read, buffer.as_mut_ptr(), buffer.len() as u32, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..n as usize]);
        }
        unsafe { CloseHandle(out_read) };
        out = lines(&data).into_iter().map(Value::Str).collect();
    }
    unsafe { WaitForSingleObject(pi.process, INFINITE) };
    let mut code = 0u32;
    unsafe { GetExitCodeProcess(pi.process, &mut code) };
    unsafe {
        CloseHandle(pi.process as HANDLE);
        CloseHandle(pi.thread as HANDLE);
    }
    sh.last_exit = code as i32;
    sh.ok = code == 0;
    Ok(out)
}

/// A property's value, or a calculated one's (`@{Name=...; Expression={...}}`).
fn select_value(sh: &mut Shell, item: &Value, prop: &Value) -> Result<(String, Value), Flow> {
    match prop {
        Value::Hash(h) => {
            let h = h.borrow().clone();
            let get = |names: &[&str]| h.iter().find(|(k, _)| names.iter().any(|n| n.eq_ignore_ascii_case(k))).map(|(_, v)| v.clone());
            let name = get(&["name", "n", "label", "l"]).map(|v| v.to_string()).unwrap_or_default();
            let value = match get(&["expression", "e"]) {
                Some(Value::Block(b)) => Value::from_items(sh.call(&b, Vec::new(), Vec::new(), Some(item.clone()), true)?),
                Some(other) => item.property(&other.to_string()),
                None => Value::Null,
            };
            Ok((name, value))
        }
        Value::Block(b) => Ok((b.text.trim().to_string(), Value::from_items(sh.call(b, Vec::new(), Vec::new(), Some(item.clone()), true)?))),
        other => {
            let name = other.to_string();
            Ok((name.clone(), item.property(&name)))
        }
    }
}

/// The names `-Property` asks for, `*` taking every property the item has.
fn property_names(item: &Value, wanted: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for w in wanted {
        match w {
            Value::Str(s) if s.contains(['*', '?']) => {
                if let Value::Object(o) = item {
                    out.extend(o.props.iter().filter(|(k, _)| like(k, s, false) && !k.starts_with("PS")).map(|(k, _)| Value::str(k.clone())));
                } else if let Value::Hash(h) = item {
                    out.extend(h.borrow().iter().filter(|(k, _)| like(k, s, false)).map(|(k, _)| Value::str(k.clone())));
                }
            }
            other => out.push(other.clone()),
        }
    }
    out
}

fn sort_key(sh: &mut Shell, item: &Value, props: &[Value]) -> Result<Vec<Value>, Flow> {
    if props.is_empty() {
        return Ok(vec![item.clone()]);
    }
    let mut keys = Vec::new();
    for p in props {
        keys.push(select_value(sh, item, p)?.1);
    }
    Ok(keys)
}

pub fn run(sh: &mut Shell, name: &str, args: Vec<(Option<String>, Option<Value>)>, input: Vec<Value>) -> Out {
    match name {
        "get-childitem" => {
            let b = Bound::new(args, &["recurse", "name", "file", "directory", "force", "hidden"]);
            let paths: Vec<String> = match b.arg(&["path", "literalpath"], 0) {
                Some(v) => v.items().iter().map(|p| p.to_string()).collect(),
                None => vec![String::from(".")],
            };
            let filter = b.text(&["filter", "include"], 1);
            let (recurse, names, files, dirs, force) = (b.has(&["recurse"]), b.has(&["name"]), b.has(&["file"]), b.has(&["directory"]), b.has(&["force", "hidden"]));
            let mut out = Vec::new();
            for path in paths {
                if let Some(rest) = env_path(&path) {
                    out.extend(env_entries(&rest));
                    continue;
                }
                let full = full_path(&path);
                let (dir, pattern) = if attributes(&full).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) {
                    (full.clone(), filter.clone().unwrap_or_else(|| String::from("*")))
                } else if full.contains(['*', '?']) {
                    let (d, p) = full.rsplit_once('\\').unwrap_or((".", full.as_str()));
                    (d.to_string(), p.to_string())
                } else if attributes(&full).is_some() {
                    let (d, _) = full.rsplit_once('\\').unwrap_or((".", ""));
                    let found = find_all(&full);
                    out.extend(found.iter().map(|f| item(d, f)));
                    continue;
                } else {
                    return Err(not_found(&path));
                };
                let mut stack = vec![dir];
                while let Some(d) = stack.pop() {
                    let base = d.trim_end_matches('\\');
                    for f in find_all(&format!("{base}\\{pattern}")) {
                        let is_dir = f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                        if !force && f.attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
                            continue;
                        }
                        if (files && is_dir) || (dirs && !is_dir) {
                            continue;
                        }
                        if names {
                            let full_name = format!("{base}\\{}", narrow(&f.name));
                            out.push(Value::str(full_name.strip_prefix(&format!("{}\\", full_path(&path).trim_end_matches('\\'))).unwrap_or(&full_name).to_string()));
                        } else {
                            out.push(item(base, &f));
                        }
                    }
                    if recurse {
                        let mut subs: Vec<String> = find_all(&format!("{base}\\*")).iter().filter(|f| f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0).map(|f| format!("{base}\\{}", narrow(&f.name))).collect();
                        subs.reverse();
                        stack.extend(subs);
                    }
                }
            }
            Ok(out)
        }
        "get-item" => {
            let b = Bound::new(args, &["force"]);
            let mut out = Vec::new();
            for p in b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or_default() {
                let path = p.to_string();
                if let Some(rest) = env_path(&path) {
                    let found = env_entries(&rest);
                    if found.is_empty() {
                        return Err(error(&format!("Cannot find path 'Env:\\{rest}' because it does not exist.")));
                    }
                    out.extend(found);
                    continue;
                }
                let resolved = resolve(&path);
                if resolved.is_empty() {
                    return Err(not_found(&path));
                }
                for r in resolved {
                    let (d, _) = r.rsplit_once('\\').unwrap_or((".", ""));
                    let d = if d.ends_with(':') { format!("{d}\\") } else { d.to_string() };
                    out.extend(find_all(&r).iter().map(|f| item(&d, f)));
                    if r.ends_with(":\\") {
                        out.push(Value::object("System.IO.DirectoryInfo", vec![("Mode", Value::str("d-----")), ("Name", Value::str(r.clone())), ("FullName", Value::str(r.clone())), ("PSIsContainer", Value::Bool(true))]));
                    }
                }
            }
            Ok(out)
        }
        "set-location" => {
            let b = Bound::new(args, &["passthru"]);
            let target = b.text(&["path", "literalpath"], 0).unwrap_or_else(|| get_env("USERPROFILE").unwrap_or_else(|| String::from("C:\\")));
            let target = if target == "-" { sh.locations.last().cloned().unwrap_or_else(current_dir) } else { target };
            let full = full_path(&target);
            let w = wide(&full);
            if unsafe { SetCurrentDirectoryW(w.as_ptr()) } == 0 {
                return Err(not_found(&target));
            }
            Ok(Vec::new())
        }
        "get-location" => Ok(vec![Value::object("System.Management.Automation.PathInfo", vec![("Path", Value::str(current_dir()))])]),
        "push-location" => {
            let b = Bound::new(args, &[]);
            sh.locations.push(current_dir());
            if let Some(target) = b.text(&["path", "literalpath"], 0) {
                let w = wide(&full_path(&target));
                if unsafe { SetCurrentDirectoryW(w.as_ptr()) } == 0 {
                    sh.locations.pop();
                    return Err(not_found(&target));
                }
            }
            Ok(Vec::new())
        }
        "pop-location" => {
            if let Some(d) = sh.locations.pop() {
                let w = wide(&d);
                unsafe { SetCurrentDirectoryW(w.as_ptr()) };
            }
            Ok(Vec::new())
        }
        "get-content" => {
            let b = Bound::new(args, &["raw", "wait"]);
            let mut out = Vec::new();
            for p in b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or_default() {
                let path = p.to_string();
                if let Some(rest) = env_path(&path) {
                    out.extend(get_env(&rest).map(Value::Str));
                    continue;
                }
                let resolved = resolve(&path);
                if resolved.is_empty() {
                    return Err(not_found(&path));
                }
                for r in resolved {
                    let data = read_file(&r).ok_or_else(|| error(&format!("Access to the path '{r}' is denied.")))?;
                    if b.has(&["raw"]) {
                        out.push(Value::str(String::from_utf8_lossy(&data).into_owned()));
                        continue;
                    }
                    let mut all: Vec<Value> = lines(&data).into_iter().map(Value::Str).collect();
                    if let Some(n) = b.get(&["totalcount", "first", "head"]).and_then(|v| v.int()) {
                        all.truncate(n.max(0) as usize);
                    }
                    if let Some(n) = b.get(&["tail", "last"]).and_then(|v| v.int()) {
                        let skip = all.len().saturating_sub(n.max(0) as usize);
                        all.drain(..skip);
                    }
                    out.extend(all);
                }
            }
            Ok(out)
        }
        "set-content" | "add-content" => {
            let b = Bound::new(args, &["nonewline", "force", "passthru"]);
            let path = b.text(&["path", "literalpath"], 0).ok_or_else(|| error("Cannot bind argument to parameter 'Path' because it is null."))?;
            let values = b.arg(&["value"], 1).map(|v| v.items()).unwrap_or(input);
            if let Some(rest) = env_path(&path) {
                set_env(&rest, Some(&text_of(&values, false)));
                return Ok(Vec::new());
            }
            let text = text_of(&values, !b.has(&["nonewline"]));
            write_file(&path, text.as_bytes(), name == "add-content").map_err(|e| error(&e))?;
            Ok(Vec::new())
        }
        "clear-content" => {
            let b = Bound::new(args, &[]);
            let path = b.text(&["path"], 0).unwrap_or_default();
            write_file(&path, b"", false).map_err(|e| error(&e))?;
            Ok(Vec::new())
        }
        "out-file" => {
            let b = Bound::new(args, &["append", "nonewline", "force", "noclobber"]);
            let path = b.text(&["filepath", "path", "literalpath"], 0).ok_or_else(|| error("Cannot bind argument to parameter 'FilePath'."))?;
            let values = b.arg(&["inputobject"], 1).map(|v| v.items()).unwrap_or(input);
            write_file(&path, fmt::to_text(&values).as_bytes(), b.has(&["append"])).map_err(|e| error(&e))?;
            Ok(Vec::new())
        }
        "new-item" | "mkdir" => {
            let b = Bound::new(args, &["force"]);
            let path = b.text(&["path"], 0).unwrap_or_else(|| String::from("."));
            let path = match b.text(&["name"], usize::MAX) {
                Some(n) => format!("{}\\{n}", path.trim_end_matches('\\')),
                None => path,
            };
            if let Some(rest) = env_path(&path) {
                set_env(&rest, Some(&b.text(&["value"], 1).unwrap_or_default()));
                return Ok(Vec::new());
            }
            let kind = if name == "mkdir" { String::from("directory") } else { b.text(&["itemtype", "type"], usize::MAX).unwrap_or_else(|| String::from("file")).to_ascii_lowercase() };
            let full = full_path(&path);
            let exists = attributes(&full).is_some();
            if exists && !b.has(&["force"]) {
                return Err(error(&format!("An item with the specified name {full} already exists.")));
            }
            if kind.starts_with('d') {
                // Each folder on the way.
                let mut so_far = String::new();
                for (i, part) in full.split('\\').enumerate() {
                    if i > 0 {
                        so_far.push('\\');
                    }
                    so_far.push_str(part);
                    if part.is_empty() || part.ends_with(':') || attributes(&so_far).is_some() {
                        continue;
                    }
                    let w = wide(&so_far);
                    if unsafe { CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) } == 0 {
                        return Err(error(&format!("Could not find a part of the path '{full}'.")));
                    }
                }
            } else {
                let value = b.text(&["value"], 1).unwrap_or_default();
                write_file(&full, value.as_bytes(), false).map_err(|e| error(&e))?;
            }
            let (d, _) = full.rsplit_once('\\').unwrap_or((".", ""));
            Ok(find_all(&full).iter().map(|f| item(d, f)).collect())
        }
        "remove-item" => {
            let b = Bound::new(args, &["recurse", "force", "whatif", "confirm"]);
            let paths: Vec<Value> = b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or(input);
            for p in paths {
                let path = match &p {
                    Value::Object(_) => p.property("FullName").to_string(),
                    other => other.to_string(),
                };
                if let Some(rest) = env_path(&path) {
                    set_env(&rest, None);
                    continue;
                }
                let resolved = resolve(&path);
                if resolved.is_empty() {
                    return Err(not_found(&path));
                }
                for r in resolved {
                    let a = attributes(&r).unwrap_or(0);
                    let w = wide(&r);
                    if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        let has_children = !find_all(&format!("{r}\\*")).is_empty();
                        if has_children && !b.has(&["recurse"]) {
                            return Err(error(&format!("The item at {r} has children and the Recurse parameter was not specified.")));
                        }
                        remove_tree(&r);
                        if unsafe { RemoveDirectoryW(w.as_ptr()) } == 0 {
                            return Err(error(&format!("Cannot remove item {r}: The directory is not empty.")));
                        }
                    } else {
                        if b.has(&["force"]) {
                            unsafe { SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL) };
                        }
                        if unsafe { DeleteFileW(w.as_ptr()) } == 0 {
                            return Err(error(&format!("Cannot remove item {r}: Access to the path is denied.")));
                        }
                    }
                }
            }
            Ok(Vec::new())
        }
        "copy-item" | "move-item" => {
            let b = Bound::new(args, &["recurse", "force", "passthru"]);
            let paths: Vec<Value> = b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or(input);
            let dest = b.text(&["destination"], 1).ok_or_else(|| error("Cannot bind argument to parameter 'Destination'."))?;
            let dest_full = full_path(&dest);
            for p in paths {
                let path = match &p {
                    Value::Object(_) => p.property("FullName").to_string(),
                    other => other.to_string(),
                };
                let resolved = resolve(&path);
                if resolved.is_empty() {
                    return Err(not_found(&path));
                }
                for r in resolved {
                    let leaf = r.rsplit('\\').next().unwrap_or("").to_string();
                    let target = if attributes(&dest_full).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) { format!("{}\\{leaf}", dest_full.trim_end_matches('\\')) } else { dest_full.clone() };
                    let is_dir = attributes(&r).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0);
                    let (from, to) = (wide(&r), wide(&target));
                    let ok = if name == "move-item" {
                        unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_COPY_ALLOWED | if b.has(&["force"]) { MOVEFILE_REPLACE_EXISTING } else { 0 }) != 0 }
                    } else if is_dir {
                        copy_tree(&r, &target, b.has(&["recurse"]))
                    } else {
                        unsafe { CopyFileW(from.as_ptr(), to.as_ptr(), 0) != 0 }
                    };
                    if !ok {
                        return Err(error(&format!("Cannot {} item {r} to {target}.", if name == "move-item" { "move" } else { "copy" })));
                    }
                }
            }
            Ok(Vec::new())
        }
        "rename-item" => {
            let b = Bound::new(args, &["force", "passthru"]);
            let path = b.text(&["path", "literalpath"], 0).ok_or_else(|| error("Cannot bind argument to parameter 'Path'."))?;
            let new = b.text(&["newname"], 1).ok_or_else(|| error("Cannot bind argument to parameter 'NewName'."))?;
            let full = full_path(&path);
            let dir = full.rsplit_once('\\').map_or(String::new(), |(d, _)| d.to_string());
            let target = if new.contains('\\') { new.clone() } else { format!("{dir}\\{new}") };
            let (from, to) = (wide(&full), wide(&target));
            if attributes(&full).is_none() {
                return Err(not_found(&path));
            }
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
                return Err(error(&format!("Cannot rename because item at '{full}' does not exist or '{new}' already exists.")));
            }
            Ok(Vec::new())
        }
        "test-path" => {
            let b = Bound::new(args, &["isvalid"]);
            let kind = b.text(&["pathtype"], usize::MAX).map(|k| k.to_ascii_lowercase());
            let mut out = Vec::new();
            for p in b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or(input) {
                let path = p.to_string();
                if let Some(rest) = env_path(&path) {
                    out.push(Value::Bool(rest.is_empty() || get_env(&rest).is_some()));
                    continue;
                }
                let resolved = resolve(&path);
                let ok = resolved.iter().any(|r| {
                    let a = attributes(r).unwrap_or(0);
                    match kind.as_deref() {
                        Some("leaf") => a & FILE_ATTRIBUTE_DIRECTORY == 0,
                        Some("container") => a & FILE_ATTRIBUTE_DIRECTORY != 0,
                        _ => true,
                    }
                });
                out.push(Value::Bool(ok));
            }
            Ok(out)
        }
        "resolve-path" => {
            let b = Bound::new(args, &["relative"]);
            let mut out = Vec::new();
            for p in b.arg(&["path", "literalpath"], 0).map(|v| v.items()).unwrap_or(input) {
                let resolved = resolve(&p.to_string());
                if resolved.is_empty() {
                    return Err(not_found(&p.to_string()));
                }
                out.extend(resolved.into_iter().map(|r| Value::object("System.Management.Automation.PathInfo", vec![("Path", Value::str(r))])));
            }
            Ok(out)
        }
        "join-path" => {
            let b = Bound::new(args, &["resolve"]);
            let mut path = b.text(&["path"], 0).unwrap_or_default();
            let mut parts: Vec<String> = b.get(&["childpath"]).map(|v| v.items().iter().map(|i| i.to_string()).collect()).unwrap_or_default();
            parts.extend(b.positional.iter().skip(if b.get(&["path"]).is_some() { 0 } else { 1 }).map(|v| v.to_string()));
            for part in parts {
                path = format!("{}\\{}", path.trim_end_matches(['\\', '/']), part.trim_start_matches(['\\', '/']));
            }
            Ok(vec![Value::str(path)])
        }
        "split-path" => {
            let b = Bound::new(args, &["parent", "leaf", "qualifier", "noqualifier", "isabsolute", "extension", "leafbase"]);
            let path = b.text(&["path", "literalpath"], 0).or_else(|| input.first().map(|v| v.to_string())).unwrap_or_default();
            let trimmed = path.trim_end_matches(['\\', '/']);
            let leaf = trimmed.rsplit(['\\', '/']).next().unwrap_or("").to_string();
            let v = if b.has(&["leaf"]) {
                leaf
            } else if b.has(&["leafbase"]) {
                leaf.rfind('.').map_or(leaf.clone(), |i| leaf[..i].to_string())
            } else if b.has(&["extension"]) {
                leaf.rfind('.').map_or(String::new(), |i| leaf[i..].to_string())
            } else if b.has(&["qualifier"]) {
                path.split_once(':').map_or(String::new(), |(q, _)| format!("{q}:"))
            } else if b.has(&["noqualifier"]) {
                path.split_once(':').map_or(path.clone(), |(_, r)| r.to_string())
            } else if b.has(&["isabsolute"]) {
                return Ok(vec![Value::Bool(path.contains(':') || path.starts_with('\\'))]);
            } else {
                match trimmed.rfind(['\\', '/']) {
                    Some(i) if trimmed[..i].ends_with(':') => trimmed[..=i].to_string(),
                    Some(i) => trimmed[..i].to_string(),
                    None => String::new(),
                }
            };
            Ok(vec![Value::str(v)])
        }
        "set-item" => {
            let b = Bound::new(args, &["force"]);
            let path = b.text(&["path"], 0).unwrap_or_default();
            let value = b.text(&["value"], 1).unwrap_or_default();
            match env_path(&path) {
                Some(rest) => {
                    set_env(&rest, if value.is_empty() { None } else { Some(&value) });
                    Ok(Vec::new())
                }
                None => Err(error("Set-Item only sets environment variables here (Env:\\NAME).")),
            }
        }
        "write-output" => {
            let mut out = input;
            for (n, v) in args {
                if n.is_some_and(|n| "noenumerate".starts_with(&n.to_ascii_lowercase())) {
                    continue;
                }
                if let Some(v) = v {
                    out.extend(v.items());
                }
            }
            Ok(out)
        }
        "write-host" => {
            let b = Bound::new(args, &["nonewline"]);
            let sep = b.text(&["separator"], usize::MAX).unwrap_or_else(|| String::from(" "));
            let mut items: Vec<String> = b.get(&["object"]).map(|v| vec![v.to_string()]).unwrap_or_default();
            items.extend(b.positional.iter().map(|v| v.to_string()));
            items.extend(input.iter().map(|v| v.to_string()));
            let mut text = items.join(&sep);
            let color = b.text(&["foregroundcolor"], usize::MAX).and_then(|c| vt_color(&c));
            if let Some(c) = color.filter(|_| is_console(unsafe { GetStdHandle(STD_OUTPUT) })) {
                text = format!("\x1b[{c}m{text}\x1b[0m");
            }
            if !b.has(&["nonewline"]) {
                text.push_str("\r\n");
            }
            out(&text);
            Ok(Vec::new())
        }
        "write-error" => {
            let b = Bound::new(args, &[]);
            let message = b.text(&["message"], 0).or_else(|| input.first().map(|v| v.to_string())).unwrap_or_default();
            Err(error(&message))
        }
        "write-warning" => {
            let b = Bound::new(args, &[]);
            let message = b.text(&["message"], 0).or_else(|| input.first().map(|v| v.to_string())).unwrap_or_default();
            let console = is_console(unsafe { GetStdHandle(STD_OUTPUT) });
            out(&if console { format!("\x1b[33mWARNING: {message}\x1b[0m\r\n") } else { format!("WARNING: {message}\r\n") });
            Ok(Vec::new())
        }
        "write-verbose" | "write-debug" | "write-information" => Ok(Vec::new()),
        "clear-host" => {
            out("\x1b[2J\x1b[3J\x1b[H");
            Ok(Vec::new())
        }
        "foreach-object" => {
            let b = Bound::new(args, &[]);
            let begin = b.get(&["begin"]);
            let end = b.get(&["end"]);
            let process = b.get(&["process"]).or_else(|| b.pos(0));
            let mut out = Vec::new();
            if let Some(Value::Block(blk)) = begin {
                out.extend(sh.call(&blk, Vec::new(), Vec::new(), None, true)?);
            }
            for item in input {
                match &process {
                    Some(Value::Block(blk)) => {
                        let saved = sh.get("_");
                        sh.set("_", item.clone());
                        let r = sh.run_block(blk);
                        sh.set("_", saved);
                        match r {
                            Ok(o) => out.extend(o),
                            Err(Flow::Continue) => {}
                            Err(Flow::Break) => break,
                            Err(f) => return Err(f),
                        }
                    }
                    Some(member) => {
                        // `ForEach-Object Name`: a property, or with arguments a method.
                        let name = member.to_string();
                        let v = item.property(&name);
                        out.extend(v.items());
                    }
                    None => out.push(item),
                }
            }
            if let Some(Value::Block(blk)) = end {
                out.extend(sh.call(&blk, Vec::new(), Vec::new(), None, true)?);
            }
            Ok(out)
        }
        "where-object" => {
            const OPS: &[&str] = &["eq", "ne", "gt", "ge", "lt", "le", "like", "notlike", "match", "notmatch", "contains", "notcontains", "in", "notin", "ceq", "cne", "clike", "cmatch"];
            let b = Bound::new(args, &[]);
            let filter = b.get(&["filterscript"]).or_else(|| b.pos(0));
            let mut out = Vec::new();
            match filter {
                Some(Value::Block(blk)) => {
                    for item in input {
                        let saved = sh.get("_");
                        sh.set("_", item.clone());
                        let r = sh.run_block(&blk);
                        sh.set("_", saved);
                        if Value::from_items(r?).truthy() {
                            out.push(item);
                        }
                    }
                }
                Some(prop) => {
                    // `Where-Object Name -eq value`.
                    let prop = prop.to_string();
                    let op = b.named.iter().find(|(n, _)| OPS.contains(&n.as_str())).cloned();
                    for item in input {
                        let v = item.property(&prop);
                        let keep = match &op {
                            Some((op, value)) => {
                                let value = if matches!(value, Value::Bool(true)) { b.pos(1).unwrap_or(Value::Bool(true)) } else { value.clone() };
                                sh.binary(op, v, value)?.truthy()
                            }
                            None => v.truthy(),
                        };
                        if keep {
                            out.push(item);
                        }
                    }
                }
                None => out = input,
            }
            Ok(out)
        }
        "select-object" => {
            let b = Bound::new(args, &["unique", "wait"]);
            let mut items = input;
            if let Some(n) = b.get(&["skip"]).and_then(|v| v.int()) {
                items.drain(..(n.max(0) as usize).min(items.len()));
            }
            if b.has(&["unique"]) {
                let mut seen: Vec<Value> = Vec::new();
                items.retain(|i| {
                    if seen.iter().any(|s| equal(s, i, true)) {
                        false
                    } else {
                        seen.push(i.clone());
                        true
                    }
                });
            }
            if let Some(n) = b.get(&["first"]).and_then(|v| v.int()) {
                items.truncate(n.max(0) as usize);
            }
            if let Some(n) = b.get(&["last"]).and_then(|v| v.int()) {
                let skip = items.len().saturating_sub(n.max(0) as usize);
                items.drain(..skip);
            }
            if let Some(name) = b.text(&["expandproperty"], usize::MAX) {
                let mut out = Vec::new();
                for i in &items {
                    out.extend(i.property(&name).items());
                }
                return Ok(out);
            }
            let Some(props) = b.arg(&["property"], 0) else { return Ok(items) };
            let wanted = props.items();
            let mut out = Vec::new();
            for i in &items {
                let mut picked = Vec::new();
                for p in property_names(i, &wanted) {
                    picked.push(select_value(sh, i, &p)?);
                }
                out.push(Value::Object(Rc::new(value::Object { type_name: String::from("Selected.System.Management.Automation.PSCustomObject"), props: picked, text: None })));
            }
            Ok(out)
        }
        "sort-object" => {
            let b = Bound::new(args, &["descending", "unique", "casesensitive"]);
            let props = b.arg(&["property"], 0).map(|v| v.items()).unwrap_or_default();
            let mut keyed = Vec::new();
            for i in input {
                let k = sort_key(sh, &i, &props)?;
                keyed.push((k, i));
            }
            let case = b.has(&["casesensitive"]);
            keyed.sort_by(|(a, _), (b2, _)| {
                for (x, y) in a.iter().zip(b2) {
                    match compare(x, y, case) {
                        Some(core::cmp::Ordering::Equal) | None => continue,
                        Some(o) => return o,
                    }
                }
                core::cmp::Ordering::Equal
            });
            if b.has(&["descending"]) {
                keyed.reverse();
            }
            let mut out: Vec<Value> = keyed.into_iter().map(|(_, i)| i).collect();
            if b.has(&["unique"]) {
                out.dedup_by(|a, b2| equal(a, b2, case));
            }
            Ok(out)
        }
        "measure-object" => {
            let b = Bound::new(args, &["sum", "average", "maximum", "minimum", "line", "word", "character"]);
            let prop = b.text(&["property"], 0);
            if b.has(&["line"]) || b.has(&["word"]) || b.has(&["character"]) {
                let text: Vec<String> = input.iter().map(|v| v.to_string()).collect();
                return Ok(vec![Value::object(
                    "Microsoft.PowerShell.Commands.TextMeasureInfo",
                    vec![
                        ("Lines", Value::Int(text.len() as i64)),
                        ("Words", Value::Int(text.iter().map(|t| t.split_whitespace().count() as i64).sum())),
                        ("Characters", Value::Int(text.iter().map(|t| t.chars().count() as i64).sum())),
                        ("Property", Value::Null),
                    ],
                )]);
            }
            let values: Vec<Value> = input.iter().map(|i| match &prop {
                Some(p) => i.property(p),
                None => i.clone(),
            }).collect();
            let numbers: Vec<f64> = values.iter().filter_map(Value::number).collect();
            let count = values.len() as i64;
            let sum: f64 = numbers.iter().sum();
            let num = |d: f64| if d == (d as i64) as f64 { Value::Int(d as i64) } else { Value::Double(d) };
            let pick = |want: bool, v: Value| if want { v } else { Value::Null };
            let max = values.iter().cloned().reduce(|a, b2| if compare(&b2, &a, false) == Some(core::cmp::Ordering::Greater) { b2 } else { a }).unwrap_or(Value::Null);
            let min = values.iter().cloned().reduce(|a, b2| if compare(&b2, &a, false) == Some(core::cmp::Ordering::Less) { b2 } else { a }).unwrap_or(Value::Null);
            Ok(vec![Value::object(
                "Microsoft.PowerShell.Commands.GenericMeasureInfo",
                vec![
                    ("Count", Value::Int(count)),
                    ("Average", pick(b.has(&["average"]), if numbers.is_empty() { Value::Null } else { Value::Double(sum / numbers.len() as f64) })),
                    ("Sum", pick(b.has(&["sum"]), num(sum))),
                    ("Maximum", pick(b.has(&["maximum"]), max)),
                    ("Minimum", pick(b.has(&["minimum"]), min)),
                    ("Property", prop.map_or(Value::Null, Value::Str)),
                ],
            )])
        }
        "group-object" => {
            let b = Bound::new(args, &["noelement", "casesensitive"]);
            let props = b.arg(&["property"], 0).map(|v| v.items()).unwrap_or_default();
            let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
            for i in input {
                let key: Vec<String> = sort_key(sh, &i, &props)?.iter().map(|k| k.to_string()).collect();
                let key = key.join(", ");
                match groups.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
                    Some(g) => g.1.push(i),
                    None => groups.push((key, vec![i])),
                }
            }
            Ok(groups.into_iter().map(|(k, g)| Value::object("Microsoft.PowerShell.Commands.GroupInfo", vec![("Count", Value::Int(g.len() as i64)), ("Name", Value::str(k)), ("Group", Value::array(g))])).collect())
        }
        "format-table" | "format-list" | "format-wide" => {
            let b = Bound::new(args, &["autosize", "wrap", "hidetableheaders", "force"]);
            let props = b.arg(&["property"], 0).map(|v| v.items());
            let mut rows = Vec::new();
            for i in &input {
                match &props {
                    Some(wanted) => {
                        let mut picked = Vec::new();
                        for p in property_names(i, wanted) {
                            picked.push(select_value(sh, i, &p)?);
                        }
                        rows.push(Value::Object(Rc::new(value::Object { type_name: String::from("PSCustomObject"), props: picked, text: None })));
                    }
                    None => rows.push(i.clone()),
                }
            }
            let text = match name {
                "format-list" => fmt::list(&rows),
                "format-wide" => rows.iter().map(|r| fmt::name_of(r)).collect::<Vec<_>>().join("\r\n") + "\r\n",
                _ => fmt::table(&rows, b.has(&["hidetableheaders"])),
            };
            Ok(text.trim_end_matches(['\r', '\n']).split("\r\n").map(|l| Value::str(l.to_string())).collect())
        }
        "out-string" => {
            let b = Bound::new(args, &["stream"]);
            let text = fmt::to_text(&input);
            if b.has(&["stream"]) {
                return Ok(text.split("\r\n").map(|l| Value::str(l.to_string())).collect());
            }
            Ok(vec![Value::str(text)])
        }
        "out-host" | "out-default" => {
            fmt::out_default(&input);
            Ok(Vec::new())
        }
        "out-null" => Ok(Vec::new()),
        "get-date" => {
            let b = Bound::new(args, &[]);
            let mut st = SYSTEMTIME::default();
            unsafe { GetLocalTime(&mut st) };
            match b.text(&["format"], usize::MAX) {
                Some(f) => Ok(vec![Value::str(format_date(&st, &f))]),
                None => Ok(vec![date_value(st)]),
            }
        }
        "start-sleep" => {
            let b = Bound::new(args, &[]);
            let ms = match b.get(&["milliseconds"]).and_then(|v| v.number()) {
                Some(ms) => ms,
                None => b.arg(&["seconds"], 0).and_then(|v| v.number()).unwrap_or(0.0) * 1000.0,
            };
            unsafe { Sleep(ms.max(0.0) as u32) };
            Ok(Vec::new())
        }
        "get-command" => {
            let b = Bound::new(args, &[]);
            let pattern = b.text(&["name"], 0).unwrap_or_else(|| String::from("*"));
            let mut out = Vec::new();
            for (alias, target) in &sh.aliases {
                if like(alias, &pattern, false) {
                    out.push(Value::object("System.Management.Automation.AliasInfo", vec![("CommandType", Value::str("Alias")), ("Name", Value::str(format!("{alias} -> {}", display_name(target)))), ("Version", Value::str("")), ("Source", Value::str(""))]));
                }
            }
            for f in sh.functions.keys() {
                if like(f, &pattern, false) {
                    out.push(Value::object("System.Management.Automation.FunctionInfo", vec![("CommandType", Value::str("Function")), ("Name", Value::str(f.clone())), ("Version", Value::str("")), ("Source", Value::str(""))]));
                }
            }
            for c in CMDLETS {
                if like(c, &pattern, false) {
                    out.push(Value::object("System.Management.Automation.CmdletInfo", vec![("CommandType", Value::str("Cmdlet")), ("Name", Value::str(*c)), ("Version", Value::str("3.1.0.0")), ("Source", Value::str("Microsoft.PowerShell.Management"))]));
                }
            }
            if out.is_empty() && !pattern.contains(['*', '?']) {
                if let Some(p) = find_program(&pattern) {
                    let file = p.rsplit('\\').next().unwrap_or(&p).to_string();
                    out.push(Value::object("System.Management.Automation.ApplicationInfo", vec![("CommandType", Value::str("Application")), ("Name", Value::str(file)), ("Version", Value::str("0.0.0.0")), ("Source", Value::str(p))]));
                } else {
                    return Err(error(&format!("The term '{pattern}' is not recognized as the name of a cmdlet, function, script file, or operable program.")));
                }
            }
            Ok(out)
        }
        "get-alias" => {
            let b = Bound::new(args, &[]);
            let pattern = b.text(&["name"], 0).unwrap_or_else(|| String::from("*"));
            Ok(sh
                .aliases
                .iter()
                .filter(|(a, _)| like(a, &pattern, false))
                .map(|(a, t)| Value::object("System.Management.Automation.AliasInfo", vec![("CommandType", Value::str("Alias")), ("Name", Value::str(format!("{a} -> {}", display_name(t)))), ("Definition", Value::str(display_name(t)))]))
                .collect())
        }
        "set-alias" | "new-alias" => {
            let b = Bound::new(args, &["force"]);
            let alias = b.text(&["name"], 0).unwrap_or_default();
            let target = b.text(&["value"], 1).unwrap_or_default();
            sh.aliases.insert(alias.to_ascii_lowercase(), target.to_ascii_lowercase());
            Ok(Vec::new())
        }
        "get-variable" => {
            let b = Bound::new(args, &["valueonly"]);
            let pattern = b.text(&["name"], 0).unwrap_or_else(|| String::from("*"));
            let values = b.has(&["valueonly"]);
            Ok(sh.variables().into_iter().filter(|(k, _)| like(k, &pattern, false)).map(|(k, v)| if values { v } else { Value::object("System.Management.Automation.PSVariable", vec![("Name", Value::str(k)), ("Value", v)]) }).collect())
        }
        "set-variable" | "new-variable" => {
            let b = Bound::new(args, &["passthru", "force"]);
            let n = b.text(&["name"], 0).unwrap_or_default();
            let v = b.arg(&["value"], 1).unwrap_or(Value::from_items(input));
            sh.set(&n, v);
            Ok(Vec::new())
        }
        "remove-variable" => {
            let b = Bound::new(args, &["force"]);
            let n = b.text(&["name"], 0).unwrap_or_default();
            sh.remove(&n);
            Ok(Vec::new())
        }
        "clear-variable" => {
            let b = Bound::new(args, &[]);
            let n = b.text(&["name"], 0).unwrap_or_default();
            sh.set(&n, Value::Null);
            Ok(Vec::new())
        }
        "get-help" => {
            let b = Bound::new(args, &["full", "detailed", "examples", "online"]);
            let topic = b.text(&["name"], 0).unwrap_or_default();
            if topic.is_empty() {
                out("\r\nTOPIC\r\n    Windows PowerShell Help System (atlas's)\r\n\r\nSHORT DESCRIPTION\r\n    Get-Command lists the commands there are; Get-Alias their aliases.\r\n\r\n");
            } else {
                let resolved = sh.aliases.get(&topic.to_ascii_lowercase()).cloned().unwrap_or(topic.to_ascii_lowercase());
                out(&format!("\r\nNAME\r\n    {}\r\n\r\nSYNTAX\r\n    {} [<parameters>]\r\n\r\n", display_name(&resolved), display_name(&resolved)));
            }
            Ok(Vec::new())
        }
        "invoke-expression" => {
            let b = Bound::new(args, &[]);
            let text = b.text(&["command"], 0).or_else(|| input.first().map(|v| v.to_string())).unwrap_or_default();
            let block = Parser::script(&text).map_err(|e| error(&e.message))?;
            sh.run_block(&block)
        }
        "invoke-command" => {
            let b = Bound::new(args, &["noprofile"]);
            match b.arg(&["scriptblock"], 0) {
                Some(Value::Block(blk)) => {
                    let list = b.get(&["argumentlist", "args"]).map(|v| v.items()).unwrap_or_default();
                    sh.call(&blk, list, Vec::new(), None, false)
                }
                _ => Err(error("Invoke-Command needs -ScriptBlock.")),
            }
        }
        "start-process" => {
            let b = Bound::new(args, &["wait", "nonewwindow", "passthru", "usenewenvironment"]);
            let file = b.text(&["filepath"], 0).ok_or_else(|| error("Cannot bind argument to parameter 'FilePath'."))?;
            let list: Vec<String> = b.arg(&["argumentlist", "args"], 1).map(|v| v.items().iter().map(|i| i.to_string()).collect()).unwrap_or_default();
            let path = find_program(&file).ok_or_else(|| error(&format!("This command cannot be run because \"{file}\" could not be found.")))?;
            let mut line = format!("\"{path}\"");
            for a in &list {
                line.push(' ');
                line.push_str(a);
            }
            let dir = b.text(&["workingdirectory"], usize::MAX).map(|d| wide(&full_path(&d)));
            let app = wide(&path);
            let mut line_w = wide(&line);
            let mut si = STARTUPINFOW::default();
            let mut pi = PROCESS_INFORMATION::default();
            let ok = unsafe { CreateProcessW(app.as_ptr(), line_w.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), TRUE, 0, core::ptr::null_mut(), dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()), &mut si, &mut pi) };
            if ok == 0 {
                return Err(error(&format!("This command cannot be run due to the error: the program \"{file}\" didn't start.")));
            }
            let mut out = Vec::new();
            if b.has(&["wait"]) {
                unsafe { WaitForSingleObject(pi.process, INFINITE) };
                let mut code = 0u32;
                unsafe { GetExitCodeProcess(pi.process, &mut code) };
                sh.last_exit = code as i32;
            }
            if b.has(&["passthru"]) {
                out.push(Value::object("System.Diagnostics.Process", vec![("Id", Value::Int(i64::from(pi.process_id))), ("ProcessName", Value::str(file))]));
            }
            unsafe {
                CloseHandle(pi.process as HANDLE);
                CloseHandle(pi.thread as HANDLE);
            }
            Ok(out)
        }
        "select-string" => {
            let b = Bound::new(args, &["simplematch", "casesensitive", "notmatch", "quiet", "list"]);
            let pattern = b.text(&["pattern"], 0).ok_or_else(|| error("Cannot bind argument to parameter 'Pattern'."))?;
            let case = b.has(&["casesensitive"]);
            let simple = b.has(&["simplematch"]);
            let invert = b.has(&["notmatch"]);
            let re = if simple { None } else { Some(crate::regex::Regex::new(&pattern, !case).map_err(|e| error(&e))?) };
            let hit = |line: &str| -> bool {
                let found = match &re {
                    Some(r) => r.is_match(line),
                    None if case => line.contains(&pattern),
                    None => line.to_ascii_lowercase().contains(&pattern.to_ascii_lowercase()),
                };
                found != invert
            };
            let mut out = Vec::new();
            let paths = b.arg(&["path", "literalpath"], 1).map(|v| v.items());
            match paths {
                Some(paths) => {
                    for p in paths {
                        for r in resolve(&p.to_string()) {
                            let data = read_file(&r).unwrap_or_default();
                            for (n, line) in lines(&data).iter().enumerate() {
                                if hit(line) {
                                    let short = r.rsplit('\\').next().unwrap_or(&r).to_string();
                                    out.push(match_info(line, n + 1, &short, &r));
                                }
                            }
                        }
                    }
                }
                None => {
                    for (n, v) in input.iter().enumerate() {
                        let line = v.to_string();
                        if hit(&line) {
                            out.push(match_info(&line, n + 1, "InputStream", ""));
                        }
                    }
                }
            }
            if b.has(&["quiet"]) {
                return Ok(vec![Value::Bool(!out.is_empty())]);
            }
            Ok(out)
        }
        "get-member" => {
            let mut seen: Vec<String> = Vec::new();
            let mut out = Vec::new();
            for i in &input {
                let ty = i.type_name();
                if seen.contains(&ty) {
                    continue;
                }
                seen.push(ty.clone());
                out.push(Value::str(format!("\r\n   TypeName: {ty}\r\n")));
                let props: Vec<(String, Value)> = match i {
                    Value::Object(o) => o.props.clone(),
                    Value::Hash(h) => h.borrow().clone(),
                    Value::Str(s) => vec![(String::from("Length"), Value::Int(s.chars().count() as i64))],
                    _ => Vec::new(),
                };
                for (k, v) in props {
                    out.push(Value::object("Microsoft.PowerShell.Commands.MemberDefinition", vec![("Name", Value::str(k)), ("MemberType", Value::str("Property")), ("Definition", Value::str(format!("{} {{get;set;}}", v.type_name())))]));
                }
            }
            Ok(out)
        }
        "read-host" => {
            let b = Bound::new(args, &["assecurestring"]);
            if let Some(p) = b.text(&["prompt"], 0) {
                out(&format!("{p}: "));
            }
            Ok(vec![Value::str(Input::stdin().line().unwrap_or_default())])
        }
        "get-random" => {
            let b = Bound::new(args, &[]);
            let seed = unsafe { GetTickCount64() }.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 17;
            if !input.is_empty() {
                let n = b.get(&["count"]).and_then(|v| v.int()).unwrap_or(1).max(1) as usize;
                return Ok((0..n.min(input.len())).map(|k| input[(seed as usize + k * 7919) % input.len()].clone()).collect());
            }
            let min = b.get(&["minimum"]).and_then(|v| v.int()).unwrap_or(0);
            let max = b.arg(&["maximum"], 0).and_then(|v| v.int()).unwrap_or(i64::from(i32::MAX));
            let span = (max - min).max(1) as u64;
            Ok(vec![Value::Int(min + (seed % span) as i64)])
        }
        "new-object" => {
            let b = Bound::new(args, &[]);
            let ty = b.text(&["typename"], 0).unwrap_or_default().to_ascii_lowercase();
            match ty.as_str() {
                "psobject" | "pscustomobject" | "system.management.automation.psobject" => {
                    let props = match b.get(&["property"]) {
                        Some(Value::Hash(h)) => h.borrow().clone(),
                        _ => Vec::new(),
                    };
                    Ok(vec![Value::Object(Rc::new(value::Object { type_name: String::from("System.Management.Automation.PSCustomObject"), props, text: None }))])
                }
                "system.collections.arraylist" | "collections.arraylist" | "system.collections.generic.list[object]" => Ok(vec![Value::array(Vec::new())]),
                "system.collections.hashtable" | "hashtable" => Ok(vec![Value::hash(Vec::new())]),
                _ => Err(error(&format!("Cannot find type [{ty}]: verify that the assembly containing this type is loaded."))),
            }
        }
        "tee-object" => {
            let b = Bound::new(args, &["append"]);
            if let Some(path) = b.text(&["filepath", "path"], 0) {
                write_file(&path, fmt::to_text(&input).as_bytes(), b.has(&["append"])).map_err(|e| error(&e))?;
            } else if let Some(var) = b.text(&["variable"], usize::MAX) {
                sh.set(&var, Value::from_items(input.clone()));
            }
            Ok(input)
        }
        "get-unique" => {
            let mut out: Vec<Value> = Vec::new();
            for i in input {
                if !out.last().is_some_and(|l| equal(l, &i, true)) {
                    out.push(i);
                }
            }
            Ok(out)
        }
        "convertto-json" => {
            let b = Bound::new(args, &["compress"]);
            let compress = b.has(&["compress"]);
            let items = b.arg(&["inputobject"], 0).map(|v| v.items()).unwrap_or(input);
            let v = Value::from_items(items);
            Ok(vec![Value::str(json(&v, compress, 0))])
        }
        "get-host" => Ok(vec![Value::object("System.Management.Automation.Internal.Host.InternalHost", vec![("Name", Value::str("ConsoleHost")), ("Version", Value::str("5.1.19041.1")), ("CurrentCulture", Value::str("en-US"))])]),
        "get-process" => Ok(vec![Value::object("System.Diagnostics.Process", vec![("Handles", Value::Int(0)), ("NPM(K)", Value::Int(0)), ("PM(K)", Value::Int(0)), ("Id", Value::Int(0)), ("ProcessName", Value::str("powershell"))])]),
        "stop-process" => Err(error("Stop-Process isn't there yet: use taskkill from cmd.")),
        _ => Err(error(&format!("The term '{name}' is not recognized."))),
    }
}

fn match_info(line: &str, number: usize, file: &str, path: &str) -> Value {
    let text = if path.is_empty() { line.to_string() } else { format!("{file}:{number}:{line}") };
    Value::Object(Rc::new(value::Object {
        type_name: String::from("Microsoft.PowerShell.Commands.MatchInfo"),
        props: vec![(String::from("LineNumber"), Value::Int(number as i64)), (String::from("Line"), Value::str(line)), (String::from("Filename"), Value::str(file)), (String::from("Path"), Value::str(path))],
        text: Some(text),
    }))
}

fn vt_color(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "black" => 30,
        "darkred" => 31,
        "darkgreen" => 32,
        "darkyellow" => 33,
        "darkblue" => 34,
        "darkmagenta" => 35,
        "darkcyan" => 36,
        "gray" => 37,
        "darkgray" => 90,
        "red" => 91,
        "green" => 92,
        "yellow" => 93,
        "blue" => 94,
        "magenta" => 95,
        "cyan" => 96,
        "white" => 97,
        _ => return None,
    })
}

fn json(v: &Value, compress: bool, depth: usize) -> String {
    let (nl, ind, ind2, sep) = if compress { (String::new(), String::new(), String::new(), ":") } else { (String::from("\r\n"), "    ".repeat(depth), "    ".repeat(depth + 1), ":  ") };
    let esc = |s: &str| {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    };
    let object = |pairs: Vec<(String, Value)>| -> String {
        if pairs.is_empty() {
            return String::from("{}");
        }
        let parts: Vec<String> = pairs.iter().map(|(k, x)| format!("{ind2}{}{sep}{}", esc(k), json(x, compress, depth + 1))).collect();
        format!("{{{nl}{}{nl}{ind}}}", parts.join(&format!(",{nl}")))
    };
    match v {
        Value::Null => String::from("null"),
        Value::Bool(b) => String::from(if *b { "true" } else { "false" }),
        Value::Int(n) => format!("{n}"),
        Value::Double(d) => crate::value::format_double(*d),
        Value::Str(s) => esc(s),
        Value::Array(a) => {
            if a.is_empty() {
                return String::from("[]");
            }
            let parts: Vec<String> = a.iter().map(|x| format!("{ind2}{}", json(x, compress, depth + 1))).collect();
            format!("[{nl}{}{nl}{ind}]", parts.join(&format!(",{nl}")))
        }
        Value::Hash(h) => object(h.borrow().clone()),
        Value::Object(o) if o.text.is_some() => esc(o.text.as_deref().unwrap_or("")),
        Value::Object(o) => object(o.props.clone()),
        Value::Block(b) => esc(&b.text),
    }
}

fn copy_tree(from: &str, to: &str, recurse: bool) -> bool {
    let w = wide(to);
    if attributes(to).is_none() && unsafe { CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) } == 0 {
        return false;
    }
    if !recurse {
        return true;
    }
    for f in find_all(&format!("{from}\\*")) {
        let name = narrow(&f.name);
        let (src, dst) = (format!("{from}\\{name}"), format!("{to}\\{name}"));
        if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            if !copy_tree(&src, &dst, true) {
                return false;
            }
        } else {
            let (a, b) = (wide(&src), wide(&dst));
            if unsafe { CopyFileW(a.as_ptr(), b.as_ptr(), 0) } == 0 {
                return false;
            }
        }
    }
    true
}

fn remove_tree(dir: &str) {
    for f in find_all(&format!("{dir}\\*")) {
        let path = format!("{dir}\\{}", narrow(&f.name));
        let w = wide(&path);
        if f.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            if f.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                remove_tree(&path);
            }
            unsafe { RemoveDirectoryW(w.as_ptr()) };
        } else {
            unsafe {
                SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL);
                DeleteFileW(w.as_ptr());
            }
        }
    }
}

#[allow(dead_code)]
fn unused(_: &Value) -> Option<core::cmp::Ordering> {
    let _ = like;
    None
}
