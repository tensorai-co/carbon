//! user32's smaller calls, as larger programs make them starting up: rectangles; the keyboard's layout (US English)
//! and its keys and characters; registered messages and window properties; a window's ancestors, placement and
//! information; the display's one device and its mode; the cursor; and what's taken but has no effect here (hooks,
//! event hooks, device notifications, raw input, layered windows' attributes, mouse tracking).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use display::Rect;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;
use crate::win32k::{self, keys};

const TRUE: u64 = 1;
const FALSE: u64 = 0;
/// English (United States), with its keyboard of the same.
const HKL_US: u64 = 0x0409_0409;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

fn rect_at(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<Rect> {
    task.read_rect(cx, at)
}

fn put_rect(task: &mut Task, cx: &mut Cx<'_>, at: u64, r: &Rect) -> Reply {
    Reply::Value(u64::from(task.write_rect(cx, at, r)))
}

// ---- Rectangles ----

fn is_empty(r: &Rect) -> bool {
    r.right <= r.left || r.bottom <= r.top
}

/// `IntersectRect(&out, &a, &b)`: their overlap, or an empty rectangle and FALSE.
pub fn intersect_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(x), Some(y)) = (rect_at(task, cx, a[1]), rect_at(task, cx, a[2])) else { return Reply::Value(FALSE) };
    let r = Rect { left: x.left.max(y.left), top: x.top.max(y.top), right: x.right.min(y.right), bottom: x.bottom.min(y.bottom) };
    if is_empty(&r) {
        task.write_rect(cx, a[0], &Rect::default());
        return Reply::Value(FALSE);
    }
    put_rect(task, cx, a[0], &r)
}

/// `UnionRect(&out, &a, &b)`: the smallest rectangle around both (an empty one counting for nothing).
pub fn union_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(x), Some(y)) = (rect_at(task, cx, a[1]), rect_at(task, cx, a[2])) else { return Reply::Value(FALSE) };
    let r = match (is_empty(&x), is_empty(&y)) {
        (true, true) => Rect::default(),
        (true, false) => y,
        (false, true) => x,
        _ => Rect { left: x.left.min(y.left), top: x.top.min(y.top), right: x.right.max(y.right), bottom: x.bottom.max(y.bottom) },
    };
    task.write_rect(cx, a[0], &r);
    Reply::Value(u64::from(!is_empty(&r)))
}

pub fn is_rect_empty(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(rect_at(task, cx, a[0]).is_none_or(|r| is_empty(&r))))
}

/// `OffsetRect(&rect, dx, dy)`.
pub fn offset_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = rect_at(task, cx, a[0]) else { return Reply::Value(FALSE) };
    put_rect(task, cx, a[0], &r.offset(a[1] as u32 as i32, a[2] as u32 as i32))
}

/// `SetRect(&rect, left, top, right, bottom)`.
pub fn set_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let [left, top, right, bottom] = [a[1], a[2], a[3], a[4]].map(|v| v as u32 as i32);
    put_rect(task, cx, a[0], &Rect { left, top, right, bottom })
}

pub fn equal_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(x), Some(y)) = (rect_at(task, cx, a[0]), rect_at(task, cx, a[1])) else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(x == y))
}

/// `InflateRect(&rect, dx, dy)`: wider and taller by twice each (narrower for negative ones).
pub fn inflate_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = rect_at(task, cx, a[0]) else { return Reply::Value(FALSE) };
    let (dx, dy) = (a[1] as u32 as i32, a[2] as u32 as i32);
    put_rect(task, cx, a[0], &Rect { left: r.left - dx, top: r.top - dy, right: r.right + dx, bottom: r.bottom + dy })
}

pub fn set_rect_empty(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_rect(task, cx, a[0], &Rect::default())
}

/// `PtInRect(&rect, point)`: the point by value, x in its low half; right and bottom aren't in.
pub fn pt_in_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y) = (a[1] as u32 as i32, (a[1] >> 32) as u32 as i32);
    let Some(r) = rect_at(task, cx, a[0]) else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(x >= r.left && x < r.right && y >= r.top && y < r.bottom))
}

// ---- The keyboard ----

pub fn get_keyboard_layout(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(HKL_US)
}

/// `GetKeyboardLayoutList(room, &list)`: the one layout; with no room, how many there are.
pub fn get_keyboard_layout_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] as u32 as i32 > 0 && a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], HKL_US);
    }
    Reply::Value(1)
}

/// `ActivateKeyboardLayout(layout, flags)`: the one there was, which is still it.
pub fn activate_keyboard_layout(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(HKL_US)
}

/// The character a key types with Shift held or not (US English), if it types one.
fn typed(vk: u8, shift: bool) -> Option<char> {
    let code = keys::code(vk)?;
    display::keys::text(code, if shift { display::keys::SHIFT } else { 0 })
}

/// `MapVirtualKey(Ex)W(code, type, layout)` and `A`: MAPVK_VK_TO_VSC (0), MAPVK_VSC_TO_VK (1), MAPVK_VK_TO_CHAR (2) and
/// MAPVK_VSC_TO_VK_EX (3).
pub fn map_virtual_key(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let code = a[0] as u32;
    Reply::Value(match a[1] as u32 {
        0 | 4 => keys::code(code as u8).map_or(0, |c| u64::from(keys::scan(c).0)),
        1 | 3 => {
            let extended = code & 0xe000 == 0xe000;
            keys::from_scan((code & 0xff) as u16, extended).map_or(0, |c| {
                let vk = keys::vk(c);
                u64::from(if a[1] as u32 == 1 { keys::generic(vk) } else { vk })
            })
        }
        2 => typed(code as u8, false).map_or(0, |c| u64::from(c.to_ascii_uppercase() as u32)),
        _ => 0,
    })
}

/// `VkKeyScanExW(char, layout)` and `A`: the key that types it, in the low byte, and Shift (1) in the high one if it's
/// needed; -1 for a character no key types.
pub fn vk_key_scan_ex(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(c) = char::from_u32(a[0] as u32 & 0xffff) else { return Reply::Value(0xffff) };
    for shift in [false, true] {
        for vk in 0x08..=0xfeu8 {
            if typed(vk, shift) == Some(c) {
                return Reply::Value(u64::from(vk) | if shift { 0x100 } else { 0 });
            }
        }
    }
    Reply::Value(0xffff)
}

/// `ToUnicode(vk, scan, &key state, out, room, flags)`: the character the key types with Shift as the state has it
/// (Ctrl and Alt type nothing); 1, or 0 for none.
pub fn to_unicode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok(state) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 256)) else { return Reply::Value(0) };
    let held = |vk: u8| state[vk as usize] & 0x80 != 0;
    if held(keys::VK_CONTROL) || held(keys::VK_MENU) || a[4] as u32 as i32 <= 0 {
        return Reply::Value(0);
    }
    match typed(a[0] as u8, held(keys::VK_SHIFT)) {
        Some(c) => {
            let mut b = [0u16; 2];
            let units: Vec<u8> = c.encode_utf16(&mut b).iter().flat_map(|u| u.to_le_bytes()).collect();
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &units[..2]));
            Reply::Value(1)
        }
        None => Reply::Value(0),
    }
}

/// `GetKeyboardState(&state)`: the 256 keys as this thread's messages have had them.
pub fn get_keyboard_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let tid = task.tid;
    let state = task.win32k().thread_keys(tid);
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &state)).is_ok()))
}

// ---- Messages, properties, windows ----

/// `RegisterWindowMessageW(name)` and `A`: a message number of its own for the name, from 0xC000, shared with the
/// clipboard's registered formats as on Windows.
pub fn register_window_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    super::user32::register_clipboard_format_any(task, cx, a, wide)
}

pub fn register_window_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register_window_message(task, cx, a, true)
}

pub fn register_window_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register_window_message(task, cx, a, false)
}

/// Window properties, by window and name (or atom, for a "name" below 0x10000).
static PROPS: Mutex<Option<HashMap<(u64, String), u64>>> = Mutex::new(None);

fn prop_name(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> String {
    if at < 0x1_0000 { format!("#{at}") } else { task.wstr(cx, at).to_lowercase() }
}

/// `SetPropW(window, name, value)`.
pub fn set_prop_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.win32k().windows.contains_key(&a[0]) {
        return Reply::Value(FALSE);
    }
    let name = prop_name(task, cx, a[1]);
    lock(&PROPS).get_or_insert_with(HashMap::new).insert((a[0], name), a[2]);
    Reply::Value(TRUE)
}

pub fn get_prop_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = prop_name(task, cx, a[1]);
    Reply::Value(lock(&PROPS).as_ref().and_then(|p| p.get(&(a[0], name)).copied()).unwrap_or(0))
}

pub fn remove_prop_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = prop_name(task, cx, a[1]);
    Reply::Value(lock(&PROPS).as_mut().and_then(|p| p.remove(&(a[0], name))).unwrap_or(0))
}

/// `IsChild(parent, window)`: whether `window` is below `parent`, through child windows (not owned ones).
pub fn is_child(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let mut at = a[1];
    for _ in 0..64 {
        let Some(win) = w.windows.get(&at) else { return Reply::Value(FALSE) };
        if win.top_level() {
            return Reply::Value(FALSE);
        }
        if win.parent == a[0] {
            return Reply::Value(TRUE);
        }
        at = win.parent;
    }
    Reply::Value(FALSE)
}

/// `GetAncestor(window, flags)`: GA_PARENT (1), GA_ROOT (2), GA_ROOTOWNER (3).
pub fn get_ancestor(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(win) = w.windows.get(&a[0]) else { return Reply::Value(0) };
    Reply::Value(match a[1] as u32 {
        1 => {
            if win.top_level() {
                win32k::DESKTOP
            } else if win.message_only() {
                u64::MAX - 2
            } else {
                win.parent
            }
        }
        2 => w.top_level_of(a[0]).unwrap_or(a[0]),
        3 => {
            let mut root = w.top_level_of(a[0]).unwrap_or(a[0]);
            for _ in 0..64 {
                match w.windows.get(&root).map(|r| r.owner) {
                    Some(owner) if owner != 0 && w.windows.contains_key(&owner) => root = w.top_level_of(owner).unwrap_or(owner),
                    _ => break,
                }
            }
            root
        }
        _ => 0,
    })
}

/// `GetClassNameW(window, out, room)` and `A`.
fn get_class_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(class) = task.win32k().windows.get(&a[0]).map(|w| w.class.clone()) else {
        task.set_last_error(cx, ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    task.put_text(cx, a[1], a[2] as u32 as u64, &class, wide)
}

pub fn get_class_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_class_name(task, cx, a, true)
}

pub fn get_class_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_class_name(task, cx, a, false)
}

/// A window's rectangle on the screen, and its client area's.
fn screen_rects(task: &mut Task, hwnd: u64) -> Option<(Rect, Rect, u32, u32)> {
    let w = task.win32k();
    let win = w.windows.get(&hwnd)?;
    let outer = if win.top_level() {
        win.rect
    } else {
        let (x, y) = w.screen_origin(win.parent).unwrap_or_default();
        win.rect.offset(x, y)
    };
    let (cx, cy) = w.screen_origin(hwnd).unwrap_or((outer.left, outer.top));
    let client = win.client();
    Some((outer, client.offset(cx, cy), win.style, win.ex_style))
}

/// `GetWindowPlacement(window, &placement)`: shown normally (or minimized or maximized, as its style says), where it is.
pub fn get_window_placement(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const WS_MINIMIZE: u32 = 0x2000_0000;
    const WS_MAXIMIZE: u32 = 0x0100_0000;
    let Some((outer, _, style, _)) = screen_rects(task, a[0]) else { return Reply::Value(FALSE) };
    let show = if style & WS_MINIMIZE != 0 { 2 } else if style & WS_MAXIMIZE != 0 { 3 } else { 1 };
    let mut b = Vec::new();
    for v in [44i32, 0, show, -1, -1, -1, -1, outer.left, outer.top, outer.right, outer.bottom] {
        b.extend(v.to_le_bytes());
    }
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_ok()))
}

/// `GetWindowInfo(window, &info)`: WINDOWINFO's rectangles on the screen, styles, whether it's active, its borders.
pub fn get_window_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((outer, client, style, ex_style)) = screen_rects(task, a[0]) else { return Reply::Value(FALSE) };
    let mut b = 60u32.to_le_bytes().to_vec();
    for r in [outer, client] {
        for v in [r.left, r.top, r.right, r.bottom] {
            b.extend(v.to_le_bytes());
        }
    }
    let border = (client.left - outer.left).max(0) as u32;
    for v in [style, ex_style, 0, border, border] {
        b.extend(v.to_le_bytes());
    }
    b.extend([0u8; 4]);
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_ok()))
}

/// `GetUpdateRect(window, &rect, erase)`: what needs painting; whether anything does.
pub fn get_update_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.win32k().windows.get(&a[0]).map(|w| w.invalid) else { return Reply::Value(FALSE) };
    if a[1] != 0 {
        task.write_rect(cx, a[1], &r);
    }
    Reply::Value(u64::from(!is_empty(&r)))
}

/// `GetShellWindow()`: there's no shell's desktop window.
pub fn get_shell_window(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

pub fn get_message_extra_info(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `GetMessageTime()`: now, as messages' times count (milliseconds since the machine started).
pub fn get_message_time(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from((task.k.host.monotonic() / 1_000_000) as u32))
}

/// `GetMessagePos()`: where the pointer is, x in the low word and y in the high.
pub fn get_message_pos(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    let (x, y) = task.desktop(|d| d.pointer());
    Reply::Value(u64::from(x as u16) | u64::from(y as u16) << 16)
}

pub fn get_double_click_time(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(500)
}

/// `GetQueueStatus(flags)`: nothing new (the queue's kinds of message aren't counted apart).
pub fn get_queue_status(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

pub fn get_clipboard_sequence_number(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

/// `CharLowerBuffW(text, length)`: lowercased where it is, one character for one.
pub fn char_lower_buff_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let n = a[1] as u32 as usize;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], n * 2)) else { return Reply::Value(0) };
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let lower: Vec<u8> = char::decode_utf16(units.iter().copied())
        .flat_map(|c| {
            let c = c.unwrap_or('\u{fffd}');
            let mut l = c.to_lowercase();
            let one = if l.len() == 1 { l.next().unwrap_or(c) } else { c };
            let mut buf = [0u16; 2];
            one.encode_utf16(&mut buf).iter().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()
        })
        .collect();
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &lower));
    Reply::Value(n as u64)
}

// ---- The display ----

/// `EnumDisplayDevicesW(device, index, &device info, flags)` and `A`: one adapter, DISPLAY1, attached and primary.
fn enum_display_devices(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if a[1] as u32 != 0 {
        return Reply::Value(FALSE);
    }
    let text = |s: &str, n: usize| -> Vec<u8> {
        let mut b: Vec<u8> = if wide { s.encode_utf16().flat_map(u16::to_le_bytes).collect() } else { s.as_bytes().to_vec() };
        b.resize(if wide { 2 * n } else { n }, 0);
        b
    };
    let monitor = a[0] != 0;
    let mut b = Vec::new();
    let Ok(size) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[2])) else { return Reply::Value(FALSE) };
    b.extend(size.to_le_bytes());
    b.extend(text(if monitor { "\\\\.\\DISPLAY1\\Monitor0" } else { "\\\\.\\DISPLAY1" }, 32));
    b.extend(text(if monitor { "Generic Monitor" } else { "Carbon Display" }, 128));
    b.extend(5u32.to_le_bytes());
    b.extend(text("", 128));
    b.extend(text("", 128));
    b.truncate(size as usize);
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_ok()))
}

pub fn enum_display_devices_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_display_devices(task, cx, a, true)
}

pub fn enum_display_devices_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_display_devices(task, cx, a, false)
}

/// `EnumDisplaySettingsW(device, mode, &devmode)` and `A`: the current mode (ENUM_CURRENT_SETTINGS or
/// ENUM_REGISTRY_SETTINGS, or mode 0, the only one): the desktop's size, 32 bits a pixel, 60 Hz.
fn enum_display_settings(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if !matches!(a[1] as u32, 0 | 0xffff_ffff | 0xffff_fffe) {
        return Reply::Value(FALSE);
    }
    let (w, h) = task.desktop(|d| d.size());
    // DEVMODE's fields after the device name, which is 32 characters.
    let name = if wide { 64u64 } else { 32 };
    let fields = 0x0004_0000u32 | 0x0008_0000 | 0x0010_0000 | 0x0020_0000 | 0x0040_0000;
    let writes: [(u64, u32); 6] = [(name + 8, fields), (name + 104, 32), (name + 108, w as u32), (name + 112, h as u32), (name + 116, 0), (name + 120, 60)];
    let ok = task.mem(cx, true, |t| {
        for (at, v) in writes {
            user::write_u32(&mut t.space(), a[2] + at, v)?;
        }
        Ok(())
    });
    Reply::Value(u64::from(ok.is_ok()))
}

pub fn enum_display_settings_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_display_settings(task, cx, a, true)
}

pub fn enum_display_settings_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_display_settings(task, cx, a, false)
}

/// The display configuration calls: ERROR_NOT_SUPPORTED, so programs use the older ones.
pub fn not_supported(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(50)
}

// ---- The cursor ----

static CURSOR_COUNT: AtomicI32 = AtomicI32::new(0);

/// `ShowCursor(show)`: the display count, up or down one; the pointer shows at 0 and above.
pub fn show_cursor(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let by = if a[0] as u32 != 0 { 1 } else { -1 };
    Reply::Value(u64::from((CURSOR_COUNT.fetch_add(by, Ordering::Relaxed) + by) as u32))
}

/// `GetCursorInfo(&info)`: CURSORINFO: showing, the cursor, where the pointer is.
pub fn get_cursor_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y) = task.desktop(|d| d.pointer());
    let abi = task.abi();
    let showing = u32::from(CURSOR_COUNT.load(Ordering::Relaxed) >= 0);
    let mut b = (if abi.width == 8 { 24u32 } else { 20 }).to_le_bytes().to_vec();
    b.extend(showing.to_le_bytes());
    b.extend(abi.ptr(0x0001_0003));
    b.extend(x.to_le_bytes());
    b.extend(y.to_le_bytes());
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &b)).is_ok()))
}

// ---- What's taken and has no effect here ----

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(0x00a1_0000);

/// A handle of its own: for a hook, an event hook, a device notification. None of them is ever called.
pub fn new_handle(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(NEXT_HANDLE.fetch_add(4, Ordering::Relaxed))
}

pub fn succeed(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(TRUE)
}

pub fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `GetRawInputDeviceList(&list, &count, size)`: no raw input devices.
pub fn get_raw_input_device_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 0));
    Reply::Value(0)
}

/// `OpenInputDesktop(flags, inherit, access)` and `OpenDesktopA`: the thread's desktop.
pub fn open_desktop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    super::user32::thread_desktop(task, cx, a)
}

/// `SendMessageTimeoutW(window, message, w, l, flags, timeout, &result)` and `A`: sent, as `SendMessage` sends it,
/// its result where asked.
fn send_message_timeout(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.win32k().windows.contains_key(&a[0]) && a[0] != 0xffff {
        task.set_last_error(cx, ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    }
    if a[0] == 0xffff {
        // HWND_BROADCAST: to no one, done.
        if a[6] != 0 {
            let _ = task.write_ptr(cx, a[6], 0);
        }
        return Reply::Value(1);
    }
    match task.send_message(cx, a[0], a[1] as u32, a[2], a[3]) {
        Ok(result) => {
            if a[6] != 0 {
                let _ = task.write_ptr(cx, a[6], result);
            }
            Reply::Value(1)
        }
        Err(reply) => reply,
    }
}

pub fn send_message_timeout_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_message_timeout(task, cx, a)
}

/// `SystemParametersInfoW(action, uint, &value, flags)` and `A`: the work area (all of the desktop); fonts smoothed;
/// and for the rest, TRUE with the value as it was (zeros, mostly).
pub fn system_parameters_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const SPI_GETWORKAREA: u32 = 0x30;
    const SPI_GETFONTSMOOTHING: u32 = 0x4a;
    const SPI_GETKEYBOARDDELAY: u32 = 0x16;
    const SPI_GETKEYBOARDSPEED: u32 = 0xa;
    const SPI_GETWHEELSCROLLLINES: u32 = 0x68;
    let (w, h) = task.desktop(|d| d.size());
    match a[0] as u32 {
        SPI_GETWORKAREA => put_rect(task, cx, a[2], &Rect { left: 0, top: 0, right: w, bottom: h }),
        action @ (SPI_GETFONTSMOOTHING | SPI_GETKEYBOARDDELAY | SPI_GETKEYBOARDSPEED | SPI_GETWHEELSCROLLLINES) => {
            let v = match action {
                SPI_GETFONTSMOOTHING => 1,
                SPI_GETKEYBOARDDELAY => 1,
                SPI_GETKEYBOARDSPEED => 31,
                _ => 3,
            };
            Reply::Value(u64::from(a[2] == 0 || task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], v)).is_ok()))
        }
        _ => Reply::Value(TRUE),
    }
}

// ---- Window stations and desktops a program makes and opens ----

use crate::win32k::stations::{DESKTOP, WINDOW_STATION};

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ALREADY_EXISTS: u32 = 183;

/// A name argument: none for a null pointer.
fn name_arg(task: &mut Task, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
    if at == 0 {
        return None;
    }
    if wide {
        task.mem(cx, false, |t| user::read_wstr(&t.space(), at, 260)).ok()
    } else {
        task.mem(cx, false, |t| user::read_cstr(&t.space(), at, 260)).ok().map(|b| crate::ansi::text(&b))
    }
}

/// A new user object's descriptor, from its SECURITY_ATTRIBUTES, kept as `SetSecurityInfo` would keep it.
fn keep_descriptor(task: &mut Task, cx: &mut Cx<'_>, handle: u64, attributes: u64) {
    if let Some(d) = task.attributes_descriptor(cx, attributes) {
        lock(&task.k.user_security).insert(handle, d);
    }
}

/// What a lookup's failure means: there already (183), or not found (2).
fn failed(task: &mut Task, cx: &mut Cx<'_>, exists: bool) -> Reply {
    task.set_last_error(cx, if exists { ERROR_ALREADY_EXISTS } else { ERROR_FILE_NOT_FOUND });
    Reply::Value(0)
}

/// `CreateWindowStationW(name, flags, access, &attributes)` and `A`: a new one (named `Service-0x0-N$` without a
/// name), or the one by that name unless CWF_CREATE_ONLY (1).
fn create_window_station(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = name_arg(task, cx, a[0], wide);
    let made = task.win32k().stations.station(name.as_deref(), true, a[1] & 1 != 0);
    match made {
        Ok(handle) => {
            keep_descriptor(task, cx, handle, a[3]);
            Reply::Value(handle)
        }
        Err(exists) => failed(task, cx, exists),
    }
}

pub fn create_window_station_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_window_station(task, cx, a, true)
}

pub fn create_window_station_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_window_station(task, cx, a, false)
}

/// `OpenWindowStationW(name, inherit, access)` and `A`.
fn open_window_station(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = name_arg(task, cx, a[0], wide);
    let found = task.win32k().stations.station(name.as_deref(), false, false);
    match found {
        Ok(handle) => Reply::Value(handle),
        Err(_) => failed(task, cx, false),
    }
}

pub fn open_window_station_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_window_station(task, cx, a, true)
}

pub fn open_window_station_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_window_station(task, cx, a, false)
}

/// `CreateDesktopW(name, device, mode, flags, access, &attributes)` and `A`: in the process's window station.
fn create_desktop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = name_arg(task, cx, a[0], wide).unwrap_or_default();
    let pid = task.process.pid;
    let made = {
        let mut w = task.win32k();
        let station = w.stations.current(pid);
        w.stations.desktop(station, &name, true)
    };
    match made {
        Ok(handle) => {
            keep_descriptor(task, cx, handle, a[5]);
            Reply::Value(handle)
        }
        Err(exists) => failed(task, cx, exists),
    }
}

pub fn create_desktop_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_desktop(task, cx, a, true)
}

pub fn create_desktop_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_desktop(task, cx, a, false)
}

/// `OpenDesktopW(name, flags, inherit, access)` and `A`: in the process's window station.
fn open_desktop_named(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = name_arg(task, cx, a[0], wide).unwrap_or_default();
    let pid = task.process.pid;
    let found = {
        let mut w = task.win32k();
        let station = w.stations.current(pid);
        w.stations.desktop(station, &name, false)
    };
    match found {
        Ok(handle) => Reply::Value(handle),
        Err(_) => failed(task, cx, false),
    }
}

pub fn open_desktop_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_desktop_named(task, cx, a, true)
}

pub fn open_desktop_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_desktop_named(task, cx, a, false)
}

fn close_user_object(task: &mut Task, cx: &mut Cx<'_>, handle: u64, station: bool) -> Reply {
    let closed = task.win32k().stations.close(handle, station);
    if closed {
        if handle != WINDOW_STATION && handle != DESKTOP {
            lock(&task.k.user_security).remove(&handle);
        }
        Reply::Value(TRUE)
    } else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        Reply::Value(FALSE)
    }
}

/// `CloseWindowStation(station)`: one the program made goes, with its desktops.
pub fn close_window_station(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    close_user_object(task, cx, a[0], true)
}

pub fn close_desktop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    close_user_object(task, cx, a[0], false)
}

/// `SetProcessWindowStation(station)`: where the process's next desktops are made and opened.
pub fn set_process_window_station(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = task.process.pid;
    if task.win32k().stations.set_current(pid, a[0]) {
        Reply::Value(TRUE)
    } else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        Reply::Value(FALSE)
    }
}

/// `SetThreadDesktop(desktop)`: any desktop; every desktop's windows go on the one screen.
pub fn set_thread_desktop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.win32k().stations.get(a[0]).is_some_and(|o| !o.is_station()) {
        Reply::Value(TRUE)
    } else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        Reply::Value(FALSE)
    }
}

/// `RegisterSuspendResumeNotification(recipient, flags)` and `RegisterPowerSettingNotification(recipient, &setting,
/// flags)`: a handle to unregister with. The machine never sleeps and its power settings never change, so nothing is
/// ever sent.
pub fn register_notification(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0x5300)
}

/// `GetCaretBlinkTime()`: Windows' default, in milliseconds.
pub fn get_caret_blink_time(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(530)
}
