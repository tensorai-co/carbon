//! A machine's registries (`--registries`): its package managers answered at its gateway, as
//! `registry.carbon`, by the crate `registries-core`, with each request sent upstream as one checked call.
//!
//! Here's the join: a connection to the gateway's port 80 is a [`Pipe`], whose machine end is a `net::HostStream` and
//! whose other end a thread of the [`Server`]'s reads and writes as an ordinary stream. Nothing touches a host socket
//! until the server sends its call.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use net::{HostStream, Ready, Shutdown, Waker};
use registries_core::{Adapter, Server};

pub use registries_core::Registries;

use crate::error::{Error, Result};

/// The name a machine reaches its registries by.
pub const NAME: &str = "registry.carbon";
/// The gateway's port they answer on.
pub const PORT: u16 = 80;

/// The registries this Carbon has.
pub fn adapters() -> Vec<Arc<dyn Adapter>> {
    #[allow(unused_mut)]
    let mut all: Vec<Arc<dyn Adapter>> = Vec::new();
    #[cfg(feature = "apt")]
    all.push(registries_apt::Apt::adapter());
    #[cfg(feature = "npm")]
    all.push(registries_npm::Npm::adapter());
    #[cfg(feature = "pypi")]
    all.push(registries_pypi::Pypi::adapter());
    all
}

/// The services this Carbon has, as `--registries` takes them.
pub fn known() -> Vec<&'static str> {
    adapters().iter().map(|a| a.service()).collect()
}

/// `settings`' server, its upstreams asked what they serve: one that can't be reached, or doesn't serve a service
/// asked for, stops the machine here, before it starts.
pub(crate) fn open(settings: &Registries, hub: &hooks::Hub) -> Result<Arc<Server>> {
    let tls = images::client_config().map_err(Error::io)?;
    Server::open(settings, adapters(), tls, hub.clone()).map(Arc::new).map_err(Error::invalid)
}

/// Starts answering one connection from the machine, on a thread of its own, and gives the machine's end.
pub(crate) fn connect(server: &Arc<Server>, wake: Waker) -> Box<dyn HostStream> {
    let pipe = Arc::new(Pipe { state: Mutex::default(), changed: Condvar::new(), wake });
    let (server, end) = (server.clone(), ServerEnd(pipe.clone()));
    let spawned = std::thread::Builder::new().name("carbon registry".into()).spawn(move || server.serve(end));
    if spawned.is_err() {
        let mut state = pipe.lock();
        state.server_gone = true;
    }
    Box::new(MachineEnd(pipe))
}

/// How much each way holds before its writer waits.
const CAPACITY: usize = 256 << 10;

#[derive(Default)]
struct State {
    to_server: VecDeque<u8>,
    to_machine: VecDeque<u8>,
    /// The machine won't write again.
    machine_done: bool,
    /// The machine's end is gone.
    machine_gone: bool,
    /// The server's end is gone: what's left to read is all there is.
    server_gone: bool,
}

struct Pipe {
    state: Mutex<State>,
    /// Told when the server's end may go on.
    changed: Condvar,
    /// Told when the machine's end may go on.
    wake: Waker,
}

impl Pipe {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The machine's end: never waits, as `net` wants.
struct MachineEnd(Arc<Pipe>);

impl HostStream for MachineEnd {
    fn connected(&self) -> net::Result<bool> {
        Ok(true)
    }

    fn ready(&self) -> Ready {
        let s = self.0.lock();
        let write_open = !s.machine_done && !s.server_gone;
        Ready {
            read: !s.to_machine.is_empty() || s.server_gone,
            write: write_open && s.to_server.len() < CAPACITY,
            read_closed: s.server_gone && s.to_machine.is_empty(),
            hangup: s.server_gone && s.to_machine.is_empty() && !write_open,
            error: false,
        }
    }

    fn read(&self, buffer: &mut [u8]) -> net::Result<usize> {
        let n = {
            let mut s = self.0.lock();
            if s.to_machine.is_empty() {
                return if s.server_gone { Ok(0) } else { Err(net::Error::WouldBlock) };
            }
            let n = buffer.len().min(s.to_machine.len());
            for (b, byte) in buffer.iter_mut().zip(s.to_machine.drain(..n)) {
                *b = byte;
            }
            n
        };
        self.0.changed.notify_all();
        Ok(n)
    }

    fn write(&self, data: &[u8]) -> net::Result<usize> {
        let n = {
            let mut s = self.0.lock();
            if s.machine_done || s.server_gone {
                return Err(net::Error::BrokenPipe);
            }
            let room = CAPACITY.saturating_sub(s.to_server.len());
            if room == 0 {
                return Err(net::Error::WouldBlock);
            }
            let n = data.len().min(room);
            s.to_server.extend(&data[..n]);
            n
        };
        self.0.changed.notify_all();
        Ok(n)
    }

    fn shutdown(&self, how: Shutdown) {
        if matches!(how, Shutdown::Write | Shutdown::Both) {
            self.0.lock().machine_done = true;
            self.0.changed.notify_all();
        }
    }
}

impl Drop for MachineEnd {
    fn drop(&mut self) {
        {
            let mut s = self.0.lock();
            s.machine_done = true;
            s.machine_gone = true;
        }
        self.0.changed.notify_all();
    }
}

/// The server's end: an ordinary stream that waits.
struct ServerEnd(Arc<Pipe>);

impl Read for ServerEnd {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = {
            let mut s = self.0.lock();
            while s.to_server.is_empty() && !s.machine_done {
                s = self.0.changed.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            let n = buffer.len().min(s.to_server.len());
            for (b, byte) in buffer.iter_mut().zip(s.to_server.drain(..n)) {
                *b = byte;
            }
            n
        };
        (self.0.wake)();
        Ok(n)
    }
}

impl Write for ServerEnd {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let n = {
            let mut s = self.0.lock();
            while s.to_machine.len() >= CAPACITY && !s.machine_gone {
                s = self.0.changed.wait(s).unwrap_or_else(|p| p.into_inner());
            }
            if s.machine_gone {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the machine's end is gone"));
            }
            let n = data.len().min(CAPACITY - s.to_machine.len());
            s.to_machine.extend(&data[..n]);
            n
        };
        (self.0.wake)();
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ServerEnd {
    fn drop(&mut self) {
        self.0.lock().server_gone = true;
        (self.0.wake)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn until(what: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }

    #[test]
    fn the_pipe_carries_both_ways_and_ends() {
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let pipe = Arc::new(Pipe { state: Mutex::default(), changed: Condvar::new(), wake: Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }) });
        let machine = MachineEnd(pipe.clone());
        let mut server = ServerEnd(pipe.clone());
        // A server that echoes in capitals, until the machine's done writing.
        let echo = std::thread::spawn(move || {
            let mut buf = [0u8; 4];
            loop {
                let n = server.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                server.write_all(&buf[..n].to_ascii_uppercase()).unwrap();
            }
        });
        assert_eq!(machine.read(&mut [0; 8]), Err(net::Error::WouldBlock));
        assert_eq!(machine.write(b"hello world"), Ok(11));
        assert!(until(|| machine.ready().read));
        let mut got: Vec<u8> = Vec::new();
        while got.len() < 11 {
            let mut buf = [0u8; 16];
            match machine.read(&mut buf) {
                Ok(n) => got.extend(&buf[..n]),
                Err(net::Error::WouldBlock) => std::thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(got, b"HELLO WORLD");
        assert!(woken.load(Ordering::SeqCst) > 0);
        machine.shutdown(Shutdown::Write);
        echo.join().unwrap();
        // The server's end gone, the machine reads the end.
        assert!(until(|| machine.ready().hangup));
        assert_eq!(machine.read(&mut [0; 8]), Ok(0));
        assert_eq!(machine.write(b"x"), Err(net::Error::BrokenPipe));
    }

    #[test]
    fn a_server_waits_while_the_machine_is_slow_and_stops_when_its_gone() {
        let pipe = Arc::new(Pipe { state: Mutex::default(), changed: Condvar::new(), wake: Arc::new(|| {}) });
        let machine = MachineEnd(pipe.clone());
        let mut server = ServerEnd(pipe);
        let writer = std::thread::spawn(move || server.write_all(&vec![7u8; CAPACITY * 3]));
        assert!(until(|| machine.ready().read));
        let mut buf = vec![0u8; CAPACITY];
        assert_eq!(machine.read(&mut buf), Ok(CAPACITY));
        drop(machine);
        assert!(writer.join().unwrap().is_err());
    }
}
