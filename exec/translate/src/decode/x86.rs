//! x86-64: prefixes, REX, the one- and two-byte opcode maps, ModRM, SIB, displacements and immediates.
//!
//! It decodes every instruction's length and operands, whether or not the emitter can lower it, so a block always
//! ends at a real instruction boundary. Opcodes it doesn't know at all are `Error::Undecodable`.

use super::{Decoder, Flow, Insn};
use crate::{Error, Isa};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seg {
    Fs,
    Gs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Reg(u8),
    Rip,
}

/// A memory operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mem {
    pub base: Option<Base>,
    /// The index register, and its scale as a shift: 0 to 3.
    pub index: Option<(u8, u8)>,
    pub disp: i64,
    pub seg: Option<Seg>,
    /// `67`: the address is 32 bits.
    pub addr32: bool,
}

/// ModRM's r/m operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rm {
    Reg(u8),
    Mem(Mem),
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inst {
    pub at: u64,
    pub len: u8,
    /// 0: one-byte opcodes; 1: `0F`; 2: `0F 38`; 3: `0F 3A`.
    pub map: u8,
    pub op: u8,
    pub opsize16: bool,
    pub rep: bool,
    pub repne: bool,
    pub lock: bool,
    /// The REX byte, or 0.
    pub rex: u8,
    /// ModRM's reg field with REX.R, or for an opcode with the register in its low bits, that register with REX.B.
    pub reg: u8,
    /// ModRM's reg field alone: the opcode extension of a group.
    pub ext: u8,
    pub rm: Option<Rm>,
    pub imm: i64,
}

impl Inst {
    pub fn next(&self) -> u64 {
        self.at + u64::from(self.len)
    }

    pub fn w(&self) -> bool {
        self.rex & 8 != 0
    }

    /// The size of a `v` operand: 64 with REX.W, 16 with `66`, else 32.
    pub fn size_v(&self) -> u8 {
        if self.w() {
            64
        } else if self.opsize16 {
            16
        } else {
            32
        }
    }

    /// Where a relative branch goes.
    pub fn target(&self) -> u64 {
        self.next().wrapping_add(self.imm as u64)
    }
}

fn has_modrm(map: u8, op: u8) -> bool {
    match map {
        0 => (op < 0x40 && op & 7 < 4)
            || matches!(op, 0x62 | 0x63 | 0x69 | 0x6B | 0x80..=0x8F | 0xC0 | 0xC1 | 0xC4..=0xC7 | 0xD0..=0xD3 | 0xD8..=0xDF | 0xF6 | 0xF7 | 0xFE | 0xFF),
        1 => !matches!(op, 0x05..=0x09 | 0x0B | 0x0E | 0x30..=0x37 | 0x77 | 0x80..=0x8F | 0xA0..=0xA2 | 0xA8..=0xAA | 0xC8..=0xCF),
        _ => true,
    }
}

/// How many immediate bytes follow, or None for an opcode this doesn't know.
fn imm_len(inst: &Inst) -> Option<u8> {
    let z = if inst.opsize16 { 2 } else { 4 };
    Some(match (inst.map, inst.op) {
        (0, op) if op < 0x40 && op & 7 == 4 => 1,
        (0, op) if op < 0x40 && op & 7 == 5 => z,
        (0, op) if op < 0x40 => 0,
        (0, 0x40..=0x5F | 0x63 | 0x84..=0x8F | 0x90..=0x99 | 0x9B..=0x9F | 0xA4..=0xA7 | 0xAA..=0xAF | 0xC3 | 0xC9 | 0xCB | 0xCC | 0xD0..=0xD3) => 0,
        (0, 0xD8..=0xDF | 0xF1 | 0xF4 | 0xF5 | 0xF8..=0xFD | 0xFE | 0xFF | 0x62 | 0xC4 | 0xC5 | 0xEC..=0xEF | 0xCF) => 0,
        (0, 0x68 | 0x69 | 0x81 | 0xA9 | 0xC7) => z,
        (0, 0x6A | 0x6B | 0x70..=0x7F | 0x80 | 0x82 | 0x83 | 0xA8 | 0xB0..=0xB7 | 0xC0 | 0xC1 | 0xC6 | 0xCD | 0xD4 | 0xD5 | 0xE0..=0xE7 | 0xEB) => 1,
        (0, 0xB8..=0xBF) => {
            if inst.w() {
                8
            } else {
                z
            }
        }
        (0, 0xA0..=0xA3) => 8,
        (0, 0xC2 | 0xCA) => 2,
        (0, 0xC8) => 3,
        (0, 0xE8 | 0xE9) => 4,
        (0, 0xF6) => {
            if inst.ext < 2 {
                1
            } else {
                0
            }
        }
        (0, 0xF7) => {
            if inst.ext < 2 {
                z
            } else {
                0
            }
        }
        (0, _) => return None,
        (1, 0x70..=0x73 | 0xA4 | 0xAC | 0xBA | 0xC2 | 0xC4..=0xC6) => 1,
        (1, 0x80..=0x8F) => 4,
        (1, _) => 0,
        (2, _) => 0,
        (3, _) => 1,
        _ => return None,
    })
}

/// Decodes the instruction at the start of `bytes`, which is at `at`.
pub fn decode(at: u64, bytes: &[u8]) -> Result<Inst, Error> {
    let mut i = 0usize;
    let get = |i: usize| bytes.get(i).copied().ok_or(Error::Truncated(at + i as u64));
    let mut inst = Inst { at, len: 0, map: 0, op: 0, opsize16: false, rep: false, repne: false, lock: false, rex: 0, reg: 0, ext: 0, rm: None, imm: 0 };
    let mut seg = None;
    let mut addr32 = false;
    loop {
        match get(i)? {
            0xF0 => inst.lock = true,
            0xF2 => inst.repne = true,
            0xF3 => inst.rep = true,
            0x66 => inst.opsize16 = true,
            0x67 => addr32 = true,
            0x64 => seg = Some(Seg::Fs),
            0x65 => seg = Some(Seg::Gs),
            0x26 | 0x2E | 0x36 | 0x3E => {}
            _ => break,
        }
        i += 1;
        if i >= 14 {
            return Err(Error::Undecodable(at));
        }
    }
    if let b @ 0x40..=0x4F = get(i)? {
        inst.rex = b;
        i += 1;
    }
    let mut op = get(i)?;
    i += 1;
    if op == 0x0F {
        inst.map = 1;
        op = get(i)?;
        i += 1;
        if op == 0x38 || op == 0x3A {
            inst.map = if op == 0x38 { 2 } else { 3 };
            op = get(i)?;
            i += 1;
        }
    }
    inst.op = op;
    let rex_r = (inst.rex >> 2 & 1) << 3;
    let rex_x = (inst.rex >> 1 & 1) << 3;
    let rex_b = (inst.rex & 1) << 3;

    if has_modrm(inst.map, op) {
        let modrm = get(i)?;
        i += 1;
        let (md, reg, rm) = (modrm >> 6, modrm >> 3 & 7, modrm & 7);
        inst.ext = reg;
        inst.reg = reg | rex_r;
        if md == 3 {
            inst.rm = Some(Rm::Reg(rm | rex_b));
        } else {
            let mut mem = Mem { base: None, index: None, disp: 0, seg, addr32 };
            let mut disp_len = match md {
                1 => 1,
                2 => 4,
                _ => 0,
            };
            if rm == 4 {
                let sib = get(i)?;
                i += 1;
                let (scale, index, base) = (sib >> 6, sib >> 3 & 7 | rex_x, sib & 7);
                if index != 4 {
                    mem.index = Some((index, scale));
                }
                if base == 5 && md == 0 {
                    disp_len = 4;
                } else {
                    mem.base = Some(Base::Reg(base | rex_b));
                }
            } else if rm == 5 && md == 0 {
                mem.base = Some(Base::Rip);
                disp_len = 4;
            } else {
                mem.base = Some(Base::Reg(rm | rex_b));
            }
            mem.disp = read_signed(bytes, &mut i, disp_len, at)?;
            inst.rm = Some(Rm::Mem(mem));
        }
    } else if inst.map == 0 && matches!(op, 0x50..=0x5F | 0x90..=0x97 | 0xB0..=0xBF) || inst.map == 1 && matches!(op, 0xC8..=0xCF) {
        inst.reg = (op & 7) | rex_b;
    }

    let imm_bytes = imm_len(&inst).ok_or(Error::Undecodable(at))?;
    inst.imm = read_signed(bytes, &mut i, imm_bytes, at)?;
    if i > 15 {
        return Err(Error::Undecodable(at));
    }
    inst.len = i as u8;
    Ok(inst)
}

/// A little-endian value of `len` bytes, sign-extended. An 8-byte one is `movabs`'s, and taken as it is.
fn read_signed(bytes: &[u8], i: &mut usize, len: u8, at: u64) -> Result<i64, Error> {
    let len = usize::from(len);
    let Some(raw) = bytes.get(*i..*i + len) else { return Err(Error::Truncated(at + (*i + len - 1) as u64)) };
    *i += len;
    Ok(match len {
        0 => 0,
        1 => i64::from(raw[0] as i8),
        2 => i64::from(i16::from_le_bytes([raw[0], raw[1]])),
        3 => i64::from(u16::from_le_bytes([raw[0], raw[1]])) | i64::from(raw[2]) << 16,
        4 => i64::from(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])),
        _ => i64::from_le_bytes([raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7]]),
    })
}

/// Control flow, for the runtime.
pub fn flow(inst: &Inst) -> Flow {
    match (inst.map, inst.op) {
        (0, 0x70..=0x7F) | (1, 0x80..=0x8F) | (0, 0xE0..=0xE3) => Flow::Branch(inst.target()),
        (0, 0xE9 | 0xEB) => Flow::Jump(inst.target()),
        (0, 0xE8) => Flow::Call(inst.target()),
        (0, 0xC2 | 0xC3 | 0xCA | 0xCB | 0xCF) => Flow::Indirect,
        (0, 0xFF) if matches!(inst.ext, 2..=5) => Flow::Indirect,
        (1, 0x05) | (0, 0xCD) => Flow::SystemCall,
        (0, 0xCC | 0xF4) | (1, 0x0B) => Flow::Trap,
        _ => Flow::Next,
    }
}

pub struct X86 {
    pub mode: Isa,
}

impl Decoder for X86 {
    fn decode(&self, at: u64, bytes: &[u8]) -> Result<Insn, Error> {
        if self.mode != Isa::X86_64 {
            return Err(Error::Undecodable(at));
        }
        let inst = decode(at, bytes)?;
        Ok(Insn { at, len: inst.len, flow: flow(&inst) })
    }
}
