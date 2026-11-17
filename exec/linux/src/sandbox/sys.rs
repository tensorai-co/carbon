//! The Linux calls the sandbox makes that `libc` doesn't wrap, the constants it lacks, and the `ptrace` that `strict`
//! mode uses to change a stub's mappings.

use std::io::Error;

/// `prctl`'s Syscall User Dispatch: every system call from outside an allowed range raises `SIGSYS`, as long as the
/// selector byte says to block.
pub const PR_SET_SYSCALL_USER_DISPATCH: u64 = 59;
pub const PR_SYS_DISPATCH_ON: u64 = 1;
pub const SYSCALL_DISPATCH_FILTER_ALLOW: u8 = 0;
pub const SYSCALL_DISPATCH_FILTER_BLOCK: u8 = 1;

pub const SECCOMP_SET_MODE_FILTER: u64 = 1;
/// `AT_HWCAP2`'s bit for `rdfsbase` and the rest being allowed.
pub const HWCAP2_FSGSBASE: u64 = 2;
pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
pub const AUDIT_ARCH_I386: u32 = 0x4000_0003;

pub const SEGV_ACCERR: i32 = 2;
/// A general-protection fault's `si_code`.
pub const SI_KERNEL: i32 = 0x80;

/// `close_range`'s flag: mark them close-on-exec, rather than closing them now.
pub const CLOSE_RANGE_CLOEXEC: u32 = 4;
/// memfd_create's flag for a memfd that may be executed, where `vm.memfd_noexec` would make it not.
pub const MFD_EXEC: u32 = 0x10;

/// `sigaction`'s flag for a handler that returns through its own restorer (the kernel's `rt_sigreturn` caller).
pub const SA_RESTORER: u64 = 0x0400_0000;

/// `PTRACE_INTERRUPT`'s stop, as `waitpid`'s status has it above the signal.
pub const PTRACE_EVENT_STOP: i32 = 128;
const PTRACE_SEIZE: libc::c_uint = 0x4206;
const PTRACE_INTERRUPT: libc::c_uint = 0x4207;

pub type UserRegs = libc::user_regs_struct;

fn result(value: libc::c_long, what: &str) -> Result<libc::c_long, String> {
    if value < 0 { Err(format!("{what}: {}", Error::last_os_error())) } else { Ok(value) }
}

/// A `ptrace` request whose data is a pointer or a number.
fn ptrace(request: libc::c_uint, pid: i32, addr: u64, data: u64) -> libc::c_long {
    // SAFETY: each caller passes what its request takes.
    unsafe { libc::ptrace(request, pid, addr as *mut libc::c_void, data as *mut libc::c_void) }
}

pub fn getregs(pid: i32) -> Result<UserRegs, String> {
    // SAFETY: plain data, filled in below.
    let mut regs: UserRegs = unsafe { std::mem::zeroed() };
    result(ptrace(libc::PTRACE_GETREGS, pid, 0, &raw mut regs as u64), "PTRACE_GETREGS")?;
    Ok(regs)
}

pub fn setregs(pid: i32, regs: &UserRegs) -> Result<(), String> {
    result(ptrace(libc::PTRACE_SETREGS, pid, 0, regs as *const UserRegs as u64), "PTRACE_SETREGS").map(|_| ())
}

/// Carries on, delivering `signal` (0 for none: a signal the stub stopped on is swallowed).
pub fn cont(pid: i32, signal: i32) -> Result<(), String> {
    result(ptrace(libc::PTRACE_CONT, pid, 0, signal as u64), "PTRACE_CONT").map(|_| ())
}

/// Carries on to the next system call's entry or exit, swallowing any signal it stopped on.
pub fn syscall_step(pid: i32) -> Result<(), String> {
    result(ptrace(libc::PTRACE_SYSCALL, pid, 0, 0), "PTRACE_SYSCALL").map(|_| ())
}

/// Lets the stub go: it runs on its own until it ends.
pub fn detach(pid: i32) -> Result<(), String> {
    result(ptrace(libc::PTRACE_DETACH, pid, 0, 0), "PTRACE_DETACH").map(|_| ())
}

/// Takes a running stub (one of this process's children) under `ptrace`, without stopping it.
pub fn seize(pid: i32) -> Result<(), String> {
    result(ptrace(PTRACE_SEIZE, pid, 0, (libc::PTRACE_O_EXITKILL | libc::PTRACE_O_TRACESYSGOOD) as u64), "PTRACE_SEIZE").map(|_| ())
}

/// Stops a seized stub, wherever it is, with `PTRACE_EVENT_STOP`.
pub fn interrupt(pid: i32) -> Result<(), String> {
    result(ptrace(PTRACE_INTERRUPT, pid, 0, 0), "PTRACE_INTERRUPT").map(|_| ())
}

/// Writes 8 bytes of the stub's memory, whatever its protection.
pub fn poke(pid: i32, addr: u64, value: u64) -> Result<(), String> {
    result(ptrace(libc::PTRACE_POKEDATA, pid, addr, value), "PTRACE_POKEDATA").map(|_| ())
}

/// Waits for the stub to stop or end, through any interruption.
pub fn wait(pid: i32) -> Result<i32, String> {
    loop {
        let mut status = 0;
        // SAFETY: an out-parameter.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r == pid {
            return Ok(status);
        }
        let error = Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(format!("waiting for the stub: {error}"));
        }
    }
}

/// Sends a signal to a stub's one thread.
pub fn tgkill(pid: i32, signal: i32) {
    // SAFETY: a process this made; a signal its handler takes.
    unsafe { libc::syscall(libc::SYS_tgkill, pid, pid, signal) };
}
