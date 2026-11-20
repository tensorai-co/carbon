//! The control protocol: how a running machine is reached from outside its process, by `carbon` commands, by
//! [`crate::Machine`] in another program, and by clients in other languages. It's a Unix socket in the host's
//! temporary folder (a socket's path has to be short), named in the machine's folder as `control`.
//!
//! One request per connection: a line, then the answer: `ok <length>` and that many bytes, or `error <why>`.
//!
//! | Request | Answer |
//! | --- | --- |
//! | `screenshot` | the screen, as a PNG |
//! | `windows` | its windows, top first, as a table; `windows-json`: as a JSON array |
//! | `move X Y`, `click X Y [BUTTON] [COUNT]`, `drag X1 Y1 X2 Y2`, `scroll X Y NOTCHES [SIDEWAYS]`, `type TEXT`, `key COMBO...` | input, as a user's |
//! | `resize WxH[@SCALE]`, `clipboard [TEXT]`, `drop X Y text TEXT`, `drop X Y files PATHS` | the desktop's |
//! | `stop [SIGNAL]`, `kill` | asks the machine to end (as `docker stop`), or ends it now |
//! | `exec <json>` | starts a process; then frames, below |
//! | `fs <json>` | an operation on its files (below) |
//! | `watch <json>` | then a JSON line for each hooked event, as it's decided, until either end closes |
//!
//! Text in a request is one line: `%0A` is a new line and `%25` a `%`.
//!
//! `fs <json>` is an operation on the machine's files (`crate::files::Op`, as JSON): a `write`'s bytes follow its line;
//! its answer is a `read`'s bytes, or JSON.
//!
//! `exec`'s request is `{"args": [...], "env": [...], "workdir": "...", "tty": [rows, cols], "detach": false}`. After
//! its `ok 0`, the connection carries frames both ways, a kind byte, a 4-byte little-endian length and that many
//! bytes. In: stdin (0), stdin's end (1), a terminal size (2: rows and columns, 2 bytes each), a signal (3: 4 bytes).
//! Out: stdout (1), stderr (2), and its exit code (3: 4 bytes), last.
//!
//! `watch`'s request is `{"kinds": ["exec", "output", "connect", "lookup"]}`; each line after its `ok 0` is an
//! event as [`observed_json`] writes it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::stop::Stopper;

/// What `exec` asks a running machine to start.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExecRequest {
    pub args: Vec<String>,
    /// `NAME=value`, over the machine's own environment.
    pub env: Vec<String>,
    pub workdir: Option<String>,
    /// A terminal of its own, this many rows and columns.
    pub tty: Option<(u16, u16)>,
    /// Started and left running: nothing comes back but whether it started.
    pub detach: bool,
    /// Who it runs as (`name[:group]`, `uid[:gid]`): the machine's first program's user if None.
    pub user: Option<String>,
}

/// What comes out of a process `exec` started.
pub(crate) enum Output {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Exit(i32),
}

/// A process `exec` started, whichever kernel's.
pub(crate) trait Session: Send + Sync {
    fn input(&self, data: &[u8]);
    fn close_input(&self);
    fn resize(&self, rows: u16, cols: u16);
    fn signal(&self, signal: i32);
    fn next(&self, timeout: Duration) -> Option<Output>;
}

/// How a machine starts what `exec` asks for.
pub(crate) type Enter = Arc<dyn Fn(ExecRequest) -> std::result::Result<Box<dyn Session>, String> + Send + Sync>;

impl Session for lambda::Entered {
    fn input(&self, data: &[u8]) {
        lambda::Entered::input(self, data);
    }
    fn close_input(&self) {
        lambda::Entered::close_input(self);
    }
    fn resize(&self, rows: u16, cols: u16) {
        lambda::Entered::resize(self, rows, cols);
    }
    fn signal(&self, signal: i32) {
        lambda::Entered::signal(self, signal);
    }
    fn next(&self, timeout: Duration) -> Option<Output> {
        lambda::Entered::next(self, timeout).map(|e| match e {
            lambda::Event::Out(d) => Output::Out(d),
            lambda::Event::Err(d) => Output::Err(d),
            lambda::Event::Exit(c) => Output::Exit(c),
        })
    }
}

impl Session for atlas::Entered {
    fn input(&self, data: &[u8]) {
        atlas::Entered::input(self, data);
    }
    fn close_input(&self) {
        atlas::Entered::close_input(self);
    }
    fn resize(&self, rows: u16, cols: u16) {
        atlas::Entered::resize(self, rows, cols);
    }
    fn signal(&self, signal: i32) {
        atlas::Entered::signal(self, signal);
    }
    fn next(&self, timeout: Duration) -> Option<Output> {
        atlas::Entered::next(self, timeout).map(|e| match e {
            atlas::Event::Out(d) => Output::Out(d),
            atlas::Event::Err(d) => Output::Err(d),
            atlas::Event::Exit(c) => Output::Exit(c),
        })
    }
}

/// An `exec` its hooks answered: no process runs; it gives the answer's output and code, and ends.
pub(crate) struct Answered(std::sync::Mutex<std::collections::VecDeque<Output>>);

impl Answered {
    pub fn new(reply: hooks::Reply) -> Answered {
        let mut out = std::collections::VecDeque::new();
        if !reply.stdout.is_empty() {
            out.push_back(Output::Out(reply.stdout));
        }
        if !reply.stderr.is_empty() {
            out.push_back(Output::Err(reply.stderr));
        }
        out.push_back(Output::Exit(reply.code));
        Answered(std::sync::Mutex::new(out))
    }
}

impl Session for Answered {
    fn input(&self, _data: &[u8]) {}
    fn close_input(&self) {}
    fn resize(&self, _rows: u16, _cols: u16) {}
    fn signal(&self, _signal: i32) {}
    fn next(&self, timeout: Duration) -> Option<Output> {
        let next = self.0.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
        if next.is_none() {
            std::thread::sleep(timeout);
        }
        next
    }
}

/// A session whose output goes through the machine's hooks, as `Source::Exec(id)`.
pub(crate) struct Hooked {
    inner: Box<dyn Session>,
    hub: hooks::Hub,
    id: u64,
}

impl Hooked {
    pub fn new(inner: Box<dyn Session>, hub: hooks::Hub) -> Hooked {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Hooked { inner, hub, id: NEXT.fetch_add(1, Ordering::Relaxed) }
    }
}

impl Session for Hooked {
    fn input(&self, data: &[u8]) {
        self.inner.input(data);
    }
    fn close_input(&self) {
        self.inner.close_input();
    }
    fn resize(&self, rows: u16, cols: u16) {
        self.inner.resize(rows, cols);
    }
    fn signal(&self, signal: i32) {
        self.inner.signal(signal);
    }
    fn next(&self, timeout: Duration) -> Option<Output> {
        let next = self.inner.next(timeout)?;
        if !self.hub.wants(hooks::Kind::Output) {
            return Some(next);
        }
        let (stream, bytes) = match next {
            Output::Out(b) => (hooks::Stream::Stdout, b),
            Output::Err(b) => (hooks::Stream::Stderr, b),
            exit => return Some(exit),
        };
        let wrap = |b| if stream == hooks::Stream::Stdout { Output::Out(b) } else { Output::Err(b) };
        match self.hub.output(hooks::Output { stream, source: hooks::Source::Exec(self.id), bytes: bytes.clone() }) {
            // Dropped: nothing to give this time.
            hooks::Decision::Deny => Some(wrap(Vec::new())),
            hooks::Decision::Rewrite(o) => Some(wrap(o.bytes)),
            _ => Some(wrap(bytes)),
        }
    }
}

pub(crate) fn request_json(r: &ExecRequest) -> String {
    json!({ "args": r.args, "env": r.env, "workdir": r.workdir, "tty": r.tty.map(|(rows, cols)| [rows, cols]), "detach": r.detach, "user": r.user }).to_string()
}

fn request_from_json(text: &str) -> Option<ExecRequest> {
    let v: Value = serde_json::from_str(text).ok()?;
    let list = |key: &str| v.get(key).and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
    let tty = v.get("tty").and_then(|t| t.as_array()).and_then(|t| Some((t.first()?.as_u64()? as u16, t.get(1)?.as_u64()? as u16)));
    Some(ExecRequest { args: list("args"), env: list("env"), workdir: v.get("workdir").and_then(|w| w.as_str()).map(String::from), tty, detach: v.get("detach").and_then(|d| d.as_bool()).unwrap_or(false), user: v.get("user").and_then(|u| u.as_str()).map(String::from) })
}

// ---------------------------------------------------------------------------------------------------------------
// Events as JSON

fn ip_list(ips: &[std::net::IpAddr]) -> Value {
    Value::from(ips.iter().map(|ip| ip.to_string()).collect::<Vec<_>>())
}

/// An event as JSON: `{"kind": "exec", ...its fields}`. Output's bytes are given as text, any that aren't UTF-8
/// replaced.
pub fn event_json(event: &hooks::Event) -> Value {
    match event {
        hooks::Event::Exec(e) => json!({
            "kind": "exec",
            "os": if e.os == hooks::Os::Linux { "linux" } else { "windows" },
            "origin": match e.origin { hooks::Origin::First => "first", hooks::Origin::Program => "program", hooks::Origin::Outside => "outside" },
            "pid": e.pid, "parent": e.parent, "path": e.path, "argv": e.argv, "env": e.env, "cwd": e.cwd, "found": e.found,
        }),
        hooks::Event::Output(o) => json!({
            "kind": "output",
            "stream": if o.stream == hooks::Stream::Stdout { "stdout" } else { "stderr" },
            "source": match o.source { hooks::Source::Machine => Value::from("machine"), hooks::Source::Exec(id) => Value::from(id) },
            "text": o.text(),
        }),
        hooks::Event::Connect(c) => json!({
            "kind": "connect", "proto": if c.proto == hooks::Proto::Tcp { "tcp" } else { "udp" }, "to": c.to.to_string(), "name": c.name,
        }),
        hooks::Event::Lookup(l) => json!({ "kind": "lookup", "name": l.name }),
        hooks::Event::Registry(r) => json!({
            "kind": "registry", "service": r.service, "op": r.op,
            "params": r.params.iter().map(|(n, v)| (n.clone(), Value::from(v.clone()))).collect::<serde_json::Map<String, Value>>(),
        }),
    }
}

pub fn event_from_json(v: &Value) -> Option<hooks::Event> {
    let s = |key: &str| v.get(key).and_then(Value::as_str);
    let list = |key: &str| -> Vec<String> { v.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    Some(match s("kind")? {
        "exec" => hooks::Event::Exec(hooks::Exec {
            os: if s("os") == Some("windows") { hooks::Os::Windows } else { hooks::Os::Linux },
            origin: match s("origin") {
                Some("first") => hooks::Origin::First,
                Some("outside") => hooks::Origin::Outside,
                _ => hooks::Origin::Program,
            },
            pid: v.get("pid").and_then(Value::as_u64).map(|p| p as u32),
            parent: v.get("parent").and_then(Value::as_u64).map(|p| p as u32),
            path: s("path").unwrap_or_default().to_string(),
            argv: list("argv"),
            env: list("env"),
            cwd: s("cwd").map(String::from),
            found: v.get("found").and_then(Value::as_bool).unwrap_or(true),
        }),
        "output" => hooks::Event::Output(hooks::Output {
            stream: if s("stream") == Some("stderr") { hooks::Stream::Stderr } else { hooks::Stream::Stdout },
            source: v.get("source").and_then(Value::as_u64).map_or(hooks::Source::Machine, hooks::Source::Exec),
            bytes: s("text").unwrap_or_default().as_bytes().to_vec(),
        }),
        "connect" => hooks::Event::Connect(hooks::Connect {
            proto: if s("proto") == Some("udp") { hooks::Proto::Udp } else { hooks::Proto::Tcp },
            to: s("to")?.parse().ok()?,
            name: s("name").map(String::from),
        }),
        "lookup" => hooks::Event::Lookup(hooks::Lookup::new(s("name")?)),
        "registry" => hooks::Event::Registry(hooks::Registry {
            service: s("service")?.to_string(),
            op: s("op")?.to_string(),
            params: v.get("params").and_then(Value::as_object).map(|m| m.iter().filter_map(|(n, x)| Some((n.clone(), x.as_str()?.to_string()))).collect()).unwrap_or_default(),
        }),
        _ => return None,
    })
}

/// An observed event as one JSON object: the machine, when (seconds since 1970), the event, and what was decided
/// (`"verdict": "allow" | "deny" | "rewrite" | "answer"`, with `"rewrite"` or `"answer"` saying what to).
pub fn observed_json(o: &hooks::Observed) -> Value {
    let at = o.at.duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let mut v = json!({ "machine": o.machine, "at": at, "event": event_json(&o.event), "verdict": o.verdict.name() });
    match &o.verdict {
        hooks::Verdict::Rewrite(e) => v["rewrite"] = event_json(e),
        hooks::Verdict::Answer(hooks::Answer::Exec(r)) => v["answer"] = json!({ "stdout": String::from_utf8_lossy(&r.stdout), "stderr": String::from_utf8_lossy(&r.stderr), "code": r.code }),
        hooks::Verdict::Answer(hooks::Answer::Lookup(ips)) => v["answer"] = ip_list(ips),
        _ => {}
    }
    v
}

pub fn observed_from_json(v: &Value) -> Option<hooks::Observed> {
    let event = event_from_json(v.get("event")?)?;
    let verdict = match v.get("verdict")?.as_str()? {
        "allow" => hooks::Verdict::Allow,
        "deny" => hooks::Verdict::Deny,
        "rewrite" => hooks::Verdict::Rewrite(event_from_json(v.get("rewrite")?)?),
        "answer" => {
            let a = v.get("answer")?;
            if let Some(ips) = a.as_array() {
                hooks::Verdict::Answer(hooks::Answer::Lookup(ips.iter().filter_map(|ip| ip.as_str()?.parse().ok()).collect()))
            } else {
                let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().as_bytes().to_vec();
                hooks::Verdict::Answer(hooks::Answer::Exec(hooks::Reply { stdout: text("stdout"), stderr: text("stderr"), code: a.get("code").and_then(Value::as_i64).unwrap_or(0) as i32 }))
            }
        }
        _ => return None,
    };
    let at = std::time::UNIX_EPOCH + Duration::from_secs_f64(v.get("at").and_then(Value::as_f64).unwrap_or(0.0).max(0.0));
    Some(hooks::Observed { machine: v.get("machine").and_then(Value::as_str).unwrap_or_default().to_string(), event, verdict, at })
}

/// An observed event from a line [`Watch::next_line`] gave.
pub fn parse_observed(line: &str) -> Option<hooks::Observed> {
    observed_from_json(&serde_json::from_str(line).ok()?)
}

fn interest_json(interest: hooks::Interest) -> String {
    json!({ "kinds": interest.kinds().map(hooks::Kind::name).collect::<Vec<_>>() }).to_string()
}

fn interest_from_json(text: &str) -> hooks::Interest {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut interest = hooks::Interest::none();
    for kind in v.get("kinds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(hooks::Kind::parse) {
        interest |= hooks::Interest::of(kind);
    }
    if interest.is_empty() { hooks::Interest::all() } else { interest }
}

// ---------------------------------------------------------------------------------------------------------------
// Frames

#[cfg(unix)]
pub(crate) fn frame(kind: u8, data: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + data.len());
    f.push(kind);
    f.extend((data.len() as u32).to_le_bytes());
    f.extend(data);
    f
}

/// Reads one frame: its kind and bytes, or None at the end.
#[cfg(unix)]
pub(crate) fn read_frame(r: &mut impl std::io::Read) -> Option<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head).ok()?;
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut data = vec![0; len.min(16 << 20)];
    r.read_exact(&mut data).ok()?;
    Some((head[0], data))
}

/// The request's line, read a byte at a time so nothing after it (an `exec`'s frames) is taken with it.
#[cfg(unix)]
fn read_line(stream: &mut impl std::io::Read) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte)? == 1 && byte[0] != b'\n' {
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------------------------------------------------------------------------------------------------------------
// The machine's side

/// How a running machine does an operation on its files, on its kernel's disk.
pub(crate) type FilesFn = Arc<dyn Fn(&crate::files::Op, &[u8]) -> Result<Vec<u8>> + Send + Sync>;

/// What the socket serves for one machine.
#[derive(Clone)]
pub(crate) struct Served {
    pub desktop: Option<display::desktop::Shared>,
    pub enter: Option<Enter>,
    pub files: Option<FilesFn>,
    pub stopper: Stopper,
    pub hub: hooks::Hub,
}

/// An `fs` connection, after its line: a write's bytes in, then the answer.
#[cfg(unix)]
fn run_fs(stream: &mut std::os::unix::net::UnixStream, json: &str, files: &FilesFn) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let Some(op) = serde_json::from_str::<Value>(json).ok().as_ref().and_then(crate::files::Op::from_json) else {
        return stream.write_all(b"error fs's request made no sense\n");
    };
    let mut data = Vec::new();
    if let crate::files::Op::Write { len, .. } = &op {
        data.resize(*len as usize, 0);
        stream.read_exact(&mut data)?;
    }
    match files(&op, &data) {
        Ok(answer) => {
            stream.write_all(format!("ok {}\n", answer.len()).as_bytes())?;
            stream.write_all(&answer)
        }
        Err(e) => stream.write_all(format!("error {}\n", escape(e.message())).as_bytes()),
    }
}

/// An `exec` connection, after its request: the session's output out, the client's input in, until it ends.
#[cfg(unix)]
fn run_exec(stream: &mut std::os::unix::net::UnixStream, request: ExecRequest, enter: &Enter) -> std::io::Result<()> {
    use std::io::Write;
    let detach = request.detach;
    let session: Arc<dyn Session> = match enter(request) {
        Ok(session) => Arc::from(session),
        Err(why) => return stream.write_all(format!("error {}\n", why.replace('\n', " ")).as_bytes()),
    };
    stream.write_all(b"ok 0\n")?;
    if detach {
        // Left running: its output is drained, so it never waits on a full pipe, until it ends.
        session.close_input();
        std::thread::spawn(move || while !matches!(session.next(Duration::from_secs(3600)), Some(Output::Exit(_))) {});
        return Ok(());
    }
    let mut reader = stream.try_clone()?;
    let input = session.clone();
    std::thread::spawn(move || {
        while let Some((kind, data)) = read_frame(&mut reader) {
            match kind {
                0 => input.input(&data),
                1 => input.close_input(),
                2 if data.len() >= 4 => input.resize(u16::from_le_bytes([data[0], data[1]]), u16::from_le_bytes([data[2], data[3]])),
                3 if data.len() >= 4 => input.signal(i32::from_le_bytes([data[0], data[1], data[2], data[3]])),
                _ => {}
            }
        }
    });
    loop {
        match session.next(Duration::from_millis(250)) {
            Some(Output::Out(data)) if !data.is_empty() => stream.write_all(&frame(1, &data))?,
            Some(Output::Err(data)) if !data.is_empty() => stream.write_all(&frame(2, &data))?,
            Some(Output::Exit(code)) => return stream.write_all(&frame(3, &code.to_le_bytes())),
            _ => {}
        }
    }
}

/// A `watch` connection: each event the machine's hooks decide, as a JSON line, until the client goes or the
/// machine ends.
#[cfg(unix)]
fn run_watch(stream: &mut std::os::unix::net::UnixStream, interest: hooks::Interest, served: &Served) -> std::io::Result<()> {
    use std::io::Write;
    let observer = served.hub.observe(interest, 4096);
    stream.write_all(b"ok 0\n")?;
    let mut lost = 0;
    while !served.stopper.is_done() {
        if let Ok(observed) = observer.next_timeout(Duration::from_millis(250)) {
            let mut line = observed_json(&observed);
            if observer.lost() != lost {
                lost = observer.lost();
                line["lost"] = Value::from(lost);
            }
            stream.write_all(format!("{line}\n").as_bytes())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn answer(stream: &mut std::os::unix::net::UnixStream, served: &Served) -> std::io::Result<()> {
    use std::io::Write;
    // Only the line's end goes: text typed, copied or dropped keeps its own spaces at either end.
    let line = read_line(stream)?;
    if let Some(json) = line.strip_prefix("exec ") {
        return match (request_from_json(json), &served.enter) {
            (Some(request), Some(enter)) => run_exec(stream, request, enter),
            (None, _) => stream.write_all(b"error exec's request made no sense\n"),
            (_, None) => stream.write_all(b"error this machine can't start a process from outside yet\n"),
        };
    }
    if let Some(json) = line.strip_prefix("watch") {
        return run_watch(stream, interest_from_json(json.trim()), served);
    }
    if let Some(json) = line.strip_prefix("fs ") {
        return match &served.files {
            Some(files) => run_fs(stream, json, files),
            None => stream.write_all(b"error this machine's files can't be reached from outside yet\n"),
        };
    }
    let line = line.strip_suffix('\r').unwrap_or(&line);
    if line == "kill" {
        served.stopper.kill();
        return stream.write_all(b"ok 0\n");
    }
    if line == "stop" || line.starts_with("stop ") {
        let signal = line["stop".len()..].trim().parse().unwrap_or(15);
        served.stopper.stop(signal);
        return stream.write_all(b"ok 0\n");
    }
    let desktop = &served.desktop;
    match (line, desktop) {
        ("screenshot", Some(desktop)) => {
            let png = display::desktop::lock(desktop).screen().png();
            stream.write_all(format!("ok {}\n", png.len()).as_bytes())?;
            stream.write_all(&png)
        }
        ("windows", Some(desktop)) => {
            let text = windows(&display::desktop::lock(desktop));
            stream.write_all(format!("ok {}\n", text.len()).as_bytes())?;
            stream.write_all(text.as_bytes())
        }
        ("windows-json", Some(desktop)) => {
            let text = windows_json(&display::desktop::lock(desktop)).to_string();
            stream.write_all(format!("ok {}\n", text.len()).as_bytes())?;
            stream.write_all(text.as_bytes())
        }
        ("clipboard", Some(desktop)) => {
            let text = display::desktop::paste(desktop, display::selection::Selection::Clipboard, display::selection::TEXT, Duration::from_secs(2));
            match text {
                Some(bytes) => {
                    stream.write_all(format!("ok {}\n", bytes.len()).as_bytes())?;
                    stream.write_all(&bytes)
                }
                None => stream.write_all(b"error the clipboard has no text\n"),
            }
        }
        (line, Some(desktop)) if line.starts_with("clipboard ") => {
            // From outside the machine: an offer with no window, its text given at once. Whoever had it hears so.
            let text = unescape(&line["clipboard ".len()..]);
            let waker = {
                let mut d = display::desktop::lock(desktop);
                let source = d.offer(display::selection::Selection::Clipboard, None, Vec::new());
                d.offer_data(source, display::selection::TEXT, text.into_bytes());
                if d.has_events() { d.waker() } else { None }
            };
            if let Some(wake) = waker {
                wake();
            }
            stream.write_all(b"ok 0\n")
        }
        (line, Some(desktop)) if line.starts_with("drop ") => {
            // From outside the machine, as the host's drag brings it in: `drop X Y text TEXT` or `drop X Y files PATHS`
            // (one a line). What the window there did with it comes back.
            let mut parts = line["drop ".len()..].splitn(4, ' ');
            let (x, y, kind, body) = (parts.next(), parts.next(), parts.next(), parts.next().map(unescape).unwrap_or_default());
            let (Some(Ok(x)), Some(Ok(y))) = (x.map(str::parse::<i32>), y.map(str::parse::<i32>)) else {
                return stream.write_all(b"error a drop needs a point: X Y\n");
            };
            use display::desktop::action;
            use display::selection::{TEXT, URI_LIST, uri_list};
            let data = match kind {
                Some("text") => vec![(TEXT.to_string(), body.into_bytes())],
                Some("files") => vec![(URI_LIST.to_string(), uri_list(&body.lines().collect::<Vec<_>>()))],
                _ => return stream.write_all(b"error a drop is of text or files\n"),
            };
            match display::desktop::drop_from_outside(desktop, x, y, data, action::COPY, Duration::from_secs(2)) {
                Ok(done) => {
                    let what = match done {
                        action::COPY => "copied",
                        action::MOVE => "moved",
                        action::LINK => "linked",
                        action::ASK => "asked",
                        _ => "nothing there took it",
                    };
                    stream.write_all(format!("ok {}\n{what}", what.len()).as_bytes())
                }
                Err(e) => stream.write_all(format!("error {e}\n").as_bytes()),
            }
        }
        (line, Some(desktop)) if line.starts_with("resize ") => match display::output::parse_size(&line["resize ".len()..]) {
            Some((width, height, scale)) => {
                let waker = {
                    let mut d = display::desktop::lock(desktop);
                    let output = display::output::Output { width, height, scale, ..d.output().clone() };
                    d.set_output(output);
                    d.waker()
                };
                // Its kernel reads what that made, without the desktop's lock: configured windows, new scales.
                if let Some(wake) = waker {
                    wake();
                }
                stream.write_all(b"ok 0\n")
            }
            None => stream.write_all(b"error resize takes WIDTHxHEIGHT or WIDTHxHEIGHT@SCALE\n"),
        },
        (line, Some(desktop)) if INPUT.iter().any(|v| line.split(' ').next() == Some(*v)) => match input(desktop, line) {
            Ok(()) => stream.write_all(b"ok 0\n"),
            Err(why) => stream.write_all(format!("error {why}\n").as_bytes()),
        },
        (line, None) if line.starts_with("screenshot") || line.starts_with("windows") || line.starts_with("clipboard") || line.starts_with("drop ") || line.starts_with("resize ") || INPUT.iter().any(|v| line.split(' ').next() == Some(*v)) => {
            stream.write_all(b"error this machine has no desktop (run it with a display)\n")
        }
        (other, _) => stream.write_all(format!("error {other:?} isn't something a machine is asked\n").as_bytes()),
    }
}

/// The requests that are input.
const INPUT: [&str; 6] = ["move", "click", "drag", "scroll", "type", "key"];

/// Input for the desktop, as a user's would come: each event through `display::desktop::send`, which wakes the
/// kernel. Checked whole before any of it's sent, so a bad request does nothing.
#[cfg_attr(not(unix), allow(dead_code))]
fn input(desktop: &display::desktop::Shared, line: &str) -> std::result::Result<(), String> {
    use display::desktop::{Button, Input, send};
    use display::keys;
    let mut words = line.splitn(2, ' ');
    let verb = words.next().unwrap_or_default();
    let rest = words.next().unwrap_or_default();
    let numbers = |want: std::ops::RangeInclusive<usize>| -> std::result::Result<Vec<i32>, String> {
        let n: Vec<i32> = rest.split_whitespace().map_while(|w| w.parse().ok()).collect();
        if want.contains(&n.len()) { Ok(n) } else { Err(format!("{verb} takes {}", usage(verb))) }
    };
    let mut events = Vec::new();
    match verb {
        "move" => {
            let n = numbers(2..=2)?;
            events.push(Input::PointerTo(n[0], n[1]));
        }
        "click" => {
            let n = numbers(2..=2)?;
            let words: Vec<&str> = rest.split_whitespace().skip(2).collect();
            let button = match words.first().copied() {
                None | Some("left") => Button::Left,
                Some("right") => Button::Right,
                Some("middle") => Button::Middle,
                Some(other) => return Err(format!("{other} isn't a button: left, right, middle")),
            };
            let count = match words.get(1) {
                Some(c) => c.parse::<u32>().ok().filter(|c| (1..=3).contains(c)).ok_or("a click's count is 1, 2 or 3")?,
                None => 1,
            };
            events.push(Input::PointerTo(n[0], n[1]));
            for _ in 0..count {
                events.push(Input::Button(button, true));
                events.push(Input::Button(button, false));
            }
        }
        "drag" => {
            let n = numbers(4..=4)?;
            events.push(Input::PointerTo(n[0], n[1]));
            events.push(Input::Button(Button::Left, true));
            // On the way, so it moves as a hand would.
            for step in 1..=8 {
                events.push(Input::PointerTo(n[0] + (n[2] - n[0]) * step / 8, n[1] + (n[3] - n[1]) * step / 8));
            }
            events.push(Input::Button(Button::Left, false));
        }
        "scroll" => {
            let n = numbers(3..=4)?;
            events.push(Input::PointerTo(n[0], n[1]));
            events.push(Input::Scroll { x: n.get(3).copied().unwrap_or(0) * 120, y: n[2] * 120 });
        }
        "type" => {
            // Keys for what the keyboard types (US English), as a user's; anything else as text, as an input method
            // commits it.
            let text = unescape(rest);
            let mut other = String::new();
            for c in text.chars() {
                let Some((code, shift)) = keys::for_char(c) else {
                    other.push(c);
                    continue;
                };
                if !other.is_empty() {
                    events.push(Input::Text(std::mem::take(&mut other)));
                }
                if shift {
                    events.push(Input::Key(keys::LEFT_SHIFT, true));
                }
                events.push(Input::Key(code, true));
                events.push(Input::Key(code, false));
                if shift {
                    events.push(Input::Key(keys::LEFT_SHIFT, false));
                }
            }
            if !other.is_empty() {
                events.push(Input::Text(other));
            }
        }
        "key" => {
            if rest.trim().is_empty() {
                return Err(format!("key takes {}", usage("key")));
            }
            for combo in rest.split_whitespace() {
                let codes: Vec<u16> = combo.split('+').map(|k| keys::by_name(k).ok_or_else(|| format!("{k:?} isn't a key's name"))).collect::<std::result::Result<_, _>>()?;
                events.extend(codes.iter().map(|&c| Input::Key(c, true)));
                events.extend(codes.iter().rev().map(|&c| Input::Key(c, false)));
            }
        }
        _ => return Err(format!("{verb:?} isn't input")),
    }
    for e in events {
        send(desktop, e);
    }
    Ok(())
}

/// How each input request is written.
fn usage(verb: &str) -> &'static str {
    match verb {
        "move" => "X Y",
        "click" => "X Y [left|right|middle] [COUNT]",
        "drag" => "FROM_X FROM_Y TO_X TO_Y",
        "scroll" => "X Y NOTCHES [SIDEWAYS] (up and right positive)",
        "type" => "TEXT",
        _ => "a key or keys held together, as ctrl+s or alt+f4",
    }
}

/// A request's text, with `%0A` for a new line and `%25` for `%` (a request is one line).
fn unescape(text: &str) -> String {
    text.replace("%0A", "\n").replace("%25", "%")
}

/// Text made fit for a one-line request: what `unescape` undoes.
pub(crate) fn escape(text: &str) -> String {
    text.replace('%', "%25").replace('\n', "%0A")
}

/// A window's states, as words: active, minimized or hidden, maximized, fullscreen, attention, popup or dialog of.
#[cfg_attr(not(unix), allow(dead_code))]
fn window_states(d: &display::Desktop, w: &display::observe::WindowInfo) -> Vec<String> {
    use display::desktop::state;
    let mut what = Vec::new();
    if d.active() == Some(w.id) {
        what.push("active".to_string());
    }
    if !w.on_screen {
        what.push(if w.states & state::MINIMIZED != 0 { "minimized" } else { "hidden" }.to_string());
    }
    for (bit, name) in [(state::MAXIMIZED, "maximized"), (state::FULLSCREEN, "fullscreen")] {
        if w.states & bit != 0 {
            what.push(name.to_string());
        }
    }
    if w.attention {
        what.push("attention".to_string());
    }
    match (w.popup, w.parent) {
        (true, Some(p)) => what.push(format!("popup of {p}")),
        (false, Some(p)) => what.push(format!("dialog of {p}")),
        _ => {}
    }
    what
}

/// The desktop's windows, top first, as `carbon display windows` shows them: for an agent to find what it's to click.
#[cfg_attr(not(unix), allow(dead_code))]
fn windows(d: &display::Desktop) -> String {
    let mut out = format!("{:<8}{:<12}{:<12}{:<28}TITLE\n", "WINDOW", "AT", "SIZE", "STATE");
    for w in d.windows().iter().rev() {
        let what = window_states(d, w);
        let state = if what.is_empty() { "-".to_string() } else { what.join(",") };
        let (at, size) = (format!("{},{}", w.rect.left, w.rect.top), format!("{}x{}", w.rect.width(), w.rect.height()));
        out.push_str(&format!("{:<8}{:<12}{:<12}{:<28}{}\n", w.id, at, size, state, w.title));
    }
    out
}

/// The same, as JSON, for programs.
#[cfg_attr(not(unix), allow(dead_code))]
fn windows_json(d: &display::Desktop) -> Value {
    let list: Vec<Value> = d
        .windows()
        .iter()
        .rev()
        .map(|w| {
            json!({
                "id": w.id.to_string().parse::<u64>().unwrap_or(0),
                "x": w.rect.left, "y": w.rect.top, "width": w.rect.width(), "height": w.rect.height(),
                "states": window_states(d, w), "title": w.title,
            })
        })
        .collect();
    Value::from(list)
}

/// The socket, removed when the machine ends, and the thread that answers on it told to end.
pub(crate) struct Control {
    #[cfg_attr(not(unix), allow(dead_code))]
    paths: Vec<PathBuf>,
    #[cfg_attr(not(unix), allow(dead_code))]
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Control {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        // A connection of its own wakes the thread waiting to accept, which sees it's closed.
        #[cfg(unix)]
        if let Some(socket) = self.paths.first() {
            let _ = std::os::unix::net::UnixStream::connect(socket);
        }
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Starts answering on a socket for the machine whose folder is `dir`.
#[cfg(unix)]
pub(crate) fn serve(dir: PathBuf, served: Served) -> Option<Control> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    let socket = std::env::temp_dir().join(format!("carbon-{:016x}.sock", hasher.finish()));
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket).ok()?;
    let named = dir.join("control");
    std::fs::write(&named, socket.to_string_lossy().as_bytes()).ok()?;
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = closed.clone();
    let _ = std::thread::Builder::new().name("carbon control".into()).spawn(move || {
        // Each on a thread of its own: an `exec` lasts as long as its process.
        for mut stream in listener.incoming().flatten() {
            if seen.load(Ordering::Acquire) || served.stopper.is_done() {
                return;
            }
            let served = served.clone();
            let _ = std::thread::Builder::new().name("carbon control".into()).spawn(move || answer(&mut stream, &served));
        }
    });
    Some(Control { paths: vec![socket, named], closed })
}

#[cfg(not(unix))]
pub(crate) fn serve(_dir: PathBuf, _served: Served) -> Option<Control> {
    None
}

// ---------------------------------------------------------------------------------------------------------------
// The client's side

/// Connects to the running machine whose folder is `dir`, sends `request`, and reads the answer's first line: the
/// connection, for what comes after it.
#[cfg(unix)]
pub(crate) fn open(dir: &Path, request: &str) -> Result<(std::os::unix::net::UnixStream, String)> {
    use std::io::Write;
    let socket = std::fs::read_to_string(dir.join("control")).map_err(|_| Error::new(crate::ErrorKind::NotRunning, "the machine isn't running"))?;
    let mut stream = std::os::unix::net::UnixStream::connect(socket.trim()).map_err(|e| Error::io(format!("reaching the machine: {e}")))?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let line = read_line(&mut stream)?;
    if let Some(why) = line.strip_prefix("error ") {
        return Err(error_from(why));
    }
    if !line.starts_with("ok") {
        return Err(Error::io("the machine's answer made no sense"));
    }
    Ok((stream, line))
}

/// What the machine said went wrong, with a kind where the words say which.
fn error_from(why: &str) -> Error {
    let kind = if why.contains("hooks") || why.contains("denied") {
        crate::ErrorKind::Denied
    } else if why.contains("is there already") {
        crate::ErrorKind::AlreadyExists
    } else if why.contains("not found") || why.contains("not on the machine") || why.contains("isn't there") || why.contains("no such") {
        crate::ErrorKind::NotFound
    } else if why.contains("no desktop") || why.contains("can't") {
        crate::ErrorKind::Unsupported
    } else {
        crate::ErrorKind::InvalidInput
    };
    Error::new(kind, why)
}

#[cfg(not(unix))]
pub(crate) fn open(_dir: &Path, _request: &str) -> Result<((), String)> {
    Err(Error::unsupported("reaching a running machine from outside it isn't there yet on Windows hosts"))
}

/// Does an operation on the files of the running machine whose folder is `dir`: `data` is a write's bytes.
#[cfg(unix)]
pub(crate) fn fs(dir: &Path, op: &Value, data: &[u8]) -> Result<Vec<u8>> {
    use std::io::{Read, Write};
    let socket = std::fs::read_to_string(dir.join("control")).map_err(|_| Error::new(crate::ErrorKind::NotRunning, "the machine isn't running"))?;
    let mut stream = std::os::unix::net::UnixStream::connect(socket.trim()).map_err(|e| Error::io(format!("reaching the machine: {e}")))?;
    stream.write_all(format!("fs {op}\n").as_bytes())?;
    stream.write_all(data)?;
    let line = read_line(&mut stream)?;
    if let Some(why) = line.strip_prefix("error ") {
        return Err(error_from(&unescape(why)));
    }
    let len: usize = line.strip_prefix("ok ").and_then(|n| n.trim().parse().ok()).ok_or_else(|| Error::io("the machine's answer made no sense"))?;
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(unix))]
pub(crate) fn fs(_dir: &Path, _op: &Value, _data: &[u8]) -> Result<Vec<u8>> {
    Err(Error::unsupported("reaching a running machine from outside it isn't there yet on Windows hosts"))
}

/// Asks the running machine whose folder is `dir`, and gives its answer's bytes.
#[cfg(unix)]
pub(crate) fn ask(dir: &Path, request: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let (mut stream, line) = open(dir, request)?;
    let len: usize = line.strip_prefix("ok ").and_then(|n| n.trim().parse().ok()).ok_or_else(|| Error::io("the machine's answer made no sense"))?;
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(unix))]
pub(crate) fn ask(_dir: &Path, _request: &str) -> Result<Vec<u8>> {
    Err(Error::unsupported("reaching a running machine from outside it isn't there yet on Windows hosts"))
}

/// A watch on a running machine: its hooked events as they're decided, read from its socket.
pub struct Watch {
    #[cfg(unix)]
    lines: std::io::BufReader<std::os::unix::net::UnixStream>,
    lost: u64,
}

impl Watch {
    #[cfg(unix)]
    pub(crate) fn open(dir: &Path, interest: hooks::Interest) -> Result<Watch> {
        let (stream, _) = open(dir, &format!("watch {}", interest_json(interest)))?;
        Ok(Watch { lines: std::io::BufReader::new(stream), lost: 0 })
    }

    #[cfg(not(unix))]
    pub(crate) fn open(_dir: &Path, _interest: hooks::Interest) -> Result<Watch> {
        Err(Error::unsupported("watching a running machine isn't there yet on Windows hosts"))
    }

    /// The next event as its JSON line, waiting for it; None once the machine ends.
    pub fn next_line(&mut self) -> Option<String> {
        #[cfg(unix)]
        {
            use std::io::BufRead;
            let mut line = String::new();
            match self.lines.read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => {
                    if let Some(lost) = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("lost").and_then(Value::as_u64)) {
                        self.lost = lost;
                    }
                    Some(line.trim_end().to_string())
                }
            }
        }
        #[cfg(not(unix))]
        None
    }

    /// How many events the machine dropped because this fell behind, so far.
    pub fn lost(&self) -> u64 {
        self.lost
    }
}

impl Iterator for Watch {
    type Item = hooks::Observed;
    fn next(&mut self) -> Option<hooks::Observed> {
        loop {
            let line = self.next_line()?;
            if let Some(observed) = parse_observed(&line) {
                return Some(observed);
            }
        }
    }
}

/// `carbon exec`'s side, attached to this process: asks the running machine whose folder is `dir` to start
/// `request`, then relays this process's stdin (if `interactive`), terminal size and signals to it and its output
/// back, until it ends; its exit code. With a terminal (`request.tty`), this one's is put in raw mode for as long.
#[cfg(unix)]
pub(crate) fn exec_attached(dir: &Path, request: ExecRequest, interactive: bool) -> Result<i32> {
    use std::io::{Read, Write};
    use std::sync::Mutex;
    let mut request = request;
    // A terminal's starting size: this one's.
    if request.tty.is_some() {
        request.tty = Some(host::Host::terminal_size(&host::Std::new()).unwrap_or((24, 80)));
    }
    let (mut stream, _) = open(dir, &format!("exec {}", request_json(&request)))?;
    if request.detach {
        return Ok(0);
    }
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let send = {
        let writer = writer.clone();
        move |kind: u8, data: &[u8]| writer.lock().map(|mut w| w.write_all(&frame(kind, data)).is_ok()).unwrap_or(false)
    };

    // This terminal, raw, for as long as it's the process's.
    struct Raw(Option<host::Std>);
    impl Drop for Raw {
        fn drop(&mut self) {
            if let Some(host) = &self.0 {
                host::Host::set_raw(host, false);
            }
        }
    }
    let raw = Raw(request.tty.map(|_| {
        let host = host::Std::new();
        host::Host::set_raw(&host, true);
        host
    }));

    if interactive {
        let send = send.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buffer = [0u8; 4096];
            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if !send(0, &buffer[..n]) {
                            return;
                        }
                    }
                }
            }
            send(1, &[]);
        });
    } else {
        send(1, &[]);
    }
    if request.tty.is_some() {
        // Its size, as this terminal's changes.
        let send = send.clone();
        std::thread::spawn(move || {
            let host = host::Std::new();
            let mut last = None;
            loop {
                let size = host::Host::terminal_size(&host);
                if size != last {
                    if let Some((rows, cols)) = size {
                        let mut d = rows.to_le_bytes().to_vec();
                        d.extend(cols.to_le_bytes());
                        if !send(2, &d) {
                            return;
                        }
                    }
                    last = size;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    } else {
        // Signals to this process go to it, as Docker's --sig-proxy does; with a terminal, ^C is a key it reads.
        let send = send.clone();
        // SAFETY: a set made empty and given three signals, blocked in this thread (and those made after it) and
        // waited for on one of its own.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                libc::sigaddset(&mut set, signal);
            }
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            let set = set;
            std::thread::spawn(move || loop {
                let mut signal = 0;
                if libc::sigwait(&set, &mut signal) != 0 || !send(3, &signal.to_le_bytes()) {
                    return;
                }
            });
        }
    }

    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let code = loop {
        match read_frame(&mut stream) {
            Some((1, data)) => {
                let _ = out.write_all(&data).and_then(|()| out.flush());
            }
            Some((2, data)) => {
                let _ = err.write_all(&data);
            }
            Some((3, data)) if data.len() >= 4 => break i32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            Some(_) => {}
            None => break 255,
        }
    };
    drop(raw);
    Ok(code)
}

#[cfg(not(unix))]
pub(crate) fn exec_attached(_dir: &Path, _request: ExecRequest, _interactive: bool) -> Result<i32> {
    Err(Error::unsupported("exec into a running machine isn't there yet on Windows hosts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_as_json() {
        let events = [
            hooks::Event::Exec(hooks::Exec::new("/usr/bin/git", ["git", "push"]).with_var("HOME", "/root")),
            hooks::Event::Output(hooks::Output { stream: hooks::Stream::Stderr, source: hooks::Source::Exec(4), bytes: b"oops\n".to_vec() }),
            hooks::Event::Connect(hooks::Connect { proto: hooks::Proto::Udp, to: "[::1]:53".parse().unwrap(), name: None }),
            hooks::Event::Lookup(hooks::Lookup::new("pypi.org")),
        ];
        for event in events {
            assert_eq!(event_from_json(&event_json(&event)).as_ref(), Some(&event));
        }
        let observed = hooks::Observed {
            machine: "dev".into(),
            event: hooks::Event::Lookup(hooks::Lookup::new("api.internal")),
            verdict: hooks::Verdict::Answer(hooks::Answer::Lookup(vec!["10.89.0.1".parse().unwrap()])),
            at: std::time::UNIX_EPOCH + Duration::from_secs(5),
        };
        assert_eq!(observed_from_json(&observed_json(&observed)), Some(observed));
        assert_eq!(interest_from_json(&interest_json(hooks::Interest::network())), hooks::Interest::network());
    }
}
