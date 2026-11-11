//! Processes and threads: each guest thread's [`Task`], what happens at each stop, and the system call dispatcher.

mod clone;
mod creds;
mod elf;
mod load;

use std::collections::BTreeMap;
use std::sync::Arc;

use exec::{CpuHandle, Exit, FpRegs, Isa, SpaceId, Stop};
use kernels_core::lock;
use kernels_core::mm::Regions;
use kernels_core::thread::{Cx, Flow, Guest};
use kernels_core::wait::Waiter;
use std::sync::Mutex;

use crate::abi::*;
use crate::fs::Files;
use crate::mm::Area;
use crate::signal::{AltStack, Change, Interrupted, Signals, ThreadSignals};
use crate::sys::{self, Sys};
use crate::tty::Tty;
use crate::{Kernel, Status};

pub(crate) use load::exe_path;

pub(crate) struct Process {
    pub pid: i32,
    pub state: Mutex<ProcState>,
    /// Told when a signal becomes pending, for `signalfd`s being waited on.
    pub signal_watchers: kernels_core::wait::Watchers,
}

pub(crate) struct ProcState {
    pub ppid: i32,
    pub pgid: i32,
    pub sid: i32,
    pub isa: Isa,
    pub space: SpaceId,
    pub regions: Regions<Area>,
    /// Where `mmap` finds room, top down.
    pub mmap: std::ops::Range<u64>,
    /// The heap: where it starts, and where it ends now.
    pub brk: (u64, u64),
    pub files: Files,
    /// The working directory, and its path.
    pub cwd: (layers::Ino, String),
    pub umask: u32,
    pub creds: creds::Creds,
    pub exe: String,
    pub threads: BTreeMap<i32, Live>,
    /// Set once, when the process starts to end; every thread ends at its next chance.
    pub exiting: Option<Status>,
    /// Set when its last thread has ended, until its parent collects it.
    pub zombie: Option<Status>,
    pub children: Vec<i32>,
    /// Threads in `wait4`, to wake when a child changes.
    pub child_waiters: Vec<Arc<Waiter>>,
    pub signals: Signals,
    /// Signals for the process as a whole, which any thread not blocking them may take, and what each is about.
    pub pending: u64,
    pub infos: BTreeMap<i32, crate::signal::Info>,
    /// Stopped, by this signal, until SIGCONT.
    pub stopped: Option<i32>,
    /// A stop or continue its parent hasn't heard of through `wait4` yet.
    pub report: Option<Report>,
    /// Its session's controlling terminal.
    pub ctty: Option<Arc<Tty>>,
    /// Where `rt_sigreturn`'s trampoline is, for handlers with no restorer of their own.
    pub sigreturn_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Report {
    Stopped(i32),
    Continued,
}

/// A running thread, as its process knows it.
pub(crate) struct Live {
    pub handle: CpuHandle,
    pub waiter: Arc<Waiter>,
    pub signals: Arc<ThreadSignals>,
}

impl Process {
    /// Starts ending the process with `status`, unless it's already ending: every thread stops at its next chance.
    pub fn end(&self, status: Status) {
        let mut state = lock(&self.state);
        if state.exiting.is_none() {
            state.exiting = Some(status);
        }
        for live in state.threads.values() {
            live.waiter.interrupt();
            live.handle.preempt();
        }
    }
}

/// What a system call did.
pub(crate) enum Outcome {
    /// Its result, to set as the call returns.
    Return(u64),
    /// It set the registers itself (`execve`, `rt_sigreturn`).
    Keep,
    /// This thread is over.
    End,
}

impl From<Result<u64, Errno>> for Outcome {
    fn from(result: Result<u64, Errno>) -> Outcome {
        Outcome::Return(match result {
            Ok(value) => value,
            Err(Errno(e)) => (-(e as i64)) as u64,
        })
    }
}

/// One guest thread, as lambda runs it.
pub(crate) struct Task {
    pub k: Arc<Kernel>,
    pub process: Arc<Process>,
    pub tid: i32,
    pub waiter: Arc<Waiter>,
    /// Where to write 0 and wake a futex when this thread ends (`set_tid_address`, `CLONE_CHILD_CLEARTID`).
    pub clear_tid: u64,
    pub isa: Isa,
    pub signals: Arc<ThreadSignals>,
    /// Where a fault was, and what it was, for the signal it sends.
    pub fault_info: Option<(u64, String)>,
    /// The mask to put back after `rt_sigsuspend`'s handler.
    pub suspend_mask: Option<u64>,
    pub altstack: Option<AltStack>,
    /// FP registers kept for each signal frame, by its address, until `rt_sigreturn`.
    pub saved_fp: Vec<(u64, FpRegs)>,
    /// The system call that just returned EINTR.
    interrupted: Option<Interrupted>,
}

/// What a system call returns for EINTR.
const EINTR_RETURN: u64 = (-(EINTR.0 as i64)) as u64;

impl Task {
    pub fn new(k: Arc<Kernel>, process: Arc<Process>, tid: i32, waiter: Arc<Waiter>, signals: Arc<ThreadSignals>, isa: Isa) -> Task {
        Task { k, process, tid, waiter, clear_tid: 0, isa, signals, fault_info: None, suspend_mask: None, altstack: None, saved_fp: Vec::new(), interrupted: None }
    }
}

impl Guest for Task {
    fn exit(&mut self, cx: &mut Cx<'_>, exit: Exit) -> Flow {
        let stop = cx.turn(|t| t.stop(exit));
        if self.ending() {
            return self.end_thread(cx);
        }
        self.interrupted = None;
        let outcome = match stop {
            None | Some(Stop::Preempted) => Outcome::Keep,
            Some(Stop::Syscall { nr, args }) => self.syscall(cx, nr, args),
            Some(Stop::Fault { addr, access, pc }) => {
                if self.fault(cx, addr, access).is_err() {
                    self.fault_signal(SIGSEGV, addr, &format!("bad access to {addr:#x} ({access:?}) at {pc:#x}"));
                }
                Outcome::Keep
            }
            Some(Stop::Undefined { pc }) => {
                // The bytes there, so what's missing (from the translator, say) can be named.
                let bytes = cx.turn(|t| kernels_core::user::read_vec(&t.space(), pc, 16)).unwrap_or_default();
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                self.fatal(SIGILL, pc, &format!("undefined instruction at {pc:#x} ({hex})"))
            }
            Some(Stop::Breakpoint { pc }) => self.fatal(SIGTRAP, pc, &format!("breakpoint at {pc:#x}")),
            Some(Stop::Alignment { pc }) => self.fatal(SIGBUS, pc, &format!("misaligned at {pc:#x}")),
            Some(Stop::DivideError { pc }) => self.fatal(SIGFPE, pc, &format!("divide error at {pc:#x}")),
            Some(Stop::FastFail { pc, code }) => self.fatal(SIGSEGV, pc, &format!("fast fail {code} at {pc:#x}")),
            Some(Stop::Internal { pc, detail }) => self.fatal(SIGKILL, pc, &format!("exec couldn't run it at {pc:#x} ({detail:#x})")),
        };
        if self.ending() {
            return self.end_thread(cx);
        }
        let interrupted = match outcome {
            Outcome::Return(value) => match self.interrupted.take() {
                Some(call) if value == EINTR_RETURN => Some(call),
                _ => {
                    cx.turn(|t| t.thread().set_return(value));
                    None
                }
            },
            Outcome::Keep => None,
            Outcome::End => return self.end_thread(cx),
        };
        match self.deliver(cx, interrupted) {
            Flow::Run => Flow::Run,
            Flow::End => self.end_thread(cx),
        }
    }
}

impl Task {
    /// Copies in or out of the program's memory in a turn, filling pages it may use as its own touch would.
    pub fn mem<R>(&mut self, cx: &mut Cx<'_>, write: bool, copy: impl FnMut(&mut exec::Turn<'_>) -> Result<R, kernels_core::user::Fault>) -> Result<R, Errno> {
        Ok(kernels_core::user::with_faults(cx, write, |cx, addr, access| self.fault(cx, addr, access).is_ok(), copy)?)
    }

    pub fn ending(&self) -> bool {
        lock(&self.process.state).exiting.is_some()
    }

    /// A futex's key: its address in this process's space.
    pub fn futex_key(&self, addr: u64) -> u64 {
        let space = lock(&self.process.state).space;
        space.0 << 48 ^ addr
    }

    /// The program did something it can't carry on from, and gets `signal` for it.
    fn fatal(&mut self, signal: i32, pc: u64, what: &str) -> Outcome {
        self.fault_signal(signal, pc, what);
        Outcome::Keep
    }

    /// This thread is over: clear its tid, and if it was the last, the process is too.
    fn end_thread(&mut self, cx: &mut Cx<'_>) -> Flow {
        if self.clear_tid != 0 {
            let at = self.clear_tid;
            let key = self.futex_key(at);
            let _ = self.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), at, 0));
            self.k.futexes.wake(key, 1);
        }
        let last = {
            let mut state = lock(&self.process.state);
            state.threads.remove(&self.tid);
            state.threads.is_empty()
        };
        if last {
            self.finish_process(cx);
        }
        Flow::End
    }

    /// The last thread has ended: give everything back, and tell the parent.
    fn finish_process(&mut self, cx: &mut Cx<'_>) {
        let (status, space, ppid, children, files, hangup) = {
            let mut state = lock(&self.process.state);
            let status = state.exiting.unwrap_or(Status::Exited(0));
            state.zombie = Some(status);
            // A session leader leaving hangs up its terminal.
            let hangup = if state.sid == self.process.pid { state.ctty.take().filter(|t| t.session() == Some(self.process.pid)) } else { None };
            (status, state.space, state.ppid, std::mem::take(&mut state.children), std::mem::take(&mut state.files), hangup)
        };
        drop(files);
        if let Some(tty) = hangup {
            let foreground = tty.foreground();
            tty.set_session(None, 0);
            if foreground > 0 && foreground != self.process.pid {
                self.k.signal_group(foreground, crate::signal::SIGHUP);
            }
        }
        cx.turn(|t| t.destroy_space(space));
        // Orphans go to process 1.
        if self.process.pid != 1 {
            if let Some(init) = self.k.process(1) {
                for &child in &children {
                    if let Some(c) = self.k.process(child) {
                        lock(&c.state).ppid = 1;
                    }
                }
                let mut init = lock(&init.state);
                init.children.extend(children);
                for w in init.child_waiters.drain(..) {
                    w.wake();
                }
            }
        } else {
            let (result, ended) = &self.k.init;
            *lock(result) = Some(status);
            ended.notify_all();
        }
        match self.k.process(ppid) {
            Some(_) => self.k.child_changed(ppid, self.process.pid, Change::Ended(status)),
            // No one will collect it.
            None => {
                lock(&self.k.processes).remove(&self.process.pid);
            }
        }
    }

    fn syscall(&mut self, cx: &mut Cx<'_>, nr: u64, args: [u64; 6]) -> Outcome {
        let Some(call) = sys::decode(self.isa, nr, args) else {
            if self.k.options.trace {
                self.k.host.write(host::Stream::Err, format!("[{}] unknown system call {nr}\n", self.tid).as_bytes());
            }
            return Err(ENOSYS).into();
        };
        let a = call.args;
        let outcome = match call.sys {
            Sys::Exit => Outcome::End,
            Sys::ExitGroup => {
                self.process.end(Status::Exited(a[0] as i32 & 0xff));
                Outcome::End
            }
            Sys::Execve => self.execve(cx, a),
            Sys::RtSigreturn => match self.rt_sigreturn(cx) {
                Ok(()) => Outcome::Keep,
                Err(_) => {
                    self.fault_signal(SIGSEGV, 0, "a bad signal frame");
                    Outcome::Keep
                }
            },
            sys => self.call(cx, sys, a).into(),
        };
        if matches!(outcome, Outcome::Return(EINTR_RETURN)) {
            self.interrupted = Some(Interrupted { sys: call.sys, nr, args });
        }
        if self.k.options.trace {
            let result = match &outcome {
                Outcome::Return(v) => format!("{}", *v as i64),
                Outcome::Keep => "(registers set)".to_string(),
                Outcome::End => "(thread ends)".to_string(),
            };
            self.k.host.write(host::Stream::Err, format!("[{}] {:?}({:#x}, {:#x}, {:#x}, {:#x}) = {result}\n", self.tid, call.sys, a[0], a[1], a[2], a[3]).as_bytes());
        }
        outcome
    }

    fn call(&mut self, cx: &mut Cx<'_>, sys: Sys, a: [u64; 6]) -> Result<u64, Errno> {
        let pid = self.process.pid;
        match sys {
            // Files.
            Sys::Read => self.read(cx, a[0] as i32, a[1], a[2]),
            Sys::Write => self.write(cx, a[0] as i32, a[1], a[2]),
            Sys::Readv => self.readv(cx, a[0] as i32, a[1], a[2]),
            Sys::Writev => self.writev(cx, a[0] as i32, a[1], a[2]),
            Sys::Pread64 => self.pread(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Pwrite64 => self.pwrite(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Openat => self.openat(cx, a[0] as i32, a[1], a[2] as u32, a[3] as u32),
            Sys::Close => self.close(a[0] as i32),
            Sys::Lseek => self.lseek(a[0] as i32, a[1] as i64, a[2] as u32),
            Sys::Fstat => self.fstat(cx, a[0] as i32, a[1]),
            Sys::Newfstatat => self.newfstatat(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Statx => self.statx(cx, a[0] as i32, a[1], a[2], a[4]),
            Sys::Getdents64 => self.getdents64(cx, a[0] as i32, a[1], a[2]),
            Sys::Ioctl => self.ioctl(cx, a[0] as i32, a[1], a[2]),
            // F_GETLK and F_OFD_GETLK: nothing's in the way.
            Sys::Fcntl if a[1] == crate::fs::F_GETLK || a[1] == 36 => self.get_lock(cx, a[0] as i32, a[2]),
            Sys::Fcntl => self.fcntl(a[0] as i32, a[1], a[2]),
            Sys::Dup => self.dup(a[0] as i32),
            Sys::Dup2 => self.dup3(a[0] as i32, a[1] as i32, 0, true),
            Sys::Dup3 => self.dup3(a[0] as i32, a[1] as i32, a[2] as u32, false),
            Sys::Pipe2 => self.pipe2(cx, a[0], a[1] as u32),
            Sys::Faccessat => self.faccessat(cx, a[0] as i32, a[1]),
            Sys::Readlinkat => self.readlinkat(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Mkdirat => self.mkdirat(cx, a[0] as i32, a[1], a[2] as u32),
            Sys::Unlinkat => self.unlinkat(cx, a[0] as i32, a[1], a[2]),
            Sys::Renameat2 => self.renameat(cx, a[0] as i32, a[1], a[2] as i32, a[3]),
            Sys::Symlinkat => self.symlinkat(cx, a[0], a[1] as i32, a[2]),
            Sys::Ftruncate => self.ftruncate(a[0] as i32, a[1]),
            // The disk is one, so syncing a file syncs everything written so far.
            Sys::Fsync => lock(&self.k.disk).sync().map(|()| 0).map_err(Errno::from),
            Sys::Fadvise64 => Ok(0),
            Sys::Statfs => self.statfs(cx, a[1]),
            Sys::Fstatfs => self.fstat_check(a[0] as i32).and_then(|()| self.statfs(cx, a[1])),
            Sys::Getcwd => self.getcwd(cx, a[0], a[1]),
            Sys::Chdir => self.chdir(cx, a[0]),
            Sys::Fchdir => self.fchdir(a[0] as i32),
            Sys::Umask => {
                let mut state = lock(&self.process.state);
                Ok(u64::from(std::mem::replace(&mut state.umask, a[0] as u32 & 0o777)))
            }
            Sys::Getxattr => Err(ENODATA),
            Sys::Listxattr => Ok(0),
            Sys::Setxattr => Err(ENOTSUP),
            Sys::Getgroups => {
                let groups = lock(&self.process.state).creds.groups.clone();
                if a[0] == 0 {
                    return Ok(groups.len() as u64);
                }
                if (a[0] as usize) < groups.len() {
                    return Err(EINVAL);
                }
                let bytes: Vec<u8> = groups.iter().flat_map(|g| g.to_le_bytes()).collect();
                self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[1], &bytes))?;
                Ok(groups.len() as u64)
            }
            Sys::Setgroups => {
                if a[0] > 65536 {
                    return Err(EINVAL);
                }
                let bytes = self.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), a[1], a[0] as usize * 4))?;
                let groups = bytes.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
                lock(&self.process.state).creds.setgroups(groups).map(|()| 0)
            }
            Sys::Fchmodat => self.chmod(cx, a[0] as i32, a[1], a[2] as u32, 0),
            Sys::Fchmod => self.chmod(cx, a[0] as i32, 0, a[1] as u32, AT_EMPTY_PATH),
            Sys::Fchownat => self.chown(cx, a[0] as i32, a[1], a[2] as u32, a[3] as u32, a[4]),
            Sys::Fchown => self.chown(cx, a[0] as i32, 0, a[1] as u32, a[2] as u32, AT_EMPTY_PATH),
            Sys::Utimensat => self.utimensat(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Linkat => self.linkat(cx, a[0] as i32, a[1], a[2] as i32, a[3], a[4]),
            Sys::Flock => self.flock(a[0] as i32, a[1]),
            Sys::CloseRange => self.close_range(a[0], a[1], a[2]),
            // Memory.
            Sys::Mmap => self.mmap(cx, a[0], a[1], a[2], a[3], a[4] as i32, a[5]),
            Sys::Munmap => self.munmap(cx, a[0], a[1]),
            Sys::Mprotect => self.mprotect(cx, a[0], a[1], a[2]),
            Sys::Mremap => self.mremap(cx, a[0], a[1], a[2], a[3]),
            Sys::Madvise => self.madvise(cx, a[0], a[1], a[2]),
            Sys::Brk => self.brk(cx, a[0]),
            // Processes and threads.
            Sys::Clone => self.clone(cx, a[0], a[1], a[2], a[3], a[4]),
            Sys::Clone3 => Err(ENOSYS),
            Sys::Wait4 => self.wait4(cx, a[0] as i32, a[1], a[2]),
            Sys::Getpid => Ok(pid as u64),
            Sys::Gettid => Ok(self.tid as u64),
            Sys::Getppid => Ok(lock(&self.process.state).ppid as u64),
            Sys::Getuid => Ok(u64::from(lock(&self.process.state).creds.uid[0])),
            Sys::Geteuid => Ok(u64::from(lock(&self.process.state).creds.uid[1])),
            Sys::Getgid => Ok(u64::from(lock(&self.process.state).creds.gid[0])),
            Sys::Getegid => Ok(u64::from(lock(&self.process.state).creds.gid[1])),
            Sys::Getresuid | Sys::Getresgid => {
                let creds = lock(&self.process.state).creds.clone();
                let ids = if sys == Sys::Getresuid { creds.uid } else { creds.gid };
                self.mem(cx, true, |t| (0..3).try_for_each(|i| kernels_core::user::write_u32(&mut t.space(), a[i], ids[i])))?;
                Ok(0)
            }
            Sys::Setuid => lock(&self.process.state).creds.setuid(a[0] as u32).map(|()| 0),
            Sys::Setgid => lock(&self.process.state).creds.setgid(a[0] as u32).map(|()| 0),
            Sys::Setreuid => lock(&self.process.state).creds.setreuid(a[0] as u32, a[1] as u32).map(|()| 0),
            Sys::Setregid => lock(&self.process.state).creds.setregid(a[0] as u32, a[1] as u32).map(|()| 0),
            Sys::Setresuid => lock(&self.process.state).creds.setresuid(a[0] as u32, a[1] as u32, a[2] as u32).map(|()| 0),
            Sys::Setresgid => lock(&self.process.state).creds.setresgid(a[0] as u32, a[1] as u32, a[2] as u32).map(|()| 0),
            Sys::Setfsuid => Ok(u64::from(lock(&self.process.state).creds.setfs(false, a[0] as u32))),
            Sys::Setfsgid => Ok(u64::from(lock(&self.process.state).creds.setfs(true, a[0] as u32))),
            Sys::Setpgid => {
                let target = if a[0] == 0 { pid } else { a[0] as i32 };
                let group = if a[1] == 0 { target } else { a[1] as i32 };
                let process = self.k.process(target).ok_or(ESRCH)?;
                if target != pid && lock(&process.state).ppid != pid {
                    return Err(ESRCH);
                }
                let mut state = lock(&process.state);
                if state.sid == target {
                    return Err(EPERM);
                }
                state.pgid = group;
                Ok(0)
            }
            Sys::Getpgid => {
                let target = if a[0] == 0 { pid } else { a[0] as i32 };
                let process = self.k.process(target).ok_or(ESRCH)?;
                Ok(lock(&process.state).pgid as u64)
            }
            Sys::Setsid => {
                let leads = lock(&self.k.processes).values().any(|p| p.pid != pid && lock(&p.state).pgid == pid);
                let mut state = lock(&self.process.state);
                if state.pgid == pid && (leads || state.sid == pid) {
                    return Err(EPERM);
                }
                state.sid = pid;
                state.pgid = pid;
                state.ctty = None;
                Ok(pid as u64)
            }
            Sys::Getsid => {
                let target = if a[0] == 0 { pid } else { a[0] as i32 };
                let process = self.k.process(target).ok_or(ESRCH)?;
                Ok(lock(&process.state).sid as u64)
            }
            Sys::SetTidAddress => {
                self.clear_tid = a[0];
                Ok(self.tid as u64)
            }
            Sys::SetRobustList | Sys::Prctl | Sys::SchedYield | Sys::Membarrier => Ok(0),
            Sys::Rseq => Err(ENOSYS),
            Sys::Futex => self.futex(cx, a),
            Sys::SchedGetaffinity => {
                let cpus = self.k.host.cpus().min(64);
                let mask: u64 = if cpus == 64 { u64::MAX } else { (1 << cpus) - 1 };
                let size = a[1].min(8) as usize;
                self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[2], &mask.to_le_bytes()[..size]))?;
                Ok(size as u64)
            }
            Sys::Prlimit64 => self.prlimit(cx, a[1], a[3]),
            Sys::ArchPrctl => self.arch_prctl(cx, a[0], a[1]),
            Sys::Getrusage => {
                self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[1], &[0; 144]))?;
                Ok(0)
            }
            // Signals.
            Sys::RtSigaction => self.rt_sigaction(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::RtSigprocmask => self.rt_sigprocmask(cx, a[0], a[1], a[2]),
            Sys::Sigaltstack => self.sigaltstack(cx, a[0], a[1]),
            Sys::RtSigpending => self.rt_sigpending(cx, a[0]),
            Sys::RtSigsuspend => self.rt_sigsuspend(cx, a[0]),
            Sys::Kill => self.kill(a[0] as i32, a[1] as i32),
            Sys::Tkill => self.tgkill(None, a[0] as i32, a[1] as i32),
            Sys::Tgkill => self.tgkill(Some(a[0] as i32), a[1] as i32, a[2] as i32),
            // Time.
            Sys::ClockGettime => self.clock_gettime(cx, a[0], a[1]),
            Sys::ClockGetres => self.clock_getres(cx, a[1]),
            Sys::Gettimeofday => self.gettimeofday(cx, a[0]),
            Sys::Time => {
                let now = self.k.host.wall() / 1_000_000_000;
                if a[0] != 0 {
                    self.mem(cx, true, |t| kernels_core::user::write_u64(&mut t.space(), a[0], now))?;
                }
                Ok(now)
            }
            Sys::Nanosleep => self.nanosleep(cx, 1, 0, a[0], a[1]),
            Sys::ClockNanosleep => self.nanosleep(cx, a[0], a[1], a[2], a[3]),
            // The machine.
            Sys::Uname => {
                let machine = if self.isa == Isa::Arm64 { "aarch64" } else { "x86_64" };
                let data = utsname(machine, &self.k.options.hostname);
                self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[0], &data))?;
                Ok(0)
            }
            Sys::Sysinfo => self.sysinfo(cx, a[0]),
            Sys::Getrandom => {
                let mut bytes = vec![0; a[1].min(1 << 20) as usize];
                self.k.host.random(&mut bytes);
                self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[0], &bytes))?;
                Ok(bytes.len() as u64)
            }
            Sys::Socket => self.socket(a[0], a[1], a[2]),
            Sys::Socketpair => self.socketpair(cx, a[0], a[1], a[2], a[3]),
            Sys::Bind => self.bind(cx, a[0] as i32, a[1], a[2]),
            Sys::Listen => self.listen(a[0] as i32, a[1]),
            Sys::Accept4 => self.accept4(cx, a[0] as i32, a[1], a[2], a[3] as u32),
            Sys::Connect => self.connect(cx, a[0] as i32, a[1], a[2]),
            Sys::Getsockname => self.getsockname(cx, a[0] as i32, a[1], a[2], false),
            Sys::Getpeername => self.getsockname(cx, a[0] as i32, a[1], a[2], true),
            Sys::Sendto => self.sendto(cx, a[0] as i32, a[1], a[2], a[3], a[4], a[5]),
            Sys::Recvfrom => self.recvfrom(cx, a[0] as i32, a[1], a[2], a[3], a[4], a[5]),
            Sys::Setsockopt => self.setsockopt(cx, a[0] as i32, a[1] as i32, a[2] as i32, a[3], a[4]),
            Sys::Getsockopt => self.getsockopt(cx, a[0] as i32, a[1] as i32, a[2] as i32, a[3], a[4]),
            Sys::Shutdown => self.shutdown(a[0] as i32, a[1]),
            Sys::Sendmsg => self.sendmsg(cx, a[0] as i32, a[1], a[2]),
            Sys::Recvmsg => self.recvmsg(cx, a[0] as i32, a[1], a[2]),
            Sys::Sendmmsg => self.sendmmsg(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Recvmmsg => self.recvmmsg(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Ppoll => {
                let timeout = self.timespec_timeout(cx, a[2])?;
                if a[3] != 0 && a[4] != 8 {
                    return Err(EINVAL);
                }
                self.with_mask(cx, a[3], |t, cx| t.ppoll(cx, a[0], a[1], timeout))
            }
            Sys::Poll => self.ppoll(cx, a[0], a[1], (a[2] as i32 >= 0).then(|| std::time::Duration::from_millis(a[2]))),
            Sys::Pselect6 => {
                let timeout = self.timespec_timeout(cx, a[4])?;
                // The sixth argument points at the mask's address and size.
                let mask_at = if a[5] == 0 { 0 } else { self.mem(cx, false, |t| kernels_core::user::read_u64(&t.space(), a[5]))? };
                self.with_mask(cx, mask_at, |t, cx| t.select(cx, a[0], [a[1], a[2], a[3]], timeout))
            }
            Sys::Select => {
                let timeout = self.timeval_timeout(cx, a[4])?;
                self.select(cx, a[0], [a[1], a[2], a[3]], timeout)
            }
            Sys::Signalfd => self.signalfd(cx, a[0] as i32, a[1], a[2], a[3] as u32),
            Sys::EpollCreate1 => self.epoll_create(u32::try_from(a[0]).map_err(|_| EINVAL)?),
            Sys::EpollCtl => self.epoll_ctl(cx, a[0] as i32, a[1], a[2] as i32, a[3]),
            Sys::EpollPwait | Sys::EpollPwait2 => {
                let timeout = if sys == Sys::EpollPwait {
                    (a[3] as i32 >= 0).then(|| std::time::Duration::from_millis(u64::from(a[3] as i32 as u32)))
                } else {
                    self.timespec_timeout(cx, a[3])?
                };
                if a[4] != 0 && a[5] != 8 {
                    return Err(EINVAL);
                }
                self.with_mask(cx, a[4], |t, cx| t.epoll_wait(cx, a[0] as i32, a[1], a[2] as i32, timeout))
            }
            Sys::Eventfd2 => self.eventfd(a[0], a[1] as u32),
            Sys::TimerfdCreate => self.timerfd_create(a[0], a[1] as u32),
            Sys::TimerfdSettime => self.timerfd_settime(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::TimerfdGettime => self.timerfd_gettime(cx, a[0] as i32, a[1]),
            Sys::Exit | Sys::ExitGroup | Sys::Execve | Sys::RtSigreturn => Err(ENOSYS),
        }
    }

    fn prlimit(&mut self, cx: &mut Cx<'_>, resource: u64, old: u64) -> Result<u64, Errno> {
        if old == 0 {
            return Ok(0);
        }
        let (cur, max) = match resource {
            3 => (8 << 20, u64::MAX),  // RLIMIT_STACK
            7 => (1024, 4096),         // RLIMIT_NOFILE
            _ => (u64::MAX, u64::MAX), // everything else: no limit
        };
        self.mem(cx, true, |t| {
            kernels_core::user::write_u64(&mut t.space(), old, cur)?;
            kernels_core::user::write_u64(&mut t.space(), old + 8, max)
        })?;
        Ok(0)
    }

    /// x86-64's thread pointer, in fs.
    fn arch_prctl(&mut self, cx: &mut Cx<'_>, code: u64, addr: u64) -> Result<u64, Errno> {
        const SET_FS: u64 = 0x1002;
        const GET_FS: u64 = 0x1003;
        let mut regs = cx.turn(|t| t.thread().regs());
        let exec::Regs::X86(r) = &mut regs else { return Err(EINVAL) };
        match code {
            SET_FS => {
                r.fs_base = addr;
                cx.turn(|t| t.thread().set_regs(&regs));
                Ok(0)
            }
            GET_FS => {
                let fs = r.fs_base;
                self.mem(cx, true, |t| kernels_core::user::write_u64(&mut t.space(), addr, fs))?;
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }

    fn sysinfo(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        let uptime = self.k.host.monotonic() / 1_000_000_000;
        let processes = lock(&self.k.processes).len() as u16;
        let (total, free) = {
            let mut exec = lock(&self.k.exec);
            let size = exec.memory().size();
            (size.limit, size.available)
        };
        let mut b = Vec::with_capacity(112);
        b.extend(uptime.to_le_bytes());
        b.extend([0; 24]); // load averages
        b.extend(total.to_le_bytes());
        b.extend(free.to_le_bytes());
        b.extend([0; 32]); // shared, buffers, swap
        b.extend(processes.to_le_bytes());
        b.extend([0; 6]);
        b.extend([0; 16]); // high memory
        b.extend(1u32.to_le_bytes()); // mem_unit
        b.resize(112, 0);
        self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }
}

/// Process 1: `argv[0]` loaded from the disk, with stdin, stdout and stderr the console if there is one, else the
/// host's own.
pub(crate) fn start_init(k: &Arc<Kernel>, cx: &mut Cx<'_>, argv: &[String], env: &[String]) -> Result<Task, Errno> {
    let pid = k.new_id();
    if let Some(console) = &k.console {
        console.set_session(Some(pid), pid);
    }
    let mut state = ProcState {
        ppid: 0,
        pgid: pid,
        sid: pid,
        isa: Isa::host(),
        space: SpaceId(0),
        regions: Regions::default(),
        mmap: 0..0,
        brk: (0, 0),
        files: Files::standard(k.console.clone()),
        cwd: (layers::ROOT, "/".to_string()),
        umask: 0o022,
        creds: creds::Creds::default(),
        exe: String::new(),
        threads: BTreeMap::new(),
        exiting: None,
        zombie: None,
        children: Vec::new(),
        child_waiters: Vec::new(),
        signals: Signals::default(),
        pending: 0,
        infos: BTreeMap::new(),
        stopped: None,
        report: None,
        ctty: k.console.clone(),
        sigreturn_at: 0,
    };
    let path = argv.first().ok_or(ENOENT)?.clone();
    load::exec_into(k, cx, &mut state, &path, argv.to_vec(), env.to_vec())?;
    let waiter = Waiter::new();
    let signals = ThreadSignals::with_mask(0);
    state.threads.insert(pid, Live { handle: cx.handle(), waiter: waiter.clone(), signals: signals.clone() });
    let isa = state.isa;
    let process = Arc::new(Process { pid, state: Mutex::new(state), signal_watchers: Default::default() });
    lock(&k.processes).insert(pid, process.clone());
    Ok(Task::new(k.clone(), process, pid, waiter, signals, isa))
}
