//! shell32.dll: files dropped on windows (`DragAcceptFiles`, WM_DROPFILES and its HDROP), over the desktop's drag and
//! drop. The rest of the shell isn't there yet.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::WS_EX_ACCEPTFILES;

dll! {
    "SHELL32.dll";
    host DragAcceptFiles(2) => drag_accept_files;
    host DragQueryFileW(4) => drag_query_file_w;
    host DragQueryPoint(2) => drag_query_point;
    host DragFinish(1) => drag_finish;
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
    let (index, buffer, cch) = (a[1] as u32, a[2], a[3] as u32 as usize);
    let file = {
        let w = task.win32k();
        let Some(drop) = w.hdrops.get(&a[0]) else { return Reply::Value(0) };
        if index == u32::MAX {
            return Reply::Value(drop.files.len() as u64);
        }
        let Some(file) = drop.files.get(index as usize) else { return Reply::Value(0) };
        file.encode_utf16().collect::<Vec<u16>>()
    };
    if buffer == 0 {
        return Reply::Value(file.len() as u64);
    }
    if cch == 0 {
        return Reply::Value(0);
    }
    let n = file.len().min(cch - 1);
    let bytes: Vec<u8> = file[..n].iter().chain(&[0]).flat_map(|u| u.to_le_bytes()).collect();
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
