//! The Mac's keyboard, buttons and cursor, as both modes take and show them.

use std::collections::BTreeSet;

use display::cursor::Cursor;
use display::desktop::{self, Button, Input, Shared};

use crate::ime::{self, Got};
use crate::map;
use crate::objc::{Id, Images, NIL, Point, Size, class, msg, release_image};

/// `NSEvent.modifierFlags`' device-independent bits.
const SHIFT: u64 = 0x2_0000;
const CONTROL: u64 = 0x4_0000;
const OPTION: u64 = 0x8_0000;
const COMMAND: u64 = 0x10_0000;

/// The keys sent down, to send up if the keyboard goes elsewhere.
#[derive(Default)]
pub(crate) struct Keys {
    down: BTreeSet<u16>,
}

impl Keys {
    /// A key event (down, up, or a modifier changing), sent to the desktop: true if it was one. Every key's the
    /// desktop's, Command too: no menu takes it first, and taking them off the queue before AppKit does means
    /// Command's key-ups aren't lost.
    ///
    /// A key that types goes through the input method in `view` (an `ime` view), unless Command, Control or Option is
    /// held (a shortcut, and Option as Alt): what it types as a US keyboard would goes in as the key, so the machine
    /// types it itself; anything else (an IME's commit, a dead key's accent, another layout's letter) as text, and a
    /// composition as preedit. Other keys (arrows, function keys) go in as keys, unless an input method's composing.
    pub fn take(&mut self, desktop: &Shared, event: Id, kind: usize, view: Id) -> bool {
        match kind {
            10 => {
                let code = map::evdev(msg!(u16; event, "keyCode"));
                let flags = msg!(u64; event, "modifierFlags");
                let composing = ime::composing();
                let typing = code.is_none_or(|c| display::keys::text(c, 0).is_some());
                if view.is_null() || (!composing && (flags & (COMMAND | CONTROL | OPTION) != 0 || !typing)) {
                    if let Some(code) = code {
                        self.press(desktop, code, true);
                    }
                    return true;
                }
                let mut key = false;
                for got in ime::interpret(view, event) {
                    match got {
                        Got::Insert(text) => {
                            if !composing && !ime::composing() && code.is_some_and(|c| map::typed_by_key(&text, c, flags & SHIFT != 0)) {
                                key = true;
                            } else if !text.is_empty() {
                                desktop::send(desktop, Input::Text(text));
                            }
                        }
                        Got::Marked(text, start, len) => {
                            let cursor = (!text.is_empty()).then(|| map::utf16_to_bytes(&text, start, len));
                            desktop::send(desktop, Input::Preedit { text, cursor });
                        }
                        Got::Command => key |= !composing,
                    }
                }
                if key && let Some(code) = code {
                    self.press(desktop, code, true);
                }
                true
            }
            // Up, for a key that went down as a key (not one that typed text).
            11 => {
                if let Some(code) = map::evdev(msg!(u16; event, "keyCode")).filter(|c| self.down.contains(c)) {
                    self.press(desktop, code, false);
                }
                true
            }
            // A modifier, down or up; Caps Lock as a press each time it's switched.
            12 => {
                let mac = msg!(u16; event, "keyCode");
                if let (Some(code), Some(bit)) = (map::evdev(mac), map::modifier_bit(mac)) {
                    if code == display::keys::CAPS_LOCK {
                        desktop::send(desktop, Input::Key(code, true));
                        desktop::send(desktop, Input::Key(code, false));
                    } else {
                        let down = msg!(u64; event, "modifierFlags") & bit != 0;
                        self.press(desktop, code, down);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn press(&mut self, desktop: &Shared, code: u16, down: bool) {
        if down {
            self.down.insert(code);
        } else {
            self.down.remove(&code);
        }
        desktop::send(desktop, Input::Key(code, down));
    }

    /// Every key held, let go.
    pub fn let_go(&mut self, desktop: &Shared) {
        for code in std::mem::take(&mut self.down) {
            desktop::send(desktop, Input::Key(code, false));
        }
    }
}

/// A mouse button event's button, its bit in the desktop's `buttons`, and whether it went down; None for another
/// event, or a button beyond the third.
pub(crate) fn button_of(event: Id, kind: usize) -> Option<(Button, u8, bool)> {
    let (button, down) = match kind {
        1 => (Button::Left, true),
        2 => (Button::Left, false),
        3 => (Button::Right, true),
        4 => (Button::Right, false),
        25 | 26 if msg!(isize; event, "buttonNumber") == 2 => (Button::Middle, kind == 25),
        _ => return None,
    };
    let bit = match button {
        Button::Left => 1,
        Button::Right => 2,
        Button::Middle => 4,
    };
    Some((button, bit, down))
}

/// Buttons held, let go.
pub(crate) fn let_go_buttons(desktop: &Shared, buttons: u8) {
    for (bit, button) in [(1, Button::Left), (2, Button::Right), (4, Button::Middle)] {
        if buttons & bit != 0 {
            desktop::send(desktop, Input::Button(button, false));
        }
    }
}

/// The desktop's cursor, shown as AppKit's nearest (or as its own picture) while the pointer's over its content.
pub(crate) struct Cursors {
    cursor: Cursor,
    hidden: bool,
    inside: bool,
    /// An `NSCursor` made from a window's picture, while it's the one shown.
    custom: Id,
}

impl Default for Cursors {
    fn default() -> Cursors {
        Cursors { cursor: Cursor::default(), hidden: false, inside: false, custom: NIL }
    }
}

impl Cursors {
    /// The desktop's cursor changed.
    pub fn set(&mut self, cursor: Cursor) {
        if !self.custom.is_null() {
            msg!((); self.custom, "release");
            self.custom = NIL;
        }
        if let Cursor::Image { pixels, hotspot } = &cursor {
            self.custom = image_cursor(pixels, *hotspot);
        }
        self.cursor = cursor;
        if self.inside {
            self.show();
        }
    }

    /// The pointer came over the desktop's content, or left it: its cursor there, the Mac's own elsewhere.
    pub fn hover(&mut self, inside: bool) {
        let was = self.inside;
        self.inside = inside;
        if inside {
            self.show();
        } else if was {
            self.unhide();
            msg!((); msg!(Id; class("NSCursor"), "arrowCursor"), "set");
        }
    }

    fn show(&mut self) {
        match &self.cursor {
            Cursor::Hidden => {
                if !self.hidden {
                    msg!((); class("NSCursor"), "hide");
                    self.hidden = true;
                }
            }
            other => {
                let name = match other {
                    Cursor::Shape(shape) => map::cursor_name(*shape),
                    _ => "arrowCursor",
                };
                self.unhide();
                let cursor = if self.custom.is_null() { msg!(Id; class("NSCursor"), name) } else { self.custom };
                msg!((); cursor, "set");
            }
        }
    }

    pub fn unhide(&mut self) {
        if self.hidden {
            msg!((); class("NSCursor"), "unhide");
            self.hidden = false;
        }
    }
}

/// An `NSCursor` of a window's own picture (premultiplied `0xAARRGGBB`, a logical pixel to a point) with its hotspot;
/// nil if it can't be made. The caller releases it.
fn image_cursor(pixels: &display::Surface, hotspot: (i32, i32)) -> Id {
    let image = Images::new().image(&pixels.pixels, pixels.width, pixels.height, true);
    if image.is_null() {
        return NIL;
    }
    let size = Size { width: f64::from(pixels.width), height: f64::from(pixels.height) };
    let picture = msg!(Id; msg!(Id; class("NSImage"), "alloc"), "initWithCGImage:size:", image => Id, size => Size);
    release_image(image);
    if picture.is_null() {
        return NIL;
    }
    let spot = Point { x: f64::from(hotspot.0), y: f64::from(hotspot.1) };
    let cursor = msg!(Id; msg!(Id; class("NSCursor"), "alloc"), "initWithImage:hotSpot:", picture => Id, spot => Point);
    msg!((); picture, "release");
    cursor
}
