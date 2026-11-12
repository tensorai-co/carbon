//! futex2: `futex_wait`, `futex_wake` and `futex_requeue` with sizes and masks of their own, and `futex_waitv`,
//! waiting on up to 128 futexes at once (as Wine and Proton do, for Windows' `WaitForMultipleObjects`). Only 32-bit
//! futexes, as on Linux; a wait's mask is accepted but every wake wakes, as `FUTEX_WAKE_BITSET` does here.

use std::time::{Duration, Instant};

use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Changed, Woke};

use super::Task;
use crate::abi::*;

const FUTEX2_SIZE_U32: u64 = 2;
const FUTEX2_SIZE_MASK: u64 = 3;
const FUTEX2_PRIVATE: u64 = FUTEX_PRIVATE_FLAG;
const FUTEX_WAITV_MAX: u64 = 128;

/// A futex2 flags word: a 32-bit futex, private or not. Whether it's private.
fn futex2_private(flags: u64) -> Result<bool, Errno> {
    if flags & !(FUTEX2_SIZE_MASK | FUTEX2_PRIVATE) != 0 || flags & FUTEX2_SIZE_MASK != FUTEX2_SIZE_U32 {
        return Err(EINVAL);
    }
    Ok(flags & FUTEX2_PRIVATE != 0)
}

/// `struct futex_waitv`: the value expected, where, and its flags.
struct Waitv {
    val: u32,
    addr: u64,
    private: bool,
}

impl Task {
    /// An absolute timeout on `clock` (real time or monotonic), as an instant; none for no timeout.
    fn futex2_deadline(&mut self, cx: &mut Cx<'_>, at: u64, clock: u64) -> Result<Option<Instant>, Errno> {
        if at == 0 {
            return Ok(None);
        }
        let now = match clock {
            CLOCK_REALTIME => self.k.host.wall(),
            1 => self.k.host.monotonic(),
            _ => return Err(EINVAL),
        };
        let (sec, nsec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?;
        if (sec as i64) < 0 || nsec >= 1_000_000_000 {
            return Err(EINVAL);
        }
        let at = sec.saturating_mul(1_000_000_000).saturating_add(nsec);
        Ok(Some(Instant::now() + Duration::from_nanos(at.saturating_sub(now))))
    }

    /// The futexes at `at`, `count` of them, checked: each 32 bits, aligned, with what's expected fitting.
    fn read_waitv(&mut self, cx: &mut Cx<'_>, at: u64, count: u64) -> Result<Vec<Waitv>, Errno> {
        let words = self.mem(cx, false, |t| (0..count * 3).map(|i| user::read_u64(&t.space(), at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
        words
            .chunks(3)
            .map(|w| {
                let (val, addr, flags) = (w[0], w[1], w[2]);
                // The flags word is 32 bits, and the 32 after it reserved.
                if flags >> 32 != 0 || !addr.is_multiple_of(4) || val > u64::from(u32::MAX) {
                    return Err(EINVAL);
                }
                Ok(Waitv { val: val as u32, addr, private: futex2_private(flags)? })
            })
            .collect()
    }

    pub(crate) fn futex_wake(&mut self, addr: u64, mask: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        let private = futex2_private(flags)?;
        if mask as u32 == 0 || !addr.is_multiple_of(4) {
            return Err(EINVAL);
        }
        let key = self.futex_key(addr, private);
        Ok(self.k.futexes.wake(key, count.min(i32::MAX as u64) as usize) as u64)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn futex_wait(&mut self, cx: &mut Cx<'_>, addr: u64, val: u64, mask: u64, flags: u64, timeout_at: u64, clock: u64) -> Result<u64, Errno> {
        let private = futex2_private(flags)?;
        if mask as u32 == 0 || !addr.is_multiple_of(4) || val > u64::from(u32::MAX) {
            return Err(EINVAL);
        }
        let deadline = self.futex2_deadline(cx, timeout_at, clock)?;
        // Brought in first, so the check under the futex table's lock can read it.
        self.mem(cx, false, |t| user::read_u32(&t.space(), addr))?;
        let key = self.futex_key(addr, private);
        let waiter = self.waiter.clone();
        let still = || cx.turn(|t| user::read_u32(&t.space(), addr)) == Ok(val as u32);
        match self.k.futexes.wait(key, &waiter, still, deadline) {
            Err(Changed) => Err(EAGAIN),
            Ok(Woke::Woken) => Ok(0),
            Ok(Woke::Interrupted) => Err(EINTR),
            Ok(Woke::TimedOut) => Err(ETIMEDOUT),
        }
    }

    /// `futex_requeue`: if the first futex holds what's expected, wakes `wake` of its waiters and moves `requeue` more
    /// to the second.
    pub(crate) fn futex_requeue(&mut self, cx: &mut Cx<'_>, at: u64, flags: u64, wake: u64, requeue: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let futexes = self.read_waitv(cx, at, 2)?;
        let now = self.mem(cx, false, |t| user::read_u32(&t.space(), futexes[0].addr))?;
        if now != futexes[0].val {
            return Err(EAGAIN);
        }
        let (from, to) = (self.futex_key(futexes[0].addr, futexes[0].private), self.futex_key(futexes[1].addr, futexes[1].private));
        let woken = self.k.futexes.wake(from, wake.min(i32::MAX as u64) as usize);
        let moved = self.k.futexes.requeue(from, to, requeue.min(i32::MAX as u64) as usize);
        Ok((woken + moved) as u64)
    }

    /// `futex_waitv`: waits until one of the futexes is woken, giving its index; EAGAIN if one already doesn't hold
    /// what's expected.
    pub(crate) fn futex_waitv(&mut self, cx: &mut Cx<'_>, at: u64, count: u64, flags: u64, timeout_at: u64, clock: u64) -> Result<u64, Errno> {
        if flags != 0 || count == 0 || count > FUTEX_WAITV_MAX {
            return Err(EINVAL);
        }
        let futexes = self.read_waitv(cx, at, count)?;
        let deadline = self.futex2_deadline(cx, timeout_at, clock)?;
        for f in &futexes {
            self.mem(cx, false, |t| user::read_u32(&t.space(), f.addr))?;
        }
        let keys: Vec<u64> = futexes.iter().map(|f| self.futex_key(f.addr, f.private)).collect();
        let waiter = self.waiter.clone();
        loop {
            let still = || cx.turn(|t| futexes.iter().all(|f| user::read_u32(&t.space(), f.addr) == Ok(f.val)));
            match self.k.futexes.wait_any(&keys, &waiter, still, deadline) {
                Err(Changed) => return Err(EAGAIN),
                Ok((Woke::Woken, Some(index))) => return Ok(index as u64),
                // Woken by something else: look again.
                Ok((Woke::Woken, None)) => continue,
                Ok((Woke::Interrupted, _)) => return Err(EINTR),
                Ok((Woke::TimedOut, _)) => return Err(ETIMEDOUT),
            }
        }
    }
}
