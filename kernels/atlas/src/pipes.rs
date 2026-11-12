//! Pipes: one direction's buffer, which anonymous pipes (`CreatePipe`) are one of and named pipes two of. A read takes
//! what's there, waits for data, or (overlapped) leaves a pending read the writer completes. Once every handle to the
//! write end is closed (in every process that had one), a read of an empty pipe is ERROR_BROKEN_PIPE (or, after a
//! named pipe's server disconnects, ERROR_PIPE_NOT_CONNECTED). A write waits for room, unless it's overlapped, which
//! the pipe always takes at once; once every handle to the read end is closed, it's ERROR_NO_DATA.
//!
//! A message pipe keeps each write's boundary: a read in message mode takes one message, or as much of it as fits
//! with ERROR_MORE_DATA, the rest left for the next read.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use kernels_core::lock;
use kernels_core::wait::{Waiter, Woke};

use crate::os::Reply;
use crate::ports::{Completion, ERROR_MORE_DATA, ERROR_OPERATION_ABORTED, Op};
use crate::process::{Process, Task};

pub const ERROR_BROKEN_PIPE: u32 = 109;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_PIPE_NOT_CONNECTED: u32 = 233;

/// How much a pipe holds when its maker asks for the default.
const DEFAULT_SIZE: usize = 4096;

pub struct Pipe {
    state: Mutex<State>,
    size: usize,
}

struct State {
    data: VecDeque<u8>,
    /// Each message's length, for a message pipe.
    messages: Option<VecDeque<usize>>,
    reader: bool,
    writer: bool,
    /// What a read of the empty pipe gets once the writer's gone.
    broken: u32,
    waiters: Vec<Arc<Waiter>>,
    /// Overlapped reads waiting for data: each one's operation, how much it takes, and whether in message mode.
    pending: VecDeque<(Arc<Op>, usize, bool)>,
}

impl State {
    fn wake(&mut self) {
        for w in self.waiters.drain(..) {
            w.wake();
        }
    }

    /// Takes up to `len`: one message (or what fits of it, and true for more), or bytes across messages.
    fn take(&mut self, len: usize, message_mode: bool) -> (Vec<u8>, bool) {
        let Some(messages) = &mut self.messages else {
            let n = len.min(self.data.len());
            return (self.data.drain(..n).collect(), false);
        };
        if message_mode {
            let Some(first) = messages.front_mut() else { return (Vec::new(), false) };
            let n = len.min(*first);
            let more = n < *first;
            if more {
                *first -= n;
            } else {
                messages.pop_front();
            }
            return (self.data.drain(..n).collect(), more);
        }
        let n = len.min(self.data.len());
        let mut left = n;
        while left > 0 {
            let Some(first) = messages.front_mut() else { break };
            if *first > left {
                *first -= left;
                left = 0;
            } else {
                left -= *first;
                messages.pop_front();
            }
        }
        (self.data.drain(..n).collect(), false)
    }

    fn has_data(&self) -> bool {
        !self.data.is_empty() || self.messages.as_ref().is_some_and(|m| !m.is_empty())
    }

    /// Pending reads that what's there now completes.
    fn serve(&mut self) -> Vec<Completion> {
        let mut done = Vec::new();
        while self.has_data() {
            let Some((op, len, message_mode)) = self.pending.pop_front() else { break };
            let (data, more) = self.take(len, message_mode);
            done.push(Completion { op, data, error: if more { ERROR_MORE_DATA } else { 0 } });
        }
        if !self.writer && !self.has_data() {
            let broken = self.broken;
            done.extend(self.pending.drain(..).map(|(op, _, _)| Completion { op, data: Vec::new(), error: broken }));
        }
        done
    }
}

/// What a read found.
pub enum Read {
    /// Data, and whether a message has more.
    Data(Vec<u8>, bool),
    /// Nothing there, and nothing will be: the error.
    Ended(u32),
    /// Nothing yet: waiting (or queued, for an overlapped read).
    Empty,
}

impl Pipe {
    pub fn new(size: usize, message: bool) -> Arc<Pipe> {
        let size = if size == 0 { DEFAULT_SIZE } else { size };
        let state = State { data: VecDeque::new(), messages: message.then(VecDeque::new), reader: true, writer: true, broken: ERROR_BROKEN_PIPE, waiters: Vec::new(), pending: VecDeque::new() };
        Arc::new(Pipe { state: Mutex::new(state), size })
    }

    pub fn is_message(&self) -> bool {
        lock(&self.state).messages.is_some()
    }

    /// Reads what's there, or with `op`, queues it to be completed when there's something.
    pub fn read(&self, len: usize, message_mode: bool, op: Option<Arc<Op>>) -> Read {
        let mut s = lock(&self.state);
        if s.has_data() {
            let (data, more) = s.take(len, message_mode);
            // A writer waiting for room has some.
            s.wake();
            return Read::Data(data, more);
        }
        if !s.writer {
            return Read::Ended(s.broken);
        }
        if let Some(op) = op {
            s.pending.push_back((op, len, message_mode));
        }
        Read::Empty
    }

    /// Takes all of `data` at once, as an overlapped write: the pending reads it completes.
    pub fn write_all(&self, data: &[u8]) -> Result<Vec<Completion>, u32> {
        let mut s = lock(&self.state);
        if !s.reader {
            return Err(ERROR_NO_DATA);
        }
        s.data.extend(data);
        if let Some(m) = &mut s.messages {
            m.push_back(data.len());
        }
        s.wake();
        Ok(s.serve())
    }

    /// What `PeekNamedPipe` sees: up to `len` bytes without taking them, how many there are in all, and how many are
    /// left of the first message after those.
    pub fn peek(&self, len: usize) -> Result<(Vec<u8>, usize, usize), u32> {
        let s = lock(&self.state);
        if !s.has_data() && !s.writer {
            return Err(s.broken);
        }
        let first = s.messages.as_ref().and_then(|m| m.front().copied());
        let data: Vec<u8> = s.data.iter().take(first.unwrap_or(usize::MAX).min(len)).copied().collect();
        let left = first.map_or(0, |f| f - data.len());
        Ok((data, s.data.len(), left))
    }

    /// The write end is closed (with `broken`, the error reads get then): the pending reads it ends.
    pub fn close_writer(&self, broken: u32) -> Vec<Completion> {
        let mut s = lock(&self.state);
        s.writer = false;
        s.broken = broken;
        s.wake();
        s.serve()
    }

    /// The read end is closed: writers can't write any more, and what was there goes.
    pub fn close_reader(&self) {
        let mut s = lock(&self.state);
        s.reader = false;
        s.data.clear();
        if let Some(m) = &mut s.messages {
            m.clear();
        }
        s.wake();
    }

    /// Cancels the pending reads `process` asked for (just the one at `overlapped`, if that's given).
    pub fn cancel(&self, process: &Arc<Process>, overlapped: Option<u64>) -> Vec<Completion> {
        let mut s = lock(&self.state);
        let mut kept = VecDeque::new();
        let mut done = Vec::new();
        for (op, len, mm) in s.pending.drain(..) {
            let mine = op.process.upgrade().is_some_and(|p| Arc::ptr_eq(&p, process)) && overlapped.is_none_or(|o| o == op.overlapped);
            if mine {
                done.push(Completion { op, data: Vec::new(), error: ERROR_OPERATION_ABORTED });
            } else {
                kept.push_back((op, len, mm));
            }
        }
        s.pending = kept;
        done
    }
}

/// One end of an anonymous pipe. Every handle to it shares it, so it's dropped when the last of them closes, and the
/// other end sees that end close then.
pub struct End {
    pub pipe: Arc<Pipe>,
    pub write: bool,
}

impl Drop for End {
    fn drop(&mut self) {
        // An anonymous pipe's reads never wait as pending I/O, so closing its writer leaves nothing to complete.
        if self.write {
            let _ = self.pipe.close_writer(ERROR_BROKEN_PIPE);
        } else {
            self.pipe.close_reader();
        }
    }
}

/// A new anonymous pipe's read end and write end. `size` 0 is the default.
pub fn pair(size: usize) -> (End, End) {
    let pipe = Pipe::new(size, false);
    (End { pipe: pipe.clone(), write: false }, End { pipe, write: true })
}

/// Why a pipe call didn't do what was asked: an error to give the program, or the thread is to end.
pub enum Failed {
    Error(u32),
    Ended(Reply),
}

impl Task {
    /// Waits on a pipe until `ready` (run with its state locked) has an answer.
    fn on_pipe<R>(&mut self, pipe: &Pipe, mut ready: impl FnMut(&mut State) -> Option<Result<R, u32>>) -> Result<R, Failed> {
        loop {
            self.waiter.reset();
            {
                let mut s = lock(&pipe.state);
                if let Some(answer) = ready(&mut s) {
                    return answer.map_err(Failed::Error);
                }
                s.waiters.push(self.waiter.clone());
            }
            if self.waiter.wait(None) == Woke::Interrupted {
                if self.ending() {
                    return Err(Failed::Ended(Reply::Ended));
                }
                self.waiter.clear_interrupt();
            }
        }
    }

    /// A read that waits for something: what's there (up to `len`, one message in message mode, with true if it has
    /// more).
    pub fn read_pipe(&mut self, pipe: &Pipe, len: usize, message_mode: bool) -> Result<(Vec<u8>, bool), Failed> {
        if len == 0 && !pipe.is_message() {
            return Ok((Vec::new(), false));
        }
        self.on_pipe(pipe, |s| {
            if s.has_data() {
                let taken = s.take(len, message_mode);
                s.wake();
                return Some(Ok(taken));
            }
            (!s.writer).then_some(Err(s.broken))
        })
    }

    /// A write that waits for room: all of it, as the reader takes what's there. The pending reads it completes are
    /// completed here.
    pub fn write_pipe(&mut self, cx: &mut kernels_core::thread::Cx<'_>, pipe: &Pipe, data: &[u8]) -> Result<usize, Failed> {
        let size = pipe.size;
        let message = pipe.is_message();
        let mut written = 0;
        loop {
            let (n, done) = self.on_pipe(pipe, |s| {
                if !s.reader {
                    return Some(Err(ERROR_NO_DATA));
                }
                // A message goes in whole once there's room for any of it, so its boundary stays; a read that's
                // waiting takes what comes even when the buffer's full.
                let remaining = data.len() - written;
                let room = size.saturating_sub(s.data.len());
                let open = room > 0 || !s.pending.is_empty();
                if !open && remaining > 0 {
                    return None;
                }
                let n = if message { remaining } else { remaining.min(room.max(size * usize::from(!s.pending.is_empty()))) };
                s.data.extend(&data[written..written + n]);
                if let Some(m) = &mut s.messages {
                    m.push_back(n);
                }
                s.wake();
                Some(Ok((n, s.serve())))
            })?;
            for c in done {
                let k = self.k.clone();
                k.complete(cx, c);
            }
            written += n;
            if written >= data.len() {
                return Ok(written);
            }
        }
    }
}
