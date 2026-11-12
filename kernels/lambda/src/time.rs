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

    /// A clock's reading, or None for a clock Linux doesn't have: the CPU-time clocks (2, 3 and negative ones) read
    /// the threads' times, TAI is the wall clock 37 seconds ahead, and the rest are the wall or monotonic clock.
    fn clock_reading(&self, clock: u64) -> Option<u64> {
        const TAI_OFFSET: u64 = 37_000_000_000;
        match clock as i32 {
            2 | 3 | ..0 => self.cpu_clock(clock),
            11 => Some(self.k.host.wall() + TAI_OFFSET),
            0 | 1 | 4..=9 => Some(self.clock_now(clock)),
            _ => None,
        }
    }

    pub(crate) fn clock_gettime(&mut self, cx: &mut Cx<'_>, clock: u64, at: u64) -> Result<u64, Errno> {
        let now = self.clock_reading(clock).ok_or(EINVAL)?;
        self.write_timespec(cx, at, now)?;
        Ok(0)
    }

    pub(crate) fn clock_getres(&mut self, cx: &mut Cx<'_>, clock: u64, at: u64) -> Result<u64, Errno> {
        self.clock_reading(clock).ok_or(EINVAL)?;
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
    /// `clock_settime`, and `settimeofday` with a time: a machine's real time is its host's, so it can't be set, as in
    /// a container without `CAP_SYS_TIME`. A clock that can never be set is EINVAL.
    pub(crate) fn clock_settime(&mut self, cx: &mut Cx<'_>, clock: u64, at: u64) -> Result<u64, Errno> {
        let (sec, nsec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?;
        if clock != CLOCK_REALTIME {
            return Err(EINVAL);
        }
        if (sec as i64) < 0 || nsec >= 1_000_000_000 {
            return Err(EINVAL);
        }
        Err(EPERM)
    }

    /// `settimeofday`: setting neither is nothing; setting the time is `clock_settime`'s EPERM, and so is the time
    /// zone, which needs the same privilege.
    pub(crate) fn settimeofday(&mut self, cx: &mut Cx<'_>, tv_at: u64, tz_at: u64) -> Result<u64, Errno> {
        if tv_at != 0 {
            let usec = self.mem(cx, false, |t| user::read_u64(&t.space(), tv_at + 8))?;
            if usec >= 1_000_000 {
                return Err(EINVAL);
            }
        }
        if tv_at == 0 && tz_at == 0 { Ok(0) } else { Err(EPERM) }
    }

    /// `adjtimex` and `clock_adjtime`: what the kernel's clock discipline says, read only. The host keeps its own
    /// clock in step, so it's synchronized, with a millisecond of error at most; adjusting it is EPERM.
    pub(crate) fn adjtimex(&mut self, cx: &mut Cx<'_>, clock: u64, at: u64) -> Result<u64, Errno> {
        const ADJ_OFFSET_SS_READ: u32 = 0xa001;
        const TIME_OK: u64 = 0;
        if clock != CLOCK_REALTIME {
            return Err(EINVAL);
        }
        let modes = self.mem(cx, false, |t| user::read_u32(&t.space(), at))?;
        if modes != 0 && modes != ADJ_OFFSET_SS_READ {
            return Err(EPERM);
        }
        let now = self.k.host.wall();
        // struct timex: modes, offset, freq, maxerror, esterror, status, constant, precision, tolerance, time, tick,
        // the PPS counters, then tai.
        let mut b = vec![0u8; 208];
        let mut put = |at: usize, v: i64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
        put(24, 1000); // maxerror, in microseconds
        put(48, 2); // constant
        put(56, 1); // precision
        put(64, 500 << 16); // tolerance: 500 ppm
        put(72, (now / 1_000_000_000) as i64);
        put(80, (now % 1_000_000_000 / 1000) as i64);
        put(88, 10_000); // tick: USER_HZ's, in microseconds
        b[160..164].copy_from_slice(&37i32.to_le_bytes()); // tai
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(TIME_OK)
    }

    fn write_timespec(&mut self, cx: &mut Cx<'_>, at: u64, nanos: u64) -> Result<(), Errno> {
        self.mem(cx, true, |t| {
            user::write_u64(&mut t.space(), at, nanos / 1_000_000_000)?;
            user::write_u64(&mut t.space(), at + 8, nanos % 1_000_000_000)
        })
    }
}
