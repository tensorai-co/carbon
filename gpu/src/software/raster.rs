//! The CPU device's draws: Vulkan's rasterization rules, run on the host.
//!
//! - **Vertices** are fetched from their buffers by the pipeline's attributes and shaded 32 at a time by the
//!   interpreter, each vertex once a draw instance however many triangles share it.
//! - **Triangles** are assembled (lists, strips, fans; the provoking vertex is each one's first), clipped in clip
//!   space to `0 <= z <= w` and a guard band in x and y, and taken to framebuffer coordinates by the viewport. Their
//!   facing is Vulkan's: the sign of `-½ Σ (xᵢ yᵢ₊₁ − xᵢ₊₁ yᵢ)` in framebuffer coordinates (y down).
//! - **Coverage** is tested at pixel centres with edge functions in fixed point, 8 bits of subpixel precision, and
//!   the top-left rule, so pixels on an edge two triangles share are drawn once.
//! - **Fragments** are shaded in 2×2 quads, 8 quads a group: pixels the triangle doesn't cover run as helpers, for
//!   derivatives. Inputs are interpolated perspective-correct (or linearly, or flat) with the barycentric weights.
//! - **Results** are depth-tested, blended and written in primitive order, after their group is shaded.

use std::collections::{BTreeMap, HashMap};

use gpu_backend::cpu::{self, Bound, Builtins, Lanes};
use gpu_ir::{Entry, Interpolation, Scalar, Slot};

use super::texel::Texels;
use super::{Space, Tex};
use crate::{
    Attribute, Bind, Blend, BlendOp, ColorTarget, Cull, DepthState, Draw, Error, Factor, FrontFace, IndexType, Memory,
    Rect, SamplerDesc, Topology, VertexBuffer, VertexFormat, Viewport, err,
};

/// A graphics pipeline as the CPU device keeps it.
pub(crate) struct Graphics {
    pub vertex: Entry,
    pub fragment: Entry,
    pub buffers: Vec<VertexBuffer>,
    pub attributes: Vec<Attribute>,
    pub topology: Topology,
    pub cull: Cull,
    pub front: FrontFace,
    pub depth: Option<DepthState>,
    pub colors: Vec<ColorTarget>,
}

/// The render pass a draw is in.
pub(crate) struct Target {
    pub colors: Vec<u32>,
    pub depth: Option<u32>,
    pub viewport: Viewport,
    pub scissor: Rect,
}

/// The parts of the CPU device a draw reads and writes.
pub(crate) struct Device<'a> {
    pub memory: &'a mut BTreeMap<u64, Vec<u8>>,
    pub addresses: &'a HashMap<Memory, u64>,
    pub images: &'a mut [Option<Texels>],
    pub samplers: &'a [SamplerDesc],
    pub subgroup: u32,
}

/// Lanes a group of invocations has: a subgroup, 8 quads.
const GROUP: usize = 32;
/// How far outside the viewport, in viewports, triangles are kept before they're clipped.
const GUARD: f64 = 4.0;
/// Subpixel precision: 8 bits.
const SUBPIXEL: f64 = 256.0;

/// A shaded vertex: its clip-space position and what the fragment shader reads of it, by the fragment's inputs.
#[derive(Clone)]
struct Vert {
    clip: [f64; 4],
    vary: Vec<Lanes>,
}

/// A vertex of a polygon being clipped: position, and the interpolated inputs as floats.
#[derive(Clone)]
struct ClipVert {
    p: [f64; 4],
    v: Vec<[f32; 4]>,
}

/// One fragment in a group: where, and whether it's covered (else a helper).
#[derive(Clone, Copy)]
struct Frag {
    x: u32,
    y: u32,
    z: f32,
    covered: bool,
}

fn bits(x: f32) -> u64 {
    x.to_bits() as u64
}

fn float(b: u64) -> f32 {
    f32::from_bits(b as u32)
}

impl Device<'_> {
    fn bound(&self, entry: &Entry, d: &Draw) -> Result<Vec<Bound>, Error> {
        let mut out = Vec::new();
        for b in &entry.bindings {
            let Some((_, _, bind)) = d.bindings.iter().find(|(s, n, _)| (*s, *n) == (b.set, b.binding)) else {
                return err(format!("set {} binding {} isn't bound", b.set, b.binding));
            };
            out.push(match *bind {
                Bind::Buffer(r) => Bound::Buffer(self.addresses[&r.memory] + r.offset),
                Bind::Image(i) => Bound::Image(i.0),
                Bind::Sampler(s) => Bound::Sampler(s.0),
                Bind::Combined(i, s) => Bound::Combined(i.0, s.0),
            });
        }
        Ok(out)
    }

    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), Error> {
        cpu::Memory::read(&mut Space(self.memory), addr, out).map_err(Error::from)
    }

    fn shade(&mut self, entry: &Entry, s: &cpu::Shade<'_>) -> Result<cpu::Shaded, Error> {
        let mut tex = Tex { images: self.images, samplers: self.samplers };
        cpu::shade(entry, s, &mut Space(self.memory), &mut tex).map_err(Error::from)
    }

    /// An attribute's value for one vertex or instance, as the input's components (missing ones `(0, 0, 0, 1)`).
    fn attribute(&mut self, g: &Graphics, d: &Draw, a: &Attribute, vertex: u32, instance: u32) -> Result<Lanes, Error> {
        let layout = g.buffers[a.buffer as usize];
        let range = d.vertex_buffers[a.buffer as usize];
        let element = if layout.per_instance { instance } else { vertex } as u64;
        let at = range.offset + element * layout.stride as u64 + a.offset as u64;
        let size = a.format.bytes() as u64;
        if at + size > range.offset + range.size {
            return err(format!("vertex {vertex}'s attribute at location {} is past its buffer", a.location));
        }
        let mut b = [0u8; 16];
        self.read(self.addresses[&range.memory] + at, &mut b[..size as usize])?;
        let word = |i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        Ok(match a.format {
            VertexFormat::F32(n) => {
                let mut r = [0, 0, 0, bits(1.0)];
                for (i, c) in r.iter_mut().enumerate().take(n as usize) {
                    *c = word(i) as u64;
                }
                r
            }
            VertexFormat::U32(n) | VertexFormat::I32(n) => {
                let mut r = [0, 0, 0, 1];
                for (i, c) in r.iter_mut().enumerate().take(n as usize) {
                    *c = word(i) as u64;
                }
                r
            }
            VertexFormat::Unorm8x4 => [0, 1, 2, 3].map(|i| bits(b[i] as f32 / 255.0)),
        })
    }

    pub fn draw(&mut self, g: &Graphics, d: &Draw, t: &Target) -> Result<(), Error> {
        let vbound = self.bound(&g.vertex, d)?;
        let fbound = self.bound(&g.fragment, d)?;
        let position = g.vertex.outputs.iter().position(|o| o.slot == Slot::Position).unwrap_or(0);
        // Each fragment input's vertex output.
        let mut from = Vec::new();
        for input in &g.fragment.inputs {
            match g.vertex.outputs.iter().position(|o| o.slot == input.slot) {
                Some(i) => from.push(i),
                None => return err(format!("the fragment shader's {:?} isn't written", input.slot)),
            }
        }
        // Each vertex in the draw's order: its index.
        let mut order = Vec::with_capacity(d.count as usize);
        for k in 0..d.count {
            order.push(match d.index {
                Some((range, ty)) => {
                    let size = if ty == IndexType::U16 { 2 } else { 4 };
                    let mut b = [0u8; 4];
                    let at = self.addresses[&range.memory] + range.offset + (d.first + k) as u64 * size;
                    self.read(at, &mut b[..size as usize])?;
                    let i = if ty == IndexType::U16 { u16::from_le_bytes([b[0], b[1]]) as u32 } else { u32::from_le_bytes(b) };
                    (i as i64 + d.vertex_offset as i64) as u32
                }
                None => d.first + k,
            });
        }
        for instance in d.first_instance..d.first_instance + d.instances {
            let verts = self.vertices(g, d, &vbound, &order, instance, position, &from)?;
            let mut tris = Vec::new();
            match g.topology {
                Topology::TriangleList => {
                    for k in (0..verts.len() / 3 * 3).step_by(3) {
                        tris.push([k, k + 1, k + 2]);
                    }
                }
                Topology::TriangleStrip => {
                    for k in 0..verts.len().saturating_sub(2) {
                        tris.push(if k % 2 == 0 { [k, k + 1, k + 2] } else { [k + 1, k, k + 2] });
                    }
                }
                Topology::TriangleFan => {
                    for k in 1..verts.len().saturating_sub(1) {
                        tris.push([k, k + 1, 0]);
                    }
                }
            }
            let mut batch = Batch::default();
            for (n, tri) in tris.iter().enumerate() {
                // Vulkan's provoking vertex: the first of the triangle as the topology numbers them.
                let provoking = match g.topology {
                    Topology::TriangleStrip => n,
                    Topology::TriangleFan => tri[0],
                    Topology::TriangleList => tri[0],
                };
                self.triangle(g, &fbound, t, [&verts[tri[0]], &verts[tri[1]], &verts[tri[2]]], &verts[provoking], &mut batch)?;
            }
            self.flush(g, &fbound, t, &mut batch)?;
        }
        Ok(())
    }

    /// Every vertex of one instance, shaded, in the draw's order.
    #[allow(clippy::too_many_arguments)]
    fn vertices(&mut self, g: &Graphics, d: &Draw, bound: &[Bound], order: &[u32], instance: u32, position: usize, from: &[usize]) -> Result<Vec<Vert>, Error> {
        let mut unique: Vec<u32> = order.to_vec();
        unique.sort_unstable();
        unique.dedup();
        let n_in = g.vertex.inputs.len();
        let n_out = g.vertex.outputs.len();
        let mut shaded: HashMap<u32, Vert> = HashMap::new();
        for chunk in unique.chunks(GROUP) {
            let mut inputs = Vec::with_capacity(chunk.len() * n_in);
            let mut builtins = Vec::with_capacity(chunk.len());
            for &v in chunk {
                for input in &g.vertex.inputs {
                    let Slot::Location(location) = input.slot else { continue };
                    let a = g.attributes.iter().find(|a| a.location == location).copied();
                    inputs.push(match a {
                        Some(a) => self.attribute(g, d, &a, v, instance)?,
                        None => [0; 4],
                    });
                }
                builtins.push(Builtins { vertex_index: v, instance_index: instance, ..Builtins::default() });
            }
            let helpers = vec![false; chunk.len()];
            let s = cpu::Shade { bindings: bound, subgroup_size: self.subgroup, inputs: &inputs, builtins: &builtins, helpers: &helpers };
            let out = self.shade(&g.vertex, &s)?;
            for (lane, &v) in chunk.iter().enumerate() {
                let o = &out.outputs[lane * n_out..(lane + 1) * n_out];
                let p = o.get(position).copied().unwrap_or([0; 4]);
                let clip = [0, 1, 2, 3].map(|i| float(p[i]) as f64);
                shaded.insert(v, Vert { clip, vary: from.iter().map(|&i| o[i]).collect() });
            }
        }
        Ok(order.iter().map(|v| shaded[v].clone()).collect())
    }

    /// Clips a triangle and rasterizes what's left, adding its fragments to the batch.
    fn triangle(&mut self, g: &Graphics, fbound: &[Bound], t: &Target, v: [&Vert; 3], provoking: &Vert, batch: &mut Batch) -> Result<(), Error> {
        let inputs = &g.fragment.inputs;
        let smooth = |i: usize| inputs[i].interpolation != Interpolation::Flat && inputs[i].ty.scalar() == Some(Scalar::F32);
        let mut poly: Vec<ClipVert> = v
            .iter()
            .map(|vert| ClipVert {
                p: vert.clip,
                v: (0..inputs.len()).map(|i| if smooth(i) { vert.vary[i].map(float) } else { [0.0; 4] }).collect(),
            })
            .collect();
        // Each plane as a distance that's non-negative inside.
        let planes: [fn(&[f64; 4]) -> f64; 7] = [
            |p| p[2],
            |p| p[3] - p[2],
            |p| GUARD * p[3] + p[0],
            |p| GUARD * p[3] - p[0],
            |p| GUARD * p[3] + p[1],
            |p| GUARD * p[3] - p[1],
            |p| p[3] - 1e-6,
        ];
        for plane in planes {
            if poly.iter().all(|c| plane(&c.p) >= 0.0) {
                continue;
            }
            poly = clip(&poly, plane);
            if poly.len() < 3 {
                return Ok(());
            }
        }
        let vp = t.viewport;
        let screen: Vec<Screen> = poly
            .iter()
            .map(|c| {
                let iw = 1.0 / c.p[3];
                let (x, y, z) = (c.p[0] * iw, c.p[1] * iw, c.p[2] * iw);
                Screen {
                    x: vp.x as f64 + (x + 1.0) * vp.width as f64 / 2.0,
                    y: vp.y as f64 + (y + 1.0) * vp.height as f64 / 2.0,
                    z: vp.min_depth as f64 + z * (vp.max_depth - vp.min_depth) as f64,
                    iw,
                    v: c.v.clone(),
                }
            })
            .collect();
        let flat: Vec<Lanes> = provoking.vary.clone();
        for k in 1..screen.len() - 1 {
            self.rasterize(g, fbound, t, [&screen[0], &screen[k], &screen[k + 1]], &flat, batch)?;
        }
        Ok(())
    }

    fn rasterize(&mut self, g: &Graphics, fbound: &[Bound], t: &Target, s: [&Screen; 3], flat: &[Lanes], batch: &mut Batch) -> Result<(), Error> {
        let fix = |v: f64| (v * SUBPIXEL).round() as i64;
        let mut p = s.map(|v| (fix(v.x), fix(v.y)));
        let mut s = s;
        let edge = |a: (i64, i64), b: (i64, i64), c: (i64, i64)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        let area = edge(p[0], p[1], p[2]);
        if area == 0 {
            return Ok(());
        }
        // Vulkan's area is -½ of this sum: counter-clockwise (in framebuffer coordinates) is a negative one here.
        let front_facing = match g.front {
            FrontFace::CounterClockwise => area < 0,
            FrontFace::Clockwise => area > 0,
        };
        if (g.cull == Cull::Back && !front_facing) || (g.cull == Cull::Front && front_facing) {
            return Ok(());
        }
        let area = if area < 0 {
            p.swap(1, 2);
            s.swap(1, 2);
            -area
        } else {
            area
        };
        // Each edge, opposite each vertex, and whether it's a top or left edge (whose pixel centres are inside).
        let edges = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
        let top_left = edges.map(|(a, b)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            (dy == 0 && dx > 0) || dy < 0
        });
        let first = t.colors.first().copied().or(t.depth).unwrap_or(0);
        let (w, h) = self.images[first as usize].as_ref().map_or((0, 0), |i| (i.desc.width, i.desc.height));
        let sx0 = t.scissor.x.min(w);
        let sy0 = t.scissor.y.min(h);
        let sx1 = (t.scissor.x + t.scissor.width).min(w);
        let sy1 = (t.scissor.y + t.scissor.height).min(h);
        let minx = s.iter().map(|v| v.x).fold(f64::INFINITY, f64::min).floor().max(sx0 as f64) as i64;
        let miny = s.iter().map(|v| v.y).fold(f64::INFINITY, f64::min).floor().max(sy0 as f64) as i64;
        let maxx = s.iter().map(|v| v.x).fold(f64::NEG_INFINITY, f64::max).ceil().min(sx1 as f64) as i64;
        let maxy = s.iter().map(|v| v.y).fold(f64::NEG_INFINITY, f64::max).ceil().min(sy1 as f64) as i64;
        if minx >= maxx || miny >= maxy {
            return Ok(());
        }
        let inputs = &g.fragment.inputs;
        let tri = Tri { area: area as f64, front_facing };
        let mut qy = miny & !1;
        while qy < maxy {
            let mut qx = minx & !1;
            while qx < maxx {
                let mut quad = [Frag { x: 0, y: 0, z: 0.0, covered: false }; 4];
                let mut any = false;
                let mut weights = [[0.0f64; 3]; 4];
                for (k, (dx, dy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().enumerate() {
                    let (x, y) = (qx + dx, qy + dy);
                    let c = (x * SUBPIXEL as i64 + SUBPIXEL as i64 / 2, y * SUBPIXEL as i64 + SUBPIXEL as i64 / 2);
                    let e = edges.map(|(a, b)| edge(a, b, c));
                    let inside = (0..3).all(|i| e[i] > 0 || (e[i] == 0 && top_left[i]));
                    let in_scissor = x >= sx0 as i64 && x < sx1 as i64 && y >= sy0 as i64 && y < sy1 as i64;
                    let covered = inside && in_scissor;
                    any |= covered;
                    weights[k] = e.map(|v| v as f64 / tri.area);
                    let z = weights[k][0] * s[0].z + weights[k][1] * s[1].z + weights[k][2] * s[2].z;
                    quad[k] = Frag { x: x.max(0) as u32, y: y.max(0) as u32, z: z as f32, covered };
                }
                if any {
                    for k in 0..4 {
                        let l = weights[k];
                        let iw = l[0] * s[0].iw + l[1] * s[1].iw + l[2] * s[2].iw;
                        for (i, input) in inputs.iter().enumerate() {
                            let value = match input.interpolation {
                                _ if input.ty.scalar() != Some(Scalar::F32) => flat[i],
                                Interpolation::Flat => flat[i],
                                Interpolation::Smooth => [0, 1, 2, 3].map(|c| {
                                    let num: f64 = (0..3).map(|j| l[j] * s[j].v[i][c] as f64 * s[j].iw).sum();
                                    bits((num / iw) as f32)
                                }),
                                Interpolation::NoPerspective => {
                                    [0, 1, 2, 3].map(|c| bits((0..3).map(|j| l[j] * s[j].v[i][c] as f64).sum::<f64>() as f32))
                                }
                            };
                            batch.inputs.push(value);
                        }
                        let f = quad[k];
                        batch.builtins.push(Builtins {
                            frag_coord: [f.x as f32 + 0.5, f.y as f32 + 0.5, f.z, iw as f32],
                            front_facing: tri.front_facing,
                            ..Builtins::default()
                        });
                        batch.frags.push(f);
                    }
                    if batch.frags.len() >= GROUP {
                        self.flush(g, fbound, t, batch)?;
                    }
                }
                qx += 2;
            }
            qy += 2;
        }
        Ok(())
    }

    /// Shades the batch's fragments and writes the covered ones that weren't discarded, in order.
    fn flush(&mut self, g: &Graphics, fbound: &[Bound], t: &Target, batch: &mut Batch) -> Result<(), Error> {
        if batch.frags.is_empty() {
            return Ok(());
        }
        let helpers: Vec<bool> = batch.frags.iter().map(|f| !f.covered).collect();
        let s = cpu::Shade { bindings: fbound, subgroup_size: self.subgroup, inputs: &batch.inputs, builtins: &batch.builtins, helpers: &helpers };
        let out = self.shade(&g.fragment, &s)?;
        let n_out = g.fragment.outputs.len();
        let depth_out = g.fragment.outputs.iter().position(|o| o.slot == Slot::FragDepth);
        let color_out: Vec<Option<usize>> =
            (0..g.colors.len()).map(|j| g.fragment.outputs.iter().position(|o| o.slot == Slot::Location(j as u32))).collect();
        for (lane, f) in batch.frags.iter().enumerate() {
            if !f.covered || out.discarded[lane] {
                continue;
            }
            let o = &out.outputs[lane * n_out..(lane + 1) * n_out];
            if let (Some(state), Some(depth)) = (g.depth, t.depth) {
                let z = depth_out.map_or(f.z, |i| float(o[i][0]));
                let image = self.images[depth as usize].as_mut().ok_or_else(|| Error { message: "no depth image".into() })?;
                let old = image.read(f.x, f.y)[0];
                if !state.compare.passes(z, old) {
                    continue;
                }
                if state.write {
                    image.write(f.x, f.y, [z, 0.0, 0.0, 1.0]);
                }
            }
            for (j, target) in g.colors.iter().enumerate() {
                let Some(i) = color_out[j] else { continue };
                let lanes = g.fragment.outputs[i].ty.lanes() as usize;
                let mut src = [0.0, 0.0, 0.0, 1.0];
                for (c, v) in src.iter_mut().enumerate().take(lanes) {
                    *v = float(o[i][c]);
                }
                let image = self.images[t.colors[j] as usize].as_mut().ok_or_else(|| Error { message: "no colour image".into() })?;
                let dst = image.read(f.x, f.y);
                let mut c = match target.blend {
                    Some(b) => blend(&b, src, dst),
                    None => src,
                };
                for k in 0..4 {
                    if !target.write[k] {
                        c[k] = dst[k];
                    }
                }
                image.write(f.x, f.y, c);
            }
        }
        *batch = Batch::default();
        Ok(())
    }
}

/// A polygon vertex in framebuffer coordinates: position, depth, 1/w, and the interpolated inputs.
struct Screen {
    x: f64,
    y: f64,
    z: f64,
    iw: f64,
    v: Vec<[f32; 4]>,
}

struct Tri {
    area: f64,
    front_facing: bool,
}

/// Fragments waiting to be shaded: in quads, with their inputs.
#[derive(Default)]
struct Batch {
    frags: Vec<Frag>,
    inputs: Vec<Lanes>,
    builtins: Vec<Builtins>,
}

/// Sutherland–Hodgman against one plane, in clip space: the polygon's part where `distance` is non-negative.
fn clip(poly: &[ClipVert], distance: fn(&[f64; 4]) -> f64) -> Vec<ClipVert> {
    let mut out = Vec::new();
    for i in 0..poly.len() {
        let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
        let (da, db) = (distance(&a.p), distance(&b.p));
        if da >= 0.0 {
            out.push(a.clone());
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let p = [0, 1, 2, 3].map(|k| a.p[k] + (b.p[k] - a.p[k]) * t);
            let v = a.v.iter().zip(&b.v).map(|(x, y)| [0, 1, 2, 3].map(|k| x[k] + (y[k] - x[k]) * t as f32)).collect();
            out.push(ClipVert { p, v });
        }
    }
    out
}

fn factor(f: Factor, src: [f32; 4], dst: [f32; 4], channel: usize) -> f32 {
    match f {
        Factor::Zero => 0.0,
        Factor::One => 1.0,
        Factor::SrcColor => src[channel],
        Factor::OneMinusSrcColor => 1.0 - src[channel],
        Factor::DstColor => dst[channel],
        Factor::OneMinusDstColor => 1.0 - dst[channel],
        Factor::SrcAlpha => src[3],
        Factor::OneMinusSrcAlpha => 1.0 - src[3],
        Factor::DstAlpha => dst[3],
        Factor::OneMinusDstAlpha => 1.0 - dst[3],
    }
}

fn blend(b: &Blend, src: [f32; 4], dst: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let (sf, df, op) = if c < 3 { b.color } else { b.alpha };
        let (s, d) = (src[c] * factor(sf, src, dst, c), dst[c] * factor(df, src, dst, c));
        *o = match op {
            BlendOp::Add => s + d,
            BlendOp::Subtract => s - d,
            BlendOp::ReverseSubtract => d - s,
            BlendOp::Min => src[c].min(dst[c]),
            BlendOp::Max => src[c].max(dst[c]),
        };
    }
    out
}
