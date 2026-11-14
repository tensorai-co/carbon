//! Threads: each a guest thread with a CPU, a TEB and a stack of its own, starting in ntdll's `AtlasThreadStart`.
//! Creating, suspending, resuming, terminating and exiting them, and what a process does when its threads end.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};

use exec::Prot;
use kernels_core::thread::Handle;
use kernels_core::lock;
use kernels_core::thread::{self, Cx};
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};

use crate::objects::{Apcs, Kind, Object, State};
use crate::process::{Area, AreaKind, GRANULE, Process, Task};

/// What a process knows of each of its threads, and what other threads use to reach it.
pub(crate) struct ThreadControl {
    pub tid: u64,
    pub teb: u64,
    pub object: Arc<Object>,
    pub waiter: Arc<Waiter>,
    pub cpu: Handle,
    /// How many times it's been suspended, less resumed: it runs only at 0.
    pub suspend: AtomicU32,
    /// Set by `TerminateThread`: it ends at its next chance, with this code.
    pub killed: Mutex<Option<u32>>,
    pub apcs: Apcs,
    /// Mutants it may own, to abandon if it ends holding them.
    pub owned: Mutex<Vec<Weak<Object>>>,
    /// The token it's impersonating, if it is.
    pub token: Mutex<Option<Arc<crate::security::Token>>>,
    /// `Wow64DisableWow64FsRedirection`: a 32-bit thread's System32 is System32, not SysWOW64, until it reverts.
    pub fs_redirection_off: std::sync::atomic::AtomicBool,
    /// Its registers, while another thread may look at them: from when it calls into atlas until it goes back to the
    /// program. Parked while suspended, its floating-point registers too; inside a call, those aren't read (it's
    /// costly on a VM), so `GetThreadContext` gives a new thread's.
    pub seen: Mutex<Option<(exec::Regs, Option<exec::FpRegs>)>>,
    /// Wakes those waiting for `seen`.
    pub seen_changed: std::sync::Condvar,
}

impl ThreadControl {
    pub fn new(tid: u64, teb: u64, waiter: Arc<Waiter>, cpu: Handle, suspended: bool) -> Arc<ThreadControl> {
        Arc::new(ThreadControl {
            tid,
            teb,
            object: Object::new(Kind::Thread(tid), State::default()),
            waiter,
            cpu,
            suspend: AtomicU32::new(u32::from(suspended)),
            killed: Mutex::new(None),
            apcs: Mutex::new(Default::default()),
            owned: Mutex::new(Vec::new()),
            token: Mutex::new(None),
            fs_redirection_off: std::sync::atomic::AtomicBool::new(false),
            seen: Mutex::new(None),
            seen_changed: std::sync::Condvar::new(),
        })
    }
}

/// A thread's stack, at least.
const STACK: u64 = 1 << 20;
/// How much of a new thread's stack is mapped up front.
const STACK_MAPPED: u64 = 64 << 10;
const CREATE_SUSPENDED: u32 = 4;

impl Task {
    /// `CreateThread`: a new thread at `start(parameter)`. Its handle's object, and its ID.
    pub fn create_thread(&mut self, cx: &mut Cx<'_>, stack_size: u64, start: u64, parameter: u64, flags: u32) -> Result<(Arc<Object>, u64), u32> {
        let tid = self.k.new_id();
        let size = stack_size.max(STACK).next_multiple_of(GRANULE);
        let (alloc, abi, thread_return) = (self.layout().alloc, self.abi(), self.layout().thread_return());
        let (teb_at, stack, peb, thread_start) = {
            let mut state = lock(&self.process.state);
            let teb_at = state.regions.free_above(alloc.clone(), GRANULE, GRANULE).ok_or(atlas_abi::status::NO_MEMORY)?;
            state.regions.insert(teb_at..teb_at + GRANULE, Area::of(Prot::RW, AreaKind::Committed));
            let stack = state.regions.free_above(alloc, size, GRANULE).ok_or(atlas_abi::status::NO_MEMORY)?;
            state.regions.insert(stack..stack + size, Area::of(Prot::RW, AreaKind::Committed));
            state.regions.update(stack..stack + crate::process::STACK_GUARD, |a| a.guard = true);
            (teb_at, stack, state.peb, state.thread_start)
        };
        let top = stack + size;
        let pid = self.process.pid;
        cx.turn(|t| -> Result<(), u32> {
            let nomem = |_| atlas_abi::status::NO_MEMORY;
            t.space().map(teb_at..teb_at + GRANULE, Prot::RW).zeroed().map_err(nomem)?;
            t.space().map(top - STACK_MAPPED..top, Prot::RW).zeroed().map_err(nomem)?;
            let mut memory = t.space();
            for (at, bytes) in abi.teb_fields(teb_at, top, top - STACK_MAPPED, stack, pid, tid, peb) {
                user::write(&mut memory, teb_at + at, &bytes).map_err(|_| atlas_abi::status::NO_MEMORY)?;
            }
            Ok(())
        })?;

        let (k, process, isa, convention) = (self.k.clone(), self.process.clone(), self.isa, self.convention());
        let suspended = flags & CREATE_SUSPENDED != 0;
        let (made, object) = std::sync::mpsc::channel();
        let started = thread::spawn(self.k.exec.clone(), format!("atlas {tid}"), move |cx| {
            let space = lock(&process.state).space;
            cx.turn(|t| {
                t.use_space(space);
                t.thread().start(thread_start, top);
                crate::process::windows_fp(t, isa);
                let mut regs = t.thread().regs();
                let (sp, frame) = convention.prepare(&mut regs, thread_start, &[start, parameter], thread_return);
                user::write(&mut t.space(), sp, &frame).map_err(|_| "no stack for a new thread".to_string())?;
                crate::abi::set_teb(&mut regs, isa, teb_at);
                t.thread().set_regs(&regs);
                Ok::<(), String>(())
            })?;
            let waiter = Waiter::new();
            let control = ThreadControl::new(tid, teb_at, waiter.clone(), cx.handle(), suspended);
            let _ = made.send(control.object.clone());
            lock(&process.state).threads.insert(tid, control.clone());
            Ok(Task { k, process, tid, waiter, isa, depth: 0, nested: 0, control, ending: None })
        });
        match started {
            Ok(_) => Ok((object.recv().map_err(|_| atlas_abi::status::NO_MEMORY)?, tid)),
            Err(why) => {
                self.k.host.write(host::Stream::Err, format!("atlas: CreateThread: {why}\n").as_bytes());
                Err(atlas_abi::status::NO_MEMORY)
            }
        }
    }

    /// Waits here while suspended, its registers where `GetThreadContext` can read them. False if it's to end instead.
    pub fn park_while_suspended(&mut self, cx: &mut Cx<'_>) -> bool {
        if self.control.suspend.load(Ordering::Acquire) != 0 {
            let state = cx.turn(|t| (t.thread().regs(), t.thread().fp()));
            *lock(&self.control.seen) = Some((state.0, Some(state.1)));
            self.control.seen_changed.notify_all();
        }
        loop {
            self.waiter.reset();
            if self.ending() {
                return false;
            }
            if self.control.suspend.load(Ordering::Acquire) == 0 {
                return true;
            }
            if self.waiter.wait(None) == Woke::Interrupted && self.ending() {
                return false;
            }
        }
    }

    /// Ends every thread of the process but this one, with `code`, as `NtTerminateProcess(NULL, code)` does before
    /// the loader tells DLLs the process is ending, and waits (a while) for them to be gone.
    pub fn end_other_threads(&mut self, code: u32) {
        let others: Vec<_> = lock(&self.process.state).threads.values().filter(|c| c.tid != self.tid).cloned().collect();
        for control in &others {
            lock(&control.killed).get_or_insert(code);
            control.waiter.interrupt();
            control.cpu.preempt();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while lock(&self.process.state).threads.len() > 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Whether this thread, or its whole process, is ending.
    pub fn ending(&self) -> bool {
        self.ending.is_some() || lock(&self.control.killed).is_some() || lock(&self.process.state).exiting.is_some()
    }

    /// This thread is over: it lets go of mutants it holds, its object is signalled, and if it was the process's last,
    /// the process is over too.
    pub fn finish_thread(&mut self, cx: &mut Cx<'_>) {
        let process_exit = lock(&self.process.state).exiting;
        let killed = *lock(&self.control.killed);
        let code = process_exit.or(killed).or(self.ending).unwrap_or(0);
        for mutant in lock(&self.control.owned).drain(..).filter_map(|w| w.upgrade()) {
            let tid = self.tid;
            self.k.objects.signal(&mutant, |s| {
                if s.owner == tid {
                    s.owner = 0;
                    s.count = 0;
                    s.abandoned = true;
                }
            });
        }
        self.k.exited(&self.control.object, code);
        if self.k.options.trace {
            self.k.host.write(host::Stream::Err, format!("[{}] the thread ends, {code:#x}\n", self.tid).as_bytes());
        }
        let last = {
            let mut state = lock(&self.process.state);
            state.threads.remove(&self.tid);
            state.threads.is_empty()
        };
        if last {
            let code = process_exit.unwrap_or(code);
            // What its views of file mappings wrote goes to their files.
            for (range, section, offset) in self.views() {
                self.write_back(cx, &section, offset..offset + (range.end - range.start));
            }
            // It lets go of every handle at once (a pipe's other end, in another process, sees it close), and its
            // memory.
            let (handles, space) = {
                let mut state = lock(&self.process.state);
                state.exiting.get_or_insert(code);
                (state.handles.take(), state.space)
            };
            drop(handles);
            let mut gone = lock(&self.process.remote);
            *gone = true;
            cx.turn(|t| t.destroy_space(space));
            drop(gone);
            let created = lock(&self.process.state).created;
            lock(&self.process.object.state).times = Some((created, crate::files::filetime(self.k.host.wall())));
            self.k.exited(&self.process.object, code);
            if self.k.options.trace {
                self.k.host.write(host::Stream::Err, format!("[{}] process {} ends, {code:#x}\n", self.tid, self.process.pid).as_bytes());
            }
            let (ended, done) = &self.process.ended;
            *lock(ended) = Some(code);
            done.notify_all();
            self.k.ended(self.process.pid);
            self.win32k().stations.forget(self.process.pid);
            self.win32k().forget_classes(self.process.pid);
            crate::os::dwrite::forget(self.process.pid);
        }
    }
}

impl Process {
    /// Starts ending the process: every thread ends at its next chance.
    pub fn end(&self, code: u32) {
        let mut state = lock(&self.state);
        state.exiting.get_or_insert(code);
        for control in state.threads.values() {
            control.waiter.interrupt();
            control.cpu.preempt();
        }
    }
}
