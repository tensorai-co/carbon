//! Files: each process's table of open files, and the system calls on files and paths. Regular files and
//! directories live on the machine's disk (`layers`); devices are what their numbers say; `/proc` is made as it's read.

mod epoll;
mod events;
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

/// The most files a process can have open.
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
    Null,
    Zero,
    Random,
    /// A file made when it was opened, such as `/proc/self/maps`.
    Made(Vec<u8>),
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
}

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(k) = self.k.upgrade() {
            lock(&k.disk).release(self.ino);
        }
    }
}

impl Open {
    pub fn new(node: Node, flags: u32, path: impl Into<String>) -> Arc<Open> {
        Arc::new(Open { node, flags: Mutex::new(flags), offset: Mutex::new(0), path: path.into(), _hold: None })
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
        match &self.node {
            Node::Pipe(pipe, write_end) => pipe.close(*write_end),
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
#[derive(Clone, Default)]
pub(crate) struct Files {
    fds: BTreeMap<i32, Fd>,
}

impl Files {
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
        let fd = (lowest.max(0)..MOST_FILES).find(|fd| !self.fds.contains_key(fd)).ok_or(EMFILE)?;
        self.fds.insert(fd, Fd { open, cloexec });
        Ok(fd)
    }

    /// Takes a descriptor out; its file closes when nothing else has it.
    fn remove(&mut self, fd: i32) -> Option<Arc<Open>> {
        self.fds.remove(&fd).map(|f| f.open)
    }

    pub fn close_on_exec(&mut self) {
        self.fds.retain(|_, f| !f.cloexec);
    }
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
        dev: 0x803,
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
        Node::File(ino) | Node::Dir(ino) => stat_of(disk, *ino),
        Node::Stdin | Node::Stdout | Node::Stderr | Node::Pipe(..) => Ok(Stat { mode: S_IFIFO | 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::Null => Ok(device(1 << 8 | 3)),
        Node::Zero => Ok(device(1 << 8 | 5)),
        Node::Random => Ok(device(1 << 8 | 9)),
        Node::Made(bytes) => Ok(Stat { mode: S_IFREG | 0o444, nlink: 1, size: bytes.len() as i64, blksize: 4096, ..Stat::default() }),
        Node::Signalfd(_) | Node::Eventfd(_) | Node::Timerfd(_) | Node::Epoll(_) => Ok(Stat { mode: 0o600, nlink: 1, blksize: 4096, ..Stat::default() }),
        Node::Socket(s) => Ok(Stat { mode: S_IFSOCK | 0o777, nlink: 1, ino: s.socket.id(), blksize: 4096, ..Stat::default() }),
        Node::Netlink(n) => Ok(Stat { mode: S_IFSOCK | 0o777, nlink: 1, ino: n.id, blksize: 4096, ..Stat::default() }),
    }
}

impl Task {
    pub(crate) fn file(&self, fd: i32) -> Result<Arc<Open>, Errno> {
        lock(&self.process.state).files.get(fd).ok_or(EBADF)
    }

    fn read_path(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<String, Errno> {
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 4096))?;
        String::from_utf8(raw).map_err(|_| EINVAL)
    }

    /// The directory a path is relative to: the working directory, or an open directory.
    fn start_dir(&self, dirfd: i32) -> Result<(Ino, String), Errno> {
        if dirfd == AT_FDCWD {
            return Ok(lock(&self.process.state).cwd.clone());
        }
        let open = self.file(dirfd)?;
        match open.node {
            Node::Dir(ino) => Ok((ino, open.path.clone())),
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
                let mut offset = lock(&open.offset);
                let start = at.unwrap_or(*offset);
                let mut buffer = vec![0; len];
                let n = lock(&self.k.disk).read(*ino, start, &mut buffer)?;
                buffer.truncate(n);
                if at.is_none() {
                    *offset = start + n as u64;
                }
                Ok(buffer)
            }
            Node::Dir(_) => Err(EISDIR),
            Node::Pipe(_, true) => Err(EBADF),
            Node::Pipe(pipe, false) => pipe.read(&self.waiter, len, open.nonblock()),
            Node::Null => Ok(Vec::new()),
            Node::Zero => Ok(vec![0; len]),
            Node::Random => {
                let mut buffer = vec![0; len];
                self.k.host.random(&mut buffer);
                Ok(buffer)
            }
            Node::Made(bytes) => Ok(from_bytes(bytes)),
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
            Node::Epoll(_) => Err(EINVAL),
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
            Node::Pipe(pipe, true) => match pipe.write(&self.waiter, data, open.nonblock()) {
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
            Node::Signalfd(_) | Node::Timerfd(_) | Node::Epoll(_) => Err(EINVAL),
            Node::Stdin | Node::Dir(_) | Node::Pipe(_, false) | Node::Made(_) => Err(EBADF),
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
        let data = self.read_from(&open, len, Some(at))?;
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &data))?;
        Ok(data.len() as u64)
    }

    pub(crate) fn pwrite(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64, at: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
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
        let open = self.file(fd)?;
        let iovecs = self.iovecs(cx, at, count)?;
        let total = iovecs.iter().map(|(_, len)| len).sum();
        let data = self.read_from(&open, total, None)?;
        self.mem(cx, true, |t| {
            let mut done = 0;
            for &(base, len) in &iovecs {
                let n = (len as usize).min(data.len() - done);
                user::write(&mut t.space(), base, &data[done..done + n])?;
                done += n;
            }
            Ok(())
        })?;
        Ok(data.len() as u64)
    }

    pub(crate) fn writev(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, count: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let iovecs = self.iovecs(cx, at, count)?;
        let data = self.mem(cx, false, |t| {
            let space = t.space();
            let mut data = Vec::new();
            for &(base, len) in &iovecs {
                data.extend(user::read_vec(&space, base, len.min(MOST_AT_ONCE) as usize)?);
            }
            Ok(data)
        })?;
        Ok(self.write_to(&open, &data, None)? as u64)
    }

    pub(crate) fn openat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u32, mode: u32) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let absolute = exe_path(&dir_path, &path);
        let open = match os::open(self, &absolute) {
            Some(open) => {
                let open = open?;
                if let Node::Tty(tty) = &open.node
                    && flags & O_NOCTTY == 0
                {
                    self.adopt_terminal(tty);
                }
                open
            }
            None => self.open_on_disk(dir, &path, &absolute, flags, mode)?,
        };
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn open_on_disk(&self, dir: Ino, path: &str, absolute: &str, flags: u32, mode: u32) -> Result<Arc<Open>, Errno> {
        let umask = lock(&self.process.state).umask;
        let mut disk = lock(&self.k.disk);
        let ino = match disk.resolve(dir, path, flags & O_NOFOLLOW == 0) {
            Ok(_) if flags & O_CREAT != 0 && flags & O_EXCL != 0 => return Err(EEXIST),
            Ok(ino) => ino,
            Err(layers::Error::NotFound) if flags & O_CREAT != 0 => {
                let (parent, name) = disk.resolve_parent(dir, path)?;
                disk.create(parent, name, New::File, Meta::file(mode & !umask & 0o7777), self.k.now())?
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
            Kind::File if meta.mode & S_IFMT == S_IFCHR => match meta.rdev {
                0x103 => Node::Null,
                0x105 => Node::Zero,
                0x108 | 0x109 => Node::Random,
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
                if flags & O_TRUNC != 0 && write {
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
        let hold = match node {
            Node::File(ino) => {
                disk.hold(ino);
                Some(Hold { k: Arc::downgrade(&self.k), ino })
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
            Node::Dir(_) | Node::Null | Node::Zero => 0,
            Node::Made(bytes) => bytes.len() as u64,
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
        if let Some(stat) = os::stat(&exe_path(&dir_path, &path)) {
            return Ok(stat);
        }
        let disk = lock(&self.k.disk);
        let ino = disk.resolve(dir, &path, flags & AT_SYMLINK_NOFOLLOW == 0)?;
        stat_of(&disk, ino)
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

    pub(crate) fn getdents64(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, size: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let Node::Dir(ino) = open.node else { return Err(ENOTDIR) };
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
        let mut offset = lock(&open.offset);
        let mut out = Vec::new();
        let mut index = *offset as usize;
        while let Some((name, child, kind)) = entries.get(index) {
            let length = (19 + name.len() + 1).next_multiple_of(8);
            if out.len() + length > size as usize {
                break;
            }
            out.extend(child.to_le_bytes());
            out.extend(((index + 1) as i64).to_le_bytes());
            out.extend((length as u16).to_le_bytes());
            out.push(*kind);
            out.extend(name.as_bytes());
            out.resize(out.len() + length - 19 - name.len(), 0);
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
            Node::Pipe(pipe, false) if request == FIONREAD => {
                let n = pipe.available() as u32;
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, n))?;
                Ok(0)
            }
            Node::Socket(_) | Node::Netlink(_) => self.socket_ioctl(cx, &open, request, arg),
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
        if !(0..MOST_FILES).contains(&new) {
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
        if os::stat(&exe_path(&dir_path, &path)).is_some() {
            return Ok(0);
        }
        lock(&self.k.disk).resolve(dir, &path, true)?;
        Ok(0)
    }

    pub(crate) fn readlinkat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, buf: u64, size: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, dir_path) = self.start_dir(dirfd)?;
        let target = match os::read_link(self, &exe_path(&dir_path, &path)) {
            Some(target) => target?,
            None => {
                let disk = lock(&self.k.disk);
                let ino = disk.resolve(dir, &path, false)?;
                disk.read_link(ino).map_err(|_| EINVAL)?.to_string()
            }
        };
        let bytes = &target.as_bytes()[..target.len().min(size as usize)];
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, bytes))?;
        Ok(bytes.len() as u64)
    }

    pub(crate) fn mkdirat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, mode: u32) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, _) = self.start_dir(dirfd)?;
        let umask = lock(&self.process.state).umask;
        let mut disk = lock(&self.k.disk);
        // `/`, `.` and `..` are always there, as `tar` and `dpkg-deb` find making the directory they extract into.
        if path.trim_end_matches('/').is_empty() {
            return Err(EEXIST);
        }
        let (parent, name) = disk.resolve_parent(dir, &path)?;
        if matches!(name, "" | "." | "..") {
            return Err(EEXIST);
        }
        disk.create(parent, name, New::Dir, Meta::dir(mode & !umask & 0o7777), self.k.now())?;
        Ok(0)
    }

    pub(crate) fn unlinkat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<u64, Errno> {
        let path = self.read_path(cx, path_at)?;
        let (dir, _) = self.start_dir(dirfd)?;
        let mut disk = lock(&self.k.disk);
        let (parent, name) = disk.resolve_parent(dir, &path)?;
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
        let ((from_dir, _), (to_dir, _)) = (self.start_dir(olddirfd)?, self.start_dir(newdirfd)?);
        let mut disk = lock(&self.k.disk);
        let (from, from_name) = disk.resolve_parent(from_dir, &old)?;
        let (to, to_name) = disk.resolve_parent(to_dir, &new)?;
        disk.rename(from, from_name, to, to_name, self.k.now())?;
        Ok(0)
    }

    pub(crate) fn symlinkat(&mut self, cx: &mut Cx<'_>, target_at: u64, dirfd: i32, path_at: u64) -> Result<u64, Errno> {
        let (target, path) = (self.read_path(cx, target_at)?, self.read_path(cx, path_at)?);
        let (dir, _) = self.start_dir(dirfd)?;
        let mut disk = lock(&self.k.disk);
        let (parent, name) = disk.resolve_parent(dir, &path)?;
        disk.create(parent, name, New::Symlink(target), Meta::file(0o777), self.k.now())?;
        Ok(0)
    }

    /// `statfs`: every path is on the one disk, which says it's an ext4 of the machine's memory.
    pub(crate) fn statfs(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        const EXT4: u64 = 0xef53;
        let (total, free) = {
            let mut exec = lock(&self.k.exec);
            let size = exec.memory().size();
            (size.limit / 4096, size.available / 4096)
        };
        let mut b = Vec::with_capacity(120);
        for v in [EXT4, 4096, total, free, free, 1 << 20, 1 << 20, 0, 255, 4096, 0] {
            b.extend(v.to_le_bytes());
        }
        b.resize(120, 0);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }

    pub(crate) fn fstat_check(&self, fd: i32) -> Result<(), Errno> {
        self.file(fd).map(|_| ())
    }

    pub(crate) fn ftruncate(&mut self, fd: i32, len: u64) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let Node::File(ino) = open.node else { return Err(EINVAL) };
        if !open.writable() {
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
        let mut state = lock(&self.process.state);
        let disk = lock(&self.k.disk);
        let ino = disk.resolve(state.cwd.0, &path, true)?;
        if disk.stat(ino)?.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        let absolute = exe_path(&state.cwd.1, &path);
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
