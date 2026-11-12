//! Signals to the `carbon` process, passed on to the machine: SIGTERM (`carbon stop`), SIGINT and SIGHUP go to its
//! first process; a second one, or ten seconds without it ending, ends everything. With the console in raw mode, ^C
//! is a key the machine reads, not a signal, so this is for when it isn't a terminal and for other processes' signals.

use std::sync::{Arc, Mutex};
use std::time::Duration;

type Handler = Box<dyn Fn(i32, bool) + Send>;

/// Holds the signals blocked in every thread but its own, which waits for them.
pub struct Relay {
    set: libc::sigset_t,
    handler: Arc<Mutex<Option<Handler>>>,
}

fn set() -> libc::sigset_t {
    // SAFETY: a set made empty, then given three signals.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        set
    }
}

/// Sends this process a signal, as if from outside: the relay takes it as it would Ctrl-C's.
pub fn raise(signal: i32) {
    // SAFETY: a signal to this process.
    unsafe { libc::kill(libc::getpid(), signal) };
}

impl Relay {
    /// Blocks the signals in this thread, and so in every thread it makes from now on, and starts the one that waits.
    pub fn start() -> Relay {
        let set = set();
        // SAFETY: this thread's mask, from a valid set.
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
        let handler: Arc<Mutex<Option<Handler>>> = Arc::new(Mutex::new(None));
        let waiting = handler.clone();
        let _ = std::thread::Builder::new().name("carbon signals".into()).spawn(move || {
            let set = self::set();
            let mut first = true;
            loop {
                let mut signal = 0;
                // SAFETY: waits for one of the set's signals, which are blocked here too.
                if unsafe { libc::sigwait(&set, &mut signal) } != 0 {
                    return;
                }
                let call = |signal, first| {
                    if let Some(handler) = waiting.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                        handler(signal, first);
                    }
                };
                call(signal, first);
                if first {
                    first = false;
                    // Ten seconds to end on its own, then it's ended.
                    let later = waiting.clone();
                    let _ = std::thread::Builder::new().name("carbon stop timer".into()).spawn(move || {
                        std::thread::sleep(Duration::from_secs(10));
                        if let Some(handler) = later.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                            handler(libc::SIGKILL, false);
                        }
                    });
                }
            }
        });
        Relay { set, handler }
    }

    /// What to do with a signal: `first` for the first one, else it's time to end everything.
    pub fn on_signal(&self, handler: impl Fn(i32, bool) + Send + 'static) {
        *self.handler.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(handler));
    }
}

impl Drop for Relay {
    /// The machine has ended: signals do what they always do again.
    fn drop(&mut self) {
        *self.handler.lock().unwrap_or_else(|p| p.into_inner()) = None;
        // SAFETY: this thread's mask, from a valid set.
        unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &self.set, std::ptr::null_mut()) };
    }
}
