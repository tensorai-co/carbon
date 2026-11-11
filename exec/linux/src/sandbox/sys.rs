//! The Linux calls the sandbox makes that `libc` doesn't wrap, the constants it lacks, and small `ptrace` helpers.

use std::io::Error;

/// `prctl`'s Syscall User Dispatch: every system call from outside an allowed range raises `SIGSYS`, as long as the
/// selector byte says to block.
pub const PR_SET_SYSCALL_USER_DISPATCH: u64 = 59;
pub const PR_SYS_DISPATCH_ON: u64 = 1;
pub const SYSCALL_DISPATCH_FILTER_BLOCK: u8 = 1;

pub const SECCOMP_SET_MODE_FILTER: u64 = 1;
pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
pub const AUDIT_ARCH_I386: u32 = 0x4000_0003;

pub const SEGV_MAPERR: i32 = 1;
pub const SEGV_ACCERR: i32 = 2;

/// `close_range`'s flag: mark them close-on-exec, rather than closing them now.
pub const CLOSE_RANGE_CLOEXEC: u32 = 4;

pub type UserRegs = libc::user_regs_struct;
pub type UserFpRegs = libc::user_fpregs_struct;

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

pub fn getfpregs(pid: i32) -> Result<UserFpRegs, String> {
    // SAFETY: plain data, filled in below.
    let mut regs: UserFpRegs = unsafe { std::mem::zeroed() };
    result(ptrace(libc::PTRACE_GETFPREGS, pid, 0, &raw mut regs as u64), "PTRACE_GETFPREGS")?;
    Ok(regs)
}

pub fn setfpregs(pid: i32, regs: &UserFpRegs) -> Result<(), String> {
    result(ptrace(libc::PTRACE_SETFPREGS, pid, 0, regs as *const UserFpRegs as u64), "PTRACE_SETFPREGS").map(|_| ())
}

/// The `siginfo` of the signal the stub stopped on, as raw bytes: the fields used are at fixed offsets.
pub fn getsiginfo(pid: i32) -> Result<[u8; 128], String> {
    let mut info = [0u8; 128];
    result(ptrace(libc::PTRACE_GETSIGINFO, pid, 0, info.as_mut_ptr() as u64), "PTRACE_GETSIGINFO")?;
    Ok(info)
}

pub fn setoptions(pid: i32, options: i32) -> Result<(), String> {
    result(ptrace(libc::PTRACE_SETOPTIONS, pid, 0, options as u64), "PTRACE_SETOPTIONS").map(|_| ())
}

/// Carries on, delivering `signal` (0 for none: a signal the stub stopped on is swallowed).
pub fn cont(pid: i32, signal: i32) -> Result<(), String> {
    result(ptrace(libc::PTRACE_CONT, pid, 0, signal as u64), "PTRACE_CONT").map(|_| ())
}

/// Writes 8 bytes of the stub's memory, whatever its protection.
pub fn poke(pid: i32, addr: u64, value: u64) -> Result<(), String> {
    result(ptrace(libc::PTRACE_POKEDATA, pid, addr, value), "PTRACE_POKEDATA").map(|_| ())
}

/// Reads 8 bytes of the stub's memory, or None.
pub fn peek(pid: i32, addr: u64) -> Option<u64> {
    // SAFETY: errno, which PEEKDATA's result can't be told apart from without.
    unsafe { *libc::__errno_location() = 0 };
    let value = ptrace(libc::PTRACE_PEEKDATA, pid, addr, 0);
    // SAFETY: as above.
    if value == -1 && unsafe { *libc::__errno_location() } != 0 { None } else { Some(value as u64) }
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

/// Sends a signal to the stub's one thread.
pub fn tgkill(pid: i32, signal: i32) {
    // SAFETY: a process this made; a signal it swallows.
    unsafe { libc::syscall(libc::SYS_tgkill, pid, pid, signal) };
}
