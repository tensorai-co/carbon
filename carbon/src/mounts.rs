//! Mounts and volumes: a host's folder seen live in a machine ([`Mount::bind`]), a volume, a folder Carbon keeps
//! that outlives the machines that mount it ([`Mount::volume`], [`Volume`]), and a tmpfs ([`Mount::tmpfs`]), which
//! may start with files in it ([`Mount::with_file`]): in memory only, so never on the machine's disk or in an image
//! committed from it.
//!
//! A bound folder and a volume are both host folders (`host::folder`), attached to the machine's disk as an external
//! tree (`layers::External`) and mounted where the machine wants them: every call a program makes on a file there is
//! made on the host's file as it happens, so each side sees the other's changes at once. A volume is a folder under
//! `<home>/volumes/<name>/data` that keeps each file's owner and mode with it, so a database's files are its user's
//! when it's mounted again; a new, empty volume is filled from what the image has where it's mounted, as Docker's is.

use std::path::{Path, PathBuf};

use host::folder::{FileKind, Folder};
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};

/// What a mount has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountKind {
    /// A host's folder, by its absolute path.
    Bind(PathBuf),
    /// A volume, by its name: made if it isn't there.
    Volume(String),
    /// Memory only, gone when the machine is, starting with these files.
    Tmpfs(Vec<TmpfsFile>),
}

/// A file a tmpfs starts with: its path under the tmpfs (`config.toml`, `a/b.txt`), its bytes and its mode, owned by
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmpfsFile {
    pub path: String,
    pub data: Vec<u8>,
    pub mode: u32,
}

/// A folder mounted in a machine: [`crate::MachineSpec::mount`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub kind: MountKind,
    /// Where it is in the machine: `/work`.
    pub target: String,
    pub read_only: bool,
}

impl Mount {
    /// A host's folder, at `target` in the machine: what either side changes, the other sees at once. A relative
    /// path is from this process's working folder.
    pub fn bind(host: impl AsRef<Path>, target: impl Into<String>) -> Mount {
        let host = host.as_ref();
        let host = if host.is_absolute() { host.to_path_buf() } else { std::env::current_dir().map(|d| d.join(host)).unwrap_or_else(|_| host.to_path_buf()) };
        Mount { kind: MountKind::Bind(host), target: target.into(), read_only: false }
    }

    /// The volume `name` (made if it isn't there), at `target`.
    pub fn volume(name: impl Into<String>, target: impl Into<String>) -> Mount {
        Mount { kind: MountKind::Volume(name.into()), target: target.into(), read_only: false }
    }

    /// A tmpfs at `target`: memory only, never on the disk.
    pub fn tmpfs(target: impl Into<String>) -> Mount {
        Mount { kind: MountKind::Tmpfs(Vec::new()), target: target.into(), read_only: false }
    }

    /// This tmpfs, starting with `data` at `path` under it (`config.toml`, `a/b.txt`; its folders are made), mode
    /// 0644. A tmpfs is memory only, so the file is never on the machine's disk, or in an image committed from it.
    /// Only a tmpfs has files: on any other mount, this does nothing.
    pub fn with_file(self, path: impl Into<String>, data: impl Into<Vec<u8>>) -> Mount {
        self.with_file_mode(path, data, 0o644)
    }

    /// [`Mount::with_file`], with its mode.
    pub fn with_file_mode(mut self, path: impl Into<String>, data: impl Into<Vec<u8>>, mode: u32) -> Mount {
        if let MountKind::Tmpfs(files) = &mut self.kind {
            files.push(TmpfsFile { path: path.into(), data: data.into(), mode });
        }
        self
    }

    /// Nothing in it can be changed from the machine.
    pub fn read_only(mut self) -> Mount {
        self.read_only = true;
        self
    }

    /// Docker's `-v` (`/host/path:/target[:ro]`, `name:/target`) or `--mount`
    /// (`type=bind,source=/host,target=/work,readonly`), or `--tmpfs /target`'s path.
    pub fn parse(text: &str) -> Result<Mount> {
        let bad = |why: &str| Error::invalid(format!("{text}: {why}"));
        if text.contains('=') {
            let (mut kind, mut source, mut target, mut read_only) = (String::from("volume"), None, None, false);
            for part in text.split(',') {
                let (k, v) = part.split_once('=').unwrap_or((part, "true"));
                match k {
                    "type" => kind = v.to_string(),
                    "source" | "src" => source = Some(v.to_string()),
                    "target" | "destination" | "dst" => target = Some(v.to_string()),
                    "readonly" | "ro" => read_only = v != "false" && v != "0",
                    "bind-propagation" | "consistency" | "volume-nocopy" | "tmpfs-size" | "tmpfs-mode" => {}
                    other => return Err(bad(&format!("{other} isn't something a mount says"))),
                }
            }
            let target = target.ok_or_else(|| bad("a mount needs a target"))?;
            let mount = match kind.as_str() {
                "bind" => Mount::bind(source.ok_or_else(|| bad("a bind mount needs a source"))?, target),
                "volume" => Mount::volume(source.ok_or_else(|| bad("a volume mount needs a source (its name)"))?, target),
                "tmpfs" => Mount::tmpfs(target),
                other => return Err(bad(&format!("type={other}: bind, volume or tmpfs"))),
            };
            return Ok(Mount { read_only, ..mount });
        }
        let parts: Vec<&str> = text.split(':').collect();
        let (source, target, options) = match parts.as_slice() {
            [target] => return Ok(Mount::tmpfs(*target)),
            [source, target] => (*source, *target, ""),
            [source, target, options] => (*source, *target, *options),
            _ => return Err(bad("SOURCE:TARGET[:ro]")),
        };
        if !target.starts_with('/') {
            return Err(bad("the target must be an absolute path in the machine"));
        }
        let read_only = options.split(',').any(|o| o == "ro");
        let bind = source.starts_with('/') || source.starts_with('.') || source.starts_with('~') || source.contains('/');
        let mount = if bind {
            let path = match source.strip_prefix("~/") {
                Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).ok_or_else(|| bad("no home folder for ~"))?,
                None => PathBuf::from(source),
            };
            Mount::bind(path, target)
        } else {
            Mount::volume(source, target)
        };
        Ok(Mount { read_only, ..mount })
    }

    pub(crate) fn to_json(&self) -> Value {
        let (kind, source) = match &self.kind {
            MountKind::Bind(p) => ("bind", Some(p.to_string_lossy().into_owned())),
            MountKind::Volume(n) => ("volume", Some(n.clone())),
            MountKind::Tmpfs(_) => ("tmpfs", None),
        };
        let mut v = json!({ "type": kind, "source": source, "target": self.target, "read_only": self.read_only });
        if let MountKind::Tmpfs(files) = &self.kind
            && !files.is_empty()
        {
            // Text as it is; anything else as hex.
            v["files"] = files
                .iter()
                .map(|f| match std::str::from_utf8(&f.data) {
                    Ok(text) => json!({ "path": f.path, "mode": f.mode, "text": text }),
                    Err(_) => json!({ "path": f.path, "mode": f.mode, "hex": f.data.iter().map(|b| format!("{b:02x}")).collect::<String>() }),
                })
                .collect();
        }
        v
    }

    pub(crate) fn from_json(v: &Value) -> Option<Mount> {
        let source = v.get("source").and_then(Value::as_str);
        let kind = match v.get("type")?.as_str()? {
            "bind" => MountKind::Bind(PathBuf::from(source?)),
            "volume" => MountKind::Volume(source?.to_string()),
            "tmpfs" => MountKind::Tmpfs(
                v.get("files")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|f| {
                                let data = match (f.get("text").and_then(Value::as_str), f.get("hex").and_then(Value::as_str)) {
                                    (Some(t), _) => t.as_bytes().to_vec(),
                                    (None, Some(h)) => (0..h.len() / 2).map(|i| u8::from_str_radix(h.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?,
                                    _ => return None,
                                };
                                Some(TmpfsFile { path: f.get("path")?.as_str()?.to_string(), data, mode: f.get("mode").and_then(Value::as_u64).unwrap_or(0o644) as u32 })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            _ => return None,
        };
        Some(Mount { kind, target: v.get("target")?.as_str()?.to_string(), read_only: v.get("read_only").and_then(Value::as_bool).unwrap_or(false) })
    }

    /// As `carbon inspect` and `ls` show it: `/host:/target:ro`.
    pub fn describe(&self) -> String {
        let source = match &self.kind {
            MountKind::Bind(p) => p.to_string_lossy().into_owned(),
            MountKind::Volume(n) => n.clone(),
            MountKind::Tmpfs(_) => "tmpfs".to_string(),
        };
        format!("{source}:{}{}", self.target, if self.read_only { ":ro" } else { "" })
    }
}

pub(crate) fn mounts_json(mounts: &[Mount]) -> Value {
    Value::Array(mounts.iter().map(Mount::to_json).collect())
}

pub(crate) fn mounts_from_json(v: Option<&Value>) -> Vec<Mount> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Mount::from_json).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------
// Volumes

/// A volume: a folder Carbon keeps, which machines mount and which outlives them. [`crate::Carbon::volume`].
#[derive(Debug, Clone)]
pub struct Volume {
    pub(crate) home: PathBuf,
    pub name: String,
}

/// What `Carbon::volumes` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub name: String,
    /// Seconds since 1970.
    pub created: u64,
    /// Where its files are on the host.
    pub path: PathBuf,
}

/// Whether a volume's name is one: letters, digits, `_`, `.` and `-`, starting with a letter or digit.
pub(crate) fn valid_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

pub(crate) fn volumes_dir(home: &Path) -> PathBuf {
    home.join("volumes")
}

impl Volume {
    fn dir(&self) -> PathBuf {
        volumes_dir(&self.home).join(&self.name)
    }

    /// Where its files are on the host.
    pub fn path(&self) -> PathBuf {
        self.dir().join("data")
    }

    pub fn exists(&self) -> bool {
        self.path().is_dir()
    }

    /// Makes it, if it isn't there. Whether it was made now.
    pub fn create(&self) -> Result<bool> {
        if !valid_name(&self.name) {
            return Err(Error::invalid(format!("{:?} can't be a volume's name: letters, digits, _ . and -", self.name)));
        }
        if self.exists() {
            return Ok(false);
        }
        std::fs::create_dir_all(self.path()).map_err(|e| Error::io(format!("making volume {}: {e}", self.name)))?;
        let record = json!({ "name": self.name, "created": crate::store::now() });
        std::fs::write(self.dir().join("volume.json"), record.to_string()).map_err(|e| Error::io(format!("volume {}: {e}", self.name)))?;
        Ok(true)
    }

    pub fn info(&self) -> Option<VolumeInfo> {
        let text = std::fs::read_to_string(self.dir().join("volume.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        Some(VolumeInfo { name: self.name.clone(), created: v.get("created").and_then(Value::as_u64).unwrap_or(0), path: self.path() })
    }

    /// Removes it and its files.
    pub(crate) fn remove(&self) -> Result<()> {
        if !self.dir().exists() {
            return Err(Error::not_found(format!("there's no volume {}", self.name)));
        }
        std::fs::remove_dir_all(self.dir()).map_err(|e| Error::io(format!("removing volume {}: {e}", self.name)))
    }
}

pub(crate) fn list_volumes(home: &Path) -> Vec<VolumeInfo> {
    let mut out: Vec<VolumeInfo> = std::fs::read_dir(volumes_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Volume { home: home.to_path_buf(), name: e.file_name().to_string_lossy().into_owned() }.info())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---------------------------------------------------------------------------------------------------------------
// A host folder as an external tree

/// A host folder, as the tree a machine's disk attaches, and who its files are said to be owned by when they don't
/// keep an owner of their own: the machine's user.
pub(crate) struct HostTree(pub Folder, pub (u32, u32));

fn layers_error(e: std::io::Error) -> layers::Error {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => layers::Error::NotFound,
        K::AlreadyExists => layers::Error::Exists,
        K::PermissionDenied => layers::Error::Denied,
        K::NotADirectory => layers::Error::NotDir,
        K::IsADirectory => layers::Error::IsDir,
        K::DirectoryNotEmpty => layers::Error::NotEmpty,
        K::ReadOnlyFilesystem => layers::Error::ReadOnly,
        K::CrossesDevices => layers::Error::CrossFs,
        K::InvalidInput | K::InvalidFilename => layers::Error::BadName,
        K::Unsupported => layers::Error::Unsupported,
        _ => layers::Error::Io,
    }
}

fn kind(k: FileKind) -> layers::Kind {
    match k {
        FileKind::File => layers::Kind::File,
        FileKind::Dir => layers::Kind::Dir,
        FileKind::Symlink => layers::Kind::Symlink,
    }
}

impl layers::External for HostTree {
    fn stat(&self, path: &str) -> layers::Result<layers::ExtStat> {
        let s = self.0.stat(path).map_err(layers_error)?;
        Ok(layers::ExtStat { kind: kind(s.kind), size: s.size, modified: s.modified, mode: s.mode, uid: self.1.0, gid: self.1.1, links: s.links, meta: s.meta })
    }
    fn list(&self, path: &str) -> layers::Result<Vec<(String, layers::Kind)>> {
        Ok(self.0.list(path).map_err(layers_error)?.into_iter().map(|(n, k)| (n, kind(k))).collect())
    }
    fn read(&self, path: &str, offset: u64, buffer: &mut [u8]) -> layers::Result<usize> {
        self.0.read(path, offset, buffer).map_err(layers_error)
    }
    fn write(&self, path: &str, offset: u64, data: &[u8]) -> layers::Result<usize> {
        self.0.write(path, offset, data).map_err(layers_error)
    }
    fn truncate(&self, path: &str, size: u64) -> layers::Result<()> {
        self.0.truncate(path, size).map_err(layers_error)
    }
    fn make(&self, path: &str, new: &layers::New, mode: u32) -> layers::Result<()> {
        match new {
            layers::New::File => self.0.make_file(path, mode),
            layers::New::Dir => self.0.make_dir(path, mode),
            layers::New::Symlink(target) => self.0.make_symlink(path, target),
        }
        .map_err(layers_error)
    }
    fn link(&self, existing: &str, path: &str) -> layers::Result<()> {
        self.0.link(existing, path).map_err(layers_error)
    }
    fn remove(&self, path: &str, dir: bool) -> layers::Result<()> {
        self.0.remove(path, dir).map_err(layers_error)
    }
    fn rename(&self, from: &str, to: &str) -> layers::Result<()> {
        self.0.rename(from, to).map_err(layers_error)
    }
    fn read_link(&self, path: &str) -> layers::Result<String> {
        self.0.read_link(path).map_err(layers_error)
    }
    fn set_mode(&self, path: &str, mode: u32) -> layers::Result<()> {
        self.0.set_mode(path, mode).map_err(layers_error)
    }
    fn set_modified(&self, path: &str, modified: u64) -> layers::Result<()> {
        self.0.set_modified(path, modified).map_err(layers_error)
    }
    fn keep_meta(&self, path: &str, words: [u32; 4]) -> layers::Result<bool> {
        self.0.keep_meta(path, words).map_err(layers_error)
    }
}

/// The folders a machine's mounts have, ready for its kernel: each bound folder opened (it must be there), each
/// volume made if it isn't (and, if it's new, filled from what `disk` has at its target), each tmpfs as it is.
pub(crate) fn lambda_mounts(home: &Path, mounts: &[Mount], disk: &lambda::Disk, owner: (u32, u32)) -> Result<Vec<lambda::Mount>> {
    let mut out = Vec::new();
    for m in mounts {
        let source = match &m.kind {
            MountKind::Bind(path) => {
                let missing = |e: std::io::Error| Error::new(ErrorKind::NotFound, format!("{}: a bound folder or file must be there: {e}", path.display()));
                let shown = path.to_string_lossy().into_owned();
                if std::fs::metadata(path).map_err(missing)?.is_dir() {
                    let folder = Folder::open(path, false).map_err(missing)?;
                    lambda::MountSource::Folder(lambda::Folder::new(Box::new(HostTree(folder, owner))), shown)
                } else {
                    // One file: its folder attached, and only the file mounted.
                    let (parent, name) = (path.parent().unwrap_or(Path::new("/")), path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    let folder = Folder::open(parent, false).map_err(missing)?;
                    lambda::MountSource::File(lambda::Folder::new(Box::new(HostTree(folder, owner))), name, shown)
                }
            }
            MountKind::Volume(name) => {
                let volume = Volume { home: home.to_path_buf(), name: name.clone() };
                let made = volume.create()?;
                let folder = Folder::open(volume.path(), true).map_err(|e| Error::io(format!("volume {name}: {e}")))?;
                if made {
                    fill_volume(disk, &m.target, &folder)?;
                }
                lambda::MountSource::Folder(lambda::Folder::new(Box::new(HostTree(folder, owner))), name.clone())
            }
            MountKind::Tmpfs(files) => lambda::MountSource::Tmpfs(files.iter().map(|f| lambda::TmpfsFile { path: f.path.clone(), data: f.data.clone(), mode: f.mode }).collect()),
        };
        out.push(lambda::Mount { target: m.target.clone(), source, read_only: m.read_only });
    }
    Ok(out)
}

/// A new volume, filled from what the image has where it's mounted: its files, folders and links, with their
/// owners and modes kept.
fn fill_volume(disk: &lambda::Disk, target: &str, folder: &Folder) -> Result<()> {
    let Ok(top) = disk.resolve(layers::ROOT, target, true) else { return Ok(()) };
    if disk.stat(top).map(|s| s.kind) != Ok(layers::Kind::Dir) {
        return Ok(());
    }
    let io = |e: std::io::Error| Error::io(format!("filling a volume from {target}: {e}"));
    let lay = |e: layers::Error| Error::io(format!("filling a volume from {target}: {e}"));
    // Its top takes the image's folder's owner and mode, as a database's data folder needs.
    let top_meta = disk.meta(top).map_err(lay)?;
    folder.set_mode("", top_meta.mode & 0o7777).map_err(io)?;
    folder.keep_meta("", <lambda::Meta as layers::Persist>::save(&lambda::Meta { mode: top_meta.mode & 0o7777 | 0o040000, ..top_meta })).map_err(io)?;
    let mut todo = vec![(top, String::new())];
    while let Some((dir, rel)) = todo.pop() {
        for (name, ino, k) in disk.entries(dir).map_err(lay)? {
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let meta = disk.meta(ino).map_err(lay)?;
            match k {
                layers::Kind::Dir => {
                    folder.make_dir(&path, meta.mode & 0o7777).map_err(io)?;
                    todo.push((ino, path.clone()));
                }
                layers::Kind::Symlink => folder.make_symlink(&path, &disk.read_link(ino).map_err(lay)?).map_err(io)?,
                layers::Kind::File => {
                    folder.make_file(&path, meta.mode & 0o7777).map_err(io)?;
                    folder.write(&path, 0, &disk.contents(ino).map_err(lay)?).map_err(io)?;
                }
            }
            folder.keep_meta(&path, <lambda::Meta as layers::Persist>::save(&meta)).map_err(io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> layers::Mapping<lambda::Meta> {
        layers::Mapping {
            meta: |st| match st.meta {
                Some(w) => <lambda::Meta as layers::Persist>::load(w),
                None => lambda::Meta { mode: st.mode, ..lambda::Meta::default() },
            },
            mode: |m| m.mode & 0o7777,
            save: <lambda::Meta as layers::Persist>::save,
        }
    }

    /// A disk with a host folder attached: every call reaches the folder, and the folder's changes are seen.
    #[test]
    fn a_host_folder_attached_to_a_disk() {
        let dir = std::env::temp_dir().join(format!("carbon-attach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "from the host").unwrap();
        let mut disk = lambda::Disk::new(lambda::Meta::dir(0o755), false);
        let root = disk.attach(Box::new(HostTree(Folder::open(&dir, true).unwrap(), (0, 0))), map(), false).unwrap();
        assert_eq!(disk.external(root), Some(root));
        // Read what the host has.
        let a = disk.resolve(root, "src/a.txt", true).unwrap();
        assert_eq!(disk.contents(a).unwrap(), b"from the host");
        assert_eq!(disk.resolve(root, "src/a.txt", true).unwrap(), a, "a file keeps its number");
        // Made, written, linked, renamed and removed here: there.
        let src = disk.resolve(root, "src", true).unwrap();
        let b = disk.create(src, "b.txt", layers::New::File, lambda::Meta { uid: 999, ..lambda::Meta::file(0o600) }, 0).unwrap();
        disk.write(b, 0, b"written here", 0).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("src/b.txt")).unwrap(), "written here");
        assert_eq!(disk.meta(b).unwrap().uid, 999, "kept with the file");
        disk.create(src, "l", layers::New::Symlink("b.txt".into()), lambda::Meta::file(0o777), 0).unwrap();
        assert_eq!(disk.resolve(root, "src/l", true).unwrap(), b);
        disk.rename(src, "b.txt", root, "c.txt", 0).unwrap();
        assert_eq!(disk.resolve(root, "c.txt", true).unwrap(), b, "a moved file keeps its number");
        assert!(dir.join("c.txt").exists());
        let names: Vec<String> = disk.entries(root).unwrap().into_iter().map(|e| e.0).collect();
        assert_eq!(names, ["c.txt", "src"]);
        // What the host changes is seen at once.
        std::fs::write(dir.join("src/new.txt"), "later").unwrap();
        let n = disk.resolve(root, "src/new.txt", true).unwrap();
        assert_eq!(disk.contents(n).unwrap(), b"later");
        // Nothing crosses between it and the disk's own tree.
        let own = disk.create(layers::ROOT, "own", layers::New::File, lambda::Meta::file(0o644), 0).unwrap();
        assert_eq!(disk.link(src, "x", own, 0), Err(layers::Error::CrossFs));
        assert_eq!(disk.rename(root, "c.txt", layers::ROOT, "c.txt", 0), Err(layers::Error::CrossFs));
        disk.remove(root, "c.txt", 0).unwrap();
        assert!(!dir.join("c.txt").exists());
        // Read-only: nothing changes.
        let ro = disk.attach(Box::new(HostTree(Folder::open(&dir, false).unwrap(), (0, 0))), map(), true).unwrap();
        assert_eq!(disk.create(ro, "nope", layers::New::File, lambda::Meta::file(0o644), 0), Err(layers::Error::ReadOnly));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mounts_as_docker_writes_them() {
        let m = Mount::parse("/srv/data:/data:ro").unwrap();
        assert_eq!((m.kind, m.target.as_str(), m.read_only), (MountKind::Bind("/srv/data".into()), "/data", true));
        assert_eq!(Mount::parse("pgdata:/var/lib/postgresql/data").unwrap().kind, MountKind::Volume("pgdata".into()));
        assert_eq!(Mount::parse("type=tmpfs,target=/scratch").unwrap().kind, MountKind::Tmpfs(Vec::new()));
        // A tmpfs's files go to the worker and back, text as text and anything else as hex.
        let seeded = Mount::tmpfs("/.cargo").with_file("config.toml", "[source.crates-io]\n").with_file_mode("bin/x", vec![0u8, 255, 7], 0o755);
        assert_eq!(Mount::from_json(&seeded.to_json()), Some(seeded.clone()));
        assert_eq!(seeded.to_json()["files"][0]["text"], "[source.crates-io]\n");
        assert_eq!(Mount::volume("v", "/v").with_file("x", "y"), Mount::volume("v", "/v"));
        let m = Mount::parse("type=bind,src=/a,dst=/b,readonly").unwrap();
        assert_eq!((m.kind, m.target.as_str(), m.read_only), (MountKind::Bind("/a".into()), "/b", true));
        assert!(Mount::parse("x:relative").is_err());
        assert!(Mount::parse("type=nfs,target=/x").is_err());
        let m = Mount::volume("cache", "/root/.cache").read_only();
        assert_eq!(Mount::from_json(&m.to_json()), Some(m));
        assert!(valid_name("pg-data_1.2") && !valid_name("-x") && !valid_name("a/b"));
    }
}
