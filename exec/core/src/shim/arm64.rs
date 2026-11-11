//! The ARM64 shim, for any hypervisor that runs ARM64 guests (hvf; later kvm on ARM64 hosts).
//!
//! The program runs at EL0 in TTBR0's half. The shim is one page of exception vectors at the top of TTBR1's half: each
//! vector makes a hypercall, then `eret`s. A program's pc, pstate and stack are in ELR_EL1, SPSR_EL1 and SP_EL0 while
//! it's stopped, which is where core reads and changes them.

use exec_mmu::arm64::{ACCESSED, AP_READ_ONLY, SHAREABLE_INNER, UXN, VALID};
use exec_mmu::{Frame, Mmu, Native, OutOfMemory};

use super::Shim;
use crate::isa::arm64::{self as isa, Arm64Fp, Arm64Regs, NZCV};
use crate::{FpRegs, Isa, Options, Regs, Stop};

/// The page of vectors, at the top of the address space.
pub const VECTORS: u64 = 0xffff_ffff_ffff_0000;
/// How far apart vectors are.
pub const VECTOR_SIZE: u64 = 0x80;
/// The `eret` after the vector for a synchronous exception from EL0, where every CPU goes back to its program.
pub const ERET_AT: u64 = VECTORS + 0x404;
/// The same, invalidating this CPU's TLB first: `tlbi vmalle1; dsb ish; isb; eret`.
pub const FLUSH_AT: u64 = VECTORS + 0x800;

/// The hypercall a synchronous exception from EL0 makes.
pub const CALL_FROM_EL0: u64 = 1;

/// EL1, on its own stack, with every interrupt masked.
pub const EL1H_MASKED: u64 = 0x3c5;

const fn hvc(number: u32) -> u32 {
    0xd400_0002 | number << 5
}
const ERET: u32 = 0xd69f_03e0;
const TLBI_VMALLE1: u32 = 0xd508_871f;
const DSB_ISH: u32 = 0xd503_3b9f;
const ISB: u32 = 0xd503_3fdf;

/// A vCPU's registers, as the backend moves them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Arm64VcpuRegs {
    pub x: [u64; 31],
    pub pc: u64,
    pub cpsr: u64,
    pub sp_el0: u64,
    pub elr: u64,
    pub spsr: u64,
    pub esr: u64,
    pub far: u64,
    pub tpidr: u64,
}

/// A CPU's system registers, set once.
#[derive(Debug, Clone)]
pub struct Arm64System {
    pub mair: u64,
    pub tcr: u64,
    pub ttbr1: u64,
    pub vbar: u64,
    pub cpacr: u64,
    pub cntkctl: u64,
    pub sctlr: u64,
    /// Only EnTSO (bit 1), where the hypervisor allows it.
    pub actlr: u64,
}

pub struct Arm64Shim {
    vectors: Frame,
    kernel_root: u64,
    empty_root: u64,
    tso: bool,
}

const TCR: u64 = 16 // T0SZ: 48 bits
    | 1 << 8 | 1 << 10 | 3 << 12 // write-back, inner shareable walks
    | 16 << 16 // T1SZ: 48 bits
    | 1 << 24 | 1 << 26 | 3 << 28
    | 2 << 30 // TG1: 4 KiB; TG0's is 0
    | 1 << 32 // IPS: 36 bits
    | 1 << 37; // TBI0: programs may tag pointers

const SCTLR: u64 = 0x30d0_0800 // RES1
    | 1 // M: the MMU
    | 1 << 2 // C
    | 1 << 4 // SA0: EL0's stack must be aligned
    | 1 << 6 // nAA: atomics needn't be aligned within 16 bytes, as x86's needn't
    | 1 << 12 // I
    | 1 << 14 // DZE: `dc zva` at EL0
    | 1 << 15 // UCT: CTR_EL0 at EL0
    | 1 << 26; // UCI: cache maintenance at EL0

impl Shim for Arm64Shim {
    const ISAS: &'static [Isa] = &[Isa::Arm64];
    const MAX_CPUS: usize = 256;
    type Regs = Arm64VcpuRegs;
    type Fp = Arm64Fp;
    type System = Arm64System;
    type CpuData = ();

    fn install(mmu: &mut Mmu<Native>, options: &Options) -> Result<Arm64Shim, OutOfMemory> {
        let vectors = mmu.new_frame()?;
        // 16 vectors of 32 words: from EL1, which would be a bug in this page, then from EL0.
        let mut words = [0u32; 1024];
        for vector in 0..16 {
            let number = match vector {
                8 => CALL_FROM_EL0 as u32,
                9..=11 => vector as u32 - 7,
                other => 0x100 + other as u32,
            };
            words[vector * 32] = hvc(number);
            words[vector * 32 + 1] = ERET;
        }
        words[512..516].copy_from_slice(&[TLBI_VMALLE1, DSB_ISH, ISB, ERET]);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        mmu.frame_write(vectors, 0, &bytes);

        // TTBR1's root holds only the vectors: EL1 can run them, read-only, and the program can't touch them.
        let kernel_root = mmu.new_frame()?.0;
        let entry = mmu.entry(kernel_root, VECTORS, true)?.ok_or(OutOfMemory)?;
        // SAFETY: an entry in a table in guest memory.
        unsafe { entry.write_volatile(vectors.0 | VALID | SHAREABLE_INNER | ACCESSED | AP_READ_ONLY | UXN) };
        // TTBR0 for a CPU with no space: nothing at all.
        let empty_root = mmu.new_frame()?.0;
        Ok(Arm64Shim { vectors, kernel_root, empty_root, tso: options.tso })
    }

    fn code_frames(&self) -> Vec<Frame> {
        vec![self.vectors]
    }

    fn add_cpu(&self, _mmu: &mut Mmu<Native>, index: usize) -> Result<(Arm64System, Arm64VcpuRegs, ()), OutOfMemory> {
        if index >= Self::MAX_CPUS {
            return Err(OutOfMemory);
        }
        let system = Arm64System {
            mair: 0xff, // attribute 0: ordinary write-back memory
            tcr: TCR,
            ttbr1: self.kernel_root,
            vbar: VECTORS,
            cpacr: 3 << 20, // FPEN: floating point at EL0 and EL1
            cntkctl: 3,     // the counters at EL0
            sctlr: SCTLR,
            actlr: if self.tso { 1 << 1 } else { 0 },
        };
        let regs = Arm64VcpuRegs { pc: ERET_AT, cpsr: EL1H_MASKED, ..Arm64VcpuRegs::default() };
        Ok((system, regs, ()))
    }

    fn root_register(&self, root: u64) -> u64 {
        if root == 0 { self.empty_root } else { root }
    }

    fn before_run(&self, regs: &mut Arm64VcpuRegs, flush: bool) {
        if flush && regs.pc == ERET_AT {
            regs.pc = FLUSH_AT;
        }
    }

    fn trap(&self, _mmu: &Mmu<Native>, _cpu: &(), regs: &mut Arm64VcpuRegs, at: u64) -> Stop {
        if at != CALL_FROM_EL0 {
            return Stop::Internal { pc: regs.elr, detail: 0x2000 | at };
        }
        // The hypercall returns to the `eret` after it, so it's already where it goes back from.
        let Regs::Arm64(program) = self.program(_mmu, &(), regs) else { unreachable!() };
        isa::exception(regs.esr, regs.far, regs.elr, &program)
    }

    fn preempted(&self, _mmu: &Mmu<Native>, _cpu: &(), regs: &mut Arm64VcpuRegs) -> Stop {
        // In the program: as if it had taken an exception, so it goes back by `eret`.
        if regs.cpsr & 0xf == 0 {
            regs.elr = regs.pc;
            regs.spsr = regs.cpsr;
            regs.pc = ERET_AT;
            regs.cpsr = EL1H_MASKED;
            return Stop::Preempted;
        }
        match regs.pc {
            ERET_AT => Stop::Preempted,
            // Part way through the flush, which can start again.
            pc if (FLUSH_AT..FLUSH_AT + 16).contains(&pc) => {
                regs.pc = FLUSH_AT;
                Stop::Preempted
            }
            pc => Stop::Internal { pc, detail: 0x2100 },
        }
    }

    fn program(&self, _mmu: &Mmu<Native>, _cpu: &(), regs: &Arm64VcpuRegs) -> Regs {
        Regs::Arm64(Arm64Regs { x: regs.x, sp: regs.sp_el0, pc: regs.elr, nzcv: regs.spsr & NZCV, tpidr: regs.tpidr })
    }

    fn set_program(&self, _mmu: &Mmu<Native>, _cpu: &(), regs: &mut Arm64VcpuRegs, program: &Regs) {
        let Regs::Arm64(program) = program else { return };
        regs.x = program.x;
        regs.sp_el0 = program.sp;
        regs.elr = program.pc;
        regs.spsr = (regs.spsr & !NZCV) | (program.nzcv & NZCV);
        regs.tpidr = program.tpidr;
    }

    fn start(&self, _mmu: &Mmu<Native>, _cpu: &(), regs: &mut Arm64VcpuRegs, _isa: Isa, entry: u64, stack: u64) {
        *regs = Arm64VcpuRegs { pc: ERET_AT, cpsr: EL1H_MASKED, elr: entry, sp_el0: stack, spsr: 0, ..Arm64VcpuRegs::default() };
    }

    fn set_return(&self, regs: &mut Arm64VcpuRegs, value: u64) {
        regs.x[0] = value;
    }

    fn program_fp(fp: &Arm64Fp) -> FpRegs {
        FpRegs::Arm64(*fp)
    }

    fn set_program_fp(fp: &mut Arm64Fp, program: &FpRegs) {
        if let FpRegs::Arm64(program) = program {
            *fp = *program;
        }
    }

    fn arm64(regs: &mut Arm64VcpuRegs) -> Option<&mut Arm64VcpuRegs> {
        Some(regs)
    }

    fn arm64_fp(fp: &mut Arm64Fp) -> Option<&mut Arm64Fp> {
        Some(fp)
    }
}
