//! Running one instruction set's code on another (x86 on ARM64, ARM64 on x86-64), and rewriting code for the same one
//! so its system calls reach a sandbox's gate.
//!
//! A direction is a decoder plus an emitter. Everything else is the runtime's and the same for every direction: the
//! code cache, finding a block's translation, forgetting translations when a program writes its own code, and mapping
//! a translated address back to the program's.
//!
//! Built so far: x86-64 → ARM64 (`emit::arm64`): x86-64's integer instructions, SSE2, and x87 at double precision,
//! as Debian's own programs use them (`apt`, `dpkg`, Python, git). What it doesn't cover becomes an undefined-instruction
//! stop at that address, which lambda reports with the instruction's bytes.
//! ARM64 → x86-64 and same-ISA rewriting are still to come.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod decode;
pub mod emit;
pub mod rewrite;
pub mod runtime;
#[cfg(test)]
mod tests;

/// An instruction set, as translation sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Isa {
    Arm64,
    X86_32,
    X86_64,
}

/// One way of running code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction {
    pub from: Isa,
    pub to: Isa,
}

impl Direction {
    /// Same-ISA rewriting: most instructions are copied as they are.
    pub const fn is_rewrite(self) -> bool {
        matches!((self.from, self.to), (Isa::Arm64, Isa::Arm64) | (Isa::X86_64 | Isa::X86_32, Isa::X86_64))
    }
}

/// How translated code keeps the program's memory ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    /// The host is at least as strong (x86 hosts), or the CPU has a TSO mode on (Apple silicon).
    Hardware,
    /// Barriers around loads and stores, for x86 programs on ARM64 without TSO.
    Barriers,
}

/// Why something couldn't be translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// This direction isn't built yet.
    Unsupported(Direction),
    /// An instruction the decoder doesn't know, at this address.
    Undecodable(u64),
    /// An instruction runs past the bytes there are: the first one missing is at this address.
    Truncated(u64),
    /// The code cache is full.
    Full,
}
