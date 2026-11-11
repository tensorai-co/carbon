//! New threads and processes, waiting for children, and futexes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use exec::Regs;
use kernels_core::lock;
use kernels_core::thread::{self, Cx};
use kernels_core::user;
use kernels_core::wait::{Changed, Waiter, Woke};

use super::{Live, ProcState, Process, Report, Task};
use crate::abi::*;
use crate::signal::ThreadSignals;

impl Task {
    /// `clone`, with arguments in ARM64's order: flags, stack, parent_tid, tls, child_tid.
    pub(super) fn clone(&mut self, cx: &mut Cx<'_>, flags: u64, stack: u64, parent_tid: u64, tls: u64, child_tid: u64) -> Result<u64, Errno> {
        let thread = flags & CLONE_THREAD != 0;
        if thread && flags & CLONE_VM == 0 {
            return Err(EINVAL);
        }
        let tid = self.k.new_id();
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
                umask: state.umask,
                creds: state.creds.clone(),
                exe: state.exe.clone(),
                threads: Default::default(),
                exiting: None,
                zombie: None,
                children: Vec::new(),
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
            Arc::new(Process { pid: tid, state: Mutex::new(copy), signal_watchers: Default::default() })
        };
        let space = lock(&process.state).space;
        let waiter = Waiter::new();
        let signals = ThreadSignals::with_mask(self.signals.mask.load(std::sync::atomic::Ordering::Acquire));
        let (k, isa, child_signals, altstack) = (self.k.clone(), self.isa, signals.clone(), if thread { None } else { self.altstack });
        let (child_process, child_waiter, live_signals) = (process.clone(), waiter.clone(), signals.clone());
        let started = thread::spawn(self.k.exec.clone(), format!("lambda {tid}"), move |cx| {
            cx.turn(|t| {
                t.use_space(space);
                t.thread().set_regs(&child);
                t.thread().set_fp(&fp);
            });
            let mut task = Task::new(k, child_process, tid, child_waiter, child_signals, isa);
            task.clear_tid = child_clear;
            task.altstack = altstack;
            // Through `mem`: the page is likely shared copy-on-write with the parent.
            if child_set != 0 {
                task.mem(cx, true, |t| user::write_u32(&mut t.space(), child_set, tid as u32)).map_err(|_| "CLONE_CHILD_SETTID".to_string())?;
            }
            // One of the process's threads before it can run, and so before it can end: added after, a thread that
            // ended first would stay in the list, and the process would never see its last thread go.
            lock(&task.process.state).threads.insert(tid, Live { handle: cx.handle(), waiter: task.waiter.clone(), signals: live_signals });
            Ok(task)
        });
        match started {
            Ok(_) => {}
            Err(why) => {
                if self.k.options.trace {
                    self.k.host.write(host::Stream::Err, format!("lambda: clone: {why}\n").as_bytes());
                }
                if !thread {
                    cx.turn(|t| t.destroy_space(space));
                }
                return Err(EAGAIN);
            }
        }
        if !thread {
            lock(&self.process.state).children.push(tid);
            lock(&self.k.processes).insert(tid, process);
        }
        Ok(tid as u64)
    }

    pub(super) fn wait4(&mut self, cx: &mut Cx<'_>, pid: i32, status_at: u64, options: u64) -> Result<u64, Errno> {
        const WUNTRACED: u64 = 2;
        const WCONTINUED: u64 = 8;
        loop {
            let (children, my_group) = {
                let mut me = lock(&self.process.state);
                me.child_waiters.push(self.waiter.clone());
                self.waiter.reset();
                (me.children.clone(), me.pgid)
            };
            let mut any = false;
            for child in children {
                let Some(process) = self.k.process(child) else { continue };
                let found = {
                    let mut state = lock(&process.state);
                    let wanted = match pid {
                        -1 => true,
                        0 => state.pgid == my_group,
                        ..-1 => state.pgid == -pid,
                        _ => child == pid,
                    };
                    if !wanted {
                        continue;
                    }
                    any = true;
                    if let Some(status) = state.zombie {
                        Some((status.wait_status(), true))
                    } else {
                        match state.report {
                            Some(Report::Stopped(signal)) if options & WUNTRACED != 0 => {
                                state.report = None;
                                Some((0x7f | signal << 8, false))
                            }
                            Some(Report::Continued) if options & WCONTINUED != 0 => {
                                state.report = None;
                                Some((0xffff, false))
                            }
                            _ => None,
                        }
                    }
                };
                if let Some((status, reaped)) = found {
                    self.forget_waiter();
                    if reaped {
                        lock(&self.process.state).children.retain(|&c| c != child);
                        lock(&self.k.processes).remove(&child);
                    }
                    if status_at != 0 {
                        self.mem(cx, true, |t| user::write_u32(&mut t.space(), status_at, status as u32))?;
                    }
                    return Ok(child as u64);
                }
            }
            if !any {
                self.forget_waiter();
                return Err(ECHILD);
            }
            if options & WNOHANG != 0 {
                self.forget_waiter();
                return Ok(0);
            }
            if self.waiter.wait(None) == Woke::Interrupted {
                self.forget_waiter();
                return Err(EINTR);
            }
        }
    }

    fn forget_waiter(&self) {
        let mut me = lock(&self.process.state);
        me.child_waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
    }

    pub(super) fn futex(&mut self, cx: &mut Cx<'_>, a: [u64; 6]) -> Result<u64, Errno> {
        let [addr, op, val, timeout, addr2, val3] = a;
        let key = self.futex_key(addr);
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
                let moved = self.k.futexes.requeue(key, self.futex_key(addr2), timeout as usize);
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
                    woken += self.k.futexes.wake(self.futex_key(addr2), timeout as usize);
                }
                Ok(woken as u64)
            }
            _ => Err(ENOSYS),
        }
    }
}
