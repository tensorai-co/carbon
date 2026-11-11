//! Guest RAM on Windows: committed memory, which the driver pins. The sandbox will want a pagefile-backed section
//! instead, with frames spaced 64 KiB apart, so its stub can map each page.

use exec_core::PAGE;
use exec_core::contract::GuestRam;

use crate::win32::*;

pub fn alloc(bytes: u64) -> Result<GuestRam, String> {
    let size = bytes.max(PAGE).next_multiple_of(PAGE);
    // SAFETY: a new allocation, freed by `release`.
    let ptr = unsafe { VirtualAlloc(std::ptr::null_mut(), size as usize, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if ptr.is_null() {
        return Err(format!("allocating guest memory: {}", std::io::Error::last_os_error()));
    }
    Ok(GuestRam { ptr: ptr.cast(), size, stride: PAGE, granule: PAGE, handle: -1, discard, release })
}

fn discard(ptr: *mut u8, len: usize) -> bool {
    // SAFETY: whole pages of guest memory that nothing uses: their contents may go, they stay committed.
    !unsafe { VirtualAlloc(ptr.cast(), len, MEM_RESET, PAGE_READWRITE) }.is_null()
}

fn release(ptr: *mut u8, _size: u64, _handle: i64) {
    // SAFETY: the allocation `alloc` made, which nothing uses any more.
    unsafe { VirtualFree(ptr.cast(), 0, MEM_RELEASE) };
}
