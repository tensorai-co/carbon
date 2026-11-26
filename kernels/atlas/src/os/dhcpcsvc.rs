//! dhcpcsvc.dll: DHCP client API (no DHCP option data provided).

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "DHCPCSVC.dll";
    host DhcpCApiInitialize(1) => init;
    host DhcpRequestParams(10) => request_params;
}

fn init(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[0], 2));
    }
    Reply::Value(0)
}

fn request_params(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(2) // ERROR_FILE_NOT_FOUND
}
