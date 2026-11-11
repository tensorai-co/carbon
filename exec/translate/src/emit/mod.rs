//! Emitters: decoded instructions in, host code out. `arm64` translates x86-64 to ARM64; `x86_64` will translate
//! ARM64 to x86-64; `asm` is the ARM64 assembler they share.

pub mod arm64;
pub mod asm;
pub mod x86_64;
