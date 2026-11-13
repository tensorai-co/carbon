//! shell32.dll: files dropped on windows (`DragAcceptFiles`, WM_DROPFILES and its HDROP), over the desktop's drag and
//! drop; and the shell's core (`shellapi.rs`): special folders and item ID lists, file operations, running programs,
//! choosing a folder, and what a file is.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll, shellapi};
use crate::process::Task;
use crate::win32k::WS_EX_ACCEPTFILES;

dll! {
    "SHELL32.dll";
    host DragAcceptFiles(2) => drag_accept_files;
    host DragQueryFileW(4) => drag_query_file_w;
    host DragQueryFileA(4) => drag_query_file_a;
    host DragQueryPoint(2) => drag_query_point;
    host DragFinish(1) => drag_finish;
    host SHGetFileInfoW(5) => sh_get_file_info_w;
    host SHGetFileInfoA(5) => sh_get_file_info_a;
    host SHGetSpecialFolderLocation(3) => sh_get_special_folder_location;
    host SHGetFolderLocation(5) => sh_get_folder_location;
    host SHGetPathFromIDListW(2) => sh_get_path_from_id_list_w;
    host SHGetPathFromIDListA(2) => sh_get_path_from_id_list_a;
    host SHGetFolderPathW(5) => sh_get_folder_path_w;
    host SHGetFolderPathA(5) => sh_get_folder_path_a;
    host SHGetSpecialFolderPathW(4) => sh_get_special_folder_path_w;
    host SHGetSpecialFolderPathA(4) => sh_get_special_folder_path_a;
    host ILFree(1) => il_free;
    host ILCreateFromPathW(1) => il_create_from_path_w;
    host SHFileOperationW(1) => sh_file_operation_w;
    host SHFileOperationA(1) => sh_file_operation_a;
    host ShellExecuteW(6) => shell_execute_w;
    host ShellExecuteA(6) => shell_execute_a;
    host ShellExecuteExW(1) => shell_execute_ex_w;
    host SHBrowseForFolderW(1) => sh_browse_for_folder_w;
    host SHBrowseForFolderA(1) => sh_browse_for_folder_a;
}

/// `DragAcceptFiles`: whether a window takes files dropped on it (its `WS_EX_ACCEPTFILES`).
fn drag_accept_files(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(w) = task.win32k().windows.get_mut(&a[0]) {
        if a[1] != 0 {
            w.ex_style |= WS_EX_ACCEPTFILES;
        } else {
            w.ex_style &= !WS_EX_ACCEPTFILES;
        }
    }
    Reply::Value(0)
}

/// `DragQueryFileW(hDrop, iFile, lpszFile, cch)`: with `iFile` 0xFFFFFFFF, how many files; with no buffer, a file's
/// length in characters; else the file, cut to fit `cch` with its NUL, and how many characters were copied.
fn drag_query_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    drag_query_file(task, cx, a, true)
}

fn drag_query_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    drag_query_file(task, cx, a, false)
}

fn drag_query_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (index, buffer, cch) = (a[1] as u32, a[2], a[3] as u32 as usize);
    let file = {
        let w = task.win32k();
        let Some(drop) = w.hdrops.get(&a[0]) else { return Reply::Value(0) };
        if index == u32::MAX {
            return Reply::Value(drop.files.len() as u64);
        }
        let Some(file) = drop.files.get(index as usize) else { return Reply::Value(0) };
        if wide { file.encode_utf16().collect::<Vec<u16>>() } else { crate::ansi::bytes(file).into_iter().map(u16::from).collect() }
    };
    if buffer == 0 {
        return Reply::Value(file.len() as u64);
    }
    if cch == 0 {
        return Reply::Value(0);
    }
    let n = file.len().min(cch - 1);
    let bytes: Vec<u8> = if wide { file[..n].iter().chain(&[0]).flat_map(|u| u.to_le_bytes()).collect() } else { file[..n].iter().map(|&b| b as u8).chain([0]).collect() };
    match task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &bytes)) {
        Ok(_) => Reply::Value(n as u64),
        Err(_) => Reply::Value(0),
    }
}

/// `DragQueryPoint`: where it was dropped, in the window's client coordinates; TRUE, as it's always in its client area.
fn drag_query_point(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((x, y)) = task.win32k().hdrops.get(&a[0]).map(|d| d.point) else { return Reply::Value(0) };
    let point: Vec<u8> = [x, y].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &point)).is_ok()))
}

/// `DragFinish`: the HDROP's let go.
fn drag_finish(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.win32k().hdrops.remove(&a[0]);
    Reply::Value(0)
}

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}

// ---- Special folders and item ID lists ----

const S_OK: u64 = 0;
const E_FAIL: u64 = 0x8000_4005;
const E_INVALIDARG: u64 = 0x8007_0057;

/// A special folder's PIDL, written to `out`: S_OK, or E_INVALIDARG for no such folder.
fn folder_location(task: &mut Task, cx: &mut Cx<'_>, csidl: u32, out: u64) -> Reply {
    let Some(path) = shellapi::special_folder(csidl, task.isa) else { return Reply::Value(E_INVALIDARG) };
    if let Some(p) = &path {
        task.make_folders(p);
    }
    let pidl = match task.make_pidl(cx, path.as_deref(), csidl) {
        Ok(p) => p,
        Err(reply) => return reply,
    };
    if pidl == 0 {
        return Reply::Value(E_FAIL);
    }
    let _ = task.write_ptr(cx, out, pidl);
    Reply::Value(S_OK)
}

/// `SHGetSpecialFolderLocation(window, csidl, &pidl)`.
fn sh_get_special_folder_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    folder_location(task, cx, a[1] as u32, a[2])
}

/// `SHGetFolderLocation(window, csidl, token, reserved, &pidl)`.
fn sh_get_folder_location(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    folder_location(task, cx, a[1] as u32, a[4])
}

/// `SHGetPathFromIDListW(pidl, path)`: TRUE and its path, or FALSE (and an empty string) for one that isn't on the
/// disk.
fn path_from_id_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let path = task.pidl_path(cx, a[0]);
    task.put_path_text(cx, a[1], path.as_deref().unwrap_or(""), wide);
    Reply::Value(u64::from(path.is_some()))
}

fn sh_get_path_from_id_list_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_from_id_list(task, cx, a, true)
}

fn sh_get_path_from_id_list_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_from_id_list(task, cx, a, false)
}

/// A special folder's path, made if it isn't there (as `CSIDL_FLAG_CREATE` asks, and a real profile has them): None
/// for a virtual folder or no such folder.
fn folder_path(task: &mut Task, csidl: u32) -> Option<String> {
    let path = shellapi::special_folder(csidl, task.isa)??;
    task.make_folders(&path);
    Some(path)
}

/// `SHGetFolderPathW(window, csidl, token, flags, path)`: S_OK, or E_FAIL for a folder that isn't on the disk.
fn folder_path_reply(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    match folder_path(task, a[1] as u32) {
        Some(path) => {
            task.put_path_text(cx, a[4], &path, wide);
            Reply::Value(S_OK)
        }
        None => {
            task.put_path_text(cx, a[4], "", wide);
            Reply::Value(if shellapi::special_folder(a[1] as u32, task.isa).is_some() { E_FAIL } else { E_INVALIDARG })
        }
    }
}

fn sh_get_folder_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    folder_path_reply(task, cx, a, true)
}

fn sh_get_folder_path_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    folder_path_reply(task, cx, a, false)
}

/// `SHGetSpecialFolderPathW(window, path, csidl, create)`: TRUE and its path.
fn special_folder_path(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    match folder_path(task, a[2] as u32) {
        Some(path) => {
            task.put_path_text(cx, a[1], &path, wide);
            Reply::Value(1)
        }
        None => Reply::Value(0),
    }
}

fn sh_get_special_folder_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    special_folder_path(task, cx, a, true)
}

fn sh_get_special_folder_path_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    special_folder_path(task, cx, a, false)
}

/// `ILFree(pidl)`: back to the program's heap.
fn il_free(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.task_free(cx, a[0]).map(|()| 0))
}

/// `ILCreateFromPathW(path)`: a PIDL for a file or folder that's there, or NULL.
fn il_create_from_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let path = task.wstr(cx, a[0]);
    let Ok((full, _)) = task.resolve(&path) else { return Reply::Value(0) };
    if task.path_info(&full).is_err() {
        return Reply::Value(0);
    }
    value(task.make_pidl(cx, Some(&full), 0))
}

// ---- File operations ----

/// `SHFileOperationW(&SHFILEOPSTRUCTW)`: 0, or an error; its fAnyOperationsAborted FALSE. SHFILEOPSTRUCT is packed
/// for 32-bit programs, naturally aligned for 64-bit ones.
fn file_operation(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let abi = task.abi();
    let w = abi.width as usize;
    let (func_at, from_at, to_at, flags_at, aborted_at) = if w == 8 { (8, 16, 24, 32, 36) } else { (4, 8, 12, 16, 18) };
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], aborted_at + 4)) else { return Reply::Value(87) };
    let func = u32::from_le_bytes(b[func_at..func_at + 4].try_into().unwrap_or_default());
    let flags = u16::from_le_bytes([b[flags_at], b[flags_at + 1]]);
    let from = task.path_list_of(cx, abi.ptr_at(&b, from_at), wide);
    let to = task.path_list_of(cx, abi.ptr_at(&b, to_at), wide);
    let result = task.file_operation(func, &from, &to, flags);
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0] + aborted_at as u64, &0u32.to_le_bytes()));
    Reply::Value(u64::from(result))
}

fn sh_file_operation_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    file_operation(task, cx, a, true)
}

fn sh_file_operation_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    file_operation(task, cx, a, false)
}

// ---- Running programs ----

/// `ShellExecuteW(window, verb, file, parameters, folder, show)`: above 32 if it ran, else an `SE_ERR_*`.
fn shell_execute(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let verb = task.text_arg(cx, a[1], wide);
    let file = task.text_arg(cx, a[2], wide);
    let params = task.text_arg(cx, a[3], wide);
    let dir = (a[4] != 0).then(|| task.text_arg(cx, a[4], wide));
    match task.shell_execute(cx, &verb, &file, &params, dir.as_deref(), a[5] as u32) {
        Ok(Ok(process)) => {
            kernels_core::lock(&task.process.state).handles.close(process);
            Reply::Value(33)
        }
        Ok(Err(e)) => Reply::Value(e),
        Err(reply) => reply,
    }
}

fn shell_execute_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    shell_execute(task, cx, a, true)
}

fn shell_execute_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    shell_execute(task, cx, a, false)
}

/// `ShellExecuteExW(&SHELLEXECUTEINFOW)`: its hInstApp, and with SEE_MASK_NOCLOSEPROCESS the new process's handle.
fn shell_execute_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
    let abi = task.abi();
    let w = abi.width as usize;
    // cbSize, fMask, then hwnd, lpVerb, lpFile, lpParameters, lpDirectory, nShow, hInstApp, ... hProcess.
    let (verb_at, file_at, params_at, dir_at, show_at, inst_at, process_at) = if w == 8 { (16, 24, 32, 40, 48, 56, 104) } else { (12, 16, 20, 24, 28, 32, 56) };
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], process_at + w)) else { return Reply::Value(0) };
    let mask = u32::from_le_bytes(b[4..8].try_into().unwrap_or_default());
    let verb = task.text_arg(cx, abi.ptr_at(&b, verb_at), true);
    let file = task.text_arg(cx, abi.ptr_at(&b, file_at), true);
    let params = task.text_arg(cx, abi.ptr_at(&b, params_at), true);
    let dir_ptr = abi.ptr_at(&b, dir_at);
    let dir = (dir_ptr != 0).then(|| task.text_arg(cx, dir_ptr, true));
    let show = u32::from_le_bytes(b[show_at..show_at + 4].try_into().unwrap_or_default());
    let (inst, process, ok) = match task.shell_execute(cx, &verb, &file, &params, dir.as_deref(), show) {
        Ok(Ok(process)) => {
            let keep = mask & SEE_MASK_NOCLOSEPROCESS != 0;
            if !keep {
                kernels_core::lock(&task.process.state).handles.close(process);
            }
            (33, if keep { process } else { 0 }, true)
        }
        Ok(Err(e)) => (e, 0, false),
        Err(reply) => return reply,
    };
    let _ = task.mem(cx, true, |t| {
        user::write(&mut t.space(), a[0] + inst_at as u64, &abi.ptr(inst))?;
        user::write(&mut t.space(), a[0] + process_at as u64, &abi.ptr(process))
    });
    if !ok {
        // ERROR_FILE_NOT_FOUND, or ERROR_NO_ASSOCIATION.
        task.set_last_error(cx, if inst == 2 { 2 } else { 1155 });
    }
    Reply::Value(u64::from(ok))
}

// ---- Choosing a folder ----

/// `SHBrowseForFolderW(&BROWSEINFOW)`: hwndOwner, pidlRoot, pszDisplayName, lpszTitle, ulFlags, lpfn, lParam, iImage.
fn browse_for_folder(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let abi = task.abi();
    let w = abi.width as usize;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 4 * w + 4 + 3 * w + 4)) else { return Reply::Value(0) };
    let flags_end = 4 * w + 4;
    let lpfn_at = flags_end.next_multiple_of(w);
    let (owner, display, title_at) = (abi.ptr_at(&b, 0), abi.ptr_at(&b, 2 * w), abi.ptr_at(&b, 3 * w));
    let (callback, param) = (abi.ptr_at(&b, lpfn_at), abi.ptr_at(&b, lpfn_at + w));
    let image_at = a[0] + (lpfn_at + 2 * w) as u64;
    let title = task.text_arg(cx, title_at, wide);
    value(task.browse_for_folder(cx, owner, &title, display, callback, param, wide, image_at))
}

fn sh_browse_for_folder_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    browse_for_folder(task, cx, a, true)
}

fn sh_browse_for_folder_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    browse_for_folder(task, cx, a, false)
}

// ---- What a file is ----

/// `SHGetFileInfoW(path, attributes, &info, size, flags)`: SHFILEINFO's icon, icon index, attributes, display name
/// and type name, as `flags` asks. The file must be there, unless SHGFI_USEFILEATTRIBUTES says to go by `attributes`;
/// with SHGFI_PIDL, `path` is a PIDL. TRUE, or the system image list for SHGFI_SYSICONINDEX, or (SHGFI_EXETYPE) what
/// kind of program it is; 0 for a file that isn't there. Icons aren't drawn yet: an icon's handle is a placeholder,
/// and every file's index in the image list is 0.
fn file_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    const SHGFI_ICON: u32 = 0x100;
    const SHGFI_DISPLAYNAME: u32 = 0x200;
    const SHGFI_TYPENAME: u32 = 0x400;
    const SHGFI_ATTRIBUTES: u32 = 0x800;
    const SHGFI_EXETYPE: u32 = 0x2000;
    const SHGFI_SYSICONINDEX: u32 = 0x4000;
    const SHGFI_SMALLICON: u32 = 0x1;
    const SHGFI_PIDL: u32 = 0x8;
    const SHGFI_USEFILEATTRIBUTES: u32 = 0x10;
    let (info_at, size, flags) = (a[2], (a[3] as u32).min(1024) as usize, a[4] as u32);
    let path = if flags & SHGFI_PIDL != 0 { task.pidl_path(cx, a[0]).unwrap_or_default() } else { task.text_arg(cx, a[0], wide) };
    let attributes = if flags & SHGFI_USEFILEATTRIBUTES != 0 {
        Some(a[1] as u32)
    } else if path.is_empty() {
        None
    } else {
        match task.resolve(&path).ok().and_then(|(full, _)| task.path_info(&full).ok()) {
            Some(i) => Some(i.attributes),
            None => return Reply::Value(0),
        }
    };
    if flags & SHGFI_EXETYPE != 0 {
        let Ok(bytes) = task.read_whole(&path) else { return Reply::Value(0) };
        let Ok(pe) = crate::images::pe::Pe::parse(&bytes) else { return Reply::Value(0) };
        // 'PE', and for a windowed program its subsystem's version.
        let (subsystem, major, minor) = pe.subsystem();
        let version = if subsystem == 2 { u64::from(major) << 8 | u64::from(minor) } else { 0 };
        return Reply::Value(0x4550 | version << 16);
    }
    let folder = attributes.is_some_and(|at| at & 0x10 != 0);
    let name = path.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
    let type_name = if folder {
        "File folder".to_string()
    } else {
        match name.rsplit_once('.') {
            Some((_, ext)) if ext.eq_ignore_ascii_case("exe") => "Application".to_string(),
            Some((_, ext)) if !ext.is_empty() => format!("{} File", ext.to_ascii_uppercase()),
            _ => "File".to_string(),
        }
    };
    // SFGAO_FILESYSTEM, and SFGAO_FOLDER, SFGAO_FILESYSANCESTOR and SFGAO_HASSUBFOLDER for a folder; CANCOPY,
    // CANMOVE, CANRENAME and CANDELETE; SFGAO_READONLY for a read-only file.
    let sfgao: u32 = 0x4000_0000 | 0x37 | if folder { 0xb000_0000 } else { 0 } | if attributes.is_some_and(|at| at & 1 != 0) { 0x4_0000 } else { 0 };
    let abi = task.abi();
    let w = abi.width as usize;
    let mut b = vec![0u8; size];
    let mut put = |at: usize, bytes: &[u8]| {
        if let Some(slot) = b.get_mut(at..at + bytes.len()) {
            slot.copy_from_slice(bytes);
        }
    };
    if flags & SHGFI_ICON != 0 {
        put(0, &abi.ptr(0x0001_7f00 | if folder { 3 } else { 1 }));
    }
    if flags & SHGFI_ATTRIBUTES != 0 {
        put(w + 4, &sfgao.to_le_bytes());
    }
    // szDisplayName (MAX_PATH) and szTypeName (80), in UTF-16 or ANSI.
    let text = |s: &str, room: usize| -> Vec<u8> {
        if wide {
            s.encode_utf16().take(room - 1).chain([0]).flat_map(u16::to_le_bytes).collect()
        } else {
            crate::ansi::bytes(s).into_iter().take(room - 1).chain([0]).collect()
        }
    };
    let unit = if wide { 2 } else { 1 };
    if flags & SHGFI_DISPLAYNAME != 0 {
        put(w + 8, &text(&name, 260));
    }
    if flags & SHGFI_TYPENAME != 0 {
        put(w + 8 + 260 * unit, &text(&type_name, 80));
    }
    if info_at != 0 && task.mem(cx, true, |t| user::write(&mut t.space(), info_at, &b)).is_err() {
        return Reply::Value(0);
    }
    if flags & SHGFI_SYSICONINDEX != 0 {
        let lists = {
            let w = task.win32k();
            w.system_images
        };
        let (large, small) = match lists {
            Some(l) => l,
            None => {
                let l = (task.image_list_create(32, 32), task.image_list_create(16, 16));
                task.image_list_add(l.0, 4);
                task.image_list_add(l.1, 4);
                task.win32k().system_images = Some(l);
                l
            }
        };
        return Reply::Value(if flags & SHGFI_SMALLICON != 0 { small } else { large });
    }
    Reply::Value(1)
}

fn sh_get_file_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    file_info(task, cx, a, true)
}

fn sh_get_file_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    file_info(task, cx, a, false)
}
