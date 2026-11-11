//! x86-64's page table entries.

use crate::{Format, Prot};

pub struct X86;

pub const P: u64 = 1;
pub const RW: u64 = 1 << 1;
pub const US: u64 = 1 << 2;
pub const ACCESSED: u64 = 1 << 5;
pub const DIRTY: u64 = 1 << 6;
pub const NX: u64 = 1 << 63;

impl Format for X86 {
    const ADDRESS: u64 = 0x000f_ffff_ffff_f000;
    const SHARED: u64 = 1 << 9;
    const KERNEL_FRAME: u64 = 1 << 10;
    const USER_ENTRIES: u64 = 256;

    fn is_valid(entry: u64) -> bool {
        entry & P != 0
    }

    fn table(frame: u64) -> u64 {
        frame | P | RW | US | ACCESSED
    }

    fn page(frame: u64, prot: Prot, shared: bool) -> u64 {
        let mut entry = frame | P | ACCESSED | DIRTY;
        // With no access at all a page stays mapped for ring 0, so its contents survive, but the program faults on it.
        if prot != Prot::NONE {
            entry |= US;
        }
        if prot.write() && !shared {
            entry |= RW;
        }
        if shared {
            entry |= Self::SHARED;
        }
        if !prot.exec() {
            entry |= NX;
        }
        entry
    }

    fn prot(entry: u64) -> Prot {
        if !Self::is_valid(entry) || entry & US == 0 {
            return Prot::NONE;
        }
        let mut prot = Prot::R;
        if entry & RW != 0 {
            prot = prot | Prot::W;
        }
        if entry & NX == 0 {
            prot = prot | Prot::X;
        }
        prot
    }
}
