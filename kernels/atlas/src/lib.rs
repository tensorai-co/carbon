//! atlas: Carbon's Windows kernel, and all of Windows with it, as one Rust module on `exec`.
//!
//! There's no Windows image to bring, so atlas brings the OS: every system DLL is a declaration in [`os`] (its exports,
//! and the host Rust behind them), made into a real PE in memory for each ISA. Nothing is ever a file of its own.
//! The rule for what runs where is *cross where Windows crosses*: what Windows runs inside the program (the loader,
//! heaps, locks, exception handling) is `atlas-guest`, compiled for the program's ISA; what Windows runs in its kernel
//! or its services is host Rust here, reached through a two-instruction trap stub.
//!
//! Each export is `host` (host Rust), `guest` (forwarded to `atlas-guest`'s ntdll) or `missing` (a stub that names
//! itself, so the next thing to write is obvious). Host code can call the program back with an upcall.

mod environment;
mod exceptions;
mod files;
mod handles;
mod images;
mod mapping;
mod objects;
mod threads;
mod os;
mod process;
mod registry;
mod upcall;
mod win32k;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use exec::{Cpu, Exec};
use host::Host;
use kernels_core::lock;
use kernels_core::thread::{Cx, SharedExec};

/// What a file means to atlas: its `FILE_ATTRIBUTE_*` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Meta {
    pub attributes: u32,
}

/// On disk: the attributes, and three words to spare.
impl layers::Persist for Meta {
    fn save(&self) -> [u32; 4] {
        [self.attributes, 0, 0, 0]
    }
    fn load(words: [u32; 4]) -> Meta {
        Meta { attributes: words[0] }
    }
}

/// A machine's disk, as atlas sees it: folding case, as Windows does.
pub type Disk = layers::Fs<Meta>;

/// A new disk for atlas: an empty `C:\`, whose directories fold case.
pub fn new_disk() -> Disk {
    layers::Fs::new(Meta::default(), true)
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Report each host export and its result on stderr.
    pub trace: bool,
    /// Environment variables the first process gets, over the registry's and the session's.
    pub env: Vec<(String, String)>,
    /// The desktop its windows go on; else a screen in memory of its own.
    pub display: Option<display::desktop::Shared>,
    /// The way in for stopping the machine from outside it.
    pub stop: Stop,
}

/// A running machine, from outside it: how `carbon stop` reaches in, from any thread.
#[derive(Clone, Default)]
pub struct Stop(Arc<Mutex<Option<std::sync::Weak<process::Process>>>>);

impl std::fmt::Debug for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stop")
    }
}

impl Stop {
    /// Ends the machine's program with `code`, as `TerminateProcess` would. Whether it was running.
    pub fn end(&self, code: u32) -> bool {
        let Some(process) = lock(&self.0).as_ref().and_then(std::sync::Weak::upgrade) else { return false };
        process.end(code);
        true
    }
}

/// How a machine ended.
pub struct Ended {
    /// Its first process's exit code.
    pub code: u32,
    /// Each top-level window's picture, by its title: as it was destroyed, or as the machine stopped.
    pub windows: Vec<(String, display::Surface)>,
    /// The desktop as the machine stopped.
    pub screen: display::Surface,
}

/// Boots a machine: runs `argv[0]` (a path on `disk`, as `C:\app.exe` or `/app.exe`) as its first process, on this
/// thread with the CPU `Exec::open` gave, and returns how it ended.
pub fn boot(exec: Exec, mut cpu: Cpu, disk: Disk, host: Arc<dyn Host>, argv: &[&str], options: Options) -> Result<Ended, String> {
    let mut disk = disk;
    files::populate(&mut disk);
    // The registry the machine's disk has, else a fresh machine's.
    let saved = disk.resolve(layers::ROOT, registry::SAVED, true).ok().and_then(|ino| disk.contents(ino).ok()).and_then(|bytes| registry::Registry::load(&bytes));
    let desktop = options.display.clone().unwrap_or_else(|| display::Desktop::shared(win32k::SCREEN.0, win32k::SCREEN.1));
    let kernel = Arc::new(Kernel {
        registry: Mutex::new(saved.unwrap_or_default()),
        win32k: Mutex::new(win32k::Win32k::new(desktop.clone())),
        exec: Arc::new(Mutex::new(exec)),
        host,
        disk: Mutex::new(disk),
        images: images::Images::default(),
        objects: objects::Objects::default(),
        futexes: kernels_core::wait::Futexes::default(),
        next_id: AtomicU64::new(4),
        options,
    });
    // Input on the desktop wakes the threads that have windows, to take it as messages.
    let weak = Arc::downgrade(&kernel);
    display::desktop::lock(&desktop).set_waker(Arc::new(move || {
        if let Some(k) = weak.upgrade() {
            lock(&k.win32k).wake_all();
        }
    }));
    let exec = kernel.exec.clone();
    let mut cx = Cx::new(&exec, &mut cpu);
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let mut task = process::start(&kernel, &mut cx, &argv)?;
    *lock(&kernel.options.stop.0) = Some(Arc::downgrade(&task.process));
    kernels_core::thread::run(&mut cx, &mut task);
    // The first thread is over; the process lasts until its last thread is.
    let process = task.process.clone();
    let (ended, done) = &process.ended;
    let mut code = lock(ended);
    while code.is_none() {
        code = done.wait(code).unwrap_or_else(|p| p.into_inner());
    }
    let code = code.unwrap_or(0);
    let space = lock(&process.state).space;
    cx.turn(|t| t.destroy_space(space));
    kernel.save_registry();
    // Whatever the machine wrote is kept, if its disk has somewhere to keep it.
    if let Err(e) = lock(&kernel.disk).sync() {
        kernel.host.write(host::Stream::Err, format!("atlas: saving the disk: {e}\n").as_bytes());
    }
    let windows = lock(&kernel.win32k).screenshots();
    let screen = display::desktop::lock(&desktop).screen().clone();
    Ok(Ended { code, windows, screen })
}

/// One machine's kernel, shared by its guest threads.
pub(crate) struct Kernel {
    #[allow(dead_code)]
    exec: SharedExec,
    host: Arc<dyn Host>,
    disk: Mutex<Disk>,
    images: images::Images,
    registry: Mutex<registry::Registry>,
    win32k: Mutex<win32k::Win32k>,
    objects: objects::Objects,
    /// What ntdll waits on by address: contended locks, condition variables, `WaitOnAddress`.
    futexes: kernels_core::wait::Futexes,
    /// Process and thread IDs share one count, in fours, as on Windows.
    next_id: AtomicU64,
    options: Options,
}

impl Kernel {
    /// Saves the registry in the machine's own layer.
    fn save_registry(&self) {
        let bytes = lock(&self.registry).save();
        let mut disk = lock(&self.disk);
        let now = self.host.wall();
        let dir = Meta { attributes: 0x10 };
        if let Ok(ino) = disk.put(registry::SAVED, layers::New::File, &[], Meta { attributes: 0x20 }, dir) {
            let _ = disk.truncate(ino, 0, now);
            let _ = disk.write(ino, 0, &bytes, now);
        }
    }

    fn new_id(&self) -> u64 {
        self.next_id.fetch_add(4, Ordering::Relaxed)
    }
}
