//! `vm` on Linux x86-64: KVM, with the x86 shim.
//!
//! Needs `KVM_CAP_SYNC_REGS`, so registers move through `kvm_run` rather than an ioctl each, and
//! `KVM_CAP_IMMEDIATE_EXIT`, so a preempt can't be lost between deciding to run and running. There's no in-kernel
//! interrupt controller: a halt in a shim stub exits to here.
//!
//! KVM never takes a vCPU back until the VM goes, and an ID can't be made twice. So a vCPU that's dropped waits in
//! `retired` for the next CPU made in its slot, on whatever thread that is (KVM lets a vCPU move between threads).

mod sys;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use exec_core::contract::{GuestRam, RawPreempt, Vm, VmCpu, VmExit};
use exec_core::isa::x86::X86Fp;
use exec_core::shim::x86::{USER_DATA, X86Segment, X86Shim, X86System, X86VcpuRegs, segment};
use exec_core::Options;

use crate::{preempt, ram};
use sys::{CpuidEntry, DescriptorTable, Fpu, MemoryRegion, Regs, Segment, Sregs, ioctl, ioctl_value};

/// vCPUs whose CPU has been dropped, by slot: each one's descriptor and run area.
type Retired = Mutex<Vec<(usize, i32, Arc<RunArea>)>>;

pub struct Kvm {
    kvm: i32,
    vm: i32,
    run_size: usize,
    cpuid: Vec<CpuidEntry>,
    /// The state the CPU saves and restores: x87, SSE, AVX, and AVX-512's if it has them.
    xcr0: u64,
    retired: Arc<Retired>,
}

impl Vm for Kvm {
    type Shim = X86Shim;
    type Cpu = KvmCpu;
    const NAME: &'static str = "kvm";
    /// Above 4 GiB, clear of what KVM keeps below it.
    const RAM_BASE: u64 = 0x1_0000_0000;

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Kvm, String> {
        // SAFETY: a path and flags.
        let kvm = unsafe { libc::open(c"/dev/kvm".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if kvm < 0 {
            return Err(format!("/dev/kvm: {}", std::io::Error::last_os_error()));
        }
        let fail = |why: String| {
            // SAFETY: the descriptor just opened.
            unsafe { libc::close(kvm) };
            Err(why)
        };
        if ioctl_value(kvm, sys::API_VERSION, 0) != Ok(12) {
            return fail("KVM's API isn't version 12".to_string());
        }
        for (cap, what) in [(sys::CAP_SYNC_REGS, "shared registers"), (sys::CAP_IMMEDIATE_EXIT, "immediate exits")] {
            if ioctl_value(kvm, sys::CHECK_EXTENSION, cap).unwrap_or(0) <= 0 {
                return fail(format!("this KVM hasn't {what}"));
            }
        }
        let vm = match ioctl_value(kvm, sys::CREATE_VM, 0) {
            Ok(vm) => vm,
            Err(e) => return fail(format!("creating a VM: error {e}")),
        };
        let _ = ioctl_value(vm, sys::SET_TSS_ADDR, 0xfffb_d000);
        let mut identity = 0xfffb_c000u64;
        // SAFETY: a u64, as it takes.
        let _ = unsafe { ioctl(vm, sys::SET_IDENTITY_MAP_ADDR, &mut identity) };
        let mut region = MemoryRegion { slot: 0, flags: 0, guest_phys_addr: Self::RAM_BASE, memory_size: ram.size, userspace_addr: ram.ptr as u64 };
        // SAFETY: guest memory, which outlives the VM.
        if let Err(e) = unsafe { ioctl(vm, sys::SET_USER_MEMORY_REGION, &mut region) } {
            // SAFETY: the VM just made.
            unsafe { libc::close(vm) };
            return fail(format!("giving the VM its memory: error {e}"));
        }
        let run_size = ioctl_value(kvm, sys::GET_VCPU_MMAP_SIZE, 0).map_err(|e| format!("KVM's run area: error {e}"))? as usize;
        let cpuid = supported_cpuid(kvm)?;
        let xcr0 = cpuid.iter().find(|e| e.function == 0xd && e.index == 0).map_or(3, |e| u64::from(e.eax) & 0xe7);
        preempt::install();
        Ok(Kvm { kvm, vm, run_size, cpuid, xcr0, retired: Arc::default() })
    }

    fn max_cpus(&self) -> usize {
        ioctl_value(self.kvm, sys::CHECK_EXTENSION, sys::CAP_MAX_VCPUS).map_or(1, |n| n.max(1) as usize)
    }

    fn new_cpu(&self, index: usize) -> Result<KvmCpu, String> {
        // SAFETY: only reads which thread this is.
        let thread = unsafe { libc::pthread_self() };
        let again = {
            let mut retired = self.retired.lock().unwrap_or_else(PoisonError::into_inner);
            retired.iter().position(|r| r.0 == index).map(|at| retired.swap_remove(at))
        };
        if let Some((_, fd, run)) = again {
            // What a preempt for the vCPU's last owner left is no business of this one's.
            run.immediate_exit().store(0, Ordering::SeqCst);
            // The FPU as a new vCPU has it, written before it first runs; the rest the shim sets again.
            return Ok(KvmCpu {
                fd,
                index,
                retired: Arc::downgrade(&self.retired),
                run,
                thread,
                regs: X86VcpuRegs::default(),
                sregs: Sregs::default(),
                fp: X86Fp::default(),
                fp_loaded: true,
                fp_dirty: true,
                xcr0: self.xcr0,
            });
        }
        let fd = ioctl_value(self.vm, sys::CREATE_VCPU, index as u64).map_err(|e| format!("creating a vCPU: error {e}"))?;
        // SAFETY: the vCPU's run area, shared with KVM, unmapped when the last holder drops it.
        let run = unsafe { libc::mmap(std::ptr::null_mut(), self.run_size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0) };
        if run == libc::MAP_FAILED {
            // SAFETY: the vCPU just made.
            unsafe { libc::close(fd) };
            return Err(format!("mapping a vCPU's run area: {}", std::io::Error::last_os_error()));
        }
        self.set_cpuid(fd)?;
        Ok(KvmCpu {
            fd,
            index,
            retired: Arc::downgrade(&self.retired),
            run: Arc::new(RunArea { ptr: run.cast(), size: self.run_size }),
            thread,
            regs: X86VcpuRegs::default(),
            sregs: Sregs::default(),
            fp: X86Fp::default(),
            fp_loaded: false,
            fp_dirty: false,
            xcr0: self.xcr0,
        })
    }
}

impl Kvm {
    /// What KVM supports, as it is, but saying there's no hypervisor, and that OSXSAVE follows cr4.
    fn set_cpuid(&self, fd: i32) -> Result<(), String> {
        let entries: Vec<CpuidEntry> = self
            .cpuid
            .iter()
            .copied()
            .filter(|e| !(0x4000_0000..0x5000_0000).contains(&e.function))
            .map(|mut e| {
                if e.function == 1 {
                    e.ecx |= 1 << 27;
                    e.ecx &= !(1 << 31);
                }
                e
            })
            .collect();
        let mut buffer = vec![0u8; 8 + entries.len() * size_of::<CpuidEntry>()];
        buffer[..4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for (i, entry) in entries.iter().enumerate() {
            // SAFETY: a plain structure, into room for it.
            unsafe { std::ptr::write_unaligned(buffer.as_mut_ptr().add(8 + i * size_of::<CpuidEntry>()).cast(), *entry) };
        }
        // SAFETY: a `kvm_cpuid2` with its entries.
        unsafe { ioctl(fd, sys::SET_CPUID2, buffer.as_mut_ptr()) }.map(|_| ()).map_err(|e| format!("setting CPUID: error {e}"))
    }
}

impl Drop for Kvm {
    fn drop(&mut self) {
        for (_, fd, _) in self.retired.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
            // SAFETY: a retired vCPU's descriptor, which nothing uses any more.
            unsafe { libc::close(fd) };
        }
        // SAFETY: descriptors this made, which nothing uses any more.
        unsafe {
            libc::close(self.vm);
            libc::close(self.kvm);
        }
    }
}

fn supported_cpuid(kvm: i32) -> Result<Vec<CpuidEntry>, String> {
    let mut count = 128usize;
    loop {
        let mut buffer = vec![0u8; 8 + count * size_of::<CpuidEntry>()];
        buffer[..4].copy_from_slice(&(count as u32).to_le_bytes());
        // SAFETY: a `kvm_cpuid2` with room for `count` entries.
        match unsafe { ioctl(kvm, sys::GET_SUPPORTED_CPUID, buffer.as_mut_ptr()) } {
            Ok(_) => {
                let n = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
                // SAFETY: `n` entries, as KVM filled in.
                return Ok((0..n).map(|i| unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(8 + i * size_of::<CpuidEntry>()).cast()) }).collect());
            }
            Err(libc::E2BIG) if count < 4096 => count *= 2,
            Err(e) => return Err(format!("asking KVM what CPUID it supports: error {e}")),
        }
    }
}

/// A vCPU's `kvm_run`, shared with KVM and with whoever preempts it.
struct RunArea {
    ptr: *mut u8,
    size: usize,
}

// SAFETY: other threads only set `immediate_exit`, a byte meant for exactly that.
unsafe impl Send for RunArea {}
// SAFETY: as above.
unsafe impl Sync for RunArea {}

impl RunArea {
    fn at<T>(&self, offset: usize) -> *mut T {
        // SAFETY: within the run area.
        unsafe { self.ptr.add(offset).cast() }
    }

    fn immediate_exit(&self) -> &AtomicU8 {
        // SAFETY: `kvm_run`'s `immediate_exit`, a byte any thread may set.
        unsafe { AtomicU8::from_ptr(self.at(sys::RUN_IMMEDIATE_EXIT)) }
    }
}

impl Drop for RunArea {
    fn drop(&mut self) {
        // SAFETY: the mapping, which nothing uses any more.
        unsafe { libc::munmap(self.ptr.cast(), self.size) };
    }
}

fn kvm_segment(s: X86Segment) -> Segment {
    Segment {
        base: s.base,
        limit: s.limit,
        selector: s.selector,
        kind: s.kind,
        present: u8::from(s.present),
        dpl: s.dpl,
        db: u8::from(s.db),
        s: u8::from(s.s),
        l: u8::from(s.l),
        g: u8::from(s.g),
        ..Segment::default()
    }
}

pub struct KvmCpu {
    fd: i32,
    /// Its slot, and where it goes when it's dropped while the VM's there.
    index: usize,
    retired: Weak<Retired>,
    run: Arc<RunArea>,
    thread: libc::pthread_t,
    regs: X86VcpuRegs,
    /// What `kvm_run`'s special registers are set from, kept here between runs.
    sregs: Sregs,
    fp: X86Fp,
    fp_loaded: bool,
    fp_dirty: bool,
    xcr0: u64,
}

impl KvmCpu {
    /// Registers into `kvm_run`, for the next `KVM_RUN` to take.
    fn sync_in(&mut self) {
        let r = &self.regs;
        self.sregs.cs = kvm_segment(segment(r.cs, 0));
        self.sregs.ss = kvm_segment(segment(r.ss, 0));
        self.sregs.ds = kvm_segment(segment(USER_DATA, 0));
        self.sregs.es = kvm_segment(segment(USER_DATA, 0));
        self.sregs.fs = kvm_segment(segment(r.fs, r.fs_base));
        self.sregs.gs = kvm_segment(segment(USER_DATA, r.gs_base));
        // SAFETY: `kvm_run`'s synced registers, which only this thread touches.
        unsafe {
            self.run.at::<Regs>(sys::RUN_REGS).write(Regs::from_gpr(&r.gpr, r.rip, r.rflags));
            self.run.at::<Sregs>(sys::RUN_SREGS).write(self.sregs);
            *self.run.at::<u64>(sys::RUN_DIRTY_REGS) = sys::SYNC_REGS | sys::SYNC_SREGS;
        }
    }

    /// Registers out of `kvm_run`, as `KVM_RUN` left them.
    fn sync_out(&mut self) {
        // SAFETY: as above.
        let (regs, sregs) = unsafe { (self.run.at::<Regs>(sys::RUN_REGS).read(), self.run.at::<Sregs>(sys::RUN_SREGS).read()) };
        self.sregs = sregs;
        self.regs.gpr = regs.gpr();
        self.regs.rip = regs.rip;
        self.regs.rflags = regs.rflags;
        self.regs.cs = sregs.cs.selector;
        self.regs.ss = sregs.ss.selector;
        self.regs.fs = sregs.fs.selector;
        self.regs.fs_base = sregs.fs.base;
        self.regs.gs_base = sregs.gs.base;
        self.regs.cr2 = sregs.cr2;
    }

    fn load_fp(&mut self) {
        let mut fpu = Fpu::default();
        // SAFETY: an out-parameter of the right size.
        let _ = unsafe { ioctl(self.fd, sys::GET_FPU, &mut fpu) };
        let f = &mut self.fp.fxsave;
        f[0..2].copy_from_slice(&fpu.fcw.to_le_bytes());
        f[2..4].copy_from_slice(&fpu.fsw.to_le_bytes());
        f[4] = fpu.ftwx;
        f[6..8].copy_from_slice(&fpu.last_opcode.to_le_bytes());
        f[8..16].copy_from_slice(&fpu.last_ip.to_le_bytes());
        f[16..24].copy_from_slice(&fpu.last_dp.to_le_bytes());
        f[24..28].copy_from_slice(&fpu.mxcsr.to_le_bytes());
        for (i, st) in fpu.fpr.iter().enumerate() {
            f[32 + i * 16..48 + i * 16].copy_from_slice(st);
        }
        for (i, xmm) in fpu.xmm.iter().enumerate() {
            f[160 + i * 16..176 + i * 16].copy_from_slice(xmm);
        }
        self.fp_loaded = true;
    }

    fn store_fp(&mut self) {
        let f = &self.fp.fxsave;
        let word = |at: usize| u16::from_le_bytes([f[at], f[at + 1]]);
        let quad = |at: usize| u64::from_le_bytes(f[at..at + 8].try_into().unwrap_or_default());
        let mut fpu = Fpu {
            fcw: word(0),
            fsw: word(2),
            ftwx: f[4],
            last_opcode: word(6),
            last_ip: quad(8),
            last_dp: quad(16),
            mxcsr: u32::from_le_bytes(f[24..28].try_into().unwrap_or_default()),
            ..Fpu::default()
        };
        for (i, st) in fpu.fpr.iter_mut().enumerate() {
            st.copy_from_slice(&f[32 + i * 16..48 + i * 16]);
        }
        for (i, xmm) in fpu.xmm.iter_mut().enumerate() {
            xmm.copy_from_slice(&f[160 + i * 16..176 + i * 16]);
        }
        // SAFETY: the structure just filled in.
        let _ = unsafe { ioctl(self.fd, sys::SET_FPU, &mut fpu) };
    }
}

impl VmCpu for KvmCpu {
    type Shim = X86Shim;

    fn set_system(&mut self, system: &X86System) -> Result<(), String> {
        let mut sregs = Sregs::default();
        // SAFETY: an out-parameter of the right size.
        unsafe { ioctl(self.fd, sys::GET_SREGS, &mut sregs) }.map_err(|e| format!("reading a vCPU: error {e}"))?;
        sregs.cr0 = system.cr0;
        sregs.cr4 = system.cr4;
        sregs.efer = system.efer;
        sregs.gdt = DescriptorTable { base: system.gdt.0, limit: system.gdt.1, padding: [0; 3] };
        sregs.idt = DescriptorTable { base: system.idt.0, limit: system.idt.1, padding: [0; 3] };
        sregs.tr = kvm_segment(system.tr);
        sregs.ldt = Segment { unusable: 1, ..Segment::default() };
        // SAFETY: the structure just filled in.
        unsafe { ioctl(self.fd, sys::SET_SREGS, &mut sregs) }.map_err(|e| format!("setting up a vCPU: error {e}"))?;
        self.sregs = sregs;

        let mut xcrs = sys::Xcrs { count: 1, flags: 0, xcrs: std::array::from_fn(|_| sys::Xcr { xcr: 0, reserved: 0, value: 0 }), padding: [0; 16] };
        xcrs.xcrs[0].value = self.xcr0;
        // SAFETY: the structure just filled in.
        let _ = unsafe { ioctl(self.fd, sys::SET_XCRS, &mut xcrs) };

        let mut buffer = vec![0u8; 8 + system.msrs.len() * 16];
        buffer[..4].copy_from_slice(&(system.msrs.len() as u32).to_le_bytes());
        for (i, (index, value)) in system.msrs.iter().enumerate() {
            buffer[8 + i * 16..12 + i * 16].copy_from_slice(&index.to_le_bytes());
            buffer[16 + i * 16..24 + i * 16].copy_from_slice(&value.to_le_bytes());
        }
        // SAFETY: a `kvm_msrs` with its entries.
        unsafe { ioctl(self.fd, sys::SET_MSRS, buffer.as_mut_ptr()) }.map_err(|e| format!("setting MSRs: error {e}"))?;

        // Registers come and go through `kvm_run` from now on.
        // SAFETY: the run area's field for exactly this.
        unsafe { *self.run.at::<u64>(sys::RUN_VALID_REGS) = sys::SYNC_REGS | sys::SYNC_SREGS };
        let mut regs = Regs { rflags: 2, ..Regs::default() };
        // SAFETY: the structure just made.
        unsafe { ioctl(self.fd, sys::SET_REGS, &mut regs) }.map_err(|e| format!("setting registers: error {e}"))?;
        self.fp = X86Fp::default();
        self.fp_loaded = true;
        self.store_fp();
        Ok(())
    }

    fn regs(&mut self) -> &mut X86VcpuRegs {
        &mut self.regs
    }

    fn fp(&mut self) -> &mut X86Fp {
        if !self.fp_loaded {
            self.load_fp();
        }
        self.fp_dirty = true;
        &mut self.fp
    }

    fn set_root(&mut self, value: u64) {
        self.sregs.cr3 = value;
    }

    fn run(&mut self) -> VmExit {
        if self.fp_dirty {
            self.store_fp();
            self.fp_dirty = false;
        }
        self.sync_in();
        let result = ioctl_value(self.fd, sys::RUN, 0);
        self.run.immediate_exit().store(0, Ordering::SeqCst);
        self.sync_out();
        self.fp_loaded = false;
        // SAFETY: the exit reason, which `KVM_RUN` just wrote.
        let reason = unsafe { *self.run.at::<u32>(sys::RUN_EXIT_REASON) };
        match result {
            Err(libc::EINTR) => VmExit::Preempted,
            Err(e) => VmExit::Failed(0x3000 | e as u64),
            Ok(_) if reason == sys::EXIT_HLT => VmExit::Trap { at: self.regs.rip },
            Ok(_) if reason == sys::EXIT_INTR => VmExit::Preempted,
            Ok(_) if reason == sys::EXIT_FAIL_ENTRY => {
                // SAFETY: the failure's own field, which `KVM_RUN` wrote with this reason.
                let hardware = unsafe { *self.run.at::<u64>(sys::RUN_FAIL_ENTRY_REASON) };
                VmExit::Failed(0x3409 | hardware << 16)
            }
            Ok(_) => VmExit::Failed(0x3400 | u64::from(reason)),
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let (run, thread) = (self.run.clone(), self.thread);
        Arc::new(move || {
            run.immediate_exit().store(1, Ordering::SeqCst);
            preempt::thread(thread);
        })
    }
}

impl Drop for KvmCpu {
    fn drop(&mut self) {
        match self.retired.upgrade() {
            Some(retired) => retired.lock().unwrap_or_else(PoisonError::into_inner).push((self.index, self.fd, self.run.clone())),
            None => {
                // SAFETY: this vCPU's descriptor, which nothing uses again: the VM's gone.
                unsafe { libc::close(self.fd) };
            }
        }
    }
}
