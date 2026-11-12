//! I/O completion ports: a queue of completion packets (bytes, a key, an OVERLAPPED's address, and a status) that
//! threads take from in `GetQueuedCompletionStatus`. A packet comes from I/O on a file associated with the port, or
//! from `PostQueuedCompletionStatus`. Waiting threads are woken last-in first-out, as Windows does, so the thread
//! that ran most recently gets the next packet.
//!
//! OVERLAPPED's `Internal` holds an NTSTATUS, which `status` and `error` translate to and from the Win32 errors atlas
//! works in.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use kernels_core::lock;
use kernels_core::wait::{Waiter, Woke};

use crate::Kernel;
use crate::objects::{Object, WAIT_IO_COMPLETION};
use crate::process::{Process, Task, write_remote};
use crate::threads::ThreadControl;

pub const STATUS_PENDING: u32 = 0x103;
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_OPERATION_ABORTED: u32 = 995;
pub const ERROR_IO_PENDING: u32 = 997;
pub const ERROR_IO_INCOMPLETE: u32 = 996;
pub const WAIT_TIMEOUT: u32 = 258;

/// One completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub bytes: u32,
    pub key: u64,
    pub overlapped: u64,
    /// An NTSTATUS.
    pub status: u32,
}

#[derive(Default)]
pub struct Port {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    packets: VecDeque<Packet>,
    /// Threads waiting for a packet, the latest last.
    waiters: Vec<Arc<Waiter>>,
}

impl Port {
    pub fn post(&self, packet: Packet) {
        let mut s = lock(&self.state);
        s.packets.push_back(packet);
        if let Some(w) = s.waiters.pop() {
            w.wake();
        }
    }
}

/// How a wait for packets ended without any.
pub enum Empty {
    TimedOut,
    /// An alertable wait ran APCs.
    Alerted,
    /// The thread or its process is ending.
    Ended,
}

impl Task {
    /// Takes up to `most` packets, waiting until `deadline` for the first. With `alertable`, queued APCs run instead.
    pub fn dequeue(&mut self, cx: &mut kernels_core::thread::Cx<'_>, port: &Port, most: usize, deadline: Option<Instant>, alertable: bool) -> Result<Vec<Packet>, Empty> {
        loop {
            if alertable && !lock(&self.control.apcs).is_empty() {
                return match self.run_apcs(cx) {
                    Ok(()) => Err(Empty::Alerted),
                    Err(_) => Err(Empty::Ended),
                };
            }
            self.waiter.reset();
            {
                let mut s = lock(&port.state);
                if !s.packets.is_empty() {
                    let n = most.min(s.packets.len());
                    let taken = s.packets.drain(..n).collect();
                    // Another waiter may take what's left.
                    if !s.packets.is_empty()
                        && let Some(w) = s.waiters.pop()
                    {
                        w.wake();
                    }
                    return Ok(taken);
                }
                s.waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
                s.waiters.push(self.waiter.clone());
            }
            let woke = self.waiter.wait(deadline);
            if woke != Woke::Woken {
                lock(&port.state).waiters.retain(|w| !Arc::ptr_eq(w, &self.waiter));
            }
            match woke {
                Woke::TimedOut => {
                    // A packet that came just as the time ran out is still taken.
                    let mut s = lock(&port.state);
                    if s.packets.is_empty() {
                        return Err(Empty::TimedOut);
                    }
                    let n = most.min(s.packets.len());
                    return Ok(s.packets.drain(..n).collect());
                }
                Woke::Interrupted if self.ending() => return Err(Empty::Ended),
                Woke::Interrupted => self.waiter.clear_interrupt(),
                Woke::Woken => {}
            }
        }
    }
}

/// The NTSTATUS for a Win32 error, as OVERLAPPED's `Internal` and a packet keep it.
pub fn status(error: u32) -> u32 {
    match error {
        0 => 0,
        ERROR_MORE_DATA => 0x8000_0005,
        ERROR_HANDLE_EOF => 0xc000_0011,
        5 => 0xc000_0022,
        6 => 0xc000_0008,
        87 => 0xc000_000d,
        109 => 0xc000_014b,
        232 => 0xc000_00b1,
        233 => 0xc000_00b0,
        995 => 0xc000_0120,
        other => 0xe000_0000 | other,
    }
}

/// The Win32 error for an NTSTATUS.
pub fn error(status: u32) -> u32 {
    match status {
        0 => 0,
        STATUS_PENDING => ERROR_IO_INCOMPLETE,
        0x8000_0005 => ERROR_MORE_DATA,
        0xc000_0011 => ERROR_HANDLE_EOF,
        0xc000_0022 => 5,
        0xc000_0008 => 6,
        0xc000_000d => 87,
        0xc000_014b => 109,
        0xc000_00b1 => 232,
        0xc000_00b0 => 233,
        0xc000_0120 => 995,
        s if s & 0xffff_0000 == 0xe000_0000 => s & 0xffff,
        _ => 31,
    }
}

/// What an alertable wait that ran APCs returns.
pub const IO_COMPLETION: u32 = WAIT_IO_COMPLETION;

/// An I/O that's under way (`ERROR_IO_PENDING`): where its data and completion go once another thread finishes it.
pub struct Op {
    pub process: Weak<Process>,
    /// The thread that asked, whose next alertable wait runs the routine.
    pub thread: Weak<ThreadControl>,
    pub overlapped: u64,
    pub buffer: u64,
    pub event: Option<Arc<Object>>,
    pub port: Option<(Arc<Port>, u64)>,
    /// `ReadFileEx`'s routine.
    pub routine: Option<u64>,
}

/// An I/O finished: what it read (if anything) and how it went.
pub struct Completion {
    pub op: Arc<Op>,
    pub data: Vec<u8>,
    pub error: u32,
}

impl Kernel {
    /// Finishes an I/O from a thread with a CPU: its data into the asking process's buffer and its status into the
    /// OVERLAPPED, then its event, its port's packet and its routine.
    pub(crate) fn complete(&self, cx: &mut kernels_core::thread::Cx<'_>, c: Completion) {
        if let Some(process) = c.op.process.upgrade() {
            let fields = [u64::from(status(c.error)).to_le_bytes(), (c.data.len() as u64).to_le_bytes()].concat();
            let mut writes: Vec<(u64, &[u8])> = Vec::new();
            if !c.data.is_empty() {
                writes.push((c.op.buffer, &c.data));
            }
            writes.push((c.op.overlapped, &fields));
            write_remote(self, cx, &process, &writes);
        }
        self.notify(&c);
    }

    /// Finishes an I/O where there's no CPU to write with (a handle dropped as its process ends): told now, its
    /// OVERLAPPED written by the next thread that calls into atlas. Only for one that read nothing.
    pub(crate) fn complete_later(&self, c: Completion) {
        self.notify(&c);
        lock(&self.deferred).push(c);
    }

    /// Writes what `complete_later` left.
    pub(crate) fn drain(&self, cx: &mut kernels_core::thread::Cx<'_>) {
        let list = std::mem::take(&mut *lock(&self.deferred));
        for c in list {
            if let Some(process) = c.op.process.upgrade() {
                let fields = [u64::from(status(c.error)).to_le_bytes(), 0u64.to_le_bytes()].concat();
                write_remote(self, cx, &process, &[(c.op.overlapped, &fields)]);
            }
        }
    }

    fn notify(&self, c: &Completion) {
        let n = c.data.len() as u64;
        if let Some(event) = &c.op.event {
            self.objects.signal(event, |s| s.set = true);
        }
        if let Some((port, key)) = &c.op.port {
            port.post(Packet { bytes: n as u32, key: *key, overlapped: c.op.overlapped, status: status(c.error) });
        }
        if let (Some(routine), Some(thread)) = (c.op.routine, c.op.thread.upgrade()) {
            lock(&thread.apcs).push_back((routine, vec![u64::from(c.error), n, c.op.overlapped]));
            thread.waiter.wake();
        }
    }
}
