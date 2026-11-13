//! comctl32.dll: the common controls. The progress bar and the list view are win32k's (`win32k/comctl.rs`), their
//! classes there from the start, as if comctl32 had registered them as it loaded; their procedures are exported
//! here too, so a program can call them. `InitCommonControls(Ex)` succeeds. Image lists keep their images, cut from
//! bitmaps or taken from icons, and draw them. Its exports are numbered from 17, InitCommonControls' ordinal, which
//! programs import it by.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "COMCTL32.dll" @ 17;
    host InitCommonControls(0) => init_common_controls;
    host InitCommonControlsEx(1) => init_common_controls_ex;
    host ImageList_Create(5) => image_list_create;
    host ImageList_Destroy(1) => image_list_destroy;
    host ImageList_Add(3) => image_list_add;
    host ImageList_AddMasked(3) => image_list_add_masked;
    host ImageList_ReplaceIcon(3) => image_list_replace_icon;
    host ImageList_Draw(6) => image_list_draw;
    host ImageList_GetImageCount(1) => image_list_get_image_count;
    host ImageList_GetIconSize(3) => image_list_get_icon_size;
    host ImageList_Remove(2) => image_list_remove;
    host ImageList_SetBkColor(2) => image_list_set_bk_color;
    host AtlasProgressWndProc(4) => progress_wnd_proc;
    host AtlasListViewWndProc(4) => list_view_wnd_proc;
    missing CreatePropertySheetPageW;
    missing PropertySheetW;
    missing TaskDialogIndirect;
}

/// `InitCommonControls()`: its classes are there already.
fn init_common_controls(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `InitCommonControlsEx(&init)`: TRUE.
fn init_common_controls_ex(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}

fn progress_wnd_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.progress_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn list_view_wnd_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.list_view_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

/// `ImageList_Create(cx, cy, flags, initial, grow)`: an empty list of images that size.
fn image_list_create(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.image_list_create(a[0] as i32, a[1] as i32))
}

fn image_list_destroy(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().image_lists.remove(&a[0]).is_some()))
}

/// `ImageList_Add(list, bitmap, mask)`: the bitmap's images, as many as its width holds; transparent where the mask
/// bitmap is white. The first's index, or -1.
fn image_list_add(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.image_list_add_bitmap(a[0], a[1], (a[2] != 0).then_some(a[2]), None))
}

/// `ImageList_AddMasked(list, bitmap, colour)`: the same, transparent where the bitmap is that colour.
fn image_list_add_masked(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let key = (a[2] as u32 != 0xffff_ffff).then(|| crate::win32k::gdi::swap(a[2] as u32));
    Reply::Value(task.image_list_add_bitmap(a[0], a[1], None, key))
}

/// `ImageList_ReplaceIcon(list, index, icon)`: -1 adds it.
fn image_list_replace_icon(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.image_list_replace_icon(a[0], a[1] as i32, a[2]))
}

/// `ImageList_Draw(list, index, dc, x, y, style)`.
fn image_list_draw(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.image_list_draw(a[0], a[1] as i32, a[2], a[3] as i32, a[4] as i32)))
}

fn image_list_get_image_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().image_lists.get(&a[0]).map_or(0, |l| l.images.len() as u64))
}

/// `ImageList_GetIconSize(list, &cx, &cy)`.
fn image_list_get_icon_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((w, h)) = task.win32k().image_lists.get(&a[0]).map(|l| (l.width, l.height)) else { return Reply::Value(0) };
    for (at, v) in [(a[1], w), (a[2], h)] {
        if at != 0 {
            let _ = task.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), at, &v.to_le_bytes()));
        }
    }
    Reply::Value(1)
}

/// `ImageList_Remove(list, index)`: -1 removes them all.
fn image_list_remove(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let index = a[1] as i32;
    let mut w = task.win32k();
    let Some(l) = w.image_lists.get_mut(&a[0]) else { return Reply::Value(0) };
    if index < 0 {
        l.images.clear();
    } else if (index as usize) < l.images.len() {
        l.images.remove(index as usize);
    } else {
        return Reply::Value(0);
    }
    Reply::Value(1)
}

/// `ImageList_SetBkColor(list, colour)`: the colour before.
fn image_list_set_bk_color(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(l) = w.image_lists.get_mut(&a[0]) else { return Reply::Value(0xffff_ffff) };
    Reply::Value(std::mem::replace(&mut l.background, a[1] & 0xffff_ffff))
}
