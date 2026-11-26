//! d3d12.dll: Direct3D 12 entry point returning `DXGI_ERROR_UNSUPPORTED` so callers fall back to D3D11/OpenGL.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "D3D12.dll";
    host D3D12CreateDevice(4) @ 101 => d3d12_create_device;
}

fn d3d12_create_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] != 0 {
        let _ = task.write_ptr(cx, a[3], 0);
    }
    Reply::Value(0x887a_0004) // DXGI_ERROR_UNSUPPORTED
}
