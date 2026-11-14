//! wevtapi.dll: the event log, which a machine doesn't keep. A query names a channel or a log file that isn't there,
//! so it fails as Windows does for one; there's never a handle to read events from, render or close.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WEVTAPI.dll";
    host EvtClose(1) => evt_close;
    host EvtCreateRenderContext(3) => evt_no_handle;
    host EvtNext(6) => evt_no_more;
    host EvtQuery(4) => evt_no_handle;
    host EvtRender(7) => evt_no_more;
}

/// ERROR_EVT_CHANNEL_NOT_FOUND, ERROR_NO_MORE_ITEMS and ERROR_INVALID_HANDLE.
const ERROR_EVT_CHANNEL_NOT_FOUND: u32 = 15007;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_INVALID_HANDLE: u32 = 6;

/// `EvtQuery(session, path, query, flags)` and `EvtCreateRenderContext`: no handle, as the channel isn't there.
fn evt_no_handle(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.fail(cx, ERROR_EVT_CHANNEL_NOT_FOUND)
}

/// `EvtNext(results, ...)` and `EvtRender`: there are no events, from a handle atlas never gave.
fn evt_no_more(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.fail(cx, if a[0] == 0 { ERROR_INVALID_HANDLE } else { ERROR_NO_MORE_ITEMS })
}

/// `EvtClose(handle)`: nothing to let go of.
fn evt_close(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    }
    Reply::Value(1)
}
