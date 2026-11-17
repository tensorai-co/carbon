//! A stub process: starting it, setting it up, letting it go, and the system calls it makes in itself for Carbon.
//!
//! Every CPU has a stub process of its own, an address space of its own that shows that CPU's space, over the same
//! guest memory (a memfd every stub maps shared). So CPUs in different spaces (a parent and the child it just forked)
//! run at once, and the threads of one program, each on a stub of its own, see each other's writes as they would on
//! real hardware.
//!
//! A stub is started by the thread that will drive its CPU, and sets itself up: Carbon writes the calls that do it
//! into its mailbox before it starts (its loader's mappings gone, the handler installed for its signals, its program's
//! system calls sent to that handler by Syscall User Dispatch, or by seccomp's `SECCOMP_RET_TRAP` where there's no
//! dispatch, and its own locked down), and the stub makes them from its handler's page, then waits in its handler. From
//! there it meets Carbon only through the mailbox (`handler.rs`, `mailbox.rs`). Nothing of it needs `ptrace`, but
//! `strict` mode, where Carbon takes the stub under `ptrace`, briefly, to change its mappings itself.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use exec_core::contract::MirrorRun;

use super::elf::{self, At};
use super::handler;
use super::mailbox::{Gone, Mailbox};
use super::seccomp::{self, Mapper};
use super::sys;

/// Where guest memory's memfd is in the stub.
pub const GUEST_FD: i32 = 100;
/// Where the mailbox's memfd is in the stub, until it maps it.
pub const MAILBOX_FD: i32 = 101;
/// The signal that preempts the stub. A program can't catch it, having no system calls; its handler hands it to
/// Carbon.
pub const PREEMPT_SIGNAL: i32 = libc::SIGURG;
/// The signals the handler takes.
const SIGNALS: [i32; 7] = [libc::SIGSYS, libc::SIGSEGV, libc::SIGBUS, libc::SIGFPE, libc::SIGILL, libc::SIGTRAP, PREEMPT_SIGNAL];
/// Chooses how a program's calls are caught, for testing the other way: `dispatch` or `seccomp`.
const CATCH_VAR: &str = "CARBON_SANDBOX_CATCH";
/// Chooses who maps guest memory into a stub: `fast` (its handler) or `strict` (Carbon, through `ptrace`).
const MODE_VAR: &str = "CARBON_SANDBOX";

/// How a program's system calls reach the stub's handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catch {
    /// Syscall User Dispatch (Linux 5.11), with the handler's page exempt.
    Dispatch,
    /// seccomp's `SECCOMP_RET_TRAP`, for where there's no dispatch: kernels before 5.11, and gVisor (Cloud Run's first
    /// generation).
    Seccomp,
}

/// Who maps guest memory into a stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Its handler, when Carbon asks. A machine's processes could reach each other's memory through it, about as
    /// processes of one user in a container can with `ptrace`.
    Fast,
    /// Carbon, with the stub stopped under `ptrace`; the handler can't map at all. For machines whose processes
    /// mustn't reach each other. Needs Syscall User Dispatch.
    Strict,
}

/// Whether this host has Syscall User Dispatch (Linux 5.11), asked of this process once: turning it off where it
/// isn't on is harmless, and refused only where there's no such thing.
fn has_dispatch() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    // SAFETY: a prctl that changes nothing.
    *HAS.get_or_init(|| unsafe { libc::prctl(sys::PR_SET_SYSCALL_USER_DISPATCH as i32, 0, 0, 0, 0) } == 0)
}

/// Whether the kernel lets programs use `rdfsbase` and the rest (Linux 5.9, on a CPU that has them).
fn has_fsgsbase() -> bool {
    // SAFETY: reading the auxiliary vector.
    unsafe { libc::getauxval(libc::AT_HWCAP2) & sys::HWCAP2_FSGSBASE != 0 }
}

pub struct Process {
    pub pid: i32,
    pub at: At,
    pub catch: Catch,
    pub mode: Mode,
    pub mailbox: Mailbox,
    /// The handler reads and writes `fs` and `gs` itself (`handler::FLAG_FSGSBASE`).
    pub fsgsbase: bool,
    /// The program's pages that are executable, as `mirror` mapped them: where a fault at the instruction itself can
    /// only be a write, the fetch having worked.
    executable: BTreeSet<u64>,
    /// In `fast`, the mapping calls `mirror` asked for that the handler makes as it returns to the program, with what
    /// each was for: their results are checked at the next event.
    pending: Vec<(MirrorRun, i64, [u64; 6])>,
}

/// The most mapping calls left for the handler to make as it returns: the rest of the mailbox's room is for the two
/// that set `fs` and `gs`.
const MOST_PENDING: usize = handler::MAX_OPS as usize - 2;

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

impl Gone {
    fn why(&self) -> String {
        match self {
            Gone::Ended(status) => format!("the stub ended (status {status:#x})"),
            Gone::Garbled(why) => format!("the stub's mailbox is garbled: {why}"),
        }
    }
}

/// The calls a stub makes to set itself up, in order, each with what it's for.
type Setup = Vec<(&'static str, i64, [u64; 6])>;

impl Process {
    /// Starts a stub with guest memory at `GUEST_FD`, set up and waiting in its handler. At `base`, catching calls by
    /// `catch` and mapping by `mode`, as the machine's other stubs do; or, for its first, wherever it goes highest and
    /// however this host can.
    pub fn spawn(guest_fd: i32, base: Option<u64>, catch: Option<Catch>, mode: Option<Mode>) -> Result<Process, String> {
        let catch = match catch.or_else(|| match std::env::var(CATCH_VAR).as_deref() {
            Ok("seccomp") => Some(Catch::Seccomp),
            Ok("dispatch") => Some(Catch::Dispatch),
            _ => None,
        }) {
            Some(Catch::Dispatch) if !has_dispatch() => return Err("Syscall User Dispatch was asked for, and this host doesn't have it (it needs Linux 5.11)".to_string()),
            Some(catch) => catch,
            None if has_dispatch() => Catch::Dispatch,
            None => Catch::Seccomp,
        };
        let mode = mode.unwrap_or_else(|| match std::env::var(MODE_VAR).as_deref() {
            Ok("strict") => Mode::Strict,
            _ => Mode::Fast,
        });
        if mode == Mode::Strict && catch != Catch::Dispatch {
            return Err("strict mode needs Syscall User Dispatch (Linux 5.11), which this host doesn't have".to_string());
        }
        let places: Vec<(u64, usize)> = match base {
            Some(base) => vec![(base, 8)],
            // The top three times, since what's in its way there (the new stub's stack) is random; the rest once.
            None => elf::PLACES.iter().enumerate().map(|(i, &place)| (place, if i == 0 { 3 } else { 1 })).collect(),
        };
        let mut last = String::new();
        for (base, tries) in places {
            for _ in 0..tries {
                match Process::start(guest_fd, At { base }, catch, mode) {
                    Ok(process) => return Ok(process),
                    Err(Start::Taken(why)) => last = why,
                    Err(Start::Failed(why)) => return Err(why),
                }
            }
        }
        Err(last)
    }

    fn start(guest_fd: i32, at: At, catch: Catch, mode: Mode) -> Result<Process, Start> {
        let mut mailbox = Mailbox::new(at.shared())?;
        let fsgsbase = has_fsgsbase();
        mailbox.set(handler::FLAGS, if fsgsbase { handler::FLAG_FSGSBASE } else { 0 });
        let setup = setup(&mut mailbox, at, catch, mode);
        mailbox.queue(&setup.iter().map(|&(_, nr, args)| (nr, args)).collect::<Vec<_>>());
        let pid = launch(guest_fd, mailbox.fd, at)?;
        // Dropped, it's killed: from here every failure ends it.
        let process = Process { pid, at, catch, mode, mailbox, fsgsbase, executable: BTreeSet::new(), pending: Vec::new() };
        match process.mailbox.wait(handler::EVENT, pid) {
            Ok(()) => {}
            Err(Gone::Ended(status)) if libc::WIFEXITED(status) => {
                let step = libc::WEXITSTATUS(status) as usize;
                return Err(Start::Failed(match setup.get(step.wrapping_sub(1)) {
                    Some(&(what, ..)) => {
                        let result = process.mailbox.get(handler::OPS + (step as u64 - 1) * handler::OP_SIZE + 56) as i64;
                        format!("setting the stub up, {what}: {}", std::io::Error::from_raw_os_error(-result as i32))
                    }
                    None => format!("setting the stub up: it couldn't map its mailbox (exit {step})"),
                }));
            }
            // Ended before its first instruction: the kernel couldn't put it where it goes, as when the stack it made
            // for it is there already.
            Err(Gone::Ended(status)) => return Err(Start::Taken(format!("the stub didn't start (status {status:#x})"))),
            Err(gone) => return Err(Start::Failed(format!("starting the stub: {}", gone.why()))),
        }
        let signo = process.mailbox.get(handler::SIGNO) as i32;
        if signo != libc::SIGILL {
            return Err(Start::Failed(format!("the stub started with signal {signo}, not at its kick")));
        }
        Ok(process)
    }

    /// Makes the stub show `runs` of guest memory: in `fast`, the handler maps them as it returns to the program (so a
    /// fault's page costs no round trip of its own), or at once if there are more than its mailbox holds; in `strict`,
    /// Carbon does, with the stub stopped. The stub must be waiting in its handler.
    pub fn mirror(&mut self, runs: &[MirrorRun]) -> Result<(), String> {
        let mut calls = Vec::with_capacity(runs.len());
        for run in runs {
            let (page, len) = match *run {
                MirrorRun::Map { page, len, .. } | MirrorRun::Protect { page, len, .. } | MirrorRun::Unmap { page, len } => (page, len),
            };
            if page.saturating_add(len) > self.at.base {
                return Err(format!("{page:#x} is the stub's"));
            }
            let (call, exec) = match *run {
                MirrorRun::Map { offset, prot, .. } => {
                    let flags = (libc::MAP_SHARED | libc::MAP_FIXED) as u64;
                    ((libc::SYS_mmap, [page, len, host_prot(prot), flags, GUEST_FD as u64, offset]), prot.exec())
                }
                MirrorRun::Protect { prot, .. } => ((libc::SYS_mprotect, [page, len, host_prot(prot), 0, 0, 0]), prot.exec()),
                MirrorRun::Unmap { .. } => ((libc::SYS_munmap, [page, len, 0, 0, 0, 0]), false),
            };
            calls.push((*run, call.0, call.1));
            let gone: Vec<u64> = self.executable.range(page..page + len).copied().collect();
            for page in gone {
                self.executable.remove(&page);
            }
            if exec {
                self.executable.extend((page..page + len).step_by(4096));
            }
        }
        match self.mode {
            Mode::Fast => {
                self.pending.extend(calls);
                if self.pending.len() > MOST_PENDING {
                    let pending = std::mem::take(&mut self.pending);
                    let results = self.mailbox.run(&pending.iter().map(|&(_, nr, args)| (nr, args)).collect::<Vec<_>>(), self.pid).map_err(|gone| gone.why())?;
                    check(&pending, &results)?;
                }
                Ok(())
            }
            Mode::Strict => {
                let results = self.strict_calls(&calls.iter().map(|&(_, nr, args)| (nr, args)).collect::<Vec<_>>())?;
                check(&calls, &results)
            }
        }
    }

    /// The mapping calls for the handler to make as it returns, for the mailbox; `checked` takes their results.
    pub fn take_pending(&mut self) -> Vec<(i64, [u64; 6])> {
        self.pending.iter().map(|&(_, nr, args)| (nr, args)).collect()
    }

    /// Checks the first `n` results in the mailbox, of the mapping calls the handler made as it returned.
    pub fn checked(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending);
        let results: Vec<i64> = (0..pending.len() as u64).map(|i| self.mailbox.get(handler::OPS + i * handler::OP_SIZE + 56) as i64).collect();
        check(&pending, &results)
    }

    /// `strict`: takes the stub (waiting in its handler) under `ptrace`, makes the calls through the trampoline with
    /// the selector open, and lets it go back to waiting.
    fn strict_calls(&self, calls: &[(i64, [u64; 6])]) -> Result<Vec<i64>, String> {
        sys::seize(self.pid).map_err(|why| format!("strict mode takes the stub under ptrace to map its memory, and {why}"))?;
        sys::interrupt(self.pid)?;
        loop {
            let status = sys::wait(self.pid)?;
            if !libc::WIFSTOPPED(status) {
                return Err(format!("the stub ended (status {status:#x})"));
            }
            if status >> 16 == sys::PTRACE_EVENT_STOP {
                break;
            }
            // A signal on its way (a preempt): it goes on to the handler, which waits with it blocked.
            sys::cont(self.pid, libc::WSTOPSIG(status))?;
        }
        let saved = sys::getregs(self.pid)?;
        sys::poke(self.pid, self.at.selector(), u64::from(sys::SYSCALL_DISPATCH_FILTER_ALLOW))?;
        let results = self.trampoline_calls(&saved, calls);
        sys::poke(self.pid, self.at.selector(), u64::from(sys::SYSCALL_DISPATCH_FILTER_BLOCK))?;
        sys::setregs(self.pid, &saved)?;
        sys::detach(self.pid)?;
        results
    }

    /// Makes system calls in the stub, stopped under `ptrace`, one after another through the trampoline.
    fn trampoline_calls(&self, saved: &sys::UserRegs, calls: &[(i64, [u64; 6])]) -> Result<Vec<i64>, String> {
        let mut results = Vec::with_capacity(calls.len());
        for &(nr, args) in calls {
            let mut regs = *saved;
            regs.rip = self.at.trampoline();
            regs.rax = nr as u64;
            // The handler's futex wait, which the stub was stopped in, isn't carried on with these registers; the handler
            // looks again once the saved ones are back.
            regs.orig_rax = u64::MAX;
            (regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9) = (args[0], args[1], args[2], args[3], args[4], args[5]);
            sys::setregs(self.pid, &regs)?;
            // Into the call, then out of it: two syscall stops, so the trampoline's `int3` never runs. (A SIGTRAP raised
            // while the handler has it blocked would set it back to its default, which ends the stub at the program's
            // next breakpoint.) A preempt that comes meanwhile is swallowed: whoever sent it has also marked the CPU,
            // so it's seen before the program next runs.
            for _ in 0..2 {
                loop {
                    sys::syscall_step(self.pid)?;
                    let status = sys::wait(self.pid)?;
                    if stopped_with(status, libc::SIGTRAP | 0x80) {
                        break;
                    }
                    if libc::WIFSTOPPED(status) && ![libc::SIGSYS, libc::SIGSEGV, libc::SIGTRAP].contains(&libc::WSTOPSIG(status)) {
                        continue;
                    }
                    return Err(format!("a call in the stub didn't come back (status {status:#x})"));
                }
            }
            results.push(sys::getregs(self.pid)?.rax as i64);
        }
        Ok(results)
    }

    pub fn is_executable(&self, page: u64) -> bool {
        self.executable.contains(&page)
    }
}

/// Whether each mapping call did what it was for.
fn check(calls: &[(MirrorRun, i64, [u64; 6])], results: &[i64]) -> Result<(), String> {
    for (&(run, ..), &r) in calls.iter().zip(results) {
        let what = match run {
            MirrorRun::Map { page, .. } if r >= 0 && r as u64 != page => return Err(format!("guest pages went to {r:#x}, not {page:#x}")),
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

/// The calls that set a stub up, as it makes them from its handler's page (`handler.rs`), once it's mapped its mailbox;
/// the structures they take are written to its signal stack first, which is unused until then.
///
/// 1. Everything the kernel gave it but the stub itself (its stack, the vDSO and vvar) goes, so every other address is
///    the program's, with no look at `/proc`. (The vsyscall page is above user space, and can't go.)
/// 2. The handler, on the shared signal stack, for every signal Carbon has to see, with every other signal blocked
///    while it runs.
/// 3. Syscall User Dispatch, with the handler's page exempt: with the selector at "block" (as the ELF has it), every
///    other system call raises `SIGSYS`. Carbon opens the selector only in `strict`, while it has the stub stopped.
///    Where there's none, seccomp's trap does the same.
/// 4. Its pages narrowed: code read and execute, data read-only, so a program can't open the selector by writing it.
/// 5. Its signals, all blocked since it was started, unblocked.
/// 6. The seccomp filter, last.
fn setup(mailbox: &mut Mailbox, at: At, catch: Catch, mode: Mode) -> Setup {
    let scratch = (handler::SIGNAL_STACK - handler::SHARED) as usize;
    let at_scratch = |offset: usize| at.signal_stack() + offset as u64;
    // stack_t
    let stack = mailbox.bytes_mut(scratch, 24);
    stack[0..8].copy_from_slice(&at.signal_stack().to_le_bytes());
    stack[8..16].fill(0);
    stack[16..24].copy_from_slice(&handler::SIGNAL_STACK_SIZE.to_le_bytes());
    // struct kernel_sigaction { handler, flags, restorer, mask }
    let action = mailbox.bytes_mut(scratch + 0x40, 32);
    action[0..8].copy_from_slice(&at.handler().to_le_bytes());
    let flags = (libc::SA_SIGINFO | libc::SA_ONSTACK) as u64 | sys::SA_RESTORER;
    action[8..16].copy_from_slice(&flags.to_le_bytes());
    action[16..24].copy_from_slice(&at.restorer().to_le_bytes());
    action[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    // An empty signal set.
    mailbox.bytes_mut(scratch + 0x60, 8).fill(0);
    // struct sock_fprog { unsigned short len; struct sock_filter *filter; }, then the filter.
    let mapper = if mode == Mode::Strict { Mapper::Trampoline } else { Mapper::Handler };
    let filter = seccomp::filter(at.base, GUEST_FD, mapper);
    let program = mailbox.bytes_mut(scratch + 0x80, 16);
    program[0..8].copy_from_slice(&((filter.len() / 8) as u64).to_le_bytes());
    program[8..16].copy_from_slice(&at_scratch(0x100).to_le_bytes());
    mailbox.bytes_mut(scratch + 0x100, filter.len()).copy_from_slice(&filter);

    let (base, end) = (at.base, at.base + elf::SIZE);
    let mut calls: Setup = vec![("unmapping what the kernel put below it", libc::SYS_munmap, [0, base, 0, 0, 0, 0])];
    if end < elf::TOP {
        calls.push(("unmapping what the kernel put above it", libc::SYS_munmap, [end, elf::TOP - end, 0, 0, 0, 0]));
    }
    calls.push(("closing its mailbox's descriptor", libc::SYS_close, [MAILBOX_FD as u64, 0, 0, 0, 0, 0]));
    calls.push(("giving it its signal stack", libc::SYS_sigaltstack, [at_scratch(0), 0, 0, 0, 0, 0]));
    for signal in SIGNALS {
        calls.push(("installing its handler", libc::SYS_rt_sigaction, [signal as u64, at_scratch(0x40), 0, 8, 0, 0]));
    }
    if catch == Catch::Dispatch {
        calls.push(("turning on Syscall User Dispatch", libc::SYS_prctl, [sys::PR_SET_SYSCALL_USER_DISPATCH, sys::PR_SYS_DISPATCH_ON, base, 0x1000, at.selector(), 0]));
    }
    let rx = (libc::PROT_READ | libc::PROT_EXEC) as u64;
    calls.push(("narrowing its handler's page", libc::SYS_mprotect, [base, 0x1000, rx, 0, 0, 0]));
    calls.push(("narrowing its trampoline's page", libc::SYS_mprotect, [at.trampoline(), 0x1000, rx, 0, 0, 0]));
    calls.push(("narrowing its data page", libc::SYS_mprotect, [at.data(), 0x1000, libc::PROT_READ as u64, 0, 0, 0]));
    calls.push(("unblocking its signals", libc::SYS_rt_sigprocmask, [libc::SIG_SETMASK as u64, at_scratch(0x60), 0, 8, 0, 0]));
    calls.push(("installing its seccomp filter", libc::SYS_seccomp, [sys::SECCOMP_SET_MODE_FILTER, 0, at_scratch(0x80), 0, 0, 0]));
    calls
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the stub, which PR_SET_PDEATHSIG also guards; collected so it doesn't linger.
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
    mailbox_fd: i32,
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
const STEP_EXEC: u32 = 4;
const STEP_MAILBOX_FD: u32 = 5;

/// Where the stub's ELF is run from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Memfd,
    /// A file in the temporary directory, unlinked once it's open: for where memfds can't be executed
    /// (`vm.memfd_noexec = 2`).
    TempFile,
}

/// Starts the stub's process, which sets itself up from there: its pid.
fn launch(guest_fd: i32, mailbox_fd: i32, at: At) -> Result<i32, Start> {
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
        match clone_exec(guest_fd, mailbox_fd, stub_fd) {
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
/// process is copied, however big it is). Every signal is blocked meanwhile, so none runs a handler of this process's
/// in the child; the stub unblocks its own once its handler's installed (`setup`).
fn clone_exec(guest_fd: i32, mailbox_fd: i32, stub_fd: i32) -> Result<i32, Launch> {
    let mut pipe = [0i32; 2];
    // SAFETY: an out-parameter.
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        // SAFETY: the stub's file, which nothing else has.
        unsafe { libc::close(stub_fd) };
        return Err(Launch::Failed(format!("making a pipe: {}", std::io::Error::last_os_error())));
    }
    // The child puts guest memory at GUEST_FD and the mailbox at MAILBOX_FD over whatever's there, so neither the
    // stub's file, the pipe nor the mailbox may be there: in a process with more than a hundred descriptors open (many
    // machines at once), any could be. (Guest memory may be: the child sees to that.)
    let moved = |fd: i32| {
        let to = off_fixed_fds(fd);
        if to != fd {
            // SAFETY: the descriptor just copied, which this owns.
            unsafe { libc::close(fd) };
        }
        to
    };
    let (stub_fd, report) = (moved(stub_fd), moved(pipe[1]));
    pipe[1] = report;
    // The mailbox's own descriptor is the Mailbox's to close; a copy is closed below.
    let mailbox_moved = off_fixed_fds(mailbox_fd);
    // SAFETY: an out-parameter.
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    let empty = c"";
    let child = Child {
        guest_fd,
        mailbox_fd: mailbox_moved,
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
        if mailbox_moved != mailbox_fd {
            libc::close(mailbox_moved);
        }
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
            STEP_EXEC if [libc::EACCES, libc::EPERM].contains(&code) => Launch::NoExec(format!("executing the stub was refused ({error}): memfds and the temporary directory are both non-executable here")),
            // A kernel that says so, rather than ending the new process: it couldn't put the stub where it goes.
            STEP_EXEC if [libc::ENOMEM, libc::EEXIST, libc::EINVAL].contains(&code) => Launch::Taken(format!("executing the stub: {error}")),
            STEP_EXEC => Launch::Failed(format!("executing the stub: {error}")),
            STEP_NO_NEW_PRIVS => Launch::Failed(format!("PR_SET_NO_NEW_PRIVS: {error}")),
            STEP_GUEST_FD => Launch::Failed(format!("giving the stub guest memory: {error}")),
            STEP_MAILBOX_FD => Launch::Failed(format!("giving the stub its mailbox: {error}")),
            _ => Launch::Failed(format!("starting the stub, step {step}: {error}")),
        });
    }
    Ok(pid)
}

/// `fd`, or, if it's at `GUEST_FD` or `MAILBOX_FD`, a copy of it above them, close-on-exec. The caller's descriptor
/// stays where it was; it closes the copy if it differs.
fn off_fixed_fds(fd: i32) -> i32 {
    if fd != GUEST_FD && fd != MAILBOX_FD {
        return fd;
    }
    // SAFETY: a descriptor this owns, duplicated.
    let moved = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, MAILBOX_FD + 1) };
    if moved < 0 { fd } else { moved }
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
        // The mailbox is never at either (off_fixed_fds), so guest memory's dup2 above can't have closed it.
        if libc::dup2(child.mailbox_fd, MAILBOX_FD) < 0 {
            fail(child.report, STEP_MAILBOX_FD);
        }
        // Every other descriptor closes at exec: the stub has guest memory, and its mailbox until it's mapped.
        let below = libc::syscall(libc::SYS_close_range, 0, GUEST_FD - 1, sys::CLOSE_RANGE_CLOEXEC);
        let above = libc::syscall(libc::SYS_close_range, MAILBOX_FD + 1, u32::MAX, sys::CLOSE_RANGE_CLOEXEC);
        if below != 0 || above != 0 {
            // A kernel before 5.11's CLOSE_RANGE_CLOEXEC, or gVisor without it: each one, then.
            for fd in (0..child.max_fd).filter(|&fd| fd != GUEST_FD && fd != MAILBOX_FD) {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        // Seccomp without CAP_SYS_ADMIN needs this, and the stub has no use for privileges anyway.
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            fail(child.report, STEP_NO_NEW_PRIVS);
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
