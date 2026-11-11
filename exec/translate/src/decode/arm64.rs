//! ARM64: fixed 4-byte instructions, so only control flow needs decoding to find blocks.

use super::{Decoder, Flow, Insn};
use crate::Error;

pub struct Arm64;

impl Decoder for Arm64 {
    fn decode(&self, at: u64, bytes: &[u8]) -> Result<Insn, Error> {
        let Some(word) = bytes.get(..4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes) else {
            return Err(Error::Undecodable(at));
        };
        let offset = |bits: u32, shift: u32| -> u64 {
            let signed = ((word >> shift) & ((1 << bits) - 1)) as i64;
            let signed = (signed << (64 - bits)) >> (64 - bits);
            at.wrapping_add((signed * 4) as u64)
        };
        let flow = match word {
            // svc #imm
            w if w & 0xffe0_001f == 0xd400_0001 => Flow::SystemCall,
            // brk #imm, udf
            w if w & 0xffe0_001f == 0xd420_0000 || w & 0xffff_0000 == 0 => Flow::Trap,
            // b
            w if w & 0xfc00_0000 == 0x1400_0000 => Flow::Jump(offset(26, 0)),
            // bl
            w if w & 0xfc00_0000 == 0x9400_0000 => Flow::Call(offset(26, 0)),
            // b.cond, cbz/cbnz
            w if w & 0xff00_0010 == 0x5400_0000 || w & 0x7e00_0000 == 0x3400_0000 => Flow::Branch(offset(19, 5)),
            // tbz/tbnz
            w if w & 0x7e00_0000 == 0x3600_0000 => Flow::Branch(offset(14, 5)),
            // br, blr, ret and their authenticated forms
            w if w & 0xfe00_0000 == 0xd600_0000 => Flow::Indirect,
            _ => Flow::Next,
        };
        Ok(Insn { at, len: 4, flow })
    }
}
