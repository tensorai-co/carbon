//! The stub: a two-page ELF that Carbon writes, runs, and then drives entirely through `ptrace`.
//!
//! Its first page is code: a `syscall; int3` trampoline that Carbon borrows to make system calls on the stub's behalf,
//! and an `int3` to rest on. Its second page is data: Syscall User Dispatch's selector byte, an empty signal set, and
//! the seccomp filter.
//!
//! It goes as high as it can, so programs have the address space below it: at the very top of user space if the
//! kernel's put nothing there (the new stub's stack is randomized below the top, and is usually far enough down), else
//! just under the lowest place the kernel can put that stack, else lower, for a kernel that keeps more of the top to
//! itself (gVisor loads a fixed ELF only below about `0x7f00_0000_0000`). Every CPU's stub in a machine is at the same
//! place, since they all show the same spaces.

/// Where x86-64's user space ends.
pub const TOP: u64 = 0x7fff_ffff_f000;
/// The stub's two pages.
pub const SIZE: u64 = 0x2000;
/// Under the lowest a new program's stack can start (16 GiB of randomization below the top, then its first pages), so
/// a stub here never meets it.
pub const UNDER_STACK: u64 = 0x7ffb_0000_0000;
/// Where to try, best first: the top, under the stack, where the stub always went before, and lower for gVisor.
pub const PLACES: [u64; 5] = [TOP - SIZE, UNDER_STACK, 0x7f80_0000_0000, 0x7e00_0000_0000, 0x7000_0000_0000];

const CODE_OFFSET: u64 = 64 + 56;

/// The stub's addresses, from where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub base: u64,
}

impl At {
    /// The trampoline, right after the ELF headers: `syscall` here, then `int3`.
    pub fn trampoline(self) -> u64 {
        self.base + CODE_OFFSET
    }
    /// Where the trampoline's `syscall` returns to, as seccomp sees it.
    pub fn after_syscall(self) -> u64 {
        self.trampoline() + 2
    }
    pub fn data(self) -> u64 {
        self.base + 0x1000
    }
    /// In the data page: the selector byte, the seccomp program's header, an empty signal set and the filter.
    pub fn selector(self) -> u64 {
        self.data()
    }
    pub fn filter_program(self) -> u64 {
        self.data() + 0x10
    }
    pub fn empty_sigset(self) -> u64 {
        self.data() + 0x20
    }
    pub fn filter(self) -> u64 {
        self.data() + 0x40
    }

    pub fn build(self) -> Vec<u8> {
        // syscall; int3 (the trampoline), int3 (where it rests), jmp $
        let code: [u8; 6] = [0x0f, 0x05, 0xcc, 0xcc, 0xeb, 0xfe];
        let entry = self.trampoline() + 3;
        let filesz = CODE_OFFSET + code.len() as u64;

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
        // Elf64_Phdr: one PT_LOAD, the whole file at the base, two pages in memory.
        elf.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf.extend_from_slice(&7u32.to_le_bytes()); // R|W|X, narrowed once it's set up
        elf.extend_from_slice(&0u64.to_le_bytes()); // p_offset
        elf.extend_from_slice(&self.base.to_le_bytes()); // p_vaddr
        elf.extend_from_slice(&self.base.to_le_bytes()); // p_paddr
        elf.extend_from_slice(&filesz.to_le_bytes());
        elf.extend_from_slice(&SIZE.to_le_bytes());
        elf.extend_from_slice(&0x1000u64.to_le_bytes()); // p_align
        debug_assert_eq!(elf.len() as u64, CODE_OFFSET);
        elf.extend_from_slice(&code);
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
            assert_eq!(at.build().len() as u64, CODE_OFFSET + 6);
            assert!(at.filter() + u64::from(crate::sandbox::seccomp::len()) * 8 <= at.base + SIZE);
        }
    }
}
