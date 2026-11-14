//! Files: each process's table of open files, and the system calls on files and paths. Regular files and
//! directories live on the machine's disk (`layers`); devices are what their numbers say; `/proc` is made as it's read.

pub(crate) mod aio;
mod epoll;
mod events;
pub(crate) mod fanotify;
pub(crate) mod inotify;
mod io;
pub(crate) mod mount;
pub(crate) mod xattr;
mod meta;
mod pipe;
mod poll;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use layers::{Ino, Kind, New};

use epoll::Epoll;
use events::{Eventfd, Timerfd};
pub(crate) use meta::F_GETLK;
pub(crate) use pipe::Pipe;

use crate::abi::*;
use crate::net::{Netlink, SockFile};
use crate::process::{Task, exe_path};
use crate::signal::{SIGPIPE, SIGTTIN};
use crate::tty::{Termios, Tty};
use crate::{Disk, Kernel, Meta, os};

/// The most files a process can have open, until it changes its `RLIMIT_NOFILE`.
const MOST_FILES: i32 = 1024;
/// The most one read or write moves at once.
const MOST_AT_ONCE: u64 = 1 << 24;

/// What an open file is.
pub(crate) enum Node {
    /// The host's standard streams.
    Stdin,
    Stdout,
    Stderr,
    File(Ino),
    Dir(Ino),
    /// A pipe, and whether this is its write end.
    Pipe(Arc<Pipe>, bool),
    /// A named pipe (FIFO) on the disk, by its inode: readers and writers share one buffer.
    Fifo(Arc<Pipe>, Ino),
    Null,
    Zero,
    Random,
    /// A file made when it was opened, such as `/proc/self/maps`.
    Made(Vec<u8>),
    /// A directory made when it was opened, such as `/proc/self/fd`: its entries' names and types (`DT_*`).
    Listing(Vec<(String, u8)>),
    /// A terminal: the console, or a pty's slave side.
    Tty(Arc<Tty>),
    /// A pty's master side.
    PtyMaster(Arc<Tty>),
    /// `signalfd`: reads the signals in its mask.
    Signalfd(Mutex<u64>),
    Eventfd(Arc<Eventfd>),
    Timerfd(Arc<Timerfd>),
    Epoll(Arc<Epoll>),
    Socket(Arc<SockFile>),
    /// A netlink socket: the machine's links, addresses and routes.
    Netlink(Arc<Netlink>),
    /// A pidfd: a process, readable once it's ended.
    Pidfd(Arc<crate::process::Process>),
    /// A memfd: a file in memory, and whether it's secret (`memfd_secret`: mapped, never read or written).
    Memfd(Arc<crate::mm::shared::SharedMem>, bool),
    /// A POSIX message queue.
    Mqueue(Arc<crate::ipc::mqueue::MQueue>),
    Inotify(Arc<inotify::Inotify>),
    Fanotify(Arc<fanotify::Fanotify>),
    /// A namespace, as `/proc/<pid>/ns/<kind>` opens to and `setns` takes.
    Ns(crate::ns::NsRef),
    /// The new mount API's filesystem context (`fsopen`, `fspick`) and detached mount (`fsmount`, `open_tree`).
    FsContext(Arc<std::sync::Mutex<mount::FsContext>>),
    MountFd(Arc<std::sync::Mutex<Option<mount::Detached>>>),
    /// The render node, `/dev/dri/renderD128`: the machine's GPU (`os::gpu`).
    Drm(Arc<os::gpu::Drm>),
}

/// An open file: shared by every descriptor `dup`ed or inherited from the one that opened it.
pub(crate) struct Open {
    pub node: Node,
    /// open's flags, as x86-64 has them.
    pub flags: Mutex<u32>,
    pub offset: Mutex<u64>,
    pub path: String,
    /// For a file on the disk, what keeps it while it's open.
    _hold: Option<Hold>,
}

/// A file on the disk kept while it's open, so removing its last name doesn't remove it until it's closed, as on
/// Unix: programs make nameless temporary files that way.
struct Hold {
    k: Weak<Kernel>,
    ino: Ino,
    /// Opened by fanotify for an event: what's done with it makes no events (`FMODE_NONOTIFY`).
    quiet: bool,
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(k) = self.k.upgrade() {
            lock(&k.disk).release(self.ino);
        }
    }
}

impl Task {
    /// The disk's file or directory `ino` opened with `flags`, as `path`, kept while it's open: fanotify's
    /// descriptors, opened without a path walk (or an event of their own).
    pub(crate) fn open_ino(&self, ino: Ino, flags: u32, path: &str) -> Result<Arc<Open>, Errno> {
        let mut disk = lock(&self.k.disk);
        let node = match disk.stat(ino)?.kind {
            Kind::Dir => Node::Dir(ino),
            Kind::File => Node::File(ino),
            _ => return Err(ENOENT),
        };
        disk.hold(ino);
        let hold = Some(Hold { k: Arc::downgrade(&self.k), ino, quiet: true });
        Ok(Arc::new(Open { node, flags: Mutex::new(flags & !O_CLOEXEC), offset: Mutex::new(0), path: path.to_string(), _hold: hold }))
    }
}

impl Open {
    pub fn new(node: Node, flags: u32, path: impl Into<String>) -> Arc<Open> {
        Arc::new(Open { node, flags: Mutex::new(flags), offset: Mutex::new(0), path: path.into(), _hold: None })
    }

    /// Whether what's done with it makes inotify and fanotify events.
    fn notifies(&self) -> bool {
        self._hold.as_ref().is_none_or(|h| !h.quiet)
    }

    pub(crate) fn nonblock(&self) -> bool {
        *lock(&self.flags) & O_NONBLOCK != 0
    }

    fn readable(&self) -> bool {
        let flags = *lock(&self.flags);
        flags & O_ACCMODE != O_WRONLY && flags & O_PATH == 0
    }

    fn writable(&self) -> bool {
        let flags = *lock(&self.flags);
        flags & O_ACCMODE != 0 && flags & O_PATH == 0
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // A file on the disk closing, for inotify; its hold goes after this.
        if let Some(hold) = &self._hold
            && let Some(k) = hold.k.upgrade()
            && *lock(&self.flags) & O_PATH == 0
            && !hold.quiet
        {
            let mask = if self.writable() { inotify::IN_CLOSE_WRITE } else { inotify::IN_CLOSE_NOWRITE };
            k.file_event(hold.ino, mask, &self.path);
        }
        match &self.node {
            Node::Pipe(pipe, write_end) => pipe.close(*write_end),
            Node::Fifo(pipe, _) => {
                let acc = *lock(&self.flags) & O_ACCMODE;
                if acc != O_WRONLY {
                    pipe.close(false);
                }
                if acc != 0 {
                    pipe.close(true);
                }
            }
            Node::Tty(tty) if tty.index.is_some() => tty.close_slave(),
            Node::PtyMaster(tty) => tty.close_master(),
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Fd {
    open: Arc<Open>,
    cloexec: bool,
}

/// A process's open files, by descriptor.
#[derive(Clone)]
pub(crate) struct Files {
    fds: BTreeMap<i32, Fd>,
    /// One past the highest descriptor it may have: its `RLIMIT_NOFILE`.
    limit: i32,
}

impl Default for Files {
    fn default() -> Files {
        Files { fds: BTreeMap::new(), limit: MOST_FILES }
    }
}

impl Files {
    /// A new `RLIMIT_NOFILE`. Descriptors already past it stay open, as on Linux.
    pub fn set_limit(&mut self, limit: i32) {
        self.limit = limit;
    }

    /// 0, 1 and 2: the console, or else the host's stdin, stdout and stderr.
    pub fn standard(console: Option<Arc<Tty>>) -> Files {
        let mut files = Files::default();
        if let Some(console) = console {
            let open = Open::new(Node::Tty(console), O_RDWR, "/dev/console");
            for fd in 0..3 {
                files.fds.insert(fd, Fd { open: open.clone(), cloexec: false });
            }
            return files;
        }
        for (fd, node, flags) in [(0, Node::Stdin, 0), (1, Node::Stdout, O_WRONLY), (2, Node::Stderr, O_WRONLY)] {
            files.fds.insert(fd, Fd { open: Open::new(node, flags, format!("/dev/fd/{fd}")), cloexec: false });
        }
        files
    }

    pub fn get(&self, fd: i32) -> Option<Arc<Open>> {
        self.fds.get(&fd).map(|f| f.open.clone())
    }

    /// Puts `open` at the lowest free descriptor from `lowest`.
    pub fn add(&mut self, open: Arc<Open>, cloexec: bool, lowest: i32) -> Result<i32, Errno> {
        let fd = (lowest.max(0)..self.limit).find(|fd| !self.fds.contains_key(fd)).ok_or(EMFILE)?;
        self.fds.insert(fd, Fd { open, cloexec });
        Ok(fd)
    }

    /// Takes a descriptor out; its file closes when nothing else has it.
    fn remove(&mut self, fd: i32) -> Option<Arc<Open>> {
        self.fds.remove(&fd).map(|f| f.open)
    }

    /// Its descriptors, lowest first.
    pub fn numbers(&self) -> Vec<i32> {
        self.fds.keys().copied().collect()
    }

    pub fn close_on_exec(&mut self) {
        self.fds.retain(|_, f| !f.cloexec);
    }
}

/// Whether the disk not finding a path may be because a link on the way leads somewhere lambda makes.
fn not_on_disk(e: Errno) -> bool {
    e == ENOENT || e == ENOTDIR
}

/// `stat` for a file on the disk.
fn stat_of(disk: &Disk, ino: Ino) -> Result<Stat, Errno> {
    let s = disk.stat(ino)?;
    let meta = disk.meta(ino)?;
    let kind = match s.kind {
        Kind::File => S_IFREG,
        Kind::Dir => S_IFDIR,
        Kind::Symlink => S_IFLNK,
    };
    let mode = if meta.mode & S_IFMT != 0 { meta.mode } else { kind | meta.mode & 0o7777 };
    Ok(Stat {
        dev: disk.volatile(ino).map_or(0x803, mount::tmpfs_dev),
        ino,
        mode,
        nlink: s.links,
        uid: meta.uid,
        gid: meta.gid,
        rdev: meta.rdev,
        size: if s.kind == Kind::Dir { 4096 } else { s.size as i64 },
        blksize: 4096,
        blocks: s.size.div_ceil(512) as i64,
        time: ((s.modified / 1_000_000_000) as i64, (s.modified % 1_000_000_000) as i64),
    })
}

fn stat_of_open(disk: &Disk, open: &Open) -> Result<Stat, Errno> {
    let device = |rdev: u64| Stat { mode: S_IFCHR | 0o666, nlink: 1, rdev, blksize: 4096, ..Stat::default() };
    match &open.node {
        Node::Tty(tty) => match tty.index {
            Some(n) => Ok(os::pts_stat(n)),
            None => disk.resolve(layers::ROOT, &open.path, true).map_err(Errno::from).and_then(|ino| stat_of(disk, ino)),
        },
        Node::PtyMaster(_) => disk.resolve(layers::ROOT, "/dev/ptmx", true).map_err(Errno::from).and_then(|ino| stat_of(disk, ino)),
        Node::File(ino) | Node::Dir(ino) | Node::Fifo(_, ino) => stat_of(disk, *ino),
        Node::Stdin | Node::Stdout | Node::Stderr => Ok(Stat { mode: S_IFIFO | 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        // On a device of their own, as Linux's pipefs, each with its own inode.
        Node::Pipe(pipe, _) => Ok(Stat { dev: 0xd, ino: pipe.ino, mode: S_IFIFO | 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        // Opened from its file on the disk: that file's stat. Made otherwise (no file): a device of its own.
        Node::Null | Node::Zero | Node::Random | Node::Drm(_) if let Some(hold) = &open._hold => stat_of(disk, hold.ino),
        Node::Drm(_) => Ok(device(os::gpu::RDEV)),
        Node::Null => Ok(device(1 << 8 | 3)),
        Node::Zero => Ok(device(1 << 8 | 5)),
        Node::Random => Ok(device(1 << 8 | 9)),
        Node::Made(bytes) => {
            let (dev, ino) = os::made_identity(&open.path);
            Ok(Stat { dev, ino, mode: S_IFREG | 0o444, nlink: 1, size: bytes.len() as i64, blksize: 4096, ..Stat::default() })
        }
        // Linked from its parent and itself, and from each directory in it: `/proc/self/task`'s count is its threads'
        // and two, which sandboxes read to see that a process is single-threaded.
        Node::Listing(entries) => {
            let (dev, ino) = os::made_identity(&open.path);
            let dirs = entries.iter().filter(|(_, kind)| *kind == DT_DIR).count();
            Ok(Stat { dev, ino, mode: S_IFDIR | 0o555, nlink: 2 + dirs as u32, blksize: 4096, ..Stat::default() })
        }
        Node::Signalfd(_) | Node::Eventfd(_) | Node::Timerfd(_) | Node::Epoll(_) | Node::Pidfd(_) | Node::Inotify(_) | Node::Fanotify(_) => Ok(Stat { mode: 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::Ns(r) => Ok(Stat { dev: crate::ns::NSFS_DEV, ino: r.id(), mode: S_IFREG | 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::FsContext(_) | Node::MountFd(_) => Ok(Stat { mode: 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::Socket(s) => Ok(Stat { mode: S_IFSOCK | 0o777, nlink: 1, ino: s.socket.id(), blksize: 4096, ..Stat::default() }),
        Node::Mqueue(queue) => Ok(Stat { dev: 0x13, ino: queue.ino, mode: queue.mode, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::Memfd(mem, _) => {
            let size = mem.size();
            Ok(Stat { dev: 0x5, ino: mem.ino, mode: S_IFREG | 0o777, nlink: 1, size: size as i64, blksize: 4096, blocks: (mem.resident(0, u64::MAX) * 8) as i64, ..Stat::default() })
        }
        Node::Netlink(n) => Ok(Stat { mode: S_IFSOCK | 0o777, nlink: 1, ino: n.id, blksize: 4096, ..Stat::default() }),
    }
}

impl Task {
    pub(crate) fn file(&self, fd: i32) -> Result<Arc<Open>, Errno> {
        lock(&self.process.state).files.get(fd).ok_or(EBADF)
    }

    pub(crate) fn read_path(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<String, Errno> {
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 4096))?;
        String::from_utf8(raw).map_err(|_| EINVAL)
    }

    /// The directory a path is relative to: the working directory, or an open directory.
    pub(crate) fn start_dir(&self, dirfd: i32) -> Result<(Ino, String), Errno> {
        if dirfd == AT_FDCWD {
            return Ok(lock(&self.process.state).cwd.clone());
        }
        let open = self.file(dirfd)?;
        match open.node {
            Node::Dir(ino) => Ok((ino, open.path.clone())),
            // A made directory: what's under it is found by its path (`readlinkat(fd, "3")` in `/proc/self/fd`).
            Node::Listing(_) => Ok((layers::ROOT, open.path.clone())),
            _ => Err(ENOTDIR),
        }
    }

    fn read_from(&self, open: &Open, len: u64, at: Option<u64>) -> Result<Vec<u8>, Errno> {
        if !open.readable() {
            return Err(EBADF);
        }
        let len = len.min(MOST_AT_ONCE) as usize;
        let from_bytes = |bytes: &[u8]| -> Vec<u8> {
            let mut offset = lock(&open.offset);
            let start = at.unwrap_or(*offset).min(bytes.len() as u64) as usize;
            let out = bytes[start..bytes.len().min(start + len)].to_vec();
            if at.is_none() {
                *offset = (start + out.len()) as u64;
            }
            out
        };
        match &open.node {
            Node::Stdin => {
                let mut buffer = vec![0; len.min(1 << 16)];
                let n = self.k.host.read(&mut buffer);
                buffer.truncate(n);
                Ok(buffer)
            }
            Node::Stdout | Node::Stderr => Err(EBADF),
            Node::File(ino) => {
                let buffer = {
                    let mut offset = lock(&open.offset);
                    let start = at.unwrap_or(*offset);
                    let mut buffer = vec![0; len];
                    let n = lock(&self.k.disk).read(*ino, start, &mut buffer)?;
                    buffer.truncate(n);
                    if at.is_none() {
                        *offset = start + n as u64;
                    }
                    buffer
                };
                if !buffer.is_empty() && open.notifies() {
                    self.k.file_event(*ino, inotify::IN_ACCESS, &open.path);
                }
                Ok(buffer)
            }
            Node::Dir(_) | Node::Listing(_) => Err(EISDIR),
            Node::Memfd(_, true) => Err(EINVAL),
            Node::Memfd(mem, false) => {
                let mut offset = lock(&open.offset);
                let start = at.unwrap_or(*offset);
                let mut buffer = vec![0; len];
                let n = mem.read(start, &mut buffer);
                buffer.truncate(n);
                if at.is_none() {
                    *offset = start + n as u64;
                }
                Ok(buffer)
            }
            Node::Pipe(_, true) => Err(EBADF),
            Node::Pipe(pipe, false) => pipe.read(&self.waiter, len, open.nonblock()),
            Node::Fifo(pipe, _) => pipe.read(&self.waiter, len, open.nonblock()),
            Node::Null => Ok(Vec::new()),
            Node::Zero => Ok(vec![0; len]),
            Node::Random => {
                let mut buffer = vec![0; len];
                self.k.host.random(&mut buffer);
                Ok(buffer)
            }
            Node::Made(bytes) => Ok(from_bytes(bytes)),
            Node::Mqueue(queue) => Ok(from_bytes(&queue.status())),
            Node::Tty(tty) => {
                self.background_read(tty)?;
                tty.read(&self.waiter, len, open.nonblock())
            }
            Node::PtyMaster(tty) => tty.read_master(&self.waiter, len, open.nonblock()),
            Node::Signalfd(mask) => {
                if len < 128 {
                    return Err(EINVAL);
                }
                let mask = *lock(mask);
                let take = |task: &Task| {
                    let mut out = Vec::new();
                    while out.len() + 128 <= len {
                        match task.take_signal(mask) {
                            Some(info) => out.extend(info),
                            None => break,
                        }
                    }
                    Ok((!out.is_empty()).then_some(out))
                };
                // The signals it reads are blocked, so they wake no thread: the process's signal watchers say.
                self.wait_or_not(open, take)
            }
            Node::Eventfd(eventfd) => {
                if len < 8 {
                    return Err(EINVAL);
                }
                let value = self.wait_or_not(open, |_| Ok(eventfd.take()))?;
                Ok(value.to_le_bytes().to_vec())
            }
            Node::Timerfd(timer) => {
                if len < 8 {
                    return Err(EINVAL);
                }
                let value = self.wait_or_not(open, |task| Ok(timer.take(task.timer_now(timer.realtime))))?;
                Ok(value.to_le_bytes().to_vec())
            }
            Node::Epoll(_) | Node::Pidfd(_) | Node::Ns(_) | Node::FsContext(_) | Node::MountFd(_) | Node::Drm(_) => Err(EINVAL),
            Node::Inotify(inotify) => self.wait_or_not(open, |_| inotify.take(len)),
            Node::Fanotify(group) => {
                let group = group.clone();
                self.fanotify_take(open, &group, len)
            }
            Node::Socket(s) => self.sock_recv(open, s, len, 0).map(|got| got.data),
            Node::Netlink(n) => self.wait_or_not(open, |_| Ok(n.recv(len, false).map(|(data, _)| data))),
        }
    }

    /// What `attempt` gets now; if nothing, EAGAIN for a nonblocking file, else waits for it among the file's watchers.
    fn wait_or_not<R>(&self, open: &Open, attempt: impl FnMut(&Task) -> Result<Option<R>, Errno>) -> Result<R, Errno> {
        let mut attempt = attempt;
        if open.nonblock() {
            return attempt(self)?.ok_or(EAGAIN);
        }
        self.wait_ready(None, &[open], attempt)?.ok_or(EINTR)
    }

    /// `eventfd2`: a counter that starts at `initial`.
    pub(crate) fn eventfd(&mut self, initial: u64, flags: u32) -> Result<u64, Errno> {
        const EFD_SEMAPHORE: u32 = 1;
        if flags & !(EFD_SEMAPHORE | O_NONBLOCK | O_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let open = Open::new(Node::Eventfd(Eventfd::new(initial & 0xFFFF_FFFF, flags & EFD_SEMAPHORE != 0)), O_RDWR | flags & O_NONBLOCK, "anon_inode:[eventfd]");
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    /// `timerfd_create`: a disarmed timer on `clock`.
    pub(crate) fn timerfd_create(&mut self, clock: u64, flags: u32) -> Result<u64, Errno> {
        // REALTIME and REALTIME_ALARM are the wall clock; MONOTONIC, BOOTTIME and BOOTTIME_ALARM the monotonic one.
        let realtime = match clock {
            0 | 8 => true,
            1 | 7 | 9 => false,
            _ => return Err(EINVAL),
        };
        if flags & !(O_NONBLOCK | O_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let open = Open::new(Node::Timerfd(Timerfd::new(realtime)), O_RDWR | flags & O_NONBLOCK, "anon_inode:[timerfd]");
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn timer(&self, fd: i32) -> Result<Arc<Timerfd>, Errno> {
        match &self.file(fd)?.node {
            Node::Timerfd(timer) => Ok(timer.clone()),
            _ => Err(EINVAL),
        }
    }

    /// An `itimerspec` (interval, then value) from nanoseconds.
    fn write_itimerspec(&mut self, cx: &mut Cx<'_>, at: u64, (value, interval): (u64, u64)) -> Result<(), Errno> {
        let mut bytes = Vec::with_capacity(32);
        for ns in [interval, value] {
            bytes.extend((ns / 1_000_000_000).to_le_bytes());
            bytes.extend((ns % 1_000_000_000).to_le_bytes());
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))
    }

    /// `timerfd_settime`: arms or disarms it, `TFD_TIMER_ABSTIME` for a time on its clock rather than from now.
    pub(crate) fn timerfd_settime(&mut self, cx: &mut Cx<'_>, fd: i32, flags: u64, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        const TFD_TIMER_ABSTIME: u64 = 1;
        let timer = self.timer(fd)?;
        let words = self.mem(cx, false, |t| (0..4).map(|i| user::read_u64(&t.space(), new_at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
        if words[1] >= 1_000_000_000 || words[3] >= 1_000_000_000 {
            return Err(EINVAL);
        }
        let ns = |sec: u64, nsec: u64| sec.saturating_mul(1_000_000_000).saturating_add(nsec);
        let (interval, value) = (ns(words[0], words[1]), ns(words[2], words[3]));
        let old = timer.set(self.timer_now(timer.realtime), value, interval, flags & TFD_TIMER_ABSTIME != 0);
        if old_at != 0 {
            self.write_itimerspec(cx, old_at, old)?;
        }
        Ok(0)
    }

    pub(crate) fn timerfd_gettime(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64) -> Result<u64, Errno> {
        let timer = self.timer(fd)?;
        let now = timer.get(self.timer_now(timer.realtime));
        self.write_itimerspec(cx, at, now)?;
        Ok(0)
    }

    /// `signalfd4`: a new signalfd, or a new mask for one.
    pub(crate) fn signalfd(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, size: u64, flags: u32) -> Result<u64, Errno> {
        if size != 8 {
            return Err(EINVAL);
        }
        let mask = self.mem(cx, false, |t| user::read_u64(&t.space(), at))?;
        if fd >= 0 {
            let open = self.file(fd)?;
            let Node::Signalfd(current) = &open.node else { return Err(EINVAL) };
            *lock(current) = mask;
            return Ok(fd as u64);
        }
        let open = Open::new(Node::Signalfd(Mutex::new(mask)), O_RDWR | flags & O_NONBLOCK, "anon_inode:[signalfd]");
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    /// A process in the background reading its terminal gets SIGTTIN, as job control has it.
    fn background_read(&self, tty: &Arc<Tty>) -> Result<(), Errno> {
        let (mine, pgid) = {
            let state = lock(&self.process.state);
            (state.ctty.as_ref().is_some_and(|c| Arc::ptr_eq(c, tty)), state.pgid)
        };
        if !mine || tty.session().is_none() || tty.foreground() == pgid || tty.foreground() == 0 {
            return Ok(());
        }
        if self.k.signal_group(pgid, SIGTTIN) {
            return Err(EINTR);
        }
        Ok(())
    }

    /// Typing into a pty's master, or the console's keyboard: the signals it asks for go to their process groups.
    fn type_into(&self, tty: &Tty, data: &[u8]) {
        for (pgid, signal) in tty.input(data) {
            self.k.signal_group(pgid, signal);
        }
    }

    fn write_to(&self, open: &Open, data: &[u8], at: Option<u64>) -> Result<usize, Errno> {
        if !open.writable() {
            return Err(EBADF);
        }
        match &open.node {
            Node::Stdout => {
                self.k.host.write(host::Stream::Out, data);
                Ok(data.len())
            }
            Node::Stderr => {
                self.k.host.write(host::Stream::Err, data);
                Ok(data.len())
            }
            Node::File(ino) => {
                let mut offset = lock(&open.offset);
                let mut disk = lock(&self.k.disk);
                let start = match at {
                    Some(at) => at,
                    None if *lock(&open.flags) & O_APPEND != 0 => disk.stat(*ino)?.size,
                    None => *offset,
                };
                let n = disk.write(*ino, start, data, self.k.now())?;
                if at.is_none() {
                    *offset = start + n as u64;
                }
                Ok(n)
            }
            Node::Memfd(_, true) => Err(EINVAL),
            Node::Memfd(mem, false) => {
                let mut offset = lock(&open.offset);
                let start = match at {
                    Some(at) => at,
                    None if *lock(&open.flags) & O_APPEND != 0 => mem.size(),
                    None => *offset,
                };
                let n = mem.write(start, data)?;
                if at.is_none() {
                    *offset = start + n as u64;
                }
                Ok(n)
            }
            Node::Pipe(pipe, true) | Node::Fifo(pipe, _) => match pipe.write(&self.waiter, data, open.nonblock()) {
                Err(EPIPE) => {
                    self.k.signal_thread(&self.process, self.tid, SIGPIPE);
                    Err(EPIPE)
                }
                other => other,
            },
            Node::Null | Node::Zero | Node::Random => Ok(data.len()),
            Node::Tty(tty) => tty.write(data),
            Node::PtyMaster(tty) => {
                self.type_into(tty, data);
                Ok(data.len())
            }
            Node::Eventfd(eventfd) => {
                let value = u64::from_le_bytes(data.get(..8).ok_or(EINVAL)?.try_into().map_err(|_| EINVAL)?);
                self.wait_or_not(open, |_| Ok(eventfd.add(value)?.then_some(())))?;
                Ok(8)
            }
            Node::Socket(s) => self.sock_send(open, s, data, None, None, 0),
            Node::Netlink(n) => {
                n.request(&self.k.net, data, self.process.pid as u32);
                Ok(data.len())
            }
            Node::Signalfd(_) | Node::Timerfd(_) | Node::Epoll(_) | Node::Pidfd(_) | Node::Mqueue(_) | Node::Inotify(_) | Node::Fanotify(_) | Node::Ns(_) | Node::FsContext(_) | Node::MountFd(_) | Node::Drm(_) => Err(EINVAL),
            Node::Stdin | Node::Dir(_) | Node::Listing(_) | Node::Pipe(_, false) | Node::Made(_) => Err(EBADF),
        }
    }

    pub(crate) fn read(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let data = self.read_from(&open, len, None)?;
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &data))?;
        Ok(data.len() as u64)
    }

    pub(crate) fn write(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let data = self.mem(cx, false, |t| user::read_vec(&t.space(), buf, len.min(MOST_AT_ONCE) as usize))?;
        Ok(self.write_to(&open, &data, None)? as u64)
    }

    pub(crate) fn pread(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64, at: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if !io::seekable(&open) {
            return Err(ESPIPE);
        }
        if (at as i64) < 0 {
            return Err(EINVAL);
        }
        let data = self.read_from(&open, len, Some(at))?;
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &data))?;
        Ok(data.len() as u64)
    }

    pub(crate) fn pwrite(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64, at: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if !io::seekable(&open) {
            return Err(ESPIPE);
        }
        if (at as i64) < 0 {
            return Err(EINVAL);
        }
        let data = self.mem(cx, false, |t| user::read_vec(&t.space(), buf, len.min(MOST_AT_ONCE) as usize))?;
        Ok(self.write_to(&open, &data, Some(at))? as u64)
    }

    pub(crate) fn iovecs(&mut self, cx: &mut Cx<'_>, at: u64, count: u64) -> Result<Vec<(u64, u64)>, Errno> {
        if count > 1024 {
            return Err(EINVAL);
        }
        self.mem(cx, false, |t| {
            let space = t.space();
            (0..count).map(|i| Ok((user::read_u64(&space, at + i * 16)?, user::read_u64(&space, at + i * 16 + 8)?))).collect::<Result<Vec<_>, user::Fault>>()
        })
    }

    pub(crate) fn readv(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, count: u64) -> Result<u64, Errno> {
        self.vread(cx, fd, at, count, None)
    }

    pub(crate) fn writev(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, count: u64) -> Result<u64, Errno> {
        self.vwrite(cx, fd, at, count, None)
    }

    /// Where `absolute` leads through the disk's symbolic links, if that's somewhere lambda makes rather than a file
    /// on the disk: `/dev/fd/63` (bash's process substitution) is `/proc/self/fd/63`, `/dev/stdin` is
    /// `/proc/self/fd/0`, Debian's `/etc/mtab` is `/proc/self/mounts`. None if it isn't. The disk alone can't follow
    /// them there, so a path the disk didn't find is looked at again this way. `follow_last` follows a link at the end.
    pub(crate) fn through_links(&self, absolute: &str, follow_last: bool) -> Option<String> {
        const MOST_LINKS: usize = 40;
        let view = self.view();
        let disk = lock(&self.k.disk);
        let words = |path: &str| path.split('/').filter(|w| !w.is_empty() && *w != ".").map(String::from).collect::<Vec<_>>();
        let mut rest: std::collections::VecDeque<String> = words(absolute).into();
        // The directories walked so far, by name and inode.
        let mut walked: Vec<(String, Ino)> = Vec::new();
        let mut links = 0;
        while let Some(word) = rest.pop_front() {
            let here = walked.last().map_or(view.root, |(_, ino)| *ino);
            if word == ".." {
                walked.pop();
                continue;
            }
            let at: String = walked.iter().map(|(name, _)| format!("/{name}")).collect::<String>() + "/" + &word;
            if os::stat(&self.k, &at).is_some() {
                let made = rest.iter().fold(at, |path, w| path + "/" + w);
                // sysfs follows its own links, and has the whole path or doesn't.
                if os::sysfs::is_sys(&made) {
                    return os::sysfs::lookup(&self.k, &made, follow_last).map(|(real, _)| real);
                }
                return Some(made);
            }
            // Past the disk: the system layer's, if it has the rest (`/lib/<triple>/` reaching its driver).
            let Ok(found) = disk.lookup(here, &word) else {
                let made = rest.iter().fold(at, |path, w| path + "/" + w);
                return os::system::lookup(self, &made).is_some().then_some(made);
            };
            let ino = view.mounts.cover(found);
            if disk.stat(ino).ok()?.kind == Kind::Symlink && (follow_last || !rest.is_empty()) {
                links += 1;
                if links > MOST_LINKS {
                    return None;
                }
                let target = disk.read_link(ino).ok()?;
                if target.starts_with('/') {
                    walked.clear();
                }
                for w in words(target).into_iter().rev() {
                    rest.push_front(w);
                }
                continue;
            }
            walked.push((word, ino));
        }
        None
    }

    /// A file lambda makes, opened: a terminal becomes a session leader's controlling one, as it's opened.
    fn opened_made(&self, open: Result<Arc<Open>, Errno>, flags: u32) -> Result<Arc<Open>, Errno> {
        let open = open?;
        if flags & O_DIRECTORY != 0 && !matches!(open.node, Node::Listing(_) | Node::Dir(_)) {
            return Err(ENOTDIR);
        }
        if let Node::Tty(tty) = &open.node
            && flags & O_NOCTTY == 0
        {
            self.adopt_terminal(tty);
        }
        Ok(open)
    }

    /// `openat2`: `openat` with a `struct open_how { flags, mode, resolve }` and the `RESOLVE_*` restrictions.
    pub(crate) fn openat2(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, how_at: u64, size: u64) -> Result<u64, Errno> {
        const OPEN_HOW_SIZE: u64 = 24;
        const RESOLVE_NO_XDEV: u64 = 0x01;
        const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
        const RESOLVE_NO_SYMLINKS: u64 = 0x04;
        const RESOLVE_BENEATH: u64 = 0x08;
        const RESOLVE_IN_ROOT: u64 = 0x10;
        const RESOLVE_CACHED: u64 = 0x20;
        const RESOLVE_KNOWN: u64 = RESOLVE_NO_XDEV | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH | RESOLVE_IN_ROOT | RESOLVE_CACHED;
        if size < OPEN_HOW_SIZE {
            return Err(EINVAL);
        }
        let (flags64, mode64, resolve) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), how_at)?, user::read_u64(&t.space(), how_at + 8)?, user::read_u64(&t.space(), how_at + 16)?)))?;
        // Anything past the struct this kernel knows must be zero.
        if size > OPEN_HOW_SIZE {
            let tail = self.mem(cx, false, |t| user::read_vec(&t.space(), how_at + OPEN_HOW_SIZE, (size - OPEN_HOW_SIZE).min(256) as usize))?;
            if tail.iter().any(|b| *b != 0) {
                return Err(E2BIG);
            }
        }
        // O_TMPFILE is O_DIRECTORY with this bit; openat2 refuses unknown flags and resolve bits, unlike openat.
        const O_TMPFILE_BIT: u64 = 0o20000000;
        const KNOWN_FLAGS: u64 = 0o7777777 | O_TMPFILE_BIT | O_PATH as u64;
        if flags64 & !KNOWN_FLAGS != 0 || resolve & !RESOLVE_KNOWN != 0 {
            return Err(EINVAL);
        }
        if flags64 & (O_CREAT as u64 | O_TMPFILE_BIT) == 0 && mode64 != 0 {
            return Err(EINVAL);
        }
        if resolve & RESOLVE_BENEATH != 0 && resolve & RESOLVE_IN_ROOT != 0 {
            return Err(EINVAL);
        }
        let (flags, mode) = (flags64 as u32, mode64 as u32);
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let how = mount::Resolve {
            no_xdev: resolve & RESOLVE_NO_XDEV != 0,
            no_symlinks: resolve & RESOLVE_NO_SYMLINKS != 0,
            beneath: resolve & RESOLVE_BENEATH != 0,
            in_root: resolve & RESOLVE_IN_ROOT != 0,
        };
        // The view whose root the open uses: the dirfd itself under RESOLVE_IN_ROOT.
        let base = self.view();
        let view = if how.in_root { mount::View { root: dir, mounts: base.mounts.clone() } } else { base };
        // Enforce the restrictions by resolving the target (or, to create it, its parent) under them.
        let follow = flags & O_NOFOLLOW == 0;
        let exists = view.resolve_how(&lock(&self.k.disk), dir, &path, follow, &how);
        match exists {
            Ok(_) => {}
            Err(ENOENT) if flags & O_CREAT != 0 => {
                let (parent, _) = view.resolve_parent(&lock(&self.k.disk), dir, &path)?;
                view.resolve_how(&lock(&self.k.disk), parent, ".", true, &how)?;
            }
            Err(e) => return Err(e),
        }
        // openat2 here never traverses a disk symlink into a /proc magic link, so RESOLVE_NO_MAGICLINKS needs nothing
        // more; a /proc file opened directly is an ordinary open.
        let absolute = if how.in_root { path.clone() } else { exe_path(&dir_path, &path) };
        let made = if how.in_root { None } else { os::open(self, &absolute) };
        let open = match made {
            Some(open) => self.opened_made(open, flags)?,
            None => self.open_on_disk(&view, dir, &path, &absolute, flags, mode)?,
        };
        if let Node::File(ino) | Node::Dir(ino) = open.node
            && flags & O_PATH == 0
        {
            self.k.file_event(ino, inotify::IN_OPEN, &open.path);
        }
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    pub(crate) fn openat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u32, mode: u32) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let absolute = exe_path(&dir_path, &path);
        let open = match os::open(self, &absolute) {
            Some(open) => self.opened_made(open, flags)?,
            None => match self.open_on_disk(&self.view(), dir, &path, &absolute, flags, mode) {
                Err(e) if not_on_disk(e) => match self.through_links(&absolute, flags & O_NOFOLLOW == 0).and_then(|made| os::open(self, &made)) {
                    Some(open) => self.opened_made(open, flags)?,
                    None => return Err(e),
                },
                open => open?,
            },
        };
        if let Node::File(ino) | Node::Dir(ino) = open.node
            && flags & O_PATH == 0
        {
            self.k.file_event(ino, inotify::IN_OPEN, &open.path);
        }
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn open_on_disk(&self, view: &mount::View, dir: Ino, path: &str, absolute: &str, flags: u32, mode: u32) -> Result<Arc<Open>, Errno> {
        if flags & O_CREAT != 0 {
            os::system::copy_up(self, absolute);
        }
        let umask = lock(&self.process.state).umask;
        // On a read-only mount: nothing opened for writing, truncated or made.
        let read_only = self.writable(absolute).is_err();
        if read_only && (flags & O_ACCMODE != 0 || flags & O_TRUNC != 0) && flags & O_PATH == 0 {
            return Err(EROFS);
        }
        let mut disk = lock(&self.k.disk);
        let (ino, created) = match view.resolve(&disk, dir, path, flags & O_NOFOLLOW == 0) {
            Ok(_) if flags & O_CREAT != 0 && flags & O_EXCL != 0 => return Err(EEXIST),
            Ok(ino) => (ino, false),
            Err(layers::Error::NotFound) if flags & O_CREAT != 0 => {
                if read_only {
                    return Err(EROFS);
                }
                let (parent, name) = view.resolve_parent(&disk, dir, path)?;
                (disk.create(parent, name, New::File, Meta::file(mode & !umask & 0o7777), self.k.now())?, true)
            }
            Err(e) => return Err(e.into()),
        };
        let stat = disk.stat(ino)?;
        let meta = *disk.meta(ino)?;
        let write = flags & O_ACCMODE != 0;
        let node = match stat.kind {
            Kind::Dir if write => return Err(EISDIR),
            Kind::Dir => Node::Dir(ino),
            _ if flags & O_DIRECTORY != 0 => return Err(ENOTDIR),
            // An `O_PATH` handle names the link itself, as glibc's `fchmodat(AT_SYMLINK_NOFOLLOW)` opens it to look.
            Kind::Symlink if flags & O_PATH != 0 => Node::File(ino),
            Kind::Symlink => return Err(ELOOP),
            // A Unix socket's file is found by `connect`, not opened.
            Kind::File if meta.mode & S_IFMT == S_IFSOCK => return Err(ENXIO),
            // A named pipe: readers and writers meet on a shared buffer, after the disk lock is let go.
            Kind::File if meta.mode & S_IFMT == S_IFIFO => {
                drop(disk);
                return self.open_fifo(ino, flags, absolute.to_string());
            }
            // No block devices.
            Kind::File if meta.mode & S_IFMT == S_IFBLK => return Err(ENXIO),
            Kind::File if meta.mode & S_IFMT == S_IFCHR => match meta.rdev {
                0x103 => Node::Null,
                0x105 => Node::Zero,
                0x108 | 0x109 => Node::Random,
                os::gpu::RDEV => Node::Drm(os::gpu::Drm::new(self.k.options.gpu.clone().ok_or(ENXIO)?)),
                // /dev/tty: the process's controlling terminal.
                0x500 => match lock(&self.process.state).ctty.clone() {
                    Some(tty) => {
                        if tty.index.is_some() {
                            tty.open_slave()?;
                        }
                        Node::Tty(tty)
                    }
                    None => return Err(ENXIO),
                },
                0x501 => Node::Tty(self.k.console.clone().ok_or(ENXIO)?),
                0x502 => {
                    let n = self.k.next_pty.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let tty = Tty::pty(n);
                    lock(&self.k.ptys).insert(n, Arc::downgrade(&tty));
                    Node::PtyMaster(tty)
                }
                _ => return Err(ENXIO),
            },
            Kind::File => {
                // A file just made is empty already: not truncated, so not modified, as on Linux.
                if flags & O_TRUNC != 0 && write && !created {
                    disk.truncate(ino, 0, self.k.now())?;
                }
                Node::File(ino)
            }
        };
        // Opening a terminal makes it a session leader's controlling terminal, if it has none.
        if let Node::Tty(tty) = &node
            && flags & O_NOCTTY == 0
        {
            self.adopt_terminal(tty);
        }
        // A directory too, so its close is told; the disk never keeps a removed one for it. A device's file too, so
        // `fstat` is the file's own stat, as `stat` of its path is (`cp` and `install` compare the two).
        let hold = match node {
            Node::File(ino) | Node::Dir(ino) => {
                disk.hold(ino);
                Some(Hold { k: Arc::downgrade(&self.k), ino, quiet: false })
            }
            Node::Null | Node::Zero | Node::Random | Node::Drm(_) => {
                disk.hold(ino);
                Some(Hold { k: Arc::downgrade(&self.k), ino, quiet: false })
            }
            _ => None,
        };
        Ok(Arc::new(Open { node, flags: Mutex::new(flags & !O_CLOEXEC), offset: Mutex::new(0), path: absolute.to_string(), _hold: hold }))
    }

    pub(crate) fn adopt_terminal(&self, tty: &Arc<Tty>) {
        let mut state = lock(&self.process.state);
        if state.sid == self.process.pid && state.ctty.is_none() && tty.session().is_none() {
            tty.set_session(Some(state.sid), state.pgid);
            state.ctty = Some(tty.clone());
        }
    }

    pub(crate) fn close(&mut self, fd: i32) -> Result<u64, Errno> {
        let open = lock(&self.process.state).files.remove(fd).ok_or(EBADF)?;
        drop(open);
        Ok(0)
    }

    /// `close_range`: every descriptor from `first` to `last`, closed, or with `CLOSE_RANGE_CLOEXEC` marked to close at
    /// `execve` (what `posix_spawn`'s child does before it runs the program).
    pub(crate) fn close_range(&mut self, first: u64, last: u64, flags: u64) -> Result<u64, Errno> {
        const CLOSE_RANGE_UNSHARE: u64 = 2;
        const CLOSE_RANGE_CLOEXEC: u64 = 4;
        if flags & !(CLOSE_RANGE_UNSHARE | CLOSE_RANGE_CLOEXEC) != 0 || first > last {
            return Err(EINVAL);
        }
        let (first, last) = (first.min(i32::MAX as u64) as i32, last.min(i32::MAX as u64) as i32);
        let closed: Vec<Arc<Open>> = {
            let mut state = lock(&self.process.state);
            let fds = &mut state.files.fds;
            if flags & CLOSE_RANGE_CLOEXEC != 0 {
                for (_, fd) in fds.range_mut(first..=last) {
                    fd.cloexec = true;
                }
                Vec::new()
            } else {
                let gone: Vec<i32> = fds.range(first..=last).map(|(fd, _)| *fd).collect();
                gone.into_iter().filter_map(|fd| fds.remove(&fd)).map(|f| f.open).collect()
            }
        };
        // Closed with the process's lock let go: a file's last close may want it.
        drop(closed);
        Ok(0)
    }

    pub(crate) fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let size = match &open.node {
            Node::File(ino) => lock(&self.k.disk).stat(*ino)?.size,
            // A directory's offset is an entry's place in it (`rewinddir` seeks to 0), a made one's too.
            Node::Dir(_) | Node::Listing(_) | Node::Null | Node::Zero => 0,
            Node::Made(bytes) => bytes.len() as u64,
            Node::Memfd(mem, _) => mem.size(),
            _ => return Err(ESPIPE),
        };
        let mut at = lock(&open.offset);
        let base = match whence {
            0 => 0,
            1 => *at as i64,
            2 => size as i64,
            _ => return Err(EINVAL),
        };
        let new = base.checked_add(offset).filter(|n| *n >= 0).ok_or(EINVAL)?;
        *at = new as u64;
        Ok(new as u64)
    }

    pub(crate) fn fstat(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let stat = stat_of_open(&lock(&self.k.disk), &open)?;
        let bytes = stat.encode(self.isa);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
        Ok(0)
    }

    /// The `stat` for a path, or for `dirfd` itself with `AT_EMPTY_PATH`.
    fn stat_path(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<Stat, Errno> {
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            if flags & AT_EMPTY_PATH == 0 {
                return Err(ENOENT);
            }
            let open = self.file(dirfd)?;
            return stat_of_open(&lock(&self.k.disk), &open);
        }
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let follow = flags & AT_SYMLINK_NOFOLLOW == 0;
        let absolute = exe_path(&dir_path, &path);
        if let Some(stat) = self.made_stat(&absolute, follow) {
            return stat;
        }
        let view = self.view();
        let found = view.resolve(&lock(&self.k.disk), dir, &path, follow).map_err(Errno::from);
        match found {
            Ok(ino) => stat_of(&lock(&self.k.disk), ino),
            Err(e) if not_on_disk(e) => self.through_links(&absolute, follow).and_then(|made| self.made_stat(&made, follow)).unwrap_or(Err(e)),
            Err(e) => Err(e),
        }
    }

    /// `stat` for a path lambda makes, or None if it isn't one. Followed, `/proc/<pid>/fd/<n>` is the file it's open
    /// on (so two pipes are two files), and ENOENT if nothing is.
    fn made_stat(&self, path: &str, follow: bool) -> Option<Result<Stat, Errno>> {
        if let Some(stat) = os::system::stat(self, path) {
            return Some(Ok(stat));
        }
        if os::sysfs::is_sys(path) {
            return Some(match os::sysfs::lookup(&self.k, path, follow) {
                Some((real, _)) => os::stat(&self.k, &real).ok_or(ENOENT),
                None => Err(ENOENT),
            });
        }
        if (follow && path.starts_with("/proc/") && path.contains("/fd/")) || os::is_listing(path) {
            return os::open(self, path).map(|open| open.and_then(|open| stat_of_open(&lock(&self.k.disk), &open)));
        }
        os::stat(&self.k, path).map(Ok)
    }

    pub(crate) fn newfstatat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, at: u64, flags: u64) -> Result<u64, Errno> {
        let bytes = self.stat_path(cx, dirfd, path_at, flags)?.encode(self.isa);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
        Ok(0)
    }

    pub(crate) fn statx(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64, at: u64) -> Result<u64, Errno> {
        let bytes = self.stat_path(cx, dirfd, path_at, flags)?.encode_statx();
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
        Ok(0)
    }

    /// `getdents64`, or with `old` x86-64's `getdents`, whose `linux_dirent` has the type in its last byte.
    pub(crate) fn getdents(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, size: u64, old: bool) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if let Node::Listing(made) = &open.node {
            let id = |name: &str| os::made_identity(&format!("{}/{name}", open.path)).1;
            let mut entries: Vec<(String, Ino, u8)> = vec![(".".into(), id("."), DT_DIR), ("..".into(), id(".."), DT_DIR)];
            entries.extend(made.iter().map(|(name, kind)| (name.clone(), id(name), *kind)));
            return self.write_dirents(cx, &open, entries, buf, size, old);
        }
        let Node::Dir(ino) = open.node else { return Err(ENOTDIR) };
        if open.notifies() {
            self.k.file_event(ino, inotify::IN_ACCESS, &open.path);
        }
        let mut entries: Vec<(String, Ino, u8)> = Vec::new();
        {
            let disk = lock(&self.k.disk);
            entries.push((".".into(), ino, DT_DIR));
            entries.push(("..".into(), disk.parent(ino)?, DT_DIR));
            for (name, child, kind) in disk.entries(ino)? {
                let device = disk.meta(child).is_ok_and(|m| m.mode & S_IFMT == S_IFCHR);
                let kind = match kind {
                    Kind::Dir => DT_DIR,
                    Kind::Symlink => DT_LNK,
                    Kind::File if device => DT_CHR,
                    Kind::File => DT_REG,
                };
                entries.push((name, child, kind));
            }
        }
        // The system layer's files in it, after the disk's.
        for (name, kind) in os::system::listing(self, &open.path) {
            if !entries.iter().any(|(n, _, _)| *n == name) {
                let id = os::made_identity(&format!("{}/{name}", open.path)).1;
                entries.push((name, id, kind));
            }
        }
        self.write_dirents(cx, &open, entries, buf, size, old)
    }

    /// Writes as many of a directory's `entries` as fit in `size` bytes at `buf`, from where its last read left off, as
    /// `linux_dirent64` (or for `old`, `linux_dirent`).
    fn write_dirents(&mut self, cx: &mut Cx<'_>, open: &Open, entries: Vec<(String, Ino, u8)>, buf: u64, size: u64, old: bool) -> Result<u64, Errno> {
        let mut offset = lock(&open.offset);
        let mut out = Vec::new();
        let mut index = *offset as usize;
        while let Some((name, child, kind)) = entries.get(index) {
            // ino, offset and length; then the type and the name (or for `old`, the name, and the type last).
            let length = if old { (18 + name.len() + 2).next_multiple_of(8) } else { (19 + name.len() + 1).next_multiple_of(8) };
            if out.len() + length > size as usize {
                break;
            }
            let start = out.len();
            out.extend(child.to_le_bytes());
            out.extend(((index + 1) as i64).to_le_bytes());
            out.extend((length as u16).to_le_bytes());
            if !old {
                out.push(*kind);
            }
            out.extend(name.as_bytes());
            out.resize(start + length, 0);
            if old {
                out[start + length - 1] = *kind;
            }
            index += 1;
        }
        if out.is_empty() && index < entries.len() {
            return Err(EINVAL);
        }
        *offset = index as u64;
        drop(offset);
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &out))?;
        Ok(out.len() as u64)
    }

    pub(crate) fn ioctl(&mut self, cx: &mut Cx<'_>, fd: i32, request: u64, arg: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if request == FIONBIO {
            let on = self.mem(cx, false, |t| user::read_u32(&t.space(), arg))? != 0;
            let mut flags = lock(&open.flags);
            *flags = if on { *flags | O_NONBLOCK } else { *flags & !O_NONBLOCK };
            return Ok(0);
        }
        const FIONREAD: u64 = 0x541b;
        match &open.node {
            Node::Tty(tty) => self.tty_ioctl(cx, tty, false, request, arg),
            Node::PtyMaster(tty) => self.tty_ioctl(cx, tty, true, request, arg),
            Node::Pipe(pipe, false) | Node::Fifo(pipe, _) if request == FIONREAD => {
                let n = pipe.available() as u32;
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, n))?;
                Ok(0)
            }
            Node::Socket(_) | Node::Netlink(_) => self.socket_ioctl(cx, &open, request, arg),
            Node::Drm(drm) => self.drm_ioctl(cx, drm, request, arg),
            Node::Inotify(inotify) if request == FIONREAD => {
                let n = inotify.available() as u32;
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, n))?;
                Ok(0)
            }
            Node::Fanotify(group) if request == FIONREAD => {
                let n = group.available() as u32;
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, n))?;
                Ok(0)
            }
            _ => Err(ENOTTY),
        }
    }

    fn tty_ioctl(&mut self, cx: &mut Cx<'_>, tty: &Arc<Tty>, master: bool, request: u64, arg: u64) -> Result<u64, Errno> {
        const TCGETS: u64 = 0x5401;
        const TCSETS: u64 = 0x5402;
        const TCSETSW: u64 = 0x5403;
        const TCSETSF: u64 = 0x5404;
        const TCSBRK: u64 = 0x5409;
        const TCXONC: u64 = 0x540a;
        const TCFLSH: u64 = 0x540b;
        const TIOCSCTTY: u64 = 0x540e;
        const TIOCGPGRP: u64 = 0x540f;
        const TIOCSPGRP: u64 = 0x5410;
        const TIOCOUTQ: u64 = 0x5411;
        const TIOCGWINSZ: u64 = 0x5413;
        const TIOCSWINSZ: u64 = 0x5414;
        const FIONREAD: u64 = 0x541b;
        const TIOCNOTTY: u64 = 0x5422;
        const TIOCGSID: u64 = 0x5429;
        const TIOCGPTN: u64 = 0x8004_5430;
        const TIOCSPTLCK: u64 = 0x4004_5431;
        const TIOCGPTLCK: u64 = 0x8004_5439;
        const TIOCGPTPEER: u64 = 0x5441;
        let write_u32 = |task: &mut Task, cx: &mut Cx<'_>, v: u32| task.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, v));
        match request {
            TCGETS => {
                let bytes = tty.termios().encode();
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &bytes))?;
                Ok(0)
            }
            TCSETS | TCSETSW | TCSETSF => {
                let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, 36))?;
                tty.set_termios(Termios::decode(&bytes), request == TCSETSF);
                Ok(0)
            }
            TIOCGWINSZ => {
                let size = tty.winsize();
                let bytes: Vec<u8> = size.iter().flat_map(|v| v.to_le_bytes()).collect();
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &bytes))?;
                Ok(0)
            }
            TIOCSWINSZ => {
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, 8))?;
                let size = [0, 1, 2, 3].map(|i| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]));
                if let Some((pgid, signal)) = tty.set_winsize(size) {
                    self.k.signal_group(pgid, signal);
                }
                Ok(0)
            }
            TIOCGPGRP => {
                if tty.session().is_none() {
                    return Err(ENOTTY);
                }
                write_u32(self, cx, tty.foreground() as u32)?;
                Ok(0)
            }
            TIOCSPGRP => {
                let pgid = self.mem(cx, false, |t| user::read_u32(&t.space(), arg))? as i32;
                let (sid, mine) = {
                    let state = lock(&self.process.state);
                    (state.sid, state.ctty.as_ref().is_some_and(|c| Arc::ptr_eq(c, tty)))
                };
                if !mine && !master {
                    return Err(ENOTTY);
                }
                let exists = lock(&self.k.processes).values().any(|p| {
                    let s = lock(&p.state);
                    s.pgid == pgid && s.sid == sid
                });
                if !exists {
                    return Err(EPERM);
                }
                tty.set_foreground(pgid);
                Ok(0)
            }
            TIOCGSID => {
                write_u32(self, cx, tty.session().ok_or(ENOTTY)? as u32)?;
                Ok(0)
            }
            TIOCSCTTY => {
                let state = lock(&self.process.state);
                // Already the session leader's controlling terminal (opening it without O_NOCTTY made it so, as it
                // does after setsid): nothing to do, as Linux's tiocsctty has it.
                if state.sid == self.process.pid && state.ctty.as_ref().is_some_and(|c| Arc::ptr_eq(c, tty)) {
                    return Ok(0);
                }
                if state.sid != self.process.pid || state.ctty.is_some() {
                    return Err(EPERM);
                }
                drop(state);
                if tty.session().is_some() && arg == 0 {
                    return Err(EPERM);
                }
                tty.set_session(None, 0);
                self.adopt_terminal(tty);
                Ok(0)
            }
            TIOCNOTTY => {
                let mut state = lock(&self.process.state);
                if state.ctty.as_ref().is_some_and(|c| Arc::ptr_eq(c, tty)) {
                    state.ctty = None;
                    if state.sid == self.process.pid {
                        tty.set_session(None, 0);
                    }
                }
                Ok(0)
            }
            FIONREAD => {
                write_u32(self, cx, tty.available() as u32)?;
                Ok(0)
            }
            TIOCOUTQ => {
                write_u32(self, cx, 0)?;
                Ok(0)
            }
            TCFLSH => {
                tty.flush(arg);
                Ok(0)
            }
            TCSBRK | TCXONC => Ok(0),
            TIOCGPTN if master => {
                write_u32(self, cx, tty.index.unwrap_or(0))?;
                Ok(0)
            }
            TIOCSPTLCK if master => {
                let locked = self.mem(cx, false, |t| user::read_u32(&t.space(), arg))? != 0;
                tty.set_locked(locked);
                Ok(0)
            }
            TIOCGPTLCK if master => {
                write_u32(self, cx, u32::from(tty.locked()))?;
                Ok(0)
            }
            TIOCGPTPEER if master => {
                tty.open_slave()?;
                let path = format!("/dev/pts/{}", tty.index.unwrap_or(0));
                let open = Open::new(Node::Tty(tty.clone()), (arg as u32 & !O_CLOEXEC) | O_RDWR, path);
                lock(&self.process.state).files.add(open, arg as u32 & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
            }
            _ => Err(ENOTTY),
        }
    }

    pub(crate) fn fcntl(&mut self, fd: i32, command: u64, arg: u64) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let entry = state.files.fds.get_mut(&fd).ok_or(EBADF)?;
        match command {
            F_DUPFD | F_DUPFD_CLOEXEC => {
                if arg >= state.files.limit as u64 {
                    return Err(EINVAL);
                }
                let entry = state.files.fds.get(&fd).ok_or(EBADF)?;
                let open = entry.open.clone();
                state.files.add(open, command == F_DUPFD_CLOEXEC, arg as i32).map(|fd| fd as u64)
            }
            F_GETFD => Ok(if entry.cloexec { FD_CLOEXEC } else { 0 }),
            F_SETFD => {
                entry.cloexec = arg & FD_CLOEXEC != 0;
                Ok(0)
            }
            F_GETFL => Ok(u64::from(open_flags_to(self.isa, *lock(&entry.open.flags)))),
            F_SETFL => {
                let wanted = open_flags_from(self.isa, arg as u32) & (O_APPEND | O_NONBLOCK);
                let mut flags = lock(&entry.open.flags);
                *flags = *flags & !(O_APPEND | O_NONBLOCK) | wanted;
                Ok(0)
            }
            // Locks: there's no one else to lock against yet.
            5..=7 | 36..=38 => Ok(0),
            // F_ADD_SEALS and F_GET_SEALS: a memfd's.
            1033 | 1034 => match &entry.open.node {
                Node::Memfd(mem, false) if command == 1033 => mem.add_seals(arg as u32).map(|()| 0),
                Node::Memfd(mem, false) => Ok(u64::from(mem.seals())),
                _ => Err(EINVAL),
            },
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn dup(&mut self, fd: i32) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let open = state.files.get(fd).ok_or(EBADF)?;
        state.files.add(open, false, 0).map(|fd| fd as u64)
    }

    pub(crate) fn dup3(&mut self, old: i32, new: i32, flags: u32, dup2: bool) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let open = state.files.get(old).ok_or(EBADF)?;
        if old == new {
            return if dup2 { Ok(new as u64) } else { Err(EINVAL) };
        }
        if !(0..state.files.limit).contains(&new) {
            return Err(EBADF);
        }
        let replaced = state.files.fds.insert(new, Fd { open, cloexec: flags & O_CLOEXEC != 0 });
        drop(state);
        drop(replaced);
        Ok(new as u64)
    }

    pub(crate) fn pipe2(&mut self, cx: &mut Cx<'_>, at: u64, flags: u32) -> Result<u64, Errno> {
        let pipe = Pipe::new();
        let keep = flags & O_NONBLOCK;
        let read = Open::new(Node::Pipe(pipe.clone(), false), keep, "pipe:");
        let write = Open::new(Node::Pipe(pipe, true), O_WRONLY | keep, "pipe:");
        let cloexec = flags & O_CLOEXEC != 0;
        let (r, w) = {
            let mut state = lock(&self.process.state);
            let r = state.files.add(read, cloexec, 0)?;
            match state.files.add(write, cloexec, 0) {
                Ok(w) => (r, w),
                Err(e) => {
                    state.files.remove(r);
                    return Err(e);
                }
            }
        };
        let mut bytes = (r as u32).to_le_bytes().to_vec();
        bytes.extend((w as u32).to_le_bytes());
        if let Err(e) = self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes)) {
            let mut state = lock(&self.process.state);
            state.files.remove(r);
            state.files.remove(w);
            return Err(e);
        }
        Ok(0)
    }

    pub(crate) fn faccessat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let absolute = exe_path(&dir_path, &path);
        if os::stat(&self.k, &absolute).is_some() || os::system::stat(self, &absolute).is_some() {
            return Ok(0);
        }
        let view = self.view();
        let found = view.resolve(&lock(&self.k.disk), dir, &path, true).map_err(Errno::from);
        match found {
            Ok(_) => Ok(0),
            Err(e) if not_on_disk(e) && self.through_links(&absolute, true).is_some_and(|made| os::stat(&self.k, &made).is_some() || os::system::stat(self, &made).is_some()) => Ok(0),
            Err(e) => Err(e),
        }
    }

    pub(crate) fn readlinkat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, buf: u64, size: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let absolute = exe_path(&dir_path, &path);
        let target = match os::read_link(self, &absolute) {
            Some(target) => target?,
            None => {
                let view = self.view();
                let found = view.resolve(&lock(&self.k.disk), dir, &path, false).map_err(Errno::from);
                match found {
                    Ok(ino) => lock(&self.k.disk).read_link(ino).map_err(|_| EINVAL)?.to_string(),
                    Err(e) if not_on_disk(e) => match self.through_links(&absolute, false).and_then(|made| os::read_link(self, &made)) {
                        Some(target) => target?,
                        None => return Err(e),
                    },
                    Err(e) => return Err(e),
                }
            }
        };
        let bytes = &target.as_bytes()[..target.len().min(size as usize)];
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, bytes))?;
        Ok(bytes.len() as u64)
    }

    pub(crate) fn mkdirat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, mode: u32) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let umask = lock(&self.process.state).umask;
        let view = self.view();
        let read_only = self.writable(&exe_path(&dir_path, &path));
        os::system::copy_up(self, &exe_path(&dir_path, &path));
        let mut disk = lock(&self.k.disk);
        // `/`, `.` and `..` are always there, as `tar` and `dpkg-deb` find making the directory they extract into.
        if path.trim_end_matches('/').is_empty() {
            return Err(EEXIST);
        }
        let (parent, name) = view.resolve_parent(&disk, dir, &path)?;
        if disk.lookup(parent, name).is_ok() && !matches!(name, "." | "..") {
            return Err(EEXIST);
        }
        read_only?;
        if matches!(name, "" | "." | "..") {
            return Err(EEXIST);
        }
        disk.create(parent, name, New::Dir, Meta::dir(mode & !umask & 0o7777), self.k.now())?;
        Ok(0)
    }

    /// Opens a FIFO: its buffer is shared by inode, and (unless non-blocking) a reader waits for a writer and the
    /// other way. The disk lock must be let go first, as this waits.
    fn open_fifo(&self, ino: Ino, flags: u32, path: String) -> Result<Arc<Open>, Errno> {
        let pipe = {
            let mut fifos = lock(&self.k.fifos);
            match fifos.get(&ino).and_then(Weak::upgrade) {
                Some(pipe) => pipe,
                None => {
                    let pipe = Pipe::fifo(ino);
                    fifos.insert(ino, Arc::downgrade(&pipe));
                    pipe
                }
            }
        };
        let acc = flags & O_ACCMODE;
        let (read, write) = (acc != O_WRONLY, acc != 0);
        if read {
            pipe.open_end(false);
        }
        if write {
            pipe.open_end(true);
        }
        let rendezvous = if acc == O_RDWR { Ok(()) } else { pipe.wait_open(write, &self.waiter, flags & O_NONBLOCK != 0) };
        if let Err(e) = rendezvous {
            if read {
                pipe.close(false);
            }
            if write {
                pipe.close(true);
            }
            return Err(e);
        }
        Ok(Arc::new(Open { node: Node::Fifo(pipe, ino), flags: Mutex::new(flags & !O_CLOEXEC), offset: Mutex::new(0), path, _hold: None }))
    }

    /// `mknodat`: a regular file, a FIFO, a device node (needs `CAP_MKNOD`), or a socket file.
    pub(crate) fn mknodat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, mode: u32, dev: u64) -> Result<u64, Errno> {
        let kind = mode & S_IFMT;
        if !matches!(kind, 0 | S_IFREG | S_IFIFO | S_IFCHR | S_IFBLK | S_IFSOCK) {
            return Err(EINVAL);
        }
        if matches!(kind, S_IFCHR | S_IFBLK) && !lock(&self.process.state).creds.capable(crate::process::creds::CAP_MKNOD) {
            return Err(EPERM);
        }
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        self.writable(&crate::process::exe_path(&dir_path, &path))?;
        os::system::copy_up(self, &crate::process::exe_path(&dir_path, &path));
        let umask = lock(&self.process.state).umask;
        let view = self.view();
        let mut disk = lock(&self.k.disk);
        let (parent, name) = view.resolve_parent(&disk, dir, &path)?;
        if disk.lookup(parent, name).is_ok() {
            return Err(EEXIST);
        }
        let perm = mode & !umask & 0o7777;
        // A regular file keeps only its permission bits; the rest keep their type (and a device its numbers).
        let meta = match kind {
            0 | S_IFREG => Meta::file(perm),
            S_IFCHR | S_IFBLK => Meta { mode: kind | perm, rdev: dev, ..Meta::default() },
            _ => Meta { mode: kind | perm, ..Meta::default() },
        };
        disk.create(parent, name, New::File, meta, self.k.now())?;
        Ok(0)
    }

    pub(crate) fn unlinkat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let view = self.view();
        let read_only = self.writable(&exe_path(&dir_path, &path));
        let mut disk = lock(&self.k.disk);
        let (parent, name) = view.resolve_parent(&disk, dir, &path)?;
        let child = disk.lookup(parent, name)?;
        // A mount point is busy.
        if view.mounts.cover(child) != child {
            return Err(EBUSY);
        }
        read_only?;
        let is_dir = disk.stat(disk.lookup(parent, name)?)?.kind == Kind::Dir;
        match (flags & AT_REMOVEDIR != 0, is_dir) {
            (true, false) => return Err(ENOTDIR),
            (false, true) => return Err(EISDIR),
            _ => {}
        }
        disk.remove(parent, name, self.k.now())?;
        Ok(0)
    }

    pub(crate) fn renameat(&mut self, cx: &mut Cx<'_>, olddirfd: i32, old_at: u64, newdirfd: i32, new_at: u64) -> Result<u64, Errno> {
        let (old, new) = (self.read_path(cx, old_at)?, self.read_path(cx, new_at)?);
        let ((from_dir, from_path), (to_dir, to_path)) = (self.start_dir(olddirfd)?, self.start_dir(newdirfd)?);
        let (old_abs, new_abs) = (exe_path(&from_path, &old), exe_path(&to_path, &new));
        os::system::copy_up(self, &new_abs);
        self.writable(&old_abs)?;
        self.writable(&new_abs)?;
        // Not from one mount to another, even of the same file system, as on Linux.
        let view = self.view();
        if view.mounts.containing(&self.ns_path(&old_abs)).id != view.mounts.containing(&self.ns_path(&new_abs)).id {
            return Err(EXDEV);
        }
        let mut disk = lock(&self.k.disk);
        let (from, from_name) = view.resolve_parent(&disk, from_dir, &old)?;
        let (to, to_name) = view.resolve_parent(&disk, to_dir, &new)?;
        disk.rename(from, from_name, to, to_name, self.k.now())?;
        Ok(0)
    }

    pub(crate) fn symlinkat(&mut self, cx: &mut Cx<'_>, target_at: u64, dirfd: i32, path_at: u64) -> Result<u64, Errno> {
        let (target, path) = (self.read_path(cx, target_at)?, self.read_path(cx, path_at)?);
        let (dir, dir_path) = self.start_dir(dirfd)?;
        self.writable(&exe_path(&dir_path, &path))?;
        os::system::copy_up(self, &exe_path(&dir_path, &path));
        let view = self.view();
        let mut disk = lock(&self.k.disk);
        let (parent, name) = view.resolve_parent(&disk, dir, &path)?;
        disk.create(parent, name, New::Symlink(target), Meta::file(0o777), self.k.now())?;
        Ok(0)
    }

    /// `statfs`: every path is on the one disk, which says it's an ext4 of the machine's memory.
    /// `statfs` (a path) and `fstatfs` (a descriptor): the machine's memory, as the mount the file is on says, with
    /// its type and flags.
    pub(crate) fn statfs(&mut self, cx: &mut Cx<'_>, path_at: Option<u64>, fd: i32, at: u64) -> Result<u64, Errno> {
        let visible = match path_at {
            Some(path_at) => {
                let path = self.read_path(cx, path_at)?;
                let (dir, dir_path) = self.start_dir(AT_FDCWD)?;
                let absolute = exe_path(&dir_path, &path);
                if os::stat(&self.k, &absolute).is_none() {
                    let view = self.view();
                    view.resolve(&lock(&self.k.disk), dir, &path, true)?;
                }
                absolute
            }
            None => self.file(fd)?.path.clone(),
        };
        let (what, flags, tmpfs) = self.mount_of(&visible);
        // MS_RDONLY, NOSUID, NODEV, NOEXEC, NOATIME and NODIRATIME are statfs's ST_ bits too; MS_RELATIME is ST_RELATIME.
        let st_flags = flags & 0xf | flags & 0xc00 | if flags & (1 << 21) != 0 { 0x1000 } else { 0 };
        let (mut total, mut free) = {
            let mut exec = lock(&self.k.exec);
            let size = exec.memory().size();
            (size.limit / 4096, size.available / 4096)
        };
        // A tmpfs: its size (or the machine's memory), less what its files hold.
        if let Some((tree, size)) = tmpfs {
            total = size.map_or(total, |s| (s / 4096).min(total));
            let used = lock(&self.k.disk).tree_size(tree).div_ceil(4096);
            free = total.saturating_sub(used).min(free);
        }
        let mut b = Vec::with_capacity(120);
        for v in [what.magic(), 4096, total, free, free, 1 << 20, 1 << 20, 0, 255, 4096, st_flags] {
            b.extend(v.to_le_bytes());
        }
        b.resize(120, 0);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }

    pub(crate) fn ftruncate(&mut self, fd: i32, len: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if let Node::Memfd(mem, _) = &open.node {
            if (len as i64) < 0 {
                return Err(EINVAL);
            }
            return mem.truncate(len).map(|()| 0);
        }
        let Node::File(ino) = open.node else { return Err(EINVAL) };
        if !open.writable() || (len as i64) < 0 {
            return Err(EINVAL);
        }
        lock(&self.k.disk).truncate(ino, len, self.k.now())?;
        Ok(0)
    }

    pub(crate) fn getcwd(&mut self, cx: &mut Cx<'_>, buf: u64, size: u64) -> Result<u64, Errno> {
        let mut path = lock(&self.process.state).cwd.1.clone().into_bytes();
        path.push(0);
        if path.len() as u64 > size {
            return Err(ERANGE);
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &path))?;
        Ok(path.len() as u64)
    }

    pub(crate) fn chdir(&mut self, cx: &mut Cx<'_>, path_at: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let view = self.view();
        let mut state = lock(&self.process.state);
        let disk = lock(&self.k.disk);
        let ino = view.resolve(&disk, state.cwd.0, &path, true)?;
        if disk.stat(ino)?.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        // Its real path, links followed, as `getcwd` gives it.
        let absolute = view.real_path(&disk, &state.cwd.1, &path).unwrap_or_else(|_| exe_path(&state.cwd.1, &path));
        state.cwd = (ino, absolute);
        Ok(0)
    }

    pub(crate) fn fchdir(&mut self, fd: i32) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let Node::Dir(ino) = open.node else { return Err(ENOTDIR) };
        lock(&self.process.state).cwd = (ino, open.path.clone());
        Ok(0)
    }
}
