//! Carbon's side of the memory a stub shares with it: the mailbox and the signal stack (`handler.rs` has the layout
//! and the protocol). A memfd of its own per stub, mapped here and at `SHARED` in the stub.
//!
//! Everything in it is the program's to write, as it's in the stub's address space, so it's read as untrusted: the
//! frame's pointers are checked to be on the signal stack, and the state to be one Carbon is waiting for.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::handler::{self, CARBON_CPU, CARBON_SLEEPING, CPU_SEGMENT, DONE, MAX_OPS, NOPS, OP_SIZE, OPS, SHARED_SIZE, SPIN, STATE, STUB_CPU, STUB_SLEEPING};

/// How long Carbon sleeps on the futex at a time before it looks whether the stub has died.
const WAKE_TO_CHECK: Duration = Duration::from_millis(100);
/// Neither side spins for an answer that's lately taken longer than this to come: a futex's sleep and wake cost less.
const SPIN_WHEN_UNDER: u64 = 30_000;
/// The longest either side spins, in nanoseconds.
const MOST_SPIN: u64 = 60_000;

/// How long each side spins before it sleeps on the futex. Spinning is the fastest handoff there is, but only while
/// the two are on different CPUs (on one, it keeps the other from running), and only where the answer comes soon:
/// inside gVisor every call the stub makes costs tens of microseconds, and on a small, busy host the CPU is better
/// given away. So each side spins only while the other's on another CPU (`handler::STUB_CPU`), and for about twice as
/// long as the answer has lately taken, if that's short: Carbon keeps a moving average of how long the stub takes to
/// come back to it, and of how long it takes to answer, which sets the stub's spin (`handler::SPIN`) each time.
/// `CARBON_SANDBOX_SPIN` fixes both at that many looks (0: never spin).
struct Spin {
    fixed: Option<u64>,
    /// Moving averages, in nanoseconds: from Carbon's answer to the stub's next event, and from an event to Carbon's
    /// answer.
    stub_takes: Cell<u64>,
    carbon_takes: Cell<u64>,
    told: Cell<Option<Instant>>,
    seen: Cell<Option<Instant>>,
}

impl Spin {
    /// The looks to spin for an answer that's lately taken `ns`.
    fn looks(&self, ns: u64) -> u64 {
        match self.fixed {
            Some(n) => n,
            None if ns > SPIN_WHEN_UNDER => 0,
            None => (2 * ns).min(MOST_SPIN) / ns_per_look(),
        }
    }
}

fn average(cell: &Cell<u64>, sample: u64) {
    let now = cell.get();
    cell.set(now - now / 8 + sample / 8);
}

/// How long one look of a spin takes on this host (a `pause` and a load), measured once.
fn ns_per_look() -> u64 {
    static NS: OnceLock<u64> = OnceLock::new();
    *NS.get_or_init(|| {
        let word = AtomicU32::new(0);
        let start = Instant::now();
        for _ in 0..20_000 {
            if word.load(Ordering::SeqCst) == 1 {
                break;
            }
            std::hint::spin_loop();
        }
        (start.elapsed().as_nanos() as u64 / 20_000).max(1)
    })
}

/// The host CPU this thread is on, from Linux's per-CPU segment as the vDSO's `getcpu` reads it, or `u32::MAX`.
fn host_cpu() -> u32 {
    let mut limit = CPU_SEGMENT;
    let found: u8;
    // SAFETY: `lsl` reads a segment's limit, or sets nothing but ZF if the selector isn't one.
    unsafe { std::arch::asm!("lsl {l:e}, {l:e}", "setz {f}", l = inout(reg) limit, f = out(reg_byte) found, options(nomem, nostack)) };
    if found != 0 { limit & 0xfff } else { u32::MAX }
}

pub struct Mailbox {
    /// The memfd, which the stub is given at its start.
    pub fd: i32,
    ptr: *mut u8,
    /// Where the shared memory is in the stub.
    pub stub_at: u64,
    spin: Spin,
}

// SAFETY: the mapping belongs to the Mailbox, and is unmapped once, on drop.
unsafe impl Send for Mailbox {}

/// Why a wait for the stub ended other than as asked.
#[derive(Debug)]
pub enum Gone {
    /// It ended, with this wait status.
    Ended(i32),
    /// It wrote something out of protocol.
    Garbled(String),
}

impl Mailbox {
    pub fn new(stub_at: u64) -> Result<Mailbox, String> {
        // SAFETY: a name and flags.
        let fd = unsafe { libc::memfd_create(c"carbon-mailbox".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(format!("making a stub's mailbox: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: the memfd just made, sized, then mapped shared.
        let ptr = unsafe {
            if libc::ftruncate(fd, SHARED_SIZE as libc::off_t) != 0 {
                let error = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(format!("sizing a stub's mailbox: {error}"));
            }
            libc::mmap(std::ptr::null_mut(), SHARED_SIZE as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if ptr == libc::MAP_FAILED {
            let error = std::io::Error::last_os_error();
            // SAFETY: as above.
            unsafe { libc::close(fd) };
            return Err(format!("mapping a stub's mailbox: {error}"));
        }
        let fixed = std::env::var("CARBON_SANDBOX_SPIN").ok().and_then(|v| v.parse::<u64>().ok());
        // Until it's measured, as if the answers came slowly: no spinning.
        let spin = Spin { fixed, stub_takes: Cell::new(SPIN_WHEN_UNDER * 2), carbon_takes: Cell::new(SPIN_WHEN_UNDER * 2), told: Cell::new(None), seen: Cell::new(None) };
        let mailbox = Mailbox { fd, ptr: ptr.cast(), stub_at, spin };
        mailbox.set(SPIN, mailbox.spin.looks(mailbox.spin.carbon_takes.get()));
        mailbox.word(STUB_CPU).store(u32::MAX, Ordering::Relaxed);
        mailbox.word(CARBON_CPU).store(u32::MAX, Ordering::Relaxed);
        Ok(mailbox)
    }

    fn word(&self, at: u64) -> &AtomicU32 {
        debug_assert!(at.is_multiple_of(4) && at + 4 <= SHARED_SIZE);
        // SAFETY: an aligned word of the mapping, which lives as long as self; shared with the stub, so atomic.
        unsafe { &*self.ptr.add(at as usize).cast::<AtomicU32>() }
    }

    pub fn get(&self, at: u64) -> u64 {
        debug_assert!(at + 8 <= SHARED_SIZE);
        // SAFETY: within the mapping; volatile, as the stub writes it.
        unsafe { self.ptr.add(at as usize).cast::<u64>().read_volatile() }
    }

    pub fn set(&self, at: u64, value: u64) {
        debug_assert!(at + 8 <= SHARED_SIZE);
        // SAFETY: as above.
        unsafe { self.ptr.add(at as usize).cast::<u64>().write_volatile(value) }
    }

    /// Bytes of the shared memory, at an offset from its start. The stub may change them meanwhile, but only while
    /// Carbon isn't looking: it's waiting in its handler whenever Carbon reads or writes the frame.
    pub fn bytes(&self, at: usize, len: usize) -> &[u8] {
        assert!(at + len <= SHARED_SIZE as usize);
        // SAFETY: within the mapping, which lives as long as self.
        unsafe { std::slice::from_raw_parts(self.ptr.add(at), len) }
    }

    pub fn bytes_mut(&mut self, at: usize, len: usize) -> &mut [u8] {
        assert!(at + len <= SHARED_SIZE as usize);
        // SAFETY: as above, and borrowed mutably from the one Mailbox that maps it.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(at), len) }
    }

    /// A stub address that must be within the signal stack, `len` bytes of it: its offset in the shared memory.
    pub fn on_stack(&self, address: u64, len: u64) -> Result<usize, Gone> {
        let start = self.stub_at + handler::SIGNAL_STACK;
        let end = start + handler::SIGNAL_STACK_SIZE;
        if address < start || address.checked_add(len).is_none_or(|e| e > end) {
            return Err(Gone::Garbled(format!("a frame pointer off the signal stack: {address:#x}")));
        }
        Ok((address - self.stub_at) as usize)
    }

    /// Part of an event's frame: on the signal stack, or for an event by the gate, in the gate's page.
    pub fn frame_part(&self, address: u64, len: u64, gate: bool) -> Result<usize, Gone> {
        if !gate {
            return self.on_stack(address, len);
        }
        let start = self.stub_at + handler::GATE_XSAVE - handler::SHARED;
        if address != start || len > handler::GATE_STACK - handler::GATE_XSAVE - 0x400 {
            return Err(Gone::Garbled(format!("the gate's FP state isn't where it goes: {address:#x}, {len} bytes")));
        }
        Ok((address - self.stub_at) as usize)
    }

    pub fn state(&self) -> u32 {
        self.word(STATE).load(Ordering::SeqCst)
    }

    /// Hands the stub a new state, and wakes it if it sleeps.
    pub fn tell(&self, state: u32) {
        let now = Instant::now();
        if let Some(seen) = self.spin.seen.take() {
            average(&self.spin.carbon_takes, now.duration_since(seen).as_nanos() as u64);
        }
        self.spin.told.set(Some(now));
        self.set(SPIN, self.spin.looks(self.spin.carbon_takes.get()));
        self.word(CARBON_CPU).store(host_cpu(), Ordering::Relaxed);
        self.word(STATE).store(state, Ordering::SeqCst);
        std::sync::atomic::fence(Ordering::SeqCst);
        if self.word(STUB_SLEEPING).load(Ordering::SeqCst) != 0 {
            futex(self.word(STATE), libc::FUTEX_WAKE, 1, None);
        }
    }

    /// Waits until the stub says `want`, spinning first, then sleeping, checking now and then that `pid` lives.
    pub fn wait(&self, want: u32, pid: i32) -> Result<(), Gone> {
        let result = self.wait_for(want, pid);
        let now = Instant::now();
        if let Some(told) = self.spin.told.take() {
            average(&self.spin.stub_takes, now.duration_since(told).as_nanos() as u64);
        }
        self.spin.seen.set(Some(now));
        result
    }

    fn wait_for(&self, want: u32, pid: i32) -> Result<(), Gone> {
        let looks = if self.word(STUB_CPU).load(Ordering::Relaxed) == host_cpu() { 0 } else { self.spin.looks(self.spin.stub_takes.get()) };
        for _ in 0..looks {
            if self.state() == want {
                return Ok(());
            }
            std::hint::spin_loop();
        }
        loop {
            self.word(CARBON_SLEEPING).store(1, Ordering::SeqCst);
            std::sync::atomic::fence(Ordering::SeqCst);
            let now = self.state();
            if now != want {
                futex(self.word(STATE), libc::FUTEX_WAIT, now, Some(WAKE_TO_CHECK));
            }
            self.word(CARBON_SLEEPING).store(0, Ordering::SeqCst);
            let now = self.state();
            if now == want {
                return Ok(());
            }
            if ![handler::RUNNING, handler::EVENT, handler::DO_OPS, handler::DONE, handler::RESUME].contains(&now) {
                return Err(Gone::Garbled(format!("the mailbox's state is {now}")));
            }
            let mut status = 0;
            // SAFETY: the stub, a child of this process; WNOHANG, so it only collects one that's ended.
            if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == pid {
                return Err(Gone::Ended(status));
            }
        }
    }

    /// Has the stub run `calls` in itself, `MAX_OPS` at a time, and gives each one's result. The stub must be waiting
    /// in its handler.
    pub fn run(&self, calls: &[(i64, [u64; 6])], pid: i32) -> Result<Vec<i64>, Gone> {
        let mut results = Vec::with_capacity(calls.len());
        for chunk in calls.chunks(MAX_OPS as usize) {
            self.queue(chunk);
            self.tell(handler::DO_OPS);
            self.wait(DONE, pid)?;
            for i in 0..chunk.len() as u64 {
                results.push(self.get(OPS + i * OP_SIZE + 56) as i64);
            }
        }
        Ok(results)
    }

    /// Puts `calls` in the mailbox, for the handler to run at the next `DO_OPS` or `RESUME`.
    pub fn queue(&self, calls: &[(i64, [u64; 6])]) {
        assert!(calls.len() as u64 <= MAX_OPS);
        for (i, &(nr, args)) in calls.iter().enumerate() {
            let at = OPS + i as u64 * OP_SIZE;
            self.set(at, nr as u64);
            for (j, arg) in args.iter().enumerate() {
                self.set(at + 8 + j as u64 * 8, *arg);
            }
            self.set(at + 56, 0);
        }
        self.set(NOPS, calls.len() as u64);
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        // SAFETY: the mapping and memfd `new` made, which nothing uses any more.
        unsafe {
            libc::munmap(self.ptr.cast(), SHARED_SIZE as usize);
            libc::close(self.fd);
        }
    }
}

/// A futex call on a shared (not process-private) word, as the stub's are.
fn futex(word: &AtomicU32, op: i32, value: u32, timeout: Option<Duration>) {
    let timespec = timeout.map(|t| libc::timespec { tv_sec: t.as_secs() as libc::time_t, tv_nsec: t.subsec_nanos() as libc::c_long });
    let timeout_ptr = timespec.as_ref().map_or(std::ptr::null(), std::ptr::from_ref);
    // SAFETY: a word of shared memory, and a timeout that outlives the call.
    unsafe { libc::syscall(libc::SYS_futex, word.as_ptr(), op, value, timeout_ptr, 0usize, 0u32) };
}
