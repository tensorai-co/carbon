//! powershell.exe, atlas's own: a PowerShell-language shell (the real one is a .NET program), with Windows PowerShell
//! 5.1's command line: `-Command` (or `-c`, or `-` to read the script from stdin), `-File` with arguments,
//! `-EncodedCommand`, `-NoLogo`, `-NoProfile`, `-NonInteractive`, `-NoExit`, `-ExecutionPolicy`, and the rest taken
//! and ignored (`-InputFormat`, `-OutputFormat`, `-WindowStyle`, `-Version`, `-Sta`, `-Mta`). With no command it's the
//! prompt, `PS C:\>`; with stdin a file or pipe, the script is read from it.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

mod cmdlets;
mod eval;
mod format;
mod parse;
mod regex;
mod value;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use eval::{Flow, Shell, message};
use parse::Parser;
use programs::*;
use value::Value;

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    exit(main())
}

/// Base64, as `-EncodedCommand` gives it (UTF-16LE inside).
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut n = 0;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        bits = bits << 6 | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

/// Runs a script's text; what it exits with, if it says (`exit`), and whether it ended in an error.
fn run_text(sh: &mut Shell, text: &str, show: bool) -> (Option<i32>, bool) {
    let block = match Parser::script(text) {
        Ok(b) => b,
        Err(e) => {
            err(&format!("{}\r\n", e.message));
            return (None, true);
        }
    };
    let mut failed = false;
    for stmt in &block.stmts {
        sh.ok = true;
        match sh.stmt(stmt) {
            Ok(values) => {
                if show {
                    format::out_default(&values);
                }
                failed = !sh.ok;
            }
            Err(Flow::Exit(code)) => return (Some(code), false),
            Err(Flow::Throw(e)) => {
                err(&format!("{}\r\n", message(&e)));
                sh.ok = false;
                failed = true;
            }
            Err(Flow::Return(values)) => {
                if show {
                    format::out_default(&values);
                }
                return (None, false);
            }
            Err(Flow::Break | Flow::Continue) => {}
        }
    }
    (None, failed)
}

fn main() -> i32 {
    let mut sh = Shell::new();
    let args = arguments();
    let (mut logo, mut no_exit) = (true, false);
    let mut command: Option<String> = None;
    let mut file: Option<(String, Vec<String>)> = None;
    let mut i = 0;
    let is = |a: &str, names: &[&str]| -> bool {
        let lower = a.trim_start_matches(['-', '/']).to_ascii_lowercase();
        !lower.is_empty() && (a.starts_with('-') || a.starts_with('/')) && names.iter().any(|n| n.starts_with(&lower))
    };
    while i < args.len() {
        let a = &args[i];
        if is(a, &["nologo"]) {
            logo = false;
        } else if is(a, &["noexit"]) {
            no_exit = true;
        } else if is(a, &["noprofile", "noninteractive", "sta", "mta", "nop", "noni"]) {
        } else if is(a, &["executionpolicy", "ep", "ex", "inputformat", "outputformat", "windowstyle", "version", "configurationname", "psconsolefile", "workingdirectory"]) {
            if is(a, &["workingdirectory"])
                && let Some(d) = args.get(i + 1)
            {
                let w = wide(d);
                unsafe { SetCurrentDirectoryW(w.as_ptr()) };
            }
            i += 1;
        } else if is(a, &["command"]) || a.eq_ignore_ascii_case("-c") {
            command = Some(args[i + 1..].join(" "));
            break;
        } else if is(a, &["encodedcommand", "ec"]) || a.eq_ignore_ascii_case("-e") {
            let decoded = args.get(i + 1).and_then(|t| base64(t)).map(|bytes| {
                let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
                String::from_utf16_lossy(&units)
            });
            match decoded {
                Some(text) => command = Some(text),
                None => {
                    err("Cannot process the command because the value specified with -EncodedCommand is not properly encoded.\r\n");
                    return 1;
                }
            }
            i += 1;
        } else if is(a, &["file"]) || a.eq_ignore_ascii_case("-f") {
            if let Some(path) = args.get(i + 1) {
                file = Some((path.clone(), args[i + 2..].to_vec()));
            }
            break;
        } else if a == "-?" || a == "/?" || is(a, &["help"]) {
            out("PowerShell[.exe] [-NoLogo] [-NoExit] [-NoProfile] [-NonInteractive] [-ExecutionPolicy <policy>]\r\n    [-Command { - | <script-block> [-args <arg-array>] | <string> [<CommandParameters>] }]\r\n    [-File <filePath> <args>] [-EncodedCommand <Base64EncodedCommand>]\r\n");
            return 0;
        } else {
            // What's left with no switch before it is the command, as Windows PowerShell takes it.
            command = Some(args[i..].join(" "));
            break;
        }
        i += 1;
    }
    let code_of = |sh: &Shell, (exit, failed): (Option<i32>, bool)| exit.unwrap_or(if failed { 1 } else { sh.last_exit });
    if let Some((path, rest)) = file {
        let full = cmdlets::full_path(&path);
        let Some(bytes) = read_file(&full) else {
            err(&format!("The argument '{path}' to the -File parameter does not exist. Provide the path to an existing '.ps1' file as an argument to the -File parameter.\r\n"));
            return 64;
        };
        let text: String = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').into();
        let script = match Parser::script(&text) {
            Ok(b) => b,
            Err(e) => {
                err(&format!("{}\r\n", e.message));
                return 1;
            }
        };
        let values: Vec<Value> = rest.iter().map(|a| value::parse_number(a).unwrap_or_else(|| Value::str(a.clone()))).collect();
        let r = sh.call(&script, values, Vec::new(), None, true);
        let code = match r {
            Ok(v) => {
                format::out_default(&v);
                if sh.ok { 0 } else { 1 }
            }
            Err(Flow::Exit(c)) => c,
            Err(Flow::Throw(e)) => {
                err(&format!("{}\r\n", message(&e)));
                1
            }
            Err(_) => 0,
        };
        if !no_exit {
            return code;
        }
    }
    if let Some(command) = command {
        let text = if command.trim() == "-" { String::from_utf8_lossy(&read_stdin()).into_owned() } else { command };
        let r = run_text(&mut sh, &text, true);
        if !no_exit || r.0.is_some() {
            return code_of(&sh, r);
        }
    }
    let mut input = Input::stdin();
    if !input.is_console() {
        // A script from a file or pipe.
        let text = String::from_utf8_lossy(&read_stdin()).into_owned();
        let r = run_text(&mut sh, &text, true);
        return code_of(&sh, r);
    }
    if logo {
        out("Windows PowerShell\r\natlas: Carbon's own PowerShell.\r\n\r\n");
    }
    loop {
        out(&format!("PS {}> ", current_dir()));
        let Some(mut text) = input.line() else { return sh.last_exit };
        // More lines while what's there is incomplete: a block, string or pipe left open.
        loop {
            match Parser::script(&text) {
                Err(e) if e.incomplete => {
                    out(">> ");
                    match input.line() {
                        Some(more) if !more.is_empty() => {
                            text.push('\n');
                            text.push_str(&more);
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        if let (Some(code), _) = run_text(&mut sh, &text, true) {
            return code;
        }
    }
}
