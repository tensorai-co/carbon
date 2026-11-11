//! The one implementation of `exec`, for any host's two backends.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use exec_mmu::{Mmu, Native, PAGE};

use crate::contract::{GuestRam, Sandbox, SandboxCpu, Vm, VmCpu};
use crate::cpu::{Cpu, CpuInner, CpuShared, Retire};
use crate::sandbox::Mirror;
use crate::shim::{CpuSetup, Shim};
use crate::ticker::Ticker;
use crate::translated::{self, Spaces};
use crate::turn::{Memory, Turn};
use crate::{Backend, Caps, Error, Isa, Isolation, OpenError, Options, Pressure, Tuning};

/// Where a sandbox's guest RAM starts, as frames name it. Nothing walks its tables but core, so any base would do.
const SANDBOX_BASE: u64 = 0x1_0000_0000;

/// An opened machine and its first CPU.
pub type Opened<V, S> = (Core<V, S>, Cpu<V, S>);

pub(crate) enum Mode<V: Vm, S: Sandbox> {
    Vm { vm: V, shim: Arc<V::Shim> },
    Sandbox { sandbox: S, mirror: Mirror },
}

/// A machine's memory and CPUs, on one of this host's two backends.
pub struct Core<V: Vm, S: Sandbox> {
    pub(crate) mode: Mode<V, S>,
    pub(crate) mmu: Mmu<Native>,
    pub(crate) ram: GuestRam,
    /// Every space, by its root, with its instruction set.
    pub(crate) spaces: BTreeMap<u64, Isa>,
    /// Each CPU's space, or 0.
    pub(crate) roots: Vec<u64>,
    /// Spaces whose x86 code is translated, by root.
    pub(crate) translated: Spaces,
    /// Per CPU, in a translated space: the x86 address it goes to next, when that isn't where it stopped.
    pub(crate) entries: Vec<Option<u64>>,
    /// Per CPU: where it last stopped for code it couldn't fetch, which its kernel has been told of.
    pub(crate) fetch_reported: Vec<Option<u64>>,
    /// Per CPU, a chain exit to link as its turn ends.
    pub(crate) chains: Vec<Option<crate::translated::Chain>>,
    /// Per CPU, where its last turn left it in a code cache.
    pub(crate) resume: Vec<Option<crate::translated::Resume>>,
    /// Per CPU slot on a vm backend, what the shim set it up with, for the next CPU in the slot.
    pub(crate) setups: Vec<Option<CpuSetup<V::Shim>>>,
    pub(crate) cpus: Arc<Mutex<Vec<Arc<CpuShared>>>>,
    /// TLB flushes so far: a CPU that's behind flushes before it next runs.
    pub(crate) generation: Arc<AtomicU64>,
    pub(crate) tuning: Tuning,
    pub(crate) keep_free: u64,
    pub(crate) give_back_min: u64,
    pub(crate) giving_back: bool,
    pub(crate) ticker: Ticker,
    pub(crate) debug: bool,
    backend: Backend,
    caps: Caps,
}

impl<V: Vm, S: Sandbox> Core<V, S> {
    /// Starts this host's backend for `options.isolation`, with its first CPU, which belongs to the calling thread.
    pub fn open(options: Options) -> Result<Opened<V, S>, OpenError> {
        let mut tried = Vec::new();
        if matches!(options.isolation, Isolation::Auto | Isolation::Vm) {
            match Self::open_vm(&options) {
                Ok(opened) => return Ok(opened),
                Err(why) => tried.push((V::NAME, why)),
            }
        }
        if matches!(options.isolation, Isolation::Auto | Isolation::Sandbox) {
            match Self::open_sandbox(&options) {
                Ok(opened) => return Ok(opened),
                Err(why) => tried.push((S::NAME, why)),
            }
        }
        Err(OpenError { wanted: options.isolation, tried })
    }

    fn open_vm(options: &Options) -> Result<Opened<V, S>, String> {
        let ram = V::alloc_ram(options.memory)?;
        let vm = V::open(&ram, options)?;
        let mut mmu = Mmu::new(ram.ptr, V::RAM_BASE, ram.size, ram.stride, ram.granule);
        let shim = V::Shim::install(&mut mmu, options).map_err(|_| "no room for the shim in guest memory".to_string())?;
        for frame in shim.code_frames() {
            vm.sync_code(mmu.host(frame), PAGE as usize);
        }
        let backend = Backend { isolation: Isolation::Vm, name: V::NAME };
        // An ARM64 backend runs x86-64 too, translated.
        let isas = if V::Shim::ISAS == [Isa::Arm64] { translated::TRANSLATING } else { V::Shim::ISAS };
        let caps = Caps { isas, max_cpus: vm.max_cpus().min(V::Shim::MAX_CPUS), tso: vm.tso() };
        let mode = Mode::Vm { vm, shim: Arc::new(shim) };
        Self::finish(mode, mmu, ram, options, backend, caps)
    }

    fn open_sandbox(options: &Options) -> Result<Opened<V, S>, String> {
        let ram = S::alloc_ram(options.memory)?;
        let sandbox = S::open(&ram, options)?;
        let mmu = Mmu::new(ram.ptr, SANDBOX_BASE, ram.size, ram.stride, ram.granule);
        let backend = Backend { isolation: Isolation::Sandbox, name: S::NAME };
        let caps = Caps { isas: sandbox.isas(), max_cpus: sandbox.max_cpus(), tso: false };
        let mode = Mode::Sandbox { sandbox, mirror: Mirror::default() };
        Self::finish(mode, mmu, ram, options, backend, caps)
    }

    fn finish(mode: Mode<V, S>, mmu: Mmu<Native>, ram: GuestRam, options: &Options, backend: Backend, caps: Caps) -> Result<Opened<V, S>, String> {
        let cpus = Arc::new(Mutex::new(Vec::new()));
        let mut core = Core {
            mode,
            mmu,
            ram,
            spaces: BTreeMap::new(),
            roots: Vec::new(),
            translated: Spaces::new(),
            entries: Vec::new(),
            fetch_reported: Vec::new(),
            chains: Vec::new(),
            resume: Vec::new(),
            setups: Vec::new(),
            ticker: Ticker::start(cpus.clone(), options.tuning.time_slice),
            cpus,
            generation: Arc::new(AtomicU64::new(0)),
            keep_free: options.tuning.keep_free,
            give_back_min: options.tuning.give_back_min,
            tuning: options.tuning.clone(),
            giving_back: true,
            debug: std::env::var_os("CARBON_EXEC_DEBUG").is_some(),
            backend,
            caps,
        };
        let cpu = core.add_cpu().map_err(|e| format!("making the first CPU: {e}"))?;
        Ok((core, cpu))
    }

    /// Which backend this is.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn caps(&self) -> &Caps {
        &self.caps
    }

    /// Another CPU, which belongs to the calling thread. On a vm backend it takes the slot of one that's been dropped,
    /// if there is one, so a machine can make CPUs for as long as it runs, as many at once as the backend has.
    pub fn add_cpu(&mut self) -> Result<Cpu<V, S>, Error> {
        let free = match self.mode {
            Mode::Vm { .. } => self.cpus.lock().unwrap_or_else(PoisonError::into_inner).iter().position(|c| c.gone.load(Ordering::Acquire)),
            // A sandbox's stub thread belongs to the host thread that drives it, so its slot can't move to another.
            Mode::Sandbox { .. } => None,
        };
        let index = free.unwrap_or(self.roots.len());
        if index >= self.caps.max_cpus {
            return Err(Error::NoMoreCpus);
        }
        let (inner, preempt) = match &self.mode {
            Mode::Vm { vm, shim } => {
                let mut cpu = vm.new_cpu(index).map_err(|_| Error::NoMoreCpus)?;
                // A slot's pages are the shim's to keep: a CPU made again in it has them again.
                let (system, regs, data) = match self.setups.get(index).cloned().flatten() {
                    Some(setup) => setup,
                    None => {
                        let setup = shim.add_cpu(&mut self.mmu, index)?;
                        self.setups.resize(index + 1, None);
                        self.setups[index] = Some(setup.clone());
                        setup
                    }
                };
                cpu.set_system(&system).map_err(|_| Error::NoMoreCpus)?;
                *cpu.regs() = regs;
                cpu.set_root(shim.root_register(0));
                let preempt = cpu.preempt_handle();
                (CpuInner::Vm { cpu, shim: shim.clone(), data }, preempt)
            }
            Mode::Sandbox { sandbox, .. } => {
                let cpu = sandbox.new_cpu(index).map_err(|_| Error::NoMoreCpus)?;
                let preempt = cpu.preempt_handle();
                (CpuInner::Sandbox { cpu }, preempt)
            }
        };
        let shared = Arc::new(CpuShared {
            running: AtomicBool::new(false),
            ran: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            // Behind, so it flushes before it first runs.
            flushed: AtomicU64::new(u64::MAX),
            gone: AtomicBool::new(false),
            preempt,
        });
        if free.is_some() {
            self.cpus.lock().unwrap_or_else(PoisonError::into_inner)[index] = shared.clone();
            self.roots[index] = 0;
            self.entries[index] = None;
            self.fetch_reported[index] = None;
            self.chains[index] = None;
            self.resume[index] = None;
        } else {
            self.cpus.lock().unwrap_or_else(PoisonError::into_inner).push(shared.clone());
            self.roots.push(0);
            self.entries.push(None);
            self.fetch_reported.push(None);
            self.chains.push(None);
            self.resume.push(None);
        }
        let retire = Retire(shared.clone());
        Ok(Cpu { index, inner, shared, generation: self.generation.clone(), _thread: PhantomData, _retire: retire })
    }

    /// A CPU's turn in the kernel, on its own thread. Ending it settles: TLBs synced, the sandbox's stub brought up to
    /// date, memory given back.
    pub fn enter<'e>(&'e mut self, cpu: &'e mut Cpu<V, S>) -> Turn<'e, V, S> {
        let mut turn = Turn { core: self, cpu };
        turn.catch_up();
        turn
    }

    /// The machine's memory as a whole.
    pub fn memory(&mut self) -> Memory<'_, V, S> {
        Memory { core: self }
    }

    /// Brings everything up to date after a turn's changes.
    pub(crate) fn settle(&mut self, index: usize) {
        match &mut self.mode {
            Mode::Vm { .. } => {
                if self.mmu.needs_flush {
                    self.mmu.needs_flush = false;
                    self.flush_others(index);
                }
            }
            Mode::Sandbox { sandbox, mirror } => {
                self.mmu.needs_flush = false;
                let root = self.roots.get(index).copied().unwrap_or(0);
                mirror.apply(sandbox, &self.mmu, root);
            }
        }
        self.give_back();
    }

    /// Every other CPU forgets its cached translations before it runs again. One that's running is stopped and waited
    /// for, so nothing can use a page that's gone once this returns.
    fn flush_others(&mut self, me: usize) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let cpus = self.cpus.lock().unwrap_or_else(PoisonError::into_inner).clone();
        for (index, cpu) in cpus.iter().enumerate() {
            if index == me || cpu.gone.load(Ordering::Acquire) || !cpu.running.load(Ordering::SeqCst) {
                continue;
            }
            (cpu.preempt)();
            while cpu.running.load(Ordering::SeqCst) && !cpu.gone.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        }
    }

    /// Gives free memory back to the host, past what's kept at hand.
    fn give_back(&mut self) {
        if !self.giving_back || self.mmu.needs_flush {
            return;
        }
        // Most turns end with nothing worth giving back: see so without looking at every granule.
        let frames = &self.mmu.frames;
        let could = frames.idle_bytes().min(frames.free_bytes().saturating_sub(self.keep_free));
        if could == 0 || could < self.give_back_min {
            return;
        }
        let runs = self.mmu.frames.excess(self.keep_free, self.tuning.give_back_batch);
        let total: u64 = runs.iter().map(|run| run.bytes).sum();
        if total == 0 || total < self.give_back_min {
            return;
        }
        let stride = self.ram.stride.max(PAGE);
        for run in runs {
            let host = self.mmu.host(run.start);
            if (self.ram.discard)(host, (run.bytes / PAGE * stride) as usize) {
                self.mmu.frames.returned(run);
            } else {
                self.giving_back = false;
                return;
            }
        }
    }

    pub(crate) fn pressure(&mut self, level: Pressure) {
        let calm = level == Pressure::Normal;
        self.keep_free = if calm { self.tuning.keep_free } else { 0 };
        self.give_back_min = if calm { self.tuning.give_back_min } else { 0 };
        self.give_back();
    }

    pub(crate) fn preempt(&self, cpu: usize) {
        if let Some(shared) = self.cpus.lock().unwrap_or_else(PoisonError::into_inner).get(cpu) {
            shared.preempt();
        }
    }
}
