//! The few Win32 calls used, declared by hand.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;

pub type HANDLE = *mut c_void;
pub type BOOL = i32;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const OPEN_EXISTING: u32 = 3;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
pub const ERROR_IO_PENDING: i32 = 997;
pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_RESET: u32 = 0x8_0000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const PAGE_READWRITE: u32 = 0x04;

/// `OVERLAPPED`: where an asynchronous call's result goes, and the event that says it has.
#[repr(C)]
pub struct OVERLAPPED {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u32,
    pub offset_high: u32,
    pub event: HANDLE,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    pub fn DeviceIoControl(device: HANDLE, code: u32, input: *const c_void, input_len: u32, output: *mut c_void, output_len: u32, returned: *mut u32, overlapped: *mut c_void) -> BOOL;
    pub fn CloseHandle(handle: HANDLE) -> BOOL;
    pub fn CreateEventW(security: *mut c_void, manual_reset: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn GetOverlappedResult(file: HANDLE, overlapped: *mut OVERLAPPED, transferred: *mut u32, wait: BOOL) -> BOOL;
    pub fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    pub fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> BOOL;
}

/// A NUL-terminated UTF-16 string.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
