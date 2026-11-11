//! An ARM64 assembler: just the instructions the emitters use, each checked against LLVM's encoding by the tests.
//! `sf` picks 64-bit (`x`) over 32-bit (`w`). Register 31 is the zero register everywhere here: nothing uses `sp`.

use alloc::vec::Vec;

pub const ZR: u8 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Eq = 0,
    Ne = 1,
    Cs = 2,
    Cc = 3,
    Mi = 4,
    Pl = 5,
    Vs = 6,
    Vc = 7,
    Hi = 8,
    Ls = 9,
    Ge = 10,
    Lt = 11,
    Gt = 12,
    Le = 13,
}

impl Cond {
    pub fn invert(self) -> Cond {
        use Cond::*;
        match self {
            Eq => Ne,
            Ne => Eq,
            Cs => Cc,
            Cc => Cs,
            Mi => Pl,
            Pl => Mi,
            Vs => Vc,
            Vc => Vs,
            Hi => Ls,
            Ls => Hi,
            Ge => Lt,
            Lt => Ge,
            Gt => Le,
            Le => Gt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Lsl = 0,
    Lsr = 1,
    Asr = 2,
}

#[derive(Default)]
pub struct Asm {
    pub code: Vec<u32>,
}

fn r(reg: u8) -> u32 {
    u32::from(reg & 31)
}

fn sf_bit(sf: bool) -> u32 {
    if sf { 1 << 31 } else { 0 }
}

/// The size field of a load, store or atomic: 0 for bytes up to 3 for doublewords.
pub fn size_code(bits: u8) -> u32 {
    match bits {
        8 => 0,
        16 => 1,
        32 => 2,
        _ => 3,
    }
}

impl Asm {
    pub fn new() -> Asm {
        Asm::default()
    }

    /// Instructions so far.
    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    pub fn truncate(&mut self, len: usize) {
        self.code.truncate(len);
    }

    pub fn put(&mut self, word: u32) {
        self.code.push(word);
    }

    // ---- Moves ----

    pub fn movz(&mut self, sf: bool, rd: u8, imm16: u16, hw: u8) {
        self.put(0x5280_0000 | sf_bit(sf) | u32::from(hw) << 21 | u32::from(imm16) << 5 | r(rd));
    }

    pub fn movk(&mut self, sf: bool, rd: u8, imm16: u16, hw: u8) {
        self.put(0x7280_0000 | sf_bit(sf) | u32::from(hw) << 21 | u32::from(imm16) << 5 | r(rd));
    }

    /// Any 64-bit constant, in as few instructions as its non-zero halfwords allow.
    pub fn mov_imm(&mut self, rd: u8, value: u64) {
        let mut first = true;
        for hw in 0..4u8 {
            let part = (value >> (16 * hw)) as u16;
            if part == 0 {
                continue;
            }
            if first {
                self.movz(true, rd, part, hw);
                first = false;
            } else {
                self.movk(true, rd, part, hw);
            }
        }
        if first {
            self.movz(true, rd, 0, 0);
        }
    }

    pub fn mov(&mut self, sf: bool, rd: u8, rm: u8) {
        self.orr(sf, rd, ZR, rm);
    }

    // ---- Logical, shifted register ----

    #[allow(clippy::too_many_arguments)]
    fn logical(&mut self, base: u32, sf: bool, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.put(base | sf_bit(sf) | (shift as u32) << 22 | r(rm) << 16 | u32::from(amount & 63) << 10 | r(rn) << 5 | r(rd));
    }

    pub fn and(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.logical(0x0A00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn ands(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.logical(0x6A00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn orr(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.logical(0x2A00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn orr_shift(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.logical(0x2A00_0000, sf, rd, rn, rm, shift, amount);
    }
    pub fn orn(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.logical(0x2A20_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn eor(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.logical(0x4A00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }

    // ---- Add and subtract ----

    #[allow(clippy::too_many_arguments)]
    fn arith(&mut self, base: u32, sf: bool, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.put(base | sf_bit(sf) | (shift as u32) << 22 | r(rm) << 16 | u32::from(amount & 63) << 10 | r(rn) << 5 | r(rd));
    }

    pub fn add(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.arith(0x0B00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn add_shift(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.arith(0x0B00_0000, sf, rd, rn, rm, shift, amount);
    }
    pub fn adds(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.arith(0x2B00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    /// `adcs`: rn + rm + C, setting flags.
    pub fn adcs(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x3A00_0000 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    /// `sbcs`: rn - rm - !C, setting flags.
    pub fn sbcs(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x7A00_0000 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn sub(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.arith(0x4B00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    /// `rd = rn - (rm shifted)`, leaving the flags.
    pub fn sub_shift(&mut self, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.put(0xCB00_0000 | (shift as u32) << 22 | r(rm) << 16 | u32::from(amount & 63) << 10 | r(rn) << 5 | r(rd));
    }
    pub fn subs(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.arith(0x6B00_0000, sf, rd, rn, rm, Shift::Lsl, 0);
    }
    pub fn subs_shift(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, shift: Shift, amount: u8) {
        self.arith(0x6B00_0000, sf, rd, rn, rm, shift, amount);
    }

    /// `rd = rn + imm`, `imm` under 4096. Never with register 31, which would be `sp` here.
    pub fn add_imm(&mut self, sf: bool, rd: u8, rn: u8, imm: u16) {
        self.put(0x1100_0000 | sf_bit(sf) | u32::from(imm & 0xFFF) << 10 | r(rn) << 5 | r(rd));
    }
    pub fn sub_imm(&mut self, sf: bool, rd: u8, rn: u8, imm: u16) {
        self.put(0x5100_0000 | sf_bit(sf) | u32::from(imm & 0xFFF) << 10 | r(rn) << 5 | r(rd));
    }
    /// `cmp rn, #imm`.
    pub fn cmp_imm(&mut self, sf: bool, rn: u8, imm: u16) {
        self.put(0x7100_0000 | sf_bit(sf) | u32::from(imm & 0xFFF) << 10 | r(rn) << 5 | r(ZR));
    }

    // ---- Bitfields ----

    fn bitfield(&mut self, base: u32, sf: bool, rd: u8, rn: u8, immr: u8, imms: u8) {
        let n = if sf { 1 << 22 } else { 0 };
        self.put(base | sf_bit(sf) | n | u32::from(immr & 63) << 16 | u32::from(imms & 63) << 10 | r(rn) << 5 | r(rd));
    }

    pub fn sbfm(&mut self, sf: bool, rd: u8, rn: u8, immr: u8, imms: u8) {
        self.bitfield(0x1300_0000, sf, rd, rn, immr, imms);
    }
    pub fn bfm(&mut self, sf: bool, rd: u8, rn: u8, immr: u8, imms: u8) {
        self.bitfield(0x3300_0000, sf, rd, rn, immr, imms);
    }
    pub fn ubfm(&mut self, sf: bool, rd: u8, rn: u8, immr: u8, imms: u8) {
        self.bitfield(0x5300_0000, sf, rd, rn, immr, imms);
    }

    fn width(sf: bool) -> u8 {
        if sf { 64 } else { 32 }
    }

    pub fn lsl_imm(&mut self, sf: bool, rd: u8, rn: u8, shift: u8) {
        let w = Self::width(sf);
        self.ubfm(sf, rd, rn, (w - shift) % w, w - 1 - shift);
    }
    pub fn lsr_imm(&mut self, sf: bool, rd: u8, rn: u8, shift: u8) {
        self.ubfm(sf, rd, rn, shift, Self::width(sf) - 1);
    }
    pub fn asr_imm(&mut self, sf: bool, rd: u8, rn: u8, shift: u8) {
        self.sbfm(sf, rd, rn, shift, Self::width(sf) - 1);
    }
    /// `width` bits from `lsb`, to the bottom, zero-extended.
    pub fn ubfx(&mut self, sf: bool, rd: u8, rn: u8, lsb: u8, width: u8) {
        self.ubfm(sf, rd, rn, lsb, lsb + width - 1);
    }
    /// The same, sign-extended.
    pub fn sbfx(&mut self, sf: bool, rd: u8, rn: u8, lsb: u8, width: u8) {
        self.sbfm(sf, rd, rn, lsb, lsb + width - 1);
    }
    /// `width` bits from the bottom of `rn`, into `rd` at `lsb`, the rest of `rd` kept.
    pub fn bfi(&mut self, sf: bool, rd: u8, rn: u8, lsb: u8, width: u8) {
        let w = Self::width(sf);
        self.bfm(sf, rd, rn, (w - lsb) % w, width - 1);
    }

    // ---- Multiply and divide ----

    pub fn madd(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, ra: u8) {
        self.put(0x1B00_0000 | sf_bit(sf) | r(rm) << 16 | r(ra) << 10 | r(rn) << 5 | r(rd));
    }
    pub fn mul(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.madd(sf, rd, rn, rm, ZR);
    }
    pub fn msub(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, ra: u8) {
        self.put(0x1B00_8000 | sf_bit(sf) | r(rm) << 16 | r(ra) << 10 | r(rn) << 5 | r(rd));
    }
    pub fn smulh(&mut self, rd: u8, rn: u8, rm: u8) {
        self.put(0x9B40_7C00 | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn umulh(&mut self, rd: u8, rn: u8, rm: u8) {
        self.put(0x9BC0_7C00 | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn udiv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_0800 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn sdiv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_0C00 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }

    // ---- Variable shifts ----

    /// `rd = (rn:rm) >> lsb`, 64-bit: with `lsb` 63, `rn << 1` with `rm`'s top bit coming in.
    pub fn extr(&mut self, rd: u8, rn: u8, rm: u8, lsb: u8) {
        self.put(0x93C0_0000 | r(rm) << 16 | u32::from(lsb & 63) << 10 | r(rn) << 5 | r(rd));
    }
    pub fn lslv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_2000 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn lsrv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_2400 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn asrv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_2800 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }
    pub fn rorv(&mut self, sf: bool, rd: u8, rn: u8, rm: u8) {
        self.put(0x1AC0_2C00 | sf_bit(sf) | r(rm) << 16 | r(rn) << 5 | r(rd));
    }

    // ---- Conditional select, and bits ----

    pub fn csel(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, cond: Cond) {
        self.put(0x1A80_0000 | sf_bit(sf) | r(rm) << 16 | (cond as u32) << 12 | r(rn) << 5 | r(rd));
    }
    pub fn csinc(&mut self, sf: bool, rd: u8, rn: u8, rm: u8, cond: Cond) {
        self.put(0x1A80_0400 | sf_bit(sf) | r(rm) << 16 | (cond as u32) << 12 | r(rn) << 5 | r(rd));
    }
    /// `rd = cond ? 1 : 0`.
    pub fn cset(&mut self, sf: bool, rd: u8, cond: Cond) {
        self.csinc(sf, rd, ZR, ZR, cond.invert());
    }
    pub fn clz(&mut self, sf: bool, rd: u8, rn: u8) {
        self.put(0x5AC0_1000 | sf_bit(sf) | r(rn) << 5 | r(rd));
    }
    pub fn rbit(&mut self, sf: bool, rd: u8, rn: u8) {
        self.put(0x5AC0_0000 | sf_bit(sf) | r(rn) << 5 | r(rd));
    }
    /// Byte reverse.
    pub fn rev(&mut self, sf: bool, rd: u8, rn: u8) {
        self.put(if sf { 0xDAC0_0C00 } else { 0x5AC0_0800 } | r(rn) << 5 | r(rd));
    }

    // ---- Loads and stores, at [rn] ----

    /// A load of `bits`, zero-extended.
    pub fn ldr(&mut self, bits: u8, rt: u8, rn: u8) {
        self.put(0x3940_0000 | size_code(bits) << 30 | r(rn) << 5 | r(rt));
    }
    pub fn str(&mut self, bits: u8, rt: u8, rn: u8) {
        self.put(0x3900_0000 | size_code(bits) << 30 | r(rn) << 5 | r(rt));
    }
    /// `ldr xt, [xn, xm, lsl #3]`.
    pub fn ldr_index(&mut self, rt: u8, rn: u8, rm: u8) {
        self.put(0xF860_7800 | r(rm) << 16 | r(rn) << 5 | r(rt));
    }
    /// A load of `bits` (8, 16 or 32), sign-extended to 64.
    pub fn ldrs(&mut self, bits: u8, rt: u8, rn: u8) {
        self.put(0x3980_0000 | size_code(bits) << 30 | r(rn) << 5 | r(rt));
    }
    pub fn ldr_q(&mut self, vt: u8, rn: u8) {
        self.put(0x3DC0_0000 | r(rn) << 5 | r(vt));
    }
    pub fn str_q(&mut self, vt: u8, rn: u8) {
        self.put(0x3D80_0000 | r(rn) << 5 | r(vt));
    }
    pub fn ldr_d(&mut self, vt: u8, rn: u8) {
        self.put(0xFD40_0000 | r(rn) << 5 | r(vt));
    }
    pub fn str_d(&mut self, vt: u8, rn: u8) {
        self.put(0xFD00_0000 | r(rn) << 5 | r(vt));
    }
    pub fn ldr_s(&mut self, vt: u8, rn: u8) {
        self.put(0xBD40_0000 | r(rn) << 5 | r(vt));
    }
    pub fn str_s(&mut self, vt: u8, rn: u8) {
        self.put(0xBD00_0000 | r(rn) << 5 | r(vt));
    }

    // ---- Atomics (LSE), acquire and release ----

    fn atomic(&mut self, base: u32, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.put(base | size_code(bits) << 30 | r(rs) << 16 | r(rn) << 5 | r(rt));
    }
    /// `rt = [rn]; [rn] = rs`.
    pub fn swpal(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x38E0_8000, bits, rs, rt, rn);
    }
    /// `if [rn] == rs { [rn] = rt }; rs = old [rn]`.
    pub fn casal(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x08E0_FC00, bits, rs, rt, rn);
    }
    /// `rt = [rn]; [rn] += rs`.
    pub fn ldaddal(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x38E0_0000, bits, rs, rt, rn);
    }
    /// `rt = [rn]; [rn] &= !rs`.
    pub fn ldclral(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x38E0_1000, bits, rs, rt, rn);
    }
    pub fn ldeoral(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x38E0_2000, bits, rs, rt, rn);
    }
    pub fn ldsetal(&mut self, bits: u8, rs: u8, rt: u8, rn: u8) {
        self.atomic(0x38E0_3000, bits, rs, rt, rn);
    }

    // ---- Branches: offsets in instructions ----

    pub fn br(&mut self, rn: u8) {
        self.put(0xD61F_0000 | r(rn) << 5);
    }
    pub fn b(&mut self, offset: i32) {
        self.put(0x1400_0000 | (offset as u32 & 0x03FF_FFFF));
    }
    pub fn b_cond(&mut self, cond: Cond, offset: i32) {
        self.put(0x5400_0000 | (offset as u32 & 0x7FFFF) << 5 | cond as u32);
    }
    pub fn cbz(&mut self, sf: bool, rt: u8, offset: i32) {
        self.put(0x3400_0000 | sf_bit(sf) | (offset as u32 & 0x7FFFF) << 5 | r(rt));
    }
    pub fn cbnz(&mut self, sf: bool, rt: u8, offset: i32) {
        self.put(0x3500_0000 | sf_bit(sf) | (offset as u32 & 0x7FFFF) << 5 | r(rt));
    }

    /// Points the branch at `at` (a `b`, `b.cond`, `cbz` or `cbnz`) to `target`.
    pub fn patch(&mut self, at: usize, target: usize) {
        let offset = target as i32 - at as i32;
        let word = self.code[at];
        self.code[at] = if word & 0xFC00_0000 == 0x1400_0000 {
            (word & 0xFC00_0000) | (offset as u32 & 0x03FF_FFFF)
        } else {
            (word & !(0x7FFFF << 5)) | (offset as u32 & 0x7FFFF) << 5
        };
    }

    // ---- System ----

    pub fn brk(&mut self, imm: u16) {
        self.put(0xD420_0000 | u32::from(imm) << 5);
    }
    pub fn mrs_nzcv(&mut self, rt: u8) {
        self.put(0xD53B_4200 | r(rt));
    }
    pub fn msr_nzcv(&mut self, rt: u8) {
        self.put(0xD51B_4200 | r(rt));
    }
    pub fn mrs_cntvct(&mut self, rt: u8) {
        self.put(0xD53B_E040 | r(rt));
    }
    /// Inverts the carry flag (FEAT_FlagM).
    pub fn cfinv(&mut self) {
        self.put(0xD500_401F);
    }
    pub fn dmb_ish(&mut self) {
        self.put(0xD503_3BBF);
    }
    pub fn yield_(&mut self) {
        self.put(0xD503_203F);
    }

    // ---- SIMD, on whole 128-bit registers ----

    pub fn v_and(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x4E20_1C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    /// `vd = vn & !vm`.
    pub fn v_bic(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x4E60_1C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    pub fn v_orr(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x4EA0_1C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    pub fn v_eor(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x6E20_1C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    pub fn v_mov(&mut self, vd: u8, vn: u8) {
        self.v_orr(vd, vn, vn);
    }
    /// Bytes equal: all ones, else zero.
    pub fn cmeq_16b(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x6E20_8C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    /// Bytes negative: all ones, else zero.
    pub fn cmlt0_16b(&mut self, vd: u8, vn: u8) {
        self.put(0x4E20_A800 | r(vn) << 5 | r(vd));
    }
    /// The sum of the low 8 bytes, into byte 0, the rest zeroed.
    pub fn addv_8b(&mut self, vd: u8, vn: u8) {
        self.put(0x0E31_B800 | r(vn) << 5 | r(vd));
    }
    /// Bytes `index..` of `vn` then of `vm`.
    pub fn ext_16b(&mut self, vd: u8, vn: u8, vm: u8, index: u8) {
        self.put(0x6E00_0000 | r(vm) << 16 | u32::from(index & 15) << 11 | r(vn) << 5 | r(vd));
    }
    /// The low 64 bits from a general register, the rest zeroed.
    pub fn fmov_d_x(&mut self, vd: u8, xn: u8) {
        self.put(0x9E67_0000 | r(xn) << 5 | r(vd));
    }
    pub fn fmov_x_d(&mut self, xd: u8, vn: u8) {
        self.put(0x9E66_0000 | r(vn) << 5 | r(xd));
    }
    /// The low 32 bits from a general register, the rest zeroed.
    pub fn fmov_s_w(&mut self, vd: u8, wn: u8) {
        self.put(0x1E27_0000 | r(wn) << 5 | r(vd));
    }
    pub fn fmov_w_s(&mut self, wd: u8, vn: u8) {
        self.put(0x1E26_0000 | r(vn) << 5 | r(wd));
    }
    /// The low 64 bits of `vn`, the rest zeroed.
    pub fn fmov_d_d(&mut self, vd: u8, vn: u8) {
        self.put(0x1E60_4000 | r(vn) << 5 | r(vd));
    }
    /// `vd.d[0] = vn.d[0]`, the rest of `vd` kept.
    pub fn ins_d0(&mut self, vd: u8, vn: u8) {
        self.put(0x6E08_0400 | r(vn) << 5 | r(vd));
    }
    /// `vd.s[0] = vn.s[0]`, the rest of `vd` kept.
    pub fn ins_s0(&mut self, vd: u8, vn: u8) {
        self.put(0x6E04_0400 | r(vn) << 5 | r(vd));
    }
    /// `vd.d[1] = vn.d[0]`.
    pub fn ins_d1_d0(&mut self, vd: u8, vn: u8) {
        self.put(0x6E18_0400 | r(vn) << 5 | r(vd));
    }

    // ---- SIMD by element: sizes are 0 bytes, 1 halfwords, 2 words, 3 doublewords; always 128 bits ----

    /// A three-register vector instruction, from its 128-bit base encoding, at element `size`.
    pub fn v3(&mut self, base: u32, size: u8, vd: u8, vn: u8, vm: u8) {
        self.put(base | u32::from(size & 3) << 22 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    /// A two-register (unary) vector instruction, from its 128-bit base encoding, at element `size`.
    pub fn v2(&mut self, base: u32, size: u8, vd: u8, vn: u8) {
        self.put(base | u32::from(size & 3) << 22 | r(vn) << 5 | r(vd));
    }
    /// `vd = vd ? vn : vm`, bit by bit.
    pub fn bsl(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x6E60_1C00 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    pub fn v_not(&mut self, vd: u8, vn: u8) {
        self.put(0x6E20_5800 | r(vn) << 5 | r(vd));
    }
    /// Each element shifted left by `shift` (below its width).
    pub fn shl(&mut self, size: u8, vd: u8, vn: u8, shift: u8) {
        let e = 8u32 << size;
        self.put(0x4F00_5400 | (e + u32::from(shift)) << 16 | r(vn) << 5 | r(vd));
    }
    /// Each element shifted right, logically, by `shift` (1 to its width).
    pub fn ushr(&mut self, size: u8, vd: u8, vn: u8, shift: u8) {
        let e = 8u32 << size;
        self.put(0x6F00_0400 | (2 * e - u32::from(shift)) << 16 | r(vn) << 5 | r(vd));
    }
    /// Each element shifted right, arithmetically, by `shift` (1 to its width).
    pub fn sshr(&mut self, size: u8, vd: u8, vn: u8, shift: u8) {
        let e = 8u32 << size;
        self.put(0x4F00_0400 | (2 * e - u32::from(shift)) << 16 | r(vn) << 5 | r(vd));
    }
    /// Bytes of `vn` (and `vn + 1` with `two`) picked by `vm`'s bytes as indexes; an index past the table gives 0.
    pub fn tbl(&mut self, vd: u8, vn: u8, two: bool, vm: u8) {
        self.put(0x4E00_0000 | r(vm) << 16 | u32::from(two) << 13 | r(vn) << 5 | r(vd));
    }
    /// The low 32 bits of each of `vn`'s two doublewords, into `vd`'s low two words, the high half zeroed (`xtn .2s`).
    pub fn xtn_2s(&mut self, vd: u8, vn: u8) {
        self.put(0x0EA1_2800 | r(vn) << 5 | r(vd));
    }
    /// Elements of `size` from `vn` and `vm` multiplied into elements twice as wide: the low half's (`umull`, `smull`) or,
    /// with `high`, the high half's (`umull2`, `smull2`).
    pub fn mull(&mut self, signed: bool, high: bool, size: u8, vd: u8, vn: u8, vm: u8) {
        let base = if signed { 0x0E20_C000 } else { 0x2E20_C000 };
        self.put(base | u32::from(high) << 30 | u32::from(size & 3) << 22 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    /// Every element of `size` of `vd` set to `rn`'s low bits (`dup`, from a general register).
    pub fn dup_gen(&mut self, size: u8, vd: u8, rn: u8) {
        self.put(0x4E00_0C00 | (1u32 << size) << 16 | r(rn) << 5 | r(vd));
    }
    /// Adjacent pairs of `vn`'s elements of `size` added into elements twice as wide (`uaddlp`, 128 bits).
    pub fn uaddlp(&mut self, size: u8, vd: u8, vn: u8) {
        self.put(0x6E20_2800 | u32::from(size & 3) << 22 | r(vn) << 5 | r(vd));
    }
    /// `vn`'s and `vm`'s low two words, multiplied unsigned into two doublewords (`umull .2d, .2s, .2s`).
    pub fn umull_2d(&mut self, vd: u8, vn: u8, vm: u8) {
        self.put(0x2EA0_C000 | r(vm) << 16 | r(vn) << 5 | r(vd));
    }
    /// `vd.T[i] = vn.T[j]`, for elements of `size`.
    pub fn ins_elem(&mut self, size: u8, vd: u8, i: u8, vn: u8, j: u8) {
        let imm5 = (u32::from(i) << (size + 1)) | 1 << size;
        let imm4 = u32::from(j) << size;
        self.put(0x6E00_0400 | imm5 << 16 | imm4 << 11 | r(vn) << 5 | r(vd));
    }
    /// `vd.T[i] = wn` (or `xn` for doublewords).
    pub fn ins_gen(&mut self, size: u8, vd: u8, i: u8, rn: u8) {
        let imm5 = (u32::from(i) << (size + 1)) | 1 << size;
        self.put(0x4E00_1C00 | imm5 << 16 | r(rn) << 5 | r(vd));
    }
    /// `wd = vn.T[i]`, zero-extended (`xd` for doublewords).
    pub fn umov(&mut self, size: u8, rd: u8, vn: u8, i: u8) {
        let imm5 = (u32::from(i) << (size + 1)) | 1 << size;
        let q = u32::from(size == 3) << 30;
        self.put(0x0E00_3C00 | q | imm5 << 16 | r(vn) << 5 | r(rd));
    }
    /// The low half of `vd` from `vn`'s elements of `size + 1`, saturated signed (`unsigned` saturates signed values to
    /// unsigned), the high half zeroed; or with `high`, the high half, the low kept.
    pub fn narrow(&mut self, size: u8, unsigned: bool, high: bool, vd: u8, vn: u8) {
        let base = if unsigned { 0x2E21_2800 } else { 0x0E21_4800 };
        self.put(base | u32::from(high) << 30 | u32::from(size & 3) << 22 | r(vn) << 5 | r(vd));
    }

    // ---- Floating point; `double` picks 64-bit over 32 ----

    /// A scalar operation: `op` is 0 mul, 1 div, 2 add, 3 sub, 4 max, 5 min.
    pub fn f_scalar(&mut self, op: u8, double: bool, vd: u8, vn: u8, vm: u8) {
        self.put(0x1E20_0800 | u32::from(double) << 22 | r(vm) << 16 | u32::from(op) << 12 | r(vn) << 5 | r(vd));
    }
    /// One-source operations: 0 `fmov`, 1 `fabs`, 2 `fneg`, 3 `fsqrt`, 8 `frintn`, 9 `frintp`, 10 `frintm`, 11 `frintz`,
    /// 15 `frinti`.
    pub fn f_unary(&mut self, op: u8, double: bool, vd: u8, vn: u8) {
        self.put(0x1E20_4000 | u32::from(double) << 22 | u32::from(op) << 15 | r(vn) << 5 | r(vd));
    }
    /// `vd = va - vn * vm`, fused: one rounding.
    pub fn fmsub(&mut self, double: bool, vd: u8, vn: u8, vm: u8, va: u8) {
        self.put(0x1F00_8000 | u32::from(double) << 22 | r(vm) << 16 | r(va) << 10 | r(vn) << 5 | r(vd));
    }
    /// An unsigned integer (`sf`: 64 bits) to floating point, rounded as FPCR says.
    pub fn ucvtf(&mut self, sf: bool, double: bool, vd: u8, rn: u8) {
        self.put(0x1E23_0000 | u32::from(sf) << 31 | u32::from(double) << 22 | r(rn) << 5 | r(vd));
    }
    pub fn fsqrt(&mut self, double: bool, vd: u8, vn: u8) {
        self.put(0x1E21_C000 | u32::from(double) << 22 | r(vn) << 5 | r(vd));
    }
    pub fn fcmp(&mut self, double: bool, vn: u8, vm: u8) {
        self.put(0x1E20_2000 | u32::from(double) << 22 | r(vm) << 16 | r(vn) << 5);
    }
    pub fn fcsel(&mut self, double: bool, vd: u8, vn: u8, vm: u8, cond: Cond) {
        self.put(0x1E20_0C00 | u32::from(double) << 22 | r(vm) << 16 | (cond as u32) << 12 | r(vn) << 5 | r(vd));
    }
    /// Single to double, or with `to_single`, double to single.
    pub fn fcvt(&mut self, to_single: bool, vd: u8, vn: u8) {
        self.put(if to_single { 0x1E62_4000 } else { 0x1E22_C000 } | r(vn) << 5 | r(vd));
    }
    /// A signed integer (`sf`: 64 bits) to floating point, rounded as FPCR says.
    pub fn scvtf(&mut self, sf: bool, double: bool, vd: u8, rn: u8) {
        self.put(0x1E22_0000 | u32::from(sf) << 31 | u32::from(double) << 22 | r(rn) << 5 | r(vd));
    }
    /// Floating point to a signed integer, toward zero, saturating.
    pub fn fcvtzs(&mut self, sf: bool, double: bool, rd: u8, vn: u8) {
        self.put(0x1E38_0000 | u32::from(sf) << 31 | u32::from(double) << 22 | r(vn) << 5 | r(rd));
    }
    /// Rounded to an integral value as FPCR says.
    pub fn frinti(&mut self, double: bool, vd: u8, vn: u8) {
        self.put(0x1E27_C000 | u32::from(double) << 22 | r(vn) << 5 | r(vd));
    }
    pub fn mrs_fpcr(&mut self, rt: u8) {
        self.put(0xD53B_4400 | r(rt));
    }
    pub fn msr_fpcr(&mut self, rt: u8) {
        self.put(0xD51B_4400 | r(rt));
    }
}

/// 128-bit vector instructions' base encodings, for `Asm::v3` and `Asm::v2`.
pub mod vec {
    pub const ADD: u32 = 0x4E20_8400;
    pub const SUB: u32 = 0x6E20_8400;
    pub const CMEQ: u32 = 0x6E20_8C00;
    pub const CMGT: u32 = 0x4E20_3400;
    pub const SQADD: u32 = 0x4E20_0C00;
    pub const UQADD: u32 = 0x6E20_0C00;
    pub const SQSUB: u32 = 0x4E20_2C00;
    pub const UQSUB: u32 = 0x6E20_2C00;
    pub const SMAX: u32 = 0x4E20_6400;
    pub const UMAX: u32 = 0x6E20_6400;
    pub const SMIN: u32 = 0x4E20_6C00;
    pub const UMIN: u32 = 0x6E20_6C00;
    pub const MUL: u32 = 0x4E20_9C00;
    pub const URHADD: u32 = 0x6E20_1400;
    pub const ZIP1: u32 = 0x4E00_3800;
    /// The odd-numbered elements of `vn` then of `vm`.
    pub const UZP2: u32 = 0x4E00_5800;
    /// Adjacent pairs of `vn`'s elements, then of `vm`'s, added.
    pub const ADDP: u32 = 0x4E20_BC00;
    /// Shifts by each element of `vm`'s low byte, signed: left if positive, right (logical) if negative.
    pub const USHL: u32 = 0x6E20_4400;
    /// The same, but arithmetic right.
    pub const SSHL: u32 = 0x4E20_4400;
    /// The absolute difference, unsigned.
    pub const UABD: u32 = 0x6E20_7400;
    pub const ZIP2: u32 = 0x4E00_7800;
    /// Floating point: pass size 0 for singles and 1 for doubles. Bit 22 is the precision, and bit 23 is part of
    /// these opcodes (set in `FSUB`, `FCMGT` and `FSQRT`), so a size of 2 or 3 would change the instruction.
    pub const FADD: u32 = 0x4E20_D400;
    pub const FSUB: u32 = 0x4EA0_D400;
    pub const FMUL: u32 = 0x6E20_DC00;
    pub const FDIV: u32 = 0x6E20_FC00;
    pub const FCMEQ: u32 = 0x4E20_E400;
    pub const FCMGE: u32 = 0x6E20_E400;
    pub const FCMGT: u32 = 0x6EA0_E400;
    pub const FSQRT: u32 = 0x6EA1_F800;
}
