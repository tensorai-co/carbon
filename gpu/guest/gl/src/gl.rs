//! GL's entry points, as GLVND's dispatch reaches them for the current context: each call recorded for the host
//! (`gpu_wire::gl`), and sent when it has to be (a call that answers, `glFlush` and `glFinish`, a swap). Pixel-store
//! state, which only says how the program's own memory is laid out, is kept here.
//!
//! Here: what clears a window and reads it back (the clear values, `glClear`, the viewport and scissor, `glEnable` and
//! `glDisable`, the write masks), `glGet*`, `glGetError`, `glGetString`, `glPixelStore`, `glReadPixels`, `glFlush`
//! and `glFinish`; shaders, programs, buffers, vertex arrays, textures and draws are `objects`. GL's version is 3.3,
//! of which this is the part built so far.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr;

use gpu_wire::gl as wire;
use gpu_wire::records::{self, Reader};

use crate::current;
use crate::glx::Context;
use crate::sync::Spin;

/// The GL this library makes: what a context asked for by version may be at most, desktop GL and OpenGL ES.
pub(crate) const VERSION: (u32, u32) = (3, 3);
pub(crate) const ES_VERSION: (u32, u32) = (3, 0);
const VERSION_STRING: &[u8] = b"3.3 Carbon 0.2.0\0";
const GLSL_STRING: &[u8] = b"3.30\0";
const ES_VERSION_STRING: &[u8] = b"OpenGL ES 3.0 Carbon 0.2.0\0";
const ES_GLSL_STRING: &[u8] = b"OpenGL ES GLSL ES 3.00\0";
/// An OpenGL ES context's extensions: ES's names for what's built.
const ES_EXTENSIONS_LIST: &[&[u8]] = &[
    b"GL_EXT_texture_format_BGRA8888\0",
    b"GL_OES_depth_texture\0",
    b"GL_OES_depth24\0",
    b"GL_OES_depth32\0",
    b"GL_OES_packed_depth_stencil\0",
    b"GL_OES_rgb8_rgba8\0",
    b"GL_OES_element_index_uint\0",
    b"GL_OES_texture_npot\0",
    b"GL_OES_vertex_array_object\0",
    b"GL_EXT_blend_minmax\0",
    b"GL_OES_texture_float\0",
    b"GL_OES_texture_half_float\0",
    b"GL_EXT_color_buffer_float\0",
    b"GL_EXT_color_buffer_half_float\0",
];
const VENDOR_STRING: &[u8] = b"Carbon\0";
/// The extensions this GL has: each one's whole, as built. `GL_EXTENSIONS` is these, joined by spaces.
const EXTENSIONS_LIST: &[&[u8]] = &[
    b"GL_ARB_framebuffer_object\0",
    b"GL_ARB_ES2_compatibility\0",
    b"GL_ARB_ES3_compatibility\0",
    b"GL_EXT_framebuffer_object\0",
    b"GL_EXT_framebuffer_blit\0",
    b"GL_ARB_draw_buffers\0",
    b"GL_ARB_depth_texture\0",
    b"GL_EXT_packed_depth_stencil\0",
    b"GL_ARB_texture_float\0",
    b"GL_ARB_half_float_pixel\0",
    b"GL_ARB_vertex_buffer_object\0",
    b"GL_ARB_vertex_array_object\0",
    b"GL_ARB_texture_non_power_of_two\0",
    b"GL_ARB_multitexture\0",
    b"GL_ARB_texture_env_combine\0",
    b"GL_EXT_texture_env_combine\0",
    b"GL_ARB_texture_env_add\0",
    b"GL_EXT_bgra\0",
    b"GL_EXT_vertex_array_bgra\0",
    b"GL_EXT_blend_func_separate\0",
    b"GL_EXT_blend_minmax\0",
    b"GL_EXT_blend_subtract\0",
    b"GL_ARB_texture_mirrored_repeat\0",
    b"GL_ARB_texture_border_clamp\0",
    b"GL_EXT_texture_edge_clamp\0",
    b"GL_ARB_instanced_arrays\0",
    b"GL_ARB_draw_instanced\0",
    b"GL_ARB_explicit_attrib_location\0",
    b"GL_ARB_shading_language_100\0",
];

// GL's names this module uses.
const INVALID_ENUM: u32 = 0x0500;
const INVALID_VALUE: u32 = 0x0501;
const VENDOR: u32 = 0x1f00;
const RENDERER: u32 = 0x1f01;
const VERSION_NAME: u32 = 0x1f02;
const EXTENSIONS: u32 = 0x1f03;
const COLOR_CLEAR_VALUE: u32 = 0x0c22;
const DEPTH_CLEAR_VALUE: u32 = 0x0b73;
const UNPACK_ROW_LENGTH: u32 = 0x0cf2;
const UNPACK_SKIP_ROWS: u32 = 0x0cf3;
const UNPACK_SKIP_PIXELS: u32 = 0x0cf4;
const UNPACK_ALIGNMENT: u32 = 0x0cf5;
const PACK_ROW_LENGTH: u32 = 0x0d02;
const PACK_SKIP_ROWS: u32 = 0x0d03;
const PACK_SKIP_PIXELS: u32 = 0x0d04;
const PACK_ALIGNMENT: u32 = 0x0d05;
const RGB: u32 = 0x1907;

/// How the program lays pixels out in its own memory: `glPixelStore`'s state, packing (reads) and unpacking.
pub(crate) struct Store {
    /// Row length, skipped rows, skipped pixels, alignment: packing's, then unpacking's.
    values: [(u32, i32); 8],
}

impl Default for Store {
    fn default() -> Store {
        Store {
            values: [(PACK_ROW_LENGTH, 0), (PACK_SKIP_ROWS, 0), (PACK_SKIP_PIXELS, 0), (PACK_ALIGNMENT, 4), (UNPACK_ROW_LENGTH, 0), (UNPACK_SKIP_ROWS, 0), (UNPACK_SKIP_PIXELS, 0), (UNPACK_ALIGNMENT, 4)],
        }
    }
}

impl Store {
    fn get(&self, name: u32) -> Option<i32> {
        self.values.iter().find(|v| v.0 == name).map(|v| v.1)
    }

    pub(crate) fn unpack_row_length(&self) -> i32 {
        self.get(UNPACK_ROW_LENGTH).unwrap_or(0)
    }

    pub(crate) fn unpack_skip_rows(&self) -> i32 {
        self.get(UNPACK_SKIP_ROWS).unwrap_or(0)
    }

    pub(crate) fn unpack_skip_pixels(&self) -> i32 {
        self.get(UNPACK_SKIP_PIXELS).unwrap_or(0)
    }

    pub(crate) fn unpack_alignment(&self) -> i32 {
        self.get(UNPACK_ALIGNMENT).unwrap_or(4)
    }

    fn set(&mut self, name: u32, value: i32) -> Result<(), u32> {
        let Some(v) = self.values.iter_mut().find(|v| v.0 == name) else { return Err(INVALID_ENUM) };
        let alignment = name == PACK_ALIGNMENT || name == UNPACK_ALIGNMENT;
        if (alignment && ![1, 2, 4, 8].contains(&value)) || value < 0 {
            return Err(INVALID_VALUE);
        }
        v.1 = value;
        Ok(())
    }
}

/// The current context, if there's one: a call made without one does nothing, as GL leaves it undefined.
fn context<'a>() -> Option<&'a mut Context> {
    unsafe { current().as_mut() }
}

fn words(c: &mut Context, kind: u32, words: &[u32]) {
    let mut p = Vec::with_capacity(words.len() * 4);
    for &w in words {
        records::put_u32(&mut p, w);
    }
    c.record(kind, &p);
}

/// An error found here, kept by the context as its own.
fn fail(c: &mut Context, error: u32) {
    words(c, wire::ERROR, &[error]);
}

unsafe extern "C" fn glClearColor(r: f32, g: f32, b: f32, a: f32) {
    if let Some(c) = context() {
        words(c, wire::CLEAR_COLOR, &[r.to_bits(), g.to_bits(), b.to_bits(), a.to_bits()]);
    }
}

unsafe extern "C" fn glClearDepth(d: f64) {
    if let Some(c) = context() {
        c.record(wire::CLEAR_DEPTH, &d.to_bits().to_le_bytes());
    }
}

unsafe extern "C" fn glClearDepthf(d: f32) {
    unsafe { glClearDepth(f64::from(d)) }
}

unsafe extern "C" fn glDepthRange(near: f64, far: f64) {
    if let Some(c) = context() {
        let mut p = Vec::new();
        records::put_u64(&mut p, near.to_bits());
        records::put_u64(&mut p, far.to_bits());
        c.record(wire::DEPTH_RANGE, &p);
    }
}

unsafe extern "C" fn glDepthRangef(near: f32, far: f32) {
    unsafe { glDepthRange(f64::from(near), f64::from(far)) }
}

/// The precision of a shader's floats and integers: IEEE single precision and 32-bit integers, at every
/// precision, as the device computes them.
unsafe extern "C" fn glGetShaderPrecisionFormat(_shader: u32, precision: u32, range: *mut i32, bits: *mut i32) {
    // LOW_FLOAT..HIGH_FLOAT, then LOW_INT..HIGH_INT.
    let (lo, hi, p) = if (0x8df0..=0x8df2).contains(&precision) { (127, 127, 23) } else { (31, 30, 0) };
    unsafe {
        if !range.is_null() {
            *range = lo;
            *range.add(1) = hi;
        }
        if !bits.is_null() {
            *bits = p;
        }
    }
}

unsafe extern "C" fn glReleaseShaderCompiler() {}

/// No binary formats: every one is an invalid enum.
unsafe extern "C" fn glShaderBinary(_count: i32, _shaders: *const u32, _format: u32, _binary: *const c_void, _length: i32) {
    if let Some(c) = context() {
        fail(c, INVALID_ENUM);
    }
}

fn clear_buffer(buffer: u32, drawbuffer: i32, words: [u32; 4]) {
    if let Some(c) = context() {
        let w = [buffer, drawbuffer.max(0) as u32, words[0], words[1], words[2], words[3]];
        self::words(c, wire::CLEAR_BUFFER, &w);
    }
}

unsafe extern "C" fn glClearBufferfv(buffer: u32, drawbuffer: i32, v: *const f32) {
    if v.is_null() {
        return;
    }
    let n = if buffer == 0x1800 { 4 } else { 1 };
    let mut w = [0u32; 4];
    for (i, x) in w.iter_mut().enumerate().take(n) {
        *x = unsafe { *v.add(i) }.to_bits();
    }
    clear_buffer(buffer, drawbuffer, w);
}

unsafe extern "C" fn glClearBufferiv(buffer: u32, drawbuffer: i32, v: *const i32) {
    if v.is_null() {
        return;
    }
    // A stencil value, or an integer colour (as floats: the buffers here are normalized).
    let w = if buffer == 0x1800 { core::array::from_fn(|i| (unsafe { *v.add(i) } as f32).to_bits()) } else { [0, unsafe { *v } as u32, 0, 0] };
    clear_buffer(buffer, drawbuffer, w);
}

unsafe extern "C" fn glClearBufferuiv(buffer: u32, drawbuffer: i32, v: *const u32) {
    if !v.is_null() {
        clear_buffer(buffer, drawbuffer, core::array::from_fn(|i| (unsafe { *v.add(i) } as f32).to_bits()));
    }
}

unsafe extern "C" fn glClearBufferfi(buffer: u32, drawbuffer: i32, depth: f32, stencil: i32) {
    clear_buffer(buffer, drawbuffer, [depth.to_bits(), stencil as u32, 0, 0]);
}

// Sync objects. Work is done in order and a ring waits for it, so a fence is signalled once what came before it has
// been sent and finished: a client wait sends it all (as `glFinish`) and finds it signalled.

/// What a `GLsync` is here: any address that isn't null.
const SYNC: *const c_void = 1 as *const c_void;

unsafe extern "C" fn glFenceSync(_condition: u32, _flags: u32) -> *const c_void {
    SYNC
}

unsafe extern "C" fn glClientWaitSync(_sync: *const c_void, _flags: u32, _timeout: u64) -> u32 {
    unsafe { glFinish() };
    // ALREADY_SIGNALED.
    0x911a
}

unsafe extern "C" fn glWaitSync(_sync: *const c_void, _flags: u32, _timeout: u64) {}

unsafe extern "C" fn glDeleteSync(_sync: *const c_void) {}

unsafe extern "C" fn glIsSync(sync: *const c_void) -> u8 {
    u8::from(!sync.is_null())
}

unsafe extern "C" fn glGetSynciv(_sync: *const c_void, pname: u32, size: i32, length: *mut i32, out: *mut i32) {
    // OBJECT_TYPE (SYNC_FENCE), SYNC_STATUS (SIGNALED), SYNC_CONDITION (SYNC_GPU_COMMANDS_COMPLETE), SYNC_FLAGS.
    let v = match pname {
        0x9112 => 0x9116,
        0x9114 => 0x9119,
        0x9113 => 0x9117,
        _ => 0,
    };
    if size > 0 && !out.is_null() {
        unsafe { *out = v };
    }
    if !length.is_null() {
        unsafe { *length = 1 };
    }
}

// The stencil test's state: taken, and nothing done with it, as the framebuffers here have no stencil yet.
unsafe extern "C" fn glStencilFunc(_func: u32, _reference: i32, _mask: u32) {}
unsafe extern "C" fn glStencilFuncSeparate(_face: u32, _func: u32, _reference: i32, _mask: u32) {}
unsafe extern "C" fn glStencilOp(_fail: u32, _zfail: u32, _zpass: u32) {}
unsafe extern "C" fn glStencilOpSeparate(_face: u32, _fail: u32, _zfail: u32, _zpass: u32) {}
unsafe extern "C" fn glStencilMask(_mask: u32) {}
unsafe extern "C" fn glStencilMaskSeparate(_face: u32, _mask: u32) {}

unsafe extern "C" fn glClearStencil(s: i32) {
    if let Some(c) = context() {
        words(c, wire::CLEAR_STENCIL, &[s as u32]);
    }
}

unsafe extern "C" fn glClear(mask: u32) {
    if let Some(c) = context() {
        words(c, wire::CLEAR, &[mask]);
    }
}

unsafe extern "C" fn glViewport(x: i32, y: i32, width: i32, height: i32) {
    if let Some(c) = context() {
        words(c, wire::VIEWPORT, &[x as u32, y as u32, width as u32, height as u32]);
    }
}

unsafe extern "C" fn glScissor(x: i32, y: i32, width: i32, height: i32) {
    if let Some(c) = context() {
        words(c, wire::SCISSOR, &[x as u32, y as u32, width as u32, height as u32]);
    }
}

unsafe extern "C" fn glEnable(cap: u32) {
    if let Some(c) = context() {
        words(c, wire::ENABLE, &[cap]);
    }
}

unsafe extern "C" fn glDisable(cap: u32) {
    if let Some(c) = context() {
        words(c, wire::DISABLE, &[cap]);
    }
}

unsafe extern "C" fn glIsEnabled(cap: u32) -> u8 {
    let Some(c) = context() else { return 0 };
    let mut then = Vec::new();
    records::record(&mut then, wire::IS_ENABLED, &cap.to_le_bytes());
    c.sync(&then).map_or(0, |a| u8::from(Reader::new(&a).u32().unwrap_or(0) != 0))
}

unsafe extern "C" fn glColorMask(r: u8, g: u8, b: u8, a: u8) {
    if let Some(c) = context() {
        words(c, wire::COLOR_MASK, &[u32::from(r != 0), u32::from(g != 0), u32::from(b != 0), u32::from(a != 0)]);
    }
}

unsafe extern "C" fn glDepthMask(flag: u8) {
    if let Some(c) = context() {
        words(c, wire::DEPTH_MASK, &[u32::from(flag != 0)]);
    }
}

/// Both send everything recorded to the device and wait for it, so a single-buffered program's frame is drawn.
unsafe extern "C" fn glFlush() {
    unsafe { glFinish() }
}

unsafe extern "C" fn glFinish() {
    if let Some(c) = context() {
        let mut then = Vec::new();
        records::record(&mut then, wire::FINISH, &[]);
        c.sync(&then);
    }
}

unsafe extern "C" fn glGetError() -> u32 {
    let Some(c) = context() else { return 0 };
    let mut then = Vec::new();
    records::record(&mut then, wire::GET_ERROR, &[]);
    c.sync(&then).map_or(0, |a| Reader::new(&a).u32().unwrap_or(0))
}

/// A value of `glGet*`'s, as the host answered it.
#[derive(Clone, Copy)]
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// `glGet*`'s values for `pname`: pixel-store state from here, the rest from the host.
fn get(c: &mut Context, pname: u32) -> Vec<Value> {
    if let Some(v) = c.store.get(pname) {
        return alloc::vec![Value::Int(i64::from(v))];
    }
    // NUM_EXTENSIONS: as many as `glGetStringi` lists. An ES context's version is ES's, with no profile.
    match pname {
        0x821d => return alloc::vec![Value::Int(extensions(c).len() as i64)],
        0x821b if c.es => return alloc::vec![Value::Int(i64::from(ES_VERSION.0))],
        0x821c if c.es => return alloc::vec![Value::Int(i64::from(ES_VERSION.1))],
        0x9126 if c.es => return alloc::vec![Value::Int(0)],
        _ => {}
    }
    let mut then = Vec::new();
    records::record(&mut then, wire::GET, &pname.to_le_bytes());
    let Some(answers) = c.sync(&then) else { return Vec::new() };
    let mut r = Reader::new(&answers);
    let n = r.u32().unwrap_or(0);
    (0..n)
        .map_while(|_| {
            let (kind, bits) = (r.u32().ok()?, r.u64().ok()?);
            Some(match kind {
                wire::INT => Value::Int(bits as i64),
                wire::FLOAT => Value::Float(f64::from_bits(bits)),
                _ => Value::Bool(bits != 0),
            })
        })
        .collect()
}

/// Writes values to the program's array, converted as `glGet*` converts them.
unsafe fn put<T>(out: *mut T, values: &[Value], convert: impl Fn(Value) -> T) {
    if out.is_null() {
        return;
    }
    for (i, &v) in values.iter().enumerate() {
        unsafe { out.add(i).write(convert(v)) };
    }
}

unsafe extern "C" fn glGetIntegerv(pname: u32, out: *mut i32) {
    let Some(c) = context() else { return };
    let values = get(c, pname);
    // Colours and depths as integers: [-1, 1] spread linearly over the integers' range.
    let spread = pname == COLOR_CLEAR_VALUE || pname == DEPTH_CLEAR_VALUE;
    unsafe {
        put(out, &values, |v| match v {
            Value::Int(i) => i.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            Value::Float(f) if spread => round((f * (u32::MAX as f64) - 1.0) / 2.0) as i32,
            Value::Float(f) => round(f) as i32,
            Value::Bool(b) => i32::from(b),
        })
    }
}

unsafe extern "C" fn glGetInteger64v(pname: u32, out: *mut i64) {
    let Some(c) = context() else { return };
    let values = get(c, pname);
    unsafe {
        put(out, &values, |v| match v {
            Value::Int(i) => i,
            Value::Float(f) => round(f),
            Value::Bool(b) => i64::from(b),
        })
    }
}

unsafe extern "C" fn glGetFloatv(pname: u32, out: *mut f32) {
    let Some(c) = context() else { return };
    let values = get(c, pname);
    unsafe { put(out, &values, |v| as_f64(v) as f32) }
}

unsafe extern "C" fn glGetDoublev(pname: u32, out: *mut f64) {
    let Some(c) = context() else { return };
    let values = get(c, pname);
    unsafe { put(out, &values, as_f64) }
}

unsafe extern "C" fn glGetBooleanv(pname: u32, out: *mut u8) {
    let Some(c) = context() else { return };
    let values = get(c, pname);
    unsafe {
        put(out, &values, |v| match v {
            Value::Int(i) => u8::from(i != 0),
            Value::Float(f) => u8::from(f != 0.0),
            Value::Bool(b) => u8::from(b),
        })
    }
}

/// To the nearest integer, halves away from zero, as `glGet` rounds (`as` saturates what's too large).
fn round(f: f64) -> i64 {
    if f >= 0.0 { (f + 0.5) as i64 } else { (f - 0.5) as i64 }
}

fn as_f64(v: Value) -> f64 {
    match v {
        Value::Int(i) => i as f64,
        Value::Float(f) => f,
        Value::Bool(b) => f64::from(u8::from(b)),
    }
}

/// `GL_RENDERER`: the device's name, as Carbon's.
static RENDERER_STRING: Spin<Vec<u8>> = Spin::new(Vec::new());
/// `GL_EXTENSIONS`: [`EXTENSIONS_LIST`] (or an ES context's) as one string.
static EXTENSIONS_STRING: Spin<Vec<u8>> = Spin::new(Vec::new());
static ES_EXTENSIONS_STRING: Spin<Vec<u8>> = Spin::new(Vec::new());

/// The current context's extensions, one at a time.
fn extensions(c: &Context) -> &'static [&'static [u8]] {
    if c.es { ES_EXTENSIONS_LIST } else { EXTENSIONS_LIST }
}

unsafe extern "C" fn glGetString(name: u32) -> *const u8 {
    let Some(c) = context() else { return ptr::null() };
    match name {
        VENDOR => VENDOR_STRING.as_ptr(),
        VERSION_NAME if c.es => ES_VERSION_STRING.as_ptr(),
        VERSION_NAME => VERSION_STRING.as_ptr(),
        EXTENSIONS => {
            let (lock, list) = if c.es { (&ES_EXTENSIONS_STRING, ES_EXTENSIONS_LIST) } else { (&EXTENSIONS_STRING, EXTENSIONS_LIST) };
            let mut s = lock.lock();
            if s.is_empty() {
                for (i, e) in list.iter().enumerate() {
                    if i > 0 {
                        s.push(b' ');
                    }
                    s.extend_from_slice(&e[..e.len() - 1]);
                }
                s.push(0);
            }
            // Made once and never changed again, so the pointer stays good.
            s.as_ptr()
        }
        // SHADING_LANGUAGE_VERSION.
        0x8b8c if c.es => ES_GLSL_STRING.as_ptr(),
        0x8b8c => GLSL_STRING.as_ptr(),
        RENDERER => {
            let mut s = RENDERER_STRING.lock();
            if s.is_empty() {
                s.extend_from_slice(b"Carbon GPU (");
                s.extend(crate::device_name());
                s.extend_from_slice(b")\0");
            }
            // Made once and never changed again, so the pointer stays good.
            s.as_ptr()
        }
        _ => {
            fail(c, INVALID_ENUM);
            ptr::null()
        }
    }
}

unsafe extern "C" fn glPixelStorei(pname: u32, param: i32) {
    if let Some(c) = context()
        && let Err(e) = c.store.set(pname, param)
    {
        fail(c, e);
    }
}

unsafe extern "C" fn glPixelStoref(pname: u32, param: f32) {
    unsafe { glPixelStorei(pname, round(f64::from(param)) as i32) }
}

unsafe extern "C" fn glReadPixels(x: i32, y: i32, width: i32, height: i32, format: u32, kind: u32, out: *mut c_void) {
    let Some(c) = context() else { return };
    if out.is_null() {
        return;
    }
    let bytes = if format == RGB { 3 } else { 4 };
    let row = match c.store.get(PACK_ROW_LENGTH).unwrap_or(0) {
        0 => width.max(0),
        n => n,
    } as u64
        * bytes;
    let alignment = c.store.get(PACK_ALIGNMENT).unwrap_or(4) as u64;
    let stride = row.next_multiple_of(alignment);
    let skip = c.store.get(PACK_SKIP_ROWS).unwrap_or(0) as u64 * stride + c.store.get(PACK_SKIP_PIXELS).unwrap_or(0) as u64 * bytes;
    let mut p = Vec::new();
    for w in [x as u32, y as u32, width as u32, height as u32, format, kind] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, out as u64 + skip);
    records::put_u32(&mut p, stride as u32);
    let mut then = Vec::new();
    records::record(&mut then, wire::READ_PIXELS, &p);
    c.sync(&then);
}

/// `glGetStringi`: the extensions, one at a time.
unsafe extern "C" fn glGetStringi(name: u32, index: u32) -> *const u8 {
    let Some(c) = context() else { return ptr::null() };
    match extensions(c).get(index as usize) {
        Some(e) if name == EXTENSIONS => e.as_ptr(),
        _ => {
            fail(c, if name == EXTENSIONS { INVALID_VALUE } else { INVALID_ENUM });
            ptr::null()
        }
    }
}

/// Hints, and which buffer draws and reads go to: a default framebuffer's back buffer, the only one there is, or a
/// framebuffer object's colour attachment (or none, for draws).
unsafe extern "C" fn glHint(_target: u32, _mode: u32) {}

unsafe extern "C" fn glDrawBuffer(mode: u32) {
    unsafe { crate::objects::glDrawBuffers(1, &mode) }
}

unsafe extern "C" fn glReadBuffer(mode: u32) {
    if let Some(c) = context() {
        words(c, wire::READ_COLOR, &[mode]);
    }
}

macro_rules! entry {
    ($f:ident) => {
        (stringify!($f).as_bytes(), $f as *const c_void)
    };
}

/// GL's functions, by name.
pub(crate) fn lookup(name: &[u8]) -> Option<*const c_void> {
    let table: &[(&[u8], *const c_void)] = &[
        entry!(glClearColor),
        entry!(glClearDepth),
        entry!(glClearDepthf),
        entry!(glStencilFunc),
        entry!(glStencilFuncSeparate),
        entry!(glStencilOp),
        entry!(glStencilOpSeparate),
        entry!(glStencilMask),
        entry!(glStencilMaskSeparate),
        entry!(glClearBufferfv),
        entry!(glClearBufferiv),
        entry!(glClearBufferuiv),
        entry!(glClearBufferfi),
        entry!(glFenceSync),
        entry!(glClientWaitSync),
        entry!(glWaitSync),
        entry!(glDeleteSync),
        entry!(glIsSync),
        entry!(glGetSynciv),
        entry!(glDepthRange),
        entry!(glDepthRangef),
        entry!(glGetShaderPrecisionFormat),
        entry!(glReleaseShaderCompiler),
        entry!(glShaderBinary),
        entry!(glClearStencil),
        entry!(glClear),
        entry!(glViewport),
        entry!(glScissor),
        entry!(glEnable),
        entry!(glDisable),
        entry!(glIsEnabled),
        entry!(glColorMask),
        entry!(glDepthMask),
        entry!(glFlush),
        entry!(glFinish),
        entry!(glGetError),
        entry!(glGetIntegerv),
        entry!(glGetInteger64v),
        entry!(glGetFloatv),
        entry!(glGetDoublev),
        entry!(glGetBooleanv),
        entry!(glGetString),
        entry!(glPixelStorei),
        entry!(glPixelStoref),
        entry!(glReadPixels),
        entry!(glHint),
        entry!(glDrawBuffer),
        entry!(glReadBuffer),
        entry!(glGetStringi),
    ];
    table.iter().find(|e| e.0 == name).map(|e| e.1)
}
