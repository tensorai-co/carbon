//! What atlas and `atlas-guest` (the part of Windows that runs inside programs) share: the numbers of atlas's calls,
//! Windows' status codes, and where things are in the TEB and PEB. Both sides come from the same build, so there's no
//! version to check.
//!
//! The layouts with pointers in them come twice, 64-bit (`teb64`, `peb64`, …) and 32-bit (`teb32`, …), and `teb`,
//! `peb`, `ldr`, `params` and `record` are the ones of whatever this is built for: the guest gets its own ISA's, and
//! atlas, always 64-bit, picks a 32-bit process's by name.
#![no_std]

#[cfg(target_pointer_width = "64")]
pub use self::{ldr64 as ldr, params64 as params, peb64 as peb, record64 as record, teb64 as teb};
#[cfg(target_pointer_width = "32")]
pub use self::{ldr32 as ldr, params32 as params, peb32 as peb, record32 as record, teb32 as teb};

/// How a program reaches atlas: `svc #0` with the number in `x8` (ARM64), or `syscall` with it in `eax` and the
/// first argument in `r10` (x86-64), as Windows' own system calls do. 32-bit x86 uses `int 0x2e` with the number in
/// `eax` and its arguments on the stack above a return address, as a stdcall function has them when it's called: a
/// host export's stub is `mov eax, nr; int 0x2e; ret 4n`, and ntdll's own calls push theirs and a dummy return
/// address the same way.
pub mod call {
    /// atlas's own page of code in every process, below the system DLLs: what threads and upcalls return to, and
    /// x86-64's trampoline into atlas. A 32-bit process has it below 2 GiB, as WoW64 has its system DLLs.
    pub const ATLAS_PAGE64: u64 = 0x6ff0_0000_0000 - 0x1_0000;
    pub const ATLAS_PAGE32: u64 = 0x7000_0000 - 0x1_0000;
    #[cfg(target_pointer_width = "64")]
    pub const ATLAS_PAGE: u64 = ATLAS_PAGE64;
    #[cfg(target_pointer_width = "32")]
    pub const ATLAS_PAGE: u64 = ATLAS_PAGE32;
    /// x86-64's way into atlas for code that can't know the backend (ntdll): `call` here, with the number in `eax` and
    /// the arguments as for `syscall`. It's `syscall; ret`, or, where programs reach their kernel through a gate (the
    /// Windows sandbox), a jump to the gate, which returns to the caller.
    pub const TRAP_AT: u64 = ATLAS_PAGE + 0x40;

    /// Calls `atlas-guest` makes, at the Nt level: one numbering for every ISA.
    /// Ends the process: `(handle, code)`. With a handle of 0, as `RtlExitUserProcess` does first, it ends every
    /// other thread instead, and returns once they're gone.
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
    /// Unmaps a DLL the loader has let go of: `(base, size)`. atlas's own DLLs stay.
    pub const UNMAP_IMAGE: u64 = 13;
    /// Whether this is the process's only thread: 1 or 0 (`ThreadAmILastThread`).
    pub const LAST_THREAD: u64 = 14;
    /// Starts a thread of this process at `start(parameter)`, through ntdll's thread start as `CreateThread`'s do:
    /// `(start, parameter)`, giving 1, or 0 if it couldn't. The thread pool's workers.
    pub const CREATE_THREAD: u64 = 15;
    /// The call number of one of kernel32's host exports, by name: `(name, length)`, giving 0 if there's none. ntdll
    /// calls it with that number as a program's stub would, with up to four arguments.
    pub const EXPORT_NUMBER: u64 = 16;

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
    pub const GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;
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
    pub const DLL_INIT_FAILED: u32 = 0xc000_0142;
    pub const PROCEDURE_NOT_FOUND: u32 = 0xc000_007a;
    pub const ORDINAL_NOT_FOUND: u32 = 0xc000_0138;
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

    /// x86's: 0x2cc bytes, 4-aligned. FloatSave is x87's `fnsave` area (0x70 bytes); ExtendedRegisters is
    /// `fxsave`'s 512, with the xmm registers.
    pub const X86_SIZE: u64 = 0x2cc;
    pub const X86_FLAGS: u64 = 0;
    pub const X86_FLOAT_SAVE: u64 = 0x1c;
    pub const X86_SEG_GS: u64 = 0x8c;
    pub const X86_SEG_FS: u64 = 0x90;
    pub const X86_EDI: u64 = 0x9c;
    pub const X86_ESI: u64 = 0xa0;
    pub const X86_EBX: u64 = 0xa4;
    pub const X86_EDX: u64 = 0xa8;
    pub const X86_ECX: u64 = 0xac;
    pub const X86_EAX: u64 = 0xb0;
    pub const X86_EBP: u64 = 0xb4;
    pub const X86_EIP: u64 = 0xb8;
    pub const X86_SEG_CS: u64 = 0xbc;
    pub const X86_EFLAGS: u64 = 0xc0;
    pub const X86_ESP: u64 = 0xc4;
    pub const X86_SEG_SS: u64 = 0xc8;
    pub const X86_EXTENDED: u64 = 0xcc;
    /// CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS | FLOATING_POINT | EXTENDED_REGISTERS.
    pub const X86_FULL: u32 = 0x0001_002f;
}

/// EXCEPTION_RECORD, 64-bit: 0x98 bytes.
pub mod record64 {
    pub const SIZE: u64 = 0x98;
    pub const CODE: u64 = 0;
    pub const FLAGS: u64 = 4;
    pub const ADDRESS: u64 = 0x10;
    pub const PARAMETER_COUNT: u64 = 0x18;
    pub const PARAMETERS: u64 = 0x20;
    pub const NONCONTINUABLE: u32 = 1;
}

/// EXCEPTION_RECORD, 32-bit: 0x50 bytes.
pub mod record32 {
    pub const SIZE: u64 = 0x50;
    pub const CODE: u64 = 0;
    pub const FLAGS: u64 = 4;
    pub const ADDRESS: u64 = 0x0c;
    pub const PARAMETER_COUNT: u64 = 0x10;
    pub const PARAMETERS: u64 = 0x14;
    pub const NONCONTINUABLE: u32 = 1;
}

/// Where things are in a 64-bit TEB.
pub mod teb64 {
    /// NT_TIB: the stack's top and bottom.
    pub const STACK_BASE: u64 = 0x08;
    pub const STACK_LIMIT: u64 = 0x10;
    /// NT_TIB.FiberData: the fiber running on this thread, if it's been made a fiber.
    pub const FIBER_DATA: u64 = 0x20;
    /// NT_TIB.Self: the TEB's own address.
    pub const SELF: u64 = 0x30;
    pub const CLIENT_ID: u64 = 0x40;
    pub const PEB: u64 = 0x60;
    pub const LAST_ERROR: u64 = 0x68;
    /// The array of each module's static TLS block.
    pub const THREAD_LOCAL_STORAGE: u64 = 0x58;
    /// DeallocationStack: the bottom of the stack's reservation, below its guard pages.
    pub const DEALLOCATION_STACK: u64 = 0x1478;
    /// `TlsAlloc`'s 64 slots.
    pub const TLS_SLOTS: u64 = 0x1480;
    /// FlsData: a block of `FlsAlloc`'s slots, made when first wanted.
    pub const FLS_DATA: u64 = 0x17c8;
    /// SameTebFlags, a u16; `HasFiberData` (4) says the thread is a fiber.
    pub const SAME_TEB_FLAGS: u64 = 0x17ee;
    /// atlas-guest's own: the exception dispatch in progress on this thread. Past the TEB's end; atlas gives every TEB
    /// room for it.
    pub const ATLAS_DISPATCH: u64 = 0x1900;
    /// atlas-guest's own: the C runtime's `errno` for this thread.
    pub const ATLAS_ERRNO: u64 = 0x1908;
    pub const SIZE: u64 = 0x1838;
}

/// Where things are in a 32-bit TEB. `fs` points at it.
pub mod teb32 {
    /// NT_TIB.ExceptionList: the chain of SEH frames, newest first, ending at -1.
    pub const EXCEPTION_LIST: u64 = 0x00;
    pub const STACK_BASE: u64 = 0x04;
    pub const STACK_LIMIT: u64 = 0x08;
    pub const FIBER_DATA: u64 = 0x10;
    pub const SELF: u64 = 0x18;
    pub const CLIENT_ID: u64 = 0x20;
    pub const THREAD_LOCAL_STORAGE: u64 = 0x2c;
    pub const PEB: u64 = 0x30;
    pub const LAST_ERROR: u64 = 0x34;
    pub const DEALLOCATION_STACK: u64 = 0xe0c;
    pub const TLS_SLOTS: u64 = 0xe10;
    pub const FLS_DATA: u64 = 0xfb4;
    pub const SAME_TEB_FLAGS: u64 = 0xfca;
    /// atlas-guest's own, past the TEB's end, as in the 64-bit one.
    pub const ATLAS_DISPATCH: u64 = 0x1000;
    pub const ATLAS_ERRNO: u64 = 0x1004;
    pub const SIZE: u64 = 0xff8;
}

/// Where things are in a 64-bit PEB.
pub mod peb64 {
    pub const IMAGE_BASE: u64 = 0x10;
    /// PEB_LDR_DATA: the loader's three lists of modules, which ntdll keeps and programs walk.
    pub const LDR: u64 = 0x18;
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

/// Where things are in a 32-bit PEB.
pub mod peb32 {
    pub const IMAGE_BASE: u64 = 0x08;
    pub const LDR: u64 = 0x0c;
    pub const PROCESS_PARAMETERS: u64 = 0x10;
    pub const PROCESS_HEAP: u64 = 0x18;
    pub const NUMBER_OF_PROCESSORS: u64 = 0x64;
    pub const OS_MAJOR_VERSION: u64 = 0xa4;
    pub const OS_MINOR_VERSION: u64 = 0xa8;
    pub const OS_BUILD_NUMBER: u64 = 0xac;
    pub const OS_PLATFORM_ID: u64 = 0xb0;
    pub const IMAGE_SUBSYSTEM: u64 = 0xb4;
    pub const IMAGE_SUBSYSTEM_MAJOR: u64 = 0xb8;
    pub const IMAGE_SUBSYSTEM_MINOR: u64 = 0xbc;
    pub const SIZE: u64 = 0x480;
}

/// The loader's structures, 64-bit: PEB_LDR_DATA, and an LDR_DATA_TABLE_ENTRY for each module, as Windows 10 has them.
pub mod ldr64 {
    pub const DATA_SIZE: u64 = 0x58;
    pub const DATA_INITIALIZED: u64 = 0x04;
    /// Three LIST_ENTRY heads: load order, memory order, initialization order.
    pub const IN_LOAD_ORDER: u64 = 0x10;
    pub const IN_MEMORY_ORDER: u64 = 0x20;
    pub const IN_INITIALIZATION_ORDER: u64 = 0x30;
    pub const DATA_SHUTDOWN_IN_PROGRESS: u64 = 0x48;

    pub const ENTRY_SIZE: u64 = 0x120;
    /// An entry's links into each list.
    pub const LOAD_LINKS: u64 = 0x00;
    pub const MEMORY_LINKS: u64 = 0x10;
    pub const INITIALIZATION_LINKS: u64 = 0x20;
    pub const DLL_BASE: u64 = 0x30;
    pub const ENTRY_POINT: u64 = 0x38;
    pub const SIZE_OF_IMAGE: u64 = 0x40;
    /// UNICODE_STRINGs.
    pub const FULL_DLL_NAME: u64 = 0x48;
    pub const BASE_DLL_NAME: u64 = 0x58;
    pub const FLAGS: u64 = 0x68;
    pub const OBSOLETE_LOAD_COUNT: u64 = 0x6c;
    pub const TLS_INDEX: u64 = 0x6e;
    pub const TIME_DATE_STAMP: u64 = 0x80;
    pub const ORIGINAL_BASE: u64 = 0xf8;
    pub const LOAD_REASON: u64 = 0x10c;
    pub const REFERENCE_COUNT: u64 = 0x114;

    /// Flags: an image that's a DLL; its DllMain ran; its DllMain had DLL_PROCESS_ATTACH.
    pub const IMAGE_DLL: u32 = 0x4;
    pub const ENTRY_PROCESSED: u32 = 0x4000;
    pub const PROCESS_ATTACH_CALLED: u32 = 0x8_0000;
    /// LDR_DLL_LOAD_REASON: a static import, or `LoadLibrary`.
    pub const REASON_STATIC: u32 = 0;
    pub const REASON_DYNAMIC: u32 = 4;
}

/// The same, 32-bit: LIST_ENTRYs of two pointers, UNICODE_STRINGs of 8 bytes.
pub mod ldr32 {
    pub const DATA_SIZE: u64 = 0x30;
    pub const DATA_INITIALIZED: u64 = 0x04;
    pub const IN_LOAD_ORDER: u64 = 0x0c;
    pub const IN_MEMORY_ORDER: u64 = 0x14;
    pub const IN_INITIALIZATION_ORDER: u64 = 0x1c;
    pub const DATA_SHUTDOWN_IN_PROGRESS: u64 = 0x28;

    pub const ENTRY_SIZE: u64 = 0xa8;
    pub const LOAD_LINKS: u64 = 0x00;
    pub const MEMORY_LINKS: u64 = 0x08;
    pub const INITIALIZATION_LINKS: u64 = 0x10;
    pub const DLL_BASE: u64 = 0x18;
    pub const ENTRY_POINT: u64 = 0x1c;
    pub const SIZE_OF_IMAGE: u64 = 0x20;
    pub const FULL_DLL_NAME: u64 = 0x24;
    pub const BASE_DLL_NAME: u64 = 0x2c;
    pub const FLAGS: u64 = 0x34;
    pub const OBSOLETE_LOAD_COUNT: u64 = 0x38;
    pub const TLS_INDEX: u64 = 0x3a;
    pub const TIME_DATE_STAMP: u64 = 0x44;
    pub const ORIGINAL_BASE: u64 = 0x80;
    pub const LOAD_REASON: u64 = 0x94;
    pub const REFERENCE_COUNT: u64 = 0x9c;

    pub use super::ldr64::{ENTRY_PROCESSED, IMAGE_DLL, PROCESS_ATTACH_CALLED, REASON_DYNAMIC, REASON_STATIC};
}

/// Where things are in 64-bit RTL_USER_PROCESS_PARAMETERS. Each is a UNICODE_STRING: lengths, then a pointer.
pub mod params64 {
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

/// The same, 32-bit.
pub mod params32 {
    pub const STANDARD_INPUT: u64 = 0x18;
    pub const STANDARD_OUTPUT: u64 = 0x1c;
    pub const STANDARD_ERROR: u64 = 0x20;
    pub const CURRENT_DIRECTORY: u64 = 0x24;
    pub const IMAGE_PATH_NAME: u64 = 0x38;
    pub const COMMAND_LINE: u64 = 0x40;
    pub const ENVIRONMENT: u64 = 0x48;
    pub const ENVIRONMENT_SIZE: u64 = 0x290;
    pub const SIZE: u64 = 0x2b0;
}

/// Windows-1252, the ANSI code page atlas's `*A` functions speak (`GetACP`), both ways. Its 0x80 to 0x9F aren't
/// Latin-1's; the bytes Windows leaves unassigned there stand for the C1 controls, as `MultiByteToWideChar` has them.
pub mod ansi {
    const HIGH: [u16; 32] = [
        0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008d, 0x017d, 0x008f,
        0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
    ];

    /// A byte of Windows-1252 as UTF-16.
    pub fn from_1252(b: u8) -> u16 {
        if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] } else { u16::from(b) }
    }

    /// A UTF-16 unit as Windows-1252, if it has it.
    pub fn to_1252(u: u16) -> Option<u8> {
        match u {
            0..0x80 | 0xa0..0x100 => Some(u as u8),
            _ => HIGH.iter().position(|&h| h == u).map(|i| 0x80 + i as u8),
        }
    }
}
