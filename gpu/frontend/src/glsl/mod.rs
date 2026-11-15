//! GLSL to gpu's IR: OpenGL's and OpenGL ES's shading language, as programs hand it to `glShaderSource`, compiled
//! by Carbon's own GL (`gpu_api::gl`) for both kernels. GLSL 1.10 to 4.60 and ES 1.00 to 3.20 are read; what the IR
//! and this compiler have of them so far is said where it's missing.
//!
//! [`compile`] runs the preprocessor (`pp`), the parser (`parse`) and the lowering (`lower`, `expr`, `builtins`)
//! for one stage, giving its entry point and its interface: the inputs, outputs, uniforms (the default block's,
//! laid out std430 for GL to fill, and samplers), and uniform and buffer blocks, each with the binding it got.
//! [`link`] matches a vertex shader's outputs to a fragment shader's inputs by name and gives both the same locations.
//!
//! Errors read as GL's info logs do: `0:LINE: error: what`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use gpu_ir::{Entry, Slot};

pub mod ast;
mod builtins;
mod expr;
mod lower;
mod parse;
mod pp;
pub mod types;

#[cfg(test)]
mod tests;

pub use types::Ty;

/// A GLSL version: its number, and whether it's GLSL ES's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub number: u32,
    pub es: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
}

/// Why a shader didn't compile or link, as an info log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) fn error<T>(line: u32, what: &str) -> Result<T, Error> {
    Err(Error { message: format!("0:{line}: error: {what}") })
}

/// The descriptor sets a shader's bindings are in, by what they are: the default uniform block (binding 0), uniform
/// blocks and buffer blocks (each numbered in declaration order), and samplers (numbered as declared).
pub const SET_DEFAULT: u32 = 0;
pub const SET_UNIFORM_BLOCK: u32 = 1;
pub const SET_BUFFER: u32 = 2;
pub const SET_SAMPLER: u32 = 3;

/// An input or output: its name and type, the location it asked for, and the entry's inputs or outputs it takes
/// (one for each location: a matrix's columns, an array's elements).
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub name: String,
    pub ty: Ty,
    pub location: Option<u32>,
    pub io: Vec<u32>,
}

/// A plain uniform, as GL names it (`light.color`, `lights[1].color`, `weights`): its type (an array's element, with
/// its count), where it is in the default block (offset, array and matrix strides), its explicit location, and for a
/// sampler, its binding in the entry and its explicit `binding`.
#[derive(Debug, Clone, PartialEq)]
pub struct Uniform {
    pub name: String,
    pub ty: Ty,
    pub count: Option<u32>,
    pub offset: u32,
    pub stride: u32,
    pub matrix_stride: u32,
    pub location: Option<u32>,
    pub binding: Option<(u32, Option<u32>)>,
}

/// A uniform or buffer block: its name, its binding in the entry, the binding point it asked for, its size, and its
/// members with their offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub name: String,
    pub buffer: bool,
    pub binding: u32,
    pub explicit: Option<u32>,
    pub size: u32,
    pub members: Vec<(String, Ty, u32)>,
}

/// What a compiled shader takes and gives.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Interface {
    pub inputs: Vec<Variable>,
    pub outputs: Vec<Variable>,
    pub uniforms: Vec<Uniform>,
    /// The default block's binding in the entry, and its size.
    pub default_block: Option<(u32, u32)>,
    pub blocks: Vec<BlockInfo>,
}

/// A compiled shader.
#[derive(Debug, Clone, PartialEq)]
pub struct Shader {
    pub stage: Stage,
    pub version: Version,
    pub entry: Entry,
    pub interface: Interface,
    /// Warnings, as info log lines.
    pub log: Vec<String>,
}

/// Compiles one stage's source (`es`: GLSL ES when the source has no `#version`, as an ES context's are).
pub fn compile(source: &str, stage: Stage, es: bool) -> Result<Shader, Error> {
    let pre = pp::run(source, es)?;
    let unit = parse::parse(&pre.tokens, pre.version)?;
    if stage == Stage::Compute && pre.version.number < if pre.version.es { 310 } else { 430 } && !pre.extensions.contains_key("GL_ARB_compute_shader") {
        return error(0, &format!("compute shaders aren't in GLSL {}", pre.version));
    }
    let mut l = lower::Lower::new(stage, pre.version);
    l.compat = pre.compatibility;
    l.unit(&unit)?;
    gpu_ir::validate(&l.e).map_err(|e| Error { message: format!("0:0: error: the IR made isn't valid (a bug in the compiler): {}", e.message) })?;
    Ok(Shader { stage, version: pre.version, entry: l.e, interface: l.iface, log: pre.warnings })
}

/// A linked program: its two entries, with each sampler's and block's binding (in [`SET_SAMPLER`],
/// [`SET_UNIFORM_BLOCK`] and [`SET_BUFFER`]) numbered across the program by name, so a sampler both stages use is one
/// binding. The default blocks stay each stage's own (vertex 0, fragment 1, in [`SET_DEFAULT`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub vertex: Entry,
    pub fragment: Entry,
    /// Each sampler uniform's binding number, by its name.
    pub samplers: Vec<(String, u32)>,
    /// Each block's set and binding number, by its name.
    pub blocks: Vec<(String, u32, u32)>,
}

/// A vertex and a fragment shader linked: each of the fragment shader's inputs fed by the vertex shader's output of
/// the same name and type, both given the same locations (an explicit one where either says it, else the next free),
/// and the resources numbered across the program.
pub fn link(vertex: &Shader, fragment: &Shader) -> Result<Program, Error> {
    if vertex.stage != Stage::Vertex || fragment.stage != Stage::Fragment {
        return error(0, "a program links a vertex shader and a fragment shader");
    }
    let (mut v, mut f) = (vertex.entry.clone(), fragment.entry.clone());
    let mut next = 0u32;
    let mut used: Vec<u32> = Vec::new();
    let explicit: Vec<u32> = vertex.interface.outputs.iter().chain(&fragment.interface.inputs).filter_map(|x| x.location).collect();
    for input in &fragment.interface.inputs {
        let source = fed_by(&input.name);
        let Some(output) = vertex.interface.outputs.iter().find(|o| o.name == source) else {
            return error(0, &format!("the fragment shader's {} isn't written by the vertex shader", input.name));
        };
        if output.ty != input.ty {
            return error(0, &format!("{} is {} in the vertex shader and {} in the fragment shader", input.name, output.ty, input.ty));
        }
        let first = match input.location.or(output.location) {
            Some(l) => l,
            None => {
                while (next..next + input.io.len() as u32).any(|l| used.contains(&l) || explicit.contains(&l)) {
                    next += 1;
                }
                next
            }
        };
        for (k, (&i, &o)) in input.io.iter().zip(&output.io).enumerate() {
            let at = first + k as u32;
            used.push(at);
            f.inputs[i as usize].slot = Slot::Location(at);
            v.outputs[o as usize].slot = Slot::Location(at);
            // Interpolation is the fragment shader's to say.
            v.outputs[o as usize].interpolation = f.inputs[i as usize].interpolation;
        }
    }
    // Outputs nothing reads keep locations clear of those that are read.
    let mut spare = used.iter().copied().max().map_or(0, |m| m + 1);
    let read: Vec<u32> = fragment.interface.inputs.iter().flat_map(|i| vertex.interface.outputs.iter().find(|o| o.name == fed_by(&i.name))).flat_map(|o| o.io.clone()).collect();
    for o in &vertex.interface.outputs {
        for &k in &o.io {
            if !read.contains(&k) {
                v.outputs[k as usize].slot = Slot::Location(spare);
                spare += 1;
            }
        }
    }
    // Samplers and blocks, numbered by name across both stages.
    let mut samplers: Vec<(String, u32)> = Vec::new();
    let mut blocks: Vec<(String, u32, u32)> = Vec::new();
    for (shader, entry) in [(vertex, &mut v), (fragment, &mut f)] {
        for u in &shader.interface.uniforms {
            if let Some((b, _)) = u.binding {
                let n = match samplers.iter().find(|s| s.0 == u.name) {
                    Some(s) => s.1,
                    None => {
                        samplers.push((u.name.clone(), samplers.len() as u32));
                        samplers.len() as u32 - 1
                    }
                };
                entry.bindings[b as usize].binding = n;
            }
        }
        for b in &shader.interface.blocks {
            let set = if b.buffer { SET_BUFFER } else { SET_UNIFORM_BLOCK };
            let n = match blocks.iter().find(|k| k.0 == b.name) {
                Some(k) if k.1 != set => return error(0, &format!("{} is a uniform block in one stage and a buffer in the other", b.name)),
                Some(k) => k.2,
                None => {
                    let n = blocks.iter().filter(|k| k.1 == set).count() as u32;
                    blocks.push((b.name.clone(), set, n));
                    n
                }
            };
            entry.bindings[b.binding as usize].binding = n;
        }
    }
    Ok(Program { vertex: v, fragment: f, samplers, blocks })
}

/// The vertex shader's output a fragment shader's input of a name reads: its own name, but the compatibility
/// profile's `gl_Color` and `gl_SecondaryColor` are the front faces' colours.
fn fed_by(input: &str) -> &str {
    match input {
        "gl_Color" => "gl_FrontColor",
        "gl_SecondaryColor" => "gl_FrontSecondaryColor",
        other => other,
    }
}

/// A vertex shader's position made the device's: GL's clip space has z from −w to w, the device's (Vulkan's and
/// Metal's) from 0 to w, so z becomes (z + w) / 2 wherever the position is written.
pub fn gl_clip(entry: &mut Entry) {
    let Some(position) = entry.outputs.iter().position(|o| o.slot == Slot::Position) else { return };
    let mut body = core::mem::take(&mut entry.body);
    clip_block(entry, &mut body, position as u32);
    entry.body = body;
}

fn clip_block(entry: &mut Entry, block: &mut gpu_ir::Block, position: u32) {
    use gpu_ir::{BinOp, Op, Stmt, Type};
    let mut out = Vec::with_capacity(block.len());
    for s in core::mem::take(block) {
        match s {
            Stmt::Output(n, v) if n == position => {
                let mut let_ = |ty: Type, op: Op, out: &mut Vec<Stmt>| {
                    let x = entry.value(ty);
                    out.push(Stmt::Let(x, op));
                    x
                };
                let z = let_(Type::F32, Op::Extract(v, 2), &mut out);
                let w = let_(Type::F32, Op::Extract(v, 3), &mut out);
                let sum = let_(Type::F32, Op::Binary(BinOp::Add, z, w), &mut out);
                let half = let_(Type::F32, Op::Const(gpu_ir::Scalar::F32, u64::from(0.5f32.to_bits())), &mut out);
                let depth = let_(Type::F32, Op::Binary(BinOp::Mul, sum, half), &mut out);
                let moved = let_(Type::Vector(gpu_ir::Scalar::F32, 4), Op::Insert(v, 2, depth), &mut out);
                out.push(Stmt::Output(n, moved));
            }
            Stmt::If { cond, mut then, mut otherwise } => {
                clip_block(entry, &mut then, position);
                clip_block(entry, &mut otherwise, position);
                out.push(Stmt::If { cond, then, otherwise });
            }
            Stmt::Loop { mut body, mut continuing, break_if } => {
                clip_block(entry, &mut body, position);
                clip_block(entry, &mut continuing, position);
                out.push(Stmt::Loop { body, continuing, break_if });
            }
            other => out.push(other),
        }
    }
    *block = out;
}

/// The compatibility profile's alpha test on a fragment shader of the program's own: the first colour output's alpha
/// compared with `reference` by `func` (`GL_NEVER` to `GL_ALWAYS`), and the fragment discarded where it fails, as GL
/// tests every fragment after its shader runs. Outputs are written as `main` ends, so the test goes before that write.
pub fn alpha_test(entry: &mut Entry, func: u32, reference: f32) {
    use gpu_ir::{Cmp, Op, Stmt, Type};
    let cmp = match func {
        0x0200 => None,
        0x0201 => Some(Cmp::Lt),
        0x0202 => Some(Cmp::Eq),
        0x0203 => Some(Cmp::Le),
        0x0204 => Some(Cmp::Gt),
        0x0205 => Some(Cmp::Ne),
        0x0206 => Some(Cmp::Ge),
        _ => return,
    };
    let Some(color) = entry.outputs.iter().position(|o| o.slot == Slot::Location(0) && o.ty.lanes() == 4) else { return };
    let mut out = Vec::with_capacity(entry.body.len() + 4);
    for s in core::mem::take(&mut entry.body) {
        if let Stmt::Output(n, v) = s
            && n == color as u32
        {
            match cmp {
                None => out.push(Stmt::Discard),
                Some(cmp) => {
                    let alpha = entry.value(Type::F32);
                    out.push(Stmt::Let(alpha, Op::Extract(v, 3)));
                    let r = entry.value(Type::F32);
                    out.push(Stmt::Let(r, Op::Const(gpu_ir::Scalar::F32, u64::from(reference.to_bits()))));
                    let pass = entry.value(Type::BOOL);
                    out.push(Stmt::Let(pass, Op::Compare(cmp, alpha, r)));
                    out.push(Stmt::If { cond: pass, then: Vec::new(), otherwise: alloc::vec![Stmt::Discard] });
                }
            }
        }
        out.push(s);
    }
    entry.body = out;
}

/// A fragment shader's colour outputs kept to the first `colors` locations (a framebuffer with fewer attachments than
/// `gl_FragData` has), the rest and their writes dropped.
pub fn keep_colors(entry: &mut Entry, colors: u32) {
    use gpu_ir::Stmt;
    let keep: Vec<bool> = entry.outputs.iter().map(|o| !matches!(o.slot, Slot::Location(l) if l >= colors)).collect();
    if keep.iter().all(|k| *k) {
        return;
    }
    let mut renumber = Vec::new();
    let mut next = 0u32;
    for &k in &keep {
        renumber.push(if k { Some(next) } else { None });
        next += u32::from(k);
    }
    let mut i = 0;
    entry.outputs.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    fn walk(block: &mut gpu_ir::Block, renumber: &[Option<u32>]) {
        block.retain_mut(|s| match s {
            Stmt::Output(n, _) => match renumber[*n as usize] {
                Some(m) => {
                    *n = m;
                    true
                }
                None => false,
            },
            Stmt::If { then, otherwise, .. } => {
                walk(then, renumber);
                walk(otherwise, renumber);
                true
            }
            Stmt::Loop { body, continuing, .. } => {
                walk(body, renumber);
                walk(continuing, renumber);
                true
            }
            _ => true,
        });
    }
    walk(&mut entry.body, &renumber);
}
