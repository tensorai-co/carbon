//! A machine to run: [`MachineSpec`], made by [`Carbon::machine`], [`Carbon::program`] or [`Carbon::again`], and
//! what running it gives: an [`Exit`], an [`Output`], or a [`Running`] machine.

use std::path::PathBuf;
use std::thread::JoinHandle;

use exec::Isolation;
use hooks::{Connect, Decision, Exec, Filter, Hooks, Hub, Lookup};
use images::Platform;

use crate::boot::{self, Launch, Started};
use crate::error::{Error, ErrorKind, Result};
use crate::machine::Machine;
use crate::stdio::{Chunks, Stdin, Stdio};
use crate::stop::Stopper;
use crate::store::Source;
use crate::{Carbon, Gpu, Network, Publish, Rules};

/// Where a machine's windows go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// A screen in memory this big (in logical pixels), at a scale (in 120ths: 120 is 1×, 240 is 2×).
    Headless(i32, i32, u32),
    /// On the host's screen, in a window whose content starts this big (in points), resized as the user likes. On
    /// macOS, it takes the main thread: [`MachineSpec::run`] it from `main`.
    Host(i32, i32),
    /// On the host's screen, each window in a host window of its own, among the host's other windows.
    HostWindows,
    /// No screen at all.
    None,
}

impl Display {
    /// A screen in memory, at 1×.
    pub fn headless(width: i32, height: i32) -> Display {
        Display::Headless(width, height, display::output::SCALE_1X)
    }

    /// `none`, `host`, `host=WxH`, `host-windows`, `headless=WxH` or `headless=WxH@SCALE`, as `--display` takes it.
    pub fn parse(text: &str) -> Option<Display> {
        match text {
            "none" => Some(Display::None),
            "host" => Some(Display::Host(1280, 800)),
            "host-windows" => Some(Display::HostWindows),
            _ if text.starts_with("host=") => {
                let (w, h, scale) = display::output::parse_size(&text["host=".len()..])?;
                // The window's scale is the host's own: no @SCALE.
                (scale == display::output::SCALE_1X).then_some(Display::Host(w, h))
            }
            _ => {
                let (w, h, scale) = display::output::parse_size(text.strip_prefix("headless=")?)?;
                Some(Display::Headless(w, h, scale))
            }
        }
    }
}

impl Default for Display {
    fn default() -> Display {
        Display::headless(1280, 800)
    }
}

/// Where a machine's sound goes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Audio {
    /// The host's speakers when the machine's on the host's screen, else nowhere.
    #[default]
    Auto,
    /// The host's speakers (Core Audio's default output on a Mac).
    Host,
    /// Everything played, into a WAV file (16-bit stereo, 48 kHz).
    Wav(PathBuf),
    /// Nowhere: sound is taken at the pace it would play, and heard by no one.
    None,
}

/// A size in bytes, as `512m`, `4g` or `100`: what [`MachineSpec::memory`] takes.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase();
    let (number, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => (&text[..at], &text[at..]),
        None => (text.as_str(), ""),
    };
    let shift = match unit.trim_end_matches('b').trim_end_matches('i') {
        "" => 0,
        "k" => 10,
        "m" => 20,
        "g" => 30,
        _ => return None,
    };
    number.parse::<u64>().ok().and_then(|n| n.checked_shl(shift))
}

/// How much memory a machine may use: bytes, or text as [`parse_size`] reads it.
pub trait MemorySize {
    fn bytes(&self) -> Option<u64>;
}

impl MemorySize for u64 {
    fn bytes(&self) -> Option<u64> {
        Some(*self)
    }
}

impl MemorySize for &str {
    fn bytes(&self) -> Option<u64> {
        parse_size(self)
    }
}

impl MemorySize for String {
    fn bytes(&self) -> Option<u64> {
        parse_size(self)
    }
}

/// How a machine's first process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub i32);

impl Exit {
    pub fn code(self) -> i32 {
        self.0
    }
    pub fn success(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit {}", self.0)
    }
}

/// A run to its end, with what it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// It was killed at its timeout ([`crate::ExecSpec::timeout`]); what it wrote until then is here.
    pub timed_out: bool,
}

impl Output {
    pub fn code(&self) -> i32 {
        self.exit.0
    }
    pub fn success(&self) -> bool {
        self.exit.success()
    }
    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A machine to run, as a builder. Nothing happens until [`MachineSpec::run`], [`MachineSpec::output`] or
/// [`MachineSpec::start`].
///
/// By default a machine has no stdin (it's at its end from the start, as `docker run` without `-i`), writes to this
/// process's stdout and stderr, gets a hardware VM where there is one (and says so when it falls back), 4 GiB of
/// memory (less in a smaller container: [`default_memory`]) it takes only as it touches it, a network of its own (NAT), and a 1280×800 screen in memory.
pub struct MachineSpec {
    carbon: Carbon,
    launch: Launch,
    invalid: Option<Error>,
    /// Its rules, as data, for a machine launched in a worker ([`MachineSpec::launch`]), where they're decided.
    rules: Rules,
    /// Hooks with code were added, or its hub was asked for: a launched machine asks this process about them.
    code_hooks: std::cell::Cell<bool>,
}

/// A machine's memory unless it's told otherwise: 4 GiB, or three quarters of what this process may use if that's
/// less (a container's limit, as on Cloud Run), and never under 256 MiB. Guest memory is only backed as it's touched,
/// but it's backed in this process's cgroup, so a machine that filled more than its container has would get the
/// container killed.
pub fn default_memory() -> u64 {
    const MOST: u64 = 4 << 30;
    const LEAST: u64 = 256 << 20;
    match host::memory_limit() {
        Some(limit) => (limit / 4 * 3).clamp(LEAST, MOST) & !((1 << 20) - 1),
        None => MOST,
    }
}

impl MachineSpec {
    pub(crate) fn new(carbon: Carbon, name: Option<String>, source: Option<Source>) -> MachineSpec {
        let launch = Launch {
            name,
            hostname: None,
            labels: Vec::new(),
            extra_hosts: None,
            source,
            args: Vec::new(),
            isolation: Isolation::Auto,
            platform: None,
            env: Vec::new(),
            workdir: None,
            display: Display::default(),
            gpu: Gpu::Auto,
            audio: Audio::Auto,
            memory: default_memory(),
            network: None,
            publish_all: false,
            publish: None,
            stdin: Stdio::Null,
            stdout: Stdio::Inherit,
            stderr: Stdio::Inherit,
            tty: false,
            relay_signals: false,
            remove: false,
            timeout: None,
            mounts: None,
            user: None,
            registries: None,
            hub: Hub::new(),
        };
        MachineSpec { carbon, launch, invalid: None, rules: Rules::new(), code_hooks: std::cell::Cell::new(false) }
    }

    fn invalid(mut self, error: Error) -> MachineSpec {
        self.invalid.get_or_insert(error);
        self
    }

    /// Its name: a new machine's, kept so it can be started again ([`Carbon::again`]). Without one, a new machine is
    /// named after its image or program (`debian`, `debian-2`, ...).
    pub fn name(mut self, name: impl Into<String>) -> MachineSpec {
        self.launch.name = Some(name.into());
        self
    }

    /// Its host name, as `uname` and its network give it (`--hostname`, as Docker's): kept, so it has it each time it
    /// runs. Without one, its name. Letters, digits and `-`, in dot-separated labels of at most 63, 253 in all.
    pub fn hostname(mut self, hostname: impl Into<String>) -> MachineSpec {
        let hostname = hostname.into();
        if !valid_hostname(&hostname) {
            return self.invalid(Error::invalid(format!("--hostname {hostname}: letters, digits and '-', in labels of at most 63 characters, separated by '.'")));
        }
        self.launch.hostname = Some(hostname);
        self
    }

    /// A label, as Docker's (`--label`): kept in its config, over its image's own, and so in an image committed
    /// from it.
    pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> MachineSpec {
        let key = key.into();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return self.invalid(Error::invalid(format!("--label {key:?}: a label's key is one word")));
        }
        self.launch.labels.push((key, value.into()));
        self
    }

    /// A line its `/etc/hosts` gets (`--add-host`): `name` at `ip`. Kept, as its hostname is; a run that gives any
    /// replaces what it had.
    pub fn add_host(mut self, name: impl Into<String>, ip: std::net::IpAddr) -> MachineSpec {
        match crate::ExtraHost::new(name, ip) {
            Ok(host) => {
                self.launch.extra_hosts.get_or_insert_with(Vec::new).push(host);
                self
            }
            Err(e) => self.invalid(e),
        }
    }

    /// A hardware VM ([`Isolation::Vm`]), the process sandbox, or the best there is ([`Isolation::Auto`], the
    /// default, which says so when it falls back).
    pub fn isolation(mut self, isolation: Isolation) -> MachineSpec {
        self.launch.isolation = isolation;
        self
    }

    /// Which build of an image to pull, if it isn't here: Linux for this host's CPU by default.
    pub fn platform(mut self, platform: Platform) -> MachineSpec {
        self.launch.platform = Some(platform);
        self
    }

    /// An environment variable, over the image's.
    pub fn env(mut self, name: &str, value: &str) -> MachineSpec {
        self.launch.env.retain(|e| e.split('=').next() != Some(name));
        self.launch.env.push(format!("{name}={value}"));
        self
    }

    /// Where its first program starts: the image's working directory by default.
    pub fn workdir(mut self, dir: impl Into<String>) -> MachineSpec {
        self.launch.workdir = Some(dir.into());
        self
    }

    /// How much memory it may use: bytes, or `"512m"`, `"4g"`. Only what it touches is taken.
    pub fn memory(mut self, size: impl MemorySize) -> MachineSpec {
        match size.bytes() {
            Some(bytes) if bytes >= 16 << 20 => self.launch.memory = bytes,
            _ => return self.invalid(Error::invalid("a machine's memory is a size of 16m or more: 512m, 4g")),
        }
        self
    }

    /// What its network reaches: its own addresses out through the host ([`Network::Nat`], the default), the host's
    /// ports themselves ([`Network::Host`]), or loopback only ([`Network::None`]).
    pub fn network(mut self, network: Network) -> MachineSpec {
        self.launch.network = Some(network);
        self
    }

    /// A host port that reaches one of its ports, as Docker's `-p` writes it: `2222:22`, `127.0.0.1:8080:80`,
    /// `5353:53/udp`.
    pub fn publish(mut self, port: &str) -> MachineSpec {
        if let Err(why) = Publish::parse(port) {
            return self.invalid(Error::invalid(format!("--publish {why}")));
        }
        self.launch.publish.get_or_insert_with(Vec::new).push(port.to_string());
        self
    }

    /// Every port its image says it listens on (`EXPOSE`), each published on a free host port, as Docker's `-P`.
    pub fn publish_all(mut self) -> MachineSpec {
        self.launch.publish_all = true;
        self
    }

    /// Who its first program runs as: `name`, `uid`, `name:group` or `uid:gid`, from its image's `/etc/passwd` and
    /// `/etc/group` (Docker's `--user`). The image's `USER` by default, else root. Anyone but root runs without root's
    /// capabilities, and files are checked against them.
    pub fn user(mut self, user: impl Into<String>) -> MachineSpec {
        self.launch.user = Some(user.into());
        self
    }

    /// Mounts a host's folder, a volume or a tmpfs in it ([`crate::Mount`]), as many as wanted, in order. A machine
    /// started again keeps its mounts unless it's given new ones.
    pub fn mount(mut self, mount: crate::Mount) -> MachineSpec {
        self.launch.mounts.get_or_insert_with(Vec::new).push(mount);
        self
    }

    pub fn display(mut self, display: Display) -> MachineSpec {
        self.launch.display = display;
        self
    }

    pub fn gpu(mut self, gpu: Gpu) -> MachineSpec {
        self.launch.gpu = gpu;
        self
    }

    pub fn audio(mut self, audio: Audio) -> MachineSpec {
        self.launch.audio = audio;
        self
    }

    /// Where its stdin comes from: nothing ([`Stdio::Null`], the default), this process's ([`Stdio::Inherit`], as
    /// `-i`), or a pipe ([`Stdio::Piped`]: [`Running::take_stdin`]).
    pub fn stdin(mut self, stdio: Stdio) -> MachineSpec {
        self.launch.stdin = stdio;
        self
    }

    /// Where its stdout goes: this process's ([`Stdio::Inherit`], the default), nowhere, or a pipe
    /// ([`Running::take_stdout`]).
    pub fn stdout(mut self, stdio: Stdio) -> MachineSpec {
        self.launch.stdout = stdio;
        self
    }

    pub fn stderr(mut self, stdio: Stdio) -> MachineSpec {
        self.launch.stderr = stdio;
        self
    }

    /// With stdin and stdout this process's, and a terminal: the terminal is the machine's console, in raw mode, with
    /// its kernel's own line editing, so a shell's prompt, Ctrl-C and full-screen programs work (`-it`).
    pub fn tty(mut self, tty: bool) -> MachineSpec {
        self.launch.tty = tty;
        self
    }

    /// Takes this process's SIGINT, SIGTERM and SIGHUP for as long as the machine runs, and gives them to it: the
    /// first asks it to end, a second (or ten seconds) ends it. For a program that is the machine, as `carbon run` is.
    pub fn relay_signals(mut self, relay: bool) -> MachineSpec {
        self.launch.relay_signals = relay;
        self
    }

    /// Removes it, and its files, once it ends, as Docker's `--rm`: for a machine that's only for one command.
    pub fn rm(mut self, remove: bool) -> MachineSpec {
        self.launch.remove = remove;
        self
    }

    /// Stops it, as [`Running::stop`] does, if it's still running this long after it starts.
    pub fn timeout(mut self, timeout: std::time::Duration) -> MachineSpec {
        self.launch.timeout = Some(timeout);
        self
    }

    /// Adds a policy to its hooks, after any added before.
    pub fn hooks(self, hooks: impl Hooks) -> MachineSpec {
        self.launch.hub.add(hooks);
        self.code_hooks.set(true);
        self
    }

    /// Adds rules to its hooks.
    pub fn rules(mut self, rules: Rules) -> MachineSpec {
        if !rules.is_empty() {
            self.launch.hub.add(rules.clone());
            self.rules = std::mem::take(&mut self.rules).merge(rules);
        }
        self
    }

    /// Decides the programs it starts that `filter` takes.
    pub fn on_exec(self, filter: Filter, f: impl Fn(&Exec) -> Decision<Exec> + Send + Sync + 'static) -> MachineSpec {
        self.launch.hub.on_exec(filter, f);
        self.code_hooks.set(true);
        self
    }

    /// Decides what it writes to its stdout and stderr (and an `exec`'s).
    pub fn on_output(self, filter: Filter, f: impl Fn(&hooks::Output) -> Decision<hooks::Output> + Send + Sync + 'static) -> MachineSpec {
        self.launch.hub.on_output(filter, f);
        self.code_hooks.set(true);
        self
    }

    /// Decides the connections it makes.
    pub fn on_connect(self, filter: Filter, f: impl Fn(&Connect) -> Decision<Connect> + Send + Sync + 'static) -> MachineSpec {
        self.launch.hub.on_connect(filter, f);
        self.code_hooks.set(true);
        self
    }

    /// Decides the names it looks up.
    pub fn on_lookup(self, filter: Filter, f: impl Fn(&Lookup) -> Decision<Lookup> + Send + Sync + 'static) -> MachineSpec {
        self.launch.hub.on_lookup(filter, f);
        self.code_hooks.set(true);
        self
    }

    /// Decides the calls its package managers make to its registries ([`MachineSpec::registries`]).
    pub fn on_registry(self, filter: Filter, f: impl Fn(&hooks::Registry) -> Decision<hooks::Registry> + Send + Sync + 'static) -> MachineSpec {
        self.launch.hub.on_registry(filter, f);
        self.code_hooks.set(true);
        self
    }

    /// Its registries (`--registries`): its package managers answered at `registry.carbon` by Carbon, and each request
    /// sent to `registries`' servers as one checked call, with their token, which never goes in the machine. The
    /// servers are asked what they serve when it starts, and a service one doesn't have stops it there. Only for this
    /// run, as its rules are.
    pub fn registries(mut self, registries: crate::Registries) -> MachineSpec {
        if let Err(e) = registries.check(&crate::registries::known()) {
            return self.invalid(Error::invalid(e));
        }
        self.launch.registries = Some(registries);
        self
    }

    /// Its hooks, to observe or subscribe to before it starts: nothing it does is missed.
    pub fn hub(&self) -> &Hub {
        self.code_hooks.set(true);
        &self.launch.hub
    }

    fn with_args<I, S>(mut self, argv: I) -> Result<MachineSpec>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if let Some(error) = self.invalid.take() {
            return Err(error);
        }
        self.launch.args = argv.into_iter().map(Into::into).collect();
        // A machine started again keeps the rules it was made with ([`MachineSpec::create`]), with these added.
        if self.launch.source.is_none()
            && let Some(record) = self.launch.name.as_deref().and_then(|n| self.carbon.0.store.record(n))
            && !record.rules.is_empty()
        {
            self = self.rules(record.rules);
        }
        Ok(self)
    }

    /// Runs it on this thread until its first process ends: `argv`, or what its image says if it's empty.
    pub fn run<I, S>(self, argv: I) -> Result<Exit>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let spec = self.with_args(argv)?;
        let notify = spec.carbon.notify();
        boot::run(&spec.carbon.0.store, spec.launch, &notify, &mut |_| {}).map(Exit)
    }

    /// Runs it to its end, and gives what it wrote to its stdout and stderr.
    pub fn output<I, S>(self, argv: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut running = self.stdout(Stdio::Piped).stderr(Stdio::Piped).start(argv)?;
        let (mut out, mut err) = (running.take_stdout(), running.take_stderr());
        let reading = std::thread::spawn(move || err.as_mut().map(Chunks::collect_all).unwrap_or_default());
        let stdout = out.as_mut().map(Chunks::collect_all).unwrap_or_default();
        let stderr = reading.join().unwrap_or_default();
        let exit = running.wait()?;
        Ok(Output { exit, stdout, stderr, timed_out: false })
    }

    /// Makes it without running it, as `docker create`: a stopped machine (its image pulled if it isn't here), whose
    /// [`Machine::files`] can be written before it first runs, and which [`Carbon::again`] runs.
    ///
    /// What it's given is kept for each time it runs, as Docker keeps a container's config: its env, workdir and user
    /// over its image's, `argv` (if any) as its command (an image's entrypoint stays), its host name, its network,
    /// ports and mounts, and its rules.
    pub fn create(self) -> Result<Machine> {
        self.create_with(Vec::<String>::new())
    }

    /// As [`MachineSpec::create`], with `argv` as its command: Docker's `Cmd`.
    pub fn create_with<I, S>(self, argv: I) -> Result<Machine>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let spec = self.with_args(argv)?;
        let launch = spec.launch;
        let store = &spec.carbon.0.store;
        let source = launch.source.as_ref().ok_or_else(|| Error::invalid("create makes a new machine: give it an image or a program"))?;
        if let Some(name) = launch.name.as_deref()
            && store.record(name).is_some()
        {
            return Err(Error::new(ErrorKind::AlreadyExists, format!("there's a machine called {name} already")));
        }
        let notify = spec.carbon.notify();
        let (mut record, program) = store.create(launch.name.as_deref(), launch.platform.as_ref(), source, &|n| notify(n))?;
        if let Some(network) = launch.network {
            record.network = network;
        }
        if let Some(publish) = launch.publish {
            record.publish = publish;
        }
        if let Some(mounts) = launch.mounts {
            record.mounts = mounts;
        }
        for var in &launch.env {
            let key = var.split('=').next().unwrap_or("");
            record.config.env.retain(|e| e.split('=').next() != Some(key));
            record.config.env.push(var.clone());
        }
        if let Some(workdir) = &launch.workdir {
            record.config.workdir = workdir.clone();
        }
        if let Some(user) = &launch.user {
            record.config.user = user.clone();
        }
        if !launch.args.is_empty() && record.program.is_none() {
            record.config.cmd = launch.args.clone();
        }
        record.hostname = launch.hostname.clone();
        for (key, value) in &launch.labels {
            record.config.labels.insert(key.clone(), value.clone());
        }
        if let Some(hosts) = &launch.extra_hosts {
            record.extra_hosts = hosts.clone();
        }
        record.rules = spec.rules.clone();
        store.save_record(&record)?;
        let machine = Machine::new(spec.carbon.clone(), record.name.clone());
        if let (Some(bytes), Some(path)) = (program, &record.program) {
            machine.files().write(path, bytes, crate::Write::new().mode(0o755).parents(true))?;
        }
        Ok(machine)
    }

    /// Launches it in a process of its own, a worker (this binary, run again by [`crate::init`]), and gives it back
    /// once its kernel is up: a [`Machine`] to `exec` into, read the [`Machine::logs`] of, stop and wait for, from this
    /// process or any other. It runs on when this process ends, until its first process does.
    ///
    /// Its stdin is empty, and what it writes is kept ([`Machine::logs`]). Its [`Rules`] go to the worker, which
    /// decides them there; hooks with code stay here, and the worker asks this process about each event they want, so
    /// those decisions need this process running: once it's gone, a launched machine's programs and connections that
    /// hooks with code wanted are refused (its output passes). Each machine in a worker of its own has a VM of its
    /// own, so several run at once on macOS, where a process has one.
    pub fn launch<I, S>(self, argv: I) -> Result<Machine>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let spec = self.with_args(argv)?;
        crate::worker::launch(&spec.carbon, spec.launch, spec.rules, spec.code_hooks.get())
    }

    /// Starts it on a thread of its own, and gives it back once it's made, before its kernel boots: a [`Running`]
    /// machine, with its piped streams, to `exec` into, watch, stop and wait for.
    ///
    /// On macOS, Hypervisor.framework allows one VM in a process at a time, so a second machine started while one runs
    /// falls back to the sandbox (and says so): [`MachineSpec::launch`] gives each a process, and a VM, of its own.
    pub fn start<I, S>(self, argv: I) -> Result<Running>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let spec = self.with_args(argv)?;
        let carbon = spec.carbon.clone();
        let hub = spec.launch.hub.clone();
        let (tell, told) = std::sync::mpsc::channel::<Started>();
        let thread = std::thread::Builder::new()
            .name("carbon machine".into())
            .spawn(move || {
                let notify = spec.carbon.notify();
                boot::run(&spec.carbon.0.store, spec.launch, &notify, &mut |started| {
                    let _ = tell.send(started);
                })
            })
            .map_err(|e| Error::new(ErrorKind::Failed, format!("starting the machine's thread: {e}")))?;
        match told.recv() {
            Ok(started) => {
                // Given back once its kernel is up, so it can be `exec`ed into at once.
                started.stopper.wait_up(std::time::Duration::from_secs(30));
                Ok(Running {
                machine: Machine::new(carbon, started.name),
                thread: Some(thread),
                stopper: started.stopper,
                stdin: started.ends.stdin,
                stdout: started.ends.stdout,
                stderr: started.ends.stderr,
                hub,
            })
            }
            // It ended before it started: why.
            Err(_) => match thread.join() {
                Ok(Err(e)) => Err(e),
                Ok(Ok(_)) => Err(Error::new(ErrorKind::Failed, "the machine ended before it started")),
                Err(_) => Err(Error::new(ErrorKind::Failed, "the machine's thread panicked")),
            },
        }
    }
}

/// A machine this process is running, on a thread of its own. Dropping it leaves the machine running until its first
/// process ends (or this process does); [`Running::wait`] waits for that.
pub struct Running {
    machine: Machine,
    thread: Option<JoinHandle<Result<i32>>>,
    stopper: Stopper,
    stdin: Option<Stdin>,
    stdout: Option<Chunks>,
    stderr: Option<Chunks>,
    hub: Hub,
}

impl Running {
    pub fn name(&self) -> &str {
        self.machine.name()
    }

    /// The machine, by name: to `exec` into it, see its screen, its ports and so on.
    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    /// Its hooks: to add to, observe or subscribe to while it runs.
    pub fn hooks(&self) -> &Hub {
        &self.hub
    }

    /// Its stdin, if it was [`Stdio::Piped`].
    pub fn take_stdin(&mut self) -> Option<Stdin> {
        self.stdin.take()
    }

    /// Its stdout, if it was [`Stdio::Piped`].
    pub fn take_stdout(&mut self) -> Option<Chunks> {
        self.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<Chunks> {
        self.stderr.take()
    }

    /// Asks it to end, as `docker stop` does: SIGTERM to a Linux machine's first process (a Windows machine is
    /// ended), and everything ended ten seconds later if it hasn't. [`Running::wait`] for it.
    pub fn stop(&self) {
        self.stopper.stop(15);
    }

    /// Ends everything in it now.
    pub fn kill(&self) {
        self.stopper.kill();
    }

    /// Whether its first process has ended.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Waits for its first process to end, and how it did.
    pub fn wait(mut self) -> Result<Exit> {
        let thread = self.thread.take().ok_or_else(|| Error::new(ErrorKind::Failed, "waited for already"))?;
        // Its streams go first, so it never waits on a pipe nobody reads.
        drop(self.stdin.take());
        match thread.join() {
            Ok(result) => result.map(Exit),
            Err(_) => Err(Error::new(ErrorKind::Failed, "the machine's thread panicked")),
        }
    }
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running").field("name", &self.name()).field("finished", &self.is_finished()).finish()
    }
}

/// Whether `name` is a host name Linux takes: dot-separated labels of letters, digits and `-`, none starting or
/// ending with `-`, each at most 63 long, 253 in all.
pub(crate) fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-') && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_displays() {
        assert_eq!(parse_size("512m"), Some(512 << 20));
        assert_eq!(parse_size("4G"), Some(4 << 30));
        assert_eq!(parse_size("2gib"), Some(2 << 30));
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("lots"), None);
        assert_eq!(Display::parse("headless=800x600"), Some(Display::Headless(800, 600, 120)));
        assert_eq!(Display::parse("headless=1440x900@2"), Some(Display::Headless(1440, 900, 240)));
        assert_eq!(Display::parse("none"), Some(Display::None));
        assert_eq!(Display::parse("host"), Some(Display::Host(1280, 800)));
        assert_eq!(Display::parse("host=1024x700"), Some(Display::Host(1024, 700)));
        assert_eq!(Display::parse("host=1024x700@2"), None);
        assert_eq!(Display::parse("host-windows"), Some(Display::HostWindows));
        assert_eq!(Display::parse("big"), None);
    }

    #[test]
    fn host_names_as_linux_takes_them() {
        for ok in ["sbx-8f3a", "a", "web.internal", "x1.y-2.z3"] {
            assert!(valid_hostname(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", "a..b", "a_b", "a b", &"x".repeat(64), &format!("{}.x", "a.".repeat(126))] {
            assert!(!valid_hostname(bad), "{bad}");
        }
    }
}
