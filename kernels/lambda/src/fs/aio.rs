//! Linux AIO, as libaio and databases use it: `io_setup`, `io_submit`, `io_getevents`, `io_pgetevents`, `io_cancel`
//! and `io_destroy`.
//!
//! A request is done as it's submitted, as Linux does reads and writes of a file not opened `O_DIRECT` (and lambda's
//! disk is in memory): `io_submit` does it and queues its completion, and `io_getevents` takes completions, waiting
//! for them if it's to. A context is a process's, as Linux's are its memory's: not inherited, gone at `execve`. Its ID
//! is the address of memory mapped for it, as Linux's is of its completion ring. libaio reads that ring's header for a
//! fast path; it's left zeroed here, without the ring's magic, so libaio always asks the kernel.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};

use super::Node;
use crate::abi::*;
use crate::process::Task;

const IOCB_CMD_PREAD: u16 = 0;
const IOCB_CMD_PWRITE: u16 = 1;
const IOCB_CMD_FSYNC: u16 = 2;
const IOCB_CMD_FDSYNC: u16 = 3;
const IOCB_CMD_PREADV: u16 = 7;
const IOCB_CMD_PWRITEV: u16 = 8;
/// Tell an eventfd (`aio_resfd`) when it's done.
const IOCB_FLAG_RESFD: u32 = 1;
const IOCB_FLAG_IOPRIO: u32 = 2;
/// `RWF_HIPRI`, `RWF_DSYNC`, `RWF_SYNC`, `RWF_NOWAIT`, `RWF_APPEND`: nothing here waits, or needs them.
const RWF_KNOWN: u32 = 0x1f;
/// Linux's `aio-max-nr`, and the size of `struct io_event`.
const MOST_EVENTS: u64 = 65536;
const EVENT: u64 = 32;

/// A process's AIO context.
pub(crate) struct AioCtx {
    /// How many completions it holds at most: `io_setup`'s count.
    most: usize,
    /// The memory mapped for it, as `io_destroy` unmaps it.
    ring: (u64, u64),
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// `struct io_event`s not yet taken: the request's `aio_data`, its iocb's address, and its result.
    done: VecDeque<[u64; 4]>,
    waiting: Vec<Arc<Waiter>>,
    destroyed: bool,
}

impl AioCtx {
    fn wake_all(state: &mut State) {
        for waiter in state.waiting.drain(..) {
            waiter.wake();
        }
    }
}

/// One `struct iocb`.
struct Iocb {
    data: u64,
    rw_flags: u32,
    opcode: u16,
    fd: i32,
    buf: u64,
    nbytes: u64,
    offset: u64,
    reserved: u64,
    flags: u32,
    resfd: i32,
}

impl Iocb {
    fn decode(b: &[u8]) -> Iocb {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or_default());
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
        Iocb {
            data: u64_at(0),
            rw_flags: u32_at(12),
            opcode: u16::from_le_bytes([b[16], b[17]]),
            fd: u32_at(20) as i32,
            buf: u64_at(24),
            nbytes: u64_at(32),
            offset: u64_at(40),
            reserved: u64_at(48),
            flags: u32_at(56),
            resfd: u32_at(60) as i32,
        }
    }
}

impl Task {
    fn aio_ctx(&self, id: u64) -> Result<Arc<AioCtx>, Errno> {
        lock(&self.process.state).aio.get(&id).cloned().ok_or(EINVAL)
    }

    pub(crate) fn io_setup(&mut self, cx: &mut Cx<'_>, count: u64, id_at: u64) -> Result<u64, Errno> {
        let count = count as u32 as u64;
        if count == 0 {
            return Err(EINVAL);
        }
        if count > MOST_EVENTS {
            return Err(EAGAIN);
        }
        if self.mem(cx, false, |t| user::read_u64(&t.space(), id_at))? != 0 {
            return Err(EINVAL);
        }
        let len = (EVENT + EVENT * count).next_multiple_of(4096);
        let ring = self.mmap(cx, 0, len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0)?;
        if let Err(e) = self.mem(cx, true, |t| user::write_u64(&mut t.space(), id_at, ring)) {
            let _ = self.munmap(cx, ring, len);
            return Err(e);
        }
        let ctx = Arc::new(AioCtx { most: count as usize, ring: (ring, len), state: Mutex::default() });
        lock(&self.process.state).aio.insert(ring, ctx);
        Ok(0)
    }

    pub(crate) fn io_destroy(&mut self, cx: &mut Cx<'_>, id: u64) -> Result<u64, Errno> {
        let ctx = lock(&self.process.state).aio.remove(&id).ok_or(EINVAL)?;
        {
            let mut state = lock(&ctx.state);
            state.destroyed = true;
            AioCtx::wake_all(&mut state);
        }
        let _ = self.munmap(cx, ctx.ring.0, ctx.ring.1);
        Ok(0)
    }

    /// Does one request: its result (a count, or a negative errno), or why it can't be submitted.
    fn aio_do(&mut self, cx: &mut Cx<'_>, cb: &Iocb) -> Result<i64, Errno> {
        if cb.reserved != 0 || cb.flags & !(IOCB_FLAG_RESFD | IOCB_FLAG_IOPRIO) != 0 {
            return Err(EINVAL);
        }
        if cb.rw_flags & !RWF_KNOWN != 0 {
            return Err(EOPNOTSUPP);
        }
        let open = self.file(cb.fd)?;
        let vectored = |t: &mut Task, cx: &mut Cx<'_>, write: bool| {
            if !super::io::seekable(&open) {
                return Err(ESPIPE);
            }
            if (cb.offset as i64) < 0 {
                return Err(EINVAL);
            }
            if write { t.vwrite(cx, cb.fd, cb.buf, cb.nbytes, Some(cb.offset)) } else { t.vread(cx, cb.fd, cb.buf, cb.nbytes, Some(cb.offset)) }
        };
        let result = match cb.opcode {
            IOCB_CMD_PREAD => self.pread(cx, cb.fd, cb.buf, cb.nbytes, cb.offset),
            IOCB_CMD_PWRITE => self.pwrite(cx, cb.fd, cb.buf, cb.nbytes, cb.offset),
            IOCB_CMD_PREADV => vectored(self, cx, false),
            IOCB_CMD_PWRITEV => vectored(self, cx, true),
            IOCB_CMD_FSYNC | IOCB_CMD_FDSYNC => match open.node {
                Node::File(_) | Node::Dir(_) => lock(&self.k.disk).sync().map(|()| 0).map_err(Errno::from),
                _ => Err(EINVAL),
            },
            // `IOCB_CMD_POLL` completes when its file is ready, later: not here yet. `IOCB_CMD_NOOP` isn't Linux's.
            _ => return Err(EINVAL),
        };
        Ok(match result {
            Ok(n) => n as i64,
            Err(e) => -i64::from(e.0),
        })
    }

    pub(crate) fn io_submit(&mut self, cx: &mut Cx<'_>, id: u64, count: u64, list_at: u64) -> Result<u64, Errno> {
        let ctx = self.aio_ctx(id)?;
        if (count as i64) < 0 {
            return Err(EINVAL);
        }
        let count = count.min(ctx.most as u64);
        for i in 0..count {
            let submitted = (|| {
                let at = self.mem(cx, false, |t| user::read_u64(&t.space(), list_at + i * 8))?;
                let cb = Iocb::decode(&self.mem(cx, false, |t| user::read_vec(&t.space(), at, 64))?);
                let resfd = if cb.flags & IOCB_FLAG_RESFD != 0 {
                    match &self.file(cb.resfd).map_err(|_| EBADF)?.node {
                        Node::Eventfd(eventfd) => Some(eventfd.clone()),
                        _ => return Err(EINVAL),
                    }
                } else {
                    None
                };
                if lock(&ctx.state).done.len() >= ctx.most {
                    return Err(EAGAIN);
                }
                let result = self.aio_do(cx, &cb)?;
                {
                    let mut state = lock(&ctx.state);
                    state.done.push_back([cb.data, at, result as u64, 0]);
                    AioCtx::wake_all(&mut state);
                }
                if let Some(eventfd) = resfd {
                    let _ = eventfd.add(1);
                }
                Ok(())
            })();
            if let Err(e) = submitted {
                return if i == 0 { Err(e) } else { Ok(i) };
            }
        }
        Ok(count)
    }

    /// `io_cancel`: every request is done as it's submitted, so there's never one to cancel.
    pub(crate) fn io_cancel(&mut self, id: u64) -> Result<u64, Errno> {
        self.aio_ctx(id)?;
        Err(EINVAL)
    }

    /// `io_getevents` and `io_pgetevents`: at least `least` completions (fewer if the timeout comes first), at most
    /// `most`. `sig_at` is `io_pgetevents`' signal mask and its size, or 0.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn io_getevents(&mut self, cx: &mut Cx<'_>, id: u64, least: u64, most: u64, events_at: u64, timeout_at: u64, sig_at: u64) -> Result<u64, Errno> {
        let ctx = self.aio_ctx(id)?;
        let (least, most) = (least as i64, most as i64);
        if least < 0 || most < 0 || least > most {
            return Err(EINVAL);
        }
        let deadline = self.timespec_timeout(cx, timeout_at)?.map(|d| Instant::now() + d);
        let mask_at = if sig_at == 0 { 0 } else { self.mem(cx, false, |t| user::read_u64(&t.space(), sig_at))? };
        let taken = self.with_mask(cx, mask_at, |task, _| task.aio_wait(&ctx, least as usize, most as usize, deadline))?;
        let bytes: Vec<u8> = taken.iter().flat_map(|e| e.iter().flat_map(|w| w.to_le_bytes())).collect();
        self.mem(cx, true, |t| user::write(&mut t.space(), events_at, &bytes))?;
        Ok(taken.len() as u64)
    }

    fn aio_wait(&self, ctx: &AioCtx, least: usize, most: usize, deadline: Option<Instant>) -> Result<Vec<[u64; 4]>, Errno> {
        let mut timed_out = false;
        loop {
            {
                let mut state = lock(&ctx.state);
                if state.destroyed {
                    return Err(EINVAL);
                }
                if state.done.len() >= least || timed_out {
                    let n = state.done.len().min(most);
                    return Ok(state.done.drain(..n).collect());
                }
                self.waiter.reset();
                state.waiting.push(self.waiter.clone());
            }
            match self.waiter.wait(deadline) {
                Woke::Woken => {}
                woke => {
                    let mut state = lock(&ctx.state);
                    state.waiting.retain(|w| !Arc::ptr_eq(w, &self.waiter));
                    // A signal: what's there already, if anything, else EINTR.
                    if woke == Woke::Interrupted {
                        let n = state.done.len().min(most);
                        return if n == 0 { Err(EINTR) } else { Ok(state.done.drain(..n).collect()) };
                    }
                    timed_out = true;
                }
            }
        }
    }
}
