//! Calls into atlas, by its own numbers: `svc #0` with the number in `x8` on ARM64; on x86-64, the number in `eax` and
//! the first argument in `r10`, as Windows' own system calls have them, through atlas's trampoline (`call::TRAP_AT`),
//! which is `syscall` or the backend's gate.

use atlas_abi::call;

#[inline(always)]
pub fn call4(nr: u64, a: u64, b: u64, c: u64, d: u64) -> u64 {
    let result: u64;
    // SAFETY: a trap into atlas, which changes only the result register (and, on x86-64, what `syscall` does).
    unsafe {
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("svc #0", in("x8") nr, inlateout("x0") a => result, in("x1") b, in("x2") c, in("x3") d, options(nostack));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("call r11", inlateout("rax") nr => result, in("r10") a, in("rdx") b, in("r8") c, in("r9") d,
            inlateout("r11") call::TRAP_AT => _, lateout("rcx") _);
    }
    result
}

pub fn terminate(status: u32) -> ! {
    call4(call::TERMINATE_PROCESS, u64::MAX, u64::from(status), 0, 0);
    // atlas never comes back from ending the process.
    loop {
        core::hint::spin_loop();
    }
}

/// Commits `size` bytes of new memory, read-write: its address, or null.
pub fn allocate(size: usize) -> *mut u8 {
    const COMMIT_RESERVE: u64 = 0x3000;
    const READ_WRITE: u64 = 4;
    call4(call::ALLOCATE_VIRTUAL_MEMORY, 0, size as u64, COMMIT_RESERVE, READ_WRITE) as *mut u8
}

/// Changes pages' protection (a `PAGE_*`), giving the old one, or None.
pub fn protect(addr: u64, size: u64, protection: u32) -> Option<u32> {
    let mut old = 0u32;
    let status = call4(call::PROTECT_VIRTUAL_MEMORY, addr, size, u64::from(protection), (&raw mut old) as u64);
    (status == 0).then_some(old)
}

/// Maps a DLL by name: its base and path, or a status.
pub fn map_image(name: &[u8], path: &mut [u16; 260]) -> Result<u64, u32> {
    let mut base = 0u64;
    let status = call4(call::MAP_IMAGE, name.as_ptr() as u64, name.len() as u64, (&raw mut base) as u64, path.as_mut_ptr() as u64) as u32;
    if status == 0 { Ok(base) } else { Err(status) }
}

/// Sets this thread's registers from `context`, and carries on there.
pub fn continue_with(context: &crate::arch::Context) -> ! {
    call4(call::CONTINUE, context as *const _ as u64, 0, 0, 0);
    terminate(atlas_abi::status::INVALID_PARAMETER)
}

pub fn debug_print(text: &[u8]) {
    call4(call::DEBUG_PRINT, text.as_ptr() as u64, text.len() as u64, 0, 0);
}
