//! RANDR (1.5): the desktop's output as X11 sees it: one CRTC driving one connected, primary output in one mode, the
//! output's size and refresh, and one monitor; the 1.0 screen configuration too; gamma ramps (linear, kept as set);
//! and the change events (`RRScreenChangeNotify`, and `RRNotify`'s CRTC and output changes) when the output changes:
//! a host window resized, `carbon display resize`. Nothing a client sets changes the output: the desktop decides it.

use super::wire::{Ev, Reply, Req, err};
use super::{ROOT, Server};

pub const RANDR: u8 = 134;
pub const RANDR_EVENT: u8 = 67;
/// RANDR's errors from here: BadOutput, BadCrtc, BadMode, BadProvider.
pub const RANDR_ERROR: u8 = 135;

const CRTC: u32 = 0x40;
const OUTPUT: u32 = 0x41;
/// Each mode's ID: one more each time the output changes, so a client sees a new mode as one.
const MODES: u32 = 0x1000;

/// `SelectInput`'s masks.
const SCREEN_CHANGE: u16 = 1;
const CRTC_CHANGE: u16 = 2;
const OUTPUT_CHANGE: u16 = 4;

/// Gamma ramps' size.
const GAMMA: usize = 256;

impl Server {
    fn mode_id(&self) -> u32 {
        MODES + self.output_generation
    }

    /// The output's physical size in millimetres: its own, or at 96 DPI.
    fn output_mm(&self) -> (u32, u32) {
        let o = &self.output;
        if o.physical_mm.0 > 0 { (o.physical_mm.0 as u32, o.physical_mm.1 as u32) } else { ((o.width * 254 / 960) as u32, (o.height * 254 / 960) as u32) }
    }

    fn mode_name(&self) -> String {
        format!("{}x{}", self.output.width, self.output.height)
    }

    /// A mode's description: its size, and a dot clock that makes its refresh the output's.
    fn mode_info(&self, rep: &mut Reply) {
        let (w, h) = (self.output.width.max(1) as u32, self.output.height.max(1) as u32);
        let clock = (u64::from(w) * u64::from(h) * u64::from(self.output.refresh_mhz.max(1000)) / 1000) as u32;
        rep.u32(self.mode_id()).u16(w as u16).u16(h as u16).u32(clock);
        rep.u16(w as u16).u16(w as u16).u16(w as u16).u16(0).u16(h as u16).u16(h as u16).u16(h as u16).u16(self.mode_name().len() as u16).u32(0);
    }

    pub(super) fn randr(&mut self, ci: u32, r: Req) {
        match r.b1() {
            // QueryVersion: 1.5, or what the client knows if it's less.
            0 => {
                let (major, minor) = if (r.u32(4), r.u32(8)) < (1, 5) { (r.u32(4), r.u32(8)) } else { (1, 5) };
                let mut rep = Reply::new(0);
                rep.u32(major).u32(minor);
                self.reply(ci, rep);
            }
            // SetScreenConfig: the configuration as it is is the only one.
            2 => {
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).u32(self.output_time).u32(ROOT).u16(0);
                self.reply(ci, rep);
            }
            4 => {
                let (w, mask) = (r.u32(4), r.u16(8));
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                self.randr_selects.retain(|&(c, win, _)| !(c == ci && win == w));
                if mask != 0 {
                    self.randr_selects.push((ci, w, mask));
                }
            }
            5 => {
                let (w, h) = (self.output.width as u16, self.output.height as u16);
                let (mw, mh) = self.output_mm();
                let rate = (self.output.refresh_mhz / 1000) as u16;
                let mut rep = Reply::new(1);
                rep.u32(ROOT).u32(self.output_time).u32(self.output_time).u16(1).u16(0).u16(1).u16(rate).u16(2).skip(2);
                rep.u16(w).u16(h).u16(mw as u16).u16(mh as u16).u16(1).u16(rate);
                self.reply(ci, rep);
            }
            6 => {
                let mut rep = Reply::new(0);
                rep.u16(8).u16(8).u16(32767).u16(32767);
                self.reply(ci, rep);
            }
            7 | 13 | 14 | 24 | 26 | 30 | 43 | 44 => {}
            8 | 25 => {
                let name = self.mode_name();
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).u32(self.output_time).u16(1).u16(1).u16(1).u16(name.len() as u16).skip(8);
                rep.u32(CRTC).u32(OUTPUT);
                self.mode_info(&mut rep);
                rep.bytes(name.as_bytes());
                self.reply(ci, rep);
            }
            9 => {
                if r.u32(4) != OUTPUT {
                    return self.error(ci, RANDR_ERROR, r.u32(4));
                }
                let (mw, mh) = self.output_mm();
                let name = self.output.name.clone();
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).u32(CRTC).u32(mw).u32(mh).u8(0).u8(0).u16(1).u16(1).u16(1).u16(0).u16(name.len() as u16);
                rep.u32(CRTC).u32(self.mode_id()).bytes(name.as_bytes());
                self.reply(ci, rep);
            }
            // ListOutputProperties: none.
            10 => {
                let mut rep = Reply::new(0);
                rep.u16(0);
                self.reply(ci, rep);
            }
            11 => self.error(ci, err::NAME, r.u32(8)),
            12 | 16..=19 => self.error(ci, err::ACCESS, r.u32(4)),
            // GetOutputProperty: nothing there.
            15 => {
                let mut rep = Reply::new(0);
                rep.u32(0).u32(0).u32(0);
                self.reply(ci, rep);
            }
            20 => {
                if r.u32(4) != CRTC {
                    return self.error(ci, RANDR_ERROR + 1, r.u32(4));
                }
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).i16(0).i16(0).u16(self.output.width as u16).u16(self.output.height as u16).u32(self.mode_id());
                rep.u16(1).u16(1).u16(1).u16(1).u32(OUTPUT).u32(OUTPUT);
                self.reply(ci, rep);
            }
            // SetCrtcConfig: granted only as it is.
            21 => {
                let same = r.u32(4) == CRTC && r.u32(20) == self.mode_id();
                let mut rep = Reply::new(if same { 0 } else { 3 });
                rep.u32(self.output_time);
                self.reply(ci, rep);
            }
            22 => {
                let mut rep = Reply::new(0);
                rep.u16(GAMMA as u16);
                self.reply(ci, rep);
            }
            23 => {
                let mut rep = Reply::new(0);
                rep.u16(GAMMA as u16).skip(22);
                for _ in 0..3 {
                    for i in 0..GAMMA {
                        rep.u16((i * 65535 / (GAMMA - 1)) as u16);
                    }
                }
                self.reply(ci, rep);
            }
            // GetCrtcTransform: the identity, now and pending, no filters.
            27 => {
                let identity = |rep: &mut Reply| {
                    for i in 0..9 {
                        rep.u32(if i % 4 == 0 { 0x10000 } else { 0 });
                    }
                };
                let mut rep = Reply::new(0);
                identity(&mut rep);
                rep.u8(1).skip(3);
                identity(&mut rep);
                rep.skip(4).u16(0).u16(0).u16(0).u16(0);
                self.reply(ci, rep);
            }
            // GetPanning: none.
            28 => {
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).skip(24);
                self.reply(ci, rep);
            }
            29 => {
                let mut rep = Reply::new(0);
                rep.u32(self.output_time);
                self.reply(ci, rep);
            }
            31 => {
                let mut rep = Reply::new(0);
                rep.u32(OUTPUT);
                self.reply(ci, rep);
            }
            // GetProviders: none.
            32 => {
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).u16(0);
                self.reply(ci, rep);
            }
            42 => {
                let name = self.atom(&self.output.name.clone());
                let (mw, mh) = self.output_mm();
                let mut rep = Reply::new(0);
                rep.u32(self.output_time).u32(1).u32(1).skip(12);
                rep.u32(name).u8(1).u8(1).u16(1).i16(0).i16(0).u16(self.output.width as u16).u16(self.output.height as u16).u32(mw).u32(mh).u32(OUTPUT);
                self.reply(ci, rep);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    /// The output changed: a new mode, and every client that asked told so.
    pub(super) fn randr_output_changed(&mut self) {
        self.output_generation += 1;
        self.output_time = self.time();
        let (w, h) = (self.output.width as u16, self.output.height as u16);
        let (mw, mh) = self.output_mm();
        let (t, mode) = (self.output_time, self.mode_id());
        for (ci, win, mask) in self.randr_selects.clone() {
            if mask & SCREEN_CHANGE != 0 {
                let e = Ev::new(RANDR_EVENT).u8(1, 1).u32(4, t).u32(8, t).u32(12, ROOT).u32(16, win).u16(20, 0).u16(22, 0).u16(24, w).u16(26, h).u16(28, mw as u16).u16(30, mh as u16);
                self.event(ci, e);
            }
            if mask & CRTC_CHANGE != 0 {
                let e = Ev::new(RANDR_EVENT + 1).u8(1, 0).u32(4, t).u32(8, win).u32(12, CRTC).u32(16, mode).u16(20, 1).i16(24, 0).i16(26, 0).u16(28, w).u16(30, h);
                self.event(ci, e);
            }
            if mask & OUTPUT_CHANGE != 0 {
                let e = Ev::new(RANDR_EVENT + 1).u8(1, 1).u32(4, t).u32(8, t).u32(12, win).u32(16, OUTPUT).u32(20, CRTC).u32(24, mode).u16(28, 1).u8(30, 0).u8(31, 0);
                self.event(ci, e);
            }
        }
    }
}
