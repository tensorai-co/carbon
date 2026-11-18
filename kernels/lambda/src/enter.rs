//! Processes started from outside a running machine: `carbon exec`, as `docker exec` starts one in a running
//! container. The process is a session of its own with no parent in the machine (0), its environment and working
//! directory as it's given them, and for its descriptors 0, 1 and 2 either a new pty's slave, also its controlling
//! terminal (`-t`), or three pipes. What it writes comes out as [`Event`]s, its input, resizes and signals go in, and
//! its end is the last event.

use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::time::Duration;

use kernels_core::lock;
use kernels_core::wait::Waiter;

use crate::abi::*;
use crate::fs::{Files, Node, Open, Pipe};
use crate::tty::Tty;
use crate::{Kernel, Status, Stop, process};

/// What to start.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// The program and its arguments; a program named without a `/` is found on the environment's `PATH`.
    pub argv: Vec<String>,
    pub env: Vec<String>,
    /// Its working directory; `/` if None.
    pub cwd: Option<String>,
    /// A terminal of its own, this many rows and columns: `-t`. Without one, its stdin, stdout and stderr are pipes.
    pub tty: Option<(u16, u16)>,
    /// Who it runs as: the machine's first program's user if None.
    pub user: Option<crate::User>,
}

/// What comes out of a process started from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Out(Vec<u8>),
    Err(Vec<u8>),
    /// It ended, with this code (128 and the signal, if one ended it); nothing comes after.
    Exit(i32),
}

/// A process started from outside, from outside.
pub struct Entered {
    k: Weak<Kernel>,
    pid: i32,
    tty: Option<Arc<Tty>>,
    stdin: Option<Arc<Pipe>>,
    events: std::sync::Mutex<mpsc::Receiver<Event>>,
}

/// How long what a process wrote just before it ended has to come out after.
const DRAIN: Duration = Duration::from_millis(500);

impl Stop {
    /// Starts a process in the running machine, as `docker exec` does.
    pub fn enter(&self, spec: Spec) -> Result<Entered, String> {
        let k = self.kernel().ok_or("the machine isn't running")?;
        let argv0 = spec.argv.first().ok_or("exec needs a program")?.clone();
        let path = find(&k, &argv0, &spec.env).ok_or_else(|| format!("exec: {argv0:?}: no such file on the machine's PATH"))?;
        let cwd = {
            let dir = spec.cwd.clone().unwrap_or_else(|| "/".to_string());
            let disk = lock(&k.disk);
            let ino = disk.resolve(layers::ROOT, &dir, true).map_err(|_| format!("exec: the working directory {dir} isn't there"))?;
            (ino, dir)
        };
        let (events_in, events) = mpsc::channel();
        let pid = k.new_id();
        // `argv[0]` stays the name it was asked for (`bash`), as Docker's exec leaves it; `path` is what runs.
        let argv = spec.argv.clone();
        let user = spec.user.clone();
        // Its descriptors, and the other ends, ours.
        let (files, tty, stdin, outputs) = match spec.tty {
            Some((rows, cols)) => {
                let n = k.next_pty.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let tty = Tty::pty(n);
                lock(&k.ptys).insert(n, Arc::downgrade(&tty));
                tty.set_locked(false);
                let _ = tty.set_winsize([rows, cols, 0, 0]);
                tty.open_slave().map_err(|e| format!("exec: a terminal: {e:?}"))?;
                let open = Open::new(Node::Tty(tty.clone()), O_RDWR, format!("/dev/pts/{n}"));
                (Files::with_standard([open.clone(), open.clone(), open]), Some(tty), None, Vec::new())
            }
            None => {
                let (stdin, stdout, stderr) = (Pipe::new(), Pipe::new(), Pipe::new());
                let files = Files::with_standard([
                    Open::new(Node::Pipe(stdin.clone(), false), 0, "pipe:"),
                    Open::new(Node::Pipe(stdout.clone(), true), O_WRONLY, "pipe:"),
                    Open::new(Node::Pipe(stderr.clone(), true), O_WRONLY, "pipe:"),
                ]);
                (files, None, Some(stdin), vec![(stdout, false), (stderr, true)])
            }
        };
        let ended = Arc::new((std::sync::Mutex::new(None::<Status>), std::sync::Condvar::new()));
        lock(&k.entered).insert(pid, ended.clone());
        let started = {
            let mut env = spec.env.clone();
            k.own_env(&mut env);
            let (k, ctty) = (k.clone(), tty.clone());
            kernels_core::thread::spawn(k.exec.clone(), format!("lambda {pid}"), move |cx| {
                process::start_entered(&k, cx, pid, &path, &argv, &env, files, ctty, cwd, user.as_ref()).map_err(|e| format!("exec: {path}: {e:?}"))
            })
        };
        if let Err(why) = started {
            lock(&k.entered).remove(&pid);
            if let Some(tty) = &tty {
                tty.close_master();
            }
            return Err(why);
        }

        // What it writes: the pty's master, or its stdout and stderr pipes, each on a thread, until its end.
        let mut drained = Vec::new();
        if let Some(tty) = &tty {
            let (tty, events_in, (done, wait)) = (tty.clone(), events_in.clone(), mpsc::channel::<()>());
            drained.push(wait);
            let _ = std::thread::Builder::new().name(format!("exec {pid} out")).spawn(move || {
                let me = Waiter::new();
                while let Ok(data) = tty.read_master(&me, 64 << 10, false) {
                    if events_in.send(Event::Out(data)).is_err() {
                        break;
                    }
                }
                let _ = done.send(());
            });
        }
        for (pipe, err) in outputs {
            let (events_in, (done, wait)) = (events_in.clone(), mpsc::channel::<()>());
            drained.push(wait);
            let _ = std::thread::Builder::new().name(format!("exec {pid} out")).spawn(move || {
                let me = Waiter::new();
                while let Ok(data) = pipe.read(&me, 64 << 10, false) {
                    if data.is_empty() {
                        break;
                    }
                    let _ = events_in.send(if err { Event::Err(data) } else { Event::Out(data) });
                }
                pipe.close(false);
                let _ = done.send(());
            });
        }
        // Its end, once what it wrote is out (or has had a moment to be: a child of its may hold its output open).
        let _ = std::thread::Builder::new().name(format!("exec {pid} end")).spawn(move || {
            let (status, cond) = &*ended;
            let mut status = lock(status);
            while status.is_none() {
                status = cond.wait(status).unwrap_or_else(|p| p.into_inner());
            }
            let code = status.map_or(0, Status::code);
            drop(status);
            let deadline = std::time::Instant::now() + DRAIN;
            for wait in drained {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                let _ = wait.recv_timeout(left);
            }
            let _ = events_in.send(Event::Exit(code));
        });
        Ok(Entered { k: Arc::downgrade(&k), pid, tty, stdin, events: std::sync::Mutex::new(events) })
    }
}

impl Entered {
    /// Its process's ID in the machine.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Typed at its terminal, or written to its stdin.
    pub fn input(&self, data: &[u8]) {
        let Some(k) = self.k.upgrade() else { return };
        if let Some(tty) = &self.tty {
            for (pgid, signal) in tty.input(data) {
                k.signal_group(pgid, signal);
            }
        } else if let Some(stdin) = &self.stdin {
            let _ = stdin.write(&Waiter::new(), data, false);
        }
    }

    /// Its stdin ends: a pipe's write end closes; a terminal is sent its end-of-file character.
    pub fn close_input(&self) {
        if let Some(stdin) = &self.stdin {
            stdin.close(true);
        } else if self.tty.is_some() {
            self.input(&[0x04]);
        }
    }

    /// Its terminal's size changed.
    pub fn resize(&self, rows: u16, cols: u16) {
        if let (Some(k), Some(tty)) = (self.k.upgrade(), &self.tty)
            && let Some((pgid, signal)) = tty.set_winsize([rows, cols, 0, 0])
        {
            k.signal_group(pgid, signal);
        }
    }

    /// Sends `signal` to its process.
    pub fn signal(&self, signal: i32) {
        if let Some(k) = self.k.upgrade()
            && let Some(process) = k.process(self.pid)
        {
            k.signal_process(&process, signal);
        }
    }

    /// The next thing out of it, waiting up to `timeout`; None if nothing came by then.
    pub fn next(&self, timeout: Duration) -> Option<Event> {
        lock(&self.events).recv_timeout(timeout).ok()
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        if let Some(tty) = &self.tty {
            tty.close_master();
        }
        if let Some(stdin) = &self.stdin {
            stdin.close(true);
        }
    }
}

/// A program as `execvp` finds it: itself if it has a `/`, else the first file of its name on `env`'s `PATH`.
fn find(k: &Kernel, program: &str, env: &[String]) -> Option<String> {
    let disk = lock(&k.disk);
    if program.contains('/') {
        return disk.resolve(layers::ROOT, program, true).is_ok().then(|| program.to_string());
    }
    let path = env.iter().find_map(|e| e.strip_prefix("PATH=")).unwrap_or("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    path.split(':').filter(|dir| !dir.is_empty()).map(|dir| format!("{}/{program}", dir.trim_end_matches('/'))).find(|candidate| disk.resolve(layers::ROOT, candidate, true).is_ok())
}
