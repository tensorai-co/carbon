//! Calls into atlas, by its own numbers: `svc #0` with the number in `x8` on ARM64; on x86-64, the number in `eax` and
//! the first argument in `r10`, as Windows' own system calls have them, through atlas's trampoline (`call::TRAP_AT`),
//! which is `syscall` or the backend's gate; on x86, `int 0x2e` with the number in `eax` and the arguments on the stack
//! above a return address, as a stdcall host export's stub has them. Arguments are pointer-sized there: each is
//! passed as its low 32 bits.

use atlas_abi::call;

#[inline(always)]
pub fn call4(nr: u64, a: u64, b: u64, c: u64, d: u64) -> u64 {
    let result: u64;
    // SAFETY: a trap into atlas, which changes only the result register (and, on x86-64, what `syscall` does).
    #[cfg(not(target_arch = "x86"))]
    unsafe {
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("svc #0", in("x8") nr, inlateout("x0") a => result, in("x1") b, in("x2") c, in("x3") d, options(nostack));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("call r11", inlateout("rax") nr => result, in("r10") a, in("rdx") b, in("r8") c, in("r9") d,
            inlateout("r11") call::TRAP_AT => _, lateout("rcx") _);
    }
    #[cfg(target_arch = "x86")]
    {
        result = stack_call(nr, &[a as u32, b as u32, c as u32, d as u32, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    result
}

/// x86's way in: the eight arguments pushed, last first, then a dummy return address, then `int 0x2e`.
#[cfg(target_arch = "x86")]
#[inline(always)]
fn stack_call(nr: u64, args: &[u32; MOST_ARGS]) -> u64 {
    let result: u32;
    // SAFETY: a trap into atlas, which changes only eax; the stack is put back as it was.
    unsafe {
        core::arch::asm!(
            "push dword ptr [{p} + 44]",
            "push dword ptr [{p} + 40]",
            "push dword ptr [{p} + 36]",
            "push dword ptr [{p} + 32]",
            "push dword ptr [{p} + 28]",
            "push dword ptr [{p} + 24]",
            "push dword ptr [{p} + 20]",
            "push dword ptr [{p} + 16]",
            "push dword ptr [{p} + 12]",
            "push dword ptr [{p} + 8]",
            "push dword ptr [{p} + 4]",
            "push dword ptr [{p}]",
            "push 0",
            "int 0x2e",
            "add esp, 52",
            p = in(reg) args.as_ptr(),
            inlateout("eax") nr as u32 => result,
            lateout("ecx") _, lateout("edx") _,
        );
    }
    u64::from(result)
}

/// The most arguments `call_n` passes, as many as the widest host export ntdll calls takes.
pub const MOST_ARGS: usize = 12;

/// A call with up to twelve arguments, where a Windows call has them: `x0` to `x7` then the stack on ARM64; four
/// registers then the stack past the home space on x86-64; all on the stack on x86. How ntdll calls a host export that
/// takes more than four.
#[inline(always)]
pub fn call_n(nr: u64, args: &[u64; MOST_ARGS]) -> u64 {
    let result: u64;
    // SAFETY: a trap into atlas, as `call4`; the stack is put back as it was.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let rest = [args[8], args[9], args[10], args[11]];
        core::arch::asm!(
            "sub sp, sp, #0x20",
            "ldp x9, x10, [{rest}]",
            "stp x9, x10, [sp]",
            "ldp x9, x10, [{rest}, #16]",
            "stp x9, x10, [sp, #16]",
            "svc #0",
            "add sp, sp, #0x20",
            rest = in(reg) rest.as_ptr(),
            in("x8") nr, inlateout("x0") args[0] => result, in("x1") args[1], in("x2") args[2], in("x3") args[3],
            in("x4") args[4], in("x5") args[5], in("x6") args[6], in("x7") args[7],
            out("x9") _, out("x10") _,
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let rest = &args[4..];
        core::arch::asm!(
            "sub rsp, 0x68",
            "mov rax, [{rest}]",
            "mov [rsp + 0x20], rax",
            "mov rax, [{rest} + 8]",
            "mov [rsp + 0x28], rax",
            "mov rax, [{rest} + 16]",
            "mov [rsp + 0x30], rax",
            "mov rax, [{rest} + 24]",
            "mov [rsp + 0x38], rax",
            "mov rax, [{rest} + 32]",
            "mov [rsp + 0x40], rax",
            "mov rax, [{rest} + 40]",
            "mov [rsp + 0x48], rax",
            "mov rax, [{rest} + 48]",
            "mov [rsp + 0x50], rax",
            "mov rax, [{rest} + 56]",
            "mov [rsp + 0x58], rax",
            "mov rax, {nr}",
            "call r11",
            "add rsp, 0x68",
            rest = in(reg) rest.as_ptr(),
            nr = in(reg) nr,
            out("rax") result, in("r10") args[0], in("rdx") args[1], in("r8") args[2], in("r9") args[3],
            inlateout("r11") call::TRAP_AT => _, lateout("rcx") _,
        );
    }
    #[cfg(target_arch = "x86")]
    {
        result = stack_call(nr, &args.map(|a| a as u32));
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
    let n = call4(call::EXPORT_NUMBER, name.as_ptr() as usize as u64, name.len() as u64, 0, 0);
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
        const GIVEN: usize = <[&str]>::len(&[$(stringify!($arg)),*]);
        const { assert!(GIVEN <= $crate::sys::MOST_ARGS, "more arguments than call_n passes") };
        let given: [u64; GIVEN] = [$($arg),*];
        let mut a = [0u64; $crate::sys::MOST_ARGS];
        a[..GIVEN].copy_from_slice(&given);
        $crate::sys::call_n($n, &a)
    }};
}
pub(crate) use host;

pub fn terminate(status: u32) -> ! {
    call4(call::TERMINATE_PROCESS, usize::MAX as u64, u64::from(status), 0, 0);
    // atlas never comes back from ending the process.
    loop {
        core::hint::spin_loop();
    }
}

/// Commits `size` bytes of new memory, read-write: its address, or null.
pub fn allocate(size: usize) -> *mut u8 {
    const COMMIT_RESERVE: u64 = 0x3000;
    const READ_WRITE: u64 = 4;
    call4(call::ALLOCATE_VIRTUAL_MEMORY, 0, size as u64, COMMIT_RESERVE, READ_WRITE) as usize as *mut u8
}

/// Changes pages' protection (a `PAGE_*`), giving the old one, or None.
pub fn protect(addr: u64, size: u64, protection: u32) -> Option<u32> {
    let mut old = 0u32;
    let status = call4(call::PROTECT_VIRTUAL_MEMORY, addr, size, u64::from(protection), (&raw mut old) as usize as u64);
    (status == 0).then_some(old)
}

/// Maps a DLL by name: its base and path, or a status.
pub fn map_image(name: &[u8], path: &mut [u16; 260]) -> Result<u64, u32> {
    let mut base = 0usize;
    let status = call4(call::MAP_IMAGE, name.as_ptr() as usize as u64, name.len() as u64, (&raw mut base) as usize as u64, path.as_mut_ptr() as usize as u64) as u32;
    if status == 0 { Ok(base as u64) } else { Err(status) }
}

/// Sets this thread's registers from `context`, and carries on there.
pub fn continue_with(context: &crate::arch::Context) -> ! {
    call4(call::CONTINUE, context as *const _ as usize as u64, 0, 0, 0);
    terminate(atlas_abi::status::INVALID_PARAMETER)
}

pub fn debug_print(text: &[u8]) {
    call4(call::DEBUG_PRINT, text.as_ptr() as usize as u64, text.len() as u64, 0, 0);
}
