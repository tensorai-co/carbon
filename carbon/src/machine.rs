//! A machine by name, running or not, and what can be done with it: [`Machine`], with [`Machine::exec`]'s
//! [`Process`], its [`Screen`], and what's known of it, [`MachineInfo`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use images::{Image, Platform};

use crate::error::{Error, Result};
use crate::spec::{Exit, Output};
use crate::store::{Kernel, Record};
use crate::wire::{self, ExecRequest, Watch};
use crate::{Carbon, Network};

/// A host port a machine's port is published on, while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// The machine's port, and its protocol (`tcp`, `udp`).
    pub port: u16,
    pub protocol: String,
    /// Where on the host it's reached.
    pub host: SocketAddr,
}

impl Port {
    /// From `0.0.0.0:2222->22/tcp`, as a record keeps it.
    fn parse(text: &str) -> Option<Port> {
        let (host, machine) = text.split_once("->")?;
        let (port, protocol) = machine.split_once('/').unwrap_or((machine, "tcp"));
        Some(Port { port: port.parse().ok()?, protocol: protocol.to_string(), host: host.parse().ok()? })
    }
}

impl std::fmt::Display for Port {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{} -> {}", self.port, self.protocol, self.host)
    }
}

/// What's kept of a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineInfo {
    pub name: String,
    pub kernel: Kernel,
    /// The image it was made from, in full (`docker.io/library/debian:13`), or the program (`C:\setup.exe`).
    pub image: Option<String>,
    pub program: Option<String>,
    pub platform: Platform,
    pub created: SystemTime,
    /// The process it runs in, while it runs.
    pub pid: Option<u32>,
    pub running: bool,
    /// Where it last ran: `vm/hvf`, `sandbox/linux`, ...
    pub backend: String,
    pub network: Network,
    /// Its ports as they were asked for (`2222:22`), and, while it runs, where they're published.
    pub publish: Vec<String>,
    pub ports: Vec<Port>,
    /// How its first process ended, the last time it ran (None while it runs, or if it never ended).
    pub exit: Option<i32>,
    /// Its host name: its own (`--hostname`), or its name.
    pub hostname: String,
    /// Its config: its image's, and what it was given (env, command, working folder, user, labels).
    pub config: images::Config,
    /// Its `/etc/hosts`' extra lines (`--add-host`).
    pub extra_hosts: Vec<crate::ExtraHost>,
}

impl MachineInfo {
    pub(crate) fn of(record: Record) -> MachineInfo {
        let running = record.running();
        let hostname = record.host_name();
        MachineInfo {
            hostname,
            config: record.config,
            extra_hosts: record.extra_hosts,
            running,
            pid: record.pid.filter(|_| running),
            ports: if running { record.ports.iter().filter_map(|p| Port::parse(p)).collect() } else { Vec::new() },
            name: record.name,
            kernel: record.kernel,
            image: record.image,
            program: record.program,
            platform: record.platform,
            created: UNIX_EPOCH + Duration::from_secs(record.created),
            backend: record.backend,
            network: record.network,
            publish: record.publish,
            exit: record.exit.filter(|_| !running),
        }
    }

    /// What it's from, as `carbon ps` shows it: `debian:13`, or the program.
    pub fn from(&self) -> String {
        self.image.as_deref().map(|i| i.strip_prefix("docker.io/library/").unwrap_or(i).to_string()).or(self.program.clone()).unwrap_or_default()
    }
}

/// How [`Machine::commit_with`] makes an image, as `docker commit`'s options do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub(crate) name: Option<String>,
    pub(crate) config: Option<images::Config>,
    pub(crate) changes: Vec<String>,
    pub(crate) step: Option<String>,
    pub(crate) comment: String,
    pub(crate) history: Option<Vec<images::History>>,
    pub(crate) source: Option<String>,
}

impl Commit {
    /// An unnamed image, with the machine's config, and `carbon commit NAME` as its step.
    pub fn new() -> Commit {
        Commit::default()
    }
    /// What it's called: `app:2`.
    pub fn name(mut self, name: impl Into<String>) -> Commit {
        self.name = Some(name.into());
        self
    }
    /// Its config, instead of the machine's.
    pub fn config(mut self, config: images::Config) -> Commit {
        self.config = Some(config);
        self
    }
    /// A Dockerfile instruction to apply to its config (`--change`): `CMD`, `ENTRYPOINT`, `ENV`, `EXPOSE`, `LABEL`,
    /// `ONBUILD`, `USER`, `VOLUME` or `WORKDIR`, as many as wanted, in order.
    pub fn change(mut self, instruction: impl Into<String>) -> Commit {
        self.changes.push(instruction.into());
        self
    }
    /// What its history says made it: `RUN apt-get install -y curl`.
    pub fn step(mut self, step: impl Into<String>) -> Commit {
        self.step = Some(step.into());
        self
    }
    /// Its history entry's comment (`--message`).
    pub fn comment(mut self, comment: impl Into<String>) -> Commit {
        self.comment = comment.into();
        self
    }
    /// Where `carbon images` says it came from, instead of the machine it was committed from.
    pub fn source(mut self, source: impl Into<String>) -> Commit {
        self.source = Some(source.into());
        self
    }
    /// Its whole history, instead of its base's with one step added.
    pub fn history(mut self, history: Vec<images::History>) -> Commit {
        self.history = Some(history);
        self
    }
}

/// A machine by name. It needn't be running: what needs it running, or stopped, says so when it isn't.
#[derive(Clone)]
pub struct Machine {
    carbon: Carbon,
    name: String,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Machine").field(&self.name).finish()
    }
}

impl Machine {
    pub(crate) fn new(carbon: Carbon, name: String) -> Machine {
        Machine { carbon, name }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn dir(&self) -> PathBuf {
        self.carbon.0.store.dir(&self.name)
    }

    pub fn info(&self) -> Result<MachineInfo> {
        Ok(MachineInfo::of(self.carbon.0.store.get(&self.name)?))
    }

    pub fn is_running(&self) -> bool {
        self.carbon.0.store.record(&self.name).is_some_and(|r| r.running())
    }

    fn running(&self) -> Result<()> {
        if self.carbon.0.store.get(&self.name)?.running() { Ok(()) } else { Err(Error::not_running(&self.name)) }
    }

    /// Where its ports are published, while it runs.
    pub fn ports(&self) -> Result<Vec<Port>> {
        Ok(self.info()?.ports)
    }

    /// A process to start in it, as `docker exec` starts one: with the machine's environment and working directory
    /// unless told otherwise.
    pub fn exec<I, S>(&self, argv: I) -> ExecSpec
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        ExecSpec { machine: self.clone(), request: ExecRequest { args: argv.into_iter().map(Into::into).collect(), ..ExecRequest::default() }, timeout: None }
    }

    /// Asks it to end, as `docker stop` does: SIGTERM to a Linux machine's first process (a Windows machine is
    /// ended), and everything ended ten seconds later if it hasn't. It returns at once; [`Machine::wait`] for it.
    pub fn stop(&self) -> Result<()> {
        let record = self.carbon.0.store.get(&self.name)?;
        if !record.running() {
            return Err(Error::not_running(&self.name));
        }
        // The signal its image says stops it (SIGTERM unless it says otherwise).
        match wire::ask(&self.dir(), &format!("stop {}", record.config.stop_signal_number())) {
            Ok(_) => Ok(()),
            // A machine from before there was a `stop` to ask: the process it's in, signalled.
            Err(_) => signal_process(&self.name, record.pid),
        }
    }

    /// Ends everything in it now.
    pub fn kill(&self) -> Result<()> {
        self.running()?;
        wire::ask(&self.dir(), "kill").map(|_| ())
    }

    /// Waits until it isn't running, up to `timeout` (forever if None). Whether it stopped.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let started = std::time::Instant::now();
        while self.is_running() {
            if timeout.is_some_and(|t| started.elapsed() >= t) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        true
    }

    /// What it wrote, kept, as lines: a launched machine's ([`crate::MachineSpec::launch`]), running or not.
    pub fn logs(&self, logs: crate::Logs) -> Result<crate::LogReader> {
        let (carbon, name) = (self.carbon.clone(), self.name.clone());
        let running = Box::new(move || carbon.0.store.record(&name).is_some_and(|r| r.running()));
        crate::logs::LogReader::open(&self.dir(), logs, running)
    }

    /// Its screen, while it runs.
    pub fn screen(&self) -> Screen {
        Screen { dir: self.dir() }
    }

    /// A live feed of what it does, as its hooks decide it: the kinds of event in `interest`. It runs in another
    /// process, or in this one; either way, this reads its control socket.
    pub fn watch(&self, interest: hooks::Interest) -> Result<Watch> {
        self.running()?;
        Watch::open(&self.dir(), interest)
    }

    /// An image of its disk as it is now, called `image`, which new machines can start from, as `docker commit`
    /// makes one: its image, and one more layer of what it changed. Running or stopped: a running machine's writes
    /// wait while its disk is read, so the image is the disk at one moment.
    pub fn commit(&self, image: &str) -> Result<Image> {
        self.commit_with(&Commit::new().name(image))
    }

    /// As [`Machine::commit`], saying what the image is called (or nothing, for an unnamed one found by its id), its
    /// config, and the step its history gives for it: what a build does after each step.
    pub fn commit_with(&self, how: &Commit) -> Result<Image> {
        self.carbon.0.store.commit(&self.name, how)
    }

    /// Its files, running or stopped: read, written, listed, made, moved and removed.
    pub fn files(&self) -> crate::Files {
        crate::Files { carbon: self.carbon.clone(), name: self.name.clone() }
    }

    /// Copies its file, or folder and all in it, at `path` (as it names it: `/etc/hosts`, `C:\Program Files\App`)
    /// into the host's folder `to`, under its own name, running or stopped: kinds, modes, times and links kept. How
    /// many files, folders and links.
    pub fn copy_out(&self, path: &str, to: impl AsRef<Path>, how: &crate::Copy) -> Result<usize> {
        self.files().copy_out(path, to.as_ref(), how)
    }

    /// Copies the host's file, or folder and all in it, into its folder `into`, under its own name (what's in the
    /// folder, for a `from` ending in `/.`), running or stopped: kinds, modes, times and links kept; root's unless
    /// `how` says otherwise. How many files, folders and links.
    pub fn copy_in(&self, from: impl AsRef<Path>, into: &str, how: &crate::Copy) -> Result<usize> {
        self.files().copy_in(from.as_ref(), into, how)
    }

    /// Removes it, and its files; its image stays. It must be stopped.
    pub fn remove(self) -> Result<()> {
        self.carbon.0.store.remove(&self.name)
    }
}

#[cfg(unix)]
fn signal_process(name: &str, pid: Option<u32>) -> Result<()> {
    let pid = pid.ok_or_else(|| Error::not_running(name))?;
    // SAFETY: asking a process to end.
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err(Error::io(format!("couldn't signal {name}'s process {pid}")));
    }
    Ok(())
}

#[cfg(not(unix))]
fn signal_process(_name: &str, _pid: Option<u32>) -> Result<()> {
    Err(Error::unsupported("stopping a machine from another process isn't there yet on Windows hosts"))
}

/// A process to start in a running machine.
pub struct ExecSpec {
    machine: Machine,
    request: ExecRequest,
    timeout: Option<Duration>,
}

impl ExecSpec {
    /// An environment variable, over the machine's.
    pub fn env(mut self, name: &str, value: &str) -> ExecSpec {
        self.request.env.retain(|e| e.split('=').next() != Some(name));
        self.request.env.push(format!("{name}={value}"));
        self
    }

    pub fn workdir(mut self, dir: impl Into<String>) -> ExecSpec {
        self.request.workdir = Some(dir.into());
        self
    }

    /// Who it runs as (`docker exec --user`): `name`, `uid`, `name:group`, `uid:gid`, from the machine's own
    /// `/etc/passwd` and `/etc/group`; the machine's first program's user by default.
    pub fn user(mut self, user: impl Into<String>) -> ExecSpec {
        self.request.user = Some(user.into());
        self
    }

    /// Kills it (SIGKILL; in a Windows machine, ends it) if it's still running this long after it starts. Its
    /// [`Output::timed_out`] says so, and [`Process::timed_out`].
    pub fn timeout(mut self, timeout: Duration) -> ExecSpec {
        self.timeout = Some(timeout);
        self
    }

    /// A terminal of its own, this big, instead of pipes (`-t`).
    pub fn tty(mut self, rows: u16, cols: u16) -> ExecSpec {
        self.request.tty = Some((rows, cols));
        self
    }

    fn checked(&self) -> Result<()> {
        if self.request.args.is_empty() {
            return Err(Error::invalid("exec needs a command"));
        }
        self.machine.running()
    }

    /// Starts it, with its stdin, stdout and stderr this program's to use.
    pub fn spawn(self) -> Result<Process> {
        self.checked()?;
        let mut process = Process::open(&self.machine.dir(), &self.request)?;
        if let Some(timeout) = self.timeout {
            process.watch(timeout);
        }
        Ok(process)
    }

    /// Runs it to its end, its stdin closed, and gives what it wrote.
    pub fn output(self) -> Result<Output> {
        let mut process = self.spawn()?;
        process.close_stdin();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let exit = loop {
            match process.next_event() {
                Some(ProcessEvent::Stdout(d)) => stdout.extend(d),
                Some(ProcessEvent::Stderr(d)) => stderr.extend(d),
                Some(ProcessEvent::Exit(code)) => break Exit(code),
                None => break Exit(255),
            }
        };
        Ok(Output { exit, stdout, stderr, timed_out: process.timed_out() })
    }

    /// Starts it and leaves it running: nothing comes back but whether it started.
    pub fn detach(mut self) -> Result<()> {
        self.checked()?;
        self.request.detach = true;
        wire::exec_attached(&self.machine.dir(), self.request, false).map(|_| ())
    }

    /// Runs it attached to this process, as `carbon exec` does: this process's stdin (if `interactive`), and with a
    /// [`ExecSpec::tty`], this terminal, raw, for as long; its signals passed on. Its exit code.
    pub fn attach(self, interactive: bool) -> Result<Exit> {
        self.checked()?;
        wire::exec_attached(&self.machine.dir(), self.request, interactive).map(Exit)
    }
}

/// What comes from a process started in a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// Its exit code, last.
    Exit(i32),
}

/// A process started in a running machine ([`ExecSpec::spawn`]): its stdin to write, what it writes, and how it ends.
pub struct Process {
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
    exited: Option<i32>,
    /// Set once its timeout has killed it; its watcher stops once it's ended.
    timed_out: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Process {
    #[cfg(unix)]
    fn open(dir: &Path, request: &ExecRequest) -> Result<Process> {
        let (stream, _) = wire::open(dir, &format!("exec {}", wire::request_json(request)))?;
        Ok(Process { stream, exited: None, timed_out: Default::default(), ended: Default::default() })
    }

    #[cfg(not(unix))]
    fn open(_dir: &Path, _request: &ExecRequest) -> Result<Process> {
        Err(Error::unsupported("exec into a running machine isn't there yet on Windows hosts"))
    }

    #[cfg(unix)]
    fn send(&self, kind: u8, data: &[u8]) -> Result<()> {
        use std::io::Write;
        (&self.stream).write_all(&wire::frame(kind, data)).map_err(|e| Error::io(format!("the process's machine: {e}")))
    }

    #[cfg(not(unix))]
    fn send(&self, _kind: u8, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported("exec isn't there yet on Windows hosts"))
    }

    /// Kills it if it hasn't ended `timeout` from now.
    fn watch(&mut self, timeout: Duration) {
        #[cfg(unix)]
        if let Ok(stream) = self.stream.try_clone() {
            let (timed_out, ended) = (self.timed_out.clone(), self.ended.clone());
            let _ = std::thread::Builder::new().name("carbon exec timeout".into()).spawn(move || {
                let (step, mut waited) = (Duration::from_millis(50), Duration::ZERO);
                while waited < timeout {
                    if ended.load(std::sync::atomic::Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(step);
                    waited += step;
                }
                if !ended.load(std::sync::atomic::Ordering::Acquire) {
                    timed_out.store(true, std::sync::atomic::Ordering::Release);
                    use std::io::Write;
                    let _ = (&stream).write_all(&wire::frame(3, &9i32.to_le_bytes()));
                }
            });
        }
        #[cfg(not(unix))]
        let _ = timeout;
    }

    /// Whether its timeout ([`ExecSpec::timeout`]) killed it.
    pub fn timed_out(&self) -> bool {
        self.timed_out.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Writes to its stdin.
    pub fn write_stdin(&self, data: &[u8]) -> Result<()> {
        self.send(0, data)
    }

    /// Ends its stdin.
    pub fn close_stdin(&self) {
        let _ = self.send(1, &[]);
    }

    /// Its terminal's new size, if it has one.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let mut d = rows.to_le_bytes().to_vec();
        d.extend(cols.to_le_bytes());
        self.send(2, &d)
    }

    /// Sends it a signal (in a Windows machine, any signal ends it).
    pub fn signal(&self, signal: i32) -> Result<()> {
        self.send(3, &signal.to_le_bytes())
    }

    /// A handle to write its stdin and signal it from another thread, while this one reads what it writes
    /// ([`Process::next_event`] waits, and needs it alone).
    pub fn handle(&self) -> Result<ProcessHandle> {
        #[cfg(unix)]
        {
            let stream = self.stream.try_clone().map_err(|e| Error::io(format!("the process's machine: {e}")))?;
            Ok(ProcessHandle { stream: std::sync::Arc::new(stream) })
        }
        #[cfg(not(unix))]
        Err(Error::unsupported("exec isn't there yet on Windows hosts"))
    }

    /// What it writes, in order, as it writes it, then its exit code; None after that.
    pub fn next_event(&mut self) -> Option<ProcessEvent> {
        if self.exited.is_some() {
            return None;
        }
        #[cfg(unix)]
        loop {
            match wire::read_frame(&mut self.stream) {
                Some((1, data)) => return Some(ProcessEvent::Stdout(data)),
                Some((2, data)) => return Some(ProcessEvent::Stderr(data)),
                Some((3, data)) if data.len() >= 4 => {
                    let code = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    self.exited = Some(code);
                    self.ended.store(true, std::sync::atomic::Ordering::Release);
                    return Some(ProcessEvent::Exit(code));
                }
                Some(_) => {}
                None => {
                    self.exited = Some(255);
                    self.ended.store(true, std::sync::atomic::Ordering::Release);
                    return Some(ProcessEvent::Exit(255));
                }
            }
        }
        #[cfg(not(unix))]
        None
    }

    /// Its stdout, as it comes, until it ends: stderr is dropped. [`Process::wait`] then gives its exit code.
    pub fn stdout(&mut self) -> impl Iterator<Item = Vec<u8>> + '_ {
        std::iter::from_fn(move || loop {
            match self.next_event()? {
                ProcessEvent::Stdout(data) => return Some(data),
                ProcessEvent::Stderr(_) => {}
                ProcessEvent::Exit(_) => return None,
            }
        })
    }

    /// Waits for it to end (what it writes until then is dropped), and how it did.
    pub fn wait(&mut self) -> Exit {
        while let Some(event) = self.next_event() {
            if let ProcessEvent::Exit(code) = event {
                return Exit(code);
            }
        }
        Exit(self.exited.unwrap_or(255))
    }
}

impl Iterator for Process {
    type Item = ProcessEvent;
    fn next(&mut self) -> Option<ProcessEvent> {
        self.next_event()
    }
}

/// A [`Process`]'s stdin and signals, apart from what it writes: [`Process::handle`]. Clones share the process.
#[derive(Debug, Clone)]
pub struct ProcessHandle {
    #[cfg(unix)]
    stream: std::sync::Arc<std::os::unix::net::UnixStream>,
}

impl ProcessHandle {
    #[cfg(unix)]
    fn send(&self, kind: u8, data: &[u8]) -> Result<()> {
        use std::io::Write;
        (&*self.stream).write_all(&wire::frame(kind, data)).map_err(|e| Error::io(format!("the process's machine: {e}")))
    }

    #[cfg(not(unix))]
    fn send(&self, _kind: u8, _data: &[u8]) -> Result<()> {
        Err(Error::unsupported("exec isn't there yet on Windows hosts"))
    }

    /// Writes to its stdin.
    pub fn write_stdin(&self, data: &[u8]) -> Result<()> {
        self.send(0, data)
    }

    /// Ends its stdin.
    pub fn close_stdin(&self) {
        let _ = self.send(1, &[]);
    }

    /// Its terminal's new size, if it has one.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let mut d = rows.to_le_bytes().to_vec();
        d.extend(cols.to_le_bytes());
        self.send(2, &d)
    }

    /// Sends it a signal (in a Windows machine, any signal ends it).
    pub fn signal(&self, signal: i32) -> Result<()> {
        self.send(3, &signal.to_le_bytes())
    }
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

/// One of a desktop's top-level windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: u64,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// `active`, `minimized`, `hidden`, `maximized`, `fullscreen`, `attention`, `popup of N`, `dialog of N`.
    pub states: Vec<String>,
    pub title: String,
}

/// A running machine's desktop, for computer use: its screen as a PNG, its windows, and input as a user's.
#[derive(Debug, Clone)]
pub struct Screen {
    dir: PathBuf,
}

impl Screen {
    fn ask(&self, request: &str) -> Result<Vec<u8>> {
        wire::ask(&self.dir, request)
    }

    fn text(&self, request: &str) -> Result<String> {
        self.ask(request).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// The screen, as a PNG.
    pub fn screenshot(&self) -> Result<Vec<u8>> {
        self.ask("screenshot")
    }

    /// Its windows, top first.
    pub fn windows(&self) -> Result<Vec<Window>> {
        let text = self.text("windows-json")?;
        let list: serde_json::Value = serde_json::from_str(&text).map_err(|e| Error::io(format!("the machine's windows: {e}")))?;
        let n = |w: &serde_json::Value, k: &str| w.get(k).and_then(serde_json::Value::as_i64).unwrap_or(0);
        Ok(list
            .as_array()
            .into_iter()
            .flatten()
            .map(|w| Window {
                id: n(w, "id") as u64,
                x: n(w, "x") as i32,
                y: n(w, "y") as i32,
                width: n(w, "width") as i32,
                height: n(w, "height") as i32,
                states: w.get("states").and_then(serde_json::Value::as_array).map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                title: w.get("title").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
            })
            .collect())
    }

    /// Its windows as `carbon display windows` shows them: a table.
    pub fn windows_table(&self) -> Result<String> {
        self.text("windows")
    }

    pub fn move_to(&self, x: i32, y: i32) -> Result<()> {
        self.ask(&format!("move {x} {y}")).map(|_| ())
    }

    /// A left click there.
    pub fn click(&self, x: i32, y: i32) -> Result<()> {
        self.click_with(x, y, Button::Left, 1)
    }

    /// A click with `button`, `count` times (2 for a double click).
    pub fn click_with(&self, x: i32, y: i32, button: Button, count: u32) -> Result<()> {
        let button = match button {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => "middle",
        };
        self.ask(&format!("click {x} {y} {button} {count}")).map(|_| ())
    }

    /// A drag with the left button.
    pub fn drag(&self, from: (i32, i32), to: (i32, i32)) -> Result<()> {
        self.ask(&format!("drag {} {} {} {}", from.0, from.1, to.0, to.1)).map(|_| ())
    }

    /// Scrolls there, up and right positive, in notches.
    pub fn scroll(&self, x: i32, y: i32, notches: i32, sideways: i32) -> Result<()> {
        self.ask(&format!("scroll {x} {y} {notches} {sideways}")).map(|_| ())
    }

    /// Types text, as keys on a US keyboard (and anything else as an input method commits it).
    pub fn type_text(&self, text: &str) -> Result<()> {
        self.ask(&format!("type {}", wire::escape(text))).map(|_| ())
    }

    /// Presses keys: `enter`, `esc`, `ctrl+s`, `alt+f4`; several, one after another, separated by spaces.
    pub fn key(&self, combo: &str) -> Result<()> {
        self.ask(&format!("key {combo}")).map(|_| ())
    }

    /// Its screen's size, and scale (1.0, 1.5, 2.0, ...).
    pub fn resize(&self, width: i32, height: i32, scale: f32) -> Result<()> {
        self.ask(&format!("resize {width}x{height}@{scale}")).map(|_| ())
    }

    /// Its clipboard's text.
    pub fn clipboard(&self) -> Result<String> {
        self.text("clipboard")
    }

    /// Makes its clipboard `text`, as if copied on the host.
    pub fn set_clipboard(&self, text: &str) -> Result<()> {
        self.ask(&format!("clipboard {}", wire::escape(text))).map(|_| ())
    }

    /// Drops text there, as dragged in from the host: what the window there did with it.
    pub fn drop_text(&self, x: i32, y: i32, text: &str) -> Result<String> {
        self.text(&format!("drop {x} {y} text {}", wire::escape(text)))
    }

    /// Drops files there (paths as the machine names them).
    pub fn drop_files(&self, x: i32, y: i32, paths: &[&str]) -> Result<String> {
        self.text(&format!("drop {x} {y} files {}", wire::escape(&paths.join("\n"))))
    }

    /// Sends an input request as `carbon display` writes it: `click 10 20 right`, `key ctrl+s`.
    pub fn send(&self, verb: &str, args: &[String]) -> Result<()> {
        let rest = if verb == "type" { wire::escape(&args.join(" ")) } else { args.join(" ") };
        self.ask(&format!("{verb} {rest}")).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_as_records_keep_them() {
        let port = Port::parse("0.0.0.0:2222->22/tcp").unwrap();
        assert_eq!((port.port, port.protocol.as_str(), port.host.port()), (22, "tcp", 2222));
        assert_eq!(port.to_string(), "22/tcp -> 0.0.0.0:2222");
        assert_eq!(Port::parse("[::]:5353->53/udp").unwrap().protocol, "udp");
        assert!(Port::parse("nonsense").is_none());
    }
}
