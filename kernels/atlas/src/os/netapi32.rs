//! netapi32.dll: the machine as a workstation in the WORKGROUP workgroup, not in a domain or joined to a directory;
//! accounts' details aren't there yet.

use kernels_core::thread::Cx;

use super::kernel32::{global_new, global_release};
use super::{Reply, dll};
use crate::process::Task;

dll! {
    "NETAPI32.dll";
    host DsRoleFreeMemory(1) => free_buffer;
    host DsRoleGetPrimaryDomainInformation(3) => ds_role_get_primary_domain_information;
    host NetApiBufferFree(1) => free_buffer;
    host NetFreeAadJoinInformation(1) => zero;
    host NetGetAadJoinInformation(2) => net_get_aad_join_information;
    host NetGetJoinInformation(3) => net_get_join_information;
    missing NetUserGetInfo;
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `NetApiBufferFree(buffer)` and `DsRoleFreeMemory`: a buffer these gave.
fn free_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 {
        global_release(task, cx, a[0]);
    }
    Reply::Value(0)
}

/// `NetGetAadJoinInformation(tenant, &info)`: not joined, so no information.
fn net_get_aad_join_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(0)
}

/// `NetGetJoinInformation(server, &name, &status)`: the workgroup's name, and NetSetupWorkgroupName (3).
fn net_get_join_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name: Vec<u8> = "WORKGROUP".encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let block = global_new(task, cx, &name);
    if block == 0 {
        return Reply::Value(8);
    }
    let _ = task.write_ptr(cx, a[1], block);
    let _ = task.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), a[2], 3));
    Reply::Value(0)
}

/// `DsRoleGetPrimaryDomainInformation(server, level, &buffer)`: DsRolePrimaryDomainInfoBasic (1), a standalone
/// workstation in the workgroup, with no DNS domain, forest or GUID.
fn ds_role_get_primary_domain_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 1 {
        return Reply::Value(87);
    }
    let w = task.abi().width as usize;
    let head = (8 + 3 * w + 16).next_multiple_of(w);
    let name: Vec<u8> = "WORKGROUP".encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let mut b = vec![0u8; head];
    b.extend(&name);
    let block = global_new(task, cx, &b);
    if block == 0 {
        return Reply::Value(8);
    }
    let _ = task.write_ptr(cx, block + 8, block + head as u64);
    let _ = task.write_ptr(cx, a[2], block);
    Reply::Value(0)
}
