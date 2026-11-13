//! Structured exception handling as x86 has it. There are no unwind tables: each function that handles exceptions
//! links an EXCEPTION_REGISTRATION_RECORD (the next one, and its handler) onto a chain whose head is the TEB's first
//! word (`fs:[0]`), newest first, ending at -1. The dispatcher asks each handler on the chain in turn; one that takes
//! the exception unwinds with `RtlUnwind`, which calls each handler above its own frame again, as unwinding, and
//! takes it off the chain, then returns to its caller. The C runtime's `_except_handler3` and `_except_handler4`,
//! which read MSVC's scope tables, are the handlers compiled code links; they're the program's, or ucrtbase's.
//!
//! `RtlRaiseException` and `RtlUnwind` capture the registers their caller will have when they return, so carrying on
//! from that CONTEXT is the same as returning: assembly stubs build it on the stack and pass it to Rust.

use core::ffi::c_void;
use core::sync::atomic::Ordering;

use atlas_abi::{record, status, teb};

use crate::arch::Context;
use crate::exception::{CONTINUE_EXECUTION, CONTINUE_SEARCH, EXIT_UNWIND, MOST_FRAMES, Record, UNWINDING, dispatch_from, dispatching, raise_simple};
use crate::sys;

/// EXCEPTION_REGISTRATION_RECORD: a frame's link in the chain.
#[repr(C)]
pub struct Registration {
    pub next: *mut Registration,
    pub handler: usize,
}

/// The end of the chain.
const END: usize = usize::MAX;

/// What a handler says, past carrying on and searching on.
const NESTED_EXCEPTION: u32 = 2;
const COLLIDED_UNWIND: u32 = 3;
/// EXCEPTION_NESTED_CALL: a handler is being asked about an exception raised while another was dispatched.
const NESTED_CALL: u32 = 0x10;
const STATUS_INVALID_DISPOSITION: u32 = 0xc000_0026;
const STATUS_INVALID_UNWIND_TARGET: u32 = 0xc000_0029;

/// A frame's handler, as compiled code has them: cdecl, with the frame's own registration as its establisher frame.
type Handler = unsafe extern "C" fn(*mut Record, *mut Registration, *mut Context, *mut c_void) -> u32;

fn chain() -> *mut *mut Registration {
    // SAFETY: the TEB's first word is the chain's head.
    unsafe { crate::teb().add(teb::EXCEPTION_LIST as usize).cast() }
}

fn head() -> *mut Registration {
    // SAFETY: as above.
    unsafe { chain().read_volatile() }
}

fn set_head(frame: *mut Registration) {
    // SAFETY: as above.
    unsafe { chain().write_volatile(frame) }
}

/// Whether `frame` is a registration that can be on this thread's chain: aligned, and on its stack.
fn plausible(frame: *mut Registration) -> bool {
    let at = frame as usize;
    // SAFETY: the TEB's stack bounds.
    let (base, limit) = unsafe {
        let t = crate::teb();
        (t.add(teb::STACK_BASE as usize).cast::<usize>().read(), t.add(teb::DEALLOCATION_STACK as usize).cast::<usize>().read())
    };
    at & 3 == 0 && at != 0 && (limit == 0 || at >= limit) && at + size_of::<Registration>() <= base
}

/// Asks each handler on the chain about an exception, newest first. True to carry on from `context`.
pub fn dispatch_frames(record: &mut Record, context: &mut Context) -> bool {
    let mut frame = head();
    for _ in 0..MOST_FRAMES {
        if frame as usize == END || !plausible(frame) {
            break;
        }
        // SAFETY: a registration on this thread's stack, and the handler it names.
        let (handler, next) = unsafe { (core::mem::transmute::<usize, Handler>((*frame).handler), (*frame).next) };
        let mut dispatcher: *mut Registration = core::ptr::null_mut();
        // SAFETY: the program's handler, as Windows calls it.
        let disposition = unsafe { handler(record, frame, context, (&raw mut dispatcher).cast()) };
        match disposition {
            CONTINUE_EXECUTION if record.flags & record::NONCONTINUABLE != 0 => {
                raise_simple(status::NONCONTINUABLE_EXCEPTION, record::NONCONTINUABLE);
            }
            CONTINUE_EXECUTION => return true,
            CONTINUE_SEARCH => {}
            NESTED_EXCEPTION => record.flags |= NESTED_CALL,
            _ => raise_simple(STATUS_INVALID_DISPOSITION, record::NONCONTINUABLE),
        }
        frame = next;
    }
    false
}

/// Where `RtlRaiseException`'s stub goes with the record and its caller's context: dispatched, and either carried on
/// from (as a return from `RtlRaiseException`) or the process ends.
#[unsafe(no_mangle)]
extern "system" fn AtlasRaiseFrom(record: *mut Record, context: *mut Context) -> ! {
    // SAFETY: the caller's record, and the context the stub built on the stack.
    let (record, context) = unsafe { (&mut *record, &mut *context) };
    if record.address == 0 {
        record.address = context.eip as usize;
    }
    dispatch_from(record, context)
}

/// Where `RtlUnwind`'s stub goes: every frame above `target_frame` is called as unwinding and taken off the chain
/// (all of them with no target), then its caller carries on with `return_value` in eax, as `RtlUnwind` returning.
/// `target_ip` is where x64 would carry on; on x86 it's the return address, as Windows' own does.
#[unsafe(no_mangle)]
extern "system" fn AtlasUnwindFrom(target_frame: usize, _target_ip: usize, record: *mut Record, return_value: usize, context: *mut Context) -> ! {
    // SAFETY: the context the stub built on the stack.
    let context = unsafe { &mut *context };
    let mut own = Record { address: context.eip as usize, ..Record::new(status::UNWIND, 0) };
    // SAFETY: the caller's record, if it gave one.
    let record = if record.is_null() { &mut own } else { unsafe { &mut *record } };
    record.flags |= UNWINDING;
    if target_frame == 0 {
        record.flags |= EXIT_UNWIND;
    }
    let mut frame = head();
    for _ in 0..MOST_FRAMES {
        if frame as usize == END || frame as usize == target_frame {
            break;
        }
        if target_frame != 0 && frame as usize > target_frame || !plausible(frame) {
            raise_simple(STATUS_INVALID_UNWIND_TARGET, record::NONCONTINUABLE);
            break;
        }
        // SAFETY: as in `dispatch_frames`.
        let (handler, next) = unsafe { (core::mem::transmute::<usize, Handler>((*frame).handler), (*frame).next) };
        let mut dispatcher: *mut Registration = core::ptr::null_mut();
        // SAFETY: the program's handler, told it's unwinding.
        let disposition = unsafe { handler(record, frame, context, (&raw mut dispatcher).cast()) };
        if disposition != CONTINUE_SEARCH && disposition != COLLIDED_UNWIND {
            raise_simple(STATUS_INVALID_DISPOSITION, record::NONCONTINUABLE);
        }
        set_head(next);
        frame = next;
    }
    context.eax = return_value as u32;
    // A dispatch this unwinds out of is over.
    let current = dispatching().load(Ordering::Acquire);
    if !current.is_null() {
        // SAFETY: the dispatch in progress.
        dispatching().store(unsafe { (*current).previous }, Ordering::Release);
    }
    sys::continue_with(context)
}

unsafe extern "system" {
    /// Raises an exception in this thread, as if it had happened where this was called from.
    pub fn RtlRaiseException(record: *mut Record);
}

/// The registers as they are at a stub's entry, into a CONTEXT at `esp`, with eip the return address and esp where it
/// will be once the stub has returned and popped `$args` bytes of arguments.
macro_rules! capture {
    ($args:literal) => {
        concat!(
            "sub esp, 0x2cc\n",
            "mov [esp + 0xb0], eax\n",
            "mov [esp + 0xac], ecx\n",
            "mov [esp + 0xa8], edx\n",
            "mov [esp + 0xa4], ebx\n",
            "mov [esp + 0xa0], esi\n",
            "mov [esp + 0x9c], edi\n",
            "mov [esp + 0xb4], ebp\n",
            "mov eax, [esp + 0x2cc]\n",
            "mov [esp + 0xb8], eax\n",
            "lea eax, [esp + 0x2cc + 4 + ", $args, "]\n",
            "mov [esp + 0xc4], eax\n",
            "mov dword ptr [esp + 0xc0], 0x202\n",
            "mov dword ptr [esp + 0xbc], 0x23\n",
            "mov dword ptr [esp + 0xc8], 0x2b\n",
            "mov dword ptr [esp + 0x98], 0x2b\n",
            "mov dword ptr [esp + 0x94], 0x2b\n",
            "mov dword ptr [esp + 0x90], 0x53\n",
            "mov dword ptr [esp + 0x8c], 0x2b\n",
            "mov dword ptr [esp], 0x10007\n",
            "mov eax, esp\n",
            "lea ecx, [esp + 0x2cc]\n",
        )
    };
}

// Each pushes the context, then its own arguments (at ecx + 4 on), and goes to Rust, which never comes back.
core::arch::global_asm!(
    ".globl \"_RtlRaiseException@4\"",
    "\"_RtlRaiseException@4\":",
    capture!("4"),
    "push eax",
    "push dword ptr [ecx + 4]",
    "call \"_AtlasRaiseFrom@8\"",
    "ud2",
    "",
    ".globl \"_RtlUnwind@16\"",
    "\"_RtlUnwind@16\":",
    capture!("16"),
    "push eax",
    "push dword ptr [ecx + 16]",
    "push dword ptr [ecx + 12]",
    "push dword ptr [ecx + 8]",
    "push dword ptr [ecx + 4]",
    "call \"_AtlasUnwindFrom@20\"",
    "ud2",
);

// ---- `_except_handler3`: the handler MSVC's and clang's `__try` register on x86 ----

/// The registration `__try` makes: the chain's link and handler, then its scope table and the try level it's in.
/// Below it, the function keeps its stack pointer (at -8) and, while a filter runs, the EXCEPTION_POINTERS (at -4).
#[repr(C)]
struct Eh3Registration {
    next: *mut Registration,
    handler: usize,
    scope_table: *const ScopeEntry,
    try_level: i32,
}

/// One `__try`'s entry: the level it's nested in, its filter (null for a `__finally`), and its handler (the
/// `__except` block, or the `__finally` funclet).
#[repr(C)]
struct ScopeEntry {
    previous: i32,
    filter: usize,
    handler: usize,
}

/// The frame pointer filters, handlers and `__finally` funclets expect: the registration's address and 16, as
/// MSVC's frame has its own ebp just above the registration.
fn frame_ebp(frame: *mut Eh3Registration) -> usize {
    frame as usize + 0x10
}

unsafe extern "C" {
    /// Calls `f` with ebp set to `ebp`, giving its result.
    fn atlas_call_with_ebp(f: usize, ebp: usize) -> u32;
    /// Jumps to `target` with ebp set to `ebp`: an `__except` block, which sets its own stack.
    fn atlas_jump_with_ebp(target: usize, ebp: usize) -> !;
}

core::arch::global_asm!(
    ".globl _atlas_call_with_ebp",
    "_atlas_call_with_ebp:",
    "push ebp",
    "push ebx",
    "push esi",
    "push edi",
    "mov eax, [esp + 20]",
    "mov ebp, [esp + 24]",
    "call eax",
    "pop edi",
    "pop esi",
    "pop ebx",
    "pop ebp",
    "ret",
    "",
    ".globl _atlas_jump_with_ebp",
    "_atlas_jump_with_ebp:",
    "mov eax, [esp + 4]",
    "mov ebp, [esp + 8]",
    "jmp eax",
);

/// Runs the `__finally` blocks of a frame from its current try level out to `stop`, each level taken off first, so a
/// block that raises isn't run again.
unsafe fn local_unwind(frame: *mut Eh3Registration, stop: i32) {
    // SAFETY: the registration and its scope table, as the caller says.
    unsafe {
        for _ in 0..MOST_FRAMES {
            let level = (*frame).try_level;
            if level == -1 || level == stop {
                break;
            }
            let entry = &*(*frame).scope_table.add(level as usize);
            (*frame).try_level = entry.previous;
            if entry.filter == 0 {
                atlas_call_with_ebp(entry.handler, frame_ebp(frame));
            }
        }
    }
}

/// `_local_unwind2(frame, stop)`, as MSVC's runtime exports it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _local_unwind2(frame: *mut Registration, stop: i32) {
    // SAFETY: a `__try` frame of the caller's.
    unsafe { local_unwind(frame.cast(), stop) }
}

/// `_except_handler3(record, frame, context, dispatcher)`: asked about an exception, it tries each `__try` around where
/// it happened, innermost first: a filter that says to carry on, carries on; one that takes it has every frame
/// above unwound (`RtlUnwind`), then the `__finally` blocks inside its own, then its `__except` block runs. Asked
/// while unwinding, it runs the frame's `__finally` blocks.
///
/// # Safety
///
/// Called by the dispatcher with a registration `__try` made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _except_handler3(record: *mut Record, frame: *mut Registration, context: *mut Context, _dispatcher: *mut c_void) -> u32 {
    const EXECUTE_HANDLER: u32 = 1;
    const CONTINUE_EXECUTION_FILTER: u32 = u32::MAX;
    let frame: *mut Eh3Registration = frame.cast();
    // SAFETY: the dispatcher's record and context, and the frame's registration and scope table.
    unsafe {
        if (*record).flags & (UNWINDING | EXIT_UNWIND) != 0 {
            local_unwind(frame, -1);
            return CONTINUE_SEARCH;
        }
        // EXCEPTION_POINTERS, where the filter's `GetExceptionInformation()` finds them.
        let mut pointers = [record as usize, context as usize];
        (frame as *mut usize).sub(1).write(pointers.as_mut_ptr() as usize);
        let mut level = (*frame).try_level;
        for _ in 0..MOST_FRAMES {
            if level == -1 {
                break;
            }
            let entry = &*(*frame).scope_table.add(level as usize);
            if entry.filter != 0 {
                match atlas_call_with_ebp(entry.filter, frame_ebp(frame)) {
                    CONTINUE_EXECUTION_FILTER => return CONTINUE_EXECUTION,
                    EXECUTE_HANDLER => {
                        RtlUnwind(frame.cast(), core::ptr::null_mut(), record, core::ptr::null_mut());
                        local_unwind(frame, level);
                        (*frame).try_level = entry.previous;
                        atlas_jump_with_ebp(entry.handler, frame_ebp(frame));
                    }
                    _ => {}
                }
            }
            level = entry.previous;
        }
        CONTINUE_SEARCH
    }
}

unsafe extern "system" {
    fn RtlUnwind(target_frame: *mut Registration, target_ip: *mut c_void, record: *mut Record, return_value: *mut c_void);
}
