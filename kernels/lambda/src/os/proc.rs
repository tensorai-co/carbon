//! `/proc`'s listings and the process files `ps`, `top` and `pgrep` read: `/proc` itself (a directory for each
//! process), `/proc/<pid>` (its files), `stat`, `statm`, `cmdline`, `environ` and `comm`, and the machine's `stat`,
//! `loadavg` and `filesystems`, in Linux's formats.
//!
//! `cmdline` and `environ` are read from the process's memory, where its last `execve` put its strings, so a title a
//! program writes over them (`process.title`, `setproctitle`) is what's seen, as on Linux; `stat` and `statm` count
//! its resident pages. Both need its address space, so they're made where a thread can reach it
//! (`Task::open_proc_memory`); the rest is made from the process's state alone.

use std::sync::Arc;

use kernels_core::lock;

use crate::abi::*;
use crate::mm::Backing;
use crate::process::{Process, Task};
use crate::Kernel;

/// Clock ticks (`USER_HZ`, 100 a second), in nanoseconds: the unit `stat`'s times are in.
const TICK: u64 = 10_000_000;
const PAGE: u64 = 4096;

/// What's in `/proc` besides the processes.
const MACHINE: [(&str, u8); 11] = [
    ("cpuinfo", DT_REG),
    ("filesystems", DT_REG),
    ("loadavg", DT_REG),
    ("meminfo", DT_REG),
    ("mounts", DT_REG),
    ("stat", DT_REG),
    ("uptime", DT_REG),
    ("version", DT_REG),
    ("net", DT_DIR),
    ("sys", DT_DIR),
    ("self", DT_DIR),
];

/// What's in `/proc/<pid>`.
pub(crate) const PROCESS: [(&str, u8); 17] = [
    ("cmdline", DT_REG),
    ("comm", DT_REG),
    ("cwd", DT_LNK),
    ("environ", DT_REG),
    ("exe", DT_LNK),
    ("fd", DT_DIR),
    ("maps", DT_REG),
    ("mountinfo", DT_REG),
    ("mounts", DT_REG),
    ("net", DT_DIR),
    ("ns", DT_DIR),
    ("root", DT_LNK),
    ("stat", DT_REG),
    ("statm", DT_REG),
    ("status", DT_REG),
    ("task", DT_DIR),
    ("thread-self", DT_DIR),
];

/// What's in `/proc/<pid>/task/<tid>`: the files a thread has that tools read.
pub(crate) const THREAD: [(&str, u8); 7] = [("cmdline", DT_REG), ("comm", DT_REG), ("environ", DT_REG), ("stat", DT_REG), ("statm", DT_REG), ("status", DT_REG), ("syscall", DT_REG)];

/// The names of the files a process has, for `parse` and `stat`.
pub(crate) fn is_process_file(name: &str) -> bool {
    PROCESS.iter().any(|(n, _)| *n == name)
}

/// `/proc`: each process, then the machine's files.
pub(crate) fn root(k: &Kernel) -> Vec<(String, u8)> {
    let pids: Vec<i32> = lock(&k.processes).keys().copied().collect();
    let mut out: Vec<(String, u8)> = pids.into_iter().map(|pid| (pid.to_string(), DT_DIR)).collect();
    out.extend(MACHINE.iter().map(|(n, t)| (n.to_string(), *t)));
    out.push(("thread-self".to_string(), DT_DIR));
    out
}

/// A listing of names, made into `Node::Listing`'s entries.
pub(crate) fn entries(names: &[(&str, u8)]) -> Vec<(String, u8)> {
    names.iter().map(|(n, t)| (n.to_string(), *t)).collect()
}

/// `/proc/<pid>/ns`.
pub(crate) fn namespaces() -> Vec<(String, u8)> {
    crate::ns::NsKind::ALL.into_iter().map(|k| (k.name().to_string(), DT_LNK)).collect()
}

/// A device number as Linux encodes it for `stat`'s `tty_nr`: the major in bits 8-19, the minor's low byte below it
/// and the rest of it above.
fn encode_dev(major: u32, minor: u32) -> u64 {
    u64::from(minor & 0xff) | (u64::from(major) << 8) | (u64::from(minor & !0xff) << 12)
}

/// A process's one-letter state, as `stat` and `status` give it.
pub(crate) fn state_letter(task: &Task, p: &Process) -> char {
    let state = lock(&p.state);
    if state.zombie.is_some() {
        return 'Z';
    }
    let leader = state.threads.get(&p.pid).or_else(|| state.threads.values().next());
    if leader.is_some_and(|l| l.trace.stopped()) {
        't'
    } else if state.stopped.is_some() {
        'T'
    } else if p.pid == task.process.pid {
        // Whoever reads it is running; the rest are waiting for something, as nearly every process is.
        'R'
    } else {
        'S'
    }
}

/// `/proc/<pid>/stat`: one line, Linux's 52 fields. `resident` is its resident memory in bytes.
pub(crate) fn stat(task: &Task, p: &Process, resident: u64) -> String {
    let letter = state_letter(task, p);
    let (user, system) = p.cpu_times();
    let state = lock(&p.state);
    let comm = Task::comm(&state);
    let (tty_nr, tpgid) = match &state.ctty {
        Some(tty) => (tty.index.map_or(encode_dev(5, 1), |n| encode_dev(136, n)), i64::from(tty.foreground())),
        None => (0, -1),
    };
    let (vsize, code) = sizes(&state);
    let (ignored, caught) = state.signals.ignored_and_caught();
    let blocked = state.threads.get(&p.pid).map_or(0, |l| l.signals.mask.load(std::sync::atomic::Ordering::Acquire));
    let (cutime, cstime) = state.usage.children;
    let exit_code = state.zombie.map_or(0, crate::Status::wait_status);
    let fields: [u64; 49] = [
        state.ppid as u64,
        state.pgid as u64,
        state.sid as u64,
        tty_nr,
        tpgid as u64,
        0x0040_0000, // flags: PF_FORKNOEXEC's neighbour, as an ordinary process's
        0,
        0,
        0,
        0,
        user / TICK,
        system / TICK,
        cutime / TICK,
        cstime / TICK,
        20,
        0,
        state.threads.len().max(1) as u64,
        0,
        state.started / TICK,
        vsize,
        resident / PAGE,
        u64::MAX,
        code.start,
        code.end,
        state.arg_area.start,
        0,
        0,
        state.pending,
        blocked,
        ignored,
        caught,
        0,
        0,
        0,
        17,
        0,
        0,
        0,
        0,
        0,
        0,
        state.brk.0,
        state.brk.0,
        state.brk.0,
        state.arg_area.start,
        state.arg_area.end,
        state.env_area.start,
        state.env_area.end,
        exit_code as u64,
    ];
    // tpgid is -1 with no terminal, as Linux writes it.
    let mut text = format!("{} ({comm}) {letter}", p.pid);
    for (i, field) in fields.iter().enumerate() {
        if i == 4 && tpgid < 0 {
            text.push_str(" -1");
        } else {
            text.push_str(&format!(" {field}"));
        }
    }
    text.push('\n');
    text
}

/// Its mapped size in bytes, and where its program's code is (`stat`'s startcode and endcode: both 0 is a kernel
/// thread to `ps`, so a process's are never).
fn sizes(state: &crate::process::ProcState) -> (u64, std::ops::Range<u64>) {
    let mut vsize = 0;
    let mut code: Option<std::ops::Range<u64>> = None;
    for (range, area) in state.regions.iter() {
        if matches!(area.backing, Backing::Reserved) {
            continue;
        }
        vsize += range.end - range.start;
        if code.is_none() && area.prot.exec() {
            code = Some(range.clone());
        }
    }
    (vsize, code.unwrap_or(0x1000..0x2000))
}

/// `/proc/<pid>/statm`: sizes in pages: all of it, resident, shared, code, libraries (0), data and stack, dirty (0).
pub(crate) fn statm(p: &Process, usage: exec::SpaceUsage) -> String {
    let state = lock(&p.state);
    let (vsize, _) = sizes(&state);
    let resident = usage.code + usage.read_only + usage.writable;
    format!("{} {} {} {} 0 {} 0\n", vsize / PAGE, resident / PAGE, usage.shared / PAGE, usage.code / PAGE, usage.writable / PAGE)
}

/// What `status` adds about memory: its size and what of it is resident.
pub(crate) fn status_memory(state: &crate::process::ProcState, resident: Option<u64>) -> String {
    let (vsize, code) = sizes(state);
    let mut text = format!("VmPeak:\t{} kB\nVmSize:\t{} kB\n", vsize / 1024, vsize / 1024);
    if let Some(resident) = resident {
        text += &format!("VmHWM:\t{0} kB\nVmRSS:\t{0} kB\nRssAnon:\t{0} kB\nRssFile:\t0 kB\nRssShmem:\t0 kB\n", resident / 1024);
    }
    text += &format!("VmExe:\t{} kB\nVmStk:\t{} kB\n", (code.end - code.start) / 1024, 8 << 10);
    text
}

/// `/proc/<pid>/cmdline` from its argument strings as they are now: up to the end of the last one, or, if a title
/// was written past it (its last byte isn't a NUL), on into the environment's to the title's end, as Linux reads it.
pub(crate) fn cmdline(args: Vec<u8>, env: impl FnOnce() -> Vec<u8>) -> Vec<u8> {
    if args.last().is_none_or(|&b| b == 0) {
        return args;
    }
    let mut out = args;
    let more = env();
    let end = more.iter().position(|&b| b == 0).map_or(more.len(), |i| i + 1);
    out.extend_from_slice(&more[..end]);
    out
}

/// `/proc/stat`: the CPUs' time (the processes' together, the rest idle), when the machine booted, and its
/// processes.
pub(crate) fn kernel_stat(k: &Kernel) -> String {
    let processes: Vec<Arc<Process>> = lock(&k.processes).values().cloned().collect();
    let (mut user, mut system) = (0u64, 0u64);
    let mut running = 0;
    for p in &processes {
        let (u, s) = p.cpu_times();
        user += u;
        system += s;
        let state = lock(&p.state);
        if state.zombie.is_none() && state.stopped.is_none() && !state.threads.is_empty() {
            running += 1;
        }
    }
    let cpus = k.host.cpus().max(1) as u64;
    let up = k.host.monotonic();
    let (user, system) = (user / TICK, system / TICK);
    let idle = (up / TICK * cpus).saturating_sub(user + system);
    let mut text = format!("cpu  {user} 0 {system} {idle} 0 0 0 0 0 0\n");
    for i in 0..cpus {
        let share = |n: u64| n / cpus + u64::from(i < n % cpus);
        text += &format!("cpu{i} {} 0 {} {} 0 0 0 0 0 0\n", share(user), share(system), share(idle));
    }
    let boot = k.host.wall().saturating_sub(up) / 1_000_000_000;
    text += &format!("intr 0\nctxt 0\nbtime {boot}\nprocesses {}\nprocs_running {}\nprocs_blocked 0\nsoftirq 0 0 0 0 0 0 0 0 0 0 0\n", processes.len(), running.max(1));
    text
}

/// `/proc/loadavg`: no load history is kept, so its averages are 0; then running and all threads, and the newest pid.
pub(crate) fn loadavg(k: &Kernel) -> String {
    let processes: Vec<Arc<Process>> = lock(&k.processes).values().cloned().collect();
    let threads: usize = processes.iter().map(|p| lock(&p.state).threads.len()).sum();
    let newest = processes.iter().map(|p| p.pid).max().unwrap_or(1);
    format!("0.00 0.00 0.00 1/{threads} {newest}\n")
}

/// `/proc/filesystems`: the kinds lambda mounts.
pub(crate) const FILESYSTEMS: &str = "nodev\tsysfs\nnodev\ttmpfs\nnodev\tproc\nnodev\tdevpts\nnodev\tmqueue\n\text4\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers_as_linux_encodes_them() {
        assert_eq!(encode_dev(136, 0), 0x8800);
        assert_eq!(encode_dev(136, 3), 0x8803);
        assert_eq!(encode_dev(5, 1), 0x501);
        assert_eq!(encode_dev(136, 256), 0x10_8800);
    }

    #[test]
    fn cmdline_as_linux_reads_it() {
        // As exec left it.
        assert_eq!(cmdline(b"node\0npm\0--version\0".to_vec(), || b"HOME=/root\0".to_vec()), b"node\0npm\0--version\0");
        // A title written over the arguments, padded with NULs.
        assert_eq!(cmdline(b"npm\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec(), Vec::new), b"npm\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        // A title running on into the environment's strings.
        assert_eq!(cmdline(b"a long title tha".to_vec(), || b"t goes on\0HOME=/root\0".to_vec()), b"a long title that goes on\0");
    }
}
