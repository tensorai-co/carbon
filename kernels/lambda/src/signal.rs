//! Signals: what each one does, sending them to processes, groups and threads, and delivering them to a thread on its
//! way back to its program, with a frame on the program's stack for a handler and `rt_sigreturn` to come back.
//!
//! A signal is delivered at a thread's next stop: a system call ending, a fault, or a preempt, which sending one makes
//! happen. A system call a signal interrupted is restarted, unless a handler without `SA_RESTART` ran.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use exec::isa::x86::{R8, R9, R10, R11, RAX, RBP, RBX, RCX, RDI, RDX, RSI, RSP, USER_FLAGS};
use exec::{FpRegs, Regs};
use kernels_core::lock;
use kernels_core::thread::{Cx, Flow};
use kernels_core::user;

use crate::abi::*;
use crate::process::{Process, Report, Task};
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
const SI_USER: i32 = 0;
const SI_KERNEL: i32 = 0x80;
const SI_TKILL: i32 = -6;

pub const fn bit(signal: i32) -> u64 {
    1 << (signal - 1)
}

/// SIGKILL and SIGSTOP can't be blocked, caught or ignored.
const UNBLOCKABLE: u64 = bit(SIGKILL) | bit(SIGSTOP);
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

/// A thread's pending signals and mask, shared with whoever sends it signals.
#[derive(Debug, Default)]
pub(crate) struct ThreadSignals {
    pub pending: AtomicU64,
    pub mask: AtomicU64,
}

impl ThreadSignals {
    pub fn with_mask(mask: u64) -> Arc<ThreadSignals> {
        Arc::new(ThreadSignals { pending: AtomicU64::new(0), mask: AtomicU64::new(mask) })
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
    /// Who sent it, or which child it's about.
    pub pid: i32,
    /// A child's exit code or signal.
    pub status: i32,
    /// Where a fault was.
    pub addr: u64,
}

impl Info {
    pub fn user(pid: i32) -> Info {
        Info { code: SI_USER, pid, ..Info::default() }
    }

    /// `siginfo_t`, as a handler gets it.
    fn siginfo(&self, signal: i32) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&signal.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        if self.addr != 0 {
            b[16..24].copy_from_slice(&self.addr.to_le_bytes());
        } else {
            b[16..20].copy_from_slice(&self.pid.to_le_bytes());
            b[24..28].copy_from_slice(&self.status.to_le_bytes());
        }
        b
    }

    /// `struct signalfd_siginfo`, as `signalfd` gives it.
    fn signalfd(&self, signal: i32) -> [u8; 128] {
        let mut b = [0u8; 128];
        b[0..4].copy_from_slice(&signal.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        b[12..16].copy_from_slice(&self.pid.to_le_bytes());
        b[40..44].copy_from_slice(&self.status.to_le_bytes());
        b[72..80].copy_from_slice(&self.addr.to_le_bytes());
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

    pub(crate) fn send(&self, process: &Arc<Process>, tid: Option<i32>, signal: i32, info: Info) {
        if signal == 0 {
            return;
        }
        let mut state = lock(&process.state);
        if state.exiting.is_some() || state.zombie.is_some() {
            return;
        }
        if signal == SIGKILL {
            drop(state);
            process.end(Status::Killed(SIGKILL));
            return;
        }
        if signal == SIGCONT {
            // Continuing happens as it's sent, whatever the handler: a stopped process can't take it any other way.
            state.pending &= !STOPS;
            for live in state.threads.values() {
                live.signals.pending.fetch_and(!STOPS, Ordering::AcqRel);
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
            return;
        }
        // The thread asked for, or the first that isn't blocking it.
        let target = match tid {
            Some(tid) if bit(signal) & (UNBLOCKABLE | STOPS | bit(SIGCONT)) == 0 => {
                let Some(live) = state.threads.get(&tid) else { return };
                live.signals.pending.fetch_or(bit(signal), Ordering::AcqRel);
                Some(live)
            }
            _ => {
                state.pending |= bit(signal);
                state.infos.insert(signal, info);
                state.threads.values().find(|l| l.signals.mask.load(Ordering::Acquire) & bit(signal) & !UNBLOCKABLE == 0)
            }
        };
        if let Some(live) = target {
            live.waiter.interrupt();
            live.handle.preempt();
        }
        drop(state);
        // A signal blocked everywhere wakes no thread, but a `signalfd` being waited on may want it.
        process.signal_watchers.notify();
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
            self.send(&parent, None, SIGCHLD, Info { code, pid, status, addr: 0 });
        }
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
            let taken = {
                let mut state = lock(&self.process.state);
                let ready = self.deliverable_with(state.pending);
                if ready == 0 {
                    None
                } else {
                    let signal = ready.trailing_zeros() as i32 + 1;
                    let mut info = Info::user(0);
                    if self.signals.pending.fetch_and(!bit(signal), Ordering::AcqRel) & bit(signal) == 0 {
                        state.pending &= !bit(signal);
                        info = state.infos.remove(&signal).unwrap_or(info);
                    }
                    Some((signal, state.signals.action(signal), info))
                }
            };
            let Some((signal, action, info)) = taken else { break };
            match action.handler {
                SIG_IGN if bit(signal) & UNBLOCKABLE == 0 => {}
                SIG_DFL | SIG_IGN => match default_of(signal) {
                    Default::Ignore | Default::Continue => {}
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
                Some((_, action, _)) => action.flags & SA_RESTART != 0 && !matches!(call.sys, Sys::Nanosleep | Sys::ClockNanosleep | Sys::Ppoll | Sys::Poll | Sys::Pselect6 | Sys::Select | Sys::EpollPwait | Sys::EpollPwait2 | Sys::RtSigsuspend),
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
                    for live in state.threads.values() {
                        live.signals.pending.fetch_and(!bit(signal), Ordering::AcqRel);
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
        let mut info = Info::user(0);
        if self.signals.pending.fetch_and(!bit(signal), Ordering::AcqRel) & bit(signal) == 0 {
            state.pending &= !bit(signal);
            info = state.infos.remove(&signal).unwrap_or(info);
        }
        Some(info.signalfd(signal))
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
        for target in live {
            self.k.send(&target, None, signal, Info::user(self.process.pid));
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
