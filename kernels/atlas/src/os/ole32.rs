//! ole32.dll: COM and OLE, as far as programs that only start them need. Apartments: `CoInitialize(Ex)` and
//! `OleInitialize` succeed and their `Uninitialize`s do nothing, as nothing is kept per apartment yet. The task
//! allocator is guest code, on the process heap. No class is registered, so `CoCreateInstance` says so, as Windows
//! does for a class it doesn't know, and a program falls back as it would there (an installer
//! skips the shortcut it was making).

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "ole32.dll";
    host CoInitialize(1) => co_initialize;
    host CoInitializeEx(2) => co_initialize;
    host CoUninitialize(0) => nothing;
    host CoCreateInstance(5) => co_create_instance;
    guest CoTaskMemAlloc => AtlasCoTaskMemAlloc;
    guest CoTaskMemRealloc => AtlasCoTaskMemRealloc;
    guest CoTaskMemFree => AtlasCoTaskMemFree;
    host OleInitialize(1) => co_initialize;
    host OleUninitialize(0) => nothing;
}

const S_OK: u64 = 0;
/// REGDB_E_CLASSNOTREG.
const CLASS_NOT_REGISTERED: u64 = 0x8004_0154;

/// `CoInitialize(reserved)`, `CoInitializeEx(reserved, model)` and `OleInitialize(reserved)`: S_OK.
fn co_initialize(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

fn nothing(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `CoCreateInstance(&clsid, outer, context, &iid, &object)`: no class is registered, so `*object` is null and the
/// result REGDB_E_CLASSNOTREG.
fn co_create_instance(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[4] != 0 {
        let _ = task.write_ptr(cx, a[4], 0);
    }
    Reply::Value(CLASS_NOT_REGISTERED)
}
