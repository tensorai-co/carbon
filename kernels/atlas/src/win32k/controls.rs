//! The standard controls whose procedures are win32k's own: buttons (push buttons, check boxes, radio buttons and
//! group boxes), statics (text, rectangles, frames and etched lines) and edits (one line, or several). Each paints
//! itself with win32k's GDI into its top-level window's pixels, in the colours its parent gives with `WM_CTLCOLOR*`,
//! and tells its parent what happened with `WM_COMMAND` (`BN_CLICKED`, `EN_CHANGE`). Text is drawn in the one font;
//! a static's wraps at its width. Images (a static's icon or bitmap, a button's) are kept and not drawn yet.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::font::{CELL_HEIGHT, CELL_WIDTH};
use display::{Rect, Rgb, Surface};

use super::dialog::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, BS_TYPE, DLGC_BUTTON, DLGC_DEFPUSHBUTTON, DLGC_HASSETSEL, DLGC_RADIOBUTTON,
    DLGC_STATIC, DLGC_UNDEFPUSHBUTTON, DLGC_WANTALLKEYS, DLGC_WANTARROWS, DLGC_WANTCHARS,
};
use super::{
    WM_CHAR, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_ENABLE, WM_ERASEBKGND,
    WM_GETDLGCODE, WM_GETFONT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_SETFOCUS, WM_SETFONT, WM_SETTEXT, WS_GROUP, gdi, keys,
};
use crate::os::Reply;
use crate::process::Task;

pub const BM_GETCHECK: u32 = 0xf0;
pub const BM_SETCHECK: u32 = 0xf1;
pub const BM_GETSTATE: u32 = 0xf2;
pub const BM_SETSTATE: u32 = 0xf3;
pub const BM_SETSTYLE: u32 = 0xf4;
pub const BM_CLICK: u32 = 0xf5;
pub const BM_GETIMAGE: u32 = 0xf6;
pub const BM_SETIMAGE: u32 = 0xf7;
const BST_PUSHED: u64 = 4;
const BST_FOCUS: u64 = 8;
const BN_CLICKED: u64 = 0;

const BS_CHECKBOX: u32 = 2;
const BS_AUTOCHECKBOX: u32 = 3;
const BS_RADIOBUTTON: u32 = 4;
const BS_3STATE: u32 = 5;
const BS_AUTO3STATE: u32 = 6;
const BS_GROUPBOX: u32 = 7;
const BS_AUTORADIOBUTTON: u32 = 9;
const BS_PUSHBOX: u32 = 0xa;
const BS_OWNERDRAW: u32 = 0xb;

const STM_SETICON: u32 = 0x170;
const STM_GETICON: u32 = 0x171;
const STM_SETIMAGE: u32 = 0x172;
const STM_GETIMAGE: u32 = 0x173;
const SS_NOTIFY: u32 = 0x100;
const SS_NOPREFIX: u32 = 0x80;
const SS_SUNKEN: u32 = 0x1000;
const STN_CLICKED: u64 = 0;

const EM_GETSEL: u32 = 0xb0;
pub const EM_SETSEL: u32 = 0xb1;
const EM_SCROLLCARET: u32 = 0xb7;
const EM_GETMODIFY: u32 = 0xb8;
const EM_SETMODIFY: u32 = 0xb9;
const EM_GETLINECOUNT: u32 = 0xba;
const EM_REPLACESEL: u32 = 0xc2;
const EM_LIMITTEXT: u32 = 0xc5;
const EM_SETPASSWORDCHAR: u32 = 0xcc;
const EM_SETREADONLY: u32 = 0xcf;
const EM_GETPASSWORDCHAR: u32 = 0xd2;
const EM_SETMARGINS: u32 = 0xd3;
const EM_GETLIMITTEXT: u32 = 0xd5;
const ES_MULTILINE: u32 = 0x4;
const ES_UPPERCASE: u32 = 0x8;
const ES_LOWERCASE: u32 = 0x10;
const ES_PASSWORD: u32 = 0x20;
const ES_READONLY: u32 = 0x800;
const ES_NUMBER: u32 = 0x2000;
const EN_SETFOCUS: u64 = 0x100;
const EN_KILLFOCUS: u64 = 0x200;
const EN_CHANGE: u64 = 0x300;
const EN_UPDATE: u64 = 0x400;
const EN_MAXTEXT: u64 = 0x501;
/// An edit's text, unless `EM_LIMITTEXT` says otherwise.
const EDIT_LIMIT: usize = 30000;

pub(super) const WS_BORDER: u32 = 0x0080_0000;
pub(super) const WS_EX_CLIENTEDGE: u32 = 0x200;
const WS_EX_STATICEDGE: u32 = 0x2_0000;

const FACE: Rgb = 0xe1e1e1;
const FACE_PRESSED: Rgb = 0xcce4f7;
const EDGE: Rgb = 0xadadad;
pub(super) const ACCENT: Rgb = 0x0078d7;
const MARK: Rgb = 0x333333;
pub(super) const BORDER: Rgb = 0x7a7a7a;
pub(super) const GRAY: Rgb = 0x6d6d6d;

/// A control's own state.
#[derive(Debug, Clone, Default)]
pub struct Control {
    /// A check box's or radio button's: BST_UNCHECKED, BST_CHECKED or BST_INDETERMINATE.
    pub check: u64,
    /// Held down: by the mouse, Space, or `BM_SETSTATE`.
    pub pressed: bool,
    pub font: u64,
    /// `BM_SETIMAGE`'s or `STM_SETIMAGE`'s, kept.
    pub image: u64,
    /// An edit's caret and the other end of its selection, in characters of its text.
    pub caret: usize,
    pub anchor: usize,
    /// Its limit, if `EM_LIMITTEXT` set one.
    pub limit: Option<usize>,
    /// The first character a one-line edit shows: as far along as keeps its caret in view.
    pub scroll: usize,
    /// `EM_SETPASSWORDCHAR`'s, or `ES_PASSWORD`'s asterisk.
    pub password: Option<u16>,
    pub modified: bool,
    /// A progress bar's range, position and colours.
    pub progress: super::comctl::Progress,
    /// A list view's columns and items.
    pub list: super::comctl::ListView,
}

/// A label's text without its `&` mnemonics (`&&` is an `&`), and the place of the character it marks.
pub fn label(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Text in lines no wider than `width` characters, broken at spaces where it can be and mid-word where it can't, its
/// own line breaks kept.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word = word.to_string();
            loop {
                let have = line.chars().count();
                let need = word.chars().count() + usize::from(have > 0);
                if have + need <= width {
                    if have > 0 {
                        line.push(' ');
                    }
                    line.push_str(&word);
                    break;
                }
                if have > 0 {
                    lines.push(std::mem::take(&mut line));
                    continue;
                }
                // Longer than a line on its own: as much as fits.
                let head: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                lines.push(head);
                if word.is_empty() {
                    break;
                }
            }
        }
        lines.push(line);
    }
    lines
}

/// Draws a rectangle of dots, as a focused control's outline is.
pub(super) fn dotted(s: &mut Surface, r: &Rect, color: Rgb, clip: &Rect) {
    for x in (r.left..r.right).step_by(2) {
        s.set(x, r.top, color, clip);
        s.set(x, r.bottom - 1, color, clip);
    }
    for y in (r.top..r.bottom).step_by(2) {
        s.set(r.left, y, color, clip);
        s.set(r.right - 1, y, color, clip);
    }
}

/// The colours a control is drawn in, as its parent gave them: the background (None to leave what's under it), the
/// text's, and the text's background (None for TRANSPARENT).
pub(super) struct Colors {
    pub background: Option<Rgb>,
    pub text: Rgb,
    pub text_background: Option<Rgb>,
}

impl Task {
    /// Works on a control's state, made the first time it's asked for.
    pub(super) fn control<R>(&self, hwnd: u64, f: impl FnOnce(&mut super::Window, &mut Control) -> R) -> Option<R> {
        let mut w = self.win32k();
        let w = &mut *w;
        let win = w.windows.get_mut(&hwnd)?;
        let c = w.controls.entry(hwnd).or_default();
        Some(f(win, c))
    }

    /// Tells a control's parent something happened: `WM_COMMAND` with its ID and `code`.
    pub(super) fn notify(&mut self, cx: &mut Cx<'_>, hwnd: u64, code: u64) -> Result<(), Reply> {
        let Some((parent, id)) = self.win32k().windows.get(&hwnd).map(|w| (w.parent, w.id)) else { return Ok(()) };
        if parent != super::DESKTOP {
            self.send_message(cx, parent, WM_COMMAND, code << 16 | (id & 0xffff), hwnd)?;
        }
        Ok(())
    }

    /// Asks a control's parent for its colours (`message` is the `WM_CTLCOLOR*` for it), with a DC for it: the brush it
    /// gives fills the control, and the DC's text colour, background and mode as it left them draw its text.
    pub(super) fn colors(&mut self, cx: &mut Cx<'_>, hwnd: u64, hdc: u64, message: u32, default: Rgb) -> Result<Colors, Reply> {
        let parent = self.win32k().windows.get(&hwnd).map_or(0, |w| w.parent);
        let brush = if parent != super::DESKTOP && parent != 0 {
            self.send_message(cx, parent, message, hdc, hwnd)?
        } else {
            self.def_window_proc(cx, hwnd, message, hdc, hwnd)?
        };
        let w = self.win32k();
        let background = match w.gdi.object(brush) {
            Some(gdi::Object::Brush(color)) => color,
            _ => Some(default),
        };
        let dc = w.gdi.dcs.get(&hdc).cloned();
        let (text, text_background) = match dc {
            Some(dc) => (dc.text, (dc.mode == gdi::OPAQUE).then_some(dc.background)),
            None => (0, None),
        };
        Ok(Colors { background, text, text_background })
    }

    /// Paints a control: validated, its parent asked for its colours, then `draw` with its pixels, where its client
    /// area is in them, the clip, and what it's to show.
    pub(super) fn paint_control(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, default: Rgb, draw: impl FnOnce(&mut Surface, (i32, i32), &Rect, &Colors, &PaintInfo)) -> Result<u64, Reply> {
        let info = {
            let mut w = self.win32k();
            let focus = w.focus;
            let enabled = w.enabled(hwnd);
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(0) };
            win.invalid = Rect::default();
            win.erase = false;
            let (client, text, style, ex_style) = (win.client(), win.text.clone(), win.style, win.ex_style);
            let c = w.controls.get(&hwnd).cloned().unwrap_or_default();
            PaintInfo { client, text, style, ex_style, focused: focus == hwnd, enabled, control: c }
        };
        let hdc = self.get_dc(hwnd);
        // 0: a control that keeps its colours itself, as comctl32's do.
        let colors = if message == 0 { Colors { background: Some(default), text: 0, text_background: None } } else { self.colors(cx, hwnd, hdc, message, default)? };
        self.win32k().draw(hdc, |s, origin, clip, _| draw(s, origin, clip, &colors, &info));
        self.release_dc(hdc);
        Ok(0)
    }

    pub(super) fn capture(&mut self, hwnd: u64, on: bool) {
        let shown = {
            let mut w = self.win32k();
            let shown = w.top_level_of(hwnd).and_then(|t| w.windows.get(&t)).map_or(0, |t| t.shown);
            w.capture = if on { hwnd } else { 0 };
            shown
        };
        self.desktop(|d| d.set_capture(if on && shown != 0 { Some(shown) } else { None }));
    }

    /// What's common to the controls: their font, and what a disabled one or one losing the focus needs (painting
    /// again).
    pub(super) fn control_common(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<Option<u64>, Reply> {
        Ok(Some(match message {
            WM_SETFONT => {
                self.control(hwnd, |_, c| c.font = wparam);
                if lparam != 0 {
                    self.invalidate(hwnd, None, true);
                }
                0
            }
            WM_GETFONT => self.control(hwnd, |_, c| c.font).unwrap_or(0),
            WM_ENABLE => {
                self.invalidate(hwnd, None, true);
                0
            }
            WM_SETTEXT => {
                let r = self.def_window_proc(cx, hwnd, message, wparam, lparam)?;
                self.invalidate(hwnd, None, true);
                r
            }
            WM_ERASEBKGND => 1,
            _ => return Ok(None),
        }))
    }

    // ---- Buttons ----

    fn button_type(&self, hwnd: u64) -> u32 {
        self.win32k().windows.get(&hwnd).map_or(0, |w| w.style & BS_TYPE)
    }

    /// A button pressed and let go: a check box's check changed, an automatic radio button chosen (and the others in
    /// its group not), and BN_CLICKED to its parent.
    fn click(&mut self, cx: &mut Cx<'_>, hwnd: u64) -> Result<(), Reply> {
        match self.button_type(hwnd) {
            BS_AUTOCHECKBOX => {
                self.control(hwnd, |_, c| c.check = u64::from(c.check == 0));
            }
            BS_AUTO3STATE => {
                self.control(hwnd, |_, c| c.check = (c.check + 1) % 3);
            }
            BS_AUTORADIOBUTTON => {
                for other in self.radio_group(hwnd) {
                    if other != hwnd {
                        self.control(other, |_, c| c.check = 0);
                        self.invalidate(other, None, true);
                    }
                }
                self.control(hwnd, |_, c| c.check = 1);
            }
            _ => {}
        }
        self.invalidate(hwnd, None, true);
        self.notify(cx, hwnd, BN_CLICKED)
    }

    /// The automatic radio buttons in a radio button's group: its siblings from the `WS_GROUP` one at or before it to
    /// before the next.
    fn radio_group(&self, hwnd: u64) -> Vec<u64> {
        let w = self.win32k();
        let Some(parent) = w.windows.get(&hwnd).map(|c| c.parent) else { return Vec::new() };
        let siblings: Vec<(u64, u32, bool)> = w.windows.iter().filter(|(_, c)| c.parent == parent).map(|(&h, c)| (h, c.style, c.class == "button")).collect();
        let Some(at) = siblings.iter().position(|s| s.0 == hwnd) else { return Vec::new() };
        let start = (0..=at).rev().find(|&i| siblings[i].1 & WS_GROUP != 0).unwrap_or(0);
        let end = (at + 1..siblings.len()).find(|&i| siblings[i].1 & WS_GROUP != 0).unwrap_or(siblings.len());
        siblings[start..end].iter().filter(|s| s.2 && s.1 & BS_TYPE == BS_AUTORADIOBUTTON).map(|s| s.0).collect()
    }

    pub fn button_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        if let Some(r) = self.control_common(cx, hwnd, message, wparam, lparam)? {
            return Ok(r);
        }
        let ty = self.button_type(hwnd);
        match message {
            WM_CREATE => {
                self.control(hwnd, |_, _| ());
                Ok(0)
            }
            WM_GETDLGCODE => Ok(match ty {
                BS_PUSHBUTTON | BS_PUSHBOX => DLGC_BUTTON | DLGC_UNDEFPUSHBUTTON,
                BS_DEFPUSHBUTTON => DLGC_BUTTON | DLGC_DEFPUSHBUTTON,
                BS_RADIOBUTTON | BS_AUTORADIOBUTTON => DLGC_BUTTON | DLGC_RADIOBUTTON,
                BS_GROUPBOX => DLGC_STATIC,
                _ => DLGC_BUTTON,
            }),
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if ty != BS_GROUPBOX => {
                self.set_focus(cx, hwnd)?;
                self.capture(hwnd, true);
                self.control(hwnd, |_, c| c.pressed = true);
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            WM_MOUSEMOVE if self.win32k().capture == hwnd => {
                let inside = self.win32k().windows.get(&hwnd).is_some_and(|w| w.client().contains(lparam as u16 as i16 as i32, (lparam >> 16) as u16 as i16 as i32));
                if self.control(hwnd, |_, c| std::mem::replace(&mut c.pressed, inside) != inside) == Some(true) {
                    self.invalidate(hwnd, None, true);
                }
                Ok(0)
            }
            WM_LBUTTONUP if self.win32k().capture == hwnd => {
                self.capture(hwnd, false);
                let was = self.control(hwnd, |_, c| std::mem::replace(&mut c.pressed, false)).unwrap_or(false);
                self.invalidate(hwnd, None, true);
                if was {
                    self.click(cx, hwnd)?;
                }
                Ok(0)
            }
            WM_KEYDOWN if wparam == u64::from(keys::VK_SPACE) && ty != BS_GROUPBOX => {
                self.control(hwnd, |_, c| c.pressed = true);
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            WM_KEYUP if wparam == u64::from(keys::VK_SPACE) => {
                if self.control(hwnd, |_, c| std::mem::replace(&mut c.pressed, false)) == Some(true) {
                    self.click(cx, hwnd)?;
                }
                Ok(0)
            }
            WM_CHAR if wparam == u64::from(b' ') => Ok(0),
            WM_SETFOCUS | WM_KILLFOCUS => {
                if message == WM_KILLFOCUS {
                    self.control(hwnd, |_, c| c.pressed = false);
                }
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            BM_GETCHECK => Ok(self.control(hwnd, |_, c| c.check).unwrap_or(0)),
            BM_SETCHECK => {
                let check = if matches!(ty, BS_3STATE | BS_AUTO3STATE) { wparam.min(2) } else { u64::from(wparam != 0) };
                self.control(hwnd, |_, c| c.check = check);
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            BM_GETSTATE => {
                let focus = self.win32k().focus == hwnd;
                Ok(self.control(hwnd, |_, c| c.check | if c.pressed { BST_PUSHED } else { 0 }).unwrap_or(0) | if focus { BST_FOCUS } else { 0 })
            }
            BM_SETSTATE => {
                self.control(hwnd, |_, c| c.pressed = wparam != 0);
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            BM_SETSTYLE => {
                if let Some(w) = self.win32k().windows.get_mut(&hwnd) {
                    w.style = w.style & !0xffff | wparam as u32 & 0xffff;
                }
                if lparam != 0 {
                    self.invalidate(hwnd, None, true);
                }
                Ok(0)
            }
            BM_CLICK => {
                if self.win32k().enabled(hwnd) {
                    self.click(cx, hwnd)?;
                }
                Ok(0)
            }
            BM_SETIMAGE => Ok(self.control(hwnd, |_, c| std::mem::replace(&mut c.image, lparam)).unwrap_or(0)),
            BM_GETIMAGE => Ok(self.control(hwnd, |_, c| c.image).unwrap_or(0)),
            WM_PAINT => {
                let message = if matches!(ty, BS_PUSHBUTTON | BS_DEFPUSHBUTTON | BS_PUSHBOX | BS_OWNERDRAW) { WM_CTLCOLORBTN } else { WM_CTLCOLORSTATIC };
                self.paint_control(cx, hwnd, message, gdi::sys_color(15), |s, (dx, dy), clip, colors, p| paint_button(s, (dx, dy), clip, colors, p, ty))
            }
            _ => self.def_window_proc(cx, hwnd, message, wparam, lparam),
        }
    }

    // ---- Statics ----

    pub fn static_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        if let Some(r) = self.control_common(cx, hwnd, message, wparam, lparam)? {
            return Ok(r);
        }
        match message {
            WM_GETDLGCODE => Ok(DLGC_STATIC),
            WM_LBUTTONDOWN if self.win32k().windows.get(&hwnd).is_some_and(|w| w.style & SS_NOTIFY != 0) => {
                self.notify(cx, hwnd, STN_CLICKED)?;
                Ok(0)
            }
            STM_SETICON | STM_SETIMAGE => {
                let image = if message == STM_SETICON { wparam } else { lparam };
                let old = self.control(hwnd, |_, c| std::mem::replace(&mut c.image, image)).unwrap_or(0);
                self.invalidate(hwnd, None, true);
                Ok(old)
            }
            STM_GETICON | STM_GETIMAGE => Ok(self.control(hwnd, |_, c| c.image).unwrap_or(0)),
            WM_PAINT => self.paint_control(cx, hwnd, WM_CTLCOLORSTATIC, gdi::sys_color(15), paint_static),
            _ => self.def_window_proc(cx, hwnd, message, wparam, lparam),
        }
    }

    // ---- Edits ----

    fn edit_style(&self, hwnd: u64) -> u32 {
        self.win32k().windows.get(&hwnd).map_or(0, |w| w.style)
    }

    /// The selection, start before end, and the text's characters.
    fn edit_state(&self, hwnd: u64) -> (Vec<char>, usize, usize) {
        self.control(hwnd, |w, c| {
            let chars: Vec<char> = w.text.chars().collect();
            c.caret = c.caret.min(chars.len());
            c.anchor = c.anchor.min(chars.len());
            (chars, c.caret.min(c.anchor), c.caret.max(c.anchor))
        })
        .unwrap_or_default()
    }

    /// Puts `insert` where the selection is (as much as the limit lets in), the caret after it; EN_UPDATE and EN_CHANGE
    /// if `notify`, EN_MAXTEXT if some didn't fit.
    fn edit_replace(&mut self, cx: &mut Cx<'_>, hwnd: u64, insert: &str, notify: bool) -> Result<(), Reply> {
        let (mut chars, start, end) = self.edit_state(hwnd);
        let limit = self.control(hwnd, |_, c| c.limit.unwrap_or(EDIT_LIMIT)).unwrap_or(EDIT_LIMIT);
        let room = limit.saturating_sub(chars.len() - (end - start));
        let insert: Vec<char> = insert.chars().collect();
        let fits = &insert[..insert.len().min(room)];
        if fits.is_empty() && start == end {
            if !insert.is_empty() {
                self.notify(cx, hwnd, EN_MAXTEXT)?;
            }
            return Ok(());
        }
        chars.splice(start..end, fits.iter().copied());
        let caret = start + fits.len();
        self.control(hwnd, |w, c| {
            w.text = chars.iter().collect();
            c.caret = caret;
            c.anchor = caret;
            c.modified = true;
        });
        self.invalidate(hwnd, None, true);
        if fits.len() < insert.len() {
            self.notify(cx, hwnd, EN_MAXTEXT)?;
        }
        if notify {
            self.notify(cx, hwnd, EN_UPDATE)?;
            self.notify(cx, hwnd, EN_CHANGE)?;
        }
        Ok(())
    }

    fn edit_move(&mut self, hwnd: u64, to: usize, extend: bool) {
        self.control(hwnd, |w, c| {
            let to = to.min(w.text.chars().count());
            c.caret = to;
            if !extend {
                c.anchor = to;
            }
        });
        self.invalidate(hwnd, None, true);
    }

    pub fn edit_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let style = self.edit_style(hwnd);
        let multiline = style & ES_MULTILINE != 0;
        match message {
            WM_CREATE => {
                self.control(hwnd, |_, c| {
                    c.password = (style & ES_PASSWORD != 0 && !multiline).then_some(u16::from(b'*'));
                });
                Ok(0)
            }
            WM_SETTEXT => {
                self.def_window_proc(cx, hwnd, message, wparam, lparam)?;
                self.control(hwnd, |_, c| {
                    c.caret = 0;
                    c.anchor = 0;
                    c.scroll = 0;
                    c.modified = false;
                });
                self.invalidate(hwnd, None, true);
                if !multiline {
                    self.notify(cx, hwnd, EN_CHANGE)?;
                }
                Ok(1)
            }
            WM_GETDLGCODE => Ok(DLGC_WANTCHARS | DLGC_HASSETSEL | DLGC_WANTARROWS | if multiline { DLGC_WANTALLKEYS } else { 0 }),
            WM_SETFOCUS | WM_KILLFOCUS => {
                self.invalidate(hwnd, None, true);
                self.notify(cx, hwnd, if message == WM_SETFOCUS { EN_SETFOCUS } else { EN_KILLFOCUS })?;
                Ok(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                self.set_focus(cx, hwnd)?;
                let x = i32::from(lparam as u16 as i16);
                let inset = edit_inset(style, self.win32k().windows.get(&hwnd).map_or(0, |w| w.ex_style));
                let scroll = self.control(hwnd, |_, c| c.scroll).unwrap_or(0);
                let at = scroll + ((x - inset).max(0) + CELL_WIDTH / 2) as usize / CELL_WIDTH as usize;
                self.edit_move(hwnd, at, false);
                Ok(0)
            }
            WM_CHAR => {
                let readonly = style & ES_READONLY != 0;
                match wparam as u16 {
                    // Ctrl+A: all of it.
                    1 => {
                        let n = self.edit_state(hwnd).0.len();
                        self.control(hwnd, |_, c| {
                            c.anchor = 0;
                            c.caret = n;
                        });
                        self.invalidate(hwnd, None, true);
                    }
                    8 if !readonly => {
                        let (_, start, end) = self.edit_state(hwnd);
                        if start == end && start > 0 {
                            self.control(hwnd, |_, c| c.anchor = start - 1);
                        }
                        if start != end || start > 0 {
                            self.edit_replace(cx, hwnd, "", true)?;
                        }
                    }
                    13 if multiline && !readonly => self.edit_replace(cx, hwnd, "\r\n", true)?,
                    u if u >= 0x20 && !readonly => {
                        let Some(mut c) = char::from_u32(u32::from(u)) else { return Ok(0) };
                        if style & ES_NUMBER != 0 && !c.is_ascii_digit() {
                            return Ok(0);
                        }
                        if style & ES_UPPERCASE != 0 {
                            c = c.to_uppercase().next().unwrap_or(c);
                        } else if style & ES_LOWERCASE != 0 {
                            c = c.to_lowercase().next().unwrap_or(c);
                        }
                        self.edit_replace(cx, hwnd, &c.to_string(), true)?;
                    }
                    _ => {}
                }
                Ok(0)
            }
            WM_KEYDOWN => {
                let shift = self.win32k().thread_keys(self.tid)[keys::VK_SHIFT as usize] & 0x80 != 0;
                let (chars, start, end) = self.edit_state(hwnd);
                let caret = self.control(hwnd, |_, c| c.caret).unwrap_or(0);
                match wparam as u8 {
                    keys::VK_LEFT => {
                        let to = if !shift && start != end { start } else { caret.saturating_sub(1) };
                        self.edit_move(hwnd, to, shift);
                    }
                    keys::VK_RIGHT => {
                        let to = if !shift && start != end { end } else { (caret + 1).min(chars.len()) };
                        self.edit_move(hwnd, to, shift);
                    }
                    keys::VK_HOME => self.edit_move(hwnd, 0, shift),
                    keys::VK_END => self.edit_move(hwnd, chars.len(), shift),
                    keys::VK_DELETE if style & ES_READONLY == 0 => {
                        if start == end && end < chars.len() {
                            self.control(hwnd, |_, c| c.anchor = end + 1);
                        }
                        if start != end || end < chars.len() {
                            self.edit_replace(cx, hwnd, "", true)?;
                        }
                    }
                    _ => {}
                }
                Ok(0)
            }
            EM_GETSEL => {
                let (_, start, end) = self.edit_state(hwnd);
                for (at, v) in [(wparam, start), (lparam, end)] {
                    if at != 0 {
                        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, &(v as u32).to_le_bytes()));
                    }
                }
                Ok((end.min(0xffff) as u64) << 16 | start.min(0xffff) as u64)
            }
            EM_SETSEL => {
                // -1 for the start takes the selection away; -1 (or past the end) for the end is the end.
                let n = self.edit_state(hwnd).0.len();
                let (from, to) = (wparam as i32, lparam as i32);
                self.control(hwnd, |_, c| {
                    if from < 0 {
                        c.anchor = c.caret;
                    } else {
                        let to = if to < 0 { n } else { (to as usize).min(n) };
                        c.anchor = (from as usize).min(n);
                        c.caret = to;
                    }
                });
                self.invalidate(hwnd, None, true);
                Ok(1)
            }
            EM_REPLACESEL => {
                let text = self.wstr(cx, lparam);
                self.edit_replace(cx, hwnd, &text, true)?;
                Ok(0)
            }
            EM_LIMITTEXT => {
                let limit = if wparam == 0 { 0x7fff_fffe } else { wparam as usize };
                self.control(hwnd, |_, c| c.limit = Some(limit));
                Ok(0)
            }
            EM_GETLIMITTEXT => Ok(self.control(hwnd, |_, c| c.limit.unwrap_or(EDIT_LIMIT)).unwrap_or(EDIT_LIMIT) as u64),
            EM_SETREADONLY => {
                if let Some(w) = self.win32k().windows.get_mut(&hwnd) {
                    if wparam != 0 {
                        w.style |= ES_READONLY;
                    } else {
                        w.style &= !ES_READONLY;
                    }
                }
                self.invalidate(hwnd, None, true);
                Ok(1)
            }
            EM_SETPASSWORDCHAR => {
                self.control(hwnd, |_, c| c.password = (wparam != 0).then_some(wparam as u16));
                self.invalidate(hwnd, None, true);
                Ok(0)
            }
            EM_GETPASSWORDCHAR => Ok(self.control(hwnd, |_, c| c.password.map_or(0, u64::from)).unwrap_or(0)),
            EM_GETMODIFY => Ok(u64::from(self.control(hwnd, |_, c| c.modified).unwrap_or(false))),
            EM_SETMODIFY => {
                self.control(hwnd, |_, c| c.modified = wparam != 0);
                Ok(0)
            }
            EM_GETLINECOUNT => Ok(self.win32k().windows.get(&hwnd).map_or(1, |w| w.text.split('\n').count()) as u64),
            EM_SCROLLCARET | EM_SETMARGINS => Ok(u64::from(message == EM_SCROLLCARET)),
            WM_PAINT => {
                let message = if style & ES_READONLY != 0 || !self.win32k().enabled(hwnd) { WM_CTLCOLORSTATIC } else { WM_CTLCOLOREDIT };
                let default = if message == WM_CTLCOLOREDIT { 0xffffff } else { gdi::sys_color(15) };
                self.keep_caret_in_view(hwnd);
                self.paint_control(cx, hwnd, message, default, paint_edit)
            }
            _ => match self.control_common(cx, hwnd, message, wparam, lparam)? {
                Some(r) => Ok(r),
                None => self.def_window_proc(cx, hwnd, message, wparam, lparam),
            },
        }
    }

    /// Scrolls a one-line edit so its caret is in view.
    fn keep_caret_in_view(&self, hwnd: u64) {
        self.control(hwnd, |w, c| {
            if w.style & ES_MULTILINE != 0 {
                c.scroll = 0;
                return;
            }
            let inset = edit_inset(w.style, w.ex_style);
            let shown = ((w.client().width() - 2 * inset) / CELL_WIDTH).max(1) as usize;
            if c.caret < c.scroll {
                c.scroll = c.caret;
            } else if c.caret > c.scroll + shown {
                c.scroll = c.caret - shown;
            }
        });
    }
}

/// What a control's painting needs to know of it.
pub(super) struct PaintInfo {
    pub client: Rect,
    pub text: String,
    pub style: u32,
    pub ex_style: u32,
    pub focused: bool,
    pub enabled: bool,
    pub control: Control,
}

impl PaintInfo {
    fn text_color(&self, colors: &Colors) -> Rgb {
        if self.enabled { colors.text } else { GRAY }
    }
}

/// Where an edit's text starts inside it: past its border, and a margin.
fn edit_inset(style: u32, ex_style: u32) -> i32 {
    let border = style & WS_BORDER != 0 || ex_style & (WS_EX_CLIENTEDGE | WS_EX_STATICEDGE) != 0;
    if border { 3 } else { 1 }
}

fn paint_button(s: &mut Surface, (dx, dy): (i32, i32), clip: &Rect, colors: &Colors, p: &PaintInfo, ty: u32) {
    let r = p.client.offset(dx, dy);
    let text = label(&p.text);
    let fg = p.text_color(colors);
    let width = text.chars().count() as i32 * CELL_WIDTH;
    let middle = r.top + (r.height() - CELL_HEIGHT) / 2;
    match ty {
        BS_CHECKBOX | BS_AUTOCHECKBOX | BS_3STATE | BS_AUTO3STATE | BS_RADIOBUTTON | BS_AUTORADIOBUTTON => {
            if let Some(bg) = colors.background {
                s.fill(&r, bg, clip);
            }
            let size = 13;
            let bx = r.left;
            let by = r.top + (r.height() - size) / 2;
            let edge = if p.enabled { MARK } else { GRAY };
            let inside = if p.control.pressed { FACE_PRESSED } else { 0xffffff };
            if matches!(ty, BS_RADIOBUTTON | BS_AUTORADIOBUTTON) {
                for y in 0..size {
                    for x in 0..size {
                        let d = (x - 6) * (x - 6) + (y - 6) * (y - 6);
                        let color = if d <= 6 && p.control.check != 0 {
                            Some(edge)
                        } else if d < 30 {
                            Some(inside)
                        } else if d <= 42 {
                            Some(edge)
                        } else {
                            None
                        };
                        if let Some(color) = color {
                            s.set(bx + x, by + y, color, clip);
                        }
                    }
                }
            } else {
                let b = Rect::new(bx, by, bx + size, by + size);
                s.fill(&b, inside, clip);
                s.frame(&b, edge, clip);
                match p.control.check {
                    1 => {
                        // A tick: down to the bottom, then up to the right.
                        for t in 0..2 {
                            s.line((bx + 3, by + 6 + t), (bx + 5, by + 8 + t), edge, clip);
                            s.line((bx + 5, by + 8 + t), (bx + 10, by + 3 + t), edge, clip);
                        }
                    }
                    2 => s.fill(&Rect::new(bx + 3, by + 3, bx + size - 3, by + size - 3), edge, clip),
                    _ => {}
                }
            }
            let tx = r.left + size + 4;
            s.text(tx, middle, &text, fg, colors.text_background, clip);
            if p.focused && !text.is_empty() {
                dotted(s, &Rect::new(tx - 1, middle - 1, tx + width + 1, middle + CELL_HEIGHT + 1), MARK, clip);
            }
        }
        BS_GROUPBOX => {
            let top = r.top + CELL_HEIGHT / 2;
            s.frame(&Rect::new(r.left, top, r.right, r.bottom), 0xdcdcdc, clip);
            if !text.is_empty() {
                let tx = r.left + 8;
                let back = colors.background.or(colors.text_background).unwrap_or(gdi::sys_color(15));
                s.fill(&Rect::new(tx - 2, r.top, tx + width + 2, r.top + CELL_HEIGHT), back, clip);
                s.text(tx, r.top, &text, fg, None, clip);
            }
        }
        BS_OWNERDRAW => s.fill(&r, FACE, clip),
        _ => {
            let default = ty == BS_DEFPUSHBUTTON;
            s.fill(&r, if p.control.pressed { FACE_PRESSED } else { FACE }, clip);
            let edge = if (default || p.focused) && p.enabled { ACCENT } else { EDGE };
            s.frame(&r, edge, clip);
            if default && p.enabled {
                s.frame(&Rect::new(r.left + 1, r.top + 1, r.right - 1, r.bottom - 1), edge, clip);
            }
            s.text(r.left + (r.width() - width) / 2, middle, &text, fg, None, clip);
            if p.focused {
                dotted(s, &Rect::new(r.left + 3, r.top + 3, r.right - 3, r.bottom - 3), MARK, clip);
            }
        }
    }
}

fn paint_static(s: &mut Surface, (dx, dy): (i32, i32), clip: &Rect, colors: &Colors, p: &PaintInfo) {
    let r = p.client.offset(dx, dy);
    let ty = p.style & 0x1f;
    let fill = |s: &mut Surface, color: Rgb| s.fill(&r, color, clip);
    match ty {
        4 => fill(s, 0x000000),
        5 => fill(s, gdi::sys_color(16)),
        6 => fill(s, 0xffffff),
        7 => s.frame(&r, 0x000000, clip),
        8 => s.frame(&r, gdi::sys_color(16), clip),
        9 => s.frame(&r, 0xffffff, clip),
        // Etched: a shadow line with a highlight under it (or beside it).
        0x10 => {
            s.line((r.left, r.top), (r.right, r.top), gdi::sys_color(16), clip);
            s.line((r.left, r.top + 1), (r.right, r.top + 1), 0xffffff, clip);
        }
        0x11 => {
            s.line((r.left, r.top), (r.left, r.bottom), gdi::sys_color(16), clip);
            s.line((r.left + 1, r.top), (r.left + 1, r.bottom), 0xffffff, clip);
        }
        0x12 => {
            s.frame(&r, gdi::sys_color(16), clip);
            s.frame(&Rect::new(r.left + 1, r.top + 1, r.right, r.bottom), 0xffffff, clip);
        }
        // Text: left, centred, right, one line (simple, or no wrapping), as SS_LEFT, SS_CENTER, SS_RIGHT, SS_SIMPLE
        // and SS_LEFTNOWORDWRAP say.
        0 | 1 | 2 | 0xb | 0xc => {
            if let Some(bg) = colors.background {
                fill(s, bg);
            }
            let text = if p.style & SS_NOPREFIX != 0 { p.text.clone() } else { label(&p.text) };
            let columns = (r.width() / CELL_WIDTH).max(1) as usize;
            let lines = if matches!(ty, 0xb | 0xc) { text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect() } else { wrap(&text, columns) };
            let fg = p.text_color(colors);
            for (i, line) in lines.iter().enumerate() {
                let width = line.chars().count() as i32 * CELL_WIDTH;
                let x = match ty {
                    1 => r.left + (r.width() - width) / 2,
                    2 => r.right - width,
                    _ => r.left,
                };
                s.text(x, r.top + i as i32 * CELL_HEIGHT, line, fg, colors.text_background, &clip.intersect(&r));
            }
        }
        // Icons, bitmaps, metafiles, and what its parent draws: its background.
        _ => {
            if let Some(bg) = colors.background {
                fill(s, bg);
            }
        }
    }
    if p.style & SS_SUNKEN != 0 {
        s.frame(&r, gdi::sys_color(16), clip);
    }
}

fn paint_edit(s: &mut Surface, (dx, dy): (i32, i32), clip: &Rect, colors: &Colors, p: &PaintInfo) {
    let r = p.client.offset(dx, dy);
    if let Some(bg) = colors.background {
        s.fill(&r, bg, clip);
    }
    let inset = edit_inset(p.style, p.ex_style);
    if inset > 1 {
        s.frame(&r, if p.focused { ACCENT } else { BORDER }, clip);
    }
    let inner = Rect::new(r.left + inset, r.top + 1, r.right - inset, r.bottom - 1);
    let clip = clip.intersect(&inner);
    let fg = p.text_color(colors);
    let c = &p.control;
    let shown: Vec<char> = match c.password {
        Some(mask) => std::iter::repeat_n(char::from_u32(u32::from(mask)).unwrap_or('*'), p.text.chars().count()).collect(),
        None => p.text.chars().collect(),
    };
    let (start, end) = (c.caret.min(c.anchor), c.caret.max(c.anchor));
    let multiline = p.style & ES_MULTILINE != 0;
    // Each line's characters and where they start in the text.
    let mut lines: Vec<(usize, Vec<char>)> = Vec::new();
    if multiline {
        let mut at = 0;
        let mut line = Vec::new();
        for (i, &ch) in shown.iter().enumerate() {
            if ch == '\n' {
                lines.push((at, std::mem::take(&mut line)));
                at = i + 1;
            } else if ch != '\r' {
                line.push(ch);
            }
        }
        lines.push((at, line));
    } else {
        lines.push((0, shown));
    }
    let top = if multiline { inner.top } else { r.top + (r.height() - CELL_HEIGHT) / 2 };
    for (row, (at, line)) in lines.iter().enumerate() {
        let y = top + row as i32 * CELL_HEIGHT;
        let first = if multiline { 0 } else { c.scroll };
        for (i, &ch) in line.iter().enumerate().skip(first) {
            let index = at + i;
            let x = inner.left + (i - first) as i32 * CELL_WIDTH;
            let selected = p.focused && index >= start && index < end;
            let (color, back) = if selected { (0xffffff, Some(ACCENT)) } else { (fg, colors.text_background) };
            s.text(x, y, &ch.to_string(), color, back, &clip);
        }
        // The caret, where nothing's selected.
        if p.focused && start == end && c.caret >= *at && c.caret <= at + line.len() {
            let x = inner.left + (c.caret - at).saturating_sub(first) as i32 * CELL_WIDTH;
            s.line((x, y), (x, y + CELL_HEIGHT - 1), fg, &clip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_lose_their_mnemonics() {
        assert_eq!(label("&Next >"), "Next >");
        assert_eq!(label("Fish && &Chips"), "Fish & Chips");
    }

    #[test]
    fn text_wraps_at_spaces() {
        assert_eq!(wrap("the quick brown fox", 10), ["the quick", "brown fox"]);
        assert_eq!(wrap("one\r\ntwo", 10), ["one", "two"]);
        assert_eq!(wrap("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
    }
}
