//! cmd.exe, atlas's own: Windows' command interpreter, as a Windows program in System32. Its own commands (DIR, CD,
//! SET, COPY, IF, FOR, CALL, GOTO and the rest), batch files, `%` and delayed `!` expansion, pipes, redirection and
//! `&`, `&&`, `||`, and the switches cmd takes: `/C` and `/K` with `/S`, `/Q`, `/D`, `/V:ON|OFF`, `/E:ON|OFF`, and `/A`,
//! `/U`, `/T` and `/F` taken and ignored.

#![no_std]
#![no_main]

extern crate alloc;
extern crate atlas_programs as programs;

mod arith;
mod dir;
mod expand;
mod parse;
mod shell;

use alloc::format;
use alloc::string::{String, ToString};

use programs::*;
use shell::{Flow, Shell};

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    exit(main())
}

/// What runs after `/C` or `/K`, with `/S`'s rule for quotes: the first and last taken off, unless (without `/S`)
/// there are just two, around a program's name with a space in it and nothing cmd would read specially.
fn command_string(text: &str, strip: bool) -> String {
    let text = text.trim_start();
    if !text.starts_with('"') {
        return text.to_string();
    }
    let quotes = text.matches('"').count();
    if !strip && quotes == 2 {
        let inner = &text[1..text.rfind('"').unwrap_or(text.len())];
        if inner.contains(' ') && !inner.contains(['&', '<', '>', '(', ')', '@', '^', '|']) && shell::search(inner).is_some() {
            return text.to_string();
        }
    }
    let mut s = text[1..].to_string();
    if let Some(last) = s.rfind('"') {
        s.remove(last);
    }
    s
}

fn main() -> i32 {
    let mut sh = Shell::new();
    let args = arguments_text();
    let mut rest = args.as_str();
    let mut run: Option<(String, bool)> = None;
    let mut strip = false;
    // Switches, up to /C or /K; what's after either is the command.
    loop {
        rest = rest.trim_start();
        let Some(sw) = rest.strip_prefix('/') else { break };
        let end = sw.find([' ', '\t', '/']).unwrap_or(sw.len());
        let word = sw[..end].to_ascii_lowercase();
        let after = &sw[end..];
        match word.as_str() {
            "c" | "k" => {
                run = Some((command_string(after, strip), word == "k"));
                break;
            }
            "s" => strip = true,
            "q" => sh.echo = false,
            "v:on" => sh.delayed = true,
            "v:off" => sh.delayed = false,
            "e:off" => sh.extensions = false,
            "?" => {
                out("Starts a new instance of the Windows command interpreter\r\n\r\nCMD [/A | /U] [/Q] [/D] [/E:ON | /E:OFF] [/F:ON | /F:OFF] [/V:ON | /V:OFF]\r\n    [[/S] [/C | /K] string]\r\n\r\n/C      Carries out the command specified by string and then terminates\r\n/K      Carries out the command specified by string but remains\r\n/S      Modifies the treatment of string after /C or /K\r\n/Q      Turns echo off\r\n/D      Disable execution of AutoRun commands\r\n/V:ON   Enable delayed environment variable expansion using ! as the\r\n        delimiter\r\n");
                return 0;
            }
            _ => {}
        }
        rest = after;
    }
    // `cmd.exe file.bat`-like arguments with no switch run as a command too, as cmd's own /C would not: keep cmd's
    // behaviour, which is to start interactively and ignore them.
    if let Some((command, keep)) = &run {
        match sh.run_line(command) {
            Flow::Exit(code) => return code,
            _ if !keep => return sh.errorlevel,
            _ => {}
        }
    } else {
        out(&format!("{}\r\n(c) atlas: Carbon's own command interpreter.\r\n\r\n", shell::VERSION));
    }
    interactive(&mut sh)
}

/// The prompt: a line at a time (a block left open, or a `^` at the end, asks `More?` for the rest), until EXIT or
/// the input's end.
fn interactive(sh: &mut Shell) -> i32 {
    sh.interactive = true;
    let mut input = Input::stdin();
    let echo_input = !input.is_console();
    loop {
        if sh.echo {
            out(&sh.prompt());
        }
        let Some(mut line) = input.line() else { return sh.errorlevel };
        if echo_input && sh.echo {
            out(&format!("{line}\r\n"));
        }
        loop {
            if line.ends_with('^') && !line.ends_with("^^") {
                line.pop();
                out("More? ");
                match input.line() {
                    Some(more) => line.push_str(&more),
                    None => break,
                }
            } else if parse::unbalanced(&line) {
                out("More? ");
                match input.line() {
                    Some(more) => {
                        line.push('\n');
                        line.push_str(&more);
                    }
                    None => break,
                }
            } else {
                break;
            }
        }
        let empty = line.trim().is_empty();
        if let Flow::Exit(code) = sh.run_line(&line) {
            return code;
        }
        if !empty && sh.echo {
            out("\r\n");
        }
    }
}
