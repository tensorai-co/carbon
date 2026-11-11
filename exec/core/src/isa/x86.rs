//! x86 and x86-64, as a program sees them.

use super::Trap;
use crate::Stop;

/// A program's general registers. `gpr` is in encoding order: rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct X86Regs {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
}

pub const RAX: usize = 0;
pub const RCX: usize = 1;
pub const RDX: usize = 2;
pub const RBX: usize = 3;
pub const RSP: usize = 4;
pub const RBP: usize = 5;
pub const RSI: usize = 6;
pub const RDI: usize = 7;
pub const R8: usize = 8;
pub const R9: usize = 9;
pub const R10: usize = 10;
pub const R11: usize = 11;

/// The flags a program can change: CF, PF, AF, ZF, SF, DF, OF.
pub const USER_FLAGS: u64 = 0xcd5;
/// IF, and the bit that's always set.
pub const RFLAGS_DEFAULT: u64 = 0x202;

impl X86Regs {
    pub fn rsp(&self) -> u64 {
        self.gpr[RSP]
    }
}

/// A program's x87, MMX and SSE state, as `fxsave` lays it out (the first 512 bytes of `xsave`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X86Fp {
    pub fxsave: [u8; 512],
}

impl Default for X86Fp {
    /// What `fninit` and a reset SSE leave, and what a new thread has on Linux: x87 at extended precision and SSE,
    /// rounding to nearest with every exception masked. (Windows starts x87 at double precision, 0x27F: atlas sets
    /// that itself.)
    fn default() -> X86Fp {
        let mut fxsave = [0u8; 512];
        fxsave[0..2].copy_from_slice(&0x37fu16.to_le_bytes());
        fxsave[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
        fxsave[28..32].copy_from_slice(&0xffffu32.to_le_bytes());
        X86Fp { fxsave }
    }
}

impl X86Fp {
    pub fn mxcsr(&self) -> u32 {
        u32::from_le_bytes(self.fxsave[24..28].try_into().unwrap_or_default())
    }

    pub fn xmm(&self, index: usize) -> u128 {
        let at = 160 + index * 16;
        u128::from_le_bytes(self.fxsave[at..at + 16].try_into().unwrap_or_default())
    }

    pub fn set_xmm(&mut self, index: usize, value: u128) {
        let at = 160 + index * 16;
        self.fxsave[at..at + 16].copy_from_slice(&value.to_le_bytes());
    }
}

/// A system call, from the registers at the trap.
pub fn syscall(trap: Trap, regs: &X86Regs) -> Stop {
    let r = &regs.gpr;
    let (nr, args) = match trap {
        Trap::Syscall => (r[RAX], [r[RDI], r[RSI], r[RDX], r[R10], r[R8], r[R9]]),
        Trap::Int2e => (r[RAX], [r[RBX], r[RCX], r[RDX], r[RSI], r[RDI], r[RBP]]),
        Trap::Int80 => (r[RAX] & 0xffff_ffff, [r[RBX], r[RCX], r[RDX], r[RSI], r[RDI], r[RBP]].map(|v| v & 0xffff_ffff)),
        Trap::Svc => return Stop::Internal { pc: regs.rip, detail: 0x100 },
    };
    Stop::Syscall { nr, args }
}

/// Where a system call's result goes.
pub fn set_result(regs: &mut X86Regs, value: u64) {
    regs.gpr[RAX] = value;
}

/// Interrupt vectors, and what each means to a kernel. `error` is the CPU's error code, `cr2` the faulting address.
pub fn exception(vector: u8, pc: u64, error: u64, cr2: u64, regs: &X86Regs) -> Stop {
    use exec_mmu::Prot;
    match vector {
        0 => Stop::DivideError { pc },
        3 => Stop::Breakpoint { pc: pc.wrapping_sub(1) },
        13 => Stop::Fault { addr: u64::MAX, access: Prot::R, pc },
        14 => {
            let access = if error & 16 != 0 {
                Prot::X
            } else if error & 2 != 0 {
                Prot::W
            } else {
                Prot::R
            };
            Stop::Fault { addr: cr2, access, pc }
        }
        17 => Stop::Alignment { pc },
        0x29 => Stop::FastFail { pc: pc.wrapping_sub(2), code: regs.gpr[RCX] },
        0x2e => syscall(Trap::Int2e, regs),
        0x80 => syscall(Trap::Int80, regs),
        _ => Stop::Undefined { pc },
    }
}

/// Whether the CPU pushes an error code for this vector.
pub const fn has_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30)
}

/// Vectors a program may raise itself with `int n`.
pub const fn user_vector(vector: u8) -> bool {
    matches!(vector, 3 | 4 | 0x29 | 0x2e | 0x80)
}
