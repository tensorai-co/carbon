//! Keys as Windows has them: virtual-key codes and scan codes, from and to the desktop's evdev numbers.

use display::keys as ev;

pub const VK_LBUTTON: u8 = 0x01;
pub const VK_RBUTTON: u8 = 0x02;
pub const VK_MBUTTON: u8 = 0x04;
pub const VK_SHIFT: u8 = 0x10;
pub const VK_CONTROL: u8 = 0x11;
pub const VK_MENU: u8 = 0x12;
pub const VK_F4: u8 = 0x73;
const VK_LSHIFT: u8 = 0xa0;
const VK_RSHIFT: u8 = 0xa1;
const VK_LCONTROL: u8 = 0xa2;
const VK_RCONTROL: u8 = 0xa3;
const VK_LMENU: u8 = 0xa4;
const VK_RMENU: u8 = 0xa5;

/// Keys other than letters, digits and F-keys: evdev, virtual key, and whether its scan code is extended.
const TABLE: [(u16, u8, bool); 39] = [
    (ev::ESC, 0x1b, false),
    (ev::BACKSPACE, 0x08, false),
    (ev::TAB, 0x09, false),
    (ev::ENTER, 0x0d, false),
    (ev::SPACE, 0x20, false),
    (ev::CAPS_LOCK, 0x14, false),
    (ev::LEFT_SHIFT, VK_LSHIFT, false),
    (ev::RIGHT_SHIFT, VK_RSHIFT, false),
    (ev::LEFT_CTRL, VK_LCONTROL, false),
    (ev::RIGHT_CTRL, VK_RCONTROL, true),
    (ev::LEFT_ALT, VK_LMENU, false),
    (ev::RIGHT_ALT, VK_RMENU, true),
    (ev::LEFT_META, 0x5b, true),
    (ev::RIGHT_META, 0x5c, true),
    (ev::PAGE_UP, 0x21, true),
    (ev::PAGE_DOWN, 0x22, true),
    (ev::END, 0x23, true),
    (ev::HOME, 0x24, true),
    (ev::LEFT, 0x25, true),
    (ev::UP, 0x26, true),
    (ev::RIGHT, 0x27, true),
    (ev::DOWN, 0x28, true),
    (ev::INSERT, 0x2d, true),
    (ev::DELETE, 0x2e, true),
    (ev::SEMICOLON, 0xba, false),
    (ev::EQUAL, 0xbb, false),
    (ev::COMMA, 0xbc, false),
    (ev::MINUS, 0xbd, false),
    (ev::DOT, 0xbe, false),
    (ev::SLASH, 0xbf, false),
    (ev::GRAVE, 0xc0, false),
    (ev::LEFT_BRACE, 0xdb, false),
    (ev::BACKSLASH, 0xdc, false),
    (ev::RIGHT_BRACE, 0xdd, false),
    (ev::APOSTROPHE, 0xde, false),
    (ev::F11, 0x7a, false),
    (ev::F12, 0x7b, false),
    // The generic modifiers, for keys sent by them: the left one.
    (ev::LEFT_SHIFT, VK_SHIFT, false),
    (ev::LEFT_CTRL, VK_CONTROL, false),
];

/// A key's virtual-key code (left and right modifiers as themselves), or 0 for one Windows has none for.
pub fn vk(code: u16) -> u8 {
    if let Some(i) = ev::LETTERS.iter().position(|&k| k == code) {
        return b'A' + i as u8;
    }
    if let Some(d) = ev::DIGITS.iter().position(|&k| k == code) {
        return b'0' + d as u8;
    }
    if (ev::F1..ev::F1 + 10).contains(&code) {
        return 0x70 + (code - ev::F1) as u8;
    }
    TABLE.iter().find(|t| t.0 == code).map_or(0, |t| t.1)
}

/// The key for a virtual-key code: generic modifiers as their left keys.
pub fn code(vk: u8) -> Option<u16> {
    match vk {
        b'A'..=b'Z' => Some(ev::LETTERS[(vk - b'A') as usize]),
        b'0'..=b'9' => Some(ev::DIGITS[(vk - b'0') as usize]),
        0x70..=0x79 => Some(ev::F1 + u16::from(vk - 0x70)),
        VK_MENU => Some(ev::LEFT_ALT),
        _ => TABLE.iter().find(|t| t.1 == vk).map(|t| t.0),
    }
}

/// The generic code messages carry for a modifier (`VK_SHIFT` for either shift), and the key itself otherwise.
pub fn generic(vk: u8) -> u8 {
    match vk {
        VK_LSHIFT | VK_RSHIFT => VK_SHIFT,
        VK_LCONTROL | VK_RCONTROL => VK_CONTROL,
        VK_LMENU | VK_RMENU => VK_MENU,
        v => v,
    }
}

/// A key's set-1 scan code, which evdev's numbers are below 89, and whether it's extended.
pub fn scan(code: u16) -> (u8, bool) {
    let extended = TABLE.iter().any(|t| t.0 == code && t.2);
    let scan = match code {
        ev::RIGHT_CTRL => 0x1d,
        ev::RIGHT_ALT => 0x38,
        ev::HOME => 0x47,
        ev::UP => 0x48,
        ev::PAGE_UP => 0x49,
        ev::LEFT => 0x4b,
        ev::RIGHT => 0x4d,
        ev::END => 0x4f,
        ev::DOWN => 0x50,
        ev::PAGE_DOWN => 0x51,
        ev::INSERT => 0x52,
        ev::DELETE => 0x53,
        ev::LEFT_META => 0x5b,
        ev::RIGHT_META => 0x5c,
        c if c < 89 => c as u8,
        _ => 0,
    };
    (scan, extended)
}

/// The key for a set-1 scan code.
pub fn from_scan(scan: u16, extended: bool) -> Option<u16> {
    if !extended && (1..89).contains(&scan) {
        return Some(scan);
    }
    TABLE.iter().map(|t| t.0).find(|&c| self::scan(c) == (scan as u8, extended))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_both_ways() {
        for key in [ev::LETTERS[0], ev::LETTERS[25], ev::DIGITS[0], ev::ENTER, ev::UP, ev::F1, ev::F12, ev::SLASH] {
            assert_eq!(code(vk(key)), Some(key), "{key}");
            let (s, e) = scan(key);
            assert_eq!(from_scan(u16::from(s), e), Some(key), "{key}");
        }
        assert_eq!(vk(ev::LETTERS[0]), b'A');
        assert_eq!(generic(vk(ev::RIGHT_SHIFT)), VK_SHIFT);
        assert_eq!(code(VK_SHIFT), Some(ev::LEFT_SHIFT));
        assert_eq!(scan(ev::UP), (0x48, true));
    }
}
