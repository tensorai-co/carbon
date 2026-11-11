//! x86-64 code, for ARM64 programs on x86-64 hosts.
//!
//! Not built yet. ARM64's 31 registers don't fit in x86-64's 16, so the hottest live in registers and the rest in a
//! context block in memory. Memory ordering needs nothing: x86's is stronger than ARM's.

use crate::{Direction, Error, Isa};

/// Always `Unsupported` for now.
pub fn translate(_start: u64, _bytes: &[u8]) -> Result<(), Error> {
    Err(Error::Unsupported(Direction { from: Isa::Arm64, to: Isa::X86_64 }))
}
