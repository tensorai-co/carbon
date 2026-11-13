//! user32.dll: windows, messages and painting, over `win32k`. Wide (W) functions only, for now, but the message box.

use display::Rect;
use display::desktop::{self as dt, Button, Input, Insets};
use display::selection::{Selection, TEXT};
use kernels_core::thread::Cx;
use kernels_core::user;

use super::resapi::{load_string_a, load_string_w};
use super::{Reply, dll};
use crate::process::Task;
use crate::win32k::{self, Class, DESKTOP, Filter, Msg, WM_GETTEXT, WM_GETTEXTLENGTH, WM_SETTEXT, dialog, gdi, keys};

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
    host GetWindowLongW(2) => get_window_long_w;
    host SetWindowLongW(3) => set_window_long_w;
    host GetClassLongPtrW(2) => get_class_long_ptr_w;
    host GetWindowLongPtrA(2) => get_window_long_ptr_w;
    host SetWindowLongPtrA(3) => set_window_long_ptr_a;
    host GetWindowLongA(2) => get_window_long_w;
    host SetWindowLongA(3) => set_window_long_a;
    host GetClassLongPtrA(2) => get_class_long_ptr_w;
    host GetClassLongW(2) => get_class_long_ptr_w;
    host GetClassLongA(2) => get_class_long_ptr_w;
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
    host FindWindowA(2) => find_window_a;
    host FindWindowExW(4) => find_window_ex_w;
    host FindWindowExA(4) => find_window_ex_a;
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
    host DrawTextA(5) => draw_text_a;
    host GetSysColor(1) => get_sys_color;
    host GetSysColorBrush(1) => get_sys_color_brush;
    host GetSystemMetrics(1) => get_system_metrics;
    host LoadCursorW(2) => load_resource;
    host LoadIconW(2) => load_resource;
    host LoadCursorA(2) => load_resource;
    host LoadIconA(2) => load_resource;
    host LoadBitmapW(2) => load_resource;
    host LoadImageW(6) => load_resource;
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
    host RegisterClipboardFormatA(1) => register_clipboard_format_a;
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
    host CreateWindowExA(12) => create_window_ex_a;
    host RegisterClassExA(1) => register_class_ex_a;
    host RegisterClassA(1) => register_class_a;
    host UnregisterClassA(2) => unregister_class_a;
    host DefWindowProcA(4) => def_window_proc_a;
    host CallWindowProcA(5) => call_window_proc_w;
    host GetMessageA(4) => get_message_a;
    host PeekMessageA(5) => peek_message_a;
    host DispatchMessageA(1) => dispatch_message_a;
    host SendMessageA(4) => send_message_a;
    host PostMessageA(4) => post_message_a;
    host PostThreadMessageA(4) => post_thread_message_a;
    host SetWindowTextA(2) => set_window_text_a;
    host GetWindowTextA(3) => get_window_text_a;
    host GetWindowTextLengthA(1) => get_window_text_length_a;
    host IsWindowUnicode(1) => is_window_unicode;
    // Dialogs, and the standard controls' procedures, which atlas's user32 exports so a program can call them.
    host DefDlgProcW(4) => def_dlg_proc;
    host DefDlgProcA(4) => def_dlg_proc;
    host AtlasButtonWndProc(4) => button_wnd_proc;
    host AtlasStaticWndProc(4) => static_wnd_proc;
    host AtlasEditWndProc(4) => edit_wnd_proc;
    host CreateDialogParamW(5) => create_dialog_param_w;
    host CreateDialogParamA(5) => create_dialog_param_a;
    host CreateDialogIndirectParamW(5) => create_dialog_indirect_param;
    host CreateDialogIndirectParamA(5) => create_dialog_indirect_param;
    host DialogBoxParamW(5) => dialog_box_param_w;
    host DialogBoxParamA(5) => dialog_box_param_a;
    host DialogBoxIndirectParamW(5) => dialog_box_indirect_param;
    host DialogBoxIndirectParamA(5) => dialog_box_indirect_param;
    host EndDialog(2) => end_dialog;
    host IsDialogMessageW(2) => is_dialog_message;
    host IsDialogMessageA(2) => is_dialog_message;
    host GetDlgItem(2) => get_dlg_item;
    host GetDlgCtrlID(1) => get_dlg_ctrl_id;
    host SetDlgItemTextW(3) => set_dlg_item_text_w;
    host SetDlgItemTextA(3) => set_dlg_item_text_a;
    host GetDlgItemTextW(4) => get_dlg_item_text_w;
    host GetDlgItemTextA(4) => get_dlg_item_text_a;
    host SetDlgItemInt(4) => set_dlg_item_int;
    host GetDlgItemInt(4) => get_dlg_item_int;
    host SendDlgItemMessageW(5) => send_dlg_item_message_w;
    host SendDlgItemMessageA(5) => send_dlg_item_message_a;
    host CheckDlgButton(3) => check_dlg_button;
    host IsDlgButtonChecked(2) => is_dlg_button_checked;
    host CheckRadioButton(4) => check_radio_button;
    host GetNextDlgTabItem(3) => get_next_dlg_tab_item;
    host GetNextDlgGroupItem(3) => get_next_dlg_group_item;
    host MapDialogRect(2) => map_dialog_rect;
    host GetDialogBaseUnits(0) => get_dialog_base_units;
    host EnableWindow(2) => enable_window;
    host IsWindowEnabled(1) => is_window_enabled;
    host GetClassInfoW(3) => get_class_info_w;
    host GetClassInfoA(3) => get_class_info_a;
    host GetClassInfoExW(3) => get_class_info_ex_w;
    host GetClassInfoExA(3) => get_class_info_ex_a;
    host SetClassLongW(3) => set_class_long;
    host SetClassLongA(3) => set_class_long;
    host SetClassLongPtrW(3) => set_class_long;
    host SetClassLongPtrA(3) => set_class_long;
    host GetSystemMenu(2) => get_system_menu;
    host DestroyIcon(1) => destroy_icon;
    host EnableMenuItem(3) => enable_menu_item;
    // Not written yet: what installers import.
    missing AppendMenuW;
    guest CharNextA => AtlasCharNextA;
    guest CharNextW => AtlasCharNextW;
    guest CharPrevW => AtlasCharPrevW;
    guest CharPrevA => AtlasCharPrevA;
    missing CreatePopupMenu;
    missing ExitWindowsEx;
    missing GetMessagePos;
    missing MessageBoxIndirectW;
    missing SendMessageTimeoutW;
    missing SystemParametersInfoW;
    missing TrackPopupMenu;
    guest wsprintfA => AtlasWsprintfA;
    guest wsprintfW => AtlasWsprintfW;
    guest wvsprintfA => AtlasWvsprintfA;
    guest wvsprintfW => AtlasWvsprintfW;
}

const TRUE: u64 = 1;
const FALSE: u64 = 0;

fn value(result: Result<u64, Reply>) -> Reply {
    match result {
        Ok(v) => Reply::Value(v),
        Err(reply) => reply,
    }
}



/// `LoadCursorW`, `LoadIconW`, `LoadBitmapW` and `LoadImageW(instance, name, type, cx, cy, flags)`: something that
/// isn't null, until there are resources.
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
    let read = |task: &mut Task, cx: &mut Cx<'_>, at: u64| task.mem(cx, false, |t| user::read_cstr(&t.space(), at, 4096)).map(|b| crate::ansi::text(&b)).unwrap_or_default();
    let text = read(task, cx, a[1]);
    let title = if a[2] == 0 { "Error".to_string() } else { read(task, cx, a[2]) };
    task.k.host.write(host::Stream::Err, format!("[message box] {title}: {text}\n").as_bytes());
    Reply::Value(1)
}

/// A WNDCLASS(EX)W (or, with `ansi`, A), from where its fields start (after `cbSize` and `style` for the EX form).
fn register(task: &mut Task, cx: &mut Cx<'_>, at: u64, ex: bool, ansi: bool) -> Reply {
    // WNDCLASSEXW: cbSize, style, lpfnWndProc, cbClsExtra, cbWndExtra, hInstance, hIcon, hCursor, hbrBackground,
    // lpszMenuName, lpszClassName, hIconSm. WNDCLASSW is the same from `style`, with no cbSize and no hIconSm. Its
    // pointers and handles are a pointer wide: 80 and 72 bytes, or 48 and 40 for a 32-bit program.
    let abi = task.abi();
    let w = abi.width as usize;
    let start = if ex { 8 } else { w };
    let size = start + 7 * w + 8 + if ex { w } else { 0 };
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, size)) else { return Reply::Value(0) };
    let get = |o: usize| abi.ptr_at(&b, o);
    let get32 = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
    let style = get32(if ex { 4 } else { 0 });
    let (wndproc, extra, instance) = (get(start), get32(start + w + 4), get(start + w + 8));
    let handles = start + w + 8 + w;
    let (cursor, background, name_at) = (get(handles + w), get(handles + 2 * w), get(handles + 4 * w));
    let name = if name_at < 0x1_0000 { format!("#{name_at}") } else { task.text_arg(cx, name_at, !ansi) };
    match task.register_class(Class { name, atom: 0, wndproc, style, extra: extra as usize, background, instance, cursor, ansi }) {
        Ok(atom) => Reply::Value(u64::from(atom)),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0)
        }
    }
}

fn register_class_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], true, false)
}

fn register_class_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], false, false)
}

fn register_class_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], true, true)
}

fn register_class_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register(task, cx, a[0], false, true)
}

fn unregister_class_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.class_named(cx, a[0]).unwrap_or_default();
    Reply::Value(u64::from(task.unregister_class(&name)))
}

fn unregister_class_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.class_named_as(cx, a[0], false).unwrap_or_default();
    Reply::Value(u64::from(task.unregister_class(&name)))
}

fn create_window_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.create_window(cx, a, false))
}

fn create_window_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.create_window(cx, a, true))
}

/// `IsWindowUnicode`: whether its `WndProc` takes UTF-16.
fn is_window_unicode(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().windows.get(&a[0]).is_some_and(|w| !w.ansi)))
}

/// The messages whose `wParam` is a character.
fn is_char(message: u32) -> bool {
    matches!(message, win32k::WM_CHAR | win32k::WM_SYSCHAR | win32k::WM_DEADCHAR | win32k::WM_SYSDEADCHAR)
}

/// A message in ANSI, as an `*A` function was given it, made UTF-16 as win32k has every message: a character's
/// `wParam`, or WM_SETTEXT's text (copied to scratch), then `send`. For WM_GETTEXT, the UTF-16 it gets back is
/// converted into the caller's buffer, and the count is of its bytes.
fn from_ansi(task: &mut Task, cx: &mut Cx<'_>, message: u32, wparam: u64, lparam: u64, send: impl FnOnce(&mut Task, &mut Cx<'_>, u64, u64) -> Result<u64, Reply>) -> Result<u64, Reply> {
    match message {
        m if is_char(m) => send(task, cx, u64::from(atlas_abi::ansi::from_1252(wparam as u8)), lparam),
        WM_SETTEXT if lparam != 0 => {
            let text = task.text_arg(cx, lparam, false);
            let wide: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            task.with_scratch(cx, wide.len(), |_| wide.clone(), |task, cx, at| send(task, cx, wparam, at))
        }
        WM_GETTEXT if lparam != 0 && wparam != 0 => {
            let room = (wparam as u32 as usize).min(1 << 16);
            task.with_scratch(cx, room * 2, |_| vec![0; room * 2], |task, cx, at| {
                send(task, cx, room as u64, at)?;
                let text = task.wstr(cx, at);
                let ansi: Vec<u8> = crate::ansi::bytes(&text).into_iter().take(room - 1).collect();
                let bytes: Vec<u8> = ansi.iter().copied().chain([0]).collect();
                let _ = task.mem(cx, true, |t| user::write(&mut t.space(), lparam, &bytes));
                Ok(ansi.len() as u64)
            })
        }
        _ => send(task, cx, wparam, lparam),
    }
}

fn send_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (hwnd, message) = (a[0], a[1] as u32);
    value(from_ansi(task, cx, message, a[2], a[3], |task, cx, w, l| task.send_message(cx, hwnd, message, w, l)))
}

fn set_window_text_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_message_a(task, cx, &[a[0], u64::from(WM_SETTEXT), 0, a[1]])
}

fn get_window_text_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    send_message_a(task, cx, &[a[0], u64::from(WM_GETTEXT), a[2] & 0xffff_ffff, a[1]])
}

/// `GetWindowTextLengthA`: in bytes of the ANSI code page, which for 1252 is characters.
fn get_window_text_length_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.send_message(cx, a[0], WM_GETTEXTLENGTH, 0, 0))
}

/// `DefWindowProcA`: as `DefWindowProcW`, with the message's text in ANSI.
fn def_window_proc_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (hwnd, message) = (a[0], a[1] as u32);
    if message == WM_GETTEXTLENGTH {
        let text = task.win32k().windows.get(&hwnd).map(|w| w.text.clone()).unwrap_or_default();
        return Reply::Value(crate::ansi::bytes(&text).len() as u64);
    }
    value(from_ansi(task, cx, message, a[2], a[3], |task, cx, w, l| task.def_window_proc(cx, hwnd, message, w, l)))
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
    get_message(task, cx, a, false)
}

fn get_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_message(task, cx, a, true)
}

/// A message as an `*A` function gives it: a character in ANSI.
fn as_ansi(mut m: Msg) -> Msg {
    if is_char(m.message) {
        m.wparam = u64::from(atlas_abi::ansi::to_1252(m.wparam as u16).unwrap_or(b'?'));
    }
    m
}

fn get_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], ansi: bool) -> Reply {
    match task.next_message(cx, filter(a), true, true) {
        Ok(Some(m)) => {
            let width = task.abi().width;
            let bytes = if ansi { as_ansi(m).encode(width) } else { m.encode(width) };
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
    peek_message(task, cx, a, false)
}

fn peek_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    peek_message(task, cx, a, true)
}

fn peek_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], ansi: bool) -> Reply {
    const PM_REMOVE: u64 = 1;
    match task.next_message(cx, filter(a), a[4] & PM_REMOVE != 0, false) {
        Ok(Some(m)) => {
            let width = task.abi().width;
            let bytes = if ansi { as_ansi(m).encode(width) } else { m.encode(width) };
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &bytes));
            Reply::Value(TRUE)
        }
        Ok(None) => Reply::Value(FALSE),
        Err(reply) => reply,
    }
}

/// `TranslateMessage`: for a key that types something, WM_CHAR (or WM_SYSCHAR), to be taken next. TRUE for any
/// key-down message.
fn translate_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], Msg::size(width))) else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(task.translate(&Msg::decode(&b, width))))
}

fn dispatch_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], Msg::size(width))) else { return Reply::Value(0) };
    value(task.dispatch(cx, Msg::decode(&b, width)))
}

/// `DispatchMessageA`: the message it's given in ANSI, as `GetMessageA` gave it.
fn dispatch_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], Msg::size(width))) else { return Reply::Value(0) };
    let mut m = Msg::decode(&b, width);
    if is_char(m.message) {
        m.wparam = u64::from(atlas_abi::ansi::from_1252(m.wparam as u8));
    }
    value(task.dispatch(cx, m))
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

/// `PostMessageA`: a character posted is made UTF-16, as win32k keeps messages.
fn post_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let wparam = if is_char(a[1] as u32) { u64::from(atlas_abi::ansi::from_1252(a[2] as u8)) } else { a[2] };
    post_message_w(task, cx, &[a[0], a[1], wparam, a[3]])
}

fn post_thread_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let wparam = if is_char(a[1] as u32) { u64::from(atlas_abi::ansi::from_1252(a[2] as u8)) } else { a[2] };
    post_thread_message_w(task, cx, &[a[0], a[1], wparam, a[3]])
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
    let width = task.abi().width as usize;
    get_window_long(task, cx, a, width)
}

fn get_window_long_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_window_long(task, cx, a, 4)
}

/// `GetWindowLong(Ptr)`: what it names, its extra bytes `size` at a time (a LONG, or a pointer for the Ptr forms).
fn get_window_long(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], size: usize) -> Reply {
    let w = task.win32k();
    let Some(win) = w.windows.get(&a[0]) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let index = a[1] as i32;
    let v = match index {
        GWLP_WNDPROC => win.wndproc,
        GWLP_HINSTANCE => win.instance,
        GWLP_HWNDPARENT => if win.parent == DESKTOP { win.owner } else { win.parent },
        GWLP_ID => win.id,
        GWL_STYLE => u64::from(win.style | if win.visible { 0x1000_0000 } else { 0 }),
        GWL_EXSTYLE => u64::from(win.ex_style),
        GWLP_USERDATA => win.userdata,
        i if i >= 0 && (i as usize) + size <= win.extra.len() => {
            let at = i as usize;
            let mut b = [0u8; 8];
            b[..size].copy_from_slice(&win.extra[at..at + size]);
            u64::from_le_bytes(b)
        }
        _ => {
            drop(w);
            task.set_last_error(cx, 1413); // ERROR_INVALID_INDEX
            return Reply::Value(0);
        }
    };
    drop(w);
    Reply::Value(if index == GWLP_WNDPROC { task.wndproc_address(v) } else { v })
}

fn set_window_long_ptr_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width as usize;
    set_window_long(task, cx, a, width)
}

fn set_window_long_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_window_long(task, cx, a, 4)
}

/// `SetWindowLong(Ptr)`: what it had before.
fn set_window_long(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], size: usize) -> Reply {
    let mut w = task.win32k();
    let Some(win) = w.windows.get_mut(&a[0]) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let new = if size == 4 { a[2] & 0xffff_ffff } else { a[2] };
    let index = a[1] as i32;
    let old = match index {
        GWLP_WNDPROC => {
            // A `WndProc` set by the W function takes UTF-16 (the A one says otherwise after).
            win.ansi = false;
            std::mem::replace(&mut win.wndproc, new)
        }
        GWLP_HINSTANCE => std::mem::replace(&mut win.instance, new),
        GWLP_ID => std::mem::replace(&mut win.id, new),
        GWL_STYLE => u64::from(std::mem::replace(&mut win.style, new as u32 & !0x1000_0000)),
        GWL_EXSTYLE => u64::from(std::mem::replace(&mut win.ex_style, new as u32)),
        GWLP_USERDATA => std::mem::replace(&mut win.userdata, new),
        i if i >= 0 && (i as usize) + size <= win.extra.len() => {
            let at = i as usize;
            let mut b = [0u8; 8];
            b[..size].copy_from_slice(&win.extra[at..at + size]);
            win.extra[at..at + size].copy_from_slice(&new.to_le_bytes()[..size]);
            u64::from_le_bytes(b)
        }
        _ => {
            drop(w);
            task.set_last_error(cx, 1413); // ERROR_INVALID_INDEX
            return Reply::Value(0);
        }
    };
    drop(w);
    Reply::Value(if index == GWLP_WNDPROC { task.wndproc_address(old) } else { old })
}

/// `SetWindowLongPtrA`: as the W, and a `WndProc` set this way takes ANSI.
fn set_window_long_ptr_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width as usize;
    set_window_long_ansi(task, cx, a, width)
}

fn set_window_long_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_window_long_ansi(task, cx, a, 4)
}

fn set_window_long_ansi(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], size: usize) -> Reply {
    let reply = set_window_long(task, cx, a, size);
    if a[1] as i32 == GWLP_WNDPROC
        && let Some(w) = task.win32k().windows.get_mut(&a[0])
    {
        w.ansi = true;
    }
    reply
}

fn get_class_long_ptr_w(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width;
    let v = {
        let w = task.win32k();
        let Some(class) = w.windows.get(&a[0]).and_then(|win| w.class_by_name(&win.class)) else { return Reply::Value(0) };
        match a[1] as i32 {
            -10 => class.background,         // GCLP_HBRBACKGROUND
            -12 => class.cursor,             // GCLP_HCURSOR
            -16 => class.instance,           // GCLP_HMODULE
            -24 => class.wndproc,            // GCLP_WNDPROC
            -26 => u64::from(class.style),   // GCL_STYLE
            -32 => u64::from(class.atom),    // GCW_ATOM
            -18 => class_extra(class, width) as u64, // GCL_CBWNDEXTRA
            _ => 0,
        }
    };
    Reply::Value(if a[1] as i32 == -24 { task.wndproc_address(v) } else { v })
}

/// A class's extra bytes for its windows, as a program of this width is told: a dialog's are `DLGWINDOWEXTRA`.
fn class_extra(class: &Class, width: u64) -> usize {
    if win32k::SysProc::of(class.wndproc) == Some(win32k::SysProc::Dialog) && width == 4 { 30 } else { class.extra }
}

/// `SetClassLong(Ptr)`: a class's background, cursor, style or procedure changed, for every window of it. Its icons
/// aren't drawn, so they're kept nowhere. What it had before.
fn set_class_long(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(name) = w.windows.get(&a[0]).map(|win| win.class.clone()) else {
        drop(w);
        task.set_last_error(cx, win32k::ERROR_INVALID_WINDOW_HANDLE);
        return Reply::Value(0);
    };
    let Some(class) = w.class_mut(&name) else { return Reply::Value(0) };
    let new = a[2];
    let old = match a[1] as i32 {
        -10 => std::mem::replace(&mut class.background, new),
        -12 => std::mem::replace(&mut class.cursor, new),
        -16 => std::mem::replace(&mut class.instance, new),
        -24 => std::mem::replace(&mut class.wndproc, new),
        -26 => u64::from(std::mem::replace(&mut class.style, new as u32)),
        // GCLP_HICON, GCLP_HICONSM.
        -14 | -34 => 0,
        _ => {
            drop(w);
            task.set_last_error(cx, 1413); // ERROR_INVALID_INDEX
            return Reply::Value(0);
        }
    };
    drop(w);
    Reply::Value(if a[1] as i32 == -24 { task.wndproc_address(old) } else { old })
}

/// `GetClassInfo(Ex)W` and `A`: a class's WNDCLASS(EX), its procedure as an address the program can call. The name
/// it's given back is the one it asked by. The class's atom, or 0 with ERROR_CLASS_DOES_NOT_EXIST.
fn class_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], ex: bool, ansi: bool) -> Reply {
    let name = task.class_named_as(cx, a[1], !ansi).unwrap_or_default();
    let width = task.abi().width;
    let Some(class) = task.win32k().class_by_name(&name).cloned() else {
        task.set_last_error(cx, 1411);
        return Reply::Value(0);
    };
    let abi = task.abi();
    let w = width as usize;
    let start = if ex { 8 } else { w };
    let size = start + 7 * w + 8 + if ex { w } else { 0 };
    let mut b = vec![0u8; size];
    let put = |b: &mut Vec<u8>, at: usize, v: u64| b[at..at + w].copy_from_slice(&abi.ptr(v));
    let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    if ex {
        put32(&mut b, 0, size as u32);
    }
    put32(&mut b, if ex { 4 } else { 0 }, class.style);
    put(&mut b, start, task.wndproc_address(class.wndproc));
    put32(&mut b, start + w + 4, class_extra(&class, width) as u32);
    let instance = if class.instance == 0 { a[0] } else { class.instance };
    put(&mut b, start + w + 8, instance);
    let handles = start + w + 8 + w;
    put(&mut b, handles + w, class.cursor);
    put(&mut b, handles + 2 * w, class.background);
    put(&mut b, handles + 4 * w, a[1]);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() {
        return Reply::Value(0);
    }
    Reply::Value(u64::from(class.atom))
}

fn get_class_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    class_info(task, cx, a, false, false)
}

fn get_class_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    class_info(task, cx, a, false, true)
}

fn get_class_info_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    class_info(task, cx, a, true, false)
}

fn get_class_info_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    class_info(task, cx, a, true, true)
}

/// `DestroyIcon`: icons aren't objects yet (`LoadIcon`'s and `SHGetFileInfo`'s are placeholders), so there's nothing
/// to free.
fn destroy_icon(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(a[0] != 0))
}

// ---- System menus ----

/// `GetSystemMenu(window, revert)`: its system menu's handle, or, with `revert`, its commands enabled again and NULL.
/// There's no menu to show yet, only the commands' states, which `DefWindowProc` heeds.
fn get_system_menu(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut w = task.win32k();
    let Some(top) = w.top_level_of(a[0]) else { return Reply::Value(0) };
    if a[1] as u32 != 0 {
        w.system_menus.remove(&top);
        return Reply::Value(0);
    }
    w.system_menus.entry(top).or_insert_with(|| win32k::SYSTEM_COMMANDS.iter().map(|&c| (c, 0)).collect());
    Reply::Value(win32k::SYSTEM_MENUS + top)
}

/// `EnableMenuItem(menu, item, flags)`: a system menu's command enabled, greyed or disabled (by command, or by
/// position with MF_BYPOSITION); the state it had, or -1 if there's no such item. Other menus aren't there yet.
fn enable_menu_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    const MF_BYPOSITION: u32 = 0x400;
    const NONE: u64 = 0xffff_ffff;
    let flags = a[2] as u32;
    let mut w = task.win32k();
    let Some(menu) = a[0].checked_sub(win32k::SYSTEM_MENUS).and_then(|top| w.system_menus.get_mut(&top)) else { return Reply::Value(NONE) };
    let command = if flags & MF_BYPOSITION != 0 { win32k::SYSTEM_COMMANDS.get(a[1] as usize).copied() } else { Some(a[1] & 0xffff) };
    match command.and_then(|c| menu.get_mut(&c)) {
        Some(state) => Reply::Value(u64::from(std::mem::replace(state, flags & 3))),
        None => Reply::Value(NONE),
    }
}

// ---- Dialogs ----

fn def_dlg_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.def_dlg_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn button_wnd_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.button_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn static_wnd_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.static_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

fn edit_wnd_proc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    value(task.edit_proc(cx, a[0], a[1] as u32, a[2], a[3]))
}

/// A template from a module's resources, by name or number: its address and contents.
fn resource_template(task: &mut Task, cx: &mut Cx<'_>, instance: u64, name: u64, wide: bool) -> Option<(u64, dialog::Template)> {
    let (at, bytes) = task.dialog_resource(cx, instance, name, wide)?;
    Some((at, dialog::parse(&bytes)?))
}

fn memory_template(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<dialog::Template> {
    let bytes = task.dialog_in_memory(cx, at);
    dialog::parse(&bytes)
}

/// `CreateDialogParamW(instance, template, owner, procedure, parameter)`, and `A`.
fn create_dialog_param(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some((at, t)) = resource_template(task, cx, a[0], a[1], wide) else {
        task.set_last_error(cx, 1814); // ERROR_RESOURCE_NAME_NOT_FOUND
        return Reply::Value(0);
    };
    value(task.create_dialog(cx, &t, at, a[0], a[2], a[3], a[4], false))
}

fn create_dialog_param_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_dialog_param(task, cx, a, true)
}

fn create_dialog_param_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_dialog_param(task, cx, a, false)
}

/// `CreateDialogIndirectParamW(instance, template, owner, procedure, parameter)`: from a template in memory.
fn create_dialog_indirect_param(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(t) = memory_template(task, cx, a[1]) else {
        task.set_last_error(cx, 87);
        return Reply::Value(0);
    };
    value(task.create_dialog(cx, &t, a[1], a[0], a[2], a[3], a[4], false))
}

fn dialog_box_param(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some((at, t)) = resource_template(task, cx, a[0], a[1], wide) else {
        task.set_last_error(cx, 1814);
        return Reply::Value(u64::MAX);
    };
    value(task.dialog_box(cx, &t, at, a[0], a[2], a[3], a[4]))
}

/// `DialogBoxParamW(instance, template, owner, procedure, parameter)`: modal, until `EndDialog`.
fn dialog_box_param_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    dialog_box_param(task, cx, a, true)
}

fn dialog_box_param_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    dialog_box_param(task, cx, a, false)
}

fn dialog_box_indirect_param(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(t) = memory_template(task, cx, a[1]) else {
        task.set_last_error(cx, 87);
        return Reply::Value(u64::MAX);
    };
    value(task.dialog_box(cx, &t, a[1], a[0], a[2], a[3], a[4]))
}

fn end_dialog(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.end_dialog(cx, a[0], a[1]) {
        Ok(done) => Reply::Value(u64::from(done)),
        Err(reply) => reply,
    }
}

/// `IsDialogMessageW(dialog, &msg)`.
fn is_dialog_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let width = task.abi().width;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], Msg::size(width))) else { return Reply::Value(FALSE) };
    match task.is_dialog_message(cx, a[0], &Msg::decode(&b, width)) {
        Ok(done) => Reply::Value(u64::from(done)),
        Err(reply) => reply,
    }
}

fn get_dlg_item(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        task.set_last_error(cx, 1421); // ERROR_CONTROL_ID_NOT_FOUND
    }
    Reply::Value(item)
}

fn get_dlg_ctrl_id(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.win32k().windows.get(&a[0]).filter(|w| !w.top_level()).map_or(0, |w| w.id))
}

fn set_dlg_item_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(FALSE);
    }
    value(task.send_message(cx, item, WM_SETTEXT, 0, a[2]))
}

fn set_dlg_item_text_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(FALSE);
    }
    send_message_a(task, cx, &[item, u64::from(WM_SETTEXT), 0, a[2]])
}

/// `GetDlgItemTextW(dialog, id, buffer, size)`: its text's length; an empty string for no such control.
fn get_dlg_item_text_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        if a[3] as u32 > 0 {
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &[0, 0]));
        }
        return Reply::Value(0);
    }
    value(task.send_message(cx, item, WM_GETTEXT, a[3] & 0xffff_ffff, a[2]))
}

fn get_dlg_item_text_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        if a[3] as u32 > 0 {
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &[0]));
        }
        return Reply::Value(0);
    }
    send_message_a(task, cx, &[item, u64::from(WM_GETTEXT), a[3] & 0xffff_ffff, a[2]])
}

/// `SetDlgItemInt(dialog, id, value, signed)`.
fn set_dlg_item_int(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(FALSE);
    }
    let text = if a[3] != 0 { (a[2] as u32 as i32).to_string() } else { (a[2] as u32).to_string() };
    let wide: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    value(task.with_scratch(cx, wide.len(), |_| wide.clone(), |task, cx, at| task.send_message(cx, item, WM_SETTEXT, 0, at)))
}

/// `GetDlgItemInt(dialog, id, &translated, signed)`: its text as a number, if it's one.
fn get_dlg_item_int(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    let text = task.win32k().windows.get(&item).map(|w| w.text.trim().to_string()).unwrap_or_default();
    let parsed: Option<u64> = if a[3] != 0 { text.parse::<i32>().ok().map(|v| v as u32 as u64) } else { text.parse::<u32>().ok().map(u64::from) };
    if a[2] != 0 {
        let ok = u32::from(parsed.is_some());
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &ok.to_le_bytes()));
    }
    Reply::Value(parsed.unwrap_or(0))
}

fn send_dlg_item_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(0);
    }
    value(task.send_message(cx, item, a[2] as u32, a[3], a[4]))
}

fn send_dlg_item_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(0);
    }
    send_message_a(task, cx, &[item, a[2], a[3], a[4]])
}

/// `CheckDlgButton(dialog, id, check)`: BM_SETCHECK to it.
fn check_dlg_button(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(FALSE);
    }
    match task.send_message(cx, item, win32k::controls::BM_SETCHECK, a[2] & 0xffff_ffff, 0) {
        Ok(_) => Reply::Value(TRUE),
        Err(reply) => reply,
    }
}

fn is_dlg_button_checked(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let item = task.dialog_item(a[0], a[1]);
    if item == 0 {
        return Reply::Value(0);
    }
    value(task.send_message(cx, item, win32k::controls::BM_GETCHECK, 0, 0))
}

/// `CheckRadioButton(dialog, first, last, check)`: `check` checked, and the others from `first` to `last` not.
fn check_radio_button(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (first, last, check) = (a[1] as u32, a[2] as u32, a[3] as u32);
    for id in first..=last.min(first.saturating_add(4096)) {
        let item = task.dialog_item(a[0], u64::from(id));
        if item != 0
            && let Err(reply) = task.send_message(cx, item, win32k::controls::BM_SETCHECK, u64::from(id == check), 0)
        {
            return reply;
        }
    }
    Reply::Value(TRUE)
}

fn get_next_dlg_tab_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.next_tab_item(a[0], a[1], a[2] != 0))
}

fn get_next_dlg_group_item(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(task.next_group_item(a[0], a[1], a[2] != 0))
}

/// `MapDialogRect(dialog, &rect)`: dialog units to pixels.
fn map_dialog_rect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(r) = task.read_rect(cx, a[1]) else { return Reply::Value(FALSE) };
    Reply::Value(u64::from(task.write_rect(cx, a[1], &dialog::map_dialog_rect(&r))))
}

/// `GetDialogBaseUnits`: the one font's cell, a dialog's base units, its width low and its height high.
fn get_dialog_base_units(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value((dialog::BASE_Y as u64) << 16 | dialog::BASE_X as u64)
}

/// `EnableWindow(window, enable)`: whether it was disabled before.
fn enable_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.enable_window(cx, a[0], a[1] as u32 != 0) {
        Ok(was) => Reply::Value(u64::from(was)),
        Err(reply) => reply,
    }
}

fn is_window_enabled(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(task.win32k().windows.get(&a[0]).is_some_and(|w| w.style & win32k::WS_DISABLED == 0)))
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
    find_window(task, cx, a, true)
}

fn find_window_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_window(task, cx, a, false)
}

fn find_window(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let class = if a[0] == 0 { None } else { task.class_named_as(cx, a[0], wide) };
    let title = (a[1] != 0).then(|| task.text_arg(cx, a[1], wide));
    let w = task.win32k();
    let found = w.windows.iter().find(|(_, win)| {
        win.parent == DESKTOP && class.as_ref().is_none_or(|c| *c == win.class) && title.as_ref().is_none_or(|t| *t == win.text)
    });
    Reply::Value(found.map_or(0, |(&h, _)| h))
}

/// `FindWindowExW(parent, after, class, title)`: the first of `parent`'s children (the top-level windows' for none)
/// after `after`, in their stacking order, with that class and title, either of which may be left out.
fn find_window_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    // HWND_MESSAGE: message-only windows, which there are none of.
    if a[0] as u32 == (-3i32) as u32 {
        return Reply::Value(0);
    }
    let class = if a[2] == 0 { None } else { task.class_named_as(cx, a[2], wide) };
    let title = (a[3] != 0).then(|| task.text_arg(cx, a[3], wide));
    let w = task.win32k();
    let parent = if a[0] == 0 { DESKTOP } else { a[0] };
    let order = w.z_order(parent);
    let start = if a[1] == 0 { 0 } else { order.iter().position(|&h| h == a[1]).map_or(order.len(), |i| i + 1) };
    let found = order[start..].iter().find(|h| {
        w.windows.get(h).is_some_and(|win| class.as_ref().is_none_or(|c| *c == win.class) && title.as_ref().is_none_or(|t| *t == win.text))
    });
    Reply::Value(found.copied().unwrap_or(0))
}

fn find_window_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_window_ex(task, cx, a, true)
}

fn find_window_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_window_ex(task, cx, a, false)
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
    draw_text(task, cx, a, true)
}

fn draw_text_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    draw_text(task, cx, a, false)
}

fn draw_text(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let len = a[2] as i32;
    let text = if len < 0 {
        task.text_arg(cx, a[1], wide)
    } else if wide {
        let raw = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], len as usize * 2)).unwrap_or_default();
        String::from_utf16_lossy(&raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    } else {
        crate::ansi::text(&task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], len as usize)).unwrap_or_default())
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
    // FLASHWINFO: cbSize, then hwnd at a pointer's alignment, then its flags.
    let abi = task.abi();
    let w = abi.width as usize;
    let Ok(b) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[0], 2 * w + 12)) else { return Reply::Value(FALSE) };
    let hwnd = abi.ptr_at(&b, w);
    let flags = u32::from_le_bytes(b[2 * w..2 * w + 4].try_into().unwrap_or_default());
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

/// `SendInput`: mouse and keyboard input, onto the desktop as if from the user. INPUT is 40 bytes (28 for a 32-bit
/// program): its type, then at a pointer's alignment a MOUSEINPUT (dx, dy, mouseData, dwFlags) or KEYBDINPUT (wVk,
/// wScan, dwFlags), each ending in a pointer-sized dwExtraInfo.
fn send_input(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (count, at, size) = (a[0] as usize & 0xffff_ffff, a[1], a[2] as usize & 0xffff_ffff);
    let w = task.abi().width as usize;
    let input = if w == 8 { 40 } else { 28 };
    if size != input {
        task.set_last_error(cx, 87);
        return Reply::Value(0);
    }
    let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), at, count * input)) else { return Reply::Value(0) };
    // Each as the 64-bit layout has it, so what follows reads one: the union moved to 8.
    let b: Vec<u8> = raw.chunks(input).flat_map(|e| [&e[..4], &[0u8; 4][..], &e[w..]].concat().into_iter().chain(std::iter::repeat(0)).take(40)).collect();
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

/// Text from a format's bytes: UTF-16 for CF_UNICODETEXT, the ANSI code page for CF_TEXT and the OEM one for
/// CF_OEMTEXT, to the first NUL.
fn text_from(format: u32, bytes: &[u8]) -> Vec<u8> {
    let text: String = if format == CF_UNICODETEXT {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).take_while(|&u| u != 0).collect();
        String::from_utf16_lossy(&units)
    } else {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let cp = if format == CF_OEMTEXT { crate::ansi::OEMCP } else { crate::ansi::ACP };
        String::from_utf16_lossy(&crate::ansi::decode(cp, &bytes[..end], false).unwrap_or_default())
    };
    text.into_bytes()
}

/// A format's bytes from text, with its NUL: a character the code page hasn't is `?` in CF_TEXT and CF_OEMTEXT.
fn text_as(format: u32, utf8: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(utf8);
    if format == CF_UNICODETEXT {
        text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    } else {
        let cp = if format == CF_OEMTEXT { crate::ansi::OEMCP } else { crate::ansi::ACP };
        let units: Vec<u16> = text.encode_utf16().collect();
        crate::ansi::encode(cp, &units, b'?', false).map(|(b, _)| b).unwrap_or_default().into_iter().chain([0]).collect()
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
    register_clipboard_format(task, cx, a, true)
}

fn register_clipboard_format_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    register_clipboard_format(task, cx, a, false)
}

fn register_clipboard_format(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let name = task.text_arg(cx, a[0], wide);
    if name.is_empty() {
        return Reply::Value(0);
    }
    Reply::Value(u64::from(register_format(task, &name)))
}
