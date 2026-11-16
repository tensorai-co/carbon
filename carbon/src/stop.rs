//! How a running machine is stopped from outside its kernel's threads: `Running::stop`, `carbon stop` (over the
//! control socket), and the signals `carbon run` relays. Stopping asks its first process to end (SIGTERM, in a Linux
//! machine), and ends everything ten seconds later if it hasn't; killing ends everything now.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a machine has to end on its own once asked, as `docker stop` gives it.
pub(crate) const GRACE: Duration = Duration::from_secs(10);

/// A kernel's way in.
#[derive(Clone)]
pub(crate) enum Handle {
    Lambda(lambda::Stop),
    Atlas(atlas::Stop),
}

#[derive(Default)]
struct Inner {
    handle: Mutex<Option<Handle>>,
    /// The machine has ended.
    done: AtomicBool,
    /// It's been asked to stop once already.
    asked: AtomicBool,
}

/// Shared by everything that may stop one machine.
#[derive(Clone, Default)]
pub(crate) struct Stopper(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Stopper {
    pub fn set(&self, handle: Handle) {
        *lock(&self.0.handle) = Some(handle);
    }

    pub fn finished(&self) {
        self.0.done.store(true, Ordering::Release);
    }

    pub fn is_done(&self) -> bool {
        self.0.done.load(Ordering::Acquire)
    }

    /// Waits up to `timeout` for its kernel to be up (or for it to have ended already). Whether it's up.
    pub fn wait_up(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let up = match lock(&self.0.handle).as_ref() {
                Some(Handle::Lambda(stop)) => stop.is_running(),
                Some(Handle::Atlas(stop)) => stop.is_running(),
                None => false,
            };
            if up || self.is_done() {
                return up;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Asks the machine to end: `signal` to a Linux machine's first process (SIGTERM, SIGINT or SIGHUP), and a
    /// Windows machine ended; a second ask, or `GRACE` without ending, ends everything.
    pub fn stop(&self, signal: i32) {
        if self.0.asked.swap(true, Ordering::AcqRel) {
            return self.kill();
        }
        match lock(&self.0.handle).clone() {
            Some(Handle::Lambda(stop)) => {
                stop.signal(signal);
            }
            Some(Handle::Atlas(stop)) => {
                stop.end(1);
            }
            None => {}
        }
        let later = self.clone();
        let _ = std::thread::Builder::new().name("carbon stop timer".into()).spawn(move || {
            let step = Duration::from_millis(100);
            let mut waited = Duration::ZERO;
            while waited < GRACE {
                if later.is_done() {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
            }
            later.kill();
        });
    }

    /// Ends everything in the machine now.
    pub fn kill(&self) {
        match lock(&self.0.handle).clone() {
            Some(Handle::Lambda(stop)) => {
                stop.end();
            }
            Some(Handle::Atlas(stop)) => {
                stop.end(137);
            }
            None => {}
        }
    }
}
