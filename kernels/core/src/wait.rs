//! One way to wait. A guest thread waiting on anything (a futex, a pipe, a child, a Windows event, a timer) waits on
//! its own [`Waiter`], which whatever it waits for wakes, and which a signal or APC can interrupt.
//!
//! Something that can become ready (a pipe with data, a terminal with a line, a signal arriving) keeps [`Watchers`]:
//! whoever wants telling when its readiness may have changed. A thread in `poll` adds its waiter to each file's
//! watchers while it waits; an `epoll` instance adds itself to each file it watches for as long as it watches it.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Instant;

use crate::lock;

/// Why a wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woke {
    Woken,
    Interrupted,
    TimedOut,
}

#[derive(Default)]
struct State {
    woken: bool,
    interrupted: bool,
}

/// One guest thread's way to wait.
#[derive(Default)]
pub struct Waiter {
    state: Mutex<State>,
    ready: Condvar,
}

impl Waiter {
    pub fn new() -> Arc<Waiter> {
        Arc::new(Waiter::default())
    }

    /// Ends its wait, or the next one, as woken.
    pub fn wake(&self) {
        lock(&self.state).woken = true;
        self.ready.notify_all();
    }

    /// Ends its wait, and every wait until `clear_interrupt`, as interrupted.
    pub fn interrupt(&self) {
        lock(&self.state).interrupted = true;
        self.ready.notify_all();
    }

    pub fn clear_interrupt(&self) {
        lock(&self.state).interrupted = false;
    }

    pub fn is_interrupted(&self) -> bool {
        lock(&self.state).interrupted
    }

    /// Forgets a wake that came before the thread started waiting for this.
    pub fn reset(&self) {
        lock(&self.state).woken = false;
    }

    /// Waits until woken, interrupted, or `deadline`. Woken wins over interrupted, as on Linux: what was waited for
    /// happened, and the interruption (it stays) is dealt with next.
    pub fn wait(&self, deadline: Option<Instant>) -> Woke {
        let mut state = lock(&self.state);
        loop {
            if state.woken {
                state.woken = false;
                return Woke::Woken;
            }
            if state.interrupted {
                return Woke::Interrupted;
            }
            match deadline {
                None => state = self.ready.wait(state).unwrap_or_else(|p| p.into_inner()),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Woke::TimedOut;
                    }
                    state = self.ready.wait_timeout(state, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
                }
            }
        }
    }
}

/// Something to tell when something it watches may have changed.
pub trait Wake: Send + Sync {
    fn wake(&self);
}

impl Wake for Waiter {
    fn wake(&self) {
        Waiter::wake(self);
    }
}

/// Who to tell when something's readiness may have changed. Held weakly: a watcher that's gone is dropped.
#[derive(Default)]
pub struct Watchers {
    list: Mutex<Vec<Weak<dyn Wake>>>,
}

impl Watchers {
    pub fn add(&self, watcher: Weak<dyn Wake>) {
        let mut list = lock(&self.list);
        list.retain(|w| w.strong_count() > 0);
        list.push(watcher);
    }

    pub fn remove(&self, watcher: &Weak<dyn Wake>) {
        lock(&self.list).retain(|w| w.strong_count() > 0 && !std::ptr::addr_eq(w.as_ptr(), watcher.as_ptr()));
    }

    /// Tells every watcher. They're told with the list unlocked, so one can add or remove itself as it's told.
    pub fn notify(&self) {
        let live: Vec<Arc<dyn Wake>> = lock(&self.list).iter().filter_map(Weak::upgrade).collect();
        for watcher in live {
            watcher.wake();
        }
    }
}

/// Waits keyed by an address: Linux's futexes, Windows' `WaitOnAddress` and keyed events.
#[derive(Default)]
pub struct Futexes {
    waiting: Mutex<HashMap<u64, Vec<Arc<Waiter>>>>,
}

/// The value wasn't what the waiter expected, so it didn't wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changed;

impl Futexes {
    /// Waits on `key`, if `still()` says to. `still` runs with this table locked (it may take a turn to read the
    /// program's memory), so no wake can come between it and the wait.
    pub fn wait(&self, key: u64, me: &Arc<Waiter>, still: impl FnOnce() -> bool, deadline: Option<Instant>) -> Result<Woke, Changed> {
        {
            let mut waiting = lock(&self.waiting);
            if !still() {
                return Err(Changed);
            }
            me.reset();
            waiting.entry(key).or_default().push(me.clone());
        }
        let woke = me.wait(deadline);
        if woke != Woke::Woken {
            let mut waiting = lock(&self.waiting);
            if let Some(queue) = waiting.get_mut(&key) {
                queue.retain(|w| !Arc::ptr_eq(w, me));
                if queue.is_empty() {
                    waiting.remove(&key);
                }
            }
        }
        Ok(woke)
    }

    /// Wakes up to `count` threads waiting on `key`, first come first woken, giving how many.
    pub fn wake(&self, key: u64, count: usize) -> usize {
        let mut waiting = lock(&self.waiting);
        let Some(queue) = waiting.get_mut(&key) else { return 0 };
        let n = count.min(queue.len());
        for waiter in queue.drain(..n) {
            waiter.wake();
        }
        if queue.is_empty() {
            waiting.remove(&key);
        }
        n
    }

    /// Moves up to `count` waiters from `from` to `to`, without waking them (`FUTEX_REQUEUE`).
    pub fn requeue(&self, from: u64, to: u64, count: usize) -> usize {
        let mut waiting = lock(&self.waiting);
        let Some(queue) = waiting.get_mut(&from) else { return 0 };
        let n = count.min(queue.len());
        let moved: Vec<_> = queue.drain(..n).collect();
        if queue.is_empty() {
            waiting.remove(&from);
        }
        waiting.entry(to).or_default().extend(moved);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn wake_and_time_out() {
        let futexes = Arc::new(Futexes::default());
        let me = Waiter::new();
        assert_eq!(futexes.wait(1, &me, || false, None), Err(Changed));
        assert_eq!(futexes.wait(1, &me, || true, Some(Instant::now() + Duration::from_millis(5))), Ok(Woke::TimedOut));
        let (f, w) = (futexes.clone(), me.clone());
        let waiter = std::thread::spawn(move || f.wait(2, &w, || true, None));
        while futexes.wake(2, 1) == 0 {
            std::thread::yield_now();
        }
        assert_eq!(waiter.join().unwrap(), Ok(Woke::Woken));
        let w = me.clone();
        let interrupted = std::thread::spawn(move || w.wait(None));
        me.interrupt();
        assert_eq!(interrupted.join().unwrap(), Woke::Interrupted);
    }

    #[test]
    fn watchers_tell_whoever_is_still_there() {
        let watchers = Watchers::default();
        let (a, b) = (Waiter::new(), Waiter::new());
        let (wa, wb): (Weak<dyn Wake>, Weak<dyn Wake>) = (Arc::downgrade(&a) as Weak<dyn Wake>, Arc::downgrade(&b) as Weak<dyn Wake>);
        watchers.add(wa);
        watchers.add(wb.clone());
        watchers.notify();
        assert_eq!(a.wait(Some(Instant::now())), Woke::Woken);
        assert_eq!(b.wait(Some(Instant::now())), Woke::Woken);
        watchers.remove(&wb);
        watchers.notify();
        assert_eq!(b.wait(Some(Instant::now())), Woke::TimedOut);
        // A watcher that's gone is skipped.
        drop(a);
        watchers.notify();
    }
}
