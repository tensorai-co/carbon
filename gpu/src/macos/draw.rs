//! Metal's half of the device model's draws: textures (in the GPU's own memory: on Apple silicon depth can't be
//! shared, and nothing reads them but the GPU and blits), sampler states, render pipeline states made from two AIR
//! libraries (the vertex function fetching its own attributes, `air::VertexInput`), depth-stencil states, render
//! passes, and blits between buffers and textures.
//!
//! Each stage's bindings are at their index in that stage's entry point (`[[buffer(n)]]`, `[[texture(n)]]`,
//! `[[sampler(n)]]`), so a draw binds what the device model names by (set, binding) at each stage's own numbers, and
//! the vertex buffers after the vertex function's bindings. Metal has no triangle fans: a fan is drawn as the list of
//! triangles it makes, through an index buffer made for the draw.

use std::ffi::c_void;

use gpu_backend::air;
use gpu_ir::Entry;

use super::Metal;
use super::objc::{Id, class, error_text, msg, pooled, release};
use crate::{Address, Bind, Blend, BlendOp, Compare, Cull, Draw, Error, Factor, Filter, Format, FrontFace, GraphicsDesc, Image, ImageDesc, IndexType, Load, Rect, Sampler, SamplerDesc, Topology, VertexFormat, Viewport, err};

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct MtlViewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    znear: f64,
    zfar: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct ScissorRect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ClearColor {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Origin {
    x: usize,
    y: usize,
    z: usize,
}

const STORAGE_PRIVATE: usize = 2;
const USAGE_SHADER_READ: usize = 1;
const USAGE_RENDER_TARGET: usize = 4;
const LOAD_DONT_CARE: usize = 0;
const LOAD_LOAD: usize = 1;
const LOAD_CLEAR: usize = 2;
const STORE_STORE: usize = 1;
const PRIMITIVE_TRIANGLE: usize = 3;
const PRIMITIVE_TRIANGLE_STRIP: usize = 4;

/// A texture, and what it was made as.
pub(super) struct Texture {
    pub(super) texture: Id,
    pub(super) desc: ImageDesc,
}

/// A render pipeline state and the fixed-function state Metal sets on the encoder, with each stage's bindings in
/// the order its function takes them.
pub(super) struct Graphics {
    state: Id,
    depth: Id,
    cull: usize,
    winding: usize,
    topology: Topology,
    vertex: Vec<(u32, u32)>,
    fragment: Vec<(u32, u32)>,
    vertex_buffers_at: u32,
}

impl Drop for Graphics {
    fn drop(&mut self) {
        release(self.state);
        release(self.depth);
    }
}

fn pixel_format(f: Format) -> usize {
    match f {
        Format::Rgba8Unorm => 70,
        Format::Rgba8Srgb => 71,
        Format::Bgra8Unorm => 80,
        Format::Bgra8Srgb => 81,
        Format::Rgba32Float => 125,
        Format::D16Unorm => 250,
        Format::D32Float => 252,
    }
}

fn blend_factor(f: Factor) -> usize {
    match f {
        Factor::Zero => 0,
        Factor::One => 1,
        Factor::SrcColor => 2,
        Factor::OneMinusSrcColor => 3,
        Factor::SrcAlpha => 4,
        Factor::OneMinusSrcAlpha => 5,
        Factor::DstColor => 6,
        Factor::OneMinusDstColor => 7,
        Factor::DstAlpha => 8,
        Factor::OneMinusDstAlpha => 9,
    }
}

fn blend_op(op: BlendOp) -> usize {
    match op {
        BlendOp::Add => 0,
        BlendOp::Subtract => 1,
        BlendOp::ReverseSubtract => 2,
        BlendOp::Min => 3,
        BlendOp::Max => 4,
    }
}

/// Metal's compare functions are numbered as the device model lists them.
fn compare(c: Compare) -> usize {
    c as usize
}

fn vertex_format(f: VertexFormat) -> air::Format {
    match f {
        VertexFormat::F32(n) => air::Format::F32(n),
        VertexFormat::U32(n) => air::Format::U32(n),
        VertexFormat::I32(n) => air::Format::I32(n),
        VertexFormat::Unorm8x4 => air::Format::Unorm8x4,
    }
}

impl Metal {
    /// A function from an entry point's AIR library; a vertex function's with its vertex input.
    pub(super) fn function(&self, entry: &Entry, vertex: Option<&air::VertexInput>) -> Result<(Id, air::Library), Error> {
        let lib = air::write_with(entry, vertex).map_err(|e| Error { message: format!("{}: {e}", entry.name) })?;
        let f = self.load(&entry.name, &lib)?;
        Ok((f, lib))
    }

    pub(super) fn make_image(&mut self, desc: &ImageDesc) -> Result<Image, Error> {
        let texture = pooled(|| {
            let d = msg!(Id; class("MTLTextureDescriptor"), "texture2DDescriptorWithPixelFormat:width:height:mipmapped:",
                pixel_format(desc.format) => usize, desc.width as usize => usize, desc.height as usize => usize, false => bool);
            msg!((); d, "setStorageMode:", STORAGE_PRIVATE => usize);
            msg!((); d, "setUsage:", USAGE_SHADER_READ | USAGE_RENDER_TARGET => usize);
            msg!(Id; self.device, "newTextureWithDescriptor:", d => Id)
        });
        if texture.is_null() {
            return err(format!("Metal couldn't make a {}×{} {:?} texture", desc.width, desc.height, desc.format));
        }
        let image = Image(self.next_image);
        self.next_image += 1;
        self.textures.insert(image, Texture { texture, desc: *desc });
        Ok(image)
    }

    pub(super) fn drop_image(&mut self, image: Image) {
        if let Some(t) = self.textures.remove(&image) {
            release(t.texture);
        }
    }

    pub(super) fn make_sampler(&mut self, desc: &SamplerDesc) -> Result<Sampler, Error> {
        let filter = |f: Filter| if f == Filter::Linear { 1usize } else { 0 };
        let address = |a: Address| match a {
            Address::ClampToEdge => 0usize,
            Address::Repeat => 2,
            Address::MirroredRepeat => 3,
        };
        let state = pooled(|| {
            let d = msg!(Id; msg!(Id; class("MTLSamplerDescriptor"), "alloc"), "init");
            msg!((); d, "setMinFilter:", filter(desc.min) => usize);
            msg!((); d, "setMagFilter:", filter(desc.mag) => usize);
            msg!((); d, "setSAddressMode:", address(desc.address[0]) => usize);
            msg!((); d, "setTAddressMode:", address(desc.address[1]) => usize);
            let s = msg!(Id; self.device, "newSamplerStateWithDescriptor:", d => Id);
            release(d);
            s
        });
        if state.is_null() {
            return err("Metal couldn't make a sampler state");
        }
        self.samplers.push(state);
        Ok(Sampler(self.samplers.len() as u32 - 1))
    }

    /// A render pipeline: the vertex function (fetching its attributes by the description's layout) and the
    /// fragment function, the attachments' formats and blending, and the depth test as a depth-stencil state.
    pub(super) fn make_graphics(&mut self, desc: &GraphicsDesc<'_>) -> Result<Graphics, Error> {
        let layout = air::VertexInput {
            buffers: desc.buffers.iter().map(|b| air::VertexBuffer { stride: b.stride, per_instance: b.per_instance }).collect(),
            attributes: desc.attributes.iter().map(|a| air::Attribute { location: a.location, buffer: a.buffer, offset: a.offset, format: vertex_format(a.format) }).collect(),
        };
        let (vf, vlib) = self.function(desc.vertex, Some(&layout))?;
        let ff = match self.function(desc.fragment, None) {
            Ok((f, _)) => f,
            Err(e) => {
                release(vf);
                return Err(e);
            }
        };
        let made = pooled(|| -> Result<(Id, Id), Error> {
            let d = msg!(Id; msg!(Id; class("MTLRenderPipelineDescriptor"), "alloc"), "init");
            msg!((); d, "setVertexFunction:", vf => Id);
            msg!((); d, "setFragmentFunction:", ff => Id);
            let colors = msg!(Id; d, "colorAttachments");
            for (i, c) in desc.colors.iter().enumerate() {
                let a = msg!(Id; colors, "objectAtIndexedSubscript:", i => usize);
                msg!((); a, "setPixelFormat:", pixel_format(c.format) => usize);
                // Metal's write mask is red 8, green 4, blue 2, alpha 1.
                let mask = c.write.iter().zip([8usize, 4, 2, 1]).filter(|(on, _)| **on).map(|(_, bit)| bit).sum::<usize>();
                msg!((); a, "setWriteMask:", mask => usize);
                if let Some(Blend { color, alpha }) = c.blend {
                    msg!((); a, "setBlendingEnabled:", true => bool);
                    msg!((); a, "setSourceRGBBlendFactor:", blend_factor(color.0) => usize);
                    msg!((); a, "setDestinationRGBBlendFactor:", blend_factor(color.1) => usize);
                    msg!((); a, "setRgbBlendOperation:", blend_op(color.2) => usize);
                    msg!((); a, "setSourceAlphaBlendFactor:", blend_factor(alpha.0) => usize);
                    msg!((); a, "setDestinationAlphaBlendFactor:", blend_factor(alpha.1) => usize);
                    msg!((); a, "setAlphaBlendOperation:", blend_op(alpha.2) => usize);
                }
            }
            if let Some(f) = desc.depth_format {
                msg!((); d, "setDepthAttachmentPixelFormat:", pixel_format(f) => usize);
            }
            let mut error: Id = std::ptr::null_mut();
            let state = msg!(Id; self.device, "newRenderPipelineStateWithDescriptor:error:", d => Id, &mut error => *mut Id);
            release(d);
            if state.is_null() {
                return err(format!("Metal couldn't make a render pipeline: {}", error_text(error)));
            }
            let ds = msg!(Id; msg!(Id; class("MTLDepthStencilDescriptor"), "alloc"), "init");
            match desc.depth {
                Some(depth) => {
                    msg!((); ds, "setDepthCompareFunction:", compare(depth.compare) => usize);
                    msg!((); ds, "setDepthWriteEnabled:", depth.write => bool);
                }
                None => msg!((); ds, "setDepthCompareFunction:", compare(Compare::Always) => usize),
            }
            let depth = msg!(Id; self.device, "newDepthStencilStateWithDescriptor:", ds => Id);
            release(ds);
            Ok((state, depth))
        });
        release(vf);
        release(ff);
        let (state, depth) = made?;
        let cull = match desc.cull {
            Cull::None => 0,
            Cull::Front => 1,
            Cull::Back => 2,
        };
        // Metal's windings: clockwise 0, counter-clockwise 1. The vertex function negates y, so a triangle lands where
        // Vulkan's would and keeps its winding on the screen.
        let winding = if desc.front == FrontFace::Clockwise { 0 } else { 1 };
        let list = |e: &Entry| e.bindings.iter().map(|b| (b.set, b.binding)).collect();
        Ok(Graphics {
            state,
            depth,
            cull,
            winding,
            topology: desc.topology,
            vertex: list(desc.vertex),
            fragment: list(desc.fragment),
            vertex_buffers_at: vlib.vertex_buffers_at,
        })
    }

    pub(super) fn texture(&self, image: Image) -> Result<&Texture, Error> {
        self.textures.get(&image).ok_or_else(|| Error { message: format!("{image:?} isn't made") })
    }

    /// A render pass's encoder: its attachments, loaded, cleared or not, and stored.
    pub(super) fn begin_render(&self, cb: Id, colors: &[crate::ColorAttachment], depth: Option<crate::DepthAttachment>) -> Result<Id, Error> {
        let d = msg!(Id; class("MTLRenderPassDescriptor"), "renderPassDescriptor");
        let attachments = msg!(Id; d, "colorAttachments");
        for (i, c) in colors.iter().enumerate() {
            let a = msg!(Id; attachments, "objectAtIndexedSubscript:", i => usize);
            msg!((); a, "setTexture:", self.texture(c.image)?.texture => Id);
            let action = match c.load {
                Load::Load => LOAD_LOAD,
                Load::Clear(v) => {
                    msg!((); a, "setClearColor:", ClearColor { r: v[0] as f64, g: v[1] as f64, b: v[2] as f64, a: v[3] as f64 } => ClearColor);
                    LOAD_CLEAR
                }
                Load::DontCare => LOAD_DONT_CARE,
            };
            msg!((); a, "setLoadAction:", action => usize);
            msg!((); a, "setStoreAction:", STORE_STORE => usize);
        }
        if let Some(z) = depth {
            let a = msg!(Id; d, "depthAttachment");
            msg!((); a, "setTexture:", self.texture(z.image)?.texture => Id);
            let action = match z.load {
                Load::Load => LOAD_LOAD,
                Load::Clear(v) => {
                    msg!((); a, "setClearDepth:", v as f64 => f64);
                    LOAD_CLEAR
                }
                Load::DontCare => LOAD_DONT_CARE,
            };
            msg!((); a, "setLoadAction:", action => usize);
            msg!((); a, "setStoreAction:", STORE_STORE => usize);
        }
        let enc = msg!(Id; cb, "renderCommandEncoderWithDescriptor:", d => Id);
        if enc.is_null() {
            return err("Metal gave no render encoder");
        }
        Ok(enc)
    }

    pub(super) fn viewport(enc: Id, v: &Viewport) {
        let mv = MtlViewport { x: v.x as f64, y: v.y as f64, width: v.width as f64, height: v.height as f64, znear: v.min_depth as f64, zfar: v.max_depth as f64 };
        msg!((); enc, "setViewport:", mv => MtlViewport);
    }

    pub(super) fn scissor(enc: Id, r: &Rect) {
        let s = ScissorRect { x: r.x as usize, y: r.y as usize, width: r.width as usize, height: r.height as usize };
        msg!((); enc, "setScissorRect:", s => ScissorRect);
    }

    /// One draw: the pipeline's state, each stage's bindings at its own numbers, the vertex buffers, then the draw
    /// (a fan as the triangles it makes).
    pub(super) fn draw(&self, enc: Id, g: &Graphics, d: &Draw, made: &mut Vec<Id>) -> Result<(), Error> {
        msg!((); enc, "setRenderPipelineState:", g.state => Id);
        msg!((); enc, "setDepthStencilState:", g.depth => Id);
        msg!((); enc, "setCullMode:", g.cull => usize);
        msg!((); enc, "setFrontFacingWinding:", g.winding => usize);
        for (stage, list) in [("Vertex", &g.vertex), ("Fragment", &g.fragment)] {
            for (slot, (set, binding)) in list.iter().enumerate() {
                let Some((_, _, bind)) = d.bindings.iter().find(|(s, b, _)| s == set && b == binding) else {
                    return err(format!("set {set} binding {binding} isn't bound"));
                };
                match *bind {
                    Bind::Buffer(r) => {
                        let (buffer, _) = self.buffer(r.memory)?;
                        msg!((); enc, &format!("set{stage}Buffer:offset:atIndex:"), buffer => Id, r.offset as usize => usize, slot => usize);
                    }
                    Bind::Image(i) => msg!((); enc, &format!("set{stage}Texture:atIndex:"), self.texture(i)?.texture => Id, slot => usize),
                    Bind::Sampler(s) => msg!((); enc, &format!("set{stage}SamplerState:atIndex:"), self.samplers[s.0 as usize] => Id, slot => usize),
                    Bind::Combined(i, s) => {
                        msg!((); enc, &format!("set{stage}Texture:atIndex:"), self.texture(i)?.texture => Id, slot => usize);
                        msg!((); enc, &format!("set{stage}SamplerState:atIndex:"), self.samplers[s.0 as usize] => Id, slot => usize);
                    }
                }
            }
        }
        for (k, r) in d.vertex_buffers.iter().enumerate() {
            let (buffer, _) = self.buffer(r.memory)?;
            let slot = g.vertex_buffers_at as usize + k;
            msg!((); enc, "setVertexBuffer:offset:atIndex:", buffer => Id, r.offset as usize => usize, slot => usize);
        }
        let instances = d.instances as usize;
        if g.topology == Topology::TriangleFan {
            return self.draw_fan(enc, d, made);
        }
        let primitive = if g.topology == Topology::TriangleStrip { PRIMITIVE_TRIANGLE_STRIP } else { PRIMITIVE_TRIANGLE };
        match d.index {
            None => msg!((); enc, "drawPrimitives:vertexStart:vertexCount:instanceCount:baseInstance:",
                primitive => usize, d.first as usize => usize, d.count as usize => usize, instances => usize, d.first_instance as usize => usize),
            Some((r, ty)) => {
                let (buffer, _) = self.buffer(r.memory)?;
                let (mtl_type, size) = if ty == IndexType::U16 { (0usize, 2u64) } else { (1usize, 4u64) };
                msg!((); enc, "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:baseVertex:baseInstance:",
                    primitive => usize, d.count as usize => usize, mtl_type => usize, buffer => Id, (r.offset + d.first as u64 * size) as usize => usize,
                    instances => usize, d.vertex_offset as isize => isize, d.first_instance as usize => usize);
            }
        }
        Ok(())
    }

    /// A triangle fan as a triangle list: vertex 0 with each pair after it, through 32-bit indices made for the draw
    /// (an indexed fan's own indices, read from its buffer, which the CPU sees).
    fn draw_fan(&self, enc: Id, d: &Draw, made: &mut Vec<Id>) -> Result<(), Error> {
        let vertices: Vec<u32> = match d.index {
            None => (0..d.count).map(|i| d.first + i).collect(),
            Some((r, ty)) => {
                let size = if ty == IndexType::U16 { 2 } else { 4 };
                let base = self.contents(r.memory, r.offset + d.first as u64 * size, (d.count as u64 * size) as usize)?;
                (0..d.count as usize)
                    .map(|i| unsafe {
                        // SAFETY: inside the range just checked.
                        if size == 2 { u32::from(u16::from_le_bytes([*base.add(i * 2), *base.add(i * 2 + 1)])) } else { u32::from_le_bytes([*base.add(i * 4), *base.add(i * 4 + 1), *base.add(i * 4 + 2), *base.add(i * 4 + 3)]) }
                    })
                    .map(|v| (v as i64 + if d.index.is_some() { d.vertex_offset as i64 } else { 0 }) as u32)
                    .collect()
            }
        };
        let mut list = Vec::new();
        for i in 1..vertices.len().saturating_sub(1) {
            list.extend([vertices[0], vertices[i], vertices[i + 1]]);
        }
        if list.is_empty() {
            return Ok(());
        }
        let buffer = msg!(Id; self.device, "newBufferWithBytes:length:options:", list.as_ptr() as *const c_void => *const c_void, list.len() * 4 => usize, 0usize => usize);
        if buffer.is_null() {
            return err("Metal couldn't make a fan's index buffer");
        }
        made.push(buffer);
        msg!((); enc, "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:baseVertex:baseInstance:",
            PRIMITIVE_TRIANGLE => usize, list.len() => usize, 1usize => usize, buffer => Id, 0usize => usize,
            d.instances as usize => usize, 0isize => isize, d.first_instance as usize => usize);
        Ok(())
    }

    /// Texels from a buffer into a region of a texture, `row` texels to a row (0: the region's width).
    pub(super) fn copy_to_texture(&self, cb: Id, from: crate::Range, row: u32, to: Image, region: Rect) -> Result<(), Error> {
        let t = self.texture(to)?;
        let (buffer, _) = self.buffer(from.memory)?;
        let bytes_per_row = row.max(region.width) as usize * t.desc.format.bytes() as usize;
        let size = super::Size { width: region.width as usize, height: region.height as usize, depth: 1 };
        let origin = Origin { x: region.x as usize, y: region.y as usize, z: 0 };
        let enc = msg!(Id; cb, "blitCommandEncoder");
        msg!((); enc, "copyFromBuffer:sourceOffset:sourceBytesPerRow:sourceBytesPerImage:sourceSize:toTexture:destinationSlice:destinationLevel:destinationOrigin:",
            buffer => Id, from.offset as usize => usize, bytes_per_row => usize, bytes_per_row * region.height as usize => usize, size => super::Size,
            t.texture => Id, 0usize => usize, 0usize => usize, origin => Origin);
        msg!((); enc, "endEncoding");
        Ok(())
    }

    /// A region of a texture into a buffer: what a present reads back.
    pub(super) fn copy_from_texture(&self, cb: Id, from: Image, region: Rect, to: crate::Range, row: u32) -> Result<(), Error> {
        let t = self.texture(from)?;
        let (buffer, _) = self.buffer(to.memory)?;
        let bytes_per_row = row.max(region.width) as usize * t.desc.format.bytes() as usize;
        let size = super::Size { width: region.width as usize, height: region.height as usize, depth: 1 };
        let origin = Origin { x: region.x as usize, y: region.y as usize, z: 0 };
        let enc = msg!(Id; cb, "blitCommandEncoder");
        msg!((); enc, "copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:",
            t.texture => Id, 0usize => usize, 0usize => usize, origin => Origin, size => super::Size,
            buffer => Id, to.offset as usize => usize, bytes_per_row => usize, bytes_per_row * region.height as usize => usize);
        msg!((); enc, "endEncoding");
        Ok(())
    }
}
