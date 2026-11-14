//! The suite's draw tests. Each picks its geometry so the answer is exact by Vulkan's rules (pixel centres off
//! edges, or on them where the fill rule decides), and computes it here.
//!
//! A device that has no draws yet reports these as unsupported, by what it said when asked for an image.

use gpu::ir::{Entry, Stage};
use gpu::{
    Address, Attribute, Bind, Blend, BlendOp, ColorAttachment, ColorTarget, Command, Compare, Cull, DepthAttachment,
    DepthState, Device, Draw, Factor, Filter, Format, FrontFace, GraphicsDesc, Image, ImageDesc, IndexType, Load, Rect, SamplerDesc, Topology, VertexBuffer, VertexFormat,
};

use super::{Ran, SHADERS, download, e, run_now, upload};

fn shader(name: &str, stage: Stage) -> Result<Entry, String> {
    let bytes = SHADERS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b).unwrap_or(&[]);
    let module = gpu::spirv(bytes).map_err(e)?;
    module.entries.into_iter().find(|en| en.stage == stage).ok_or_else(|| format!("{name} has no {stage:?} entry point"))
}

/// An image, or why the device has none: a device without draws says so here.
fn image(d: &mut Device, format: Format, width: u32, height: u32) -> Result<Result<Image, String>, String> {
    match d.image(ImageDesc { format, width, height }) {
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
    run_now(d, &[Command::CopyImageToBuffer { from: image, region, to: out, row_pixels: 0 }])?;
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
            Command::BeginRender { colors: vec![ColorAttachment { image: target, load: clear }, ColorAttachment { image: flat, load: clear }], depth: None },
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
        colors: vec![ColorAttachment { image: target, load: Load::Clear([0.0; 4]) }, ColorAttachment { image: flat, load: Load::DontCare }],
        depth: Some(DepthAttachment { image: zbuf, load: Load::Clear(1.0) }),
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
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, region: Rect { x: 0, y: 0, width: 4, height: 4 } }])?;
    let sampler = d.sampler(SamplerDesc { mag: Filter::Nearest, min: Filter::Nearest, address: [Address::Repeat; 2] }).map_err(e)?;
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
                colors: vec![ColorAttachment { image: target, load: Load::DontCare }, ColorAttachment { image: info, load: Load::DontCare }],
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
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, region: Rect { x: 0, y: 0, width: 2, height: 2 } }])?;
    let sampler = d.sampler(SamplerDesc { mag: Filter::Linear, min: Filter::Linear, address: [Address::ClampToEdge; 2] }).map_err(e)?;
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
                colors: vec![ColorAttachment { image: target, load: Load::Clear(clear) }],
                depth: Some(DepthAttachment { image: zbuf, load: Load::Clear(1.0) }),
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
