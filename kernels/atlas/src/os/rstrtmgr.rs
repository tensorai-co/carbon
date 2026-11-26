//! rstrtmgr.dll: Restart Manager API. Reports no processes holding locks on registered resources.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "RstrtMgr.dll";
    host RmEndSession(1) => rm_end_session;
    host RmGetList(5) => rm_get_list;
    host RmRegisterResources(7) => rm_register_resources;
    host RmStartSession(3) => rm_start_session;
}

const SESSION: u32 = 1;

fn rm_start_session(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[0], SESSION));
    }
    if a[2] != 0 {
        let key: Vec<u8> = "00000000000000000000000000000001\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &key));
    }
    Reply::Value(0)
}

fn rm_register_resources(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn rm_get_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    for ptr in [a[1], a[2], a[4]] {
        if ptr != 0 {
            let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), ptr, 0));
        }
    }
    Reply::Value(0)
}

fn rm_end_session(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}
