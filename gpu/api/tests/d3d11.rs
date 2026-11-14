//! `gpu_api::d3d11` on the CPU device and on the host's GPU: draws, depth, constant buffers changed between draws,
//! copies, maps, compute and mip levels, through Direct3D 11's calls, with HLSL compiled by vkd3d's compiler
//! (`gpu/examples/programs`).

use gpu_api::Gpu;
use gpu_api::d3d11::{self, BufferDesc, Data, Device, InputElement, RasterizerDesc, Region, SamplerDesc, SrvDesc, Stage, Texture2dDesc, Viewport, bind, format, map, misc, usage};

const QUAD_VS: &[u8] = include_bytes!("../../examples/programs/quad.vs.dxbc");
const QUAD_PS: &[u8] = include_bytes!("../../examples/programs/quad.ps.dxbc");
const COLOR_VS: &[u8] = include_bytes!("../../examples/programs/color.vs.dxbc");
const COLOR_PS: &[u8] = include_bytes!("../../examples/programs/color.ps.dxbc");
const ARITH: &[u8] = include_bytes!("../../examples/programs/arith.dxbc");

/// Each test on every device this host has.
fn each(test: fn(&mut Device) -> Result<(), String>) {
    for choice in [gpu::Choice::Software, gpu::Choice::Host] {
        let Ok(Some(gpu)) = Gpu::open(choice) else {
            eprintln!("{choice:?}: no device here, skipped");
            continue;
        };
        let name = gpu.info().describe();
        let mut d = Device::new(gpu);
        if let Err(why) = test(&mut d) {
            panic!("{name}: {why}");
        }
    }
}

fn e(err: d3d11::Error) -> String {
    format!("{:#x}: {}", err.hresult, err.message)
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn words(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

fn texture(d: &mut Device, width: u32, height: u32, fmt: u32, bind_flags: u32, data: &[Data<'_>]) -> Result<u64, String> {
    let desc = Texture2dDesc { width, height, mip_levels: 1, array_size: 1, format: fmt, sample_count: 1, usage: usage::DEFAULT, bind: bind_flags, cpu_access: 0, misc: 0 };
    d.create_texture2d(desc, data).map_err(e)
}

fn buffer(d: &mut Device, bytes: &[u8], bind_flags: u32, use_: u32) -> Result<u64, String> {
    let desc = BufferDesc { size: bytes.len() as u32, usage: use_, bind: bind_flags, cpu_access: 0, misc: 0, stride: 0 };
    d.create_buffer(desc, Some(bytes)).map_err(e)
}

fn element(semantic: &str, fmt: u32, offset: u32) -> InputElement {
    InputElement { semantic: semantic.into(), index: 0, format: fmt, slot: 0, offset, per_instance: false, step_rate: 0 }
}

/// A textured quad from HLSL: the texture's rows upside down from the suite's (Direct3D's y is up), the pixel's
/// position in pixels from the top left, a derivative.
#[test]
fn textured_quad() {
    each(|d| {
        const N: u32 = 8;
        let target = texture(d, N, N, format::R8G8B8A8_UNORM, bind::RENDER_TARGET, &[])?;
        let info = texture(d, N, N, format::R32G32B32A32_FLOAT, bind::RENDER_TARGET, &[])?;
        let texels: Vec<u32> = (0..16u32).map(|i| u32::from_le_bytes([(i * 16) as u8, (255 - i * 16) as u8, (i * 5) as u8, 255])).collect();
        let bytes: Vec<u8> = texels.iter().flat_map(|t| t.to_le_bytes()).collect();
        let tex = texture(d, 4, 4, format::R8G8B8A8_UNORM, bind::SHADER_RESOURCE, &[Data { bytes: &bytes, row_pitch: 16 }])?;
        let srv = d.create_srv(tex, None).map_err(e)?;
        let point = SamplerDesc { filter: 0, ..SamplerDesc::default() };
        let sampler = d.create_sampler(point).map_err(e)?;
        let (rtv, rtv2) = (d.create_rtv(target, None).map_err(e)?, d.create_rtv(info, None).map_err(e)?);
        let vs = d.create_shader(Stage::Vertex, QUAD_VS).map_err(e)?;
        let ps = d.create_shader(Stage::Pixel, QUAD_PS).map_err(e)?;
        let layout = d.create_input_layout(&[element("POSITION", format::R32G32_FLOAT, 0), element("TEXCOORD", format::R32G32_FLOAT, u32::MAX)], QUAD_VS).map_err(e)?;
        let vb = buffer(d, &floats(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]), bind::VERTEX_BUFFER, usage::IMMUTABLE)?;
        let cb = buffer(d, &floats(&[2.0, 2.0, -1.0, -1.0]), bind::CONSTANT_BUFFER, usage::DEFAULT)?;
        let none = d.create_rasterizer_state(RasterizerDesc { cull: 1, ..RasterizerDesc::default() }).map_err(e)?;
        d.set_input_layout(Some(layout));
        d.set_topology(5);
        d.set_vertex_buffers(0, &[(Some(vb), 16, 0)]);
        d.set_shader(Stage::Vertex, Some(vs));
        d.set_constant_buffers(Stage::Vertex, 0, &[Some(cb)]);
        d.set_shader(Stage::Pixel, Some(ps));
        d.set_shader_resources(Stage::Pixel, 0, &[Some(srv)]);
        d.set_samplers(Stage::Pixel, 0, &[Some(sampler)]);
        d.set_rasterizer_state(Some(none));
        d.set_render_targets(&[Some(rtv), Some(rtv2)], None);
        d.set_viewports(&[Viewport { x: 0.0, y: 0.0, width: N as f32, height: N as f32, min_depth: 0.0, max_depth: 1.0 }]);
        d.draw(4, 0).map_err(e)?;
        let (w, h, got) = d.read_texture(target, 0).map_err(e)?;
        let (_, _, infos) = d.read_texture(info, 0).map_err(e)?;
        assert_eq!((w, h), (N, N));
        let (got, infos) = (words(&got), words(&infos));
        for y in 0..N {
            for x in 0..N {
                let at = (y * N + x) as usize;
                let want = texels[((3 - y / 2) * 4 + x / 2) as usize];
                if got[at] != want {
                    return Err(format!("({x}, {y}) is {:#010x}, wanted {want:#010x}", got[at]));
                }
                let i = [0, 1, 2, 3].map(|c| f32::from_bits(infos[at * 4 + c]));
                let want = [x as f32 + 0.5, y as f32 + 0.5, 1.0 / 8.0, 1.0];
                if i != want {
                    return Err(format!("({x}, {y})'s position, derivative and w {i:?}, wanted {want:?}"));
                }
            }
        }
        Ok(())
    });
}

/// Depth (a 24-bit depth-stencil format, as 32-bit), indexed draws, back faces culled with Direct3D's clockwise
/// front, a scissor, and a dynamic constant buffer mapped with `WRITE_DISCARD` between draws: green at 0.5 everywhere,
/// red at 0.25 in the left half (the scissor), blue at 0.75 behind both; left red, right green.
#[test]
fn depth_scissor_and_constants() {
    each(|d| {
        const N: u32 = 8;
        let target = texture(d, N, N, format::B8G8R8A8_UNORM, bind::RENDER_TARGET, &[])?;
        let zbuf = texture(d, N, N, format::D24_UNORM_S8_UINT, bind::DEPTH_STENCIL, &[])?;
        let rtv = d.create_rtv(target, None).map_err(e)?;
        let dsv = d.create_dsv(zbuf, None).map_err(e)?;
        let vs = d.create_shader(Stage::Vertex, COLOR_VS).map_err(e)?;
        let ps = d.create_shader(Stage::Pixel, COLOR_PS).map_err(e)?;
        let layout = d.create_input_layout(&[element("POSITION", format::R32G32B32_FLOAT, 0), element("COLOR", format::R8G8B8A8_UNORM, 12)], COLOR_VS).map_err(e)?;
        // Bottom left, top left, top right, bottom right: clockwise on the screen.
        let mut v = Vec::new();
        for (x, y) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, 1.0), (1.0, -1.0)] {
            v.extend(floats(&[x, y, 0.0]));
            v.extend([255, 255, 255, 255]);
        }
        let vb = buffer(d, &v, bind::VERTEX_BUFFER, usage::DEFAULT)?;
        let ib = buffer(d, &[0u16, 1, 2, 0, 2, 3].iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>(), bind::INDEX_BUFFER, usage::DEFAULT)?;
        let cb = d.create_buffer(BufferDesc { size: 32, usage: usage::DYNAMIC, bind: bind::CONSTANT_BUFFER, cpu_access: 0x10000, misc: 0, stride: 0 }, None).map_err(e)?;
        let raster = d.create_rasterizer_state(RasterizerDesc { scissor_enable: true, ..RasterizerDesc::default() }).map_err(e)?;
        d.set_input_layout(Some(layout));
        d.set_topology(4);
        d.set_vertex_buffers(0, &[(Some(vb), 16, 0)]);
        d.set_index_buffer(Some(ib), format::R16_UINT, 0);
        d.set_shader(Stage::Vertex, Some(vs));
        d.set_shader(Stage::Pixel, Some(ps));
        d.set_constant_buffers(Stage::Vertex, 0, &[Some(cb)]);
        d.set_rasterizer_state(Some(raster));
        d.set_render_targets(&[Some(rtv)], Some(dsv));
        d.set_viewports(&[Viewport { x: 0.0, y: 0.0, width: N as f32, height: N as f32, min_depth: 0.0, max_depth: 1.0 }]);
        d.clear_render_target(rtv, [0.0, 0.0, 0.0, 1.0]).map_err(e)?;
        d.clear_depth_stencil(dsv, 1, 1.0).map_err(e)?;
        let n = N as i32;
        for (tint, z, scissor) in [([0.0, 1.0, 0.0, 1.0], 0.5, [0, 0, n, n]), ([1.0, 0.0, 0.0, 1.0], 0.25, [0, 0, n / 2, n]), ([0.0, 0.0, 1.0, 1.0], 0.75, [0, 0, n, n])] {
            let mut bytes = d.map(cb, 0, map::WRITE_DISCARD).map_err(e)?;
            bytes.copy_from_slice(&floats(&[tint[0], tint[1], tint[2], tint[3], 0.0, 0.0, z, 0.0]));
            d.unmap(cb, 0, Some(&bytes)).map_err(e)?;
            d.set_scissor_rects(&[scissor]);
            d.draw_indexed(6, 0, 0).map_err(e)?;
        }
        let (_, _, got) = d.read_texture(target, 0).map_err(e)?;
        let got = words(&got);
        // BGRA in memory: red is 0xffff0000 as a little-endian word, green 0xff00ff00.
        for y in 0..N {
            for x in 0..N {
                let want = if x < N / 2 { 0xffff_0000 } else { 0xff00_ff00 };
                let at = (y * N + x) as usize;
                if got[at] != want {
                    return Err(format!("({x}, {y}) is {:#010x}, wanted {want:#010x}", got[at]));
                }
            }
        }
        // Front faces counter-clockwise: the quad is a back face now, culled, and nothing changes.
        let ccw = d.create_rasterizer_state(RasterizerDesc { front_counter_clockwise: true, ..RasterizerDesc::default() }).map_err(e)?;
        d.set_rasterizer_state(Some(ccw));
        d.clear_depth_stencil(dsv, 1, 1.0).map_err(e)?;
        d.draw_indexed(6, 0, 0).map_err(e)?;
        let (_, _, again) = d.read_texture(target, 0).map_err(e)?;
        if words(&again) != got {
            return Err("a back face was drawn".into());
        }
        Ok(())
    });
}

/// `UpdateSubresource` into a region, copies between textures and into staging ones, `Map` of staging textures and
/// buffers, and buffer copies.
#[test]
fn copies_and_maps() {
    each(|d| {
        let a = texture(d, 4, 4, format::R8G8B8A8_UNORM, bind::SHADER_RESOURCE, &[Data { bytes: &[7; 64], row_pitch: 16 }])?;
        let b = texture(d, 4, 4, format::R8G8B8A8_UNORM, bind::SHADER_RESOURCE, &[])?;
        // A 2×2 region at (1, 2), its rows 12 bytes apart in the data.
        let region: Vec<u8> = (0..24u8).collect();
        d.update_subresource(a, 0, Some(Region { left: 1, top: 2, front: 0, right: 3, bottom: 4, back: 1 }), Data { bytes: &region, row_pitch: 12 }).map_err(e)?;
        // a's rows 2 and 3 into b at (0, 0).
        d.copy_subresource_region(b, 0, 0, 0, a, 0, Some(Region { left: 0, top: 2, front: 0, right: 4, bottom: 4, back: 1 })).map_err(e)?;
        let staging = d
            .create_texture2d(Texture2dDesc { width: 4, height: 4, mip_levels: 1, array_size: 1, format: format::R8G8B8A8_UNORM, sample_count: 1, usage: usage::STAGING, bind: 0, cpu_access: 0x20000, misc: 0 }, &[])
            .map_err(e)?;
        d.copy_resource(staging, b).map_err(e)?;
        let got = d.map(staging, 0, map::READ).map_err(e)?;
        d.unmap(staging, 0, None).map_err(e)?;
        let mut want = vec![0u8; 64];
        for y in 0..2 {
            want[y * 16..y * 16 + 16].copy_from_slice(&[7; 16]);
            want[y * 16 + 4..y * 16 + 12].copy_from_slice(&region[y * 12..y * 12 + 8]);
        }
        if got != want {
            return Err(format!("the staging texture has {got:?}, wanted {want:?}"));
        }
        let (_, _, whole) = d.read_texture(a, 0).map_err(e)?;
        if whole[..32] != [7; 32] || whole[32..48] != want[..16] {
            return Err(format!("a has {whole:?}"));
        }
        let src = buffer(d, &(0..64u8).collect::<Vec<u8>>(), bind::VERTEX_BUFFER, usage::DEFAULT)?;
        let read = d.create_buffer(BufferDesc { size: 64, usage: usage::STAGING, bind: 0, cpu_access: 0x20000, misc: 0, stride: 0 }, None).map_err(e)?;
        d.update_subresource(src, 0, Some(Region { left: 8, top: 0, front: 0, right: 12, bottom: 1, back: 1 }), Data { bytes: &[0xaa; 4], row_pitch: 0 }).map_err(e)?;
        d.copy_subresource_region(read, 0, 16, 0, src, 0, Some(Region { left: 0, top: 0, front: 0, right: 16, bottom: 1, back: 1 })).map_err(e)?;
        let got = d.map(read, 0, map::READ).map_err(e)?;
        let mut want = vec![0u8; 64];
        for (i, w) in want[16..32].iter_mut().enumerate() {
            *w = if (8..12).contains(&i) { 0xaa } else { i as u8 };
        }
        if got != want {
            return Err(format!("the staging buffer has {got:?}, wanted {want:?}"));
        }
        Ok(())
    });
}

/// A compute shader on raw buffers through views: the HLSL arith program's first words.
#[test]
fn compute() {
    each(|d| {
        let n = 64usize;
        let fs: Vec<f32> = (0..n).map(|i| i as f32 / 4.0 - 8.0).collect();
        let input: Vec<u8> = (0..n).flat_map(|i| [fs[i].to_bits(), 1000 + i as u32 * 7]).flat_map(|w| w.to_le_bytes()).collect();
        let raw = |bind_flags| BufferDesc { size: 0, usage: usage::DEFAULT, bind: bind_flags, cpu_access: 0, misc: misc::BUFFER_ALLOW_RAW_VIEWS, stride: 0 };
        let src = d.create_buffer(BufferDesc { size: input.len() as u32, ..raw(bind::SHADER_RESOURCE) }, Some(&input)).map_err(e)?;
        let dst = d.create_buffer(BufferDesc { size: (n * 100) as u32, ..raw(bind::UNORDERED_ACCESS) }, None).map_err(e)?;
        let srv = d.create_srv(src, Some(SrvDesc { dimension: 11, elements: (input.len() / 4) as u32, flags: 1, ..SrvDesc::default() })).map_err(e)?;
        let uav = d.create_uav(dst, None).map_err(e)?;
        let cs = d.create_shader(Stage::Compute, ARITH).map_err(e)?;
        d.set_shader(Stage::Compute, Some(cs));
        d.set_shader_resources(Stage::Compute, 0, &[Some(srv)]);
        d.set_uavs(0, &[Some(uav)]);
        d.dispatch(1, 1, 1).map_err(e)?;
        let read = d.create_buffer(BufferDesc { size: (n * 100) as u32, usage: usage::STAGING, bind: 0, cpu_access: 0x20000, misc: 0, stride: 0 }, None).map_err(e)?;
        d.copy_resource(read, dst).map_err(e)?;
        let got = words(&d.map(read, 0, map::READ).map_err(e)?);
        for (i, x) in fs.iter().enumerate() {
            let row = &got[i * 25..i * 25 + 25];
            let b = 1000 + i as u32 * 7;
            let want = [(0, (x + 1.5).to_bits()), (4, (x - x.floor()).to_bits()), (12, b.wrapping_sub(i as u32)), (24, b * 3 + i as u32)];
            for (at, w) in want {
                if row[at] != w {
                    return Err(format!("thread {i} word {at} is {:#x}, wanted {w:#x}", row[at]));
                }
            }
        }
        if d.create_shader(Stage::Pixel, ARITH).is_ok() {
            return Err("a compute shader made as a pixel shader".into());
        }
        Ok(())
    });
}

/// `GenerateMips`: an 8×8 texture of every level, whose last level is the average.
#[test]
fn generate_mips() {
    each(|d| {
        let level0: Vec<u8> = (0..64u32).flat_map(|i| [if (i % 8 + i / 8) % 2 == 0 { 0 } else { 200 }, 40, 80, 255]).collect();
        let desc = Texture2dDesc {
            width: 8,
            height: 8,
            mip_levels: 0,
            array_size: 1,
            format: format::R8G8B8A8_UNORM,
            sample_count: 1,
            usage: usage::DEFAULT,
            bind: bind::SHADER_RESOURCE | bind::RENDER_TARGET,
            cpu_access: 0,
            misc: misc::GENERATE_MIPS,
        };
        let mut data = vec![Data { bytes: &level0, row_pitch: 32 }];
        let zero = [0u8; 64];
        for _ in 1..4 {
            data.push(Data { bytes: &zero, row_pitch: 0 });
        }
        let t = d.create_texture2d(desc, &data).map_err(e)?;
        if d.texture_desc(t).map(|t| t.mip_levels) != Some(4) {
            return Err("an 8×8 texture of every level hasn't 4".into());
        }
        let srv = d.create_srv(t, None).map_err(e)?;
        d.generate_mips(srv).map_err(e)?;
        let (w, h, last) = d.read_texture(t, 3).map_err(e)?;
        if (w, h, last.as_slice()) != (1, 1, &[100u8, 40, 80, 255][..]) {
            return Err(format!("level 3 is {w}×{h} {last:?}"));
        }
        Ok(())
    });
}
