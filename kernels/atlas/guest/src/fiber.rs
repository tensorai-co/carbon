//! Fibers: threads' own coroutines, as Windows has them. A fiber is a block of the program's memory with its
//! parameter first (so `GetFiberData` reads it), the TEB's stack fields and FLS block as they were when it last ran,
//! and its callee-saved registers. `SwitchToFiber` saves the running fiber's, puts the other's in the TEB and the
//! registers, and carries on where it left off, all in the program: there's no call into atlas.
//!
//! A fiber made by `CreateFiber` has a stack of its own, with guard pages at its bottom as a thread's has, so a
//! stack overflow on it is caught as on a thread. One that returns from its routine ends its thread (with 0), as on Windows.

use atlas_abi::teb;

use crate::sys::{self, call4};

/// Where each field is, for the assembly.
const PARAMETER: usize = 0x00;
const STACK_BASE: usize = 0x10;
const STACK_LIMIT: usize = 0x18;
const DEALLOCATION_STACK: usize = 0x20;
const FLS_DATA: usize = 0x28;
const START: usize = 0x30;
/// Its stack is its own, made by `CreateFiber`: deleting the fiber lets it go.
const OWNS_STACK: usize = 0x38;
const SP: usize = 0x40;
const PC: usize = 0x48;
/// x64: rbx, rbp, rdi, rsi, r12 to r15, then xmm6 to xmm15 at 0x90. ARM64: x19 to x30, then d8 to d15 at 0xb0.
const SIZE: usize = 0x140;

const HAS_FIBER_DATA: u16 = 4;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_ALREADY_FIBER: u32 = 1280;
const ERROR_ALREADY_THREAD: u32 = 1281;
/// A fiber's stack, unless it asks for more, and the guard pages at its bottom.
const STACK: u64 = 1 << 20;
const STACK_GUARD: u64 = 16 << 10;
const PAGE_READWRITE_GUARD: u32 = 0x104;
const MEM_RELEASE: u64 = 0x8000;

fn field(fiber: u64, at: usize) -> *mut u64 {
    (fiber + at as u64) as *mut u64
}

fn teb_u64(at: u64) -> *mut u64 {
    // SAFETY: a field of this thread's TEB.
    unsafe { crate::teb().add(at as usize).cast() }
}

fn flags() -> *mut u16 {
    // SAFETY: as above.
    unsafe { crate::teb().add(teb::SAME_TEB_FLAGS as usize).cast() }
}

fn current() -> u64 {
    // SAFETY: the TEB's FiberData.
    unsafe { teb_u64(teb::FIBER_DATA).read() }
}

/// `IsThreadAFiber()`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasIsThreadAFiber() -> i32 {
    // SAFETY: the TEB's flags.
    i32::from(unsafe { flags().read() } & HAS_FIBER_DATA != 0)
}

/// `ConvertThreadToFiber(parameter)`: this thread, as a fiber that runs on its own stack.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasConvertThreadToFiber(parameter: u64) -> u64 {
    AtlasConvertThreadToFiberEx(parameter, 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasConvertThreadToFiberEx(parameter: u64, _flags: u32) -> u64 {
    if AtlasIsThreadAFiber() != 0 {
        crate::RtlSetLastWin32Error(ERROR_ALREADY_FIBER);
        return 0;
    }
    let fiber = crate::heap::allocate(SIZE, true) as u64;
    if fiber == 0 {
        crate::RtlSetLastWin32Error(ERROR_NOT_ENOUGH_MEMORY);
        return 0;
    }
    // SAFETY: the new fiber, and this thread's TEB.
    unsafe {
        *field(fiber, PARAMETER) = parameter;
        *teb_u64(teb::FIBER_DATA) = fiber;
        *flags() |= HAS_FIBER_DATA;
    }
    fiber
}

/// `ConvertFiberToThread()`: the thread isn't a fiber any more.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasConvertFiberToThread() -> i32 {
    if AtlasIsThreadAFiber() == 0 {
        crate::RtlSetLastWin32Error(ERROR_ALREADY_THREAD);
        return 0;
    }
    let fiber = current();
    // SAFETY: this thread's TEB.
    unsafe {
        *teb_u64(teb::FIBER_DATA) = 0;
        *flags() &= !HAS_FIBER_DATA;
    }
    crate::heap::free(fiber as *mut u8);
    1
}

/// `CreateFiber(stack size, routine, parameter)`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateFiber(stack_size: u64, start: u64, parameter: u64) -> u64 {
    AtlasCreateFiberEx(stack_size, stack_size, 0, start, parameter)
}

/// `CreateFiberEx(commit, reserve, flags, routine, parameter)`: a fiber with a stack of its own, which first runs
/// `routine(parameter)` when it's switched to.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateFiberEx(commit: u64, reserve: u64, _flags: u32, start: u64, parameter: u64) -> u64 {
    if start == 0 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let size = reserve.max(commit).max(STACK).next_multiple_of(0x1_0000);
    let stack = sys::allocate(size as usize) as u64;
    let fiber = crate::heap::allocate(SIZE, true) as u64;
    if stack == 0 || fiber == 0 {
        crate::RtlSetLastWin32Error(ERROR_NOT_ENOUGH_MEMORY);
        return 0;
    }
    sys::protect(stack, STACK_GUARD, PAGE_READWRITE_GUARD);
    let top = stack + size;
    // SAFETY: the new fiber.
    unsafe {
        *field(fiber, PARAMETER) = parameter;
        *field(fiber, STACK_BASE) = top;
        *field(fiber, STACK_LIMIT) = stack + STACK_GUARD;
        *field(fiber, DEALLOCATION_STACK) = stack;
        *field(fiber, START) = start;
        *field(fiber, OWNS_STACK) = 1;
        *field(fiber, PC) = AtlasFiberStart as *const () as u64;
        // As if `AtlasFiberStart` had been called: x64 has a return address (0) on the stack, with the 32 bytes of
        // home space above it that it may use; ARM64's stack pointer is 16-aligned, with its link register 0.
        *field(fiber, SP) = if cfg!(target_arch = "x86_64") { top - 0x38 } else { top - 0x10 };
    }
    fiber
}

/// Where a fiber made by `CreateFiber` starts: its routine, and then, if that returns, the thread's end.
extern "system" fn AtlasFiberStart() -> ! {
    let fiber = current();
    // SAFETY: the fiber that's running: its routine and parameter.
    let (start, parameter) = unsafe { (*field(fiber, START), *field(fiber, PARAMETER)) };
    // SAFETY: the program's fiber routine, as Windows calls it. It returns nothing: the thread ends with 0.
    let start: extern "system" fn(u64) = unsafe { core::mem::transmute(start) };
    start(parameter);
    crate::loader::AtlasExitThread(0)
}

/// `DeleteFiber(fiber)`: its FLS's callbacks run, and its stack goes. Deleting the running fiber ends the thread.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDeleteFiber(fiber: u64) {
    if fiber == 0 {
        return;
    }
    if fiber == current() {
        crate::loader::AtlasExitThread(0);
    }
    // SAFETY: a fiber that isn't running, which the program gives up.
    unsafe {
        crate::sync::release_fls(*field(fiber, FLS_DATA) as *mut u64);
        if *field(fiber, OWNS_STACK) != 0 {
            call4(atlas_abi::call::FREE_VIRTUAL_MEMORY, *field(fiber, DEALLOCATION_STACK), 0, MEM_RELEASE, 0);
        }
    }
    crate::heap::free(fiber as *mut u8);
}

#[allow(dead_code)]
unsafe extern "system" {
    /// `SwitchToFiber(fiber)`: the running fiber's registers and TEB fields saved in it, and `fiber`'s taken.
    pub fn AtlasSwitchToFiber(fiber: u64);
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl AtlasSwitchToFiber",
    "AtlasSwitchToFiber:",
    "mov rdx, gs:[0x30]",
    "mov rax, [rdx + 0x20]",
    // This fiber: its callee-saved registers, where it returns to, and its stack as the TEB has it.
    "mov [rax + 0x50], rbx",
    "mov [rax + 0x58], rbp",
    "mov [rax + 0x60], rdi",
    "mov [rax + 0x68], rsi",
    "mov [rax + 0x70], r12",
    "mov [rax + 0x78], r13",
    "mov [rax + 0x80], r14",
    "mov [rax + 0x88], r15",
    "movdqa [rax + 0x90], xmm6",
    "movdqa [rax + 0xa0], xmm7",
    "movdqa [rax + 0xb0], xmm8",
    "movdqa [rax + 0xc0], xmm9",
    "movdqa [rax + 0xd0], xmm10",
    "movdqa [rax + 0xe0], xmm11",
    "movdqa [rax + 0xf0], xmm12",
    "movdqa [rax + 0x100], xmm13",
    "movdqa [rax + 0x110], xmm14",
    "movdqa [rax + 0x120], xmm15",
    "lea r8, [rsp + 8]",
    "mov [rax + 0x40], r8",
    "mov r8, [rsp]",
    "mov [rax + 0x48], r8",
    "mov r8, [rdx + 0x08]",
    "mov [rax + 0x10], r8",
    "mov r8, [rdx + 0x10]",
    "mov [rax + 0x18], r8",
    "mov r8, [rdx + 0x1478]",
    "mov [rax + 0x20], r8",
    "mov r8, [rdx + 0x17c8]",
    "mov [rax + 0x28], r8",
    // The other: its stack and FLS in the TEB, then its registers, then where it left off.
    "mov [rdx + 0x20], rcx",
    "mov r8, [rcx + 0x10]",
    "mov [rdx + 0x08], r8",
    "mov r8, [rcx + 0x18]",
    "mov [rdx + 0x10], r8",
    "mov r8, [rcx + 0x20]",
    "mov [rdx + 0x1478], r8",
    "mov r8, [rcx + 0x28]",
    "mov [rdx + 0x17c8], r8",
    "mov rbx, [rcx + 0x50]",
    "mov rbp, [rcx + 0x58]",
    "mov rdi, [rcx + 0x60]",
    "mov rsi, [rcx + 0x68]",
    "mov r12, [rcx + 0x70]",
    "mov r13, [rcx + 0x78]",
    "mov r14, [rcx + 0x80]",
    "mov r15, [rcx + 0x88]",
    "movdqa xmm6, [rcx + 0x90]",
    "movdqa xmm7, [rcx + 0xa0]",
    "movdqa xmm8, [rcx + 0xb0]",
    "movdqa xmm9, [rcx + 0xc0]",
    "movdqa xmm10, [rcx + 0xd0]",
    "movdqa xmm11, [rcx + 0xe0]",
    "movdqa xmm12, [rcx + 0xf0]",
    "movdqa xmm13, [rcx + 0x100]",
    "movdqa xmm14, [rcx + 0x110]",
    "movdqa xmm15, [rcx + 0x120]",
    "mov rsp, [rcx + 0x40]",
    "mov r8, [rcx + 0x48]",
    "jmp r8",
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".globl AtlasSwitchToFiber",
    "AtlasSwitchToFiber:",
    "ldr x9, [x18, #0x20]",
    "stp x19, x20, [x9, #0x50]",
    "stp x21, x22, [x9, #0x60]",
    "stp x23, x24, [x9, #0x70]",
    "stp x25, x26, [x9, #0x80]",
    "stp x27, x28, [x9, #0x90]",
    "stp x29, x30, [x9, #0xa0]",
    "stp d8, d9, [x9, #0xb0]",
    "stp d10, d11, [x9, #0xc0]",
    "stp d12, d13, [x9, #0xd0]",
    "stp d14, d15, [x9, #0xe0]",
    "mov x10, sp",
    "str x10, [x9, #0x40]",
    "str x30, [x9, #0x48]",
    "ldr x10, [x18, #0x08]",
    "str x10, [x9, #0x10]",
    "ldr x10, [x18, #0x10]",
    "str x10, [x9, #0x18]",
    "ldr x10, [x18, #0x1478]",
    "str x10, [x9, #0x20]",
    "ldr x10, [x18, #0x17c8]",
    "str x10, [x9, #0x28]",
    "str x0, [x18, #0x20]",
    "ldr x10, [x0, #0x10]",
    "str x10, [x18, #0x08]",
    "ldr x10, [x0, #0x18]",
    "str x10, [x18, #0x10]",
    "ldr x10, [x0, #0x20]",
    "str x10, [x18, #0x1478]",
    "ldr x10, [x0, #0x28]",
    "str x10, [x18, #0x17c8]",
    "ldp x19, x20, [x0, #0x50]",
    "ldp x21, x22, [x0, #0x60]",
    "ldp x23, x24, [x0, #0x70]",
    "ldp x25, x26, [x0, #0x80]",
    "ldp x27, x28, [x0, #0x90]",
    "ldp x29, x30, [x0, #0xa0]",
    "ldp d8, d9, [x0, #0xb0]",
    "ldp d10, d11, [x0, #0xc0]",
    "ldp d12, d13, [x0, #0xd0]",
    "ldp d14, d15, [x0, #0xe0]",
    "ldr x10, [x0, #0x40]",
    "mov sp, x10",
    "ldr x10, [x0, #0x48]",
    "br x10",
);

const _: () = assert!(SIZE >= 0x130 && PARAMETER == 0);
