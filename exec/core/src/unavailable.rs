//! Backends that aren't there: a host fills a slot it has nothing for yet with one of these, which never opens and
//! says why. Their CPUs can't exist, so there's nothing else to say about them.

use std::marker::PhantomData;

use exec_mmu::Prot;

use crate::contract::{GuestRam, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit, Vm, VmCpu, VmExit};
use crate::shim::Shim;
use crate::{FpRegs, Isa, Layout, Options, Regs};

/// Why a slot is empty.
pub trait Why: Send + 'static {
    const NAME: &'static str;
    const WHY: &'static str;
}

enum Never {}

/// A `vm` backend this host doesn't have.
pub struct NoVm<Sh: Shim, W: Why>(Never, PhantomData<(Sh, W)>);

/// Its CPU, which can't exist.
pub struct NoVmCpu<Sh: Shim>(Never, PhantomData<Sh>);

impl<Sh: Shim, W: Why> Vm for NoVm<Sh, W> {
    type Shim = Sh;
    type Cpu = NoVmCpu<Sh>;
    const NAME: &'static str = W::NAME;
    const RAM_BASE: u64 = 0;

    fn alloc_ram(_bytes: u64) -> Result<GuestRam, String> {
        Err(W::WHY.to_string())
    }
    fn open(_ram: &GuestRam, _options: &Options) -> Result<Self, String> {
        Err(W::WHY.to_string())
    }
    fn max_cpus(&self) -> usize {
        match self.0 {}
    }
    fn new_cpu(&self, _index: usize) -> Result<NoVmCpu<Sh>, String> {
        match self.0 {}
    }
}

impl<Sh: Shim> VmCpu for NoVmCpu<Sh> {
    type Shim = Sh;
    fn set_system(&mut self, _system: &Sh::System) -> Result<(), String> {
        match self.0 {}
    }
    fn regs(&mut self) -> &mut Sh::Regs {
        match self.0 {}
    }
    fn fp(&mut self) -> &mut Sh::Fp {
        match self.0 {}
    }
    fn set_root(&mut self, _value: u64) {
        match self.0 {}
    }
    fn run(&mut self) -> VmExit {
        match self.0 {}
    }
    fn preempt_handle(&self) -> RawPreempt {
        match self.0 {}
    }
}

/// A `sandbox` backend this host doesn't have yet.
pub struct NoSandbox<W: Why>(Never, PhantomData<W>);

/// Its CPU, which can't exist.
pub struct NoSandboxCpu(Never);

impl<W: Why> Sandbox for NoSandbox<W> {
    type Cpu = NoSandboxCpu;
    const NAME: &'static str = W::NAME;

    fn alloc_ram(_bytes: u64) -> Result<GuestRam, String> {
        Err(W::WHY.to_string())
    }
    fn open(_ram: &GuestRam, _options: &Options) -> Result<Self, String> {
        Err(W::WHY.to_string())
    }
    fn isas(&self) -> &'static [Isa] {
        match self.0 {}
    }
    fn reach(&self) -> Reach {
        match self.0 {}
    }
    fn layout(&self, _isa: Isa) -> Layout {
        match self.0 {}
    }
    fn max_cpus(&self) -> usize {
        match self.0 {}
    }
    fn mirror(&mut self, _page: u64, _ram_offset: Option<u64>, _prot: Prot) -> Result<(), String> {
        match self.0 {}
    }
    fn new_cpu(&self, _index: usize) -> Result<NoSandboxCpu, String> {
        match self.0 {}
    }
}

impl SandboxCpu for NoSandboxCpu {
    fn regs(&mut self) -> &mut Regs {
        match self.0 {}
    }
    fn fp(&mut self) -> &mut FpRegs {
        match self.0 {}
    }
    fn resume(&mut self) -> SandboxExit {
        match self.0 {}
    }
    fn preempt_handle(&self) -> RawPreempt {
        match self.0 {}
    }
}
