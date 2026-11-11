//! `machine`: puts one machine together and owns its life. A machine is a kernel (lambda for a Linux image or an ELF,
//! atlas for a Windows image or a PE), on exec with the isolation asked for, over a disk that's its image's index plus
//! its own writable layer, with a display and the host's services. It runs in the process that asked for it (hvf
//! allows one VM per process, and a machine is one), relaying stdin, stdout, signals and its exit code.
//!
//! A machine is kept by name, with its record and its log in `<home>/machines/<name>/`, so it can be started again
//! with its files as it left them, stopped from another process, frozen into an image, or removed.

mod control;
#[cfg(unix)]
mod signals;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use exec::{Cpu, Exec, Options as ExecOptions, Tuning};
use images::{Config, Image, Images, Platform};
use layers::{Fs, Persist, ROOT};
use serde_json::{Value, json};

pub use exec::Isolation;

/// What a machine starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An image by name, pulled if it isn't here.
    Image(String),
    /// A program on the host: an ELF for lambda, a PE for atlas, put on an empty disk.
    Program(PathBuf),
}

/// Where a machine's windows go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// A screen in memory this big, for screenshots.
    Headless(i32, i32),
    None,
}

/// Which kernel a machine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kernel {
    Lambda,
    Atlas,
}

impl Kernel {
    fn name(self) -> &'static str {
        match self {
            Kernel::Lambda => "lambda",
            Kernel::Atlas => "atlas",
        }
    }
}

/// What to run.
#[derive(Debug, Clone)]
pub struct Run {
    /// Its name: a new machine's, or one to start again. A new one without one is named after its image or program.
    pub name: Option<String>,
    /// What a new machine starts from. None starts the machine called `name` again.
    pub source: Option<Source>,
    /// The program and its arguments; empty for what the image says (its entrypoint and command).
    pub args: Vec<String>,
    pub isolation: Isolation,
    /// For an image that isn't here yet: which build to pull. Linux for this host's CPU, if not said.
    pub platform: Option<Platform>,
    /// `NAME=value`, over the image's.
    pub env: Vec<String>,
    pub display: Display,
    /// How much memory it may use (only what's touched is taken).
    pub memory: u64,
}

impl Default for Run {
    fn default() -> Run {
        Run { name: None, source: None, args: Vec::new(), isolation: Isolation::Auto, platform: None, env: Vec::new(), display: Display::Headless(1280, 800), memory: 4 << 30 }
    }
}

/// A machine, as kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
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
        })
    }

    /// Whether its process is running now.
    pub fn running(&self) -> bool {
        self.pid.is_some_and(alive)
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    pid != 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

#[cfg(not(unix))]
fn alive(_pid: u32) -> bool {
    false
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The machines on this host.
pub struct Machines {
    images: Images,
}

impl Machines {
    pub fn open(home: Option<&Path>) -> Result<Machines, String> {
        Ok(Machines { images: Images::open(home)? })
    }

    pub fn images(&self) -> &Images {
        &self.images
    }

    fn record_path(&self, name: &str) -> PathBuf {
        self.images.machine_dir(name).join("machine.json")
    }

    pub fn record(&self, name: &str) -> Option<Record> {
        let text = fs::read_to_string(self.record_path(name)).ok()?;
        Record::from_json(&serde_json::from_str(&text).ok()?)
    }

    fn save_record(&self, record: &Record) -> Result<(), String> {
        let path = self.record_path(&record.name);
        fs::create_dir_all(path.parent().ok_or("a bad machine name")?).map_err(|e| format!("{}: {e}", path.display()))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, serde_json::to_vec_pretty(&record.to_json()).unwrap_or_default()).map_err(|e| format!("{}: {e}", temporary.display()))?;
        fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))
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

    /// Asks a running machine to stop, as `docker stop` does: SIGTERM to its first process, and everything ended ten
    /// seconds later if it's still going.
    pub fn stop(&self, name: &str) -> Result<(), String> {
        let record = self.record(name).ok_or_else(|| format!("there's no machine called {name}"))?;
        let pid = record.pid.filter(|pid| alive(*pid)).ok_or_else(|| format!("{name} isn't running"))?;
        #[cfg(unix)]
        {
            // SAFETY: asking a process to end.
            if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
                return Err(format!("couldn't signal {name}'s process {pid}"));
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Err("stopping a machine from another process isn't there yet on Windows hosts".into())
        }
    }

    /// Removes a stopped machine: its record and its writable layer. Its image stays.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let record = self.record(name).ok_or_else(|| format!("there's no machine called {name}"))?;
        if record.running() {
            return Err(format!("{name} is running: stop it first"));
        }
        fs::remove_dir_all(self.images.machine_dir(name)).map_err(|e| format!("removing {name}: {e}"))
    }

    /// The running machine's screen, as a PNG.
    pub fn screenshot(&self, name: &str) -> Result<Vec<u8>, String> {
        let record = self.record(name).ok_or_else(|| format!("there's no machine called {name}"))?;
        if !record.running() {
            return Err(format!("{name} isn't running"));
        }
        control::ask(&self.images.machine_dir(name), "screenshot")
    }

    /// Freezes a stopped machine's disk, as it is, into an image called `image`.
    pub fn freeze(&self, name: &str, image: &str) -> Result<Image, String> {
        let record = self.record(name).ok_or_else(|| format!("there's no machine called {name}"))?;
        if record.running() {
            return Err(format!("{name} is running: stop it first"));
        }
        let index = match record.kernel {
            Kernel::Lambda => self.disk::<lambda::Meta>(&record, lambda::Meta::dir(0o755), false)?.freeze(),
            Kernel::Atlas => self.disk::<atlas::Meta>(&record, atlas::Meta { attributes: 0x10 }, true)?.freeze(),
        }
        .map_err(|e| format!("freezing {name}: {e}"))?;
        self.images.save(image, &index, &record.platform, &record.config, &format!("frozen from machine {name}"))
    }

    /// A machine's disk: its image's index, if it has one, under its own log.
    fn disk<M: Persist>(&self, record: &Record, root: M, fold: bool) -> Result<Fs<M>, String> {
        let index = match &record.index {
            Some(hash) => {
                let hash = images::store::parse_hash(hash).ok_or("the machine's record is corrupt")?;
                let bytes = layers::Store::get(&mut self.images.store()?, &hash).map_err(|_| format!("{}'s image index is missing from the store", record.name))?;
                Some(bytes)
            }
            None => None,
        };
        Fs::open(index.as_deref(), Box::new(self.images.machine_store(&record.name)?), root, fold).map_err(|e| format!("opening {}'s disk: {e}", record.name))
    }

    /// A name for a new machine, from what it's made from: `debian`, `debian-2`, …
    fn new_name(&self, base: &str) -> String {
        let base: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' }).collect();
        let base = if base.is_empty() { "machine".to_string() } else { base };
        (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|name| self.record(name).is_none()).unwrap_or(base)
    }

    /// A new machine's record, from its source.
    fn create(&self, run: &Run, source: &Source, progress: &mut dyn FnMut(&str)) -> Result<(Record, Option<Vec<u8>>), String> {
        match source {
            Source::Image(name) => {
                // One here for another platform than the one asked for is pulled again, for that one.
                let here = self.images.image(name).filter(|image| run.platform.as_ref().is_none_or(|p| *p == image.platform));
                let image = match here {
                    Some(image) => image,
                    None => {
                        let platform = run.platform.clone().unwrap_or_else(Platform::linux_here);
                        progress(&format!("pulling {} for {platform}", images::full_name(name)));
                        if platform.os == "windows" {
                            self.images.pull(name, &platform, |_| atlas::Meta::default(), atlas::Meta { attributes: 0x10 }, progress)?
                        } else {
                            self.images.pull(name, &platform, lambda::Meta::from, lambda::Meta::dir(0o755), progress)?
                        }
                    }
                };
                let kernel = if image.platform.os == "windows" { Kernel::Atlas } else { Kernel::Lambda };
                let base = image.short_name().split([':', '@']).next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                let record = Record {
                    name: run.name.clone().unwrap_or_else(|| self.new_name(&base)),
                    kernel,
                    image: Some(image.name.clone()),
                    index: Some(image.index.clone()),
                    program: None,
                    platform: image.platform.clone(),
                    config: image.config.clone(),
                    created: now(),
                    pid: None,
                    backend: String::new(),
                };
                Ok((record, None))
            }
            Source::Program(path) => {
                let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("program").to_string();
                let (kernel, platform, program) = if bytes.starts_with(b"\x7fELF") {
                    // e_machine: 0xB7 is ARM64, 0x3E is x86-64.
                    let arch = if bytes.get(18) == Some(&0xB7) { "arm64" } else { "amd64" };
                    (Kernel::Lambda, Platform { os: "linux".into(), arch: arch.into() }, format!("/{file}"))
                } else if bytes.starts_with(b"MZ") {
                    let x86 = bytes.windows(6).take(1024).any(|w| w == b"PE\0\0\x64\x86");
                    let arch = if x86 { "amd64" } else { "arm64" };
                    (Kernel::Atlas, Platform { os: "windows".into(), arch: arch.into() }, format!("C:\\{file}"))
                } else if file.to_ascii_lowercase().ends_with(".msi") {
                    return Err("installing an .msi needs msiexec, which isn't there yet".into());
                } else {
                    return Err(format!("{} isn't a program this knows: not an ELF or a PE", path.display()));
                };
                let stem = file.rsplit_once('.').map_or(file.as_str(), |(stem, _)| stem).to_string();
                let record = Record {
                    name: run.name.clone().unwrap_or_else(|| self.new_name(&stem)),
                    kernel,
                    image: None,
                    index: None,
                    program: Some(program),
                    platform,
                    config: Config::default(),
                    created: now(),
                    pid: None,
                    backend: String::new(),
                };
                Ok((record, Some(bytes)))
            }
        }
    }

    /// Runs a machine, new or again, until its first process ends, and gives its exit code.
    pub fn run(&self, run: Run, progress: &mut dyn FnMut(&str)) -> Result<i32, String> {
        let existing = run.name.as_deref().and_then(|name| self.record(name));
        let (mut record, program) = match (existing, &run.source) {
            (Some(record), None) => (record, None),
            (Some(record), Some(_)) => return Err(format!("there's a machine called {} already: leave out what to run, to start it again", record.name)),
            (None, Some(source)) => self.create(&run, source, progress)?,
            (None, None) => return Err(format!("there's no machine called {}", run.name.unwrap_or_default())),
        };
        if record.running() {
            return Err(format!("{} is running already", record.name));
        }

        // Signals are taken by a thread of their own, from here on: block them before any thread is made.
        #[cfg(unix)]
        let relay = signals::Relay::start();

        let tso = record.platform.arch == "amd64" && cfg!(target_arch = "aarch64");
        let (exec, cpu) = open_exec(run.isolation, tso, run.memory)?;
        record.backend = exec.backend().to_string();
        record.pid = Some(std::process::id());
        self.save_record(&record)?;
        let ended = self.boot(&record, &run, exec, cpu, program, #[cfg(unix)] &relay);
        record.pid = None;
        let _ = self.save_record(&record);
        ended
    }

    fn boot(&self, record: &Record, run: &Run, exec: Exec, cpu: Cpu, program: Option<Vec<u8>>, #[cfg(unix)] relay: &signals::Relay) -> Result<i32, String> {
        let host: Arc<dyn host::Host> = Arc::new(host::Std::new());
        let mut env = record.config.env.clone();
        for var in &run.env {
            let key = var.split('=').next().unwrap_or("");
            env.retain(|e| e.split('=').next() != Some(key));
            env.push(var.clone());
        }
        let args: Vec<String> = if !run.args.is_empty() {
            run.args.clone()
        } else if let Some(program) = &record.program {
            vec![program.clone()]
        } else {
            record.config.entrypoint.iter().chain(&record.config.cmd).cloned().collect()
        };
        if args.is_empty() {
            return Err(format!("{} says nothing to run: name a program after --", record.image.as_deref().unwrap_or(&record.name)));
        }
        match record.kernel {
            Kernel::Lambda => {
                let mut disk = self.disk::<lambda::Meta>(record, lambda::Meta::dir(0o755), false)?;
                if let (Some(bytes), Some(path)) = (&program, &record.program) {
                    disk.put(path, layers::New::File, bytes, lambda::Meta::file(0o755), lambda::Meta::dir(0o755)).map_err(|e| format!("putting {path} on the disk: {e}"))?;
                }
                let program = find_program(&disk, &args[0], &env).ok_or_else(|| format!("{}: not found on the machine's disk", args[0]))?;
                let argv: Vec<&str> = std::iter::once(program.as_str()).chain(args[1..].iter().map(String::as_str)).collect();
                let stop = lambda::Stop::default();
                #[cfg(unix)]
                relay.on_signal({
                    let stop = stop.clone();
                    move |signal, first| if first { stop.signal(signal); } else { stop.end(); }
                });
                let _control = control::serve(self.images.machine_dir(&record.name), None);
                let mut options = lambda::Options { stop, ..lambda::Options::default() };
                if !env.is_empty() {
                    options.env = env;
                }
                options.hostname = record.name.clone();
                lambda::boot(exec, cpu, disk, host, &argv, options).map(lambda::Status::code)
            }
            Kernel::Atlas => {
                let mut disk = self.disk::<atlas::Meta>(record, atlas::Meta { attributes: 0x10 }, true)?;
                if let (Some(bytes), Some(path)) = (&program, &record.program) {
                    let unix = format!("/{}", path.trim_start_matches("C:\\").replace('\\', "/"));
                    disk.put(&unix, layers::New::File, bytes, atlas::Meta::default(), atlas::Meta { attributes: 0x10 }).map_err(|e| format!("putting {path} on the disk: {e}"))?;
                }
                let stop = atlas::Stop::default();
                #[cfg(unix)]
                relay.on_signal({
                    let stop = stop.clone();
                    move |_, _| {
                        stop.end(1);
                    }
                });
                let desktop = match run.display {
                    Display::Headless(width, height) => Some(display::Desktop::shared(width, height)),
                    Display::None => None,
                };
                let _control = control::serve(self.images.machine_dir(&record.name), desktop.clone());
                let env = env.iter().filter_map(|e| e.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
                let options = atlas::Options { stop, env, display: desktop, trace: std::env::var_os("CARBON_TRACE").is_some() };
                let argv: Vec<&str> = args.iter().map(String::as_str).collect();
                atlas::boot(exec, cpu, disk, host, &argv, options).map(|ended| ended.code as i32)
            }
        }
    }
}

/// Opens exec with the isolation asked for. `auto` takes a hardware VM where there is one, and says so, loudly, with
/// why, when it falls back to the sandbox.
fn open_exec(isolation: Isolation, tso: bool, memory: u64) -> Result<(Exec, Cpu), String> {
    let options = |isolation| ExecOptions { memory, isolation, tso, tuning: Tuning::default() };
    match isolation {
        Isolation::Auto => match Exec::open(options(Isolation::Vm)) {
            Ok(opened) => Ok(opened),
            Err(vm) => {
                let opened = Exec::open(options(Isolation::Sandbox)).map_err(|e| e.to_string())?;
                let why = vm.tried.first().map_or_else(|| vm.to_string(), |(name, why)| format!("{name}: {why}"));
                eprintln!("carbon: no hypervisor here ({why}), running with --isolation sandbox");
                Ok(opened)
            }
        },
        other => Exec::open(options(other)).map_err(|e| e.to_string()),
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
        };
        assert_eq!(Record::from_json(&record.to_json()), Some(record));
    }

    #[test]
    fn programs_are_found_on_the_path() {
        let mut disk = lambda::Disk::new(lambda::Meta::dir(0o755), false);
        disk.put("/usr/bin/bash", layers::New::File, b"\x7fELF", lambda::Meta::file(0o755), lambda::Meta::dir(0o755)).unwrap();
        let env = vec!["PATH=/usr/local/bin:/usr/bin".to_string()];
        assert_eq!(find_program(&disk, "bash", &env).as_deref(), Some("/usr/bin/bash"));
        assert_eq!(find_program(&disk, "zsh", &env), None);
        assert_eq!(find_program(&disk, "/bin/sh", &env).as_deref(), Some("/bin/sh"));
    }
}
