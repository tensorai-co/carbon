//! Guest RAM on macOS: anonymous memory, in the host's 16 KiB pages, which both backends use.

use exec_core::PAGE;
use exec_core::contract::GuestRam;

/// Apple silicon's page size.
pub const GRANULE: u64 = 16 << 10;

pub fn alloc(bytes: u64) -> Result<GuestRam, String> {
    let size = bytes.max(GRANULE).next_multiple_of(GRANULE);
    // SAFETY: a new anonymous mapping, freed by `release`.
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_NORESERVE, -1, 0)
    };
    if ptr == libc::MAP_FAILED {
        return Err(format!("mapping guest memory: {}", std::io::Error::last_os_error()));
    }
    Ok(GuestRam { ptr: ptr.cast(), size, stride: PAGE, granule: GRANULE, handle: -1, discard, release })
}

fn discard(ptr: *mut u8, len: usize) -> bool {
    // SAFETY: whole granules of guest memory that nothing uses.
    unsafe { libc::madvise(ptr.cast(), len, libc::MADV_FREE_REUSABLE) == 0 }
}

fn release(ptr: *mut u8, size: u64, _handle: i64) {
    // SAFETY: the mapping `alloc` made, which nothing uses any more.
    unsafe { libc::munmap(ptr.cast(), size as usize) };
}
