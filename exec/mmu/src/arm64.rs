//! ARM64's stage-1 descriptors, with a 4 KiB granule. Programs live in TTBR0's half, all of it, so every root entry
//! is a program's: the shim has a root of its own in TTBR1.

use crate::{Format, Prot};

pub struct Arm64;

pub const VALID: u64 = 0b11;
pub const AP_EL0: u64 = 1 << 6;
pub const AP_READ_ONLY: u64 = 1 << 7;
pub const SHAREABLE_INNER: u64 = 3 << 8;
pub const ACCESSED: u64 = 1 << 10;
pub const NOT_GLOBAL: u64 = 1 << 11;
pub const PXN: u64 = 1 << 53;
pub const UXN: u64 = 1 << 54;

impl Format for Arm64 {
    const ADDRESS: u64 = 0x0000_ffff_ffff_f000;
    const SHARED: u64 = 1 << 55;
    const KERNEL_FRAME: u64 = 1 << 56;
    const USER_ENTRIES: u64 = 512;

    fn is_valid(entry: u64) -> bool {
        entry & VALID == VALID
    }

    fn table(frame: u64) -> u64 {
        frame | VALID
    }

    fn page(frame: u64, prot: Prot, shared: bool) -> u64 {
        let mut entry = frame | VALID | SHAREABLE_INNER | ACCESSED | NOT_GLOBAL | PXN;
        if shared {
            entry |= Self::SHARED;
        }
        if !prot.exec() {
            entry |= UXN;
        }
        // With no access at all a page stays mapped for EL1, so its contents survive, but the program faults on it.
        if prot != Prot::NONE {
            entry |= AP_EL0;
        }
        if !prot.write() || shared {
            entry |= AP_READ_ONLY;
        }
        entry
    }

    fn prot(entry: u64) -> Prot {
        if !Self::is_valid(entry) || entry & AP_EL0 == 0 {
            return Prot::NONE;
        }
        let mut prot = Prot::R;
        if entry & AP_READ_ONLY == 0 {
            prot = prot | Prot::W;
        }
        if entry & UXN == 0 {
            prot = prot | Prot::X;
        }
        prot
    }
}
