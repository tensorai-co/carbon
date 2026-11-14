//! dxgi.dll: there's no DXGI on the machine yet (no adapters, and no Direct3D for one to drive), so making a factory
//! fails with DXGI_ERROR_UNSUPPORTED, as on a Windows without it; programs then draw without the GPU.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "DXGI.dll";
    host CreateDXGIFactory(2) => create_factory;
    host CreateDXGIFactory1(2) => create_factory;
}

const DXGI_ERROR_UNSUPPORTED: u64 = 0x887a_0004;

/// `CreateDXGIFactory(&iid, &factory)` and `CreateDXGIFactory1`: none, and the pointer NULL.
fn create_factory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(DXGI_ERROR_UNSUPPORTED)
}
