//! XKEYBOARD (1.0): the keyboard as XKB describes it, made from the desktop's keymap (`display::keymap`, the one
//! the Wayland server sends): the four canonical key types, each key's keysyms by level, actions for the modifier
//! keys, the modifier map, names (keycodes, types and their levels, indicators, the group, each key's), the compat
//! and indicator maps, controls and state; and `StateNotify` as the modifiers change, which is how libxkbcommon's X11
//! clients and Xlib keep their keyboard state. One keyboard, the core one, device 3.

use display::keymap;

use super::wire::{Ev, Reply, Req, err};
use super::Server;
use super::wire::ev;

pub const XKEYBOARD: u8 = 133;
pub const XKB_EVENT: u8 = 66;
pub const XKB_ERROR: u8 = 134;
/// The core keyboard's device ID.
const DEVICE: u8 = 3;
const MIN_KEY: u8 = 8;
const MAX_KEY: u8 = 255;
const KEYS: usize = (MAX_KEY - MIN_KEY) as usize + 1;

/// `SelectEvents`' StateNotify.
pub const STATE_NOTIFY: u16 = 4;

/// Real modifiers.
const SHIFT: u8 = 1;
const LOCK: u8 = 2;
const CONTROL: u8 = 4;
const MOD1: u8 = 8;
const MOD2: u8 = 16;
const MOD4: u8 = 64;

/// A key type: its name, modifier mask, level names, and map entries (the modifiers, and the level they choose).
type KeyType = (&'static str, u8, &'static [&'static str], &'static [(u8, u8)]);

/// The key types, in XKB's canonical order.
const TYPES: [KeyType; 4] = [
    ("ONE_LEVEL", 0, &["Any"], &[]),
    ("TWO_LEVEL", SHIFT, &["Base", "Shift"], &[(SHIFT, 1)]),
    ("ALPHABETIC", SHIFT | LOCK, &["Base", "Caps"], &[(SHIFT, 1), (LOCK, 1)]),
    ("KEYPAD", SHIFT | MOD2, &["Base", "Number"], &[(MOD2, 1)]),
];

/// The indicators: Caps Lock, Num Lock and Scroll Lock, and the modifier each shows when it's locked.
const INDICATORS: [(&str, u8); 3] = [("Caps Lock", LOCK), ("Num Lock", MOD2), ("Scroll Lock", 0)];

/// One key as XKB has it.
#[derive(Clone, Default)]
pub struct Key {
    name: [u8; 4],
    kind: u8,
    syms: Vec<u32>,
    mods: u8,
    action: Option<[u8; 8]>,
}

/// The keyboard: every keycode from 8 to 255, most of them with nothing on them.
pub struct Keyboard {
    keys: Vec<Key>,
}

impl Keyboard {
    pub fn new() -> Keyboard {
        let mut keys = vec![Key::default(); KEYS];
        for (code, name, kind, syms) in keymap::keys() {
            let Some(at) = (code as usize + 8).checked_sub(MIN_KEY as usize).filter(|&i| i < KEYS) else { continue };
            let kind = TYPES.iter().position(|t| t.0 == kind).unwrap_or(0) as u8;
            let levels = TYPES[kind as usize].2.len();
            let mut list: Vec<u32> = syms.iter().map(|s| keymap::keysym(s)).collect();
            list.resize(levels, *list.last().unwrap_or(&0));
            let mods = match syms[0] {
                "Shift_L" | "Shift_R" => SHIFT,
                "Caps_Lock" => LOCK,
                "Control_L" | "Control_R" => CONTROL,
                "Alt_L" | "Alt_R" => MOD1,
                "Num_Lock" => MOD2,
                "Super_L" | "Super_R" => MOD4,
                _ => 0,
            };
            // The modifier keys set their modifier while they're held (the locks lock it), from the modifier map.
            let action = (mods != 0).then_some(if mods == LOCK || mods == MOD2 { [3, 4, 0, 0, 0, 0, 0, 0] } else { [1, 5, 0, 0, 0, 0, 0, 0] });
            let mut n = [0u8; 4];
            for (i, b) in name.bytes().take(4).enumerate() {
                n[i] = b;
            }
            keys[at] = Key { name: n, kind, syms: list, mods, action };
        }
        Keyboard { keys }
    }
}

impl Server {
    pub(super) fn xkb(&mut self, ci: u32, r: Req) {
        match r.b1() {
            // UseExtension: 1.0.
            0 => {
                if let Some(c) = self.clients.get_mut(&ci) {
                    c.xkb = true;
                }
                let mut rep = Reply::new(1);
                rep.u16(1).u16(0);
                self.reply(ci, rep);
            }
            // SelectEvents: which events it wants (their details aren't looked at).
            1 => {
                let (affect, clear, all) = (r.u16(6), r.u16(8), r.u16(10));
                if let Some(c) = self.clients.get_mut(&ci) {
                    c.xkb_events = (c.xkb_events & !affect) | (affect & !clear & (all | affect));
                }
            }
            // Bell, LatchLockState, SetControls, SetMap, SetCompatMap, SetIndicatorMap, SetNamedIndicator, SetNames,
            // SetDeviceInfo: the keyboard is the desktop's, and stays as it is.
            3 | 5 | 7 | 9 | 11 | 14 | 16 | 18 | 25 => {}
            4 => self.xkb_state(ci),
            6 => self.xkb_controls(ci),
            8 => self.xkb_map(ci, r.u16(6) | r.u16(8), r.u16(18)),
            10 => {
                let groups = r.u8(6);
                let mut rep = Reply::new(DEVICE);
                rep.u8(groups).skip(1).u16(0).u16(0).u16(0).skip(16);
                for _ in 0..groups.count_ones() {
                    rep.u32(0);
                }
                self.reply(ci, rep);
            }
            12 => {
                let mut rep = Reply::new(DEVICE);
                rep.u32(self.indicator_state());
                self.reply(ci, rep);
            }
            13 => {
                let which = r.u32(8);
                let mut rep = Reply::new(DEVICE);
                rep.u32(which).u32(0b111).u8(which.count_ones() as u8).skip(15);
                for i in 0..32 {
                    if which & (1 << i) != 0 {
                        let mods = INDICATORS.get(i).map_or(0, |ind| ind.1);
                        // Shown from the locked modifiers.
                        rep.u8(0).u8(0).u8(0).u8(if mods != 0 { 4 } else { 0 }).u8(mods).u8(mods).u16(0).u32(0);
                    }
                }
                self.reply(ci, rep);
            }
            15 => {
                let name = r.u32(12);
                let found = INDICATORS.iter().position(|ind| self.atom_name(name) == Some(ind.0));
                let state = self.indicator_state();
                let mut rep = Reply::new(DEVICE);
                rep.u32(name);
                match found {
                    Some(i) => {
                        let mods = INDICATORS[i].1;
                        rep.u8(1).u8(u8::from(state & (1 << i) != 0)).u8(1).u8(i as u8);
                        rep.u8(0).u8(0).u8(0).u8(if mods != 0 { 4 } else { 0 }).u8(mods).u8(mods).u16(0).u32(0).u8(1);
                    }
                    None => {
                        rep.skip(17);
                    }
                }
                self.reply(ci, rep);
            }
            17 => self.xkb_names(ci, r.u32(8)),
            // PerClientFlags: kept, and said back.
            21 => {
                let (change, value) = (r.u32(8), r.u32(12));
                let supported = 0b1111;
                let flags = self.clients.get_mut(&ci).map_or(0, |c| {
                    c.xkb_flags = (c.xkb_flags & !change) | (value & change & supported);
                    c.xkb_flags
                });
                let mut rep = Reply::new(DEVICE);
                rep.u32(supported).u32(flags).u32(0).u32(0);
                self.reply(ci, rep);
            }
            24 => {
                let name = b"carbon keyboard";
                let kind = self.atom("KEYBOARD");
                let mut rep = Reply::new(DEVICE);
                rep.u16(0).u16(0).u16(0).u16(0).u8(0).u8(0).u8(0).u8(0).u8(0).u8(1).u16(0x100).u16(0x100).skip(2).u32(kind);
                rep.u16(name.len() as u16).bytes(name);
                self.reply(ci, rep);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    /// The indicators lit: Caps Lock's.
    fn indicator_state(&self) -> u32 {
        u32::from(self.caps)
    }

    /// The modifiers as XKB's state: held (base), locked, and both (effective).
    fn xkb_mods(&self) -> (u8, u8, u8) {
        let locked = if self.caps { LOCK } else { 0 };
        let base = (self.mods as u8) & !LOCK;
        (base, locked, base | locked)
    }

    fn xkb_state(&mut self, ci: u32) {
        let (base, locked, mods) = self.xkb_mods();
        let buttons = super::button_state(self.buttons);
        let mut rep = Reply::new(DEVICE);
        rep.u8(mods).u8(base).u8(0).u8(locked).u8(0).u8(0).i16(0).i16(0).u8(mods).u8(mods).u8(mods).u8(mods).u8(mods).skip(1).u16(buttons);
        self.reply(ci, rep);
    }

    fn xkb_controls(&mut self, ci: u32) {
        let repeat = display::desktop::lock(&self.desktop).key_repeat();
        let interval = 1000u32.checked_div(repeat.rate).unwrap_or(0) as u16;
        let mut rep = Reply::new(DEVICE);
        // The default button, one group, wrapping; no internal or ignored-lock modifiers.
        rep.u8(1).u8(1).u8(0).u8(0).u8(0).u8(0).u8(0).skip(1).u16(0).u16(0);
        rep.u16(repeat.delay as u16).u16(interval).u16(300).u16(300).u16(160).u16(40).u16(30).u16(10).i16(0);
        rep.u16(0).u16(0).u16(0).u16(0).skip(2).u32(0).u32(0);
        // Enabled: repeating keys.
        rep.u32(1).bytes(&[0xff; 32]);
        self.reply(ci, rep);
    }

    /// GetMap: every part asked for, for every key (whatever range was asked).
    fn xkb_map(&mut self, ci: u32, parts: u16, _vmods: u16) {
        const TYPES_PART: u16 = 1;
        const SYMS: u16 = 2;
        const MODMAP: u16 = 4;
        const EXPLICIT: u16 = 8;
        const ACTIONS: u16 = 16;
        const BEHAVIORS: u16 = 32;
        const VMODS: u16 = 64;
        const VMODMAP: u16 = 128;
        let kb = &self.keyboard.keys;
        let present = parts & (TYPES_PART | SYMS | MODMAP | EXPLICIT | ACTIONS | BEHAVIORS | VMODS | VMODMAP);
        let has = |p: u16| present & p != 0;
        let total_syms: usize = kb.iter().map(|k| k.syms.len()).sum();
        let actions: Vec<(usize, [u8; 8])> = kb.iter().enumerate().filter_map(|(i, k)| k.action.map(|a| (i, a))).collect();
        let modmap: Vec<(u8, u8)> = kb.iter().enumerate().filter(|(_, k)| k.mods != 0).map(|(i, k)| (MIN_KEY + i as u8, k.mods)).collect();
        let n = KEYS as u8;
        let range = |on: bool| if on { (MIN_KEY, n) } else { (0, 0) };
        let mut rep = Reply::new(DEVICE);
        rep.skip(2).u8(MIN_KEY).u8(MAX_KEY).u16(present);
        let t = if has(TYPES_PART) { 4 } else { 0 };
        rep.u8(0).u8(t).u8(t);
        let (f, c) = range(has(SYMS));
        rep.u8(f).u16(if has(SYMS) { total_syms as u16 } else { 0 }).u8(c);
        let (f, c) = range(has(ACTIONS));
        rep.u8(f).u16(if has(ACTIONS) { actions.len() as u16 } else { 0 }).u8(c);
        let (f, c) = range(has(BEHAVIORS));
        rep.u8(f).u8(c).u8(0);
        let (f, c) = range(has(EXPLICIT));
        rep.u8(f).u8(c).u8(0);
        let (f, c) = range(has(MODMAP));
        rep.u8(f).u8(c).u8(if has(MODMAP) { modmap.len() as u8 } else { 0 });
        let (f, c) = range(has(VMODMAP));
        rep.u8(f).u8(c).u8(0);
        rep.skip(1).u16(0);
        if has(TYPES_PART) {
            for (_, mask, levels, entries) in TYPES {
                rep.u8(mask).u8(mask).u16(0).u8(levels.len() as u8).u8(entries.len() as u8).u8(0).skip(1);
                for &(mods, level) in entries {
                    rep.u8(1).u8(mods).u8(level).u8(mods).u16(0).skip(2);
                }
            }
        }
        if has(SYMS) {
            for k in kb {
                let groups = u8::from(!k.syms.is_empty());
                rep.u8(k.kind).u8(0).u8(0).u8(0).u8(groups).u8(k.syms.len() as u8).u16(k.syms.len() as u16);
                for &s in &k.syms {
                    rep.u32(s);
                }
            }
        }
        if has(ACTIONS) {
            for k in kb {
                rep.u8(u8::from(k.action.is_some()));
            }
            rep.skip(super::wire::pad(KEYS));
            for (_, a) in &actions {
                rep.bytes(a);
            }
        }
        if has(MODMAP) {
            for &(code, mods) in &modmap {
                rep.u8(code).u8(mods);
            }
            rep.skip(super::wire::pad(modmap.len() * 2));
        }
        self.reply(ci, rep);
    }

    /// GetNames: the names asked for.
    fn xkb_names(&mut self, ci: u32, which: u32) {
        const LEVEL_NAMES: u32 = 128;
        let which = which & (1 | 2 | 4 | 8 | 16 | 32 | 64 | LEVEL_NAMES | 256 | 512 | 1024 | 2048 | 4096);
        let has = |bit: u32| which & bit != 0;
        let names: Vec<u32> = ["evdev", "carbon", "pc+us", "pc+us", "complete", "complete"].iter().map(|n| self.atom(n)).collect();
        let types: Vec<u32> = TYPES.iter().map(|t| self.atom(t.0)).collect();
        let levels: Vec<Vec<u32>> = TYPES.iter().map(|t| t.2.iter().map(|l| self.atom(l)).collect()).collect();
        let indicators: Vec<u32> = INDICATORS.iter().map(|i| self.atom(i.0)).collect();
        let group = self.atom("English (US)");
        let n_levels: usize = TYPES.iter().map(|t| t.2.len()).sum();
        let mut rep = Reply::new(DEVICE);
        rep.u32(which).u8(MIN_KEY).u8(MAX_KEY).u8(TYPES.len() as u8).u8(if has(4096) { 1 } else { 0 }).u16(0);
        rep.u8(MIN_KEY).u8(KEYS as u8).u32(if has(256) { 0b111 } else { 0 }).u8(0).u8(0).u16(n_levels as u16).skip(4);
        for (bit, atom) in [1u32, 2, 4, 8, 16, 32].iter().zip(&names) {
            if has(*bit) {
                rep.u32(*atom);
            }
        }
        if has(64) {
            for t in &types {
                rep.u32(*t);
            }
        }
        if has(LEVEL_NAMES) {
            for l in &levels {
                rep.u8(l.len() as u8);
            }
            rep.skip(super::wire::pad(levels.len()));
            for a in levels.iter().flatten() {
                rep.u32(*a);
            }
        }
        if has(256) {
            for a in &indicators {
                rep.u32(*a);
            }
        }
        if has(4096) {
            rep.u32(group);
        }
        if has(512) {
            for k in &self.keyboard.keys {
                rep.bytes(&k.name);
            }
        }
        self.reply(ci, rep);
    }

    /// The modifiers changed (a key's): `StateNotify` to every client that asked for it.
    pub(super) fn xkb_state_changed(&mut self, old: (u8, u8, u8), keycode: u8, down: bool) {
        let (base, locked, mods) = self.xkb_mods();
        if (base, locked, mods) == old {
            return;
        }
        let changed = (u16::from(old.0 != base) * 2) | (u16::from(old.1 != locked) * 8) | u16::from(old.2 != mods);
        let buttons = super::button_state(self.buttons);
        let e = Ev::new(XKB_EVENT)
            .u8(1, 2)
            .u32(4, self.time())
            .u8(8, DEVICE)
            .u8(9, mods)
            .u8(10, base)
            .u8(11, 0)
            .u8(12, locked)
            .u8(19, mods)
            .u8(20, mods)
            .u8(21, mods)
            .u8(22, mods)
            .u8(23, mods)
            .u16(24, buttons)
            .u16(26, changed)
            .u8(28, keycode)
            .u8(29, if down { ev::KEY_PRESS } else { ev::KEY_RELEASE });
        let to: Vec<u32> = self.clients.iter().filter(|(_, c)| c.xkb_events & STATE_NOTIFY != 0).map(|(&id, _)| id).collect();
        for c in to {
            self.event(c, e);
        }
    }

    /// XKB's view of the modifiers now, to compare with after a key.
    pub(super) fn xkb_before(&self) -> (u8, u8, u8) {
        self.xkb_mods()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboard_has_its_keys_and_modifiers() {
        let kb = Keyboard::new();
        // KEY_A, keycode 38: alphabetic, a and A.
        let a = &kb.keys[38 - MIN_KEY as usize];
        assert_eq!((a.kind, a.syms.clone(), &a.name), (2, vec![0x61, 0x41], b"AC01"));
        // Shift_L (evdev 42, keycode 50): one level, Shift in the modifier map, an action that sets it.
        let shift = &kb.keys[50 - MIN_KEY as usize];
        assert_eq!((shift.kind, shift.mods, shift.action.map(|a| a[0])), (0, SHIFT, Some(1)));
        // Every key's keysyms are as many as its type's levels.
        assert!(kb.keys.iter().all(|k| k.syms.is_empty() || k.syms.len() == TYPES[k.kind as usize].2.len()));
    }
}
