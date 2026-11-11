//! Registers as the program's own CPU has them, whatever runs it.

use crate::Isa;
use crate::isa::arm64::{Arm64Fp, Arm64Regs};
use crate::isa::x86::{RSP, X86Fp, X86Regs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regs {
    X86(X86Regs),
    Arm64(Arm64Regs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpRegs {
    X86(X86Fp),
    Arm64(Arm64Fp),
}

impl Regs {
    /// A fresh thread's, for this instruction set.
    pub fn new(isa: Isa) -> Regs {
        if isa.is_x86() {
            Regs::X86(X86Regs { rflags: crate::isa::x86::RFLAGS_DEFAULT, ..X86Regs::default() })
        } else {
            Regs::Arm64(Arm64Regs::default())
        }
    }

    pub fn pc(&self) -> u64 {
        match self {
            Regs::X86(r) => r.rip,
            Regs::Arm64(r) => r.pc,
        }
    }

    pub fn sp(&self) -> u64 {
        match self {
            Regs::X86(r) => r.gpr[RSP],
            Regs::Arm64(r) => r.sp,
        }
    }

    pub fn set_pc(&mut self, pc: u64) {
        match self {
            Regs::X86(r) => r.rip = pc,
            Regs::Arm64(r) => r.pc = pc,
        }
    }

    pub fn set_sp(&mut self, sp: u64) {
        match self {
            Regs::X86(r) => r.gpr[RSP] = sp,
            Regs::Arm64(r) => r.sp = sp,
        }
    }

    pub fn set_result(&mut self, value: u64) {
        match self {
            Regs::X86(r) => crate::isa::x86::set_result(r, value),
            Regs::Arm64(r) => crate::isa::arm64::set_result(r, value),
        }
    }
}

impl FpRegs {
    pub fn new(isa: Isa) -> FpRegs {
        if isa.is_x86() { FpRegs::X86(X86Fp::default()) } else { FpRegs::Arm64(Arm64Fp::default()) }
    }
}

/// A thread's whole state, to resume it later on any CPU.
#[derive(Debug, Clone)]
pub struct Context {
    pub(crate) regs: Regs,
    pub(crate) fp: FpRegs,
}
