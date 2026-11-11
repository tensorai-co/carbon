//! Each ISA's CONTEXT, as Windows lays it out, and capturing the caller's registers into one.

use atlas_abi::context;

#[cfg(target_arch = "x86_64")]
mod imp {
    use super::context;

    /// x64's CONTEXT. Only what atlas-guest reads or writes is named; the rest is kept as it came.
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    pub struct Context {
        pub home: [u64; 6],
        pub flags: u32,
        pub mxcsr: u32,
        pub segments: [u16; 6],
        pub eflags: u32,
        pub debug: [u64; 6],
        /// Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi, R8 to R15: x86's own numbering.
        pub x: [u64; 16],
        pub pc: u64,
        pub float_header: [u8; 0xa0],
        pub xmm: [u128; 16],
        pub rest: [u8; 0x4d0 - 0x2a0],
    }

    const _: () = assert!(size_of::<Context>() == context::X64_SIZE as usize);
    const _: () = assert!(core::mem::offset_of!(Context, x) == context::X64_GPRS as usize);
    const _: () = assert!(core::mem::offset_of!(Context, pc) == context::X64_RIP as usize);
    const _: () = assert!(core::mem::offset_of!(Context, xmm) == context::X64_XMM as usize);

    pub const SP: usize = 4;
    /// Where a function's result goes.
    pub const RESULT: usize = 0;

    impl Context {
        pub fn sp(&self) -> u64 {
            self.x[SP]
        }
    }

    unsafe extern "system" {
        /// Fills a CONTEXT with its caller's registers, as they'll be when it returns.
        pub fn RtlCaptureContext(context: *mut Context);
    }

    core::arch::global_asm!(
        ".globl RtlCaptureContext",
        "RtlCaptureContext:",
        "mov [rcx + 0x78], rax",
        "mov [rcx + 0x80], rcx",
        "mov [rcx + 0x88], rdx",
        "mov [rcx + 0x90], rbx",
        "lea rax, [rsp + 8]",
        "mov [rcx + 0x98], rax",
        "mov [rcx + 0xa0], rbp",
        "mov [rcx + 0xa8], rsi",
        "mov [rcx + 0xb0], rdi",
        "mov [rcx + 0xb8], r8",
        "mov [rcx + 0xc0], r9",
        "mov [rcx + 0xc8], r10",
        "mov [rcx + 0xd0], r11",
        "mov [rcx + 0xd8], r12",
        "mov [rcx + 0xe0], r13",
        "mov [rcx + 0xe8], r14",
        "mov [rcx + 0xf0], r15",
        "mov rax, [rsp]",
        "mov [rcx + 0xf8], rax",
        // The flags and MXCSR as a fresh call has them: nothing that unwinds or continues from here depends on them,
        // and `pushfq` and `stmxcsr` are beyond exec's x86 translator.
        "mov dword ptr [rcx + 0x44], 0x202",
        "mov dword ptr [rcx + 0x34], 0x1f80",
        "movdqa [rcx + 0x1a0], xmm0",
        "movdqa [rcx + 0x1b0], xmm1",
        "movdqa [rcx + 0x1c0], xmm2",
        "movdqa [rcx + 0x1d0], xmm3",
        "movdqa [rcx + 0x1e0], xmm4",
        "movdqa [rcx + 0x1f0], xmm5",
        "movdqa [rcx + 0x200], xmm6",
        "movdqa [rcx + 0x210], xmm7",
        "movdqa [rcx + 0x220], xmm8",
        "movdqa [rcx + 0x230], xmm9",
        "movdqa [rcx + 0x240], xmm10",
        "movdqa [rcx + 0x250], xmm11",
        "movdqa [rcx + 0x260], xmm12",
        "movdqa [rcx + 0x270], xmm13",
        "movdqa [rcx + 0x280], xmm14",
        "movdqa [rcx + 0x290], xmm15",
        "mov dword ptr [rcx + 0x30], 0x10000f",
        "mov rax, [rcx + 0x78]",
        "ret",
    );
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use super::context;

    /// ARM64's CONTEXT.
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    pub struct Context {
        pub flags: u32,
        pub cpsr: u32,
        /// X0 to X28, Fp (x29) and Lr (x30).
        pub x: [u64; 31],
        pub sp: u64,
        pub pc: u64,
        pub v: [u128; 32],
        pub fpcr: u32,
        pub fpsr: u32,
        pub debug: [u8; 0x390 - 0x318],
    }

    const _: () = assert!(size_of::<Context>() == context::ARM64_SIZE as usize);
    const _: () = assert!(core::mem::offset_of!(Context, x) == context::ARM64_X as usize);
    const _: () = assert!(core::mem::offset_of!(Context, sp) == context::ARM64_SP as usize);
    const _: () = assert!(core::mem::offset_of!(Context, v) == context::ARM64_V as usize);

    pub const FP: usize = 29;
    pub const LR: usize = 30;
    pub const RESULT: usize = 0;

    impl Context {
        pub fn sp(&self) -> u64 {
            self.sp
        }
    }

    unsafe extern "system" {
        /// Fills a CONTEXT with its caller's registers, as they'll be when it returns.
        pub fn RtlCaptureContext(context: *mut Context);
    }

    core::arch::global_asm!(
        ".globl RtlCaptureContext",
        ".p2align 2",
        "RtlCaptureContext:",
        "stp x0, x1, [x0, #0x08]",
        "stp x2, x3, [x0, #0x18]",
        "stp x4, x5, [x0, #0x28]",
        "stp x6, x7, [x0, #0x38]",
        "stp x8, x9, [x0, #0x48]",
        "stp x10, x11, [x0, #0x58]",
        "stp x12, x13, [x0, #0x68]",
        "stp x14, x15, [x0, #0x78]",
        "stp x16, x17, [x0, #0x88]",
        "stp x18, x19, [x0, #0x98]",
        "stp x20, x21, [x0, #0xa8]",
        "stp x22, x23, [x0, #0xb8]",
        "stp x24, x25, [x0, #0xc8]",
        "stp x26, x27, [x0, #0xd8]",
        "stp x28, x29, [x0, #0xe8]",
        "str x30, [x0, #0xf8]",
        "mov x9, sp",
        "str x9, [x0, #0x100]",
        "str x30, [x0, #0x108]",
        "mrs x9, nzcv",
        "str w9, [x0, #4]",
        "stp q0, q1, [x0, #0x110]",
        "stp q2, q3, [x0, #0x130]",
        "stp q4, q5, [x0, #0x150]",
        "stp q6, q7, [x0, #0x170]",
        "stp q8, q9, [x0, #0x190]",
        "stp q10, q11, [x0, #0x1b0]",
        "stp q12, q13, [x0, #0x1d0]",
        "stp q14, q15, [x0, #0x1f0]",
        "stp q16, q17, [x0, #0x210]",
        "stp q18, q19, [x0, #0x230]",
        "stp q20, q21, [x0, #0x250]",
        "stp q22, q23, [x0, #0x270]",
        "stp q24, q25, [x0, #0x290]",
        "stp q26, q27, [x0, #0x2b0]",
        "stp q28, q29, [x0, #0x2d0]",
        "stp q30, q31, [x0, #0x2f0]",
        "mrs x9, fpcr",
        "str w9, [x0, #0x310]",
        "mrs x9, fpsr",
        "str w9, [x0, #0x314]",
        "mov w9, #0x7",
        "movk w9, #0x40, lsl #16",
        "str w9, [x0]",
        "ldr x9, [x0, #0x50]",
        "ret",
    );
}

pub use imp::*;

/// A context, zeroed.
pub fn empty() -> Context {
    // SAFETY: every field is plain integers, for which zero is a value.
    unsafe { core::mem::zeroed() }
}
