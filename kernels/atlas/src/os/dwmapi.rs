//! dwmapi.dll: the desktop window manager, as programs ask it: composition is on; windows' attributes are taken,
//! and their frame's bounds are their rectangles.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "DWMAPI.dll";
    host DwmExtendFrameIntoClientArea(2) => s_ok;
    host DwmSetWindowAttribute(4) => s_ok;
    // Not written yet: what larger programs import.
    host DwmDefWindowProc(5) => zero;
    host DwmGetCompositionTimingInfo(2) => not_implemented;
    host DwmGetWindowAttribute(4) => dwm_get_window_attribute;
    host DwmIsCompositionEnabled(1) => dwm_is_composition_enabled;
    host DwmFlush(0) => s_ok;
}

fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn not_implemented(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0x8000_4001)
}

/// `DwmIsCompositionEnabled(&enabled)`: it is.
fn dwm_is_composition_enabled(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[0], 1)).is_ok() { 0 } else { 0x8007_0057 })
}

/// `DwmGetWindowAttribute(window, attribute, &value, size)`: DWMWA_EXTENDED_FRAME_BOUNDS (9) is its rectangle on the
/// screen; DWMWA_CLOAKED (14) and the rest are 0.
fn dwm_get_window_attribute(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] as u32 == 9 {
        return match super::user32::window_rect_on_screen(task, a[0]) {
            Some(r) if task.write_rect(cx, a[2], &r) => Reply::Value(0),
            _ => Reply::Value(0x8007_0057),
        };
    }
    let zeros = vec![0u8; (a[3] as u32 as usize).min(64)];
    Reply::Value(if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &zeros)).is_ok() { 0 } else { 0x8007_0057 })
}
