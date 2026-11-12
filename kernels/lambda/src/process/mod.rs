//! Processes and threads: each guest thread's [`Task`], what happens at each stop, and the system call dispatcher.

mod clone;
pub(crate) mod creds;
mod elf;
mod futex2;
mod kcmp;
mod limits;
mod load;
mod pidfd;
pub(crate) mod prctl;
pub(crate) mod ptrace;
mod robust;
mod sched;
mod usage;
mod vm;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use exec::{CpuHandle, Exit, FpRegs, Isa, SpaceId, Stop};
use kernels_core::lock;
use kernels_core::mm::Regions;
use kernels_core::thread::{Cx, Flow, Guest, Times};
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
    /// Told when a signal becomes pending, for `signalfd`s and `sigtimedwait` being waited on.
    pub signal_watchers: kernels_core::wait::Watchers,
    /// Told when it ends, for its pidfds.
    pub exit_watchers: kernels_core::wait::Watchers,
    /// Whether it has a timer on CPU time, to check at each of its threads' stops.
    pub cpu_timers: std::sync::atomic::AtomicBool,
    /// Whether it has seccomp filters, to check each system call against.
    pub filtered: std::sync::atomic::AtomicBool,
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
    /// Its root (`chroot`), and where that is in its mount namespace.
    pub root: (layers::Ino, String),
    /// Its namespaces (mount, UTS, IPC, and the shared ones).
    pub ns: crate::ns::Namespaces,
    pub umask: u32,
    /// Its resource limits (`prlimit64`).
    pub limits: limits::Limits,
    /// Its nice value, scheduling policy, CPU affinity and I/O priority.
    pub sched: sched::Sched,
    /// `personality`'s execution domain and flags.
    pub personality: u32,
    /// The CPU time of its threads that have ended, and of its children that have been waited for.
    pub usage: usage::Usage,
    /// Its interval timers and POSIX timers.
    pub timers: crate::timer::ProcTimers,
    /// `mempolicy`'s mode (with its flags) and nodes.
    pub mempolicy: (u32, u64),
    /// Its SysV shared memory attachments: each address, and the segment and length there.
    pub shm_attached: BTreeMap<u64, (i32, u64)>,
    /// What its `SEM_UNDO` semaphore operations come to, to undo when it ends: by set and semaphore.
    pub sem_undo: BTreeMap<(i32, u16), i32>,
    /// Its AIO contexts, by ID: not inherited, and gone at `execve`, as they're its memory's.
    pub aio: BTreeMap<u64, Arc<crate::fs::aio::AioCtx>>,
    /// Its seccomp filters, `no_new_privs` and whether it's dumpable.
    pub security: crate::security::Security,
    /// `PR_SET_NAME`'s name, if it's been given one since its last `execve`.
    pub comm: Option<String>,
    pub creds: creds::Creds,
    pub exe: String,
    pub threads: BTreeMap<i32, Live>,
    /// Set once, when the process starts to end; every thread ends at its next chance.
    pub exiting: Option<Status>,
    /// Set when its last thread has ended, until its parent collects it.
    pub zombie: Option<Status>,
    pub children: Vec<i32>,
    /// The threads it traces (`ptrace`), which its waits hear of as of its children.
    pub tracees: Vec<ptrace::Tracing>,
    /// Threads in `wait4`, to wake when a child changes.
    pub child_waiters: Vec<Arc<Waiter>>,
    pub signals: Signals,
    /// Signals for the process as a whole, which any thread not blocking them may take, and what each is about.
    pub pending: u64,
    pub infos: crate::signal::Queues,
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
    /// Its CPU time so far.
    pub times: Arc<Times>,
    /// Its robust futex list's head (`set_robust_list`).
    pub robust: Arc<AtomicU64>,
    /// Its tracing, for its tracer.
    pub trace: Arc<ptrace::Trace>,
}

impl Process {
    pub fn new(pid: i32, state: ProcState) -> Arc<Process> {
        let filtered = std::sync::atomic::AtomicBool::new(state.security.seccomp.is_some());
        Arc::new(Process { pid, state: Mutex::new(state), signal_watchers: Default::default(), exit_watchers: Default::default(), cpu_timers: Default::default(), filtered })
    }

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
    /// Its CPU time, and the host thread's CPU time when it was last added to.
    pub times: Arc<Times>,
    pub cpu_mark: u64,
    pub robust: Arc<AtomicU64>,
    /// Whether, and by whom, it's traced (`ptrace`).
    pub trace: Arc<ptrace::Trace>,
    /// What `exit` gave, for its tracer to hear when it ends.
    exit_code: i32,
    /// A traced thread or process `clone` just made, with the event its creator stops at for it.
    forked: Option<(i32, i32)>,
}

/// What a system call returns for EINTR.
const EINTR_RETURN: u64 = (-(EINTR.0 as i64)) as u64;

impl Task {
    pub fn new(k: Arc<Kernel>, process: Arc<Process>, tid: i32, waiter: Arc<Waiter>, signals: Arc<ThreadSignals>, isa: Isa) -> Task {
        let cpu_mark = k.host.thread_cpu();
        Task {
            k,
            process,
            tid,
            waiter,
            clear_tid: 0,
            isa,
            signals,
            fault_info: None,
            suspend_mask: None,
            altstack: None,
            saved_fp: Vec::new(),
            interrupted: None,
            times: Arc::default(),
            cpu_mark,
            robust: Arc::default(),
            trace: Arc::default(),
            exit_code: 0,
            forked: None,
        }
    }

    /// How its process sees it, running on `handle`.
    pub fn live(&self, handle: CpuHandle) -> Live {
        Live { handle, waiter: self.waiter.clone(), signals: self.signals.clone(), times: self.times.clone(), robust: self.robust.clone(), trace: self.trace.clone() }
    }
}

impl Guest for Task {
    /// A traced thread starts stopped: at the SIGSTOP or `PTRACE_EVENT_STOP` its tracer gave it.
    fn start(&mut self, cx: &mut Cx<'_>) -> Flow {
        crate::fs::fanotify::set_caller(self.process.pid, self.tid);
        if !self.traced() {
            return Flow::Run;
        }
        match self.deliver(cx, None) {
            Flow::Run => Flow::Run,
            Flow::End => self.end_thread(cx),
        }
    }

    /// The program stopped: what it ran until now was its own time, and dealing with the stop is lambda's.
    fn exit(&mut self, cx: &mut Cx<'_>, exit: Exit) -> Flow {
        self.account_stop();
        self.check_cpu_timers();
        let flow = self.stopped(cx, exit);
        self.account_resume();
        flow
    }
}

impl Task {
    fn stopped(&mut self, cx: &mut Cx<'_>, exit: Exit) -> Flow {
        let stop = cx.turn(|t| t.stop(exit));
        if self.ending() {
            return self.end_thread(cx);
        }
        self.interrupted = None;
        // The system call it's in, if its tracer stops at each call's exit.
        let mut traced_call = None;
        let outcome = match stop {
            None | Some(Stop::Preempted) => Outcome::Keep,
            Some(Stop::Syscall { nr, args }) if self.tracing_syscalls() => {
                traced_call = Some(nr);
                match self.syscall_entry_stop(cx, nr, args) {
                    Some((nr, args)) => {
                        traced_call = Some(nr);
                        self.syscall(cx, nr, args)
                    }
                    // Skipped: its result is as the tracer left the registers.
                    None => Outcome::Keep,
                }
            }
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
                Some(call) if value == EINTR_RETURN => {
                    // Its tracer sees EINTR at the call's exit; restarting it puts back what it was.
                    if traced_call.is_some() {
                        cx.turn(|t| t.thread().set_return(value));
                    }
                    Some(call)
                }
                _ => {
                    cx.turn(|t| t.thread().set_return(value));
                    None
                }
            },
            Outcome::Keep => None,
            Outcome::End => return self.end_thread(cx),
        };
        if let Some(nr) = traced_call {
            self.syscall_exit_stop(cx, nr);
            if self.ending() {
                return self.end_thread(cx);
            }
        }
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

    /// A futex's key: its address in this process's space; or, unless it's `private`, its place in the shared
    /// memory it's in, so processes sharing that memory wait and wake on the same futex (a process-shared mutex, a
    /// POSIX semaphore).
    pub fn futex_key(&self, addr: u64, private: bool) -> u64 {
        let state = lock(&self.process.state);
        if !private
            && let Some((range, area)) = state.regions.find(addr)
            && let crate::mm::Backing::Shared { mem, offset, private: false } = &area.backing
        {
            return 1 << 63 | mem.ino << 40 ^ (offset + addr - range.start);
        }
        state.space.0 << 48 ^ addr
    }

    /// The process `id` names: one with that pid, or one with a thread of that ID.
    /// This process's UTS and IPC namespaces.
    pub(crate) fn uts_ns(&self) -> Arc<crate::ns::UtsNs> {
        lock(&self.process.state).ns.uts.clone()
    }

    pub(crate) fn ipc_ns(&self) -> Arc<crate::ns::IpcNs> {
        lock(&self.process.state).ns.ipc.clone()
    }

    pub(crate) fn process_of(&self, id: i32) -> Option<Arc<Process>> {
        if id == 0 {
            return Some(self.process.clone());
        }
        self.k.process(id).or_else(|| lock(&self.k.processes).values().find(|p| lock(&p.state).threads.contains_key(&id)).cloned())
    }

    /// The program did something it can't carry on from, and gets `signal` for it.
    fn fatal(&mut self, signal: i32, pc: u64, what: &str) -> Outcome {
        self.fault_signal(signal, pc, what);
        Outcome::Keep
    }

    /// This thread is over: let go of its robust futexes, clear its tid, add its time to its process's, and if it was
    /// the last thread, the process is over too.
    fn end_thread(&mut self, cx: &mut Cx<'_>) -> Flow {
        self.release_robust(cx);
        if self.clear_tid != 0 {
            let at = self.clear_tid;
            let key = self.futex_key(at, false);
            let _ = self.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), at, 0));
            self.k.futexes.wake(key, 1);
        }
        self.account_resume();
        let status = lock(&self.process.state).exiting.unwrap_or(Status::Exited(self.exit_code));
        self.ptrace_ended(status);
        let last = {
            let mut state = lock(&self.process.state);
            state.threads.remove(&self.tid);
            state.usage.ended = usage::add(state.usage.ended, self.times.get());
            state.threads.is_empty()
        };
        if last {
            self.finish_process(cx);
        }
        Flow::End
    }

    /// The last thread has ended: give everything back, and tell the parent.
    fn finish_process(&mut self, cx: &mut Cx<'_>) {
        let (status, space, ppid, children, files, hangup, attached) = {
            let mut state = lock(&self.process.state);
            let status = state.exiting.unwrap_or(Status::Exited(0));
            state.zombie = Some(status);
            crate::timer::cancel_all(&self.k, &mut state);
            let attached = !std::mem::take(&mut state.shm_attached).is_empty();
            let undo = std::mem::take(&mut state.sem_undo);
            // A session leader leaving hangs up its terminal.
            let hangup = if state.sid == self.process.pid { state.ctty.take().filter(|t| t.session() == Some(self.process.pid)) } else { None };
            (status, state.space, state.ppid, std::mem::take(&mut state.children), std::mem::take(&mut state.files), hangup, (attached, undo))
        };
        let (attached, undo) = attached;
        if !undo.is_empty() {
            crate::ipc::apply_undo(&self.ipc_ns(), self.process.pid, undo);
        }
        if attached {
            self.shm_tidy();
        }
        drop(files);
        self.ptrace_let_go();
        self.process.exit_watchers.notify();
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
        if self.filtered()
            && let Some(outcome) = self.seccomp_check(cx, nr, &args)
        {
            return outcome;
        }
        let Some(call) = sys::decode(self.isa, nr, args) else {
            if self.k.options.trace {
                self.k.host.write(host::Stream::Err, format!("[{}] unknown system call {nr}\n", self.tid).as_bytes());
            }
            return Err(ENOSYS).into();
        };
        let a = call.args;
        let outcome = match call.sys {
            Sys::Exit => {
                self.exit_code = a[0] as i32 & 0xff;
                self.ptrace_exit(cx, Status::Exited(self.exit_code));
                Outcome::End
            }
            Sys::ExitGroup => {
                self.ptrace_exit(cx, Status::Exited(a[0] as i32 & 0xff));
                self.process.end(Status::Exited(a[0] as i32 & 0xff));
                Outcome::End
            }
            Sys::Execve | Sys::Execveat => {
                let outcome = if call.sys == Sys::Execve { self.execve(cx, a) } else { self.execveat(cx, a) };
                if matches!(outcome, Outcome::Keep) {
                    self.ptrace_exec(cx);
                }
                outcome
            }
            Sys::Clone => {
                let result = self.clone(cx, a[0], a[1], a[2], a[3], a[4]);
                if let Some((event, tid)) = self.forked.take() {
                    self.ptrace_forked(cx, event, tid);
                }
                result.into()
            }
            Sys::RtSigreturn => match self.rt_sigreturn(cx) {
                Ok(()) => Outcome::Keep,
                Err(_) => {
                    self.fault_signal(SIGSEGV, 0, "a bad signal frame");
                    Outcome::Keep
                }
            },
            sys => self.call(cx, sys, a).into(),
        };
        // `restart_syscall`'s own EINTR is its answer, not a call to restart.
        if matches!(outcome, Outcome::Return(EINTR_RETURN)) && call.sys != Sys::RestartSyscall {
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
            Sys::Getdents64 => self.getdents(cx, a[0] as i32, a[1], a[2], false),
            Sys::Getdents => self.getdents(cx, a[0] as i32, a[1], a[2], true),
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
            Sys::Statfs => self.statfs(cx, Some(a[0]), 0, a[1]),
            Sys::Fstatfs => self.statfs(cx, None, a[0] as i32, a[1]),
            Sys::Getcwd => self.getcwd(cx, a[0], a[1]),
            Sys::Chdir => self.chdir(cx, a[0]),
            Sys::Fchdir => self.fchdir(a[0] as i32),
            Sys::Umask => {
                let mut state = lock(&self.process.state);
                Ok(u64::from(std::mem::replace(&mut state.umask, a[0] as u32 & 0o777)))
            }
            // Extended attributes. value/name are pointers; size and flags unsigned.
            Sys::Setxattr => self.setxattr(cx, crate::fs::xattr::Form::Path, a[0], a[1], a[2], a[3], a[4]),
            Sys::Lsetxattr => self.setxattr(cx, crate::fs::xattr::Form::Link, a[0], a[1], a[2], a[3], a[4]),
            Sys::Fsetxattr => self.setxattr(cx, crate::fs::xattr::Form::Fd, a[0], a[1], a[2], a[3], a[4]),
            Sys::Getxattr => self.getxattr(cx, crate::fs::xattr::Form::Path, a[0], a[1], a[2], a[3]),
            Sys::Lgetxattr => self.getxattr(cx, crate::fs::xattr::Form::Link, a[0], a[1], a[2], a[3]),
            Sys::Fgetxattr => self.getxattr(cx, crate::fs::xattr::Form::Fd, a[0], a[1], a[2], a[3]),
            Sys::Listxattr => self.listxattr(cx, crate::fs::xattr::Form::Path, a[0], a[1], a[2]),
            Sys::Llistxattr => self.listxattr(cx, crate::fs::xattr::Form::Link, a[0], a[1], a[2]),
            Sys::Flistxattr => self.listxattr(cx, crate::fs::xattr::Form::Fd, a[0], a[1], a[2]),
            Sys::Removexattr => self.removexattr(cx, crate::fs::xattr::Form::Path, a[0], a[1]),
            Sys::Lremovexattr => self.removexattr(cx, crate::fs::xattr::Form::Link, a[0], a[1]),
            Sys::Fremovexattr => self.removexattr(cx, crate::fs::xattr::Form::Fd, a[0], a[1]),
            Sys::Setxattrat => self.setxattrat(cx, a[0] as i32, a[1], a[2] as u32 as u64, a[3], a[4], a[5]),
            Sys::Getxattrat => self.getxattrat(cx, a[0] as i32, a[1], a[2] as u32 as u64, a[3], a[4], a[5]),
            Sys::Listxattrat => self.listxattrat(cx, a[0] as i32, a[1], a[2] as u32 as u64, a[3], a[4]),
            Sys::Removexattrat => self.removexattrat(cx, a[0] as i32, a[1], a[2] as u32 as u64, a[3]),
            Sys::Mknodat => self.mknodat(cx, a[0] as i32, a[1], a[2] as u32, a[3]),
            Sys::Openat2 => self.openat2(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::FileGetattr => self.file_getattr(cx, a[0] as i32, a[1], a[2], a[3], a[4] as u32 as u64),
            Sys::FileSetattr => self.file_setattr(cx, a[0] as i32, a[1], a[2], a[3], a[4] as u32 as u64),
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
            Sys::Fchmodat2 => {
                if a[3] & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
                    return Err(EINVAL);
                }
                self.chmod(cx, a[0] as i32, a[1], a[2] as u32, a[3])
            }
            Sys::Utime => self.utime(cx, a[0], a[1]),
            Sys::Futimesat => self.futimesat(cx, a[0] as i32, a[1], a[2]),
            Sys::Truncate => self.truncate(cx, a[0], a[1]),
            Sys::Fallocate => self.fallocate(a[0] as i32, a[1], a[2], a[3]),
            // On 64-bit ISAs the offset is one register; the high half beside it is 0.
            Sys::Preadv => self.preadv(cx, a[0] as i32, a[1], a[2], a[3] as i64, None),
            Sys::Pwritev => self.pwritev(cx, a[0] as i32, a[1], a[2], a[3] as i64, None),
            Sys::Preadv2 => self.preadv(cx, a[0] as i32, a[1], a[2], a[3] as i64, Some(a[5])),
            Sys::Pwritev2 => self.pwritev(cx, a[0] as i32, a[1], a[2], a[3] as i64, Some(a[5])),
            Sys::Sendfile => self.sendfile(cx, a[0] as i32, a[1] as i32, a[2], a[3]),
            Sys::CopyFileRange => self.copy_file_range(cx, a[0] as i32, a[1], a[2] as i32, a[3], a[4], a[5]),
            Sys::Splice => self.splice(cx, a[0] as i32, a[1], a[2] as i32, a[3], a[4], a[5]),
            Sys::Tee => self.tee(a[0] as i32, a[1] as i32, a[2], a[3]),
            Sys::Vmsplice => self.vmsplice(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Readahead => self.readahead(a[0] as i32),
            Sys::Sync => {
                // `sync` can't fail; what couldn't be saved is reported when the machine ends.
                let _ = lock(&self.k.disk).sync();
                Ok(0)
            }
            Sys::Syncfs => self.file(a[0] as i32).and_then(|_| lock(&self.k.disk).sync().map(|()| 0).map_err(Errno::from)),
            Sys::SyncFileRange => self.sync_file_range(a[0] as i32, a[1], a[2], a[3]),
            Sys::NameToHandleAt => self.name_to_handle_at(cx, a[0] as i32, a[1], a[4]),
            Sys::OpenByHandleAt => self.open_by_handle_at(cx, a[0] as i32, a[1]),
            Sys::IoprioGet => self.ioprio_get(a[0], a[1] as i32),
            Sys::IoprioSet => self.ioprio_set(a[0], a[1] as i32, a[2]),
            // Memory.
            Sys::Mmap => self.mmap(cx, a[0], a[1], a[2], a[3], a[4] as i32, a[5]),
            Sys::Munmap => self.munmap(cx, a[0], a[1]),
            Sys::Mprotect => self.mprotect(cx, a[0], a[1], a[2]),
            Sys::Mremap => self.mremap(cx, a[0], a[1], a[2], a[3]),
            Sys::Madvise => self.madvise(cx, a[0], a[1], a[2]),
            Sys::Brk => self.brk(cx, a[0]),
            Sys::Msync => self.msync(a[0], a[1], a[2]),
            Sys::Mincore => self.mincore(cx, a[0], a[1], a[2]),
            Sys::Mlock => self.mlock(cx, a[0], a[1], 0),
            Sys::Mlock2 => self.mlock(cx, a[0], a[1], a[2]),
            Sys::Munlock => self.munlock(a[0], a[1]),
            Sys::Mlockall => self.mlockall(a[0]),
            Sys::Munlockall => Ok(0),
            Sys::Mseal => self.mseal(a[0], a[1], a[2]),
            Sys::InotifyInit1 => self.inotify_init1(a[0] as u32),
            Sys::InotifyAddWatch => self.inotify_add_watch(cx, a[0] as i32, a[1], a[2] as u32),
            Sys::InotifyRmWatch => self.inotify_rm_watch(a[0] as i32, a[1] as i32),
            Sys::IoSetup => self.io_setup(cx, a[0], a[1]),
            Sys::IoDestroy => self.io_destroy(cx, a[0]),
            Sys::IoSubmit => self.io_submit(cx, a[0], a[1], a[2]),
            Sys::IoCancel => self.io_cancel(a[0]),
            Sys::IoGetevents => self.io_getevents(cx, a[0], a[1], a[2], a[3], a[4], 0),
            Sys::IoPgetevents => self.io_getevents(cx, a[0], a[1], a[2], a[3], a[4], a[5]),
            // Security. A pid is an int.
            Sys::Capget => self.capget(cx, a[0], a[1]),
            Sys::Capset => self.capset(cx, a[0], a[1]),
            Sys::Seccomp => self.seccomp(cx, a[0], a[1], a[2]),
            Sys::LsmGetSelfAttr => self.lsm_get_self_attr(cx, a[0] as u32 as u64, a[2], a[3] as u32 as u64),
            Sys::LsmSetSelfAttr => self.lsm_set_self_attr(a[0] as u32 as u64, a[3] as u32 as u64),
            Sys::LsmListModules => self.lsm_list_modules(cx, a[0], a[1], a[2] as u32 as u64),
            // Landlock, as on a kernel booted with it off.
            Sys::LandlockCreateRuleset | Sys::LandlockAddRule | Sys::LandlockRestrictSelf => Err(EOPNOTSUPP),
            Sys::Bpf | Sys::PerfEventOpen => Err(EPERM),
            // The machine's administration.
            Sys::Sethostname => self.set_name(cx, false, a[0], a[1]),
            Sys::Setdomainname => self.set_name(cx, true, a[0], a[1]),
            Sys::Reboot => self.reboot(a[0], a[1], a[2]),
            Sys::Syslog => self.syslog(cx, a[0], a[1], a[2]),
            Sys::Vhangup => self.vhangup(),
            Sys::Quotactl => self.quotactl(a[0], None),
            Sys::QuotactlFd => self.quotactl(a[1], Some(a[0] as i32)),
            Sys::KexecLoad | Sys::KexecFileLoad | Sys::InitModule | Sys::FinitModule | Sys::DeleteModule | Sys::Acct => Err(EPERM),
            Sys::Iopl => self.iopl(a[0] as u32 as u64),
            Sys::Ioperm => self.ioperm(a[0], a[1], a[2] as u32 as u64),
            // Namespaces.
            Sys::Unshare => self.unshare(a[0]),
            Sys::Setns => self.setns(a[0] as i32, a[1] as u32 as u64),
            Sys::Listns => self.listns(cx, a[0], a[1], a[2], a[3]),
            // The new mount API.
            Sys::Fsopen => self.fsopen(cx, a[0], a[1]),
            Sys::Fsconfig => self.fsconfig(cx, a[0] as i32, a[1], a[2], a[3], a[4]),
            Sys::Fsmount => self.fsmount(a[0] as i32, a[1], a[2]),
            Sys::Fspick => self.fspick(cx, a[0] as i32, a[1], a[2]),
            Sys::MoveMount => self.move_mount(cx, a[0] as i32, a[1], a[2] as i32, a[3], a[4]),
            Sys::OpenTree => self.open_tree(cx, a[0] as i32, a[1], a[2]),
            Sys::MountSetattr => self.mount_setattr(cx, a[0] as i32, a[1], a[2], a[3], a[4]),
            // open_tree_attr: open_tree, then its attrs, in one call; its attrs aren't taken yet.
            Sys::OpenTreeAttr => self.open_tree(cx, a[0] as i32, a[1], a[2]),
            // Mounts and roots. Flags are an unsigned long for mount, an int for umount2.
            Sys::Mount => self.mount(cx, a[0], a[1], a[2], a[3], a[4]),
            Sys::Umount2 => self.umount2(cx, a[0], a[1] as u32 as u64),
            Sys::PivotRoot => self.pivot_root(cx, a[0], a[1]),
            Sys::Chroot => self.chroot(cx, Some(a[0]), 0),
            Sys::Fchroot => self.chroot(cx, None, a[0] as i32),
            // Modes and flags are ints: only their 32 bits count.
            Sys::SetMempolicy => self.set_mempolicy(cx, a[0] as u32 as u64, a[1], a[2]),
            Sys::GetMempolicy => self.get_mempolicy(cx, a[0], a[1], a[2], a[3], a[4]),
            Sys::Mbind => self.mbind(cx, a[0], a[1], a[2] as u32 as u64, a[3], a[4], a[5]),
            Sys::MigratePages => self.migrate_pages(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::MovePages => self.move_pages(cx, a[0] as i32, a[1], a[2], a[3], a[4], a[5] as u32 as u64),
            Sys::SetMempolicyHomeNode => self.set_mempolicy_home_node(a[0], a[1], a[2], a[3]),
            Sys::PkeyAlloc => self.pkey_alloc(a[0], a[1]),
            Sys::PkeyFree => Err(EINVAL),
            Sys::PkeyMprotect => self.pkey_mprotect(cx, a[0], a[1], a[2], a[3] as i32),
            Sys::MapShadowStack => Err(EOPNOTSUPP),
            Sys::ProcessMadvise => self.process_madvise(cx, a[0] as i32, a[1], a[2], a[3] as u32 as u64, a[4]),
            Sys::ProcessMrelease => self.process_mrelease(a[0] as i32, a[1]),
            Sys::Cachestat => self.cachestat(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Swapon | Sys::Swapoff => Err(EPERM),
            Sys::MemfdCreate => self.memfd_create(cx, a[0], a[1] as u32 as u64),
            Sys::MemfdSecret => self.memfd_secret(a[0] as u32 as u64),
            Sys::Shmget => self.shmget(a[0] as i32, a[1], a[2] as u32 as u64),
            Sys::Shmat => self.shmat(cx, a[0] as i32, a[1], a[2] as u32 as u64),
            Sys::Shmdt => self.shmdt(cx, a[0]),
            Sys::Shmctl => self.shmctl(cx, a[0] as i32, a[1] as u32 as u64, a[2]),
            // IDs, keys, flags and commands are ints.
            Sys::Msgget => self.msgget(a[0] as i32, a[1] as u32 as u64),
            Sys::Msgsnd => self.msgsnd(cx, a[0] as i32, a[1], a[2], a[3] as u32 as u64),
            Sys::Msgrcv => self.msgrcv(cx, a[0] as i32, a[1], a[2], a[3] as i64, a[4] as u32 as u64),
            Sys::Msgctl => self.msgctl(cx, a[0] as i32, a[1] as u32 as u64, a[2]),
            Sys::Semget => self.semget(a[0] as i32, a[1] as u32 as u64, a[2] as u32 as u64),
            Sys::Semtimedop => self.semtimedop(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Semop => self.semtimedop(cx, a[0] as i32, a[1], a[2], 0),
            Sys::Semctl => self.semctl(cx, a[0] as i32, a[1] as u32 as u64, a[2] as u32 as u64, a[3]),
            Sys::MqOpen => self.mq_open(cx, a[0], a[1] as u32, a[2] as u32, a[3]),
            Sys::MqUnlink => self.mq_unlink(cx, a[0]),
            Sys::MqTimedsend => self.mq_timedsend(cx, a[0] as i32, a[1], a[2], a[3] as u32 as u64, a[4]),
            Sys::MqTimedreceive => self.mq_timedreceive(cx, a[0] as i32, a[1], a[2], a[3], a[4]),
            Sys::MqNotify => self.mq_notify(cx, a[0] as i32, a[1]),
            Sys::MqGetsetattr => self.mq_getsetattr(cx, a[0] as i32, a[1], a[2]),
            Sys::FutexWait => self.futex_wait(cx, a[0], a[1], a[2], a[3] as u32 as u64, a[4], a[5] as u32 as u64),
            Sys::FutexWake => self.futex_wake(a[0], a[1], a[2] as u32 as u64, a[3] as u32 as u64),
            Sys::FutexRequeue => self.futex_requeue(cx, a[0], a[1] as u32 as u64, a[2] as u32 as u64, a[3] as u32 as u64),
            Sys::FutexWaitv => self.futex_waitv(cx, a[0], a[1] as u32 as u64, a[2] as u32 as u64, a[3], a[4] as u32 as u64),
            // Processes and threads.
            Sys::Clone => self.clone(cx, a[0], a[1], a[2], a[3], a[4]),
            Sys::Clone3 => Err(ENOSYS),
            // Requests and pids are ints (`addr` and `data` are whole words).
            Sys::Ptrace => self.ptrace(cx, a[0] as u32 as u64, a[1] as i32, a[2], a[3]),
            Sys::ProcessVmReadv => self.process_vm(cx, false, a[0] as i32, a[1], a[2], a[3], a[4], a[5]),
            Sys::ProcessVmWritev => self.process_vm(cx, true, a[0] as i32, a[1], a[2], a[3], a[4], a[5]),
            Sys::Statmount => self.statmount(cx, a[0], a[1], a[2], a[3]),
            Sys::Listmount => self.listmount(cx, a[0], a[1], a[2], a[3]),
            Sys::FanotifyInit => self.fanotify_init(a[0] as u32, a[1] as u32),
            Sys::FanotifyMark => self.fanotify_mark(cx, a[0] as i32, a[1] as u32, a[2], a[3] as i32, a[4]),
            Sys::RemapFilePages => self.remap_file_pages(cx, a[0], a[1], a[2], a[3]),
            Sys::Ustat => self.ustat(cx, a[0] as u32, a[1]),
            Sys::Sysfs => self.sysfs(cx, a[0] as u32 as u64, a[1], a[2]),
            Sys::ModifyLdt => self.modify_ldt(cx, a[0], a[1], a[2]),
            // Called from no trampoline: Linux's answers, ENXIO, and SIGILL for uretprobe.
            Sys::Uprobe => Err(ENXIO),
            Sys::Uretprobe => {
                self.force_signal(SIGILL, crate::signal::Info::default());
                Err(EPERM)
            }
            Sys::Wait4 => self.wait4(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Waitid => self.waitid(cx, a[0], a[1] as i32, a[2], a[3], a[4]),
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
            Sys::SetRobustList => self.set_robust_list(a[0], a[1]),
            Sys::GetRobustList => self.get_robust_list(cx, a[0] as i32, a[1], a[2]),
            Sys::SchedYield | Sys::Membarrier => Ok(0),
            Sys::Prctl => self.prctl(cx, a[0], [a[1], a[2], a[3], a[4]]),
            // A system call restarted by its own number: lambda restarts them where they were, so there's never one.
            Sys::RestartSyscall => Err(EINTR),
            Sys::Getpriority => self.getpriority(a[0], a[1] as i32),
            Sys::Setpriority => self.setpriority(a[0], a[1] as i32, a[2] as i32),
            Sys::SchedSetscheduler => self.sched_setscheduler(cx, a[0] as i32, a[1], a[2]),
            Sys::SchedGetscheduler => self.sched_getscheduler(a[0] as i32),
            Sys::SchedSetparam => self.sched_setparam(cx, a[0] as i32, a[1]),
            Sys::SchedGetparam => self.sched_getparam(cx, a[0] as i32, a[1]),
            Sys::SchedGetPriorityMax => self.sched_priority_bound(a[0], true),
            Sys::SchedGetPriorityMin => self.sched_priority_bound(a[0], false),
            Sys::SchedRrGetInterval => self.sched_rr_get_interval(cx, a[0] as i32, a[1]),
            Sys::SchedSetattr => self.sched_setattr(cx, a[0] as i32, a[1], a[2]),
            Sys::SchedGetattr => self.sched_getattr(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::SchedSetaffinity => self.sched_setaffinity(cx, a[0] as i32, a[1], a[2]),
            Sys::Getcpu => self.getcpu(cx, a[0], a[1]),
            Sys::Kcmp => self.kcmp(a[0] as i32, a[1] as i32, a[2], a[3], a[4]),
            Sys::Personality => {
                let mut state = lock(&self.process.state);
                let old = state.personality;
                if a[0] as u32 != u32::MAX {
                    state.personality = a[0] as u32;
                }
                Ok(u64::from(old))
            }
            Sys::Times => self.times(cx, a[0]),
            Sys::Rseq => Err(ENOSYS),
            Sys::Futex => self.futex(cx, a),
            Sys::SchedGetaffinity => self.sched_getaffinity(cx, a[0] as i32, a[1], a[2]),
            Sys::Prlimit64 => self.prlimit(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::Getrlimit => self.prlimit(cx, 0, a[0], 0, a[1]),
            Sys::Setrlimit => self.prlimit(cx, 0, a[0], a[1], 0),
            Sys::ArchPrctl => self.arch_prctl(cx, a[0], a[1]),
            // `who` is an int: -1 arrives as 0xffffffff.
            Sys::Getrusage => self.getrusage(cx, i64::from(a[0] as i32), a[1]),
            // Signals.
            Sys::RtSigaction => self.rt_sigaction(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::RtSigprocmask => self.rt_sigprocmask(cx, a[0], a[1], a[2]),
            Sys::Sigaltstack => self.sigaltstack(cx, a[0], a[1]),
            Sys::RtSigpending => self.rt_sigpending(cx, a[0]),
            Sys::RtSigsuspend => self.rt_sigsuspend(cx, a[0]),
            Sys::Kill => self.kill(a[0] as i32, a[1] as i32),
            Sys::Tkill => self.tgkill(None, a[0] as i32, a[1] as i32),
            Sys::Tgkill => self.tgkill(Some(a[0] as i32), a[1] as i32, a[2] as i32),
            Sys::RtSigtimedwait => self.rt_sigtimedwait(cx, a[0], a[1], a[2], a[3]),
            Sys::RtSigqueueinfo => self.rt_sigqueueinfo(cx, a[0] as i32, None, a[1] as i32, a[2]),
            Sys::RtTgsigqueueinfo => self.rt_sigqueueinfo(cx, a[0] as i32, Some(a[1] as i32), a[2] as i32, a[3]),
            Sys::Pause => self.pause(),
            Sys::Alarm => self.alarm(a[0]),
            // `which` is an int: a negative one arrives as 32 bits, and isn't one.
            Sys::Getitimer => self.getitimer(cx, a[0] as u32 as u64, a[1]),
            Sys::Setitimer => self.setitimer(cx, a[0] as u32 as u64, a[1], a[2]),
            Sys::TimerCreate => self.timer_create(cx, a[0], a[1], a[2]),
            Sys::TimerSettime => self.timer_settime(cx, a[0] as i32, a[1], a[2], a[3]),
            Sys::TimerGettime => self.timer_gettime(cx, a[0] as i32, a[1]),
            Sys::TimerGetoverrun => self.timer_getoverrun(a[0] as i32),
            Sys::TimerDelete => self.timer_delete(a[0] as i32),
            Sys::ClockSettime => self.clock_settime(cx, a[0] as u32 as u64, a[1]),
            Sys::Settimeofday => self.settimeofday(cx, a[0], a[1]),
            Sys::Adjtimex => self.adjtimex(cx, CLOCK_REALTIME, a[0]),
            Sys::ClockAdjtime => self.adjtimex(cx, a[0] as u32 as u64, a[1]),
            Sys::PidfdOpen => self.pidfd_open(a[0] as i32, a[1]),
            Sys::PidfdSendSignal => self.pidfd_send_signal(cx, a[0] as i32, a[1] as i32, a[2], a[3]),
            Sys::PidfdGetfd => self.pidfd_getfd(a[0] as i32, a[1] as i32, a[2]),
            // Time.
            Sys::ClockGettime => self.clock_gettime(cx, a[0], a[1]),
            Sys::ClockGetres => self.clock_getres(cx, a[0], a[1]),
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
                // PER_LINUX32 (as `setarch i686` asks for) names the 32-bit machine.
                let linux32 = lock(&self.process.state).personality & 0xff == 0x0008;
                let machine = match (self.isa, linux32) {
                    (Isa::Arm64, false) => "aarch64",
                    (Isa::Arm64, true) => "armv8l",
                    (_, false) => "x86_64",
                    (_, true) => "i686",
                };
                let names = lock(&self.uts_ns().names).clone();
                let data = utsname(machine, &names.host, &names.domain);
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
            Sys::Exit | Sys::ExitGroup | Sys::Execve | Sys::Execveat | Sys::RtSigreturn => Err(ENOSYS),
        }
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
        root: (layers::ROOT, "/".to_string()),
        ns: k.boot_ns.clone(),
        umask: 0o022,
        limits: limits::Limits::default(),
        sched: sched::Sched::new(k.host.cpus()),
        personality: 0,
        usage: usage::Usage::default(),
        timers: Default::default(),
        mempolicy: (0, 0),
        shm_attached: BTreeMap::new(),
        sem_undo: BTreeMap::new(),
        aio: BTreeMap::new(),
        security: Default::default(),
        comm: None,
        creds: creds::Creds::default(),
        exe: String::new(),
        threads: BTreeMap::new(),
        exiting: None,
        zombie: None,
        children: Vec::new(),
        tracees: Vec::new(),
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
    let isa = state.isa;
    let process = Process::new(pid, state);
    let task = Task::new(k.clone(), process.clone(), pid, Waiter::new(), ThreadSignals::with_mask(0), isa);
    lock(&process.state).threads.insert(pid, task.live(cx.handle()));
    lock(&k.processes).insert(pid, process);
    Ok(task)
}
