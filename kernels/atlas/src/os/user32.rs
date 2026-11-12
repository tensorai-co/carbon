//! user32.dll: windows, messages and painting, over `win32k`. Wide (W) functions only, for now, but the message box.

use display::Rect;
use display::desktop::{self as dt, Button, Input, Insets};
use display::selection::{Selection, TEXT};
use kernels_core::thread::Cx;
use kernels_core::user;

use super::resapi::{load_string_a, load_string_w};
use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::{self, Class, DESKTOP, Filter, Msg, WM_GETTEXT, WM_GETTEXTLENGTH, WM_SETTEXT, gdi, keys};

dll! {
    "USER32.dll";
    host MessageBoxW(4) => message_box_w;
    host MessageBoxA(4) => message_box_a;
    host LoadStringW(4) => load_string_w;
    host LoadStringA(4) => load_string_a;
    host RegisterClassExW(1) => register_class_ex_w;
    host RegisterClassW(1) => register_class_w;
    host UnregisterClassW(2) => unregister_class_w;
    host CreateWindowExW(12) => create_window_ex_w;
    host DestroyWindow(1) => destroy_window;
    host DefWindowProcW(4) => def_window_proc_w;
    host CallWindowProcW(5) => call_window_proc_w;
    host ShowWindow(2) => show_window;
    host UpdateWindow(1) => update_window;
    host GetMessageW(4) => get_message_w;
    host PeekMessageW(5) => peek_message_w;
    host TranslateMessage(1) => translate_message;
    host DispatchMessageW(1) => dispatch_message_w;
    host SendMessageW(4) => send_message_w;
    host PostMessageW(4) => post_message_w;
    host PostThreadMessageW(4) => post_thread_message_w;
    host PostQuitMessage(1) => post_quit_message;
    host SetWindowTextW(2) => set_window_text_w;
    host GetWindowTextW(3) => get_window_text_w;
    host GetWindowTextLengthW(1) => get_window_text_length_w;
    host GetWindowLongPtrW(2) => get_window_long_ptr_w;
    host SetWindowLongPtrW(3) => set_window_long_ptr_w;
    host GetWindowLongW(2) => get_window_long_ptr_w;
    host SetWindowLongW(3) => set_window_long_ptr_w;
    host GetClassLongPtrW(2) => get_class_long_ptr_w;
    host GetClientRect(2) => get_client_rect;
    host GetWindowRect(2) => get_window_rect;
    host MoveWindow(6) => move_window;
    host SetWindowPos(7) => set_window_pos;
    host AdjustWindowRect(3) => adjust_window_rect;
    host AdjustWindowRectEx(4) => adjust_window_rect;
    host IsWindow(1) => is_window;
    host IsWindowVisible(1) => is_window_visible;
    host IsZoomed(1) => is_zoomed;
    host IsIconic(1) => is_iconic;
    host GetParent(1) => get_parent;
    host GetDesktopWindow(0) => get_desktop_window;
    host FindWindowW(2) => find_window_w;
    host EnumWindows(2) => enum_windows;
    host GetWindow(2) => get_window;
    host GetTopWindow(1) => get_top_window;
    host EnumChildWindows(3) => enum_child_windows;
    host GetWindowThreadProcessId(2) => get_window_thread_process_id;
    host SetTimer(4) => set_timer;
    host KillTimer(2) => kill_timer;
    host InvalidateRect(3) => invalidate_rect;
    host ValidateRect(2) => validate_rect;
    host BeginPaint(2) => begin_paint;
    host EndPaint(2) => end_paint;
    host GetDC(1) => get_dc;
    host ReleaseDC(2) => release_dc;
    host FillRect(3) => fill_rect;
    host FrameRect(3) => frame_rect;
    host DrawTextW(5) => draw_text_w;
    host GetSysColor(1) => get_sys_color;
    host GetSysColorBrush(1) => get_sys_color_brush;
    host GetSystemMetrics(1) => get_system_metrics;
    host LoadCursorW(2) => load_resource;
    host LoadIconW(2) => load_resource;
    host SetCursor(1) => set_cursor;
    host GetCursor(0) => get_cursor;
    host GetFocus(0) => get_focus;
    host OpenClipboard(1) => open_clipboard;
    host CloseClipboard(0) => close_clipboard;
    host EmptyClipboard(0) => empty_clipboard;
    host SetClipboardData(2) => set_clipboard_data;
    host GetClipboardData(1) => get_clipboard_data;
    host IsClipboardFormatAvailable(1) => is_clipboard_format_available;
    host CountClipboardFormats(0) => count_clipboard_formats;
    host EnumClipboardFormats(1) => enum_clipboard_formats;
    host GetClipboardOwner(0) => get_clipboard_owner;
    host RegisterClipboardFormatW(1) => register_clipboard_format_w;
    host GetDpiForWindow(1) => get_dpi;
    host GetDpiForSystem(0) => get_dpi;
    host SetCapture(1) => set_capture;
    host GetCapture(0) => get_capture;
    host ReleaseCapture(0) => release_capture;
    host SetFocus(1) => set_focus;
    host GetActiveWindow(0) => get_active_window;
    host GetForegroundWindow(0) => get_active_window;
    host SetActiveWindow(1) => set_active_window;
    host SetForegroundWindow(1) => activate_window;
    host FlashWindow(2) => flash_window;
    host FlashWindowEx(1) => flash_window_ex;
    host ClipCursor(1) => clip_cursor;
    host GetClipCursor(1) => get_clip_cursor;
    host BringWindowToTop(1) => activate_window;
    host GetKeyState(1) => get_key_state;
    host GetAsyncKeyState(1) => get_async_key_state;
    host SendInput(3) => send_input;
    host SetCursorPos(2) => set_cursor_pos;
    host GetCursorPos(1) => get_cursor_pos;
    host ClientToScreen(2) => client_to_screen;
    host ScreenToClient(2) => screen_to_client;
    host WindowFromPoint(1) => window_from_point;
    missing CreateWindowExA;
    missing RegisterClassExA;
    missing DefWindowProcA;
    missing GetMessageA;
    missing DispatchMessageA;
}

const TRUE: u64 = 1;
const FALSE: u64 = 0;

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}



/// `LoadCursorW` and `LoadIconW`: something that isn't null, until there are resources.
fn load_resource(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(0x0001_0000 | (a[1] & 0xffff))
}

fn message_box_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let text = task.wstr(cx, a[1]);
    let title = if a[2] == 0 { "Error".to_string() } else { task.wstr(cx, a[2]) };
    task.k.host.write(host::Stream::Err, format!("[message box] {title}: {text}\n").as_bytes());
    Reply::Value(1)
}

fn message_box_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let read = |task: &mut Task, cx: &mut Cx<'_>, at: u64| task.mem(cx, false, |t| user::read_cstr(&t.space(), at, 4096)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let text = read(task, cx, a[1]);
    let title = if a[2] == 0 { "Error".to_string() } else { read(task, cx, a[2]) };
    task.k.host.write(host::Stream::Err, format!("[message box] {title}: {text}\n").as_bytes());
    Reply::Value(1)
}

/// A WNDCLASS(EX)W, from where its fields start (after `cbSize` and `style` for the EX form).
fn register(task: &mut Task, cx: &mut Cx<'_>, at: u64, ex: bool) -> Reply {
    let size = if ex { 80 } else { 72 };
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, size)) else { return Reply::Value(0) };
    let get = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default());
    let get32 = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
    // WNDCLASSEXW: cbSize, style, lpfnWndProc, cbClsExtra, cbWndExtra, hInstance, hIcon, hCursor, hbrBackground,
    // lpszMenuName, lpszClassName, hIconSm. WNDCLASSW is the same from `style`, with no cbSize and no hIconSm.
    let (style, wndproc, extra, instance, background, name_at) = if ex {
        (get32(4), get(8), get32(20), get(24), get(48), get(64))
    } else {
        (get32(0), get(8), get32(20), get(24), get(48), get(64))
    };
    let cursor = get(40);
    let name = if name_at < 0x1_0000 { format!("#{name_at}") } else { task.wstr(cx, name_at) };
    match task.register_class(Class { name, atom: 0, wndproc, style, extra: extra as usize, background, instance, cursor }) {
        Ok(atom) => Reply::Value(u64::from(atom)),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0)
        }
    }
}

fn register_class_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], true)
}

fn register_class_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], false)
}

fn unregister_class_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.class_named(cx, a[0]).unwrap_or_default();
    Reply::Value(u64::from(task.unregister_class(&name)))
}

fn create_window_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.create_window(cx, a))
}

fn destroy_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.destroy_window(cx, a[0]) {
        Ok(done) => Reply::Value(u64::from(done)),
        Err(reply) => reply,
    }
}

fn def_window_proc_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.def_window_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn call_window_proc_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 {
        return Reply::Value(0);
    }
    value(task.upcall(cx, a[0], &[a[1], a[2] & 0xffff_ffff, a[3], a[4]]))
}

fn show_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.show_window(cx, a[0], a[1] as u32) {
        Ok(was) => Reply::Value(u64::from(was)),
        Err(reply) => reply,
    }
}

fn update_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let needs = task.win32k().windows.get(&a[0]).map(|w| w.visible && !w.invalid.is_empty());
    match needs {
        None => Reply::Value(FALSE),
        Some(false) => Reply::Value(TRUE),
        Some(true) => match task.send_message(cx, a[0], win32k::WM_PAINT, 0, 0) {
            Ok(_) => Reply::Value(TRUE),
            Err(reply) => reply,
        },
    }
}

fn filter(a: &[u64]) -> Filter {
    Filter { hwnd: if a[1] as u32 == u32::MAX { u64::MAX } else { a[1] }, min: a[2] as u32, max: a[3] as u32 }
}

fn get_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.next_message(cx, filter(a), true, true) {
        Ok(Some(m)) => {
            let bytes = m.encode();
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &bytes)).is_err() {
                return Reply::Value(u64::from(u32::MAX));
            }
            Reply::Value(u64::from(m.message != win32k::WM_QUIT))
        }
        Ok(None) => Reply::Value(0),
        Err(reply) => reply,
    }
}

fn peek_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const PM_REMOVE: u64 = 1;
    match task.next_message(cx, filter(a), a[4] & PM_REMOVE != 0, false) {
        Ok(Some(m)) => {
            let bytes = m.encode();
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &bytes));
            Reply::Value(TRUE)
        }
        Ok(None) => Reply::Value(FALSE),
        Err(reply) => reply,
    }
}

/// No keyboard yet, so there's nothing to translate.
/// `TranslateMessage`: for a key that types something, as the thread's key state has it, WM_CHAR (or WM_SYSCHAR),
/// to be taken next. TRUE for any key-down message.
fn translate_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 48)) else { return Reply::Value(FALSE) };
    let m = Msg::decode(&b);
    if !matches!(m.message, win32k::WM_KEYDOWN | win32k::WM_SYSKEYDOWN) {
        return Reply::Value(FALSE);
    }
    let tid = task.tid;
    let mut w = task.win32k();
    let held = w.thread_keys(tid);
    let mut modifiers = 0;
    for (vk, bit) in [(keys::VK_SHIFT, display::keys::SHIFT), (keys::VK_CONTROL, display::keys::CTRL), (keys::VK_MENU, display::keys::ALT)] {
        if held[vk as usize] & 0x80 != 0 {
            modifiers |= bit;
        }
    }
    // Ctrl+Alt types nothing here (it's AltGr on other layouts).
    let both = display::keys::CTRL | display::keys::ALT;
    if let Some(c) = keys::code(m.wparam as u8).and_then(|code| display::keys::text(code, modifiers)).filter(|_| modifiers & both != both) {
        let message = if m.message == win32k::WM_SYSKEYDOWN { win32k::WM_SYSCHAR } else { win32k::WM_CHAR };
        w.post_front(m.hwnd, message, u64::from(c), m.lparam, m.time);
    }
    Reply::Value(TRUE)
}

fn dispatch_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 48)) else { return Reply::Value(0) };
    value(task.dispatch(cx, Msg::decode(&b)))
}

fn send_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.send_message(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn post_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] != 0 && !task.window_exists(a[0]) {
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(FALSE);
    }
    let tid = task.tid;
    Reply::Value(u64::from(task.post_message(a[0], tid, a[1] as u32, a[2], a[3])))
}

fn post_thread_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.post_message(0, a[0] & 0xffff_ffff, a[1] as u32, a[2], a[3]) {
        task.set_last_error(cx, 1444); // ERROR_INVALID_THREAD_ID
        return Reply::Value(FALSE);
    }
    Reply::Value(TRUE)
}

fn post_quit_message(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.post_quit(a[0] & 0xffff_ffff);
    Reply::Value(0)
}

fn set_window_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.send_message(cx, a[0], WM_SETTEXT, 0, a[1]))
}

fn get_window_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.send_message(cx, a[0], WM_GETTEXT, a[2] & 0xffff_ffff, a[1]))
}

fn get_window_text_length_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.send_message(cx, a[0], WM_GETTEXTLENGTH, 0, 0))
}

const GWLP_WNDPROC: i32 = -4;
const GWLP_HINSTANCE: i32 = -6;
const GWLP_HWNDPARENT: i32 = -8;
const GWLP_ID: i32 = -12;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const GWLP_USERDATA: i32 = -21;

fn get_window_long_ptr_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(win) = w.windows.get(&a[0]) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let index = a[1] as i32;
    Reply::Value(match index {
        GWLP_WNDPROC => win.wndproc,
        GWLP_HINSTANCE => win.instance,
        GWLP_HWNDPARENT => if win.parent == DESKTOP { 0 } else { win.parent },
        GWLP_ID => win.id,
        GWL_STYLE => u64::from(win.style | if win.visible { 0x1000_0000 } else { 0 }),
        GWL_EXSTYLE => u64::from(win.ex_style),
        GWLP_USERDATA => win.userdata,
        i if i >= 0 => {
            let at = i as usize;
            let mut b = [0u8; 8];
            let n = win.extra.len().saturating_sub(at).min(8);
            b[..n].copy_from_slice(&win.extra[at..at + n]);
            u64::from_le_bytes(b)
        }
        _ => 0,
    })
}

fn set_window_long_ptr_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(win) = w.windows.get_mut(&a[0]) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let new = a[2];
    let old = match a[1] as i32 {
        GWLP_WNDPROC => std::mem::replace(&mut win.wndproc, new),
        GWLP_HINSTANCE => std::mem::replace(&mut win.instance, new),
        GWLP_ID => std::mem::replace(&mut win.id, new),
        GWL_STYLE => u64::from(std::mem::replace(&mut win.style, new as u32 & !0x1000_0000)),
        GWL_EXSTYLE => u64::from(std::mem::replace(&mut win.ex_style, new as u32)),
        GWLP_USERDATA => std::mem::replace(&mut win.userdata, new),
        i if i >= 0 && (i as usize) + 8 <= win.extra.len() => {
            let at = i as usize;
            let old = u64::from_le_bytes(win.extra[at..at + 8].try_into().unwrap_or_default());
            win.extra[at..at + 8].copy_from_slice(&new.to_le_bytes());
            old
        }
        _ => {
            drop(w);
            task.set_last_error(cx, 1413); // ERROR_INVALID_INDEX
            return Reply::Value(0);
        }
    };
    Reply::Value(old)
}

fn get_class_long_ptr_w(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(class) = w.windows.get(&a[0]).and_then(|win| w.class_by_name(&win.class)) else { return Reply::Value(0) };
    Reply::Value(match a[1] as i32 {
        -10 => class.background,                // GCLP_HBRBACKGROUND
        -14 => class.instance,                  // GCLP_HMODULE
        -24 => class.wndproc,                   // GCLP_WNDPROC
        -26 => u64::from(class.style),          // GCL_STYLE
        -32 => u64::from(class.atom),           // GCW_ATOM
        -18 => class.extra as u64,              // GCL_CBWNDEXTRA
        _ => 0,
    })
}

fn get_client_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.win32k().windows.get(&a[0]).map(|w| w.client()) else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(task.write_rect(cx, a[1], &r)))
}

/// `GetWindowRect`: the whole window, frame and all, on the screen.
fn get_window_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let found = {
        let w = task.win32k();
        w.windows.get(&a[0]).map(|win| {
            if win.top_level() {
                win.rect
            } else {
                let (x, y) = w.screen_origin(win.parent).unwrap_or_default();
                win.rect.offset(x, y)
            }
        })
    };
    let Some(r) = found else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(task.write_rect(cx, a[1], &r)))
}

fn move_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.place_window(cx, a[0], a[1] as i32, a[2] as i32, a[3] as i32, a[4] as i32, 0) {
        Ok(done) => Reply::Value(u64::from(done)),
        Err(reply) => reply,
    }
}

/// `SetWindowPos`: its place and size, then its place in the stacking order (HWND_TOP, HWND_TOPMOST,
/// HWND_NOTOPMOST), then activating it unless SWP_NOACTIVATE.
fn set_window_pos(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const NOZORDER: u32 = 4;
    const NOACTIVATE: u32 = 0x10;
    let (hwnd, after, flags) = (a[0], a[1] as i64, a[6] as u32);
    match task.place_window(cx, hwnd, a[2] as i32, a[3] as i32, a[4] as i32, a[5] as i32, flags) {
        Ok(true) => {}
        Ok(false) => return Reply::Value(FALSE),
        Err(reply) => return reply,
    }
    let (shown, visible) = task.win32k().windows.get(&hwnd).map_or((0, false), |w| (w.shown, w.visible));
    if shown != 0 && flags & NOZORDER == 0 {
        // Another window to go behind: its top-level window's place on the desktop.
        let behind = if after > 1 { shown_of(task, after as u64) } else { None };
        task.desktop(|d| match after {
            -1 => {
                d.set_layer(shown, dt::Layer::Top);
                d.raise(shown);
            }
            -2 => d.set_layer(shown, dt::Layer::Normal),
            0 => d.raise(shown),
            // HWND_BOTTOM: below the rest, no longer topmost.
            1 => {
                d.set_layer(shown, dt::Layer::Normal);
                d.stack(shown, dt::Stack::Bottom);
            }
            _ => {
                if let Some(b) = behind {
                    d.stack(shown, dt::Stack::Below(b));
                }
            }
        });
    }
    if shown != 0
        && visible
        && flags & NOACTIVATE == 0
        && let Err(reply) = task.activate(cx, hwnd)
    {
        return reply;
    }
    Reply::Value(TRUE)
}

/// `AdjustWindowRect(Ex)`: the window a client rectangle needs, with the frame its style gets.
fn adjust_window_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.read_rect(cx, a[0]) else { return Reply::Value(FALSE) };
    let r = Insets::of(win32k::framed(a[1] as u32)).around(&r);
    Reply::Value(u64::from(task.write_rect(cx, a[0], &r)))
}

fn is_window(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.window_exists(a[0])))
}

/// Whether a top-level window is in a state on the desktop: maximized, or minimized.
fn top_level_state(task: &mut Task, hwnd: u64, state: u32) -> Reply {
    let shown = task.win32k().windows.get(&hwnd).map_or(0, |w| w.shown);
    let on = shown != 0 && task.desktop(|d| d.states(shown) & state != 0);
    Reply::Value(u64::from(on))
}

fn is_zoomed(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    top_level_state(task, a[0], display::desktop::state::MAXIMIZED)
}

fn is_iconic(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    top_level_state(task, a[0], display::desktop::state::MINIMIZED)
}

fn is_window_visible(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    // Visible only if it and every window it's in are.
    let mut at = a[0];
    loop {
        match w.windows.get(&at) {
            Some(win) if !win.visible => return Reply::Value(FALSE),
            Some(win) if win.parent == DESKTOP => return Reply::Value(TRUE),
            Some(win) => at = win.parent,
            None => return Reply::Value(FALSE),
        }
    }
}

fn get_parent(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().windows.get(&a[0]).map_or(0, |w| if w.parent == DESKTOP { 0 } else { w.parent }))
}

fn get_desktop_window(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(DESKTOP)
}

fn find_window_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let class = if a[0] == 0 { None } else { task.class_named(cx, a[0]) };
    let title = (a[1] != 0).then(|| task.wstr(cx, a[1]));
    let w = task.win32k();
    let found = w.windows.iter().find(|(_, win)| {
        win.parent == DESKTOP && class.as_ref().is_none_or(|c| *c == win.class) && title.as_ref().is_none_or(|t| *t == win.text)
    });
    Reply::Value(found.map_or(0, |(&h, _)| h))
}

/// Calls `callback(hwnd, lParam)` for each window, until it returns FALSE.
fn enumerate(task: &mut Task, cx: &mut Cx<'_>, windows: Vec<u64>, callback: u64, lparam: u64) -> Reply {
    for hwnd in windows {
        match task.upcall(cx, callback, &[hwnd, lparam]) {
            Ok(0) => return Reply::Value(FALSE),
            Ok(_) => {}
            Err(reply) => return reply,
        }
    }
    Reply::Value(TRUE)
}

fn enum_windows(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    // Top first, as Windows enumerates them.
    let top = task.win32k().z_order(DESKTOP);
    enumerate(task, cx, top, a[0], a[1])
}

/// `GetWindow`: GW_HWNDFIRST, GW_HWNDLAST, GW_HWNDNEXT (the one below) and GW_HWNDPREV (above) among its siblings
/// in z-order, GW_OWNER, GW_CHILD (its top child); 0 for none.
fn get_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.win32k();
    let Some(win) = w.windows.get(&a[0]) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let (parent, owner) = (win.parent, win.owner);
    let siblings = || w.z_order(parent);
    let at = |z: &[u64]| z.iter().position(|&h| h == a[0]);
    Reply::Value(match a[1] {
        0 => siblings().first().copied().unwrap_or(0),
        1 => siblings().last().copied().unwrap_or(0),
        2 => {
            let z = siblings();
            at(&z).and_then(|i| z.get(i + 1)).copied().unwrap_or(0)
        }
        3 => {
            let z = siblings();
            at(&z).and_then(|i| i.checked_sub(1)).map_or(0, |i| z[i])
        }
        4 => owner,
        5 => w.z_order(a[0]).first().copied().unwrap_or(0),
        _ => 0,
    })
}

/// `GetTopWindow`: a window's top child, or with NULL, the top top-level window.
fn get_top_window(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let parent = if a[0] == 0 { DESKTOP } else { a[0] };
    Reply::Value(task.win32k().z_order(parent).first().copied().unwrap_or(0))
}

fn enum_child_windows(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let children: Vec<u64> = {
        let w = task.win32k();
        let mut out = Vec::new();
        let mut stack = vec![a[0]];
        while let Some(parent) = stack.pop() {
            for (&h, win) in &w.windows {
                if win.parent == parent {
                    out.push(h);
                    stack.push(h);
                }
            }
        }
        out
    };
    enumerate(task, cx, children, a[1], a[2])
}

fn get_window_thread_process_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((tid, pid)) = task.win32k().windows.get(&a[0]).map(|w| (w.tid, w.pid)) else { return Reply::Value(0) };
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], pid as u32));
    }
    Reply::Value(tid)
}

fn set_timer(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.set_timer(a[0], a[1], a[2] as u32, a[3]))
}

fn kill_timer(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.kill_timer(a[0], a[1])))
}

fn invalidate_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let rect = task.read_rect(cx, a[1]);
    Reply::Value(u64::from(task.invalidate(a[0], rect, a[2] as u32 != 0)))
}

fn validate_rect(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(win) = task.win32k().windows.get_mut(&a[0]) {
        win.invalid = Rect::default();
        win.erase = false;
        return Reply::Value(TRUE);
    }
    Reply::Value(FALSE)
}

fn begin_paint(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.begin_paint(cx, a[0], a[1]))
}

fn end_paint(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.end_paint(cx, a[1])))
}

fn get_dc(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.get_dc(a[0]))
}

fn release_dc(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.release_dc(a[1])))
}

/// A brush handle, or a system colour's index plus one, as `FillRect` and classes take them.
fn brush(handle: u64) -> u64 {
    if handle > 0 && handle < 0x100 { gdi::Gdi::sys_brush(handle - 1) } else { handle }
}

fn fill_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(rect) = task.read_rect(cx, a[1]) else { return Reply::Value(0) };
    Reply::Value(u64::from(task.win32k().fill_rect(a[0], rect, brush(a[2]))))
}

fn frame_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(rect) = task.read_rect(cx, a[1]) else { return Reply::Value(0) };
    let mut w = task.win32k();
    let Some(color) = w.gdi.brush_color(brush(a[2])) else { return Reply::Value(0) };
    Reply::Value(u64::from(w.draw(a[0], |s, (dx, dy), clip, _| s.frame(&rect.offset(dx, dy), color, clip))))
}

fn draw_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let len = a[2] as i32;
    let text = if len < 0 {
        task.wstr(cx, a[1])
    } else {
        let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], len as usize * 2)).unwrap_or_default();
        String::from_utf16_lossy(&raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    };
    let Some(mut rect) = task.read_rect(cx, a[3]) else { return Reply::Value(0) };
    let height = task.win32k().draw_text(a[0], &text, &mut rect, a[4] as u32);
    if a[4] as u32 & 0x400 != 0 {
        task.write_rect(cx, a[3], &rect);
    }
    Reply::Value(height as u64)
}

fn get_sys_color(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(gdi::swap(gdi::sys_color(a[0] as u32))))
}

fn get_sys_color_brush(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(gdi::Gdi::sys_brush(a[0] & 0xff))
}

/// `GetDpiForWindow` and `GetDpiForSystem`: 96, as a program that isn't DPI aware is told (`win32k::PROGRAM_DPI`);
/// its windows are magnified to the output's scale.
fn get_dpi(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(win32k::PROGRAM_DPI))
}

fn get_system_metrics(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(match a[0] as u32 {
        0 | 16 | 78 => task.desktop(|d| d.size().0) as u64,
        1 | 17 | 79 => task.desktop(|d| d.size().1) as u64,
        // SM_CYCAPTION; SM_CXBORDER and SM_CYBORDER; SM_CXFRAME and SM_CYFRAME: the desktop's frames.
        4 => dt::CAPTION as u64,
        5 | 6 => 1,
        32 | 33 => dt::BORDER as u64,
        // No scroll bars or menus of atlas's own yet.
        _ => 0,
    })
}

/// `SetCapture`: every mouse message to this window, wherever the pointer is, until `ReleaseCapture`. The window that
/// had it.
fn set_capture(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (old, shown) = {
        let mut w = task.win32k();
        if !w.windows.contains_key(&a[0]) {
            return Reply::Value(0);
        }
        let shown = w.top_level_of(a[0]).and_then(|t| w.windows.get(&t)).map_or(0, |t| t.shown);
        (std::mem::replace(&mut w.capture, a[0]), shown)
    };
    if shown != 0 {
        task.desktop(|d| d.set_capture(Some(shown)));
    }
    Reply::Value(old)
}

/// `SetCursor`: what the pointer looks like now, over the window it's in (if it's this machine's), else the active
/// one's. The cursor before.
fn set_cursor(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let old = std::mem::replace(&mut task.win32k().cursor, a[0]);
    task.desktop(|d| {
        if let Some(id) = d.hovered().or(d.capture()).or(d.active()) {
            d.set_cursor(id, crate::win32k::cursor_of(a[0]));
        }
    });
    Reply::Value(old)
}

fn get_cursor(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().cursor)
}

fn get_capture(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().capture)
}

fn release_capture(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.win32k().capture = 0;
    task.desktop(|d| d.set_capture(None));
    task.pump();
    Reply::Value(TRUE)
}

fn get_focus(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().focus)
}

fn set_focus(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.set_focus(cx, a[0]))
}

fn get_active_window(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.win32k().active)
}

/// `SetActiveWindow`: the window that was active.
fn set_active_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.window_exists(a[0]) {
        return Reply::Value(0);
    }
    value(task.activate(cx, a[0]))
}

/// `SetForegroundWindow` and `BringWindowToTop`: raised and active.
/// `SetForegroundWindow`: TRUE if it's the foreground window now, FALSE if the foreground rules refused it (and it
/// flashes instead).
fn activate_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.window_exists(a[0]) {
        return Reply::Value(FALSE);
    }
    if let Err(reply) = task.activate(cx, a[0]) {
        return reply;
    }
    let w = task.win32k();
    Reply::Value(u64::from(w.top_level_of(a[0]).is_some_and(|t| t == w.active)))
}

/// A top-level window's place on the desktop, and whether it's the active window.
fn shown_and_active(task: &mut Task, hwnd: u64) -> Option<(u64, bool)> {
    let shown = shown_of(task, hwnd)?;
    let w = task.win32k();
    Some((shown, w.top_level_of(hwnd).is_some_and(|t| t == w.active)))
}

/// `FlashWindow`: with `bInvert`, the window asks for the user's attention (until it's active). Whether it was active.
fn flash_window(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((shown, active)) = shown_and_active(task, a[0]) else { return Reply::Value(FALSE) };
    task.desktop(|d| d.set_attention(shown, a[1] != 0));
    Reply::Value(u64::from(active))
}

/// `FlashWindowEx`: a FLASHWINFO (cbSize, hwnd, dwFlags, uCount, dwTimeout); FLASHW_STOP (no flags) stops it.
fn flash_window_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 24)) else { return Reply::Value(FALSE) };
    let hwnd = u64::from_le_bytes(b[8..16].try_into().unwrap_or_default());
    let flags = u32::from_le_bytes(b[16..20].try_into().unwrap_or_default());
    let Some((shown, active)) = shown_and_active(task, hwnd) else { return Reply::Value(FALSE) };
    task.desktop(|d| d.set_attention(shown, flags != 0));
    Reply::Value(u64::from(active))
}

/// `ClipCursor`: keeps the pointer within a rectangle on the screen, or with NULL, lets it go anywhere. The desktop
/// keeps it for the active window, while that's active.
fn clip_cursor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let old = task.win32k().clip.take();
    if let Some((shown, _)) = old {
        task.desktop(|d| d.set_pointer_constraint(shown, None));
    }
    if a[0] != 0 {
        let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 16)) else { return Reply::Value(FALSE) };
        let at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
        let (left, top) = (at(0), at(4));
        let r = Rect::new(left, top, at(8).max(left + 1), at(12).max(top + 1));
        let shown = task.desktop(|d| {
            let shown = d.active()?;
            let c = d.content_rect(shown)?;
            d.set_pointer_constraint(shown, Some(dt::Constraint::Confine(r.offset(-c.left, -c.top))));
            Some(shown)
        });
        if let Some(shown) = shown {
            task.win32k().clip = Some((shown, r));
        }
    }
    task.pump();
    Reply::Value(TRUE)
}

/// `GetClipCursor`: the rectangle `ClipCursor` set, or the whole screen.
fn get_clip_cursor(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let clip = task.win32k().clip.map(|(_, r)| r);
    let r = clip.unwrap_or_else(|| {
        let (w, h) = task.desktop(|d| d.size());
        Rect::new(0, 0, w, h)
    });
    let b: Vec<u8> = [r.left, r.top, r.right, r.bottom].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &b)).is_ok()))
}

/// A key state as `GetKeyState` gives it: a SHORT, negative while down, its low bit toggled by each press.
fn key_state(state: u8) -> Reply {
    let short = (if state & 0x80 != 0 { 0x8000u16 } else { 0 }) | u16::from(state & 1);
    Reply::Value(short as i16 as i64 as u64)
}

/// `GetKeyState`: as of the input messages this thread has taken.
fn get_key_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let tid = task.tid;
    key_state(task.win32k().thread_keys(tid)[(a[0] & 0xff) as usize])
}

/// `GetAsyncKeyState`: as the keys are now.
fn get_async_key_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    key_state(task.win32k().keys[(a[0] & 0xff) as usize] & 0x80)
}

/// `SendInput`: mouse and keyboard input, onto the desktop as if from the user. INPUT is 40 bytes: its type, then a
/// MOUSEINPUT (dx, dy, mouseData, dwFlags) or KEYBDINPUT (wVk, wScan, dwFlags) at 8.
fn send_input(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (count, at, size) = (a[0] as usize & 0xffff_ffff, a[1], a[2] as usize & 0xffff_ffff);
    if size != 40 {
        task.set_last_error(cx, 87);
        return Reply::Value(0);
    }
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, count * 40)) else { return Reply::Value(0) };
    let shared = task.win32k().display.clone();
    let ((width, height), mut pointer) = {
        let d = dt::lock(&shared);
        (d.size(), d.pointer())
    };
    let mut inputs = Vec::new();
    for e in b.as_chunks::<40>().0 {
        let u32_at = |o: usize| u32::from_le_bytes(e[o..o + 4].try_into().unwrap_or_default());
        let u16_at = |o: usize| u16::from_le_bytes(e[o..o + 2].try_into().unwrap_or_default());
        match u32_at(0) {
            0 => {
                let (dx, dy, data, flags) = (u32_at(8) as i32, u32_at(12) as i32, u32_at(16) as i32, u32_at(20));
                if flags & 1 != 0 {
                    // MOUSEEVENTF_ABSOLUTE: 0 to 65535 across the screen.
                    pointer = if flags & 0x8000 != 0 {
                        ((i64::from(dx) * i64::from(width) / 65536) as i32, (i64::from(dy) * i64::from(height) / 65536) as i32)
                    } else {
                        (pointer.0 + dx, pointer.1 + dy)
                    };
                    inputs.push(Input::PointerTo(pointer.0, pointer.1));
                }
                let buttons = [(2, Button::Left, true), (4, Button::Left, false), (8, Button::Right, true), (0x10, Button::Right, false), (0x20, Button::Middle, true), (0x40, Button::Middle, false)];
                for (bit, button, down) in buttons {
                    if flags & bit != 0 {
                        inputs.push(Input::Button(button, down));
                    }
                }
                // MOUSEEVENTF_WHEEL and MOUSEEVENTF_HWHEEL, in WHEEL_DELTA's 120ths of a notch, as they come.
                if flags & 0x800 != 0 {
                    inputs.push(Input::Scroll { x: 0, y: data });
                }
                if flags & 0x1000 != 0 {
                    inputs.push(Input::Scroll { x: data, y: 0 });
                }
            }
            1 => {
                let (vk, scan, flags) = (u16_at(8), u16_at(10), u32_at(12));
                let up = flags & 2 != 0;
                if flags & 4 != 0 {
                    // KEYEVENTF_UNICODE: the key that types it, on the desktop's US layout, with shift if it needs it.
                    let c = char::from_u32(u32::from(scan)).filter(|_| !up);
                    if let Some((code, shift)) = c.and_then(display::keys::for_char) {
                        if shift {
                            inputs.push(Input::Key(display::keys::LEFT_SHIFT, true));
                        }
                        inputs.extend([Input::Key(code, true), Input::Key(code, false)]);
                        if shift {
                            inputs.push(Input::Key(display::keys::LEFT_SHIFT, false));
                        }
                    } else if let Some(c) = c {
                        // No key types it: text, as Windows sends a VK_PACKET's WM_CHAR.
                        inputs.push(Input::Text(c.to_string()));
                    }
                } else {
                    // KEYEVENTF_SCANCODE, with KEYEVENTF_EXTENDEDKEY; else the virtual key.
                    let code = if flags & 8 != 0 { keys::from_scan(scan, flags & 1 != 0) } else { keys::code(vk as u8) };
                    if let Some(code) = code {
                        inputs.push(Input::Key(code, !up));
                    }
                }
            }
            _ => {}
        }
    }
    for input in inputs {
        dt::send(&shared, input);
    }
    Reply::Value(count as u64)
}

fn set_cursor_pos(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let shared = task.win32k().display.clone();
    dt::send(&shared, Input::PointerTo(a[0] as i32, a[1] as i32));
    Reply::Value(TRUE)
}

fn get_cursor_pos(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y) = task.desktop(|d| d.pointer());
    let point: Vec<u8> = [x, y].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &point)).is_ok()))
}

/// A POINT in the program's memory, moved by a window's client area's place on the screen (`sign` 1 or -1).
fn convert_point(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], sign: i32) -> Reply {
    let Some((ox, oy)) = task.win32k().screen_origin(a[0]) else { return Reply::Value(FALSE) };
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], 8)) else { return Reply::Value(FALSE) };
    let at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
    let point: Vec<u8> = [at(0) + sign * ox, at(4) + sign * oy].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(u64::from(task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &point)).is_ok()))
}

fn client_to_screen(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_point(task, cx, a, 1)
}

fn screen_to_client(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    convert_point(task, cx, a, -1)
}

/// `WindowFromPoint`: a POINT passed by value, in one register. The deepest window there: a top-level window for its
/// frame, and 0 for the desktop's background.
fn window_from_point(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (x, y) = (a[0] as u32 as i32, (a[0] >> 32) as u32 as i32);
    let Some(shown) = task.desktop(|d| d.window_at(x, y)) else { return Reply::Value(0) };
    let w = task.win32k();
    let Some(top) = w.hwnd_of(shown) else { return Reply::Value(0) };
    let Some((ox, oy)) = w.screen_origin(top) else { return Reply::Value(0) };
    let client = w.windows.get(&top).map(win32k::Window::client).unwrap_or_default();
    if !client.contains(x - ox, y - oy) {
        return Reply::Value(top);
    }
    Reply::Value(w.child_at(top, x - ox, y - oy).0)
}

// ---- The clipboard ----
//
// Windows' clipboard over the desktop's: every machine window's, every kernel's, and the host's. Text is one type,
// UTF-8, whichever of CF_UNICODETEXT, CF_TEXT and CF_OEMTEXT it was copied as, and each of those is made from it as
// it's pasted. A format of another number goes as its own bytes; a registered one by its name.


const CF_TEXT: u32 = 1;
const CF_OEMTEXT: u32 = 7;
const CF_UNICODETEXT: u32 = 13;

/// The type a format's data goes as.
fn mime_of(task: &mut Task, format: u32) -> String {
    match format {
        CF_TEXT | CF_OEMTEXT | CF_UNICODETEXT => TEXT.to_string(),
        f if f >= 0xc000 => match task.win32k().clipboard.formats.iter().find(|(_, n)| **n == f) {
            Some((name, _)) => format!("application/x-windows-format;name={name}"),
            None => format!("application/x-windows-format-{f}"),
        },
        f => format!("application/x-windows-format-{f}"),
    }
}

/// The formats a type can be pasted as.
fn formats_of(task: &mut Task, mime: &str) -> Vec<u32> {
    if mime == TEXT {
        return vec![CF_UNICODETEXT, CF_TEXT, CF_OEMTEXT];
    }
    if let Some(name) = mime.strip_prefix("application/x-windows-format;name=") {
        return vec![register_format(task, name)];
    }
    mime.strip_prefix("application/x-windows-format-").and_then(|n| n.parse().ok()).into_iter().collect()
}

fn register_format(task: &mut Task, name: &str) -> u32 {
    let mut w = task.win32k();
    let next = 0xc000 + w.clipboard.formats.len() as u32;
    *w.clipboard.formats.entry(name.to_lowercase()).or_insert(next)
}

/// The window the desktop knows a window by: its top-level window's.
fn shown_of(task: &mut Task, hwnd: u64) -> Option<u64> {
    let w = task.win32k();
    w.top_level_of(hwnd).and_then(|t| w.windows.get(&t)).map(|t| t.shown).filter(|&s| s != 0)
}

/// Text from a format's bytes: UTF-16 for CF_UNICODETEXT, one byte to a character (Latin-1) for the others, to the
/// first NUL.
fn text_from(format: u32, bytes: &[u8]) -> Vec<u8> {
    let text: String = if format == CF_UNICODETEXT {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).take_while(|&u| u != 0).collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect()
    };
    text.into_bytes()
}

/// A format's bytes from text, with its NUL: a character Latin-1 hasn't is `?` in CF_TEXT and CF_OEMTEXT.
fn text_as(format: u32, utf8: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(utf8);
    if format == CF_UNICODETEXT {
        text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    } else {
        text.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).chain([0]).collect()
    }
}

fn open_clipboard(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let opener = if a[0] == 0 { win32k::OPENED_BY_NONE } else { a[0] };
    let mut w = task.win32k();
    match w.clipboard.open {
        Some(by) if by != opener => Reply::Value(FALSE),
        _ => {
            w.clipboard.open = Some(opener);
            Reply::Value(TRUE)
        }
    }
}

fn close_clipboard(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(if task.win32k().clipboard.open.take().is_some() { TRUE } else { FALSE })
}

/// Takes the clipboard for the window that opened it: an offer of nothing yet, which SetClipboardData fills.
fn empty_clipboard(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    let Some(opener) = task.win32k().clipboard.open else { return Reply::Value(FALSE) };
    let owner = if opener == win32k::OPENED_BY_NONE { 0 } else { opener };
    let shown = shown_of(task, owner);
    let source = task.desktop(|d| d.offer(Selection::Clipboard, shown, Vec::new()));
    let handles = {
        let mut w = task.win32k();
        w.clipboard.source = Some(source);
        w.clipboard.owner = owner;
        if let Some(s) = shown {
            w.clipboard.owners.insert(s, owner);
        }
        std::mem::take(&mut w.clipboard.handles)
    };
    for h in handles {
        crate::os::kernel32::global_release(task, cx, h);
    }
    task.pump();
    Reply::Value(TRUE)
}

fn set_clipboard_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (format, handle) = (a[0] as u32, a[1]);
    if task.win32k().clipboard.open.is_none() || handle == 0 {
        return Reply::Value(0);
    }
    let Some(bytes) = crate::os::kernel32::global_read(task, cx, handle) else { return Reply::Value(0) };
    let mime = mime_of(task, format);
    let data = if mime == TEXT { text_from(format, &bytes) } else { bytes };
    let source = task.win32k().clipboard.source;
    let given = source.is_some_and(|s| task.desktop(|d| d.offer_data(s, &mime, data.clone())));
    if !given {
        // Not emptied by this window first, or taken since: an offer of its own.
        let owner = task.win32k().clipboard.owner;
        let shown = shown_of(task, owner);
        let source = task.desktop(|d| {
            let s = d.offer(Selection::Clipboard, shown, Vec::new());
            d.offer_data(s, &mime, data);
            s
        });
        let mut w = task.win32k();
        w.clipboard.source = Some(source);
        if let Some(s) = shown {
            w.clipboard.owners.insert(s, owner);
        }
    }
    task.pump();
    Reply::Value(handle)
}

/// The clipboard's data in a format, in a global block the clipboard keeps; pasted from whoever offered it, and
/// waited for (a second at most) if they send it when asked.
fn get_clipboard_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let format = a[0] as u32;
    if task.win32k().clipboard.open.is_none() {
        return Reply::Value(0);
    }
    let mime = mime_of(task, format);
    let shared = task.win32k().display.clone();
    let Some(data) = dt::paste(&shared, Selection::Clipboard, &mime, std::time::Duration::from_secs(1)) else { return Reply::Value(0) };
    let bytes = if mime == TEXT { text_as(format, &data) } else { data };
    let handle = crate::os::kernel32::global_new(task, cx, &bytes);
    if handle != 0 {
        task.win32k().clipboard.handles.push(handle);
    }
    Reply::Value(handle)
}

/// The formats the clipboard has now, in the order EnumClipboardFormats gives them.
fn clipboard_formats(task: &mut Task) -> Vec<u32> {
    let mimes = task.desktop(|d| d.selection(Selection::Clipboard)).map(|(_, m)| m).unwrap_or_default();
    let mut formats = Vec::new();
    for m in mimes {
        for f in formats_of(task, &m) {
            if !formats.contains(&f) {
                formats.push(f);
            }
        }
    }
    formats
}

fn is_clipboard_format_available(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(clipboard_formats(task).contains(&(a[0] as u32))))
}

fn count_clipboard_formats(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(clipboard_formats(task).len() as u64)
}

fn enum_clipboard_formats(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let formats = clipboard_formats(task);
    let next = if a[0] == 0 { formats.first() } else { formats.iter().skip_while(|&&f| f != a[0] as u32).nth(1) };
    Reply::Value(u64::from(next.copied().unwrap_or(0)))
}

/// The window that emptied the clipboard last, if what it offered is still the clipboard's.
fn get_clipboard_owner(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    let (owner, source) = {
        let w = task.win32k();
        (w.clipboard.owner, w.clipboard.source)
    };
    let still = source.is_some() && task.desktop(|d| d.selection(Selection::Clipboard)).is_some();
    Reply::Value(if still { owner } else { 0 })
}

fn register_clipboard_format_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.wstr(cx, a[0]);
    if name.is_empty() {
        return Reply::Value(0);
    }
    Reply::Value(u64::from(register_format(task, &name)))
}
