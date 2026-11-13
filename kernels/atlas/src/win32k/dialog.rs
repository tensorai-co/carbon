//! Dialogs: the dialog manager Windows keeps in user32, over win32k's windows. A dialog is made from a template
//! (`DLGTEMPLATE` or `DLGTEMPLATEEX`, from a resource or the program's memory): a window of the dialog's class
//! (`#32770` unless the template names another) with a control for each item, every size and place in dialog units
//! (a quarter of the font's average character's width, an eighth of its height; here the one font's 6 by 13 cell).
//! Its procedure (`DLGPROC`) and what it gives back (`DWLP_MSGRESULT`) are in the window's extra bytes where Windows
//! keeps them, so `GetWindowLongPtr` and `SetWindowLongPtr` find them; `DefDlgProc`, the class's window procedure,
//! calls it and does what it doesn't. `DialogBox` runs the dialog's own message loop with its owner disabled until
//! `EndDialog`; `IsDialogMessage` gives Tab, Enter and Escape their meaning in a dialog, modal or not.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::Rect;

use super::{
    DESKTOP, Msg, SW_SHOW, WM_ACTIVATE, WM_CLOSE, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORDLG, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_ERASEBKGND, WM_GETDLGCODE, WM_GETFONT, WM_INITDIALOG, WM_KEYDOWN,
    WM_PAINT, WM_QUIT, WM_SETFOCUS, WM_SETFONT, WS_CAPTION, WS_CHILD, WS_DISABLED, WS_EX_CONTROLPARENT, WS_GROUP,
    WS_POPUP, WS_TABSTOP, WS_VISIBLE, gdi, keys,
};
use crate::os::Reply;
use crate::process::Task;

/// Dialog base units: the one font's cell, which is a dialog font's average character at 96 DPI.
pub const BASE_X: i32 = display::font::CELL_WIDTH;
pub const BASE_Y: i32 = display::font::CELL_HEIGHT;

pub const DS_SETFONT: u32 = 0x40;
const DS_NOFAILCREATE: u32 = 0x10;
const DS_CONTROL: u32 = 0x400;
const DS_CENTER: u32 = 0x800;
const DS_CENTERMOUSE: u32 = 0x1000;

pub const IDOK: u64 = 1;
pub const IDCANCEL: u64 = 2;
const DM_GETDEFID: u32 = 0x400;
const DM_SETDEFID: u32 = 0x401;
const DM_REPOSITION: u32 = 0x402;
const DC_HASDEFID: u64 = 0x534b;
const WM_NEXTDLGCTL: u32 = 0x28;
const EM_SETSEL: u32 = 0xb1;

pub const DLGC_WANTARROWS: u64 = 0x1;
pub const DLGC_WANTTAB: u64 = 0x2;
pub const DLGC_WANTALLKEYS: u64 = 0x4;
pub const DLGC_HASSETSEL: u64 = 0x8;
pub const DLGC_DEFPUSHBUTTON: u64 = 0x10;
pub const DLGC_UNDEFPUSHBUTTON: u64 = 0x20;
pub const DLGC_RADIOBUTTON: u64 = 0x40;
pub const DLGC_WANTCHARS: u64 = 0x80;
pub const DLGC_STATIC: u64 = 0x100;
pub const DLGC_BUTTON: u64 = 0x2000;

/// A button's `BS_DEFPUSHBUTTON`, and its type's bits.
pub const BS_PUSHBUTTON: u32 = 0;
pub const BS_DEFPUSHBUTTON: u32 = 1;
pub const BS_TYPE: u32 = 0xf;

/// A string or a number, as templates and resources name things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Id {
    None,
    Num(u16),
    Name(String),
}

/// One control in a template.
#[derive(Debug, Clone)]
pub struct Item {
    pub style: u32,
    pub ex_style: u32,
    /// x, y, width and height, in dialog units.
    pub rect: [i32; 4],
    pub id: u32,
    pub class: Id,
    pub title: Id,
    /// Where its creation data is in the template (its size word, which `WM_CREATE`'s parameter points at), if it has
    /// any.
    pub data: Option<usize>,
}

/// A dialog template, as `DLGTEMPLATE` or `DLGTEMPLATEEX` has it.
#[derive(Debug, Clone)]
pub struct Template {
    pub style: u32,
    pub ex_style: u32,
    pub rect: [i32; 4],
    /// Its menu: none is made yet, as there are no menus.
    #[allow(dead_code)]
    pub menu: Id,
    pub class: Id,
    pub title: String,
    /// Its font: points and typeface. There's one font, so it only says the dialog has one.
    pub font: Option<(u16, String)>,
    pub items: Vec<Item>,
}

/// Reads a template, from where it starts (a DWORD boundary, as templates are).
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.b.get(self.at..self.at + 2)?.try_into().ok()?);
        self.at += 2;
        Some(v)
    }

    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(v)
    }

    fn i16(&mut self) -> Option<i32> {
        self.u16().map(|v| i32::from(v as i16))
    }

    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }

    /// A NUL-terminated UTF-16 string.
    fn string(&mut self) -> Option<String> {
        let mut units = Vec::new();
        loop {
            match self.u16()? {
                0 => return Some(String::from_utf16_lossy(&units)),
                u => units.push(u),
            }
        }
    }

    /// `sz_Or_Ord`: nothing (0), a number (0xFFFF then it), or a string.
    fn id(&mut self) -> Option<Id> {
        match u16::from_le_bytes(self.b.get(self.at..self.at + 2)?.try_into().ok()?) {
            0 => {
                self.at += 2;
                Some(Id::None)
            }
            0xffff => {
                self.at += 2;
                Some(Id::Num(self.u16()?))
            }
            _ => Some(Id::Name(self.string()?)),
        }
    }

    fn align(&mut self, to: usize) {
        self.at = self.at.next_multiple_of(to);
    }
}

/// A dialog template's contents: `DLGTEMPLATE`, or `DLGTEMPLATEEX` (version 1, signature 0xFFFF).
pub fn parse(b: &[u8]) -> Option<Template> {
    let mut r = Reader { b, at: 0 };
    let ex = b.get(0..4)? == [1, 0, 0xff, 0xff];
    let (style, ex_style) = if ex {
        r.at = 8; // dlgVer, signature, helpID
        let ex_style = r.u32()?;
        (r.u32()?, ex_style)
    } else {
        let style = r.u32()?;
        (style, r.u32()?)
    };
    let count = r.u16()?;
    let rect = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
    let menu = r.id()?;
    let class = r.id()?;
    let title = r.string()?;
    let font = if style & DS_SETFONT != 0 {
        let points = r.u16()?;
        if ex {
            r.u16()?; // weight
            r.u8()?; // italic
            r.u8()?; // charset
        }
        Some((points, r.string()?))
    } else {
        None
    };
    let mut items = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        r.align(4);
        let (style, ex_style) = if ex {
            r.u32()?; // helpID
            let ex_style = r.u32()?;
            (r.u32()?, ex_style)
        } else {
            let style = r.u32()?;
            (style, r.u32()?)
        };
        let rect = [r.i16()?, r.i16()?, r.i16()?, r.i16()?];
        let id = if ex { r.u32()? } else { u32::from(r.u16()?) };
        let class = r.id()?;
        let title = r.id()?;
        // The creation data: its size in bytes, then that many (as Windows reads both forms' counts).
        let at = r.at;
        let size = r.u16()?;
        let data = (size != 0).then_some(at);
        r.at += usize::from(size);
        if r.at > b.len() {
            return None;
        }
        items.push(Item { style, ex_style, rect, id, class, title, data });
    }
    Some(Template { style, ex_style, rect, menu, class, title, font, items })
}

/// Dialog units as pixels: a quarter of the base width each across, an eighth of its height down, rounded.
pub fn units_to_pixels(r: [i32; 4]) -> [i32; 4] {
    let across = |v: i32| (v * BASE_X * 2 + 4).div_euclid(8);
    let down = |v: i32| (v * BASE_Y * 2 + 8).div_euclid(16);
    [across(r[0]), down(r[1]), across(r[2]), down(r[3])]
}

/// The class a template's item names: the six with numbers by their names, the rest as they are.
fn item_class(class: &Id) -> Option<String> {
    Some(match class {
        Id::Num(0x80) => "Button".into(),
        Id::Num(0x81) => "Edit".into(),
        Id::Num(0x82) => "Static".into(),
        Id::Num(0x83) => "ListBox".into(),
        Id::Num(0x84) => "ScrollBar".into(),
        Id::Num(0x85) => "ComboBox".into(),
        Id::Name(name) => name.clone(),
        _ => return None,
    })
}

/// The dialog manager's own state for a dialog.
#[derive(Debug, Clone, Default)]
pub struct Dialog {
    pub font: u64,
    /// The control that had the focus when it was last active, to have it again.
    pub focus: u64,
    /// What `EndDialog` gave, once it's been called.
    pub ended: Option<u64>,
    /// `SHBrowseForFolder`'s, for the dialog it shows: shell32's own, with no dialog procedure.
    pub browse: Option<crate::os::shellapi::Browse>,
}

/// `DWLP_MSGRESULT`, `DWLP_DLGPROC` and `DWLP_USER`: a pointer each, from the start of a dialog's extra bytes.
pub fn dwlp(width: u64) -> (usize, usize, usize) {
    let w = width as usize;
    (0, w, 2 * w)
}

/// Reads part of the program's memory as far as it's there, up to `most` bytes: an in-memory template's size isn't
/// known before it's read.
fn read_upto(task: &mut Task, cx: &mut Cx<'_>, at: u64, most: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut p = at;
    while out.len() < most {
        let room = (4096 - (p % 4096)) as usize;
        let n = room.min(most - out.len());
        match task.mem(cx, false, |t| user::read_vec(&t.space(), p, n)) {
            Ok(b) => out.extend(b),
            Err(_) => break,
        }
        p += n as u64;
    }
    out
}

impl Task {
    // ---- Extra bytes ----

    /// A pointer-sized value in a window's extra bytes, as this process's pointers are.
    pub fn window_extra(&self, hwnd: u64, at: usize) -> u64 {
        let width = self.abi().width as usize;
        let w = self.win32k();
        let Some(win) = w.windows.get(&hwnd) else { return 0 };
        let mut b = [0u8; 8];
        if let Some(s) = win.extra.get(at..at + width) {
            b[..width].copy_from_slice(s);
        }
        u64::from_le_bytes(b)
    }

    pub fn set_window_extra(&self, hwnd: u64, at: usize, value: u64) {
        let width = self.abi().width as usize;
        let mut w = self.win32k();
        if let Some(s) = w.windows.get_mut(&hwnd).and_then(|win| win.extra.get_mut(at..at + width)) {
            s.copy_from_slice(&value.to_le_bytes()[..width]);
        }
    }

    // ---- Templates ----

    /// A dialog template from a module's resources (`RT_DIALOG`): its address and its bytes.
    pub fn dialog_resource(&mut self, cx: &mut Cx<'_>, instance: u64, name: u64, wide: bool) -> Option<(u64, Vec<u8>)> {
        let name = self.resource_id(cx, name, wide)?;
        let (_, data, size) = self.find_resource(cx, instance, &crate::resources::Id::Num(5), &name, None).ok()?;
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), data, size as usize)).ok()?;
        Some((data, bytes))
    }

    /// A template in the program's memory: its bytes, as many as it could be.
    pub fn dialog_in_memory(&mut self, cx: &mut Cx<'_>, at: u64) -> Vec<u8> {
        read_upto(self, cx, at, 1 << 16)
    }

    // ---- Making a dialog ----

    /// Makes a window as `CreateWindowExW` does, from win32k's own strings: the class's name and its text go on
    /// the stack while it's made.
    #[allow(clippy::too_many_arguments)]
    pub fn create_window_named(&mut self, cx: &mut Cx<'_>, ex_style: u32, class: &str, text: &str, style: u32, rect: [i32; 4], parent: u64, menu: u64, instance: u64, param: u64) -> Result<u64, Reply> {
        let wide = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() };
        let (class_w, text_w) = (wide(class), wide(text));
        let len = class_w.len() + text_w.len();
        let bytes: Vec<u8> = class_w.iter().chain(&text_w).copied().collect();
        let class_len = class_w.len() as u64;
        self.with_scratch(cx, len, |_| bytes.clone(), |task, cx, at| {
            let a = [
                u64::from(ex_style),
                at,
                at + class_len,
                u64::from(style),
                rect[0] as u32 as u64,
                rect[1] as u32 as u64,
                rect[2] as u32 as u64,
                rect[3] as u32 as u64,
                parent,
                menu,
                instance,
                param,
            ];
            task.create_window(cx, &a, false)
        })
    }

    /// `CreateDialogIndirectParamW` and what `CreateDialogParamW` and the `DialogBox` functions make: the dialog and
    /// its controls from a template at `at` (where its items' creation data is), with `WM_SETFONT` and then
    /// `WM_INITDIALOG`. Shown if its style says, or if it's `modal`. 0 if it, or a control, couldn't be made.
    #[allow(clippy::too_many_arguments)]
    pub fn create_dialog(&mut self, cx: &mut Cx<'_>, t: &Template, at: u64, instance: u64, owner: u64, proc: u64, param: u64, modal: bool) -> Result<u64, Reply> {
        let class = match &t.class {
            Id::None => "#32770".to_string(),
            Id::Name(name) => name.clone(),
            Id::Num(atom) => {
                let w = self.win32k();
                match w.class_by_atom(*atom) {
                    Some(name) => name,
                    None => return Ok(0),
                }
            }
        };
        let child = t.style & WS_CHILD != 0;
        let style = t.style & !(WS_VISIBLE | DS_SETFONT);
        let [x, y, width, height] = units_to_pixels(t.rect);
        // The client area is the template's size; a framed window's is inside its frame.
        let framed = !child && super::framed(style | if style & WS_POPUP == 0 { WS_CAPTION } else { 0 });
        let insets = display::desktop::Insets::of(framed);
        let rect = if child {
            [x, y, width, height]
        } else {
            let (sw, sh) = self.desktop(|d| (d.output().width, d.output().height));
            let outer = insets.around(&Rect::new(0, 0, width, height));
            let (ow, oh) = (outer.width(), outer.height());
            let origin = {
                let w = self.win32k();
                w.top_level_of(owner).and_then(|o| w.screen_origin(o)).unwrap_or((0, 0))
            };
            if t.style & (DS_CENTER | DS_CENTERMOUSE) != 0 {
                [(sw - ow).max(0) / 2, (sh - oh).max(0) / 2, ow, oh]
            } else {
                // Its place is its client area's, from its owner's.
                [origin.0 + x - insets.left, origin.1 + y - insets.top, ow, oh]
            }
        };
        // A child dialog's parent is the window it's in; a top-level one's, as CreateWindowEx has it, its owner.
        let parent = owner;
        let ex_style = t.ex_style | if t.style & DS_CONTROL != 0 { WS_EX_CONTROLPARENT } else { 0 };
        let hwnd = self.create_window_named(cx, ex_style, &class, &t.title, style, rect, parent, 0, instance, param)?;
        if hwnd == 0 {
            return Ok(0);
        }
        let font = if t.font.is_some() { self.win32k().gdi.create(gdi::Object::Font) } else { 0 };
        self.win32k().dialogs.insert(hwnd, Dialog { font, ..Dialog::default() });
        let (_, dlgproc, _) = dwlp(self.abi().width);
        self.set_window_extra(hwnd, dlgproc, proc);
        if font != 0 {
            self.send_message(cx, hwnd, WM_SETFONT, font, 0)?;
        }
        for item in &t.items {
            let Some(class) = item_class(&item.class) else { continue };
            let title = match &item.title {
                Id::Name(s) => s.clone(),
                // An ordinal names a static's icon or bitmap in the dialog's module.
                Id::Num(n) if class.eq_ignore_ascii_case("static") => format!("#{n}"),
                _ => String::new(),
            };
            let style = item.style | WS_CHILD;
            // WS_EX_NOPARENTNOTIFY: a dialog's controls don't tell it they were made.
            let ex_style = item.ex_style | 0x4;
            let create = item.data.map_or(0, |o| at + o as u64);
            let control = self.create_window_named(cx, ex_style, &class, &title, style, units_to_pixels(item.rect), hwnd, u64::from(item.id), instance, create)?;
            if control == 0 {
                if t.style & DS_NOFAILCREATE != 0 {
                    continue;
                }
                self.destroy_window(cx, hwnd)?;
                return Ok(0);
            }
            if font != 0 {
                self.send_message(cx, control, WM_SETFONT, font, 0)?;
            }
        }
        let first = self.next_tab_item(hwnd, 0, false);
        let set = self.send_message(cx, hwnd, WM_INITDIALOG, first, param)?;
        if !self.window_exists(hwnd) {
            return Ok(0);
        }
        // TRUE: the focus goes to the first control that takes it; FALSE: the dialog procedure set it.
        if set != 0 && first != 0 {
            if let Some(d) = self.win32k().dialogs.get_mut(&hwnd) {
                d.focus = first;
            }
            self.focus_control(cx, first)?;
        }
        if t.style & WS_VISIBLE != 0 || modal {
            let ended = self.win32k().dialogs.get(&hwnd).is_some_and(|d| d.ended.is_some());
            if !ended {
                self.show_window(cx, hwnd, SW_SHOW)?;
            }
        }
        Ok(hwnd)
    }

    /// Gives a control the focus, its text all selected if it's an edit (`DLGC_HASSETSEL`), as Tab does.
    fn focus_control(&mut self, cx: &mut Cx<'_>, control: u64) -> Result<(), Reply> {
        let code = self.send_message(cx, control, WM_GETDLGCODE, 0, 0)?;
        self.set_focus(cx, control)?;
        if code & DLGC_HASSETSEL != 0 {
            self.send_message(cx, control, EM_SETSEL, 0, u64::MAX)?;
        }
        Ok(())
    }

    /// `DialogBoxIndirectParamW`, and what `DialogBoxParamW` runs: the dialog, its owner disabled, and its own
    /// message loop until `EndDialog`. What `EndDialog` gave, or -1 if it couldn't be made.
    #[allow(clippy::too_many_arguments)]
    pub fn dialog_box(&mut self, cx: &mut Cx<'_>, t: &Template, at: u64, instance: u64, owner: u64, proc: u64, param: u64) -> Result<u64, Reply> {
        self.dialog_box_with(cx, t, at, instance, owner, proc, param, |_, _, _| Ok(()))
    }

    /// `dialog_box`, with `setup` run once the dialog's made, before its loop: what a dialog of the system's own does
    /// before it's used.
    #[allow(clippy::too_many_arguments)]
    pub fn dialog_box_with(&mut self, cx: &mut Cx<'_>, t: &Template, at: u64, instance: u64, owner: u64, proc: u64, param: u64, setup: impl FnOnce(&mut Task, &mut Cx<'_>, u64) -> Result<(), Reply>) -> Result<u64, Reply> {
        // The owner is the top-level window it's in.
        let owner = {
            let w = self.win32k();
            if owner == 0 { 0 } else { w.top_level_of(owner).unwrap_or(0) }
        };
        let owner_was_enabled = owner != 0 && self.win32k().enabled(owner);
        if owner_was_enabled {
            self.enable_window(cx, owner, false)?;
        }
        let hwnd = self.create_dialog(cx, t, at, instance, owner, proc, param, true)?;
        if hwnd == 0 {
            if owner_was_enabled {
                self.enable_window(cx, owner, true)?;
            }
            return Ok(u64::MAX);
        }
        setup(self, cx, hwnd)?;
        let all = super::Filter { hwnd: 0, min: 0, max: 0 };
        let result = loop {
            if let Some(result) = self.win32k().dialogs.get(&hwnd).map(|d| d.ended) {
                if let Some(result) = result {
                    break result;
                }
            } else {
                // Destroyed without EndDialog.
                break 0;
            }
            match self.next_message(cx, all, true, true)? {
                Some(m) if m.message == WM_QUIT => {
                    // WM_QUIT ends the dialog and goes on to the loop around it.
                    self.post_quit(m.wparam);
                    break 0;
                }
                Some(m) => {
                    let handled = self.is_dialog_message(cx, hwnd, &m)?;
                    if !handled {
                        self.translate(&m);
                        self.dispatch(cx, m)?;
                    }
                }
                None => {}
            }
        };
        if owner_was_enabled {
            self.enable_window(cx, owner, true)?;
        }
        if self.window_exists(hwnd) {
            self.destroy_window(cx, hwnd)?;
        }
        if owner != 0 && self.window_exists(owner) {
            self.activate(cx, owner)?;
        }
        Ok(result)
    }

    /// `EndDialog`: the dialog hidden, its result kept for its loop, and the loop woken. FALSE for a window that
    /// isn't there.
    pub fn end_dialog(&mut self, cx: &mut Cx<'_>, hwnd: u64, result: u64) -> Result<bool, Reply> {
        {
            let mut w = self.win32k();
            if !w.windows.contains_key(&hwnd) {
                return Ok(false);
            }
            w.dialogs.entry(hwnd).or_default().ended = Some(result);
        }
        self.show_window(cx, hwnd, 0)?;
        // WM_NULL wakes its loop, which may be waiting with nothing posted: EndDialog called while a message sent
        // from another thread was being dealt with, inside that wait.
        let tid = self.tid;
        self.post_message(hwnd, tid, 0, 0, 0);
        Ok(true)
    }

    /// `EnableWindow`: whether it was disabled before. A window disabled loses the focus, and hears WM_ENABLE.
    pub fn enable_window(&mut self, cx: &mut Cx<'_>, hwnd: u64, enable: bool) -> Result<bool, Reply> {
        let (was_disabled, changed) = {
            let mut w = self.win32k();
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(false) };
            let was = win.style & WS_DISABLED != 0;
            if enable {
                win.style &= !WS_DISABLED;
            } else {
                win.style |= WS_DISABLED;
            }
            (was, was == enable)
        };
        if changed {
            if !enable {
                let focus = self.win32k().focus;
                if focus != 0 && (focus == hwnd || is_within(self, focus, hwnd)) {
                    self.set_focus(cx, 0)?;
                }
            }
            self.send_message(cx, hwnd, super::WM_ENABLE, u64::from(enable), 0)?;
            self.invalidate(hwnd, None, true);
        }
        Ok(was_disabled)
    }

    // ---- The dialog manager ----

    /// A dialog's controls in Tab's order, those in a child that's a parent of controls (`WS_EX_CONTROLPARENT`)
    /// among them in its place, as the dialog manager walks them.
    fn tab_items(&self, dialog: u64) -> Vec<u64> {
        let w = self.win32k();
        let mut out = Vec::new();
        fn walk(w: &super::Win32k, parent: u64, out: &mut Vec<u64>) {
            for (&h, c) in w.windows.iter().filter(|(_, c)| c.parent == parent) {
                if c.ex_style & WS_EX_CONTROLPARENT != 0 {
                    if c.visible && c.style & WS_DISABLED == 0 {
                        walk(w, h, out);
                    }
                } else {
                    out.push(h);
                }
            }
        }
        walk(&w, dialog, &mut out);
        out
    }

    /// `GetNextDlgTabItem`: the next control after `from` (or before it, with `previous`) that's shown, enabled and
    /// `WS_TABSTOP`, around and around; with `from` 0, the first. 0 if there's none.
    pub fn next_tab_item(&self, dialog: u64, from: u64, previous: bool) -> u64 {
        let items = self.tab_items(dialog);
        let w = self.win32k();
        // Its own WS_VISIBLE, as the dialog manager reads it: a dialog's controls are walked before it's shown.
        let takes = |h: &u64| w.windows.get(h).is_some_and(|c| c.visible && c.style & WS_TABSTOP != 0 && c.style & WS_DISABLED == 0);
        let n = items.len();
        if n == 0 {
            return 0;
        }
        let start = items.iter().position(|&h| h == from);
        for step in 1..=n {
            let i = match (start, previous) {
                (None, false) => step - 1,
                (None, true) => n - step,
                (Some(s), false) => (s + step) % n,
                (Some(s), true) => (s + n - step % n) % n,
            };
            if takes(&items[i]) {
                return items[i];
            }
        }
        0
    }

    /// `GetNextDlgGroupItem`: the next control in `from`'s group (from a `WS_GROUP` control to before the next),
    /// shown and enabled, around and around.
    pub fn next_group_item(&self, dialog: u64, from: u64, previous: bool) -> u64 {
        let items = self.tab_items(dialog);
        let w = self.win32k();
        let Some(at) = items.iter().position(|&h| h == from) else { return 0 };
        let group = |h: u64| w.windows.get(&h).is_some_and(|c| c.style & WS_GROUP != 0);
        let start = (0..=at).rev().find(|&i| group(items[i])).unwrap_or(0);
        let end = (at + 1..items.len()).find(|&i| group(items[i])).unwrap_or(items.len());
        let members = &items[start..end];
        let n = members.len();
        let i = members.iter().position(|&h| h == from).unwrap_or(0);
        for step in 1..=n {
            let j = if previous { (i + n - step % n) % n } else { (i + step) % n };
            let h = members[j];
            if w.windows.get(&h).is_some_and(|c| c.visible && c.style & WS_DISABLED == 0) {
                return h;
            }
        }
        from
    }

    /// The default push button's ID: what DM_SETDEFID set, or the first `BS_DEFPUSHBUTTON` control's.
    fn default_id(&self, dialog: u64) -> Option<u64> {
        let items = self.tab_items(dialog);
        let w = self.win32k();
        items.into_iter().find_map(|h| {
            let c = w.windows.get(&h)?;
            (c.class == "button" && c.style & BS_TYPE == BS_DEFPUSHBUTTON).then_some(c.id)
        })
    }

    /// A dialog's control by its ID: `GetDlgItem`, among its children.
    pub fn dialog_item(&self, dialog: u64, id: u64) -> u64 {
        let w = self.win32k();
        w.windows.iter().find(|(_, c)| c.parent == dialog && c.id as u32 == id as u32).map_or(0, |(&h, _)| h)
    }

    /// `WM_COMMAND` for a button pressed, to its dialog: its ID and BN_CLICKED, and the control.
    fn command(&mut self, cx: &mut Cx<'_>, dialog: u64, id: u64) -> Result<(), Reply> {
        let control = self.dialog_item(dialog, id);
        if control != 0 && !self.win32k().enabled(control) {
            return Ok(());
        }
        self.send_message(cx, dialog, WM_COMMAND, id & 0xffff, control)?;
        Ok(())
    }

    /// `IsDialogMessageW`: whether the message was for the dialog (or a window in it), and dealt with as a dialog
    /// does: Tab and Shift+Tab move the focus, Enter presses the default button (or IDOK), Escape IDCANCEL, an arrow
    /// moves within a group of radio buttons; the rest translated and dispatched as usual.
    pub fn is_dialog_message(&mut self, cx: &mut Cx<'_>, dialog: u64, m: &Msg) -> Result<bool, Reply> {
        let inside = m.hwnd == dialog || is_within(self, m.hwnd, dialog);
        if !inside {
            return Ok(false);
        }
        let code = if m.hwnd != dialog && matches!(m.message, WM_KEYDOWN | super::WM_CHAR) {
            self.send_message(cx, m.hwnd, WM_GETDLGCODE, m.wparam, 0)?
        } else {
            0
        };
        let key = m.wparam as u8;
        match m.message {
            WM_KEYDOWN if key == keys::VK_TAB && code & (DLGC_WANTTAB | DLGC_WANTALLKEYS) == 0 => {
                let shift = self.win32k().thread_keys(self.tid)[keys::VK_SHIFT as usize] & 0x80 != 0;
                let from = self.win32k().focus;
                let next = self.next_tab_item(dialog, from, shift);
                if next != 0 {
                    self.focus_control(cx, next)?;
                }
                return Ok(true);
            }
            WM_KEYDOWN if key == keys::VK_RETURN && code & DLGC_WANTALLKEYS == 0 => {
                // A push button with the focus is pressed; else the default button, or IDOK.
                let focus = self.win32k().focus;
                let id = if code & (DLGC_DEFPUSHBUTTON | DLGC_UNDEFPUSHBUTTON) != 0 {
                    self.win32k().windows.get(&focus).map_or(IDOK, |c| c.id)
                } else {
                    self.default_id(dialog).unwrap_or(IDOK)
                };
                self.command(cx, dialog, id)?;
                return Ok(true);
            }
            WM_KEYDOWN if key == keys::VK_ESCAPE && code & DLGC_WANTALLKEYS == 0 => {
                self.command(cx, dialog, IDCANCEL)?;
                return Ok(true);
            }
            WM_KEYDOWN if matches!(key, keys::VK_LEFT | keys::VK_UP | keys::VK_RIGHT | keys::VK_DOWN) && code & (DLGC_WANTARROWS | DLGC_WANTALLKEYS) == 0 => {
                let from = self.win32k().focus;
                let previous = matches!(key, keys::VK_LEFT | keys::VK_UP);
                let next = self.next_group_item(dialog, from, previous);
                if next != 0 && next != from {
                    self.set_focus(cx, next)?;
                    // A radio button moved to is chosen, as a click would.
                    if self.send_message(cx, next, WM_GETDLGCODE, 0, 0)? & DLGC_RADIOBUTTON != 0 {
                        self.send_message(cx, next, super::controls::BM_CLICK, 0, 0)?;
                    }
                }
                return Ok(true);
            }
            // The characters those keys type go nowhere.
            super::WM_CHAR if matches!(m.wparam, 9 | 13 | 27) && code & (DLGC_WANTCHARS | DLGC_WANTALLKEYS | DLGC_WANTTAB) == 0 => return Ok(true),
            _ => {}
        }
        self.translate(m);
        self.dispatch(cx, *m)?;
        Ok(true)
    }

    /// `DefDlgProcW`: calls the dialog's procedure, and does what it doesn't. A non-zero result from it means it dealt
    /// with the message, the result then in `DWLP_MSGRESULT`, but for the messages whose result it gives itself.
    pub fn def_dlg_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let (msgresult, dlgproc, _) = dwlp(self.abi().width);
        let proc = self.window_extra(hwnd, dlgproc);
        if proc != 0 {
            let width = self.abi().width;
            let handled = self.upcall(cx, proc, &[hwnd, u64::from(message), wparam, lparam])?;
            let handled = if width == 4 { handled & 0xffff_ffff } else { handled };
            if handled != 0 {
                const WM_COMPAREITEM: u32 = 0x39;
                const WM_VKEYTOITEM: u32 = 0x2e;
                const WM_CHARTOITEM: u32 = 0x2f;
                const WM_QUERYDRAGICON: u32 = 0x37;
                return Ok(match message {
                    WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN | WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | 0x137 | WM_COMPAREITEM | WM_VKEYTOITEM | WM_CHARTOITEM | WM_QUERYDRAGICON | WM_INITDIALOG => handled,
                    _ => self.window_extra(hwnd, msgresult),
                });
            }
            if !self.window_exists(hwnd) {
                return Ok(0);
            }
        }
        if self.win32k().dialogs.get(&hwnd).is_some_and(|d| d.browse.is_some())
            && let Some(r) = self.browse_message(cx, hwnd, message, wparam, lparam)?
        {
            return Ok(r);
        }
        match message {
            WM_ERASEBKGND => {
                // The class's brush, or a dialog's face.
                if !self.erase_background(hwnd, wparam) {
                    let brush = super::Gdi::sys_brush(15);
                    let client = self.win32k().windows.get(&hwnd).map(super::Window::client).unwrap_or_default();
                    self.win32k().fill_rect(wparam, client, brush);
                }
                Ok(1)
            }
            WM_PAINT => self.def_window_proc(cx, hwnd, message, wparam, lparam),
            // Closing it is Cancel, unless Cancel is there and disabled.
            WM_CLOSE => {
                let cancel = self.dialog_item(hwnd, IDCANCEL);
                if cancel == 0 || self.win32k().enabled(cancel) {
                    let tid = self.tid;
                    self.post_message(hwnd, tid, WM_COMMAND, IDCANCEL, cancel);
                }
                Ok(0)
            }
            WM_ACTIVATE => {
                if wparam & 0xffff != 0 {
                    let saved = self.win32k().dialogs.get(&hwnd).map_or(0, |d| d.focus);
                    let target = if saved != 0 && self.window_exists(saved) { saved } else { self.next_tab_item(hwnd, 0, false) };
                    if target != 0 {
                        self.set_focus(cx, target)?;
                    } else {
                        self.set_focus(cx, hwnd)?;
                    }
                } else {
                    let focus = self.win32k().focus;
                    if is_within(self, focus, hwnd)
                        && let Some(d) = self.win32k().dialogs.get_mut(&hwnd)
                    {
                        d.focus = focus;
                    }
                }
                Ok(0)
            }
            WM_SETFOCUS => {
                let saved = self.win32k().dialogs.get(&hwnd).map_or(0, |d| d.focus);
                let target = if saved != 0 && self.window_exists(saved) { saved } else { self.next_tab_item(hwnd, 0, false) };
                if target != 0 {
                    self.set_focus(cx, target)?;
                }
                Ok(0)
            }
            DM_GETDEFID => Ok(self.default_id(hwnd).map_or(0, |id| DC_HASDEFID << 16 | (id & 0xffff))),
            DM_SETDEFID => {
                // The button that was the default isn't, and this one is.
                let items = self.tab_items(hwnd);
                let changed: Vec<u64> = {
                    let mut w = self.win32k();
                    let mut changed = Vec::new();
                    for h in items {
                        let Some(c) = w.windows.get_mut(&h) else { continue };
                        if c.class != "button" || !matches!(c.style & BS_TYPE, BS_PUSHBUTTON | BS_DEFPUSHBUTTON) {
                            continue;
                        }
                        let ty = if c.id as u32 == wparam as u32 { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON };
                        if c.style & BS_TYPE != ty {
                            c.style = c.style & !BS_TYPE | ty;
                            changed.push(h);
                        }
                    }
                    changed
                };
                for h in changed {
                    self.invalidate(h, None, true);
                }
                Ok(1)
            }
            DM_REPOSITION => Ok(0),
            WM_NEXTDLGCTL => {
                let target = if lparam & 0xffff != 0 {
                    wparam
                } else {
                    let from = self.win32k().focus;
                    self.next_tab_item(hwnd, from, wparam != 0)
                };
                if target != 0 && self.window_exists(target) {
                    self.focus_control(cx, target)?;
                }
                Ok(0)
            }
            WM_SETFONT => {
                if let Some(d) = self.win32k().dialogs.get_mut(&hwnd) {
                    d.font = wparam;
                }
                Ok(0)
            }
            WM_GETFONT => Ok(self.win32k().dialogs.get(&hwnd).map_or(0, |d| d.font)),
            WM_INITDIALOG | WM_GETDLGCODE => Ok(0),
            _ => self.def_window_proc(cx, hwnd, message, wparam, lparam),
        }
    }

}

/// A rectangle in dialog units, made pixels: `MapDialogRect`, each edge on its own.
pub fn map_dialog_rect(r: &Rect) -> Rect {
    let [l, t, right, bottom] = units_to_pixels([r.left, r.top, r.right, r.bottom]);
    Rect::new(l, t, right, bottom)
}

/// Whether `hwnd` is inside `ancestor`.
fn is_within(task: &Task, hwnd: u64, ancestor: u64) -> bool {
    let w = task.win32k();
    let mut h = hwnd;
    while let Some(c) = w.windows.get(&h) {
        if c.parent == ancestor {
            return true;
        }
        if c.parent == DESKTOP {
            return false;
        }
        h = c.parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    /// A DLGTEMPLATE with a font, a button and a static, as rc writes one.
    fn template() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend((0x80c8_0000u32 | DS_SETFONT).to_le_bytes()); // WS_POPUP | WS_CAPTION | WS_SYSMENU, DS_SETFONT
        b.extend(0u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        for v in [10i16, 20, 200, 100] {
            b.extend(v.to_le_bytes());
        }
        b.extend([0, 0, 0, 0]); // no menu, the dialog class
        b.extend(wide("Title"));
        b.extend(8u16.to_le_bytes());
        b.extend(wide("MS Shell Dlg"));
        // The button.
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend(0x5001_0001u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        for v in [5i16, 6, 50, 14] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1u16.to_le_bytes());
        b.extend([0xff, 0xff, 0x80, 0x00]);
        b.extend(wide("OK"));
        b.extend(0u16.to_le_bytes());
        // The static, with two bytes of creation data.
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend(0x5000_0000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        for v in [5i16, 30, 100, 8] {
            b.extend(v.to_le_bytes());
        }
        b.extend(0xffffu16.to_le_bytes());
        b.extend([0xff, 0xff, 0x82, 0x00]);
        b.extend([0xff, 0xff, 0x07, 0x00]); // an ordinal title, as an icon's is
        b.extend(2u16.to_le_bytes());
        b.extend([0xab, 0xcd]);
        b
    }

    #[test]
    fn parses_a_dlgtemplate() {
        let t = parse(&template()).expect("parses");
        assert_eq!(t.rect, [10, 20, 200, 100]);
        assert_eq!(t.title, "Title");
        assert_eq!(t.font, Some((8, "MS Shell Dlg".into())));
        assert_eq!(t.class, Id::None);
        assert_eq!(t.items.len(), 2);
        assert_eq!((t.items[0].id, &t.items[0].class, &t.items[0].title), (1, &Id::Num(0x80), &Id::Name("OK".into())));
        assert_eq!(t.items[0].data, None);
        assert_eq!((t.items[1].id, &t.items[1].title), (0xffff, &Id::Num(7)));
        assert!(t.items[1].data.is_some());
    }

    #[test]
    fn parses_a_dlgtemplateex() {
        let mut b = vec![1, 0, 0xff, 0xff];
        b.extend(0u32.to_le_bytes()); // helpID
        b.extend(0x8u32.to_le_bytes()); // exStyle: WS_EX_TOPMOST
        b.extend((0x8000_0000u32 | DS_SETFONT).to_le_bytes());
        b.extend(1u16.to_le_bytes());
        for v in [0i16, 0, 100, 50] {
            b.extend(v.to_le_bytes());
        }
        b.extend([0, 0]);
        b.extend(wide("_Nb"));
        b.extend(wide(""));
        b.extend(8u16.to_le_bytes());
        b.extend(400u16.to_le_bytes());
        b.extend([0, 1]);
        b.extend(wide("MS Shell Dlg"));
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend(0u32.to_le_bytes());
        b.extend(0x200u32.to_le_bytes()); // WS_EX_CLIENTEDGE
        b.extend(0x5081_0080u32.to_le_bytes());
        for v in [7i16, 7, 80, 12] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1200u32.to_le_bytes());
        b.extend(wide("Edit"));
        b.extend([0, 0]);
        b.extend(0u16.to_le_bytes());
        let t = parse(&b).expect("parses");
        assert_eq!((t.ex_style, t.class.clone()), (0x8, Id::Name("_Nb".into())));
        assert_eq!(t.items[0].id, 1200);
        assert_eq!(t.items[0].ex_style, 0x200);
        assert_eq!(t.items[0].class, Id::Name("Edit".into()));
        assert_eq!(t.items[0].title, Id::None);
    }

    #[test]
    fn a_short_template_is_refused() {
        assert!(parse(&template()[..40]).is_none());
    }

    #[test]
    fn dialog_units() {
        // 4 across is the base width, 8 down the base height.
        assert_eq!(units_to_pixels([4, 8, 40, 16]), [BASE_X, BASE_Y, 10 * BASE_X, 2 * BASE_Y]);
    }
}
