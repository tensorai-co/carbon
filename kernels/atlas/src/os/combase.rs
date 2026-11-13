//! combase.dll: the Windows Runtime's start-up and its strings. `RoInitialize` is COM's initialization; no runtime
//! class is registered on the machine, so activating one is REGDB_E_CLASSNOTREG, as for a class Windows doesn't have,
//! and programs carry on without it. HSTRINGs are atlas's own layout behind the opaque handle: { flags, length, the
//! characters' address }, the characters after it for one `WindowsCreateString` made, or the caller's for a fast-pass
//! string in its HSTRING_HEADER (which has room: 20 bytes, or 24 on 64-bit).

use std::collections::HashMap;
use std::sync::Mutex;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::kernel32::{global_new, global_release};
use super::{Reply, dll};
use crate::process::Task;

dll! {
    "COMBASE.dll";
    host GetRestrictedErrorInfo(1) => get_restricted_error_info;
    host RoActivateInstance(2) => ro_activate_instance;
    host RoGetActivationFactory(3) => ro_get_activation_factory;
    host RoGetApartmentIdentifier(1) => ro_get_apartment_identifier;
    host RoInitialize(1) => s_ok;
    host RoOriginateError(2) => false_;
    host RoOriginateErrorW(3) => false_;
    host RoRegisterActivationFactories(4) => not_implemented;
    host RoRegisterForApartmentShutdown(3) => ro_register_for_apartment_shutdown;
    host RoRevokeActivationFactories(1) => s_ok;
    host RoTransformError(3) => false_;
    host RoUninitialize(0) => s_ok;
    host RoUnregisterForApartmentShutdown(1) => s_ok;
    host SetRestrictedErrorInfo(1) => s_ok;
    host WindowsCompareStringOrdinal(3) => windows_compare_string_ordinal;
    host WindowsCreateString(3) => windows_create_string;
    host WindowsCreateStringReference(4) => windows_create_string_reference;
    host WindowsDeleteString(1) => windows_delete_string;
    host WindowsDuplicateString(2) => windows_duplicate_string;
    host WindowsGetStringLen(1) => windows_get_string_len;
    host WindowsGetStringRawBuffer(2) => windows_get_string_raw_buffer;
    host WindowsIsStringEmpty(1) => windows_is_string_empty;
    host WindowsStringHasEmbeddedNull(2) => windows_string_has_embedded_null;
}

const S_OK: u64 = 0;
const S_FALSE: u64 = 1;
const E_NOTIMPL: u64 = 0x8000_4001;
const E_INVALIDARG: u64 = 0x8007_0057;
const E_OUTOFMEMORY: u64 = 0x8007_000e;
/// REGDB_E_CLASSNOTREG.
const CLASS_NOT_REGISTERED: u64 = 0x8004_0154;
/// An HSTRING's flags: a fast-pass string, whose characters are its caller's.
const REFERENCE: u32 = 1;

fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

fn false_(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn not_implemented(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(E_NOTIMPL)
}

/// `RoGetActivationFactory(class, &iid, &factory)`: no runtime class is registered.
fn ro_get_activation_factory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0 {
        let _ = task.write_ptr(cx, a[2], 0);
    }
    Reply::Value(CLASS_NOT_REGISTERED)
}

/// `RoActivateInstance(class, &instance)`.
fn ro_activate_instance(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(CLASS_NOT_REGISTERED)
}

/// `RoGetApartmentIdentifier(&id)`: the one apartment.
fn ro_get_apartment_identifier(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[0], 1)).is_err() {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(S_OK)
}

/// `RoRegisterForApartmentShutdown(callback, &id, &cookie)`: the apartment never shuts down while the process runs.
fn ro_register_for_apartment_shutdown(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 || a[2] == 0 {
        return Reply::Value(E_INVALIDARG);
    }
    let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[1], 1));
    let _ = task.write_ptr(cx, a[2], 0x5200);
    Reply::Value(S_OK)
}

/// `GetRestrictedErrorInfo(&info)`: none.
fn get_restricted_error_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 {
        return Reply::Value(E_INVALIDARG);
    }
    let _ = task.write_ptr(cx, a[0], 0);
    Reply::Value(S_FALSE)
}

// ---- HSTRING ----

/// An HSTRING's header's size: flags, length, then the characters' address, pointer-aligned.
fn header_size(task: &Task) -> u64 {
    8 + task.abi().width
}

/// An HSTRING's flags, length and characters' address; a null one is the empty string.
fn parts(task: &mut Task, cx: &mut Cx<'_>, h: u64) -> Option<(u32, u32, u64)> {
    if h == 0 {
        return Some((0, 0, 0));
    }
    let width = task.abi().width as usize;
    let b = task.mem(cx, false, |t| user::read_vec(&t.space(), h, 8 + width)).ok()?;
    let at = task.abi().ptr_at(&b, 8);
    Some((u32::from_le_bytes(b[0..4].try_into().ok()?), u32::from_le_bytes(b[4..8].try_into().ok()?), at))
}

fn chars(task: &mut Task, cx: &mut Cx<'_>, h: u64) -> Option<Vec<u16>> {
    let (_, len, at) = parts(task, cx, h)?;
    if len == 0 {
        return Some(Vec::new());
    }
    let bytes = task.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize * 2)).ok()?;
    Some(bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
}

/// A new HSTRING holding `text`: its header, then the characters and a NUL. 0 if there's no memory.
fn new_string(task: &mut Task, cx: &mut Cx<'_>, text: &[u16]) -> u64 {
    let head = header_size(task);
    let mut b = vec![0u8; head as usize];
    b[4..8].copy_from_slice(&(text.len() as u32).to_le_bytes());
    b.extend(text.iter().chain([&0]).flat_map(|c| c.to_le_bytes()));
    let block = global_new(task, cx, &b);
    if block != 0 {
        let _ = task.write_ptr(cx, block + 8, block + head);
    }
    block
}

/// `WindowsCreateString(text, length, &string)`: a copy; none for an empty one.
fn windows_create_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let len = a[1] as u32 as usize;
    if a[2] == 0 || (a[0] == 0 && len != 0) {
        return Reply::Value(E_INVALIDARG);
    }
    if len == 0 {
        let _ = task.write_ptr(cx, a[2], 0);
        return Reply::Value(S_OK);
    }
    let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], len * 2)) else { return Reply::Value(E_INVALIDARG) };
    let text: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let h = new_string(task, cx, &text);
    if h == 0 {
        return Reply::Value(E_OUTOFMEMORY);
    }
    let _ = task.write_ptr(cx, a[2], h);
    Reply::Value(S_OK)
}

/// `WindowsCreateStringReference(text, length, &header, &string)`: the caller's characters, which must end in a NUL,
/// behind the header it gives.
fn windows_create_string_reference(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let len = a[1] as u32;
    if a[2] == 0 || a[3] == 0 || (a[0] == 0 && len != 0) {
        return Reply::Value(E_INVALIDARG);
    }
    if len == 0 {
        let _ = task.write_ptr(cx, a[3], 0);
        return Reply::Value(S_OK);
    }
    let terminated = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0] + u64::from(len) * 2, 2)).is_ok_and(|b| b == [0, 0]);
    if !terminated {
        return Reply::Value(E_INVALIDARG);
    }
    let mut b = vec![0u8; header_size(task) as usize];
    b[0..4].copy_from_slice(&REFERENCE.to_le_bytes());
    b[4..8].copy_from_slice(&len.to_le_bytes());
    let ptr = task.abi().ptr(a[0]);
    b[8..].copy_from_slice(&ptr);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() {
        return Reply::Value(E_INVALIDARG);
    }
    let _ = task.write_ptr(cx, a[3], a[2]);
    Reply::Value(S_OK)
}

/// `WindowsDeleteString(string)`: a fast-pass string is its caller's to keep.
fn windows_delete_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some((flags, _, _)) = parts(task, cx, a[0])
        && a[0] != 0
        && flags & REFERENCE == 0
    {
        global_release(task, cx, a[0]);
    }
    Reply::Value(S_OK)
}

/// `WindowsDuplicateString(string, &copy)`: a copy of its own (fast-pass strings too, as Windows does).
fn windows_duplicate_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] == 0 {
        return Reply::Value(E_INVALIDARG);
    }
    let Some(text) = chars(task, cx, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let h = if text.is_empty() { 0 } else { new_string(task, cx, &text) };
    if !text.is_empty() && h == 0 {
        return Reply::Value(E_OUTOFMEMORY);
    }
    let _ = task.write_ptr(cx, a[1], h);
    Reply::Value(S_OK)
}

fn windows_get_string_len(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(parts(task, cx, a[0]).map_or(0, |(_, len, _)| u64::from(len)))
}

/// The empty string each process's null HSTRING reads as, made once.
static EMPTY: Mutex<Option<HashMap<u64, u64>>> = Mutex::new(None);

/// `WindowsGetStringRawBuffer(string, &length)`: its characters (an empty string's for a null one).
fn windows_get_string_raw_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((_, len, mut at)) = parts(task, cx, a[0]) else { return Reply::Value(0) };
    if a[0] == 0 {
        let pid = task.process.pid;
        let made = lock(&EMPTY).get_or_insert_with(HashMap::new).get(&pid).copied();
        at = match made {
            Some(at) => at,
            None => {
                let at = global_new(task, cx, &[0, 0]);
                lock(&EMPTY).get_or_insert_with(HashMap::new).insert(pid, at);
                at
            }
        };
    }
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], len));
    }
    Reply::Value(at)
}

fn windows_is_string_empty(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(parts(task, cx, a[0]).is_none_or(|(_, len, _)| len == 0)))
}

/// `WindowsStringHasEmbeddedNull(string, &has)`.
fn windows_string_has_embedded_null(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(text) = chars(task, cx, a[0]) else { return Reply::Value(E_INVALIDARG) };
    if a[1] == 0 || task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], u32::from(text.contains(&0)))).is_err() {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(S_OK)
}

/// `WindowsCompareStringOrdinal(a, b, &result)`: -1, 0 or 1, by UTF-16 code units.
fn windows_compare_string_ordinal(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(x), Some(y)) = (chars(task, cx, a[0]), chars(task, cx, a[1])) else { return Reply::Value(E_INVALIDARG) };
    let order = x.cmp(&y) as i32;
    if a[2] == 0 || task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], order as u32)).is_err() {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(S_OK)
}
