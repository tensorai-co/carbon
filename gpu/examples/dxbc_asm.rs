//! A small DXBC assembler for the suite's DXBC tests: shader model 5.0 tokens and a container around them, written by
//! hand from `d3d11TokenizedProgramFormat.hpp`'s layout. It's the reader's own understanding of the format, so these
//! tests check the translation and the devices, not the reading; programs from a real HLSL compiler check that.
#![allow(dead_code)]

/// Operand types.
pub const TEMP: u32 = 0;
pub const INPUT: u32 = 1;
pub const OUTPUT: u32 = 2;
pub const XTEMP: u32 = 3;
pub const IMM: u32 = 4;
pub const SAMPLER: u32 = 6;
pub const RESOURCE: u32 = 7;
pub const CB: u32 = 8;
pub const ICB: u32 = 9;
pub const NULL: u32 = 13;
pub const UAV: u32 = 30;
pub const TGSM: u32 = 31;
pub const THREAD_ID: u32 = 32;
pub const THREAD_ID_IN_GROUP: u32 = 34;
pub const THREAD_ID_FLAT: u32 = 36;

/// An operand's words.
#[derive(Clone)]
pub struct Opnd(pub Vec<u32>);

fn mask_bits(m: &str) -> u32 {
    m.chars().map(|c| match c {
        'x' => 1,
        'y' => 2,
        'z' => 4,
        _ => 8,
    }).sum()
}

fn swizzle_bits(s: &str) -> u32 {
    let lanes: Vec<u32> = s.chars().map(|c| match c {
        'x' => 0,
        'y' => 1,
        'z' => 2,
        _ => 3,
    }).collect();
    (0..4).map(|i| lanes[i.min(lanes.len() - 1)] << (2 * i)).sum()
}

/// A destination: four components, a mask.
pub fn dst(ty: u32, index: &[u32], mask: &str) -> Opnd {
    let t = 2 | mask_bits(mask) << 4 | ty << 12 | (index.len() as u32) << 20;
    Opnd([&[t][..], index].concat())
}

/// A source: four components, a swizzle.
pub fn src(ty: u32, index: &[u32], swizzle: &str) -> Opnd {
    let t = 2 | 1 << 2 | swizzle_bits(swizzle) << 4 | ty << 12 | (index.len() as u32) << 20;
    Opnd([&[t][..], index].concat())
}

/// A source of one component (select-1), as `if` and indices take.
pub fn one(ty: u32, index: &[u32], c: char) -> Opnd {
    let t = 2 | 2 << 2 | swizzle_bits(&c.to_string()) << 4 | ty << 12 | (index.len() as u32) << 20;
    Opnd([&[t][..], index].concat())
}

/// The operand with its index `d` made relative: the number it has, plus a register's component.
pub fn with_relative(o: Opnd, d: usize, rel: Opnd) -> Opnd {
    let mut w = o.0;
    w[0] |= 3 << (22 + 3 * d);
    let at = 2 + d;
    let tail = w.split_off(at);
    w.extend(rel.0);
    w.extend(tail);
    Opnd(w)
}

pub fn r(n: u32, mask: &str) -> Opnd {
    dst(TEMP, &[n], mask)
}

pub fn rs(n: u32, swizzle: &str) -> Opnd {
    src(TEMP, &[n], swizzle)
}

pub fn l4(v: [u32; 4]) -> Opnd {
    Opnd(vec![2 | IMM << 12, v[0], v[1], v[2], v[3]])
}

pub fn l1(v: u32) -> Opnd {
    Opnd(vec![1 | IMM << 12, v])
}

pub fn f1(v: f32) -> Opnd {
    l1(v.to_bits())
}

pub fn f4(v: [f32; 4]) -> Opnd {
    l4(v.map(f32::to_bits))
}

/// An operand with no components and one index: a sampler, or a resource in a declaration.
pub fn slot(ty: u32, n: u32) -> Opnd {
    Opnd(vec![ty << 12 | 1 << 20, n])
}

pub fn null() -> Opnd {
    Opnd(vec![NULL << 12])
}

/// The operand with a modifier: 1 negate, 2 absolute value, 3 both.
pub fn modify(o: Opnd, modifier: u32) -> Opnd {
    let mut w = o.0;
    w[0] |= 1 << 31;
    w.insert(1, 1 | modifier << 6);
    Opnd(w)
}

pub fn neg(o: Opnd) -> Opnd {
    modify(o, 1)
}

pub fn abs(o: Opnd) -> Opnd {
    modify(o, 2)
}

/// Opcodes.
pub mod op {
    pub const ADD: u32 = 0;
    pub const AND: u32 = 1;
    pub const BREAK: u32 = 2;
    pub const BREAKC: u32 = 3;
    pub const CASE: u32 = 6;
    pub const CONTINUE: u32 = 7;
    pub const CONTINUEC: u32 = 8;
    pub const DEFAULT: u32 = 10;
    pub const DERIV_RTX: u32 = 11;
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
    pub const NOT: u32 = 59;
    pub const OR: u32 = 60;
    pub const RET: u32 = 62;
    pub const ROUND_NE: u32 = 64;
    pub const ROUND_NI: u32 = 65;
    pub const ROUND_PI: u32 = 66;
    pub const ROUND_Z: u32 = 67;
    pub const SAMPLE: u32 = 69;
    pub const SAMPLE_L: u32 = 72;
    pub const SQRT: u32 = 75;
    pub const SWITCH: u32 = 76;
    pub const UDIV: u32 = 78;
    pub const ULT: u32 = 79;
    pub const UGE: u32 = 80;
    pub const UMUL: u32 = 81;
    pub const UMAX: u32 = 83;
    pub const UMIN: u32 = 84;
    pub const USHR: u32 = 85;
    pub const UTOF: u32 = 86;
    pub const XOR: u32 = 87;
    pub const DCL_RESOURCE: u32 = 88;
    pub const DCL_CONSTANT_BUFFER: u32 = 89;
    pub const DCL_SAMPLER: u32 = 90;
    pub const DCL_INPUT: u32 = 95;
    pub const DCL_INPUT_SGV: u32 = 96;
    pub const DCL_INPUT_PS: u32 = 98;
    pub const DCL_INPUT_PS_SGV: u32 = 99;
    pub const DCL_INPUT_PS_SIV: u32 = 100;
    pub const DCL_OUTPUT: u32 = 101;
    pub const DCL_OUTPUT_SIV: u32 = 103;
    pub const DCL_TEMPS: u32 = 104;
    pub const DCL_INDEXABLE_TEMP: u32 = 105;
    pub const DCL_GLOBAL_FLAGS: u32 = 106;
    pub const COUNTBITS: u32 = 134;
    pub const DCL_THREAD_GROUP: u32 = 155;
    pub const DCL_UAV_RAW: u32 = 157;
    pub const DCL_UAV_STRUCTURED: u32 = 158;
    pub const DCL_TGSM_RAW: u32 = 159;
    pub const DCL_RESOURCE_RAW: u32 = 161;
    pub const DCL_RESOURCE_STRUCTURED: u32 = 162;
    pub const LD_RAW: u32 = 165;
    pub const STORE_RAW: u32 = 166;
    pub const LD_STRUCTURED: u32 = 167;
    pub const STORE_STRUCTURED: u32 = 168;
    pub const ATOMIC_UMAX: u32 = 176;
    pub const IMM_ATOMIC_IADD: u32 = 180;
    pub const SYNC: u32 = 190;
}

/// Bits of an opcode token.
pub const SAT: u32 = 1 << 13;
pub const NONZERO: u32 = 1 << 18;

/// A signature element: a semantic, its index, system value, component type (1 uint, 2 sint, 3 float), register and
/// mask.
pub struct Sig {
    pub name: &'static str,
    pub index: u32,
    pub system: u32,
    pub component: u32,
    pub register: u32,
    pub mask: u8,
}

pub struct Asm {
    kind: u32,
    words: Vec<u32>,
}

impl Asm {
    /// 0 pixel, 1 vertex, 5 compute.
    pub fn new(kind: u32) -> Asm {
        Asm { kind, words: Vec::new() }
    }

    /// An instruction: its opcode and the token's other bits, its operands, then a declaration's own words.
    pub fn op(&mut self, opcode: u32, bits: u32, operands: &[Opnd], words: &[u32]) -> &mut Asm {
        let body: Vec<u32> = operands.iter().flat_map(|o| o.0.iter().copied()).chain(words.iter().copied()).collect();
        self.words.push(opcode | bits | ((body.len() as u32 + 1) << 24));
        self.words.extend(body);
        self
    }

    pub fn i(&mut self, opcode: u32, operands: &[Opnd]) -> &mut Asm {
        self.op(opcode, 0, operands, &[])
    }

    /// The immediate constant buffer: custom data of class 3.
    pub fn icb(&mut self, data: &[u32]) -> &mut Asm {
        self.words.push(op::CUSTOMDATA | 3 << 11);
        self.words.push(data.len() as u32 + 2);
        self.words.extend(data);
        self
    }

    fn signature(elements: &[Sig]) -> Vec<u8> {
        let mut table = Vec::new();
        let base = 8 + 24 * elements.len();
        let mut out = Vec::new();
        out.extend((elements.len() as u32).to_le_bytes());
        out.extend(8u32.to_le_bytes());
        for e in elements {
            let at = base + table.len();
            table.extend(e.name.as_bytes());
            table.push(0);
            for w in [at as u32, e.index, e.system, e.component, e.register, e.mask as u32 | (e.mask as u32) << 8] {
                out.extend(w.to_le_bytes());
            }
        }
        out.extend(table);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }

    /// The container: signatures and the program.
    pub fn finish(&self, inputs: &[Sig], outputs: &[Sig]) -> Vec<u8> {
        let mut program = Vec::new();
        for w in [self.kind << 16 | 5 << 4, self.words.len() as u32 + 2].iter().chain(&self.words) {
            program.extend(w.to_le_bytes());
        }
        let chunks: [(&[u8; 4], Vec<u8>); 3] = [(b"ISGN", Self::signature(inputs)), (b"OSGN", Self::signature(outputs)), (b"SHEX", program)];
        let header = 32 + 4 * chunks.len();
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for (code, data) in &chunks {
            offsets.push((header + body.len()) as u32);
            body.extend(*code);
            body.extend((data.len() as u32).to_le_bytes());
            body.extend(data);
        }
        let mut out = b"DXBC".to_vec();
        out.extend([0u8; 16]);
        out.extend(1u32.to_le_bytes());
        out.extend(((header + body.len()) as u32).to_le_bytes());
        out.extend((chunks.len() as u32).to_le_bytes());
        for o in offsets {
            out.extend(o.to_le_bytes());
        }
        out.extend(body);
        out
    }
}
