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
