//! # carbon
//!
//! Carbon as a library: run Linux and Windows programs in machines of their own, with no guest OS to install, and
//! see and decide what they do as they do it.
//!
//! A machine gives a program its own disk, network, processes and memory, walled off from the host and from every
//! other machine. Carbon brings its own kernels, written in Rust (lambda speaks Linux's system calls, atlas is
//! Windows' kernel and its system DLLs), so a machine is ready the moment it starts, and program code runs straight
//! on the CPU, in a hardware VM or a process sandbox. Programs come from images, as Docker's do: `debian:13`, or
//! [atlas-base](https://github.com/tensorai-co/atlas-base) for Windows' own.
//!
//! ```toml
//! [dependencies]
//! carbon = { git = "https://github.com/tensorai-co/carbon" }
//! ```
//!
//! ```no_run
//! use carbon::{Carbon, Isolation, Network};
//! use carbon::hooks::{Decision, Filter};
//!
//! fn main() -> carbon::Result<()> {
//!     carbon::init(); // first: the process sandbox's stub is this same binary, run again
//!     let carbon = Carbon::open()?; // CARBON_HOME, or ~/.carbon
//!
//!     // A command in a machine of its own, and what it wrote.
//!     let out = carbon
//!         .machine("debian:13")
//!         .isolation(Isolation::Vm)
//!         .network(Network::None)
//!         .memory("512m")
//!         .output(["sh", "-c", "echo hello from $(uname -s)"])?;
//!     print!("{}", out.stdout_lossy());
//!
//!     // A machine whose programs are decided as they start, and whose connections are too.
//!     let dev = carbon
//!         .machine("debian:13")
//!         .name("dev")
//!         .publish("2222:22")
//!         .on_exec(Filter::program("rm"), |e| if e.args().iter().any(|a| a == "-rf") { Decision::deny() } else { Decision::pass() })
//!         .on_connect(Filter::any(), |c| if c.name() == Some("pypi.org") { Decision::allow() } else { Decision::refuse() })
//!         .start(["sleep", "infinity"])?;
//!     let ls = dev.machine().exec(["ls", "/"]).output()?;
//!     print!("{}", ls.stdout_lossy());
//!     dev.stop();
//!     dev.wait()?;
//!     Ok(())
//! }
//! ```
//!
//! ## What a program using it needs
//!
//! - **`carbon::init()` first in `main`.** The process sandbox runs a program in a stub, which is this binary run
//!   again; `init` is where the stub takes over.
//! - **On macOS, the binary signed for the hypervisor**:
//!   `codesign --force --sign - --entitlements carbon.entitlements target/release/yourapp`, with the entitlements in
//!   this repository (`com.apple.security.hypervisor`, `cs.debugger`, `get-task-allow`). Unsigned, machines fall back to
//!   the sandbox, and say so. On Linux, `/dev/kvm` (the `kvm` group); on Windows, `winkvm.sys`.
//! - **Building Windows' and the GPU's guest code** (ntdll, the Vulkan and GL drivers), which atlas's and lambda's
//!   build scripts do with `rust-src` (`rustup component add rust-src`) and LLVM's `lld-link` and `ld.lld`. Without
//!   them, everything builds, and Linux machines run; Windows machines and the guest GPU drivers say what's missing.
//!
//! ## The parts
//!
//! | Type | What it is |
//! | --- | --- |
//! | [`Carbon`] | the store on the host's disk: [`Carbon::machine`] and [`Carbon::program`] to make a [`MachineSpec`]; machines, images, pull, import |
//! | [`MachineSpec`] | a builder: name, isolation, memory, platform, env, network and ports, display, GPU, sound, streams, hooks and rules; then [`MachineSpec::run`], [`MachineSpec::output`] or [`MachineSpec::start`] |
//! | [`Running`] | a machine this process runs: its piped streams, its hooks, `stop`, `kill`, `wait` |
//! | [`Machine`] | a machine by name, in this process or another: `exec`, `stop`, `ports`, `screen`, `watch`, `freeze`, `copy_in`, `copy_out`, `remove` |
//! | [`Process`] | what [`Machine::exec`] started: stdin, output as it comes, signals, its exit |
//! | [`Screen`] | a running machine's desktop: screenshot, windows, click, type, keys, clipboard, drops |
//! | [`hooks`] | events (programs started, output, connections, lookups), decisions, and the [`hooks::Hub`] every hook point asks |
//! | [`Rules`] | decisions without code: programs that may not start, where the network may go |
//! | [`build`] | building images from a Carbonfile (the Dockerfile's grammar, exactly): [`Carbon::build`] |
//! | [`wire`] | the control protocol other processes and languages use |

mod boot;
pub mod build;
mod error;
mod files;
pub mod logs;
mod machine;
mod mounts;
mod users;
mod network;
mod rules;
#[cfg(unix)]
mod signals;
mod spec;
mod stdio;
mod stop;
mod store;
pub mod wire;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Events, decisions, hooks and the hub: see the crate `hooks`.
pub use hooks;

pub use error::{Error, ErrorKind, Result};
pub use machine::{Button, ExecSpec, Freeze, Machine, MachineInfo, Port, Process, ProcessEvent, ProcessHandle, Screen, Window};
pub use files::{Copy, DirEntry, FileInfo, FileKind, Files, MOST_READ, Remove, Write};
pub use logs::{LogLine, LogReader, LogStream, Logs};
pub use mounts::{Mount, MountKind, Volume, VolumeInfo};
pub use rules::Rules;
pub use spec::{Audio, Display, Exit, MachineSpec, MemorySize, Output, Running, parse_size};
pub use stdio::{Chunks, Stdin, Stdio};
pub use images::{PullProgress, PullStage};
pub use store::{BUILD_CACHE_KEEP, Kernel, PRUNE_GRACE, Pruned, network_name, parse_network};
pub use wire::Watch;

/// A hardware VM, the process sandbox, or the best there is.
pub use exec::Isolation;
/// An instruction set: what [`atlas::imports`] checks a program against.
pub use exec::Isa;
/// Which GPU a machine gets: the host's, the CPU device, the host's if there is one (the default), or none.
pub use gpu_api::Choice as Gpu;
/// An image on the host's disk, and the OS and CPU it's for.
pub use images::{Config as ImageConfig, Disk, Healthcheck, History, Image, Layer, Lineage, Platform, PushProgress, PushStage, Pushed};
/// What a machine's network reaches, and a port it publishes on the host's.
pub use net::{Mode as Network, Publish};

/// How to push an image: as another name than its own, or squashed into one layer.
#[derive(Debug, Clone, Default)]
pub struct Push {
    to: Option<String>,
    squash: bool,
}

impl Push {
    pub fn new() -> Push {
        Push::default()
    }

    /// Pushes it as `name` (`registry.example.com/team/app:2`), as if it were tagged so first.
    pub fn to(mut self, name: impl Into<String>) -> Push {
        self.to = Some(name.into());
        self
    }

    /// Its whole disk as one layer, not a layer per step.
    pub fn squash(mut self, squash: bool) -> Push {
        self.squash = squash;
        self
    }
}

/// An image as JSON, as `carbon inspect` prints it: its name, index, platform, config, history and layers.
pub fn image_json(image: &Image) -> String {
    let v = serde_json::json!({
        "name": image.name,
        "index": image.index,
        "platform": image.platform.to_string(),
        "created": image.created,
        "source": image.source,
        "config": image.config.to_json(),
        "history": image.history.iter().map(History::to_json).collect::<Vec<_>>(),
        "layers": image.lineage.layers.iter().map(|l| &l.digest).collect::<Vec<_>>(),
        "repository": image.lineage.repository,
        "disks": image.lineage.disks.len(),
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// What atlas has of Windows' API, for checking programs against it (`carbon atlas imports`).
pub mod atlas {
    pub use ::atlas::imports;
}

/// Something a machine said as it was made or ran: a fallback it took (no hypervisor, so the sandbox; no GPU, so the
/// CPU device), a warning, or what it's doing (pulling an image).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// What it's doing: pulling, unpacking.
    Progress,
    /// It took something other than the best there is, and why. Carbon never falls back silently.
    Fallback,
    Warning,
}

impl Notice {
    pub(crate) fn progress(message: String) -> Notice {
        Notice { kind: NoticeKind::Progress, message }
    }
    pub(crate) fn fallback(message: String) -> Notice {
        Notice { kind: NoticeKind::Fallback, message }
    }
    pub(crate) fn warning(message: String) -> Notice {
        Notice { kind: NoticeKind::Warning, message }
    }
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Call first in `main`, before anything else: where Carbon's own uses of this binary take over. The process
/// sandbox's stub (on macOS), and a launched machine's worker ([`MachineSpec::launch`]), are this binary run again;
/// `init` runs them and never returns, in those processes. In any other it returns at once.
pub fn init() {
    exec::run_if_stub();
    worker::run_if_worker();
}

struct Inner {
    store: store::Store,
    notify: RwLock<boot::Notify>,
}

/// Carbon on this host: its machines and images, kept in a folder (`CARBON_HOME`, or `~/.carbon`). Cloning it shares
/// it.
#[derive(Clone)]
pub struct Carbon(Arc<Inner>);

impl std::fmt::Debug for Carbon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Carbon").field(&self.home()).finish()
    }
}

impl Carbon {
    /// Carbon's folder: `CARBON_HOME`, or `~/.carbon`.
    pub fn open() -> Result<Carbon> {
        Carbon::open_in(None)
    }

    /// Carbon in a folder of its own: a test's, or a program's that keeps its machines apart.
    pub fn open_at(home: impl AsRef<Path>) -> Result<Carbon> {
        Carbon::open_in(Some(home.as_ref()))
    }

    fn open_in(home: Option<&Path>) -> Result<Carbon> {
        let notify: boot::Notify = Arc::new(|notice: Notice| eprintln!("carbon: {notice}"));
        Ok(Carbon(Arc::new(Inner { store: store::Store::open(home)?, notify: RwLock::new(notify) })))
    }

    /// Its folder.
    pub fn home(&self) -> PathBuf {
        self.0.store.images.home().to_path_buf()
    }

    /// Who's told what machines fall back to, warn of, or are doing: stderr, as `carbon: ...`, unless this says
    /// otherwise.
    pub fn on_notice(&self, f: impl Fn(Notice) + Send + Sync + 'static) {
        *self.0.notify.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(f);
    }

    pub(crate) fn notify(&self) -> boot::Notify {
        self.0.notify.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// A new machine from an image, pulled if it isn't here: `debian:13`, `ghcr.io/org/app:1`, `atlas` (atlas-base,
    /// imported), or `windows` (atlas's system layer alone).
    pub fn machine(&self, image: impl Into<String>) -> MachineSpec {
        MachineSpec::new(self.clone(), None, Some(store::Source::Image(image.into())))
    }

    /// A new machine from a program on this host, put on an empty disk: an ELF for lambda, a PE for atlas.
    pub fn program(&self, path: impl Into<PathBuf>) -> MachineSpec {
        MachineSpec::new(self.clone(), None, Some(store::Source::Program(path.into())))
    }

    /// The stopped machine called `name`, started again with its files as it left them.
    pub fn again(&self, name: impl Into<String>) -> MachineSpec {
        MachineSpec::new(self.clone(), Some(name.into()), None)
    }

    /// The machine called `name`.
    pub fn get(&self, name: &str) -> Result<Machine> {
        self.0.store.get(name)?;
        Ok(Machine::new(self.clone(), name.to_string()))
    }

    /// Every machine, by name.
    pub fn machines(&self) -> Vec<MachineInfo> {
        self.0.store.list().into_iter().map(MachineInfo::of).collect()
    }

    /// The images here.
    pub fn images(&self) -> Vec<Image> {
        self.0.store.images.images()
    }

    pub fn image(&self, name: &str) -> Option<Image> {
        self.0.store.images.image(name)
    }

    /// The volume `name`: [`Volume::create`] makes it, and a machine that mounts it makes it if it isn't there.
    pub fn volume(&self, name: &str) -> Volume {
        Volume { home: self.home(), name: name.to_string() }
    }

    /// Every volume.
    pub fn volumes(&self) -> Vec<VolumeInfo> {
        mounts::list_volumes(&self.home())
    }

    /// Removes a volume and its files, unless a running machine mounts it.
    pub fn remove_volume(&self, name: &str) -> Result<()> {
        if let Some(machine) = self.0.store.list().into_iter().find(|r| r.running() && r.mounts.iter().any(|m| m.kind == MountKind::Volume(name.to_string()))) {
            return Err(Error::new(ErrorKind::Running, format!("volume {name} is mounted by {}, which is running", machine.name)));
        }
        self.volume(name).remove()
    }

    /// Pulls an image from its registry, for `platform` (Linux for this host's CPU by default). Its progress goes to
    /// [`Carbon::on_notice`] a step at a time; [`Carbon::pull_with`] has it as it happens.
    pub fn pull(&self, name: &str, platform: Option<Platform>) -> Result<Image> {
        let platform = platform.unwrap_or_else(Platform::linux_here);
        let notify = self.notify();
        self.0.store.pull(name, &platform, &mut store::stage_notices(&|what| notify(Notice::progress(what))))
    }

    /// Pulls an image as [`Carbon::pull`] does, telling `progress` where it is as each piece of each layer arrives:
    /// what it's doing, which layer, and bytes here of the total, for a progress bar.
    pub fn pull_with(&self, name: &str, platform: Option<Platform>, mut progress: impl FnMut(&PullProgress)) -> Result<Image> {
        let platform = platform.unwrap_or_else(Platform::linux_here);
        self.0.store.pull(name, &platform, &mut progress)
    }

    /// An image from a tar of a root file system, as `docker import` makes one: named `name`, for `platform` (else a
    /// Windows one for this host's CPU if the tar is of `C:\`, as atlas-base's are, or Linux's), its config from
    /// `changes`, each a Dockerfile instruction: `CMD`, `ENTRYPOINT`, `ENV`, `WORKDIR`, `USER`.
    pub fn import(&self, name: &str, tar: &[u8], platform: Option<Platform>, changes: &[&str]) -> Result<Image> {
        let changes: Vec<String> = changes.iter().map(|c| c.to_string()).collect();
        self.0.store.import(name, tar, platform, &changes)
    }

    /// Another name for an image (`app:2` as `registry.example.com/team/app:2`): the same disk and config.
    pub fn tag(&self, source: &str, target: &str) -> Result<Image> {
        self.0.store.images.tag(source, target).map_err(error::image)
    }

    /// Pushes an image to its registry, as `docker push` does: a layer per step of what made it (a pulled base's
    /// layers as the registry has them, taken from their own repository when they're on the same registry), with the
    /// login [`Carbon::login`] keeps for that registry, or Docker's. Its progress goes to [`Carbon::on_notice`];
    /// [`Carbon::push_with`] has it as it happens, and pushes as another name or squashed.
    pub fn push(&self, name: &str) -> Result<Pushed> {
        let notify = self.notify();
        self.0.store.push(name, None, false, &mut |p| notify(Notice::progress(p.to_string())))
    }

    /// Pushes an image as [`Push`] says, telling `progress` where it is as each layer is made and sent.
    pub fn push_with(&self, name: &str, how: &Push, mut progress: impl FnMut(&PushProgress)) -> Result<Pushed> {
        self.0.store.push(name, how.to.as_deref(), how.squash, &mut progress)
    }

    /// Checks a login with a registry (`docker.io`, `ghcr.io`, `localhost:5000`), and keeps it for its pulls and
    /// pushes, in `<home>/auth.json`, readable by its owner alone. A registry Carbon has no login for is looked for in
    /// Docker's own (`~/.docker/config.json` and its credential helpers).
    pub fn login(&self, registry: &str, user: &str, secret: &str) -> Result<()> {
        let credentials = images::auth::Credentials { user: user.into(), secret: secret.into() };
        self.0.store.images.login(registry, &credentials).map_err(|e| Error::new(ErrorKind::Denied, e))
    }

    /// Forgets the login Carbon keeps for a registry; whether it had one (Docker's own are left alone).
    pub fn logout(&self, registry: &str) -> Result<bool> {
        self.0.store.images.logout(registry).map_err(error::image)
    }

    /// Writes images to `out` as one archive, as `docker save` does, that `docker load` and [`Carbon::load`] read: an
    /// OCI image layout with Docker's `manifest.json`, a layer per step.
    pub fn save(&self, names: &[&str], mut out: impl std::io::Write) -> Result<()> {
        self.0.store.save(names, &mut out, &mut |_| {})
    }

    /// Loads the images an archive has (`docker save`'s or [`Carbon::save`]'s, or an OCI layout as a tar; gzipped or
    /// not), named as they were saved, or by their ids. What it loaded.
    pub fn load(&self, path: impl AsRef<Path>) -> Result<Vec<Image>> {
        self.0.store.load(path.as_ref())
    }

    /// Forgets an image.
    pub fn remove_image(&self, name: &str) -> Result<()> {
        self.0.store.images.remove(name).map_err(error::image)
    }

    /// Frees the disk space of what nothing uses any more: the chunks of removed machines' files and removed images,
    /// which no image, machine or machine's base still names. Nothing is pruned while any machine runs, and chunks
    /// written in the last 15 minutes are kept, for a pull still under way. How much went.
    pub fn prune(&self) -> Result<Pruned> {
        self.0.store.prune()
    }
}

/// The host's devices a machine's GPU can be, as `carbon gpu` lists them: its GPU if it has one, and the CPU device.
pub fn gpus() -> Vec<std::result::Result<gpu_api::Info, String>> {
    [Gpu::Host, Gpu::Software].into_iter().map(|choice| gpu_api::Gpu::open(choice).map_err(|e| e.message).and_then(|g| g.map(|g| g.info().clone()).ok_or_else(|| "none".to_string()))).collect()
}
