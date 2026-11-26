//! d3d9.dll: Direct3D 9 entry points reporting no D3D9 device so graphics callers select D3D11.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "D3D9.dll";
    host D3DPERF_BeginEvent(2) => zero;
    host D3DPERF_EndEvent(0) => zero;
    host D3DPERF_GetStatus(0) => zero;
    host D3DPERF_SetMarker(2) => zero;
    host Direct3DCreate9(1) => zero;
    host Direct3DCreate9Ex(2) => create9_ex;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn create9_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(0x8876_086A) // D3DERR_NOTAVAILABLE
}
