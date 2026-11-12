//! inotify: `inotify_init1`, `inotify_add_watch` and `inotify_rm_watch`, and the events editors, build tools and
//! systemd wait for. An instance is a file whose `read` gives `struct inotify_event`s, ready in `poll` and `epoll`
//! while it holds one.
//!
//! What changes the disk is told by the disk itself: `layers` tells its observer (`observer`, set at boot) of each
//! create, link, removal, move, write and change of metadata, whichever system call made it. What only touches a file
//! (opened, read, closed) is told from here, as it happens (`Kernel::file_event`). Each goes to the watches on the file
//! itself and to those on a directory it's in, with its name there. A file's own watches find it by inode; for a
//! change that names no directory (a write, a chmod), the watched directories are looked through for it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::wait::Watchers;
use layers::{Change, Ino, Kind};

use super::{Node, Open};
use crate::abi::*;
use crate::process::Task;
use crate::{Disk, Kernel};

pub(crate) const IN_ACCESS: u32 = 0x1;
pub(crate) const IN_MODIFY: u32 = 0x2;
pub(crate) const IN_ATTRIB: u32 = 0x4;
pub(crate) const IN_CLOSE_WRITE: u32 = 0x8;
pub(crate) const IN_CLOSE_NOWRITE: u32 = 0x10;
pub(crate) const IN_OPEN: u32 = 0x20;
const IN_MOVED_FROM: u32 = 0x40;
const IN_MOVED_TO: u32 = 0x80;
const IN_CREATE: u32 = 0x100;
const IN_DELETE: u32 = 0x200;
const IN_DELETE_SELF: u32 = 0x400;
const IN_MOVE_SELF: u32 = 0x800;
const IN_ALL_EVENTS: u32 = 0xfff;
const IN_Q_OVERFLOW: u32 = 0x4000;
const IN_IGNORED: u32 = 0x8000;
const IN_ONLYDIR: u32 = 0x0100_0000;
const IN_DONT_FOLLOW: u32 = 0x0200_0000;
const IN_EXCL_UNLINK: u32 = 0x0400_0000;
const IN_MASK_CREATE: u32 = 0x1000_0000;
const IN_MASK_ADD: u32 = 0x2000_0000;
const IN_ISDIR: u32 = 0x4000_0000;
const IN_ONESHOT: u32 = 0x8000_0000;
/// Linux's `max_queued_events`: past it, one `IN_Q_OVERFLOW` and nothing more until it's read.
const MOST_QUEUED: usize = 16384;
/// `struct inotify_event` without its name.
const HEADER: usize = 16;

/// Pairs an `IN_MOVED_FROM` with its `IN_MOVED_TO`.
static COOKIE: AtomicU32 = AtomicU32::new(1);

/// Which instances watch each file, by inode, and as which watch: the machine's, shared with the disk's observer.
#[derive(Default)]
pub(crate) struct Watches {
    by_ino: BTreeMap<Ino, Vec<(Weak<Inotify>, i32)>>,
}

pub(crate) type Registry = Arc<Mutex<Watches>>;

struct Event {
    wd: i32,
    mask: u32,
    cookie: u32,
    name: Option<String>,
}

impl Event {
    /// Its size read: the header, then the name with a NUL, padded to a multiple of the header's size.
    fn size(&self) -> usize {
        HEADER + self.name.as_ref().map_or(0, |n| (n.len() + 1).div_ceil(HEADER) * HEADER)
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let len = self.size() - HEADER;
        out.extend(self.wd.to_le_bytes());
        out.extend(self.mask.to_le_bytes());
        out.extend(self.cookie.to_le_bytes());
        out.extend((len as u32).to_le_bytes());
        let start = out.len();
        if let Some(name) = &self.name {
            out.extend(name.as_bytes());
        }
        out.resize(start + len, 0);
    }
}

#[derive(Default)]
struct State {
    events: VecDeque<Event>,
    /// Its watches, by descriptor: the file, and what it's watched for.
    watches: BTreeMap<i32, (Ino, u32)>,
    last_wd: i32,
}

/// An inotify instance.
pub(crate) struct Inotify {
    state: Mutex<State>,
    registry: Registry,
    /// Told when it gets an event, for `read`, `poll` and `epoll`.
    pub watchers: Watchers,
}

impl Drop for Inotify {
    fn drop(&mut self) {
        let me = self as *const Inotify;
        let mut registry = lock(&self.registry);
        for ino in lock(&self.state).watches.values().map(|(ino, _)| *ino) {
            if let Some(list) = registry.by_ino.get_mut(&ino) {
                list.retain(|(w, _)| !std::ptr::eq(w.as_ptr(), me));
                if list.is_empty() {
                    registry.by_ino.remove(&ino);
                }
            }
        }
    }
}

impl Inotify {
    fn push(state: &mut State, event: Event) {
        if let Some(last) = state.events.back()
            && last.wd == event.wd
            && last.mask == event.mask
            && last.cookie == event.cookie
            && last.name == event.name
        {
            // The same as the one before, still unread: Linux merges them.
            return;
        }
        if state.events.len() >= MOST_QUEUED {
            if state.events.back().is_none_or(|e| e.mask != IN_Q_OVERFLOW) {
                state.events.push_back(Event { wd: -1, mask: IN_Q_OVERFLOW, cookie: 0, name: None });
            }
            return;
        }
        state.events.push_back(event);
    }

    /// An event for watch `wd`, if it's watching for it: whether the watch is now over (one-shot), so it's to leave
    /// the registry.
    fn offer(&self, wd: i32, mask: u32, cookie: u32, name: Option<&str>) -> bool {
        let over = {
            let mut state = lock(&self.state);
            let Some(&(_, wanted)) = state.watches.get(&wd) else { return false };
            if wanted & mask & IN_ALL_EVENTS == 0 {
                return false;
            }
            let event = Event { wd, mask: mask & (wanted | IN_ISDIR), cookie, name: name.map(String::from) };
            Self::push(&mut state, event);
            let over = wanted & IN_ONESHOT != 0;
            if over {
                state.watches.remove(&wd);
                Self::push(&mut state, Event { wd, mask: IN_IGNORED, cookie: 0, name: None });
            }
            over
        };
        self.watchers.notify();
        over
    }

    /// Watch `wd` is over (its file is gone, or it was taken off): `IN_IGNORED`.
    fn ended(&self, wd: i32) {
        {
            let mut state = lock(&self.state);
            if state.watches.remove(&wd).is_none() {
                return;
            }
            Self::push(&mut state, Event { wd, mask: IN_IGNORED, cookie: 0, name: None });
        }
        self.watchers.notify();
    }

    pub fn readable(&self) -> bool {
        !lock(&self.state).events.is_empty()
    }

    /// How many bytes a read would give, all of it (`FIONREAD`).
    pub fn available(&self) -> usize {
        lock(&self.state).events.iter().map(Event::size).sum()
    }

    /// As many whole events as fit in `len` bytes; none if there are none; EINVAL if the first doesn't fit.
    pub fn take(&self, len: usize) -> Result<Option<Vec<u8>>, Errno> {
        let mut state = lock(&self.state);
        let Some(first) = state.events.front() else { return Ok(None) };
        if first.size() > len {
            return Err(EINVAL);
        }
        let mut out = Vec::new();
        while let Some(event) = state.events.front() {
            if out.len() + event.size() > len {
                break;
            }
            event.encode(&mut out);
            state.events.pop_front();
        }
        Ok(Some(out))
    }
}

/// Tells each watch on `ino` of `mask`, with `name` if it's of something in the directory `ino`.
fn deliver(registry: &Registry, ino: Ino, mask: u32, cookie: u32, name: Option<&str>) {
    let targets: Vec<(Arc<Inotify>, i32)> = match lock(registry).by_ino.get(&ino) {
        Some(list) => list.iter().filter_map(|(w, wd)| Some((w.upgrade()?, *wd))).collect(),
        None => return,
    };
    let over: Vec<(Arc<Inotify>, i32)> = targets.into_iter().filter(|(inotify, wd)| inotify.offer(*wd, mask, cookie, name)).collect();
    if !over.is_empty() {
        let mut registry = lock(registry);
        for (inotify, wd) in &over {
            forget_watch(&mut registry, ino, inotify, *wd);
        }
    }
}

fn forget_watch(registry: &mut Watches, ino: Ino, inotify: &Arc<Inotify>, wd: i32) {
    if let Some(list) = registry.by_ino.get_mut(&ino) {
        list.retain(|(w, d)| !(*d == wd && std::ptr::eq(w.as_ptr(), Arc::as_ptr(inotify))));
        if list.is_empty() {
            registry.by_ino.remove(&ino);
        }
    }
}

/// `ino` is gone: `IN_DELETE_SELF` to its watches, and each is over.
fn gone(registry: &Registry, ino: Ino) {
    deliver(registry, ino, IN_DELETE_SELF, 0, None);
    let ended: Vec<(Arc<Inotify>, i32)> = lock(registry).by_ino.remove(&ino).unwrap_or_default().iter().filter_map(|(w, wd)| Some((w.upgrade()?, *wd))).collect();
    for (inotify, wd) in ended {
        inotify.ended(wd);
    }
}

/// The names `ino` has in watched directories that watch for any of `mask`.
fn names_in_watched(registry: &Registry, disk: &Disk, ino: Ino, mask: u32) -> Vec<(Ino, String)> {
    // Taken under the registry's lock, looked at after it: an instance let go here must not drop while it's held.
    let watching: Vec<(Ino, Arc<Inotify>, i32)> = lock(registry)
        .by_ino
        .iter()
        .filter(|(dir, _)| **dir != ino)
        .flat_map(|(dir, list)| list.iter().filter_map(move |(w, wd)| Some((*dir, w.upgrade()?, *wd))))
        .collect();
    let mut dirs: Vec<Ino> = watching.iter().filter(|(_, i, wd)| lock(&i.state).watches.get(wd).is_some_and(|(_, m)| m & mask != 0)).map(|(dir, _, _)| *dir).collect();
    dirs.dedup();
    let mut out = Vec::new();
    for dir in dirs {
        if let Ok(entries) = disk.entries(dir) {
            out.extend(entries.into_iter().filter(|(_, child, _)| *child == ino).map(|(name, _, _)| (dir, name)));
        }
    }
    out
}

fn isdir(kind: Kind) -> u32 {
    if kind == Kind::Dir { IN_ISDIR } else { 0 }
}

/// Something happened to `ino` that names no directory: told to its watches, and to the watched directories it's in.
fn of_file(registry: &Registry, disk: &Disk, ino: Ino, mask: u32) {
    let mask = mask | disk.stat(ino).map_or(0, |s| isdir(s.kind));
    deliver(registry, ino, mask, 0, None);
    for (dir, name) in names_in_watched(registry, disk, ino, mask & IN_ALL_EVENTS) {
        deliver(registry, dir, mask, 0, Some(&name));
    }
}

/// The disk's observer: what changed, as inotify events.
pub(crate) fn observer(registry: Registry) -> layers::Observer<crate::Meta> {
    Box::new(move |disk: &Disk, change: Change<'_>| {
        if lock(&registry).by_ino.is_empty() {
            return;
        }
        match change {
            Change::Created { dir, name, kind, .. } => deliver(&registry, dir, IN_CREATE | isdir(kind), 0, Some(name)),
            Change::Linked { dir, name, ino } => {
                deliver(&registry, ino, IN_ATTRIB, 0, None);
                deliver(&registry, dir, IN_CREATE, 0, Some(name));
            }
            Change::Removed { dir, name, ino, kind, gone: is_gone } => {
                if kind != Kind::Dir {
                    deliver(&registry, ino, IN_ATTRIB, 0, None);
                }
                deliver(&registry, dir, IN_DELETE | isdir(kind), 0, Some(name));
                if is_gone {
                    gone(&registry, ino);
                }
            }
            Change::Moved { from_dir, from, to_dir, to, ino, kind, replaced } => {
                let cookie = COOKIE.fetch_add(1, Ordering::Relaxed);
                deliver(&registry, from_dir, IN_MOVED_FROM | isdir(kind), cookie, Some(from));
                deliver(&registry, to_dir, IN_MOVED_TO | isdir(kind), cookie, Some(to));
                deliver(&registry, ino, IN_MOVE_SELF | isdir(kind), 0, None);
                if let Some((old, is_gone)) = replaced {
                    deliver(&registry, old, IN_ATTRIB, 0, None);
                    if is_gone {
                        gone(&registry, old);
                    }
                }
            }
            Change::Written { ino } => of_file(&registry, disk, ino, IN_MODIFY),
            Change::Changed { ino } => of_file(&registry, disk, ino, IN_ATTRIB),
        }
    })
}

impl Kernel {
    /// Something happened to the file `ino` (opened, read, closed), found as `path`: told to its watches, and to its
    /// directory's with its name there.
    pub(crate) fn file_event(&self, ino: Ino, mask: u32, path: &str) {
        let inotify = !lock(&self.watches).by_ino.is_empty();
        let fanotify = self.fanotify_listening();
        if !inotify && !fanotify {
            return;
        }
        let (kind, parent) = {
            let disk = lock(&self.disk);
            let kind = disk.stat(ino).map_or(0, |s| isdir(s.kind));
            let parent = disk.resolve_parent(layers::ROOT, path).ok().filter(|(dir, name)| disk.lookup(*dir, name) == Ok(ino)).map(|(dir, name)| (dir, name.to_string()));
            (kind, parent)
        };
        if fanotify {
            self.fanotify_event(ino, parent.as_ref().map(|(dir, _)| *dir), kind != 0, mask, path);
        }
        if !inotify {
            return;
        }
        deliver(&self.watches, ino, mask | kind, 0, None);
        if let Some((dir, name)) = parent {
            deliver(&self.watches, dir, mask | kind, 0, Some(&name));
        }
    }
}

impl Task {
    pub(crate) fn inotify_init1(&mut self, flags: u32) -> Result<u64, Errno> {
        if flags & !(O_NONBLOCK | O_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let inotify = Arc::new(Inotify { state: Mutex::default(), registry: self.k.watches.clone(), watchers: Watchers::default() });
        let open = Open::new(Node::Inotify(inotify), flags & O_NONBLOCK, "anon_inode:inotify");
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn inotify(&self, fd: i32) -> Result<Arc<Inotify>, Errno> {
        match &self.file(fd)?.node {
            Node::Inotify(inotify) => Ok(inotify.clone()),
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn inotify_add_watch(&mut self, cx: &mut Cx<'_>, fd: i32, path_at: u64, mask: u32) -> Result<u64, Errno> {
        let inotify = self.inotify(fd)?;
        if mask & IN_ALL_EVENTS == 0 || (mask & IN_MASK_ADD != 0 && mask & IN_MASK_CREATE != 0) {
            return Err(EINVAL);
        }
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (cwd, _) = self.start_dir(AT_FDCWD)?;
        let ino = {
            let view = self.view();
            let disk = lock(&self.k.disk);
            let ino = view.resolve(&disk, cwd, &path, mask & IN_DONT_FOLLOW == 0)?;
            if mask & IN_ONLYDIR != 0 && disk.stat(ino)?.kind != Kind::Dir {
                return Err(ENOTDIR);
            }
            ino
        };
        let keep = mask & (IN_ALL_EVENTS | IN_ONESHOT | IN_EXCL_UNLINK);
        let mut registry = lock(&self.k.watches);
        let mut state = lock(&inotify.state);
        if let Some((&wd, &(_, old))) = state.watches.iter().find(|(_, (i, _))| *i == ino) {
            if mask & IN_MASK_CREATE != 0 {
                return Err(EEXIST);
            }
            let new = if mask & IN_MASK_ADD != 0 { old | keep } else { keep };
            state.watches.insert(wd, (ino, new));
            return Ok(wd as u64);
        }
        state.last_wd += 1;
        let wd = state.last_wd;
        state.watches.insert(wd, (ino, keep));
        registry.by_ino.entry(ino).or_default().push((Arc::downgrade(&inotify), wd));
        Ok(wd as u64)
    }

    pub(crate) fn inotify_rm_watch(&mut self, fd: i32, wd: i32) -> Result<u64, Errno> {
        let inotify = self.inotify(fd)?;
        let ino = lock(&inotify.state).watches.get(&wd).map(|(ino, _)| *ino).ok_or(EINVAL)?;
        forget_watch(&mut lock(&self.k.watches), ino, &inotify, wd);
        inotify.ended(wd);
        Ok(0)
    }
}
