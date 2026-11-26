//! urlmon.dll: URL moniker security manager stub.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "URLMON.dll";
    host CoInternetCreateSecurityManager(3) => not_impl;
}

fn not_impl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(0x8000_4001) // E_NOTIMPL
}
