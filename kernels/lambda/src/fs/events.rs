//! `eventfd` and `timerfd`: files that are only a counter. An eventfd's counter is what's been written and not yet
//! read; a timerfd's is how many times it has expired since it was last read. Each tells its watchers when its counter
//! changes (or its timer is set), and a thread that has to wait for one waits as `poll` does, among its watchers. A
//! timer needs no thread of its own: whoever waits on one waits until its next expiry, at the latest, and whoever
//! first sees that it has expired tells its watchers (so an edge-triggered epoll interest in it fires).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kernels_core::lock;
use kernels_core::wait::Watchers;

use crate::abi::*;

/// The most an eventfd's counter holds: one less than all ones.
const MOST: u64 = u64::MAX - 1;

pub(crate) struct Eventfd {
    count: Mutex<u64>,
    /// `EFD_SEMAPHORE`: a read takes one, not all.
    semaphore: bool,
    pub watchers: Watchers,
}

impl Eventfd {
    pub fn new(initial: u64, semaphore: bool) -> Arc<Eventfd> {
        Arc::new(Eventfd { count: Mutex::new(initial), semaphore, watchers: Watchers::default() })
    }

    pub fn readable(&self) -> bool {
        *lock(&self.count) > 0
    }

    pub fn writable(&self) -> bool {
        *lock(&self.count) < MOST
    }

    /// Takes what a read gets, if it wouldn't wait.
    pub fn take(&self) -> Option<u64> {
        let mut count = lock(&self.count);
        if *count == 0 {
            return None;
        }
        let taken = if self.semaphore { 1 } else { *count };
        *count -= taken;
        drop(count);
        self.watchers.notify();
        Some(taken)
    }

    /// Adds what a write gives, if it fits without waiting.
    pub fn add(&self, value: u64) -> Result<bool, Errno> {
        if value == u64::MAX {
            return Err(EINVAL);
        }
        let mut count = lock(&self.count);
        if MOST - *count < value {
            return Ok(false);
        }
        *count += value;
        drop(count);
        if value > 0 {
            self.watchers.notify();
        }
        Ok(true)
    }
}

struct Timer {
    /// When it next expires, on its clock, in nanoseconds; None while it's disarmed.
    next: Option<u64>,
    /// How often it expires after that; 0 for once.
    interval: u64,
    /// Expiries not yet read.
    expired: u64,
}

impl Timer {
    /// Counts the expiries up to `now`; whether there were any new ones.
    fn tick(&mut self, now: u64) -> bool {
        let Some(next) = self.next else { return false };
        if now < next {
            return false;
        }
        match (now - next).checked_div(self.interval) {
            Some(more) => {
                self.expired = self.expired.saturating_add(more + 1);
                self.next = Some(next.saturating_add((more + 1).saturating_mul(self.interval)));
            }
            None => {
                self.expired = self.expired.saturating_add(1);
                self.next = None;
            }
        }
        true
    }
}

pub(crate) struct Timerfd {
    /// On the wall clock (`CLOCK_REALTIME`), else the monotonic one.
    pub realtime: bool,
    timer: Mutex<Timer>,
    pub watchers: Watchers,
}

impl Timerfd {
    pub fn new(realtime: bool) -> Arc<Timerfd> {
        Arc::new(Timerfd { realtime, timer: Mutex::new(Timer { next: None, interval: 0, expired: 0 }), watchers: Watchers::default() })
    }

    /// Counts its expiries up to `now`, telling its watchers if there were new ones; then `f` of the timer.
    fn ticked<R>(&self, now: u64, f: impl FnOnce(&mut Timer) -> R) -> R {
        let mut timer = lock(&self.timer);
        let new = timer.tick(now);
        let result = f(&mut timer);
        drop(timer);
        if new {
            self.watchers.notify();
        }
        result
    }

    /// Whether it has expired since it was last read, as of `now` on its clock.
    pub fn readable(&self, now: u64) -> bool {
        self.ticked(now, |timer| timer.expired > 0)
    }

    /// Takes the expiries since the last read, if there are any.
    pub fn take(&self, now: u64) -> Option<u64> {
        self.ticked(now, |timer| (timer.expired > 0).then(|| std::mem::take(&mut timer.expired)))
    }

    /// How long from `now` until it next expires, if it's armed.
    pub fn until_next(&self, now: u64) -> Option<Duration> {
        lock(&self.timer).next.map(|next| Duration::from_nanos(next.saturating_sub(now)))
    }

    /// What's left until it expires and its interval, in nanoseconds, as `timerfd_gettime` says.
    pub fn get(&self, now: u64) -> (u64, u64) {
        self.ticked(now, |timer| (timer.next.map_or(0, |next| next.saturating_sub(now)), timer.interval))
    }

    /// Arms it to expire at `value` (from `now`, unless `absolute`) and every `interval` after, or disarms it for a
    /// `value` of 0. Gives what `get` did before.
    pub fn set(&self, now: u64, value: u64, interval: u64, absolute: bool) -> (u64, u64) {
        let old = self.get(now);
        let mut timer = lock(&self.timer);
        timer.expired = 0;
        timer.interval = interval;
        timer.next = (value > 0).then(|| if absolute { value } else { now.saturating_add(value) });
        drop(timer);
        self.watchers.notify();
        old
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_eventfd_counts() {
        let e = Eventfd::new(2, false);
        assert_eq!(e.take(), Some(2));
        assert_eq!(e.take(), None);
        assert_eq!(e.add(5), Ok(true));
        assert_eq!(e.add(u64::MAX), Err(EINVAL));
        assert_eq!(e.add(MOST), Ok(false));
        let s = Eventfd::new(3, true);
        assert_eq!((s.take(), s.take(), s.take(), s.take()), (Some(1), Some(1), Some(1), None));
    }

    #[test]
    fn a_timer_counts_its_expiries() {
        let t = Timerfd::new(false);
        assert!(!t.readable(0));
        t.set(1_000, 500, 100, false);
        assert_eq!(t.until_next(1_000), Some(Duration::from_nanos(500)));
        assert!(!t.readable(1_499));
        // At 1,500 and every 100 after: four by 1,850.
        assert_eq!(t.take(1_850), Some(4));
        assert_eq!(t.get(1_850), (50, 100));
        t.set(2_000, 0, 0, false);
        assert_eq!(t.take(10_000), None);
        t.set(0, 3_000, 0, true);
        assert_eq!(t.take(5_000), Some(1));
        assert_eq!(t.until_next(5_000), None);
    }
}
