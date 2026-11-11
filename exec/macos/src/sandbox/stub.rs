//! The stub: Carbon's own binary, run again with `CARBON_EXEC_STUB=1`, which tells Carbon it's ready and then waits.
//! Carbon takes its task with `task_for_pid`, which the stub allows (it's signed `get-task-allow`) and Carbon may ask
//! (`com.apple.security.cs.debugger`): both are this one binary, signed with `carbon.entitlements`. With the task,
//! Carbon maps memory into the stub, makes a thread in it to run the program, and catches that thread's exceptions.
//!
//! (A task port the stub sends itself can't be used: its control port is guarded, and using it kills the receiver.)
//!
//! A program using the macOS sandbox must call `run_if_stub()` first thing in `main`.

use std::ffi::CString;
use std::ops::Range;

use super::mach::*;

const STUB_ENV: &str = "CARBON_EXEC_STUB";
/// The descriptor the stub says it's ready on.
const READY_FD: i32 = 3;

/// If this process is a stub, says it's ready and waits to be ended. Otherwise returns at once.
pub fn run_if_stub() {
    if std::env::var_os(STUB_ENV).is_none() {
        return;
    }
    // SAFETY: the pipe Carbon gave the stub, written once and closed.
    unsafe {
        libc::write(READY_FD, b"r".as_ptr().cast(), 1);
        libc::close(READY_FD);
    }
    // Carbon ends the stub; if Carbon itself goes first, the stub is reparented and follows.
    let parent = std::os::unix::process::parent_id();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if std::os::unix::process::parent_id() != parent {
            std::process::exit(0);
        }
    }
}

/// A stub, as Carbon holds it.
pub struct Stub {
    pub pid: i32,
    pub task: mach_port_t,
    /// What the stub already has (its own binary, dyld, libSystem, its stack), which programs must stay out of.
    pub kept: Vec<Range<u64>>,
}

impl Stub {
    pub fn spawn() -> Result<Stub, String> {
        let exe = std::env::current_exe().map_err(|e| format!("finding Carbon's binary: {e}"))?;
        let path = CString::new(exe.as_os_str().as_encoded_bytes()).map_err(|_| "Carbon's path has a NUL in it")?;
        let env = CString::new(format!("{STUB_ENV}=1")).unwrap_or_default();
        let argv = [path.as_ptr(), std::ptr::null()];
        let envp = [env.as_ptr(), std::ptr::null()];
        let mut ready = [0i32; 2];
        // SAFETY: a new pipe, into an array of two.
        if unsafe { libc::pipe(ready.as_mut_ptr()) } != 0 {
            return Err(format!("making the stub's pipe: {}", std::io::Error::last_os_error()));
        }
        let mut pid = 0;
        // SAFETY: spawn attributes and file actions, set up and destroyed here, around one posix_spawn of Carbon's own
        // binary: no descriptors of Carbon's but the pipe's write end, as READY_FD.
        let spawned = unsafe {
            let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
            let mut actions: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
            libc::posix_spawnattr_init(&mut attr);
            libc::posix_spawnattr_setflags(&mut attr, 0x4000 as libc::c_short);
            libc::posix_spawn_file_actions_init(&mut actions);
            libc::posix_spawn_file_actions_adddup2(&mut actions, ready[1], READY_FD);
            let spawned = libc::posix_spawn(&mut pid, path.as_ptr(), &actions, &attr, argv.as_ptr().cast(), envp.as_ptr().cast());
            libc::posix_spawn_file_actions_destroy(&mut actions);
            libc::posix_spawnattr_destroy(&mut attr);
            libc::close(ready[1]);
            spawned
        };
        if spawned != 0 {
            // SAFETY: the pipe's read end.
            unsafe { libc::close(ready[0]) };
            return Err(format!("starting the stub: {}", std::io::Error::from_raw_os_error(spawned)));
        }
        let mut poll = libc::pollfd { fd: ready[0], events: libc::POLLIN, revents: 0 };
        let mut byte = 0u8;
        // SAFETY: the pipe's read end, waited on for 5 seconds, read once, and closed.
        let is_ready = unsafe { libc::poll(&mut poll, 1, 5000) == 1 && libc::read(ready[0], (&raw mut byte).cast(), 1) == 1 };
        // SAFETY: as above.
        unsafe { libc::close(ready[0]) };
        let mut task = MACH_PORT_NULL;
        // SAFETY: an out-parameter for the stub's task.
        let got = if is_ready { unsafe { task_for_pid(task_self(), pid, &mut task) } } else { -1 };
        if got != KERN_SUCCESS {
            // SAFETY: the stub, which can't be used.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, &mut 0, 0);
            }
            return Err(if is_ready {
                format!("task_for_pid on the stub (Mach error {got:#x}): is Carbon signed with carbon.entitlements?")
            } else {
                "the stub didn't start: does main call exec::run_if_stub()?".to_string()
            });
        }
        // XNU keeps the bottom 4 GiB of every 64-bit process empty, and won't map anything there: programs live above.
        let mut kept = regions(task);
        kept.insert(0, 0..1 << 32);
        Ok(Stub { pid, task, kept })
    }

    /// Writes `data` into the stub at `addr`.
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<(), String> {
        // SAFETY: the stub's memory, which Carbon has allocated there; the data outlives the call.
        check("writing to the stub", unsafe { mach_vm_write(self.task, addr, data.as_ptr() as usize, data.len() as u32) })
    }

    /// Reads the stub's memory at `addr` into `buffer`.
    pub fn read(&self, addr: u64, buffer: &mut [u8]) -> Result<(), String> {
        let mut got = 0;
        // SAFETY: into a buffer of the size given.
        check("reading the stub", unsafe { mach_vm_read_overwrite(self.task, addr, buffer.len() as u64, buffer.as_mut_ptr() as u64, &mut got) })
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        // SAFETY: the stub, ended and collected, and Carbon's right to its task.
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
            mach_port_deallocate(task_self(), self.task);
            let mut status = 0;
            libc::waitpid(self.pid, &mut status, 0);
        }
    }
}

/// Every region of a task's address space.
fn regions(task: mach_port_t) -> Vec<Range<u64>> {
    let mut kept: Vec<Range<u64>> = Vec::new();
    let mut addr = 0u64;
    loop {
        let (mut size, mut info, mut count, mut object) = (0u64, [0u32; 9], 9u32, MACH_PORT_NULL);
        // SAFETY: out-parameters of the sizes given.
        let found = unsafe { mach_vm_region(task, &mut addr, &mut size, VM_REGION_BASIC_INFO_64, info.as_mut_ptr(), &mut count, &mut object) };
        if found != KERN_SUCCESS || size == 0 {
            break;
        }
        match kept.last_mut() {
            Some(last) if last.end >= addr => last.end = last.end.max(addr + size),
            _ => kept.push(addr..addr + size),
        }
        addr += size;
    }
    kept
}
