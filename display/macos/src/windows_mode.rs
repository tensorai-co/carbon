//! Windows mode: each of the desktop's top-level windows in an `NSWindow` of its own, among the Mac's other windows.
//! The desktop is laid over the main screen (below its menu bar, at its backing scale) and stays the truth: each Mac
//! window follows its window there (place, size, title, frame, layer, shown, minimized, stacking, the active one),
//! and what the user does to a Mac window goes back (moved, resized, minimized, closed, made key) as the desktop's
//! own frames would tell it. A framed window gets the Mac's title bar, where the desktop's frame is; an unframed one is
//! borderless; a popup is a child window of its owner's, so it moves with it.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use display::Rect;
use display::desktop::{self, Input, Insets, Layer, Shared, WindowId, state};
use display::observe::{ObserverId, WindowInfo};
use display::output::{Output, SCALE_1X};

use crate::clipboard::Pasteboard;
use crate::ime;
use crate::input::{Cursors, Keys, button_of, let_go_buttons};
use crate::map;
use crate::objc::{
    BOOL, Frame, Id, Images, NIL, Point, Sel, Size, app, class, define_class, frame_for, frame_of, global_location, is_main_thread, layer_for, msg, ns_string, pooled,
    pump, refresh_of, start_app, wake_loop,
};

thread_local! {
    /// Mac windows whose close button was clicked, for the loop to tell the desktop of.
    static CLOSES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// `canBecomeKeyWindow` and `canBecomeMainWindow`: a borderless window takes the keyboard too.
extern "C" fn yes(_: Id, _: Sel) -> bool {
    true
}

/// `windowShouldClose:`: the close button asks the window's kernel (`Event::Close`), which closes it, or doesn't.
extern "C" fn should_close(_: Id, _: Sel, window: Id) -> bool {
    CLOSES.with(|c| c.borrow_mut().push(window as usize));
    false
}

/// `NSPopUpMenuWindowLevel` and `NSFloatingWindowLevel`.
const POPUP_LEVEL: isize = 101;
const TOP_LEVEL: isize = 3;

/// The desktop's place on the Mac's main screen: its visible part, below the menu bar.
#[derive(Debug, Clone, Copy)]
struct Space {
    left: f64,
    /// The visible part's top, in Cocoa's coordinates (y up from the main screen's bottom), and in CoreGraphics'
    /// (y down from its top).
    top: f64,
    top_down: f64,
    width: i32,
    height: i32,
}

impl Space {
    fn of(screen: Id) -> Space {
        let (full, visible) = (frame_of(screen, "frame"), frame_of(screen, "visibleFrame"));
        let top = visible.origin.y + visible.size.height;
        Space {
            left: visible.origin.x,
            top,
            top_down: full.origin.y + full.size.height - top,
            width: visible.size.width.floor() as i32,
            height: visible.size.height.floor() as i32,
        }
    }

    /// A rectangle on the desktop, as a Mac window's (content) rectangle.
    fn to_mac(self, r: Rect) -> Frame {
        Frame {
            origin: Point { x: self.left + f64::from(r.left), y: self.top - f64::from(r.bottom) },
            size: Size { width: f64::from(r.width()), height: f64::from(r.height()) },
        }
    }

    fn desktop_rect(self, f: Frame) -> Rect {
        let (left, top) = ((f.origin.x - self.left).round() as i32, (self.top - (f.origin.y + f.size.height)).round() as i32);
        Rect::new(left, top, left + f.size.width.round() as i32, top + f.size.height.round() as i32)
    }

    /// A point on the screen, as `global_location` gives it, on the desktop.
    fn desktop_point(self, p: Point) -> (i32, i32) {
        ((p.x - self.left).floor() as i32, (p.y - self.top_down).floor() as i32)
    }
}

/// A desktop window's Mac window, as last made to follow it.
struct Shown {
    ns: Id,
    view: Id,
    layer: Id,
    framed: bool,
    popup: bool,
    parent: Option<WindowId>,
    top: bool,
    title: String,
    /// Its content, on the desktop.
    content: Rect,
    visible: bool,
    minimized: bool,
}

/// Shows each of the desktop's windows in a Mac window of its own until `done` is set (the machine ended), on the main
/// thread. The machine goes on without windows; it ends as it would headless.
pub fn run_windows(desktop: Shared, done: &AtomicBool) -> Result<(), String> {
    if !is_main_thread() {
        return Err("the desktop's windows have to be run on the main thread".into());
    }
    pooled(|| show(desktop, done))
}

fn show(desktop: Shared, done: &AtomicBool) -> Result<(), String> {
    let app = start_app()?;
    let types = format!("{BOOL}@:");
    let window_class = define_class("CarbonWindow", "NSWindow", &[("canBecomeKeyWindow", yes as *const c_void, &types), ("canBecomeMainWindow", yes as *const c_void, &types)], &[]);
    let delegate_class = define_class("CarbonWindowDelegate", "NSObject", &[("windowShouldClose:", should_close as *const c_void, &format!("{BOOL}@:@"))], &[]);
    if window_class.is_null() || delegate_class.is_null() {
        return Err("couldn't make the window classes".into());
    }
    let delegate = msg!(Id; msg!(Id; delegate_class, "alloc"), "init");
    msg!((); app, "activateIgnoringOtherApps:", true => bool);

    let screen = msg!(Id; msg!(Id; class("NSScreen"), "screens"), "objectAtIndex:", 0usize => usize);
    if screen.is_null() {
        return Err("this Mac has no screen".into());
    }
    let fps = refresh_of(screen);
    let factor = msg!(f64; screen, "backingScaleFactor");
    let scale = if factor > 0.0 { (factor * f64::from(SCALE_1X)).round() as u32 } else { SCALE_1X };
    let posted = Arc::new(AtomicBool::new(false));
    let observer = {
        let mut d = desktop::lock(&desktop);
        let id = d.observe();
        let p = posted.clone();
        d.set_observer_waker(id, Arc::new(move || wake_loop(&p)));
        let p = posted.clone();
        d.set_clock(Arc::new(move || wake_loop(&p)));
        id
    };
    let mut host = Host {
        desktop: desktop.clone(),
        observer,
        space: Space::of(screen),
        window_class,
        delegate,
        shown: BTreeMap::new(),
        by_ns: HashMap::new(),
        images: Images::new(),
        keys: Keys::default(),
        scroll: map::Scroll::default(),
        buttons: 0,
        cursors: Cursors::default(),
        key_ours: false,
        board: Pasteboard::new(),
    };
    // The guest's text cursor, on the Mac's screen: where an input method's window opens.
    let (caret_desktop, space) = (desktop.clone(), host.space);
    ime::set_caret(Box::new(move || {
        let (id, r) = desktop::lock(&caret_desktop).text_input()?;
        let c = desktop::lock(&caret_desktop).content_rect(id)?;
        let at = Rect::new(c.left + r.left, c.top + r.top, c.left + r.left + r.width().max(1), c.top + r.top + r.height().max(1));
        Some(space.to_mac(at))
    }));
    let output = Output { name: "host".into(), width: host.space.width, height: host.space.height, scale, refresh_mhz: fps * 1000, physical_mm: (0, 0) };
    host.with_desktop(|d| d.set_output(output));

    let period = Duration::from_secs_f64(1.0 / f64::from(fps));
    let start = Instant::now();
    let mut next_tick = start;
    while !done.load(Ordering::Acquire) {
        pooled(|| {
            let wants = desktop::lock(&desktop).wants_frame();
            let wait = if wants { next_tick.saturating_duration_since(Instant::now()) } else { Duration::from_millis(100) };
            pump(app, wait, &posted, |event| host.take(event));
            host.follow_host();
            host.board.follow(&desktop);
            let now = Instant::now();
            if now >= next_tick {
                if wants {
                    display::clock::tick(&desktop, now.duration_since(start).as_millis() as u32);
                }
                next_tick = now + period;
            }
            host.present();
        });
    }
    host.finish();
    desktop::lock(&desktop).unobserve(observer);
    msg!((); delegate, "release");
    Ok(())
}

struct Host {
    desktop: Shared,
    observer: ObserverId,
    space: Space,
    window_class: Id,
    delegate: Id,
    shown: BTreeMap<WindowId, Shown>,
    by_ns: HashMap<usize, WindowId>,
    images: Images,
    keys: Keys,
    scroll: map::Scroll,
    buttons: u8,
    cursors: Cursors,
    /// One of its windows had the keyboard, as last looked.
    key_ours: bool,
    board: Pasteboard,
}

impl Host {
    fn send(&self, input: Input) {
        desktop::send(&self.desktop, input);
    }

    /// Changes the desktop, and wakes its kernel if that made events.
    fn with_desktop<R>(&self, f: impl FnOnce(&mut desktop::Desktop) -> R) -> R {
        let (r, waker) = {
            let mut d = desktop::lock(&self.desktop);
            let r = f(&mut d);
            (r, if d.has_events() { d.waker() } else { None })
        };
        if let Some(wake) = waker {
            wake();
        }
        r
    }

    /// An event, if it's for one of its windows: true if it's taken, and AppKit's not to see it. Mouse events go on
    /// to AppKit too, for the title bars, the windows' edges and their buttons.
    fn take(&mut self, event: Id) -> bool {
        let kind = msg!(usize; event, "type");
        let id = self.by_ns.get(&(msg!(Id; event, "window") as usize)).copied();
        let Some(id) = id else { return false };
        // Where on the desktop, whichever window it was for and wherever that's moved since.
        let (x, y) = self.space.desktop_point(global_location(event));
        let inside = self.shown.get(&id).is_some_and(|s| s.content.contains(x, y));
        if let Some((button, bit, down)) = button_of(event, kind) {
            if down && inside {
                // What the host raised and made key is what the desktop has on top, so the press goes to it.
                self.with_desktop(|d| {
                    if !d.is_popup(id) && d.active() != Some(id) {
                        d.activate(id);
                    }
                });
                self.buttons |= bit;
                self.send(Input::PointerTo(x, y));
                self.send(Input::Button(button, true));
            } else if !down && self.buttons & bit != 0 {
                self.buttons &= !bit;
                self.send(Input::PointerTo(x, y));
                self.send(Input::Button(button, false));
            }
            return false;
        }
        match kind {
            5 | 6 | 7 | 27 => {
                if inside || self.buttons != 0 {
                    self.send(Input::PointerTo(x, y));
                }
                self.cursors.hover(inside);
                false
            }
            22 => {
                if inside {
                    let precise = msg!(bool; event, "hasPreciseScrollingDeltas");
                    let (dx, dy) = (msg!(f64; event, "scrollingDeltaX"), msg!(f64; event, "scrollingDeltaY"));
                    let (sx, sy) = self.scroll.add(dx, dy, precise);
                    self.send(Input::PointerTo(x, y));
                    if (sx, sy) != (0, 0) {
                        self.send(Input::Scroll { x: sx, y: sy });
                    }
                }
                true
            }
            _ => {
                let view = self.shown.get(&id).map_or(NIL, |s| s.view);
                self.keys.take(&self.desktop, event, kind, view)
            }
        }
    }

    /// What the user did to its Mac windows, told to the desktop: close buttons, the key window, and windows moved,
    /// resized or minimized.
    fn follow_host(&mut self) {
        let closes = CLOSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
        for ns in closes {
            if let Some(&id) = self.by_ns.get(&ns) {
                self.with_desktop(|d| d.request_close(id));
            }
        }
        let key = self.by_ns.get(&(msg!(Id; app(), "keyWindow") as usize)).copied();
        match key {
            Some(id) => {
                self.key_ours = true;
                self.with_desktop(|d| {
                    if !d.is_popup(id) && d.active() != Some(id) {
                        d.activate(id);
                    }
                });
            }
            None if self.key_ours => {
                // The keyboard's gone to another app: what's held is let go.
                self.key_ours = false;
                self.keys.let_go(&self.desktop);
                let_go_buttons(&self.desktop, self.buttons);
                self.buttons = 0;
            }
            None => {}
        }
        let mut placed = Vec::new();
        let mut minimized = Vec::new();
        for (&id, s) in self.shown.iter_mut().filter(|(_, s)| s.visible && !s.popup) {
            let mini = msg!(bool; s.ns, "isMiniaturized");
            if mini != s.minimized {
                s.minimized = mini;
                minimized.push((id, mini));
            }
            if mini {
                continue;
            }
            let content = self.space.desktop_rect(frame_for(s.ns, "contentRectForFrameRect:", frame_of(s.ns, "frame")));
            if content != s.content && !content.is_empty() {
                s.content = content;
                placed.push((id, Insets::of(s.framed).around(&content)));
            }
        }
        if !placed.is_empty() || !minimized.is_empty() {
            self.with_desktop(|d| {
                for (id, mini) in minimized {
                    d.set_minimized(id, mini);
                }
                for (id, rect) in placed {
                    d.placed(id, rect);
                }
            });
        }
    }

    /// What changed on the desktop, followed by the Mac windows: new and gone, each window's state, the active one,
    /// the stacking, content, and the cursor.
    fn present(&mut self) {
        let (observed, pixels) = {
            let mut d = desktop::lock(&self.desktop);
            let observed = d.take_observed(self.observer);
            let mut ids: Vec<WindowId> = observed.damage.iter().map(|(id, _)| *id).collect();
            ids.extend(observed.windows.iter().filter(|w| !self.shown.contains_key(&w.id)).map(|w| w.id));
            ids.sort_unstable();
            ids.dedup();
            let pixels: Vec<_> = ids.into_iter().filter_map(|id| d.read_pixels(id).map(|p| (id, p))).collect();
            (observed, pixels)
        };
        for id in &observed.gone {
            self.forget(*id);
        }
        for info in &observed.windows {
            self.follow(info);
        }
        if let Some(Some(active)) = observed.active
            && let Some(s) = self.shown.get(&active).filter(|s| s.visible && !s.popup)
            && !msg!(bool; s.ns, "isKeyWindow")
        {
            msg!((); s.ns, "makeKeyAndOrderFront:", NIL => Id);
        }
        if let Some(order) = &observed.stacking {
            self.restack(order);
        }
        for (id, p) in pixels {
            if let Some(s) = self.shown.get(&id) {
                self.images.show(s.layer, &p.pixels, p.width, p.height);
            }
        }
        if let Some(cursor) = observed.cursor {
            self.cursors.set(cursor);
        }
        if observed.selections.contains(&display::selection::Selection::Clipboard) {
            self.board.offered(&self.desktop);
        }
    }

    /// A Mac window made to be as a desktop window is now, made first if it's new.
    fn follow(&mut self, info: &WindowInfo) {
        if !self.shown.contains_key(&info.id) {
            self.make(info);
        }
        let parent_ns = info.parent.and_then(|p| self.shown.get(&p)).map(|p| p.ns);
        let space = self.space;
        let Some(s) = self.shown.get_mut(&info.id) else { return };
        if s.title != info.title {
            s.title = info.title.clone();
            msg!((); s.ns, "setTitle:", ns_string(&info.title) => Id);
        }
        if !s.popup && s.framed != info.framed {
            s.framed = info.framed;
            msg!((); s.ns, "setStyleMask:", if info.framed { 15usize } else { 0 } => usize);
        }
        let top = info.layer == Layer::Top;
        if !s.popup && s.top != top {
            s.top = top;
            msg!((); s.ns, "setLevel:", if top { TOP_LEVEL } else { 0 } => isize);
        }
        if s.content != info.content && !info.content.is_empty() {
            s.content = info.content;
            let frame = frame_for(s.ns, "frameRectForContentRect:", space.to_mac(info.content));
            msg!((); s.ns, "setFrame:display:", frame => Frame, true => bool);
        }
        let minimized = info.states & state::MINIMIZED != 0;
        let visible = info.on_screen || minimized;
        if !visible && s.visible {
            if s.popup
                && let Some(parent) = parent_ns
            {
                msg!((); parent, "removeChildWindow:", s.ns => Id);
            }
            msg!((); s.ns, "orderOut:", NIL => Id);
            s.visible = false;
            s.minimized = false;
        } else if visible && !s.visible {
            if s.popup
                && let Some(parent) = parent_ns
            {
                msg!((); parent, "addChildWindow:ordered:", s.ns => Id, 1isize => isize);
            }
            msg!((); s.ns, "orderFront:", NIL => Id);
            s.visible = true;
        }
        if s.visible && minimized != s.minimized {
            s.minimized = minimized;
            msg!((); s.ns, if minimized { "miniaturize:" } else { "deminiaturize:" }, NIL => Id);
        }
    }

    fn make(&mut self, info: &WindowInfo) {
        let popup = info.popup;
        let class = if popup { class("NSWindow") } else { self.window_class };
        let style = if info.framed && !popup { 15usize } else { 0 };
        let ns = msg!(Id; msg!(Id; class, "alloc"), "initWithContentRect:styleMask:backing:defer:", self.space.to_mac(info.content) => Frame, style => usize, 2usize => usize, false => bool);
        if ns.is_null() {
            return;
        }
        msg!((); ns, "setReleasedWhenClosed:", false => bool);
        msg!((); ns, "setTitle:", ns_string(&info.title) => Id);
        msg!((); ns, "setAcceptsMouseMovedEvents:", true => bool);
        msg!((); ns, "setDelegate:", self.delegate => Id);
        let top = info.layer == Layer::Top;
        if popup {
            msg!((); ns, "setLevel:", POPUP_LEVEL => isize);
            msg!((); ns, "setHasShadow:", true => bool);
        } else if top {
            msg!((); ns, "setLevel:", TOP_LEVEL => isize);
        }
        // A view that takes the keyboard through the Mac's input method (popups never take it).
        if !popup {
            let ours = ime::new_view(f64::from(info.content.width()), f64::from(info.content.height()));
            if !ours.is_null() {
                msg!((); ns, "setContentView:", ours => Id);
                msg!((); ours, "release");
            }
        }
        let view = msg!(Id; ns, "contentView");
        if !popup {
            msg!(bool; ns, "makeFirstResponder:", view => Id);
        }
        let layer = layer_for(view);
        self.by_ns.insert(ns as usize, info.id);
        self.shown.insert(
            info.id,
            Shown { ns, view, layer, framed: info.framed, popup, parent: info.parent, top, title: info.title.clone(), content: info.content, visible: false, minimized: false },
        );
    }

    /// A desktop window gone: its Mac window closed.
    fn forget(&mut self, id: WindowId) {
        let Some(s) = self.shown.remove(&id) else { return };
        self.by_ns.remove(&(s.ns as usize));
        if s.popup
            && let Some(parent) = s.parent.and_then(|p| self.shown.get(&p))
        {
            msg!((); parent.ns, "removeChildWindow:", s.ns => Id);
        }
        msg!((); s.ns, "orderOut:", NIL => Id);
        msg!((); s.ns, "setDelegate:", NIL => Id);
        msg!((); s.ns, "close");
        msg!((); s.ns, "release");
    }

    /// The Mac windows stacked as the desktop has them, each just above the one below it.
    fn restack(&self, order: &[WindowId]) {
        let mut below: Option<isize> = None;
        for id in order {
            let Some(s) = self.shown.get(id).filter(|s| s.visible && !s.popup && !s.minimized) else { continue };
            if let Some(number) = below {
                msg!((); s.ns, "orderWindow:relativeTo:", 1isize => isize, number => isize);
            }
            below = Some(msg!(isize; s.ns, "windowNumber"));
        }
    }

    fn finish(&mut self) {
        self.keys.let_go(&self.desktop);
        let_go_buttons(&self.desktop, self.buttons);
        self.cursors.unhide();
        let ids: Vec<WindowId> = self.shown.keys().copied().collect();
        for id in ids.into_iter().rev() {
            self.forget(id);
        }
    }
}
