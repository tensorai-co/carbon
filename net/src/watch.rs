//! Whoever wants telling when a socket's readiness may have changed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::{Waker, lock};

#[derive(Default)]
pub(crate) struct Watchers {
    list: Mutex<Vec<(u64, Waker)>>,
    next: AtomicU64,
}

impl Watchers {
    pub fn add(self: &Arc<Self>, waker: Waker) -> Watch {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.list).push((id, waker));
        Watch { id, watchers: Arc::downgrade(self) }
    }

    /// Tells every watcher, with the list unlocked.
    pub fn notify(&self) {
        let wakers: Vec<Waker> = lock(&self.list).iter().map(|(_, w)| w.clone()).collect();
        for wake in wakers {
            wake();
        }
    }
}

/// A watch on a socket, from [`crate::Socket::watch`]: it ends when this is dropped.
pub struct Watch {
    id: u64,
    watchers: Weak<Watchers>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(watchers) = self.watchers.upgrade() {
            lock(&watchers.list).retain(|(id, _)| *id != self.id);
        }
    }
}

/// A waker that tells `watchers`, if they're still there.
pub(crate) fn waker_for(watchers: &Arc<Watchers>) -> Waker {
    let weak = Arc::downgrade(watchers);
    Arc::new(move || {
        if let Some(w) = weak.upgrade() {
            w.notify();
        }
    })
}
