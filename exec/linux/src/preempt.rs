//! The real-time signal kvm preempts a vCPU's thread with: it does nothing, but interrupts `KVM_RUN`. (The sandbox
//! signals its stub instead.)

use std::sync::Once;

/// SIGRTMIN + 3, which nothing else here uses.
pub fn signal() -> i32 {
    libc::SIGRTMIN() + 3
}

extern "C" fn ignore(_: i32) {}

/// Installs the handler, once per process: without `SA_RESTART`, so what it interrupts comes back with `EINTR`.
pub fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        // SAFETY: a handler that does nothing.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = ignore as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal(), &action, std::ptr::null_mut());
        }
    });
}

/// Interrupts a thread.
pub fn thread(thread: libc::pthread_t) {
    // SAFETY: a thread that exists for as long as its CPU, and a signal whose handler does nothing.
    unsafe { libc::pthread_kill(thread, signal()) };
}
