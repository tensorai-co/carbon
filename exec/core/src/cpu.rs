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
        if self.shared.pending.swap(false, Ordering::SeqCst) {
            return Exit(RawExit::Preempted);
        }
        self.shared.ran.store(true, Ordering::SeqCst);
        self.shared.running.store(true, Ordering::SeqCst);
        // Running first, then the checks: a preempt or a flush that comes after this sees it running and stops it.
        if self.shared.pending.swap(false, Ordering::SeqCst) {
            // It didn't go in after all: its registers are still as its last turn left them.
            self.shared.running.store(false, Ordering::SeqCst);
            self.shared.ran.store(false, Ordering::SeqCst);
            return Exit(RawExit::Preempted);
        }
        let generation = self.generation.load(Ordering::SeqCst);
        let flush = self.shared.flushed.load(Ordering::Acquire) != generation;
        let raw = match &mut self.inner {
            CpuInner::Vm { cpu, shim, .. } => {
                shim.before_run(cpu.regs(), flush);
                if flush {
                    self.shared.flushed.store(generation, Ordering::Release);
                }
                RawExit::Vm(cpu.run())
            }
            CpuInner::Sandbox { cpu } => RawExit::Sandbox(cpu.resume()),
        };
        self.shared.running.store(false, Ordering::SeqCst);
        Exit(raw)
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
