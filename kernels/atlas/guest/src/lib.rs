//! atlas-guest: the part of Windows that has to run inside the program, presented as `ntdll.dll`. It's built for each
//! ISA a program can use (by atlas's `build.rs`), embedded in Carbon's binary, and mapped into every process. Every
//! other system DLL forwards its guest exports here.
//!
//! What's here is what Windows runs in the program because it must be fast or must touch the program's own memory:
//! the loader (`loader`), structured exception handling (`exception`, `unwind`), heaps, the last error and critical
//! sections. The C runtime's hot half comes next.
#![no_std]
#![allow(non_snake_case)]

mod arch;
mod env;
mod exception;
mod heap;
mod loader;
mod pe;
mod strings;
mod sync;
mod sys;
mod unwind;

use core::ffi::c_void;

/// MSVC's linker wants this wherever floating point might be used; the C runtime that usually defines it isn't here.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    sys::terminate(atlas_abi::status::STACK_BUFFER_OVERRUN)
}

/// This thread's TEB: in `x18` on ARM64, at `gs:[0x30]` on x86-64.
#[inline(always)]
pub(crate) fn teb() -> *mut u8 {
    let teb: *mut u8;
    // SAFETY: reading the register Windows keeps the TEB in.
    unsafe {
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("mov {}, x18", out(reg) teb, options(nomem, nostack, preserves_flags));
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, preserves_flags, readonly));
    }
    teb
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlGetLastWin32Error() -> u32 {
    // SAFETY: the TEB is this thread's, and the last error is a u32 in it.
    unsafe { teb().add(atlas_abi::teb::LAST_ERROR as usize).cast::<u32>().read_volatile() }
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlSetLastWin32Error(error: u32) {
    // SAFETY: as above.
    unsafe { teb().add(atlas_abi::teb::LAST_ERROR as usize).cast::<u32>().write_volatile(error) }
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlGetProcessHeap() -> *mut c_void {
    heap::process_heap()
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlAllocateHeap(_heap: *mut c_void, flags: u32, size: usize) -> *mut c_void {
    heap::allocate(size, flags & heap::ZERO_MEMORY != 0).cast()
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlFreeHeap(_heap: *mut c_void, _flags: u32, ptr: *mut c_void) -> u32 {
    heap::free(ptr.cast());
    1
}
