//! shlwapi.dll: paths; the rest is declared so programs that import it load, and says it isn't written yet.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "SHLWAPI.dll";
    host PathFileExistsW(1) => path_file_exists_w;
    missing SHDeleteKeyA;
    host PathFileExistsA(1) => path_file_exists_a;
    // Not written yet: what larger programs import.
    missing AssocQueryStringW;
    missing PathFindExtensionW;
    missing PathFindFileNameW;
    missing PathMatchSpecW;
    missing PathRemoveExtensionW;
}

/// `PathFileExistsW(path)` and `A`: whether there's a file or folder there.
fn path_file_exists(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let found = task.path_arg(cx, a[0], wide).is_some_and(|p: String| task.path_info(&p).is_ok());
    Reply::Value(u64::from(found))
}

fn path_file_exists_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_file_exists(task, cx, a, true)
}

fn path_file_exists_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_file_exists(task, cx, a, false)
}
