//! Images on the host's disk: what a machine starts from. An image is an index (its layers, merged: see `layers`) and
//! what its config says to run, by name. Images are pulled from OCI registries over HTTPS, imported from a tar of a root
//! file system, or saved from a machine's frozen disk. Every chunk of every image and machine is in one store, by its
//! SHA-256, so what images share is kept once.
//!
//! ```text
//! <home>/chunks/…                 the store (store.rs), indexes included
//! <home>/images/<registry>/<repository>/<tag>.json    an image's record
//! <home>/machines/<name>/log      a machine's writable layer
//! ```
//!
//! `<home>` is `CARBON_HOME`, or `.carbon` in the user's home folder. Nothing here knows a kernel: a kernel's `M` is made
//! from each tar entry by the caller.

mod pull;
pub mod store;
mod tls;

use std::fs;
use std::path::{Path, PathBuf};

use layers::{Persist, Store};
use serde_json::{Value, json};

pub use layers::tar::Entry;
pub use store::DiskStore;

/// What an image's config says about running it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub env: Vec<String>,
    pub entrypoint: Vec<String>,
    pub cmd: Vec<String>,
    pub workdir: String,
    pub user: String,
}

/// Which build of an image: `linux/arm64`, `linux/amd64`, `windows/amd64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

impl Platform {
    /// Linux, for this host's CPU, which runs natively.
    pub fn linux_here() -> Platform {
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
        Platform { os: "linux".into(), arch: arch.into() }
    }

    /// `os/arch`, or just `arch` meaning Linux.
    pub fn parse(text: &str) -> Option<Platform> {
        let (os, arch) = text.split_once('/').unwrap_or(("linux", text));
        let arch = match arch {
            "arm64" | "aarch64" => "arm64",
            "amd64" | "x86_64" | "x86-64" => "amd64",
            _ => return None,
        };
        matches!(os, "linux" | "windows").then(|| Platform { os: os.into(), arch: arch.into() })
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.os, self.arch)
    }
}

/// An image, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// In full: `docker.io/library/debian:13`.
    pub name: String,
    /// The index's SHA-256, in hex: its name in the store.
    pub index: String,
    pub platform: Platform,
    pub config: Config,
    /// Seconds since 1970.
    pub created: u64,
    /// Where it came from: the registry, a tar, a machine.
    pub source: String,
}

impl Image {
    /// The name as people write it: `debian:13` for one from Docker Hub's library.
    pub fn short_name(&self) -> &str {
        self.name.strip_prefix("docker.io/library/").or_else(|| self.name.strip_prefix("docker.io/")).unwrap_or(&self.name)
    }
}

/// A name in full: `debian` is `docker.io/library/debian:latest`, `ghcr.io/org/app:1` is itself.
pub fn full_name(name: &str) -> String {
    let (first, rest) = name.split_once('/').unwrap_or(("", name));
    let has_registry = first.contains('.') || first.contains(':') || first == "localhost";
    let (registry, path) = if has_registry { (first, rest) } else { ("docker.io", name) };
    // A tag is after the last `:` past the last `/`, so a registry's port isn't taken for one.
    let tail_start = path.rfind('/').map_or(0, |i| i + 1);
    let (path, tag) = match path[tail_start..].rfind(':') {
        Some(at) => (&path[..tail_start + at], &path[tail_start + at + 1..]),
        None => (path, "latest"),
    };
    let path = if registry == "docker.io" && !path.contains('/') { format!("library/{path}") } else { path.to_string() };
    format!("{registry}/{path}:{tag}")
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The images on this host, and the store they're in.
pub struct Images {
    home: PathBuf,
}

impl Images {
    /// The images in `home`, or in `CARBON_HOME`, or in `.carbon` in the user's home folder.
    pub fn open(home: Option<&Path>) -> Result<Images, String> {
        let home = match home {
            Some(home) => home.to_path_buf(),
            None => match std::env::var_os("CARBON_HOME") {
                Some(home) => PathBuf::from(home),
                None => {
                    let user = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).ok_or("no home folder to keep images in (set CARBON_HOME)")?;
                    PathBuf::from(user).join(".carbon")
                }
            },
        };
        fs::create_dir_all(home.join("images")).map_err(|e| format!("{}: {e}", home.display()))?;
        Ok(Images { home })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The store, for chunks only.
    pub fn store(&self) -> Result<DiskStore, String> {
        DiskStore::chunks(self.home.join("chunks")).map_err(|e| format!("the store: {e}"))
    }

    /// The store, with a machine's log: that machine's disk, over its image.
    pub fn machine_store(&self, machine: &str) -> Result<DiskStore, String> {
        DiskStore::with_log(self.home.join("chunks"), &self.machine_dir(machine).join("log")).map_err(|e| format!("machine {machine}'s disk: {e}"))
    }

    /// Where a machine keeps what's its own.
    pub fn machine_dir(&self, machine: &str) -> PathBuf {
        self.home.join("machines").join(machine)
    }

    fn record_path(&self, full: &str) -> PathBuf {
        // The tag is the file; everything before it, folders. A digest's `:` can't be in a file name everywhere.
        let (path, tag) = full.rsplit_once(':').unwrap_or((full, "latest"));
        let mut out = self.home.join("images");
        for part in path.split('/') {
            out.push(part.replace(':', "_"));
        }
        out.push(format!("{}.json", tag.replace(':', "_")));
        out
    }

    /// An image by name.
    pub fn image(&self, name: &str) -> Option<Image> {
        let text = fs::read_to_string(self.record_path(&full_name(name))).ok()?;
        Self::parse_record(&text)
    }

    fn parse_record(text: &str) -> Option<Image> {
        let v: Value = serde_json::from_str(text).ok()?;
        let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let platform = Platform::parse(&s("platform"))?;
        let config = v.get("config");
        Some(Image {
            name: s("name"),
            index: s("index"),
            platform,
            config: Config {
                env: strings(config.and_then(|c| c.get("env"))),
                entrypoint: strings(config.and_then(|c| c.get("entrypoint"))),
                cmd: strings(config.and_then(|c| c.get("cmd"))),
                workdir: config.and_then(|c| c.get("workdir")).and_then(Value::as_str).unwrap_or_default().to_string(),
                user: config.and_then(|c| c.get("user")).and_then(Value::as_str).unwrap_or_default().to_string(),
            },
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
            source: s("source"),
        })
    }

    /// Every image, by name.
    pub fn images(&self) -> Vec<Image> {
        let mut out = Vec::new();
        let mut stack = vec![self.home.join("images")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "json")
                    && let Some(image) = fs::read_to_string(&path).ok().as_deref().and_then(Self::parse_record)
                {
                    out.push(image);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Forgets an image's name. Its chunks stay, for whatever else uses them.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        let full = full_name(name);
        fs::remove_file(self.record_path(&full)).map_err(|_| format!("there's no image {full}"))
    }

    /// An image's index, from the store.
    pub fn index(&self, image: &Image) -> Result<Vec<u8>, String> {
        let hash = store::parse_hash(&image.index).ok_or_else(|| format!("{}'s record is corrupt", image.name))?;
        self.store()?.get(&hash).map_err(|_| format!("{}'s index is missing from the store", image.name))
    }

    /// Names an index: puts it in the store and writes the image's record. The name's only taken once it's all there.
    pub fn save(&self, name: &str, index: &[u8], platform: &Platform, config: &Config, source: &str) -> Result<Image, String> {
        let hash = self.store()?.put(index).map_err(|_| "putting the index in the store".to_string())?;
        let image = Image { name: full_name(name), index: store::hex(&hash), platform: platform.clone(), config: config.clone(), created: now(), source: source.into() };
        let record = json!({
            "name": image.name,
            "index": image.index,
            "platform": image.platform.to_string(),
            "created": image.created,
            "source": image.source,
            "config": {
                "env": image.config.env,
                "entrypoint": image.config.entrypoint,
                "cmd": image.config.cmd,
                "workdir": image.config.workdir,
                "user": image.config.user,
            },
        });
        let path = self.record_path(&image.name);
        let dir = path.parent().ok_or("a bad image name")?;
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, serde_json::to_vec_pretty(&record).unwrap_or_default()).map_err(|e| format!("{}: {e}", temporary.display()))?;
        fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(image)
    }

    /// An image from a tar of a root file system (`docker export`, for one): each entry's `M` made by `meta`.
    pub fn import<M: Persist>(&self, name: &str, tar: &[u8], platform: &Platform, config: &Config, meta: impl FnMut(Entry) -> M, root: M) -> Result<Image, String> {
        let mut disk = layers::Fs::building(root, platform.os == "windows", Box::new(self.store()?));
        layers::tar::import(&mut disk, tar, meta).map_err(|e| format!("reading the tar: {e}"))?;
        let index = disk.freeze().map_err(|e| format!("writing the index: {e}"))?;
        self.save(name, &index, platform, config, "imported")
    }

    /// Pulls an image from its registry for `platform`, merges its layers into an index, and names it. Each tar
    /// entry's `M` is made by `meta`, the root's is `root`. Nothing is named until every blob has matched its digest.
    pub fn pull<M: Persist>(&self, name: &str, platform: &Platform, meta: impl FnMut(Entry) -> M, root: M, progress: &mut dyn FnMut(&str)) -> Result<Image, String> {
        pull::pull(self, name, platform, meta, root, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_full() {
        assert_eq!(full_name("debian:13"), "docker.io/library/debian:13");
        assert_eq!(full_name("nginx"), "docker.io/library/nginx:latest");
        assert_eq!(full_name("someone/thing:v2"), "docker.io/someone/thing:v2");
        assert_eq!(full_name("ghcr.io/org/app:1.0"), "ghcr.io/org/app:1.0");
        assert_eq!(full_name("localhost:5000/app"), "localhost:5000/app:latest");
    }

    #[test]
    fn platforms() {
        assert_eq!(Platform::parse("linux/arm64"), Some(Platform { os: "linux".into(), arch: "arm64".into() }));
        assert_eq!(Platform::parse("amd64").map(|p| p.to_string()).as_deref(), Some("linux/amd64"));
        assert_eq!(Platform::parse("linux/s390x"), None);
    }

    #[test]
    fn imported_saved_and_found_again() {
        let home = std::env::temp_dir().join(format!("carbon-images-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let images = Images::open(Some(&home)).unwrap();
        // A tar with one file: `etc/hostname`.
        let mut tar = vec![0u8; 512];
        tar[..12].copy_from_slice(b"etc/hostname");
        tar[100..107].copy_from_slice(b"0000644");
        tar[124..135].copy_from_slice(b"00000000005");
        tar[156] = b'0';
        tar[257..262].copy_from_slice(b"ustar");
        let sum: u32 = tar.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { u32::from(*b) }).sum();
        tar[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        tar.extend_from_slice(b"box\n\0");
        tar.resize(512 * 2, 0);
        tar.resize(512 * 4, 0);
        let config = Config { cmd: vec!["/bin/sh".into()], ..Config::default() };
        let image = images.import("box:1", &tar, &Platform::linux_here(), &config, |e| e.mode, 0o755u32).unwrap();
        assert_eq!(image.short_name(), "box:1");
        let found = images.image("box:1").unwrap();
        assert_eq!(found, image);
        assert_eq!(images.images(), vec![image.clone()]);
        let index = images.index(&found).unwrap();
        let disk: layers::Fs<u32> = layers::Fs::open(Some(&index), Box::new(images.machine_store("m").unwrap()), 0, false).unwrap();
        let file = disk.resolve(layers::ROOT, "/etc/hostname", true).unwrap();
        assert_eq!(disk.contents(file).unwrap(), b"box\n\0");
        assert_eq!(*disk.meta(file).unwrap(), 0o644);
        images.remove("box:1").unwrap();
        assert!(images.image("box:1").is_none());
        fs::remove_dir_all(&home).unwrap();
    }
}
