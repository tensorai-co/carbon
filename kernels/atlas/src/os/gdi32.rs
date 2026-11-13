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
