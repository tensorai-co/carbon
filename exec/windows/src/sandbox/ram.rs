//! Guest RAM for the sandbox: a pagefile-backed section, which Carbon maps whole and the stub maps a page at a time.
//!
//! A view's offset into a section must be 64 KiB-aligned, so each frame starts a 64 KiB slot of its own (core's
//! `GuestRam::stride`), and only its first page is ever used. The section is reserved, not committed: Carbon's own view
//! commits a slot's page the first time it's touched, through a vectored exception handler, so the address space is
//! large but the memory is only what's used.

use std::ffi::c_void;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

use exec_core::PAGE;
use exec_core::contract::GuestRam;

use super::sys::*;

/// Windows' allocation granularity, and the spacing of frames in the section.
pub const SLOT: u64 = 0x1_0000;

static LOW: AtomicU64 = AtomicU64::new(u64::MAX);
static HIGH: AtomicU64 = AtomicU64::new(0);
static HANDLER: Once = Once::new();

/// Commits the page Carbon just touched in its view of guest memory, and only there.
extern "system" fn commit(pointers: *mut ExceptionPointers) -> i32 {
    // SAFETY: what Windows passes a vectored handler.
    let record = unsafe { &*(*pointers).record };
    if record.code != EXCEPTION_ACCESS_VIOLATION || record.parameters < 2 {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let addr = record.information[1] as u64;
    if addr < LOW.load(Ordering::Relaxed) || addr >= HIGH.load(Ordering::Relaxed) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let page = (addr & !(PAGE - 1)) as *mut c_void;
    // SAFETY: one page of the reserved section, within Carbon's view of it.
    let got = unsafe { VirtualAlloc(page, PAGE as usize, MEM_COMMIT, PAGE_READWRITE) };
    if got.is_null() { EXCEPTION_CONTINUE_SEARCH } else { EXCEPTION_CONTINUE_EXECUTION }
}

pub fn alloc(bytes: u64) -> Result<GuestRam, String> {
    let frames = bytes.max(PAGE) / PAGE;
    let size = frames * SLOT;
    // SAFETY: a new anonymous section, reserved only. Executable, so the stub's views of program code can be.
    let section = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_EXECUTE_READWRITE | SEC_RESERVE, (size >> 32) as u32, size as u32, std::ptr::null()) };
    if section.is_null() {
        return Err(format!("making guest memory: {}", last_error()));
    }
    // SAFETY: the whole section, for Carbon.
    let view = unsafe { MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size as usize) };
    if view.is_null() {
        let error = last_error();
        // SAFETY: the section just made.
        unsafe { CloseHandle(section) };
        return Err(format!("mapping guest memory: {error}"));
    }
    LOW.fetch_min(view as u64, Ordering::Relaxed);
    HIGH.fetch_max(view as u64 + size, Ordering::Relaxed);
    HANDLER.call_once(|| {
        // SAFETY: a handler that only commits within guest memory's views.
        unsafe { AddVectoredExceptionHandler(1, commit) };
    });
    Ok(GuestRam { ptr: view.cast(), size, stride: SLOT, granule: SLOT, handle: section as i64, discard, release })
}

fn discard(ptr: *mut u8, len: usize) -> bool {
    // SAFETY: whole slots of guest memory nothing uses: their contents may go; they stay committed.
    !unsafe { VirtualAlloc(ptr.cast(), len, MEM_RESET, PAGE_READWRITE) }.is_null()
}

fn release(ptr: *mut u8, _size: u64, handle: i64) {
    // SAFETY: the view and section `alloc` made, which nothing uses any more.
    unsafe {
        UnmapViewOfFile(ptr.cast());
        CloseHandle(handle as HANDLE);
    }
}
