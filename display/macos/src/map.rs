//! What the Mac's input means to the desktop, as plain functions: its key codes as evdev numbers, its scrolling as
//! 120ths of a notch, and the desktop's cursors as AppKit's.

use display::cursor::Shape;
use display::keys;

/// A Mac virtual key code (`kVK_*`, as `NSEvent.keyCode` gives it) as an evdev number, for the keys a PC keyboard has.
pub fn evdev(mac: u16) -> Option<u16> {
    let code = match mac {
        0x00 => 30,  // A
        0x01 => 31,  // S
        0x02 => 32,  // D
        0x03 => 33,  // F
        0x04 => 35,  // H
        0x05 => 34,  // G
        0x06 => 44,  // Z
        0x07 => 45,  // X
        0x08 => 46,  // C
        0x09 => 47,  // V
        0x0a => 86,  // the ISO key beside the left Shift: KEY_102ND
        0x0b => 48,  // B
        0x0c => 16,  // Q
        0x0d => 17,  // W
        0x0e => 18,  // E
        0x0f => 19,  // R
        0x10 => 21,  // Y
        0x11 => 20,  // T
        0x12 => 2,   // 1
        0x13 => 3,   // 2
        0x14 => 4,   // 3
        0x15 => 5,   // 4
        0x16 => 7,   // 6
        0x17 => 6,   // 5
        0x18 => keys::EQUAL,
        0x19 => 10,  // 9
        0x1a => 8,   // 7
        0x1b => keys::MINUS,
        0x1c => 9,   // 8
        0x1d => 11,  // 0
        0x1e => keys::RIGHT_BRACE,
        0x1f => 24,  // O
        0x20 => 22,  // U
        0x21 => keys::LEFT_BRACE,
        0x22 => 23,  // I
        0x23 => 25,  // P
        0x24 => keys::ENTER,
        0x25 => 38,  // L
        0x26 => 36,  // J
        0x27 => keys::APOSTROPHE,
        0x28 => 37,  // K
        0x29 => keys::SEMICOLON,
        0x2a => keys::BACKSLASH,
        0x2b => keys::COMMA,
        0x2c => keys::SLASH,
        0x2d => 49,  // N
        0x2e => 50,  // M
        0x2f => keys::DOT,
        0x30 => keys::TAB,
        0x31 => keys::SPACE,
        0x32 => keys::GRAVE,
        0x33 => keys::BACKSPACE,
        0x35 => keys::ESC,
        0x36 => keys::RIGHT_META,
        0x37 => keys::LEFT_META,
        0x38 => keys::LEFT_SHIFT,
        0x39 => keys::CAPS_LOCK,
        0x3a => keys::LEFT_ALT,
        0x3b => keys::LEFT_CTRL,
        0x3c => keys::RIGHT_SHIFT,
        0x3d => keys::RIGHT_ALT,
        0x3e => keys::RIGHT_CTRL,
        0x41 => 83,  // keypad .
        0x43 => 55,  // keypad *
        0x45 => 78,  // keypad +
        0x47 => 69,  // keypad clear: Num Lock
        0x4b => 98,  // keypad /
        0x4c => 96,  // keypad Enter
        0x4e => 74,  // keypad -
        0x51 => 117, // keypad =
        0x52 => 82,  // keypad 0
        0x53 => 79,
        0x54 => 80,
        0x55 => 81,
        0x56 => 75,
        0x57 => 76,
        0x58 => 77,
        0x59 => 71,
        0x5b => 72,
        0x5c => 73,  // keypad 9
        0x60 => 63,  // F5
        0x61 => 64,  // F6
        0x62 => 65,  // F7
        0x63 => 61,  // F3
        0x64 => 66,  // F8
        0x65 => 67,  // F9
        0x67 => keys::F11,
        0x69 => 183, // F13
        0x6b => 184, // F14
        0x6d => 68,  // F10
        0x6f => keys::F12,
        0x71 => 185, // F15
        0x72 => keys::INSERT, // Help, where a PC keyboard has Insert
        0x73 => keys::HOME,
        0x74 => keys::PAGE_UP,
        0x75 => keys::DELETE,
        0x76 => 62,  // F4
        0x77 => keys::END,
        0x78 => 60,  // F2
        0x79 => keys::PAGE_DOWN,
        0x7a => keys::F1,
        0x7b => keys::LEFT,
        0x7c => keys::RIGHT,
        0x7d => keys::DOWN,
        0x7e => keys::UP,
        _ => return None,
    };
    Some(code)
}

/// For a modifier key's code, the bit in `NSEvent.modifierFlags` that's set while it (that one, left or right) is down:
/// `flagsChanged` says a modifier changed, and this says which way.
pub fn modifier_bit(mac: u16) -> Option<u64> {
    Some(match mac {
        0x3b => 0x0001,  // left Control
        0x38 => 0x0002,  // left Shift
        0x3c => 0x0004,  // right Shift
        0x37 => 0x0008,  // left Command
        0x36 => 0x0010,  // right Command
        0x3a => 0x0020,  // left Option
        0x3d => 0x0040,  // right Option
        0x3e => 0x2000,  // right Control
        0x39 => 0x1_0000, // Caps Lock (on, not down)
        _ => return None,
    })
}

/// Scrolling as it comes from the Mac (points from a trackpad or Magic Mouse, `precise`; else lines from a wheel), as
/// the 120ths of a notch the desktop takes, with what's left over kept for the next.
#[derive(Debug, Default)]
pub struct Scroll {
    x: f64,
    y: f64,
}

/// Points a notch is, for a trackpad: about what a wheel's notch scrolls a page by.
const POINTS_A_NOTCH: f64 = 40.0;

impl Scroll {
    /// `dx` and `dy` as `scrollingDeltaX` and `scrollingDeltaY` have them (content moving right and down positive, as
    /// the user's natural-scrolling setting already has it): the desktop's `x` right and `y` up, in 120ths.
    pub fn add(&mut self, dx: f64, dy: f64, precise: bool) -> (i32, i32) {
        let unit = if precise { 120.0 / POINTS_A_NOTCH } else { 120.0 };
        self.x += -dx * unit;
        self.y += dy * unit;
        let (x, y) = (self.x.trunc(), self.y.trunc());
        self.x -= x;
        self.y -= y;
        (x as i32, y as i32)
    }
}

/// The name of AppKit's cursor (an `NSCursor` class method) nearest a shape.
pub fn cursor_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Text | Shape::VerticalText => "IBeamCursor",
        Shape::Pointer => "pointingHandCursor",
        Shape::Crosshair | Shape::Cell => "crosshairCursor",
        Shape::EResize | Shape::WResize | Shape::EwResize | Shape::ColResize => "resizeLeftRightCursor",
        Shape::NResize | Shape::SResize | Shape::NsResize | Shape::RowResize => "resizeUpDownCursor",
        Shape::Grab => "openHandCursor",
        Shape::Grabbing | Shape::Move | Shape::AllScroll => "closedHandCursor",
        Shape::NoDrop | Shape::NotAllowed => "operationNotAllowedCursor",
        Shape::Copy => "dragCopyCursor",
        Shape::Alias => "dragLinkCursor",
        Shape::ContextMenu => "contextualMenuCursor",
        _ => "arrowCursor",
    }
}

/// Whether text the Mac typed for a key is what that key types on a US keyboard (with Shift or not): then the key goes
/// in as a key, and the machine types it itself. Anything else (a dead key's é, another layout's letter, what an input
/// method committed) goes in as text.
pub fn typed_by_key(text: &str, code: u16, shift: bool) -> bool {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return false };
    keys::text(code, if shift { keys::SHIFT } else { 0 }) == Some(c)
}

/// A range in UTF-16 units (an `NSRange` in an `NSString`) as byte offsets in the same text as UTF-8, clamped to it.
pub fn utf16_to_bytes(text: &str, start: usize, len: usize) -> (usize, usize) {
    let at = |unit: usize| {
        let mut units = 0;
        for (byte, c) in text.char_indices() {
            if units >= unit {
                return byte;
            }
            units += c.len_utf16();
        }
        text.len()
    };
    (at(start), at(start.saturating_add(len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_keys_are_a_pcs() {
        assert_eq!(evdev(0x00), Some(keys::LETTERS[0]));
        assert_eq!(evdev(0x06), Some(keys::LETTERS[25]));
        assert_eq!(evdev(0x1d), Some(keys::DIGITS[0]));
        assert_eq!(evdev(0x12), Some(keys::DIGITS[1]));
        assert_eq!(evdev(0x24), Some(keys::ENTER));
        assert_eq!(evdev(0x7a), Some(keys::F1));
        assert_eq!(evdev(0x7e), Some(keys::UP));
        assert_eq!(evdev(0x3f), None); // fn
        // Every letter, once.
        let letters: Vec<u16> = (0..0x30).filter_map(evdev).filter(|c| keys::LETTERS.contains(c)).collect();
        assert_eq!(letters.len(), 26);
        assert_eq!(modifier_bit(0x3c), Some(4));
    }

    #[test]
    fn scrolling_keeps_what_is_left_over() {
        let mut s = Scroll::default();
        // A wheel's notch away from the user, and a trackpad's few points to the left (content moving right).
        assert_eq!(s.add(0.0, 1.0, false), (0, 120));
        assert_eq!(s.add(10.0, 0.0, true), (-30, 0));
        // Small steps add up.
        assert_eq!(s.add(0.0, 0.15, true), (0, 0));
        assert_eq!(s.add(0.0, 0.15, true), (0, 0));
        assert_eq!(s.add(0.0, 0.15, true), (0, 1));
    }

    #[test]
    fn every_shape_has_a_cursor() {
        assert_eq!(cursor_name(Shape::Text), "IBeamCursor");
        assert_eq!(cursor_name(Shape::NwseResize), "arrowCursor");
        assert_eq!(cursor_name(Shape::Default), "arrowCursor");
    }

    #[test]
    fn keys_type_what_a_us_keyboard_types() {
        assert!(typed_by_key("a", keys::LETTERS[0], false));
        assert!(typed_by_key("A", keys::LETTERS[0], true));
        assert!(!typed_by_key("é", keys::LETTERS[4], false));
        // AZERTY's A is where QWERTY's Q is.
        assert!(!typed_by_key("a", keys::LETTERS[16], false));
        assert!(!typed_by_key("ab", keys::LETTERS[0], false));
    }

    #[test]
    fn utf16_ranges_are_byte_ranges() {
        // "日本語" is 3 units and 9 bytes; "😀" is 2 units and 4 bytes.
        assert_eq!(utf16_to_bytes("日本語", 1, 1), (3, 6));
        assert_eq!(utf16_to_bytes("a😀b", 3, 0), (5, 5));
        assert_eq!(utf16_to_bytes("abc", 2, 10), (2, 3));
    }
}
