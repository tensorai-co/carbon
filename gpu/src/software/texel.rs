//! Texels: each format's bytes to and from four floats, and sampling with a sampler's filtering and addressing.

use crate::{Address, Filter, Format, ImageDesc, SamplerDesc};

/// An image's texels, tightly packed, rows top first.
pub(crate) struct Texels {
    pub desc: ImageDesc,
    pub data: Vec<u8>,
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

/// Four floats from a texel's bytes; a depth format's value is red, with 0 green and blue and 1 alpha.
pub(crate) fn decode(format: Format, b: &[u8]) -> [f32; 4] {
    let n = |i: usize| b[i] as f32 / 255.0;
    match format {
        Format::Rgba8Unorm => [n(0), n(1), n(2), n(3)],
        Format::Bgra8Unorm => [n(2), n(1), n(0), n(3)],
        Format::Rgba8Srgb => [srgb_to_linear(n(0)), srgb_to_linear(n(1)), srgb_to_linear(n(2)), n(3)],
        Format::Bgra8Srgb => [srgb_to_linear(n(2)), srgb_to_linear(n(1)), srgb_to_linear(n(0)), n(3)],
        Format::Rgba32Float => {
            let f = |i: usize| f32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
            [f(0), f(1), f(2), f(3)]
        }
        Format::D32Float => [f32::from_le_bytes([b[0], b[1], b[2], b[3]]), 0.0, 0.0, 1.0],
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
        Format::Rgba32Float => {
            for (i, x) in c.iter().enumerate() {
                out[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
            }
        }
        Format::D32Float => out[..4].copy_from_slice(&c[0].to_le_bytes()),
        Format::D16Unorm => out[..2].copy_from_slice(&(unorm(c[0], 65535.0) as u16).to_le_bytes()),
    }
}

impl Texels {
    pub fn new(desc: ImageDesc) -> Texels {
        let size = desc.width as usize * desc.height as usize * desc.format.bytes() as usize;
        Texels { desc, data: vec![0; size] }
    }

    fn at(&self, x: u32, y: u32) -> usize {
        (y as usize * self.desc.width as usize + x as usize) * self.desc.format.bytes() as usize
    }

    pub fn read(&self, x: u32, y: u32) -> [f32; 4] {
        let at = self.at(x, y);
        decode(self.desc.format, &self.data[at..at + self.desc.format.bytes() as usize])
    }

    pub fn write(&mut self, x: u32, y: u32, c: [f32; 4]) {
        let (at, n) = (self.at(x, y), self.desc.format.bytes() as usize);
        encode(self.desc.format, c, &mut self.data[at..at + n]);
    }

    /// A sample at a normalized coordinate: the magnification filter at a level of detail at or below 0, the
    /// minification one above (images have one level, so it's always read).
    pub fn sample(&self, s: &SamplerDesc, coord: [f32; 2], lod: f32) -> [f32; 4] {
        let filter = if lod > 0.0 { s.min } else { s.mag };
        let (w, h) = (self.desc.width as i64, self.desc.height as i64);
        let u = coord[0] * w as f32;
        let v = coord[1] * h as f32;
        match filter {
            Filter::Nearest => {
                let x = wrap(u.floor() as i64, w, s.address[0]);
                let y = wrap(v.floor() as i64, h, s.address[1]);
                self.read(x, y)
            }
            Filter::Linear => {
                let (u, v) = (u - 0.5, v - 0.5);
                let (x0, y0) = (u.floor(), v.floor());
                let (a, b) = (u - x0, v - y0);
                let (x0, y0) = (x0 as i64, y0 as i64);
                let xs = [wrap(x0, w, s.address[0]), wrap(x0 + 1, w, s.address[0])];
                let ys = [wrap(y0, h, s.address[1]), wrap(y0 + 1, h, s.address[1])];
                let t = [[self.read(xs[0], ys[0]), self.read(xs[1], ys[0])], [self.read(xs[0], ys[1]), self.read(xs[1], ys[1])]];
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

/// A texel coordinate that may be outside the image, brought inside by the addressing mode.
fn wrap(i: i64, size: i64, mode: Address) -> u32 {
    (match mode {
        Address::Repeat => i.rem_euclid(size),
        Address::MirroredRepeat => {
            let period = 2 * size;
            let m = i.rem_euclid(period);
            if m < size { m } else { period - 1 - m }
        }
        Address::ClampToEdge => i.clamp(0, size - 1),
    }) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip() {
        for format in [Format::Rgba8Unorm, Format::Bgra8Unorm, Format::Rgba8Srgb, Format::Rgba32Float] {
            let mut b = [0u8; 16];
            encode(format, [1.0, 0.0, 0.5, 1.0], &mut b);
            let c = decode(format, &b);
            assert_eq!((c[0], c[1], c[3]), (1.0, 0.0, 1.0), "{format:?}");
            assert!((c[2] - 0.5).abs() < 0.01, "{format:?}: {c:?}");
        }
        let mut b = [0u8; 4];
        encode(Format::Bgra8Unorm, [1.0, 0.0, 0.0, 1.0], &mut b);
        assert_eq!(b, [0, 0, 255, 255]);
    }

    #[test]
    fn addressing() {
        assert_eq!(wrap(-1, 4, Address::Repeat), 3);
        assert_eq!(wrap(4, 4, Address::MirroredRepeat), 3);
        assert_eq!(wrap(-1, 4, Address::MirroredRepeat), 0);
        assert_eq!(wrap(9, 4, Address::ClampToEdge), 3);
    }
}
