//! What Linux itself provides beyond system calls: the device files in `/dev`, put on the disk when a machine boots,
//! `/proc`, made each time it's read, and the machine's own `/etc/resolv.conf`, `/etc/hosts` and `/etc/hostname`.
//!
//! The Wayland server is `wayland`, the X server `x11`, the vDSO and its clock page `vdso`. Still to come here, as the
//! proposal has them: sysfs.

pub(crate) mod vdso;
pub(crate) mod wayland;
pub(crate) mod x11;

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
    /// `/proc/<pid>/fd`: its open descriptors, as links.
    Fds(Arc<Process>),
    /// `/proc/<pid>/task`: its threads, by their IDs.
    Tasks(Arc<Process>),
    Maps(Arc<Process>),
    Status(Arc<Process>),
    Global(&'p str),
    /// A process's mounts: `mountinfo`, or `mounts`.
    Mounts(Arc<Process>, bool),
    /// A setting under `/proc/sys`.
    Sys(&'p str),
    /// `/proc/net/<name>`.
    Net(&'p str),
    /// `/proc/<pid>/ns/<kind>`.
    Ns(Arc<Process>, crate::ns::NsKind),
}

/// The namespace a `/proc/<pid>/ns/` leaf names.
fn ns_kind(leaf: &str) -> Option<crate::ns::NsKind> {
    crate::ns::NsKind::ALL.into_iter().find(|k| k.name() == leaf)
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
        (Some(p), "mounts") => Some(Proc::Mounts(p, false)),
        (Some(p), "mountinfo") => Some(Proc::Mounts(p, true)),
        (Some(p), ns) if ns.strip_prefix("ns/").is_some_and(|k| ns_kind(k).is_some()) => Some(Proc::Ns(p, ns_kind(&ns[3..]).unwrap())),
        (None, "") if who == "mounts" => Some(Proc::Mounts(task.process.clone(), false)),
        (Some(p), fd) if fd.starts_with("fd/") => fd[3..].parse().ok().map(|fd| Proc::Fd(p, fd)),
        (Some(p), "fd" | "fd/") => Some(Proc::Fds(p)),
        (Some(p), "task" | "task/") => Some(Proc::Tasks(p)),
        (Some(_), net) if net.starts_with("net/") => Some(Proc::Net(&net[4..])),
        (None, net) if who == "net" => Some(Proc::Net(net)),
        (None, "") if matches!(who, "cpuinfo" | "meminfo" | "uptime" | "version") => Some(Proc::Global(who)),
        (None, setting) if who == "sys" && SYSCTLS.contains(&setting) => Some(Proc::Sys(setting)),
        _ => None,
    }
}

/// The `/proc/sys` settings there are.
const SYSCTLS: [&str; 7] = [
    "kernel/cap_last_cap",
    "kernel/hostname",
    "kernel/domainname",
    "kernel/osrelease",
    "kernel/dmesg_restrict",
    "kernel/seccomp/actions_avail",
    "kernel/perf_event_paranoid",
];

/// A `/proc/sys` setting's value.
fn sysctl(task: &Task, setting: &str) -> String {
    let names = lock(&task.uts_ns().names).clone();
    match setting {
        "kernel/cap_last_cap" => format!("{}\n", crate::process::creds::CAP_LAST_CAP),
        "kernel/hostname" => format!("{}\n", names.host),
        "kernel/domainname" => format!("{}\n", names.domain),
        "kernel/osrelease" => format!("{RELEASE}\n"),
        "kernel/seccomp/actions_avail" => "kill_process kill_thread trap errno trace log allow\n".to_string(),
        // Performance counters are refused: the most paranoid setting says so.
        "kernel/perf_event_paranoid" => "4\n".to_string(),
        _ => "0\n".to_string(),
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
        Proc::Fds(p) => {
            let fds = lock(&p.state).files.numbers();
            Some(Ok(Open::new(Node::Listing(fds.into_iter().map(|fd| (fd.to_string(), DT_LNK)).collect()), O_DIRECTORY, path)))
        }
        Proc::Tasks(p) => {
            let tids: Vec<i32> = lock(&p.state).threads.keys().copied().collect();
            Some(Ok(Open::new(Node::Listing(tids.into_iter().map(|tid| (tid.to_string(), DT_DIR)).collect()), O_DIRECTORY, path)))
        }
        Proc::Exe(_) | Proc::Cwd(_) => None,
        Proc::Ns(p, kind) => Some(Ok(Open::new(Node::Ns(lock(&p.state).ns.reference(kind)), 0, path))),
        Proc::Maps(p) => made(maps(&p)),
        Proc::Status(p) => {
            let state = lock(&p.state);
            // Its main thread's, as Linux's file is the thread group leader's.
            let leader = state.threads.get(&p.pid).or_else(|| state.threads.values().next()).map(|l| l.trace.clone());
            let tracing = leader.as_ref().is_some_and(|t| t.stopped());
            let tracer = leader.and_then(|t| lock(&t.0).tracer).unwrap_or(0);
            let run = if tracing {
                "t (tracing stop)"
            } else if state.stopped.is_some() {
                "T (stopped)"
            } else {
                "R (running)"
            };
            let (u, g, c) = (state.creds.uid, state.creds.gid, state.creds.caps);
            let seccomp = crate::security::seccomp::Seccomp::mode(&state.security.seccomp);
            let filters = state.security.seccomp.as_ref().map_or(0, |s| s.filters.len());
            made(format!(
                "Name:\t{}\nState:\t{run}\nPid:\t{}\nPPid:\t{}\nTracerPid:\t{tracer}\nUid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nThreads:\t{}\n\
                 CapInh:\t{:016x}\nCapPrm:\t{:016x}\nCapEff:\t{:016x}\nCapBnd:\t{:016x}\nCapAmb:\t{:016x}\nNoNewPrivs:\t{}\n\
                 Seccomp:\t{seccomp}\nSeccomp_filters:\t{filters}\n",
                Task::comm(&state),
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
                state.threads.len(),
                c.inheritable,
                c.permitted,
                c.effective,
                c.bounding,
                c.ambient,
                u8::from(state.security.no_new_privs),
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
        Proc::Global(_) => made(format!("Linux version {RELEASE} (lambda) {VERSION}\n")),
        Proc::Sys(setting) => made(sysctl(task, setting)),
        Proc::Mounts(p, info) => {
            let (mounts, root) = {
                let state = lock(&p.state);
                (state.ns.mnt.current(), state.root.1.clone())
            };
            made(if info { mounts.mountinfo_text(&root) } else { mounts.mounts_text(&root) })
        }
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
    let (dev, ino) = made_identity(path);
    if matches!(path, "/etc/resolv.conf" | "/etc/hosts" | "/etc/hostname") {
        return Some(Stat { dev, ino, mode: S_IFREG | 0o644, nlink: 1, blksize: 4096, ..Stat::default() });
    }
    let rest = path.strip_prefix("/proc/")?;
    if is_listing(path) {
        return Some(Stat { dev, ino, mode: S_IFDIR | 0o555, nlink: 2, blksize: 4096, ..Stat::default() });
    }
    let link = rest.ends_with("/exe") || rest.ends_with("/cwd") || rest.contains("/fd/") || rest.rsplit_once("/ns/").is_some_and(|(_, k)| ns_kind(k).is_some());
    let mode = if link { S_IFLNK | 0o777 } else { S_IFREG | 0o444 };
    Some(Stat { dev, ino, mode, nlink: 1, blksize: 4096, ..Stat::default() })
}

/// Whether `path` is a directory lambda makes when it's opened (`/proc/<pid>/fd`, `/proc/<pid>/task`), whose `stat`
/// depends on what's in it then.
pub(crate) fn is_listing(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/proc/") else { return false };
    let (who, what) = rest.split_once('/').unwrap_or((rest, ""));
    (who == "self" || who == "thread-self" || who.parse::<i32>().is_ok()) && matches!(what.trim_end_matches('/'), "fd" | "task")
}

/// A file lambda makes: its device (proc's, or for `/etc`'s, one of their own) and an inode of its own, never 0. A
/// file with no identity looks like any other with none: GNU grep took `/proc/self/status` for its own output, a
/// pipe's zeroes, and wouldn't read it.
pub(crate) fn made_identity(path: &str) -> (u64, u64) {
    let dev = if path.starts_with("/proc/") { 0x16 } else { 0x17 };
    // FNV-1a.
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    (dev, hash >> 32 | 1)
}

/// The target of a `/proc` link, or None if `path` isn't one lambda makes.
pub(crate) fn read_link(task: &Task, path: &str) -> Option<Result<String, Errno>> {
    match parse(task, path)? {
        Proc::Exe(p) => Some(Ok(lock(&p.state).exe.clone())),
        Proc::Cwd(p) => Some(Ok(lock(&p.state).cwd.1.clone())),
        Proc::Fd(p, fd) => Some(lock(&p.state).files.get(fd).map(|open| open.path.clone()).ok_or(ENOENT)),
        Proc::Ns(p, kind) => Some(Ok(format!("{}:[{}]", kind.name(), lock(&p.state).ns.id(kind)))),
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
        let (offset, ino, shared) = match area.backing {
            Backing::File { ino, offset } => (offset, ino, false),
            Backing::Shared { ref mem, offset, private } => (offset, mem.ino, !private),
            _ => (0, 0, false),
        };
        out += &format!(
            "{:x}-{:x} {}{}{}{} {offset:08x} 00:00 {ino} {}\n",
            range.start,
            range.end,
            if p.read() { 'r' } else { '-' },
            if p.write() { 'w' } else { '-' },
            if p.exec() { 'x' } else { '-' },
            if shared { 's' } else { 'p' },
            area.name
        );
    }
    out
}
