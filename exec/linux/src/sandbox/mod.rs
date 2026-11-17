//! `sandbox` on Linux x86-64: the program runs natively in stub processes, driven by `ptrace`, with no hypervisor,
//! root or driver.
//!
//! - **CPUs**: each CPU is a stub process of its own, started and traced by the host thread that drives the CPU. Each
//!   is an address space of its own, which shows its CPU's space, so a parent and the child it forked, or a program's
//!   threads, run at once (`MAX_CPUS`). A dropped CPU's slot is used again by the next thread, with a new stub.
//! - **Memory**: guest RAM is the memfd `ram.rs` makes, which every stub has at a fixed descriptor; core's mirror maps
//!   guest pages into a stub at the program's own addresses, in runs, so its addresses are real and its code runs as
//!   is. Every stub maps the same memory shared, so what one CPU writes, the others see.
//! - **System calls**: Syscall User Dispatch, with no part of the stub exempt, turns every `syscall` (and `int 0x80`)
//!   into a `SIGSYS`, which stops the stub for Carbon (`Reach::Dispatch`). Where the kernel has no dispatch (before
//!   5.11, or gVisor), `PTRACE_SYSEMU` stops the stub at each system call instead. atlas's `int 0x2e` is a general
//!   protection fault with `cd 2e` at rip, read as a call too.
//! - **Lockdown**: a stub has guest memory and nothing else, no privileges, a read-only dispatch selector, and a
//!   seccomp filter that kills it for any real system call but `mmap`, `mprotect`, `munmap` and exit from its own
//!   trampoline.
//! - **Faults** stop the stub as signals, which Carbon reads and swallows.
//! - **Registers**: the general ones, and x87, SSE and AVX's state through XSAVE's register set (FXSAVE's where the
//!   kernel has no XSAVE set).
#![cfg(target_arch = "x86_64")]

mod elf;
mod process;
mod seccomp;
mod sys;

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::ThreadId;

use exec_core::contract::{FaultKind, GuestRam, HostFault, MirrorRun, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::Trap;
use exec_core::isa::x86::{X86Fp, X86Regs};
use exec_core::{FpRegs, Isa, Layout, Options, Prot, Regs};

use crate::ram;
use process::{Catch, PREEMPT_SIGNAL, Process};

/// The most CPUs a machine's sandbox has at once. Each is a host thread and a stub process, so this bounds what a
/// program's threads and processes cost the host, not anything about the stubs themselves.
const MAX_CPUS: usize = 256;

/// XSAVE's standard layout: the legacy area, then the header with its feature bitmap, then AVX's component.
const XSAVE_BV: usize = 512;
const XSAVE_AVX: usize = 576;
const XSAVE_AVX_END: usize = XSAVE_AVX + 256;
/// In the legacy area, what the kernel leaves for software: the features this CPU's XSAVE has, at 472.
const XSAVE_FEATURES: usize = 472;
const FEATURE_X87_SSE: u64 = 0b011;
const FEATURE_AVX: u64 = 0b100;

pub struct Stub {
    /// Guest memory's memfd, which each new stub is given.
    ram_fd: i32,
    /// Where every stub is, and how each catches its program's calls: as the first one found, so they're all alike.
    base: u64,
    catch: Catch,
    /// The first CPU's stub, started by `open` on the thread that will drive it, so a sandbox that can't start here
    /// says why at once.
    first: Mutex<Option<(ThreadId, Process)>>,
}

impl Sandbox for Stub {
    type Cpu = StubCpu;
    const NAME: &'static str = "linux";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Stub, String> {
        let ram_fd = ram.handle as i32;
        let process = Process::spawn(ram_fd, None, None)?;
        let (base, catch) = (process.at.base, process.catch);
        Ok(Stub { ram_fd, base, catch, first: Mutex::new(Some((std::thread::current().id(), process))) })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Dispatch
    }

    fn layout(&self, isa: Isa) -> Layout {
        Layout { top: exec_core::isa::space_top(isa).min(self.base), kept: Vec::new(), gate: None }
    }

    fn max_cpus(&self) -> usize {
        MAX_CPUS
    }

    fn space_per_cpu(&self) -> bool {
        true
    }

    fn new_cpu(&self, _index: usize) -> Result<StubCpu, String> {
        let mut first = self.first.lock().unwrap_or_else(PoisonError::into_inner);
        let process = match first.take() {
            Some((thread, process)) if thread == std::thread::current().id() => process,
            // One started on another thread, which `ptrace` won't let this one drive, goes; this thread starts its own.
            _ => {
                drop(first);
                Process::spawn(self.ram_fd, Some(self.base), Some(self.catch))?
            }
        };
        let raw = sys::getregs(process.pid)?;
        Ok(StubCpu { process, raw, regs: Regs::X86(from_raw(&raw)), regs_dirty: false, fp: None, fp_dirty: false, xstate: Vec::new() })
    }
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

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

pub struct StubCpu {
    process: Process,
    /// The stub's registers as `ptrace` last gave them.
    raw: sys::UserRegs,
    regs: Regs,
    regs_dirty: bool,
    /// Loaded when first asked for after a stop.
    fp: Option<FpRegs>,
    fp_dirty: bool,
    /// The stub's whole XSAVE state as last read, which writing the FP registers goes back over (keeping what core
    /// doesn't hold); empty where the kernel has no XSAVE register set.
    xstate: Vec<u8>,
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
            if self.process.catch == Catch::Sysemu {
                // Not in a system call, as far as the kernel's concerned: a preempt that comes as it returns from an
                // emulated one mustn't restart it, whatever its result.
                self.raw.orig_rax = u64::MAX;
            }
            sys::setregs(self.pid(), &self.raw)?;
            self.regs_dirty = false;
        }
        if self.fp_dirty {
            if let Some(FpRegs::X86(fp)) = &self.fp {
                if self.xstate.len() >= XSAVE_AVX {
                    let x = &mut self.xstate;
                    // The legacy area but for what the kernel keeps for software at its end.
                    x[..464].copy_from_slice(&fp.fxsave[..464]);
                    let mut bv = word(x, XSAVE_BV) | FEATURE_X87_SSE;
                    if word(x, XSAVE_FEATURES) & FEATURE_AVX != 0 && x.len() >= XSAVE_AVX_END {
                        x[XSAVE_AVX..XSAVE_AVX_END].copy_from_slice(&fp.ymmh);
                        bv |= FEATURE_AVX;
                    }
                    x[XSAVE_BV..XSAVE_BV + 8].copy_from_slice(&bv.to_le_bytes());
                    sys::setxstate(self.process.pid, x)?;
                } else {
                    // SAFETY: `user_fpregs_struct` is FXSAVE's 512-byte layout.
                    let raw: sys::UserFpRegs = unsafe { std::mem::transmute_copy(&fp.fxsave) };
                    sys::setfpregs(self.pid(), &raw)?;
                }
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

    /// The FP state: XSAVE's, with AVX's upper halves, or FXSAVE's where the kernel has no XSAVE register set.
    fn load_fp(&mut self) -> X86Fp {
        let mut fp = X86Fp::default();
        let mut buffer = std::mem::take(&mut self.xstate);
        buffer.resize(8192, 0);
        match sys::getxstate(self.pid(), &mut buffer) {
            Ok(len) if len >= XSAVE_AVX => {
                buffer.truncate(len);
                fp.fxsave.copy_from_slice(&buffer[..512]);
                if word(&buffer, XSAVE_FEATURES) & FEATURE_AVX != 0 && len >= XSAVE_AVX_END {
                    fp.ymmh.copy_from_slice(&buffer[XSAVE_AVX..XSAVE_AVX_END]);
                }
                self.xstate = buffer;
            }
            _ => {
                if let Ok(raw) = sys::getfpregs(self.pid()) {
                    // SAFETY: `user_fpregs_struct` is FXSAVE's 512-byte layout.
                    fp.fxsave = unsafe { std::mem::transmute_copy(&raw) };
                }
            }
        }
        fp
    }

    fn rip(&self) -> u64 {
        self.raw.rip
    }

    /// What a stop on `signal` means, with the stub's registers loaded.
    fn exit_for(&mut self, signal: i32) -> SandboxExit {
        let pc = self.rip();
        match signal {
            // PTRACE_SYSEMU: rip is past the call, orig_rax its number, and rax the kernel's -ENOSYS.
            sys::SYSCALL_STOP => {
                if let Regs::X86(x) = &mut self.regs {
                    x.gpr[0] = self.raw.orig_rax;
                    self.regs_dirty = true;
                }
                let int80 = sys::peek(self.pid(), pc.wrapping_sub(2)).map(|w| w as u16) == Some(0x80cd);
                return SandboxExit::Gate(if int80 { Trap::Int80 } else { Trap::Syscall });
            }
            PREEMPT_SIGNAL => return SandboxExit::Preempted,
            libc::SIGTRAP => return SandboxExit::Fault(HostFault { kind: FaultKind::Trap, addr: 0, access: Prot::NONE, pc: pc.wrapping_sub(1) }),
            libc::SIGSYS | libc::SIGSEGV | libc::SIGILL | libc::SIGFPE | libc::SIGBUS => {}
            // Anything else from outside (SIGSTOP, SIGCONT, …) is swallowed, and the program carries on.
            _ => return SandboxExit::Preempted,
        }
        let info = sys::getsiginfo(self.pid()).unwrap_or([0; 128]);
        let code = i32::from_le_bytes([info[8], info[9], info[10], info[11]]);
        let addr = u64::from_le_bytes(info[16..24].try_into().unwrap_or_default());
        let fault = |kind, addr, access| SandboxExit::Fault(HostFault { kind, addr, access, pc });
        match signal {
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
                    // An instruction that runs onto the next page, which the stub doesn't have executable: its
                    // fetch, which gVisor (Cloud Run's first generation) reports as an access error even where the
                    // page isn't there at all.
                    let straddles = addr > pc && addr - pc < 15 && addr & !0xfff != pc & !0xfff && !self.process.is_executable(addr & !0xfff);
                    let access = if (addr == pc && !fetched) || straddles {
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
            libc::SIGILL => fault(FaultKind::Ill, addr, Prot::NONE),
            libc::SIGFPE => fault(FaultKind::DivZero, addr, Prot::NONE),
            _ => fault(FaultKind::Bus, addr, Prot::NONE),
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
            let fp = self.load_fp();
            self.fp = Some(FpRegs::X86(fp));
        }
        self.fp_dirty = true;
        self.fp.get_or_insert(FpRegs::X86(X86Fp::default()))
    }

    fn resume(&mut self) -> SandboxExit {
        loop {
            if let Err(_why) = self.store() {
                return SandboxExit::Failed(0x4400);
            }
            let resumed = match self.process.catch {
                Catch::Dispatch => sys::cont(self.pid(), 0),
                Catch::Sysemu => sys::sysemu(self.pid()),
            };
            if resumed.is_err() {
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
            let signal = libc::WSTOPSIG(status);
            if self.process.catch == Catch::Sysemu && signal == sys::SYSCALL_STOP && self.raw.orig_rax == u64::MAX {
                // gVisor, after a call it handed over as a seccomp event (which `store` told it to skip), stops for
                // `PTRACE_SYSEMU` too, at a call that's no call any more: nothing for the kernel.
                continue;
            }
            if signal == libc::SIGTRAP && status >> 16 == sys::PTRACE_EVENT_SECCOMP {
                // gVisor asks seccomp before `PTRACE_SYSEMU`: the program's call, stopped as an event, as it would
                // have been stopped for emulation.
                return self.exit_for(sys::SYSCALL_STOP);
            }
            return self.exit_for(signal);
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let pid = self.pid();
        Arc::new(move || sys::tgkill(pid, PREEMPT_SIGNAL))
    }

    fn mirror(&mut self, runs: &[MirrorRun]) -> Result<(), String> {
        self.process.mirror(runs)
    }

    /// The stub's CPU time, from its process's CPU-time clock: it has one thread, its program's.
    fn program_time(&self) -> Option<u64> {
        // MAKE_PROCESS_CPUCLOCK(pid, CPUCLOCK_SCHED)
        let clock = (!self.pid() << 3) | 2;
        // SAFETY: an out-parameter.
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        (unsafe { libc::clock_gettime(clock, &mut time) } == 0).then(|| time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64)
    }
}

const _: () = assert!(size_of::<sys::UserFpRegs>() == 512);
