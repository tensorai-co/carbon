//! Anonymous pipes (`CreatePipe`): a buffer with a read end and a write end, each an open file that handles share, in
//! one process or several. A read waits for data; once every handle to the write end is closed (in every process
//! that had one), a read of an empty pipe is ERROR_BROKEN_PIPE. A write waits for room; once every handle to the read
//! end is closed, it's ERROR_NO_DATA.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use kernels_core::lock;
use kernels_core::wait::{Waiter, Woke};

use crate::os::Reply;
use crate::process::Task;

pub const ERROR_BROKEN_PIPE: u32 = 109;
pub const ERROR_NO_DATA: u32 = 232;

/// How much a pipe holds when `CreatePipe` asks for the default.
const DEFAULT_SIZE: usize = 4096;

pub struct Pipe {
    state: Mutex<State>,
    size: usize,
}

struct State {
    data: VecDeque<u8>,
    reader: bool,
    writer: bool,
    waiters: Vec<Arc<Waiter>>,
}

impl State {
    fn wake(&mut self) {
        for w in self.waiters.drain(..) {
            w.wake();
        }
    }
}

/// One end of a pipe. Every handle to it shares it, so it's dropped when the last of them closes: the other end
/// sees that end close then.
pub struct End {
    pub pipe: Arc<Pipe>,
    pub write: bool,
}

impl Drop for End {
    fn drop(&mut self) {
        let mut s = lock(&self.pipe.state);
        if self.write {
            s.writer = false;
        } else {
            s.reader = false;
        }
        s.wake();
    }
}

/// A new pipe's read end and write end. `size` 0 is the default.
pub fn pair(size: usize) -> (End, End) {
    let size = if size == 0 { DEFAULT_SIZE } else { size };
    let pipe = Arc::new(Pipe { state: Mutex::new(State { data: VecDeque::new(), reader: true, writer: true, waiters: Vec::new() }), size });
    (End { pipe: pipe.clone(), write: false }, End { pipe, write: true })
}

/// Why a pipe call didn't do what was asked: an error to give the program, or the thread is to end.
pub enum Failed {
    Error(u32),
    Ended(Reply),
}

impl Pipe {
    /// What `PeekNamedPipe` sees: up to `len` bytes without taking them, and how many there are in all.
    pub fn peek(&self, len: usize) -> Result<(Vec<u8>, usize), u32> {
        let s = lock(&self.state);
        if s.data.is_empty() && !s.writer {
            return Err(ERROR_BROKEN_PIPE);
        }
        Ok((s.data.iter().take(len).copied().collect(), s.data.len()))
    }
}

impl Task {
    /// Waits on a pipe until `ready` (run with its state locked) has an answer. False from `ready` means not yet.
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

    /// `ReadFile` on a pipe's read end: what's there (up to `len`), waiting for something if it's empty.
    pub fn read_pipe(&mut self, end: &End, len: usize) -> Result<Vec<u8>, Failed> {
        if len == 0 {
            return Ok(Vec::new());
        }
        self.on_pipe(&end.pipe, |s| {
            if !s.data.is_empty() {
                let n = len.min(s.data.len());
                let data = s.data.drain(..n).collect();
                // A writer waiting for room has some.
                s.wake();
                return Some(Ok(data));
            }
            (!s.writer).then_some(Err(ERROR_BROKEN_PIPE))
        })
    }

    /// `WriteFile` on a pipe's write end: all of it, waiting for room as the reader takes what's there.
    pub fn write_pipe(&mut self, end: &End, data: &[u8]) -> Result<usize, Failed> {
        let size = end.pipe.size;
        let mut written = 0;
        while written < data.len() || data.is_empty() {
            let n = self.on_pipe(&end.pipe, |s| {
                if !s.reader {
                    return Some(Err(ERROR_NO_DATA));
                }
                let room = size.saturating_sub(s.data.len());
                if room == 0 && !data.is_empty() {
                    return None;
                }
                let n = room.min(data.len() - written);
                s.data.extend(&data[written..written + n]);
                s.wake();
                Some(Ok(n))
            })?;
            written += n;
            if data.is_empty() {
                break;
            }
        }
        Ok(written)
    }
}
