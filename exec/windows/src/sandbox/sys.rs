//! The Win32 and NT calls the sandbox makes, declared by hand, and the structures they pass.
#![allow(non_snake_case, clippy::upper_case_acronyms, clippy::too_many_arguments)]

use std::ffi::c_void;

pub type HANDLE = *mut c_void;
pub type BOOL = i32;

pub const INFINITE: u32 = u32::MAX;
pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

pub const DEBUG_ONLY_THIS_PROCESS: u32 = 0x2;
pub const CREATE_SUSPENDED: u32 = 0x4;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const EXCEPTION_DEBUG_EVENT: u32 = 1;
pub const CREATE_THREAD_DEBUG_EVENT: u32 = 2;
pub const CREATE_PROCESS_DEBUG_EVENT: u32 = 3;
pub const EXIT_PROCESS_DEBUG_EVENT: u32 = 5;
pub const LOAD_DLL_DEBUG_EVENT: u32 = 6;
pub const DBG_CONTINUE: u32 = 0x0001_0002;

pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;
pub const STATUS_DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const STATUS_PRIVILEGED_INSTRUCTION: u32 = 0xC000_0096;
pub const STATUS_INTEGER_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
/// `div` or `idiv` whose quotient doesn't fit: x86's #DE, as the divide by zero is.
pub const STATUS_INTEGER_OVERFLOW: u32 = 0xC000_0095;
pub const STATUS_STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const MEM_FREE: u32 = 0x1_0000;
pub const MEM_RESET: u32 = 0x8_0000;
pub const MEM_RESERVE_PLACEHOLDER: u32 = 0x4_0000;
pub const MEM_REPLACE_PLACEHOLDER: u32 = 0x4000;
pub const MEM_PRESERVE_PLACEHOLDER: u32 = 0x2;
pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READONLY: u32 = 0x02;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_EXECUTE: u32 = 0x10;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const SEC_RESERVE: u32 = 0x0400_0000;
pub const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;

pub const TOKEN_DUPLICATE: u32 = 0x0002;
pub const TOKEN_QUERY: u32 = 0x0008;
pub const TOKEN_ASSIGN_PRIMARY: u32 = 0x0001;
pub const TOKEN_ADJUST_DEFAULT: u32 = 0x0080;
pub const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
pub const LUA_TOKEN: u32 = 0x4;
pub const TOKEN_INTEGRITY_LEVEL: u32 = 25;
pub const SE_GROUP_INTEGRITY: u32 = 0x20;

pub const JOB_OBJECT_BASIC_UI_RESTRICTIONS: u32 = 4;
pub const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
pub const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
pub const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x400;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
pub const JOB_OBJECT_UILIMIT_ALL: u32 = 0xFF;

pub const CONTEXT_AMD64: u32 = 0x0010_0000;
/// Control (rip, rsp, eflags), integer and floating-point state. Never segments or debug registers.
pub const CONTEXT_PROGRAM: u32 = CONTEXT_AMD64 | 0x1 | 0x2 | 0x8;

pub const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// A 64-bit thread's `CONTEXT`. The general registers are in x86's encoding order.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Context {
    pub home: [u64; 6],
    pub flags: u32,
    pub mxcsr: u32,
    pub segments: [u16; 6],
    pub eflags: u32,
    pub debug: [u64; 6],
    /// rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15.
    pub gpr: [u64; 16],
    pub rip: u64,
    /// FXSAVE's layout.
    pub fxsave: [u8; 512],
    pub vector: [u8; 416],
    pub tail: [u64; 6],
}

const _: () = assert!(size_of::<Context>() == 1232);
const _: () = assert!(std::mem::offset_of!(Context, gpr) == 0x78);
const _: () = assert!(std::mem::offset_of!(Context, rip) == 0xF8);
const _: () = assert!(std::mem::offset_of!(Context, fxsave) == 0x100);

impl Context {
    pub fn new() -> Context {
        // SAFETY: plain integers, for which all zeroes is a value.
        let mut context: Context = unsafe { std::mem::zeroed() };
        context.flags = CONTEXT_PROGRAM;
        context
    }
}

/// A `DEBUG_EVENT`, its union read as needed.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct DebugEvent {
    pub code: u32,
    pub pid: u32,
    pub tid: u32,
    pad: u32,
    pub info: [u64; 20],
}

const _: () = assert!(size_of::<DebugEvent>() == 176);

impl DebugEvent {
    pub fn new() -> DebugEvent {
        DebugEvent { code: 0, pid: 0, tid: 0, pad: 0, info: [0; 20] }
    }

    pub fn exception_code(&self) -> u32 {
        self.info[0] as u32
    }

    pub fn exception_address(&self) -> u64 {
        self.info[2]
    }

    /// `ExceptionInformation[i]`.
    pub fn exception_parameter(&self, i: usize) -> u64 {
        self.info.get(4 + i).copied().unwrap_or(0)
    }

    /// The first handle the event carries: a file for CREATE_PROCESS and LOAD_DLL, a thread for CREATE_THREAD.
    pub fn handle(&self) -> HANDLE {
        self.info[0] as HANDLE
    }

    /// CREATE_PROCESS's image base.
    pub fn image_base(&self) -> u64 {
        self.info[3]
    }
}

#[repr(C)]
pub struct StartupInfo {
    pub cb: u32,
    pub rest: [u8; 100],
}

#[repr(C)]
pub struct ProcessInformation {
    pub process: HANDLE,
    pub thread: HANDLE,
    pub pid: u32,
    pub tid: u32,
}

#[repr(C)]
pub struct MemoryBasicInformation {
    pub base: u64,
    pub allocation_base: u64,
    pub allocation_protect: u32,
    pub partition: u16,
    pub size: u64,
    pub state: u32,
    pub protect: u32,
    pub kind: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct JobLimits {
    pub per_process_time: i64,
    pub per_job_time: i64,
    pub flags: u32,
    pub min_working_set: usize,
    pub max_working_set: usize,
    pub active_processes: u32,
    pub affinity: usize,
    pub priority_class: u32,
    pub scheduling_class: u32,
    pub io: [u64; 6],
    pub process_memory: usize,
    pub job_memory: usize,
    pub peak_process_memory: usize,
    pub peak_job_memory: usize,
}

const _: () = assert!(size_of::<JobLimits>() == 144);

#[repr(C)]
pub struct ExceptionPointers {
    pub record: *mut ExceptionRecord,
    pub context: *mut c_void,
}

#[repr(C)]
pub struct ExceptionRecord {
    pub code: u32,
    pub flags: u32,
    pub next: *mut ExceptionRecord,
    pub address: *mut c_void,
    pub parameters: u32,
    pub information: [usize; 15],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn CreateProcessAsUserW(token: HANDLE, app: *const u16, command: *mut u16, pa: *const c_void, ta: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, dir: *const u16, si: *const StartupInfo, pi: *mut ProcessInformation) -> BOOL;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn WaitForDebugEventEx(event: *mut DebugEvent, milliseconds: u32) -> BOOL;
    pub fn ContinueDebugEvent(pid: u32, tid: u32, status: u32) -> BOOL;
    pub fn DebugActiveProcessStop(pid: u32) -> BOOL;
    pub fn DebugSetProcessKillOnExit(kill: BOOL) -> BOOL;
    pub fn GetThreadContext(thread: HANDLE, context: *mut Context) -> BOOL;
    pub fn SetThreadContext(thread: HANDLE, context: *const Context) -> BOOL;
    pub fn SuspendThread(thread: HANDLE) -> u32;
    pub fn ResumeThread(thread: HANDLE) -> u32;
    pub fn TerminateProcess(process: HANDLE, code: u32) -> BOOL;
    pub fn TerminateThread(thread: HANDLE, code: u32) -> BOOL;
    pub fn CloseHandle(handle: HANDLE) -> BOOL;
    pub fn ReadProcessMemory(process: HANDLE, at: u64, buffer: *mut c_void, size: usize, done: *mut usize) -> BOOL;
    pub fn WriteProcessMemory(process: HANDLE, at: u64, buffer: *const c_void, size: usize, done: *mut usize) -> BOOL;
    pub fn VirtualAlloc(at: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    pub fn VirtualFreeEx(process: HANDLE, at: u64, size: usize, kind: u32) -> BOOL;
    pub fn VirtualQueryEx(process: HANDLE, at: u64, info: *mut MemoryBasicInformation, size: usize) -> usize;
    pub fn CreateFileMappingW(file: HANDLE, attributes: *const c_void, protect: u32, high: u32, low: u32, name: *const u16) -> HANDLE;
    pub fn MapViewOfFile(section: HANDLE, access: u32, high: u32, low: u32, size: usize) -> *mut c_void;
    pub fn UnmapViewOfFile(at: *const c_void) -> BOOL;
    pub fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> HANDLE;
    pub fn SetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, size: u32) -> BOOL;
    pub fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> BOOL;
    pub fn AddVectoredExceptionHandler(first: u32, handler: extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
}

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn VirtualAlloc2(process: HANDLE, at: u64, size: usize, kind: u32, protect: u32, parameters: *mut c_void, count: u32) -> u64;
    pub fn MapViewOfFile3(section: HANDLE, process: HANDLE, at: u64, offset: u64, size: usize, kind: u32, protect: u32, parameters: *mut c_void, count: u32) -> u64;
    pub fn UnmapViewOfFile2(process: HANDLE, at: u64, flags: u32) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    pub fn CreateRestrictedToken(token: HANDLE, flags: u32, disable_count: u32, disable: *const c_void, delete_count: u32, delete: *const c_void, restrict_count: u32, restrict: *const c_void, new: *mut HANDLE) -> BOOL;
    pub fn SetTokenInformation(token: HANDLE, class: u32, info: *const c_void, size: u32) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn NtQueryInformationThread(thread: HANDLE, class: u32, info: *mut c_void, size: u32, returned: *mut u32) -> i32;
}

pub fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// A NUL-terminated UTF-16 string.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A handle that can cross threads: the sandbox only uses them from the thread that owns its CPU, but ends them from
/// wherever it's dropped.
#[derive(Clone, Copy)]
pub struct Handle(pub HANDLE);

// SAFETY: an OS handle, valid from any thread.
unsafe impl Send for Handle {}
// SAFETY: as above.
unsafe impl Sync for Handle {}
