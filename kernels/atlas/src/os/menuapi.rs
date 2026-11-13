//! user32's menus, over `win32k/menu.rs`: making and changing them, reading them back, loading them from resources,
//! a window's menu, and `TrackPopupMenu(Ex)`.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;
use crate::win32k::menu::{Item, MF_BYPOSITION, MF_CHECKED, MF_DEFAULT, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_RADIOCHECK, MF_SEPARATOR, parse_menu};

const TRUE: u64 = 1;
const FALSE: u64 = 0;
/// -1, as a UINT: "no such item".
const NONE: u64 = 0xffff_ffff;
/// MF_BITMAP and MF_OWNERDRAW: what's passed as text isn't a string.
const NOT_TEXT: u32 = 0x4 | 0x100;

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}

/// An item as `AppendMenu`, `InsertMenu` and `ModifyMenu` describe it.
fn item_of(task: &mut Task, cx: &mut Cx<'_>, flags: u32, id: u64, text: u64, wide: bool) -> Item {
    let text = if flags & (NOT_TEXT | MF_SEPARATOR) == 0 { task.text_arg(cx, text, wide) } else { String::new() };
    let popup = flags & MF_POPUP != 0;
    Item { id: if popup { 0 } else { id & 0xffff_ffff }, text, flags: flags & !MF_BYPOSITION, submenu: if popup { id } else { 0 }, data: 0 }
}

pub fn create_menu(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().create_menu())
}

pub fn destroy_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().destroy_menu(a[0])))
}

/// `AppendMenuW(menu, flags, id or submenu, text)`.
fn append(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let item = item_of(task, cx, a[1] as u32, a[2], a[3], wide);
    match task.win32k().menus.get_mut(&a[0]) {
        Some(m) => {
            m.items.push(item);
            Reply::Value(TRUE)
        }
        None => Reply::Value(FALSE),
    }
}

pub fn append_menu_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    append(task, cx, a, true)
}

pub fn append_menu_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    append(task, cx, a, false)
}

/// Where a new item goes: before the item at `position` (by place, or by command), or at the end for -1 or past it.
fn insert_at(task: &Task, menu: u64, position: u64, by_position: bool) -> Option<(u64, usize)> {
    let w = task.win32k();
    let len = w.menus.get(&menu)?.items.len();
    if by_position {
        let at = position as u32;
        return Some((menu, if at == u32::MAX { len } else { (at as usize).min(len) }));
    }
    w.find_item(menu, position, false).or(Some((menu, len)))
}

/// `InsertMenuW(menu, position, flags, id or submenu, text)`.
fn insert(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let flags = a[2] as u32;
    let item = item_of(task, cx, flags, a[3], a[4], wide);
    let Some((menu, at)) = insert_at(task, a[0], a[1], flags & MF_BYPOSITION != 0) else { return Reply::Value(FALSE) };
    match task.win32k().menus.get_mut(&menu) {
        Some(m) => {
            m.items.insert(at, item);
            Reply::Value(TRUE)
        }
        None => Reply::Value(FALSE),
    }
}

pub fn insert_menu_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    insert(task, cx, a, true)
}

pub fn insert_menu_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    insert(task, cx, a, false)
}

/// `ModifyMenuW(menu, position, flags, id or submenu, text)`: the item replaced.
fn modify(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let flags = a[2] as u32;
    let new = item_of(task, cx, flags, a[3], a[4], wide);
    match task.win32k().item_mut(a[0], a[1], flags & MF_BYPOSITION != 0) {
        Some(item) => {
            *item = new;
            Reply::Value(TRUE)
        }
        None => Reply::Value(FALSE),
    }
}

pub fn modify_menu_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    modify(task, cx, a, true)
}

pub fn modify_menu_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    modify(task, cx, a, false)
}

/// `RemoveMenu(menu, position, flags)`, and `DeleteMenu`, which destroys a submenu too.
fn remove(task: &mut Task, a: &[u64], destroy: bool) -> Reply {
    let mut w = task.win32k();
    let Some((menu, at)) = w.find_item(a[0], a[1], a[2] as u32 & MF_BYPOSITION != 0) else { return Reply::Value(FALSE) };
    let Some(item) = w.menus.get_mut(&menu).map(|m| m.items.remove(at)) else { return Reply::Value(FALSE) };
    if destroy && item.flags & MF_POPUP != 0 {
        w.destroy_menu(item.submenu);
    }
    Reply::Value(TRUE)
}

pub fn remove_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    remove(task, a, false)
}

pub fn delete_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    remove(task, a, true)
}

/// `EnableMenuItem(menu, item, flags)` on a menu of the program's: the state it had (MF_GRAYED, MF_DISABLED), or -1.
pub fn enable_item(task: &mut Task, a: &[u64]) -> Option<u64> {
    let flags = a[2] as u32;
    let mut w = task.win32k();
    let item = w.item_mut(a[0], a[1], flags & MF_BYPOSITION != 0)?;
    let old = item.flags & (MF_GRAYED | MF_DISABLED);
    item.flags = item.flags & !(MF_GRAYED | MF_DISABLED) | flags & (MF_GRAYED | MF_DISABLED);
    Some(u64::from(old))
}

/// `CheckMenuItem(menu, item, flags)`: the check it had, or -1.
pub fn check_menu_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let flags = a[2] as u32;
    let mut w = task.win32k();
    let Some(item) = w.item_mut(a[0], a[1], flags & MF_BYPOSITION != 0) else { return Reply::Value(NONE) };
    let old = item.flags & MF_CHECKED;
    item.flags = item.flags & !MF_CHECKED | flags & MF_CHECKED;
    Reply::Value(u64::from(old))
}

/// `CheckMenuRadioItem(menu, first, last, check, flags)`: `check` checked with a dot, the others in the range not.
pub fn check_menu_radio_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (first, last, check, by_position) = (a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32 & MF_BYPOSITION != 0);
    let mut w = task.win32k();
    let mut found = false;
    for id in first..=last.min(first.saturating_add(4096)) {
        if let Some(item) = w.item_mut(a[0], u64::from(id), by_position) {
            if id == check {
                item.flags |= MF_CHECKED | MF_RADIOCHECK;
                found = true;
            } else {
                item.flags &= !MF_CHECKED;
            }
        }
    }
    Reply::Value(u64::from(found))
}

pub fn get_menu_item_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().menus.get(&a[0]).map_or(NONE, |m| m.items.len() as u64))
}

/// `GetMenuItemID(menu, position)`: -1 for a submenu or no item.
pub fn get_menu_item_id(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let item = w.menus.get(&a[0]).and_then(|m| m.items.get(a[1] as u32 as usize));
    Reply::Value(item.filter(|i| i.flags & MF_POPUP == 0).map_or(NONE, |i| i.id))
}

pub fn get_sub_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    Reply::Value(w.menus.get(&a[0]).and_then(|m| m.items.get(a[1] as u32 as usize)).filter(|i| i.flags & MF_POPUP != 0).map_or(0, |i| i.submenu))
}

/// `GetMenuState(menu, item, flags)`: its flags (a submenu's with its item count in the high byte), or -1.
pub fn get_menu_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some((menu, at)) = w.find_item(a[0], a[1], a[2] as u32 & MF_BYPOSITION != 0) else { return Reply::Value(NONE) };
    let item = &w.menus[&menu].items[at];
    let count = if item.flags & MF_POPUP != 0 { w.menus.get(&item.submenu).map_or(0, |m| m.items.len() as u32) } else { 0 };
    Reply::Value(u64::from(count << 8 | item.flags & 0xff | item.flags & (MF_SEPARATOR | MF_DEFAULT)))
}

/// `GetMenuStringW(menu, item, buffer, size, flags)`: its text's length (what was copied, with a buffer).
fn get_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let text = {
        let w = task.win32k();
        let Some((menu, at)) = w.find_item(a[0], a[1], a[4] as u32 & MF_BYPOSITION != 0) else { return Reply::Value(0) };
        w.menus[&menu].items[at].text.clone()
    };
    let room = a[3] as u32 as usize;
    if a[2] == 0 || room == 0 {
        return Reply::Value(if wide { text.encode_utf16().count() } else { crate::ansi::bytes(&text).len() } as u64);
    }
    let (bytes, n): (Vec<u8>, usize) = if wide {
        let units: Vec<u16> = text.encode_utf16().take(room - 1).collect();
        (units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect(), units.len())
    } else {
        let b: Vec<u8> = crate::ansi::bytes(&text).into_iter().take(room - 1).collect();
        let n = b.len();
        (b.into_iter().chain([0]).collect(), n)
    };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &bytes));
    Reply::Value(n as u64)
}

pub fn get_menu_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_string(task, cx, a, true)
}

pub fn get_menu_string_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_string(task, cx, a, false)
}

/// `SetMenuDefaultItem(menu, item, by_position)`: only it is the default; -1 for none.
pub fn set_menu_default_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let target = if a[1] as u32 == u32::MAX { None } else { w.find_item(a[0], a[1], a[2] != 0) };
    let Some(m) = w.menus.get_mut(&a[0]) else { return Reply::Value(FALSE) };
    for item in &mut m.items {
        item.flags &= !MF_DEFAULT;
    }
    if let Some((menu, at)) = target
        && let Some(item) = w.menus.get_mut(&menu).and_then(|m| m.items.get_mut(at))
    {
        item.flags |= MF_DEFAULT;
    }
    Reply::Value(TRUE)
}

/// `GetMenuDefaultItem(menu, by_position, flags)`: its ID or place, or -1.
pub fn get_menu_default_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(m) = w.menus.get(&a[0]) else { return Reply::Value(NONE) };
    Reply::Value(m.items.iter().position(|i| i.flags & MF_DEFAULT != 0).map_or(NONE, |at| if a[1] != 0 { at as u64 } else { m.items[at].id }))
}

// ---- MENUITEMINFO ----

/// MENUITEMINFO's layout for a program of this width: where wID, hSubMenu, dwItemData, dwTypeData and cch are, and
/// its size.
fn mii(width: usize) -> (usize, usize, usize, usize, usize, usize) {
    if width == 8 { (16, 24, 48, 56, 64, 80) } else { (16, 20, 32, 36, 40, 48) }
}

const MIIM_STATE: u32 = 0x1;
const MIIM_ID: u32 = 0x2;
const MIIM_SUBMENU: u32 = 0x4;
const MIIM_TYPE: u32 = 0x10;
const MIIM_DATA: u32 = 0x20;
const MIIM_STRING: u32 = 0x40;
const MIIM_FTYPE: u32 = 0x100;
/// MFT_* and MFS_* bits an item keeps, as MF_* flags.
const TYPE_BITS: u32 = MF_SEPARATOR | MF_RADIOCHECK | NOT_TEXT;
const STATE_BITS: u32 = MF_GRAYED | MF_DISABLED | MF_CHECKED | MF_DEFAULT;

/// Changes an item by a MENUITEMINFO, as `SetMenuItemInfo` and `InsertMenuItem` do.
fn apply_info(task: &mut Task, cx: &mut Cx<'_>, at: u64, item: &mut Item, wide: bool) -> bool {
    let abi = task.abi();
    let (id_at, sub_at, data_at, text_at, _, size) = mii(abi.width as usize);
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, size)) else { return false };
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
    let (mask, kind, state) = (u(4), u(8), u(12));
    if mask & (MIIM_FTYPE | MIIM_TYPE) != 0 {
        item.flags = item.flags & !TYPE_BITS | kind & TYPE_BITS;
    }
    if mask & MIIM_STATE != 0 {
        item.flags = item.flags & !STATE_BITS | state & STATE_BITS;
    }
    if mask & MIIM_ID != 0 {
        item.id = u64::from(u(id_at));
    }
    if mask & MIIM_SUBMENU != 0 {
        item.submenu = abi.ptr_at(&b, sub_at);
        if item.submenu != 0 {
            item.flags |= MF_POPUP;
        } else {
            item.flags &= !MF_POPUP;
        }
    }
    if mask & MIIM_DATA != 0 {
        item.data = abi.ptr_at(&b, data_at);
    }
    if mask & (MIIM_STRING | MIIM_TYPE) != 0 && kind & (MF_SEPARATOR | NOT_TEXT) == 0 {
        item.text = task.text_arg(cx, abi.ptr_at(&b, text_at), wide);
    }
    true
}

/// `InsertMenuItemW(menu, item, by_position, &info)`.
fn insert_item(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let mut item = Item::default();
    if !apply_info(task, cx, a[3], &mut item, wide) {
        return Reply::Value(FALSE);
    }
    let Some((menu, at)) = insert_at(task, a[0], a[1], a[2] != 0) else { return Reply::Value(FALSE) };
    match task.win32k().menus.get_mut(&menu) {
        Some(m) => {
            m.items.insert(at, item);
            Reply::Value(TRUE)
        }
        None => Reply::Value(FALSE),
    }
}

pub fn insert_menu_item_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    insert_item(task, cx, a, true)
}

pub fn insert_menu_item_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    insert_item(task, cx, a, false)
}

/// `SetMenuItemInfoW(menu, item, by_position, &info)`.
fn set_item_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(mut item) = task.win32k().item_mut(a[0], a[1], a[2] != 0).cloned() else { return Reply::Value(FALSE) };
    if !apply_info(task, cx, a[3], &mut item, wide) {
        return Reply::Value(FALSE);
    }
    match task.win32k().item_mut(a[0], a[1], a[2] != 0) {
        Some(slot) => {
            *slot = item;
            Reply::Value(TRUE)
        }
        None => Reply::Value(FALSE),
    }
}

pub fn set_menu_item_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_item_info(task, cx, a, true)
}

pub fn set_menu_item_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_item_info(task, cx, a, false)
}

/// `GetMenuItemInfoW(menu, item, by_position, &info)`: what its mask asks for; its text into dwTypeData's buffer
/// (cch the room, set to the length), or with no buffer just cch.
fn get_item_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(item) = task.win32k().item_mut(a[0], a[1], a[2] != 0).cloned() else { return Reply::Value(FALSE) };
    let abi = task.abi();
    let (id_at, sub_at, data_at, text_at, cch_at, size) = mii(abi.width as usize);
    let Ok(mut b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[3], size)) else { return Reply::Value(FALSE) };
    let mask = u32::from_le_bytes(b[4..8].try_into().unwrap_or_default());
    let put32 = |b: &mut Vec<u8>, o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    if mask & (MIIM_FTYPE | MIIM_TYPE) != 0 {
        put32(&mut b, 8, item.flags & TYPE_BITS);
    }
    if mask & MIIM_STATE != 0 {
        put32(&mut b, 12, item.flags & STATE_BITS);
    }
    if mask & MIIM_ID != 0 {
        put32(&mut b, id_at, item.id as u32);
    }
    if mask & MIIM_SUBMENU != 0 {
        b[sub_at..sub_at + abi.width as usize].copy_from_slice(&abi.ptr(item.submenu));
    }
    if mask & MIIM_DATA != 0 {
        b[data_at..data_at + abi.width as usize].copy_from_slice(&abi.ptr(item.data));
    }
    if mask & (MIIM_STRING | MIIM_TYPE) != 0 {
        let buffer = abi.ptr_at(&b, text_at);
        let room = u32::from_le_bytes(b[cch_at..cch_at + 4].try_into().unwrap_or_default());
        let length = if wide { item.text.encode_utf16().count() } else { crate::ansi::bytes(&item.text).len() };
        let written = if buffer != 0 && room > 0 {
            let r = get_string(task, cx, &[a[0], a[1], buffer, u64::from(room), if a[2] != 0 { u64::from(MF_BYPOSITION) } else { 0 }], wide);
            match r {
                Reply::Value(n) => n as u32,
                other => return other,
            }
        } else {
            length as u32
        };
        put32(&mut b, cch_at, written);
    }
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &b)).is_ok()))
}

pub fn get_menu_item_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_item_info(task, cx, a, true)
}

pub fn get_menu_item_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_item_info(task, cx, a, false)
}

// ---- Resources and windows ----

/// `LoadMenuW(module, name)`: a `MENU` or `MENUEX` resource made into menus.
fn load(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(name) = task.resource_id(cx, a[1], wide) else { return Reply::Value(0) };
    let Ok((_, data, size)) = task.find_resource(cx, a[0], &crate::resources::Id::Num(4), &name, None) else {
        task.set_last_error(cx, 1814);
        return Reply::Value(0);
    };
    let Ok(bytes) = task.mem(cx, false, |t| user::read_vec(&t.space(), data, size as usize)) else { return Reply::Value(0) };
    Reply::Value(parse_menu(&bytes, &mut task.win32k()).unwrap_or(0))
}

pub fn load_menu_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    load(task, cx, a, true)
}

pub fn load_menu_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    load(task, cx, a, false)
}

/// `LoadMenuIndirectW(template)`: from a template in memory.
pub fn load_menu_indirect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let bytes = task.dialog_in_memory(cx, a[0]);
    Reply::Value(parse_menu(&bytes, &mut task.win32k()).unwrap_or(0))
}

/// `SetMenu(window, menu)`: its menu kept (menu bars aren't drawn yet).
pub fn set_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    if !w.windows.contains_key(&a[0]) || (a[1] != 0 && !w.menus.contains_key(&a[1])) {
        return Reply::Value(FALSE);
    }
    if a[1] == 0 {
        w.menu_bars.remove(&a[0]);
    } else {
        w.menu_bars.insert(a[0], a[1]);
    }
    Reply::Value(TRUE)
}

pub fn get_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().menu_bars.get(&a[0]).copied().unwrap_or(0))
}

pub fn draw_menu_bar(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.window_exists(a[0])))
}

/// `TrackPopupMenu(menu, flags, x, y, reserved, window, rect)`.
pub fn track_popup_menu(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.track_popup_menu(cx, a[0], a[1] as u32, a[2] as i32, a[3] as i32, a[5]))
}

/// `TrackPopupMenuEx(menu, flags, x, y, window, params)`.
pub fn track_popup_menu_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.track_popup_menu(cx, a[0], a[1] as u32, a[2] as i32, a[3] as i32, a[4]))
}
