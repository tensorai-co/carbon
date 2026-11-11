//! Decoders: one instruction at a time, into what the runtime and emitters need to know about it.

pub mod arm64;
pub mod x86;

/// What a decoded instruction does to control flow, which is all the runtime needs to find a block's end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// Always goes to a known address.
    Jump(u64),
    /// Goes to a known address or falls through.
    Branch(u64),
    /// Goes somewhere only known when it runs: a register, memory, a return.
    Indirect,
    /// Calls a known address.
    Call(u64),
    /// A system call: `syscall`, `int 0x2e`, `int 0x80`, `svc`.
    SystemCall,
    /// Something that stops the program: a breakpoint, an undefined instruction.
    Trap,
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub at: u64,
    pub len: u8,
    pub flow: Flow,
}

/// A decoder for one instruction set.
pub trait Decoder {
    /// Decodes the instruction at the start of `bytes`, which is at `at` in the program.
    fn decode(&self, at: u64, bytes: &[u8]) -> Result<Insn, crate::Error>;
}
