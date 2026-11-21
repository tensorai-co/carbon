//! The Engine API: the control plane, for the programs that run models (a sandbox's gateway, CI, an app), not for a
//! model. `carbon mcp --engine` serves it beside MCP: as JSON-RPC methods named `engine/…` on any connection, and as
//! REST under `/v1` over HTTP ([`rest`]).
//!
//! Carbon's nouns: `machines.*` (a machine is a VM or a process sandbox, never a container), `images.*`, `files.*`.
//! Docker's field names where they mean what Docker's do (`Image`, `Hostname`, `Env`, `Cmd`, `WorkingDir`, `User`,
//! `HostConfig`), and Carbon's own where it goes further (`Rules`). Every method is a call or two into the library, as
//! every MCP tool is.
//!
//! Bytes go in binary frames where the connection has them (a WebSocket): a request that names a `Stream` gets its
//! output, or sends its input, as frames on that stream, rather than as base64 in JSON.

pub mod rest;

use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::Duration;

use carbon::{ErrorKind, Mount, Rules};
use serde_json::{Map, Value, json};

use crate::{Kind, Peer, Server};

/// Why a method failed: what kind of failure (as Carbon's errors have them), and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    pub kind: &'static str,
    pub message: String,
}

impl EngineError {
    fn new(kind: &'static str, message: impl Into<String>) -> EngineError {
        EngineError { kind, message: message.into() }
    }
    fn invalid(message: impl Into<String>) -> EngineError {
        EngineError::new("invalid", message)
    }
    fn not_found(message: impl Into<String>) -> EngineError {
        EngineError::new("not_found", message)
    }

    /// Its JSON-RPC error code.
    pub fn rpc_code(&self) -> i64 {
        match self.kind {
            "invalid" => -32602,
            "not_found" => -32004,
            "conflict" => -32009,
            "forbidden" => crate::protocol::FORBIDDEN,
            "unsupported" => -32005,
            "no_method" => -32601,
            _ => -32000,
        }
    }

    /// Its HTTP status, for REST.
    pub fn status(&self) -> u16 {
        match self.kind {
            "invalid" => 400,
            "forbidden" => 403,
            "not_found" | "no_method" => 404,
            "conflict" => 409,
            "unsupported" => 501,
            _ => 500,
        }
    }
}

impl From<carbon::Error> for EngineError {
    fn from(e: carbon::Error) -> EngineError {
        let kind = match e.kind() {
            ErrorKind::NotFound => "not_found",
            ErrorKind::AlreadyExists | ErrorKind::Running | ErrorKind::NotRunning => "conflict",
            ErrorKind::InvalidInput => "invalid",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Denied => "forbidden",
            _ => "failed",
        };
        EngineError::new(kind, e.message())
    }
}

type R = Result<Value, EngineError>;

/// A request's params.
struct P<'a>(&'a Value);

impl P<'_> {
    fn opt_str(&self, key: &str) -> Option<String> {
        self.0.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
    }
    fn str(&self, key: &str) -> Result<String, EngineError> {
        self.opt_str(key).ok_or_else(|| EngineError::invalid(format!("{key} is needed")))
    }
    fn bool(&self, key: &str) -> bool {
        match self.0.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.as_str(), "1" | "true"),
            Some(Value::Number(n)) => n.as_u64() == Some(1),
            _ => false,
        }
    }
    fn u64(&self, key: &str) -> Option<u64> {
        match self.0.get(key) {
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::String(s)) => s.parse().ok(),
            _ => None,
        }
    }
    fn strings(&self, key: &str) -> Result<Vec<String>, EngineError> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(String::from).ok_or_else(|| EngineError::invalid(format!("{key} must be strings")))).collect(),
            Some(Value::String(s)) => Ok(vec![s.clone()]),
            Some(_) => Err(EngineError::invalid(format!("{key} must be a list of strings"))),
        }
    }
    fn object(&self, key: &str) -> Option<&Map<String, Value>> {
        self.0.get(key).and_then(Value::as_object)
    }
    fn sub(&self, key: &str) -> Value {
        self.0.get(key).cloned().unwrap_or(Value::Null)
    }
}

/// One `engine/…` method, by its name without `engine/`.
pub fn call(server: &Server, peer: &Arc<Peer>, method: &str, params: &Value) -> R {
    let p = P(params);
    match method {
        "system.version" => Ok(version()),
        "system.ping" => Ok(json!("OK")),
        "machines.create" => create(server, &p),
        "machines.start" => start(server, &p),
        "machines.stop" => stop(server, &p),
        "machines.kill" => kill(server, &p),
        "machines.wait" => wait(server, &p),
        "machines.remove" => remove(server, &p),
        "machines.inspect" => inspect(server, &p),
        "machines.list" => list_machines(server, &p),
        "machines.logs" => logs(server, &p),
        "machines.commit" => commit(server, &p),
        "machines.exec" => exec(server, peer, &p),
        "files.stat" => stat(server, &p),
        "files.list" => list_files(server, &p),
        "files.read" => read_file(server, peer, &p),
        "files.write" => write_file(server, peer, &p),
        "files.remove" => remove_file(server, &p),
        "images.pull" => pull(server, &p),
        "images.push" => push(server, &p),
        "images.login" => login(server, &p),
        "images.logout" => logout(server, &p),
        "images.tag" => tag(server, &p),
        "images.remove" => remove_image(server, &p),
        "images.inspect" => inspect_image(server, &p),
        "images.list" => Ok(Value::Array(server.carbon().images().iter().map(image_row).collect())),
        _ => Err(EngineError::new("no_method", format!("engine/{method} isn't a method of the Engine API"))),
    }
}

fn version() -> Value {
    json!({
        "Version": env!("CARGO_PKG_VERSION"),
        "ApiVersion": "1",
        "Os": std::env::consts::OS,
        "Arch": std::env::consts::ARCH,
    })
}

fn rfc3339(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    images::rfc3339(secs)
}

fn machine(server: &Server, p: &P) -> Result<carbon::Machine, EngineError> {
    let id = p.str("Id")?;
    Ok(server.carbon().get(&id)?)
}

/// `Rules`, Carbon's own: `{"Deny": ["0.0.0.0/0"], "Allow": ["pypi.org:443"], "DenyExec": ["rm -rf *"]}`.
fn rules(v: &Value) -> Result<Rules, EngineError> {
    let mut rules = Rules::new();
    if v.is_null() {
        return Ok(rules);
    }
    let p = P(v);
    for target in p.strings("Deny")? {
        rules = rules.deny(&target).map_err(|e| EngineError::invalid(format!("Rules.Deny {target}: {e}")))?;
    }
    for target in p.strings("Allow")? {
        rules = rules.allow(&target).map_err(|e| EngineError::invalid(format!("Rules.Allow {target}: {e}")))?;
    }
    for pattern in p.strings("DenyExec")? {
        rules = rules.deny_exec(&pattern);
    }
    for pattern in p.strings("DenyRegistry")? {
        rules = rules.deny_registry(&pattern).map_err(|e| EngineError::invalid(format!("Rules.DenyRegistry {pattern}: {e}")))?;
    }
    Ok(rules)
}

/// A machine's registries, as `machines.create` takes them.
fn registries(p: &P) -> Result<carbon::Registries, EngineError> {
    let bad = |e: String| EngineError::invalid(format!("Registries: {e}"));
    let mut r = carbon::Registries::new(p.strings("Services")?);
    if let Some(url) = p.opt_str("Upstream") {
        r = r.upstream(&url).map_err(bad)?;
    }
    for (service, url) in p.object("Upstreams").into_iter().flatten() {
        let url = url.as_str().ok_or_else(|| EngineError::invalid(format!("Registries.Upstreams.{service} must be a string")))?;
        r = r.upstream_for_service(service, url).map_err(bad)?;
    }
    if let Some(token) = p.opt_str("Token") {
        r = r.token(token);
    }
    for (key, value) in p.object("Meta").into_iter().flatten() {
        let value = value.as_str().ok_or_else(|| EngineError::invalid(format!("Registries.Meta.{key} must be a string")))?;
        r = r.meta(key.clone(), value);
    }
    Ok(r)
}

fn create(server: &Server, p: &P) -> R {
    let image = p.str("Image")?;
    for unsupported in ["Entrypoint", "ExposedPorts", "Healthcheck", "StopSignal"] {
        if p.0.get(unsupported).is_some_and(|v| !v.is_null()) {
            return Err(EngineError::new("unsupported", format!("{unsupported} isn't supported by machines.create yet")));
        }
    }
    let mut spec = server.carbon().machine(image);
    if let Some(name) = p.opt_str("Name") {
        spec = spec.name(name);
    }
    if let Some(h) = p.opt_str("Hostname") {
        spec = spec.hostname(h);
    }
    if let Some(labels) = p.object("Labels") {
        for (key, value) in labels {
            let value = value.as_str().ok_or_else(|| EngineError::invalid(format!("Labels.{key} must be a string")))?;
            spec = spec.label(key.clone(), value);
        }
    }
    if let Some(text) = p.opt_str("Platform") {
        spec = spec.platform(carbon::Platform::parse(&text).ok_or_else(|| EngineError::invalid(format!("Platform {text}: linux/arm64, linux/amd64, windows/amd64…")))?);
    }
    for var in p.strings("Env")? {
        let (k, v) = var.split_once('=').ok_or_else(|| EngineError::invalid(format!("Env {var}: NAME=VALUE")))?;
        spec = spec.env(k, v);
    }
    if let Some(w) = p.opt_str("WorkingDir") {
        spec = spec.workdir(w);
    }
    if let Some(u) = p.opt_str("User") {
        spec = spec.user(u);
    }
    let host = P(&p.sub("HostConfig"));
    match host.opt_str("NetworkMode") {
        Some(mode) => {
            let mode = if mode == "bridge" || mode == "default" { "nat".to_string() } else { mode };
            spec = spec.network(carbon::parse_network(&mode).ok_or_else(|| EngineError::invalid(format!("HostConfig.NetworkMode {mode}: nat, none or host")))?);
        }
        None => {
            if let Some(n) = server.options().network {
                spec = spec.network(n);
            }
        }
    }
    match host.u64("Memory") {
        Some(bytes) if bytes > 0 => spec = spec.memory(bytes),
        _ => {
            if let Some(m) = server.options().memory.clone() {
                spec = spec.memory(m);
            }
        }
    }
    // `Registries`: {"Services": ["apt"], "Upstream": URL, "Upstreams": {"npm": URL}, "Token": T, "Meta": {"sid": …}}.
    match p.0.get("Registries").filter(|r| !r.is_null()) {
        Some(r) => spec = spec.registries(registries(&P(r))?),
        None => {
            if let Some(r) = &server.options().registries {
                spec = spec.registries(r.clone());
            }
        }
    }
    for text in host.strings("ExtraHosts")? {
        let extra = carbon::ExtraHost::parse(&text)?;
        spec = spec.add_host(extra.name, extra.ip);
    }
    if let Some(tmpfs) = host.object("Tmpfs") {
        for target in tmpfs.keys() {
            spec = spec.mount(Mount::tmpfs(target.clone()));
        }
    }
    for bind in host.strings("Binds")? {
        // Docker's SOURCE:TARGET[:ro]: a host folder, or a volume's name.
        let parts: Vec<&str> = bind.split(':').collect();
        let (source, target, ro) = match parts.as_slice() {
            [s, t] => (*s, *t, false),
            [s, t, o] => (*s, *t, *o == "ro"),
            _ => return Err(EngineError::invalid(format!("HostConfig.Binds {bind}: SOURCE:TARGET[:ro]"))),
        };
        let mount = if source.starts_with('/') || source.starts_with('.') || source.starts_with('~') {
            Mount::bind(server.host_path(source, true).map_err(EngineError::invalid)?, target)
        } else {
            Mount::volume(source, target)
        };
        spec = spec.mount(if ro { mount.read_only() } else { mount });
    }
    if let Some(mounts) = host.0.get("Mounts").and_then(Value::as_array) {
        for m in mounts {
            let m = P(m);
            let target = m.str("Target")?;
            let mount = match m.opt_str("Type").as_deref().unwrap_or("volume") {
                "bind" => Mount::bind(server.host_path(&m.str("Source")?, true).map_err(EngineError::invalid)?, target),
                "volume" => Mount::volume(m.str("Source")?, target),
                "tmpfs" => {
                    // Files it starts with: {"Path": "config.toml", "Content": "…", "Mode": 420}. In memory only, so
                    // never on the machine's disk or in an image committed from it (a secret, a tool's config).
                    let mut tmpfs = Mount::tmpfs(target.clone());
                    for f in m.0.get("Files").and_then(Value::as_array).into_iter().flatten() {
                        let f = P(f);
                        let path = f.str("Path")?;
                        if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == ".." || p == ".") {
                            return Err(EngineError::invalid(format!("HostConfig.Mounts {target} Files Path {path}: a path under the tmpfs")));
                        }
                        tmpfs = tmpfs.with_file_mode(path, f.str("Content")?.into_bytes(), f.u64("Mode").unwrap_or(0o644) as u32);
                    }
                    tmpfs
                }
                other => return Err(EngineError::invalid(format!("HostConfig.Mounts Type {other}: bind, volume or tmpfs"))),
            };
            spec = spec.mount(if m.bool("ReadOnly") { mount.read_only() } else { mount });
        }
    }
    let rules = server.rules().clone().merge(rules(&p.sub("Rules"))?);
    let cmd = p.strings("Cmd")?;
    let machine = spec.rules(rules).create_with(cmd)?;
    Ok(json!({ "Id": machine.name(), "Warnings": [] }))
}

fn start(server: &Server, p: &P) -> R {
    let id = p.str("Id")?;
    let mut spec = server.carbon().again(id.clone()).rules(server.rules().clone());
    if let Some(t) = server.options().timeout {
        spec = spec.timeout(t);
    }
    spec.launch(Vec::<String>::new())?;
    Ok(json!({}))
}

/// As Docker's: asks it to stop, waits `Timeout` seconds (10 by default), then kills it; answers once it has stopped.
fn stop(server: &Server, p: &P) -> R {
    let m = machine(server, p)?;
    m.stop()?;
    let grace = Duration::from_secs(p.u64("Timeout").unwrap_or(10));
    if !m.wait(Some(grace)) {
        m.kill()?;
        if !m.wait(Some(Duration::from_secs(30))) {
            return Err(EngineError::new("failed", format!("{} didn't stop", m.name())));
        }
    }
    Ok(json!({}))
}

fn kill(server: &Server, p: &P) -> R {
    machine(server, p)?.kill()?;
    Ok(json!({}))
}

fn wait(server: &Server, p: &P) -> R {
    let m = machine(server, p)?;
    let ended = m.wait(p.u64("Timeout").map(Duration::from_secs));
    let info = m.info()?;
    Ok(json!({ "StatusCode": info.exit.unwrap_or(0), "Ended": ended }))
}

fn remove(server: &Server, p: &P) -> R {
    let m = machine(server, p)?;
    if m.is_running() {
        if !p.bool("Force") {
            return Err(EngineError::new("conflict", format!("{} is running: stop it first, or Force", m.name())));
        }
        m.kill()?;
        m.wait(Some(Duration::from_secs(10)));
    }
    m.remove()?;
    Ok(json!({}))
}

/// Every machine, or with `Label` (`KEY` or `KEY=VALUE`, one or a list) those with every one of them.
fn list_machines(server: &Server, p: &P) -> R {
    let wanted: Vec<(String, Option<String>)> = p
        .strings("Label")?
        .into_iter()
        .map(|l| match l.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (l, None),
        })
        .collect();
    let has = |m: &carbon::MachineInfo| wanted.iter().all(|(k, v)| m.config.labels.get(k).is_some_and(|have| v.as_ref().is_none_or(|v| v == have)));
    Ok(Value::Array(server.carbon().machines().iter().filter(|m| has(m)).map(machine_row).collect()))
}

fn machine_row(info: &carbon::MachineInfo) -> Value {
    let state = if info.running { "running" } else if info.exit.is_some() { "exited" } else { "created" };
    json!({
        "Id": info.name,
        "Name": info.name,
        "Image": info.image.clone().or_else(|| info.program.clone()).unwrap_or_default(),
        "Created": rfc3339(info.created),
        "State": state,
        "Platform": info.platform.to_string(),
        "Labels": info.config.labels,
    })
}

fn inspect(server: &Server, p: &P) -> R {
    let info = machine(server, p)?.info()?;
    let status = if info.running { "running" } else if info.exit.is_some() { "exited" } else { "created" };
    Ok(json!({
        "Id": info.name,
        "Name": info.name,
        "Image": info.image.clone().or_else(|| info.program.clone()).unwrap_or_default(),
        "Platform": info.platform.to_string(),
        "Created": rfc3339(info.created),
        "State": { "Status": status, "Running": info.running, "ExitCode": info.exit.unwrap_or(0), "Pid": info.pid.unwrap_or(0) },
        "Config": {
            "Hostname": info.hostname, "Env": info.config.env, "Cmd": info.config.cmd, "Entrypoint": info.config.entrypoint,
            "WorkingDir": info.config.workdir, "User": info.config.user, "Labels": info.config.labels,
        },
        "HostConfig": {
            "NetworkMode": carbon::network_name(info.network),
            "ExtraHosts": info.extra_hosts.iter().map(ToString::to_string).collect::<Vec<_>>(),
        },
        "NetworkSettings": { "Ports": info.ports.iter().map(|port| json!(format!("{port:?}"))).collect::<Vec<_>>() },
        "Backend": info.backend,
    }))
}

fn logs(server: &Server, p: &P) -> R {
    let m = machine(server, p)?;
    let mut how = carbon::Logs::new();
    if let Some(n) = p.u64("Tail") {
        how = how.tail(n as usize);
    }
    let lines: Vec<Value> = m.logs(how)?.map(|l| json!({ "Stream": format!("{:?}", l.stream).to_ascii_lowercase(), "Text": l.text() })).collect();
    Ok(Value::Array(lines))
}

fn commit(server: &Server, p: &P) -> R {
    let m = machine(server, p)?;
    let mut how = carbon::Commit::new();
    if let Some(repo) = p.opt_str("Repo") {
        let tag = p.opt_str("Tag").unwrap_or_else(|| "latest".into());
        how = how.name(format!("{repo}:{tag}"));
    }
    if let Some(c) = p.opt_str("Comment") {
        how = how.comment(c);
    }
    for change in p.strings("Changes")? {
        how = how.change(change);
    }
    let image = m.commit_with(&how)?;
    Ok(json!({ "Id": image.name }))
}

/// Opens a stream a request names: its frames from the client come here, until it's closed.
fn open_stream(peer: &Arc<Peer>, p: &P) -> Result<Option<(u32, crate::Frames)>, EngineError> {
    let Some(stream) = p.u64("Stream") else { return Ok(None) };
    if !peer.has_frames() {
        return Err(EngineError::invalid("Stream needs a connection with binary frames: a WebSocket"));
    }
    let stream = u32::try_from(stream).map_err(|_| EngineError::invalid("Stream is a u32"))?;
    if let Some(rx) = peer.take_stream(stream) {
        return Ok(Some((stream, rx)));
    }
    let (tx, rx) = channel();
    let mut streams = peer.streams.lock().unwrap_or_else(|e| e.into_inner());
    if streams.contains_key(&stream) {
        return Err(EngineError::invalid(format!("Stream {stream} is open already")));
    }
    streams.insert(stream, tx);
    Ok(Some((stream, rx)))
}

fn close_stream(peer: &Peer, stream: u32) {
    peer.streams.lock().unwrap_or_else(|e| e.into_inner()).remove(&stream);
}

fn exec(server: &Server, peer: &Arc<Peer>, p: &P) -> R {
    let m = machine(server, p)?;
    let cmd = p.strings("Cmd")?;
    if cmd.is_empty() {
        return Err(EngineError::invalid("Cmd is needed"));
    }
    let mut spec = m.exec(cmd);
    for var in p.strings("Env")? {
        let (k, v) = var.split_once('=').ok_or_else(|| EngineError::invalid(format!("Env {var}: NAME=VALUE")))?;
        spec = spec.env(k, v);
    }
    if let Some(w) = p.opt_str("WorkingDir") {
        spec = spec.workdir(w);
    }
    if let Some(u) = p.opt_str("User") {
        spec = spec.user(u);
    }
    if let Some(t) = p.u64("Timeout") {
        spec = spec.timeout(Duration::from_secs(t));
    }
    let Some((stream, input)) = open_stream(peer, p)? else {
        // No stream: run it to its end, and give its output in the answer.
        let out = spec.output()?;
        return Ok(json!({ "ExitCode": out.exit.code(), "Stdout": out.stdout_lossy(), "Stderr": out.stderr_lossy(), "TimedOut": out.timed_out }));
    };
    let result = (|| -> R {
        let mut process = spec.spawn()?;
        let handle = process.handle()?;
        // The client's stdin frames, to the process, until it says it's done.
        std::thread::spawn(move || {
            while let Ok((kind, bytes)) = input.recv() {
                match kind {
                    Kind::Stdin | Kind::Data => {
                        if handle.write_stdin(&bytes).is_err() {
                            break;
                        }
                    }
                    Kind::End => {
                        handle.close_stdin();
                        break;
                    }
                    _ => {}
                }
            }
        });
        let mut code = -1;
        while let Some(event) = process.next_event() {
            match event {
                carbon::ProcessEvent::Stdout(b) => {
                    peer.out.frame(stream, Kind::Stdout, &b);
                }
                carbon::ProcessEvent::Stderr(b) => {
                    peer.out.frame(stream, Kind::Stderr, &b);
                }
                carbon::ProcessEvent::Exit(c) => {
                    code = c;
                    break;
                }
            }
        }
        peer.out.frame(stream, Kind::Exit, &code.to_be_bytes());
        Ok(json!({ "ExitCode": code }))
    })();
    close_stream(peer, stream);
    result
}

fn file_kind(kind: carbon::FileKind) -> &'static str {
    match kind {
        carbon::FileKind::File => "file",
        carbon::FileKind::Dir => "dir",
        carbon::FileKind::Symlink => "symlink",
    }
}

fn file_json(info: &carbon::FileInfo) -> Value {
    json!({ "Kind": file_kind(info.kind), "Size": info.size, "Mode": info.mode, "Uid": info.uid, "Gid": info.gid, "Modified": rfc3339(info.modified), "Link": info.link })
}

fn stat(server: &Server, p: &P) -> R {
    let path = p.str("Path")?;
    Ok(file_json(&machine(server, p)?.files().stat(&path)?))
}

fn list_files(server: &Server, p: &P) -> R {
    let path = p.str("Path")?;
    let entries = machine(server, p)?.files().list(&path)?;
    Ok(Value::Array(entries.iter().map(|e| {
        let mut v = file_json(&e.info);
        v["Name"] = json!(e.name);
        v
    }).collect()))
}

fn read_file(server: &Server, peer: &Arc<Peer>, p: &P) -> R {
    use base64::Engine as _;
    let path = p.str("Path")?;
    let files = machine(server, p)?.files();
    let size = files.stat(&path)?.size;
    let stream = open_stream(peer, p)?;
    let mut at = 0u64;
    let mut whole = Vec::new();
    let result = (|| -> Result<(), EngineError> {
        while at < size {
            let end = (at + carbon::MOST_READ).min(size);
            let chunk = files.read(&path, at..end)?;
            if chunk.is_empty() {
                break;
            }
            at += chunk.len() as u64;
            match &stream {
                Some((id, _)) => {
                    peer.out.frame(*id, Kind::Data, &chunk);
                }
                None => whole.extend_from_slice(&chunk),
            }
        }
        Ok(())
    })();
    if let Some((id, _)) = &stream {
        peer.out.frame(*id, Kind::End, &[]);
        close_stream(peer, *id);
    }
    result?;
    Ok(match stream {
        Some(_) => json!({ "Size": at }),
        None => json!({ "Size": at, "Data": base64::engine::general_purpose::STANDARD.encode(whole) }),
    })
}

fn write_file(server: &Server, peer: &Arc<Peer>, p: &P) -> R {
    use base64::Engine as _;
    let path = p.str("Path")?;
    let files = machine(server, p)?.files();
    let data = match open_stream(peer, p)? {
        Some((id, input)) => {
            let mut data = Vec::new();
            let got = loop {
                match input.recv_timeout(Duration::from_secs(300)) {
                    Ok((Kind::Data, bytes)) => data.extend_from_slice(&bytes),
                    Ok((Kind::End, _)) => break Ok(()),
                    Ok(_) => {}
                    Err(_) => break Err(EngineError::invalid(format!("Stream {id} ended without an end frame"))),
                }
            };
            close_stream(peer, id);
            got?;
            data
        }
        None => match p.opt_str("Data") {
            Some(text) => base64::engine::general_purpose::STANDARD.decode(text).map_err(|e| EngineError::invalid(format!("Data isn't base64: {e}")))?,
            None => Vec::new(),
        },
    };
    let mut how = carbon::Write::new().parents(p.bool("Parents"));
    if let Some(mode) = p.u64("Mode") {
        how = how.mode(mode as u32);
    }
    if let Some(owner) = p.opt_str("Owner") {
        how = how.owner(&owner);
    }
    files.write(&path, &data, how)?;
    Ok(json!({ "Size": data.len() }))
}

fn remove_file(server: &Server, p: &P) -> R {
    let path = p.str("Path")?;
    let how = if p.bool("Recursive") { carbon::Remove::recursive() } else { carbon::Remove::new() };
    machine(server, p)?.files().remove(&path, how)?;
    Ok(json!({}))
}

fn image_name(p: &P) -> Result<String, EngineError> {
    let name = p.opt_str("Name").ok_or_else(|| EngineError::invalid("Name is needed"))?;
    Ok(match p.opt_str("Tag") {
        Some(tag) if !name.contains('@') && !name.rsplit('/').next().unwrap_or("").contains(':') => format!("{name}:{tag}"),
        _ => name,
    })
}

fn pull(server: &Server, p: &P) -> R {
    let name = image_name(p)?;
    let platform = match p.opt_str("Platform") {
        Some(text) => Some(carbon::Platform::parse(&text).ok_or_else(|| EngineError::invalid(format!("Platform {text}: linux/arm64, linux/amd64…")))?),
        None => None,
    };
    let image = server.carbon().pull_with(&name, platform, |_| {})?;
    Ok(json!({ "Id": image.name, "Platform": image.platform.to_string() }))
}

fn push(server: &Server, p: &P) -> R {
    let name = image_name(p)?;
    let mut how = carbon::Push::new().squash(p.bool("Squash"));
    if let Some(to) = p.opt_str("To") {
        how = how.to(to);
    }
    let pushed = server.carbon().push_with(&name, &how, |_| {})?;
    Ok(json!({ "Name": pushed.name, "Digest": pushed.digest, "Layers": pushed.layers, "Sent": pushed.sent }))
}

/// A login to a registry for this server's pulls and pushes, as `carbon login`: checked with the registry, then kept
/// in the server's home. The secret never comes back.
fn login(server: &Server, p: &P) -> R {
    let registry = p.str("Registry")?;
    let user = p.str("Username")?;
    let secret = p.str("Password")?;
    server.carbon().login(&registry, &user, &secret)?;
    Ok(json!({ "Status": "Login Succeeded" }))
}

fn logout(server: &Server, p: &P) -> R {
    let registry = p.str("Registry")?;
    Ok(json!({ "LoggedOut": server.carbon().logout(&registry)? }))
}

fn tag(server: &Server, p: &P) -> R {
    let source = p.str("Source")?;
    let repo = p.str("Repo")?;
    let target = match p.opt_str("Tag") {
        Some(tag) => format!("{repo}:{tag}"),
        None => repo,
    };
    let image = server.carbon().tag(&source, &target)?;
    Ok(json!({ "Id": image.name }))
}

fn remove_image(server: &Server, p: &P) -> R {
    let name = image_name(p)?;
    server.carbon().remove_image(&name)?;
    Ok(json!([{ "Untagged": name }]))
}

fn image_row(i: &carbon::Image) -> Value {
    json!({ "Id": i.name, "RepoTags": [i.short_name()], "Platform": i.platform.to_string(), "Created": images::rfc3339(i.created), "Source": i.source })
}

fn inspect_image(server: &Server, p: &P) -> R {
    let name = image_name(p)?;
    let image = server.carbon().image(&name).ok_or_else(|| EngineError::not_found(format!("there's no image {name} here")))?;
    let c = &image.config;
    let history: Vec<Value> = image
        .history
        .iter()
        .map(|h| json!({ "Created": h.created, "CreatedBy": h.created_by, "EmptyLayer": h.empty_layer, "Comment": h.comment }))
        .collect();
    let mut row = image_row(&image);
    row["Config"] = json!({
        "Env": c.env, "Entrypoint": c.entrypoint, "Cmd": c.cmd, "WorkingDir": c.workdir, "User": c.user,
        "Labels": c.labels, "ExposedPorts": c.exposed_ports, "Volumes": c.volumes, "StopSignal": c.stop_signal,
    });
    row["History"] = Value::Array(history);
    Ok(row)
}
