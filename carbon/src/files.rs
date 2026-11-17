//! A machine's files, running or stopped: [`Files`] (stat, list, read, write, make folders, links, rename, remove)
//! and copies between it and the host that keep what a file is ([`Copy`]).
//!
//! Every operation is one of [`Op`]'s, done on the machine's disk by [`apply`], the same code for both kernels and both
//! cases. A stopped machine's disk is opened here, once for a call or a whole copy, and saved after. A running one's is
//! its kernel's: the operation goes over its control socket (`fs`), and its process does it under the lock its
//! programs' own calls take, so a write is what they see next, and a watching program hears of it.
//!
//! Paths are as the machine names them: `/etc/hosts`, or `C:\Program Files\App` in a Windows machine. Owners are
//! numbers or names from the machine's own `/etc/passwd` and `/etc/group` (`app`, `app:staff`, `1000:1000`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use layers::{Fs, Ino, Kind, New, Persist, ROOT};
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::store::{Kernel, Record, Store};

/// The most a read gives at once.
pub const MOST_READ: u64 = 64 << 20;
/// How much of a file a copy moves in one write.
const CHUNK: usize = 8 << 20;

/// What a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
}

/// What's known of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub kind: FileKind,
    pub size: u64,
    /// Permission bits. A Windows machine's are made from its attributes: read-only or not.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    /// A Windows machine's attributes (`FILE_ATTRIBUTE_*`); 0 in a Linux machine.
    pub attributes: u32,
    pub modified: SystemTime,
    /// What a symbolic link points to.
    pub link: Option<String>,
    /// The same for every name of one file, and how many names it has (hard links).
    pub id: u64,
    pub links: u32,
}

/// A folder's entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub info: FileInfo,
}

/// How [`Files::write`] writes: the file's mode and owner (a new file's are `0644` and root's), its folders made if
/// they aren't there, appended to rather than replaced, its modified time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Write {
    mode: Option<u32>,
    owner: Option<String>,
    parents: bool,
    append: bool,
    modified: Option<SystemTime>,
}

impl Write {
    pub fn new() -> Write {
        Write::default()
    }
    pub fn mode(mut self, mode: u32) -> Write {
        self.mode = Some(mode);
        self
    }
    /// `user`, `user:group`, or numbers, as the machine knows them.
    pub fn owner(mut self, owner: &str) -> Write {
        self.owner = Some(owner.to_string());
        self
    }
    /// Makes the folders it's in, if they aren't there.
    pub fn parents(mut self, parents: bool) -> Write {
        self.parents = parents;
        self
    }
    pub fn append(mut self, append: bool) -> Write {
        self.append = append;
        self
    }
    pub fn modified(mut self, at: SystemTime) -> Write {
        self.modified = Some(at);
        self
    }
}

/// How [`Files::remove`] removes: a folder and everything in it, or only an empty one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Remove {
    recursive: bool,
}

impl Remove {
    pub fn new() -> Remove {
        Remove::default()
    }
    pub fn recursive() -> Remove {
        Remove { recursive: true }
    }
}

/// How [`crate::Machine::copy_in`] and [`crate::Machine::copy_out`] copy. Kinds, modes, times, symbolic links and hard
/// links are kept either way. What's copied in is root's, as `docker cp` makes it, unless `archive` keeps the host's
/// owners or `chown` names one; what's copied out is this process's, unless `archive` keeps the machine's (which needs
/// the right to).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Copy {
    owner: Option<String>,
    mode: Option<u32>,
    archive: bool,
    /// Patterns, each with where what's copied is under the patterns' root (`src` when copying `src`'s contents with
    /// the context's ignore file).
    exclude: Vec<(crate::build::Patterns, String)>,
}

impl Copy {
    pub fn new() -> Copy {
        Copy::default()
    }
    /// Every file and folder copied in owned by `owner` (`user`, `user:group`, or numbers).
    pub fn chown(mut self, owner: &str) -> Copy {
        self.owner = Some(owner.to_string());
        self
    }
    /// Every file copied given these permissions (folders keep theirs).
    pub fn chmod(mut self, mode: u32) -> Copy {
        self.mode = Some(mode);
        self
    }
    /// Owners kept as they are, as `docker cp -a`.
    pub fn archive(mut self) -> Copy {
        self.archive = true;
        self
    }
    /// Leaves out what `patterns` exclude, as an ignore file's patterns (`node_modules`, `**/*.log`, `!keep.log`),
    /// matched against paths under what's copied: copying the folder `app`, `app/node_modules` is `node_modules`.
    pub fn exclude(self, patterns: crate::build::Patterns) -> Copy {
        self.exclude_under(patterns, "")
    }
    /// As [`Copy::exclude`], with what's copied at `prefix` under the patterns' root (a build context's ignore file,
    /// copying `src` from it).
    pub(crate) fn exclude_under(mut self, patterns: crate::build::Patterns, prefix: &str) -> Copy {
        if !patterns.is_empty() {
            self.exclude.push((patterns, prefix.trim_matches('/').to_string()));
        }
        self
    }
    /// Whether the path `rel` under what's copied is left out, and (for a folder) whether something in it might not be.
    fn excluded(&self, rel: &str) -> (bool, bool) {
        let (mut out, mut inside) = (false, false);
        for (patterns, prefix) in &self.exclude {
            let path = if prefix.is_empty() { rel.to_string() } else { format!("{prefix}/{rel}") };
            if patterns.excludes(&path) {
                out = true;
                inside |= patterns.may_include_inside(&path);
            }
        }
        (out, inside || !out)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The operations, on a disk

/// What each kernel keeps of a file, as files see it.
pub(crate) trait Meta: Persist + Clone + 'static {
    /// The disk's root's, as a store opens it.
    fn root() -> Self;
    const FOLD_CASE: bool;
    fn make(kind: Kind, mode: u32, uid: u32, gid: u32) -> Self;
    /// Mode, uid, gid and attributes.
    fn describe(&self, kind: Kind) -> (u32, u32, u32, u32);
    /// Owners by name, from the disk: Linux's passwd and group files.
    fn owner(disk: &Fs<Self>, owner: &str) -> Result<(u32, u32)>;
}

impl Meta for lambda::Meta {
    fn root() -> Self {
        lambda::Meta::dir(0o755)
    }
    const FOLD_CASE: bool = false;
    fn make(_kind: Kind, mode: u32, uid: u32, gid: u32) -> Self {
        lambda::Meta { mode: mode & 0o7777, uid, gid, rdev: 0 }
    }
    fn describe(&self, _kind: Kind) -> (u32, u32, u32, u32) {
        (self.mode & 0o7777, self.uid, self.gid, 0)
    }
    fn owner(disk: &Fs<Self>, owner: &str) -> Result<(u32, u32)> {
        let read = |path: &str| disk.resolve(ROOT, path, true).and_then(|ino| disk.contents(ino)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let (user, group) = owner.split_once(':').map_or((owner, None), |(u, g)| (u, Some(g)));
        // A name's line in a passwd or group file: its number, and (for a user) its group's.
        let find = |file: &str, name: &str| -> Option<(u32, u32)> {
            read(file).lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.len() >= 3 && f[0] == name).map(|f| (f[2].parse().unwrap_or(0), f.get(3).and_then(|g| g.parse().ok()).unwrap_or(0)))
        };
        let (uid, primary) = match user.parse::<u32>() {
            Ok(uid) => (uid, uid),
            Err(_) => find("/etc/passwd", user).ok_or_else(|| Error::not_found(format!("{user}: no such user in the machine's /etc/passwd")))?,
        };
        let gid = match group {
            None => primary,
            Some(g) => match g.parse::<u32>() {
                Ok(gid) => gid,
                Err(_) => find("/etc/group", g).map(|(gid, _)| gid).ok_or_else(|| Error::not_found(format!("{g}: no such group in the machine's /etc/group")))?,
            },
        };
        Ok((uid, gid))
    }
}

/// Windows' attributes: a folder, a file to archive, read-only.
const ATTRIBUTE_READONLY: u32 = 0x1;
const ATTRIBUTE_DIRECTORY: u32 = 0x10;
const ATTRIBUTE_ARCHIVE: u32 = 0x20;

impl Meta for atlas::Meta {
    fn root() -> Self {
        atlas::Meta { attributes: ATTRIBUTE_DIRECTORY }
    }
    const FOLD_CASE: bool = true;
    fn make(kind: Kind, mode: u32, _uid: u32, _gid: u32) -> Self {
        let readonly = if mode & 0o222 == 0 { ATTRIBUTE_READONLY } else { 0 };
        atlas::Meta { attributes: readonly | if kind == Kind::Dir { ATTRIBUTE_DIRECTORY } else { ATTRIBUTE_ARCHIVE } }
    }
    fn describe(&self, kind: Kind) -> (u32, u32, u32, u32) {
        let mode = match kind {
            Kind::Dir => 0o755,
            _ if self.attributes & ATTRIBUTE_READONLY != 0 => 0o444,
            _ => 0o644,
        };
        (mode, 0, 0, self.attributes)
    }
    fn owner(_disk: &Fs<Self>, _owner: &str) -> Result<(u32, u32)> {
        // A Windows machine's files have no owners of the Unix kind: they're everyone's.
        Ok((0, 0))
    }
}

/// An operation on a machine's disk, as it goes over the control socket.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Op {
    Stat { path: String },
    List { path: String },
    Read { path: String, offset: u64, len: u64 },
    /// Its bytes follow the request on the socket.
    Write { path: String, len: u64, mode: Option<u32>, owner: Option<String>, parents: bool, append: bool, modified: Option<u64> },
    Mkdir { path: String, mode: Option<u32>, owner: Option<String>, parents: bool, modified: Option<u64> },
    Symlink { path: String, target: String, owner: Option<String> },
    /// Another name for the file at `to`.
    Link { path: String, to: String },
    Rename { from: String, to: String },
    Remove { path: String, recursive: bool },
}

impl Op {
    pub fn to_json(&self) -> Value {
        match self {
            Op::Stat { path } => json!({ "op": "stat", "path": path }),
            Op::List { path } => json!({ "op": "list", "path": path }),
            Op::Read { path, offset, len } => json!({ "op": "read", "path": path, "offset": offset, "len": len }),
            Op::Write { path, len, mode, owner, parents, append, modified } => json!({ "op": "write", "path": path, "len": len, "mode": mode, "owner": owner, "parents": parents, "append": append, "modified": modified }),
            Op::Mkdir { path, mode, owner, parents, modified } => json!({ "op": "mkdir", "path": path, "mode": mode, "owner": owner, "parents": parents, "modified": modified }),
            Op::Symlink { path, target, owner } => json!({ "op": "symlink", "path": path, "target": target, "owner": owner }),
            Op::Link { path, to } => json!({ "op": "link", "path": path, "to": to }),
            Op::Rename { from, to } => json!({ "op": "rename", "from": from, "to": to }),
            Op::Remove { path, recursive } => json!({ "op": "remove", "path": path, "recursive": recursive }),
        }
    }

    pub fn from_json(v: &Value) -> Option<Op> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        let u = |k: &str| v.get(k).and_then(Value::as_u64);
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let mode = u("mode").map(|m| m as u32);
        Some(match v.get("op")?.as_str()? {
            "stat" => Op::Stat { path: s("path")? },
            "list" => Op::List { path: s("path")? },
            "read" => Op::Read { path: s("path")?, offset: u("offset").unwrap_or(0), len: u("len").unwrap_or(MOST_READ) },
            "write" => Op::Write { path: s("path")?, len: u("len")?, mode, owner: s("owner"), parents: b("parents"), append: b("append"), modified: u("modified") },
            "mkdir" => Op::Mkdir { path: s("path")?, mode, owner: s("owner"), parents: b("parents"), modified: u("modified") },
            "symlink" => Op::Symlink { path: s("path")?, target: s("target")?, owner: s("owner") },
            "link" => Op::Link { path: s("path")?, to: s("to")? },
            "rename" => Op::Rename { from: s("from")?, to: s("to")? },
            "remove" => Op::Remove { path: s("path")?, recursive: b("recursive") },
            _ => return None,
        })
    }

    /// Whether it changes the disk (a stopped machine's is saved after).
    fn changes(&self) -> bool {
        !matches!(self, Op::Stat { .. } | Op::List { .. } | Op::Read { .. })
    }
}

fn failed(path: &str, e: layers::Error) -> Error {
    let kind = match e {
        layers::Error::NotFound => ErrorKind::NotFound,
        layers::Error::Exists => ErrorKind::AlreadyExists,
        layers::Error::Io => ErrorKind::Io,
        layers::Error::ReadOnly | layers::Error::Denied => ErrorKind::Denied,
        _ => ErrorKind::InvalidInput,
    };
    let what = match e {
        layers::Error::NotFound => "not found",
        layers::Error::NotDir => "a folder in its path isn't one",
        layers::Error::IsDir => "is a folder",
        layers::Error::Exists => "is there already",
        layers::Error::NotEmpty => "is a folder with something in it (remove it recursively)",
        layers::Error::Loop => "too many symbolic links",
        layers::Error::BadName => "isn't a name a file can have",
        layers::Error::CrossFs => "is on another file system",
        layers::Error::Io => "the disk couldn't be read or written",
        layers::Error::ReadOnly => "is on a read-only mount",
        layers::Error::Denied => "the host refused it",
        layers::Error::Unsupported => "can't be kept there",
    };
    Error::new(kind, format!("{path}: {what}"))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

fn info<M: Meta>(disk: &Fs<M>, ino: Ino) -> std::result::Result<FileInfo, layers::Error> {
    let stat = disk.stat(ino)?;
    let (mode, uid, gid, attributes) = disk.meta(ino)?.describe(stat.kind);
    Ok(FileInfo {
        kind: match stat.kind {
            Kind::File => FileKind::File,
            Kind::Dir => FileKind::Dir,
            Kind::Symlink => FileKind::Symlink,
        },
        size: stat.size,
        mode,
        uid,
        gid,
        attributes,
        modified: UNIX_EPOCH + Duration::from_nanos(stat.modified),
        link: (stat.kind == Kind::Symlink).then(|| disk.read_link(ino)).transpose()?,
        id: stat.ino,
        links: stat.links,
    })
}

pub(crate) fn info_json(i: &FileInfo) -> Value {
    json!({
        "kind": match i.kind { FileKind::File => "file", FileKind::Dir => "dir", FileKind::Symlink => "symlink" },
        "size": i.size, "mode": i.mode, "uid": i.uid, "gid": i.gid, "attributes": i.attributes,
        "modified": i.modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64),
        "link": i.link, "id": i.id, "links": i.links,
    })
}

fn info_from_json(v: &Value) -> Option<FileInfo> {
    let u = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(FileInfo {
        kind: match v.get("kind")?.as_str()? {
            "dir" => FileKind::Dir,
            "symlink" => FileKind::Symlink,
            _ => FileKind::File,
        },
        size: u("size"),
        mode: u("mode") as u32,
        uid: u("uid") as u32,
        gid: u("gid") as u32,
        attributes: u("attributes") as u32,
        modified: UNIX_EPOCH + Duration::from_nanos(u("modified")),
        link: v.get("link").and_then(Value::as_str).map(String::from),
        id: u("id"),
        links: u("links") as u32,
    })
}

/// Folders along `path`, made where they aren't there: each `mode` (or `0755`), `owner`'s.
fn make_dirs<M: Meta>(disk: &mut Fs<M>, path: &str, meta: &M, at: u64) -> Result<Ino> {
    let mut dir = ROOT;
    for name in path.split('/').filter(|n| !n.is_empty()) {
        dir = match disk.lookup(dir, name) {
            Ok(ino) => match disk.stat(ino).map_err(|e| failed(path, e))?.kind {
                Kind::Dir => ino,
                Kind::Symlink => disk.resolve(dir, name, true).map_err(|e| failed(path, e))?,
                Kind::File => return Err(failed(path, layers::Error::NotDir)),
            },
            Err(layers::Error::NotFound) => disk.create(dir, name, New::Dir, meta.clone(), at).map_err(|e| failed(path, e))?,
            Err(e) => return Err(failed(path, e)),
        };
    }
    Ok(dir)
}

fn remove_tree<M: Meta>(disk: &mut Fs<M>, dir: Ino, name: &str, path: &str, at: u64) -> Result<()> {
    let ino = disk.lookup(dir, name).map_err(|e| failed(path, e))?;
    if disk.stat(ino).map_err(|e| failed(path, e))?.kind == Kind::Dir {
        for (child, _, _) in disk.entries(ino).map_err(|e| failed(path, e))? {
            remove_tree(disk, ino, &child, path, at)?;
        }
    }
    disk.remove(dir, name, at).map_err(|e| failed(path, e))
}

/// Does `op` on `disk`: its answer's bytes (JSON, or a read's bytes). `data` is a write's.
pub(crate) fn apply<M: Meta>(disk: &mut Fs<M>, op: &Op, data: &[u8]) -> Result<Vec<u8>> {
    let at = now();
    let done = || Ok(b"{}".to_vec());
    let owner = |disk: &Fs<M>, owner: &Option<String>| owner.as_deref().map(|o| M::owner(disk, o)).transpose();
    match op {
        Op::Stat { path } => {
            let ino = disk.resolve(ROOT, path, false).map_err(|e| failed(path, e))?;
            Ok(info_json(&info(disk, ino).map_err(|e| failed(path, e))?).to_string().into_bytes())
        }
        Op::List { path } => {
            let dir = disk.resolve(ROOT, path, true).map_err(|e| failed(path, e))?;
            let mut list = Vec::new();
            for (name, ino, _) in disk.entries(dir).map_err(|e| failed(path, e))? {
                let mut entry = info_json(&info(disk, ino).map_err(|e| failed(path, e))?);
                entry["name"] = json!(name);
                list.push(entry);
            }
            Ok(Value::Array(list).to_string().into_bytes())
        }
        Op::Read { path, offset, len } => {
            let ino = disk.resolve(ROOT, path, true).map_err(|e| failed(path, e))?;
            let stat = disk.stat(ino).map_err(|e| failed(path, e))?;
            if stat.kind == Kind::Dir {
                return Err(failed(path, layers::Error::IsDir));
            }
            let len = (*len).min(MOST_READ).min(stat.size.saturating_sub(*offset)) as usize;
            let mut buffer = vec![0; len];
            let n = disk.read(ino, *offset, &mut buffer).map_err(|e| failed(path, e))?;
            buffer.truncate(n);
            Ok(buffer)
        }
        Op::Write { path, mode, owner: who, parents, append, modified, .. } => {
            let (uid, gid) = owner(disk, who)?.unwrap_or((0, 0));
            let (parent_path, name) = split(path)?;
            let dir = if *parents { make_dirs(disk, parent_path, &M::make(Kind::Dir, 0o755, uid, gid), at)? } else { disk.resolve(ROOT, parent_path, true).map_err(|e| failed(path, e))? };
            let ino = match disk.resolve(dir, name, true) {
                Ok(ino) => {
                    if disk.stat(ino).map_err(|e| failed(path, e))?.kind == Kind::Dir {
                        return Err(failed(path, layers::Error::IsDir));
                    }
                    if !*append {
                        disk.truncate(ino, 0, at).map_err(|e| failed(path, e))?;
                    }
                    // Mode and owner as asked; else as they were.
                    if mode.is_some() || who.is_some() {
                        let kind = disk.stat(ino).map_err(|e| failed(path, e))?.kind;
                        let (old_mode, old_uid, old_gid, _) = disk.meta(ino).map_err(|e| failed(path, e))?.describe(kind);
                        let (uid, gid) = if who.is_some() { (uid, gid) } else { (old_uid, old_gid) };
                        disk.set_meta(ino, M::make(kind, mode.unwrap_or(old_mode), uid, gid)).map_err(|e| failed(path, e))?;
                    }
                    ino
                }
                Err(layers::Error::NotFound) => disk.create(dir, name, New::File, M::make(Kind::File, mode.unwrap_or(0o644), uid, gid), at).map_err(|e| failed(path, e))?,
                Err(e) => return Err(failed(path, e)),
            };
            let end = disk.stat(ino).map_err(|e| failed(path, e))?.size;
            disk.write(ino, if *append { end } else { 0 }, data, at).map_err(|e| failed(path, e))?;
            if let Some(m) = modified {
                disk.set_modified(ino, *m).map_err(|e| failed(path, e))?;
            }
            done()
        }
        Op::Mkdir { path, mode, owner: who, parents, modified } => {
            let (uid, gid) = owner(disk, who)?.unwrap_or((0, 0));
            let meta = M::make(Kind::Dir, mode.unwrap_or(0o755), uid, gid);
            let ino = if *parents {
                make_dirs(disk, path, &meta, at)?
            } else {
                let (parent_path, name) = split(path)?;
                let dir = disk.resolve(ROOT, parent_path, true).map_err(|e| failed(path, e))?;
                disk.create(dir, name, New::Dir, meta.clone(), at).map_err(|e| failed(path, e))?
            };
            // A folder there already, made by a copy: its mode and owner as asked.
            if mode.is_some() || who.is_some() {
                disk.set_meta(ino, meta).map_err(|e| failed(path, e))?;
            }
            if let Some(m) = modified {
                disk.set_modified(ino, *m).map_err(|e| failed(path, e))?;
            }
            done()
        }
        Op::Symlink { path, target, owner: who } => {
            let (uid, gid) = owner(disk, who)?.unwrap_or((0, 0));
            let (parent_path, name) = split(path)?;
            let dir = disk.resolve(ROOT, parent_path, true).map_err(|e| failed(path, e))?;
            // What's there goes, as a copy replaces a file.
            if disk.lookup(dir, name).is_ok() {
                remove_tree(disk, dir, name, path, at)?;
            }
            disk.create(dir, name, New::Symlink(target.clone()), M::make(Kind::Symlink, 0o777, uid, gid), at).map_err(|e| failed(path, e))?;
            done()
        }
        Op::Link { path, to } => {
            let ino = disk.resolve(ROOT, to, false).map_err(|e| failed(to, e))?;
            let (parent_path, name) = split(path)?;
            let dir = disk.resolve(ROOT, parent_path, true).map_err(|e| failed(path, e))?;
            if disk.lookup(dir, name).is_ok_and(|there| there != ino) {
                remove_tree(disk, dir, name, path, at)?;
            }
            if disk.lookup(dir, name).is_err() {
                disk.link(dir, name, ino, at).map_err(|e| failed(path, e))?;
            }
            done()
        }
        Op::Rename { from, to } => {
            let (from_path, from_name) = split(from)?;
            let (to_path, to_name) = split(to)?;
            let from_dir = disk.resolve(ROOT, from_path, true).map_err(|e| failed(from, e))?;
            let to_dir = disk.resolve(ROOT, to_path, true).map_err(|e| failed(to, e))?;
            disk.rename(from_dir, from_name, to_dir, to_name, at).map_err(|e| failed(from, e))?;
            done()
        }
        Op::Remove { path, recursive } => {
            let (parent_path, name) = split(path)?;
            let dir = disk.resolve(ROOT, parent_path, true).map_err(|e| failed(path, e))?;
            if *recursive {
                remove_tree(disk, dir, name, path, at)?;
            } else {
                disk.remove(dir, name, at).map_err(|e| failed(path, e))?;
            }
            done()
        }
    }
}

/// A path's folder and last name.
fn split(path: &str) -> Result<(&str, &str)> {
    let path = path.trim_end_matches('/');
    match path.rsplit_once('/') {
        Some((dir, name)) if !name.is_empty() => Ok((if dir.is_empty() { "/" } else { dir }, name)),
        _ => Err(Error::invalid(format!("{path}: a path from the machine's root, as /etc/hosts"))),
    }
}

/// A Windows machine's path as its disk has it: `C:\Program Files\App` is `/Program Files/App`.
pub(crate) fn disk_path(kernel: Kernel, path: &str) -> String {
    match kernel {
        Kernel::Lambda => path.to_string(),
        Kernel::Atlas => {
            let path = path.strip_prefix("C:").or_else(|| path.strip_prefix("c:")).unwrap_or(path).replace('\\', "/");
            if path.starts_with('/') { path } else { format!("/{path}") }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// A machine's files

/// Where operations go: a running machine's control socket, or a stopped one's disk, opened once and saved after.
enum Session {
    Running(PathBuf),
    Lambda(Box<lambda::Disk>, bool),
    Atlas(Box<atlas::Disk>, bool),
}

impl Session {
    fn open(store: &Store, name: &str) -> Result<(Session, Kernel)> {
        let record = store.get(name)?;
        let kernel = record.kernel;
        if record.running() {
            return Ok((Session::Running(store.dir(name)), kernel));
        }
        Ok((Session::stopped(store, &record)?, kernel))
    }

    fn stopped(store: &Store, record: &Record) -> Result<Session> {
        Ok(match record.kernel {
            Kernel::Lambda => Session::Lambda(Box::new(store.disk(record, lambda::Meta::root(), lambda::Meta::FOLD_CASE)?), false),
            Kernel::Atlas => Session::Atlas(Box::new(store.disk(record, atlas::Meta::root(), atlas::Meta::FOLD_CASE)?), false),
        })
    }

    fn op(&mut self, op: &Op, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Session::Running(dir) => crate::wire::fs(dir, &op.to_json(), data),
            Session::Lambda(disk, changed) => {
                *changed |= op.changes();
                apply(disk.as_mut(), op, data)
            }
            Session::Atlas(disk, changed) => {
                *changed |= op.changes();
                apply(disk.as_mut(), op, data)
            }
        }
    }

    fn stat(&mut self, path: &str) -> Result<FileInfo> {
        let bytes = self.op(&Op::Stat { path: path.to_string() }, &[])?;
        serde_json::from_slice::<Value>(&bytes).ok().as_ref().and_then(info_from_json).ok_or_else(|| Error::io("a file's information made no sense"))
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let bytes = self.op(&Op::List { path: path.to_string() }, &[])?;
        let list: Vec<Value> = serde_json::from_slice(&bytes).map_err(|_| Error::io("a folder's list made no sense"))?;
        Ok(list.iter().filter_map(|v| Some(DirEntry { name: v.get("name")?.as_str()?.to_string(), info: info_from_json(v)? })).collect())
    }

    /// Saves a stopped machine's disk, if anything changed it.
    fn finish(self) -> Result<()> {
        match self {
            Session::Lambda(mut disk, true) => disk.sync().map_err(|e| Error::io(format!("saving the machine's disk: {e}"))),
            Session::Atlas(mut disk, true) => disk.sync().map_err(|e| Error::io(format!("saving the machine's disk: {e}"))),
            _ => Ok(()),
        }
    }
}

/// A machine's files, running or stopped: [`crate::Machine::files`].
pub struct Files {
    pub(crate) carbon: crate::Carbon,
    pub(crate) name: String,
}

impl Files {
    /// One operation, in a session of its own.
    fn one(&self, op: Op, data: &[u8]) -> Result<Vec<u8>> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        let op = paths(op, kernel);
        let answer = session.op(&op, data)?;
        session.finish()?;
        Ok(answer)
    }

    /// What's known of the file at `path`; a symbolic link's own, not what it points to.
    pub fn stat(&self, path: &str) -> Result<FileInfo> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        session.stat(&disk_path(kernel, path))
    }

    /// What's in the folder at `path`.
    pub fn list(&self, path: &str) -> Result<Vec<DirEntry>> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        session.list(&disk_path(kernel, path))
    }

    /// Part of a file: `range` of its bytes (`..` for all of it), at most [`MOST_READ`] at once.
    pub fn read(&self, path: &str, range: impl std::ops::RangeBounds<u64>) -> Result<Vec<u8>> {
        use std::ops::Bound;
        let offset = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };
        let len = match range.end_bound() {
            Bound::Included(&n) => n + 1 - offset,
            Bound::Excluded(&n) => n.saturating_sub(offset),
            Bound::Unbounded => MOST_READ,
        };
        self.one(Op::Read { path: path.to_string(), offset, len }, &[])
    }

    /// A whole file, as text (bytes that aren't UTF-8 replaced).
    pub fn read_to_string(&self, path: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.read(path, ..)?).into_owned())
    }

    /// Writes a file: made if it isn't there, replaced (or appended to) if it is.
    pub fn write(&self, path: &str, data: impl AsRef<[u8]>, how: Write) -> Result<()> {
        let data = data.as_ref();
        let modified = how.modified.and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64);
        let op = Op::Write { path: path.to_string(), len: data.len() as u64, mode: how.mode, owner: how.owner, parents: how.parents, append: how.append, modified };
        self.one(op, data).map(|_| ())
    }

    /// Makes a folder, and the folders it's in.
    pub fn mkdir(&self, path: &str) -> Result<()> {
        self.one(Op::Mkdir { path: path.to_string(), mode: None, owner: None, parents: true, modified: None }, &[]).map(|_| ())
    }

    /// Moves or renames a file or folder.
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.one(Op::Rename { from: from.to_string(), to: to.to_string() }, &[]).map(|_| ())
    }

    /// Removes a file, a symbolic link, or a folder ([`Remove::recursive`] for one with something in it).
    pub fn remove(&self, path: &str, how: Remove) -> Result<()> {
        self.one(Op::Remove { path: path.to_string(), recursive: how.recursive }, &[]).map(|_| ())
    }

    /// Makes a symbolic link at `path`, to `target`.
    pub fn symlink(&self, path: &str, target: &str) -> Result<()> {
        self.one(Op::Symlink { path: path.to_string(), target: target.to_string(), owner: None }, &[]).map(|_| ())
    }

    /// Copies the host's file or folder `from` into the machine's folder `into`, under its own name (or, for a `from`
    /// ending in `/.`, what's in the folder). How many files, folders and links it copied.
    pub fn copy_in(&self, from: &Path, into: &str, how: &Copy) -> Result<usize> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        let into = disk_path(kernel, into).trim_end_matches('/').to_string();
        let contents = from.as_os_str().to_string_lossy().ends_with("/.");
        let n = copy_in(&mut session, from, &into, contents, how)?;
        session.finish()?;
        Ok(n)
    }

    /// Copies `other`'s file or folder `path` into this machine's folder `into`, under its own name (or, for a `path`
    /// ending in `/.`, what's in the folder), disk to disk with nothing on the host: kinds, modes, owners (unless
    /// [`Copy::chown`] names one), times and links kept. How many files, folders and links it copied. `COPY --from`.
    pub fn copy_from(&self, other: &Files, path: &str, into: &str, how: &Copy) -> Result<usize> {
        let (mut from, from_kernel) = Session::open(&other.carbon.0.store, &other.name)?;
        let (mut to, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        let contents = path.ends_with("/.") || path == ".";
        let source = disk_path(from_kernel, path.strip_suffix("/.").unwrap_or(path));
        let into = disk_path(kernel, into).trim_end_matches('/').to_string();
        let mut seen = std::collections::HashMap::new();
        let n = if contents {
            let mut n = 0;
            for entry in from.list(&source)? {
                let at = format!("{}/{}", source.trim_end_matches('/'), entry.name);
                n += copy_between(&mut from, &at, &mut to, &format!("{into}/{}", entry.name), &entry.name, how, &mut seen)?;
            }
            n
        } else {
            let name = source.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
            copy_between(&mut from, &source, &mut to, &format!("{into}/{name}"), "", how, &mut seen)?
        };
        to.finish()?;
        Ok(n)
    }

    /// Several operations in one session, the disk opened and saved once: what a build's `ADD` unpacks.
    pub(crate) fn batch(&self, f: impl FnOnce(&mut dyn FnMut(Op, &[u8]) -> Result<Vec<u8>>) -> Result<usize>) -> Result<usize> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        let n = f(&mut |op, data| session.op(&paths(op, kernel), data))?;
        session.finish()?;
        Ok(n)
    }

    /// Copies the machine's file or folder `path` into the host's folder `to`, under its own name. How many files,
    /// folders and links it copied.
    pub fn copy_out(&self, path: &str, to: &Path, how: &Copy) -> Result<usize> {
        let (mut session, kernel) = Session::open(&self.carbon.0.store, &self.name)?;
        let n = copy_out(&mut session, &disk_path(kernel, path), to, how)?;
        session.finish()?;
        Ok(n)
    }
}

/// An operation's paths as the machine's disk has them.
fn paths(op: Op, kernel: Kernel) -> Op {
    let p = |path: String| disk_path(kernel, &path);
    match op {
        Op::Stat { path } => Op::Stat { path: p(path) },
        Op::List { path } => Op::List { path: p(path) },
        Op::Read { path, offset, len } => Op::Read { path: p(path), offset, len },
        Op::Write { path, len, mode, owner, parents, append, modified } => Op::Write { path: p(path), len, mode, owner, parents, append, modified },
        Op::Mkdir { path, mode, owner, parents, modified } => Op::Mkdir { path: p(path), mode, owner, parents, modified },
        Op::Symlink { path, target, owner } => Op::Symlink { path: p(path), target, owner },
        Op::Link { path, to } => Op::Link { path: p(path), to: p(to) },
        Op::Rename { from, to } => Op::Rename { from: p(from), to: p(to) },
        Op::Remove { path, recursive } => Op::Remove { path: p(path), recursive },
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Copies

fn modified_ns(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos() as u64)
}

#[cfg(unix)]
fn host_owner(meta: &std::fs::Metadata, how: &Copy) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    how.owner.clone().or_else(|| how.archive.then(|| format!("{}:{}", meta.uid(), meta.gid())))
}

#[cfg(not(unix))]
fn host_owner(_meta: &std::fs::Metadata, how: &Copy) -> Option<String> {
    how.owner.clone()
}

#[cfg(unix)]
fn host_mode(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn host_mode(meta: &std::fs::Metadata) -> u32 {
    match (meta.is_dir(), meta.permissions().readonly()) {
        (true, _) => 0o755,
        (false, true) => 0o444,
        (false, false) => 0o644,
    }
}

/// A host file's identity, for hard links: device and inode.
#[cfg(unix)]
fn host_id(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (meta.nlink() > 1).then(|| (meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn host_id(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

fn copy_in(session: &mut Session, from: &Path, into: &str, contents: bool, how: &Copy) -> Result<usize> {
    let mut seen = std::collections::HashMap::new();
    let at = |e: &dyn std::fmt::Display| Error::io(format!("{}: {e}", from.display()));
    // `Path::parent` skips a trailing `.`, so `src/.`'s folder is taken from its text.
    let trimmed = from.as_os_str().to_string_lossy().strip_suffix("/.").map(PathBuf::from);
    let from_dir = if contents { trimmed.as_deref().unwrap_or(from) } else { from };
    let meta = std::fs::symlink_metadata(from_dir).map_err(|e| at(&e))?;
    if contents {
        if !meta.is_dir() {
            return Err(Error::invalid(format!("{}: only a folder has contents to copy (PATH/.)", from.display())));
        }
        let mut n = 0;
        for entry in std::fs::read_dir(from_dir).map_err(|e| at(&e))? {
            let entry = entry.map_err(|e| at(&e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            n += copy_in_one(session, &entry.path(), &format!("{into}/{name}"), &name, how, &mut seen)?;
        }
        return Ok(n);
    }
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| Error::invalid(format!("{}: copy a file or a folder by its name", from.display())))?;
    copy_in_one(session, from, &format!("{}/{name}", if into == "/" { "" } else { into }), "", how, &mut seen)
}

/// `rel` is where `from` is under what's copied (empty for its top), for [`Copy::exclude`].
fn copy_in_one(session: &mut Session, from: &Path, path: &str, rel: &str, how: &Copy, seen: &mut std::collections::HashMap<(u64, u64), String>) -> Result<usize> {
    let at = |e: &dyn std::fmt::Display| Error::io(format!("{}: {e}", from.display()));
    let meta = std::fs::symlink_metadata(from).map_err(|e| at(&e))?;
    let (left_out, look_inside) = if rel.is_empty() { (false, true) } else { how.excluded(rel) };
    if left_out && !(meta.is_dir() && look_inside) {
        return Ok(0);
    }
    if left_out {
        // Excluded, but an exception may bring back something in it: only that is copied, its folders made for it.
        let mut n = 0;
        for entry in std::fs::read_dir(from).map_err(|e| at(&e))? {
            let entry = entry.map_err(|e| at(&e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            n += copy_in_one(session, &entry.path(), &format!("{path}/{name}"), &format!("{rel}/{name}"), how, seen)?;
        }
        return Ok(n);
    }
    let owner = host_owner(&meta, how);
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from).map_err(|e| at(&e))?.to_string_lossy().into_owned();
        session.op(&Op::Symlink { path: path.to_string(), target, owner }, &[])?;
        return Ok(1);
    }
    if meta.is_dir() {
        session.op(&Op::Mkdir { path: path.to_string(), mode: Some(host_mode(&meta)), owner: owner.clone(), parents: true, modified: None }, &[])?;
        let mut n = 1;
        for entry in std::fs::read_dir(from).map_err(|e| at(&e))? {
            let entry = entry.map_err(|e| at(&e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            n += copy_in_one(session, &entry.path(), &format!("{path}/{name}"), &rel, how, seen)?;
        }
        // Its time last, since what's put in it changes it.
        session.op(&Op::Mkdir { path: path.to_string(), mode: Some(host_mode(&meta)), owner, parents: true, modified: modified_ns(&meta) }, &[])?;
        return Ok(n);
    }
    if !meta.is_file() {
        // A device, a socket or a pipe: nothing a copy can carry.
        return Ok(0);
    }
    if let Some(id) = host_id(&meta) {
        if let Some(first) = seen.get(&id) {
            session.op(&Op::Link { path: path.to_string(), to: first.clone() }, &[])?;
            return Ok(1);
        }
        seen.insert(id, path.to_string());
    }
    let mut file = std::fs::File::open(from).map_err(|e| at(&e))?;
    let mode = Some(how.mode.unwrap_or_else(|| host_mode(&meta)));
    let mut chunk = vec![0; CHUNK];
    let mut first = true;
    loop {
        let n = read_full(&mut file, &mut chunk).map_err(|e| at(&e))?;
        if n == 0 && !first {
            break;
        }
        session.op(&Op::Write { path: path.to_string(), len: n as u64, mode, owner: owner.clone(), parents: true, append: !first, modified: None }, &chunk[..n])?;
        first = false;
        if n < CHUNK {
            break;
        }
    }
    // Its time last, once all of it's there.
    session.op(&Op::Write { path: path.to_string(), len: 0, mode, owner, parents: false, append: true, modified: modified_ns(&meta) }, &[])?;
    Ok(1)
}

/// Reads until `buffer` is full or the file ends.
/// One file, folder or link from one disk to another; `rel` is where it is under what's copied.
fn copy_between(from: &mut Session, path: &str, to: &mut Session, dest: &str, rel: &str, how: &Copy, seen: &mut std::collections::HashMap<u64, String>) -> Result<usize> {
    let info = from.stat(path)?;
    let (left_out, look_inside) = if rel.is_empty() { (false, true) } else { how.excluded(rel) };
    if left_out && !(info.kind == FileKind::Dir && look_inside) {
        return Ok(0);
    }
    let owner = Some(how.owner.clone().unwrap_or_else(|| format!("{}:{}", info.uid, info.gid)));
    let modified = info.modified.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos() as u64);
    match info.kind {
        FileKind::Symlink => {
            to.op(&Op::Symlink { path: dest.to_string(), target: info.link.clone().unwrap_or_default(), owner }, &[])?;
            Ok(1)
        }
        FileKind::Dir => {
            if !left_out {
                to.op(&Op::Mkdir { path: dest.to_string(), mode: Some(info.mode), owner: owner.clone(), parents: true, modified: None }, &[])?;
            }
            let mut n = usize::from(!left_out);
            for entry in from.list(path)? {
                let child = if rel.is_empty() { entry.name.clone() } else { format!("{rel}/{}", entry.name) };
                n += copy_between(from, &format!("{}/{}", path.trim_end_matches('/'), entry.name), to, &format!("{dest}/{}", entry.name), &child, how, seen)?;
            }
            if !left_out {
                to.op(&Op::Mkdir { path: dest.to_string(), mode: Some(info.mode), owner, parents: true, modified }, &[])?;
            }
            Ok(n)
        }
        FileKind::File => {
            if info.links > 1 {
                if let Some(first) = seen.get(&info.id) {
                    to.op(&Op::Link { path: dest.to_string(), to: first.clone() }, &[])?;
                    return Ok(1);
                }
                seen.insert(info.id, dest.to_string());
            }
            let mode = Some(how.mode.unwrap_or(info.mode));
            let mut offset = 0;
            let mut first = true;
            while first || offset < info.size {
                let bytes = if info.size == 0 { Vec::new() } else { from.op(&Op::Read { path: path.to_string(), offset, len: (info.size - offset).min(CHUNK as u64) }, &[])? };
                to.op(&Op::Write { path: dest.to_string(), len: bytes.len() as u64, mode, owner: owner.clone(), parents: true, append: !first, modified: None }, &bytes)?;
                first = false;
                if bytes.is_empty() {
                    break;
                }
                offset += bytes.len() as u64;
            }
            to.op(&Op::Write { path: dest.to_string(), len: 0, mode, owner, parents: false, append: true, modified }, &[])?;
            Ok(1)
        }
    }
}

fn read_full(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    let mut n = 0;
    while n < buffer.len() {
        match file.read(&mut buffer[n..])? {
            0 => break,
            got => n += got,
        }
    }
    Ok(n)
}

fn copy_out(session: &mut Session, path: &str, to: &Path, how: &Copy) -> Result<usize> {
    let name = path.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("root").to_string();
    std::fs::create_dir_all(to).map_err(|e| Error::io(format!("{}: {e}", to.display())))?;
    let mut seen = std::collections::HashMap::new();
    copy_out_one(session, path, &to.join(name), how, &mut seen)
}

fn copy_out_one(session: &mut Session, path: &str, dest: &Path, how: &Copy, seen: &mut std::collections::HashMap<u64, PathBuf>) -> Result<usize> {
    let at = |e: &dyn std::fmt::Display| Error::io(format!("{}: {e}", dest.display()));
    let info = session.stat(path)?;
    match info.kind {
        FileKind::Symlink => {
            let _ = std::fs::remove_file(dest);
            #[cfg(unix)]
            std::os::unix::fs::symlink(info.link.as_deref().unwrap_or_default(), dest).map_err(|e| at(&e))?;
            #[cfg(not(unix))]
            return Ok(0);
            #[cfg(unix)]
            return Ok(1);
        }
        FileKind::Dir => {
            std::fs::create_dir_all(dest).map_err(|e| at(&e))?;
            let mut n = 1;
            for entry in session.list(path)? {
                n += copy_out_one(session, &format!("{}/{}", path.trim_end_matches('/'), entry.name), &dest.join(&entry.name), how, seen)?;
            }
            settle(dest, &info, how, None);
            Ok(n)
        }
        FileKind::File => {
            if info.links > 1 {
                if let Some(first) = seen.get(&info.id) {
                    let _ = std::fs::remove_file(dest);
                    std::fs::hard_link(first, dest).map_err(|e| at(&e))?;
                    return Ok(1);
                }
                seen.insert(info.id, dest.to_path_buf());
            }
            use std::io::Write as _;
            let mut file = std::fs::File::create(dest).map_err(|e| at(&e))?;
            let mut offset = 0;
            while offset < info.size {
                let bytes = session.op(&Op::Read { path: path.to_string(), offset, len: (info.size - offset).min(MOST_READ) }, &[])?;
                if bytes.is_empty() {
                    break;
                }
                file.write_all(&bytes).map_err(|e| at(&e))?;
                offset += bytes.len() as u64;
            }
            settle(dest, &info, how, Some(&file));
            Ok(1)
        }
    }
}

/// A file or folder copied out given its mode, time, and (archived) owner, as far as this process may.
fn settle(dest: &Path, info: &FileInfo, how: &Copy, file: Option<&std::fs::File>) {
    let mode = if info.kind == FileKind::File { how.mode.unwrap_or(info.mode) } else { info.mode };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode));
        if how.archive {
            let _ = std::os::unix::fs::chown(dest, Some(info.uid), Some(info.gid));
        }
    }
    #[cfg(not(unix))]
    let _ = mode;
    match file {
        Some(file) => {
            let _ = file.set_modified(info.modified);
        }
        None => {
            if let Ok(dir) = std::fs::File::open(dest) {
                let _ = dir.set_modified(info.modified);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn disk() -> lambda::Disk {
        let mut disk = lambda::Disk::new(lambda::Meta::root(), false);
        disk.put("/etc/passwd", New::File, b"root:x:0:0::/root:/bin/sh\napp:x:1000:1000::/home/app:/bin/sh\n", lambda::Meta::file(0o644), lambda::Meta::dir(0o755)).unwrap();
        disk.put("/etc/group", New::File, b"root:x:0:\nstaff:x:50:\n", lambda::Meta::file(0o644), lambda::Meta::dir(0o755)).unwrap();
        disk
    }

    fn op(disk: &mut lambda::Disk, op: Op, data: &[u8]) -> Result<Vec<u8>> {
        // Each through JSON and back, as over the socket.
        apply(disk, &Op::from_json(&op.to_json()).unwrap(), data)
    }

    fn stat(disk: &mut lambda::Disk, path: &str) -> FileInfo {
        info_from_json(&serde_json::from_slice(&op(disk, Op::Stat { path: path.into() }, &[]).unwrap()).unwrap()).unwrap()
    }

    #[test]
    fn files_are_written_read_listed_and_removed() {
        let mut d = disk();
        let w = |path: &str, mode, owner: Option<&str>, parents, append| Op::Write { path: path.into(), len: 0, mode, owner: owner.map(String::from), parents, append, modified: Some(5_000_000_000) };
        op(&mut d, w("/srv/app/run.sh", Some(0o755), Some("app:staff"), true, false), b"#!/bin/sh\necho hi\n").unwrap();
        op(&mut d, w("/srv/app/run.sh", None, None, false, true), b"echo more\n").unwrap();
        let info = stat(&mut d, "/srv/app/run.sh");
        assert_eq!((info.kind, info.size, info.mode, info.uid, info.gid), (FileKind::File, 28, 0o755, 1000, 50));
        assert_eq!(info.modified, UNIX_EPOCH + Duration::from_secs(5));
        assert_eq!(op(&mut d, Op::Read { path: "/srv/app/run.sh".into(), offset: 10, len: 7 }, &[]).unwrap(), b"echo hi");
        // Replaced, not appended: its mode and owner stay.
        op(&mut d, w("/srv/app/run.sh", None, None, false, false), b"x").unwrap();
        assert_eq!((stat(&mut d, "/srv/app/run.sh").size, stat(&mut d, "/srv/app/run.sh").mode), (1, 0o755));
        assert!(op(&mut d, w("/nowhere/x", None, None, false, false), b"").is_err_and(|e| e.kind() == ErrorKind::NotFound));
        op(&mut d, Op::Symlink { path: "/srv/link".into(), target: "app/run.sh".into(), owner: None }, &[]).unwrap();
        op(&mut d, Op::Link { path: "/srv/hard".into(), to: "/srv/app/run.sh".into() }, &[]).unwrap();
        let list: Vec<Value> = serde_json::from_slice(&op(&mut d, Op::List { path: "/srv".into() }, &[]).unwrap()).unwrap();
        let mut names: Vec<&str> = list.iter().map(|v| v["name"].as_str().unwrap()).collect();
        names.sort_unstable();
        assert_eq!(names, ["app", "hard", "link"]);
        assert_eq!(stat(&mut d, "/srv/link").link.as_deref(), Some("app/run.sh"));
        assert_eq!(stat(&mut d, "/srv/hard").links, 2);
        op(&mut d, Op::Rename { from: "/srv/hard".into(), to: "/srv/app/also".into() }, &[]).unwrap();
        assert!(op(&mut d, Op::Remove { path: "/srv/app".into(), recursive: false }, &[]).is_err());
        op(&mut d, Op::Remove { path: "/srv/app".into(), recursive: true }, &[]).unwrap();
        assert!(op(&mut d, Op::Stat { path: "/srv/app".into() }, &[]).is_err_and(|e| e.kind() == ErrorKind::NotFound));
        assert!(op(&mut d, Op::Write { path: "/x".into(), len: 0, mode: None, owner: Some("nobody".into()), parents: false, append: false, modified: None }, b"").is_err());
    }

    #[test]
    fn windows_paths_and_attributes() {
        assert_eq!(disk_path(Kernel::Atlas, r"C:\Program Files\App"), "/Program Files/App");
        assert_eq!(disk_path(Kernel::Atlas, r"Windows\x"), "/Windows/x");
        assert_eq!(disk_path(Kernel::Lambda, "/etc/hosts"), "/etc/hosts");
        let mut d = atlas::Disk::new(atlas::Meta::root(), true);
        apply(&mut d, &Op::Write { path: "/Data/Notes.txt".into(), len: 2, mode: Some(0o444), owner: None, parents: true, append: false, modified: None }, b"hi").unwrap();
        let ino = d.resolve(ROOT, "/data/notes.TXT", false).unwrap();
        assert_eq!(info(&d, ino).unwrap().attributes, ATTRIBUTE_READONLY | ATTRIBUTE_ARCHIVE);
        assert_eq!(info(&d, ino).unwrap().mode, 0o444);
    }

    #[cfg(unix)]
    #[test]
    fn copies_keep_modes_links_and_times() {
        use std::os::unix::fs::PermissionsExt;
        let host = std::env::temp_dir().join(format!("carbon-files-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&host);
        std::fs::create_dir_all(host.join("app/sub")).unwrap();
        std::fs::write(host.join("app/run.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(host.join("app/run.sh"), std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::write(host.join("app/sub/big.bin"), vec![7u8; 9 << 20]).unwrap();
        std::fs::hard_link(host.join("app/run.sh"), host.join("app/again.sh")).unwrap();
        std::os::unix::fs::symlink("sub/big.bin", host.join("app/big")).unwrap();
        let mut session = Session::Lambda(Box::new(disk()), false);
        assert_eq!(copy_in(&mut session, &host.join("app"), "/opt", false, &Copy::new().chown("app")).unwrap(), 6);
        let Session::Lambda(d, _) = &mut session else { unreachable!() };
        let run = d.resolve(ROOT, "/opt/app/run.sh", false).unwrap();
        let i = info(d, run).unwrap();
        assert_eq!((i.mode, i.uid, i.links), (0o750, 1000, 2));
        assert_eq!(d.resolve(ROOT, "/opt/app/again.sh", false).unwrap(), run);
        assert_eq!(d.read_link(d.resolve(ROOT, "/opt/app/big", false).unwrap()).unwrap(), "sub/big.bin");
        assert_eq!(d.stat(d.resolve(ROOT, "/opt/app/sub/big.bin", false).unwrap()).unwrap().size, 9 << 20);
        let out = host.join("out");
        assert_eq!(copy_out(&mut session, "/opt/app", &out, &Copy::new()).unwrap(), 6);
        assert_eq!(std::fs::metadata(out.join("app/run.sh")).unwrap().permissions().mode() & 0o7777, 0o750);
        assert_eq!(std::fs::read_link(out.join("app/big")).unwrap(), PathBuf::from("sub/big.bin"));
        assert_eq!(std::fs::read(out.join("app/sub/big.bin")).unwrap().len(), 9 << 20);
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(out.join("app/run.sh")).unwrap().ino(), std::fs::metadata(out.join("app/again.sh")).unwrap().ino());
        assert_eq!(std::fs::metadata(out.join("app/run.sh")).unwrap().modified().unwrap(), std::fs::metadata(host.join("app/run.sh")).unwrap().modified().unwrap());
        let _ = std::fs::remove_dir_all(&host);
    }

    /// A tar of a root file system with `/etc/passwd` and a file, for an image.
    pub(crate) fn rootfs_tar(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (path, data, mode) in files {
            let mut h = vec![0u8; 512];
            h[..path.len()].copy_from_slice(path.as_bytes());
            h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
            h[108..115].copy_from_slice(b"0000000");
            h[116..123].copy_from_slice(b"0000000");
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[136..147].copy_from_slice(b"00000000000");
            h[156] = b'0';
            h[257..262].copy_from_slice(b"ustar");
            let sum: u32 = h.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { u32::from(*b) }).sum();
            h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
            tar.extend_from_slice(&h);
            tar.extend_from_slice(data);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        tar.resize(tar.len() + 1024, 0);
        tar
    }

    /// Made without running, copied into (some of it excluded), copied disk to disk, frozen unnamed with a config.
    #[test]
    fn created_copied_and_frozen() {
        let home = std::env::temp_dir().join(format!("carbon-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let carbon = crate::Carbon::open_at(&home).unwrap();
        let tar = rootfs_tar(&[("etc/passwd", b"root:x:0:0::/root:/bin/sh\napp:x:1000:1000::/home/app:/bin/sh\n", 0o644), ("etc/group", b"root:x:0:\napp:x:1000:\n", 0o644)]);
        carbon.import("base:1", &tar, Some(images::Platform::linux_here()), &[]).unwrap();

        let ctx = home.join("ctx");
        std::fs::create_dir_all(ctx.join("src/node_modules/x")).unwrap();
        std::fs::create_dir_all(ctx.join("src/logs")).unwrap();
        std::fs::write(ctx.join("src/main.py"), "print(1)\n").unwrap();
        std::fs::write(ctx.join("src/node_modules/x/a.js"), "x").unwrap();
        std::fs::write(ctx.join("src/logs/a.log"), "x").unwrap();
        std::fs::write(ctx.join("src/logs/keep.log"), "kept").unwrap();

        let a = carbon.machine("base:1").name("a").create().unwrap();
        assert!(!a.is_running());
        let ignore = crate::build::Patterns::parse("src/node_modules\n**/*.log\n!src/logs/keep.log\n");
        let n = a.copy_in(ctx.join("src/."), "/app", &Copy::new().chown("app").exclude_under(ignore, "src")).unwrap();
        let files = a.files();
        assert_eq!(files.read_to_string("/app/main.py").unwrap(), "print(1)\n");
        assert_eq!(files.read_to_string("/app/logs/keep.log").unwrap(), "kept");
        assert!(files.stat("/app/node_modules").is_err());
        assert!(files.stat("/app/logs/a.log").is_err());
        assert_eq!(files.stat("/app/main.py").unwrap().uid, 1000);
        assert_eq!(n, 3, "main.py, logs/ and keep.log");

        // Disk to disk, owners kept.
        let b = carbon.machine("base:1").name("b").create().unwrap();
        b.files().copy_from(&a.files(), "/app", "/srv", &Copy::new()).unwrap();
        assert_eq!(b.files().read_to_string("/srv/app/main.py").unwrap(), "print(1)\n");
        assert_eq!(b.files().stat("/srv/app/logs/keep.log").unwrap().uid, 1000);

        // Frozen unnamed, with a config and a step; found by its id, never listed.
        let config = images::Config { workdir: "/srv/app".into(), ..images::Config::default() };
        let image = b.freeze_with(&crate::Freeze::new().config(config).step("COPY --from=a /app /srv")).unwrap();
        assert!(image.name.starts_with("sha256:"));
        assert_eq!(image.history.last().unwrap().created_by, "COPY --from=a /app /srv");
        assert_eq!(carbon.image(&image.name).unwrap().config.workdir, "/srv/app");
        assert!(carbon.images().iter().all(|i| !i.name.starts_with("sha256:")));
        let c = carbon.machine(&image.name).name("c").create().unwrap();
        assert_eq!(c.files().read_to_string("/srv/app/logs/keep.log").unwrap(), "kept");
        std::fs::remove_dir_all(&home).unwrap();
    }

}
