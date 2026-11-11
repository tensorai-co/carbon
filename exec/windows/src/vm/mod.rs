//! `vm` on Windows x86-64: Carbon's own driver, `winkvm.sys`, with the x86 shim that kvm uses too.
//!
//! The driver is KVM-shaped (`driver/winkvm.h`), so this is a thin translation: registers in and out of `RUN`, and
//! nothing else. It won't open until the driver is installed.
//!
//! The driver keeps a VM's vCPUs until the VM goes, so a vCPU that's dropped waits in `retired` for the next CPU made;
//! `RUN` and `CANCEL` name a vCPU, not a thread, so any thread can take it.

mod sys;

use std::sync::{Arc, Mutex, PoisonError, Weak};

use exec_core::contract::{GuestRam, RawPreempt, Vm, VmCpu, VmExit};
use exec_core::isa::x86::X86Fp;
use exec_core::shim::x86::{USER_DATA, X86Shim, X86System, X86VcpuRegs, segment};
use exec_core::Options;

use crate::ram;
use sys::{Caps, Device, Fp, Memory, Msr, Regs, Run, System, VcpuId};

pub struct WinKvm {
    device: Arc<Device>,
    vm: u32,
    max_vcpus: u32,
    /// vCPUs whose CPU has been dropped.
    retired: Arc<Mutex<Vec<VcpuId>>>,
}

impl Vm for WinKvm {
    type Shim = X86Shim;
    type Cpu = WinKvmCpu;
    const NAME: &'static str = "winkvm";
    const RAM_BASE: u64 = 0x1_0000_0000;

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<WinKvm, String> {
        let device = Arc::new(Device::open()?);
        let mut caps = Caps::default();
        device.call(sys::CAPS, &(), &mut caps)?;
        if caps.version != sys::API_VERSION {
            return Err(format!("winkvm.sys speaks version {:#x}, not {:#x}", caps.version, sys::API_VERSION));
        }
        if caps.vmx == 0 || caps.ept == 0 {
            return Err("this CPU hasn't VT-x with EPT switched on".to_string());
        }
        let mut id = VcpuId::default();
        device.call(sys::CREATE_VM, &(), &mut id)?;
        let memory = Memory { vm: id.vm, reserved: 0, guest_phys: Self::RAM_BASE, size: ram.size, user_address: ram.ptr as u64 };
        if let Err(why) = device.send(sys::SET_MEMORY, &memory) {
            let _ = device.send(sys::DESTROY_VM, &id);
            return Err(why);
        }
        Ok(WinKvm { device, vm: id.vm, max_vcpus: caps.max_vcpus.max(1), retired: Arc::default() })
    }

    fn max_cpus(&self) -> usize {
        self.max_vcpus as usize
    }

    fn new_cpu(&self, _index: usize) -> Result<WinKvmCpu, String> {
        let retired = Arc::downgrade(&self.retired);
        if let Some(id) = self.retired.lock().unwrap_or_else(PoisonError::into_inner).pop() {
            // The FPU as a new vCPU has it, written before it first runs; the rest the shim sets again.
            return Ok(WinKvmCpu { device: self.device.clone(), id, retired, regs: X86VcpuRegs::default(), cr3: 0, fp: X86Fp::default(), fp_loaded: true, fp_dirty: true });
        }
        let mut id = VcpuId::default();
        self.device.call(sys::CREATE_VCPU, &VcpuId { vm: self.vm, vcpu: 0 }, &mut id)?;
        Ok(WinKvmCpu { device: self.device.clone(), id, retired, regs: X86VcpuRegs::default(), cr3: 0, fp: X86Fp::default(), fp_loaded: true, fp_dirty: false })
    }
}

impl Drop for WinKvm {
    fn drop(&mut self) {
        let _ = self.device.send(sys::DESTROY_VM, &VcpuId { vm: self.vm, vcpu: 0 });
    }
}

pub struct WinKvmCpu {
    device: Arc<Device>,
    id: VcpuId,
    /// Where it goes when it's dropped while the VM's there.
    retired: Weak<Mutex<Vec<VcpuId>>>,
    regs: X86VcpuRegs,
    cr3: u64,
    fp: X86Fp,
    fp_loaded: bool,
    fp_dirty: bool,
}

impl Drop for WinKvmCpu {
    fn drop(&mut self) {
        if let Some(retired) = self.retired.upgrade() {
            retired.lock().unwrap_or_else(PoisonError::into_inner).push(self.id);
        }
    }
}

impl VmCpu for WinKvmCpu {
    type Shim = X86Shim;

    fn set_system(&mut self, system: &X86System) -> Result<(), String> {
        let mut msrs = [Msr::default(); 8];
        for (slot, (index, value)) in msrs.iter_mut().zip(&system.msrs) {
            *slot = Msr { index: *index, reserved: 0, value: *value };
        }
        let call = System {
            vm: self.id.vm,
            vcpu: self.id.vcpu,
            cr0: system.cr0,
            cr4: system.cr4,
            efer: system.efer,
            // x87 and SSE; the driver widens it to what the CPU has.
            xcr0: 3,
            gdt_base: system.gdt.0,
            idt_base: system.idt.0,
            gdt_limit: system.gdt.1,
            idt_limit: system.idt.1,
            msr_count: system.msrs.len().min(8) as u32,
            tr: system.tr.into(),
            msrs,
        };
        self.device.send(sys::SET_SYSTEM, &call)?;
        self.fp_dirty = true;
        Ok(())
    }

    fn regs(&mut self) -> &mut X86VcpuRegs {
        &mut self.regs
    }

    fn fp(&mut self) -> &mut X86Fp {
        if !self.fp_loaded {
            let mut fp = Fp { vm: self.id.vm, vcpu: self.id.vcpu, fxsave: [0; 512] };
            if self.device.call(sys::GET_FP, &self.id, &mut fp).is_ok() {
                self.fp.fxsave = fp.fxsave;
            }
            self.fp_loaded = true;
        }
        self.fp_dirty = true;
        &mut self.fp
    }

    fn set_root(&mut self, value: u64) {
        self.cr3 = value;
    }

    fn run(&mut self) -> VmExit {
        if self.fp_dirty {
            let fp = Fp { vm: self.id.vm, vcpu: self.id.vcpu, fxsave: self.fp.fxsave };
            let _ = self.device.send(sys::SET_FP, &fp);
            self.fp_dirty = false;
        }
        let r = &self.regs;
        let regs = Regs {
            gpr: r.gpr,
            rip: r.rip,
            rflags: r.rflags,
            cr2: r.cr2,
            cr3: self.cr3,
            cs: segment(r.cs, 0).into(),
            ss: segment(r.ss, 0).into(),
            ds: segment(USER_DATA, 0).into(),
            es: segment(USER_DATA, 0).into(),
            fs: segment(r.fs, r.fs_base).into(),
            gs: segment(USER_DATA, r.gs_base).into(),
        };
        let mut run = Run { vm: self.id.vm, vcpu: self.id.vcpu, regs, ..Run::default() };
        let input = run;
        if let Err(_why) = self.device.call(sys::RUN, &input, &mut run) {
            return VmExit::Failed(0x5000);
        }
        self.fp_loaded = false;
        let out = &run.regs;
        self.regs.gpr = out.gpr;
        self.regs.rip = out.rip;
        self.regs.rflags = out.rflags;
        self.regs.cr2 = out.cr2;
        self.regs.cs = out.cs.selector;
        self.regs.ss = out.ss.selector;
        self.regs.fs = out.fs.selector;
        self.regs.fs_base = out.fs.base;
        self.regs.gs_base = out.gs.base;
        match run.exit_reason {
            sys::EXIT_HLT => VmExit::Trap { at: self.regs.rip },
            sys::EXIT_CANCELED => VmExit::Preempted,
            other => VmExit::Failed(0x5100 | u64::from(other) << 32 | (run.qualification & 0xffff_ffff)),
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let (device, id) = (self.device.clone(), self.id);
        Arc::new(move || {
            let _ = device.send(sys::CANCEL, &id);
        })
    }
}
