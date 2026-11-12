//! Signals: what each one does, sending them to processes, groups and threads, and delivering them to a thread on its
//! way back to its program, with a frame on the program's stack for a handler and `rt_sigreturn` to come back.
//!
//! A signal is delivered at a thread's next stop: a system call ending, a fault, or a preempt, which sending one makes
//! happen. A system call a signal interrupted is restarted, unless a handler without `SA_RESTART` ran.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use exec::isa::x86::{R8, R9, R10, R11, RAX, RBP, RBX, RCX, RDI, RDX, RSI, RSP, USER_FLAGS};
use exec::{FpRegs, Regs};
use kernels_core::lock;
use kernels_core::thread::{Cx, Flow};
use kernels_core::user;

use crate::abi::*;
use crate::process::{ProcState, Process, Report, Task};
use crate::sys::Sys;
use crate::{Kernel, Status};

pub const SIGHUP: i32 = 1;
pub const SIGINT: i32 = 2;
pub const SIGQUIT: i32 = 3;
pub const SIGPIPE: i32 = 13;
pub const SIGCONT: i32 = 18;
pub const SIGSTOP: i32 = 19;
pub const SIGTSTP: i32 = 20;
pub const SIGTTIN: i32 = 21;
pub const SIGTTOU: i32 = 22;
pub const SIGWINCH: i32 = 28;
pub const SIGSYS: i32 = 31;
/// SIGSYS's `si_code` from a seccomp filter's `SECCOMP_RET_TRAP`.
pub(crate) const SYS_SECCOMP: i32 = 1;

const SIG_DFL: u64 = 0;
const SIG_IGN: u64 = 1;
const SA_NOCLDSTOP: u64 = 1;
const SA_RESTORER: u64 = 0x0400_0000;
const SA_ONSTACK: u64 = 0x0800_0000;
const SA_RESTART: u64 = 0x1000_0000;
const SA_NODEFER: u64 = 0x4000_0000;
const SA_RESETHAND: u64 = 0x8000_0000;
const SS_ONSTACK: u32 = 1;
const SS_DISABLE: u32 = 2;
/// `si_code`s.
pub(crate) const SI_USER: i32 = 0;
const SI_KERNEL: i32 = 0x80;
/// From a POSIX timer, with its ID, overrun and value.
pub(crate) const SI_TIMER: i32 = -2;
const SI_TKILL: i32 = -6;
/// The first real-time signal, as the kernel counts: each one sent is queued, not merged with one pending.
pub(crate) const SIGRTMIN: i32 = 32;
/// The resource limit on how many signals a process may have queued.
const RLIMIT_SIGPENDING: usize = 11;

pub const fn bit(signal: i32) -> u64 {
    1 << (signal - 1)
}

/// SIGKILL and SIGSTOP can't be blocked, caught or ignored.
pub(crate) const UNBLOCKABLE: u64 = bit(SIGKILL) | bit(SIGSTOP);
const STOPS: u64 = bit(SIGSTOP) | bit(SIGTSTP) | bit(SIGTTIN) | bit(SIGTTOU);

/// What a signal does when nothing's been said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Default {
    Term,
    Core,
    Ignore,
    Stop,
    Continue,
}

fn default_of(signal: i32) -> Default {
    match signal {
        SIGCHLD | 23 | SIGWINCH => Default::Ignore,
        SIGCONT => Default::Continue,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => Default::Stop,
        SIGQUIT | SIGILL | SIGTRAP | 6 | SIGBUS | SIGFPE | SIGSEGV | 24 | 25 | 31 => Default::Core,
        _ => Default::Term,
    }
}

/// Whether `signal` ends a process with a core dump, when nothing's been said (`CLD_DUMPED`).
pub(crate) fn dumps_core(signal: i32) -> bool {
    default_of(signal) == Default::Core
}

/// `struct sigaction`, as the kernel has it on both ISAs: handler, flags, restorer, mask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Action {
    handler: u64,
    flags: u64,
    restorer: u64,
    mask: u64,
}

impl Action {
    fn encode(&self) -> [u8; 32] {
        let mut b = [0; 32];
        for (i, v) in [self.handler, self.flags, self.restorer, self.mask].into_iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn decode(b: &[u8]) -> Action {
        let at = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or_default());
        Action { handler: at(0), flags: at(1), restorer: at(2), mask: at(3) }
    }
}

/// A process's signal handlers.
#[derive(Debug, Clone)]
pub(crate) struct Signals {
    actions: [Action; 64],
}

impl std::default::Default for Signals {
    fn default() -> Signals {
        Signals { actions: [Action::default(); 64] }
    }
}

impl Signals {
    fn action(&self, signal: i32) -> Action {
        self.actions[(signal - 1) as usize]
    }

    /// Whether sending `signal` does nothing at all.
    fn ignores(&self, signal: i32) -> bool {
        if bit(signal) & UNBLOCKABLE != 0 {
            return false;
        }
        match self.action(signal).handler {
            SIG_IGN => true,
            SIG_DFL => default_of(signal) == Default::Ignore,
            _ => false,
        }
    }

    /// After `execve`, handlers are gone (the code they pointed at is), but ignored signals stay ignored.
    pub fn reset_on_exec(&mut self) {
        for action in &mut self.actions {
            if action.handler != SIG_IGN {
                *action = Action::default();
            }
        }
    }
}

/// A thread's pending signals and mask, shared with whoever sends it signals, and what each signal sent to it alone
/// is about.
#[derive(Debug, Default)]
pub(crate) struct ThreadSignals {
    pub pending: AtomicU64,
    pub mask: AtomicU64,
    pub infos: Mutex<Queues>,
}

impl ThreadSignals {
    pub fn with_mask(mask: u64) -> Arc<ThreadSignals> {
        Arc::new(ThreadSignals { pending: AtomicU64::new(0), mask: AtomicU64::new(mask), infos: Mutex::default() })
    }
}

/// What each pending signal is about, oldest first: one for a standard signal, one for each sent for a real-time one.
pub(crate) type Queues = BTreeMap<i32, VecDeque<Info>>;

/// Puts `info` on `signal`'s queue: a real-time signal's grows, up to `limit`; another's holds one, the first sent.
/// False if it's full.
fn queue(infos: &mut Queues, pending: bool, signal: i32, info: Info, limit: usize) -> bool {
    let q = infos.entry(signal).or_default();
    if signal >= SIGRTMIN {
        if q.len() >= limit {
            return false;
        }
        q.push_back(info);
    } else if !pending || q.is_empty() {
        q.clear();
        q.push_back(info);
    }
    true
}

/// The oldest of `signal`'s queue, taking it off.
fn take(infos: &mut Queues, signal: i32) -> Info {
    let Some(q) = infos.get_mut(&signal) else { return Info::user(0) };
    let info = q.pop_front().unwrap_or(Info::user(0));
    if q.is_empty() {
        infos.remove(&signal);
    }
    info
}

/// Takes queued signals `drop` picks out of a process's queues and its threads', clearing what's no longer pending: a
/// deleted timer's.
pub(crate) fn purge(state: &mut ProcState, drop: impl Fn(&Info) -> bool) {
    let prune = |infos: &mut Queues| -> u64 {
        let mut emptied = 0;
        infos.retain(|signal, q| {
            q.retain(|info| !drop(info));
            if q.is_empty() {
                emptied |= bit(*signal);
            }
            !q.is_empty()
        });
        emptied
    };
    let emptied = prune(&mut state.infos);
    state.pending &= !emptied;
    for live in state.threads.values() {
        let emptied = prune(&mut lock(&live.signals.infos));
        live.signals.pending.fetch_and(!emptied, Ordering::AcqRel);
    }
}

/// A thread's alternate signal stack.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AltStack {
    pub sp: u64,
    pub size: u64,
}

/// What a signal is about, for its siginfo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Info {
    pub code: i32,
    /// Who sent it, or which child it's about, and their user.
    pub pid: i32,
    pub uid: u32,
    /// A child's exit code or signal.
    pub status: i32,
    /// Where a fault was.
    pub addr: u64,
    /// `sigqueue`'s or a timer's `sigval`.
    pub value: u64,
    /// A POSIX timer's ID, and how many expiries its signal stands for beyond the first.
    pub timer: i32,
    pub overrun: i32,
    /// `si_errno`.
    pub errno: i32,
    /// For SIGSYS from seccomp: the call's number and audit arch (with `addr`, where it was called).
    pub syscall: (i32, u32),
}

impl Info {
    pub fn user(pid: i32) -> Info {
        Info { code: SI_USER, pid, ..Info::default() }
    }

    /// `siginfo_t`, as a handler gets it.
    pub(crate) fn siginfo(&self, signal: i32) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&signal.to_le_bytes());
        b[4..8].copy_from_slice(&self.errno.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        if self.addr != 0 {
            b[16..24].copy_from_slice(&self.addr.to_le_bytes());
            if signal == SIGSYS && self.code == SYS_SECCOMP {
                b[24..28].copy_from_slice(&self.syscall.0.to_le_bytes());
                b[28..32].copy_from_slice(&self.syscall.1.to_le_bytes());
            }
        } else if self.code == SI_TIMER {
            b[16..20].copy_from_slice(&self.timer.to_le_bytes());
            b[20..24].copy_from_slice(&self.overrun.to_le_bytes());
            b[24..32].copy_from_slice(&self.value.to_le_bytes());
        } else {
            b[16..20].copy_from_slice(&self.pid.to_le_bytes());
            b[20..24].copy_from_slice(&self.uid.to_le_bytes());
            // A program's own codes (sigqueue's, a message queue's) carry a value; a child's change, its status.
            if self.code < 0 && self.code != SI_TKILL {
                b[24..32].copy_from_slice(&self.value.to_le_bytes());
            } else {
                b[24..28].copy_from_slice(&self.status.to_le_bytes());
            }
        }
        b
    }

    /// `struct signalfd_siginfo`, as `signalfd` gives it.
    fn signalfd(&self, signal: i32) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&signal.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        b[12..16].copy_from_slice(&self.pid.to_le_bytes());
        b[16..20].copy_from_slice(&self.uid.to_le_bytes());
        b[24..28].copy_from_slice(&self.timer.to_le_bytes());
        b[32..36].copy_from_slice(&self.overrun.to_le_bytes());
        b[40..44].copy_from_slice(&self.status.to_le_bytes());
        b[44..48].copy_from_slice(&(self.value as i32).to_le_bytes());
        b[48..56].copy_from_slice(&self.value.to_le_bytes());
        b[72..80].copy_from_slice(&self.addr.to_le_bytes());
        b[4..8].copy_from_slice(&self.errno.to_le_bytes());
        if signal == SIGSYS && self.code == SYS_SECCOMP {
            b[84..88].copy_from_slice(&self.syscall.0.to_le_bytes());
            b[88..96].copy_from_slice(&self.addr.to_le_bytes());
            b[96..100].copy_from_slice(&self.syscall.1.to_le_bytes());
        }
        b
    }
}

/// How a child changed, for its parent's SIGCHLD.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Change {
    Ended(Status),
    Stopped(i32),
    Continued,
}

/// A system call that a signal interrupted, to restart or not.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Interrupted {
    pub sys: Sys,
    pub nr: u64,
    pub args: [u64; 6],
}

impl Kernel {
    /// Sends `signal` to a process: any one of its threads not blocking it takes it.
    pub(crate) fn signal_process(&self, process: &Arc<Process>, signal: i32) {
        self.send(process, None, signal, Info::user(0));
    }

    /// Sends `signal` to one thread of a process. Stopping, continuing and killing are the whole process's still.
    pub(crate) fn signal_thread(&self, process: &Arc<Process>, tid: i32, signal: i32) {
        self.send(process, Some(tid), signal, Info { code: SI_TKILL, ..Info::default() });
    }

    /// Sends `signal` to every process in a group. Whether there were any.
    pub(crate) fn signal_group(&self, pgid: i32, signal: i32) -> bool {
        let group: Vec<Arc<Process>> = lock(&self.processes).values().filter(|p| lock(&p.state).pgid == pgid).cloned().collect();
        for process in &group {
            self.signal_process(process, signal);
        }
        !group.is_empty()
    }

    /// Sends `signal` to a process, or with `tid` to one of its threads, about `info`. A real-time signal is queued
    /// as many times as it's sent, each with its own `info`; another is pending once. False only if a real-time one
    /// couldn't be queued, as the process has `RLIMIT_SIGPENDING` of them already.
    pub(crate) fn send(&self, process: &Arc<Process>, tid: Option<i32>, signal: i32, info: Info) -> bool {
        if signal == 0 {
            return true;
        }
        let mut state = lock(&process.state);
        if state.exiting.is_some() || state.zombie.is_some() {
            return true;
        }
        if signal == SIGKILL {
            drop(state);
            process.end(Status::Killed(SIGKILL));
            return true;
        }
        if signal == SIGCONT {
            // Continuing happens as it's sent, whatever the handler: a stopped process can't take it any other way.
            state.pending &= !STOPS;
            for live in state.threads.values() {
                live.signals.pending.fetch_and(!STOPS, Ordering::AcqRel);
            }
            // A tracee `PTRACE_LISTEN` left in its group stop carries on.
            for live in state.threads.values() {
                Task::ptrace_continued(&live.trace, &live.waiter);
            }
            if state.stopped.take().is_some() {
                state.report = Some(Report::Continued);
                for live in state.threads.values() {
                    live.waiter.wake();
                }
                let ppid = state.ppid;
                drop(state);
                self.child_changed(ppid, process.pid, Change::Continued);
                state = lock(&process.state);
            }
        } else if bit(signal) & STOPS != 0 {
            state.pending &= !bit(SIGCONT);
        }
        // An ignored signal is dropped, unless something blocks it: then it waits, for `signalfd` or `sigwait`.
        let blocked = state.threads.values().any(|l| l.signals.mask.load(Ordering::Acquire) & bit(signal) != 0);
        if state.signals.ignores(signal) && !blocked {
            return true;
        }
        let limit = state.limits.get(RLIMIT_SIGPENDING).0.min(1 << 20) as usize;
        // The thread asked for, or the first that isn't blocking it.
        let target = match tid {
            Some(tid) if bit(signal) & (UNBLOCKABLE | STOPS | bit(SIGCONT)) == 0 => {
                let Some(live) = state.threads.get(&tid) else { return true };
                let pending = live.signals.pending.load(Ordering::Acquire) & bit(signal) != 0;
                if !queue(&mut lock(&live.signals.infos), pending, signal, info, limit) {
                    return false;
                }
                live.signals.pending.fetch_or(bit(signal), Ordering::AcqRel);
                Some(live)
            }
            _ => {
                let pending = state.pending & bit(signal) != 0;
                if !queue(&mut state.infos, pending, signal, info, limit) {
                    return false;
                }
                state.pending |= bit(signal);
                state.threads.values().find(|l| l.signals.mask.load(Ordering::Acquire) & bit(signal) & !UNBLOCKABLE == 0)
            }
        };
        if let Some(live) = target {
            live.waiter.interrupt();
            live.handle.preempt();
        }
        drop(state);
        // A signal blocked everywhere wakes no thread, but a `signalfd` or `sigtimedwait` waiting on it may want it.
        process.signal_watchers.notify();
        true
    }

    /// Child `pid` of `ppid` stopped, continued or ended: wake its parent's waiters, and send it SIGCHLD.
    pub(crate) fn child_changed(&self, ppid: i32, pid: i32, change: Change) {
        let Some(parent) = self.process(ppid) else { return };
        let quiet = {
            let mut state = lock(&parent.state);
            for w in state.child_waiters.drain(..) {
                w.wake();
            }
            !matches!(change, Change::Ended(_)) && state.signals.action(SIGCHLD).flags & SA_NOCLDSTOP != 0
        };
        // CLD_EXITED, CLD_KILLED, CLD_DUMPED, CLD_STOPPED, CLD_CONTINUED.
        let (code, status) = match change {
            Change::Ended(Status::Exited(code)) => (1, code),
            Change::Ended(Status::Killed(signal)) if default_of(signal) == Default::Core => (3, signal),
            Change::Ended(Status::Killed(signal)) => (2, signal),
            Change::Stopped(signal) => (5, signal),
            Change::Continued => (6, SIGCONT),
        };
        if !quiet {
            self.send(&parent, None, SIGCHLD, Info { code, pid, status, ..Info::default() });
        }
    }

    /// Thread `tid`, which `tracer` traces, stopped with `exit_code` `stopped`, or ended with `ended` (or ended, and
    /// its parent will say so: neither): wake the tracer's waits, and send it SIGCHLD.
    pub(crate) fn tracee_changed(&self, tracer: i32, tid: i32, stopped: Option<i32>, ended: Option<Status>) {
        let Some(tracer) = self.process(tracer) else { return };
        let nocldstop = {
            let mut state = lock(&tracer.state);
            for w in state.child_waiters.drain(..) {
                w.wake();
            }
            state.signals.action(SIGCHLD).flags & SA_NOCLDSTOP != 0
        };
        // CLD_TRAPPED, or as a child's end.
        let (code, status) = match (stopped, ended) {
            (Some(_), _) if nocldstop => return,
            (Some(code), _) => (4, code),
            (None, Some(Status::Exited(code))) => (1, code),
            (None, Some(Status::Killed(signal))) if default_of(signal) == Default::Core => (3, signal),
            (None, Some(Status::Killed(signal))) => (2, signal),
            (None, None) => return,
        };
        self.send(&tracer, None, SIGCHLD, Info { code, pid: tid, status, ..Info::default() });
    }
}

impl Task {
    pub(crate) fn mask(&self) -> u64 {
        self.signals.mask.load(Ordering::Acquire)
    }

    pub(crate) fn set_mask(&self, mask: u64) {
        self.signals.mask.store(mask & !UNBLOCKABLE, Ordering::Release);
    }

    /// Signals this thread could take now, given its process's pending ones.
    fn deliverable_with(&self, process: u64) -> u64 {
        (self.signals.pending.load(Ordering::Acquire) | process) & !(self.mask() & !UNBLOCKABLE)
    }

    pub(crate) fn deliverable(&self) -> u64 {
        let process = lock(&self.process.state).pending;
        self.deliverable_with(process)
    }

    /// The program did something it can't carry on from, and Linux sends it `signal` at once. If it's blocked or
    /// ignored, it's the default action instead: the program can't skip it.
    /// Sends this thread `signal` with `info`, as the kernel forces one: if it's blocked or ignored, its default action
    /// comes back and it's let through (SIGSYS from seccomp).
    pub(crate) fn force_signal(&mut self, signal: i32, info: Info) {
        {
            let mut state = lock(&self.process.state);
            let action = state.signals.action(signal);
            if action.handler == SIG_IGN || self.mask() & bit(signal) != 0 {
                state.signals.actions[(signal - 1) as usize] = Action::default();
                self.signals.mask.fetch_and(!bit(signal), Ordering::AcqRel);
            }
        }
        self.k.send(&self.process.clone(), Some(self.tid), signal, info);
    }

    pub(crate) fn fault_signal(&mut self, signal: i32, addr: u64, what: &str) {
        {
            let mut state = lock(&self.process.state);
            let action = state.signals.action(signal);
            if action.handler == SIG_IGN || self.mask() & bit(signal) != 0 {
                state.signals.actions[(signal - 1) as usize] = Action::default();
                self.signals.mask.fetch_and(!bit(signal), Ordering::AcqRel);
            }
        }
        self.signals.pending.fetch_or(bit(signal), Ordering::AcqRel);
        self.fault_info = Some((addr, what.to_string()));
    }

    /// Delivers what's pending, on the thread's way back to its program: default actions, stopping, and at most one
    /// handler. `interrupted` is the system call that just returned EINTR, which this restarts or lets fail.
    pub(crate) fn deliver(&mut self, cx: &mut Cx<'_>, interrupted: Option<Interrupted>) -> Flow {
        let mut handler = None;
        loop {
            self.waiter.clear_interrupt();
            if self.park() == Flow::End {
                return Flow::End;
            }
            // `PTRACE_INTERRUPT`, or a seized new thread's first stop.
            if self.ptrace_interrupted(cx) {
                continue;
            }
            let taken = {
                let mut state = lock(&self.process.state);
                let ready = self.deliverable_with(state.pending);
                if ready == 0 {
                    None
                } else {
                    let signal = ready.trailing_zeros() as i32 + 1;
                    let info = self.dequeue(&mut state, signal);
                    Some((signal, state.signals.action(signal), info))
                }
            };
            let Some((mut signal, mut action, mut info)) = taken else { break };
            // A traced thread stops for its tracer first, which may let the signal through, change it, or drop it.
            if signal != SIGKILL && self.traced() {
                match self.ptrace_signal(cx, signal, info) {
                    Some((s, i)) => {
                        (signal, info) = (s, i);
                        action = lock(&self.process.state).signals.action(signal);
                    }
                    None if self.ending() => return Flow::End,
                    None => continue,
                }
            }
            match action.handler {
                SIG_IGN if bit(signal) & UNBLOCKABLE == 0 => {}
                SIG_DFL | SIG_IGN => match default_of(signal) {
                    Default::Ignore | Default::Continue => {}
                    Default::Stop if self.traced() => self.ptrace_group_stop(cx, signal),
                    Default::Stop => self.stop_process(signal),
                    Default::Term | Default::Core => {
                        if self.k.options.trace || default_of(signal) == Default::Core {
                            let why = self.fault_info.take().map(|(_, what)| format!(": {what}")).unwrap_or_default();
                            self.k.host.write(host::Stream::Err, format!("lambda: process {} killed by signal {signal}{why}\n", self.process.pid).as_bytes());
                        }
                        self.process.end(Status::Killed(signal));
                        return Flow::End;
                    }
                },
                _ => {
                    handler = Some((signal, action, info));
                    break;
                }
            }
        }
        if let Some(call) = interrupted {
            let restart = match handler {
                None => true,
                Some((_, action, _)) => {
                    action.flags & SA_RESTART != 0
                        && !matches!(call.sys, Sys::Nanosleep | Sys::ClockNanosleep | Sys::Ppoll | Sys::Poll | Sys::Pselect6 | Sys::Select | Sys::EpollPwait | Sys::EpollPwait2 | Sys::RtSigsuspend | Sys::RtSigtimedwait | Sys::Pause)
                }
            };
            if restart {
                if let Some(mask) = self.suspend_mask.take() {
                    self.set_mask(mask);
                }
                cx.turn(|t| {
                    let mut regs = t.thread().regs();
                    match &mut regs {
                        Regs::Arm64(r) => {
                            r.pc -= 4;
                            r.x[0] = call.args[0];
                        }
                        Regs::X86(r) => {
                            r.rip -= 2;
                            r.gpr[RAX] = call.nr;
                        }
                    }
                    t.thread().set_regs(&regs);
                });
            } else {
                cx.turn(|t| t.thread().set_return((-(EINTR.0 as i64)) as u64));
            }
        }
        if let Some((signal, action, info)) = handler
            && self.setup_frame(cx, signal, action, info).is_err()
        {
            self.k.host.write(host::Stream::Err, format!("lambda: process {}: no room for a signal frame\n", self.process.pid).as_bytes());
            self.process.end(Status::Killed(SIGSEGV));
            return Flow::End;
        }
        Flow::Run
    }

    /// Waits while the process is stopped. End if it's ending.
    fn park(&mut self) -> Flow {
        loop {
            self.waiter.reset();
            {
                let state = lock(&self.process.state);
                if state.exiting.is_some() {
                    return Flow::End;
                }
                if state.stopped.is_none() {
                    return Flow::Run;
                }
            }
            self.waiter.wait(None);
            self.waiter.clear_interrupt();
        }
    }

    /// Stops the whole process, and tells its parent.
    fn stop_process(&mut self, signal: i32) {
        let ppid = {
            let mut state = lock(&self.process.state);
            if state.stopped.is_some() || state.exiting.is_some() {
                return;
            }
            state.stopped = Some(signal);
            state.report = Some(Report::Stopped(signal));
            for (tid, live) in &state.threads {
                if *tid != self.tid {
                    live.waiter.interrupt();
                    live.handle.preempt();
                }
            }
            state.ppid
        };
        self.k.child_changed(ppid, self.process.pid, Change::Stopped(signal));
    }

    /// Puts a frame for `signal`'s handler on the program's stack, and points the thread at the handler.
    fn setup_frame(&mut self, cx: &mut Cx<'_>, signal: i32, action: Action, info: Info) -> Result<(), Errno> {
        let (regs, fp) = cx.turn(|t| (t.thread().regs(), t.thread().fp()));
        let old_mask = self.suspend_mask.take().unwrap_or(self.mask());
        let trampoline = lock(&self.process.state).sigreturn_at;
        let restorer = if action.flags & SA_RESTORER != 0 && action.restorer != 0 { action.restorer } else { trampoline };
        let (sp, on_alt) = {
            let sp = regs.sp();
            let alt = self.altstack.filter(|a| action.flags & SA_ONSTACK != 0 && !(a.sp..a.sp + a.size).contains(&sp));
            match alt {
                Some(a) => (a.sp + a.size, true),
                None => (sp, false),
            }
        };
        let stack = match self.altstack {
            Some(a) => (a.sp, if on_alt { SS_ONSTACK } else { 0 }, a.size),
            None => (0, SS_DISABLE, 0),
        };
        let info = match self.fault_info.take() {
            Some((addr, _)) => Info { code: SI_KERNEL, addr, ..Info::default() },
            None => info,
        };
        let addr = info.addr;
        let info = info.siginfo(signal);
        let put = |b: &mut Vec<u8>, at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());

        let (frame, bytes, mut new) = match regs {
            Regs::Arm64(r) => {
                // rt_sigframe: siginfo, then ucontext (flags, link, stack, sigmask, padding, sigcontext), then the
                // frame record that x29 points at.
                const SIZE: u64 = 4704;
                let frame = (sp - SIZE) & !15;
                let mut b = vec![0u8; SIZE as usize];
                b[..128].copy_from_slice(&info);
                put(&mut b, 144, stack.0);
                b[152..156].copy_from_slice(&stack.1.to_le_bytes());
                put(&mut b, 160, stack.2);
                put(&mut b, 168, old_mask);
                put(&mut b, 304, addr);
                for (i, x) in r.x.iter().enumerate() {
                    put(&mut b, 312 + i * 8, *x);
                }
                put(&mut b, 560, r.sp);
                put(&mut b, 568, r.pc);
                put(&mut b, 576, r.nzcv);
                put(&mut b, 4688, r.x[29]);
                put(&mut b, 4696, r.x[30]);
                let mut n = r;
                n.x[0] = signal as u64;
                n.x[1] = frame;
                n.x[2] = frame + 128;
                n.x[29] = frame + 4688;
                n.x[30] = restorer;
                n.sp = frame;
                n.pc = action.handler;
                (frame, b, Regs::Arm64(n))
            }
            Regs::X86(r) => {
                // rt_sigframe: the return address, ucontext (flags, link, stack, sigcontext, sigmask), siginfo.
                const SIZE: u64 = 440;
                let frame = ((sp - 128 - SIZE) & !15) - 8;
                let mut b = vec![0u8; SIZE as usize];
                put(&mut b, 0, restorer);
                put(&mut b, 24, stack.0);
                b[32..36].copy_from_slice(&stack.1.to_le_bytes());
                put(&mut b, 40, stack.2);
                let g = &r.gpr;
                let greg = [g[R8], g[R9], g[R10], g[R11], g[12], g[13], g[14], g[15], g[RDI], g[RSI], g[RBP], g[RBX], g[RDX], g[RAX], g[RCX], g[RSP], r.rip, r.rflags, 0x33];
                for (i, v) in greg.iter().enumerate() {
                    put(&mut b, 48 + i * 8, *v);
                }
                put(&mut b, 48 + 22 * 8, old_mask); // oldmask
                put(&mut b, 48 + 23 * 8, addr); // cr2
                put(&mut b, 304, old_mask);
                b[312..440].copy_from_slice(&info);
                let mut n = r;
                n.gpr[RDI] = signal as u64;
                n.gpr[RSI] = frame + 312;
                n.gpr[RDX] = frame + 8;
                n.gpr[RAX] = 0;
                n.gpr[RSP] = frame;
                n.rip = action.handler;
                n.rflags &= !0x400; // DF
                (frame, b, Regs::X86(n))
            }
        };
        self.mem(cx, true, |t| user::write(&mut t.space(), frame, &bytes))?;
        // The FP registers wait here for `rt_sigreturn`, rather than in the frame.
        self.saved_fp.retain(|(at, _)| *at > frame);
        self.saved_fp.push((frame, fp));
        if let Regs::Arm64(n) = &mut new {
            n.nzcv &= 0xf000_0000;
        }
        cx.turn(|t| t.thread().set_regs(&new));
        let mut mask = old_mask | action.mask;
        if action.flags & SA_NODEFER == 0 {
            mask |= bit(signal);
        }
        self.set_mask(mask);
        if action.flags & SA_RESETHAND != 0 {
            lock(&self.process.state).signals.actions[(signal - 1) as usize] = Action::default();
        }
        Ok(())
    }

    /// `rt_sigreturn`: back from a handler, to where the frame says.
    pub(crate) fn rt_sigreturn(&mut self, cx: &mut Cx<'_>) -> Result<(), Errno> {
        let regs = cx.turn(|t| t.thread().regs());
        let (frame, restored, mask) = match regs {
            Regs::Arm64(r) => {
                let frame = r.sp;
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), frame, 584))?;
                let get = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default());
                let mut n = r;
                for i in 0..31 {
                    n.x[i] = get(312 + i * 8);
                }
                n.sp = get(560);
                n.pc = get(568);
                n.nzcv = get(576) & 0xf000_0000;
                (frame, Regs::Arm64(n), get(168))
            }
            Regs::X86(r) => {
                let frame = r.gpr[RSP] - 8;
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), frame, 312))?;
                let get = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default());
                let mut n = r;
                for (i, reg) in [R8, R9, R10, R11, 12, 13, 14, 15, RDI, RSI, RBP, RBX, RDX, RAX, RCX, RSP].into_iter().enumerate() {
                    n.gpr[reg] = get(48 + i * 8);
                }
                n.rip = get(48 + 16 * 8);
                n.rflags = r.rflags & !USER_FLAGS | get(48 + 17 * 8) & USER_FLAGS;
                (frame, Regs::X86(n), get(304))
            }
        };
        let fp: Option<FpRegs> = self.saved_fp.iter().rposition(|(at, _)| *at == frame).map(|i| self.saved_fp.drain(i..).next().map(|(_, fp)| fp)).and_then(|fp| fp);
        cx.turn(|t| {
            t.thread().set_regs(&restored);
            if let Some(fp) = &fp {
                t.thread().set_fp(fp);
            }
        });
        self.set_mask(mask);
        Ok(())
    }

    pub(crate) fn rt_sigaction(&mut self, cx: &mut Cx<'_>, signal: i32, new: u64, old: u64, setsize: u64) -> Result<u64, Errno> {
        if !(1..=64).contains(&signal) || setsize != 8 {
            return Err(EINVAL);
        }
        let new_action = if new != 0 { Some(Action::decode(&self.mem(cx, false, |t| user::read_vec(&t.space(), new, 32))?)) } else { None };
        let previous = {
            let mut state = lock(&self.process.state);
            let previous = state.signals.action(signal);
            if let Some(action) = new_action {
                if bit(signal) & UNBLOCKABLE != 0 {
                    return Err(EINVAL);
                }
                state.signals.actions[(signal - 1) as usize] = action;
                // Ignoring a signal drops it if it's pending.
                if state.signals.ignores(signal) {
                    state.pending &= !bit(signal);
                    state.infos.remove(&signal);
                    for live in state.threads.values() {
                        live.signals.pending.fetch_and(!bit(signal), Ordering::AcqRel);
                        lock(&live.signals.infos).remove(&signal);
                    }
                }
            }
            previous
        };
        if old != 0 {
            self.mem(cx, true, |t| user::write(&mut t.space(), old, &previous.encode()))?;
        }
        Ok(0)
    }

    pub(crate) fn rt_sigprocmask(&mut self, cx: &mut Cx<'_>, how: u64, set: u64, old: u64) -> Result<u64, Errno> {
        let previous = self.mask();
        if set != 0 {
            let change = self.mem(cx, false, |t| user::read_u64(&t.space(), set))?;
            self.set_mask(match how {
                0 => previous | change,
                1 => previous & !change,
                2 => change,
                _ => return Err(EINVAL),
            });
        }
        if old != 0 {
            self.mem(cx, true, |t| user::write_u64(&mut t.space(), old, previous))?;
        }
        Ok(0)
    }

    pub(crate) fn rt_sigpending(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        let pending = (self.signals.pending.load(Ordering::Acquire) | lock(&self.process.state).pending) & self.mask();
        self.mem(cx, true, |t| user::write_u64(&mut t.space(), at, pending))?;
        Ok(0)
    }

    /// Takes one pending signal in `mask`, for `signalfd`: its `signalfd_siginfo`.
    pub(crate) fn take_signal(&self, mask: u64) -> Option<[u8; 128]> {
        let mut state = lock(&self.process.state);
        let ready = (self.signals.pending.load(Ordering::Acquire) | state.pending) & mask;
        if ready == 0 {
            return None;
        }
        let signal = ready.trailing_zeros() as i32 + 1;
        Some(self.dequeue(&mut state, signal).signalfd(signal))
    }

    /// Takes `signal` off this thread's queue, or else its process's, with what it's about.
    pub(crate) fn dequeue(&self, state: &mut ProcState, signal: i32) -> Info {
        let mut info = if self.signals.pending.load(Ordering::Acquire) & bit(signal) != 0 {
            let mut infos = lock(&self.signals.infos);
            let info = take(&mut infos, signal);
            if !infos.contains_key(&signal) {
                self.signals.pending.fetch_and(!bit(signal), Ordering::AcqRel);
            }
            info
        } else {
            let info = take(&mut state.infos, signal);
            if !state.infos.contains_key(&signal) {
                state.pending &= !bit(signal);
            }
            info
        };
        crate::timer::delivered(state, &mut info);
        info
    }

    /// A signal in `set` that's pending now, taken, with what it's about.
    fn take_one(&self, set: u64) -> Option<(i32, Info)> {
        let mut state = lock(&self.process.state);
        let ready = (self.signals.pending.load(Ordering::Acquire) | state.pending) & set;
        if ready == 0 {
            return None;
        }
        let signal = ready.trailing_zeros() as i32 + 1;
        Some((signal, self.dequeue(&mut state, signal)))
    }

    /// `rt_sigtimedwait`: takes a signal in the set at `set_at` (normally blocked), waiting for one until the timeout
    /// (none: for ever; zero: not at all). Gives the signal, and its `siginfo` at `info_at`. EAGAIN if none came, EINTR
    /// if another signal, not blocked, came first.
    pub(crate) fn rt_sigtimedwait(&mut self, cx: &mut Cx<'_>, set_at: u64, info_at: u64, timeout_at: u64, setsize: u64) -> Result<u64, Errno> {
        if setsize != 8 {
            return Err(EINVAL);
        }
        let set = self.mem(cx, false, |t| user::read_u64(&t.space(), set_at))? & !UNBLOCKABLE;
        let deadline = self.timespec_timeout(cx, timeout_at)?.map(|d| std::time::Instant::now() + d);
        // A signal every thread blocks wakes none; the process's signal watchers say it came.
        let me: std::sync::Weak<dyn kernels_core::wait::Wake> = Arc::downgrade(&self.waiter) as _;
        self.process.signal_watchers.add(me.clone());
        let result = loop {
            self.waiter.reset();
            if let Some(found) = self.take_one(set) {
                break Ok(found);
            }
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                break Err(EAGAIN);
            }
            if self.waiter.wait(deadline) == kernels_core::wait::Woke::Interrupted {
                // Interrupted for a signal it's waiting for (sent to this thread), or for one to deliver, or to end.
                if self.ending() || self.deliverable() != 0 {
                    break Err(EINTR);
                }
                self.waiter.clear_interrupt();
            }
        };
        self.process.signal_watchers.remove(&me);
        let (signal, info) = result?;
        if info_at != 0 {
            self.mem(cx, true, |t| user::write(&mut t.space(), info_at, &info.siginfo(signal)))?;
        }
        Ok(signal as u64)
    }

    /// `rt_sigqueueinfo`, and with `tid` `rt_tgsigqueueinfo`: a signal with the sender's own `siginfo`, whose code must
    /// say it's from a program (negative) unless it's for this process.
    pub(crate) fn rt_sigqueueinfo(&mut self, cx: &mut Cx<'_>, tgid: i32, tid: Option<i32>, signal: i32, at: u64) -> Result<u64, Errno> {
        if !(0..=64).contains(&signal) || tgid <= 0 || tid.is_some_and(|t| t <= 0) {
            return Err(EINVAL);
        }
        let info = self.read_siginfo(cx, at)?;
        let process = self.k.process(tgid).ok_or(ESRCH)?;
        if tid.is_some_and(|t| !lock(&process.state).threads.contains_key(&t)) {
            return Err(ESRCH);
        }
        self.queue_to(&process, tid, signal, info)
    }

    /// A `siginfo` a program gives, for a signal it sends: its code, sender and value.
    pub(crate) fn read_siginfo(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Info, Errno> {
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, 32))?;
        let i32_at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
        Ok(Info { code: i32_at(8), pid: i32_at(16), uid: i32_at(20) as u32, value: u64::from_le_bytes(b[24..32].try_into().unwrap_or_default()), ..Info::default() })
    }

    /// Sends a signal with a `siginfo` from the program, checking what it claims: a code of the kernel's (0 or more,
    /// or `SI_TKILL`) only to itself. EAGAIN if the queue's full.
    pub(crate) fn queue_to(&mut self, process: &Arc<Process>, tid: Option<i32>, signal: i32, info: Info) -> Result<u64, Errno> {
        if (info.code >= 0 || info.code == SI_TKILL) && process.pid != self.process.pid {
            return Err(EPERM);
        }
        if lock(&process.state).zombie.is_some() {
            return Err(ESRCH);
        }
        if signal != 0 && !self.k.send(process, tid, signal, info) {
            return Err(EAGAIN);
        }
        Ok(0)
    }

    /// x86-64's `pause`: waits for a signal that runs a handler or ends the process.
    pub(crate) fn pause(&mut self) -> Result<u64, Errno> {
        while self.deliverable() == 0 && !self.ending() {
            if self.waiter.wait(None) == kernels_core::wait::Woke::Interrupted {
                break;
            }
        }
        Err(EINTR)
    }

    /// Whether a signal in `mask` is pending.
    pub(crate) fn has_signal(&self, mask: u64) -> bool {
        let state = lock(&self.process.state);
        (self.signals.pending.load(Ordering::Acquire) | state.pending) & mask != 0
    }

    /// `rt_sigsuspend`: waits with this mask until a signal comes. The mask is put back when its handler returns.
    pub(crate) fn rt_sigsuspend(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        let mask = self.mem(cx, false, |t| user::read_u64(&t.space(), at))?;
        self.suspend_mask = Some(self.mask());
        self.set_mask(mask);
        while self.deliverable() == 0 {
            if self.waiter.wait(None) == kernels_core::wait::Woke::Interrupted {
                break;
            }
        }
        Err(EINTR)
    }

    pub(crate) fn sigaltstack(&mut self, cx: &mut Cx<'_>, new: u64, old: u64) -> Result<u64, Errno> {
        let sp = cx.turn(|t| t.thread().regs().sp());
        let on = self.altstack.is_some_and(|a| (a.sp..a.sp + a.size).contains(&sp));
        if old != 0 {
            let (base, flags, size) = match self.altstack {
                Some(a) => (a.sp, if on { SS_ONSTACK } else { 0 }, a.size),
                None => (0, SS_DISABLE, 0),
            };
            let mut b = base.to_le_bytes().to_vec();
            b.extend(flags.to_le_bytes());
            b.extend([0; 4]);
            b.extend(size.to_le_bytes());
            self.mem(cx, true, |t| user::write(&mut t.space(), old, &b))?;
        }
        if new != 0 {
            if on {
                return Err(EPERM);
            }
            let b = self.mem(cx, false, |t| user::read_vec(&t.space(), new, 24))?;
            let get = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default());
            self.altstack = if get(8) as u32 & SS_DISABLE != 0 { None } else { Some(AltStack { sp: get(0), size: get(16) }) };
        }
        Ok(0)
    }

    pub(crate) fn kill(&mut self, pid: i32, signal: i32) -> Result<u64, Errno> {
        if !(0..=64).contains(&signal) {
            return Err(EINVAL);
        }
        let targets: Vec<Arc<Process>> = match pid {
            1.. => vec![self.k.process(pid).ok_or(ESRCH)?],
            -1 => lock(&self.k.processes).values().filter(|p| p.pid != 1 && p.pid != self.process.pid).cloned().collect(),
            _ => {
                let group = if pid == 0 { lock(&self.process.state).pgid } else { -pid };
                lock(&self.k.processes).values().filter(|p| lock(&p.state).pgid == group).cloned().collect()
            }
        };
        let live: Vec<_> = targets.into_iter().filter(|p| lock(&p.state).zombie.is_none() || signal == 0).collect();
        if live.is_empty() {
            return Err(ESRCH);
        }
        let uid = lock(&self.process.state).creds.uid[0];
        for target in live {
            self.k.send(&target, None, signal, Info { uid, ..Info::user(self.process.pid) });
        }
        Ok(0)
    }

    /// `tkill` and `tgkill`: a signal for one thread. `tgid` is None for `tkill`.
    pub(crate) fn tgkill(&mut self, tgid: Option<i32>, tid: i32, signal: i32) -> Result<u64, Errno> {
        if !(0..=64).contains(&signal) {
            return Err(EINVAL);
        }
        let process = match tgid {
            Some(tgid) => self.k.process(tgid).filter(|p| lock(&p.state).threads.contains_key(&tid)),
            None => lock(&self.k.processes).values().find(|p| lock(&p.state).threads.contains_key(&tid)).cloned(),
        }
        .ok_or(ESRCH)?;
        self.k.signal_thread(&process, tid, signal);
        Ok(0)
    }
}

/// The code `rt_sigreturn` is reached by when a handler returns and the program gave no restorer of its own: on a
/// page of its own in every process, as the vDSO's is on Linux.
pub(crate) fn trampoline(isa: exec::Isa) -> Vec<u8> {
    match isa {
        // movz x8, #139; svc #0
        exec::Isa::Arm64 => [0xd280_1168u32, 0xd400_0001].iter().flat_map(|w| w.to_le_bytes()).collect(),
        // mov eax, 15; syscall
        _ => vec![0xb8, 15, 0, 0, 0, 0x0f, 0x05],
    }
}
