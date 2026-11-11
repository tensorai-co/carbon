//! user32.dll: windows, messages and painting, over `win32k`. Wide (W) functions only, for now, but the message box.

use display::Rect;
use display::desktop::{self as dt, Button, Input, Insets};
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::{self, Class, DESKTOP, Filter, Msg, WM_GETTEXT, WM_GETTEXTLENGTH, WM_SETTEXT, gdi, keys};

dll! {
    "USER32.dll";
    host MessageBoxW(4) => message_box_w;
    host MessageBoxA(4) => message_box_a;
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
    host GetParent(1) => get_parent;
    host GetDesktopWindow(0) => get_desktop_window;
    host FindWindowW(2) => find_window_w;
    host EnumWindows(2) => enum_windows;
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
    host SetCursor(1) => zero;
    host GetFocus(0) => get_focus;
    host SetFocus(1) => set_focus;
    host GetActiveWindow(0) => get_active_window;
    host GetForegroundWindow(0) => get_active_window;
    host SetActiveWindow(1) => set_active_window;
    host SetForegroundWindow(1) => activate_window;
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

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
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
    let name = if name_at < 0x1_0000 { format!("#{name_at}") } else { task.wstr(cx, name_at) };
    match task.register_class(Class { name, atom: 0, wndproc, style, extra: extra as usize, background, instance }) {
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
        task.desktop(|d| match after {
            -1 => {
                d.set_layer(shown, dt::Layer::Top);
                d.raise(shown);
            }
            -2 => d.set_layer(shown, dt::Layer::Normal),
            0 => d.raise(shown),
            _ => {}
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
    let top: Vec<u64> = task.win32k().windows.iter().filter(|(_, w)| w.parent == DESKTOP).map(|(&h, _)| h).collect();
    enumerate(task, cx, top, a[0], a[1])
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
fn activate_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if !task.window_exists(a[0]) {
        return Reply::Value(FALSE);
    }
    match task.activate(cx, a[0]) {
        Ok(_) => Reply::Value(TRUE),
        Err(reply) => reply,
    }
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
                if flags & 0x800 != 0 {
                    inputs.push(Input::Wheel(data / 120));
                }
            }
            1 => {
                let (vk, scan, flags) = (u16_at(8), u16_at(10), u32_at(12));
                let up = flags & 2 != 0;
                if flags & 4 != 0 {
                    // KEYEVENTF_UNICODE: the key that types it, on the desktop's US layout, with shift if it needs it.
                    if let Some((code, shift)) = char::from_u32(u32::from(scan)).and_then(display::keys::for_char).filter(|_| !up) {
                        if shift {
                            inputs.push(Input::Key(display::keys::LEFT_SHIFT, true));
                        }
                        inputs.extend([Input::Key(code, true), Input::Key(code, false)]);
                        if shift {
                            inputs.push(Input::Key(display::keys::LEFT_SHIFT, false));
                        }
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
