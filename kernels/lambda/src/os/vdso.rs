//! The vDSO, made in memory for each ISA, and the clock page (`[vvar]`) it reads: `clock_gettime` for the wall and
//! monotonic clocks without a system call, from the CPU's counter (ARM64's `CNTVCT_EL0`; x86-64's `rdtsc`, which the
//! translator reads the same counter for) and what the kernel last wrote on the clock page: a counter reading paired
//! with the host's monotonic time, nanoseconds per tick, and the wall clock's offset. Anything else falls through to
//! the system call, as does every clock while the page isn't ready (a host whose counter the guest's may not be).
//!
//! The image is an ELF shared object glibc, musl and Go all take: one loadable segment, versioned dynamic symbols
//! (`LINUX_2.6.39`'s `__kernel_*` on ARM64, `LINUX_2.6`'s `__vdso_*` on x86-64) found through a SysV hash table, and
//! no relocations. It also holds the code signal handlers return through when a program gives no restorer.

use std::sync::atomic::{Ordering, fence};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use exec::{Isa, PAGE};

use crate::Kernel;
use crate::mm::shared::SharedMem;

/// The clock page's fields, at their offsets: a sequence number (odd while the kernel writes), whether it's ready,
/// the counter reading and the monotonic nanoseconds paired with it, the wall clock's offset from the monotonic one,
/// nanoseconds per tick (32.32 fixed point), and how many ticks past the reading it's still good for.
pub(crate) mod vvar {
    pub const SEQ: u64 = 0;
    pub const READY: u64 = 8;
    pub const COUNTER: u64 = 16;
    pub const MONO: u64 = 24;
    pub const REAL: u64 = 32;
    pub const MULT: u64 = 40;
    pub const LIMIT: u64 = 48;
}

/// A vDSO image for an ISA, and where its signal return is in it.
pub(crate) struct Image {
    pub bytes: Vec<u8>,
    pub sigreturn: u64,
}

/// The ISA's image, made once.
pub(crate) fn image(isa: Isa) -> &'static Image {
    static ARM64: OnceLock<Image> = OnceLock::new();
    static X86_64: OnceLock<Image> = OnceLock::new();
    match isa {
        Isa::Arm64 => ARM64.get_or_init(|| build(Isa::Arm64)),
        _ => X86_64.get_or_init(|| build(Isa::X86_64)),
    }
}

/// The clocks the vDSO answers itself: REALTIME, MONOTONIC, MONOTONIC_RAW, REALTIME_COARSE, MONOTONIC_COARSE and
/// BOOTTIME (bits 0, 1, 4, 5, 6, 7); REALTIME and REALTIME_COARSE (0 and 5) with the wall clock's offset.
const FAST_CLOCKS: u32 = 0xf3;

// ---- ARM64 ----

/// An ARM64 function's words, with labels and branches resolved once it's done.
#[derive(Default)]
struct A64 {
    words: Vec<u32>,
    labels: Vec<Option<usize>>,
    /// Each branch: its word, its label, and its kind (the immediate's width and place).
    fixups: Vec<(usize, usize, Branch)>,
}

#[derive(Clone, Copy)]
enum Branch {
    /// `b.cond`, `cbz`, `cbnz`: 19 bits at bit 5.
    Imm19,
    /// `tbnz`, `tbz`: 14 bits at bit 5.
    Imm14,
    /// `b`: 26 bits at bit 0.
    Imm26,
}

const EQ: u32 = 0;
const NE: u32 = 1;
const HI: u32 = 8;

impl A64 {
    fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }

    fn bind(&mut self, l: usize) {
        self.labels[l] = Some(self.words.len());
    }

    fn w(&mut self, word: u32) {
        self.words.push(word);
    }

    fn branch(&mut self, word: u32, l: usize, kind: Branch) {
        self.fixups.push((self.words.len(), l, kind));
        self.words.push(word);
    }

    fn b_cond(&mut self, cond: u32, l: usize) {
        self.branch(0x5400_0000 | cond, l, Branch::Imm19);
    }

    fn finish(mut self) -> Vec<u32> {
        for (at, l, kind) in std::mem::take(&mut self.fixups) {
            let to = self.labels[l].expect("a bound label") as i64;
            let off = (to - at as i64) as u32;
            self.words[at] |= match kind {
                Branch::Imm19 => (off & 0x7_ffff) << 5,
                Branch::Imm14 => (off & 0x3fff) << 5,
                Branch::Imm26 => off & 0x3ff_ffff,
            };
        }
        self.words
    }
}

fn movz(rd: u32, imm: u32, shift: u32) -> u32 {
    0xd280_0000 | (shift / 16) << 21 | (imm & 0xffff) << 5 | rd
}

fn movk(rd: u32, imm: u32, shift: u32) -> u32 {
    0xf280_0000 | (shift / 16) << 21 | (imm & 0xffff) << 5 | rd
}

fn ldr_x(rt: u32, rn: u32, off: u64) -> u32 {
    0xf940_0000 | ((off / 8) as u32) << 10 | rn << 5 | rt
}

fn ldr_w(rt: u32, rn: u32, off: u64) -> u32 {
    0xb940_0000 | ((off / 4) as u32) << 10 | rn << 5 | rt
}

const RET: u32 = 0xd65f_03c0;
const SVC: u32 = 0xd400_0001;

/// ARM64's code: `clock_gettime` and `gettimeofday` (one path, `x16` the divisor from nanoseconds: 1, or 1,000 for
/// a `timeval`'s microseconds), then the system-call-only functions, then the signal return. `vvar` is where the clock
/// page is, in bytes from the code's start. Each function's offset from the code's start.
fn arm64_code(vvar: i64) -> (Vec<u8>, [u64; 4]) {
    let mut a = A64::default();
    let (common, retry, slow, real, sum, timeval_slow, timeval_raw) = (a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label());
    // clock_gettime(x0 clock, x1 ts)
    a.w(movz(16, 1, 0)); // mov x16, #1
    a.w(0xf100_1c1f); // cmp x0, #7
    a.b_cond(HI, slow);
    a.branch(0xb400_0001, slow, Branch::Imm19); // cbz x1, slow
    a.w(movz(9, 1, 0)); // mov x9, #1
    a.w(0x9ac0_2129); // lsl x9, x9, x0
    a.w(movz(10, FAST_CLOCKS, 0)); // mov x10, #FAST_CLOCKS
    a.w(0xea0a_013f); // tst x9, x10
    a.b_cond(EQ, slow);
    // The clock in x0, where it goes in x1, the divisor in x16.
    a.bind(common);
    let here = (a.words.len() * 4) as i64;
    let off = (vvar - here) as u32;
    a.w(0x1000_000a | (off & 3) << 29 | ((off >> 2) & 0x7_ffff) << 5); // adr x10, vvar
    a.bind(retry);
    a.w(0x88df_fd4b); // ldar w11, [x10]
    a.branch(0x3700_000b, retry, Branch::Imm14); // tbnz w11, #0, retry
    a.w(ldr_w(12, 10, vvar::READY)); // ldr w12, [x10, #READY]
    a.branch(0x3400_000c, slow, Branch::Imm19); // cbz w12, slow
    a.w(0xd503_3fdf); // isb
    a.w(0xd53b_e04d); // mrs x13, cntvct_el0
    a.w(ldr_x(14, 10, vvar::COUNTER));
    a.w(0xcb0e_01ad); // sub x13, x13, x14
    a.w(ldr_x(14, 10, vvar::LIMIT));
    a.w(0xeb0e_01bf); // cmp x13, x14
    a.b_cond(HI, slow);
    a.w(ldr_x(14, 10, vvar::MULT));
    a.w(0x9b0e_7daf); // mul x15, x13, x14
    a.w(0x9bce_7dad); // umulh x13, x13, x14
    a.w(0xd360_fdef); // lsr x15, x15, #32
    a.w(0xaa0d_81ed); // orr x13, x15, x13, lsl #32
    a.w(ldr_x(14, 10, vvar::MONO));
    a.w(0x8b0e_01ad); // add x13, x13, x14
    a.branch(0xb400_0000, real, Branch::Imm19); // cbz x0, real
    a.w(0xf100_141f); // cmp x0, #5
    a.b_cond(NE, sum);
    a.bind(real);
    a.w(ldr_x(14, 10, vvar::REAL));
    a.w(0x8b0e_01ad); // add x13, x13, x14
    a.bind(sum);
    a.w(0xd503_39bf); // dmb ishld
    a.w(ldr_w(12, 10, vvar::SEQ)); // ldr w12, [x10]
    a.w(0x6b0b_019f); // cmp w12, w11
    a.b_cond(NE, retry);
    a.w(movz(14, 0xca00, 0)); // 1,000,000,000
    a.w(movk(14, 0x3b9a, 16));
    a.w(0x9ace_09af); // udiv x15, x13, x14
    a.w(0x9b0e_b5ec); // msub x12, x15, x14, x13
    a.w(0x9ad0_098c); // udiv x12, x12, x16
    a.w(0xa900_302f); // stp x15, x12, [x1]
    a.w(movz(0, 0, 0)); // mov x0, #0
    a.w(RET);
    // The system call, with the counter's reading beside it for the kernel to learn the counter by; or for a
    // `timeval`, `gettimeofday`'s, its arguments as they came.
    a.bind(slow);
    a.w(0xf10f_a21f); // cmp x16, #1000
    a.b_cond(EQ, timeval_slow);
    a.w(0xd503_3fdf); // isb
    a.w(0xd53b_e042); // mrs x2, cntvct_el0
    a.w(movz(3, MARKER as u32 & 0xffff, 0));
    a.w(movk(3, (MARKER >> 16) as u32 & 0xffff, 16));
    a.w(movz(8, 113, 0)); // clock_gettime
    a.w(SVC);
    a.w(RET);
    a.bind(timeval_slow);
    a.w(0xaa01_03e0); // mov x0, x1
    a.w(movz(1, 0, 0)); // mov x1, #0
    a.bind(timeval_raw);
    a.w(movz(8, 169, 0)); // gettimeofday
    a.w(SVC);
    a.w(RET);
    let mut offs = [0u64; 4];
    // gettimeofday(x0 tv, x1 tz): the wall clock's path for a `timeval`, unless it asks for a time zone.
    offs[1] = (a.words.len() * 4) as u64;
    a.branch(0xb500_0001, timeval_raw, Branch::Imm19); // cbnz x1, raw
    a.branch(0xb400_0000, timeval_raw, Branch::Imm19); // cbz x0, raw
    a.w(0xaa00_03e1); // mov x1, x0
    a.w(movz(0, 0, 0)); // mov x0, #0 (CLOCK_REALTIME)
    a.w(movz(16, 1000, 0)); // mov x16, #1000
    a.branch(0x1400_0000, common, Branch::Imm26); // b common
    // clock_getres, rt_sigreturn: their system calls.
    for (i, nr) in [114u32, 139].iter().enumerate() {
        offs[i + 2] = (a.words.len() * 4) as u64;
        a.w(movz(8, *nr, 0));
        a.w(SVC);
        a.w(RET);
    }
    let words = a.finish();
    (words.iter().flat_map(|w| w.to_le_bytes()).collect(), offs)
}

// ---- x86-64 ----

/// x86-64's code, with 32-bit relative jumps to labels.
#[derive(Default)]
struct X64 {
    bytes: Vec<u8>,
    labels: Vec<Option<usize>>,
    fixups: Vec<(usize, usize)>,
}

impl X64 {
    fn label(&mut self) -> usize {
        self.labels.push(None);
        self.labels.len() - 1
    }

    fn bind(&mut self, l: usize) {
        self.labels[l] = Some(self.bytes.len());
    }

    fn b(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// A jump (`jmp`, or `jcc` with `cc`) to a label.
    fn jump(&mut self, cc: Option<u8>, l: usize) {
        match cc {
            Some(cc) => self.b(&[0x0f, 0x80 | cc]),
            None => self.b(&[0xe9]),
        }
        self.fixups.push((self.bytes.len(), l));
        self.b(&[0; 4]);
    }

    fn finish(mut self) -> Vec<u8> {
        for (at, l) in std::mem::take(&mut self.fixups) {
            let to = self.labels[l].expect("a bound label") as i64;
            let rel = (to - (at as i64 + 4)) as i32;
            self.bytes[at..at + 4].copy_from_slice(&rel.to_le_bytes());
        }
        self.bytes
    }
}

const JE: u8 = 0x4;
const JNE: u8 = 0x5;
const JA: u8 = 0x7;

/// x86-64's code: `clock_gettime` and `gettimeofday` (one path, `r11` the divisor from nanoseconds: 1, or 1,000 for
/// a `timeval`'s microseconds), the system-call-only functions, and the signal return. Each function's offset.
fn x86_64_code(vvar: i64) -> (Vec<u8>, [u64; 6]) {
    let mut a = X64::default();
    let (common, retry, slow, real, sum, timeval_slow, timeval_raw) = (a.label(), a.label(), a.label(), a.label(), a.label(), a.label(), a.label());
    // clock_gettime(edi clock, rsi ts)
    a.b(&[0x41, 0xbb, 1, 0, 0, 0]); // mov r11d, 1
    a.b(&[0x83, 0xff, 0x07]); // cmp edi, 7
    a.jump(Some(JA), slow);
    a.b(&[0x89, 0xf9]); // mov ecx, edi
    a.b(&[0xb8, 1, 0, 0, 0]); // mov eax, 1
    a.b(&[0xd3, 0xe0]); // shl eax, cl
    a.b(&[0xa9]); // test eax, FAST_CLOCKS
    a.b(&FAST_CLOCKS.to_le_bytes());
    a.jump(Some(JE), slow);
    a.b(&[0x48, 0x85, 0xf6]); // test rsi, rsi
    a.jump(Some(JE), slow);
    // The clock in edi, where it goes in rsi, the divisor in r11.
    a.bind(common);
    a.b(&[0x4c, 0x8d, 0x05]); // lea r8, [rip + vvar]
    let end = a.bytes.len() as i64 + 4;
    a.b(&((vvar - end) as i32).to_le_bytes());
    a.bind(retry);
    a.b(&[0x45, 0x8b, 0x08]); // mov r9d, [r8]
    a.b(&[0x41, 0xf7, 0xc1, 1, 0, 0, 0]); // test r9d, 1
    a.jump(Some(JNE), retry);
    a.b(&[0x41, 0x83, 0x78, vvar::READY as u8, 0]); // cmp dword [r8 + READY], 0
    a.jump(Some(JE), slow);
    a.b(&[0x0f, 0x31]); // rdtsc
    a.b(&[0x48, 0xc1, 0xe2, 0x20]); // shl rdx, 32
    a.b(&[0x48, 0x09, 0xd0]); // or rax, rdx
    a.b(&[0x49, 0x2b, 0x40, vvar::COUNTER as u8]); // sub rax, [r8 + COUNTER]
    a.b(&[0x49, 0x3b, 0x40, vvar::LIMIT as u8]); // cmp rax, [r8 + LIMIT]
    a.jump(Some(JA), slow);
    a.b(&[0x49, 0x8b, 0x48, vvar::MULT as u8]); // mov rcx, [r8 + MULT]
    a.b(&[0x48, 0xf7, 0xe1]); // mul rcx
    a.b(&[0x48, 0xc1, 0xe8, 0x20]); // shr rax, 32
    a.b(&[0x48, 0xc1, 0xe2, 0x20]); // shl rdx, 32
    a.b(&[0x48, 0x09, 0xd0]); // or rax, rdx
    a.b(&[0x49, 0x03, 0x40, vvar::MONO as u8]); // add rax, [r8 + MONO]
    a.b(&[0x85, 0xff]); // test edi, edi
    a.jump(Some(JE), real);
    a.b(&[0x83, 0xff, 0x05]); // cmp edi, 5
    a.jump(Some(JNE), sum);
    a.bind(real);
    a.b(&[0x49, 0x03, 0x40, vvar::REAL as u8]); // add rax, [r8 + REAL]
    a.bind(sum);
    a.b(&[0x45, 0x3b, 0x08]); // cmp r9d, [r8]
    a.jump(Some(JNE), retry);
    a.b(&[0x31, 0xd2]); // xor edx, edx
    a.b(&[0xb9]); // mov ecx, 1,000,000,000
    a.b(&1_000_000_000u32.to_le_bytes());
    a.b(&[0x48, 0xf7, 0xf1]); // div rcx: seconds in rax, nanoseconds in rdx
    a.b(&[0x48, 0x89, 0xc1]); // mov rcx, rax
    a.b(&[0x48, 0x89, 0xd0]); // mov rax, rdx
    a.b(&[0x31, 0xd2]); // xor edx, edx
    a.b(&[0x49, 0xf7, 0xf3]); // div r11
    a.b(&[0x48, 0x89, 0x0e]); // mov [rsi], rcx
    a.b(&[0x48, 0x89, 0x46, 0x08]); // mov [rsi + 8], rax
    a.b(&[0x31, 0xc0]); // xor eax, eax
    a.b(&[0xc3]); // ret
    // The system call, with the counter's reading beside it (rdx) and the marker (r10); or for a `timeval`,
    // `gettimeofday`'s, its arguments as they came.
    a.bind(slow);
    a.b(&[0x41, 0x81, 0xfb, 0xe8, 0x03, 0, 0]); // cmp r11d, 1000
    a.jump(Some(JE), timeval_slow);
    a.b(&[0x0f, 0x31]); // rdtsc
    a.b(&[0x48, 0xc1, 0xe2, 0x20]); // shl rdx, 32
    a.b(&[0x48, 0x09, 0xc2]); // or rdx, rax
    a.b(&[0x41, 0xba]); // mov r10d, MARKER
    a.b(&(MARKER as u32).to_le_bytes());
    a.b(&[0xb8, 228, 0, 0, 0, 0x0f, 0x05, 0xc3]); // mov eax, clock_gettime; syscall; ret
    a.bind(timeval_slow);
    a.b(&[0x48, 0x89, 0xf7]); // mov rdi, rsi
    a.b(&[0x31, 0xf6]); // xor esi, esi
    a.bind(timeval_raw);
    a.b(&[0xb8, 96, 0, 0, 0, 0x0f, 0x05, 0xc3]); // mov eax, gettimeofday; syscall; ret
    let mut offs = [0u64; 6];
    // gettimeofday(rdi tv, rsi tz): the wall clock's path for a `timeval`, unless it asks for a time zone.
    offs[1] = a.bytes.len() as u64;
    a.b(&[0x48, 0x85, 0xf6]); // test rsi, rsi
    a.jump(Some(JNE), timeval_raw);
    a.b(&[0x48, 0x85, 0xff]); // test rdi, rdi
    a.jump(Some(JE), timeval_raw);
    a.b(&[0x48, 0x89, 0xfe]); // mov rsi, rdi
    a.b(&[0x31, 0xff]); // xor edi, edi
    a.b(&[0x41, 0xbb, 0xe8, 0x03, 0, 0]); // mov r11d, 1000
    a.jump(None, common);
    // time, getcpu, clock_getres, rt_sigreturn.
    for (i, nr) in [201u32, 309, 229, 15].iter().enumerate() {
        offs[i + 2] = a.bytes.len() as u64;
        a.b(&[0xb8]);
        a.b(&nr.to_le_bytes());
        a.b(&[0x0f, 0x05, 0xc3]);
    }
    (a.finish(), offs)
}

// ---- The ELF image ----

/// SysV's ELF hash, as `DT_HASH` and version definitions use.
fn elf_hash(name: &str) -> u32 {
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = (h << 4).wrapping_add(u32::from(b));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// Where the code starts in the image: past the headers and tables, at 16 bytes.
const CODE: usize = 0x400;

fn build(isa: Isa) -> Image {
    // The clock page is the page before the image; the code's start is `CODE` into it.
    let vvar = -(PAGE as i64) - CODE as i64;
    let (machine, version, names, code, offs): (u16, &str, Vec<&str>, Vec<u8>, Vec<u64>) = match isa {
        Isa::Arm64 => {
            let (code, offs) = arm64_code(vvar);
            (183, "LINUX_2.6.39", vec!["__kernel_clock_gettime", "__kernel_gettimeofday", "__kernel_clock_getres", "__kernel_rt_sigreturn"], code, offs.to_vec())
        }
        _ => {
            let (code, offs) = x86_64_code(vvar);
            (62, "LINUX_2.6", vec!["__vdso_clock_gettime", "__vdso_gettimeofday", "__vdso_time", "__vdso_getcpu", "__vdso_clock_getres"], code, offs[..5].to_vec())
        }
    };
    let sigreturn = CODE as u64 + match isa {
        Isa::Arm64 => offs[3],
        _ => x86_64_code(vvar).1[5],
    };
    // The string table: the image's name, its version, each symbol.
    let mut strtab = vec![0u8];
    let mut add = |s: &str| {
        let at = strtab.len() as u32;
        strtab.extend_from_slice(s.as_bytes());
        strtab.push(0);
        at
    };
    let soname = add("linux-vdso.so.1");
    let ver_name = add(version);
    let sym_names: Vec<u32> = names.iter().map(|n| add(n)).collect();
    let nsyms = names.len() + 1;
    // Tables, one after another from the headers' end: hash, symbols, strings, versions of each symbol, version
    // definitions, the dynamic section.
    let hash_at = 64 + 2 * 56;
    let hash_len = 4 * (2 + 1 + nsyms);
    let sym_at = (hash_at + hash_len).next_multiple_of(8);
    let str_at = sym_at + 24 * nsyms;
    let versym_at = (str_at + strtab.len()).next_multiple_of(2);
    let verdef_at = (versym_at + 2 * nsyms).next_multiple_of(4);
    let dyn_at = (verdef_at + 2 * 28).next_multiple_of(8);
    let dyn_entries: Vec<(u64, u64)> = vec![
        (4, hash_at as u64),
        (5, str_at as u64),
        (6, sym_at as u64),
        (10, strtab.len() as u64),
        (11, 24),
        (14, u64::from(soname)),
        (0x6fff_fff0, versym_at as u64),
        (0x6fff_fffc, verdef_at as u64),
        (0x6fff_fffd, 2),
        (0, 0),
    ];
    let dyn_len = 16 * dyn_entries.len();
    assert!(dyn_at + dyn_len <= CODE, "the vDSO's tables fit before its code");
    let size = CODE + code.len();
    assert!(size <= PAGE as usize, "the vDSO fits in a page");
    let mut img = vec![0u8; size];
    let put16 = |img: &mut Vec<u8>, at: usize, v: u16| img[at..at + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |img: &mut Vec<u8>, at: usize, v: u32| img[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |img: &mut Vec<u8>, at: usize, v: u64| img[at..at + 8].copy_from_slice(&v.to_le_bytes());
    // The ELF header: 64-bit, little-endian, a shared object.
    img[0..8].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    put16(&mut img, 16, 3);
    put16(&mut img, 18, machine);
    put32(&mut img, 20, 1);
    put64(&mut img, 32, 64);
    put16(&mut img, 52, 64);
    put16(&mut img, 54, 56);
    put16(&mut img, 56, 2);
    put16(&mut img, 58, 64);
    // One loadable segment (read and execute), and the dynamic section.
    let ph = 64;
    put32(&mut img, ph, 1);
    put32(&mut img, ph + 4, 5);
    put64(&mut img, ph + 32, size as u64);
    put64(&mut img, ph + 40, size as u64);
    put64(&mut img, ph + 48, PAGE);
    let ph = 64 + 56;
    put32(&mut img, ph, 2);
    put32(&mut img, ph + 4, 4);
    put64(&mut img, ph + 8, dyn_at as u64);
    put64(&mut img, ph + 16, dyn_at as u64);
    put64(&mut img, ph + 32, dyn_len as u64);
    put64(&mut img, ph + 40, dyn_len as u64);
    put64(&mut img, ph + 48, 8);
    // The hash table: one bucket, every symbol on its chain.
    put32(&mut img, hash_at, 1);
    put32(&mut img, hash_at + 4, nsyms as u32);
    put32(&mut img, hash_at + 8, 1);
    for i in 0..nsyms {
        put32(&mut img, hash_at + 12 + 4 * i, if i == 0 || i + 1 == nsyms { 0 } else { (i + 1) as u32 });
    }
    // The symbols: global functions, defined (in a section that isn't 0), each with its version.
    for (i, (&name, &off)) in sym_names.iter().zip(&offs).enumerate() {
        let at = sym_at + 24 * (i + 1);
        put32(&mut img, at, name);
        img[at + 4] = 0x12;
        put16(&mut img, at + 6, 1);
        put64(&mut img, at + 8, CODE as u64 + off);
        put64(&mut img, at + 16, 8);
        put16(&mut img, versym_at + 2 * (i + 1), 2);
    }
    img[str_at..str_at + strtab.len()].copy_from_slice(&strtab);
    // Version definitions: the image itself (the base), then the version its symbols have.
    for (i, (flags, name, ndx)) in [(1u16, soname, 1u16), (0, ver_name, 2)].iter().enumerate() {
        let at = verdef_at + 28 * i;
        let text = if *ndx == 1 { "linux-vdso.so.1" } else { version };
        put16(&mut img, at, 1);
        put16(&mut img, at + 2, *flags);
        put16(&mut img, at + 4, *ndx);
        put16(&mut img, at + 6, 1);
        put32(&mut img, at + 8, elf_hash(text));
        put32(&mut img, at + 12, 20);
        put32(&mut img, at + 16, if i == 0 { 28 } else { 0 });
        put32(&mut img, at + 20, *name);
    }
    for (i, (tag, value)) in dyn_entries.iter().enumerate() {
        put64(&mut img, dyn_at + 16 * i, *tag);
        put64(&mut img, dyn_at + 16 * i + 8, *value);
    }
    img[CODE..].copy_from_slice(&code);
    Image { bytes: img, sigreturn }
}

// ---- The clock page ----

/// What the vDSO's fallback puts beside its counter reading (in `clock_gettime`'s third and fourth arguments, which
/// Linux ignores, so a seccomp filter sees an ordinary call).
pub(crate) const MARKER: u64 = 0x7664_736f;

/// The host's counter and its frequency, where the guest's ticks with it: on Apple silicon, the hypervisor's virtual
/// counter, the process sandbox's and the translator's (for `rdtsc`) all count the host's ticks, each perhaps from an
/// offset of its own that the vDSO's fallback teaches the kernel. None elsewhere: the clock page stays unready, and
/// the vDSO makes system calls.
fn host_counter() -> Option<(u64, u64)> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let (count, freq): (u64, u64);
        // SAFETY: reading the counter and its frequency, which user code may on macOS.
        unsafe {
            std::arch::asm!("isb", "mrs {c}, cntvct_el0", "mrs {f}, cntfrq_el0", c = out(reg) count, f = out(reg) freq);
        }
        Some((count, freq))
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        None
    }
}

/// The clock page, and what it's written from.
pub(crate) struct Clock {
    pub page: Arc<SharedMem>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The host's counter paired with the host's monotonic clock, once, and nanoseconds per tick (32.32).
    anchor: Option<(u64, u64, u64)>,
    /// How far the guest's counter is ahead of the host's, once the vDSO's said: the most any of its readings has
    /// been (each is read a little before the kernel reads the host's, so a little under the truth), and how many
    /// readings that's from. Ready at `SAMPLES`, and kept from then on (raising it would step the clock back).
    offset: Option<i64>,
    samples: u32,
}

/// The vDSO's fallback readings the clock page waits for before it's ready.
const SAMPLES: u32 = 8;

impl Clock {
    pub(crate) fn new(page: Arc<SharedMem>) -> Clock {
        Clock { page, state: Mutex::new(State::default()) }
    }

    /// Writes the page from the state: the counter's reading in the guest's terms, the monotonic time it was, the
    /// wall clock's offset now, and ready once the guest's offset is known.
    fn write(&self, k: &Kernel, state: &mut State) {
        let Some((c0, freq)) = host_counter() else { return };
        let mono = k.host.monotonic();
        let (c1, _) = host_counter().unwrap_or((c0, freq));
        let wall = k.host.wall();
        let (counter, mono_base, mult) = *state.anchor.get_or_insert_with(|| (c0 / 2 + c1 / 2, mono, ((1_000_000_000u128 << 32) / u128::from(freq.max(1))) as u64));
        let page = &self.page;
        let seq = read_u32(page, vvar::SEQ);
        let put = |at: u64, v: u64| {
            let _ = page.write(at, &v.to_le_bytes());
        };
        let _ = page.write(vvar::SEQ, &(seq.wrapping_add(1) | 1).to_le_bytes());
        fence(Ordering::SeqCst);
        put(vvar::COUNTER, counter.wrapping_add_signed(state.offset.unwrap_or(0)));
        put(vvar::MONO, mono_base);
        put(vvar::REAL, wall.wrapping_sub(mono));
        put(vvar::MULT, mult);
        // A reading this far past the pairing (a day) asks the kernel again.
        put(vvar::LIMIT, freq.saturating_mul(86_400));
        let _ = page.write(vvar::READY, &u32::from(state.offset.is_some() && state.samples >= SAMPLES).to_le_bytes());
        fence(Ordering::SeqCst);
        let _ = page.write(vvar::SEQ, &((seq | 1).wrapping_add(1)).to_le_bytes());
    }
}

/// The vDSO's counter reading, from its fallback: how the guest's counter stands against the host's.
pub(crate) fn sample(k: &Kernel, guest: u64) {
    let Some((host, freq)) = host_counter() else { return };
    let seen = guest.wrapping_sub(host) as i64;
    let clock = &k.clock;
    let mut state = kernels_core::lock(&clock.state);
    // The most seen while learning; once ready, kept unless it's off by more than a millisecond (the host slept, and
    // its counter's offset changed), when it's learned again.
    let ms = (freq / 1000) as i64;
    match state.offset {
        Some(old) if (seen - old).abs() > ms => {
            state.offset = Some(seen);
            state.samples = 1;
        }
        Some(_) if state.samples >= SAMPLES => return,
        Some(old) => {
            state.offset = Some(old.max(seen));
            state.samples += 1;
        }
        None => {
            state.offset = Some(seen);
            state.samples = 1;
        }
    }
    clock.write(k, &mut state);
}

fn read_u32(page: &SharedMem, at: u64) -> u32 {
    let mut b = [0u8; 4];
    page.read(at, &mut b);
    u32::from_le_bytes(b)
}

/// Starts keeping the clock page: written now, and twice a second after (the wall clock may be set on the host).
pub(crate) fn start(k: &Arc<Kernel>) {
    if host_counter().is_none() {
        return;
    }
    k.clock.write(k, &mut kernels_core::lock(&k.clock.state));
    let k = k.clone();
    let _ = std::thread::Builder::new().name("lambda clock".into()).spawn(move || {
        while !k.done.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(500));
            k.clock.write(&k, &mut kernels_core::lock(&k.clock.state));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_shared_objects_with_their_symbols() {
        for (isa, machine, first) in [(Isa::Arm64, 183u16, "__kernel_clock_gettime"), (Isa::X86_64, 62, "__vdso_clock_gettime")] {
            let img = &image(isa).bytes;
            assert_eq!(&img[0..4], b"\x7fELF");
            assert_eq!(u16::from_le_bytes([img[18], img[19]]), machine);
            assert!(img.windows(first.len()).any(|w| w == first.as_bytes()));
            assert!(img.len() <= PAGE as usize);
            assert!(image(isa).sigreturn > CODE as u64 && image(isa).sigreturn < img.len() as u64);
        }
        assert_eq!(elf_hash("LINUX_2.6.39"), 0x075f_cb89);
    }

    #[test]
    fn arm64_branches_land_on_their_labels() {
        let (code, _) = arm64_code(-8192);
        let words: Vec<u32> = code.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        // The first b.hi goes forward to the slow path: is it a `timeval`, then `isb`, the counter, the marker,
        // `mov x8, #113`.
        let target = 2 + ((words[2] >> 5) & 0x7_ffff) as usize;
        assert_eq!(words[target], 0xf10f_a21f);
        assert_eq!(words[target + 6], movz(8, 113, 0));
    }
}
