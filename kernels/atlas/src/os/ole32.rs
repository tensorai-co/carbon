//! ole32.dll: COM and OLE. Apartments: `CoInitialize(Ex)` and `OleInitialize` succeed and their `Uninitialize`s do
//! nothing, as nothing is kept per apartment yet. The task allocator is guest code, on the process heap.
//! `CoCreateInstance` makes atlas's own classes (`com.rs`: the shell link), whose methods are exports here; for any
//! other class it says none is registered, as Windows does for a class it doesn't know.

use kernels_core::thread::Cx;

use super::com::*;
use super::{Reply, dll};
use crate::process::Task;

dll! {
    "ole32.dll";
    host CoInitialize(1) => co_initialize;
    host CoInitializeEx(2) => co_initialize;
    host CoUninitialize(0) => nothing;
    host CoCreateInstance(5) => co_create_instance;
    guest CoTaskMemAlloc => AtlasCoTaskMemAlloc;
    guest CoTaskMemRealloc => AtlasCoTaskMemRealloc;
    guest CoTaskMemFree => AtlasCoTaskMemFree;
    host OleInitialize(1) => co_initialize;
    host OleUninitialize(0) => nothing;
    // The methods of atlas's COM objects, which their vtables point at.
    host AtlasComQueryInterface(3) => com_query_interface;
    host AtlasComAddRef(1) => com_add_ref;
    host AtlasComRelease(1) => com_release;
    host AtlasLinkGetPathW(5) => link_get_path_w;
    host AtlasLinkGetPathA(5) => link_get_path_a;
    host AtlasLinkGetIDList(2) => link_get_id_list;
    host AtlasLinkSetIDList(2) => link_set_id_list;
    host AtlasLinkGetDescriptionW(3) => link_get_description_w;
    host AtlasLinkGetDescriptionA(3) => link_get_description_a;
    host AtlasLinkSetDescriptionW(2) => link_set_description_w;
    host AtlasLinkSetDescriptionA(2) => link_set_description_a;
    host AtlasLinkGetWorkingDirectoryW(3) => link_get_working_directory_w;
    host AtlasLinkGetWorkingDirectoryA(3) => link_get_working_directory_a;
    host AtlasLinkSetWorkingDirectoryW(2) => link_set_working_directory_w;
    host AtlasLinkSetWorkingDirectoryA(2) => link_set_working_directory_a;
    host AtlasLinkGetArgumentsW(3) => link_get_arguments_w;
    host AtlasLinkGetArgumentsA(3) => link_get_arguments_a;
    host AtlasLinkSetArgumentsW(2) => link_set_arguments_w;
    host AtlasLinkSetArgumentsA(2) => link_set_arguments_a;
    host AtlasLinkGetHotkey(2) => link_get_hotkey;
    host AtlasLinkSetHotkey(2) => link_set_hotkey;
    host AtlasLinkGetShowCmd(2) => link_get_show_cmd;
    host AtlasLinkSetShowCmd(2) => link_set_show_cmd;
    host AtlasLinkGetIconLocationW(4) => link_get_icon_location_w;
    host AtlasLinkGetIconLocationA(4) => link_get_icon_location_a;
    host AtlasLinkSetIconLocationW(3) => link_set_icon_location_w;
    host AtlasLinkSetIconLocationA(3) => link_set_icon_location_a;
    host AtlasLinkSetRelativePathW(3) => link_set_relative_path_w;
    host AtlasLinkSetRelativePathA(3) => link_set_relative_path_a;
    host AtlasLinkResolve(3) => link_resolve;
    host AtlasLinkSetPathW(2) => link_set_path_w;
    host AtlasLinkSetPathA(2) => link_set_path_a;
    host AtlasLinkGetClassID(2) => link_get_class_id;
    host AtlasLinkIsDirty(1) => link_is_dirty;
    host AtlasLinkLoad(3) => link_load;
    host AtlasLinkSave(3) => link_save;
    host AtlasLinkSaveCompleted(2) => link_save_completed;
    host AtlasLinkGetCurFile(2) => link_get_cur_file;
    // Not written yet: what larger programs import.
    host CLSIDFromString(2) => clsid_from_string;
    host IIDFromString(2) => clsid_from_string;
    host StringFromCLSID(2) => string_from_iid;
    host CoSetProxyBlanket(8) => s_ok;
    host DoDragDrop(4) => do_drag_drop;
    guest PropVariantClear => AtlasPropVariantClear;
    host RegisterDragDrop(2) => s_ok;
    host ReleaseStgMedium(1) => nothing;
    host RevokeDragDrop(1) => s_ok;
    host StringFromIID(2) => string_from_iid;
    missing CoAddRefServerProcess;
    missing CoAllowSetForegroundWindow;
    missing CoCreateFreeThreadedMarshaler;
    missing CoCreateGuid;
    missing CoGetApartmentType;
    missing CoGetContextToken;
    missing CoGetInterfaceAndReleaseStream;
    missing CoGetMalloc;
    missing CoGetObjectContext;
    missing CoMarshalInterThreadInterfaceInStream;
    missing CoRegisterInitializeSpy;
    missing CoReleaseServerProcess;
    missing CoRevokeInitializeSpy;
    missing CreateILockBytesOnHGlobal;
    missing CreateStreamOnHGlobal;
    missing GetHGlobalFromILockBytes;
    missing GetHGlobalFromStream;
    missing OleDuplicateData;
    missing PropVariantCopy;
    missing StgCreateDocfileOnILockBytes;
    missing StringFromGUID2;
}

const S_OK: u64 = 0;
/// REGDB_E_CLASSNOTREG.
const CLASS_NOT_REGISTERED: u64 = 0x8004_0154;

/// `CoInitialize(reserved)`, `CoInitializeEx(reserved, model)` and `OleInitialize(reserved)`: S_OK.
fn co_initialize(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

fn nothing(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `CoCreateInstance(&clsid, outer, context, &iid, &object)`: one of atlas's classes, through the interface asked for;
/// else `*object` is null and the result REGDB_E_CLASSNOTREG. Aggregation (an outer object) isn't there yet.
fn co_create_instance(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const CLASS_E_NOAGGREGATION: u64 = 0x8004_0110;
    if a[4] != 0 {
        let _ = task.write_ptr(cx, a[4], 0);
    }
    if a[1] != 0 {
        return Reply::Value(CLASS_E_NOAGGREGATION);
    }
    match task.create_com_object(cx, a[0], a[3], a[4]) {
        Ok(Some(hr)) => Reply::Value(hr),
        Ok(None) => Reply::Value(CLASS_NOT_REGISTERED),
        Err(reply) => reply,
    }
}

fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

/// `DoDragDrop(data, source, effects, &effect)`: DRAGDROP_S_CANCEL, nothing dropped (dragging out of a window isn't
/// there yet).
fn do_drag_drop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] != 0 {
        let _ = task.mem(cx, true, |t| kernels_core::user::write_u32(&mut t.space(), a[3], 0));
    }
    Reply::Value(0x0004_0101)
}

/// A GUID's text, `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`, as its 16 bytes.
fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let hex: String = s.strip_prefix('{')?.strip_suffix('}')?.split('-').collect();
    let parts: Vec<&str> = s.trim_matches(['{', '}']).split('-').collect();
    if hex.len() != 32 || parts.iter().map(|p| p.len()).collect::<Vec<_>>() != [8, 4, 4, 4, 12] {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok();
    let mut g = [0u8; 16];
    let a = u32::from_str_radix(&hex[..8], 16).ok()?;
    let b = u16::from_str_radix(&hex[8..12], 16).ok()?;
    let c = u16::from_str_radix(&hex[12..16], 16).ok()?;
    g[..4].copy_from_slice(&a.to_le_bytes());
    g[4..6].copy_from_slice(&b.to_le_bytes());
    g[6..8].copy_from_slice(&c.to_le_bytes());
    for (i, b) in g.iter_mut().enumerate().skip(8) {
        *b = byte(i)?;
    }
    Some(g)
}

/// `CLSIDFromString(text, &clsid)` and `IIDFromString`: a GUID's braced text; CO_E_CLASSSTRING for what isn't one.
fn clsid_from_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let text = task.wstr(cx, a[0]);
    match parse_guid(&text) {
        Some(g) if task.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), a[1], &g)).is_ok() => Reply::Value(S_OK),
        _ => Reply::Value(0x8004_01f3),
    }
}

/// `StringFromIID(&iid, &text)` and `StringFromCLSID`: its braced text, in a block from `CoTaskMemAlloc`.
fn string_from_iid(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok(g) = task.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), a[0], 16)) else { return Reply::Value(0x8007_0057) };
    let text = format!(
        "{{{:08X}-{:04X}-{:04X}-{}-{}}}",
        u32::from_le_bytes([g[0], g[1], g[2], g[3]]),
        u16::from_le_bytes([g[4], g[5]]),
        u16::from_le_bytes([g[6], g[7]]),
        g[8..10].iter().map(|b| format!("{b:02X}")).collect::<String>(),
        g[10..16].iter().map(|b| format!("{b:02X}")).collect::<String>()
    );
    let bytes: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    match task.task_alloc(cx, &bytes) {
        Ok(at) if at != 0 && task.write_ptr(cx, a[1], at).is_ok() => Reply::Value(S_OK),
        Ok(_) => Reply::Value(0x8007_000e),
        Err(reply) => reply,
    }
}
