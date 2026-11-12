//! fanotify: `fanotify_init` and `fanotify_mark`, in its classic mode, where each event read comes with a descriptor
//! open on the file it's about. A group is a file whose `read` gives `struct fanotify_event_metadata`s, ready in `poll`
//! and `epoll` while it holds one.
//!
//! A group marks inodes (and a directory's children, with `FAN_EVENT_ON_CHILD`), or a whole file system
//! (`FAN_MARK_FILESYSTEM`, and `FAN_MARK_MOUNT`, which here covers the file system its mount is of). Events come from
//! where inotify's do: opens, reads and closes as they happen (`Kernel::file_event`), and writes from the disk's
//! observer, whichever system call made them, each with the process (or thread) that caused it.
//!
//! Not yet: the modes that report file handles (`FAN_REPORT_FID` and those built on it, so the directory events,
//! `FAN_CREATE`, `FAN_DELETE`, `FAN_MOVE` and `FAN_ATTRIB`, which need it), pidfds (`FAN_REPORT_PIDFD`),
//! permission events (`FAN_OPEN_PERM`, `FAN_ACCESS_PERM`: nothing waits for a group's answer), and `FAN_OPEN_EXEC`,
//! taken but not told apart from an open.

use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::wait::Watchers;
use layers::{Change, Ino, Kind};

use super::{Node, Open};
use crate::abi::*;
use crate::process::Task;
use crate::{Disk, Kernel};

const FAN_ACCESS: u64 = 0x1;
const FAN_MODIFY: u64 = 0x2;
const FAN_CLOSE_WRITE: u64 = 0x8;
const FAN_CLOSE_NOWRITE: u64 = 0x10;
const FAN_OPEN: u64 = 0x20;
const FAN_OPEN_EXEC: u64 = 0x1000;
const FAN_Q_OVERFLOW: u64 = 0x4000;
const FAN_EVENT_ON_CHILD: u64 = 0x0800_0000;
const FAN_ONDIR: u64 = 0x4000_0000;
/// What a classic group can be told of.
const EVENTS: u64 = FAN_ACCESS | FAN_MODIFY | FAN_CLOSE_WRITE | FAN_CLOSE_NOWRITE | FAN_OPEN | FAN_OPEN_EXEC;

const FAN_CLOEXEC: u32 = 0x1;
const FAN_NONBLOCK: u32 = 0x2;
const FAN_CLASSES: u32 = 0xc;
const FAN_UNLIMITED_QUEUE: u32 = 0x10;
const FAN_UNLIMITED_MARKS: u32 = 0x20;
const FAN_ENABLE_AUDIT: u32 = 0x40;
const FAN_REPORT_TID: u32 = 0x100;
const FAN_REPORT_FD_ERROR: u32 = 0x2000;
const INIT_KNOWN: u32 = FAN_CLOEXEC | FAN_NONBLOCK | FAN_CLASSES | FAN_UNLIMITED_QUEUE | FAN_UNLIMITED_MARKS | FAN_ENABLE_AUDIT | FAN_REPORT_TID | FAN_REPORT_FD_ERROR;

const FAN_MARK_ADD: u32 = 0x1;
const FAN_MARK_REMOVE: u32 = 0x2;
const FAN_MARK_DONT_FOLLOW: u32 = 0x4;
const FAN_MARK_ONLYDIR: u32 = 0x8;
const FAN_MARK_MOUNT: u32 = 0x10;
const FAN_MARK_IGNORED_MASK: u32 = 0x20;
const FAN_MARK_IGNORED_SURV_MODIFY: u32 = 0x40;
const FAN_MARK_FLUSH: u32 = 0x80;
const FAN_MARK_FILESYSTEM: u32 = 0x100;
const FAN_MARK_EVICTABLE: u32 = 0x200;
const FAN_MARK_IGNORE: u32 = 0x400;
const MARK_KNOWN: u32 = FAN_MARK_ADD
    | FAN_MARK_REMOVE
    | FAN_MARK_DONT_FOLLOW
    | FAN_MARK_ONLYDIR
    | FAN_MARK_MOUNT
    | FAN_MARK_IGNORED_MASK
    | FAN_MARK_IGNORED_SURV_MODIFY
    | FAN_MARK_FLUSH
    | FAN_MARK_FILESYSTEM
    | FAN_MARK_EVICTABLE
    | FAN_MARK_IGNORE;

/// `struct fanotify_event_metadata`, and its version.
const METADATA_LEN: usize = 24;
const METADATA_VERSION: u8 = 3;
const FAN_NOFD: i32 = -1;
/// Linux's default queue length; past it, one `FAN_Q_OVERFLOW`.
const MOST_QUEUED: usize = 16384;

thread_local! {
    /// The guest thread this host thread runs, as its process and its own ID: who an event was caused by.
    static CALLER: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

/// This host thread runs the guest thread `tid` of process `pid`.
pub(crate) fn set_caller(pid: i32, tid: i32) {
    CALLER.with(|c| c.set((pid, tid)));
}

/// The machine's groups, and the paths files were last opened by, to name a file a write is told of.
#[derive(Default)]
pub(crate) struct Groups {
    groups: Vec<Weak<Fanotify>>,
    paths: BTreeMap<Ino, String>,
}

pub(crate) type Registry = Arc<Mutex<Groups>>;

#[derive(Debug, Clone, Copy, Default)]
struct Mark {
    mask: u64,
    ignored: u64,
    /// The ignore mask outlives a write to the file.
    survives: bool,
}

struct Event {
    mask: u64,
    ino: Ino,
    path: String,
    pid: i32,
}

#[derive(Default)]
struct State {
    marks: BTreeMap<Ino, Mark>,
    /// A file system's mark: every file on it.
    whole: Option<Mark>,
    events: VecDeque<Event>,
}

/// A fanotify group.
pub(crate) struct Fanotify {
    state: Mutex<State>,
    report_tid: bool,
    unlimited: bool,
    fd_error: bool,
    /// What each event's descriptor is opened with.
    event_flags: u32,
    /// Told when it gets an event, for `read`, `poll` and `epoll`.
    pub watchers: Watchers,
}

impl Fanotify {
    /// `mask` happened to `ino` (in `dir`, if known), by `caller`: queued if a mark wants it.
    fn offer(&self, ino: Ino, dir: Option<Ino>, mask: u64, path: &str, caller: (i32, i32)) {
        {
            let mut state = lock(&self.state);
            let (mut wanted, mut ignored) = (0, 0);
            let mut take = |m: &Mark| {
                wanted |= m.mask;
                ignored |= m.ignored;
            };
            if let Some(m) = state.marks.get(&ino) {
                take(m);
            }
            if let Some(m) = dir.and_then(|d| state.marks.get(&d)).filter(|m| m.mask & FAN_EVENT_ON_CHILD != 0) {
                take(m);
            }
            if let Some(m) = &state.whole {
                take(m);
            }
            if mask & FAN_MODIFY != 0
                && let Some(m) = state.marks.get_mut(&ino)
                && !m.survives
            {
                m.ignored = 0;
            }
            if mask & FAN_ONDIR != 0 && wanted & FAN_ONDIR == 0 {
                return;
            }
            let got = mask & EVENTS & wanted & !ignored;
            if got == 0 {
                return;
            }
            let pid = if self.report_tid { caller.1 } else { caller.0 };
            let flags = mask & FAN_ONDIR;
            if let Some(last) = state.events.back_mut()
                && last.ino == ino
                && last.pid == pid
                && last.mask != FAN_Q_OVERFLOW
            {
                // Still unread: the same file and process merge, as on Linux.
                last.mask |= got | flags;
            } else if state.events.len() >= MOST_QUEUED && !self.unlimited {
                if state.events.back().is_none_or(|e| e.mask != FAN_Q_OVERFLOW) {
                    state.events.push_back(Event { mask: FAN_Q_OVERFLOW, ino: 0, path: String::new(), pid: 0 });
                }
                return;
            } else {
                state.events.push_back(Event { mask: got | flags, ino, path: path.to_string(), pid });
            }
        }
        self.watchers.notify();
    }

    pub fn readable(&self) -> bool {
        !lock(&self.state).events.is_empty()
    }

    /// How many bytes a read would give, all of it (`FIONREAD`).
    pub fn available(&self) -> usize {
        lock(&self.state).events.len() * METADATA_LEN
    }

    /// As many events as fit in `len` bytes; none if there are none; EINVAL if not even one fits.
    fn take(&self, len: usize) -> Result<Option<Vec<Event>>, Errno> {
        let mut state = lock(&self.state);
        if state.events.is_empty() {
            return Ok(None);
        }
        if len < METADATA_LEN {
            return Err(EINVAL);
        }
        let n = (len / METADATA_LEN).min(state.events.len());
        Ok(Some(state.events.drain(..n).collect()))
    }
}

/// Tells every group of `mask` on `ino`, found as `path`, in the directory `dir` if it's known.
fn deliver(registry: &Registry, ino: Ino, dir: Option<Ino>, mask: u64, path: &str) {
    let groups: Vec<Arc<Fanotify>> = {
        let mut r = lock(registry);
        r.groups.retain(|g| g.strong_count() > 0);
        r.groups.iter().filter_map(Weak::upgrade).collect()
    };
    let caller = CALLER.with(Cell::get);
    for group in groups {
        group.offer(ino, dir, mask, path, caller);
    }
}

/// The disk's observer, inotify's and fanotify's: a write is `FAN_MODIFY`, named by the path it was last opened by.
pub(crate) fn observer(inotify: super::inotify::Registry, fanotify: Registry) -> layers::Observer<crate::Meta> {
    let mut first = super::inotify::observer(inotify);
    Box::new(move |disk: &Disk, change: Change<'_>| {
        if let Change::Written { ino } = change
            && !lock(&fanotify).groups.is_empty()
        {
            let path = lock(&fanotify).paths.get(&ino).cloned().unwrap_or_default();
            let dir = disk.resolve_parent(layers::ROOT, &path).ok().filter(|(d, name)| disk.lookup(*d, name) == Ok(ino)).map(|(d, _)| d);
            deliver(&fanotify, ino, dir, FAN_MODIFY, &path);
        }
        first(disk, change);
    })
}

impl Kernel {
    /// An open, read or close of `ino`, found as `path` (in the directory `dir`, a directory if `isdir`), for the
    /// fanotify groups. inotify's `IN_*` values for these are fanotify's.
    pub(crate) fn fanotify_event(&self, ino: Ino, dir: Option<Ino>, isdir: bool, mask: u32, path: &str) {
        {
            let mut r = lock(&self.fanotify);
            if r.groups.is_empty() {
                return;
            }
            if mask & super::inotify::IN_OPEN != 0 {
                r.paths.insert(ino, path.to_string());
            }
        }
        let mask = u64::from(mask) & EVENTS | if isdir { FAN_ONDIR } else { 0 };
        deliver(&self.fanotify, ino, dir, mask, path);
    }

    /// Whether any group is listening.
    pub(crate) fn fanotify_listening(&self) -> bool {
        !lock(&self.fanotify).groups.is_empty()
    }
}

impl Task {
    /// `fanotify_init`: a classic notification group (`CAP_SYS_ADMIN`).
    pub(crate) fn fanotify_init(&mut self, flags: u32, event_flags: u32) -> Result<u64, Errno> {
        const EVENT_FLAGS: u32 = O_ACCMODE | O_LARGEFILE | O_CLOEXEC | O_NONBLOCK | O_APPEND | 0o10000 | 0o4010000 | 0o1000000;
        if !lock(&self.process.state).creds.capable(crate::process::creds::CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        if flags & !INIT_KNOWN != 0 || flags & FAN_CLASSES == FAN_CLASSES || event_flags & !EVENT_FLAGS != 0 {
            return Err(EINVAL);
        }
        let group = Arc::new(Fanotify {
            state: Mutex::default(),
            report_tid: flags & FAN_REPORT_TID != 0,
            unlimited: flags & FAN_UNLIMITED_QUEUE != 0,
            fd_error: flags & FAN_REPORT_FD_ERROR != 0,
            event_flags,
            watchers: Watchers::default(),
        });
        lock(&self.k.fanotify).groups.push(Arc::downgrade(&group));
        let open = Open::new(Node::Fanotify(group), O_RDWR | if flags & FAN_NONBLOCK != 0 { O_NONBLOCK } else { 0 }, "anon_inode:[fanotify]");
        lock(&self.process.state).files.add(open, flags & FAN_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn fanotify(&self, fd: i32) -> Result<Arc<Fanotify>, Errno> {
        match &self.file(fd)?.node {
            Node::Fanotify(group) => Ok(group.clone()),
            _ => Err(EINVAL),
        }
    }

    /// `fanotify_mark`: adds to, takes from, or flushes a group's marks on a file, or on a whole file system.
    pub(crate) fn fanotify_mark(&mut self, cx: &mut Cx<'_>, fd: i32, flags: u32, mask: u64, dirfd: i32, path_at: u64) -> Result<u64, Errno> {
        let group = self.fanotify(fd)?;
        let action = flags & (FAN_MARK_ADD | FAN_MARK_REMOVE | FAN_MARK_FLUSH);
        let whole = flags & (FAN_MARK_MOUNT | FAN_MARK_FILESYSTEM) != 0;
        if flags & !MARK_KNOWN != 0 || action.count_ones() != 1 || flags & FAN_MARK_MOUNT != 0 && flags & FAN_MARK_FILESYSTEM != 0 {
            return Err(EINVAL);
        }
        if action == FAN_MARK_FLUSH {
            let mut state = lock(&group.state);
            if whole {
                state.whole = None;
            } else {
                state.marks.clear();
            }
            return Ok(0);
        }
        if mask & !(EVENTS | FAN_ONDIR | FAN_EVENT_ON_CHILD) != 0 || mask == 0 {
            return Err(EINVAL);
        }
        let (ino, path) = if path_at == 0 {
            let open = self.file(dirfd)?;
            match open.node {
                Node::File(ino) | Node::Dir(ino) => (ino, open.path.clone()),
                _ => return Err(EINVAL),
            }
        } else {
            let path = self.read_path(cx, path_at)?;
            let (start, start_path) = self.start_dir(dirfd)?;
            let view = self.view();
            let disk = lock(&self.k.disk);
            let ino = view.resolve(&disk, start, &path, flags & FAN_MARK_DONT_FOLLOW == 0)?;
            if flags & FAN_MARK_ONLYDIR != 0 && disk.stat(ino)?.kind != Kind::Dir {
                return Err(ENOTDIR);
            }
            (ino, crate::process::exe_path(&start_path, &path))
        };
        let ignore = flags & (FAN_MARK_IGNORED_MASK | FAN_MARK_IGNORE) != 0;
        let mut state = lock(&group.state);
        let mark = if whole { state.whole.get_or_insert_default() } else { state.marks.entry(ino).or_default() };
        match (action, ignore) {
            (FAN_MARK_ADD, false) => mark.mask |= mask,
            (FAN_MARK_ADD, true) => {
                mark.ignored |= mask;
                mark.survives |= flags & FAN_MARK_IGNORED_SURV_MODIFY != 0;
            }
            (_, false) => mark.mask &= !mask,
            (_, true) => mark.ignored &= !mask,
        }
        let empty = mark.mask & (EVENTS | FAN_ONDIR | FAN_EVENT_ON_CHILD) == 0 && mark.ignored == 0;
        if empty {
            if whole {
                state.whole = None;
            } else {
                state.marks.remove(&ino);
            }
        }
        drop(state);
        lock(&self.k.fanotify).paths.entry(ino).or_insert(path);
        Ok(0)
    }

    /// A group's `read`: each event's metadata, with a descriptor open on its file in this process.
    fn fanotify_read(&self, group: &Arc<Fanotify>, events: Vec<Event>) -> Vec<u8> {
        let mut out = Vec::with_capacity(events.len() * METADATA_LEN);
        for e in events {
            let fd = if e.mask == FAN_Q_OVERFLOW {
                FAN_NOFD
            } else {
                match self.open_ino(e.ino, group.event_flags, &e.path) {
                    Ok(open) => lock(&self.process.state).files.add(open, group.event_flags & O_CLOEXEC != 0, 0).unwrap_or(FAN_NOFD),
                    Err(Errno(errno)) if group.fd_error => -errno,
                    Err(_) => FAN_NOFD,
                }
            };
            out.extend((METADATA_LEN as u32).to_le_bytes());
            out.extend([METADATA_VERSION, 0]);
            out.extend((METADATA_LEN as u16).to_le_bytes());
            out.extend(e.mask.to_le_bytes());
            out.extend(fd.to_le_bytes());
            out.extend(e.pid.to_le_bytes());
        }
        out
    }

    /// Waits for (or with `O_NONBLOCK`, doesn't) events for a read of `len` bytes.
    pub(crate) fn fanotify_take(&self, open: &Open, group: &Arc<Fanotify>, len: usize) -> Result<Vec<u8>, Errno> {
        let events = self.wait_or_not(open, |_| group.take(len))?;
        Ok(self.fanotify_read(group, events))
    }
}
