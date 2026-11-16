//! GL on the CPU device, through its records: clears, gets and errors, swaps, and draws.

use super::*;
use gpu_wire::gl as wire;
use gpu_wire::records::{self, Reader};

/// A program's memory, as a vector.
struct Memory(Vec<u8>);

impl Guest for Memory {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), String> {
        out.copy_from_slice(&self.0[addr as usize..addr as usize + out.len()]);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
        self.0[addr as usize..addr as usize + data.len()].copy_from_slice(data);
        Ok(())
    }
}

fn record(out: &mut Vec<u8>, kind: u32, words: &[u32]) {
    let mut p = Vec::new();
    for &w in words {
        records::put_u32(&mut p, w);
    }
    records::record(out, kind, &p);
}

fn record64(out: &mut Vec<u8>, kind: u32, first: u64, words: &[u32]) {
    let mut p = Vec::new();
    records::put_u64(&mut p, first);
    for &w in words {
        records::put_u32(&mut p, w);
    }
    records::record(out, kind, &p);
}

/// A context bound to a 4×2 drawable with depth.
fn bound(gl: &Gl, memory: &mut Memory) {
    let mut r = Vec::new();
    record64(&mut r, wire::CONTEXT, 1, &[0, 0]);
    record64(&mut r, wire::USE, 1, &[]);
    record64(&mut r, wire::BIND, 9, &[4, 2, 24, 8, 0]);
    assert_eq!(gl.execute(&r, memory), wire::OK.to_le_bytes());
}

/// The whole 4×2 colour buffer as RGBA, lowest row first.
fn read_back(gl: &Gl, memory: &mut Memory) -> Vec<u8> {
    // Six words, then the address and the row's bytes.
    let mut p = Vec::new();
    for w in [0, 0, 4, 2, RGBA, UNSIGNED_BYTE] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, 0);
    records::put_u32(&mut p, 16);
    let mut r = Vec::new();
    record64(&mut r, wire::USE, 1, &[]);
    records::record(&mut r, wire::READ_PIXELS, &p);
    assert_eq!(gl.execute(&r, memory), wire::OK.to_le_bytes());
    memory.0[..32].to_vec()
}

#[test]
fn clears_whole_scissored_and_masked() {
    let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
    let gl = Gl::new(gpu);
    let mut memory = Memory(vec![0; 256]);
    bound(&gl, &mut memory);
    let red = [1.0f32, 0.0, 0.0, 1.0].map(f32::to_bits);
    let mut r = Vec::new();
    record64(&mut r, wire::USE, 1, &[]);
    record(&mut r, wire::CLEAR_COLOR, &red);
    record(&mut r, wire::CLEAR, &[COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT]);
    assert_eq!(gl.execute(&r, &mut memory), wire::OK.to_le_bytes());
    assert_eq!(read_back(&gl, &mut memory), [255, 0, 0, 255].repeat(8));
    // Green, only in the scissor (x 1 to 2, the top row), and only its green channel.
    let green = [0.0f32, 1.0, 0.0, 1.0].map(f32::to_bits);
    let mut r = Vec::new();
    record64(&mut r, wire::USE, 1, &[]);
    record(&mut r, wire::CLEAR_COLOR, &green);
    record(&mut r, wire::SCISSOR, &[1, 1, 2, 1]);
    record(&mut r, wire::ENABLE, &[SCISSOR_TEST]);
    record(&mut r, wire::COLOR_MASK, &[0, 1, 0, 0]);
    record(&mut r, wire::CLEAR, &[COLOR_BUFFER_BIT]);
    assert_eq!(gl.execute(&r, &mut memory), wire::OK.to_le_bytes());
    let yellow = [255, 255, 0, 255];
    let red = [255, 0, 0, 255];
    let want: Vec<u8> = [red, red, red, red, red, yellow, yellow, red].concat();
    assert_eq!(read_back(&gl, &mut memory), want);
}

#[test]
fn gets_errors_and_the_first_bind() {
    let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
    let gl = Gl::new(gpu);
    let mut memory = Memory(vec![0; 64]);
    bound(&gl, &mut memory);
    let mut r = Vec::new();
    record64(&mut r, wire::USE, 1, &[]);
    record(&mut r, wire::GET, &[0x0ba2]);
    record(&mut r, wire::ENABLE, &[0x1234]);
    record(&mut r, wire::GET_ERROR, &[]);
    record(&mut r, wire::GET_ERROR, &[]);
    let reply = gl.execute(&r, &mut memory);
    let mut a = Reader::new(&reply);
    assert_eq!(a.u32().unwrap(), wire::OK);
    assert_eq!(a.u32().unwrap(), 4);
    let viewport: Vec<(u32, u64)> = (0..4).map(|_| (a.u32().unwrap(), a.u64().unwrap())).collect();
    assert_eq!(viewport, [(INT, 0), (INT, 0), (INT, 4), (INT, 2)]);
    assert_eq!((a.u32().unwrap(), a.u32().unwrap()), (INVALID_ENUM, NO_ERROR));
    assert!(a.is_empty());
}

#[test]
fn swaps_turn_the_frame_the_right_way_up() {
    let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
    let gl = Gl::new(gpu);
    let mut memory = Memory(vec![0; 64]);
    bound(&gl, &mut memory);
    // The bottom row blue, the top row white.
    let mut r = Vec::new();
    record64(&mut r, wire::USE, 1, &[]);
    record(&mut r, wire::CLEAR_COLOR, &[0.0f32, 0.0, 1.0, 1.0].map(f32::to_bits));
    record(&mut r, wire::CLEAR, &[COLOR_BUFFER_BIT]);
    record(&mut r, wire::CLEAR_COLOR, &[1.0f32; 4].map(f32::to_bits));
    record(&mut r, wire::SCISSOR, &[0, 1, 4, 1]);
    record(&mut r, wire::ENABLE, &[SCISSOR_TEST]);
    record(&mut r, wire::CLEAR, &[COLOR_BUFFER_BIT]);
    let mut p = Vec::new();
    records::put_u64(&mut p, 9);
    records::put_u64(&mut p, 0);
    records::put_u32(&mut p, 16);
    records::record(&mut r, wire::SWAP, &p);
    assert_eq!(gl.execute(&r, &mut memory), wire::OK.to_le_bytes());
    // BGRA, top row first.
    assert_eq!(&memory.0[..32], [[255u8, 255, 255, 255].repeat(4), [255, 0, 0, 255].repeat(4)].concat());
}

/// Records built call by call, as the guest library builds them.
#[derive(Default)]
struct Calls(Vec<u8>);

impl Calls {
    fn new() -> Calls {
        let mut c = Calls::default();
        c.raw(wire::USE, &1u64.to_le_bytes());
        c
    }
    fn raw(&mut self, kind: u32, payload: &[u8]) -> &mut Calls {
        records::record(&mut self.0, kind, payload);
        self
    }
    fn w(&mut self, kind: u32, words: &[u32]) -> &mut Calls {
        let p: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.raw(kind, &p)
    }
    /// Words, then bytes with their length.
    fn wb(&mut self, kind: u32, words: &[u32], bytes: &[u8]) -> &mut Calls {
        let mut p: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        records::put_bytes(&mut p, bytes);
        self.raw(kind, &p)
    }
    /// Words, then a 64-bit value.
    fn w64(&mut self, kind: u32, words: &[u32], last: u64) -> &mut Calls {
        let mut p: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        records::put_u64(&mut p, last);
        self.raw(kind, &p)
    }
    fn floats(&mut self, kind: u32, words: &[u32], floats: &[f32]) -> &mut Calls {
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.wb(kind, words, &bytes)
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Runs calls: the reply's answers, after its status.
fn calls(gl: &Gl, memory: &mut Memory, c: &Calls) -> Vec<u8> {
    let reply = gl.execute(&c.0, memory);
    assert_eq!(&reply[..4], &wire::OK.to_le_bytes(), "{}", String::from_utf8_lossy(&reply[4..]));
    reply[4..].to_vec()
}

/// A context bound to an 8×8 drawable with depth, and a program from two shaders (names 1 and 2, program 3).
fn with_program(vs: &str, fs: &str) -> (Gl, Memory) {
    let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
    let gl = Gl::new(gpu);
    let mut memory = Memory(vec![0; 8192]);
    let mut c = Calls::default();
    c.raw(wire::CONTEXT, &[1u64.to_le_bytes(), 0u64.to_le_bytes()].concat());
    c.raw(wire::USE, &1u64.to_le_bytes());
    let mut b = Vec::new();
    records::put_u64(&mut b, 9);
    for w in [8u32, 8, 24, 8, 0] {
        records::put_u32(&mut b, w);
    }
    c.raw(wire::BIND, &b);
    c.w(wire::SHADER, &[1, 0x8b31]).wb(wire::SHADER_SOURCE, &[1], vs.as_bytes()).w(wire::COMPILE, &[1]);
    c.w(wire::SHADER, &[2, 0x8b30]).wb(wire::SHADER_SOURCE, &[2], fs.as_bytes()).w(wire::COMPILE, &[2]);
    c.w(wire::PROGRAM, &[3]).w(wire::ATTACH, &[3, 1]).w(wire::ATTACH, &[3, 2]).w(wire::LINK, &[3]);
    c.w(wire::GET_PROGRAM, &[3, 0x8b82]).w(wire::PROGRAM_LOG, &[3]).w(wire::USE_PROGRAM, &[3]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    let linked = r.u32().unwrap();
    let log = String::from_utf8_lossy(r.bytes().unwrap()).into_owned();
    assert_eq!(linked, 1, "{log}");
    (gl, memory)
}

/// The 8×8 colour buffer as RGBA, lowest row first.
fn pixels(gl: &Gl, memory: &mut Memory) -> Vec<[u8; 4]> {
    // Six words, the address, then the row's bytes.
    let mut c = Calls::new();
    let mut p: Vec<u8> = [0u32, 0, 8, 8, RGBA, UNSIGNED_BYTE].iter().flat_map(|w| w.to_le_bytes()).collect();
    records::put_u64(&mut p, 4096);
    records::put_u32(&mut p, 32);
    c.raw(wire::READ_PIXELS, &p);
    calls(gl, memory, &c);
    memory.0[4096..4096 + 256].as_chunks::<4>().0.to_vec()
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 1)
}

#[test]
fn a_program_draws_from_a_buffer_with_uniforms_and_a_generic_attribute() {
    let vs = "#version 330 core
layout(location = 0) in vec2 pos;
in vec3 color;
out vec3 c;
uniform float scale;
void main() { c = color; gl_Position = vec4(pos * scale, 0.0, 1.0); }";
    let fs = "#version 330 core
in vec3 c;
out vec4 o;
uniform vec4 tint;
void main() { o = vec4(c, 1.0) * tint; }";
    let (gl, mut memory) = with_program(vs, fs);
    let mut c = Calls::new();
    c.wb(wire::ATTRIB_LOCATION, &[3], b"color").wb(wire::UNIFORM_LOCATION, &[3], b"scale").wb(wire::UNIFORM_LOCATION, &[3], b"tint");
    c.w(wire::GET_PROGRAM, &[3, 0x8b86]).w(wire::ACTIVE_UNIFORM, &[3, 1]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    let (color, scale, tint, count) = (r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap());
    assert_eq!((color, scale, tint, count), (1, 0, 1, 2));
    assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.bytes().unwrap()), (1, 0x8b52, &b"tint"[..]));
    // A quad over the whole target, two triangles; colour from the generic attribute.
    let quad = f32s(&[-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0]);
    let mut c = Calls::new();
    c.w(wire::BIND_BUFFER, &[0x8892, 10]);
    let mut p = Vec::new();
    records::put_u32(&mut p, 0x8892);
    records::put_u64(&mut p, quad.len() as u64);
    records::put_u32(&mut p, 0x88e4);
    records::put_u32(&mut p, 1);
    records::put_bytes(&mut p, &quad);
    c.raw(wire::BUFFER_DATA, &p);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 0).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w(wire::ATTRIB_VALUE, &[color, 0.2f32.to_bits(), 0.4f32.to_bits(), 0.6f32.to_bits(), 1.0f32.to_bits()]);
    c.floats(wire::UNIFORM, &[scale, wire::FLOAT, 1, 1, 1, 0], &[1.0]);
    c.floats(wire::UNIFORM, &[tint, wire::FLOAT, 4, 1, 1, 0], &[1.0, 0.5, 1.0, 1.0]);
    c.w(wire::DRAW_ARRAYS, &[4, 0, 6, 1]);
    c.w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    assert!(px.iter().all(|&p| close(p, [51, 51, 153, 255])), "{px:?}");
}

const FLAT_VS: &str = "#version 330 core
in vec3 pos;
in vec4 color;
out vec4 c;
void main() { c = color; gl_Position = vec4(pos, 1.0); }";
const FLAT_FS: &str = "#version 330 core
in vec4 c;
out vec4 o;
void main() { o = c; }";

/// Vertices in the program's own memory: x, y, z and an RGBA colour of bytes, each 16 bytes.
fn vertex(x: f32, y: f32, z: f32, rgba: [u8; 4]) -> Vec<u8> {
    [f32s(&[x, y, z]), rgba.to_vec()].concat()
}

#[test]
fn client_arrays_byte_indices_depth_culling_and_blending() {
    let (gl, mut memory) = with_program(FLAT_VS, FLAT_FS);
    let mut c = Calls::new();
    c.wb(wire::ATTRIB_LOCATION, &[3], b"pos").wb(wire::ATTRIB_LOCATION, &[3], b"color");
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    let (pos, color) = (r.u32().unwrap(), r.u32().unwrap());
    // A far green square over everything, and a near red one over the left half, drawn after it: indexed, from the
    // program's own arrays, counter-clockwise.
    let square = |x0: f32, x1: f32, z: f32, rgba: [u8; 4]| [vertex(x0, -1.0, z, rgba), vertex(x1, -1.0, z, rgba), vertex(x1, 1.0, z, rgba), vertex(x0, 1.0, z, rgba)].concat();
    let verts = [square(-1.0, 1.0, 0.5, [0, 255, 0, 255]), square(-1.0, 0.0, -0.5, [255, 0, 0, 255]), square(-1.0, 1.0, 0.9, [0, 0, 255, 255])].concat();
    memory.0[1024..1024 + verts.len()].copy_from_slice(&verts);
    let idx: Vec<u8> = vec![0, 1, 2, 0, 2, 3];
    memory.0[2048..2054].copy_from_slice(&idx);
    // The blue square, wound clockwise: culled as a back face.
    memory.0[2064..2070].copy_from_slice(&[8, 10, 9, 8, 11, 10]);
    let mut c = Calls::new();
    c.w64(wire::ATTRIB_POINTER, &[pos, 3, 0x1406, 0, 0, 16], 1024).w(wire::ENABLE_ATTRIB, &[pos, 1]);
    c.w64(wire::ATTRIB_POINTER, &[color, 4, UNSIGNED_BYTE, 1, 0, 16], 1036).w(wire::ENABLE_ATTRIB, &[color, 1]);
    c.w(wire::CLEAR, &[COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT]);
    c.w(wire::ENABLE, &[DEPTH_TEST]).w(wire::ENABLE, &[CULL_FACE]);
    c.w64(wire::DRAW_ELEMENTS, &[4, 6, UNSIGNED_BYTE, 1, 0], 2048);
    c.w64(wire::DRAW_ELEMENTS, &[4, 6, UNSIGNED_BYTE, 1, 4], 2048);
    c.w64(wire::DRAW_ELEMENTS, &[4, 6, UNSIGNED_BYTE, 1, 0], 2064);
    c.w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    for (i, &p) in px.iter().enumerate() {
        let want = if i % 8 < 4 { [255, 0, 0, 255] } else { [0, 255, 0, 255] };
        assert!(close(p, want), "pixel {i} is {p:?}, wanted {want:?}");
    }
    // Half-transparent white over all of it, blended.
    let white = square(-1.0, 1.0, 0.0, [255, 255, 255, 128]);
    memory.0[1024..1024 + white.len()].copy_from_slice(&white);
    let mut c = Calls::new();
    c.w(wire::DISABLE, &[DEPTH_TEST]).w(wire::ENABLE, &[BLEND]).w(wire::BLEND_FUNC, &[0x0302, 0x0303, 1, 0]);
    c.w64(wire::DRAW_ELEMENTS, &[4, 6, UNSIGNED_BYTE, 1, 0], 2048);
    calls(&gl, &mut memory, &c);
    let px = pixels(&gl, &mut memory);
    assert!(close(px[0], [255, 128, 128, 255]) && close(px[7], [128, 255, 128, 255]), "{:?} {:?}", px[0], px[7]);
}

#[test]
fn a_texture_from_rgb_bytes() {
    let vs = "#version 120
attribute vec2 pos;
varying vec2 uv;
void main() { uv = pos * 0.5 + 0.5; gl_Position = vec4(pos, 0.0, 1.0); }";
    let fs = "#version 120
uniform sampler2D tex;
varying vec2 uv;
void main() { gl_FragColor = texture2D(tex, uv); }";
    let (gl, mut memory) = with_program(vs, fs);
    let quad = f32s(&[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]);
    memory.0[1024..1024 + quad.len()].copy_from_slice(&quad);
    // 2×2 RGB texels, rows lowest first: red, green / blue, white.
    let texels = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let mut c = Calls::new();
    c.wb(wire::UNIFORM_LOCATION, &[3], b"tex");
    let a = calls(&gl, &mut memory, &c);
    let tex = Reader::new(&a).u32().unwrap();
    let mut c = Calls::new();
    c.w(wire::ACTIVE_TEXTURE, &[2]).w(wire::BIND_TEXTURE, &[0x0de1, 20]);
    c.wb(wire::TEX_IMAGE, &[0x0de1, 0, RGB, 2, 2, RGB, UNSIGNED_BYTE, 1], &texels);
    c.w(wire::TEX_PARAMETER, &[0x0de1, 0x2801, 0x2600, 0]).w(wire::TEX_PARAMETER, &[0x0de1, 0x2800, 0x2600, 0]);
    c.wb(wire::UNIFORM, &[tex, wire::INT, 1, 1, 1, 0], &2u32.to_le_bytes());
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    let want = |x: usize, y: usize| match (x / 4, y / 4) {
        (0, 0) => [255, 0, 0, 255],
        (1, 0) => [0, 255, 0, 255],
        (0, 1) => [0, 0, 255, 255],
        _ => [255, 255, 255, 255],
    };
    for y in 0..8 {
        for x in 0..8 {
            assert!(close(px[y * 8 + x], want(x, y)), "({x}, {y}) is {:?}", px[y * 8 + x]);
        }
    }
}

/// A context bound to an 8×8 drawable with depth, and no program.
fn bare() -> (Gl, Memory) {
    let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
    let gl = Gl::new(gpu);
    let mut memory = Memory(vec![0; 8192]);
    let mut c = Calls::default();
    c.raw(wire::CONTEXT, &[1u64.to_le_bytes(), 0u64.to_le_bytes()].concat());
    c.raw(wire::USE, &1u64.to_le_bytes());
    let mut b = Vec::new();
    records::put_u64(&mut b, 9);
    for w in [8u32, 8, 24, 8, 0] {
        records::put_u32(&mut b, w);
    }
    c.raw(wire::BIND, &b);
    calls(&gl, &mut memory, &c);
    (gl, memory)
}

#[test]
fn fixed_function_draws_with_matrices_a_texture_and_the_alpha_test() {
    let (gl, mut memory) = bare();
    // A quad over the whole target in pixel coordinates, with texture coordinates, from the program's own arrays.
    memory.0[1024..1056].copy_from_slice(&f32s(&[0.0, 0.0, 8.0, 0.0, 8.0, 8.0, 0.0, 8.0]));
    memory.0[1088..1120].copy_from_slice(&f32s(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]));
    // White texels, the lower right one transparent.
    let texels = [255u8, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 255, 255, 255];
    let mut c = Calls::new();
    c.w(wire::MATRIX_MODE, &[0x1701]).floats(wire::MATRIX, &[wire::MATRIX_ORTHO], &[0.0, 8.0, 0.0, 8.0, -1.0, 1.0]);
    c.w(wire::MATRIX_MODE, &[0x1700]).floats(wire::MATRIX, &[wire::MATRIX_IDENTITY], &[]);
    c.w(wire::BIND_TEXTURE, &[0x0de1, 20]).wb(wire::TEX_IMAGE, &[0x0de1, 0, RGBA, 2, 2, RGBA, UNSIGNED_BYTE, 1], &texels);
    c.w(wire::TEX_PARAMETER, &[0x0de1, 0x2801, 0x2600, 0]).w(wire::TEX_PARAMETER, &[0x0de1, 0x2800, 0x2600, 0]);
    c.w(wire::ENABLE, &[0x0de1]).w(wire::ENABLE, &[0x0bc0]).w(wire::ALPHA_FUNC, &[0x0204, 0.5f32.to_bits()]);
    // glVertexPointer, glTexCoordPointer on unit 0, and glColor: attributes 0, 8 and 3.
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w64(wire::ATTRIB_POINTER, &[8, 2, 0x1406, 0, 0, 0], 1088).w(wire::ENABLE_ATTRIB, &[8, 1]);
    c.w(wire::ATTRIB_VALUE, &[3, 1.0f32.to_bits(), 0, 0, 1.0f32.to_bits()]);
    c.w(wire::DRAW_ARRAYS, &[6, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    for y in 0..8 {
        for x in 0..8 {
            let want = if x >= 4 && y < 4 { [0, 0, 0, 255] } else { [255, 0, 0, 255] };
            assert!(close(px[y * 8 + x], want), "({x}, {y}) is {:?}, wanted {want:?}", px[y * 8 + x]);
        }
    }
    // Moved half off the target by the model-view matrix, without texturing: the left half green, the right half
    // as it was.
    let mut c = Calls::new();
    c.w(wire::DISABLE, &[0x0de1]).w(wire::DISABLE, &[0x0bc0]);
    c.floats(wire::MATRIX, &[wire::MATRIX_TRANSLATE], &[-4.0, 0.0, 0.0]);
    c.w(wire::ATTRIB_VALUE, &[3, 0, 1.0f32.to_bits(), 0, 1.0f32.to_bits()]);
    c.w(wire::DRAW_ARRAYS, &[6, 0, 4, 1]);
    calls(&gl, &mut memory, &c);
    let px = pixels(&gl, &mut memory);
    assert!(close(px[0], [0, 255, 0, 255]) && close(px[7], [0, 0, 0, 255]) && close(px[63], [255, 0, 0, 255]), "{:?} {:?} {:?}", px[0], px[7], px[63]);
}

#[test]
fn a_glsl_120_program_reads_the_matrices() {
    let vs = "#version 120
void main() { gl_FrontColor = gl_Color; gl_Position = ftransform(); }";
    let fs = "#version 120
void main() { gl_FragColor = gl_Color; }";
    let (gl, mut memory) = with_program(vs, fs);
    memory.0[1024..1056].copy_from_slice(&f32s(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]));
    // A unit square scaled to the target's left half by the model-view matrix, in [0, 1] by the projection.
    let mut c = Calls::new();
    c.w(wire::MATRIX_MODE, &[0x1701]).floats(wire::MATRIX, &[wire::MATRIX_ORTHO], &[0.0, 1.0, 0.0, 1.0, -1.0, 1.0]);
    c.w(wire::MATRIX_MODE, &[0x1700]).floats(wire::MATRIX, &[wire::MATRIX_SCALE], &[0.5, 1.0, 1.0]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w(wire::ATTRIB_VALUE, &[3, 0, 0, 1.0f32.to_bits(), 1.0f32.to_bits()]);
    c.w(wire::DRAW_ARRAYS, &[6, 0, 4, 1]).w(wire::GET_ERROR, &[]).wb(wire::UNIFORM_LOCATION, &[3], b"gl_ModelViewMatrix");
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    assert_eq!((r.u32().unwrap(), r.u32().unwrap() as i32), (NO_ERROR, -1));
    let px = pixels(&gl, &mut memory);
    assert!(close(px[0], [0, 0, 255, 255]) && close(px[3], [0, 0, 255, 255]) && close(px[4], [0, 0, 0, 255]), "{:?} {:?} {:?}", px[0], px[3], px[4]);
}

#[test]
fn a_framebuffer_object_of_a_texture_and_a_depth_renderbuffer_is_drawn_into_and_sampled() {
    let vs = "#version 120
attribute vec2 pos;
varying vec2 uv;
uniform float z;
void main() { uv = pos * 0.5 + 0.5; gl_Position = vec4(pos, z, 1.0); }";
    let fs = "#version 120
uniform sampler2D tex;
uniform vec4 tint;
uniform float sampled;
varying vec2 uv;
void main() { gl_FragColor = mix(tint, texture2D(tex, uv), sampled); }";
    let (gl, mut memory) = with_program(vs, fs);
    // The left half of the target, and the whole of it.
    memory.0[1024..1056].copy_from_slice(&f32s(&[-1.0, -1.0, 0.0, -1.0, -1.0, 1.0, 0.0, 1.0]));
    memory.0[1088..1120].copy_from_slice(&f32s(&[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]));
    let mut c = Calls::new();
    c.wb(wire::UNIFORM_LOCATION, &[3], b"tex").wb(wire::UNIFORM_LOCATION, &[3], b"tint").wb(wire::UNIFORM_LOCATION, &[3], b"sampled").wb(wire::UNIFORM_LOCATION, &[3], b"z");
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    let (tex, tint, sampled, z) = (r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap());
    // A 4×4 texture with no texels, and a depth renderbuffer, attached to framebuffer 30.
    let mut c = Calls::new();
    c.w(wire::BIND_TEXTURE, &[0x0de1, 20]).wb(wire::TEX_IMAGE, &[0x0de1, 0, RGBA, 4, 4, RGBA, UNSIGNED_BYTE, 0], &[]);
    c.w(wire::TEX_PARAMETER, &[0x0de1, 0x2801, 0x2600, 0]).w(wire::TEX_PARAMETER, &[0x0de1, 0x2800, 0x2600, 0]);
    c.w(wire::BIND_FRAMEBUFFER, &[0x8d40, 30]).w(wire::CHECK_FRAMEBUFFER, &[0x8d40]);
    c.w(wire::FRAMEBUFFER_TEXTURE, &[0x8d40, 0x8ce0, 0x0de1, 20, 0]);
    c.w(wire::BIND_RENDERBUFFER, &[0x8d41, 40]).w(wire::RENDERBUFFER_STORAGE, &[0x8d41, 0x81a6, 4, 4]);
    c.w(wire::FRAMEBUFFER_RENDERBUFFER, &[0x8d40, 0x8d00, 0x8d41, 40]).w(wire::CHECK_FRAMEBUFFER, &[0x8d40]);
    c.w(wire::GET, &[0x8ca6]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    assert_eq!((r.u32().unwrap(), r.u32().unwrap()), (0x8cd7, 0x8cd5), "incomplete with nothing attached, then complete");
    assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.u64().unwrap()), (1, INT, 30));
    // Into it, at its own size: cleared blue, then green on its left half, near; then red over all of it, farther,
    // which the depth test keeps to the right half.
    let mut c = Calls::new();
    c.w(wire::VIEWPORT, &[0, 0, 4, 4]).w(wire::CLEAR_COLOR, &[0, 0, 1.0f32.to_bits(), 1.0f32.to_bits()]);
    c.w(wire::CLEAR, &[COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT]).w(wire::ENABLE, &[DEPTH_TEST]);
    c.floats(wire::UNIFORM, &[sampled, wire::FLOAT, 1, 1, 1, 0], &[0.0]);
    c.floats(wire::UNIFORM, &[tint, wire::FLOAT, 4, 1, 1, 0], &[0.0, 1.0, 0.0, 1.0]).floats(wire::UNIFORM, &[z, wire::FLOAT, 1, 1, 1, 0], &[-0.5]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]).w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]);
    c.floats(wire::UNIFORM, &[tint, wire::FLOAT, 4, 1, 1, 0], &[1.0, 0.0, 0.0, 1.0]).floats(wire::UNIFORM, &[z, wire::FLOAT, 1, 1, 1, 0], &[0.5]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1088).w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]);
    // Then the drawable, with the texture over all of it.
    c.w(wire::BIND_FRAMEBUFFER, &[0x8d40, 0]).w(wire::VIEWPORT, &[0, 0, 8, 8]).w(wire::DISABLE, &[DEPTH_TEST]);
    c.wb(wire::UNIFORM, &[tex, wire::INT, 1, 1, 1, 0], &0u32.to_le_bytes()).floats(wire::UNIFORM, &[sampled, wire::FLOAT, 1, 1, 1, 0], &[1.0]);
    c.w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    for y in 0..8 {
        for x in 0..8 {
            let want = if x < 4 { [0, 255, 0, 255] } else { [255, 0, 0, 255] };
            assert!(close(px[y * 8 + x], want), "({x}, {y}) is {:?}, wanted {want:?}", px[y * 8 + x]);
        }
    }
}

#[test]
fn lines_are_a_pixel_wide_and_never_culled() {
    let (gl, mut memory) = bare();
    // Pixel coordinates: a horizontal line through row 2's centres, and a line strip down column 5's, closed back
    // to its start as a loop.
    memory.0[1024..1040].copy_from_slice(&f32s(&[0.0, 2.5, 8.0, 2.5]));
    memory.0[1088..1112].copy_from_slice(&f32s(&[5.5, 4.0, 5.5, 8.0, 5.5, 6.0]));
    let mut c = Calls::new();
    c.w(wire::MATRIX_MODE, &[0x1701]).floats(wire::MATRIX, &[wire::MATRIX_ORTHO], &[0.0, 8.0, 0.0, 8.0, -1.0, 1.0]);
    c.w(wire::ENABLE, &[CULL_FACE]).w(wire::CULL_FACE, &[0x0408]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w(wire::ATTRIB_VALUE, &[3, 1.0f32.to_bits(), 1.0f32.to_bits(), 1.0f32.to_bits(), 1.0f32.to_bits()]);
    c.w(wire::DRAW_ARRAYS, &[1, 0, 2, 1]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1088).w(wire::DRAW_ARRAYS, &[2, 0, 3, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    for y in 0..8 {
        for x in 0..8 {
            let lit = y == 2 || (x == 5 && y >= 4);
            let want = if lit { [255, 255, 255, 255] } else { [0, 0, 0, 255] };
            assert!(close(px[y * 8 + x], want), "({x}, {y}) is {:?}, wanted {want:?}", px[y * 8 + x]);
        }
    }
}

#[test]
fn the_alpha_test_applies_to_a_programs_own_colour() {
    let vs = "#version 120
attribute vec2 pos;
varying float a;
void main() { a = pos.x * 0.5 + 0.5; gl_Position = vec4(pos, 0.0, 1.0); }";
    let fs = "#version 120
varying float a;
void main() { gl_FragColor = vec4(1.0, 1.0, 0.0, a); }";
    let (gl, mut memory) = with_program(vs, fs);
    memory.0[1024..1056].copy_from_slice(&f32s(&[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]));
    // Alpha rises left to right: GL_GREATER than a half keeps the right half.
    let mut c = Calls::new();
    c.w(wire::ENABLE, &[0x0bc0]).w(wire::ALPHA_FUNC, &[0x0204, 0.5f32.to_bits()]);
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    c.w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    for x in 0..8 {
        let want = if x >= 4 { [255, 255, 0, 255] } else { [0, 0, 0, 255] };
        assert!(close(px[3 * 8 + x], want), "({x}, 3) is {:?}, wanted {want:?}", px[3 * 8 + x]);
    }
}

/// A full-target quad, as a triangle strip, in the program's memory at 1024: attribute 0, two floats.
fn quad_at_1024(gl: &Gl, memory: &mut Memory) {
    memory.0[1024..1056].copy_from_slice(&f32s(&[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]));
    let mut c = Calls::new();
    c.w64(wire::ATTRIB_POINTER, &[0, 2, 0x1406, 0, 0, 0], 1024).w(wire::ENABLE_ATTRIB, &[0, 1]);
    calls(gl, memory, &c);
}

const QUAD_VS: &str = "#version 330 core
layout(location = 0) in vec2 pos;
void main() { gl_Position = vec4(pos, 0.0, 1.0); }";

#[test]
fn two_draw_buffers_take_two_outputs_and_clear_apart() {
    let fs = "#version 330 core
layout(location = 0) out vec4 a;
layout(location = 1) out vec4 b;
void main() { a = vec4(1.0, 0.0, 0.0, 1.0); b = vec4(0.0, 1.0, 0.0, 1.0); }";
    let (gl, mut memory) = with_program(QUAD_VS, fs);
    quad_at_1024(&gl, &mut memory);
    // Two 8×8 textures as colour attachments 0 and 1 of framebuffer 30, both drawn into.
    let mut c = Calls::new();
    for (t, unit) in [(20, 0), (21, 1)] {
        c.w(wire::ACTIVE_TEXTURE, &[unit]).w(wire::BIND_TEXTURE, &[0x0de1, t]).wb(wire::TEX_IMAGE, &[0x0de1, 0, RGBA, 8, 8, RGBA, UNSIGNED_BYTE, 0], &[]);
    }
    c.w(wire::BIND_FRAMEBUFFER, &[0x8d40, 30]).w(wire::FRAMEBUFFER_TEXTURE, &[0x8d40, 0x8ce0, 0x0de1, 20, 0]).w(wire::FRAMEBUFFER_TEXTURE, &[0x8d40, 0x8ce1, 0x0de1, 21, 0]);
    c.w(wire::DRAW_BUFFERS, &[2, 0x8ce0, 0x8ce1]).w(wire::CHECK_FRAMEBUFFER, &[0x8d40]).w(wire::GET, &[0x8824]);
    c.w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    assert_eq!(r.u32().unwrap(), 0x8cd5);
    assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.u64().unwrap()), (1, INT, 4), "four draw buffers");
    assert_eq!(r.u32().unwrap(), NO_ERROR);
    let read = |gl: &Gl, memory: &mut Memory, attachment: u32| {
        let mut c = Calls::new();
        c.w(wire::READ_COLOR, &[attachment]);
        calls(gl, memory, &c);
        pixels(gl, memory)
    };
    assert!(read(&gl, &mut memory, 0x8ce0).iter().all(|&p| close(p, [255, 0, 0, 255])), "attachment 0 is output 0's");
    assert!(read(&gl, &mut memory, 0x8ce1).iter().all(|&p| close(p, [0, 255, 0, 255])), "attachment 1 is output 1's");
    // Draw buffer 1 cleared blue alone: attachment 0 keeps its red.
    let mut c = Calls::new();
    c.w(wire::CLEAR_BUFFER, &[0x1800, 1, 0, 0, 1.0f32.to_bits(), 1.0f32.to_bits()]);
    calls(&gl, &mut memory, &c);
    assert!(read(&gl, &mut memory, 0x8ce1).iter().all(|&p| close(p, [0, 0, 255, 255])));
    assert!(read(&gl, &mut memory, 0x8ce0).iter().all(|&p| close(p, [255, 0, 0, 255])));
}

#[test]
fn a_cube_map_is_sampled_by_direction() {
    let fs = "#version 330 core
uniform samplerCube sky;
uniform vec3 dir;
out vec4 o;
void main() { o = texture(sky, dir); }";
    let (gl, mut memory) = with_program(QUAD_VS, fs);
    quad_at_1024(&gl, &mut memory);
    let mut c = Calls::new();
    c.wb(wire::UNIFORM_LOCATION, &[3], b"sky").wb(wire::UNIFORM_LOCATION, &[3], b"dir");
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    let (sky, dir) = (r.u32().unwrap(), r.u32().unwrap());
    // Each face 2×2 of one colour: +X red, -X green, +Y blue, -Y white, +Z yellow, -Z cyan.
    let colours: [[u8; 4]; 6] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255], [255, 255, 0, 255], [0, 255, 255, 255]];
    let mut c = Calls::new();
    c.w(wire::ACTIVE_TEXTURE, &[3]).w(wire::BIND_TEXTURE, &[0x8513, 40]);
    for (face, colour) in colours.iter().enumerate() {
        c.wb(wire::TEX_IMAGE, &[0x8515 + face as u32, 0, RGBA, 2, 2, RGBA, UNSIGNED_BYTE, 1], &colour.repeat(4));
    }
    c.w(wire::TEX_PARAMETER, &[0x8513, 0x2801, 0x2601, 0]).w(wire::TEX_PARAMETER, &[0x8513, 0x2800, 0x2601, 0]);
    c.wb(wire::UNIFORM, &[sky, wire::INT, 1, 1, 1, 0], &3u32.to_le_bytes()).w(wire::GET, &[0x8514]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.u64().unwrap()), (1, INT, 40));
    for (d, want) in [([1.0f32, 0.1, -0.2], colours[0]), ([-1.0, 0.3, 0.2], colours[1]), ([0.1, 1.0, 0.1], colours[2]), ([0.2, -1.0, -0.3], colours[3]), ([0.1, 0.2, 1.0], colours[4]), ([-0.1, 0.1, -1.0], colours[5])] {
        let mut c = Calls::new();
        c.floats(wire::UNIFORM, &[dir, wire::FLOAT, 3, 1, 1, 0], &d).w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]);
        calls(&gl, &mut memory, &c);
        let px = pixels(&gl, &mut memory);
        assert!(px.iter().all(|&p| close(p, want)), "{d:?} reads {:?}, wanted {want:?}", px[0]);
    }
}

#[test]
fn texel_fetch_and_texture_size_read_exact_texels() {
    let fs = "#version 330 core
uniform sampler2D tex;
out vec4 o;
void main() {
    ivec2 size = textureSize(tex, 0);
    vec4 t = texelFetch(tex, ivec2(2, 1), 0);
    o = (size == ivec2(4, 2)) ? t : vec4(1.0, 0.0, 1.0, 1.0);
}";
    let (gl, mut memory) = with_program(QUAD_VS, fs);
    quad_at_1024(&gl, &mut memory);
    // 4×2 texels, each its own colour, linear filtering (a fetch is exact whatever the filter).
    let texels: Vec<u8> = (0..8u8).flat_map(|i| [i * 30, 255 - i * 30, i * 10, 255]).collect();
    let mut c = Calls::new();
    c.wb(wire::UNIFORM_LOCATION, &[3], b"tex");
    let a = calls(&gl, &mut memory, &c);
    let tex = Reader::new(&a).u32().unwrap();
    let mut c = Calls::new();
    c.w(wire::BIND_TEXTURE, &[0x0de1, 20]).wb(wire::TEX_IMAGE, &[0x0de1, 0, RGBA, 4, 2, RGBA, UNSIGNED_BYTE, 1], &texels);
    c.w(wire::TEX_PARAMETER, &[0x0de1, 0x2801, 0x2601, 0]).w(wire::TEX_PARAMETER, &[0x0de1, 0x2800, 0x2601, 0]);
    c.wb(wire::UNIFORM, &[tex, wire::INT, 1, 1, 1, 0], &0u32.to_le_bytes()).w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]);
    calls(&gl, &mut memory, &c);
    // Texel (2, 1) is the seventh: row 1's third.
    let want = [6 * 30, 255 - 6 * 30, 60, 255];
    let px = pixels(&gl, &mut memory);
    assert!(px.iter().all(|&p| close(p, want)), "{:?}, wanted {want:?}", px[0]);
}

#[test]
fn a_uniform_block_reads_the_buffer_at_its_binding_point() {
    let fs = "#version 330 core
layout(std140) uniform Look { vec4 unused; vec4 tint; };
out vec4 o;
void main() { o = tint; }";
    let (gl, mut memory) = with_program(QUAD_VS, fs);
    quad_at_1024(&gl, &mut memory);
    let mut c = Calls::new();
    c.wb(wire::UNIFORM_BLOCK_INDEX, &[3], b"Look").wb(wire::UNIFORM_BLOCK_INDEX, &[3], b"Nope").w(wire::ACTIVE_UNIFORM_BLOCK, &[3, 0, 0x8a40]).w(wire::GET_PROGRAM, &[3, 0x8a36]);
    let a = calls(&gl, &mut memory, &c);
    let mut r = Reader::new(&a);
    assert_eq!((r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap()), (0, u32::MAX, 32, 1), "index, none, size, count");
    // The block's data at offset 256 of buffer 50, bound at point 2 by a range.
    let mut data = vec![0u8; 512];
    data[256 + 16..256 + 32].copy_from_slice(&f32s(&[0.0, 0.5, 1.0, 1.0]));
    let mut c = Calls::new();
    c.w(wire::BIND_BUFFER, &[0x8a11, 50]);
    let mut p = Vec::new();
    records::put_u32(&mut p, 0x8a11);
    records::put_u64(&mut p, data.len() as u64);
    records::put_u32(&mut p, 0x88e4);
    records::put_u32(&mut p, 1);
    records::put_bytes(&mut p, &data);
    c.raw(wire::BUFFER_DATA, &p);
    let mut p = Vec::new();
    for w in [0x8a11u32, 2, 50] {
        records::put_u32(&mut p, w);
    }
    records::put_u64(&mut p, 256);
    records::put_u64(&mut p, 256);
    c.raw(wire::BIND_BUFFER_RANGE, &p);
    c.w(wire::UNIFORM_BLOCK_BINDING, &[3, 0, 2]).w(wire::DRAW_ARRAYS, &[5, 0, 4, 1]).w(wire::GET_ERROR, &[]);
    let a = calls(&gl, &mut memory, &c);
    assert_eq!(Reader::new(&a).u32().unwrap(), NO_ERROR);
    let px = pixels(&gl, &mut memory);
    assert!(px.iter().all(|&p| close(p, [0, 128, 255, 255])), "{:?}", px[0]);
}
