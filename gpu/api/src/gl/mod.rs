//! OpenGL, lowered onto a machine's device on the host: one [`Gl`] for each guest driver instance (lambda's
//! `libGLX_carbon` through a render node open; later atlas's `opengl32`), holding its contexts, its objects and its
//! drawables' default framebuffers. A GL call is a record ([`gpu_wire::gl`]) run here in order, against the context
//! the records say they're for; state and errors are the context's, as GL has them, and only what has to touch the
//! device does.
//!
//! - **Framebuffers**: a drawable's default framebuffer is a colour image (`Bgra8Unorm`, as X11's 24- and 32-bit
//!   visuals lay pixels out) and, when its config has depth, a depth image (`D32Float`). Its rows are kept as GL
//!   numbers them, lowest first, so window coordinates are image coordinates and the device's rasterizer, which puts
//!   NDC's −1 at row 0, needs no flip (GL's counter-clockwise is then the device's clockwise); a swap turns the frame
//!   the right way up as it's read back.
//! - **Programs** (`program`): shaders compiled by `gpu-frontend`'s GLSL compiler, linked, their uniforms kept in
//!   each stage's default block as GL sets them.
//! - **Framebuffer objects** (`framebuffer`): a program's own framebuffers, of textures' levels and renderbuffers,
//!   which draws, clears and reads go to instead of the drawable's when one is bound. Rows are GL's way up in them
//!   too, so what's drawn into a texture is sampled as GL samples it.
//! - **Draws** (`draw`): buffers, vertex arrays and textures as device objects; each draw a pipeline (made once for
//!   its program and state) in a render pass that stays open over the draws that follow, sent at the next point
//!   something has to wait. What a draw reads that can change after it (uniforms, the program's own arrays, indices
//!   converted) is copied into the batch's arena as it's recorded, so each draw sees what was there when it was made.
//!
//! Stencil buffers aren't on the device yet, so stencil clears and tests do nothing to pixels.

mod compat;
mod draw;
mod framebuffer;
mod program;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use gpu::{ColorAttachment, Command, DepthAttachment, Format, Image, ImageDesc, Load, Rect, Sub, Submit};
use gpu_wire::gl::{self as wire, BOOL, FLOAT, INT};
use gpu_wire::records::{self, Reader};

use crate::Gpu;
use crate::vulkan::Guest;

use draw::{Buffer, Texture, Vao};
use framebuffer::{Fbo, Renderbuffer};
use program::{Program, Shader};

// GL's names this module uses.
const NO_ERROR: u32 = 0;
const INVALID_ENUM: u32 = 0x0500;
const INVALID_VALUE: u32 = 0x0501;
const INVALID_OPERATION: u32 = 0x0502;
const DEPTH_BUFFER_BIT: u32 = 0x0100;
const STENCIL_BUFFER_BIT: u32 = 0x0400;
const COLOR_BUFFER_BIT: u32 = 0x4000;
const SCISSOR_TEST: u32 = 0x0c11;
const CULL_FACE: u32 = 0x0b44;
const DEPTH_TEST: u32 = 0x0b71;
const BLEND: u32 = 0x0be2;
const DITHER: u32 = 0x0bd0;
const MULTISAMPLE: u32 = 0x809d;
const INVALID_FRAMEBUFFER_OPERATION: u32 = 0x0506;
const RGB: u32 = 0x1907;
const RGBA: u32 = 0x1908;
const BGRA: u32 = 0x80e1;
const UNSIGNED_BYTE: u32 = 0x1401;

/// The capabilities `glEnable` knows, and whether each starts on.
const CAPS: &[(u32, bool)] = &[
    (CULL_FACE, false),
    (DEPTH_TEST, false),
    (0x0b90, false), // STENCIL_TEST
    (DITHER, true),
    (BLEND, false),
    (SCISSOR_TEST, false),
    (0x8037, false), // POLYGON_OFFSET_FILL
    (MULTISAMPLE, true),
    (0x809e, false), // SAMPLE_ALPHA_TO_COVERAGE
    (0x80a0, false), // SAMPLE_COVERAGE
    (0x8642, false), // PROGRAM_POINT_SIZE
    (0x8d69, false), // PRIMITIVE_RESTART_FIXED_INDEX
    (0x884f, true),  // TEXTURE_CUBE_MAP_SEAMLESS
    (0x8db9, false), // FRAMEBUFFER_SRGB
    (0x0b20, false), // LINE_SMOOTH
    (0x0bf2, false), // COLOR_LOGIC_OP
    // The compatibility profile's: the alpha test and fog are drawn; lighting and the rest are kept, not drawn yet.
    (compat::ALPHA_TEST, false),
    (compat::FOG, false),
    (compat::LIGHTING, false),
    (0x4000, false), // LIGHT0..
    (0x4001, false),
    (0x4002, false),
    (0x4003, false),
    (0x4004, false),
    (0x4005, false),
    (0x4006, false),
    (0x4007, false),
    (0x0b57, false), // COLOR_MATERIAL
    (0x0ba1, false), // NORMALIZE
    (0x803a, false), // RESCALE_NORMAL
    (0x0b10, false), // POINT_SMOOTH
    (0x0b41, false), // POLYGON_SMOOTH
    (0x0b24, false), // LINE_STIPPLE
    (0x2a01, false), // POLYGON_OFFSET_POINT
    (0x2a02, false), // POLYGON_OFFSET_LINE
    (0x0c60, false), // TEXTURE_GEN_S..Q
    (0x0c61, false),
    (0x0c62, false),
    (0x0c63, false),
    (0x8458, false), // COLOR_SUM
    (0x8861, false), // POINT_SPRITE
    (0x8643, false), // VERTEX_PROGRAM_TWO_SIDE
    (0x0d80, false), // AUTO_NORMAL
    (0x0de0, false), // TEXTURE_1D
    (0x8513, false), // TEXTURE_CUBE_MAP
    (0x806f, false), // TEXTURE_3D
];

/// Vertex attributes and texture units a context has (`GL_MAX_VERTEX_ATTRIBS`, `GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS`).
pub const ATTRIBS: usize = 16;
pub const UNITS: usize = 32;

/// How long a submission may run before GL gives up on it.
const LONGEST: Duration = Duration::from_secs(600);

/// One guest driver instance's GL: its contexts, objects and drawables, and the device's timeline its work signals.
pub struct Gl {
    gpu: Gpu,
    state: Mutex<Objects>,
}

/// What a guest driver instance's contexts have, shared between them as share lists share GL's objects.
#[derive(Default)]
struct Objects {
    contexts: HashMap<u64, Context>,
    drawables: HashMap<u64, Framebuffer>,
    /// Framebuffer objects and renderbuffers, by their names.
    fbos: HashMap<u32, Fbo>,
    renderbuffers: HashMap<u32, Renderbuffer>,
    /// Colour images nothing is written to, by size: a pass with only depth to draw into has one, so a program's
    /// colour outputs have somewhere to go.
    scratch: HashMap<(u32, u32), Image>,
    shaders: HashMap<u32, Shader>,
    programs: HashMap<u32, Program>,
    buffers: HashMap<u32, Buffer>,
    textures: HashMap<u32, Texture>,
    /// Samplers made, by what they are: the device keeps them.
    samplers: HashMap<draw::SamplerKey, gpu::Sampler>,
    /// What a texture unit with no complete texture reads: one black texel.
    black: Option<Image>,
    batch: Batch,
    /// Fixed function's programs, by their shape.
    fixed: Vec<(compat::Shape, u32)>,
    fence: Option<gpu::Fence>,
    value: u64,
}

/// Draws not sent yet: a render pass on one framebuffer (its [`Framebuffer::key`]), its commands, and the bytes they
/// read.
#[derive(Default)]
struct Batch {
    target: u64,
    items: Vec<draw::Item>,
    arena: Vec<u8>,
}

/// What draws, clears and reads go to: a drawable's default framebuffer, or a framebuffer object's attachments as
/// they are now.
#[derive(Clone, Copy)]
struct Framebuffer {
    /// What draws into it are batched by: a drawable's id, or a framebuffer object's name with the top bit set.
    key: u64,
    width: u32,
    height: u32,
    /// The colour image and the level drawn into, and its format.
    color: Option<(Image, Sub)>,
    format: Format,
    depth: Option<(Image, Sub)>,
    alpha: bool,
    stencil_bits: u32,
}

/// A context's state, as GL keeps it.
struct Context {
    drawable: u64,
    /// Whether it's been bound to a drawable before: the first time sets its viewport and scissor.
    bound: bool,
    error: u32,
    clear_color: [f32; 4],
    clear_depth: f64,
    clear_stencil: i32,
    viewport: [i32; 4],
    scissor: [i32; 4],
    enabled: Vec<u32>,
    color_mask: [bool; 4],
    depth_mask: bool,
    program: u32,
    /// Buffers bound, by target (`GL_ARRAY_BUFFER` and the rest; the element buffer is the vertex array's).
    bindings: HashMap<u32, u32>,
    /// Vertex arrays, this context's own as GL has them (0 is the default one).
    vaos: HashMap<u32, Vao>,
    vao: u32,
    unit: u32,
    /// Each unit's 2D texture.
    units: [u32; UNITS],
    blend: [u32; 4],
    equation: [u32; 2],
    depth_func: u32,
    cull_mode: u32,
    front_face: u32,
    /// Each generic attribute's value when its array is off.
    attrib_values: [[u32; 4]; ATTRIBS],
    /// The framebuffers draws and reads go to (0 for the drawable's), and the renderbuffer bound.
    draw_fbo: u32,
    read_fbo: u32,
    renderbuffer: u32,
    /// The compatibility profile's state.
    compat: compat::Compat,
}

impl Context {
    fn new() -> Context {
        let mut vaos = HashMap::new();
        vaos.insert(0, Vao::default());
        Context {
            drawable: 0,
            bound: false,
            error: NO_ERROR,
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            viewport: [0; 4],
            scissor: [0; 4],
            enabled: CAPS.iter().filter(|c| c.1).map(|c| c.0).collect(),
            color_mask: [true; 4],
            depth_mask: true,
            program: 0,
            bindings: HashMap::new(),
            vaos,
            vao: 0,
            unit: 0,
            units: [0; UNITS],
            // ONE, ZERO; FUNC_ADD; LESS; BACK; CCW.
            blend: [1, 0, 1, 0],
            equation: [0x8006, 0x8006],
            depth_func: 0x0201,
            cull_mode: 0x0405,
            front_face: 0x0901,
            attrib_values: {
                // The current normal (0, 0, 1) and colour (1, 1, 1, 1), the compatibility profile's, alias attributes 2 and 3.
                let mut v = [[0, 0, 0, 1.0f32.to_bits()]; ATTRIBS];
                v[2] = [0, 0, 1.0f32.to_bits(), 1.0f32.to_bits()];
                v[3] = [1.0f32.to_bits(); 4];
                v
            },
            draw_fbo: 0,
            read_fbo: 0,
            renderbuffer: 0,
            compat: compat::Compat::default(),
        }
    }

    /// Records an error, unless one is waiting to be read already.
    fn fail(&mut self, error: u32) {
        if self.error == NO_ERROR {
            self.error = error;
        }
    }

    fn is_enabled(&self, cap: u32) -> bool {
        self.enabled.contains(&cap)
    }

    fn vao_mut(&mut self) -> &mut Vao {
        let at = self.vao;
        self.vaos.entry(at).or_default()
    }
}

/// A value `glGet*` answers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// A step of work on the device, as a context's call makes it, sent at once.
enum Step {
    /// A render pass that only clears: each attachment cleared, or loaded as it is.
    Clear { color: Option<(Image, Sub, Load<[f32; 4]>)>, depth: Option<(Image, Sub, Load<f32>)> },
    /// Texels into a region of one level of an image.
    Upload { bytes: Vec<u8>, to: Image, level: u32, region: Rect },
    /// A region of one level of an image read back, `bytes` a texel: one of the answers `run` gives.
    Download { from: Image, sub: Sub, region: Rect, bytes: u32 },
    /// An image's levels made from its first.
    Mips(Image),
}

/// A record's reader, with its errors said as the host's.
struct R<'a>(Reader<'a>);

impl<'a> R<'a> {
    fn u32(&mut self) -> Result<u32, String> {
        self.0.u32().map_err(malformed)
    }
    fn i32(&mut self) -> Result<i32, String> {
        self.u32().map(|w| w as i32)
    }
    fn u64(&mut self) -> Result<u64, String> {
        self.0.u64().map_err(malformed)
    }
    fn f32(&mut self) -> Result<f32, String> {
        self.0.f32().map_err(malformed)
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        self.0.bytes().map_err(malformed)
    }
    fn words<const N: usize>(&mut self) -> Result<[u32; N], String> {
        let mut out = [0; N];
        for w in &mut out {
            *w = self.u32()?;
        }
        Ok(out)
    }
}

impl Gl {
    pub fn new(gpu: Gpu) -> Gl {
        Gl { gpu, state: Mutex::new(Objects::default()) }
    }

    /// Runs a doorbell's records, and gives the reply: [`wire::OK`] and the answers, or [`wire::FAILED`] and why.
    pub fn execute(&self, records: &[u8], guest: &mut dyn Guest) -> Vec<u8> {
        let mut reply = Vec::new();
        records::put_u32(&mut reply, wire::OK);
        let mut o = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // Draws stay in the open batch across rings, a frame's in one submission: whatever reads what they drew (a
        // swap, `glReadPixels`), or changes what they read, sends them first, and so does `glFinish`.
        let result = self.run(&mut o, records, guest, &mut reply);
        if result.is_err() {
            o.batch = Batch::default();
        }
        if let Err(why) = result {
            reply.clear();
            records::put_u32(&mut reply, wire::FAILED);
            reply.extend_from_slice(why.as_bytes());
        }
        reply
    }

    fn run(&self, o: &mut Objects, records: &[u8], guest: &mut dyn Guest, reply: &mut Vec<u8>) -> Result<(), String> {
        let mut r = Reader::new(records);
        let mut current = 0u64;
        while !r.is_empty() {
            let (kind, p) = r.record().map_err(malformed)?;
            let mut p = R(p);
            match kind {
                wire::CONTEXT => {
                    let (id, _share) = (p.u64()?, p.u64()?);
                    o.contexts.insert(id, Context::new());
                }
                wire::FREE_CONTEXT => {
                    o.contexts.remove(&p.u64()?);
                }
                wire::USE => {
                    current = p.u64()?;
                    if !o.contexts.contains_key(&current) {
                        return Err(format!("context {current:#x} isn't this driver's"));
                    }
                }
                wire::FREE_DRAWABLE => {
                    let id = p.u64()?;
                    self.flush(o)?;
                    if let Some(f) = o.drawables.remove(&id) {
                        self.free_framebuffer(f)?;
                    }
                }
                wire::BIND => {
                    let drawable = p.u64()?;
                    let [width, height, depth_bits, stencil_bits, alpha] = p.words::<5>()?;
                    if drawable != 0 {
                        self.framebuffer(o, drawable, width.max(1), height.max(1), depth_bits > 0, stencil_bits, alpha != 0)?;
                    }
                    let c = context(o, current)?;
                    c.drawable = drawable;
                    if drawable != 0 && !c.bound {
                        c.bound = true;
                        c.viewport = [0, 0, width as i32, height as i32];
                        c.scissor = c.viewport;
                    }
                }
                wire::GET_ERROR => {
                    let c = context(o, current)?;
                    records::put_u32(reply, std::mem::replace(&mut c.error, NO_ERROR));
                }
                wire::ERROR => {
                    let code = p.u32()?;
                    context(o, current)?.fail(code);
                }
                wire::FINISH => self.flush(o)?,
                wire::GET => {
                    let pname = p.u32()?;
                    let fb = framebuffer_of(o, current, false);
                    let c = context(o, current)?;
                    let values = get(c, fb.as_ref(), pname);
                    if values.is_none() {
                        c.fail(INVALID_ENUM);
                    }
                    let values = values.unwrap_or_default();
                    records::put_u32(reply, values.len() as u32);
                    for v in values {
                        let (kind, bits) = match v {
                            Value::Int(i) => (INT, i as u64),
                            Value::Float(f) => (FLOAT, f.to_bits()),
                            Value::Bool(b) => (BOOL, u64::from(b)),
                        };
                        records::put_u32(reply, kind);
                        records::put_u64(reply, bits);
                    }
                }
                wire::IS_ENABLED => {
                    let cap = p.u32()?;
                    let c = context(o, current)?;
                    if !CAPS.iter().any(|k| k.0 == cap) {
                        c.fail(INVALID_ENUM);
                    }
                    records::put_u32(reply, u32::from(c.is_enabled(cap)));
                }
                wire::CLEAR_COLOR => {
                    let v = [p.f32()?, p.f32()?, p.f32()?, p.f32()?];
                    context(o, current)?.clear_color = v;
                }
                wire::CLEAR_DEPTH => {
                    let d = f64::from_bits(p.u64()?);
                    context(o, current)?.clear_depth = d.clamp(0.0, 1.0);
                }
                wire::CLEAR_STENCIL => {
                    let s = p.i32()?;
                    context(o, current)?.clear_stencil = s;
                }
                wire::VIEWPORT | wire::SCISSOR => {
                    let v = [p.i32()?, p.i32()?, p.i32()?, p.i32()?];
                    let c = context(o, current)?;
                    if v[2] < 0 || v[3] < 0 {
                        c.fail(INVALID_VALUE);
                    } else if kind == wire::VIEWPORT {
                        // Clamped to the largest the implementation allows.
                        let most = gpu::MAX_IMAGE as i32;
                        c.viewport = [v[0], v[1], v[2].min(most), v[3].min(most)];
                    } else {
                        c.scissor = v;
                    }
                }
                wire::ENABLE | wire::DISABLE => {
                    let cap = p.u32()?;
                    let c = context(o, current)?;
                    if cap == compat::TEXTURE_2D {
                        // Texturing is each unit's: the active one's.
                        let unit = (c.unit as usize).min(compat::UNITS - 1);
                        c.compat.textures[unit] = kind == wire::ENABLE;
                    } else if !CAPS.iter().any(|k| k.0 == cap) {
                        c.fail(INVALID_ENUM);
                    } else {
                        c.enabled.retain(|&e| e != cap);
                        if kind == wire::ENABLE {
                            c.enabled.push(cap);
                        }
                    }
                }
                wire::COLOR_MASK => {
                    let m = p.words::<4>()?.map(|w| w != 0);
                    context(o, current)?.color_mask = m;
                }
                wire::DEPTH_MASK => {
                    let m = p.u32()? != 0;
                    context(o, current)?.depth_mask = m;
                }
                wire::CLEAR => {
                    let mask = p.u32()?;
                    self.clear(o, current, mask)?;
                }
                wire::READ_PIXELS => {
                    let v = p.words::<6>()?;
                    let (at, stride) = (p.u64()?, p.u32()?);
                    self.read_pixels(o, current, [v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32], v[4], v[5], at, stride, guest)?;
                }
                wire::SWAP => {
                    let (drawable, at, stride) = (p.u64()?, p.u64()?, p.u32()?);
                    let Some(&f) = o.drawables.get(&drawable) else { return Err(format!("drawable {drawable:#x} isn't bound to a context")) };
                    let Some((color, sub)) = f.color else { return Err(format!("drawable {drawable:#x} has no colour buffer")) };
                    let whole = Rect { x: 0, y: 0, width: f.width, height: f.height };
                    let frame = self.steps(o, vec![Step::Download { from: color, sub, region: whole, bytes: 4 }])?.pop().unwrap_or_default();
                    let row = f.width as usize * 4;
                    // Top first, as the window shows it.
                    for (i, texels) in frame.chunks_exact(row).rev().enumerate() {
                        guest.write(at + i as u64 * u64::from(stride), texels)?;
                    }
                }
                wire::BLEND_FUNC => {
                    let f = p.words::<4>()?;
                    let c = context(o, current)?;
                    if f.iter().all(|&x| draw::factor(x).is_some() || matches!(x, 0x8001..=0x8004 | 0x0308)) {
                        c.blend = f;
                    } else {
                        c.fail(INVALID_ENUM);
                    }
                }
                wire::BLEND_EQUATION => {
                    let e = p.words::<2>()?;
                    let c = context(o, current)?;
                    if e.iter().all(|&x| draw::equation(x).is_some()) {
                        c.equation = e;
                    } else {
                        c.fail(INVALID_ENUM);
                    }
                }
                wire::DEPTH_FUNC => {
                    let f = p.u32()?;
                    let c = context(o, current)?;
                    if draw::compare(f).is_some() {
                        c.depth_func = f;
                    } else {
                        c.fail(INVALID_ENUM);
                    }
                }
                wire::CULL_FACE => {
                    let m = p.u32()?;
                    let c = context(o, current)?;
                    if matches!(m, 0x0404 | 0x0405 | 0x0408) {
                        c.cull_mode = m;
                    } else {
                        c.fail(INVALID_ENUM);
                    }
                }
                wire::FRONT_FACE => {
                    let m = p.u32()?;
                    let c = context(o, current)?;
                    if matches!(m, 0x0900 | 0x0901) {
                        c.front_face = m;
                    } else {
                        c.fail(INVALID_ENUM);
                    }
                }
                _ => {
                    if !self.program_record(o, current, kind, &mut p, reply)?
                        && !self.draw_record(o, current, kind, &mut p, guest)?
                        && !self.compat_record(o, current, kind, &mut p)?
                        && !self.framebuffer_record(o, current, kind, &mut p, reply)?
                    {
                        return Err(format!("a GL record of kind {kind} this host doesn't know"));
                    }
                }
            }
        }
        Ok(())
    }

    /// A drawable's framebuffer, made or remade to fit its size and config.
    #[allow(clippy::too_many_arguments)]
    fn framebuffer(&self, o: &mut Objects, id: u64, width: u32, height: u32, depth: bool, stencil_bits: u32, alpha: bool) -> Result<(), String> {
        if let Some(f) = o.drawables.get(&id) {
            if (f.width, f.height, f.depth.is_some()) == (width, height, depth) {
                return Ok(());
            }
            self.flush(o)?;
            let f = o.drawables.remove(&id).expect("there");
            self.free_framebuffer(f)?;
        }
        let (color, depth_image) = self
            .gpu
            .run(move |d| -> Result<(Image, Option<Image>), gpu::Error> {
                let color = d.image(ImageDesc::new(Format::Bgra8Unorm, width, height))?;
                let depth = if depth { Some(d.image(ImageDesc::new(Format::D32Float, width, height))?) } else { None };
                Ok((color, depth))
            })
            .map_err(device_error)?
            .map_err(device_error)?;
        let sub = Sub::default();
        // What a new buffer holds is undefined; it's made black, and far, so it's the same every time.
        let depth_load = depth_image.map(|i| (i, sub, Load::Clear(1.0)));
        self.steps(o, vec![Step::Clear { color: Some((color, sub, Load::Clear([0.0, 0.0, 0.0, if alpha { 0.0 } else { 1.0 }]))), depth: depth_load }])?;
        let f = Framebuffer { key: id, width, height, color: Some((color, sub)), format: Format::Bgra8Unorm, depth: depth_image.map(|i| (i, sub)), alpha, stencil_bits };
        o.drawables.insert(id, f);
        Ok(())
    }

    fn free_framebuffer(&self, f: Framebuffer) -> Result<(), String> {
        self.gpu
            .run(move |d| {
                for (image, _) in f.color.into_iter().chain(f.depth) {
                    d.free_image(image);
                }
            })
            .map_err(device_error)
    }

    /// `glClear`: the buffers in `mask`, inside the scissor when its test is on, as the write masks allow.
    fn clear(&self, o: &mut Objects, current: u64, mask: u32) -> Result<(), String> {
        let fb = framebuffer_of(o, current, false);
        let c = context(o, current)?;
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            c.fail(INVALID_VALUE);
            return Ok(());
        }
        let Some(f) = fb else {
            if c.draw_fbo != 0 {
                c.fail(INVALID_FRAMEBUFFER_OPERATION);
            }
            return Ok(());
        };
        let whole = [0, 0, f.width as i32, f.height as i32];
        let area = if c.is_enabled(SCISSOR_TEST) { intersect(c.scissor, whole) } else { Some(whole) };
        let Some(area) = area else { return Ok(()) };
        let all = area == whole;
        let region = Rect { x: area[0] as u32, y: area[1] as u32, width: area[2] as u32, height: area[3] as u32 };
        let color = if mask & COLOR_BUFFER_BIT != 0 && c.color_mask.contains(&true) { f.color } else { None };
        let depth = if mask & DEPTH_BUFFER_BIT != 0 && c.depth_mask { f.depth } else { None };
        // Colour is kept in [0, 1] in a fixed-point buffer; an alpha the buffer hasn't reads as 1.
        let mut rgba = c.clear_color;
        if !f.format_is_float() {
            rgba = rgba.map(|v| v.clamp(0.0, 1.0));
        }
        if !f.alpha {
            rgba[3] = 1.0;
        }
        let (color_mask, z) = (c.color_mask, c.clear_depth as f32);
        let mut steps = Vec::new();
        if all && (color.is_none() || color_mask == [true; 4]) {
            // A pass that clears what it's asked to and loads the rest.
            if color.is_some() || depth.is_some() {
                let color_load = f.color.map(|(i, sub)| (i, sub, if color.is_some() { Load::Clear(rgba) } else { Load::Load }));
                let depth_load = f.depth.map(|(i, sub)| (i, sub, if depth.is_some() { Load::Clear(z) } else { Load::Load }));
                steps.push(Step::Clear { color: color_load, depth: depth_load });
            }
        } else {
            let n = region.width as usize * region.height as usize;
            if let Some((image, sub)) = color {
                match texel(f.format, rgba) {
                    Some(t) => {
                        let mut bytes: Vec<u8> = t.iter().copied().cycle().take(n * t.len()).collect();
                        if color_mask != [true; 4] {
                            // The channels it mustn't write, as they are.
                            let now = self.steps(o, vec![Step::Download { from: image, sub, region, bytes: t.len() as u32 }])?.pop().unwrap_or_default();
                            let size = t.len() / 4;
                            let order = channels(f.format);
                            for (texel, old) in bytes.chunks_exact_mut(t.len()).zip(now.chunks_exact(t.len())) {
                                for (k, &channel) in order.iter().enumerate() {
                                    if !color_mask[channel] {
                                        texel[k * size..(k + 1) * size].copy_from_slice(&old[k * size..(k + 1) * size]);
                                    }
                                }
                            }
                        }
                        steps.push(Step::Upload { bytes, to: image, level: sub.level, region });
                    }
                    None => draw::not_yet("a part of a buffer of this format cleared", 16),
                }
            }
            if let Some((image, sub)) = depth {
                let bytes = z.to_le_bytes().iter().copied().cycle().take(n * 4).collect();
                steps.push(Step::Upload { bytes, to: image, level: sub.level, region });
            }
        }
        if !steps.is_empty() {
            self.steps(o, steps)?;
        }
        Ok(())
    }

    /// `glReadPixels` from the read framebuffer's colour, as RGBA, BGRA or RGB bytes: what's inside the framebuffer is
    /// written to the program, the rest left as it was, as GL leaves it undefined.
    #[allow(clippy::too_many_arguments)]
    fn read_pixels(&self, o: &mut Objects, current: u64, area: [i32; 4], format: u32, kind: u32, at: u64, stride: u32, guest: &mut dyn Guest) -> Result<(), String> {
        let fb = framebuffer_of(o, current, true);
        let c = context(o, current)?;
        if area[2] < 0 || area[3] < 0 {
            c.fail(INVALID_VALUE);
            return Ok(());
        }
        if !matches!(format, RGBA | BGRA | RGB) || kind != UNSIGNED_BYTE {
            c.fail(INVALID_ENUM);
            return Ok(());
        }
        let Some((f, (image, sub))) = fb.and_then(|f| f.color.map(|c| (f, c))) else {
            c.fail(if c.read_fbo != 0 { INVALID_FRAMEBUFFER_OPERATION } else { INVALID_OPERATION });
            return Ok(());
        };
        let Some(inside) = intersect(area, [0, 0, f.width as i32, f.height as i32]) else { return Ok(()) };
        let region = Rect { x: inside[0] as u32, y: inside[1] as u32, width: inside[2] as u32, height: inside[3] as u32 };
        let size = f.format.bytes();
        let texels = self.steps(o, vec![Step::Download { from: image, sub, region, bytes: size }])?.pop().unwrap_or_default();
        let out_size = if format == RGB { 3 } else { 4 };
        for (row, line) in texels.chunks_exact(region.width as usize * size as usize).enumerate() {
            let mut out = Vec::with_capacity(region.width as usize * out_size);
            for t in line.chunks_exact(size as usize) {
                let [r, g, b, a] = rgba8(f.format, t);
                let a = if f.alpha { a } else { 255 };
                match format {
                    BGRA => out.extend([b, g, r, a]),
                    RGBA => out.extend([r, g, b, a]),
                    _ => out.extend([r, g, b]),
                }
            }
            let y = (inside[1] - area[1]) as u64 + row as u64;
            let x = (inside[0] - area[0]) as u64;
            guest.write(at + y * u64::from(stride) + x * out_size as u64, &out)?;
        }
        Ok(())
    }

    /// Runs steps on the device and waits for them, after any draws waiting: the downloads' bytes, in order.
    fn steps(&self, o: &mut Objects, steps: Vec<Step>) -> Result<Vec<Vec<u8>>, String> {
        self.flush(o)?;
        o.value += 1;
        let (fence, value) = (o.fence, o.value);
        let (fence, back) = self.gpu.run(move |d| run(d, fence, value, steps)).map_err(device_error)??;
        o.fence = Some(fence);
        Ok(back)
    }
}

/// Steps as one submission, on the service's thread.
fn run(d: &mut gpu::Device, fence: Option<gpu::Fence>, value: u64, steps: Vec<Step>) -> Result<(gpu::Fence, Vec<Vec<u8>>), String> {
    let fence = match fence {
        Some(f) => f,
        None => d.fence().map_err(device_error)?,
    };
    let mut temps = Vec::new();
    let mut downloads = Vec::new();
    let result = (|| {
        let mut commands = Vec::new();
        for step in &steps {
            match step {
                Step::Clear { color, depth } => {
                    let colors = color.iter().map(|&(image, sub, load)| ColorAttachment { image, sub, load }).collect();
                    let depth = depth.map(|(image, sub, load)| DepthAttachment { image, sub, load });
                    commands.push(Command::BeginRender { colors, depth });
                    commands.push(Command::EndRender);
                }
                Step::Upload { bytes, to, level, region } => {
                    let m = d.alloc(bytes.len().max(4) as u64).map_err(device_error)?;
                    temps.push(m);
                    d.write(m, 0, bytes).map_err(device_error)?;
                    let from = gpu::Range { memory: m, offset: 0, size: bytes.len() as u64 };
                    commands.push(Command::CopyBufferToImage { from, row_pixels: 0, to: *to, sub: Sub { level: *level, layer: 0 }, region: *region });
                }
                Step::Download { from, sub, region, bytes } => {
                    let size = u64::from(region.width) * u64::from(region.height) * u64::from(*bytes);
                    let m = d.alloc(size.max(4)).map_err(device_error)?;
                    temps.push(m);
                    downloads.push((m, size));
                    let to = gpu::Range { memory: m, offset: 0, size };
                    commands.push(Command::CopyImageToBuffer { from: *from, sub: *sub, region: *region, to, row_pixels: 0 });
                }
                Step::Mips(image) => commands.push(Command::GenerateMips(*image)),
            }
        }
        d.submit(Submit { commands: &commands, wait: &[], signal: &[(fence, value)] }).map_err(device_error)?;
        if !d.wait(fence, value, LONGEST).map_err(device_error)? {
            return Err("GL's work on the device didn't finish in time".to_string());
        }
        let mut back = Vec::new();
        for &(m, size) in &downloads {
            let mut bytes = vec![0; size as usize];
            d.read(m, 0, &mut bytes).map_err(device_error)?;
            back.push(bytes);
        }
        Ok(back)
    })();
    for m in temps {
        d.free(m);
    }
    result.map(|back| (fence, back))
}

/// `glGet*`'s values for `pname`, or None for a name GL hasn't (or this one doesn't know yet).
fn get(c: &Context, fb: Option<&Framebuffer>, pname: u32) -> Option<Vec<Value>> {
    let ints = |v: &[i32]| v.iter().map(|&i| Value::Int(i64::from(i))).collect();
    let int = |i: u32| vec![Value::Int(i64::from(i))];
    let bits = |f: fn(&Framebuffer) -> u32| int(fb.map_or(0, f));
    Some(match pname {
        0x0ba2 => ints(&c.viewport),
        0x0c10 => ints(&c.scissor),
        0x0c22 => c.clear_color.iter().map(|&f| Value::Float(f64::from(f))).collect(),
        0x0b73 => vec![Value::Float(c.clear_depth)],
        0x0b91 => vec![Value::Int(i64::from(c.clear_stencil))],
        0x0c23 => c.color_mask.iter().map(|&b| Value::Bool(b)).collect(),
        0x0b72 => vec![Value::Bool(c.depth_mask)],
        // MAX_VIEWPORT_DIMS, MAX_TEXTURE_SIZE, MAX_RENDERBUFFER_SIZE.
        0x0d3a => ints(&[gpu::MAX_IMAGE as i32; 2]),
        0x0d33 | 0x84e8 => int(gpu::MAX_IMAGE),
        // RED_BITS, GREEN_BITS, BLUE_BITS, ALPHA_BITS, DEPTH_BITS, STENCIL_BITS.
        0x0d52..=0x0d54 => bits(|f| if f.color.is_none() { 0 } else if f.format_is_float() { 16 } else { 8 }),
        0x0d55 => bits(|f| if !f.alpha || f.color.is_none() { 0 } else if f.format_is_float() { 16 } else { 8 }),
        0x0d56 => bits(|f| if f.depth.is_some() { 24 } else { 0 }),
        0x0d57 => bits(|f| f.stencil_bits),
        // DOUBLEBUFFER.
        0x0c32 => vec![Value::Bool(true)],
        // CURRENT_PROGRAM, ARRAY_BUFFER_BINDING, ELEMENT_ARRAY_BUFFER_BINDING, VERTEX_ARRAY_BINDING, ACTIVE_TEXTURE,
        // TEXTURE_BINDING_2D.
        0x8b8d => int(c.program),
        0x8894 => int(c.bindings.get(&0x8892).copied().unwrap_or(0)),
        0x8895 => int(c.vaos.get(&c.vao).map_or(0, |v| v.elements)),
        0x85b5 => int(c.vao),
        // DRAW_FRAMEBUFFER_BINDING (FRAMEBUFFER_BINDING), READ_FRAMEBUFFER_BINDING, RENDERBUFFER_BINDING, and which
        // buffer draws and reads go to: the back buffer, or the colour attachment.
        0x8ca6 => int(c.draw_fbo),
        0x8caa => int(c.read_fbo),
        0x8ca7 => int(c.renderbuffer),
        0x0c01 | 0x8825 => int(if c.draw_fbo != 0 { 0x8ce0 } else { 0x0405 }),
        0x0c02 => int(if c.read_fbo != 0 { 0x8ce0 } else { 0x0405 }),
        0x84e0 => int(0x84c0 + c.unit),
        0x8069 => int(c.units[c.unit as usize]),
        // MAX_VERTEX_ATTRIBS, MAX_TEXTURE_IMAGE_UNITS, MAX_COMBINED_TEXTURE_IMAGE_UNITS, MAX_VERTEX_TEXTURE_IMAGE_UNITS,
        // MAX_DRAW_BUFFERS, MAX_COLOR_ATTACHMENTS, MAX_VARYING_COMPONENTS, MAX_VERTEX_UNIFORM_COMPONENTS,
        // MAX_FRAGMENT_UNIFORM_COMPONENTS, MAX_TEXTURE_COORDS.
        0x8869 => int(ATTRIBS as u32),
        0x8872 | 0x8b4c => int(16),
        0x8b4d => int(UNITS as u32),
        0x8824 | 0x8cdf => int(1),
        0x8b4b => int(64),
        0x8b4a | 0x8b49 => int(4096),
        0x8871 => int(8),
        // BLEND_SRC_RGB, BLEND_DST_RGB, BLEND_SRC_ALPHA, BLEND_DST_ALPHA, BLEND_EQUATION_RGB, BLEND_EQUATION_ALPHA,
        // DEPTH_FUNC, CULL_FACE_MODE, FRONT_FACE.
        0x80c9 => int(c.blend[0]),
        0x80c8 => int(c.blend[1]),
        0x80cb => int(c.blend[2]),
        0x80ca => int(c.blend[3]),
        0x8009 => int(c.equation[0]),
        0x883d => int(c.equation[1]),
        0x0b74 => int(c.depth_func),
        0x0b45 => int(c.cull_mode),
        0x0b46 => int(c.front_face),
        // The matrices, their mode and stacks' depths, and the compatibility profile's limits.
        0x0ba6..=0x0ba8 => {
            let which = 0x1700 + pname - 0x0ba6;
            c.compat.matrix(which, c.unit as usize).iter().map(|&f| Value::Float(f64::from(f))).collect()
        }
        0x0ba0 => int(c.compat.mode()),
        0x0d36 => int(32),
        0x0d38 | 0x0d39 => int(4),
        0x84e2 => int(compat::UNITS as u32),
        0x0d31 => int(8),
        0x0d32 => int(6),
        0x0b00 => c.attrib_values[3].iter().map(|&w| Value::Float(f64::from(f32::from_bits(w)))).collect(),
        0x0de1 => vec![Value::Bool(c.compat.textures[(c.unit as usize).min(compat::UNITS - 1)])],
        // Points and lines: one pixel; anisotropy: none; multisampling: none; the elements' and texture sizes.
        0x846d | 0x846e => vec![Value::Float(1.0), Value::Float(1.0)],
        0x0b11 | 0x0b21 => vec![Value::Float(1.0)],
        0x84ff => vec![Value::Float(1.0)],
        0x80a8 | 0x80a9 => int(0),
        0x8d57 => int(1),
        0x80e8 | 0x80e9 => int(1 << 20),
        0x0d50 => int(4),
        0x8073 | 0x851c => int(2048),
        0x88ff => int(256),
        0x84fd => vec![Value::Float(16.0)],
        // MAJOR_VERSION, MINOR_VERSION, NUM_EXTENSIONS (the guest answers it, from its list), CONTEXT_PROFILE_MASK.
        0x821b => int(3),
        0x821c => int(3),
        0x821d => int(0),
        0x9126 => int(1),
        cap if CAPS.iter().any(|k| k.0 == cap) => vec![Value::Bool(c.is_enabled(cap))],
        _ => return None,
    })
}

fn context(o: &mut Objects, id: u64) -> Result<&mut Context, String> {
    o.contexts.get_mut(&id).ok_or_else(|| if id == 0 { "a GL call with no context used".to_string() } else { format!("context {id:#x} isn't this driver's") })
}

/// What the current context's draws (or reads) go to: its drawable's default framebuffer, or the framebuffer object
/// bound, if that's complete.
fn framebuffer_of(o: &Objects, current: u64, read: bool) -> Option<Framebuffer> {
    let c = o.contexts.get(&current)?;
    let name = if read { c.read_fbo } else { c.draw_fbo };
    if name == 0 {
        return o.drawables.get(&c.drawable).copied();
    }
    framebuffer::resolve(o, name, read).ok()
}

impl Framebuffer {
    fn format_is_float(&self) -> bool {
        matches!(self.format, Format::Rgba16Float | Format::Rgba32Float | Format::R32Float | Format::Rg11b10Float)
    }
}

/// Which of red, green, blue and alpha each of a format's four channels is, in memory order.
fn channels(format: Format) -> [usize; 4] {
    match format {
        Format::Bgra8Unorm | Format::Bgra8Srgb => [2, 1, 0, 3],
        _ => [0, 1, 2, 3],
    }
}

/// A colour as one texel of a format, for the formats a part of a buffer is cleared in.
fn texel(format: Format, rgba: [f32; 4]) -> Option<Vec<u8>> {
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let srgb = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        unorm(if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 })
    };
    Some(match format {
        Format::Rgba8Unorm => rgba.map(unorm).to_vec(),
        Format::Bgra8Unorm => bgra8(rgba).to_vec(),
        Format::Rgba8Srgb => vec![srgb(rgba[0]), srgb(rgba[1]), srgb(rgba[2]), unorm(rgba[3])],
        Format::Bgra8Srgb => vec![srgb(rgba[2]), srgb(rgba[1]), srgb(rgba[0]), unorm(rgba[3])],
        Format::Rgba32Float => rgba.iter().flat_map(|v| v.to_le_bytes()).collect(),
        Format::Rgba16Float => rgba.iter().flat_map(|&v| draw::to_half(v).to_le_bytes()).collect(),
        _ => return None,
    })
}

/// One texel of a format read as RGBA bytes, as `glReadPixels` gives them.
fn rgba8(format: Format, t: &[u8]) -> [u8; 4] {
    let unorm = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    match format {
        Format::Bgra8Unorm | Format::Bgra8Srgb => [t[2], t[1], t[0], t[3]],
        Format::Rgba8Unorm | Format::Rgba8Srgb => [t[0], t[1], t[2], t[3]],
        Format::Rgba32Float => {
            let f = |k: usize| f64::from(f32::from_le_bytes([t[4 * k], t[4 * k + 1], t[4 * k + 2], t[4 * k + 3]]));
            [unorm(f(0)), unorm(f(1)), unorm(f(2)), unorm(f(3))]
        }
        Format::Rgba16Float => {
            let f = |k: usize| draw::half(u16::from_le_bytes([t[2 * k], t[2 * k + 1]]));
            [unorm(f(0)), unorm(f(1)), unorm(f(2)), unorm(f(3))]
        }
        Format::R8Unorm => [t[0], 0, 0, 255],
        Format::Rg8Unorm => [t[0], t[1], 0, 255],
        _ => [0, 0, 0, 255],
    }
}

/// Where two rectangles (`x`, `y`, `width`, `height`) overlap, if they do.
fn intersect(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// A colour in [0, 1] as a `Bgra8Unorm` texel.
fn bgra8(rgba: [f32; 4]) -> [u8; 4] {
    let b = |v: f32| (v * 255.0).round() as u8;
    [b(rgba[2]), b(rgba[1]), b(rgba[0]), b(rgba[3])]
}

fn malformed(_: records::Malformed) -> String {
    "the GL driver's records don't read".into()
}

fn device_error(e: gpu::Error) -> String {
    e.message
}
