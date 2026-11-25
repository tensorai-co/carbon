//! uxtheme.dll: desktop visual styles. Atlas has no theme engine (classic look), so `OpenThemeData` returns NULL and
//! operations on a theme handle report `E_HANDLE`.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "UXTHEME.dll";
    host CloseThemeData(1) => s_ok;
    host DrawThemeBackground(6) => e_handle;
    host GetThemeBackgroundContentRect(6) => e_handle;
    host GetThemePartSize(7) => e_handle;
    host OpenThemeData(2) => null;
}

const S_OK: u64 = 0;
const E_HANDLE: u64 = 0x8007_0006;

fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

fn e_handle(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(E_HANDLE)
}

fn null(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}
