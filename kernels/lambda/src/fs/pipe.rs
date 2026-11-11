//! Pipes: a buffer between two ends, where a reader waits for data and a writer waits for room. Its watchers are told
//! whenever either changes, for `poll` and `epoll`.

use std::collections::VecDeque;
use std::sync::Arc;

use kernels_core::lock;
use kernels_core::wait::{Waiter, Watchers, Woke};

use crate::abi::*;

/// How much a pipe holds before a writer waits, as on Linux.
const CAPACITY: usize = 64 << 10;

pub(crate) struct Pipe {
    state: std::sync::Mutex<State>,
    pub watchers: Watchers,
}

struct State {
    data: VecDeque<u8>,
    readers: usize,
    writers: usize,
    /// Threads waiting for data or room, to wake when either changes.
    waiting: Vec<Arc<Waiter>>,
}

impl State {
    fn wake_all(&mut self, watchers: &Watchers) {
        for w in self.waiting.drain(..) {
            w.wake();
        }
        watchers.notify();
    }
}

impl Pipe {
    pub fn new() -> Arc<Pipe> {
        Arc::new(Pipe { state: std::sync::Mutex::new(State { data: VecDeque::new(), readers: 1, writers: 1, waiting: Vec::new() }), watchers: Watchers::default() })
    }

    /// Whether a read wouldn't wait (there's data, or no writer is left), and whether no writer is left.
    pub fn readable(&self) -> (bool, bool) {
        let state = lock(&self.state);
        (!state.data.is_empty() || state.writers == 0, state.writers == 0)
    }

    /// How much there is to read now (`FIONREAD`).
    pub fn available(&self) -> usize {
        lock(&self.state).data.len()
    }

    /// Whether a write wouldn't wait (there's room), and whether every reader has gone.
    pub fn writable(&self) -> (bool, bool) {
        let state = lock(&self.state);
        (state.data.len() < CAPACITY, state.readers == 0)
    }

    /// Up to `len` bytes, waiting for some unless `nonblock`. Nothing means every writer has gone.
    pub fn read(&self, me: &Arc<Waiter>, len: usize, nonblock: bool) -> Result<Vec<u8>, Errno> {
        loop {
            {
                let mut state = lock(&self.state);
                if !state.data.is_empty() {
                    let n = len.min(state.data.len());
                    let out: Vec<u8> = state.data.drain(..n).collect();
                    state.wake_all(&self.watchers);
                    return Ok(out);
                }
                if state.writers == 0 || len == 0 {
                    return Ok(Vec::new());
                }
                if nonblock {
                    return Err(EAGAIN);
                }
                me.reset();
                state.waiting.push(me.clone());
            }
            if me.wait(None) == Woke::Interrupted {
                return Err(EINTR);
            }
        }
    }

    /// Writes all of `data`, waiting for room as it goes, unless `nonblock`.
    pub fn write(&self, me: &Arc<Waiter>, data: &[u8], nonblock: bool) -> Result<usize, Errno> {
        let mut written = 0;
        loop {
            {
                let mut state = lock(&self.state);
                if state.readers == 0 {
                    return if written > 0 { Ok(written) } else { Err(EPIPE) };
                }
                let room = CAPACITY.saturating_sub(state.data.len());
                let n = room.min(data.len() - written);
                state.data.extend(&data[written..written + n]);
                written += n;
                if n > 0 {
                    state.wake_all(&self.watchers);
                }
                if written == data.len() {
                    return Ok(written);
                }
                if nonblock {
                    return if written > 0 { Ok(written) } else { Err(EAGAIN) };
                }
                me.reset();
                state.waiting.push(me.clone());
            }
            if me.wait(None) == Woke::Interrupted {
                return if written > 0 { Ok(written) } else { Err(EINTR) };
            }
        }
    }

    /// One end has closed.
    pub fn close(&self, write_end: bool) {
        let mut state = lock(&self.state);
        if write_end {
            state.writers -= 1;
        } else {
            state.readers -= 1;
        }
        state.wake_all(&self.watchers);
    }
}
