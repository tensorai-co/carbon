//! The thread pool, in the program, as Windows' ntdll has it: work, simple callbacks and `QueueUserWorkItem`; timers;
//! waits on handles, and `RegisterWaitForSingleObject` on them; pools of their own with their thread limits; cleanup
//! groups; I/O completions (`CreateThreadpoolIo`, `BindIoCompletionCallback`); timer queues (`CreateTimerQueueTimer`);
//! and what a callback's instance does as it returns (an event set, a semaphore or mutex released, a critical section
//! left, a DLL freed).
//!
//! A pool is a queue of callbacks waiting to run and the threads that run them. A worker takes the next and runs it;
//! with nothing to do, it waits on the pool's signal (by address, in atlas), and after a while idle it ends, down to
//! the pool's minimum. Submitting wakes an idle worker, or starts one if none is idle and the pool isn't at its
//! maximum. Timers are kept by one thread that sleeps until the next is due and queues it then; each wait object has
//! a thread of its own, waiting on its handle and an event that says it's been changed. Handles for I/O are
//! associated with one completion port of the process's, which one thread waits on, queuing each completion's
//! callback. Threads come from atlas
//! (`CREATE_THREAD`), and start as any thread does, DLLs told; events, waits and semaphores are kernel32's host
//! exports, called by number (`EXPORT_NUMBER`).
//!
//! One lock covers every pool and object. It's never held while a callback runs or a thread waits.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use atlas_abi::call;

use crate::sys::{call4, host};

const INFINITE: u32 = u32::MAX;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
/// How long a worker waits for work before it ends, if the pool has more than its minimum.
const IDLE_MS: u32 = 20_000;
/// The default pool's limit, as Windows has it.
const MOST_THREADS: u32 = 512;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
/// `QueueUserWorkItem`'s and `RegisterWaitForSingleObject`'s flags.
const WT_EXECUTEONLYONCE: u32 = 0x8;
const WT_EXECUTELONGFUNCTION: u32 = 0x10;

// ---- The lock, and waiting by address ----

struct Global {
    busy: AtomicBool,
    state: UnsafeCell<State>,
}

// SAFETY: `state` is only touched while `busy` is held.
unsafe impl Sync for Global {}

struct State {
    default: *mut Pool,
    /// The process's completion port for I/O objects, once there is one.
    port: u64,
    /// Every timer that's set, and the thread that keeps them.
    timers: *mut Object,
    timer_thread: bool,
}

static GLOBAL: Global = Global { busy: AtomicBool::new(false), state: UnsafeCell::new(State { default: core::ptr::null_mut(), port: 0, timers: core::ptr::null_mut(), timer_thread: false }) };
/// Changed whenever a timer is, to wake the timer thread.
static TIMER_SIGNAL: AtomicU32 = AtomicU32::new(0);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    while GLOBAL.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: held exclusively, as `busy` says.
    let result = f(unsafe { &mut *GLOBAL.state.get() });
    GLOBAL.busy.store(false, Ordering::Release);
    result
}

/// Waits while `word` is `value`, up to `timeout_ms`: false if the time ran out.
fn wait_while(word: &AtomicU32, value: u32, timeout_ms: u32) -> bool {
    call4(call::WAIT_ADDRESS, word as *const _ as u64, (&raw const value) as u64, 4, u64::from(timeout_ms)) != u64::from(WAIT_TIMEOUT)
}

/// Changes `word` and wakes one waiter on it, or all.
fn signal(word: &AtomicU32, all: bool) {
    word.fetch_add(1, Ordering::AcqRel);
    call4(call::WAKE_ADDRESS, word as *const _ as u64, u64::from(all), 0, 0);
}

/// Starts one of the pool's own threads at `start(parameter)`. Its parameter is pointer-sized, as every thread
/// routine's is (a u64 would be two stack slots on x86).
fn spawn(start: extern "system" fn(usize) -> u32, parameter: u64) -> bool {
    call4(call::CREATE_THREAD, start as *const () as usize as u64, parameter, 0, 0) != 0
}

// ---- kernel32's host exports, by number ----

host!(set_event, b"SetEvent", (h));
host!(release_semaphore, b"ReleaseSemaphore", (h, count, previous));
host!(release_mutex, b"ReleaseMutex", (h));
host!(close_handle, b"CloseHandle", (h));
host!(create_event, b"CreateEventW", (attributes, manual, initial, name));
host!(wait_for_multiple, b"WaitForMultipleObjects", (count, handles, all, timeout));
host!(create_io_completion_port, b"CreateIoCompletionPort", (file, port, key, threads));
host!(get_queued_completion_status, b"GetQueuedCompletionStatus", (port, bytes, key, overlapped, timeout));

// ---- Time, from the shared data page ----

/// A KSYSTEM_TIME in the shared data page, read as its writer expects: High1, then Low, then High2, until they agree.
fn ksystem_time(at: u64) -> u64 {
    loop {
        // SAFETY: KUSER_SHARED_DATA, mapped read-only in every process.
        let (high1, low, high2) = unsafe { (core::ptr::read_volatile((at + 4) as *const u32), core::ptr::read_volatile(at as *const u32), core::ptr::read_volatile((at + 8) as *const u32)) };
        if high1 == high2 {
            return u64::from(high1) << 32 | u64::from(low);
        }
    }
}

/// The interrupt time, in 100-nanosecond units: what timers count by.
fn now() -> u64 {
    ksystem_time(0x7ffe_0008)
}

/// The system time, as a FILETIME.
fn system_time() -> u64 {
    ksystem_time(0x7ffe_0014)
}

/// A FILETIME the program gives for when: negative is that long from now, positive a time of day (made from now).
fn deadline(ft: i64) -> u64 {
    if ft < 0 { now() + ft.unsigned_abs() } else { now() + (ft as u64).saturating_sub(system_time()) }
}

/// How long until `when`, in milliseconds, rounded up.
fn ms_until(when: u64) -> u32 {
    (when.saturating_sub(now()).div_ceil(10_000)).min(u64::from(INFINITE - 1)) as u32
}

// ---- Pools, objects and the queue ----

struct Pool {
    head: *mut Item,
    tail: *mut Item,
    workers: u32,
    idle: u32,
    max: u32,
    min: u32,
    /// Changed when there's work, to wake an idle worker.
    signal: AtomicU32,
    closing: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `CreateThreadpoolWork`: `(instance, context, work)`.
    Work,
    /// `TrySubmitThreadpoolCallback`: `(instance, context)`, once, then it's gone.
    Simple,
    /// `QueueUserWorkItem`: `context -> DWORD`, once.
    UserItem,
    /// `CreateThreadpoolTimer`: `(instance, context, timer)`.
    Timer,
    /// `CreateThreadpoolWait`: `(instance, context, wait, result)`.
    Wait,
    /// `RegisterWaitForSingleObject`: `(context, timed out)`.
    Registered,
    /// `CreateThreadpoolIo`: `(instance, context, overlapped, result, bytes, io)`.
    Io,
    /// `BindIoCompletionCallback`: `(error, bytes, overlapped)`.
    Bound,
    /// `CreateTimerQueueTimer`: `(context, TRUE)`.
    QueueTimer,
}

/// A work, timer or wait object, as the program holds it.
struct Object {
    kind: Kind,
    callback: u64,
    context: u64,
    pool: *mut Pool,
    group: *mut Group,
    /// Next in its cleanup group.
    group_next: *mut Object,
    /// Callbacks queued and running, and a signal changed when both are 0.
    pending: u32,
    running: u32,
    done: AtomicU32,
    closed: bool,
    /// Its threads (a wait's), which it can't go before.
    refs: u32,
    long: bool,
    // A timer: set or not, when it's due, how often it comes again, and the next one set.
    armed: bool,
    due: u64,
    period_ms: u32,
    timer_next: *mut Object,
    // A wait: its handle, when it times out (0: never), the event that says it's changed, and its own signal.
    handle: u64,
    timeout: u64,
    changed: u64,
    wait_signal: AtomicU32,
    /// Registered waits: once, or every time.
    once: bool,
    /// An I/O object's operations started (`StartThreadpoolIo`) and not completed.
    pending_io: u32,
}

struct Item {
    object: *mut Object,
    /// A wait's result, or an I/O's error.
    result: u32,
    /// An I/O's OVERLAPPED and how many bytes it moved.
    overlapped: u64,
    bytes: u64,
    next: *mut Item,
}

/// A cleanup group: the objects made with it, to close together.
struct Group {
    head: *mut Object,
}

/// A callback's instance, as the program is given it.
#[repr(C)]
struct Instance {
    object: *mut Object,
    event: u64,
    semaphore: u64,
    semaphore_count: u64,
    mutex: u64,
    section: u64,
    library: u64,
    disassociated: bool,
}

fn new<T>(value: T) -> *mut T {
    let at = crate::heap::allocate(size_of::<T>(), true).cast::<T>();
    if !at.is_null() {
        // SAFETY: fresh memory, big enough.
        unsafe { at.write(value) };
    }
    at
}

fn free<T>(at: *mut T) {
    crate::heap::free(at.cast());
}

fn new_pool(max: u32) -> *mut Pool {
    new(Pool { head: core::ptr::null_mut(), tail: core::ptr::null_mut(), workers: 0, idle: 0, max, min: 0, signal: AtomicU32::new(0), closing: false })
}

/// The pool a callback environment names, else the process's own.
fn pool_of(state: &mut State, environment: u64) -> *mut Pool {
    // SAFETY: TP_CALLBACK_ENVIRON's Pool, if there's one.
    let named = if environment != 0 { unsafe { crate::pe::ptr_at(environment + crate::pe::PTR) } } else { 0 };
    if named != 0 {
        return named as *mut Pool;
    }
    if state.default.is_null() {
        state.default = new_pool(MOST_THREADS);
    }
    state.default
}

fn group_of(environment: u64) -> *mut Group {
    // SAFETY: TP_CALLBACK_ENVIRON's CleanupGroup.
    (if environment != 0 { unsafe { crate::pe::ptr_at(environment + 2 * crate::pe::PTR) } } else { 0 }) as usize as *mut Group
}

/// A new object, in its pool and cleanup group.
fn make(kind: Kind, callback: u64, context: u64, environment: u64) -> *mut Object {
    if callback == 0 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }
    let object = with(|s| {
        let pool = pool_of(s, environment);
        let group = group_of(environment);
        let object = new(Object {
            kind,
            callback,
            context,
            pool,
            group,
            group_next: core::ptr::null_mut(),
            pending: 0,
            running: 0,
            done: AtomicU32::new(0),
            closed: false,
            refs: 0,
            long: false,
            armed: false,
            due: 0,
            period_ms: 0,
            timer_next: core::ptr::null_mut(),
            handle: 0,
            timeout: 0,
            changed: 0,
            wait_signal: AtomicU32::new(0),
            once: false,
            pending_io: 0,
        });
        if !object.is_null() && !group.is_null() {
            // SAFETY: the group and the new object, under the lock.
            unsafe {
                (*object).group_next = (*group).head;
                (*group).head = object;
            }
        }
        object
    });
    if object.is_null() {
        crate::RtlSetLastWin32Error(ERROR_NOT_ENOUGH_MEMORY);
    }
    object
}

/// Queues a callback of `object`'s, waking a worker or starting one. Under the lock.
///
/// # Safety
///
/// `object` is live, and the lock is held.
unsafe fn submit(object: *mut Object, result: u32) {
    // SAFETY: as the caller says.
    unsafe { submit_with(object, result, 0, 0) }
}

/// The same, with an I/O's OVERLAPPED and byte count.
///
/// # Safety
///
/// As for `submit`.
unsafe fn submit_with(object: *mut Object, result: u32, overlapped: u64, bytes: u64) {
    // SAFETY: as the caller says.
    unsafe {
        let pool = (*object).pool;
        let item = new(Item { object, result, overlapped, bytes, next: core::ptr::null_mut() });
        if item.is_null() {
            return;
        }
        if (*pool).tail.is_null() {
            (*pool).head = item;
        } else {
            (*(*pool).tail).next = item;
        }
        (*pool).tail = item;
        (*object).pending += 1;
        if (*pool).idle == 0 && (*pool).workers < (*pool).max {
            (*pool).workers += 1;
            if !spawn(AtlasPoolWorker, pool as u64) {
                (*pool).workers -= 1;
            }
        }
        signal(&(*pool).signal, false);
    }
}

/// Takes `object`'s callbacks that haven't started out of its pool's queue. Under the lock.
///
/// # Safety
///
/// As for `submit`.
unsafe fn cancel_queued(object: *mut Object) {
    // SAFETY: as the caller says.
    unsafe {
        let pool = (*object).pool;
        let mut at: *mut *mut Item = &raw mut (*pool).head;
        let mut last: *mut Item = core::ptr::null_mut();
        while !(*at).is_null() {
            let item = *at;
            if (*item).object == object {
                *at = (*item).next;
                (*object).pending -= 1;
                free(item);
            } else {
                last = item;
                at = &raw mut (*item).next;
            }
        }
        (*pool).tail = last;
        finished_if_idle(object);
    }
}

/// Tells waiters `object` has nothing queued or running, if it hasn't; and lets it go if it's closed too.
///
/// # Safety
///
/// As for `submit`.
unsafe fn finished_if_idle(object: *mut Object) {
    // SAFETY: as the caller says.
    unsafe {
        if (*object).pending == 0 && (*object).running == 0 {
            signal(&(*object).done, true);
            if (*object).closed && (*object).refs == 0 {
                free(object);
            }
        }
    }
}

/// Waits until `object` has nothing queued or running, first taking what's queued away if `cancel`.
fn wait_callbacks(object: *mut Object, cancel: bool) {
    if object.is_null() {
        return;
    }
    loop {
        // SAFETY: the program's object, which it hasn't closed, under the lock.
        let waiting = with(|_| unsafe {
            if cancel {
                cancel_queued(object);
            }
            ((*object).pending != 0 || (*object).running != 0).then(|| (*object).done.load(Ordering::Acquire))
        });
        let Some(seen) = waiting else { return };
        // SAFETY: as above: it stays until it's closed and idle.
        wait_while(unsafe { &(*object).done }, seen, INFINITE);
    }
}

/// Takes an object out of its cleanup group. Under the lock.
///
/// # Safety
///
/// As for `submit`.
unsafe fn leave_group(object: *mut Object) {
    // SAFETY: as the caller says.
    unsafe {
        let group = core::mem::take(&mut (*object).group);
        if group.is_null() {
            return;
        }
        let mut at: *mut *mut Object = &raw mut (*group).head;
        while !(*at).is_null() {
            if *at == object {
                *at = (*object).group_next;
                return;
            }
            at = &raw mut (**at).group_next;
        }
    }
}

/// Closes an object: it goes once its callbacks have run, out of its cleanup group.
fn close(object: *mut Object) {
    if object.is_null() {
        return;
    }
    let changed = with(|s| {
        // SAFETY: the program's object, under the lock.
        unsafe {
            leave_group(object);
            if matches!((*object).kind, Kind::Timer | Kind::QueueTimer) {
                unlink_timer(s, object);
            }
            (*object).closed = true;
            (*object).armed = false;
            let changed = (*object).changed;
            finished_if_idle(object);
            changed
        }
    });
    // A wait's thread hears it's closed, and ends. (1 is a wait whose event is still being made.)
    if changed > 1 {
        set_event(changed);
    }
}

// ---- Workers ----

/// A pool's worker: runs what's queued, and waits for more, until it's been idle a while (above the pool's minimum)
/// or the pool's closed.
extern "system" fn AtlasPoolWorker(pool: usize) -> u32 {
    let pool = pool as u64;
    let pool = pool as *mut Pool;
    loop {
        // SAFETY: the pool, which stays while it has workers, under the lock.
        let next = with(|_| unsafe {
            let item = (*pool).head;
            if !item.is_null() {
                (*pool).head = (*item).next;
                if (*pool).head.is_null() {
                    (*pool).tail = core::ptr::null_mut();
                }
                let object = (*item).object;
                (*object).pending -= 1;
                (*object).running += 1;
                let done = ((*item).result, (*item).overlapped, (*item).bytes);
                free(item);
                return Ok((object, done));
            }
            (*pool).idle += 1;
            Err((*pool).signal.load(Ordering::Acquire))
        });
        match next {
            Ok((object, (result, overlapped, bytes))) => run(object, result, overlapped, bytes),
            Err(seen) => {
                // SAFETY: as above.
                let woken = wait_while(unsafe { &(*pool).signal }, seen, IDLE_MS);
                let leave = with(|_| unsafe {
                    (*pool).idle -= 1;
                    let leave = (*pool).head.is_null() && ((*pool).closing || (!woken && (*pool).workers > (*pool).min));
                    if leave {
                        (*pool).workers -= 1;
                        if (*pool).closing && (*pool).workers == 0 {
                            free(pool);
                        }
                    }
                    leave
                });
                if leave {
                    return 0;
                }
            }
        }
    }
}

/// Runs one callback, and what its instance was told to do as it returns.
fn run(object: *mut Object, result: u32, overlapped: u64, bytes: u64) {
    let mut instance = Instance { object, event: 0, semaphore: 0, semaphore_count: 0, mutex: 0, section: 0, library: 0, disassociated: false };
    // SAFETY: the object, which can't go while it's running; its callback, as Windows calls each kind.
    unsafe {
        let (kind, callback, context) = ((*object).kind, (*object).callback, (*object).context);
        let me = &raw mut instance as usize;
        let (callback, context, object_at, overlapped) = (callback as usize, context as usize, object as usize, overlapped as usize);
        match kind {
            Kind::Work | Kind::Timer => {
                let f: extern "system" fn(usize, usize, usize) = core::mem::transmute(callback);
                f(me, context, object_at);
            }
            Kind::Wait => {
                let f: extern "system" fn(usize, usize, usize, u32) = core::mem::transmute(callback);
                f(me, context, object_at, result);
            }
            Kind::Simple => {
                let f: extern "system" fn(usize, usize) = core::mem::transmute(callback);
                f(me, context);
            }
            Kind::UserItem => {
                let f: extern "system" fn(usize) -> u32 = core::mem::transmute(callback);
                f(context);
            }
            Kind::Registered => {
                let f: extern "system" fn(usize, u8) = core::mem::transmute(callback);
                f(context, u8::from(result == WAIT_TIMEOUT));
            }
            Kind::Io => {
                let f: extern "system" fn(usize, usize, usize, u32, usize, usize) = core::mem::transmute(callback);
                f(me, context, overlapped, result, bytes as usize, object_at);
            }
            Kind::Bound => {
                let f: extern "system" fn(u32, u32, usize) = core::mem::transmute(callback);
                f(result, bytes as u32, overlapped);
            }
            Kind::QueueTimer => {
                let f: extern "system" fn(usize, u8) = core::mem::transmute(callback);
                f(context, 1);
            }
        }
    }
    if instance.section != 0 {
        crate::sync::RtlLeaveCriticalSection(instance.section as *mut _);
    }
    if instance.mutex != 0 {
        release_mutex(instance.mutex);
    }
    if instance.semaphore != 0 {
        release_semaphore(instance.semaphore, instance.semaphore_count, 0);
    }
    if instance.event != 0 {
        set_event(instance.event);
    }
    if instance.library != 0 {
        crate::loader::AtlasFreeLibrary(instance.library as usize);
    }
    if !instance.disassociated {
        callback_over(object);
    }
}

/// A callback that's over, as waiting for callbacks sees it. One-shot objects go now.
fn callback_over(object: *mut Object) {
    // SAFETY: a running callback's object, under the lock.
    with(|_| unsafe {
        (*object).running -= 1;
        if matches!((*object).kind, Kind::Simple | Kind::UserItem) {
            leave_group(object);
            (*object).closed = true;
        }
        finished_if_idle(object);
    });
}

// ---- Pools ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpool(_reserved: usize) -> usize {
    let result = (|| -> u64 {
        let pool = new_pool(MOST_THREADS);
        if pool.is_null() {
            crate::RtlSetLastWin32Error(ERROR_NOT_ENOUGH_MEMORY);
        }
        pool as u64
    })();
    result as usize
}

/// `CloseThreadpool`: its workers end once what's queued has run, and it goes with the last.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpool(pool: usize) {
    let pool = pool as u64;
    let pool = pool as *mut Pool;
    if pool.is_null() {
        return;
    }
    // SAFETY: the program's pool, under the lock.
    with(|_| unsafe {
        (*pool).closing = true;
        (*pool).min = 0;
        if (*pool).workers == 0 {
            free(pool);
        } else {
            signal(&(*pool).signal, true);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetThreadpoolThreadMaximum(pool: usize, most: u32) {
    let pool = pool as u64;
    let pool = pool as *mut Pool;
    // SAFETY: the program's pool, under the lock.
    with(|_| unsafe {
        (*pool).max = most.max(1);
        (*pool).min = (*pool).min.min((*pool).max);
    });
}

/// `SetThreadpoolThreadMinimum`: as many workers as that start now, and stay.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetThreadpoolThreadMinimum(pool: usize, least: u32) -> i32 {
    let pool = pool as u64;
    let pool = pool as *mut Pool;
    // SAFETY: as above.
    with(|_| unsafe {
        (*pool).min = least;
        (*pool).max = (*pool).max.max(least);
        while (*pool).workers < least {
            (*pool).workers += 1;
            if !spawn(AtlasPoolWorker, pool as u64) {
                (*pool).workers -= 1;
                return 0;
            }
        }
        1
    })
}

// ---- Work ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpoolWork(callback: usize, context: usize, environment: usize) -> usize {
    let (callback, context, environment) = (callback as u64, context as u64, environment as u64);
    let result = (|| -> u64 {
        make(Kind::Work, callback, context, environment) as u64
    })();
    result as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSubmitThreadpoolWork(work: usize) {
    let work = work as u64;
    // SAFETY: the program's work, under the lock.
    with(|_| unsafe { submit(work as *mut Object, 0) });
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWaitForThreadpoolWorkCallbacks(work: usize, cancel: i32) {
    let work = work as u64;
    wait_callbacks(work as *mut Object, cancel != 0);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolWork(work: usize) {
    let work = work as u64;
    close(work as *mut Object);
}

/// `TrySubmitThreadpoolCallback(callback, context, environment)`: once, on the pool.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasTrySubmitThreadpoolCallback(callback: usize, context: usize, environment: usize) -> i32 {
    let (callback, context, environment) = (callback as u64, context as u64, environment as u64);
    let object = make(Kind::Simple, callback, context, environment);
    if object.is_null() {
        return 0;
    }
    // SAFETY: the new object, under the lock.
    with(|_| unsafe { submit(object, 0) });
    1
}

/// `QueueUserWorkItem(function, context, flags)`: `function(context)` once, on the process's pool.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasQueueUserWorkItem(function: usize, context: usize, flags: u32) -> i32 {
    let (function, context) = (function as u64, context as u64);
    let object = make(Kind::UserItem, function, context, 0);
    if object.is_null() {
        return 0;
    }
    // SAFETY: as above.
    with(|_| unsafe {
        (*object).long = flags & WT_EXECUTELONGFUNCTION != 0;
        if (*object).long {
            more_if_none_idle((*object).pool);
        }
        submit(object, 0);
    });
    1
}

/// Starts another worker if none is idle, for a callback that may run long. Under the lock. Whether there's room.
///
/// # Safety
///
/// `pool` is live, and the lock is held.
unsafe fn more_if_none_idle(pool: *mut Pool) -> bool {
    // SAFETY: as the caller says.
    unsafe {
        if (*pool).idle > 0 {
            return true;
        }
        if (*pool).workers >= (*pool).max {
            return false;
        }
        (*pool).workers += 1;
        if spawn(AtlasPoolWorker, pool as u64) {
            true
        } else {
            (*pool).workers -= 1;
            false
        }
    }
}

// ---- Timers ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpoolTimer(callback: usize, context: usize, environment: usize) -> usize {
    let (callback, context, environment) = (callback as u64, context as u64, environment as u64);
    let result = (|| -> u64 {
        make(Kind::Timer, callback, context, environment) as u64
    })();
    result as usize
}

/// Takes a timer off the list of those set. Under the lock.
///
/// # Safety
///
/// As for `submit`.
unsafe fn unlink_timer(state: &mut State, timer: *mut Object) {
    // SAFETY: as the caller says.
    unsafe {
        let mut at: *mut *mut Object = &raw mut state.timers;
        while !(*at).is_null() {
            if *at == timer {
                *at = (*timer).timer_next;
                (*timer).timer_next = core::ptr::null_mut();
                return;
            }
            at = &raw mut (**at).timer_next;
        }
    }
}

/// `SetThreadpoolTimer(timer, &due, period, window)`: due when the FILETIME says (relative if negative), then every
/// `period` milliseconds if that isn't 0; with no due time, not set. Callbacks queued already still run.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetThreadpoolTimer(timer: usize, due: *const i64, period_ms: u32, _window: u32) {
    let timer = timer as u64;
    // SAFETY: the program's due time, if it gives one.
    let when = if due.is_null() { None } else { Some(deadline(unsafe { due.read_unaligned() })) };
    arm_timer(timer as *mut Object, when, period_ms);
}

/// Sets a timer (of the pool's or a timer queue's) due at `due` (interrupt time), then every `period_ms` if that
/// isn't 0; with none, not set.
fn arm_timer(timer: *mut Object, due: Option<u64>, period_ms: u32) {
    // SAFETY: the program's timer, under the lock.
    let start = with(|s| unsafe {
        unlink_timer(s, timer);
        (*timer).armed = due.is_some();
        let Some(due) = due else { return false };
        (*timer).due = due;
        (*timer).period_ms = period_ms;
        (*timer).timer_next = s.timers;
        s.timers = timer;
        !core::mem::replace(&mut s.timer_thread, true)
    });
    if start && !spawn(AtlasPoolTimers, 0) {
        with(|s| s.timer_thread = false);
    }
    signal(&TIMER_SIGNAL, true);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasIsThreadpoolTimerSet(timer: usize) -> i32 {
    let timer = timer as u64;
    // SAFETY: the program's timer.
    with(|_| i32::from(unsafe { (*(timer as *mut Object)).armed }))
}

/// `WaitForThreadpoolTimerCallbacks(timer, cancel)`: with `cancel`, it isn't set any more either.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasWaitForThreadpoolTimerCallbacks(timer: usize, cancel: i32) {
    let timer = timer as u64;
    wait_callbacks(timer as *mut Object, cancel != 0);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolTimer(timer: usize) {
    let timer = timer as u64;
    close(timer as *mut Object);
    signal(&TIMER_SIGNAL, true);
}

/// The timer thread: queues each timer as it comes due, and sleeps until the next one is.
extern "system" fn AtlasPoolTimers(_: usize) -> u32 {
    loop {
        let (next, seen) = with(|s| {
            let at = now();
            let mut next = u64::MAX;
            let mut timer = s.timers;
            // SAFETY: the list of timers set, under the lock.
            unsafe {
                while !timer.is_null() {
                    let following = (*timer).timer_next;
                    if (*timer).due <= at {
                        submit(timer, 0);
                        if (*timer).period_ms == 0 {
                            (*timer).armed = false;
                            unlink_timer(s, timer);
                        } else {
                            let period = u64::from((*timer).period_ms) * 10_000;
                            (*timer).due = ((*timer).due + period).max(at + 1);
                        }
                    }
                    if (*timer).armed {
                        next = next.min((*timer).due);
                    }
                    timer = following;
                }
            }
            (next, TIMER_SIGNAL.load(Ordering::Acquire))
        });
        let timeout = if next == u64::MAX { INFINITE } else { ms_until(next).max(1) };
        wait_while(&TIMER_SIGNAL, seen, timeout);
    }
}

// ---- Waits ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpoolWait(callback: usize, context: usize, environment: usize) -> usize {
    let (callback, context, environment) = (callback as u64, context as u64, environment as u64);
    let result = (|| -> u64 {
        make(Kind::Wait, callback, context, environment) as u64
    })();
    result as usize
}

/// Sets a wait (or a registered wait) on `handle`, timing out at `timeout` (0: never); with no handle, not set.
fn arm(wait: *mut Object, handle: u64, timeout: u64) {
    // SAFETY: the program's wait, under the lock.
    let (start, changed) = with(|_| unsafe {
        (*wait).armed = handle != 0;
        (*wait).handle = handle;
        (*wait).timeout = timeout;
        let start = handle != 0 && (*wait).changed == 0;
        if start {
            (*wait).changed = 1;
            (*wait).refs += 1;
        }
        (start, (*wait).changed)
    });
    if start {
        // An auto-reset event that says the wait's been changed, or closed, to its thread.
        let event = create_event(0, 0, 0, 0);
        // SAFETY: as above.
        with(|_| unsafe { (*wait).changed = event });
        if event == 0 || !spawn(AtlasPoolWaiter, wait as u64) {
            // SAFETY: as above.
            with(|_| unsafe {
                (*wait).refs -= 1;
                (*wait).changed = 0;
                (*wait).armed = false;
            });
        }
    } else if changed > 1 {
        set_event(changed);
    }
    // SAFETY: as above.
    signal(unsafe { &(*wait).wait_signal }, true);
}

/// `SetThreadpoolWait(wait, handle, &timeout)`: one callback, as the handle's signalled or the time's up (none: never;
/// relative if negative), with WAIT_OBJECT_0 or WAIT_TIMEOUT.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetThreadpoolWait(wait: usize, handle: usize, timeout: *const i64) {
    let (wait, handle) = (wait as u64, handle as u64);
    // SAFETY: the program's FILETIME, if it gives one.
    let when = if timeout.is_null() { 0 } else { deadline(unsafe { timeout.read_unaligned() }).max(1) };
    arm(wait as *mut Object, handle, when);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWaitForThreadpoolWaitCallbacks(wait: usize, cancel: i32) {
    let wait = wait as u64;
    if cancel != 0 {
        // SAFETY: the program's wait, under the lock.
        with(|_| unsafe { (*(wait as *mut Object)).armed = false });
    }
    wait_callbacks(wait as *mut Object, cancel != 0);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolWait(wait: usize) {
    let wait = wait as u64;
    close(wait as *mut Object);
}

/// A wait's thread: waits on its handle and its event, queuing its callback as the handle's signalled or the time's
/// up, and taking its changes as they come, until it's closed.
extern "system" fn AtlasPoolWaiter(wait: usize) -> u32 {
    let wait = wait as u64;
    let wait = wait as *mut Object;
    loop {
        // SAFETY: the wait, which can't go while this thread holds it, under the lock.
        let now_armed = with(|_| unsafe {
            if (*wait).closed {
                return Err(true);
            }
            if !(*wait).armed {
                return Err(false);
            }
            Ok(((*wait).handle, (*wait).timeout, (*wait).changed))
        });
        let (handle, timeout, changed) = match now_armed {
            Ok(armed) => armed,
            Err(true) => break,
            Err(false) => {
                // SAFETY: as above.
                let (signal_word, seen) = unsafe { (&(*wait).wait_signal, (*wait).wait_signal.load(Ordering::Acquire)) };
                let still = with(|_| unsafe { !(*wait).armed && !(*wait).closed });
                if still {
                    wait_while(signal_word, seen, INFINITE);
                }
                continue;
            }
        };
        // HANDLEs: pointer-sized.
        let handles = [handle as usize, changed as usize];
        let ms = if timeout == 0 { INFINITE } else { ms_until(timeout) };
        let r = wait_for_multiple(2, handles.as_ptr() as u64, 0, u64::from(ms)) as u32;
        // SAFETY: as above.
        with(|_| unsafe {
            let same = (*wait).armed && (*wait).handle == handle && (*wait).timeout == timeout && !(*wait).closed;
            if same && (r == WAIT_OBJECT_0 || r == WAIT_TIMEOUT) {
                // A registered wait that isn't once comes round again, timing out as long after this as it did
                // before; the rest wait to be set again.
                if (*wait).kind != Kind::Registered || (*wait).once {
                    (*wait).armed = false;
                } else if (*wait).timeout != 0 {
                    (*wait).timeout = now() + u64::from((*wait).period_ms) * 10_000;
                }
                submit(wait, r);
            }
            if r == u32::MAX {
                // The handle's no good: nothing to wait for.
                (*wait).armed = false;
            }
        });
    }
    // SAFETY: as above.
    let changed = with(|_| unsafe { core::mem::replace(&mut (*wait).changed, 0) });
    close_handle(changed);
    // SAFETY: as above: this thread lets go of it.
    with(|_| unsafe {
        (*wait).refs -= 1;
        finished_if_idle(wait);
    });
    0
}

/// `RegisterWaitForSingleObject(&wait, handle, callback, context, milliseconds, flags)`: `callback(context, timed
/// out)` each time the handle's signalled (or the time's up), or only the first time (WT_EXECUTEONLYONCE).
#[unsafe(no_mangle)]
pub extern "system" fn AtlasRegisterWaitForSingleObject(out: *mut usize, handle: usize, callback: usize, context: usize, ms: u32, flags: u32) -> i32 {
    let (handle, callback, context) = (handle as u64, callback as u64, context as u64);
    if out.is_null() || handle == 0 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let wait = make(Kind::Registered, callback, context, 0);
    if wait.is_null() {
        return 0;
    }
    // SAFETY: the new wait, and the program's out pointer.
    unsafe {
        (*wait).once = flags & WT_EXECUTEONLYONCE != 0;
        (*wait).period_ms = ms;
        out.write_unaligned((wait as u64) as usize);
    }
    arm(wait, handle, if ms == INFINITE { 0 } else { now() + u64::from(ms) * 10_000 });
    1
}

/// `UnregisterWaitEx(wait, event)`: no more callbacks. With INVALID_HANDLE_VALUE, it waits for those running; with an
/// event, it sets it once they've run.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasUnregisterWaitEx(wait: usize, event: usize) -> i32 {
    let (wait, event) = (wait as u64, event as u64);
    let object = wait as *mut Object;
    if object.is_null() {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the program's wait, under the lock.
    with(|_| unsafe { (*object).armed = false });
    if event != 0 {
        wait_callbacks(object, true);
    }
    close(object);
    if event != 0 && event != crate::pe::INVALID_HANDLE {
        set_event(event);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasUnregisterWait(wait: usize) -> i32 {
    let wait = wait as u64;
    AtlasUnregisterWaitEx(wait as usize, 0)
}

// ---- I/O ----

/// The process's completion port for I/O objects, made (with the thread that waits on it) the first time it's
/// wanted. 0 if it can't be.
fn io_port() -> u64 {
    let port = with(|s| s.port);
    if port != 0 {
        return port;
    }
    let made = create_io_completion_port(crate::pe::INVALID_HANDLE, 0, 0, 0);
    if made == 0 {
        return 0;
    }
    let (port, ours) = with(|s| {
        if s.port == 0 {
            s.port = made;
            (made, true)
        } else {
            (s.port, false)
        }
    });
    if ours {
        if !spawn(AtlasPoolIo, port) {
            return 0;
        }
    } else {
        close_handle(made);
    }
    port
}

/// The I/O thread: each completion on the port is its object's callback, queued on its pool.
extern "system" fn AtlasPoolIo(port: usize) -> u32 {
    let port = port as u64;
    const ERROR_ABANDONED_WAIT_0: u32 = 735;
    loop {
        let (mut bytes, mut key, mut overlapped) = (0u32, 0usize, 0usize);
        let ok = get_queued_completion_status(port, (&raw mut bytes) as usize as u64, (&raw mut key) as usize as u64, (&raw mut overlapped) as usize as u64, u64::from(INFINITE)) != 0;
        let (key, overlapped) = (key as u64, overlapped as u64);
        let error = if ok { 0 } else { crate::RtlGetLastWin32Error() };
        if !ok && overlapped == 0 {
            if error == ERROR_ABANDONED_WAIT_0 {
                return 0;
            }
            continue;
        }
        let object = key as *mut Object;
        if object.is_null() {
            continue;
        }
        // SAFETY: an I/O object or a bound callback, which is never let go, under the lock.
        with(|_| unsafe {
            (*object).pending_io = (*object).pending_io.saturating_sub(1);
            submit_with(object, error, overlapped, u64::from(bytes));
        });
    }
}

/// An object whose handle's completions come to the port, keyed by it. It's never let go: a completion may still
/// come for it after it's closed.
fn bind(kind: Kind, handle: u64, callback: u64, context: u64, environment: u64) -> *mut Object {
    let object = make(kind, callback, context, environment);
    if object.is_null() {
        return object;
    }
    // SAFETY: the new object, under the lock.
    with(|_| unsafe { (*object).refs = 1 });
    let port = io_port();
    if port == 0 || create_io_completion_port(handle, port, object as u64, 0) == 0 {
        // SAFETY: as above: nothing else has it.
        with(|_| unsafe {
            (*object).refs = 0;
            (*object).closed = true;
            finished_if_idle(object);
        });
        return core::ptr::null_mut();
    }
    object
}

/// `CreateThreadpoolIo(file, callback, context, environment)`: the file's overlapped I/O, completed, is the callback's
/// on the pool.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpoolIo(file: usize, callback: usize, context: usize, environment: usize) -> usize {
    let (file, callback, context, environment) = (file as u64, callback as u64, context as u64, environment as u64);
    let result = (|| -> u64 {
        bind(Kind::Io, file, callback, context, environment) as u64
    })();
    result as usize
}

/// `StartThreadpoolIo`: before each operation, as Windows asks.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasStartThreadpoolIo(io: usize) {
    let io = io as u64;
    // SAFETY: the program's I/O object, under the lock.
    with(|_| unsafe { (*(io as *mut Object)).pending_io += 1 });
}

/// `CancelThreadpoolIo`: an operation that failed at once, and won't complete.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCancelThreadpoolIo(io: usize) {
    let io = io as u64;
    // SAFETY: as above.
    with(|_| unsafe {
        let o = io as *mut Object;
        (*o).pending_io = (*o).pending_io.saturating_sub(1);
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasWaitForThreadpoolIoCallbacks(io: usize, cancel: i32) {
    let io = io as u64;
    wait_callbacks(io as *mut Object, cancel != 0);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolIo(io: usize) {
    let io = io as u64;
    close(io as *mut Object);
}

/// `BindIoCompletionCallback(file, function, flags)`: each of the file's completions is `function(error, bytes,
/// overlapped)` on the process's pool.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasBindIoCompletionCallback(file: usize, function: usize, _flags: u32) -> i32 {
    let (file, function) = (file as u64, function as u64);
    i32::from(!bind(Kind::Bound, file, function, 0, 0).is_null())
}

// ---- Timer queues ----

/// `CreateTimerQueue`: a group of timers, deleted together.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateTimerQueue() -> usize {
    let result = (|| -> u64 {
        new(Group { head: core::ptr::null_mut() }) as u64
    })();
    result as usize
}

/// `CreateTimerQueueTimer(&timer, queue, callback, parameter, due ms, period ms, flags)`: `callback(parameter,
/// TRUE)` on the process's pool when it's due, and every period after (but once, with WT_EXECUTEONLYONCE).
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateTimerQueueTimer(out: *mut usize, queue: usize, callback: usize, parameter: usize, due_ms: u32, period_ms: u32, flags: u32) -> i32 {
    let (queue, callback, parameter) = (queue as u64, callback as u64, parameter as u64);
    if out.is_null() {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let timer = make(Kind::QueueTimer, callback, parameter, 0);
    if timer.is_null() {
        return 0;
    }
    let queue = queue as *mut Group;
    if !queue.is_null() {
        // SAFETY: the program's queue and the new timer, under the lock.
        with(|_| unsafe {
            (*timer).group = queue;
            (*timer).group_next = (*queue).head;
            (*queue).head = timer;
        });
    }
    let period = if flags & WT_EXECUTEONLYONCE != 0 { 0 } else { period_ms };
    arm_timer(timer, Some(now() + u64::from(due_ms) * 10_000), period);
    // SAFETY: the program's out pointer.
    unsafe { out.write_unaligned((timer as u64) as usize) };
    1
}

/// `ChangeTimerQueueTimer(queue, timer, due ms, period ms)`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasChangeTimerQueueTimer(_queue: usize, timer: usize, due_ms: u32, period_ms: u32) -> i32 {
    let timer = timer as u64;
    arm_timer(timer as *mut Object, Some(now() + u64::from(due_ms) * 10_000), period_ms);
    1
}

/// Stops and closes a timer-queue timer: with INVALID_HANDLE_VALUE, once its callbacks have run; with an event, set
/// once they have.
fn delete_queue_timer(timer: *mut Object, event: u64) {
    arm_timer(timer, None, 0);
    if event != 0 {
        wait_callbacks(timer, true);
    }
    close(timer);
    if event != 0 && event != crate::pe::INVALID_HANDLE {
        set_event(event);
    }
}

/// `DeleteTimerQueueTimer(queue, timer, completion event)`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDeleteTimerQueueTimer(_queue: usize, timer: usize, event: usize) -> i32 {
    let (timer, event) = (timer as u64, event as u64);
    if timer == 0 {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    delete_queue_timer(timer as *mut Object, event);
    1
}

/// `DeleteTimerQueueEx(queue, completion event)`: each of its timers deleted, then the queue.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDeleteTimerQueueEx(queue: usize, event: usize) -> i32 {
    let (queue, event) = (queue as u64, event as u64);
    let queue = queue as *mut Group;
    if queue.is_null() {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    loop {
        // SAFETY: the program's queue, under the lock.
        let next = with(|_| unsafe { (*queue).head });
        if next.is_null() {
            break;
        }
        delete_queue_timer(next, if event == 0 { 0 } else { crate::pe::INVALID_HANDLE });
    }
    free(queue);
    if event != 0 && event != crate::pe::INVALID_HANDLE {
        set_event(event);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasDeleteTimerQueue(queue: usize) -> i32 {
    let queue = queue as u64;
    AtlasDeleteTimerQueueEx(queue as usize, 0)
}

// ---- Cleanup groups ----

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCreateThreadpoolCleanupGroup() -> usize {
    let result = (|| -> u64 {
        new(Group { head: core::ptr::null_mut() }) as u64
    })();
    result as usize
}

/// `CloseThreadpoolCleanupGroupMembers(group, cancel, context)`: every object made with it, closed once its callbacks
/// have run (those queued not run at all, with `cancel`).
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolCleanupGroupMembers(group: usize, cancel: i32, _context: usize) {
    let group = group as u64;
    let group = group as *mut Group;
    loop {
        // SAFETY: the program's group, under the lock.
        let next = with(|_| unsafe { (*group).head });
        if next.is_null() {
            return;
        }
        // SAFETY: as above.
        with(|_| unsafe { (*next).armed = false });
        wait_callbacks(next, cancel != 0);
        close(next);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCloseThreadpoolCleanupGroup(group: usize) {
    let group = group as u64;
    free(group as *mut Group);
}

// ---- What a callback's instance does as it returns ----

fn instance(at: u64) -> &'static mut Instance {
    // SAFETY: the instance the pool gave the running callback, on its worker's stack.
    unsafe { &mut *(at as *mut Instance) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetEventWhenCallbackReturns(at: usize, event: usize) {
    let (at, event) = (at as u64, event as u64);
    instance(at).event = event;
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasReleaseSemaphoreWhenCallbackReturns(at: usize, semaphore: usize, count: u32) {
    let (at, semaphore) = (at as u64, semaphore as u64);
    let i = instance(at);
    i.semaphore = semaphore;
    i.semaphore_count = u64::from(count);
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasReleaseMutexWhenCallbackReturns(at: usize, mutex: usize) {
    let (at, mutex) = (at as u64, mutex as u64);
    instance(at).mutex = mutex;
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLeaveCriticalSectionWhenCallbackReturns(at: usize, section: usize) {
    let (at, section) = (at as u64, section as u64);
    instance(at).section = section;
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFreeLibraryWhenCallbackReturns(at: usize, module: usize) {
    let (at, module) = (at as u64, module as u64);
    instance(at).library = module;
}

/// `CallbackMayRunLong`: another worker starts if none is idle, so the pool isn't held up. FALSE if it's at its most.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCallbackMayRunLong(at: usize) -> i32 {
    let at = at as u64;
    let object = instance(at).object;
    // SAFETY: the running callback's object and its pool, under the lock.
    with(|_| i32::from(unsafe { more_if_none_idle((*object).pool) }))
}

/// `DisassociateCurrentThreadFromCallback`: waiting for its object's callbacks doesn't wait for this one any more.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDisassociateCurrentThreadFromCallback(at: usize) {
    let at = at as u64;
    let i = instance(at);
    if !i.disassociated {
        i.disassociated = true;
        callback_over(i.object);
    }
}
