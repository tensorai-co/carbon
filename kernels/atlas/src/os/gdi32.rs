//! gdi32.dll: drawing, over `win32k::gdi`, which draws host-side into windows' surfaces.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::{Rect, font};

use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::gdi::{Gdi, Object, swap};

dll! {
    "GDI32.dll";
    host GetStockObject(1) => get_stock_object;
    host CreateSolidBrush(1) => create_solid_brush;
    host CreateBrushIndirect(1) => create_brush_indirect;
    host CreatePen(3) => create_pen;
    host CreateFontW(14) => create_font;
    host CreateFontIndirectW(1) => create_font;
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
    host GetObjectW(3) => get_object;
    host GetObjectA(3) => get_object;
    host SetStretchBltMode(2) => set_stretch_blt_mode;
    missing CreateDIBSection;
    missing StretchDIBits;
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
    host CreateFontA(14) => create_font;
    host CreateFontIndirectA(1) => create_font;
    host CreateRectRgn(4) => create_rect_rgn;
    missing DescribePixelFormat;
    host EnumFontFamiliesExA(5) => enum_font_families_ex_a;
    host EnumFontFamiliesExW(5) => enum_font_families_ex_w;
    missing ExtTextOutA;
    missing ExtTextOutW;
    host GdiFlush(0) => one;
    missing GetBitmapBits;
    missing GetCharABCWidthsW;
    host GetClipBox(2) => get_clip_box;
    host GetCurrentObject(2) => get_current_object;
    missing GetDIBits;
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
    missing SetDIBitsToDevice;
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
    missing CreateRectRgnIndirect;
    missing DeleteEnhMetaFile;
    missing EndDoc;
    missing EndPage;
    missing EndPath;
    missing EnumEnhMetaFile;
    missing EqualRgn;
    missing ExtCreatePen;
    missing ExtEscape;
    missing FillPath;
    missing GdiAlphaBlend;
    missing GdiComment;
    missing GetCharWidthW;
    missing GetClipRgn;
    missing GetEnhMetaFileBits;
    missing GetEnhMetaFileHeader;
    missing GetObjectType;
    missing GetOutlineTextMetricsW;
    missing GetRegionData;
    missing GetRgnBox;
    missing GetTextFaceA;
    missing GetWorldTransform;
    missing IntersectClipRect;
    missing ModifyWorldTransform;
    missing PlayEnhMetaFile;
    missing PlayEnhMetaFileRecord;
    missing PolyBezierTo;
    missing PtInRegion;
    missing RestoreDC;
    missing SaveDC;
    missing SelectClipPath;
    missing SetAbortProc;
    missing SetArcDirection;
    missing SetDCPenColor;
    missing SetDIBits;
    missing SetEnhMetaFileBits;
    missing SetGraphicsMode;
    missing SetMiterLimit;
    missing SetPolyFillMode;
    missing SetROP2;
    missing SetWorldTransform;
    missing StartDocW;
    missing StartPage;
    missing StrokeAndFillPath;
    missing StrokePath;
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
fn create_font(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.create(Object::Font))
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

/// `GetObject(object, size, &out)`: a bitmap's BITMAP (type, width, height, bytes a row, planes, bits a pixel, no
/// bits of its own here); the size it needs with no buffer. Other objects aren't described yet.
fn get_object(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.abi().width as usize;
    let size = if w == 8 { 32 } else { 24 };
    let Some((width, height)) = task.win32k().gdi.bitmaps.get(&a[0]).map(|s| (s.width, s.height)) else { return Reply::Value(0) };
    if a[2] == 0 {
        return Reply::Value(size as u64);
    }
    let mut b = vec![0u8; size];
    for (o, v) in [(4, width), (8, height), (12, width * 4)] {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[16..18].copy_from_slice(&1u16.to_le_bytes());
    b[18..20].copy_from_slice(&32u16.to_le_bytes());
    let n = size.min(a[1] as u32 as usize);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b[..n])).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(n as u64)
}

/// `BitBlt(dst, x, y, cx, cy, src, sx, sy, rop)`.
fn bit_blt(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y, w, h) = (a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (sx, sy) = (a[6] as i32, a[7] as i32);
    let to = Rect::new(x, y, x + w, y + h);
    let from = Rect::new(sx, sy, sx + w, sy + h);
    Reply::Value(u64::from(task.win32k().blit(a[0], to, a[5], from, a[8] as u32)))
}

/// `StretchBlt(dst, x, y, cx, cy, src, sx, sy, scx, scy, rop)`: scaled, each pixel the nearest.
fn stretch_blt(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y, w, h) = (a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32);
    let (sx, sy, sw, sh) = (a[6] as i32, a[7] as i32, a[8] as i32, a[9] as i32);
    let to = Rect::new(x, y, x + w, y + h);
    let from = Rect::new(sx, sy, sx + sw, sy + sh);
    Reply::Value(u64::from(task.win32k().blit(a[0], to, a[5], from, a[10] as u32)))
}

/// `SetStretchBltMode`: stretching is always to the nearest pixel; the mode before, BLACKONWHITE.
fn set_stretch_blt_mode(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn select_object(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.select(a[0], a[1]).unwrap_or(0))
}

fn delete_object(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().gdi.delete(a[0])))
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

/// `SetPixelFormat(dc, format, &descriptor)`: format 1 is taken; there's no OpenGL to draw with it yet.
fn set_pixel_format(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[1] as u32 == 1))
}

/// `SwapBuffers(dc)`: nothing drawn behind to show.
fn swap_buffers(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
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
