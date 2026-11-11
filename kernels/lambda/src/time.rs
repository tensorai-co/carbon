//! Clocks and sleeping. Every clock is the host's: real time is its wall clock, and the rest are its monotonic one.

use std::time::{Duration, Instant};

use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::Woke;

use crate::abi::*;
use crate::process::Task;

impl Task {
    fn clock_now(&self, clock: u64) -> u64 {
        if clock == CLOCK_REALTIME || clock == 5 || clock == 8 { self.k.host.wall() } else { self.k.host.monotonic() }
    }

    pub(crate) fn clock_gettime(&mut self, cx: &mut Cx<'_>, clock: u64, at: u64) -> Result<u64, Errno> {
        let now = self.clock_now(clock);
        self.write_timespec(cx, at, now)?;
        Ok(0)
    }

    pub(crate) fn clock_getres(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        if at != 0 {
            self.write_timespec(cx, at, 1)?;
        }
        Ok(0)
    }

    pub(crate) fn gettimeofday(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u64, Errno> {
        if at != 0 {
            let now = self.k.host.wall();
            self.mem(cx, true, |t| {
                user::write_u64(&mut t.space(), at, now / 1_000_000_000)?;
                user::write_u64(&mut t.space(), at + 8, now % 1_000_000_000 / 1000)
            })?;
        }
        Ok(0)
    }

    /// `nanosleep` and `clock_nanosleep`: sleeps until woken by a signal, or the time comes.
    pub(crate) fn nanosleep(&mut self, cx: &mut Cx<'_>, clock: u64, flags: u64, request: u64, remain: u64) -> Result<u64, Errno> {
        let (sec, nsec) = self.mem(cx, false, |t| {
            let space = t.space();
            Ok((user::read_u64(&space, request)?, user::read_u64(&space, request + 8)?))
        })?;
        if nsec >= 1_000_000_000 {
            return Err(EINVAL);
        }
        let mut wanted = Duration::new(sec, nsec as u32);
        if flags & TIMER_ABSTIME != 0 {
            wanted = wanted.saturating_sub(Duration::from_nanos(self.clock_now(clock)));
        }
        let deadline = Instant::now() + wanted;
        self.waiter.reset();
        match self.waiter.wait(Some(deadline)) {
            Woke::Interrupted => {
                if remain != 0 && flags & TIMER_ABSTIME == 0 {
                    let left = deadline.saturating_duration_since(Instant::now()).as_nanos() as u64;
                    self.write_timespec(cx, remain, left)?;
                }
                Err(EINTR)
            }
            _ => Ok(0),
        }
    }
}

impl Task {
    fn write_timespec(&mut self, cx: &mut Cx<'_>, at: u64, nanos: u64) -> Result<(), Errno> {
        self.mem(cx, true, |t| {
            user::write_u64(&mut t.space(), at, nanos / 1_000_000_000)?;
            user::write_u64(&mut t.space(), at + 8, nanos % 1_000_000_000)
        })
    }
}
