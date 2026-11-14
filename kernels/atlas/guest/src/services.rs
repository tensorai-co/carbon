//! ntdll's system services, as Windows 10's x86-64 ntdll has them: each `Nt` export is a stub of 32 bytes,
//!
//! ```text
//! 4c 8b d1                  mov r10, rcx
//! b8 nn nn nn nn            mov eax, number
//! f6 04 25 08 03 fe 7f 01   test byte ptr [0x7ffe0308], 1   ; KUSER_SHARED_DATA.SystemCall: 0 here
//! 75 03                     jne 1f
//! 0f 05                     syscall
//! c3                        ret
//! cd 2e                  1: int 0x2e
//! c3                        ret
//! 0f 1f 84 00 00 00 00 00   nop
//! ```
//!
//! which programs and sandboxes that read ntdll's code, copy it or patch it (a browser's sandbox puts its interceptions
//! there) find as they expect. The number is atlas's (`call::NT_SERVICES` plus the service's place in the list below,
//! which never changes), and atlas, trapping the `syscall`, runs the export's body (exported as `AtlasImpl` and its
//! name) as if the stub had called it: the arguments where the caller put them, the first back in `rcx`. A copy of a
//! stub elsewhere works the same. On ARM64 and x86, each export is its body itself.
//!
//! Not yet: Windows-host sandboxes, which reach atlas through a gate rather than `syscall`, can't run these stubs.

/// A service's body, as written: on x86-64 under `AtlasImpl` and its name, for its stub to reach; elsewhere the export
/// itself.
macro_rules! service {
    ($(#[$attr:meta])* pub extern "system" fn $name:ident $args:tt -> $ret:ty $body:block) => {
        $(#[$attr])*
        #[cfg_attr(target_arch = "x86_64", unsafe(export_name = concat!("AtlasImpl", stringify!($name))))]
        #[cfg_attr(not(target_arch = "x86_64"), unsafe(no_mangle))]
        pub extern "system" fn $name $args -> $ret $body
    };
}

/// Services atlas hasn't written: on x86-64, a stub whose body says so and ends the process, as a host export's stub
/// does. Elsewhere there's no export at all, as before there were stubs.
macro_rules! unwritten_service {
    ($($name:ident),* $(,)?) => {$(
        #[cfg(target_arch = "x86_64")]
        #[unsafe(export_name = concat!("AtlasImpl", stringify!($name)))]
        pub extern "system" fn $name() -> u32 {
            crate::nt::unwritten(stringify!($name).as_bytes())
        }
    )*};
}

/// The stubs, and their exports (rustc doesn't export what's written in assembly).
macro_rules! stubs {
    ($($n:literal $name:ident),* $(,)?) => {
        #[cfg(target_arch = "x86_64")]
        core::arch::global_asm!(
            ".text",
            $(
                ".p2align 5",
                concat!(".globl ", stringify!($name)),
                concat!(stringify!($name), ":"),
                ".byte 0x4c, 0x8b, 0xd1, 0xb8",
                ".long {}",
                ".byte 0xf6, 0x04, 0x25, 0x08, 0x03, 0xfe, 0x7f, 0x01",
                ".byte 0x75, 0x03, 0x0f, 0x05, 0xc3, 0xcd, 0x2e, 0xc3",
                ".byte 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00",
            )*
            ".section .drectve,\"yni\"",
            $( concat!(".ascii \" /EXPORT:", stringify!($name), "\""), )*
            ".text",
            $( const atlas_abi::call::NT_SERVICES as u32 + $n, )*
        );
    };
}

// Each service's number, for good: a new one goes at the end.
stubs! {
    0 NtAllocateVirtualMemory,
    1 NtAlpcConnectPort,
    2 NtClose,
    3 NtCreateFile,
    4 NtCreateKey,
    5 NtCreateSection,
    6 NtDeleteKey,
    7 NtDeviceIoControlFile,
    8 NtDuplicateObject,
    9 NtFreeVirtualMemory,
    10 NtFsControlFile,
    11 NtGetNextThread,
    12 NtMapViewOfSection,
    13 NtOpenFile,
    14 NtOpenKey,
    15 NtOpenKeyEx,
    16 NtOpenProcess,
    17 NtOpenProcessToken,
    18 NtOpenProcessTokenEx,
    19 NtOpenThread,
    20 NtOpenThreadToken,
    21 NtOpenThreadTokenEx,
    22 NtProtectVirtualMemory,
    23 NtQueryAttributesFile,
    24 NtQueryFullAttributesFile,
    25 NtQueryInformationProcess,
    26 NtQueryInformationThread,
    27 NtQueryObject,
    28 NtQuerySection,
    29 NtQuerySystemInformation,
    30 NtQueryValueKey,
    31 NtQueryVirtualMemory,
    32 NtReadVirtualMemory,
    33 NtResumeProcess,
    34 NtSetInformationFile,
    35 NtSetInformationThread,
    36 NtSetValueKey,
    37 NtSignalAndWaitForSingleObject,
    38 NtSuspendProcess,
    39 NtUnmapViewOfSection,
    40 NtWaitForSingleObject,
    41 NtWriteFile,
}

unwritten_service!(
    NtAllocateVirtualMemory,
    NtAlpcConnectPort,
    NtCreateSection,
    NtDuplicateObject,
    NtFreeVirtualMemory,
    NtFsControlFile,
    NtMapViewOfSection,
    NtOpenFile,
    NtOpenProcess,
    NtOpenProcessToken,
    NtOpenProcessTokenEx,
    NtOpenThread,
    NtOpenThreadToken,
    NtOpenThreadTokenEx,
    NtProtectVirtualMemory,
    NtQueryAttributesFile,
    NtQueryFullAttributesFile,
    NtReadVirtualMemory,
    NtResumeProcess,
    NtSetInformationFile,
    NtSetInformationThread,
    NtSignalAndWaitForSingleObject,
    NtSuspendProcess,
    NtUnmapViewOfSection,
    NtWaitForSingleObject,
);
