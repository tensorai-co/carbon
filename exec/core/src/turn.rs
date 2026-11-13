//! A CPU's turn in the kernel, and what the kernel can do in it: spaces, mappings, registers, memory.

use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};

use exec_mmu::{Format, Frame, Native, PAGE, Prot};

use crate::contract::{Sandbox, SandboxCpu, SandboxExit, Vm, VmCpu, VmExit};
use crate::cpu::{Cpu, CpuInner, Exit, RawExit};
use crate::exec::{Core, Mode};
use crate::sandbox::{self, gate, layout};
use crate::shim::Shim;
use crate::translated::{self, REGION, Translated};
use crate::{
    Context, CpuId, Error, FpRegs, Isa, Layout, MemorySize, MemoryStats, Page, PageCensus, Pressure, Regs, Resident, SpaceId, SpaceUsage,
    Stop,
};

/// A CPU's turn in the kernel. Only the CPU's own thread can have one. Ending it settles.
pub struct Turn<'e, V: Vm, S: Sandbox> {
    pub(crate) core: &'e mut Core<V, S>,
    pub(crate) cpu: &'e mut Cpu<V, S>,
}

impl<V: Vm, S: Sandbox> Drop for Turn<'_, V, S> {
    fn drop(&mut self) {
        let (index, root) = (self.cpu.index, self.root());
        if self.core.translating(root)
            && let CpuInner::Vm { cpu, .. } = &mut self.cpu.inner
            && let Some(regs) = V::Shim::arm64(cpu.regs())
        {
            self.core.enter_translation(index, root, regs);
            self.core.note_resume(index, root, regs.elr);
        }
        self.cpu.shared.ran.store(false, Ordering::SeqCst);
        self.core.settle(index);
    }
}

impl<'e, V: Vm, S: Sandbox> Turn<'e, V, S> {
    fn root(&self) -> u64 {
        self.core.roots[self.cpu.index]
    }

    /// As the turn starts, in a translated space: moves the CPU off code the cache has moved on from. Only a CPU that
    /// hasn't run since its last turn: its registers are as that turn left them. One that has run has an exit to go
    /// through `stop` first, and its `elr` may not even say where it is until then (a preempt's doesn't).
    pub(crate) fn catch_up(&mut self) {
        let (index, root) = (self.cpu.index, self.root());
        if self.core.translating(root)
            && !self.cpu.shared.ran.load(Ordering::SeqCst)
            && let CpuInner::Vm { cpu, .. } = &mut self.cpu.inner
            && let Some(regs) = V::Shim::arm64(cpu.regs())
        {
            self.core.catch_up(index, root, regs.elr);
        }
    }

    pub fn cpu(&self) -> CpuId {
        CpuId(self.cpu.index)
    }

    /// What the exit means to the kernel, or `None` if core dealt with it and the CPU should just run again.
    pub fn stop(&mut self, exit: Exit) -> Option<Stop> {
        let root = self.root();
        let index = self.cpu.index;
        let core = &mut *self.core;
        let stop = match (exit.0, &mut self.cpu.inner, &mut core.mode) {
            (RawExit::Preempted, ..) => Stop::Preempted,
            (RawExit::Vm(exit), CpuInner::Vm { cpu, shim, data }, _) => {
                let stop = match exit {
                    VmExit::Trap { at } => shim.trap(&core.mmu, data, cpu.regs(), at),
                    VmExit::Preempted => shim.preempted(&core.mmu, data, cpu.regs()),
                    VmExit::Failed(code) => Stop::Internal { pc: 0, detail: code },
                };
                // Translated x86: block exits are core's, and the kernel sees x86 addresses.
                if core.translating(root)
                    && let Some(regs) = V::Shim::arm64(cpu.regs())
                {
                    core.translated_stop(index, root, regs, stop)?
                } else {
                    stop
                }
            }
            (RawExit::Sandbox(exit), CpuInner::Sandbox { cpu }, Mode::Sandbox { sandbox, mirror }) => {
                if mirror.broken.is_some() {
                    Stop::Internal { pc: cpu.regs().pc(), detail: 0x4000 }
                } else {
                    if let SandboxExit::Gate(_) = exit {
                        gate::arrived(sandbox.reach(), cpu.regs(), &core.mmu, root);
                    }
                    sandbox::stop(exit, cpu.regs())
                }
            }
            _ => Stop::Internal { pc: 0, detail: 0x4100 },
        };
        if core.debug {
            crate::debug::stop(core.backend(), self.cpu.index, &stop);
        }
        Some(stop)
    }

    /// A new, empty space for programs of this instruction set.
    pub fn new_space(&mut self, isa: Isa) -> Result<SpaceId, Error> {
        if !self.core.caps().isas.contains(&isa) {
            return Err(Error::Unsupported(isa));
        }
        let root = self.core.mmu.new_root()?;
        self.core.spaces.insert(root, isa);
        if isa.is_x86() && Isa::host() == Isa::Arm64 && matches!(self.core.mode, Mode::Vm { .. }) {
            self.core.translated.insert(root, Translated::new(isa));
        }
        Ok(SpaceId(root))
    }

    /// A copy of the current space, sharing its pages until either side writes.
    pub fn fork_space(&mut self) -> Result<SpaceId, Error> {
        let root = self.root();
        let isa = *self.core.spaces.get(&root).ok_or(Error::NoSpace)?;
        let copy = self.core.mmu.fork(root)?;
        self.core.spaces.insert(copy, isa);
        if self.core.translating(root) {
            // Its translations stay the parent's: the child starts with an empty code cache of its own, and its pages
            // with the access the kernel gave them.
            self.core.mmu.unmap(copy, REGION);
            self.core.unguard_copy(root, copy);
            self.core.translated.insert(copy, Translated::new(isa));
        }
        // The source's pages went read-only.
        self.core.changed(root, 0..u64::MAX);
        Ok(SpaceId(copy))
    }

    /// Frees a space. It can be the current one, as long as another is used before running.
    pub fn destroy_space(&mut self, id: SpaceId) {
        if self.core.spaces.remove(&id.0).is_none() {
            return;
        }
        self.core.mmu.free_root(id.0);
        self.core.translated.remove(&id.0);
        self.core.forget_resumes(id.0);
        for root in &mut self.core.roots {
            if *root == id.0 {
                *root = 0;
            }
        }
        if let Mode::Sandbox { mirror, .. } = &mut self.core.mode {
            mirror.forget(id.0);
        }
    }

    pub fn use_space(&mut self, id: SpaceId) {
        let index = self.cpu.index;
        if self.core.roots[index] == id.0 {
            return;
        }
        self.core.roots[index] = id.0;
        if let CpuInner::Vm { cpu, shim, .. } = &mut self.cpu.inner {
            cpu.set_root(shim.root_register(id.0));
            // Spaces have no ASIDs of their own yet, so a switch forgets everything cached.
            self.cpu.shared.flushed.store(u64::MAX, Ordering::Release);
        }
    }

    /// The current space.
    pub fn space(&mut self) -> Space<'_, V, S> {
        let root = self.root();
        Space { core: self.core, root }
    }

    /// This CPU's thread: its registers.
    pub fn thread(&mut self) -> Thread<'_, V, S> {
        let root = self.root();
        Thread { core: self.core, cpu: self.cpu, root }
    }

    pub fn memory(&mut self) -> Memory<'_, V, S> {
        Memory { core: self.core }
    }

    /// Makes another CPU stop soon, with `Stop::Preempted`.
    pub fn preempt(&mut self, cpu: CpuId) {
        self.core.preempt(cpu.0);
    }

    /// Whether every CPU is preempted now and then.
    pub fn time_slices(&mut self, on: bool) {
        self.core.ticker.set(on);
    }

    /// Makes every CPU forget cached translations, when the turn ends.
    pub fn flush_tlb(&mut self) {
        self.core.mmu.needs_flush = true;
    }

    /// For a CPU with nothing to run: settles, gives memory back, and ends the turn.
    pub fn tidy(self) {}
}

impl<V: Vm, S: Sandbox> Core<V, S> {
    /// A turn changed `range` of a space: a sandbox's stub must catch up.
    pub(crate) fn changed(&mut self, root: u64, range: Range<u64>) {
        if let Mode::Sandbox { mirror, .. } = &mut self.mode {
            mirror.changed(root, range);
        }
    }
}

/// The current space.
pub struct Space<'t, V: Vm, S: Sandbox> {
    core: &'t mut Core<V, S>,
    root: u64,
}

impl<V: Vm, S: Sandbox> Space<'_, V, S> {
    pub fn id(&self) -> SpaceId {
        SpaceId(self.root)
    }

    pub fn isa(&self) -> Option<Isa> {
        self.core.spaces.get(&self.root).copied()
    }

    /// Where programs may live.
    pub fn layout(&self) -> Layout {
        let isa = self.isa().unwrap_or(Isa::host());
        match &self.core.mode {
            Mode::Vm { .. } => {
                let mut layout = layout::vm(isa);
                if self.core.translating(self.root) {
                    layout.kept.push(REGION);
                }
                layout
            }
            Mode::Sandbox { sandbox, .. } => sandbox.layout(isa),
        }
    }

    /// Maps new pages: say what goes in them.
    pub fn map(&mut self, pages: Range<u64>, prot: Prot) -> Map<'_, V, S> {
        debug_assert!(pages.start.is_multiple_of(PAGE) && pages.end.is_multiple_of(PAGE));
        Map { core: &mut *self.core, root: self.root, pages, prot }
    }

    pub fn unmap(&mut self, pages: Range<u64>) {
        self.core.forget_code(self.root, pages.clone());
        self.core.mmu.unmap(self.root, pages.clone());
        self.core.changed(self.root, pages);
    }

    pub fn protect(&mut self, pages: Range<u64>, prot: Prot) {
        self.core.forget_code(self.root, pages.clone());
        self.core.mmu.protect(self.root, pages.clone(), prot);
        self.core.changed(self.root, pages);
    }

    /// Moves the pages in `[from, from + len)` to the same offsets from `to`, dropping what was there.
    pub fn move_pages(&mut self, from: u64, to: u64, len: u64) -> Result<(), Error> {
        self.core.forget_code(self.root, from..from + len);
        self.core.forget_code(self.root, to..to + len);
        self.core.mmu.move_pages(self.root, from, to, len)?;
        self.core.changed(self.root, from..from + len);
        self.core.changed(self.root, to..to + len);
        Ok(())
    }

    /// Gives this space its own copy of a shared page, as writable as its access allows. False if it wasn't shared.
    pub fn make_writable(&mut self, page: u64, prot: Prot) -> Result<bool, Error> {
        self.core.forget_code(self.root, page..page + PAGE);
        let done = self.core.mmu.make_writable(self.root, page, prot)?;
        self.core.changed(self.root, page..page + PAGE);
        Ok(done)
    }

    pub fn page(&self, page: u64) -> Option<Page> {
        let entry = self.core.mmu.descriptor(self.root, page);
        Native::is_valid(entry)
            .then(|| Page { prot: self.core.kernel_prot(self.root, page, Native::prot(entry)), shared: entry & Native::SHARED != 0 })
    }

    pub fn resident(&self, pages: Range<u64>) -> Resident {
        self.core.resident(self.root, pages)
    }

    pub fn usage(&self) -> SpaceUsage {
        self.core.usage(self.root)
    }

    /// Copies from program memory, whatever its access, which the kernel checks itself.
    pub fn read(&self, addr: u64, buffer: &mut [u8]) -> Result<(), Error> {
        let target = buffer.as_mut_ptr();
        self.core.mmu.copy(self.root, addr, buffer.len(), false, &mut |host, len, done| {
            // SAFETY: `len` bytes of a frame, into the part of the buffer at `done`.
            unsafe { std::ptr::copy_nonoverlapping(host, target.add(done), len) };
        })?;
        Ok(())
    }

    /// Copies to program memory. A shared page counts as unmapped, so the kernel can `make_writable` it first.
    pub fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), Error> {
        self.core.forget_code(self.root, addr..addr.saturating_add(data.len() as u64));
        let source = data.as_ptr();
        self.core.mmu.copy(self.root, addr, data.len(), true, &mut |host, len, done| {
            // SAFETY: `len` bytes of the data at `done`, into a frame.
            unsafe { std::ptr::copy_nonoverlapping(source.add(done), host, len) };
        })?;
        Ok(())
    }

    /// Reads an aligned value in one go, as other CPUs may be changing it.
    pub fn load_u32(&self, addr: u64) -> Result<u32, Error> {
        let at = self.core.mmu.value_at(self.root, addr, false)?;
        // SAFETY: aligned, as the kernel checks, and within guest memory.
        Ok(unsafe { AtomicU32::from_ptr(at.cast()) }.load(Ordering::SeqCst))
    }

    /// Changes an aligned value atomically, as other CPUs may be changing it, and returns the old value.
    pub fn update_u32(&mut self, addr: u64, mut update: impl FnMut(u32) -> u32) -> Result<u32, Error> {
        self.core.forget_code(self.root, addr..addr + 4);
        let at = self.core.mmu.value_at(self.root, addr, true)?;
        // SAFETY: as for load_u32.
        let value = unsafe { AtomicU32::from_ptr(at.cast()) };
        // A compare-and-swap loop: `fetch_update` is renamed in newer Rust, and this reads the same in both.
        let mut old = value.load(Ordering::SeqCst);
        loop {
            match value.compare_exchange_weak(old, update(old), Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Ok(old),
                Err(now) => old = now,
            }
        }
    }

    /// Where the next thread's environment block must go, on a host that decides that itself.
    pub fn thread_block(&mut self, size: u64) -> Option<u64> {
        match &mut self.core.mode {
            Mode::Sandbox { sandbox, .. } => sandbox.thread_block(size),
            Mode::Vm { .. } => None,
        }
    }
}

/// A mapping being made.
pub struct Map<'s, V: Vm, S: Sandbox> {
    core: &'s mut Core<V, S>,
    root: u64,
    pages: Range<u64>,
    prot: Prot,
}

impl<V: Vm, S: Sandbox> Map<'_, V, S> {
    fn each(self, mut fill: impl FnMut(u64, Frame, &Core<V, S>)) -> Result<(), Error> {
        self.core.forget_code(self.root, self.pages.clone());
        let mut va = self.pages.start;
        let result = loop {
            if va >= self.pages.end {
                break Ok(());
            }
            let frame = match self.core.mmu.new_frame() {
                Ok(frame) => frame,
                Err(e) => break Err(e.into()),
            };
            fill(va, frame, self.core);
            if let Err(e) = self.core.mmu.map(self.root, va, frame, self.prot, false, false) {
                self.core.mmu.release(frame);
                break Err(e.into());
            }
            va += PAGE;
        };
        self.core.changed(self.root, self.pages.start..va);
        result
    }

    /// Zeroed pages.
    pub fn zeroed(self) -> Result<(), Error> {
        self.each(|_, _, _| {})
    }

    /// Pages holding `data`, zero-filled after it. Other CPUs see each page only once it's complete.
    pub fn with(self, data: &[u8]) -> Result<(), Error> {
        let start = self.pages.start;
        self.each(|va, frame, core| {
            let offset = (va - start) as usize;
            if offset < data.len() {
                core.mmu.frame_write(frame, 0, &data[offset..data.len().min(offset + PAGE as usize)]);
            }
        })
    }

    /// The kernel's own frame, shared and writable as `MAP_SHARED` is. One page.
    pub fn frame(self, frame: Frame) -> Result<(), Error> {
        self.core.forget_code(self.root, self.pages.start..self.pages.start + PAGE);
        self.core.mmu.hold(frame);
        if let Err(e) = self.core.mmu.map(self.root, self.pages.start, frame, self.prot, false, true) {
            self.core.mmu.release(frame);
            return Err(e.into());
        }
        self.core.changed(self.root, self.pages.start..self.pages.start + PAGE);
        Ok(())
    }

    /// A frame holding what the page starts as, read-only until a write makes the kernel `make_writable` it. One page.
    pub fn copy_on_write(self, frame: Frame) -> Result<(), Error> {
        self.core.forget_code(self.root, self.pages.start..self.pages.start + PAGE);
        self.core.mmu.hold(frame);
        if let Err(e) = self.core.mmu.map(self.root, self.pages.start, frame, self.prot, true, false) {
            self.core.mmu.release(frame);
            return Err(e.into());
        }
        self.core.changed(self.root, self.pages.start..self.pages.start + PAGE);
        Ok(())
    }
}

/// This CPU's thread.
pub struct Thread<'t, V: Vm, S: Sandbox> {
    core: &'t mut Core<V, S>,
    cpu: &'t mut Cpu<V, S>,
    root: u64,
}

impl<V: Vm, S: Sandbox> Thread<'_, V, S> {
    fn isa(&self) -> Isa {
        self.core.spaces.get(&self.root).copied().unwrap_or(Isa::host())
    }

    fn translating(&self) -> bool {
        self.core.translating(self.root)
    }

    /// Starts a new thread at `entry` with `stack`, every other register cleared.
    pub fn start(&mut self, entry: u64, stack: u64) {
        let isa = self.isa();
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, shim, data } => {
                shim.start(&self.core.mmu, data, cpu.regs(), isa, entry, stack);
                *cpu.fp() = Default::default();
                if self.core.translating(self.root)
                    && let Some(r) = V::Shim::arm64(cpu.regs())
                {
                    // x86's stack is in x4, not the CPU's own; CF clear is ARM's carry set.
                    r.x = [0; 31];
                    r.x[4] = stack;
                    r.spsr = 1 << 29;
                    self.core.entries[self.cpu.index] = Some(entry);
                }
            }
            CpuInner::Sandbox { cpu } => {
                let mut regs = Regs::new(isa);
                regs.set_pc(entry);
                regs.set_sp(stack);
                *cpu.regs() = regs;
                *cpu.fp() = FpRegs::new(isa);
            }
        }
    }

    /// The result of the system call it stopped for, by its trap's convention.
    pub fn set_return(&mut self, value: u64) {
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, shim, .. } => shim.set_return(cpu.regs(), value),
            CpuInner::Sandbox { cpu } => cpu.regs().set_result(value),
        }
    }

    /// Where it's stopped, in its own code.
    pub fn program_pc(&mut self) -> u64 {
        self.regs().pc()
    }

    pub fn regs(&mut self) -> Regs {
        let (index, root, translating) = (self.cpu.index, self.root, self.translating());
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, shim, data } => {
                if translating && let Some(r) = V::Shim::arm64(cpu.regs()) {
                    let rip = self.core.x86_pc(index, root, r.elr);
                    return translated::x86_regs(r, rip, self.core.translating_x32(root));
                }
                shim.program(&self.core.mmu, data, cpu.regs())
            }
            CpuInner::Sandbox { cpu } => *cpu.regs(),
        }
    }

    pub fn set_regs(&mut self, regs: &Regs) {
        let (index, root, translating) = (self.cpu.index, self.root, self.translating());
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, shim, data } => {
                if translating
                    && let (Some(r), Regs::X86(x)) = (V::Shim::arm64(cpu.regs()), regs)
                {
                    // A new rip means a new translation, found as the turn ends.
                    if x.rip != self.core.x86_pc(index, root, r.elr) {
                        self.core.entries[index] = Some(x.rip);
                    }
                    translated::set_x86_regs(r, x, self.core.translating_x32(root));
                    return;
                }
                shim.set_program(&self.core.mmu, data, cpu.regs(), regs)
            }
            CpuInner::Sandbox { cpu } => *cpu.regs() = *regs,
        }
    }

    pub fn fp(&mut self) -> FpRegs {
        let translating = self.translating();
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, .. } => {
                if translating && let Some(fp) = V::Shim::arm64_fp(cpu.fp()) {
                    return translated::x86_fp(fp);
                }
                V::Shim::program_fp(cpu.fp())
            }
            CpuInner::Sandbox { cpu } => *cpu.fp(),
        }
    }

    pub fn set_fp(&mut self, regs: &FpRegs) {
        let translating = self.translating();
        match &mut self.cpu.inner {
            CpuInner::Vm { cpu, .. } => {
                if translating && let Some(fp) = V::Shim::arm64_fp(cpu.fp()) {
                    translated::set_x86_fp(fp, regs);
                    return;
                }
                V::Shim::set_program_fp(cpu.fp(), regs)
            }
            CpuInner::Sandbox { cpu } => *cpu.fp() = *regs,
        }
    }

    /// Everything, to resume this thread later, on any CPU.
    pub fn save(&mut self) -> Context {
        Context { regs: self.regs(), fp: self.fp() }
    }

    pub fn restore(&mut self, context: &Context) {
        self.set_regs(&context.regs);
        self.set_fp(&context.fp);
    }
}

/// The machine's memory as a whole.
pub struct Memory<'e, V: Vm, S: Sandbox> {
    pub(crate) core: &'e mut Core<V, S>,
}

impl<V: Vm, S: Sandbox> Memory<'_, V, S> {
    pub fn size(&self) -> MemorySize {
        MemorySize { limit: self.core.mmu.size(), available: self.core.mmu.frames.available_bytes() }
    }

    /// A zeroed frame for the kernel, which spaces can map.
    pub fn new_frame(&mut self) -> Result<Frame, Error> {
        Ok(self.core.mmu.new_frame()?)
    }

    /// The kernel's done with a frame. It lives on as long as anything maps it.
    pub fn free_frame(&mut self, frame: Frame) {
        self.core.mmu.release(frame);
    }

    pub fn frame_read(&self, frame: Frame, offset: usize, buffer: &mut [u8]) {
        self.core.mmu.frame_read(frame, offset, buffer);
    }

    pub fn frame_write(&mut self, frame: Frame, offset: usize, data: &[u8]) {
        self.core.mmu.frame_write(frame, offset, data);
    }

    /// Whether anything maps a frame, besides whoever holds it.
    pub fn frame_is_mapped(&self, frame: Frame) -> bool {
        self.core.mmu.refs(frame) > 1
    }

    /// Takes every copy-on-write mapping of these frames out of every space, so touching one faults again.
    pub fn unmap_frames(&mut self, mut wanted: impl FnMut(Frame) -> bool) {
        let roots: Vec<u64> = self.core.spaces.keys().copied().collect();
        for root in roots {
            let mut gone = Vec::new();
            self.core.mmu.for_each(root, 0..u64::MAX, &mut |mmu, va, slot| {
                // SAFETY: for_each gives entries in tables.
                let entry = unsafe { slot.read_volatile() };
                let frame = Frame(entry & Native::ADDRESS);
                if entry & Native::SHARED != 0 && entry & Native::KERNEL_FRAME == 0 && wanted(frame) {
                    // SAFETY: as above.
                    unsafe { slot.write_volatile(0) };
                    mmu.release(frame);
                    mmu.needs_flush = true;
                    gone.push(va);
                }
            });
            for va in gone {
                self.core.changed(root, va..va + PAGE);
            }
        }
    }

    pub fn pressure(&mut self, level: Pressure) {
        self.core.pressure(level);
    }

    pub fn stats(&self) -> MemoryStats {
        self.core.stats()
    }

    /// Looks at every mapped page, reading all of guest memory: only for when someone asks.
    pub fn census(&self) -> PageCensus {
        self.core.census()
    }
}
