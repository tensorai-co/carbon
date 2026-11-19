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
    /// Another thread wants a CPU: this one gives its up (`thread`'s seats) and goes on waiting.
    nudged: bool,
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

    /// Asks it, while it waits, to give its CPU up for a thread that needs one. Not a wake: it goes on waiting.
    pub(crate) fn nudge(&self) {
        lock(&self.state).nudged = true;
        self.ready.notify_all();
    }

    /// Forgets a wake that came before the thread started waiting for this.
    pub fn reset(&self) {
        lock(&self.state).woken = false;
    }

    /// Waits until woken, interrupted, or `deadline`. Woken wins over interrupted, as on Linux: what was waited for
    /// happened, and the interruption (it stays) is dealt with next.
    /// While it waits, a guest thread's CPU may be asked for by a thread that has none (`thread`'s seats).
    pub fn wait(&self, deadline: Option<Instant>) -> Woke {
        // An ask left over from a wait that ended first is no longer meant.
        lock(&self.state).nudged = false;
        let mut blocking = crate::thread::Blocking::begin(self);
        let mut state = lock(&self.state);
        loop {
            if state.woken {
                state.woken = false;
                return Woke::Woken;
            }
            if state.interrupted {
                return Woke::Interrupted;
            }
            if state.nudged {
                state.nudged = false;
                drop(state);
                blocking.give_up();
                state = lock(&self.state);
                continue;
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

fn watching() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CARBON_EXEC_WATCH").is_some())
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

/// A futex's key for an address in a process's own memory: its space and the address, whole. (A key that squeezed
/// both into 64 bits once kept only 4 bits of the space, and processes running the same program, whose futexes are at
/// the same addresses, woke each other's waiters and lost their own wakes.) The address's top byte, a pointer tag, is
/// dropped, as Linux's futexes drop it.
pub fn private_key(space: u64, addr: u64) -> u128 {
    u128::from(space) << 64 | u128::from(addr & 0x00ff_ffff_ffff_ffff)
}

/// A futex's key for a place in shared memory, as every process that maps it sees it: the memory, and the offset.
pub fn shared_key(memory: u64, offset: u64) -> u128 {
    1 << 127 | u128::from(memory) << 64 | u128::from(offset)
}

/// Waits keyed by an address: Linux's futexes, Windows' `WaitOnAddress` and keyed events.
#[derive(Default)]
pub struct Futexes {
    waiting: Mutex<HashMap<u128, Vec<Arc<Waiter>>>>,
    /// `CARBON_EXEC_WATCH`: each key's last wake, when, by which thread, and how many it woke.
    last_wake: Mutex<HashMap<u128, (Instant, String, usize)>>,
}

/// The value wasn't what the waiter expected, so it didn't wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changed;

impl Futexes {
    /// Waits on `key`, if `still()` says to. `still` runs with this table locked (it may take a turn to read the
    /// program's memory), so no wake can come between it and the wait.
    pub fn wait(&self, key: u128, me: &Arc<Waiter>, still: impl FnOnce() -> bool, deadline: Option<Instant>) -> Result<Woke, Changed> {
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

    /// Waits on every one of `keys` at once (`futex_waitv`), if `still()` says to, as `wait` does. Gives how the wait
    /// ended and, if a wake ended it, the first of `keys` it was woken by.
    pub fn wait_any(&self, keys: &[u128], me: &Arc<Waiter>, still: impl FnOnce() -> bool, deadline: Option<Instant>) -> Result<(Woke, Option<usize>), Changed> {
        {
            let mut waiting = lock(&self.waiting);
            if !still() {
                return Err(Changed);
            }
            me.reset();
            for key in keys {
                waiting.entry(*key).or_default().push(me.clone());
            }
        }
        let woke = me.wait(deadline);
        // A wake took this waiter off its key's queue; it's still on the others', which it leaves.
        let mut waiting = lock(&self.waiting);
        let mut by = None;
        for (i, key) in keys.iter().enumerate() {
            let (still_there, empty) = match waiting.get_mut(key) {
                Some(queue) => {
                    let before = queue.len();
                    queue.retain(|w| !Arc::ptr_eq(w, me));
                    (before != queue.len(), queue.is_empty())
                }
                None => (false, false),
            };
            if empty {
                waiting.remove(key);
            }
            if !still_there && by.is_none() {
                by = Some(i);
            }
        }
        Ok((woke, if woke == Woke::Woken { by } else { None }))
    }

    /// `CARBON_EXEC_WATCH`: what's known of `key`'s last wake: how long ago, by which thread, how many it woke.
    pub fn last_wake(&self, key: u128) -> Option<(std::time::Duration, String, usize)> {
        lock(&self.last_wake).get(&key).map(|(at, who, n)| (at.elapsed(), who.clone(), *n))
    }

    /// Whether `waiter` is on `key`'s queue now.
    pub fn queued(&self, key: u128, waiter: &Arc<Waiter>) -> bool {
        lock(&self.waiting).get(&key).is_some_and(|q| q.iter().any(|w| Arc::ptr_eq(w, waiter)))
    }

    /// Wakes up to `count` threads waiting on `key`, first come first woken, giving how many.
    pub fn wake(&self, key: u128, count: usize) -> usize {
        let mut waiting = lock(&self.waiting);
        let watching = watching();
        let Some(queue) = waiting.get_mut(&key) else {
            if watching {
                lock(&self.last_wake).insert(key, (Instant::now(), std::thread::current().name().unwrap_or("?").to_string(), 0));
            }
            return 0;
        };
        let n = count.min(queue.len());
        if watching {
            lock(&self.last_wake).insert(key, (Instant::now(), std::thread::current().name().unwrap_or("?").to_string(), n));
        }
        for waiter in queue.drain(..n) {
            waiter.wake();
        }
        if queue.is_empty() {
            waiting.remove(&key);
        }
        n
    }

    /// Moves up to `count` waiters from `from` to `to`, without waking them (`FUTEX_REQUEUE`).
    pub fn requeue(&self, from: u128, to: u128, count: usize) -> usize {
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
    fn wait_on_several_and_learn_which() {
        let futexes = Arc::new(Futexes::default());
        let me = Waiter::new();
        assert_eq!(futexes.wait_any(&[1, 2], &me, || false, None), Err(Changed));
        let (f, waiter) = (futexes.clone(), me.clone());
        let waking = std::thread::spawn(move || {
            while f.wake(30, 1) == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
            drop(waiter);
        });
        assert_eq!(futexes.wait_any(&[10, 20, 30], &me, || true, None), Ok((Woke::Woken, Some(2))));
        waking.join().unwrap();
        // It left the others' queues: nothing's waiting on 10 or 20.
        assert_eq!(futexes.wake(10, 1) + futexes.wake(20, 1), 0);
        let soon = Instant::now() + Duration::from_millis(10);
        assert_eq!(futexes.wait_any(&[40], &me, || true, Some(soon)), Ok((Woke::TimedOut, None)));
    }

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

    #[test]
    fn keys_tell_processes_apart() {
        // Two spaces whose roots share bits 12-15, and one address: the old key, `root << 48 ^ addr`, was the same for
        // both, so one process's wake went to the other's waiter.
        let (a, b, addr) = (0x8000_1000u64, 0x8011_1000u64, 0x0000_4000_0008_0148u64);
        assert_eq!(a << 48 ^ addr, b << 48 ^ addr, "the old key collides");
        assert_ne!(super::private_key(a, addr), super::private_key(b, addr));
        // A tagged pointer is the same futex as the untagged one.
        assert_eq!(super::private_key(a, 0x5a00_4000_0008_0148), super::private_key(a, addr));
        // Shared memory's keys never meet a private one.
        assert_ne!(super::shared_key(7, 0x148), super::private_key(7, 0x148));
        // A wake on one process's key leaves the other's waiter be.
        let futexes = Futexes::default();
        let me = Waiter::new();
        assert_eq!(futexes.wake(super::private_key(a, addr), 1), 0);
        assert_eq!(futexes.wait(super::private_key(b, addr), &me, || true, Some(Instant::now() + Duration::from_millis(5))), Ok(Woke::TimedOut));
    }
}
