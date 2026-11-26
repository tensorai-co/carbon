//! secur32.dll: SSPI interface table and clean unsupported-function replies when no TLS/SSPI session is implemented.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "SECUR32.dll";
    host AcceptSecurityContext(9) => unsupported;
    host AcquireCredentialsHandleA(9) => unsupported;
    host ApplyControlToken(2) => unsupported;
    host CompleteAuthToken(2) => unsupported;
    host DecryptMessage(4) => unsupported;
    host DeleteSecurityContext(1) => sec_ok;
    host EncryptMessage(4) => unsupported;
    host FreeContextBuffer(1) => sec_ok;
    host FreeCredentialsHandle(1) => sec_ok;
    host InitializeSecurityContextA(12) => unsupported;
    host QueryContextAttributesA(3) => unsupported;
    host SetContextAttributesA(4) => unsupported;
    host AcquireCredentialsHandleW(9) => unsupported;
    host GetUserNameExW(3) => get_user_name_ex_w;
    host InitializeSecurityContextW(12) => unsupported;
    missing LsaConnectUntrusted;
    missing LsaDeregisterLogonProcess;
    missing LsaFreeReturnBuffer;
    missing LsaLogonUser;
    host QueryContextAttributesW(3) => unsupported;
    host QuerySecurityPackageInfoW(2) => unsupported;
    host InitSecurityInterfaceA(0) => init_security_interface_a;
}

const SEC_E_OK: u64 = 0;
const SEC_E_UNSUPPORTED_FUNCTION: u64 = 0x8009_0302;

fn sec_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(SEC_E_OK)
}

fn unsupported(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(SEC_E_UNSUPPORTED_FUNCTION)
}

/// `InitSecurityInterfaceA()`: allocates and populates a `SecurityFunctionTableA` in guest memory.
fn init_security_interface_a(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    let abi = task.abi();
    let w = abi.width as usize;
    let total = w + 28 * w; // dwVersion (+ pad) + 28 function pointer slots
    let Some(base) = task.virtual_alloc(cx, 0, total as u64, 0x3000, 0x04) else {
        return Reply::Value(0);
    };
    let mut buf = vec![0u8; total];
    let entries: [(usize, &str); 12] = [
        (2, "AcquireCredentialsHandleA"),
        (3, "FreeCredentialsHandle"),
        (5, "InitializeSecurityContextA"),
        (6, "AcceptSecurityContext"),
        (7, "CompleteAuthToken"),
        (8, "DeleteSecurityContext"),
        (9, "ApplyControlToken"),
        (10, "QueryContextAttributesA"),
        (15, "FreeContextBuffer"),
        (24, "EncryptMessage"),
        (25, "DecryptMessage"),
        (26, "SetContextAttributesA"),
    ];
    for (slot, name) in entries {
        if let Some(addr) = task.system_export("SECUR32.dll", name) {
            let off = w + slot * w;
            let bytes = abi.ptr(addr);
            buf[off..off + w].copy_from_slice(&bytes);
        }
    }
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), base, &buf));
    Reply::Value(base)
}

/// `GetUserNameExW(format, buffer, &size)`: reports the local workstation user name.
fn get_user_name_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (fmt, buf, size_ptr) = (a[0] as u32, a[1], a[2]);
    if size_ptr == 0 {
        task.set_last_error(cx, 87);
        return Reply::Value(0);
    }
    let name = match fmt {
        2 => "WORKGROUP\\User", // NameSamCompatible
        3 => "User",            // NameDisplay
        8 => "User@workgroup",  // NameUserPrincipal
        _ => "User",
    };
    let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let cap = task.mem(cx, false, |t| user::read_u32(&t.space(), size_ptr)).unwrap_or(0) as usize;
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), size_ptr, units.len() as u32));
    if buf == 0 || cap < units.len() {
        task.set_last_error(cx, 234); // ERROR_MORE_DATA
        return Reply::Value(0);
    }
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    if task.mem(cx, true, |t| user::write(&mut t.space(), buf, &bytes)).is_err() {
        task.set_last_error(cx, 87);
        return Reply::Value(0);
    }
    Reply::Value(1)
}
