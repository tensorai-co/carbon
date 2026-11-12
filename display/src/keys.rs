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
