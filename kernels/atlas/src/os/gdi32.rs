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
    host CreatePen(3) => create_pen;
    host CreateFontW(14) => create_font;
    host CreateFontIndirectW(1) => create_font;
    host SelectObject(2) => select_object;
    host DeleteObject(1) => delete_object;
    host SetTextColor(2) => set_text_color;
    host SetBkColor(2) => set_bk_color;
    host SetBkMode(2) => set_bk_mode;
    host TextOutW(5) => text_out_w;
    host Rectangle(5) => rectangle;
    host MoveToEx(4) => move_to_ex;
    host LineTo(3) => line_to;
    host SetPixel(4) => set_pixel;
    host GetPixel(3) => get_pixel;
    host GetTextExtentPoint32W(4) => get_text_extent_point32_w;
    host GetDeviceCaps(2) => get_device_caps;
    missing BitBlt;
    missing CreateCompatibleDC;
    missing CreateCompatibleBitmap;
    missing CreateDIBSection;
    missing StretchDIBits;
}

const CLR_INVALID: u64 = 0xffff_ffff;

fn get_stock_object(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(Gdi::stock_object(a[0]))
}

fn create_solid_brush(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().gdi.create(Object::Brush(Some(swap(a[0] as u32)))))
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
