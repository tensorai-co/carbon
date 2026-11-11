//! What Linux itself provides beyond system calls: the device files in `/dev`, put on the disk when a machine boots,
//! `/proc`, made each time it's read, and the machine's own `/etc/resolv.conf`, `/etc/hosts` and `/etc/hostname`.
//!
//! Still to come here, as the proposal has them: sysfs, ptys, a vDSO, and the Wayland and X11 servers.

use std::sync::Arc;

use kernels_core::lock;
use layers::New;

use crate::abi::*;
use crate::fs::{Node, Open};
use crate::mm::Backing;
use crate::process::{Process, Task};
use crate::{Disk, Meta};

/// Puts what every Linux machine has onto the disk, where the image hasn't: `/dev`'s devices, `/proc`, `/tmp`.
pub(crate) fn populate(disk: &mut Disk) {
    let dir = Meta::dir(0o755);
    let devices = [("/dev/null", 1, 3), ("/dev/zero", 1, 5), ("/dev/random", 1, 8), ("/dev/urandom", 1, 9), ("/dev/tty", 5, 0), ("/dev/console", 5, 1), ("/dev/ptmx", 5, 2)];
    for (path, major, minor) in devices {
        let _ = disk.put(path, New::File, &[], Meta::device(major, minor), dir);
    }
    for (link, target) in [("/dev/stdin", "/proc/self/fd/0"), ("/dev/stdout", "/proc/self/fd/1"), ("/dev/stderr", "/proc/self/fd/2"), ("/dev/fd", "/proc/self/fd")] {
        let _ = disk.put(link, New::Symlink(target.to_string()), &[], Meta::file(0o777), dir);
    }
    let _ = disk.put("/proc", New::Dir, &[], dir, dir);
    let _ = disk.put("/dev/pts", New::Dir, &[], dir, dir);
    let _ = disk.put("/tmp", New::Dir, &[], Meta::dir(0o1777), dir);
}

/// What a `/proc` path names, for the process asking.
enum Proc<'p> {
    Exe(Arc<Process>),
    Cwd(Arc<Process>),
    Fd(Arc<Process>, i32),
    Maps(Arc<Process>),
    Status(Arc<Process>),
    Global(&'p str),
    /// `/proc/net/<name>`.
    Net(&'p str),
}

fn parse<'p>(task: &Task, path: &'p str) -> Option<Proc<'p>> {
    let rest = path.strip_prefix("/proc/")?;
    let (who, what) = rest.split_once('/').unwrap_or((rest, ""));
    let process = match who {
        "self" | "thread-self" => Some(task.process.clone()),
        _ => who.parse().ok().and_then(|pid| task.k.process(pid)),
    };
    match (process, what) {
        (Some(p), "exe") => Some(Proc::Exe(p)),
        (Some(p), "cwd") => Some(Proc::Cwd(p)),
        (Some(p), "maps") => Some(Proc::Maps(p)),
        (Some(p), "status") => Some(Proc::Status(p)),
        (Some(p), fd) if fd.starts_with("fd/") => fd[3..].parse().ok().map(|fd| Proc::Fd(p, fd)),
        (Some(_), net) if net.starts_with("net/") => Some(Proc::Net(&net[4..])),
        (None, net) if who == "net" => Some(Proc::Net(net)),
        (None, "") if matches!(who, "cpuinfo" | "meminfo" | "uptime" | "version") => Some(Proc::Global(who)),
        _ => None,
    }
}

/// `stat` for `/dev/pts/N`.
pub(crate) fn pts_stat(n: u32) -> Stat {
    Stat { ino: 0x1_0000 + u64::from(n), mode: S_IFCHR | 0o620, nlink: 1, rdev: 136 << 8 | u64::from(n), blksize: 4096, ..Stat::default() }
}

fn pts_number(path: &str) -> Option<u32> {
    path.strip_prefix("/dev/pts/")?.parse().ok()
}

/// Opens a `/proc` file or a pty's slave, or None if `path` isn't one lambda makes.
pub(crate) fn open(task: &Task, path: &str) -> Option<Result<Arc<Open>, Errno>> {
    if let Some(n) = pts_number(path) {
        let tty = lock(&task.k.ptys).get(&n).and_then(std::sync::Weak::upgrade);
        return Some(match tty {
            Some(tty) => tty.open_slave().map(|()| Open::new(Node::Tty(tty), O_RDWR, path)),
            None => Err(ENOENT),
        });
    }
    if let Some(bytes) = crate::net::etc_file(&task.k, path) {
        return Some(Ok(Open::new(Node::Made(bytes), 0, path)));
    }
    let made = |bytes: String| Some(Ok(Open::new(Node::Made(bytes.into_bytes()), 0, path)));
    match parse(task, path)? {
        Proc::Fd(p, fd) => Some(lock(&p.state).files.get(fd).ok_or(ENOENT)),
        Proc::Exe(_) | Proc::Cwd(_) => None,
        Proc::Maps(p) => made(maps(&p)),
        Proc::Status(p) => {
            let state = lock(&p.state);
            let run = if state.stopped.is_some() { "T (stopped)" } else { "R (running)" };
            let (u, g) = (state.creds.uid, state.creds.gid);
            made(format!(
                "Name:\t{}\nState:\t{run}\nPid:\t{}\nPPid:\t{}\nUid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nThreads:\t{}\n",
                state.exe.rsplit('/').next().unwrap_or(""),
                p.pid,
                state.ppid,
                u[0],
                u[1],
                u[2],
                u[3],
                g[0],
                g[1],
                g[2],
                g[3],
                state.threads.len()
            ))
        }
        Proc::Global("cpuinfo") => made((0..task.k.host.cpus()).map(|i| format!("processor\t: {i}\n\n")).collect()),
        Proc::Global("meminfo") => {
            let size = lock(&task.k.exec).memory().size();
            made(format!("MemTotal:\t{} kB\nMemFree:\t{} kB\nMemAvailable:\t{} kB\n", size.limit / 1024, size.available / 1024, size.available / 1024))
        }
        Proc::Global("uptime") => {
            let up = task.k.host.monotonic() as f64 / 1e9;
            made(format!("{up:.2} {up:.2}\n"))
        }
        Proc::Global(_) => made("Linux version 6.12.0-carbon (Carbon v2)\n".to_string()),
        Proc::Net(name) => match crate::net::proc_net(&task.k, name) {
            Some(text) => made(text),
            None => Some(Err(ENOENT)),
        },
    }
}

/// `stat` for a `/proc` path, or None if it isn't one lambda makes.
pub(crate) fn stat(path: &str) -> Option<Stat> {
    if let Some(n) = pts_number(path) {
        return Some(pts_stat(n));
    }
    if matches!(path, "/etc/resolv.conf" | "/etc/hosts" | "/etc/hostname") {
        return Some(Stat { mode: S_IFREG | 0o644, nlink: 1, blksize: 4096, ..Stat::default() });
    }
    let rest = path.strip_prefix("/proc/")?;
    let link = rest.ends_with("/exe") || rest.ends_with("/cwd") || rest.contains("/fd/");
    let mode = if link { S_IFLNK | 0o777 } else { S_IFREG | 0o444 };
    Some(Stat { mode, nlink: 1, blksize: 4096, ..Stat::default() })
}

/// The target of a `/proc` link, or None if `path` isn't one lambda makes.
pub(crate) fn read_link(task: &Task, path: &str) -> Option<Result<String, Errno>> {
    match parse(task, path)? {
        Proc::Exe(p) => Some(Ok(lock(&p.state).exe.clone())),
        Proc::Cwd(p) => Some(Ok(lock(&p.state).cwd.1.clone())),
        Proc::Fd(p, fd) => Some(lock(&p.state).files.get(fd).map(|open| open.path.clone()).ok_or(ENOENT)),
        _ => Some(Err(EINVAL)),
    }
}

fn maps(process: &Process) -> String {
    let state = lock(&process.state);
    let mut out = String::new();
    for (range, area) in state.regions.iter() {
        if matches!(area.backing, Backing::Reserved) {
            continue;
        }
        let p = area.prot;
        let (offset, ino) = match area.backing {
            Backing::File { ino, offset } => (offset, ino),
            _ => (0, 0),
        };
        out += &format!(
            "{:x}-{:x} {}{}{}p {offset:08x} 00:00 {ino} {}\n",
            range.start,
            range.end,
            if p.read() { 'r' } else { '-' },
            if p.write() { 'w' } else { '-' },
            if p.exec() { 'x' } else { '-' },
            area.name
        );
    }
    out
}
