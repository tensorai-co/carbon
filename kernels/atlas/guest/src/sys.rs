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

/// A call with up to eight arguments: past the fourth they go where a Windows call has them, in `x4` to `x7` on
/// ARM64 and on the stack, past the home space, on x86-64. How ntdll calls a host export that takes more than four.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub fn call8(nr: u64, a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64) -> u64 {
    let result: u64;
    // SAFETY: a trap into atlas, as `call4`; on x86-64, the stack is put back as it was.
    unsafe {
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("svc #0", in("x8") nr, inlateout("x0") a => result, in("x1") b, in("x2") c, in("x3") d, in("x4") e, in("x5") f, in("x6") g, in("x7") h, options(nostack));
        #[cfg(target_arch = "x86_64")]
        {
            let rest = [e, f, g, h];
            core::arch::asm!(
                "sub rsp, 0x48",
                "mov rax, [{rest}]",
                "mov [rsp + 0x20], rax",
                "mov rax, [{rest} + 8]",
                "mov [rsp + 0x28], rax",
                "mov rax, [{rest} + 16]",
                "mov [rsp + 0x30], rax",
                "mov rax, [{rest} + 24]",
                "mov [rsp + 0x38], rax",
                "mov rax, {nr}",
                "call r11",
                "add rsp, 0x48",
                rest = in(reg) rest.as_ptr(),
                nr = in(reg) nr,
                out("rax") result, in("r10") a, in("rdx") b, in("r8") c, in("r9") d,
                inlateout("r11") call::TRAP_AT => _, lateout("rcx") _,
            );
        }
    }
    result
}

/// A kernel32 host export's call number, found by name the first time it's wanted (`EXPORT_NUMBER`).
pub fn export(slot: &core::sync::atomic::AtomicU64, name: &[u8]) -> u64 {
    use core::sync::atomic::Ordering;
    let n = slot.load(Ordering::Relaxed);
    if n != 0 {
        return n;
    }
    let n = call4(call::EXPORT_NUMBER, name.as_ptr() as u64, name.len() as u64, 0, 0);
    slot.store(n, Ordering::Relaxed);
    n
}

/// Declares a function calling one of kernel32's host exports by number, as a program's stub would: 0 if atlas has
/// no such export.
macro_rules! host {
    ($name:ident, $export:literal, ($($arg:ident),*)) => {
        #[allow(clippy::too_many_arguments)]
        fn $name($($arg: u64),*) -> u64 {
            static NUMBER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
            let n = $crate::sys::export(&NUMBER, $export);
            if n == 0 {
                return 0;
            }
            $crate::sys::host!(@call n $(, $arg)*)
        }
    };
    (@call $n:ident $(, $arg:ident)*) => { $crate::sys::host!(@pad $n, [$($arg),*]) };
    (@pad $n:ident, [$($arg:ident),*]) => {{
        let a: [u64; 8] = { let mut a = [0u64; 8]; let given = [$($arg),*]; let mut i = 0; while i < given.len() { a[i] = given[i]; i += 1; } a };
        $crate::sys::call8($n, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7])
    }};
}
pub(crate) use host;

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
