//! Pictures: device-independent bitmaps (a `BITMAP` resource, a `.bmp` file) and icons and cursors (`ICON`, `CURSOR`
//! and their groups, `.ico` and `.cur` files), decoded into pixels GDI draws with.
//!
//! A DIB is a `BITMAPINFOHEADER` (or `BITMAPCOREHEADER`, or a later, longer header), its palette or bit fields, then
//! its rows, bottom-up unless its height is negative, each padded to four bytes: 1, 4 and 8 bits a pixel through the
//! palette, 16 (5-5-5, or its bit fields), 24 and 32. An icon's image is a DIB of twice its height, its colours and
//! then its AND mask (a set bit is transparent); a 32-bit one's own alpha says how it blends, where it has any. An
//! icon's PNG images (Vista's large ones) aren't decoded yet: another size in its group is chosen instead.

use display::{Rect, Surface};

/// A picture with alpha: `0xAARRGGBB` a pixel, rows top first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
    /// A cursor's hot spot.
    pub hotspot: (i32, i32),
}

impl Image {
    /// Opaque, on `background` where it's transparent: as a bitmap is.
    pub fn to_surface(&self) -> Surface {
        let mut s = Surface::new(self.width, self.height, 0);
        for (d, &p) in s.pixels.iter_mut().zip(&self.pixels) {
            *d = p & 0xff_ffff;
        }
        s
    }

    pub fn from_surface(s: &Surface) -> Image {
        Image { width: s.width, height: s.height, pixels: s.pixels.iter().map(|&p| 0xff00_0000 | p).collect(), hotspot: (0, 0) }
    }

    /// Scaled to a size, each pixel the nearest.
    pub fn scaled(&self, width: i32, height: i32) -> Image {
        if (width, height) == (self.width, self.height) || width <= 0 || height <= 0 || self.width <= 0 || self.height <= 0 {
            return self.clone();
        }
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            let sy = (i64::from(y) * i64::from(self.height) / i64::from(height)) as i32;
            for x in 0..width {
                let sx = (i64::from(x) * i64::from(self.width) / i64::from(width)) as i32;
                pixels.push(self.pixels[(sy * self.width + sx) as usize]);
            }
        }
        let hotspot = (self.hotspot.0 * width / self.width, self.hotspot.1 * height / self.height);
        Image { width, height, pixels, hotspot }
    }

    /// Part of it: `count` images `width` wide, side by side, as an image list's strip is cut.
    pub fn cut(&self, at: i32, width: i32) -> Image {
        let width = width.min(self.width - at).max(0);
        let mut pixels = Vec::with_capacity((width * self.height) as usize);
        for y in 0..self.height {
            let row = (y * self.width + at) as usize;
            pixels.extend_from_slice(&self.pixels[row..row + width as usize]);
        }
        Image { width, height: self.height, pixels, hotspot: (0, 0) }
    }
}

/// Draws an image onto a surface at `(x, y)`, blended by its alpha, within `clip`.
pub fn blend(s: &mut Surface, image: &Image, x: i32, y: i32, clip: &Rect) {
    let area = Rect::new(x, y, x + image.width, y + image.height).intersect(clip).intersect(&s.bounds());
    if area.left >= area.right {
        return;
    }
    for py in area.top..area.bottom {
        for px in area.left..area.right {
            let p = image.pixels[((py - y) * image.width + (px - x)) as usize];
            let alpha = p >> 24;
            let at = (py * s.width + px) as usize;
            s.pixels[at] = match alpha {
                0 => continue,
                255 => p & 0xff_ffff,
                a => {
                    let under = s.pixels[at];
                    let mix = |shift: u32| (((p >> shift) & 0xff) * a + ((under >> shift) & 0xff) * (255 - a)) / 255;
                    mix(16) << 16 | mix(8) << 8 | mix(0)
                }
            };
        }
    }
}

/// Copies a surface's part onto another at `(x, y)`, within `clip`, combined as a raster operation says
/// (`SRCCOPY`, `SRCPAINT`, `SRCAND`, `SRCINVERT`, `NOTSRCCOPY`; anything else copies).
pub fn copy(dst: &mut Surface, x: i32, y: i32, src: &Surface, from: &Rect, rop: u32, clip: &Rect) {
    const SRCPAINT: u32 = 0xee_0086;
    const SRCAND: u32 = 0x88_00c6;
    const SRCINVERT: u32 = 0x66_0046;
    const NOTSRCCOPY: u32 = 0x33_0008;
    let area = Rect::new(x, y, x + from.width(), y + from.height()).intersect(clip).intersect(&dst.bounds());
    for py in area.top..area.bottom {
        for px in area.left..area.right {
            let (sx, sy) = (from.left + px - x, from.top + py - y);
            let Some(p) = src.get(sx, sy) else { continue };
            let at = (py * dst.width + px) as usize;
            let d = dst.pixels[at];
            dst.pixels[at] = match rop {
                SRCPAINT => d | p,
                SRCAND => d & p,
                SRCINVERT => d ^ p,
                NOTSRCCOPY => !p & 0xff_ffff,
                _ => p,
            };
        }
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// A DIB: its header at the start of `b`, its bits at `bits` (None: right after its palette, as in a resource or an
/// icon). An icon's (`icon`) is twice its height, its AND mask after its colours.
pub fn decode_dib(b: &[u8], bits: Option<usize>, icon: bool) -> Option<Image> {
    let header = u32_at(b, 0)? as usize;
    let (width, height, bpp, compression, used) = if header == 12 {
        (i32::from(u16_at(b, 4)?), i32::from(u16_at(b, 6)? as i16), u16_at(b, 10)?, 0, 0)
    } else if header >= 40 {
        (u32_at(b, 4)? as i32, u32_at(b, 8)? as i32, u16_at(b, 14)?, u32_at(b, 16)?, u32_at(b, 32)? as usize)
    } else {
        return None;
    };
    if width <= 0 || height == 0 || width > 16384 || height.abs() > 16384 {
        return None;
    }
    let top_down = height < 0;
    let rows = if icon { height.abs() / 2 } else { height.abs() };
    // BI_RGB and BI_BITFIELDS; RLE, JPEG and PNG aren't decoded.
    if compression != 0 && compression != 3 {
        return None;
    }
    let entry = if header == 12 { 3 } else { 4 };
    let colors = if bpp <= 8 { if used != 0 { used } else { 1 << bpp } } else { 0 };
    // Bit fields: after a 40-byte header, three masks; in a longer header they're its own.
    let masks_at = if compression == 3 { Some(40) } else { None };
    let palette_at = header + if compression == 3 && header == 40 { 12 } else { 0 };
    let palette: Vec<u32> = (0..colors).map(|i| {
        let o = palette_at + i * entry;
        b.get(o..o + 3).map_or(0, |c| u32::from(c[2]) << 16 | u32::from(c[1]) << 8 | u32::from(c[0]))
    }).collect();
    let start = bits.unwrap_or(palette_at + colors * entry);
    let stride = ((width as usize * usize::from(bpp)).div_ceil(32)) * 4;
    let masks = match masks_at {
        Some(at) => Some((u32_at(b, at)?, u32_at(b, at + 4)?, u32_at(b, at + 8)?, if header >= 56 { u32_at(b, at + 12)? } else { 0 })),
        None if bpp == 16 => Some((0x7c00, 0x3e0, 0x1f, 0)),
        None => None,
    };
    // A mask's channel, scaled to 8 bits.
    let channel = |v: u32, mask: u32| -> u32 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        ((v & mask) >> shift) * 255 / max.max(1)
    };
    let mut pixels = vec![0u32; (width * rows) as usize];
    let mut any_alpha = false;
    for row in 0..rows {
        let line = start + row as usize * stride;
        let src = b.get(line..line + stride)?;
        let y = if top_down { row } else { rows - 1 - row };
        for x in 0..width as usize {
            let rgb = match bpp {
                1 => palette.get(usize::from(src[x / 8] >> (7 - x % 8) & 1)).copied()? | 0xff00_0000,
                4 => palette.get(usize::from(src[x / 2] >> if x % 2 == 0 { 4 } else { 0 } & 0xf)).copied()? | 0xff00_0000,
                8 => palette.get(usize::from(src[x])).copied()? | 0xff00_0000,
                16 => {
                    let v = u32::from(u16::from_le_bytes([src[2 * x], src[2 * x + 1]]));
                    let (r, g, bl, _) = masks?;
                    0xff00_0000 | channel(v, r) << 16 | channel(v, g) << 8 | channel(v, bl)
                }
                24 => 0xff00_0000 | u32::from(src[3 * x + 2]) << 16 | u32::from(src[3 * x + 1]) << 8 | u32::from(src[3 * x]),
                32 => {
                    let v = u32::from_le_bytes([src[4 * x], src[4 * x + 1], src[4 * x + 2], src[4 * x + 3]]);
                    match masks {
                        Some((r, g, bl, a)) => {
                            let alpha = if a != 0 { channel(v, a) } else { 255 };
                            alpha << 24 | channel(v, r) << 16 | channel(v, g) << 8 | channel(v, bl)
                        }
                        None => {
                            any_alpha |= v >> 24 != 0;
                            v
                        }
                    }
                }
                _ => return None,
            };
            pixels[(y * width) as usize + x] = rgb;
        }
    }
    // A 32-bit picture with no alpha at all is opaque.
    if bpp == 32 && masks.is_none() && !any_alpha {
        for p in &mut pixels {
            *p |= 0xff00_0000;
        }
    }
    if icon {
        // The AND mask, one bit a pixel, after the colours: set is transparent. A 32-bit icon with alpha goes by that.
        let mask_at = start + rows as usize * stride;
        let mask_stride = (width as usize).div_ceil(32) * 4;
        let has_alpha = bpp == 32 && any_alpha;
        for row in 0..rows {
            let Some(line) = b.get(mask_at + row as usize * mask_stride..mask_at + (row as usize + 1) * mask_stride) else { break };
            let y = if top_down { row } else { rows - 1 - row };
            for x in 0..width as usize {
                if !has_alpha && line[x / 8] >> (7 - x % 8) & 1 != 0 {
                    pixels[(y * width) as usize + x] = 0;
                }
            }
        }
    }
    Some(Image { width, height: rows, pixels, hotspot: (0, 0) })
}

/// A `.bmp` file: BITMAPFILEHEADER, then a DIB whose bits are where the file header says.
pub fn decode_bmp_file(b: &[u8]) -> Option<Image> {
    if b.get(0..2)? != b"BM" {
        return None;
    }
    let bits = u32_at(b, 10)? as usize;
    decode_dib(b.get(14..)?, Some(bits.checked_sub(14)?), false)
}

/// One image of an icon or cursor: a DIB (a cursor's after its hot spot), or None for a PNG one.
pub fn decode_icon_image(b: &[u8], cursor: bool) -> Option<Image> {
    let (hotspot, b) = if cursor { ((i32::from(u16_at(b, 0)?), i32::from(u16_at(b, 2)?)), b.get(4..)?) } else { ((0, 0), b) };
    if b.starts_with(b"\x89PNG") {
        return None;
    }
    let mut image = decode_dib(b, None, true)?;
    image.hotspot = hotspot;
    Some(image)
}

/// An icon or cursor group's (`GRPICONDIR`'s, or an `.ico` or `.cur` file's `ICONDIR`'s) entries: width, height, bits a
/// pixel, the image's ID (or, in a file, its offset) and its size. A cursor group's sizes are words, its height
/// doubled.
pub fn group_entries(b: &[u8], file: bool, cursor_group: bool) -> Vec<(i32, i32, u16, u32, u32)> {
    let Some(count) = u16_at(b, 4) else { return Vec::new() };
    let size = if file { 16 } else { 14 };
    (0..usize::from(count))
        .filter_map(|i| {
            let e = b.get(6 + i * size..6 + (i + 1) * size)?;
            // 0 is 256.
            let dim = |v: u8| if v == 0 { 256 } else { i32::from(v) };
            let (w, h) = if cursor_group && !file {
                (i32::from(u16::from_le_bytes([e[0], e[1]])), i32::from(u16::from_le_bytes([e[2], e[3]])) / 2)
            } else {
                (dim(e[0]), dim(e[1]))
            };
            let bpp = u16::from_le_bytes([e[6], e[7]]);
            let length = u32::from_le_bytes([e[8], e[9], e[10], e[11]]);
            let id = if file { u32::from_le_bytes([e[12], e[13], e[14], e[15]]) } else { u32::from(u16::from_le_bytes([e[12], e[13]])) };
            Some((w, h, bpp, id, length))
        })
        .collect()
}

/// The entries in the order to try them for a size: the same size first, most colours first; then the nearest
/// larger, then smaller.
pub fn choose(entries: &[(i32, i32, u16, u32, u32)], want: (i32, i32)) -> Vec<usize> {
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| {
        let (w, h, bpp, ..) = entries[i];
        let distance = (w - want.0).abs() + (h - want.1).abs();
        let smaller = w < want.0 || h < want.1;
        (distance != 0, smaller, distance, std::cmp::Reverse(bpp))
    });
    order
}

/// An image as a cursor the desktop shows: its pixels premultiplied by their alpha.
pub fn cursor_picture(image: &Image) -> display::cursor::Cursor {
    let mut s = Surface::new(image.width, image.height, 0);
    for (d, &p) in s.pixels.iter_mut().zip(&image.pixels) {
        let a = p >> 24;
        let m = |shift: u32| ((p >> shift) & 0xff) * a / 255;
        *d = a << 24 | m(16) << 16 | m(8) << 8 | m(0);
    }
    display::cursor::Cursor::Image { pixels: s, hotspot: image.hotspot }
}

/// `LoadImage`'s kinds of picture.
pub const IMAGE_BITMAP: u32 = 0;
pub const IMAGE_ICON: u32 = 1;
pub const IMAGE_CURSOR: u32 = 2;
const LR_LOADFROMFILE: u32 = 0x10;
const LR_DEFAULTSIZE: u32 = 0x40;
/// SM_CXICON and SM_CXCURSOR: an icon's and a cursor's size, unless asked for another.
const ICON_SIZE: i32 = 32;
const RT_CURSOR: u16 = 1;
const RT_BITMAP: u16 = 2;
const RT_ICON: u16 = 3;
const RT_GROUP_CURSOR: u16 = 12;
const RT_GROUP_ICON: u16 = 14;

/// What `LoadImage` found.
pub enum Loaded {
    Bitmap(Surface),
    Icon(Image),
    /// It's there, but in a form that isn't decoded (a PNG icon, a compressed bitmap): a blank picture of its size.
    Undecoded(i32, i32),
}

impl crate::process::Task {
    /// A resource's bytes in a module, by type and name (as the program passed the name).
    fn resource_bytes(&mut self, cx: &mut kernels_core::thread::Cx<'_>, module: u64, kind: u16, name: &crate::resources::Id) -> Option<Vec<u8>> {
        let (_, data, size) = self.find_resource(cx, module, &crate::resources::Id::Num(kind), name, None).ok()?;
        self.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), data, size as usize)).ok()
    }

    /// An icon or cursor of a group, from the module's resources or a file's bytes: the image nearest the size asked
    /// for (or, for 0, the default size, or the group's first), at that size if one was asked for.
    #[allow(clippy::too_many_arguments)]
    fn icon_from_group(&mut self, cx: &mut kernels_core::thread::Cx<'_>, module: u64, group: &[u8], file: Option<&[u8]>, cursor: bool, width: i32, height: i32, flags: u32) -> Loaded {
        let entries = group_entries(group, file.is_some(), cursor);
        let asked = width > 0 || height > 0 || flags & LR_DEFAULTSIZE != 0;
        let want = if width > 0 || height > 0 {
            (if width > 0 { width } else { height }, if height > 0 { height } else { width })
        } else if flags & LR_DEFAULTSIZE != 0 || entries.is_empty() {
            (ICON_SIZE, ICON_SIZE)
        } else {
            (entries[0].0, entries[0].1)
        };
        for i in choose(&entries, want) {
            let (_, _, _, id, length) = entries[i];
            let bytes = match file {
                Some(f) => f.get(id as usize..id as usize + length as usize).map(<[u8]>::to_vec),
                None => self.resource_bytes(cx, module, if cursor { RT_CURSOR } else { RT_ICON }, &crate::resources::Id::Num(id as u16)),
            };
            // A cursor file's hot spot is in its entry, not before its image.
            let Some(image) = bytes.and_then(|b| decode_icon_image(&b, cursor && file.is_none())) else { continue };
            return Loaded::Icon(if asked { image.scaled(want.0, want.1) } else { image });
        }
        Loaded::Undecoded(want.0, want.1)
    }

    /// `LoadImage`'s work: a bitmap, icon or cursor from a module's resources (`name` an ID or a name) or, with
    /// LR_LOADFROMFILE, a file. None if it isn't there. The system's own (a NULL module) are the caller's.
    #[allow(clippy::too_many_arguments)]
    pub fn load_picture(&mut self, cx: &mut kernels_core::thread::Cx<'_>, module: u64, name: u64, kind: u32, width: i32, height: i32, flags: u32, wide: bool) -> Option<Loaded> {
        let scale_bitmap = |image: Image| -> Surface {
            let (w, h) = (if width > 0 { width } else { image.width }, if height > 0 { height } else { image.height });
            image.scaled(w, h).to_surface()
        };
        if flags & LR_LOADFROMFILE != 0 {
            let path = self.text_arg(cx, name, wide);
            let bytes = self.read_whole(&path).ok()?;
            return Some(match kind {
                IMAGE_BITMAP => match decode_bmp_file(&bytes) {
                    Some(image) => Loaded::Bitmap(scale_bitmap(image)),
                    None => Loaded::Undecoded(width.max(1), height.max(1)),
                },
                _ => self.icon_from_group(cx, 0, &bytes, Some(&bytes), kind == IMAGE_CURSOR, width, height, flags),
            });
        }
        let id = self.resource_id(cx, name, wide)?;
        self.load_resource_picture(cx, module, &id, kind, width, height, flags)
    }

    /// A bitmap, icon or cursor from a module's resources, by its resource name.
    #[allow(clippy::too_many_arguments)]
    pub fn load_resource_picture(&mut self, cx: &mut kernels_core::thread::Cx<'_>, module: u64, id: &crate::resources::Id, kind: u32, width: i32, height: i32, flags: u32) -> Option<Loaded> {
        let scale_bitmap = |image: Image| -> Surface {
            let (w, h) = (if width > 0 { width } else { image.width }, if height > 0 { height } else { image.height });
            image.scaled(w, h).to_surface()
        };
        let id = id.clone();
        Some(match kind {
            IMAGE_BITMAP => {
                let bytes = self.resource_bytes(cx, module, RT_BITMAP, &id)?;
                match decode_dib(&bytes, None, false) {
                    Some(image) => Loaded::Bitmap(scale_bitmap(image)),
                    None => Loaded::Undecoded(width.max(1), height.max(1)),
                }
            }
            _ => {
                let cursor = kind == IMAGE_CURSOR;
                let group = self.resource_bytes(cx, module, if cursor { RT_GROUP_CURSOR } else { RT_GROUP_ICON }, &id)?;
                self.icon_from_group(cx, module, &group, None, cursor, width, height, flags)
            }
        })
    }

    /// A picture `LoadImage` found, as a GDI object: a bitmap, or an icon (a blank one, its size, if it wasn't
    /// decoded).
    pub fn picture_handle(&self, loaded: Loaded, kind: u32) -> u64 {
        let mut w = self.win32k();
        match loaded {
            Loaded::Bitmap(s) => w.gdi.create_bitmap(s),
            Loaded::Icon(i) => w.gdi.create_icon(i),
            Loaded::Undecoded(width, height) if kind == IMAGE_BITMAP => w.gdi.create_bitmap(Surface::new(width, height, 0xc0c0c0)),
            Loaded::Undecoded(width, height) => w.gdi.create_icon(Image { width, height, pixels: vec![0; (width * height) as usize], hotspot: (0, 0) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 by 2 24-bit DIB, bottom-up: its top row red, green, blue; its bottom row white, black, grey.
    fn dib24() -> Vec<u8> {
        let mut b = Vec::new();
        for v in [40u32, 3, 2] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1u16.to_le_bytes());
        b.extend(24u16.to_le_bytes());
        b.extend([0u8; 24]);
        // Bottom row first, each padded to 12 bytes.
        b.extend([0xff, 0xff, 0xff, 0, 0, 0, 0x80, 0x80, 0x80, 0, 0, 0]);
        b.extend([0, 0, 0xff, 0, 0xff, 0, 0xff, 0, 0, 0, 0, 0]);
        b
    }

    #[test]
    fn a_24_bit_dib() {
        let i = decode_dib(&dib24(), None, false).expect("decodes");
        assert_eq!((i.width, i.height), (3, 2));
        assert_eq!(i.pixels, vec![0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0xffff_ffff, 0xff00_0000, 0xff80_8080]);
    }

    #[test]
    fn a_bmp_file() {
        let dib = dib24();
        let mut file = b"BM".to_vec();
        file.extend((14 + dib.len() as u32).to_le_bytes());
        file.extend([0u8; 4]);
        file.extend(54u32.to_le_bytes());
        file.extend(&dib);
        assert_eq!(decode_bmp_file(&file).map(|i| i.pixels[0]), Some(0xffff_0000));
    }

    #[test]
    fn a_4_bit_icon_with_its_mask() {
        // 2 by 2, palette black and white; the top left pixel masked out.
        let mut b = Vec::new();
        for v in [40u32, 2, 4] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1u16.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend([0u8; 24]);
        b.extend([0, 0, 0, 0, 0xff, 0xff, 0xff, 0]);
        b.extend([0u8; 14 * 4]);
        // Colours, bottom row first: white white; black white.
        b.extend([0x11, 0, 0, 0]);
        b.extend([0x01, 0, 0, 0]);
        // Mask, bottom row first: none; the left one.
        b.extend([0, 0, 0, 0]);
        b.extend([0x80, 0, 0, 0]);
        let i = decode_icon_image(&b, false).expect("decodes");
        assert_eq!(i.pixels, vec![0, 0xffff_ffff, 0xffff_ffff, 0xffff_ffff]);
    }

    #[test]
    fn choosing_a_size() {
        let entries = [(16, 16, 32, 1, 0), (32, 32, 4, 2, 0), (32, 32, 32, 3, 0), (48, 48, 32, 4, 0)];
        assert_eq!(choose(&entries, (32, 32))[0], 2);
        // Larger before smaller, then most colours.
        assert_eq!(choose(&entries, (24, 24))[0], 2);
        assert_eq!(choose(&entries, (64, 64))[0], 3);
    }

    #[test]
    fn scaling_and_cutting() {
        let i = Image { width: 2, height: 1, pixels: vec![1, 2], hotspot: (0, 0) };
        assert_eq!(i.scaled(4, 2).pixels, vec![1, 1, 2, 2, 1, 1, 2, 2]);
        assert_eq!(i.cut(1, 1).pixels, vec![2]);
    }
}
