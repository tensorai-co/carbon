//! Buffers, vertex arrays, textures and draws.
//!
//! A draw's vertex inputs come from its vertex array: an enabled attribute's array straight from its buffer when the
//! device reads its format as the shader's input type, else converted into the batch's arena (the program's own
//! arrays, `GL_SHORT`s, BGRA bytes and the rest); an attribute that's off is its generic value, once an instance. Its
//! pipeline is made once for its program and the state that shapes one (formats, topology, culling, depth, blending,
//! write mask) and kept with the program.

use std::sync::atomic::{AtomicU64, Ordering};

use gpu::{
    Address, Attribute, Bind, Blend, BlendOp, ColorAttachment, ColorTarget, Command, Compare, Cull, DepthAttachment, DepthState, Draw, Factor, Filter, Format,
    FrontFace, GraphicsDesc, Image, ImageDesc, IndexType, Load, Range, Rect, SamplerDesc, Sub, Submit, Topology, VertexBuffer, VertexFormat, Viewport,
};
use gpu::ir::{Scalar, Slot};
use gpu_frontend::glsl;
use gpu_wire::gl as wire;

use super::{
    ATTRIBS, BLEND, CULL_FACE, DEPTH_TEST, Gl, INVALID_ENUM, INVALID_FRAMEBUFFER_OPERATION, INVALID_OPERATION, INVALID_VALUE, LONGEST, Objects, R, SCISSOR_TEST, Step, UNITS,
    context, device_error, framebuffer_of, intersect,
};
use crate::vulkan::Guest;

const ARRAY_BUFFER: u32 = 0x8892;
const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const TEXTURE_2D: u32 = 0x0de1;
const BYTE: u32 = 0x1400;
const UNSIGNED_BYTE: u32 = 0x1401;
const SHORT: u32 = 0x1402;
const UNSIGNED_SHORT: u32 = 0x1403;
const INT: u32 = 0x1404;
const UNSIGNED_INT: u32 = 0x1405;
const FLOAT: u32 = 0x1406;
const HALF_FLOAT: u32 = 0x140b;
const BGRA: u32 = 0x80e1;

pub(super) struct Buffer {
    memory: Option<gpu::Memory>,
    size: u64,
}

/// A vertex array: each attribute's array, and the element buffer.
#[derive(Clone, Default)]
pub(super) struct Vao {
    attribs: [Attrib; ATTRIBS],
    pub elements: u32,
}

#[derive(Clone, Copy)]
struct Attrib {
    enabled: bool,
    /// 1 to 4, or `GL_BGRA`.
    size: u32,
    kind: u32,
    normalized: bool,
    integer: bool,
    stride: u32,
    /// An offset in `buffer`, or with no buffer, the program's own address.
    pointer: u64,
    buffer: u32,
    divisor: u32,
}

impl Default for Attrib {
    fn default() -> Attrib {
        Attrib { enabled: false, size: 4, kind: FLOAT, normalized: false, integer: false, stride: 0, pointer: 0, buffer: 0, divisor: 0 }
    }
}

impl Attrib {
    fn components(&self) -> u32 {
        if self.size == BGRA { 4 } else { self.size }
    }

    /// The bytes of one element.
    fn bytes(&self) -> u32 {
        self.components() * kind_size(self.kind)
    }

    fn stride(&self) -> u32 {
        if self.stride == 0 { self.bytes() } else { self.stride }
    }
}

fn kind_size(kind: u32) -> u32 {
    match kind {
        BYTE | UNSIGNED_BYTE => 1,
        SHORT | UNSIGNED_SHORT | HALF_FLOAT => 2,
        _ => 4,
    }
}

pub(super) struct Texture {
    image: Option<Image>,
    width: u32,
    height: u32,
    format: Format,
    /// Each level defined, a bit each.
    defined: u32,
    min: u32,
    mag: u32,
    wrap: [u32; 2],
    max_level: u32,
}

impl Texture {
    /// A level's image, format and size, if the texture has that level.
    pub(super) fn level(&self, level: u32) -> Option<(Image, Format, u32, u32)> {
        let image = self.image?;
        (level < ImageDesc::most_levels(self.width, self.height)).then(|| (image, self.format, (self.width >> level).max(1), (self.height >> level).max(1)))
    }
}

impl Default for Texture {
    fn default() -> Texture {
        // NEAREST_MIPMAP_LINEAR, LINEAR, REPEAT.
        Texture { image: None, width: 0, height: 0, format: Format::Rgba8Unorm, defined: 0, min: 0x2702, mag: 0x2601, wrap: [0x2901; 2], max_level: 1000 }
    }
}

/// A sampler, by what it is.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SamplerKey {
    mag: Filter,
    min: Filter,
    mipmap: Filter,
    address: [Address; 2],
    /// The highest level it reads.
    top: u32,
}

/// What shapes a pipeline beside its program.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct PipelineKey {
    attributes: Vec<(u32, VertexFormat, u32, bool)>,
    topology: Topology,
    cull: Cull,
    front: FrontFace,
    depth: Option<DepthState>,
    depth_attachment: bool,
    /// The colour attachment's format.
    color: Format,
    /// The compatibility profile's alpha test, on a program of the program's own: its function and reference's bits.
    alpha: Option<(u32, u32)>,
    blend: Option<Blend>,
    write: [bool; 4],
}

/// Where a draw's memory is: the device's already, or the batch's arena (offset, size).
#[derive(Clone, Copy)]
pub(super) enum Src {
    Device(Range),
    Arena(u64, u64),
}

/// What's waiting in a batch.
pub(super) enum Item {
    Command(Command),
    Draw(Box<Planned>),
}

pub(super) struct Planned {
    draw: Draw,
    buffers: Vec<(u32, u32, Src)>,
    images: Vec<(u32, u32, Image, gpu::Sampler)>,
    vertex: Vec<Src>,
    index: Option<(Src, IndexType)>,
}

/// Says something this GL doesn't do yet, once for each thing, on the host's stderr.
pub(super) fn not_yet(what: &str, bit: u64) {
    static SAID: AtomicU64 = AtomicU64::new(0);
    if SAID.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        eprintln!("carbon gl: {what} isn't drawn yet");
    }
}

impl Gl {
    /// A buffer's, vertex array's, texture's or draw's record, if it's one: whether it was.
    pub(super) fn draw_record(&self, o: &mut Objects, current: u64, kind: u32, p: &mut R<'_>, guest: &mut dyn Guest) -> Result<bool, String> {
        match kind {
            wire::BIND_BUFFER => {
                let (target, name) = (p.u32()?, p.u32()?);
                if name != 0 {
                    o.buffers.entry(name).or_insert(Buffer { memory: None, size: 0 });
                }
                let c = context(o, current)?;
                if target == ELEMENT_ARRAY_BUFFER {
                    c.vao_mut().elements = name;
                } else {
                    c.bindings.insert(target, name);
                }
            }
            wire::BUFFER_DATA => {
                let (target, size, _usage, given) = (p.u32()?, p.u64()?, p.u32()?, p.u32()? != 0);
                let data = p.bytes()?;
                let Some(name) = self.bound_buffer(o, current, target)? else { return Ok(true) };
                if given && data.len() as u64 != size {
                    return Err("a buffer's data isn't its size".into());
                }
                self.flush(o)?;
                let old = o.buffers.get_mut(&name).and_then(|b| b.memory.take());
                let bytes = if given { data.to_vec() } else { vec![0; size as usize] };
                let memory = self
                    .gpu
                    .run(move |d| -> Result<gpu::Memory, gpu::Error> {
                        if let Some(m) = old {
                            d.free(m);
                        }
                        let m = d.alloc((bytes.len() as u64).max(4))?;
                        d.write(m, 0, &bytes)?;
                        Ok(m)
                    })
                    .map_err(device_error)?
                    .map_err(device_error)?;
                o.buffers.insert(name, Buffer { memory: Some(memory), size });
            }
            wire::BUFFER_SUB_DATA => {
                let (target, offset) = (p.u32()?, p.u64()?);
                let data = p.bytes()?.to_vec();
                let Some(name) = self.bound_buffer(o, current, target)? else { return Ok(true) };
                let (memory, size) = o.buffers.get(&name).map_or((None, 0), |b| (b.memory, b.size));
                let Some(memory) = memory.filter(|_| offset + data.len() as u64 <= size) else {
                    context(o, current)?.fail(INVALID_VALUE);
                    return Ok(true);
                };
                self.flush(o)?;
                self.gpu.run(move |d| d.write(memory, offset, &data)).map_err(device_error)?.map_err(device_error)?;
            }
            wire::DELETE_BUFFER => {
                let name = p.u32()?;
                if let Some(b) = o.buffers.remove(&name) {
                    self.flush(o)?;
                    if let Some(m) = b.memory {
                        self.gpu.run(move |d| d.free(m)).map_err(device_error)?;
                    }
                    for c in o.contexts.values_mut() {
                        c.bindings.retain(|_, n| *n != name);
                        for v in c.vaos.values_mut() {
                            if v.elements == name {
                                v.elements = 0;
                            }
                        }
                    }
                }
            }
            wire::BIND_VERTEX_ARRAY => {
                let name = p.u32()?;
                let c = context(o, current)?;
                c.vaos.entry(name).or_default();
                c.vao = name;
            }
            wire::DELETE_VERTEX_ARRAY => {
                let name = p.u32()?;
                let c = context(o, current)?;
                if name != 0 {
                    c.vaos.remove(&name);
                    if c.vao == name {
                        c.vao = 0;
                    }
                }
            }
            wire::ATTRIB_POINTER => {
                let [index, size, kind, normalized, integer, stride] = p.words::<6>()?;
                let pointer = p.u64()?;
                let c = context(o, current)?;
                let ok_size = (1..=4).contains(&size) || (size == BGRA && kind == UNSIGNED_BYTE);
                if index as usize >= ATTRIBS || !ok_size || stride > 2048 {
                    c.fail(INVALID_VALUE);
                    return Ok(true);
                }
                if !matches!(kind, BYTE | UNSIGNED_BYTE | SHORT | UNSIGNED_SHORT | INT | UNSIGNED_INT | FLOAT | HALF_FLOAT) {
                    c.fail(INVALID_ENUM);
                    return Ok(true);
                }
                let buffer = c.bindings.get(&ARRAY_BUFFER).copied().unwrap_or(0);
                let a = &mut c.vao_mut().attribs[index as usize];
                *a = Attrib { size, kind, normalized: normalized != 0, integer: integer != 0, stride, pointer, buffer, ..*a };
            }
            wire::ENABLE_ATTRIB => {
                let (index, on) = (p.u32()?, p.u32()? != 0);
                let c = context(o, current)?;
                match c.vao_mut().attribs.get_mut(index as usize) {
                    Some(a) => a.enabled = on,
                    None => c.fail(INVALID_VALUE),
                }
            }
            wire::ATTRIB_VALUE => {
                let index = p.u32()?;
                let v = p.words::<4>()?;
                let c = context(o, current)?;
                match c.attrib_values.get_mut(index as usize) {
                    Some(a) => *a = v,
                    None => c.fail(INVALID_VALUE),
                }
            }
            wire::ATTRIB_DIVISOR => {
                let (index, divisor) = (p.u32()?, p.u32()?);
                let c = context(o, current)?;
                match c.vao_mut().attribs.get_mut(index as usize) {
                    Some(a) => a.divisor = divisor,
                    None => c.fail(INVALID_VALUE),
                }
            }
            wire::ACTIVE_TEXTURE => {
                let unit = p.u32()?;
                let c = context(o, current)?;
                if (unit as usize) < UNITS {
                    c.unit = unit;
                } else {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::BIND_TEXTURE => {
                let (target, name) = (p.u32()?, p.u32()?);
                if target != TEXTURE_2D {
                    // Cube maps, 3D and array textures: kept nowhere yet, and said so once.
                    not_yet("a texture other than GL_TEXTURE_2D", 1);
                    return Ok(true);
                }
                if name != 0 {
                    o.textures.entry(name).or_default();
                }
                let c = context(o, current)?;
                let unit = c.unit as usize;
                c.units[unit] = name;
            }
            wire::TEX_IMAGE | wire::TEX_SUB_IMAGE => self.tex_image(o, current, kind, p)?,
            wire::TEX_PARAMETER => {
                let [target, pname, bits, float] = p.words::<4>()?;
                if target != TEXTURE_2D {
                    return Ok(true);
                }
                let value = if float != 0 { f32::from_bits(bits) as i64 as u32 } else { bits };
                let c = context(o, current)?;
                let name = c.units[c.unit as usize];
                let Some(t) = o.textures.get_mut(&name) else {
                    context(o, current)?.fail(INVALID_OPERATION);
                    return Ok(true);
                };
                let wrap_ok = matches!(value, 0x2901 | 0x812f | 0x8370 | 0x812d | 0x2900);
                let ok = match pname {
                    0x2801 if matches!(value, 0x2600 | 0x2601 | 0x2700..=0x2703) => {
                        t.min = value;
                        true
                    }
                    0x2800 if matches!(value, 0x2600 | 0x2601) => {
                        t.mag = value;
                        true
                    }
                    0x2802 if wrap_ok => {
                        t.wrap[0] = value;
                        true
                    }
                    0x2803 if wrap_ok => {
                        t.wrap[1] = value;
                        true
                    }
                    0x813d => {
                        t.max_level = value;
                        true
                    }
                    // WRAP_R, BASE_LEVEL, MIN_LOD, MAX_LOD, anisotropy, comparison, swizzles, border colour, LOD bias.
                    0x8072 | 0x813c | 0x813a | 0x813b | 0x84fe | 0x884c | 0x884d | 0x8e42..=0x8e46 | 0x1004 | 0x8501 => true,
                    _ => false,
                };
                if !ok {
                    context(o, current)?.fail(INVALID_ENUM);
                }
            }
            wire::GENERATE_MIPMAP => {
                let _target = p.u32()?;
                let c = context(o, current)?;
                let name = c.units[c.unit as usize];
                match o.textures.get_mut(&name).and_then(|t| t.image.map(|i| (t, i))) {
                    // A depth texture's levels aren't made: the device makes only colour images'.
                    Some((t, _)) if t.format.is_depth() || !t.format.is_renderable() => t.defined |= 1,
                    Some((t, image)) => {
                        t.defined = u32::MAX;
                        self.steps(o, vec![Step::Mips(image)])?;
                    }
                    None => context(o, current)?.fail(INVALID_OPERATION),
                }
            }
            wire::DELETE_TEXTURE => {
                let name = p.u32()?;
                if let Some(t) = o.textures.remove(&name) {
                    self.flush(o)?;
                    if let Some(i) = t.image {
                        self.gpu.run(move |d| d.free_image(i)).map_err(device_error)?;
                    }
                    for c in o.contexts.values_mut() {
                        for u in &mut c.units {
                            if *u == name {
                                *u = 0;
                            }
                        }
                    }
                }
            }
            wire::DRAW_ARRAYS => {
                let [mode, first, count, instances] = p.words::<4>()?;
                self.draw(o, current, mode, first, count, instances, None, None, guest)?;
            }
            wire::RECT => {
                let r = p.words::<4>()?.map(f32::from_bits);
                let corners = [r[0], r[1], 0.0, 1.0, r[2], r[1], 0.0, 1.0, r[2], r[3], 0.0, 1.0, r[0], r[3], 0.0, 1.0];
                self.draw(o, current, 6, 0, 4, 1, None, Some(corners), guest)?;
            }
            wire::DRAW_ELEMENTS => {
                let [mode, count, kind, instances, base] = p.words::<5>()?;
                let at = p.u64()?;
                if !matches!(kind, UNSIGNED_BYTE | UNSIGNED_SHORT | UNSIGNED_INT) {
                    context(o, current)?.fail(INVALID_ENUM);
                    return Ok(true);
                }
                self.draw(o, current, mode, 0, count, instances, Some((kind, at, base as i32)), None, guest)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The buffer bound at a target, or the error that there isn't one.
    fn bound_buffer(&self, o: &mut Objects, current: u64, target: u32) -> Result<Option<u32>, String> {
        let c = context(o, current)?;
        let name = if target == ELEMENT_ARRAY_BUFFER { c.vaos.get(&c.vao).map_or(0, |v| v.elements) } else { c.bindings.get(&target).copied().unwrap_or(0) };
        if name == 0 {
            c.fail(INVALID_OPERATION);
            return Ok(None);
        }
        Ok(Some(name))
    }

    fn tex_image(&self, o: &mut Objects, current: u64, kind: u32, p: &mut R<'_>) -> Result<(), String> {
        let sub = kind == wire::TEX_SUB_IMAGE;
        let (target, level) = (p.u32()?, p.u32()?);
        let (internal, x, y) = if sub { (0, p.u32()?, p.u32()?) } else { (p.u32()?, 0, 0) };
        let (width, height, format, ty) = (p.u32()?, p.u32()?, p.u32()?, p.u32()?);
        let given = if sub { true } else { p.u32()? != 0 };
        let data = p.bytes()?;
        if target != TEXTURE_2D {
            not_yet("a texture other than GL_TEXTURE_2D", 1);
            return Ok(());
        }
        let c = context(o, current)?;
        let name = c.units[c.unit as usize];
        if name == 0 || !o.textures.contains_key(&name) {
            context(o, current)?.fail(INVALID_OPERATION);
            return Ok(());
        }
        if width > gpu::MAX_IMAGE || height > gpu::MAX_IMAGE || level > 14 {
            context(o, current)?.fail(INVALID_VALUE);
            return Ok(());
        }
        let converted = if given { texels(format, ty, internal, width, height, data) } else { texels(format, ty, internal, 0, 0, &[]) };
        let (fmt, bytes) = match converted {
            Ok(x) => x,
            Err(e) => {
                context(o, current)?.fail(e);
                return Ok(());
            }
        };
        let t = o.textures.get_mut(&name).expect("there");
        if !sub {
            // Level 0 says the texture's size and format: an image of every level is made for it.
            if level == 0 && (t.image.is_none() || (t.width, t.height, t.format) != (width, height, fmt)) {
                if width == 0 || height == 0 {
                    return Ok(());
                }
                let old = t.image.take();
                let levels = ImageDesc::most_levels(width, height);
                self.flush(o)?;
                let image = self
                    .gpu
                    .run(move |d| -> Result<Image, gpu::Error> {
                        if let Some(i) = old {
                            d.free_image(i);
                        }
                        d.image(ImageDesc { format: fmt, width, height, levels, layers: 1 })
                    })
                    .map_err(device_error)?
                    .map_err(device_error)?;
                let t = o.textures.get_mut(&name).expect("there");
                *t = Texture { image: Some(image), width, height, format: fmt, defined: 0, ..*t };
            }
        }
        let t = o.textures.get_mut(&name).expect("there");
        let Some(image) = t.image else {
            context(o, current)?.fail(INVALID_OPERATION);
            return Ok(());
        };
        let (lw, lh) = ((t.width >> level).max(1), (t.height >> level).max(1));
        let fits = if sub { x + width <= lw && y + height <= lh } else { (width, height) == (lw, lh) && fmt == t.format };
        if !fits || level >= ImageDesc::most_levels(t.width, t.height) {
            context(o, current)?.fail(if sub { INVALID_VALUE } else { INVALID_OPERATION });
            return Ok(());
        }
        if !sub {
            t.defined |= 1 << level;
        }
        if width == 0 || height == 0 {
            return Ok(());
        }
        let bytes = if given { bytes } else { vec![0; (width * height * fmt.bytes()) as usize] };
        self.steps(o, vec![Step::Upload { bytes, to: image, level, region: Rect { x, y, width, height } }])?;
        Ok(())
    }

    /// The sampler a texture's parameters make, made once.
    fn sampler(&self, o: &mut Objects, t: &Texture) -> Result<gpu::Sampler, String> {
        let filter = |f: u32| if matches!(f, 0x2601 | 0x2701 | 0x2703) { Filter::Linear } else { Filter::Nearest };
        let address = |w: u32| match w {
            0x2901 => Address::Repeat,
            0x8370 => Address::MirroredRepeat,
            0x812d => Address::ClampToBorder,
            _ => Address::ClampToEdge,
        };
        let mipmapped = matches!(t.min, 0x2700..=0x2703);
        // The levels it may read: the first only without mipmaps, else as far as they're defined in a row.
        let defined = (t.defined | 1).trailing_ones().saturating_sub(1);
        let top = if mipmapped { defined.min(t.max_level) } else { 0 };
        let key = SamplerKey { mag: filter(t.mag), min: filter(t.min), mipmap: if matches!(t.min, 0x2702 | 0x2703) { Filter::Linear } else { Filter::Nearest }, address: [address(t.wrap[0]), address(t.wrap[1])], top };
        if let Some(&s) = o.samplers.get(&key) {
            return Ok(s);
        }
        let mut desc = SamplerDesc::new(key.mag, key.min, key.address);
        desc.mipmap = key.mipmap;
        desc.lod = [0.0, top as f32];
        let s = self.gpu.run(move |d| d.sampler(desc)).map_err(device_error)?.map_err(device_error)?;
        o.samplers.insert(key, s);
        Ok(s)
    }

    /// What an incomplete texture reads: (0, 0, 0, 1).
    fn black(&self, o: &mut Objects) -> Result<Image, String> {
        if let Some(i) = o.black {
            return Ok(i);
        }
        let i = self.gpu.run(|d| d.image(ImageDesc::new(Format::Rgba8Unorm, 1, 1))).map_err(device_error)?.map_err(device_error)?;
        self.steps(o, vec![Step::Upload { bytes: vec![0, 0, 0, 255], to: i, level: 0, region: Rect { x: 0, y: 0, width: 1, height: 1 } }])?;
        o.black = Some(i);
        Ok(i)
    }

    /// `glDrawArrays` (`elements` None) or `glDrawElements` (their type, where they are, the base vertex).
    #[allow(clippy::too_many_arguments)]
    /// `rect`: immediate mode's four corners (`glRectf`), each a `vec4`: the position, with every other attribute its
    /// current value.
    fn draw(&self, o: &mut Objects, current: u64, mode: u32, first: u32, count: u32, instances: u32, elements: Option<(u32, u64, i32)>, rect: Option<[f32; 16]>, guest: &mut dyn Guest) -> Result<(), String> {
        let Some(fb) = framebuffer_of(o, current, false) else {
            let c = context(o, current)?;
            if c.draw_fbo != 0 {
                c.fail(INVALID_FRAMEBUFFER_OPERATION);
            }
            return Ok(());
        };
        let topology = match mode {
            4 => Topology::TriangleList,
            5 => Topology::TriangleStrip,
            6 => Topology::TriangleFan,
            1 => Topology::LineList,
            // A line loop is a strip with its first vertex again at the end, through indices made for it.
            2 | 3 => Topology::LineStrip,
            0 => {
                not_yet("a point", 2);
                return Ok(());
            }
            _ => {
                context(o, current)?.fail(INVALID_ENUM);
                return Ok(());
            }
        };
        // The program in use, or with none, fixed function's for the context's state.
        let bound = context(o, current)?.program;
        let program = if bound != 0 {
            bound
        } else {
            match self.fixed_program(o, current)? {
                Some(p) => p,
                None => return Ok(()),
            }
        };
        let c = context(o, current)?;
        if count == 0 || instances == 0 {
            return Ok(());
        }
        // The state that shapes the draw, from the context.
        let mut vao = c.vaos.get(&c.vao).cloned().unwrap_or_default();
        if rect.is_some() {
            for a in &mut vao.attribs {
                a.enabled = false;
            }
        }
        let values = c.attrib_values;
        let units = c.units;
        let viewport = c.viewport;
        let scissor = if c.is_enabled(SCISSOR_TEST) { c.scissor } else { [0, 0, fb.width as i32, fb.height as i32] };
        let looped = mode == 2;
        let cull = if topology.is_line() || !c.is_enabled(CULL_FACE) {
            // Lines aren't culled.
            Cull::None
        } else {
            match c.cull_mode {
                0x0404 => Cull::Front,
                0x0405 => Cull::Back,
                _ => return Ok(()),
            }
        };
        // GL's counter-clockwise is the device's clockwise: rows are kept GL's way up.
        let front = if c.front_face == 0x0901 { FrontFace::Clockwise } else { FrontFace::CounterClockwise };
        let depth = (c.is_enabled(DEPTH_TEST) && fb.depth.is_some()).then(|| DepthState { compare: compare(c.depth_func).unwrap_or(Compare::Less), write: c.depth_mask });
        let blend = c.is_enabled(BLEND).then(|| blend_state(c.blend, c.equation));
        // Fixed function's programs test alpha themselves; a program's own is tested as it writes its colour.
        let alpha = match c.compat.alpha() {
            (func, reference) if bound != 0 && c.is_enabled(super::compat::ALPHA_TEST) && func != 0x0207 => Some((func, reference.to_bits())),
            _ => None,
        };
        let mut write = c.color_mask;
        if !fb.alpha {
            // An alpha the buffer hasn't stays 1.
            write[3] = false;
        }
        if fb.color.is_none() {
            // Only depth to draw into: the colour outputs go to a scratch image, written to nowhere.
            write = [false; 4];
        }
        // The compatibility profile's built-ins (and fixed function's own uniforms), as the state is now.
        if let (Some(ctx), Some(l)) = (o.contexts.get(&current), o.programs.get_mut(&program).and_then(|p| p.linked.as_mut())) {
            super::compat::fill(l, ctx);
        }
        let Some(linked) = o.programs.get(&program).and_then(|p| p.linked.as_ref()) else {
            context(o, current)?.fail(INVALID_OPERATION);
            return Ok(());
        };
        if linked.has_blocks {
            not_yet("a program with uniform or buffer blocks", 4);
            return Ok(());
        }
        let Some(area) = intersect(scissor, [0, 0, fb.width as i32, fb.height as i32]) else { return Ok(()) };
        if viewport[2] <= 0 || viewport[3] <= 0 {
            return Ok(());
        }
        let inputs: Vec<(u32, Scalar, u8)> = linked
            .vertex
            .inputs
            .iter()
            .map(|io| match io.slot {
                Slot::Location(l) => (l, io.ty.scalar().unwrap_or(Scalar::F32), io.ty.lanes()),
                _ => (0, Scalar::F32, 4),
            })
            .collect();
        let sampler_units = linked.units.clone();
        let blocks = linked.blocks.clone();
        // Anything that sends the batch is done before its arena is taken for this draw: draws waiting on another
        // framebuffer, and the texture an incomplete one reads (made once).
        if o.batch.target != fb.key && !o.batch.items.is_empty() {
            self.flush(o)?;
        }
        if !sampler_units.is_empty() {
            self.black(o)?;
        }
        let mut arena = std::mem::take(&mut o.batch.arena);
        let result = (|| -> Result<Option<Box<Planned>>, String> {
            // Indices: from the element buffer if the device reads them there, else copied.
            let mut index = None;
            let mut index_values: Option<Vec<u32>> = None;
            if let Some((kind, at, _)) = elements {
                let size = kind_size(kind);
                let bytes = u64::from(count) * u64::from(size);
                let source = if vao.elements != 0 { o.buffers.get(&vao.elements).and_then(|b| b.memory.map(|m| (m, b.size))) } else { None };
                match source {
                    Some((m, buffer_size)) if kind != UNSIGNED_BYTE && at % 4 == 0 => {
                        if at + bytes > buffer_size {
                            return Ok(None);
                        }
                        index = Some((Src::Device(Range { memory: m, offset: at, size: bytes }), if kind == UNSIGNED_SHORT { IndexType::U16 } else { IndexType::U32 }));
                    }
                    _ => {
                        let raw = self.read_source(source.map(|s| s.0), at, bytes, guest)?;
                        let values = indices(&raw, kind);
                        let (src, ty) = if kind == UNSIGNED_INT {
                            (put(&mut arena, &raw, 4), IndexType::U32)
                        } else {
                            let shorts: Vec<u8> = values.iter().flat_map(|&v| (v as u16).to_le_bytes()).collect();
                            (put(&mut arena, &shorts, 4), IndexType::U16)
                        };
                        index_values = Some(values);
                        index = Some((src, ty));
                    }
                }
            }
            if looped {
                let mut values = match (index_values.take(), elements) {
                    (Some(v), _) => v,
                    (None, Some((kind, at, _))) => {
                        let b = if vao.elements != 0 { o.buffers.get(&vao.elements).and_then(|b| b.memory) } else { None };
                        indices(&self.read_source(b, at, u64::from(count) * u64::from(kind_size(kind)), guest)?, kind)
                    }
                    (None, None) => (first..first + count).collect(),
                };
                if let Some(&start) = values.first() {
                    values.push(start);
                }
                let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                index = Some((put(&mut arena, &bytes, 4), IndexType::U32));
                index_values = Some(values);
            }
            // How many vertices a copied array needs.
            let mut vertices = |me: &Self, guest: &mut dyn Guest| -> Result<u64, String> {
                Ok(match elements {
                    None => u64::from(first) + u64::from(count),
                    Some((kind, at, base)) => {
                        if index_values.is_none() {
                            let b = o.buffers.get(&vao.elements).and_then(|b| b.memory);
                            let raw = me.read_source(b, at, u64::from(count) * u64::from(kind_size(kind)), guest)?;
                            index_values = Some(indices(&raw, kind));
                        }
                        let top = index_values.as_ref().and_then(|v| v.iter().max()).copied().unwrap_or(0);
                        (i64::from(top) + i64::from(base) + 1).max(0) as u64
                    }
                })
            };
            let mut key_attrs = Vec::new();
            let mut vertex = Vec::new();
            for &(location, scalar, lanes) in &inputs {
                if let (0, Some(corners)) = (location, rect) {
                    let bytes: Vec<u8> = corners.iter().flat_map(|f| f.to_le_bytes()).collect();
                    vertex.push(put(&mut arena, &bytes, 16));
                    key_attrs.push((0, VertexFormat::F32(4), 16, false));
                    continue;
                }
                let a = vao.attribs.get(location as usize).copied().unwrap_or_default();
                let want = |n: u8| match scalar {
                    Scalar::I32 => VertexFormat::I32(n),
                    Scalar::U32 => VertexFormat::U32(n),
                    _ => VertexFormat::F32(n),
                };
                if !a.enabled {
                    // The generic value, once an instance.
                    let v: Vec<u8> = (0..instances).flat_map(|_| values[location as usize]).flat_map(|w| w.to_le_bytes()).collect();
                    vertex.push(put(&mut arena, &v, 16));
                    key_attrs.push((location, want(4), 16, true));
                    continue;
                }
                let per_instance = a.divisor > 0;
                let native = match (a.kind, a.size, a.normalized, a.integer, scalar) {
                    (FLOAT, 1..=4, _, false, Scalar::F32) => Some(VertexFormat::F32(a.size as u8)),
                    (UNSIGNED_BYTE, 4, true, false, Scalar::F32) => Some(VertexFormat::Unorm8x4),
                    (INT, 1..=4, _, true, Scalar::I32) => Some(VertexFormat::I32(a.size as u8)),
                    (UNSIGNED_INT, 1..=4, _, true, Scalar::U32) => Some(VertexFormat::U32(a.size as u8)),
                    _ => None,
                };
                let buffer = o.buffers.get(&a.buffer).and_then(|b| b.memory.map(|m| (m, b.size)));
                match (native, buffer) {
                    (Some(format), Some((m, size))) if a.divisor <= 1 && a.pointer % 4 == 0 && a.stride % 4 == 0 && a.pointer < size => {
                        vertex.push(Src::Device(Range { memory: m, offset: a.pointer, size: size - a.pointer }));
                        key_attrs.push((location, format, a.stride(), per_instance));
                    }
                    _ => {
                        // Converted into the arena: as many elements as the draw reads.
                        let n = if per_instance { u64::from(instances.div_ceil(a.divisor)) } else { vertices(self, &mut *guest)? };
                        if n == 0 {
                            return Ok(None);
                        }
                        let span = u64::from(a.stride()) * (n - 1) + u64::from(a.bytes());
                        let raw = self.read_source(buffer.map(|b| b.0), a.pointer, span, guest)?;
                        let lanes_out = a.components().max(u32::from(lanes).min(4)) as u8;
                        let out = convert_attrib(&raw, &a, n, scalar, lanes_out);
                        vertex.push(put(&mut arena, &out, 4));
                        key_attrs.push((location, want(lanes_out), 4 * u32::from(lanes_out), per_instance));
                    }
                }
            }
            let color = if fb.color.is_some() { fb.format } else { Format::Rgba8Unorm };
            let blend = if fb.color.is_some() { blend } else { None };
            let key = PipelineKey { attributes: key_attrs, topology, cull, front, depth, depth_attachment: fb.depth.is_some(), color, alpha, blend, write };
            let pipeline = self.pipeline(o, program, &key)?;
            let Some(pipeline) = pipeline else { return Ok(None) };
            // Uniforms as they are now, and the textures each sampler's unit has.
            let mut buffers = Vec::new();
            for (stage, bytes) in blocks.iter().enumerate() {
                if !bytes.is_empty() {
                    buffers.push((glsl::SET_DEFAULT, stage as u32, put(&mut arena, bytes, 256)));
                }
            }
            let mut images = Vec::new();
            for (binding, &unit) in sampler_units.iter().enumerate() {
                let name = units.get(unit as usize).copied().unwrap_or(0);
                let found = o.textures.get(&name).and_then(|t| t.image.map(|i| (i, t.defined & 1 != 0)));
                let (image, sampler) = match found {
                    Some((image, true)) => {
                        let t = o.textures.remove(&name).expect("there");
                        let s = self.sampler(o, &t);
                        o.textures.insert(name, t);
                        (image, s?)
                    }
                    _ => {
                        let black = self.black(o)?;
                        let s = self.sampler(o, &Texture::default())?;
                        (black, s)
                    }
                };
                images.push((glsl::SET_SAMPLER, binding as u32, image, sampler));
            }
            let (first, vertex_offset) = match elements {
                Some((_, _, base)) => (0, base),
                None if looped => (0, 0),
                None => (first, 0),
            };
            let count = count + u32::from(looped);
            let draw = Draw { pipeline, bindings: Vec::new(), vertex_buffers: Vec::new(), index: None, first, count, vertex_offset, first_instance: 0, instances };
            Ok(Some(Box::new(Planned { draw, buffers, images, vertex, index })))
        })();
        o.batch.arena = arena;
        let Some(planned) = result? else { return Ok(()) };
        // In the framebuffer's render pass, open since the last draw on it (sent above if it was another's) or begun now.
        if o.batch.items.is_empty() {
            let color = match fb.color {
                Some(c) => c,
                None => (self.scratch(o, fb.width, fb.height)?, Sub::default()),
            };
            o.batch.target = fb.key;
            let depth = fb.depth.map(|(image, sub)| DepthAttachment { image, sub, load: Load::Load });
            o.batch.items.push(Item::Command(Command::BeginRender { colors: vec![ColorAttachment { image: color.0, sub: color.1, load: Load::Load }], depth }));
        }
        o.batch.items.push(Item::Command(Command::SetViewport(Viewport { x: viewport[0] as f32, y: viewport[1] as f32, width: viewport[2] as f32, height: viewport[3] as f32, min_depth: 0.0, max_depth: 1.0 })));
        o.batch.items.push(Item::Command(Command::SetScissor(Rect { x: area[0] as u32, y: area[1] as u32, width: area[2] as u32, height: area[3] as u32 })));
        o.batch.items.push(Item::Draw(planned));
        Ok(())
    }

    /// Bytes a draw reads: a buffer's (from the device), or with none, the program's own memory.
    fn read_source(&self, buffer: Option<gpu::Memory>, at: u64, bytes: u64, guest: &mut dyn Guest) -> Result<Vec<u8>, String> {
        let mut out = vec![0u8; bytes as usize];
        match buffer {
            Some(m) => {
                let back = self
                    .gpu
                    .run(move |d| {
                        let mut out = vec![0u8; bytes as usize];
                        d.read(m, at, &mut out).map(|()| out)
                    })
                    .map_err(device_error)?;
                out = back.map_err(|e| format!("a draw reads past its buffer: {}", e.message))?;
            }
            None => guest.read(at, &mut out)?,
        }
        Ok(out)
    }

    /// A program's pipeline for a draw's state: made the first time, kept with the program.
    fn pipeline(&self, o: &mut Objects, program: u32, key: &PipelineKey) -> Result<Option<gpu::Pipeline>, String> {
        let Some(linked) = o.programs.get_mut(&program).and_then(|p| p.linked.as_mut()) else { return Ok(None) };
        if let Some(&p) = linked.pipelines.get(key) {
            return Ok(Some(p));
        }
        let (vertex, mut fragment, k) = (linked.vertex.clone(), linked.fragment.clone(), key.clone());
        if let Some((func, reference)) = key.alpha {
            glsl::alpha_test(&mut fragment, func, f32::from_bits(reference));
        }
        let made = self
            .gpu
            .run(move |d| {
                let buffers: Vec<VertexBuffer> = k.attributes.iter().map(|a| VertexBuffer { stride: a.2, per_instance: a.3 }).collect();
                let attributes: Vec<Attribute> = k.attributes.iter().enumerate().map(|(i, a)| Attribute { location: a.0, buffer: i as u32, offset: 0, format: a.1 }).collect();
                let colors = [ColorTarget { format: k.color, blend: k.blend, write: k.write }];
                d.graphics(&GraphicsDesc {
                    vertex: &vertex,
                    fragment: &fragment,
                    buffers: &buffers,
                    attributes: &attributes,
                    topology: k.topology,
                    cull: k.cull,
                    front: k.front,
                    depth: k.depth,
                    depth_format: k.depth_attachment.then_some(Format::D32Float),
                    colors: &colors,
                })
            })
            .map_err(device_error)?;
        match made {
            Ok(p) => {
                linked.pipelines.insert(key.clone(), p);
                Ok(Some(p))
            }
            Err(e) => {
                // A program the device can't run: said, and the draw left out.
                eprintln!("carbon gl: a program's pipeline: {}", e.message);
                Ok(None)
            }
        }
    }

    /// Sends the draws waiting: their render pass ended, their arena on the device, and the work waited for.
    pub(super) fn flush(&self, o: &mut Objects) -> Result<(), String> {
        if o.batch.items.is_empty() {
            return Ok(());
        }
        let batch = std::mem::take(&mut o.batch);
        o.value += 1;
        let (fence, value) = (o.fence, o.value);
        let fence = self
            .gpu
            .run(move |d| -> Result<gpu::Fence, String> {
                let fence = match fence {
                    Some(f) => f,
                    None => d.fence().map_err(device_error)?,
                };
                let arena = d.alloc((batch.arena.len() as u64).max(256)).map_err(device_error)?;
                let result = (|| {
                    d.write(arena, 0, &batch.arena).map_err(device_error)?;
                    let range = |s: Src| match s {
                        Src::Device(r) => r,
                        Src::Arena(offset, size) => Range { memory: arena, offset, size },
                    };
                    let mut commands = Vec::new();
                    for item in batch.items {
                        match item {
                            Item::Command(c) => commands.push(c),
                            Item::Draw(p) => {
                                let mut draw = p.draw;
                                draw.bindings = p.buffers.iter().map(|&(s, b, src)| (s, b, Bind::Buffer(range(src)))).chain(p.images.iter().map(|&(s, b, i, sm)| (s, b, Bind::Combined(i, sm)))).collect();
                                draw.vertex_buffers = p.vertex.iter().map(|&s| range(s)).collect();
                                draw.index = p.index.map(|(s, t)| (range(s), t));
                                commands.push(Command::Draw(draw));
                            }
                        }
                    }
                    commands.push(Command::EndRender);
                    d.submit(Submit { commands: &commands, wait: &[], signal: &[(fence, value)] }).map_err(device_error)?;
                    if !d.wait(fence, value, LONGEST).map_err(device_error)? {
                        return Err("GL's draws didn't finish in time".to_string());
                    }
                    Ok(())
                })();
                d.free(arena);
                result.map(|()| fence)
            })
            .map_err(device_error)??;
        o.fence = Some(fence);
        Ok(())
    }
}

impl Gl {
    /// A colour image of a size that nothing is written to, for a pass with only depth to draw into.
    fn scratch(&self, o: &mut Objects, width: u32, height: u32) -> Result<Image, String> {
        if let Some(&i) = o.scratch.get(&(width, height)) {
            return Ok(i);
        }
        let i = self.gpu.run(move |d| d.image(ImageDesc::new(Format::Rgba8Unorm, width, height))).map_err(device_error)?.map_err(device_error)?;
        o.scratch.insert((width, height), i);
        Ok(i)
    }
}

/// Bytes put in the arena at an alignment: where.
fn put(arena: &mut Vec<u8>, bytes: &[u8], align: usize) -> Src {
    let at = arena.len().next_multiple_of(align);
    arena.resize(at, 0);
    arena.extend_from_slice(bytes);
    Src::Arena(at as u64, bytes.len().max(4) as u64)
}

fn indices(raw: &[u8], kind: u32) -> Vec<u32> {
    match kind {
        UNSIGNED_BYTE => raw.iter().map(|&b| u32::from(b)).collect(),
        UNSIGNED_SHORT => raw.as_chunks::<2>().0.iter().map(|w| u32::from(u16::from_le_bytes(*w))).collect(),
        _ => raw.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect(),
    }
}

/// An attribute's `n` elements converted to the shader's input type, `lanes` each (missing components 0, 0, 0, 1).
fn convert_attrib(raw: &[u8], a: &Attrib, n: u64, scalar: Scalar, lanes: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(n as usize * lanes as usize * 4);
    let size = kind_size(a.kind) as usize;
    for i in 0..n as usize {
        let at = i * a.stride() as usize;
        let mut comps = [0.0f64, 0.0, 0.0, 1.0];
        let mut ints = [0i64, 0, 0, 1];
        for k in 0..a.components() as usize {
            let b = &raw[at + k * size..at + k * size + size];
            let (v, int): (f64, i64) = match a.kind {
                BYTE => (norm(f64::from(b[0] as i8), 127.0, a.normalized, true), i64::from(b[0] as i8)),
                UNSIGNED_BYTE => (norm(f64::from(b[0]), 255.0, a.normalized, false), i64::from(b[0])),
                SHORT => {
                    let x = i16::from_le_bytes([b[0], b[1]]);
                    (norm(f64::from(x), 32767.0, a.normalized, true), i64::from(x))
                }
                UNSIGNED_SHORT => {
                    let x = u16::from_le_bytes([b[0], b[1]]);
                    (norm(f64::from(x), 65535.0, a.normalized, false), i64::from(x))
                }
                INT => {
                    let x = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    (norm(f64::from(x), 2147483647.0, a.normalized, true), i64::from(x))
                }
                UNSIGNED_INT => {
                    let x = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                    (norm(f64::from(x), 4294967295.0, a.normalized, false), i64::from(x))
                }
                HALF_FLOAT => {
                    let f = half(u16::from_le_bytes([b[0], b[1]]));
                    (f, f as i64)
                }
                _ => {
                    let f = f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    (f, f as i64)
                }
            };
            comps[k] = v;
            ints[k] = int;
        }
        if a.size == BGRA {
            comps.swap(0, 2);
            ints.swap(0, 2);
        }
        for k in 0..lanes as usize {
            let w = match scalar {
                Scalar::F32 => (comps[k] as f32).to_bits(),
                _ => ints[k] as u32,
            };
            out.extend(w.to_le_bytes());
        }
    }
    out
}

/// An integer as a float: normalized to [0, 1] or [-1, 1], or as it is.
fn norm(x: f64, max: f64, normalized: bool, signed: bool) -> f64 {
    if !normalized {
        return x;
    }
    if signed { (x / max).max(-1.0) } else { x / max }
}

/// An IEEE half's value.
pub(super) fn half(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = i32::from((h >> 10) & 0x1f);
    let m = f64::from(h & 0x3ff);
    sign * match e {
        0 => m * 2f64.powi(-24),
        31 if m == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + m / 1024.0) * 2f64.powi(e - 15),
    }
}

/// A float as the nearest IEEE half (too large is infinite, too small is zero or subnormal).
pub(super) fn to_half(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // Subnormal: the implicit bit made explicit, shifted down, rounded to nearest.
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = (m >> shift) + ((m >> (shift - 1)) & 1);
        return sign | half as u16;
    }
    let rounded = (mant >> 13) + ((mant >> 12) & 1);
    // A carry out of the mantissa moves the exponent up, as it should.
    sign | (((e as u32) << 10) + rounded) as u16
}

/// GL's internal formats with float components (16- and 32-bit), as the device's format for them.
fn float_internal(internal: u32) -> Option<Format> {
    match internal {
        // RGBA16F, RGB16F, R16F, RG16F, R11F_G11F_B10F.
        0x881a | 0x881b | 0x822d | 0x822f | 0x8c3a => Some(Format::Rgba16Float),
        // RGBA32F, RGB32F, R32F, RG32F.
        0x8814 | 0x8815 | 0x822e | 0x8230 => Some(Format::Rgba32Float),
        _ => None,
    }
}

/// Depth texels as the program gives them (`GL_DEPTH_COMPONENT` or `GL_DEPTH_STENCIL`), as `D32Float`'s: stencil isn't
/// on the device, so it's left out.
fn depth_texels(ty: u32, n: usize, data: &[u8]) -> Result<(Format, Vec<u8>), u32> {
    let size = match ty {
        UNSIGNED_SHORT => 2,
        // UNSIGNED_INT, FLOAT, UNSIGNED_INT_24_8.
        UNSIGNED_INT | FLOAT | 0x84fa => 4,
        // FLOAT_32_UNSIGNED_INT_24_8_REV.
        0x8dad => 8,
        _ => return Err(INVALID_ENUM),
    };
    if data.len() < n * size {
        return Err(INVALID_VALUE);
    }
    let mut out = Vec::with_capacity(n * 4);
    for t in data[..n * size].chunks_exact(size) {
        let word = |at: usize| u32::from_le_bytes([t[at], t[at + 1], t[at + 2], t[at + 3]]);
        let z = match ty {
            UNSIGNED_SHORT => f32::from(u16::from_le_bytes([t[0], t[1]])) / 65535.0,
            UNSIGNED_INT => (f64::from(word(0)) / 4_294_967_295.0) as f32,
            0x84fa => (word(0) >> 8) as f32 / 16_777_215.0,
            _ => f32::from_bits(word(0)),
        };
        out.extend(z.to_le_bytes());
    }
    Ok((Format::D32Float, out))
}

/// Texels as the program gives them (rows tightly packed) made the device's: each format's components spread to
/// RGBA, as GL reads them (luminance to red, green and blue; alpha alone with black), in 8-bit, sRGB, half- or
/// full-float images; depth as `D32Float`.
fn texels(format: u32, ty: u32, internal: u32, width: u32, height: u32, data: &[u8]) -> Result<(Format, Vec<u8>), u32> {
    // DEPTH_COMPONENT, DEPTH_STENCIL.
    if matches!(format, 0x1902 | 0x84f9) {
        return depth_texels(ty, (width * height) as usize, data);
    }
    // RED, RG, RGB, BGR, RGBA, BGRA, ALPHA, LUMINANCE, LUMINANCE_ALPHA.
    let comps = match format {
        0x1903 | 0x1906 | 0x1909 => 1,
        0x8227 | 0x190a => 2,
        0x1907 | 0x80e0 => 3,
        0x1908 | BGRA => 4,
        _ => return Err(INVALID_ENUM),
    };
    let srgb = matches!(internal, 0x8c40..=0x8c43);
    let (fmt, size) = match ty {
        UNSIGNED_BYTE => (float_internal(internal).unwrap_or(if srgb { Format::Rgba8Srgb } else { Format::Rgba8Unorm }), 1),
        FLOAT => (float_internal(internal).unwrap_or(Format::Rgba32Float), 4),
        HALF_FLOAT | 0x8d61 => (float_internal(internal).unwrap_or(Format::Rgba16Float), 2),
        // The packed kinds: 5-6-5, 4-4-4-4, 5-5-5-1, and 8-8-8-8 reversed.
        0x8363 | 0x8033 | 0x8034 | 0x8367 => (Format::Rgba8Unorm, 0),
        _ => return Err(INVALID_ENUM),
    };
    let n = (width * height) as usize;
    let mut out = Vec::with_capacity(n * fmt.bytes() as usize);
    if size == 0 {
        let unit = if ty == 0x8367 { 4 } else { 2 };
        if data.len() < n * unit {
            return Err(INVALID_VALUE);
        }
        for i in 0..n {
            let rgba: [u8; 4] = match ty {
                0x8363 => {
                    let p = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
                    [scale(p >> 11, 31), scale((p >> 5) & 63, 63), scale(p & 31, 31), 255]
                }
                0x8033 => {
                    let p = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
                    [scale(p >> 12, 15), scale((p >> 8) & 15, 15), scale((p >> 4) & 15, 15), scale(p & 15, 15)]
                }
                0x8034 => {
                    let p = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
                    [scale(p >> 11, 31), scale((p >> 6) & 31, 31), scale((p >> 1) & 31, 31), if p & 1 != 0 { 255 } else { 0 }]
                }
                _ => {
                    // 8-8-8-8 reversed: the bytes in memory order, as format names them.
                    let b = [data[4 * i], data[4 * i + 1], data[4 * i + 2], data[4 * i + 3]];
                    if format == BGRA { [b[2], b[1], b[0], b[3]] } else { b }
                }
            };
            out.extend(rgba);
        }
        return Ok((Format::Rgba8Unorm, out));
    }
    if data.len() < n * comps * size {
        return Err(INVALID_VALUE);
    }
    for i in 0..n {
        let c = |k: usize| -> f64 {
            let at = (i * comps + k) * size;
            match size {
                1 => f64::from(data[at]) / 255.0,
                2 => half(u16::from_le_bytes([data[at], data[at + 1]])),
                _ => f64::from(f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])),
            }
        };
        let rgba = match format {
            0x1903 => [c(0), 0.0, 0.0, 1.0],
            0x1906 => [0.0, 0.0, 0.0, c(0)],
            0x1909 => [c(0), c(0), c(0), 1.0],
            0x190a => [c(0), c(0), c(0), c(1)],
            0x8227 => [c(0), c(1), 0.0, 1.0],
            0x1907 => [c(0), c(1), c(2), 1.0],
            0x80e0 => [c(2), c(1), c(0), 1.0],
            0x1908 => [c(0), c(1), c(2), c(3)],
            _ => [c(2), c(1), c(0), c(3)],
        };
        for v in rgba {
            match fmt {
                Format::Rgba32Float => out.extend((v as f32).to_le_bytes()),
                Format::Rgba16Float => out.extend(to_half(v as f32).to_le_bytes()),
                _ => out.push((v * 255.0).round().clamp(0.0, 255.0) as u8),
            }
        }
    }
    Ok((fmt, out))
}

fn scale(v: u16, max: u16) -> u8 {
    ((u32::from(v) * 255 + u32::from(max) / 2) / u32::from(max)) as u8
}

pub(super) fn factor(f: u32) -> Option<Factor> {
    Some(match f {
        0 => Factor::Zero,
        1 => Factor::One,
        0x0300 => Factor::SrcColor,
        0x0301 => Factor::OneMinusSrcColor,
        0x0302 => Factor::SrcAlpha,
        0x0303 => Factor::OneMinusSrcAlpha,
        0x0304 => Factor::DstAlpha,
        0x0305 => Factor::OneMinusDstAlpha,
        0x0306 => Factor::DstColor,
        0x0307 => Factor::OneMinusDstColor,
        _ => return None,
    })
}

pub(super) fn equation(e: u32) -> Option<BlendOp> {
    Some(match e {
        0x8006 => BlendOp::Add,
        0x800a => BlendOp::Subtract,
        0x800b => BlendOp::ReverseSubtract,
        0x8007 => BlendOp::Min,
        0x8008 => BlendOp::Max,
        _ => return None,
    })
}

pub(super) fn compare(f: u32) -> Option<Compare> {
    Some(match f {
        0x0200 => Compare::Never,
        0x0201 => Compare::Less,
        0x0202 => Compare::Equal,
        0x0203 => Compare::LessOrEqual,
        0x0204 => Compare::Greater,
        0x0205 => Compare::NotEqual,
        0x0206 => Compare::GreaterOrEqual,
        0x0207 => Compare::Always,
        _ => return None,
    })
}

/// The context's blending as the device's (the constant colour's factors, which it hasn't, as one).
fn blend_state(f: [u32; 4], e: [u32; 2]) -> Blend {
    let fa = |x: u32| {
        factor(x).unwrap_or_else(|| {
            not_yet("blending with a constant colour or SRC_ALPHA_SATURATE", 8);
            Factor::One
        })
    };
    let eq = |x: u32| equation(x).unwrap_or(BlendOp::Add);
    Blend { color: (fa(f[0]), fa(f[1]), eq(e[0])), alpha: (fa(f[2]), fa(f[3]), eq(e[1])) }
}

