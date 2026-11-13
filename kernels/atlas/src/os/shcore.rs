//! shcore.dll: DPI. Every monitor is 96 DPI (100%), and the process is told what awareness it asks for.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "SHCORE.dll";
    host GetDpiForMonitor(4) => get_dpi_for_monitor;
    host SetProcessDpiAwareness(1) => s_ok;
    host GetProcessDpiAwareness(2) => get_process_dpi_awareness;
    host GetScaleFactorForMonitor(2) => get_scale_factor_for_monitor;
}

const E_INVALIDARG: u64 = 0x8007_0057;

fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `GetDpiForMonitor(monitor, type, &x, &y)`: 96 by 96.
fn get_dpi_for_monitor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let wrote = task.mem(cx, true, |t| {
        user::write_u32(&mut t.space(), a[2], 96)?;
        user::write_u32(&mut t.space(), a[3], 96)
    });
    Reply::Value(if wrote.is_ok() { 0 } else { E_INVALIDARG })
}

/// `GetProcessDpiAwareness(process, &awareness)`: PROCESS_PER_MONITOR_DPI_AWARE (2), which is right at 96 DPI.
fn get_process_dpi_awareness(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 2)).is_ok() { 0 } else { E_INVALIDARG })
}

/// `GetScaleFactorForMonitor(monitor, &scale)`: SCALE_100_PERCENT.
fn get_scale_factor_for_monitor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 100)).is_ok() { 0 } else { E_INVALIDARG })
}
