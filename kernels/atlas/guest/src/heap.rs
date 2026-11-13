//! The process heap: size classes of powers of two, each with a free list, carved from chunks of memory atlas commits.
//! Every allocation has a 16-byte header: its class, then the size asked for (`HeapSize`'s answer).

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
        block.cast::<usize>().add(1).write(size);
        let ptr = block.add(HEADER);
        if zero {
            core::ptr::write_bytes(ptr, 0, (1 << class) - HEADER);
        }
        ptr
    }
}

/// How many bytes a block from `allocate` has room for.
pub fn usable(ptr: *mut u8) -> usize {
    if ptr.is_null() {
        return 0;
    }
    // SAFETY: `ptr` came from `allocate`, so its header is just before it.
    let class = unsafe { ptr.sub(HEADER).cast::<usize>().read() };
    if class >= CLASSES { 0 } else { (1 << class) - HEADER }
}

/// The size a block was asked for, or last resized to.
pub fn size(ptr: *mut u8) -> usize {
    if ptr.is_null() {
        return usize::MAX;
    }
    // SAFETY: `ptr` came from `allocate`, so its header is just before it.
    unsafe { ptr.sub(HEADER).cast::<usize>().add(1).read() }
}

/// Resizes a block where it is, if it has the room: zeroing what it grows by, with `zero`.
pub fn resize_in_place(ptr: *mut u8, new: usize, zero: bool) -> bool {
    if ptr.is_null() || new > usable(ptr) {
        return false;
    }
    let old = size(ptr);
    // SAFETY: the block has room for `new` bytes, and its header is just before it.
    unsafe {
        if zero && new > old {
            core::ptr::write_bytes(ptr.add(old), 0, new - old);
        }
        ptr.sub(HEADER).cast::<usize>().add(1).write(new);
    }
    true
}

/// `HeapReAlloc`'s work: in place if it fits, else (unless `in_place_only`) moved to a new block.
pub fn reallocate(ptr: *mut u8, new: usize, zero: bool, in_place_only: bool) -> *mut u8 {
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    if resize_in_place(ptr, new, zero) {
        return ptr;
    }
    if in_place_only {
        return core::ptr::null_mut();
    }
    let moved = allocate(new, zero);
    if !moved.is_null() {
        // SAFETY: both blocks have room for the smaller size.
        unsafe { core::ptr::copy_nonoverlapping(ptr, moved, size(ptr).min(new)) };
        free(ptr);
    }
    moved
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
