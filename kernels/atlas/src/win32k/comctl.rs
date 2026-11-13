//! comctl32's controls, whose procedures are win32k's own like the standard controls': the progress bar
//! (`msctls_progress32`) and the list view (`SysListView32`); and image lists, which hold images for them by number.
//!
//! A progress bar has a range, a position and a step, its bar's and background's colours, and is drawn smooth (as
//! comctl32 6 draws every bar), across or (`PBS_VERTICAL`) up. A list view keeps its columns and its items, each with
//! its text in every column, its state (selected, focused), image and `lParam`; it's drawn as a report (rows under a
//! header, unless `LVS_NOCOLUMNHEADER`), and its other views as one column of the same rows. Its parent hears of
//! clicks and selection changes through `WM_NOTIFY` (`NM_CLICK`, `NM_DBLCLK`, `NM_RCLICK`, `LVN_ITEMCHANGED`). An
//! image list counts its images and keeps their size; bitmaps aren't drawn yet, so neither are its images.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::font::{CELL_HEIGHT, CELL_WIDTH};
use display::{Rect, Rgb, Surface};

use super::controls::{ACCENT, BORDER, Colors, GRAY, PaintInfo, WS_BORDER, WS_EX_CLIENTEDGE};
use super::{WM_GETDLGCODE, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEWHEEL, WM_PAINT, WM_SETFOCUS, gdi, keys};
use crate::os::Reply;
use crate::process::Task;

// ---- Progress bars ----

const PBM_SETRANGE: u32 = 0x401;
const PBM_SETPOS: u32 = 0x402;
const PBM_DELTAPOS: u32 = 0x403;
const PBM_SETSTEP: u32 = 0x404;
const PBM_STEPIT: u32 = 0x405;
const PBM_SETRANGE32: u32 = 0x406;
const PBM_GETRANGE: u32 = 0x407;
const PBM_GETPOS: u32 = 0x408;
const PBM_SETBARCOLOR: u32 = 0x409;
const PBM_SETMARQUEE: u32 = 0x40a;
const PBM_GETSTEP: u32 = 0x40d;
const PBM_GETBKCOLOR: u32 = 0x40e;
const PBM_GETBARCOLOR: u32 = 0x40f;
const PBM_SETSTATE: u32 = 0x410;
const PBM_GETSTATE: u32 = 0x411;
const CCM_SETBKCOLOR: u32 = 0x2001;
const PBS_VERTICAL: u32 = 0x4;
const PBS_MARQUEE: u32 = 0x8;
/// `CLR_DEFAULT`: the control's own colour.
const CLR_DEFAULT: u64 = 0xff00_0000;
const CLR_NONE: u64 = 0xffff_ffff;

const BAR: Rgb = 0x06b025;
const TRACK: Rgb = 0xe6e6e6;
const TRACK_EDGE: Rgb = 0xbcbcbc;

#[derive(Debug, Clone)]
pub struct Progress {
    pub low: i32,
    pub high: i32,
    pub pos: i32,
    pub step: i32,
    /// COLORREFs, `CLR_DEFAULT` for the control's own.
    pub bar: u64,
    pub background: u64,
    /// PBST_NORMAL, PBST_ERROR or PBST_PAUSED.
    pub state: u64,
    /// A marquee's block, moved along by each PBM_SETMARQUEE and PBM_STEPIT.
    pub marquee: i32,
}

impl Default for Progress {
    fn default() -> Progress {
        Progress { low: 0, high: 100, pos: 0, step: 10, bar: CLR_DEFAULT, background: CLR_DEFAULT, state: 1, marquee: 0 }
    }
}

impl Progress {
    fn set_pos(&mut self, pos: i32) -> i32 {
        let old = self.pos;
        self.pos = pos.clamp(self.low.min(self.high), self.high.max(self.low));
        old
    }

    /// How much of `length` pixels the bar fills.
    fn filled(&self, length: i32) -> i32 {
        let span = i64::from(self.high) - i64::from(self.low);
        if span <= 0 {
            return 0;
        }
        (i64::from(length) * (i64::from(self.pos) - i64::from(self.low)) / span).clamp(0, i64::from(length)) as i32
    }
}

fn colorref(value: u64, default: Rgb) -> Rgb {
    if value == CLR_DEFAULT || value == CLR_NONE { default } else { gdi::swap(value as u32) }
}

fn paint_progress(s: &mut Surface, (dx, dy): (i32, i32), clip: &Rect, _: &Colors, p: &PaintInfo) {
    let r = p.client.offset(dx, dy);
    let g = &p.control.progress;
    s.fill(&r, colorref(g.background, TRACK), clip);
    s.frame(&r, TRACK_EDGE, clip);
    let inner = Rect::new(r.left + 1, r.top + 1, r.right - 1, r.bottom - 1);
    // PBST_ERROR is red, PBST_PAUSED yellow, as comctl32 6 draws them.
    let bar = match g.state {
        2 => 0xe81123,
        3 => 0xdab600,
        _ => colorref(g.bar, BAR),
    };
    let bar = if p.enabled { bar } else { GRAY };
    let block = if p.style & PBS_MARQUEE != 0 {
        let width = inner.width() / 4;
        let left = inner.left + g.marquee.rem_euclid(inner.width() + width) - width;
        Rect::new(left.max(inner.left), inner.top, (left + width).min(inner.right), inner.bottom)
    } else if p.style & PBS_VERTICAL != 0 {
        let h = g.filled(inner.height());
        Rect::new(inner.left, inner.bottom - h, inner.right, inner.bottom)
    } else {
        Rect::new(inner.left, inner.top, inner.left + g.filled(inner.width()), inner.bottom)
    };
    s.fill(&block, bar, clip);
}

// ---- List views ----

const LVM_GETBKCOLOR: u32 = 0x1000;
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_SETIMAGELIST: u32 = 0x1003;
const LVM_GETITEMCOUNT: u32 = 0x1004;
const LVM_GETITEMA: u32 = 0x1005;
const LVM_SETITEMA: u32 = 0x1006;
const LVM_INSERTITEMA: u32 = 0x1007;
const LVM_DELETEITEM: u32 = 0x1008;
const LVM_DELETEALLITEMS: u32 = 0x1009;
const LVM_GETNEXTITEM: u32 = 0x100c;
const LVM_ENSUREVISIBLE: u32 = 0x1013;
const LVM_SCROLL: u32 = 0x1014;
const LVM_REDRAWITEMS: u32 = 0x1015;
const LVM_INSERTCOLUMNA: u32 = 0x101b;
const LVM_DELETECOLUMN: u32 = 0x101c;
const LVM_GETCOLUMNWIDTH: u32 = 0x101d;
const LVM_SETCOLUMNWIDTH: u32 = 0x101e;
const LVM_GETHEADER: u32 = 0x101f;
const LVM_GETTEXTCOLOR: u32 = 0x1023;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_GETTEXTBKCOLOR: u32 = 0x1025;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_GETCOUNTPERPAGE: u32 = 0x1028;
const LVM_UPDATE: u32 = 0x102a;
const LVM_SETITEMSTATE: u32 = 0x102b;
const LVM_GETITEMSTATE: u32 = 0x102c;
const LVM_GETITEMTEXTA: u32 = 0x102d;
const LVM_SETITEMTEXTA: u32 = 0x102e;
const LVM_SETITEMCOUNT: u32 = 0x102f;
const LVM_GETSELECTEDCOUNT: u32 = 0x1032;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
const LVM_GETITEMW: u32 = 0x104b;
const LVM_SETITEMW: u32 = 0x104c;
const LVM_INSERTITEMW: u32 = 0x104d;
const LVM_GETSTRINGWIDTHW: u32 = 0x1057;
const LVM_INSERTCOLUMNW: u32 = 0x1061;
const LVM_GETITEMTEXTW: u32 = 0x1073;
const LVM_SETITEMTEXTW: u32 = 0x1074;
const LVM_GETSTRINGWIDTHA: u32 = 0x1011;

const LVIF_TEXT: u32 = 0x1;
const LVIF_IMAGE: u32 = 0x2;
const LVIF_PARAM: u32 = 0x4;
const LVIF_STATE: u32 = 0x8;
const LVCF_FMT: u32 = 0x1;
const LVCF_WIDTH: u32 = 0x2;
const LVCF_TEXT: u32 = 0x4;
const LVIS_FOCUSED: u32 = 0x1;
const LVIS_SELECTED: u32 = 0x2;
const LVNI_FOCUSED: u64 = 0x1;
const LVNI_SELECTED: u64 = 0x2;
const LVS_REPORT: u32 = 0x1;
const LVS_SINGLESEL: u32 = 0x4;
const LVS_SHOWSELALWAYS: u32 = 0x8;
const LVS_NOCOLUMNHEADER: u32 = 0x4000;
const LVS_EX_FULLROWSELECT: u32 = 0x20;
const LVSCW_AUTOSIZE: i32 = -1;
const LVSCW_AUTOSIZE_USEHEADER: i32 = -2;

const WM_NOTIFY: u32 = 0x4e;
const NM_CLICK: i32 = -2;
const NM_DBLCLK: i32 = -3;
const NM_RCLICK: i32 = -5;
const LVN_ITEMCHANGED: i32 = -101;
const LVIF_STATE_CHANGED: u32 = LVIF_STATE;
const WM_RBUTTONDOWN: u32 = 0x204;

/// A row's height, and the header's.
const ROW: i32 = CELL_HEIGHT + 2;
const HEADER: i32 = CELL_HEIGHT + 4;

#[derive(Debug, Clone, Default)]
pub struct Column {
    pub text: String,
    pub width: i32,
    pub format: i32,
}

#[derive(Debug, Clone, Default)]
pub struct Item {
    /// Its text in each column, the first its own.
    pub texts: Vec<String>,
    pub state: u32,
    pub image: i32,
    pub param: u64,
}

#[derive(Debug, Clone)]
pub struct ListView {
    pub columns: Vec<Column>,
    pub items: Vec<Item>,
    /// The first row shown.
    pub top: usize,
    pub extended: u32,
    /// Its image lists: normal, small and state.
    pub images: [u64; 3],
    /// COLORREFs, `CLR_DEFAULT` for its own.
    pub background: u64,
    pub text: u64,
    pub text_background: u64,
}

impl Default for ListView {
    fn default() -> ListView {
        ListView { columns: Vec::new(), items: Vec::new(), top: 0, extended: 0, images: [0; 3], background: CLR_DEFAULT, text: CLR_DEFAULT, text_background: CLR_DEFAULT }
    }
}

impl ListView {
    fn text(&self, item: usize, sub: usize) -> String {
        self.items.get(item).and_then(|i| i.texts.get(sub)).cloned().unwrap_or_default()
    }

    fn set_text(&mut self, item: usize, sub: usize, text: String) {
        if let Some(i) = self.items.get_mut(item) {
            if i.texts.len() <= sub {
                i.texts.resize(sub + 1, String::new());
            }
            i.texts[sub] = text;
        }
    }

    /// The widest a column's text is, its header's too if `header`, in pixels with a margin.
    fn widest(&self, column: usize, header: bool) -> i32 {
        let items = self.items.iter().map(|i| i.texts.get(column).map_or(0, |t| t.chars().count()));
        let header = if header { self.columns.get(column).map_or(0, |c| c.text.chars().count()) } else { 0 };
        (items.chain([header]).max().unwrap_or(0) as i32) * CELL_WIDTH + 12
    }
}

/// Where a list view's rows start, below its border and header.
fn rows_top(style: u32, ex_style: u32) -> i32 {
    border(style, ex_style) + if style & 3 == LVS_REPORT && style & LVS_NOCOLUMNHEADER == 0 { HEADER } else { 0 }
}

fn border(style: u32, ex_style: u32) -> i32 {
    i32::from(style & WS_BORDER != 0 || ex_style & WS_EX_CLIENTEDGE != 0)
}

/// How many whole rows fit.
fn per_page(client: &Rect, style: u32, ex_style: u32) -> usize {
    ((client.height() - rows_top(style, ex_style) - border(style, ex_style)) / ROW).max(1) as usize
}

fn paint_list(s: &mut Surface, (dx, dy): (i32, i32), clip: &Rect, _: &Colors, p: &PaintInfo) {
    let r = p.client.offset(dx, dy);
    let l = &p.control.list;
    let background = colorref(l.background, 0xffffff);
    s.fill(&r, background, clip);
    let b = border(p.style, p.ex_style);
    if b != 0 {
        s.frame(&r, if p.focused { ACCENT } else { BORDER }, clip);
    }
    let inner = Rect::new(r.left + b, r.top + b, r.right - b, r.bottom - b);
    let clip = clip.intersect(&inner);
    let report = p.style & 3 == LVS_REPORT;
    // Report view's columns; the other views are one column of the items' own text.
    let columns: Vec<(i32, i32)> = if report && !l.columns.is_empty() {
        let mut x = inner.left;
        l.columns.iter().map(|c| {
            let at = (x, c.width);
            x += c.width;
            at
        }).collect()
    } else {
        vec![(inner.left, inner.width())]
    };
    if report && p.style & LVS_NOCOLUMNHEADER == 0 {
        let header = Rect::new(inner.left, inner.top, inner.right, inner.top + HEADER);
        s.fill(&header, 0xffffff, &clip);
        s.line((inner.left, header.bottom - 1), (inner.right, header.bottom - 1), 0xe5e5e5, &clip);
        for (i, &(x, w)) in columns.iter().enumerate() {
            let title = l.columns.get(i).map(|c| c.text.as_str()).unwrap_or("");
            let cell = clip.intersect(&Rect::new(x, header.top, x + w - 1, header.bottom));
            s.text(x + 6, header.top + 2, title, 0x000000, None, &cell);
            s.line((x + w - 1, header.top), (x + w - 1, header.bottom - 1), 0xe5e5e5, &clip);
        }
    }
    let top = inner.top + rows_top(p.style, p.ex_style) - b;
    let text = if p.enabled { colorref(l.text, 0x000000) } else { GRAY };
    let text_background = (l.text_background != CLR_NONE && l.text_background != CLR_DEFAULT).then(|| gdi::swap(l.text_background as u32));
    let full_row = l.extended & LVS_EX_FULLROWSELECT != 0 || !report;
    for (row, item) in l.items.iter().enumerate().skip(l.top) {
        let y = top + (row - l.top) as i32 * ROW;
        if y >= inner.bottom {
            break;
        }
        let selected = item.state & LVIS_SELECTED != 0 && (p.focused || p.style & LVS_SHOWSELALWAYS != 0);
        let (highlight, fg) = if !selected {
            (None, text)
        } else if p.focused {
            (Some(ACCENT), 0xffffff)
        } else {
            (Some(0xd9d9d9), text)
        };
        for (i, &(x, w)) in columns.iter().enumerate() {
            let cell = Rect::new(x, y, x + w, y + ROW);
            let marked = highlight.filter(|_| i == 0 || full_row);
            if let Some(h) = marked {
                s.fill(&cell, h, &clip);
            } else if let Some(bg) = text_background {
                s.fill(&cell, bg, &clip);
            }
            let words = item.texts.get(i).map(String::as_str).unwrap_or("");
            let color = if marked.is_some() { fg } else { text };
            // LVCFMT_RIGHT and LVCFMT_CENTER, but for the first column, which is always on the left.
            let used = words.chars().count() as i32 * CELL_WIDTH;
            let tx = match l.columns.get(i).map_or(0, |c| c.format & 3) {
                1 if i > 0 => x + w - 6 - used,
                2 if i > 0 => x + (w - used) / 2,
                _ => x + 2 + if i == 0 { 2 } else { 4 },
            };
            s.text(tx, y + 1, words, color, None, &clip.intersect(&Rect::new(x, y, x + w - 2, y + ROW)));
        }
        if item.state & LVIS_FOCUSED != 0 && p.focused {
            super::controls::dotted(s, &Rect::new(inner.left, y, columns.last().map_or(inner.right, |&(x, w)| (x + w).min(inner.right)), y + ROW), if selected { 0xffffff } else { 0x000000 }, &clip);
        }
    }
}

/// An image list: how big its images are and how many it has.
#[derive(Debug, Clone, Default)]
pub struct ImageList {
    pub width: i32,
    pub height: i32,
    pub count: i32,
    pub background: u64,
}

/// Image lists' handles start here.
const IMAGE_LISTS: u64 = 0x0060_0000;

impl Task {
    // ---- Progress bars ----

    pub fn progress_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        if let Some(r) = self.control_common(cx, hwnd, message, wparam, lparam)? {
            return Ok(r);
        }
        let update = |task: &mut Task, f: &mut dyn FnMut(&mut Progress) -> u64| -> u64 {
            let r = task.control(hwnd, |_, c| f(&mut c.progress)).unwrap_or(0);
            task.invalidate(hwnd, None, false);
            r
        };
        let word = |v: u64| v as u16 as i16 as i32;
        Ok(match message {
            PBM_SETRANGE => update(self, &mut |g| {
                let old = (g.high as u16 as u64) << 16 | g.low as u16 as u64;
                (g.low, g.high) = (word(lparam), word(lparam >> 16));
                g.set_pos(g.pos);
                old
            }),
            PBM_SETRANGE32 => update(self, &mut |g| {
                let old = (g.high as u16 as u64) << 16 | g.low as u16 as u64;
                (g.low, g.high) = (wparam as i32, lparam as i32);
                g.set_pos(g.pos);
                old
            }),
            PBM_GETRANGE => {
                let g = self.control(hwnd, |_, c| c.progress.clone()).unwrap_or_default();
                if lparam != 0 {
                    let bytes: Vec<u8> = [g.low, g.high].iter().flat_map(|v| v.to_le_bytes()).collect();
                    let _ = self.mem(cx, true, |t| user::write(&mut t.space(), lparam, &bytes));
                }
                (if wparam != 0 { g.low } else { g.high }) as u32 as u64
            }
            PBM_SETPOS => update(self, &mut |g| g.set_pos(wparam as i32) as u32 as u64),
            PBM_DELTAPOS => update(self, &mut |g| g.set_pos(g.pos.saturating_add(wparam as i32)) as u32 as u64),
            PBM_SETSTEP => update(self, &mut |g| std::mem::replace(&mut g.step, wparam as i32) as u32 as u64),
            PBM_GETSTEP => self.control(hwnd, |_, c| c.progress.step as u32 as u64).unwrap_or(0),
            // Past the top it starts again from the bottom, as comctl32's does.
            PBM_STEPIT => update(self, &mut |g| {
                let old = g.pos;
                let next = g.pos.saturating_add(g.step);
                g.pos = if next > g.high { g.low + (next - g.high) } else { next.max(g.low) };
                g.marquee += 8;
                old as u32 as u64
            }),
            PBM_GETPOS => self.control(hwnd, |_, c| c.progress.pos as u32 as u64).unwrap_or(0),
            PBM_SETBARCOLOR => update(self, &mut |g| std::mem::replace(&mut g.bar, lparam & 0xffff_ffff)),
            CCM_SETBKCOLOR => update(self, &mut |g| std::mem::replace(&mut g.background, lparam & 0xffff_ffff)),
            PBM_GETBARCOLOR => self.control(hwnd, |_, c| c.progress.bar).unwrap_or(CLR_DEFAULT),
            PBM_GETBKCOLOR => self.control(hwnd, |_, c| c.progress.background).unwrap_or(CLR_DEFAULT),
            PBM_SETSTATE => update(self, &mut |g| std::mem::replace(&mut g.state, wparam)),
            PBM_GETSTATE => self.control(hwnd, |_, c| c.progress.state).unwrap_or(1),
            PBM_SETMARQUEE => update(self, &mut |g| {
                g.marquee += 8;
                1
            }),
            WM_PAINT => self.paint_control(cx, hwnd, 0, TRACK, paint_progress)?,
            _ => self.def_window_proc(cx, hwnd, message, wparam, lparam)?,
        })
    }

    // ---- List views ----

    fn list<R>(&self, hwnd: u64, f: impl FnOnce(&mut ListView) -> R) -> Option<R> {
        self.control(hwnd, |_, c| f(&mut c.list))
    }

    /// A list view's text argument: UTF-16 or ANSI, and `LPSTR_TEXTCALLBACK` (-1) as none.
    fn list_text(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> String {
        let width = self.abi().width;
        let callback = if width == 4 { 0xffff_ffff } else { u64::MAX };
        if at == 0 || at == callback { String::new() } else { self.text_arg(cx, at, wide) }
    }

    /// An LVITEM: mask, iItem, iSubItem, state, stateMask, pszText, cchTextMax, iImage, lParam; with where its text
    /// pointer, its room and its lParam are.
    fn read_lvitem(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<LvItem> {
        let abi = self.abi();
        let w = abi.width as usize;
        let text = (20usize).next_multiple_of(w);
        let param = (text + w + 8).next_multiple_of(w);
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, param + w)).ok()?;
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
        Some(LvItem {
            mask: u(0),
            item: u(4) as i32,
            sub: u(8) as i32,
            state: u(12),
            state_mask: u(16),
            text: abi.ptr_at(&b, text),
            room: u(text + w) as i32,
            image: u(text + w + 4) as i32,
            param: abi.ptr_at(&b, param),
            offsets: (text, param),
        })
    }

    /// Writes text into a program's buffer of `room` characters, cut and terminated; the characters written.
    fn put_list_text(&mut self, cx: &mut Cx<'_>, at: u64, room: i32, text: &str, wide: bool) -> u64 {
        if at == 0 || room <= 0 {
            return 0;
        }
        let room = room as usize;
        let bytes: Vec<u8>;
        let n;
        if wide {
            let units: Vec<u16> = text.encode_utf16().take(room - 1).collect();
            n = units.len();
            bytes = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
        } else {
            let ansi: Vec<u8> = crate::ansi::bytes(text).into_iter().take(room - 1).collect();
            n = ansi.len();
            bytes = ansi.into_iter().chain([0]).collect();
        }
        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes));
        n as u64
    }

    /// `WM_NOTIFY` to a list view's parent: NMHDR, then (for `item`) NMLISTVIEW's (and NMITEMACTIVATE's) fields.
    fn list_notify(&mut self, cx: &mut Cx<'_>, hwnd: u64, code: i32, item: Option<(i32, u32, u32, u32, u64)>) -> Result<u64, Reply> {
        let Some((parent, id)) = self.win32k().windows.get(&hwnd).map(|w| (w.parent, w.id)) else { return Ok(0) };
        if parent == super::DESKTOP {
            return Ok(0);
        }
        let abi = self.abi();
        let w = abi.width as usize;
        let head = 3 * w;
        let param = (head + 28).next_multiple_of(w);
        let size = param + w + 4;
        let mut b = vec![0u8; size.next_multiple_of(w)];
        b[0..w].copy_from_slice(&abi.ptr(hwnd));
        b[w..2 * w].copy_from_slice(&abi.ptr(id));
        b[2 * w..2 * w + 4].copy_from_slice(&code.to_le_bytes());
        if let Some((index, new, old, changed, lparam)) = item {
            for (o, v) in [(0, index as u32), (4, 0), (8, new), (12, old), (16, changed)] {
                b[head + o..head + o + 4].copy_from_slice(&v.to_le_bytes());
            }
            b[param..param + w].copy_from_slice(&abi.ptr(lparam));
        } else {
            b[head..head + 4].copy_from_slice(&(-1i32).to_le_bytes());
        }
        let len = b.len();
        self.with_scratch(cx, len, |_| b.clone(), |task, cx, at| task.send_message(cx, parent, WM_NOTIFY, id, at))
    }

    /// Sets items' state: `item` (or all, for -1), the `mask` bits of it to `state`'s; LVN_ITEMCHANGED for each one
    /// that changed. Single selection keeps one selected.
    fn set_item_state(&mut self, cx: &mut Cx<'_>, hwnd: u64, item: i64, state: u32, mask: u32) -> Result<bool, Reply> {
        let single = self.win32k().windows.get(&hwnd).is_some_and(|w| w.style & LVS_SINGLESEL != 0);
        let changes = self
            .list(hwnd, |l| {
                let n = l.items.len();
                let targets: Vec<usize> = if item < 0 { (0..n).collect() } else if (item as usize) < n { vec![item as usize] } else { return None };
                let mut changes = Vec::new();
                // Selecting one in a single-selection list, or focusing one, takes it from the others.
                let taken = (if single { state & mask & LVIS_SELECTED } else { 0 }) | (state & mask & LVIS_FOCUSED);
                if taken != 0 && item >= 0 {
                    for (i, it) in l.items.iter_mut().enumerate() {
                        if i as i64 != item && it.state & taken != 0 {
                            let old = it.state;
                            it.state &= !taken;
                            changes.push((i, it.state, old, it.param));
                        }
                    }
                }
                for i in targets {
                    let it = &mut l.items[i];
                    let old = it.state;
                    it.state = it.state & !mask | state & mask;
                    if it.state != old {
                        changes.push((i, it.state, old, it.param));
                    }
                }
                Some(changes)
            })
            .flatten();
        let Some(changes) = changes else { return Ok(false) };
        if !changes.is_empty() {
            self.invalidate(hwnd, None, true);
        }
        for (i, new, old, param) in changes {
            self.list_notify(cx, hwnd, LVN_ITEMCHANGED, Some((i as i32, new, old, LVIF_STATE_CHANGED, param)))?;
        }
        Ok(true)
    }

    /// Scrolls so `item` is in view.
    fn ensure_visible(&mut self, hwnd: u64, item: usize) {
        let Some((client, style, ex_style)) = self.win32k().windows.get(&hwnd).map(|w| (w.client(), w.style, w.ex_style)) else { return };
        let page = per_page(&client, style, ex_style);
        self.list(hwnd, |l| {
            if item < l.top {
                l.top = item;
            } else if item >= l.top + page {
                l.top = item + 1 - page;
            }
        });
        self.invalidate(hwnd, None, true);
    }

    /// The row at a point in the client area, if there's an item there.
    fn row_at(&self, hwnd: u64, y: i32) -> Option<usize> {
        let (style, ex_style) = self.win32k().windows.get(&hwnd).map(|w| (w.style, w.ex_style))?;
        let y = y - rows_top(style, ex_style);
        if y < 0 {
            return None;
        }
        self.list(hwnd, |l| {
            let row = l.top + (y / ROW) as usize;
            (row < l.items.len()).then_some(row)
        })
        .flatten()
    }

    pub fn list_view_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        if let Some(r) = self.control_common(cx, hwnd, message, wparam, lparam)? {
            return Ok(r);
        }
        let wide = matches!(message, LVM_INSERTITEMW | LVM_SETITEMW | LVM_GETITEMW | LVM_SETITEMTEXTW | LVM_GETITEMTEXTW | LVM_INSERTCOLUMNW | LVM_GETSTRINGWIDTHW);
        Ok(match message {
            LVM_GETITEMCOUNT => self.list(hwnd, |l| l.items.len() as u64).unwrap_or(0),
            LVM_SETITEMCOUNT | LVM_REDRAWITEMS | LVM_UPDATE => {
                self.invalidate(hwnd, None, true);
                1
            }
            LVM_INSERTITEMW | LVM_INSERTITEMA => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(u64::MAX) };
                let text = if it.mask & LVIF_TEXT != 0 { self.list_text(cx, it.text, wide) } else { String::new() };
                let item = Item {
                    texts: vec![text],
                    state: if it.mask & LVIF_STATE != 0 { it.state & it.state_mask } else { 0 },
                    image: if it.mask & LVIF_IMAGE != 0 { it.image } else { 0 },
                    param: if it.mask & LVIF_PARAM != 0 { it.param } else { 0 },
                };
                let at = self.list(hwnd, |l| {
                    let at = (it.item.max(0) as usize).min(l.items.len());
                    l.items.insert(at, item);
                    at
                });
                self.invalidate(hwnd, None, true);
                at.map_or(u64::MAX, |a| a as u64)
            }
            LVM_SETITEMW | LVM_SETITEMA => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(0) };
                let (item, sub) = (it.item.max(0) as usize, it.sub.max(0) as usize);
                if self.list(hwnd, |l| item < l.items.len()) != Some(true) {
                    return Ok(0);
                }
                if it.mask & LVIF_TEXT != 0 {
                    let text = self.list_text(cx, it.text, wide);
                    self.list(hwnd, |l| l.set_text(item, sub, text));
                }
                self.list(hwnd, |l| {
                    let i = &mut l.items[item];
                    if it.mask & LVIF_IMAGE != 0 && sub == 0 {
                        i.image = it.image;
                    }
                    if it.mask & LVIF_PARAM != 0 && sub == 0 {
                        i.param = it.param;
                    }
                });
                if it.mask & LVIF_STATE != 0 && sub == 0 {
                    self.set_item_state(cx, hwnd, item as i64, it.state, it.state_mask)?;
                }
                self.invalidate(hwnd, None, true);
                1
            }
            LVM_GETITEMW | LVM_GETITEMA => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(0) };
                let (item, sub) = (it.item.max(0) as usize, it.sub.max(0) as usize);
                let Some(found) = self.list(hwnd, |l| l.items.get(item).cloned()).flatten() else { return Ok(0) };
                if it.mask & LVIF_TEXT != 0 {
                    let text = found.texts.get(sub).cloned().unwrap_or_default();
                    self.put_list_text(cx, it.text, it.room, &text, wide);
                }
                let abi = self.abi();
                let w = abi.width as usize;
                let (text_at, param_at) = it.offsets;
                let mut writes: Vec<(u64, Vec<u8>)> = Vec::new();
                if it.mask & LVIF_STATE != 0 {
                    writes.push((lparam + 12, (found.state & it.state_mask).to_le_bytes().to_vec()));
                }
                if it.mask & LVIF_IMAGE != 0 {
                    writes.push((lparam + (text_at + w + 4) as u64, found.image.to_le_bytes().to_vec()));
                }
                if it.mask & LVIF_PARAM != 0 {
                    writes.push((lparam + param_at as u64, abi.ptr(found.param).to_vec()));
                }
                for (at, bytes) in writes {
                    let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes));
                }
                1
            }
            LVM_SETITEMTEXTW | LVM_SETITEMTEXTA => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(0) };
                let text = self.list_text(cx, it.text, wide);
                let done = self.list(hwnd, |l| {
                    let ok = (wparam as usize) < l.items.len();
                    l.set_text(wparam as usize, it.sub.max(0) as usize, text);
                    ok
                });
                self.invalidate(hwnd, None, true);
                u64::from(done == Some(true))
            }
            LVM_GETITEMTEXTW | LVM_GETITEMTEXTA => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(0) };
                let text = self.list(hwnd, |l| l.text(wparam as usize, it.sub.max(0) as usize)).unwrap_or_default();
                self.put_list_text(cx, it.text, it.room, &text, wide)
            }
            LVM_DELETEITEM => {
                let done = self.list(hwnd, |l| {
                    let i = wparam as usize;
                    if i < l.items.len() {
                        l.items.remove(i);
                        l.top = l.top.min(l.items.len().saturating_sub(1));
                        true
                    } else {
                        false
                    }
                });
                self.invalidate(hwnd, None, true);
                u64::from(done == Some(true))
            }
            LVM_DELETEALLITEMS => {
                self.list(hwnd, |l| {
                    l.items.clear();
                    l.top = 0;
                });
                self.invalidate(hwnd, None, true);
                1
            }
            LVM_INSERTCOLUMNW | LVM_INSERTCOLUMNA => {
                // LVCOLUMN: mask, fmt, cx, pszText, cchTextMax, iSubItem.
                let abi = self.abi();
                let w = abi.width as usize;
                let text_at = 12usize.next_multiple_of(w);
                let Ok(b) = self.mem(cx, false, |t| user::read_vec(&t.space(), lparam, text_at + w)) else { return Ok(u64::MAX) };
                let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap_or_default());
                let mask = u(0);
                let text = if mask & LVCF_TEXT != 0 { self.list_text(cx, abi.ptr_at(&b, text_at), wide) } else { String::new() };
                let column = Column {
                    width: if mask & LVCF_WIDTH != 0 { u(8) as i32 } else { 50 },
                    format: if mask & LVCF_FMT != 0 { u(4) as i32 } else { 0 },
                    text,
                };
                let at = self.list(hwnd, |l| {
                    let at = (wparam as usize).min(l.columns.len());
                    l.columns.insert(at, column);
                    // The items' texts move with their columns, but the first's, which is each item's own.
                    if at > 0 {
                        for i in &mut l.items {
                            if i.texts.len() > at {
                                i.texts.insert(at, String::new());
                            }
                        }
                    }
                    at
                });
                self.invalidate(hwnd, None, true);
                at.map_or(u64::MAX, |a| a as u64)
            }
            LVM_DELETECOLUMN => {
                let done = self.list(hwnd, |l| {
                    let at = wparam as usize;
                    if at >= l.columns.len() {
                        return false;
                    }
                    l.columns.remove(at);
                    for i in &mut l.items {
                        if i.texts.len() > at && at > 0 {
                            i.texts.remove(at);
                        }
                    }
                    true
                });
                self.invalidate(hwnd, None, true);
                u64::from(done == Some(true))
            }
            LVM_SETCOLUMNWIDTH => {
                let client = self.win32k().windows.get(&hwnd).map(|w| w.client().width()).unwrap_or(0);
                let done = self.list(hwnd, |l| {
                    let at = wparam as usize;
                    // A list without columns (the list view): its one column.
                    if l.columns.is_empty() && at == 0 {
                        l.columns.push(Column::default());
                    }
                    let width = match lparam as i32 {
                        LVSCW_AUTOSIZE => l.widest(at, false),
                        // The last column takes what's left.
                        LVSCW_AUTOSIZE_USEHEADER if at + 1 == l.columns.len() => {
                            let before: i32 = l.columns[..at].iter().map(|c| c.width).sum();
                            (client - before).max(l.widest(at, true))
                        }
                        LVSCW_AUTOSIZE_USEHEADER => l.widest(at, true),
                        w => w,
                    };
                    match l.columns.get_mut(at) {
                        Some(c) => {
                            c.width = width;
                            true
                        }
                        None => false,
                    }
                });
                self.invalidate(hwnd, None, true);
                u64::from(done == Some(true))
            }
            LVM_GETCOLUMNWIDTH => self.list(hwnd, |l| l.columns.get(wparam as usize).map_or(0, |c| c.width as u32 as u64)).unwrap_or(0),
            LVM_GETHEADER => 0,
            LVM_ENSUREVISIBLE => {
                let n = self.list(hwnd, |l| l.items.len()).unwrap_or(0);
                if (wparam as usize) < n {
                    self.ensure_visible(hwnd, wparam as usize);
                }
                u64::from((wparam as usize) < n)
            }
            LVM_SCROLL => {
                let rows = (lparam as i32) / ROW;
                self.list(hwnd, |l| {
                    let last = l.items.len().saturating_sub(1) as i64;
                    l.top = (l.top as i64 + i64::from(rows)).clamp(0, last.max(0)) as usize;
                });
                self.invalidate(hwnd, None, true);
                1
            }
            LVM_GETTOPINDEX => self.list(hwnd, |l| l.top as u64).unwrap_or(0),
            LVM_GETCOUNTPERPAGE => self.win32k().windows.get(&hwnd).map_or(0, |w| per_page(&w.client(), w.style, w.ex_style) as u64),
            LVM_SETEXTENDEDLISTVIEWSTYLE => {
                let mask = if wparam == 0 { u32::MAX } else { wparam as u32 };
                let old = self.list(hwnd, |l| {
                    let old = l.extended;
                    l.extended = l.extended & !mask | lparam as u32 & mask;
                    old
                });
                self.invalidate(hwnd, None, true);
                u64::from(old.unwrap_or(0))
            }
            LVM_GETEXTENDEDLISTVIEWSTYLE => self.list(hwnd, |l| u64::from(l.extended)).unwrap_or(0),
            LVM_SETIMAGELIST => self.list(hwnd, |l| l.images.get_mut(wparam as usize).map_or(0, |slot| std::mem::replace(slot, lparam))).unwrap_or(0),
            LVM_SETBKCOLOR | LVM_SETTEXTCOLOR | LVM_SETTEXTBKCOLOR => {
                self.list(hwnd, |l| {
                    let slot = match message {
                        LVM_SETBKCOLOR => &mut l.background,
                        LVM_SETTEXTCOLOR => &mut l.text,
                        _ => &mut l.text_background,
                    };
                    *slot = lparam & 0xffff_ffff;
                });
                self.invalidate(hwnd, None, true);
                1
            }
            LVM_GETBKCOLOR => self.list(hwnd, |l| colorref_value(l.background, 0xffffff)).unwrap_or(0),
            LVM_GETTEXTCOLOR => self.list(hwnd, |l| colorref_value(l.text, 0)).unwrap_or(0),
            LVM_GETTEXTBKCOLOR => self.list(hwnd, |l| l.text_background).unwrap_or(CLR_NONE),
            LVM_GETNEXTITEM => {
                let start = wparam as i32;
                let flags = lparam & 0xffff;
                self.list(hwnd, |l| {
                    let want = (if flags & LVNI_SELECTED != 0 { LVIS_SELECTED } else { 0 }) | (if flags & LVNI_FOCUSED != 0 { LVIS_FOCUSED } else { 0 });
                    let from = if start < 0 { 0 } else { start as usize + 1 };
                    l.items.iter().enumerate().skip(from).find(|(_, i)| i.state & want == want).map_or(u64::MAX, |(i, _)| i as u64)
                })
                .unwrap_or(u64::MAX)
            }
            LVM_GETSELECTEDCOUNT => self.list(hwnd, |l| l.items.iter().filter(|i| i.state & LVIS_SELECTED != 0).count() as u64).unwrap_or(0),
            LVM_SETITEMSTATE => {
                let Some(it) = self.read_lvitem(cx, lparam) else { return Ok(0) };
                u64::from(self.set_item_state(cx, hwnd, wparam as i32 as i64, it.state, it.state_mask)?)
            }
            LVM_GETITEMSTATE => self.list(hwnd, |l| l.items.get(wparam as usize).map_or(0, |i| u64::from(i.state) & lparam)).unwrap_or(0),
            LVM_GETSTRINGWIDTHW | LVM_GETSTRINGWIDTHA => (self.text_arg(cx, lparam, wide).chars().count() as i32 * CELL_WIDTH) as u64,
            WM_GETDLGCODE => super::dialog::DLGC_WANTARROWS | super::dialog::DLGC_WANTCHARS,
            WM_SETFOCUS | super::WM_KILLFOCUS => {
                self.invalidate(hwnd, None, true);
                0
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN => {
                self.set_focus(cx, hwnd)?;
                let row = self.row_at(hwnd, (lparam >> 16) as u16 as i16 as i32);
                if let Some(row) = row {
                    self.set_item_state(cx, hwnd, row as i64, LVIS_SELECTED | LVIS_FOCUSED, LVIS_SELECTED | LVIS_FOCUSED)?;
                }
                let code = match message {
                    WM_LBUTTONDOWN => NM_CLICK,
                    WM_LBUTTONDBLCLK => NM_DBLCLK,
                    _ => NM_RCLICK,
                };
                let param = row.and_then(|r| self.list(hwnd, |l| l.items[r].param)).unwrap_or(0);
                self.list_notify(cx, hwnd, code, Some((row.map_or(-1, |r| r as i32), 0, 0, 0, param)))?;
                0
            }
            WM_KEYDOWN => {
                let (n, focused) = self.list(hwnd, |l| (l.items.len(), l.items.iter().position(|i| i.state & LVIS_FOCUSED != 0))).unwrap_or((0, None));
                let page = self.win32k().windows.get(&hwnd).map_or(1, |w| per_page(&w.client(), w.style, w.ex_style));
                let at = focused.unwrap_or(0);
                let to = match wparam as u8 {
                    keys::VK_UP => Some(at.saturating_sub(1)),
                    keys::VK_DOWN => Some((at + 1).min(n.saturating_sub(1))),
                    keys::VK_HOME => Some(0),
                    keys::VK_END => Some(n.saturating_sub(1)),
                    0x21 => Some(at.saturating_sub(page)),
                    0x22 => Some((at + page).min(n.saturating_sub(1))),
                    _ => None,
                };
                if let Some(to) = to.filter(|_| n > 0) {
                    self.set_item_state(cx, hwnd, to as i64, LVIS_SELECTED | LVIS_FOCUSED, LVIS_SELECTED | LVIS_FOCUSED)?;
                    self.ensure_visible(hwnd, to);
                }
                0
            }
            WM_MOUSEWHEEL => {
                // Three rows a notch.
                let notches = i32::from((wparam >> 16) as u16 as i16) / 120;
                self.list(hwnd, |l| {
                    let last = l.items.len().saturating_sub(1) as i64;
                    l.top = (l.top as i64 - i64::from(notches) * 3).clamp(0, last.max(0)) as usize;
                });
                self.invalidate(hwnd, None, true);
                0
            }
            WM_PAINT => {
                let background = self.list(hwnd, |l| colorref(l.background, 0xffffff)).unwrap_or(0xffffff);
                self.paint_control(cx, hwnd, 0, background, paint_list)?
            }
            _ => self.def_window_proc(cx, hwnd, message, wparam, lparam)?,
        })
    }

    // ---- Image lists ----

    /// `ImageList_Create(cx, cy, flags, initial, grow)`.
    pub fn image_list_create(&mut self, width: i32, height: i32) -> u64 {
        let mut w = self.win32k();
        let handle = IMAGE_LISTS + 4 * (w.image_lists.keys().next_back().map_or(0, |h| (h - IMAGE_LISTS) / 4) + 1);
        w.image_lists.insert(handle, ImageList { width, height, count: 0, background: CLR_NONE });
        handle
    }

    /// Adds `n` images: the index of the first, or -1 for a list that isn't there.
    pub fn image_list_add(&mut self, handle: u64, n: i32) -> u64 {
        match self.win32k().image_lists.get_mut(&handle) {
            Some(l) => {
                let first = l.count;
                l.count += n;
                first as u32 as u64
            }
            None => u64::from(u32::MAX),
        }
    }
}

/// A list view's colour as it gives it: its own as a COLORREF.
fn colorref_value(value: u64, default: Rgb) -> u64 {
    if value == CLR_DEFAULT { u64::from(gdi::swap(default)) } else { value }
}

/// An LVITEM as read, with the offsets of its text pointer and lParam.
struct LvItem {
    mask: u32,
    item: i32,
    sub: i32,
    state: u32,
    state_mask: u32,
    text: u64,
    room: i32,
    image: i32,
    param: u64,
    offsets: (usize, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_fills_by_its_range() {
        let mut g = Progress { low: 0, high: 200, ..Progress::default() };
        g.set_pos(50);
        assert_eq!(g.filled(100), 25);
        assert_eq!(g.set_pos(500), 50);
        assert_eq!(g.pos, 200);
        let empty = Progress { low: 5, high: 5, ..Progress::default() };
        assert_eq!(empty.filled(100), 0);
    }

    #[test]
    fn list_texts_by_column() {
        let mut l = ListView { items: vec![Item::default()], ..ListView::default() };
        l.set_text(0, 2, "third".into());
        assert_eq!(l.text(0, 2), "third");
        assert_eq!(l.text(0, 1), "");
        assert_eq!(l.text(3, 0), "");
        assert_eq!(l.widest(2, false), 5 * CELL_WIDTH + 12);
    }
}
