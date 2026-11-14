//! X, through the program's own libraries: its Xlib display's XCB connection (`XGetXCBConnection`), GLX's requests
//! sent on it as raw extension requests (no `libxcb-glx`), windows' sizes, and frames put in windows. Every library is
//! the copy the program has loaded, found with `dlopen` by name; none is linked.

// The libraries' functions are found by name and given the type of the field they're put in.
#![allow(clippy::missing_transmute_annotations)]

use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;

use crate::sync::Spin;

unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const RTLD_NOLOAD: c_int = 4;
const XCB: &CStr = c"libxcb.so.1";
const X11: &CStr = c"libX11.so.6";
const X11_XCB: &CStr = c"libX11-xcb.so.1";
/// `xcb_send_request`'s flag for a request whose error comes back with its reply, not as an event.
const CHECKED: c_int = 1;

/// A symbol of a library, the copy the program has loaded if it has (or else loaded now).
fn sym(library: &CStr, name: &CStr) -> *mut c_void {
    unsafe {
        let mut handle = dlopen(library.as_ptr(), RTLD_NOW | RTLD_NOLOAD);
        if handle.is_null() {
            handle = dlopen(library.as_ptr(), RTLD_NOW);
        }
        if handle.is_null() {
            return ptr::null_mut();
        }
        dlsym(handle, name.as_ptr())
    }
}

macro_rules! function {
    ($library:expr, $name:literal) => {{
        let f = sym($library, $name);
        if f.is_null() {
            return None;
        }
        unsafe { core::mem::transmute::<*mut c_void, _>(f) }
    }};
}

/// `xcb_extension_t`: XCB keeps each extension's opcode under the address of one of these.
#[repr(C)]
struct Extension {
    name: *const c_char,
    global_id: c_int,
}

static mut GLX_EXTENSION: Extension = Extension { name: c"GLX".as_ptr(), global_id: 0 };

/// `xcb_protocol_request_t`.
#[repr(C)]
struct Request {
    count: usize,
    ext: *mut Extension,
    opcode: u8,
    isvoid: u8,
}

#[repr(C)]
struct IoVec {
    base: *const c_void,
    len: usize,
}

/// The functions of the program's X libraries this library calls.
#[derive(Clone, Copy)]
pub(crate) struct X {
    connection: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pub(crate) visual_info: unsafe extern "C" fn(*mut c_void, i64, *mut VisualInfo, *mut c_int) -> *mut VisualInfo,
    pub(crate) generate_id: unsafe extern "C" fn(*mut c_void) -> u32,
    send_request: unsafe extern "C" fn(*mut c_void, c_int, *mut IoVec, *const Request) -> u32,
    wait_for_reply: unsafe extern "C" fn(*mut c_void, u32, *mut *mut c_void) -> *mut u8,
    extension_data: unsafe extern "C" fn(*mut c_void, *mut Extension) -> *const u8,
    flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    get_geometry: unsafe extern "C" fn(*mut c_void, u32) -> u32,
    geometry_reply: unsafe extern "C" fn(*mut c_void, u32, *mut *mut c_void) -> *mut u8,
    create_gc: unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const u32) -> u32,
    put_image: unsafe extern "C" fn(*mut c_void, u8, u32, u32, u16, u16, i16, i16, u8, u8, u32, *const u8) -> u32,
    max_request: unsafe extern "C" fn(*mut c_void) -> u32,
}

static LOADED: Spin<Option<Option<X>>> = Spin::new(None);

impl X {
    /// The program's X libraries, or None if it hasn't them all.
    pub(crate) fn get() -> Option<X> {
        *LOADED.lock().get_or_insert_with(X::load)
    }

    fn load() -> Option<X> {
        Some(X {
            connection: function!(X11_XCB, c"XGetXCBConnection"),
            visual_info: function!(X11, c"XGetVisualInfo"),
            generate_id: function!(XCB, c"xcb_generate_id"),
            send_request: function!(XCB, c"xcb_send_request"),
            wait_for_reply: function!(XCB, c"xcb_wait_for_reply"),
            extension_data: function!(XCB, c"xcb_get_extension_data"),
            flush: function!(XCB, c"xcb_flush"),
            get_geometry: function!(XCB, c"xcb_get_geometry"),
            geometry_reply: function!(XCB, c"xcb_get_geometry_reply"),
            create_gc: function!(XCB, c"xcb_create_gc"),
            put_image: function!(XCB, c"xcb_put_image"),
            max_request: function!(XCB, c"xcb_get_maximum_request_length"),
        })
    }

    /// An Xlib display's XCB connection.
    pub(crate) fn connection(&self, dpy: *mut c_void) -> *mut c_void {
        unsafe { (self.connection)(dpy) }
    }

    /// Whether the server has GLX.
    pub(crate) fn has_glx(&self, c: *mut c_void) -> bool {
        // `xcb_query_extension_reply_t`: its `present` byte is the ninth.
        let reply = unsafe { (self.extension_data)(c, &raw mut GLX_EXTENSION) };
        !reply.is_null() && unsafe { *reply.add(8) } != 0
    }

    /// Sends a GLX request (`minor`, then its fields after the header, a whole number of words): its reply, whole, if
    /// it has one (`reply`) and the server sent one rather than an error. A request without a reply returns None,
    /// its errors going to the program's Xlib as any request's do.
    pub(crate) fn glx(&self, c: *mut c_void, minor: u8, fields: &[u32], reply: bool) -> Option<Vec<u8>> {
        if !self.has_glx(c) {
            return None;
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(4 + fields.len() * 4);
        bytes.extend([0u8; 4]);
        for w in fields {
            bytes.extend(w.to_le_bytes());
        }
        // Two slots before the request's own, which XCB uses itself.
        let mut parts = [IoVec { base: ptr::null(), len: 0 }, IoVec { base: ptr::null(), len: 0 }, IoVec { base: bytes.as_ptr() as *const c_void, len: bytes.len() }];
        let request = Request { count: 1, ext: &raw mut GLX_EXTENSION, opcode: minor, isvoid: u8::from(!reply) };
        let seq = unsafe { (self.send_request)(c, if reply { CHECKED } else { 0 }, parts.as_mut_ptr().add(2), &request) };
        if !reply {
            return None;
        }
        let mut error = ptr::null_mut();
        let r = unsafe { (self.wait_for_reply)(c, seq, &mut error) };
        if !error.is_null() {
            unsafe { crate::free(error) };
        }
        if r.is_null() {
            return None;
        }
        // 32 bytes and as many more words as its length says.
        let words = unsafe { u32::from_le_bytes([*r.add(4), *r.add(5), *r.add(6), *r.add(7)]) } as usize;
        let out = unsafe { core::slice::from_raw_parts(r, 32 + words * 4) }.to_vec();
        unsafe { crate::free(r as *mut c_void) };
        Some(out)
    }

    pub(crate) fn flush(&self, c: *mut c_void) {
        unsafe { (self.flush)(c) };
    }

    /// A drawable's size and depth, or None if it isn't one.
    pub(crate) fn geometry(&self, c: *mut c_void, drawable: u32) -> Option<(u32, u32, u8)> {
        let r = unsafe { (self.geometry_reply)(c, (self.get_geometry)(c, drawable), ptr::null_mut()) };
        if r.is_null() {
            return None;
        }
        // response_type, depth, sequence, length, root, x, y, width, height, border_width.
        let (depth, width, height) = unsafe { (*r.add(1), u16::from_le_bytes([*r.add(16), *r.add(17)]), u16::from_le_bytes([*r.add(18), *r.add(19)])) };
        unsafe { crate::free(r as *mut c_void) };
        Some((u32::from(width), u32::from(height), depth))
    }

    /// A GC for putting frames in `drawable`.
    pub(crate) fn gc(&self, c: *mut c_void, drawable: u32) -> u32 {
        unsafe {
            let gc = (self.generate_id)(c);
            (self.create_gc)(c, gc, drawable, 0, ptr::null());
            gc
        }
    }

    /// A frame (rows top first, `width` 32-bit pixels each) put in a drawable of `depth`, in as many requests as the
    /// server's largest allows.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put(&self, c: *mut c_void, drawable: u32, gc: u32, depth: u8, width: u32, height: u32, pixels: &[u8]) {
        let row = width as usize * 4;
        let most = (unsafe { (self.max_request)(c) } as usize * 4).saturating_sub(32);
        let rows = (most / row.max(1)).clamp(1, height.max(1) as usize);
        let mut y = 0;
        while y < height as usize {
            let n = rows.min(height as usize - y);
            unsafe {
                (self.put_image)(c, 2, drawable, gc, width as u16, n as u16, 0, y as i16, 0, depth, (n * row) as u32, pixels.as_ptr().add(y * row));
            }
            y += n;
        }
        self.flush(c);
    }
}

/// Xlib's `XVisualInfo`.
#[repr(C)]
pub(crate) struct VisualInfo {
    pub(crate) visual: *mut c_void,
    pub(crate) visualid: u64,
    pub(crate) screen: c_int,
    pub(crate) depth: c_int,
    pub(crate) class: c_int,
    pub(crate) red_mask: u64,
    pub(crate) green_mask: u64,
    pub(crate) blue_mask: u64,
    pub(crate) colormap_size: c_int,
    pub(crate) bits_per_rgb: c_int,
}

/// `XGetVisualInfo`'s masks.
pub(crate) const VISUAL_ID_MASK: i64 = 1;
pub(crate) const VISUAL_SCREEN_MASK: i64 = 2;

/// A word of a reply, at a byte offset.
pub(crate) fn word(reply: &[u8], at: usize) -> u32 {
    reply.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
