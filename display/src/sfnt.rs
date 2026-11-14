//! OpenType fonts as their tables have them: the metrics a face's `head`, `hhea`, `OS/2` and `post` give, its
//! character map (`cmap`'s formats 4 and 12), its glyphs' advances and side bearings (`hmtx`, `vmtx`), and a font file
//! put together again from its tables, for programs that read one. The tables come from wherever the face does (the
//! host's fonts, for atlas); nothing here reads a file.

/// A big-endian reader over a table, giving 0 past its end, as a font with a short table is still drawn.
#[derive(Clone, Copy)]
struct Be<'a>(&'a [u8]);

impl Be<'_> {
    fn u16(&self, at: usize) -> u16 {
        self.0.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&self, at: usize) -> i16 {
        self.u16(at) as i16
    }
    fn u32(&self, at: usize) -> u32 {
        self.0.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// What a face's tables say of it as a whole, in font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Metrics {
    pub units_per_em: u16,
    /// Typographic: above and below the baseline (both positive), and the gap between lines.
    pub ascent: u16,
    pub descent: u16,
    pub line_gap: i16,
    pub cap_height: u16,
    pub x_height: u16,
    /// Below the baseline is negative, as OpenType has it.
    pub underline_position: i16,
    pub underline_thickness: u16,
    pub strikeout_position: i16,
    pub strikeout_thickness: u16,
    /// Every glyph's box together: x min, y min, x max, y max.
    pub bounds: [i16; 4],
    pub weight: u16,
    pub width: u16,
    pub italic: bool,
    pub fixed_pitch: bool,
    pub glyph_count: u16,
    pub h_metrics: u16,
}

impl Metrics {
    /// From the tables (any may be missing but `head`): the ascent and descent `OS/2`'s Windows metrics give, as
    /// Windows uses them for lines, else `hhea`'s.
    pub fn read(head: &[u8], hhea: Option<&[u8]>, os2: Option<&[u8]>, post: Option<&[u8]>, maxp: Option<&[u8]>) -> Option<Metrics> {
        let head = Be(head);
        if head.u32(12) != 0x5f0f_3cf5 {
            return None;
        }
        let units_per_em = head.u16(18).max(16);
        let mut m = Metrics {
            units_per_em,
            bounds: [head.i16(36), head.i16(38), head.i16(40), head.i16(42)],
            italic: head.u16(44) & 2 != 0,
            weight: if head.u16(44) & 1 != 0 { 700 } else { 400 },
            width: 5,
            glyph_count: maxp.map_or(0, |m| Be(m).u16(4)),
            ..Metrics::default()
        };
        if let Some(h) = hhea.map(Be) {
            m.ascent = h.i16(4).max(0) as u16;
            m.descent = (-i32::from(h.i16(6))).clamp(0, 0xffff) as u16;
            m.line_gap = h.i16(8);
            m.h_metrics = h.u16(34);
        }
        if let Some(o) = os2.map(Be).filter(|o| o.0.len() >= 78) {
            m.weight = o.u16(4).clamp(1, 1000);
            m.width = o.u16(6).clamp(1, 9);
            m.strikeout_thickness = o.u16(26);
            m.strikeout_position = o.i16(28);
            m.italic = o.u16(62) & 1 != 0;
            let (win_ascent, win_descent) = (o.u16(74), o.u16(76));
            if win_ascent != 0 || win_descent != 0 {
                m.ascent = win_ascent;
                m.descent = win_descent;
                // Windows' line gap from the Windows metrics: what hhea's line height has beyond them.
                if let Some(h) = hhea.map(Be) {
                    let hhea_height = i32::from(h.i16(4)) - i32::from(h.i16(6)) + i32::from(h.i16(8));
                    m.line_gap = (hhea_height - i32::from(win_ascent) - i32::from(win_descent)).clamp(0, 0x7fff) as i16;
                }
            }
            if o.u16(0) >= 2 && o.0.len() >= 90 {
                m.x_height = o.i16(86).max(0) as u16;
                m.cap_height = o.i16(88).max(0) as u16;
            }
        }
        if m.cap_height == 0 {
            m.cap_height = (u32::from(units_per_em) * 7 / 10) as u16;
        }
        if m.x_height == 0 {
            m.x_height = (u32::from(units_per_em) / 2) as u16;
        }
        if m.strikeout_thickness == 0 {
            m.strikeout_thickness = units_per_em / 20;
            m.strikeout_position = (m.x_height / 2) as i16;
        }
        if let Some(p) = post.map(Be) {
            m.underline_position = p.i16(8);
            m.underline_thickness = p.u16(10);
            m.fixed_pitch = p.u32(12) != 0;
        }
        if m.underline_thickness == 0 {
            m.underline_thickness = units_per_em / 20;
            m.underline_position = -((units_per_em / 10) as i16);
        }
        Some(m)
    }
}

/// A face's map from characters to glyphs.
#[derive(Debug, Clone)]
pub struct Cmap {
    /// Ranges of characters, each `(first, last, delta)` (glyph = c + delta) or `(first, last, glyph of first)` for
    /// format 12's.
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    first: u32,
    last: u32,
    kind: SegmentKind,
}

#[derive(Debug, Clone, Copy)]
enum SegmentKind {
    /// Format 4 with no glyph array: glyph = (c + delta) mod 65536.
    Delta(u16),
    /// Format 4 through its glyph array: the array's index of `first`, and delta.
    Array(usize, u16),
    /// Format 12: the glyph of `first`, the rest in order.
    Sequential(u32),
}

impl Cmap {
    /// The best Unicode subtable a `cmap` has: (3,10) or (0,4..6) in format 12, else (3,1) or (0,3) in format 4, else
    /// (3,0) for a symbol font.
    pub fn read(table: &[u8]) -> Option<Cmap> {
        let t = Be(table);
        let count = usize::from(t.u16(2));
        let mut best: Option<(u8, usize)> = None;
        for i in 0..count {
            let (platform, encoding, offset) = (t.u16(4 + i * 8), t.u16(6 + i * 8), t.u32(8 + i * 8) as usize);
            let format = t.u16(offset);
            let rank = match (platform, encoding, format) {
                (3, 10, 12) | (0, 4..=6, 12) => 4,
                (3, 1, 4) | (0, 0..=3, 4) => 3,
                (3, 0, 4) => 2,
                _ => 0,
            };
            if rank > 0 && best.is_none_or(|(r, _)| rank > r) {
                best = Some((rank, offset));
            }
        }
        let (_, offset) = best?;
        let sub = Be(table.get(offset..)?);
        let mut segments = Vec::new();
        match sub.u16(0) {
            4 => {
                let n = usize::from(sub.u16(6) / 2);
                let (ends, starts, deltas, ranges) = (14, 16 + 2 * n, 16 + 4 * n, 16 + 6 * n);
                for s in 0..n {
                    let (first, last) = (u32::from(sub.u16(starts + 2 * s)), u32::from(sub.u16(ends + 2 * s)));
                    if first > last || first == 0xffff {
                        continue;
                    }
                    let (delta, range) = (sub.u16(deltas + 2 * s), usize::from(sub.u16(ranges + 2 * s)));
                    let kind = if range == 0 { SegmentKind::Delta(delta) } else { SegmentKind::Array(ranges + 2 * s + range, delta) };
                    segments.push(Segment { first, last, kind });
                }
            }
            12 => {
                let n = sub.u32(12) as usize;
                for g in 0..n.min(sub.0.len() / 12) {
                    let at = 16 + 12 * g;
                    segments.push(Segment { first: sub.u32(at), last: sub.u32(at + 4), kind: SegmentKind::Sequential(sub.u32(at + 8)) });
                }
            }
            _ => return None,
        }
        // Format 4's array is read through the subtable, which is kept as its segments' source.
        let array: Vec<u8> = if segments.iter().any(|s| matches!(s.kind, SegmentKind::Array(..))) { sub.0.to_vec() } else { Vec::new() };
        Some(Cmap { segments: segments.into_iter().map(|s| Self::resolve(s, &array)).collect::<Vec<_>>().concat() })
    }

    /// A format 4 segment through its glyph array, as format 12's runs of one glyph each where they're not 0.
    fn resolve(s: Segment, sub: &[u8]) -> Vec<Segment> {
        match s.kind {
            SegmentKind::Array(at, delta) => (s.first..=s.last)
                .filter_map(|c| {
                    let g = Be(sub).u16(at + 2 * (c - s.first) as usize);
                    (g != 0).then(|| Segment { first: c, last: c, kind: SegmentKind::Sequential(u32::from(g.wrapping_add(delta))) })
                })
                .collect(),
            _ => vec![s],
        }
    }

    /// A character's glyph: 0 (.notdef) for one it hasn't.
    pub fn glyph(&self, c: u32) -> u16 {
        let i = self.segments.partition_point(|s| s.last < c);
        // Segments are in order, but format 4's resolved ones may interleave with their neighbours: look at those
        // that could hold it.
        for s in &self.segments[i.saturating_sub(1)..] {
            if s.first > c && s.last > c {
                break;
            }
            if (s.first..=s.last).contains(&c) {
                return match s.kind {
                    SegmentKind::Delta(d) => (c as u16).wrapping_add(d),
                    SegmentKind::Sequential(g) => (g + (c - s.first)) as u16,
                    SegmentKind::Array(..) => 0,
                };
            }
        }
        0
    }
}

/// A glyph's advance and side bearing, from `hmtx` (or `vmtx`): glyphs past the last full entry share its advance.
pub fn advance(mtx: &[u8], long_metrics: u16, glyph: u16) -> (u16, i16) {
    let t = Be(mtx);
    let n = usize::from(long_metrics.max(1));
    let g = usize::from(glyph);
    if g < n {
        (t.u16(4 * g), t.i16(4 * g + 2))
    } else {
        (t.u16(4 * (n - 1)), t.i16(4 * n + 2 * (g - n)))
    }
}

/// A font file made of these tables: the table directory (checksummed, in tag order, each table on four bytes), as
/// `IDWriteFontFileStream` and the like hand a program its font.
pub fn build(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut tables: Vec<&([u8; 4], Vec<u8>)> = tables.iter().collect();
    tables.sort_by_key(|t| t.0);
    let n = tables.len() as u16;
    let power = if n == 0 { 0 } else { 1u16 << (15 - n.leading_zeros()) };
    let search = power * 16;
    let selector = power.trailing_zeros() as u16;
    // CFF outlines make it OpenType ("OTTO"), else TrueType.
    let version: u32 = if tables.iter().any(|t| &t.0 == b"CFF " || &t.0 == b"CFF2") { 0x4f54_544f } else { 0x0001_0000 };
    let mut out = Vec::new();
    out.extend(version.to_be_bytes());
    for v in [n, search, selector, n * 16 - search] {
        out.extend(v.to_be_bytes());
    }
    let mut at = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend(tag);
        out.extend(checksum(data).to_be_bytes());
        out.extend((at as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        body.extend(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        at = 12 + 16 * tables.len() + body.len();
    }
    out.extend(body);
    out
}

/// OpenType's table checksum: the sum of its big-endian words, the last padded.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn a_format_4_map_by_delta_and_by_its_array() {
        // Two segments, 'A'..'C' by delta (glyphs 1..3) and 'a'..'b' through the array (glyphs 7 and 0), then 0xFFFF.
        let n = 3u16;
        let mut sub = be(&[4, 0, 0, n * 2, 0, 0, 0]);
        sub.extend(be(&[0x43, 0x62, 0xffff, 0])); // ends, then the pad
        sub.extend(be(&[0x41, 0x61, 0xffff])); // starts
        sub.extend(be(&[(1i32 - 0x41) as u16, 0, 1])); // deltas
        // Range offsets: the second's array follows the offsets, 4 bytes after its own entry.
        sub.extend(be(&[0, 4, 0]));
        sub.extend(be(&[7, 0]));
        let mut cmap = be(&[0, 1, 3, 1, 0, 12]);
        cmap.extend(sub);
        let map = Cmap::read(&cmap).unwrap();
        assert_eq!([map.glyph(0x41), map.glyph(0x43), map.glyph(0x44), map.glyph(0x61), map.glyph(0x62), map.glyph(0x20)], [1, 3, 0, 7, 0, 0]);
    }

    #[test]
    fn a_format_12_map_beyond_the_bmp() {
        let mut sub = be(&[12, 0]);
        sub.extend(28u32.to_be_bytes());
        sub.extend(0u32.to_be_bytes());
        sub.extend(1u32.to_be_bytes());
        for v in [0x1f600u32, 0x1f602, 50] {
            sub.extend(v.to_be_bytes());
        }
        let mut cmap = be(&[0, 1, 3, 10, 0, 12]);
        cmap.extend(sub);
        let map = Cmap::read(&cmap).unwrap();
        assert_eq!([map.glyph(0x1f600), map.glyph(0x1f602), map.glyph(0x1f603)], [50, 52, 0]);
    }

    #[test]
    fn advances_past_the_long_metrics() {
        let hmtx = be(&[500, 10, 600, 20, 30, 40]);
        assert_eq!(advance(&hmtx, 2, 1), (600, 20));
        assert_eq!(advance(&hmtx, 2, 3), (600, 40));
    }

    #[test]
    fn a_file_from_its_tables() {
        let file = build(&[(*b"head", vec![1, 2, 3]), (*b"cmap", vec![9; 8])]);
        assert_eq!(&file[0..4], &[0, 1, 0, 0]);
        assert_eq!(Be(&file).u16(4), 2);
        // cmap sorts first; its data at 44, head's after it at 52, padded.
        assert_eq!(&file[12..16], b"cmap");
        assert_eq!((Be(&file).u32(20), Be(&file).u32(36)), (44, 52));
        assert_eq!(&file[52..55], &[1, 2, 3]);
        assert_eq!(file.len(), 56);
    }
}
