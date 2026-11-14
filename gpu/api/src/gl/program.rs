//! Shaders and programs: GLSL compiled by `gpu-frontend`'s compiler as `glCompileShader` asks, programs linked (their
//! attributes given GL's locations, explicit, bound or the next free; their position made the device's depth range),
//! and their uniforms: each one GL names, its locations, and its values kept in each stage's default block as the
//! program sets them.

use std::collections::HashMap;

use gpu::ir::{Entry, Slot as IoSlot};
use gpu_frontend::glsl::{self, Stage, Ty, types::Base};
use gpu_wire::gl as wire;
use gpu_wire::records;

use super::{ATTRIBS, Gl, INVALID_ENUM, INVALID_OPERATION, INVALID_VALUE, Objects, R, context};

const VERTEX_SHADER: u32 = 0x8b31;
const FRAGMENT_SHADER: u32 = 0x8b30;
const COMPUTE_SHADER: u32 = 0x91b9;

pub(super) struct Shader {
    stage: Stage,
    source: String,
    compiled: Option<glsl::Shader>,
    log: String,
    deleted: bool,
}

pub(super) struct Program {
    shaders: Vec<u32>,
    /// `glBindAttribLocation`'s, for the next link.
    binds: Vec<(String, u32)>,
    pub linked: Option<Linked>,
    log: String,
    deleted: bool,
    validated: bool,
}

/// What a link made: both stages' entries, the uniforms and attributes, and the pipelines made for its draws.
pub(super) struct Linked {
    pub vertex: Entry,
    pub fragment: Entry,
    pub slots: Vec<Uniform>,
    /// How many of `slots` are the program's own: the rest are built-ins.
    pub visible: usize,
    /// Each location's uniform and element.
    pub locations: Vec<(usize, u32)>,
    /// Each attribute's name, first location and type.
    pub attribs: Vec<(String, u32, Ty)>,
    /// The vertex and fragment shaders' default blocks.
    pub blocks: [Vec<u8>; 2],
    /// Each sampler binding's texture unit.
    pub units: Vec<u32>,
    pub pipelines: HashMap<super::draw::PipelineKey, gpu::Pipeline>,
    /// Uniform or buffer blocks, which draws don't bind yet.
    pub has_blocks: bool,
}

/// One uniform as GL names it: its element type, its array size, where each stage keeps it (offset, array stride,
/// matrix stride), and for a sampler, its binding.
pub(super) struct Uniform {
    pub name: String,
    pub ty: Ty,
    pub count: Option<u32>,
    pub at: [Option<(u32, u32, u32)>; 2],
    pub sampler: Option<u32>,
}

impl Gl {
    /// A shader's or program's record, if it's one: whether it was.
    pub(super) fn program_record(&self, o: &mut Objects, current: u64, kind: u32, p: &mut R<'_>, reply: &mut Vec<u8>) -> Result<bool, String> {
        match kind {
            wire::SHADER => {
                let (name, ty) = (p.u32()?, p.u32()?);
                let stage = match ty {
                    VERTEX_SHADER => Stage::Vertex,
                    FRAGMENT_SHADER => Stage::Fragment,
                    COMPUTE_SHADER => Stage::Compute,
                    _ => {
                        context(o, current)?.fail(INVALID_ENUM);
                        return Ok(true);
                    }
                };
                o.shaders.insert(name, Shader { stage, source: String::new(), compiled: None, log: String::new(), deleted: false });
            }
            wire::SHADER_SOURCE => {
                let name = p.u32()?;
                let source = String::from_utf8_lossy(p.bytes()?).into_owned();
                match o.shaders.get_mut(&name) {
                    Some(s) => s.source = source,
                    None => context(o, current)?.fail(INVALID_VALUE),
                }
            }
            wire::COMPILE => {
                let name = p.u32()?;
                match o.shaders.get_mut(&name) {
                    Some(s) => match glsl::compile(&s.source, s.stage, false) {
                        Ok(c) => {
                            s.log = c.log.iter().map(|l| format!("{l}\n")).collect();
                            s.compiled = Some(c);
                        }
                        Err(e) => {
                            s.log = format!("{}\n", e.message);
                            s.compiled = None;
                        }
                    },
                    None => context(o, current)?.fail(INVALID_VALUE),
                }
            }
            wire::DELETE_SHADER => {
                let name = p.u32()?;
                let attached = o.programs.values().any(|p| p.shaders.contains(&name));
                match o.shaders.get_mut(&name) {
                    Some(s) if attached => s.deleted = true,
                    Some(_) => {
                        o.shaders.remove(&name);
                    }
                    None if name == 0 => {}
                    None => context(o, current)?.fail(INVALID_VALUE),
                }
            }
            wire::GET_SHADER => {
                let (name, pname) = (p.u32()?, p.u32()?);
                let v = match o.shaders.get(&name) {
                    Some(s) => match pname {
                        0x8b4f => Some(match s.stage {
                            Stage::Vertex => VERTEX_SHADER,
                            Stage::Fragment => FRAGMENT_SHADER,
                            Stage::Compute => COMPUTE_SHADER,
                        } as i32),
                        0x8b80 => Some(i32::from(s.deleted)),
                        0x8b81 => Some(i32::from(s.compiled.is_some())),
                        0x8b84 => Some(log_length(&s.log)),
                        0x8b88 => Some(if s.source.is_empty() { 0 } else { s.source.len() as i32 + 1 }),
                        _ => None,
                    },
                    None => {
                        let error = if o.programs.contains_key(&name) { INVALID_OPERATION } else { INVALID_VALUE };
                        context(o, current)?.fail(error);
                        Some(0)
                    }
                };
                answer_int(o, current, reply, v)?;
            }
            wire::SHADER_LOG => {
                let name = p.u32()?;
                let log = o.shaders.get(&name).map(|s| s.log.clone()).unwrap_or_default();
                records::put_bytes(reply, log.as_bytes());
            }
            wire::PROGRAM => {
                let name = p.u32()?;
                o.programs.insert(name, Program { shaders: Vec::new(), binds: Vec::new(), linked: None, log: String::new(), deleted: false, validated: false });
            }
            wire::ATTACH | wire::DETACH => {
                let (program, shader) = (p.u32()?, p.u32()?);
                if !o.shaders.contains_key(&shader) {
                    context(o, current)?.fail(INVALID_VALUE);
                    return Ok(true);
                }
                let Some(prog) = o.programs.get_mut(&program) else {
                    context(o, current)?.fail(INVALID_VALUE);
                    return Ok(true);
                };
                let there = prog.shaders.contains(&shader);
                if kind == wire::ATTACH {
                    if there {
                        context(o, current)?.fail(INVALID_OPERATION);
                    } else {
                        prog.shaders.push(shader);
                    }
                } else if !there {
                    context(o, current)?.fail(INVALID_OPERATION);
                } else {
                    prog.shaders.retain(|&s| s != shader);
                    if o.shaders.get(&shader).is_some_and(|s| s.deleted) && !o.programs.values().any(|p| p.shaders.contains(&shader)) {
                        o.shaders.remove(&shader);
                    }
                }
            }
            wire::BIND_ATTRIB => {
                let (program, index) = (p.u32()?, p.u32()?);
                let name = String::from_utf8_lossy(p.bytes()?).into_owned();
                if index as usize >= ATTRIBS || name.starts_with("gl_") {
                    let e = if name.starts_with("gl_") { INVALID_OPERATION } else { INVALID_VALUE };
                    context(o, current)?.fail(e);
                    return Ok(true);
                }
                match o.programs.get_mut(&program) {
                    Some(prog) => {
                        prog.binds.retain(|b| b.0 != name);
                        prog.binds.push((name, index));
                    }
                    None => context(o, current)?.fail(INVALID_VALUE),
                }
            }
            wire::LINK => {
                let name = p.u32()?;
                let Some(prog) = o.programs.get(&name) else {
                    context(o, current)?.fail(INVALID_VALUE);
                    return Ok(true);
                };
                let shaders: Vec<&Shader> = prog.shaders.iter().filter_map(|s| o.shaders.get(s)).collect();
                let result = link(&shaders, &prog.binds);
                let prog = o.programs.get_mut(&name).expect("there");
                match result {
                    Ok(l) => {
                        prog.linked = Some(l);
                        prog.log.clear();
                    }
                    Err(e) => {
                        prog.linked = None;
                        prog.log = format!("{e}\n");
                    }
                }
            }
            wire::VALIDATE => {
                let name = p.u32()?;
                match o.programs.get_mut(&name) {
                    Some(prog) => prog.validated = prog.linked.is_some(),
                    None => context(o, current)?.fail(INVALID_VALUE),
                }
            }
            wire::USE_PROGRAM => {
                let name = p.u32()?;
                let ok = name == 0 || o.programs.get(&name).is_some_and(|p| p.linked.is_some());
                let c = context(o, current)?;
                if ok {
                    let old = std::mem::replace(&mut c.program, name);
                    self.forget_program(o, old);
                } else {
                    c.fail(INVALID_OPERATION);
                }
            }
            wire::DELETE_PROGRAM => {
                let name = p.u32()?;
                if let Some(prog) = o.programs.get_mut(&name) {
                    prog.deleted = true;
                    self.forget_program(o, name);
                }
            }
            wire::GET_PROGRAM => {
                let (name, pname) = (p.u32()?, p.u32()?);
                let v = match o.programs.get(&name) {
                    Some(prog) => {
                        let l = prog.linked.as_ref();
                        match pname {
                            0x8b80 => Some(i32::from(prog.deleted)),
                            0x8b82 => Some(i32::from(l.is_some())),
                            0x8b83 => Some(i32::from(prog.validated)),
                            0x8b84 => Some(log_length(&prog.log)),
                            0x8b85 => Some(prog.shaders.len() as i32),
                            0x8b86 => Some(l.map_or(0, |l| l.visible as i32)),
                            0x8b87 => Some(l.map_or(0, |l| l.slots[..l.visible].iter().map(|s| active_name(s).len() as i32 + 1).max().unwrap_or(0))),
                            0x8b89 => Some(l.map_or(0, |l| l.attribs.len() as i32)),
                            0x8b8a => Some(l.map_or(0, |l| l.attribs.iter().map(|a| a.0.len() as i32 + 1).max().unwrap_or(0))),
                            // ACTIVE_UNIFORM_BLOCKS.
                            0x8a36 => Some(0),
                            _ => None,
                        }
                    }
                    None => {
                        let error = if o.shaders.contains_key(&name) { INVALID_OPERATION } else { INVALID_VALUE };
                        context(o, current)?.fail(error);
                        Some(0)
                    }
                };
                answer_int(o, current, reply, v)?;
            }
            wire::PROGRAM_LOG => {
                let name = p.u32()?;
                let log = o.programs.get(&name).map(|p| p.log.clone()).unwrap_or_default();
                records::put_bytes(reply, log.as_bytes());
            }
            wire::UNIFORM_LOCATION | wire::ATTRIB_LOCATION => {
                let program = p.u32()?;
                let name = String::from_utf8_lossy(p.bytes()?).into_owned();
                let linked = o.programs.get(&program).and_then(|p| p.linked.as_ref());
                let at = match linked {
                    Some(l) if kind == wire::UNIFORM_LOCATION => uniform_location(l, &name),
                    Some(l) => l.attribs.iter().find(|a| a.0 == name).map_or(-1, |a| a.1 as i32),
                    None => {
                        context(o, current)?.fail(INVALID_OPERATION);
                        -1
                    }
                };
                records::put_u32(reply, at as u32);
            }
            wire::ACTIVE_UNIFORM | wire::ACTIVE_ATTRIB => {
                let (program, index) = (p.u32()?, p.u32()?);
                let linked = o.programs.get(&program).and_then(|p| p.linked.as_ref());
                let found = linked.and_then(|l| {
                    if kind == wire::ACTIVE_UNIFORM {
                        l.slots[..l.visible].get(index as usize).map(|s| (s.count.unwrap_or(1), gl_type(&s.ty), active_name(s)))
                    } else {
                        l.attribs.get(index as usize).map(|a| (1, gl_type(&a.2), a.0.clone()))
                    }
                });
                match found {
                    Some((size, ty, name)) => {
                        records::put_u32(reply, size);
                        records::put_u32(reply, ty);
                        records::put_bytes(reply, name.as_bytes());
                    }
                    None => {
                        context(o, current)?.fail(INVALID_VALUE);
                        records::put_u32(reply, 0);
                        records::put_u32(reply, 0);
                        records::put_bytes(reply, &[]);
                    }
                }
            }
            wire::UNIFORM => {
                let [location, value_kind, cols, rows, count, transpose] = p.words::<6>()?;
                let words: Vec<u32> = p.bytes()?.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
                let c = context(o, current)?;
                let program = c.program;
                if location as i32 == -1 {
                    return Ok(true);
                }
                let error = match o.programs.get_mut(&program).and_then(|p| p.linked.as_mut()) {
                    Some(l) => set_uniform(l, location, value_kind, cols, rows, count, transpose != 0, &words).err(),
                    None => Some(INVALID_OPERATION),
                };
                if let Some(e) = error {
                    context(o, current)?.fail(e);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A program deleted while it was in use goes when nothing uses it.
    fn forget_program(&self, o: &mut Objects, name: u32) {
        if o.programs.get(&name).is_some_and(|p| p.deleted) && !o.contexts.values().any(|c| c.program == name) {
            let shaders = o.programs.remove(&name).map(|p| p.shaders).unwrap_or_default();
            for s in shaders {
                if o.shaders.get(&s).is_some_and(|s| s.deleted) && !o.programs.values().any(|p| p.shaders.contains(&s)) {
                    o.shaders.remove(&s);
                }
            }
        }
    }
}

fn answer_int(o: &mut Objects, current: u64, reply: &mut Vec<u8>, v: Option<i32>) -> Result<(), String> {
    if v.is_none() {
        context(o, current)?.fail(INVALID_ENUM);
    }
    records::put_u32(reply, v.unwrap_or(0) as u32);
    Ok(())
}

fn log_length(log: &str) -> i32 {
    if log.is_empty() { 0 } else { log.len() as i32 + 1 }
}

/// A program linked from its shaders: one vertex and one fragment shader, both compiled.
fn link(shaders: &[&Shader], binds: &[(String, u32)]) -> Result<Linked, String> {
    let one = |stage: Stage| -> Result<&glsl::Shader, String> {
        let mut of = shaders.iter().filter(|s| s.stage == stage);
        let s = of.next().ok_or_else(|| format!("error: no {stage:?} shader is attached"))?;
        if of.next().is_some() {
            return Err(format!("error: two {stage:?} shaders, which this linker doesn't join yet"));
        }
        s.compiled.as_ref().ok_or_else(|| format!("error: the {stage:?} shader didn't compile"))
    };
    let (vs, fs) = (one(Stage::Vertex)?, one(Stage::Fragment)?);
    let program = glsl::link(vs, fs).map_err(|e| e.message)?;
    let (mut vertex, mut fragment) = (program.vertex, program.fragment);
    // Attributes: an explicit location, else a bound one, else the first free.
    let mut used: Vec<u32> = Vec::new();
    let mut attribs = Vec::new();
    let fixed: Vec<(usize, u32)> = vs.interface.inputs.iter().enumerate().filter_map(|(i, v)| v.location.or_else(|| binds.iter().find(|b| b.0 == v.name).map(|b| b.1)).map(|l| (i, l))).collect();
    for &(i, l) in &fixed {
        used.extend(l..l + vs.interface.inputs[i].io.len() as u32);
    }
    for (i, v) in vs.interface.inputs.iter().enumerate() {
        let n = v.io.len() as u32;
        let first = match fixed.iter().find(|f| f.0 == i) {
            Some(&(_, l)) => l,
            None => {
                let mut l = 0;
                while (l..l + n).any(|x| used.contains(&x)) {
                    l += 1;
                }
                used.extend(l..l + n);
                l
            }
        };
        if first + n > ATTRIBS as u32 {
            return Err(format!("error: attribute {} needs locations past the {ATTRIBS} there are", v.name));
        }
        for (k, &io) in v.io.iter().enumerate() {
            vertex.inputs[io as usize].slot = IoSlot::Location(first + k as u32);
        }
        attribs.push((v.name.clone(), first, v.ty.clone()));
    }
    glsl::gl_clip(&mut vertex);
    glsl::keep_colors(&mut fragment, 1);
    // Uniforms, both stages' merged by name.
    let mut slots: Vec<Uniform> = Vec::new();
    for (stage, shader) in [vs, fs].into_iter().enumerate() {
        for u in &shader.interface.uniforms {
            let sampler = u.binding.map(|_| program.samplers.iter().find(|s| s.0 == u.name).map_or(0, |s| s.1));
            let at = Some((u.offset, u.stride, u.matrix_stride));
            match slots.iter_mut().find(|s| s.name == u.name) {
                Some(s) => {
                    if s.ty != u.ty || s.count != u.count {
                        return Err(format!("error: uniform {} is {} in one shader and {} in the other", u.name, s.ty, u.ty));
                    }
                    if sampler.is_none() {
                        s.at[stage] = at;
                    }
                }
                None => {
                    let mut s = Uniform { name: u.name.clone(), ty: u.ty.clone(), count: u.count, at: [None, None], sampler };
                    if sampler.is_none() {
                        s.at[stage] = at;
                    }
                    slots.push(s);
                }
            }
        }
    }
    // The program's own uniforms first; the compatibility profile's built-ins after, listed and found by name never.
    slots.sort_by_key(|s| s.name.starts_with("gl_"));
    let visible = slots.iter().filter(|s| !s.name.starts_with("gl_")).count();
    let locations = slots.iter().enumerate().flat_map(|(i, s)| (0..s.count.unwrap_or(1)).map(move |e| (i, e))).collect();
    let block = |s: &glsl::Shader| vec![0u8; s.interface.default_block.map_or(0, |b| b.1) as usize];
    // Samplers with `layout(binding = n)` start on unit n.
    let mut units = vec![0; program.samplers.len()];
    for shader in [vs, fs] {
        for u in &shader.interface.uniforms {
            if let (Some((_, Some(unit))), Some(s)) = (u.binding, program.samplers.iter().find(|s| s.0 == u.name)) {
                units[s.1 as usize] = unit;
            }
        }
    }
    let has_blocks = !vs.interface.blocks.is_empty() || !fs.interface.blocks.is_empty();
    Ok(Linked { vertex, fragment, slots, visible, locations, attribs, blocks: [block(vs), block(fs)], units, pipelines: HashMap::new(), has_blocks })
}

/// A uniform's name as GL lists it: an array's with `[0]`.
/// A program made from GLSL here (fixed function's), compiled and linked.
pub(super) fn program_from(vs: &str, fs: &str) -> Result<Program, String> {
    let make = |stage: Stage, source: &str| -> Result<Shader, String> {
        let compiled = glsl::compile(source, stage, false).map_err(|e| e.message)?;
        Ok(Shader { stage, source: source.to_string(), compiled: Some(compiled), log: String::new(), deleted: false })
    };
    let (v, f) = (make(Stage::Vertex, vs)?, make(Stage::Fragment, fs)?);
    let linked = link(&[&v, &f], &[])?;
    Ok(Program { shaders: Vec::new(), binds: Vec::new(), linked: Some(linked), log: String::new(), deleted: false, validated: false })
}

fn active_name(s: &Uniform) -> String {
    if s.count.is_some() { format!("{}[0]", s.name) } else { s.name.clone() }
}

/// `glGetUniformLocation`: a uniform's name, an array's alone or with `[0]` (its first element's), or an element's.
fn uniform_location(l: &Linked, name: &str) -> i32 {
    let at = |slot: usize, elem: u32| l.locations.iter().position(|&(s, e)| s == slot && e == elem).map_or(-1, |p| p as i32);
    if name.starts_with("gl_") {
        return -1;
    }
    if let Some(i) = l.slots.iter().position(|s| s.name == name) {
        return at(i, 0);
    }
    if let Some((base, index)) = name.strip_suffix(']').and_then(|n| n.rsplit_once('[')) {
        let Ok(k) = index.parse::<u32>() else { return -1 };
        if let Some(i) = l.slots.iter().position(|s| s.name == base && s.count.is_some_and(|c| k < c)) {
            return at(i, k);
        }
    }
    let first = format!("{name}[0]");
    l.slots.iter().position(|s| s.name == first).map_or(-1, |i| at(i, 0))
}

/// `glUniform*`: `count` elements from `location` on, each `cols` by `rows` values of a kind, checked against the
/// uniform's type and written where each stage keeps it.
#[allow(clippy::too_many_arguments)]
fn set_uniform(l: &mut Linked, location: u32, kind: u32, cols: u32, rows: u32, count: u32, transpose: bool, words: &[u32]) -> Result<(), u32> {
    let per = (cols * rows) as usize;
    if words.len() < per * count as usize {
        return Err(INVALID_VALUE);
    }
    for i in 0..count {
        let Some(&(slot, elem)) = l.locations.get((location + i) as usize) else {
            // Past an array's end: what's past is left, as GL does.
            if i > 0 {
                return Ok(());
            }
            return Err(INVALID_OPERATION);
        };
        let s = &l.slots[slot];
        let (shape_cols, shape_rows, base) = match &s.ty {
            Ty::Scalar(b) => (1, 1, *b),
            Ty::Vector(b, n) => (u32::from(*n), 1, *b),
            Ty::Matrix(c, r) => (u32::from(*c), u32::from(*r), Base::Float),
            Ty::Sampler(_) => (1, 1, Base::Int),
            _ => return Err(INVALID_OPERATION),
        };
        if (shape_cols, shape_rows) != (cols, rows) {
            return Err(INVALID_OPERATION);
        }
        let fits = match base {
            Base::Float => kind == wire::FLOAT,
            Base::Int => kind == wire::INT,
            Base::Uint => kind == wire::UINT,
            Base::Bool => true,
        };
        if !fits || (transpose && rows == 1) {
            return Err(INVALID_OPERATION);
        }
        let given = &words[i as usize * per..(i as usize + 1) * per];
        if let Some(n) = s.sampler {
            l.units[n as usize] = given[0];
            continue;
        }
        let at = s.at;
        for (stage, at) in at.iter().enumerate() {
            let Some((offset, stride, matrix_stride)) = *at else { continue };
            let block = &mut l.blocks[stage];
            let base_at = (offset + elem * stride) as usize;
            for col in 0..cols {
                for row in 0..rows {
                    let w = if transpose { given[(row * cols + col) as usize] } else { given[(col * rows + row) as usize] };
                    // Booleans are words, 0 or 1, whatever kind they were given as.
                    let w = if base == Base::Bool {
                        u32::from(if kind == wire::FLOAT { f32::from_bits(w) != 0.0 } else { w != 0 })
                    } else {
                        w
                    };
                    // A vector's components are words in a row; a matrix's columns are a matrix stride apart.
                    let at = base_at + if rows == 1 { col * 4 } else { col * matrix_stride + row * 4 } as usize;
                    if let Some(dst) = block.get_mut(at..at + 4) {
                        dst.copy_from_slice(&w.to_le_bytes());
                    }
                }
            }
        }
    }
    Ok(())
}

/// GL's name for a type, as `glGetActiveUniform` gives it.
fn gl_type(ty: &Ty) -> u32 {
    use gpu_frontend::glsl::types::Dim;
    match ty {
        Ty::Scalar(Base::Float) => 0x1406,
        Ty::Scalar(Base::Int) => 0x1404,
        Ty::Scalar(Base::Uint) => 0x1405,
        Ty::Scalar(Base::Bool) => 0x8b56,
        Ty::Vector(Base::Float, n) => 0x8b50 + u32::from(*n) - 2,
        Ty::Vector(Base::Int, n) => 0x8b53 + u32::from(*n) - 2,
        Ty::Vector(Base::Uint, n) => 0x8dc6 + u32::from(*n) - 2,
        Ty::Vector(Base::Bool, n) => 0x8b57 + u32::from(*n) - 2,
        Ty::Matrix(2, 2) => 0x8b5a,
        Ty::Matrix(3, 3) => 0x8b5b,
        Ty::Matrix(4, 4) => 0x8b5c,
        Ty::Matrix(2, 3) => 0x8b65,
        Ty::Matrix(2, 4) => 0x8b66,
        Ty::Matrix(3, 2) => 0x8b67,
        Ty::Matrix(3, 4) => 0x8b68,
        Ty::Matrix(4, 2) => 0x8b69,
        Ty::Matrix(4, 3) => 0x8b6a,
        Ty::Sampler(s) => match s.dim {
            Dim::D2 => 0x8b5e,
            Dim::D3 => 0x8b5f,
            Dim::Cube => 0x8b60,
            Dim::Rect => 0x8b63,
        },
        _ => 0,
    }
}
