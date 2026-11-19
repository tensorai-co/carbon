//! CPUs: each belongs to the host thread that made it, and runs its program without the kernel's lock.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::CpuId;
use crate::contract::{RawPreempt, Sandbox, SandboxCpu, SandboxExit, Vm, VmCpu, VmExit};
use crate::shim::Shim;

/// What every thread can see of a CPU.
pub(crate) struct CpuShared {
    /// It's in its program now.
    pub running: AtomicBool,
    /// It has gone into its program since its last turn ended.
    pub ran: AtomicBool,
    /// Someone asked it to stop; it will, as soon as it runs.
    pub pending: AtomicBool,
    /// The TLB flushes it has caught up with.
    pub flushed: AtomicU64,
    /// Its thread has finished with it.
    pub gone: AtomicBool,
    /// When it last went into its program ([`crate::watch::now`]), for `CARBON_EXEC_WATCH`; 0 while it's out.
    pub entered: AtomicU64,
    /// The name of the thread that runs it, for the watch's reports.
    pub thread: std::sync::OnceLock<String>,
    /// For the watch: when it last came out for something other than a kick (a trap, a fault), the runs since, those
    /// that didn't go in at all, and when it last reported.
    pub progress: std::sync::Mutex<crate::watch::Progress>,
    pub preempt: RawPreempt,
}

impl CpuShared {
    pub fn preempt(&self) {
        self.pending.store(true, Ordering::SeqCst);
        if self.running.load(Ordering::SeqCst) {
            (self.preempt)();
        }
    }
}

pub(crate) enum CpuInner<V: Vm, S: Sandbox> {
    Vm { cpu: V::Cpu, shim: Arc<V::Shim>, data: <V::Shim as Shim>::CpuData },
    Sandbox { cpu: S::Cpu },
}

/// One CPU, owned by the thread that made it: it can't be sent or shared. Dropping it retires the CPU, and on a vm
/// backend its slot is free for the next CPU made.
pub struct Cpu<V: Vm, S: Sandbox> {
    pub(crate) index: usize,
    pub(crate) inner: CpuInner<V, S>,
    pub(crate) shared: Arc<CpuShared>,
    pub(crate) paused: Arc<AtomicBool>,
    pub(crate) generation: Arc<AtomicU64>,
    /// `!Send` and `!Sync`: hvf's vCPUs, for one, only work on their own thread.
    pub(crate) _thread: PhantomData<*const ()>,
    /// Last, so it's dropped last: the slot is only marked free once the backend's CPU (`inner`) is gone.
    pub(crate) _retire: Retire,
}

/// Marks a CPU gone when it's dropped.
pub(crate) struct Retire(pub Arc<CpuShared>);

impl Drop for Retire {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
        self.0.gone.store(true, Ordering::Release);
    }
}

/// What `run` came back with. Only `Turn::stop` can read it.
#[derive(Debug, Clone, Copy)]
pub struct Exit(pub(crate) RawExit);

#[derive(Debug, Clone, Copy)]
pub(crate) enum RawExit {
    /// It didn't run: someone had asked it to stop.
    Preempted,
    Vm(VmExit),
    Sandbox(SandboxExit),
}

impl<V: Vm, S: Sandbox> Cpu<V, S> {
    pub fn id(&self) -> CpuId {
        CpuId(self.index)
    }

    /// Runs the program until it needs its kernel. The one call made without the kernel's lock.
    pub fn run(&mut self) -> Exit {
        if self.shared.pending.swap(false, Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
            if crate::watch::on() {
                crate::watch::kicked(&self.shared, self.index, true, String::new);
            }
            return Exit(RawExit::Preempted);
        }
        self.shared.ran.store(true, Ordering::SeqCst);
        self.shared.running.store(true, Ordering::SeqCst);
        // Running first, then the checks: a preempt, pause or flush that comes after this sees it running and stops it.
        if self.shared.pending.swap(false, Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
            // It didn't go in after all: its registers are still as its last turn left them.
            self.shared.running.store(false, Ordering::SeqCst);
            self.shared.ran.store(false, Ordering::SeqCst);
            return Exit(RawExit::Preempted);
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let flush = self.shared.flushed.load(Ordering::Acquire) != generation;
        let watching = crate::watch::on();
        if watching {
            self.shared.thread.get_or_init(|| std::thread::current().name().unwrap_or("?").to_string());
            self.shared.entered.store(crate::watch::now(), Ordering::Release);
        }
        let raw = match &mut self.inner {
            CpuInner::Vm { cpu, shim, .. } => {
                // Caught up only once the flush is really on its way: a CPU that isn't where it can go back by way of
                // one stays behind, and flushes next time.
                if shim.before_run(cpu.regs(), flush) && flush {
                    self.shared.flushed.store(generation, Ordering::Release);
                }
                RawExit::Vm(cpu.run())
            }
            CpuInner::Sandbox { cpu } => RawExit::Sandbox(cpu.resume()),
        };
        self.shared.running.store(false, Ordering::SeqCst);
        if watching {
            let kicked = matches!(raw, RawExit::Preempted | RawExit::Vm(VmExit::Preempted) | RawExit::Sandbox(SandboxExit::Preempted));
            if kicked {
                let inner = &mut self.inner;
                crate::watch::kicked(&self.shared, self.index, false, || match inner {
                    CpuInner::Vm { cpu, .. } => format!("{:x?}", cpu.regs()),
                    CpuInner::Sandbox { cpu } => format!("{:x?}", cpu.regs()),
                });
            } else {
                crate::watch::progressed(&self.shared);
            }
            let entered = self.shared.entered.swap(0, Ordering::AcqRel);
            let stayed = crate::watch::now().saturating_sub(entered);
            if stayed >= crate::watch::LONG {
                let regs = match &mut self.inner {
                    CpuInner::Vm { cpu, .. } => format!("{:x?}", cpu.regs()),
                    CpuInner::Sandbox { cpu } => format!("{:x?}", cpu.regs()),
                };
                eprintln!(
                    "exec watch: cpu {} ({}) came out after {:.1}s in its program: {raw:x?}; {regs}",
                    self.index,
                    self.shared.thread.get().map_or("?", String::as_str),
                    stayed as f64 / 1e9
                );
            }
        }
        Exit(raw)
    }

    /// The host CPU time its program has had, in nanoseconds, where that isn't the calling thread's own: a sandbox's
    /// program runs in its stub, another process. None on a vm backend, where the thread that runs a CPU is the one
    /// whose time it uses, or where the host can't say.
    pub fn program_time(&self) -> Option<u64> {
        match &self.inner {
            CpuInner::Vm { .. } => None,
            CpuInner::Sandbox { cpu } => cpu.program_time(),
        }
    }

    /// A handle any thread can use.
    pub fn handle(&self) -> CpuHandle {
        CpuHandle { shared: self.shared.clone() }
    }
}

/// A CPU, as any thread may hold it.
#[derive(Clone)]
pub struct CpuHandle {
    shared: Arc<CpuShared>,
}

impl CpuHandle {
    /// Makes `run` return soon with `Stop::Preempted`, or the next time it runs.
    pub fn preempt(&self) {
        self.shared.preempt();
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }
}
