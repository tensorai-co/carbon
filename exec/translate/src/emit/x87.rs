//! x87 on ARM64's floating-point registers, at double precision.
//!
//! # Where x87 lives
//!
//! ST(0) to ST(7) are `d16`–`d23`, always in that order: a push moves each register up one and a pop down one, so TOP
//! is always 0 and every instruction knows its registers when it's translated. `v24` holds the rest: its halfword 0 is
//! the control word XORed with 0x37F (so a register that's never been written reads as the default control word), and
//! halfword 1 the status word's condition bits, C0 to C3. Core puts all of this into the `fxsave` image a kernel sees.
//!
//! # What's different from an x87
//!
//! Values are doubles, not 80-bit extended numbers: loads and stores of the 80-bit format convert (rounding to nearest
//! on the way in), and arithmetic rounds to 53 bits, as an x87 set to double precision does (Windows' default). The
//! control word's rounding control is honoured where it matters to integers (`fist`, `frndint`); the rest round as
//! MXCSR says. Exceptions are never raised and their flags never set, tags aren't kept (`fxam` never says "empty"),
//! and the status word's TOP is always 0. The transcendental instructions (`fsin`, `fpatan`, `fyl2x`, `f2xm1`, …),
//! `fprem1`, `fxtract`, `fsave`/`frstor` and the BCD forms are still unsupported stops.
//!
//! `fxsave` and `fxrstor` (with SSE's state too) are here: glibc's lazy binding saves and restores every register
//! with them around the resolver.

use super::super::asm::{Cond, Shift};
use super::{A, Emitter, F0, F1, F2, Inst, Lowered, Next, R, RAX, Rm, S0, S1, S2, S3, T, Unsupported, V_TMP, ZR, cond};

/// ST(0); ST(i) is `ST0 + i`.
const ST0: u8 = 16;
/// The control word (XOR 0x37F) in halfword 0, the condition bits in halfword 1.
const CTL: u8 = 24;
const FCW_DEFAULT: u64 = 0x37F;
/// Scratch, as SSE's are.
const V0: u8 = 28;
const V1: u8 = 29;
const V2: u8 = 30;
const V3: u8 = V_TMP;

const C0: u64 = 1 << 8;
const C1: u64 = 1 << 9;
const C2: u64 = 1 << 10;
const C3: u64 = 1 << 14;

/// `f_scalar`'s operations.
const FMUL: u8 = 0;
const FDIV: u8 = 1;
const FADD: u8 = 2;
const FSUB: u8 = 3;
/// `f_unary`'s.
const FABS: u8 = 1;
const FNEG: u8 = 2;
const FRINTN: u8 = 8;
const FRINTP: u8 = 9;
const FRINTM: u8 = 10;
const FRINTZ: u8 = 11;

fn st(i: u8) -> u8 {
    ST0 + (i & 7)
}

/// A memory operand's format.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Single,
    Double,
    Extended,
    Int16,
    Int32,
    Int64,
}

impl Emitter {
    /// `D8`–`DF`.
    pub(super) fn x87(&mut self, inst: &Inst) -> Lowered {
        match inst.rm {
            Some(Rm::Mem(mem)) => {
                self.address(&mem, inst.next(), true);
                self.x87_mem(inst.op, inst.ext)?;
            }
            Some(Rm::Reg(num)) => self.x87_reg(inst.op, inst.ext, num & 7)?,
            None => return Err(Unsupported),
        }
        Ok(Next::Continue)
    }

    fn x87_mem(&mut self, op: u8, ext: u8) -> Result<(), Unsupported> {
        match (op, ext) {
            // Arithmetic with ST(0) and a float or an integer from memory.
            (0xD8 | 0xDA | 0xDC | 0xDE, _) => {
                let format = match op {
                    0xD8 => Format::Single,
                    0xDA => Format::Int32,
                    0xDC => Format::Double,
                    _ => Format::Int16,
                };
                self.load_value(format);
                self.with_st0(ext, V0);
            }
            (0xD9, 0) => self.fld(Format::Single),
            (0xDD, 0) => self.fld(Format::Double),
            (0xDB, 5) => self.fld(Format::Extended),
            (0xDF, 0) => self.fld(Format::Int16),
            (0xDB, 0) => self.fld(Format::Int32),
            (0xDF, 5) => self.fld(Format::Int64),
            (0xD9, 2 | 3) => self.fst(Format::Single, ext == 3),
            (0xDD, 2 | 3) => self.fst(Format::Double, ext == 3),
            (0xDB, 7) => self.fst(Format::Extended, true),
            (0xDF, 2 | 3) => self.fist(16, ext == 3, false),
            (0xDB, 2 | 3) => self.fist(32, ext == 3, false),
            (0xDF, 7) => self.fist(64, true, false),
            // fisttp: always toward zero, and popped.
            (0xDF, 1) => self.fist(16, true, true),
            (0xDB, 1) => self.fist(32, true, true),
            (0xDD, 1) => self.fist(64, true, true),
            (0xD9, 5) => {
                // fldcw
                let a = &mut self.asm;
                a.ldr(16, S0, A);
                a.mov_imm(S1, FCW_DEFAULT);
                a.eor(false, S0, S0, S1);
                a.ins_gen(1, CTL, 0, S0);
            }
            (0xD9, 7) => {
                // fnstcw
                self.control_word(S0);
                self.asm.str(16, S0, A);
            }
            (0xDD, 7) => {
                // fnstsw
                self.asm.umov(1, S0, CTL, 1);
                self.asm.str(16, S0, A);
            }
            (0xD9, 6) => self.fnstenv(),
            (0xD9, 4) => {
                // fldenv: the control word, and the status word's condition bits.
                let a = &mut self.asm;
                a.ldr(16, S0, A);
                a.mov_imm(S1, FCW_DEFAULT);
                a.eor(false, S0, S0, S1);
                a.ins_gen(1, CTL, 0, S0);
                a.add_imm(true, T, A, 4);
                a.ldr(16, S0, T);
                a.mov_imm(S1, C0 | C1 | C2 | C3);
                a.and(false, S0, S0, S1);
                a.ins_gen(1, CTL, 1, S0);
            }
            _ => return Err(Unsupported),
        }
        Ok(())
    }

    fn x87_reg(&mut self, op: u8, ext: u8, i: u8) -> Result<(), Unsupported> {
        let si = st(i);
        match (op, ext) {
            // ST(0) = ST(0) op ST(i); fcom and fcomp.
            (0xD8, _) => self.with_st0(ext, si),
            // ST(i) = ST(i) op ST(0), and popped for DE. Here x86 names the subtractions and divisions the other way
            // round: E0+i is fsubr, E8+i fsub, F0+i fdivr, F8+i fdiv.
            (0xDC | 0xDE, 0 | 1 | 4..=7) => {
                let kind = match ext {
                    4 => 5,
                    5 => 4,
                    6 => 7,
                    7 => 6,
                    k => k,
                };
                self.arith(kind, si, si, ST0);
                if op == 0xDE {
                    self.st_pop();
                }
            }
            (0xDC, 2 | 3) => self.with_st0(ext, si),
            // fcompp
            (0xDE, 3) if i == 1 => {
                self.fcom(ST0, st(1));
                self.st_pop();
                self.st_pop();
            }
            (0xD9, 0) => {
                // fld st(i)
                self.asm.v_mov(V0, si);
                self.st_push(V0);
            }
            (0xD9, 1) => {
                // fxch
                self.asm.v_mov(V0, ST0);
                self.asm.v_mov(ST0, si);
                self.asm.v_mov(si, V0);
            }
            // fnop
            (0xD9, 2) if i == 0 => {}
            // fst st(i), fstp st(i)
            (0xDD, 2) => self.asm.v_mov(si, ST0),
            (0xDD | 0xD9, 3) => {
                self.asm.v_mov(si, ST0);
                self.st_pop();
            }
            // ffree: tags aren't kept. ffreep: freed and popped.
            (0xDD, 0) => {}
            (0xDF, 0) => self.st_pop(),
            (0xD9, 4) => match i {
                0 => self.asm.f_unary(FNEG, true, ST0, ST0),
                1 => self.asm.f_unary(FABS, true, ST0, ST0),
                4 => {
                    // ftst
                    self.asm.fmov_d_x(V0, ZR);
                    self.fcom(ST0, V0);
                }
                5 => self.fxam(),
                _ => return Err(Unsupported),
            },
            (0xD9, 5) => {
                // fld1, fldl2t, fldl2e, fldpi, fldlg2, fldln2, fldz
                let bits: u64 = match i {
                    0 => 0x3FF0_0000_0000_0000,
                    1 => 0x400A_934F_0979_A371,
                    2 => 0x3FF7_1547_652B_82FE,
                    3 => 0x4009_21FB_5444_2D18,
                    4 => 0x3FD3_4413_509F_79FF,
                    5 => 0x3FE6_2E42_FEFA_39EF,
                    6 => 0,
                    _ => return Err(Unsupported),
                };
                if bits == 0 {
                    self.asm.fmov_d_x(V0, ZR);
                } else {
                    self.asm.mov_imm(S0, bits);
                    self.asm.fmov_d_x(V0, S0);
                }
                self.st_push(V0);
            }
            (0xD9, 7) => match i {
                0 => self.fprem(),
                2 => self.asm.fsqrt(true, ST0, ST0),
                4 => self.round_rc(ST0, ST0),
                5 => self.fscale(),
                _ => return Err(Unsupported),
            },
            // fcmovb, fcmove, fcmovbe, fcmovu; and for DB, their negations.
            (0xDA | 0xDB, 0..=3) => {
                let cc = [2, 4, 6, 10][usize::from(ext)] + u8::from(op == 0xDB);
                let c = cond(cc).ok_or(Unsupported)?;
                self.asm.fcsel(true, ST0, si, ST0, c);
            }
            // fucompp
            (0xDA, 5) if i == 1 => {
                self.fcom(ST0, st(1));
                self.st_pop();
                self.st_pop();
            }
            // fnclex: no exception flags are kept. feni, fdisi and fsetpm do nothing on later x87s.
            (0xDB, 4) if matches!(i, 0 | 1 | 2 | 4) => {}
            // fninit
            (0xDB, 4) if i == 3 => self.asm.ins_gen(2, CTL, 0, ZR),
            // fucomi, fcomi; fucomip, fcomip.
            (0xDB | 0xDF, 5 | 6) => {
                self.asm.fcmp(true, ST0, si);
                self.compare_flags();
                if op == 0xDF {
                    self.st_pop();
                }
            }
            // fucom, fucomp.
            (0xDD, 4 | 5) => {
                self.fcom(ST0, si);
                if ext == 5 {
                    self.st_pop();
                }
            }
            // fnstsw ax
            (0xDF, 4) if i == 0 => {
                self.asm.umov(1, S0, CTL, 1);
                self.asm.bfi(true, RAX, S0, 0, 16);
            }
            _ => return Err(Unsupported),
        }
        Ok(())
    }

    // ---- The stack ----

    /// Pushes `v`, which mustn't be one of the stack's own registers.
    fn st_push(&mut self, v: u8) {
        for i in (1..8).rev() {
            self.asm.v_mov(st(i), st(i - 1));
        }
        self.asm.v_mov(ST0, v);
    }

    fn st_pop(&mut self) {
        for i in 0..7 {
            self.asm.v_mov(st(i), st(i + 1));
        }
    }

    // ---- Memory ----

    fn fld(&mut self, format: Format) {
        self.load_value(format);
        self.st_push(V0);
    }

    /// A memory operand at `x17`, into `V0`.
    fn load_value(&mut self, format: Format) {
        let a = &mut self.asm;
        match format {
            Format::Single => {
                a.ldr_s(V0, A);
                a.fcvt(false, V0, V0);
            }
            Format::Double => a.ldr_d(V0, A),
            Format::Extended => self.load_extended(),
            Format::Int16 | Format::Int32 => {
                a.ldrs(if format == Format::Int16 { 16 } else { 32 }, S0, A);
                a.scvtf(true, true, V0, S0);
            }
            Format::Int64 => {
                a.ldr(64, S0, A);
                a.scvtf(true, true, V0, S0);
            }
        }
    }

    fn fst(&mut self, format: Format, pop: bool) {
        let a = &mut self.asm;
        match format {
            Format::Single => {
                a.fcvt(true, V0, ST0);
                a.str_s(V0, A);
            }
            Format::Double => a.str_d(ST0, A),
            _ => self.store_extended(ST0),
        }
        if pop {
            self.st_pop();
        }
    }

    /// `fist`, `fistp` and `fisttp` (`truncate`): ST(0) as a `size`-bit integer, rounded as the control word says. A
    /// NaN or a value out of range gives x86's "integer indefinite", the lowest value.
    fn fist(&mut self, size: u8, pop: bool, truncate: bool) {
        let src = if truncate {
            ST0
        } else {
            self.round_rc(V0, ST0);
            V0
        };
        let a = &mut self.asm;
        a.mrs_nzcv(F0);
        a.fcvtzs(true, true, S0, src);
        a.mov_imm(S1, 1 << (size - 1));
        let out_of_range = if size == 64 {
            a.mov_imm(S2, i64::MAX as u64);
            a.subs(true, ZR, S0, S2);
            Cond::Eq
        } else {
            a.sbfx(true, S2, S0, 0, size);
            a.subs(true, ZR, S0, S2);
            Cond::Ne
        };
        a.csel(true, S0, S1, S0, out_of_range);
        a.fcmp(true, src, src);
        a.csel(true, S0, S1, S0, Cond::Vs);
        a.msr_nzcv(F0);
        a.str(size, S0, A);
        if pop {
            self.st_pop();
        }
    }

    /// The 80-bit number at `x17`, into `V0`, rounded to a double: the 64-bit significand converted as an integer
    /// (one rounding), then scaled by its exponent in two exact steps. Exponents past a double's range are clamped
    /// first, where they still overflow to infinity or vanish to zero.
    fn load_extended(&mut self) {
        let a = &mut self.asm;
        a.ldr(64, S0, A);
        a.add_imm(true, T, A, 8);
        a.ldr(16, S1, T);
        a.mrs_nzcv(F0);
        a.ubfx(true, S2, S1, 0, 15);
        // The value is significand * 2^(exponent - 16383 - 63).
        a.mov_imm(F1, 16446);
        a.sub(true, F1, S2, F1);
        a.mov_imm(T, (-1140i64) as u64);
        a.subs(true, ZR, F1, T);
        a.csel(true, F1, T, F1, Cond::Lt);
        a.mov_imm(T, 970);
        a.subs(true, ZR, F1, T);
        a.csel(true, F1, T, F1, Cond::Gt);
        a.asr_imm(true, S3, F1, 1);
        a.sub(true, F1, F1, S3);
        a.ucvtf(true, true, V0, S0);
        for k in [S3, F1] {
            a.add_imm(true, k, k, 1023);
            a.lsl_imm(true, k, k, 52);
            a.fmov_d_x(V1, k);
            a.f_scalar(FMUL, true, V0, V0, V1);
        }
        // Infinity, or a NaN that keeps its payload's top bits and is quiet.
        a.lsl_imm(true, S3, S0, 1);
        a.cmp_imm(true, S3, 0);
        a.mov_imm(T, 0x7FF8_0000_0000_0000);
        a.mov_imm(F1, 0x7FF0_0000_0000_0000);
        a.csel(true, T, T, F1, Cond::Ne);
        a.orr_shift(true, T, T, S3, Shift::Lsr, 12);
        a.fmov_d_x(V1, T);
        a.mov_imm(T, 0x7FFF);
        a.subs(true, ZR, S2, T);
        a.fcsel(true, V0, V1, V0, Cond::Eq);
        // The sign, last, so zeros keep theirs.
        a.fmov_x_d(T, V0);
        a.lsr_imm(true, S1, S1, 15);
        a.orr_shift(true, T, T, S1, Shift::Lsl, 63);
        a.fmov_d_x(V0, T);
        a.msr_nzcv(F0);
    }

    /// `v` as an 80-bit number at `x17`: exact. The significand (with a double's implicit bit, or a denormal's
    /// leading zeros shifted out) goes to the top of 64 bits.
    fn store_extended(&mut self, v: u8) {
        let a = &mut self.asm;
        a.fmov_x_d(S0, v);
        a.mrs_nzcv(F0);
        a.ubfx(true, S1, S0, 52, 11);
        a.ubfx(true, S2, S0, 0, 52);
        a.cmp_imm(true, S1, 0);
        a.cset(true, F1, Cond::Ne);
        a.orr_shift(true, S2, S2, F1, Shift::Lsl, 52);
        // A denormal's exponent is the smallest normal one's.
        a.csinc(true, S3, S1, ZR, Cond::Ne);
        a.clz(true, F2, S2);
        a.lslv(true, R, S2, F2);
        a.mov_imm(T, 15371);
        a.add(true, S3, S3, T);
        a.sub(true, S3, S3, F2);
        a.mov_imm(T, 0x7FF);
        a.subs(true, ZR, S1, T);
        a.mov_imm(T, 0x7FFF);
        a.csel(true, S3, T, S3, Cond::Eq);
        a.cmp_imm(true, S2, 0);
        a.csel(true, S3, ZR, S3, Cond::Eq);
        a.lsr_imm(true, S0, S0, 63);
        a.orr_shift(true, S3, S3, S0, Shift::Lsl, 15);
        a.str(64, R, A);
        a.add_imm(true, T, A, 8);
        a.str(16, S3, T);
        a.msr_nzcv(F0);
    }

    /// `fxsave` (`restore` false) and `fxrstor`: the 512-byte image at the memory operand. In it: the control word, the
    /// status word's condition bits, no tags or last instruction, MXCSR (and a mask of every bit), ST(0) to ST(7) as
    /// 80-bit numbers (each exact, being a double), and `xmm0`–`xmm15`. What `fxsave` leaves out it leaves as it was.
    pub(super) fn fxsave(&mut self, inst: &Inst, restore: bool) -> Lowered {
        let Some(Rm::Mem(mem)) = inst.rm else { return Err(Unsupported) };
        self.address(&mem, inst.next(), true);
        // `x17` walks through the image: each part is reached by adding to it.
        let step = |a: &mut super::super::asm::Asm, by: u16| a.add_imm(true, A, A, by);
        if restore {
            let a = &mut self.asm;
            a.ldr(16, S0, A);
            a.mov_imm(S1, FCW_DEFAULT);
            a.eor(false, S0, S0, S1);
            a.ins_gen(1, CTL, 0, S0);
            a.add_imm(true, T, A, 2);
            a.ldr(16, S0, T);
            a.mov_imm(S1, C0 | C1 | C2 | C3);
            a.and(false, S0, S0, S1);
            a.ins_gen(1, CTL, 1, S0);
            step(a, 24);
            self.mxcsr_at(false);
            step(&mut self.asm, 8);
            for i in 0..8 {
                self.load_extended();
                self.asm.v_mov(st(i), V0);
                step(&mut self.asm, 16);
            }
            for x in 0..16 {
                self.asm.ldr_q(x, A);
                step(&mut self.asm, 16);
            }
        } else {
            self.control_word(S0);
            let a = &mut self.asm;
            a.umov(1, S1, CTL, 1);
            a.orr_shift(false, S0, S0, S1, Shift::Lsl, 16);
            a.str(32, S0, A);
            for at in [4, 8, 12, 16, 20] {
                a.add_imm(true, T, A, at);
                a.str(32, ZR, T);
            }
            step(a, 24);
            self.mxcsr_at(true);
            let a = &mut self.asm;
            a.mov_imm(S0, 0xFFFF);
            a.add_imm(true, T, A, 4);
            a.str(32, S0, T);
            step(a, 8);
            for i in 0..8 {
                self.store_extended(st(i));
                let a = &mut self.asm;
                a.add_imm(true, T, A, 10);
                a.str(16, ZR, T);
                a.add_imm(true, T, A, 12);
                a.str(32, ZR, T);
                step(a, 16);
            }
            for x in 0..16 {
                self.asm.str_q(x, A);
                step(&mut self.asm, 16);
            }
        }
        Ok(Next::Continue)
    }

    /// `fnstenv`, in the 28-byte protected-mode layout: the control and status words, every register tagged empty,
    /// and no last instruction or operand. Then every exception is masked, as an x87 does.
    fn fnstenv(&mut self) {
        self.control_word(S0);
        let a = &mut self.asm;
        a.str(32, S0, A);
        a.umov(1, S1, CTL, 1);
        a.add_imm(true, T, A, 4);
        a.str(32, S1, T);
        a.mov_imm(S1, 0xFFFF);
        a.add_imm(true, T, A, 8);
        a.str(32, S1, T);
        for at in (12..28).step_by(4) {
            a.add_imm(true, T, A, at);
            a.str(32, ZR, T);
        }
        // Masked is set in the control word, which is clear in the XORed one.
        a.umov(1, S0, CTL, 0);
        a.mov_imm(S1, 0xFFC0);
        a.and(false, S0, S0, S1);
        a.ins_gen(1, CTL, 0, S0);
    }

    /// The control word, into `rd`.
    fn control_word(&mut self, rd: u8) {
        self.asm.umov(1, rd, CTL, 0);
        self.asm.mov_imm(S3, FCW_DEFAULT);
        self.asm.eor(false, rd, rd, S3);
    }

    // ---- Arithmetic ----

    /// D8's operations on ST(0) and `src`: 0 add, 1 mul, 2 fcom, 3 fcomp, 4 sub, 5 subr, 6 div, 7 divr.
    fn with_st0(&mut self, ext: u8, src: u8) {
        match ext {
            2 | 3 => {
                self.fcom(ST0, src);
                if ext == 3 {
                    self.st_pop();
                }
            }
            _ => self.arith(ext, ST0, ST0, src),
        }
    }

    /// `dst = a op b`: 0 add, 1 mul, 4 `a - b`, 5 `b - a`, 6 `a / b`, 7 `b / a`.
    fn arith(&mut self, kind: u8, dst: u8, a: u8, b: u8) {
        let (op, x, y) = match kind {
            0 => (FADD, a, b),
            1 => (FMUL, a, b),
            4 => (FSUB, a, b),
            5 => (FSUB, b, a),
            6 => (FDIV, a, b),
            _ => (FDIV, b, a),
        };
        self.asm.f_scalar(op, true, dst, x, y);
    }

    /// `fcom` and its kin: C3, C2, C0 = 000 for greater, 001 less, 100 equal, 111 unordered. The flags are kept.
    fn fcom(&mut self, a: u8, b: u8) {
        let asm = &mut self.asm;
        asm.mrs_nzcv(F0);
        asm.fcmp(true, a, b);
        // ARM's lt is less or unordered; eq or vs is equal or unordered.
        asm.cset(false, S0, Cond::Lt);
        asm.cset(false, S1, Cond::Vs);
        asm.cset(false, S2, Cond::Eq);
        asm.orr(false, S2, S2, S1);
        asm.lsl_imm(false, S3, S0, 8);
        asm.orr_shift(false, S3, S3, S1, Shift::Lsl, 10);
        asm.orr_shift(false, S3, S3, S2, Shift::Lsl, 14);
        asm.ins_gen(1, CTL, 1, S3);
        asm.msr_nzcv(F0);
    }

    /// `fxam`: C3, C2, C0 = 001 NaN, 010 normal, 011 infinity, 100 zero, 110 denormal; C1 the sign.
    fn fxam(&mut self) {
        let a = &mut self.asm;
        a.mrs_nzcv(F0);
        a.fmov_x_d(S0, ST0);
        a.ubfx(true, S1, S0, 52, 11);
        a.lsl_imm(true, S2, S0, 12);
        a.mov_imm(R, C2);
        a.cmp_imm(true, S1, 0);
        a.mov_imm(S3, C3 | C2);
        a.csel(true, R, S3, R, Cond::Eq);
        a.lsl_imm(true, T, S0, 1);
        a.cmp_imm(true, T, 0);
        a.mov_imm(S3, C3);
        a.csel(true, R, S3, R, Cond::Eq);
        a.cmp_imm(true, S2, 0);
        a.mov_imm(S3, C2 | C0);
        a.mov_imm(T, C0);
        a.csel(true, S3, S3, T, Cond::Eq);
        a.mov_imm(T, 0x7FF);
        a.subs(true, ZR, S1, T);
        a.csel(true, R, S3, R, Cond::Eq);
        a.lsr_imm(true, S0, S0, 63);
        a.orr_shift(true, R, R, S0, Shift::Lsl, 9);
        a.ins_gen(1, CTL, 1, R);
        a.msr_nzcv(F0);
    }

    /// `vd = vn` rounded to an integer as the control word's rounding control says: nearest, down, up, toward zero.
    fn round_rc(&mut self, vd: u8, vn: u8) {
        // The default's rounding bits are 0, so the XORed word has them as they are.
        self.asm.umov(1, S0, CTL, 0);
        self.asm.ubfx(false, S0, S0, 10, 2);
        let mut ends = alloc::vec::Vec::new();
        for (rc, op) in [(0u16, FRINTN), (1, FRINTM), (2, FRINTP)] {
            let a = &mut self.asm;
            let skip = if rc == 0 {
                let at = a.len();
                a.cbnz(false, S0, 0);
                at
            } else {
                a.sub_imm(false, S1, S0, rc);
                let at = a.len();
                a.cbnz(false, S1, 0);
                at
            };
            a.f_unary(op, true, vd, vn);
            ends.push(a.len());
            a.b(0);
            let here = a.len();
            a.patch(skip, here);
        }
        self.asm.f_unary(FRINTZ, true, vd, vn);
        let end = self.asm.len();
        for at in ends {
            self.asm.patch(at, end);
        }
    }

    /// `fprem`: ST(0) = ST(0) - q * ST(1), q = ST(0) / ST(1) toward zero, with ST(0)'s sign. As on an x87, a quotient
    /// too big to do at once is done in parts: ST(1) scaled up by a power of two, and C2 set to say there's more.
    /// Every step is exact. C0, C3 and C1 are the quotient's low three bits.
    fn fprem(&mut self) {
        let a = &mut self.asm;
        a.mrs_nzcv(F0);
        a.fmov_x_d(S0, ST0);
        a.ubfx(true, S0, S0, 52, 11);
        a.fmov_x_d(S1, st(1));
        a.lsl_imm(true, S2, S1, 1);
        a.ubfx(true, S1, S1, 52, 11);
        a.v_mov(V1, st(1));
        a.mov_imm(R, 0);
        // A part at a time when the exponents are 53 or more apart, for finite numbers and a divisor that isn't zero.
        let mut skips = alloc::vec::Vec::new();
        skips.push(a.len());
        a.cbz(true, S2, 0);
        a.mov_imm(T, 0x7FF);
        a.subs(true, ZR, S0, T);
        skips.push(a.len());
        a.b_cond(Cond::Eq, 0);
        a.sub(true, S2, S0, S1);
        a.cmp_imm(true, S2, 53);
        skips.push(a.len());
        a.b_cond(Cond::Lt, 0);
        a.sub_imm(true, S2, S2, 32);
        a.asr_imm(true, S3, S2, 1);
        a.sub(true, S2, S2, S3);
        for k in [S3, S2] {
            a.add_imm(true, k, k, 1023);
            a.lsl_imm(true, k, k, 52);
            a.fmov_d_x(V2, k);
            a.f_scalar(FMUL, true, V1, V1, V2);
        }
        a.mov_imm(R, C2);
        let here = a.len();
        for at in skips {
            a.patch(at, here);
        }
        // q, and the remainder in one fused step; q = 0 leaves ST(0) as it is (even beside an infinite divisor).
        a.f_scalar(FDIV, true, V0, ST0, V1);
        a.f_unary(FRINTZ, true, V0, V0);
        a.fmsub(true, V2, V0, V1, ST0);
        a.fmov_d_x(V3, ZR);
        a.fcmp(true, V0, V3);
        a.fcsel(true, V2, ST0, V2, Cond::Eq);
        // q one too big leaves a remainder of the other sign: add the divisor back, signed as ST(0).
        a.fmov_x_d(S0, V2);
        a.fmov_x_d(S1, ST0);
        a.eor(true, S3, S0, S1);
        a.lsr_imm(true, S3, S3, 63);
        a.lsl_imm(true, T, S0, 1);
        a.cmp_imm(true, T, 0);
        a.cset(true, F1, Cond::Ne);
        a.and(true, F1, F1, S3);
        a.fmov_x_d(S2, V1);
        a.lsr_imm(true, S1, S1, 63);
        a.bfi(true, S2, S1, 63, 1);
        a.fmov_d_x(V3, S2);
        a.f_scalar(FADD, true, V3, V2, V3);
        a.cmp_imm(true, F1, 0);
        a.fcsel(true, V2, V3, V2, Cond::Ne);
        // The quotient's low bits: Q2 to C0, Q1 to C3, Q0 to C1.
        a.f_unary(FABS, true, V0, V0);
        a.fcvtzs(true, true, S0, V0);
        a.sub(true, S0, S0, F1);
        for (bit, at) in [(0u8, 9u8), (1, 14), (2, 8)] {
            a.ubfx(true, S1, S0, bit, 1);
            a.orr_shift(true, R, R, S1, Shift::Lsl, at);
        }
        a.ins_gen(1, CTL, 1, R);
        a.v_mov(ST0, V2);
        a.msr_nzcv(F0);
    }

    /// `fscale`: ST(0) * 2^ST(1), ST(1) toward zero; in three steps, so each power of two is a double.
    fn fscale(&mut self) {
        let a = &mut self.asm;
        a.mrs_nzcv(F0);
        a.fcvtzs(true, true, S0, st(1));
        a.mov_imm(T, 3000);
        a.subs(true, ZR, S0, T);
        a.csel(true, S0, T, S0, Cond::Gt);
        a.mov_imm(T, (-3000i64) as u64);
        a.subs(true, ZR, S0, T);
        a.csel(true, S0, T, S0, Cond::Lt);
        a.mov_imm(T, 3);
        a.sdiv(true, S1, S0, T);
        a.sub(true, S2, S0, S1);
        a.sub(true, S2, S2, S1);
        for k in [S1, S1, S2] {
            a.add_imm(true, S3, k, 1023);
            a.lsl_imm(true, S3, S3, 52);
            a.fmov_d_x(V1, S3);
            a.f_scalar(FMUL, true, ST0, ST0, V1);
        }
        // A NaN scale is a NaN.
        a.fcmp(true, st(1), st(1));
        a.fcsel(true, ST0, st(1), ST0, Cond::Vs);
        a.msr_nzcv(F0);
    }

    /// `sahf`: SF, ZF, PF and CF from ah. PF goes to V, as after a floating-point compare.
    pub(super) fn sahf(&mut self) {
        let a = &mut self.asm;
        a.ubfx(true, S0, RAX, 8, 8);
        a.ubfx(true, S1, S0, 7, 1);
        a.lsl_imm(true, R, S1, 31);
        a.ubfx(true, S1, S0, 6, 1);
        a.orr_shift(true, R, R, S1, Shift::Lsl, 30);
        a.ubfx(true, S1, S0, 0, 1);
        a.mov_imm(T, 1);
        a.eor(true, S1, S1, T);
        a.orr_shift(true, R, R, S1, Shift::Lsl, 29);
        a.ubfx(true, S1, S0, 2, 1);
        a.orr_shift(true, R, R, S1, Shift::Lsl, 28);
        a.msr_nzcv(R);
    }

    /// `lahf`: ah = SF, ZF, 0, AF (0), 0, PF (V), 1, CF.
    pub(super) fn lahf(&mut self) {
        let a = &mut self.asm;
        a.mrs_nzcv(S0);
        a.ubfx(true, S1, S0, 31, 1);
        a.lsl_imm(true, R, S1, 7);
        a.ubfx(true, S1, S0, 30, 1);
        a.orr_shift(true, R, R, S1, Shift::Lsl, 6);
        a.ubfx(true, S1, S0, 28, 1);
        a.orr_shift(true, R, R, S1, Shift::Lsl, 2);
        a.ubfx(true, S1, S0, 29, 1);
        a.mov_imm(T, 3);
        a.eor(true, S1, S1, T);
        a.orr(true, R, R, S1);
        a.bfi(true, RAX, R, 8, 8);
    }
}
