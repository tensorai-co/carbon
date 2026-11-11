//! Same-ISA rewriting, for sandboxes whose host won't send system calls anywhere but its own kernel.
//!
//! ARM64 → ARM64: every instruction is copied as it is, except the ones that can't run from somewhere else or must not
//! run at all. `svc` and every branch end the block at a `brk`, whose address says (in `exits`) where the program goes
//! next, so no register of the program's is touched. `adr`, `adrp` and literal loads become their absolute
//! equivalents. A program only ever runs rewritten code, so it can't reach a real system call.
//!
//! x86 → x86 isn't built: its variable-length instructions need the full code cache, but no register mapping.

use alloc::vec::Vec;

use crate::emit::asm::Asm;

/// The `brk` every exit is.
pub const EXIT_BRK: u16 = 0xCA7;

/// Where a rewritten block goes when it reaches one of its `brk`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// To a known address.
    Jump(u64),
    /// To the address in a register (`br`, `ret`); with `link`, `x30` becomes it first (`blr`).
    Indirect { reg: u8, link: Option<u64> },
    /// `bl`: `x30` becomes `link`, then to `target`.
    Call { target: u64, link: u64 },
    /// `svc`, carrying on at `next`.
    Syscall { next: u64 },
    /// An instruction this can't rewrite, at `pc`.
    Unsupported { pc: u64 },
    /// Code that couldn't be read at `addr`, part of the instruction at `pc`.
    Fetch { addr: u64, pc: u64 },
}

/// A rewritten block.
#[derive(Debug, Clone, Default)]
pub struct Rewrite {
    pub code: Vec<u8>,
    /// (byte offset in `code`, program address) for each instruction.
    pub map: Vec<(u32, u64)>,
    /// (byte offset in `code` of a `brk`, where it goes).
    pub exits: Vec<(u32, Exit)>,
}

fn sign_extend(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    (i64::from(value) << shift) >> shift
}

struct Writer {
    asm: Asm,
    exits: Vec<(u32, Exit)>,
}

impl Writer {
    fn exit(&mut self, exit: Exit) {
        self.exits.push(((self.asm.len() * 4) as u32, exit));
        self.asm.brk(EXIT_BRK);
    }

    /// A conditional branch, kept, pointed past the not-taken exit to the taken one.
    fn branch(&mut self, word: u32, field_mask: u32, field_shift: u32, taken: u64, next: u64) {
        let offset_two = (2 << field_shift) & field_mask;
        self.asm.put((word & !field_mask) | offset_two);
        self.exit(Exit::Jump(next));
        self.exit(Exit::Jump(taken));
    }
}

/// Rewrites the ARM64 block at `start`, whose bytes as far as they can be read are `bytes`.
pub fn arm64(start: u64, bytes: &[u8], most: usize) -> Rewrite {
    let mut w = Writer { asm: Asm::new(), exits: Vec::new() };
    let mut map = Vec::new();
    let mut pc = start;
    for count in 0.. {
        let offset = (w.asm.len() * 4) as u32;
        if count == most {
            w.exit(Exit::Jump(pc));
            break;
        }
        map.push((offset, pc));
        let at = (pc - start) as usize;
        let Some(word) = bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) else {
            w.exit(Exit::Fetch { addr: start + bytes.len() as u64, pc });
            break;
        };
        let next = pc + 4;
        let target = |bits: u32, shift: u32| pc.wrapping_add((sign_extend((word >> shift) & ((1 << bits) - 1), bits) * 4) as u64);
        match word {
            // svc
            _ if word & 0xFFE0_001F == 0xD400_0001 => {
                w.exit(Exit::Syscall { next });
                break;
            }
            // hvc, smc: nothing a program may do
            _ if word & 0xFFE0_001C == 0xD400_0000 && word & 3 != 0 => {
                w.exit(Exit::Unsupported { pc });
                break;
            }
            // b, bl
            _ if word & 0x7C00_0000 == 0x1400_0000 => {
                let to = target(26, 0);
                if word & 0x8000_0000 != 0 {
                    w.exit(Exit::Call { target: to, link: next });
                } else {
                    w.exit(Exit::Jump(to));
                }
                break;
            }
            // b.cond
            _ if word & 0xFF00_0010 == 0x5400_0000 => {
                w.branch(word, 0x7FFFF << 5, 5, target(19, 5), next);
                break;
            }
            // cbz, cbnz
            _ if word & 0x7E00_0000 == 0x3400_0000 => {
                w.branch(word, 0x7FFFF << 5, 5, target(19, 5), next);
                break;
            }
            // tbz, tbnz
            _ if word & 0x7E00_0000 == 0x3600_0000 => {
                w.branch(word, 0x3FFF << 5, 5, target(14, 5), next);
                break;
            }
            // br, blr, ret (without pointer authentication)
            _ if word & 0xFF9F_FC1F == 0xD61F_0000 => {
                let reg = (word >> 5 & 31) as u8;
                let link = (word >> 21 & 3 == 1).then_some(next);
                w.exit(Exit::Indirect { reg, link });
                break;
            }
            // anything else that branches, with pointer authentication or not, isn't covered yet
            _ if word & 0xFE00_0000 == 0xD600_0000 => {
                w.exit(Exit::Unsupported { pc });
                break;
            }
            // adr, adrp: the address they'd make, as a constant
            _ if word & 0x1F00_0000 == 0x1000_0000 => {
                let imm = sign_extend((word >> 5 & 0x7FFFF) << 2 | (word >> 29 & 3), 21);
                let rd = (word & 31) as u8;
                let value = if word & 0x8000_0000 != 0 {
                    (pc & !0xFFF).wrapping_add((imm << 12) as u64)
                } else {
                    pc.wrapping_add(imm as u64)
                };
                w.asm.mov_imm(rd, value);
            }
            // ldr (literal) into a general register: the address into the register, then a plain load
            _ if word & 0x3B00_0000 == 0x1800_0000 => {
                let rt = (word & 31) as u8;
                let addr = target(19, 5);
                match (word >> 30, word >> 26 & 1) {
                    (0, 0) => {
                        w.asm.mov_imm(rt, addr);
                        w.asm.ldr(32, rt, rt);
                    }
                    (1, 0) => {
                        w.asm.mov_imm(rt, addr);
                        w.asm.ldr(64, rt, rt);
                    }
                    (2, 0) => {
                        w.asm.mov_imm(rt, addr);
                        w.asm.ldrs(32, rt, rt);
                    }
                    // prfm (literal): only a hint
                    (3, 0) => {}
                    _ => {
                        w.exit(Exit::Unsupported { pc });
                        break;
                    }
                }
            }
            // everything else runs as it is
            _ => w.asm.put(word),
        }
        pc = next;
    }
    Rewrite { code: w.asm.code.iter().flat_map(|w| w.to_le_bytes()).collect(), map, exits: w.exits }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(r: &Rewrite) -> Vec<u32> {
        r.code.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    #[test]
    fn plain_code_is_copied_and_svc_is_an_exit() {
        // mov x8, #93; mov x0, #7; svc #0
        let bytes = [0xa8, 0x0b, 0x80, 0xd2, 0xe0, 0x00, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4];
        let r = arm64(0x1000, &bytes, 64);
        assert_eq!(words(&r), [0xd280_0ba8, 0xd280_00e0, 0xd420_0000 | u32::from(EXIT_BRK) << 5]);
        assert_eq!(r.exits, [(8, Exit::Syscall { next: 0x100c })]);
    }

    #[test]
    fn a_conditional_branch_has_two_exits() {
        // cbz x1, #+16
        let r = arm64(0x2000, &[0x81, 0x00, 0x00, 0xb4], 64);
        assert_eq!(words(&r)[0], 0xb400_0041); // cbz x1, #+8
        assert_eq!(r.exits, [(4, Exit::Jump(0x2004)), (8, Exit::Jump(0x2010))]);
    }

    #[test]
    fn calls_returns_and_adrp() {
        // adrp x0, #0x1000; bl #-8
        let r = arm64(0x1_0000, &[0x00, 0x00, 0x00, 0xb0, 0xfe, 0xff, 0xff, 0x97], 64);
        assert_eq!(r.exits, [(r.code.len() as u32 - 4, Exit::Call { target: 0xFFFC, link: 0x1_0008 })]);
        // ret
        let r = arm64(0x1000, &[0xc0, 0x03, 0x5f, 0xd6], 64);
        assert_eq!(r.exits, [(0, Exit::Indirect { reg: 30, link: None })]);
    }
}
