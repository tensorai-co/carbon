//! The keyboard as a keymap: one description the servers derive theirs from. Today that's a US layout over the
//! desktop's evdev numbers, as an XKB text keymap (Wayland's `wl_keyboard.keymap`, and X11's `XKEYBOARD` to come):
//! fully compiled, needing nothing from the client's own XKB files, in the form libxkbcommon itself writes one.
//! Keycodes are evdev's plus 8, as XKB has them.

use std::fmt::Write;

/// Each key: its evdev number, its XKB name, its type (None for one level, or inferred from its symbols), and its
/// keysyms by level.
const KEYS: &[(u16, &str, Option<&str>, &[&str])] = &[
    (1, "ESC", None, &["Escape"]),
    (2, "AE01", Some("TWO_LEVEL"), &["1", "exclam"]),
    (3, "AE02", Some("TWO_LEVEL"), &["2", "at"]),
    (4, "AE03", Some("TWO_LEVEL"), &["3", "numbersign"]),
    (5, "AE04", Some("TWO_LEVEL"), &["4", "dollar"]),
    (6, "AE05", Some("TWO_LEVEL"), &["5", "percent"]),
    (7, "AE06", Some("TWO_LEVEL"), &["6", "asciicircum"]),
    (8, "AE07", Some("TWO_LEVEL"), &["7", "ampersand"]),
    (9, "AE08", Some("TWO_LEVEL"), &["8", "asterisk"]),
    (10, "AE09", Some("TWO_LEVEL"), &["9", "parenleft"]),
    (11, "AE10", Some("TWO_LEVEL"), &["0", "parenright"]),
    (12, "AE11", Some("TWO_LEVEL"), &["minus", "underscore"]),
    (13, "AE12", Some("TWO_LEVEL"), &["equal", "plus"]),
    (14, "BKSP", None, &["BackSpace"]),
    (15, "TAB", Some("TWO_LEVEL"), &["Tab", "ISO_Left_Tab"]),
    (16, "AD01", Some("ALPHABETIC"), &["q", "Q"]),
    (17, "AD02", Some("ALPHABETIC"), &["w", "W"]),
    (18, "AD03", Some("ALPHABETIC"), &["e", "E"]),
    (19, "AD04", Some("ALPHABETIC"), &["r", "R"]),
    (20, "AD05", Some("ALPHABETIC"), &["t", "T"]),
    (21, "AD06", Some("ALPHABETIC"), &["y", "Y"]),
    (22, "AD07", Some("ALPHABETIC"), &["u", "U"]),
    (23, "AD08", Some("ALPHABETIC"), &["i", "I"]),
    (24, "AD09", Some("ALPHABETIC"), &["o", "O"]),
    (25, "AD10", Some("ALPHABETIC"), &["p", "P"]),
    (26, "AD11", Some("TWO_LEVEL"), &["bracketleft", "braceleft"]),
    (27, "AD12", Some("TWO_LEVEL"), &["bracketright", "braceright"]),
    (28, "RTRN", None, &["Return"]),
    (29, "LCTL", None, &["Control_L"]),
    (30, "AC01", Some("ALPHABETIC"), &["a", "A"]),
    (31, "AC02", Some("ALPHABETIC"), &["s", "S"]),
    (32, "AC03", Some("ALPHABETIC"), &["d", "D"]),
    (33, "AC04", Some("ALPHABETIC"), &["f", "F"]),
    (34, "AC05", Some("ALPHABETIC"), &["g", "G"]),
    (35, "AC06", Some("ALPHABETIC"), &["h", "H"]),
    (36, "AC07", Some("ALPHABETIC"), &["j", "J"]),
    (37, "AC08", Some("ALPHABETIC"), &["k", "K"]),
    (38, "AC09", Some("ALPHABETIC"), &["l", "L"]),
    (39, "AC10", Some("TWO_LEVEL"), &["semicolon", "colon"]),
    (40, "AC11", Some("TWO_LEVEL"), &["apostrophe", "quotedbl"]),
    (41, "TLDE", Some("TWO_LEVEL"), &["grave", "asciitilde"]),
    (42, "LFSH", None, &["Shift_L"]),
    (43, "BKSL", Some("TWO_LEVEL"), &["backslash", "bar"]),
    (44, "AB01", Some("ALPHABETIC"), &["z", "Z"]),
    (45, "AB02", Some("ALPHABETIC"), &["x", "X"]),
    (46, "AB03", Some("ALPHABETIC"), &["c", "C"]),
    (47, "AB04", Some("ALPHABETIC"), &["v", "V"]),
    (48, "AB05", Some("ALPHABETIC"), &["b", "B"]),
    (49, "AB06", Some("ALPHABETIC"), &["n", "N"]),
    (50, "AB07", Some("ALPHABETIC"), &["m", "M"]),
    (51, "AB08", Some("TWO_LEVEL"), &["comma", "less"]),
    (52, "AB09", Some("TWO_LEVEL"), &["period", "greater"]),
    (53, "AB10", Some("TWO_LEVEL"), &["slash", "question"]),
    (54, "RTSH", None, &["Shift_R"]),
    (55, "KPMU", None, &["KP_Multiply"]),
    (56, "LALT", None, &["Alt_L"]),
    (57, "SPCE", None, &["space"]),
    (58, "CAPS", None, &["Caps_Lock"]),
    (59, "FK01", None, &["F1"]),
    (60, "FK02", None, &["F2"]),
    (61, "FK03", None, &["F3"]),
    (62, "FK04", None, &["F4"]),
    (63, "FK05", None, &["F5"]),
    (64, "FK06", None, &["F6"]),
    (65, "FK07", None, &["F7"]),
    (66, "FK08", None, &["F8"]),
    (67, "FK09", None, &["F9"]),
    (68, "FK10", None, &["F10"]),
    (69, "NMLK", None, &["Num_Lock"]),
    (70, "SCLK", None, &["Scroll_Lock"]),
    (71, "KP7", Some("KEYPAD"), &["KP_Home", "KP_7"]),
    (72, "KP8", Some("KEYPAD"), &["KP_Up", "KP_8"]),
    (73, "KP9", Some("KEYPAD"), &["KP_Prior", "KP_9"]),
    (74, "KPSU", None, &["KP_Subtract"]),
    (75, "KP4", Some("KEYPAD"), &["KP_Left", "KP_4"]),
    (76, "KP5", Some("KEYPAD"), &["KP_Begin", "KP_5"]),
    (77, "KP6", Some("KEYPAD"), &["KP_Right", "KP_6"]),
    (78, "KPAD", None, &["KP_Add"]),
    (79, "KP1", Some("KEYPAD"), &["KP_End", "KP_1"]),
    (80, "KP2", Some("KEYPAD"), &["KP_Down", "KP_2"]),
    (81, "KP3", Some("KEYPAD"), &["KP_Next", "KP_3"]),
    (82, "KP0", Some("KEYPAD"), &["KP_Insert", "KP_0"]),
    (83, "KPDL", Some("KEYPAD"), &["KP_Delete", "KP_Decimal"]),
    (86, "LSGT", Some("TWO_LEVEL"), &["less", "greater"]),
    (87, "FK11", None, &["F11"]),
    (88, "FK12", None, &["F12"]),
    (96, "KPEN", None, &["KP_Enter"]),
    (97, "RCTL", None, &["Control_R"]),
    (98, "KPDV", None, &["KP_Divide"]),
    (99, "PRSC", None, &["Print"]),
    (100, "RALT", None, &["Alt_R"]),
    (102, "HOME", None, &["Home"]),
    (103, "UP", None, &["Up"]),
    (104, "PGUP", None, &["Prior"]),
    (105, "LEFT", None, &["Left"]),
    (106, "RGHT", None, &["Right"]),
    (107, "END", None, &["End"]),
    (108, "DOWN", None, &["Down"]),
    (109, "PGDN", None, &["Next"]),
    (110, "INS", None, &["Insert"]),
    (111, "DELE", None, &["Delete"]),
    (117, "KPEQ", None, &["KP_Equal"]),
    (119, "PAUS", None, &["Pause"]),
    (125, "LWIN", None, &["Super_L"]),
    (126, "RWIN", None, &["Super_R"]),
    (127, "COMP", None, &["Menu"]),
    (183, "FK13", None, &["F13"]),
    (184, "FK14", None, &["F14"]),
    (185, "FK15", None, &["F15"]),
];

/// XKB's real modifiers, as a keymap's `modifiers` events give them.
pub mod mods {
    pub const SHIFT: u32 = 1;
    pub const LOCK: u32 = 2;
    pub const CONTROL: u32 = 4;
    /// Alt.
    pub const MOD1: u32 = 8;
    /// Num Lock.
    pub const MOD2: u32 = 16;
    /// Super (the Windows key, the Mac's Command).
    pub const MOD4: u32 = 64;
}

/// The desktop's modifiers held (`keys::SHIFT` and the rest), as XKB's depressed modifiers.
pub fn xkb_mods(held: u8) -> u32 {
    use crate::keys;
    [(keys::SHIFT, mods::SHIFT), (keys::CTRL, mods::CONTROL), (keys::ALT, mods::MOD1), (keys::META, mods::MOD4)]
        .iter()
        .filter(|(k, _)| held & k != 0)
        .fold(0, |m, (_, x)| m | x)
}

/// The US keymap, as XKB text.
pub fn xkb_us() -> String {
    let mut s = String::from("xkb_keymap {\nxkb_keycodes \"carbon\" {\n\tminimum = 8;\n\tmaximum = 255;\n");
    for (code, name, _, _) in KEYS {
        let _ = writeln!(s, "\t<{name}> = {};", code + 8);
    }
    s.push_str("\tindicator 1 = \"Caps Lock\";\n\tindicator 2 = \"Num Lock\";\n\tindicator 3 = \"Scroll Lock\";\n};\n\n");
    s.push_str(TYPES);
    s.push_str(COMPAT);
    s.push_str("xkb_symbols \"carbon\" {\n\tname[1]=\"English (US)\";\n\n");
    for (_, name, kind, syms) in KEYS {
        let list = syms.join(", ");
        match kind {
            None => {
                let _ = writeln!(s, "\tkey <{name}> {{\t[ {list} ] }};");
            }
            Some(kind) => {
                let _ = writeln!(s, "\tkey <{name}> {{\n\t\ttype= \"{kind}\",\n\t\tsymbols[1]= [ {list} ]\n\t}};");
            }
        }
    }
    s.push_str("\tmodifier_map Shift { <LFSH>, <RTSH> };\n\tmodifier_map Lock { <CAPS> };\n\tmodifier_map Control { <LCTL>, <RCTL> };\n");
    s.push_str("\tmodifier_map Mod1 { <LALT>, <RALT> };\n\tmodifier_map Mod2 { <NMLK> };\n\tmodifier_map Mod4 { <LWIN>, <RWIN> };\n};\n\n};\n");
    s
}

const TYPES: &str = "xkb_types \"carbon\" {
\tvirtual_modifiers NumLock,Alt,Super;

\ttype \"ONE_LEVEL\" {
\t\tmodifiers= none;
\t\tlevel_name[1]= \"Any\";
\t};
\ttype \"TWO_LEVEL\" {
\t\tmodifiers= Shift;
\t\tmap[Shift]= 2;
\t\tlevel_name[1]= \"Base\";
\t\tlevel_name[2]= \"Shift\";
\t};
\ttype \"ALPHABETIC\" {
\t\tmodifiers= Shift+Lock;
\t\tmap[Shift]= 2;
\t\tmap[Lock]= 2;
\t\tlevel_name[1]= \"Base\";
\t\tlevel_name[2]= \"Caps\";
\t};
\ttype \"KEYPAD\" {
\t\tmodifiers= Shift+NumLock;
\t\tmap[Shift]= 2;
\t\tmap[NumLock]= 2;
\t\tlevel_name[1]= \"Base\";
\t\tlevel_name[2]= \"Number\";
\t};
};

";

const COMPAT: &str = "xkb_compatibility \"carbon\" {
\tvirtual_modifiers NumLock,Alt,Super;

\tinterpret.useModMapMods= AnyLevel;
\tinterpret.repeat= False;
\tinterpret Num_Lock+AnyOf(all) {
\t\tvirtualModifier= NumLock;
\t\taction= LockMods(modifiers=NumLock);
\t};
\tinterpret Alt_L+AnyOf(all) {
\t\tvirtualModifier= Alt;
\t\taction= SetMods(modifiers=modMapMods,clearLocks);
\t};
\tinterpret Alt_R+AnyOf(all) {
\t\tvirtualModifier= Alt;
\t\taction= SetMods(modifiers=modMapMods,clearLocks);
\t};
\tinterpret Super_L+AnyOf(all) {
\t\tvirtualModifier= Super;
\t\taction= SetMods(modifiers=modMapMods,clearLocks);
\t};
\tinterpret Super_R+AnyOf(all) {
\t\tvirtualModifier= Super;
\t\taction= SetMods(modifiers=modMapMods,clearLocks);
\t};
\tinterpret Caps_Lock+AnyOfOrNone(all) {
\t\taction= LockMods(modifiers=Lock);
\t};
\tinterpret Any+AnyOf(all) {
\t\taction= SetMods(modifiers=modMapMods,clearLocks);
\t};
\tindicator \"Caps Lock\" {
\t\twhichModState= locked;
\t\tmodifiers= Lock;
\t};
\tindicator \"Num Lock\" {
\t\twhichModState= locked;
\t\tmodifiers= NumLock;
\t};
};

";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    #[test]
    fn the_keymap_names_every_key_the_desktop_sends() {
        let map = xkb_us();
        assert!(map.starts_with("xkb_keymap {") && map.trim_end().ends_with("};"));
        assert_eq!(map.matches('{').count(), map.matches('}').count());
        // Each key's code declared once, and its symbols once.
        for (code, name, _, _) in KEYS {
            assert_eq!(map.matches(&format!("\t<{name}> = {};", code + 8)).count(), 1, "{name}");
            assert_eq!(map.matches(&format!("key <{name}> {{")).count(), 1, "{name}");
        }
        // The keys the desktop's own tables have are all there.
        for code in keys::LETTERS.iter().chain(&keys::DIGITS).chain(&[keys::ENTER, keys::ESC, keys::LEFT_SHIFT, keys::LEFT_CTRL, keys::LEFT_ALT, keys::LEFT_META, keys::F12, keys::DELETE, keys::UP]) {
            assert!(KEYS.iter().any(|k| k.0 == *code), "evdev {code}");
        }
        assert!(map.contains("\tkey <AC01> {\n\t\ttype= \"ALPHABETIC\",\n\t\tsymbols[1]= [ a, A ]\n\t};"));
        assert!(map.contains("\t<AC01> = 38;"));
        assert_eq!(xkb_mods(keys::SHIFT | keys::CTRL), mods::SHIFT | mods::CONTROL);
    }
}

/// Each key's keysym names by level: what an X server's core keyboard mapping (`GetKeyboardMapping`) is made from.
pub fn levels() -> impl Iterator<Item = (u16, &'static [&'static str])> {
    KEYS.iter().map(|(code, _, _, syms)| (*code, *syms))
}

/// A keysym's value, from its name as `keysymdef.h` has it (the names `KEYS` uses): Latin-1's are their characters'
/// codes, the rest X11's own. 0 (`NoSymbol`) for a name that isn't one.
pub fn keysym(name: &str) -> u32 {
    const NAMED: &[(&str, u32)] = &[
        ("space", 0x20), ("exclam", 0x21), ("quotedbl", 0x22), ("numbersign", 0x23), ("dollar", 0x24), ("percent", 0x25),
        ("ampersand", 0x26), ("apostrophe", 0x27), ("parenleft", 0x28), ("parenright", 0x29), ("asterisk", 0x2a), ("plus", 0x2b),
        ("comma", 0x2c), ("minus", 0x2d), ("period", 0x2e), ("slash", 0x2f), ("colon", 0x3a), ("semicolon", 0x3b), ("less", 0x3c),
        ("equal", 0x3d), ("greater", 0x3e), ("question", 0x3f), ("at", 0x40), ("bracketleft", 0x5b), ("backslash", 0x5c),
        ("bracketright", 0x5d), ("asciicircum", 0x5e), ("underscore", 0x5f), ("grave", 0x60), ("braceleft", 0x7b), ("bar", 0x7c),
        ("braceright", 0x7d), ("asciitilde", 0x7e),
        ("BackSpace", 0xff08), ("Tab", 0xff09), ("Return", 0xff0d), ("Pause", 0xff13), ("Scroll_Lock", 0xff14), ("Escape", 0xff1b),
        ("Home", 0xff50), ("Left", 0xff51), ("Up", 0xff52), ("Right", 0xff53), ("Down", 0xff54), ("Prior", 0xff55), ("Next", 0xff56),
        ("End", 0xff57), ("Print", 0xff61), ("Insert", 0xff63), ("Menu", 0xff67), ("Num_Lock", 0xff7f), ("KP_Enter", 0xff8d),
        ("KP_Home", 0xff95), ("KP_Left", 0xff96), ("KP_Up", 0xff97), ("KP_Right", 0xff98), ("KP_Down", 0xff99), ("KP_Prior", 0xff9a),
        ("KP_Next", 0xff9b), ("KP_End", 0xff9c), ("KP_Begin", 0xff9d), ("KP_Insert", 0xff9e), ("KP_Delete", 0xff9f),
        ("KP_Multiply", 0xffaa), ("KP_Add", 0xffab), ("KP_Subtract", 0xffad), ("KP_Decimal", 0xffae), ("KP_Divide", 0xffaf),
        ("KP_Equal", 0xffbd), ("Shift_L", 0xffe1), ("Shift_R", 0xffe2), ("Control_L", 0xffe3), ("Control_R", 0xffe4),
        ("Caps_Lock", 0xffe5), ("Alt_L", 0xffe9), ("Alt_R", 0xffea), ("Super_L", 0xffeb), ("Super_R", 0xffec), ("Delete", 0xffff),
        ("ISO_Left_Tab", 0xfe20),
    ];
    if let Some(&(_, v)) = NAMED.iter().find(|(n, _)| *n == name) {
        return v;
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => c as u32,
        _ => {
            if let Some(n) = name.strip_prefix("KP_").and_then(|d| d.parse::<u32>().ok()).filter(|n| *n <= 9) {
                return 0xffb0 + n;
            }
            match name.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()) {
                Some(n @ 1..=35) => 0xffbe + n - 1,
                _ => 0,
            }
        }
    }
}

#[cfg(test)]
mod keysym_tests {
    #[test]
    fn every_key_has_its_keysyms() {
        for (code, syms) in super::levels() {
            for s in syms {
                assert_ne!(super::keysym(s), 0, "key {code}'s {s}");
            }
        }
        assert_eq!(super::keysym("a"), 0x61);
        assert_eq!(super::keysym("F12"), 0xffc9);
        assert_eq!(super::keysym("KP_7"), 0xffb7);
    }
}
