//! What macOS and Linux do alike: the terminal and randomness.

use std::sync::Mutex;

/// Puts stdin's terminal in raw mode, or back as it was.
pub fn set_raw(raw: bool) {
    static SAVED: Mutex<Option<libc::termios>> = Mutex::new(None);
    let mut saved = SAVED.lock().unwrap_or_else(|p| p.into_inner());
    // SAFETY: termios structures for stdin, read and written whole.
    unsafe {
        if raw {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return;
            }
            saved.get_or_insert(t);
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(0, libc::TCSANOW, &t);
        } else if let Some(t) = saved.take() {
            libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
}

/// The CPU time the calling thread has used, in nanoseconds.
pub fn thread_cpu() -> u64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a timespec to fill in, for a clock every supported Unix has.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) } != 0 {
        return 0;
    }
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

/// stdout's terminal's rows and columns.
pub fn terminal_size() -> Option<(u16, u16)> {
    // SAFETY: a winsize for stdout's terminal to fill in.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(1, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_row > 0).then_some((size.ws_row, size.ws_col))
    }
}

/// The kernel's random bytes, at most 256 at a time as `getentropy` allows.
pub fn random(buffer: &mut [u8]) {
    for chunk in buffer.chunks_mut(256) {
        // SAFETY: room for `chunk.len()` bytes, at most 256.
        let ok = unsafe { libc::getentropy(chunk.as_mut_ptr().cast(), chunk.len()) } == 0;
        assert!(ok, "the host's kernel gave no random bytes");
    }
}
