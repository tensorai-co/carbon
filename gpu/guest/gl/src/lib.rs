//! Carbon's OpenGL vendor library, `libGLX_carbon.so.0`: what the image's GLVND (`libGL`, `libGLX`, `libOpenGL`)
//! loads into a program when lambda's X server names `carbon` as a screen's vendor, or `__GLX_VENDOR_LIBRARY_NAME`
//! does. GLVND hands it GLX's calls for the screen and, once one of its contexts is current, GL's.
//!
//! GLX is spoken to the X server on the program's own connection (`x`), as any GLX library does, so the server knows
//! the contexts and drawables; GL's calls are recorded as `gpu_wire::gl`'s records and rung through the machine's
//! render node, where `gpu_api::gl` runs them on the device. A swap reads the frame back into the program and puts it
//! in the window with `PutImage`, as the Vulkan driver's X11 swapchains do.
//!
//! It's `no_std`, over the C library the program has already, bound by the dynamic loader when GLVND loads it; X's
//! libraries are the program's, found with `dlopen`, and none is linked.
#![no_std]
#![allow(non_snake_case, clippy::missing_safety_doc)]

extern crate alloc;

mod compat;
mod gl;
mod glx;
mod objects;
mod missing;
mod sync;
mod x;

use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr;
use core::sync::atomic::{AtomicI32, Ordering};

use gpu_wire::gl as wire;
use gpu_wire::records::Reader;

// The C library's, from the program.
unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int;
    pub(crate) fn free(p: *mut c_void);
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn write(fd: c_int, p: *const c_void, n: usize) -> isize;
    fn abort() -> !;
    fn pthread_key_create(key: *mut c_uint, destructor: Option<unsafe extern "C" fn(*mut c_void)>) -> c_int;
    fn pthread_getspecific(key: c_uint) -> *mut c_void;
    fn pthread_setspecific(key: c_uint, value: *const c_void) -> c_int;
}

struct Malloc;

unsafe impl core::alloc::GlobalAlloc for Malloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() <= 16 {
            return unsafe { malloc(layout.size().max(1)) as *mut u8 };
        }
        let mut p = ptr::null_mut();
        if unsafe { posix_memalign(&mut p, layout.align(), layout.size().max(1)) } != 0 {
            return ptr::null_mut();
        }
        p as *mut u8
    }

    unsafe fn dealloc(&self, p: *mut u8, _: core::alloc::Layout) {
        unsafe { free(p as *mut c_void) }
    }
}

#[global_allocator]
static ALLOCATOR: Malloc = Malloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

const O_RDWR: c_int = 2;
const O_CLOEXEC: c_int = 0o2000000;

/// The render node, opened once for the process: -1 before it's tried, -2 where there's none (or it isn't Carbon's).
static NODE: AtomicI32 = AtomicI32::new(-1);
/// The device's description, read as the node's opened.
static INFO: sync::Spin<Option<gpu_wire::Info>> = sync::Spin::new(None);

/// The render node, if the machine has a Carbon GPU.
pub(crate) fn node() -> Option<c_int> {
    match NODE.load(Ordering::Acquire) {
        -1 => {}
        -2 => return None,
        fd => return Some(fd),
    }
    let mut info = INFO.lock();
    // Another thread may have opened it while this one waited.
    match NODE.load(Ordering::Acquire) {
        -1 => {}
        -2 => return None,
        fd => return Some(fd),
    }
    let fd = unsafe { open(c"/dev/dri/renderD128".as_ptr(), O_RDWR | O_CLOEXEC) };
    let mut bytes = [0u8; gpu_wire::INFO_SIZE];
    let ok = fd >= 0 && unsafe { ioctl(fd, gpu_wire::INFO as c_ulong, bytes.as_mut_ptr()) } == 0 && gpu_wire::Info::from_bytes(&bytes).version == gpu_wire::VERSION;
    if !ok {
        if fd >= 0 {
            unsafe { close(fd) };
        }
        NODE.store(-2, Ordering::Release);
        return None;
    }
    *info = Some(gpu_wire::Info::from_bytes(&bytes));
    NODE.store(fd, Ordering::Release);
    Some(fd)
}

/// The device's name, as the host gives it (`Apple M1`, `software`).
pub(crate) fn device_name() -> Vec<u8> {
    let info = INFO.lock();
    let Some(i) = info.as_ref() else { return Vec::new() };
    let end = i.name.iter().position(|b| *b == 0).unwrap_or(i.name.len());
    i.name[..end].to_vec()
}

/// Memory from the C library's `malloc`, for what the program frees with `free` or `XFree`.
pub(crate) unsafe fn malloc_bytes(n: usize) -> *mut c_void {
    unsafe { malloc(n.max(1)) }
}

/// Says what went wrong, on the program's stderr, as Mesa's drivers do.
pub(crate) fn complain(message: &[u8]) {
    let prefix = b"carbon gl: ";
    unsafe {
        write(2, prefix.as_ptr() as *const c_void, prefix.len());
        write(2, message.as_ptr() as *const c_void, message.len());
        write(2, b"\n".as_ptr() as *const c_void, 1);
    }
}

/// Sends records to the host and waits for them to have run: the answers of those that answer, or None if the
/// host couldn't run them (said on stderr).
pub(crate) fn ring(records: &[u8]) -> Option<Vec<u8>> {
    let fd = node()?;
    // Room for every answer one ring can have (a `glGet`'s values are the most): only text saying what failed is
    // ever longer, and it's cut.
    let mut reply: Vec<u8> = alloc::vec![0; 4096];
    let words: [u64; 4] = [records.as_ptr() as u64, records.len() as u64, reply.as_mut_ptr() as u64, reply.len() as u64];
    let n = unsafe { ioctl(fd, wire::GL_SUBMIT_IOCTL as c_ulong, words.as_ptr()) };
    if n < 4 {
        complain(b"the render node's GL doorbell failed");
        return None;
    }
    reply.truncate((n as usize).min(reply.len()));
    let mut r = Reader::new(&reply);
    match r.u32() {
        Ok(wire::OK) => Some(reply[4..].to_vec()),
        _ => {
            complain(&reply[4..]);
            None
        }
    }
}

// The current context, each thread's.

static KEY: sync::Spin<Option<c_uint>> = sync::Spin::new(None);

fn key() -> c_uint {
    let mut k = KEY.lock();
    *k.get_or_insert_with(|| {
        let mut key = 0;
        unsafe { pthread_key_create(&mut key, None) };
        key
    })
}

pub(crate) fn current() -> *mut glx::Context {
    unsafe { pthread_getspecific(key()) as *mut glx::Context }
}

pub(crate) fn set_current(c: *mut glx::Context) {
    unsafe { pthread_setspecific(key(), c as *const c_void) };
}

// GLVND's way in: `libglxabi.h`'s, version 1.0.

const ABI_MAJOR: u32 = 1;

/// `__GLXapiImports`: what the vendor gives GLVND.
#[repr(C)]
pub struct Imports {
    is_screen_supported: Option<unsafe extern "C" fn(*mut c_void, c_int) -> c_int>,
    get_proc_address: Option<unsafe extern "C" fn(*const u8) -> *const c_void>,
    get_dispatch_address: Option<unsafe extern "C" fn(*const u8) -> *const c_void>,
    set_dispatch_index: Option<unsafe extern "C" fn(*const u8, c_int)>,
    notify_error: *const c_void,
    is_patch_supported: *const c_void,
    initiate_patch: *const c_void,
    release_patch: *const c_void,
    patch_thread_attach: *const c_void,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __glx_Main(version: u32, _exports: *const c_void, _vendor: *mut c_void, imports: *mut Imports) -> c_int {
    if version >> 16 != ABI_MAJOR || imports.is_null() {
        return 0;
    }
    unsafe {
        *imports = Imports {
            is_screen_supported: Some(is_screen_supported),
            get_proc_address: Some(get_proc_address),
            get_dispatch_address: Some(get_dispatch_address),
            set_dispatch_index: Some(set_dispatch_index),
            notify_error: ptr::null(),
            is_patch_supported: ptr::null(),
            initiate_patch: ptr::null(),
            release_patch: ptr::null(),
            patch_thread_attach: ptr::null(),
        };
    }
    1
}

/// A screen this library can render for: the machine has a Carbon GPU, and the X server has GLX.
unsafe extern "C" fn is_screen_supported(dpy: *mut c_void, screen: c_int) -> c_int {
    c_int::from(node().is_some() && glx::screen(dpy, screen).is_some())
}

/// GLX's and GL's functions, by name: GLVND's static GLX table, and its GL dispatch table, filled as names are asked.
unsafe extern "C" fn get_proc_address(name: *const u8) -> *const c_void {
    if name.is_null() {
        return ptr::null();
    }
    let name = unsafe { CStr::from_ptr(name as *const c_char) }.to_bytes();
    let find = |n: &[u8]| glx::lookup(n).or_else(|| gl::lookup(n)).or_else(|| objects::lookup(n)).or_else(|| compat::lookup(n)).or_else(|| missing::lookup(n));
    // An extension's name for what's core now (`glBindBufferARB`, `glBlendEquationEXT`) is the core function.
    let got = find(name).or_else(|| name.strip_suffix(b"ARB").or_else(|| name.strip_suffix(b"EXT")).and_then(find));
    got.unwrap_or_default()
}

/// GLX extension functions that pick their vendor by display: none here. Those this library has take a current
/// context, so GLVND reaches them through GL's dispatch.
unsafe extern "C" fn get_dispatch_address(_name: *const u8) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn set_dispatch_index(_name: *const u8, _index: c_int) {}
