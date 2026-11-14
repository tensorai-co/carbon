//! Shader model 4 and 5's tokens: the version and length, then instructions, each an opcode token (with extended
//! tokens after it), then its operands and, for declarations, words of their own.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::fail;
use crate::Error;

/// Opcodes, as `d3d11TokenizedProgramFormat.hpp` numbers them.
pub(super) mod op {
    pub const ADD: u32 = 0;
    pub const AND: u32 = 1;
    pub const BREAK: u32 = 2;
    pub const BREAKC: u32 = 3;
    pub const CASE: u32 = 6;
    pub const CONTINUE: u32 = 7;
    pub const CONTINUEC: u32 = 8;
    pub const DEFAULT: u32 = 10;
    pub const DERIV_RTX: u32 = 11;
    pub const DERIV_RTY: u32 = 12;
    pub const DISCARD: u32 = 13;
    pub const DIV: u32 = 14;
    pub const DP2: u32 = 15;
    pub const DP3: u32 = 16;
    pub const DP4: u32 = 17;
    pub const ELSE: u32 = 18;
    pub const ENDIF: u32 = 21;
    pub const ENDLOOP: u32 = 22;
    pub const ENDSWITCH: u32 = 23;
    pub const EQ: u32 = 24;
    pub const EXP: u32 = 25;
    pub const FRC: u32 = 26;
    pub const FTOI: u32 = 27;
    pub const FTOU: u32 = 28;
    pub const GE: u32 = 29;
    pub const IADD: u32 = 30;
    pub const IF: u32 = 31;
    pub const IEQ: u32 = 32;
    pub const IGE: u32 = 33;
    pub const ILT: u32 = 34;
    pub const IMAD: u32 = 35;
    pub const IMAX: u32 = 36;
    pub const IMIN: u32 = 37;
    pub const IMUL: u32 = 38;
    pub const INE: u32 = 39;
    pub const INEG: u32 = 40;
    pub const ISHL: u32 = 41;
    pub const ISHR: u32 = 42;
    pub const ITOF: u32 = 43;
    pub const LD: u32 = 45;
    pub const LOG: u32 = 47;
    pub const LOOP: u32 = 48;
    pub const LT: u32 = 49;
    pub const MAD: u32 = 50;
    pub const MIN: u32 = 51;
    pub const MAX: u32 = 52;
    pub const CUSTOMDATA: u32 = 53;
    pub const MOV: u32 = 54;
    pub const MOVC: u32 = 55;
    pub const MUL: u32 = 56;
    pub const NE: u32 = 57;
    pub const NOP: u32 = 58;
    pub const NOT: u32 = 59;
    pub const OR: u32 = 60;
    pub const RET: u32 = 62;
    pub const RETC: u32 = 63;
    pub const ROUND_NE: u32 = 64;
    pub const ROUND_NI: u32 = 65;
    pub const ROUND_PI: u32 = 66;
    pub const ROUND_Z: u32 = 67;
    pub const RSQ: u32 = 68;
    pub const SAMPLE: u32 = 69;
    pub const SAMPLE_L: u32 = 72;
    pub const SAMPLE_B: u32 = 74;
    pub const SQRT: u32 = 75;
    pub const SWITCH: u32 = 76;
    pub const SINCOS: u32 = 77;
    pub const UDIV: u32 = 78;
    pub const ULT: u32 = 79;
    pub const UGE: u32 = 80;
    pub const UMUL: u32 = 81;
    pub const UMAD: u32 = 82;
    pub const UMAX: u32 = 83;
    pub const UMIN: u32 = 84;
    pub const USHR: u32 = 85;
    pub const UTOF: u32 = 86;
    pub const XOR: u32 = 87;
    pub const DCL_RESOURCE: u32 = 88;
    pub const DCL_CONSTANT_BUFFER: u32 = 89;
    pub const DCL_SAMPLER: u32 = 90;
    pub const DCL_INDEX_RANGE: u32 = 91;
    pub const DCL_INPUT: u32 = 95;
    pub const DCL_INPUT_SGV: u32 = 96;
    pub const DCL_INPUT_SIV: u32 = 97;
    pub const DCL_INPUT_PS: u32 = 98;
    pub const DCL_INPUT_PS_SGV: u32 = 99;
    pub const DCL_INPUT_PS_SIV: u32 = 100;
    pub const DCL_OUTPUT: u32 = 101;
    pub const DCL_OUTPUT_SGV: u32 = 102;
    pub const DCL_OUTPUT_SIV: u32 = 103;
    pub const DCL_TEMPS: u32 = 104;
    pub const DCL_INDEXABLE_TEMP: u32 = 105;
    pub const DCL_GLOBAL_FLAGS: u32 = 106;
    pub const DERIV_RTX_COARSE: u32 = 122;
    pub const DERIV_RTX_FINE: u32 = 123;
    pub const DERIV_RTY_COARSE: u32 = 124;
    pub const DERIV_RTY_FINE: u32 = 125;
    pub const RCP: u32 = 129;
    pub const COUNTBITS: u32 = 134;
    pub const SWAPC: u32 = 142;
    pub const DCL_THREAD_GROUP: u32 = 155;
    pub const DCL_UAV_TYPED: u32 = 156;
    pub const DCL_UAV_RAW: u32 = 157;
    pub const DCL_UAV_STRUCTURED: u32 = 158;
    pub const DCL_TGSM_RAW: u32 = 159;
    pub const DCL_TGSM_STRUCTURED: u32 = 160;
    pub const DCL_RESOURCE_RAW: u32 = 161;
    pub const DCL_RESOURCE_STRUCTURED: u32 = 162;
    pub const LD_RAW: u32 = 165;
    pub const STORE_RAW: u32 = 166;
    pub const LD_STRUCTURED: u32 = 167;
    pub const STORE_STRUCTURED: u32 = 168;
    pub const ATOMIC_AND: u32 = 169;
    pub const ATOMIC_OR: u32 = 170;
    pub const ATOMIC_XOR: u32 = 171;
    pub const ATOMIC_CMP_STORE: u32 = 172;
    pub const ATOMIC_IADD: u32 = 173;
    pub const ATOMIC_IMAX: u32 = 174;
    pub const ATOMIC_IMIN: u32 = 175;
    pub const ATOMIC_UMAX: u32 = 176;
    pub const ATOMIC_UMIN: u32 = 177;
    pub const IMM_ATOMIC_IADD: u32 = 180;
    pub const IMM_ATOMIC_AND: u32 = 181;
    pub const IMM_ATOMIC_OR: u32 = 182;
    pub const IMM_ATOMIC_XOR: u32 = 183;
    pub const IMM_ATOMIC_EXCH: u32 = 184;
    pub const IMM_ATOMIC_CMP_EXCH: u32 = 185;
    pub const IMM_ATOMIC_IMAX: u32 = 186;
    pub const IMM_ATOMIC_IMIN: u32 = 187;
    pub const IMM_ATOMIC_UMAX: u32 = 188;
    pub const IMM_ATOMIC_UMIN: u32 = 189;
    pub const SYNC: u32 = 190;
}

/// Operand types.
pub(super) mod ty {
    pub const TEMP: u32 = 0;
    pub const INPUT: u32 = 1;
    pub const OUTPUT: u32 = 2;
    pub const INDEXABLE_TEMP: u32 = 3;
    pub const IMMEDIATE32: u32 = 4;
    pub const IMMEDIATE64: u32 = 5;
    pub const RESOURCE: u32 = 7;
    pub const CONSTANT_BUFFER: u32 = 8;
    pub const IMMEDIATE_CONSTANT_BUFFER: u32 = 9;
    pub const OUTPUT_DEPTH: u32 = 12;
    pub const NULL: u32 = 13;
    pub const UAV: u32 = 30;
    pub const TGSM: u32 = 31;
    pub const THREAD_ID: u32 = 32;
    pub const THREAD_GROUP_ID: u32 = 33;
    pub const THREAD_ID_IN_GROUP: u32 = 34;
    pub const THREAD_ID_IN_GROUP_FLATTENED: u32 = 36;
}

/// Which of a register's components an operand names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Select {
    /// A destination's: the components written, x lowest.
    Mask(u8),
    /// A source's: the component read for each of x, y, z and w.
    Swizzle([u8; 4]),
}

/// One index of an operand: a number, plus a register's component when it's relative.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Index {
    pub imm: u32,
    pub rel: Option<Box<Operand>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Operand {
    pub ty: u32,
    /// 0, 1 or 4.
    pub components: u8,
    pub select: Select,
    pub index: Vec<Index>,
    /// 0 none, 1 negate, 2 absolute value, 3 both (the absolute value negated).
    pub modifier: u8,
    /// An immediate's components, as bits.
    pub imm: [u32; 4],
}

impl Operand {
    /// The source component read for component `c`.
    pub fn component(&self, c: usize) -> u8 {
        match self.select {
            Select::Swizzle(s) => s[c],
            Select::Mask(_) => c as u8,
        }
    }

    /// The destination's mask (a source's: every component).
    pub fn mask(&self) -> u8 {
        match self.select {
            Select::Mask(m) => m,
            Select::Swizzle(_) => 0xf,
        }
    }

    pub fn reg(&self) -> u32 {
        self.index.first().map_or(0, |i| i.imm)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Instruction {
    pub op: u32,
    /// The opcode token, for the fields declarations and some instructions keep in it.
    pub token: u32,
    pub saturate: bool,
    /// The test of `if`, `breakc` and the like: nonzero, or zero.
    pub nonzero: bool,
    /// Texel offsets from a sample-controls extended token.
    pub offsets: [i8; 3],
    pub operands: Vec<Operand>,
    /// A declaration's own words after its operands.
    pub words: Vec<u32>,
}

/// The program's stage, version and instructions.
pub(super) struct Program {
    /// 0 pixel, 1 vertex, 2 geometry, 3 hull, 4 domain, 5 compute.
    pub kind: u32,
    pub major: u32,
    pub minor: u32,
    pub instructions: Vec<Instruction>,
}

struct Reader<'a> {
    words: &'a [u32],
    at: usize,
    end: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Result<u32, Error> {
        if self.at >= self.end {
            return fail(alloc::format!("an instruction runs past its length at word {}", self.at));
        }
        self.at += 1;
        Ok(self.words[self.at - 1])
    }

    fn operand(&mut self) -> Result<Operand, Error> {
        let t = self.next()?;
        let components = match t & 3 {
            0 => 0,
            1 => 1,
            2 => 4,
            _ => return fail("an operand of N components"),
        };
        let bits = (t >> 4) & 0xff;
        let select = if components == 4 {
            match (t >> 2) & 3 {
                0 => Select::Mask(bits as u8 & 0xf),
                1 => Select::Swizzle([0, 1, 2, 3].map(|i| ((bits >> (2 * i)) & 3) as u8)),
                2 => Select::Swizzle([(bits & 3) as u8; 4]),
                m => return fail(alloc::format!("component selection mode {m}")),
            }
        } else {
            Select::Swizzle([0; 4])
        };
        let ty = (t >> 12) & 0xff;
        let dims = (t >> 20) & 3;
        let mut modifier = 0;
        let mut extended = t >> 31 != 0;
        while extended {
            let e = self.next()?;
            if e & 0x3f == 1 {
                modifier = ((e >> 6) & 0xff) as u8;
            }
            extended = e >> 31 != 0;
        }
        let mut imm = [0; 4];
        match ty {
            ty::IMMEDIATE32 => {
                for slot in imm.iter_mut().take(components as usize) {
                    *slot = self.next()?;
                }
                if components == 1 {
                    imm = [imm[0]; 4];
                }
            }
            ty::IMMEDIATE64 => return fail("64-bit immediates (doubles) aren't read yet"),
            _ => {}
        }
        let mut index = Vec::new();
        for d in 0..dims {
            let rep = (t >> (22 + 3 * d)) & 7;
            let (imm, rel) = match rep {
                0 => (self.next()?, false),
                1 => {
                    let lo = self.next()?;
                    self.next()?;
                    (lo, false)
                }
                2 => (0, true),
                3 => (self.next()?, true),
                4 => {
                    let lo = self.next()?;
                    self.next()?;
                    (lo, true)
                }
                r => return fail(alloc::format!("index representation {r}")),
            };
            let rel = if rel { Some(Box::new(self.operand()?)) } else { None };
            index.push(Index { imm, rel });
        }
        Ok(Operand { ty, components, select, index, modifier, imm })
    }
}

/// How many operands a declaration has before its own words; `None` for an instruction, whose words are all
/// operands.
fn declaration_operands(op: u32) -> Option<usize> {
    Some(match op {
        op::DCL_TEMPS | op::DCL_INDEXABLE_TEMP | op::DCL_GLOBAL_FLAGS | op::DCL_THREAD_GROUP => 0,
        92..=94 | 143..=154 | 206 => 0,
        op::DCL_RESOURCE..=op::DCL_INDEX_RANGE | op::DCL_INPUT..=op::DCL_OUTPUT_SIV => 1,
        op::DCL_UAV_TYPED..=op::DCL_RESOURCE_STRUCTURED => 1,
        _ => return None,
    })
}

/// The program chunk's tokens.
pub(super) fn read(bytes: &[u8]) -> Result<Program, Error> {
    if !bytes.len().is_multiple_of(4) || bytes.len() < 8 {
        return fail("the program chunk isn't whole words");
    }
    let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
    let version = words[0];
    let length = (words[1] as usize).min(words.len());
    let (kind, major, minor) = (version >> 16, (version >> 4) & 0xf, version & 0xf);
    let mut instructions = Vec::new();
    let mut at = 2;
    while at < length {
        let token = words[at];
        let op = token & 0x7ff;
        if op == op::CUSTOMDATA {
            // A class, then the length (in words, these two included), then the data.
            let len = *words.get(at + 1).ok_or_else(|| Error { message: "custom data with no length".into() })? as usize;
            if len < 2 || at + len > length {
                return fail("custom data past the program's end");
            }
            instructions.push(Instruction {
                op,
                token,
                saturate: false,
                nonzero: false,
                offsets: [0; 3],
                operands: Vec::new(),
                words: words[at + 2..at + len].to_vec(),
            });
            at += len;
            continue;
        }
        let len = ((token >> 24) & 0x7f) as usize;
        if len == 0 || at + len > length {
            return fail(alloc::format!("instruction {op} at word {at} has length {len}"));
        }
        let mut r = Reader { words: &words, at: at + 1, end: at + len };
        let mut offsets = [0; 3];
        let mut extended = token >> 31 != 0;
        while extended {
            let e = r.next()?;
            if e & 0x3f == 1 {
                let field = |shift: u32| (((e >> shift) & 0xf) as i8) << 4 >> 4;
                offsets = [field(9), field(13), field(17)];
            }
            extended = e >> 31 != 0;
        }
        let mut operands = Vec::new();
        let mut own = Vec::new();
        match declaration_operands(op) {
            Some(n) => {
                for _ in 0..n {
                    operands.push(r.operand()?);
                }
                while r.at < r.end {
                    own.push(r.next()?);
                }
            }
            None => {
                while r.at < r.end {
                    operands.push(r.operand()?);
                }
            }
        }
        instructions.push(Instruction {
            op,
            token,
            saturate: token & (1 << 13) != 0,
            nonzero: token & (1 << 18) != 0,
            offsets,
            operands,
            words: own,
        });
        at += len;
    }
    Ok(Program { kind, major, minor, instructions })
}
