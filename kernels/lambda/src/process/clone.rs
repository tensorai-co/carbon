//! New threads and processes, waiting for children, and futexes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use exec::Regs;
use kernels_core::lock;
use kernels_core::thread::{self, Cx};
use kernels_core::user;
use kernels_core::wait::{Changed, Waiter, Woke};

use super::{ProcState, Process, Report, Task, usage};
use crate::Status;
use crate::abi::*;
use crate::signal::ThreadSignals;

/// What a wait is for.
#[derive(Debug, Clone, Copy)]
struct WaitFor {
    exited: bool,
    stopped: bool,
    continued: bool,
    nohang: bool,
    /// Leave the child as it is, to be waited for again.
    nowait: bool,
}

/// What happened to a child.
#[derive(Debug, Clone, Copy)]
enum Event {
    Ended(Status),
    Stopped(i32),
    Continued,
    /// A tracee's stop, with its `exit_code` (the signal, and an event).
    Traced(i32),
}

/// A child a wait found: who, whose, what happened, and its CPU time with its own waited-for children's.
struct Waited {
    pid: i32,
    uid: u32,
    event: Event,
    times: (u64, u64),
}

impl Task {
    /// `clone`, with arguments in ARM64's order: flags, stack, parent_tid, tls, child_tid.
    pub(super) fn clone(&mut self, cx: &mut Cx<'_>, flags: u64, stack: u64, parent_tid: u64, tls: u64, child_tid: u64) -> Result<u64, Errno> {
        let thread = flags & CLONE_THREAD != 0;
        if thread && flags & CLONE_VM == 0 {
            return Err(EINVAL);
        }
        self.clone_namespaces(flags, thread)?;
        // A pidfd goes where the parent's tid would, so not both; and it's for a process.
        if flags & CLONE_PIDFD != 0 && (thread || flags & CLONE_PARENT_SETTID != 0) {
            return Err(EINVAL);
        }
        let tid = self.k.new_id();
        // Traced too, if its creator's tracer asked to follow it.
        let follow = self.ptrace_follow(flags);
        let (regs, fp) = cx.turn(|t| (t.thread().regs(), t.thread().fp()));
        let mut child = regs;
        child.set_result(0);
        if stack != 0 {
            child.set_sp(stack);
        }
        if flags & CLONE_SETTLS != 0 {
            match &mut child {
                Regs::Arm64(r) => r.tpidr = tls,
                Regs::X86(r) => r.fs_base = tls,
            }
        }
        if flags & CLONE_PARENT_SETTID != 0 {
            self.mem(cx, true, |t| user::write_u32(&mut t.space(), parent_tid, tid as u32))?;
        }
        let child_set = if flags & CLONE_CHILD_SETTID != 0 { child_tid } else { 0 };
        let child_clear = if flags & CLONE_CHILD_CLEARTID != 0 { child_tid } else { 0 };

        // A thread joins this process; anything else is a new process with a copy of this one's memory (`vfork` and
        // `CLONE_VM` without `CLONE_THREAD` too, for now: the child only ever runs `execve` or `_exit`).
        let process = if thread {
            self.process.clone()
        } else {
            let space = cx.turn(|t| t.fork_space()).map_err(|_| ENOMEM)?;
            let state = lock(&self.process.state);
            let mut child_ns = state.ns.clone();
            child_ns.unshare(flags & crate::ns::NEW_SUPPORTED);
            let copy = ProcState {
                ppid: self.process.pid,
                pgid: state.pgid,
                sid: state.sid,
                isa: state.isa,
                space,
                regions: state.regions.clone(),
                mmap: state.mmap.clone(),
                brk: state.brk,
                files: state.files.clone(),
                cwd: state.cwd.clone(),
                root: state.root.clone(),
                ns: child_ns,
                umask: state.umask,
                limits: state.limits,
                sched: state.sched.forked(),
                personality: state.personality,
                usage: Default::default(),
                timers: Default::default(),
                mempolicy: state.mempolicy,
                shm_attached: state.shm_attached.clone(),
                sem_undo: Default::default(),
                aio: Default::default(),
                security: state.security.clone(),
                comm: state.comm.clone(),
                creds: state.creds.clone(),
                exe: state.exe.clone(),
                threads: Default::default(),
                exiting: None,
                zombie: None,
                children: Vec::new(),
                tracees: Vec::new(),
                child_waiters: Vec::new(),
                signals: state.signals.clone(),
                pending: 0,
                infos: Default::default(),
                stopped: None,
                report: None,
                ctty: state.ctty.clone(),
                sigreturn_at: state.sigreturn_at,
            };
            drop(state);
            Process::new(tid, copy)
        };
        let space = lock(&process.state).space;
        let waiter = Waiter::new();
        let signals = ThreadSignals::with_mask(self.signals.mask.load(std::sync::atomic::Ordering::Acquire));
        let (k, isa, child_signals, altstack) = (self.k.clone(), self.isa, signals.clone(), if thread { None } else { self.altstack });
        let (child_process, child_waiter) = (process.clone(), waiter.clone());
        let mut adopted = None;
        let child_trace = match follow {
            Some((event, trace)) => {
                let tracing = super::ptrace::Tracing { tid, process: process.clone(), trace: trace.clone(), waiter: waiter.clone() };
                if self.ptrace_adopt(tracing, &signals) {
                    adopted = Some(event);
                }
                trace
            }
            None => Arc::default(),
        };
        // A new process is its parent's child, and can be found by its pid, before it runs: it may signal or wait on
        // itself at once.
        if !thread {
            lock(&self.process.state).children.push(tid);
            lock(&self.k.processes).insert(tid, process.clone());
        }
        if flags & CLONE_PIDFD != 0 {
            let fd = self.add_pidfd(process, false)?;
            if self.mem(cx, true, |t| user::write_u32(&mut t.space(), parent_tid, fd as u32)).is_err() {
                let _ = self.close(fd);
                lock(&self.process.state).children.retain(|&c| c != tid);
                lock(&self.k.processes).remove(&tid);
                cx.turn(|t| t.destroy_space(space));
                return Err(EFAULT);
            }
        }
        let started = thread::spawn(self.k.exec.clone(), format!("lambda {tid}"), move |cx| {
            cx.turn(|t| {
                t.use_space(space);
                t.thread().set_regs(&child);
                t.thread().set_fp(&fp);
            });
            let mut task = Task::new(k, child_process, tid, child_waiter, child_signals, isa);
            task.clear_tid = child_clear;
            task.altstack = altstack;
            task.trace = child_trace;
            // Through `mem`: the page is likely shared copy-on-write with the parent.
            if child_set != 0 {
                task.mem(cx, true, |t| user::write_u32(&mut t.space(), child_set, tid as u32)).map_err(|_| "CLONE_CHILD_SETTID".to_string())?;
            }
            // One of the process's threads before it can run, and so before it can end: added after, a thread that
            // ended first would stay in the list, and the process would never see its last thread go.
            let live = task.live(cx.handle());
            lock(&task.process.state).threads.insert(tid, live);
            Ok(task)
        });
        match started {
            Ok(_) => {}
            Err(why) => {
                if self.k.options.trace {
                    self.k.host.write(host::Stream::Err, format!("lambda: clone: {why}\n").as_bytes());
                }
                if adopted.is_some() {
                    for p in lock(&self.k.processes).values() {
                        lock(&p.state).tracees.retain(|t| t.tid != tid);
                    }
                }
                if !thread {
                    lock(&self.process.state).children.retain(|&c| c != tid);
                    lock(&self.k.processes).remove(&tid);
                    cx.turn(|t| t.destroy_space(space));
                }
                return Err(EAGAIN);
            }
        }
        if let Some(event) = adopted {
            self.forked = Some((event, tid));
        }
        Ok(tid as u64)
    }

    /// Waits for a child `wanted` picks to end (or with `how`, to stop or continue), and reaps one that ended unless
    /// `how.nowait`. None if `how.nohang` and none has yet; ECHILD if there's no such child to wait for.
    fn wait_child(&mut self, wanted: impl Fn(i32, &ProcState) -> bool, how: WaitFor) -> Result<Option<Waited>, Errno> {
        loop {
            let (children, tracees) = {
                let mut me = lock(&self.process.state);
                me.child_waiters.push(self.waiter.clone());
                self.waiter.reset();
                (me.children.clone(), me.tracees.clone())
            };
            let mut any = false;
            // Its tracees first: their stops whatever `how` asks (as Linux's), and their ends, unless that's a child's
            // end, which its parent hears of below. A tracee that's gone is forgotten once it's heard of.
            for tracee in tracees {
                let (found, forget) = {
                    let state = lock(&tracee.process.state);
                    if !wanted(tracee.tid, &state) {
                        continue;
                    }
                    any = true;
                    let parents_child = tracee.tid == tracee.process.pid && state.ppid == self.process.pid;
                    let uid = state.creds.uid[0];
                    let ended = lock(&tracee.trace.0).ended;
                    match (tracee.trace.report(how.nowait), ended) {
                        (Some(code), _) => (Some(Waited { pid: tracee.tid, uid, event: Event::Traced(code), times: (0, 0) }), false),
                        (None, Some(_)) if parents_child => (None, true),
                        (None, Some(status)) if how.exited => (Some(Waited { pid: tracee.tid, uid, event: Event::Ended(status), times: (0, 0) }), !how.nowait),
                        _ => (None, false),
                    }
                };
                if forget {
                    lock(&self.process.state).tracees.retain(|t| !Arc::ptr_eq(&t.trace, &tracee.trace));
                }
                if let Some(waited) = found {
                    self.forget_waiter();
                    return Ok(Some(waited));
                }
            }
            for child in children {
                let Some(process) = self.k.process(child) else { continue };
                let found = {
                    let mut state = lock(&process.state);
                    if !wanted(child, &state) {
                        continue;
                    }
                    any = true;
                    let times = usage::add(state.usage.ended, state.usage.children);
                    let event = match (state.zombie, state.report) {
                        (Some(status), _) if how.exited => Some(Event::Ended(status)),
                        (Some(_), _) => None,
                        (None, Some(Report::Stopped(signal))) if how.stopped => Some(Event::Stopped(signal)),
                        (None, Some(Report::Continued)) if how.continued => Some(Event::Continued),
                        _ => None,
                    };
                    if event.is_some() && !matches!(event, Some(Event::Ended(_))) && !how.nowait {
                        state.report = None;
                    }
                    event.map(|event| Waited { pid: child, uid: state.creds.uid[0], event, times })
                };
                if let Some(waited) = found {
                    self.forget_waiter();
                    if matches!(waited.event, Event::Ended(_)) && !how.nowait {
                        {
                            let mut me = lock(&self.process.state);
                            me.children.retain(|&c| c != child);
                            me.usage.children = usage::add(me.usage.children, waited.times);
                        }
                        lock(&self.k.processes).remove(&child);
                    }
                    return Ok(Some(waited));
                }
            }
            if !any {
                self.forget_waiter();
                return Err(ECHILD);
            }
            if how.nohang {
                self.forget_waiter();
                return Ok(None);
            }
            if self.waiter.wait(None) == Woke::Interrupted {
                self.forget_waiter();
                return Err(EINTR);
            }
        }
    }

    /// `wait4`: by pid (-1 for any, 0 for this process group, -pgid for another), reporting stops with `WUNTRACED`
    /// and continues with `WCONTINUED`, with the child's usage if asked.
    pub(super) fn wait4(&mut self, cx: &mut Cx<'_>, pid: i32, status_at: u64, options: u64, rusage_at: u64) -> Result<u64, Errno> {
        const WNOHANG: u64 = 1;
        const WUNTRACED: u64 = 2;
        const WCONTINUED: u64 = 8;
        let my_group = lock(&self.process.state).pgid;
        let wanted = move |child: i32, state: &ProcState| match pid {
            -1 => true,
            0 => state.pgid == my_group,
            ..-1 => state.pgid == -pid,
            _ => child == pid,
        };
        let how = WaitFor { exited: true, stopped: options & WUNTRACED != 0, continued: options & WCONTINUED != 0, nohang: options & WNOHANG != 0, nowait: false };
        let Some(waited) = self.wait_child(wanted, how)? else { return Ok(0) };
        let status = match waited.event {
            Event::Ended(status) => status.wait_status(),
            Event::Stopped(signal) => 0x7f | signal << 8,
            Event::Continued => 0xffff,
            Event::Traced(code) => super::ptrace::wait_status(code),
        };
        if status_at != 0 {
            self.mem(cx, true, |t| user::write_u32(&mut t.space(), status_at, status as u32))?;
        }
        if rusage_at != 0 {
            self.write_rusage(cx, rusage_at, waited.times)?;
        }
        Ok(waited.pid as u64)
    }

    /// `waitid`: by `P_ALL`, `P_PID` or `P_PGID`, for what `options` asks (`WEXITED`, `WSTOPPED`, `WCONTINUED`), with
    /// `WNOWAIT` to leave the child to be waited for again. Says what happened in a `siginfo`.
    pub(super) fn waitid(&mut self, cx: &mut Cx<'_>, idtype: u64, id: i32, info_at: u64, options: u64, rusage_at: u64) -> Result<u64, Errno> {
        const WNOHANG: u64 = 1;
        const WSTOPPED: u64 = 2;
        const WEXITED: u64 = 4;
        const WCONTINUED: u64 = 8;
        const WNOWAIT: u64 = 0x0100_0000;
        // __WNOTHREAD, __WALL and __WCLONE change nothing here.
        const KNOWN: u64 = WNOHANG | WSTOPPED | WEXITED | WCONTINUED | WNOWAIT | 0xe000_0000;
        const P_ALL: u64 = 0;
        const P_PID: u64 = 1;
        const P_PGID: u64 = 2;
        const P_PIDFD: u64 = 3;
        if options & !KNOWN != 0 || options & (WEXITED | WSTOPPED | WCONTINUED) == 0 {
            return Err(EINVAL);
        }
        let my_group = lock(&self.process.state).pgid;
        let group = if id == 0 { my_group } else { id };
        // Which children: any (None), one by pid (or pidfd), or a process group's. A non-blocking pidfd doesn't wait.
        let mut pidfd_nonblock = false;
        let (pid, pgid) = match idtype {
            P_ALL => (None, None),
            P_PID if id > 0 => (Some(id), None),
            P_PGID if id >= 0 => (None, Some(group)),
            P_PIDFD => {
                let (process, nonblock) = self.pidfd_process(id)?;
                pidfd_nonblock = nonblock;
                (Some(process.pid), None)
            }
            _ => return Err(EINVAL),
        };
        let wanted = move |child: i32, state: &ProcState| pid.is_none_or(|p| p == child) && pgid.is_none_or(|g| g == state.pgid);
        let how = WaitFor {
            exited: options & WEXITED != 0,
            stopped: options & WSTOPPED != 0,
            continued: options & WCONTINUED != 0,
            nohang: options & WNOHANG != 0 || pidfd_nonblock,
            nowait: options & WNOWAIT != 0,
        };
        let waited = self.wait_child(wanted, how)?;
        if waited.is_none() && pidfd_nonblock && options & WNOHANG == 0 {
            return Err(EAGAIN);
        }
        if info_at != 0 {
            // si_signo, si_errno, si_code; then si_pid, si_uid, si_status, si_utime and si_stime. With WNOHANG and
            // nothing to report, all zero.
            let mut b = vec![0u8; 48];
            if let Some(w) = &waited {
                let (code, status): (i32, i32) = match w.event {
                    Event::Ended(Status::Exited(code)) => (1, code),
                    Event::Ended(Status::Killed(signal)) if crate::signal::dumps_core(signal) => (3, signal),
                    Event::Ended(Status::Killed(signal)) => (2, signal),
                    Event::Stopped(signal) => (5, signal),
                    Event::Continued => (6, crate::signal::SIGCONT),
                    // CLD_TRAPPED.
                    Event::Traced(code) => (4, code),
                };
                b[0..4].copy_from_slice(&SIGCHLD.to_le_bytes());
                b[8..12].copy_from_slice(&code.to_le_bytes());
                b[16..20].copy_from_slice(&w.pid.to_le_bytes());
                b[20..24].copy_from_slice(&w.uid.to_le_bytes());
                b[24..28].copy_from_slice(&status.to_le_bytes());
                b[32..40].copy_from_slice(&usage::ticks(w.times.0).to_le_bytes());
                b[40..48].copy_from_slice(&usage::ticks(w.times.1).to_le_bytes());
            }
            self.mem(cx, true, |t| user::write(&mut t.space(), info_at, &b))?;
        }
        if rusage_at != 0 {
            self.write_rusage(cx, rusage_at, waited.as_ref().map_or((0, 0), |w| w.times))?;
        }
        Ok(0)
    }

    fn forget_waiter(&self) {
        let mut me = lock(&self.process.state);
        me.child_waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
    }

    pub(super) fn futex(&mut self, cx: &mut Cx<'_>, a: [u64; 6]) -> Result<u64, Errno> {
        let [addr, op, val, timeout, addr2, val3] = a;
        let private = op & FUTEX_PRIVATE_FLAG != 0;
        let key = self.futex_key(addr, private);
        match op & FUTEX_CMD_MASK {
            FUTEX_WAIT | FUTEX_WAIT_BITSET => {
                let deadline = if timeout == 0 {
                    None
                } else {
                    let (sec, nsec) = self.mem(cx, false, |t| {
                        let space = t.space();
                        Ok((user::read_u64(&space, timeout)?, user::read_u64(&space, timeout + 8)?))
                    })?;
                    let duration = Duration::new(sec, nsec as u32);
                    Some(if op & FUTEX_CMD_MASK == FUTEX_WAIT_BITSET {
                        // An absolute time, on the monotonic clock (or the real-time one, if asked).
                        let now = if op & 256 != 0 { self.k.host.wall() } else { self.k.host.monotonic() };
                        Instant::now() + duration.saturating_sub(Duration::from_nanos(now))
                    } else {
                        Instant::now() + duration
                    })
                };
                let waiter = self.waiter.clone();
                let still = || cx.turn(|t| user::read_u32(&t.space(), addr)) == Ok(val as u32);
                match self.k.futexes.wait(key, &waiter, still, deadline) {
                    Err(Changed) => Err(EAGAIN),
                    Ok(Woke::Woken) => Ok(0),
                    Ok(Woke::Interrupted) => Err(EINTR),
                    Ok(Woke::TimedOut) => Err(ETIMEDOUT),
                }
            }
            FUTEX_WAKE | FUTEX_WAKE_BITSET => Ok(self.k.futexes.wake(key, val as usize) as u64),
            FUTEX_REQUEUE | FUTEX_CMP_REQUEUE => {
                if op & FUTEX_CMD_MASK == FUTEX_CMP_REQUEUE && cx.turn(|t| user::read_u32(&t.space(), addr))? != val3 as u32 {
                    return Err(EAGAIN);
                }
                let woken = self.k.futexes.wake(key, val as usize);
                let moved = self.k.futexes.requeue(key, self.futex_key(addr2, private), timeout as usize);
                Ok((woken + moved) as u64)
            }
            FUTEX_WAKE_OP => {
                let (kind, compare) = ((val3 >> 28) & 7, (val3 >> 24) & 15);
                let (mut arg, against) = ((val3 >> 12) & 0xfff, val3 & 0xfff);
                if kind & 8 != 0 {
                    arg = 1 << arg;
                }
                let old = cx.turn(|t| {
                    t.space().update_u32(addr2, |v| match kind & 7 {
                        0 => arg as u32,
                        1 => v.wrapping_add(arg as u32),
                        2 => v | arg as u32,
                        3 => v & !(arg as u32),
                        _ => v ^ arg as u32,
                    })
                });
                let old = u64::from(old.map_err(|_| EFAULT)?);
                let mut woken = self.k.futexes.wake(key, val as usize);
                let yes = match compare {
                    0 => old == against,
                    1 => old != against,
                    2 => old < against,
                    3 => old <= against,
                    4 => old > against,
                    _ => old >= against,
                };
                if yes {
                    woken += self.k.futexes.wake(self.futex_key(addr2, private), timeout as usize);
                }
                Ok(woken as u64)
            }
            _ => Err(ENOSYS),
        }
    }
}
