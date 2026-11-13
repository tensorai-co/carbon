//! dwrite.dll: DirectWrite isn't there yet. `DWriteCreateFactory` says so (E_NOTIMPL, and no factory), as on a
//! Windows without it, so programs draw text with GDI instead.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "DWRITE.dll";
    host DWriteCreateFactory(3) => d_write_create_factory;
}

fn d_write_create_factory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0 {
        let _ = task.write_ptr(cx, a[2], 0);
    }
    Reply::Value(0x8000_4001)
}
