//! The compatibility profile's calls, as GLVND's dispatch reaches them: the matrix stacks; the conventional arrays and
//! current values, which are generic attributes here (`gl_Vertex` 0, `gl_Normal` 2, `gl_Color` 3,
//! `gl_SecondaryColor` 4, `gl_FogCoord` 5, `gl_MultiTexCoord<n>` 8 + n, as NVIDIA's drivers alias them); texture
//! environments, the alpha test, fog and the shade model. The host draws fixed function as programs of its own.
//!
//! What isn't drawn yet (lighting and materials, polygon modes and offsets, clip planes, texture coordinate
//! generation, copies from the framebuffer) is said once, on the program's stderr, the first time it's asked for.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

use gpu_wire::gl as wire;
use gpu_wire::records;

use crate::current;
use crate::glx::Context;

const FLOAT: u32 = 0x1406;

fn context<'a>() -> Option<&'a mut Context> {
    unsafe { current().as_mut() }
}

fn words(kind: u32, w: &[u32]) {
    if let Some(c) = context() {
        let mut p = Vec::with_capacity(w.len() * 4);
        for &x in w {
            records::put_u32(&mut p, x);
        }
        c.record(kind, &p);
    }
}

/// Says once that something isn't drawn yet.
fn not_yet(what: &[u8], bit: u64) {
    static SAID: AtomicU64 = AtomicU64::new(0);
    if SAID.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        let mut m = Vec::from(what);
        m.extend_from_slice(b" isn't drawn yet");
        crate::complain(&m);
    }
}

// ---- Matrices ----

fn matrix(op: u32, floats: &[f32]) {
    let Some(c) = context() else { return };
    let mut p = Vec::new();
    records::put_u32(&mut p, op);
    let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    records::put_bytes(&mut p, &bytes);
    c.record(wire::MATRIX, &p);
}

unsafe fn sixteen<T: Copy + Into<f64>>(m: *const T, transpose: bool) -> [f32; 16] {
    core::array::from_fn(|i| {
        let at = if transpose { (i % 4) * 4 + i / 4 } else { i };
        if m.is_null() { 0.0 } else { unsafe { (*m.add(at)).into() as f32 } }
    })
}

unsafe extern "C" fn glMatrixMode(mode: u32) {
    words(wire::MATRIX_MODE, &[mode]);
}

unsafe extern "C" fn glLoadIdentity() {
    matrix(wire::MATRIX_IDENTITY, &[]);
}

unsafe extern "C" fn glLoadMatrixf(m: *const f32) {
    matrix(wire::MATRIX_LOAD, &unsafe { sixteen(m, false) });
}

unsafe extern "C" fn glLoadMatrixd(m: *const f64) {
    matrix(wire::MATRIX_LOAD, &unsafe { sixteen(m, false) });
}

unsafe extern "C" fn glMultMatrixf(m: *const f32) {
    matrix(wire::MATRIX_MULTIPLY, &unsafe { sixteen(m, false) });
}

unsafe extern "C" fn glMultMatrixd(m: *const f64) {
    matrix(wire::MATRIX_MULTIPLY, &unsafe { sixteen(m, false) });
}

unsafe extern "C" fn glLoadTransposeMatrixf(m: *const f32) {
    matrix(wire::MATRIX_LOAD, &unsafe { sixteen(m, true) });
}

unsafe extern "C" fn glMultTransposeMatrixf(m: *const f32) {
    matrix(wire::MATRIX_MULTIPLY, &unsafe { sixteen(m, true) });
}

unsafe extern "C" fn glPushMatrix() {
    matrix(wire::MATRIX_PUSH, &[]);
}

unsafe extern "C" fn glPopMatrix() {
    matrix(wire::MATRIX_POP, &[]);
}

unsafe extern "C" fn glTranslatef(x: f32, y: f32, z: f32) {
    matrix(wire::MATRIX_TRANSLATE, &[x, y, z]);
}

unsafe extern "C" fn glTranslated(x: f64, y: f64, z: f64) {
    matrix(wire::MATRIX_TRANSLATE, &[x as f32, y as f32, z as f32]);
}

unsafe extern "C" fn glScalef(x: f32, y: f32, z: f32) {
    matrix(wire::MATRIX_SCALE, &[x, y, z]);
}

unsafe extern "C" fn glScaled(x: f64, y: f64, z: f64) {
    matrix(wire::MATRIX_SCALE, &[x as f32, y as f32, z as f32]);
}

unsafe extern "C" fn glRotatef(angle: f32, x: f32, y: f32, z: f32) {
    matrix(wire::MATRIX_ROTATE, &[angle, x, y, z]);
}

unsafe extern "C" fn glRotated(angle: f64, x: f64, y: f64, z: f64) {
    matrix(wire::MATRIX_ROTATE, &[angle as f32, x as f32, y as f32, z as f32]);
}

unsafe extern "C" fn glOrtho(l: f64, r: f64, b: f64, t: f64, n: f64, f: f64) {
    matrix(wire::MATRIX_ORTHO, &[l, r, b, t, n, f].map(|x| x as f32));
}

unsafe extern "C" fn glFrustum(l: f64, r: f64, b: f64, t: f64, n: f64, f: f64) {
    matrix(wire::MATRIX_FRUSTUM, &[l, r, b, t, n, f].map(|x| x as f32));
}

// ---- Conventional arrays and current values ----

fn pointer(index: u32, size: i32, kind: u32, normalized: bool, stride: i32, at: *const c_void) {
    let Some(c) = context() else { return };
    let mut p = Vec::new();
    for w in [index, size as u32, kind, u32::from(normalized), 0, stride.max(0) as u32] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, at as u64);
    c.record(wire::ATTRIB_POINTER, &p);
}

unsafe extern "C" fn glVertexPointer(size: i32, kind: u32, stride: i32, at: *const c_void) {
    pointer(0, size, kind, false, stride, at);
}

unsafe extern "C" fn glNormalPointer(kind: u32, stride: i32, at: *const c_void) {
    pointer(2, 3, kind, kind != FLOAT, stride, at);
}

unsafe extern "C" fn glColorPointer(size: i32, kind: u32, stride: i32, at: *const c_void) {
    pointer(3, size, kind, kind != FLOAT, stride, at);
}

unsafe extern "C" fn glSecondaryColorPointer(size: i32, kind: u32, stride: i32, at: *const c_void) {
    pointer(4, size, kind, kind != FLOAT, stride, at);
}

unsafe extern "C" fn glFogCoordPointer(kind: u32, stride: i32, at: *const c_void) {
    pointer(5, 1, kind, false, stride, at);
}

unsafe extern "C" fn glTexCoordPointer(size: i32, kind: u32, stride: i32, at: *const c_void) {
    let unit = context().map_or(0, |c| c.client_unit);
    pointer(8 + unit, size, kind, false, stride, at);
}

unsafe extern "C" fn glClientActiveTexture(unit: u32) {
    if let Some(c) = context() {
        c.client_unit = unit.wrapping_sub(0x84c0).min(7);
    }
}

/// A conventional array's attribute.
fn array_index(array: u32) -> Option<u32> {
    Some(match array {
        0x8074 => 0,
        0x8075 => 2,
        0x8076 => 3,
        0x845e => 4,
        0x8457 => 5,
        0x8078 => 8 + context().map_or(0, |c| c.client_unit),
        _ => return None,
    })
}

unsafe extern "C" fn glEnableClientState(array: u32) {
    if let Some(i) = array_index(array) {
        words(wire::ENABLE_ATTRIB, &[i, 1]);
    }
}

unsafe extern "C" fn glDisableClientState(array: u32) {
    if let Some(i) = array_index(array) {
        words(wire::ENABLE_ATTRIB, &[i, 0]);
    }
}

fn current_value(index: u32, v: [f32; 4]) {
    words(wire::ATTRIB_VALUE, &[index, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()]);
}

fn ub(x: u8) -> f32 {
    f32::from(x) / 255.0
}

unsafe extern "C" fn glColor3f(r: f32, g: f32, b: f32) {
    current_value(3, [r, g, b, 1.0]);
}

unsafe extern "C" fn glColor4f(r: f32, g: f32, b: f32, a: f32) {
    current_value(3, [r, g, b, a]);
}

unsafe extern "C" fn glColor3d(r: f64, g: f64, b: f64) {
    current_value(3, [r as f32, g as f32, b as f32, 1.0]);
}

unsafe extern "C" fn glColor4d(r: f64, g: f64, b: f64, a: f64) {
    current_value(3, [r as f32, g as f32, b as f32, a as f32]);
}

unsafe extern "C" fn glColor3ub(r: u8, g: u8, b: u8) {
    current_value(3, [ub(r), ub(g), ub(b), 1.0]);
}

unsafe extern "C" fn glColor4ub(r: u8, g: u8, b: u8, a: u8) {
    current_value(3, [ub(r), ub(g), ub(b), ub(a)]);
}

unsafe extern "C" fn glColor3fv(v: *const f32) {
    if !v.is_null() {
        current_value(3, unsafe { [*v, *v.add(1), *v.add(2), 1.0] });
    }
}

unsafe extern "C" fn glColor4fv(v: *const f32) {
    if !v.is_null() {
        current_value(3, unsafe { [*v, *v.add(1), *v.add(2), *v.add(3)] });
    }
}

unsafe extern "C" fn glColor4ubv(v: *const u8) {
    if !v.is_null() {
        current_value(3, unsafe { [ub(*v), ub(*v.add(1)), ub(*v.add(2)), ub(*v.add(3))] });
    }
}

unsafe extern "C" fn glNormal3f(x: f32, y: f32, z: f32) {
    current_value(2, [x, y, z, 1.0]);
}

unsafe extern "C" fn glNormal3fv(v: *const f32) {
    if !v.is_null() {
        current_value(2, unsafe { [*v, *v.add(1), *v.add(2), 1.0] });
    }
}

unsafe extern "C" fn glTexCoord2f(s: f32, t: f32) {
    current_value(8, [s, t, 0.0, 1.0]);
}

unsafe extern "C" fn glTexCoord2fv(v: *const f32) {
    if !v.is_null() {
        current_value(8, unsafe { [*v, *v.add(1), 0.0, 1.0] });
    }
}

unsafe extern "C" fn glMultiTexCoord2f(unit: u32, s: f32, t: f32) {
    current_value(8 + unit.wrapping_sub(0x84c0).min(7), [s, t, 0.0, 1.0]);
}

unsafe extern "C" fn glMultiTexCoord4f(unit: u32, s: f32, t: f32, r: f32, q: f32) {
    current_value(8 + unit.wrapping_sub(0x84c0).min(7), [s, t, r, q]);
}

unsafe extern "C" fn glSecondaryColor3f(r: f32, g: f32, b: f32) {
    current_value(4, [r, g, b, 1.0]);
}

unsafe extern "C" fn glFogCoordf(f: f32) {
    current_value(5, [f, 0.0, 0.0, 1.0]);
}

// ---- Texture environments, the alpha test, fog, shading ----

/// A parameter of up to four values, ints or floats.
fn parameter(kind: u32, pname: u32, values: [u32; 4], float: bool) {
    words(kind, &[pname, values[0], values[1], values[2], values[3], u32::from(float)]);
}

unsafe fn four<T: Copy>(v: *const T, n: usize, bits: impl Fn(T) -> u32) -> [u32; 4] {
    core::array::from_fn(|i| if i < n && !v.is_null() { bits(unsafe { *v.add(i) }) } else { 0 })
}

/// How many values a parameter has: a colour's four, else one.
fn count(pname: u32) -> usize {
    if matches!(pname, 0x2201 | 0x0b66) { 4 } else { 1 }
}

unsafe extern "C" fn glTexEnvi(target: u32, pname: u32, value: i32) {
    if target == 0x2300 {
        parameter(wire::TEX_ENV, pname, [value as u32, 0, 0, 0], false);
    }
}

unsafe extern "C" fn glTexEnvf(target: u32, pname: u32, value: f32) {
    if target == 0x2300 {
        parameter(wire::TEX_ENV, pname, [value.to_bits(), 0, 0, 0], true);
    }
}

unsafe extern "C" fn glTexEnviv(target: u32, pname: u32, v: *const i32) {
    if target == 0x2300 {
        parameter(wire::TEX_ENV, pname, unsafe { four(v, count(pname), |x| x as u32) }, false);
    }
}

unsafe extern "C" fn glTexEnvfv(target: u32, pname: u32, v: *const f32) {
    if target == 0x2300 {
        parameter(wire::TEX_ENV, pname, unsafe { four(v, count(pname), f32::to_bits) }, true);
    }
}

unsafe extern "C" fn glAlphaFunc(func: u32, reference: f32) {
    words(wire::ALPHA_FUNC, &[func, reference.to_bits()]);
}

unsafe extern "C" fn glFogf(pname: u32, value: f32) {
    parameter(wire::FOG, pname, [value.to_bits(), 0, 0, 0], true);
}

unsafe extern "C" fn glFogi(pname: u32, value: i32) {
    parameter(wire::FOG, pname, [value as u32, 0, 0, 0], false);
}

unsafe extern "C" fn glFogfv(pname: u32, v: *const f32) {
    parameter(wire::FOG, pname, unsafe { four(v, count(pname), f32::to_bits) }, true);
}

unsafe extern "C" fn glFogiv(pname: u32, v: *const i32) {
    parameter(wire::FOG, pname, unsafe { four(v, count(pname), |x| x as u32) }, false);
}

unsafe extern "C" fn glShadeModel(mode: u32) {
    words(wire::SHADE_MODEL, &[mode]);
}

// ---- Kept, not drawn yet ----

unsafe extern "C" fn glLightfv(_light: u32, _pname: u32, _v: *const f32) {
    not_yet(b"lighting", 1);
}

unsafe extern "C" fn glLightf(_light: u32, _pname: u32, _v: f32) {
    not_yet(b"lighting", 1);
}

unsafe extern "C" fn glLightModelfv(_pname: u32, _v: *const f32) {}

unsafe extern "C" fn glLightModeli(_pname: u32, _v: i32) {}

unsafe extern "C" fn glMaterialfv(_face: u32, _pname: u32, _v: *const f32) {
    not_yet(b"a material", 2);
}

unsafe extern "C" fn glMaterialf(_face: u32, _pname: u32, _v: f32) {
    not_yet(b"a material", 2);
}

unsafe extern "C" fn glColorMaterial(_face: u32, _mode: u32) {}

unsafe extern "C" fn glPolygonMode(_face: u32, mode: u32) {
    // GL_FILL is what's drawn.
    if mode != 0x1b02 {
        not_yet(b"a polygon mode other than GL_FILL", 4);
    }
}

unsafe extern "C" fn glPolygonOffset(factor: f32, units: f32) {
    if factor != 0.0 || units != 0.0 {
        not_yet(b"a polygon offset", 8);
    }
}

unsafe extern "C" fn glPointSize(_size: f32) {}

unsafe extern "C" fn glLineWidth(_width: f32) {}

unsafe extern "C" fn glClipPlane(_plane: u32, _equation: *const f64) {
    not_yet(b"a clip plane", 16);
}

unsafe extern "C" fn glTexGeni(_coord: u32, _pname: u32, _v: i32) {
    not_yet(b"texture coordinate generation", 32);
}

unsafe extern "C" fn glTexGenfv(_coord: u32, _pname: u32, _v: *const f32) {
    not_yet(b"texture coordinate generation", 32);
}

unsafe extern "C" fn glCopyTexSubImage2D(_target: u32, _level: i32, _x: i32, _y: i32, _ox: i32, _oy: i32, _w: i32, _h: i32) {
    not_yet(b"a copy from the framebuffer to a texture", 64);
}

unsafe extern "C" fn glRectf(x1: f32, y1: f32, x2: f32, y2: f32) {
    // Drawn at once, as a draw is: the arrays the program has on are left as they are.
    if let Some(c) = context() {
        let mut p = Vec::new();
        for v in [x1, y1, x2, y2] {
            records::put_u32(&mut p, v.to_bits());
        }
        c.record(wire::RECT, &p);
        c.sync(&[]);
    }
}

macro_rules! entry {
    ($f:ident) => {
        (stringify!($f).as_bytes(), $f as *const c_void)
    };
}

pub(crate) fn lookup(name: &[u8]) -> Option<*const c_void> {
    let table: &[(&[u8], *const c_void)] = &[
        entry!(glMatrixMode),
        entry!(glLoadIdentity),
        entry!(glLoadMatrixf),
        entry!(glLoadMatrixd),
        entry!(glMultMatrixf),
        entry!(glMultMatrixd),
        entry!(glLoadTransposeMatrixf),
        entry!(glMultTransposeMatrixf),
        entry!(glPushMatrix),
        entry!(glPopMatrix),
        entry!(glTranslatef),
        entry!(glTranslated),
        entry!(glScalef),
        entry!(glScaled),
        entry!(glRotatef),
        entry!(glRotated),
        entry!(glOrtho),
        entry!(glFrustum),
        entry!(glVertexPointer),
        entry!(glNormalPointer),
        entry!(glColorPointer),
        entry!(glSecondaryColorPointer),
        entry!(glFogCoordPointer),
        entry!(glTexCoordPointer),
        entry!(glClientActiveTexture),
        entry!(glEnableClientState),
        entry!(glDisableClientState),
        entry!(glColor3f),
        entry!(glColor4f),
        entry!(glColor3d),
        entry!(glColor4d),
        entry!(glColor3ub),
        entry!(glColor4ub),
        entry!(glColor3fv),
        entry!(glColor4fv),
        entry!(glColor4ubv),
        entry!(glNormal3f),
        entry!(glNormal3fv),
        entry!(glTexCoord2f),
        entry!(glTexCoord2fv),
        entry!(glMultiTexCoord2f),
        entry!(glMultiTexCoord4f),
        entry!(glSecondaryColor3f),
        entry!(glFogCoordf),
        entry!(glTexEnvi),
        entry!(glTexEnvf),
        entry!(glTexEnviv),
        entry!(glTexEnvfv),
        entry!(glAlphaFunc),
        entry!(glFogf),
        entry!(glFogi),
        entry!(glFogfv),
        entry!(glFogiv),
        entry!(glShadeModel),
        entry!(glLightfv),
        entry!(glLightf),
        entry!(glLightModelfv),
        entry!(glLightModeli),
        entry!(glMaterialfv),
        entry!(glMaterialf),
        entry!(glColorMaterial),
        entry!(glPolygonMode),
        entry!(glPolygonOffset),
        entry!(glPointSize),
        entry!(glLineWidth),
        entry!(glClipPlane),
        entry!(glTexGeni),
        entry!(glTexGenfv),
        entry!(glCopyTexSubImage2D),
        entry!(glRectf),
    ];
    table.iter().find(|e| e.0 == name).map(|e| e.1)
}
