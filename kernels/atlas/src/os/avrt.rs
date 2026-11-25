//! avrt.dll: Multimedia Class Scheduler Service (MMCSS) thread priority registration.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "AVRT.dll";
    host AvRevertMmThreadCharacteristics(1) => av_revert;
    host AvSetMmThreadCharacteristicsW(2) => av_set;
}

const MMCSS_HANDLE: u64 = 0xaa00;

/// `AvSetMmThreadCharacteristicsW(task_name, &task_index)`.
fn av_set(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &1u32.to_le_bytes()));
    }
    Reply::Value(MMCSS_HANDLE)
}

/// `AvRevertMmThreadCharacteristics(handle)`.
fn av_revert(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] == MMCSS_HANDLE))
}
