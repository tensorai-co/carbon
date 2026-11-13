//! Event tracing's provider side (advapi32's `EventRegister` and the older `RegisterTraceGuids`): providers register
//! and write, and since no trace session ever runs on the machine, nothing enables them and what they write goes
//! nowhere, as on a Windows where nobody is tracing. A registration handle is always 64 bits, on every ISA.

use std::sync::atomic::{AtomicU64, Ordering};

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;

const ERROR_INVALID_PARAMETER: u64 = 87;

/// Registration handles: any nonzero number a provider gives back.
static NEXT: AtomicU64 = AtomicU64::new(0x7700_0001);

fn new_handle() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn put_u64(task: &mut Task, cx: &mut Cx<'_>, at: u64, v: u64) -> bool {
    task.mem(cx, true, |t| user::write_u64(&mut t.space(), at, v)).is_ok()
}

/// `EventRegister(&provider, callback, context, &handle)`: never enabled, so the callback is never called.
pub fn event_register(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || a[3] == 0 || !put_u64(task, cx, a[3], new_handle()) {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    Reply::Value(0)
}

/// `RegisterTraceGuidsW(callback, context, &control, count, registrations, image, resource, &handle)` and `A`: each
/// TRACE_GUID_REGISTRATION ({ guid, handle }) gets a handle too.
pub fn register_trace_guids(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || a[2] == 0 || a[7] == 0 || !put_u64(task, cx, a[7], new_handle()) {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let width = task.abi().width;
    for i in 0..a[3].min(1024) {
        let at = a[4] + i * 2 * width + width;
        let _ = task.write_ptr(cx, at, new_handle());
    }
    Reply::Value(0)
}

/// `GetTraceLoggerHandle(buffer)`: only a session's control callback has one, and none calls.
pub fn get_trace_logger_handle(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.set_last_error(cx, 6);
    Reply::Value(u64::MAX)
}
