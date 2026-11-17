//! CPU time. Each thread's is split into time running its program (user) and time in lambda dealing with a stop
//! (system), from the host's count for the host thread it runs on (`Host::thread_cpu`). In a hardware VM that's the
//! program's own time too; in a process sandbox, where the program runs in another process, the program's is that
//! process's, which its CPU reports (`Cx::program_time`). A process's time is its threads' together, those still running and those that have ended; its children's
//! is added as each is waited for, with theirs. `times`, `getrusage`, the usage `wait4` and `waitid` give, and the
//! CPU-time clocks (`CLOCK_PROCESS_CPUTIME_ID`, `CLOCK_THREAD_CPUTIME_ID`, and another's by `clock_getcpuclockid`)
//! read them.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Process, Task};
use crate::abi::*;

/// Clock ticks, as `times` and `waitid` count: Linux's `USER_HZ`.
const TICK: u64 = 10_000_000;

// `getrusage`'s whose.
const RUSAGE_SELF: i64 = 0;
const RUSAGE_CHILDREN: i64 = -1;
const RUSAGE_THREAD: i64 = 1;

/// A process's CPU time beyond its running threads': user and system, in nanoseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    /// Its threads that have ended.
    pub ended: (u64, u64),
    /// Its children that have been waited for, and theirs.
    pub children: (u64, u64),
}

pub(crate) fn add(a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    (a.0 + b.0, a.1 + b.1)
}

impl Process {
    /// Its threads' time, running and ended.
    pub(crate) fn cpu_times(&self) -> (u64, u64) {
        let state = lock(&self.state);
        state.threads.values().fold(state.usage.ended, |sum, live| add(sum, live.times.get()))
    }
}

/// How stale a thread's CPU time may get before a stop reads the host's clocks again. Each read is a system call with
/// no vDSO behind it (about a microsecond in a cloud VM), and reading two or three at every stop cost more than most
/// system calls do. In between, the monotonic clock marks when the thread went between its program and lambda.
const SAMPLE_EVERY: u64 = 1_000_000;
/// A stop at least this long reads them at its end: the thread may have slept, and in a hardware VM, where one clock
/// counts both, sleeping mustn't count against the split.
const LONG_STOP: u64 = 50_000;

/// Where a thread's time stands since its CPU time was last read: wall-clock nanoseconds in its program and in lambda,
/// and when it last went from one to the other.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Wall {
    pub mark: u64,
    pub sampled: u64,
    program: u64,
    lambda: u64,
}

impl Wall {
    pub(crate) fn new(now: u64) -> Wall {
        Wall { mark: now, sampled: now, program: 0, lambda: 0 }
    }
}

impl Task {
    /// The program has stopped: what ran since the last mark was the program's, on this host thread or, in a process
    /// sandbox, in its stub.
    pub(crate) fn account_stop(&mut self, cx: &mut Cx<'_>) {
        let now = self.k.host.monotonic();
        self.wall.program += now.saturating_sub(self.wall.mark);
        self.wall.mark = now;
        if now.saturating_sub(self.wall.sampled) >= SAMPLE_EVERY || self.process.cpu_timers.load(std::sync::atomic::Ordering::Relaxed) {
            self.sample(cx);
        }
    }

    /// Back to the program: what ran since the stop was lambda's.
    pub(crate) fn account_resume(&mut self, cx: &mut Cx<'_>) {
        let now = self.k.host.monotonic();
        if now.saturating_sub(self.wall.mark) >= LONG_STOP {
            self.sample(cx);
        } else {
            self.wall.lambda += now.saturating_sub(self.wall.mark);
            self.wall.mark = now;
        }
    }

    /// Reads the host's clocks and adds what they counted since the last read: in a process sandbox, the stub's is the
    /// program's and this thread's is lambda's; where one clock counts both (a hardware VM, on this thread), it's split
    /// by how long each ran. Called in lambda, so the time since the last mark is lambda's.
    pub(crate) fn sample(&mut self, cx: &mut Cx<'_>) {
        let now = self.k.host.monotonic();
        self.wall.lambda += now.saturating_sub(self.wall.mark);
        let cpu = self.k.host.thread_cpu();
        let host = cpu.saturating_sub(self.cpu_mark);
        self.cpu_mark = cpu;
        let (user, system) = match cx.program_time() {
            Some(program) => {
                let user = program.saturating_sub(self.program_mark);
                self.program_mark = program;
                (user, host)
            }
            None => {
                let wall = self.wall.program + self.wall.lambda;
                let user = if wall == 0 { 0 } else { (u128::from(host) * u128::from(self.wall.program) / u128::from(wall)) as u64 };
                (user, host - user)
            }
        };
        self.times.add(user, system);
        self.wall = Wall::new(now);
    }

    /// This thread's time, as last read (`sample`).
    fn own_times(&self) -> (u64, u64) {
        self.times.get()
    }

    /// This process's time, with this thread's as last read and its other threads' as they last read theirs.
    fn own_process_times(&self) -> (u64, u64) {
        self.process.cpu_times()
    }

    /// A CPU-time clock's reading, or None if `clock` isn't one or names no thread or process. 2 and 3 are this
    /// process's and this thread's; a negative one is `(~pid << 3) | kind`, with 4 in it for a thread, and `kind` 0
    /// (user and system), 1 (user) or 2 (user and system, as the scheduler counts it).
    pub(crate) fn cpu_clock(&self, clock: u64) -> Option<u64> {
        let clock = clock as i32;
        let ((user, system), kind) = match clock {
            2 => (self.own_process_times(), 0),
            3 => (self.own_times(), 0),
            ..0 => {
                let (id, thread, kind) = (!(clock >> 3), clock & 4 != 0, clock & 3);
                if kind == 3 {
                    return None;
                }
                let times = match (thread, id) {
                    (true, 0) => self.own_times(),
                    (true, tid) if tid == self.tid => self.own_times(),
                    (true, tid) => lock(&self.k.processes).values().find_map(|p| lock(&p.state).threads.get(&tid).map(|live| live.times.get()))?,
                    (false, 0) => self.own_process_times(),
                    (false, pid) if pid == self.process.pid => self.own_process_times(),
                    (false, pid) => self.k.process(pid)?.cpu_times(),
                };
                (times, kind)
            }
            _ => return None,
        };
        Some(if kind == 1 { user } else { user + system })
    }

    /// `times`: this process's and its waited-for children's user and system time, in clock ticks; and the ticks
    /// since the machine started.
    pub(crate) fn times(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        if at != 0 {
            self.sample(cx);
            let (user, system) = self.own_process_times();
            let children = lock(&self.process.state).usage.children;
            let bytes: Vec<u8> = [user, system, children.0, children.1].iter().flat_map(|ns| (ns / TICK).to_le_bytes()).collect();
            self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
        }
        Ok(self.k.host.monotonic() / TICK)
    }

    /// `getrusage`: this process's time, its waited-for children's, or this thread's.
    pub(crate) fn getrusage(&mut self, cx: &mut Cx<'_>, who: i64, at: u64) -> Result<u64, Errno> {
        self.sample(cx);
        let times = match who {
            RUSAGE_SELF => self.own_process_times(),
            RUSAGE_CHILDREN => lock(&self.process.state).usage.children,
            RUSAGE_THREAD => self.own_times(),
            _ => return Err(EINVAL),
        };
        self.write_rusage(cx, at, times)?;
        Ok(0)
    }

    /// A `struct rusage` with user and system time; nothing else in it is counted yet.
    pub(crate) fn write_rusage(&mut self, cx: &mut Cx<'_>, at: u64, (user, system): (u64, u64)) -> Result<(), Errno> {
        let mut b = Vec::with_capacity(144);
        for ns in [user, system] {
            b.extend((ns / 1_000_000_000).to_le_bytes());
            b.extend((ns % 1_000_000_000 / 1000).to_le_bytes());
        }
        b.resize(144, 0);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))
    }
}

/// Nanoseconds in clock ticks, for `waitid`'s `siginfo`.
pub(crate) fn ticks(ns: u64) -> u64 {
    ns / TICK
}
