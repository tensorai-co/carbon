//! Keys, by Linux's evdev numbers, which both kernels' events use, and the keyboard's layout (US English): what a
//! key types.

pub const ESC: u16 = 1;
/// `1` to `9`, then `0`: 2 to 11.
pub const DIGITS: [u16; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
pub const MINUS: u16 = 12;
pub const EQUAL: u16 = 13;
pub const BACKSPACE: u16 = 14;
pub const TAB: u16 = 15;
pub const LEFT_BRACE: u16 = 26;
pub const RIGHT_BRACE: u16 = 27;
pub const ENTER: u16 = 28;
pub const LEFT_CTRL: u16 = 29;
pub const SEMICOLON: u16 = 39;
pub const APOSTROPHE: u16 = 40;
pub const GRAVE: u16 = 41;
pub const LEFT_SHIFT: u16 = 42;
pub const BACKSLASH: u16 = 43;
pub const COMMA: u16 = 51;
pub const DOT: u16 = 52;
pub const SLASH: u16 = 53;
pub const RIGHT_SHIFT: u16 = 54;
pub const LEFT_ALT: u16 = 56;
pub const SPACE: u16 = 57;
pub const CAPS_LOCK: u16 = 58;
/// F1 to F10 are 59 to 68; F11 and F12 are 87 and 88.
pub const F1: u16 = 59;
pub const F11: u16 = 87;
pub const F12: u16 = 88;
pub const RIGHT_CTRL: u16 = 97;
pub const RIGHT_ALT: u16 = 100;
pub const HOME: u16 = 102;
pub const UP: u16 = 103;
pub const PAGE_UP: u16 = 104;
pub const LEFT: u16 = 105;
pub const RIGHT: u16 = 106;
pub const END: u16 = 107;
pub const DOWN: u16 = 108;
pub const PAGE_DOWN: u16 = 109;
pub const INSERT: u16 = 110;
pub const DELETE: u16 = 111;
pub const LEFT_META: u16 = 125;
pub const RIGHT_META: u16 = 126;

/// Each letter's key, `a` to `z`.
pub const LETTERS: [u16; 26] = [30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44];

/// Which modifiers are held, as bits.
pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const META: u8 = 8;

/// The modifier a key is, if it's one.
pub fn modifier(code: u16) -> u8 {
    match code {
        LEFT_SHIFT | RIGHT_SHIFT => SHIFT,
        LEFT_CTRL | RIGHT_CTRL => CTRL,
        LEFT_ALT | RIGHT_ALT => ALT,
        LEFT_META | RIGHT_META => META,
        _ => 0,
    }
}

/// Keys that type a character, with it unshifted and shifted.
const TYPED: [(u16, char, char); 14] = [
    (MINUS, '-', '_'),
    (EQUAL, '=', '+'),
    (LEFT_BRACE, '[', '{'),
    (RIGHT_BRACE, ']', '}'),
    (SEMICOLON, ';', ':'),
    (APOSTROPHE, '\'', '"'),
    (GRAVE, '`', '~'),
    (BACKSLASH, '\\', '|'),
    (COMMA, ',', '<'),
    (DOT, '.', '>'),
    (SLASH, '/', '?'),
    (SPACE, ' ', ' '),
    (TAB, '\t', '\t'),
    (ENTER, '\r', '\r'),
];

const SHIFTED_DIGITS: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];

/// What a key types with these modifiers, on a US keyboard: none for keys that type nothing.
pub fn text(code: u16, modifiers: u8) -> Option<char> {
    let shift = modifiers & SHIFT != 0;
    if let Some(i) = LETTERS.iter().position(|&k| k == code) {
        let c = (b'a' + i as u8) as char;
        if modifiers & CTRL != 0 {
            return Some(char::from(i as u8 + 1));
        }
        return Some(if shift { c.to_ascii_uppercase() } else { c });
    }
    if let Some(d) = DIGITS.iter().position(|&k| k == code) {
        return Some(if shift { SHIFTED_DIGITS[d] } else { (b'0' + d as u8) as char });
    }
    match code {
        BACKSPACE => Some('\u{8}'),
        ESC => Some('\u{1b}'),
        _ => TYPED.iter().find(|t| t.0 == code).map(|t| if shift { t.2 } else { t.1 }),
    }
}

/// The key and whether it needs shift, to type `c` on a US keyboard.
pub fn for_char(c: char) -> Option<(u16, bool)> {
    if c.is_ascii_lowercase() {
        return Some((LETTERS[(c as u8 - b'a') as usize], false));
    }
    if c.is_ascii_uppercase() {
        return Some((LETTERS[(c as u8 - b'A') as usize], true));
    }
    if c.is_ascii_digit() {
        return Some((DIGITS[(c as u8 - b'0') as usize], false));
    }
    if let Some(d) = SHIFTED_DIGITS.iter().position(|&s| s == c) {
        return Some((DIGITS[d], true));
    }
    match c {
        '\n' => Some((ENTER, false)),
        '\u{8}' => Some((BACKSPACE, false)),
        _ => TYPED.iter().find_map(|t| if t.1 == c { Some((t.0, false)) } else if t.2 == c { Some((t.0, true)) } else { None }),
    }
}

/// A key by its W3C `KeyboardEvent.code` (`KeyA`, `Digit1`, `Enter`, `ShiftLeft`, `ArrowUp`, `F5`, `Numpad7`, ...): the
/// physical key, whatever the layout, as browsers and most toolkits name it. What an app that embeds the desktop
/// (`view`) sends its keys as.
pub fn by_code(code: &str) -> Option<u16> {
    if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
        let c = letter.chars().next()?.to_ascii_lowercase();
        return c.is_ascii_lowercase().then(|| LETTERS[(c as u8 - b'a') as usize]);
    }
    if let Some(digit) = code.strip_prefix("Digit").and_then(|d| d.parse::<usize>().ok()) {
        return DIGITS.get(digit).copied();
    }
    if let Some(f) = code.strip_prefix('F').and_then(|d| d.parse::<u16>().ok()) {
        return match f {
            1..=10 => Some(F1 + f - 1),
            11 => Some(F11),
            12 => Some(F12),
            _ => None,
        };
    }
    Some(match code {
        "Escape" => ESC,
        "Minus" => MINUS,
        "Equal" => EQUAL,
        "Backspace" => BACKSPACE,
        "Tab" => TAB,
        "BracketLeft" => LEFT_BRACE,
        "BracketRight" => RIGHT_BRACE,
        "Enter" => ENTER,
        "ControlLeft" => LEFT_CTRL,
        "ControlRight" => RIGHT_CTRL,
        "Semicolon" => SEMICOLON,
        "Quote" => APOSTROPHE,
        "Backquote" => GRAVE,
        "ShiftLeft" => LEFT_SHIFT,
        "ShiftRight" => RIGHT_SHIFT,
        "Backslash" | "IntlBackslash" => BACKSLASH,
        "Comma" => COMMA,
        "Period" => DOT,
        "Slash" => SLASH,
        "AltLeft" => LEFT_ALT,
        "AltRight" => RIGHT_ALT,
        "Space" => SPACE,
        "CapsLock" => CAPS_LOCK,
        "MetaLeft" | "OSLeft" => LEFT_META,
        "MetaRight" | "OSRight" => RIGHT_META,
        "Home" => HOME,
        "End" => END,
        "PageUp" => PAGE_UP,
        "PageDown" => PAGE_DOWN,
        "Insert" => INSERT,
        "Delete" => DELETE,
        "ArrowUp" => UP,
        "ArrowDown" => DOWN,
        "ArrowLeft" => LEFT,
        "ArrowRight" => RIGHT,
        // The keypad, as Linux numbers it.
        "Numpad7" => 71,
        "Numpad8" => 72,
        "Numpad9" => 73,
        "NumpadSubtract" => 74,
        "Numpad4" => 75,
        "Numpad5" => 76,
        "Numpad6" => 77,
        "NumpadAdd" => 78,
        "Numpad1" => 79,
        "Numpad2" => 80,
        "Numpad3" => 81,
        "Numpad0" => 82,
        "NumpadDecimal" => 83,
        "NumpadEnter" => 96,
        "NumpadDivide" => 98,
        "NumpadMultiply" => 55,
        "NumLock" => 69,
        "ScrollLock" => 70,
        "PrintScreen" => 99,
        "Pause" => 119,
        "ContextMenu" => 127,
        _ => return None,
    })
}

/// A key by its name, as `carbon display key` takes it: a letter or digit, `f1` to `f12`, or a name (`enter`, `esc`,
/// `tab`, `space`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, the arrows, `ctrl`, `shift`,
/// `alt`, `meta`, and the punctuation keys by name), whatever its case.
pub fn by_name(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphanumeric()
    {
        return for_char(c).map(|(code, _)| code);
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return match f {
            1..=10 => Some(F1 + f - 1),
            11 => Some(F11),
            12 => Some(F12),
            _ => None,
        };
    }
    Some(match n.as_str() {
        "enter" | "return" => ENTER,
        "esc" | "escape" => ESC,
        "tab" => TAB,
        "space" => SPACE,
        "backspace" => BACKSPACE,
        "delete" | "del" => DELETE,
        "insert" | "ins" => INSERT,
        "home" => HOME,
        "end" => END,
        "pageup" | "pgup" => PAGE_UP,
        "pagedown" | "pgdn" => PAGE_DOWN,
        "up" => UP,
        "down" => DOWN,
        "left" => LEFT,
        "right" => RIGHT,
        "ctrl" | "control" => LEFT_CTRL,
        "shift" => LEFT_SHIFT,
        "alt" | "option" => LEFT_ALT,
        "meta" | "super" | "win" | "cmd" => LEFT_META,
        "capslock" => CAPS_LOCK,
        "minus" => MINUS,
        "equal" => EQUAL,
        "comma" => COMMA,
        "period" | "dot" => DOT,
        "slash" => SLASH,
        "backslash" => BACKSLASH,
        "semicolon" => SEMICOLON,
        "apostrophe" | "quote" => APOSTROPHE,
        "grave" | "backtick" => GRAVE,
        "bracketleft" => LEFT_BRACE,
        "bracketright" => RIGHT_BRACE,
        _ => return None,
    })
}

#[cfg(test)]
mod code_tests {
    use super::*;

    #[test]
    fn w3c_codes_are_the_keys_a_us_keyboard_names() {
        assert_eq!(by_code("KeyA"), Some(30));
        assert_eq!(by_code("KeyZ"), by_name("z"));
        assert_eq!(by_code("Digit0"), Some(11));
        assert_eq!(by_code("Digit1"), Some(2));
        assert_eq!(by_code("F5"), Some(F1 + 4));
        assert_eq!(by_code("F12"), Some(F12));
        assert_eq!(by_code("ShiftLeft"), Some(LEFT_SHIFT));
        assert_eq!(by_code("MetaLeft"), Some(LEFT_META));
        assert_eq!(by_code("ArrowUp"), Some(UP));
        assert_eq!(by_code("Numpad7"), Some(71));
        assert_eq!(by_code("Keyboard"), None);
        assert_eq!(by_code("F13"), None);
    }
}
