//! The desktop: the window manager both kernels share. It has a machine's top-level windows, in their places and
//! stacking order, draws their frames, decides which is active, routes input to them, and composites them into one
//! screen. What's inside a window (win32k's child windows and messages, Wayland's surfaces) is its kernel's.
//!
//! A kernel makes and places windows, draws into their content, and asks to raise or activate them; it reads back
//! [`Event`]s: windows moved or resized by their frames, activated or not, asked to close, and pointer and key input
//! in their content's coordinates. Input comes in as [`Input`], from the host, from `carbon display`, or from a
//! program (`SendInput`); [`send`] wakes the kernel to read what it made.
//!
//! This is the built-in backend, on a screen in memory. A host's windows are the other, behind the same calls.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::keys;
use crate::{Rect, Rgb, Surface, font};

pub type WindowId = u64;

/// A frame's border, where it can be resized from.
pub const BORDER: i32 = 4;
/// A frame's title bar, below its top border.
pub const CAPTION: i32 = 20;
/// How far along an edge from a corner still resizes from that corner.
const CORNER: i32 = 16;
const MIN_WIDTH: i32 = 2 * BORDER + 4 * CAPTION;
const MIN_HEIGHT: i32 = 2 * BORDER + CAPTION;

const BACKGROUND: Rgb = 0x2b_3a4a;
const OUTLINE: Rgb = 0x1a_2129;
const ACTIVE: Rgb = 0x3d_5a80;
const INACTIVE: Rgb = 0x8a_96a3;
const TITLE: Rgb = 0xff_ffff;

/// How far a window's content is from its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    /// A window's, with a frame or without.
    pub fn of(framed: bool) -> Insets {
        if framed { Insets { left: BORDER, top: BORDER + CAPTION, right: BORDER, bottom: BORDER } } else { Insets::default() }
    }

    /// The content within a window's rectangle.
    pub fn inside(&self, r: &Rect) -> Rect {
        Rect::new(r.left + self.left, r.top + self.top, (r.right - self.right).max(r.left + self.left), (r.bottom - self.bottom).max(r.top + self.top))
    }

    /// The window around some content.
    pub fn around(&self, r: &Rect) -> Rect {
        Rect::new(r.left - self.left, r.top - self.top, r.right + self.right, r.bottom + self.bottom)
    }
}

/// Windows always on top go above the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Layer {
    #[default]
    Normal,
    Top,
}

/// A new window.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// The whole window, frame and all, on the screen.
    pub rect: Rect,
    pub title: String,
    /// With the desktop's frame, or drawing its own (or none).
    pub framed: bool,
    pub layer: Layer,
    /// The window it belongs to, which it stays above.
    pub owner: Option<WindowId>,
    /// Its content's colour until it's drawn.
    pub fill: Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn bit(self) -> u8 {
        match self {
            Button::Left => 1,
            Button::Right => 2,
            Button::Middle => 4,
        }
    }
}

/// What came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// The pointer, to a place on the screen.
    PointerTo(i32, i32),
    Button(Button, bool),
    /// The wheel, in notches: up (away from the user) is positive.
    Wheel(i32),
    /// A key (an evdev number), down or up.
    Key(u16, bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Move,
    Down(Button),
    Up(Button),
    Wheel(i32),
}

/// What a kernel reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The user moved or resized it by its frame: its new rectangle, frame and all.
    Placed { window: WindowId, rect: Rect },
    /// It became the active window, or stopped being.
    Activated { window: WindowId, active: bool },
    /// Its close button was clicked.
    Close { window: WindowId },
    /// The pointer, in its content's coordinates, with the buttons and modifiers held after it.
    Pointer { window: WindowId, x: i32, y: i32, what: Pointer, buttons: u8, modifiers: u8 },
    /// A key, to the active window.
    Key { window: WindowId, code: u16, down: bool, modifiers: u8 },
}

/// Called when there are events to read, without the desktop's lock held.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

struct Window {
    rect: Rect,
    title: String,
    framed: bool,
    layer: Layer,
    owner: Option<WindowId>,
    visible: bool,
    fill: Rgb,
    content: Surface,
}

impl Window {
    fn content_rect(&self) -> Rect {
        Insets::of(self.framed).inside(&self.rect)
    }

    fn caption(&self) -> Rect {
        let r = &self.rect;
        Rect::new(r.left + BORDER, r.top + BORDER, r.right - BORDER, r.top + BORDER + CAPTION)
    }

    fn close_button(&self) -> Rect {
        let c = self.caption();
        Rect::new(c.right - CAPTION + 2, c.top + 2, c.right - 2, c.bottom - 2)
    }

    /// What part of it is at a point on the screen.
    fn part(&self, x: i32, y: i32) -> Option<Part> {
        if !self.rect.contains(x, y) {
            return None;
        }
        if !self.framed || self.content_rect().contains(x, y) {
            return Some(Part::Content);
        }
        if self.close_button().contains(x, y) {
            return Some(Part::Close);
        }
        if self.caption().contains(x, y) {
            return Some(Part::Caption);
        }
        let r = &self.rect;
        let mut edges = 0;
        if x < r.left + BORDER || (x < r.left + CORNER && (y < r.top + BORDER || y >= r.bottom - BORDER)) {
            edges |= LEFT;
        }
        if x >= r.right - BORDER || (x >= r.right - CORNER && (y < r.top + BORDER || y >= r.bottom - BORDER)) {
            edges |= RIGHT;
        }
        if y < r.top + BORDER || (y < r.top + CORNER && (x < r.left + BORDER || x >= r.right - BORDER)) {
            edges |= TOP;
        }
        if y >= r.bottom - BORDER || (y >= r.bottom - CORNER && (x < r.left + BORDER || x >= r.right - BORDER)) {
            edges |= BOTTOM;
        }
        Some(Part::Edge(edges))
    }
}

const LEFT: u8 = 1;
const TOP: u8 = 2;
const RIGHT: u8 = 4;
const BOTTOM: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Content,
    Caption,
    Close,
    Edge(u8),
}

/// What a held button is doing.
#[derive(Debug, Clone, Copy)]
enum Grab {
    Content(WindowId),
    Move { window: WindowId, dx: i32, dy: i32 },
    Resize { window: WindowId, edges: u8, start: Rect, from: (i32, i32) },
    Close(WindowId),
}

pub struct Desktop {
    screen: Surface,
    windows: BTreeMap<WindowId, Window>,
    /// Bottom to top.
    order: Vec<WindowId>,
    next: WindowId,
    active: Option<WindowId>,
    pointer: (i32, i32),
    buttons: u8,
    modifiers: u8,
    grab: Option<Grab>,
    events: VecDeque<Event>,
    /// What's changed on the screen since it was last composited.
    damage: Rect,
    waker: Option<Waker>,
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desktop").field("size", &(self.screen.width, self.screen.height)).field("windows", &self.order).field("active", &self.active).finish()
    }
}

/// A desktop shared between its kernel and whatever sends it input.
pub type Shared = Arc<Mutex<Desktop>>;

pub fn lock(desktop: &Mutex<Desktop>) -> std::sync::MutexGuard<'_, Desktop> {
    desktop.lock().unwrap_or_else(|p| p.into_inner())
}

/// Gives the desktop some input, and wakes its kernel if that made events.
pub fn send(desktop: &Mutex<Desktop>, input: Input) {
    let waker = {
        let mut d = lock(desktop);
        d.input(input);
        if d.events.is_empty() { None } else { d.waker.clone() }
    };
    if let Some(wake) = waker {
        wake();
    }
}

impl Desktop {
    pub fn new(width: i32, height: i32) -> Desktop {
        let screen = Surface::new(width, height, BACKGROUND);
        Desktop {
            damage: screen.bounds(),
            screen,
            windows: BTreeMap::new(),
            order: Vec::new(),
            next: 1,
            active: None,
            pointer: (width / 2, height / 2),
            buttons: 0,
            modifiers: 0,
            grab: None,
            events: VecDeque::new(),
            waker: None,
        }
    }

    pub fn shared(width: i32, height: i32) -> Shared {
        Arc::new(Mutex::new(Desktop::new(width, height)))
    }

    pub fn size(&self) -> (i32, i32) {
        (self.screen.width, self.screen.height)
    }

    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    // ---- Windows, as their kernel asks ----

    /// A new window, hidden until it's shown.
    pub fn create(&mut self, spec: Spec) -> WindowId {
        let id = self.next;
        self.next += 1;
        let content = Insets::of(spec.framed).inside(&spec.rect);
        let owner = spec.owner.filter(|o| self.windows.contains_key(o));
        let window = Window {
            rect: spec.rect,
            title: spec.title,
            framed: spec.framed,
            layer: spec.layer,
            owner,
            visible: false,
            fill: spec.fill,
            content: Surface::new(content.width(), content.height(), spec.fill),
        };
        self.windows.insert(id, window);
        self.order.push(id);
        self.restack();
        id
    }

    pub fn destroy(&mut self, id: WindowId) {
        let Some(w) = self.windows.remove(&id) else { return };
        self.damage_rect(w.rect);
        self.order.retain(|&o| o != id);
        for other in self.windows.values_mut() {
            if other.owner == Some(id) {
                other.owner = None;
            }
        }
        if matches!(self.grab, Some(Grab::Content(g) | Grab::Close(g) | Grab::Move { window: g, .. } | Grab::Resize { window: g, .. }) if g == id) {
            self.grab = None;
        }
        self.events.retain(|e| event_window(e) != id);
        if self.active == Some(id) {
            self.active = None;
            self.activate_next(w.owner);
        }
    }

    pub fn exists(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    pub fn rect(&self, id: WindowId) -> Option<Rect> {
        self.windows.get(&id).map(|w| w.rect)
    }

    /// Where its content is on the screen.
    pub fn content_rect(&self, id: WindowId) -> Option<Rect> {
        self.windows.get(&id).map(Window::content_rect)
    }

    /// Moves or resizes it (frame and all). Content that's still inside stays; what's new is its fill.
    pub fn set_rect(&mut self, id: WindowId, rect: Rect) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        let old = w.rect;
        w.rect = rect;
        let inner = w.content_rect();
        if (inner.width(), inner.height()) != (w.content.width, w.content.height) {
            let fill = w.fill;
            w.content.resize(inner.width(), inner.height(), fill);
        }
        if w.visible {
            self.damage_rect(old);
            self.damage_rect(rect);
        }
    }

    pub fn set_title(&mut self, id: WindowId, title: &str) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        w.title = title.to_string();
        let caption = w.caption();
        if w.visible && w.framed {
            self.damage_rect(caption);
        }
    }

    pub fn set_fill(&mut self, id: WindowId, fill: Rgb) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.fill = fill;
        }
    }

    pub fn set_layer(&mut self, id: WindowId, layer: Layer) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.layer = layer;
            let r = w.rect;
            self.restack();
            self.damage_rect(r);
        }
    }

    /// Shows or hides it. A hidden active window passes being active on.
    pub fn set_visible(&mut self, id: WindowId, visible: bool) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        if w.visible == visible {
            return;
        }
        w.visible = visible;
        let (r, owner) = (w.rect, w.owner);
        self.damage_rect(r);
        if !visible && self.active == Some(id) {
            self.set_active(None);
            self.activate_next(owner);
        }
    }

    pub fn is_visible(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|w| w.visible)
    }

    /// Puts it on top of its layer, with the windows it owns above it.
    pub fn raise(&mut self, id: WindowId) {
        if !self.windows.contains_key(&id) {
            return;
        }
        let family = self.owned_by(id);
        self.order.retain(|o| !family.contains(o));
        self.order.extend(family);
        self.restack();
        if let Some(r) = self.rect(id) {
            self.damage_rect(r);
        }
    }

    /// Raises it and makes it the active window, if it's shown.
    pub fn activate(&mut self, id: WindowId) {
        if self.is_visible(id) {
            self.set_active(Some(id));
        }
    }

    pub fn active(&self) -> Option<WindowId> {
        self.active
    }

    /// Bottom to top.
    pub fn stacking(&self) -> &[WindowId] {
        &self.order
    }

    /// Its pixels, to draw into: all of them are counted as changed.
    pub fn content_mut(&mut self, id: WindowId) -> Option<&mut Surface> {
        let w = self.windows.get(&id)?;
        if w.visible {
            let r = w.content_rect();
            self.damage_rect(r);
        }
        self.windows.get_mut(&id).map(|w| &mut w.content)
    }

    pub fn content(&self, id: WindowId) -> Option<&Surface> {
        self.windows.get(&id).map(|w| &w.content)
    }

    /// The top window shown at a point on the screen.
    pub fn window_at(&self, x: i32, y: i32) -> Option<WindowId> {
        self.order.iter().rev().copied().find(|id| self.windows.get(id).is_some_and(|w| w.visible && w.rect.contains(x, y)))
    }

    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }

    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }

    /// What's happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// The screen as it is now, every shown window composited.
    pub fn screen(&mut self) -> &Surface {
        self.composite();
        &self.screen
    }

    /// What's changed on the screen since the last call, composited: for a host to show.
    pub fn take_damage(&mut self) -> Rect {
        let r = self.damage.intersect(&self.screen.bounds());
        self.composite_rect(r);
        self.damage = Rect::default();
        r
    }

    // ---- Input ----

    pub fn input(&mut self, input: Input) {
        match input {
            Input::PointerTo(x, y) => self.pointer_to(x, y),
            Input::Button(b, down) => self.button(b, down),
            Input::Wheel(notches) => self.wheel(notches),
            Input::Key(code, down) => self.key(code, down),
        }
    }

    fn pointer_to(&mut self, x: i32, y: i32) {
        let (w, h) = self.size();
        let (x, y) = (x.clamp(0, w - 1), y.clamp(0, h - 1));
        if (x, y) == self.pointer {
            return;
        }
        self.pointer = (x, y);
        match self.grab {
            Some(Grab::Content(id)) => self.pointer_event(id, Pointer::Move),
            Some(Grab::Move { window, dx, dy }) => {
                if let Some(r) = self.rect(window) {
                    let to = Rect::new(x - dx, y - dy, x - dx + r.width(), y - dy + r.height());
                    self.user_placed(window, to);
                }
            }
            Some(Grab::Resize { window, edges, start, from }) => {
                let (ddx, ddy) = (x - from.0, y - from.1);
                let mut r = start;
                if edges & LEFT != 0 {
                    r.left = (start.left + ddx).min(start.right - MIN_WIDTH);
                }
                if edges & RIGHT != 0 {
                    r.right = (start.right + ddx).max(start.left + MIN_WIDTH);
                }
                if edges & TOP != 0 {
                    r.top = (start.top + ddy).min(start.bottom - MIN_HEIGHT);
                }
                if edges & BOTTOM != 0 {
                    r.bottom = (start.bottom + ddy).max(start.top + MIN_HEIGHT);
                }
                self.user_placed(window, r);
            }
            Some(Grab::Close(_)) => {}
            None => {
                if let Some(id) = self.window_at(x, y)
                    && self.windows[&id].part(x, y) == Some(Part::Content)
                {
                    self.pointer_event(id, Pointer::Move);
                }
            }
        }
    }

    fn button(&mut self, b: Button, down: bool) {
        let (x, y) = self.pointer;
        if down {
            if self.buttons & b.bit() != 0 {
                return;
            }
            self.buttons |= b.bit();
            if let Some(grab) = self.grab {
                if let Grab::Content(id) = grab {
                    self.pointer_event(id, Pointer::Down(b));
                }
                return;
            }
            let Some(id) = self.window_at(x, y) else {
                self.set_active(None);
                return;
            };
            self.set_active(Some(id));
            let w = &self.windows[&id];
            match (w.part(x, y), b) {
                (Some(Part::Content), _) => {
                    self.grab = Some(Grab::Content(id));
                    self.pointer_event(id, Pointer::Down(b));
                }
                (Some(Part::Caption), Button::Left) => self.grab = Some(Grab::Move { window: id, dx: x - w.rect.left, dy: y - w.rect.top }),
                (Some(Part::Close), Button::Left) => self.grab = Some(Grab::Close(id)),
                (Some(Part::Edge(edges)), Button::Left) => self.grab = Some(Grab::Resize { window: id, edges, start: w.rect, from: (x, y) }),
                _ => {}
            }
        } else {
            if self.buttons & b.bit() == 0 {
                return;
            }
            self.buttons &= !b.bit();
            match self.grab {
                Some(Grab::Content(id)) => self.pointer_event(id, Pointer::Up(b)),
                Some(Grab::Close(id)) if b == Button::Left && self.windows.get(&id).is_some_and(|w| w.close_button().contains(x, y)) => {
                    self.events.push_back(Event::Close { window: id });
                }
                _ => {}
            }
            if self.buttons == 0 {
                self.grab = None;
            }
        }
    }

    fn wheel(&mut self, notches: i32) {
        let (x, y) = self.pointer;
        let target = match self.grab {
            Some(Grab::Content(id)) => Some(id),
            _ => self.window_at(x, y).filter(|id| self.windows[id].part(x, y) == Some(Part::Content)).or(self.active),
        };
        if let Some(id) = target {
            self.pointer_event(id, Pointer::Wheel(notches));
        }
    }

    fn key(&mut self, code: u16, down: bool) {
        let m = keys::modifier(code);
        if down {
            self.modifiers |= m;
        } else {
            self.modifiers &= !m;
        }
        if let Some(id) = self.active {
            self.events.push_back(Event::Key { window: id, code, down, modifiers: self.modifiers });
        }
    }

    fn pointer_event(&mut self, id: WindowId, what: Pointer) {
        let Some(c) = self.content_rect(id) else { return };
        let (x, y) = (self.pointer.0 - c.left, self.pointer.1 - c.top);
        let event = Event::Pointer { window: id, x, y, what, buttons: self.buttons, modifiers: self.modifiers };
        // Moves the kernel hasn't read yet are only where the pointer was on the way.
        if what == Pointer::Move
            && let Some(Event::Pointer { window, what: Pointer::Move, .. }) = self.events.back()
            && *window == id
        {
            self.events.pop_back();
        }
        self.events.push_back(event);
    }

    /// Placed by its frame: moved there, and its kernel told.
    fn user_placed(&mut self, id: WindowId, rect: Rect) {
        self.set_rect(id, rect);
        if let Some(Event::Placed { window, .. }) = self.events.back()
            && *window == id
        {
            self.events.pop_back();
        }
        self.events.push_back(Event::Placed { window: id, rect });
    }

    // ---- Stacking and activation ----

    /// It and every window it owns, through their owners, bottom to top as they are now.
    fn owned_by(&self, id: WindowId) -> Vec<WindowId> {
        let mut family = vec![id];
        let mut grew = true;
        while grew {
            grew = false;
            for (&o, w) in &self.windows {
                if !family.contains(&o) && w.owner.is_some_and(|owner| family.contains(&owner)) {
                    family.push(o);
                    grew = true;
                }
            }
        }
        self.order.iter().copied().filter(|o| family.contains(o)).collect()
    }

    /// Layers in order, keeping the order within each.
    fn restack(&mut self) {
        let windows = &self.windows;
        self.order.sort_by_key(|id| windows.get(id).map(|w| w.layer));
    }

    fn set_active(&mut self, id: Option<WindowId>) {
        if self.active == id {
            if let Some(id) = id {
                self.raise(id);
            }
            return;
        }
        if let Some(old) = self.active.take() {
            self.events.push_back(Event::Activated { window: old, active: false });
            if let Some(r) = self.rect(old) {
                self.damage_rect(r);
            }
        }
        if let Some(new) = id {
            self.active = Some(new);
            self.raise(new);
            self.events.push_back(Event::Activated { window: new, active: true });
        }
    }

    /// After the active window goes: its owner if that's shown, else the top window shown.
    fn activate_next(&mut self, owner: Option<WindowId>) {
        let next = owner.filter(|&o| self.is_visible(o)).or_else(|| self.order.iter().rev().copied().find(|&o| self.is_visible(o)));
        if let Some(next) = next {
            self.set_active(Some(next));
        }
    }

    // ---- Compositing ----

    fn damage_rect(&mut self, r: Rect) {
        self.damage = if self.damage.is_empty() { r } else { self.damage.union(&r) };
    }

    fn composite(&mut self) {
        let r = self.damage.intersect(&self.screen.bounds());
        self.composite_rect(r);
        self.damage = Rect::default();
    }

    fn composite_rect(&mut self, clip: Rect) {
        if clip.is_empty() {
            return;
        }
        self.screen.fill(&clip, BACKGROUND, &clip);
        for id in &self.order {
            let w = &self.windows[id];
            if !w.visible || w.rect.intersect(&clip).is_empty() {
                continue;
            }
            if w.framed {
                draw_frame(&mut self.screen, w, self.active == Some(*id), &clip);
            }
            let c = w.content_rect();
            self.screen.blit(&w.content, c.left, c.top, &clip);
        }
    }
}

fn event_window(e: &Event) -> WindowId {
    match e {
        Event::Placed { window, .. } | Event::Activated { window, .. } | Event::Close { window } | Event::Pointer { window, .. } | Event::Key { window, .. } => *window,
    }
}

/// A window's frame: an outline, its border and title bar in the active or inactive colour, its title, and a close
/// button.
fn draw_frame(s: &mut Surface, w: &Window, active: bool, clip: &Rect) {
    let colour = if active { ACTIVE } else { INACTIVE };
    let r = &w.rect;
    s.fill(r, colour, clip);
    s.frame(r, OUTLINE, clip);
    let caption = w.caption();
    let room = (caption.width() - CAPTION - 8) / font::CELL_WIDTH;
    let title: String = w.title.chars().take(room.max(0) as usize).collect();
    let text_clip = caption.intersect(clip);
    s.text(caption.left + 4, caption.top + (CAPTION - font::CELL_HEIGHT) / 2, &title, TITLE, None, &text_clip);
    let close = w.close_button();
    s.frame(&close, TITLE, clip);
    let (a, b) = ((close.left + 4, close.top + 4), (close.right - 5, close.bottom - 5));
    s.line(a, b, TITLE, clip);
    s.line((a.0, b.1), (b.0, a.1), TITLE, clip);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(left: i32, top: i32, w: i32, h: i32) -> Spec {
        Spec { rect: Rect::new(left, top, left + w, top + h), title: "w".into(), framed: true, fill: 0xffffff, ..Spec::default() }
    }

    fn click(d: &mut Desktop, x: i32, y: i32) {
        d.input(Input::PointerTo(x, y));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
    }

    #[test]
    fn insets_go_both_ways() {
        let i = Insets::of(true);
        let r = Rect::new(10, 10, 210, 110);
        assert_eq!(i.around(&i.inside(&r)), r);
        assert_eq!(Insets::of(false).inside(&r), r);
    }

    #[test]
    fn clicking_activates_raises_and_reaches_content() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        let b = d.create(framed(200, 150, 300, 200));
        d.set_visible(a, true);
        d.set_visible(b, true);
        d.activate(b);
        d.take_events();
        assert_eq!(d.stacking(), &[a, b]);
        // a's content, where b doesn't cover it: hovering, then a click.
        d.input(Input::PointerTo(120, 140));
        assert!(matches!(d.take_events()[..], [Event::Pointer { window, what: Pointer::Move, .. }] if window == a));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.stacking(), &[b, a]);
        let content = d.content_rect(a).unwrap();
        let events = d.take_events();
        assert_eq!(events[0], Event::Activated { window: b, active: false });
        assert_eq!(events[1], Event::Activated { window: a, active: true });
        assert_eq!(events[2], Event::Pointer { window: a, x: 120 - content.left, y: 140 - content.top, what: Pointer::Down(Button::Left), buttons: 1, modifiers: 0 });
        assert!(matches!(events[3], Event::Pointer { window, what: Pointer::Up(Button::Left), buttons: 0, .. } if window == a));
        assert_eq!(d.window_at(250, 200), Some(a));
    }

    #[test]
    fn frames_move_resize_and_close() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.activate(a);
        d.take_events();
        // Drag the title bar by (50, 30).
        d.input(Input::PointerTo(150, 100 + BORDER + 5));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(180, 120 + BORDER));
        d.input(Input::PointerTo(200, 130 + BORDER + 5));
        d.input(Input::Button(Button::Left, false));
        let moved = Rect::new(150, 130, 450, 330);
        assert_eq!(d.rect(a), Some(moved));
        assert_eq!(d.take_events(), vec![Event::Placed { window: a, rect: moved }]);
        // Drag the bottom right corner out by (40, 20).
        d.input(Input::PointerTo(449, 329));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(489, 349));
        d.input(Input::Button(Button::Left, false));
        let bigger = Rect::new(150, 130, 490, 350);
        assert_eq!(d.rect(a), Some(bigger));
        let content = d.content_rect(a).unwrap();
        assert_eq!((d.content(a).unwrap().width, d.content(a).unwrap().height), (content.width(), content.height()));
        assert_eq!(d.take_events(), vec![Event::Placed { window: a, rect: bigger }]);
        // The close button, pressed and released on it.
        let (cx, cy) = (bigger.right - BORDER - CAPTION / 2, bigger.top + BORDER + CAPTION / 2);
        click(&mut d, cx, cy);
        assert_eq!(d.take_events(), vec![Event::Close { window: a }]);
        // Pressed on it, released off it: nothing.
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(cx - 100, cy));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.take_events(), vec![]);
    }

    #[test]
    fn a_drag_stays_with_its_window_and_keys_go_to_the_active_one() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(Spec { rect: Rect::new(0, 0, 100, 100), fill: 0, ..Spec::default() });
        d.set_visible(a, true);
        d.input(Input::PointerTo(50, 50));
        d.input(Input::Button(Button::Left, true));
        d.take_events();
        // Outside it, still to it; and only the last of the moves it hasn't read.
        d.input(Input::PointerTo(300, 300));
        d.input(Input::PointerTo(400, 350));
        assert_eq!(d.take_events(), vec![Event::Pointer { window: a, x: 400, y: 350, what: Pointer::Move, buttons: 1, modifiers: 0 }]);
        d.input(Input::Button(Button::Left, false));
        d.input(Input::Key(keys::LEFT_SHIFT, true));
        d.input(Input::Key(keys::LETTERS[0], true));
        let events = d.take_events();
        assert_eq!(events.last(), Some(&Event::Key { window: a, code: keys::LETTERS[0], down: true, modifiers: keys::SHIFT }));
        assert_eq!(keys::text(keys::LETTERS[0], keys::SHIFT), Some('A'));
    }

    #[test]
    fn owned_windows_stay_above_and_the_top_layer_above_all() {
        let mut d = Desktop::new(800, 600);
        let owner = d.create(framed(0, 0, 200, 200));
        let owned = d.create(Spec { owner: Some(owner), ..framed(50, 50, 100, 100) });
        let top = d.create(Spec { layer: Layer::Top, ..framed(300, 300, 100, 100) });
        let other = d.create(framed(400, 0, 200, 200));
        for id in [owner, owned, top, other] {
            d.set_visible(id, true);
        }
        assert_eq!(d.stacking(), &[owner, owned, other, top]);
        d.activate(owner);
        assert_eq!(d.stacking(), &[other, owner, owned, top]);
        d.set_visible(owner, false);
        assert_eq!(d.active(), Some(top));
        // Hiding an owner leaves what it owns shown: that's the top window now.
        d.destroy(top);
        assert_eq!(d.active(), Some(owned));
    }

    #[test]
    fn compositing_draws_frames_and_content() {
        let mut d = Desktop::new(400, 300);
        let a = d.create(framed(10, 10, 200, 100));
        d.set_visible(a, true);
        d.content_mut(a).unwrap().fill(&Rect::new(0, 0, 5, 5), 0xff0000, &Rect::new(0, 0, 1000, 1000));
        d.activate(a);
        let c = d.content_rect(a).unwrap();
        let screen = d.screen();
        assert_eq!(screen.get(c.left, c.top), Some(0xff0000));
        assert_eq!(screen.get(c.left + 10, c.top + 10), Some(0xffffff));
        assert_eq!(screen.get(10, 10), Some(OUTLINE));
        assert_eq!(screen.get(c.left + 2, c.top - 2), Some(ACTIVE));
        assert_eq!(screen.get(300, 250), Some(BACKGROUND));
        d.destroy(a);
        assert_eq!(d.screen().get(c.left, c.top), Some(BACKGROUND));
    }

    #[test]
    fn send_wakes_the_kernel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let d = Desktop::shared(100, 100);
        let woken = Arc::new(AtomicUsize::new(0));
        let count = woken.clone();
        lock(&d).set_waker(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let a = lock(&d).create(Spec { rect: Rect::new(0, 0, 100, 100), ..Spec::default() });
        lock(&d).set_visible(a, true);
        send(&d, Input::PointerTo(10, 10));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
        lock(&d).take_events();
        send(&d, Input::PointerTo(10, 10));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
    }
}
