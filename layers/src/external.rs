//! External trees: a folder somewhere else (a host's folder, a volume), attached to a disk and seen live. Its files
//! aren't kept in the disk's tree, log or index: every call on one asks the tree's [`External`] as it's made, by the
//! file's path in the tree, so what changes there is seen at once, and what a kernel does here is done there.
//!
//! A file in one gets a number the first time it's found, from a range of its own, and keeps it while its path does.
//! Its kernel's `M` is made from what the folder says ([`Mapping::meta`]): kept with each file by a tree that can keep
//! it (a volume, in an extended attribute), else made from the host's mode and owner, with what the kernel sets kept
//! in memory. Nothing links or moves between an external tree and the rest (`CrossFs`), as for a volatile tree.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{Ino, Kind, New, Result};

/// The first number an external tree's file gets: far above any a disk's own tree reaches.
pub(crate) const FIRST: Ino = 1 << 48;

/// What an external tree says of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtStat {
    pub kind: Kind,
    pub size: u64,
    /// Last change to its contents, in nanoseconds since 1970.
    pub modified: u64,
    /// The host's permission bits, owner and group.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub links: u32,
    /// The kernel's `M`, as [`crate::Persist`] saved it, if the tree keeps it with the file.
    pub meta: Option<[u32; 4]>,
}

/// A folder attached as a tree, reached by paths within it: `""` for its top, `a/b` below it.
pub trait External: Send {
    fn stat(&self, path: &str) -> Result<ExtStat>;
    /// What's in a folder, each with its kind.
    fn list(&self, path: &str) -> Result<Vec<(String, Kind)>>;
    fn read(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize>;
    fn write(&self, path: &str, offset: u64, data: &[u8]) -> Result<usize>;
    fn truncate(&self, path: &str, size: u64) -> Result<()>;
    /// Makes a file, a folder or a link, with these permission bits.
    fn make(&self, path: &str, new: &New, mode: u32) -> Result<()>;
    /// Another name, `path`, for the file at `existing`.
    fn link(&self, existing: &str, path: &str) -> Result<()>;
    fn remove(&self, path: &str, dir: bool) -> Result<()>;
    /// Moves `from` to `to`, replacing what's there.
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn read_link(&self, path: &str) -> Result<String>;
    fn set_mode(&self, path: &str, mode: u32) -> Result<()>;
    fn set_modified(&self, path: &str, modified: u64) -> Result<()>;
    /// Keeps the kernel's `M` with the file, if this tree keeps it (a volume does). Whether it did.
    fn keep_meta(&self, path: &str, words: [u32; 4]) -> Result<bool>;
    /// Whether names that differ only in case are one name, as on a Windows host.
    fn fold_case(&self) -> bool {
        false
    }
}

/// How a tree's files' `M` is made, and given back.
pub struct Mapping<M> {
    /// The kernel's `M` from what the tree says of a file (its saved words, if it keeps them).
    pub meta: fn(&ExtStat) -> M,
    /// The permission bits to give the host for an `M`.
    pub mode: fn(&M) -> u32,
    /// `M` as words, to keep with a file, and to tell when it's changed.
    pub save: fn(&M) -> [u32; 4],
}

pub(crate) struct Tree<M> {
    pub backend: Box<dyn External>,
    pub map: Mapping<M>,
    pub read_only: bool,
}

/// Every external tree a disk has, and what's been found in them.
pub(crate) struct Ext<M> {
    pub trees: BTreeMap<Ino, Tree<M>>,
    /// Each file found, by number: its tree's root and its path in the tree.
    pub files: BTreeMap<Ino, (Ino, String)>,
    pub by_path: BTreeMap<(Ino, String), Ino>,
    pub next: Ino,
    /// What the kernel set on a file of a tree that doesn't keep `M`: kept here while the disk is.
    pub set: BTreeMap<Ino, M>,
}

impl<M> Default for Ext<M> {
    fn default() -> Ext<M> {
        Ext {
            trees: BTreeMap::new(),
            files: BTreeMap::new(),
            by_path: BTreeMap::new(),
            next: FIRST,
            set: BTreeMap::new(),
        }
    }
}

impl<M> Ext<M> {
    /// The number for a path in a tree: the one it has, or a new one.
    pub fn ino(&mut self, root: Ino, path: &str) -> Ino {
        if path.is_empty() {
            return root;
        }
        if let Some(ino) = self.by_path.get(&(root, path.to_string())) {
            return *ino;
        }
        let ino = self.next;
        self.next += 1;
        self.by_path.insert((root, path.to_string()), ino);
        self.files.insert(ino, (root, path.to_string()));
        ino
    }

    /// Forgets a path, and everything under it.
    pub fn forget(&mut self, root: Ino, path: &str) {
        let under = alloc::format!("{path}/");
        let gone: Vec<(Ino, String)> = self.by_path.keys().filter(|(r, p)| *r == root && (p == path || p.starts_with(&under))).cloned().collect();
        for key in gone {
            if let Some(ino) = self.by_path.remove(&key) {
                self.files.remove(&ino);
                self.set.remove(&ino);
            }
        }
    }

    /// Moves a path, and everything under it, to another, keeping their numbers.
    pub fn moved(&mut self, root: Ino, from: &str, to: &str) {
        let under = alloc::format!("{from}/");
        let moving: Vec<(Ino, String)> = self.by_path.keys().filter(|(r, p)| *r == root && (p == from || p.starts_with(&under))).cloned().collect();
        for key in moving {
            if let Some(ino) = self.by_path.remove(&key) {
                let path = alloc::format!("{to}{}", &key.1[from.len()..]);
                self.by_path.insert((root, path.clone()), ino);
                self.files.insert(ino, (root, path));
            }
        }
    }
}

/// A path in a tree with a name added.
pub(crate) fn join(path: &str, name: &str) -> String {
    if path.is_empty() { name.to_string() } else { alloc::format!("{path}/{name}") }
}

/// The folder a path in a tree is in.
pub(crate) fn parent_of(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}
