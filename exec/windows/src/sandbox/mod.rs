//! `sandbox` on Windows x86-64: the program runs natively on the main thread of a stub process that Carbon debugs.
//!
//! - **Memory**: guest RAM is a pagefile-backed section; core's mirror maps each guest page into the stub as a one-page
//!   view, at the program's own address, over a placeholder. Frames are 64 KiB apart in the section, as a view's offset
//!   must be. Pages that have to be where Windows already is (the thread's TEB, for `thread_block`) are copied in and
//!   out around every run instead.
//! - **System calls**: Windows has no way to send a user-mode `syscall` anywhere but its own kernel. atlas calls the
//!   gate instead (`Layout::gate`, `Reach::Breakpoint`), an `int3` the debugger catches. lambda's `syscall` needs
//!   x86 → x86 rewriting, which isn't built, so this sandbox runs atlas only.
//! - **Isolation, and its limits**: the stub runs with Carbon's token minus its privileges, at low integrity, alone in a
//!   job with no UI, killed when Carbon goes. But a program that executes `syscall` or `int 0x2e` itself makes a real
//!   NT system call in the stub, and only that token stops it. That's weaker than Linux's sandbox, whose seccomp filter
//!   allows nothing, and far weaker than `--isolation vm`.
//! - **Preempting**: the debugger waits in slices of a few milliseconds; asked to preempt, it suspends the thread.
//!
//! One CPU for now, the stub's main thread. Any thread a program creates is ended as soon as it appears.

mod process;
mod ram;
mod stub;
mod sys;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use exec_core::contract::{FaultKind, GuestRam, HostFault, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::Trap;
use exec_core::isa::x86::{X86Fp, X86Regs};
use exec_core::{FpRegs, Isa, Layout, Options, Prot, Regs};

use process::{Process, SPACE_TOP};
use sys::*;

/// How long the debugger waits for an event before it looks for a preempt.
const SLICE_MS: u32 = 2;

pub struct Stub {
    process: Arc<Process>,
}

impl Sandbox for Stub {
    type Cpu = StubCpu;
    const NAME: &'static str = "windows";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        ram::alloc(bytes)
    }

    fn open(ram: &GuestRam, _options: &Options) -> Result<Stub, String> {
        let process = Process::spawn(ram.handle as HANDLE, ram.ptr as u64)?;
        Ok(Stub { process: Arc::new(process) })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Breakpoint
    }

    fn layout(&self, isa: Isa) -> Layout {
        Layout { top: exec_core::isa::space_top(isa).min(SPACE_TOP), kept: self.process.kept.clone(), gate: Some(stub::GATE) }
    }

    fn max_cpus(&self) -> usize {
        1
    }

    fn mirror(&mut self, page: u64, ram_offset: Option<u64>, prot: Prot) -> Result<(), String> {
        self.process.mirror(page, ram_offset, prot)
    }

    /// The thread's own TEB: Windows owns gs's base, so a program's thread block has to be where Windows put it.
    fn thread_block(&mut self, size: u64) -> Option<u64> {
        (size <= 0x2000).then_some(self.process.teb)
    }

    fn new_cpu(&self, index: usize) -> Result<StubCpu, String> {
        if index != 0 {
            return Err("the Windows sandbox has one CPU".to_string());
        }
        let mut cpu = StubCpu {
            process: self.process.clone(),
            context: Context::new(),
            regs: Regs::X86(X86Regs::default()),
            fp: FpRegs::X86(X86Fp::default()),
            dirty: false,
            preempt: Arc::new(AtomicBool::new(false)),
        };
        cpu.load()?;
        Ok(cpu)
    }
}

pub struct StubCpu {
    process: Arc<Process>,
    /// The thread's context as Windows last gave it.
    context: Context,
    regs: Regs,
    fp: FpRegs,
    dirty: bool,
    preempt: Arc<AtomicBool>,
}

impl StubCpu {
    fn thread(&self) -> HANDLE {
        self.process.thread.0
    }

    fn load(&mut self) -> Result<(), String> {
        self.context = Context::new();
        // SAFETY: an out-parameter, for a thread stopped at an event or suspended.
        if unsafe { GetThreadContext(self.thread(), &mut self.context) } == 0 {
            return Err(format!("reading the stub's registers: {}", last_error()));
        }
        let c = &self.context;
        // gs's base is the thread's TEB, which Windows sets; fs's isn't used on x64.
        self.regs = Regs::X86(X86Regs { gpr: c.gpr, rip: c.rip, rflags: u64::from(c.eflags), fs_base: 0, gs_base: self.process.teb });
        self.fp = FpRegs::X86(X86Fp { fxsave: c.fxsave });
        self.dirty = false;
        Ok(())
    }

    fn store(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if let Regs::X86(x) = &self.regs {
            self.context.gpr = x.gpr;
            self.context.rip = x.rip;
            self.context.eflags = x.rflags as u32;
        }
        if let FpRegs::X86(fp) = &self.fp {
            self.context.fxsave = fp.fxsave;
            self.context.mxcsr = fp.mxcsr();
        }
        self.context.flags = CONTEXT_PROGRAM;
        // SAFETY: the context read from this thread, with the program's registers in it.
        if unsafe { SetThreadContext(self.thread(), &self.context) } == 0 {
            return Err(format!("setting the stub's registers: {}", last_error()));
        }
        self.dirty = false;
        Ok(())
    }

    /// Lets the thread go: from a suspend, from a debug event, or both.
    fn release(&self) {
        let mut state = self.process.lock();
        if std::mem::take(&mut state.suspended) {
            // SAFETY: the thread this suspended.
            unsafe { ResumeThread(self.thread()) };
        }
        if let Some((pid, tid)) = state.pending.take() {
            // SAFETY: the event it's stopped at, which Carbon has handled: the program never sees it.
            unsafe { ContinueDebugEvent(pid, tid, DBG_CONTINUE) };
        }
    }

    /// The next event that stops the program, dealing with the rest on the way. None if it was preempted instead.
    fn next_event(&mut self) -> Result<Option<DebugEvent>, SandboxExit> {
        let mut event = DebugEvent::new();
        loop {
            // SAFETY: an out-parameter for the next event, on the debugger's thread.
            if unsafe { WaitForDebugEventEx(&mut event, SLICE_MS) } == 0 {
                if !self.preempt.swap(false, Ordering::AcqRel) {
                    continue;
                }
                // SAFETY: the stub's thread, suspended until the next run.
                unsafe { SuspendThread(self.thread()) };
                self.process.lock().suspended = true;
                // An event that came just before the suspend is still the stop.
                // SAFETY: as above, without waiting.
                if unsafe { WaitForDebugEventEx(&mut event, 0) } == 0 {
                    return Ok(None);
                }
            }
            if event.pid != self.process.pid {
                // Another stub's, from before: not this program's.
                // SAFETY: continuing it, so it can go.
                unsafe { ContinueDebugEvent(event.pid, event.tid, DBG_CONTINUE) };
                continue;
            }
            match event.code {
                EXCEPTION_DEBUG_EVENT if event.tid == self.process.tid => {
                    self.process.lock().pending = Some((event.pid, event.tid));
                    return Ok(Some(event));
                }
                EXIT_PROCESS_DEBUG_EVENT => {
                    self.process.lock().pending = Some((event.pid, event.tid));
                    return Err(SandboxExit::Failed(0x6000));
                }
                CREATE_THREAD_DEBUG_EVENT => {
                    // A program has no business with threads of its own: CPUs are core's.
                    // SAFETY: the new thread's handle, which the event carries.
                    unsafe { TerminateThread(event.handle(), 0) };
                }
                LOAD_DLL_DEBUG_EVENT | CREATE_PROCESS_DEBUG_EVENT => {
                    // SAFETY: the file handle the event carries, which the debugger closes.
                    unsafe { CloseHandle(event.handle()) };
                }
                EXCEPTION_DEBUG_EVENT => {
                    // Another thread's exception: one of Windows' own threads. It isn't the program's.
                }
                _ => {}
            }
            // SAFETY: continuing the event just handled.
            unsafe { ContinueDebugEvent(event.pid, event.tid, DBG_CONTINUE) };
        }
    }

    /// What an exception means, with the thread's registers loaded.
    fn exit_for(&self, event: &DebugEvent) -> SandboxExit {
        let pc = self.context.rip;
        let fault = |kind, addr, access| SandboxExit::Fault(HostFault { kind, addr, access, pc });
        match event.exception_code() {
            STATUS_BREAKPOINT if event.exception_address() == stub::GATE => SandboxExit::Gate(Trap::Int2e),
            STATUS_BREAKPOINT => SandboxExit::Fault(HostFault { kind: FaultKind::Trap, addr: 0, access: Prot::NONE, pc: event.exception_address() }),
            STATUS_ACCESS_VIOLATION => {
                let access = match event.exception_parameter(0) {
                    1 => Prot::W,
                    8 => Prot::X,
                    _ => Prot::R,
                };
                fault(FaultKind::Segv, event.exception_parameter(1), access)
            }
            // `int 0x29`, `__fastfail`: its code is the first parameter.
            STATUS_STACK_BUFFER_OVERRUN => fault(FaultKind::FastFail, event.exception_parameter(0), Prot::NONE),
            STATUS_INTEGER_DIVIDE_BY_ZERO | STATUS_INTEGER_OVERFLOW => fault(FaultKind::DivZero, 0, Prot::NONE),
            STATUS_DATATYPE_MISALIGNMENT => fault(FaultKind::Bus, 0, Prot::NONE),
            STATUS_ILLEGAL_INSTRUCTION | STATUS_PRIVILEGED_INSTRUCTION => fault(FaultKind::Ill, 0, Prot::NONE),
            STATUS_SINGLE_STEP => SandboxExit::Preempted,
            other => SandboxExit::Failed(0x6100 | u64::from(other & 0xffff)),
        }
    }
}

impl SandboxCpu for StubCpu {
    fn regs(&mut self) -> &mut Regs {
        self.dirty = true;
        &mut self.regs
    }

    fn fp(&mut self) -> &mut FpRegs {
        self.dirty = true;
        &mut self.fp
    }

    fn resume(&mut self) -> SandboxExit {
        if self.store().is_err() {
            return SandboxExit::Failed(0x6200);
        }
        self.process.copy_in();
        self.release();
        let event = match self.next_event() {
            Ok(event) => event,
            Err(exit) => return exit,
        };
        if self.load().is_err() {
            return SandboxExit::Failed(0x6201);
        }
        self.process.copy_out();
        match event {
            Some(event) => self.exit_for(&event),
            None => SandboxExit::Preempted,
        }
    }

    fn preempt_handle(&self) -> RawPreempt {
        let preempt = self.preempt.clone();
        Arc::new(move || preempt.store(true, Ordering::Release))
    }
}
