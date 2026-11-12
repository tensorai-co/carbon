//! Guest threads. Each is a host thread that owns its exec `Cpu` (a `Cpu` never leaves its thread), runs it, and deals
//! with each stop through its kernel's [`Guest`]. A thread that blocks (reading a pipe, waiting on a futex, running a
//! program's callback) blocks its own host thread, and nothing else.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use exec::{Cpu, CpuHandle, Exec, Exit, SpaceId, Turn};

use crate::lock;

/// A machine's exec, shared by its guest threads.
pub type SharedExec = Arc<Mutex<Exec>>;

/// A guest thread's way to its CPU: run it, or take a turn with it.
pub struct Cx<'c> {
    exec: &'c SharedExec,
    cpu: &'c mut Cpu,
    /// Inside `elsewhere`: the space each turn uses, and the CPU's own to go back to before the turn ends.
    away: Option<(SpaceId, SpaceId)>,
}

impl<'c> Cx<'c> {
    pub fn new(exec: &'c SharedExec, cpu: &'c mut Cpu) -> Cx<'c> {
        Cx { exec, cpu, away: None }
    }

    /// A turn: exec's lock, held only while `f` runs. Nothing slow may happen in `f`, and it may take no other lock.
    pub fn turn<R>(&mut self, f: impl FnOnce(&mut Turn<'_>) -> R) -> R {
        let mut exec = lock(self.exec);
        let mut turn = exec.enter(self.cpu);
        match self.away {
            Some((there, home)) => {
                turn.use_space(there);
                let result = f(&mut turn);
                turn.use_space(home);
                result
            }
            None => f(&mut turn),
        }
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
        self.cpu.run()
    }

    /// A handle any thread can preempt this CPU with.
    pub fn handle(&self) -> CpuHandle {
        self.cpu.handle()
    }

    pub fn exec(&self) -> &SharedExec {
        self.exec
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

/// Starts a guest thread on a new host thread with a CPU of its own. `setup` makes the thread's [`Guest`] there, taking
/// turns and locks in the usual order. Returns once it's started, with its CPU's handle, or why it couldn't start.
pub fn spawn<G: Guest + 'static>(
    exec: SharedExec,
    name: String,
    setup: impl FnOnce(&mut Cx<'_>) -> Result<G, String> + Send + 'static,
) -> Result<(CpuHandle, JoinHandle<()>), String> {
    let (started, wait) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let cpu = lock(&exec).add_cpu();
            let mut cpu = match cpu {
                Ok(cpu) => cpu,
                Err(why) => {
                    let _ = started.send(Err(format!("no CPU for a new thread: {why:?}")));
                    return;
                }
            };
            let handle = cpu.handle();
            let mut cx = Cx::new(&exec, &mut cpu);
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
