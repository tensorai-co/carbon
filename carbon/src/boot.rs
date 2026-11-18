//! Running a machine: a kernel (lambda for a Linux image or an ELF, atlas for a Windows image or a PE) on exec with
//! the isolation asked for, over a disk that's its image's index plus its own writable layer, with a display, sound,
//! a network of its own and the host's services, and its hooks at every hook point. It runs on the thread that
//! asked (hvf ties a VM's CPUs to the thread that made them), until its first process ends.

use std::sync::Arc;

use exec::{Cpu, Exec, Isolation, Options as ExecOptions, Tuning};
use hooks::Hub;
use images::Platform;
use layers::ROOT;

use crate::error::{Error, ErrorKind, Result};
use crate::stdio::{Ends, MachineHost, Stdio};
use crate::stop::{Handle, Stopper};
use crate::store::{Kernel, Record, Source, Store, network_name};
use crate::wire::{self, Enter, ExecRequest, Served, Session};
use crate::{Audio, Display, Gpu, Network, Notice, Publish};

/// Who's told what a machine falls back to, warns of, or is doing.
pub(crate) type Notify = Arc<dyn Fn(Notice) + Send + Sync>;

/// Everything a run is asked for.
pub(crate) struct Launch {
    pub name: Option<String>,
    /// Its host name, as `uname` and its network give it (`--hostname`); None for what it had, or its name.
    pub hostname: Option<String>,
    /// Labels (`--label`), over what it had and its image's.
    pub labels: Vec<(String, String)>,
    /// Its `/etc/hosts`' extra lines (`--add-host`); None for what it had.
    pub extra_hosts: Option<Vec<crate::ExtraHost>>,
    /// What a new machine starts from; None starts the machine called `name` again.
    pub source: Option<Source>,
    /// The program and its arguments; empty for what the image says (its entrypoint and command).
    pub args: Vec<String>,
    pub isolation: Isolation,
    /// For an image that isn't here yet: which build to pull.
    pub platform: Option<Platform>,
    /// `NAME=value`, over the image's.
    pub env: Vec<String>,
    pub workdir: Option<String>,
    pub display: Display,
    pub gpu: Gpu,
    pub audio: Audio,
    pub memory: u64,
    /// None for what it had last, or NAT for a new machine.
    pub network: Option<Network>,
    /// Every port its image exposes published, each on a free host port (`-P`).
    pub publish_all: bool,
    /// As written (`2222:22`); None for what it had last.
    pub publish: Option<Vec<String>>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
    pub tty: bool,
    /// Take this process's SIGINT, SIGTERM and SIGHUP for the machine, as `carbon run` does.
    pub relay_signals: bool,
    /// Removed, with its files, once it ends (`--rm`).
    pub remove: bool,
    /// Stopped (as `stop` does) if it's still running this long after it starts.
    pub timeout: Option<std::time::Duration>,
    /// Its mounts; None for what it had last.
    pub mounts: Option<Vec<crate::Mount>>,
    /// Who its first program runs as (`name[:group]`, `uid[:gid]`); None for the image's `USER`, else root.
    pub user: Option<String>,
    pub hub: Hub,
}

/// What the caller gets once the machine is made and about to boot.
pub(crate) struct Started {
    pub name: String,
    pub stopper: Stopper,
    pub ends: Ends,
}

/// Whether this is the process's main thread, which AppKit must have.
#[cfg(target_os = "macos")]
fn on_main_thread() -> bool {
    // SAFETY: asks about the calling thread only.
    unsafe { libc::pthread_main_np() == 1 }
}

/// Runs a machine, new or again, until its first process ends, and gives its exit code. `started` is told once
/// it's made, before its kernel boots.
pub(crate) fn run(store: &Store, launch: Launch, notify: &Notify, started: &mut (dyn FnMut(Started) + Send)) -> Result<i32> {
    let existing = launch.name.as_deref().and_then(|name| store.record(name));
    let (mut record, program) = match (existing, &launch.source) {
        (Some(record), None) => (record, None),
        (Some(record), Some(_)) => return Err(Error::new(ErrorKind::AlreadyExists, format!("there's a machine called {} already: leave out what to run, to start it again", record.name))),
        (None, Some(source)) => store.create(launch.name.as_deref(), launch.platform.as_ref(), source, &|n| notify(n))?,
        (None, None) => return Err(Error::no_machine(launch.name.as_deref().unwrap_or_default())),
    };
    if record.running() {
        return Err(Error::new(ErrorKind::Running, format!("{} is running already", record.name)));
    }
    if matches!(launch.display, Display::Host(..) | Display::HostWindows) {
        #[cfg(target_os = "macos")]
        if !on_main_thread() {
            return Err(Error::unsupported("a machine on the host's screen needs the main thread, which AppKit takes: run it with `run` from main"));
        }
        #[cfg(not(target_os = "macos"))]
        return Err(Error::unsupported("--display host isn't there on this host yet: use headless=WxH"));
    }
    // What it's asked for this time, else what it had.
    if let Some(network) = launch.network {
        record.network = network;
    }
    if let Some(publish) = &launch.publish {
        record.publish = publish.clone();
    }
    if let Some(mounts) = &launch.mounts {
        record.mounts = mounts.clone();
    }
    if let Some(hostname) = &launch.hostname {
        record.hostname = Some(hostname.clone());
    }
    for (key, value) in &launch.labels {
        record.config.labels.insert(key.clone(), value.clone());
    }
    if let Some(hosts) = &launch.extra_hosts {
        record.extra_hosts = hosts.clone();
    }
    let mut ports = Vec::new();
    let exposed: Vec<String> = if launch.publish_all { record.config.exposed_ports.clone() } else { Vec::new() };
    for text in record.publish.iter().chain(&exposed) {
        ports.push((text.clone(), Publish::parse(text).map_err(|e| Error::invalid(format!("--publish {e}")))?));
    }
    if !ports.is_empty() && record.network != Network::Nat {
        notify(Notice::warning(format!("published ports are discarded with --network {}", network_name(record.network))));
    }
    launch.hub.set_machine(&record.name);

    // Signals are taken by a thread of their own, from here on: block them before any thread is made.
    #[cfg(unix)]
    let relay = launch.relay_signals.then(crate::signals::Relay::start);

    let (host, ends) = MachineHost::new(launch.stdin, launch.stdout, launch.stderr, launch.tty, launch.hub.clone());
    let host = Arc::new(host);
    let stopper = Stopper::default();
    let mut ends = Some(ends);
    if let Some(timeout) = launch.timeout {
        let stopper = stopper.clone();
        let _ = std::thread::Builder::new().name("carbon machine timeout".into()).spawn(move || {
            let (step, mut waited) = (std::time::Duration::from_millis(100), std::time::Duration::ZERO);
            while waited < timeout {
                if stopper.is_done() {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
            }
            stopper.stop(15);
        });
    }

    // x86 of either width on Apple silicon wants the CPU's own x86 memory ordering.
    let tso = matches!(record.platform.arch.as_str(), "amd64" | "386") && cfg!(target_arch = "aarch64");
    // Opens exec and boots the kernel on this thread, which its CPUs belong to.
    let mut start = |record: &mut Record, desktop: Option<display::desktop::Shared>| -> Result<i32> {
        let (exec, cpu) = open_exec(launch.isolation, tso, launch.memory, notify)?;
        // Its network, with its ports published on the host's before anything runs: a host port that's taken stops
        // it here, saying which. Its connections and lookups go through its hooks.
        let config = net::Config { hostname: record.host_name(), mode: record.network, ..net::Config::default() };
        let network = net::Network::new(config, Arc::new(crate::network::HookedNet::new(launch.hub.clone())));
        let mut published = Vec::new();
        for (text, ports) in &ports {
            published.push(network.publish(ports).map_err(|e| match e {
                net::Error::AddrInUse => Error::new(ErrorKind::Failed, format!("--publish {text}: the host's port is in use")),
                net::Error::AddrNotAvailable => Error::invalid(format!("--publish {text}: that address isn't this host's")),
                e => Error::new(ErrorKind::Failed, format!("--publish {text}: {e}")),
            })?);
        }
        record.ports = published.iter().flat_map(net::Published::bound).map(|b| b.to_string()).collect();
        record.backend = exec.backend().to_string();
        record.pid = Some(std::process::id());
        record.exit = None;
        store.save_record(record)?;
        // Its caller hears it's started once it can be reached from outside (or once it's over, if it never could be).
        let name = record.name.clone();
        let mut ready = || {
            if let Some(ends) = ends.take() {
                started(Started { name: name.clone(), stopper: stopper.clone(), ends });
            }
        };
        let parts = Parts { exec, cpu, program: program.clone(), host_desktop: desktop, network, host: host.clone(), stopper: stopper.clone() };
        #[cfg(unix)]
        let ended = boot(store, record, &launch, parts, notify, &mut ready, relay.as_ref());
        #[cfg(not(unix))]
        let ended = boot(store, record, &launch, parts, notify, &mut ready);
        ready();
        stopper.finished();
        host.close_output();
        drop(published);
        record.pid = None;
        record.ports.clear();
        record.exit = ended.as_ref().ok().copied();
        if launch.remove {
            let _ = std::fs::remove_dir_all(store.dir(&record.name));
        } else {
            let _ = store.save_record(record);
        }
        ended
    };
    match launch.display {
        // Windows on the host's screen: AppKit takes this, the main thread, and the kernel boots on another.
        Display::Host(..) | Display::HostWindows => {
            #[cfg(target_os = "macos")]
            {
                let (width, height) = match launch.display {
                    Display::Host(w, h) => (w, h),
                    _ => (1280, 800),
                };
                let desktop = display::Desktop::shared(width, height);
                let done = std::sync::atomic::AtomicBool::new(false);
                let title = format!("{} — carbon", record.name);
                let closer = stopper.clone();
                std::thread::scope(|scope| {
                    let kernel = scope.spawn(|| {
                        let ended = start(&mut record, Some(desktop.clone()));
                        done.store(true, std::sync::atomic::Ordering::Release);
                        ended
                    });
                    let shown = match launch.display {
                        Display::Host(..) => {
                            let options = display_macos::Options { title, width, height };
                            // Closing the window stops the machine, as Ctrl-C does.
                            let close = move || closer.stop(libc::SIGTERM);
                            display_macos::run(desktop.clone(), options, &done, &close)
                        }
                        _ => display_macos::run_windows(desktop.clone(), &done),
                    };
                    if let Err(e) = shown {
                        notify(Notice::warning(e.to_string()));
                    }
                    kernel.join().unwrap_or_else(|_| Err(Error::from("the machine's kernel thread panicked")))
                })
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = &mut start;
                Err(Error::unsupported("--display host isn't there on this host yet: use headless=WxH"))
            }
        }
        _ => start(&mut record, None),
    }
}

/// What `run` made for `boot`.
struct Parts {
    exec: Exec,
    cpu: Cpu,
    program: Option<Vec<u8>>,
    host_desktop: Option<display::desktop::Shared>,
    network: net::Network,
    host: Arc<MachineHost>,
    stopper: Stopper,
}

/// What the machine's hooks make of its first program: what to run, or an answer to give instead.
enum First {
    Run(String, Vec<String>, Vec<String>),
    Answered(hooks::Reply),
}

/// Asks the hooks about a machine's first program, found at `path`. `refind` finds a rewritten program by name.
fn hook_first(hub: &Hub, os: hooks::Os, path: String, args: Vec<String>, env: Vec<String>, cwd: Option<String>, refind: impl Fn(&str, &[String]) -> Option<String>) -> Result<First> {
    if !hub.wants(hooks::Kind::Exec) {
        return Ok(First::Run(path, args, env));
    }
    let event = hooks::Exec { os, origin: hooks::Origin::First, pid: None, parent: None, path: path.clone(), argv: args, env, cwd, found: true };
    match hub.exec(event.clone()) {
        hooks::Decision::Pass | hooks::Decision::Allow => Ok(First::Run(path, event.argv, event.env)),
        hooks::Decision::Deny => Err(Error::new(ErrorKind::Denied, format!("{}: the machine's hooks refused to start it", event.program()))),
        hooks::Decision::Answer(reply) => Ok(First::Answered(reply)),
        hooks::Decision::Rewrite(e) => {
            let path = if e.path != event.path { e.path.clone() } else { refind(e.argv.first().map_or("", String::as_str), &e.env).ok_or_else(|| Error::not_found(format!("{}: not found on the machine's disk", e.program())))? };
            Ok(First::Run(path, e.argv, e.env))
        }
    }
}

/// Starts what `exec` asks for through the machine's hooks: they may refuse it, change it, or answer it, and its
/// output goes through them too.
fn hooked_enter(enter: Enter, hub: Hub, os: hooks::Os) -> Enter {
    Arc::new(move |mut request: ExecRequest| {
        if hub.wants(hooks::Kind::Exec) {
            let event = hooks::Exec { os, origin: hooks::Origin::Outside, pid: None, parent: None, path: request.args.first().cloned().unwrap_or_default(), argv: request.args.clone(), env: request.env.clone(), cwd: request.workdir.clone(), found: true };
            match hub.exec(event.clone()) {
                hooks::Decision::Deny => return Err(format!("{}: refused by the machine's hooks", event.program())),
                hooks::Decision::Answer(reply) => return Ok(Box::new(wire::Answered::new(reply)) as Box<dyn Session>),
                hooks::Decision::Rewrite(e) => {
                    request.args = if e.path != event.path && !e.path.is_empty() { std::iter::once(e.path).chain(e.argv.into_iter().skip(1)).collect() } else { e.argv };
                    request.env = e.env;
                    request.workdir = e.cwd;
                }
                hooks::Decision::Pass | hooks::Decision::Allow => {}
            }
        }
        let session = enter(request)?;
        Ok(Box::new(wire::Hooked::new(session, hub.clone())) as Box<dyn Session>)
    })
}

#[allow(clippy::too_many_lines)]
fn boot(store: &Store, record: &Record, launch: &Launch, parts: Parts, notify: &Notify, ready: &mut dyn FnMut(), #[cfg(unix)] relay: Option<&crate::signals::Relay>) -> Result<i32> {
    let Parts { exec, cpu, program, host_desktop, network, host, stopper } = parts;
    let hub = launch.hub.clone();
    let mut env = record.config.env.clone();
    for var in &launch.env {
        let key = var.split('=').next().unwrap_or("");
        env.retain(|e| e.split('=').next() != Some(key));
        env.push(var.clone());
    }
    // A machine made from a program runs it, with the arguments given; an image's runs what's given, or else what the
    // image says.
    let args: Vec<String> = if let Some(program) = &record.program {
        std::iter::once(program.clone()).chain(launch.args.iter().cloned()).collect()
    } else if !launch.args.is_empty() {
        launch.args.clone()
    } else {
        record.config.entrypoint.iter().chain(&record.config.cmd).cloned().collect()
    };
    if args.is_empty() {
        return Err(Error::invalid(format!("{} says nothing to run: name a program to run in it", record.image.as_deref().unwrap_or(&record.name))));
    }
    let workdir = launch.workdir.clone().or_else(|| Some(record.config.workdir.clone()).filter(|w| !w.is_empty()));
    // A desktop for its windows: a screen in memory, the host's screen, or none.
    let desktop = match launch.display {
        Display::Headless(width, height, scale) => {
            let desktop = display::Desktop::shared(width, height);
            display::desktop::lock(&desktop).set_output(display::output::Output::headless(width, height, scale));
            Some(desktop)
        }
        Display::Host(..) | Display::HostWindows => host_desktop.clone(),
        Display::None => None,
    };
    // A headless desktop's frames come from a clock of its own, ticking only while a window waits for one; on the
    // host's screen, its refresh is the clock.
    let headless = matches!(launch.display, Display::Headless(..));
    let _clock = desktop.clone().filter(|_| headless).map(|d| display::clock::Clock::start(d, 60));
    let as_host: Arc<dyn host::Host> = host.clone();
    let answered = |reply: hooks::Reply| {
        host.write_unhooked(host::Stream::Out, &reply.stdout);
        host.write_unhooked(host::Stream::Err, &reply.stderr);
        Ok(reply.code)
    };
    match record.kernel {
        Kernel::Lambda => {
            let mut disk = store.disk::<lambda::Meta>(record, lambda::Meta::dir(0o755), false)?;
            if let (Some(bytes), Some(path)) = (&program, &record.program) {
                disk.put(path, layers::New::File, bytes, lambda::Meta::file(0o755), lambda::Meta::dir(0o755)).map_err(|e| Error::io(format!("putting {path} on the disk: {e}")))?;
            }
            // An image's own environment takes the place of lambda's, and few set HOME: it's the image user's home
            // from its passwd file, as a container runtime makes it.
            let mut env = if env.is_empty() { lambda::Options::default().env } else { env };
            // Where its display and sound are is the machine's to say: an image's own `DISPLAY=:99` or
            // `XDG_RUNTIME_DIR` names a server that isn't there, so it goes, and lambda puts its own. One set with `-e`
            // stays, as asked. (It always has sound: the mixer below.)
            let display = desktop.is_some();
            env.retain(|e| {
                let key = e.split('=').next().unwrap_or("");
                !lambda::owns_env(key, display, true) || launch.env.iter().any(|v| v.split('=').next() == Some(key))
            });
            // A console is a terminal programs can draw on: `xterm`, as Docker says with `-t`.
            if host::Host::is_terminal(&*host) && !env.iter().any(|e| e.starts_with("TERM=")) {
                env.push("TERM=xterm".into());
            }
            // Who it runs as: as asked, else the image's USER, else root; and that user's home.
            let user_spec = launch.user.clone().unwrap_or_else(|| record.config.user.clone());
            let who = crate::users::from_disk(&disk, &user_spec)?;
            if !env.iter().any(|e| e.starts_with("HOME=")) {
                env.push(format!("HOME={}", who.home));
            }
            let path = find_program(&disk, &args[0], &env).ok_or_else(|| Error::not_found(format!("{}: not found on the machine's disk", args[0])))?;
            let (path, args, env) = match hook_first(&hub, hooks::Os::Linux, path, args, env, workdir.clone(), |name, env| find_program(&disk, name, env))? {
                First::Run(path, args, env) => (path, args, env),
                First::Answered(reply) => return answered(reply),
            };
            // `argv[0]` as it was asked for (`sh`, not `/usr/bin/sh`), as Docker runs it; the path found is what runs.
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            let program = Some(path.clone());
            let stop = lambda::Stop::default();
            stopper.set(Handle::Lambda(stop.clone()));
            #[cfg(unix)]
            if let Some(relay) = relay {
                let stopper = stopper.clone();
                relay.on_signal(move |signal, first| if first { stopper.stop(signal) } else { stopper.kill() });
            }
            // `exec`: a process started in it from outside, with its environment and working directory unless the
            // request says otherwise, as Docker's are.
            let enter: Enter = {
                let (stop, base) = (stop.clone(), env.iter().filter(|e| !e.starts_with("TERM=")).cloned().collect::<Vec<_>>());
                let workdir = workdir.clone();
                Arc::new(move |request: ExecRequest| {
                    let mut env = base.clone();
                    for var in &request.env {
                        let key = var.split('=').next().unwrap_or("");
                        env.retain(|e| e.split('=').next() != Some(key));
                        env.push(var.clone());
                    }
                    if request.tty.is_some() && !env.iter().any(|e| e.starts_with("TERM=")) {
                        env.push("TERM=xterm".into());
                    }
                    // Another user, as asked: read from the machine's own files as they are now.
                    let user = match request.user.as_deref() {
                        Some(spec) => match stop.with_disk(|disk| crate::users::from_disk(disk, spec)) {
                            Some(Ok(who)) => {
                                // Its own home, unless the request says otherwise, as Docker's exec gives it.
                                if !request.env.iter().any(|e| e.starts_with("HOME=")) {
                                    env.retain(|e| !e.starts_with("HOME="));
                                    env.push(format!("HOME={}", who.home));
                                }
                                Some(who.user)
                            }
                            Some(Err(e)) => return Err(e.to_string()),
                            None => return Err("the machine's kernel isn't up".to_string()),
                        },
                        None => None,
                    };
                    let spec = lambda::Spec { argv: request.args, env, cwd: request.workdir.or_else(|| workdir.clone()), tty: request.tty, user };
                    stop.enter(spec).map(|entered| Box::new(entered) as Box<dyn Session>)
                })
            };
            let files: wire::FilesFn = {
                let stop = stop.clone();
                // As its programs see paths, through its mounts: a file written into a tmpfs is in the tmpfs.
                Arc::new(move |op: &crate::files::Op, data: &[u8]| {
                    stop.with_files(|disk, paths| crate::files::apply_with(disk, &crate::files::Mounted(paths), op, data))
                        .unwrap_or_else(|| Err(Error::new(ErrorKind::NotRunning, "the machine's kernel isn't up")))
                })
            };
            let served = Served { desktop: desktop.clone(), enter: Some(hooked_enter(enter, hub.clone(), hooks::Os::Linux)), files: Some(files), stopper: stopper.clone(), hub: hub.clone() };
            let _control = wire::serve(store.dir(&record.name), served);
            ready();
            let gpu = open_gpu(launch.gpu, notify)?;
            let (mixer, _audio) = open_audio(&launch.audio, launch.display, notify)?;
            // Its mounts: host folders and volumes attached to its disk, and tmpfs.
            let mounts = crate::mounts::lambda_mounts(store.images.home(), &record.mounts, &disk, (who.user.uid, who.user.gid))?;
            let mut options = lambda::Options { stop, display: desktop.clone(), gpu, audio: Some(mixer), env, network: Some(network), hooks: hub, program, mounts, user: Some(who.user), ..lambda::Options::default() };
            options.hostname = record.host_name();
            options.extra_hosts = record.extra_hosts.iter().map(|h| (h.name.clone(), h.ip)).collect();
            options.console = launch.tty;
            options.cwd = workdir.clone();
            lambda::boot(exec, cpu, disk, as_host, &argv, options).map(lambda::Status::code).map_err(Error::from)
        }
        Kernel::Atlas => {
            if !record.extra_hosts.is_empty() {
                return Err(Error::unsupported("--add-host in a Windows machine isn't there yet: Linux machines have it"));
            }
            if !record.mounts.is_empty() {
                return Err(Error::unsupported("mounts in a Windows machine aren't there yet: Linux machines have them"));
            }
            if launch.user.as_deref().is_some_and(|u| !u.is_empty()) {
                return Err(Error::unsupported("--user in a Windows machine isn't there yet: Linux machines have it"));
            }
            let mut disk = store.disk::<atlas::Meta>(record, atlas::Meta { attributes: 0x10 }, true)?;
            if let (Some(bytes), Some(path)) = (&program, &record.program) {
                let unix = format!("/{}", path.trim_start_matches("C:\\").replace('\\', "/"));
                disk.put(&unix, layers::New::File, bytes, atlas::Meta::default(), atlas::Meta { attributes: 0x10 }).map_err(|e| Error::io(format!("putting {path} on the disk: {e}")))?;
            }
            // A program named without a path is found where Windows keeps its own (an image's, as atlas-base's), or at
            // the drive's root.
            let path = atlas::find_program(&disk, &args[0]).ok_or_else(|| {
                Error::not_found(format!(
                    "{}: not on the machine's disk. Windows' programs (cmd, powershell, where, ...) come from atlas-base: build it (github.com/tensorai-co/atlas-base), `carbon import` it, and run a machine from that image",
                    args[0]
                ))
            })?;
            let (path, args, env) = match hook_first(&hub, hooks::Os::Windows, path, args, env, workdir.clone(), |name, _| atlas::find_program(&disk, name))? {
                First::Run(path, args, env) => (path, args, env),
                First::Answered(reply) => return answered(reply),
            };
            let stop = atlas::Stop::default();
            stopper.set(Handle::Atlas(stop.clone()));
            #[cfg(unix)]
            if let Some(relay) = relay {
                let stopper = stopper.clone();
                relay.on_signal(move |signal, first| if first { stopper.stop(signal) } else { stopper.kill() });
            }
            // `exec`: a process started in it from outside, with a console of its own.
            let enter: Enter = {
                let stop = stop.clone();
                let workdir = workdir.clone();
                Arc::new(move |request: ExecRequest| {
                    if request.user.as_deref().is_some_and(|u| !u.is_empty()) {
                        return Err("--user in a Windows machine isn't there yet: Linux machines have it".to_string());
                    }
                    let spec = atlas::Spec { argv: request.args, env: request.env, cwd: request.workdir.or_else(|| workdir.clone()), tty: request.tty };
                    stop.enter(spec).map(|entered| Box::new(entered) as Box<dyn Session>)
                })
            };
            let files: wire::FilesFn = {
                let stop = stop.clone();
                Arc::new(move |op: &crate::files::Op, data: &[u8]| stop.with_disk(|disk| crate::files::apply(disk, op, data)).unwrap_or_else(|| Err(Error::new(ErrorKind::NotRunning, "the machine's kernel isn't up"))))
            };
            let served = Served { desktop: desktop.clone(), enter: Some(hooked_enter(enter, hub.clone(), hooks::Os::Windows)), files: Some(files), stopper: stopper.clone(), hub: hub.clone() };
            let _control = wire::serve(store.dir(&record.name), served);
            ready();
            let env = env.iter().filter_map(|e| e.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let gpu = open_gpu(launch.gpu, notify)?;
            let pipes = !host::Host::is_terminal(&*host);
            let options = atlas::Options { stop, env, display: desktop, trace: std::env::var_os("CARBON_TRACE").is_some(), gpu, network: Some(network), pipes, hooks: hub };
            let argv: Vec<&str> = std::iter::once(path.as_str()).chain(args[1..].iter().map(String::as_str)).collect();
            atlas::boot(exec, cpu, disk, as_host, &argv, options).map(|ended| ended.code as i32).map_err(Error::from)
        }
    }
}

/// The machine's mixer, and what plays it for as long as it's kept: the host's speakers, a WAV file being written, or
/// a thread taking it at the pace it would play.
fn open_audio(choice: &Audio, display: Display, notify: &Notify) -> Result<(audio::Mixer, Box<dyn std::any::Any>)> {
    let mixer = audio::Mixer::new();
    let host = match choice {
        Audio::Auto => matches!(display, Display::Host(..) | Display::HostWindows),
        Audio::Host => true,
        Audio::Wav(path) => return Ok((mixer.clone(), Box::new(audio::output::Timed::wav(mixer, path)?))),
        Audio::None => false,
    };
    if host {
        #[cfg(target_os = "macos")]
        match audio_macos::Speakers::start(mixer.clone()) {
            Ok(speakers) => return Ok((mixer, Box::new(speakers))),
            Err(e) if *choice == Audio::Host => return Err(Error::from(e)),
            Err(e) => notify(Notice::warning(format!("no sound: {e}"))),
        }
        #[cfg(not(target_os = "macos"))]
        if *choice == Audio::Host {
            return Err(Error::unsupported("--audio host isn't there on this host yet: use none or a .wav file"));
        }
    }
    let _ = notify;
    Ok((mixer.clone(), Box::new(audio::output::Timed::none(mixer))))
}

/// Opens the machine's GPU on its own thread: the device asked for, or none. `auto` takes the host's GPU where there
/// is one, and says so, as `--isolation auto` does, when it has to take the CPU device instead.
fn open_gpu(choice: Gpu, notify: &Notify) -> Result<Option<gpu_api::Gpu>> {
    let gpu = gpu_api::Gpu::open(choice).map_err(|e| Error::new(ErrorKind::Failed, format!("--gpu: {}", e.message)))?;
    if let Some(why) = gpu.as_ref().and_then(|g| g.info().why.as_ref()) {
        notify(Notice::fallback(format!("no GPU here to use ({why}), the machine's GPU is the CPU device")));
    }
    Ok(gpu)
}

/// Opens exec with the isolation asked for. `auto` takes a hardware VM where there is one, and says so, loudly, with
/// why, when it falls back to the sandbox.
fn open_exec(isolation: Isolation, tso: bool, memory: u64, notify: &Notify) -> Result<(Exec, Cpu)> {
    let options = |isolation| ExecOptions { memory, isolation, tso, tuning: Tuning::default() };
    let failed = |e: exec::OpenError| Error::new(ErrorKind::Failed, e.to_string());
    match isolation {
        Isolation::Auto => match Exec::open(options(Isolation::Vm)) {
            Ok(opened) => Ok(opened),
            Err(vm) => {
                let opened = Exec::open(options(Isolation::Sandbox)).map_err(failed)?;
                let why = vm.tried.first().map_or_else(|| vm.to_string(), |(name, why)| format!("{name}: {why}"));
                notify(Notice::fallback(format!("no hypervisor here ({why}), running with --isolation sandbox")));
                Ok(opened)
            }
        },
        other => Exec::open(options(other)).map_err(failed),
    }
}

/// A program by name, as a shell finds it: itself if it has a `/`, else the first in the machine's `PATH`.
fn find_program(disk: &lambda::Disk, program: &str, env: &[String]) -> Option<String> {
    if program.contains('/') {
        return Some(program.to_string());
    }
    let path = env.iter().find_map(|e| e.strip_prefix("PATH=")).unwrap_or("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    path.split(':').map(|dir| format!("{}/{program}", dir.trim_end_matches('/'))).find(|candidate| {
        disk.resolve(ROOT, candidate, true).and_then(|ino| disk.stat(ino)).is_ok_and(|stat| stat.kind == layers::Kind::File)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_found_on_the_path() {
        let mut disk = lambda::Disk::new(lambda::Meta::dir(0o755), false);
        disk.put("/usr/bin/bash", layers::New::File, b"\x7fELF", lambda::Meta::file(0o755), lambda::Meta::dir(0o755)).unwrap();
        let env = vec!["PATH=/usr/local/bin:/usr/bin".to_string()];
        assert_eq!(find_program(&disk, "bash", &env).as_deref(), Some("/usr/bin/bash"));
        assert_eq!(find_program(&disk, "zsh", &env), None);
        assert_eq!(find_program(&disk, "/bin/sh", &env).as_deref(), Some("/bin/sh"));
    }

    #[test]
    fn a_first_program_is_hooked() {
        let hub = Hub::new();
        hub.on_exec(hooks::Filter::program("sh"), |e| hooks::Decision::rewrite(e.clone().with_argv(["bash", "-c", "true"])));
        hub.on_exec(hooks::Filter::program("rm"), |_| hooks::Decision::deny());
        let refind = |name: &str, _: &[String]| Some(format!("/usr/bin/{name}"));
        match hook_first(&hub, hooks::Os::Linux, "/bin/sh".into(), vec!["sh".into()], vec![], None, refind).unwrap() {
            First::Run(path, args, _) => assert_eq!((path.as_str(), args.len()), ("/usr/bin/bash", 3)),
            First::Answered(_) => panic!("answered"),
        }
        let denied = hook_first(&hub, hooks::Os::Linux, "/bin/rm".into(), vec!["rm".into()], vec![], None, refind);
        assert_eq!(denied.err().map(|e| e.kind()), Some(ErrorKind::Denied));
    }
}
