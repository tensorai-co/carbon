//! Where a call's arguments are, for each ISA and calling convention: for reading a call a program made into its
//! kernel (atlas's host exports), and for making a call into a program (atlas's upcalls).
//!
//! Linux's system-call registers aren't here: exec's `Stop::Syscall` already gives a call's number and arguments.

use exec::Regs;
use exec::isa::x86::{R8, R9, RAX, RCX, RDI, RDX, RSI, RSP};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    /// ARM64's procedure call standard, on Linux and Windows alike: x0–x7, then the stack.
    Arm64,
    /// Windows x64: rcx, rdx, r8, r9, then the stack above 32 bytes of home space.
    Win64,
    /// System V x86-64 (Linux): rdi, rsi, rdx, rcx, r8, r9, then the stack.
    SysV64,
    /// 32-bit x86 `stdcall`: everything on the stack; the callee pops it.
    Stdcall,
    /// 32-bit x86 `cdecl`: everything on the stack; the caller pops it.
    Cdecl,
}

impl Convention {
    /// A pointer's size, in bytes.
    pub fn width(self) -> u64 {
        match self {
            Convention::Stdcall | Convention::Cdecl => 4,
            _ => 8,
        }
    }

    /// Argument `index` of a call that has just been made: the callee is about to run its first instruction, so on x86
    /// the return address is at the stack pointer. `stack` reads a pointer-sized value of the program's memory.
    pub fn arg(self, regs: &Regs, index: usize, stack: &mut impl FnMut(u64) -> Option<u64>) -> Option<u64> {
        match (self, regs) {
            (Convention::Arm64, Regs::Arm64(r)) => {
                if index < 8 { Some(r.x[index]) } else { stack(r.sp + 8 * (index as u64 - 8)) }
            }
            (Convention::Win64, Regs::X86(r)) => match index {
                0..4 => Some(r.gpr[[RCX, RDX, R8, R9][index]]),
                _ => stack(r.gpr[RSP] + 8 + 32 + 8 * (index as u64 - 4)),
            },
            (Convention::SysV64, Regs::X86(r)) => match index {
                0..6 => Some(r.gpr[[RDI, RSI, RDX, RCX, R8, R9][index]]),
                _ => stack(r.gpr[RSP] + 8 + 8 * (index as u64 - 6)),
            },
            (Convention::Stdcall | Convention::Cdecl, Regs::X86(r)) => stack((r.gpr[RSP] + 4 + 4 * index as u64) & 0xffff_ffff),
            _ => None,
        }
    }

    /// Sets the value a call returns.
    pub fn set_return(self, regs: &mut Regs, value: u64) {
        match regs {
            Regs::Arm64(r) => r.x[0] = value,
            Regs::X86(r) if self.width() == 4 => r.gpr[RAX] = value & 0xffff_ffff,
            Regs::X86(r) => r.gpr[RAX] = value,
        }
    }

    /// Sets up a call to `target` with `args`, to return to `ret`. Changes the registers, and gives the new stack pointer
    /// and the bytes to write there (stack arguments, and on x86 the return address).
    pub fn prepare(self, regs: &mut Regs, target: u64, args: &[u64], ret: u64) -> (u64, Vec<u8>) {
        let sp = regs.sp();
        match regs {
            Regs::Arm64(r) => {
                let stacked: Vec<u64> = args.iter().skip(8).copied().collect();
                for (i, &a) in args.iter().take(8).enumerate() {
                    r.x[i] = a;
                }
                r.x[30] = ret;
                r.pc = target;
                let new_sp = (sp - 8 * stacked.len() as u64) & !15;
                r.sp = new_sp;
                (new_sp, words(&stacked, 8))
            }
            Regs::X86(r) => {
                let (in_regs, home, width): (&[usize], u64, u64) = match self {
                    Convention::Win64 => (&[RCX, RDX, R8, R9], 32, 8),
                    Convention::SysV64 => (&[RDI, RSI, RDX, RCX, R8, R9], 0, 8),
                    _ => (&[], 0, 4),
                };
                for (i, &a) in args.iter().take(in_regs.len()).enumerate() {
                    r.gpr[in_regs[i]] = a;
                }
                let stacked: Vec<u64> = args.iter().skip(in_regs.len()).copied().collect();
                // The return address, home space, then the stack arguments; aligned as if by a `call` from an aligned
                // stack.
                let above = home + width * stacked.len() as u64;
                let base = (sp - above) & !15;
                let new_sp = base - width;
                let mut bytes = words(&[ret], width);
                bytes.extend(vec![0; home as usize]);
                bytes.extend(words(&stacked, width));
                r.gpr[RSP] = new_sp;
                r.rip = target;
                (new_sp, bytes)
            }
        }
    }
}

fn words(values: &[u64], width: u64) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()[..width as usize].to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use exec::Isa;

    #[test]
    fn win64_round_trip() {
        let mut regs = Regs::new(Isa::X86_64);
        regs.set_sp(0x10_0000);
        let args = [1, 2, 3, 4, 5, 6];
        let (sp, bytes) = Convention::Win64.prepare(&mut regs, 0x4000, &args, 0x5000);
        assert_eq!((sp + 8) % 16, 0);
        let mut read = |addr: u64| {
            let at = (addr - sp) as usize;
            bytes.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        };
        for (i, &a) in args.iter().enumerate() {
            assert_eq!(Convention::Win64.arg(&regs, i, &mut read), Some(a));
        }
        assert_eq!(read(sp), Some(0x5000));
    }

    #[test]
    fn stdcall_round_trip() {
        let mut regs = Regs::new(Isa::X86_32);
        regs.set_sp(0x10_0000);
        let (sp, bytes) = Convention::Stdcall.prepare(&mut regs, 0x4000, &[7, 8], 0x5000);
        let mut read = |addr: u64| {
            let at = (addr - sp) as usize;
            bytes.get(at..at + 4).map(|b| u64::from(u32::from_le_bytes(b.try_into().unwrap())))
        };
        assert_eq!(Convention::Stdcall.arg(&regs, 1, &mut read), Some(8));
    }
}
