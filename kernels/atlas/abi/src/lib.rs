//! What atlas and `atlas-guest` (the part of Windows that runs inside programs) share: the numbers of atlas's calls,
//! Windows' status codes, and where things are in the TEB and PEB. Both sides come from the same build, so there's no
//! version to check.
#![no_std]

/// How a program reaches atlas: `svc #0` with the number in `x8` (ARM64), or `syscall` with it in `eax` and the
/// first argument in `r10` (x86-64), as Windows' own system calls do.
pub mod call {
    /// atlas's own page of code in every process, below the system DLLs: what threads and upcalls return to, and
    /// x86-64's trampoline into atlas.
    pub const ATLAS_PAGE: u64 = 0x6ff0_0000_0000 - 0x1_0000;
    /// x86-64's way into atlas for code that can't know the backend (ntdll): `call` here, with the number in `eax` and
    /// the arguments as for `syscall`. It's `syscall; ret`, or, where programs reach their kernel through a gate (the
    /// Windows sandbox), a jump to the gate, which returns to the caller.
    pub const TRAP_AT: u64 = ATLAS_PAGE + 0x40;

    /// Calls `atlas-guest` makes, at the Nt level: one numbering for every ISA.
    pub const TERMINATE_PROCESS: u64 = 1;
    /// Maps a DLL, as the loader needs it: `(name, length, base_out, path_out)`. The name is ASCII, a bare file name
    /// or a Windows path. It's atlas's own, made for this ISA, or a file's, found as Windows would. atlas relocates it
    /// if it can't go at its preferred base, and writes its base, and its full path as UTF-16 (`MAX_PATH` units).
    pub const MAP_IMAGE: u64 = 2;
    pub const ALLOCATE_VIRTUAL_MEMORY: u64 = 3;
    pub const FREE_VIRTUAL_MEMORY: u64 = 4;
    pub const PROTECT_VIRTUAL_MEMORY: u64 = 5;
    pub const DEBUG_PRINT: u64 = 6;
    /// Sets the thread's registers from a CONTEXT: `NtContinue`, for the end of an exception or an unwind.
    pub const CONTINUE: u64 = 7;
    /// Ends this thread: `(exit code)`.
    pub const EXIT_THREAD: u64 = 8;
    /// Waits while memory is what it was: `(address, compare, size, timeout_ms)`, with `u32::MAX` for no timeout.
    /// 0 when woken or changed, STATUS_TIMEOUT (0x102) when the time's up.
    pub const WAIT_ADDRESS: u64 = 9;
    /// Wakes one waiter on an address, or all: `(address, all)`.
    pub const WAKE_ADDRESS: u64 = 10;
    /// The first thing a new thread does: a suspended thread waits here until it's resumed.
    pub const THREAD_STARTED: u64 = 11;
    /// `NtQuerySystemInformation(class, buffer, length, return_length)`: an NTSTATUS.
    pub const QUERY_SYSTEM_INFORMATION: u64 = 12;

    /// An upcall's callback has returned: this is the trap at the return address atlas gave it.
    pub const UPCALL_RETURN: u64 = 0xfff;
    /// The first thread returned from the program's entry point.
    pub const THREAD_RETURN: u64 = 0xffe;
    /// Host exports are numbered from here, in the order atlas declares them.
    pub const HOST_EXPORTS: u64 = 0x1000;
}

/// NTSTATUS values.
pub mod status {
    pub const SUCCESS: u32 = 0;
    pub const BREAKPOINT: u32 = 0x8000_0003;
    pub const NOT_IMPLEMENTED: u32 = 0xc000_0002;
    pub const ACCESS_VIOLATION: u32 = 0xc000_0005;
    pub const INVALID_HANDLE: u32 = 0xc000_0008;
    pub const INVALID_PARAMETER: u32 = 0xc000_000d;
    pub const NO_MEMORY: u32 = 0xc000_0017;
    pub const ILLEGAL_INSTRUCTION: u32 = 0xc000_001d;
    pub const DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
    pub const INTEGER_DIVIDE_BY_ZERO: u32 = 0xc000_0094;
    pub const DLL_NOT_FOUND: u32 = 0xc000_0135;
    pub const ENTRYPOINT_NOT_FOUND: u32 = 0xc000_0139;
    pub const STACK_OVERFLOW: u32 = 0xc000_00fd;
    pub const STACK_BUFFER_OVERRUN: u32 = 0xc000_0409;
    pub const NONCONTINUABLE_EXCEPTION: u32 = 0xc000_0025;
    pub const INVALID_IMAGE_FORMAT: u32 = 0xc000_007b;
    pub const UNWIND: u32 = 0xc000_0027;
}

/// The CONTEXT records exceptions carry, as Windows lays them out.
pub mod context {
    /// x64's: 0x4d0 bytes, 16-aligned. Rax to R15 are in x86's own numbering from `X64_GPRS`.
    pub const X64_SIZE: u64 = 0x4d0;
    pub const X64_FLAGS: u64 = 0x30;
    pub const X64_MXCSR: u64 = 0x34;
    pub const X64_EFLAGS: u64 = 0x44;
    pub const X64_GPRS: u64 = 0x78;
    pub const X64_RIP: u64 = 0xf8;
    pub const X64_XMM: u64 = 0x1a0;
    /// CONTEXT_AMD64 | CONTROL | INTEGER | SEGMENTS | FLOATING_POINT.
    pub const X64_FULL: u32 = 0x0010_000f;

    /// ARM64's: 0x390 bytes. X0 to X28, then Fp and Lr, are 31 registers in a row from `ARM64_X`.
    pub const ARM64_SIZE: u64 = 0x390;
    pub const ARM64_FLAGS: u64 = 0;
    pub const ARM64_CPSR: u64 = 4;
    pub const ARM64_X: u64 = 8;
    pub const ARM64_SP: u64 = 0x100;
    pub const ARM64_PC: u64 = 0x108;
    pub const ARM64_V: u64 = 0x110;
    pub const ARM64_FPCR: u64 = 0x310;
    pub const ARM64_FPSR: u64 = 0x314;
    /// CONTEXT_ARM64 | CONTROL | INTEGER | FLOATING_POINT.
    pub const ARM64_FULL: u32 = 0x0040_0007;
}

/// EXCEPTION_RECORD, 64-bit: 0x98 bytes.
pub mod record {
    pub const SIZE: u64 = 0x98;
    pub const CODE: u64 = 0;
    pub const FLAGS: u64 = 4;
    pub const ADDRESS: u64 = 0x10;
    pub const PARAMETER_COUNT: u64 = 0x18;
    pub const PARAMETERS: u64 = 0x20;
    pub const NONCONTINUABLE: u32 = 1;
}

/// Where things are in a 64-bit TEB.
pub mod teb {
    /// NT_TIB: the stack's top and bottom.
    pub const STACK_BASE: u64 = 0x08;
    pub const STACK_LIMIT: u64 = 0x10;
    /// NT_TIB.Self: the TEB's own address.
    pub const SELF: u64 = 0x30;
    pub const CLIENT_ID: u64 = 0x40;
    pub const PEB: u64 = 0x60;
    pub const LAST_ERROR: u64 = 0x68;
    /// The array of each module's static TLS block.
    pub const THREAD_LOCAL_STORAGE: u64 = 0x58;
    /// `TlsAlloc`'s 64 slots.
    pub const TLS_SLOTS: u64 = 0x1480;
    /// FlsData: a block of `FlsAlloc`'s slots, made when first wanted.
    pub const FLS_DATA: u64 = 0x17c8;
    /// atlas-guest's own: the exception dispatch in progress on this thread. Past the TEB's end; atlas gives every TEB
    /// room for it.
    pub const ATLAS_DISPATCH: u64 = 0x1900;
    pub const SIZE: u64 = 0x1838;
}

/// Where things are in a 64-bit PEB.
pub mod peb {
    pub const IMAGE_BASE: u64 = 0x10;
    pub const PROCESS_PARAMETERS: u64 = 0x20;
    pub const PROCESS_HEAP: u64 = 0x30;
    pub const NUMBER_OF_PROCESSORS: u64 = 0xb8;
    /// The Windows it is: major, minor, build (a u16), service pack (a u16), platform (2, NT).
    pub const OS_MAJOR_VERSION: u64 = 0x118;
    pub const OS_MINOR_VERSION: u64 = 0x11c;
    pub const OS_BUILD_NUMBER: u64 = 0x120;
    pub const OS_PLATFORM_ID: u64 = 0x124;
    /// The program's subsystem (2 GUI, 3 console) and its version, from its PE header.
    pub const IMAGE_SUBSYSTEM: u64 = 0x128;
    pub const IMAGE_SUBSYSTEM_MAJOR: u64 = 0x12c;
    pub const IMAGE_SUBSYSTEM_MINOR: u64 = 0x130;
    pub const SIZE: u64 = 0x7c8;
}

/// Where things are in 64-bit RTL_USER_PROCESS_PARAMETERS. Each is a UNICODE_STRING: lengths, then a pointer.
pub mod params {
    pub const STANDARD_INPUT: u64 = 0x20;
    pub const STANDARD_OUTPUT: u64 = 0x28;
    pub const STANDARD_ERROR: u64 = 0x30;
    pub const CURRENT_DIRECTORY: u64 = 0x38;
    pub const IMAGE_PATH_NAME: u64 = 0x60;
    pub const COMMAND_LINE: u64 = 0x70;
    /// The environment block: UTF-16 `NAME=VALUE` strings, each with a NUL, then a NUL.
    pub const ENVIRONMENT: u64 = 0x80;
    /// Its size, in bytes.
    pub const ENVIRONMENT_SIZE: u64 = 0x3f0;
    pub const SIZE: u64 = 0x440;
}
