//! GL's objects and draws, as GLVND's dispatch reaches them: shaders and programs, uniforms, buffers, vertex arrays,
//! textures, the state draws read, and the draws. Names are handed out here (one counter for every kind, so no two
//! objects share one); what each names is the host's. Calls that answer wait for the host; data (a buffer's, a
//! texture's texels packed as `glPixelStore` says) goes with the call, copied, as GL copies it.
//!
//! A draw is sent at once: one that reads the program's own arrays has them read while it waits.

use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

use gpu_wire::gl as wire;
use gpu_wire::records::{self, Reader};

use crate::current;
use crate::glx::Context;

static NAMES: AtomicU32 = AtomicU32::new(1);

fn context<'a>() -> Option<&'a mut Context> {
    unsafe { current().as_mut() }
}

fn record(c: &mut Context, kind: u32, words: &[u32], bytes: Option<&[u8]>, wide: Option<u64>) {
    let mut p = Vec::with_capacity(words.len() * 4 + bytes.map_or(0, |b| b.len() + 4) + 8);
    for &w in words {
        records::put_u32(&mut p, w);
    }
    if let Some(w) = wide {
        records::put_u64(&mut p, w);
    }
    if let Some(b) = bytes {
        records::put_bytes(&mut p, b);
    }
    c.record(kind, &p);
}

fn call(kind: u32, words: &[u32]) {
    if let Some(c) = context() {
        record(c, kind, words, None, None);
    }
}

/// A call that answers: sent with what's recorded, and its answers read.
fn ask(c: &mut Context, kind: u32, words: &[u32], bytes: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut then = Vec::new();
    let mut p = Vec::new();
    for &w in words {
        records::put_u32(&mut p, w);
    }
    if let Some(b) = bytes {
        records::put_bytes(&mut p, b);
    }
    records::record(&mut then, kind, &p);
    c.sync(&then)
}

fn ask_int(kind: u32, words: &[u32], bytes: Option<&[u8]>) -> i32 {
    let Some(c) = context() else { return 0 };
    ask(c, kind, words, bytes).map_or(0, |a| Reader::new(&a).u32().unwrap_or(0) as i32)
}

fn names(n: i32, out: *mut u32, made: impl Fn(&mut Context, u32)) {
    if out.is_null() || n < 0 {
        return;
    }
    for i in 0..n as usize {
        let name = NAMES.fetch_add(1, Ordering::Relaxed);
        unsafe { *out.add(i) = name };
        if let Some(c) = context() {
            made(c, name);
        }
    }
}

fn each(n: i32, list: *const u32, kind: u32) {
    if list.is_null() || n < 0 {
        return;
    }
    for i in 0..n as usize {
        let name = unsafe { *list.add(i) };
        if name != 0 {
            call(kind, &[name]);
        }
    }
}

fn c_str<'a>(p: *const c_char) -> &'a [u8] {
    if p.is_null() { &[] } else { unsafe { CStr::from_ptr(p) }.to_bytes() }
}

/// Text answered by the host, copied to the program's buffer as GL copies it: cut to fit, with a NUL.
fn copy_out(text: &[u8], size: i32, length: *mut i32, out: *mut c_char) {
    let n = if size > 0 { text.len().min(size as usize - 1) } else { 0 };
    if !out.is_null() && size > 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr(), out.cast::<u8>(), n);
            *out.add(n) = 0;
        }
    }
    if !length.is_null() {
        unsafe { *length = n as i32 };
    }
}

// ---- Shaders and programs ----

unsafe extern "C" fn glCreateShader(kind: u32) -> u32 {
    let name = NAMES.fetch_add(1, Ordering::Relaxed);
    call(wire::SHADER, &[name, kind]);
    name
}

unsafe extern "C" fn glShaderSource(shader: u32, count: i32, strings: *const *const c_char, lengths: *const i32) {
    let Some(c) = context() else { return };
    let mut source = Vec::new();
    for i in 0..count.max(0) as usize {
        let s = unsafe { *strings.add(i) };
        if s.is_null() {
            continue;
        }
        let len = if lengths.is_null() { -1 } else { unsafe { *lengths.add(i) } };
        if len < 0 {
            source.extend_from_slice(c_str(s));
        } else {
            source.extend_from_slice(unsafe { core::slice::from_raw_parts(s.cast::<u8>(), len as usize) });
        }
    }
    record(c, wire::SHADER_SOURCE, &[shader], Some(&source), None);
}

unsafe extern "C" fn glCompileShader(shader: u32) {
    call(wire::COMPILE, &[shader]);
}

unsafe extern "C" fn glDeleteShader(shader: u32) {
    call(wire::DELETE_SHADER, &[shader]);
}

unsafe extern "C" fn glGetShaderiv(shader: u32, pname: u32, out: *mut i32) {
    let v = ask_int(wire::GET_SHADER, &[shader, pname], None);
    if !out.is_null() {
        unsafe { *out = v };
    }
}

unsafe extern "C" fn glGetShaderInfoLog(shader: u32, size: i32, length: *mut i32, out: *mut c_char) {
    let Some(c) = context() else { return };
    let a = ask(c, wire::SHADER_LOG, &[shader], None).unwrap_or_default();
    copy_out(Reader::new(&a).bytes().unwrap_or(&[]), size, length, out);
}

unsafe extern "C" fn glCreateProgram() -> u32 {
    let name = NAMES.fetch_add(1, Ordering::Relaxed);
    call(wire::PROGRAM, &[name]);
    name
}

unsafe extern "C" fn glAttachShader(program: u32, shader: u32) {
    call(wire::ATTACH, &[program, shader]);
}

unsafe extern "C" fn glDetachShader(program: u32, shader: u32) {
    call(wire::DETACH, &[program, shader]);
}

unsafe extern "C" fn glBindAttribLocation(program: u32, index: u32, name: *const c_char) {
    if let Some(c) = context() {
        record(c, wire::BIND_ATTRIB, &[program, index], Some(c_str(name)), None);
    }
}

unsafe extern "C" fn glLinkProgram(program: u32) {
    call(wire::LINK, &[program]);
}

unsafe extern "C" fn glValidateProgram(program: u32) {
    call(wire::VALIDATE, &[program]);
}

unsafe extern "C" fn glUseProgram(program: u32) {
    call(wire::USE_PROGRAM, &[program]);
}

unsafe extern "C" fn glDeleteProgram(program: u32) {
    if program != 0 {
        call(wire::DELETE_PROGRAM, &[program]);
    }
}

unsafe extern "C" fn glGetProgramiv(program: u32, pname: u32, out: *mut i32) {
    let v = ask_int(wire::GET_PROGRAM, &[program, pname], None);
    if !out.is_null() {
        unsafe { *out = v };
    }
}

unsafe extern "C" fn glGetProgramInfoLog(program: u32, size: i32, length: *mut i32, out: *mut c_char) {
    let Some(c) = context() else { return };
    let a = ask(c, wire::PROGRAM_LOG, &[program], None).unwrap_or_default();
    copy_out(Reader::new(&a).bytes().unwrap_or(&[]), size, length, out);
}

unsafe extern "C" fn glGetUniformLocation(program: u32, name: *const c_char) -> i32 {
    ask_int(wire::UNIFORM_LOCATION, &[program], Some(c_str(name)))
}

unsafe extern "C" fn glGetAttribLocation(program: u32, name: *const c_char) -> i32 {
    ask_int(wire::ATTRIB_LOCATION, &[program], Some(c_str(name)))
}

#[allow(clippy::too_many_arguments)]
fn active(kind: u32, program: u32, index: u32, size: i32, length: *mut i32, count: *mut i32, ty: *mut u32, out: *mut c_char) {
    let Some(c) = context() else { return };
    let a = ask(c, kind, &[program, index], None).unwrap_or_default();
    let mut r = Reader::new(&a);
    let (n, t) = (r.u32().unwrap_or(0), r.u32().unwrap_or(0));
    let name = r.bytes().unwrap_or(&[]);
    unsafe {
        if !count.is_null() {
            *count = n as i32;
        }
        if !ty.is_null() {
            *ty = t;
        }
    }
    copy_out(name, size, length, out);
}

unsafe extern "C" fn glGetActiveUniform(program: u32, index: u32, size: i32, length: *mut i32, count: *mut i32, ty: *mut u32, name: *mut c_char) {
    active(wire::ACTIVE_UNIFORM, program, index, size, length, count, ty, name);
}

unsafe extern "C" fn glGetActiveAttrib(program: u32, index: u32, size: i32, length: *mut i32, count: *mut i32, ty: *mut u32, name: *mut c_char) {
    active(wire::ACTIVE_ATTRIB, program, index, size, length, count, ty, name);
}

// ---- Uniforms ----

/// `glUniform*`: `count` elements of `cols` by `rows` words of a kind, from the program's memory.
fn uniform(location: i32, kind: u32, cols: u32, rows: u32, count: i32, transpose: bool, words: *const u32) {
    let Some(c) = context() else { return };
    if words.is_null() || count < 0 {
        return;
    }
    let n = (cols * rows) as usize * count as usize;
    let bytes: Vec<u8> = (0..n).flat_map(|i| unsafe { *words.add(i) }.to_le_bytes()).collect();
    record(c, wire::UNIFORM, &[location as u32, kind, cols, rows, count as u32, u32::from(transpose)], Some(&bytes), None);
}

macro_rules! uniform_values {
    ($name:ident, $kind:expr, $t:ty, $($v:ident),+) => {
        unsafe extern "C" fn $name(location: i32, $($v: $t),+) {
            let words = [$($v.to_bits_word()),+];
            uniform(location, $kind, words.len() as u32, 1, 1, false, words.as_ptr());
        }
    };
}

macro_rules! uniform_arrays {
    ($name:ident, $kind:expr, $n:expr, $t:ty) => {
        unsafe extern "C" fn $name(location: i32, count: i32, v: *const $t) {
            uniform(location, $kind, $n, 1, count, false, v as *const u32);
        }
    };
}

macro_rules! uniform_matrices {
    ($name:ident, $cols:expr, $rows:expr) => {
        unsafe extern "C" fn $name(location: i32, count: i32, transpose: u8, v: *const f32) {
            uniform(location, wire::FLOAT, $cols, $rows, count, transpose != 0, v as *const u32);
        }
    };
}

/// A uniform's value as the word that carries it.
trait Word {
    fn to_bits_word(self) -> u32;
}

impl Word for f32 {
    fn to_bits_word(self) -> u32 {
        self.to_bits()
    }
}

impl Word for i32 {
    fn to_bits_word(self) -> u32 {
        self as u32
    }
}

impl Word for u32 {
    fn to_bits_word(self) -> u32 {
        self
    }
}

uniform_values!(glUniform1f, wire::FLOAT, f32, x);
uniform_values!(glUniform2f, wire::FLOAT, f32, x, y);
uniform_values!(glUniform3f, wire::FLOAT, f32, x, y, z);
uniform_values!(glUniform4f, wire::FLOAT, f32, x, y, z, w);
uniform_values!(glUniform1i, wire::INT, i32, x);
uniform_values!(glUniform2i, wire::INT, i32, x, y);
uniform_values!(glUniform3i, wire::INT, i32, x, y, z);
uniform_values!(glUniform4i, wire::INT, i32, x, y, z, w);
uniform_values!(glUniform1ui, wire::UINT, u32, x);
uniform_values!(glUniform2ui, wire::UINT, u32, x, y);
uniform_values!(glUniform3ui, wire::UINT, u32, x, y, z);
uniform_values!(glUniform4ui, wire::UINT, u32, x, y, z, w);
uniform_arrays!(glUniform1fv, wire::FLOAT, 1, f32);
uniform_arrays!(glUniform2fv, wire::FLOAT, 2, f32);
uniform_arrays!(glUniform3fv, wire::FLOAT, 3, f32);
uniform_arrays!(glUniform4fv, wire::FLOAT, 4, f32);
uniform_arrays!(glUniform1iv, wire::INT, 1, i32);
uniform_arrays!(glUniform2iv, wire::INT, 2, i32);
uniform_arrays!(glUniform3iv, wire::INT, 3, i32);
uniform_arrays!(glUniform4iv, wire::INT, 4, i32);
uniform_arrays!(glUniform1uiv, wire::UINT, 1, u32);
uniform_arrays!(glUniform2uiv, wire::UINT, 2, u32);
uniform_arrays!(glUniform3uiv, wire::UINT, 3, u32);
uniform_arrays!(glUniform4uiv, wire::UINT, 4, u32);
uniform_matrices!(glUniformMatrix2fv, 2, 2);
uniform_matrices!(glUniformMatrix3fv, 3, 3);
uniform_matrices!(glUniformMatrix4fv, 4, 4);
uniform_matrices!(glUniformMatrix2x3fv, 2, 3);
uniform_matrices!(glUniformMatrix3x2fv, 3, 2);
uniform_matrices!(glUniformMatrix2x4fv, 2, 4);
uniform_matrices!(glUniformMatrix4x2fv, 4, 2);
uniform_matrices!(glUniformMatrix3x4fv, 3, 4);
uniform_matrices!(glUniformMatrix4x3fv, 4, 3);

// ---- Buffers and vertex arrays ----

unsafe extern "C" fn glGenBuffers(n: i32, out: *mut u32) {
    names(n, out, |_, _| {});
}

unsafe extern "C" fn glDeleteBuffers(n: i32, list: *const u32) {
    each(n, list, wire::DELETE_BUFFER);
}

unsafe extern "C" fn glBindBuffer(target: u32, name: u32) {
    call(wire::BIND_BUFFER, &[target, name]);
}

unsafe extern "C" fn glBufferData(target: u32, size: isize, data: *const c_void, usage: u32) {
    let Some(c) = context() else { return };
    let size = size.max(0) as usize;
    let bytes: &[u8] = if data.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(data as *const u8, size) } };
    let mut p = Vec::with_capacity(bytes.len() + 24);
    records::put_u32(&mut p, target);
    records::put_u64(&mut p, size as u64);
    records::put_u32(&mut p, usage);
    records::put_u32(&mut p, u32::from(!data.is_null()));
    records::put_bytes(&mut p, bytes);
    c.record(wire::BUFFER_DATA, &p);
}

unsafe extern "C" fn glBufferSubData(target: u32, offset: isize, size: isize, data: *const c_void) {
    let Some(c) = context() else { return };
    if data.is_null() || size <= 0 {
        return;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data as *const u8, size as usize) };
    let mut p = Vec::with_capacity(bytes.len() + 16);
    records::put_u32(&mut p, target);
    records::put_u64(&mut p, offset as u64);
    records::put_bytes(&mut p, bytes);
    c.record(wire::BUFFER_SUB_DATA, &p);
}

unsafe extern "C" fn glGenVertexArrays(n: i32, out: *mut u32) {
    names(n, out, |_, _| {});
}

unsafe extern "C" fn glDeleteVertexArrays(n: i32, list: *const u32) {
    each(n, list, wire::DELETE_VERTEX_ARRAY);
}

unsafe extern "C" fn glBindVertexArray(name: u32) {
    call(wire::BIND_VERTEX_ARRAY, &[name]);
}

fn pointer(index: u32, size: i32, kind: u32, normalized: bool, integer: bool, stride: i32, at: *const c_void) {
    if let Some(c) = context() {
        record(c, wire::ATTRIB_POINTER, &[index, size as u32, kind, u32::from(normalized), u32::from(integer), stride.max(0) as u32], None, Some(at as u64));
    }
}

unsafe extern "C" fn glVertexAttribPointer(index: u32, size: i32, kind: u32, normalized: u8, stride: i32, at: *const c_void) {
    pointer(index, size, kind, normalized != 0, false, stride, at);
}

unsafe extern "C" fn glVertexAttribIPointer(index: u32, size: i32, kind: u32, stride: i32, at: *const c_void) {
    pointer(index, size, kind, false, true, stride, at);
}

unsafe extern "C" fn glEnableVertexAttribArray(index: u32) {
    call(wire::ENABLE_ATTRIB, &[index, 1]);
}

unsafe extern "C" fn glDisableVertexAttribArray(index: u32) {
    call(wire::ENABLE_ATTRIB, &[index, 0]);
}

unsafe extern "C" fn glVertexAttribDivisor(index: u32, divisor: u32) {
    call(wire::ATTRIB_DIVISOR, &[index, divisor]);
}

fn attrib(index: u32, v: [f32; 4]) {
    call(wire::ATTRIB_VALUE, &[index, v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()]);
}

unsafe extern "C" fn glVertexAttrib1f(index: u32, x: f32) {
    attrib(index, [x, 0.0, 0.0, 1.0]);
}

unsafe extern "C" fn glVertexAttrib2f(index: u32, x: f32, y: f32) {
    attrib(index, [x, y, 0.0, 1.0]);
}

unsafe extern "C" fn glVertexAttrib3f(index: u32, x: f32, y: f32, z: f32) {
    attrib(index, [x, y, z, 1.0]);
}

unsafe extern "C" fn glVertexAttrib4f(index: u32, x: f32, y: f32, z: f32, w: f32) {
    attrib(index, [x, y, z, w]);
}

unsafe extern "C" fn glVertexAttrib4fv(index: u32, v: *const f32) {
    if !v.is_null() {
        attrib(index, unsafe { [*v, *v.add(1), *v.add(2), *v.add(3)] });
    }
}

// ---- Textures ----

unsafe extern "C" fn glGenTextures(n: i32, out: *mut u32) {
    names(n, out, |_, _| {});
}

unsafe extern "C" fn glDeleteTextures(n: i32, list: *const u32) {
    each(n, list, wire::DELETE_TEXTURE);
}

unsafe extern "C" fn glBindTexture(target: u32, name: u32) {
    call(wire::BIND_TEXTURE, &[target, name]);
}

unsafe extern "C" fn glActiveTexture(unit: u32) {
    call(wire::ACTIVE_TEXTURE, &[unit.wrapping_sub(0x84c0)]);
}

/// Bytes a pixel of a format and type takes, as the program lays them out.
fn pixel_bytes(format: u32, kind: u32) -> usize {
    let comps = match format {
        0x1903 | 0x1906 | 0x1909 | 0x1902 => 1,
        0x8227 | 0x190a => 2,
        0x1907 | 0x80e0 => 3,
        _ => 4,
    };
    match kind {
        0x1401 | 0x1400 => comps,
        0x1402 | 0x1403 | 0x140b => comps * 2,
        0x8363 | 0x8033 | 0x8034 => 2,
        0x8367 => 4,
        _ => comps * 4,
    }
}

/// A texture image's texels from the program's memory, rows tightly packed: unpacking's row length, skips and
/// alignment applied.
fn unpack(c: &Context, width: i32, height: i32, format: u32, kind: u32, pixels: *const c_void) -> Vec<u8> {
    let (w, h) = (width.max(0) as usize, height.max(0) as usize);
    let size = pixel_bytes(format, kind);
    let row = w * size;
    let length = c.store.unpack_row_length().max(0) as usize;
    let alignment = c.store.unpack_alignment().max(1) as usize;
    let stride = (if length > 0 { length } else { w } * size).next_multiple_of(alignment);
    let start = c.store.unpack_skip_rows().max(0) as usize * stride + c.store.unpack_skip_pixels().max(0) as usize * size;
    let mut out = Vec::with_capacity(row * h);
    for y in 0..h {
        let at = unsafe { (pixels as *const u8).add(start + y * stride) };
        out.extend_from_slice(unsafe { core::slice::from_raw_parts(at, row) });
    }
    out
}

unsafe extern "C" fn glTexImage2D(target: u32, level: i32, internal: i32, width: i32, height: i32, _border: i32, format: u32, kind: u32, pixels: *const c_void) {
    let Some(c) = context() else { return };
    let texels = if pixels.is_null() { Vec::new() } else { unpack(c, width, height, format, kind, pixels) };
    let words = [target, level as u32, internal as u32, width as u32, height as u32, format, kind, u32::from(!pixels.is_null())];
    record(c, wire::TEX_IMAGE, &words, Some(&texels), None);
}

unsafe extern "C" fn glTexSubImage2D(target: u32, level: i32, x: i32, y: i32, width: i32, height: i32, format: u32, kind: u32, pixels: *const c_void) {
    let Some(c) = context() else { return };
    if pixels.is_null() {
        return;
    }
    let texels = unpack(c, width, height, format, kind, pixels);
    record(c, wire::TEX_SUB_IMAGE, &[target, level as u32, x as u32, y as u32, width as u32, height as u32, format, kind], Some(&texels), None);
}

unsafe extern "C" fn glTexParameteri(target: u32, pname: u32, value: i32) {
    call(wire::TEX_PARAMETER, &[target, pname, value as u32, 0]);
}

unsafe extern "C" fn glTexParameterf(target: u32, pname: u32, value: f32) {
    call(wire::TEX_PARAMETER, &[target, pname, value.to_bits(), 1]);
}

unsafe extern "C" fn glTexParameteriv(target: u32, pname: u32, v: *const i32) {
    if !v.is_null() {
        unsafe { glTexParameteri(target, pname, *v) };
    }
}

unsafe extern "C" fn glTexParameterfv(target: u32, pname: u32, v: *const f32) {
    if !v.is_null() {
        unsafe { glTexParameterf(target, pname, *v) };
    }
}

unsafe extern "C" fn glGenerateMipmap(target: u32) {
    call(wire::GENERATE_MIPMAP, &[target]);
}

// ---- State ----

unsafe extern "C" fn glBlendFunc(src: u32, dst: u32) {
    call(wire::BLEND_FUNC, &[src, dst, src, dst]);
}

unsafe extern "C" fn glBlendFuncSeparate(src_rgb: u32, dst_rgb: u32, src_alpha: u32, dst_alpha: u32) {
    call(wire::BLEND_FUNC, &[src_rgb, dst_rgb, src_alpha, dst_alpha]);
}

unsafe extern "C" fn glBlendEquation(mode: u32) {
    call(wire::BLEND_EQUATION, &[mode, mode]);
}

unsafe extern "C" fn glBlendEquationSeparate(rgb: u32, alpha: u32) {
    call(wire::BLEND_EQUATION, &[rgb, alpha]);
}

unsafe extern "C" fn glDepthFunc(f: u32) {
    call(wire::DEPTH_FUNC, &[f]);
}

unsafe extern "C" fn glCullFace(mode: u32) {
    call(wire::CULL_FACE, &[mode]);
}

unsafe extern "C" fn glFrontFace(mode: u32) {
    call(wire::FRONT_FACE, &[mode]);
}

// ---- Draws ----

/// A draw, sent at once: the program's own arrays are read while it waits.
fn draw(kind: u32, words: &[u32], at: Option<u64>) {
    let Some(c) = context() else { return };
    record(c, kind, words, None, at);
    c.sync(&[]);
}

unsafe extern "C" fn glDrawArrays(mode: u32, first: i32, count: i32) {
    draw(wire::DRAW_ARRAYS, &[mode, first as u32, count.max(0) as u32, 1], None);
}

unsafe extern "C" fn glDrawArraysInstanced(mode: u32, first: i32, count: i32, instances: i32) {
    draw(wire::DRAW_ARRAYS, &[mode, first as u32, count.max(0) as u32, instances.max(0) as u32], None);
}

unsafe extern "C" fn glDrawElements(mode: u32, count: i32, kind: u32, at: *const c_void) {
    draw(wire::DRAW_ELEMENTS, &[mode, count.max(0) as u32, kind, 1, 0], Some(at as u64));
}

unsafe extern "C" fn glDrawElementsInstanced(mode: u32, count: i32, kind: u32, at: *const c_void, instances: i32) {
    draw(wire::DRAW_ELEMENTS, &[mode, count.max(0) as u32, kind, instances.max(0) as u32, 0], Some(at as u64));
}

unsafe extern "C" fn glDrawElementsBaseVertex(mode: u32, count: i32, kind: u32, at: *const c_void, base: i32) {
    draw(wire::DRAW_ELEMENTS, &[mode, count.max(0) as u32, kind, 1, base as u32], Some(at as u64));
}

unsafe extern "C" fn glDrawRangeElements(mode: u32, _start: u32, _end: u32, count: i32, kind: u32, at: *const c_void) {
    unsafe { glDrawElements(mode, count, kind, at) }
}

macro_rules! entry {
    ($f:ident) => {
        (stringify!($f).as_bytes(), $f as *const c_void)
    };
}

/// The functions here, by name.
pub(crate) fn lookup(name: &[u8]) -> Option<*const c_void> {
    let table: &[(&[u8], *const c_void)] = &[
        entry!(glCreateShader),
        entry!(glShaderSource),
        entry!(glCompileShader),
        entry!(glDeleteShader),
        entry!(glGetShaderiv),
        entry!(glGetShaderInfoLog),
        entry!(glCreateProgram),
        entry!(glAttachShader),
        entry!(glDetachShader),
        entry!(glBindAttribLocation),
        entry!(glLinkProgram),
        entry!(glValidateProgram),
        entry!(glUseProgram),
        entry!(glDeleteProgram),
        entry!(glGetProgramiv),
        entry!(glGetProgramInfoLog),
        entry!(glGetUniformLocation),
        entry!(glGetAttribLocation),
        entry!(glGetActiveUniform),
        entry!(glGetActiveAttrib),
        entry!(glUniform1f),
        entry!(glUniform2f),
        entry!(glUniform3f),
        entry!(glUniform4f),
        entry!(glUniform1i),
        entry!(glUniform2i),
        entry!(glUniform3i),
        entry!(glUniform4i),
        entry!(glUniform1ui),
        entry!(glUniform2ui),
        entry!(glUniform3ui),
        entry!(glUniform4ui),
        entry!(glUniform1fv),
        entry!(glUniform2fv),
        entry!(glUniform3fv),
        entry!(glUniform4fv),
        entry!(glUniform1iv),
        entry!(glUniform2iv),
        entry!(glUniform3iv),
        entry!(glUniform4iv),
        entry!(glUniform1uiv),
        entry!(glUniform2uiv),
        entry!(glUniform3uiv),
        entry!(glUniform4uiv),
        entry!(glUniformMatrix2fv),
        entry!(glUniformMatrix3fv),
        entry!(glUniformMatrix4fv),
        entry!(glUniformMatrix2x3fv),
        entry!(glUniformMatrix3x2fv),
        entry!(glUniformMatrix2x4fv),
        entry!(glUniformMatrix4x2fv),
        entry!(glUniformMatrix3x4fv),
        entry!(glUniformMatrix4x3fv),
        entry!(glGenBuffers),
        entry!(glDeleteBuffers),
        entry!(glBindBuffer),
        entry!(glBufferData),
        entry!(glBufferSubData),
        entry!(glGenVertexArrays),
        entry!(glDeleteVertexArrays),
        entry!(glBindVertexArray),
        entry!(glVertexAttribPointer),
        entry!(glVertexAttribIPointer),
        entry!(glEnableVertexAttribArray),
        entry!(glDisableVertexAttribArray),
        entry!(glVertexAttribDivisor),
        entry!(glVertexAttrib1f),
        entry!(glVertexAttrib2f),
        entry!(glVertexAttrib3f),
        entry!(glVertexAttrib4f),
        entry!(glVertexAttrib4fv),
        entry!(glGenTextures),
        entry!(glDeleteTextures),
        entry!(glBindTexture),
        entry!(glActiveTexture),
        entry!(glTexImage2D),
        entry!(glTexSubImage2D),
        entry!(glTexParameteri),
        entry!(glTexParameterf),
        entry!(glTexParameteriv),
        entry!(glTexParameterfv),
        entry!(glGenerateMipmap),
        entry!(glBlendFunc),
        entry!(glBlendFuncSeparate),
        entry!(glBlendEquation),
        entry!(glBlendEquationSeparate),
        entry!(glDepthFunc),
        entry!(glCullFace),
        entry!(glFrontFace),
        entry!(glDrawArrays),
        entry!(glDrawArraysInstanced),
        entry!(glDrawElements),
        entry!(glDrawElementsInstanced),
        entry!(glDrawElementsBaseVertex),
        entry!(glDrawRangeElements),
    ];
    table.iter().find(|e| e.0 == name).map(|e| e.1)
}
