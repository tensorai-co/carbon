//! Machines: one-off commands (`run`), machines that keep running, and what's known of them.

use std::time::{Duration, Instant};

use carbon::{Image, Logs, LogStream, MachineSpec, Mount, MountKind, Platform, Rules};
use serde_json::{Value, json};

use super::{Is, Tool, command_prop, env_prop, prop, strings_prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

const RUN_DEFAULT: Duration = Duration::from_secs(60);
const RUN_MOST: Duration = Duration::from_secs(900);

/// The options `run` and `start_machine` share.
fn machine_options() -> Value {
    json!({
        "image": prop("string", "The image: debian:13, python:3.13-slim, node:22, atlas (Windows), any image a registry has; pulled if it isn't here"),
        "env": env_prop(),
        "workdir": prop("string", "Where the command starts; the image's working folder by default"),
        "user": prop("string", "Who it runs as: a name or uid, optionally :group; the image's USER by default, else root (Linux images)"),
        "network": { "type": "string", "enum": ["nat", "none", "host"], "description": "nat: its own addresses, out through the host (the default); none: loopback only; host: the host's ports" },
        "memory": prop("string", "How much memory it may use: 512m, 2g"),
        "platform": prop("string", "Which build of the image: linux/arm64, linux/amd64 (translated on Apple silicon), windows/arm64"),
        "mounts": strings_prop("Mounts, as docker run's --mount: type=volume,source=NAME,target=/path; type=tmpfs,target=/path; type=bind,source=HOST_PATH,target=/path (a host path under a shared host folder); add ,readonly. Linux images only"),
        "deny_exec": strings_prop("Programs it may not start, by pattern: 'git push *', 'rm -rf *'. Added to the server's own rules"),
        "deny": strings_prop("Where it may not connect: a host, address or network, with :port if wanted (0.0.0.0/0 for everywhere). Added to the server's own rules"),
    })
}

pub(crate) fn tools() -> Vec<Tool> {
    let mut run_props = machine_options();
    run_props["command"] = command_prop("The command: a string, run by the image's shell (sh -c; cmd /c for Windows), or an array, run as it is");
    run_props["timeout_secs"] = prop("integer", "How long it may run: 60 by default, 900 at most");
    let mut start_props = machine_options();
    start_props["name"] = prop("string", "Its name; one is made if not given");
    start_props["command"] = command_prop("What it runs. Without one, a Linux machine runs `sleep infinity`, so it keeps running for exec; set use_image_command to run the image's own instead");
    start_props["use_image_command"] = prop("boolean", "Run the image's own command (a server's, as docker run -d does), not sleep infinity");
    start_props["publish"] = strings_prop("Ports on the host, as docker run's -p: 8080:80, 127.0.0.1:5432:5432, 53:53/udp");
    start_props["publish_all"] = prop("boolean", "Every port the image exposes, each on a free host port");
    start_props["timeout_secs"] = prop("integer", "Stop it this long after it starts; it runs until stopped without one");
    vec![
        Tool {
            name: "run",
            title: "Run a command in a new machine",
            description: "Runs a command in a new machine made from an image, and removes the machine after, as docker run --rm: its exit code, stdout and stderr. For several commands in the same machine, use start_machine and exec.",
            properties: run_props,
            required: &["image", "command"],
            is: Is::OpenWorld,
            on_machine: false,
            host: false,
            run,
        },
        Tool {
            name: "start_machine",
            title: "Start a machine",
            description: "Starts a machine that keeps running, as docker run -d, for exec, the file tools and screens. Its name comes back; stop it with stop_machine, and remove it with remove_machine.",
            properties: start_props,
            required: &["image"],
            is: Is::OpenWorld,
            on_machine: false,
            host: false,
            run: start,
        },
        Tool {
            name: "restart_machine",
            title: "Start a stopped machine again",
            description: "Starts a stopped machine again, with its files as it left them and its mounts and config, as docker start.",
            properties: json!({ "machine": prop("string", "The machine, by name") }),
            required: &["machine"],
            is: Is::Changes,
            on_machine: false,
            host: false,
            run: restart,
        },
        Tool {
            name: "stop_machine",
            title: "Stop a machine",
            description: "Stops a running machine, as docker stop: its image's stop signal, then after ten seconds it's ended. Its files stay, for restart_machine.",
            properties: json!({ "machine": prop("string", "The machine, by name") }),
            required: &["machine"],
            is: Is::Idempotent,
            on_machine: false,
            host: false,
            run: stop,
        },
        Tool {
            name: "remove_machine",
            title: "Remove a machine",
            description: "Removes a machine and its files, as docker rm. A running one only with force, which stops it first.",
            properties: json!({ "machine": prop("string", "The machine, by name"), "force": prop("boolean", "Stop it first if it's running") }),
            required: &["machine"],
            is: Is::Destructive,
            on_machine: false,
            host: false,
            run: remove,
        },
        Tool {
            name: "list_machines",
            title: "List machines",
            description: "Every machine here: name, image, kernel, platform, whether it runs (or how it ended), and its published ports.",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: false,
            host: false,
            run: list,
        },
        Tool {
            name: "inspect_machine",
            title: "Inspect a machine",
            description: "A machine's record: image, kernel, platform, where it runs (a VM or the sandbox), network, published ports, created, and how it ended.",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: inspect,
        },
        Tool {
            name: "machine_logs",
            title: "A machine's logs",
            description: "What a machine started with start_machine wrote to its stdout and stderr, as docker logs: its last lines, or those after a cursor a call before gave back.",
            properties: json!({
                "tail": prop("integer", "How many of the last lines: 200 by default"),
                "cursor": prop("integer", "The cursor a call before gave back: only the lines after it"),
            }),
            required: &[],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: logs,
        },
    ]
}

/// The image a call names, pulled if it isn't here.
pub(crate) fn ensure_image(server: &Server, name: &str, platform: Option<&Platform>, progress: &Progress) -> Result<Image, Failure> {
    if let Some(image) = server.carbon.image(name).filter(|i| platform.is_none_or(|p| &i.platform == p)) {
        return Ok(image);
    }
    let mut said = Instant::now();
    Ok(server.carbon.pull_with(name, platform.cloned(), |p| {
        if said.elapsed() >= Duration::from_millis(500) {
            said = Instant::now();
            progress.say(&format!("pulling {name}: {p}"), None);
        }
    })?)
}

/// A command as the machine runs it: an array as it is; a string by the image's shell.
pub(crate) fn argv(command: &Value, image: Option<&Image>) -> Result<Vec<String>, Failure> {
    argv_with(command, shell(image))
}

/// A command as the machine runs it, a string by `shell`.
pub(crate) fn argv_with(command: &Value, shell: Vec<String>) -> Result<Vec<String>, Failure> {
    match command {
        Value::Array(a) => {
            let argv: Vec<String> = a.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            if argv.is_empty() { Err(Failure("`command` is empty".into())) } else { Ok(argv) }
        }
        Value::String(s) if !s.trim().is_empty() => {
            let mut argv = shell;
            argv.push(s.clone());
            Ok(argv)
        }
        _ => Err(Failure("`command` is needed: a string, or an array".into())),
    }
}

pub(crate) fn shell(image: Option<&Image>) -> Vec<String> {
    match image {
        Some(i) if i.platform.os == "windows" => vec!["cmd.exe".into(), "/c".into()],
        Some(i) if !i.config.shell.is_empty() => i.config.shell.clone(),
        _ => vec!["/bin/sh".into(), "-c".into()],
    }
}

/// The rules a machine gets: the server's, and what the call adds (only refusals).
pub(crate) fn rules(server: &Server, args: &Args) -> Result<Rules, Failure> {
    let mut extra = Rules::new();
    for pattern in args.strings("deny_exec") {
        extra = extra.deny_exec(&pattern);
    }
    for target in args.strings("deny") {
        extra = extra.deny(&target).map_err(|e| Failure(format!("deny {target}: {e}")))?;
    }
    Ok(server.rules.clone().merge(extra))
}

/// A machine as a call describes it.
fn spec(server: &Server, args: &Args, image: &Image) -> Result<MachineSpec, Failure> {
    let mut spec = server.carbon.machine(image.name.clone()).platform(image.platform.clone());
    for (k, v) in args.env("env") {
        spec = spec.env(&k, &v);
    }
    if let Some(w) = args.opt_str("workdir") {
        spec = spec.workdir(w);
    }
    if let Some(u) = args.opt_str("user") {
        spec = spec.user(u);
    }
    match args.opt_str("network") {
        Some(n) => spec = spec.network(carbon::parse_network(&n).ok_or_else(|| Failure(format!("network {n}: nat, none or host")))?),
        None => {
            if let Some(n) = server.options.network {
                spec = spec.network(n);
            }
        }
    }
    if let Some(m) = args.opt_str("memory").or_else(|| server.options.memory.clone()) {
        spec = spec.memory(m);
    }
    for text in args.strings("mounts") {
        let mount = Mount::parse(&text)?;
        if let MountKind::Bind(path) = &mount.kind {
            shape::host_path(&server.host_dirs, &path.display().to_string(), true)?;
        }
        spec = spec.mount(mount);
    }
    Ok(spec.rules(rules(server, args)?))
}

fn platform(args: &Args) -> Result<Option<Platform>, Failure> {
    args.opt_str("platform").map(|p| Platform::parse(&p).ok_or_else(|| Failure(format!("{p} isn't a platform: linux/arm64, linux/amd64, windows/arm64")))).transpose()
}

fn run(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let image = ensure_image(server, &args.str("image")?, platform(args)?.as_ref(), progress)?;
    let command = args.get("command").cloned().unwrap_or(Value::Null);
    let argv = argv(&command, Some(&image))?;
    let timeout = args.timeout(server.options.timeout.unwrap_or(RUN_DEFAULT), RUN_MOST);
    let name = format!("mcp-run-{}", shape::new_id());
    // In a worker of its own, as `carbon run -d`: this process keeps no VM, so it can run many at once.
    let started = Instant::now();
    let machine = spec(server, args, &image)?.name(name.clone()).timeout(timeout).launch(argv)?;
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let mut said = Instant::now();
    match machine.logs(Logs::new().follow()) {
        Ok(lines) => {
            for line in lines {
                if said.elapsed() >= Duration::from_secs(1) {
                    said = Instant::now();
                    progress.say(line.text().trim_end(), None);
                }
                match line.stream {
                    LogStream::Stdout => stdout.extend_from_slice(&line.bytes),
                    LogStream::Stderr => stderr.extend_from_slice(&line.bytes),
                }
            }
        }
        Err(_) => {
            machine.wait(Some(timeout + Duration::from_secs(30)));
        }
    }
    machine.wait(Some(Duration::from_secs(30)));
    let info = machine.info()?;
    let code = info.exit.unwrap_or(-1);
    let timed_out = started.elapsed() >= timeout;
    if info.running {
        let _ = machine.kill();
        machine.wait(Some(Duration::from_secs(10)));
    }
    let _ = machine.remove();
    Ok(output_result(code, &stdout, &stderr, timed_out, json!({ "image": image.short_name() })))
}

/// A command's result, as a model reads it: its exit code, then stdout, then stderr, each clipped.
pub(crate) fn output_result(code: i32, stdout: &[u8], stderr: &[u8], timed_out: bool, mut extra: Value) -> Value {
    let (out, out_cut) = shape::clip(stdout, shape::OUTPUT_CAP);
    let (err, err_cut) = shape::clip(stderr, shape::OUTPUT_CAP);
    let mut text = if timed_out { format!("timed out, and ended (exit code {code})\n") } else { format!("exit code {code}\n") };
    if !out.is_empty() {
        text.push_str(&format!("--- stdout ---\n{out}"));
        if !out.ends_with('\n') {
            text.push('\n');
        }
    }
    if !err.is_empty() {
        text.push_str(&format!("--- stderr ---\n{err}"));
    }
    let structured = json!({ "exit_code": code, "stdout": out, "stderr": err, "stdout_truncated": out_cut, "stderr_truncated": err_cut, "timed_out": timed_out });
    if let (Some(e), Some(s)) = (extra.as_object_mut(), structured.as_object()) {
        for (k, v) in s {
            e.insert(k.clone(), v.clone());
        }
    }
    let mut result = shape::text_and(text, extra);
    if code != 0 || timed_out {
        // A command that failed isn't the tool failing: the model reads the code. It's said in the text.
        result["isError"] = json!(false);
    }
    result
}

fn start(server: &Server, args: &Args, progress: &Progress) -> Outcome {
    let image = ensure_image(server, &args.str("image")?, platform(args)?.as_ref(), progress)?;
    let argv = match args.get("command") {
        Some(c) => argv(c, Some(&image))?,
        None if args.bool("use_image_command") || image.platform.os == "windows" => Vec::new(),
        None => vec!["sleep".into(), "infinity".into()],
    };
    let name = args.opt_str("name").unwrap_or_else(|| format!("mcp-{}", shape::new_id()));
    let mut spec = spec(server, args, &image)?.name(name.clone());
    for p in args.strings("publish") {
        spec = spec.publish(&p);
    }
    if args.bool("publish_all") {
        spec = spec.publish_all();
    }
    if let Some(t) = args.u64("timeout_secs").map(Duration::from_secs).or(server.options.timeout) {
        spec = spec.timeout(t);
    }
    let machine = spec.launch(argv)?;
    super::events::watch(server, machine.name());
    let info = machine.info()?;
    Ok(shape::data(json!({ "machine": info.name, "running": info.running, "backend": info.backend, "ports": ports(&info) })))
}

fn restart(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let name = args.str("machine")?;
    let machine = server.carbon.again(name).launch(Vec::<String>::new())?;
    super::events::watch(server, machine.name());
    let info = machine.info()?;
    Ok(shape::data(json!({ "machine": info.name, "running": info.running, "backend": info.backend, "ports": ports(&info) })))
}

fn stop(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let machine = server.machine(args)?;
    if !machine.is_running() {
        return Ok(shape::text(format!("{} isn't running", machine.name())));
    }
    machine.stop()?;
    let ended = machine.wait(Some(Duration::from_secs(20)));
    let info = machine.info()?;
    Ok(shape::data(json!({ "machine": info.name, "stopped": ended, "exit_code": info.exit })))
}

fn remove(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let machine = server.machine(args)?;
    let name = machine.name().to_string();
    if machine.is_running() {
        if !args.bool("force") {
            return Err(Failure(format!("{name} is running: stop it first, or remove it with force")));
        }
        let _ = machine.kill();
        machine.wait(Some(Duration::from_secs(15)));
    }
    machine.remove()?;
    Ok(shape::text(format!("removed {name}")))
}

fn ports(info: &carbon::MachineInfo) -> Value {
    json!(info.ports.iter().map(|p| json!({ "port": p.port, "protocol": p.protocol, "host": p.host.to_string() })).collect::<Vec<_>>())
}

pub(crate) fn info_json(info: &carbon::MachineInfo) -> Value {
    json!({
        "name": info.name,
        "kernel": info.kernel.name(),
        "image": info.image,
        "program": info.program,
        "platform": info.platform.to_string(),
        "running": info.running,
        "exit_code": info.exit,
        "backend": info.backend,
        "network": carbon::network_name(info.network),
        "publish": info.publish,
        "ports": ports(info),
        "created": shape::time(info.created),
    })
}

fn list(server: &Server, _: &Args, _: &Progress) -> Outcome {
    let machines: Vec<Value> = server.carbon.machines().iter().map(info_json).collect();
    Ok(shape::data(json!({ "machines": machines })))
}

fn inspect(server: &Server, args: &Args, _: &Progress) -> Outcome {
    Ok(shape::data(info_json(&server.machine(args)?.info()?)))
}

fn logs(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let machine = server.machine(args)?;
    let lines: Vec<carbon::LogLine> = machine.logs(Logs::new())?.collect();
    let total = lines.len() as u64;
    let from = match args.u64("cursor") {
        Some(c) => c.min(total),
        None => total.saturating_sub(args.u64("tail").unwrap_or(200)),
    };
    let mut text = Vec::new();
    for line in &lines[from as usize..] {
        if line.stream == LogStream::Stderr {
            text.extend_from_slice(b"[stderr] ");
        }
        text.extend_from_slice(&line.bytes);
    }
    let (text, cut) = shape::clip(&text, shape::OUTPUT_CAP);
    let summary = format!("lines {from} to {total} of {total} (cursor {total})\n");
    Ok(shape::text_and(summary + &text, json!({ "cursor": total, "from": from, "truncated": cut, "text": text })))
}
