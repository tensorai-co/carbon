//! A stub process: starting it, making system calls on its behalf, and ending it.
//!
//! Every CPU has a stub process of its own, an address space of its own that shows that CPU's space, over the same
//! guest memory (a memfd every stub maps shared). So CPUs in different spaces (a parent and the child it just forked)
//! run at once, and the threads of one program, each on a stub of its own, see each other's writes as they would on
//! real hardware.
//!
//! `ptrace` answers only the thread that traces, which is the thread that started the stub. So a CPU's stub is started
//! by the thread that drives the CPU, and everything here is done from it. Ending a stub works from anywhere.

use std::collections::BTreeSet;

use exec_core::contract::MirrorRun;

use super::elf::{self, At};
use super::{seccomp, sys};

/// Where guest memory's memfd is in the stub.
pub const GUEST_FD: i32 = 100;
/// The signal that preempts the stub. A program can't catch it, having no system calls; Carbon swallows it.
pub const PREEMPT_SIGNAL: i32 = libc::SIGURG;
/// Chooses how a program's calls are caught, for testing the other way: `dispatch` or `sysemu`.
const CATCH_VAR: &str = "CARBON_SANDBOX_CATCH";

/// How a program's system calls reach Carbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catch {
    /// Syscall User Dispatch (Linux 5.11): every `syscall`, with no part of the stub exempt, raises `SIGSYS`, which
    /// stops the stub.
    Dispatch,
    /// `PTRACE_SYSEMU`: the stub stops at every system call's entry, and the call never runs. For where there's no
    /// dispatch: kernels before 5.11, and gVisor (Cloud Run's first generation).
    Sysemu,
}

pub struct Process {
    pub pid: i32,
    pub at: At,
    pub catch: Catch,
    /// Syscall User Dispatch is on, so Carbon's own calls open the selector first.
    dispatching: bool,
    /// The program's pages that are executable, as `mirror` mapped them: where a fault at the instruction itself can
    /// only be a write, the fetch having worked.
    executable: BTreeSet<u64>,
}

/// Why a stub didn't start.
enum Start {
    /// The kernel had something where the stub goes: another try may find it free.
    Taken(String),
    Failed(String),
}

impl From<String> for Start {
    fn from(why: String) -> Start {
        Start::Failed(why)
    }
}

fn stopped_with(status: i32, signal: i32) -> bool {
    libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) == signal
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

impl Process {
    /// Starts a stub with guest memory at `GUEST_FD`, stopped, everything the kernel gave it gone, its system calls
    /// going to Carbon, and its own locked down to what the filter allows. At `base` and catching calls by `catch`, as
    /// the machine's other stubs do; or, for its first, wherever it goes highest and however this host can.
    pub fn spawn(guest_fd: i32, base: Option<u64>, catch: Option<Catch>) -> Result<Process, String> {
        let places: Vec<(u64, usize)> = match base {
            Some(base) => vec![(base, 8)],
            // The top three times, since what's in its way there (the new stub's stack) is random; the rest once.
            None => elf::PLACES.iter().enumerate().map(|(i, &place)| (place, if i == 0 { 3 } else { 1 })).collect(),
        };
        let mut last = String::new();
        for (base, tries) in places {
            for _ in 0..tries {
                match Process::start(guest_fd, At { base }, catch) {
                    Ok(process) => return Ok(process),
                    Err(Start::Taken(why)) => last = why,
                    Err(Start::Failed(why)) => return Err(why),
                }
            }
        }
        Err(last)
    }

    fn start(guest_fd: i32, at: At, catch: Option<Catch>) -> Result<Process, Start> {
        let pid = launch(guest_fd, at)?;
        // Dropped, it's killed: from here every failure ends it.
        let mut process = Process { pid, at, catch: Catch::Dispatch, dispatching: false, executable: BTreeSet::new() };
        sys::setoptions(pid, libc::PTRACE_O_EXITKILL | libc::PTRACE_O_TRACESYSGOOD | libc::PTRACE_O_TRACESECCOMP)?;
        process.clear_loader_mappings()?;
        process.unblock_signals()?;
        process.catch = process.set_up_catch(catch)?;
        process.lock_down()?;
        Ok(process)
    }

    /// Runs system calls in the stub, one after another through the trampoline, and gives each one's result. The stub
    /// must be stopped, and is left as it was.
    pub fn calls(&self, calls: &[(i64, [u64; 6])]) -> Result<Vec<i64>, String> {
        let saved = sys::getregs(self.pid)?;
        if self.dispatching {
            sys::poke(self.pid, self.at.selector(), u64::from(sys::SYSCALL_DISPATCH_FILTER_ALLOW))?;
        }
        let results = self.run_calls(&saved, calls);
        if self.dispatching {
            sys::poke(self.pid, self.at.selector(), u64::from(sys::SYSCALL_DISPATCH_FILTER_BLOCK))?;
        }
        sys::setregs(self.pid, &saved)?;
        results
    }

    fn run_calls(&self, saved: &sys::UserRegs, calls: &[(i64, [u64; 6])]) -> Result<Vec<i64>, String> {
        let mut results = Vec::with_capacity(calls.len());
        for &(nr, args) in calls {
            let mut regs = *saved;
            regs.rip = self.at.trampoline();
            regs.rax = nr as u64;
            // Whatever call the stub was stopped in is skipped, not carried on with these registers: gVisor's stop
            // for a seccomp event is at a call that's still to run.
            regs.orig_rax = u64::MAX;
            (regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9) = (args[0], args[1], args[2], args[3], args[4], args[5]);
            sys::setregs(self.pid, &regs)?;
            // It ends at the trampoline's `int3`. A preempt that comes meanwhile is swallowed: whoever sent it has also
            // marked the CPU, so it's seen before the program next runs.
            loop {
                sys::cont(self.pid, 0)?;
                let status = sys::wait(self.pid)?;
                if stopped_with(status, libc::SIGTRAP) {
                    break;
                }
                if libc::WIFSTOPPED(status) && ![libc::SIGSYS, libc::SIGSEGV, sys::SYSCALL_STOP].contains(&libc::WSTOPSIG(status)) {
                    continue;
                }
                return Err(format!("a call in the stub didn't come back (status {status:#x})"));
            }
            results.push(sys::getregs(self.pid)?.rax as i64);
        }
        Ok(results)
    }

    fn checked(&self, calls: &[(&str, i64, [u64; 6])]) -> Result<(), String> {
        let results = self.calls(&calls.iter().map(|&(_, nr, args)| (nr, args)).collect::<Vec<_>>())?;
        for (&(what, ..), r) in calls.iter().zip(results) {
            if (-4095..0).contains(&r) {
                return Err(format!("{what}: {}", std::io::Error::from_raw_os_error(-r as i32)));
            }
        }
        Ok(())
    }

    /// Makes the stub show `runs` of guest memory, all in one stop.
    pub fn mirror(&mut self, runs: &[MirrorRun]) -> Result<(), String> {
        let mut calls = Vec::with_capacity(runs.len());
        for run in runs {
            let (page, len) = match *run {
                MirrorRun::Map { page, len, .. } | MirrorRun::Protect { page, len, .. } | MirrorRun::Unmap { page, len } => (page, len),
            };
            if page.saturating_add(len) > self.at.base {
                return Err(format!("{page:#x} is the stub's"));
            }
            let exec = match *run {
                MirrorRun::Map { offset, prot, .. } => {
                    let flags = (libc::MAP_SHARED | libc::MAP_FIXED) as u64;
                    calls.push((libc::SYS_mmap, [page, len, host_prot(prot), flags, GUEST_FD as u64, offset]));
                    prot.exec()
                }
                MirrorRun::Protect { prot, .. } => {
                    calls.push((libc::SYS_mprotect, [page, len, host_prot(prot), 0, 0, 0]));
                    prot.exec()
                }
                MirrorRun::Unmap { .. } => {
                    calls.push((libc::SYS_munmap, [page, len, 0, 0, 0, 0]));
                    false
                }
            };
            let gone: Vec<u64> = self.executable.range(page..page + len).copied().collect();
            for page in gone {
                self.executable.remove(&page);
            }
            if exec {
                self.executable.extend((page..page + len).step_by(4096));
            }
        }
        let results = self.calls(&calls)?;
        for (run, r) in runs.iter().zip(results) {
            let what = match run {
                MirrorRun::Map { page, .. } if r >= 0 && r as u64 != *page => return Err(format!("guest pages went to {r:#x}, not {page:#x}")),
                MirrorRun::Map { .. } => "mapping guest pages",
                MirrorRun::Protect { .. } => "protecting guest pages",
                MirrorRun::Unmap { .. } => "unmapping guest pages",
            };
            if (-4095..0).contains(&r) {
                return Err(format!("{what} ({run:?}): errno {}", -r));
            }
        }
        Ok(())
    }

    pub fn is_executable(&self, page: u64) -> bool {
        self.executable.contains(&page)
    }

    /// Takes away everything the kernel gave the stub but its own two pages (its stack, the vDSO and vvar), so every
    /// other address is the program's. Everything above and below them goes, which needs no look at `/proc`. (The
    /// vsyscall page is above user space, and can't go.)
    fn clear_loader_mappings(&self) -> Result<(), String> {
        let (base, end) = (self.at.base, self.at.base + elf::SIZE);
        let mut calls = vec![("unmapping what the kernel put below the stub", libc::SYS_munmap, [0, base, 0, 0, 0, 0])];
        if end < elf::TOP {
            calls.push(("unmapping what the kernel put above the stub", libc::SYS_munmap, [end, elf::TOP - end, 0, 0, 0, 0]));
        }
        self.checked(&calls)
    }

    /// The stub started with every signal blocked, as it was started; the preempt signal has to reach it.
    fn unblock_signals(&self) -> Result<(), String> {
        self.checked(&[("unblocking the stub's signals", libc::SYS_rt_sigprocmask, [libc::SIG_SETMASK as u64, self.at.empty_sigset(), 0, 8, 0, 0])])
    }

    /// Syscall User Dispatch where the kernel has it, unless told otherwise; else `PTRACE_SYSEMU`, checked to work.
    fn set_up_catch(&mut self, wanted: Option<Catch>) -> Result<Catch, String> {
        let wanted = wanted.or_else(|| match std::env::var(CATCH_VAR).as_deref() {
            Ok("sysemu") => Some(Catch::Sysemu),
            Ok("dispatch") => Some(Catch::Dispatch),
            _ => None,
        });
        let mut no_dispatch = None;
        if wanted != Some(Catch::Sysemu) {
            match self.set_up_dispatch() {
                Ok(()) => return Ok(Catch::Dispatch),
                Err(why) if wanted == Some(Catch::Dispatch) => return Err(why),
                Err(why) => no_dispatch = Some(why),
            }
        }
        match self.check_sysemu() {
            Ok(()) => Ok(Catch::Sysemu),
            Err(why) => Err(match no_dispatch {
                Some(dispatch) => format!("{dispatch}, and {why}"),
                None => why,
            }),
        }
    }

    /// Syscall User Dispatch with no part of the stub exempt: with the selector at "block", every system call raises
    /// `SIGSYS`, which is how a program's calls reach Carbon, and a program that jumps to the trampoline is caught like
    /// any other. Carbon opens the selector for its own calls (`calls`), and it's read-only to the stub.
    fn set_up_dispatch(&mut self) -> Result<(), String> {
        self.checked(&[(
            "turning on Syscall User Dispatch (it needs Linux 5.11)",
            libc::SYS_prctl,
            [sys::PR_SET_SYSCALL_USER_DISPATCH, sys::PR_SYS_DISPATCH_ON, 0, 0, self.at.selector(), 0],
        )])?;
        self.dispatching = true;
        sys::poke(self.pid, self.at.selector(), u64::from(sys::SYSCALL_DISPATCH_FILTER_BLOCK))
    }

    /// Whether `PTRACE_SYSEMU` stops the stub at a system call without running it: tried on `getpid`, which the
    /// filter-to-be would kill the stub for, so a call that ran would show.
    fn check_sysemu(&self) -> Result<(), String> {
        let saved = sys::getregs(self.pid)?;
        let mut regs = saved;
        regs.rip = self.at.trampoline();
        regs.rax = libc::SYS_getpid as u64;
        sys::setregs(self.pid, &regs)?;
        sys::sysemu(self.pid).map_err(|why| format!("ptrace can't emulate system calls here ({why})"))?;
        let status = sys::wait(self.pid)?;
        if !stopped_with(status, sys::SYSCALL_STOP) {
            return Err(format!("PTRACE_SYSEMU didn't stop the stub at its system call (status {status:#x})"));
        }
        let at = sys::getregs(self.pid)?;
        if at.orig_rax != libc::SYS_getpid as u64 || at.rip != self.at.after_syscall() {
            return Err(format!("PTRACE_SYSEMU stopped the stub somewhere else ({:#x}, call {})", at.rip, at.orig_rax as i64));
        }
        sys::setregs(self.pid, &saved)
    }

    /// The seccomp filter, then the stub's own pages narrowed: code read and execute, data read-only, so a program
    /// can't open the selector by writing it.
    fn lock_down(&self) -> Result<(), String> {
        let otherwise = if self.catch == Catch::Sysemu { seccomp::RET_TRACE } else { seccomp::RET_KILL_PROCESS };
        let filter = seccomp::filter(self.at.after_syscall(), otherwise);
        for (i, chunk) in filter.chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            sys::poke(self.pid, self.at.filter() + i as u64 * 8, u64::from_le_bytes(word))?;
        }
        // struct sock_fprog { unsigned short len; struct sock_filter *filter; }
        sys::poke(self.pid, self.at.filter_program(), u64::from(seccomp::len()))?;
        sys::poke(self.pid, self.at.filter_program() + 8, self.at.filter())?;
        let rx = (libc::PROT_READ | libc::PROT_EXEC) as u64;
        self.checked(&[
            ("installing the stub's seccomp filter", libc::SYS_seccomp, [sys::SECCOMP_SET_MODE_FILTER, 0, self.at.filter_program(), 0, 0, 0]),
            ("narrowing the stub's code", libc::SYS_mprotect, [self.at.base, 0x1000, rx, 0, 0, 0]),
            ("narrowing the stub's data", libc::SYS_mprotect, [self.at.data(), 0x1000, libc::PROT_READ as u64, 0, 0, 0]),
        ])
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the stub, which PTRACE_O_EXITKILL and PR_SET_PDEATHSIG also guard; collected so it doesn't linger.
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(self.pid, &mut status, 0);
        }
    }
}

pub fn host_prot(prot: exec_core::Prot) -> u64 {
    let mut p = libc::PROT_NONE;
    if prot.read() {
        p |= libc::PROT_READ;
    }
    if prot.write() {
        p |= libc::PROT_WRITE;
    }
    if prot.exec() {
        p |= libc::PROT_EXEC;
    }
    p as u64
}

/// What the child between `clone` and the stub's exec needs, on the starting thread's stack.
#[repr(C)]
struct Child {
    guest_fd: i32,
    stub_fd: i32,
    /// Where it writes the step that failed and its errno, if one does. Closed at exec.
    report: i32,
    /// The most descriptors there can be, for a kernel without `close_range`.
    max_fd: i32,
    empty: *const libc::c_char,
    argv: [*const libc::c_char; 2],
    envp: [*const libc::c_char; 1],
}

/// The steps the child takes, as it reports them.
const STEP_GUEST_FD: u32 = 1;
const STEP_NO_NEW_PRIVS: u32 = 2;
const STEP_TRACEME: u32 = 3;
const STEP_EXEC: u32 = 4;

/// Where the stub's ELF is run from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Memfd,
    /// A file in the temporary directory, unlinked once it's open: for where memfds can't be executed
    /// (`vm.memfd_noexec = 2`).
    TempFile,
}

/// Starts the stub's process, stopped at its exec: its pid.
fn launch(guest_fd: i32, at: At) -> Result<i32, Start> {
    let elf = at.build();
    let mut why = String::new();
    for source in [Source::Memfd, Source::TempFile] {
        let stub_fd = match stub_file(&elf, source) {
            Ok(fd) => fd,
            Err(error) => {
                why = error;
                continue;
            }
        };
        match clone_exec(guest_fd, stub_fd) {
            Ok(pid) => return Ok(pid),
            Err(Launch::NoExec(error)) => why = error,
            Err(Launch::Taken(error)) => return Err(Start::Taken(error)),
            Err(Launch::Failed(error)) => return Err(Start::Failed(error)),
        }
    }
    Err(Start::Failed(why))
}

fn stub_file(elf: &[u8], source: Source) -> Result<i32, String> {
    let fd = match source {
        Source::Memfd => {
            // SAFETY: a name and flags.
            let mut fd = unsafe { libc::memfd_create(c"carbon-stub".as_ptr(), libc::MFD_CLOEXEC | sys::MFD_EXEC) };
            if fd < 0 && errno() == libc::EINVAL {
                // A kernel from before MFD_EXEC, whose memfds are all executable.
                // SAFETY: as above.
                fd = unsafe { libc::memfd_create(c"carbon-stub".as_ptr(), libc::MFD_CLOEXEC) };
            }
            if fd < 0 {
                return Err(format!("making the stub: {}", std::io::Error::last_os_error()));
            }
            fd
        }
        Source::TempFile => {
            static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("carbon-stub-{}-{n}", std::process::id()));
            use std::os::unix::fs::OpenOptionsExt;
            let written = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, elf));
            // Opened again read-only: a file open for writing can't be executed.
            let file = written.and_then(|()| std::fs::OpenOptions::new().read(true).custom_flags(libc::O_CLOEXEC).open(&path));
            let _ = std::fs::remove_file(&path);
            let file = file.map_err(|e| format!("writing the stub to {}: {e}", path.display()))?;
            return Ok(std::os::fd::IntoRawFd::into_raw_fd(file));
        }
    };
    // SAFETY: the memfd just made, and the ELF's bytes.
    let written = unsafe { libc::write(fd, elf.as_ptr().cast(), elf.len()) };
    if written != elf.len() as isize {
        // SAFETY: as above.
        unsafe { libc::close(fd) };
        return Err("writing the stub".to_string());
    }
    Ok(fd)
}

enum Launch {
    /// This kind of file can't be executed here.
    NoExec(String),
    Taken(String),
    Failed(String),
}

/// `clone`s a child that shares this process's memory until it execs the stub (as `vfork` does, so nothing of this
/// process is copied, however big it is), traced by this thread. Every signal is blocked meanwhile, so none runs a
/// handler of this process's in the child; the stub unblocks its own once it's stopped (`unblock_signals`).
fn clone_exec(guest_fd: i32, stub_fd: i32) -> Result<i32, Launch> {
    let mut pipe = [0i32; 2];
    // SAFETY: an out-parameter.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        // SAFETY: the stub's file, which nothing else has.
        unsafe { libc::close(stub_fd) };
        return Err(Launch::Failed(format!("making a pipe: {}", std::io::Error::last_os_error())));
    }
    // SAFETY: an out-parameter.
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    let empty = c"";
    let child = Child {
        guest_fd,
        stub_fd,
        report: pipe[1],
        max_fd: limit.rlim_cur.clamp(1024, 1 << 16) as i32,
        empty: empty.as_ptr(),
        argv: [empty.as_ptr(), std::ptr::null()],
        envp: [std::ptr::null()],
    };
    let mut stack = vec![0u8; 64 << 10];
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !15;
    // SAFETY: signal sets, the child's own stack, and arguments that outlive it: with CLONE_VFORK this thread waits
    // until the child has exec'd or exited.
    let (pid, error) = unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        let mut old: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all);
        libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut old);
        let pid = libc::clone(child_main, top as *mut libc::c_void, libc::CLONE_VM | libc::CLONE_VFORK | libc::SIGCHLD, (&raw const child).cast_mut().cast());
        let error = std::io::Error::last_os_error();
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
        libc::close(pipe[1]);
        libc::close(stub_fd);
        (pid, error)
    };
    drop(stack);
    if pid < 0 {
        // SAFETY: the pipe's other end.
        unsafe { libc::close(pipe[0]) };
        return Err(Launch::Failed(format!("starting the stub: {error}")));
    }
    let mut report = [0u8; 8];
    // SAFETY: the pipe's read end, whose every write end is closed by now: what the child wrote, or nothing.
    let got = unsafe {
        let got = libc::read(pipe[0], report.as_mut_ptr().cast(), 8);
        libc::close(pipe[0]);
        got
    };
    if got == 8 {
        let step = u32::from_le_bytes(report[..4].try_into().unwrap_or_default());
        let code = i32::from_le_bytes(report[4..].try_into().unwrap_or_default());
        // SAFETY: the child, which has exited.
        unsafe { libc::waitpid(pid, &mut 0, 0) };
        let error = std::io::Error::from_raw_os_error(code);
        return Err(match step {
            STEP_TRACEME => Launch::Failed(format!(
                "ptrace(PTRACE_TRACEME) was refused ({error}): this host or container forbids ptrace (a seccomp profile, AppArmor or SELinux, or kernel.yama.ptrace_scope = 3)"
            )),
            STEP_EXEC if [libc::EACCES, libc::EPERM].contains(&code) => Launch::NoExec(format!("executing the stub was refused ({error}): memfds and the temporary directory are both non-executable here")),
            // A kernel that says so, rather than ending the new process: it couldn't put the stub where it goes.
            STEP_EXEC if [libc::ENOMEM, libc::EEXIST, libc::EINVAL].contains(&code) => Launch::Taken(format!("executing the stub: {error}")),
            STEP_EXEC => Launch::Failed(format!("executing the stub: {error}")),
            STEP_NO_NEW_PRIVS => Launch::Failed(format!("PR_SET_NO_NEW_PRIVS: {error}")),
            STEP_GUEST_FD => Launch::Failed(format!("giving the stub guest memory: {error}")),
            _ => Launch::Failed(format!("starting the stub, step {step}: {error}")),
        });
    }
    let status = sys::wait(pid).map_err(Launch::Failed)?;
    if stopped_with(status, libc::SIGTRAP) {
        return Ok(pid);
    }
    // Exec'd, then ended before its first instruction: the kernel couldn't put it where it goes, as when the stack it
    // made for it is there already.
    if libc::WIFSTOPPED(status) {
        // SAFETY: the stub that didn't start.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, &mut 0, 0);
        }
    }
    Err(Launch::Taken(format!("the stub didn't start (status {status:#x})")))
}

/// The child, from `clone` to the stub's exec: raw calls only, on memory it shares with the thread that started it.
extern "C" fn child_main(arg: *mut libc::c_void) -> libc::c_int {
    // SAFETY: `Child`, which outlives this: its thread waits until this execs or exits.
    unsafe {
        let child = &*(arg as *const Child);
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
        if child.guest_fd == GUEST_FD {
            libc::fcntl(GUEST_FD, libc::F_SETFD, 0);
        } else if libc::dup2(child.guest_fd, GUEST_FD) < 0 {
            fail(child.report, STEP_GUEST_FD);
        }
        // Every other descriptor closes at exec: the stub has guest memory and nothing else.
        let below = libc::syscall(libc::SYS_close_range, 0, GUEST_FD - 1, sys::CLOSE_RANGE_CLOEXEC);
        let above = libc::syscall(libc::SYS_close_range, GUEST_FD + 1, u32::MAX, sys::CLOSE_RANGE_CLOEXEC);
        if below != 0 || above != 0 {
            // A kernel before 5.11's CLOSE_RANGE_CLOEXEC, or gVisor without it: each one, then.
            for fd in (0..child.max_fd).filter(|&fd| fd != GUEST_FD) {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        // Seccomp without CAP_SYS_ADMIN needs this, and the stub has no use for privileges anyway.
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            fail(child.report, STEP_NO_NEW_PRIVS);
        }
        if libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) != 0 {
            fail(child.report, STEP_TRACEME);
        }
        libc::syscall(libc::SYS_execveat, child.stub_fd, child.empty, child.argv.as_ptr(), child.envp.as_ptr(), libc::AT_EMPTY_PATH);
        fail(child.report, STEP_EXEC)
    }
}

/// Tells the starting thread which step failed, and how, and ends the child.
unsafe fn fail(report: i32, step: u32) -> ! {
    // SAFETY: errno, then a write and an exit.
    unsafe {
        let code = *libc::__errno_location();
        let mut message = [0u8; 8];
        message[..4].copy_from_slice(&step.to_le_bytes());
        message[4..].copy_from_slice(&code.to_le_bytes());
        libc::write(report, message.as_ptr().cast(), 8);
        libc::_exit(127)
    }
}
