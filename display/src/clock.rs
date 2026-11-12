//! The desktop's clock, for a desktop with no host screen to follow: it ticks at a steady rate, waking the kernel with
//! `Frame` events, but only while some window is waiting for a frame. An idle desktop costs nothing: the clock's
//! thread sleeps until a commit asks for one. A host backend drives the desktop with its own display's refresh
//! instead, calling [`tick`] itself.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::desktop::{self, Shared};

/// Ticks the desktop at `time` milliseconds: every window waiting gets its `Frame`, and the kernel is woken to read
/// them, without the desktop's lock held.
pub fn tick(desktop: &Mutex<desktop::Desktop>, time: u32) {
    let waker = {
        let mut d = desktop::lock(desktop);
        d.frame(time);
        if d.has_events() { d.waker() } else { None }
    };
    if let Some(wake) = waker {
        wake();
    }
}

struct Signal {
    waiting: Mutex<bool>,
    ready: Condvar,
    stop: AtomicBool,
}

/// A running clock: dropping it stops its thread.
pub struct Clock {
    signal: Arc<Signal>,
    thread: Option<JoinHandle<()>>,
}

impl Clock {
    /// Starts a clock for `desktop` at `hz` ticks a second.
    pub fn start(desktop: Shared, hz: u32) -> Clock {
        let signal = Arc::new(Signal { waiting: Mutex::new(false), ready: Condvar::new(), stop: AtomicBool::new(false) });
        {
            let signal = signal.clone();
            desktop::lock(&desktop).set_clock(Arc::new(move || {
                *signal.waiting.lock().unwrap_or_else(|p| p.into_inner()) = true;
                signal.ready.notify_all();
            }));
        }
        let period = Duration::from_nanos(1_000_000_000 / u64::from(hz.max(1)));
        let thread = {
            let signal = signal.clone();
            std::thread::Builder::new().name("display clock".into()).spawn(move || run(&desktop, &signal, period)).ok()
        };
        Clock { signal, thread }
    }
}

fn run(desktop: &Shared, signal: &Signal, period: Duration) {
    let start = Instant::now();
    loop {
        {
            let mut waiting = signal.waiting.lock().unwrap_or_else(|p| p.into_inner());
            while !*waiting && !signal.stop.load(Ordering::Acquire) {
                waiting = signal.ready.wait(waiting).unwrap_or_else(|p| p.into_inner());
            }
            *waiting = false;
        }
        if signal.stop.load(Ordering::Acquire) {
            return;
        }
        // The next tick on the clock's own grid, so frames come evenly however late a commit was.
        loop {
            let elapsed = start.elapsed();
            let next = period * (elapsed.as_nanos() / period.as_nanos() + 1) as u32;
            std::thread::sleep(next.saturating_sub(elapsed));
            if signal.stop.load(Ordering::Acquire) {
                return;
            }
            tick(desktop, start.elapsed().as_millis() as u32);
            // A window that drew and asked again keeps the clock going; otherwise it sleeps until one asks.
            if !desktop::lock(desktop).wants_frame() {
                break;
            }
        }
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        self.signal.stop.store(true, Ordering::Release);
        self.signal.ready.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
