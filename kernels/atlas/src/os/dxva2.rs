//! dxva2.dll: DirectX Video Acceleration 2 (not available).

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "DXVA2.dll";
    host DXVA2CreateVideoService(3) => not_impl;
    host DXVA2CreateDirect3DDeviceManager9(2) => not_impl;
}

fn not_impl(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0x8000_4001) // E_NOTIMPL
}
