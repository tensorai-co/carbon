//! The whole contract a backend answers. Each host crate implements one `Vm` and one `Sandbox`, and nothing else:
//! spaces, frames, copy-on-write, statistics, time slices and what a stop means are all core's.

use std::sync::Arc;

use exec_mmu::Prot;

use crate::isa::Trap;
use crate::shim::Shim;
use crate::{FpRegs, Isa, Layout, Options, Regs};

/// Makes a CPU's `run` return soon. Any thread may call it.
pub type RawPreempt = Arc<dyn Fn() + Send + Sync>;

/// Guest memory: one host mapping, which the host crate makes and frees.
pub struct GuestRam {
    pub ptr: *mut u8,
    pub size: u64,
    /// Bytes from one frame to the next: a page, unless the host can only map larger pieces into a sandbox's stub.
    pub stride: u64,
    /// The host's own page size, which is the least that can be given back.
    pub granule: u64,
    /// The host's handle for the memory behind the mapping (a memfd, a section), for a sandbox to map into its stub,
    /// or -1.
    pub handle: i64,
    /// Gives `len` bytes at a pointer back to the host, which reads as zeroes or garbage afterwards. False if it can't.
    pub discard: fn(*mut u8, usize) -> bool,
    /// Frees the mapping and the handle.
    pub release: fn(*mut u8, u64, i64),
}

// SAFETY: the mapping belongs to whoever holds the GuestRam, and is freed once, on drop.
unsafe impl Send for GuestRam {}

impl Drop for GuestRam {
    fn drop(&mut self) {
        (self.release)(self.ptr, self.size, self.handle);
    }
}

/// Why a vm backend's CPU came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmExit {
    /// The shim stopped it: on x86, the address after its halt; on ARM64, the hypercall's number.
    Trap { at: u64 },
    /// Something preempted it.
    Preempted,
    /// The hypervisor failed, with its own code.
    Failed(u64),
}

/// How a sandbox's programs reach the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The host sends their system calls to the stub (Linux's Syscall User Dispatch).
    Dispatch,
    /// They call the gate, which is a breakpoint (Windows).
    Breakpoint,
    /// Their code is rewritten so system calls go to the gate (macOS).
    Rewrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// A bad memory access.
    Segv,
    /// An instruction that doesn't exist or isn't allowed.
    Ill,
    /// A breakpoint.
    Trap,
    /// A misaligned access.
    Bus,
    DivZero,
    /// x86's `int 0x29`, Windows' way to end at once; `HostFault::addr` holds its code.
    FastFail,
}

/// A fault in a sandbox's stub, however the host reported it: a signal, a debug event, a Mach exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostFault {
    pub kind: FaultKind,
    pub addr: u64,
    pub access: Prot,
    pub pc: u64,
}

/// Why a sandbox's CPU came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxExit {
    /// A system call reached the gate, by this kind of trap.
    Gate(Trap),
    Fault(HostFault),
    Preempted,
    Failed(u64),
}

/// A vm backend. Guest page tables live in guest RAM and the hardware walks them. The program runs at ring 3 or EL0;
/// every exception goes into the shim, which halts or makes a hypercall, and core works out what happened.
pub trait Vm: Sized + Send + 'static {
    type Shim: Shim;
    type Cpu: VmCpu<Shim = Self::Shim>;
    const NAME: &'static str;
    /// Where guest RAM starts in guest physical memory.
    const RAM_BASE: u64;

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String>;
    fn open(ram: &GuestRam, options: &Options) -> Result<Self, String>;
    fn max_cpus(&self) -> usize;
    /// Whether x86 memory ordering can be switched on.
    fn tso(&self) -> bool {
        false
    }
    /// Code was written at this host address, so instruction caches must forget it.
    fn sync_code(&self, _host: *mut u8, _len: usize) {}
    /// A CPU, which belongs to the calling thread.
    fn new_cpu(&self, index: usize) -> Result<Self::Cpu, String>;
}

pub trait VmCpu: 'static {
    type Shim: Shim;
    /// Once, from the shim: GDT/IDT/TSS/MSRs, or SCTLR/TCR/MAIR/VBAR.
    fn set_system(&mut self, system: &<Self::Shim as Shim>::System) -> Result<(), String>;
    /// The CPU's registers. Changes reach the CPU before it next runs.
    fn regs(&mut self) -> &mut <Self::Shim as Shim>::Regs;
    fn fp(&mut self) -> &mut <Self::Shim as Shim>::Fp;
    /// Which tables it walks: the value for cr3 or TTBR0, from `Shim::root_register`.
    fn set_root(&mut self, value: u64);
    fn run(&mut self) -> VmExit;
    fn preempt_handle(&self) -> RawPreempt;
}

/// A run of pages a CPU's stub is told to show, for a sandbox whose CPUs each have an address space of their own
/// (`Sandbox::space_per_cpu`). Core merges what a turn changed into as few runs as it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorRun {
    /// `len` bytes from `page` show guest RAM from `offset` on, page for page, with `prot`.
    Map { page: u64, len: u64, offset: u64, prot: Prot },
    /// `len` bytes from `page`, already showing guest RAM, now with `prot`.
    Protect { page: u64, len: u64, prot: Prot },
    /// `len` bytes from `page` show nothing.
    Unmap { page: u64, len: u64 },
}

/// A sandbox backend. Program code runs natively in a stub process; core keeps the page tables and mirrors them into
/// the stub.
///
/// A stub is one of two kinds. Either every CPU shares one address space, which shows one space at a time and which
/// `Sandbox::mirror` changes a page at a time (such a sandbox has one CPU); or each CPU's stub is an address space of
/// its own (`space_per_cpu`), which shows that CPU's space, so CPUs in different spaces run at once, and
/// `SandboxCpu::mirror` changes it, in runs, from that CPU's own thread.
pub trait Sandbox: Sized + Send + 'static {
    type Cpu: SandboxCpu;
    const NAME: &'static str;

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String>;
    fn open(ram: &GuestRam, options: &Options) -> Result<Self, String>;
    fn isas(&self) -> &'static [Isa];
    fn reach(&self) -> Reach;
    fn layout(&self, isa: Isa) -> Layout;
    fn max_cpus(&self) -> usize;
    /// Whether each CPU's stub is an address space of its own. A CPU's slot can then be used again by another thread,
    /// which makes a stub of its own.
    fn space_per_cpu(&self) -> bool {
        false
    }
    /// Makes the shared stub's page at `page` show guest RAM at `ram_offset` with `prot`, or nothing. Only for a
    /// sandbox without a space per CPU.
    fn mirror(&mut self, _page: u64, _ram_offset: Option<u64>, _prot: Prot) -> Result<(), String> {
        Err("this sandbox's stubs are mirrored per CPU".to_string())
    }
    /// Where the next thread's environment block must go, on a host that decides that itself.
    fn thread_block(&mut self, _size: u64) -> Option<u64> {
        None
    }
    /// A stub thread, which belongs to the calling thread.
    fn new_cpu(&self, index: usize) -> Result<Self::Cpu, String>;
}

pub trait SandboxCpu: 'static {
    /// The program's registers: in a sandbox the program runs on the host's own CPU, so they're its own.
    fn regs(&mut self) -> &mut Regs;
    fn fp(&mut self) -> &mut FpRegs;
    fn resume(&mut self) -> SandboxExit;
    fn preempt_handle(&self) -> RawPreempt;
    /// Makes this CPU's own stub show `runs`, in order. Only for a sandbox with a space per CPU; called on the CPU's
    /// own thread, while it isn't running.
    fn mirror(&mut self, _runs: &[MirrorRun]) -> Result<(), String> {
        Err("this sandbox's CPUs share one stub".to_string())
    }
    /// The host CPU time this CPU's program has had, in nanoseconds, where it runs in a process of its own (so the
    /// calling thread's own CPU time doesn't count it). None if the host can't say.
    fn program_time(&self) -> Option<u64> {
        None
    }
}
