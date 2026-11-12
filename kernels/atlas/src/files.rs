//! Files: Windows paths onto the machine's disk (`layers`, folding case as Windows does), open files, and the system
//! layer, where `C:\Windows\System32` has atlas's DLLs as files that nothing stores: reading one reads the image atlas
//! made in memory.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use kernels_core::lock;
use layers::{Ino, Kind, New};

use crate::process::Task;
use crate::{Disk, Meta, images, os};

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
/// The disk's store failed under it.
pub const ERROR_IO_DEVICE: u32 = 1117;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NO_MORE_FILES: u32 = 18;
pub const ERROR_FILE_EXISTS: u32 = 80;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_NAME: u32 = 123;
pub const ERROR_DIR_NOT_EMPTY: u32 = 145;
/// A move or link between two volumes.
pub const ERROR_NOT_SAME_DEVICE: u32 = 17;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_DIRECTORY: u32 = 267;

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// Where System32 is, on the disk, and its DLLs' folder for 32-bit programs.
const SYSTEM32: &str = "/windows/system32";
const DIRECTORIES: [&str; 11] = [
    "/Windows",
    "/Windows/System32",
    "/Windows/System32/config",
    "/Windows/SysWOW64",
    "/Windows/Temp",
    "/Program Files",
    "/Program Files (x86)",
    "/ProgramData",
    "/Users/carbon/AppData/Roaming",
    "/Users/carbon/AppData/Local",
    "/Users/carbon/Documents",
];

/// Puts the system layer's folders on a fresh disk.
pub fn populate(disk: &mut Disk) {
    let dir = Meta { attributes: FILE_ATTRIBUTE_DIRECTORY };
    for path in DIRECTORIES {
        let _ = disk.put(path, New::Dir, &[], dir, dir);
    }
}

/// What an open file is.
pub enum Node {
    File(Ino),
    Dir(Ino),
    /// A file of the system layer: an image atlas made.
    System(Arc<Vec<u8>>),
    /// `NUL`: reads nothing, takes anything.
    Null,
    /// `CONOUT$` and `CONIN$`.
    Console(host::Stream),
    Input,
}

pub struct File {
    pub node: Node,
    pub read: bool,
    pub write: bool,
    /// Opened only to append (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`).
    pub append: bool,
    pub position: Mutex<u64>,
    /// Its full Windows path, and where it is on the disk.
    pub path: String,
    pub disk: String,
    pub delete_on_close: bool,
}

/// One entry `FindFirstFile` found.
#[derive(Debug, Clone)]
pub struct Found {
    pub name: String,
    pub attributes: u32,
    pub size: u64,
    /// Nanoseconds since 1970.
    pub modified: u64,
}

pub type Find = Mutex<VecDeque<Found>>;

/// A path as the disk spells it: each part in the case it was made with.
fn canonical(disk: &Disk, disk_path: &str) -> String {
    let mut at = layers::ROOT;
    let mut parts = Vec::new();
    for part in disk_path.split('/').filter(|p| !p.is_empty()) {
        let named = disk.entries(at).ok().and_then(|entries| entries.into_iter().find(|(name, _, _)| name.eq_ignore_ascii_case(part)));
        match named {
            Some((name, ino, _)) => {
                parts.push(name);
                at = ino;
            }
            None => parts.push(part.to_string()),
        }
    }
    format!("C:\\{}", parts.join("\\"))
}

/// What's known of a path.
#[derive(Debug, Clone, Copy)]
pub struct Info {
    pub attributes: u32,
    pub size: u64,
    pub modified: u64,
    /// Its file number, for `GetFileInformationByHandle`.
    pub index: u64,
    pub links: u32,
}

/// A time in nanoseconds since 1970 as a FILETIME: 100-nanosecond ticks since 1601.
pub fn filetime(ns: u64) -> u64 {
    ns / 100 + 116_444_736_000_000_000
}

/// A path the program gave, resolved: its full Windows path, and its path on the disk.
pub fn resolve(cwd: &str, raw: &str) -> Result<(String, String), u32> {
    let mut path = raw.replace('/', "\\");
    for prefix in ["\\\\?\\", "\\??\\"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            path = rest.to_string();
        }
    }
    if path.is_empty() || path.starts_with("\\\\") {
        return Err(ERROR_INVALID_NAME);
    }
    let joined = match path.as_bytes() {
        [letter, b':', rest @ ..] => {
            if !letter.eq_ignore_ascii_case(&b'c') {
                return Err(ERROR_PATH_NOT_FOUND);
            }
            let rest = String::from_utf8_lossy(rest).into_owned();
            if rest.starts_with('\\') { format!("C:{rest}") } else { format!("{cwd}\\{rest}") }
        }
        [b'\\', ..] => format!("C:{path}"),
        _ => format!("{cwd}\\{path}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined[2..].split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            // Windows drops trailing dots and spaces from names.
            p => parts.push(p.trim_end_matches(['.', ' '])),
        }
    }
    if parts.iter().any(|p| p.contains(['<', '>', '"', '|', '*', '?', ':'])) {
        return Err(ERROR_INVALID_NAME);
    }
    Ok((format!("C:\\{}", parts.join("\\")), format!("/{}", parts.join("/"))))
}

/// A device name the program can open by itself, as `NUL` or `CONOUT$`.
pub fn device(raw: &str) -> Option<Node> {
    let name = raw.rsplit(['\\', '/']).next().unwrap_or(raw).to_ascii_uppercase();
    match name.as_str() {
        "NUL" => Some(Node::Null),
        "CONOUT$" | "CON" => Some(Node::Console(host::Stream::Out)),
        "CONIN$" => Some(Node::Input),
        _ => None,
    }
}

pub(crate) fn layers_error(e: layers::Error) -> u32 {
    match e {
        layers::Error::NotFound => ERROR_FILE_NOT_FOUND,
        layers::Error::NotDir => ERROR_PATH_NOT_FOUND,
        layers::Error::IsDir => ERROR_ACCESS_DENIED,
        layers::Error::Exists => ERROR_ALREADY_EXISTS,
        layers::Error::NotEmpty => ERROR_DIR_NOT_EMPTY,
        layers::Error::Loop => ERROR_PATH_NOT_FOUND,
        layers::Error::BadName => ERROR_INVALID_NAME,
        layers::Error::CrossFs => ERROR_NOT_SAME_DEVICE,
        layers::Error::Io => ERROR_IO_DEVICE,
    }
}

/// The folder `disk` is in, and its name in it. A missing folder is ERROR_PATH_NOT_FOUND, as Windows says.
fn parent<'p>(disk: &Disk, path: &'p str) -> Result<(Ino, &'p str), u32> {
    disk.resolve_parent(layers::ROOT, path).map_err(|e| if e == layers::Error::NotFound { ERROR_PATH_NOT_FOUND } else { layers_error(e) })
}

fn info_of(disk: &Disk, ino: Ino) -> Result<Info, u32> {
    let stat = disk.stat(ino).map_err(layers_error)?;
    let meta = disk.meta(ino).map_err(layers_error)?;
    let attributes = match stat.kind {
        Kind::Dir => FILE_ATTRIBUTE_DIRECTORY | meta.attributes,
        _ if meta.attributes == 0 => FILE_ATTRIBUTE_NORMAL,
        _ => meta.attributes & !FILE_ATTRIBUTE_DIRECTORY,
    };
    let size = if stat.kind == Kind::Dir { 0 } else { stat.size };
    Ok(Info { attributes, size, modified: stat.modified, index: ino, links: stat.links })
}

/// Whether `name` matches a `FindFirstFile` pattern, with `*` and `?`, folding case.
pub fn matches(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first().is_some_and(|m| c.to_lowercase().eq(m.to_lowercase())) && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // `*.*` matches names without a dot too, as it always has.
    pattern == "*.*" || go(&p, &n)
}

impl Task {
    /// The process's current directory, as a full Windows path with no trailing backslash (but `C:\`).
    pub fn cwd(&self) -> String {
        lock(&self.process.state).cwd.clone()
    }

    fn cwd_for_join(&self) -> String {
        self.cwd().trim_end_matches('\\').to_string()
    }

    pub fn resolve(&self, raw: &str) -> Result<(String, String), u32> {
        resolve(&self.cwd_for_join(), raw)
    }

    /// A system-layer file: one of atlas's DLLs, as `System32` has it.
    fn system_file(&self, disk_path: &str) -> Option<Arc<Vec<u8>>> {
        let lower = disk_path.to_ascii_lowercase();
        let name = lower.strip_prefix(SYSTEM32)?.strip_prefix('/')?;
        if name == "ntdll.dll" {
            return images::guest(self.isa).map(|image| Arc::new(image.to_vec()));
        }
        let dll = os::find(name)?;
        (os::DLLS[dll].name.to_ascii_lowercase() == name).then(|| self.k.images.get(self.isa, dll))
    }

    /// The system layer's entries in a folder: atlas's DLLs, in System32.
    fn system_listing(&self, disk_dir: &str) -> Vec<Found> {
        if disk_dir.to_ascii_lowercase() != SYSTEM32 {
            return Vec::new();
        }
        let mut found: Vec<Found> = os::DLLS
            .iter()
            .enumerate()
            .map(|(i, d)| Found { name: d.name.to_string(), attributes: FILE_ATTRIBUTE_ARCHIVE, size: self.k.images.get(self.isa, i).len() as u64, modified: 0 })
            .collect();
        if let Some(image) = images::guest(self.isa) {
            found.push(Found { name: "ntdll.dll".to_string(), attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0 });
        }
        found
    }

    /// What's at a path: attributes, size, time.
    pub fn path_info(&self, raw: &str) -> Result<Info, u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let disk = lock(&self.k.disk);
        match disk.resolve(layers::ROOT, &disk_path, true) {
            Ok(ino) => info_of(&disk, ino),
            Err(e) => match self.system_file(&disk_path) {
                Some(image) => Ok(Info { attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0, index: 0, links: 1 }),
                None if e == layers::Error::NotFound => {
                    parent(&disk, &disk_path)?;
                    Err(ERROR_FILE_NOT_FOUND)
                }
                None => Err(layers_error(e)),
            },
        }
    }

    pub fn file_info(&self, file: &File) -> Result<Info, u32> {
        match &file.node {
            Node::File(ino) | Node::Dir(ino) => info_of(&lock(&self.k.disk), *ino),
            Node::System(image) => Ok(Info { attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0, index: 0, links: 1 }),
            _ => Ok(Info { attributes: FILE_ATTRIBUTE_NORMAL, size: 0, modified: 0, index: 0, links: 1 }),
        }
    }

    /// `CreateFile`: opens or makes a file (or a folder, with backup semantics). The file, and whether it was already
    /// there.
    pub fn open_file(&self, raw: &str, access: u32, disposition: u32, flags: u32) -> Result<(Arc<File>, bool), u32> {
        const GENERIC_READ: u32 = 0x8000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const GENERIC_ALL: u32 = 0x1000_0000;
        const FILE_READ_DATA: u32 = 1;
        const FILE_WRITE_DATA: u32 = 2;
        const FILE_APPEND_DATA: u32 = 4;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const DELETE_ON_CLOSE: u32 = 0x0400_0000;
        let read = access & (GENERIC_READ | GENERIC_ALL | FILE_READ_DATA) != 0;
        let write = access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA) != 0;
        let append = access & FILE_APPEND_DATA != 0 && access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA) == 0;
        let make = |node: Node, path: String, disk: String| Arc::new(File { node, read, write, append, position: Mutex::new(0), path, disk, delete_on_close: flags & DELETE_ON_CLOSE != 0 });
        if let Some(node) = device(raw) {
            return Ok((make(node, raw.to_string(), String::new()), true));
        }
        let (path, disk_path) = self.resolve(raw)?;
        if !(1..=5).contains(&disposition) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut disk = lock(&self.k.disk);
        let found = disk.resolve(layers::ROOT, &disk_path, true);
        if found.is_err()
            && let Some(image) = self.system_file(&disk_path)
        {
            if write || disposition == 1 {
                return Err(if disposition == 1 { ERROR_FILE_EXISTS } else { ERROR_ACCESS_DENIED });
            }
            return Ok((make(Node::System(image), path, disk_path), true));
        }
        let (ino, existed) = match (found, disposition) {
            // CREATE_NEW
            (Ok(_), 1) => return Err(ERROR_FILE_EXISTS),
            // OPEN_EXISTING, TRUNCATE_EXISTING
            (Err(layers::Error::NotFound), 3 | 5) => {
                parent(&disk, &disk_path)?;
                return Err(ERROR_FILE_NOT_FOUND);
            }
            (Err(layers::Error::NotFound), _) => {
                let (dir, name) = parent(&disk, &disk_path)?;
                let ino = disk.create(dir, name, New::File, Meta { attributes: FILE_ATTRIBUTE_ARCHIVE }, self.k.host.wall()).map_err(layers_error)?;
                (ino, false)
            }
            (Err(e), _) => return Err(layers_error(e)),
            (Ok(ino), _) => (ino, true),
        };
        let path = canonical(&disk, &disk_path);
        let kind = disk.stat(ino).map_err(layers_error)?.kind;
        if kind == Kind::Dir {
            if flags & BACKUP_SEMANTICS == 0 {
                return Err(ERROR_ACCESS_DENIED);
            }
            return Ok((make(Node::Dir(ino), path, disk_path), existed));
        }
        let readonly = disk.meta(ino).is_ok_and(|m| m.attributes & FILE_ATTRIBUTE_READONLY != 0);
        if write && readonly {
            return Err(ERROR_ACCESS_DENIED);
        }
        // CREATE_ALWAYS and TRUNCATE_EXISTING empty it.
        if existed && matches!(disposition, 2 | 5) {
            disk.truncate(ino, 0, self.k.host.wall()).map_err(layers_error)?;
        }
        Ok((make(Node::File(ino), path, disk_path), existed))
    }

    /// Reads from where the file's at (or at `at`), moving it on.
    pub fn read_from(&self, file: &File, len: usize, at: Option<u64>) -> Result<Vec<u8>, u32> {
        if !file.read {
            return Err(ERROR_ACCESS_DENIED);
        }
        let mut position = lock(&file.position);
        let start = at.unwrap_or(*position);
        let data = match &file.node {
            Node::File(ino) => {
                let mut buffer = vec![0; len];
                let n = lock(&self.k.disk).read(*ino, start, &mut buffer).map_err(layers_error)?;
                buffer.truncate(n);
                buffer
            }
            Node::System(image) => image.get(start as usize..).map_or(Vec::new(), |rest| rest[..len.min(rest.len())].to_vec()),
            Node::Null => Vec::new(),
            Node::Input => {
                let mut buffer = vec![0; len.min(1 << 16)];
                let n = self.k.host.read(&mut buffer);
                buffer.truncate(n);
                return Ok(buffer);
            }
            Node::Dir(_) | Node::Console(_) => return Err(ERROR_ACCESS_DENIED),
        };
        *position = start + data.len() as u64;
        Ok(data)
    }

    /// Writes where the file's at (or at `at`, or its end if it's only for appending), moving it on.
    pub fn write_to_file(&self, file: &File, data: &[u8], at: Option<u64>) -> Result<usize, u32> {
        if !file.write {
            return Err(ERROR_ACCESS_DENIED);
        }
        let mut position = lock(&file.position);
        match &file.node {
            Node::File(ino) => {
                let mut disk = lock(&self.k.disk);
                let start = if file.append { disk.stat(*ino).map_err(layers_error)?.size } else { at.unwrap_or(*position) };
                let n = disk.write(*ino, start, data, self.k.host.wall()).map_err(layers_error)?;
                *position = start + n as u64;
                Ok(n)
            }
            Node::Null => Ok(data.len()),
            Node::Console(stream) => {
                self.k.host.write(*stream, data);
                Ok(data.len())
            }
            _ => Err(ERROR_ACCESS_DENIED),
        }
    }

    /// Lets go of a handle to a file: one opened to delete when closed goes, once nothing has it open.
    pub fn closed_file(&self, file: Arc<File>) {
        if file.delete_on_close && Arc::strong_count(&file) == 1 {
            let mut disk = lock(&self.k.disk);
            if let Ok((dir, name)) = parent(&disk, &file.disk) {
                let _ = disk.remove(dir, name, self.k.host.wall());
            }
        }
    }

    pub fn delete_file(&self, raw: &str) -> Result<(), u32> {
        let (_, disk_path) = self.resolve(raw)?;
        if self.system_file(&disk_path).is_some() {
            return Err(ERROR_ACCESS_DENIED);
        }
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &disk_path)?;
        let ino = disk.lookup(dir, name).map_err(layers_error)?;
        let info = info_of(&disk, ino)?;
        if info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.attributes & FILE_ATTRIBUTE_READONLY != 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        disk.remove(dir, name, self.k.host.wall()).map_err(layers_error)
    }

    pub fn create_directory(&self, raw: &str) -> Result<(), u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &disk_path)?;
        disk.create(dir, name, New::Dir, Meta { attributes: FILE_ATTRIBUTE_DIRECTORY }, self.k.host.wall()).map(|_| ()).map_err(layers_error)
    }

    pub fn remove_directory(&self, raw: &str) -> Result<(), u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &disk_path)?;
        let ino = disk.lookup(dir, name).map_err(layers_error)?;
        if disk.stat(ino).map_err(layers_error)?.kind != Kind::Dir {
            return Err(ERROR_DIRECTORY);
        }
        disk.remove(dir, name, self.k.host.wall()).map_err(layers_error)
    }

    /// `MoveFileEx`: renames, replacing a file at the target only if asked.
    pub fn move_file(&self, from: &str, to: &str, replace: bool) -> Result<(), u32> {
        let ((_, from), (_, to)) = (self.resolve(from)?, self.resolve(to)?);
        let mut disk = lock(&self.k.disk);
        let (from_dir, from_name) = parent(&disk, &from)?;
        disk.lookup(from_dir, from_name).map_err(layers_error)?;
        let (to_dir, to_name) = parent(&disk, &to)?;
        if let Ok(existing) = disk.lookup(to_dir, to_name) {
            let same = disk.lookup(from_dir, from_name).is_ok_and(|i| i == existing);
            if !same && (!replace || disk.stat(existing).is_ok_and(|s| s.kind == Kind::Dir)) {
                return Err(ERROR_ALREADY_EXISTS);
            }
        }
        disk.rename(from_dir, from_name, to_dir, to_name, self.k.host.wall()).map_err(layers_error)
    }

    pub fn copy_file(&self, from: &str, to: &str, fail_if_exists: bool) -> Result<(), u32> {
        let (from_file, _) = self.open_file(from, 0x8000_0000, 3, 0)?;
        let data = match &from_file.node {
            Node::File(ino) => lock(&self.k.disk).contents(*ino).map_err(layers_error)?.to_vec(),
            Node::System(image) => image.to_vec(),
            _ => return Err(ERROR_ACCESS_DENIED),
        };
        let (to_file, _) = self.open_file(to, 0x4000_0000, if fail_if_exists { 1 } else { 2 }, 0)?;
        self.write_to_file(&to_file, &data, Some(0))?;
        Ok(())
    }

    /// `FindFirstFile`: every entry in the pattern's folder that matches it, with `.` and `..` first (but at the root).
    pub fn find(&self, raw: &str) -> Result<VecDeque<Found>, u32> {
        // The pattern is the last part; what's before it is a folder, resolved as any path is.
        let (dir_raw, pattern) = match raw.rfind(['\\', '/']) {
            Some(at) => (&raw[..=at], &raw[at + 1..]),
            None => (".", raw),
        };
        if pattern.is_empty() {
            return Err(ERROR_FILE_NOT_FOUND);
        }
        let (dir_full, dir_disk) = self.resolve(dir_raw)?;
        let disk = lock(&self.k.disk);
        let dir = disk.resolve(layers::ROOT, &dir_disk, true).map_err(|_| ERROR_PATH_NOT_FOUND)?;
        let mut found = VecDeque::new();
        if dir_full.len() > 3 {
            let me = info_of(&disk, dir)?;
            for dot in [".", ".."] {
                if matches(pattern, dot) {
                    found.push_back(Found { name: dot.to_string(), attributes: FILE_ATTRIBUTE_DIRECTORY, size: 0, modified: me.modified });
                }
            }
        }
        let mut names: Vec<String> = Vec::new();
        for (name, ino, _) in disk.entries(dir).map_err(|_| ERROR_PATH_NOT_FOUND)? {
            if matches(pattern, &name) {
                let info = info_of(&disk, ino)?;
                names.push(name.to_ascii_lowercase());
                found.push_back(Found { name, attributes: info.attributes, size: info.size, modified: info.modified });
            }
        }
        drop(disk);
        for entry in self.system_listing(&dir_disk) {
            if matches(pattern, &entry.name) && !names.contains(&entry.name.to_ascii_lowercase()) {
                found.push_back(entry);
            }
        }
        if found.is_empty() { Err(ERROR_FILE_NOT_FOUND) } else { Ok(found) }
    }

    pub fn set_current_directory(&self, raw: &str) -> Result<(), u32> {
        let (full, disk_path) = self.resolve(raw)?;
        let disk = lock(&self.k.disk);
        let ino = disk.resolve(layers::ROOT, &disk_path, true).map_err(|e| if e == layers::Error::NotFound { ERROR_FILE_NOT_FOUND } else { layers_error(e) })?;
        if disk.stat(ino).map_err(layers_error)?.kind != Kind::Dir {
            return Err(ERROR_DIRECTORY);
        }
        drop(disk);
        lock(&self.process.state).cwd = full;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let cwd = "C:\\work";
        assert_eq!(resolve(cwd, "a.txt").unwrap(), ("C:\\work\\a.txt".into(), "/work/a.txt".into()));
        assert_eq!(resolve(cwd, "C:\\x\\..\\y\\.\\z").unwrap(), ("C:\\y\\z".into(), "/y/z".into()));
        assert_eq!(resolve(cwd, "\\top").unwrap().1, "/top");
        assert_eq!(resolve(cwd, "c:rel").unwrap().1, "/work/rel");
        assert_eq!(resolve(cwd, "\\\\?\\C:\\long").unwrap().1, "/long");
        assert_eq!(resolve("C:", "C:\\").unwrap(), ("C:\\".into(), "/".into()));
        assert_eq!(resolve(cwd, "D:\\x"), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(resolve(cwd, "a*b"), Err(ERROR_INVALID_NAME));
    }

    #[test]
    fn wildcards() {
        assert!(matches("*.TXT", "notes.txt"));
        assert!(matches("k*.dll", "KERNEL32.dll"));
        assert!(matches("a?c", "abc") && !matches("a?c", "ac"));
        assert!(matches("*.*", "README"));
        assert!(!matches("*.txt", "notes.txt.bak"));
    }
}
