//! comdlg32.dll: common dialog boxes.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "COMDLG32.dll";
    host ChooseColorW(1) => zero;
    host ChooseFontW(1) => zero;
    host CommDlgExtendedError(0) => zero;
    missing GetOpenFileNameW;
    missing GetSaveFileNameW;
    // Not written yet: what larger programs import.
    missing PrintDlgExW;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}
