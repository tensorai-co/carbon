//! `sandbox` on Linux x86-64: the program runs natively in stub processes that handle their own traps, with no
//! hypervisor, root or driver.
//!
//! - **CPUs**: each CPU is a stub process of its own, an address space showing its CPU's space, so a parent and the
//!   child it forked, or a program's threads, run at once (`MAX_CPUS`). A dropped CPU's slot is used again by the next
//!   thread, with a new stub.
//! - **Memory**: guest RAM is the memfd `ram.rs` makes, which every stub maps shared; core's mirror maps guest pages
//!   into a stub at the program's own addresses, in runs, lazily, so its addresses are real and its code runs as is.
//! - **The gate**: atlas's ntdll calls a gate in the stub instead of making a `syscall` (`Layout::gate`), which saves
//!   the signal and `rt_sigreturn`: the gate saves the program's registers in a frame laid out as a signal's, and the
//!   rest is the same. Linux programs make their own `syscall`s, so they take the signal.
//! - **Traps**: a program's system calls (Syscall User Dispatch, or seccomp's trap where there's none: gVisor, Linux
//!   before 5.11), faults and preempts are signals to a handler in its own stub (`handler.rs`), on a signal stack in
//!   memory Carbon maps too. Carbon reads and changes the program's registers in that signal frame, and the two meet
//!   through a mailbox there with a futex (`mailbox.rs`), each spinning first while the other's on another CPU and
//!   answers quickly. The stub sets itself up, from calls Carbon leaves in the mailbox, so no `ptrace` at all.
//! - **Mapping**: Linux can't map into another process, so the stub maps guest memory into itself: its handler, when
//!   Carbon asks (`fast`), or Carbon through `ptrace`, with the stub stopped (`strict`, `CARBON_SANDBOX=strict`).
//! - **Lockdown**: a stub has guest memory and nothing else, no privileges, and a seccomp filter (`seccomp.rs`) that
//!   allows real system calls only from its handler's page, and only those the handler makes.
//! - **Registers**: the general ones from the frame, `fs` and `gs` through the handler (`rdfsbase` and the rest where
//!   the kernel allows them; else only Carbon can change them, so it keeps them, and the handler sets them with
//!   `arch_prctl`), and x87, SSE and AVX's state from the frame's XSAVE area.
#![cfg(target_arch = "x86_64")]

mod elf;
mod handler;
mod mailbox;
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
use handler::uc;
use mailbox::Gone;
use process::{Catch, Mode, PREEMPT_SIGNAL, Process};

/// The most CPUs a machine's sandbox has at once. Each is a host thread and a stub process, so this bounds what a
/// program's threads and processes cost the host, not anything about the stubs themselves.
const MAX_CPUS: usize = 256;

/// XSAVE's standard layout, as a signal frame has it: the legacy area, then the header with its feature bitmap, then
/// AVX's component. In the legacy area, what the kernel leaves for software: a magic number, the whole area's size,
/// and this CPU's features.
const XSAVE_BV: usize = 512;
const XSAVE_AVX: usize = 576;
const XSAVE_AVX_END: usize = XSAVE_AVX + 256;
const SW_MAGIC: usize = 464;
const SW_EXTENDED_SIZE: usize = 468;
const SW_FEATURES: usize = 472;
const FP_XSTATE_MAGIC1: u32 = 0x4650_5853;
const FEATURE_X87_SSE: u64 = 0b011;
const FEATURE_AVX: u64 = 0b100;

pub struct Stub {
    /// Guest memory's memfd, which each new stub is given.
    ram_fd: i32,
    /// Where every stub is, how each catches its program's calls, and who maps: as the first one found, so they're
    /// all alike.
    base: u64,
    catch: Catch,
    mode: Mode,
    /// Programs may call the stub's gate rather than trap (`handler.rs`): where XSAVE is on, unless
    /// `CARBON_SANDBOX_GATE=0`.
    gate: bool,
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
        let process = Process::spawn(ram_fd, None, None, None)?;
        let (base, catch, mode) = (process.at.base, process.catch, process.mode);
        let xsave = std::arch::x86_64::__cpuid(1).ecx & (1 << 27) != 0;
        let gate = xsave && std::env::var("CARBON_SANDBOX_GATE").as_deref() != Ok("0");
        Ok(Stub { ram_fd, base, catch, mode, gate, first: Mutex::new(Some((std::thread::current().id(), process))) })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Dispatch
    }

    fn layout(&self, isa: Isa) -> Layout {
        let gate = (isa == Isa::X86_64 && self.gate).then(|| self.base + handler::gate_at());
        Layout { top: exec_core::isa::space_top(isa).min(self.base), kept: Vec::new(), gate, traps: true }
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
            // One started on another thread (whose death would end it) goes; this thread starts its own.
            _ => {
                drop(first);
                Process::spawn(self.ram_fd, Some(self.base), Some(self.catch), Some(self.mode))?
            }
        };
        StubCpu::new(process).map_err(|gone| format!("a new CPU's stub: {gone:?}"))
    }
}

/// The event the stub is waiting in its handler with: where its signal frame is in the shared memory.
#[derive(Default)]
struct Frame {
    signo: i32,
    /// It came by the gate: `uc` is the gate's frame, and there's no siginfo.
    gate: bool,
    info: usize,
    uc: usize,
    /// The FP state's offset and length (512, or XSAVE's whole area).
    fpstate: Option<(usize, usize)>,
}

pub struct StubCpu {
    process: Process,
    frame: Frame,
    /// `fs` and `gs`: as the handler read them at the event, or, where it can't (`Process::fsgsbase`), as Carbon last
    /// set them, which then only Carbon can change.
    bases: (u64, u64),
    regs: Regs,
    regs_dirty: bool,
    /// Loaded when first asked for after a stop.
    fp: Option<FpRegs>,
    fp_dirty: bool,
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default())
}

/// x86's registers in encoding order (rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15), where the frame has them.
const GPRS: [usize; 16] = [uc::RAX, uc::RCX, uc::RDX, uc::RBX, uc::RSP, uc::RBP, uc::RSI, uc::RDI, uc::R8, uc::R9, uc::R10, uc::R11, uc::R12, uc::R13, uc::R14, uc::R15];

impl StubCpu {
    fn new(process: Process) -> Result<StubCpu, Gone> {
        let mut cpu = StubCpu { process, frame: Frame::default(), bases: (0, 0), regs: Regs::X86(X86Regs::default()), regs_dirty: false, fp: None, fp_dirty: false };
        cpu.load()?;
        Ok(cpu)
    }

    fn pid(&self) -> i32 {
        self.process.pid
    }

    /// Reads the event the stub's waiting with: its signal, and the program's registers from its frame.
    fn load(&mut self) -> Result<(), Gone> {
        let mailbox = &self.process.mailbox;
        let signo = mailbox.get(handler::SIGNO) as i32;
        let gate = mailbox.get(handler::VIA_GATE) as u32 != 0;
        let (info, uc) = if gate {
            // The gate's frame, always in the same place, and no siginfo.
            if mailbox.get(handler::UC) != mailbox.stub_at + handler::GATE_UC - handler::SHARED {
                return Err(Gone::Garbled("the gate's frame isn't where it goes".to_string()));
            }
            (None, (handler::GATE_UC - handler::SHARED) as usize)
        } else {
            (Some(mailbox.on_stack(mailbox.get(handler::INFO), 128)?), mailbox.on_stack(mailbox.get(handler::UC), uc::SIZE as u64)?)
        };
        let frame = mailbox.bytes(uc, uc::SIZE);
        let fpstate = match read_u64(frame, uc::FPSTATE) {
            0 => None,
            at => {
                let legacy = mailbox.frame_part(at, 512, gate)?;
                let fx = mailbox.bytes(legacy, 512);
                let len = if read_u32(fx, SW_MAGIC) == FP_XSTATE_MAGIC1 { u64::from(read_u32(fx, SW_EXTENDED_SIZE)).max(512) } else { 512 };
                Some((mailbox.frame_part(at, len, gate)?, len as usize))
            }
        };
        let mut x = X86Regs { rip: read_u64(frame, uc::RIP), rflags: read_u64(frame, uc::EFLAGS), ..X86Regs::default() };
        for (gpr, &at) in x.gpr.iter_mut().zip(GPRS.iter()) {
            *gpr = read_u64(frame, at);
        }
        if let (libc::SIGSYS, Some(info)) = (signo, info) {
            // The call's number, from what the host kernel reports: dispatch leaves it in rax too, seccomp's trap may not.
            x.gpr[0] = u64::from(read_u32(mailbox.bytes(info, 128), 24));
        }
        if self.process.fsgsbase {
            self.bases = (mailbox.get(handler::FS_BASE), mailbox.get(handler::GS_BASE));
        }
        (x.fs_base, x.gs_base) = self.bases;
        self.frame = Frame { signo, gate, info: info.unwrap_or(0), uc, fpstate };
        self.regs = Regs::X86(x);
        self.regs_dirty = false;
        self.fp = None;
        self.fp_dirty = false;
        Ok(())
    }

    /// Puts what the kernel changed back in the frame, and queues the calls the handler makes as it returns: the
    /// mappings `mirror` left it, and those that set `fs` and `gs` if they changed.
    fn store(&mut self) {
        let mut calls = self.process.take_pending();
        let mailbox = &mut self.process.mailbox;
        if self.regs_dirty
            && let Regs::X86(x) = &self.regs
        {
            let frame = mailbox.bytes_mut(self.frame.uc, uc::SIZE);
            for (gpr, &at) in x.gpr.iter().zip(GPRS.iter()) {
                frame[at..at + 8].copy_from_slice(&gpr.to_le_bytes());
            }
            frame[uc::RIP..uc::RIP + 8].copy_from_slice(&x.rip.to_le_bytes());
            frame[uc::EFLAGS..uc::EFLAGS + 8].copy_from_slice(&x.rflags.to_le_bytes());
            if self.process.fsgsbase {
                // The handler sets both from the mailbox as it returns.
                mailbox.set(handler::FS_BASE, x.fs_base);
                mailbox.set(handler::GS_BASE, x.gs_base);
            } else {
                if x.fs_base != self.bases.0 {
                    calls.push((libc::SYS_arch_prctl, [0x1002, x.fs_base, 0, 0, 0, 0]));
                }
                if x.gs_base != self.bases.1 {
                    calls.push((libc::SYS_arch_prctl, [0x1001, x.gs_base, 0, 0, 0, 0]));
                }
            }
            self.bases = (x.fs_base, x.gs_base);
        }
        if self.fp_dirty
            && let (Some(FpRegs::X86(fp)), Some((at, len))) = (&self.fp, self.frame.fpstate)
        {
            let x = mailbox.bytes_mut(at, len);
            // The legacy area but for what the kernel keeps for software at its end.
            x[..SW_MAGIC].copy_from_slice(&fp.fxsave[..SW_MAGIC]);
            if len >= XSAVE_AVX {
                let mut bv = read_u64(x, XSAVE_BV) | FEATURE_X87_SSE;
                if read_u64(x, SW_FEATURES) & FEATURE_AVX != 0 && len >= XSAVE_AVX_END {
                    x[XSAVE_AVX..XSAVE_AVX_END].copy_from_slice(&fp.ymmh);
                    bv |= FEATURE_AVX;
                }
                x[XSAVE_BV..XSAVE_BV + 8].copy_from_slice(&bv.to_le_bytes());
            }
        }
        mailbox.queue(&calls);
        self.regs_dirty = false;
        self.fp_dirty = false;
    }

    /// The FP state from the frame: XSAVE's, with AVX's upper halves, or FXSAVE's.
    fn load_fp(&self) -> X86Fp {
        let mut fp = X86Fp::default();
        if let Some((at, len)) = self.frame.fpstate {
            let x = self.process.mailbox.bytes(at, len);
            fp.fxsave.copy_from_slice(&x[..512]);
            // AVX's upper halves, where XSAVE wrote them: where XSTATE_BV says they're in their first state, zeros.
            if len >= XSAVE_AVX_END && read_u64(x, SW_FEATURES) & FEATURE_AVX != 0 && read_u64(x, XSAVE_BV) & FEATURE_AVX != 0 {
                fp.ymmh.copy_from_slice(&x[XSAVE_AVX..XSAVE_AVX_END]);
            }
        }
        fp
    }

    fn info(&self) -> &[u8] {
        self.process.mailbox.bytes(self.frame.info, 128)
    }

    /// What the event means to core.
    fn exit(&mut self) -> SandboxExit {
        let Regs::X86(x) = &self.regs else { return SandboxExit::Failed(0x4300) };
        let (pc, rcx) = (x.rip, x.gpr[1]);
        let info = self.info();
        let code = read_u32(info, 8) as i32;
        let addr = read_u64(info, 16);
        let fault = |kind, addr, access| SandboxExit::Fault(HostFault { kind, addr, access, pc });
        // A call by the gate, as the Windows sandbox's gate reports one: atlas reads its arguments where its own
        // convention puts them either way.
        if self.frame.gate {
            return SandboxExit::Gate(Trap::Int2e);
        }
        match self.frame.signo {
            PREEMPT_SIGNAL => SandboxExit::Preempted,
            libc::SIGSYS => {
                let arch = read_u32(info, 28);
                SandboxExit::Gate(if arch == sys::AUDIT_ARCH_I386 { Trap::Int80 } else { Trap::Syscall })
            }
            libc::SIGSEGV if code == sys::SI_KERNEL => match self.process.mailbox.get(handler::CODE) as u16 {
                // `int 0x2e` and `int 0x29` fault as general protection: their bytes at rip say which they were.
                0x2ecd => {
                    if let Regs::X86(x) = &mut self.regs {
                        x.rip += 2;
                        self.regs_dirty = true;
                    }
                    SandboxExit::Gate(Trap::Int2e)
                }
                0x29cd => fault(FaultKind::FastFail, rcx, Prot::NONE),
                // A privileged instruction, a non-canonical address.
                _ => fault(FaultKind::Segv, u64::MAX, Prot::R),
            },
            libc::SIGSEGV => {
                // A fetch faults at the instruction itself, unless its page is present and executable: then the fetch
                // worked, and the instruction wrote to itself. A present page (ACCERR) is readable, so anything else
                // that faults on one is a write.
                let fetched = code == sys::SEGV_ACCERR && self.process.is_executable(pc & !0xfff);
                // An instruction that runs onto the next page, which the stub doesn't have executable: its fetch, which
                // gVisor reports as an access error even where the page isn't there at all.
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
            libc::SIGTRAP => SandboxExit::Fault(HostFault { kind: FaultKind::Trap, addr: 0, access: Prot::NONE, pc: pc.wrapping_sub(1) }),
            libc::SIGILL => fault(FaultKind::Ill, addr, Prot::NONE),
            libc::SIGFPE => fault(FaultKind::DivZero, addr, Prot::NONE),
            libc::SIGBUS => fault(FaultKind::Bus, addr, Prot::NONE),
            _ => SandboxExit::Preempted,
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
            self.fp = Some(FpRegs::X86(self.load_fp()));
        }
        self.fp_dirty = true;
        self.fp.get_or_insert(FpRegs::X86(X86Fp::default()))
    }

    fn resume(&mut self) -> SandboxExit {
        self.store();
        let mailbox = &self.process.mailbox;
        mailbox.tell(handler::RESUME);
        match mailbox.wait(handler::EVENT, self.process.pid) {
            Ok(()) => {}
            // It ended: killed by its seccomp filter, or from outside.
            Err(Gone::Ended(status)) => return SandboxExit::Failed(0x4500 | (status as u64 & 0xff)),
            Err(Gone::Garbled(_)) => return SandboxExit::Failed(0x4600),
        }
        if let Err(why) = self.process.checked() {
            eprintln!("carbon: sandbox: {why}");
            return SandboxExit::Failed(0x4602);
        }
        if self.load().is_err() {
            return SandboxExit::Failed(0x4601);
        }
        self.exit()
    }

    fn preempt_handle(&self) -> RawPreempt {
        let pid = self.pid();
        Arc::new(move || sys::tgkill(pid, PREEMPT_SIGNAL))
    }

    fn mirror(&mut self, runs: &[MirrorRun]) -> Result<(), String> {
        self.process.mirror(runs)
    }

    /// The stub's CPU time, from its process's CPU-time clock: it has one thread, its program's (and its handler's).
    fn program_time(&self) -> Option<u64> {
        // MAKE_PROCESS_CPUCLOCK(pid, CPUCLOCK_SCHED)
        let clock = (!self.pid() << 3) | 2;
        // SAFETY: an out-parameter.
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: as above.
        (unsafe { libc::clock_gettime(clock, &mut time) } == 0).then(|| time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64)
    }
}
