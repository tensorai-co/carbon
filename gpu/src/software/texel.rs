//! Texels: each format's bytes to and from four floats, and sampling with a sampler's filtering, addressing, levels
//! of detail, border colour and comparison, by Vulkan's rules.

use crate::{Address, Filter, Format, ImageDesc, SamplerDesc, Sub};

/// An image's texels, tightly packed, rows top first: each layer's levels in turn, largest first, layer after layer.
pub(crate) struct Texels {
    pub desc: ImageDesc,
    pub data: Vec<u8>,
    /// Where each layer's level starts in `data`, at `layer * levels + level`.
    starts: Vec<usize>,
}

/// sRGB's transfer function, from encoded to linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A float to an n-bit unsigned normalized integer: clamped to `[0, 1]`, rounded to nearest.
fn unorm(c: f32, max: f32) -> u32 {
    let c = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
    (c * max + 0.5).floor() as u32
}

/// `v >> shift`, rounded to nearest, ties to even.
fn round_shift(v: u32, shift: u32) -> u32 {
    if shift == 0 {
        return v;
    }
    if shift >= 32 {
        return 0;
    }
    let (q, rem, half) = (v >> shift, v & ((1 << shift) - 1), 1 << (shift - 1));
    if rem > half || (rem == half && q & 1 == 1) { q + 1 } else { q }
}

/// A float's magnitude as a small float with a 5-bit exponent (bias 15) and `m` bits of mantissa, rounded to
/// nearest even: an IEEE half's lower 15 bits for `m` 10, and the unsigned 11- and 10-bit floats for 6 and 5.
fn small_float(x: f32, m: u32) -> u32 {
    let b = x.to_bits() & 0x7fff_ffff;
    let (exp, man) = ((b >> 23) as i32, b & 0x7f_ffff);
    let inf = 0x1f << m;
    if exp == 0xff {
        return if man != 0 { inf | 1 << (m - 1) } else { inf };
    }
    let e = exp - 112;
    let v = if e > 0 { round_shift(((e as u32) << 23) | man, 23 - m) } else { round_shift(man | 0x80_0000, 23 - m + (1 - e) as u32) };
    v.min(inf)
}

/// A small float's value, as [`small_float`] lays it out.
fn from_small_float(v: u32, m: u32) -> f32 {
    let (exp, man) = (v >> m, v & ((1 << m) - 1));
    match exp {
        0 => man as f32 * (2.0f32).powi(-14 - m as i32),
        0x1f => f32::from_bits(0x7f80_0000 | man << (23 - m)),
        _ => f32::from_bits((exp + 112) << 23 | man << (23 - m)),
    }
}

pub(crate) fn f32_to_f16(x: f32) -> u16 {
    let sign = ((x.to_bits() >> 16) & 0x8000) as u16;
    sign | small_float(x, 10) as u16
}

pub(crate) fn f16_to_f32(h: u16) -> f32 {
    let v = from_small_float(u32::from(h & 0x7fff), 10);
    if h & 0x8000 != 0 { -v } else { v }
}

/// An unsigned small float: negative values and negative zero are 0.
fn unsigned_small(x: f32, m: u32) -> u32 {
    if x.is_nan() {
        return small_float(x, m);
    }
    if x.is_sign_negative() { 0 } else { small_float(x, m) }
}

/// Four floats from a texel's bytes; a depth format's value is red, with 0 green and blue and 1 alpha. Channels a
/// format doesn't have read 0, and alpha 1.
pub(crate) fn decode(format: Format, b: &[u8]) -> [f32; 4] {
    let n = |i: usize| b[i] as f32 / 255.0;
    let word = || u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    match format {
        Format::Rgba8Unorm => [n(0), n(1), n(2), n(3)],
        Format::Bgra8Unorm => [n(2), n(1), n(0), n(3)],
        Format::Rgba8Srgb => [srgb_to_linear(n(0)), srgb_to_linear(n(1)), srgb_to_linear(n(2)), n(3)],
        Format::Bgra8Srgb => [srgb_to_linear(n(2)), srgb_to_linear(n(1)), srgb_to_linear(n(0)), n(3)],
        Format::R8Unorm => [n(0), 0.0, 0.0, 1.0],
        Format::Rg8Unorm => [n(0), n(1), 0.0, 1.0],
        Format::A8Unorm => [0.0, 0.0, 0.0, n(0)],
        Format::R32Float => [f32::from_bits(word()), 0.0, 0.0, 1.0],
        Format::Rgba16Float => [0, 1, 2, 3].map(|i| f16_to_f32(u16::from_le_bytes([b[2 * i], b[2 * i + 1]]))),
        Format::Rgb10a2Unorm => {
            let w = word();
            let ten = |shift: u32| ((w >> shift) & 0x3ff) as f32 / 1023.0;
            [ten(0), ten(10), ten(20), (w >> 30) as f32 / 3.0]
        }
        Format::Rg11b10Float => {
            let w = word();
            [from_small_float(w & 0x7ff, 6), from_small_float((w >> 11) & 0x7ff, 6), from_small_float(w >> 22, 5), 1.0]
        }
        Format::Rgba32Float => {
            let f = |i: usize| f32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
            [f(0), f(1), f(2), f(3)]
        }
        Format::D32Float => [f32::from_bits(word()), 0.0, 0.0, 1.0],
        Format::D16Unorm => [u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0, 0.0, 0.0, 1.0],
    }
}

/// A texel's bytes from four floats.
pub(crate) fn encode(format: Format, c: [f32; 4], out: &mut [u8]) {
    let u8n = |x: f32| unorm(x, 255.0) as u8;
    match format {
        Format::Rgba8Unorm => out[..4].copy_from_slice(&[u8n(c[0]), u8n(c[1]), u8n(c[2]), u8n(c[3])]),
        Format::Bgra8Unorm => out[..4].copy_from_slice(&[u8n(c[2]), u8n(c[1]), u8n(c[0]), u8n(c[3])]),
        Format::Rgba8Srgb | Format::Bgra8Srgb => {
            let (r, g, b) = (u8n(linear_to_srgb(c[0])), u8n(linear_to_srgb(c[1])), u8n(linear_to_srgb(c[2])));
            let bytes = if format == Format::Rgba8Srgb { [r, g, b, u8n(c[3])] } else { [b, g, r, u8n(c[3])] };
            out[..4].copy_from_slice(&bytes);
        }
        Format::R8Unorm => out[0] = u8n(c[0]),
        Format::Rg8Unorm => out[..2].copy_from_slice(&[u8n(c[0]), u8n(c[1])]),
        Format::A8Unorm => out[0] = u8n(c[3]),
        Format::R32Float | Format::D32Float => out[..4].copy_from_slice(&c[0].to_le_bytes()),
        Format::Rgba16Float => {
            for (i, x) in c.iter().enumerate() {
                out[2 * i..2 * i + 2].copy_from_slice(&f32_to_f16(*x).to_le_bytes());
            }
        }
        Format::Rgb10a2Unorm => {
            let w = unorm(c[0], 1023.0) | unorm(c[1], 1023.0) << 10 | unorm(c[2], 1023.0) << 20 | unorm(c[3], 3.0) << 30;
            out[..4].copy_from_slice(&w.to_le_bytes());
        }
        Format::Rg11b10Float => {
            let w = unsigned_small(c[0], 6) | unsigned_small(c[1], 6) << 11 | unsigned_small(c[2], 5) << 22;
            out[..4].copy_from_slice(&w.to_le_bytes());
        }
        Format::Rgba32Float => {
            for (i, x) in c.iter().enumerate() {
                out[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
            }
        }
        Format::D16Unorm => out[..2].copy_from_slice(&(unorm(c[0], 65535.0) as u16).to_le_bytes()),
    }
}

impl Texels {
    pub fn new(desc: ImageDesc) -> Texels {
        let bpp = desc.format.bytes() as usize;
        let mut starts = Vec::new();
        let mut size = 0;
        for _ in 0..desc.layers {
            for level in 0..desc.levels {
                starts.push(size);
                let (w, h) = desc.size(level);
                size += w as usize * h as usize * bpp;
            }
        }
        Texels { desc, data: vec![0; size], starts }
    }

    /// A byte offset of a texel in one level of one layer.
    pub fn at(&self, sub: Sub, x: u32, y: u32) -> usize {
        let (w, _) = self.desc.size(sub.level);
        let start = self.starts[(sub.layer * self.desc.levels + sub.level) as usize];
        start + (y as usize * w as usize + x as usize) * self.desc.format.bytes() as usize
    }

    pub fn read(&self, sub: Sub, x: u32, y: u32) -> [f32; 4] {
        let at = self.at(sub, x, y);
        decode(self.desc.format, &self.data[at..at + self.desc.format.bytes() as usize])
    }

    pub fn write(&mut self, sub: Sub, x: u32, y: u32, c: [f32; 4]) {
        let (at, n) = (self.at(sub, x, y), self.desc.format.bytes() as usize);
        encode(self.desc.format, c, &mut self.data[at..at + n]);
    }

    /// Every texel of one level of one layer set to a value.
    pub fn clear(&mut self, sub: Sub, c: [f32; 4]) {
        let (w, h) = self.desc.size(sub.level);
        let mut texel = [0u8; 16];
        let n = self.desc.format.bytes() as usize;
        encode(self.desc.format, c, &mut texel);
        let start = self.at(sub, 0, 0);
        for t in self.data[start..start + w as usize * h as usize * n].chunks_exact_mut(n) {
            t.copy_from_slice(&texel[..n]);
        }
    }

    /// Every level after the first made from the one before, in each layer: each texel the average of the (up to)
    /// four it covers, in linear colour.
    pub fn generate_mips(&mut self) {
        for layer in 0..self.desc.layers {
            for level in 1..self.desc.levels {
                let from = Sub { level: level - 1, layer };
                let (pw, ph) = self.desc.size(level - 1);
                let (w, h) = self.desc.size(level);
                for y in 0..h {
                    for x in 0..w {
                        let mut sum = [0.0f32; 4];
                        let xs = [(2 * x).min(pw - 1), (2 * x + 1).min(pw - 1)];
                        let ys = [(2 * y).min(ph - 1), (2 * y + 1).min(ph - 1)];
                        for (sy, sx) in ys.iter().flat_map(|y| xs.iter().map(move |x| (*y, *x))) {
                            let t = self.read(from, sx, sy);
                            for c in 0..4 {
                                sum[c] += t[c];
                            }
                        }
                        self.write(Sub { level, layer }, x, y, sum.map(|s| s / 4.0));
                    }
                }
            }
        }
    }

    /// A sample of layer 0 at a normalized coordinate and level of detail, by Vulkan's rules: the level of detail
    /// kept to the sampler's range; the magnification filter at or below 0 and the minification one above; the
    /// nearest level, or the two around it blended, by the mipmap filter. With a reference value, a comparison
    /// sampler's texels are each 1 or 0 by its test, before filtering.
    pub fn sample(&self, s: &SamplerDesc, coord: [f32; 2], lod: f32, reference: Option<f32>) -> [f32; 4] {
        let lod = lod.max(s.lod[0]).min(s.lod[1]);
        let lod = if lod.is_nan() { 0.0 } else { lod };
        let filter = if lod > 0.0 { s.min } else { s.mag };
        let q = (self.desc.levels - 1) as f32;
        let d = lod.clamp(0.0, q);
        let one = |level: u32| self.sample_level(s, filter, level, coord, reference);
        match s.mipmap {
            Filter::Nearest => one(if lod > 0.0 { ((d + 0.5).ceil() - 1.0).max(0.0) as u32 } else { 0 }),
            Filter::Linear => {
                let hi = d.floor();
                let delta = d - hi;
                let a = one(hi as u32);
                if delta == 0.0 {
                    return a;
                }
                let b = one((hi as u32 + 1).min(self.desc.levels - 1));
                [0, 1, 2, 3].map(|c| a[c] * (1.0 - delta) + b[c] * delta)
            }
        }
    }

    fn sample_level(&self, s: &SamplerDesc, filter: Filter, level: u32, coord: [f32; 2], reference: Option<f32>) -> [f32; 4] {
        let sub = Sub { level, layer: 0 };
        let (w, h) = self.desc.size(level);
        let (w, h) = (w as i64, h as i64);
        let border = s.border.rgba();
        let fetch = |x: i64, y: i64| -> [f32; 4] {
            let t = match (wrap(x, w, s.address[0]), wrap(y, h, s.address[1])) {
                (Some(x), Some(y)) => self.read(sub, x, y),
                _ => border,
            };
            match (reference, s.compare) {
                (Some(r), Some(test)) => [if test.passes(r, t[0]) { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0],
                _ => t,
            }
        };
        let u = coord[0] * w as f32;
        let v = coord[1] * h as f32;
        match filter {
            Filter::Nearest => fetch(u.floor() as i64, v.floor() as i64),
            Filter::Linear => {
                let (u, v) = (u - 0.5, v - 0.5);
                let (x0, y0) = (u.floor(), v.floor());
                let (a, b) = (u - x0, v - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let t = [[fetch(x0, y0), fetch(x0 + 1, y0)], [fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1)]];
                let mut out = [0.0; 4];
                for (i, o) in out.iter_mut().enumerate() {
                    let top = t[0][0][i] * (1.0 - a) + t[0][1][i] * a;
                    let bottom = t[1][0][i] * (1.0 - a) + t[1][1][i] * a;
                    *o = top * (1.0 - b) + bottom * b;
                }
                out
            }
        }
    }
}

/// A texel coordinate that may be outside the image, brought inside by the addressing mode; `None` for the border.
fn wrap(i: i64, size: i64, mode: Address) -> Option<u32> {
    Some(match mode {
        Address::Repeat => i.rem_euclid(size),
        Address::MirroredRepeat => {
            let period = 2 * size;
            let m = i.rem_euclid(period);
            if m < size { m } else { period - 1 - m }
        }
        Address::ClampToEdge => i.clamp(0, size - 1),
        Address::ClampToBorder if !(0..size).contains(&i) => return None,
        Address::ClampToBorder => i,
    } as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Border, Compare};

    #[test]
    fn formats_round_trip() {
        for format in [Format::Rgba8Unorm, Format::Bgra8Unorm, Format::Rgba8Srgb, Format::Rgba32Float, Format::Rgba16Float, Format::Rgb10a2Unorm] {
            let mut b = [0u8; 16];
            encode(format, [1.0, 0.0, 0.5, 1.0], &mut b);
            let c = decode(format, &b);
            assert_eq!((c[0], c[1], c[3]), (1.0, 0.0, 1.0), "{format:?}");
            assert!((c[2] - 0.5).abs() < 0.01, "{format:?}: {c:?}");
        }
        let mut b = [0u8; 4];
        encode(Format::Bgra8Unorm, [1.0, 0.0, 0.0, 1.0], &mut b);
        assert_eq!(b, [0, 0, 255, 255]);
        encode(Format::Rg11b10Float, [1.0, 0.5, 2.0, 1.0], &mut b);
        assert_eq!(decode(Format::Rg11b10Float, &b), [1.0, 0.5, 2.0, 1.0]);
        encode(Format::Rg11b10Float, [-1.0, 0.0, 1e9, 1.0], &mut b);
        assert_eq!(decode(Format::Rg11b10Float, &b), [0.0, 0.0, f32::INFINITY, 1.0]);
        encode(Format::A8Unorm, [1.0, 1.0, 1.0, 0.2], &mut b);
        assert_eq!(b[0], 51);
        assert_eq!(decode(Format::A8Unorm, &b), [0.0, 0.0, 0.0, 0.2]);
    }

    #[test]
    fn halves() {
        for (x, h) in [(1.0f32, 0x3c00u16), (-2.0, 0xc000), (65504.0, 0x7bff), (1e6, 0x7c00), (5.960_464_5e-8, 1), (0.0, 0), (f32::INFINITY, 0x7c00)] {
            assert_eq!(f32_to_f16(x), h, "{x}");
            if h != 0x7c00 || x.is_infinite() {
                assert_eq!(f16_to_f32(h), x, "{h:#x}");
            }
        }
        // Ties to even: 1 + 2^-11 is halfway between 1 and the next half, and goes to 1.
        assert_eq!(f32_to_f16(1.0 + (2.0f32).powi(-11)), 0x3c00);
        assert_eq!(f32_to_f16(1.0 + 3.0 * (2.0f32).powi(-11)), 0x3c02);
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
    }

    #[test]
    fn addressing() {
        assert_eq!(wrap(-1, 4, Address::Repeat), Some(3));
        assert_eq!(wrap(4, 4, Address::MirroredRepeat), Some(3));
        assert_eq!(wrap(-1, 4, Address::MirroredRepeat), Some(0));
        assert_eq!(wrap(9, 4, Address::ClampToEdge), Some(3));
        assert_eq!(wrap(4, 4, Address::ClampToBorder), None);
        assert_eq!(wrap(3, 4, Address::ClampToBorder), Some(3));
    }

    #[test]
    fn levels_and_layers() {
        let desc = ImageDesc { format: Format::R8Unorm, width: 4, height: 2, levels: 3, layers: 2 };
        let mut t = Texels::new(desc);
        // 8 + 2 + 1 bytes a layer.
        assert_eq!(t.data.len(), 22);
        assert_eq!(t.at(Sub { level: 2, layer: 1 }, 0, 0), 21);
        for (i, v) in [0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 28.0].iter().enumerate() {
            t.write(Sub::default(), i as u32 % 4, i as u32 / 4, [v / 255.0, 0.0, 0.0, 1.0]);
        }
        t.generate_mips();
        let at = |t: &Texels, level, x| (t.read(Sub { level, layer: 0 }, x, 0)[0] * 255.0).round();
        // Level 1 is 2×1: (0 + 4 + 16 + 20) / 4 and (8 + 12 + 24 + 28) / 4; level 2 is their average.
        assert_eq!((at(&t, 1, 0), at(&t, 1, 1), at(&t, 2, 0)), (10.0, 18.0, 14.0));
        let mut s = SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToEdge; 2]);
        let sample = |s: &SamplerDesc, lod| (t.sample(s, [0.1, 0.25], lod, None)[0] * 255.0).round();
        assert_eq!((sample(&s, 0.0), sample(&s, 0.9), sample(&s, 1.6), sample(&s, 9.0)), (0.0, 10.0, 14.0, 14.0));
        s.lod = [0.0, 1.0];
        assert_eq!(sample(&s, 9.0), 10.0);
        s.mipmap = Filter::Linear;
        s.lod = [0.0, 1000.0];
        assert_eq!(sample(&s, 1.5), 12.0);
    }

    #[test]
    fn border_and_comparison() {
        let mut t = Texels::new(ImageDesc::new(Format::D32Float, 2, 1));
        t.write(Sub::default(), 0, 0, [0.25, 0.0, 0.0, 1.0]);
        t.write(Sub::default(), 1, 0, [0.75, 0.0, 0.0, 1.0]);
        let mut s = SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToBorder; 2]);
        s.border = Border::OpaqueWhite;
        assert_eq!(t.sample(&s, [1.5, 0.5], 0.0, None), [1.0; 4]);
        // The reference against each texel: 0.5 < 0.25 fails, 0.5 < 0.75 passes.
        s.compare = Some(Compare::Less);
        assert_eq!(t.sample(&s, [0.25, 0.5], 0.0, Some(0.5))[0], 0.0);
        assert_eq!(t.sample(&s, [0.75, 0.5], 0.0, Some(0.5))[0], 1.0);
        // Linear, between the two: percentage-closer filtering gives a half.
        s.mag = Filter::Linear;
        s.address = [Address::ClampToEdge; 2];
        assert_eq!(t.sample(&s, [0.5, 0.5], 0.0, Some(0.5))[0], 0.5);
    }
}
