//! wtsapi32.dll: Remote Desktop's sessions, as a desktop program asks about them: there's one, the console's, and a
//! window registered for its notifications never gets one (it never changes).

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WTSAPI32.dll";
    host WTSRegisterSessionNotification(2) => accept;
    host WTSUnRegisterSessionNotification(1) => accept;
    host WTSFreeMemory(1) => wts_free_memory;
    missing WTSQuerySessionInformationW;
}

fn accept(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

/// `WTSFreeMemory(block)`: what a WTS call gave, back to the heap.
fn wts_free_memory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.task_free(cx, a[0]) {
        Ok(()) => Reply::Value(0),
        Err(reply) => reply,
    }
}
