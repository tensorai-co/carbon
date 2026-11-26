//! Direct3D 11's semantics, host-side, lowered onto the device: what atlas's `d3d11.dll` and `dxgi.dll` call once
//! they've read a COM method's arguments from the program's memory.
//!
//! Objects are named by [`Id`]s, and described with Direct3D 11's own numbers (`D3D11_USAGE_*`, `DXGI_FORMAT_*`,
//! `D3D11_FILTER_*`), so a caller passes a program's structures through as they are. A [`Device`] is the device and
//! its immediate context together (Direct3D 11 has one of each per device).
//!
//! - **Resources**: buffers are device memory; 2D textures are images with their mip levels and array slices;
//!   staging textures (no bind flags) are memory holding their subresources tightly packed, which is what copies to
//!   and from them and their maps read and write. Dynamic resources keep a copy on the host for `Map`.
//! - **The immediate context** records commands and submits them when something needs their results: a map that
//!   reads, a read-back, `Flush`. An update (`UpdateSubresource`, a dynamic resource's `Unmap`) is an upload ordered
//!   among the commands, so a constant buffer changed between two draws is seen by each as it was then. Consecutive
//!   draws to the same targets are one render pass; a clear is a pass of its own.
//! - **Pipelines** are made at a draw from the state that shapes one (shaders, input layout and vertex strides,
//!   topology, blend, depth and rasterizer state, the targets' formats) and kept by it. The pixel shader's bindings
//!   are moved to sets 4 to 7, since each of Direct3D's stages has tables of its own; its inputs are linked to the
//!   vertex shader's outputs by semantic, and the input layout's elements to the vertex shader's inputs the same way.
//! - **Coordinates**: Direct3D's y points up in normalized device coordinates and the device's (Vulkan's) down, so
//!   the viewport is flipped; windings are then the same on the screen as Direct3D's.
//!
//! Not yet: stencil (a depth-stencil state's stencil test is ignored), multisampling, texture arrays as shader
//! resources (layer 0 is read), views that reinterpret a format, typed buffers and texture UAVs, lines and points,
//! deferred contexts, queries.

use std::collections::HashMap;
use std::time::Duration;

use gpu::dxbc::{self, Element};
use gpu::ir::{BindingKind, Entry, Slot};
use gpu::{Address, Bind, Blend, BlendOp, Compare, Cull, Factor, Filter, Format, FrontFace, IndexType, Load, Rect, Sub, Topology, VertexFormat};

use crate::Gpu;

/// An object of the device's.
pub type Id = u64;

/// Why a call failed, as the HRESULT the program sees and a message for traces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub hresult: u32,
    pub message: String,
}

pub const E_INVALIDARG: u32 = 0x8007_0057;
pub const E_NOTIMPL: u32 = 0x8000_4001;
pub const E_OUTOFMEMORY: u32 = 0x8007_000e;
pub const DXGI_ERROR_DEVICE_REMOVED: u32 = 0x887a_0005;

fn invalid<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error { hresult: E_INVALIDARG, message: message.into() })
}

fn unsupported<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error { hresult: E_NOTIMPL, message: message.into() })
}

fn device(e: gpu::Error) -> Error {
    Error { hresult: DXGI_ERROR_DEVICE_REMOVED, message: e.message }
}

/// `D3D11_USAGE`.
pub mod usage {
    pub const DEFAULT: u32 = 0;
    pub const IMMUTABLE: u32 = 1;
    pub const DYNAMIC: u32 = 2;
    pub const STAGING: u32 = 3;
}

/// `D3D11_BIND_FLAG`.
pub mod bind {
    pub const VERTEX_BUFFER: u32 = 0x1;
    pub const INDEX_BUFFER: u32 = 0x2;
    pub const CONSTANT_BUFFER: u32 = 0x4;
    pub const SHADER_RESOURCE: u32 = 0x8;
    pub const STREAM_OUTPUT: u32 = 0x10;
    pub const RENDER_TARGET: u32 = 0x20;
    pub const DEPTH_STENCIL: u32 = 0x40;
    pub const UNORDERED_ACCESS: u32 = 0x80;
}

/// `D3D11_RESOURCE_MISC_FLAG`s that change what a resource is.
pub mod misc {
    pub const GENERATE_MIPS: u32 = 0x1;
    pub const TEXTURECUBE: u32 = 0x4;
    pub const BUFFER_ALLOW_RAW_VIEWS: u32 = 0x20;
    pub const BUFFER_STRUCTURED: u32 = 0x40;
}

/// `D3D11_MAP`.
pub mod map {
    pub const READ: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const READ_WRITE: u32 = 3;
    pub const WRITE_DISCARD: u32 = 4;
    pub const WRITE_NO_OVERWRITE: u32 = 5;
}

/// The `DXGI_FORMAT`s this layer knows.
pub mod format {
    pub const UNKNOWN: u32 = 0;
    pub const R32G32B32A32_TYPELESS: u32 = 1;
    pub const R32G32B32A32_FLOAT: u32 = 2;
    pub const R32G32B32A32_UINT: u32 = 3;
    pub const R32G32B32A32_SINT: u32 = 4;
    pub const R32G32B32_FLOAT: u32 = 6;
    pub const R32G32B32_UINT: u32 = 7;
    pub const R32G32B32_SINT: u32 = 8;
    pub const R16G16B16A16_TYPELESS: u32 = 9;
    pub const R16G16B16A16_FLOAT: u32 = 10;
    pub const R32G32_FLOAT: u32 = 16;
    pub const R32G32_UINT: u32 = 17;
    pub const R32G32_SINT: u32 = 18;
    pub const R10G10B10A2_TYPELESS: u32 = 23;
    pub const R10G10B10A2_UNORM: u32 = 24;
    pub const R11G11B10_FLOAT: u32 = 26;
    pub const R8G8B8A8_TYPELESS: u32 = 27;
    pub const R8G8B8A8_UNORM: u32 = 28;
    pub const R8G8B8A8_UNORM_SRGB: u32 = 29;
    pub const R32_TYPELESS: u32 = 39;
    pub const D32_FLOAT: u32 = 40;
    pub const R32_FLOAT: u32 = 41;
    pub const R32_UINT: u32 = 42;
    pub const R32_SINT: u32 = 43;
    pub const R24G8_TYPELESS: u32 = 44;
    pub const D24_UNORM_S8_UINT: u32 = 45;
    pub const R24_UNORM_X8_TYPELESS: u32 = 46;
    pub const R8G8_TYPELESS: u32 = 48;
    pub const R8G8_UNORM: u32 = 49;
    pub const R16_TYPELESS: u32 = 53;
    pub const D16_UNORM: u32 = 55;
    pub const R16_UNORM: u32 = 56;
    pub const R16_UINT: u32 = 57;
    pub const R8_TYPELESS: u32 = 60;
    pub const R8_UNORM: u32 = 61;
    pub const A8_UNORM: u32 = 65;
    pub const B8G8R8A8_UNORM: u32 = 87;
    pub const B8G8R8X8_UNORM: u32 = 88;
    pub const B8G8R8A8_TYPELESS: u32 = 90;
    pub const B8G8R8A8_UNORM_SRGB: u32 = 91;
    pub const B8G8R8X8_TYPELESS: u32 = 92;
    pub const B8G8R8X8_UNORM_SRGB: u32 = 93;
}

/// A texture's format as the device's: a typeless format by what its bind flags say it will be (a depth format if
/// it's a depth-stencil), and 24-bit depth as 32-bit float, which every device has.
pub fn texture_format(dxgi: u32, bind_flags: u32) -> Result<Format, Error> {
    use format::*;
    let depth = bind_flags & bind::DEPTH_STENCIL != 0;
    Ok(match dxgi {
        R8G8B8A8_TYPELESS | R8G8B8A8_UNORM => Format::Rgba8Unorm,
        R8G8B8A8_UNORM_SRGB => Format::Rgba8Srgb,
        B8G8R8A8_TYPELESS | B8G8R8A8_UNORM | B8G8R8X8_UNORM | B8G8R8X8_TYPELESS => Format::Bgra8Unorm,
        B8G8R8A8_UNORM_SRGB | B8G8R8X8_UNORM_SRGB => Format::Bgra8Srgb,
        R32G32B32A32_TYPELESS | R32G32B32A32_FLOAT => Format::Rgba32Float,
        R16G16B16A16_TYPELESS | R16G16B16A16_FLOAT => Format::Rgba16Float,
        R10G10B10A2_TYPELESS | R10G10B10A2_UNORM => Format::Rgb10a2Unorm,
        R11G11B10_FLOAT => Format::Rg11b10Float,
        R8G8_TYPELESS | R8G8_UNORM => Format::Rg8Unorm,
        R8_TYPELESS | R8_UNORM => Format::R8Unorm,
        A8_UNORM => Format::A8Unorm,
        R32_TYPELESS if depth => Format::D32Float,
        R32_TYPELESS | R32_FLOAT => Format::R32Float,
        D32_FLOAT => Format::D32Float,
        R24G8_TYPELESS | D24_UNORM_S8_UINT | R24_UNORM_X8_TYPELESS => Format::D32Float,
        R16_TYPELESS if depth => Format::D16Unorm,
        D16_UNORM => Format::D16Unorm,
        f => return unsupported(format!("DXGI format {f} for a texture")),
    })
}

/// Whether a view's format reads a texture of the device's format without reinterpreting its bits.
fn view_compatible(view: u32, texture: Format) -> bool {
    use format::*;
    if view == UNKNOWN {
        return true;
    }
    match texture {
        Format::D32Float => matches!(view, D32_FLOAT | R32_FLOAT | R32_TYPELESS | D24_UNORM_S8_UINT | R24_UNORM_X8_TYPELESS | R24G8_TYPELESS),
        Format::D16Unorm => matches!(view, D16_UNORM | R16_UNORM | R16_TYPELESS),
        _ => texture_format(view, 0).is_ok_and(|f| f == texture),
    }
}

/// A vertex attribute's format.
fn vertex_format(dxgi: u32) -> Result<VertexFormat, Error> {
    use format::*;
    Ok(match dxgi {
        R32G32B32A32_FLOAT => VertexFormat::F32(4),
        R32G32B32_FLOAT => VertexFormat::F32(3),
        R32G32_FLOAT => VertexFormat::F32(2),
        R32_FLOAT => VertexFormat::F32(1),
        R32G32B32A32_UINT => VertexFormat::U32(4),
        R32G32B32_UINT => VertexFormat::U32(3),
        R32G32_UINT => VertexFormat::U32(2),
        R32_UINT => VertexFormat::U32(1),
        R32G32B32A32_SINT => VertexFormat::I32(4),
        R32G32B32_SINT => VertexFormat::I32(3),
        R32G32_SINT => VertexFormat::I32(2),
        R32_SINT => VertexFormat::I32(1),
        R8G8B8A8_UNORM => VertexFormat::Unorm8x4,
        B8G8R8A8_UNORM | B8G8R8A8_UNORM_SRGB => VertexFormat::Bgra8Unorm,
        f => return unsupported(format!("DXGI format {f} for a vertex attribute")),
    })
}

// Descriptions, with Direct3D 11's fields and numbers.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufferDesc {
    pub size: u32,
    pub usage: u32,
    pub bind: u32,
    pub cpu_access: u32,
    pub misc: u32,
    pub stride: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Texture2dDesc {
    pub width: u32,
    pub height: u32,
    /// 0: every level down to 1×1.
    pub mip_levels: u32,
    pub array_size: u32,
    pub format: u32,
    pub sample_count: u32,
    pub usage: u32,
    pub bind: u32,
    pub cpu_access: u32,
    pub misc: u32,
}

/// One subresource's initial or updated bytes: rows `row_pitch` apart.
#[derive(Debug, Clone, Copy)]
pub struct Data<'a> {
    pub bytes: &'a [u8],
    pub row_pitch: u32,
}

/// A region of a resource: `D3D11_BOX`, left/top/front inclusive, right/bottom/back exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub left: u32,
    pub top: u32,
    pub front: u32,
    pub right: u32,
    pub bottom: u32,
    pub back: u32,
}

/// A shader resource view: `D3D11_SHADER_RESOURCE_VIEW_DESC`, its union's fields for the dimension named.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SrvDesc {
    pub format: u32,
    /// `D3D11_SRV_DIMENSION`: 1 buffer, 4 a 2D texture, 5 a 2D texture array, 11 an extended buffer.
    pub dimension: u32,
    pub most_detailed_mip: u32,
    pub mip_levels: u32,
    pub first_slice: u32,
    pub slices: u32,
    pub first_element: u32,
    pub elements: u32,
    /// An extended buffer's: 1 raw.
    pub flags: u32,
}

/// A render target or depth-stencil view: one level of one slice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetDesc {
    pub format: u32,
    /// `D3D11_RTV_DIMENSION` (4 a 2D texture, 5 an array) or `D3D11_DSV_DIMENSION` (3 a 2D texture, 4 an array).
    pub dimension: u32,
    pub mip_slice: u32,
    pub first_slice: u32,
}

/// An unordered access view of a buffer: `D3D11_UNORDERED_ACCESS_VIEW_DESC`'s buffer fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UavDesc {
    pub format: u32,
    /// `D3D11_UAV_DIMENSION`: 1 buffer.
    pub dimension: u32,
    pub first_element: u32,
    pub elements: u32,
    /// 1 raw.
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerDesc {
    /// `D3D11_FILTER`.
    pub filter: u32,
    /// `D3D11_TEXTURE_ADDRESS_MODE` for u, v and w: 1 wrap, 2 mirror, 3 clamp, 4 border, 5 mirror once.
    pub address: [u32; 3],
    pub mip_lod_bias: f32,
    pub max_anisotropy: u32,
    /// `D3D11_COMPARISON_FUNC`.
    pub comparison: u32,
    pub border: [f32; 4],
    pub min_lod: f32,
    pub max_lod: f32,
}

impl Default for SamplerDesc {
    /// Direct3D's default sampler: trilinear, clamped, every level.
    fn default() -> Self {
        SamplerDesc { filter: 0x15, address: [3; 3], mip_lod_bias: 0.0, max_anisotropy: 1, comparison: 1, border: [1.0; 4], min_lod: f32::MIN, max_lod: f32::MAX }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetBlend {
    pub enable: bool,
    /// `D3D11_BLEND`s and `D3D11_BLEND_OP`s.
    pub src: u32,
    pub dest: u32,
    pub op: u32,
    pub src_alpha: u32,
    pub dest_alpha: u32,
    pub op_alpha: u32,
    /// Red 1, green 2, blue 4, alpha 8.
    pub write_mask: u8,
}

impl Default for TargetBlend {
    fn default() -> Self {
        TargetBlend { enable: false, src: 2, dest: 1, op: 1, src_alpha: 2, dest_alpha: 1, op_alpha: 1, write_mask: 0xf }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlendDesc {
    pub alpha_to_coverage: bool,
    pub independent: bool,
    pub targets: [TargetBlend; 8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthStencilDesc {
    pub depth_enable: bool,
    /// `D3D11_DEPTH_WRITE_MASK`: 0 none, 1 all.
    pub depth_write: u32,
    pub depth_func: u32,
    pub stencil_enable: bool,
}

impl Default for DepthStencilDesc {
    fn default() -> Self {
        DepthStencilDesc { depth_enable: true, depth_write: 1, depth_func: 2, stencil_enable: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterizerDesc {
    /// `D3D11_FILL_MODE`: 2 wireframe, 3 solid.
    pub fill: u32,
    /// `D3D11_CULL_MODE`: 1 none, 2 front, 3 back.
    pub cull: u32,
    pub front_counter_clockwise: bool,
    pub depth_clip: bool,
    pub scissor_enable: bool,
}

impl Default for RasterizerDesc {
    fn default() -> Self {
        RasterizerDesc { fill: 3, cull: 3, front_counter_clockwise: false, depth_clip: true, scissor_enable: false }
    }
}

/// `D3D11_INPUT_ELEMENT_DESC`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputElement {
    pub semantic: String,
    pub index: u32,
    pub format: u32,
    pub slot: u32,
    /// `D3D11_APPEND_ALIGNED_ELEMENT` (all ones) for just after the slot's last element.
    pub offset: u32,
    pub per_instance: bool,
    pub step_rate: u32,
}

/// `D3D11_VIEWPORT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

/// A shader stage's tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Vertex,
    Pixel,
    Compute,
}

const CONSTANT_SLOTS: usize = 14;
const RESOURCE_SLOTS: usize = 128;
const SAMPLER_SLOTS: usize = 16;
const UAV_SLOTS: usize = 8;
const TARGETS: usize = 8;
const VERTEX_SLOTS: usize = 32;
/// Constant buffers are allocated at Direct3D's largest, so a shader reading past a small one's end reads zeroes,
/// as Direct3D's do, and doesn't fault.
const CONSTANT_BYTES: u64 = 4096 * 16;
const LONGEST: Duration = Duration::from_secs(600);

// Objects.

struct Buffer {
    memory: gpu::Memory,
    desc: BufferDesc,
    /// A dynamic buffer's bytes, for `Map`.
    shadow: Option<Vec<u8>>,
}

struct Texture {
    desc: Texture2dDesc,
    format: Format,
    levels: u32,
    /// The image; `None` for a staging texture, which is `memory`.
    image: Option<gpu::Image>,
    memory: Option<gpu::Memory>,
    /// A dynamic texture's subresources' bytes, for `Map`.
    shadow: Option<Vec<Vec<u8>>>,
}

impl Texture {
    fn size(&self, level: u32) -> (u32, u32) {
        ((self.desc.width >> level).max(1), (self.desc.height >> level).max(1))
    }

    fn sub(&self, index: u32) -> Sub {
        Sub { level: index % self.levels, layer: index / self.levels }
    }

    /// A subresource's tightly packed bytes.
    fn sub_bytes(&self, index: u32) -> u64 {
        let (w, h) = self.size(index % self.levels);
        w as u64 * h as u64 * self.format.bytes() as u64
    }

    /// Where a subresource starts in a staging texture's memory: subresources in index order, tightly packed.
    fn sub_offset(&self, index: u32) -> u64 {
        (0..index).map(|i| self.sub_bytes(i)).sum()
    }

    fn subresources(&self) -> u32 {
        self.levels * self.desc.array_size.max(1)
    }
}

enum Resource {
    Buffer(Buffer),
    Texture(Texture),
}

#[derive(Clone, Copy)]
enum View {
    /// A texture's image, read by samples.
    Image(Id, gpu::Image),
    /// Bytes of a buffer: a raw or structured shader resource, or an unordered access view.
    Buffer(Id, gpu::Range),
    /// One level of one slice of a texture, drawn into.
    Target(Id, gpu::Image, Sub, Format),
}

struct Shader {
    stage: Stage,
    shader: dxbc::Shader,
}

#[derive(Clone)]
struct Layout {
    elements: Vec<InputElement>,
}

#[derive(Clone, Copy)]
struct StageState {
    shader: Option<Id>,
    constants: [Option<Id>; CONSTANT_SLOTS],
    resources: [Option<Id>; RESOURCE_SLOTS],
    samplers: [Option<Id>; SAMPLER_SLOTS],
    uavs: [Option<Id>; UAV_SLOTS],
}

impl Default for StageState {
    fn default() -> Self {
        StageState { shader: None, constants: [None; CONSTANT_SLOTS], resources: [None; RESOURCE_SLOTS], samplers: [None; SAMPLER_SLOTS], uavs: [None; UAV_SLOTS] }
    }
}

/// What makes a graphics pipeline.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineKey {
    vs: Id,
    ps: Option<Id>,
    layout: Option<Id>,
    strides: Vec<(u32, u32)>,
    topology: u32,
    blend: Option<Id>,
    depth: Option<Id>,
    raster: Option<Id>,
    colors: Vec<Format>,
    depth_format: Option<Format>,
}

struct Pipeline {
    device: gpu::Pipeline,
    /// The D3D vertex buffer slot of each of the pipeline's vertex buffers.
    slots: Vec<u32>,
    vs_bindings: Vec<gpu::ir::Binding>,
    ps_bindings: Vec<gpu::ir::Binding>,
}

/// Something recorded: a command, or bytes the submission brings with it.
enum Rec {
    Command(gpu::Command),
    ToMemory { bytes: Vec<u8>, to: gpu::Memory, at: u64 },
    ToImage { bytes: Vec<u8>, to: gpu::Image, sub: Sub, region: Rect },
    /// A region of one image into another, through memory the submission makes.
    ImageToImage { from: gpu::Image, from_sub: Sub, region: Rect, to: gpu::Image, to_sub: Sub, x: u32, y: u32, bytes: u32 },
}

/// The render pass being recorded: its attachments.
#[derive(Clone, PartialEq)]
struct Pass {
    colors: Vec<(gpu::Image, Sub)>,
    depth: Option<(gpu::Image, Sub)>,
}

/// A device and its immediate context.
pub struct Device {
    gpu: Gpu,
    next: Id,
    resources: HashMap<Id, Resource>,
    views: HashMap<Id, View>,
    samplers: HashMap<Id, gpu::Sampler>,
    shaders: HashMap<Id, Shader>,
    layouts: HashMap<Id, Layout>,
    blends: HashMap<Id, BlendDesc>,
    depths: HashMap<Id, DepthStencilDesc>,
    rasters: HashMap<Id, RasterizerDesc>,
    pipelines: HashMap<PipelineKey, Pipeline>,
    compute: HashMap<Id, gpu::Pipeline>,
    // The context's state.
    vs: StageState,
    ps: StageState,
    cs: StageState,
    layout: Option<Id>,
    topology: u32,
    vertex_buffers: [Option<(Id, u32, u32)>; VERTEX_SLOTS],
    index: Option<(Id, u32, u32)>,
    raster: Option<Id>,
    viewports: Vec<Viewport>,
    scissors: Vec<Rect>,
    targets: [Option<Id>; TARGETS],
    depth_target: Option<Id>,
    blend: Option<Id>,
    depth: Option<Id>,
    // What's recorded and not yet submitted.
    recorded: Vec<Rec>,
    pass: Option<Pass>,
    /// Bound where a shader reads a slot nothing is in: zeroes, an image of transparent black, a sampler.
    zeroes: Option<(gpu::Memory, gpu::Image, gpu::Sampler)>,
    fence: Option<gpu::Fence>,
    value: u64,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("d3d11::Device").field("objects", &self.next).finish()
    }
}

fn blend_factor(b: u32) -> Result<Factor, Error> {
    Ok(match b {
        1 => Factor::Zero,
        2 => Factor::One,
        3 => Factor::SrcColor,
        4 => Factor::OneMinusSrcColor,
        5 => Factor::SrcAlpha,
        6 => Factor::OneMinusSrcAlpha,
        7 => Factor::DstAlpha,
        8 => Factor::OneMinusDstAlpha,
        9 => Factor::DstColor,
        10 => Factor::OneMinusDstColor,
        b => return unsupported(format!("blend factor {b}")),
    })
}

fn blend_op(op: u32) -> Result<BlendOp, Error> {
    Ok(match op {
        1 => BlendOp::Add,
        2 => BlendOp::Subtract,
        3 => BlendOp::ReverseSubtract,
        4 => BlendOp::Min,
        5 => BlendOp::Max,
        op => return invalid(format!("blend operation {op}")),
    })
}

fn compare(func: u32) -> Result<Compare, Error> {
    Ok(match func {
        1 => Compare::Never,
        2 => Compare::Less,
        3 => Compare::Equal,
        4 => Compare::LessOrEqual,
        5 => Compare::Greater,
        6 => Compare::NotEqual,
        7 => Compare::GreaterOrEqual,
        8 => Compare::Always,
        f => return invalid(format!("comparison function {f}")),
    })
}

/// The register a signature gives a semantic.
fn register_of(signature: &[Element], semantic: &str, index: u32) -> Option<u32> {
    signature.iter().find(|e| e.semantic.eq_ignore_ascii_case(semantic) && e.index == index).map(|e| e.register)
}

impl Device {
    pub fn new(gpu: Gpu) -> Device {
        Device {
            gpu,
            next: 1,
            resources: HashMap::new(),
            views: HashMap::new(),
            samplers: HashMap::new(),
            shaders: HashMap::new(),
            layouts: HashMap::new(),
            blends: HashMap::new(),
            depths: HashMap::new(),
            rasters: HashMap::new(),
            pipelines: HashMap::new(),
            compute: HashMap::new(),
            vs: StageState::default(),
            ps: StageState::default(),
            cs: StageState::default(),
            layout: None,
            topology: 0,
            vertex_buffers: [None; VERTEX_SLOTS],
            index: None,
            raster: None,
            viewports: Vec::new(),
            scissors: Vec::new(),
            targets: [None; TARGETS],
            depth_target: None,
            blend: None,
            depth: None,
            recorded: Vec::new(),
            pass: None,
            zeroes: None,
            fence: None,
            value: 0,
        }
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    fn id(&mut self) -> Id {
        self.next += 1;
        self.next - 1
    }

    fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut gpu::Device) -> Result<R, gpu::Error> + Send + 'static) -> Result<R, Error> {
        self.gpu.run(f).map_err(device)?.map_err(device)
    }

    // Resources.

    pub fn create_buffer(&mut self, desc: BufferDesc, initial: Option<&[u8]>) -> Result<Id, Error> {
        if desc.size == 0 {
            return invalid("a buffer of 0 bytes");
        }
        if desc.usage == usage::IMMUTABLE && initial.is_none() {
            return invalid("an immutable buffer with no initial data");
        }
        let mut size = (desc.size as u64).next_multiple_of(16);
        if desc.bind & bind::CONSTANT_BUFFER != 0 {
            size = size.max(CONSTANT_BYTES);
        }
        let mut bytes = initial.map(|b| b[..b.len().min(desc.size as usize)].to_vec());
        let memory = self.run(move |d| {
            let m = d.alloc(size)?;
            match bytes.take() {
                Some(b) => d.write(m, 0, &b)?,
                None => d.write(m, 0, &vec![0; size as usize])?,
            }
            Ok(m)
        })?;
        let shadow = (desc.usage == usage::DYNAMIC).then(|| {
            let mut s = vec![0u8; desc.size as usize];
            if let Some(i) = initial {
                let n = i.len().min(s.len());
                s[..n].copy_from_slice(&i[..n]);
            }
            s
        });
        let id = self.id();
        self.resources.insert(id, Resource::Buffer(Buffer { memory, desc, shadow }));
        Ok(id)
    }

    pub fn create_texture2d(&mut self, desc: Texture2dDesc, initial: &[Data<'_>]) -> Result<Id, Error> {
        if desc.width == 0 || desc.height == 0 || desc.width > gpu::MAX_IMAGE || desc.height > gpu::MAX_IMAGE {
            return invalid(format!("a texture of {}×{}", desc.width, desc.height));
        }
        if desc.sample_count > 1 {
            return unsupported("multisampled textures");
        }
        let format = texture_format(desc.format, desc.bind)?;
        let most = gpu::ImageDesc::most_levels(desc.width, desc.height);
        let levels = if desc.mip_levels == 0 { most } else { desc.mip_levels.min(most) };
        let layers = desc.array_size.max(1);
        let mut t = Texture { desc: Texture2dDesc { array_size: layers, ..desc }, format, levels, image: None, memory: None, shadow: None };
        let subs = t.subresources();
        if desc.usage == usage::STAGING {
            let total: u64 = (0..subs).map(|i| t.sub_bytes(i)).sum();
            let mut packed = vec![0u8; total as usize];
            for (i, data) in initial.iter().enumerate().take(subs as usize) {
                let at = t.sub_offset(i as u32) as usize;
                let rows = t.packed(i as u32, data)?;
                packed[at..at + rows.len()].copy_from_slice(&rows);
            }
            t.memory = Some(self.run(move |d| {
                let m = d.alloc(total.max(4))?;
                d.write(m, 0, &packed)?;
                Ok(m)
            })?);
        } else {
            let image_desc = gpu::ImageDesc { format, width: desc.width, height: desc.height, levels, layers };
            let image = self.run(move |d| d.image(image_desc))?;
            t.image = Some(image);
            // Device images start as anything; Direct3D's start as given, or zeroes.
            self.end_pass();
            for i in 0..subs {
                let (w, h) = t.size(i % levels);
                let bytes = match initial.get(i as usize) {
                    Some(data) => t.packed(i, data)?,
                    None => vec![0; t.sub_bytes(i) as usize],
                };
                if desc.bind & bind::DEPTH_STENCIL != 0 && initial.is_empty() {
                    continue;
                }
                self.recorded.push(Rec::ToImage { bytes, to: image, sub: t.sub(i), region: Rect { x: 0, y: 0, width: w, height: h } });
            }
            if desc.usage == usage::DYNAMIC {
                t.shadow = Some((0..subs).map(|i| initial.get(i as usize).map_or_else(|| Ok(vec![0; t.sub_bytes(i) as usize]), |d| t.packed(i, d))).collect::<Result<_, _>>()?);
            }
        }
        let id = self.id();
        self.resources.insert(id, Resource::Texture(t));
        Ok(id)
    }

    /// A buffer's or texture's description, as it was made.
    pub fn buffer_desc(&self, id: Id) -> Option<BufferDesc> {
        match self.resources.get(&id) {
            Some(Resource::Buffer(b)) => Some(b.desc),
            _ => None,
        }
    }

    /// A subresource's layout as `Map` gives it: the bytes from one row to the next, and its size.
    pub fn layout(&self, id: Id, index: u32) -> Option<(u32, u32)> {
        match self.resources.get(&id)? {
            Resource::Buffer(b) => Some((b.desc.size, b.desc.size)),
            Resource::Texture(t) => {
                let (w, _) = t.size(index % t.levels);
                Some((w * t.format.bytes(), t.sub_bytes(index) as u32))
            }
        }
    }

    pub fn texture_desc(&self, id: Id) -> Option<Texture2dDesc> {
        match self.resources.get(&id) {
            Some(Resource::Texture(t)) => Some(Texture2dDesc { mip_levels: t.levels, ..t.desc }),
            _ => None,
        }
    }

    // Views.

    fn texture(&self, id: Id) -> Result<&Texture, Error> {
        match self.resources.get(&id) {
            Some(Resource::Texture(t)) => Ok(t),
            _ => invalid(format!("{id} isn't a texture")),
        }
    }

    fn buffer(&self, id: Id) -> Result<&Buffer, Error> {
        match self.resources.get(&id) {
            Some(Resource::Buffer(b)) => Ok(b),
            _ => invalid(format!("{id} isn't a buffer")),
        }
    }

    /// A buffer view's bytes: its elements of the buffer's stride (or 4-byte words when raw).
    fn buffer_range(&self, id: Id, first: u32, elements: u32) -> Result<gpu::Range, Error> {
        let b = self.buffer(id)?;
        let stride = if b.desc.misc & misc::BUFFER_STRUCTURED != 0 { b.desc.stride.max(1) } else { 4 } as u64;
        let offset = first as u64 * stride;
        let size = if elements == 0 { b.desc.size as u64 - offset.min(b.desc.size as u64) } else { elements as u64 * stride };
        if offset + size > b.desc.size as u64 || size == 0 {
            return invalid(format!("elements {first} to {} past the buffer's {} bytes", first + elements, b.desc.size));
        }
        Ok(gpu::Range { memory: b.memory, offset, size })
    }

    pub fn create_srv(&mut self, resource: Id, desc: Option<SrvDesc>) -> Result<Id, Error> {
        let view = match self.resources.get(&resource) {
            Some(Resource::Texture(t)) => {
                let Some(image) = t.image else { return invalid("a view of a staging texture") };
                if let Some(d) = desc {
                    if !view_compatible(d.format, t.format) {
                        return unsupported(format!("a DXGI format {} view of a {:?} texture", d.format, t.format));
                    }
                    if !matches!(d.dimension, 4 | 5 | 9 | 10) {
                        return unsupported(format!("shader resource views of dimension {}", d.dimension));
                    }
                }
                View::Image(resource, image)
            }
            Some(Resource::Buffer(_)) => {
                let d = desc.unwrap_or_default();
                View::Buffer(resource, self.buffer_range(resource, d.first_element, d.elements)?)
            }
            None => return invalid(format!("{resource} isn't a resource")),
        };
        let id = self.id();
        self.views.insert(id, view);
        Ok(id)
    }

    fn target_view(&mut self, resource: Id, desc: Option<TargetDesc>, depth: bool) -> Result<Id, Error> {
        let t = self.texture(resource)?;
        let Some(image) = t.image else { return invalid("a view of a staging texture") };
        let d = desc.unwrap_or_default();
        if !view_compatible(d.format, t.format) {
            return unsupported(format!("a DXGI format {} view of a {:?} texture", d.format, t.format));
        }
        if depth != t.format.is_depth() {
            return invalid(format!("a {} view of a {:?} texture", if depth { "depth-stencil" } else { "render target" }, t.format));
        }
        let sub = Sub { level: d.mip_slice, layer: d.first_slice };
        if sub.level >= t.levels || sub.layer >= t.desc.array_size {
            return invalid(format!("level {} of slice {} of a texture of {} levels and {} slices", sub.level, sub.layer, t.levels, t.desc.array_size));
        }
        let format = t.format;
        let id = self.id();
        self.views.insert(id, View::Target(resource, image, sub, format));
        Ok(id)
    }

    pub fn create_rtv(&mut self, resource: Id, desc: Option<TargetDesc>) -> Result<Id, Error> {
        self.target_view(resource, desc, false)
    }

    pub fn create_dsv(&mut self, resource: Id, desc: Option<TargetDesc>) -> Result<Id, Error> {
        self.target_view(resource, desc, true)
    }

    pub fn create_uav(&mut self, resource: Id, desc: Option<UavDesc>) -> Result<Id, Error> {
        if self.texture(resource).is_ok() {
            return unsupported("unordered access views of textures");
        }
        let d = desc.unwrap_or_default();
        let range = self.buffer_range(resource, d.first_element, d.elements)?;
        let id = self.id();
        self.views.insert(id, View::Buffer(resource, range));
        Ok(id)
    }

    /// The resource a view is of.
    pub fn view_resource(&self, view: Id) -> Option<Id> {
        self.views.get(&view).map(|v| match *v {
            View::Image(r, _) | View::Buffer(r, _) | View::Target(r, ..) => r,
        })
    }

    // States and shaders.

    pub fn create_sampler(&mut self, desc: SamplerDesc) -> Result<Id, Error> {
        let f = desc.filter;
        let linear = |bit: u32| if f & bit != 0 || f & 0x40 != 0 { Filter::Linear } else { Filter::Nearest };
        let address = |a: u32| match a {
            1 => Ok(Address::Repeat),
            2 => Ok(Address::MirroredRepeat),
            3 | 5 => Ok(Address::ClampToEdge),
            4 => Ok(Address::ClampToBorder),
            a => invalid(format!("address mode {a}")),
        };
        let c = desc.border;
        let border = if c[3] < 0.5 {
            gpu::Border::TransparentBlack
        } else if c[0] + c[1] + c[2] > 1.5 {
            gpu::Border::OpaqueWhite
        } else {
            gpu::Border::OpaqueBlack
        };
        let mut s = gpu::SamplerDesc::new(linear(0x4), linear(0x10), [address(desc.address[0])?, address(desc.address[1])?]);
        s.mipmap = linear(0x1);
        s.lod = [desc.min_lod.max(0.0), desc.max_lod.max(desc.min_lod.max(0.0)).min(1000.0)];
        s.border = border;
        if f & 0x180 == 0x80 {
            s.compare = Some(compare(desc.comparison)?);
        }
        let sampler = self.run(move |d| d.sampler(s))?;
        let id = self.id();
        self.samplers.insert(id, sampler);
        Ok(id)
    }

    pub fn create_blend_state(&mut self, desc: BlendDesc) -> Result<Id, Error> {
        for t in &desc.targets[..if desc.independent { TARGETS } else { 1 }] {
            if t.enable {
                for f in [t.src, t.dest, t.src_alpha, t.dest_alpha] {
                    blend_factor(f)?;
                }
                blend_op(t.op)?;
                blend_op(t.op_alpha)?;
            }
        }
        let id = self.id();
        self.blends.insert(id, desc);
        Ok(id)
    }

    pub fn create_depth_stencil_state(&mut self, desc: DepthStencilDesc) -> Result<Id, Error> {
        compare(desc.depth_func)?;
        let id = self.id();
        self.depths.insert(id, desc);
        Ok(id)
    }

    pub fn create_rasterizer_state(&mut self, desc: RasterizerDesc) -> Result<Id, Error> {
        if !matches!(desc.cull, 1..=3) || !matches!(desc.fill, 2 | 3) {
            return invalid(format!("cull mode {} and fill mode {}", desc.cull, desc.fill));
        }
        let id = self.id();
        self.rasters.insert(id, desc);
        Ok(id)
    }

    /// A vertex, pixel or compute shader from its DXBC; refused if it's another stage than the one asked for.
    pub fn create_shader(&mut self, stage: Stage, bytecode: &[u8]) -> Result<Id, Error> {
        let shader = dxbc::read(bytecode).map_err(|e| Error { hresult: E_INVALIDARG, message: e.message })?;
        let is = match shader.entry.stage {
            gpu::ir::Stage::Vertex => Stage::Vertex,
            gpu::ir::Stage::Fragment => Stage::Pixel,
            gpu::ir::Stage::Compute => Stage::Compute,
        };
        if is != stage {
            return invalid(format!("a {is:?} shader made as a {stage:?} one"));
        }
        let id = self.id();
        if stage == Stage::Compute {
            let entry = shader.entry.clone();
            if entry.bindings.iter().any(|b| !b.kind.is_buffer()) {
                return unsupported("compute shaders reading textures or samplers");
            }
            let p = self.run(move |d| d.pipeline(&entry))?;
            self.compute.insert(id, p);
        }
        self.shaders.insert(id, Shader { stage, shader });
        Ok(id)
    }

    /// A shader's stage.
    pub fn shader_stage(&self, id: Id) -> Option<Stage> {
        self.shaders.get(&id).map(|s| s.stage)
    }

    /// An input layout: its elements, checked against the vertex shader's input signature it's made for.
    pub fn create_input_layout(&mut self, elements: &[InputElement], vs_bytecode: &[u8]) -> Result<Id, Error> {
        let shader = dxbc::read(vs_bytecode).map_err(|e| Error { hresult: E_INVALIDARG, message: e.message })?;
        for el in elements {
            vertex_format(el.format)?;
            if el.slot as usize >= VERTEX_SLOTS {
                return invalid(format!("vertex buffer slot {}", el.slot));
            }
        }
        for input in shader.inputs.iter().filter(|i| i.system == 0) {
            if !elements.iter().any(|e| e.semantic.eq_ignore_ascii_case(&input.semantic) && e.index == input.index) {
                return invalid(format!("the shader's input {}{} isn't in the layout", input.semantic, input.index));
            }
        }
        // Appended offsets made concrete: just after the slot's element before.
        let mut ends = [0u32; VERTEX_SLOTS];
        let mut resolved = Vec::new();
        for el in elements {
            let size = vertex_format(el.format)?.bytes();
            let offset = if el.offset == u32::MAX { ends[el.slot as usize] } else { el.offset };
            ends[el.slot as usize] = offset + size;
            resolved.push(InputElement { offset, ..el.clone() });
        }
        let id = self.id();
        self.layouts.insert(id, Layout { elements: resolved });
        Ok(id)
    }

    /// Lets an object go. A resource's views keep naming it, as Direct3D's keep theirs alive; the caller releases
    /// a resource after its last view.
    pub fn release(&mut self, id: Id) {
        let mut memory = Vec::new();
        let mut image = None;
        if let Some(r) = self.resources.remove(&id) {
            match r {
                Resource::Buffer(b) => memory.push(b.memory),
                Resource::Texture(t) => {
                    memory.extend(t.memory);
                    image = t.image;
                }
            }
        }
        self.views.remove(&id);
        self.samplers.remove(&id);
        self.shaders.remove(&id);
        self.layouts.remove(&id);
        self.blends.remove(&id);
        self.depths.remove(&id);
        self.rasters.remove(&id);
        self.pipelines.retain(|k, _| k.vs != id && k.ps != Some(id) && k.layout != Some(id) && k.blend != Some(id) && k.depth != Some(id) && k.raster != Some(id));
        self.compute.remove(&id);
        if !memory.is_empty() || image.is_some() {
            // Freed after what's recorded has run.
            let _ = self.flush();
            let _ = self.gpu.run(move |d| {
                for m in memory {
                    d.free(m);
                }
                if let Some(i) = image {
                    d.free_image(i);
                }
            });
        }
    }

    // The context's state.

    fn stage(&mut self, stage: Stage) -> &mut StageState {
        match stage {
            Stage::Vertex => &mut self.vs,
            Stage::Pixel => &mut self.ps,
            Stage::Compute => &mut self.cs,
        }
    }

    pub fn set_shader(&mut self, stage: Stage, shader: Option<Id>) {
        self.stage(stage).shader = shader;
    }

    pub fn set_constant_buffers(&mut self, stage: Stage, first: u32, buffers: &[Option<Id>]) {
        let s = self.stage(stage);
        for (i, b) in buffers.iter().enumerate() {
            if let Some(slot) = s.constants.get_mut(first as usize + i) {
                *slot = *b;
            }
        }
    }

    pub fn set_shader_resources(&mut self, stage: Stage, first: u32, views: &[Option<Id>]) {
        let s = self.stage(stage);
        for (i, v) in views.iter().enumerate() {
            if let Some(slot) = s.resources.get_mut(first as usize + i) {
                *slot = *v;
            }
        }
    }

    pub fn set_samplers(&mut self, stage: Stage, first: u32, samplers: &[Option<Id>]) {
        let s = self.stage(stage);
        for (i, v) in samplers.iter().enumerate() {
            if let Some(slot) = s.samplers.get_mut(first as usize + i) {
                *slot = *v;
            }
        }
    }

    pub fn set_uavs(&mut self, first: u32, views: &[Option<Id>]) {
        for (i, v) in views.iter().enumerate() {
            if let Some(slot) = self.cs.uavs.get_mut(first as usize + i) {
                *slot = *v;
            }
        }
    }

    pub fn set_input_layout(&mut self, layout: Option<Id>) {
        self.layout = layout;
    }

    /// `D3D11_PRIMITIVE_TOPOLOGY`.
    pub fn set_topology(&mut self, topology: u32) {
        self.topology = topology;
    }

    /// Buffers, strides and offsets from slot `first` on.
    pub fn set_vertex_buffers(&mut self, first: u32, buffers: &[(Option<Id>, u32, u32)]) {
        for (i, (b, stride, offset)) in buffers.iter().enumerate() {
            if let Some(slot) = self.vertex_buffers.get_mut(first as usize + i) {
                *slot = b.map(|b| (b, *stride, *offset));
            }
        }
    }

    /// The index buffer, its format (`R16_UINT` or `R32_UINT`) and offset.
    pub fn set_index_buffer(&mut self, buffer: Option<Id>, format: u32, offset: u32) {
        self.index = buffer.map(|b| (b, format, offset));
    }

    pub fn set_rasterizer_state(&mut self, state: Option<Id>) {
        self.raster = state;
    }

    pub fn set_viewports(&mut self, viewports: &[Viewport]) {
        self.viewports = viewports.to_vec();
    }

    /// Scissor rectangles as left, top, right and bottom.
    pub fn set_scissor_rects(&mut self, rects: &[[i32; 4]]) {
        self.scissors = rects
            .iter()
            .map(|r| {
                let (l, t) = (r[0].max(0) as u32, r[1].max(0) as u32);
                Rect { x: l, y: t, width: (r[2].max(0) as u32).saturating_sub(l), height: (r[3].max(0) as u32).saturating_sub(t) }
            })
            .collect();
    }

    pub fn set_render_targets(&mut self, targets: &[Option<Id>], depth: Option<Id>) {
        self.targets = [None; TARGETS];
        for (slot, t) in self.targets.iter_mut().zip(targets) {
            *slot = *t;
        }
        self.depth_target = depth;
    }

    pub fn set_blend_state(&mut self, state: Option<Id>) {
        self.blend = state;
    }

    pub fn set_depth_stencil_state(&mut self, state: Option<Id>) {
        self.depth = state;
    }

    /// Unbinds everything, as `ClearState` does.
    pub fn clear_state(&mut self) {
        self.vs = StageState::default();
        self.ps = StageState::default();
        self.cs = StageState::default();
        self.layout = None;
        self.topology = 0;
        self.vertex_buffers = [None; VERTEX_SLOTS];
        self.index = None;
        self.raster = None;
        self.viewports.clear();
        self.scissors.clear();
        self.targets = [None; TARGETS];
        self.depth_target = None;
        self.blend = None;
        self.depth = None;
    }

    // Recording.

    fn end_pass(&mut self) {
        if self.pass.take().is_some() {
            self.recorded.push(Rec::Command(gpu::Command::EndRender));
        }
    }

    fn target(&self, view: Id) -> Result<(gpu::Image, Sub, Format), Error> {
        match self.views.get(&view) {
            Some(View::Target(_, image, sub, format)) => Ok((*image, *sub, *format)),
            _ => invalid(format!("{view} isn't a render target or depth-stencil view")),
        }
    }

    pub fn clear_render_target(&mut self, view: Id, color: [f32; 4]) -> Result<(), Error> {
        let (image, sub, _) = self.target(view)?;
        self.end_pass();
        self.recorded.push(Rec::Command(gpu::Command::BeginRender {
            colors: vec![gpu::ColorAttachment { image, sub, load: Load::Clear(color) }],
            depth: None,
        }));
        self.recorded.push(Rec::Command(gpu::Command::EndRender));
        Ok(())
    }

    /// Clears depth (flag 1); stencil (flag 2) isn't kept.
    pub fn clear_depth_stencil(&mut self, view: Id, flags: u32, depth: f32) -> Result<(), Error> {
        let (image, sub, _) = self.target(view)?;
        if flags & 1 == 0 {
            return Ok(());
        }
        // A pass needs a colour attachment or a depth one; this has only depth.
        self.end_pass();
        self.recorded.push(Rec::Command(gpu::Command::BeginRender { colors: Vec::new(), depth: Some(gpu::DepthAttachment { image, sub, load: Load::Clear(depth) }) }));
        self.recorded.push(Rec::Command(gpu::Command::EndRender));
        Ok(())
    }

    fn zeroes(&mut self) -> Result<(gpu::Memory, gpu::Image, gpu::Sampler), Error> {
        if let Some(z) = self.zeroes {
            return Ok(z);
        }
        let z = self.run(|d| {
            let m = d.alloc(CONSTANT_BYTES)?;
            d.write(m, 0, &vec![0; CONSTANT_BYTES as usize])?;
            let i = d.image(gpu::ImageDesc::new(Format::Rgba8Unorm, 1, 1))?;
            let s = d.sampler(gpu::SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToEdge; 2]))?;
            Ok((m, i, s))
        })?;
        // First, outside any pass: nothing recorded can have used it yet.
        self.recorded.insert(0, Rec::ToImage { bytes: vec![0; 4], to: z.1, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 1, height: 1 } });
        self.zeroes = Some(z);
        Ok(z)
    }

    /// What a stage has at one of a shader's bindings: by its set (the kind of slot) and binding (the slot).
    fn bound(&mut self, stage: Stage, b: gpu::ir::Binding) -> Result<Bind, Error> {
        let s = *self.stage(stage);
        let (zero_memory, zero_image, zero_sampler) = self.zeroes()?;
        let slot = b.binding as usize;
        Ok(match b.set {
            dxbc::SET_CONSTANTS => match s.constants.get(slot).copied().flatten() {
                Some(id) => {
                    let buf = self.buffer(id)?;
                    Bind::Buffer(gpu::Range { memory: buf.memory, offset: 0, size: (buf.desc.size as u64).next_multiple_of(16).max(CONSTANT_BYTES) })
                }
                None => Bind::Buffer(gpu::Range { memory: zero_memory, offset: 0, size: CONSTANT_BYTES }),
            },
            dxbc::SET_RESOURCES => match (b.kind, s.resources.get(slot).copied().flatten().and_then(|v| self.views.get(&v).copied())) {
                (BindingKind::Image, Some(View::Image(_, image))) => Bind::Image(image),
                (BindingKind::Image, _) => Bind::Image(zero_image),
                (_, Some(View::Buffer(_, range))) => Bind::Buffer(range),
                (_, _) => Bind::Buffer(gpu::Range { memory: zero_memory, offset: 0, size: CONSTANT_BYTES }),
            },
            dxbc::SET_SAMPLERS => Bind::Sampler(s.samplers.get(slot).copied().flatten().and_then(|id| self.samplers.get(&id).copied()).unwrap_or(zero_sampler)),
            dxbc::SET_UAVS => match s.uavs.get(slot).copied().flatten().and_then(|v| self.views.get(&v).copied()) {
                Some(View::Buffer(_, range)) => Bind::Buffer(range),
                _ => Bind::Buffer(gpu::Range { memory: zero_memory, offset: 0, size: CONSTANT_BYTES }),
            },
            set => return invalid(format!("a binding in set {set}")),
        })
    }

    /// The pipeline the current state draws with, made if it's new.
    fn pipeline(&mut self, colors: &[Format], depth_format: Option<Format>) -> Result<PipelineKey, Error> {
        let Some(vs) = self.vs.shader else { return invalid("a draw with no vertex shader") };
        let layout = self.layout;
        let mut strides = Vec::new();
        if let Some(l) = layout.and_then(|l| self.layouts.get(&l)) {
            let mut slots: Vec<(u32, bool)> = l.elements.iter().map(|e| (e.slot, e.per_instance)).collect();
            slots.sort_unstable();
            slots.dedup_by_key(|s| s.0);
            for (slot, _) in slots {
                let stride = self.vertex_buffers[slot as usize].map_or(0, |(_, s, _)| s);
                strides.push((slot, stride));
            }
        }
        let key = PipelineKey {
            vs,
            ps: self.ps.shader,
            layout,
            strides,
            topology: self.topology,
            blend: self.blend,
            depth: self.depth,
            raster: self.raster,
            colors: colors.to_vec(),
            depth_format,
        };
        if self.pipelines.contains_key(&key) {
            return Ok(key);
        }
        let vs_shader = &self.shaders.get(&vs).ok_or_else(|| Error { hresult: E_INVALIDARG, message: "the vertex shader is gone".into() })?.shader;
        let vertex = vs_shader.entry.clone();
        // A pixel shader's bindings in sets of their own, and its inputs at the registers of the vertex shader's
        // outputs of their semantics; with no pixel shader, one that writes nothing.
        let fragment = match key.ps.and_then(|p| self.shaders.get(&p)) {
            Some(ps) => {
                let mut f = ps.shader.entry.clone();
                for b in &mut f.bindings {
                    b.set += 4;
                }
                for io in &mut f.inputs {
                    let Slot::Location(r) = io.slot else { continue };
                    let Some(el) = ps.shader.inputs.iter().find(|e| e.register == r && e.system == 0) else { continue };
                    let Some(to) = register_of(&vs_shader.outputs, &el.semantic, el.index) else {
                        return invalid(format!("the pixel shader reads {}{}, which the vertex shader doesn't write", el.semantic, el.index));
                    };
                    io.slot = Slot::Location(to);
                }
                f
            }
            None => Entry::shader("nothing", gpu::ir::Stage::Fragment),
        };
        let (buffers, attributes, slots) = match layout.and_then(|l| self.layouts.get(&l)) {
            Some(l) => {
                let slots: Vec<u32> = key.strides.iter().map(|(s, _)| *s).collect();
                let buffers = key
                    .strides
                    .iter()
                    .map(|(slot, stride)| gpu::VertexBuffer { stride: *stride, per_instance: l.elements.iter().any(|e| e.slot == *slot && e.per_instance) })
                    .collect::<Vec<_>>();
                let mut attributes = Vec::new();
                for el in &l.elements {
                    let Some(location) = register_of(&vs_shader.inputs, &el.semantic, el.index) else { continue };
                    let mut format = vertex_format(el.format)?;
                    // The shader's type wins: a float attribute read as an integer is that integer's bits, which the
                    // device can't do, so it's refused.
                    let want = vertex.inputs.iter().find(|i| i.slot == Slot::Location(location)).and_then(|i| i.ty.scalar());
                    if let (Some(s), VertexFormat::F32(n)) = (want, format)
                        && s != gpu::ir::Scalar::F32 {
                            format = if s == gpu::ir::Scalar::U32 { VertexFormat::U32(n) } else { VertexFormat::I32(n) };
                        }
                    let buffer = slots.iter().position(|s| *s == el.slot).unwrap_or(0) as u32;
                    attributes.push(gpu::Attribute { location, buffer, offset: el.offset, format });
                }
                (buffers, attributes, slots)
            }
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let topology = match key.topology {
            4 => Topology::TriangleList,
            5 => Topology::TriangleStrip,
            0 => return invalid("a draw with no primitive topology"),
            t => return unsupported(format!("primitive topology {t} (only triangle lists and strips)")),
        };
        let raster = key.raster.and_then(|r| self.rasters.get(&r)).copied().unwrap_or_default();
        let cull = match raster.cull {
            2 => Cull::Front,
            3 => Cull::Back,
            _ => Cull::None,
        };
        // With the viewport flipped, Direct3D's windings on the screen are the device's in its framebuffer.
        let front = if raster.front_counter_clockwise { FrontFace::CounterClockwise } else { FrontFace::Clockwise };
        let ds = key.depth.and_then(|d| self.depths.get(&d)).copied().unwrap_or_default();
        let depth = match (depth_format, ds.depth_enable) {
            (Some(_), true) => Some(gpu::DepthState { compare: compare(ds.depth_func)?, write: ds.depth_write != 0 }),
            _ => None,
        };
        let blend = key.blend.and_then(|b| self.blends.get(&b)).copied().unwrap_or_default();
        let mut color_targets = Vec::new();
        for (i, format) in colors.iter().enumerate() {
            let t = blend.targets[if blend.independent { i } else { 0 }];
            let b = if t.enable {
                Some(Blend { color: (blend_factor(t.src)?, blend_factor(t.dest)?, blend_op(t.op)?), alpha: (blend_factor(t.src_alpha)?, blend_factor(t.dest_alpha)?, blend_op(t.op_alpha)?) })
            } else {
                None
            };
            color_targets.push(gpu::ColorTarget { format: *format, blend: b, write: [0, 1, 2, 3].map(|c| t.write_mask & (1 << c) != 0) });
        }
        let vs_bindings = vertex.bindings.clone();
        let ps_bindings = fragment.bindings.clone();
        let device = self.run(move |d| {
            d.graphics(&gpu::GraphicsDesc {
                vertex: &vertex,
                fragment: &fragment,
                buffers: &buffers,
                attributes: &attributes,
                topology,
                cull,
                front,
                depth,
                depth_format,
                colors: &color_targets,
            })
        })?;
        self.pipelines.insert(key.clone(), Pipeline { device, slots, vs_bindings, ps_bindings });
        Ok(key)
    }

    /// A draw: `count` vertices (or indices) from `first`, instances from `first_instance`.
    fn draw_any(&mut self, indexed: bool, count: u32, first: u32, vertex_offset: i32, instances: u32, first_instance: u32) -> Result<(), Error> {
        if count == 0 || instances == 0 {
            return Ok(());
        }
        let used = self.targets.iter().rposition(|t| t.is_some()).map_or(0, |n| n + 1);
        let mut colors = Vec::new();
        let mut attachments = Vec::new();
        for t in &self.targets[..used] {
            let Some(view) = t else { return unsupported("render targets with gaps between them") };
            let (image, sub, format) = self.target(*view)?;
            colors.push(format);
            attachments.push((image, sub));
        }
        let depth_target = match self.depth_target {
            Some(v) => Some(self.target(v)?),
            None => None,
        };
        if attachments.is_empty() && depth_target.is_none() {
            return Ok(());
        }
        let key = self.pipeline(&colors, depth_target.map(|d| d.2))?;
        let pass = Pass { colors: attachments, depth: depth_target.map(|d| (d.0, d.1)) };
        if self.pass.as_ref() != Some(&pass) {
            self.end_pass();
            self.recorded.push(Rec::Command(gpu::Command::BeginRender {
                colors: pass.colors.iter().map(|(image, sub)| gpu::ColorAttachment { image: *image, sub: *sub, load: Load::Load }).collect(),
                depth: pass.depth.map(|(image, sub)| gpu::DepthAttachment { image, sub, load: Load::Load }),
            }));
            self.pass = Some(pass.clone());
        }
        // The attachments' size, at their level.
        let (w, h) = match pass.colors.first().copied().or(pass.depth) {
            Some((image, sub)) => self.image_size(image, sub),
            None => (1, 1),
        };
        let v = self.viewports.first().copied().unwrap_or(Viewport { x: 0.0, y: 0.0, width: w as f32, height: h as f32, min_depth: 0.0, max_depth: 1.0 });
        if v.width <= 0.0 || v.height <= 0.0 {
            return Ok(());
        }
        let viewport = gpu::Viewport { x: v.x, y: v.y + v.height, width: v.width, height: -v.height, min_depth: v.min_depth.clamp(0.0, 1.0), max_depth: v.max_depth.clamp(0.0, 1.0) };
        self.recorded.push(Rec::Command(gpu::Command::SetViewport(viewport)));
        let raster = self.raster.and_then(|r| self.rasters.get(&r)).copied().unwrap_or_default();
        let scissor = match (raster.scissor_enable, self.scissors.first()) {
            (true, Some(r)) => *r,
            (true, None) => Rect { x: 0, y: 0, width: 0, height: 0 },
            (false, _) => Rect { x: 0, y: 0, width: w, height: h },
        };
        self.recorded.push(Rec::Command(gpu::Command::SetScissor(scissor)));
        let p = &self.pipelines[&key];
        let (device, slots, vs_b, ps_b) = (p.device, p.slots.clone(), p.vs_bindings.clone(), p.ps_bindings.clone());
        let mut bindings = Vec::new();
        for b in vs_b {
            bindings.push((b.set, b.binding, self.bound(Stage::Vertex, b)?));
        }
        for b in ps_b {
            let mut original = b;
            original.set -= 4;
            bindings.push((b.set, b.binding, self.bound(Stage::Pixel, original)?));
        }
        let (zero_memory, _, _) = self.zeroes()?;
        let mut vertex_buffers = Vec::new();
        for slot in slots {
            vertex_buffers.push(match self.vertex_buffers[slot as usize] {
                Some((id, _, offset)) => {
                    let b = self.buffer(id)?;
                    let offset = (offset as u64).min(b.desc.size as u64);
                    gpu::Range { memory: b.memory, offset, size: (b.desc.size as u64 - offset).max(1) }
                }
                None => gpu::Range { memory: zero_memory, offset: 0, size: CONSTANT_BYTES },
            });
        }
        let index = if indexed {
            let Some((id, format, offset)) = self.index else { return invalid("an indexed draw with no index buffer") };
            let ty = match format {
                format::R16_UINT => IndexType::U16,
                format::R32_UINT => IndexType::U32,
                f => return invalid(format!("index format {f}")),
            };
            let b = self.buffer(id)?;
            Some((gpu::Range { memory: b.memory, offset: offset as u64, size: b.desc.size as u64 - (offset as u64).min(b.desc.size as u64) }, ty))
        } else {
            None
        };
        self.recorded.push(Rec::Command(gpu::Command::Draw(gpu::Draw {
            pipeline: device,
            bindings,
            vertex_buffers,
            index,
            first,
            count,
            vertex_offset,
            first_instance,
            instances,
        })));
        Ok(())
    }

    fn image_size(&self, image: gpu::Image, sub: Sub) -> (u32, u32) {
        for r in self.resources.values() {
            if let Resource::Texture(t) = r
                && t.image == Some(image) {
                    return t.size(sub.level);
                }
        }
        (1, 1)
    }

    pub fn draw(&mut self, count: u32, first: u32) -> Result<(), Error> {
        self.draw_any(false, count, first, 0, 1, 0)
    }

    pub fn draw_indexed(&mut self, count: u32, first: u32, base_vertex: i32) -> Result<(), Error> {
        self.draw_any(true, count, first, base_vertex, 1, 0)
    }

    pub fn draw_instanced(&mut self, count: u32, instances: u32, first: u32, first_instance: u32) -> Result<(), Error> {
        self.draw_any(false, count, first, 0, instances, first_instance)
    }

    pub fn draw_indexed_instanced(&mut self, count: u32, instances: u32, first: u32, base_vertex: i32, first_instance: u32) -> Result<(), Error> {
        self.draw_any(true, count, first, base_vertex, instances, first_instance)
    }

    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) -> Result<(), Error> {
        let Some(cs) = self.cs.shader else { return invalid("a dispatch with no compute shader") };
        let Some(&pipeline) = self.compute.get(&cs) else { return invalid(format!("{cs} isn't a compute shader")) };
        if x == 0 || y == 0 || z == 0 {
            return Ok(());
        }
        let entry_bindings = self.shaders[&cs].shader.entry.bindings.clone();
        let mut bindings = Vec::new();
        for b in entry_bindings {
            match self.bound(Stage::Compute, b)? {
                Bind::Buffer(r) => bindings.push(r),
                _ => return unsupported("compute shaders reading textures"),
            }
        }
        self.end_pass();
        self.recorded.push(Rec::Command(gpu::Command::Dispatch { pipeline, bindings, groups: [x, y, z] }));
        Ok(())
    }

    // Copies, updates and maps.

    /// A texture's subresource's region, made sure of.
    fn region(&self, t: &Texture, index: u32, r: Option<Region>) -> Result<Rect, Error> {
        if index >= t.subresources() {
            return invalid(format!("subresource {index} of {}", t.subresources()));
        }
        let (w, h) = t.size(index % t.levels);
        let r = r.unwrap_or(Region { left: 0, top: 0, front: 0, right: w, bottom: h, back: 1 });
        if r.left >= r.right || r.top >= r.bottom || r.right > w || r.bottom > h {
            return invalid(format!("{r:?} of a {w}×{h} subresource"));
        }
        Ok(Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top })
    }

    /// `UpdateSubresource`: bytes into a buffer's range, or a texture's subresource's region, ordered among what's
    /// recorded.
    pub fn update_subresource(&mut self, resource: Id, index: u32, region: Option<Region>, data: Data<'_>) -> Result<(), Error> {
        self.end_pass();
        match self.resources.get_mut(&resource) {
            Some(Resource::Buffer(b)) => {
                let (from, to) = region.map_or((0, b.desc.size), |r| (r.left, r.right.min(b.desc.size)));
                let n = (to.saturating_sub(from) as usize).min(data.bytes.len());
                let bytes = data.bytes[..n].to_vec();
                if let Some(s) = &mut b.shadow {
                    s[from as usize..from as usize + n].copy_from_slice(&bytes);
                }
                self.recorded.push(Rec::ToMemory { bytes, to: b.memory, at: from as u64 });
                Ok(())
            }
            Some(Resource::Texture(_)) => {
                let t = self.texture(resource)?;
                let rect = self.region(t, index, region)?;
                let bpp = t.format.bytes() as usize;
                let row = rect.width as usize * bpp;
                let pitch = if data.row_pitch == 0 { row } else { data.row_pitch as usize };
                let mut bytes = Vec::with_capacity(row * rect.height as usize);
                for y in 0..rect.height as usize {
                    let at = y * pitch;
                    let Some(src) = data.bytes.get(at..at + row) else { return invalid("the data ends inside the region") };
                    bytes.extend_from_slice(src);
                }
                match (t.image, t.memory) {
                    (Some(image), _) => self.recorded.push(Rec::ToImage { bytes, to: image, sub: t.sub(index), region: rect }),
                    (None, Some(memory)) => {
                        let (w, _) = t.size(index % t.levels);
                        let base = t.sub_offset(index);
                        for (y, chunk) in bytes.chunks(row).enumerate() {
                            let at = base + ((rect.y as u64 + y as u64) * w as u64 + rect.x as u64) * bpp as u64;
                            self.recorded.push(Rec::ToMemory { bytes: chunk.to_vec(), to: memory, at });
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            None => invalid(format!("{resource} isn't a resource")),
        }
    }

    /// `CopyResource`: all of one into another of the same shape.
    pub fn copy_resource(&mut self, to: Id, from: Id) -> Result<(), Error> {
        match (self.resources.get(&to), self.resources.get(&from)) {
            (Some(Resource::Buffer(a)), Some(Resource::Buffer(b))) => {
                let size = a.desc.size.min(b.desc.size) as u64;
                let (to_m, from_m) = (a.memory, b.memory);
                self.end_pass();
                self.recorded.push(Rec::Command(gpu::Command::Copy { from: gpu::Range { memory: from_m, offset: 0, size }, to: to_m, at: 0 }));
                Ok(())
            }
            (Some(Resource::Texture(a)), Some(Resource::Texture(_))) => {
                for i in 0..a.subresources() {
                    self.copy_subresource_region(to, i, 0, 0, from, i, None)?;
                }
                Ok(())
            }
            _ => invalid("a copy between resources of different kinds"),
        }
    }

    /// `CopySubresourceRegion`: a region of one subresource to a place in another (a buffer's bytes, or a texture's
    /// texels).
    #[allow(clippy::too_many_arguments)]
    pub fn copy_subresource_region(&mut self, to: Id, to_index: u32, x: u32, y: u32, from: Id, from_index: u32, region: Option<Region>) -> Result<(), Error> {
        self.end_pass();
        if let (Ok(a), Ok(b)) = (self.buffer(to), self.buffer(from)) {
            let (start, end) = region.map_or((0, b.desc.size), |r| (r.left, r.right.min(b.desc.size)));
            if start >= end || x + (end - start) > a.desc.size {
                return invalid("a buffer copy past an end");
            }
            let c = gpu::Command::Copy { from: gpu::Range { memory: b.memory, offset: start as u64, size: (end - start) as u64 }, to: a.memory, at: x as u64 };
            self.recorded.push(Rec::Command(c));
            return Ok(());
        }
        let (dst, src) = (self.texture(to)?, self.texture(from)?);
        if dst.format != src.format {
            return invalid(format!("a copy from a {:?} texture to a {:?} one", src.format, dst.format));
        }
        let rect = self.region(src, from_index, region)?;
        let (dw, dh) = dst.size(to_index % dst.levels.max(1));
        if to_index >= dst.subresources() || x + rect.width > dw || y + rect.height > dh {
            return invalid("a texture copy past the destination's edge");
        }
        let bpp = src.format.bytes();
        let rec = match (src.image, src.memory, dst.image, dst.memory) {
            (Some(from), _, Some(to_image), _) => {
                Rec::ImageToImage { from, from_sub: src.sub(from_index), region: rect, to: to_image, to_sub: dst.sub(to_index), x, y, bytes: bpp }
            }
            (Some(from), _, None, Some(memory)) => {
                // Into a staging texture: rows of its subresource, at the region's place.
                let at = dst.sub_offset(to_index) + (y as u64 * dw as u64 + x as u64) * bpp as u64;
                Rec::Command(gpu::Command::CopyImageToBuffer {
                    from,
                    sub: src.sub(from_index),
                    region: rect,
                    to: gpu::Range { memory, offset: at, size: dst.sub_bytes(to_index) - (at - dst.sub_offset(to_index)) },
                    row_pixels: dw,
                })
            }
            (None, Some(memory), Some(to_image), _) => {
                let (sw, _) = src.size(from_index % src.levels);
                let at = src.sub_offset(from_index) + (rect.y as u64 * sw as u64 + rect.x as u64) * bpp as u64;
                Rec::Command(gpu::Command::CopyBufferToImage {
                    from: gpu::Range { memory, offset: at, size: src.sub_bytes(from_index) - (at - src.sub_offset(from_index)) },
                    row_pixels: sw,
                    to: to_image,
                    sub: dst.sub(to_index),
                    region: Rect { x, y, width: rect.width, height: rect.height },
                })
            }
            _ => return unsupported("copies between two staging textures"),
        };
        self.recorded.push(rec);
        Ok(())
    }

    /// `GenerateMips` on a view's texture.
    pub fn generate_mips(&mut self, view: Id) -> Result<(), Error> {
        let Some(View::Image(_, image)) = self.views.get(&view).copied() else { return invalid(format!("{view} isn't a texture's shader resource view")) };
        self.end_pass();
        self.recorded.push(Rec::Command(gpu::Command::GenerateMips(image)));
        Ok(())
    }

    /// `Map`: a subresource's bytes as they are (tightly packed: a texture's rows are its width times its texel's
    /// bytes apart). A staging resource's are read after everything recorded has run; a dynamic one's are its copy
    /// on the host.
    pub fn map(&mut self, resource: Id, index: u32, kind: u32) -> Result<Vec<u8>, Error> {
        match self.resources.get(&resource) {
            Some(Resource::Buffer(b)) => {
                if let Some(s) = &b.shadow {
                    if kind == map::READ {
                        return invalid("a read map of a dynamic buffer");
                    }
                    return Ok(s.clone());
                }
                if b.desc.usage != usage::STAGING {
                    return invalid("a map of a buffer that's neither dynamic nor staging");
                }
                let (memory, size) = (b.memory, b.desc.size as usize);
                self.flush()?;
                self.run(move |d| {
                    let mut out = vec![0; size];
                    d.read(memory, 0, &mut out)?;
                    Ok(out)
                })
            }
            Some(Resource::Texture(t)) => {
                if index >= t.subresources() {
                    return invalid(format!("subresource {index}"));
                }
                if let Some(s) = &t.shadow {
                    return Ok(s[index as usize].clone());
                }
                let Some(memory) = t.memory else { return invalid("a map of a texture that's neither dynamic nor staging") };
                let (at, size) = (t.sub_offset(index), t.sub_bytes(index) as usize);
                self.flush()?;
                self.run(move |d| {
                    let mut out = vec![0; size];
                    d.read(memory, at, &mut out)?;
                    Ok(out)
                })
            }
            None => invalid(format!("{resource} isn't a resource")),
        }
    }

    /// `Unmap`, with the subresource's bytes as the program left them (`None` for a map that only read).
    pub fn unmap(&mut self, resource: Id, index: u32, bytes: Option<&[u8]>) -> Result<(), Error> {
        let Some(bytes) = bytes else { return Ok(()) };
        self.end_pass();
        match self.resources.get_mut(&resource) {
            Some(Resource::Buffer(b)) => {
                let n = bytes.len().min(b.desc.size as usize);
                if let Some(s) = &mut b.shadow {
                    s[..n].copy_from_slice(&bytes[..n]);
                    self.recorded.push(Rec::ToMemory { bytes: bytes[..n].to_vec(), to: b.memory, at: 0 });
                } else {
                    let memory = b.memory;
                    let data = bytes[..n].to_vec();
                    self.flush()?;
                    self.run(move |d| d.write(memory, 0, &data))?;
                }
                Ok(())
            }
            Some(Resource::Texture(t)) => {
                let n = (t.sub_bytes(index) as usize).min(bytes.len());
                let data = bytes[..n].to_vec();
                let (w, h) = t.size(index % t.levels);
                let sub = t.sub(index);
                if let Some(s) = &mut t.shadow {
                    s[index as usize][..n].copy_from_slice(&data);
                    let Some(image) = t.image else { return Ok(()) };
                    self.recorded.push(Rec::ToImage { bytes: data, to: image, sub, region: Rect { x: 0, y: 0, width: w, height: h } });
                } else if let Some(memory) = t.memory {
                    let at = t.sub_offset(index);
                    self.flush()?;
                    self.run(move |d| d.write(memory, at, &data))?;
                }
                Ok(())
            }
            None => invalid(format!("{resource} isn't a resource")),
        }
    }

    /// A texture's subresource read back, tightly packed: what a swap chain's present shows.
    pub fn read_texture(&mut self, texture: Id, index: u32) -> Result<(u32, u32, Vec<u8>), Error> {
        let t = self.texture(texture)?;
        let rect = self.region(t, index, None)?;
        let Some(image) = t.image else { return Ok((rect.width, rect.height, self.map(texture, index, map::READ)?)) };
        let (sub, size) = (t.sub(index), t.sub_bytes(index));
        self.flush()?;
        let fence = self.fence()?;
        self.value += 1;
        let value = self.value;
        let bytes = self.run(move |d| {
            let m = d.alloc(size.max(4))?;
            let c = [gpu::Command::CopyImageToBuffer { from: image, sub, region: rect, to: gpu::Range { memory: m, offset: 0, size }, row_pixels: 0 }];
            d.submit(gpu::Submit { commands: &c, wait: &[], signal: &[(fence, value)] })?;
            d.wait(fence, value, LONGEST)?;
            let mut out = vec![0; size as usize];
            d.read(m, 0, &mut out)?;
            d.free(m);
            Ok(out)
        })?;
        Ok((rect.width, rect.height, bytes))
    }

    fn fence(&mut self) -> Result<gpu::Fence, Error> {
        if let Some(f) = self.fence {
            return Ok(f);
        }
        let f = self.run(|d| d.fence())?;
        self.fence = Some(f);
        Ok(f)
    }

    /// Submits what's recorded and waits for it: `Flush`, and before anything reads results.
    pub fn flush(&mut self) -> Result<(), Error> {
        self.end_pass();
        if self.recorded.is_empty() {
            return Ok(());
        }
        let recorded = std::mem::take(&mut self.recorded);
        let fence = self.fence()?;
        self.value += 1;
        let value = self.value;
        self.run(move |d| {
            let mut temps = Vec::new();
            let mut commands = Vec::new();
            let temp = |d: &mut gpu::Device, temps: &mut Vec<gpu::Memory>, bytes: &[u8], size: u64| -> Result<gpu::Memory, gpu::Error> {
                let m = d.alloc(size.max(4))?;
                if !bytes.is_empty() {
                    d.write(m, 0, bytes)?;
                }
                temps.push(m);
                Ok(m)
            };
            for r in recorded {
                match r {
                    Rec::Command(c) => commands.push(c),
                    Rec::ToMemory { bytes, to, at } => {
                        if bytes.is_empty() {
                            continue;
                        }
                        let m = temp(d, &mut temps, &bytes, bytes.len() as u64)?;
                        commands.push(gpu::Command::Copy { from: gpu::Range { memory: m, offset: 0, size: bytes.len() as u64 }, to, at });
                    }
                    Rec::ToImage { bytes, to, sub, region } => {
                        let m = temp(d, &mut temps, &bytes, bytes.len() as u64)?;
                        commands.push(gpu::Command::CopyBufferToImage { from: gpu::Range { memory: m, offset: 0, size: bytes.len() as u64 }, row_pixels: 0, to, sub, region });
                    }
                    Rec::ImageToImage { from, from_sub, region, to, to_sub, x, y, bytes } => {
                        let size = region.width as u64 * region.height as u64 * bytes as u64;
                        let m = temp(d, &mut temps, &[], size)?;
                        let range = gpu::Range { memory: m, offset: 0, size };
                        commands.push(gpu::Command::CopyImageToBuffer { from, sub: from_sub, region, to: range, row_pixels: 0 });
                        commands.push(gpu::Command::CopyBufferToImage { from: range, row_pixels: 0, to, sub: to_sub, region: Rect { x, y, ..region } });
                    }
                }
            }
            let done = d.submit(gpu::Submit { commands: &commands, wait: &[], signal: &[(fence, value)] }).and_then(|()| d.wait(fence, value, LONGEST));
            for m in temps {
                d.free(m);
            }
            match done? {
                true => Ok(()),
                false => Err(gpu::Error { message: "the work didn't finish in time".into() }),
            }
        })
    }
}

impl Texture {
    /// A subresource's bytes from initial data whose rows are `row_pitch` apart, tightly packed.
    fn packed(&self, index: u32, data: &Data<'_>) -> Result<Vec<u8>, Error> {
        let (w, h) = self.size(index % self.levels);
        let row = w as usize * self.format.bytes() as usize;
        let pitch = if data.row_pitch == 0 { row } else { data.row_pitch as usize };
        let mut out = Vec::with_capacity(row * h as usize);
        for y in 0..h as usize {
            let Some(src) = data.bytes.get(y * pitch..y * pitch + row) else { return invalid(format!("subresource {index}'s data ends at row {y}")) };
            out.extend_from_slice(src);
        }
        Ok(out)
    }
}
