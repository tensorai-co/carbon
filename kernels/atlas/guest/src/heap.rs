//! The process heap: size classes of powers of two, each with a free list, carved from chunks of memory atlas commits.
//! Every allocation has a 16-byte header with its class.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

pub const ZERO_MEMORY: u32 = 0x8;
const HEADER: usize = 16;
const CLASSES: usize = 40;
const SMALLEST: usize = 5; // 32 bytes, header included
const CHUNK: usize = 1 << 20;

struct State {
    free: [*mut u8; CLASSES],
    next: usize,
    end: usize,
}

struct Heap {
    busy: AtomicBool,
    state: UnsafeCell<State>,
}

// SAFETY: `state` is only touched while `busy` is held.
unsafe impl Sync for Heap {}

static HEAP: Heap = Heap { busy: AtomicBool::new(false), state: UnsafeCell::new(State { free: [core::ptr::null_mut(); CLASSES], next: 0, end: 0 }) };

pub fn process_heap() -> *mut c_void {
    (&raw const HEAP).cast_mut().cast()
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    while HEAP.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: held exclusively, as `busy` says.
    let result = f(unsafe { &mut *HEAP.state.get() });
    HEAP.busy.store(false, Ordering::Release);
    result
}

fn class_of(size: usize) -> Option<usize> {
    let total = size.checked_add(HEADER)?.max(1 << SMALLEST);
    let class = total.next_power_of_two().trailing_zeros() as usize;
    (class < CLASSES).then_some(class)
}

pub fn allocate(size: usize, zero: bool) -> *mut u8 {
    let Some(class) = class_of(size) else { return core::ptr::null_mut() };
    let block = with(|s| {
        let head = s.free[class];
        if !head.is_null() {
            // SAFETY: a free block's first word is the next free block.
            s.free[class] = unsafe { head.cast::<*mut u8>().read() };
            return head;
        }
        let bytes = 1usize << class;
        if s.next + bytes > s.end {
            let chunk = bytes.max(CHUNK);
            let fresh = crate::sys::allocate(chunk);
            if fresh.is_null() {
                return core::ptr::null_mut();
            }
            s.next = fresh as usize;
            s.end = s.next + chunk;
        }
        let block = s.next as *mut u8;
        s.next += bytes;
        block
    });
    if block.is_null() {
        return block;
    }
    // SAFETY: a block of 1 << class bytes, ours alone now.
    unsafe {
        block.cast::<usize>().write(class);
        let ptr = block.add(HEADER);
        if zero {
            core::ptr::write_bytes(ptr, 0, (1 << class) - HEADER);
        }
        ptr
    }
}

pub fn free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` came from `allocate`, so its header is just before it.
    let block = unsafe { ptr.sub(HEADER) };
    let class = unsafe { block.cast::<usize>().read() };
    if class >= CLASSES {
        return;
    }
    with(|s| {
        // SAFETY: the block is free now, and holds the list's next pointer.
        unsafe { block.cast::<*mut u8>().write(s.free[class]) };
        s.free[class] = block;
    });
}
