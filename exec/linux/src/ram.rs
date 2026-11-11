//! Guest RAM on Linux: a memfd, mapped shared, so KVM can use it and a sandbox can map the same memory into its stub.

use exec_core::PAGE;
use exec_core::contract::GuestRam;

pub fn alloc(bytes: u64) -> Result<GuestRam, String> {
    let size = bytes.max(PAGE).next_multiple_of(PAGE);
    // SAFETY: a name and flags.
    let fd = unsafe { libc::memfd_create(c"carbon-guest".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!("making guest memory: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: the memfd just made.
    if unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: as above, which nothing else has.
        unsafe { libc::close(fd) };
        return Err(format!("sizing guest memory: {error}"));
    }
    // SAFETY: a new shared mapping of the memfd, freed by `release`.
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED | libc::MAP_NORESERVE, fd, 0)
    };
    if ptr == libc::MAP_FAILED {
        let error = std::io::Error::last_os_error();
        // SAFETY: as above.
        unsafe { libc::close(fd) };
        return Err(format!("mapping guest memory: {error}"));
    }
    Ok(GuestRam { ptr: ptr.cast(), size, stride: PAGE, granule: PAGE, handle: i64::from(fd), discard, release })
}

fn discard(ptr: *mut u8, len: usize) -> bool {
    // SAFETY: whole pages of guest memory that nothing uses; for a shared memfd, MADV_REMOVE frees the pages behind them.
    unsafe { libc::madvise(ptr.cast(), len, libc::MADV_REMOVE) == 0 }
}

fn release(ptr: *mut u8, size: u64, handle: i64) {
    // SAFETY: the mapping and memfd `alloc` made, which nothing uses any more.
    unsafe {
        libc::munmap(ptr.cast(), size as usize);
        libc::close(handle as i32);
    }
}
