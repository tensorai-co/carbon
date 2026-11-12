//! Desktop mode: the whole desktop in one window, whose content view's layer shows the composited screen at the
//! window's backing scale.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use display::desktop::{self, Input, Shared};
use display::observe::ObserverId;
use display::output::{Output, SCALE_1X};

use crate::clipboard::Pasteboard;
use crate::ime;
use crate::input::{Cursors, Keys, button_of, let_go_buttons};
use crate::objc::{Frame, Id, Images, NIL, Point, Size, class, frame_for, frame_of, is_main_thread, layer_for, msg, ns_string, pooled, pump, refresh_of, start_app, wake_loop};
use crate::{Options, map};

/// Shows the desktop in a window of its own until `done` is set (the machine ended), on the main thread. `close` is
/// called once if the user closes the window: the machine should end, and set `done`.
pub fn run(desktop: Shared, options: Options, done: &AtomicBool, close: &dyn Fn()) -> Result<(), String> {
    if !is_main_thread() {
        return Err("the desktop's window has to be run on the main thread".into());
    }
    pooled(|| show(desktop, options, done, close))
}

fn show(desktop: Shared, options: Options, done: &AtomicBool, close: &dyn Fn()) -> Result<(), String> {
    let app = start_app()?;
    let (width, height) = (options.width.max(64), options.height.max(64));
    let content = Frame { origin: Point::default(), size: Size { width: f64::from(width), height: f64::from(height) } };
    // Titled, closable, miniaturizable, resizable; buffered.
    let window = msg!(Id; msg!(Id; class("NSWindow"), "alloc"), "initWithContentRect:styleMask:backing:defer:", content => Frame, 15usize => usize, 2usize => usize, false => bool);
    if window.is_null() {
        return Err("AppKit made no window".into());
    }
    msg!((); window, "setReleasedWhenClosed:", false => bool);
    msg!((); window, "setTitle:", ns_string(&options.title) => Id);
    msg!((); window, "setAcceptsMouseMovedEvents:", true => bool);
    // A view that takes the keyboard through the Mac's input method.
    let ours = ime::new_view(f64::from(width), f64::from(height));
    if !ours.is_null() {
        msg!((); window, "setContentView:", ours => Id);
        msg!((); ours, "release");
    }
    let view = msg!(Id; window, "contentView");
    msg!(bool; window, "makeFirstResponder:", view => Id);
    let layer = layer_for(view);
    // The guest's text cursor, on the Mac's screen: where an input method's window opens.
    let caret_desktop = desktop.clone();
    ime::set_caret(Box::new(move || {
        let (id, r) = desktop::lock(&caret_desktop).text_input()?;
        let c = desktop::lock(&caret_desktop).content_rect(id)?;
        let height = frame_of(view, "bounds").size.height;
        let (x, y) = (c.left + r.left, c.top + r.top);
        let local = Frame { origin: Point { x: f64::from(x), y: height - f64::from(y + r.height().max(1)) }, size: Size { width: f64::from(r.width().max(1)), height: f64::from(r.height().max(1)) } };
        Some(frame_for(window, "convertRectToScreen:", local))
    }));
    msg!((); window, "center");
    msg!((); window, "makeKeyAndOrderFront:", NIL => Id);
    msg!((); app, "activateIgnoringOtherApps:", true => bool);

    let fps = refresh_of(msg!(Id; window, "screen"));
    let posted = Arc::new(AtomicBool::new(false));
    let observer = {
        let mut d = desktop::lock(&desktop);
        let id = d.observe();
        let p = posted.clone();
        d.set_observer_waker(id, Arc::new(move || wake_loop(&p)));
        // The desktop's clock is this loop: woken when a window starts waiting for a frame.
        let p = posted.clone();
        d.set_clock(Arc::new(move || wake_loop(&p)));
        id
    };
    let mut host = Host::new(desktop.clone(), window, view, layer, observer, fps);
    host.set_output(width, height);

    let period = Duration::from_secs_f64(1.0 / f64::from(fps));
    let start = Instant::now();
    let mut next_tick = start;
    let mut closed = false;
    while !done.load(Ordering::Acquire) {
        pooled(|| {
            let wants = desktop::lock(&desktop).wants_frame();
            let wait = if wants { next_tick.saturating_duration_since(Instant::now()) } else { Duration::from_millis(100) };
            pump(app, wait, &posted, |event| host.take(event));
            if !closed && !msg!(bool; window, "isVisible") {
                closed = true;
                host.let_go();
                close();
            }
            host.follow_window();
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
    host.let_go();
    host.cursors.unhide();
    desktop::lock(&desktop).unobserve(observer);
    if !closed {
        msg!((); window, "close");
    }
    Ok(())
}

/// What the loop keeps between events.
struct Host {
    desktop: Shared,
    window: Id,
    view: Id,
    layer: Id,
    observer: ObserverId,
    fps: u32,
    /// The output's size, as the window's content has it, and its scale, as the window's backing has it (120ths).
    size: (i32, i32),
    scale: u32,
    /// The screen as last shown, in its own pixels.
    pixels: Vec<u32>,
    shown: (i32, i32),
    images: Images,
    scroll: map::Scroll,
    keys: Keys,
    buttons: u8,
    cursors: Cursors,
    key_window: bool,
    board: Pasteboard,
}

impl Host {
    fn new(desktop: Shared, window: Id, view: Id, layer: Id, observer: ObserverId, fps: u32) -> Host {
        Host {
            desktop,
            window,
            view,
            layer,
            observer,
            fps,
            size: (0, 0),
            scale: SCALE_1X,
            pixels: Vec::new(),
            shown: (0, 0),
            images: Images::new(),
            scroll: map::Scroll::default(),
            keys: Keys::default(),
            buttons: 0,
            cursors: Cursors::default(),
            key_window: true,
            board: Pasteboard::new(),
        }
    }

    fn send(&self, input: Input) {
        desktop::send(&self.desktop, input);
    }

    /// The window's backing scale (2 on a Retina screen), in 120ths.
    fn backing_scale(&self) -> u32 {
        let factor = msg!(f64; self.window, "backingScaleFactor");
        if factor > 0.0 { (factor * f64::from(SCALE_1X)).round() as u32 } else { SCALE_1X }
    }

    /// The output made the window's content's size, at its backing scale, and the kernel woken to hear of it.
    fn set_output(&mut self, width: i32, height: i32) {
        self.size = (width, height);
        self.scale = self.backing_scale();
        let output = Output { name: "host".into(), width, height, scale: self.scale, refresh_mhz: self.fps * 1000, physical_mm: (0, 0) };
        let waker = {
            let mut d = desktop::lock(&self.desktop);
            d.set_output(output);
            d.waker()
        };
        if let Some(wake) = waker {
            wake();
        }
    }

    /// An event, if it's the desktop's: true if it's taken, and AppKit's not to see it. Mouse events go on to AppKit
    /// too, for the title bar, the window's edges and its buttons.
    fn take(&mut self, event: Id) -> bool {
        let kind = msg!(usize; event, "type");
        if msg!(Id; event, "window") != self.window {
            return false;
        }
        if let Some((button, bit, down)) = button_of(event, kind) {
            let (x, y, inside) = self.location(event);
            if down && inside {
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
            // Moved, and dragged with each button.
            5 | 6 | 7 | 27 => {
                let (x, y, inside) = self.location(event);
                if inside || self.buttons != 0 {
                    self.send(Input::PointerTo(x, y));
                }
                self.cursors.hover(inside);
                false
            }
            // The wheel, or a trackpad's two fingers.
            22 => {
                let (x, y, inside) = self.location(event);
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
            _ => self.keys.take(&self.desktop, event, kind, self.view),
        }
    }

    /// Where a mouse event is, in the desktop's logical pixels from the content's top left, and whether that's in it.
    fn location(&self, event: Id) -> (i32, i32, bool) {
        let at = msg!(Point; event, "locationInWindow");
        let p = msg!(Point; self.view, "convertPoint:fromView:", at => Point, NIL => Id);
        let b = frame_of(self.view, "bounds");
        let (x, y) = (p.x, b.size.height - p.y);
        let inside = x >= 0.0 && y >= 0.0 && x < b.size.width && y < b.size.height;
        (x.floor() as i32, y.floor() as i32, inside)
    }

    /// Follows the window: its content's size is the output's, its backing scale the output's scale (it changes as
    /// the window moves to another screen), and keys and buttons held when it stops having the keyboard are let go.
    fn follow_window(&mut self) {
        let b = frame_of(self.view, "bounds");
        let (w, h) = (b.size.width.round() as i32, b.size.height.round() as i32);
        if ((w, h) != self.size || self.backing_scale() != self.scale) && w > 0 && h > 0 {
            self.set_output(w, h);
        }
        let key = msg!(bool; self.window, "isKeyWindow");
        if self.key_window && !key {
            self.let_go();
        }
        self.key_window = key;
    }

    /// Everything held, let go.
    fn let_go(&mut self) {
        self.keys.let_go(&self.desktop);
        let_go_buttons(&self.desktop, self.buttons);
        self.buttons = 0;
        self.scroll = map::Scroll::default();
    }

    /// Shows what changed on the screen since last time, and follows the desktop's cursor and output.
    fn present(&mut self) {
        let (copied, cursor, resize, offered) = {
            let mut d = desktop::lock(&self.desktop);
            let observed = d.take_observed(self.observer);
            // Changed from outside (`carbon display resize`): the window follows.
            let resize = observed.output.as_ref().map(|o| (o.width, o.height)).filter(|&s| s != self.size);
            // In the screen's own pixels: at the backing scale, sharp where a window drew at it.
            let mut r = d.output().rect_to_pixels(observed.screen);
            let copied = if r.is_empty() {
                false
            } else {
                let screen = d.screen_pixels();
                let (w, h) = (screen.width, screen.height);
                if (w, h) != self.shown || self.pixels.len() != (w * h) as usize {
                    self.shown = (w, h);
                    self.pixels = vec![0; (w * h) as usize];
                    r = screen.bounds();
                }
                for y in r.top.max(0)..r.bottom.min(h) {
                    let at = (y * w) as usize;
                    let (l, rr) = (r.left.max(0) as usize, r.right.min(w) as usize);
                    self.pixels[at + l..at + rr].copy_from_slice(&screen.pixels[at + l..at + rr]);
                }
                true
            };
            (copied, observed.cursor, resize, observed.selections.contains(&display::selection::Selection::Clipboard))
        };
        if offered {
            self.board.offered(&self.desktop);
        }
        if let Some(cursor) = cursor {
            self.cursors.set(cursor);
        }
        if let Some((w, h)) = resize {
            self.size = (w, h);
            msg!((); self.window, "setContentSize:", Size { width: f64::from(w), height: f64::from(h) } => Size);
        }
        if copied {
            self.images.show(self.layer, &self.pixels, self.shown.0, self.shown.1);
        }
    }
}
