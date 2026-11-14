//! The object manager's waitable objects, and waiting on them: events, mutexes (mutants), semaphores, and threads and
//! processes, which are signalled when they end. Objects can have names, which `Open*` finds them by.
//!
//! As NT's dispatcher lock does, one lock covers every object's state while a wait checks and takes them, so a wait
//! for all of several objects takes them all at once or none.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::wait::{Waiter, Woke};

use crate::Kernel;
use crate::os::Reply;
use crate::process::Task;

pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_ABANDONED_0: u32 = 0x80;
pub const WAIT_IO_COMPLETION: u32 = 0xc0;
pub const WAIT_TIMEOUT: u32 = 0x102;
pub const WAIT_FAILED: u32 = 0xffff_ffff;
/// What [`Task::wait_or_wake`] gives when the thread was woken for something else (a message, for one).
pub const WOKEN: u32 = 0xffff_fffe;
pub const INFINITE: u32 = 0xffff_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Event { manual: bool },
    Mutant,
    Semaphore { max: i64 },
    /// A waitable timer: a notification timer (manual) stays set; a synchronization timer lets one wait through.
    Timer { manual: bool },
    /// A thread, by its ID.
    Thread(u64),
    /// A process, by its ID.
    Process(u64),
}

#[derive(Default)]
pub struct State {
    /// An event's.
    pub set: bool,
    /// A semaphore's count, a mutant's recursion count, or how many times a timer's been set (so an expiry from
    /// before it was set again, or cancelled, does nothing).
    pub count: i64,
    /// The thread that owns a mutant, or 0.
    pub owner: u64,
    /// A mutant whose owner ended without releasing it.
    pub abandoned: bool,
    /// A thread's or process's exit code, once it has ended.
    pub exit: Option<u32>,
    /// An ended process's creation and exit times (FILETIMEs), which its handles can still ask for.
    pub times: Option<(u64, u64)>,
    pub waiters: Vec<Arc<Waiter>>,
}

pub struct Object {
    pub kind: Kind,
    pub state: Mutex<State>,
    /// The descriptor it was made with or given since; None for its maker's token's default.
    pub security: Mutex<Option<crate::security::Descriptor>>,
}

impl Object {
    pub fn new(kind: Kind, state: State) -> Arc<Object> {
        Arc::new(Object { kind, state: Mutex::new(state), security: Mutex::new(None) })
    }

    /// Whether a wait by thread `tid` would be satisfied now.
    fn ready(&self, s: &State, tid: u64) -> bool {
        match self.kind {
            Kind::Event { .. } | Kind::Timer { .. } => s.set,
            Kind::Mutant => s.owner == 0 || s.owner == tid,
            Kind::Semaphore { .. } => s.count > 0,
            Kind::Thread(_) | Kind::Process(_) => s.exit.is_some(),
        }
    }

    /// Takes the object for thread `tid`: what a satisfied wait does. True if it was an abandoned mutant.
    fn take(&self, s: &mut State, tid: u64) -> bool {
        match self.kind {
            Kind::Event { manual: false } | Kind::Timer { manual: false } => s.set = false,
            Kind::Mutant => {
                s.owner = tid;
                s.count += 1;
                return std::mem::take(&mut s.abandoned);
            }
            Kind::Semaphore { .. } => s.count -= 1,
            _ => {}
        }
        false
    }

    /// Wakes everything waiting on it, to look again.
    pub fn wake(s: &mut State) {
        for w in s.waiters.drain(..) {
            w.wake();
        }
    }
}

/// A name as the namespace keeps it: `Local\X` is the session's `X`, and the machine has one session, whose
/// namespace `Global\X` is too (as sections' names are kept).
pub fn namespaced(name: &str) -> &str {
    name.strip_prefix("Local\\").or_else(|| name.strip_prefix("Global\\")).unwrap_or(name)
}

/// The object manager's part of the kernel.
#[derive(Default)]
pub struct Objects {
    /// The dispatcher lock: held while a wait checks and takes objects, and while anything signals one.
    dispatcher: Mutex<()>,
    names: Mutex<HashMap<String, Weak<Object>>>,
}

impl Objects {
    /// Runs `f` on an object's state with the dispatcher lock held, then wakes its waiters.
    pub fn signal<R>(&self, object: &Object, f: impl FnOnce(&mut State) -> R) -> R {
        let _dispatcher = lock(&self.dispatcher);
        let mut s = lock(&object.state);
        let result = f(&mut s);
        Object::wake(&mut s);
        result
    }

    /// The object called `name`, if it still exists.
    pub fn named(&self, name: &str) -> Option<Arc<Object>> {
        lock(&self.names).get(namespaced(name)).and_then(Weak::upgrade)
    }

    /// An object's name, if it has one (as the namespace keeps it: without `Local\` or `Global\`).
    pub fn name_of(&self, object: &Arc<Object>) -> Option<String> {
        lock(&self.names).iter().find(|(_, w)| w.upgrade().is_some_and(|o| Arc::ptr_eq(&o, object))).map(|(n, _)| n.clone())
    }

    /// Makes an object, or with a name already taken, gives the existing one (and true), if it's of the same kind.
    pub fn create(&self, name: Option<&str>, kind: Kind, state: State) -> Result<(Arc<Object>, bool), ()> {
        let Some(name) = name.map(namespaced) else { return Ok((Object::new(kind, state), false)) };
        let mut names = lock(&self.names);
        if let Some(existing) = names.get(name).and_then(Weak::upgrade) {
            return if std::mem::discriminant(&existing.kind) == std::mem::discriminant(&kind) { Ok((existing, true)) } else { Err(()) };
        }
        names.retain(|_, w| w.strong_count() > 0);
        let object = Object::new(kind, state);
        names.insert(name.to_string(), Arc::downgrade(&object));
        Ok((object, false))
    }
}

impl Task {
    /// Waits for one (`all` false) or all of `objects`, up to `timeout_ms`; alertable waits run queued APCs and
    /// return. Gives a `WAIT_*` value, or the reply that the thread or process is ending.
    pub fn wait_for(&mut self, cx: &mut Cx<'_>, objects: &[Arc<Object>], all: bool, timeout_ms: u32, alertable: bool) -> Result<u32, Reply> {
        self.wait_or_wake(cx, objects, all, timeout_ms, alertable, false)
    }

    /// The same wait, which with `woken` also ends (with [`WOKEN`]) when anything wakes the thread: what else it
    /// waits for, as `MsgWaitForMultipleObjects` waits for messages too, is the caller's to check.
    pub fn wait_or_wake(&mut self, cx: &mut Cx<'_>, objects: &[Arc<Object>], all: bool, timeout_ms: u32, alertable: bool, woken: bool) -> Result<u32, Reply> {
        let deadline = (timeout_ms != INFINITE).then(|| Instant::now() + Duration::from_millis(u64::from(timeout_ms)));
        let tid = self.tid;
        let objects_lock = &self.k.objects;
        let result = loop {
            self.waiter.reset();
            {
                let _dispatcher = lock(&objects_lock.dispatcher);
                let mut states: Vec<_> = objects.iter().map(|o| lock(&o.state)).collect();
                let ready: Vec<bool> = objects.iter().zip(&states).map(|(o, s)| o.ready(s, tid)).collect();
                let found = if all {
                    (!objects.is_empty() && ready.iter().all(|&r| r)).then(|| {
                        let abandoned = objects.iter().zip(states.iter_mut()).fold(false, |a, (o, s)| o.take(s, tid) | a);
                        if abandoned { WAIT_ABANDONED_0 } else { WAIT_OBJECT_0 }
                    })
                } else {
                    ready.iter().position(|&r| r).map(|i| {
                        let abandoned = objects[i].take(&mut states[i], tid);
                        (if abandoned { WAIT_ABANDONED_0 } else { WAIT_OBJECT_0 }) + i as u32
                    })
                };
                if let Some(found) = found {
                    for s in &mut states {
                        s.waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
                    }
                    drop(states);
                    for (i, o) in objects.iter().enumerate() {
                        if o.kind == Kind::Mutant && (all || found == i as u32 || found == WAIT_ABANDONED_0 + i as u32) {
                            lock(&self.control.owned).push(Arc::downgrade(o));
                        }
                    }
                    break Ok(found);
                }
                for s in &mut states {
                    if !s.waiters.iter().any(|w| Arc::ptr_eq(w, &self.waiter)) {
                        s.waiters.push(self.waiter.clone());
                    }
                }
            }
            if alertable && !lock(&self.control.apcs).is_empty() {
                break Ok(WAIT_IO_COMPLETION);
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                break Ok(WAIT_TIMEOUT);
            }
            let woke = self.waiter.wait(deadline);
            if woke == Woke::Interrupted && self.ending() {
                break Err(Reply::Ended);
            }
            if woken && deadline.is_none_or(|d| Instant::now() < d) {
                break Ok(WOKEN);
            }
        };
        {
            let _dispatcher = lock(&objects_lock.dispatcher);
            for o in objects {
                lock(&o.state).waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
            }
        }
        if result == Ok(WAIT_IO_COMPLETION) {
            self.run_apcs(cx)?;
        }
        result
    }

    /// Runs this thread's queued APCs, each an upcall.
    pub fn run_apcs(&mut self, cx: &mut Cx<'_>) -> Result<(), Reply> {
        loop {
            let next = lock(&self.control.apcs).pop_front();
            let Some((function, arguments)) = next else { return Ok(()) };
            self.upcall(cx, function, &arguments)?;
        }
    }
}

/// A thread's queue of APCs: a function and its arguments each (`QueueUserAPC`'s one, a timer's three).
pub type Apcs = Mutex<VecDeque<(u64, Vec<u64>)>>;

impl Kernel {
    /// Ends a thread's or process's object with `code`, waking what waits on it.
    pub(crate) fn exited(&self, object: &Object, code: u32) {
        self.objects.signal(object, |s| {
            s.exit.get_or_insert(code);
        });
    }
}
