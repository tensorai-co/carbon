//! Carbon's GLSL compiler held to glslang's: each program in `tests/glsl/` compiled both ways (ours from the source,
//! glslang's SPIR-V through gpu's SPIR-V frontend), run on the same input on the CPU device and on the host's GPU, and
//! every word of output compared (floats to a few ULPs). The built-ins the SPIR-V frontend can't lower are checked
//! against Rust's own functions instead.

use gpu::ir::Entry;
use gpu::{Choice, Command, Device, Range, Submit};
use gpu_frontend::glsl::{self, Stage};
use std::time::Duration;

/// Runs a compute entry over `groups` workgroups with binding 0 the input and binding 1 the output (zeroed).
fn run(d: &mut Device, entry: &Entry, input: &[u8], output: usize, groups: u32) -> Vec<u8> {
    let p = d.pipeline(entry).unwrap_or_else(|e| panic!("{}: {}", entry.name, e.message));
    let inp = d.alloc(input.len() as u64).unwrap();
    d.write(inp, 0, input).unwrap();
    let out = d.alloc(output as u64).unwrap();
    d.write(out, 0, &vec![0; output]).unwrap();
    let bindings = entry
        .bindings
        .iter()
        .map(|b| match b.binding {
            0 => Range { memory: inp, offset: 0, size: input.len() as u64 },
            _ => Range { memory: out, offset: 0, size: output as u64 },
        })
        .collect();
    let fence = d.fence().unwrap();
    d.submit(Submit { commands: &[Command::Dispatch { pipeline: p, bindings, groups: [groups, 1, 1] }], wait: &[], signal: &[(fence, 1)] }).unwrap();
    assert!(d.wait(fence, 1, Duration::from_secs(60)).unwrap());
    let mut bytes = vec![0; output];
    d.read(out, 0, &mut bytes).unwrap();
    d.free(inp);
    d.free(out);
    bytes
}

fn ours(source: &str) -> Entry {
    match glsl::compile(source, Stage::Compute, false) {
        Ok(s) => s.entry,
        Err(e) => panic!("{e}"),
    }
}

fn theirs(spirv: &[u8]) -> Entry {
    gpu::spirv(spirv).unwrap().entries.remove(0)
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect()
}

/// Deterministic inputs: integers in [-200, 200], or floats in [-4, 4].
fn inputs(n: usize, float: bool) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    let mut out = Vec::new();
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let w = if float { ((x % 8001) as f32 / 1000.0 - 4.0).to_bits() } else { ((x % 401) as i32 - 200) as u32 };
        out.extend(w.to_le_bytes());
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn compare(name: &str, choice: Choice, source: &str, spirv: &[u8], float: bool, n_in: usize, out_words: usize, groups: u32) {
    let Ok(Some(mut d)) = Device::open(choice) else {
        eprintln!("{name}: no {choice:?} device here; skipped");
        return;
    };
    let input = inputs(n_in, float);
    let a = words(&run(&mut d, &ours(source), &input, out_words * 4, groups));
    let b = words(&run(&mut d, &theirs(spirv), &input, out_words * 4, groups));
    let mut bad = 0;
    for (k, (x, y)) in a.iter().zip(&b).enumerate() {
        let same = if float {
            let (x, y) = (f32::from_bits(*x), f32::from_bits(*y));
            x == y || (x - y).abs() <= 1e-5 * y.abs().max(1.0) || (x.is_nan() && y.is_nan())
        } else {
            x == y
        };
        if !same {
            if bad < 12 {
                if float {
                    eprintln!("{name} on {choice:?}: word {k} (invocation {}, value {}) is {} from GLSL, {} from glslang", k / (out_words / 64).max(1), k % (out_words / 64).max(1), f32::from_bits(*x), f32::from_bits(*y));
                } else {
                    eprintln!("{name} on {choice:?}: word {k} is {x:#x} ({}) from GLSL, {y:#x} ({}) from glslang", *x as i32, *y as i32);
                }
            }
            bad += 1;
        }
    }
    assert_eq!(bad, 0, "{name} on {choice:?}: {bad} of {} words differ", a.len());
}

const INTS: &str = include_str!("glsl/ints.comp");
const FLOATS: &str = include_str!("glsl/floats.comp");
const SHARED: &str = include_str!("glsl/shared.comp");

fn all(choice: Choice) {
    compare("ints", choice, INTS, include_bytes!("glsl/ints.comp.spv"), false, 64, 64 * 16, 1);
    compare("floats", choice, FLOATS, include_bytes!("glsl/floats.comp.spv"), true, 64, 64 * 32, 1);
    compare("shared", choice, SHARED, include_bytes!("glsl/shared.comp.spv"), false, 256, 1 + 4, 4);
}

#[test]
fn glsl_matches_glslang_on_the_cpu_device() {
    all(Choice::Software);
}

#[test]
fn glsl_matches_glslang_on_the_host_gpu() {
    all(Choice::Host);
}

/// The built-ins only Carbon's compiler lowers, against Rust's: each invocation's input x in [-0.9, 0.9].
const VALUES: &str = "#version 430
layout(local_size_x = 64) in;
layout(std430, binding = 0) readonly buffer In { float data[]; };
layout(std430, binding = 1) buffer Out { float values[]; };
void main() {
    uint i = gl_GlobalInvocationID.x;
    float x = data[i] * 0.225;
    float y = data[(i + 7u) % 64u];
    uint b = i * 16u;
    values[b + 0u] = asin(x);
    values[b + 1u] = acos(x);
    values[b + 2u] = atan(x * 5.0);
    values[b + 3u] = atan(y, x);
    values[b + 4u] = sinh(x) + cosh(x) * 10.0;
    values[b + 5u] = tanh(y);
    values[b + 6u] = asinh(y) + acosh(abs(y) + 1.0) * 10.0;
    values[b + 7u] = atanh(x);
    mat3 m = mat3(2.0, x, 0.5, y, 3.0, 1.0, 0.25, x * y, 4.0);
    values[b + 8u] = determinant(m);
    mat3 inv = inverse(m) * m;
    values[b + 9u] = inv[0][0] + inv[1][1] * 10.0 + inv[2][2] * 100.0 + inv[1][0] * 1000.0;
    mat2 m2 = inverse(mat2(3.0, x, y, 2.0));
    values[b + 10u] = m2[0][0] + m2[1][0] * 10.0;
    vec3 r = refract(normalize(vec3(x, -1.0, 0.0)), vec3(0.0, 1.0, 0.0), 0.75);
    values[b + 11u] = r.x + r.y * 10.0;
    float whole;
    values[b + 12u] = modf(y * 2.5, whole) + whole * 10.0;
    values[b + 13u] = float(isnan(log2(x))) + float(isinf(1.0 / (x - x))) * 10.0;
    mat4 m4 = mat4(4.0, x, 0.0, 1.0, y, 3.0, 0.5, 0.0, 0.0, 1.0, 2.0, x, 1.0, 0.0, y, 5.0);
    values[b + 14u] = determinant(m4);
    mat4 id = inverse(m4) * m4;
    values[b + 15u] = id[0][0] + id[3][3] + id[1][2] * 100.0 + id[2][1] * 1000.0;
}";

fn det3(m: [[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2]) + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2])
}

fn det4(m: [[f64; 4]; 4]) -> f64 {
    (0..4)
        .map(|r| {
            let mut sub = [[0.0; 3]; 3];
            for (ci, c) in (1..4).enumerate() {
                for (ri, rr) in (0..4).filter(|&k| k != r).enumerate() {
                    sub[ci][ri] = m[c][rr];
                }
            }
            let sign = if r % 2 == 0 { 1.0 } else { -1.0 };
            sign * m[0][r] * det3(sub)
        })
        .sum()
}

fn values(choice: Choice) {
    let Ok(Some(mut d)) = Device::open(choice) else { return };
    let input = inputs(64, true);
    let got = words(&run(&mut d, &ours(VALUES), &input, 64 * 16 * 4, 1));
    let data: Vec<f64> = words(&input).iter().map(|w| f64::from(f32::from_bits(*w))).collect();
    let mut bad = 0;
    for i in 0..64 {
        let x = f64::from((data[i] as f32) * 0.225);
        let y = data[(i + 7) % 64];
        let m = [[2.0, x, 0.5], [y, 3.0, 1.0], [0.25, x * y, 4.0]];
        let m4 = [[4.0, x, 0.0, 1.0], [y, 3.0, 0.5, 0.0], [0.0, 1.0, 2.0, x], [1.0, 0.0, y, 5.0]];
        let d2 = 6.0 - x * y;
        let n = {
            let l = (x * x + 1.0).sqrt();
            (x / l, -1.0 / l)
        };
        let k = 1.0 - 0.75f64 * 0.75 * (1.0 - n.1 * n.1);
        let r = if k < 0.0 { (0.0, 0.0) } else { (0.75 * n.0, 0.75 * n.1 - (0.75 * n.1 + k.sqrt())) };
        let whole = (y as f32 * 2.5) as f64;
        let want = [
            x.asin(),
            x.acos(),
            (x * 5.0).atan(),
            y.atan2(x),
            x.sinh() + x.cosh() * 10.0,
            y.tanh(),
            y.asinh() + (y.abs() + 1.0).acosh() * 10.0,
            x.atanh(),
            det3(m),
            111.0,
            2.0 / d2 + (-y / d2) * 10.0,
            r.0 + r.1 * 10.0,
            (y as f32 * 2.5).fract() as f64 + whole.trunc() * 10.0,
            if x < 0.0 { 1.0 } else { 0.0 } + 10.0,
            det4(m4),
            2.0,
        ];
        for (j, w) in want.iter().enumerate() {
            let g = f64::from(f32::from_bits(got[i * 16 + j]));
            if (g - w).abs() > 2e-4 * w.abs().max(1.0) {
                if bad < 12 {
                    eprintln!("values on {choice:?}: invocation {i}, value {j}: {g} from GLSL, {w} wanted (x = {x}, y = {y})");
                }
                bad += 1;
            }
        }
    }
    assert_eq!(bad, 0, "{bad} values differ on {choice:?}");
}

#[test]
fn builtins_glslang_cant_check_match_rust_on_the_cpu_device() {
    values(Choice::Software);
}

#[test]
fn builtins_glslang_cant_check_match_rust_on_the_host_gpu() {
    values(Choice::Host);
}

/// Control flow glslang's SPIR-V can't be checked on (returns from nested loops, `continue` inside `switch`), against
/// the same logic in Rust.
const CONTROL: &str = "#version 430
layout(local_size_x = 64) in;
layout(std430, binding = 0) readonly buffer In { int data[]; };
layout(std430, binding = 1) buffer Out { int words[]; };
int search(int x) {
    for (int i = 0; i < 6; i++) {
        for (int j = 0; j < 6; j++) {
            if (i * j == x) return i * 10 + j;
            if (j > i) break;
        }
    }
    return -1;
}
int walk(int x) {
    int s = 0;
    for (int i = 0; i < 12; i++) {
        switch ((x + i) % 4) {
            case 0: continue;
            case 1: s += i;
            case 2: s += 1; break;
            default:
                if (i > 9) return s * 2;
                s -= 1;
        }
        s += 100;
    }
    return s;
}
int nested(int x) {
    int r = 0;
    while (true) {
        r += 1;
        if (r > 3) {
            do {
                if (x > 0) return r * 1000 + x;
                x += 2;
            } while (x < 50);
            return -r;
        }
    }
}
void main() {
    uint i = gl_GlobalInvocationID.x;
    int x = abs(data[i]) % 40;
    words[i * 3u] = search(x);
    words[i * 3u + 1u] = walk(x);
    words[i * 3u + 2u] = nested(data[i]);
}";

fn search(x: i32) -> i32 {
    for i in 0..6 {
        for j in 0..6 {
            if i * j == x {
                return i * 10 + j;
            }
            if j > i {
                break;
            }
        }
    }
    -1
}

fn walk(x: i32) -> i32 {
    let mut s = 0;
    for i in 0..12 {
        match (x + i) % 4 {
            0 => continue,
            1 => s += i + 1,
            2 => s += 1,
            _ => {
                if i > 9 {
                    return s * 2;
                }
                s -= 1;
            }
        }
        s += 100;
    }
    s
}

fn nested(mut x: i32) -> i32 {
    let r = 4;
    loop {
        if x > 0 {
            return r * 1000 + x;
        }
        x += 2;
        if x >= 50 {
            return -r;
        }
    }
}

fn control(choice: Choice) {
    let Ok(Some(mut d)) = Device::open(choice) else { return };
    let input = inputs(64, false);
    let got: Vec<i32> = words(&run(&mut d, &ours(CONTROL), &input, 64 * 3 * 4, 1)).iter().map(|&w| w as i32).collect();
    let data: Vec<i32> = words(&input).iter().map(|&w| w as i32).collect();
    for (i, &v) in data.iter().enumerate() {
        let x = v.abs() % 40;
        assert_eq!(got[i * 3..i * 3 + 3], [search(x), walk(x), nested(v)], "invocation {i}, input {v}, on {choice:?}");
    }
}

#[test]
fn control_flow_matches_rust_on_the_cpu_device() {
    control(Choice::Software);
}

#[test]
fn control_flow_matches_rust_on_the_host_gpu() {
    control(Choice::Host);
}

/// A GLSL 1.10 program drawn: attributes, plain uniforms in both stages (each stage's default block), two varyings
/// interpolated, and `texture2D` of a 2×2 texture, on an 8×8 target. A quad scaled and moved by its uniforms covers
/// the middle rows of the right half: each pixel there is its texel times the tint, with alpha the varying shade.
const DRAW_VS: &str = "
attribute vec2 pos;
attribute vec2 uv;
uniform vec2 offset;
uniform float scale;
varying vec2 v_uv;
varying float v_shade;
void main() {
    v_uv = uv;
    v_shade = pos.x * 0.5 + 0.5;
    gl_Position = vec4(pos * scale + offset, 0.0, 1.0);
}";

const DRAW_FS: &str = "
uniform sampler2D tex;
uniform vec4 tint;
varying vec2 v_uv;
varying float v_shade;
void main() {
    vec4 t = texture2D(tex, v_uv);
    gl_FragColor = vec4(t.rgb * tint.rgb, v_shade);
}";

fn draw(choice: Choice) {
    use gpu::{Address, Attribute, Bind, ColorAttachment, ColorTarget, Cull, Draw, Filter, Format, FrontFace, GraphicsDesc, ImageDesc, Load, Rect, SamplerDesc, Sub, Topology, VertexBuffer, VertexFormat, ir::Slot};
    let Ok(Some(mut d)) = Device::open(choice) else { return };
    let v = glsl::compile(DRAW_VS, Stage::Vertex, false).unwrap();
    let f = glsl::compile(DRAW_FS, Stage::Fragment, false).unwrap();
    let p = glsl::link(&v, &f).unwrap();
    let wait = |d: &mut Device, commands: &[Command]| {
        let fence = d.fence().unwrap();
        d.submit(Submit { commands, wait: &[], signal: &[(fence, 1)] }).unwrap();
        assert!(d.wait(fence, 1, Duration::from_secs(60)).unwrap());
    };
    let buffer = |d: &mut Device, bytes: &[u8]| {
        let m = d.alloc(bytes.len().max(16) as u64).unwrap();
        d.write(m, 0, bytes).unwrap();
        Range { memory: m, offset: 0, size: bytes.len().max(16) as u64 }
    };
    let f32s = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    // The texture: red, green / blue, white.
    let tex = d.image(ImageDesc::new(Format::Rgba8Unorm, 2, 2)).unwrap();
    let texels = [255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let staging = buffer(&mut d, &texels);
    wait(&mut d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 2, height: 2 } }]);
    let sampler = d.sampler(SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToEdge; 2])).unwrap();
    // Each stage's uniforms where its interface says they are.
    let uniforms = |s: &glsl::Shader, values: &[(&str, &[f32])]| {
        let mut bytes = vec![0u8; s.interface.default_block.unwrap().1 as usize];
        for (name, v) in values {
            let u = s.interface.uniforms.iter().find(|u| u.name == *name).unwrap();
            bytes[u.offset as usize..u.offset as usize + v.len() * 4].copy_from_slice(&f32s(v));
        }
        bytes
    };
    let vs_block = buffer(&mut d, &uniforms(&v, &[("offset", &[0.5, 0.0]), ("scale", &[0.5])]));
    let tint = [1.0, 0.5, 2.0, 1.0];
    let fs_block = buffer(&mut d, &uniforms(&f, &[("tint", &tint)]));
    let location = |name: &str| {
        let io = v.interface.inputs.iter().find(|i| i.name == name).unwrap().io[0];
        match p.vertex.inputs[io as usize].slot {
            Slot::Location(l) => l,
            other => panic!("{other:?}"),
        }
    };
    let attributes = [
        Attribute { location: location("pos"), buffer: 0, offset: 0, format: VertexFormat::F32(2) },
        Attribute { location: location("uv"), buffer: 0, offset: 8, format: VertexFormat::F32(2) },
    ];
    let pipeline = d
        .graphics(&GraphicsDesc {
            vertex: &p.vertex,
            fragment: &p.fragment,
            buffers: &[VertexBuffer { stride: 16, per_instance: false }],
            attributes: &attributes,
            topology: Topology::TriangleStrip,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &[ColorTarget { format: Format::Rgba32Float, blend: None, write: [true; 4] }],
        })
        .unwrap_or_else(|e| panic!("{}", e.message));
    let quad = buffer(&mut d, &f32s(&[-1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]));
    const N: u32 = 8;
    let target = d.image(ImageDesc::new(Format::Rgba32Float, N, N)).unwrap();
    let sampler_binding = p.samplers.iter().find(|s| s.0 == "tex").unwrap().1;
    wait(
        &mut d,
        &[
            Command::BeginRender { colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: Load::Clear([0.0; 4]) }], depth: None },
            Command::Draw(Draw {
                pipeline,
                bindings: vec![
                    (glsl::SET_DEFAULT, 0, Bind::Buffer(vs_block)),
                    (glsl::SET_DEFAULT, 1, Bind::Buffer(fs_block)),
                    (glsl::SET_SAMPLER, sampler_binding, Bind::Combined(tex, sampler)),
                ],
                vertex_buffers: vec![quad],
                index: None,
                first: 0,
                count: 4,
                vertex_offset: 0,
                first_instance: 0,
                instances: 1,
            }),
            Command::EndRender,
        ],
    );
    let out = d.alloc(u64::from(N * N * 16)).unwrap();
    wait(&mut d, &[Command::CopyImageToBuffer { from: target, sub: Sub::default(), region: Rect { x: 0, y: 0, width: N, height: N }, to: Range { memory: out, offset: 0, size: u64::from(N * N * 16) }, row_pixels: 0 }]);
    let mut bytes = vec![0u8; (N * N * 16) as usize];
    d.read(out, 0, &mut bytes).unwrap();
    let px: Vec<f32> = words(&bytes).iter().map(|w| f32::from_bits(*w)).collect();
    for y in 0..N {
        for x in 0..N {
            let got = &px[((y * N + x) * 4) as usize..][..4];
            let want = if (4..8).contains(&x) && (2..6).contains(&y) {
                let (tx, ty) = ((x - 4) / 2, (y - 2) / 2);
                let t = &texels[((ty * 2 + tx) * 4) as usize..][..3];
                let shade = ((x - 4) as f32 + 0.5) / 4.0;
                [f32::from(t[0]) / 255.0 * tint[0], f32::from(t[1]) / 255.0 * tint[1], f32::from(t[2]) / 255.0 * tint[2], shade]
            } else {
                [0.0; 4]
            };
            assert!(got.iter().zip(&want).all(|(g, w)| (g - w).abs() < 1e-5), "({x}, {y}) is {got:?}, wanted {want:?}, on {choice:?}");
        }
    }
}

#[test]
fn a_glsl_110_program_draws_on_the_cpu_device() {
    draw(Choice::Software);
}

#[test]
fn a_glsl_110_program_draws_on_the_host_gpu() {
    draw(Choice::Host);
}
