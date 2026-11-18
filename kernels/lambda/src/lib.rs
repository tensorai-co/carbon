//! lambda: Carbon's Linux kernel. It runs Linux programs on `exec`, from an image's own userland: the image brings
//! glibc or musl, the dynamic loader and everything above them. What lambda brings is what a Linux kernel provides:
//! system calls, processes and threads, memory, files over `layers`, and the virtual files in `/dev` and `/proc`.
//!
//! Every guest thread is a host thread that owns its exec CPU (`kernels_core::thread`). A system call is handled on
//! that thread, taking exec's lock only for short turns, so a thread that blocks never holds up another.
//!
//! ```no_run
//! # use std::sync::Arc;
//! let (exec, cpu) = exec::Exec::open(exec::Options::default())?;
//! let mut disk = lambda::Disk::new(lambda::Meta::dir(0o755), false);
//! disk.put("/bin/hello", layers::New::File, &std::fs::read("hello")?, lambda::Meta::file(0o755), lambda::Meta::dir(0o755))?;
//! let status = lambda::boot(exec, cpu, disk, Arc::new(host::Std::new()), &["/bin/hello"], lambda::Options::default())?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod abi;
mod admin;
mod fs;
mod enter;
mod ipc;
mod mm;
mod net;
mod ns;
mod os;
mod process;
mod security;
mod signal;
mod sys;
mod time;
mod timer;
mod tty;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};

use exec::{Cpu, Exec};
use host::Host;
use kernels_core::cache::PageCache;
use kernels_core::lock;
use kernels_core::thread::{Cx, SharedExec};
use kernels_core::wait::Futexes;

pub use enter::{Entered, Event, Spec};
use process::Process;
use tty::Tty;

/// What a file means to lambda: its mode, owner, and for a device, its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Meta {
    /// Permission bits, and the type bits too for what `layers` has no kind for (a device).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// A device's major and minor numbers, as `makedev` packs them.
    pub rdev: u64,
}

impl Meta {
    pub fn file(mode: u32) -> Meta {
        Meta { mode, ..Meta::default() }
    }

    pub fn dir(mode: u32) -> Meta {
        Meta { mode, ..Meta::default() }
    }

    /// A character device.
    pub fn device(major: u64, minor: u64) -> Meta {
        Meta { mode: abi::S_IFCHR | 0o666, rdev: major << 8 | minor, ..Meta::default() }
    }
}

/// On disk: the mode, owner and group, and the device number, which `device` packs into 32 bits.
impl layers::Persist for Meta {
    fn save(&self) -> [u32; 4] {
        [self.mode, self.uid, self.gid, self.rdev as u32]
    }
    fn load(words: [u32; 4]) -> Meta {
        Meta { mode: words[0], uid: words[1], gid: words[2], rdev: u64::from(words[3]) }
    }
}

impl From<layers::tar::Entry> for Meta {
    fn from(e: layers::tar::Entry) -> Meta {
        Meta { mode: e.mode, uid: e.uid, gid: e.gid, rdev: e.rdev }
    }
}

/// A machine's disk, as lambda sees it.
pub type Disk = layers::Fs<Meta>;

#[derive(Debug, Clone)]
pub struct Options {
    pub hostname: String,
    /// The first program's environment.
    pub env: Vec<String>,
    /// The first program's working directory (an image's `WORKDIR`, or `--workdir`): `/` if None.
    pub cwd: Option<String>,
    /// Report each system call and its result on stderr.
    pub trace: bool,
    /// Make the host's terminal the machine's console, if stdin and stdout are one: in raw mode, with lambda's own
    /// line discipline, so ^C, ^Z and full-screen programs work as on Linux.
    pub console: bool,
    /// The way in for stopping the machine from outside it.
    pub stop: Stop,
    /// The machine's network; if None, one of its own on the host's sockets, named `hostname`.
    pub network: Option<::net::Network>,
    /// The desktop its windows go on, if it has one: the kernel answers `$XDG_RUNTIME_DIR/wayland-0` with a Wayland
    /// compositor on it, and the first program's environment says where (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`).
    pub display: Option<display::desktop::Shared>,
    /// The machine's GPU, if it has one: the render node `/dev/dri/renderD128` reaches it, and the system layer puts
    /// lambda's Vulkan driver where the image's Vulkan loader looks.
    pub gpu: Option<gpu_api::Gpu>,
    /// The machine's mixer, if it has sound: the kernel answers `$XDG_RUNTIME_DIR/pulse/native` with a PulseAudio
    /// server playing into it.
    pub audio: Option<audio::Mixer>,
    /// The machine's hooks: each `execve` is asked about, when something wants execs.
    pub hooks: hooks::Hub,
    /// Folders mounted when it boots, in order: a host's folders and volumes, and tmpfs.
    pub mounts: Vec<Mount>,
    /// Who the first program runs as: root if None.
    pub user: Option<User>,
    /// The first program's path on the disk, when `argv[0]` is the name it was called by (`sh`, as an image's `CMD`
    /// says it) rather than where it is; None runs `argv[0]` as a path.
    pub program: Option<String>,
}

/// A running machine, from outside it: how `carbon stop` reaches in, from any thread.
#[derive(Clone, Default)]
pub struct Stop(Arc<Mutex<Option<Weak<Kernel>>>>);

impl std::fmt::Debug for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stop")
    }
}

impl Stop {
    fn kernel(&self) -> Option<Arc<Kernel>> {
        lock(&self.0).as_ref().and_then(Weak::upgrade)
    }

    /// Sends `signal` to process 1, as `kill 1` inside would. Whether the machine was running.
    pub fn signal(&self, signal: i32) -> bool {
        let Some(k) = self.kernel() else { return false };
        let Some(init) = k.process(1) else { return false };
        k.signal_process(&init, signal);
        true
    }

    /// Runs `f` on the machine's disk, under the lock its programs' own calls take: what it writes is what they see
    /// next, and a watching program (inotify, `ReadDirectoryChangesW`) hears of it. None if the kernel isn't up.
    pub fn with_disk<R>(&self, f: impl FnOnce(&mut Disk) -> R) -> Option<R> {
        let k = self.kernel()?;
        let mut disk = lock(&k.disk);
        Some(f(&mut disk))
    }

    /// Whether the machine's kernel is up, and hasn't ended: a process can be started in it from outside.
    pub fn is_running(&self) -> bool {
        self.kernel().is_some_and(|k| !k.done.load(Ordering::Acquire))
    }

    /// Ends every process at once, as SIGKILL would.
    pub fn end(&self) {
        if let Some(k) = self.kernel() {
            k.end_everything();
        }
    }
}

impl Default for Options {
    fn default() -> Options {
        Options {
            hostname: "carbon".to_string(),
            env: vec!["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(), "HOME=/root".to_string()],
            cwd: None,
            trace: std::env::var_os("CARBON_TRACE").is_some(),
            console: true,
            stop: Stop::default(),
            network: None,
            display: None,
            gpu: None,
            audio: None,
            hooks: hooks::Hub::default(),
            program: None,
            mounts: Vec::new(),
            user: None,
        }
    }
}

/// Who a process runs as: its user, group and supplementary groups (from the image's `/etc/passwd` and `/etc/group`,
/// which the caller reads). Anyone but root starts with no capabilities, as Linux's rules for leaving root give.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
}

/// A folder from elsewhere for a mount (a host's, a volume's): taken by the machine that boots with it.
#[derive(Clone)]
pub struct Folder(Arc<Mutex<Option<Box<dyn layers::External>>>>);

impl Folder {
    pub fn new(folder: Box<dyn layers::External>) -> Folder {
        Folder(Arc::new(Mutex::new(Some(folder))))
    }
}

impl std::fmt::Debug for Folder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Folder")
    }
}

/// What a mount has.
#[derive(Debug, Clone)]
pub enum MountSource {
    /// A folder from elsewhere, and what `mountinfo` names as its source (the host's path, a volume's name).
    Folder(Folder, String),
    /// One file of a folder from elsewhere, by its name in it: a single file bound over a file, as Linux allows.
    File(Folder, String, String),
    /// Memory only, gone when the machine is.
    Tmpfs,
}

/// A mount a machine boots with.
#[derive(Debug, Clone)]
pub struct Mount {
    /// Where it is in the machine: `/work`.
    pub target: String,
    pub source: MountSource,
    pub read_only: bool,
}

/// A folder's files' `Meta`: as it keeps it (a volume), else from the host's permission bits, owned as its tree says
/// (the machine's user, for a bound folder: so whoever the machine runs as can use it, as on Docker Desktop).
fn host_meta(st: &layers::ExtStat) -> Meta {
    if let Some(words) = st.meta {
        return <Meta as layers::Persist>::load(words);
    }
    let kind = match st.kind {
        layers::Kind::Dir => abi::S_IFDIR,
        layers::Kind::Symlink => abi::S_IFLNK,
        layers::Kind::File => abi::S_IFREG,
    };
    Meta { mode: kind | st.mode & 0o7777, uid: st.uid, gid: st.gid, ..Meta::default() }
}

/// Attaches the folders a machine boots with, and makes tmpfs trees, each with its folder made on the disk.
fn boot_mounts(disk: &mut Disk, mounts: &[Mount]) -> Result<Vec<fs::mount::BootMount>, String> {
    let mut out = Vec::new();
    for m in mounts {
        if !m.target.starts_with('/') || m.target.split('/').any(|p| p == "..") {
            return Err(format!("a mount's target must be an absolute path: {}", m.target));
        }
        // What it covers: a folder, or for a single file, a file.
        let point = if matches!(m.source, MountSource::File(..)) { layers::New::File } else { layers::New::Dir };
        disk.put(&m.target, point, &[], Meta::file(0o644), Meta::dir(0o755)).map_err(|e| format!("making {} for its mount: {e}", m.target))?;
        let map = || layers::Mapping { meta: host_meta, mode: |m: &Meta| m.mode & 0o7777, save: <Meta as layers::Persist>::save };
        let (root, what, source, tree) = match &m.source {
            MountSource::Folder(folder, source) => {
                let backend = lock(&folder.0).take().ok_or_else(|| format!("{}: its folder was taken by another machine", m.target))?;
                let root = disk.attach(backend, map(), m.read_only).map_err(|e| format!("{source}: {e}"))?;
                (root, fs::mount::What::Host, source.clone(), None)
            }
            MountSource::File(folder, name, source) => {
                let backend = lock(&folder.0).take().ok_or_else(|| format!("{}: its folder was taken by another machine", m.target))?;
                let top = disk.attach(backend, map(), m.read_only).map_err(|e| format!("{source}: {e}"))?;
                let root = disk.lookup(top, name).map_err(|e| format!("{source}: {e}"))?;
                (root, fs::mount::What::Host, source.clone(), None)
            }
            MountSource::Tmpfs => {
                let tree = disk.volatile_root(Meta::dir(0o755), 0);
                (tree, fs::mount::What::Tmpfs, "tmpfs".to_string(), Some(tree))
            }
        };
        out.push(fs::mount::BootMount { path: m.target.clone(), root, what, source, read_only: m.read_only, tree });
    }
    Ok(out)
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Exited(i32),
    Killed(i32),
}

impl Status {
    /// As `wait4` reports it.
    pub(crate) fn wait_status(self) -> i32 {
        match self {
            Status::Exited(code) => (code & 0xff) << 8,
            Status::Killed(signal) => signal & 0x7f,
        }
    }

    /// As a shell reports it: the code, or 128 and the signal.
    pub fn code(self) -> i32 {
        match self {
            Status::Exited(code) => code & 0xff,
            Status::Killed(signal) => 128 + signal,
        }
    }
}

/// Boots a machine: runs `options.program` (else `argv[0]`, a path on `disk`) as process 1, on this thread with the CPU `Exec::open` gave, and
/// returns how it ended. Other processes still running then are ended with it.
pub fn boot(mut exec: Exec, cpu: Cpu, disk: Disk, host: Arc<dyn Host>, argv: &[&str], options: Options) -> Result<Status, String> {
    let mut disk = disk;
    let mut options = options;
    let (display, audio) = (options.display.is_some(), options.audio.is_some());
    own_env(display, audio, &mut options.env);
    os::populate(&mut disk, options.gpu.is_some());
    let extra = boot_mounts(&mut disk, &options.mounts)?;
    let mount_ns = fs::mount::MountNs::new(fs::mount::Mounts::boot(&disk).with(&disk, extra));
    let boot_ns = ns::Namespaces::boot(mount_ns, admin::Names { host: options.hostname.clone(), domain: "(none)".to_string() });
    let watches = fs::inotify::Registry::default();
    let fanotify = fs::fanotify::Registry::default();
    disk.observe(Some(fs::fanotify::observer(watches.clone(), fanotify.clone())));
    let console = (options.console && host.is_terminal()).then(|| Tty::console(host.clone()));
    let klog = admin::Klog::boot(host.cpus(), exec.memory().size().limit);
    let net = options.network.clone().unwrap_or_else(|| ::net::Network::on_this_host(::net::Config { hostname: options.hostname.clone(), dns: None, ..Default::default() }));
    let exec = Arc::new(Mutex::new(exec));
    let kernel = Arc::new(Kernel {
        console,
        ptys: Mutex::new(BTreeMap::new()),
        next_pty: AtomicU32::new(0),
        done: AtomicBool::new(false),
        on_done: Mutex::new(Vec::new()),
        clock: os::vdso::Clock::new(mm::shared::SharedMem::new(exec.clone(), exec::PAGE, 0)),
        exec,
        host,
        disk: Mutex::new(disk),
        cache: Mutex::new(PageCache::default()),
        futexes: Futexes::default(),
        processes: Mutex::new(BTreeMap::new()),
        next_id: AtomicI32::new(1),
        init: (Mutex::new(None), Condvar::new()),
        options,
        stdin_watchers: Arc::default(),
        net,
        timers: Default::default(),
        watches,
        fanotify,
        fifos: Mutex::new(BTreeMap::new()),
        boot_ns,
        klog: Mutex::new(klog),
        system: Mutex::new(BTreeMap::new()),
        entered: Mutex::new(BTreeMap::new()),
    });
    os::vdso::start(&kernel);
    if let Some(desktop) = kernel.options.display.clone() {
        if let Err(e) = os::wayland::start(&kernel, desktop.clone()) {
            kernel.host.write(host::Stream::Err, format!("lambda: the Wayland server: {e}\n").as_bytes());
        }
        if let Err(e) = os::x11::start(&kernel, desktop) {
            kernel.host.write(host::Stream::Err, format!("lambda: the X server: {e}\n").as_bytes());
        }
    }
    if let Some(mixer) = kernel.options.audio.clone()
        && let Err(e) = os::pulse::start(&kernel, mixer)
    {
        kernel.host.write(host::Stream::Err, format!("lambda: the sound server: {e}\n").as_bytes());
    }
    let watchers = kernel.stdin_watchers.clone();
    kernel.host.watch_input(Arc::new(move || watchers.notify()));
    *lock(&kernel.options.stop.0) = Some(Arc::downgrade(&kernel));
    if kernel.console.is_some() {
        kernel.host.set_raw(true);
        let k = kernel.clone();
        let _ = std::thread::Builder::new().name("lambda console".into()).spawn(move || feed_console(&k));
    }
    let result = run_init(&kernel, cpu, argv);
    kernel.done.store(true, Ordering::Release);
    for wake in lock(&kernel.on_done).drain(..) {
        wake();
    }
    kernel.timers.stop();
    if kernel.console.is_some() {
        kernel.host.set_raw(false);
    }
    // Whatever the machine wrote is kept, if its disk has somewhere to keep it.
    if let Err(e) = lock(&kernel.disk).sync() {
        kernel.host.write(host::Stream::Err, format!("lambda: saving the disk: {e}\n").as_bytes());
    }
    kernels_core::thread::let_go(&kernel);
    result
}

/// Whether `key` is one of the variables that say where the machine's own servers are, which the kernel puts in every
/// program's environment: a machine with a desktop has `DISPLAY`, `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`; one with
/// sound has `XDG_RUNTIME_DIR` and `PULSE_SERVER`. Whoever puts a machine together drops an image's own values for
/// these (they name servers that aren't there), so the machine's are the ones its programs see.
pub fn owns_env(key: &str, display: bool, audio: bool) -> bool {
    match key {
        "DISPLAY" | "WAYLAND_DISPLAY" => display,
        "PULSE_SERVER" => audio,
        "XDG_RUNTIME_DIR" => display || audio,
        _ => false,
    }
}

/// Puts where the machine's display and sound servers are in `env`, wherever it doesn't say (unset, or set empty,
/// which X and Wayland's libraries take as unset): `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0` and
/// `XDG_RUNTIME_DIR=/run/user/0` with a desktop, `PULSE_SERVER` with sound. Done for process 1, for every `carbon
/// exec`, and at every `execve`, so a program started through `env -i`, `su -`, `sudo` or an ssh session finds the
/// display as its parent did. A value a program sets itself is kept: a nested X server's `DISPLAY=:1` still works. If
/// `XDG_RUNTIME_DIR` names somewhere else, `WAYLAND_DISPLAY` is the socket's whole path, which libwayland takes as is.
fn own_env(display: bool, audio: bool, env: &mut Vec<String>) {
    fn get<'e>(env: &'e [String], key: &str) -> Option<&'e str> {
        env.iter().rev().find_map(|e| e.strip_prefix(key)?.strip_prefix('=')).filter(|v| !v.is_empty())
    }
    fn set(env: &mut Vec<String>, key: &str, value: &str) {
        env.retain(|e| e.split('=').next() != Some(key));
        env.push(format!("{key}={value}"));
    }
    let runtime = get(env, "XDG_RUNTIME_DIR").map(|r| r.trim_end_matches('/').to_string());
    if (display || audio) && runtime.is_none() {
        set(env, "XDG_RUNTIME_DIR", os::wayland::RUNTIME_DIR);
    }
    if display {
        if get(env, "WAYLAND_DISPLAY").is_none() {
            let wayland = match runtime.as_deref() {
                None | Some(os::wayland::RUNTIME_DIR) => os::wayland::DISPLAY.to_string(),
                Some(_) => format!("{}/{}", os::wayland::RUNTIME_DIR, os::wayland::DISPLAY),
            };
            set(env, "WAYLAND_DISPLAY", &wayland);
        }
        if get(env, "DISPLAY").is_none() {
            set(env, "DISPLAY", os::x11::DISPLAY);
        }
    }
    if audio && get(env, "PULSE_SERVER").is_none() {
        set(env, "PULSE_SERVER", &format!("unix:{}", os::pulse::SOCKET));
    }
}

impl Kernel {
    /// [`own_env`], for this machine's servers.
    pub(crate) fn own_env(&self, env: &mut Vec<String>) {
        own_env(self.options.display.is_some(), self.options.audio.is_some(), env);
    }
}

fn run_init(kernel: &Arc<Kernel>, cpu: Cpu, argv: &[&str]) -> Result<Status, String> {
    let exec = kernel.exec.clone();
    let mut cx = Cx::new(&exec, cpu);
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let env = kernel.options.env.clone();
    let mut task = process::start_init(kernel, &mut cx, &argv, &env).map_err(|e| format!("starting {}: {e:?}", argv.first().map_or("", |s| s)))?;
    kernels_core::thread::run(&mut cx, &mut task);
    drop(task);
    // Process 1's first thread is over; the process may still have others.
    let (status, ended) = &kernel.init;
    let mut status = lock(status);
    while status.is_none() {
        status = ended.wait(status).unwrap_or_else(|p| p.into_inner());
    }
    let result = status.unwrap_or(Status::Exited(0));
    drop(status);
    kernel.end_everything();
    Ok(result)
}

/// One machine's kernel, shared by all its guest threads.
pub(crate) struct Kernel {
    exec: SharedExec,
    /// The clock page every process maps as `[vvar]`, for the vDSO, and what it's written from (`os::vdso`).
    clock: os::vdso::Clock,
    host: Arc<dyn Host>,
    disk: Mutex<Disk>,
    cache: Mutex<PageCache>,
    futexes: Futexes,
    processes: Mutex<BTreeMap<i32, Arc<Process>>>,
    /// Process and thread IDs share one count, as on Linux.
    next_id: AtomicI32,
    /// How process 1 ended, once it has.
    init: (Mutex<Option<Status>>, Condvar),
    options: Options,
    /// The host's terminal, as the machine's console.
    console: Option<Arc<Tty>>,
    /// Ptys, by number, while their master's open.
    ptys: Mutex<BTreeMap<u32, Weak<Tty>>>,
    next_pty: AtomicU32,
    /// The machine has stopped.
    done: AtomicBool,
    /// Called once the machine has stopped: the kernel's own servers (X11, Wayland, PulseAudio) wake from waiting, see
    /// `done`, and let the kernel go at once, rather than at their next look.
    on_done: Mutex<Vec<Box<dyn Fn() + Send>>>,
    /// Told when the host's stdin gets data or ends.
    stdin_watchers: Arc<kernels_core::wait::Watchers>,
    /// The machine's network.
    net: ::net::Network,
    /// The timers its processes' signal timers are kept by.
    timers: kernels_core::timers::Timers,
    /// Its inotify watches, by file; the disk's observer reads them too.
    watches: fs::inotify::Registry,
    /// Its fanotify groups.
    fanotify: fs::fanotify::Registry,
    /// The shared buffers of FIFOs that are open, by inode.
    fifos: Mutex<BTreeMap<layers::Ino, Weak<fs::Pipe>>>,
    /// The namespaces process 1 starts in; every other process comes from it, with its own as `clone`, `unshare`
    /// and `setns` make them.
    boot_ns: ns::Namespaces,
    /// The kernel's log, the machine's whatever its namespaces.
    klog: Mutex<admin::Klog>,
    /// The system layer's files, made as they're first opened (`os::system`).
    system: Mutex<BTreeMap<String, Arc<mm::shared::SharedMem>>>,
    /// Processes started from outside (`carbon exec`), by pid, and how each ended once it has.
    entered: Mutex<BTreeMap<i32, Arc<(Mutex<Option<Status>>, Condvar)>>>,
}

/// Feeds the host's keyboard to the console, sending the signals it asks for, and keeps its size the terminal's.
fn feed_console(k: &Arc<Kernel>) {
    let Some(tty) = k.console.clone() else { return };
    let mut buffer = [0u8; 4096];
    while !k.done.load(Ordering::Acquire) {
        if k.host.wait_input(Some(std::time::Duration::from_millis(200))) {
            let n = k.host.read(&mut buffer);
            if n == 0 {
                return;
            }
            for (pgid, signal) in tty.input(&buffer[..n]) {
                k.signal_group(pgid, signal);
            }
        }
        if let Some((rows, cols)) = k.host.terminal_size()
            && let Some((pgid, signal)) = tty.set_winsize([rows, cols, 0, 0])
        {
            k.signal_group(pgid, signal);
        }
    }
}

impl Kernel {
    /// `wake` is called once the machine has stopped (or now, if it has).
    pub(crate) fn when_done(&self, wake: Box<dyn Fn() + Send>) {
        let mut waiting = lock(&self.on_done);
        if self.done.load(Ordering::Acquire) {
            drop(waiting);
            wake();
        } else {
            waiting.push(wake);
        }
    }

    fn new_id(&self) -> i32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn process(&self, pid: i32) -> Option<Arc<Process>> {
        lock(&self.processes).get(&pid).cloned()
    }

    fn now(&self) -> u64 {
        self.host.wall()
    }

    /// Ends every process, as the machine stops.
    fn end_everything(&self) {
        let all: Vec<Arc<Process>> = lock(&self.processes).values().cloned().collect();
        for process in all {
            process.end(Status::Killed(abi::SIGKILL));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::own_env;

    fn env(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_cleared_environment_finds_the_display() {
        let mut e = env(&[]);
        own_env(true, true, &mut e);
        assert_eq!(e, env(&["XDG_RUNTIME_DIR=/run/user/0", "WAYLAND_DISPLAY=wayland-0", "DISPLAY=:0", "PULSE_SERVER=unix:/run/user/0/pulse/native"]));
    }

    #[test]
    fn empty_counts_as_unset_and_a_programs_own_value_stays() {
        let mut e = env(&["DISPLAY=", "WAYLAND_DISPLAY=nested-1"]);
        own_env(true, false, &mut e);
        assert_eq!(e, env(&["WAYLAND_DISPLAY=nested-1", "XDG_RUNTIME_DIR=/run/user/0", "DISPLAY=:0"]));
    }

    #[test]
    fn another_runtime_dir_gets_the_sockets_whole_path() {
        let mut e = env(&["XDG_RUNTIME_DIR=/tmp/runtime-root"]);
        own_env(true, false, &mut e);
        assert_eq!(e, env(&["XDG_RUNTIME_DIR=/tmp/runtime-root", "WAYLAND_DISPLAY=/run/user/0/wayland-0", "DISPLAY=:0"]));
    }

    #[test]
    fn no_desktop_no_display() {
        let mut e = env(&["PATH=/bin"]);
        own_env(false, false, &mut e);
        assert_eq!(e, env(&["PATH=/bin"]));
    }
}
