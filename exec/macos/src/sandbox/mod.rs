//! `sandbox` on macOS, Apple silicon: the program runs natively, on a thread Carbon makes in a stub task, but only ever
//! as rewritten code.
//!
//! - **System calls**: XNU sends `svc` only to its own kernel, and lets no one intercept it. So a program never runs its
//!   own code: every block is rewritten first (`exec_translate::rewrite`), `svc` and branches becoming `brk`s, into a
//!   code cache in the stub (`Reach::Rewrite`). Every `brk` arrives as a Mach exception; a branch is followed here,
//!   without the kernel, and an `svc` becomes `SandboxExit::Gate`.
//! - **The stub** is Carbon's own binary run again (`run_if_stub`), whose task Carbon takes with `task_for_pid`: Carbon
//!   needs `com.apple.security.cs.debugger` and the stub `get-task-allow`, both in `carbon.entitlements`. Its own
//!   thread waits; the program runs on a raw Mach thread Carbon makes. XNU keeps the bottom 4 GiB of every 64-bit
//!   process empty, so programs live above it (`Layout::kept`).
//! - **Memory**, in this first cut, is copied: guest pages are 4 KiB and the host's 16 KiB, so a host page can't simply
//!   map four guest frames that needn't be next to each other. Each 16 KiB window of the stub holds its guest pages'
//!   contents, written in before a run and read back after a stop: correct for one CPU, and slow. Windows whose
//!   frames happen to line up could be shared views instead; that, and per-page protection inside a window, are next.
//! - **Faults** arrive as Mach exceptions on the program's thread, and are read and answered here.
//!
//! One CPU, ARM64 programs. x86 would plug the x86 → ARM64 emitter into the same code cache.

mod mach;
mod stub;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use exec_core::contract::{FaultKind, GuestRam, HostFault, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::Trap;
use exec_core::isa::arm64::{Arm64Fp, Arm64Regs, NZCV};
use exec_core::{FpRegs, Isa, Layout, Options, PAGE, Prot, Regs};
use exec_translate::rewrite::{self, Exit};
use exec_translate::runtime::{Block, Cache};

use crate::ram;
use mach::*;
pub use stub::run_if_stub;
use stub::Stub as Process;

/// The host's page: what the stub maps and protects in.
const WINDOW: u64 = 16 << 10;
/// Programs live below this; the code cache is above it.
const SPACE_TOP: u64 = 0x6f00_0000_0000;
const CACHE: u64 = SPACE_TOP;
const CACHE_SIZE: u64 = 256 << 20;
const BLOCK_INSTRUCTIONS: usize = 64;
/// The message that wakes the exception loop to preempt.
const PREEMPT_ID: i32 = 0x5EED;

/// Guest pages in one window, by their place in it: their offset in guest memory and what the program may do.
type Window = [Option<(u64, Prot)>; 4];

struct Shared {
    stub: Process,
    ram: usize,
    windows: Mutex<BTreeMap<u64, Window>>,
    /// Windows allocated in the stub.
    allocated: Mutex<BTreeSet<u64>>,
}

impl Shared {
    fn window_prot(window: &Window) -> i32 {
        let (read, write) = window.iter().flatten().fold((false, false), |(r, w), (_, p)| (r || *p != Prot::NONE, w || p.write()));
        match (read, write) {
            (_, true) => VM_PROT_READ | VM_PROT_WRITE,
            (true, false) => VM_PROT_READ,
            _ => 0,
        }
    }

    /// Guest memory into the stub, before it runs.
    fn copy_in(&self) -> Result<(), String> {
        let task = self.stub.task;
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut allocated = self.allocated.lock().unwrap_or_else(PoisonError::into_inner);
        windows.retain(|base, window| {
            if window.iter().all(Option::is_none) {
                if allocated.remove(base) {
                    // SAFETY: a window this allocated in the stub.
                    unsafe { mach_vm_deallocate(task, *base, WINDOW) };
                }
                return false;
            }
            true
        });
        let mut bytes = vec![0u8; WINDOW as usize];
        for (&base, window) in windows.iter() {
            if allocated.insert(base) {
                let mut at = base;
                // SAFETY: a free window of the stub's address space below SPACE_TOP, which layout keeps programs in.
                check("allocating guest memory in the stub", unsafe { mach_vm_allocate(task, &mut at, WINDOW, VM_FLAGS_FIXED) })?;
            }
            for (i, page) in window.iter().enumerate() {
                let part = &mut bytes[i * PAGE as usize..(i + 1) * PAGE as usize];
                match page {
                    // SAFETY: a frame of guest memory, a page of it.
                    Some((offset, _)) => part.copy_from_slice(unsafe { std::slice::from_raw_parts((self.ram + *offset as usize) as *const u8, PAGE as usize) }),
                    None => part.fill(0),
                }
            }
            // SAFETY: a window of the stub's, opened for writing, written, then given the program's access.
            unsafe {
                check("opening a window", mach_vm_protect(task, base, WINDOW, 0, VM_PROT_READ | VM_PROT_WRITE))?;
                self.stub.write(base, &bytes)?;
                check("protecting a window", mach_vm_protect(task, base, WINDOW, 0, Self::window_prot(window)))?;
            }
        }
        Ok(())
    }

    /// The stub's writable guest pages back into guest memory, after it stops.
    fn copy_out(&self) {
        let windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut bytes = vec![0u8; WINDOW as usize];
        for (&base, window) in windows.iter() {
            if !window.iter().flatten().any(|(_, p)| p.write()) || self.stub.read(base, &mut bytes).is_err() {
                continue;
            }
            for (i, page) in window.iter().enumerate() {
                if let Some((offset, prot)) = page
                    && prot.write()
                {
                    let part = &bytes[i * PAGE as usize..(i + 1) * PAGE as usize];
                    // SAFETY: a frame of guest memory, a page of it, that the program may write.
                    unsafe { std::ptr::copy_nonoverlapping(part.as_ptr(), (self.ram + *offset as usize) as *mut u8, PAGE as usize) };
                }
            }
        }
    }

    /// Whether the program may run code at `addr`: only its own executable pages, never the stub's.
    fn executable(&self, addr: u64) -> bool {
        let windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let page = (addr % WINDOW / PAGE) as usize;
        windows.get(&(addr & !(WINDOW - 1))).and_then(|w| w[page]).is_some_and(|(_, prot)| prot.exec())
    }
}

/// The macOS sandbox backend.
pub struct Stub {
    shared: Arc<Shared>,
}

impl Sandbox for Stub {
    type Cpu = StubCpu;
    const NAME: &'static str = "macos";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Stub, String> {
        let stub = Process::spawn()?;
        let mut cache = CACHE;
        // SAFETY: a free region of the stub's, for rewritten code.
        check("making the stub's code cache", unsafe { mach_vm_allocate(stub.task, &mut cache, CACHE_SIZE, VM_FLAGS_FIXED) })?;
        Ok(Stub { shared: Arc::new(Shared { stub, ram: ram.ptr as usize, windows: Mutex::default(), allocated: Mutex::default() }) })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::Arm64]
    }

    fn reach(&self) -> Reach {
        Reach::Rewrite
    }

    fn layout(&self, isa: Isa) -> Layout {
        let kept = self.shared.stub.kept.iter().filter(|r| r.start < SPACE_TOP).cloned().collect();
        Layout { top: exec_core::isa::space_top(isa).min(SPACE_TOP), kept, gate: None }
    }

    fn max_cpus(&self) -> usize {
        1
    }

    fn mirror(&mut self, page: u64, ram_offset: Option<u64>, prot: Prot) -> Result<(), String> {
        let base = page & !(WINDOW - 1);
        if page >= SPACE_TOP || self.shared.stub.kept.iter().any(|r| r.start < base + WINDOW && base < r.end) {
            return Err(format!("{page:#x} is the stub's"));
        }
        let mut windows = self.shared.windows.lock().unwrap_or_else(PoisonError::into_inner);
        windows.entry(base).or_default()[(page % WINDOW / PAGE) as usize] = ram_offset.map(|offset| (offset, prot));
        Ok(())
    }

    fn new_cpu(&self, index: usize) -> Result<StubCpu, String> {
        if index != 0 {
            return Err("the macOS sandbox has one CPU".to_string());
        }
        StubCpu::new(self.shared.clone())
    }
}

pub struct StubCpu {
    shared: Arc<Shared>,
    thread: mach_port_t,
    /// Where the thread's exceptions come.
    port: mach_port_t,
    cache: Cache,
    exits: BTreeMap<u64, Exit>,
    next_code: u64,
    state: ThreadState,
    regs: Regs,
    fp: FpRegs,
    /// The program pc `regs` was given, to tell if the kernel moved it.
    reported_pc: u64,
    /// Where the program goes next, when that isn't where it stopped.
    entry: Option<u64>,
    started: bool,
    suspended: bool,
    /// The exception the thread is stopped at, to answer before it runs again.
    pending: Option<Header>,
    preempt: Arc<AtomicBool>,
}

impl StubCpu {
    fn new(shared: Arc<Shared>) -> Result<StubCpu, String> {
        let (me, task) = (task_self(), shared.stub.task);
        let (mut thread, mut port) = (MACH_PORT_NULL, MACH_PORT_NULL);
        // SAFETY: a new thread in the stub, suspended; a port for its exceptions, with a send right to it.
        unsafe {
            check("making the program's thread", thread_create(task, &mut thread))?;
            check("making an exception port", mach_port_allocate(me, MACH_PORT_RIGHT_RECEIVE, &mut port))?;
            check("making an exception port", mach_port_insert_right(me, port, port, MACH_MSG_TYPE_MAKE_SEND))?;
            check(
                "catching the program's exceptions",
                thread_set_exception_ports(thread, EXC_MASK_ALL_FAULTS, port, EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES, ARM_THREAD_STATE64),
            )?;
        }
        Ok(StubCpu {
            shared,
            thread,
            port,
            cache: Cache::default(),
            exits: BTreeMap::new(),
            next_code: CACHE,
            state: ThreadState::default(),
            regs: Regs::Arm64(Arm64Regs::default()),
            fp: FpRegs::Arm64(Arm64Fp::default()),
            reported_pc: 0,
            entry: None,
            started: false,
            suspended: false,
            pending: None,
            preempt: Arc::new(AtomicBool::new(false)),
        })
    }

    fn program_pc(&self, pc: u64) -> u64 {
        self.cache.program_pc(pc).unwrap_or(pc)
    }

    /// The program's registers into the thread's.
    fn apply_regs(&mut self) {
        let Regs::Arm64(r) = self.regs else { return };
        self.state.x.copy_from_slice(&r.x[..29]);
        self.state.fp = r.x[29];
        self.state.lr = r.x[30];
        self.state.sp = r.sp;
        self.state.cpsr = (self.state.cpsr & !(NZCV as u32)) | (r.nzcv & NZCV) as u32;
        if r.pc != self.reported_pc {
            self.entry = Some(r.pc);
        }
    }

    /// The thread's registers as the program's, with `pc` where it is in its own code.
    fn load_regs(&mut self) {
        let mut x = [0u64; 31];
        x[..29].copy_from_slice(&self.state.x);
        x[29] = self.state.fp;
        x[30] = self.state.lr;
        let pc = self.entry.unwrap_or_else(|| self.program_pc(self.state.pc));
        self.reported_pc = pc;
        self.regs = Regs::Arm64(Arm64Regs { x, sp: self.state.sp, pc, nzcv: u64::from(self.state.cpsr) & NZCV, tpidr: 0 });
        let mut neon = NeonState { q: [0; 32], fpsr: 0, fpcr: 0 };
        let mut count = count::<NeonState>();
        // SAFETY: an out-parameter of the size given, for a stopped thread.
        if unsafe { thread_get_state(self.thread, ARM_NEON_STATE64, (&raw mut neon).cast(), &mut count) } == KERN_SUCCESS {
            self.fp = FpRegs::Arm64(Arm64Fp { v: neon.q, fpsr: neon.fpsr, fpcr: neon.fpcr });
        }
    }

    fn reg(&self, reg: u8) -> u64 {
        match reg {
            0..=28 => self.state.x[reg as usize],
            29 => self.state.fp,
            30 => self.state.lr,
            _ => 0,
        }
    }

    /// The rewritten block for program address `pc`, made now if there isn't one.
    fn enter(&mut self, pc: u64) -> Result<u64, String> {
        if let Some(block) = self.cache.find(pc) {
            return Ok(block.at);
        }
        // Only the program's own executable pages, a page at a time, up to a block's worth.
        let mut bytes = Vec::new();
        let mut at = pc;
        while bytes.len() < BLOCK_INSTRUCTIONS * 4 && self.shared.executable(at) {
            let end = ((at & !(PAGE - 1)) + PAGE).min(pc + (BLOCK_INSTRUCTIONS * 4) as u64);
            let mut part = vec![0u8; (end - at) as usize];
            if self.shared.stub.read(at, &mut part).is_err() {
                break;
            }
            bytes.extend_from_slice(&part);
            at = end;
        }
        let block = rewrite::arm64(pc, &bytes, BLOCK_INSTRUCTIONS);
        let len = block.code.len() as u64;
        if self.next_code + len > CACHE + CACHE_SIZE {
            self.cache = Cache::default();
            self.exits.clear();
            self.next_code = CACHE;
        }
        let place = self.next_code;
        let (task, pages) = (self.shared.stub.task, (place & !(WINDOW - 1))..(place + len).next_multiple_of(WINDOW));
        // SAFETY: the stub's code cache, opened for writing, written, then made runnable and nothing else.
        unsafe {
            check("opening the code cache", mach_vm_protect(task, pages.start, pages.end - pages.start, 0, VM_PROT_READ | VM_PROT_WRITE))?;
            self.shared.stub.write(place, &block.code)?;
            check("sealing the code cache", mach_vm_protect(task, pages.start, pages.end - pages.start, 0, VM_PROT_READ | VM_PROT_EXECUTE))?;
        }
        self.next_code += len;
        for (offset, exit) in &block.exits {
            self.exits.insert(place + u64::from(*offset), *exit);
        }
        // A block that ran out of readable code isn't kept: the code may be there next time.
        if !matches!(block.exits.last(), Some((_, Exit::Fetch { .. }))) {
            let end = block.map.last().map_or(pc, |(_, last)| last + 4);
            self.cache.insert(Block { start: pc, end, at: place, len: len as u32, map: block.map });
        }
        Ok(place)
    }

    fn set_state(&self) -> Result<(), String> {
        // SAFETY: the program's thread, stopped, and a state of the size given.
        check("setting the program's registers", unsafe { thread_set_state(self.thread, ARM_THREAD_STATE64, (&raw const self.state).cast(), count::<ThreadState>()) })?;
        if let FpRegs::Arm64(fp) = &self.fp {
            let neon = NeonState { q: fp.v, fpsr: fp.fpsr, fpcr: fp.fpcr };
            // SAFETY: as above.
            unsafe { thread_set_state(self.thread, ARM_NEON_STATE64, (&raw const neon).cast(), count::<NeonState>()) };
        }
        Ok(())
    }

    /// Answers the exception the thread is stopped at, so it carries on.
    fn reply(&mut self) {
        let Some(request) = self.pending.take() else { return };
        let mut reply = ExceptionReply {
            header: Header { bits: MACH_MSG_TYPE_MOVE_SEND_ONCE, size: size_of::<ExceptionReply>() as u32, remote: request.remote, id: request.id + 100, ..Header::default() },
            ndr: NDR,
            result: KERN_SUCCESS,
        };
        // SAFETY: a complete reply, to the send-once right the request carried.
        unsafe { mach_msg(&mut reply.header, MACH_SEND_MSG, reply.header.size, 0, MACH_PORT_NULL, 0, MACH_PORT_NULL) };
    }

    /// Lets the thread go, from wherever it was held.
    fn release(&mut self) {
        if !self.started || self.suspended {
            // SAFETY: the program's thread, which Carbon made suspended or suspended itself.
            unsafe { thread_resume(self.thread) };
            self.started = true;
            self.suspended = false;
        }
        self.reply();
    }

    /// Ends a stop: the program's registers loaded, and its memory copied back.
    fn finish(&mut self, exit: SandboxExit) -> SandboxExit {
        self.load_regs();
        self.shared.copy_out();
        exit
    }

    /// Follows a block exit without the kernel, or turns it into what the kernel must see.
    fn follow(&mut self, exit: Exit) -> Option<SandboxExit> {
        let next = match exit {
            Exit::Jump(target) => target,
            Exit::Indirect { reg, link } => {
                let target = self.reg(reg);
                if let Some(link) = link {
                    self.state.lr = link;
                }
                target
            }
            Exit::Call { target, link } => {
                self.state.lr = link;
                target
            }
            Exit::Syscall { next } => {
                self.entry = Some(next);
                return Some(SandboxExit::Gate(Trap::Svc));
            }
            Exit::Unsupported { pc } => {
                self.entry = Some(pc);
                return Some(SandboxExit::Fault(HostFault { kind: FaultKind::Ill, addr: 0, access: Prot::NONE, pc }));
            }
            Exit::Fetch { addr, pc } => {
                self.entry = Some(pc);
                return Some(SandboxExit::Fault(HostFault { kind: FaultKind::Segv, addr, access: Prot::X, pc }));
            }
        };
        match self.enter(next) {
            Ok(at) => {
                self.state.pc = at;
                if self.set_state().is_err() {
                    return Some(SandboxExit::Failed(0x8001));
                }
                self.reply();
                None
            }
            Err(_) => Some(SandboxExit::Failed(0x8002)),
        }
    }

    /// What a fault at `pc` means.
    fn fault(&self, exception: i32) -> SandboxExit {
        let pc = self.program_pc(self.state.pc);
        let kind = match exception {
            EXC_BAD_INSTRUCTION => FaultKind::Ill,
            EXC_ARITHMETIC => FaultKind::DivZero,
            EXC_BREAKPOINT => FaultKind::Trap,
            EXC_BAD_ACCESS => FaultKind::Segv,
            _ => FaultKind::Segv,
        };
        let (mut info, mut count) = (ExceptionState::default(), count::<ExceptionState>());
        // SAFETY: an out-parameter of the size given.
        unsafe { thread_get_state(self.thread, ARM_EXCEPTION_STATE64, (&raw mut info).cast(), &mut count) };
        // Instruction aborts are fetches; a data abort says in WnR whether it wrote.
        let access = match info.esr >> 26 {
            0x20 | 0x21 => Prot::X,
            0x24 | 0x25 if info.esr & (1 << 6) != 0 => Prot::W,
            _ => Prot::R,
        };
        SandboxExit::Fault(HostFault { kind, addr: info.far, access, pc })
    }
}

impl SandboxCpu for StubCpu {
    fn regs(&mut self) -> &mut Regs {
        &mut self.regs
    }

    fn fp(&mut self) -> &mut FpRegs {
        &mut self.fp
    }

    fn resume(&mut self) -> SandboxExit {
        if self.shared.copy_in().is_err() {
            return SandboxExit::Failed(0x8000);
        }
        self.apply_regs();
        if let Some(pc) = self.entry.take() {
            match self.enter(pc) {
                Ok(at) => self.state.pc = at,
                Err(_) => return SandboxExit::Failed(0x8002),
            }
        }
        if self.set_state().is_err() {
            return SandboxExit::Failed(0x8001);
        }
        self.release();
        loop {
            let mut buffer = [0u32; 128];
            let header = buffer.as_mut_ptr().cast::<Header>();
            // SAFETY: room for an exception message or a preempt.
            let received = unsafe { mach_msg(header, MACH_RCV_MSG, 0, 512, self.port, 0, MACH_PORT_NULL) };
            if received != KERN_SUCCESS {
                return SandboxExit::Failed(0x8003);
            }
            // SAFETY: a header, as every message starts with.
            let id = unsafe { header.read() }.id;
            if id == PREEMPT_ID {
                if self.preempt.swap(false, Ordering::AcqRel) {
                    // SAFETY: the program's thread, held until the next run.
                    unsafe { thread_suspend(self.thread) };
                    self.suspended = true;
                    let mut count = count::<ThreadState>();
                    // SAFETY: an out-parameter of the size given.
                    unsafe { thread_get_state(self.thread, ARM_THREAD_STATE64, (&raw mut self.state).cast(), &mut count) };
                    return self.finish(SandboxExit::Preempted);
                }
                continue;
            }
            if id != MACH_EXCEPTION_RAISE {
                continue;
            }
            // SAFETY: mach_exception_raise's request, as the id says.
            let request = unsafe { buffer.as_ptr().cast::<ExceptionRequest>().read_unaligned() };
            // SAFETY: the thread and task rights the request brought, which Carbon already has.
            unsafe {
                mach_port_deallocate(task_self(), request.thread.name);
                mach_port_deallocate(task_self(), request.task.name);
            }
            self.pending = Some(request.header);
            let mut count = count::<ThreadState>();
            // SAFETY: an out-parameter of the size given, for a thread stopped at an exception.
            unsafe { thread_get_state(self.thread, ARM_THREAD_STATE64, (&raw mut self.state).cast(), &mut count) };
            let exception = request.exception;
            if exception == EXC_BREAKPOINT
                && let Some(exit) = self.exits.get(&self.state.pc).copied()
            {
                match self.follow(exit) {
                    None => continue,
                    Some(stop) => return self.finish(stop),
                }
            }
            let fault = self.fault(exception);
            return self.finish(fault);
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let (preempt, port) = (self.preempt.clone(), self.port);
        Arc::new(move || {
            preempt.store(true, Ordering::Release);
            let mut header = Header { bits: MACH_MSG_TYPE_COPY_SEND, size: size_of::<Header>() as u32, remote: port, id: PREEMPT_ID, ..Header::default() };
            // SAFETY: an empty message to the CPU's own exception port, waking its loop.
            unsafe { mach_msg(&mut header, MACH_SEND_MSG, header.size, 0, MACH_PORT_NULL, 0, MACH_PORT_NULL) };
        })
    }
}

impl Drop for StubCpu {
    fn drop(&mut self) {
        // SAFETY: Carbon's rights to the program's thread and its exception port; the stub's end takes the thread.
        unsafe {
            mach_port_deallocate(task_self(), self.thread);
            mach_port_mod_refs(task_self(), self.port, MACH_PORT_RIGHT_RECEIVE, -1);
        }
    }
}
