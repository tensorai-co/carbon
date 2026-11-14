//! The suite's draw tests. Each picks its geometry so the answer is exact by Vulkan's rules (pixel centres off
//! edges, or on them where the fill rule decides), and computes it here.
//!
//! A device that has no draws yet reports these as unsupported, by what it said when asked for an image.

use gpu::ir::{Entry, Stage};
use gpu::{
    Address, Attribute, Bind, Blend, BlendOp, ColorAttachment, ColorTarget, Command, Compare, Cull, DepthAttachment,
    DepthState, Device, Draw, Factor, Filter, Format, FrontFace, GraphicsDesc, Image, ImageDesc, IndexType, Load, Rect, SamplerDesc, Sub, Topology, VertexBuffer, VertexFormat,
};

use super::{Ran, SHADERS, download, e, run_now, upload};

fn shader(name: &str, stage: Stage) -> Result<Entry, String> {
    let bytes = SHADERS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b).unwrap_or(&[]);
    let module = gpu::spirv(bytes).map_err(e)?;
    module.entries.into_iter().find(|en| en.stage == stage).ok_or_else(|| format!("{name} has no {stage:?} entry point"))
}

/// An image, or why the device has none: a device without draws says so here.
fn image(d: &mut Device, format: Format, width: u32, height: u32) -> Result<Result<Image, String>, String> {
    match d.image(ImageDesc::new(format, width, height)) {
        Ok(i) => Ok(Ok(i)),
        Err(why) if why.message.contains("aren't on") => Ok(Err(why.message)),
        Err(why) => Err(why.message),
    }
}

fn floats(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// An image's texels, as words.
fn read_image(d: &mut Device, image: Image, width: u32, height: u32, bpp: u32) -> Result<Vec<u32>, String> {
    let out = upload(d, &vec![0; (width * height * bpp / 4) as usize])?;
    let region = Rect { x: 0, y: 0, width, height };
    run_now(d, &[Command::CopyImageToBuffer { from: image, sub: Sub::default(), region, to: out, row_pixels: 0 }])?;
    download(d, out)
}

const ALL: [bool; 4] = [true; 4];
const ADD: Blend = Blend { color: (Factor::One, Factor::One, BlendOp::Add), alpha: (Factor::One, Factor::One, BlendOp::Add) };

/// The fill rule, flat interpolation and discard: two triangles that share a diagonal on a 16×16 target, each
/// adding its own colour, so a pixel drawn twice or not at all shows; then a triangle whose fragments all discard.
pub fn triangles(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 16;
    let target = match image(d, Format::Rgba32Float, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let flat = image(d, Format::Rgba32Float, N, N)??;
    let (vs, fs) = (shader("colors.vert", Stage::Vertex)?, shader("colors.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 32, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(4) },
        Attribute { location: 1, buffer: 0, offset: 16, format: VertexFormat::F32(4) },
    ];
    let colors = [ColorTarget { format: Format::Rgba32Float, blend: Some(ADD), write: ALL }; 2];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleList,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &colors,
        })
        .map_err(e)?;
    // Pixel coordinates to NDC: x / 8 - 1. The first triangle is the upper-left half, the second the lower-right;
    // their shared diagonal runs through pixel centres where x + y = 15.
    let ndc = |x: f32, y: f32| [x / 8.0 - 1.0, y / 8.0 - 1.0, 0.0, 1.0];
    let vertex = |p: [f32; 4], c: [f32; 4]| [p, c].concat();
    let (red, green, blue, hot) = ([0.25, 0.0, 0.0, 0.0], [0.0, 0.5, 0.0, 0.0], [0.0, 0.0, 0.75, 0.0], [1.0, 0.0, 0.0, 0.0]);
    let mut v: Vec<f32> = Vec::new();
    v.extend(vertex(ndc(0.0, 0.0), red));
    v.extend(vertex(ndc(16.0, 0.0), green));
    v.extend(vertex(ndc(0.0, 16.0), blue));
    v.extend(vertex(ndc(16.0, 0.0), green));
    v.extend(vertex(ndc(16.0, 16.0), green));
    v.extend(vertex(ndc(0.0, 16.0), green));
    for p in [ndc(0.0, 0.0), ndc(16.0, 0.0), ndc(0.0, 16.0)] {
        v.extend(vertex(p, hot));
    }
    let vb = upload(d, &floats(&v))?;
    let clear = Load::Clear([0.0; 4]);
    let draw = Draw {
        pipeline: p,
        bindings: Vec::new(),
        vertex_buffers: vec![vb],
        index: None,
        first: 0,
        count: 9,
        vertex_offset: 0,
        first_instance: 0,
        instances: 1,
    };
    run_now(
        d,
        &[
            Command::BeginRender { colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: clear }, ColorAttachment { image: flat, sub: Sub::default(), load: clear }], depth: None },
            Command::Draw(draw),
            Command::EndRender,
        ],
    )?;
    let smooth = read_image(d, target, N, N, 16)?;
    let flats = read_image(d, flat, N, N, 16)?;
    let mut wrong = Vec::new();
    for y in 0..N {
        for x in 0..N {
            let at = ((y * N + x) * 4) as usize;
            let got_flat = [0, 1, 2, 3].map(|c| f32::from_bits(flats[at + c]));
            let got = [0, 1, 2, 3].map(|c| f32::from_bits(smooth[at + c]));
            let upper = x + y <= 14;
            let want_flat = if upper { red } else { green };
            if got_flat != want_flat {
                wrong.push(format!("({x}, {y}) flat {got_flat:?}, wanted {want_flat:?}"));
            }
            // The smooth colour: barycentric in the upper triangle (red at 0,0, green at 16,0, blue at 0,16).
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let want = if upper {
                [0.25 * (1.0 - cx / 16.0 - cy / 16.0), 0.5 * cx / 16.0, 0.75 * cy / 16.0, 0.0]
            } else {
                green
            };
            if got.iter().zip(&want).any(|(g, w)| (g - w).abs() > 1e-5) {
                wrong.push(format!("({x}, {y}) {got:?}, wanted {want:?}"));
            }
        }
    }
    if wrong.is_empty() { Ok(Ran::Passed) } else { Err(format!("{} wrong:\n    {}", wrong.len(), wrong.join("\n    "))) }
}

/// The depth test: three indexed quads over the whole target at depths 0.5, 0.25 and 0.75, with `Less`: the middle
/// one is seen, and the depth image holds 0.25.
pub fn depth(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 8;
    let target = match image(d, Format::Rgba8Unorm, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let flat = image(d, Format::Rgba8Unorm, N, N)??;
    let zbuf = image(d, Format::D32Float, N, N)??;
    let (vs, fs) = (shader("colors.vert", Stage::Vertex)?, shader("colors.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 32, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(4) },
        Attribute { location: 1, buffer: 0, offset: 16, format: VertexFormat::F32(4) },
    ];
    let colors = [ColorTarget { format: Format::Rgba8Unorm, blend: None, write: ALL }; 2];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleList,
            cull: Cull::Back,
            front: FrontFace::Clockwise,
            depth: Some(DepthState { compare: Compare::Less, write: true }),
            depth_format: Some(Format::D32Float),
            colors: &colors,
        })
        .map_err(e)?;
    let mut v: Vec<f32> = Vec::new();
    for (z, c) in [(0.5, [0.0, 1.0, 0.0, 1.0]), (0.25, [0.0, 0.0, 1.0, 1.0]), (0.75, [0.5, 0.5, 0.0, 1.0])] {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            v.extend([x, y, z, 1.0]);
            v.extend(c);
        }
    }
    let vb = upload(d, &floats(&v))?;
    // Two triangles a quad, clockwise in framebuffer coordinates (front-facing here, so back-face culling keeps them),
    // as 16-bit indices, the quads by the vertex offset.
    let indices: Vec<u32> = vec![0x0001_0000, 0x0000_0002, 0x0003_0002];
    let ib = upload(d, &indices)?;
    let mut commands = vec![Command::BeginRender {
        colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: Load::Clear([0.0; 4]) }, ColorAttachment { image: flat, sub: Sub::default(), load: Load::DontCare }],
        depth: Some(DepthAttachment { image: zbuf, sub: Sub::default(), load: Load::Clear(1.0) }),
    }];
    for quad in 0..3 {
        commands.push(Command::Draw(Draw {
            pipeline: p,
            bindings: Vec::new(),
            vertex_buffers: vec![vb],
            index: Some((ib, IndexType::U16)),
            first: 0,
            count: 6,
            vertex_offset: quad * 4,
            first_instance: 0,
            instances: 1,
        }));
    }
    commands.push(Command::EndRender);
    run_now(d, &commands)?;
    let got = read_image(d, target, N, N, 4)?;
    let z = read_image(d, zbuf, N, N, 4)?;
    let blue = u32::from_le_bytes([0, 0, 255, 255]);
    if let Some(i) = got.iter().position(|&w| w != blue) {
        return Err(format!("pixel {i} is {:#010x}, wanted {blue:#010x}", got[i]));
    }
    if let Some(i) = z.iter().position(|&w| f32::from_bits(w) != 0.25) {
        return Err(format!("depth {i} is {}, wanted 0.25", f32::from_bits(z[i])));
    }
    Ok(Ran::Passed)
}

/// Texturing: a 4×4 texture of distinct texels over an 8×8 target, two pixels a texel, read with nearest filtering
/// through separate image and sampler bindings; the coordinates' derivatives and the fragment's position checked
/// in a second attachment.
pub fn texture(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 8;
    let target = match image(d, Format::Rgba8Unorm, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let info = image(d, Format::Rgba32Float, N, N)??;
    let tex = image(d, Format::Rgba8Unorm, 4, 4)??;
    let texels: Vec<u32> = (0..16u32).map(|i| u32::from_le_bytes([(i * 16) as u8, (255 - i * 16) as u8, (i * 3) as u8, 255])).collect();
    let staging = upload(d, &texels)?;
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 4, height: 4 } }])?;
    let sampler = d.sampler(SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::Repeat; 2])).map_err(e)?;
    let (vs, fs) = (shader("textured.vert", Stage::Vertex)?, shader("textured.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 16, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(2) },
        Attribute { location: 1, buffer: 0, offset: 8, format: VertexFormat::F32(2) },
    ];
    let colors = [
        ColorTarget { format: Format::Rgba8Unorm, blend: None, write: ALL },
        ColorTarget { format: Format::Rgba32Float, blend: None, write: ALL },
    ];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleStrip,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &colors,
        })
        .map_err(e)?;
    // A strip over the whole target; u and v run 0 to 1 across it.
    let v = floats(&[-1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let vb = upload(d, &v)?;
    run_now(
        d,
        &[
            Command::BeginRender {
                colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: Load::DontCare }, ColorAttachment { image: info, sub: Sub::default(), load: Load::DontCare }],
                depth: None,
            },
            Command::Draw(Draw {
                pipeline: p,
                bindings: vec![(0, 0, Bind::Image(tex)), (0, 1, Bind::Sampler(sampler))],
                vertex_buffers: vec![vb],
                index: None,
                first: 0,
                count: 4,
                vertex_offset: 0,
                first_instance: 0,
                instances: 1,
            }),
            Command::EndRender,
        ],
    )?;
    let got = read_image(d, target, N, N, 4)?;
    let infos = read_image(d, info, N, N, 16)?;
    for y in 0..N {
        for x in 0..N {
            let want = texels[((y / 2) * 4 + x / 2) as usize];
            let at = (y * N + x) as usize;
            if got[at] != want {
                return Err(format!("({x}, {y}) is {:#010x}, wanted texel ({}, {}) {want:#010x}", got[at], x / 2, y / 2));
            }
            let i = [0, 1, 2, 3].map(|c| f32::from_bits(infos[at * 4 + c]));
            // u and v change by 1/8 a pixel: times 256, 32.
            let want = [32.0, 32.0, (x as f32 + 0.5) / 256.0, (y as f32 + 0.5) / 256.0];
            if i != want {
                return Err(format!("({x}, {y})'s derivatives and position {i:?}, wanted {want:?}"));
            }
        }
    }
    Ok(Ran::Passed)
}

/// vkcube's own shaders, one frame: its uniform buffer (an MVP matrix, then the 36 vertices' positions and texture
/// coordinates), a white texture, and an orthographic view straight at the cube's z = -1 face. That face covers the
/// middle half of the target, its normal from the derivatives of the position is +z, and so every pixel there is
/// the light's 0.707 times white; the rest is the clear colour.
pub fn cube(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 32;
    let target = match image(d, Format::Rgba8Unorm, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let zbuf = image(d, Format::D16Unorm, N, N)??;
    let tex = image(d, Format::Rgba8Unorm, 2, 2)??;
    let staging = upload(d, &[u32::MAX; 4])?;
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 2, height: 2 } }])?;
    let sampler = d.sampler(SamplerDesc::new(Filter::Linear, Filter::Linear, [Address::ClampToEdge; 2])).map_err(e)?;
    let (vs, fs) = (shader("cube.vert", Stage::Vertex)?, shader("cube.frag", Stage::Fragment)?);
    let colors = [ColorTarget { format: Format::Rgba8Unorm, blend: None, write: ALL }];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &[],
            attributes: &[],
            topology: Topology::TriangleList,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: Some(DepthState { compare: Compare::Less, write: true }),
            depth_format: Some(Format::D16Unorm),
            colors: &colors,
        })
        .map_err(e)?;
    // The MVP, column-major: x and y halved, z from [-1, 1] to [0.25, 0.75].
    let mvp = [0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.5, 1.0];
    // The cube's six faces, two triangles each.
    let mut positions: Vec<[f32; 3]> = Vec::new();
    for axis in 0..3 {
        for side in [-1.0f32, 1.0] {
            let corner = |a: f32, b: f32| {
                let mut p = [0.0; 3];
                p[axis] = side;
                p[(axis + 1) % 3] = a;
                p[(axis + 2) % 3] = b;
                p
            };
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(corner(a, b));
            }
        }
    }
    let mut ubo: Vec<f32> = mvp.to_vec();
    for p in &positions {
        ubo.extend([p[0], p[1], p[2], 1.0]);
    }
    for p in &positions {
        ubo.extend([(p[0] + 1.0) / 2.0, (p[1] + 1.0) / 2.0, 0.0, 0.0]);
    }
    let ub = upload(d, &floats(&ubo))?;
    let clear = [0.0, 0.0, 0.25, 1.0];
    run_now(
        d,
        &[
            Command::BeginRender {
                colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: Load::Clear(clear) }],
                depth: Some(DepthAttachment { image: zbuf, sub: Sub::default(), load: Load::Clear(1.0) }),
            },
            Command::Draw(Draw {
                pipeline: p,
                bindings: vec![(0, 0, Bind::Buffer(ub)), (0, 1, Bind::Combined(tex, sampler))],
                vertex_buffers: Vec::new(),
                index: None,
                first: 0,
                count: 36,
                vertex_offset: 0,
                first_instance: 0,
                instances: 1,
            }),
            Command::EndRender,
        ],
    )?;
    let got = read_image(d, target, N, N, 4)?;
    let lit = u32::from_le_bytes([180; 4]);
    let background = u32::from_le_bytes([0, 0, 64, 255]);
    for y in 0..N {
        for x in 0..N {
            let inside = (8..24).contains(&x) && (8..24).contains(&y);
            let want = if inside { lit } else { background };
            let at = (y * N + x) as usize;
            if got[at] != want {
                return Err(format!("({x}, {y}) is {:#010x}, wanted {want:#010x}", got[at]));
            }
        }
    }
    Ok(Ran::Passed)
}

/// One level of one layer of an image, as bytes.
fn read_sub(d: &mut Device, image: Image, sub: Sub, width: u32, height: u32, bpp: u32) -> Result<Vec<u8>, String> {
    let len = width * height * bpp;
    let out = upload(d, &vec![0; len.div_ceil(4) as usize])?;
    let region = Rect { x: 0, y: 0, width, height };
    run_now(d, &[Command::CopyImageToBuffer { from: image, sub, region, to: out, row_pixels: 0 }])?;
    let words = download(d, out)?;
    Ok(words.iter().flat_map(|w| w.to_le_bytes()).take(len as usize).collect())
}

/// Bytes into one level of one layer of an image.
fn write_sub(d: &mut Device, image: Image, sub: Sub, width: u32, height: u32, bytes: &[u8]) -> Result<(), String> {
    let words: Vec<u32> = bytes.chunks(4).map(|c| { let mut w = [0u8; 4]; w[..c.len()].copy_from_slice(c); u32::from_le_bytes(w) }).collect();
    let staging = upload(d, &words)?;
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: image, sub, region: Rect { x: 0, y: 0, width, height } }])
}

/// Formats and subresources: for each colour format, a flat colour drawn into level 1 of layer 1 of a 4×4 image of
/// two levels and two layers, and cleared into level 0 of layer 1, each texel's bytes exact (colours each format holds
/// exactly, or a unorm's away from a tie); layer 0 keeps what was copied there. An alpha-only format can't be drawn
/// into, on any device.
pub fn formats(d: &mut Device) -> Result<Ran, String> {
    if let Err(why) = image(d, Format::R8Unorm, 1, 1)? {
        return Ok(Ran::Unsupported(why));
    }
    let unorm = [0.2, 0.6, 1.0, 1.0];
    let float = [0.25, 0.75, 2.0, 1.0];
    let word = |w: u32| w.to_le_bytes().to_vec();
    let half = |h: [u16; 4]| h.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let cases: [(Format, [f32; 4], Vec<u8>); 6] = [
        (Format::R8Unorm, unorm, vec![51]),
        (Format::Rg8Unorm, unorm, vec![51, 153]),
        (Format::Rgb10a2Unorm, unorm, word(205 | 614 << 10 | 1023 << 20 | 3 << 30)),
        (Format::R32Float, float, word(0.25f32.to_bits())),
        (Format::Rgba16Float, float, half([0x3400, 0x3a00, 0x4000, 0x3c00])),
        (Format::Rg11b10Float, float, word(832 | 928 << 11 | 512 << 22)),
    ];
    let (vs, fs) = (shader("colors.vert", Stage::Vertex)?, shader("colors.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 32, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(4) },
        Attribute { location: 1, buffer: 0, offset: 16, format: VertexFormat::F32(4) },
    ];
    let desc = |colors: &'static [ColorTarget]| GraphicsDesc {
        vertex: &vs,
        fragment: &fs,
        buffers: &buffers,
        attributes: &attributes,
        topology: Topology::TriangleStrip,
        cull: Cull::None,
        front: FrontFace::CounterClockwise,
        depth: None,
        depth_format: None,
        colors,
    };
    const ALPHA: &[ColorTarget] = &[ColorTarget { format: Format::A8Unorm, blend: None, write: ALL }; 2];
    match d.graphics(&desc(ALPHA)) {
        Err(why) if why.message.contains("can't be drawn into") => {}
        other => return Err(format!("a pipeline drawing into A8Unorm: {other:?}")),
    }
    let mut wrong = Vec::new();
    for (format, color, want) in cases {
        let colors: &'static [ColorTarget] = Box::leak(Box::new([ColorTarget { format, blend: None, write: ALL }; 2]));
        let p = d.graphics(&desc(colors)).map_err(e)?;
        let bpp = format.bytes();
        let target = d.image(ImageDesc { format, width: 4, height: 4, levels: 2, layers: 2 }).map_err(e)?;
        let smooth = image(d, format, 2, 2)??;
        let pattern: Vec<u8> = (0..4 * bpp).map(|i| (i * 7 + 1) as u8).collect();
        for layer in 0..2 {
            write_sub(d, target, Sub { level: 1, layer }, 2, 2, &pattern)?;
        }
        let mut v: Vec<f32> = Vec::new();
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            v.extend([x, y, 0.0, 1.0]);
            v.extend(color);
        }
        let vb = upload(d, &floats(&v))?;
        let draw = Draw { pipeline: p, bindings: Vec::new(), vertex_buffers: vec![vb], index: None, first: 0, count: 4, vertex_offset: 0, first_instance: 0, instances: 1 };
        let at = |level, layer, load| ColorAttachment { image: target, sub: Sub { level, layer }, load };
        let other = ColorAttachment { image: smooth, sub: Sub::default(), load: Load::DontCare };
        run_now(d, &[Command::BeginRender { colors: vec![other, at(1, 1, Load::Load)], depth: None }, Command::Draw(draw), Command::EndRender])?;
        let big = image(d, format, 4, 4)??;
        let other = ColorAttachment { image: big, sub: Sub::default(), load: Load::DontCare };
        run_now(d, &[Command::BeginRender { colors: vec![other, at(0, 1, Load::Clear(color))], depth: None }, Command::EndRender])?;
        let drawn = read_sub(d, target, Sub { level: 1, layer: 1 }, 2, 2, bpp)?;
        let cleared = read_sub(d, target, Sub { level: 0, layer: 1 }, 4, 4, bpp)?;
        let kept = read_sub(d, target, Sub { level: 1, layer: 0 }, 2, 2, bpp)?;
        for (what, got, texels) in [("drawn", &drawn, 4), ("cleared", &cleared, 16)] {
            if *got != want.repeat(texels) {
                wrong.push(format!("{format:?} {what}: {got:02x?}, wanted {:02x?} each texel", want));
            }
        }
        if kept != pattern {
            wrong.push(format!("{format:?}: layer 0 is {kept:02x?}, wanted {pattern:02x?}"));
        }
        for i in [target, smooth, big] {
            d.free_image(i);
        }
    }
    if wrong.is_empty() { Ok(Ran::Passed) } else { Err(wrong.join("\n    ")) }
}

/// The mip levels a sample reads, each with its weight.
type Reads = &'static [(usize, f32)];

/// A texel of level 0 of the mip test's 8×8 texture: bits of x and y at each scale, so every level's 2×2 averages
/// are whole numbers.
fn mip_texel(x: u32, y: u32) -> f32 {
    let bit = |v: u32, b: u32| (v >> b) & 1;
    (64 * bit(x, 0) + 32 * bit(y, 0) + 16 * bit(x, 1) + 8 * bit(y, 1) + 4 * bit(x, 2) + 2 * bit(y, 2)) as f32
}

/// Mip levels: an 8×8 half-float texture's levels made by `GenerateMips` and read back exact, then sampled at an
/// explicit level of detail, with the nearest level, the two around it blended, and the sampler's range keeping a
/// level of detail of 3 to level 1.
pub fn mips(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 4;
    let target = match image(d, Format::Rgba32Float, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let tex = d.image(ImageDesc { format: Format::Rgba16Float, width: 8, height: 8, levels: 4, layers: 1 }).map_err(e)?;
    // Each level's texels, red and green, computed here: level 0's, then each 2×2's average.
    let mut levels: Vec<Vec<[f32; 2]>> = vec![(0..64).map(|i| { let v = mip_texel(i % 8, i / 8); [v, 255.0 - v] }).collect()];
    for l in 1..4u32 {
        let (w, p) = (8 >> l, &levels[l as usize - 1]);
        let pw = 2 * w;
        let next = (0..w * w)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let at = |dx: u32, dy: u32| p[((2 * y + dy) * pw + 2 * x + dx) as usize];
                let s = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                [0, 1].map(|c| s.iter().map(|t| t[c]).sum::<f32>() / 4.0)
            })
            .collect();
        levels.push(next);
    }
    let half = |x: f32| -> u16 {
        // Whole numbers below 2048 as halves: exact.
        if x == 0.0 { return 0; }
        let e = x.log2().floor() as u32;
        ((e + 15) << 10 | ((x / (1 << e) as f32 - 1.0) * 1024.0) as u32) as u16
    };
    let texel_bytes = |t: [f32; 2]| [half(t[0]), half(t[1]), 0, half(1.0)].iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>();
    let level0: Vec<u8> = levels[0].iter().flat_map(|t| texel_bytes(*t)).collect();
    write_sub(d, tex, Sub::default(), 8, 8, &level0)?;
    run_now(d, &[Command::GenerateMips(tex)])?;
    let mut wrong = Vec::new();
    for l in 1..4u32 {
        let w = 8 >> l;
        let got = read_sub(d, tex, Sub { level: l, layer: 0 }, w, w, 8)?;
        let want: Vec<u8> = levels[l as usize].iter().flat_map(|t| texel_bytes(*t)).collect();
        if got != want {
            wrong.push(format!("level {l}: {got:02x?}, wanted {want:02x?}"));
        }
    }
    let (vs, fs) = (shader("textured.vert", Stage::Vertex)?, shader("levels.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 16, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(2) },
        Attribute { location: 1, buffer: 0, offset: 8, format: VertexFormat::F32(2) },
    ];
    let colors = [ColorTarget { format: Format::Rgba32Float, blend: None, write: ALL }];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleStrip,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &colors,
        })
        .map_err(e)?;
    let vb = upload(d, &floats(&[-1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]))?;
    let at = |l: usize, x: u32, y: u32| {
        // Pixel (x, y) of the 4×4 target is texel (x, y) scaled to the level's size.
        let w = 8 >> l;
        let (tx, ty) = (x * w / N, y * w / N);
        levels[l][(ty * w + tx) as usize]
    };
    // Each case's sampler, level of detail, and the levels it reads with their weights.
    let nearest = SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToEdge; 2]);
    let mut linear = nearest;
    linear.mipmap = Filter::Linear;
    let mut clamped = nearest;
    clamped.lod = [0.0, 1.0];
    let cases: [(&str, SamplerDesc, f32, Reads); 3] = [
        ("level 2", nearest, 2.0, &[(2, 1.0)]),
        ("between levels 1 and 2", linear, 1.5, &[(1, 0.5), (2, 0.5)]),
        ("kept to level 1", clamped, 3.0, &[(1, 1.0)]),
    ];
    let want = |reads: &[(usize, f32)], x: u32, y: u32| [0, 1].map(|c| reads.iter().map(|&(l, w)| at(l, x, y)[c] * w).sum::<f32>());
    for (what, desc, lod, reads) in cases {
        let sampler = d.sampler(desc).map_err(e)?;
        let ub = upload(d, &[lod.to_bits(), 0, 0, 0])?;
        run_now(
            d,
            &[
                Command::BeginRender { colors: vec![ColorAttachment { image: target, sub: Sub::default(), load: Load::DontCare }], depth: None },
                Command::Draw(Draw {
                    pipeline: p,
                    bindings: vec![(0, 0, Bind::Image(tex)), (0, 1, Bind::Sampler(sampler)), (0, 2, Bind::Buffer(ub))],
                    vertex_buffers: vec![vb],
                    index: None,
                    first: 0,
                    count: 4,
                    vertex_offset: 0,
                    first_instance: 0,
                    instances: 1,
                }),
                Command::EndRender,
            ],
        )?;
        let got = read_image(d, target, N, N, 16)?;
        for y in 0..N {
            for x in 0..N {
                let at = ((y * N + x) * 4) as usize;
                let g = [f32::from_bits(got[at]), f32::from_bits(got[at + 1])];
                let w = want(reads, x, y);
                if g != w {
                    wrong.push(format!("{what}: ({x}, {y}) is {g:?}, wanted {w:?}"));
                }
            }
        }
    }
    if wrong.is_empty() { Ok(Ran::Passed) } else { Err(format!("{} wrong:\n    {}", wrong.len(), wrong.join("\n    "))) }
}

/// The border colour: a 2×2 texture over the middle half of an 8×8 target, its coordinates running from -0.5 to 1.5,
/// read with `ClampToBorder` and opaque white: the texels inside, white around them.
pub fn border(d: &mut Device) -> Result<Ran, String> {
    const N: u32 = 8;
    let target = match image(d, Format::Rgba8Unorm, N, N)? {
        Ok(i) => i,
        Err(why) => return Ok(Ran::Unsupported(why)),
    };
    let info = image(d, Format::Rgba32Float, N, N)??;
    let tex = image(d, Format::Rgba8Unorm, 2, 2)??;
    let texels = [0xff00_00ffu32, 0xff00_ff00, 0xffff_0000, 0xff80_8080];
    let staging = upload(d, &texels)?;
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 2, height: 2 } }])?;
    let mut desc = SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToBorder; 2]);
    desc.border = gpu::Border::OpaqueWhite;
    let sampler = d.sampler(desc).map_err(e)?;
    let (vs, fs) = (shader("textured.vert", Stage::Vertex)?, shader("textured.frag", Stage::Fragment)?);
    let buffers = [VertexBuffer { stride: 16, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(2) },
        Attribute { location: 1, buffer: 0, offset: 8, format: VertexFormat::F32(2) },
    ];
    let colors = [
        ColorTarget { format: Format::Rgba8Unorm, blend: None, write: ALL },
        ColorTarget { format: Format::Rgba32Float, blend: None, write: ALL },
    ];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs,
            fragment: &fs,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleStrip,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &colors,
        })
        .map_err(e)?;
    let v = floats(&[-1.0, -1.0, -0.5, -0.5, 1.0, -1.0, 1.5, -0.5, -1.0, 1.0, -0.5, 1.5, 1.0, 1.0, 1.5, 1.5]);
    let vb = upload(d, &v)?;
    run_now(
        d,
        &[
            Command::BeginRender {
                colors: vec![
                    ColorAttachment { image: target, sub: Sub::default(), load: Load::DontCare },
                    ColorAttachment { image: info, sub: Sub::default(), load: Load::DontCare },
                ],
                depth: None,
            },
            Command::Draw(Draw {
                pipeline: p,
                bindings: vec![(0, 0, Bind::Image(tex)), (0, 1, Bind::Sampler(sampler))],
                vertex_buffers: vec![vb],
                index: None,
                first: 0,
                count: 4,
                vertex_offset: 0,
                first_instance: 0,
                instances: 1,
            }),
            Command::EndRender,
        ],
    )?;
    let got = read_image(d, target, N, N, 4)?;
    for y in 0..N {
        for x in 0..N {
            let inside = |c: u32| (2..6).contains(&c);
            let want = if inside(x) && inside(y) { texels[(((y - 2) / 2) * 2 + (x - 2) / 2) as usize] } else { u32::MAX };
            let at = (y * N + x) as usize;
            if got[at] != want {
                return Err(format!("({x}, {y}) is {:#010x}, wanted {want:#010x}", got[at]));
            }
        }
    }
    Ok(Ran::Passed)
}
