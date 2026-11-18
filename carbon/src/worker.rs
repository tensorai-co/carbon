//! Workers: a machine in a process of its own ([`crate::MachineSpec::launch`]), so it outlives the program that
//! launched it, a crash or a hang in it takes nothing else down, and each has a VM of its own (on macOS a process has
//! one).
//!
//! A worker is the launching program's own binary, run again with `CARBON_WORKER` set, which [`crate::init`] sees
//! first: it reads what to run from its stdin (JSON, [`spec_json`]), runs the machine on its main thread as
//! [`crate::MachineSpec::run`] would, keeps what it writes in its log ([`crate::logs`]), and tells the launcher how it
//! went on a pipe at descriptor 3, a line each: `notice KIND TEXT` (a pull's progress, a fallback), then `ok NAME`
//! once its kernel is up, or `error KIND MESSAGE`. It's a session of its own, so a terminal's Ctrl-C to the launcher
//! doesn't reach it; SIGTERM asks it to stop, as `carbon stop` does.
//!
//! Its rules are decided in the worker. Hooks with code stay in the launcher, and the worker asks about each event
//! they want over a **hook link**: a Unix socket the launcher listens on, which the worker connects to before its
//! kernel boots, so nothing is missed. A line each way: `{"id": N, "event": {...}}`, then `{"id": N, "verdict":
//! "pass"|"allow"|"deny"|"rewrite"|"answer", "event"|"answer": ...}`. With the launcher gone, an event the link wanted
//! is refused (its output passes), so a policy in code never quietly stops holding.

use std::path::{Path, PathBuf};
use std::time::Duration;

use exec::Isolation;
use hooks::{Decision, Event, Hooked, Hooks, Hub, Interest, Kind};
use serde_json::{Value, json};

use crate::boot::Launch;
use crate::error::{Error, ErrorKind, Result};
use crate::machine::Machine;
use crate::store::{Source, network_name, parse_network};
use crate::wire::{event_from_json, event_json};
use crate::{Audio, Carbon, Display, Gpu, Rules};

const WORKER_VAR: &str = "CARBON_WORKER";
/// Where a worker writes how it went.
const REPORT_FD: i32 = 3;
/// How long a hook link waits for the launcher's decision before the event is refused.
const DECISION_WAIT: Duration = Duration::from_secs(120);

fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::NotFound => "not-found",
        ErrorKind::AlreadyExists => "exists",
        ErrorKind::Running => "running",
        ErrorKind::NotRunning => "not-running",
        ErrorKind::InvalidInput => "invalid",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Denied => "denied",
        ErrorKind::Image => "image",
        ErrorKind::Io => "io",
        _ => "failed",
    }
}

fn kind_of(name: &str) -> ErrorKind {
    match name {
        "not-found" => ErrorKind::NotFound,
        "exists" => ErrorKind::AlreadyExists,
        "running" => ErrorKind::Running,
        "not-running" => ErrorKind::NotRunning,
        "invalid" => ErrorKind::InvalidInput,
        "unsupported" => ErrorKind::Unsupported,
        "denied" => ErrorKind::Denied,
        "image" => ErrorKind::Image,
        "io" => ErrorKind::Io,
        _ => ErrorKind::Failed,
    }
}

fn kinds(interest: Interest) -> Vec<&'static str> {
    [(Kind::Exec, "exec"), (Kind::Output, "output"), (Kind::Connect, "connect"), (Kind::Lookup, "lookup")]
        .into_iter()
        .filter(|(k, _)| interest.has(*k))
        .map(|(_, name)| name)
        .collect()
}

fn interest_of(names: &[String]) -> Interest {
    names.iter().fold(Interest::none(), |i, n| {
        i | match n.as_str() {
            "exec" => Interest::exec(),
            "output" => Interest::output(),
            "connect" => Interest::connect(),
            "lookup" => Interest::lookup(),
            _ => Interest::none(),
        }
    })
}

/// What a hub wants asked: the kinds it would make events for.
fn hub_interest(hub: &Hub) -> Interest {
    Interest::all().kinds().filter(|k| hub.wants(*k)).fold(Interest::none(), |all, k| all | Interest::of(k))
}

// ---------------------------------------------------------------------------------------------------------------
// What a worker is told to run

/// Everything a worker needs, as JSON.
fn spec_json(home: &Path, launch: &Launch, rules: &Rules, link: Option<(&Path, Interest)>) -> Value {
    let display = match launch.display {
        Display::Headless(w, h, scale) => json!({ "headless": [w, h, scale] }),
        Display::Host(w, h) => json!({ "host": [w, h] }),
        Display::HostWindows => json!("host-windows"),
        Display::None => json!("none"),
    };
    let audio = match &launch.audio {
        Audio::Auto => json!("auto"),
        Audio::Host => json!("host"),
        Audio::None => json!("none"),
        Audio::Wav(path) => json!({ "wav": path }),
    };
    json!({
        "home": home,
        "name": launch.name,
        "hostname": launch.hostname,
        "source": match &launch.source {
            Some(Source::Image(image)) => json!({ "image": image }),
            Some(Source::Program(path)) => json!({ "program": path }),
            None => Value::Null,
        },
        "args": launch.args,
        "isolation": match launch.isolation { Isolation::Vm => "vm", Isolation::Sandbox => "sandbox", _ => "auto" },
        "platform": launch.platform.as_ref().map(ToString::to_string),
        "env": launch.env,
        "workdir": launch.workdir,
        "display": display,
        "gpu": match launch.gpu { Gpu::Host => "host", Gpu::Software => "software", Gpu::None => "none", _ => "auto" },
        "audio": audio,
        "memory": launch.memory,
        "network": launch.network.map(network_name),
        "publish": launch.publish,
        "publish_all": launch.publish_all,
        "remove": launch.remove,
        "timeout_ms": launch.timeout.map(|t| t.as_millis() as u64),
        "mounts": launch.mounts.as_deref().map(crate::mounts::mounts_json),
        "user": launch.user,
        "rules": rules.to_json(),
        "link": link.map(|(path, interest)| json!({ "path": path, "kinds": kinds(interest) })),
    })
}

/// What a worker reads back: its home, the launch (its hub empty), its rules, and its hook link's socket and kinds.
#[allow(clippy::type_complexity)]
fn spec_from_json(v: &Value) -> Result<(PathBuf, Launch, Rules, Option<(PathBuf, Interest)>)> {
    let bad = |what: &str| Error::invalid(format!("a worker's spec has no good {what}"));
    let s = |key: &str| v.get(key).and_then(Value::as_str).map(String::from);
    let list = |key: &str| v.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(String::from).collect::<Vec<_>>());
    let home = PathBuf::from(s("home").ok_or_else(|| bad("home"))?);
    let source = match v.get("source") {
        Some(Value::Object(o)) if o.contains_key("image") => Some(Source::Image(o["image"].as_str().ok_or_else(|| bad("image"))?.to_string())),
        Some(Value::Object(o)) if o.contains_key("program") => Some(Source::Program(PathBuf::from(o["program"].as_str().ok_or_else(|| bad("program"))?))),
        _ => None,
    };
    let pair = |v: &Value| -> Option<Vec<i64>> { v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()) };
    let display = match v.get("display") {
        Some(Value::String(t)) if t == "none" => Display::None,
        Some(Value::String(t)) if t == "host-windows" => Display::HostWindows,
        Some(Value::Object(o)) if o.contains_key("host") => match pair(&o["host"]).as_deref() {
            Some([w, h]) => Display::Host(*w as i32, *h as i32),
            _ => return Err(bad("display")),
        },
        Some(Value::Object(o)) if o.contains_key("headless") => match pair(&o["headless"]).as_deref() {
            Some([w, h, scale]) => Display::Headless(*w as i32, *h as i32, *scale as u32),
            _ => return Err(bad("display")),
        },
        _ => Display::default(),
    };
    let audio = match v.get("audio") {
        Some(Value::String(t)) if t == "host" => Audio::Host,
        Some(Value::String(t)) if t == "none" => Audio::None,
        Some(Value::Object(o)) if o.contains_key("wav") => Audio::Wav(PathBuf::from(o["wav"].as_str().ok_or_else(|| bad("audio"))?)),
        _ => Audio::Auto,
    };
    let launch = Launch {
        name: s("name"),
        hostname: s("hostname"),
        source,
        args: list("args").unwrap_or_default(),
        isolation: match s("isolation").as_deref() {
            Some("vm") => Isolation::Vm,
            Some("sandbox") => Isolation::Sandbox,
            _ => Isolation::Auto,
        },
        platform: s("platform").and_then(|p| images::Platform::parse(&p)),
        env: list("env").unwrap_or_default(),
        workdir: s("workdir"),
        display,
        gpu: match s("gpu").as_deref() {
            Some("host") => Gpu::Host,
            Some("software") => Gpu::Software,
            Some("none") => Gpu::None,
            _ => Gpu::Auto,
        },
        audio,
        memory: v.get("memory").and_then(Value::as_u64).ok_or_else(|| bad("memory"))?,
        network: s("network").as_deref().and_then(parse_network),
        publish: list("publish"),
        publish_all: v.get("publish_all").and_then(Value::as_bool).unwrap_or(false),
        stdin: crate::Stdio::Null,
        stdout: crate::Stdio::Piped,
        stderr: crate::Stdio::Piped,
        tty: false,
        // SIGTERM to the worker stops its machine, as `carbon stop` does.
        relay_signals: true,
        remove: v.get("remove").and_then(Value::as_bool).unwrap_or(false),
        timeout: v.get("timeout_ms").and_then(Value::as_u64).map(Duration::from_millis),
        mounts: v.get("mounts").filter(|m| !m.is_null()).map(|m| crate::mounts::mounts_from_json(Some(m))),
        user: s("user"),
        hub: Hub::new(),
    };
    let rules = Rules::from_json(v.get("rules").unwrap_or(&Value::Null))?;
    let link = v.get("link").filter(|l| !l.is_null()).map(|l| {
        let path = PathBuf::from(l.get("path").and_then(Value::as_str).unwrap_or_default());
        let names: Vec<String> = l.get("kinds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
        (path, interest_of(&names))
    });
    Ok((home, launch, rules, link))
}

// ---------------------------------------------------------------------------------------------------------------
// Decisions over the hook link

fn decision_json<E: Hooked>(decision: Decision<E>) -> Value {
    match decision {
        Decision::Pass => json!({ "verdict": "pass" }),
        Decision::Allow => json!({ "verdict": "allow" }),
        Decision::Deny => json!({ "verdict": "deny" }),
        Decision::Rewrite(e) => json!({ "verdict": "rewrite", "event": event_json(&e.into_event()) }),
        Decision::Answer(a) => match E::answer_of(a) {
            Some(hooks::Answer::Exec(r)) => json!({ "verdict": "answer", "answer": { "stdout": String::from_utf8_lossy(&r.stdout), "stderr": String::from_utf8_lossy(&r.stderr), "code": r.code } }),
            Some(hooks::Answer::Lookup(ips)) => json!({ "verdict": "answer", "answer": ips.iter().map(ToString::to_string).collect::<Vec<_>>() }),
            None => json!({ "verdict": "pass" }),
        },
    }
}

fn decision_from_json<E: Hooked>(v: &Value) -> Option<Decision<E>> {
    Some(match v.get("verdict")?.as_str()? {
        "pass" => Decision::Pass,
        "allow" => Decision::Allow,
        "deny" => Decision::Deny,
        "rewrite" => Decision::Rewrite(E::from_event(event_from_json(v.get("event")?)?)?),
        "answer" => {
            let a = v.get("answer")?;
            let answer = if let Some(ips) = a.as_array() {
                hooks::Answer::Lookup(ips.iter().filter_map(|ip| ip.as_str()?.parse().ok()).collect())
            } else {
                let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().as_bytes().to_vec();
                hooks::Answer::Exec(hooks::Reply { stdout: text("stdout"), stderr: text("stderr"), code: a.get("code").and_then(Value::as_i64).unwrap_or(0) as i32 })
            };
            Decision::Answer(E::answer_from(answer)?)
        }
        _ => return None,
    })
}

/// The launcher's side: decides an event the worker sent, with the machine's hooks here.
fn decide(hub: &Hub, event: Event) -> Value {
    match event {
        Event::Exec(e) => decision_json(hub.exec(e)),
        Event::Output(e) => decision_json(hub.output(e)),
        Event::Connect(e) => decision_json(hub.connect(e)),
        Event::Lookup(e) => decision_json(hub.lookup(e)),
    }
}

#[cfg(unix)]
mod unix {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::fd::FromRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::Notice;
    use crate::logs::{LogStream, Writer};
    use crate::stdio::Chunks;

    /// The worker's side of a hook link: a [`Hooks`] that asks the launcher.
    struct Remote {
        conn: Mutex<Option<(BufReader<UnixStream>, UnixStream)>>,
        next: AtomicU64,
        gone: AtomicBool,
        interest: Interest,
    }

    impl Remote {
        fn connect(path: &Path, interest: Interest) -> Result<Remote> {
            let stream = UnixStream::connect(path).map_err(|e| Error::io(format!("reaching the launcher's hooks: {e}")))?;
            let _ = stream.set_read_timeout(Some(DECISION_WAIT));
            let reader = BufReader::new(stream.try_clone().map_err(|e| Error::io(e.to_string()))?);
            Ok(Remote { conn: Mutex::new(Some((reader, stream))), next: AtomicU64::new(1), gone: AtomicBool::new(false), interest })
        }

        /// The launcher's decision, or None if it can't be asked.
        fn ask<E: Hooked>(&self, event: &E) -> Option<Decision<E>> {
            if self.gone.load(Ordering::Acquire) {
                return None;
            }
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
            let (reader, writer) = conn.as_mut()?;
            let line = json!({ "id": id, "event": event_json(&event.clone().into_event()) }).to_string() + "\n";
            let mut answer = String::new();
            let ok = writer.write_all(line.as_bytes()).is_ok() && reader.read_line(&mut answer).is_ok_and(|n| n > 0);
            let decision = ok.then(|| serde_json::from_str::<Value>(&answer).ok()).flatten().filter(|v| v.get("id").and_then(Value::as_u64) == Some(id)).and_then(|v| decision_from_json(&v));
            if decision.is_none() {
                self.gone.store(true, Ordering::Release);
                *conn = None;
            }
            decision
        }
    }

    impl Hooks for Remote {
        fn exec(&self, e: &hooks::Exec) -> Decision<hooks::Exec> {
            self.ask(e).unwrap_or(Decision::Deny)
        }
        fn output(&self, e: &hooks::Output) -> Decision<hooks::Output> {
            self.ask(e).unwrap_or(Decision::Pass)
        }
        fn connect(&self, e: &hooks::Connect) -> Decision<hooks::Connect> {
            self.ask(e).unwrap_or(Decision::Deny)
        }
        fn lookup(&self, e: &hooks::Lookup) -> Decision<hooks::Lookup> {
            self.ask(e).unwrap_or(Decision::Deny)
        }
        fn interest(&self) -> Interest {
            self.interest
        }
    }

    /// The launcher's side of a hook link: a socket the worker connects to once, then decisions for as long as it runs.
    fn listen(hub: Hub) -> Result<PathBuf> {
        static N: AtomicU64 = AtomicU64::new(0);
        // A socket's path has to be short: the temporary folder, not the home.
        let path = std::env::temp_dir().join(format!("carbon-hooks-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).map_err(|e| Error::io(format!("listening for the worker's hooks: {e}")))?;
        let at = path.clone();
        std::thread::Builder::new()
            .name("carbon hook link".into())
            .spawn(move || {
                let accepted = listener.accept();
                let _ = std::fs::remove_file(&at);
                let Ok((stream, _)) = accepted else { return };
                let Ok(mut writer) = stream.try_clone() else { return };
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { break };
                    let Some(event) = v.get("event").and_then(event_from_json) else { break };
                    let mut answer = decide(&hub, event);
                    answer["id"] = v.get("id").cloned().unwrap_or(Value::Null);
                    if writer.write_all((answer.to_string() + "\n").as_bytes()).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| Error::new(ErrorKind::Failed, format!("starting the hook link: {e}")))?;
        Ok(path)
    }

    pub(crate) fn launch(carbon: &Carbon, launch: Launch, rules: Rules, code_hooks: bool) -> Result<Machine> {
        if matches!(launch.stdin, crate::Stdio::Inherit | crate::Stdio::Piped) || launch.tty {
            return Err(Error::invalid("a launched machine has no stdin or terminal: exec into it for those"));
        }
        let interest = hub_interest(&launch.hub);
        let link = if code_hooks && !interest.is_empty() { Some((listen(launch.hub.clone())?, interest)) } else { None };
        let spec = spec_json(&carbon.home(), &launch, &rules, link.as_ref().map(|(p, i)| (p.as_path(), *i)));

        let mut fds = [0i32; 2];
        // SAFETY: an out-parameter for the pipe's two ends.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(Error::io(format!("a pipe for the worker: {}", std::io::Error::last_os_error())));
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        // SAFETY: the pipe's ends, which this owns. Neither reaches the worker as it is: the write end only as its copy
        // at REPORT_FD (which doesn't take the flag), so it ends when the worker closes its report.
        let report = unsafe {
            libc::fcntl(read_fd, libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(write_fd, libc::F_SETFD, libc::FD_CLOEXEC);
            std::fs::File::from_raw_fd(read_fd)
        };
        let exe = std::env::current_exe().map_err(|e| Error::io(format!("finding this program to run as a worker: {e}")))?;
        let mut command = std::process::Command::new(exe);
        command.env(WORKER_VAR, "1").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // SAFETY: between fork and exec, only async-signal-safe calls: a session of its own, and the report pipe at 3.
        unsafe {
            command.pre_exec(move || {
                libc::setsid();
                if write_fd == REPORT_FD {
                    libc::fcntl(REPORT_FD, libc::F_SETFD, 0);
                } else if libc::dup2(write_fd, REPORT_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        // SAFETY: the write end is the worker's now.
        unsafe { libc::close(write_fd) };
        let mut child = spawned.map_err(|e| Error::new(ErrorKind::Failed, format!("starting the machine's worker: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(spec.to_string().as_bytes());
        }
        // Collected when it ends, so it's never left a zombie that looks alive while this process lives.
        let _ = std::thread::Builder::new().name("carbon worker".into()).spawn(move || {
            let _ = child.wait();
        });
        let notify = carbon.notify();
        let mut started = None;
        for line in BufReader::new(report).lines() {
            let Ok(line) = line else { break };
            let (verb, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match verb {
                "notice" => {
                    let (kind, text) = rest.split_once(' ').unwrap_or(("warning", rest));
                    notify(match kind {
                        "progress" => Notice::progress(text.to_string()),
                        "fallback" => Notice::fallback(text.to_string()),
                        _ => Notice::warning(text.to_string()),
                    });
                }
                "ok" => {
                    started = Some(rest.to_string());
                    break;
                }
                "error" => {
                    let (kind, message) = rest.split_once(' ').unwrap_or(("failed", rest));
                    return Err(Error::new(kind_of(kind), message.replace("%0A", "\n")));
                }
                _ => {}
            }
        }
        match started {
            Some(name) => Ok(Machine::new(carbon.clone(), name)),
            None => Err(Error::new(ErrorKind::Failed, "the machine's worker ended before the machine started")),
        }
    }

    /// The worker's report pipe, a line at a time.
    #[derive(Clone)]
    struct Report(Arc<Mutex<Option<std::fs::File>>>);

    impl Report {
        fn line(&self, line: &str) {
            if let Some(file) = self.0.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                let _ = file.write_all(format!("{}\n", line.replace('\n', "%0A")).as_bytes());
            }
        }
        fn close(&self) {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).take();
        }
        fn is_open(&self) -> bool {
            self.0.lock().unwrap_or_else(|p| p.into_inner()).is_some()
        }
    }

    /// Where notices go: the launcher, until the machine's up, then the machine's log.
    type Log = Arc<Mutex<Option<Writer>>>;

    pub(crate) fn run_if_worker() {
        if std::env::var_os(WORKER_VAR).is_none() {
            return;
        }
        // SAFETY: init runs first in main, before any thread is made.
        unsafe { std::env::remove_var(WORKER_VAR) };
        // SAFETY: the report pipe the launcher put at 3.
        let report = Report(Arc::new(Mutex::new(Some(unsafe { std::fs::File::from_raw_fd(REPORT_FD) }))));
        let mut text = String::new();
        let _ = std::io::stdin().read_to_string(&mut text);
        let code = match serde_json::from_str::<Value>(&text).map_err(|e| Error::invalid(format!("a worker's spec: {e}"))).and_then(|v| work(&v, &report)) {
            Ok(code) => code,
            Err(e) => {
                report.line(&format!("error {} {}", kind_name(e.kind()), e.message()));
                1
            }
        };
        report.close();
        std::process::exit(code)
    }

    fn work(v: &Value, report: &Report) -> Result<i32> {
        let (home, launch, rules, link) = spec_from_json(v)?;
        let carbon = Carbon::open_at(&home)?;
        let log: Log = Arc::new(Mutex::new(None));
        {
            let (report, log) = (report.clone(), log.clone());
            carbon.on_notice(move |notice: Notice| {
                let kind = match notice.kind {
                    crate::NoticeKind::Progress => "progress",
                    crate::NoticeKind::Fallback => "fallback",
                    crate::NoticeKind::Warning => "warning",
                };
                if report.is_open() {
                    report.line(&format!("notice {kind} {}", notice.message));
                } else if let Some(writer) = log.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                    writer.write(LogStream::Stderr, format!("carbon: {}\n", notice.message).as_bytes());
                }
            });
        }
        // Its rules decide here; hooks with code are the launcher's, asked over the link.
        if !rules.is_empty() {
            launch.hub.add(rules);
        }
        if let Some((path, interest)) = link {
            launch.hub.add(Remote::connect(&path, interest)?);
        }
        let notify = carbon.notify();
        let store = &carbon.0.store;
        let drains: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::default();
        let ended = crate::boot::run(store, launch, &notify, &mut |started| {
            let dir = store.dir(&started.name);
            match Writer::create(&dir) {
                Ok(writer) => *log.lock().unwrap_or_else(|p| p.into_inner()) = Some(writer),
                Err(e) => report.line(&format!("notice warning its output isn't kept: {e}")),
            }
            let mut ends = started.ends;
            for (stream, chunks) in [(LogStream::Stdout, ends.stdout.take()), (LogStream::Stderr, ends.stderr.take())] {
                let Some(chunks) = chunks else { continue };
                let log = log.clone();
                if let Ok(thread) = std::thread::Builder::new().name("carbon log".into()).spawn(move || drain(chunks, stream, &log)) {
                    drains.lock().unwrap_or_else(|p| p.into_inner()).push(thread);
                }
            }
            // The launcher hears it's started once it can be reached: its kernel up, or it already over.
            let (report, stopper, name) = (report.clone(), started.stopper, started.name);
            let _ = std::thread::Builder::new().name("carbon worker report".into()).spawn(move || {
                stopper.wait_up(Duration::from_secs(60));
                report.line(&format!("ok {name}"));
                report.close();
            });
        });
        // What it wrote as it ended, kept before the worker goes: its streams end with it.
        for thread in std::mem::take(&mut *drains.lock().unwrap_or_else(|p| p.into_inner())) {
            let _ = thread.join();
        }
        ended
    }

    fn drain(chunks: Chunks, stream: LogStream, log: &Log) {
        for chunk in chunks {
            if let Some(writer) = log.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                writer.write(stream, &chunk);
            }
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::{launch, run_if_worker};

#[cfg(not(unix))]
pub(crate) fn launch(_carbon: &Carbon, _launch: Launch, _rules: Rules, _code_hooks: bool) -> Result<Machine> {
    Err(Error::unsupported("launching a machine in a process of its own isn't there yet on Windows hosts (it needs the control socket, which is Unix-only so far)"))
}

#[cfg(not(unix))]
pub(crate) fn run_if_worker() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_goes_to_the_worker_and_back() {
        let launch = Launch {
            name: Some("dev".into()),
            hostname: Some("sbx-8f3a".into()),
            source: Some(Source::Image("debian:13".into())),
            args: vec!["sleep".into(), "infinity".into()],
            isolation: Isolation::Sandbox,
            platform: images::Platform::parse("linux/amd64"),
            env: vec!["A=1".into()],
            workdir: Some("/srv".into()),
            display: Display::Headless(800, 600, 240),
            gpu: Gpu::None,
            audio: Audio::Wav("/tmp/x.wav".into()),
            memory: 512 << 20,
            network: Some(crate::Network::None),
            publish_all: true,
            publish: Some(vec!["2222:22".into()]),
            stdin: crate::Stdio::Null,
            stdout: crate::Stdio::Inherit,
            stderr: crate::Stdio::Inherit,
            tty: false,
            relay_signals: false,
            remove: true,
            timeout: Some(Duration::from_secs(5)),
            mounts: Some(vec![crate::Mount::volume("data", "/data")]),
            user: Some("node".into()),
            hub: Hub::new(),
        };
        let rules = Rules::new().deny_exec("rm -rf *").deny("0.0.0.0/0").unwrap();
        let v = spec_json(Path::new("/tmp/home"), &launch, &rules, Some((Path::new("/tmp/link.sock"), Interest::exec() | Interest::lookup())));
        let (home, back, rules_back, link) = spec_from_json(&v).unwrap();
        assert_eq!(home, PathBuf::from("/tmp/home"));
        assert_eq!((back.name, back.source, back.args), (launch.name, launch.source, launch.args));
        assert_eq!((back.isolation, back.platform, back.env, back.workdir), (Isolation::Sandbox, launch.platform, launch.env, launch.workdir));
        assert!(matches!(back.gpu, Gpu::None));
        assert_eq!((back.display, back.audio, back.memory), (launch.display, launch.audio, launch.memory));
        assert_eq!((back.network, back.publish, back.remove, back.timeout, back.publish_all), (launch.network, launch.publish, true, launch.timeout, true));
        assert_eq!(back.hostname.as_deref(), Some("sbx-8f3a"));
        assert_eq!(rules_back, rules);
        assert_eq!(link, Some((PathBuf::from("/tmp/link.sock"), Interest::exec() | Interest::lookup())));
    }

    #[test]
    fn decisions_go_over_the_link_and_back() {
        let exec = hooks::Exec::new("/bin/rm", ["rm", "-rf", "/"]);
        let rewritten = exec.clone().with_argv(["true"]);
        let back = |d: Decision<hooks::Exec>| decision_from_json::<hooks::Exec>(&decision_json(d));
        assert!(matches!(back(Decision::Deny), Some(Decision::Deny)));
        assert!(matches!(back(Decision::Pass), Some(Decision::Pass)));
        match back(Decision::Rewrite(rewritten)) {
            Some(Decision::Rewrite(e)) => assert_eq!(e.argv, vec!["true".to_string()]),
            other => panic!("{other:?}"),
        }
        match back(Decision::Answer(hooks::Reply::new().stdout("hi\n").exit(3))) {
            Some(Decision::Answer(r)) => assert_eq!((r.stdout.as_slice(), r.code), (&b"hi\n"[..], 3)),
            other => panic!("{other:?}"),
        }
        let hub = Hub::new();
        hub.on_exec(hooks::Filter::program("rm"), |_| Decision::deny());
        assert_eq!(decide(&hub, Event::Exec(exec))["verdict"], "deny");
    }
}
