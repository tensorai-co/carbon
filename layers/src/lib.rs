//! The file system programs see, for any kernel. It knows only what's true of any file: its kind, size, times, contents,
//! names and links. What a file means to its kernel (owner and mode to lambda, attributes to atlas) is that kernel's
//! `M`, kept here but never read.
//!
//! The tree is kept in memory, names and metadata; a file's contents are kept in chunks of 64 KiB, each in memory or,
//! once it's been saved, by name in a [`Store`], from which it's read back as needed. A disk can be opened from an
//! image's index (the image's layers, merged, in the on-disk format `index.rs` describes) and a machine's store, whose log
//! (`log.rs`) holds every change since: that's the writable top layer, and the machine keeps its files. `sync` makes
//! the changes so far survive a crash; `freeze` writes the whole tree as a new index.
//!
//! A directory can fold case, as Windows' do: names that differ only in case are one name, and keep the case they
//! were given.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod index;
mod log;
pub mod store;
pub mod tar;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

pub use index::{INLINE_MAX, MAGIC};
use log::Record;
pub use store::{CHUNK, Hash, MemoryStore, Store};

/// A kernel's `M` as it's kept on disk: four words, laid out as that kernel decides.
pub trait Persist: Clone {
    fn save(&self) -> [u32; 4];
    fn load(words: [u32; 4]) -> Self;
}

impl Persist for () {
    fn save(&self) -> [u32; 4] {
        [0; 4]
    }
    fn load(_: [u32; 4]) -> Self {}
}

impl Persist for u32 {
    fn save(&self) -> [u32; 4] {
        [*self, 0, 0, 0]
    }
    fn load(words: [u32; 4]) -> Self {
        words[0]
    }
}

impl Persist for [u32; 4] {
    fn save(&self) -> [u32; 4] {
        *self
    }
    fn load(words: [u32; 4]) -> Self {
        words
    }
}

/// A file's number, which lasts as long as the file.
pub type Ino = u64;

/// The root directory.
pub const ROOT: Ino = 1;

/// How many symbolic links one lookup follows before giving up.
const MOST_LINKS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotFound,
    NotDir,
    IsDir,
    Exists,
    NotEmpty,
    /// Too many symbolic links, or a loop.
    Loop,
    /// A name that can't be one: empty, `.`, `..`, or with a `/`.
    BadName,
    /// A link or a move between a volatile tree and the rest, or another volatile tree.
    CrossFs,
    /// The store failed, or what's in it (an index, the log, a chunk) is corrupt.
    Io,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Error::NotFound => "not found",
            Error::NotDir => "not a directory",
            Error::IsDir => "is a directory",
            Error::Exists => "already there",
            Error::NotEmpty => "directory not empty",
            Error::Loop => "too many symbolic links",
            Error::BadName => "not a name a file can have",
            Error::CrossFs => "not in the same tree",
            Error::Io => "the disk's store failed, or what's in it is corrupt",
        })
    }
}

impl core::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;

/// What's true of a file, whatever kernel looks at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub ino: Ino,
    pub kind: Kind,
    pub size: u64,
    pub links: u32,
    /// Last change to its contents, in nanoseconds since 1970.
    pub modified: u64,
    /// Goes up with every change to its contents, so a cache keyed by it can't find what's stale.
    pub generation: u64,
}

/// A change to the tree, as a kernel watching it is told (inotify, `ReadDirectoryChangesW`): one for each call that
/// changed something, after it has. Whatever a change says of names is as they were given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change<'a> {
    /// `name` made in `dir`.
    Created { dir: Ino, name: &'a str, ino: Ino, kind: Kind },
    /// Another name, `name` in `dir`, for a file.
    Linked { dir: Ino, name: &'a str, ino: Ino },
    /// `name` taken out of `dir`, and whether that was the file's last name (a file held open is gone all the same:
    /// nothing can find it).
    Removed { dir: Ino, name: &'a str, ino: Ino, kind: Kind, gone: bool },
    /// `from` in `from_dir` moved to `to` in `to_dir`, and the file it replaced there, if any, and whether that's gone.
    Moved { from_dir: Ino, from: &'a str, to_dir: Ino, to: &'a str, ino: Ino, kind: Kind, replaced: Option<(Ino, bool)> },
    /// A file's contents (or size) changed.
    Written { ino: Ino },
    /// A file's `M` or modification time was set.
    Changed { ino: Ino },
}

/// Told of every change, with the tree as it is after it. It runs while the caller has the disk, so it mustn't wait
/// on anything that waits for the disk.
pub type Observer<M> = Box<dyn FnMut(&Fs<M>, Change<'_>) + Send>;

/// What to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum New {
    File,
    Dir,
    Symlink(String),
}

/// Once this much has been written and not saved, a disk with a store saves it.
const DIRTY_MOST: u64 = 16 << 20;
/// Chunks read back from the store, kept for the next read.
const CACHED: usize = 16;

#[derive(Debug, Clone)]
struct Node<M> {
    meta: M,
    modified: u64,
    generation: u64,
    links: u32,
    data: Data,
    /// The volatile tree it's in, by its root: never logged or saved (a kernel's tmpfs).
    volatile: Option<Ino>,
    /// Extended attributes, by name. Kept in the log; not yet in a frozen index.
    xattrs: BTreeMap<String, Vec<u8>>,
}

/// One chunk of a file.
#[derive(Debug, Clone)]
enum Chunk {
    /// Never written: zeroes.
    Hole,
    /// In the store, by name.
    Stored(Hash),
    /// In memory, and whether it's changed since it was last saved. Shorter than the chunk is: zeroes after it.
    Bytes(Vec<u8>, bool),
}

#[derive(Debug, Clone)]
struct File {
    size: u64,
    /// One for each `CHUNK` of `size`, rounded up.
    chunks: Vec<Chunk>,
}

#[derive(Debug, Clone)]
enum Data {
    File(File),
    /// Entries by their key (folded, if the directory folds case), each with the name as given.
    Dir { entries: BTreeMap<String, (String, Ino)>, parent: Ino, fold: bool },
    Symlink(String),
}

fn chunk_count(size: u64) -> usize {
    size.div_ceil(CHUNK) as usize
}

/// A file system: one tree of files, each with its kernel's `M`.
pub struct Fs<M> {
    nodes: BTreeMap<Ino, Node<M>>,
    next: Ino,
    /// Where chunks are saved and the log goes, if anywhere.
    store: Option<RefCell<Box<dyn Store>>>,
    /// Chunks read from the store lately, the latest last.
    cache: RefCell<Vec<(Hash, Vec<u8>)>>,
    /// How `M` is saved, once changes are logged.
    save: Option<fn(&M) -> [u32; 4]>,
    /// Files changed since they were last saved, each with the smallest size it's had since.
    dirty: BTreeMap<Ino, u64>,
    dirty_bytes: u64,
    /// Files a kernel has open, and how many times: one with no links left lives until it's let go.
    held: BTreeMap<Ino, u32>,
    /// Who's told of changes, if anyone.
    observer: Option<Observer<M>>,
}

impl<M> core::fmt::Debug for Fs<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fs").field("files", &self.nodes.len()).field("store", &self.store.is_some()).field("logging", &self.save.is_some()).finish()
    }
}

impl<M: Clone> Fs<M> {
    /// An empty file system in memory. If `fold_case`, its root and every directory made in it fold case.
    pub fn new(root: M, fold_case: bool) -> Fs<M> {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            ROOT,
            Node { meta: root, modified: 0, generation: 0, links: 2, data: Data::Dir { entries: BTreeMap::new(), parent: ROOT, fold: fold_case }, volatile: None, xattrs: BTreeMap::new() },
        );
        Fs { nodes, next: ROOT + 1, store: None, cache: RefCell::new(Vec::new()), save: None, dirty: BTreeMap::new(), dirty_bytes: 0, held: BTreeMap::new(), observer: None }
    }

    /// Tells `observer` of every change from now on (none: no one).
    pub fn observe(&mut self, observer: Option<Observer<M>>) {
        self.observer = observer;
    }

    fn tell(&mut self, change: Change<'_>) {
        if let Some(mut observer) = self.observer.take() {
            observer(self, change);
            self.observer = Some(observer);
        }
    }

    fn kind(&self, ino: Ino) -> Kind {
        self.stat(ino).map(|s| s.kind).unwrap_or(Kind::File)
    }

    /// The volatile tree `ino` is in, if it's in one.
    pub fn volatile(&self, ino: Ino) -> Option<Ino> {
        self.nodes.get(&ino).and_then(|n| n.volatile)
    }

    /// Whether a change to `ino` is logged: not in a volatile tree.
    fn logged(&self, ino: Ino) -> bool {
        self.volatile(ino).is_none()
    }

    /// Makes a volatile tree: an empty directory, in no other, held until [`Fs::drop_tree`]. What's made in it is in
    /// memory only: never logged, saved or frozen, and gone when the disk is.
    pub fn volatile_root(&mut self, meta: M, now: u64) -> Ino {
        let ino = self.next;
        self.next += 1;
        let fold = matches!(self.nodes.get(&ROOT).map(|n| &n.data), Some(Data::Dir { fold: true, .. }));
        self.nodes.insert(ino, Node { meta, modified: now, generation: 0, links: 2, data: Data::Dir { entries: BTreeMap::new(), parent: ino, fold }, volatile: Some(ino), xattrs: BTreeMap::new() });
        self.held.insert(ino, 1);
        ino
    }

    /// Lets a volatile tree go: everything in it, but for files held open, which go when they're let go.
    pub fn drop_tree(&mut self, root: Ino) -> Result<()> {
        if self.volatile(root) != Some(root) {
            return Err(Error::NotFound);
        }
        let mut stack = vec![root];
        while let Some(ino) = stack.pop() {
            let Some(node) = self.nodes.get_mut(&ino) else { continue };
            if let Data::Dir { entries, .. } = &mut node.data {
                stack.extend(core::mem::take(entries).into_values().map(|(_, child)| child).filter(|child| *child != ino));
            }
            node.links = 0;
            if ino == root || !self.held.contains_key(&ino) {
                self.nodes.remove(&ino);
            }
        }
        self.held.remove(&root);
        Ok(())
    }

    /// How many bytes the files under `dir` hold (as `df` asks a tmpfs).
    pub fn tree_size(&self, dir: Ino) -> u64 {
        let mut stack = vec![dir];
        let mut total = 0;
        while let Some(ino) = stack.pop() {
            match self.nodes.get(&ino).map(|n| &n.data) {
                Some(Data::Dir { entries, .. }) => stack.extend(entries.values().map(|(_, child)| *child).filter(|c| *c != ino)),
                Some(Data::File(file)) => total += file.size,
                _ => {}
            }
        }
        total
    }

    /// Whether nothing can find `ino` any more.
    fn gone(&self, ino: Ino) -> bool {
        self.nodes.get(&ino).is_none_or(|n| n.links == 0)
    }

    fn node(&self, ino: Ino) -> Result<&Node<M>> {
        self.nodes.get(&ino).ok_or(Error::NotFound)
    }

    fn node_mut(&mut self, ino: Ino) -> Result<&mut Node<M>> {
        self.nodes.get_mut(&ino).ok_or(Error::NotFound)
    }

    /// Appends to the log, if changes are being logged.
    fn record(&mut self, record: impl FnOnce(fn(&M) -> [u32; 4]) -> Record) -> Result<()> {
        if let (Some(save), Some(store)) = (self.save, &self.store) {
            store.borrow_mut().log_append(&record(save).encode())?;
        }
        Ok(())
    }

    /// A chunk from the store, from the cache if it was read lately.
    fn fetch(&self, hash: &Hash) -> Result<Vec<u8>> {
        let mut cache = self.cache.borrow_mut();
        if let Some(at) = cache.iter().position(|(h, _)| h == hash) {
            let entry = cache.remove(at);
            let bytes = entry.1.clone();
            cache.push(entry);
            return Ok(bytes);
        }
        let bytes = self.store.as_ref().ok_or(Error::Io)?.borrow_mut().get(hash)?;
        if cache.len() == CACHED {
            cache.remove(0);
        }
        cache.push((*hash, bytes.clone()));
        Ok(bytes)
    }

    /// Notes that a file changed, for the next save.
    fn touched(&mut self, ino: Ino, size_before: u64, written: u64) {
        if self.store.is_none() || !self.logged(ino) {
            return;
        }
        let low = self.dirty.entry(ino).or_insert(size_before);
        *low = (*low).min(size_before);
        self.dirty_bytes += written;
    }

    pub fn stat(&self, ino: Ino) -> Result<Stat> {
        let node = self.node(ino)?;
        let (kind, size) = match &node.data {
            Data::File(file) => (Kind::File, file.size),
            Data::Dir { entries, .. } => (Kind::Dir, entries.len() as u64),
            Data::Symlink(target) => (Kind::Symlink, target.len() as u64),
        };
        Ok(Stat { ino, kind, size, links: node.links, modified: node.modified, generation: node.generation })
    }

    pub fn meta(&self, ino: Ino) -> Result<&M> {
        Ok(&self.node(ino)?.meta)
    }

    pub fn set_meta(&mut self, ino: Ino, meta: M) -> Result<()> {
        self.node_mut(ino)?.meta = meta.clone();
        if self.logged(ino) {
            self.record(|save| Record::Meta { ino, meta: save(&meta) })?;
        }
        self.tell(Change::Changed { ino });
        Ok(())
    }

    /// The entry `name` in `dir`, without following it if it's a link.
    pub fn lookup(&self, dir: Ino, name: &str) -> Result<Ino> {
        match &self.node(dir)?.data {
            Data::Dir { entries, parent, fold } => match name {
                "" | "." => Ok(dir),
                ".." => Ok(*parent),
                _ => entries.get(&key(name, *fold)).map(|(_, ino)| *ino).ok_or(Error::NotFound),
            },
            _ => Err(Error::NotDir),
        }
    }

    /// The directory a directory is in.
    pub fn parent(&self, dir: Ino) -> Result<Ino> {
        self.lookup(dir, "..")
    }

    /// Follows `path` from `start` (or from the root, if it starts with `/`). Links along the way are followed, and so
    /// is the last one if `follow`.
    pub fn resolve(&self, start: Ino, path: &str, follow: bool) -> Result<Ino> {
        let mut links = 0;
        self.walk(start, path, follow, &mut links)
    }

    /// Like `resolve`, but stops at the last component's directory, giving it and the last name.
    pub fn resolve_parent<'p>(&self, start: Ino, path: &'p str) -> Result<(Ino, &'p str)> {
        let trimmed = path.trim_end_matches('/');
        let (dir, name) = match trimmed.rfind('/') {
            None => (start, trimmed),
            Some(0) => (ROOT, &trimmed[1..]),
            Some(at) => (self.resolve(start, &trimmed[..at], true)?, &trimmed[at + 1..]),
        };
        if name.is_empty() {
            return Err(Error::BadName);
        }
        Ok((dir, name))
    }

    fn walk(&self, start: Ino, path: &str, follow: bool, links: &mut usize) -> Result<Ino> {
        let mut at = if path.starts_with('/') { ROOT } else { start };
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        for (index, part) in parts.iter().enumerate() {
            let last = index + 1 == parts.len();
            let next = self.lookup(at, part)?;
            if let Data::Symlink(target) = &self.node(next)?.data
                && (!last || follow)
            {
                *links += 1;
                if *links > MOST_LINKS {
                    return Err(Error::Loop);
                }
                at = self.walk(at, &target.clone(), true, links)?;
            } else {
                at = next;
            }
        }
        Ok(at)
    }

    fn file(&self, ino: Ino) -> Result<&File> {
        match &self.node(ino)?.data {
            Data::File(file) => Ok(file),
            Data::Dir { .. } => Err(Error::IsDir),
            Data::Symlink(_) => Err(Error::NotFound),
        }
    }

    /// Reads from a file at `offset`, giving how much was read: 0 at its end.
    pub fn read(&self, ino: Ino, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let file = self.file(ino)?;
        let count = buffer.len().min(file.size.saturating_sub(offset) as usize);
        let mut done = 0;
        while done < count {
            let at = offset + done as u64;
            let (number, within) = ((at / CHUNK) as usize, (at % CHUNK) as usize);
            let part = (count - done).min(CHUNK as usize - within);
            let target = &mut buffer[done..done + part];
            let fill = |bytes: &[u8], target: &mut [u8]| {
                let have = bytes.len().saturating_sub(within).min(target.len());
                target[..have].copy_from_slice(&bytes[within..within + have]);
                target[have..].fill(0);
            };
            match file.chunks.get(number) {
                Some(Chunk::Bytes(bytes, _)) => fill(bytes, target),
                Some(Chunk::Stored(hash)) => fill(&self.fetch(hash)?, target),
                _ => target.fill(0),
            }
            done += part;
        }
        Ok(count)
    }

    /// The whole of a file.
    pub fn contents(&self, ino: Ino) -> Result<Vec<u8>> {
        let mut bytes = vec![0; self.file(ino)?.size as usize];
        self.read(ino, 0, &mut bytes)?;
        Ok(bytes)
    }

    /// A chunk of a file in memory, to change: one in the store is read back, a hole becomes zeroes.
    fn chunk_bytes(&self, chunk: &Chunk) -> Result<Vec<u8>> {
        match chunk {
            Chunk::Hole => Ok(Vec::new()),
            Chunk::Stored(hash) => self.fetch(hash),
            Chunk::Bytes(bytes, _) => Ok(bytes.clone()),
        }
    }

    /// Writes to a file at `offset`, growing it as needed. `now` is the time, in nanoseconds since 1970.
    pub fn write(&mut self, ino: Ino, offset: u64, data: &[u8], now: u64) -> Result<usize> {
        let size_before = self.file(ino)?.size;
        let end = offset.checked_add(data.len() as u64).ok_or(Error::Io)?;
        let mut done = 0;
        while done < data.len() {
            let at = offset + done as u64;
            let (number, within) = ((at / CHUNK) as usize, (at % CHUNK) as usize);
            let part = (data.len() - done).min(CHUNK as usize - within);
            let existing = self.file(ino)?.chunks.get(number).cloned().unwrap_or(Chunk::Hole);
            let mut bytes = match existing {
                Chunk::Bytes(bytes, _) => bytes,
                other => self.chunk_bytes(&other)?,
            };
            if bytes.len() < within + part {
                bytes.resize(within + part, 0);
            }
            bytes[within..within + part].copy_from_slice(&data[done..done + part]);
            let Data::File(file) = &mut self.node_mut(ino)?.data else { return Err(Error::IsDir) };
            if file.chunks.len() <= number {
                file.chunks.resize(number + 1, Chunk::Hole);
            }
            file.chunks[number] = Chunk::Bytes(bytes, true);
            done += part;
        }
        let node = self.node_mut(ino)?;
        if let Data::File(file) = &mut node.data {
            file.size = file.size.max(end);
            file.chunks.resize(chunk_count(file.size), Chunk::Hole);
        }
        node.modified = now;
        node.generation += 1;
        self.touched(ino, size_before, data.len() as u64);
        self.tell(Change::Written { ino });
        self.save_if_much()?;
        Ok(data.len())
    }

    pub fn truncate(&mut self, ino: Ino, size: u64, now: u64) -> Result<()> {
        let size_before = self.file(ino)?.size;
        // The last chunk kept, cut to the new size, so growing again later shows zeroes, not what was cut.
        let last = if size < size_before && !size.is_multiple_of(CHUNK) {
            let number = (size / CHUNK) as usize;
            let chunk = self.file(ino)?.chunks.get(number).cloned().unwrap_or(Chunk::Hole);
            let mut bytes = self.chunk_bytes(&chunk)?;
            bytes.truncate((size % CHUNK) as usize);
            Some((number, Chunk::Bytes(bytes, true)))
        } else {
            None
        };
        let node = self.node_mut(ino)?;
        let Data::File(file) = &mut node.data else { return Err(Error::IsDir) };
        file.size = size;
        file.chunks.resize(chunk_count(size), Chunk::Hole);
        if let Some((number, chunk)) = last {
            file.chunks[number] = chunk;
        }
        node.modified = now;
        node.generation += 1;
        self.touched(ino, size_before.min(size), 0);
        self.tell(Change::Written { ino });
        Ok(())
    }

    /// Zeroes `len` bytes of a file from `offset`, keeping its size: what's past its end is left alone. A chunk zeroed
    /// whole keeps no bytes (an empty chunk reads as zeroes), so punching a hole in a big file frees what it held.
    pub fn zero(&mut self, ino: Ino, offset: u64, len: u64, now: u64) -> Result<()> {
        let size = self.file(ino)?.size;
        let end = offset.saturating_add(len).min(size);
        if offset >= end {
            return Ok(());
        }
        for number in (offset / CHUNK) as usize..=((end - 1) / CHUNK) as usize {
            let start = number as u64 * CHUNK;
            let (lo, hi) = ((offset.max(start) - start) as usize, (end.min(start + CHUNK) - start) as usize);
            let existing = self.file(ino)?.chunks.get(number).cloned().unwrap_or(Chunk::Hole);
            let chunk = if lo == 0 && (hi == CHUNK as usize || start + hi as u64 == size) {
                match existing {
                    Chunk::Hole => continue,
                    _ => Chunk::Bytes(Vec::new(), true),
                }
            } else {
                let mut bytes = self.chunk_bytes(&existing)?;
                if bytes.len() <= lo {
                    continue;
                }
                let top = hi.min(bytes.len());
                bytes[lo..top].fill(0);
                Chunk::Bytes(bytes, true)
            };
            let Data::File(file) = &mut self.node_mut(ino)?.data else { return Err(Error::IsDir) };
            file.chunks[number] = chunk;
        }
        let node = self.node_mut(ino)?;
        node.modified = now;
        node.generation += 1;
        self.touched(ino, size, 0);
        self.tell(Change::Written { ino });
        Ok(())
    }

    /// An extended attribute's value, or `None` if the file has no such attribute.
    pub fn get_xattr(&self, ino: Ino, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.node(ino)?.xattrs.get(name).cloned())
    }

    /// Every extended attribute's name, in order.
    pub fn list_xattrs(&self, ino: Ino) -> Result<Vec<String>> {
        Ok(self.node(ino)?.xattrs.keys().cloned().collect())
    }

    /// Sets an extended attribute.
    pub fn set_xattr(&mut self, ino: Ino, name: &str, value: &[u8]) -> Result<()> {
        self.node_mut(ino)?.xattrs.insert(name.to_string(), value.to_vec());
        if self.logged(ino) {
            self.record(|_| Record::Xattr { ino, name: name.to_string(), value: Some(value.to_vec()) })?;
        }
        Ok(())
    }

    /// Removes an extended attribute; whether it was there.
    pub fn remove_xattr(&mut self, ino: Ino, name: &str) -> Result<bool> {
        let had = self.node_mut(ino)?.xattrs.remove(name).is_some();
        if had && self.logged(ino) {
            self.record(|_| Record::Xattr { ino, name: name.to_string(), value: None })?;
        }
        Ok(had)
    }

    /// A link's target.
    pub fn read_link(&self, ino: Ino) -> Result<&str> {
        match &self.node(ino)?.data {
            Data::Symlink(target) => Ok(target),
            _ => Err(Error::NotFound),
        }
    }

    /// Makes `name` in `dir`.
    pub fn create(&mut self, dir: Ino, name: &str, new: New, meta: M, now: u64) -> Result<Ino> {
        let ino = self.next;
        let (kind, target) = match &new {
            New::File => (0, String::new()),
            New::Dir => (1, String::new()),
            New::Symlink(target) => (2, target.clone()),
        };
        self.create_as(dir, name, ino, new, meta.clone(), now)?;
        if self.logged(dir) {
            self.record(|save| Record::Create { dir, ino, kind, meta: save(&meta), now, name: name.to_string(), target })?;
        }
        let kind = self.kind(ino);
        self.tell(Change::Created { dir, name, ino, kind });
        Ok(ino)
    }

    /// Makes `name` in `dir` as `ino`.
    fn create_as(&mut self, dir: Ino, name: &str, ino: Ino, new: New, meta: M, now: u64) -> Result<()> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(Error::BadName);
        }
        if self.nodes.contains_key(&ino) {
            return Err(Error::Io);
        }
        let fold = match &self.node(dir)?.data {
            Data::Dir { entries, fold, .. } => {
                if entries.contains_key(&key(name, *fold)) {
                    return Err(Error::Exists);
                }
                *fold
            }
            _ => return Err(Error::NotDir),
        };
        self.next = self.next.max(ino + 1);
        let (data, links) = match new {
            New::File => (Data::File(File { size: 0, chunks: Vec::new() }), 1),
            New::Dir => (Data::Dir { entries: BTreeMap::new(), parent: dir, fold }, 2),
            New::Symlink(target) => (Data::Symlink(target), 1),
        };
        let is_dir = matches!(data, Data::Dir { .. });
        let volatile = self.volatile(dir);
        self.nodes.insert(ino, Node { meta, modified: now, generation: 0, links, data, volatile, xattrs: BTreeMap::new() });
        let parent = self.node_mut(dir)?;
        if let Data::Dir { entries, .. } = &mut parent.data {
            entries.insert(key(name, fold), (name.to_string(), ino));
        }
        parent.modified = now;
        if is_dir {
            parent.links += 1;
        }
        Ok(())
    }

    /// Adds another name for an existing file (not a directory): a hard link.
    pub fn link(&mut self, dir: Ino, name: &str, ino: Ino, now: u64) -> Result<()> {
        if self.volatile(dir) != self.volatile(ino) {
            return Err(Error::CrossFs);
        }
        self.link_as(dir, name, ino, now)?;
        if self.logged(dir) {
            self.record(|_| Record::Link { dir, ino, now, name: name.to_string() })?;
        }
        self.tell(Change::Linked { dir, name, ino });
        Ok(())
    }

    fn link_as(&mut self, dir: Ino, name: &str, ino: Ino, now: u64) -> Result<()> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(Error::BadName);
        }
        if matches!(self.node(ino)?.data, Data::Dir { .. }) {
            return Err(Error::IsDir);
        }
        let parent = self.node_mut(dir)?;
        let Data::Dir { entries, fold, .. } = &mut parent.data else { return Err(Error::NotDir) };
        let folded = key(name, *fold);
        if entries.contains_key(&folded) {
            return Err(Error::Exists);
        }
        entries.insert(folded, (name.to_string(), ino));
        parent.modified = now;
        self.node_mut(ino)?.links += 1;
        Ok(())
    }

    /// Sets a file's modification time, in nanoseconds since 1970.
    pub fn set_modified(&mut self, ino: Ino, now: u64) -> Result<()> {
        self.node_mut(ino)?.modified = now;
        if self.logged(ino) {
            self.record(|_| Record::Modified { ino, now })?;
        }
        self.tell(Change::Changed { ino });
        Ok(())
    }

    /// Takes `name` out of `dir`. A directory must be empty. The file goes when nothing links to it.
    pub fn remove(&mut self, dir: Ino, name: &str, now: u64) -> Result<()> {
        let ino = self.lookup(dir, name)?;
        let kind = self.kind(ino);
        self.remove_as(dir, name, now)?;
        if self.logged(dir) {
            self.record(|_| Record::Remove { dir, now, name: name.to_string() })?;
        }
        let gone = self.gone(ino);
        self.tell(Change::Removed { dir, name, ino, kind, gone });
        Ok(())
    }

    fn remove_as(&mut self, dir: Ino, name: &str, now: u64) -> Result<()> {
        let ino = self.lookup(dir, name)?;
        if name == "." || name == ".." {
            return Err(Error::BadName);
        }
        let is_dir = match &self.node(ino)?.data {
            Data::Dir { entries, .. } if !entries.is_empty() => return Err(Error::NotEmpty),
            Data::Dir { .. } => true,
            _ => false,
        };
        let parent = self.node_mut(dir)?;
        if let Data::Dir { entries, fold, .. } = &mut parent.data {
            let folded = key(name, *fold);
            entries.remove(&folded);
        }
        parent.modified = now;
        if is_dir {
            parent.links -= 1;
            self.nodes.remove(&ino);
        } else {
            let node = self.node_mut(ino)?;
            node.links -= 1;
            if node.links == 0 {
                // A file that's open stays, nameless and never saved, until it's let go.
                if !self.held.contains_key(&ino) {
                    self.nodes.remove(&ino);
                }
                self.dirty.remove(&ino);
            }
        }
        Ok(())
    }

    /// Keeps a file while a kernel has it open, as Unix does: removing its last name doesn't remove it then.
    pub fn hold(&mut self, ino: Ino) {
        *self.held.entry(ino).or_insert(0) += 1;
    }

    /// Lets go of a file [`Fs::hold`] kept: if it has no names left and nothing else holds it, it goes now.
    pub fn release(&mut self, ino: Ino) {
        let Some(count) = self.held.get_mut(&ino) else { return };
        *count -= 1;
        if *count > 0 {
            return;
        }
        self.held.remove(&ino);
        if self.nodes.get(&ino).is_some_and(|n| n.links == 0) {
            self.nodes.remove(&ino);
            self.dirty.remove(&ino);
        }
    }

    /// Moves `from_dir`/`from` to `to_dir`/`to`, replacing a file (not a directory) already there.
    pub fn rename(&mut self, from_dir: Ino, from: &str, to_dir: Ino, to: &str, now: u64) -> Result<()> {
        let ino = self.lookup(from_dir, from)?;
        let kind = self.kind(ino);
        let replaced = self.lookup(to_dir, to).ok().filter(|r| *r != ino);
        if self.volatile(from_dir) != self.volatile(to_dir) {
            return Err(Error::CrossFs);
        }
        if self.rename_as(from_dir, from, to_dir, to, now)? {
            if self.logged(from_dir) {
                self.record(|_| Record::Rename { from_dir, to_dir, now, from: from.to_string(), to: to.to_string() })?;
            }
            let replaced = replaced.map(|r| (r, self.gone(r)));
            self.tell(Change::Moved { from_dir, from, to_dir, to, ino, kind, replaced });
        }
        Ok(())
    }

    /// Whether anything moved.
    fn rename_as(&mut self, from_dir: Ino, from: &str, to_dir: Ino, to: &str, now: u64) -> Result<bool> {
        let ino = self.lookup(from_dir, from)?;
        if let Ok(existing) = self.lookup(to_dir, to) {
            if existing == ino {
                return Ok(false);
            }
            self.remove_as(to_dir, to, now)?;
        }
        let fold = match &self.node(to_dir)?.data {
            Data::Dir { fold, .. } => *fold,
            _ => return Err(Error::NotDir),
        };
        if let Data::Dir { entries, fold: from_fold, .. } = &mut self.node_mut(from_dir)?.data {
            entries.remove(&key(from, *from_fold));
        }
        if let Data::Dir { entries, .. } = &mut self.node_mut(to_dir)?.data {
            entries.insert(key(to, fold), (to.to_string(), ino));
        }
        if let Data::Dir { parent, .. } = &mut self.node_mut(ino)?.data {
            *parent = to_dir;
        }
        Ok(true)
    }

    /// A directory's entries, as named, in order.
    pub fn entries(&self, dir: Ino) -> Result<Vec<(String, Ino, Kind)>> {
        match &self.node(dir)?.data {
            Data::Dir { entries, .. } => Ok(entries
                .values()
                .map(|(name, ino)| (name.clone(), *ino, self.stat(*ino).map(|s| s.kind).unwrap_or(Kind::File)))
                .collect()),
            _ => Err(Error::NotDir),
        }
    }

    /// Builds: puts `data` at the absolute `path`, making directories on the way with `dir_meta`.
    pub fn put(&mut self, path: &str, new: New, data: &[u8], meta: M, dir_meta: M) -> Result<Ino> {
        let mut at = ROOT;
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let Some((last, dirs)) = parts.split_last() else { return Err(Error::BadName) };
        for part in dirs {
            at = match self.lookup(at, part) {
                Ok(ino) => ino,
                Err(Error::NotFound) => self.create(at, part, New::Dir, dir_meta.clone(), 0)?,
                Err(e) => return Err(e),
            };
        }
        let ino = match self.lookup(at, last) {
            Ok(ino) => ino,
            Err(_) => self.create(at, last, new, meta, 0)?,
        };
        if !data.is_empty() {
            self.write(ino, 0, data, 0)?;
        }
        Ok(ino)
    }

    /// Saves once enough has been written.
    fn save_if_much(&mut self) -> Result<()> {
        if self.dirty_bytes >= DIRTY_MOST { self.save_files() } else { Ok(()) }
    }

    /// Puts every changed chunk in the store, and logs each changed file's size and chunks.
    fn save_files(&mut self) -> Result<()> {
        let Some(store) = &self.store else { return Ok(()) };
        for (ino, cut) in core::mem::take(&mut self.dirty) {
            // A file with no name left (open, but removed) is never saved.
            let Some(Node { data: Data::File(file), modified, links: 1.., .. }) = self.nodes.get_mut(&ino) else { continue };
            let mut changed = Vec::new();
            for (number, chunk) in file.chunks.iter_mut().enumerate() {
                if let Chunk::Bytes(bytes, true) = chunk {
                    let hash = store.borrow_mut().put(bytes)?;
                    changed.push((number as u32, Some(hash)));
                    *chunk = Chunk::Stored(hash);
                }
            }
            let record = Record::File { ino, cut, size: file.size, modified: *modified, chunks: changed };
            if self.save.is_some() {
                store.borrow_mut().log_append(&record.encode())?;
            }
        }
        self.dirty_bytes = 0;
        Ok(())
    }

    /// Makes every change so far survive a crash: changed chunks go to the store, the log is synced.
    pub fn sync(&mut self) -> Result<()> {
        self.save_files()?;
        if let (Some(store), Some(_)) = (&self.store, self.save) {
            store.borrow_mut().log_sync()?;
        }
        Ok(())
    }

    /// Does what a record in the log says, as it was done the first time.
    fn apply(&mut self, record: Record, load: fn([u32; 4]) -> M) -> Result<()> {
        match record {
            Record::Create { dir, ino, kind, meta, now, name, target } => {
                let new = match kind {
                    0 => New::File,
                    1 => New::Dir,
                    2 => New::Symlink(target),
                    _ => return Err(Error::Io),
                };
                self.create_as(dir, &name, ino, new, load(meta), now)
            }
            Record::Link { dir, ino, now, name } => self.link_as(dir, &name, ino, now),
            Record::Remove { dir, now, name } => self.remove_as(dir, &name, now),
            Record::Rename { from_dir, to_dir, now, from, to } => self.rename_as(from_dir, &from, to_dir, &to, now).map(|_| ()),
            Record::Meta { ino, meta } => {
                self.node_mut(ino)?.meta = load(meta);
                Ok(())
            }
            Record::Modified { ino, now } => {
                self.node_mut(ino)?.modified = now;
                Ok(())
            }
            Record::Xattr { ino, name, value } => {
                let node = self.node_mut(ino)?;
                match value {
                    Some(value) => {
                        node.xattrs.insert(name, value);
                    }
                    None => {
                        node.xattrs.remove(&name);
                    }
                }
                Ok(())
            }
            Record::File { ino, cut, size, modified, chunks } => {
                let node = self.node_mut(ino)?;
                let Data::File(file) = &mut node.data else { return Err(Error::Io) };
                file.chunks.truncate(chunk_count(cut));
                file.size = size;
                file.chunks.resize(chunk_count(size), Chunk::Hole);
                for (number, hash) in chunks {
                    let slot = file.chunks.get_mut(number as usize).ok_or(Error::Io)?;
                    *slot = hash.map_or(Chunk::Hole, Chunk::Stored);
                }
                node.modified = modified;
                Ok(())
            }
        }
    }
}

impl<M: Persist> Fs<M> {
    /// A machine's disk: the image's `index` (or, with none, an empty one, with `root` and `fold_case`), the machine's
    /// `store`, and every change in its log since, replayed. A log a crash left half-written is cut back to its last
    /// whole record. From here on, changes are logged.
    pub fn open(index: Option<&[u8]>, store: Box<dyn Store>, root: M, fold_case: bool) -> Result<Fs<M>> {
        let mut fs = match index {
            Some(bytes) => Self::from_index(bytes)?,
            None => Fs::new(root, fold_case),
        };
        fs.store = Some(RefCell::new(store));
        let log = fs.store.as_ref().ok_or(Error::Io)?.borrow_mut().log_read()?;
        let (records, whole) = log::read(&log);
        if whole < log.len() {
            fs.store.as_ref().ok_or(Error::Io)?.borrow_mut().log_truncate(whole as u64)?;
        }
        for record in records {
            fs.apply(record, M::load)?;
        }
        fs.save = Some(M::save);
        Ok(fs)
    }

    /// A disk being built, as `images` builds one from an image's layers: chunks go to `store`, nothing is logged, and
    /// `freeze` makes it an index.
    pub fn building(root: M, fold_case: bool, store: Box<dyn Store>) -> Fs<M> {
        let mut fs = Fs::new(root, fold_case);
        fs.store = Some(RefCell::new(store));
        fs
    }

    /// The tree as it was written in an index.
    fn from_index(bytes: &[u8]) -> Result<Fs<M>> {
        let index = index::Index::open(bytes)?;
        let root = index.inode(ROOT)?;
        if root.kind != Kind::Dir {
            return Err(Error::Io);
        }
        let mut fs = Fs::new(M::load(root.meta), root.folds_case);
        fs.next = index.inode_count() + 1;
        let mut stack = vec![(ROOT, ROOT)];
        while let Some((ino, parent)) = stack.pop() {
            if fs.nodes.contains_key(&ino) && ino != ROOT {
                continue;
            }
            let inode = index.inode(ino)?;
            let data = match inode.kind {
                Kind::Dir => {
                    let mut entries = BTreeMap::new();
                    for (name, child) in index.entries(&inode)? {
                        if child == 0 || child > index.inode_count() {
                            return Err(Error::Io);
                        }
                        entries.insert(key(&name, inode.folds_case), (name, child));
                        if !fs.nodes.contains_key(&child) {
                            stack.push((child, ino));
                        }
                    }
                    Data::Dir { entries, parent, fold: inode.folds_case }
                }
                Kind::Symlink => Data::Symlink(index.link_target(&inode)?),
                Kind::File => {
                    let chunks = match index.inline_data(&inode)? {
                        Some(data) => vec![Chunk::Bytes(data.to_vec(), false)],
                        None => index.chunks(&inode)?.into_iter().map(|hash| hash.map_or(Chunk::Hole, Chunk::Stored)).collect(),
                    };
                    let mut file = File { size: inode.size, chunks };
                    file.chunks.resize(chunk_count(file.size), Chunk::Hole);
                    Data::File(file)
                }
            };
            fs.nodes.insert(ino, Node { meta: M::load(inode.meta), modified: inode.modified, generation: 0, links: inode.links, data, volatile: None, xattrs: BTreeMap::new() });
        }
        Ok(fs)
    }

    /// The whole tree as an index, as it is now, with every chunk in the store. Only what's reachable from the root
    /// goes in, numbered in the order it's reached.
    pub fn freeze(&mut self) -> Result<Vec<u8>> {
        let store = self.store.as_ref().ok_or(Error::Io)?;
        let mut numbers: BTreeMap<Ino, u32> = BTreeMap::new();
        let mut order = vec![ROOT];
        numbers.insert(ROOT, 1);
        let mut at = 0;
        while let Some(&ino) = order.get(at) {
            at += 1;
            if let Data::Dir { entries, .. } = &self.node(ino)?.data {
                for (_, child) in entries.values() {
                    if !numbers.contains_key(child) {
                        order.push(*child);
                        numbers.insert(*child, order.len() as u32);
                    }
                }
            }
        }
        let save = M::save;
        let mut out = Vec::with_capacity(order.len());
        for ino in &order {
            let node = self.node(*ino)?;
            let stat = self.stat(*ino)?;
            let (content, folds_case) = match &node.data {
                Data::Dir { entries, fold, .. } => {
                    (index::Content::Dir(entries.values().map(|(name, child)| (name.clone(), numbers[child])).collect()), *fold)
                }
                Data::Symlink(target) => (index::Content::Symlink(target.clone()), false),
                Data::File(file) if file.size as usize <= INLINE_MAX => (index::Content::Inline(self.contents(*ino)?), false),
                Data::File(file) => {
                    let mut hashes = Vec::with_capacity(file.chunks.len());
                    for chunk in &file.chunks {
                        hashes.push(match chunk {
                            Chunk::Hole => None,
                            Chunk::Stored(hash) => Some(*hash),
                            Chunk::Bytes(bytes, _) => Some(store.borrow_mut().put(bytes)?),
                        });
                    }
                    (index::Content::Chunks(hashes), false)
                }
            };
            out.push(index::Out { kind: stat.kind, meta: save(&node.meta), links: node.links, size: stat.size, modified: node.modified, folds_case, content });
        }
        Ok(index::build(&out))
    }
}

fn key(name: &str, fold: bool) -> String {
    if fold { name.to_lowercase() } else { name.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xattrs_are_kept_and_replayed() {
        let store = Shared::default();
        let mut fs = open(None, &store);
        let f = fs.put("/f", New::File, b"x", 0, 0).unwrap();
        fs.set_xattr(f, "user.comment", b"hello").unwrap();
        fs.set_xattr(f, "security.capability", b"\x01\x00").unwrap();
        fs.set_xattr(f, "user.comment", b"world").unwrap();
        assert_eq!(fs.get_xattr(f, "user.comment").unwrap().as_deref(), Some(&b"world"[..]));
        assert_eq!(fs.list_xattrs(f).unwrap(), vec!["security.capability".to_string(), "user.comment".to_string()]);
        assert!(fs.remove_xattr(f, "security.capability").unwrap());
        assert!(!fs.remove_xattr(f, "security.capability").unwrap());
        assert_eq!(fs.get_xattr(f, "security.capability").unwrap(), None);
        // On a volatile tree: kept in memory, never logged.
        let tmp = fs.volatile_root(0, 1);
        let t = fs.create(tmp, "t", New::File, 0, 1).unwrap();
        fs.set_xattr(t, "user.x", b"1").unwrap();
        assert_eq!(fs.get_xattr(t, "user.x").unwrap().as_deref(), Some(&b"1"[..]));
        fs.sync().unwrap();
        // Reopened from the log: the attribute's latest value is there, the removed one isn't.
        let again = open(None, &store);
        let g = again.resolve(ROOT, "/f", true).unwrap();
        assert_eq!(again.get_xattr(g, "user.comment").unwrap().as_deref(), Some(&b"world"[..]));
        assert_eq!(again.list_xattrs(g).unwrap(), vec!["user.comment".to_string()]);
    }

    #[test]
    fn a_volatile_tree_is_never_logged() {
        let store = Shared::default();
        let mut fs = open(None, &store);
        fs.put("/kept", New::File, b"k", 0, 0).unwrap();
        fs.sync().unwrap();
        let tmp = fs.volatile_root(0, 1);
        let dir = fs.create(tmp, "d", New::Dir, 0, 1).unwrap();
        let file = fs.create(dir, "f", New::File, 0, 1).unwrap();
        fs.write(file, 0, &pattern(3 * CHUNK as usize, 3), 2).unwrap();
        fs.set_meta(file, 9).unwrap();
        assert_eq!(fs.volatile(file), Some(tmp));
        // Nothing crosses between it and the rest.
        assert_eq!(fs.link(ROOT, "f", file, 3), Err(Error::CrossFs));
        assert_eq!(fs.rename(dir, "f", ROOT, "f", 3), Err(Error::CrossFs));
        fs.sync().unwrap();
        // Reopened from its log, only what's outside it is there.
        let again = open(None, &store);
        assert_eq!(read_all(&again, "/kept"), b"k");
        assert_eq!(again.nodes.len(), fs.nodes.len() - 3);
        // A file held open outlives its tree.
        fs.hold(file);
        fs.drop_tree(tmp).unwrap();
        assert!(fs.stat(dir).is_err() && fs.stat(tmp).is_err());
        assert_eq!(fs.stat(file).unwrap().links, 0);
        fs.release(file);
        assert!(fs.stat(file).is_err());
        assert_eq!(fs.drop_tree(ROOT), Err(Error::NotFound));
    }

    #[test]
    fn an_observer_is_told_of_each_change() {
        extern crate std;
        use std::format;
        use std::sync::{Arc, Mutex};
        let mut fs = Fs::new(0u32, false);
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        fs.observe(Some(Box::new(move |fs: &Fs<u32>, change: Change<'_>| {
            // The tree as it is after the change.
            let after = match change {
                Change::Created { dir, name, .. } | Change::Linked { dir, name, .. } => fs.lookup(dir, name).is_ok(),
                Change::Removed { dir, name, .. } => fs.lookup(dir, name).is_err(),
                _ => true,
            };
            log.lock().unwrap().push(format!("{change:?} {after}"));
        })));
        let d = fs.create(ROOT, "d", New::Dir, 0, 1).unwrap();
        let f = fs.create(d, "f", New::File, 0, 1).unwrap();
        fs.write(f, 0, b"x", 2).unwrap();
        fs.truncate(f, 0, 3).unwrap();
        fs.set_meta(f, 7).unwrap();
        fs.link(ROOT, "g", f, 4).unwrap();
        let other = fs.create(ROOT, "h", New::File, 0, 5).unwrap();
        fs.rename(d, "f", ROOT, "h", 6).unwrap();
        fs.rename(ROOT, "h", ROOT, "g", 6).unwrap();
        fs.remove(ROOT, "g", 7).unwrap();
        fs.remove(ROOT, "d", 8).unwrap();
        let _ = fs.remove(ROOT, "nothing", 9);
        let seen = seen.lock().unwrap();
        let want = [
            format!("Created {{ dir: 1, name: \"d\", ino: {d}, kind: Dir }} true"),
            format!("Created {{ dir: {d}, name: \"f\", ino: {f}, kind: File }} true"),
            format!("Written {{ ino: {f} }} true"),
            format!("Written {{ ino: {f} }} true"),
            format!("Changed {{ ino: {f} }} true"),
            format!("Linked {{ dir: 1, name: \"g\", ino: {f} }} true"),
            format!("Created {{ dir: 1, name: \"h\", ino: {other}, kind: File }} true"),
            format!("Moved {{ from_dir: {d}, from: \"f\", to_dir: 1, to: \"h\", ino: {f}, kind: File, replaced: Some(({other}, true)) }} true"),
            // Both names one file: nothing moves, and no one's told.
            format!("Removed {{ dir: 1, name: \"g\", ino: {f}, kind: File, gone: false }} true"),
            format!("Removed {{ dir: 1, name: \"d\", ino: {d}, kind: Dir, gone: true }} true"),
        ];
        assert_eq!(seen.as_slice(), want.as_slice());
    }

    #[test]
    fn build_and_resolve() {
        let mut fs = Fs::new(0u32, false);
        let hello = fs.put("/usr/bin/hello", New::File, b"hi", 1, 2).unwrap();
        fs.put("/bin", New::Symlink("usr/bin".into()), b"", 3, 2).unwrap();
        assert_eq!(fs.resolve(ROOT, "/bin/hello", true), Ok(hello));
        assert_eq!(fs.resolve(ROOT, "/usr/bin/../bin/./hello", true), Ok(hello));
        let mut buffer = [0; 4];
        assert_eq!(fs.read(hello, 0, &mut buffer), Ok(2));
        assert_eq!(fs.resolve_parent(ROOT, "/usr/bin/new"), Ok((fs.resolve(ROOT, "/usr/bin", true).unwrap(), "new")));
        assert_eq!(fs.resolve_parent(ROOT, "/top"), Ok((ROOT, "top")));
    }

    #[test]
    fn an_open_file_outlives_its_name() {
        let mut fs = Fs::new((), false);
        let file = fs.put("/tmp/scratch", New::File, b"", (), ()).unwrap();
        let tmp = fs.resolve(ROOT, "/tmp", true).unwrap();
        fs.hold(file);
        fs.hold(file);
        fs.remove(tmp, "scratch", 0).unwrap();
        assert_eq!(fs.resolve(ROOT, "/tmp/scratch", true), Err(Error::NotFound));
        // Still written and read through what holds it.
        fs.write(file, 0, b"still here", 1).unwrap();
        assert_eq!(fs.contents(file).unwrap(), b"still here");
        assert_eq!(fs.stat(file).unwrap().links, 0);
        fs.release(file);
        assert!(fs.stat(file).is_ok());
        fs.release(file);
        assert!(fs.stat(file).is_err());
        // A file let go with its name still there stays.
        let kept = fs.put("/tmp/kept", New::File, b"x", (), ()).unwrap();
        fs.hold(kept);
        fs.release(kept);
        assert!(fs.stat(kept).is_ok());
    }

    #[test]
    fn folding_case() {
        let mut fs = Fs::new((), true);
        let file = fs.put("/Windows/System32/KERNEL32.dll", New::File, b"", (), ()).unwrap();
        assert_eq!(fs.resolve(ROOT, "/windows/system32/kernel32.DLL", true), Ok(file));
        assert_eq!(fs.entries(fs.resolve(ROOT, "/WINDOWS/system32", true).unwrap()).unwrap()[0].0, "KERNEL32.dll");
    }

    #[test]
    fn writes_and_removes() {
        let mut fs = Fs::new((), false);
        let file = fs.put("/tmp/a", New::File, b"", (), ()).unwrap();
        fs.write(file, 4, b"xy", 9).unwrap();
        assert_eq!(fs.contents(file).unwrap(), b"\0\0\0\0xy");
        assert_eq!(fs.stat(file).unwrap().generation, 1);
        let tmp = fs.resolve(ROOT, "/tmp", true).unwrap();
        assert_eq!(fs.remove(ROOT, "tmp", 0), Err(Error::NotEmpty));
        fs.rename(tmp, "a", ROOT, "b", 0).unwrap();
        fs.remove(tmp, "a", 0).unwrap_err();
        fs.remove(ROOT, "tmp", 0).unwrap();
        assert!(fs.resolve(ROOT, "/b", true).is_ok());
    }

    extern crate std;
    use std::sync::{Arc, Mutex};

    /// A memory store the test keeps a hold of, to open the disk again from.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<MemoryStore>>);

    impl Store for Shared {
        fn get(&mut self, hash: &Hash) -> Result<Vec<u8>> {
            self.0.lock().unwrap().get(hash)
        }
        fn put(&mut self, data: &[u8]) -> Result<Hash> {
            self.0.lock().unwrap().put(data)
        }
        fn log_append(&mut self, data: &[u8]) -> Result<()> {
            self.0.lock().unwrap().log_append(data)
        }
        fn log_read(&mut self) -> Result<Vec<u8>> {
            self.0.lock().unwrap().log_read()
        }
        fn log_truncate(&mut self, len: u64) -> Result<()> {
            self.0.lock().unwrap().log_truncate(len)
        }
        fn log_sync(&mut self) -> Result<()> {
            self.0.lock().unwrap().log_sync()
        }
    }

    impl Shared {
        fn crashed(&self) -> Shared {
            Shared(Arc::new(Mutex::new(self.0.lock().unwrap().crashed())))
        }

        /// The same chunks, and an empty log: another machine on them.
        fn chunks_only(&self) -> Shared {
            let copy = self.crashed();
            copy.0.lock().unwrap().log_truncate(0).unwrap();
            copy
        }
    }

    /// Bytes that don't repeat, so no two chunks are alike.
    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        let mut x = u32::from(seed) | 1;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect()
    }

    fn open(index: Option<&[u8]>, store: &Shared) -> Fs<u32> {
        Fs::open(index, Box::new(store.clone()), 0o755, false).unwrap()
    }

    fn read_all(fs: &Fs<u32>, path: &str) -> Vec<u8> {
        fs.contents(fs.resolve(ROOT, path, true).unwrap()).unwrap()
    }

    #[test]
    fn a_disk_keeps_its_files() {
        let store = Shared::default();
        let big = pattern(200_000, 7);
        {
            let mut fs = open(None, &store);
            let etc = fs.create(ROOT, "etc", New::Dir, 0o755, 1).unwrap();
            let file = fs.create(etc, "big", New::File, 0o644, 2).unwrap();
            fs.write(file, 0, &big, 3).unwrap();
            fs.put("/etc/small", New::File, b"hello", 0o600, 0o755).unwrap();
            fs.create(ROOT, "link", New::Symlink("etc/big".into()), 0o777, 4).unwrap();
            fs.link(ROOT, "hard", file, 5).unwrap();
            let sparse = fs.create(ROOT, "sparse", New::File, 0o644, 6).unwrap();
            fs.write(sparse, 300_000, b"end", 7).unwrap();
            fs.set_meta(file, 0o640).unwrap();
            fs.rename(ROOT, "hard", etc, "hard", 8).unwrap();
            fs.put("/gone", New::File, b"x", 0, 0).unwrap();
            fs.remove(ROOT, "gone", 9).unwrap();
            fs.sync().unwrap();
        }
        let fs = open(None, &store);
        assert_eq!(read_all(&fs, "/link"), big);
        assert_eq!(read_all(&fs, "/etc/hard"), big);
        assert_eq!(read_all(&fs, "/etc/small"), b"hello");
        let file = fs.resolve(ROOT, "/etc/big", true).unwrap();
        assert_eq!(*fs.meta(file).unwrap(), 0o640);
        assert_eq!(fs.stat(file).unwrap().links, 2);
        assert_eq!(fs.stat(file).unwrap().modified, 3);
        assert_eq!(fs.resolve(ROOT, "/gone", true), Err(Error::NotFound));
        let sparse = read_all(&fs, "/sparse");
        assert_eq!(sparse.len(), 300_003);
        assert!(sparse[..300_000].iter().all(|b| *b == 0) && &sparse[300_000..] == b"end");
        // Four chunks for the big file, one each for the small file and the sparse file's end; holes take none.
        assert_eq!(store.0.lock().unwrap().chunks(), 6);
    }

    #[test]
    fn what_wasnt_synced_is_lost_cleanly() {
        let store = Shared::default();
        let mut fs = open(None, &store);
        fs.put("/kept", New::File, b"kept", 0, 0).unwrap();
        fs.sync().unwrap();
        fs.put("/second", New::File, b"second", 0, 0).unwrap();
        let kept = fs.resolve(ROOT, "/kept", true).unwrap();
        fs.write(kept, 0, b"KEPT and more", 1).unwrap();
        fs.sync().unwrap();
        fs.write(kept, 0, b"neither", 2).unwrap();
        fs.put("/also-lost", New::File, b"", 0, 0).unwrap();
        // A crash: only what was synced survives, and a record cut in half on the way.
        let crashed = store.crashed();
        crashed.0.lock().unwrap().log_append(&[42, 0, 0, 0, 1, 2]).unwrap();
        let fs = open(None, &crashed);
        assert_eq!(read_all(&fs, "/kept"), b"KEPT and more");
        assert_eq!(read_all(&fs, "/second"), b"second");
        assert_eq!(fs.resolve(ROOT, "/also-lost", true), Err(Error::NotFound));
        // The torn record was cut off.
        let store = crashed.0.lock().unwrap();
        assert_eq!(store.log().len(), store.synced);
    }

    #[test]
    fn cut_then_grown_shows_zeroes() {
        let store = Shared::default();
        let mut fs = open(None, &store);
        let file = fs.put("/f", New::File, &pattern(150_000, 1), 0, 0).unwrap();
        fs.sync().unwrap();
        fs.truncate(file, 10, 1).unwrap();
        fs.truncate(file, 140_000, 2).unwrap();
        fs.write(file, 139_999, b"z", 3).unwrap();
        fs.sync().unwrap();
        let fs = open(None, &store);
        let got = read_all(&fs, "/f");
        assert_eq!(got.len(), 140_000);
        assert_eq!(&got[..10], &pattern(150_000, 1)[..10]);
        assert!(got[10..139_999].iter().all(|b| *b == 0));
        assert_eq!(got[139_999], b'z');
    }

    #[test]
    fn zeroed_ranges_keep_the_size_and_survive_reopening() {
        let store = Shared::default();
        let mut fs = open(None, &store);
        let data = pattern(3 * CHUNK as usize + 100, 7);
        let file = fs.put("/f", New::File, &data, 0, 0).unwrap();
        fs.sync().unwrap();
        // Part of the first chunk, all of the second, part of the third; then the tail, and past the end.
        fs.zero(file, 10, 2 * CHUNK + 5 - 10, 1).unwrap();
        fs.zero(file, 3 * CHUNK + 50, 1 << 20, 2).unwrap();
        fs.zero(file, 1 << 30, 10, 3).unwrap();
        fs.sync().unwrap();
        let fs = open(None, &store);
        let got = read_all(&fs, "/f");
        assert_eq!(got.len(), data.len());
        assert_eq!(&got[..10], &data[..10]);
        assert!(got[10..2 * CHUNK as usize + 5].iter().all(|b| *b == 0));
        assert_eq!(&got[2 * CHUNK as usize + 5..3 * CHUNK as usize + 50], &data[2 * CHUNK as usize + 5..3 * CHUNK as usize + 50]);
        assert!(got[3 * CHUNK as usize + 50..].iter().all(|b| *b == 0));
        assert_eq!(fs.stat(fs.resolve(ROOT, "/f", true).unwrap()).unwrap().modified, 2);
    }

    #[test]
    fn frozen_into_an_index_and_opened_again() {
        let chunks = Shared::default();
        let mut image: Fs<u32> = Fs::building(0o755, false, Box::new(chunks.clone()));
        image.put("/usr/bin/tool", New::File, &pattern(100_000, 3), 0o755, 0o755).unwrap();
        image.put("/usr/share/note", New::File, b"note", 0o644, 0o755).unwrap();
        image.put("/bin", New::Symlink("usr/bin".into()), b"", 0o777, 0o755).unwrap();
        let tool = image.resolve(ROOT, "/usr/bin/tool", true).unwrap();
        let share = image.resolve(ROOT, "/usr/share", true).unwrap();
        image.link(share, "tool-again", tool, 0).unwrap();
        let index = image.freeze().unwrap();
        // Building logs nothing.
        assert!(chunks.0.lock().unwrap().log().is_empty());

        // A machine on that image: its own changes in its log, the image's chunks shared.
        let machine = Shared(chunks.0.clone());
        {
            let mut fs = open(Some(&index), &machine);
            assert_eq!(read_all(&fs, "/bin/tool"), pattern(100_000, 3));
            assert_eq!(fs.stat(fs.resolve(ROOT, "/usr/share/tool-again", true).unwrap()).unwrap().links, 2);
            fs.put("/usr/share/new", New::File, b"mine", 0o600, 0o755).unwrap();
            let tool = fs.resolve(ROOT, "/usr/bin/tool", true).unwrap();
            fs.write(tool, 5, b"PATCH", 9).unwrap();
            fs.remove(fs.resolve(ROOT, "/usr/share", true).unwrap(), "note", 9).unwrap();
            fs.sync().unwrap();
        }
        let mut fs = open(Some(&index), &machine);
        assert_eq!(read_all(&fs, "/usr/share/new"), b"mine");
        assert_eq!(&read_all(&fs, "/usr/bin/tool")[..12], &[&pattern(12, 3)[..5], b"PATCH", &pattern(12, 3)[10..]].concat()[..]);
        assert_eq!(fs.resolve(ROOT, "/usr/share/note", true), Err(Error::NotFound));
        // And frozen again, with the machine's changes in.
        let again = fs.freeze().unwrap();
        let fs = open(Some(&again), &chunks.chunks_only());
        assert_eq!(read_all(&fs, "/usr/share/new"), b"mine");
    }

    #[test]
    fn folding_survives_an_index() {
        let store = Shared::default();
        let mut fs: Fs<u32> = Fs::building(0, true, Box::new(store.clone()));
        fs.put("/Windows/System32/KERNEL32.dll", New::File, b"MZ", 0, 0).unwrap();
        let index = fs.freeze().unwrap();
        let fs: Fs<u32> = Fs::open(Some(&index), Box::new(store.clone()), 0, true).unwrap();
        let file = fs.resolve(ROOT, "/windows/system32/kernel32.DLL", true).unwrap();
        assert_eq!(fs.contents(file).unwrap(), b"MZ");
        assert_eq!(fs.entries(fs.resolve(ROOT, "/WINDOWS/system32", true).unwrap()).unwrap()[0].0, "KERNEL32.dll");
    }

    #[test]
    fn a_bad_index_is_refused() {
        let store = Shared::default();
        let mut fs: Fs<u32> = Fs::building(0, false, Box::new(store.clone()));
        fs.put("/a/b", New::File, b"c", 0, 0).unwrap();
        let index = fs.freeze().unwrap();
        for cut in [0, 8, 63, index.len() - 1] {
            assert_eq!(Fs::<u32>::open(Some(&index[..cut]), Box::new(Shared::default()), 0, false).err(), Some(Error::Io));
        }
        let mut wrong = index.clone();
        // An entry's inode number past the end.
        let at = 64 + 3 * 64 + 8;
        wrong[at..at + 4].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(Fs::<u32>::open(Some(&wrong), Box::new(Shared::default()), 0, false).err(), Some(Error::Io));
    }
}
