//! The compatibility profile's state, as GL 2.1 has it: the matrix stacks, each unit's texture environment and
//! whether it's on, the alpha test, fog and the shade model. What a program reads of it, the compatibility profile's
//! built-in uniforms (`gl_ModelViewProjectionMatrix` and the rest), is written into its default blocks at each draw.
//!
//! Fixed function is GLSL here: with no program in use, a draw's program is one written from the state that shapes it
//! (the units on and their environments, the alpha test's function, fog's mode), compiled by the GLSL compiler, kept
//! by that shape, and fed what changes without reshaping it (the alpha reference, fog's numbers, the environments'
//! colours) as uniforms.

use std::fmt::Write as _;

use gpu_wire::gl as wire;

use super::program::{Linked, Program};
use super::{Context, Gl, INVALID_ENUM, INVALID_VALUE, Objects, R, context};

pub(super) const TEXTURE_2D: u32 = 0x0de1;
pub(super) const ALPHA_TEST: u32 = 0x0bc0;
pub(super) const FOG: u32 = 0x0b60;
pub(super) const LIGHTING: u32 = 0x0b50;
const MODELVIEW: u32 = 0x1700;
const PROJECTION: u32 = 0x1701;
const TEXTURE: u32 = 0x1702;
const STACK_OVERFLOW: u32 = 0x0503;
const STACK_UNDERFLOW: u32 = 0x0504;
/// Texture units fixed function has (`GL_MAX_TEXTURE_UNITS`).
pub(super) const UNITS: usize = 8;

/// A 4×4 matrix, column-major.
pub(super) type M = [f32; 16];

const IDENTITY: M = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A unit's texture environment: what shapes its program, and its colour, which doesn't.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Env {
    mode: u32,
    combine: [u32; 2],
    sources: [[u32; 3]; 2],
    operands: [[u32; 3]; 2],
    /// RGB_SCALE and ALPHA_SCALE, as bits.
    scales: [u32; 2],
}

impl Default for Env {
    fn default() -> Env {
        // MODULATE; COMBINE's defaults: MODULATE of TEXTURE, PREVIOUS and CONSTANT, colours then alpha.
        let sources = [0x1702, 0x8578, 0x8576];
        Env { mode: 0x2100, combine: [0x2100, 0x2100], sources: [sources; 2], operands: [[0x0300, 0x0300, 0x0302], [0x0302; 3]], scales: [1.0f32.to_bits(); 2] }
    }
}

#[derive(Clone)]
pub(super) struct Compat {
    mode: u32,
    modelview: Vec<M>,
    projection: Vec<M>,
    texture: [Vec<M>; UNITS],
    pub env: [Env; UNITS],
    env_color: [[f32; 4]; UNITS],
    /// `glEnable(GL_TEXTURE_2D)`, each unit's.
    pub textures: [bool; UNITS],
    alpha: (u32, f32),
    fog_mode: u32,
    /// Fog's density, start and end, and colour.
    fog: [f32; 3],
    fog_color: [f32; 4],
    shade: u32,
}

impl Default for Compat {
    fn default() -> Compat {
        Compat {
            mode: MODELVIEW,
            modelview: vec![IDENTITY],
            projection: vec![IDENTITY],
            texture: std::array::from_fn(|_| vec![IDENTITY]),
            env: [Env::default(); UNITS],
            env_color: [[0.0; 4]; UNITS],
            textures: [false; UNITS],
            alpha: (0x0207, 0.0),
            fog_mode: 0x0800,
            fog: [1.0, 0.0, 1.0],
            fog_color: [0.0; 4],
            shade: 0x1d01,
        }
    }
}

/// What shapes a fixed-function program.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Shape {
    units: Vec<(usize, Env)>,
    alpha: Option<u32>,
    fog: Option<u32>,
}

impl Compat {
    fn stack(&mut self, unit: u32) -> &mut Vec<M> {
        match self.mode {
            PROJECTION => &mut self.projection,
            TEXTURE => &mut self.texture[(unit as usize).min(UNITS - 1)],
            _ => &mut self.modelview,
        }
    }

    pub(super) fn matrix(&self, which: u32, unit: usize) -> M {
        let top = |s: &Vec<M>| *s.last().unwrap_or(&IDENTITY);
        match which {
            PROJECTION => top(&self.projection),
            TEXTURE => top(&self.texture[unit.min(UNITS - 1)]),
            _ => top(&self.modelview),
        }
    }

    pub(super) fn mode(&self) -> u32 {
        self.mode
    }

    /// The alpha test's function and reference.
    pub(super) fn alpha(&self) -> (u32, f32) {
        self.alpha
    }
}

impl Gl {
    /// A compatibility profile record, if it's one: whether it was.
    pub(super) fn compat_record(&self, o: &mut Objects, current: u64, kind: u32, p: &mut R<'_>) -> Result<bool, String> {
        match kind {
            wire::MATRIX_MODE => {
                let mode = p.u32()?;
                let c = context(o, current)?;
                if matches!(mode, MODELVIEW | PROJECTION | TEXTURE) {
                    c.compat.mode = mode;
                } else {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::MATRIX => {
                let op = p.u32()?;
                let f: Vec<f32> = p.bytes()?.as_chunks::<4>().0.iter().map(|w| f32::from_le_bytes(*w)).collect();
                let c = context(o, current)?;
                let unit = c.unit;
                let depth = if c.compat.mode == MODELVIEW { 32 } else { 4 };
                let need = match op {
                    wire::MATRIX_LOAD | wire::MATRIX_MULTIPLY => 16,
                    wire::MATRIX_TRANSLATE | wire::MATRIX_SCALE => 3,
                    wire::MATRIX_ROTATE => 4,
                    wire::MATRIX_ORTHO | wire::MATRIX_FRUSTUM => 6,
                    _ => 0,
                };
                if f.len() < need {
                    return Err("a matrix operation without its numbers".into());
                }
                let stack = c.compat.stack(unit);
                let top = *stack.last().expect("a matrix");
                let error = match op {
                    wire::MATRIX_IDENTITY => set(stack, IDENTITY),
                    wire::MATRIX_LOAD => set(stack, f[..16].try_into().expect("16")),
                    wire::MATRIX_MULTIPLY => set(stack, mul(&top, &f[..16].try_into().expect("16"))),
                    wire::MATRIX_PUSH if stack.len() >= depth => Some(STACK_OVERFLOW),
                    wire::MATRIX_PUSH => {
                        stack.push(top);
                        None
                    }
                    wire::MATRIX_POP if stack.len() <= 1 => Some(STACK_UNDERFLOW),
                    wire::MATRIX_POP => {
                        stack.pop();
                        None
                    }
                    wire::MATRIX_TRANSLATE => set(stack, mul(&top, &translate(f[0], f[1], f[2]))),
                    wire::MATRIX_SCALE => set(stack, mul(&top, &scale(f[0], f[1], f[2]))),
                    wire::MATRIX_ROTATE => set(stack, mul(&top, &rotate(f[0], f[1], f[2], f[3]))),
                    wire::MATRIX_ORTHO | wire::MATRIX_FRUSTUM => match projection(op == wire::MATRIX_ORTHO, &f) {
                        Some(m) => set(stack, mul(&top, &m)),
                        None => Some(INVALID_VALUE),
                    },
                    _ => Some(INVALID_ENUM),
                };
                if let Some(e) = error {
                    c.fail(e);
                }
            }
            wire::TEX_ENV => {
                let pname = p.u32()?;
                let v = p.words::<4>()?;
                let float = p.u32()? != 0;
                let c = context(o, current)?;
                let unit = (c.unit as usize).min(UNITS - 1);
                let int = if float { f32::from_bits(v[0]) as u32 } else { v[0] };
                let num = if float { f32::from_bits(v[0]) } else { v[0] as f32 };
                let e = &mut c.compat.env[unit];
                let ok = match pname {
                    0x2200 => {
                        e.mode = int;
                        matches!(int, 0x2100 | 0x1e01 | 0x2101 | 0x0be2 | 0x0104 | 0x8570)
                    }
                    0x2201 => {
                        c.compat.env_color[unit] = v.map(|w| if float { f32::from_bits(w) } else { w as i32 as f32 / i32::MAX as f32 });
                        true
                    }
                    0x8571 | 0x8572 => {
                        e.combine[(pname - 0x8571) as usize] = int;
                        true
                    }
                    0x8580..=0x8582 => {
                        e.sources[0][(pname - 0x8580) as usize] = int;
                        true
                    }
                    0x8588..=0x858a => {
                        e.sources[1][(pname - 0x8588) as usize] = int;
                        true
                    }
                    0x8590..=0x8592 => {
                        e.operands[0][(pname - 0x8590) as usize] = int;
                        true
                    }
                    0x8598..=0x859a => {
                        e.operands[1][(pname - 0x8598) as usize] = int;
                        true
                    }
                    0x8573 => {
                        e.scales[0] = num.to_bits();
                        true
                    }
                    0x0d1c => {
                        e.scales[1] = num.to_bits();
                        true
                    }
                    // LOD bias and point sprites' coordinates: not drawn with yet, and harmless to keep.
                    0x8501 | 0x8862 => true,
                    _ => false,
                };
                if !ok {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::ALPHA_FUNC => {
                let (func, reference) = (p.u32()?, f32::from_bits(p.u32()?));
                let c = context(o, current)?;
                if super::draw::compare(func).is_some() {
                    c.compat.alpha = (func, reference.clamp(0.0, 1.0));
                } else {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::FOG => {
                let pname = p.u32()?;
                let v = p.words::<4>()?;
                let float = p.u32()? != 0;
                let c = context(o, current)?;
                let num = |w: u32| if float { f32::from_bits(w) } else { w as i32 as f32 };
                let ok = match pname {
                    0x0b65 => {
                        let m = num(v[0]) as u32;
                        c.compat.fog_mode = m;
                        matches!(m, 0x2601 | 0x0800 | 0x0801)
                    }
                    0x0b62..=0x0b64 => {
                        c.compat.fog[(pname - 0x0b62) as usize] = num(v[0]);
                        true
                    }
                    0x0b66 => {
                        c.compat.fog_color = v.map(|w| if float { f32::from_bits(w) } else { w as i32 as f32 / i32::MAX as f32 });
                        true
                    }
                    // FOG_COORD_SRC, FOG_INDEX, and the hint.
                    0x8450 | 0x0b61 => true,
                    _ => false,
                };
                if !ok {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::SHADE_MODEL => {
                let mode = p.u32()?;
                let c = context(o, current)?;
                match mode {
                    0x1d01 => c.compat.shade = mode,
                    0x1d00 => {
                        c.compat.shade = mode;
                        super::draw::not_yet("flat shading (it's drawn smooth)", 16);
                    }
                    _ => c.fail(INVALID_ENUM),
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The program a draw with none in use runs: fixed function's, written for the context's state, made once for each
    /// shape. Its name is past every name the guest hands out.
    pub(super) fn fixed_program(&self, o: &mut Objects, current: u64) -> Result<Option<u32>, String> {
        let c = context(o, current)?;
        if c.is_enabled(LIGHTING) {
            super::draw::not_yet("fixed function's lighting (it's drawn unlit)", 32);
        }
        let shape = Shape {
            units: (0..UNITS).filter(|&u| c.compat.textures[u]).map(|u| (u, c.compat.env[u])).collect(),
            alpha: (c.is_enabled(ALPHA_TEST) && c.compat.alpha.0 != 0x0207).then_some(c.compat.alpha.0),
            fog: c.is_enabled(FOG).then_some(c.compat.fog_mode),
        };
        if let Some(&(_, name)) = o.fixed.iter().find(|f| f.0 == shape) {
            return Ok(Some(name));
        }
        let (vs, fs) = sources(&shape);
        let program = match super::program::program_from(&vs, &fs) {
            Ok(p) => p,
            Err(e) => return Err(format!("fixed function's program didn't build: {e}\n{vs}\n{fs}")),
        };
        let name = 0x8000_0000 + o.fixed.len() as u32;
        let mut program: Program = program;
        // Each unit's sampler reads that unit.
        if let Some(l) = program.linked.as_mut() {
            for (u, _) in &shape.units {
                let n = format!("carbon_tex{u}");
                if let Some(binding) = l.slots.iter().find(|s| s.name == n).and_then(|s| s.sampler) {
                    l.units[binding as usize] = *u as u32;
                }
            }
        }
        o.programs.insert(name, program);
        o.fixed.push((shape, name));
        Ok(Some(name))
    }
}

/// Sets a stack's top: no error.
fn set(stack: &mut [M], m: M) -> Option<u32> {
    if let Some(top) = stack.last_mut() {
        *top = m;
    }
    None
}

/// `a * b`, column-major.
pub(super) fn mul(a: &M, b: &M) -> M {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    out
}

fn translate(x: f32, y: f32, z: f32) -> M {
    let mut m = IDENTITY;
    m[12] = x;
    m[13] = y;
    m[14] = z;
    m
}

fn scale(x: f32, y: f32, z: f32) -> M {
    let mut m = IDENTITY;
    m[0] = x;
    m[5] = y;
    m[10] = z;
    m
}

/// `glRotate`'s matrix: degrees about an axis (normalized).
fn rotate(degrees: f32, x: f32, y: f32, z: f32) -> M {
    let len = (x * x + y * y + z * z).sqrt();
    if len == 0.0 {
        return IDENTITY;
    }
    let (x, y, z) = (x / len, y / len, z / len);
    let (s, c) = degrees.to_radians().sin_cos();
    let t = 1.0 - c;
    [
        x * x * t + c,
        y * x * t + z * s,
        x * z * t - y * s,
        0.0,
        x * y * t - z * s,
        y * y * t + c,
        y * z * t + x * s,
        0.0,
        x * z * t + y * s,
        y * z * t - x * s,
        z * z * t + c,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// `glOrtho`'s or `glFrustum`'s matrix, or None for the values GL refuses.
fn projection(ortho: bool, f: &[f32]) -> Option<M> {
    let (l, r, b, t, n, fa) = (f[0], f[1], f[2], f[3], f[4], f[5]);
    if l == r || b == t || n == fa || (!ortho && (n <= 0.0 || fa <= 0.0)) {
        return None;
    }
    let mut m = [0.0; 16];
    if ortho {
        m[0] = 2.0 / (r - l);
        m[5] = 2.0 / (t - b);
        m[10] = -2.0 / (fa - n);
        m[12] = -(r + l) / (r - l);
        m[13] = -(t + b) / (t - b);
        m[14] = -(fa + n) / (fa - n);
        m[15] = 1.0;
    } else {
        m[0] = 2.0 * n / (r - l);
        m[5] = 2.0 * n / (t - b);
        m[8] = (r + l) / (r - l);
        m[9] = (t + b) / (t - b);
        m[10] = -(fa + n) / (fa - n);
        m[11] = -1.0;
        m[14] = -2.0 * fa * n / (fa - n);
    }
    Some(m)
}

fn transpose(m: &M) -> M {
    std::array::from_fn(|i| m[(i % 4) * 4 + i / 4])
}

/// A 4×4 matrix's inverse, by cofactors (all zeros for one that has none).
fn inverse(m: &M) -> M {
    let det3 = |a: [f32; 9]| a[0] * (a[4] * a[8] - a[7] * a[5]) - a[3] * (a[1] * a[8] - a[7] * a[2]) + a[6] * (a[1] * a[5] - a[4] * a[2]);
    let minor = |col: usize, row: usize| {
        let mut out = [0.0; 9];
        let mut k = 0;
        for c in (0..4).filter(|&c| c != col) {
            for r in (0..4).filter(|&r| r != row) {
                out[k] = m[c * 4 + r];
                k += 1;
            }
        }
        det3(out)
    };
    let det: f32 = (0..4).map(|r| if r % 2 == 0 { 1.0 } else { -1.0 } * m[r] * minor(0, r)).sum();
    if det == 0.0 {
        return [0.0; 16];
    }
    // Entry (row, col) of the inverse is cofactor (col, row) over the determinant.
    std::array::from_fn(|i| {
        let (col, row) = (i / 4, i % 4);
        let sign = if (row + col) % 2 == 0 { 1.0 } else { -1.0 };
        sign * minor(row, col) / det
    })
}

/// Writes the compatibility profile's built-ins (and fixed function's own uniforms) into a program's default blocks,
/// from the context's state as it is at a draw.
/// `size_of` is the size of the 2D texture or cube map (`true`) on a unit, for the samplers' sizes `texelFetch`,
/// `textureSize` and cube maps read.
pub(super) fn fill(l: &mut Linked, c: &Context, size_of: &dyn Fn(u32, bool) -> (u32, u32)) {
    let k = &c.compat;
    let mv = k.matrix(MODELVIEW, 0);
    let p = k.matrix(PROJECTION, 0);
    let mvp = mul(&p, &mv);
    let variant = |m: &M, suffix: &str| match suffix {
        "Inverse" => inverse(m),
        "Transpose" => transpose(m),
        "InverseTranspose" => transpose(&inverse(m)),
        _ => *m,
    };
    let names: Vec<(usize, String)> = l.slots.iter().enumerate().filter(|(_, s)| s.name.starts_with("gl_") || s.name.starts_with("carbon_")).map(|(i, s)| (i, s.name.clone())).collect();
    for (slot, name) in names {
        let size = name.strip_prefix("carbon_size_").map(|sampler| {
            let binding = l.slots.iter().find(|s| s.name == sampler).and_then(|s| s.sampler);
            let unit = binding.and_then(|b| l.units.get(b as usize).copied());
            let cube = binding.is_some_and(|b| l.cube.get(b as usize).copied().unwrap_or(false));
            unit.map_or((1, 1), |u| size_of(u, cube))
        });
        let words = |v: &[f32]| v.iter().map(|f| f.to_bits()).collect::<Vec<u32>>();
        let mut put = |elem: u32, cols: u32, rows: u32, v: &[f32]| write(l, slot, elem, cols, rows, &words(v));
        if let Some(s) = name.strip_prefix("gl_ModelViewProjectionMatrix") {
            put(0, 4, 4, &variant(&mvp, s));
        } else if let Some(s) = name.strip_prefix("gl_ModelViewMatrix") {
            put(0, 4, 4, &variant(&mv, s));
        } else if let Some(s) = name.strip_prefix("gl_ProjectionMatrix") {
            put(0, 4, 4, &variant(&p, s));
        } else if let Some(s) = name.strip_prefix("gl_TextureMatrix") {
            for u in 0..UNITS {
                put(u as u32, 4, 4, &variant(&k.matrix(TEXTURE, u), s));
            }
        } else if name == "gl_NormalMatrix" {
            // The upper left 3×3 of the model-view matrix's inverse transpose.
            let n = transpose(&inverse(&mv));
            put(0, 3, 3, &[n[0], n[1], n[2], n[4], n[5], n[6], n[8], n[9], n[10]]);
        } else if name == "gl_NormalScale" {
            put(0, 1, 1, &[1.0]);
        } else if name == "carbon_alpha_ref" {
            put(0, 1, 1, &[k.alpha.1]);
        } else if name == "carbon_fog" {
            let [density, start, end] = k.fog;
            put(0, 4, 1, &[start, end, density, if end != start { 1.0 / (end - start) } else { 0.0 }]);
        } else if name == "carbon_fog_color" {
            put(0, 4, 1, &k.fog_color);
        } else if let Some((w, h)) = size {
            put(0, 2, 1, &[w as f32, h as f32]);
        } else if let Some(u) = name.strip_prefix("carbon_env").and_then(|u| u.parse::<usize>().ok()) {
            put(0, 4, 1, &k.env_color[u.min(UNITS - 1)]);
        }
    }
}

/// Floats into a uniform's element, where each stage keeps it.
fn write(l: &mut Linked, slot: usize, elem: u32, cols: u32, rows: u32, words: &[u32]) {
    let at = l.slots[slot].at;
    for (stage, at) in at.iter().enumerate() {
        let Some((offset, stride, matrix_stride)) = *at else { continue };
        let base = (offset + elem * stride) as usize;
        for col in 0..cols {
            for row in 0..rows {
                let at = base + if rows == 1 { col * 4 } else { col * matrix_stride + row * 4 } as usize;
                if let Some(dst) = l.blocks[stage].get_mut(at..at + 4) {
                    dst.copy_from_slice(&words[(col * rows + row) as usize].to_le_bytes());
                }
            }
        }
    }
}

/// Fixed function as GLSL 1.20: the vertex transformed, its colour and each unit's coordinates passed on, each unit's
/// texel combined as its environment says, the alpha test, and fog.
fn sources(shape: &Shape) -> (String, String) {
    let mut vs = String::from("#version 120\nvoid main() {\n    gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex;\n    gl_FrontColor = gl_Color;\n");
    for (u, _) in &shape.units {
        let _ = writeln!(vs, "    gl_TexCoord[{u}] = gl_TextureMatrix[{u}] * gl_MultiTexCoord{u};");
    }
    if shape.fog.is_some() {
        vs.push_str("    gl_FogFragCoord = abs((gl_ModelViewMatrix * gl_Vertex).z);\n");
    }
    vs.push_str("}\n");
    let mut fs = String::from("#version 120\n");
    for (u, _) in &shape.units {
        let _ = writeln!(fs, "uniform sampler2D carbon_tex{u};\nuniform vec4 carbon_env{u};");
    }
    fs.push_str("uniform float carbon_alpha_ref;\nuniform vec4 carbon_fog;\nuniform vec4 carbon_fog_color;\nvoid main() {\n    vec4 primary = gl_Color;\n    vec4 previous = primary;\n");
    for (u, _) in &shape.units {
        let _ = writeln!(fs, "    vec4 t{u} = texture2D(carbon_tex{u}, gl_TexCoord[{u}].xy);");
    }
    for (u, env) in &shape.units {
        let _ = writeln!(fs, "    previous = {};", environment(*u, env, &shape.units));
    }
    if let Some(func) = shape.alpha {
        let test = match func {
            0x0200 => "false".to_string(),
            f => format!("previous.a {} carbon_alpha_ref", ["<", "==", "<=", ">", "!=", ">="][(f - 0x0201) as usize]),
        };
        let _ = writeln!(fs, "    if (!({test})) discard;");
    }
    if let Some(mode) = shape.fog {
        let f = match mode {
            0x2601 => "(carbon_fog.y - gl_FogFragCoord) * carbon_fog.w",
            0x0800 => "exp(-carbon_fog.z * gl_FogFragCoord)",
            _ => "exp(-(carbon_fog.z * gl_FogFragCoord) * (carbon_fog.z * gl_FogFragCoord))",
        };
        let _ = writeln!(fs, "    float fog = clamp({f}, 0.0, 1.0);\n    previous.rgb = mix(carbon_fog_color.rgb, previous.rgb, fog);");
    }
    fs.push_str("    gl_FragColor = previous;\n}\n");
    (vs, fs)
}

/// One unit's texture environment, as an expression of its texel (`t<u>`), what came before, and the rest.
fn environment(u: usize, env: &Env, units: &[(usize, Env)]) -> String {
    let t = format!("t{u}");
    match env.mode {
        0x1e01 => t,
        0x2101 => format!("vec4(mix(previous.rgb, {t}.rgb, {t}.a), previous.a)"),
        0x0be2 => format!("vec4(mix(previous.rgb, carbon_env{u}.rgb, {t}.rgb), previous.a * {t}.a)"),
        0x0104 => format!("vec4(previous.rgb + {t}.rgb, previous.a * {t}.a)"),
        0x8570 => {
            let source = |s: u32| match s {
                0x1702 => t.clone(),
                0x8576 => format!("carbon_env{u}"),
                0x8577 => "primary".into(),
                0x84c0..=0x84c7 if units.iter().any(|x| x.0 == (s - 0x84c0) as usize) => format!("t{}", s - 0x84c0),
                0x84c0..=0x84c7 => "vec4(0.0, 0.0, 0.0, 1.0)".into(),
                _ => "previous".into(),
            };
            let rgb_arg = |i: usize| {
                let s = source(env.sources[0][i]);
                match env.operands[0][i] {
                    0x0301 => format!("(vec3(1.0) - {s}.rgb)"),
                    0x0302 => format!("vec3({s}.a)"),
                    0x0303 => format!("vec3(1.0 - {s}.a)"),
                    _ => format!("{s}.rgb"),
                }
            };
            let alpha_arg = |i: usize| {
                let s = source(env.sources[1][i]);
                if env.operands[1][i] == 0x0303 { format!("(1.0 - {s}.a)") } else { format!("{s}.a") }
            };
            let func = |f: u32, a: &dyn Fn(usize) -> String, one: &str| match f {
                0x1e01 => a(0),
                0x0104 => format!("({} + {})", a(0), a(1)),
                0x8574 => format!("({} + {} - {one}(0.5))", a(0), a(1)),
                0x8575 => format!("({0} * {2} + {1} * ({one}(1.0) - {2}))", a(0), a(1), a(2)),
                0x84e7 => format!("({} - {})", a(0), a(1)),
                0x86ae | 0x86af => format!("{one}(4.0 * dot({} - vec3(0.5), {} - vec3(0.5)))", a(0), a(1)),
                _ => format!("({} * {})", a(0), a(1)),
            };
            let (rs, als) = (f32::from_bits(env.scales[0]), f32::from_bits(env.scales[1]));
            let rgb = func(env.combine[0], &rgb_arg, "vec3");
            // DOT3_RGBA puts its dot product in alpha too.
            let alpha = if env.combine[0] == 0x86af { format!("({rgb}).r") } else { func(env.combine[1], &alpha_arg, "float") };
            format!("clamp(vec4({rgb} * {rs:?}, {alpha} * {als:?}), 0.0, 1.0)")
        }
        _ => format!("previous * {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices() {
        let m = mul(&translate(1.0, 2.0, 3.0), &scale(2.0, 2.0, 2.0));
        let back = mul(&inverse(&m), &m);
        assert!(back.iter().zip(IDENTITY).all(|(a, b)| (a - b).abs() < 1e-6));
        let r = rotate(90.0, 0.0, 0.0, 1.0);
        // x goes to y.
        assert!((r[1] - 1.0).abs() < 1e-6 && r[0].abs() < 1e-6);
        let o = projection(true, &[0.0, 8.0, 0.0, 8.0, -1.0, 1.0]).unwrap();
        // (8, 8) is the top right corner of clip space.
        assert!((o[0] * 8.0 + o[12] - 1.0).abs() < 1e-6 && (o[5] * 8.0 + o[13] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn every_environment_compiles() {
        let mut units = Vec::new();
        for (u, mode) in [0x2100, 0x1e01, 0x2101, 0x0be2, 0x0104, 0x8570].into_iter().enumerate() {
            units.push((u, Env { mode, ..Env::default() }));
        }
        let mut combine = Env { mode: 0x8570, ..Env::default() };
        for f in [0x1e01, 0x2100, 0x0104, 0x8574, 0x8575, 0x84e7, 0x86ae] {
            combine.combine = [f, if f == 0x86ae { 0x2100 } else { f }];
            combine.sources = [[0x1702, 0x8577, 0x84c0], [0x8576, 0x8578, 0x1702]];
            combine.operands = [[0x0301, 0x0302, 0x0303], [0x0303, 0x0302, 0x0302]];
            units.push((6, combine));
            for (alpha, fog) in [(Some(0x0204), Some(0x2601)), (Some(0x0200), Some(0x0800)), (None, Some(0x0801))] {
                let (vs, fs) = sources(&Shape { units: units.clone(), alpha, fog });
                if let Err(e) = super::super::program::program_from(&vs, &fs) {
                    panic!("{e}\n{fs}");
                }
            }
            units.pop();
        }
    }
}
