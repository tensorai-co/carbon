//! Time slices: a thread that preempts every running CPU now and then, while they're on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::cpu::CpuShared;

pub(crate) struct Ticker {
    on: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    pub fn start(cpus: Arc<Mutex<Vec<Arc<CpuShared>>>>, slice: Duration) -> Ticker {
        let (on, stop) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (thread_on, thread_stop) = (on.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("exec-time-slices".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    if !thread_on.load(Ordering::Acquire) {
                        thread::park();
                        continue;
                    }
                    thread::sleep(slice);
                    for cpu in cpus.lock().unwrap_or_else(PoisonError::into_inner).iter() {
                        if cpu.running.load(Ordering::Acquire) {
                            cpu.preempt();
                        }
                    }
                }
            })
            .ok();
        Ticker { on, stop, thread }
    }

    pub fn set(&self, on: bool) {
        if self.on.swap(on, Ordering::AcqRel) != on
            && on
            && let Some(thread) = &self.thread
        {
            thread.thread().unpark();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
