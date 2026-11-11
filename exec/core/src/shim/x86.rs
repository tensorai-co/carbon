//! The x86-64 shim, for any hypervisor that runs x86 guests (kvm, winkvm).
//!
//! The program runs at ring 3. Every interrupt, exception and `syscall` goes to a one-byte stub that halts, with
//! interrupts off, so the halt exits to the host, and which stub says what happened. The program's rip, rsp, rflags,
//! cs and ss are then in an interrupt frame at the top of that CPU's ring-0 stack, which is where core reads and
//! changes them, and it goes back by `iretq`. The shim's pages are in the top 512 GiB of every space, out of reach.

use exec_mmu::x86::{ACCESSED, DIRTY, NX, P, RW};
use exec_mmu::{Frame, Mmu, Native, OutOfMemory, PAGE};

use super::Shim;
use crate::isa::Trap;
use crate::isa::x86::{self as isa, R11, RCX, RFLAGS_DEFAULT, RSP, USER_FLAGS, X86Fp, X86Regs};
use crate::{FpRegs, Isa, Options, Regs, Stop};

/// The shim's pages: the top 512 GiB of every space.
pub const KERNEL: u64 = 0xffff_ff80_0000_0000;
pub const GDT: u64 = KERNEL;
pub const IDT: u64 = KERNEL + 0x1000;
/// A halt for each vector, 16 bytes apart.
pub const STUBS: u64 = KERNEL + 0x2000;
/// How every CPU goes back to its program: `iretq`.
pub const IRET_STUB: u64 = KERNEL + 0x3000;
/// The same, reloading cr3 first, for a CPU whose TLB is behind: `push rax; mov rax, cr3; mov cr3, rax; pop rax; iretq`.
pub const FLUSH_STUB: u64 = KERNEL + 0x3010;
/// Where `syscall` goes: a halt.
pub const SYSCALL_STUB: u64 = KERNEL + 0x3040;
/// Each CPU's TSS and ring-0 stack, 16 KiB apart.
pub const CPU_AREAS: u64 = KERNEL + 0x10_0000;
pub const CPU_AREA: u64 = 0x4000;
/// The kernel table's entries from here are CPU areas, 4 pages each.
const FIRST_AREA_ENTRY: u64 = 256;

pub const KERNEL_CODE: u16 = 0x10;
pub const KERNEL_DATA: u16 = 0x18;
pub const USER_CODE32: u16 = 0x23;
pub const USER_DATA: u16 = 0x2b;
pub const USER_CODE64: u16 = 0x33;
pub const TSS: u16 = 0x40;
/// 32-bit Windows programs find their thread block through fs, with this selector.
pub const USER_TEB32: u16 = 0x53;

pub const MSR_STAR: u32 = 0xc000_0081;
pub const MSR_LSTAR: u32 = 0xc000_0082;
pub const MSR_CSTAR: u32 = 0xc000_0083;
pub const MSR_SFMASK: u32 = 0xc000_0084;

/// The interrupt frame: rip, cs, rflags, rsp, ss.
const FRAME_BYTES: u64 = 40;

/// A vCPU's registers, as the backend moves them: the general registers, and the system state that changes.
#[derive(Debug, Clone, Copy, Default)]
pub struct X86VcpuRegs {
    /// In encoding order: rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15.
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub cs: u16,
    pub ss: u16,
    pub fs: u16,
    pub fs_base: u64,
    pub gs_base: u64,
    /// The address of the last page fault.
    pub cr2: u64,
}

/// A segment, as a hypervisor's API wants it described.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct X86Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub kind: u8,
    pub present: bool,
    pub dpl: u8,
    pub db: bool,
    pub s: bool,
    pub l: bool,
    pub g: bool,
}

/// The segment for one of the shim's selectors.
pub fn segment(selector: u16, base: u64) -> X86Segment {
    let flat = X86Segment { base, limit: 0xffff_ffff, selector, present: true, s: true, g: true, ..X86Segment::default() };
    match selector {
        KERNEL_CODE => X86Segment { kind: 0xb, l: true, ..flat },
        KERNEL_DATA => X86Segment { kind: 3, db: true, ..flat },
        USER_CODE64 => X86Segment { kind: 0xb, dpl: 3, l: true, ..flat },
        USER_CODE32 => X86Segment { kind: 0xb, dpl: 3, db: true, ..flat },
        TSS => X86Segment { base, limit: 0x67, selector, kind: 0xb, present: true, ..X86Segment::default() },
        _ => X86Segment { kind: 3, dpl: 3, db: true, ..flat },
    }
}

/// A CPU's system state, set once.
#[derive(Debug, Clone)]
pub struct X86System {
    pub cr0: u64,
    pub cr4: u64,
    pub efer: u64,
    pub gdt: (u64, u16),
    pub idt: (u64, u16),
    pub tr: X86Segment,
    pub msrs: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, Copy)]
pub struct X86Cpu {
    /// The CPU area's address in the guest.
    area: u64,
    /// The frame holding the top page of its ring-0 stack, where the interrupt frame is.
    stack_top: Frame,
}

pub struct X86Shim {
    /// The last-level table for the shim's pages.
    table: Frame,
    code: Vec<Frame>,
}

const fn kernel_page(frame: Frame, code: bool) -> u64 {
    frame.0 | P | ACCESSED | DIRTY | if code { 0 } else { RW | NX }
}

impl X86Shim {
    fn map(&self, mmu: &Mmu<Native>, va: u64, frame: Frame, code: bool) {
        let index = (va >> 12) & 511;
        // SAFETY: the shim's last-level table, a frame of 512 entries.
        unsafe { mmu.host(self.table).cast::<u64>().add(index as usize).write_volatile(kernel_page(frame, code)) };
    }

    /// Where an address on a CPU's ring-0 stack is in the host. Only the stack's top page is ever read.
    fn stack(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, va: u64) -> Option<*mut u64> {
        let top = cpu.area + CPU_AREA;
        if va < top - PAGE || va + 8 > top {
            return None;
        }
        // SAFETY: within the stack's top page.
        Some(unsafe { mmu.host(cpu.stack_top).add((va - (top - PAGE)) as usize).cast() })
    }

    fn frame_at(cpu: &X86Cpu) -> u64 {
        cpu.area + CPU_AREA - FRAME_BYTES
    }

    fn read_frame(&self, mmu: &Mmu<Native>, cpu: &X86Cpu) -> [u64; 5] {
        let mut frame = [0; 5];
        if let Some(at) = self.stack(mmu, cpu, Self::frame_at(cpu)) {
            for (i, value) in frame.iter_mut().enumerate() {
                // SAFETY: five values within the stack's top page.
                *value = unsafe { at.add(i).read() };
            }
        }
        frame
    }

    /// Puts the program's state in the frame, and the CPU at `iretq`, ready to go back to it.
    fn push_frame(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &mut X86VcpuRegs, frame: [u64; 5]) {
        if let Some(at) = self.stack(mmu, cpu, Self::frame_at(cpu)) {
            for (i, value) in frame.iter().enumerate() {
                // SAFETY: five values within the stack's top page.
                unsafe { at.add(i).write(*value) };
            }
        }
        regs.gpr[RSP] = Self::frame_at(cpu);
        regs.rip = IRET_STUB;
        regs.cs = KERNEL_CODE;
        regs.ss = KERNEL_DATA;
        regs.rflags = RFLAGS_DEFAULT & !(1 << 9);
    }
}

impl Shim for X86Shim {
    const ISAS: &'static [Isa] = &[Isa::X86_64, Isa::X86_32];
    const MAX_CPUS: usize = ((512 - FIRST_AREA_ENTRY) / (CPU_AREA / PAGE)) as usize;
    type Regs = X86VcpuRegs;
    type Fp = X86Fp;
    type System = X86System;
    type CpuData = X86Cpu;

    fn install(mmu: &mut Mmu<Native>, _options: &Options) -> Result<X86Shim, OutOfMemory> {
        // A directory pointer table, directory and table for the top 512 GiB, of which only the first 2 MiB is used.
        let (pdpt, pd, table) = (mmu.new_frame()?, mmu.new_frame()?, mmu.new_frame()?);
        // SAFETY: the frames just taken, 512 entries each.
        unsafe {
            mmu.host(pdpt).cast::<u64>().write_volatile(pd.0 | P | RW | ACCESSED);
            mmu.host(pd).cast::<u64>().write_volatile(table.0 | P | RW | ACCESSED);
        }
        mmu.set_kernel_entry(511, pdpt.0 | P | RW | ACCESSED);
        let mut shim = X86Shim { table, code: Vec::new() };

        let gdt = mmu.new_frame()?;
        let entries: [(u16, u64); 8] = [
            (KERNEL_CODE, 0x00af_9b00_0000_ffff),
            (KERNEL_DATA, 0x00cf_9300_0000_ffff),
            (USER_CODE32 & !3, 0x00cf_fb00_0000_ffff),
            (USER_DATA & !3, 0x00cf_f300_0000_ffff),
            (USER_CODE64 & !3, 0x00af_fb00_0000_ffff),
            (USER_TEB32 & !3, 0x00cf_f300_0000_ffff),
            // The TSS is per CPU, through each one's cached descriptor; this only has to be valid.
            (TSS, 0x0000_8b00_0000_0067),
            (TSS + 8, 0),
        ];
        for (offset, value) in entries {
            mmu.frame_write(gdt, usize::from(offset), &value.to_le_bytes());
        }
        shim.map(mmu, GDT, gdt, false);

        let idt = mmu.new_frame()?;
        for vector in 0..256u64 {
            let target = STUBS + vector * 16;
            let dpl: u64 = if isa::user_vector(vector as u8) { 3 } else { 0 };
            let low = (target & 0xffff) | u64::from(KERNEL_CODE) << 16 | (0x8e | dpl << 5) << 40 | (target >> 16 & 0xffff) << 48;
            let high = target >> 32;
            mmu.frame_write(idt, vector as usize * 16, &low.to_le_bytes());
            mmu.frame_write(idt, vector as usize * 16 + 8, &high.to_le_bytes());
        }
        shim.map(mmu, IDT, idt, false);

        let stubs = mmu.new_frame()?;
        let mut code = vec![0xccu8; PAGE as usize];
        for vector in 0..256 {
            code[vector * 16] = 0xf4;
        }
        mmu.frame_write(stubs, 0, &code);
        shim.map(mmu, STUBS, stubs, true);

        let tail = mmu.new_frame()?;
        let mut code = vec![0xccu8; PAGE as usize];
        code[0..2].copy_from_slice(&[0x48, 0xcf]);
        code[0x10..0x1a].copy_from_slice(&[0x50, 0x0f, 0x20, 0xd8, 0x0f, 0x22, 0xd8, 0x58, 0x48, 0xcf]);
        code[0x40] = 0xf4;
        mmu.frame_write(tail, 0, &code);
        shim.map(mmu, IRET_STUB, tail, true);

        shim.code = vec![stubs, tail];
        Ok(shim)
    }

    fn code_frames(&self) -> Vec<Frame> {
        self.code.clone()
    }

    fn add_cpu(&self, mmu: &mut Mmu<Native>, index: usize) -> Result<(X86System, X86VcpuRegs, X86Cpu), OutOfMemory> {
        if index >= Self::MAX_CPUS {
            return Err(OutOfMemory);
        }
        let area = CPU_AREAS + index as u64 * CPU_AREA;
        let mut frames = [Frame(0); 4];
        for (page, frame) in frames.iter_mut().enumerate() {
            *frame = mmu.new_frame()?;
            self.map(mmu, area + page as u64 * PAGE, *frame, false);
        }
        // The TSS: where interrupts from ring 3 find their stack, and no I/O bitmap.
        mmu.frame_write(frames[0], 4, &(area + CPU_AREA).to_le_bytes());
        mmu.frame_write(frames[0], 102, &104u16.to_le_bytes());

        let system = X86System {
            // PE, MP, ET, NE, WP, PG
            cr0: 1 | 1 << 1 | 1 << 4 | 1 << 5 | 1 << 16 | 1 << 31,
            // PAE, OSFXSR, OSXMMEXCPT, OSXSAVE
            cr4: 1 << 5 | 1 << 9 | 1 << 10 | 1 << 18,
            // SCE, LME, LMA, NXE
            efer: 1 | 1 << 8 | 1 << 10 | 1 << 11,
            gdt: (GDT, 0x5f),
            idt: (IDT, 0xfff),
            tr: segment(TSS, area),
            msrs: vec![
                (MSR_STAR, u64::from(USER_CODE32) << 48 | u64::from(KERNEL_CODE) << 32),
                (MSR_LSTAR, SYSCALL_STUB),
                (MSR_CSTAR, SYSCALL_STUB),
                // TF, DF, IF, AC and NT are cleared on `syscall`.
                (MSR_SFMASK, 0x4_7700),
            ],
        };
        let mut regs = X86VcpuRegs { rip: IRET_STUB, rflags: 2, cs: KERNEL_CODE, ss: KERNEL_DATA, fs: USER_DATA, ..X86VcpuRegs::default() };
        regs.gpr[RSP] = area + CPU_AREA;
        Ok((system, regs, X86Cpu { area, stack_top: frames[3] }))
    }

    fn root_register(&self, root: u64) -> u64 {
        root
    }

    fn before_run(&self, regs: &mut X86VcpuRegs, flush: bool) {
        if flush && regs.rip == IRET_STUB {
            regs.rip = FLUSH_STUB;
        }
    }

    fn trap(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &mut X86VcpuRegs, at: u64) -> Stop {
        // `syscall` left the program's rip in rcx and its flags in r11, and its stack as it was.
        if at == SYSCALL_STUB || at == SYSCALL_STUB + 1 {
            let frame = [regs.gpr[RCX], u64::from(USER_CODE64), regs.gpr[R11], regs.gpr[RSP], u64::from(USER_DATA)];
            self.push_frame(mmu, cpu, regs, frame);
            let Regs::X86(program) = self.program(mmu, cpu, regs) else { unreachable!() };
            return isa::syscall(Trap::Syscall, &program);
        }
        if !(STUBS..STUBS + 0x1000).contains(&at) {
            return Stop::Internal { pc: at, detail: 0x3500 };
        }
        // `at` is the halt or just past it: both are in the vector's 16 bytes.
        let vector = ((at - STUBS) / 16) as u8;
        let error = if isa::has_error_code(vector) {
            let code = self.stack(mmu, cpu, regs.gpr[RSP]).map_or(0, |at| {
                // SAFETY: on the stack's top page, where the CPU pushed it.
                unsafe { at.read() }
            });
            regs.gpr[RSP] += 8;
            code
        } else {
            0
        };
        regs.rip = IRET_STUB;
        // An exception from ring 3 left ss null, as 64-bit mode does; going back in through `iretq`, the CPU needs
        // the shim's own, as the system-call path sets. A hypervisor told a null ss gets a ring-3 segment under ring-0
        // code, which VMX refuses as invalid guest state.
        regs.ss = KERNEL_DATA;
        let frame = self.read_frame(mmu, cpu);
        if frame[1] & 3 != 3 {
            return Stop::Internal { pc: frame[0], detail: 0x3600 | u64::from(vector) };
        }
        let Regs::X86(program) = self.program(mmu, cpu, regs) else { unreachable!() };
        isa::exception(vector, frame[0], error, regs.cr2, &program)
    }

    fn preempted(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &mut X86VcpuRegs) -> Stop {
        // In the program: its state goes into a frame, as if it had trapped.
        if regs.cs & 3 == 3 {
            let frame = [regs.rip, u64::from(regs.cs), regs.rflags, regs.gpr[RSP], u64::from(regs.ss)];
            self.push_frame(mmu, cpu, regs, frame);
            return Stop::Preempted;
        }
        match regs.rip {
            IRET_STUB | FLUSH_STUB => Stop::Preempted,
            // Part way through the flush, with rax pushed: undone, to start again.
            rip if (FLUSH_STUB + 1..FLUSH_STUB + 8).contains(&rip) => {
                if let Some(at) = self.stack(mmu, cpu, regs.gpr[RSP]) {
                    // SAFETY: on the stack's top page, where the stub pushed rax.
                    regs.gpr[0] = unsafe { at.read() };
                }
                regs.gpr[RSP] += 8;
                regs.rip = FLUSH_STUB;
                Stop::Preempted
            }
            // At the flush's `iretq`: the same as being at the other one.
            rip if rip == FLUSH_STUB + 8 => {
                regs.rip = IRET_STUB;
                Stop::Preempted
            }
            // In a stub, about to halt.
            rip => self.trap(mmu, cpu, regs, rip),
        }
    }

    fn program(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &X86VcpuRegs) -> Regs {
        let frame = self.read_frame(mmu, cpu);
        let mut gpr = regs.gpr;
        gpr[RSP] = frame[3];
        Regs::X86(X86Regs { gpr, rip: frame[0], rflags: frame[2], fs_base: regs.fs_base, gs_base: regs.gs_base })
    }

    fn set_program(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &mut X86VcpuRegs, program: &Regs) {
        let Regs::X86(program) = program else { return };
        let mut frame = self.read_frame(mmu, cpu);
        if frame[1] & 3 != 3 {
            // A CPU that has never run a program has no frame yet: give it a 64-bit program's selectors and flags,
            // or `iretq` would load a null code segment.
            frame[1] = u64::from(USER_CODE64);
            frame[2] = RFLAGS_DEFAULT;
            frame[4] = u64::from(USER_DATA);
        }
        let ring0 = regs.gpr[RSP];
        regs.gpr = program.gpr;
        // rsp here is the ring-0 stack's; the program's is in its frame.
        regs.gpr[RSP] = ring0;
        frame[0] = program.rip;
        frame[2] = (frame[2] & !USER_FLAGS) | (program.rflags & USER_FLAGS);
        frame[3] = program.gpr[RSP];
        regs.fs_base = program.fs_base;
        regs.gs_base = program.gs_base;
        self.push_frame(mmu, cpu, regs, frame);
    }

    fn start(&self, mmu: &Mmu<Native>, cpu: &X86Cpu, regs: &mut X86VcpuRegs, isa: Isa, entry: u64, stack: u64) {
        let code = if isa == Isa::X86_32 { USER_CODE32 } else { USER_CODE64 };
        regs.gpr = [0; 16];
        regs.fs = if isa == Isa::X86_32 { USER_TEB32 } else { USER_DATA };
        regs.fs_base = 0;
        regs.gs_base = 0;
        self.push_frame(mmu, cpu, regs, [entry, u64::from(code), RFLAGS_DEFAULT, stack, u64::from(USER_DATA)]);
    }

    fn set_return(&self, regs: &mut X86VcpuRegs, value: u64) {
        regs.gpr[0] = value;
    }

    fn program_fp(fp: &X86Fp) -> FpRegs {
        FpRegs::X86(*fp)
    }

    fn set_program_fp(fp: &mut X86Fp, program: &FpRegs) {
        if let FpRegs::X86(program) = program {
            *fp = *program;
        }
    }
}
