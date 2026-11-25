//! A machine's standard streams, and the `host::Host` its kernel is given: this process's own stdin, stdout and
//! stderr, nothing, or pipes this program reads and writes; the machine's output passed through its hooks first.

use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use host::{CertificateTrust, CertificateUsage, FontFace, Host, HostFont, Stream};

/// Where one of a machine's standard streams goes, or comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stdio {
    /// This process's own: what `carbon run` does. For stdin, Docker's `-i`.
    #[default]
    Inherit,
    /// Nothing: stdin at its end from the start, output dropped.
    Null,
    /// A pipe this program has: [`crate::Running::take_stdin`], [`crate::Running::take_stdout`].
    Piped,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Piped stdin: what's been written and not yet read, whether it's ended, and whether a waiter's been woken; as
/// `host::Std` keeps the real stdin, so a kernel can wait on it with a timeout.
#[derive(Default)]
pub(crate) struct Feed {
    state: Mutex<(VecDeque<u8>, bool, bool)>,
    ready: Condvar,
    watchers: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

impl Feed {
    fn notify(&self) {
        self.ready.notify_all();
        let watchers = lock(&self.watchers).clone();
        for notify in watchers {
            notify();
        }
    }

    fn push(&self, data: &[u8]) {
        lock(&self.state).0.extend(data);
        self.notify();
    }

    fn close(&self) {
        lock(&self.state).1 = true;
        self.notify();
    }

    fn wait(&self, deadline: Option<Instant>) -> bool {
        let mut state = lock(&self.state);
        loop {
            if !state.0.is_empty() || state.1 {
                return true;
            }
            if state.2 {
                state.2 = false;
                return false;
            }
            match deadline {
                None => state = self.ready.wait(state).unwrap_or_else(|p| p.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    state = self.ready.wait_timeout(state, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
                }
            }
        }
    }

    fn wake(&self) {
        lock(&self.state).2 = true;
        self.ready.notify_all();
    }

    fn read(&self, buffer: &mut [u8]) -> usize {
        while !self.wait(None) {}
        let mut state = lock(&self.state);
        let n = buffer.len().min(state.0.len());
        for (b, byte) in buffer.iter_mut().zip(state.0.drain(..n)) {
            *b = byte;
        }
        n
    }
}

/// A running machine's stdin, when it's piped: what's written here, it reads. Dropping it (or [`Stdin::close`]) ends
/// it, as closing a pipe does.
pub struct Stdin(Arc<Feed>);

impl Stdin {
    /// Ends the machine's stdin: what's been written is still read, then its end.
    pub fn close(self) {}
}

impl io::Write for Stdin {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.push(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Stdin {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// A machine's or a process's piped output, as it comes: chunks as they were written, until it ends. It's also an
/// [`io::Read`], so `read_to_end` and `BufReader::lines` work.
pub struct Chunks {
    from: Receiver<Vec<u8>>,
    left: Vec<u8>,
}

impl Chunks {
    pub(crate) fn new(from: Receiver<Vec<u8>>) -> Chunks {
        Chunks { from, left: Vec::new() }
    }

    /// The next chunk, waiting for it; None at the end.
    pub fn next_chunk(&mut self) -> Option<Vec<u8>> {
        if !self.left.is_empty() {
            return Some(std::mem::take(&mut self.left));
        }
        self.from.recv().ok()
    }

    /// The next chunk within `timeout`: None if none came (see [`Chunks::is_ended`] for whether one ever will).
    pub fn next_timeout(&mut self, timeout: Duration) -> Option<Vec<u8>> {
        if !self.left.is_empty() {
            return Some(std::mem::take(&mut self.left));
        }
        self.from.recv_timeout(timeout).ok()
    }

    /// Everything until the end.
    pub fn collect_all(&mut self) -> Vec<u8> {
        let mut all = std::mem::take(&mut self.left);
        while let Ok(chunk) = self.from.recv() {
            all.extend(chunk);
        }
        all
    }
}

impl Iterator for Chunks {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Vec<u8>> {
        self.next_chunk()
    }
}

impl io::Read for Chunks {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.left.is_empty() {
            match self.from.recv() {
                Ok(chunk) => self.left = chunk,
                Err(_) => return Ok(0),
            }
        }
        let n = buffer.len().min(self.left.len());
        buffer[..n].copy_from_slice(&self.left[..n]);
        self.left.drain(..n);
        Ok(n)
    }
}

enum In {
    Inherit,
    Null,
    Pipe(Arc<Feed>),
}

enum Out {
    Inherit,
    Null,
    Pipe(Mutex<Sender<Vec<u8>>>),
}

/// The ends of a machine's piped streams that this program keeps.
#[derive(Default)]
pub(crate) struct Ends {
    pub stdin: Option<Stdin>,
    pub stdout: Option<Chunks>,
    pub stderr: Option<Chunks>,
}

/// The host a machine's kernel is given: this host's clocks, randomness, memory, fonts and certificate trust, with
/// its standard streams as asked, and what it writes passed through its hooks.
pub(crate) struct MachineHost {
    std: host::Std,
    stdin: In,
    stdout: Out,
    stderr: Out,
    tty: bool,
    hub: hooks::Hub,
}

impl MachineHost {
    /// The host for a machine with these streams; `tty` makes this terminal its console, where stdin and stdout are
    /// this process's and a terminal. The ends that are piped are given back.
    pub fn new(stdin: Stdio, stdout: Stdio, stderr: Stdio, tty: bool, hub: hooks::Hub) -> (MachineHost, Ends) {
        let mut ends = Ends::default();
        let stdin = match stdin {
            Stdio::Inherit => In::Inherit,
            Stdio::Null => In::Null,
            Stdio::Piped => {
                let feed = Arc::new(Feed::default());
                ends.stdin = Some(Stdin(feed.clone()));
                In::Pipe(feed)
            }
        };
        let out = |stdio: Stdio, end: &mut Option<Chunks>| match stdio {
            Stdio::Inherit => Out::Inherit,
            Stdio::Null => Out::Null,
            Stdio::Piped => {
                let (tx, rx) = mpsc::channel();
                *end = Some(Chunks::new(rx));
                Out::Pipe(Mutex::new(tx))
            }
        };
        let stdout_end = &mut ends.stdout;
        let stdout = out(stdout, stdout_end);
        let stderr = out(stderr, &mut ends.stderr);
        let tty = tty && matches!(stdin, In::Inherit) && matches!(stdout, Out::Inherit);
        let std = host::Std::stdio(matches!(stdin, In::Inherit), tty);
        (MachineHost { std, stdin, stdout, stderr, tty, hub }, ends)
    }

    /// Writes as the machine would, without its hooks: an answered first program's output.
    pub fn write_unhooked(&self, stream: Stream, data: &[u8]) {
        let out = if stream == Stream::Out { &self.stdout } else { &self.stderr };
        match out {
            Out::Inherit => self.std.write(stream, data),
            Out::Null => {}
            Out::Pipe(tx) => {
                let _ = lock(tx).send(data.to_vec());
            }
        }
    }

    /// Ends its piped output, so whoever reads it sees the end, as the machine's first process ends.
    pub fn close_output(&self) {
        for out in [&self.stdout, &self.stderr] {
            if let Out::Pipe(tx) = out {
                // A sender that goes nowhere takes this one's place, and this one is dropped.
                let (gone, _) = mpsc::channel();
                *lock(tx) = gone;
            }
        }
    }
}

impl Host for MachineHost {
    fn write(&self, stream: Stream, data: &[u8]) {
        if self.hub.wants(hooks::Kind::Output) {
            let which = if stream == Stream::Out { hooks::Stream::Stdout } else { hooks::Stream::Stderr };
            match self.hub.output(hooks::Output { stream: which, source: hooks::Source::Machine, bytes: data.to_vec() }) {
                hooks::Decision::Deny => return,
                hooks::Decision::Rewrite(rewritten) => return self.write_unhooked(stream, &rewritten.bytes),
                _ => {}
            }
        }
        self.write_unhooked(stream, data);
    }

    fn read(&self, buffer: &mut [u8]) -> usize {
        match &self.stdin {
            In::Inherit => self.std.read(buffer),
            In::Null => 0,
            In::Pipe(feed) => feed.read(buffer),
        }
    }

    fn wait_input(&self, timeout: Option<Duration>) -> bool {
        match &self.stdin {
            In::Inherit => self.std.wait_input(timeout),
            In::Null => true,
            In::Pipe(feed) => feed.wait(timeout.map(|t| Instant::now() + t)),
        }
    }

    fn wake(&self) {
        match &self.stdin {
            In::Inherit => self.std.wake(),
            In::Null => {}
            In::Pipe(feed) => feed.wake(),
        }
    }

    fn watch_input(&self, notify: Arc<dyn Fn() + Send + Sync>) {
        match &self.stdin {
            In::Inherit => self.std.watch_input(notify),
            In::Null => {}
            In::Pipe(feed) => lock(&feed.watchers).push(notify),
        }
    }

    fn wall(&self) -> u64 {
        self.std.wall()
    }
    fn monotonic(&self) -> u64 {
        self.std.monotonic()
    }
    fn random(&self, buffer: &mut [u8]) {
        self.std.random(buffer);
    }
    fn thread_cpu(&self) -> u64 {
        self.std.thread_cpu()
    }
    fn cpus(&self) -> usize {
        self.std.cpus()
    }
    fn memory_available(&self) -> u64 {
        self.std.memory_available()
    }
    fn is_terminal(&self) -> bool {
        self.tty && self.std.is_terminal()
    }
    fn set_raw(&self, raw: bool) {
        if self.tty {
            self.std.set_raw(raw);
        }
    }
    fn terminal_size(&self) -> Option<(u16, u16)> {
        if self.tty { self.std.terminal_size() } else { None }
    }
    fn check_certificates(&self, chain: &[&[u8]], usage: CertificateUsage, name: Option<&str>, at: Option<i64>) -> CertificateTrust {
        self.std.check_certificates(chain, usage, name, at)
    }
    fn fonts(&self) -> Vec<FontFace> {
        self.std.fonts()
    }
    fn open_font(&self, id: &str) -> Option<Arc<dyn HostFont>> {
        self.std.open_font(id)
    }
    fn open_font_bytes(&self, bytes: &[u8]) -> Option<Arc<dyn HostFont>> {
        self.std.open_font_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn piped_streams_carry_what_the_machine_reads_and_writes() {
        let hub = hooks::Hub::new();
        hub.on_output(hooks::Filter::any(), |o| o.replace(o.text().replace("secret", "******")));
        let (host, mut ends) = MachineHost::new(Stdio::Piped, Stdio::Piped, Stdio::Null, false, hub);
        let mut stdin = ends.stdin.take().unwrap();
        stdin.write_all(b"hello").unwrap();
        drop(stdin);
        let mut buffer = [0u8; 16];
        assert!(host.wait_input(Some(Duration::from_millis(10))));
        assert_eq!(host.read(&mut buffer), 5);
        assert_eq!(host.read(&mut buffer), 0);
        host.write(Stream::Out, b"my secret\n");
        host.write(Stream::Err, b"dropped");
        host.close_output();
        let mut out = String::new();
        ends.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!(out, "my ******\n");
        assert!(!host.is_terminal());
    }
}
