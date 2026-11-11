//! Windows: memory from `GlobalMemoryStatusEx`, randomness from the system's generator, and certificates checked by
//! Windows' own chain engine. The console's raw mode is still to come (the to-do list's console on Windows hosts).

mod trust;

pub use trust::check as check_certificates;

#[repr(C)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
}

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *mut core::ffi::c_void, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;

/// Physical memory that's free now.
pub fn memory_available() -> u64 {
    // SAFETY: all numbers, so all-zero is one; the length says which version this is.
    let mut status: MemoryStatusEx = unsafe { std::mem::zeroed() };
    status.length = size_of::<MemoryStatusEx>() as u32;
    // SAFETY: room for what it fills in.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return 0;
    }
    status.avail_phys
}

pub fn random(buffer: &mut [u8]) {
    for chunk in buffer.chunks_mut(u32::MAX as usize) {
        // SAFETY: room for `chunk.len()` bytes, from the system's own generator.
        let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), chunk.as_mut_ptr(), chunk.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        assert!(status == 0, "the host gave no random bytes ({status:#x})");
    }
}

/// Raw mode on the console comes with the console on Windows hosts; until then, line editing is the console's own.
pub fn set_raw(_raw: bool) {}

pub fn terminal_size() -> Option<(u16, u16)> {
    None
}
