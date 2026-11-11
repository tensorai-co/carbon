//! The stub: a two-page ELF that Carbon writes, runs from a memfd, and then drives entirely through `ptrace`.
//!
//! Its first page is code: a `syscall; int3` trampoline that Carbon borrows to make system calls on the stub's behalf,
//! and an `int3` to rest on. Its second page is data: Syscall User Dispatch's selector byte, and the seccomp filter.

/// Where the stub lives, high up and out of a program's way. Programs live below it.
pub const STUB: u64 = 0x7f80_0000_0000;
/// The trampoline, right after the ELF headers: `syscall` here, then `int3`.
pub const TRAMPOLINE: u64 = STUB + CODE_OFFSET as u64;
/// The data page.
pub const DATA: u64 = STUB + 0x1000;
/// In the data page: the selector byte, the seccomp program's header, and its instructions.
pub const SELECTOR: u64 = DATA;
pub const FILTER_PROGRAM: u64 = DATA + 0x10;
pub const FILTER: u64 = DATA + 0x20;

const CODE_OFFSET: usize = 64 + 56;

pub fn build() -> Vec<u8> {
    // syscall; int3 (the trampoline), int3 (where it rests), jmp $
    let code: [u8; 6] = [0x0f, 0x05, 0xcc, 0xcc, 0xeb, 0xfe];
    let entry = TRAMPOLINE + 3;
    let filesz = (CODE_OFFSET + code.len()) as u64;
    let memsz: u64 = 0x2000;

    let mut elf = Vec::with_capacity(CODE_OFFSET + code.len());
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
    // Elf64_Phdr: one PT_LOAD, the whole file at STUB, two pages in memory.
    elf.extend_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    elf.extend_from_slice(&7u32.to_le_bytes()); // R|W|X, narrowed once it's set up
    elf.extend_from_slice(&0u64.to_le_bytes()); // p_offset
    elf.extend_from_slice(&STUB.to_le_bytes()); // p_vaddr
    elf.extend_from_slice(&STUB.to_le_bytes()); // p_paddr
    elf.extend_from_slice(&filesz.to_le_bytes());
    elf.extend_from_slice(&memsz.to_le_bytes());
    elf.extend_from_slice(&0x1000u64.to_le_bytes()); // p_align
    debug_assert_eq!(elf.len(), CODE_OFFSET);
    elf.extend_from_slice(&code);
    elf
}
