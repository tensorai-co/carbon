//! `ptrace`: one process tracing another's threads, for strace and debuggers. A traced thread stops where Linux's
//! would — at a signal about to be delivered, at each system call's entry and exit (`PTRACE_SYSCALL`), at the events
//! its tracer asked for (fork, vfork, clone, exec, exit), at a group stop, and when interrupted — and its tracer hears
//! of each stop through `wait4`/`waitid`, as of a child's, whether or not it's the parent.
//!
//! A thread's registers are only its own CPU's, so a stopped thread puts a copy of them where its tracer can read and
//! change them, and takes back what was changed when it's resumed. Its memory is its process's space, which the
//! tracer reads and writes in turns of its own (`Cx::elsewhere`), filling pages as the tracee's touch would; pokes to
//! code that can't be written go to a private copy of the page, as Linux's do.
//!
//! Not yet: single-stepping (`PTRACE_SINGLESTEP` and `PTRACE_SYSEMU*` are EIO, as there's no trap after one
//! instruction); `PTRACE_O_TRACESECCOMP`'s stops (`SECCOMP_RET_TRACE` is still ENOSYS); `PTRACE_EVENT_VFORK_DONE`
//! (a vfork's parent doesn't wait); hardware breakpoints and watchpoints; `PTRACE_PEEKSIGINFO` and the seccomp-filter
//! requests. A group stop of a traced thread is its own, not its whole process's.

use std::sync::{Arc, Mutex};

use exec::isa::x86::{RAX, USER_FLAGS};
use exec::{FpRegs, Isa, Regs};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::Waiter;

use super::{Process, Task};
use crate::Status;
use crate::abi::*;
use crate::signal::{Info, SI_USER, SIGSTOP, bit};

const PTRACE_TRACEME: u64 = 0;
const PTRACE_PEEKTEXT: u64 = 1;
const PTRACE_PEEKDATA: u64 = 2;
const PTRACE_PEEKUSR: u64 = 3;
const PTRACE_POKETEXT: u64 = 4;
const PTRACE_POKEDATA: u64 = 5;
const PTRACE_POKEUSR: u64 = 6;
const PTRACE_CONT: u64 = 7;
const PTRACE_KILL: u64 = 8;
const PTRACE_SINGLESTEP: u64 = 9;
const PTRACE_GETREGS: u64 = 12;
const PTRACE_SETREGS: u64 = 13;
const PTRACE_GETFPREGS: u64 = 14;
const PTRACE_SETFPREGS: u64 = 15;
const PTRACE_ATTACH: u64 = 16;
const PTRACE_DETACH: u64 = 17;
const PTRACE_SYSCALL: u64 = 24;
const PTRACE_ARCH_PRCTL: u64 = 30;
const PTRACE_SYSEMU: u64 = 31;
const PTRACE_SYSEMU_SINGLESTEP: u64 = 32;
const PTRACE_SETOPTIONS: u64 = 0x4200;
const PTRACE_GETEVENTMSG: u64 = 0x4201;
const PTRACE_GETSIGINFO: u64 = 0x4202;
const PTRACE_SETSIGINFO: u64 = 0x4203;
const PTRACE_GETREGSET: u64 = 0x4204;
const PTRACE_SETREGSET: u64 = 0x4205;
const PTRACE_SEIZE: u64 = 0x4206;
const PTRACE_INTERRUPT: u64 = 0x4207;
const PTRACE_LISTEN: u64 = 0x4208;
const PTRACE_GETSIGMASK: u64 = 0x420a;
const PTRACE_SETSIGMASK: u64 = 0x420b;
const PTRACE_GET_SYSCALL_INFO: u64 = 0x420e;
const PTRACE_GET_RSEQ_CONFIGURATION: u64 = 0x420f;

const PTRACE_O_TRACESYSGOOD: u64 = 1;
const PTRACE_O_TRACEFORK: u64 = 2;
const PTRACE_O_TRACEVFORK: u64 = 4;
const PTRACE_O_TRACECLONE: u64 = 8;
const PTRACE_O_TRACEEXEC: u64 = 0x10;
const PTRACE_O_TRACEEXIT: u64 = 0x40;
const PTRACE_O_EXITKILL: u64 = 1 << 20;
const PTRACE_O_SUSPEND_SECCOMP: u64 = 1 << 21;
/// Every option Linux knows (`PTRACE_O_MASK`).
const OPTIONS: u64 = 0xff | PTRACE_O_EXITKILL | PTRACE_O_SUSPEND_SECCOMP;

pub(crate) const PTRACE_EVENT_FORK: i32 = 1;
pub(crate) const PTRACE_EVENT_VFORK: i32 = 2;
pub(crate) const PTRACE_EVENT_CLONE: i32 = 3;
const PTRACE_EVENT_EXEC: i32 = 4;
const PTRACE_EVENT_EXIT: i32 = 6;
const PTRACE_EVENT_STOP: i32 = 128;

/// `ptrace_message` at a system call's stops.
const EVENTMSG_SYSCALL_ENTRY: u64 = 1;
const EVENTMSG_SYSCALL_EXIT: u64 = 2;

const SIGTRAP: i32 = 5;
const CLONE_UNTRACED: u64 = 0x0080_0000;
const CAP_SYS_PTRACE: u32 = 19;
const CAP_SYS_ADMIN: u32 = 21;
const NT_PRSTATUS: u64 = 1;
const NT_PRFPREG: u64 = 2;
const NT_X86_XSTATE: u64 = 0x202;
const NT_ARM_TLS: u64 = 0x401;
const NT_ARM_SYSTEM_CALL: u64 = 0x404;
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;
const AUDIT_ARCH_AARCH64: u32 = 0xc000_00b7;
/// `struct user`'s size on x86-64, and where its debug registers are.
const X86_USER_SIZE: u64 = 912;
const X86_USER_DEBUGREG: u64 = 848;

/// A thread's tracing: shared by the thread and its tracer.
#[derive(Default)]
pub(crate) struct Trace(pub Mutex<Traced>);

#[derive(Default)]
pub(crate) struct Traced {
    /// The tracing process.
    pub tracer: Option<i32>,
    /// Attached with `PTRACE_SEIZE`: no SIGSTOP to start, `PTRACE_INTERRUPT` and `PTRACE_LISTEN`, and group stops
    /// that say so.
    seized: bool,
    options: u64,
    /// Resumed with `PTRACE_SYSCALL`: stops at each system call's entry and exit.
    syscalls: bool,
    /// The stop it's in now, if any.
    stop: Option<Stopped>,
    /// How its tracer resumed it, for it to take.
    resume: Option<i32>,
    /// A `PTRACE_EVENT_STOP` to make at its next chance: `PTRACE_INTERRUPT`'s, or a seized new thread's first.
    interrupt: bool,
    /// `PTRACE_LISTEN`: still stopped, until SIGCONT.
    listening: bool,
    /// `PTRACE_GETEVENTMSG`'s.
    eventmsg: u64,
    /// The thread has ended, and its tracer hasn't heard yet.
    pub ended: Option<Status>,
}

/// A stop a traced thread is in.
struct Stopped {
    kind: Kind,
    /// `exit_code`, as Linux calls it: what `wait4` puts above 0x7f, the signal and an event.
    code: i32,
    /// Told to the tracer through a wait.
    reported: bool,
    regs: Regs,
    fp: FpRegs,
    regs_set: bool,
    fp_set: bool,
    /// x86-64's `orig_rax`, ARM64's `syscallno`: the system call stopped at, or -1.
    nr: i64,
    /// The signal it's about to take, and what it's about (`PTRACE_GETSIGINFO`).
    info: Info,
    signal: i32,
    /// At a system call's entry, its arguments; at its exit, its result.
    call: Option<Call>,
    /// ARM64's x7, which reads as 0 at a call's entry and 1 at its exit, as Linux shows it, and is put back after.
    x7: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Signal,
    Group,
    Syscall,
    Event,
}

#[derive(Debug, Clone, Copy)]
enum Call {
    Entry([u64; 6]),
    Exit(u64),
}

/// A thread a process traces, as its tracer keeps it: found again for its stops and its end, even once it's gone.
#[derive(Clone)]
pub(crate) struct Tracing {
    pub tid: i32,
    pub process: Arc<Process>,
    pub trace: Arc<Trace>,
    pub waiter: Arc<Waiter>,
}

/// What a stop came back with.
struct Resumed {
    signal: i32,
    info: Info,
    nr: i64,
    regs_set: bool,
}

/// What a stop is about.
struct Stopping {
    kind: Kind,
    code: i32,
    signal: i32,
    info: Info,
    nr: i64,
    call: Option<Call>,
}

impl Stopping {
    fn event(event: i32) -> Stopping {
        let code = SIGTRAP | event << 8;
        Stopping { kind: Kind::Event, code, signal: SIGTRAP, info: Info { code, ..Info::default() }, nr: -1, call: None }
    }
}

/// What a tracer's wait says of a stopped tracee, in `wait4`'s status.
pub(crate) fn wait_status(code: i32) -> i32 {
    code << 8 | 0x7f
}

impl Trace {
    /// The stop its tracer hasn't heard of yet, as its `exit_code`, marking it heard unless `peek`.
    pub(crate) fn report(&self, peek: bool) -> Option<i32> {
        let mut t = lock(&self.0);
        let stop = t.stop.as_mut().filter(|s| !s.reported)?;
        if !peek {
            stop.reported = true;
        }
        Some(stop.code)
    }

    /// Whether it's in a stop now (for `/proc`'s "t (tracing stop)").
    pub(crate) fn stopped(&self) -> bool {
        lock(&self.0).stop.is_some()
    }
}

/// Lets go of a tracee: it carries on, with `signal` if it was stopped.
fn untrace(tracee: &Tracing, signal: i32) {
    {
        let mut t = lock(&tracee.trace.0);
        t.tracer = None;
        t.seized = false;
        t.options = 0;
        t.syscalls = false;
        t.interrupt = false;
        t.listening = false;
        if t.stop.is_some() {
            t.resume = Some(signal);
        }
    }
    tracee.waiter.wake();
}

/// x86-64's `struct user_regs_struct`, in its order: r15 down to r8 by way of rbp and rbx, then rax, rcx, rdx, rsi,
/// rdi, orig_rax, rip, cs, eflags, rsp, ss, fs_base, gs_base, ds, es, fs, gs.
const X86_USER_ORDER: [usize; 15] = [15, 14, 13, 12, 5, 3, 11, 10, 9, 8, 0, 1, 2, 6, 7];

fn x86_user_regs(r: &exec::isa::x86::X86Regs, nr: i64) -> [u64; 27] {
    let mut u = [0; 27];
    for (i, &g) in X86_USER_ORDER.iter().enumerate() {
        u[i] = r.gpr[g];
    }
    u[15] = nr as u64;
    u[16] = r.rip;
    u[17] = 0x33;
    u[18] = r.rflags;
    u[19] = r.gpr[4];
    u[20] = 0x2b;
    u[21] = r.fs_base;
    u[22] = r.gs_base;
    u
}

fn set_x86_user_regs(r: &mut exec::isa::x86::X86Regs, nr: &mut i64, u: &[u64; 27]) {
    for (i, &g) in X86_USER_ORDER.iter().enumerate() {
        r.gpr[g] = u[i];
    }
    *nr = u[15] as i64;
    r.rip = u[16];
    r.rflags = r.rflags & !USER_FLAGS | u[18] & USER_FLAGS;
    r.gpr[4] = u[19];
    r.fs_base = u[21];
    r.gs_base = u[22];
}

fn words(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    bytes.as_chunks::<8>().0.iter().map(|w| u64::from_le_bytes(*w))
}

fn bytes(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A register set, as `PTRACE_GETREGSET` gives it.
fn regset(stop: &Stopped, kind: u64) -> Result<Vec<u8>, Errno> {
    match (kind, &stop.regs, &stop.fp) {
        (NT_PRSTATUS, Regs::Arm64(r), _) => {
            let mut w = r.x.to_vec();
            w.extend([r.sp, r.pc, r.nzcv]);
            Ok(bytes(&w))
        }
        (NT_PRSTATUS, Regs::X86(r), _) => Ok(bytes(&x86_user_regs(r, stop.nr))),
        (NT_PRFPREG, _, FpRegs::Arm64(f)) => {
            let mut b: Vec<u8> = f.v.iter().flat_map(|v| v.to_le_bytes()).collect();
            b.extend(f.fpsr.to_le_bytes());
            b.extend(f.fpcr.to_le_bytes());
            b.extend([0; 8]);
            Ok(b)
        }
        (NT_PRFPREG, _, FpRegs::X86(f)) => Ok(f.fxsave.to_vec()),
        // The XSAVE area of a CPU with only x87 and SSE state: the legacy area, with `xcr0` in its software-reserved
        // bytes (where gdb looks), and a header saying both are there.
        (NT_X86_XSTATE, _, FpRegs::X86(f)) => {
            let mut b = f.fxsave.to_vec();
            b[464..472].copy_from_slice(&3u64.to_le_bytes());
            b.extend([0; 64]);
            b[512..520].copy_from_slice(&3u64.to_le_bytes());
            Ok(b)
        }
        (NT_ARM_TLS, Regs::Arm64(r), _) => Ok(r.tpidr.to_le_bytes().to_vec()),
        (NT_ARM_SYSTEM_CALL, Regs::Arm64(_), _) => Ok((stop.nr as i32).to_le_bytes().to_vec()),
        _ => Err(EINVAL),
    }
}

/// Changes a register set from `b`, which is all of it (what wasn't given is as it was).
fn set_regset(stop: &mut Stopped, kind: u64, b: &[u8]) -> Result<(), Errno> {
    match (kind, &mut stop.regs, &mut stop.fp) {
        (NT_PRSTATUS, Regs::Arm64(r), _) => {
            let w: Vec<u64> = words(b).collect();
            r.x.copy_from_slice(&w[..31]);
            (r.sp, r.pc) = (w[31], w[32]);
            r.nzcv = w[33] & exec::isa::arm64::NZCV;
            stop.regs_set = true;
        }
        (NT_PRSTATUS, Regs::X86(r), _) => {
            let w: Vec<u64> = words(b).collect();
            set_x86_user_regs(r, &mut stop.nr, w.as_slice().try_into().map_err(|_| EINVAL)?);
            stop.regs_set = true;
        }
        (NT_PRFPREG, _, FpRegs::Arm64(f)) => {
            for (i, v) in b[..512].as_chunks::<16>().0.iter().enumerate() {
                f.v[i] = u128::from_le_bytes(*v);
            }
            f.fpsr = u32::from_le_bytes(b[512..516].try_into().unwrap_or_default());
            f.fpcr = u32::from_le_bytes(b[516..520].try_into().unwrap_or_default());
            stop.fp_set = true;
        }
        (NT_PRFPREG | NT_X86_XSTATE, _, FpRegs::X86(f)) => {
            f.fxsave.copy_from_slice(&b[..512]);
            stop.fp_set = true;
        }
        (NT_ARM_TLS, Regs::Arm64(r), _) => {
            r.tpidr = u64::from_le_bytes(b[..8].try_into().unwrap_or_default());
            stop.regs_set = true;
        }
        (NT_ARM_SYSTEM_CALL, Regs::Arm64(_), _) => stop.nr = i64::from(i32::from_le_bytes(b[..4].try_into().unwrap_or_default())),
        _ => return Err(EINVAL),
    }
    Ok(())
}

impl Task {
    fn tracer(&self) -> Option<i32> {
        lock(&self.trace.0).tracer
    }

    pub(crate) fn traced(&self) -> bool {
        self.tracer().is_some()
    }

    /// Whether this thread stops at each system call's entry and exit.
    pub(crate) fn tracing_syscalls(&self) -> bool {
        let t = lock(&self.trace.0);
        t.tracer.is_some() && t.syscalls
    }

    /// Whether its tracer asked to hear of this event.
    fn traces(&self, option: u64) -> bool {
        let t = lock(&self.trace.0);
        t.tracer.is_some() && t.options & option != 0
    }

    /// Stops for its tracer, until it's resumed: None if it's ending (or isn't traced).
    fn ptrace_stop(&mut self, cx: &mut Cx<'_>, stopping: Stopping) -> Option<Resumed> {
        let (mut regs, fp) = cx.turn(|t| (t.thread().regs(), t.thread().fp()));
        let mut x7 = None;
        match (&mut regs, stopping.call) {
            (Regs::Arm64(r), Some(call)) => {
                x7 = Some(r.x[7]);
                r.x[7] = u64::from(matches!(call, Call::Exit(_)));
            }
            // At a call's entry, rax reads as -ENOSYS: the call is in orig_rax.
            (Regs::X86(r), Some(Call::Entry(_))) => r.gpr[RAX] = (-(ENOSYS.0 as i64)) as u64,
            _ => {}
        }
        let tracer = {
            let mut t = lock(&self.trace.0);
            let tracer = t.tracer?;
            t.resume = None;
            t.eventmsg = match stopping.call {
                Some(Call::Entry(_)) => EVENTMSG_SYSCALL_ENTRY,
                Some(Call::Exit(_)) => EVENTMSG_SYSCALL_EXIT,
                None => t.eventmsg,
            };
            let Stopping { kind, code, signal, info, nr, call } = stopping;
            t.stop = Some(Stopped { kind, code, reported: false, regs, fp, regs_set: false, fp_set: false, nr, info, signal, call, x7 });
            tracer
        };
        self.k.tracee_changed(tracer, self.tid, Some(stopping.code), None);
        let resumed = loop {
            self.waiter.reset();
            if self.ending() {
                break None;
            }
            {
                let mut t = lock(&self.trace.0);
                if let Some(signal) = t.resume.take() {
                    break Some(signal);
                }
            }
            self.waiter.wait(None);
            self.waiter.clear_interrupt();
        };
        let stop = lock(&self.trace.0).stop.take();
        // A signal that came meanwhile interrupts what it does next.
        if self.deliverable() != 0 {
            self.waiter.interrupt();
        }
        let (signal, stop) = (resumed?, stop?);
        if stop.regs_set {
            let mut regs = stop.regs;
            if let (Regs::Arm64(r), Some(x7)) = (&mut regs, stop.x7) {
                r.x[7] = x7;
            }
            cx.turn(|t| t.thread().set_regs(&regs));
        }
        if stop.fp_set {
            cx.turn(|t| t.thread().set_fp(&stop.fp));
        }
        Some(Resumed { signal, info: stop.info, nr: stop.nr, regs_set: stop.regs_set })
    }

    /// A signal its tracer gave back at a system call's or an event's stop is sent to it, as Linux does.
    fn resend(&mut self, resumed: Option<Resumed>) {
        if let Some(r) = resumed
            && r.signal != 0
        {
            let info = Info { code: SI_USER, pid: self.tracer().unwrap_or(0), ..Info::default() };
            self.k.send(&self.process.clone(), Some(self.tid), r.signal, info);
        }
    }

    /// A system call's entry, for its tracer: the call to make, which the tracer may have changed (from the registers
    /// and orig_rax or syscallno), or None to skip it (-1), with the result as the registers have it.
    pub(crate) fn syscall_entry_stop(&mut self, cx: &mut Cx<'_>, nr: u64, args: [u64; 6]) -> Option<(u64, [u64; 6])> {
        let code = self.syscall_code();
        let info = Info { code, pid: self.tid, ..Info::default() };
        let resumed = self.ptrace_stop(cx, Stopping { kind: Kind::Syscall, code, signal: SIGTRAP, info, nr: nr as i64, call: Some(Call::Entry(args)) })?;
        let (new, changed) = (resumed.nr, resumed.regs_set || resumed.nr != nr as i64);
        self.resend(Some(resumed));
        if new < 0 {
            return None;
        }
        if !changed {
            return Some((nr, args));
        }
        let regs = cx.turn(|t| t.thread().regs());
        let args = match regs {
            Regs::Arm64(r) => [r.x[0], r.x[1], r.x[2], r.x[3], r.x[4], r.x[5]],
            Regs::X86(r) => [r.gpr[7], r.gpr[6], r.gpr[2], r.gpr[10], r.gpr[8], r.gpr[9]],
        };
        Some((new as u64, args))
    }

    /// A system call's exit, for its tracer: its result is in the registers already, for the tracer to change.
    pub(crate) fn syscall_exit_stop(&mut self, cx: &mut Cx<'_>, nr: u64) {
        let code = self.syscall_code();
        let result = cx.turn(|t| match t.thread().regs() {
            Regs::Arm64(r) => r.x[0],
            Regs::X86(r) => r.gpr[RAX],
        });
        let info = Info { code, pid: self.tid, ..Info::default() };
        let resumed = self.ptrace_stop(cx, Stopping { kind: Kind::Syscall, code, signal: SIGTRAP, info, nr: nr as i64, call: Some(Call::Exit(result)) });
        self.resend(resumed);
    }

    /// SIGTRAP, or with `PTRACE_O_TRACESYSGOOD` SIGTRAP | 0x80, as a system call's stops say.
    fn syscall_code(&self) -> i32 {
        if self.traces(PTRACE_O_TRACESYSGOOD) { SIGTRAP | 0x80 } else { SIGTRAP }
    }

    /// An event's stop, with `msg` for `PTRACE_GETEVENTMSG`.
    fn ptrace_event(&mut self, cx: &mut Cx<'_>, event: i32, msg: u64) {
        lock(&self.trace.0).eventmsg = msg;
        let resumed = self.ptrace_stop(cx, Stopping::event(event));
        self.resend(resumed);
    }

    /// A signal about to be delivered, stopped at for the tracer: the signal to deliver instead (and what it's about),
    /// or None if the tracer suppressed it, or the thread is ending.
    pub(crate) fn ptrace_signal(&mut self, cx: &mut Cx<'_>, signal: i32, info: Info) -> Option<(i32, Info)> {
        let resumed = self.ptrace_stop(cx, Stopping { kind: Kind::Signal, code: signal, signal, info, nr: -1, call: None })?;
        match resumed.signal {
            0 => None,
            s if s == signal => Some((s, resumed.info)),
            s => {
                let tracer = self.tracer().unwrap_or(0);
                Some((s, Info { code: SI_USER, pid: tracer, ..Info::default() }))
            }
        }
    }

    /// A stop signal's default action, for a traced thread: a group stop it reports to its tracer, which ends when
    /// the tracer resumes it (or, after `PTRACE_LISTEN`, at SIGCONT).
    pub(crate) fn ptrace_group_stop(&mut self, cx: &mut Cx<'_>, signal: i32) {
        let seized = lock(&self.trace.0).seized;
        let code = if seized { signal | PTRACE_EVENT_STOP << 8 } else { signal };
        self.ptrace_stop(cx, Stopping { kind: Kind::Group, code, signal, info: Info::user(0), nr: -1, call: None });
    }

    /// A `PTRACE_EVENT_STOP` the tracer asked for (`PTRACE_INTERRUPT`, or a seized new thread's first): made now if
    /// there is one. True if it stopped.
    pub(crate) fn ptrace_interrupted(&mut self, cx: &mut Cx<'_>) -> bool {
        let wanted = {
            let mut t = lock(&self.trace.0);
            t.tracer.is_some() && std::mem::take(&mut t.interrupt)
        };
        if wanted {
            self.ptrace_stop(cx, Stopping::event(PTRACE_EVENT_STOP));
        }
        wanted
    }

    /// After a successful `execve`: `PTRACE_EVENT_EXEC`, or for a tracee that isn't seized and didn't ask, SIGTRAP.
    pub(crate) fn ptrace_exec(&mut self, cx: &mut Cx<'_>) {
        if self.traces(PTRACE_O_TRACEEXEC) {
            self.ptrace_event(cx, PTRACE_EVENT_EXEC, self.tid as u64);
        } else if self.traced() && !lock(&self.trace.0).seized {
            self.k.send(&self.process.clone(), Some(self.tid), SIGTRAP, Info::user(0));
        }
    }

    /// As it ends by `exit` or `exit_group`, with `PTRACE_O_TRACEEXIT`: a stop with the status it ends with.
    pub(crate) fn ptrace_exit(&mut self, cx: &mut Cx<'_>, status: Status) {
        if self.traces(PTRACE_O_TRACEEXIT) {
            self.ptrace_event(cx, PTRACE_EVENT_EXIT, status.wait_status() as u64);
        }
    }

    /// For `clone`: whether its new thread or process is traced too, by the same tracer, as the options say, and so
    /// the event its creator stops at. The new one starts with a SIGSTOP, or seized, with `PTRACE_EVENT_STOP`.
    pub(crate) fn ptrace_follow(&self, flags: u64) -> Option<(i32, Arc<Trace>)> {
        if flags & CLONE_UNTRACED != 0 {
            return None;
        }
        let (event, option) = if flags & CLONE_VFORK != 0 {
            (PTRACE_EVENT_VFORK, PTRACE_O_TRACEVFORK)
        } else if flags & 0xff != SIGCHLD as u64 {
            (PTRACE_EVENT_CLONE, PTRACE_O_TRACECLONE)
        } else {
            (PTRACE_EVENT_FORK, PTRACE_O_TRACEFORK)
        };
        let t = lock(&self.trace.0);
        let tracer = t.tracer.filter(|_| t.options & option != 0)?;
        let child = Traced { tracer: Some(tracer), seized: t.seized, options: t.options, interrupt: t.seized, ..Traced::default() };
        Some((event, Arc::new(Trace(Mutex::new(child)))))
    }

    /// The new thread `tid` is traced (`ptrace_follow`): its tracer keeps it, and a new one not seized starts with
    /// SIGSTOP, which `signals` will have.
    pub(crate) fn ptrace_adopt(&self, tracee: Tracing, signals: &crate::signal::ThreadSignals) -> bool {
        let (tracer, seized) = {
            let t = lock(&tracee.trace.0);
            (t.tracer, t.seized)
        };
        let Some(tracer) = tracer.and_then(|pid| self.k.process(pid)) else { return false };
        lock(&tracer.state).tracees.push(tracee);
        if !seized {
            signals.pending.fetch_or(bit(SIGSTOP), std::sync::atomic::Ordering::AcqRel);
        }
        true
    }

    /// After `clone` made a traced thread or process `tid`: the creator's event stop.
    pub(crate) fn ptrace_forked(&mut self, cx: &mut Cx<'_>, event: i32, tid: i32) {
        self.ptrace_event(cx, event, tid as u64);
    }

    /// The thread has ended with `status`: its tracer hears of it.
    pub(crate) fn ptrace_ended(&self, status: Status) {
        let tracer = {
            let mut t = lock(&self.trace.0);
            let Some(tracer) = t.tracer.take() else { return };
            t.ended = Some(status);
            tracer
        };
        // The parent hears of its child's end itself, when the last thread is gone.
        let parents_child = self.tid == self.process.pid && lock(&self.process.state).ppid == tracer;
        self.k.tracee_changed(tracer, self.tid, None, (!parents_child).then_some(status));
    }

    /// This process is ending: its tracees carry on without it, or with `PTRACE_O_EXITKILL`, end.
    pub(crate) fn ptrace_let_go(&self) {
        let tracees = std::mem::take(&mut lock(&self.process.state).tracees);
        for tracee in tracees {
            let kill = {
                let t = lock(&tracee.trace.0);
                t.tracer == Some(self.process.pid) && t.options & PTRACE_O_EXITKILL != 0
            };
            untrace(&tracee, 0);
            if kill {
                tracee.process.end(Status::Killed(crate::abi::SIGKILL));
            }
        }
    }

    /// Whether this process may trace (or read and write the memory of) the process `target` is: one with the same
    /// real, effective and saved IDs as its own real ones, that's dumpable; or any, with `CAP_SYS_PTRACE`.
    pub(crate) fn may_trace(&self, target: &Arc<Process>) -> bool {
        let creds = lock(&self.process.state).creds.clone();
        if creds.capable(CAP_SYS_PTRACE) {
            return true;
        }
        let state = lock(&target.state);
        let (uid, gid) = (creds.uid[0], creds.gid[0]);
        state.creds.uid[..3].iter().all(|&u| u == uid) && state.creds.gid[..3].iter().all(|&g| g == gid) && state.security.dumpable
    }

    /// `ptrace`.
    pub(crate) fn ptrace(&mut self, cx: &mut Cx<'_>, request: u64, pid: i32, addr: u64, data: u64) -> Result<u64, Errno> {
        match request {
            PTRACE_TRACEME => return self.traceme(),
            PTRACE_ATTACH | PTRACE_SEIZE => return self.attach(pid, request == PTRACE_SEIZE, addr, data),
            _ => {}
        }
        let tracee = lock(&self.process.state).tracees.iter().find(|t| t.tid == pid).cloned().ok_or(ESRCH)?;
        {
            let t = lock(&tracee.trace.0);
            if t.tracer != Some(self.process.pid) {
                return Err(ESRCH);
            }
            match request {
                PTRACE_KILL => {
                    drop(t);
                    tracee.process.end(Status::Killed(SIGKILL));
                    return Ok(0);
                }
                PTRACE_INTERRUPT => {
                    if !t.seized {
                        return Err(EIO);
                    }
                    // One already stopped stops again as soon as it's resumed.
                    let stopped = t.stop.is_some();
                    drop(t);
                    lock(&tracee.trace.0).interrupt = true;
                    if !stopped {
                        self.poke_thread(&tracee);
                    }
                    return Ok(0);
                }
                _ => {}
            }
            // The rest are for a tracee stopped.
            if t.stop.is_none() {
                return Err(ESRCH);
            }
        }
        match request {
            PTRACE_PEEKTEXT | PTRACE_PEEKDATA => {
                let word = self.peek(cx, &tracee.process, addr, 8).ok().filter(|b| b.len() == 8).ok_or(EIO)?;
                self.mem(cx, true, |t| user::write(&mut t.space(), data, &word))?;
                Ok(0)
            }
            PTRACE_POKETEXT | PTRACE_POKEDATA => {
                if self.poke(cx, &tracee.process, addr, &data.to_le_bytes())? != 8 {
                    return Err(EIO);
                }
                Ok(0)
            }
            PTRACE_PEEKUSR => {
                let value = self.with_stop(&tracee, |s| match &s.regs {
                    Regs::X86(r) if addr.is_multiple_of(8) && addr < 27 * 8 => Ok(x86_user_regs(r, s.nr)[addr as usize / 8]),
                    Regs::X86(_) if addr.is_multiple_of(8) && addr < X86_USER_SIZE => Ok(0),
                    _ => Err(EIO),
                })?;
                self.mem(cx, true, |t| user::write_u64(&mut t.space(), data, value))?;
                Ok(0)
            }
            PTRACE_POKEUSR => self.with_stop(&tracee, |s| {
                let nr = &mut s.nr;
                match &mut s.regs {
                    Regs::X86(r) if addr.is_multiple_of(8) && addr < 27 * 8 => {
                        let mut u = x86_user_regs(r, *nr);
                        u[addr as usize / 8] = data;
                        set_x86_user_regs(r, nr, &u);
                        s.regs_set = true;
                        Ok(0)
                    }
                    // The debug registers: there are none, so only clearing one is taken.
                    Regs::X86(_) if addr.is_multiple_of(8) && (X86_USER_DEBUGREG..X86_USER_SIZE).contains(&addr) && data == 0 => Ok(0),
                    _ => Err(EIO),
                }
            }),
            PTRACE_GETREGS | PTRACE_GETFPREGS => {
                let kind = if request == PTRACE_GETREGS { NT_PRSTATUS } else { NT_PRFPREG };
                let b = self.with_stop(&tracee, |s| if matches!(s.regs, Regs::X86(_)) { regset(s, kind) } else { Err(EIO) })?;
                self.mem(cx, true, |t| user::write(&mut t.space(), data, &b))?;
                Ok(0)
            }
            PTRACE_SETREGS | PTRACE_SETFPREGS => {
                let (kind, len) = if request == PTRACE_SETREGS { (NT_PRSTATUS, 27 * 8) } else { (NT_PRFPREG, 512) };
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), data, len))?;
                self.with_stop(&tracee, |s| if matches!(s.regs, Regs::X86(_)) { set_regset(s, kind, &b) } else { Err(EIO) })?;
                Ok(0)
            }
            PTRACE_GETREGSET => {
                let (base, len) = self.read_iovec(cx, data)?;
                let b = self.with_stop(&tracee, |s| regset(s, addr))?;
                let n = b.len().min(len as usize);
                self.mem(cx, true, |t| {
                    user::write(&mut t.space(), base, &b[..n])?;
                    user::write_u64(&mut t.space(), data + 8, n as u64)
                })?;
                Ok(0)
            }
            PTRACE_SETREGSET => {
                let (base, len) = self.read_iovec(cx, data)?;
                let size = self.with_stop(&tracee, |s| regset(s, addr).map(|b| b.len()))?;
                let n = size.min(len as usize);
                let given = self.mem(cx, false, |t| user::read_vec(&t.space(), base, n))?;
                self.with_stop(&tracee, |s| {
                    let mut b = regset(s, addr)?;
                    b[..n].copy_from_slice(&given);
                    set_regset(s, addr, &b)
                })?;
                self.mem(cx, true, |t| user::write_u64(&mut t.space(), data + 8, n as u64))?;
                Ok(0)
            }
            PTRACE_ARCH_PRCTL => {
                const ARCH_SET_GS: u64 = 0x1001;
                const ARCH_SET_FS: u64 = 0x1002;
                const ARCH_GET_FS: u64 = 0x1003;
                const ARCH_GET_GS: u64 = 0x1004;
                // As ptrace(2) has it: the code in `data`, the value or where it goes in `addr`.
                let got = self.with_stop(&tracee, |s| {
                    let Regs::X86(r) = &mut s.regs else { return Err(EIO) };
                    match data {
                        ARCH_SET_FS => r.fs_base = addr,
                        ARCH_SET_GS => r.gs_base = addr,
                        ARCH_GET_FS => return Ok(Some(r.fs_base)),
                        ARCH_GET_GS => return Ok(Some(r.gs_base)),
                        _ => return Err(EINVAL),
                    }
                    s.regs_set = true;
                    Ok(None)
                })?;
                if let Some(value) = got {
                    self.mem(cx, true, |t| user::write_u64(&mut t.space(), addr, value))?;
                }
                Ok(0)
            }
            PTRACE_CONT | PTRACE_SYSCALL | PTRACE_DETACH => {
                let signal = data as i32;
                if !(0..=64).contains(&signal) || data > 64 {
                    return Err(EIO);
                }
                if request == PTRACE_DETACH {
                    lock(&self.process.state).tracees.retain(|t| t.tid != pid);
                    untrace(&tracee, signal);
                    return Ok(0);
                }
                {
                    let mut t = lock(&tracee.trace.0);
                    t.syscalls = request == PTRACE_SYSCALL;
                    t.listening = false;
                    t.resume = Some(signal);
                }
                tracee.waiter.wake();
                Ok(0)
            }
            PTRACE_LISTEN => {
                let mut t = lock(&tracee.trace.0);
                let group = t.stop.as_ref().is_some_and(|s| s.kind == Kind::Group);
                if !t.seized || !group {
                    return Err(EIO);
                }
                t.listening = true;
                Ok(0)
            }
            PTRACE_SINGLESTEP | PTRACE_SYSEMU | PTRACE_SYSEMU_SINGLESTEP => Err(EIO),
            PTRACE_SETOPTIONS => {
                if data & !OPTIONS != 0 {
                    return Err(EINVAL);
                }
                if data & PTRACE_O_SUSPEND_SECCOMP != 0 && !lock(&self.process.state).creds.capable(CAP_SYS_ADMIN) {
                    return Err(EPERM);
                }
                lock(&tracee.trace.0).options = data;
                Ok(0)
            }
            PTRACE_GETEVENTMSG => {
                let msg = lock(&tracee.trace.0).eventmsg;
                self.mem(cx, true, |t| user::write_u64(&mut t.space(), data, msg))?;
                Ok(0)
            }
            PTRACE_GETSIGINFO => {
                let seized = lock(&tracee.trace.0).seized;
                let b = self.with_stop(&tracee, |s| {
                    // A seized tracee's group stop has no signal to tell of: how a tracer knows it's one.
                    if s.kind == Kind::Group && seized {
                        return Err(EINVAL);
                    }
                    Ok(s.info.siginfo(s.signal))
                })?;
                self.mem(cx, true, |t| user::write(&mut t.space(), data, &b))?;
                Ok(0)
            }
            PTRACE_SETSIGINFO => {
                let info = self.read_siginfo(cx, data)?;
                self.with_stop(&tracee, |s| {
                    if s.kind == Kind::Signal {
                        s.info = info;
                    }
                    Ok(0)
                })
            }
            PTRACE_GETSIGMASK | PTRACE_SETSIGMASK => {
                if addr != 8 {
                    return Err(EINVAL);
                }
                let signals = lock(&tracee.process.state).threads.get(&pid).map(|l| l.signals.clone()).ok_or(ESRCH)?;
                let ordering = std::sync::atomic::Ordering::AcqRel;
                if request == PTRACE_GETSIGMASK {
                    let mask = signals.mask.load(ordering);
                    self.mem(cx, true, |t| user::write_u64(&mut t.space(), data, mask))?;
                } else {
                    let mask = self.mem(cx, false, |t| user::read_u64(&t.space(), data))?;
                    signals.mask.store(mask & !crate::signal::UNBLOCKABLE, ordering);
                }
                Ok(0)
            }
            PTRACE_GET_SYSCALL_INFO => {
                let isa = lock(&tracee.process.state).isa;
                let b = self.with_stop(&tracee, |s| Ok(syscall_info(s, isa)))?;
                let n = b.len().min(addr as usize);
                self.mem(cx, true, |t| user::write(&mut t.space(), data, &b[..n]))?;
                Ok(b.len() as u64)
            }
            // No rseq area is ever registered here (`rseq` is ENOSYS): all zeros.
            PTRACE_GET_RSEQ_CONFIGURATION => {
                let n = 24.min(addr as usize);
                self.mem(cx, true, |t| user::write(&mut t.space(), data, &[0; 24][..n]))?;
                Ok(24)
            }
            _ => Err(EIO),
        }
    }

    /// `f` on the stop the tracee is in: ESRCH if it's no longer in one.
    fn with_stop<R>(&self, tracee: &Tracing, f: impl FnOnce(&mut Stopped) -> Result<R, Errno>) -> Result<R, Errno> {
        let mut t = lock(&tracee.trace.0);
        f(t.stop.as_mut().ok_or(ESRCH)?)
    }

    fn read_iovec(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<(u64, u64), Errno> {
        self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))
    }

    /// Makes a tracee's thread stop what it's doing soon: a wait it's in ends, and it comes back from its program.
    fn poke_thread(&self, tracee: &Tracing) {
        tracee.waiter.interrupt();
        if let Some(live) = lock(&tracee.process.state).threads.get(&tracee.tid) {
            live.handle.preempt();
        }
    }

    /// `PTRACE_TRACEME`: its parent traces it.
    fn traceme(&mut self) -> Result<u64, Errno> {
        let ppid = lock(&self.process.state).ppid;
        let parent = self.k.process(ppid).ok_or(EPERM)?;
        {
            let mut t = lock(&self.trace.0);
            if t.tracer.is_some() {
                return Err(EPERM);
            }
            *t = Traced { tracer: Some(ppid), ..Traced::default() };
        }
        let tracing = Tracing { tid: self.tid, process: self.process.clone(), trace: self.trace.clone(), waiter: self.waiter.clone() };
        lock(&parent.state).tracees.push(tracing);
        Ok(0)
    }

    /// `PTRACE_ATTACH` and `PTRACE_SEIZE`: this process traces thread `tid` of another, with SIGSTOP to stop it,
    /// or seized, with `options` and not stopped.
    fn attach(&mut self, tid: i32, seize: bool, addr: u64, options: u64) -> Result<u64, Errno> {
        if seize && (addr != 0 || options & !OPTIONS != 0) {
            return Err(EIO);
        }
        if tid <= 0 {
            return Err(ESRCH);
        }
        let process = self.process_of(tid).ok_or(ESRCH)?;
        if Arc::ptr_eq(&process, &self.process) {
            return Err(EPERM);
        }
        if !self.may_trace(&process) {
            return Err(EPERM);
        }
        let (tracing, signals) = {
            let state = lock(&process.state);
            if state.exiting.is_some() || state.zombie.is_some() {
                return Err(EPERM);
            }
            let live = state.threads.get(&tid).ok_or(ESRCH)?;
            (Tracing { tid, process: process.clone(), trace: live.trace.clone(), waiter: live.waiter.clone() }, live.signals.clone())
        };
        {
            let mut t = lock(&tracing.trace.0);
            if t.tracer.is_some() {
                return Err(EPERM);
            }
            *t = Traced { tracer: Some(self.process.pid), seized: seize, options: if seize { options } else { 0 }, ..Traced::default() };
        }
        lock(&self.process.state).tracees.push(tracing.clone());
        if !seize {
            signals.pending.fetch_or(bit(SIGSTOP), std::sync::atomic::Ordering::AcqRel);
            self.poke_thread(&tracing);
        }
        Ok(0)
    }

    /// SIGCONT came: a tracee that `PTRACE_LISTEN` left stopped carries on.
    pub(crate) fn ptrace_continued(trace: &Trace, waiter: &Waiter) {
        let mut t = lock(&trace.0);
        if std::mem::take(&mut t.listening) && t.stop.is_some() {
            t.resume = Some(0);
            waiter.wake();
        }
    }

    /// Reads up to `len` bytes of `process`'s memory at `addr`, filling its pages as its own touch would: what could
    /// be read, up to the first byte that couldn't. EFAULT if none could; ESRCH if it's gone.
    pub(crate) fn peek(&mut self, cx: &mut Cx<'_>, process: &Arc<Process>, addr: u64, len: usize) -> Result<Vec<u8>, Errno> {
        let mut out = Vec::with_capacity(len);
        self.remote(cx, process, |cx, k, state| {
            while out.len() < len {
                let at = addr.wrapping_add(out.len() as u64);
                let chunk = (len - out.len()).min((PAGE - at % PAGE) as usize);
                let got = user::with_faults(cx, false, |cx, a, access| crate::mm::fill(k, state, cx, a, access, false).is_ok(), |t| user::read_vec(&t.space(), at, chunk));
                match got {
                    Ok(b) => out.extend(b),
                    Err(_) => break,
                }
            }
        })?;
        if out.is_empty() && len > 0 { Err(EFAULT) } else { Ok(out) }
    }

    /// Writes `data` to `process`'s memory at `addr`, as a tracer's poke: to a copy of a page the program may not
    /// write, if it's private. How much was written, up to the first byte that couldn't be. EFAULT if none could.
    pub(crate) fn poke(&mut self, cx: &mut Cx<'_>, process: &Arc<Process>, addr: u64, data: &[u8]) -> Result<usize, Errno> {
        self.write_remote(cx, process, addr, data, true)
    }

    pub(crate) fn write_remote(&mut self, cx: &mut Cx<'_>, process: &Arc<Process>, addr: u64, data: &[u8], force: bool) -> Result<usize, Errno> {
        let mut done = 0;
        self.remote(cx, process, |cx, k, state| {
            while done < data.len() {
                let at = addr.wrapping_add(done as u64);
                let chunk = (data.len() - done).min((PAGE - at % PAGE) as usize);
                let part = &data[done..done + chunk];
                let wrote = user::with_faults(cx, true, |cx, a, access| crate::mm::fill(k, state, cx, a, access, force).is_ok(), |t| user::write(&mut t.space(), at, part));
                match wrote {
                    Ok(()) => done += chunk,
                    Err(_) => break,
                }
            }
        })?;
        if done == 0 && !data.is_empty() { Err(EFAULT) } else { Ok(done) }
    }

    /// `f` with every turn in `process`'s space, and its state held, so its space stays what it is: ESRCH if it has
    /// none any more.
    fn remote(&mut self, cx: &mut Cx<'_>, process: &Arc<Process>, f: impl FnOnce(&mut Cx<'_>, &crate::Kernel, &super::ProcState)) -> Result<(), Errno> {
        let k = self.k.clone();
        let state = lock(&process.state);
        if state.zombie.is_some() || state.threads.is_empty() {
            return Err(ESRCH);
        }
        let space = state.space;
        cx.elsewhere(space, |cx| f(cx, &k, &state));
        Ok(())
    }
}

const PAGE: u64 = 4096;

/// `struct ptrace_syscall_info` for the stop the tracee is in: what call it's entering, or what it returned.
fn syscall_info(s: &Stopped, isa: Isa) -> Vec<u8> {
    const OP_NONE: u8 = 0;
    const OP_ENTRY: u8 = 1;
    const OP_EXIT: u8 = 2;
    let arch = if isa.is_x86() { AUDIT_ARCH_X86_64 } else { AUDIT_ARCH_AARCH64 };
    let op = match s.call {
        Some(Call::Entry(_)) => OP_ENTRY,
        Some(Call::Exit(_)) => OP_EXIT,
        None => OP_NONE,
    };
    let mut b = vec![op, 0, 0, 0];
    b.extend(arch.to_le_bytes());
    b.extend(s.regs.pc().to_le_bytes());
    b.extend(s.regs.sp().to_le_bytes());
    match s.call {
        Some(Call::Entry(args)) => {
            b.extend((s.nr as u64).to_le_bytes());
            for a in args {
                b.extend(a.to_le_bytes());
            }
        }
        Some(Call::Exit(rval)) => {
            b.extend(rval.to_le_bytes());
            b.push(u8::from((rval as i64) < 0 && (rval as i64) >= -4095));
        }
        None => {}
    }
    b
}
