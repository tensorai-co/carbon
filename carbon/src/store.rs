//! Machines on the host's disk: each kept by name, with its record and its log in `<home>/machines/<name>/`, so it
//! can be started again with its files as it left them, stopped from another process, frozen into an image, copied
//! into and out of, or removed. And what a new machine's record is made from: an image, or a program on the host.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use images::{Config, Image, Images, Platform};
use layers::{Fs, Persist, ROOT};
use serde_json::{Value, json};

use crate::error::{self, Error, ErrorKind, Result};
use crate::{Network, Notice};

/// Which kernel a machine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kernel {
    /// Linux's system calls, for Linux images and ELF programs.
    Lambda,
    /// Windows' kernel and its system DLLs, for Windows images and PE programs.
    Atlas,
}

impl Kernel {
    pub fn name(self) -> &'static str {
        match self {
            Kernel::Lambda => "lambda",
            Kernel::Atlas => "atlas",
        }
    }
}

/// What a machine starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// An image by name, pulled if it isn't here.
    Image(String),
    /// A program on the host: an ELF for lambda, a PE for atlas, put on an empty disk.
    Program(PathBuf),
}

/// A network mode's name, as `--network` takes it.
pub fn network_name(mode: Network) -> &'static str {
    match mode {
        Network::Nat => "nat",
        Network::Host => "host",
        Network::None => "none",
    }
}

/// `--network`'s modes by name: `nat` (or Docker's `bridge`), `host`, `none`.
pub fn parse_network(name: &str) -> Option<Network> {
    match name {
        "nat" | "bridge" | "default" => Some(Network::Nat),
        "host" => Some(Network::Host),
        "none" => Some(Network::None),
        _ => None,
    }
}

/// A machine, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub name: String,
    pub kernel: Kernel,
    /// The image it was made from, in full, and that image's index then: the base its log is over, kept even if the
    /// name is pulled again later.
    pub image: Option<String>,
    pub index: Option<String>,
    /// The program it was made from, as it's named on its disk.
    pub program: Option<String>,
    pub platform: Platform,
    pub config: Config,
    /// Seconds since 1970.
    pub created: u64,
    /// Its process, while it runs.
    pub pid: Option<u32>,
    /// Where it was last run: `vm/hvf`, `sandbox/linux`, …
    pub backend: String,
    /// Its network mode, and the ports it publishes as they were written (`2222:22`), kept for when it runs again.
    pub network: Network,
    pub publish: Vec<String>,
    /// The host addresses its ports are published on while it runs (`0.0.0.0:2222->22/tcp`).
    pub ports: Vec<String>,
}

impl Record {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "kernel": self.kernel.name(),
            "image": self.image,
            "index": self.index,
            "program": self.program,
            "platform": self.platform.to_string(),
            "config": { "env": self.config.env, "entrypoint": self.config.entrypoint, "cmd": self.config.cmd, "workdir": self.config.workdir, "user": self.config.user },
            "created": self.created,
            "pid": self.pid,
            "backend": self.backend,
            "network": network_name(self.network),
            "publish": self.publish,
            "ports": self.ports,
        })
    }

    fn from_json(v: &Value) -> Option<Record> {
        let s = |key: &str| v.get(key).and_then(Value::as_str).map(String::from);
        let list = |c: Option<&Value>, key: &str| -> Vec<String> {
            c.and_then(|c| c.get(key)).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
        };
        let config = v.get("config");
        Some(Record {
            name: s("name")?,
            kernel: match s("kernel")?.as_str() {
                "lambda" => Kernel::Lambda,
                "atlas" => Kernel::Atlas,
                _ => return None,
            },
            image: s("image"),
            index: s("index"),
            program: s("program"),
            platform: Platform::parse(&s("platform")?)?,
            config: Config {
                env: list(config, "env"),
                entrypoint: list(config, "entrypoint"),
                cmd: list(config, "cmd"),
                workdir: config.and_then(|c| c.get("workdir")).and_then(Value::as_str).unwrap_or_default().into(),
                user: config.and_then(|c| c.get("user")).and_then(Value::as_str).unwrap_or_default().into(),
            },
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
            pid: v.get("pid").and_then(Value::as_u64).map(|p| p as u32),
            backend: s("backend").unwrap_or_default(),
            network: s("network").as_deref().and_then(parse_network).unwrap_or_default(),
            publish: list(Some(v), "publish"),
            ports: list(Some(v), "ports"),
        })
    }

    /// Whether its process is running now.
    pub fn running(&self) -> bool {
        self.pid.is_some_and(alive)
    }
}

#[cfg(unix)]
pub(crate) fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    pid != 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

#[cfg(not(unix))]
pub(crate) fn alive(pid: u32) -> bool {
    // Without a way to ask yet, only this process is known to be running.
    pid == std::process::id()
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The arch of this host's CPU, as an image's platform names it.
pub(crate) fn host_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" }
}

/// The machines and images on this host.
/// How long a chunk is kept after it's written, whatever names it: an image being pulled names its chunks only once
/// it's all there.
pub const PRUNE_GRACE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// What [`Store::prune`] deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    pub chunks: u64,
    pub bytes: u64,
}

type Hash = layers::Hash;

/// A pull's progress as notices: one each time it starts something new (a layer, unpacking, the index), not one for
/// every piece of a download.
pub(crate) fn stage_notices<'a>(notice: &'a dyn Fn(String)) -> impl FnMut(&images::PullProgress) + 'a {
    let mut last: Option<(images::PullStage, usize)> = None;
    move |p| {
        if last != Some((p.stage, p.layer)) {
            last = Some((p.stage, p.layer));
            notice(match p.stage {
                images::PullStage::Downloading => format!("layer {} of {}: {:.1} MiB in all", p.layer, p.layers, p.total as f64 / f64::from(1 << 20)),
                _ => p.to_string(),
            });
        }
    }
}

pub(crate) struct Store {
    pub images: Images,
}

impl Store {
    pub fn open(home: Option<&Path>) -> Result<Store> {
        Ok(Store { images: Images::open(home).map_err(error::image)? })
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        self.images.machine_dir(name)
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.dir(name).join("machine.json")
    }

    pub fn record(&self, name: &str) -> Option<Record> {
        let text = fs::read_to_string(self.record_path(name)).ok()?;
        Record::from_json(&serde_json::from_str(&text).ok()?)
    }

    /// A machine's record, or that there's no such machine.
    pub fn get(&self, name: &str) -> Result<Record> {
        self.record(name).ok_or_else(|| Error::no_machine(name))
    }

    pub fn save_record(&self, record: &Record) -> Result<()> {
        let path = self.record_path(&record.name);
        fs::create_dir_all(path.parent().ok_or_else(|| Error::invalid("a bad machine name"))?).map_err(|e| Error::io(format!("{}: {e}", path.display())))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, serde_json::to_vec_pretty(&record.to_json()).unwrap_or_default()).map_err(|e| Error::io(format!("{}: {e}", temporary.display())))?;
        fs::rename(&temporary, &path).map_err(|e| Error::io(format!("{}: {e}", path.display())))
    }

    /// Every machine, by name.
    pub fn list(&self) -> Vec<Record> {
        let mut out: Vec<Record> = fs::read_dir(self.images.home().join("machines"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| self.record(&entry.file_name().to_string_lossy()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// A stopped machine's record, or why it can't be had.
    fn stopped(&self, name: &str) -> Result<Record> {
        let record = self.get(name)?;
        if record.running() {
            return Err(Error::running(name));
        }
        Ok(record)
    }

    /// Removes a stopped machine: its record and its writable layer. Its image stays. The chunks of what it wrote stay
    /// in the store until [`Store::prune`] finds nothing names them.
    pub fn remove(&self, name: &str) -> Result<()> {
        self.stopped(name)?;
        fs::remove_dir_all(self.dir(name)).map_err(|e| Error::io(format!("removing {name}: {e}")))
    }

    /// Deletes every chunk in the store that nothing names: no image's index, no machine's log, and no index a
    /// machine's disk is over. A running machine writes chunks before its log names them, so nothing is pruned while
    /// any machine runs; and a chunk written in the last [`PRUNE_GRACE`] is kept, for an image still being pulled or
    /// imported, by this process or another. How many chunks went, and their bytes.
    pub fn prune(&self) -> Result<Pruned> {
        let records = self.list();
        if let Some(running) = records.iter().find(|r| r.running()) {
            return Err(Error::running(&running.name));
        }
        let mut store = self.images.store().map_err(error::image)?;
        let mut live: HashSet<Hash> = HashSet::new();
        let keep_index = |hash: Hash, store: &mut images::store::DiskStore, live: &mut HashSet<Hash>| -> Result<()> {
            if !live.insert(hash) {
                return Ok(());
            }
            // An index that can't be read means the store isn't what the records say: prune nothing rather than guess.
            let index = layers::Store::get(store, &hash).map_err(|_| Error::io(format!("an index named {} is missing or corrupt; nothing pruned", images::store::hex(&hash))))?;
            live.extend(layers::index_chunks(&index).map_err(|_| Error::io(format!("the index {} can't be read; nothing pruned", images::store::hex(&hash))))?);
            Ok(())
        };
        for image in self.images.images() {
            let hash = images::store::parse_hash(&image.index).ok_or_else(|| Error::io(format!("{}'s record is corrupt; nothing pruned", image.name)))?;
            keep_index(hash, &mut store, &mut live)?;
        }
        for record in &records {
            if let Some(index) = record.index.as_deref() {
                let hash = images::store::parse_hash(index).ok_or_else(|| Error::io(format!("{}'s record is corrupt; nothing pruned", record.name)))?;
                keep_index(hash, &mut store, &mut live)?;
            }
            match fs::read(self.dir(&record.name).join("log")) {
                Ok(log) => live.extend(layers::log_chunks(&log)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(format!("{}'s disk: {e}; nothing pruned", record.name))),
            }
        }
        // Machines whose records can't be read still have logs: their chunks are kept too.
        if let Ok(dirs) = fs::read_dir(self.images.home().join("machines")) {
            for dir in dirs.flatten() {
                if let Ok(log) = fs::read(dir.path().join("log")) {
                    live.extend(layers::log_chunks(&log));
                }
            }
        }

        let chunks = self.images.home().join("chunks");
        let young = std::time::SystemTime::now().checked_sub(PRUNE_GRACE).unwrap_or(std::time::UNIX_EPOCH);
        let mut pruned = Pruned::default();
        for shard in fs::read_dir(&chunks).map_err(|e| Error::io(format!("{}: {e}", chunks.display())))?.flatten() {
            let Ok(entries) = fs::read_dir(shard.path()) else { continue };
            let prefix = shard.file_name().to_string_lossy().into_owned();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                // A name starting with `.` is a chunk still being written.
                let Some(hash) = (!name.starts_with('.')).then(|| images::store::parse_hash(&format!("{prefix}{name}"))).flatten() else { continue };
                if live.contains(&hash) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if meta.modified().map_or(true, |m| m > young) {
                    continue;
                }
                if fs::remove_file(entry.path()).is_ok() {
                    pruned.chunks += 1;
                    pruned.bytes += meta.len();
                }
            }
        }
        Ok(pruned)
    }

    /// An image from a tar of a root file system, as `docker import` makes one: named `name`, for `platform` (else a
    /// Windows one for this host's CPU if the tar's first entry is `Windows/`, as atlas-base's are, or Linux's), with
    /// its config from `changes`, each a Dockerfile instruction: `CMD`, `ENTRYPOINT`, `ENV`, `WORKDIR`, `USER`.
    pub fn import(&self, name: &str, tar: &[u8], platform: Option<Platform>, changes: &[String]) -> Result<Image> {
        let first = tar.get(..100).map(|n| String::from_utf8_lossy(n).trim_end_matches('\0').to_string()).unwrap_or_default();
        let platform = platform.unwrap_or_else(|| {
            if first.trim_start_matches("./").to_ascii_lowercase().starts_with("windows") {
                Platform { os: "windows".into(), arch: host_arch().into() }
            } else {
                Platform::linux_here()
            }
        });
        let windows = platform.os == "windows";
        let mut config = Config::default();
        for change in changes {
            let bad = |why: String| Error::invalid(format!("--change {change}: {why}"));
            let (instruction, rest) = change.trim().split_once(char::is_whitespace).ok_or_else(|| bad("an instruction and its value".into()))?;
            let rest = rest.trim();
            // `["a", "b"]`, or a shell's line, as Docker reads them.
            let argv = || -> Result<Vec<String>> {
                if rest.starts_with('[') {
                    serde_json::from_str::<Vec<String>>(rest).map_err(|e| bad(e.to_string()))
                } else if windows {
                    Ok(vec!["cmd".into(), "/S".into(), "/C".into(), rest.to_string()])
                } else {
                    Ok(vec!["/bin/sh".into(), "-c".into(), rest.to_string()])
                }
            };
            match instruction.to_ascii_uppercase().as_str() {
                "CMD" => config.cmd = argv()?,
                "ENTRYPOINT" => config.entrypoint = argv()?,
                "WORKDIR" => config.workdir = rest.to_string(),
                "USER" => config.user = rest.to_string(),
                "ENV" => {
                    let (k, v) = rest.split_once(['=', ' ']).ok_or_else(|| bad("ENV NAME=VALUE".into()))?;
                    config.env.retain(|e| e.split('=').next() != Some(k));
                    config.env.push(format!("{k}={}", v.trim_matches('"')));
                }
                other => return Err(bad(format!("{other} isn't CMD, ENTRYPOINT, ENV, WORKDIR or USER"))),
            }
        }
        if windows {
            self.images.import(name, tar, &platform, &config, |_| atlas::Meta::default(), atlas::Meta { attributes: 0x10 })
        } else {
            self.images.import(name, tar, &platform, &config, lambda::Meta::from, lambda::Meta::dir(0o755))
        }
        .map_err(error::image)
    }

    /// Pulls an image for `platform` (Linux for this host's CPU, if not said).
    pub fn pull(&self, name: &str, platform: &Platform, progress: &mut dyn FnMut(&images::PullProgress)) -> Result<Image> {
        if platform.os == "windows" {
            self.images.pull(name, platform, |_| atlas::Meta::default(), atlas::Meta { attributes: 0x10 }, progress)
        } else {
            self.images.pull(name, platform, lambda::Meta::from, lambda::Meta::dir(0o755), progress)
        }
        .map_err(error::image)
    }

    /// Freezes a stopped machine's disk, as it is, into an image called `image`.
    pub fn freeze(&self, name: &str, image: &str) -> Result<Image> {
        let record = self.stopped(name)?;
        let index = match record.kernel {
            Kernel::Lambda => self.disk::<lambda::Meta>(&record, lambda::Meta::dir(0o755), false)?.freeze(),
            Kernel::Atlas => self.disk::<atlas::Meta>(&record, atlas::Meta { attributes: 0x10 }, true)?.freeze(),
        }
        .map_err(|e| Error::io(format!("freezing {name}: {e}")))?;
        self.images.save(image, &index, &record.platform, &record.config, &format!("frozen from machine {name}")).map_err(error::image)
    }

    /// Copies a stopped machine's file, or folder and all in it, at `path` (as the machine names it: `/etc/hosts`, or
    /// `C:\\Program Files\\App` on Windows) into the host's folder `to`, under its own name. How many files it copied.
    /// Symbolic links are left out.
    pub fn copy_out(&self, name: &str, path: &str, to: &Path) -> Result<usize> {
        let record = self.stopped(name)?;
        match record.kernel {
            Kernel::Lambda => copy_tree(&self.disk::<lambda::Meta>(&record, lambda::Meta::dir(0o755), false)?, path, to),
            Kernel::Atlas => copy_tree(&self.disk::<atlas::Meta>(&record, atlas::Meta { attributes: 0x10 }, true)?, &windows_path(path), to),
        }
    }

    /// Copies the host's file, or folder and all in it, at `from` into a stopped machine's folder `path` (as
    /// `copy_out` names it), under its own name; a `from` that ends in `/.` copies what's in the folder instead. A
    /// file already there is replaced, a folder already there is added to. How many files it copied.
    pub fn copy_in(&self, name: &str, from: &Path, path: &str) -> Result<usize> {
        let record = self.stopped(name)?;
        let contents = from.as_os_str().to_string_lossy().ends_with("/.");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        match record.kernel {
            Kernel::Lambda => {
                let mut disk = self.disk::<lambda::Meta>(&record, lambda::Meta::dir(0o755), false)?;
                let metas = (lambda::Meta::dir(0o755), lambda::Meta::file(0o644));
                let n = copy_tree_in(&mut disk, from, path, contents, &metas, now)?;
                disk.sync().map_err(|e| Error::io(format!("saving {name}'s disk: {e}")))?;
                Ok(n)
            }
            Kernel::Atlas => {
                let mut disk = self.disk::<atlas::Meta>(&record, atlas::Meta { attributes: 0x10 }, true)?;
                // FILE_ATTRIBUTE_DIRECTORY and FILE_ATTRIBUTE_ARCHIVE, as atlas makes them.
                let metas = (atlas::Meta { attributes: 0x10 }, atlas::Meta { attributes: 0x20 });
                let n = copy_tree_in(&mut disk, from, &windows_path(path), contents, &metas, now)?;
                disk.sync().map_err(|e| Error::io(format!("saving {name}'s disk: {e}")))?;
                Ok(n)
            }
        }
    }

    /// A machine's disk: its image's index, if it has one, under its own log.
    pub fn disk<M: Persist>(&self, record: &Record, root: M, fold: bool) -> Result<Fs<M>> {
        let index = match &record.index {
            Some(hash) => {
                let hash = images::store::parse_hash(hash).ok_or_else(|| Error::io("the machine's record is corrupt"))?;
                let bytes = layers::Store::get(&mut self.images.store().map_err(error::image)?, &hash).map_err(|_| error::image(format!("{}'s image index is missing from the store", record.name)))?;
                Some(bytes)
            }
            None => None,
        };
        Fs::open(index.as_deref(), Box::new(self.images.machine_store(&record.name).map_err(error::image)?), root, fold).map_err(|e| Error::io(format!("opening {}'s disk: {e}", record.name)))
    }

    /// A name for a new machine, from what it's made from: `debian`, `debian-2`, …
    pub fn new_name(&self, base: &str) -> String {
        let base: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
        let base = if base.is_empty() { "machine".to_string() } else { base };
        (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|name| self.record(name).is_none()).unwrap_or(base)
    }

    /// A new machine's record from its source, and a program's bytes, to put on its disk.
    pub fn create(&self, name: Option<&str>, platform: Option<&Platform>, source: &Source, notice: &dyn Fn(Notice)) -> Result<(Record, Option<Vec<u8>>)> {
        if let Some(name) = name
            && (name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.'))
        {
            return Err(Error::invalid(format!("{name:?} can't be a machine's name: letters, digits, - and _")));
        }
        let record = |name: String, kernel, image, index, program, platform, config| Record {
            name,
            kernel,
            image,
            index,
            program,
            platform,
            config,
            created: now(),
            pid: None,
            backend: String::new(),
            network: Network::Nat,
            publish: Vec::new(),
            ports: Vec::new(),
        };
        match source {
            // `windows`: atlas's system layer and nothing else, running cmd.exe (from an image that has it) unless told
            // otherwise, as a Windows container image runs it.
            Source::Image(image) if self.images.image(image).is_none() && matches!(image.to_ascii_lowercase().as_str(), "windows" | "windows:latest") => {
                let name = name.map_or_else(|| self.new_name("windows"), String::from);
                let platform = Platform { os: "windows".into(), arch: host_arch().into() };
                Ok((record(name, Kernel::Atlas, Some("windows".into()), None, None, platform, Config { cmd: vec!["cmd.exe".into()], ..Config::default() }), None))
            }
            Source::Image(wanted) => {
                // One here for another platform than the one asked for is pulled again, for that one.
                let here = self.images.image(wanted).filter(|image| platform.is_none_or(|p| *p == image.platform));
                let image = match here {
                    Some(image) => image,
                    None => {
                        let platform = platform.cloned().unwrap_or_else(Platform::linux_here);
                        notice(Notice::progress(format!("pulling {} for {platform}", images::full_name(wanted))));
                        self.pull(wanted, &platform, &mut stage_notices(&|what| notice(Notice::progress(what))))?
                    }
                };
                let kernel = if image.platform.os == "windows" { Kernel::Atlas } else { Kernel::Lambda };
                let base = image.short_name().split([':', '@']).next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                let name = name.map_or_else(|| self.new_name(&base), String::from);
                Ok((record(name, kernel, Some(image.name.clone()), Some(image.index.clone()), None, image.platform.clone(), image.config.clone()), None))
            }
            Source::Program(path) => {
                let bytes = fs::read(path).map_err(|e| Error::new(ErrorKind::NotFound, format!("{}: {e}", path.display())))?;
                let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("program").to_string();
                let (kernel, platform, program) = if bytes.starts_with(b"\x7fELF") {
                    // e_machine: 0xB7 is ARM64, 0x3E is x86-64.
                    let arch = if bytes.get(18) == Some(&0xB7) { "arm64" } else { "amd64" };
                    (Kernel::Lambda, Platform { os: "linux".into(), arch: arch.into() }, format!("/{file}"))
                } else if bytes.starts_with(b"MZ") {
                    // The PE header's machine: x86-64, 32-bit x86 (WoW64), or else ARM64.
                    let machine = |m: &[u8; 2]| bytes.windows(6).take(1024).any(|w| w[..4] == *b"PE\0\0" && w[4..] == *m);
                    let arch = if machine(b"\x64\x86") {
                        "amd64"
                    } else if machine(b"\x4c\x01") {
                        "386"
                    } else {
                        "arm64"
                    };
                    (Kernel::Atlas, Platform { os: "windows".into(), arch: arch.into() }, format!("C:\\{file}"))
                } else if file.to_ascii_lowercase().ends_with(".msi") {
                    return Err(Error::unsupported("installing an .msi needs msiexec, which isn't there yet"));
                } else {
                    return Err(Error::invalid(format!("{} isn't a program this knows: not an ELF or a PE", path.display())));
                };
                let stem = file.rsplit_once('.').map_or(file.as_str(), |(stem, _)| stem).to_string();
                let name = name.map_or_else(|| self.new_name(&stem), String::from);
                Ok((record(name, kernel, None, None, Some(program), platform, Config::default()), Some(bytes)))
            }
        }
    }
}

/// A Windows path on a machine's one drive, as its disk has it.
fn windows_path(path: &str) -> String {
    path.strip_prefix("C:").or_else(|| path.strip_prefix("c:")).unwrap_or(path).replace('\\', "/")
}

/// `path` on `disk`, and all in it, copied into the host's folder `to`. How many files.
pub(crate) fn copy_tree<M: Persist>(disk: &Fs<M>, path: &str, to: &Path) -> Result<usize> {
    let ino = disk.resolve(ROOT, path, false).map_err(|e| Error::not_found(format!("{path}: {e:?}")))?;
    let name = path.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("root");
    fn walk<M: Persist>(disk: &Fs<M>, ino: layers::Ino, at: &Path) -> Result<usize> {
        match disk.entries(ino) {
            Ok(entries) => {
                fs::create_dir_all(at).map_err(|e| Error::io(format!("{}: {e}", at.display())))?;
                let mut n = 0;
                for (name, child, kind) in entries {
                    match kind {
                        layers::Kind::Symlink => {}
                        _ => n += walk(disk, child, &at.join(&name))?,
                    }
                }
                Ok(n)
            }
            Err(_) => {
                let bytes = disk.contents(ino).map_err(|e| Error::io(format!("{}: {e:?}", at.display())))?;
                fs::write(at, bytes).map_err(|e| Error::io(format!("{}: {e}", at.display())))?;
                Ok(1)
            }
        }
    }
    walk(disk, ino, &to.join(name))
}

/// The host's `from`, and all in it, copied into the folder `path` on `disk` (or what's in `from`, if `contents`),
/// folders and files made with `metas`. How many files. Symbolic links are left out.
pub(crate) fn copy_tree_in<M: Persist + Clone>(disk: &mut Fs<M>, from: &Path, path: &str, contents: bool, metas: &(M, M), now: u64) -> Result<usize> {
    let into = disk.resolve(ROOT, path, true).map_err(|e| Error::not_found(format!("{path}: {e:?}")))?;
    fn walk<M: Persist + Clone>(disk: &mut Fs<M>, from: &Path, dir: layers::Ino, name: Option<&str>, metas: &(M, M), now: u64) -> Result<usize> {
        let at = |e: &dyn std::fmt::Display| Error::io(format!("{}: {e}", from.display()));
        let kind = fs::symlink_metadata(from).map_err(|e| at(&e))?.file_type();
        if kind.is_symlink() {
            return Ok(0);
        }
        // Where it goes: the folder itself, or a new or existing entry in it.
        let place = |disk: &mut Fs<M>, new: layers::New, meta: &M| -> Result<layers::Ino> {
            let Some(name) = name else { return Ok(dir) };
            match disk.resolve(dir, name, false) {
                Ok(ino) => Ok(ino),
                Err(_) => disk.create(dir, name, new, meta.clone(), now).map_err(|e| at(&e)),
            }
        };
        if kind.is_dir() {
            let ino = place(disk, layers::New::Dir, &metas.0)?;
            let mut n = 0;
            let mut entries: Vec<_> = fs::read_dir(from).map_err(|e| at(&e))?.filter_map(std::result::Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let child = entry.file_name().to_string_lossy().into_owned();
                n += walk(disk, &entry.path(), ino, Some(&child), metas, now)?;
            }
            return Ok(n);
        }
        let ino = place(disk, layers::New::File, &metas.1)?;
        disk.truncate(ino, 0, now).map_err(|e| at(&e))?;
        let mut file = fs::File::open(from).map_err(|e| at(&e))?;
        let mut buffer = vec![0u8; 4 << 20];
        let mut offset = 0u64;
        loop {
            let got = std::io::Read::read(&mut file, &mut buffer).map_err(|e| at(&e))?;
            if got == 0 {
                break;
            }
            disk.write(ino, offset, &buffer[..got], now).map_err(|e| at(&e))?;
            offset += got as u64;
        }
        Ok(1)
    }
    if contents {
        walk(disk, from, into, None, metas, now)
    } else {
        let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| Error::invalid(format!("{}: no name to copy it in as", from.display())))?;
        walk(disk, from, into, Some(&name), metas, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let record = Record {
            name: "dev".into(),
            kernel: Kernel::Lambda,
            image: Some("docker.io/library/debian:13".into()),
            index: Some("ab".repeat(32)),
            program: None,
            platform: Platform::linux_here(),
            config: Config { cmd: vec!["bash".into()], env: vec!["PATH=/bin".into()], ..Config::default() },
            created: 5,
            pid: Some(42),
            backend: "vm/hvf".into(),
            network: Network::Host,
            publish: vec!["2222:22".into(), "127.0.0.1:5353:53/udp".into()],
            ports: vec!["0.0.0.0:2222->22/tcp".into()],
        };
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
    }

    #[test]
    fn host_folders_copy_in_and_out_again() {
        let host = std::env::temp_dir().join(format!("carbon-copy-in-{}", std::process::id()));
        let _ = fs::remove_dir_all(&host);
        fs::create_dir_all(host.join("app/sub")).unwrap();
        fs::write(host.join("app/a.txt"), b"one").unwrap();
        fs::write(host.join("app/sub/b.bin"), vec![7u8; 5 << 20]).unwrap();
        let mut disk = lambda::Disk::new(lambda::Meta::dir(0o755), false);
        disk.put("/opt/a.txt", layers::New::File, b"older and longer", lambda::Meta::file(0o644), lambda::Meta::dir(0o755)).unwrap();
        let metas = (lambda::Meta::dir(0o755), lambda::Meta::file(0o644));
        // The folder by name, then what's in it over a file already there.
        assert_eq!(copy_tree_in(&mut disk, &host.join("app"), "/", false, &metas, 1).unwrap(), 2);
        assert_eq!(copy_tree_in(&mut disk, &host.join("app/."), "/opt", true, &metas, 1).unwrap(), 2);
        assert_eq!(disk.contents(disk.resolve(ROOT, "/app/a.txt", false).unwrap()).unwrap(), b"one");
        assert_eq!(disk.contents(disk.resolve(ROOT, "/opt/a.txt", false).unwrap()).unwrap(), b"one");
        assert_eq!(disk.contents(disk.resolve(ROOT, "/opt/sub/b.bin", false).unwrap()).unwrap(), vec![7u8; 5 << 20]);
        let out = host.join("out");
        assert_eq!(copy_tree(&disk, "/app", &out).unwrap(), 2);
        assert_eq!(fs::read(out.join("app/sub/b.bin")).unwrap().len(), 5 << 20);
        let _ = fs::remove_dir_all(&host);
    }

    /// Chunks a removed machine wrote, and a removed image's, go; a kept image's and a kept machine's stay, readable.
    #[test]
    fn prune_frees_only_what_nothing_names() {
        let home = std::env::temp_dir().join(format!("carbon-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let store = Store::open(Some(&home)).unwrap();
        let (dir, file) = (lambda::Meta::dir(0o755), lambda::Meta::file(0o644));
        // Each its own contents, bigger than an index keeps inline, so each is chunks in the store.
        let contents = |byte: u8| vec![byte; 300 << 10];
        let image = |name: &str, byte: u8| {
            let mut disk = layers::Fs::building(dir, false, Box::new(store.images.store().unwrap()));
            disk.put("/data.bin", layers::New::File, &contents(byte), file, dir).unwrap();
            let index = disk.freeze().unwrap();
            store.images.save(name, &index, &Platform::linux_here(), &Config::default(), "test").unwrap()
        };
        image("kept:1", 1);
        image("gone:1", 2);
        let machine = |name: &str, byte: u8| {
            let (record, _) = store.create(Some(name), None, &Source::Image("kept:1".into()), &|_| {}).unwrap();
            store.save_record(&record).unwrap();
            let mut disk = store.disk::<lambda::Meta>(&record, dir, false).unwrap();
            disk.put("/written.bin", layers::New::File, &contents(byte), file, dir).unwrap();
            disk.sync().unwrap();
            record
        };
        let kept = machine("keeps", 3);
        machine("goes", 4);
        store.remove("goes").unwrap();
        store.images.remove("gone:1").unwrap();

        // Nothing written in the last quarter hour goes: all of it is that young now.
        assert_eq!(store.prune().unwrap(), Pruned::default());
        // An hour later, by the files' clocks.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for shard in fs::read_dir(home.join("chunks")).unwrap().flatten() {
            for chunk in fs::read_dir(shard.path()).unwrap().flatten() {
                fs::File::options().write(true).open(chunk.path()).unwrap().set_modified(old).unwrap();
            }
        }
        let pruned = store.prune().unwrap();
        assert!(pruned.chunks >= 2 && pruned.bytes >= 2 * (300 << 10) / 64, "{pruned:?}");
        let has = |bytes: &[u8]| images::store::DiskStore::chunks(home.join("chunks")).unwrap().has(&images::store::sha256(bytes));
        let chunk = |byte: u8| vec![byte; layers::CHUNK as usize];
        assert!(has(&chunk(1)) && has(&chunk(3)), "the kept image's and the kept machine's chunks stay");
        assert!(!has(&chunk(2)) && !has(&chunk(4)), "the removed image's and the removed machine's chunks go");
        let disk = store.disk::<lambda::Meta>(&kept, dir, false).unwrap();
        assert_eq!(disk.contents(disk.resolve(ROOT, "/written.bin", false).unwrap()).unwrap(), contents(3));
        assert_eq!(disk.contents(disk.resolve(ROOT, "/data.bin", false).unwrap()).unwrap(), contents(1));
        // Pruning again finds nothing more.
        assert_eq!(store.prune().unwrap(), Pruned::default());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn names_are_checked_and_made() {
        let home = std::env::temp_dir().join(format!("carbon-store-{}", std::process::id()));
        let store = Store::open(Some(&home)).unwrap();
        assert_eq!(store.new_name("Debian 13"), "debian-13");
        let bad = store.create(Some("../x"), None, &Source::Image("windows".into()), &|_| {});
        assert_eq!(bad.unwrap_err().kind(), ErrorKind::InvalidInput);
        let (record, _) = store.create(None, None, &Source::Image("windows".into()), &|_| {}).unwrap();
        assert_eq!((record.name.as_str(), record.kernel), ("windows", Kernel::Atlas));
        let _ = fs::remove_dir_all(&home);
    }
}
