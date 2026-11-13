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
mod crt;
mod env;
mod exception;
mod fiber;
mod heap;
mod loader;
mod pe;
mod pool;
mod strings;
mod stdio;
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

/// This process's PEB.
fn peb() -> *mut u8 {
    // SAFETY: the TEB's PEB pointer, which atlas set and never moves.
    unsafe { teb().add(atlas_abi::teb::PEB as usize).cast::<*mut u8>().read() }
}

fn peb_u32(at: u64) -> u32 {
    // SAFETY: a field of the PEB atlas made.
    unsafe { peb().add(at as usize).cast::<u32>().read() }
}

/// `RtlGetVersion(&info)`: `RTL_OSVERSIONINFO(EX)W`, by the size in it, from the PEB. Unlike `GetVersionEx`, it tells
/// every program the truth, whatever its manifest says.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetVersion(info: *mut u8) -> u32 {
    use atlas_abi::peb;
    const STATUS_INVALID_PARAMETER: u32 = 0xc000_000d;
    // SAFETY: the program's structure, as big as its size says.
    unsafe {
        let size = info.cast::<u32>().read_unaligned();
        if size != 276 && size != 284 {
            return STATUS_INVALID_PARAMETER;
        }
        let fields = [
            peb_u32(peb::OS_MAJOR_VERSION),
            peb_u32(peb::OS_MINOR_VERSION),
            peb_u32(peb::OS_BUILD_NUMBER) & 0xffff,
            peb_u32(peb::OS_PLATFORM_ID),
        ];
        for (i, v) in fields.into_iter().enumerate() {
            info.add(4 + i * 4).cast::<u32>().write_unaligned(v);
        }
        // No service pack.
        core::ptr::write_bytes(info.add(20), 0, 256);
        if size == 284 {
            info.add(276).cast::<u32>().write_unaligned(0);
            // VER_SUITE_SINGLEUSERTS; VER_NT_WORKSTATION.
            info.add(280).cast::<u16>().write_unaligned(0x100);
            info.add(282).cast::<u16>().write_unaligned(1);
        }
    }
    0
}

/// `RtlGetNtVersionNumbers(&major, &minor, &build)`: the build with 0xf0000000, a free build's mark.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetNtVersionNumbers(major: *mut u32, minor: *mut u32, build: *mut u32) {
    use atlas_abi::peb;
    let values = [peb_u32(peb::OS_MAJOR_VERSION), peb_u32(peb::OS_MINOR_VERSION), peb_u32(peb::OS_BUILD_NUMBER) & 0xffff | 0xf000_0000];
    for (at, v) in [major, minor, build].into_iter().zip(values) {
        if !at.is_null() {
            // SAFETY: the program's u32.
            unsafe { at.write_unaligned(v) }
        }
    }
}

/// `NtQuerySystemInformation(class, buffer, length, &return length)`: atlas answers it.
#[unsafe(no_mangle)]
pub extern "system" fn NtQuerySystemInformation(class: u32, buffer: *mut c_void, length: u32, returned: *mut u32) -> u32 {
    sys::call4(atlas_abi::call::QUERY_SYSTEM_INFORMATION, u64::from(class), buffer as u64, u64::from(length), returned as u64) as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn ZwQuerySystemInformation(class: u32, buffer: *mut c_void, length: u32, returned: *mut u32) -> u32 {
    NtQuerySystemInformation(class, buffer, length, returned)
}
