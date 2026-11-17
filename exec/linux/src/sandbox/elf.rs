//! The stub: a small ELF that Carbon writes and runs, which sets itself up and waits in its handler.
//!
//! ```text
//! base             handler page   R X   the ELF's headers, then the handler and where the stub starts (handler.rs):
//!                                       exempt from dispatch
//! base + 0x1000    trampoline     R X   `syscall; int3`, for Carbon's own calls while it has the stub stopped
//!                                       (`strict`)
//! base + 0x2000    data           R     Syscall User Dispatch's selector byte, at "block"
//! base + 0x3000    shared         R W   the mailbox's page, then the signal stack, from a memfd Carbon maps too
//! ```
//!
//! It goes as high as it can, so programs have the address space below it: at the very top of user space if the
//! kernel's put nothing there (the new stub's stack is randomized below the top, and is usually far enough down), else
//! just under the lowest place the kernel can put that stack, else lower, for a kernel that keeps more of the top to
//! itself (gVisor loads a fixed ELF only below about `0x7f00_0000_0000`). Every CPU's stub in a machine is at the same
//! place, since they all show the same spaces.

use super::handler;

/// Where x86-64's user space ends.
pub const TOP: u64 = 0x7fff_ffff_f000;
/// The stub, its shared memory included.
pub const SIZE: u64 = handler::SHARED + handler::SHARED_SIZE;
/// Under the lowest a new program's stack can start (16 GiB of randomization below the top, then its first pages), so
/// a stub here never meets it.
pub const UNDER_STACK: u64 = 0x7ffb_0000_0000;
/// Where to try, best first: the top, under the stack, where the stub always went before, and lower for gVisor.
pub const PLACES: [u64; 5] = [TOP - SIZE, UNDER_STACK, 0x7f80_0000_0000, 0x7e00_0000_0000, 0x7000_0000_0000];

/// The ELF's two headers.
const HEADERS: u64 = 64 + 56;
/// `syscall; int3` (the trampoline), `int3` (where it rests), `jmp $`.
const TRAMPOLINE_CODE: [u8; 6] = [0x0f, 0x05, 0xcc, 0xcc, 0xeb, 0xfe];

/// The stub's addresses, from where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub base: u64,
}

impl At {
    pub fn handler(self) -> u64 {
        self.base + handler::handler_at()
    }
    pub fn restorer(self) -> u64 {
        self.base + handler::restorer_at()
    }
    /// The trampoline: `syscall` here, then `int3`.
    pub fn trampoline(self) -> u64 {
        self.base + handler::TRAMPOLINE_PAGE
    }
    pub fn data(self) -> u64 {
        self.base + handler::DATA_PAGE
    }
    /// Syscall User Dispatch's selector byte, the data page's first.
    pub fn selector(self) -> u64 {
        self.data()
    }
    pub fn shared(self) -> u64 {
        self.base + handler::SHARED
    }
    pub fn signal_stack(self) -> u64 {
        self.base + handler::SIGNAL_STACK
    }

    pub fn build(self) -> Vec<u8> {
        let entry = self.base + handler::setup_at();
        let filesz = handler::DATA_PAGE + 1;
        let mut elf = Vec::with_capacity(filesz as usize);
        // Elf64_Ehdr
        elf.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        elf.extend_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        elf.extend_from_slice(&0x3eu16.to_le_bytes()); // EM_X86_64
        elf.extend_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
        elf.extend_from_slice(&entry.to_le_bytes());
        elf.extend_from_slice(&64u64.to_le_bytes()); // e_phoff
        elf.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
        elf.extend_from_slice(&0u32.to_le_bytes()); // e_flags
        elf.extend_from_slice(&64u16.to_le_bytes()); // e_ehsize
        elf.extend_from_slice(&56u16.to_le_bytes()); // e_phentsize
        elf.extend_from_slice(&1u16.to_le_bytes()); // e_phnum
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_shentsize
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_shnum
        elf.extend_from_slice(&0u16.to_le_bytes()); // e_shstrndx
        // Elf64_Phdr: one PT_LOAD, the file at the base and the whole stub in memory, so the kernel won't start it
        // where something already is.
        elf.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf.extend_from_slice(&7u32.to_le_bytes()); // R|W|X, narrowed once it's set up
        elf.extend_from_slice(&0u64.to_le_bytes()); // p_offset
        elf.extend_from_slice(&self.base.to_le_bytes()); // p_vaddr
        elf.extend_from_slice(&self.base.to_le_bytes()); // p_paddr
        elf.extend_from_slice(&filesz.to_le_bytes());
        elf.extend_from_slice(&SIZE.to_le_bytes());
        elf.extend_from_slice(&0x1000u64.to_le_bytes()); // p_align
        debug_assert_eq!(elf.len() as u64, HEADERS);
        elf.resize(handler::HANDLER as usize, 0);
        elf.extend_from_slice(handler::code());
        elf.resize(handler::TRAMPOLINE_PAGE as usize, 0);
        elf.extend_from_slice(&TRAMPOLINE_CODE);
        elf.resize(handler::DATA_PAGE as usize, 0);
        elf.push(super::sys::SYSCALL_DISPATCH_FILTER_BLOCK);
        elf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_fits_where_it_goes() {
        for base in PLACES {
            let at = At { base };
            assert!(at.base + SIZE <= TOP);
            assert!(handler::kick_at() < 0x1000 && handler::restorer_at() < 0x1000 && handler::setup_at() < 0x1000);
            let elf = at.build();
            assert_eq!(elf.len() as u64, handler::DATA_PAGE + 1);
            assert_eq!(&elf[handler::HANDLER as usize..handler::HANDLER as usize + 3], &handler::code()[..3]);
        }
        const { assert!(HEADERS <= handler::HANDLER) };
    }
}
