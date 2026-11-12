//! Guest threads. Each is a host thread that owns its exec `Cpu` (a `Cpu` never leaves its thread), runs it, and deals
//! with each stop through its kernel's [`Guest`]. A thread that blocks (reading a pipe, waiting on a futex, running a
//! program's callback) blocks its own host thread, and nothing else.
//!
//! A thread's CPU is its seat, and a seat can be given up. A host gives a VM only so many vCPUs (64 on an M1), and a
//! program may have more threads than that, most of them waiting. So on a vm backend, when a thread wants a CPU and
//! none is free, a thread that's blocked in [`Waiter::wait`](crate::wait::Waiter::wait) is asked to give its up: it
//! saves its registers and space, drops its CPU, and goes on waiting. Whatever it does next with its CPU (a turn, or
//! running its program) takes one again first, waiting for one if it must, and puts its registers and space back.
//! Nothing changes while there are CPUs enough: no thread gives its seat up unless another is waiting for one.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use exec::{Context, Cpu, CpuHandle, Exec, Exit, Isolation, SpaceId, Turn};

use crate::lock;
use crate::wait::Waiter;

/// A machine's exec, shared by its guest threads.
pub type SharedExec = Arc<Mutex<Exec>>;

/// A guest thread's way to its CPU: run it, or take a turn with it. It owns the thread's seat for as long as it lives.
pub struct Cx<'c> {
    exec: &'c SharedExec,
    /// Inside `elsewhere`: the space each turn uses, and the CPU's own to go back to before the turn ends.
    away: Option<(SpaceId, SpaceId)>,
    /// A seat is its host thread's: `!Send`.
    _thread: PhantomData<*const ()>,
}

impl<'c> Cx<'c> {
    /// The guest thread on this host thread, seated on `cpu`.
    pub fn new(exec: &'c SharedExec, cpu: Cpu) -> Cx<'c> {
        let shares = lock(exec).backend().isolation == Isolation::Vm;
        let handle = Handle::default();
        handle.set(Some(cpu.handle()));
        let shared = Arc::new(SeatShared::default());
        if shares {
            SEATS.add(&shared);
        }
        let seat = Seat { exec: exec.clone(), cpu: Some(cpu), parked: None, handle, shared, shares };
        SEAT.with(|s| *s.borrow_mut() = Some(seat));
        Cx { exec, away: None, _thread: PhantomData }
    }

    /// A turn: exec's lock, held only while `f` runs. Nothing slow may happen in `f`, and it may take no other lock.
    pub fn turn<R>(&mut self, f: impl FnOnce(&mut Turn<'_>) -> R) -> R {
        let (exec, away) = (self.exec, self.away);
        with_cpu(|cpu| {
            let mut exec = lock(exec);
            let mut turn = exec.enter(cpu);
            match away {
                Some((there, home)) => {
                    turn.use_space(there);
                    let result = f(&mut turn);
                    turn.use_space(home);
                    result
                }
                None => f(&mut turn),
            }
        })
    }

    /// Runs `f` with every turn in `space`, another process's, as a kernel reads or writes another program's memory
    /// (`ptrace`, `process_vm_readv`). Each turn goes back to the CPU's own space before it ends, so nothing outside
    /// a turn ever sees the CPU in a space that isn't its own. The caller keeps `space` alive meanwhile.
    pub fn elsewhere<R>(&mut self, space: SpaceId, f: impl FnOnce(&mut Cx<'_>) -> R) -> R {
        let home = match self.away {
            Some((_, home)) => home,
            None => self.turn(|t| t.space().id()),
        };
        let before = self.away.replace((space, home));
        let result = f(self);
        self.away = before;
        result
    }

    /// Runs the program until it stops. No lock is held: other CPUs run at the same time.
    pub fn run(&mut self) -> Exit {
        with_cpu(|cpu| cpu.run())
    }

    /// A handle any thread can preempt this thread's CPU with, whichever CPU it's on.
    pub fn handle(&self) -> Handle {
        SEAT.with(|s| s.borrow().as_ref().map(|seat| seat.handle.clone()).unwrap_or_default())
    }

    pub fn exec(&self) -> &SharedExec {
        self.exec
    }
}

impl Drop for Cx<'_> {
    /// The thread's over: its CPU goes, and its seat is free for another.
    fn drop(&mut self) {
        let seat = SEAT.with(|s| s.borrow_mut().take());
        if let Some(seat) = seat {
            seat.handle.set(None);
            let shares = seat.shares;
            drop(seat);
            if shares {
                SEATS.freed();
            }
        }
    }
}

/// A preempt handle that follows its thread from CPU to CPU. One that comes while the thread has no CPU is kept, and
/// its next CPU is preempted as soon as it has it, so it stops before it runs.
#[derive(Clone, Default)]
pub struct Handle {
    inner: Arc<HandleInner>,
}

#[derive(Default)]
struct HandleInner {
    current: Mutex<Option<CpuHandle>>,
    pending: AtomicBool,
}

impl Handle {
    /// Makes its CPU's `run` return soon with `Stop::Preempted`, or the next time it runs.
    pub fn preempt(&self) {
        let current = lock(&self.inner.current);
        match current.as_ref() {
            Some(cpu) => cpu.preempt(),
            None => self.inner.pending.store(true, Ordering::SeqCst),
        }
    }

    fn set(&self, cpu: Option<CpuHandle>) {
        let mut current = lock(&self.inner.current);
        if let Some(cpu) = &cpu
            && self.inner.pending.swap(false, Ordering::SeqCst)
        {
            cpu.preempt();
        }
        *current = cpu;
    }
}

/// This host thread's guest thread's seat: its CPU, or what it was doing when it gave it up.
struct Seat {
    exec: SharedExec,
    cpu: Option<Cpu>,
    /// Off its CPU: its registers and space, to put back on the next one.
    parked: Option<(Context, SpaceId)>,
    handle: Handle,
    shared: Arc<SeatShared>,
    /// On a vm backend, where CPUs can be made again: it may give its up. A sandbox's can't move between threads.
    shares: bool,
}

thread_local! {
    static SEAT: RefCell<Option<Seat>> = const { RefCell::new(None) };
}

/// What other threads see of a seat: the waiter its thread is blocked on, while it's blocked with a CPU, so a thread
/// that wants one can ask it to give its up.
#[derive(Default)]
struct SeatShared {
    blocked: Mutex<Option<Blocked>>,
}

/// A thread's waiter, while it waits, and since when. The pointer's only used under its seat's `blocked` lock, which
/// the waiting thread takes to clear it before its wait returns, so the waiter outlives every use.
struct Blocked {
    waiter: *const Waiter,
    since: Instant,
}

// SAFETY: a `Waiter` is `Sync`; the pointer is only followed while the wait it belongs to is still going (above).
unsafe impl Send for Blocked {}

/// Every seat that may be asked for its CPU, and the threads waiting for one.
struct Seats {
    list: Mutex<Vec<Weak<SeatShared>>>,
    /// Threads waiting for a CPU now. While there are none, nothing gives a seat up.
    wanted: AtomicUsize,
    /// Seats given up or ended, so far: a waiting thread sees a new one and tries again.
    freed: Mutex<u64>,
    changed: Condvar,
}

static SEATS: Seats = Seats { list: Mutex::new(Vec::new()), wanted: AtomicUsize::new(0), freed: Mutex::new(0), changed: Condvar::new() };

/// How long a thread waiting for a CPU waits before it asks again, in case the seat it asked for went to another.
const RETRY: Duration = Duration::from_millis(20);

/// With `CARBON_TRACE` set, seats given up and taken are told on stderr, with the host thread's name.
fn trace(what: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("CARBON_TRACE").is_some()) {
        let me = std::thread::current();
        eprintln!("[seat] {}: {what} ({} wanted)", me.name().unwrap_or("?"), SEATS.wanted.load(Ordering::Relaxed));
    }
}

impl Seats {
    fn add(&self, seat: &Arc<SeatShared>) {
        let mut list = lock(&self.list);
        list.retain(|s| s.strong_count() > 0);
        list.push(Arc::downgrade(seat));
    }

    /// A seat was given up, or its thread ended.
    fn freed(&self) {
        *lock(&self.freed) += 1;
        self.changed.notify_all();
    }

    /// Asks the thread that's been blocked with a CPU longest to give it up: the least likely to want it back soon (a
    /// pool's idle thread, not a busy one between two quick waits). False if none is blocked with one.
    fn ask_one(&self) -> bool {
        let list: Vec<Arc<SeatShared>> = lock(&self.list).iter().filter_map(Weak::upgrade).collect();
        let mut by_age: Vec<(Instant, &Arc<SeatShared>)> = list.iter().filter_map(|seat| lock(&seat.blocked).as_ref().map(|b| (b.since, seat))).collect();
        by_age.sort_by_key(|(since, _)| *since);
        // The oldest that's still blocked as it's asked (one may have woken meanwhile).
        for (_, seat) in by_age {
            let mut blocked = lock(&seat.blocked);
            if let Some(Blocked { waiter, .. }) = blocked.take() {
                // SAFETY: still set, under its lock, so its wait hasn't returned (`Blocked`).
                unsafe { (*waiter).nudge() };
                return true;
            }
        }
        false
    }
}

/// Runs `f` with this thread's CPU, taking one again first if it gave its up.
fn with_cpu<R>(f: impl FnOnce(&mut Cpu) -> R) -> R {
    SEAT.with(|s| {
        let mut s = s.borrow_mut();
        let seat = s.as_mut().expect("a guest thread's CPU, on a thread that has none");
        if seat.cpu.is_none() {
            seat.sit_again();
        }
        f(seat.cpu.as_mut().expect("a CPU, just taken"))
    })
}

impl Seat {
    /// A CPU again, with the registers and space it had when it gave its last up.
    fn sit_again(&mut self) {
        let Some((context, space)) = self.parked.take() else { return };
        let mut cpu = match take_cpu(&self.exec, true) {
            Ok(cpu) => cpu,
            Err(why) => panic!("no CPU to go back to: {why:?}"),
        };
        {
            let mut exec = lock(&self.exec);
            let mut turn = exec.enter(&mut cpu);
            turn.use_space(space);
            turn.thread().restore(&context);
        }
        self.handle.set(Some(cpu.handle()));
        self.cpu = Some(cpu);
        trace("took a CPU again");
    }

    /// Gives its CPU up, keeping what it needs to go on later.
    fn stand_up(&mut self) {
        let Some(mut cpu) = self.cpu.take() else { return };
        let saved = {
            let mut exec = lock(&self.exec);
            let mut turn = exec.enter(&mut cpu);
            let space = turn.space().id();
            (turn.thread().save(), space)
        };
        self.handle.set(None);
        // Its backend CPU goes here, on its own thread, as hvf's must; its slot is then free.
        drop(cpu);
        self.parked = Some(saved);
        SEATS.freed();
        trace("gave its CPU up");
    }
}

/// A CPU for this thread, waiting for one to be free if `share` and there's none: threads blocked with one are asked to
/// give theirs up meanwhile.
fn take_cpu(exec: &SharedExec, share: bool) -> Result<Cpu, exec::Error> {
    loop {
        let seen = *lock(&SEATS.freed);
        match lock(exec).add_cpu() {
            Ok(cpu) => return Ok(cpu),
            Err(exec::Error::NoMoreCpus) if share => {}
            Err(why) => return Err(why),
        }
        SEATS.wanted.fetch_add(1, Ordering::SeqCst);
        let asked = SEATS.ask_one();
        trace(if asked { "waits for a CPU, having asked a blocked thread for one" } else { "waits for a CPU, with no blocked thread to ask" });
        let freed = lock(&SEATS.freed);
        if *freed == seen {
            drop(SEATS.changed.wait_timeout(freed, RETRY).unwrap_or_else(|p| p.into_inner()));
        }
        SEATS.wanted.fetch_sub(1, Ordering::SeqCst);
    }
}

/// While this thread is blocked: its CPU can be asked for. Made as a wait starts, dropped as it ends.
pub(crate) struct Blocking {
    active: bool,
}

impl Blocking {
    /// This thread starts waiting on `waiter`. If a thread is already waiting for a CPU, it gives its up at once;
    /// otherwise it's listed as one that could be asked to.
    pub(crate) fn begin(waiter: &Waiter) -> Blocking {
        let active = SEAT.with(|s| {
            // Borrowed already: it's waiting from inside its own CPU's use, which mustn't happen; it keeps its CPU.
            let Ok(mut s) = s.try_borrow_mut() else { return false };
            let Some(seat) = s.as_mut().filter(|seat| seat.shares && seat.cpu.is_some()) else { return false };
            if SEATS.wanted.load(Ordering::SeqCst) > 0 {
                seat.stand_up();
                return false;
            }
            *lock(&seat.shared.blocked) = Some(Blocked { waiter, since: Instant::now() });
            true
        });
        Blocking { active }
    }

    /// It was asked for its CPU while waiting: it gives it up, and goes on waiting.
    pub(crate) fn give_up(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        SEAT.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut()
                && let Some(seat) = s.as_mut()
            {
                *lock(&seat.shared.blocked) = None;
                seat.stand_up();
            }
        });
    }
}

impl Drop for Blocking {
    /// The wait's over: it can't be asked any more. (A CPU it gave up is taken again when it next needs one.)
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        SEAT.with(|s| {
            if let Ok(s) = s.try_borrow()
                && let Some(seat) = s.as_ref()
            {
                *lock(&seat.shared.blocked) = None;
            }
        });
    }
}

/// A guest thread's CPU time, in nanoseconds: running its program (user), and in its kernel dealing with a stop
/// (system). Its own thread adds to it; any thread may read it.
#[derive(Debug, Default)]
pub struct Times {
    user: AtomicU64,
    system: AtomicU64,
}

impl Times {
    pub fn add(&self, user: u64, system: u64) {
        self.user.fetch_add(user, Ordering::Relaxed);
        self.system.fetch_add(system, Ordering::Relaxed);
    }

    /// User and system time so far.
    pub fn get(&self) -> (u64, u64) {
        (self.user.load(Ordering::Relaxed), self.system.load(Ordering::Relaxed))
    }
}

/// What a guest thread does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Run,
    /// The thread is over: its host thread ends, and its CPU with it.
    End,
}

/// A kernel's side of one guest thread.
pub trait Guest {
    /// The CPU stopped: find out why with `cx.turn(|t| t.stop(exit))`, and deal with it.
    fn exit(&mut self, cx: &mut Cx<'_>, exit: Exit) -> Flow;

    /// Before it first runs: anything it must do first (a traced thread stops for its tracer).
    fn start(&mut self, _cx: &mut Cx<'_>) -> Flow {
        Flow::Run
    }
}

/// Runs a guest thread on this host thread until it ends.
pub fn run(cx: &mut Cx<'_>, guest: &mut impl Guest) {
    if guest.start(cx) == Flow::End {
        return;
    }
    loop {
        let exit = cx.run();
        if guest.exit(cx, exit) == Flow::End {
            return;
        }
    }
}

/// Starts a guest thread on a new host thread with a CPU of its own (waiting for one, on a vm backend, while every
/// CPU is taken). `setup` makes the thread's [`Guest`] there, taking turns and locks in the usual order. Returns once
/// it's started, with its preempt handle, or why it couldn't start.
pub fn spawn<G: Guest + 'static>(
    exec: SharedExec,
    name: String,
    setup: impl FnOnce(&mut Cx<'_>) -> Result<G, String> + Send + 'static,
) -> Result<(Handle, JoinHandle<()>), String> {
    let (started, wait) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let share = lock(&exec).backend().isolation == Isolation::Vm;
            let cpu = match take_cpu(&exec, share) {
                Ok(cpu) => cpu,
                Err(why) => {
                    let _ = started.send(Err(format!("no CPU for a new thread: {why:?}")));
                    return;
                }
            };
            let mut cx = Cx::new(&exec, cpu);
            let handle = cx.handle();
            match setup(&mut cx) {
                Ok(mut guest) => {
                    let _ = started.send(Ok(handle));
                    run(&mut cx, &mut guest);
                }
                Err(why) => {
                    let _ = started.send(Err(why));
                }
            }
        })
        .map_err(|e| format!("starting a host thread: {e}"))?;
    match wait.recv() {
        Ok(Ok(handle)) => Ok((handle, thread)),
        Ok(Err(why)) => Err(why),
        Err(_) => Err("the new thread ended before it started".to_string()),
    }
}
