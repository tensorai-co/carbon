//! The extensions' table (RENDER's, XKEYBOARD's, RANDR's and XFIXES' requests are in their own modules), and the
//! small ones:
//! BIG-REQUESTS (requests past 256 KiB), XC-MISC (resource IDs), SHAPE (shaped and input-shaped
//! windows: a top-level's bounding shape is its backing's transparency, its input shape the desktop's input region),
//! and MIT-SHM (images through shared memory, a SysV segment or a memfd, read and written in place).

use std::sync::Arc;

use display::Rect;
use kernels_core::lock;

use super::draw::{self, Region};
use super::render::{RENDER, RENDER_ERROR};
use super::randr::{RANDR, RANDR_ERROR, RANDR_EVENT};
use super::xfixes::{XFIXES, XFIXES_ERROR, XFIXES_EVENT};
use super::xkb::{XKB_ERROR, XKB_EVENT, XKEYBOARD};
use super::wire::{Ev, Reply, Req, err};
use super::{ID_MASK, MAX_REQUEST, Res, SHIFT, Server, top_input};
use crate::fs::{Node, Open};
use crate::mm::shared::SharedMem;

pub const BIG_REQUESTS: u8 = 128;
pub const XC_MISC: u8 = 129;
pub const SHAPE: u8 = 130;
pub const MIT_SHM: u8 = 131;
const SHAPE_EVENT: u8 = 64;
const SHM_EVENT: u8 = 65;
const SHM_ERROR: u8 = 128;

/// Each extension: its name, major opcode, first event and first error.
pub const EXTENSIONS: &[(&str, u8, u8, u8)] = &[
    ("BIG-REQUESTS", BIG_REQUESTS, 0, 0),
    ("XC-MISC", XC_MISC, 0, 0),
    ("SHAPE", SHAPE, SHAPE_EVENT, 0),
    ("MIT-SHM", MIT_SHM, SHM_EVENT, SHM_ERROR),
    ("RENDER", RENDER, 0, RENDER_ERROR),
    ("XKEYBOARD", XKEYBOARD, XKB_EVENT, XKB_ERROR),
    ("RANDR", RANDR, RANDR_EVENT, RANDR_ERROR),
    ("XFIXES", XFIXES, XFIXES_EVENT, XFIXES_ERROR),
];

impl Server {
    pub(super) fn extension(&mut self, ci: u32, r: Req) {
        match r.major() {
            BIG_REQUESTS => {
                if let Some(c) = self.clients.get_mut(&ci) {
                    c.big = true;
                }
                let mut rep = Reply::new(0);
                rep.u32(MAX_REQUEST);
                self.reply(ci, rep);
            }
            XC_MISC => self.xc_misc(ci, r),
            SHAPE => self.shape(ci, r),
            MIT_SHM => self.shm(ci, r),
            RENDER => self.render(ci, r),
            XKEYBOARD => self.xkb(ci, r),
            RANDR => self.randr(ci, r),
            XFIXES => self.xfixes(ci, r),
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    fn xc_misc(&mut self, ci: u32, r: Req) {
        let base = ci << SHIFT;
        let used = self.res.keys().chain(self.windows.keys()).filter(|&&id| id >> SHIFT == ci).map(|&id| id & ID_MASK).max().map_or(0, |m| m + 1);
        match r.b1() {
            0 => {
                let mut rep = Reply::new(0);
                rep.u16(1).u16(1);
                self.reply(ci, rep);
            }
            1 => {
                let mut rep = Reply::new(0);
                rep.u32(base + used).u32(ID_MASK + 1 - used);
                self.reply(ci, rep);
            }
            2 => {
                let n = r.u32(4).min(ID_MASK + 1 - used);
                let mut rep = Reply::new(0);
                rep.u32(n).skip(20);
                for i in 0..n {
                    rep.u32(base + used + i);
                }
                self.reply(ci, rep);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    /// A window's region of one kind (0 bounding, 1 clip, 2 input), as it is: its shape, or all of it.
    fn region_of(&self, w: u32, kind: u8) -> Region {
        let Some(win) = self.windows.get(&w) else { return Vec::new() };
        let shape = if kind == 2 { &win.input } else { &win.bounding };
        shape.clone().unwrap_or_else(|| vec![Rect::new(0, 0, win.w, win.h)])
    }

    pub(super) fn set_region(&mut self, w: u32, kind: u8, region: Option<Region>) {
        let Some(win) = self.windows.get_mut(&w) else { return };
        match kind {
            0 => win.bounding = region,
            2 => win.input = region,
            _ => return,
        }
        // A top-level's shapes are the desktop's: its bounding shape what of its backing shows, both its input region.
        let Some(t) = &win.top else { return };
        *lock(&t.backing.shape) = win.bounding.clone();
        let (window, input) = (t.window, top_input(win));
        let mut d = display::desktop::lock(&self.desktop);
        d.set_input_region(window, input);
        d.damage(window, Rect::new(0, 0, 1 << 15, 1 << 15));
        d.commit(window);
    }

    /// A region combined into a window's: Set, Union, Intersect, Subtract or Invert.
    fn combine(&mut self, w: u32, kind: u8, op: u8, src: Region) {
        let dst = self.region_of(w, kind);
        let out = match op {
            0 => src,
            1 => draw::union(&dst, &src),
            2 => draw::intersect(&dst, &src),
            3 => src.iter().fold(dst, |a, r| draw::subtract(&a, r)),
            4 => dst.iter().fold(src, |a, r| draw::subtract(&a, r)),
            _ => dst,
        };
        self.set_region(w, kind, Some(out));
    }

    fn shape(&mut self, ci: u32, r: Req) {
        match r.b1() {
            // QueryVersion: 1.1.
            0 => {
                let mut rep = Reply::new(0);
                rep.u16(1).u16(1);
                self.reply(ci, rep);
            }
            // Rectangles.
            1 => {
                let (op, kind, w, x, y) = (r.u8(4), r.u8(5), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                let rects: Region = r.rects(16).into_iter().map(|(rx, ry, rw, rh)| Rect::new(x + rx, y + ry, x + rx + rw, y + ry + rh)).filter(|r| !r.is_empty()).collect();
                // Rectangles that may overlap made apart, so a region's never counted twice.
                let rects = rects.iter().fold(Vec::new(), |a: Region, r| draw::union(&a, &[*r]));
                self.combine(w, kind, op, rects);
            }
            // Mask: a bitmap's set pixels (none: the shape taken away).
            2 => {
                let (op, kind, w, x, y, pixmap) = (r.u8(4), r.u8(5), r.u32(8), r.i16(12) as i32, r.i16(14) as i32, r.u32(16));
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                if pixmap == 0 {
                    return self.set_region(w, kind, None);
                }
                let bits = match self.res.get(&pixmap) {
                    Some((_, Res::Pixmap(p))) => lock(&p.surface).clone(),
                    _ => return self.error(ci, err::PIXMAP, pixmap),
                };
                let region = draw::bitmap_region(&bits, x, y);
                self.combine(w, kind, op, region);
            }
            // Combine: another window's region, moved.
            3 => {
                let (op, kind, src_kind, w, x, y, src) = (r.u8(4), r.u8(5), r.u8(6), r.u32(8), r.i16(12) as i32, r.i16(14) as i32, r.u32(16));
                if !self.windows.contains_key(&w) || !self.windows.contains_key(&src) {
                    return self.error(ci, err::WINDOW, w);
                }
                let region = self.region_of(src, src_kind).into_iter().map(|r| r.offset(x, y)).collect();
                self.combine(w, kind, op, region);
            }
            // Offset.
            4 => {
                let (kind, w, x, y) = (r.u8(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
                let current = self.windows.get(&w).and_then(|win| if kind == 2 { win.input.clone() } else { win.bounding.clone() });
                if let Some(region) = current {
                    self.set_region(w, kind, Some(region.into_iter().map(|r| r.offset(x, y)).collect()));
                }
            }
            // QueryExtents.
            5 => {
                let w = r.u32(4);
                let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
                let shaped = win.bounding.is_some();
                let b = self.region_of(w, 0).iter().fold(Rect::default(), |a, r| a.union(r));
                let c = Rect::new(0, 0, win.w, win.h);
                let mut rep = Reply::new(0);
                rep.u8(u8::from(shaped)).u8(0).skip(2).i16(b.left as i16).i16(b.top as i16).u16(b.width() as u16).u16(b.height() as u16);
                rep.i16(0).i16(0).u16(c.width() as u16).u16(c.height() as u16);
                self.reply(ci, rep);
            }
            // SelectInput: ShapeNotify isn't sent yet.
            6 => {}
            // InputSelected.
            7 => {
                let rep = Reply::new(0);
                self.reply(ci, rep);
            }
            // GetRectangles.
            8 => {
                let (w, kind) = (r.u32(4), r.u8(8));
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                let region = self.region_of(w, kind);
                let mut rep = Reply::new(0);
                rep.u32(region.len() as u32).skip(20);
                for r in region {
                    rep.i16(r.left as i16).i16(r.top as i16).u16(r.width() as u16).u16(r.height() as u16);
                }
                self.reply(ci, rep);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    fn segment(&self, id: u32) -> Option<Arc<SharedMem>> {
        match self.res.get(&id) {
            Some((_, Res::Shm(mem))) => Some(mem.clone()),
            _ => None,
        }
    }

    fn shm(&mut self, ci: u32, r: Req) {
        match r.b1() {
            // QueryVersion: 1.2, no shared pixmaps, images as whole pixels.
            0 => {
                let mut rep = Reply::new(0);
                rep.u16(1).u16(2).u16(0).u16(0).u8(2);
                self.reply(ci, rep);
            }
            // Attach: a SysV segment, by its ID, in the machine's first IPC namespace.
            1 => {
                let (seg, shmid) = (r.u32(4), r.u32(8));
                let Some(mem) = crate::mm::shared::shm_segment(&self.k.boot_ns.ipc, shmid as i32) else { return self.error(ci, err::ACCESS, shmid) };
                if self.new_id(ci, seg) {
                    self.res.insert(seg, (ci, Res::Shm(mem)));
                }
            }
            // Detach.
            2 => {
                self.res.remove(&r.u32(4));
            }
            // PutImage.
            3 => {
                let (d, gcid) = (r.u32(4), r.u32(8));
                let (total_w, total_h, sx, sy, sw, sh) = (r.u16(12) as i32, r.u16(14) as i32, r.u16(16) as i32, r.u16(18) as i32, r.u16(20) as i32, r.u16(22) as i32);
                let (dx, dy, depth, format, send_event, seg, offset) = (r.i16(24) as i32, r.i16(26) as i32, r.u8(28), r.u8(29), r.u8(30) != 0, r.u32(32), r.u32(36) as u64);
                let Some(mem) = self.segment(seg) else { return self.error(ci, SHM_ERROR, seg) };
                let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
                if sx + sw > total_w || sy + sh > total_h {
                    return self.error(ci, err::VALUE, 0);
                }
                let px = if format == 2 && depth != 1 {
                    // Whole pixels: just the rows and columns wanted, read in place.
                    let bpp = draw::bits_per_pixel(depth);
                    let stride = (total_w as usize * bpp).div_ceil(32) * 4;
                    let row_bytes = sw as usize * bpp / 8;
                    let mut data = vec![0u8; row_bytes.div_ceil(4) * 4 * sh as usize];
                    let packed = row_bytes.div_ceil(4) * 4;
                    for row in 0..sh as usize {
                        let at = offset + ((sy as usize + row) * stride + sx as usize * bpp / 8) as u64;
                        mem.read(at, &mut data[row * packed..row * packed + row_bytes]);
                    }
                    draw::decode_image(2, depth, sw, sh, 0, &data, gc.fg, gc.bg)
                } else {
                    let stride = (total_w as usize).div_ceil(32) * 4;
                    let mut data = vec![0u8; stride * total_h as usize];
                    mem.read(offset, &mut data);
                    draw::decode_image(format, depth, total_w, total_h, 0, &data, gc.fg, gc.bg).map(|all| {
                        (0..sh).flat_map(|row| (0..sw).map(move |col| ((sy + row) * total_w + sx + col) as usize)).map(|i| all[i]).collect()
                    })
                };
                let Some(px) = px else { return self.error(ci, err::VALUE, 0) };
                self.blit_image(ci, d, &gc, dx, dy, sw, sh, &px);
                if send_event {
                    let e = Ev::new(SHM_EVENT).u32(4, d).u16(8, 3).u8(10, MIT_SHM).u32(12, seg).u32(16, offset as u32);
                    self.event(ci, e);
                }
            }
            // GetImage: written into the segment.
            4 => {
                let (d, x, y, w, h, format, seg, offset) = (r.u32(4), r.i16(8) as i32, r.i16(10) as i32, r.u16(12) as i32, r.u16(14) as i32, r.u8(20), r.u32(24), r.u32(28) as u64);
                let Some(mem) = self.segment(seg) else { return self.error(ci, SHM_ERROR, seg) };
                let (px, depth, visual) = match self.read_drawable(d, x, y, w, h) {
                    Ok(v) => v,
                    Err(e) => return self.error(ci, e, d),
                };
                let px: Vec<u32> = if depth == 24 { px.iter().map(|p| p & 0xffffff).collect() } else { px };
                let bytes = draw::encode_image(if format == 2 { depth } else { 1 }, w, h, &px);
                let _ = mem.write(offset, &bytes);
                let mut rep = Reply::new(depth);
                rep.u32(visual).u32(bytes.len() as u32);
                self.reply(ci, rep);
            }
            // AttachFd: a memfd, passed beside the request.
            6 => {
                let seg = r.u32(4);
                let file = self.clients.get_mut(&ci).and_then(|c| c.fds.pop_front());
                let Some(Node::Memfd(mem, _)) = file.as_ref().map(|f| &f.node) else { return self.error(ci, err::ACCESS, seg) };
                let mem = mem.clone();
                if self.new_id(ci, seg) {
                    self.res.insert(seg, (ci, Res::Shm(mem)));
                }
            }
            // CreateSegment: a memfd the server makes, passed back.
            7 => {
                let (seg, size) = (r.u32(4), r.u32(8) as u64);
                if !self.new_id(ci, seg) {
                    return;
                }
                let mem = SharedMem::new(self.k.exec.clone(), size.max(1), 0);
                let file = Open::new(Node::Memfd(mem.clone(), false), crate::abi::O_RDWR, "/memfd:xshm (deleted)");
                self.res.insert(seg, (ci, Res::Shm(mem)));
                let rep = Reply::new(1);
                self.reply(ci, rep);
                if let Some(c) = self.clients.get_mut(&ci) {
                    c.out_fds.push(file);
                }
            }
            // CreatePixmap: no shared pixmaps (QueryVersion says so).
            _ => self.error(ci, err::REQUEST, 0),
        }
    }
}
