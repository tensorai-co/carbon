//! Synchronization in the program, as Windows does it: critical sections, SRW locks, condition variables, one-time
//! initialization and `WaitOnAddress`. Uncontended, none leaves the program; contended, they wait in atlas by address.
//! Also TLS and FLS slots.

use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use atlas_abi::{call, teb};

use crate::sys::call4;

const INFINITE: u32 = u32::MAX;
const STATUS_TIMEOUT: u64 = 0x102;
const ERROR_TIMEOUT: u32 = 1460;

/// Waits while the `size` bytes at `addr` are what's at `compare`. False if the time ran out.
fn wait_address(addr: u64, compare: u64, size: u64, timeout_ms: u32) -> bool {
    call4(call::WAIT_ADDRESS, addr, compare, size, u64::from(timeout_ms)) != STATUS_TIMEOUT
}

fn wake_address(addr: u64, all: bool) {
    call4(call::WAKE_ADDRESS, addr, u64::from(all), 0, 0);
}

fn wait_while_u32(word: &AtomicU32, value: u32, timeout_ms: u32) -> bool {
    wait_address(word as *const _ as u64, (&raw const value) as u64, 4, timeout_ms)
}

fn wait_while_u64(word: &AtomicU64, value: u64, timeout_ms: u32) -> bool {
    wait_address(word as *const _ as u64, (&raw const value) as u64, 8, timeout_ms)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWaitOnAddress(addr: u64, compare: u64, size: usize, timeout_ms: u32) -> i32 {
    if wait_address(addr, compare, size as u64, timeout_ms) {
        1
    } else {
        crate::RtlSetLastWin32Error(ERROR_TIMEOUT);
        0
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWakeByAddressSingle(addr: u64) {
    wake_address(addr, false);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWakeByAddressAll(addr: u64) {
    wake_address(addr, true);
}

// ---- Critical sections ----

/// RTL_CRITICAL_SECTION, 64-bit.
#[repr(C)]
pub struct CriticalSection {
    debug_info: usize,
    /// -1 when free, 0 or more when held.
    lock_count: AtomicI32,
    recursion_count: i32,
    owning_thread: AtomicUsize,
    /// Nonzero once a thread has had to wait: leaving wakes one.
    lock_semaphore: AtomicUsize,
    spin_count: usize,
}

fn thread_id() -> usize {
    // SAFETY: CLIENT_ID.UniqueThread, in this thread's TEB.
    unsafe { crate::teb().add(teb::CLIENT_ID as usize + 8).cast::<usize>().read_volatile() }
}

fn process_id() -> usize {
    // SAFETY: CLIENT_ID.UniqueProcess, in this thread's TEB.
    unsafe { crate::teb().add(teb::CLIENT_ID as usize).cast::<usize>().read_volatile() }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetCurrentThreadId() -> u32 {
    thread_id() as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetCurrentProcessId() -> u32 {
    process_id() as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetCurrentThread() -> u64 {
    u64::MAX - 1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetCurrentProcess() -> u64 {
    u64::MAX
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeCriticalSection(cs: *mut CriticalSection) -> u32 {
    // SAFETY: the program's section, which it gives us to set up.
    unsafe {
        cs.write(CriticalSection {
            debug_info: 0,
            lock_count: AtomicI32::new(-1),
            recursion_count: 0,
            owning_thread: AtomicUsize::new(0),
            lock_semaphore: AtomicUsize::new(0),
            spin_count: 0,
        });
    }
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasInitializeCriticalSectionAndSpinCount(cs: *mut CriticalSection, _spin: u32) -> i32 {
    RtlInitializeCriticalSection(cs);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasInitializeCriticalSectionEx(cs: *mut CriticalSection, _spin: u32, _flags: u32) -> i32 {
    RtlInitializeCriticalSection(cs);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlDeleteCriticalSection(_cs: *mut CriticalSection) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlTryEnterCriticalSection(cs: *mut CriticalSection) -> i32 {
    // SAFETY: an initialized section.
    let cs = unsafe { &mut *cs };
    let me = thread_id();
    if cs.owning_thread.load(Ordering::Acquire) == me {
        cs.recursion_count += 1;
        return 1;
    }
    if cs.lock_count.compare_exchange(-1, 0, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        cs.owning_thread.store(me, Ordering::Relaxed);
        cs.recursion_count = 1;
        return 1;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlEnterCriticalSection(cs: *mut CriticalSection) -> u32 {
    // SAFETY: an initialized section.
    let section = unsafe { &mut *cs };
    let me = thread_id();
    if section.owning_thread.load(Ordering::Acquire) == me {
        section.recursion_count += 1;
        return 0;
    }
    let mut spins = 0;
    while section.lock_count.compare_exchange_weak(-1, 0, Ordering::Acquire, Ordering::Relaxed).is_err() {
        spins += 1;
        if spins < 64 {
            core::hint::spin_loop();
            continue;
        }
        // Contended: wait in atlas while it's held. Leaving wakes a waiter.
        section.lock_semaphore.store(1, Ordering::Release);
        let held = section.lock_count.load(Ordering::Acquire);
        if held != -1 {
            let word = &section.lock_count as *const AtomicI32 as u64;
            wait_address(word, (&raw const held) as u64, 4, INFINITE);
        }
    }
    section.owning_thread.store(me, Ordering::Relaxed);
    section.recursion_count = 1;
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlLeaveCriticalSection(cs: *mut CriticalSection) -> u32 {
    // SAFETY: a section this thread holds.
    let section = unsafe { &mut *cs };
    section.recursion_count -= 1;
    if section.recursion_count == 0 {
        section.owning_thread.store(0, Ordering::Relaxed);
        section.lock_count.store(-1, Ordering::Release);
        if section.lock_semaphore.load(Ordering::Acquire) != 0 {
            wake_address(&section.lock_count as *const AtomicI32 as u64, false);
        }
    }
    0
}

// ---- SRW locks: 0 free, 1 held exclusively, otherwise twice the number of shared holders ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasInitializeSRWLock(lock: &AtomicU64) {
    lock.store(0, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTryAcquireSRWLockExclusive(lock: &AtomicU64) -> u8 {
    u8::from(lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok())
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasAcquireSRWLockExclusive(lock: &AtomicU64) {
    loop {
        match lock.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return,
            Err(0) => continue,
            Err(held) => {
                wait_while_u64(lock, held, INFINITE);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTryAcquireSRWLockShared(lock: &AtomicU64) -> u8 {
    let s = lock.load(Ordering::Relaxed);
    u8::from(s & 1 == 0 && lock.compare_exchange(s, s + 2, Ordering::Acquire, Ordering::Relaxed).is_ok())
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasAcquireSRWLockShared(lock: &AtomicU64) {
    loop {
        let s = lock.load(Ordering::Relaxed);
        if s & 1 == 0 {
            if lock.compare_exchange_weak(s, s + 2, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return;
            }
        } else {
            wait_while_u64(lock, s, INFINITE);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasReleaseSRWLockExclusive(lock: &AtomicU64) {
    lock.store(0, Ordering::Release);
    wake_address(lock as *const AtomicU64 as u64, true);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasReleaseSRWLockShared(lock: &AtomicU64) {
    if lock.fetch_sub(2, Ordering::Release) == 2 {
        wake_address(lock as *const AtomicU64 as u64, true);
    }
}

// ---- Condition variables: a count of wakes, which sleepers wait to change ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasInitializeConditionVariable(cv: &AtomicU64) {
    cv.store(0, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWakeConditionVariable(cv: &AtomicU64) {
    cv.fetch_add(1, Ordering::AcqRel);
    wake_address(cv as *const AtomicU64 as u64, false);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWakeAllConditionVariable(cv: &AtomicU64) {
    cv.fetch_add(1, Ordering::AcqRel);
    wake_address(cv as *const AtomicU64 as u64, true);
}

/// Sleeps on `cv` with `release` let go of, and `reacquire` taken again after. FALSE with ERROR_TIMEOUT if the time ran
/// out.
fn sleep_on(cv: &AtomicU64, timeout_ms: u32, release: impl FnOnce(), reacquire: impl FnOnce()) -> i32 {
    let seen = cv.load(Ordering::Acquire);
    release();
    let woken = wait_while_u64(cv, seen, timeout_ms);
    reacquire();
    if woken || cv.load(Ordering::Acquire) != seen {
        1
    } else {
        crate::RtlSetLastWin32Error(ERROR_TIMEOUT);
        0
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSleepConditionVariableSRW(cv: &AtomicU64, lock: &AtomicU64, timeout_ms: u32, flags: u32) -> i32 {
    const SHARED: u32 = 1;
    if flags & SHARED != 0 {
        sleep_on(cv, timeout_ms, || AtlasReleaseSRWLockShared(lock), || AtlasAcquireSRWLockShared(lock))
    } else {
        sleep_on(cv, timeout_ms, || AtlasReleaseSRWLockExclusive(lock), || AtlasAcquireSRWLockExclusive(lock))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSleepConditionVariableCS(cv: &AtomicU64, cs: *mut CriticalSection, timeout_ms: u32) -> i32 {
    // The section may be held more than once: let go of it fully, and take it back as deeply.
    // SAFETY: a section this thread holds.
    let depth = unsafe { (*cs).recursion_count };
    let release = || {
        for _ in 0..depth {
            RtlLeaveCriticalSection(cs);
        }
    };
    let reacquire = || {
        for _ in 0..depth {
            RtlEnterCriticalSection(cs);
        }
    };
    sleep_on(cv, timeout_ms, release, reacquire)
}

// ---- One-time initialization: 0 not yet, 1 running, 2 done ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasInitOnceExecuteOnce(once: &AtomicU32, function: extern "system" fn(*const AtomicU32, u64, *mut u64) -> i32, parameter: u64, context: *mut u64) -> i32 {
    loop {
        match once.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                let ok = function(once, parameter, context);
                once.store(if ok != 0 { 2 } else { 0 }, Ordering::Release);
                wake_address(once as *const AtomicU32 as u64, true);
                return ok;
            }
            Err(2) => return 1,
            Err(_) => {
                wait_while_u32(once, 1, INFINITE);
            }
        }
    }
}

// ---- TLS and FLS slots: 64 of each, given out from a bitmap ----

static TLS_USED: AtomicU64 = AtomicU64::new(0);
static FLS_USED: AtomicU64 = AtomicU64::new(0);
const OUT_OF_INDEXES: u32 = u32::MAX;
const ERROR_INVALID_PARAMETER: u32 = 87;

fn alloc_slot(used: &AtomicU64) -> u32 {
    loop {
        let bits = used.load(Ordering::Acquire);
        if bits == u64::MAX {
            crate::RtlSetLastWin32Error(8);
            return OUT_OF_INDEXES;
        }
        let slot = (!bits).trailing_zeros();
        if used.compare_exchange(bits, bits | 1 << slot, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            return slot;
        }
    }
}

fn free_slot(used: &AtomicU64, slot: u32) -> i32 {
    if slot >= 64 || used.fetch_and(!(1 << slot), Ordering::AcqRel) & 1 << slot == 0 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    1
}

fn tls_slot(slot: u32) -> *mut u64 {
    // SAFETY: TlsSlots, 64 of them, in this thread's TEB.
    unsafe { crate::teb().add(teb::TLS_SLOTS as usize).cast::<u64>().add(slot as usize) }
}

/// This thread's FLS slots, made the first time they're wanted.
fn fls_slot(slot: u32) -> *mut u64 {
    // SAFETY: FlsData in this thread's TEB, and the block it points at.
    unsafe {
        let data = crate::teb().add(teb::FLS_DATA as usize).cast::<*mut u64>();
        if (*data).is_null() {
            *data = crate::heap::allocate(64 * 8, true).cast();
        }
        (*data).add(slot as usize)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTlsAlloc() -> u32 {
    alloc_slot(&TLS_USED)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTlsFree(slot: u32) -> i32 {
    free_slot(&TLS_USED, slot)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTlsGetValue(slot: u32) -> u64 {
    if slot >= 64 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // TlsGetValue clears the last error when it succeeds, so a zero value can be told from a failure.
    crate::RtlSetLastWin32Error(0);
    // SAFETY: a slot in this thread's TEB.
    unsafe { tls_slot(slot).read() }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasTlsSetValue(slot: u32, value: u64) -> i32 {
    if slot >= 64 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: as above.
    unsafe { tls_slot(slot).write(value) };
    1
}

/// FLS: TLS by another name, here. Its callbacks, run as fibers and threads end, aren't yet.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasFlsAlloc(_callback: u64) -> u32 {
    alloc_slot(&FLS_USED)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFlsFree(slot: u32) -> i32 {
    free_slot(&FLS_USED, slot)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFlsGetValue(slot: u32) -> u64 {
    if slot >= 64 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    crate::RtlSetLastWin32Error(0);
    // SAFETY: a slot in this thread's FLS block.
    unsafe { fls_slot(slot).read() }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFlsSetValue(slot: u32, value: u64) -> i32 {
    if slot >= 64 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: as above.
    unsafe { fls_slot(slot).write(value) };
    1
}
