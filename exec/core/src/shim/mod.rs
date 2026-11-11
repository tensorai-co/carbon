//! What a vm backend puts in the guest, per CPU type: a few pages of code and tables above the program, which catch
//! every exception and hand it to the host. A shim knows the guest's layout and how to read a stopped program's state
//! from it; it doesn't know which hypervisor runs it, so kvm and winkvm share one, and so would every ARM64 hypervisor.

pub mod arm64;
pub mod x86;

use exec_mmu::{Frame, Mmu, Native, OutOfMemory};

use crate::shim::arm64::Arm64VcpuRegs;
use crate::{FpRegs, Isa, Options, Regs, Stop};

pub trait Shim: Send + Sync + Sized + 'static {
    /// What programs run natively on it.
    const ISAS: &'static [Isa];
    /// The most CPUs its layout has room for.
    const MAX_CPUS: usize;
    /// The vCPU's registers, as the backend moves them.
    type Regs: Clone + Default + std::fmt::Debug + Send;
    type Fp: Clone + Default + Send;
    /// What a CPU's system registers are set to, once.
    type System: Clone + std::fmt::Debug;
    /// What it keeps about each CPU.
    type CpuData: Clone + Send;

    /// Lays out its pages in guest memory.
    fn install(mmu: &mut Mmu<Native>, options: &Options) -> Result<Self, OutOfMemory>;
    /// Frames holding code, for instruction caches.
    fn code_frames(&self) -> Vec<Frame>;
    /// Another CPU's pages, system registers and first registers.
    fn add_cpu(&self, mmu: &mut Mmu<Native>, index: usize) -> Result<CpuSetup<Self>, OutOfMemory>;
    /// What goes in cr3 or TTBR0 for a space, or for none (0).
    fn root_register(&self, root: u64) -> u64;

    /// Just before running: if `flush`, go back to the program by way of a TLB flush.
    fn before_run(&self, regs: &mut Self::Regs, flush: bool);
    /// The shim stopped the CPU at `at`. Leaves it ready to go back to the program.
    fn trap(&self, mmu: &Mmu<Native>, cpu: &Self::CpuData, regs: &mut Self::Regs, at: u64) -> Stop;
    /// It was preempted, wherever it was. Leaves it ready to go back to the program.
    fn preempted(&self, mmu: &Mmu<Native>, cpu: &Self::CpuData, regs: &mut Self::Regs) -> Stop;

    /// The program's registers, while it's stopped.
    fn program(&self, mmu: &Mmu<Native>, cpu: &Self::CpuData, regs: &Self::Regs) -> Regs;
    fn set_program(&self, mmu: &Mmu<Native>, cpu: &Self::CpuData, regs: &mut Self::Regs, program: &Regs);
    /// A new thread at `entry` with `stack`, every other register cleared.
    fn start(&self, mmu: &Mmu<Native>, cpu: &Self::CpuData, regs: &mut Self::Regs, isa: Isa, entry: u64, stack: u64);
    fn set_return(&self, regs: &mut Self::Regs, value: u64);
    fn program_fp(fp: &Self::Fp) -> FpRegs;
    fn set_program_fp(fp: &mut Self::Fp, program: &FpRegs);

    /// An ARM64 vCPU's registers, for running translated x86 on it. None on any other.
    fn arm64(_regs: &mut Self::Regs) -> Option<&mut Arm64VcpuRegs> {
        None
    }
    /// The same for its SIMD registers.
    fn arm64_fp(_fp: &mut Self::Fp) -> Option<&mut crate::isa::arm64::Arm64Fp> {
        None
    }
}

/// What a shim gives a new CPU: its system registers, first registers, and what it keeps about it.
pub type CpuSetup<S> = (<S as Shim>::System, <S as Shim>::Regs, <S as Shim>::CpuData);

/// The shim for the CPU this was built for.
#[cfg(target_arch = "aarch64")]
pub type NativeShim = arm64::Arm64Shim;
#[cfg(not(target_arch = "aarch64"))]
pub type NativeShim = x86::X86Shim;
