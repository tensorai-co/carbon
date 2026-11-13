//! Files: Windows paths onto the machine's disk (`layers`, folding case as Windows does), open files, and the system
//! layer, where `C:\Windows\System32` has atlas's DLLs as files that nothing stores: reading one reads the image atlas
//! made in memory.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use exec::Isa;

use kernels_core::lock;
use layers::{Ino, Kind, New};

use crate::process::Task;
use crate::{Disk, Meta, images, os};

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_SHARING_VIOLATION: u32 = 32;
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
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// A symbolic link's reparse tag, as `FindFirstFile` gives it.
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xa000_000c;

/// `FILE_SHARE_*`: what others opening the same file may do while it's open.
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const FILE_SHARE_DELETE: u32 = 4;
pub const FILE_SHARE_ALL: u32 = 7;
/// Where a file's creation and last access times are kept (its last write is the disk's own): 16 bytes, nanoseconds
/// since 1970.
const TIMES: &str = "atlas.times";

/// Where System32 is, on the disk, and its DLLs' folder for 32-bit programs, SysWOW64, which WoW64 shows a 32-bit
/// program as its System32. Sysnative is how a 32-bit program names the real System32.
const SYSTEM32: &str = "/windows/system32";
const SYSWOW64: &str = "/windows/syswow64";
const SYSNATIVE: &str = "/windows/sysnative";
/// System32's folders WoW64 leaves alone: they're shared by both kinds of program.
const NOT_REDIRECTED: [&str; 6] = ["/catroot", "/catroot2", "/driverstore", "/drivers/etc", "/logfiles", "/spool"];

/// The rest of `path` past `folder` (both lower case on the disk's side), if it's in it or is it.
fn under<'a>(lower: &'a str, folder: &str) -> Option<&'a str> {
    lower.strip_prefix(folder).filter(|rest| rest.is_empty() || rest.starts_with('/'))
}
const DIRECTORIES: [&str; 17] = [
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
    // The shell's special folders a profile has (`os/shellapi.rs`).
    "/Users/carbon/Desktop",
    "/Users/carbon/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
    "/ProgramData/Microsoft/Windows/Start Menu/Programs/Startup",
    "/Users/Public/Desktop",
    "/Users/Public/Documents",
    "/Windows/Fonts",
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
    /// One end of an anonymous pipe.
    Pipe(crate::pipes::End),
    /// One end of a named pipe: a server's instance, or a client's connection to one.
    NamedPipe(crate::named_pipes::PipeEnd),
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
    /// Opened with `FILE_FLAG_OVERLAPPED`: its I/O says it's done through the OVERLAPPED, and its port.
    pub overlapped: bool,
    /// The completion port it's associated with, and its key there.
    pub port: Mutex<Option<(Arc<crate::ports::Port>, u64)>>,
    /// `SetFileCompletionNotificationModes`' flags.
    pub modes: std::sync::atomic::AtomicU32,
    /// What it was opened to do, as sharing sees it, and what it lets others do.
    pub sharing: Sharing,
    /// The kernel whose disk it holds, and the file, while it's open: its name can go, but not its data.
    held: OnceLock<(Weak<crate::Kernel>, Ino)>,
}

impl Drop for File {
    fn drop(&mut self) {
        if let Some((k, ino)) = self.held.get()
            && let Some(k) = k.upgrade()
        {
            lock(&k.disk).release(*ino);
        }
    }
}

/// Each open of one file, and how it shares.
pub type Opens = Vec<(Weak<File>, Sharing)>;

/// How an open file shares: what it's opened for (reading or running, writing, deleting) and what it lets others
/// open the file for. An open for none of them (attributes only) shares with everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sharing {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub share: u32,
}

impl Sharing {
    /// Whether it's checked at all: an open for none of reading, writing or deleting isn't, either way.
    fn counts(self) -> bool {
        self.read || self.write || self.delete
    }

    /// Whether these two can be open at once.
    fn fits(self, other: Sharing) -> bool {
        if !self.counts() || !other.counts() {
            return true;
        }
        let allows = |a: Sharing, b: Sharing| (!a.read || b.share & FILE_SHARE_READ != 0) && (!a.write || b.share & FILE_SHARE_WRITE != 0) && (!a.delete || b.share & FILE_SHARE_DELETE != 0);
        allows(self, other) && allows(other, self)
    }
}

impl File {
    /// An open file that isn't on the disk: a console stream, the console's input, or a pipe's end.
    pub fn device(node: Node, read: bool, write: bool) -> File {
        File::new(node, read, write, false, String::new(), String::new(), false, false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(node: Node, read: bool, write: bool, append: bool, path: String, disk: String, delete_on_close: bool, overlapped: bool) -> File {
        let sharing = Sharing { read, write, delete: false, share: FILE_SHARE_ALL };
        File { node, read, write, append, position: Mutex::new(0), path, disk, delete_on_close, overlapped, port: Mutex::new(None), modes: Default::default(), sharing, held: OnceLock::new() }
    }
}

/// One entry `FindFirstFile` found.
#[derive(Debug, Clone)]
pub struct Found {
    pub name: String,
    pub attributes: u32,
    pub size: u64,
    /// Nanoseconds since 1970.
    pub modified: u64,
    pub created: u64,
    pub accessed: u64,
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
    /// When it was made and last read, in nanoseconds since 1970: its last write, where nothing's kept.
    pub created: u64,
    pub accessed: u64,
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
        // A link says what it is (a reparse point, and a folder's link a folder) in its own attributes.
        Kind::Symlink => FILE_ATTRIBUTE_REPARSE_POINT | meta.attributes,
        _ if meta.attributes == 0 => FILE_ATTRIBUTE_NORMAL,
        _ => meta.attributes & !FILE_ATTRIBUTE_DIRECTORY,
    };
    let size = if stat.kind == Kind::File { stat.size } else { 0 };
    let (created, accessed) = kept_times(disk, ino).unwrap_or((stat.modified, stat.modified));
    Ok(Info { attributes, size, modified: stat.modified, created, accessed, index: ino, links: stat.links })
}

/// A file's creation and last access times, if they're kept.
fn kept_times(disk: &Disk, ino: Ino) -> Option<(u64, u64)> {
    let bytes = disk.get_xattr(ino, TIMES).ok()??;
    let word = |i: usize| bytes.get(i * 8..i * 8 + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes);
    Some((word(0)?, word(1)?))
}

fn keep_times(disk: &mut Disk, ino: Ino, created: u64, accessed: u64) -> Result<(), u32> {
    let bytes: Vec<u8> = created.to_le_bytes().into_iter().chain(accessed.to_le_bytes()).collect();
    disk.set_xattr(ino, TIMES, &bytes).map_err(layers_error)
}

/// A FILETIME as nanoseconds since 1970.
pub fn from_filetime(ft: u64) -> u64 {
    ft.saturating_sub(116_444_736_000_000_000).saturating_mul(100)
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

    /// A path the program gave, as a Windows path and where it is on the disk, after WoW64's redirection.
    pub fn resolve(&self, raw: &str) -> Result<(String, String), u32> {
        let (windows, disk) = resolve(&self.cwd_for_join(), raw)?;
        Ok(self.redirect(windows, disk))
    }

    /// WoW64's file system redirection, for a 32-bit program: System32 is SysWOW64 (but for the folders both kinds
    /// share, and while the thread has turned it off), and Sysnative is System32, always.
    fn redirect(&self, windows: String, disk: String) -> (String, String) {
        if self.isa != Isa::X86_32 {
            return (windows, disk);
        }
        let lower = disk.to_ascii_lowercase();
        let to = if under(&lower, SYSNATIVE).is_some() {
            Some((SYSNATIVE, "System32"))
        } else if let Some(rest) = under(&lower, SYSTEM32)
            && !self.control.fs_redirection_off.load(Ordering::Relaxed)
            && !NOT_REDIRECTED.iter().any(|f| under(rest, f).is_some())
        {
            Some((SYSTEM32, "SysWOW64"))
        } else {
            None
        };
        let Some((from, folder)) = to else { return (windows, disk) };
        let rest = &disk[from.len()..];
        (format!("C:\\Windows\\{folder}{}", rest.replace('/', "\\")), format!("/Windows/{folder}{rest}"))
    }

    /// Which ISA's DLLs a folder of the system layer has: SysWOW64's are 32-bit; System32's are the program's own,
    /// or for a 32-bit program looking past its redirection, x86-64's, the machine's native ones.
    fn system_isa(&self, lower: &str) -> Option<Isa> {
        if lower == SYSWOW64 {
            Some(Isa::X86_32)
        } else if lower == SYSTEM32 {
            Some(if self.isa == Isa::X86_32 { Isa::X86_64 } else { self.isa })
        } else {
            None
        }
    }

    /// A system-layer file: one of atlas's DLLs, as `System32` or `SysWOW64` has it.
    fn system_file(&self, disk_path: &str) -> Option<Arc<Vec<u8>>> {
        let lower = disk_path.to_ascii_lowercase();
        let (dir, name) = lower.rsplit_once('/')?;
        let isa = self.system_isa(dir)?;
        if name == "ntdll.dll" {
            return images::guest(isa).map(|image| Arc::new(image.to_vec()));
        }
        let dll = os::find(name)?;
        (os::DLLS[dll].name.to_ascii_lowercase() == name).then(|| self.k.images.get(isa, dll))
    }

    /// The system layer's entries in a folder: atlas's DLLs, in System32 and SysWOW64.
    fn system_listing(&self, disk_dir: &str) -> Vec<Found> {
        let Some(isa) = self.system_isa(&disk_dir.to_ascii_lowercase()) else { return Vec::new() };
        let mut found: Vec<Found> = os::DLLS
            .iter()
            .enumerate()
            .map(|(i, d)| Found { name: d.name.to_string(), attributes: FILE_ATTRIBUTE_ARCHIVE, size: self.k.images.get(isa, i).len() as u64, modified: 0, created: 0, accessed: 0 })
            .collect();
        if let Some(image) = images::guest(isa) {
            found.push(Found { name: "ntdll.dll".to_string(), attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0, created: 0, accessed: 0 });
        }
        found
    }

    /// What's at a path: attributes, size, time.
    /// A link is itself: its own attributes, as `GetFileAttributes` gives them.
    pub fn path_info(&self, raw: &str) -> Result<Info, u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let disk = lock(&self.k.disk);
        match disk.resolve(layers::ROOT, &disk_path, false) {
            Ok(ino) => info_of(&disk, ino),
            Err(e) => match self.system_file(&disk_path) {
                Some(image) => Ok(Info { attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0, created: 0, accessed: 0, index: 0, links: 1 }),
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
            Node::System(image) => Ok(Info { attributes: FILE_ATTRIBUTE_ARCHIVE, size: image.len() as u64, modified: 0, created: 0, accessed: 0, index: 0, links: 1 }),
            _ => Ok(Info { attributes: FILE_ATTRIBUTE_NORMAL, size: 0, modified: 0, created: 0, accessed: 0, index: 0, links: 1 }),
        }
    }

    /// `CreateFile`: opens or makes a file (or a folder, with backup semantics). The file, and whether it was already
    /// there.
    /// It's refused (ERROR_SHARING_VIOLATION) where another open of the same file doesn't share what this is opened
    /// for, or this doesn't share what that is.
    pub fn open_file(&self, raw: &str, access: u32, share: u32, disposition: u32, flags: u32) -> Result<(Arc<File>, bool), u32> {
        const GENERIC_READ: u32 = 0x8000_0000;
        const GENERIC_WRITE: u32 = 0x4000_0000;
        const GENERIC_ALL: u32 = 0x1000_0000;
        const FILE_READ_DATA: u32 = 1;
        const FILE_WRITE_DATA: u32 = 2;
        const FILE_APPEND_DATA: u32 = 4;
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const DELETE_ON_CLOSE: u32 = 0x0400_0000;
        const GENERIC_EXECUTE: u32 = 0x2000_0000;
        const FILE_EXECUTE: u32 = 0x20;
        const DELETE: u32 = 0x1_0000;
        let read = access & (GENERIC_READ | GENERIC_ALL | FILE_READ_DATA) != 0;
        let sharing = Sharing {
            read: read || access & (GENERIC_EXECUTE | FILE_EXECUTE) != 0,
            write: access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA) != 0,
            delete: access & (DELETE | GENERIC_ALL) != 0 || flags & DELETE_ON_CLOSE != 0,
            share: share & FILE_SHARE_ALL,
        };
        let write = access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA) != 0;
        let append = access & FILE_APPEND_DATA != 0 && access & (GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA) == 0;
        const OVERLAPPED: u32 = 0x4000_0000;
        let make = |node: Node, path: String, disk: String| {
            let mut file = File::new(node, read, write, append, path, disk, flags & DELETE_ON_CLOSE != 0, flags & OVERLAPPED != 0);
            file.sharing = sharing;
            Arc::new(file)
        };
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
                let now = self.k.host.wall();
                let ino = disk.create(dir, name, New::File, Meta { attributes: FILE_ATTRIBUTE_ARCHIVE }, now).map_err(layers_error)?;
                keep_times(&mut disk, ino, now, now)?;
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
            self.shares(ino, sharing)?;
            let file = make(Node::Dir(ino), path, disk_path);
            self.opened(&mut disk, ino, &file);
            return Ok((file, existed));
        }
        let readonly = disk.meta(ino).is_ok_and(|m| m.attributes & FILE_ATTRIBUTE_READONLY != 0);
        if write && readonly {
            return Err(ERROR_ACCESS_DENIED);
        }
        self.shares(ino, sharing)?;
        // CREATE_ALWAYS and TRUNCATE_EXISTING empty it.
        if existed && matches!(disposition, 2 | 5) {
            disk.truncate(ino, 0, self.k.host.wall()).map_err(layers_error)?;
        }
        let file = make(Node::File(ino), path, disk_path);
        self.opened(&mut disk, ino, &file);
        Ok((file, existed))
    }

    /// Whether a file can be opened as `sharing` says, with what has it open now.
    pub(crate) fn shares(&self, ino: Ino, sharing: Sharing) -> Result<(), u32> {
        // Never an upgrade here: a file dropped while the disk's lock is held would deadlock letting go of it.
        let mut open = lock(&self.k.open_files);
        let Some(files) = open.get_mut(&ino) else { return Ok(()) };
        files.retain(|(f, _)| f.strong_count() > 0);
        if files.iter().all(|(_, s)| s.fits(sharing)) { Ok(()) } else { Err(ERROR_SHARING_VIOLATION) }
    }

    /// A file that's open now: sharing will see it, and the disk keeps it while it is.
    fn opened(&self, disk: &mut Disk, ino: Ino, file: &Arc<File>) {
        lock(&self.k.open_files).entry(ino).or_default().push((Arc::downgrade(file), file.sharing));
        disk.hold(ino);
        let _ = file.held.set((Arc::downgrade(&self.k), ino));
    }

    /// Whether the file at `disk_path` (not following a link there) can be deleted or moved: everything that has it
    /// open shares deleting.
    fn deletable(&self, disk: &Disk, disk_path: &str) -> Result<(), u32> {
        match disk.resolve(layers::ROOT, disk_path, false) {
            Ok(ino) => self.shares(ino, Sharing { delete: true, share: FILE_SHARE_ALL, ..Sharing::default() }),
            Err(_) => Ok(()),
        }
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
            Node::Dir(_) | Node::Console(_) | Node::Pipe(_) | Node::NamedPipe(_) => return Err(ERROR_ACCESS_DENIED),
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
        // As Windows 10 deletes, the name goes now; what has the file open keeps it until it's closed.
        self.deletable(&disk, &disk_path)?;
        disk.remove(dir, name, self.k.host.wall()).map_err(layers_error)
    }

    pub fn create_directory(&self, raw: &str) -> Result<(), u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &disk_path)?;
        let now = self.k.host.wall();
        let ino = disk.create(dir, name, New::Dir, Meta { attributes: FILE_ATTRIBUTE_DIRECTORY }, now).map_err(layers_error)?;
        keep_times(&mut disk, ino, now, now)
    }

    pub fn remove_directory(&self, raw: &str) -> Result<(), u32> {
        let (_, disk_path) = self.resolve(raw)?;
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &disk_path)?;
        let ino = disk.lookup(dir, name).map_err(layers_error)?;
        // A folder, or a link to one, which goes as a folder does: the folder it leads to stays.
        let link_to_folder = disk.stat(ino).is_ok_and(|s| s.kind == Kind::Symlink) && disk.meta(ino).is_ok_and(|m| m.attributes & FILE_ATTRIBUTE_DIRECTORY != 0);
        if disk.stat(ino).map_err(layers_error)?.kind != Kind::Dir && !link_to_folder {
            return Err(ERROR_DIRECTORY);
        }
        self.deletable(&disk, &disk_path)?;
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
            self.deletable(&disk, &to)?;
        }
        // Moving needs what has the file open to share deleting, as on Windows.
        self.deletable(&disk, &from)?;
        disk.rename(from_dir, from_name, to_dir, to_name, self.k.host.wall()).map_err(layers_error)
    }

    pub fn copy_file(&self, from: &str, to: &str, fail_if_exists: bool) -> Result<(), u32> {
        let (from_file, _) = self.open_file(from, 0x8000_0000, FILE_SHARE_READ | FILE_SHARE_DELETE, 3, 0)?;
        let data = match &from_file.node {
            Node::File(ino) => lock(&self.k.disk).contents(*ino).map_err(layers_error)?.to_vec(),
            Node::System(image) => image.to_vec(),
            _ => return Err(ERROR_ACCESS_DENIED),
        };
        let (to_file, _) = self.open_file(to, 0x4000_0000, 0, if fail_if_exists { 1 } else { 2 }, 0)?;
        self.write_to_file(&to_file, &data, Some(0))?;
        Ok(())
    }

    /// A whole file's bytes, by its Windows path.
    pub fn read_whole(&self, raw: &str) -> Result<Vec<u8>, u32> {
        let (file, _) = self.open_file(raw, 0x8000_0000, FILE_SHARE_READ, 3, 0)?;
        match &file.node {
            Node::File(ino) => lock(&self.k.disk).contents(*ino).map_err(layers_error),
            Node::System(image) => Ok(image.to_vec()),
            _ => Err(ERROR_ACCESS_DENIED),
        }
    }

    /// Writes a whole file, made new (ERROR_FILE_EXISTS if it's there, with `new_only`) or replaced.
    pub fn write_whole(&self, raw: &str, bytes: &[u8], new_only: bool) -> Result<(), u32> {
        let (file, _) = self.open_file(raw, 0x4000_0000, 0, if new_only { 1 } else { 2 }, 0)?;
        self.write_to_file(&file, bytes, Some(0)).map(|_| ())
    }

    /// `CreateHardLink`: another name for a file (not a folder) on the disk.
    pub fn create_hard_link(&self, new: &str, existing: &str) -> Result<(), u32> {
        let ((_, new), (_, existing)) = (self.resolve(new)?, self.resolve(existing)?);
        if self.system_file(&existing).is_some() {
            return Err(ERROR_ACCESS_DENIED);
        }
        let mut disk = lock(&self.k.disk);
        let ino = disk.resolve(layers::ROOT, &existing, true).map_err(|e| if e == layers::Error::NotFound { ERROR_FILE_NOT_FOUND } else { layers_error(e) })?;
        if disk.stat(ino).map_err(layers_error)?.kind != Kind::File {
            return Err(ERROR_ACCESS_DENIED);
        }
        let (dir, name) = parent(&disk, &new)?;
        disk.link(dir, name, ino, self.k.host.wall()).map_err(|e| if e == layers::Error::Exists { ERROR_ALREADY_EXISTS } else { layers_error(e) })
    }

    /// `CreateSymbolicLink`: a link at `link` to `target`, a Windows path, absolute or from the link's folder. A
    /// folder's link (`directory`) is a folder as listings and `RemoveDirectory` see it. Opening through it reaches
    /// its target.
    pub fn create_symbolic_link(&self, link: &str, target: &str, directory: bool) -> Result<(), u32> {
        let (_, link) = self.resolve(link)?;
        let absolute = target.starts_with(['\\', '/']) || target.as_bytes().get(1) == Some(&b':');
        let target = if absolute { self.resolve(target)?.1 } else { target.replace('\\', "/") };
        let mut disk = lock(&self.k.disk);
        let (dir, name) = parent(&disk, &link)?;
        let attributes = if directory { FILE_ATTRIBUTE_DIRECTORY } else { 0 };
        let now = self.k.host.wall();
        let ino = disk.create(dir, name, New::Symlink(target), Meta { attributes }, now).map_err(|e| if e == layers::Error::Exists { ERROR_ALREADY_EXISTS } else { layers_error(e) })?;
        keep_times(&mut disk, ino, now, now)
    }

    /// `SetFileTime`: any of the times it's given (nanoseconds since 1970), of a file open on the disk.
    pub fn set_file_times(&self, file: &File, created: Option<u64>, accessed: Option<u64>, written: Option<u64>) -> Result<(), u32> {
        let (Node::File(ino) | Node::Dir(ino)) = file.node else { return Err(ERROR_INVALID_PARAMETER) };
        let mut disk = lock(&self.k.disk);
        let info = info_of(&disk, ino)?;
        if created.is_some() || accessed.is_some() {
            keep_times(&mut disk, ino, created.unwrap_or(info.created), accessed.unwrap_or(info.accessed))?;
        }
        if let Some(written) = written {
            disk.set_modified(ino, written).map_err(layers_error)?;
        }
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
                    found.push_back(Found { name: dot.to_string(), attributes: FILE_ATTRIBUTE_DIRECTORY, size: 0, modified: me.modified, created: me.created, accessed: me.accessed });
                }
            }
        }
        let mut names: Vec<String> = Vec::new();
        for (name, ino, _) in disk.entries(dir).map_err(|_| ERROR_PATH_NOT_FOUND)? {
            if matches(pattern, &name) {
                let info = info_of(&disk, ino)?;
                names.push(name.to_ascii_lowercase());
                found.push_back(Found { name, attributes: info.attributes, size: info.size, modified: info.modified, created: info.created, accessed: info.accessed });
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
