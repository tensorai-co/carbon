//! Structured exception handling, as Windows does it on x64 and ARM64: atlas delivers a fault here with its CONTEXT and
//! EXCEPTION_RECORD; the dispatcher walks up the stack by each function's unwind data, asking each language handler in
//! turn; a handler that takes it unwinds to its target with `RtlUnwindEx`, running termination handlers on the way.
//! `RtlRaiseException` does the same for an exception the program raises itself. Vectored exception handlers are
//! asked first, before any frame's; vectored continue handlers hear each exception the program carries on from.
//!
//! x86 has no unwind data: its frames' handlers are a chain from the TEB, which `seh32` walks. What's here for every
//! ISA is the records, the vectored handlers, the unhandled filter and raising.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use atlas_abi::record;
#[cfg(not(target_arch = "x86"))]
use atlas_abi::status;

use crate::arch::Context;
#[cfg(not(target_arch = "x86"))]
use crate::arch::{RESULT, RtlCaptureContext};
use crate::strings::Message;
use crate::sys;
#[cfg(not(target_arch = "x86"))]
use crate::unwind::{self, CALL_BACK, EHANDLER, UHANDLER, Unwound};

/// EXCEPTION_RECORD: its pointers and its information are pointer-sized.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Record {
    pub code: u32,
    pub flags: u32,
    pub record: usize,
    pub address: usize,
    pub count: u32,
    pub information: [usize; 15],
}

const _: () = assert!(size_of::<Record>() == record::SIZE as usize);

impl Record {
    pub fn new(code: u32, flags: u32) -> Record {
        Record { code, flags, record: 0, address: 0, count: 0, information: [0; 15] }
    }
}

/// DISPATCHER_CONTEXT, as both ISAs have it (ARM64's last two fields are padding on x64).
#[cfg(not(target_arch = "x86"))]
#[repr(C)]
pub struct Dispatcher {
    pub control_pc: u64,
    pub image_base: u64,
    pub function_entry: u64,
    pub establisher_frame: u64,
    pub target_ip: u64,
    pub context: *mut Context,
    pub language_handler: u64,
    pub handler_data: u64,
    pub history_table: u64,
    pub scope_index: u32,
    pub control_pc_is_unwound: u8,
    pub pad: [u8; 3],
    pub non_volatile: u64,
}

#[repr(C)]
pub struct Pointers {
    pub record: *mut Record,
    pub context: *mut Context,
}

#[cfg(not(target_arch = "x86"))]
type Handler = extern "system" fn(*mut Record, u64, *mut Context, *mut Dispatcher) -> u32;

/// How many frames a dispatch or an unwind walks at most: each is higher up the stack than the last, so this only
/// stops a broken one going round in circles.
pub(crate) const MOST_FRAMES: usize = 1 << 20;

pub(crate) const CONTINUE_EXECUTION: u32 = 0;
pub(crate) const CONTINUE_SEARCH: u32 = 1;
pub(crate) const UNWINDING: u32 = 2;
pub(crate) const EXIT_UNWIND: u32 = 4;
#[cfg_attr(target_arch = "x86", allow(dead_code))]
pub(crate) const TARGET_UNWIND: u32 = 0x20;

/// A dispatch in progress: the context it started from, which unwinding from inside it starts from too (the frames
/// between are atlas-guest's own, with no handlers), and the one it interrupted.
pub(crate) struct Dispatch {
    /// What unwinding from inside the dispatch starts from: x64's and ARM64's, which unwind by tables.
    #[cfg_attr(target_arch = "x86", allow(dead_code))]
    pub(crate) context: *mut Context,
    pub(crate) previous: *mut Dispatch,
}

/// This thread's dispatch in progress, kept in its TEB.
pub(crate) fn dispatching() -> &'static AtomicPtr<Dispatch> {
    // SAFETY: atlas-guest's own slot in this thread's TEB, a pointer in size and alignment.
    unsafe { &*crate::teb().add(atlas_abi::teb::ATLAS_DISPATCH as usize).cast::<AtomicPtr<Dispatch>>() }
}
/// `SetUnhandledExceptionFilter`'s.
static UNHANDLED_FILTER: AtomicUsize = AtomicUsize::new(0);

/// One frame up: `c` becomes its caller. `unwound` says `c.pc` is a return address, not where it stopped. None at the
/// top of the stack, or where it can't be unwound.
#[cfg(not(target_arch = "x86"))]
fn step(c: &mut Context, handler_type: u32, unwound: bool) -> Option<(Unwound, u64, u64)> {
    let sp = c.sp();
    let found = unwind::lookup(if unwound { c.pc - CALL_BACK } else { c.pc });
    let result = match found {
        Some((base, entry)) => {
            // SAFETY: the function's own entry, and its stack.
            let u = unsafe { unwind::virtual_unwind(base, entry, c, handler_type) }.ok()?;
            (u, base, entry)
        }
        // Only the frame something stopped in can be a leaf. Its establisher frame is its own stack pointer, from
        // before its return address is popped: after, it's its caller's, and unwinding would stop a frame early.
        None if !unwound => {
            unwind::unwind_leaf(c);
            (Unwound { establisher: sp, ..Unwound::default() }, 0, 0)
        }
        None => return None,
    };
    // A frame's caller is higher up the stack.
    (c.pc != 0 && c.sp() >= sp).then_some(result)
}

/// The two lists of vectored handlers: exception handlers (0) and continue handlers (1).
const EXCEPTION_HANDLERS: usize = 0;
const CONTINUE_HANDLERS: usize = 1;
const MOST_VECTORED: usize = 64;

/// Each list's handlers and where each goes: lower first. A handler added first gets a place below every other's,
/// one added last a place above.
struct Lists {
    slots: [[(usize, i64); MOST_VECTORED]; 2],
    first: i64,
    last: i64,
}

struct Vectored {
    busy: AtomicBool,
    lists: UnsafeCell<Lists>,
}

// SAFETY: `lists` is only touched while `busy` is held.
unsafe impl Sync for Vectored {}

static VECTORED: Vectored = Vectored { busy: AtomicBool::new(false), lists: UnsafeCell::new(Lists { slots: [[(0, 0); MOST_VECTORED]; 2], first: 0, last: 0 }) };

fn with_vectored<R>(f: impl FnOnce(&mut Lists) -> R) -> R {
    while VECTORED.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: held exclusively, as `busy` says.
    let result = f(unsafe { &mut *VECTORED.lists.get() });
    VECTORED.busy.store(false, Ordering::Release);
    result
}

/// A handler's handle: its slot's address, which is the same for as long as it's there.
fn handle_of(list: usize, slot: usize) -> usize {
    (&raw const VECTORED.lists) as usize + (list * MOST_VECTORED + slot) * 16
}

fn add_vectored(list: usize, first: u32, handler: usize) -> usize {
    if handler == 0 {
        return 0;
    }
    with_vectored(|l| {
        let Some(slot) = l.slots[list].iter().position(|s| s.0 == 0) else { return 0 };
        let place = if first != 0 {
            l.first -= 1;
            l.first
        } else {
            l.last += 1;
            l.last
        };
        l.slots[list][slot] = (handler, place);
        handle_of(list, slot)
    })
}

fn remove_vectored(list: usize, handle: usize) -> u32 {
    with_vectored(|l| {
        let slot = (0..MOST_VECTORED).find(|&i| handle_of(list, i) == handle && l.slots[list][i].0 != 0);
        match slot {
            Some(i) => {
                l.slots[list][i] = (0, 0);
                1
            }
            None => 0,
        }
    })
}

/// Asks a list's handlers in turn, as `EXCEPTION_POINTERS`: true if one says to carry on (only exception handlers
/// can; every continue handler is told).
fn call_vectored(list: usize, record: &mut Record, context: &mut Context) -> bool {
    let mut handlers = [(0usize, 0i64); MOST_VECTORED];
    let n = with_vectored(|l| {
        let mut n = 0;
        for &s in l.slots[list].iter().filter(|s| s.0 != 0) {
            handlers[n] = s;
            n += 1;
        }
        n
    });
    handlers[..n].sort_unstable_by_key(|h| h.1);
    let mut pointers = Pointers { record, context };
    for &(handler, _) in &handlers[..n] {
        // SAFETY: the program's handler, as Windows calls it.
        let handler: extern "system" fn(*mut Pointers) -> i32 = unsafe { core::mem::transmute(handler) };
        if handler(&mut pointers) == -1 && list == EXCEPTION_HANDLERS {
            return true;
        }
    }
    false
}

/// `AddVectoredExceptionHandler(first, handler)`: a handle, or null.
#[unsafe(no_mangle)]
pub extern "system" fn RtlAddVectoredExceptionHandler(first: u32, handler: usize) -> usize {
    add_vectored(EXCEPTION_HANDLERS, first, handler)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlRemoveVectoredExceptionHandler(handle: usize) -> u32 {
    remove_vectored(EXCEPTION_HANDLERS, handle)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlAddVectoredContinueHandler(first: u32, handler: usize) -> usize {
    add_vectored(CONTINUE_HANDLERS, first, handler)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlRemoveVectoredContinueHandler(handle: usize) -> u32 {
    remove_vectored(CONTINUE_HANDLERS, handle)
}

/// Asks the vectored exception handlers, then each frame's handler, from where it happened up. True to carry on from
/// `context`, once the vectored continue handlers have heard.
fn dispatch(record: &mut Record, context: &mut Context) -> bool {
    #[cfg(target_arch = "x86")]
    use crate::seh32::dispatch_frames;
    let handled = call_vectored(EXCEPTION_HANDLERS, record, context) || dispatch_frames(record, context);
    if handled {
        call_vectored(CONTINUE_HANDLERS, record, context);
    }
    handled
}

/// Asks each frame's handler about an exception, from where it happened up. True to carry on from `context`.
#[cfg(not(target_arch = "x86"))]
fn dispatch_frames(record: &mut Record, context: &mut Context) -> bool {
    let mut c = *context;
    let mut unwound = false;
    for _ in 0..MOST_FRAMES {
        let pc = c.pc;
        let Some((found, base, entry)) = step(&mut c, EHANDLER, unwound) else { break };
        if found.handler != 0 {
            let mut d = Dispatcher {
                control_pc: pc,
                image_base: base,
                function_entry: entry,
                establisher_frame: found.establisher,
                target_ip: 0,
                context,
                language_handler: found.handler,
                handler_data: found.handler_data,
                history_table: 0,
                scope_index: 0,
                control_pc_is_unwound: u8::from(unwound),
                pad: [0; 3],
                non_volatile: 0,
            };
            // SAFETY: a language handler from the image's unwind data.
            let handler: Handler = unsafe { core::mem::transmute(found.handler) };
            match handler(record, found.establisher, context, &mut d) {
                CONTINUE_EXECUTION if record.flags & record::NONCONTINUABLE != 0 => {
                    raise_simple(status::NONCONTINUABLE_EXCEPTION, record::NONCONTINUABLE);
                }
                CONTINUE_EXECUTION => return true,
                _ => {}
            }
        }
        unwound = true;
    }
    false
}

/// Nothing took it: `SetUnhandledExceptionFilter`'s filter may, or else the process ends with it.
fn unhandled(record: &mut Record, context: &mut Context) -> ! {
    let filter = UNHANDLED_FILTER.load(Ordering::Acquire);
    if filter != 0 {
        let mut pointers = Pointers { record, context };
        // SAFETY: the program's filter.
        let filter: extern "system" fn(*mut Pointers) -> i32 = unsafe { core::mem::transmute(filter) };
        if filter(&mut pointers) == -1 {
            sys::continue_with(context);
        }
    }
    Message::new().text(b"atlas-guest: unhandled exception ").hex(u64::from(record.code)).text(b" at ").hex(record.address as u64).text(b"\n").print();
    sys::terminate(record.code)
}

/// Runs a dispatch with `context` as the current one.
pub(crate) fn dispatch_from(record: &mut Record, context: &mut Context) -> ! {
    let mut frame = Dispatch { context, previous: dispatching().load(Ordering::Acquire) };
    dispatching().store(&mut frame, Ordering::Release);
    let handled = dispatch(record, context);
    dispatching().store(frame.previous, Ordering::Release);
    if handled {
        sys::continue_with(context);
    }
    unhandled(record, context)
}

/// Where atlas delivers an exception: its record and the context it happened in, both on the stack.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDispatchException(record: *mut Record, context: *mut Context) -> ! {
    // SAFETY: what atlas put on the stack for this.
    let (record, context) = unsafe { (&mut *record, &mut *context) };
    dispatch_from(record, context)
}

pub(crate) fn raise_simple(code: u32, flags: u32) {
    let mut r = Record::new(code, flags);
    #[allow(unused_unsafe)]
    // SAFETY: a record of our own (x86's RtlRaiseException is an assembly stub, so declared unsafe).
    unsafe {
        RtlRaiseException(&mut r)
    };
}

#[cfg(target_arch = "x86")]
use crate::seh32::RtlRaiseException;

/// Raises an exception in this thread, as if it had happened where this was called from.
#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn RtlRaiseException(record: *mut Record) {
    let mut c = crate::arch::empty();
    // SAFETY: a context to fill in.
    unsafe { RtlCaptureContext(&mut c) };
    // `c` is this function's own: one frame up is its caller's.
    if step(&mut c, 0, false).is_none() {
        sys::terminate(status::INVALID_PARAMETER);
    }
    // SAFETY: the caller's record.
    let record = unsafe { &mut *record };
    if record.address == 0 {
        record.address = c.pc as usize;
    }
    // This returns when a handler says to carry on: the dispatcher continues at `c`, which is this call's return.
    // The compiler mustn't see that it never returns otherwise, or its caller's code after the call is gone.
    if core::hint::black_box(true) {
        dispatch_from(record, &mut c)
    }
}

/// `RaiseException`, as kernel32 has it.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasRaiseException(code: u32, flags: u32, count: u32, arguments: *const usize) {
    let mut r = Record { count: count.min(15), ..Record::new(code, flags & record::NONCONTINUABLE) };
    for i in 0..r.count as usize {
        // SAFETY: the caller's arguments, `count` of them.
        r.information[i] = unsafe { *arguments.add(i) };
    }
    #[allow(unused_unsafe)]
    // SAFETY: as in `raise_simple`.
    unsafe {
        RtlRaiseException(&mut r)
    };
}

/// Unwinds to `target_frame`, calling each frame's termination handlers on the way, then carries on at `target_ip`
/// with `return_value` as the function's result. With no target frame, unwinds the whole stack.
#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwindEx(target_frame: usize, target_ip: usize, record: *mut Record, return_value: usize, _context: *mut Context, _history: usize) -> ! {
    let (target_frame, target_ip, return_value) = (target_frame as u64, target_ip as u64, return_value as u64);
    let current = dispatching().load(Ordering::Acquire);
    let mut c = if current.is_null() {
        let mut c = crate::arch::empty();
        // SAFETY: a context to fill in.
        unsafe { RtlCaptureContext(&mut c) };
        c
    } else {
        // SAFETY: the dispatch in progress, whose context is on the stack below it.
        unsafe { *(*current).context }
    };
    let mut own = Record { address: c.pc as usize, ..Record::new(status::UNWIND, 0) };
    // SAFETY: the caller's record, if it gave one.
    let record = if record.is_null() { &mut own } else { unsafe { &mut *record } };
    record.flags |= UNWINDING;
    if target_frame == 0 {
        record.flags |= EXIT_UNWIND;
    }
    let mut unwound = false;
    for _ in 0..MOST_FRAMES {
        let mut before = c;
        let Some((found, base, entry)) = step(&mut c, UHANDLER, unwound) else { break };
        if found.establisher == target_frame {
            record.flags |= TARGET_UNWIND;
        }
        if found.handler != 0 {
            let mut d = Dispatcher {
                control_pc: before.pc,
                image_base: base,
                function_entry: entry,
                establisher_frame: found.establisher,
                target_ip,
                context: &mut before,
                language_handler: found.handler,
                handler_data: found.handler_data,
                history_table: 0,
                scope_index: 0,
                control_pc_is_unwound: u8::from(unwound),
                pad: [0; 3],
                non_volatile: 0,
            };
            // SAFETY: a language handler from the image's unwind data.
            let handler: Handler = unsafe { core::mem::transmute(found.handler) };
            handler(record, found.establisher, &mut before, &mut d);
        }
        if found.establisher == target_frame {
            // The target's own registers, at the target.
            c = before;
            break;
        }
        unwound = true;
    }
    c.pc = target_ip;
    c.x[RESULT] = return_value;
    // This dispatch is over: the stack it ran on is let go.
    if !current.is_null() {
        // SAFETY: as above.
        dispatching().store(unsafe { (*current).previous }, Ordering::Release);
    }
    sys::continue_with(&c)
}

#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwind(target_frame: usize, target_ip: usize, record: *mut Record, return_value: usize) -> ! {
    let (target_frame, target_ip, return_value) = (target_frame as u64, target_ip as u64, return_value as u64);
    RtlUnwindEx(target_frame as usize, target_ip as usize, record, return_value as usize, core::ptr::null_mut(), 0)
}

/// The handler C's `__try` uses: its data is a table of scopes, each a range of code with a filter and where its
/// `__except` is, or with a `__finally` and no target.
#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn __C_specific_handler(record: *mut Record, frame: usize, context: *mut Context, d: *mut Dispatcher) -> u32 {
    let frame = frame as u64;
    // SAFETY: what the dispatcher or unwinder gives every handler.
    let (r, d) = unsafe { (&mut *record, &mut *d) };
    let base = d.image_base;
    let table = d.handler_data;
    // SAFETY: the scope table, in the image's unwind data.
    let count = unsafe { crate::pe::u32_at(table) };
    let scope = |i: u32| -> [u32; 4] {
        let at = table + 4 + u64::from(i) * 16;
        // SAFETY: as above.
        unsafe { [crate::pe::u32_at(at), crate::pe::u32_at(at + 4), crate::pe::u32_at(at + 8), crate::pe::u32_at(at + 12)] }
    };
    let back = if d.control_pc_is_unwound != 0 && cfg!(target_arch = "aarch64") { CALL_BACK } else { 0 };
    let pc = (d.control_pc - back - base) as u32;
    if r.flags & (UNWINDING | EXIT_UNWIND) == 0 {
        let mut pointers = Pointers { record, context };
        for i in d.scope_index..count {
            let [begin, end, filter, target] = scope(i);
            if pc < begin || pc >= end || target == 0 {
                continue;
            }
            let decision = if filter == 1 {
                1
            } else {
                // SAFETY: the scope's filter, in the image.
                let filter: extern "system" fn(*mut Pointers, u64) -> i32 = unsafe { core::mem::transmute(base + u64::from(filter)) };
                filter(&mut pointers, frame)
            };
            if decision < 0 {
                return CONTINUE_EXECUTION;
            }
            if decision > 0 {
                RtlUnwind(frame as usize, (base + u64::from(target)) as usize, record, (u64::from(r.code)) as usize);
            }
        }
    } else {
        let target_rva = d.target_ip.wrapping_sub(base) as u32;
        for i in d.scope_index..count {
            let [begin, end, handler, target] = scope(i);
            if pc < begin || pc >= end {
                continue;
            }
            if r.flags & TARGET_UNWIND != 0 && target != 0 && target == target_rva {
                break;
            }
            if target == 0 {
                // A `__finally`, left by an exception: run it.
                d.scope_index = i + 1;
                // SAFETY: the scope's termination handler, in the image.
                let finally: extern "system" fn(u8, u64) = unsafe { core::mem::transmute(base + u64::from(handler)) };
                finally(1, frame);
            }
        }
    }
    CONTINUE_SEARCH
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetUnhandledExceptionFilter(filter: usize) -> usize {
    UNHANDLED_FILTER.swap(filter, Ordering::AcqRel)
}

/// `UnhandledExceptionFilter(&pointers)`, which programs call themselves from their outermost handler (as the C
/// runtime's stack-cookie check does): the program's filter if it set one, and otherwise, with no debugger to hand it
/// to, EXCEPTION_EXECUTE_HANDLER, so the caller ends the process.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasUnhandledExceptionFilter(pointers: *mut Pointers) -> i32 {
    const EXECUTE_HANDLER: i32 = 1;
    let filter = UNHANDLED_FILTER.load(Ordering::Acquire);
    if filter != 0 {
        // SAFETY: the program's filter.
        let filter: extern "system" fn(*mut Pointers) -> i32 = unsafe { core::mem::transmute(filter) };
        let verdict = filter(pointers);
        // Anything but EXCEPTION_CONTINUE_SEARCH (0).
        if verdict != 0 {
            return verdict;
        }
    }
    // SAFETY: the program's EXCEPTION_POINTERS, if any.
    if let Some(record) = unsafe { pointers.as_ref() }.and_then(|p| unsafe { p.record.as_ref() }) {
        Message::new().text(b"atlas-guest: unhandled exception ").hex(u64::from(record.code)).text(b" at ").hex(record.address as u64).text(b"\n").print();
    }
    EXECUTE_HANDLER
}

/// `DebugBreak()`: a breakpoint exception, as `int3` makes; with no debugger, the program's handlers see it.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasDebugBreak() {
    raise_simple(0x8000_0003, 0);
}

/// `RtlLookupFunctionEntry`: the `.pdata` entry for `pc`, and its image's base.
#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn RtlLookupFunctionEntry(pc: usize, base: *mut u64, _history: usize) -> usize {
    let pc = pc as u64;
    let result = (|| -> u64 {
        match unwind::lookup(pc) {
            Some((image, entry)) => {
                // SAFETY: the caller's out-parameter.
                unsafe { *base = image };
                entry
            }
            None => 0,
        }
    })();
    result as usize
}

/// `RtlVirtualUnwind`: unwinds `context` one frame, giving its language handler, if it has one of `handler_type`.
#[cfg(not(target_arch = "x86"))]
#[unsafe(no_mangle)]
pub extern "system" fn RtlVirtualUnwind(handler_type: u32, base: usize, pc: usize, entry: usize, context: *mut Context, handler_data: *mut u64, establisher: *mut u64, _pointers: usize) -> usize {
    let (base, pc, entry) = (base as u64, pc as u64, entry as u64);
    let result = (|| -> u64 {
        // SAFETY: the caller's context and out-parameters.
        unsafe {
            let c = &mut *context;
            c.pc = pc;
            match unwind::virtual_unwind(base, entry, c, handler_type) {
                Ok(u) => {
                    *establisher = u.establisher;
                    *handler_data = u.handler_data;
                    u.handler
                }
                Err(()) => 0,
            }
        }
    })();
    result as usize
}
