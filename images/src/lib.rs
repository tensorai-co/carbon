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

pub mod archive;
pub mod auth;
mod config;
mod pull;
pub mod push;
mod registry;
pub use pull::{Progress as PullProgress, Stage as PullStage};
pub use push::{Progress as PushProgress, Pushed, Stage as PushStage};
pub mod store;
mod tls;

use std::fs;
use std::path::{Path, PathBuf};

use layers::{Persist, Store};
use serde_json::{Value, json};

pub use layers::tar::Entry;
pub use store::DiskStore;

pub use config::{Config, Healthcheck, History, signal_number};

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
            // 32-bit x86: Windows programs run it (atlas's WoW64); lambda doesn't yet.
            "386" | "i386" | "i686" | "x86" if os == "windows" => "386",
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
    /// Each step that made it, oldest first.
    pub history: Vec<History>,
    /// What it was made from: the registry its first disk came from and that disk's layers there, and each disk it
    /// was made through since. So a push sends a layer per step, and only what's new.
    pub lineage: Lineage,
}

/// A layer as a registry has it, kept from a pull.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    /// Its blob's digest, `sha256:…`, as compressed.
    pub digest: String,
    /// Its blob's size, as compressed.
    pub size: u64,
    pub media_type: String,
    /// The digest of the tar inside, as the image's config lists it (`rootfs.diff_ids`).
    pub diff_id: String,
}

impl Layer {
    /// Whether it says all a push needs to send it again: its digest, size and what's in it.
    pub fn complete(&self) -> bool {
        !self.digest.is_empty() && self.size > 0 && !self.diff_id.is_empty() && !self.media_type.is_empty()
    }

    fn to_json(&self) -> Value {
        json!({ "digest": self.digest, "size": self.size, "media_type": self.media_type, "diff_id": self.diff_id })
    }

    fn from_json(v: &Value) -> Layer {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        // A record from before layers were kept whole has only their digests.
        match v.as_str() {
            Some(digest) => Layer { digest: digest.into(), ..Layer::default() },
            None => Layer { digest: s("digest"), size: v.get("size").and_then(Value::as_u64).unwrap_or(0), media_type: s("media_type"), diff_id: s("diff_id") },
        }
    }
}

/// A disk an image was made through: its index, and how many of the image's history entries made it, so the entries
/// after them are what made the next disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disk {
    /// Its index's SHA-256, in hex.
    pub index: String,
    pub history: usize,
}

/// Where an image's disk came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lineage {
    /// The repository its first disk was pulled from, in full (`docker.io/library/debian`); empty for one that began
    /// here (imported, loaded, from `scratch`).
    pub repository: String,
    /// That first disk's layers in the repository, oldest first.
    pub layers: Vec<Layer>,
    /// Each disk it was made through before its own, oldest first: the first is the one pulled or begun here.
    pub disks: Vec<Disk>,
}

impl Lineage {
    /// An image made from `base` by a step: `base`'s lineage, then `base`'s disk, made by its history up to the last
    /// entry that changed files. The entries after that changed only its config, so they're as much the next step's:
    /// a build's `WORKDIR` is said to change files only once the step after it makes its folder.
    pub fn after(base: &Image) -> Lineage {
        let mut lineage = base.lineage.clone();
        let made_by = base.history.iter().rposition(|h| !h.empty_layer).map_or(0, |i| i + 1);
        let made_by = made_by.max(lineage.disks.last().map_or(0, |d| d.history));
        lineage.disks.push(Disk { index: base.index.clone(), history: made_by });
        lineage
    }

    /// Whether its first disk can be sent as the registry's own layers, as they are.
    pub fn pulled(&self) -> bool {
        !self.repository.is_empty() && !self.layers.is_empty() && self.layers.iter().all(Layer::complete)
    }

    fn to_json(&self) -> Value {
        json!({
            "repository": self.repository,
            "layers": self.layers.iter().map(Layer::to_json).collect::<Vec<_>>(),
            "disks": self.disks.iter().map(|d| json!({ "index": d.index, "history": d.history })).collect::<Vec<_>>(),
        })
    }

    fn from_record(v: &Value) -> Lineage {
        match v.get("lineage") {
            Some(l) => Lineage {
                repository: l.get("repository").and_then(Value::as_str).unwrap_or_default().to_string(),
                layers: l.get("layers").and_then(Value::as_array).map(|a| a.iter().map(Layer::from_json).collect()).unwrap_or_default(),
                disks: l
                    .get("disks")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|d| Disk { index: d.get("index").and_then(Value::as_str).unwrap_or_default().to_string(), history: d.get("history").and_then(Value::as_u64).unwrap_or(0) as usize })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            // Older records: a pulled image's layers' digests alone.
            None => Lineage { layers: v.get("layers").and_then(Value::as_array).map(|a| a.iter().map(Layer::from_json).collect()).unwrap_or_default(), ..Lineage::default() },
        }
    }

    fn is_empty(&self) -> bool {
        self.repository.is_empty() && self.layers.is_empty() && self.disks.is_empty()
    }
}

impl Image {
    /// The name as people write it: `debian:13` for one from Docker Hub's library.
    pub fn short_name(&self) -> &str {
        self.name.strip_prefix("docker.io/library/").or_else(|| self.name.strip_prefix("docker.io/")).unwrap_or(&self.name)
    }
}

/// What a gzip stream holds (a pulled layer, or an archive a build adds).
pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).map_err(|e| format!("unpacking gzip: {e}"))?;
    Ok(out)
}

/// Seconds since 1970 as RFC 3339, as OCI writes a time: `2026-11-17T20:22:00Z`.
pub fn rfc3339(secs: u64) -> String {
    let days = secs / 86_400;
    let (h, m, s) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
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

/// Where unnamed images' records are, under `images/`: a name no registry has, and that listing skips.
const UNNAMED: &str = ".unnamed";

/// An unnamed image's id, from its name `sha256:<64 hex digits>`.
fn unnamed_id(name: &str) -> Option<&str> {
    name.strip_prefix("sha256:").filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()))
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

    /// An image by name, or an unnamed one by its id (`sha256:…`, from [`Images::save_unnamed`]).
    pub fn image(&self, name: &str) -> Option<Image> {
        let path = match unnamed_id(name) {
            Some(id) => self.unnamed_path(id),
            None => self.record_path(&full_name(name)),
        };
        Self::parse_record(&fs::read_to_string(path).ok()?)
    }

    fn unnamed_path(&self, id: &str) -> PathBuf {
        self.home.join("images").join(UNNAMED).join(format!("{id}.json"))
    }

    /// Keeps an image with no name, by an id made from what it is (its index, platform, config and history), as a
    /// build's steps are kept: found again by [`Images::image`] with its name, `sha256:<id>`, and never listed by
    /// [`Images::images`]. Saving the same image again gives the same one, and marks it used.
    #[allow(clippy::too_many_arguments)]
    pub fn save_unnamed(&self, index: &[u8], platform: &Platform, config: &Config, history: &[History], lineage: &Lineage, source: &str) -> Result<Image, String> {
        let hash = self.store()?.put(index).map_err(|_| "putting the index in the store".to_string())?;
        let what = json!({
            "index": store::hex(&hash),
            "platform": platform.to_string(),
            "config": config.to_json(),
            "history": history.iter().map(History::to_json).collect::<Vec<_>>(),
        });
        let id = store::hex(&store::sha256(&serde_json::to_vec(&what).unwrap_or_default()));
        if let Some(image) = self.image(&format!("sha256:{id}")) {
            self.touch_unnamed(&image.name);
            return Ok(image);
        }
        let image = Image { name: format!("sha256:{id}"), index: store::hex(&hash), platform: platform.clone(), config: config.clone(), created: now(), source: source.into(), history: history.to_vec(), lineage: lineage.clone() };
        self.write_record(&image)?;
        Ok(image)
    }

    /// Marks an unnamed image used now, so pruning keeps it a while longer.
    pub fn touch_unnamed(&self, name: &str) {
        if let Some(id) = unnamed_id(name) {
            let _ = fs::File::options().append(true).open(self.unnamed_path(id)).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        }
    }

    /// Every unnamed image, and when each was last used.
    pub fn unnamed(&self) -> Vec<(Image, std::time::SystemTime)> {
        let dir = self.home.join("images").join(UNNAMED);
        let mut out = Vec::new();
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let used = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            if let Some(image) = fs::read_to_string(entry.path()).ok().as_deref().and_then(Self::parse_record) {
                out.push((image, used));
            }
        }
        out
    }

    /// Forgets an unnamed image.
    pub fn remove_unnamed(&self, name: &str) -> Result<(), String> {
        let id = unnamed_id(name).ok_or_else(|| format!("{name} isn't an unnamed image's id"))?;
        fs::remove_file(self.unnamed_path(id)).map_err(|_| format!("there's no image {name}"))
    }

    fn parse_record(text: &str) -> Option<Image> {
        let v: Value = serde_json::from_str(text).ok()?;
        let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let platform = Platform::parse(&s("platform"))?;
        Some(Image {
            name: s("name"),
            index: s("index"),
            platform,
            config: v.get("config").map(Config::from_json).unwrap_or_default(),
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
            source: s("source"),
            history: v.get("history").and_then(Value::as_array).map(|h| h.iter().map(History::from_json).collect()).unwrap_or_default(),
            lineage: Lineage::from_record(&v),
        })
    }

    /// Every image, by name.
    pub fn images(&self) -> Vec<Image> {
        let mut out = Vec::new();
        let mut stack = vec![self.home.join("images")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
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
        self.index_named(&image.index).map_err(|e| format!("{}: {e}", image.name))
    }

    /// An index by its SHA-256, in hex, from the store.
    pub fn index_named(&self, hex: &str) -> Result<Vec<u8>, String> {
        let hash = store::parse_hash(hex).ok_or_else(|| format!("{hex} isn't an index's name"))?;
        self.store()?.get(&hash).map_err(|_| format!("the index {hex} is missing from the store"))
    }

    /// Names an index: puts it in the store and writes the image's record. The name's only taken once it's all there.
    pub fn save(&self, name: &str, index: &[u8], platform: &Platform, config: &Config, source: &str) -> Result<Image, String> {
        self.save_with(name, index, platform, config, &[], &Lineage::default(), source)
    }

    /// As [`Images::save`], with its history and what it was made from.
    #[allow(clippy::too_many_arguments)]
    pub fn save_with(&self, name: &str, index: &[u8], platform: &Platform, config: &Config, history: &[History], lineage: &Lineage, source: &str) -> Result<Image, String> {
        let hash = self.store()?.put(index).map_err(|_| "putting the index in the store".to_string())?;
        let image = Image { name: full_name(name), index: store::hex(&hash), platform: platform.clone(), config: config.clone(), created: now(), source: source.into(), history: history.to_vec(), lineage: lineage.clone() };
        self.write_record(&image)?;
        Ok(image)
    }

    /// Another name for an image: the same index and config, under `target` too.
    pub fn tag(&self, source: &str, target: &str) -> Result<Image, String> {
        let mut image = self.image(source).ok_or_else(|| format!("there's no image {}", full_name(source)))?;
        image.name = full_name(target);
        self.write_record(&image)?;
        Ok(image)
    }

    fn write_record(&self, image: &Image) -> Result<(), String> {
        let mut record = json!({
            "name": image.name,
            "index": image.index,
            "platform": image.platform.to_string(),
            "created": image.created,
            "source": image.source,
            "config": image.config.to_json(),
        });
        if !image.history.is_empty() {
            record["history"] = json!(image.history.iter().map(History::to_json).collect::<Vec<_>>());
        }
        if !image.lineage.is_empty() {
            record["lineage"] = image.lineage.to_json();
        }
        let path = match unnamed_id(&image.name) {
            Some(id) => self.unnamed_path(id),
            None => self.record_path(&image.name),
        };
        let dir = path.parent().ok_or("a bad image name")?;
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, serde_json::to_vec_pretty(&record).unwrap_or_default()).map_err(|e| format!("{}: {e}", temporary.display()))?;
        fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(())
    }

    /// An image from a tar of a root file system (`docker export`, for one): each entry's `M` made by `meta`.
    pub fn import<M: Persist>(&self, name: &str, tar: &[u8], platform: &Platform, config: &Config, meta: impl FnMut(Entry) -> M, root: M) -> Result<Image, String> {
        let mut disk = layers::Fs::building(root, platform.os == "windows", Box::new(self.store()?));
        layers::tar::import(&mut disk, tar, meta).map_err(|e| format!("reading the tar: {e}"))?;
        let index = disk.freeze().map_err(|e| format!("writing the index: {e}"))?;
        self.save(name, &index, platform, config, "imported")
    }

    /// Pushes an image to `target` (a name in its registry; the image's own, as `docker push` takes it, or another), a
    /// layer per step unless `squash`: each file's `M` told as a tar entry by `entry`, and `root` the root's for
    /// reading its disks. With the login `auth` keeps for that registry, if there is one.
    pub fn push<M: Persist>(&self, image: &Image, target: &str, squash: bool, entry: impl FnMut(&M) -> Entry, root: M, progress: &mut dyn FnMut(&PushProgress)) -> Result<Pushed, String> {
        push::push(self, image, target, squash, entry, root, progress)
    }

    /// Checks a login with its registry, and keeps it if it's let in.
    pub fn login(&self, registry: &str, credentials: &auth::Credentials) -> Result<(), String> {
        let reference = registry::Reference::parse(&format!("{}/x", auth::registry_key(registry)));
        registry::Client::check_login(&reference, credentials)?;
        auth::store(&self.home, registry, credentials)
    }

    /// Forgets the login Carbon keeps for a registry; whether it had one.
    pub fn logout(&self, registry: &str) -> Result<bool, String> {
        auth::forget(&self.home, registry)
    }

    /// Pulls an image from its registry for `platform`, merges its layers into an index, and names it. Each tar
    /// entry's `M` is made by `meta`, the root's is `root`. Nothing is named until every blob has matched its digest.
    pub fn pull<M: Persist>(&self, name: &str, platform: &Platform, meta: impl FnMut(Entry) -> M, root: M, progress: &mut dyn FnMut(&PullProgress)) -> Result<Image, String> {
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
        assert_eq!(Platform::parse("windows/i386").map(|p| p.to_string()).as_deref(), Some("windows/386"));
        assert_eq!(Platform::parse("linux/386"), None);
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
        assert_eq!(disk.meta(file).unwrap(), 0o644);
        // Unnamed: found by id, the same image saved twice is one, never listed, removable.
        let unnamed = images.save_unnamed(&index, &found.platform, &config, &[], &Lineage::after(&found), "a build").unwrap();
        assert!(unnamed.name.starts_with("sha256:"));
        assert_eq!(images.image(&unnamed.name), Some(unnamed.clone()));
        assert_eq!(unnamed.lineage.disks, vec![Disk { index: found.index.clone(), history: 0 }], "its lineage is kept");
        assert_eq!(images.save_unnamed(&index, &found.platform, &config, &[], &Lineage::default(), "again").unwrap().name, unnamed.name);
        assert_eq!(images.images().len(), 1);
        assert_eq!(images.unnamed().len(), 1);
        images.remove_unnamed(&unnamed.name).unwrap();
        assert!(images.image(&unnamed.name).is_none());
        images.remove("box:1").unwrap();
        assert!(images.image("box:1").is_none());
        fs::remove_dir_all(&home).unwrap();
    }

    /// A one-file tar: `name`, holding `body`.
    fn tar_of(name: &str, body: &[u8]) -> Vec<u8> {
        let mut tar = layers::tar::file_header(name, body.len() as u64);
        tar.extend_from_slice(body);
        tar.resize(tar.len().next_multiple_of(512) + 1024, 0);
        tar
    }

    #[test]
    fn saved_a_layer_per_step_and_loaded_back() {
        let home = std::env::temp_dir().join(format!("carbon-images-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let images = Images::open(Some(&home)).unwrap();
        let base = images.import("base:1", &tar_of("etc/hostname", b"box\n"), &Platform::linux_here(), &Config::default(), |e| e.mode, 0o755u32).unwrap();
        // A step on it: a file added, as a machine frozen after a RUN would be.
        let mut disk: layers::Fs<u32> = layers::Fs::open(Some(&images.index(&base).unwrap()), Box::new(images.machine_store("step").unwrap()), 0o755, false).unwrap();
        let f = disk.create(layers::ROOT, "made", layers::New::File, 0o644, 1_000_000_000).unwrap();
        disk.write(f, 0, b"by a step", 1_000_000_000).unwrap();
        let index = disk.freeze().unwrap();
        let history = vec![History { created_by: "RUN make".into(), ..History::default() }, History { created_by: "CMD x".into(), empty_layer: true, ..History::default() }];
        let app = images.save_with("app:1", &index, &Platform::linux_here(), &Config { cmd: vec!["x".into()], ..Config::default() }, &history, &Lineage::after(&base), "built").unwrap();
        let path = home.join("saved.tar");
        let mut out = fs::File::create(&path).unwrap();
        let entry = |m: &u32| Entry { mode: *m, uid: 0, gid: 0, rdev: 0 };
        archive::save(&images, &[(app.clone(), vec!["app:1".into(), "app:latest".into()])], &mut out, entry, 0o755u32, &mut |_| {}).unwrap();
        drop(out);
        let (archive, found) = archive::Archive::open(&path, &home.join("tmp")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].names, ["app:1", "app:latest"]);
        images.remove("app:1").unwrap();
        let loaded = archive::load(&images, &archive, &found[0], |e| e.mode, 0o755u32).unwrap();
        assert_eq!(loaded.len(), 2);
        let back = images.image("app:1").unwrap();
        assert_eq!(back.config.cmd, ["x"]);
        assert_eq!(back.history.iter().filter(|h| !h.empty_layer).count(), 2, "a layer for the base, one for the step");
        assert_eq!(back.lineage.disks.len(), 1, "the disk after its first layer, kept");
        // Saved again, it's the same two layers.
        let again = home.join("again.tar");
        archive::save(&images, &[(back.clone(), vec!["app:1".into()])], &mut fs::File::create(&again).unwrap(), entry, 0o755u32, &mut |_| {}).unwrap();
        let blobs = |p: &Path| archive::Archive::open(p, &home.join("tmp")).unwrap().1[0].layer_count();
        assert_eq!((blobs(&path), blobs(&again)), (2, 2));
        let disk: layers::Fs<u32> = layers::Fs::open(Some(&images.index(&back).unwrap()), Box::new(images.store().unwrap()), 0, false).unwrap();
        let made = disk.resolve(layers::ROOT, "/made", true).unwrap();
        assert_eq!(disk.contents(made).unwrap(), b"by a step");
        assert_eq!(disk.contents(disk.resolve(layers::ROOT, "/etc/hostname", true).unwrap()).unwrap(), b"box\n");
        fs::remove_dir_all(&home).unwrap();
    }
}
