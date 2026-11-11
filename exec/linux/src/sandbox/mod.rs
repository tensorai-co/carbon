//! `sandbox` on Linux x86-64: the program runs natively in a stub process, driven by `ptrace`, with no hypervisor,
//! root or driver.
//!
//! - **Memory**: guest RAM is the memfd `ram.rs` makes, which the stub has at a fixed descriptor; core's mirror maps
//!   each guest page into the stub at the program's own address, so its addresses are real and its code runs as is.
//! - **System calls**: Syscall User Dispatch turns every `syscall` (and `int 0x80`) outside the stub's own code into a
//!   `SIGSYS`, which stops the stub for Carbon (`Reach::Dispatch`). atlas's `int 0x2e` is a general protection fault
//!   with `cd 2e` at rip, read as a call too.
//! - **Lockdown**: the stub has guest memory and nothing else, no privileges, a read-only dispatch selector, and a
//!   seccomp filter that kills it for any real system call but `mmap`, `mprotect`, `munmap` and exit.
//! - **Faults** stop the stub as signals, which Carbon reads and swallows.
//!
//! One CPU for now: the stub is one thread, traced by the thread that opened the sandbox.
#![cfg(target_arch = "x86_64")]

mod elf;
mod process;
mod seccomp;
mod sys;

use std::sync::Arc;

use exec_core::contract::{FaultKind, GuestRam, HostFault, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::Trap;
use exec_core::isa::x86::{X86Fp, X86Regs};
use exec_core::{FpRegs, Isa, Layout, Options, Prot, Regs};

use crate::ram;
use process::{PREEMPT_SIGNAL, Process};

pub struct Stub {
    process: Arc<Process>,
}

impl Sandbox for Stub {
    type Cpu = StubCpu;
    const NAME: &'static str = "linux";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Stub, String> {
        let process = Process::spawn(ram.handle as i32)?;
        Ok(Stub { process: Arc::new(process) })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Dispatch
    }

    fn layout(&self, isa: Isa) -> Layout {
        Layout { top: exec_core::isa::space_top(isa).min(elf::STUB), kept: Vec::new(), gate: None }
    }

    fn max_cpus(&self) -> usize {
        1
    }

    fn mirror(&mut self, page: u64, ram_offset: Option<u64>, prot: Prot) -> Result<(), String> {
        if page >= elf::STUB {
            return Err(format!("{page:#x} is the stub's"));
        }
        self.process.set_executable(page, ram_offset.is_some() && prot.exec());
        match ram_offset {
            Some(offset) => self.process.map(page, offset, host_prot(prot)),
            None => self.process.unmap(page, 4096),
        }
    }

    fn new_cpu(&self, index: usize) -> Result<StubCpu, String> {
        if index != 0 {
            return Err("the Linux sandbox has one CPU".to_string());
        }
        let raw = sys::getregs(self.process.pid)?;
        Ok(StubCpu { process: self.process.clone(), raw, regs: Regs::X86(from_raw(&raw)), regs_dirty: false, fp: None, fp_dirty: false })
    }
}

fn host_prot(prot: Prot) -> i32 {
    let mut p = libc::PROT_NONE;
    if prot.read() {
        p |= libc::PROT_READ;
    }
    if prot.write() {
        p |= libc::PROT_WRITE;
    }
    if prot.exec() {
        p |= libc::PROT_EXEC;
    }
    p
}

/// x86's registers in encoding order: rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15.
fn from_raw(r: &sys::UserRegs) -> X86Regs {
    X86Regs {
        gpr: [r.rax, r.rcx, r.rdx, r.rbx, r.rsp, r.rbp, r.rsi, r.rdi, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15],
        rip: r.rip,
        rflags: r.eflags,
        fs_base: r.fs_base,
        gs_base: r.gs_base,
    }
}

/// Into the stub's registers, keeping its segment selectors as the kernel set them.
fn into_raw(x: &X86Regs, r: &mut sys::UserRegs) {
    let g = &x.gpr;
    (r.rax, r.rcx, r.rdx, r.rbx, r.rsp, r.rbp, r.rsi, r.rdi) = (g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7]);
    (r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15) = (g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]);
    r.rip = x.rip;
    r.eflags = x.rflags;
    r.fs_base = x.fs_base;
    r.gs_base = x.gs_base;
}

pub struct StubCpu {
    process: Arc<Process>,
    /// The stub's registers as `ptrace` last gave them.
    raw: sys::UserRegs,
    regs: Regs,
    regs_dirty: bool,
    /// Loaded when first asked for after a stop.
    fp: Option<FpRegs>,
    fp_dirty: bool,
}

impl StubCpu {
    fn pid(&self) -> i32 {
        self.process.pid
    }

    fn store(&mut self) -> Result<(), String> {
        if self.regs_dirty {
            if let Regs::X86(x) = &self.regs {
                into_raw(x, &mut self.raw);
            }
            sys::setregs(self.pid(), &self.raw)?;
            self.regs_dirty = false;
        }
        if self.fp_dirty {
            if let Some(FpRegs::X86(fp)) = &self.fp {
                // SAFETY: `user_fpregs_struct` is FXSAVE's 512-byte layout.
                let raw: sys::UserFpRegs = unsafe { std::mem::transmute_copy(&fp.fxsave) };
                sys::setfpregs(self.pid(), &raw)?;
            }
            self.fp_dirty = false;
        }
        Ok(())
    }

    fn load(&mut self) -> Result<(), String> {
        self.raw = sys::getregs(self.pid())?;
        self.regs = Regs::X86(from_raw(&self.raw));
        self.fp = None;
        Ok(())
    }

    fn rip(&self) -> u64 {
        self.raw.rip
    }

    /// What a stop on `signal` means, with the stub's registers loaded.
    fn exit_for(&mut self, signal: i32) -> SandboxExit {
        let pc = self.rip();
        let info = sys::getsiginfo(self.pid()).unwrap_or([0; 128]);
        let code = i32::from_le_bytes([info[8], info[9], info[10], info[11]]);
        let addr = u64::from_le_bytes(info[16..24].try_into().unwrap_or_default());
        let fault = |kind, addr, access| SandboxExit::Fault(HostFault { kind, addr, access, pc });
        match signal {
            PREEMPT_SIGNAL => SandboxExit::Preempted,
            libc::SIGSYS => {
                // Syscall User Dispatch: rip is past the call, rax its number.
                let arch = u32::from_le_bytes([info[28], info[29], info[30], info[31]]);
                SandboxExit::Gate(if arch == sys::AUDIT_ARCH_I386 { Trap::Int80 } else { Trap::Syscall })
            }
            libc::SIGSEGV => match sys::peek(self.pid(), pc).map(|w| w as u16) {
                // `int 0x2e` and `int 0x29` fault as general protection: their bytes at rip say which they were.
                Some(0x2ecd) => {
                    self.skip(2);
                    SandboxExit::Gate(Trap::Int2e)
                }
                Some(0x29cd) => {
                    let Regs::X86(x) = &self.regs else { return SandboxExit::Failed(0x4300) };
                    fault(FaultKind::FastFail, x.gpr[1], Prot::NONE)
                }
                _ if code == sys::SEGV_MAPERR || code == sys::SEGV_ACCERR => {
                    // A fetch faults at the instruction itself, unless its page is present and executable: then the
                    // fetch worked, and the instruction wrote to itself. A present page (ACCERR) is readable, so
                    // anything else that faults on one is a write.
                    let fetched = code == sys::SEGV_ACCERR && self.process.is_executable(pc & !0xfff);
                    let access = if addr == pc && !fetched {
                        Prot::X
                    } else if code == sys::SEGV_ACCERR {
                        Prot::W
                    } else {
                        Prot::R
                    };
                    fault(FaultKind::Segv, addr, access)
                }
                // A general protection fault: a privileged instruction, a non-canonical address.
                _ => fault(FaultKind::Segv, u64::MAX, Prot::R),
            },
            libc::SIGTRAP => SandboxExit::Fault(HostFault { kind: FaultKind::Trap, addr: 0, access: Prot::NONE, pc: pc.wrapping_sub(1) }),
            libc::SIGILL => fault(FaultKind::Ill, addr, Prot::NONE),
            libc::SIGFPE => fault(FaultKind::DivZero, addr, Prot::NONE),
            libc::SIGBUS => fault(FaultKind::Bus, addr, Prot::NONE),
            // Anything else from outside (SIGSTOP, SIGCONT, …) is swallowed, and the program carries on.
            _ => SandboxExit::Preempted,
        }
    }

    /// Moves rip past an instruction the kernel will answer.
    fn skip(&mut self, len: u64) {
        if let Regs::X86(x) = &mut self.regs {
            x.rip += len;
            self.regs_dirty = true;
        }
    }
}

impl SandboxCpu for StubCpu {
    fn regs(&mut self) -> &mut Regs {
        self.regs_dirty = true;
        &mut self.regs
    }

    fn fp(&mut self) -> &mut FpRegs {
        if self.fp.is_none() {
            let raw = sys::getfpregs(self.pid()).ok();
            let mut fp = X86Fp::default();
            if let Some(raw) = raw {
                // SAFETY: `user_fpregs_struct` is FXSAVE's 512-byte layout.
                fp.fxsave = unsafe { std::mem::transmute_copy(&raw) };
            }
            self.fp = Some(FpRegs::X86(fp));
        }
        self.fp_dirty = true;
        self.fp.get_or_insert(FpRegs::X86(X86Fp::default()))
    }

    fn resume(&mut self) -> SandboxExit {
        if let Err(_why) = self.store() {
            return SandboxExit::Failed(0x4400);
        }
        if sys::cont(self.pid(), 0).is_err() {
            return SandboxExit::Failed(0x4401);
        }
        let status = match sys::wait(self.pid()) {
            Ok(status) => status,
            Err(_) => return SandboxExit::Failed(0x4402),
        };
        if !libc::WIFSTOPPED(status) {
            // It ended: killed by its seccomp filter, or from outside.
            return SandboxExit::Failed(0x4500 | (status as u64 & 0xff));
        }
        if self.load().is_err() {
            return SandboxExit::Failed(0x4403);
        }
        self.exit_for(libc::WSTOPSIG(status))
    }

    fn preempt_handle(&self) -> RawPreempt {
        let pid = self.pid();
        Arc::new(move || sys::tgkill(pid, PREEMPT_SIGNAL))
    }
}

const _: () = assert!(size_of::<sys::UserFpRegs>() == 512);
