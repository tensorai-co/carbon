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
            trace: std::env::var_os("CARBON_TRACE").is_some(),
            console: true,
            stop: Stop::default(),
            network: None,
            display: None,
        }
    }
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

/// Boots a machine: runs `argv[0]`, a path on `disk`, as process 1, on this thread with the CPU `Exec::open` gave, and
/// returns how it ended. Other processes still running then are ended with it.
pub fn boot(mut exec: Exec, cpu: Cpu, disk: Disk, host: Arc<dyn Host>, argv: &[&str], options: Options) -> Result<Status, String> {
    let mut disk = disk;
    let mut options = options;
    if options.display.is_some() {
        for (key, value) in [("XDG_RUNTIME_DIR", os::wayland::RUNTIME_DIR), ("WAYLAND_DISPLAY", os::wayland::DISPLAY), ("DISPLAY", os::x11::DISPLAY)] {
            if !options.env.iter().any(|e| e.split('=').next() == Some(key)) {
                options.env.push(format!("{key}={value}"));
            }
        }
    }
    os::populate(&mut disk);
    let mount_ns = fs::mount::MountNs::new(fs::mount::Mounts::boot(&disk));
    let boot_ns = ns::Namespaces::boot(mount_ns, admin::Names { host: options.hostname.clone(), domain: "(none)".to_string() });
    let watches = fs::inotify::Registry::default();
    let fanotify = fs::fanotify::Registry::default();
    disk.observe(Some(fs::fanotify::observer(watches.clone(), fanotify.clone())));
    let console = (options.console && host.is_terminal()).then(|| Tty::console(host.clone()));
    let klog = admin::Klog::boot(host.cpus(), exec.memory().size().limit);
    let net = options.network.clone().unwrap_or_else(|| ::net::Network::on_this_host(::net::Config { hostname: options.hostname.clone(), dns: None }));
    let kernel = Arc::new(Kernel {
        console,
        ptys: Mutex::new(BTreeMap::new()),
        next_pty: AtomicU32::new(0),
        done: AtomicBool::new(false),
        exec: Arc::new(Mutex::new(exec)),
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
    });
    if let Some(desktop) = kernel.options.display.clone() {
        if let Err(e) = os::wayland::start(&kernel, desktop.clone()) {
            kernel.host.write(host::Stream::Err, format!("lambda: the Wayland server: {e}\n").as_bytes());
        }
        if let Err(e) = os::x11::start(&kernel, desktop) {
            kernel.host.write(host::Stream::Err, format!("lambda: the X server: {e}\n").as_bytes());
        }
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
    kernel.timers.stop();
    if kernel.console.is_some() {
        kernel.host.set_raw(false);
    }
    // Whatever the machine wrote is kept, if its disk has somewhere to keep it.
    if let Err(e) = lock(&kernel.disk).sync() {
        kernel.host.write(host::Stream::Err, format!("lambda: saving the disk: {e}\n").as_bytes());
    }
    result
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
