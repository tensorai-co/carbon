//! Menus: menu objects and popup menus on the desktop.
//!
//! A menu is a list of items, each with its text (an accelerator after a tab), its ID or the submenu it opens, and its
//! state (greyed, disabled, checked, a radio check, the default); separators among them. Programs build them
//! (`CreateMenu`, `AppendMenu`, `InsertMenuItem`, …) or load them from `MENU` and `MENUEX` resources. A window's
//! menu (`SetMenu`) is kept for it; menu bars aren't drawn yet, as frames are the desktop's.
//!
//! `TrackPopupMenu` shows one as a popup of the desktop's that grabs the keyboard and pointer, placed at a point and
//! kept on the screen by the desktop's positioner, and runs the menu loop on the calling thread until an item is
//! chosen or it's dismissed: the pointer lights items up and opens submenus as popups of their own, a click chooses,
//! the arrows move and open and close submenus, Enter chooses, Escape closes a level, an item's `&` letter chooses
//! it, and a press outside dismisses them all.

use display::desktop::{self as dt, Event, Pointer, Role, Spec};
use display::font::{CELL_HEIGHT, CELL_WIDTH};
use display::positioner::{Edge, Positioner, adjust};
use display::{Rect, Surface};

use super::controls::label;
use super::{Win32k, keys};

pub const MF_GRAYED: u32 = 0x1;
pub const MF_DISABLED: u32 = 0x2;
pub const MF_BITMAP: u32 = 0x4;
pub const MF_CHECKED: u32 = 0x8;
pub const MF_POPUP: u32 = 0x10;
pub const MF_OWNERDRAW: u32 = 0x100;
pub const MF_BYPOSITION: u32 = 0x400;
pub const MF_SEPARATOR: u32 = 0x800;
pub const MF_DEFAULT: u32 = 0x1000;
/// MFT_RADIOCHECK: a check drawn as a dot.
pub const MF_RADIOCHECK: u32 = 0x200;

/// Menus' handles start here.
const MENUS: u64 = 0x0070_0000;
const ITEM_HEIGHT: i32 = CELL_HEIGHT + 7;
const SEPARATOR_HEIGHT: i32 = 9;
const MARGIN: i32 = 24;

#[derive(Debug, Clone, Default)]
pub struct Item {
    pub id: u64,
    pub text: String,
    /// Its MF_* flags: state and type together, as `GetMenuState` gives them.
    pub flags: u32,
    pub submenu: u64,
    pub data: u64,
}

impl Item {
    pub fn separator(&self) -> bool {
        self.flags & MF_SEPARATOR != 0
    }

    pub fn enabled(&self) -> bool {
        self.flags & (MF_GRAYED | MF_DISABLED) == 0
    }

    fn height(&self) -> i32 {
        if self.separator() { SEPARATOR_HEIGHT } else { ITEM_HEIGHT }
    }

    /// Its label and accelerator, as they're drawn.
    fn parts(&self) -> (String, String) {
        let (text, accel) = self.text.split_once('\t').unwrap_or((&self.text, ""));
        (label(text), accel.to_string())
    }

    /// The letter after its `&`, lower case.
    fn mnemonic(&self) -> Option<char> {
        let mut chars = self.text.chars();
        while let Some(c) = chars.next() {
            if c == '&' {
                match chars.next() {
                    Some('&') => continue,
                    Some(m) => return Some(m.to_ascii_lowercase()),
                    None => return None,
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Default)]
pub struct Menu {
    pub items: Vec<Item>,
}

impl Menu {
    /// Each item's top and bottom, inside the border.
    fn rows(&self) -> Vec<(i32, i32)> {
        let mut y = 2;
        self.items
            .iter()
            .map(|i| {
                let top = y;
                y += i.height();
                (top, y)
            })
            .collect()
    }

    fn size(&self) -> (i32, i32) {
        let widest = self
            .items
            .iter()
            .map(|i| {
                let (text, accel) = i.parts();
                let n = text.chars().count() + if accel.is_empty() { 0 } else { 3 + accel.chars().count() };
                n as i32 * CELL_WIDTH
            })
            .max()
            .unwrap_or(0);
        let height = self.items.iter().map(Item::height).sum::<i32>() + 4;
        ((widest + 2 * MARGIN + 2).max(120), height.max(8))
    }

    fn item_at(&self, y: i32) -> Option<usize> {
        self.rows().iter().position(|&(top, bottom)| y >= top && y < bottom).filter(|&i| !self.items[i].separator())
    }
}

/// Draws a menu as a popup shows it, `hot` lit up.
fn paint(s: &mut Surface, menu: &Menu, hot: Option<usize>) {
    let all = s.bounds();
    s.fill(&all, 0xf2f2f2, &all);
    s.frame(&all, 0xcccccc, &all);
    let width = s.width;
    for (i, (item, (top, bottom))) in menu.items.iter().zip(menu.rows()).enumerate() {
        if item.separator() {
            let y = top + SEPARATOR_HEIGHT / 2;
            s.line((MARGIN, y), (width - 3, y), 0xd7d7d7, &all);
            continue;
        }
        let row = Rect::new(2, top, width - 2, bottom);
        if hot == Some(i) {
            s.fill(&row, if item.enabled() { 0x91c9f7 } else { 0xe6e6e6 }, &all);
        }
        let color = if item.enabled() { 0x000000 } else { 0x6d6d6d };
        let (text, accel) = item.parts();
        let y = top + (ITEM_HEIGHT - CELL_HEIGHT) / 2;
        s.text(MARGIN, y, &text, color, None, &all);
        if !accel.is_empty() {
            s.text(width - MARGIN - accel.chars().count() as i32 * CELL_WIDTH, y, &accel, color, None, &all);
        }
        // A check or a radio dot in the margin; a submenu's arrow at the right.
        let mid = (top + bottom) / 2;
        if item.flags & MF_CHECKED != 0 {
            if item.flags & MF_RADIOCHECK != 0 {
                s.fill(&Rect::new(9, mid - 2, 14, mid + 3), color, &all);
            } else {
                s.line((7, mid), (10, mid + 3), color, &all);
                s.line((10, mid + 3), (16, mid - 3), color, &all);
            }
        }
        if item.flags & MF_POPUP != 0 {
            for d in 0..4 {
                s.line((width - 12 + d, mid - 3 + d), (width - 12 + d, mid + 3 - d), color, &all);
            }
        }
    }
}

/// One menu shown: its popup on the desktop, the canvas it's drawn on, and its lit item.
pub struct Level {
    pub menu: u64,
    pub shown: u64,
    canvas: display::Canvas,
    pub hot: Option<usize>,
}

/// A menu being tracked: the levels shown, the window it's for, and, once it's over, what was chosen.
pub struct Tracking {
    pub levels: Vec<Level>,
    pub owner: u64,
    pub result: Option<Option<u64>>,
}

/// The window an event is for.
pub fn window_of(e: &Event) -> Option<u64> {
    match e {
        Event::Pointer { window, .. } | Event::Key { window, .. } | Event::PopupDone { window } | Event::Configure { window, .. } | Event::Text { window, .. } | Event::Activated { window, .. } => Some(*window),
        _ => None,
    }
}

impl Win32k {
    /// A new, empty menu.
    pub fn create_menu(&mut self) -> u64 {
        let handle = MENUS + 4 * (self.menus.keys().next_back().map_or(0, |h| (h - MENUS) / 4) + 1);
        self.menus.insert(handle, Menu::default());
        handle
    }

    /// Destroys a menu and the submenus in it.
    pub fn destroy_menu(&mut self, handle: u64) -> bool {
        let Some(menu) = self.menus.remove(&handle) else { return false };
        for item in menu.items {
            if item.flags & MF_POPUP != 0 {
                self.destroy_menu(item.submenu);
            }
        }
        true
    }

    /// An item, by its position or (searching submenus too, as Windows does) by its command: the menu it's in and
    /// its place.
    pub fn find_item(&self, menu: u64, item: u64, by_position: bool) -> Option<(u64, usize)> {
        let m = self.menus.get(&menu)?;
        if by_position {
            return ((item as usize) < m.items.len()).then_some((menu, item as usize));
        }
        for (i, it) in m.items.iter().enumerate() {
            if it.flags & MF_POPUP == 0 && !it.separator() && it.id as u32 == item as u32 {
                return Some((menu, i));
            }
        }
        m.items.iter().filter(|i| i.flags & MF_POPUP != 0).find_map(|i| self.find_item(i.submenu, item, false))
    }

    pub fn item_mut(&mut self, menu: u64, item: u64, by_position: bool) -> Option<&mut Item> {
        let (m, i) = self.find_item(menu, item, by_position)?;
        self.menus.get_mut(&m)?.items.get_mut(i)
    }

    /// Shows a menu as a popup of `parent` on the desktop (`at`, its place against the parent's content): its
    /// level.
    fn open_level(&mut self, menu: u64, parent: u64, anchor_rect: Rect, anchor: Edge, gravity: Edge, client: u64) -> Option<Level> {
        let m = self.menus.get(&menu)?.clone();
        let (w, h) = m.size();
        let positioner = Positioner { size: (w, h), anchor_rect, anchor, gravity, offset: (0, 0), adjust: adjust::FLIP_X | adjust::FLIP_Y | adjust::SLIDE_X | adjust::SLIDE_Y };
        let spec = Spec { rect: Rect::new(0, 0, w, h), title: String::new(), framed: false, layer: Default::default(), role: Role::Popup { parent, positioner, grab: true }, fill: 0xf2f2f2, client, no_activate: true };
        let canvas = display::Canvas::new(w, h, 0xf2f2f2);
        canvas.draw(|s| paint(s, &m, None));
        let mut d = dt::lock(&self.display);
        let shown = d.create(spec);
        d.attach(shown, Some(canvas.buffer()));
        d.commit(shown);
        d.set_visible(shown, true);
        Some(Level { menu, shown, canvas, hot: None })
    }

    fn redraw(&mut self, level: usize) {
        let Some(t) = &self.tracking else { return };
        let Some(l) = t.levels.get(level) else { return };
        let Some(m) = self.menus.get(&l.menu) else { return };
        l.canvas.draw(|s| paint(s, m, l.hot));
        let mut d = dt::lock(&self.display);
        let (w, h) = m.size();
        d.damage(l.shown, Rect::new(0, 0, w, h));
        d.commit(l.shown);
    }

    /// Closes the levels from `from` on.
    fn close_levels(&mut self, from: usize) {
        let Some(t) = &mut self.tracking else { return };
        let gone: Vec<Level> = if from < t.levels.len() { t.levels.drain(from..).collect() } else { Vec::new() };
        let mut d = dt::lock(&self.display);
        for l in gone.into_iter().rev() {
            d.destroy(l.shown);
        }
    }

    /// Lights up an item of a level, opening its submenu if it has one (and closing deeper levels).
    fn light(&mut self, level: usize, hot: Option<usize>, open: bool) {
        let Some(t) = &mut self.tracking else { return };
        let Some(l) = t.levels.get_mut(level) else { return };
        let changed = l.hot != hot;
        l.hot = hot;
        let (menu, shown) = (l.menu, l.shown);
        if changed {
            self.close_levels(level + 1);
            self.redraw(level);
        }
        let Some(i) = hot else { return };
        let Some(item) = self.menus.get(&menu).and_then(|m| m.items.get(i)).cloned() else { return };
        let depth = self.tracking.as_ref().map_or(0, |t| t.levels.len());
        if open && item.flags & MF_POPUP != 0 && item.enabled() && depth == level + 1 {
            let (top, bottom) = self.menus[&menu].rows()[i];
            let width = self.menus[&menu].size().0;
            let client = self.windows.get(&self.tracking.as_ref().map_or(0, |t| t.owner)).map_or(0, |w| w.pid);
            if let Some(sub) = self.open_level(item.submenu, shown, Rect::new(0, top, width - 2, bottom), Edge::TopRight, Edge::BottomRight, client)
                && let Some(t) = &mut self.tracking
            {
                t.levels.push(sub);
            }
        }
    }

    /// Chooses an item: its ID, if it's one that can be (enabled, not a separator or a submenu).
    fn choose(&mut self, level: usize, i: usize) {
        let Some(menu) = self.tracking.as_ref().and_then(|t| t.levels.get(level)).map(|l| l.menu) else { return };
        let Some(item) = self.menus.get(&menu).and_then(|m| m.items.get(i)).cloned() else { return };
        if item.flags & MF_POPUP != 0 {
            self.light(level, Some(i), true);
            if let Some(t) = &mut self.tracking
                && let Some(sub) = t.levels.get_mut(level + 1)
            {
                sub.hot = self.menus.get(&sub.menu).and_then(|m| m.items.iter().position(|it| !it.separator()));
            }
            self.redraw(level + 1);
            return;
        }
        if item.enabled()
            && !item.separator()
            && let Some(t) = &mut self.tracking
        {
            t.result = Some(Some(item.id));
        }
    }

    /// One of the desktop's events, for a menu being tracked.
    pub fn menu_event(&mut self, event: Event) {
        let Some(t) = &self.tracking else { return };
        let Some(level) = window_of(&event).and_then(|w| t.levels.iter().position(|l| l.shown == w)) else { return };
        let menu = t.levels[level].menu;
        let Some(m) = self.menus.get(&menu).cloned() else { return };
        match event {
            Event::Pointer { y, what: Pointer::Move | Pointer::Enter, .. } => self.light(level, m.item_at(y), true),
            Event::Pointer { y, what: Pointer::Up(_), .. } => {
                if let Some(i) = m.item_at(y) {
                    self.choose(level, i);
                }
            }
            Event::Pointer { y, what: Pointer::Down(_), .. } => self.light(level, m.item_at(y), true),
            Event::PopupDone { .. } => {
                if level == 0 {
                    if let Some(t) = &mut self.tracking {
                        t.result.get_or_insert(None);
                    }
                } else {
                    self.close_levels(level);
                }
            }
            Event::Key { code, down: true, .. } => {
                // Keys work on the deepest level.
                let deepest = self.tracking.as_ref().map_or(0, |t| t.levels.len() - 1);
                let Some(dm) = self.tracking.as_ref().and_then(|t| t.levels.get(deepest)).and_then(|l| self.menus.get(&l.menu)).cloned() else { return };
                let hot = self.tracking.as_ref().and_then(|t| t.levels.get(deepest)).and_then(|l| l.hot);
                let selectable: Vec<usize> = (0..dm.items.len()).filter(|&i| !dm.items[i].separator()).collect();
                let step = |forward: bool| -> Option<usize> {
                    if selectable.is_empty() {
                        return None;
                    }
                    let at = hot.and_then(|h| selectable.iter().position(|&s| s == h));
                    let n = selectable.len();
                    Some(selectable[match (at, forward) {
                        (None, true) => 0,
                        (None, false) => n - 1,
                        (Some(a), true) => (a + 1) % n,
                        (Some(a), false) => (a + n - 1) % n,
                    }])
                };
                match keys::vk(code) {
                    keys::VK_DOWN => self.light(deepest, step(true), false),
                    keys::VK_UP => self.light(deepest, step(false), false),
                    keys::VK_RIGHT => {
                        if let Some(h) = hot.filter(|&h| dm.items[h].flags & MF_POPUP != 0) {
                            self.choose(deepest, h);
                        }
                    }
                    keys::VK_LEFT if deepest > 0 => self.close_levels(deepest),
                    keys::VK_ESCAPE if deepest > 0 => self.close_levels(deepest),
                    keys::VK_ESCAPE => {
                        if let Some(t) = &mut self.tracking {
                            t.result = Some(None);
                        }
                    }
                    keys::VK_RETURN | keys::VK_SPACE => {
                        if let Some(h) = hot {
                            self.choose(deepest, h);
                        }
                    }
                    vk if vk.is_ascii_alphanumeric() => {
                        let c = (vk as char).to_ascii_lowercase();
                        if let Some(i) = dm.items.iter().position(|it| it.mnemonic() == Some(c)) {
                            self.light(deepest, Some(i), false);
                            self.choose(deepest, i);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl crate::process::Task {
    /// `TrackPopupMenuEx(menu, flags, x, y, window, params)`: the menu shown at a place on the screen for a window,
    /// until something's chosen or it's dismissed. With TPM_RETURNCMD the item's ID (0 for none); else TRUE, and
    /// the window gets WM_COMMAND for the item (unless TPM_NONOTIFY). FALSE if it couldn't be shown.
    pub fn track_popup_menu(&mut self, cx: &mut kernels_core::thread::Cx<'_>, menu: u64, flags: u32, x: i32, y: i32, hwnd: u64) -> Result<u64, crate::os::Reply> {
        const TPM_CENTERALIGN: u32 = 0x4;
        const TPM_RIGHTALIGN: u32 = 0x8;
        const TPM_VCENTERALIGN: u32 = 0x10;
        const TPM_BOTTOMALIGN: u32 = 0x20;
        const TPM_NONOTIFY: u32 = 0x80;
        const TPM_RETURNCMD: u32 = 0x100;
        const WM_INITMENUPOPUP: u32 = 0x117;
        let (top, shown) = {
            let w = self.win32k();
            if w.tracking.is_some() || !w.menus.contains_key(&menu) {
                return Ok(0);
            }
            let Some(top) = w.top_level_of(hwnd) else { return Ok(0) };
            (top, w.windows.get(&top).map_or(0, |t| t.shown))
        };
        if shown == 0 {
            return Ok(0);
        }
        self.send_message(cx, hwnd, WM_INITMENUPOPUP, menu, 0)?;
        let parent = self.desktop(|d| d.content_rect(shown)).unwrap_or_default();
        // Where the menu goes against the point: down and right of it, unless the flags say otherwise.
        let horizontal = if flags & TPM_RIGHTALIGN != 0 { -1 } else if flags & TPM_CENTERALIGN != 0 { 0 } else { 1 };
        let vertical = if flags & TPM_BOTTOMALIGN != 0 { -1 } else if flags & TPM_VCENTERALIGN != 0 { 0 } else { 1 };
        let gravity = match (horizontal, vertical) {
            (1, 1) => Edge::BottomRight,
            (-1, 1) => Edge::BottomLeft,
            (1, -1) => Edge::TopRight,
            (-1, -1) => Edge::TopLeft,
            (0, 1) => Edge::Bottom,
            (0, -1) => Edge::Top,
            (1, 0) => Edge::Right,
            (-1, 0) => Edge::Left,
            _ => Edge::Centre,
        };
        let anchor = Rect::new(x - parent.left, y - parent.top, x - parent.left + 1, y - parent.top + 1);
        {
            let pid = self.process.pid;
            let mut w = self.win32k();
            let Some(level) = w.open_level(menu, shown, anchor, Edge::TopLeft, gravity, pid) else { return Ok(0) };
            w.tracking = Some(Tracking { levels: vec![level], owner: hwnd, result: None });
        }
        let result = loop {
            self.waiter.reset();
            self.pump();
            self.take_sent(cx)?;
            if let Some(result) = self.win32k().tracking.as_ref().and_then(|t| t.result) {
                break result;
            }
            if !self.window_exists(hwnd) {
                break None;
            }
            if self.waiter.wait(Some(std::time::Instant::now() + std::time::Duration::from_millis(100))) == kernels_core::wait::Woke::Interrupted && self.ending() {
                self.win32k().close_levels(0);
                self.win32k().tracking = None;
                return Err(crate::os::Reply::Ended);
            }
        };
        {
            let mut w = self.win32k();
            w.close_levels(0);
            w.tracking = None;
        }
        if flags & TPM_RETURNCMD != 0 {
            return Ok(result.unwrap_or(0) & 0xffff_ffff);
        }
        if let Some(id) = result
            && flags & TPM_NONOTIFY == 0
        {
            let tid = self.tid;
            self.post_message(hwnd, tid, super::WM_COMMAND, id & 0xffff, 0);
        }
        let _ = top;
        Ok(1)
    }
}

// ---- Menu resources ----

/// A `MENU` or `MENUEX` resource's items, made into menus (the popups' submenus among them): the top menu's items.
pub fn parse_menu(b: &[u8], w: &mut Win32k) -> Option<u64> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let string = |at: &mut usize| -> Option<String> {
        let mut units = Vec::new();
        loop {
            let u = u16_at(*at)?;
            *at += 2;
            if u == 0 {
                return Some(String::from_utf16_lossy(&units));
            }
            units.push(u);
        }
    };
    let version = u16_at(0)?;
    let menu = w.create_menu();
    if version == 0 {
        // MENU: each item its flags, its ID (but a popup's), its text; MF_END (0x80) ends a level.
        fn level(b: &[u8], at: &mut usize, w: &mut Win32k, menu: u64, string: &dyn Fn(&mut usize) -> Option<String>) -> Option<()> {
            loop {
                let flags = u16::from_le_bytes(b.get(*at..*at + 2)?.try_into().ok()?);
                *at += 2;
                let popup = flags & 0x10 != 0;
                let id = if popup {
                    0
                } else {
                    let id = u16::from_le_bytes(b.get(*at..*at + 2)?.try_into().ok()?);
                    *at += 2;
                    id
                };
                let text = string(at)?;
                let mut item = Item { id: u64::from(id), text, flags: u32::from(flags & !0x80), submenu: 0, data: 0 };
                // An item with no text and no ID is a separator.
                if !popup && id == 0 && item.text.is_empty() {
                    item.flags |= MF_SEPARATOR;
                }
                if popup {
                    item.submenu = w.create_menu();
                    level(b, at, w, item.submenu, string)?;
                }
                w.menus.get_mut(&menu)?.items.push(item);
                if flags & 0x80 != 0 {
                    return Some(());
                }
            }
        }
        let mut at = 4 + usize::from(u16_at(2)?);
        level(b, &mut at, w, menu, &string)?;
    } else {
        // MENUEX: its header's offset to its items (after its help ID); each item its type, state, ID, then a word
        // saying it's a popup (0x01) or the last (0x80), its text, aligned to a DWORD, and a popup's help ID and items.
        fn level(b: &[u8], at: &mut usize, w: &mut Win32k, menu: u64, string: &dyn Fn(&mut usize) -> Option<String>, u32_at: &dyn Fn(usize) -> Option<u32>) -> Option<()> {
            loop {
                let (kind, state, id) = (u32_at(*at)?, u32_at(*at + 4)?, u32_at(*at + 8)?);
                let res = u16::from_le_bytes(b.get(*at + 12..*at + 14)?.try_into().ok()?);
                *at += 14;
                let text = string(at)?;
                *at = at.next_multiple_of(4);
                let popup = res & 0x01 != 0;
                let mut item = Item { id: u64::from(id), text, flags: kind & (MF_SEPARATOR | MF_RADIOCHECK | MF_OWNERDRAW | MF_BITMAP) | state & (MF_GRAYED | MF_DISABLED | MF_CHECKED | MF_DEFAULT), submenu: 0, data: 0 };
                if popup {
                    *at += 4;
                    item.flags |= MF_POPUP;
                    item.submenu = w.create_menu();
                    level(b, at, w, item.submenu, string, u32_at)?;
                }
                w.menus.get_mut(&menu)?.items.push(item);
                if res & 0x80 != 0 {
                    return Some(());
                }
            }
        }
        let mut at = 4 + usize::from(u16_at(2)?);
        level(b, &mut at, w, menu, &string, &u32_at)?;
    }
    Some(menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn a_menu_resource() {
        // POPUP "&File" { MENUITEM "&Open\tCtrl+O", 10; MENUITEM SEPARATOR; MENUITEM "E&xit", 11 }, MENUITEM "&Help", 20
        let mut b = vec![0, 0, 0, 0];
        b.extend(0x10u16.to_le_bytes());
        b.extend(wide("&File"));
        b.extend(0u16.to_le_bytes());
        b.extend(10u16.to_le_bytes());
        b.extend(wide("&Open\tCtrl+O"));
        b.extend(0u16.to_le_bytes());
        b.extend(0u16.to_le_bytes());
        b.extend(wide(""));
        b.extend(0x80u16.to_le_bytes());
        b.extend(11u16.to_le_bytes());
        b.extend(wide("E&xit"));
        b.extend(0x80u16.to_le_bytes());
        b.extend(20u16.to_le_bytes());
        b.extend(wide("&Help"));
        let mut w = Win32k::new(dt::Desktop::shared(640, 480));
        let menu = parse_menu(&b, &mut w).expect("parses");
        let top = &w.menus[&menu];
        assert_eq!(top.items.len(), 2);
        assert_eq!((top.items[1].id, top.items[1].text.as_str()), (20, "&Help"));
        let file = &w.menus[&top.items[0].submenu];
        assert_eq!(file.items.len(), 3);
        assert!(file.items[1].separator());
        assert_eq!(file.items[0].parts(), ("Open".to_string(), "Ctrl+O".to_string()));
        assert_eq!(file.items[2].mnemonic(), Some('x'));
        assert_eq!(w.find_item(menu, 11, false), Some((top.items[0].submenu, 2)));
    }
}
