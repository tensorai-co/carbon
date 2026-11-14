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
    host WTSQuerySessionInformationW(5) => wts_query_session_information_w;
}

/// `WTSQuerySessionInformationW(server, session, class, &buffer, &bytes)`: the console session's (1, or the
/// current one, -1): its ID, user and domain, that it's connected and active, and that it's the console (protocol 0).
/// The buffer is for `WTSFreeMemory`.
fn wts_query_session_information_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const ERROR_INVALID_PARAMETER: u32 = 87;
    let session = a[1] as u32;
    if session != 1 && session != u32::MAX {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    let wide = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() };
    let data: Vec<u8> = match a[2] as u32 {
        4 => 1u32.to_le_bytes().to_vec(),                            // WTSSessionId
        5 => wide("carbon"),                                         // WTSUserName
        6 => wide("Console"),                                        // WTSWinStationName
        7 => wide(crate::os::sysapi::computer_name()),               // WTSDomainName
        8 => 0u32.to_le_bytes().to_vec(),                            // WTSConnectState: WTSActive
        10 => wide(crate::os::sysapi::computer_name()),              // WTSClientName
        16 => 0u16.to_le_bytes().to_vec(),                           // WTSClientProtocolType: the console
        0..=3 => wide(""),                                          // the initial program, its folder, the OEM ID
        _ => return task.fail(cx, ERROR_INVALID_PARAMETER),
    };
    let block = match task.task_alloc(cx, &data) {
        Ok(b) => b,
        Err(reply) => return reply,
    };
    let _ = task.write_ptr(cx, a[3], block);
    if a[4] != 0 {
        let _ = task.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), a[4], data.len() as u32));
    }
    Reply::Value(1)
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
