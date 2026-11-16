//! Processes started from outside a running machine: `carbon exec`, as `docker exec` starts one in a running
//! container. The process has a console of its own: what it writes comes out as [`Event`]s, and what's typed or piped
//! goes in. With a terminal (`-t`) the console is one that edits its lines itself, as conhost does, for a client whose
//! terminal is raw; without one, it's pipes. Its environment is the machine's first process's, with the request's over
//! it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kernels_core::lock;

use crate::console::Stdio;
use crate::handles::{Handle, Handles};
use crate::{Stop, images, process};

/// What to start.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// The program and its arguments; a program named without a path is atlas's own in System32 (`cmd`), or one at
    /// the drive's root.
    pub argv: Vec<String>,
    /// `NAME=value`, over the machine's environment.
    pub env: Vec<String>,
    /// Its working directory; `C:\` if None.
    pub cwd: Option<String>,
    /// A console of its own, this many rows and columns: `-t`. Without one, its standard handles are pipes.
    pub tty: Option<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Out(Vec<u8>),
    Err(Vec<u8>),
    /// It ended, with this exit code; nothing comes after.
    Exit(i32),
}

/// A process started from outside, from outside.
pub struct Entered {
    process: Arc<process::Process>,
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    events: Mutex<mpsc::Receiver<Event>>,
}

impl Stop {
    /// Starts a process in the running machine, as `docker exec` does.
    pub fn enter(&self, spec: Spec) -> Result<Entered, String> {
        let k = lock(&self.0).as_ref().and_then(std::sync::Weak::upgrade).ok_or("the machine isn't running")?;
        let name = spec.argv.first().ok_or("exec needs a program")?.clone();
        // The program: as named, atlas's own in System32, or at the drive's root.
        let windows = {
            let disk = lock(&k.disk);
            crate::find_program(&disk, &name).ok_or_else(|| format!("exec: {name}: not found on the machine's disk or in System32"))?
        };
        let path = process::disk_path(&windows);
        let bytes = {
            let disk = lock(&k.disk);
            match disk.resolve(layers::ROOT, &path, true) {
                Ok(ino) => disk.contents(ino).map_err(|e| format!("exec: {windows}: {e}"))?.to_vec(),
                Err(_) => path
                    .to_ascii_lowercase()
                    .strip_prefix("/windows/system32/")
                    .and_then(|file| images::program(exec::Isa::host(), file))
                    .ok_or_else(|| format!("exec: {windows} isn't there"))?
                    .to_vec(),
            }
        };
        let isa = crate::images::pe::Pe::parse(&bytes).map_err(|_| format!("exec: {windows} isn't a Windows program"))?.isa;

        // Its console: input from a channel, output as events.
        let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>();
        let (events_tx, events) = mpsc::channel::<Event>();
        let input_rx = Mutex::new((input_rx, Vec::<u8>::new()));
        let read = Box::new(move |buffer: &mut [u8]| {
            let mut guard = lock(&input_rx);
            let (rx, left) = &mut *guard;
            if left.is_empty() {
                match rx.recv() {
                    Ok(data) => *left = data,
                    Err(_) => return 0,
                }
            }
            let n = buffer.len().min(left.len());
            buffer[..n].copy_from_slice(&left[..n]);
            left.drain(..n);
            n
        });
        let out = Mutex::new(events_tx.clone());
        let write = Box::new(move |stream: host::Stream, data: &[u8]| {
            let event = if stream == host::Stream::Err { Event::Err(data.to_vec()) } else { Event::Out(data.to_vec()) };
            let _ = lock(&out).send(event);
        });
        let tty = spec.tty.is_some();
        let stdio = Stdio::new(tty, tty, read, write);

        let mut vars = crate::environment::first(&k, isa);
        for var in &spec.env {
            if let Some((name, value)) = var.split_once('=') {
                vars.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
                vars.push((name.to_string(), value.to_string()));
            }
        }
        let mut handles = Handles::default();
        let std_handles = [handles.add_with(Handle::Input(stdio.clone()), true), handles.add_with(Handle::Console(stdio.clone(), host::Stream::Out), true), handles.add_with(Handle::Console(stdio.clone(), host::Stream::Err), true)];
        let mut argv = spec.argv.clone();
        argv[0] = windows.clone();
        let launch = process::Launch {
            path,
            bytes,
            command_line: process::command_line(&argv),
            environment: crate::environment::block(&vars),
            cwd: Some(spec.cwd.clone().unwrap_or_else(|| String::from("C:\\"))),
            handles,
            std_handles,
            suspended: false,
            parent: 0,
            token: None,
            console: stdio,
        };
        let (made, started) = mpsc::channel();
        let k2 = k.clone();
        kernels_core::thread::spawn(k.exec.clone(), "atlas exec".to_string(), move |cx| {
            let first = process::launch(&k2, cx, launch)?;
            let _ = made.send(first.process.clone());
            Ok(first)
        })?;
        let process = started.recv().map_err(|_| format!("exec: {windows} didn't start"))?;

        // Its end, after what it wrote.
        let waited = process.clone();
        std::thread::Builder::new()
            .name("atlas exec end".into())
            .spawn(move || {
                let (code, ended) = &waited.ended;
                let mut code = lock(code);
                while code.is_none() {
                    code = ended.wait(code).unwrap_or_else(|p| p.into_inner());
                }
                let _ = events_tx.send(Event::Exit(code.unwrap_or(0) as i32));
            })
            .map_err(|e| e.to_string())?;
        Ok(Entered { process, input: Mutex::new(Some(input_tx)), events: Mutex::new(events) })
    }
}

impl Entered {
    pub fn input(&self, data: &[u8]) {
        if let Some(tx) = lock(&self.input).as_ref() {
            let _ = tx.send(data.to_vec());
        }
    }

    /// Its input ends.
    pub fn close_input(&self) {
        lock(&self.input).take();
    }

    /// Its console's size changed: a console here is as big as it says when asked.
    pub fn resize(&self, _rows: u16, _cols: u16) {}

    /// A signal from outside: it's ended, as Ctrl+C's default handler (SIGINT) or `TerminateProcess` would.
    pub fn signal(&self, signal: i32) {
        const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
        self.process.end(if signal == 2 { STATUS_CONTROL_C_EXIT } else { 1 });
    }

    /// The next thing out of it, waiting up to `timeout`.
    pub fn next(&self, timeout: Duration) -> Option<Event> {
        lock(&self.events).recv_timeout(timeout).ok()
    }
}
