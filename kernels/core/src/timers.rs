//! Timers that fire with no thread waiting for them: lambda's `alarm`, interval timers and POSIX timers, and atlas's
//! waitable timers and `SetTimer` to come. One host thread per machine keeps them in order and runs each one's
//! callback when it's due, with nothing locked, so a callback may arm another (a periodic timer arms its next
//! expiry). It starts with the first timer and ends with `stop`.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use crate::lock;

/// A timer that's armed, to cancel it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimerId(u64);

type Callback = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    /// By when each is due, then by when it was armed.
    due: BTreeMap<(Instant, u64), Callback>,
    /// Each armed timer's key in `due`.
    keys: BTreeMap<u64, Instant>,
    next: u64,
    started: bool,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// A machine's timers.
#[derive(Clone, Default)]
pub struct Timers(Arc<Shared>);

impl Timers {
    /// Runs `callback` on the timers' thread at `when` (at once if that's past), unless it's cancelled first.
    pub fn at(&self, when: Instant, callback: impl FnOnce() + Send + 'static) -> TimerId {
        let mut state = lock(&self.0.state);
        let id = state.next;
        state.next += 1;
        if state.stopped {
            return TimerId(id);
        }
        state.due.insert((when, id), Box::new(callback));
        state.keys.insert(id, when);
        if !state.started {
            state.started = true;
            let shared = self.0.clone();
            let started = std::thread::Builder::new().name("timers".into()).spawn(move || run(&shared));
            if started.is_err() {
                state.started = false;
            }
        }
        self.0.changed.notify_all();
        TimerId(id)
    }

    /// Cancels a timer: whether it hadn't fired yet.
    pub fn cancel(&self, id: TimerId) -> bool {
        let mut state = lock(&self.0.state);
        match state.keys.remove(&id.0) {
            Some(when) => state.due.remove(&(when, id.0)).is_some(),
            None => false,
        }
    }

    /// Cancels every timer and ends the thread: the machine has stopped.
    pub fn stop(&self) {
        let mut state = lock(&self.0.state);
        state.stopped = true;
        state.due.clear();
        state.keys.clear();
        self.0.changed.notify_all();
    }
}

fn run(shared: &Shared) {
    let mut state = lock(&shared.state);
    loop {
        if state.stopped {
            return;
        }
        let now = Instant::now();
        match state.due.keys().next().copied() {
            Some((when, id)) if when <= now => {
                let callback = state.due.remove(&(when, id));
                state.keys.remove(&id);
                drop(state);
                if let Some(callback) = callback {
                    callback();
                }
                state = lock(&shared.state);
            }
            Some((when, _)) => state = shared.changed.wait_timeout(state, when - now).map(|(s, _)| s).unwrap_or_else(|p| p.into_inner().0),
            None => state = shared.changed.wait(state).unwrap_or_else(|p| p.into_inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn fire_in_order_cancel_and_rearm() {
        let timers = Timers::default();
        let (tx, rx) = mpsc::channel();
        let now = Instant::now();
        for (n, ms) in [(3, 30), (1, 10), (2, 20)] {
            let tx = tx.clone();
            timers.at(now + Duration::from_millis(ms), move || tx.send(n).unwrap());
        }
        let tx2 = tx.clone();
        let cancelled = timers.at(now + Duration::from_millis(15), move || tx2.send(99).unwrap());
        assert!(timers.cancel(cancelled));
        // A callback arming the next one, as a periodic timer does.
        let (again, tx3) = (timers.clone(), tx.clone());
        timers.at(now + Duration::from_millis(40), move || {
            again.at(Instant::now() + Duration::from_millis(5), move || tx3.send(5).unwrap());
        });
        let got: Vec<i32> = (0..4).map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        assert_eq!(got, [1, 2, 3, 5]);
        timers.stop();
        timers.at(Instant::now(), move || tx.send(7).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }
}
