//! gdi32.dll: drawing, over `win32k::gdi`, which draws host-side into windows' surfaces.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::{Rect, font};

use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::gdi::{Gdi, LogFont, Object, swap};

dll! {
    "GDI32.dll";
    host GetStockObject(1) => get_stock_object;
    host CreateSolidBrush(1) => create_solid_brush;
    host CreateBrushIndirect(1) => create_brush_indirect;
    host CreatePen(3) => create_pen;
    host CreateFontW(14) => create_font_w;
    host CreateFontIndirectW(1) => create_font_indirect_w;
    host SelectObject(2) => select_object;
    host DeleteObject(1) => delete_object;
    host SetTextColor(2) => set_text_color;
    host SetBkColor(2) => set_bk_color;
    host SetBkMode(2) => set_bk_mode;
    host TextOutW(5) => text_out_w;
    host TextOutA(5) => text_out_a;
    host Rectangle(5) => rectangle;
    host MoveToEx(4) => move_to_ex;
    host LineTo(3) => line_to;
    host SetPixel(4) => set_pixel;
    host GetPixel(3) => get_pixel;
    host GetTextExtentPoint32W(4) => get_text_extent_point32_w;
    host GetTextExtentPoint32A(4) => get_text_extent_point32_w;
    host GetDeviceCaps(2) => get_device_caps;
    host BitBlt(9) => bit_blt;
    host StretchBlt(11) => stretch_blt;
    host CreateCompatibleDC(1) => create_compatible_dc;
    host DeleteDC(1) => delete_dc;
    host CreateCompatibleBitmap(3) => create_compatible_bitmap;
    host CreateBitmap(5) => create_bitmap;
    host GetObjectW(3) => get_object_w;
    host GetObjectA(3) => get_object_a;
    host SetStretchBltMode(2) => set_stretch_blt_mode;
    host CreateDIBSection(6) => create_dib_section;
    host StretchDIBits(13) => stretch_di_bits;
    // Not written yet: what installers import.
    // Fonts a program brings in memory, information contexts, and OpenGL's pixel formats.
    host AddFontMemResourceEx(4) => add_font_mem_resource_ex;
    host RemoveFontMemResourceEx(1) => remove_font_mem_resource_ex;
    host CreateICW(4) => create_compatible_dc;
    host CreateICA(4) => create_compatible_dc;
    host ChoosePixelFormat(2) => choose_pixel_format;
    host SetPixelFormat(3) => set_pixel_format;
    host SwapBuffers(1) => swap_buffers;
    // Not written yet: what larger programs import.
    missing AddFontResourceA;
    missing AddFontResourceExA;
    host CombineRgn(4) => combine_rgn;
    host CreateDCW(4) => create_dc_w;
    host CreateFontA(14) => create_font_a;
    host CreateFontIndirectA(1) => create_font_indirect_a;
    host CreateRectRgn(4) => create_rect_rgn;
    host DescribePixelFormat(4) => describe_pixel_format;
    host EnumFontFamiliesExA(5) => enum_font_families_ex_a;
    host EnumFontFamiliesExW(5) => enum_font_families_ex_w;
    missing ExtTextOutA;
    host ExtTextOutW(8) => ext_text_out_w;
    host GdiFlush(0) => one;
    missing GetBitmapBits;
    missing GetCharABCWidthsW;
    host GetClipBox(2) => get_clip_box;
    host GetCurrentObject(2) => get_current_object;
    host GetDIBits(7) => get_di_bits;
    host GetDeviceGammaRamp(2) => nothing;
    missing GetFontData;
    host GetICMProfileW(3) => nothing;
    host GetPixelFormat(1) => one;
    host GetTextMetricsA(2) => get_text_metrics_a;
    host GetTextMetricsW(2) => get_text_metrics_w;
    host GetViewportOrgEx(2) => get_viewport_org_ex;
    missing Polyline;
    missing RemoveFontResourceA;
    host SelectClipRgn(2) => select_clip_rgn;
    host SetBrushOrgEx(4) => set_origin;
    missing SetDCBrushColor;
    host SetDIBitsToDevice(12) => set_di_bits_to_device;
    host SetDeviceGammaRamp(2) => nothing;
    host SetMapMode(2) => set_map_mode;
    host SetRectRgn(5) => set_rect_rgn;
    host SetTextAlign(2) => nothing;
    host SetViewportOrgEx(4) => set_origin;
    missing BeginPath;
    missing CancelDC;
    missing CloseEnhMetaFile;
    missing CloseFigure;
    missing CreateEnhMetaFileW;
    host CreateRectRgnIndirect(1) => create_rect_rgn_indirect;
    missing DeleteEnhMetaFile;
    missing EndDoc;
    missing EndPage;
    missing EndPath;
    missing EnumEnhMetaFile;
    host EqualRgn(2) => equal_rgn;
    missing ExtCreatePen;
    missing ExtEscape;
    missing FillPath;
    host GdiAlphaBlend(11) => gdi_alpha_blend;
    missing GdiComment;
    missing GetCharWidthW;
    host GetClipRgn(2) => get_clip_rgn;
    missing GetEnhMetaFileBits;
    missing GetEnhMetaFileHeader;
    host GetObjectType(1) => get_object_type;
    missing GetOutlineTextMetricsW;
    host GetRegionData(3) => get_region_data;
    host GetRgnBox(2) => get_rgn_box;
    missing GetTextFaceA;
    host GetWorldTransform(2) => get_world_transform;
    host IntersectClipRect(5) => intersect_clip_rect;
    host ModifyWorldTransform(3) => modify_world_transform;
    missing PlayEnhMetaFile;
    missing PlayEnhMetaFileRecord;
    missing PolyBezierTo;
    host PtInRegion(3) => pt_in_region;
    host RestoreDC(2) => restore_dc;
    host SaveDC(1) => save_dc;
    missing SelectClipPath;
    missing SetAbortProc;
    missing SetArcDirection;
    missing SetDCPenColor;
    host SetDIBits(7) => set_di_bits;
    missing SetEnhMetaFileBits;
    host SetGraphicsMode(2) => set_graphics_mode;
    missing SetMiterLimit;
    missing SetPolyFillMode;
    missing SetROP2;
    host SetWorldTransform(2) => set_world_transform;
    missing StartDocW;
    missing StartPage;
    missing StrokeAndFillPath;
    missing StrokePath;
    host CreateRoundRectRgn(6) => create_rect_rgn;
    host RoundRect(7) => rectangle;
    host Polygon(3) => one;
    host CreatePolygonRgn(3) => create_polygon_rgn;
    host GetBkColor(1) => nothing;
    host GetFontUnicodeRanges(2) => nothing;
    host GetGlyphIndicesW(5) => gdi_error;
    host GetGlyphOutlineW(7) => gdi_error;
    host GetTextExtentPointI(4) => nothing;
    host GetTextFaceW(3) => get_text_face_w;
    missing WidenPath;
}

const CLR_INVALID: u64 = 0xffff_ffff;

fn get_stock_object(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(Gdi::stock_object(a[0]))
}

fn create_solid_brush(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.create(Object::Brush(Some(swap(a[0] as u32)))))
}

/// `CreateBrushIndirect(&LOGBRUSH)`: its style and colour (its hatch, a pointer wide, isn't read). A solid or
/// hatched brush fills with its colour (hatching isn't drawn yet), a hollow one fills nothing; a pattern brush, whose
/// bitmap there's no drawing of yet, is its colour too.
fn create_brush_indirect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const BS_HOLLOW: u32 = 1;
    let Ok(b) = task.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), a[0], 8)) else { return Reply::Value(0) };
    let style = u32::from_le_bytes(b[0..4].try_into().unwrap_or_default());
    let color = u32::from_le_bytes(b[4..8].try_into().unwrap_or_default());
    let brush = if style == BS_HOLLOW { Object::Brush(None) } else { Object::Brush(Some(swap(color))) };
    Reply::Value(task.win32k().gdi.create(brush))
}

fn create_pen(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    const PS_NULL: u64 = 5;
    let color = (a[0] & 0xf != PS_NULL).then(|| swap(a[2] as u32));
    Reply::Value(task.win32k().gdi.create(Object::Pen(color)))
}

/// Any font asked for is the one font.
/// `CreateFontW(height, width, escapement, orientation, weight, italic, underline, strike_out, charset, out_precision,
/// clip_precision, quality, pitch_and_family, face)` and `A`: a font that keeps what it's given.
fn create_font(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let mut flags = [0u8; 8];
    for (f, v) in flags.iter_mut().zip(&a[5..13]) {
        *f = *v as u8;
    }
    let face = if a[13] == 0 { String::new() } else { task.text_arg(cx, a[13], wide) };
    let [height, width, escapement, orientation, weight] = [a[0], a[1], a[2], a[3], a[4]].map(|v| v as u32 as i32);
    Reply::Value(task.win32k().gdi.create_font(LogFont { height, width, escapement, orientation, weight, flags, face }))
}

fn create_font_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_font(task, cx, a, true)
}

fn create_font_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_font(task, cx, a, false)
}

/// `CreateFontIndirectW(&logfont)` and `A`.
fn create_font_indirect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let size = if wide { 92 } else { 60 };
    let bytes = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], size)).unwrap_or_default();
    match LogFont::from_bytes(&bytes, wide) {
        Some(font) => Reply::Value(task.win32k().gdi.create_font(font)),
        None => Reply::Value(0),
    }
}

fn create_font_indirect_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_font_indirect(task, cx, a, true)
}

fn create_font_indirect_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_font_indirect(task, cx, a, false)
}

/// `CreateCompatibleDC(dc)`: a memory DC, drawing into the bitmap selected into it.
fn create_compatible_dc(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.new_memory_dc())
}

fn delete_dc(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().gdi.dcs.remove(&a[0]).is_some()))
}

/// `CreateCompatibleBitmap(dc, cx, cy)`: a bitmap that size, black.
fn create_compatible_bitmap(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (w, h) = (a[1] as i32, a[2] as i32);
    if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
        return Reply::Value(0);
    }
    Reply::Value(task.win32k().gdi.create_bitmap(display::Surface::new(w, h, 0)))
}

/// `CreateBitmap(cx, cy, planes, bits, &pixels)`: from 32-bit or 24-bit pixels (rows top first, padded to words as
/// a DDB's are), or 1-bit (set is white); none, black.
fn create_bitmap(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (w, h, bpp, at) = (a[0] as i32, a[1] as i32, a[3] as u32, a[4]);
    if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
        return Reply::Value(0);
    }
    let mut s = display::Surface::new(w, h, 0);
    let stride = ((w as usize * bpp as usize).div_ceil(16)) * 2;
    if at != 0 && matches!(bpp, 1 | 24 | 32) {
        let Ok(bits) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, stride * h as usize)) else { return Reply::Value(0) };
        for y in 0..h as usize {
            let row = &bits[y * stride..(y + 1) * stride];
            for x in 0..w as usize {
                s.pixels[y * w as usize + x] = match bpp {
                    1 => if row[x / 8] >> (7 - x % 8) & 1 != 0 { 0xff_ffff } else { 0 },
                    24 => u32::from(row[3 * x + 2]) << 16 | u32::from(row[3 * x + 1]) << 8 | u32::from(row[3 * x]),
                    _ => u32::from(row[4 * x + 2]) << 16 | u32::from(row[4 * x + 1]) << 8 | u32::from(row[4 * x]),
                };
            }
        }
    }
    Reply::Value(task.win32k().gdi.create_bitmap(s))
}

fn get_object_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_object(task, cx, a, true)
}

fn get_object_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_object(task, cx, a, false)
}

/// `GetObject(object, size, &out)`: a font's LOGFONT (W or A), or a bitmap's BITMAP (type, width, height, bytes a
/// row, planes, bits a pixel, no bits of its own here); with no buffer, the size. Other objects aren't described yet.
fn get_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let font = task.win32k().gdi.log_font(a[0]);
    if let Some(font) = font {
        let b = font.to_bytes(wide);
        if a[2] == 0 {
            return Reply::Value(b.len() as u64);
        }
        let n = b.len().min(a[1] as u32 as usize);
        if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b[..n])).is_err() {
            return Reply::Value(0);
        }
        return Reply::Value(n as u64);
    }
    let w = task.abi().width as usize;
    let bmp_size = if w == 8 { 32 } else { 24 };
    let dib_size = if w == 8 { 104 } else { 84 };
    let (width, height, dib) = {
        let g = &task.win32k().gdi;
        let Some(s) = g.bitmaps.get(&a[0]) else { return Reply::Value(0) };
        (s.width, s.height, g.dibs.get(&a[0]).copied())
    };
    let req = a[1] as u32 as usize;
    let out_size = if dib.is_some() && req >= dib_size { dib_size } else { bmp_size };
    if a[2] == 0 {
        return Reply::Value(bmp_size as u64);
    }
    let mut b = vec![0u8; out_size];
    let (stride, bpp, bits_ptr) = match dib {
        Some(d) => (d.stride as i32, d.bits_per_pixel, d.bits),
        None => (width * 4, 32, 0),
    };
    for (o, v) in [(4, width), (8, height), (12, stride)] {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[16..18].copy_from_slice(&1u16.to_le_bytes());
    b[18..20].copy_from_slice(&bpp.to_le_bytes());
    if bits_ptr != 0 {
        let ptr_off = if w == 8 { 24 } else { 20 };
        let bytes = task.abi().ptr(bits_ptr);
        b[ptr_off..ptr_off + w].copy_from_slice(&bytes);
    }
    if let Some(d) = dib
        && out_size == dib_size
    {
        // dsBmih (BITMAPINFOHEADER, 40 bytes) right after dsBm, then dsBitfields[3] (12 bytes), dshSection, dsOffset.
        let h = bmp_size;
        b[h..h + 4].copy_from_slice(&40u32.to_le_bytes());
        b[h + 4..h + 8].copy_from_slice(&d.width.to_le_bytes());
        b[h + 8..h + 12].copy_from_slice(&d.height.to_le_bytes());
        b[h + 12..h + 14].copy_from_slice(&1u16.to_le_bytes());
        b[h + 14..h + 16].copy_from_slice(&d.bits_per_pixel.to_le_bytes());
        b[h + 16..h + 20].copy_from_slice(&d.compression.to_le_bytes());
        b[h + 20..h + 24].copy_from_slice(&((d.stride * d.height.unsigned_abs() as usize) as u32).to_le_bytes());
        for (i, m) in d.masks.iter().enumerate() {
            let off = h + 40 + i * 4;
            b[off..off + 4].copy_from_slice(&m.to_le_bytes());
        }
    }
    let n = out_size.min(req);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b[..n])).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(n as u64)
}

/// `BitBlt(dst, x, y, cx, cy, src, sx, sy, rop)`.
fn bit_blt(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    sync_dc_dib_from_guest(task, cx, a[5]);
    sync_dc_dib_from_guest(task, cx, a[0]);
    let (x, y, w, h) = (a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (sx, sy) = (a[6] as i32, a[7] as i32);
    let to = Rect::new(x, y, x + w, y + h);
    let from = Rect::new(sx, sy, sx + w, sy + h);
    let ok = task.win32k().blit(a[0], to, a[5], from, a[8] as u32);
    if ok {
        sync_dc_dib_to_guest(task, cx, a[0]);
    }
    Reply::Value(u64::from(ok))
}

/// `StretchBlt(dst, x, y, cx, cy, src, sx, sy, scx, scy, rop)`: scaled, each pixel the nearest.
fn stretch_blt(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    sync_dc_dib_from_guest(task, cx, a[5]);
    sync_dc_dib_from_guest(task, cx, a[0]);
    let (x, y, w, h) = (a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (sx, sy, sw, sh) = (a[6] as i32, a[7] as i32, a[8] as i32, a[9] as i32);
    let to = Rect::new(x, y, x + w, y + h);
    let from = Rect::new(sx, sy, sx + sw, sy + sh);
    let ok = task.win32k().blit(a[0], to, a[5], from, a[10] as u32);
    if ok {
        sync_dc_dib_to_guest(task, cx, a[0]);
    }
    Reply::Value(u64::from(ok))
}

/// `SetStretchBltMode`: stretching is always to the nearest pixel; the mode before, BLACKONWHITE.
fn set_stretch_blt_mode(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn select_object(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.select(a[0], a[1]).unwrap_or(0))
}

fn delete_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let dib = task.win32k().gdi.dibs.get(&a[0]).copied();
    let deleted = task.win32k().gdi.delete(a[0]);
    if deleted
        && let Some(dib) = dib
        && dib.pid == task.process.pid
        && dib.bits != 0
    {
        if dib.owned_alloc {
            let _ = task.virtual_free(cx, dib.bits, 0, 0x8000);
        } else {
            let _ = super::memapi::unmap_view_of_file(task, cx, &[dib.bits & !0xffff]);
        }
    }
    Reply::Value(u64::from(deleted))
}

/// Sets one of a DC's colours or modes, giving the one it had.
fn set_dc(task: &mut Task, hdc: u64, change: impl FnOnce(&mut crate::win32k::gdi::Dc) -> u32) -> Reply {
    match task.win32k().gdi.dcs.get_mut(&hdc) {
        Some(dc) => Reply::Value(u64::from(change(dc))),
        None => Reply::Value(CLR_INVALID),
    }
}

fn set_text_color(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_dc(task, a[0], |dc| swap(std::mem::replace(&mut dc.text, swap(a[1] as u32))))
}

fn set_bk_color(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_dc(task, a[0], |dc| swap(std::mem::replace(&mut dc.background, swap(a[1] as u32))))
}

fn set_bk_mode(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_dc(task, a[0], |dc| std::mem::replace(&mut dc.mode, a[1] as u32))
}

fn text(task: &mut Task, cx: &mut Cx<'_>, at: u64, count: u64) -> String {
    let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), at, (count as u32 as usize).min(4096) * 2)).unwrap_or_default();
    String::from_utf16_lossy(&raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
}

fn text_out_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let s = text(task, cx, a[3], a[4]);
    Reply::Value(u64::from(task.win32k().text_out(a[0], a[1] as i32, a[2] as i32, &s)))
}

fn ext_text_out_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[5] != 0 && (a[6] as u32) > 0 {
        let s = text(task, cx, a[5], a[6]);
        return Reply::Value(u64::from(task.win32k().text_out(a[0], a[1] as i32, a[2] as i32, &s)));
    }
    Reply::Value(1)
}

fn gdi_error(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0xffff_ffff)
}

fn create_polygon_rgn(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    create_rect_rgn(task, cx, &[0, 0, 1, 1])
}

fn get_text_face_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let units: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    let cap = a[1] as u32 as usize;
    if cap == 0 || a[2] == 0 {
        return Reply::Value(units.len() as u64);
    }
    let n = units.len().min(cap);
    let mut out = units[..n].to_vec();
    out[n - 1] = 0;
    let bytes: Vec<u8> = out.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &bytes));
    Reply::Value(n as u64)
}

/// `TextOutA`: its count is of bytes, in the ANSI code page.
fn text_out_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), a[3], (a[4] as u32 as usize).min(4096))).unwrap_or_default();
    let s = crate::ansi::text(&raw);
    Reply::Value(u64::from(task.win32k().text_out(a[0], a[1] as i32, a[2] as i32, &s)))
}

fn rectangle(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().rectangle(a[0], Rect::new(a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32))))
}

fn move_to_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let old = {
        let mut w = task.win32k();
        let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
        std::mem::replace(&mut dc.position, (a[1] as i32, a[2] as i32))
    };
    if a[3] != 0 {
        let bytes: Vec<u8> = [old.0, old.1].iter().flat_map(|v| v.to_le_bytes()).collect();
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &bytes));
    }
    Reply::Value(1)
}

fn line_to(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().line_to(a[0], a[1] as i32, a[2] as i32)))
}

fn set_pixel(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let color = swap(a[3] as u32);
    if task.win32k().set_pixel(a[0], a[1] as i32, a[2] as i32, color) { Reply::Value(u64::from(swap(color))) } else { Reply::Value(CLR_INVALID) }
}

fn get_pixel(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().get_pixel(a[0], a[1] as i32, a[2] as i32).map_or(CLR_INVALID, |c| u64::from(swap(c))))
}

fn get_text_extent_point32_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let size = [a[2] as i32 * font::CELL_WIDTH, font::CELL_HEIGHT];
    let bytes: Vec<u8> = size.iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &bytes)).is_ok()))
}

/// `GetDeviceCaps`: the screen's, from the desktop's output.
fn get_device_caps(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (width, height) = task.desktop(|d| (d.output().width, d.output().height));
    let dpi = crate::win32k::PROGRAM_DPI;
    Reply::Value(match a[1] as u32 {
        8 => width as u64,          // HORZRES
        10 => height as u64,        // VERTRES
        12 => 32,                   // BITSPIXEL
        14 => 1,                    // PLANES
        88 | 90 => u64::from(dpi),  // LOGPIXELSX, LOGPIXELSY
        _ => 0,
    })
}

/// `AddFontMemResourceEx(data, size, reserved, &count)`: a handle for the fonts, and how many there are (one, as
/// atlas draws text in its own font whichever is chosen).
fn add_font_mem_resource_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x5f00);
    if a[0] == 0 || a[1] == 0 || a[3] == 0 || task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], 1)).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(NEXT.fetch_add(4, Ordering::Relaxed))
}

fn remove_font_mem_resource_ex(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] != 0))
}

/// `ChoosePixelFormat(dc, &descriptor)`: format 1, the one there is.
fn choose_pixel_format(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[1] != 0))
}

/// `DescribePixelFormat(dc, format, bytes, &descriptor)`: describes format 1 (32-bit BGRA with 24-bit depth and 8-bit
/// stencil, advertising OpenGL double-buffering when a GPU is available), returning the maximum pixel format index (1).
fn describe_pixel_format(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let format = a[1] as u32 as i32;
    let n_bytes = a[2] as u32;
    let ppfd = a[3];
    if ppfd == 0 {
        return Reply::Value(1);
    }
    if format != 1 || n_bytes < 40 {
        return Reply::Value(0);
    }
    let mut pfd = [0u8; 40];
    pfd[0..2].copy_from_slice(&40u16.to_le_bytes()); // nSize
    pfd[2..4].copy_from_slice(&1u16.to_le_bytes()); // nVersion
    // PFD_DRAW_TO_WINDOW (0x04) | PFD_SUPPORT_OPENGL (0x20) | PFD_DOUBLEBUFFER (0x01) when GPU is present,
    // else PFD_DRAW_TO_WINDOW (0x04) | PFD_SUPPORT_GDI (0x10).
    let flags: u32 = if task.k.options.gpu.is_some() { 0x0000_0025 } else { 0x0000_0014 };
    pfd[4..8].copy_from_slice(&flags.to_le_bytes());
    pfd[8] = 0; // PFD_TYPE_RGBA
    pfd[9] = 32; // cColorBits
    pfd[10] = 8; // cRedBits
    pfd[11] = 16; // cRedShift
    pfd[12] = 8; // cGreenBits
    pfd[13] = 8; // cGreenShift
    pfd[14] = 8; // cBlueBits
    pfd[15] = 0; // cBlueShift
    pfd[16] = 8; // cAlphaBits
    pfd[17] = 24; // cAlphaShift
    pfd[23] = 24; // cDepthBits
    pfd[24] = 8; // cStencilBits
    if task.mem(cx, true, |t| user::write(&mut t.space(), ppfd, &pfd)).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(1)
}

/// `SetPixelFormat(dc, format, &descriptor)`: format 1 is taken.
fn set_pixel_format(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[1] as u32 == 1))
}

/// `SwapBuffers(dc)`: presents the OpenGL back buffer to the window if active.
fn swap_buffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    super::opengl32::swap_buffers(task, cx, a[0])
}

/// `GetCurrentObject(dc, type)`: the pen (OBJ_PEN, 1), brush (2), font (6) or bitmap (7) the DC has selected.
fn get_current_object(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(dc) = w.gdi.dcs.get(&a[0]) else { return Reply::Value(0) };
    Reply::Value(match a[1] as u32 {
        1 => dc.pen,
        2 => dc.brush,
        6 => dc.font,
        7 => dc.bitmap,
        _ => 0,
    })
}

/// `GetClipBox(dc, &rect)`: where it may draw; SIMPLEREGION (2).
fn get_clip_box(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(clip) = task.win32k().gdi.dcs.get(&a[0]).map(|dc| dc.clip) else { return Reply::Value(0) };
    if !task.write_rect(cx, a[1], &clip) {
        return Reply::Value(0);
    }
    Reply::Value(2)
}

/// A point's two LONGs written where asked (0, 0: the origin, which is all there is with MM_TEXT).
fn put_origin(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Reply {
    if at != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), at, &[0u8; 8]));
    }
    Reply::Value(1)
}

/// `GetViewportOrgEx(dc, &point)`, `SetViewportOrgEx(dc, x, y, &old)` and `SetBrushOrgEx(dc, x, y, &old)`: the
/// origin stays where it is.
fn get_viewport_org_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_origin(task, cx, a[1])
}

fn set_origin(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_origin(task, cx, a[3])
}

/// `SetMapMode(dc, mode)`: MM_TEXT (1), what it was and stays.
fn set_map_mode(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

/// What takes its arguments and does nothing more: `GdiFlush` (drawing isn't batched), `SetTextAlign` (TA_TOP |
/// TA_LEFT, the alignment there was), `GetPixelFormat` (format 1).
fn one(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn nothing(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `CreateDCW(driver, device, port, &mode)`: a DC for the display, as a memory DC.
fn create_dc_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_compatible_dc(task, cx, a)
}

// ---- Regions, as rectangles ----

const NULLREGION: u64 = 1;
const SIMPLEREGION: u64 = 2;

fn region_kind(r: &Rect) -> u64 {
    if r.right <= r.left || r.bottom <= r.top { NULLREGION } else { SIMPLEREGION }
}

fn rect_args(a: &[u64]) -> Rect {
    Rect { left: a[0] as u32 as i32, top: a[1] as u32 as i32, right: a[2] as u32 as i32, bottom: a[3] as u32 as i32 }
}

/// `CreateRectRgn(left, top, right, bottom)`.
fn create_rect_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.create(Object::Region(rect_args(a))))
}

/// `CreateRectRgnIndirect(&rect)`.
fn create_rect_rgn_indirect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.read_rect(cx, a[0]) else { return Reply::Value(0) };
    Reply::Value(task.win32k().gdi.create(Object::Region(r)))
}

/// `EqualRgn(a, b)`: TRUE if both regions cover the same rectangle (or are both empty).
fn equal_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let (Some(x), Some(y)) = (w.gdi.region(a[0]), w.gdi.region(a[1])) else { return Reply::Value(0) };
    let eq = (region_kind(&x) == NULLREGION && region_kind(&y) == NULLREGION) || x == y;
    Reply::Value(u64::from(eq))
}

/// `GetClipRgn(hdc, hrgn)`: 1 if a custom clip region was copied into `hrgn`, 0 if none is set, -1 on error.
fn get_clip_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some((hwnd, clip)) = w.gdi.dcs.get(&a[0]).map(|dc| (dc.hwnd, dc.clip)) else {
        return Reply::Value((-1i32) as u32 as u64);
    };
    let whole = w.windows.get(&hwnd).map(|win| win.client()).unwrap_or(Rect { left: 0, top: 0, right: i32::MAX / 2, bottom: i32::MAX / 2 });
    if clip == whole {
        return Reply::Value(0);
    }
    Reply::Value(if w.gdi.set_region(a[1], clip) { 1 } else { (-1i32) as u32 as u64 })
}

/// `GetRgnBox(hrgn, &rect)`: writes the bounding rectangle and returns `NULLREGION` or `SIMPLEREGION`.
fn get_rgn_box(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.win32k().gdi.region(a[0]) else { return Reply::Value(0) };
    let out = if region_kind(&r) == NULLREGION { Rect::default() } else { r };
    if a[1] != 0 {
        task.write_rect(cx, a[1], &out);
    }
    Reply::Value(region_kind(&r))
}

/// `GetRegionData(hrgn, count, data)`: `RGNDATAHEADER` (32 bytes) followed by 0 or 1 `RECT`s.
fn get_region_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.win32k().gdi.region(a[0]) else { return Reply::Value(0) };
    let empty = region_kind(&r) == NULLREGION;
    let rect_count: u32 = if empty { 0 } else { 1 };
    let rgn_size = rect_count * 16;
    let total = 32 + rgn_size;
    if a[2] == 0 {
        return Reply::Value(u64::from(total));
    }
    if (a[1] as u32) < total {
        return Reply::Value(0);
    }
    let bound = if empty { Rect::default() } else { r };
    let mut buf = Vec::with_capacity(total as usize);
    for v in [32u32, 1, rect_count, rgn_size] {
        buf.extend(v.to_le_bytes());
    }
    for v in [bound.left, bound.top, bound.right, bound.bottom] {
        buf.extend(v.to_le_bytes());
    }
    if !empty {
        for v in [r.left, r.top, r.right, r.bottom] {
            buf.extend(v.to_le_bytes());
        }
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &buf)).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(u64::from(total))
}

/// `PtInRegion(hrgn, x, y)`.
fn pt_in_region(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.win32k().gdi.region(a[0]) else { return Reply::Value(0) };
    let (x, y) = (a[1] as u32 as i32, a[2] as u32 as i32);
    Reply::Value(u64::from(x >= r.left && x < r.right && y >= r.top && y < r.bottom))
}

/// `GetObjectType(h)`: returns `OBJ_PEN`..`OBJ_MEMDC` (or 0 for an unknown handle).
fn get_object_type(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    if let Some(dc) = w.gdi.dcs.get(&a[0]) {
        return Reply::Value(if dc.memory { 10 } else { 3 });
    }
    Reply::Value(match w.gdi.object(a[0]) {
        Some(Object::Pen(_)) => 1,
        Some(Object::Brush(_)) => 2,
        Some(Object::Font) => 6,
        Some(Object::Bitmap) => 7,
        Some(Object::Region(_)) => 8,
        Some(Object::Icon) | None => 0,
    })
}

/// `SetRectRgn(region, left, top, right, bottom)`.
fn set_rect_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().gdi.set_region(a[0], rect_args(&a[1..]))))
}

/// `CombineRgn(destination, a, b, mode)`: RGN_AND (1) their overlap; RGN_OR (2) and RGN_XOR (3) the rectangle
/// around both; RGN_DIFF (4) and RGN_COPY (5) the first. What kind the result is: NULLREGION, SIMPLEREGION or ERROR.
fn combine_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(x) = w.gdi.region(a[1]) else { return Reply::Value(0) };
    let y = w.gdi.region(a[2]).unwrap_or_default();
    let r = match a[3] as u32 {
        1 => Rect { left: x.left.max(y.left), top: x.top.max(y.top), right: x.right.min(y.right), bottom: x.bottom.min(y.bottom) },
        2 | 3 if region_kind(&x) == NULLREGION => y,
        2 | 3 if region_kind(&y) == NULLREGION => x,
        2 | 3 => Rect { left: x.left.min(y.left), top: x.top.min(y.top), right: x.right.max(y.right), bottom: x.bottom.max(y.bottom) },
        4 | 5 => x,
        _ => return Reply::Value(0),
    };
    if !w.gdi.set_region(a[0], r) {
        return Reply::Value(0);
    }
    Reply::Value(region_kind(&r))
}

/// `SelectClipRgn(dc, region)`: where it may draw, the region's rectangle in its window's client area (none: all of
/// it).
fn select_clip_rgn(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let region = w.gdi.region(a[1]);
    let Some(hwnd) = w.gdi.dcs.get(&a[0]).map(|dc| dc.hwnd) else { return Reply::Value(0) };
    let whole = w.windows.get(&hwnd).map(|win| win.client()).unwrap_or(Rect { left: 0, top: 0, right: i32::MAX / 2, bottom: i32::MAX / 2 });
    let clip = region.unwrap_or(whole);
    if let Some(dc) = w.gdi.dcs.get_mut(&a[0]) {
        dc.clip = clip;
    }
    Reply::Value(region_kind(&clip))
}

// ---- Fonts listed ----

/// The font families listed: atlas draws text in one font, under any of these names.
const FAMILIES: [&str; 4] = ["Segoe UI", "Arial", "Tahoma", "Courier New"];

/// Text in a fixed field of `chars` characters.
fn face(s: &str, chars: usize, wide: bool) -> Vec<u8> {
    let mut b: Vec<u8> = if wide { s.encode_utf16().flat_map(u16::to_le_bytes).collect() } else { s.as_bytes().to_vec() };
    b.resize(if wide { chars * 2 } else { chars }, 0);
    b
}

/// ENUMLOGFONTEX and NEWTEXTMETRICEX for a family, as one block: the first at 0, the second where the second value
/// says.
fn font_entry(family: &str, wide: bool) -> (Vec<u8>, usize) {
    let mut b = Vec::new();
    // LOGFONT: height, width, escapement, orientation, weight; italic, underline, strike-out, charset, out precision,
    // clip precision, quality, pitch and family; the face.
    for v in [-16i32, 0, 0, 0, 400] {
        b.extend(v.to_le_bytes());
    }
    b.extend([0u8, 0, 0, 0, 3, 2, 1, 0x22]);
    b.extend(face(family, 32, wide));
    // The full name, the style, the script.
    b.extend(face(family, 64, wide));
    b.extend(face("Regular", 32, wide));
    b.extend(face("Western", 32, wide));
    let metrics_at = b.len();
    // TEXTMETRIC: height, ascent, descent, internal and external leading, average and widest character, weight,
    // overhang, aspect; first, last, default and break characters; italic, underlined, struck out, pitch and family,
    // charset.
    for v in [16i32, 13, 3, 3, 0, 7, 14, 400, 0, 96, 96] {
        b.extend(v.to_le_bytes());
    }
    if wide {
        for c in [0x20u16, 0xfffc, 0x1f, 0x20] {
            b.extend(c.to_le_bytes());
        }
    } else {
        b.extend([0x20u8, 0xff, 0x1f, 0x20]);
    }
    b.extend([0u8, 0, 0, 0x27, 0]);
    b.resize(b.len().next_multiple_of(4), 0);
    // NTM_REGULAR, the em square, cell height, average width; then FONTSIGNATURE, Latin 1 in its code pages.
    for v in [0x40u32, 2048, 2400, 1000, 0, 0, 0, 0, 1, 0] {
        b.extend(v.to_le_bytes());
    }
    (b, metrics_at)
}

/// `EnumFontFamiliesExW(dc, &logfont, callback, data, flags)` and `A`: the callback for each family (the one named,
/// if the LOGFONT names one), as TrueType, until it returns 0. Its last return.
fn enum_font_families_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name_at = a[1] + 28;
    let wanted = if wide { task.wstr(cx, name_at) } else { task.text_arg(cx, name_at, false) };
    let mut last = 1;
    for family in FAMILIES.iter().filter(|f| wanted.is_empty() || f.eq_ignore_ascii_case(&wanted)) {
        let (block, metrics_at) = font_entry(family, wide);
        match task.upcall_data(cx, a[2], &block, |at| vec![at, at + metrics_at as u64, 4, a[3]]) {
            Ok(0) => return Reply::Value(0),
            Ok(v) => last = v,
            Err(reply) => return reply,
        }
    }
    Reply::Value(last)
}

fn enum_font_families_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_font_families_ex(task, cx, a, true)
}

fn enum_font_families_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_font_families_ex(task, cx, a, false)
}

/// `GetTextMetricsW(dc, &metrics)` and `A`: atlas's one font's TEXTMETRIC, whatever's selected.
fn get_text_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !task.win32k().gdi.dcs.contains_key(&a[0]) {
        return Reply::Value(0);
    }
    let (block, at) = font_entry(FAMILIES[0], wide);
    let size = if wide { 60 } else { 56 };
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &block[at..at + size])).is_ok()))
}

fn get_text_metrics_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_text_metrics(task, cx, a, true)
}

fn get_text_metrics_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_text_metrics(task, cx, a, false)
}

// ---- Device-independent bitmaps ----

/// What a BITMAPINFOHEADER says of a DIB's pixels: its size (the height negative for one top row first), bits per
/// pixel, and, for 16 and 32 bits, the red, green and blue masks (BI_BITFIELDS', or BI_RGB's own).
#[derive(Debug, Clone, Copy, PartialEq)]
struct DibFormat {
    width: i32,
    height: i32,
    bits: u16,
    masks: [u32; 3],
}

impl DibFormat {
    /// From the header's bytes and the masks after it. None for a depth atlas doesn't convert (palettes) or a
    /// compression it doesn't take.
    fn parse(header: &[u8], after: &[u8]) -> Option<DibFormat> {
        let u32_at = |b: &[u8], at: usize| b.get(at..at + 4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]));
        let (width, height) = (u32_at(header, 4)? as i32, u32_at(header, 8)? as i32);
        let bits = u16::from_le_bytes([*header.get(14)?, *header.get(15)?]);
        let compression = u32_at(header, 16)?;
        let masks = match (bits, compression) {
            (16, 0) => [0x7c00, 0x03e0, 0x001f],
            (24 | 32, 0) => [0xff_0000, 0xff00, 0xff],
            (16 | 32, 3) => [u32_at(after, 0)?, u32_at(after, 4)?, u32_at(after, 8)?],
            _ => return None,
        };
        (width > 0 && height != 0 && width <= 16384 && height.unsigned_abs() <= 16384).then_some(DibFormat { width, height, bits, masks })
    }

    /// Bytes in a row, padded to four.
    fn stride(&self) -> usize {
        (self.width as usize * usize::from(self.bits)).div_ceil(32) * 4
    }

    /// The bitmap's row for DIB scan line `line` (0 is the bottom one, unless the DIB is top row first).
    fn row(&self, line: usize, bitmap_height: i32) -> i32 {
        if self.height < 0 { line as i32 } else { bitmap_height - 1 - line as i32 }
    }
}

/// A channel, scaled from its mask's bits to eight.
fn channel(value: u32, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let (shift, width) = (mask.trailing_zeros(), mask.count_ones());
    let v = (value & mask) >> shift;
    if width >= 8 { v >> (width - 8) } else { (v << (8 - width)) | (v >> (2 * width).saturating_sub(8).min(31)) }
}

/// A pixel (0x00RRGGBB) as the DIB stores it.
fn to_dib(rgb: u32, f: &DibFormat, out: &mut Vec<u8>) {
    let (r, g, b) = (rgb >> 16 & 0xff, rgb >> 8 & 0xff, rgb & 0xff);
    // An 8-bit channel, scaled to its mask's width and moved to its place.
    let put = |c: u32, mask: u32| {
        let width = mask.count_ones();
        let v = if width >= 8 { c << (width - 8) } else { c >> (8 - width) };
        (v << mask.trailing_zeros()) & mask
    };
    match f.bits {
        24 => out.extend([b as u8, g as u8, r as u8]),
        16 => out.extend(((put(r, f.masks[0]) | put(g, f.masks[1]) | put(b, f.masks[2])) as u16).to_le_bytes()),
        _ => out.extend((put(r, f.masks[0]) | put(g, f.masks[1]) | put(b, f.masks[2])).to_le_bytes()),
    }
}

/// A DIB's pixel at `x` of `row` as 0x00RRGGBB.
fn from_dib(row: &[u8], x: usize, f: &DibFormat) -> u32 {
    let v = match f.bits {
        24 => return u32::from(row[3 * x + 2]) << 16 | u32::from(row[3 * x + 1]) << 8 | u32::from(row[3 * x]),
        16 => u32::from(u16::from_le_bytes([row[2 * x], row[2 * x + 1]])),
        _ => u32::from_le_bytes([row[4 * x], row[4 * x + 1], row[4 * x + 2], row[4 * x + 3]]),
    };
    channel(v, f.masks[0]) << 16 | channel(v, f.masks[1]) << 8 | channel(v, f.masks[2])
}

/// The BITMAPINFO at `at`: its header's bytes, and the 12 after them (BI_BITFIELDS' masks).
fn read_info(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<(Vec<u8>, Vec<u8>)> {
    let size = task.mem(cx, false, |t| user::read_u32(&t.space(), at)).ok()? as u64;
    if !(40..=124).contains(&size) {
        return None;
    }
    let header = task.mem(cx, false, |t| user::read_vec(&t.space(), at, size as usize)).ok()?;
    // A BITMAPINFOHEADER's masks come after it; a V4 or V5 header has them inside, at the same place.
    let after = task.mem(cx, false, |t| user::read_vec(&t.space(), at + 40, 12)).unwrap_or_default();
    Some((header, after))
}

/// `GetDIBits(dc, bitmap, first line, lines, &bits, &info, usage)`: a bitmap's pixels as the DIB `info` describes,
/// from scan line `first` on; how many lines it wrote. With no `bits`, it fills `info`'s header in instead: the
/// bitmap's size and, if its depth was 0, 32 bits (the screen's).
fn get_di_bits(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const ERROR_INVALID_PARAMETER: u32 = 87;
    let (bitmap, first, lines, bits_at, info_at) = (a[1], a[2] as u32 as usize, a[3] as u32 as usize, a[4], a[5]);
    sync_dib_from_guest(task, cx, bitmap);
    let Some(surface) = task.win32k().gdi.bitmaps.get(&bitmap).cloned() else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let Some((header, after)) = read_info(task, cx, info_at) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if bits_at == 0 {
        let mut bits = u16::from_le_bytes([header[14], header[15]]);
        if bits == 0 {
            bits = 32;
        }
        let stride = (surface.width as usize * usize::from(bits)).div_ceil(32) * 4;
        let mut h = header[..40].to_vec();
        h[4..8].copy_from_slice(&surface.width.to_le_bytes());
        h[8..12].copy_from_slice(&surface.height.to_le_bytes());
        h[12..14].copy_from_slice(&1u16.to_le_bytes());
        h[14..16].copy_from_slice(&bits.to_le_bytes());
        if u32::from_le_bytes([h[16], h[17], h[18], h[19]]) != 3 {
            h[16..20].copy_from_slice(&0u32.to_le_bytes());
        }
        h[20..24].copy_from_slice(&((stride * surface.height as usize) as u32).to_le_bytes());
        if task.mem(cx, true, |t| user::write(&mut t.space(), info_at + 4, &h[4..])).is_err() {
            return task.fail(cx, 998);
        }
        return Reply::Value(1);
    }
    let Some(f) = DibFormat::parse(&header, &after) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let lines = lines.min((f.height.unsigned_abs() as usize).saturating_sub(first));
    let mut out = Vec::with_capacity(lines * f.stride());
    for line in first..first + lines {
        let row = f.row(line, surface.height);
        let start = out.len();
        for x in 0..f.width {
            let rgb = if (0..surface.height).contains(&row) && x < surface.width { surface.pixels[(row * surface.width + x) as usize] } else { 0 };
            to_dib(rgb & 0xff_ffff, &f, &mut out);
        }
        out.resize(start + f.stride(), 0);
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), bits_at, &out)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(lines as u64)
}

/// `SetDIBits(dc, bitmap, first line, lines, &bits, &info, usage)`: a bitmap's pixels from the DIB `info` describes,
/// from scan line `first` on; how many lines it took.
fn set_di_bits(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const ERROR_INVALID_PARAMETER: u32 = 87;
    let (bitmap, first, lines, bits_at, info_at) = (a[1], a[2] as u32 as usize, a[3] as u32 as usize, a[4], a[5]);
    let Some(height) = task.win32k().gdi.bitmaps.get(&bitmap).map(|s| s.height) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let Some((header, after)) = read_info(task, cx, info_at) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let Some(f) = DibFormat::parse(&header, &after) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let lines = lines.min((f.height.unsigned_abs() as usize).saturating_sub(first));
    let Ok(bits) = task.mem(cx, false, |t| user::read_vec(&t.space(), bits_at, lines * f.stride())) else { return task.fail(cx, 998) };
    let mut w = task.win32k();
    let Some(surface) = w.gdi.bitmaps.get_mut(&bitmap) else { return Reply::Value(0) };
    for i in 0..lines {
        let row = f.row(first + i, height);
        if !(0..surface.height).contains(&row) {
            continue;
        }
        let src = &bits[i * f.stride()..(i + 1) * f.stride()];
        for x in 0..f.width.min(surface.width) {
            surface.pixels[(row * surface.width + x) as usize] = from_dib(src, x as usize, &f);
        }
    }
    drop(w);
    sync_dib_to_guest(task, cx, bitmap);
    Reply::Value(lines as u64)
}

/// `CreateDIBSection(hdc, &bmi, usage, &ppvBits, hSection, offset)`: allocates a DIB-backed bitmap whose pixel
/// buffer lives in guest memory at `*ppvBits` and stays synchronized with GDI operations.
fn create_dib_section(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    use crate::win32k::gdi::DibSection;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    let (bmi_at, ppv_bits, section, offset) = (a[1], a[3], a[4], a[5] as u32 as u64);
    if bmi_at == 0 {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    let Some((header, after)) = read_info(task, cx, bmi_at) else {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    };
    let u32_at = |b: &[u8], at: usize| b.get(at..at + 4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]));
    let Some(width) = u32_at(&header, 4).map(|v| v as i32) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let Some(height) = u32_at(&header, 8).map(|v| v as i32) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let bits_per_pixel = u16::from_le_bytes([header[14], header[15]]);
    let compression = u32_at(&header, 16).unwrap_or(0);
    let abs_height = height.unsigned_abs() as i32;
    if width <= 0 || abs_height <= 0 || width > 16384 || abs_height > 16384 || !matches!(bits_per_pixel, 1 | 4 | 8 | 16 | 24 | 32) {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    let masks = match (bits_per_pixel, compression) {
        (16, 0) => [0x7c00, 0x03e0, 0x001f],
        (24 | 32, 0) => [0xff_0000, 0xff00, 0xff],
        (16 | 32, 3) => [u32_at(&after, 0).unwrap_or(0xff_0000), u32_at(&after, 4).unwrap_or(0xff00), u32_at(&after, 8).unwrap_or(0xff)],
        (_, 0) => [0, 0, 0],
        _ => return task.fail(cx, ERROR_INVALID_PARAMETER),
    };
    let stride = (width as usize * usize::from(bits_per_pixel)).div_ceil(32) * 4;
    let Some(byte_size) = stride.checked_mul(abs_height as usize) else {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    };
    let (bits, owned_alloc) = if section != 0 {
        let aligned = offset & !0xffff;
        let map_len = (offset - aligned) + byte_size as u64;
        let Reply::Value(base) = super::memapi::map_view_of_file(task, cx, &[section, 0x0002, 0, aligned, map_len]) else {
            return Reply::Value(0);
        };
        if base == 0 {
            return Reply::Value(0);
        }
        (base + (offset - aligned), false)
    } else {
        let alloc_size = (byte_size as u64).max(4096);
        let Some(base) = task.virtual_alloc(cx, 0, alloc_size, 0x3000, 0x04) else {
            return task.fail(cx, 8);
        };
        (base, true)
    };
    if ppv_bits != 0 && task.write_ptr(cx, ppv_bits, bits).is_err() {
        if owned_alloc {
            let _ = task.virtual_free(cx, bits, 0, 0x8000);
        } else {
            let _ = super::memapi::unmap_view_of_file(task, cx, &[bits & !0xffff]);
        }
        return task.fail(cx, 998);
    }
    let pid = task.process.pid;
    let mut w = task.win32k();
    let handle = w.gdi.create_bitmap(display::Surface::new(width, abs_height, 0));
    w.gdi.dibs.insert(handle, DibSection { pid, bits, width, height, bits_per_pixel, compression, stride, masks, owned_alloc });
    Reply::Value(handle)
}

/// Copies a DIB section's guest pixel buffer into its host `Surface`, if `bitmap` is a DIB section of this process.
pub(crate) fn sync_dib_from_guest(task: &mut Task, cx: &mut Cx<'_>, bitmap: u64) {
    let Some(dib) = task.win32k().gdi.dibs.get(&bitmap).copied() else { return };
    if dib.pid != task.process.pid || dib.bits == 0 {
        return;
    }
    let abs_height = dib.height.unsigned_abs() as usize;
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), dib.bits, dib.stride * abs_height)) else { return };
    let fmt = DibFormat { width: dib.width, height: dib.height, bits: dib.bits_per_pixel, masks: dib.masks };
    let mut w = task.win32k();
    let Some(surface) = w.gdi.bitmaps.get_mut(&bitmap) else { return };
    for line in 0..abs_height {
        let row = fmt.row(line, surface.height);
        if !(0..surface.height).contains(&row) {
            continue;
        }
        let src = &raw[line * dib.stride..(line + 1) * dib.stride];
        for x in 0..dib.width.min(surface.width) as usize {
            surface.pixels[row as usize * surface.width as usize + x] = match dib.bits_per_pixel {
                1 => if (src[x / 8] >> (7 - x % 8)) & 1 != 0 { 0xff_ffff } else { 0 },
                8 => {
                    let v = u32::from(src[x]);
                    v << 16 | v << 8 | v
                }
                16 | 24 | 32 => from_dib(src, x, &fmt),
                _ => 0,
            };
        }
    }
}

/// Reads a 32-bit DIB section's guest pixel buffer as top-down `0xAARRGGBB` (preserving alpha for `CreateIconIndirect`).
pub(crate) fn read_dib_argb(task: &mut Task, cx: &mut Cx<'_>, bitmap: u64) -> Option<(i32, i32, Vec<u32>)> {
    let dib = task.win32k().gdi.dibs.get(&bitmap).copied()?;
    if dib.pid != task.process.pid || dib.bits == 0 || dib.bits_per_pixel != 32 {
        return None;
    }
    let abs_height = dib.height.unsigned_abs() as i32;
    let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), dib.bits, dib.stride * abs_height as usize)).ok()?;
    let fmt = DibFormat { width: dib.width, height: dib.height, bits: 32, masks: dib.masks };
    let mut pixels = vec![0u32; (dib.width * abs_height) as usize];
    for line in 0..abs_height as usize {
        let row = fmt.row(line, abs_height);
        if !(0..abs_height).contains(&row) {
            continue;
        }
        let src = &raw[line * dib.stride..(line + 1) * dib.stride];
        for x in 0..dib.width as usize {
            let v = u32::from_le_bytes([src[4 * x], src[4 * x + 1], src[4 * x + 2], src[4 * x + 3]]);
            let rgb = channel(v, fmt.masks[0]) << 16 | channel(v, fmt.masks[1]) << 8 | channel(v, fmt.masks[2]);
            let a = (v >> 24) & 0xff;
            pixels[row as usize * dib.width as usize + x] = a << 24 | rgb;
        }
    }
    Some((dib.width, abs_height, pixels))
}

/// Copies a DIB section's host `Surface` back into its guest pixel buffer after host GDI drawing.
fn sync_dib_to_guest(task: &mut Task, cx: &mut Cx<'_>, bitmap: u64) {
    let (dib, surface) = {
        let w = task.win32k();
        let Some(&dib) = w.gdi.dibs.get(&bitmap) else { return };
        let Some(surface) = w.gdi.bitmaps.get(&bitmap).cloned() else { return };
        (dib, surface)
    };
    if dib.pid != task.process.pid || dib.bits == 0 || !matches!(dib.bits_per_pixel, 16 | 24 | 32) {
        return;
    }
    let abs_height = dib.height.unsigned_abs() as usize;
    let fmt = DibFormat { width: dib.width, height: dib.height, bits: dib.bits_per_pixel, masks: dib.masks };
    let mut out = Vec::with_capacity(abs_height * dib.stride);
    for line in 0..abs_height {
        let row = fmt.row(line, surface.height);
        let start = out.len();
        for x in 0..dib.width {
            let rgb = if (0..surface.height).contains(&row) && x < surface.width { surface.pixels[(row * surface.width + x) as usize] } else { 0 };
            to_dib(rgb & 0xff_ffff, &fmt, &mut out);
        }
        out.resize(start + dib.stride, 0);
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), dib.bits, &out));
}

pub(crate) fn sync_dc_dib_from_guest(task: &mut Task, cx: &mut Cx<'_>, hdc: u64) {
    let bmp = task.win32k().gdi.dcs.get(&hdc).filter(|dc| dc.memory).map(|dc| dc.bitmap);
    if let Some(b) = bmp {
        sync_dib_from_guest(task, cx, b);
    }
}

/// `SetGraphicsMode(hdc, mode)`: `GM_COMPATIBLE` (1) or `GM_ADVANCED` (2). Returns previous mode, or 0 on error.
fn set_graphics_mode(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mode = a[1] as u32 as i32;
    if mode != 1 && mode != 2 {
        return Reply::Value(0);
    }
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    let prev = std::mem::replace(&mut dc.graphics_mode, mode);
    Reply::Value(prev as u32 as u64)
}

fn read_xform(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<[f32; 6]> {
    if at == 0 {
        return None;
    }
    let b = task.mem(cx, false, |t| user::read_vec(&t.space(), at, 24)).ok()?;
    let f = |i: usize| f32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    Some([f(0), f(1), f(2), f(3), f(4), f(5)])
}

fn mul_xform(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// `GetWorldTransform(hdc, &xform)`.
fn get_world_transform(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return Reply::Value(0);
    }
    let Some(xf) = task.win32k().gdi.dcs.get(&a[0]).map(|dc| dc.xform) else { return Reply::Value(0) };
    let bytes: Vec<u8> = xf.iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &bytes)).is_ok()))
}

/// `SetWorldTransform(hdc, &xform)`.
fn set_world_transform(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(xf) = read_xform(task, cx, a[1]) else { return Reply::Value(0) };
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    if dc.graphics_mode != 2 {
        return Reply::Value(0);
    }
    dc.xform = xf;
    Reply::Value(1)
}

/// `ModifyWorldTransform(hdc, &xform, mode)`: `MWT_IDENTITY` (1), `MWT_LEFTMULTIPLY` (2), `MWT_RIGHTMULTIPLY` (3),
/// `MWT_SET` (4).
fn modify_world_transform(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mode = a[2] as u32;
    let xf = if mode == 1 { Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]) } else { read_xform(task, cx, a[1]) };
    let Some(xf) = xf else { return Reply::Value(0) };
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    if dc.graphics_mode != 2 && mode != 1 {
        return Reply::Value(0);
    }
    dc.xform = match mode {
        1 | 4 => xf,
        2 => mul_xform(xf, dc.xform),
        3 => mul_xform(dc.xform, xf),
        _ => return Reply::Value(0),
    };
    Reply::Value(1)
}

/// `SaveDC(hdc)`: saves the DC's drawing state on its stack and returns its 1-based depth.
fn save_dc(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    let snap = crate::win32k::gdi::DcState {
        clip: dc.clip,
        text: dc.text,
        background: dc.background,
        mode: dc.mode,
        brush: dc.brush,
        pen: dc.pen,
        font: dc.font,
        position: dc.position,
        bitmap: dc.bitmap,
        graphics_mode: dc.graphics_mode,
        xform: dc.xform,
    };
    dc.saved.push(snap);
    Reply::Value(dc.saved.len() as u64)
}

/// `RestoreDC(hdc, saved_dc)`: restores a positive 1-based save level or negative relative offset.
fn restore_dc(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let level = a[1] as u32 as i32;
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    let idx = if level < 0 {
        let off = (-level) as usize;
        if off == 0 || off > dc.saved.len() {
            return Reply::Value(0);
        }
        dc.saved.len() - off
    } else if level > 0 {
        let i = (level - 1) as usize;
        if i >= dc.saved.len() {
            return Reply::Value(0);
        }
        i
    } else {
        return Reply::Value(0);
    };
    let snap = dc.saved[idx].clone();
    dc.saved.truncate(idx);
    dc.clip = snap.clip;
    dc.text = snap.text;
    dc.background = snap.background;
    dc.mode = snap.mode;
    dc.brush = snap.brush;
    dc.pen = snap.pen;
    dc.font = snap.font;
    dc.position = snap.position;
    dc.bitmap = snap.bitmap;
    dc.graphics_mode = snap.graphics_mode;
    dc.xform = snap.xform;
    Reply::Value(1)
}

/// `IntersectClipRect(hdc, left, top, right, bottom)`.
fn intersect_clip_rect(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(dc) = w.gdi.dcs.get_mut(&a[0]) else { return Reply::Value(0) };
    let r = Rect::new(a[1] as u32 as i32, a[2] as u32 as i32, a[3] as u32 as i32, a[4] as u32 as i32);
    dc.clip = dc.clip.intersect(&r);
    Reply::Value(region_kind(&dc.clip))
}

/// `GdiAlphaBlend(dst, dx, dy, dw, dh, src, sx, sy, sw, sh, blend)`: copies/scales `src` into `dst` (blending with
/// `SourceConstantAlpha` and per-pixel alpha when `AC_SRC_ALPHA` is set on a 32-bit DIB).
fn gdi_alpha_blend(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (dst, dx, dy, dw, dh) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (src, sx, sy, sw, sh, blend) = (a[5], a[6] as i32, a[7] as i32, a[8] as i32, a[9] as i32, a[10] as u32);
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return Reply::Value(0);
    }
    sync_dc_dib_from_guest(task, cx, src);
    let const_alpha = ((blend >> 16) & 0xff) as u32;
    let src_alpha = (blend >> 24) & 0x01 != 0;
    let src_bmp = task.win32k().gdi.dcs.get(&src).filter(|dc| dc.memory).map(|dc| dc.bitmap);
    let argb = if src_alpha { src_bmp.and_then(|b| read_dib_argb(task, cx, b)) } else { None };
    let surface = {
        let w = task.win32k();
        src_bmp.and_then(|b| w.gdi.bitmaps.get(&b).cloned())
    };
    let Some(surface) = surface else { return Reply::Value(0) };
    let ok = task.win32k().draw(dst, |s, (ox, oy), clip, _| {
        let dest = Rect::new(dx + ox, dy + oy, dx + ox + dw, dy + oy + dh).intersect(clip);
        for y in dest.top..dest.bottom {
            let py = sy + ((y - (dy + oy)) as i64 * sh as i64 / dh as i64) as i32;
            if !(0..surface.height).contains(&py) {
                continue;
            }
            for x in dest.left..dest.right {
                let px = sx + ((x - (dx + ox)) as i64 * sw as i64 / dw as i64) as i32;
                if !(0..surface.width).contains(&px) {
                    continue;
                }
                let si = (py * surface.width + px) as usize;
                let di = (y * s.width + x) as usize;
                let (sr, sg, sb, sa) = if let Some((_, _, ref pix)) = argb {
                    let p = pix[si];
                    ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff, ((p >> 24) & 0xff) * const_alpha / 255)
                } else {
                    let p = surface.pixels[si];
                    (((p >> 16) & 0xff) * const_alpha / 255, ((p >> 8) & 0xff) * const_alpha / 255, (p & 0xff) * const_alpha / 255, const_alpha)
                };
                let dst_p = s.pixels[di];
                let inv = 255 - sa;
                let r = (sr + ((dst_p >> 16) & 0xff) * inv / 255).min(255);
                let g = (sg + ((dst_p >> 8) & 0xff) * inv / 255).min(255);
                let b = (sb + (dst_p & 0xff) * inv / 255).min(255);
                s.pixels[di] = r << 16 | g << 8 | b;
            }
        }
    });
    if ok {
        sync_dc_dib_to_guest(task, cx, dst);
    }
    Reply::Value(u64::from(ok))
}

fn sync_dc_dib_to_guest(task: &mut Task, cx: &mut Cx<'_>, hdc: u64) {
    let bmp = task.win32k().gdi.dcs.get(&hdc).filter(|dc| dc.memory).map(|dc| dc.bitmap);
    if let Some(b) = bmp {
        sync_dib_to_guest(task, cx, b);
    }
}

/// `StretchDIBits(hdc, xDest, yDest, wDest, hDest, xSrc, ySrc, wSrc, hSrc, bits, &bmi, usage, rop)`.
fn stretch_di_bits(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (hdc, dx, dy, dw, dh) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (sx, sy, sw, sh) = (a[5] as i32, a[6] as i32, a[7] as i32, a[8] as i32);
    let (bits_at, info_at, rop) = (a[9], a[10], a[12] as u32);
    if bits_at == 0 || info_at == 0 || dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return Reply::Value(0);
    }
    let Some((header, after)) = read_info(task, cx, info_at) else { return Reply::Value(0) };
    let Some(f) = DibFormat::parse(&header, &after) else { return Reply::Value(0) };
    let abs_h = f.height.unsigned_abs() as i32;
    let Ok(bits) = task.mem(cx, false, |t| user::read_vec(&t.space(), bits_at, f.stride() * abs_h as usize)) else { return Reply::Value(0) };
    let mut src_surf = display::Surface::new(f.width, abs_h, 0);
    for line in 0..abs_h as usize {
        let row = f.row(line, abs_h);
        if !(0..abs_h).contains(&row) {
            continue;
        }
        let slice = &bits[line * f.stride()..(line + 1) * f.stride()];
        for x in 0..f.width as usize {
            src_surf.pixels[row as usize * f.width as usize + x] = from_dib(slice, x, &f);
        }
    }
    let mut sub = display::Surface::new(sw, sh, 0);
    let all = sub.bounds();
    let src_rect = Rect::new(sx, sy, sx + sw, sy + sh);
    crate::win32k::image::copy(&mut sub, 0, 0, &src_surf, &src_rect, 0xcc_0020, &all);
    let scaled = if (dw, dh) != (sw, sh) {
        crate::win32k::image::Image::from_surface(&sub).scaled(dw, dh).to_surface()
    } else {
        sub
    };
    let ok = task.win32k().draw(hdc, |s, (ox, oy), clip, _| {
        let b = scaled.bounds();
        crate::win32k::image::copy(s, dx + ox, dy + oy, &scaled, &b, rop, clip);
    });
    if ok {
        sync_dc_dib_to_guest(task, cx, hdc);
        Reply::Value(abs_h as u64)
    } else {
        Reply::Value(0)
    }
}

/// `SetDIBitsToDevice(hdc, xDest, yDest, w, h, xSrc, ySrc, startScan, cLines, bits, &bmi, usage)`.
fn set_di_bits_to_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (hdc, dx, dy, w, h, sx, sy) = (a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32, a[6] as i32);
    let (start_scan, c_lines, bits_at, info_at) = (a[7] as u32 as usize, a[8] as u32 as usize, a[9], a[10]);
    if bits_at == 0 || info_at == 0 || w <= 0 || h <= 0 || c_lines == 0 {
        return Reply::Value(0);
    }
    let Some((header, after)) = read_info(task, cx, info_at) else { return Reply::Value(0) };
    let Some(f) = DibFormat::parse(&header, &after) else { return Reply::Value(0) };
    let abs_h = f.height.unsigned_abs() as i32;
    let lines = c_lines.min((abs_h as usize).saturating_sub(start_scan));
    let Ok(bits) = task.mem(cx, false, |t| user::read_vec(&t.space(), bits_at, f.stride() * lines)) else { return Reply::Value(0) };
    let mut src_surf = display::Surface::new(f.width, abs_h, 0);
    for i in 0..lines {
        let row = f.row(start_scan + i, abs_h);
        if !(0..abs_h).contains(&row) {
            continue;
        }
        let slice = &bits[i * f.stride()..(i + 1) * f.stride()];
        for x in 0..f.width as usize {
            src_surf.pixels[row as usize * f.width as usize + x] = from_dib(slice, x, &f);
        }
    }
    let src_rect = Rect::new(sx, sy, sx + w, sy + h);
    let ok = task.win32k().draw(hdc, |s, (ox, oy), clip, _| {
        crate::win32k::image::copy(s, dx + ox, dy + oy, &src_surf, &src_rect, 0xcc_0020, clip);
    });
    if ok {
        sync_dc_dib_to_guest(task, cx, hdc);
        Reply::Value(lines as u64)
    } else {
        Reply::Value(0)
    }
}

#[cfg(test)]
mod dib_tests {
    use super::*;

    fn header(width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut h = vec![0u8; 40];
        h[0..4].copy_from_slice(&40u32.to_le_bytes());
        h[4..8].copy_from_slice(&width.to_le_bytes());
        h[8..12].copy_from_slice(&height.to_le_bytes());
        h[14..16].copy_from_slice(&bits.to_le_bytes());
        h[16..20].copy_from_slice(&compression.to_le_bytes());
        h
    }

    #[test]
    fn pixels_both_ways() {
        let rgb = 0x12_80_f0;
        for (bits, compression, masks) in [(32, 0, vec![]), (24, 0, vec![]), (16, 0, vec![]), (16, 3, [0xf800u32, 0x07e0, 0x001f].iter().flat_map(|m| m.to_le_bytes()).collect())] {
            let f = DibFormat::parse(&header(3, 2, bits, compression), &masks).unwrap();
            let mut row = Vec::new();
            to_dib(rgb, &f, &mut row);
            let back = from_dib(&row, 0, &f);
            // 16 bits keep five or six of each channel's eight.
            let close = |a: u32, b: u32, s: u32| (a >> s & 0xff).abs_diff(b >> s & 0xff) <= 8;
            assert!(close(back, rgb, 16) && close(back, rgb, 8) && close(back, rgb, 0), "{bits} bits: {back:06x}");
        }
        let f = DibFormat::parse(&header(3, 2, 32, 0), &[]).unwrap();
        let mut row = Vec::new();
        to_dib(rgb, &f, &mut row);
        assert_eq!(row, [0xf0, 0x80, 0x12, 0]);
        assert_eq!((f.stride(), DibFormat::parse(&header(3, 2, 24, 0), &[]).unwrap().stride()), (12, 12));
        // Bottom-up: line 0 is the last row; top-down (negative height): the first.
        assert_eq!((f.row(0, 2), DibFormat::parse(&header(3, -2, 32, 0), &[]).unwrap().row(0, 2)), (1, 0));
        assert!(DibFormat::parse(&header(3, 2, 8, 0), &[]).is_none());
    }
}
