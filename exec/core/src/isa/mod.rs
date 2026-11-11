//! Facts about each CPU type: registers as a program sees them, where a trap's arguments are, and how flags convert.
//! Written once, so no backend decodes a system call itself.

pub mod arm64;
pub mod convert;
pub mod x86;

use crate::Isa;

/// Guest pages are 4 KiB on both CPU types.
pub const PAGE: u64 = exec_mmu::PAGE;

/// The top of the user half of a 48-bit address space.
pub const USER_TOP: u64 = 0x8000_0000_0000;

/// Whether an address is canonical: its top 17 bits are all the same.
pub const fn is_canonical(addr: u64) -> bool {
    let top = addr >> 47;
    top == 0 || top == 0x1_ffff
}

/// The top of the space a program of this instruction set can use.
pub const fn space_top(isa: Isa) -> u64 {
    match isa {
        Isa::X86_32 => 1 << 32,
        Isa::X86_64 | Isa::Arm64 => USER_TOP,
    }
}

/// What kind of trap brought a program to its kernel, which says where the call's number and arguments are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    /// x86-64's `syscall`.
    Syscall,
    /// x86's `int 0x2e`, Windows' old way in.
    Int2e,
    /// x86's `int 0x80`, 32-bit Linux's way in.
    Int80,
    /// ARM64's `svc`.
    Svc,
}
