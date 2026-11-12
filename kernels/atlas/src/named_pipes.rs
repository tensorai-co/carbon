//! Named pipes: `\\.\pipe\NAME`, made by a server (`CreateNamedPipe`, one instance a call) and opened by clients
//! (`CreateFile`). Each instance is listening, connected, broken (its client closed) or disconnected; a client takes a
//! listening one, and a connection is two pipes, one each way, byte or message. `ConnectNamedPipe` waits for a client
//! (or, overlapped, completes when one comes); `DisconnectNamedPipe` ends a client's connection, which then reads
//! ERROR_PIPE_NOT_CONNECTED; `WaitNamedPipe` waits for a listening instance. Names are the machine's, so processes
//! connect to each other's pipes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use kernels_core::lock;
use kernels_core::wait::Waiter;

use crate::Kernel;
use crate::pipes::{ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, Pipe};
use crate::ports::{Completion, ERROR_OPERATION_ABORTED, Op};

pub const ERROR_PIPE_BUSY: u32 = 231;
pub const ERROR_PIPE_CONNECTED: u32 = 535;
pub const ERROR_PIPE_LISTENING: u32 = 536;

/// A pipe name's instances, and what they were made with.
pub struct Server {
    pub message: bool,
    pub max: u32,
    pub out_size: usize,
    pub in_size: usize,
    /// `WaitNamedPipe`'s default, in milliseconds.
    pub timeout_ms: u32,
    pub instances: Mutex<Vec<Weak<Instance>>>,
    /// Threads in `WaitNamedPipe`.
    pub waiters: Mutex<Vec<Arc<Waiter>>>,
}

impl Server {
    pub fn live(&self) -> Vec<Arc<Instance>> {
        lock(&self.instances).iter().filter_map(Weak::upgrade).collect()
    }

    /// An instance is listening: whoever waits for one looks again.
    pub fn listening(&self) {
        for w in lock(&self.waiters).drain(..) {
            w.wake();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Listening,
    Connected,
    /// Its client closed: what it wrote can still be read, then ERROR_BROKEN_PIPE.
    Broken,
    /// `DisconnectNamedPipe`'d: listening again only once `ConnectNamedPipe` says so.
    Disconnected,
}

/// One instance, the server's end of it.
pub struct Instance {
    pub server: Arc<Server>,
    pub server_pid: u64,
    pub state: Mutex<InstanceState>,
}

pub struct InstanceState {
    pub phase: Phase,
    pub channel: Option<Arc<Channel>>,
    /// An overlapped `ConnectNamedPipe` waiting for a client.
    pub connect: Option<Arc<Op>>,
    pub client_pid: u64,
    /// Threads in a `ConnectNamedPipe` that waits.
    pub waiters: Vec<Arc<Waiter>>,
}

/// A connection: a pipe each way.
pub struct Channel {
    pub to_server: Arc<Pipe>,
    pub to_client: Arc<Pipe>,
}

/// One end of a named pipe, as an open file: the server's (no channel of its own: its instance's current one), or a
/// client's (the channel it connected on).
pub struct PipeEnd {
    pub k: Weak<Kernel>,
    pub instance: Arc<Instance>,
    pub channel: Option<Arc<Channel>>,
    /// `PIPE_READMODE_MESSAGE`, for this end.
    pub message_mode: AtomicBool,
    /// `PIPE_NOWAIT`.
    pub nowait: AtomicBool,
}

impl PipeEnd {
    pub fn is_server(&self) -> bool {
        self.channel.is_none()
    }

    pub fn message_mode(&self) -> bool {
        self.message_mode.load(Ordering::Relaxed)
    }

    pub fn nowait(&self) -> bool {
        self.nowait.load(Ordering::Relaxed)
    }

    /// The connection this end is on, or why there's none.
    fn channel(&self) -> Result<Arc<Channel>, u32> {
        let s = lock(&self.instance.state);
        match &self.channel {
            Some(mine) => match &s.channel {
                Some(now) if Arc::ptr_eq(now, mine) => Ok(mine.clone()),
                // The server disconnected it: what's left of it reads ERROR_PIPE_NOT_CONNECTED.
                _ => Ok(mine.clone()),
            },
            None => match (&s.channel, s.phase) {
                (Some(c), _) => Ok(c.clone()),
                (None, Phase::Listening) => Err(ERROR_PIPE_LISTENING),
                (None, _) => Err(ERROR_PIPE_NOT_CONNECTED),
            },
        }
    }

    /// The pipe this end reads.
    pub fn incoming(&self) -> Result<Arc<Pipe>, u32> {
        let c = self.channel()?;
        Ok(if self.is_server() { c.to_server.clone() } else { c.to_client.clone() })
    }

    /// The pipe this end writes.
    pub fn outgoing(&self) -> Result<Arc<Pipe>, u32> {
        if let Some(mine) = &self.channel {
            let s = lock(&self.instance.state);
            if !s.channel.as_ref().is_some_and(|now| Arc::ptr_eq(now, mine)) {
                return Err(ERROR_PIPE_NOT_CONNECTED);
            }
        }
        let c = self.channel()?;
        Ok(if self.is_server() { c.to_client.clone() } else { c.to_server.clone() })
    }
}

impl Drop for PipeEnd {
    fn drop(&mut self) {
        let mut done: Vec<Completion> = Vec::new();
        match &self.channel {
            // A client closed: the server reads what's left, then ERROR_BROKEN_PIPE.
            Some(mine) => {
                done.extend(mine.to_server.close_writer(ERROR_BROKEN_PIPE));
                mine.to_client.close_reader();
                let mut s = lock(&self.instance.state);
                if s.channel.as_ref().is_some_and(|now| Arc::ptr_eq(now, mine)) {
                    s.phase = Phase::Broken;
                }
            }
            // The server closed its instance: its client reads ERROR_BROKEN_PIPE, and the instance is gone.
            None => {
                let mut s = lock(&self.instance.state);
                if let Some(c) = s.channel.take() {
                    done.extend(c.to_client.close_writer(ERROR_BROKEN_PIPE));
                    c.to_server.close_reader();
                }
                if let Some(op) = s.connect.take() {
                    done.push(Completion { op, data: Vec::new(), error: ERROR_OPERATION_ABORTED });
                }
                s.phase = Phase::Disconnected;
                drop(s);
                let me = Arc::as_ptr(&self.instance);
                lock(&self.instance.server.instances).retain(|w| w.strong_count() > 0 && !std::ptr::eq(w.as_ptr(), me));
            }
        }
        // There's no CPU here: each is told now, and written by the next call into atlas.
        if let Some(k) = self.k.upgrade() {
            for c in done {
                k.complete_later(c);
            }
        }
    }
}

/// A pipe's name as atlas keys it: lower case, without `\\.\pipe\`. None for a name that isn't a pipe's.
pub fn key(name: &str) -> Option<String> {
    let lower = name.replace('/', "\\").to_ascii_lowercase();
    let rest = lower.strip_prefix("\\\\.\\pipe\\").or_else(|| lower.strip_prefix("\\\\?\\pipe\\"))?;
    (!rest.is_empty()).then(|| rest.to_string())
}
