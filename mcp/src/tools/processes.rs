//! Processes in running machines: `exec`, waited for or left running, and what a running one writes.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use carbon::{ProcessEvent, ProcessHandle};
use serde_json::{Value, json};

use super::{Is, Tool, command_prop, env_prop, prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

const EXEC_DEFAULT: Duration = Duration::from_secs(60);
const EXEC_MOST: Duration = Duration::from_secs(900);
/// How long a background process that's ended is kept for its last output to be read.
const KEEP_ENDED: Duration = Duration::from_secs(3600);

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "exec",
            title: "Run a command in a running machine",
            description: "Runs a command in a running machine, as docker exec, and waits for it: its exit code, stdout and stderr. With background, it's left running and its process id comes back at once, for process_output, process_input and process_signal.",
            properties: json!({
                "command": command_prop("The command: a string, run by the machine's shell (sh -c; cmd /c in Windows), or an array, run as it is"),
                "stdin": prop("string", "What it reads on stdin; it's closed after (left open with background, for process_input)"),
                "env": env_prop(),
                "workdir": prop("string", "Where it starts; the machine's working folder by default"),
                "user": prop("string", "Who it runs as: a name or uid, optionally :group; the machine's user by default"),
                "timeout_secs": prop("integer", "How long it may run before it's killed: 60 by default, 900 at most (none with background)"),
                "background": prop("boolean", "Leave it running, and give back its process id"),
            }),
            required: &["command"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: exec,
        },
        Tool {
            name: "process_output",
            title: "A background process's output",
            description: "What a process exec left running has written since a cursor (0 for all of it), and its exit code once it's ended. wait_secs waits that long for more, or for it to end.",
            properties: json!({
                "process": prop("string", "Its id, from exec"),
                "cursor": prop("integer", "The cursor a call before gave back: only what came after it"),
                "wait_secs": prop("integer", "Wait up to this long for more output or its end: 0 by default, 60 at most"),
            }),
            required: &["process"],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: output,
        },
        Tool {
            name: "process_input",
            title: "Write to a background process",
            description: "Writes text to a background process's stdin, or closes it (end of input).",
            properties: json!({
                "process": prop("string", "Its id, from exec"),
                "text": prop("string", "What to write; add \\n for a line"),
                "close": prop("boolean", "Close its stdin after"),
            }),
            required: &["process"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: input,
        },
        Tool {
            name: "process_signal",
            title: "Signal a background process",
            description: "Sends a signal to a background process: TERM (the default), INT, KILL, HUP, QUIT, USR1, USR2. In a Windows machine, any signal ends it.",
            properties: json!({
                "process": prop("string", "Its id, from exec"),
                "signal": prop("string", "TERM, INT, KILL, ..."),
            }),
            required: &["process"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: signal,
        },
    ]
}

/// A process left running: what it's written, in order, by stream, and how it ended.
pub(crate) struct Background {
    machine: String,
    handle: ProcessHandle,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// Everything it wrote, stderr marked, as one text, so a cursor is a byte offset in it.
    text: Vec<u8>,
    exit: Option<i32>,
    ended_at: Option<Instant>,
}

/// The shell a machine's commands run by, and whether it's Windows: its image's, else cmd for a Windows machine made
/// from a program.
fn machine_shell(server: &Server, machine: &carbon::Machine) -> (Vec<String>, bool) {
    let Ok(info) = machine.info() else { return (super::machines::shell(None), false) };
    let windows = info.platform.os == "windows";
    let image = info.image.as_deref().and_then(|i| server.carbon.image(i));
    match image {
        Some(i) => (super::machines::shell(Some(&i)), windows),
        None if windows => (vec!["cmd.exe".into(), "/c".into()], true),
        None => (super::machines::shell(None), false),
    }
}

fn exec(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let machine = server.machine(args)?;
    if !machine.is_running() {
        return Err(Failure(format!("{} isn't running: restart_machine starts it again", machine.name())));
    }
    let (shell, windows) = machine_shell(server, &machine);
    let argv = super::machines::argv_with(&args.get("command").cloned().unwrap_or(Value::Null), shell)?;
    let mut spec = machine.exec(argv);
    for (k, v) in args.env("env") {
        spec = spec.env(&k, &v);
    }
    if let Some(w) = args.opt_str("workdir") {
        spec = spec.workdir(w);
    }
    if let Some(u) = args.opt_str("user") {
        spec = spec.user(u);
    }
    let background = args.bool("background");
    let timeout = args.timeout(EXEC_DEFAULT, EXEC_MOST);
    if !background {
        spec = spec.timeout(timeout);
    }
    let mut process = spec.spawn()?;
    let handle = process.handle()?;
    if let Some(text) = args.opt_str("stdin") {
        handle.write_stdin(text.as_bytes())?;
    }
    if background {
        let id = format!("p{}", shape::new_id());
        let bg = Arc::new(Background { machine: machine.name().to_string(), handle, state: Mutex::new(State::default()), changed: Condvar::new() });
        let reader = bg.clone();
        std::thread::spawn(move || {
            while let Some(event) = process.next_event() {
                let mut state = reader.state.lock().unwrap_or_else(|e| e.into_inner());
                match event {
                    ProcessEvent::Stdout(b) => state.text.extend_from_slice(&b),
                    ProcessEvent::Stderr(b) => {
                        // Stderr kept apart in the text: each piece marked where it starts.
                        state.text.extend_from_slice(b"[stderr] ");
                        state.text.extend_from_slice(&b);
                    }
                    ProcessEvent::Exit(code) => {
                        state.exit = Some(code);
                        state.ended_at = Some(Instant::now());
                    }
                }
                reader.changed.notify_all();
            }
            let mut state = reader.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.exit.is_none() {
                state.exit = Some(255);
                state.ended_at = Some(Instant::now());
            }
            reader.changed.notify_all();
        });
        let mut processes = server.processes.lock().unwrap_or_else(|e| e.into_inner());
        processes.retain(|_, p| p.state.lock().map_or(true, |s| s.ended_at.is_none_or(|t| t.elapsed() < KEEP_ENDED)));
        processes.insert(id.clone(), bg);
        return Ok(shape::data(json!({ "process": id, "machine": machine.name() })));
    }
    handle.close_stdin();
    let (mut stdout, mut stderr, mut code) = (Vec::new(), Vec::new(), 255);
    let mut said = Instant::now();
    while let Some(event) = process.next_event() {
        match event {
            ProcessEvent::Stdout(b) => {
                if said.elapsed() >= Duration::from_secs(1) {
                    said = Instant::now();
                    progress.say(String::from_utf8_lossy(&b).trim_end(), None);
                }
                stdout.extend_from_slice(&b);
            }
            ProcessEvent::Stderr(b) => stderr.extend_from_slice(&b),
            ProcessEvent::Exit(c) => code = c,
        }
    }
    let timed_out = process.timed_out();
    let mut result = super::machines::output_result(code, &stdout, &stderr, timed_out, json!({ "machine": machine.name() }));
    // What was cut is kept whole in the machine, to read.
    if stdout.len() > shape::OUTPUT_CAP || stderr.len() > shape::OUTPUT_CAP {
        let id = shape::new_id();
        let files = machine.files();
        let dir = if windows { "C:\\Windows\\Temp" } else { "/tmp" };
        let sep = if windows { "\\" } else { "/" };
        let mut kept = Vec::new();
        for (what, bytes) in [("out", &stdout), ("err", &stderr)] {
            if bytes.len() > shape::OUTPUT_CAP {
                let path = format!("{dir}{sep}carbon-exec-{id}.{what}");
                if files.write(&path, bytes.as_slice(), carbon::Write::new().parents(true)).is_ok() {
                    kept.push(path);
                }
            }
        }
        if !kept.is_empty() {
            let note = format!("\n[the whole output is in the machine: {}; read it with read_file]", kept.join(", "));
            if let Some(t) = result["content"][0]["text"].as_str() {
                result["content"][0]["text"] = json!(format!("{t}{note}"));
            }
            result["structuredContent"]["kept"] = json!(kept);
        }
    }
    Ok(result)
}

fn background(server: &Server, args: &Args) -> Result<Arc<Background>, Failure> {
    let id = args.str("process")?;
    let found = server.processes.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned();
    let bg = found.ok_or_else(|| Failure(format!("there's no process {id}: exec with background gives one")))?;
    if bg.machine != args.str("machine")? {
        return Err(Failure(format!("process {id} is in {}, not {}", bg.machine, args.str("machine")?)));
    }
    Ok(bg)
}

fn output(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let bg = background(server, args)?;
    let cursor = args.u64("cursor").unwrap_or(0) as usize;
    let wait = Duration::from_secs(args.u64("wait_secs").unwrap_or(0).min(60));
    let deadline = Instant::now() + wait;
    let mut state = bg.state.lock().unwrap_or_else(|e| e.into_inner());
    while state.text.len() <= cursor && state.exit.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        state = bg.changed.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    let from = cursor.min(state.text.len());
    let (text, cut) = shape::clip(&state.text[from..], shape::OUTPUT_CAP);
    let next = state.text.len();
    let head = match state.exit {
        Some(code) => format!("ended, exit code {code} (cursor {next})\n"),
        None => format!("running (cursor {next})\n"),
    };
    Ok(shape::text_and(head + &text, json!({ "cursor": next, "running": state.exit.is_none(), "exit_code": state.exit, "output": text, "truncated": cut })))
}

fn input(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let bg = background(server, args)?;
    if let Some(text) = args.opt_str("text") {
        bg.handle.write_stdin(text.as_bytes())?;
    }
    if args.bool("close") {
        bg.handle.close_stdin();
    }
    Ok(shape::text("written"))
}

fn signal(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let bg = background(server, args)?;
    let name = args.opt_str("signal").unwrap_or_else(|| "TERM".into());
    let number = match name.trim_start_matches("SIG").to_ascii_uppercase().as_str() {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "KILL" => 9,
        "USR1" => 10,
        "USR2" => 12,
        "TERM" => 15,
        other => other.parse().map_err(|_| Failure(format!("{name} isn't a signal: TERM, INT, KILL, HUP, QUIT, USR1, USR2")))?,
    };
    bg.handle.signal(number)?;
    Ok(shape::text(format!("sent SIG{} to {}", name.trim_start_matches("SIG").to_ascii_uppercase(), args.str("process")?)))
}
