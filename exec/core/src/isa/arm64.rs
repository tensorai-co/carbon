//! ARM64, as a program sees it, and the exception syndromes that say why it stopped.

use exec_mmu::Prot;

use super::Trap;
use crate::Stop;

/// A program's general registers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Arm64Regs {
    pub x: [u64; 31],
    pub sp: u64,
    pub pc: u64,
    /// Only N, Z, C and V: a program always stays at EL0.
    pub nzcv: u64,
    /// TPIDR_EL0, the thread pointer.
    pub tpidr: u64,
}

/// A program's SIMD and floating-point registers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Arm64Fp {
    pub v: [u128; 32],
    pub fpsr: u32,
    pub fpcr: u32,
}

pub const NZCV: u64 = 0xf000_0000;

/// ESR_ELx exception classes.
pub const EC_UNKNOWN: u64 = 0x00;
pub const EC_SVE_FP: u64 = 0x07;
pub const EC_SVC: u64 = 0x15;
pub const EC_HVC: u64 = 0x16;
pub const EC_SYSREG: u64 = 0x18;
pub const EC_IABORT_LOWER: u64 = 0x20;
pub const EC_PC_ALIGNMENT: u64 = 0x22;
pub const EC_DABORT_LOWER: u64 = 0x24;
pub const EC_SP_ALIGNMENT: u64 = 0x26;
pub const EC_BRK: u64 = 0x3c;

const DFSC_MASK: u64 = 0x3f;
const DFSC_ALIGNMENT: u64 = 0x21;
const WNR: u64 = 1 << 6;
const CM: u64 = 1 << 8;

pub fn syscall(trap: Trap, regs: &Arm64Regs) -> Stop {
    if trap != Trap::Svc {
        return Stop::Internal { pc: regs.pc, detail: 0x101 };
    }
    let mut args = [0; 6];
    args.copy_from_slice(&regs.x[..6]);
    Stop::Syscall { nr: regs.x[8], args }
}

pub fn set_result(regs: &mut Arm64Regs, value: u64) {
    regs.x[0] = value;
}

/// What an exception from EL0 means, from its syndrome and faulting address. `pc` is where the program was.
pub fn exception(esr: u64, far: u64, pc: u64, regs: &Arm64Regs) -> Stop {
    match esr >> 26 & 0x3f {
        EC_SVC => syscall(Trap::Svc, regs),
        EC_DABORT_LOWER if esr & DFSC_MASK == DFSC_ALIGNMENT => Stop::Alignment { pc },
        EC_DABORT_LOWER => {
            // Cache maintenance sets WnR whatever it does, and says so in CM; it only needs read access.
            let access = if esr & WNR != 0 && esr & CM == 0 { Prot::W } else { Prot::R };
            Stop::Fault { addr: far, access, pc }
        }
        EC_IABORT_LOWER => Stop::Fault { addr: far, access: Prot::X, pc },
        EC_PC_ALIGNMENT | EC_SP_ALIGNMENT => Stop::Alignment { pc },
        EC_UNKNOWN | EC_SYSREG | EC_SVE_FP => Stop::Undefined { pc },
        EC_BRK => Stop::Breakpoint { pc },
        _ => Stop::Internal { pc, detail: esr },
    }
}
