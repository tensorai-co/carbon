//! X11's drawing, into a window's top-level backing or a pixmap (both a `Surface` of 32-bit pixels): GCs, and every
//! primitive turned into spans of pixels (each row's merged, so a raster function like XOR touches a pixel once),
//! filled through the GC's clip, fill style and function; images in and out in the formats the setup reply
//! advertises; the core font (the display's own 6 by 13 `fixed`) and the cursor font's shapes.

use std::collections::BTreeMap;

use display::cursor::Shape;
use display::{Rect, Surface, font};

/// A span: a row, and the columns from the first to before the second.
pub type Span = (i32, i32, i32);

/// The rectangles of a region, never overlapping.
pub type Region = Vec<Rect>;

/// What a GC clips to beyond its drawable: nothing, rectangles, or a depth-1 pixmap's set pixels (both from its clip
/// origin).
#[derive(Clone, Debug, PartialEq)]
pub enum Clip {
    None,
    Rects(Vec<Rect>),
    Mask(u32),
}

/// A graphics context, as the client set it.
#[derive(Clone, Debug)]
pub struct Gc {
    pub function: u8,
    pub fg: u32,
    pub bg: u32,
    pub line_width: u16,
    pub line_style: u8,
    pub cap_style: u8,
    pub join_style: u8,
    pub fill_style: u8,
    /// 0: even-odd, 1: winding.
    pub fill_rule: u8,
    pub tile: Option<u32>,
    pub stipple: Option<u32>,
    pub ts_origin: (i32, i32),
    pub font: u32,
    /// 0: clipped by its children, 1: drawing over them too.
    pub subwindow_mode: u8,
    pub exposures: bool,
    pub clip_origin: (i32, i32),
    pub clip: Clip,
    /// 0: chord, 1: pie slice.
    pub arc_mode: u8,
    pub plane_mask: u32,
}

impl Default for Gc {
    fn default() -> Gc {
        Gc {
            function: GX_COPY,
            fg: 0,
            bg: 1,
            line_width: 0,
            line_style: 0,
            cap_style: 1,
            join_style: 0,
            fill_style: 0,
            fill_rule: 0,
            tile: None,
            stipple: None,
            ts_origin: (0, 0),
            font: 0,
            subwindow_mode: 0,
            exposures: true,
            clip_origin: (0, 0),
            clip: Clip::None,
            arc_mode: 1,
            plane_mask: !0,
        }
    }
}

pub const GX_COPY: u8 = 3;

impl Gc {
    /// Sets what a value list (CreateGC's and ChangeGC's) gives, by its mask's bits. False if a value's impossible.
    pub fn set(&mut self, values: &[Option<u32>; 32]) -> bool {
        for (bit, v) in values.iter().enumerate() {
            let Some(v) = *v else { continue };
            match bit {
                0 => self.function = (v & 15) as u8,
                1 => self.plane_mask = v,
                2 => self.fg = v,
                3 => self.bg = v,
                4 => self.line_width = v as u16,
                5 => self.line_style = v as u8,
                6 => self.cap_style = v as u8,
                7 => self.join_style = v as u8,
                8 => self.fill_style = v as u8,
                9 => self.fill_rule = v as u8,
                10 => self.tile = Some(v),
                11 => self.stipple = Some(v),
                12 => self.ts_origin.0 = v as i16 as i32,
                13 => self.ts_origin.1 = v as i16 as i32,
                14 => self.font = v,
                15 => self.subwindow_mode = v as u8,
                16 => self.exposures = v != 0,
                17 => self.clip_origin.0 = v as i16 as i32,
                18 => self.clip_origin.1 = v as i16 as i32,
                19 => self.clip = if v == 0 { Clip::None } else { Clip::Mask(v) },
                22 => self.arc_mode = v as u8,
                _ => {}
            }
        }
        true
    }

    /// Copies the parts `mask` names from another (CopyGC).
    pub fn copy_from(&mut self, from: &Gc, mask: u32) {
        let bit = |b: u32| mask & (1 << b) != 0;
        if bit(0) { self.function = from.function; }
        if bit(1) { self.plane_mask = from.plane_mask; }
        if bit(2) { self.fg = from.fg; }
        if bit(3) { self.bg = from.bg; }
        if bit(4) { self.line_width = from.line_width; }
        if bit(5) { self.line_style = from.line_style; }
        if bit(6) { self.cap_style = from.cap_style; }
        if bit(7) { self.join_style = from.join_style; }
        if bit(8) { self.fill_style = from.fill_style; }
        if bit(9) { self.fill_rule = from.fill_rule; }
        if bit(10) { self.tile = from.tile; }
        if bit(11) { self.stipple = from.stipple; }
        if bit(12) { self.ts_origin.0 = from.ts_origin.0; }
        if bit(13) { self.ts_origin.1 = from.ts_origin.1; }
        if bit(14) { self.font = from.font; }
        if bit(15) { self.subwindow_mode = from.subwindow_mode; }
        if bit(16) { self.exposures = from.exposures; }
        if bit(17) { self.clip_origin.0 = from.clip_origin.0; }
        if bit(18) { self.clip_origin.1 = from.clip_origin.1; }
        if bit(19) { self.clip = from.clip.clone(); }
        if bit(22) { self.arc_mode = from.arc_mode; }
    }
}

/// How the pixels a primitive covers are painted: a GC resolved for one drawing (its tile, stipple and clip mask's
/// pixels read now), in the target's coordinates.
pub struct Paint {
    pub function: u8,
    pub fg: u32,
    pub bg: u32,
    /// The target's depth: a depth-1 pixmap keeps only the low bit.
    pub depth: u8,
    pub fill: Fill,
    /// A clip mask, and where its top left is in the target.
    pub mask: Option<(Surface, i32, i32)>,
}

pub enum Fill {
    Solid,
    /// A pixmap's pixels repeated, from its origin in the target.
    Tiled(Surface, i32, i32),
    /// A bitmap's set pixels in the foreground; its clear ones left alone, or in the background (opaque).
    Stippled(Surface, i32, i32, bool),
}

impl Paint {
    /// The source pixel at a place, or None where nothing's painted.
    fn source(&self, x: i32, y: i32) -> Option<u32> {
        if let Some((m, mx, my)) = &self.mask
            && m.get(x - mx, y - my).unwrap_or(0) & 1 == 0
        {
            return None;
        }
        match &self.fill {
            Fill::Solid => Some(self.fg),
            Fill::Tiled(t, ox, oy) => (t.width > 0 && t.height > 0).then(|| t.pixels[((y - oy).rem_euclid(t.height) * t.width + (x - ox).rem_euclid(t.width)) as usize]),
            Fill::Stippled(s, ox, oy, opaque) => {
                if s.width <= 0 || s.height <= 0 {
                    return Some(self.fg);
                }
                let set = s.pixels[((y - oy).rem_euclid(s.height) * s.width + (x - ox).rem_euclid(s.width)) as usize] & 1 != 0;
                if set { Some(self.fg) } else { opaque.then_some(self.bg) }
            }
        }
    }
}

/// `function` of a source and destination pixel, bit by bit, as X11's sixteen raster operations are.
pub fn combine(function: u8, src: u32, dst: u32) -> u32 {
    match function {
        GX_COPY => src,
        6 => src ^ dst,
        0 => 0,
        15 => !0,
        5 => dst,
        _ => {
            let mut r = 0;
            if function & 1 != 0 { r |= src & dst; }
            if function & 2 != 0 { r |= src & !dst; }
            if function & 4 != 0 { r |= !src & dst; }
            if function & 8 != 0 { r |= !src & !dst; }
            r
        }
    }
}

/// Paints spans into a surface, within a region.
pub fn fill_spans(target: &mut Surface, spans: &[Span], paint: &Paint, clip: &[Rect]) {
    let bounds = target.bounds();
    let keep = if paint.depth == 1 { 1 } else if paint.depth >= 32 { !0 } else { (1u32 << paint.depth) - 1 };
    let simple = matches!(paint.fill, Fill::Solid) && paint.mask.is_none() && paint.function == GX_COPY;
    for &(y, x0, x1) in spans {
        if y < bounds.top || y >= bounds.bottom {
            continue;
        }
        for r in clip {
            if y < r.top || y >= r.bottom {
                continue;
            }
            let (a, b) = (x0.max(r.left).max(0), x1.min(r.right).min(bounds.right));
            if a >= b {
                continue;
            }
            let row = (y * target.width) as usize;
            if simple {
                target.pixels[row + a as usize..row + b as usize].fill(paint.fg & keep);
                continue;
            }
            for x in a..b {
                if let Some(src) = paint.source(x, y) {
                    let at = row + x as usize;
                    target.pixels[at] = combine(paint.function, src, target.pixels[at]) & keep;
                }
            }
        }
    }
}

/// Pixels copied into a surface at `(x, y)`, within a region, through a raster function; `transparent` (if given)
/// marks source pixels left out.
#[allow(clippy::too_many_arguments)]
pub fn blit(target: &mut Surface, x: i32, y: i32, w: i32, h: i32, src: &[u32], function: u8, depth: u8, clip: &[Rect], transparent: Option<&[bool]>) {
    let keep = if depth == 1 { 1 } else if depth >= 32 { !0 } else { (1u32 << depth) - 1 };
    let area = Rect::new(x, y, x + w, y + h).intersect(&target.bounds());
    for r in clip {
        let r = r.intersect(&area);
        for ty in r.top..r.bottom {
            let srow = ((ty - y) * w) as usize;
            let drow = (ty * target.width) as usize;
            for tx in r.left..r.right {
                let s = srow + (tx - x) as usize;
                if transparent.is_some_and(|t| t[s]) {
                    continue;
                }
                let d = drow + tx as usize;
                target.pixels[d] = combine(function, src[s], target.pixels[d]) & keep;
            }
        }
    }
}

/// A rectangle's pixels from a surface, as many as are inside it (0 outside).
pub fn read(src: &Surface, x: i32, y: i32, w: i32, h: i32) -> Vec<u32> {
    let mut out = vec![0u32; (w.max(0) * h.max(0)) as usize];
    for row in 0..h {
        for col in 0..w {
            if let Some(p) = src.get(x + col, y + row) {
                out[(row * w + col) as usize] = p;
            }
        }
    }
    out
}

/// Spans merged row by row, so none overlap.
pub fn merge(spans: Vec<Span>) -> Vec<Span> {
    let mut rows: BTreeMap<i32, Vec<(i32, i32)>> = BTreeMap::new();
    for (y, a, b) in spans {
        if a < b {
            rows.entry(y).or_default().push((a, b));
        }
    }
    let mut out = Vec::new();
    for (y, mut row) in rows {
        row.sort_unstable();
        let mut cur = row[0];
        for &(a, b) in &row[1..] {
            if a <= cur.1 {
                cur.1 = cur.1.max(b);
            } else {
                out.push((y, cur.0, cur.1));
                cur = (a, b);
            }
        }
        out.push((y, cur.0, cur.1));
    }
    out
}

pub fn rect_spans(x: i32, y: i32, w: i32, h: i32, out: &mut Vec<Span>) {
    for row in y..y.saturating_add(h) {
        out.push((row, x, x.saturating_add(w)));
    }
}

/// A polygon's inside, by pixel centres: even-odd, or nonzero winding.
pub fn polygon_spans(points: &[(f64, f64)], winding: bool, out: &mut Vec<Span>) {
    if points.len() < 3 {
        return;
    }
    let ymin = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor() as i32;
    let ymax = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() as i32;
    let mut xs: Vec<(f64, i32)> = Vec::new();
    for y in ymin.max(-32768)..ymax.min(32768) {
        let yc = y as f64 + 0.5;
        xs.clear();
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                let t = (yc - a.1) / (b.1 - a.1);
                xs.push((a.0 + t * (b.0 - a.0), if b.1 > a.1 { 1 } else { -1 }));
            }
        }
        xs.sort_by(|p, q| p.0.total_cmp(&q.0));
        let col = |x: f64| (x - 0.5).ceil() as i32;
        if winding {
            let mut n = 0;
            for i in 0..xs.len() {
                let was = n;
                n += xs[i].1;
                if was == 0 && n != 0 {
                    let start = xs[i].0;
                    let mut j = i + 1;
                    let mut m = n;
                    while j < xs.len() {
                        m += xs[j].1;
                        if m == 0 {
                            break;
                        }
                        j += 1;
                    }
                    if j < xs.len() {
                        out.push((y, col(start), col(xs[j].0)));
                    }
                }
            }
        } else {
            for pair in xs.as_chunks::<2>().0 {
                out.push((y, col(pair[0].0), col(pair[1].0)));
            }
        }
    }
}

/// A one-pixel line, both ends in (or not the last, for `CapNotLast`).
pub fn thin_line(a: (i32, i32), b: (i32, i32), last: bool, out: &mut Vec<Span>) {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - x).abs(), -(b.1 - y).abs());
    let (sx, sy) = (if x < b.0 { 1 } else { -1 }, if y < b.1 { 1 } else { -1 });
    let mut err = dx + dy;
    let mut n = 0;
    loop {
        if (x, y) == b {
            if last {
                out.push((y, x, x + 1));
            }
            return;
        }
        out.push((y, x, x + 1));
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        n += 1;
        if n > 70_000 {
            return;
        }
    }
}

/// A line `width` wide: a quadrilateral along it, with its caps (butt, projecting or round) and round joins where
/// asked.
pub fn wide_line(a: (f64, f64), b: (f64, f64), width: f64, cap: u8, out: &mut Vec<Span>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    let half = width / 2.0;
    if len == 0.0 {
        if cap == 2 {
            disc(a, half, out);
        } else if cap == 3 {
            polygon_spans(&[(a.0 - half, a.1 - half), (a.0 + half, a.1 - half), (a.0 + half, a.1 + half), (a.0 - half, a.1 + half)], false, out);
        }
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (-uy * half, ux * half);
    let ext = if cap == 3 { half } else { 0.0 };
    let (a, b) = ((a.0 - ux * ext, a.1 - uy * ext), (b.0 + ux * ext, b.1 + uy * ext));
    polygon_spans(&[(a.0 + px, a.1 + py), (b.0 + px, b.1 + py), (b.0 - px, b.1 - py), (a.0 - px, a.1 - py)], false, out);
    if cap == 2 {
        disc(a, half, out);
        disc(b, half, out);
    }
}

/// A filled circle.
pub fn disc(c: (f64, f64), r: f64, out: &mut Vec<Span>) {
    let n = ((r * 4.0) as usize).clamp(8, 256);
    let pts: Vec<(f64, f64)> = (0..n).map(|i| {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        (c.0 + r * t.cos(), c.1 + r * t.sin())
    }).collect();
    polygon_spans(&pts, false, out);
}

/// The points along an arc: its bounding box, and its start and extent in 64ths of a degree, counterclockwise from
/// three o'clock.
pub fn arc_points(x: i32, y: i32, w: i32, h: i32, start: i32, extent: i32) -> Vec<(f64, f64)> {
    let (cx, cy, rx, ry) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0, w as f64 / 2.0, h as f64 / 2.0);
    let extent = extent.clamp(-360 * 64, 360 * 64);
    let n = (((rx + ry) * std::f64::consts::PI * (extent.abs() as f64 / (360.0 * 64.0))) as usize / 2).clamp(4, 720);
    let a0 = start as f64 / 64.0 * std::f64::consts::PI / 180.0;
    let da = extent as f64 / 64.0 * std::f64::consts::PI / 180.0;
    (0..=n).map(|i| {
        let t = a0 + da * i as f64 / n as f64;
        (cx + rx * t.cos(), cy - ry * t.sin())
    }).collect()
}

/// A region's rectangles less another rectangle's.
pub fn subtract(parts: &[Rect], cut: &Rect) -> Region {
    let mut out = Vec::new();
    for p in parts {
        let i = p.intersect(cut);
        if i.is_empty() {
            out.push(*p);
            continue;
        }
        for piece in [Rect::new(p.left, p.top, p.right, i.top), Rect::new(p.left, i.bottom, p.right, p.bottom), Rect::new(p.left, i.top, i.left, i.bottom), Rect::new(i.right, i.top, p.right, i.bottom)] {
            if !piece.is_empty() {
                out.push(piece);
            }
        }
    }
    out
}

/// Two regions' common part.
pub fn intersect(a: &[Rect], b: &[Rect]) -> Region {
    let mut out = Vec::new();
    for p in a {
        for q in b {
            let i = p.intersect(q);
            if !i.is_empty() {
                out.push(i);
            }
        }
    }
    out
}

/// Two regions together, without overlaps.
pub fn union(a: &[Rect], b: &[Rect]) -> Region {
    let mut out: Region = a.to_vec();
    for q in b {
        let mut pieces = vec![*q];
        for p in a {
            pieces = subtract(&pieces, p);
        }
        out.extend(pieces);
    }
    out
}

/// A depth-1 picture's set pixels, as rectangles (runs in each row).
pub fn bitmap_region(bits: &Surface, x: i32, y: i32) -> Region {
    let mut out = Vec::new();
    for row in 0..bits.height {
        let mut col = 0;
        while col < bits.width {
            if bits.pixels[(row * bits.width + col) as usize] & 1 == 0 {
                col += 1;
                continue;
            }
            let start = col;
            while col < bits.width && bits.pixels[(row * bits.width + col) as usize] & 1 != 0 {
                col += 1;
            }
            out.push(Rect::new(x + start, y + row, x + col, y + row + 1));
        }
    }
    out
}

/// An image's pixels from a request's data: `format` 0 (a bitmap: set bits the foreground, clear the background), 1
/// (planes, here only depth 1) or 2 (whole pixels at the depth's bits per pixel), rows padded to 32 bits, little-endian.
#[allow(clippy::too_many_arguments)]
pub fn decode_image(format: u8, depth: u8, w: i32, h: i32, left_pad: u8, data: &[u8], fg: u32, bg: u32) -> Option<Vec<u32>> {
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    let mut out = vec![0u32; w * h];
    let bit_rows = |values: (u32, u32), out: &mut Vec<u32>| -> Option<()> {
        let stride = (left_pad as usize + w).div_ceil(32) * 4;
        if data.len() < stride * h {
            return None;
        }
        for y in 0..h {
            for x in 0..w {
                let bit = left_pad as usize + x;
                let set = data[y * stride + bit / 8] >> (bit % 8) & 1 != 0;
                out[y * w + x] = if set { values.0 } else { values.1 };
            }
        }
        Some(())
    };
    match (format, depth) {
        (0, _) => bit_rows((fg, bg), &mut out)?,
        (_, 1) => bit_rows((1, 0), &mut out)?,
        (2, _) => {
            let bpp = bits_per_pixel(depth);
            let stride = (w * bpp).div_ceil(32) * 4;
            if data.len() < stride * h {
                return None;
            }
            for y in 0..h {
                let row = &data[y * stride..];
                for x in 0..w {
                    out[y * w + x] = match bpp {
                        8 => row[x] as u32,
                        16 => u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) as u32,
                        _ => u32::from_le_bytes([row[4 * x], row[4 * x + 1], row[4 * x + 2], row[4 * x + 3]]),
                    };
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// Pixels as a reply's image data: whole pixels at the depth's bits per pixel (or bits, for depth 1), rows padded.
pub fn encode_image(depth: u8, w: i32, h: i32, pixels: &[u32]) -> Vec<u8> {
    let (w, h) = (w.max(0) as usize, h.max(0) as usize);
    if depth == 1 {
        let stride = w.div_ceil(32) * 4;
        let mut out = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                if pixels[y * w + x] & 1 != 0 {
                    out[y * stride + x / 8] |= 1 << (x % 8);
                }
            }
        }
        return out;
    }
    let bpp = bits_per_pixel(depth);
    let stride = (w * bpp).div_ceil(32) * 4;
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            let p = pixels[y * w + x];
            let at = y * stride + x * bpp / 8;
            match bpp {
                8 => out[at] = p as u8,
                16 => out[at..at + 2].copy_from_slice(&(p as u16).to_le_bytes()),
                _ => out[at..at + 4].copy_from_slice(&p.to_le_bytes()),
            }
        }
    }
    out
}

/// The setup reply's pixmap formats: each depth's bits per pixel.
pub fn bits_per_pixel(depth: u8) -> usize {
    match depth {
        1 => 1,
        2..=8 => 8,
        9..=16 => 16,
        _ => 32,
    }
}

/// The depths pixmaps may have, as the setup reply lists them.
pub const DEPTHS: [u8; 7] = [1, 4, 8, 15, 16, 24, 32];

/// The core font: the display's own, `fixed`'s 6 by 13 cell.
pub mod core_font {
    use super::*;

    pub const WIDTH: i32 = font::CELL_WIDTH;
    pub const ASCENT: i32 = font::ASCENT;
    pub const DESCENT: i32 = font::CELL_HEIGHT - font::ASCENT;
    pub const FIRST: u16 = 0;
    pub const LAST: u16 = 255;
    /// The names it answers to in `ListFonts`.
    pub const NAMES: [&str; 3] = ["fixed", "6x13", "-misc-fixed-medium-r-semicondensed--13-120-75-75-c-60-iso8859-1"];

    /// The spans of a string's glyphs, from its first character's origin on the baseline.
    pub fn spans(x: i32, y: i32, chars: &[u16], out: &mut Vec<Span>) {
        for (i, &ch) in chars.iter().enumerate() {
            let cx = x + i as i32 * WIDTH;
            for (px, py) in font::pixels(u32::from(ch)) {
                let row = y - ASCENT + py;
                out.push((row, cx + px, cx + px + 1));
            }
        }
    }
}

/// The cursor font's glyph (an even index; the odd ones are their masks) as the desktop's named cursor.
pub fn cursor_glyph(index: u16) -> Shape {
    match index & !1 {
        152 => Shape::Text,
        150 => Shape::Wait,
        58 | 60 => Shape::Pointer,
        34 | 30 | 90 | 130 => Shape::Crosshair,
        52 => Shape::Move,
        108 => Shape::EwResize,
        116 => Shape::NsResize,
        134 => Shape::NwResize,
        136 => Shape::NeResize,
        12 => Shape::SwResize,
        14 => Shape::SeResize,
        138 => Shape::NResize,
        16 => Shape::SResize,
        70 => Shape::WResize,
        96 => Shape::EResize,
        92 => Shape::Help,
        88 => Shape::NotAllowed,
        _ => Shape::Default,
    }
}

/// Whether a font name matches a pattern with `*` and `?`, ignoring case, as `ListFonts` matches.
pub fn matches(pattern: &str, name: &str) -> bool {
    fn go(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
            (Some(b'?'), Some(_)) => go(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &n[1..]),
            _ => false,
        }
    }
    go(pattern.as_bytes(), name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_twice_is_nothing() {
        let mut s = Surface::new(8, 8, 0x123456);
        let mut spans = Vec::new();
        rect_spans(1, 1, 4, 4, &mut spans);
        rect_spans(3, 3, 4, 4, &mut spans);
        let spans = merge(spans);
        let paint = Paint { function: 6, fg: 0xffffff, bg: 0, depth: 24, fill: Fill::Solid, mask: None };
        let all = [s.bounds()];
        fill_spans(&mut s, &spans, &paint, &all);
        assert_eq!(s.get(3, 3), Some(0x123456 ^ 0xffffff));
        fill_spans(&mut s, &spans, &paint, &all);
        assert!(s.pixels.iter().all(|&p| p == 0x123456));
    }

    #[test]
    fn a_square_polygon_covers_its_pixels() {
        let mut spans = Vec::new();
        polygon_spans(&[(2.0, 2.0), (6.0, 2.0), (6.0, 5.0), (2.0, 5.0)], false, &mut spans);
        assert_eq!(merge(spans), vec![(2, 2, 6), (3, 2, 6), (4, 2, 6)]);
    }

    #[test]
    fn images_round_trip() {
        let px = vec![0x00112233, 0x00445566, 0x00778899];
        let bytes = encode_image(24, 3, 1, &px);
        assert_eq!(decode_image(2, 24, 3, 1, 0, &bytes, 0, 0), Some(px));
        let bits = vec![1, 0, 1, 1, 0];
        let bytes = encode_image(1, 5, 1, &bits);
        assert_eq!(decode_image(2, 1, 5, 1, 0, &bytes, 0, 0), Some(bits));
    }

    #[test]
    fn region_operations() {
        let a = vec![Rect::new(0, 0, 10, 10)];
        let cut = subtract(&a, &Rect::new(2, 2, 4, 4));
        assert_eq!(cut.iter().map(|r| r.width() * r.height()).sum::<i32>(), 96);
        let u = union(&a, &[Rect::new(5, 5, 15, 15)]);
        assert_eq!(u.iter().map(|r| r.width() * r.height()).sum::<i32>(), 175);
        assert!(matches("-*-fixed-medium-r-*", "-misc-fixed-medium-r-semicondensed--13-120-75-75-c-60-iso8859-1"));
        assert!(!matches("helvetica*", "fixed"));
    }
}
