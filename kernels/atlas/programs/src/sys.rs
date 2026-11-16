//! The Win32 calls the programs make, imported from kernel32 by name (`raw-dylib`: no import library), with the
//! structures they take.

use core::ffi::c_void;

pub type HANDLE = *mut c_void;
pub type BOOL = i32;

pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const TRUE: BOOL = 1;

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const FILE_SHARE_DELETE: u32 = 4;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
pub const FILE_END: u32 = 2;
pub const MOVEFILE_REPLACE_EXISTING: u32 = 1;
pub const MOVEFILE_COPY_ALLOWED: u32 = 2;
pub const INFINITE: u32 = u32::MAX;
pub const STARTF_USESTDHANDLES: u32 = 0x100;
pub const CREATE_NEW_CONSOLE: u32 = 0x10;
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
pub const HANDLE_FLAG_INHERIT: u32 = 1;
pub const DUPLICATE_SAME_ACCESS: u32 = 2;
pub const SYMBOLIC_LINK_FLAG_DIRECTORY: u32 = 1;

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_NO_MORE_FILES: u32 = 18;
pub const ERROR_FILE_EXISTS: u32 = 80;
pub const ERROR_DIR_NOT_EMPTY: u32 = 145;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_INVALID_NAME: u32 = 123;
pub const ERROR_DIRECTORY: u32 = 267;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub low: u32,
    pub high: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEMTIME {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WIN32_FIND_DATAW {
    pub attributes: u32,
    pub created: FILETIME,
    pub accessed: FILETIME,
    pub written: FILETIME,
    pub size_high: u32,
    pub size_low: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub name: [u16; 260],
    pub short_name: [u16; 14],
}

impl Default for WIN32_FIND_DATAW {
    fn default() -> Self {
        WIN32_FIND_DATAW { attributes: 0, created: FILETIME::default(), accessed: FILETIME::default(), written: FILETIME::default(), size_high: 0, size_low: 0, reserved0: 0, reserved1: 0, name: [0; 260], short_name: [0; 14] }
    }
}

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub length: u32,
    pub descriptor: *mut c_void,
    pub inherit: BOOL,
}

impl SECURITY_ATTRIBUTES {
    /// Handles made with these are inherited by child processes.
    pub fn inherited() -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES { length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, descriptor: core::ptr::null_mut(), inherit: TRUE }
    }
}

#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: u32,
    pub reserved: *mut u16,
    pub desktop: *mut u16,
    pub title: *mut u16,
    pub x: u32,
    pub y: u32,
    pub x_size: u32,
    pub y_size: u32,
    pub x_chars: u32,
    pub y_chars: u32,
    pub fill: u32,
    pub flags: u32,
    pub show: u16,
    pub reserved2_size: u16,
    pub reserved2: *mut u8,
    pub stdin: HANDLE,
    pub stdout: HANDLE,
    pub stderr: HANDLE,
}

impl Default for STARTUPINFOW {
    fn default() -> Self {
        // SAFETY: all fields are numbers and pointers, for which zero is valid.
        let mut si: STARTUPINFOW = unsafe { core::mem::zeroed() };
        si.cb = core::mem::size_of::<STARTUPINFOW>() as u32;
        si
    }
}

#[repr(C)]
#[derive(Default)]
pub struct PROCESS_INFORMATION {
    pub process: usize,
    pub thread: usize,
    pub process_id: u32,
    pub thread_id: u32,
}

macro_rules! imports {
    ($dll:literal; $($(#[$m:meta])* fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)?;)*) => {
        #[cfg_attr(target_arch = "x86", link(name = $dll, kind = "raw-dylib", import_name_type = "undecorated"))]
        #[cfg_attr(not(target_arch = "x86"), link(name = $dll, kind = "raw-dylib"))]
        unsafe extern "system" {
            $($(#[$m])* pub fn $name($($arg: $ty),*) $(-> $ret)?;)*
        }
    };
}

imports! {
    "kernel32";
    fn GetStdHandle(which: u32) -> HANDLE;
    fn SetStdHandle(which: u32, handle: HANDLE) -> BOOL;
    fn WriteFile(handle: HANDLE, data: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn ReadFile(handle: HANDLE, data: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn WriteConsoleW(handle: HANDLE, data: *const u16, len: u32, written: *mut u32, reserved: *mut c_void) -> BOOL;
    fn ReadConsoleW(handle: HANDLE, data: *mut u16, len: u32, read: *mut u32, control: *mut c_void) -> BOOL;
    fn GetConsoleMode(handle: HANDLE, mode: *mut u32) -> BOOL;
    fn SetConsoleMode(handle: HANDLE, mode: u32) -> BOOL;
    fn SetConsoleTitleW(title: *const u16) -> BOOL;
    fn GetFileType(handle: HANDLE) -> u32;
    fn GetCommandLineW() -> *const u16;
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetProcessHeap() -> HANDLE;
    fn HeapAlloc(heap: HANDLE, flags: u32, size: usize) -> *mut u8;
    fn HeapReAlloc(heap: HANDLE, flags: u32, ptr: *mut u8, size: usize) -> *mut u8;
    fn HeapFree(heap: HANDLE, flags: u32, ptr: *mut u8) -> BOOL;
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> BOOL;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> BOOL;
    fn GetCurrentDirectoryW(size: u32, buffer: *mut u16) -> u32;
    fn SetCurrentDirectoryW(path: *const u16) -> BOOL;
    fn GetFullPathNameW(path: *const u16, size: u32, buffer: *mut u16, file_part: *mut *mut u16) -> u32;
    fn FindFirstFileW(pattern: *const u16, found: *mut WIN32_FIND_DATAW) -> HANDLE;
    fn FindNextFileW(find: HANDLE, found: *mut WIN32_FIND_DATAW) -> BOOL;
    fn FindClose(find: HANDLE) -> BOOL;
    fn CreateFileW(path: *const u16, access: u32, share: u32, security: *mut SECURITY_ATTRIBUTES, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn SetFilePointerEx(handle: HANDLE, distance: i64, new: *mut i64, method: u32) -> BOOL;
    fn DeleteFileW(path: *const u16) -> BOOL;
    fn CreateDirectoryW(path: *const u16, security: *mut SECURITY_ATTRIBUTES) -> BOOL;
    fn RemoveDirectoryW(path: *const u16) -> BOOL;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> BOOL;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn SetFileAttributesW(path: *const u16, attributes: u32) -> BOOL;
    fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
    fn CreateHardLinkW(link: *const u16, target: *const u16, security: *mut SECURITY_ATTRIBUTES) -> BOOL;
    fn CreateProcessW(app: *const u16, line: *mut u16, process_security: *mut SECURITY_ATTRIBUTES, thread_security: *mut SECURITY_ATTRIBUTES, inherit: BOOL, flags: u32, environment: *mut c_void, directory: *const u16, startup: *mut STARTUPINFOW, info: *mut PROCESS_INFORMATION) -> BOOL;
    fn WaitForSingleObject(handle: usize, ms: u32) -> u32;
    fn GetExitCodeProcess(process: usize, code: *mut u32) -> BOOL;
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, security: *mut SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn SetHandleInformation(handle: HANDLE, mask: u32, flags: u32) -> BOOL;
    fn DuplicateHandle(source_process: HANDLE, source: HANDLE, target_process: HANDLE, target: *mut HANDLE, access: u32, inherit: BOOL, options: u32) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetLocalTime(time: *mut SYSTEMTIME);
    fn FileTimeToLocalFileTime(utc: *const FILETIME, local: *mut FILETIME) -> BOOL;
    fn FileTimeToSystemTime(time: *const FILETIME, system: *mut SYSTEMTIME) -> BOOL;
    fn GetDiskFreeSpaceExW(path: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> BOOL;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_size: u32, serial: *mut u32, max_component: *mut u32, flags: *mut u32, fs_name: *mut u16, fs_size: u32) -> BOOL;
    fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> BOOL;
    fn GetModuleFileNameW(module: usize, buffer: *mut u16, size: u32) -> u32;
    fn Sleep(ms: u32);
    fn GetTickCount64() -> u64;
}

imports! {
    "advapi32";
    fn GetUserNameW(buffer: *mut u16, size: *mut u32) -> BOOL;
}
