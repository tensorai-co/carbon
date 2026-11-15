//! The device model's draw half: images and samplers, graphics pipelines (a vertex and a fragment entry point and
//! the fixed-function state around them), and the commands that render with them.
//!
//! It's the shape Vulkan's core draws have, and Metal's, below their APIs: a render pass of colour and depth
//! attachments, a viewport and scissor, draws of triangles with or without an index buffer, and copies between
//! buffers and images. Images have mip levels and array layers; an attachment or a copy is one level of one layer,
//! and sampling reads layer 0 (array sampling comes with the frontends that ask for it). Bindings are named by Vulkan's (set, binding), shared by both stages.

use crate::{Pipeline, Range, ir};

/// An image: 2D arrays of texels in one format, at each mip level of each array layer; a draw's attachment or a
/// sampler's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Image(pub(crate) u32);

/// How images are read: filtering and addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sampler(pub(crate) u32);

/// A texel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Rgba8Unorm,
    /// sRGB-encoded colour, linear alpha: decoded when read, encoded when written.
    Rgba8Srgb,
    Bgra8Unorm,
    Bgra8Srgb,
    /// One channel, read as red (green and blue 0, alpha 1).
    R8Unorm,
    Rg8Unorm,
    /// Alpha alone, read with red, green and blue 0.
    A8Unorm,
    R32Float,
    /// Four IEEE halves.
    Rgba16Float,
    /// Red, green and blue in 10 bits each and alpha in 2, red lowest.
    Rgb10a2Unorm,
    /// Red and green as unsigned 11-bit floats and blue as a 10-bit one, red lowest.
    Rg11b10Float,
    Rgba32Float,
    D32Float,
    D16Unorm,
}

impl Format {
    pub const fn bytes(self) -> u32 {
        match self {
            Format::Rgba8Unorm | Format::Rgba8Srgb | Format::Bgra8Unorm | Format::Bgra8Srgb | Format::D32Float => 4,
            Format::R32Float | Format::Rgb10a2Unorm | Format::Rg11b10Float => 4,
            Format::R8Unorm | Format::A8Unorm => 1,
            Format::Rg8Unorm | Format::D16Unorm => 2,
            Format::Rgba16Float => 8,
            Format::Rgba32Float => 16,
        }
    }

    pub const fn is_depth(self) -> bool {
        matches!(self, Format::D32Float | Format::D16Unorm)
    }

    /// Whether it can be a colour attachment (depth formats are depth attachments). Metal can't draw into alpha-only
    /// images, so neither device does.
    pub const fn is_renderable(self) -> bool {
        !matches!(self, Format::A8Unorm)
    }
}

/// A 2D image: its format and size, its mip levels (each half the one before, rounded down, at least 1), and its
/// array layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDesc {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub layers: u32,
}

impl ImageDesc {
    /// One level and one layer.
    pub const fn new(format: Format, width: u32, height: u32) -> ImageDesc {
        ImageDesc { format, width, height, levels: 1, layers: 1 }
    }

    /// A level's width and height.
    pub const fn size(&self, level: u32) -> (u32, u32) {
        let w = self.width >> level;
        let h = self.height >> level;
        (if w == 0 { 1 } else { w }, if h == 0 { 1 } else { h })
    }

    /// The most levels an image of this size has: down to 1×1.
    pub const fn most_levels(width: u32, height: u32) -> u32 {
        let m = if width > height { width } else { height };
        32 - m.leading_zeros()
    }
}

/// One level of one layer of an image: what a render pass draws into, and what a copy reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Sub {
    pub level: u32,
    pub layer: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    Nearest,
    Linear,
}

/// What a coordinate outside `[0, 1]` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Address {
    Repeat,
    MirroredRepeat,
    ClampToEdge,
    /// The sampler's border colour.
    ClampToBorder,
}

/// The colour outside the image, for [`Address::ClampToBorder`]: the three Metal and Vulkan both have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Border {
    #[default]
    TransparentBlack,
    OpaqueBlack,
    OpaqueWhite,
}

impl Border {
    pub const fn rgba(self) -> [f32; 4] {
        match self {
            Border::TransparentBlack => [0.0; 4],
            Border::OpaqueBlack => [0.0, 0.0, 0.0, 1.0],
            Border::OpaqueWhite => [1.0; 4],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerDesc {
    /// Used when a pixel covers less than a texel (magnified).
    pub mag: Filter,
    /// Used when it covers more (minified).
    pub min: Filter,
    /// Between mip levels: the nearest level, or the two around the level of detail blended.
    pub mipmap: Filter,
    /// For u and v.
    pub address: [Address; 2],
    /// The levels of detail sampling keeps to, the lowest and highest.
    pub lod: [f32; 2],
    pub border: Border,
    /// A comparison sampler's test: a depth image's texels compared with a reference, giving 1 or 0 for each before
    /// filtering (percentage-closer filtering when linear).
    pub compare: Option<Compare>,
}

impl SamplerDesc {
    /// Filtering and addressing; every level, the default border, no comparison.
    pub const fn new(mag: Filter, min: Filter, address: [Address; 2]) -> SamplerDesc {
        SamplerDesc { mag, min, mipmap: Filter::Nearest, address, lod: [0.0, 1000.0], border: Border::TransparentBlack, compare: None }
    }
}

/// What a draw binds at one (set, binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bind {
    Buffer(Range),
    Image(Image),
    Sampler(Sampler),
    Combined(Image, Sampler),
}

/// A vertex attribute's format in its buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexFormat {
    /// 1 to 4 `f32`s.
    F32(u8),
    /// 1 to 4 `u32`s or `i32`s.
    U32(u8),
    I32(u8),
    /// Four bytes, each read as `n / 255`.
    Unorm8x4,
}

impl VertexFormat {
    pub const fn bytes(self) -> u32 {
        match self {
            VertexFormat::F32(n) | VertexFormat::U32(n) | VertexFormat::I32(n) => 4 * n as u32,
            VertexFormat::Unorm8x4 => 4,
        }
    }

    /// The scalar a shader reads it as.
    pub const fn scalar(self) -> ir::Scalar {
        match self {
            VertexFormat::F32(_) | VertexFormat::Unorm8x4 => ir::Scalar::F32,
            VertexFormat::U32(_) => ir::Scalar::U32,
            VertexFormat::I32(_) => ir::Scalar::I32,
        }
    }
}

/// A vertex buffer's layout: the bytes from one vertex (or instance) to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexBuffer {
    pub stride: u32,
    /// Stepped once an instance, not once a vertex.
    pub per_instance: bool,
}

/// Where a vertex shader input at a location comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute {
    pub location: u32,
    /// Which of the pipeline's vertex buffers.
    pub buffer: u32,
    pub offset: u32,
    pub format: VertexFormat,
}

/// What a draw's vertices make. Lines are one pixel wide, and never culled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topology {
    TriangleList,
    TriangleStrip,
    TriangleFan,
    LineList,
    LineStrip,
}

impl Topology {
    pub const fn is_line(self) -> bool {
        matches!(self, Topology::LineList | Topology::LineStrip)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cull {
    None,
    Front,
    Back,
}

/// Which winding faces the viewer, in framebuffer coordinates (y down), as Vulkan has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrontFace {
    CounterClockwise,
    Clockwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Compare {
    Never,
    Less,
    Equal,
    LessOrEqual,
    Greater,
    NotEqual,
    GreaterOrEqual,
    Always,
}

impl Compare {
    /// Whether a fragment's value passes against what's there.
    pub fn passes(self, new: f32, old: f32) -> bool {
        match self {
            Compare::Never => false,
            Compare::Less => new < old,
            Compare::Equal => new == old,
            Compare::LessOrEqual => new <= old,
            Compare::Greater => new > old,
            Compare::NotEqual => new != old,
            Compare::GreaterOrEqual => new >= old,
            Compare::Always => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthState {
    pub compare: Compare,
    pub write: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    DstColor,
    OneMinusDstColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendOp {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

/// `src * src_factor (op) dst * dst_factor`, for colour and for alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Blend {
    pub color: (Factor, Factor, BlendOp),
    pub alpha: (Factor, Factor, BlendOp),
}

/// A colour attachment the pipeline writes: fragment output location `n` goes to attachment `n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorTarget {
    pub format: Format,
    pub blend: Option<Blend>,
    /// Which of red, green, blue and alpha are written.
    pub write: [bool; 4],
}

/// A graphics pipeline's programs and fixed-function state.
#[derive(Debug, Clone, Copy)]
pub struct GraphicsDesc<'a> {
    pub vertex: &'a ir::Entry,
    pub fragment: &'a ir::Entry,
    pub buffers: &'a [VertexBuffer],
    pub attributes: &'a [Attribute],
    pub topology: Topology,
    pub cull: Cull,
    pub front: FrontFace,
    pub depth: Option<DepthState>,
    pub depth_format: Option<Format>,
    pub colors: &'a [ColorTarget],
}

/// What a render pass does with an attachment's contents at its start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Load<T> {
    Load,
    Clear(T),
    /// Whatever's there; it may be anything.
    DontCare,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorAttachment {
    pub image: Image,
    pub sub: Sub,
    pub load: Load<[f32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthAttachment {
    pub image: Image,
    pub sub: Sub,
    pub load: Load<f32>,
}

/// Where normalized device coordinates land in the framebuffer, and the depth range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// Negative flips y, as Vulkan allows.
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

/// Pixels: a scissor, or a region of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexType {
    U16,
    U32,
}

/// One draw of triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct Draw {
    pub pipeline: Pipeline,
    /// What each (set, binding) the pipeline's stages use is bound to.
    pub bindings: Vec<(u32, u32, Bind)>,
    /// One range for each of the pipeline's vertex buffers.
    pub vertex_buffers: Vec<Range>,
    /// An index buffer: the vertices are its values (plus `vertex_offset`) from index `first` on.
    pub index: Option<(Range, IndexType)>,
    /// The first vertex (or index), and how many.
    pub first: u32,
    pub count: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
    pub instances: u32,
}
