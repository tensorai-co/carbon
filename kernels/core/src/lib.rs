//! What both kernels share, with nothing of Linux or Windows in it. lambda and atlas know nothing of each other, but
//! they solve the same problems on the same `exec`:
//!
//! - [`thread`]: a guest thread is a host thread that owns its exec `Cpu`. Its turns, and the run loop.
//! - [`wait`]: one way to wait, interruptible and with a deadline, and futex-style waits keyed by address.
//! - [`mm`]: a map of an address space's regions, which each kernel fills with its own rules.
//! - [`user`]: copying in and out of program memory, with faults as errors.
//! - [`frame`]: building what goes on a program's stack.
//! - [`call`]: where a call's arguments are, for each ISA and calling convention.
//! - [`cache`]: the page cache: one frame per page of each file, shared copy-on-write.
//!
//! ## Locks
//!
//! Take them in this order, and never hold one across a wait: a kernel's process table, then one process, then a
//! shared object (a pipe, a futex table, the file system, the page cache), and exec's lock last, only for a turn.
//! [`thread::Cx::turn`] is the only way to take exec's lock, and it holds it only while its closure runs.

pub mod cache;
pub mod call;
pub mod frame;
pub mod mm;
pub mod thread;
pub mod user;
pub mod wait;

use std::sync::{Mutex, MutexGuard};

/// Locks a mutex, carrying on if a thread panicked while holding it: a kernel's state is still the best there is.
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
