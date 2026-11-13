//! comctl32.dll: the common controls (list views, progress bars, image lists, property sheets). None is written yet:
//! `InitCommonControls(Ex)` succeeds, registering nothing, so a program that calls it first starts; what it uses
//! after says it isn't written. Its exports are numbered from 17, InitCommonControls' ordinal, which programs import
//! it by.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "COMCTL32.dll" @ 17;
    host InitCommonControls(0) => init_common_controls;
    host InitCommonControlsEx(1) => init_common_controls_ex;
    missing ImageList_Create;
    missing ImageList_Destroy;
    missing ImageList_Add;
    missing ImageList_AddMasked;
    missing ImageList_ReplaceIcon;
    missing ImageList_Draw;
    missing CreatePropertySheetPageW;
    missing PropertySheetW;
    missing TaskDialogIndirect;
}

/// `InitCommonControls()`: nothing to register yet.
fn init_common_controls(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `InitCommonControlsEx(&init)`: TRUE.
fn init_common_controls_ex(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}
