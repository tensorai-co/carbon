//! A Vulkan driver's records ([`gpu_wire::records`]) lowered onto the device: one [`Context`] for each open of the
//! render node, which is one guest driver's instance. Compute (memory, pipelines, dispatches, copies, fills) and
//! graphics (images, samplers, graphics pipelines, render passes and draws, and copies between memory and images,
//! which is how a frame is read back to present it), with Vulkan's enums decoded here.
//!
//! The driver keeps buffers, descriptor sets and command buffers itself and sends memory, pipelines and work, naming
//! memory by its guest handle. Memory lives twice: the guest's copy, which the program maps and writes, and the
//! device's. A submission copies the ranges its commands use from the guest's copy to the device's, runs, and copies
//! back what it could have written, all before the doorbell returns, so the guest's memory is coherent and its fences
//! are signaled as soon as `vkQueueSubmit` returns. (Memory shared with the device without copies comes later.)

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use gpu::{Address, Bind, Compare, Cull, Factor, Filter, Format, FrontFace, IndexType, Load, Topology, VertexFormat};
use gpu_ir::BindingKind;
use gpu_wire::records::{self, Reader};
use gpu_wire::vk;

use crate::Gpu;

/// The guest's memory, as the kernel reaches it for the process that rang.
pub trait Guest {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), String>;
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String>;
}

/// How long a submission may run before the doorbell gives up on it.
const LONGEST: Duration = Duration::from_secs(600);
/// The bytes a dispatch's push constants are given in: at least Vulkan's 128, as a uniform buffer.
const PUSH_BYTES: usize = 256;

/// One guest driver instance's objects on the device.
pub struct Context {
    gpu: Gpu,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    memories: HashMap<u64, Memory>,
    pipelines: HashMap<u64, Pipeline>,
    images: HashMap<u64, Image>,
    samplers: HashMap<u64, gpu::Sampler>,
    /// The timeline submissions signal, and the last value given.
    fence: Option<gpu::Fence>,
    value: u64,
}

struct Memory {
    device: gpu::Memory,
    size: u64,
    /// Where the guest's copy is.
    guest: u64,
}

struct Pipeline {
    device: gpu::Pipeline,
    /// Its bindings: a compute pipeline's in the order a dispatch hands them over; a graphics pipeline's both stages',
    /// each (set, binding) once.
    bindings: Vec<gpu_ir::Binding>,
}

struct Image {
    device: gpu::Image,
    desc: gpu::ImageDesc,
    /// The memory it's bound to (0 for none) and where, and whether its texels are there, tightly packed (linear).
    memory: u64,
    offset: u64,
    linear: bool,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("vulkan::Context")
    }
}

/// Memory a planned command names: a guest allocation's, or bytes the submission brings (push constants, updates).
#[derive(Clone, Copy)]
enum Mem {
    Device(gpu::Memory),
    Temp(usize),
}

#[derive(Clone, Copy)]
struct Span {
    mem: Mem,
    offset: u64,
    size: u64,
}

/// What a draw binds at a (set, binding), before the submission's temporary memory exists.
#[derive(Clone, Copy)]
enum Binding {
    Buffer(Span),
    Image(gpu::Image),
    Sampler(gpu::Sampler),
    Combined(gpu::Image, gpu::Sampler),
}

/// A command, before the submission's temporary memory exists.
enum Planned {
    Dispatch { pipeline: gpu::Pipeline, bindings: Vec<Span>, groups: [u32; 3] },
    Copy { from: Span, to: gpu::Memory, at: u64 },
    Fill { range: Span, value: u32 },
    Begin { colors: Vec<gpu::ColorAttachment>, depth: Option<gpu::DepthAttachment> },
    Viewport(gpu::Viewport),
    Scissor(gpu::Rect),
    Draw { draw: gpu::Draw, bindings: Vec<(u32, u32, Binding)>, vertex: Vec<Span>, index: Option<(Span, IndexType)> },
    End,
    ToImage { from: Span, row: u32, to: gpu::Image, region: gpu::Rect },
    FromImage { from: gpu::Image, region: gpu::Rect, to: Span, row: u32 },
}

fn format(f: u32) -> Result<Format, String> {
    Ok(match f as i32 {
        vk::VK_FORMAT_R8G8B8A8_UNORM => Format::Rgba8Unorm,
        vk::VK_FORMAT_R8G8B8A8_SRGB => Format::Rgba8Srgb,
        vk::VK_FORMAT_B8G8R8A8_UNORM => Format::Bgra8Unorm,
        vk::VK_FORMAT_B8G8R8A8_SRGB => Format::Bgra8Srgb,
        vk::VK_FORMAT_R32G32B32A32_SFLOAT => Format::Rgba32Float,
        vk::VK_FORMAT_D32_SFLOAT => Format::D32Float,
        vk::VK_FORMAT_D16_UNORM => Format::D16Unorm,
        other => return Err(format!("images of Vulkan's format {other} aren't there yet")),
    })
}

fn vertex_format(f: u32) -> Result<VertexFormat, String> {
    Ok(match f as i32 {
        vk::VK_FORMAT_R32_SFLOAT => VertexFormat::F32(1),
        vk::VK_FORMAT_R32G32_SFLOAT => VertexFormat::F32(2),
        vk::VK_FORMAT_R32G32B32_SFLOAT => VertexFormat::F32(3),
        vk::VK_FORMAT_R32G32B32A32_SFLOAT => VertexFormat::F32(4),
        vk::VK_FORMAT_R32_UINT => VertexFormat::U32(1),
        vk::VK_FORMAT_R32G32_UINT => VertexFormat::U32(2),
        vk::VK_FORMAT_R32G32B32_UINT => VertexFormat::U32(3),
        vk::VK_FORMAT_R32G32B32A32_UINT => VertexFormat::U32(4),
        vk::VK_FORMAT_R32_SINT => VertexFormat::I32(1),
        vk::VK_FORMAT_R32G32_SINT => VertexFormat::I32(2),
        vk::VK_FORMAT_R32G32B32_SINT => VertexFormat::I32(3),
        vk::VK_FORMAT_R32G32B32A32_SINT => VertexFormat::I32(4),
        vk::VK_FORMAT_R8G8B8A8_UNORM => VertexFormat::Unorm8x4,
        other => Err(format!("vertex attributes of Vulkan's format {other} aren't there yet"))?,
    })
}

/// Vulkan's compare ops, blend factors and blend ops are numbered as the device model's are listed.
fn compare(op: u32) -> Result<Compare, String> {
    const ALL: [Compare; 8] = [Compare::Never, Compare::Less, Compare::Equal, Compare::LessOrEqual, Compare::Greater, Compare::NotEqual, Compare::GreaterOrEqual, Compare::Always];
    ALL.get(op as usize).copied().ok_or_else(|| format!("compare op {op}"))
}

fn factor(f: u32) -> Result<Factor, String> {
    use Factor::*;
    const ALL: [Factor; 10] = [Zero, One, SrcColor, OneMinusSrcColor, DstColor, OneMinusDstColor, SrcAlpha, OneMinusSrcAlpha, DstAlpha, OneMinusDstAlpha];
    ALL.get(f as usize).copied().ok_or_else(|| format!("blend factor {f} isn't there yet"))
}

fn blend_op(op: u32) -> Result<gpu::BlendOp, String> {
    use gpu::BlendOp::*;
    const ALL: [gpu::BlendOp; 5] = [Add, Subtract, ReverseSubtract, Min, Max];
    ALL.get(op as usize).copied().ok_or_else(|| format!("blend op {op} isn't there yet"))
}

fn filter(f: u32) -> Filter {
    if f as i32 == vk::VK_FILTER_LINEAR { Filter::Linear } else { Filter::Nearest }
}

fn address(a: u32) -> Address {
    match a as i32 {
        vk::VK_SAMPLER_ADDRESS_MODE_REPEAT => Address::Repeat,
        vk::VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT => Address::MirroredRepeat,
        // Clamping to a border colour isn't in the device model: its edge instead.
        _ => Address::ClampToEdge,
    }
}

fn load<T>(op: u32, clear: T) -> Load<T> {
    match op as i32 {
        vk::VK_ATTACHMENT_LOAD_OP_LOAD => Load::Load,
        vk::VK_ATTACHMENT_LOAD_OP_CLEAR => Load::Clear(clear),
        _ => Load::DontCare,
    }
}

/// An entry point of a stage in a SPIR-V module: its name, and the module, as a record has them.
fn entry(p: &mut Reader<'_>, stage: gpu_ir::Stage) -> Result<gpu_ir::Entry, String> {
    let name = String::from_utf8_lossy(p.bytes().map_err(malformed)?).into_owned();
    let module = gpu::spirv(p.bytes().map_err(malformed)?).map_err(device_error)?;
    module.entries.into_iter().find(|e| e.name == name && e.stage == stage).ok_or_else(|| format!("the shader module has no {stage:?} entry point {name:?}"))
}

fn malformed(_: records::Malformed) -> String {
    "the Vulkan driver's records don't read".into()
}

fn device_error(e: gpu::Error) -> String {
    e.message
}

/// Ranges of each guest allocation a submission touches, merged.
#[derive(Default)]
struct Touched(BTreeMap<u64, Vec<(u64, u64)>>);

impl Touched {
    fn add(&mut self, memory: u64, offset: u64, size: u64) {
        self.0.entry(memory).or_default().push((offset, offset.saturating_add(size)));
    }

    /// Each allocation's ranges, sorted and merged, as (offset, length).
    fn merged(&self) -> Vec<(u64, u64, u64)> {
        let mut out = Vec::new();
        for (&memory, ranges) in &self.0 {
            let mut ranges = ranges.clone();
            ranges.sort();
            let mut current: Option<(u64, u64)> = None;
            for (start, end) in ranges {
                current = match current {
                    Some((s, e)) if start <= e => Some((s, e.max(end))),
                    Some((s, e)) => {
                        out.push((memory, s, e - s));
                        Some((start, end))
                    }
                    None => Some((start, end)),
                };
            }
            if let Some((s, e)) = current {
                out.push((memory, s, e - s));
            }
        }
        out
    }
}

impl Context {
    pub fn new(gpu: Gpu) -> Context {
        Context { gpu, state: Mutex::new(State::default()) }
    }

    /// Runs a doorbell's records: what the driver made and freed, and the work it submitted, finished on return.
    pub fn execute(&self, records: &[u8], guest: &mut dyn Guest) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut r = Reader::new(records);
        while !r.is_empty() {
            let (kind, mut p) = r.record().map_err(malformed)?;
            match kind {
                records::MEMORY => {
                    let (id, size, at) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?);
                    let device = self.gpu.run(move |d| d.alloc(size.max(4))).map_err(device_error)?.map_err(device_error)?;
                    state.memories.insert(id, Memory { device, size, guest: at });
                }
                records::FREE_MEMORY => {
                    let id = p.u64().map_err(malformed)?;
                    if let Some(m) = state.memories.remove(&id) {
                        self.gpu.run(move |d| d.free(m.device)).map_err(device_error)?;
                    }
                }
                records::PIPELINE => {
                    let id = p.u64().map_err(malformed)?;
                    let name = String::from_utf8_lossy(p.bytes().map_err(malformed)?).into_owned();
                    let spirv = p.bytes().map_err(malformed)?;
                    let module = gpu::spirv(spirv).map_err(device_error)?;
                    let entry = module
                        .entries
                        .into_iter()
                        .find(|e| e.name == name && e.stage == gpu_ir::Stage::Compute)
                        .ok_or_else(|| format!("the shader module has no compute entry point {name:?}"))?;
                    let bindings = entry.bindings.clone();
                    let device = self.gpu.run(move |d| d.pipeline(&entry)).map_err(device_error)?.map_err(device_error)?;
                    state.pipelines.insert(id, Pipeline { device, bindings });
                }
                records::FREE_PIPELINE => {
                    state.pipelines.remove(&p.u64().map_err(malformed)?);
                }
                records::IMAGE => {
                    let (id, memory, offset) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?);
                    let (f, width, height, linear) = (p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)? != 0);
                    let desc = gpu::ImageDesc { format: format(f)?, width, height };
                    let device = self.gpu.run(move |d| d.image(desc)).map_err(device_error)?.map_err(device_error)?;
                    if let Some(old) = state.images.insert(id, Image { device, desc, memory, offset, linear }) {
                        let _ = self.gpu.run(move |d| d.free_image(old.device));
                    }
                }
                records::FREE_IMAGE => {
                    if let Some(image) = state.images.remove(&p.u64().map_err(malformed)?) {
                        self.gpu.run(move |d| d.free_image(image.device)).map_err(device_error)?;
                    }
                }
                records::SAMPLER => {
                    let id = p.u64().map_err(malformed)?;
                    let (mag, min, u, v) = (p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?);
                    let desc = gpu::SamplerDesc { mag: filter(mag), min: filter(min), address: [address(u), address(v)] };
                    let device = self.gpu.run(move |d| d.sampler(desc)).map_err(device_error)?.map_err(device_error)?;
                    state.samplers.insert(id, device);
                }
                records::FREE_SAMPLER => {
                    state.samplers.remove(&p.u64().map_err(malformed)?);
                }
                records::GRAPHICS => {
                    let id = p.u64().map_err(malformed)?;
                    let (pipeline, bindings) = self.graphics(&mut p)?;
                    state.pipelines.insert(id, Pipeline { device: pipeline, bindings });
                }
                records::SUBMIT => self.submit(&mut state, p, guest)?,
                other => return Err(format!("a record of kind {other} this host doesn't know")),
            }
        }
        Ok(())
    }

    /// A graphics pipeline's record, made on the device: the pipeline, and its stages' bindings.
    fn graphics(&self, p: &mut Reader<'_>) -> Result<(gpu::Pipeline, Vec<gpu_ir::Binding>), String> {
        let vertex = entry(p, gpu_ir::Stage::Vertex)?;
        let fragment = entry(p, gpu_ir::Stage::Fragment)?;
        let n = p.u32().map_err(malformed)?;
        let buffers: Vec<gpu::VertexBuffer> = (0..n).map(|_| Ok(gpu::VertexBuffer { stride: p.u32()?, per_instance: p.u32()? != 0 })).collect::<Result<_, records::Malformed>>().map_err(malformed)?;
        let n = p.u32().map_err(malformed)?;
        let mut attributes = Vec::new();
        for _ in 0..n {
            let (location, buffer, offset, f) = (p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?);
            attributes.push(gpu::Attribute { location, buffer, offset, format: vertex_format(f)? });
        }
        let topology = match p.u32().map_err(malformed)? as i32 {
            vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST => Topology::TriangleList,
            vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP => Topology::TriangleStrip,
            vk::VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN => Topology::TriangleFan,
            other => return Err(format!("topology {other}: only triangles are drawn yet")),
        };
        let cull = match p.u32().map_err(malformed)? {
            vk::VK_CULL_MODE_NONE => Cull::None,
            vk::VK_CULL_MODE_FRONT_BIT => Cull::Front,
            vk::VK_CULL_MODE_BACK_BIT => Cull::Back,
            other => return Err(format!("cull mode {other} isn't there yet")),
        };
        let front = if p.u32().map_err(malformed)? as i32 == vk::VK_FRONT_FACE_CLOCKWISE { FrontFace::Clockwise } else { FrontFace::CounterClockwise };
        let (test, op, write, depth_format) = (p.u32().map_err(malformed)? != 0, p.u32().map_err(malformed)?, p.u32().map_err(malformed)? != 0, p.u32().map_err(malformed)?);
        let depth = if test { Some(gpu::DepthState { compare: compare(op)?, write }) } else { None };
        let depth_format = if depth_format == 0 { None } else { Some(format(depth_format)?) };
        let n = p.u32().map_err(malformed)?;
        let mut colors = Vec::new();
        for _ in 0..n {
            let f = format(p.u32().map_err(malformed)?)?;
            let on = p.u32().map_err(malformed)? != 0;
            let mut w = [0u32; 6];
            for v in &mut w {
                *v = p.u32().map_err(malformed)?;
            }
            let mask = p.u32().map_err(malformed)?;
            let blend = if on { Some(gpu::Blend { color: (factor(w[0])?, factor(w[1])?, blend_op(w[2])?), alpha: (factor(w[3])?, factor(w[4])?, blend_op(w[5])?) }) } else { None };
            colors.push(gpu::ColorTarget { format: f, blend, write: [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0, mask & 8 != 0] });
        }
        let mut bindings: Vec<gpu_ir::Binding> = Vec::new();
        for b in vertex.bindings.iter().chain(&fragment.bindings) {
            if !bindings.iter().any(|x| x.set == b.set && x.binding == b.binding) {
                bindings.push(*b);
            }
        }
        let pipeline = self
            .gpu
            .run(move |d| {
                d.graphics(&gpu::GraphicsDesc { vertex: &vertex, fragment: &fragment, buffers: &buffers, attributes: &attributes, topology, cull, front, depth, depth_format, colors: &colors })
            })
            .map_err(device_error)?
            .map_err(device_error)?;
        Ok((pipeline, bindings))
    }

    fn submit(&self, state: &mut State, mut r: Reader<'_>, guest: &mut dyn Guest) -> Result<(), String> {
        let mut plan = Vec::new();
        // Linear images a draw samples, read from their memory at the start.
        let mut refresh: Vec<u64> = Vec::new();
        let mut temps: Vec<Vec<u8>> = Vec::new();
        let (mut read, mut written) = (Touched::default(), Touched::default());
        let memory = |state: &State, id: u64| state.memories.get(&id).map(|m| m.device).ok_or_else(|| format!("memory {id:#x} isn't this driver's"));
        let image = |state: &State, id: u64| state.images.get(&id).map(|i| i.device).ok_or_else(|| format!("image {id:#x} isn't this driver's"));
        let rect = |p: &mut Reader<'_>| -> Result<gpu::Rect, String> { Ok(gpu::Rect { x: p.u32().map_err(malformed)?, y: p.u32().map_err(malformed)?, width: p.u32().map_err(malformed)?, height: p.u32().map_err(malformed)? }) };
        while !r.is_empty() {
            let (kind, mut p) = r.record().map_err(malformed)?;
            match kind {
                records::DISPATCH => {
                    let id = p.u64().map_err(malformed)?;
                    let groups = [p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?];
                    let (count, push_len) = (p.u32().map_err(malformed)?, p.u32().map_err(malformed)?);
                    let bound: Vec<records::Bound> = (0..count).map(|_| p.bound()).collect::<Result<_, _>>().map_err(malformed)?;
                    let push = p.bytes().map_err(malformed)?;
                    let push = &push[..(push_len as usize).min(push.len())];
                    let pipeline = state.pipelines.get(&id).ok_or_else(|| format!("pipeline {id:#x} isn't this driver's"))?;
                    let mut bindings = Vec::new();
                    for b in &pipeline.bindings {
                        if b.set == gpu_ir::PUSH_CONSTANTS {
                            let mut bytes = push.to_vec();
                            bytes.resize(PUSH_BYTES.max(push.len().next_multiple_of(4)), 0);
                            bindings.push(Span { mem: Mem::Temp(temps.len()), offset: 0, size: bytes.len() as u64 });
                            temps.push(bytes);
                            continue;
                        }
                        if !b.kind.is_buffer() {
                            return Err(format!("set {} binding {}: images in compute pipelines aren't there yet", b.set, b.binding));
                        }
                        let found = bound.iter().find(|x| x.set == b.set && x.binding == b.binding).ok_or_else(|| format!("set {} binding {} isn't bound to a buffer", b.set, b.binding))?;
                        bindings.push(Span { mem: Mem::Device(memory(state, found.memory)?), offset: found.offset, size: found.size });
                        read.add(found.memory, found.offset, found.size);
                        if matches!(b.kind, BindingKind::Storage { writable: true }) {
                            written.add(found.memory, found.offset, found.size);
                        }
                    }
                    plan.push(Planned::Dispatch { pipeline: pipeline.device, bindings, groups });
                }
                records::COPY => {
                    let (from, from_off, to, to_off, size) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?);
                    read.add(from, from_off, size);
                    read.add(to, to_off, size);
                    written.add(to, to_off, size);
                    plan.push(Planned::Copy { from: Span { mem: Mem::Device(memory(state, from)?), offset: from_off, size }, to: memory(state, to)?, at: to_off });
                }
                records::FILL => {
                    let (id, offset, size, value) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u32().map_err(malformed)?);
                    read.add(id, offset, size);
                    written.add(id, offset, size);
                    plan.push(Planned::Fill { range: Span { mem: Mem::Device(memory(state, id)?), offset, size }, value });
                }
                records::UPDATE => {
                    let (id, offset) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?);
                    let bytes = p.bytes().map_err(malformed)?.to_vec();
                    let size = bytes.len() as u64;
                    read.add(id, offset, size);
                    written.add(id, offset, size);
                    plan.push(Planned::Copy { from: Span { mem: Mem::Temp(temps.len()), offset: 0, size }, to: memory(state, id)?, at: offset });
                    temps.push(bytes);
                }
                records::BEGIN_RENDER => {
                    let n = p.u32().map_err(malformed)?;
                    let mut colors = Vec::new();
                    for _ in 0..n {
                        let (id, op) = (p.u64().map_err(malformed)?, p.u32().map_err(malformed)?);
                        let clear = [p.f32().map_err(malformed)?, p.f32().map_err(malformed)?, p.f32().map_err(malformed)?, p.f32().map_err(malformed)?];
                        colors.push(gpu::ColorAttachment { image: image(state, id)?, load: load(op, clear) });
                    }
                    let (id, op, clear) = (p.u64().map_err(malformed)?, p.u32().map_err(malformed)?, p.f32().map_err(malformed)?);
                    let depth = if id == 0 { None } else { Some(gpu::DepthAttachment { image: image(state, id)?, load: load(op, clear) }) };
                    plan.push(Planned::Begin { colors, depth });
                }
                records::VIEWPORT => {
                    let v: Vec<f32> = (0..6).map(|_| p.f32()).collect::<Result<_, _>>().map_err(malformed)?;
                    plan.push(Planned::Viewport(gpu::Viewport { x: v[0], y: v[1], width: v[2], height: v[3], min_depth: v[4], max_depth: v[5] }));
                }
                records::SCISSOR => plan.push(Planned::Scissor(rect(&mut p)?)),
                records::END_RENDER => plan.push(Planned::End),
                records::DRAW => {
                    let id = p.u64().map_err(malformed)?;
                    let (first, count, vertex_offset, first_instance, instances) = (p.u32().map_err(malformed)?, p.u32().map_err(malformed)?, p.u32().map_err(malformed)? as i32, p.u32().map_err(malformed)?, p.u32().map_err(malformed)?);
                    let n = p.u32().map_err(malformed)?;
                    let bound: Vec<records::Bound> = (0..n).map(|_| p.bound()).collect::<Result<_, _>>().map_err(malformed)?;
                    let push = p.bytes().map_err(malformed)?.to_vec();
                    let n = p.u32().map_err(malformed)?;
                    let images: Vec<records::BoundImage> = (0..n).map(|_| p.bound_image()).collect::<Result<_, _>>().map_err(malformed)?;
                    let n = p.u32().map_err(malformed)?;
                    let mut vertex = Vec::new();
                    for _ in 0..n {
                        let (m, offset, size) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?);
                        read.add(m, offset, size);
                        vertex.push(Span { mem: Mem::Device(memory(state, m)?), offset, size });
                    }
                    let (m, offset, size, ty) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u32().map_err(malformed)?);
                    let index = if m == 0 {
                        None
                    } else {
                        read.add(m, offset, size);
                        Some((Span { mem: Mem::Device(memory(state, m)?), offset, size }, if ty as i32 == vk::VK_INDEX_TYPE_UINT16 { IndexType::U16 } else { IndexType::U32 }))
                    };
                    let pipeline = state.pipelines.get(&id).ok_or_else(|| format!("pipeline {id:#x} isn't this driver's"))?;
                    let mut bindings = Vec::new();
                    for b in &pipeline.bindings {
                        let what = if b.set == gpu_ir::PUSH_CONSTANTS {
                            let mut bytes = push.clone();
                            bytes.resize(PUSH_BYTES.max(push.len().next_multiple_of(4)), 0);
                            let span = Span { mem: Mem::Temp(temps.len()), offset: 0, size: bytes.len() as u64 };
                            temps.push(bytes);
                            Binding::Buffer(span)
                        } else if b.kind.is_buffer() {
                            let found = bound.iter().find(|x| x.set == b.set && x.binding == b.binding).ok_or_else(|| format!("set {} binding {} isn't bound to a buffer", b.set, b.binding))?;
                            read.add(found.memory, found.offset, found.size);
                            if matches!(b.kind, BindingKind::Storage { writable: true }) {
                                written.add(found.memory, found.offset, found.size);
                            }
                            Binding::Buffer(Span { mem: Mem::Device(memory(state, found.memory)?), offset: found.offset, size: found.size })
                        } else {
                            let found = images.iter().find(|x| x.set == b.set && x.binding == b.binding).ok_or_else(|| format!("set {} binding {} isn't bound to an image or sampler", b.set, b.binding))?;
                            let sampler = || state.samplers.get(&found.sampler).copied().ok_or_else(|| format!("set {} binding {} has no sampler", b.set, b.binding));
                            if found.image != 0 && state.images.get(&found.image).is_some_and(|i| i.linear) && !refresh.contains(&found.image) {
                                refresh.push(found.image);
                            }
                            match b.kind {
                                BindingKind::Image => Binding::Image(image(state, found.image)?),
                                BindingKind::Sampler => Binding::Sampler(sampler()?),
                                _ => Binding::Combined(image(state, found.image)?, sampler()?),
                            }
                        };
                        bindings.push((b.set, b.binding, what));
                    }
                    let draw = gpu::Draw { pipeline: pipeline.device, bindings: Vec::new(), vertex_buffers: Vec::new(), index: None, first, count, vertex_offset, first_instance, instances };
                    plan.push(Planned::Draw { draw, bindings, vertex, index });
                }
                records::COPY_TO_IMAGE => {
                    let (m, offset, row, id) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u32().map_err(malformed)?, p.u64().map_err(malformed)?);
                    let region = rect(&mut p)?;
                    let i = state.images.get(&id).ok_or_else(|| format!("image {id:#x} isn't this driver's"))?;
                    let size = u64::from(row.max(region.width)) * u64::from(region.height) * u64::from(i.desc.format.bytes());
                    read.add(m, offset, size);
                    plan.push(Planned::ToImage { from: Span { mem: Mem::Device(memory(state, m)?), offset, size }, row, to: i.device, region });
                }
                records::COPY_FROM_IMAGE => {
                    let id = p.u64().map_err(malformed)?;
                    let region = rect(&mut p)?;
                    let (m, offset, row) = (p.u64().map_err(malformed)?, p.u64().map_err(malformed)?, p.u32().map_err(malformed)?);
                    let i = state.images.get(&id).ok_or_else(|| format!("image {id:#x} isn't this driver's"))?;
                    let size = u64::from(row.max(region.width)) * u64::from(region.height) * u64::from(i.desc.format.bytes());
                    written.add(m, offset, size);
                    plan.push(Planned::FromImage { from: i.device, region, to: Span { mem: Mem::Device(memory(state, m)?), offset, size }, row });
                }
                other => return Err(format!("a command of kind {other} this host doesn't know")),
            }
        }
        // Linear images sampled: their texels from their memory, first.
        let mut first = Vec::new();
        for id in refresh {
            let i = &state.images[&id];
            if i.memory == 0 {
                continue;
            }
            let size = u64::from(i.desc.width) * u64::from(i.desc.height) * u64::from(i.desc.format.bytes());
            read.add(i.memory, i.offset, size);
            let region = gpu::Rect { x: 0, y: 0, width: i.desc.width, height: i.desc.height };
            first.push(Planned::ToImage { from: Span { mem: Mem::Device(memory(state, i.memory)?), offset: i.offset, size }, row: 0, to: i.device, region });
        }
        first.append(&mut plan);
        let plan = first;
        // The guest's bytes for every range the work reads, then the work, then what it may have written, back.
        let mut uploads = Vec::new();
        for (id, offset, len) in read.merged() {
            let m = &state.memories[&id];
            let len = len.min(m.size.saturating_sub(offset));
            let mut bytes = vec![0; len as usize];
            guest.read(m.guest + offset, &mut bytes)?;
            uploads.push((m.device, offset, bytes));
        }
        let downloads: Vec<(u64, gpu::Memory, u64, u64)> = written
            .merged()
            .into_iter()
            .map(|(id, offset, len)| {
                let m = &state.memories[&id];
                (m.guest, m.device, offset, len.min(m.size.saturating_sub(offset)))
            })
            .collect();
        let wanted: Vec<(gpu::Memory, u64, u64)> = downloads.iter().map(|d| (d.1, d.2, d.3)).collect();
        state.value += 1;
        let (fence, value) = (state.fence, state.value);
        let (fence, back) = self.gpu.run(move |d| run(d, fence, value, uploads, temps, plan, wanted)).map_err(device_error)??;
        state.fence = Some(fence);
        for ((at, _, offset, _), bytes) in downloads.iter().zip(back) {
            guest.write(at + offset, &bytes)?;
        }
        Ok(())
    }
}

/// A submission on the service's thread: the uploads, the temporary memory, the commands, the wait, the downloads.
#[allow(clippy::type_complexity)]
fn run(
    d: &mut gpu::Device,
    fence: Option<gpu::Fence>,
    value: u64,
    uploads: Vec<(gpu::Memory, u64, Vec<u8>)>,
    temps: Vec<Vec<u8>>,
    plan: Vec<Planned>,
    wanted: Vec<(gpu::Memory, u64, u64)>,
) -> Result<(gpu::Fence, Vec<Vec<u8>>), String> {
    let fence = match fence {
        Some(f) => f,
        None => d.fence().map_err(device_error)?,
    };
    for (memory, offset, bytes) in &uploads {
        d.write(*memory, *offset, bytes).map_err(device_error)?;
    }
    let mut made = Vec::new();
    let result = (|| {
        for bytes in &temps {
            let m = d.alloc(bytes.len().max(4) as u64).map_err(device_error)?;
            made.push(m);
            d.write(m, 0, bytes).map_err(device_error)?;
        }
        let range = |s: Span| gpu::Range {
            memory: match s.mem {
                Mem::Device(m) => m,
                Mem::Temp(i) => made[i],
            },
            offset: s.offset,
            size: s.size,
        };
        let commands: Vec<gpu::Command> = plan
            .iter()
            .map(|c| match c {
                Planned::Dispatch { pipeline, bindings, groups } => gpu::Command::Dispatch { pipeline: *pipeline, bindings: bindings.iter().map(|s| range(*s)).collect(), groups: *groups },
                Planned::Copy { from, to, at } => gpu::Command::Copy { from: range(*from), to: *to, at: *at },
                Planned::Fill { range: r, value } => gpu::Command::Fill { range: range(*r), value: *value },
                Planned::Begin { colors, depth } => gpu::Command::BeginRender { colors: colors.clone(), depth: *depth },
                Planned::Viewport(v) => gpu::Command::SetViewport(*v),
                Planned::Scissor(r) => gpu::Command::SetScissor(*r),
                Planned::End => gpu::Command::EndRender,
                Planned::Draw { draw, bindings, vertex, index } => {
                    let mut draw = draw.clone();
                    draw.bindings = bindings
                        .iter()
                        .map(|(set, binding, b)| {
                            let bind = match b {
                                Binding::Buffer(s) => Bind::Buffer(range(*s)),
                                Binding::Image(i) => Bind::Image(*i),
                                Binding::Sampler(s) => Bind::Sampler(*s),
                                Binding::Combined(i, s) => Bind::Combined(*i, *s),
                            };
                            (*set, *binding, bind)
                        })
                        .collect();
                    draw.vertex_buffers = vertex.iter().map(|s| range(*s)).collect();
                    draw.index = index.map(|(s, t)| (range(s), t));
                    gpu::Command::Draw(draw)
                }
                Planned::ToImage { from, row, to, region } => gpu::Command::CopyBufferToImage { from: range(*from), row_pixels: *row, to: *to, region: *region },
                Planned::FromImage { from, region, to, row } => gpu::Command::CopyImageToBuffer { from: *from, region: *region, to: range(*to), row_pixels: *row },
            })
            .collect();
        d.submit(gpu::Submit { commands: &commands, wait: &[], signal: &[(fence, value)] }).map_err(device_error)?;
        if !d.wait(fence, value, LONGEST).map_err(device_error)? {
            return Err(format!("the work submitted didn't finish in {} s", LONGEST.as_secs()));
        }
        let mut back = Vec::new();
        for (memory, offset, len) in &wanted {
            let mut bytes = vec![0; *len as usize];
            d.read(*memory, *offset, &mut bytes).map_err(device_error)?;
            back.push(bytes);
        }
        Ok(back)
    })();
    for m in made {
        d.free(m);
    }
    result.map(|back| (fence, back))
}

impl Drop for Context {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        let memories: Vec<gpu::Memory> = state.memories.drain().map(|(_, m)| m.device).collect();
        let images: Vec<gpu::Image> = state.images.drain().map(|(_, i)| i.device).collect();
        let _ = self.gpu.run(move |d| {
            for m in memories {
                d.free(m);
            }
            for i in images {
                d.free_image(i);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpu_wire::records::*;

    /// A program's memory, as a flat array at address 0x1000.
    struct Flat(Vec<u8>);

    impl Guest for Flat {
        fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), String> {
            let at = (addr - 0x1000) as usize;
            out.copy_from_slice(&self.0[at..at + out.len()]);
            Ok(())
        }
        fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), String> {
            let at = (addr - 0x1000) as usize;
            self.0[at..at + data.len()].copy_from_slice(data);
            Ok(())
        }
    }

    #[test]
    fn copies_fills_and_updates_reach_the_guest() {
        let gpu = Gpu::open(gpu::Choice::Software).unwrap().unwrap();
        let context = Context::new(gpu);
        let mut guest = Flat((0..64u8).collect());
        let mut out = Vec::new();
        let mut p = Vec::new();
        for v in [7u64, 64, 0x1000] {
            put_u64(&mut p, v);
        }
        record(&mut out, MEMORY, &p);
        let mut commands = Vec::new();
        let mut c = Vec::new();
        for v in [7u64, 0, 7, 32, 16] {
            put_u64(&mut c, v);
        }
        record(&mut commands, COPY, &c);
        c.clear();
        for v in [7u64, 48, 8] {
            put_u64(&mut c, v);
        }
        put_u32(&mut c, 0xaabb_ccdd);
        record(&mut commands, FILL, &c);
        c.clear();
        put_u64(&mut c, 7);
        put_u64(&mut c, 60);
        put_bytes(&mut c, &[1, 2, 3, 4]);
        record(&mut commands, UPDATE, &c);
        record(&mut out, SUBMIT, &commands);
        context.execute(&out, &mut guest).unwrap();
        let g = &guest.0;
        assert_eq!(&g[32..48], &(0..16u8).collect::<Vec<_>>()[..]);
        assert_eq!(&g[48..56], &[0xdd, 0xcc, 0xbb, 0xaa, 0xdd, 0xcc, 0xbb, 0xaa]);
        assert_eq!(&g[56..60], &[56, 57, 58, 59]);
        assert_eq!(&g[60..64], &[1, 2, 3, 4]);
    }
}
