//! x86-64 and 32-bit x86 programs on an ARM64 vm backend, through `exec-translate`.
//!
//! A 32-bit x86 space is translated the same way, with the translator in its 32-bit mode: its program lives below
//! 4 GiB, and the code cache and lookup table stay where they are, above anything its addresses can reach.
//!
//! A translated space keeps its translations in a code cache at the top of its own address space, mapped for EL0 to
//! run and nothing else. A CPU runs a block until it leaves by `brk`, with the next x86 address in `x16`; core takes
//! that here, before the kernel sees anything, and either goes straight on to the next block or turns it into a stop.
//! The kernel only ever sees x86: `Regs::X86`, x86 addresses, x86 system calls.
//!
//! What core keeps per CPU is where it goes next, when that isn't where it stopped in a translation: after a block
//! exit, at a new thread, or when the kernel moves rip. The CPU's turn resolves it, translating if it must, as it ends.
//!
//! Blocks are chained, so most block exits never reach core. A jump whose target is known when translating exits by
//! `CHAIN`, and once its target is translated core rewrites that exit into a branch straight to it. An indirect jump
//! or a return looks its target up in a table after the code cache, which core fills at each exit that misses it.
//!
//! # When the program's code changes
//!
//! A translation is only right while the bytes it came from stay as they were. Core forgets the blocks of any page
//! the kernel unmaps, moves, remaps, re-protects or writes, and of any page the program writes itself: a page the
//! program may both write and run has its write access taken away while blocks from it are live, so the first write
//! stops here, the blocks go, the access comes back and the write goes ahead. Forgetting a block unlinks every chain
//! exit that branches to it and clears its lookup entry, so nothing reaches it again. Its translation stays where it
//! is: a CPU already inside it finishes it, as a CPU that had fetched the old instructions would.
//!
//! The code cache is used a half at a time. When one half is full, its chains are all unlinked and the lookup table
//! emptied, and translation carries on in the other half. CPUs still in the full half finish the block they're in
//! and leave it at the next exit. The half is only written again once the other has filled too, and then only when
//! no CPU can still be in it: one that has run since its last turn left it there may be finishing a block, or
//! waiting for the lock to take its turn for an exit from one, so translation carries on into the slack at the end of
//! the half until it has taken that turn. One that hasn't run since (one waiting in its kernel) is made to stop
//! before it runs, and its next turn moves it off the old code by its x86 address.

use std::collections::BTreeMap;
use std::ops::Range;

use exec_mmu::{Format, Frame, Native, OutOfMemory, PAGE, Prot};
use exec_translate::emit::arm64::{self, CACHE_BASE, CACHE_SIZE, CHAIN_EXIT, LOOKUP_BASE, LOOKUP_BYTES, REGION_END, X32_EFLAGS, X32_EFLAGS_KEPT, exit};
use exec_translate::runtime::{Block, Cache};

use crate::contract::{Sandbox, Vm};
use crate::exec::{Core, Mode};
use crate::isa::arm64::{Arm64Fp, NZCV};
use crate::isa::convert::{eflags_to_nzcv, merge_nzcv};
use crate::isa::x86::{X86Fp, X86Regs};
use crate::isa::{Trap, x86};
use crate::shim::arm64::Arm64VcpuRegs;
use crate::{FpRegs, Isa, Regs, Stop};

/// Everything a translated space keeps for itself: the code cache and the lookup table after it.
pub(crate) const REGION: std::ops::Range<u64> = CACHE_BASE..REGION_END;
/// The most x86 instructions in a block.
const BLOCK_INSTRUCTIONS: usize = 64;
/// Enough bytes for that many of the longest instructions.
const FETCH_BYTES: u64 = 15 * BLOCK_INSTRUCTIONS as u64 + 16;
/// ARM's carry, which is x86's CF inverted while a translation runs.
const C: u64 = 1 << 29;

/// The instruction sets an ARM64 vm backend takes, translating x86-64 and 32-bit x86.
pub(crate) const TRANSLATING: &[Isa] = &[Isa::Arm64, Isa::X86_64, Isa::X86_32];

/// Half the code cache: one is written to at a time.
const HALF: u64 = CACHE_SIZE / 2;
/// The end of each half past the point where the cache would move on: room to keep writing while a CPU may still be
/// in the other half.
const SLACK: u64 = 64 << 20;

fn half_range(half: u64) -> Range<u64> {
    let start = CACHE_BASE + half * HALF;
    start..start + HALF
}

#[derive(Default)]
pub(crate) struct Translated {
    /// A 32-bit x86 space: translated in the translator's 32-bit mode.
    x32: bool,
    /// The books for the half of the code cache being written to.
    cache: Cache,
    /// The other half's, from before it filled: CPUs may still be finishing blocks there.
    previous: Cache,
    /// Which half is being written to, 0 or 1.
    half: u64,
    /// Where the next translation goes.
    next: u64,
    /// How far each half has been written since it was last emptied.
    written: [u64; 2],
    /// How many times a half has filled: a chain exit seen before then is gone.
    epoch: u64,
    /// Whether the lookup table's pages are mapped.
    lookup: bool,
    /// Each live block's chain exits that branch to it, by its program start.
    links: BTreeMap<u64, Vec<u64>>,
    /// Pages the program may write that live blocks came from, with the access the kernel gave them. Core took write
    /// away so the program's first write comes here, and gives it back when it forgets those blocks.
    guarded: BTreeMap<u64, Prot>,
}

impl Translated {
    /// The books for a new space of this instruction set.
    pub(crate) fn new(isa: Isa) -> Translated {
        Translated { x32: isa == Isa::X86_32, ..Translated::default() }
    }
}

/// Where a CPU was left in a code cache at the end of its last turn: enough to move it off that code later, if the
/// cache has moved on and the CPU hasn't run since.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Resume {
    root: u64,
    elr: u64,
    /// The epoch the code there was written in.
    epoch: u64,
    /// The x86 address there, if it's where an x86 instruction starts.
    pc: Option<u64>,
}

/// `prot` without write access.
fn read_only(prot: Prot) -> Prot {
    let mut out = Prot::NONE;
    if prot.read() {
        out = out | Prot::R;
    }
    if prot.exec() {
        out = out | Prot::X;
    }
    out
}

/// The pages `range` touches.
fn pages(range: Range<u64>) -> impl Iterator<Item = u64> {
    let first = range.start & !(PAGE - 1);
    (first..range.end).step_by(PAGE as usize)
}

/// A chain exit a CPU took, to be made a branch once its target's translated: where it is, where it goes, and the
/// cache's epoch then.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Chain {
    site: u64,
    target: u64,
    epoch: u64,
}

pub(crate) fn in_cache(addr: u64) -> bool {
    (CACHE_BASE..CACHE_BASE + CACHE_SIZE).contains(&addr)
}

/// A chain exit's target: the 8 bytes after its `brk`.
fn literal(words: [u8; 12]) -> u64 {
    u64::from_le_bytes(words[4..12].try_into().unwrap_or_default())
}

/// The program's registers, from a stopped vCPU's: rax–r15 are x0–x15, fs and gs bases x28 and x27. A 32-bit
/// program (`x32`) has eight, and the EFLAGS bits the translator keeps in `x8` (ID and AC) go into its flags.
pub(crate) fn x86_regs(r: &Arm64VcpuRegs, rip: u64, x32: bool) -> Regs {
    let mut gpr = [0; 16];
    let nzcv = (r.spsr & NZCV) ^ C;
    let mut rflags = merge_nzcv(0x202, nzcv);
    if x32 {
        for (g, x) in gpr.iter_mut().zip(&r.x[..8]) {
            *g = x & 0xffff_ffff;
        }
        rflags |= r.x[usize::from(X32_EFLAGS)] & X32_EFLAGS_KEPT;
    } else {
        gpr.copy_from_slice(&r.x[..16]);
    }
    Regs::X86(X86Regs { gpr, rip, rflags, fs_base: r.x[28], gs_base: r.x[27] })
}

/// The other way. A 32-bit program's registers keep their top halves clear, as its translation relies on.
pub(crate) fn set_x86_regs(r: &mut Arm64VcpuRegs, x: &X86Regs, x32: bool) {
    if x32 {
        for (reg, g) in r.x[..16].iter_mut().zip(&x.gpr) {
            *reg = g & 0xffff_ffff;
        }
        r.x[8..16].fill(0);
        r.x[usize::from(X32_EFLAGS)] = x.rflags & X32_EFLAGS_KEPT;
    } else {
        r.x[..16].copy_from_slice(&x.gpr);
    }
    r.x[28] = x.fs_base;
    r.x[27] = x.gs_base;
    r.spsr = (r.spsr & !NZCV) | (eflags_to_nzcv(x.rflags) ^ C);
}

/// Where the translator keeps x87 (see `exec-translate`'s `x87.rs`): ST(0)–ST(7) are doubles in v16–v23, and v24
/// has the control word XOR 0x37F and the status word's condition bits.
const X87_ST: usize = 16;
const X87_CTL: usize = 24;
const FCW_DEFAULT: u16 = 0x37F;

/// xmm0–xmm15 are v0–v15; x87's registers go into `fxsave`'s 80-bit slots, every one tagged empty and TOP 0.
pub(crate) fn x86_fp(fp: &Arm64Fp) -> FpRegs {
    let mut x = X86Fp::default();
    for i in 0..16 {
        x.set_xmm(i, fp.v[i]);
    }
    let ctl = fp.v[X87_CTL] as u32;
    x.fxsave[0..2].copy_from_slice(&(ctl as u16 ^ FCW_DEFAULT).to_le_bytes());
    x.fxsave[2..4].copy_from_slice(&((ctl >> 16) as u16).to_le_bytes());
    x.fxsave[4] = 0;
    for i in 0..8 {
        let at = 32 + 16 * i;
        x.fxsave[at..at + 10].copy_from_slice(&extended(f64::from_bits(fp.v[X87_ST + i] as u64)));
    }
    FpRegs::X86(x)
}

pub(crate) fn set_x86_fp(fp: &mut Arm64Fp, regs: &FpRegs) {
    if let FpRegs::X86(x) = regs {
        for i in 0..16 {
            fp.v[i] = x.xmm(i);
        }
        let fcw = u16::from_le_bytes([x.fxsave[0], x.fxsave[1]]);
        let fsw = u16::from_le_bytes([x.fxsave[2], x.fxsave[3]]) & 0x4700;
        fp.v[X87_CTL] = u128::from(fcw ^ FCW_DEFAULT) | u128::from(fsw) << 16;
        for i in 0..8 {
            let at = 32 + 16 * i;
            let mut bytes = [0u8; 10];
            bytes.copy_from_slice(&x.fxsave[at..at + 10]);
            fp.v[X87_ST + i] = u128::from(from_extended(bytes).to_bits());
        }
    }
}

/// A double as x87's 80-bit format: exact.
pub(crate) fn extended(value: f64) -> [u8; 10] {
    let bits = value.to_bits();
    let sign = (bits >> 63) as u16;
    let exponent = (bits >> 52 & 0x7FF) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (significand, exp) = match exponent {
        0x7FF => (1 << 63 | fraction << 11, 0x7FFF),
        0 if fraction == 0 => (0, 0),
        0 => {
            let shift = fraction.leading_zeros();
            (fraction << shift, 15372 - shift as i32)
        }
        _ => (1 << 63 | fraction << 11, exponent + 15360),
    };
    let mut out = [0u8; 10];
    out[..8].copy_from_slice(&significand.to_le_bytes());
    out[8..].copy_from_slice(&((sign << 15) | exp as u16).to_le_bytes());
    out
}

/// x87's 80-bit format as a double, rounded to nearest.
pub(crate) fn from_extended(bytes: [u8; 10]) -> f64 {
    let mut significand = [0u8; 8];
    significand.copy_from_slice(&bytes[..8]);
    let significand = u64::from_le_bytes(significand);
    let top = u16::from_le_bytes([bytes[8], bytes[9]]);
    let exponent = i64::from(top & 0x7FFF);
    let value = if exponent == 0x7FFF {
        let fraction = significand << 1;
        let quiet = if fraction == 0 { 0x7FF0_0000_0000_0000 } else { 0x7FF8_0000_0000_0000 };
        f64::from_bits(quiet | fraction >> 12)
    } else {
        // significand * 2^(exponent - 16446), in two exact scalings after one rounding.
        let k = (exponent - 16446).clamp(-1140, 970);
        let (k1, k2) = (k >> 1, k - (k >> 1));
        let power = |k: i64| f64::from_bits(((k + 1023) as u64) << 52);
        significand as f64 * power(k1) * power(k2)
    };
    if top & 0x8000 != 0 { -value } else { value }
}

/// What `cpuid` says: an x86-64 baseline CPU (SSE2, no AVX), as much as the translator covers.
fn cpuid(r: &mut Arm64VcpuRegs) {
    let word = |s: &[u8; 4]| u32::from_le_bytes(*s);
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32) = match r.x[0] as u32 {
        0 => (1, word(b"Genu"), word(b"ntel"), word(b"ineI")),
        // Family 6; FPU, TSC, CX8, CMOV, MMX, FXSR, SSE, SSE2.
        1 => (0x0006_0F00, 0, 0, 1 | 1 << 4 | 1 << 8 | 1 << 15 | 1 << 23 | 1 << 24 | 1 << 25 | 1 << 26),
        0x8000_0000 => (0x8000_0001, 0, 0, 0),
        // SYSCALL, NX, long mode.
        0x8000_0001 => (0, 0, 0, 1 << 11 | 1 << 20 | 1 << 29),
        _ => (0, 0, 0, 0),
    };
    r.x[0] = u64::from(eax);
    r.x[3] = u64::from(ebx);
    r.x[1] = u64::from(ecx);
    r.x[2] = u64::from(edx);
}

impl<V: Vm, S: Sandbox> Core<V, S> {
    pub(crate) fn translating(&self, root: u64) -> bool {
        self.translated.contains_key(&root)
    }

    /// Whether a translated space is 32-bit x86.
    pub(crate) fn translating_x32(&self, root: u64) -> bool {
        self.translated.get(&root).is_some_and(|t| t.x32)
    }

    /// Where a stopped thread is in its program: where it goes next, or where it stopped in a translation.
    pub(crate) fn x86_pc(&self, index: usize, root: u64, elr: u64) -> u64 {
        self.entries[index].unwrap_or_else(|| self.program_pc(root, elr))
    }

    fn program_pc(&self, root: u64, pc: u64) -> u64 {
        self.translated.get(&root).and_then(|t| t.cache.program_pc(pc).or_else(|| t.previous.program_pc(pc))).unwrap_or(pc)
    }

    /// Remembers where a CPU's turn left it, as the turn ends.
    pub(crate) fn note_resume(&mut self, index: usize, root: u64, elr: u64) {
        let Some(t) = self.translated.get(&root) else { return };
        self.resume[index] = in_cache(elr).then(|| {
            let current = half_range(t.half).contains(&elr);
            let books = if current { &t.cache } else { &t.previous };
            Resume { root, elr, epoch: if current { t.epoch } else { t.epoch.wrapping_sub(1) }, pc: books.boundary(elr) }
        });
    }

    /// As a CPU's turn starts: if it's still where its last turn left it, in code from before the cache last filled,
    /// it goes on from that x86 address instead, translated again. Its kernel then sees that address too.
    pub(crate) fn catch_up(&mut self, index: usize, root: u64, elr: u64) {
        let Some(t) = self.translated.get(&root) else { return };
        if let Some(resume) = self.resume[index]
            && resume.root == root
            && resume.elr == elr
            && resume.epoch != t.epoch
            && self.entries[index].is_none()
            && let Some(pc) = resume.pc
        {
            self.entries[index] = Some(pc);
        }
    }

    /// Forgets where CPUs were left in a space that's gone.
    pub(crate) fn forget_resumes(&mut self, root: u64) {
        for resume in &mut self.resume {
            if resume.is_some_and(|r| r.root == root) {
                *resume = None;
            }
        }
    }

    /// The access the kernel gave a page, which core may have narrowed while it guards it.
    pub(crate) fn kernel_prot(&self, root: u64, page: u64, prot: Prot) -> Prot {
        self.translated.get(&root).and_then(|t| t.guarded.get(&(page & !(PAGE - 1))).copied()).unwrap_or(prot)
    }

    /// The program's bytes in `range` are changing or going: every live block from those pages is forgotten, chain
    /// exits into them are unlinked and their lookup entries cleared, and pages core guarded get their access back.
    /// Cheap when there's nothing translated there.
    pub(crate) fn forget_code(&mut self, root: u64, range: Range<u64>) {
        let Some(t) = self.translated.get_mut(&root) else { return };
        let first = range.start & !(PAGE - 1);
        let guarded: Vec<(u64, Prot)> = t.guarded.range(first..range.end).map(|(page, prot)| (*page, *prot)).collect();
        if guarded.is_empty() && !t.cache.has_code(range.clone()) {
            return;
        }
        let gone = t.cache.forget(range);
        let mut sites = Vec::new();
        let mut lookups = Vec::new();
        for (start, at) in gone {
            sites.extend(t.links.remove(&start).unwrap_or_default());
            lookups.push(arm64::lookup_entry(start, at));
        }
        for (page, _) in &guarded {
            t.guarded.remove(page);
        }
        for site in sites {
            self.unlink(root, site);
        }
        for (slot, entry) in lookups {
            if let Some(host) = self.host_at(root, LOOKUP_BASE + slot as u64 * 8) {
                let word = host.cast::<u64>();
                // SAFETY: an aligned word of the lookup table, in guest memory. Only this turn writes it.
                unsafe {
                    if word.read_volatile() == entry {
                        word.write_volatile(0);
                    }
                }
            }
        }
        for (page, prot) in guarded {
            self.mmu.protect(root, page..page + PAGE, prot);
            // Giving access back doesn't need other CPUs stopped, but one with the old entry cached would fault again.
            self.mmu.needs_flush = true;
        }
    }

    /// A fork: the copy's pages get the access the kernel gave them, since the copy has no translations to guard.
    pub(crate) fn unguard_copy(&mut self, root: u64, copy: u64) {
        let Some(t) = self.translated.get(&root) else { return };
        let guarded: Vec<(u64, Prot)> = t.guarded.iter().map(|(page, prot)| (*page, *prot)).collect();
        for (page, prot) in guarded {
            self.mmu.protect(copy, page..page + PAGE, prot);
        }
    }

    /// Takes write access away from the pages a new block came from that the program may write, so a write to one
    /// tells core first.
    fn guard(&mut self, root: u64, range: Range<u64>) {
        for page in pages(range) {
            let entry = self.mmu.descriptor(root, page);
            if !Native::is_valid(entry) {
                continue;
            }
            let prot = Native::prot(entry);
            let Some(t) = self.translated.get_mut(&root) else { return };
            if !prot.write() || t.guarded.contains_key(&page) {
                continue;
            }
            t.guarded.insert(page, prot);
            self.mmu.protect(root, page..page + PAGE, read_only(prot));
        }
    }

    /// Makes a linked chain exit a `brk` again. A CPU running it meanwhile sees the branch or the `brk`; either is
    /// right while the block it branched to is still in the cache, which it is until the cache moves on.
    fn unlink(&mut self, root: u64, site: u64) {
        let Some(host) = self.host_at(root, site) else { return };
        // SAFETY: a word of the code cache, aligned, in guest memory.
        unsafe { host.cast::<u32>().write_volatile(CHAIN_EXIT) };
        if let Mode::Vm { vm, .. } = &self.mode {
            vm.sync_code(host, 4);
        }
    }

    /// What a stop in translated code means: a block exit is handled here (`None` to carry on), anything else has its
    /// pc made an x86 one.
    pub(crate) fn translated_stop(&mut self, index: usize, root: u64, r: &mut Arm64VcpuRegs, stop: Stop) -> Option<Stop> {
        let program = |core: &Self, pc: u64| core.program_pc(root, pc);
        self.chains[index] = None;
        match stop {
            Stop::Breakpoint { pc } if in_cache(pc) => {
                // The brk, and for a chain exit the 8 bytes after it.
                let mut words = [0u8; 12];
                let kind = self
                    .mmu
                    .copy(root, pc, 4, false, &mut |host, len, done| {
                        // SAFETY: `len` bytes of a code cache page, into the rest of a 4-byte buffer.
                        unsafe { std::ptr::copy_nonoverlapping(host, words.as_mut_ptr().add(done), len) };
                    })
                    .map(|()| (u32::from_le_bytes([words[0], words[1], words[2], words[3]]) >> 5 & 0xFFFF) as u16);
                // Another CPU linked this exit while this one was taking it: it's a branch now, so just take that.
                let word = u32::from_le_bytes([words[0], words[1], words[2], words[3]]);
                if word & 0xFC00_0000 == 0x1400_0000 {
                    return None;
                }
                if kind == Ok(exit::CHAIN) {
                    let copied = self.mmu.copy(root, pc, 12, false, &mut |host, len, done| {
                        // SAFETY: `len` bytes of the code cache, into the rest of a 12-byte buffer.
                        unsafe { std::ptr::copy_nonoverlapping(host, words.as_mut_ptr().add(done), len) };
                    });
                    if copied.is_err() {
                        return Some(Stop::Internal { pc, detail: 0x7002 });
                    }
                    let target = literal(words);
                    self.entries[index] = Some(target);
                    let epoch = self.translated.get(&root).map_or(0, |t| t.epoch);
                    self.chains[index] = Some(Chain { site: pc, target, epoch });
                    return None;
                }
                let target = r.x[16];
                self.entries[index] = Some(target);
                let Regs::X86(x) = x86_regs(r, target, self.translating_x32(root)) else { return None };
                match kind {
                    Ok(exit::JUMP) => None,
                    Ok(exit::SYSCALL) => Some(x86::syscall(Trap::Syscall, &x)),
                    Ok(exit::INT80) => Some(x86::syscall(Trap::Int80, &x)),
                    Ok(exit::INT2E) => Some(x86::syscall(Trap::Int2e, &x)),
                    Ok(exit::UNSUPPORTED) => Some(Stop::Undefined { pc: target }),
                    Ok(exit::BREAKPOINT) => Some(Stop::Breakpoint { pc: target.wrapping_sub(1) }),
                    Ok(exit::DIVIDE) => Some(Stop::DivideError { pc: target }),
                    Ok(exit::FASTFAIL) => Some(Stop::FastFail { pc: target, code: r.x[1] }),
                    Ok(exit::FETCH) => {
                        let pc = r.x[17];
                        self.entries[index] = Some(pc);
                        self.fetch_reported[index] = Some(pc);
                        Some(Stop::Fault { addr: target, access: Prot::X, pc })
                    }
                    Ok(exit::CPUID) => {
                        cpuid(r);
                        None
                    }
                    _ => Some(Stop::Internal { pc, detail: 0x7000 }),
                }
            }
            Stop::Fault { addr, access, pc } => {
                // A write to a page core guards: its blocks go, the page gets its access back, and the write runs again.
                let page = addr & !(PAGE - 1);
                if access.write() && self.translated.get(&root).is_some_and(|t| t.guarded.contains_key(&page)) {
                    self.forget_code(root, page..page + PAGE);
                    return None;
                }
                Some(Stop::Fault { addr, access, pc: program(self, pc) })
            }
            Stop::Undefined { pc } => Some(Stop::Undefined { pc: program(self, pc) }),
            Stop::Alignment { pc } => Some(Stop::Alignment { pc: program(self, pc) }),
            Stop::Breakpoint { pc } => Some(Stop::Breakpoint { pc: program(self, pc) }),
            Stop::Syscall { .. } => Some(Stop::Internal { pc: r.elr, detail: 0x7001 }),
            other => Some(other),
        }
    }

    /// Points the CPU at the translation of where it goes next, if that's changed.
    pub(crate) fn enter_translation(&mut self, index: usize, root: u64, r: &mut Arm64VcpuRegs) {
        let chain = self.chains[index].take();
        let Some(pc) = self.entries[index] else { return };
        if let Ok(Some(at)) = self.translation(index, root, pc) {
            r.elr = at;
            self.entries[index] = None;
            // Only a translation that's kept is linked to: one cut short by code it couldn't fetch is made again.
            let Some(t) = self.translated.get(&root) else { return };
            if t.cache.find(pc).is_none_or(|block| block.at != at) {
                return;
            }
            let (epoch, half) = (t.epoch, half_range(t.half));
            match chain {
                // Only exits in the half being written are linked: the other's are left to leave by `brk`.
                Some(chain) if chain.target == pc && chain.epoch == epoch && half.contains(&chain.site) => {
                    self.link(root, chain.site, at, pc)
                }
                _ => self.remember(root, pc, at),
            }
        }
    }

    /// Makes the chain exit at `site` a branch to `at`, the block at `pc`. A CPU running it meanwhile sees the `brk`
    /// or the branch: ARM lets one replace the other while it runs.
    fn link(&mut self, root: u64, site: u64, at: u64, pc: u64) {
        let Some(word) = arm64::chain_branch(site, at) else { return };
        let Some(host) = self.host_at(root, site) else { return };
        if let Some(t) = self.translated.get_mut(&root) {
            t.links.entry(pc).or_default().push(site);
        }
        // SAFETY: a word of the code cache, aligned, in guest memory.
        unsafe { host.cast::<u32>().write_volatile(word) };
        if let Mode::Vm { vm, .. } = &self.mode {
            vm.sync_code(host, 4);
        }
    }

    /// Puts the block at `pc` in the lookup table. One aligned word, so a CPU looking it up meanwhile sees the old
    /// entry or the new one.
    fn remember(&mut self, root: u64, pc: u64, at: u64) {
        let (slot, entry) = arm64::lookup_entry(pc, at);
        if let Some(host) = self.host_at(root, LOOKUP_BASE + slot as u64 * 8) {
            // SAFETY: an aligned word of the lookup table, in guest memory.
            unsafe { host.cast::<u64>().write_volatile(entry) };
        }
    }

    /// Where a byte of the space's own region is in the host.
    fn host_at(&self, root: u64, va: u64) -> Option<*mut u8> {
        let page = va & !(PAGE - 1);
        let entry = self.mmu.descriptor(root, page);
        if !Native::is_valid(entry) {
            return None;
        }
        let frame = Frame(entry & Native::ADDRESS);
        // SAFETY: within the frame.
        Some(unsafe { self.mmu.host(frame).add((va - page) as usize) })
    }

    /// The translation of the block at `pc`, made now if there isn't one. None while its first instruction still
    /// can't be fetched and its kernel has already been told: the entry stays pending, so the turn after the kernel
    /// maps the page translates it again. Made at the turn that reported it, before the kernel has had a chance, it
    /// would only report the same fault once more.
    fn translation(&mut self, index: usize, root: u64, pc: u64) -> Result<Option<u64>, OutOfMemory> {
        if let Some(block) = self.translated.get(&root).and_then(|t| t.cache.find(pc)) {
            self.fetch_reported[index] = None;
            return Ok(Some(block.at));
        }
        let bytes = self.fetch(root, pc);
        let translation = arm64::translate(pc, &bytes, BLOCK_INSTRUCTIONS, self.translating_x32(root));
        let stuck = !translation.cacheable && translation.map.len() <= 1;
        if stuck && self.fetch_reported[index] == Some(pc) {
            return Ok(None);
        }
        if !stuck {
            self.fetch_reported[index] = None;
        }
        let at = self.place(index, root, &translation.code)?;
        if translation.cacheable
            && let Some(t) = self.translated.get_mut(&root)
        {
            let end = translation.end;
            t.cache.insert(Block { start: pc, end, at, len: translation.code.len() as u32, map: translation.map });
            self.guard(root, pc..end);
        }
        Ok(Some(at))
    }

    /// The program's code from `pc`, as far as it's mapped and executable, up to a block's worth.
    fn fetch(&self, root: u64, pc: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(FETCH_BYTES as usize);
        let end = pc.saturating_add(FETCH_BYTES);
        let mut at = pc;
        while at < end {
            let page = at & !(PAGE - 1);
            let entry = self.mmu.descriptor(root, page);
            if !Native::is_valid(entry) || !Native::prot(entry).exec() {
                break;
            }
            let len = ((page + PAGE).min(end) - at) as usize;
            let start = bytes.len();
            bytes.resize(start + len, 0);
            let target = bytes[start..].as_mut_ptr();
            let copied = self.mmu.copy(root, at, len, false, &mut |host, part, done| {
                // SAFETY: `part` bytes of a frame, into the part of the buffer at `done`.
                unsafe { std::ptr::copy_nonoverlapping(host, target.add(done), part) };
            });
            if copied.is_err() {
                bytes.truncate(start);
                break;
            }
            at += len as u64;
        }
        bytes
    }

    /// Whether no CPU but `me` can still be in code from before the current epoch, so the other half can be written
    /// again. A CPU whose last turn left it there and that hasn't run since is made to stop before it runs, so its next
    /// turn moves it on (`catch_up`); one that has run since may still be finishing a block there, or waiting to take
    /// its turn for an exit from one, and the cache waits for it.
    fn other_half_free(&self, me: usize, root: u64) -> bool {
        let Some(t) = self.translated.get(&root) else { return true };
        let cpus = self.cpus.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let mut free = true;
        for (index, cpu) in cpus.iter().enumerate() {
            if index == me || cpu.gone.load(std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            let Some(resume) = self.resume.get(index).copied().flatten() else { continue };
            if resume.root != root || resume.epoch >= t.epoch {
                continue;
            }
            // Asked to stop first, then looked at: it either shows as running, or sees the request before it runs.
            cpu.preempt();
            let ran = cpu.ran.load(std::sync::atomic::Ordering::SeqCst) || cpu.running.load(std::sync::atomic::Ordering::SeqCst);
            if ran || resume.pc.is_none() {
                free = false;
            }
        }
        free
    }

    /// The half being written is full: unlink every chain exit in it and empty the lookup table, so CPUs still in it
    /// leave at their next exit, and carry on in the other half, emptied first.
    fn next_half(&mut self, root: u64) {
        let Some(t) = self.translated.get_mut(&root) else { return };
        let sites: Vec<u64> = std::mem::take(&mut t.links).into_values().flatten().collect();
        let guarded = std::mem::take(&mut t.guarded);
        let lookup = t.lookup;
        t.previous = std::mem::take(&mut t.cache);
        t.half ^= 1;
        t.epoch += 1;
        t.next = half_range(t.half).start;
        // Only as far as it was written: unmapping the rest of a half would only walk empty tables.
        let half = half_range(t.half).start..t.written[t.half as usize].max(half_range(t.half).start);
        t.written[t.half as usize] = 0;
        for site in sites {
            self.unlink(root, site);
        }
        if lookup {
            for page in (LOOKUP_BASE..LOOKUP_BASE + LOOKUP_BYTES).step_by(PAGE as usize) {
                if let Some(host) = self.host_at(root, page) {
                    let words = host.cast::<u64>();
                    for i in 0..(PAGE / 8) as usize {
                        // SAFETY: an aligned word of the lookup table's page, in guest memory.
                        unsafe { words.add(i).write_volatile(0) };
                    }
                }
            }
        }
        // No live blocks are left to guard.
        for (page, prot) in guarded {
            self.mmu.protect(root, page..page + PAGE, prot);
        }
        // What's in the half taken now was emptied a half ago: nothing can still be in it (see the module's notes).
        // Unmapping gives its frames back, and makes a CPU that somehow is fault rather than run what comes next.
        self.mmu.unmap(root, half);
        self.mmu.needs_flush = true;
    }

    /// Writes `code` into the space's code cache, mapping pages for it as needed, and gives where it went.
    fn place(&mut self, index: usize, root: u64, code: &[u8]) -> Result<u64, OutOfMemory> {
        let len = code.len() as u64;
        let Some(t) = self.translated.get(&root) else { return Ok(CACHE_BASE) };
        let half = half_range(t.half);
        let mut at = t.next.max(half.start);
        // A soft limit: past it the cache moves on as soon as no CPU can still be in the other half, and until then
        // goes on writing into the slack. Only when that's gone too does it move on regardless.
        let limit = half.start + self.tuning.code_cache.clamp(64 << 10, HALF - SLACK);
        if at + len > limit && at > half.start && (self.other_half_free(index, root) || at + len > half.end) {
            self.next_half(root);
            at = self.translated.get(&root).map_or(CACHE_BASE, |t| t.next);
        }
        // The lookup table is there whenever code is: every indirect jump reads it.
        if !self.translated.get(&root).is_some_and(|t| t.lookup) {
            for page in (LOOKUP_BASE..LOOKUP_BASE + LOOKUP_BYTES).step_by(PAGE as usize) {
                let frame = self.mmu.new_frame()?;
                self.mmu.map(root, page, frame, Prot::R, false, false)?;
            }
            if let Some(t) = self.translated.get_mut(&root) {
                t.lookup = true;
            }
        }
        let mut done = 0u64;
        while done < len {
            let va = at + done;
            let page = va & !(PAGE - 1);
            let mut entry = self.mmu.descriptor(root, page);
            if !Native::is_valid(entry) {
                let frame = self.mmu.new_frame()?;
                self.mmu.map(root, page, frame, Prot::RX, false, false)?;
                entry = self.mmu.descriptor(root, page);
            }
            let frame = Frame(entry & Native::ADDRESS);
            let offset = (va - page) as usize;
            let part = ((PAGE - (va - page)).min(len - done)) as usize;
            self.mmu.frame_write(frame, offset, &code[done as usize..done as usize + part]);
            if let Mode::Vm { vm, .. } = &self.mode {
                // SAFETY: within the frame just written.
                vm.sync_code(unsafe { self.mmu.host(frame).add(offset) }, part);
            }
            done += part as u64;
        }
        if let Some(t) = self.translated.get_mut(&root) {
            t.next = at + len;
            let half = t.half as usize;
            t.written[half] = t.written[half].max(t.next);
        }
        Ok(at)
    }
}

/// Every translated space, by root.
pub(crate) type Spaces = BTreeMap<u64, Translated>;

#[cfg(test)]
mod tests {
    use super::{extended, from_extended};

    fn bytes(significand: u64, top: u16) -> [u8; 10] {
        let mut out = [0u8; 10];
        out[..8].copy_from_slice(&significand.to_le_bytes());
        out[8..].copy_from_slice(&top.to_le_bytes());
        out
    }

    #[test]
    fn extended_round_trips() {
        for value in [0.0, -0.0, 1.0, -2.5, 0.1, 1e300, -1e-300, 5e-324, f64::MIN_POSITIVE, f64::MAX, f64::INFINITY] {
            assert_eq!(from_extended(extended(value)).to_bits(), value.to_bits(), "{value}");
        }
        assert!(from_extended(extended(f64::NAN)).is_nan());
        assert_eq!(extended(1.0), bytes(1 << 63, 0x3FFF));
        assert_eq!(extended(-2.5), bytes(0xA000_0000_0000_0000, 0xC000));
        assert_eq!(extended(5e-324), bytes(1 << 63, 0x3FFF - 1074));
    }

    #[test]
    fn extended_rounds_to_a_double() {
        // 1/3 to 64 bits, then to 53: the double nearest a third.
        assert_eq!(from_extended(bytes(0xAAAA_AAAA_AAAA_AAAB, 0x3FFD)), 1.0 / 3.0);
        // Past a double's range either way.
        assert_eq!(from_extended(bytes(1 << 63, 0x3FFF + 1024)), f64::INFINITY);
        assert_eq!(from_extended(bytes(1 << 63, 0x3FFF - 1100)), 0.0);
        assert_eq!(from_extended(bytes(1 << 63, 0xBFFF - 1074)), -5e-324);
    }
}
