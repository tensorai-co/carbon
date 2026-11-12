//! RENDER (0.11): pictures over drawables and over fills (solid colours, linear, radial and conical gradients),
//! composited with Porter-Duff's operators and the separable blend modes on premultiplied ARGB, through masks (by
//! alpha, or per component), with repeat, transforms and nearest or bilinear filtering; trapezoids and triangles as
//! antialiased coverage; glyph sets and runs of glyphs; filled rectangles; and cursors from pictures. What Xft's text
//! and cairo's drawing go through.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use display::cursor::Cursor;
use display::{Rect, Surface};
use kernels_core::lock;

use super::draw::{self, Region};
use super::wire::{Reply, Req, err};
use super::{Res, Server, Target, TargetSurface, VISUAL, VISUAL_ARGB};

pub const RENDER: u8 = 132;
/// RENDER's errors, from here: PictFormat, Picture, PictOp, GlyphSet, Glyph.
pub const RENDER_ERROR: u8 = 129;
const BAD_FORMAT: u8 = RENDER_ERROR;
const BAD_PICTURE: u8 = RENDER_ERROR + 1;
const BAD_OP: u8 = RENDER_ERROR + 2;
const BAD_GLYPHSET: u8 = RENDER_ERROR + 3;

/// A picture format: a depth, and where each channel is in a pixel (shift, and mask after shifting).
pub struct PictFormat {
    pub id: u32,
    pub depth: u8,
    pub r: (u32, u32),
    pub g: (u32, u32),
    pub b: (u32, u32),
    pub a: (u32, u32),
}

pub const A8R8G8B8: u32 = 0x30;
pub const X8R8G8B8: u32 = 0x31;
const R5G6B5: u32 = 0x32;
const X1R5G5B5: u32 = 0x33;
const A8: u32 = 0x34;
const A4: u32 = 0x35;
const A1: u32 = 0x36;

/// One format for each pixmap depth the setup reply lists.
pub const FORMATS: [PictFormat; 7] = [
    PictFormat { id: A8R8G8B8, depth: 32, r: (16, 0xff), g: (8, 0xff), b: (0, 0xff), a: (24, 0xff) },
    PictFormat { id: X8R8G8B8, depth: 24, r: (16, 0xff), g: (8, 0xff), b: (0, 0xff), a: (0, 0) },
    PictFormat { id: R5G6B5, depth: 16, r: (11, 0x1f), g: (5, 0x3f), b: (0, 0x1f), a: (0, 0) },
    PictFormat { id: X1R5G5B5, depth: 15, r: (10, 0x1f), g: (5, 0x1f), b: (0, 0x1f), a: (0, 0) },
    PictFormat { id: A8, depth: 8, r: (0, 0), g: (0, 0), b: (0, 0), a: (0, 0xff) },
    PictFormat { id: A4, depth: 4, r: (0, 0), g: (0, 0), b: (0, 0), a: (0, 0xf) },
    PictFormat { id: A1, depth: 1, r: (0, 0), g: (0, 0), b: (0, 0), a: (0, 1) },
];

pub fn format(id: u32) -> Option<&'static PictFormat> {
    FORMATS.iter().find(|f| f.id == id)
}

/// A channel's value widened to 8 bits.
fn widen(v: u32, mask: u32) -> u32 {
    match mask {
        0xff => v,
        0 => 0,
        1 => v * 255,
        m => v * 255 / m,
    }
}

impl PictFormat {
    fn has_rgb(&self) -> bool {
        self.r.1 != 0
    }

    /// A pixel of this format as premultiplied ARGB (alpha 255 where it has none; black where it has no colour).
    pub fn to_argb(&self, raw: u32) -> u32 {
        if self.id == A8R8G8B8 {
            return raw;
        }
        if self.id == X8R8G8B8 {
            return raw | 0xff00_0000;
        }
        let ch = |(shift, mask): (u32, u32)| widen((raw >> shift) & mask, mask);
        let a = if self.a.1 == 0 { 255 } else { ch(self.a) };
        (a << 24) | (ch(self.r) << 16) | (ch(self.g) << 8) | ch(self.b)
    }

    /// Premultiplied ARGB as a pixel of this format.
    pub fn pixel(&self, argb: u32) -> u32 {
        if self.id == A8R8G8B8 {
            return argb;
        }
        if self.id == X8R8G8B8 {
            return argb & 0xff_ffff;
        }
        let ch = |v: u32, (shift, mask): (u32, u32)| if mask == 0 { 0 } else { ((v * mask + 127) / 255) << shift };
        ch(argb >> 24, self.a) | ch((argb >> 16) & 0xff, self.r) | ch((argb >> 8) & 0xff, self.g) | ch(argb & 0xff, self.b)
    }
}

/// The format a window or pixmap of a depth has.
pub fn format_for_depth(depth: u8) -> u32 {
    FORMATS.iter().find(|f| f.depth == depth).map_or(X8R8G8B8, |f| f.id)
}

/// What a picture shows.
#[derive(Clone)]
pub enum Source {
    Window(u32),
    /// A pixmap's pixels, and its depth: kept by the picture after the pixmap's freed, as the protocol has it.
    Pixmap(Arc<Mutex<Surface>>, u8),
    /// Premultiplied ARGB.
    Solid(u32),
    Linear { p1: (f64, f64), p2: (f64, f64), stops: Stops },
    Radial { c1: (f64, f64), r1: f64, c2: (f64, f64), r2: f64, stops: Stops },
    Conical { c: (f64, f64), angle: f64, stops: Stops },
}

/// A gradient's stops: each place (0 to 1), and its colour, not premultiplied, as floats.
pub type Stops = Arc<Vec<(f64, [f64; 4])>>;

#[derive(Clone)]
pub struct Picture {
    pub source: Source,
    pub format: u32,
    /// 0 none, 1 normal, 2 pad, 3 reflect.
    pub repeat: u8,
    pub clip: Option<Vec<Rect>>,
    pub clip_mask: Option<Arc<Mutex<Surface>>>,
    pub clip_origin: (i32, i32),
    pub component_alpha: bool,
    /// From the destination's coordinates to this picture's, row by row.
    pub transform: Option<[f64; 9]>,
    pub bilinear: bool,
    pub subwindow_mode: u8,
}

pub struct Glyph {
    pub w: i32,
    pub h: i32,
    pub x: i32,
    pub y: i32,
    pub xoff: i32,
    pub yoff: i32,
    /// Premultiplied ARGB (alpha only, for a glyph set of an alpha format).
    pub px: Vec<u32>,
}

pub struct GlyphSet {
    pub format: u32,
    pub glyphs: HashMap<u32, Glyph>,
}

/// How a picture's colours are read for one request.
pub struct Sampler {
    kind: Kind,
    repeat: u8,
    transform: Option<[f64; 9]>,
    bilinear: bool,
    /// Its colours per component as a mask (component alpha), or only its alpha.
    pub ca: bool,
}

enum Kind {
    Empty,
    Solid(u32),
    /// Premultiplied ARGB, the part `(ox, oy)` `w` by `h` of a picture `fw` by `fh`.
    Image { px: Vec<u32>, ox: i32, oy: i32, w: i32, h: i32, fw: i32, fh: i32 },
    Linear { p1: (f64, f64), d: (f64, f64), len2: f64, stops: Stops },
    Radial { c1: (f64, f64), r1: f64, cd: (f64, f64), dr: f64, a: f64, stops: Stops },
    Conical { c: (f64, f64), angle: f64, stops: Stops },
}

fn fixed(v: u32) -> f64 {
    v as i32 as f64 / 65536.0
}

/// A coordinate within a size, as a repeat has it (None: outside, with none).
fn wrap(v: i32, size: i32, repeat: u8) -> Option<i32> {
    if size <= 0 {
        return None;
    }
    match repeat {
        1 => Some(v.rem_euclid(size)),
        2 => Some(v.clamp(0, size - 1)),
        3 => {
            let p = v.rem_euclid(2 * size);
            Some(if p < size { p } else { 2 * size - 1 - p })
        }
        _ => (0..size).contains(&v).then_some(v),
    }
}

fn wrap_t(t: f64, repeat: u8) -> Option<f64> {
    match repeat {
        1 => Some(t - t.floor()),
        2 => Some(t.clamp(0.0, 1.0)),
        3 => {
            let p = t.rem_euclid(2.0);
            Some(if p <= 1.0 { p } else { 2.0 - p })
        }
        _ => (0.0..=1.0).contains(&t).then_some(t),
    }
}

/// A gradient's colour at a place, premultiplied.
fn gradient(stops: &[(f64, [f64; 4])], t: f64) -> u32 {
    let Some(first) = stops.first() else { return 0 };
    let c = if t <= first.0 {
        first.1
    } else if let Some(i) = stops.iter().position(|s| s.0 >= t) {
        let (a, b) = (stops[i - 1], stops[i]);
        let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
        [0, 1, 2, 3].map(|k| a.1[k] + (b.1[k] - a.1[k]) * f)
    } else {
        stops[stops.len() - 1].1
    };
    let a = c[3].clamp(0.0, 1.0);
    let ch = |v: f64| ((v.clamp(0.0, 1.0) * a * 255.0) + 0.5) as u32;
    (((a * 255.0 + 0.5) as u32) << 24) | (ch(c[0]) << 16) | (ch(c[1]) << 8) | ch(c[2])
}

fn lerp(a: u32, b: u32, f: u32) -> u32 {
    let mut out = 0;
    for shift in [0, 8, 16, 24] {
        let (x, y) = ((a >> shift) & 0xff, (b >> shift) & 0xff);
        out |= ((x * (256 - f) + y * f) >> 8) << shift;
    }
    out
}

impl Sampler {
    fn empty() -> Sampler {
        Sampler { kind: Kind::Empty, repeat: 0, transform: None, bilinear: false, ca: false }
    }

    pub fn solid(argb: u32) -> Sampler {
        Sampler { kind: Kind::Solid(argb), ..Sampler::empty() }
    }

    /// Pixels already premultiplied, at their place, nothing outside them.
    pub fn image(px: Vec<u32>, r: Rect) -> Sampler {
        Sampler { kind: Kind::Image { px, ox: r.left, oy: r.top, w: r.width(), h: r.height(), fw: r.right, fh: r.bottom }, ..Sampler::empty() }
    }

    fn texel(&self, x: i32, y: i32) -> u32 {
        let Kind::Image { px, ox, oy, w, h, fw, fh } = &self.kind else { return 0 };
        // Outside the picture: as its repeat says, within what was read of it.
        let (Some(x), Some(y)) = (wrap(x, *fw, self.repeat), wrap(y, *fh, self.repeat)) else { return 0 };
        let (x, y) = (x - ox, y - oy);
        if x < 0 || y < 0 || x >= *w || y >= *h {
            return 0;
        }
        px[(y * w + x) as usize]
    }

    /// The colour at a pixel of the picture (its centre, through its transform).
    pub fn at(&self, x: i32, y: i32) -> u32 {
        let (fx, fy) = match &self.transform {
            None => {
                if let Kind::Image { .. } = self.kind {
                    return self.texel(x, y);
                }
                (x as f64 + 0.5, y as f64 + 0.5)
            }
            Some(m) => {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w = m[6] * px + m[7] * py + m[8];
                let w = if w == 0.0 { 1.0 } else { w };
                ((m[0] * px + m[1] * py + m[2]) / w, (m[3] * px + m[4] * py + m[5]) / w)
            }
        };
        match &self.kind {
            Kind::Empty => 0,
            Kind::Solid(c) => *c,
            Kind::Image { .. } => {
                if self.bilinear {
                    let (u, v) = (fx - 0.5, fy - 0.5);
                    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
                    let (fu, fv) = (((u - u.floor()) * 256.0) as u32, ((v - v.floor()) * 256.0) as u32);
                    let top = lerp(self.texel(x0, y0), self.texel(x0 + 1, y0), fu);
                    let bottom = lerp(self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1), fu);
                    lerp(top, bottom, fv)
                } else {
                    self.texel(fx.floor() as i32, fy.floor() as i32)
                }
            }
            Kind::Linear { p1, d, len2, stops } => {
                let t = if *len2 == 0.0 { 0.0 } else { ((fx - p1.0) * d.0 + (fy - p1.1) * d.1) / len2 };
                wrap_t(t, self.repeat).map_or(0, |t| gradient(stops, t))
            }
            Kind::Radial { c1, r1, cd, dr, a, stops } => {
                // The greatest t whose circle (between the two, and past them) holds the point, with a radius >= 0.
                let (px, py) = (fx - c1.0, fy - c1.1);
                let b = px * cd.0 + py * cd.1 + r1 * dr;
                let c = px * px + py * py - r1 * r1;
                let t = if a.abs() < 1e-9 {
                    if b == 0.0 { return 0 }
                    c / (2.0 * b)
                } else {
                    let disc = b * b - a * c;
                    if disc < 0.0 {
                        return 0;
                    }
                    let s = disc.sqrt();
                    let (t1, t2) = ((b + s) / a, (b - s) / a);
                    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
                    if r1 + hi * dr >= 0.0 {
                        hi
                    } else if r1 + lo * dr >= 0.0 {
                        lo
                    } else {
                        return 0;
                    }
                };
                wrap_t(t, self.repeat).map_or(0, |t| gradient(stops, t))
            }
            Kind::Conical { c, angle, stops } => {
                let a = (fy - c.1).atan2(fx - c.0) - angle;
                let t = (a / std::f64::consts::TAU).rem_euclid(1.0);
                gradient(stops, t)
            }
        }
    }
}

// ---- Compositing ----

fn mul(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Porter-Duff's factors for the source and destination: from the destination's alpha and the source's.
fn factors(op: u8, sa: u32, da: u32) -> (u32, u32) {
    match op {
        0 => (0, 0),
        1 => (255, 0),
        2 => (0, 255),
        3 => (255, 255 - sa),
        4 => (255 - da, 255),
        5 => (da, 0),
        6 => (0, sa),
        7 => (255 - da, 0),
        8 => (0, 255 - sa),
        9 => (da, 255 - sa),
        10 => (255 - da, sa),
        11 => (255 - da, 255 - sa),
        12 => (255, 255),
        13 => (((255 - da) * 255).checked_div(sa).map_or(255, |v| v.min(255)), 255),
        _ => (255, 255 - sa),
    }
}

/// The operator's result for a source (already through its mask) and a destination pixel; `ma`, a component mask
/// (each channel's own), makes each channel's source alpha the source's alpha through that channel's mask.
fn combine(op: u8, s: u32, d: u32, ma: Option<u32>) -> u32 {
    if op >= 48 {
        return blend(op, s, d, ma);
    }
    // Disjoint and conjoint operators as their plain ones.
    let op = if op >= 32 { op - 32 } else if op >= 16 { op - 16 } else { op };
    let (sa, da) = (s >> 24, d >> 24);
    if ma.is_none() {
        match op {
            1 => return s,
            3 if sa == 255 => return s,
            3 if sa == 0 => return d,
            _ => {}
        }
    }
    let mut out = 0;
    for shift in [0, 8, 16, 24] {
        let sca = match ma {
            Some(m) if shift != 24 => mul(sa, (m >> shift) & 0xff),
            _ => sa,
        };
        let (fa, fb) = factors(op, sca, da);
        let (sc, dc) = ((s >> shift) & 0xff, (d >> shift) & 0xff);
        out |= (mul(sc, fa) + mul(dc, fb)).min(255) << shift;
    }
    out
}

/// The separable blend modes (and the non-separable ones as Over): `(1 - as)·d + (1 - ad)·s + B(s, d)`.
fn blend(op: u8, s: u32, d: u32, ma: Option<u32>) -> u32 {
    let f = |v: u32, shift: u32| ((v >> shift) & 0xff) as f64 / 255.0;
    let (sa, da) = (f(s, 24), f(d, 24));
    let mut out = (((sa + da - sa * da) * 255.0 + 0.5) as u32).min(255) << 24;
    for shift in [0, 8, 16] {
        let sa = match ma {
            Some(m) => sa * f(m, shift),
            None => sa,
        };
        let (sc, dc) = (f(s, shift), f(d, shift));
        // B on premultiplied values: as·ad·B(sc/as, dc/ad).
        let (cs, cd) = (if sa > 0.0 { sc / sa } else { 0.0 }, if da > 0.0 { dc / da } else { 0.0 });
        let b = match op {
            48 => cs * cd,
            49 => cs + cd - cs * cd,
            50 => if cd <= 0.5 { 2.0 * cs * cd } else { 1.0 - 2.0 * (1.0 - cs) * (1.0 - cd) },
            51 => cs.min(cd),
            52 => cs.max(cd),
            53 => if cd == 0.0 { 0.0 } else if cs >= 1.0 { 1.0 } else { (cd / (1.0 - cs)).min(1.0) },
            54 => if cd >= 1.0 { 1.0 } else if cs <= 0.0 { 0.0 } else { 1.0 - ((1.0 - cd) / cs).min(1.0) },
            55 => if cs <= 0.5 { 2.0 * cs * cd } else { 1.0 - 2.0 * (1.0 - cs) * (1.0 - cd) },
            56 => {
                if cs <= 0.5 {
                    cd - (1.0 - 2.0 * cs) * cd * (1.0 - cd)
                } else {
                    let g = if cd <= 0.25 { ((16.0 * cd - 12.0) * cd + 4.0) * cd } else { cd.sqrt() };
                    cd + (2.0 * cs - 1.0) * (g - cd)
                }
            }
            57 => (cs - cd).abs(),
            58 => cs + cd - 2.0 * cs * cd,
            _ => cs,
        };
        let r = (1.0 - sa) * dc + (1.0 - da) * sc + sa * da * b;
        out |= (((r * 255.0) + 0.5) as u32).min(255) << shift;
    }
    out
}

/// One request's compositing: `op` of a source (through a mask) onto destination pixels in `area` (the target's
/// coordinates) within a region; source and mask coordinates are the target's plus their offsets.
pub struct Composite<'a> {
    pub op: u8,
    pub src: &'a Sampler,
    pub src_off: (i32, i32),
    pub mask: Option<(&'a Sampler, (i32, i32))>,
    pub dst: &'a PictFormat,
    /// The destination picture's clip mask, and where its corner is in the target.
    pub dst_mask: Option<(&'a Surface, i32, i32)>,
}

impl Composite<'_> {
    pub fn run(&self, target: &mut Surface, area: Rect, clip: &[Rect]) -> Rect {
        let bounds = target.bounds();
        let mut done = Rect::default();
        // A solid source, no mask, onto every pixel: one value written.
        let fill = match (&self.src.kind, &self.mask, self.dst_mask, self.op) {
            (Kind::Solid(c), None, None, 1) => Some(self.dst.pixel(*c)),
            (Kind::Solid(c), None, None, 3) if c >> 24 == 255 => Some(self.dst.pixel(*c)),
            _ => None,
        };
        for r in clip {
            let r = r.intersect(&area).intersect(&bounds);
            if r.is_empty() {
                continue;
            }
            done = done.union(&r);
            for y in r.top..r.bottom {
                let row = (y * target.width) as usize;
                if let Some(v) = fill {
                    target.pixels[row + r.left as usize..row + r.right as usize].fill(v);
                    continue;
                }
                for x in r.left..r.right {
                    if let Some((m, mx, my)) = self.dst_mask
                        && m.get(x - mx, y - my).unwrap_or(0) & 1 == 0
                    {
                        continue;
                    }
                    let mut s = self.src.at(x + self.src_off.0, y + self.src_off.1);
                    let mut ma = None;
                    if let Some((mask, off)) = &self.mask {
                        let m = mask.at(x + off.0, y + off.1);
                        if mask.ca {
                            let mut out = 0;
                            for shift in [0, 8, 16, 24] {
                                let mc = if shift == 24 { m >> 24 } else { (m >> shift) & 0xff };
                                out |= mul((s >> shift) & 0xff, mc) << shift;
                            }
                            s = out;
                            ma = Some(m);
                        } else {
                            let a = m >> 24;
                            if a == 0 && matches!(self.op, 3 | 12) {
                                continue;
                            }
                            if a != 255 {
                                s = [0, 8, 16, 24].iter().fold(0, |acc, &sh| acc | (mul((s >> sh) & 0xff, a) << sh));
                            }
                        }
                    }
                    let at = row + x as usize;
                    let d = self.dst.to_argb(target.pixels[at]);
                    target.pixels[at] = self.dst.pixel(combine(self.op, s, d, ma));
                }
            }
        }
        done
    }
}

/// Polygons' antialiased coverage (4 by 4 samples a pixel, overlapping coverage counted once at most): its
/// bounds, and premultiplied ARGB with the coverage as its alpha. None if it covers nothing.
pub fn coverage(polys: &[Vec<(f64, f64)>]) -> Option<(Rect, Vec<u32>)> {
    const S: i32 = 4;
    let (mut l, mut t, mut r, mut b) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in polys.iter().flatten() {
        l = l.min(p.0);
        t = t.min(p.1);
        r = r.max(p.0);
        b = b.max(p.1);
    }
    if l >= r || t >= b {
        return None;
    }
    let bounds = Rect::new(l.floor().max(-16384.0) as i32, t.floor().max(-16384.0) as i32, r.ceil().min(16384.0) as i32, b.ceil().min(16384.0) as i32);
    let (w, h) = (bounds.width(), bounds.height());
    if w <= 0 || h <= 0 || (w as i64) * (h as i64) > 1 << 24 {
        return None;
    }
    let mut counts = vec![0u8; (w * h) as usize];
    for poly in polys {
        let scaled: Vec<(f64, f64)> = poly.iter().map(|p| ((p.0 - bounds.left as f64) * S as f64, (p.1 - bounds.top as f64) * S as f64)).collect();
        let mut spans = Vec::new();
        draw::polygon_spans(&scaled, false, &mut spans);
        for (sy, x0, x1) in spans {
            let y = sy.div_euclid(S);
            if y < 0 || y >= h {
                continue;
            }
            for sx in x0.max(0)..x1.min(w * S) {
                let c = &mut counts[(y * w + sx / S) as usize];
                *c = c.saturating_add(1);
            }
        }
    }
    let px = counts.iter().map(|&c| {
        let a = (u32::from(c).min((S * S) as u32) * 255) / (S * S) as u32;
        a << 24 | a << 16 | a << 8 | a
    }).collect();
    Some((bounds, px))
}

impl Server {
    fn picture(&self, id: u32) -> Option<Picture> {
        match self.res.get(&id) {
            Some((_, Res::Picture(p))) => Some((**p).clone()),
            _ => None,
        }
    }

    /// A picture's sampler for one request, reading as much of its drawable as the request needs (`want`, in its
    /// coordinates; all of it, if it repeats or is transformed).
    fn sampler(&self, p: &Picture, want: Rect) -> Sampler {
        let ca = p.component_alpha;
        let base = Sampler { kind: Kind::Empty, repeat: p.repeat, transform: p.transform, bilinear: p.bilinear, ca };
        let kind = match &p.source {
            Source::Solid(c) => Kind::Solid(*c),
            Source::Linear { p1, p2, stops } => {
                let d = (p2.0 - p1.0, p2.1 - p1.1);
                Kind::Linear { p1: *p1, d, len2: d.0 * d.0 + d.1 * d.1, stops: stops.clone() }
            }
            Source::Radial { c1, r1, c2, r2, stops } => {
                let (cd, dr) = ((c2.0 - c1.0, c2.1 - c1.1), r2 - r1);
                Kind::Radial { c1: *c1, r1: *r1, cd, dr, a: cd.0 * cd.0 + cd.1 * cd.1 - dr * dr, stops: stops.clone() }
            }
            Source::Conical { c, angle, stops } => Kind::Conical { c: *c, angle: *angle, stops: stops.clone() },
            Source::Window(_) | Source::Pixmap(..) => {
                let Some((fw, fh)) = self.source_size(&p.source) else { return base };
                let full = Rect::new(0, 0, fw, fh);
                let r = if p.repeat != 0 || p.transform.is_some() { full } else { want.intersect(&full) };
                if r.is_empty() {
                    return Sampler { kind: Kind::Image { px: Vec::new(), ox: 0, oy: 0, w: 0, h: 0, fw, fh }, ..base };
                }
                let (raw, depth) = match &p.source {
                    Source::Window(d) => match self.read_drawable(*d, r.left, r.top, r.width(), r.height()) {
                        Ok((raw, depth, _)) => (raw, depth),
                        Err(_) => return base,
                    },
                    Source::Pixmap(s, depth) => (draw::read(&lock(s), r.left, r.top, r.width(), r.height()), *depth),
                    _ => return base,
                };
                let f = format(p.format).or_else(|| format(format_for_depth(depth))).expect("a format");
                let px = raw.iter().map(|&v| f.to_argb(v)).collect();
                Kind::Image { px, ox: r.left, oy: r.top, w: r.width(), h: r.height(), fw, fh }
            }
        };
        Sampler { kind, ..base }
    }

    /// The size of a picture's drawable.
    fn source_size(&self, source: &Source) -> Option<(i32, i32)> {
        match source {
            Source::Window(d) => self.windows.get(d).map(|w| (w.w, w.h)),
            Source::Pixmap(s, _) => {
                let s = lock(s);
                Some((s.width, s.height))
            }
            _ => None,
        }
    }

    /// A pixmap's pixels, by its ID (for a picture to keep).
    fn pixmap_pixels(&self, id: u32) -> Option<Arc<Mutex<Surface>>> {
        match self.res.get(&id) {
            Some((_, Res::Pixmap(p))) => Some(p.surface.clone()),
            _ => None,
        }
    }

    /// CreatePicture's and ChangePicture's values set on a picture (a clip mask's pixels kept by it).
    fn set_picture(&mut self, ci: u32, pid: u32, values: &[Option<u32>; 32]) {
        let mask = values[6].map(|id| if id == 0 { None } else { self.pixmap_pixels(id) });
        match self.res.get_mut(&pid) {
            Some((_, Res::Picture(p))) => {
                set_attributes(p, values);
                if let Some(m) = mask {
                    p.clip_mask = m;
                    p.clip = None;
                }
            }
            _ => self.error(ci, BAD_PICTURE, pid),
        }
    }

    /// Composites onto a destination picture: `area` in its coordinates, with source and mask coordinates as offsets
    /// from it.
    #[allow(clippy::too_many_arguments)]
    fn composite_onto(&mut self, ci: u32, op: u8, src: &Sampler, src_off: (i32, i32), mask: Option<(&Sampler, (i32, i32))>, dst_id: u32, area: Rect) {
        let Some(dst) = self.picture(dst_id) else { return self.error(ci, BAD_PICTURE, dst_id) };
        let t = match &dst.source {
            Source::Window(d) => match self.target(*d, dst.subwindow_mode == 1) {
                Ok(Some(t)) => t,
                Ok(None) => return,
                Err(e) => return self.error(ci, e, *d),
            },
            Source::Pixmap(s, depth) => {
                let (w, h) = { let s = lock(s); (s.width, s.height) };
                Target { surface: TargetSurface::Pixmap(s.clone()), top: None, ox: 0, oy: 0, clip: vec![Rect::new(0, 0, w, h)], depth: *depth }
            }
            _ => return self.error(ci, err::MATCH, dst_id),
        };
        let fmt = format(dst.format).unwrap_or(&FORMATS[1]);
        let (ox, oy) = (t.ox, t.oy);
        let mut clip = t.clip.clone();
        let (cx, cy) = (dst.clip_origin.0 + ox, dst.clip_origin.1 + oy);
        if let Some(rects) = &dst.clip {
            clip = draw::intersect(&clip, &rects.iter().map(|r| r.offset(cx, cy)).collect::<Region>());
        }
        let dst_mask = dst.clip_mask.as_ref().map(|m| lock(m).clone());
        let job = Composite {
            op,
            src,
            src_off: (src_off.0 - ox, src_off.1 - oy),
            mask: mask.map(|(m, off)| (m, (off.0 - ox, off.1 - oy))),
            dst: fmt,
            dst_mask: dst_mask.as_ref().map(|m| (m, cx, cy)),
        };
        let done = t.with(|s| job.run(s, area.offset(ox, oy), &clip));
        if let Some(top) = t.top {
            self.mark(top, done);
        }
    }

    pub(super) fn render(&mut self, ci: u32, r: Req) {
        match r.b1() {
            // QueryVersion: 0.11.
            0 => {
                let mut rep = Reply::new(0);
                rep.u32(0).u32(11);
                self.reply(ci, rep);
            }
            1 => self.query_pict_formats(ci),
            // QueryPictIndexValues: no indexed formats.
            2 => self.error(ci, BAD_FORMAT, r.u32(4)),
            4 => {
                let (pid, d, fmt, mask) = (r.u32(4), r.u32(8), r.u32(12), r.u32(16));
                let Some(f) = format(fmt) else { return self.error(ci, BAD_FORMAT, fmt) };
                let (source, depth) = if let Some(w) = self.windows.get(&d) {
                    (Source::Window(d), w.depth)
                } else {
                    match self.res.get(&d) {
                        Some((_, Res::Pixmap(p))) => (Source::Pixmap(p.surface.clone(), p.depth), p.depth),
                        _ => return self.error(ci, err::DRAWABLE, d),
                    }
                };
                if depth != f.depth && !(depth == 24 && f.depth == 32) {
                    return self.error(ci, err::MATCH, fmt);
                }
                if !self.new_id(ci, pid) {
                    return;
                }
                self.res.insert(pid, (ci, Res::Picture(Box::new(new_picture(source, fmt)))));
                self.set_picture(ci, pid, &r.values(mask, 20));
            }
            5 => {
                let values = r.values(r.u32(8), 12);
                self.set_picture(ci, r.u32(4), &values);
            }
            6 => {
                let rects: Vec<Rect> = r.rects(12).into_iter().map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h)).collect();
                match self.res.get_mut(&r.u32(4)) {
                    Some((_, Res::Picture(p))) => {
                        p.clip_origin = (r.i16(8) as i32, r.i16(10) as i32);
                        p.clip = Some(rects);
                        p.clip_mask = None;
                    }
                    _ => self.error(ci, BAD_PICTURE, r.u32(4)),
                }
            }
            7 | 19 => {
                self.res.remove(&r.u32(4));
            }
            8 => self.composite(ci, r),
            10..=13 => self.shapes(ci, r),
            17 => {
                let (gsid, fmt) = (r.u32(4), r.u32(8));
                if format(fmt).is_none() {
                    return self.error(ci, BAD_FORMAT, fmt);
                }
                if self.new_id(ci, gsid) {
                    self.res.insert(gsid, (ci, Res::GlyphSet(Arc::new(Mutex::new(GlyphSet { format: fmt, glyphs: HashMap::new() })))));
                }
            }
            18 => {
                let (gsid, existing) = (r.u32(4), r.u32(8));
                let Some(set) = self.glyphset(existing) else { return self.error(ci, BAD_GLYPHSET, existing) };
                if self.new_id(ci, gsid) {
                    self.res.insert(gsid, (ci, Res::GlyphSet(set)));
                }
            }
            20 => self.add_glyphs(ci, r),
            22 => {
                let Some(set) = self.glyphset(r.u32(4)) else { return self.error(ci, BAD_GLYPHSET, r.u32(4)) };
                let mut set = lock(&set);
                for c in r.rest(8).as_chunks::<4>().0 {
                    set.glyphs.remove(&u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
            }
            23..=25 => self.composite_glyphs(ci, r),
            26 => {
                let (op, dst) = (r.u8(4), r.u32(8));
                let color = color_at(&r, 12, true);
                let src = Sampler::solid(color);
                for (x, y, w, h) in r.rects(20) {
                    self.composite_onto(ci, op, &src, (0, 0), None, dst, Rect::new(x, y, x + w, y + h));
                }
            }
            27 => {
                let (cid, pid, hx, hy) = (r.u32(4), r.u32(8), r.u16(12) as i32, r.u16(14) as i32);
                let Some(p) = self.picture(pid) else { return self.error(ci, BAD_PICTURE, pid) };
                let Some((w, h)) = self.source_size(&p.source) else { return self.error(ci, err::MATCH, pid) };
                let s = self.sampler(&p, Rect::new(0, 0, w, h));
                let mut pixels = Surface::new(w, h, 0);
                for y in 0..h {
                    for x in 0..w {
                        pixels.pixels[(y * w + x) as usize] = s.at(x, y);
                    }
                }
                if self.new_id(ci, cid) {
                    self.res.insert(cid, (ci, Res::Cursor(Cursor::Image { pixels, hotspot: (hx, hy) })));
                }
            }
            28 => {
                let m: [f64; 9] = std::array::from_fn(|i| fixed(r.u32(8 + 4 * i)));
                let identity = m == [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
                match self.res.get_mut(&r.u32(4)) {
                    Some((_, Res::Picture(p))) => p.transform = (!identity).then_some(m),
                    _ => self.error(ci, BAD_PICTURE, r.u32(4)),
                }
            }
            29 => {
                const FILTERS: [&str; 5] = ["nearest", "bilinear", "fast", "good", "best"];
                let mut rep = Reply::new(0);
                rep.u32(FILTERS.len() as u32).u32(FILTERS.len() as u32).skip(16);
                for alias in [0xffffu16, 0xffff, 0, 1, 1] {
                    rep.u16(alias);
                }
                rep.skip(wire_pad(FILTERS.len() * 2));
                for f in FILTERS {
                    rep.u8(f.len() as u8).bytes(f.as_bytes());
                }
                self.reply(ci, rep);
            }
            30 => {
                let name = r.str(12, r.u16(8) as usize);
                match self.res.get_mut(&r.u32(4)) {
                    Some((_, Res::Picture(p))) => p.bilinear = !matches!(name.as_str(), "nearest" | "fast"),
                    _ => self.error(ci, BAD_PICTURE, r.u32(4)),
                }
            }
            31 => {
                // An animated cursor: its first frame.
                let (cid, first) = (r.u32(4), r.u32(8));
                let cursor = match self.res.get(&first) {
                    Some((_, Res::Cursor(c))) => c.clone(),
                    _ => Cursor::default(),
                };
                if self.new_id(ci, cid) {
                    self.res.insert(cid, (ci, Res::Cursor(cursor)));
                }
            }
            32 => self.add_traps(ci, r),
            33 => {
                let pid = r.u32(4);
                let color = color_at(&r, 8, true);
                if self.new_id(ci, pid) {
                    self.res.insert(pid, (ci, Res::Picture(Box::new(new_picture(Source::Solid(color), 0)))));
                }
            }
            34..=36 => self.gradient(ci, r),
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    fn query_pict_formats(&mut self, ci: u32) {
        let depths = draw::DEPTHS;
        let mut rep = Reply::new(0);
        rep.u32(FORMATS.len() as u32).u32(1).u32(depths.len() as u32).u32(2).u32(1).skip(4);
        for f in &FORMATS {
            rep.u32(f.id).u8(1).u8(f.depth).skip(2);
            for (shift, mask) in [f.r, f.g, f.b, f.a] {
                rep.u16(shift as u16).u16(mask as u16);
            }
            rep.u32(0);
        }
        rep.u32(depths.len() as u32).u32(X8R8G8B8);
        for d in depths {
            let visual = match d {
                24 => Some((VISUAL, X8R8G8B8)),
                32 => Some((VISUAL_ARGB, A8R8G8B8)),
                _ => None,
            };
            rep.u8(d).skip(1).u16(u16::from(visual.is_some())).skip(4);
            if let Some((v, f)) = visual {
                rep.u32(v).u32(f);
            }
        }
        rep.u32(0);
        self.reply(ci, rep);
    }

    fn composite(&mut self, ci: u32, r: Req) {
        let (op, src_id, mask_id, dst_id) = (r.u8(4), r.u32(8), r.u32(12), r.u32(16));
        let (sx, sy, mx, my, dx, dy, w, h) = (r.i16(20) as i32, r.i16(22) as i32, r.i16(24) as i32, r.i16(26) as i32, r.i16(28) as i32, r.i16(30) as i32, r.u16(32) as i32, r.u16(34) as i32);
        if !valid_op(op) {
            return self.error(ci, BAD_OP, u32::from(op));
        }
        let Some(src) = self.picture(src_id) else { return self.error(ci, BAD_PICTURE, src_id) };
        let s = self.sampler(&src, Rect::new(sx, sy, sx + w, sy + h));
        let m = match mask_id {
            0 => None,
            id => match self.picture(id) {
                Some(p) => Some(self.sampler(&p, Rect::new(mx, my, mx + w, my + h))),
                None => return self.error(ci, BAD_PICTURE, id),
            },
        };
        self.composite_onto(ci, op, &s, (sx - dx, sy - dy), m.as_ref().map(|m| (m, (mx - dx, my - dy))), dst_id, Rect::new(dx, dy, dx + w, dy + h));
    }

    /// Trapezoids, triangles, strips and fans: their coverage as a mask for the source, its origin at the first
    /// point's pixel.
    fn shapes(&mut self, ci: u32, r: Req) {
        let (op, src_id, dst_id, sx, sy) = (r.u8(4), r.u32(8), r.u32(12), r.i16(20) as i32, r.i16(22) as i32);
        if !valid_op(op) {
            return self.error(ci, BAD_OP, u32::from(op));
        }
        let f = |at: usize| (fixed(r.u32(at)), fixed(r.u32(at + 4)));
        let body = r.rest(24);
        let mut polys: Vec<Vec<(f64, f64)>> = Vec::new();
        match r.b1() {
            10 => {
                for i in 0..body.len() / 40 {
                    let at = 24 + 40 * i;
                    let (top, bottom) = (fixed(r.u32(at)), fixed(r.u32(at + 4)));
                    let (l1, l2, r1, r2) = (f(at + 8), f(at + 16), f(at + 24), f(at + 32));
                    if bottom <= top {
                        continue;
                    }
                    let x_at = |a: (f64, f64), b: (f64, f64), y: f64| if b.1 == a.1 { a.0 } else { a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) };
                    polys.push(vec![(x_at(l1, l2, top), top), (x_at(r1, r2, top), top), (x_at(r1, r2, bottom), bottom), (x_at(l1, l2, bottom), bottom)]);
                }
            }
            11 => {
                for i in 0..body.len() / 24 {
                    let at = 24 + 24 * i;
                    polys.push(vec![f(at), f(at + 8), f(at + 16)]);
                }
            }
            op => {
                let pts: Vec<(f64, f64)> = (0..body.len() / 8).map(|i| f(24 + 8 * i)).collect();
                for i in 2..pts.len() {
                    polys.push(if op == 12 { vec![pts[i - 2], pts[i - 1], pts[i]] } else { vec![pts[0], pts[i - 1], pts[i]] });
                }
            }
        }
        let first = match r.b1() {
            10 => f(24 + 8),
            _ => f(24),
        };
        let Some((bounds, px)) = coverage(&polys) else { return };
        let Some(src) = self.picture(src_id) else { return self.error(ci, BAD_PICTURE, src_id) };
        let (x0, y0) = (first.0.floor() as i32, first.1.floor() as i32);
        let s = self.sampler(&src, bounds.offset(sx - x0, sy - y0));
        let mask = Sampler::image(px, bounds);
        self.composite_onto(ci, op, &s, (sx - x0, sy - y0), Some((&mask, (0, 0))), dst_id, bounds);
    }

    /// AddTraps: trapezoids' coverage added into an alpha picture.
    fn add_traps(&mut self, ci: u32, r: Req) {
        let (pid, xo, yo) = (r.u32(4), r.i16(8) as f64, r.i16(10) as f64);
        let f = |at: usize| fixed(r.u32(at));
        let polys: Vec<Vec<(f64, f64)>> = (0..r.rest(12).len() / 24)
            .map(|i| {
                let at = 12 + 24 * i;
                let (tl, tr, ty, bl, br, by) = (f(at), f(at + 4), f(at + 8), f(at + 12), f(at + 16), f(at + 20));
                vec![(tl + xo, ty + yo), (tr + xo, ty + yo), (br + xo, by + yo), (bl + xo, by + yo)]
            })
            .collect();
        let Some((bounds, px)) = coverage(&polys) else { return };
        let white = Sampler::solid(0xffff_ffff);
        let mask = Sampler::image(px, bounds);
        self.composite_onto(ci, 12, &white, (0, 0), Some((&mask, (0, 0))), pid, bounds);
    }

    fn glyphset(&self, id: u32) -> Option<Arc<Mutex<GlyphSet>>> {
        match self.res.get(&id) {
            Some((_, Res::GlyphSet(g))) => Some(g.clone()),
            _ => None,
        }
    }

    fn add_glyphs(&mut self, ci: u32, r: Req) {
        let (gsid, n) = (r.u32(4), r.u32(8) as usize);
        let Some(set) = self.glyphset(gsid) else { return self.error(ci, BAD_GLYPHSET, gsid) };
        let mut set = lock(&set);
        let Some(f) = format(set.format) else { return };
        let ids_at = 12;
        let infos_at = ids_at + 4 * n;
        let mut data_at = infos_at + 12 * n;
        for i in 0..n {
            let id = r.u32(ids_at + 4 * i);
            let at = infos_at + 12 * i;
            let (w, h, x, y, xoff, yoff) = (r.u16(at) as i32, r.u16(at + 2) as i32, r.i16(at + 4) as i32, r.i16(at + 6) as i32, r.i16(at + 8) as i32, r.i16(at + 10) as i32);
            let bpp = draw::bits_per_pixel(f.depth);
            let stride = (w as usize * bpp).div_ceil(32) * 4;
            let size = stride * h as usize;
            let raw = draw::decode_image(2, f.depth, w, h, 0, r.bytes(data_at, size), 1, 0).unwrap_or_else(|| vec![0; (w * h) as usize]);
            data_at += size;
            let px = raw.iter().map(|&v| f.to_argb(v)).map(|c| if f.has_rgb() { c } else { let a = c >> 24; a << 24 | a << 16 | a << 8 | a }).collect();
            set.glyphs.insert(id, Glyph { w, h, x, y, xoff, yoff, px });
        }
    }

    /// A run of glyphs: their images added up into one mask (in each glyph's place, from the pen as each element
    /// moves it), the source composited through it, its origin at the first glyph's origin.
    fn composite_glyphs(&mut self, ci: u32, r: Req) {
        let size = match r.b1() {
            23 => 1,
            24 => 2,
            _ => 4,
        };
        let (op, src_id, dst_id, mut gsid, sx, sy) = (r.u8(4), r.u32(8), r.u32(12), r.u32(20), r.i16(24) as i32, r.i16(26) as i32);
        if !valid_op(op) {
            return self.error(ci, BAD_OP, u32::from(op));
        }
        let Some(mut set) = self.glyphset(gsid) else { return self.error(ci, BAD_GLYPHSET, gsid) };
        // Each glyph's place: its image's corner, and its set (which may change along the run).
        let mut placed: Vec<(i32, i32, Arc<Mutex<GlyphSet>>, u32)> = Vec::new();
        let (mut x, mut y) = (0i32, 0i32);
        let mut first: Option<(i32, i32)> = None;
        let mut at = 28;
        while at + 8 <= r.data.len() {
            let n = r.u8(at) as usize;
            let (dx, dy) = (r.i16(at + 4) as i32, r.i16(at + 6) as i32);
            if n == 255 {
                gsid = r.u32(at + 8);
                match self.glyphset(gsid) {
                    Some(s) => set = s,
                    None => return self.error(ci, BAD_GLYPHSET, gsid),
                }
                at += 12;
                continue;
            }
            x += dx;
            y += dy;
            if first.is_none() {
                first = Some((x, y));
            }
            for i in 0..n {
                let g = at + 8 + i * size;
                let id = match size {
                    1 => u32::from(r.u8(g)),
                    2 => u32::from(r.u16(g)),
                    _ => r.u32(g),
                };
                let adv = lock(&set).glyphs.get(&id).map(|gl| (gl.xoff, gl.yoff));
                if let Some((xoff, yoff)) = adv {
                    placed.push((x, y, set.clone(), id));
                    x += xoff;
                    y += yoff;
                }
            }
            at += 8 + (n * size).div_ceil(4) * 4;
        }
        let Some((x0, y0)) = first else { return };
        let mut bounds = Rect::default();
        let mut ca = false;
        for (gx, gy, s, id) in &placed {
            let s = lock(s);
            ca |= format(s.format).is_some_and(|f| f.has_rgb());
            if let Some(g) = s.glyphs.get(id) {
                bounds = bounds.union(&Rect::new(gx - g.x, gy - g.y, gx - g.x + g.w, gy - g.y + g.h));
            }
        }
        if bounds.is_empty() || (bounds.width() as i64) * (bounds.height() as i64) > 1 << 24 {
            return;
        }
        let (bw, bh) = (bounds.width(), bounds.height());
        let mut mask = vec![0u32; (bw * bh) as usize];
        for (gx, gy, s, id) in &placed {
            let s = lock(s);
            let Some(g) = s.glyphs.get(id) else { continue };
            let (ox, oy) = (gx - g.x - bounds.left, gy - g.y - bounds.top);
            for row in 0..g.h {
                for col in 0..g.w {
                    let v = g.px[(row * g.w + col) as usize];
                    let m = &mut mask[((oy + row) * bw + ox + col) as usize];
                    let mut out = 0;
                    for shift in [0, 8, 16, 24] {
                        out |= (((*m >> shift) & 0xff) + ((v >> shift) & 0xff)).min(255) << shift;
                    }
                    *m = out;
                }
            }
        }
        let Some(src) = self.picture(src_id) else { return self.error(ci, BAD_PICTURE, src_id) };
        let s = self.sampler(&src, bounds.offset(sx - x0, sy - y0));
        let mut m = Sampler::image(mask, bounds);
        m.ca = ca;
        self.composite_onto(ci, op, &s, (sx - x0, sy - y0), Some((&m, (0, 0))), dst_id, bounds);
    }

    fn gradient(&mut self, ci: u32, r: Req) {
        let pid = r.u32(4);
        let p = |at: usize| (fixed(r.u32(at)), fixed(r.u32(at + 4)));
        let (n_at, source_of): (usize, Box<dyn Fn(Stops) -> Source>) = match r.b1() {
            34 => {
                let (p1, p2) = (p(8), p(16));
                (24, Box::new(move |stops| Source::Linear { p1, p2, stops }))
            }
            35 => {
                let (c1, c2, r1, r2) = (p(8), p(16), fixed(r.u32(24)), fixed(r.u32(28)));
                (32, Box::new(move |stops| Source::Radial { c1, r1, c2, r2, stops }))
            }
            _ => {
                let (c, angle) = (p(8), fixed(r.u32(16)).to_radians());
                (20, Box::new(move |stops| Source::Conical { c, angle, stops }))
            }
        };
        let n = r.u32(n_at) as usize;
        let stops_at = n_at + 4;
        let colors_at = stops_at + 4 * n;
        let stops: Vec<(f64, [f64; 4])> = (0..n)
            .map(|i| {
                let c = colors_at + 8 * i;
                let ch = |at: usize| f64::from(r.u16(at)) / 65535.0;
                (fixed(r.u32(stops_at + 4 * i)), [ch(c), ch(c + 2), ch(c + 4), ch(c + 6)])
            })
            .collect();
        if self.new_id(ci, pid) {
            self.res.insert(pid, (ci, Res::Picture(Box::new(new_picture(source_of(Arc::new(stops)), 0)))));
        }
    }
}

fn wire_pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

fn valid_op(op: u8) -> bool {
    matches!(op, 0..=13 | 16..=27 | 32..=43 | 48..=62)
}

/// A colour (red, green, blue, alpha, 16 bits each) as premultiplied ARGB.
fn color_at(r: &Req, at: usize, premultiplied: bool) -> u32 {
    let (red, green, blue, alpha) = (u32::from(r.u16(at) >> 8), u32::from(r.u16(at + 2) >> 8), u32::from(r.u16(at + 4) >> 8), u32::from(r.u16(at + 6) >> 8));
    if premultiplied {
        alpha << 24 | red.min(alpha) << 16 | green.min(alpha) << 8 | blue.min(alpha)
    } else {
        alpha << 24 | mul(red, alpha) << 16 | mul(green, alpha) << 8 | mul(blue, alpha)
    }
}

fn new_picture(source: Source, format: u32) -> Picture {
    Picture { source, format, repeat: 0, clip: None, clip_mask: None, clip_origin: (0, 0), component_alpha: false, transform: None, bilinear: false, subwindow_mode: 0 }
}

/// CreatePicture's and ChangePicture's values, by their mask's bits.
fn set_attributes(p: &mut Picture, values: &[Option<u32>; 32]) {
    for (bit, v) in values.iter().enumerate() {
        let Some(v) = *v else { continue };
        match bit {
            0 => p.repeat = (v & 3) as u8,
            4 => p.clip_origin.0 = v as i16 as i32,
            5 => p.clip_origin.1 = v as i16 as i32,
            8 => p.subwindow_mode = v as u8,
            12 => p.component_alpha = v != 0,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porter_duff() {
        // Half-transparent red over opaque blue.
        let s = 0x8080_0000;
        let d = 0xff00_00ff;
        let over = combine(3, s, d, None);
        assert_eq!(over >> 24, 0xff);
        assert!(((over >> 16) & 0xff).abs_diff(0x80) <= 1 && (over & 0xff).abs_diff(0x7f) <= 1);
        assert_eq!(combine(1, s, d, None), s);
        assert_eq!(combine(0, s, d, None), 0);
        assert_eq!(combine(12, 0x8000_0080, 0x8000_0080, None), 0xff00_00ff);
    }

    #[test]
    fn formats_round_trip() {
        for f in &FORMATS {
            let argb = f.to_argb(f.pixel(0xff_ffffff));
            assert_eq!(argb >> 24, 0xff, "{}", f.id);
        }
        let f = format(R5G6B5).unwrap();
        assert_eq!(f.to_argb(0xf800), 0xffff_0000);
        assert_eq!(format(A8).unwrap().to_argb(0x80), 0x8000_0000);
    }

    #[test]
    fn a_square_is_covered_fully_and_its_edge_half() {
        let (r, px) = coverage(&[vec![(1.0, 1.0), (3.0, 1.0), (3.0, 3.0), (1.0, 3.0)]]).unwrap();
        assert_eq!(r, Rect::new(1, 1, 3, 3));
        assert!(px.iter().all(|&p| p >> 24 == 255));
        let (_, px) = coverage(&[vec![(0.0, 0.0), (1.5, 0.0), (1.5, 1.0), (0.0, 1.0)]]).unwrap();
        assert_eq!((px[0] >> 24, px[1] >> 24), (255, 127));
    }

    #[test]
    fn a_linear_gradient_goes_from_one_stop_to_the_other() {
        let stops: Stops = Arc::new(vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])]);
        let p = new_picture(Source::Linear { p1: (0.0, 0.0), p2: (10.0, 0.0), stops }, 0);
        let s = Server::sampler_for_test(&p);
        assert_eq!(s.at(0, 0) & 0xff, 13);
        assert_eq!(s.at(9, 0) & 0xff, 242);
        assert_eq!(s.at(20, 0), 0);
    }
}

#[cfg(test)]
impl Server {
    fn sampler_for_test(p: &Picture) -> Sampler {
        match &p.source {
            Source::Linear { p1, p2, stops } => {
                let d = (p2.0 - p1.0, p2.1 - p1.1);
                Sampler { kind: Kind::Linear { p1: *p1, d, len2: d.0 * d.0 + d.1 * d.1, stops: stops.clone() }, ..Sampler::empty() }
            }
            _ => Sampler::empty(),
        }
    }
}
