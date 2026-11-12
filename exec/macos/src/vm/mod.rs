//! `vm` on macOS: Hypervisor.framework, with the ARM64 shim.
//!
//! The framework allows one VM per process, so a machine runs in a worker process, which must be signed with
//! `com.apple.security.hypervisor`. Each vCPU belongs to the thread that made it.

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use exec_core::contract::{GuestRam, RawPreempt, Vm, VmCpu, VmExit};
use exec_core::isa::arm64::Arm64Fp;
use exec_core::shim::arm64::{Arm64Shim, Arm64System, Arm64VcpuRegs, VECTOR_SIZE};
use exec_core::Options;

use crate::ram;
use crate::sys::*;

pub struct Hvf {
    ram_size: u64,
}

impl Vm for Hvf {
    type Shim = Arm64Shim;
    type Cpu = HvfCpu;
    const NAME: &'static str = "hvf";
    const RAM_BASE: u64 = 0x4000_0000;

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Hvf, String> {
        // SAFETY: no configuration, for the default VM.
        check("creating the VM (is the binary signed with com.apple.security.hypervisor?)", unsafe { hv_vm_create(ptr::null_mut()) })?;
        let flags = HV_MEMORY_READ | HV_MEMORY_WRITE | HV_MEMORY_EXEC;
        // SAFETY: guest memory, which outlives the VM, as core drops the backend first.
        let mapped = unsafe { hv_vm_map(ram.ptr.cast::<c_void>(), Self::RAM_BASE, ram.size as usize, flags) };
        if let Err(why) = check("giving the VM its memory", mapped) {
            // SAFETY: the VM just made, with nothing in it.
            unsafe { hv_vm_destroy() };
            return Err(why);
        }
        Ok(Hvf { ram_size: ram.size })
    }

    fn max_cpus(&self) -> usize {
        let mut count = 0;
        // SAFETY: an out-parameter.
        let result = unsafe { hv_vm_get_max_vcpu_count(&mut count) };
        if result == HV_SUCCESS { count as usize } else { 1 }
    }

    fn tso(&self) -> bool {
        true
    }

    fn sync_code(&self, host: *mut u8, len: usize) {
        // SAFETY: guest memory, `len` bytes of it.
        unsafe { sys_icache_invalidate(host.cast(), len) };
    }

    fn new_cpu(&self, _index: usize) -> Result<HvfCpu, String> {
        HvfCpu::create()
    }
}

impl Drop for Hvf {
    fn drop(&mut self) {
        // SAFETY: the VM's memory and the VM itself, which nothing runs any more.
        unsafe {
            hv_vm_unmap(Self::RAM_BASE, self.ram_size as usize);
            hv_vm_destroy();
        }
    }
}

/// One vCPU. Its registers are cached here: loaded after each run, stored before the next if anything changed them.
pub struct HvfCpu {
    id: hv_vcpu_t,
    exit: *const hv_vcpu_exit_t,
    regs: Arm64VcpuRegs,
    regs_dirty: bool,
    fp: Arm64Fp,
    fp_loaded: bool,
    fp_dirty: bool,
}

/// The general registers and the system registers that change while running, in the order they're moved.
const SYS_REGS: [u16; 6] = [HV_SYS_REG_SP_EL0, HV_SYS_REG_ELR_EL1, HV_SYS_REG_SPSR_EL1, HV_SYS_REG_ESR_EL1, HV_SYS_REG_FAR_EL1, HV_SYS_REG_TPIDR_EL0];

impl HvfCpu {
    fn create() -> Result<HvfCpu, String> {
        let (mut id, mut exit) = (0, ptr::null());
        // SAFETY: out-parameters; `exit` stays valid until the vCPU is destroyed.
        check("creating a vCPU", unsafe { hv_vcpu_create(&mut id, &mut exit, ptr::null_mut()) })?;
        // Every vCPU's virtual counter is the host's, with no offset: the same on all of them, and what the kernel's
        // clock page pairs with its monotonic clock, for the vDSO.
        // SAFETY: a vCPU this thread just made.
        check("setting a vCPU's counter", unsafe { hv_vcpu_set_vtimer_offset(id, 0) })?;
        Ok(HvfCpu { id, exit, regs: Arm64VcpuRegs::default(), regs_dirty: false, fp: Arm64Fp::default(), fp_loaded: false, fp_dirty: false })
    }

    fn reg(&self, reg: u32) -> u64 {
        let mut value = 0;
        // SAFETY: a register of a vCPU this thread owns.
        unsafe { hv_vcpu_get_reg(self.id, reg, &mut value) };
        value
    }

    fn sys_reg(&self, reg: u16) -> u64 {
        let mut value = 0;
        // SAFETY: as above.
        unsafe { hv_vcpu_get_sys_reg(self.id, reg, &mut value) };
        value
    }

    fn set_reg(&self, reg: u32, value: u64) {
        // SAFETY: as above.
        unsafe { hv_vcpu_set_reg(self.id, reg, value) };
    }

    fn set_sys_reg(&self, reg: u16, value: u64) {
        // SAFETY: as above.
        unsafe { hv_vcpu_set_sys_reg(self.id, reg, value) };
    }

    fn sys_values(regs: &mut Arm64VcpuRegs) -> [&mut u64; 6] {
        [&mut regs.sp_el0, &mut regs.elr, &mut regs.spsr, &mut regs.esr, &mut regs.far, &mut regs.tpidr]
    }

    fn load(&mut self) {
        let mut regs = Arm64VcpuRegs::default();
        for (i, x) in regs.x.iter_mut().enumerate() {
            *x = self.reg(HV_REG_X0 + i as u32);
        }
        regs.pc = self.reg(HV_REG_PC);
        regs.cpsr = self.reg(HV_REG_CPSR);
        for (reg, value) in SYS_REGS.iter().zip(Self::sys_values(&mut regs)) {
            *value = self.sys_reg(*reg);
        }
        self.regs = regs;
    }

    fn store(&mut self) {
        let mut regs = self.regs;
        for (i, x) in regs.x.iter().enumerate() {
            self.set_reg(HV_REG_X0 + i as u32, *x);
        }
        self.set_reg(HV_REG_PC, regs.pc);
        self.set_reg(HV_REG_CPSR, regs.cpsr);
        for (reg, value) in SYS_REGS.iter().zip(Self::sys_values(&mut regs)) {
            self.set_sys_reg(*reg, *value);
        }
    }

    fn load_fp(&mut self) {
        for (i, v) in self.fp.v.iter_mut().enumerate() {
            // SAFETY: a register of a vCPU this thread owns, into a u128.
            unsafe { hv_vcpu_get_simd_fp_reg(self.id, i as u32, v) };
        }
        self.fp.fpsr = self.reg(HV_REG_FPSR) as u32;
        self.fp.fpcr = self.reg(HV_REG_FPCR) as u32;
        self.fp_loaded = true;
    }

    fn store_fp(&mut self) {
        for (i, v) in self.fp.v.iter().enumerate() {
            // SAFETY: as above, from a u128.
            unsafe { exec_hv_set_simd_fp_reg(self.id, i as u32, v) };
        }
        self.set_reg(HV_REG_FPSR, u64::from(self.fp.fpsr));
        self.set_reg(HV_REG_FPCR, u64::from(self.fp.fpcr));
    }
}

impl VmCpu for HvfCpu {
    type Shim = Arm64Shim;

    fn set_system(&mut self, system: &Arm64System) -> Result<(), String> {
        for (reg, value) in [
            (HV_SYS_REG_MAIR_EL1, system.mair),
            (HV_SYS_REG_TCR_EL1, system.tcr),
            (HV_SYS_REG_TTBR1_EL1, system.ttbr1),
            (HV_SYS_REG_VBAR_EL1, system.vbar),
            (HV_SYS_REG_CPACR_EL1, system.cpacr),
            (HV_SYS_REG_CNTKCTL_EL1, system.cntkctl),
            (HV_SYS_REG_SCTLR_EL1, system.sctlr),
        ] {
            // SAFETY: a register of a vCPU this thread owns.
            check("setting up a vCPU", unsafe { hv_vcpu_set_sys_reg(self.id, reg, value) })?;
        }
        // Only on macOS 15 and later, and only EnTSO, so failing is fine when TSO isn't asked for.
        // SAFETY: as above.
        let tso = unsafe { hv_vcpu_set_sys_reg(self.id, HV_SYS_REG_ACTLR_EL1, system.actlr) };
        if system.actlr != 0 {
            check("switching on TSO (it needs macOS 15)", tso)?;
        }
        Ok(())
    }

    fn regs(&mut self) -> &mut Arm64VcpuRegs {
        self.regs_dirty = true;
        &mut self.regs
    }

    fn fp(&mut self) -> &mut Arm64Fp {
        if !self.fp_loaded {
            self.load_fp();
        }
        self.fp_dirty = true;
        &mut self.fp
    }

    fn set_root(&mut self, value: u64) {
        self.set_sys_reg(HV_SYS_REG_TTBR0_EL1, value);
    }

    fn run(&mut self) -> VmExit {
        if self.regs_dirty {
            self.store();
            self.regs_dirty = false;
        }
        if self.fp_dirty {
            self.store_fp();
            self.fp_dirty = false;
        }
        let (result, exit) = loop {
            // SAFETY: a vCPU this thread owns.
            let result = unsafe { hv_vcpu_run(self.id) };
            // SAFETY: valid as long as the vCPU, and only changed by running it.
            let exit = unsafe { *self.exit };
            // Stopped just inside a vector: an exception was taken and its hypercall is still to come. One more
            // instruction finishes it, rather than losing it.
            let at_vector = self.reg(HV_REG_CPSR) & 0xf != 0 && self.reg(HV_REG_PC) & (VECTOR_SIZE - 1) == 0;
            if result == HV_SUCCESS && exit.reason == HV_EXIT_REASON_CANCELED && at_vector {
                continue;
            }
            break (result, exit);
        };
        self.load();
        self.fp_loaded = false;
        if result != HV_SUCCESS {
            return VmExit::Failed(result as u32 as u64);
        }
        match exit.reason {
            HV_EXIT_REASON_CANCELED => VmExit::Preempted,
            HV_EXIT_REASON_EXCEPTION if exit.exception.syndrome >> 26 & 0x3f == EC_HVC => VmExit::Trap { at: exit.exception.syndrome & 0xffff },
            HV_EXIT_REASON_EXCEPTION => VmExit::Failed(exit.exception.syndrome),
            other => VmExit::Failed(0x1000 | u64::from(other)),
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let id = self.id;
        Arc::new(move || {
            // SAFETY: any thread may ask a vCPU to exit.
            unsafe { hv_vcpus_exit(&id, 1) };
        })
    }
}

impl Drop for HvfCpu {
    fn drop(&mut self) {
        // SAFETY: a vCPU this thread owns, which nothing uses again.
        unsafe { hv_vcpu_destroy(self.id) };
    }
}
