//! XFIXES (5.0): regions (made from rectangles, bitmaps, windows, GCs and pictures; combined, inverted, moved,
//! expanded, read back, and used as a GC's, a picture's or a window's shape clip); selection events (an owner set, its
//! window gone, its client gone); the cursor's image and name; hiding and showing the cursor over a client's windows;
//! and pointer barriers (accepted, not kept).

use display::Rect;
use display::cursor::Cursor;
use kernels_core::lock;

use super::draw::{self, Clip, Region};
use super::wire::{Ev, Reply, Req, err};
use super::{Res, Server};

pub const XFIXES: u8 = 135;
pub const XFIXES_EVENT: u8 = 69;
/// BadRegion.
pub const XFIXES_ERROR: u8 = 139;
const BAD_REGION: u8 = XFIXES_ERROR;

/// The selection notify's subtypes (each one's bit in `SelectSelectionInput`'s mask is 1 << it): an owner set, its
/// window destroyed, its client gone.
pub const OWNER_SET: u8 = 0;
pub const OWNER_WINDOW_GONE: u8 = 1;
pub const OWNER_CLIENT_GONE: u8 = 2;

impl Server {
    fn region(&self, id: u32) -> Option<Region> {
        match self.res.get(&id) {
            Some((_, Res::Region(r))) => Some(r.clone()),
            _ => None,
        }
    }

    /// A region made or set: kept under its ID (an error if that's no region, for `set`).
    fn put_region(&mut self, ci: u32, id: u32, region: Region, new: bool) {
        let region: Region = region.into_iter().filter(|r| !r.is_empty()).collect();
        if new {
            if self.new_id(ci, id) {
                self.res.insert(id, (ci, Res::Region(region)));
            }
            return;
        }
        match self.res.get_mut(&id) {
            Some((_, Res::Region(r))) => *r = region,
            _ => self.error(ci, BAD_REGION, id),
        }
    }

    pub(super) fn xfixes(&mut self, ci: u32, r: Req) {
        let rects = |at: usize| -> Region { r.rects(at).into_iter().map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h)).fold(Vec::new(), |a, rect| draw::union(&a, &[rect])) };
        match r.b1() {
            0 => {
                let (major, minor) = if (r.u32(4), r.u32(8)) < (5, 0) { (r.u32(4), r.u32(8)) } else { (5, 0) };
                let mut rep = Reply::new(0);
                rep.u32(major).u32(minor);
                self.reply(ci, rep);
            }
            // ChangeSaveSet; SelectCursorInput (the cursor's changes aren't sent); SetCursorName; ChangeCursor and
            // ChangeCursorByName; DeletePointerBarrier.
            1 | 3 | 23 | 26 | 27 | 32 => {}
            2 => {
                let (w, selection, mask) = (r.u32(4), r.u32(8), r.u32(12));
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                self.selection_selects.retain(|&(c, win, sel, _)| !(c == ci && win == w && sel == selection));
                if mask != 0 {
                    self.selection_selects.push((ci, w, selection, mask));
                }
            }
            4 | 25 => {
                let (rx, ry) = self.root_pointer;
                let (pixels, w, h, hotspot) = self.cursor_image();
                let mut rep = Reply::new(0);
                rep.i16(rx as i16).i16(ry as i16).u16(w as u16).u16(h as u16).u16(hotspot.0 as u16).u16(hotspot.1 as u16).u32(1);
                if r.b1() == 25 {
                    rep.u32(0).u16(0).skip(2);
                } else {
                    rep.skip(8);
                }
                for p in pixels {
                    rep.u32(p);
                }
                self.reply(ci, rep);
            }
            5 => self.put_region(ci, r.u32(4), rects(8), true),
            6 => {
                let bits = match self.res.get(&r.u32(8)) {
                    Some((_, Res::Pixmap(p))) if p.depth == 1 => lock(&p.surface).clone(),
                    _ => return self.error(ci, err::PIXMAP, r.u32(8)),
                };
                self.put_region(ci, r.u32(4), draw::bitmap_region(&bits, 0, 0), true);
            }
            7 => {
                let (w, kind) = (r.u32(8), r.u8(12));
                let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
                let shape = if kind == 1 { None } else { win.bounding.clone() };
                let region = shape.unwrap_or_else(|| vec![Rect::new(0, 0, win.w, win.h)]);
                self.put_region(ci, r.u32(4), region, true);
            }
            8 => {
                let Some(gc) = self.gc(r.u32(8)) else { return self.error(ci, err::GCONTEXT, r.u32(8)) };
                let region = match gc.clip {
                    Clip::Rects(rs) => rs.into_iter().map(|rect| rect.offset(gc.clip_origin.0, gc.clip_origin.1)).collect(),
                    _ => Vec::new(),
                };
                self.put_region(ci, r.u32(4), region, true);
            }
            9 => {
                let region = match self.res.get(&r.u32(8)) {
                    Some((_, Res::Picture(p))) => p.clip.clone().unwrap_or_default().into_iter().map(|rect| rect.offset(p.clip_origin.0, p.clip_origin.1)).collect(),
                    _ => return self.error(ci, super::render::RENDER_ERROR + 1, r.u32(8)),
                };
                self.put_region(ci, r.u32(4), region, true);
            }
            10 => {
                if !matches!(self.res.get(&r.u32(4)), Some((_, Res::Region(_)))) {
                    return self.error(ci, BAD_REGION, r.u32(4));
                }
                self.res.remove(&r.u32(4));
            }
            11 => self.put_region(ci, r.u32(4), rects(8), false),
            12 => {
                let Some(src) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                self.put_region(ci, r.u32(8), src, false);
            }
            13..=15 => {
                let (Some(a), Some(b)) = (self.region(r.u32(4)), self.region(r.u32(8))) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let out = match r.b1() {
                    13 => draw::union(&a, &b),
                    14 => draw::intersect(&a, &b),
                    _ => b.iter().fold(a, |acc, cut| draw::subtract(&acc, cut)),
                };
                self.put_region(ci, r.u32(12), out, false);
            }
            16 => {
                let Some(src) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let (x, y, w, h) = (r.i16(8) as i32, r.i16(10) as i32, r.u16(12) as i32, r.u16(14) as i32);
                let out = src.iter().fold(vec![Rect::new(x, y, x + w, y + h)], |acc, cut| draw::subtract(&acc, cut));
                self.put_region(ci, r.u32(16), out, false);
            }
            17 => {
                let Some(src) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let (dx, dy) = (r.i16(8) as i32, r.i16(10) as i32);
                self.put_region(ci, r.u32(4), src.into_iter().map(|rect| rect.offset(dx, dy)).collect(), false);
            }
            18 => {
                let Some(src) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let extents = src.iter().fold(Rect::default(), |a, rect| a.union(rect));
                self.put_region(ci, r.u32(8), if extents.is_empty() { Vec::new() } else { vec![extents] }, false);
            }
            19 => {
                let Some(region) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let e = region.iter().fold(Rect::default(), |a, rect| a.union(rect));
                let mut rep = Reply::new(0);
                rep.i16(e.left as i16).i16(e.top as i16).u16(e.width() as u16).u16(e.height() as u16).skip(16);
                for rect in &region {
                    rep.i16(rect.left as i16).i16(rect.top as i16).u16(rect.width() as u16).u16(rect.height() as u16);
                }
                self.reply(ci, rep);
            }
            20 => {
                let (gc_id, region_id, x, y) = (r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
                let clip = match region_id {
                    0 => Clip::None,
                    id => match self.region(id) {
                        Some(region) => Clip::Rects(region),
                        None => return self.error(ci, BAD_REGION, id),
                    },
                };
                match self.res.get_mut(&gc_id) {
                    Some((_, Res::Gc(gc))) => {
                        gc.clip = clip;
                        gc.clip_origin = (x, y);
                    }
                    _ => self.error(ci, err::GCONTEXT, gc_id),
                }
            }
            21 => {
                let (w, kind, x, y, region_id) = (r.u32(4), r.u8(8), r.i16(12) as i32, r.i16(14) as i32, r.u32(16));
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                let region = match region_id {
                    0 => None,
                    id => match self.region(id) {
                        Some(region) => Some(region.into_iter().map(|rect| rect.offset(x, y)).collect()),
                        None => return self.error(ci, BAD_REGION, id),
                    },
                };
                self.set_region(w, kind, region);
            }
            22 => {
                let (pid, region_id, x, y) = (r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
                let clip = match region_id {
                    0 => None,
                    id => match self.region(id) {
                        Some(region) => Some(region),
                        None => return self.error(ci, BAD_REGION, id),
                    },
                };
                match self.res.get_mut(&pid) {
                    Some((_, Res::Picture(p))) => {
                        p.clip = clip;
                        p.clip_origin = (x, y);
                        p.clip_mask = None;
                    }
                    _ => self.error(ci, super::render::RENDER_ERROR + 1, pid),
                }
            }
            24 => {
                let mut rep = Reply::new(0);
                rep.u32(0).u16(0);
                self.reply(ci, rep);
            }
            28 => {
                let Some(src) = self.region(r.u32(4)) else { return self.error(ci, BAD_REGION, r.u32(4)) };
                let (l, rt, t, b) = (r.u16(12) as i32, r.u16(14) as i32, r.u16(16) as i32, r.u16(18) as i32);
                let out = src.iter().map(|rect| Rect::new(rect.left - l, rect.top - t, rect.right + rt, rect.bottom + b)).fold(Vec::new(), |a, rect| draw::union(&a, &[rect]));
                self.put_region(ci, r.u32(8), out, false);
            }
            29 | 30 => {
                let w = r.u32(4);
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                if r.b1() == 29 {
                    self.cursor_hidden.insert((ci, w));
                } else {
                    self.cursor_hidden.remove(&(ci, w));
                }
                self.update_cursor();
            }
            // CreatePointerBarrier: an ID, nothing kept.
            31 => {}
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    /// The cursor's picture now (premultiplied ARGB), its size, and its hotspot: a named shape's isn't drawn here, so
    /// it's one clear pixel.
    fn cursor_image(&self) -> (Vec<u32>, i32, i32, (i32, i32)) {
        let cursor = display::desktop::lock(&self.desktop).cursor().clone();
        match cursor {
            Cursor::Image { pixels, hotspot } => (pixels.pixels, pixels.width, pixels.height, hotspot),
            _ => (vec![0], 1, 1, (0, 0)),
        }
    }

    /// A selection's owner changed (`subtype` 0), or its window or client went (1, 2): told to every client that
    /// asked about that selection.
    pub(super) fn selection_event(&mut self, selection: u32, owner: u32, subtype: u8, time: u32) {
        let bit = 1 << subtype;
        let to: Vec<(u32, u32)> = self.selection_selects.iter().filter(|&&(_, _, sel, mask)| sel == selection && mask & bit != 0).map(|&(c, w, _, _)| (c, w)).collect();
        let now = self.time();
        for (ci, w) in to {
            let e = Ev::new(XFIXES_EVENT).u8(1, subtype).u32(4, w).u32(8, owner).u32(12, selection).u32(16, now).u32(20, time);
            self.event(ci, e);
        }
    }

    /// Whether a client hid the cursor over a window or one of its ancestors.
    pub(super) fn cursor_hidden_over(&self, mut w: u32) -> bool {
        loop {
            if self.cursor_hidden.iter().any(|&(_, hw)| hw == w) {
                return true;
            }
            let Some(win) = self.windows.get(&w) else { return false };
            if w == super::ROOT {
                return false;
            }
            w = win.parent;
        }
    }
}
