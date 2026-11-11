//! The stub process: spawning it, making system calls on its behalf, and ending it.
//!
//! `ptrace` answers only the thread that traces, which is the thread that forked the stub. So the thread that opens
//! the sandbox is its one CPU's thread, and everything here is done from it. Ending the stub works from anywhere.

use super::elf::{DATA, FILTER, FILTER_PROGRAM, SELECTOR, STUB, TRAMPOLINE};
use super::{seccomp, sys};

/// Where guest memory's memfd is in the stub.
pub const GUEST_FD: i32 = 100;
/// The signal that preempts the stub. A program can't catch it, having no system calls; Carbon swallows it.
pub const PREEMPT_SIGNAL: i32 = libc::SIGURG;

pub struct Process {
    pub pid: i32,
    /// The program's pages that are executable, as `mirror` mapped them: where a fault at the instruction itself can
    /// only be a write, the fetch having worked.
    executable: std::sync::Mutex<std::collections::HashSet<u64>>,
}

fn stopped_with(status: i32, signal: i32) -> bool {
    libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) == signal
}

impl Process {
    /// Notes whether a page of the program's is executable now.
    pub fn set_executable(&self, page: u64, executable: bool) {
        let mut pages = self.executable.lock().unwrap_or_else(|p| p.into_inner());
        if executable {
            pages.insert(page);
        } else {
            pages.remove(&page);
        }
    }

    pub fn is_executable(&self, page: u64) -> bool {
        self.executable.lock().unwrap_or_else(|p| p.into_inner()).contains(&page)
    }

    /// Starts the stub with guest memory at `GUEST_FD`, stopped, its loader's mappings gone, its system calls going to
    /// Carbon, and its own locked down to what the filter allows.
    pub fn spawn(guest_fd: i32) -> Result<Process, String> {
        let elf = super::elf::build();
        // SAFETY: a name and flags.
        let stub_fd = unsafe { libc::memfd_create(c"carbon-stub".as_ptr(), libc::MFD_CLOEXEC) };
        if stub_fd < 0 {
            return Err(format!("making the stub: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: the memfd just made, and the ELF's bytes.
        let written = unsafe { libc::write(stub_fd, elf.as_ptr().cast(), elf.len()) };
        if written != elf.len() as isize {
            // SAFETY: as above.
            unsafe { libc::close(stub_fd) };
            return Err("writing the stub".to_string());
        }

        let empty = c"";
        let argv = [empty.as_ptr(), std::ptr::null()];
        let envp = [std::ptr::null::<libc::c_char>()];
        // SAFETY: fork. The child makes raw system calls only, then execs or exits: nothing that could take a lock.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            // SAFETY: as above.
            unsafe { libc::close(stub_fd) };
            return Err(format!("fork: {}", std::io::Error::last_os_error()));
        }
        if pid == 0 {
            // SAFETY: raw calls in a just-forked child, which execs or exits.
            unsafe {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
                libc::dup2(guest_fd, GUEST_FD);
                // Every other descriptor closes at exec: the stub has guest memory and nothing else.
                libc::syscall(libc::SYS_close_range, 0, GUEST_FD - 1, sys::CLOSE_RANGE_CLOEXEC);
                libc::syscall(libc::SYS_close_range, GUEST_FD + 1, u32::MAX, sys::CLOSE_RANGE_CLOEXEC);
                // Seccomp without CAP_SYS_ADMIN needs this, and the stub has no use for privileges anyway.
                libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
                libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0);
                libc::syscall(libc::SYS_execveat, stub_fd, empty.as_ptr(), argv.as_ptr(), envp.as_ptr(), libc::AT_EMPTY_PATH);
                libc::_exit(127);
            }
        }
        let status = sys::wait(pid);
        // SAFETY: the parent's copy, which the child has exec'd from by the time it stops.
        unsafe { libc::close(stub_fd) };
        let process = Process { pid, executable: Default::default() };
        match status {
            Ok(status) if stopped_with(status, libc::SIGTRAP) => {}
            Ok(status) => return Err(format!("the stub didn't start (status {status:#x})")),
            Err(why) => return Err(why),
        }
        sys::setoptions(pid, libc::PTRACE_O_EXITKILL)?;
        process.clear_loader_mappings()?;
        process.set_up_dispatch()?;
        process.lock_down()?;
        Ok(process)
    }

    /// Runs a system call in the stub, through the trampoline, and gives its result. The stub must be stopped, and is
    /// left as it was.
    pub fn call(&self, nr: i64, args: [u64; 6]) -> Result<i64, String> {
        let saved = sys::getregs(self.pid)?;
        let mut regs = saved;
        regs.rip = TRAMPOLINE;
        regs.rax = nr as u64;
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
            if libc::WIFSTOPPED(status) && libc::WSTOPSIG(status) != libc::SIGSYS && libc::WSTOPSIG(status) != libc::SIGSEGV {
                continue;
            }
            return Err(format!("a call in the stub didn't come back (status {status:#x})"));
        }
        let after = sys::getregs(self.pid)?;
        sys::setregs(self.pid, &saved)?;
        Ok(after.rax as i64)
    }

    fn checked(&self, what: &str, nr: i64, args: [u64; 6]) -> Result<i64, String> {
        let r = self.call(nr, args)?;
        if (-4095..0).contains(&r) { Err(format!("{what}: errno {}", -r)) } else { Ok(r) }
    }

    /// Maps a page of guest memory at `page`.
    pub fn map(&self, page: u64, offset: u64, prot: i32) -> Result<(), String> {
        let flags = (libc::MAP_SHARED | libc::MAP_FIXED) as u64;
        let at = self.checked("mapping a guest page", libc::SYS_mmap, [page, 4096, prot as u64, flags, GUEST_FD as u64, offset])?;
        if at as u64 != page {
            return Err(format!("a guest page went to {at:#x}, not {page:#x}"));
        }
        Ok(())
    }

    pub fn unmap(&self, addr: u64, len: u64) -> Result<(), String> {
        self.checked("unmapping guest pages", libc::SYS_munmap, [addr, len, 0, 0, 0, 0]).map(|_| ())
    }

    /// Takes away everything the kernel gave the stub but its own two pages (the stack, vDSO and vvar), so every other
    /// address is the program's.
    fn clear_loader_mappings(&self) -> Result<(), String> {
        let maps = std::fs::read_to_string(format!("/proc/{}/maps", self.pid)).map_err(|e| format!("reading the stub's maps: {e}"))?;
        for line in maps.lines() {
            let Some((range, _)) = line.split_once(' ') else { continue };
            let Some((start, end)) = range.split_once('-') else { continue };
            let (Ok(start), Ok(end)) = (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16)) else { continue };
            // The vsyscall page can't go, and is in the kernel's half anyway.
            if (STUB..STUB + 0x2000).contains(&start) || start >= 0xffff_ffff_ff60_0000 {
                continue;
            }
            self.unmap(start, end - start)?;
        }
        Ok(())
    }

    /// Syscall User Dispatch: with the selector at "block", every system call from outside the stub's code page raises
    /// `SIGSYS`, which is how a program's calls reach Carbon.
    fn set_up_dispatch(&self) -> Result<(), String> {
        sys::poke(self.pid, SELECTOR, u64::from(sys::SYSCALL_DISPATCH_FILTER_BLOCK))?;
        self.checked("turning on Syscall User Dispatch (it needs Linux 5.11)", libc::SYS_prctl, [
            sys::PR_SET_SYSCALL_USER_DISPATCH,
            sys::PR_SYS_DISPATCH_ON,
            STUB,
            0x1000,
            SELECTOR,
            0,
        ])
        .map(|_| ())
    }

    /// The seccomp filter, then the stub's own pages narrowed: code read and execute, data read-only, so a program
    /// can't turn dispatch off by writing the selector.
    fn lock_down(&self) -> Result<(), String> {
        let filter = seccomp::filter();
        for (i, chunk) in filter.chunks(8).enumerate() {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            sys::poke(self.pid, FILTER + i as u64 * 8, u64::from_le_bytes(word))?;
        }
        // struct sock_fprog { unsigned short len; struct sock_filter *filter; }
        sys::poke(self.pid, FILTER_PROGRAM, u64::from(seccomp::len()))?;
        sys::poke(self.pid, FILTER_PROGRAM + 8, FILTER)?;
        self.checked("installing the stub's seccomp filter", libc::SYS_seccomp, [sys::SECCOMP_SET_MODE_FILTER, 0, FILTER_PROGRAM, 0, 0, 0])?;
        let rx = (libc::PROT_READ | libc::PROT_EXEC) as u64;
        self.checked("narrowing the stub's code", libc::SYS_mprotect, [STUB, 0x1000, rx, 0, 0, 0])?;
        self.checked("narrowing the stub's data", libc::SYS_mprotect, [DATA, 0x1000, libc::PROT_READ as u64, 0, 0, 0])?;
        Ok(())
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
