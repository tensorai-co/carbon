//! x86-64 → ARM64, a block at a time.
//!
//! # Where x86 lives
//!
//! The 16 general registers are `x0`–`x15` in encoding order (`rax` is `x0`, `rsp` is `x4`). The fs and gs bases are
//! `x28` and `x27`. `xmm0`–`xmm15` are `v0`–`v15`. `x16` carries a block exit's target and `x17` a memory operand's
//! address; `x19`–`x26` and `v28`–`v31` are scratch. x87's stack is `v16`–`v23` and its control and status words
//! `v24` (`x87.rs`). Nothing else is touched.
//!
//! # Flags
//!
//! SF, ZF and OF are ARM's N, Z and V. ARM's C is always the *inverse* of x86's CF: that's what ARM's `subs` leaves,
//! so `sub` and `cmp` need nothing, every unsigned condition maps to one ARM condition, and only `add` and the logic
//! instructions need a `cfinv`. AF isn't kept. PF is kept only where compilers test it, after a floating-point
//! compare (`ucomisd` and its kin), which leaves OF clear: there it's in V, so `jp`, `setp` and `cmovp` read V. After
//! an integer instruction they read OF instead, which compiled code doesn't do.
//!
//! # Leaving a block
//!
//! Every way out is `brk #kind`, which the runtime turns into a translation, a system call or a stop. Most never get
//! there: a jump to an address known now is `CHAIN`, with the address after the `brk`, and once its target is
//! translated the runtime rewrites the `brk` into a branch straight to it. An indirect jump or a return (target in
//! `x16`) first looks itself up in the table after the code cache, and branches to the translation it finds there;
//! only a miss leaves by `brk #JUMP`, after which the runtime fills that entry.
//!
//! What isn't covered becomes an `UNSUPPORTED` exit at that instruction, which reaches the kernel as an undefined
//! instruction: nothing is ever run wrong silently.

use alloc::vec::Vec;

use super::asm::{Asm, Cond, Shift, ZR};
use crate::Error;
use crate::decode::x86::{self, Base, Inst, Mem, Rm, Seg};

#[path = "sse.rs"]
mod sse;
#[path = "x87.rs"]
mod x87;

/// What `brk`'s immediate says about a block exit. `x16` is the x86 address to carry on from.
pub mod exit {
    /// To the next block.
    pub const JUMP: u16 = 1;
    pub const SYSCALL: u16 = 2;
    /// An instruction the translator doesn't cover, at `x16`.
    pub const UNSUPPORTED: u16 = 3;
    /// `int3`; `x16` is past it.
    pub const BREAKPOINT: u16 = 4;
    /// `#DE` at `x16`.
    pub const DIVIDE: u16 = 5;
    /// Code that can't be fetched: `x16` is the address, `x17` the instruction it's part of.
    pub const FETCH: u16 = 6;
    /// `cpuid`, for the runtime to answer; `x16` is past it.
    pub const CPUID: u16 = 7;
    pub const INT80: u16 = 8;
    pub const INT2E: u16 = 9;
    /// `int 0x29` at `x16`.
    pub const FASTFAIL: u16 = 10;
    /// To a block known when translating, whose address is the 8 bytes after the `brk` (`x16` isn't set). Once
    /// that block is translated, the runtime turns the `brk` into a branch to it: the blocks are chained.
    pub const CHAIN: u16 = 11;
}

/// Where a translated space's code cache is, at the top of its own address space, mapped for EL0 to run.
pub const CACHE_BASE: u64 = 0x7f00_0000_0000;
pub const CACHE_SIZE: u64 = 1 << 30;
/// After it, the table indirect jumps look themselves up in: `1 << LOOKUP_BITS` entries of 8 bytes, indexed by an
/// x86 address's low bits, mapped for EL0 to read. An entry is `lookup_entry`'s, or 0.
pub const LOOKUP_BASE: u64 = CACHE_BASE + CACHE_SIZE;
pub const LOOKUP_BITS: u32 = 14;
pub const LOOKUP_BYTES: u64 = 8 << LOOKUP_BITS;
/// The end of everything a translated space keeps for itself.
pub const REGION_END: u64 = LOOKUP_BASE + LOOKUP_BYTES;

/// The lookup table's entry for the block at x86 address `pc`, translated at `at`: where it goes in the table
/// (an index), and what goes there. One word, so a CPU reading it as it changes sees all of the old or the new: the
/// translation's offset in the cache in words (28 bits), and above it the rest of `pc` past the index, plus one, so
/// an empty entry matches nothing.
pub fn lookup_entry(pc: u64, at: u64) -> (usize, u64) {
    let index = (pc & ((1 << LOOKUP_BITS) - 1)) as usize;
    let tag = (pc >> LOOKUP_BITS) + 1;
    (index, tag << 28 | (at - CACHE_BASE) >> 2)
}

/// The branch that chains a `CHAIN` exit at `site` to a translation at `at`, if it's in a branch's reach.
pub fn chain_branch(site: u64, at: u64) -> Option<u32> {
    let words = (at as i64 - site as i64) >> 2;
    (-(1 << 25)..(1 << 25)).contains(&words).then_some(0x1400_0000 | (words as u32 & 0x03FF_FFFF))
}

/// A `CHAIN` exit's first word, the `brk` that `chain_branch` replaces: written back, it unlinks the exit. The target
/// after it is never touched, so this is all it takes.
pub const CHAIN_EXIT: u32 = 0xD420_0000 | (exit::CHAIN as u32) << 5;

pub const FS_BASE: u8 = 28;
pub const GS_BASE: u8 = 27;
const T: u8 = 16;
const A: u8 = 17;
const S0: u8 = 19;
const S1: u8 = 20;
const R: u8 = 21;
const S2: u8 = 22;
const F0: u8 = 23;
const F1: u8 = 24;
const F2: u8 = 25;
const S3: u8 = 26;
const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const RSP: u8 = 4;
const RBP: u8 = 5;
const RSI: u8 = 6;
const RDI: u8 = 7;
const V_TMP: u8 = 31;

/// A block's translation.
#[derive(Debug, Clone, Default)]
pub struct Translation {
    pub code: Vec<u8>,
    /// (byte offset in `code`, x86 address) for each instruction.
    pub map: Vec<(u32, u64)>,
    /// False if it ends on code it couldn't fetch, which may be there next time.
    pub cacheable: bool,
    /// Just past the last x86 byte it was translated from: changing any byte in `start..end` makes it wrong.
    pub end: u64,
}

/// Translates the block at `start`, whose bytes as far as they can be read are `bytes`, up to `most` instructions.
pub fn translate(start: u64, bytes: &[u8], most: usize) -> Translation {
    let mut e = Emitter { asm: Asm::new() };
    let mut map = Vec::new();
    let mut pc = start;
    let mut cacheable = true;
    let mut end = start;
    for count in 0.. {
        let offset = (e.asm.len() * 4) as u32;
        if count == most {
            e.jump(pc);
            break;
        }
        map.push((offset, pc));
        let inst = match x86::decode(pc, &bytes[(pc - start) as usize..]) {
            Ok(inst) => inst,
            Err(Error::Truncated(missing)) => {
                e.asm.mov_imm(A, pc);
                e.exit(exit::FETCH, missing);
                cacheable = false;
                break;
            }
            Err(_) => {
                // What follows could still decide what it is: as far as an instruction can be.
                end = end.max(pc + 15);
                e.exit(exit::UNSUPPORTED, pc);
                break;
            }
        };
        end = end.max(inst.next());
        let mark = e.asm.len();
        match e.lower(&inst) {
            Ok(Next::Continue) => pc = inst.next(),
            Ok(Next::End) => break,
            Err(Unsupported) => {
                e.asm.truncate(mark);
                e.exit(exit::UNSUPPORTED, pc);
                break;
            }
        }
    }
    Translation { code: e.asm.code.iter().flat_map(|w| w.to_le_bytes()).collect(), map, cacheable, end }
}

enum Next {
    Continue,
    End,
}

struct Unsupported;

type Lowered = Result<Next, Unsupported>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alu {
    Add,
    /// With carry in: 32 and 64 bits only.
    Adc,
    Sbb,
    Or,
    And,
    Sub,
    Xor,
    Cmp,
    Test,
}

/// An operand, once decoded.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// A register, and whether it's the high byte (ah, ch, dh, bh).
    Reg(u8, bool),
    /// A memory operand, whose address is in `x17`.
    Mem,
    Imm(i64),
}

fn mask(size: u8) -> u64 {
    if size == 64 { u64::MAX } else { (1u64 << size) - 1 }
}

/// x86's condition codes, under the inverted-carry rule. Parity is V, as a floating-point compare leaves it.
fn cond(cc: u8) -> Option<Cond> {
    Some(match cc & 15 {
        10 => Cond::Vs,
        11 => Cond::Vc,
        0 => Cond::Vs,
        1 => Cond::Vc,
        2 => Cond::Cc,
        3 => Cond::Cs,
        4 => Cond::Eq,
        5 => Cond::Ne,
        6 => Cond::Ls,
        7 => Cond::Hi,
        8 => Cond::Mi,
        9 => Cond::Pl,
        12 => Cond::Lt,
        13 => Cond::Ge,
        14 => Cond::Le,
        15 => Cond::Gt,
        _ => return None,
    })
}

struct Emitter {
    asm: Asm,
}

impl Emitter {
    fn exit(&mut self, kind: u16, target: u64) {
        self.asm.mov_imm(T, target);
        self.asm.brk(kind);
    }

    /// On to x86 address `target`, by an exit the runtime chains to its block once it's translated.
    fn jump(&mut self, target: u64) {
        self.asm.brk(exit::CHAIN);
        self.asm.put(target as u32);
        self.asm.put((target >> 32) as u32);
    }

    /// On to the x86 address in `x16`: straight to its translation if the lookup table has it, else by an exit, after
    /// which the runtime puts it in the table. The flags are kept.
    fn jump_indirect(&mut self) {
        let a = &mut self.asm;
        a.ubfx(true, S0, T, 0, LOOKUP_BITS as u8);
        a.mov_imm(S1, LOOKUP_BASE);
        a.ldr_index(S2, S1, S0);
        a.lsr_imm(true, S3, T, LOOKUP_BITS as u8);
        a.add_imm(true, S3, S3, 1);
        a.sub_shift(S3, S3, S2, Shift::Lsr, 28);
        let miss = a.len();
        a.cbnz(true, S3, 0);
        a.ubfx(true, S2, S2, 0, 28);
        a.mov_imm(S1, CACHE_BASE);
        a.add_shift(true, S1, S1, S2, Shift::Lsl, 2);
        a.br(S1);
        let here = a.len();
        a.patch(miss, here);
        a.brk(exit::JUMP);
    }

    /// Leaves the block if a branch emitted by `branch` (pointed past the exit) isn't taken.
    fn exit_unless(&mut self, branch: impl FnOnce(&mut Asm), kind: u16, target: u64) {
        let at = self.asm.len();
        branch(&mut self.asm);
        self.exit(kind, target);
        let here = self.asm.len();
        self.asm.patch(at, here);
    }

    // ---- Operands ----

    fn reg(&self, inst: &Inst, num: u8, size: u8) -> Op {
        if size == 8 && inst.rex == 0 && (4..8).contains(&num) { Op::Reg(num - 4, true) } else { Op::Reg(num, false) }
    }

    /// ModRM's r/m operand; a memory one has its address put in `x17` now.
    fn rm(&mut self, inst: &Inst, size: u8) -> Result<Op, Unsupported> {
        match inst.rm {
            Some(Rm::Reg(num)) => Ok(self.reg(inst, num, size)),
            Some(Rm::Mem(mem)) => {
                self.address(&mem, inst.next(), true);
                Ok(Op::Mem)
            }
            None => Err(Unsupported),
        }
    }

    /// A memory operand's address, into `x17`. `lea` leaves the segment out.
    fn address(&mut self, mem: &Mem, next: u64, segment: bool) {
        let a = &mut self.asm;
        match mem.base {
            Some(Base::Rip) => a.mov_imm(A, next.wrapping_add(mem.disp as u64)),
            Some(Base::Reg(base)) => a.mov(true, A, base),
            None => a.mov_imm(A, 0),
        }
        if let Some((index, scale)) = mem.index {
            a.add_shift(true, A, A, index, Shift::Lsl, scale);
        }
        if mem.base != Some(Base::Rip) && mem.disp != 0 {
            if (1..4096).contains(&mem.disp) {
                a.add_imm(true, A, A, mem.disp as u16);
            } else if (-4095..0).contains(&mem.disp) {
                a.sub_imm(true, A, A, (-mem.disp) as u16);
            } else {
                a.mov_imm(T, mem.disp as u64);
                a.add(true, A, A, T);
            }
        }
        if mem.addr32 {
            a.mov(false, A, A);
        }
        if segment {
            match mem.seg {
                Some(Seg::Fs) => a.add(true, A, A, FS_BASE),
                Some(Seg::Gs) => a.add(true, A, A, GS_BASE),
                None => {}
            }
        }
    }

    /// An operand's value, zero-extended, into `dst`.
    fn load(&mut self, op: Op, size: u8, dst: u8) {
        let a = &mut self.asm;
        match op {
            Op::Reg(num, high) => match size {
                64 => a.mov(true, dst, num),
                32 => a.mov(false, dst, num),
                16 => a.ubfx(true, dst, num, 0, 16),
                _ => a.ubfx(true, dst, num, if high { 8 } else { 0 }, 8),
            },
            Op::Mem => a.ldr(size, dst, A),
            Op::Imm(value) => a.mov_imm(dst, value as u64 & mask(size)),
        }
    }

    /// `src`'s low `size` bits into an operand, as x86 writes them: 32-bit register writes clear the top half, 8- and
    /// 16-bit ones keep the rest.
    fn store(&mut self, op: Op, size: u8, src: u8) -> Result<(), Unsupported> {
        let a = &mut self.asm;
        match op {
            Op::Reg(num, high) => match size {
                64 => a.mov(true, num, src),
                32 => a.mov(false, num, src),
                16 => a.bfi(true, num, src, 0, 16),
                _ => a.bfi(true, num, src, if high { 8 } else { 0 }, 8),
            },
            Op::Mem => a.str(size, src, A),
            Op::Imm(_) => return Err(Unsupported),
        }
        Ok(())
    }

    // ---- Flags ----

    /// `R = a op b`, with x86's flags. 8- and 16-bit operations shift both sides to the top of a 32-bit register first,
    /// so N, Z, C and V come out as they would at their own size.
    fn alu(&mut self, kind: Alu, size: u8, a: u8, b: u8) {
        if size < 32 && matches!(kind, Alu::Adc | Alu::Sbb) {
            return self.alu_carry_small(kind, size, a, b);
        }
        let sf = size == 64;
        let (x, y) = if size < 32 {
            let shift = 32 - size;
            self.asm.lsl_imm(false, S2, a, shift);
            self.asm.lsl_imm(false, S3, b, shift);
            (S2, S3)
        } else {
            (a, b)
        };
        let asm = &mut self.asm;
        match kind {
            Alu::Add => asm.adds(sf, R, x, y),
            // ARM's carry is x86's CF inverted: adc wants it the right way round for `adcs`; sbb wants it as it is,
            // as `sbcs` subtracts !C, and leaves C inverted from x86's borrow, as the rule has it.
            Alu::Adc => {
                asm.cfinv();
                asm.adcs(sf, R, x, y);
            }
            Alu::Sbb => asm.sbcs(sf, R, x, y),
            Alu::Sub | Alu::Cmp => asm.subs(sf, R, x, y),
            Alu::And | Alu::Test => asm.ands(sf, R, x, y),
            Alu::Or => {
                asm.orr(sf, R, x, y);
                asm.ands(sf, ZR, R, R);
            }
            Alu::Xor => {
                asm.eor(sf, R, x, y);
                asm.ands(sf, ZR, R, R);
            }
        }
        if size < 32 {
            asm.lsr_imm(false, R, R, 32 - size);
        }
        if !matches!(kind, Alu::Sub | Alu::Cmp | Alu::Sbb) {
            // ARM's carry is x86's here (add, adc) or clear (logic): either way, invert it to keep the rule.
            asm.cfinv();
        }
    }

    /// 8- and 16-bit `adc` and `sbb`, whose carry in has to go in at the operand's lowest bit, not the register's. `sbb`
    /// is `a + !b + !CF`, so both are an add of a carry `c`: with the operands at the top of a 32-bit register, the low
    /// bits of one side are all `c` and of the other just `c`, so their sum carries exactly `c` into the operand and
    /// leaves zeros, and an `adds` gives every flag as it would be at the operand's size.
    fn alu_carry_small(&mut self, kind: Alu, size: u8, a: u8, b: u8) {
        let shift = 32 - size;
        let asm = &mut self.asm;
        // x86's CF is ARM's C inverted: adc adds CF (C clear), sbb adds !CF (C set).
        asm.cset(false, F1, if kind == Alu::Adc { Cond::Cc } else { Cond::Cs });
        asm.lsl_imm(false, F2, F1, shift);
        asm.sub(false, F2, F2, F1);
        asm.lsl_imm(false, S2, a, shift);
        asm.orr(false, S2, S2, F2);
        if kind == Alu::Sbb {
            asm.orn(false, S3, ZR, b);
            asm.lsl_imm(false, S3, S3, shift);
        } else {
            asm.lsl_imm(false, S3, b, shift);
        }
        asm.orr(false, S3, S3, F1);
        asm.adds(false, R, S2, S3);
        asm.lsr_imm(false, R, R, shift);
        // ARM's carry out is x86's for adc, and its borrow inverted for sbb, which the rule wants.
        if kind == Alu::Adc {
            asm.cfinv();
        }
    }

    /// Restores CF from `saved` (an NZCV value), keeping the other flags as they are now: `inc` and `dec` leave CF.
    fn keep_carry(&mut self, saved: u8) {
        let a = &mut self.asm;
        a.mrs_nzcv(F1);
        a.lsr_imm(true, F2, saved, 29);
        a.bfi(true, F1, F2, 29, 1);
        a.msr_nzcv(F1);
    }

    /// Sets CF to `cf` (0 or 1), keeping N, Z and V.
    fn set_carry(&mut self, cf: u8) {
        let a = &mut self.asm;
        a.mrs_nzcv(F1);
        a.mov_imm(F2, 1);
        a.sub(true, F2, F2, cf);
        a.bfi(true, F1, F2, 29, 1);
        a.msr_nzcv(F1);
    }

    /// CF and OF both `ovf` (0 or 1), SF and ZF clear: what a multiply leaves.
    fn set_cf_of(&mut self, ovf: u8) {
        let a = &mut self.asm;
        a.mov_imm(F1, 1);
        a.sub(true, F1, F1, ovf);
        a.lsl_imm(true, F1, F1, 29);
        a.lsl_imm(true, F2, ovf, 28);
        a.orr(true, F1, F1, F2);
        a.msr_nzcv(F1);
    }

    /// ZF from `zf` and CF from `cf` (each 0 or 1), the rest clear.
    fn set_zf_cf(&mut self, zf: u8, cf: u8) {
        let a = &mut self.asm;
        a.mov_imm(F1, 1);
        a.sub(true, F1, F1, cf);
        a.lsl_imm(true, F1, F1, 29);
        a.lsl_imm(true, F2, zf, 30);
        a.orr(true, F1, F1, F2);
        a.msr_nzcv(F1);
    }

    // ---- The stack ----

    fn push(&mut self, value: u8) {
        self.asm.sub_imm(true, RSP, RSP, 8);
        self.asm.str(64, value, RSP);
    }

    fn pop(&mut self, into: u8) {
        self.asm.ldr(64, into, RSP);
        self.asm.add_imm(true, RSP, RSP, 8);
    }

    // ---- Instructions ----

    fn lower(&mut self, inst: &Inst) -> Lowered {
        match inst.map {
            0 => self.one_byte(inst),
            1 => self.two_byte(inst),
            _ => Err(Unsupported),
        }
    }

    fn alu_kind(n: u8) -> Option<Alu> {
        Some(match n & 7 {
            0 => Alu::Add,
            1 => Alu::Or,
            2 => Alu::Adc,
            3 => Alu::Sbb,
            4 => Alu::And,
            5 => Alu::Sub,
            6 => Alu::Xor,
            7 => Alu::Cmp,
            _ => return None,
        })
    }

    /// `dst op= src`, with its flags, atomically with `lock`.
    fn alu_inst(&mut self, inst: &Inst, kind: Alu, dst: Op, src: Op, size: u8) -> Lowered {
        if matches!(kind, Alu::Adc | Alu::Sbb) && inst.lock {
            return Err(Unsupported);
        }
        if inst.lock {
            if !matches!(dst, Op::Mem) || matches!(kind, Alu::Cmp | Alu::Test) {
                return Err(Unsupported);
            }
            self.load(src, size, S1);
            let a = &mut self.asm;
            match kind {
                Alu::Add => a.ldaddal(size, S1, S0, A),
                Alu::Sub => {
                    a.sub(true, S2, ZR, S1);
                    a.ldaddal(size, S2, S0, A);
                }
                Alu::And => {
                    a.orn(true, S2, ZR, S1);
                    a.ldclral(size, S2, S0, A);
                }
                Alu::Or => a.ldsetal(size, S1, S0, A),
                Alu::Xor => a.ldeoral(size, S1, S0, A),
                Alu::Cmp | Alu::Test | Alu::Adc | Alu::Sbb => return Err(Unsupported),
            }
            self.alu(kind, size, S0, S1);
            return Ok(Next::Continue);
        }
        self.load(dst, size, S0);
        self.load(src, size, S1);
        self.alu(kind, size, S0, S1);
        if !matches!(kind, Alu::Cmp | Alu::Test) {
            self.store(dst, size, R)?;
        }
        Ok(Next::Continue)
    }

    fn one_byte(&mut self, inst: &Inst) -> Lowered {
        let v = inst.size_v();
        let op = inst.op;
        match op {
            // add, or, adc, sbb, and, sub, xor, cmp in their six forms.
            0x00..=0x3D if op & 7 < 6 => {
                let kind = Self::alu_kind(op >> 3).ok_or(Unsupported)?;
                let size = if op & 1 == 0 { 8 } else { v };
                match op & 7 {
                    0 | 1 => {
                        let dst = self.rm(inst, size)?;
                        let src = self.reg(inst, inst.reg, size);
                        self.alu_inst(inst, kind, dst, src, size)
                    }
                    2 | 3 => {
                        let src = self.rm(inst, size)?;
                        let dst = self.reg(inst, inst.reg, size);
                        self.alu_inst(inst, kind, dst, src, size)
                    }
                    _ => self.alu_inst(inst, kind, Op::Reg(RAX, false), Op::Imm(inst.imm), size),
                }
            }
            0x50..=0x57 => {
                self.asm.mov(true, S0, inst.reg);
                self.push(S0);
                Ok(Next::Continue)
            }
            0x58..=0x5F => {
                self.pop(S0);
                self.asm.mov(true, inst.reg, S0);
                Ok(Next::Continue)
            }
            // leave: the frame's stack pointer back, then its caller's frame pointer.
            0xC9 if !inst.opsize16 => {
                self.asm.mov(true, RSP, RBP);
                self.pop(RBP);
                Ok(Next::Continue)
            }
            0x63 => {
                let src = self.rm(inst, 32)?;
                self.load(src, 32, S0);
                if inst.w() {
                    self.asm.sbfx(true, S0, S0, 0, 32);
                }
                self.store(Op::Reg(inst.reg, false), v, S0)?;
                Ok(Next::Continue)
            }
            0x68 | 0x6A => {
                self.asm.mov_imm(S0, inst.imm as u64);
                self.push(S0);
                Ok(Next::Continue)
            }
            0x69 | 0x6B => {
                let src = self.rm(inst, v)?;
                self.load(src, v, S0);
                self.asm.mov_imm(S1, inst.imm as u64);
                self.imul(inst.reg, v)
            }
            0x70..=0x7F => self.jcc(inst, op & 15),
            0x80..=0x83 if op != 0x82 => {
                let kind = Self::alu_kind(inst.ext).ok_or(Unsupported)?;
                let size = if op == 0x80 { 8 } else { v };
                let dst = self.rm(inst, size)?;
                self.alu_inst(inst, kind, dst, Op::Imm(inst.imm), size)
            }
            0x84 | 0x85 => {
                let size = if op == 0x84 { 8 } else { v };
                let dst = self.rm(inst, size)?;
                let src = self.reg(inst, inst.reg, size);
                self.alu_inst(inst, Alu::Test, dst, src, size)
            }
            0x86 | 0x87 => {
                let size = if op == 0x86 { 8 } else { v };
                let other = self.rm(inst, size)?;
                let reg = self.reg(inst, inst.reg, size);
                self.load(reg, size, S1);
                if matches!(other, Op::Mem) {
                    // xchg with memory is always atomic.
                    self.asm.swpal(size, S1, S0, A);
                } else {
                    self.load(other, size, S0);
                    self.store(other, size, S1)?;
                }
                self.store(reg, size, S0)?;
                Ok(Next::Continue)
            }
            0x88..=0x8B => {
                let size = if op & 1 == 0 { 8 } else { v };
                let rm = self.rm(inst, size)?;
                let reg = self.reg(inst, inst.reg, size);
                let (dst, src) = if op < 0x8A { (rm, reg) } else { (reg, rm) };
                self.load(src, size, S0);
                self.store(dst, size, S0)?;
                Ok(Next::Continue)
            }
            0x8D => {
                let Some(Rm::Mem(mem)) = inst.rm else { return Err(Unsupported) };
                self.address(&mem, inst.next(), false);
                self.store(Op::Reg(inst.reg, false), v, A)?;
                Ok(Next::Continue)
            }
            0x8F if inst.ext == 0 => {
                self.pop(S0);
                let dst = self.rm(inst, 64)?;
                self.store(dst, 64, S0)?;
                Ok(Next::Continue)
            }
            0x90 if inst.rep => {
                self.asm.yield_();
                Ok(Next::Continue)
            }
            0x90 if inst.rex & 1 == 0 => Ok(Next::Continue),
            0x90..=0x97 => {
                self.load(Op::Reg(RAX, false), v, S0);
                self.load(Op::Reg(inst.reg, false), v, S1);
                self.store(Op::Reg(RAX, false), v, S1)?;
                self.store(Op::Reg(inst.reg, false), v, S0)?;
                Ok(Next::Continue)
            }
            0x98 => {
                // cbw, cwde, cdqe
                let half = v / 2;
                self.asm.sbfx(true, S0, RAX, 0, half);
                self.store(Op::Reg(RAX, false), v, S0)?;
                Ok(Next::Continue)
            }
            0x99 => {
                // cwd, cdq, cqo
                self.asm.sbfx(true, S0, RAX, 0, v);
                self.asm.asr_imm(true, S0, S0, 63);
                self.store(Op::Reg(RDX, false), v, S0)?;
                Ok(Next::Continue)
            }
            0xA4 | 0xA5 | 0xAA | 0xAB => self.string(inst, op),
            // fwait: no x87 exception is ever pending.
            0x9B => Ok(Next::Continue),
            0x9E => {
                self.sahf();
                Ok(Next::Continue)
            }
            0x9F => {
                self.lahf();
                Ok(Next::Continue)
            }
            0xD8..=0xDF => self.x87(inst),
            0xA8 | 0xA9 => {
                let size = if op == 0xA8 { 8 } else { v };
                self.alu_inst(inst, Alu::Test, Op::Reg(RAX, false), Op::Imm(inst.imm), size)
            }
            0xB0..=0xB7 => {
                let dst = self.reg(inst, inst.reg, 8);
                self.load(Op::Imm(inst.imm), 8, S0);
                self.store(dst, 8, S0)?;
                Ok(Next::Continue)
            }
            0xB8..=0xBF => {
                self.load(Op::Imm(inst.imm), v, S0);
                self.store(Op::Reg(inst.reg, false), v, S0)?;
                Ok(Next::Continue)
            }
            0xC0 | 0xC1 | 0xD0..=0xD3 => {
                let size = if op & 1 == 0 { 8 } else { v };
                let count = match op {
                    0xC0 | 0xC1 => Count::Imm(inst.imm as u8),
                    0xD0 | 0xD1 => Count::Imm(1),
                    _ => Count::Cl,
                };
                self.shift(inst, size, count)
            }
            0xC2 | 0xC3 => {
                self.pop(T);
                if op == 0xC2 && inst.imm != 0 {
                    self.asm.add_imm(true, RSP, RSP, (inst.imm as u16) & 0xFFF);
                }
                self.jump_indirect();
                Ok(Next::End)
            }
            0xC6 | 0xC7 if inst.ext == 0 => {
                let size = if op == 0xC6 { 8 } else { v };
                let dst = self.rm(inst, size)?;
                self.load(Op::Imm(inst.imm), size, S0);
                self.store(dst, size, S0)?;
                Ok(Next::Continue)
            }
            0xCC => {
                self.exit(exit::BREAKPOINT, inst.next());
                Ok(Next::End)
            }
            0xCD => {
                match inst.imm as u8 {
                    0x03 => self.exit(exit::BREAKPOINT, inst.next()),
                    0x80 => self.exit(exit::INT80, inst.next()),
                    0x2E => self.exit(exit::INT2E, inst.next()),
                    0x29 => self.exit(exit::FASTFAIL, inst.at),
                    _ => return Err(Unsupported),
                }
                Ok(Next::End)
            }
            0xE3 => {
                // jrcxz
                let at = self.asm.len();
                self.asm.cbz(true, RCX, 0);
                self.jump(inst.next());
                let here = self.asm.len();
                self.asm.patch(at, here);
                self.jump(inst.target());
                Ok(Next::End)
            }
            0xE8 => {
                self.asm.mov_imm(S0, inst.next());
                self.push(S0);
                self.jump(inst.target());
                Ok(Next::End)
            }
            0xE9 | 0xEB => {
                self.jump(inst.target());
                Ok(Next::End)
            }
            0xF6 | 0xF7 => self.group3(inst, if op == 0xF6 { 8 } else { v }),
            // clc and stc: CF cleared or set; cmc: flipped (ARM's C is CF's inverse, so flipping one flips the other).
            0xF8 => {
                self.set_carry(ZR);
                Ok(Next::Continue)
            }
            0xF9 => {
                self.asm.mov_imm(S0, 1);
                self.set_carry(S0);
                Ok(Next::Continue)
            }
            0xF5 => {
                let a = &mut self.asm;
                a.mrs_nzcv(F1);
                a.mov_imm(F2, 1 << 29);
                a.eor(true, F1, F1, F2);
                a.msr_nzcv(F1);
                Ok(Next::Continue)
            }
            0xFE | 0xFF if inst.ext < 2 => {
                let size = if op == 0xFE { 8 } else { v };
                let dst = self.rm(inst, size)?;
                self.asm.mrs_nzcv(F0);
                let kind = if inst.ext == 0 { Alu::Add } else { Alu::Sub };
                self.asm.mov_imm(S1, 1);
                if inst.lock {
                    if !matches!(dst, Op::Mem) {
                        return Err(Unsupported);
                    }
                    if kind == Alu::Sub {
                        self.asm.sub(true, S2, ZR, S1);
                        self.asm.ldaddal(size, S2, S0, A);
                    } else {
                        self.asm.ldaddal(size, S1, S0, A);
                    }
                    self.alu(kind, size, S0, S1);
                } else {
                    self.load(dst, size, S0);
                    self.alu(kind, size, S0, S1);
                    self.store(dst, size, R)?;
                }
                self.keep_carry(F0);
                Ok(Next::Continue)
            }
            0xFF => match inst.ext {
                2 | 4 => {
                    let target = self.rm(inst, 64)?;
                    self.load(target, 64, T);
                    if inst.ext == 2 {
                        self.asm.mov_imm(S0, inst.next());
                        self.push(S0);
                    }
                    self.jump_indirect();
                    Ok(Next::End)
                }
                6 => {
                    let src = self.rm(inst, 64)?;
                    self.load(src, 64, S0);
                    self.push(S0);
                    Ok(Next::Continue)
                }
                _ => Err(Unsupported),
            },
            _ => Err(Unsupported),
        }
    }

    fn jcc(&mut self, inst: &Inst, cc: u8) -> Lowered {
        let cond = cond(cc).ok_or(Unsupported)?;
        let at = self.asm.len();
        self.asm.b_cond(cond, 0);
        self.jump(inst.next());
        let here = self.asm.len();
        self.asm.patch(at, here);
        self.jump(inst.target());
        Ok(Next::End)
    }

    /// `movs` and `stos`, once or with `rep`, forwards only (DF clear).
    fn string(&mut self, inst: &Inst, op: u8) -> Lowered {
        let size = if op & 1 == 0 { 8 } else { inst.size_v() };
        let step = u16::from(size / 8);
        let moving = op < 0xA8;
        let a = &mut self.asm;
        let top = a.len();
        let done = if inst.rep {
            a.cbz(true, RCX, 0);
            Some(top)
        } else {
            None
        };
        if moving {
            a.ldr(size, S0, RSI);
            a.str(size, S0, RDI);
            a.add_imm(true, RSI, RSI, step);
        } else {
            a.str(size, RAX, RDI);
        }
        a.add_imm(true, RDI, RDI, step);
        if let Some(done) = done {
            a.sub_imm(true, RCX, RCX, 1);
            let back = a.len();
            a.b(0);
            a.patch(back, top);
            let here = a.len();
            a.patch(done, here);
        }
        Ok(Next::Continue)
    }

    /// `test`, `not`, `neg`, `mul`, `imul`, `div`, `idiv`.
    fn group3(&mut self, inst: &Inst, size: u8) -> Lowered {
        let operand = self.rm(inst, size)?;
        match inst.ext {
            0 | 1 => self.alu_inst(inst, Alu::Test, operand, Op::Imm(inst.imm), size),
            2 => {
                self.load(operand, size, S0);
                self.asm.orn(true, R, ZR, S0);
                self.store(operand, size, R)?;
                Ok(Next::Continue)
            }
            3 => {
                self.load(operand, size, S0);
                self.alu(Alu::Sub, size, ZR, S0);
                self.store(operand, size, R)?;
                Ok(Next::Continue)
            }
            4..=7 if size >= 32 => {
                self.load(operand, size, S1);
                match inst.ext {
                    4 | 5 => self.widening_multiply(size, inst.ext == 5),
                    _ => self.divide(inst, size, inst.ext == 7),
                }
            }
            4 | 5 => {
                self.load(operand, size, S1);
                self.narrow_multiply(size, inst.ext == 5)
            }
            6 | 7 => {
                self.load(operand, size, S1);
                self.narrow_divide(inst, size, inst.ext == 7)
            }
            _ => Err(Unsupported),
        }
    }

    /// 8- and 16-bit `mul` and `imul`: `ax = al * S1`, or `dx:ax = ax * S1`. CF and OF say whether the high half is
    /// more than the low half's extension.
    fn narrow_multiply(&mut self, size: u8, signed: bool) -> Lowered {
        let a = &mut self.asm;
        if signed {
            a.sbfx(true, S0, RAX, 0, size);
            a.sbfx(true, S1, S1, 0, size);
        } else {
            a.ubfx(true, S0, RAX, 0, size);
            a.ubfx(true, S1, S1, 0, size);
        }
        a.mul(true, R, S0, S1);
        if signed {
            a.sbfx(true, S3, R, 0, size);
            a.subs(true, ZR, R, S3);
        } else {
            a.lsr_imm(true, S3, R, size);
            a.cmp_imm(true, S3, 0);
        }
        a.cset(true, F0, Cond::Ne);
        self.store(Op::Reg(RAX, false), 16, R)?;
        if size == 16 {
            self.asm.lsr_imm(true, S2, R, 16);
            self.store(Op::Reg(RDX, false), 16, S2)?;
        }
        self.set_cf_of(F0);
        Ok(Next::Continue)
    }

    /// 8- and 16-bit `div` and `idiv`: `ax / S1` into `al` and `ah`, or `dx:ax / S1` into `ax` and `dx`. A divisor of 0,
    /// or a quotient that doesn't fit, is x86's divide error.
    fn narrow_divide(&mut self, inst: &Inst, size: u8, signed: bool) -> Lowered {
        let pc = inst.at;
        let a = &mut self.asm;
        // The dividend: ax, or dx:ax, as 2 * size bits.
        if size == 8 {
            a.ubfx(true, S0, RAX, 0, 16);
        } else {
            a.ubfx(true, S0, RAX, 0, 16);
            a.ubfx(true, S2, RDX, 0, 16);
            a.orr_shift(true, S0, S0, S2, Shift::Lsl, 16);
        }
        if signed {
            a.sbfx(true, S0, S0, 0, size * 2);
            a.sbfx(true, S1, S1, 0, size);
        } else {
            a.ubfx(true, S1, S1, 0, size);
        }
        self.exit_unless(|a| a.cbnz(true, S1, 0), exit::DIVIDE, pc);
        let a = &mut self.asm;
        if signed { a.sdiv(true, R, S0, S1) } else { a.udiv(true, R, S0, S1) }
        if signed { a.sbfx(true, S3, R, 0, size) } else { a.ubfx(true, S3, R, 0, size) }
        a.subs(true, ZR, R, S3);
        self.exit_unless(|a| a.b_cond(Cond::Eq, 0), exit::DIVIDE, pc);
        let a = &mut self.asm;
        a.msub(true, S2, R, S1, S0);
        if size == 8 {
            a.ubfx(true, R, R, 0, 8);
            a.orr_shift(true, R, R, S2, Shift::Lsl, 8);
            self.store(Op::Reg(RAX, false), 16, R)?;
        } else {
            self.store(Op::Reg(RAX, false), 16, R)?;
            self.store(Op::Reg(RDX, false), 16, S2)?;
        }
        Ok(Next::Continue)
    }

    /// `rdx:rax = rax * S1`.
    fn widening_multiply(&mut self, size: u8, signed: bool) -> Lowered {
        let a = &mut self.asm;
        if size == 64 {
            a.mul(true, R, RAX, S1);
            if signed {
                a.smulh(S2, RAX, S1);
                a.subs_shift(true, ZR, S2, R, Shift::Asr, 63);
            } else {
                a.umulh(S2, RAX, S1);
                a.cmp_imm(true, S2, 0);
            }
            a.cset(true, F0, Cond::Ne);
            a.mov(true, RAX, R);
            a.mov(true, RDX, S2);
        } else {
            if signed {
                a.sbfx(true, S0, RAX, 0, 32);
                a.sbfx(true, S1, S1, 0, 32);
            } else {
                a.mov(false, S0, RAX);
            }
            a.mul(true, R, S0, S1);
            if signed {
                a.sbfx(true, S3, R, 0, 32);
                a.subs(true, ZR, R, S3);
            } else {
                a.lsr_imm(true, S3, R, 32);
                a.cmp_imm(true, S3, 0);
            }
            a.cset(true, F0, Cond::Ne);
            a.mov(false, RAX, R);
            a.lsr_imm(true, S2, R, 32);
            a.mov(false, RDX, S2);
        }
        self.set_cf_of(F0);
        Ok(Next::Continue)
    }

    /// `reg = S0 * S1`, two- and three-operand `imul`.
    fn imul(&mut self, reg: u8, size: u8) -> Lowered {
        let a = &mut self.asm;
        match size {
            64 => {
                a.mul(true, R, S0, S1);
                a.smulh(S2, S0, S1);
                a.subs_shift(true, ZR, S2, R, Shift::Asr, 63);
            }
            16 | 32 => {
                a.sbfx(true, S0, S0, 0, size);
                a.sbfx(true, S1, S1, 0, size);
                a.mul(true, R, S0, S1);
                a.sbfx(true, S3, R, 0, size);
                a.subs(true, ZR, R, S3);
            }
            _ => return Err(Unsupported),
        }
        a.cset(true, F0, Cond::Ne);
        self.store(Op::Reg(reg, false), size, R)?;
        self.set_cf_of(F0);
        Ok(Next::Continue)
    }

    /// `rax = rdx:rax / S1`, `rdx` the remainder. Unsigned 64-bit division with `rdx` set divides all 128 bits; a
    /// signed one whose `rdx` isn't just `rax`'s sign isn't covered.
    fn divide(&mut self, inst: &Inst, size: u8, signed: bool) -> Lowered {
        let pc = inst.at;
        let sf = size == 64;
        self.exit_unless(|a| a.cbnz(sf, S1, 0), exit::DIVIDE, pc);
        if size == 64 {
            if signed {
                self.asm.asr_imm(true, S2, RAX, 63);
                self.asm.subs(true, ZR, S2, RDX);
                self.exit_unless(|a| a.b_cond(Cond::Eq, 0), exit::UNSUPPORTED, pc);
                self.asm.sdiv(true, R, RAX, S1);
            } else {
                return self.divide_128(pc);
            }
            self.asm.msub(true, S2, R, S1, RAX);
            self.asm.mov(true, RAX, R);
            self.asm.mov(true, RDX, S2);
        } else {
            let a = &mut self.asm;
            a.lsl_imm(true, S0, RDX, 32);
            a.mov(false, S3, RAX);
            a.orr(true, S0, S0, S3);
            if signed {
                a.sbfx(true, S1, S1, 0, 32);
                a.sdiv(true, R, S0, S1);
                a.sbfx(true, S3, R, 0, 32);
                a.subs(true, ZR, R, S3);
                self.exit_unless(|a| a.b_cond(Cond::Eq, 0), exit::DIVIDE, pc);
            } else {
                a.udiv(true, R, S0, S1);
                a.lsr_imm(true, S3, R, 32);
                self.exit_unless(|a| a.cbz(true, S3, 0), exit::DIVIDE, pc);
            }
            let a = &mut self.asm;
            a.msub(true, S2, R, S1, S0);
            a.mov(false, RAX, R);
            a.mov(false, RDX, S2);
        }
        Ok(Next::Continue)
    }

    /// `shl`, `shr`, `sar`, `rol`, `ror`. An 8- or 16-bit rotate rotates the value repeated across 32 bits, whose low
    /// bits are then the value rotated: the count is masked to 5 bits, as x86 does, and every multiple of the size
    /// comes back round.
    fn shift(&mut self, inst: &Inst, size: u8, count: Count) -> Lowered {
        let kind = inst.ext;
        if matches!(kind, 2 | 3) {
            return Err(Unsupported);
        }
        let operand = self.rm(inst, size)?;
        let sf = size == 64;
        let bits = if sf { 6 } else { 5 };
        let skip = match count {
            Count::Imm(n) => {
                let n = n & ((1 << bits) - 1);
                if n == 0 {
                    return Ok(Next::Continue);
                }
                self.asm.mov_imm(S1, u64::from(n));
                None
            }
            Count::Cl => {
                self.asm.ubfx(true, S1, RCX, 0, bits);
                let at = self.asm.len();
                self.asm.cbz(true, S1, 0);
                Some(at)
            }
        };
        self.load(operand, size, S0);
        if kind == 7 && size < 64 {
            self.asm.sbfx(true, S0, S0, 0, size);
        }
        let rotate_size = if matches!(kind, 0 | 1) && size < 32 {
            let a = &mut self.asm;
            a.ubfx(false, S0, S0, 0, size);
            if size == 8 {
                a.orr_shift(false, S0, S0, S0, Shift::Lsl, 8);
            }
            a.orr_shift(false, S0, S0, S0, Shift::Lsl, 16);
            32
        } else {
            size
        };
        let a = &mut self.asm;
        match kind {
            4 | 6 => {
                a.lslv(sf, R, S0, S1);
                // CF: the last bit out, bit (size - n).
                a.mov_imm(F1, u64::from(size));
                a.sub(true, F1, F1, S1);
                a.lsrv(true, F0, S0, F1);
            }
            5 | 7 => {
                if kind == 5 { a.lsrv(sf, R, S0, S1) } else { a.asrv(true, R, S0, S1) }
                // CF: bit (n - 1).
                a.sub_imm(true, F1, S1, 1);
                a.lsrv(true, F0, S0, F1);
            }
            0 => {
                a.mov_imm(S2, u64::from(rotate_size));
                a.sub(true, S2, S2, S1);
                a.rorv(sf, R, S0, S2);
                a.mov(true, F0, R);
            }
            _ => {
                a.rorv(sf, R, S0, S1);
                a.lsr_imm(true, F0, R, size - 1);
            }
        }
        a.ubfx(true, F0, F0, 0, 1);
        if matches!(kind, 4..=7) {
            // SF and ZF from the result at its size; OF clear.
            if size < 32 {
                a.lsl_imm(false, S2, R, 32 - size);
                a.ands(false, ZR, S2, S2);
            } else {
                a.ands(sf, ZR, R, R);
            }
        }
        self.set_carry(F0);
        self.store(operand, size, R)?;
        if let Some(at) = skip {
            let here = self.asm.len();
            self.asm.patch(at, here);
        }
        Ok(Next::Continue)
    }

    fn two_byte(&mut self, inst: &Inst) -> Lowered {
        let v = inst.size_v();
        let op = inst.op;
        match op {
            0x05 => {
                // syscall: rcx is where it returns to; r11 would be the flags.
                self.asm.mov_imm(RCX, inst.next());
                self.asm.mov_imm(11, 0x202);
                self.exit(exit::SYSCALL, inst.next());
                Ok(Next::End)
            }
            0x0B => {
                self.exit(exit::UNSUPPORTED, inst.at);
                Ok(Next::End)
            }
            0x18..=0x1F => Ok(Next::Continue), // prefetches, hints, endbr64, nop r/m
            0x10..=0x17 | 0x28..=0x2F | 0x50..=0x7F | 0xC2 | 0xC4..=0xC6 | 0xD0..=0xFF => self.sse(inst),
            0xAE if matches!(inst.rm, Some(Rm::Mem(_))) && matches!(inst.ext, 2 | 3) => self.mxcsr(inst),
            0xAE if matches!(inst.rm, Some(Rm::Mem(_))) && matches!(inst.ext, 0 | 1) => self.fxsave(inst, inst.ext == 1),
            0x31 => {
                self.asm.mrs_cntvct(S0);
                self.asm.mov(false, RAX, S0);
                self.asm.lsr_imm(true, RDX, S0, 32);
                Ok(Next::Continue)
            }
            0x40..=0x4F => {
                let cond = cond(op & 15).ok_or(Unsupported)?;
                let src = self.rm(inst, v)?;
                self.load(src, v, S1);
                self.load(Op::Reg(inst.reg, false), v, S0);
                self.asm.csel(true, R, S1, S0, cond);
                self.store(Op::Reg(inst.reg, false), v, R)?;
                Ok(Next::Continue)
            }
            0x80..=0x8F => self.jcc(inst, op & 15),
            0x90..=0x9F => {
                let cond = cond(op & 15).ok_or(Unsupported)?;
                let dst = self.rm(inst, 8)?;
                self.asm.cset(true, S0, cond);
                self.store(dst, 8, S0)?;
                Ok(Next::Continue)
            }
            0xA2 => {
                self.exit(exit::CPUID, inst.next());
                Ok(Next::End)
            }
            0xAE if matches!(inst.rm, Some(Rm::Reg(_))) && matches!(inst.ext, 5..=7) => {
                self.asm.dmb_ish();
                Ok(Next::Continue)
            }
            0xAF => {
                let src = self.rm(inst, v)?;
                self.load(src, v, S1);
                self.load(Op::Reg(inst.reg, false), v, S0);
                self.imul(inst.reg, v)
            }
            0xB0 | 0xB1 => self.cmpxchg(inst, if op == 0xB0 { 8 } else { v }),
            // bt, bts, btr, btc, by a register.
            0xA3 | 0xAB | 0xB3 | 0xBB => self.bit_test(inst, v, (op >> 3) & 3, None),
            0xA4 | 0xA5 | 0xAC | 0xAD => self.double_shift(inst, v),
            // The same, by an immediate.
            0xBA if inst.ext >= 4 => self.bit_test(inst, v, inst.ext - 4, Some(inst.imm as u8)),
            0xB6 | 0xB7 | 0xBE | 0xBF => {
                let from = if op & 1 == 0 { 8 } else { 16 };
                let src = self.rm(inst, from)?;
                self.load(src, from, S0);
                if op >= 0xBE {
                    self.asm.sbfx(true, S0, S0, 0, from);
                }
                self.store(Op::Reg(inst.reg, false), v, S0)?;
                Ok(Next::Continue)
            }
            0xBC | 0xBD if v >= 32 => self.bit_scan(inst, v),
            0xC0 | 0xC1 => {
                let size = if op == 0xC0 { 8 } else { v };
                let dst = self.rm(inst, size)?;
                let reg = self.reg(inst, inst.reg, size);
                self.load(reg, size, S1);
                if matches!(dst, Op::Mem) {
                    self.asm.ldaddal(size, S1, S0, A);
                    self.alu(Alu::Add, size, S0, S1);
                } else {
                    self.load(dst, size, S0);
                    self.alu(Alu::Add, size, S0, S1);
                    self.store(dst, size, R)?;
                }
                self.store(reg, size, S0)?;
                Ok(Next::Continue)
            }
            0xC8..=0xCF if v >= 32 => {
                self.asm.rev(v == 64, inst.reg, inst.reg);
                Ok(Next::Continue)
            }
            _ => Err(Unsupported),
        }
    }

    /// Unsigned `rdx:rax / S1`: one `udiv` while `rdx` is 0, else long division, a bit at a time. A quotient too big
    /// for 64 bits (`rdx >= S1`) is x86's divide error. The flags, which x86 leaves undefined, are changed.
    fn divide_128(&mut self, pc: u64) -> Lowered {
        let slow = self.asm.len();
        self.asm.cbnz(true, RDX, 0);
        let a = &mut self.asm;
        a.udiv(true, R, RAX, S1);
        a.msub(true, S2, R, S1, RAX);
        a.mov(true, RAX, R);
        a.mov(true, RDX, S2);
        let done = a.len();
        a.b(0);
        let here = a.len();
        a.patch(slow, here);
        a.subs(true, ZR, RDX, S1);
        self.exit_unless(|a| a.b_cond(Cond::Cc, 0), exit::DIVIDE, pc);
        // S0:S3 is the dividend, shifted left a bit each round into the remainder S0; R gathers the quotient.
        let a = &mut self.asm;
        a.mov(true, S0, RDX);
        a.mov(true, S3, RAX);
        a.mov_imm(R, 0);
        a.mov_imm(F0, 64);
        let top = a.len();
        a.lsr_imm(true, F1, S0, 63);
        a.extr(S0, S0, S3, 63);
        a.lsl_imm(true, S3, S3, 1);
        // Subtract the divisor when the remainder (with the bit shifted out of it) is at least it.
        a.subs(true, ZR, S0, S1);
        a.cset(true, F2, Cond::Cs);
        a.orr(true, F2, F2, F1);
        a.sub(true, T, S0, S1);
        a.cmp_imm(true, F2, 0);
        a.csel(true, S0, T, S0, Cond::Ne);
        a.orr_shift(true, R, F2, R, Shift::Lsl, 1);
        a.sub_imm(true, F0, F0, 1);
        let back = top as i32 - a.len() as i32;
        a.cbnz(true, F0, back);
        a.mov(true, RAX, R);
        a.mov(true, RDX, S0);
        let end = a.len();
        a.patch(done, end);
        Ok(Next::Continue)
    }

    /// `shld` (A4 by an immediate, A5 by cl) and `shrd` (AC, AD): the destination shifted, the bits it gains taken
    /// from the other end of the source. A count of 0 (after masking) changes nothing, flags included. CF is the last
    /// bit shifted out; SF and ZF are the result's; OF, defined only for a count of 1, is left clear.
    fn double_shift(&mut self, inst: &Inst, size: u8) -> Lowered {
        if size < 32 {
            return Err(Unsupported);
        }
        let (left, sf) = (matches!(inst.op, 0xA4 | 0xA5), size == 64);
        let dst = self.rm(inst, size)?;
        self.load(dst, size, S0);
        self.asm.mov(sf, S1, inst.reg);
        let skip = if matches!(inst.op, 0xA4 | 0xAC) {
            let count = (inst.imm as u8) & (size - 1);
            if count == 0 {
                return Ok(Next::Continue);
            }
            self.asm.mov_imm(S2, u64::from(count));
            None
        } else {
            self.asm.ubfx(true, S2, RCX, 0, if sf { 6 } else { 5 });
            let at = self.asm.len();
            self.asm.cbz(true, S2, 0);
            Some(at)
        };
        let a = &mut self.asm;
        a.mov_imm(S3, u64::from(size));
        a.sub(sf, S3, S3, S2);
        if left {
            a.lslv(sf, R, S0, S2);
            a.lsrv(sf, F0, S1, S3);
            a.orr(sf, R, R, F0);
            // CF: bit `size - count` of the destination.
            a.lsrv(sf, F0, S0, S3);
        } else {
            a.lsrv(sf, R, S0, S2);
            a.lslv(sf, F0, S1, S3);
            a.orr(sf, R, R, F0);
            // CF: bit `count - 1` of the destination.
            a.sub_imm(sf, F0, S2, 1);
            a.lsrv(sf, F0, S0, F0);
        }
        a.ubfx(true, S3, F0, 0, 1);
        self.store(dst, size, R)?;
        self.asm.ands(sf, ZR, R, R);
        self.set_carry(S3);
        if let Some(at) = skip {
            let end = self.asm.len();
            self.asm.patch(at, end);
        }
        Ok(Next::Continue)
    }

    /// `cmpxchg`: atomic with memory, as x86's always is when locked and as nothing relies on it not being.
    fn cmpxchg(&mut self, inst: &Inst, size: u8) -> Lowered {
        let dst = self.rm(inst, size)?;
        let reg = self.reg(inst, inst.reg, size);
        self.load(Op::Reg(RAX, false), size, S2);
        self.load(reg, size, S1);
        if matches!(dst, Op::Mem) {
            self.asm.mov(true, S0, S2);
            self.asm.casal(size, S0, S1, A);
        } else {
            self.load(dst, size, S0);
        }
        // The flags are cmp's: rax against what was there.
        self.load(Op::Reg(RAX, false), size, S3);
        self.alu(Alu::Cmp, size, S3, S0);
        let at = self.asm.len();
        self.asm.b_cond(Cond::Eq, 0);
        // Not equal: rax gets what was there.
        self.store(Op::Reg(RAX, false), size, S0)?;
        let skip = self.asm.len();
        self.asm.b(0);
        let equal = self.asm.len();
        self.asm.patch(at, equal);
        if !matches!(dst, Op::Mem) {
            self.store(dst, size, S1)?;
        }
        let here = self.asm.len();
        self.asm.patch(skip, here);
        Ok(Next::Continue)
    }

    /// `bsf`, `bsr`, `tzcnt`, `lzcnt`.
    /// `bt`, `bts`, `btr`, `btc` (`kind` 0 to 3): CF is the bit, which the last three then set, clear or flip. The
    /// other flags are left as they were. A memory operand with a register bit offset can reach past the operand, and
    /// isn't covered, nor is `lock`.
    /// `bt`, `bts`, `btr` and `btc` (`kind` 0 to 3). On memory with a register's bit number, the number is signed and
    /// can reach past the operand: the address moves by its whole operands, as x86's bit strings do. With `lock`, the
    /// change is one of ARM's atomics (`ldset`, `ldclr`, `ldeor`) and CF comes from the value it replaced.
    fn bit_test(&mut self, inst: &Inst, size: u8, kind: u8, imm: Option<u8>) -> Lowered {
        let dst = self.rm(inst, size)?;
        if inst.lock && (kind == 0 || !matches!(dst, Op::Mem)) {
            return Err(Unsupported);
        }
        let sf = size == 64;
        let bits = size.trailing_zeros() as u8;
        match imm {
            Some(n) => self.asm.mov_imm(S1, u64::from(n & (size - 1))),
            None if matches!(dst, Op::Mem) => {
                self.load(Op::Reg(inst.reg, false), size, S1);
                let a = &mut self.asm;
                a.sbfx(true, S1, S1, 0, size);
                a.asr_imm(true, S2, S1, bits);
                a.add_shift(true, A, A, S2, Shift::Lsl, bits - 3);
                a.ubfx(true, S1, S1, 0, bits);
            }
            None => {
                self.load(Op::Reg(inst.reg, false), size, S1);
                self.asm.ubfx(true, S1, S1, 0, bits);
            }
        }
        if inst.lock {
            let a = &mut self.asm;
            a.mov_imm(S3, 1);
            a.lslv(true, S3, S3, S1);
            match kind {
                1 => a.ldsetal(size, S3, S0, A),
                2 => a.ldclral(size, S3, S0, A),
                _ => a.ldeoral(size, S3, S0, A),
            }
            a.lsrv(true, S2, S0, S1);
            a.ubfx(true, S2, S2, 0, 1);
            self.set_carry(S2);
            return Ok(Next::Continue);
        }
        self.load(dst, size, S0);
        let a = &mut self.asm;
        a.lsrv(true, S2, S0, S1);
        a.ubfx(true, S2, S2, 0, 1);
        self.set_carry(S2);
        if kind == 0 {
            return Ok(Next::Continue);
        }
        let a = &mut self.asm;
        a.mov_imm(S3, 1);
        a.lslv(true, S3, S3, S1);
        match kind {
            1 => a.orr(sf, S0, S0, S3),
            2 => {
                a.orn(sf, S3, ZR, S3);
                a.and(sf, S0, S0, S3);
            }
            _ => a.eor(sf, S0, S0, S3),
        }
        self.store(dst, size, S0)?;
        Ok(Next::Continue)
    }

    fn bit_scan(&mut self, inst: &Inst, size: u8) -> Lowered {
        let sf = size == 64;
        let src = self.rm(inst, size)?;
        self.load(src, size, S1);
        let forward = inst.op == 0xBC;
        let a = &mut self.asm;
        if forward {
            a.rbit(sf, S2, S1);
            a.clz(sf, R, S2);
        } else {
            a.clz(sf, R, S1);
        }
        if inst.rep {
            // tzcnt, lzcnt: CF if the source was zero, ZF if the count is.
            a.cmp_imm(sf, S1, 0);
            a.cset(true, F0, Cond::Eq);
            a.cmp_imm(sf, R, 0);
            a.cset(true, S3, Cond::Eq);
            self.set_zf_cf(S3, F0);
        } else {
            if !forward {
                a.mov_imm(S3, u64::from(size - 1));
                a.sub(sf, R, S3, R);
            }
            // bsf, bsr: ZF if the source was zero, when the destination stays as it was.
            self.load(Op::Reg(inst.reg, false), size, S0);
            let a = &mut self.asm;
            a.cmp_imm(sf, S1, 0);
            a.csel(true, R, S0, R, Cond::Eq);
        }
        self.store(Op::Reg(inst.reg, false), size, R)?;
        Ok(Next::Continue)
    }

    /// `xmm` registers by number.
    fn xmm_rm(&mut self, inst: &Inst) -> Result<Option<u8>, Unsupported> {
        match inst.rm {
            Some(Rm::Reg(n)) => Ok(Some(n)),
            Some(Rm::Mem(mem)) => {
                self.address(&mem, inst.next(), true);
                Ok(None)
            }
            None => Err(Unsupported),
        }
    }

    /// `movups`, `movaps`, `movdqu`, `movdqa`, `movss`, `movsd`, and their pd forms.
    fn sse_move(&mut self, inst: &Inst) -> Lowered {
        let op = inst.op;
        // 0x10, 0x28, 0x6F load into reg; 0x11, 0x29, 0x7F store from it.
        let loading = matches!(op, 0x10 | 0x28 | 0x6F);
        let scalar = if matches!(op, 0x10 | 0x11) && inst.rep {
            Some(32)
        } else if matches!(op, 0x10 | 0x11) && inst.repne {
            Some(64)
        } else {
            None
        };
        if matches!(op, 0x6F | 0x7F) && !(inst.opsize16 || inst.rep) {
            return Err(Unsupported); // MMX
        }
        let xmm = inst.reg;
        let rm = self.xmm_rm(inst)?;
        let a = &mut self.asm;
        match (scalar, rm, loading) {
            (None, Some(n), true) => a.v_mov(xmm, n),
            (None, Some(n), false) => a.v_mov(n, xmm),
            (None, None, true) => a.ldr_q(xmm, A),
            (None, None, false) => a.str_q(xmm, A),
            (Some(32), Some(n), true) => a.ins_s0(xmm, n),
            (Some(32), Some(n), false) => a.ins_s0(n, xmm),
            (Some(32), None, true) => a.ldr_s(xmm, A),
            (Some(32), None, false) => a.str_s(xmm, A),
            (Some(_), Some(n), true) => a.ins_d0(xmm, n),
            (Some(_), Some(n), false) => a.ins_d0(n, xmm),
            (Some(_), None, true) => a.ldr_d(xmm, A),
            (Some(_), None, false) => a.str_d(xmm, A),
        }
        Ok(Next::Continue)
    }

    /// `pxor`/`xorps`, `pand`/`andps`, `por`/`orps`, `pandn`/`andnps`, `pcmpeqb`.
    fn sse_logic(&mut self, inst: &Inst) -> Lowered {
        if inst.rep || inst.repne || (matches!(inst.op, 0xDB | 0xDF | 0xEB | 0xEF | 0x74) && !inst.opsize16) {
            return Err(Unsupported);
        }
        let d = inst.reg;
        let src = match self.xmm_rm(inst)? {
            Some(n) => n,
            None => {
                self.asm.ldr_q(V_TMP, A);
                V_TMP
            }
        };
        let a = &mut self.asm;
        match inst.op {
            0x57 | 0xEF => a.v_eor(d, d, src),
            0x54 | 0xDB => a.v_and(d, d, src),
            0x56 | 0xEB => a.v_orr(d, d, src),
            0x55 | 0xDF => a.v_bic(d, src, d),
            0x74 => a.cmeq_16b(d, d, src),
            _ => return Err(Unsupported),
        }
        Ok(Next::Continue)
    }

    /// `movd`, `movq` and `pmovmskb`: between xmm and general registers or memory.
    fn sse_gpr(&mut self, inst: &Inst) -> Lowered {
        let size = if inst.w() { 64 } else { 32 };
        match (inst.op, inst.opsize16, inst.rep) {
            // movd/movq xmm, r/m
            (0x6E, true, false) => {
                let src = self.rm(inst, size)?;
                self.load(src, size, S0);
                if size == 64 { self.asm.fmov_d_x(inst.reg, S0) } else { self.asm.fmov_s_w(inst.reg, S0) }
            }
            // movd/movq r/m, xmm
            (0x7E, true, false) => {
                let dst = self.rm(inst, size)?;
                if size == 64 { self.asm.fmov_x_d(S0, inst.reg) } else { self.asm.fmov_w_s(S0, inst.reg) }
                self.store(dst, size, S0)?;
            }
            // movq xmm, xmm/m64
            (0x7E, false, true) => match self.xmm_rm(inst)? {
                Some(n) => self.asm.fmov_d_d(inst.reg, n),
                None => self.asm.ldr_d(inst.reg, A),
            },
            // movq xmm/m64, xmm
            (0xD6, true, false) => match self.xmm_rm(inst)? {
                Some(n) => self.asm.fmov_d_d(n, inst.reg),
                None => self.asm.str_d(inst.reg, A),
            },
            // pmovmskb r, xmm: each byte's top bit
            (0xD7, true, false) => {
                let Some(Rm::Reg(n)) = inst.rm else { return Err(Unsupported) };
                let a = &mut self.asm;
                a.cmlt0_16b(31, n);
                a.mov_imm(T, 0x8040_2010_0804_0201);
                a.fmov_d_x(30, T);
                a.ins_d1_d0(30, 30);
                a.v_and(31, 31, 30);
                a.addv_8b(28, 31);
                a.ext_16b(29, 31, 31, 8);
                a.addv_8b(29, 29);
                a.fmov_w_s(S0, 28);
                a.fmov_w_s(S1, 29);
                a.orr_shift(false, S0, S0, S1, Shift::Lsl, 8);
                a.mov(false, inst.reg, S0);
            }
            _ => return Err(Unsupported),
        }
        Ok(Next::Continue)
    }
}

#[derive(Clone, Copy)]
enum Count {
    Imm(u8),
    Cl,
}
