//! SSE and SSE2 on ARM64's vector registers: `xmm0`–`xmm15` are `v0`–`v15`, and `v28`–`v31` are scratch.
//!
//! What's covered is what C compilers and glibc's baseline (x86-64-v1) code use: moves of every width and half,
//! logic, packed integer arithmetic and compares, shifts by an immediate, unpacks, packs and shuffles, the
//! `movmsk`s, scalar and packed floating point with x86's own `min`/`max` and conversion results, compares into masks
//! and into flags, and MXCSR's rounding and flush-to-zero (as FPCR's). MMX (the same opcodes without `66`), SSE3 and
//! later, and shifts by a register count are still unsupported stops.
//!
//! x86's scalar operations keep the rest of the destination register; ARM's zero it. So a scalar result is made in a
//! scratch register and put into lane 0.

use super::super::asm::{Cond, Shift, vec};
use super::{A, Emitter, F0, F1, Inst, Lowered, Next, Rm, S0, S1, S2, S3, Unsupported, V_TMP, ZR};

/// Scratch vector registers, besides `V_TMP` (`v31`). `V0` and `V1` are consecutive, for a two-register table.
const V0: u8 = 28;
const V1: u8 = 29;
const V2: u8 = 30;

/// Which of SSE's prefixes an instruction has: they pick the form (packed single, packed double, scalar single,
/// scalar double, or the integer forms' `66`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum P {
    None,
    P66,
    F3,
    F2,
}

fn prefix(inst: &Inst) -> P {
    if inst.rep {
        P::F3
    } else if inst.repne {
        P::F2
    } else if inst.opsize16 {
        P::P66
    } else {
        P::None
    }
}

/// The packed integer operations: their vector instruction and element size.
fn integer(op: u8) -> Option<(u32, u8)> {
    Some(match op {
        0xD4 => (vec::ADD, 3),
        0xD5 => (vec::MUL, 1),
        0xD8 => (vec::UQSUB, 0),
        0xD9 => (vec::UQSUB, 1),
        0xDA => (vec::UMIN, 0),
        0xDC => (vec::UQADD, 0),
        0xDD => (vec::UQADD, 1),
        0xDE => (vec::UMAX, 0),
        0xE0 => (vec::URHADD, 0),
        0xE3 => (vec::URHADD, 1),
        0xE8 => (vec::SQSUB, 0),
        0xE9 => (vec::SQSUB, 1),
        0xEA => (vec::SMIN, 1),
        0xEC => (vec::SQADD, 0),
        0xED => (vec::SQADD, 1),
        0xEE => (vec::SMAX, 1),
        0xF8 => (vec::SUB, 0),
        0xF9 => (vec::SUB, 1),
        0xFA => (vec::SUB, 2),
        0xFB => (vec::SUB, 3),
        0xFC => (vec::ADD, 0),
        0xFD => (vec::ADD, 1),
        0xFE => (vec::ADD, 2),
        0x64 => (vec::CMGT, 0),
        0x65 => (vec::CMGT, 1),
        0x66 => (vec::CMGT, 2),
        0x74 => (vec::CMEQ, 0),
        0x75 => (vec::CMEQ, 1),
        0x76 => (vec::CMEQ, 2),
        _ => return None,
    })
}

impl Emitter {
    /// An SSE or SSE2 instruction: a `0F` opcode in 10–17, 28–2F, 50–7F, C2, C4–C6 or D0–FF.
    pub(super) fn sse(&mut self, inst: &Inst) -> Lowered {
        let (op, p) = (inst.op, prefix(inst));
        let d = inst.reg;
        match (op, p) {
            (0x10 | 0x11 | 0x28 | 0x29, _) | (0x6F | 0x7F, P::P66 | P::F3) => {
                self.sse_move(inst)?;
            }
            (0x12 | 0x13 | 0x16 | 0x17, P::None | P::P66) => {
                self.half_move(inst)?;
            }
            (0x14 | 0x15, P::None | P::P66) => {
                let s = self.src128(inst)?;
                let base = if op == 0x14 { vec::ZIP1 } else { vec::ZIP2 };
                self.asm.v3(base, if p == P::P66 { 3 } else { 2 }, d, d, s);
            }
            (0x2A, P::F3 | P::F2) => self.int_to_float(inst, p == P::F2)?,
            (0x2B, P::None | P::P66) | (0xE7, P::P66) => {
                let Some(Rm::Mem(mem)) = inst.rm else { return Err(Unsupported) };
                self.address(&mem, inst.next(), true);
                self.asm.str_q(d, A);
            }
            (0x2C | 0x2D, P::F3 | P::F2) => self.float_to_int(inst, p == P::F2, op == 0x2C)?,
            (0x2E | 0x2F, P::None | P::P66) => {
                let double = p == P::P66;
                let s = self.src_scalar(inst, double)?;
                self.asm.fcmp(double, d, s);
                self.compare_flags();
            }
            (0x50, P::None | P::P66) => self.movmsk(inst, p == P::P66)?,
            (0x51, _) => {
                let s = self.src_for(inst, p)?;
                match p {
                    P::None | P::P66 => self.asm.v2(vec::FSQRT, u8::from(p == P::P66), d, s),
                    _ => {
                        self.asm.fsqrt(p == P::F2, V0, s);
                        self.scalar_result(p, d);
                    }
                }
            }
            (0x54..=0x57, P::None | P::P66) | (0xDB | 0xDF | 0xEB | 0xEF, P::P66) => {
                self.sse_logic(inst)?;
            }
            (0x58 | 0x59 | 0x5C | 0x5E, _) => {
                let s = self.src_for(inst, p)?;
                let (base, scalar) = match op {
                    0x58 => (vec::FADD, 2),
                    0x59 => (vec::FMUL, 0),
                    0x5C => (vec::FSUB, 3),
                    _ => (vec::FDIV, 1),
                };
                match p {
                    P::None | P::P66 => self.asm.v3(base, u8::from(p == P::P66), d, d, s),
                    _ => {
                        self.asm.f_scalar(scalar, p == P::F2, V0, d, s);
                        self.scalar_result(p, d);
                    }
                }
            }
            (0x5D | 0x5F, _) => {
                // x86's min and max: the first operand if it's strictly less (greater), else the second, which is what
                // comes back for a NaN or for two zeros. ARM's fmin and fmax answer those differently.
                let s = self.src_for(inst, p)?;
                let min = op == 0x5D;
                match p {
                    P::None | P::P66 => {
                        let size = u8::from(p == P::P66);
                        if min { self.asm.v3(vec::FCMGT, size, V0, s, d) } else { self.asm.v3(vec::FCMGT, size, V0, d, s) }
                        self.asm.bsl(V0, d, s);
                        self.asm.v_mov(d, V0);
                    }
                    _ => {
                        let double = p == P::F2;
                        self.asm.fcmp(double, d, s);
                        self.asm.fcsel(double, V0, d, s, if min { Cond::Mi } else { Cond::Gt });
                        self.scalar_result(p, d);
                    }
                }
            }
            (0x5A, _) => {
                let s = self.src_for(inst, p)?;
                match p {
                    // cvtss2sd, cvtsd2ss
                    P::F3 | P::F2 => {
                        self.asm.fcvt(p == P::F2, V0, s);
                        self.scalar_result(if p == P::F3 { P::F2 } else { P::F3 }, d);
                    }
                    // cvtps2pd: fcvtl v.2d, v.2s
                    P::None => self.asm.put(0x0E61_7800 | u32::from(s) << 5 | u32::from(d)),
                    // cvtpd2ps: fcvtn v.2s, v.2d, the high half zeroed
                    P::P66 => self.asm.put(0x0E61_6800 | u32::from(s) << 5 | u32::from(d)),
                }
            }
            (0x5B, _) => {
                let s = self.src128(inst)?;
                match p {
                    // cvtdq2ps: scvtf v.4s
                    P::None => self.asm.put(0x4E21_D800 | u32::from(s) << 5 | u32::from(d)),
                    // cvttps2dq: fcvtzs v.4s
                    P::F3 => self.asm.put(0x4EA1_B800 | u32::from(s) << 5 | u32::from(d)),
                    // cvtps2dq: rounded as MXCSR says (frinti v.4s), then converted
                    P::P66 => {
                        self.asm.put(0x6EA1_9800 | u32::from(s) << 5 | u32::from(V0));
                        self.asm.put(0x4EA1_B800 | u32::from(V0) << 5 | u32::from(d));
                    }
                    P::F2 => return Err(Unsupported),
                }
            }
            (0x60..=0x62 | 0x68..=0x6A | 0x6C | 0x6D, P::P66) => {
                let s = self.src128(inst)?;
                let size = match op & 0xF {
                    0x0 | 0x8 => 0,
                    0x1 | 0x9 => 1,
                    0x2 | 0xA => 2,
                    _ => 3,
                };
                let base = if matches!(op, 0x60..=0x62 | 0x6C) { vec::ZIP1 } else { vec::ZIP2 };
                self.asm.v3(base, size, d, d, s);
            }
            (0x63 | 0x67 | 0x6B, P::P66) => {
                // packsswb, packuswb, packssdw: the destination's elements narrowed into the low half, the source's
                // into the high half.
                let s = self.src128(inst)?;
                let (size, unsigned) = match op {
                    0x63 => (0, false),
                    0x67 => (0, true),
                    _ => (1, false),
                };
                self.asm.narrow(size, unsigned, false, V0, d);
                self.asm.narrow(size, unsigned, true, V0, s);
                self.asm.v_mov(d, V0);
            }
            (0x64..=0x66 | 0x74..=0x76 | 0xD4 | 0xD5 | 0xD8..=0xDA | 0xDC..=0xDE | 0xE0 | 0xE3 | 0xE8..=0xEA | 0xEC..=0xEE | 0xF8..=0xFE, P::P66) => {
                let (base, size) = integer(op).ok_or(Unsupported)?;
                let s = self.src128(inst)?;
                self.asm.v3(base, size, d, d, s);
            }
            // Shifts by a count in a register (its low 64 bits): psrlw/d/q, psraw/d, psllw/d/q. A count past the width
            // empties each element, or fills it with its sign.
            (0xD1..=0xD3 | 0xE1 | 0xE2 | 0xF1..=0xF3, P::P66) => {
                let s = self.src128(inst)?;
                let size = op & 3;
                let width = 8u64 << size;
                let a = &mut self.asm;
                // x86's flags live in NZCV, which the clamp would change.
                a.mrs_nzcv(F0);
                a.fmov_x_d(S0, s);
                a.mov_imm(S1, width);
                a.subs(true, ZR, S0, S1);
                a.csel(true, S0, S1, S0, Cond::Hi);
                a.msr_nzcv(F0);
                if op < 0xF0 {
                    a.sub(true, S0, ZR, S0);
                }
                a.dup_gen(size, V0, S0);
                a.v3(if op & 0xF0 == 0xE0 { vec::SSHL } else { vec::USHL }, size, d, d, V0);
            }
            // pmulhuw and pmulhw: each 16-bit product's high half.
            (0xE4 | 0xE5, P::P66) => {
                let s = self.src128(inst)?;
                let signed = op == 0xE5;
                self.asm.mull(signed, false, 1, V0, d, s);
                self.asm.mull(signed, true, 1, V1, d, s);
                self.asm.v3(vec::UZP2, 1, d, V0, V1);
            }
            // pmaddwd: signed 16-bit products, each adjacent pair added into a 32-bit sum.
            (0xF5, P::P66) => {
                let s = self.src128(inst)?;
                self.asm.mull(true, false, 1, V0, d, s);
                self.asm.mull(true, true, 1, V1, d, s);
                self.asm.v3(vec::ADDP, 2, d, V0, V1);
            }
            // psadbw: each 8-byte half's absolute differences summed into its low 16 bits.
            (0xF6, P::P66) => {
                let s = self.src128(inst)?;
                self.asm.v3(vec::UABD, 0, V0, d, s);
                self.asm.uaddlp(0, V0, V0);
                self.asm.uaddlp(1, V0, V0);
                self.asm.uaddlp(2, d, V0);
            }
            // pmuludq: the low unsigned 32 bits of each quadword, multiplied into a quadword.
            (0xF4, P::P66) => {
                let s = self.src128(inst)?;
                self.asm.xtn_2s(V0, d);
                self.asm.xtn_2s(V1, s);
                self.asm.umull_2d(d, V0, V1);
            }
            (0x6E | 0x7E | 0xD6 | 0xD7, _) => {
                self.sse_gpr(inst)?;
            }
            (0x70, P::P66 | P::F3 | P::F2) => self.shuffle(inst, p)?,
            (0x71..=0x73, P::P66) => self.shift_imm(inst)?,
            (0xC2, _) => {
                let s = self.src_for(inst, p)?;
                let size = u8::from(matches!(p, P::P66 | P::F2));
                self.compare_mask(size, (inst.imm & 7) as u8, d, s);
                match p {
                    P::None | P::P66 => self.asm.v_mov(d, V0),
                    _ => self.scalar_result(p, d),
                }
            }
            (0xC4, P::P66) => {
                // pinsrw xmm, r32/m16, imm
                let src = self.rm(inst, 16)?;
                self.load(src, 16, S0);
                self.asm.ins_gen(1, d, (inst.imm & 7) as u8, S0);
            }
            (0xC5, P::P66) => {
                // pextrw r32, xmm, imm
                let Some(Rm::Reg(n)) = inst.rm else { return Err(Unsupported) };
                self.asm.umov(1, d, n, (inst.imm & 7) as u8);
            }
            (0xC6, P::None | P::P66) => self.shufp(inst, p == P::P66)?,
            (0xF0, P::F2) => {
                // lddqu
                let Some(Rm::Mem(mem)) = inst.rm else { return Err(Unsupported) };
                self.address(&mem, inst.next(), true);
                self.asm.ldr_q(d, A);
            }
            _ => return Err(Unsupported),
        }
        Ok(Next::Continue)
    }

    /// The full 128-bit source: its register, or the memory operand loaded into `V_TMP`.
    fn src128(&mut self, inst: &Inst) -> Result<u8, Unsupported> {
        Ok(match self.xmm_rm(inst)? {
            Some(n) => n,
            None => {
                self.asm.ldr_q(V_TMP, A);
                V_TMP
            }
        })
    }

    /// A scalar source: its register, or 4 or 8 bytes of memory loaded into `V_TMP`.
    fn src_scalar(&mut self, inst: &Inst, double: bool) -> Result<u8, Unsupported> {
        Ok(match self.xmm_rm(inst)? {
            Some(n) => n,
            None => {
                if double { self.asm.ldr_d(V_TMP, A) } else { self.asm.ldr_s(V_TMP, A) }
                V_TMP
            }
        })
    }

    /// The source as the prefix says: scalar for `F3` and `F2`, else all of it.
    fn src_for(&mut self, inst: &Inst, p: P) -> Result<u8, Unsupported> {
        match p {
            P::F3 => self.src_scalar(inst, false),
            P::F2 => self.src_scalar(inst, true),
            _ => self.src128(inst),
        }
    }

    /// A scalar result in `V0` into lane 0 of `d`, its other lanes kept.
    fn scalar_result(&mut self, p: P, d: u8) {
        if p == P::F2 { self.asm.ins_d0(d, V0) } else { self.asm.ins_s0(d, V0) }
    }

    /// 16 constant bytes into `vd`.
    fn constant(&mut self, vd: u8, bytes: [u8; 16]) {
        let (lo, hi) = (u64::from_le_bytes(bytes[..8].try_into().unwrap_or_default()), u64::from_le_bytes(bytes[8..].try_into().unwrap_or_default()));
        self.asm.mov_imm(S0, lo);
        self.asm.fmov_d_x(vd, S0);
        self.asm.mov_imm(S0, hi);
        self.asm.ins_gen(3, vd, 1, S0);
    }

    /// The flags `ucomiss`, `ucomisd`, `comiss` and `comisd` leave, after an `fcmp`: ZF, PF and CF all set for an
    /// unordered pair, CF for less, ZF for equal, none for greater; SF and OF clear. PF is kept in ARM's V, since OF
    /// is always clear here and nothing else keeps PF: so a `jp` or `jnp` after these is right, and only after these.
    pub(super) fn compare_flags(&mut self) {
        let a = &mut self.asm;
        a.cset(false, S0, Cond::Ge); // ARM's C: not CF
        a.cset(false, S1, Cond::Vs); // unordered: PF
        a.cset(false, S2, Cond::Eq);
        a.orr(false, S2, S2, S1); // ZF
        a.lsl_imm(false, S3, S2, 30);
        a.orr_shift(false, S3, S3, S0, Shift::Lsl, 29);
        a.orr_shift(false, S3, S3, S1, Shift::Lsl, 28);
        a.msr_nzcv(S3);
    }

    /// `cmpps`'s predicates into `V0`: all ones where it holds. 0 eq, 1 lt, 2 le, 3 unordered, 4 neq, 5 nlt, 6 nle,
    /// 7 ordered.
    fn compare_mask(&mut self, size: u8, predicate: u8, d: u8, s: u8) {
        let a = &mut self.asm;
        match predicate & 3 {
            0 => a.v3(vec::FCMEQ, size, V0, d, s),
            1 => a.v3(vec::FCMGT, size, V0, s, d),
            2 => a.v3(vec::FCMGE, size, V0, s, d),
            _ => {
                // Ordered: each is equal to itself.
                a.v3(vec::FCMEQ, size, V0, d, d);
                a.v3(vec::FCMEQ, size, V1, s, s);
                a.v_and(V0, V0, V1);
                // Unordered is its inverse, like 4 to 6 are 0 to 2's.
                if predicate == 3 {
                    a.v_not(V0, V0);
                }
                return;
            }
        }
        if predicate >= 4 {
            a.v_not(V0, V0);
        }
    }

    /// `cvtsi2ss` and `cvtsi2sd`: a 32- or 64-bit signed integer into lane 0.
    fn int_to_float(&mut self, inst: &Inst, double: bool) -> Result<(), Unsupported> {
        let size = if inst.w() { 64 } else { 32 };
        let src = self.rm(inst, size)?;
        self.load(src, size, S0);
        self.asm.scvtf(size == 64, double, V0, S0);
        self.scalar_result(if double { P::F2 } else { P::F3 }, inst.reg);
        Ok(())
    }

    /// `cvttss2si`, `cvttsd2si` (`truncate`), `cvtss2si` and `cvtsd2si` (rounded as MXCSR says): into a 32- or
    /// 64-bit register. A NaN or an out-of-range value gives x86's "integer indefinite", the lowest value, where ARM
    /// would saturate. The flags are kept.
    fn float_to_int(&mut self, inst: &Inst, double: bool, truncate: bool) -> Result<(), Unsupported> {
        let mut s = self.src_scalar(inst, double)?;
        let size64 = inst.w();
        if !truncate {
            self.asm.frinti(double, V0, s);
            s = V0;
        }
        let a = &mut self.asm;
        a.mrs_nzcv(F0);
        // Always to 64 bits first, so a 32-bit result out of range can be seen.
        a.fcvtzs(true, double, S0, s);
        a.mov_imm(S1, if size64 { 1 << 63 } else { 0x8000_0000 });
        let out_of_range = if size64 {
            // Saturated high means it was too big; saturated low is already indefinite.
            a.mov_imm(S2, i64::MAX as u64);
            a.subs(true, ZR, S0, S2);
            Cond::Eq
        } else {
            // Out of a 32-bit register's range if it isn't its own low half, sign-extended.
            a.sbfx(true, S2, S0, 0, 32);
            a.subs(true, ZR, S0, S2);
            Cond::Ne
        };
        a.csel(true, S0, S1, S0, out_of_range);
        a.fcmp(double, s, s);
        a.csel(true, S0, S1, S0, Cond::Vs);
        a.msr_nzcv(F0);
        a.mov(size64, inst.reg, S0);
        Ok(())
    }

    /// `movmskps`, `movmskpd`: each element's sign bit, into a general register.
    fn movmsk(&mut self, inst: &Inst, double: bool) -> Result<(), Unsupported> {
        let Some(Rm::Reg(n)) = inst.rm else { return Err(Unsupported) };
        let a = &mut self.asm;
        if double {
            a.ushr(3, V0, n, 63);
            a.umov(3, S0, V0, 0);
            a.umov(3, S1, V0, 1);
            a.orr_shift(true, S0, S0, S1, Shift::Lsl, 1);
        } else {
            a.ushr(2, V0, n, 31);
            a.umov(2, S0, V0, 0);
            for lane in 1..4 {
                a.umov(2, S1, V0, lane);
                a.orr_shift(false, S0, S0, S1, Shift::Lsl, lane);
            }
        }
        a.mov(false, inst.reg, S0);
        Ok(())
    }

    /// `movlps`/`movlpd` (12, 13) and `movhps`/`movhpd` (16, 17) with memory; `movhlps` (12) and `movlhps` (16)
    /// between registers.
    fn half_move(&mut self, inst: &Inst) -> Lowered {
        let d = inst.reg;
        match (inst.op, inst.rm) {
            (0x12, Some(Rm::Reg(n))) if !inst.opsize16 => self.asm.ins_elem(3, d, 0, n, 1),
            (0x16, Some(Rm::Reg(n))) if !inst.opsize16 => self.asm.ins_elem(3, d, 1, n, 0),
            (0x12 | 0x16, Some(Rm::Mem(mem))) => {
                self.address(&mem, inst.next(), true);
                self.asm.ldr_d(V_TMP, A);
                self.asm.ins_elem(3, d, u8::from(inst.op == 0x16), V_TMP, 0);
            }
            (0x13 | 0x17, Some(Rm::Mem(mem))) => {
                self.address(&mem, inst.next(), true);
                if inst.op == 0x17 {
                    self.asm.ins_elem(3, V_TMP, 0, d, 1);
                    self.asm.str_d(V_TMP, A);
                } else {
                    self.asm.str_d(d, A);
                }
            }
            _ => return Err(Unsupported),
        }
        Ok(Next::Continue)
    }

    /// `pshufd` (66), `pshufhw` (F3), `pshuflw` (F2): elements picked by the immediate, through a byte table.
    fn shuffle(&mut self, inst: &Inst, p: P) -> Result<(), Unsupported> {
        let s = self.src128(inst)?;
        let imm = inst.imm as u8;
        let pick = |i: usize| usize::from(imm >> (2 * i) & 3);
        let mut index: [u8; 16] = core::array::from_fn(|b| b as u8);
        match p {
            P::P66 => {
                for (b, byte) in index.iter_mut().enumerate() {
                    *byte = (pick(b / 4) * 4 + b % 4) as u8;
                }
            }
            P::F2 => {
                for (b, byte) in index[..8].iter_mut().enumerate() {
                    *byte = (pick(b / 2) * 2 + b % 2) as u8;
                }
            }
            _ => {
                for (b, byte) in index[8..].iter_mut().enumerate() {
                    *byte = (8 + pick(b / 2) * 2 + b % 2) as u8;
                }
            }
        }
        self.constant(V2, index);
        self.asm.tbl(inst.reg, s, false, V2);
        Ok(())
    }

    /// `shufps` and `shufpd`: the low half of the result from the destination, the high half from the source.
    fn shufp(&mut self, inst: &Inst, double: bool) -> Result<(), Unsupported> {
        let s = self.src128(inst)?;
        let imm = inst.imm as u8;
        let d = inst.reg;
        let mut index = [0u8; 16];
        for (b, byte) in index.iter_mut().enumerate() {
            *byte = if double {
                let lane = b / 8;
                let pick = usize::from(imm >> lane & 1);
                (lane * 16 + pick * 8 + b % 8) as u8
            } else {
                let lane = b / 4;
                let pick = usize::from(imm >> (2 * lane) & 3);
                ((lane / 2) * 16 + pick * 4 + b % 4) as u8
            };
        }
        // A two-register table: the destination's bytes, then the source's.
        self.asm.v_mov(V0, d);
        self.asm.v_mov(V1, s);
        self.constant(V2, index);
        self.asm.tbl(d, V0, true, V2);
        Ok(())
    }

    /// `66 0F 71`–`73`: words, doublewords and quadwords shifted by an immediate, and whole registers by bytes.
    fn shift_imm(&mut self, inst: &Inst) -> Result<(), Unsupported> {
        let Some(Rm::Reg(n)) = inst.rm else { return Err(Unsupported) };
        let count = (inst.imm as u64 & 0xFF) as u8;
        let size = inst.op - 0x71 + 1;
        let width = 8u8 << size;
        let a = &mut self.asm;
        match (inst.op, inst.ext) {
            // psrlw, psrld, psrlq; psllw, pslld, psllq
            (_, 2 | 6) => {
                if count >= width {
                    a.v_eor(n, n, n);
                } else if count > 0 {
                    if inst.ext == 2 { a.ushr(size, n, n, count) } else { a.shl(size, n, n, count) }
                }
            }
            // psraw, psrad: past the width, every bit is the sign
            (0x71 | 0x72, 4) => {
                if count > 0 {
                    a.sshr(size, n, n, count.min(width));
                }
            }
            // psrldq, pslldq: by whole bytes
            (0x73, 3 | 7) => {
                if count >= 16 {
                    a.v_eor(n, n, n);
                } else if count > 0 {
                    a.v_eor(V0, V0, V0);
                    if inst.ext == 3 { a.ext_16b(n, n, V0, count) } else { a.ext_16b(n, V0, n, 16 - count) }
                }
            }
            _ => return Err(Unsupported),
        }
        Ok(())
    }

    /// `ldmxcsr` (`/2`) and `stmxcsr` (`/3`): MXCSR's rounding and flush-to-zero, as ARM's FPCR has them. Its
    /// exception flags and masks aren't kept: it reads back as all masked, nothing raised.
    pub(super) fn mxcsr(&mut self, inst: &Inst) -> Lowered {
        let Some(Rm::Mem(mem)) = inst.rm else { return Err(Unsupported) };
        self.address(&mem, inst.next(), true);
        self.mxcsr_at(inst.ext == 3);
        Ok(Next::Continue)
    }

    /// MXCSR stored at `x17` (`store`), or loaded from it: `stmxcsr` and `ldmxcsr`, and part of `fxsave`/`fxrstor`.
    pub(super) fn mxcsr_at(&mut self, store: bool) {
        let a = &mut self.asm;
        // ARM's rounding mode is 0 nearest, 1 up, 2 down, 3 toward zero; x86's has up and down the other way round.
        if store {
            a.mrs_fpcr(S0);
            a.ubfx(true, S1, S0, 22, 1);
            a.ubfx(true, S2, S0, 23, 1);
            a.ubfx(true, S3, S0, 24, 1);
            a.mov_imm(S0, 0x1F80);
            a.orr_shift(false, S0, S0, S2, Shift::Lsl, 13);
            a.orr_shift(false, S0, S0, S1, Shift::Lsl, 14);
            a.orr_shift(false, S0, S0, S3, Shift::Lsl, 15);
            a.str(32, S0, A);
        } else {
            a.ldr(32, S0, A);
            a.ubfx(true, S1, S0, 13, 1);
            a.ubfx(true, S2, S0, 14, 1);
            a.ubfx(true, S3, S0, 15, 1);
            a.mrs_fpcr(F1);
            a.bfi(true, F1, S2, 22, 1);
            a.bfi(true, F1, S1, 23, 1);
            a.bfi(true, F1, S3, 24, 1);
            a.msr_fpcr(F1);
        }
    }
}
