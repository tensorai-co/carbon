//! A view: the desktop as an app embedding it shows it, in a part of its own window (an IDE's side panel, say),
//! at its own pace. Sans-IO: it takes what changed from an observer and gives back the view's state and the tiles of
//! pixels that changed, and turns the app's input into the desktop's. `carbon`'s control socket carries it to another
//! process (`view`), and any app draws its tiles natively.
//!
//! Two modes, switched at any time:
//!
//! - **Desktop**: the whole screen, the desktop's own window manager and frames and all. The view's size is the
//!   screen's: resizing the view resizes the screen.
//! - **Windows**: each top-level window is a tab. Every one is maximized to the view (and put back when the view
//!   leaves this mode or goes), and the view shows one: the active window's, or the window a dialog or popup that's
//!   active belongs to, with its dialogs and popups over it. Its frame is left out, so its content fills the view. A
//!   tab is activated or closed (as its close button would ask) from the app's own tab bar.
//!
//! Coordinates the app sends and receives are the view's, in logical pixels from its top left; tiles are in the
//! output's own pixels (2× on a Retina screen), from the same corner.

use std::collections::BTreeSet;

use crate::Rect;
use crate::cursor::Cursor;
use crate::desktop::{self, Button, Input, Layer, Shared, Waker, WindowId, state};
use crate::observe::{ObserverId, WindowInfo};
use crate::output::Output;

/// How a view shows the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Desktop,
    Windows,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Desktop => "desktop",
            Mode::Windows => "windows",
        }
    }

    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "desktop" => Some(Mode::Desktop),
            "windows" => Some(Mode::Windows),
            _ => None,
        }
    }
}

/// A top-level window, as a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: WindowId,
    pub title: String,
    /// It wants the user: an activation refused, a flash.
    pub attention: bool,
    pub minimized: bool,
}

/// What a view shows now, besides its pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub mode: Mode,
    /// The view's size in logical pixels, and its scale in 120ths: the tiles are `width * scale / 120` across.
    pub width: i32,
    pub height: i32,
    pub scale: u32,
    /// The top-level windows, oldest first: the tabs in windows mode, a taskbar's in desktop mode.
    pub tabs: Vec<Tab>,
    /// The active window's tab.
    pub active: Option<WindowId>,
    /// In windows mode, the tab shown.
    pub shown: Option<WindowId>,
    /// The pointer's cursor, by its CSS name (`default`, `text`, `pointer`, `nwse-resize`, ...), `none` when it's
    /// hidden. A window's own picture shows as `default` for now.
    pub cursor: String,
}

/// Pixels that changed: `0x00RRGGBB` each, a row after another, at `(x, y)` in the view's pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}

/// What changed since the view last took: its state if that did, and the pixels to draw again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    pub state: Option<State>,
    pub tiles: Vec<Tile>,
}

impl Update {
    pub fn is_empty(&self) -> bool {
        self.state.is_none() && self.tiles.is_empty()
    }
}

/// What an app sends a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The pointer, to a place in the view.
    PointerTo(i32, i32),
    Button(Button, bool),
    /// In 120ths of a notch: up and right positive.
    Scroll { x: i32, y: i32 },
    /// An evdev number (see `keys::by_code` for the W3C `KeyboardEvent.code` names).
    Key(u16, bool),
    /// Text an input method committed, or that no key types.
    Text(String),
    /// The view's new size in logical pixels, and its scale in 120ths.
    Resize { width: i32, height: i32, scale: u32 },
    Mode(Mode),
    /// A tab chosen: it's activated, raised, and restored if it was minimized.
    Activate(WindowId),
    /// A tab's close: asked of its program, as its close button would.
    Close(WindowId),
    /// The screen's colour where no window is, `0x00RRGGBB`: to suit the app's own look.
    Background(u32),
}

/// A view of a desktop. Dropping it stops observing, and puts back the windows windows mode maximized.
pub struct View {
    desktop: Shared,
    observer: ObserverId,
    mode: Mode,
    /// The windows this view maximized, to put back.
    maximized: BTreeSet<WindowId>,
    /// What the app was last told.
    told: Option<State>,
    /// The part of the screen shown, in logical pixels: the whole screen, or the shown tab's content.
    region: Rect,
    /// Everything has to be drawn again: a new view, a new size or mode, another tab.
    redraw: bool,
    cursor: String,
}

impl View {
    /// A view `width` by `height` (logical pixels, at `scale` 120ths) of `desktop`, in `mode`.
    pub fn new(desktop: &Shared, mode: Mode, width: i32, height: i32, scale: u32) -> View {
        let observer = desktop::lock(desktop).observe();
        let mut view = View {
            desktop: desktop.clone(),
            observer,
            mode,
            maximized: BTreeSet::new(),
            told: None,
            region: Rect::default(),
            redraw: true,
            cursor: "default".into(),
        };
        view.resize(width, height, scale);
        view
    }

    /// Called when there's something new to take, from the desktop's thread: it should only wake the app's.
    pub fn set_waker(&self, waker: Waker) {
        desktop::lock(&self.desktop).set_observer_waker(self.observer, waker);
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Carries out what the app sent.
    pub fn command(&mut self, command: Command) {
        match command {
            Command::PointerTo(x, y) => {
                let (x, y) = (x + self.region.left, y + self.region.top);
                desktop::send(&self.desktop, Input::PointerTo(x, y));
            }
            Command::Button(button, down) => desktop::send(&self.desktop, Input::Button(button, down)),
            Command::Scroll { x, y } => desktop::send(&self.desktop, Input::Scroll { x, y }),
            Command::Key(code, down) => desktop::send(&self.desktop, Input::Key(code, down)),
            Command::Text(text) => desktop::send(&self.desktop, Input::Text(text)),
            Command::Resize { width, height, scale } => self.resize(width, height, scale),
            Command::Mode(mode) => self.set_mode(mode),
            Command::Activate(id) => self.activate(id),
            Command::Close(id) => self.with_desktop(|d| d.request_close(id)),
            Command::Background(color) => self.with_desktop(|d| d.set_background(color)),
        }
    }

    /// The view's new size: the screen's in desktop mode; in windows mode, the screen around a maximized window whose
    /// content is this size.
    pub fn resize(&mut self, width: i32, height: i32, scale: u32) {
        let (width, height, scale) = (width.clamp(64, 16384), height.clamp(64, 16384), scale.clamp(60, 960));
        let (w, h) = match self.mode {
            Mode::Desktop => (width, height),
            // A maximized window keeps its frame: the screen's that much bigger, and the frame's left out.
            Mode::Windows => {
                let i = desktop::Insets::of(true);
                (width + i.left + i.right, height + i.top + i.bottom)
            }
        };
        self.with_desktop(|d| {
            if (d.output().width, d.output().height, d.output().scale) != (w, h, scale) {
                d.set_output(Output { width: w, height: h, scale, ..d.output().clone() });
            }
        });
        self.region = Rect::new(0, 0, width, height);
        self.redraw = true;
        self.told = None;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        let (width, height, scale) = self.size();
        if self.mode == Mode::Windows {
            self.restore();
        }
        self.mode = mode;
        self.resize(width, height, scale);
    }

    /// The view's size and scale, as the app last gave them.
    fn size(&self) -> (i32, i32, u32) {
        let scale = desktop::lock(&self.desktop).output().scale;
        (self.region.width(), self.region.height(), scale)
    }

    fn activate(&mut self, id: WindowId) {
        self.with_desktop(|d| {
            if d.states(id) & state::MINIMIZED != 0 {
                d.set_minimized(id, false);
            }
            d.raise(id);
            d.activate(id);
        });
    }

    /// Runs `f` on the desktop, and wakes its kernel if that gave it events to read.
    fn with_desktop<R>(&self, f: impl FnOnce(&mut desktop::Desktop) -> R) -> R {
        let (out, waker) = {
            let mut d = desktop::lock(&self.desktop);
            let out = f(&mut d);
            (out, if d.has_events() { d.waker() } else { None })
        };
        if let Some(wake) = waker {
            wake();
        }
        out
    }

    /// Puts back the windows this view maximized.
    fn restore(&mut self) {
        let windows = std::mem::take(&mut self.maximized);
        self.with_desktop(|d| {
            for id in windows {
                if d.exists(id) {
                    d.set_maximized(id, false);
                }
            }
        });
    }

    /// What changed since the last take: the state if it did, and the pixels to draw again.
    pub fn take(&mut self) -> Update {
        let mode = self.mode;
        let mut maximized = std::mem::take(&mut self.maximized);
        let (state, region, tiles_from, redraw) = {
            let mut d = desktop::lock(&self.desktop);
            let observed = d.take_observed(self.observer);
            if let Some(cursor) = &observed.cursor {
                self.cursor = match cursor {
                    Cursor::Shape(shape) => shape.name().to_string(),
                    Cursor::Image { .. } => "default".to_string(),
                    Cursor::Hidden => "none".to_string(),
                };
            }
            let all = d.windows();
            let tabs: Vec<&WindowInfo> = all.iter().filter(|w| is_tab(w)).collect();
            maximized.retain(|id| d.exists(*id));
            if mode == Mode::Windows {
                for w in &tabs {
                    if w.states & state::MAXIMIZED == 0 && w.states & state::FULLSCREEN == 0 {
                        d.set_maximized(w.id, true);
                        maximized.insert(w.id);
                    }
                }
            }
            let active = d.active();
            let active_tab = active.map(|a| root(&all, a));
            let shown = match mode {
                Mode::Windows => active_tab.filter(|t| tabs.iter().any(|w| w.id == *t)).or_else(|| tabs.iter().rev().find(|w| w.on_screen).map(|w| w.id)),
                Mode::Desktop => None,
            };
            let (width, height) = (self.region.width(), self.region.height());
            let region = match shown.and_then(|id| d.content_rect(id)) {
                Some(c) if mode == Mode::Windows => Rect::new(c.left, c.top, c.left + width, c.top + height),
                _ => Rect::new(0, 0, width, height),
            };
            let output = d.output().clone();
            let state = State {
                mode,
                width,
                height,
                scale: output.scale,
                tabs: tabs.iter().map(|w| Tab { id: w.id, title: w.title.clone(), attention: w.attention, minimized: w.states & state::MINIMIZED != 0 }).collect(),
                active: active_tab,
                shown,
                cursor: self.cursor.clone(),
            };
            let redraw = self.redraw || region != self.region;
            // What to send: everything, or the part of the screen that changed, in the view's coordinates.
            let changed = if redraw { region } else { observed.screen.intersect(&region) };
            let tiles_from = if changed.is_empty() {
                None
            } else {
                let px = output.rect_to_pixels(changed);
                let origin = output.rect_to_pixels(region);
                let screen = d.screen_pixels();
                Some(crop(screen, px, origin.left, origin.top))
            };
            (state, region, tiles_from, redraw)
        };
        self.maximized = maximized;
        self.region = region;
        self.redraw = false;
        let state = (self.told.as_ref() != Some(&state) || redraw).then(|| {
            self.told = Some(state.clone());
            state
        });
        Update { state, tiles: tiles_from.into_iter().collect() }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.restore();
        desktop::lock(&self.desktop).unobserve(self.observer);
    }
}

/// A window that's a tab: top level (not a popup or a dialog), among the normal ones (not a dock, a notification or
/// a desktop's icons).
fn is_tab(w: &WindowInfo) -> bool {
    !w.popup && w.parent.is_none() && w.layer == Layer::Normal
}

/// The top-level window a dialog or popup belongs to (itself, for a top-level one).
fn root(all: &[WindowInfo], mut id: WindowId) -> WindowId {
    for _ in 0..64 {
        match all.iter().find(|w| w.id == id).and_then(|w| w.parent) {
            Some(parent) => id = parent,
            None => break,
        }
    }
    id
}

/// `rect` of `screen` (in its pixels), as a tile placed from `(ox, oy)`.
fn crop(screen: &crate::Surface, rect: Rect, ox: i32, oy: i32) -> Tile {
    let r = rect.intersect(&screen.bounds());
    let (w, h) = (r.width().max(0), r.height().max(0));
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in r.top..r.bottom {
        let at = (y * screen.width + r.left) as usize;
        pixels.extend_from_slice(&screen.pixels[at..at + w as usize]);
    }
    Tile { x: r.left - ox, y: r.top - oy, width: w, height: h, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Canvas;
    use crate::desktop::{Desktop, Event, Role, Spec};
    use crate::output::SCALE_1X;

    fn window(d: &Shared, rect: Rect, title: &str, color: u32) -> (WindowId, Canvas) {
        let mut desk = desktop::lock(d);
        let id = desk.create(Spec { rect, title: title.into(), framed: true, fill: 0xffffff, ..Spec::default() });
        desk.set_visible(id, true);
        let c = desk.content_rect(id).unwrap();
        let canvas = Canvas::new(c.width(), c.height(), color);
        desk.attach(id, Some(canvas.buffer()));
        desk.commit(id);
        desk.activate(id);
        (id, canvas)
    }

    fn pixel(tiles: &[Tile], x: i32, y: i32) -> Option<u32> {
        tiles.iter().rev().find(|t| x >= t.x && y >= t.y && x < t.x + t.width && y < t.y + t.height).map(|t| t.pixels[((y - t.y) * t.width + (x - t.x)) as usize])
    }

    #[test]
    fn desktop_mode_is_the_screen_and_then_only_what_changed() {
        let d = Desktop::shared(800, 600);
        let (a, canvas) = window(&d, Rect::new(100, 100, 400, 300), "Editor", 0x00ff00);
        let mut view = View::new(&d, Mode::Desktop, 640, 480, SCALE_1X);
        let first = view.take();
        let state = first.state.expect("the first take says everything");
        assert_eq!((state.width, state.height, state.mode), (640, 480, Mode::Desktop));
        assert_eq!(state.tabs, vec![Tab { id: a, title: "Editor".into(), attention: false, minimized: false }]);
        assert_eq!(state.active, Some(a));
        assert_eq!(desktop::lock(&d).output().width, 640, "the view's size is the screen's");
        assert_eq!(first.tiles.len(), 1);
        assert_eq!((first.tiles[0].width, first.tiles[0].height), (640, 480));
        let c = desktop::lock(&d).content_rect(a).unwrap();
        assert_eq!(pixel(&first.tiles, c.left + 5, c.top + 5), Some(0x00ff00));

        // Nothing changed: nothing to send.
        assert!(view.take().is_empty());

        // A few pixels drawn: just those, and no state.
        canvas.draw(|s| s.fill(&Rect::new(0, 0, 4, 4), 0xff0000, &Rect::new(0, 0, 9999, 9999)));
        {
            let mut desk = desktop::lock(&d);
            desk.damage(a, Rect::new(0, 0, 4, 4));
            desk.commit(a);
        }
        let next = view.take();
        assert!(next.state.is_none());
        assert_eq!(next.tiles.len(), 1);
        assert!(next.tiles[0].width <= 8 && next.tiles[0].height <= 8, "only the damage: {:?}", (next.tiles[0].width, next.tiles[0].height));
        assert_eq!(pixel(&next.tiles, c.left, c.top), Some(0xff0000));
    }

    #[test]
    fn input_reaches_the_window_under_the_pointer() {
        let d = Desktop::shared(800, 600);
        let (a, _canvas) = window(&d, Rect::new(100, 100, 400, 300), "Editor", 0x00ff00);
        let mut view = View::new(&d, Mode::Desktop, 800, 600, SCALE_1X);
        view.take();
        desktop::lock(&d).take_events();
        let c = desktop::lock(&d).content_rect(a).unwrap();
        view.command(Command::PointerTo(c.left + 3, c.top + 4));
        view.command(Command::Button(Button::Left, true));
        let events = desktop::lock(&d).take_events();
        assert!(events.iter().any(|e| matches!(e, Event::Pointer { window, x: 3, y: 4, what: desktop::Pointer::Down(Button::Left), .. } if *window == a)), "{events:?}");
    }

    #[test]
    fn windows_mode_maximizes_each_tab_shows_the_active_one_and_puts_them_back() {
        let d = Desktop::shared(800, 600);
        let (a, _ca) = window(&d, Rect::new(10, 10, 300, 200), "Terminal", 0x0000ff);
        let (b, _cb) = window(&d, Rect::new(50, 50, 400, 300), "Browser", 0x00ff00);
        let mut view = View::new(&d, Mode::Windows, 500, 400, SCALE_1X);
        let first = view.take();
        let state = first.state.unwrap();
        assert_eq!(state.tabs.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Terminal", "Browser"]);
        assert_eq!((state.active, state.shown), (Some(b), Some(b)));
        {
            let desk = desktop::lock(&d);
            assert!(desk.states(a) & state::MAXIMIZED != 0 && desk.states(b) & state::MAXIMIZED != 0);
            // The content, frame left out, is exactly the view.
            let c = desk.content_rect(b).unwrap();
            assert_eq!((c.width(), c.height()), (500, 400));
        }
        // The shown tab's content fills the view: no frame in it.
        assert_eq!(pixel(&first.tiles, 0, 0), Some(0x00ff00));
        // Past what it's drawn so far (its program draws again at the new size), its fill: still no frame.
        assert_eq!(pixel(&first.tiles, 499, 399), Some(0xffffff));

        // Another tab: it's shown, all of it drawn again, and input is placed in its content.
        view.command(Command::Activate(a));
        let next = view.take();
        assert_eq!(next.state.unwrap().shown, Some(a));
        assert_eq!(pixel(&next.tiles, 10, 10), Some(0x0000ff));
        desktop::lock(&d).take_events();
        view.command(Command::PointerTo(7, 8));
        view.command(Command::Button(Button::Left, true));
        let events = desktop::lock(&d).take_events();
        assert!(events.iter().any(|e| matches!(e, Event::Pointer { window, x: 7, y: 8, .. } if *window == a)), "{events:?}");

        // A tab's close asks its program.
        view.command(Command::Close(b));
        assert!(desktop::lock(&d).take_events().contains(&Event::Close { window: b }));

        // Back to desktop mode: put back as they were.
        view.command(Command::Mode(Mode::Desktop));
        view.take();
        let desk = desktop::lock(&d);
        assert_eq!(desk.states(a) & state::MAXIMIZED, 0);
        assert_eq!(desk.rect(a), Some(Rect::new(10, 10, 300, 200)));
    }

    #[test]
    fn windows_mode_shows_a_dialogs_own_window_and_takes_new_windows_as_tabs() {
        let d = Desktop::shared(800, 600);
        let (a, _ca) = window(&d, Rect::new(10, 10, 300, 200), "Editor", 0x0000ff);
        let mut view = View::new(&d, Mode::Windows, 400, 300, SCALE_1X);
        view.take();
        let dialog = {
            let mut desk = desktop::lock(&d);
            let id = desk.create(Spec { rect: Rect::new(50, 50, 250, 150), title: "Save?".into(), framed: true, role: Role::Dialog { parent: a }, ..Spec::default() });
            desk.set_visible(id, true);
            desk.activate(id);
            id
        };
        // A dialog isn't a tab, and its window stays the one shown: nothing new to say.
        assert!(view.take().state.is_none());
        assert_eq!(desktop::lock(&d).states(dialog) & state::MAXIMIZED, 0, "nor is it maximized");

        let (b, _cb) = window(&d, Rect::new(20, 20, 200, 100), "Second", 0x00ff00);
        let state = view.take().state.unwrap();
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.shown, Some(b));
        assert!(desktop::lock(&d).states(b) & state::MAXIMIZED != 0);

        // Dropped: put back.
        drop(view);
        assert_eq!(desktop::lock(&d).states(b) & state::MAXIMIZED, 0);
    }

    #[test]
    fn the_app_picks_the_background() {
        let d = Desktop::shared(800, 600);
        let mut view = View::new(&d, Mode::Desktop, 300, 200, SCALE_1X);
        view.take();
        view.command(Command::Background(0xffffff));
        let next = view.take();
        assert_eq!((next.tiles[0].width, next.tiles[0].height), (300, 200), "all of it again");
        assert_eq!(pixel(&next.tiles, 150, 100), Some(0xffffff));
    }

    #[test]
    fn a_retina_view_gets_twice_the_pixels() {
        let d = Desktop::shared(800, 600);
        let _w = window(&d, Rect::new(10, 10, 300, 200), "Editor", 0x0000ff);
        let mut view = View::new(&d, Mode::Desktop, 300, 200, 2 * SCALE_1X);
        let first = view.take();
        assert_eq!(first.state.unwrap().scale, 240);
        assert_eq!((first.tiles[0].width, first.tiles[0].height), (600, 400));
    }
}
