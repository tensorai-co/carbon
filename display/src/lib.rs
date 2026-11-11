//! The display: a machine's screen, whichever kernel draws on it. The pixels windows are drawn in (`Surface`), with
//! drawing (rectangles, lines, text in its one font) and PNG; and the desktop (`desktop`), the window manager both
//! kernels share, on a screen in memory. Keys are evdev numbers (`keys`). Showing the desktop in the host's own
//! windows comes later, per host, as `display/macos` and the rest.

pub mod desktop;
pub mod font;
pub mod keys;

pub use desktop::Desktop;

/// A colour as `0x00RRGGBB`.
pub type Rgb = u32;

/// A rectangle, right and bottom not included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        Rect::new(self.left.max(other.left), self.top.max(other.top), self.right.min(other.right), self.bottom.min(other.bottom))
    }

    /// The smallest rectangle covering both (an empty one counts for nothing).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Rect::new(self.left.min(other.left), self.top.min(other.top), self.right.max(other.right), self.bottom.max(other.bottom))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<Rgb>,
}

impl Surface {
    /// A surface of one colour.
    pub fn new(width: i32, height: i32, fill: Rgb) -> Surface {
        let (w, h) = (width.max(0), height.max(0));
        Surface { width: w, height: h, pixels: vec![fill; (w * h) as usize] }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// A new size, keeping what's drawn where it still fits, and `fill` where it's new.
    pub fn resize(&mut self, width: i32, height: i32, fill: Rgb) {
        let mut next = Surface::new(width, height, fill);
        for y in 0..self.height.min(next.height) {
            for x in 0..self.width.min(next.width) {
                next.pixels[(y * next.width + x) as usize] = self.pixels[(y * self.width + x) as usize];
            }
        }
        *self = next;
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Rgb> {
        self.bounds().contains(x, y).then(|| self.pixels[(y * self.width + x) as usize])
    }

    /// Sets a pixel, if it's within `clip` and the surface.
    pub fn set(&mut self, x: i32, y: i32, color: Rgb, clip: &Rect) {
        if clip.contains(x, y) && self.bounds().contains(x, y) {
            self.pixels[(y * self.width + x) as usize] = color;
        }
    }

    pub fn fill(&mut self, rect: &Rect, color: Rgb, clip: &Rect) {
        let r = rect.intersect(clip).intersect(&self.bounds());
        for y in r.top..r.bottom {
            let row = (y * self.width) as usize;
            self.pixels[row + r.left as usize..row + r.right as usize].fill(color);
        }
    }

    /// A one-pixel line, both ends included.
    pub fn line(&mut self, from: (i32, i32), to: (i32, i32), color: Rgb, clip: &Rect) {
        let (mut x, mut y) = from;
        let (dx, dy) = ((to.0 - x).abs(), -(to.1 - y).abs());
        let (sx, sy) = (if x < to.0 { 1 } else { -1 }, if y < to.1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.set(x, y, color, clip);
            if (x, y) == to {
                return;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// A rectangle's one-pixel outline.
    pub fn frame(&mut self, rect: &Rect, color: Rgb, clip: &Rect) {
        if rect.is_empty() {
            return;
        }
        let (r, b) = (rect.right - 1, rect.bottom - 1);
        self.line((rect.left, rect.top), (r, rect.top), color, clip);
        self.line((rect.left, b), (r, b), color, clip);
        self.line((rect.left, rect.top), (rect.left, b), color, clip);
        self.line((r, rect.top), (r, b), color, clip);
    }

    /// Text in the font, its cells' top left at `(x, y)`, with each cell's background filled if `background` is given.
    pub fn text(&mut self, x: i32, y: i32, text: &str, color: Rgb, background: Option<Rgb>, clip: &Rect) {
        for (i, c) in text.chars().enumerate() {
            let cx = x + i as i32 * font::CELL_WIDTH;
            if let Some(bg) = background {
                self.fill(&Rect::new(cx, y, cx + font::CELL_WIDTH, y + font::CELL_HEIGHT), bg, clip);
            }
            for (px, py) in font::pixels(c as u32) {
                self.set(cx + px, y + py, color, clip);
            }
        }
    }

    /// Copies `from`'s pixels in, at `(x, y)`.
    pub fn blit(&mut self, from: &Surface, x: i32, y: i32, clip: &Rect) {
        let r = Rect::new(x, y, x + from.width, y + from.height).intersect(clip).intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let (n, sx) = ((r.right - r.left) as usize, (r.left - x) as usize);
        for dy in r.top..r.bottom {
            let src = ((dy - y) * from.width) as usize + sx;
            let dst = (dy * self.width + r.left) as usize;
            self.pixels[dst..dst + n].copy_from_slice(&from.pixels[src..src + n]);
        }
    }

    /// As a PNG: 8-bit RGB, deflated as stored blocks.
    pub fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.width * self.height * 3 + self.height) as usize);
        for y in 0..self.height {
            raw.push(0);
            for x in 0..self.width {
                let p = self.pixels[(y * self.width + x) as usize];
                raw.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend((self.width as u32).to_be_bytes());
        header.extend((self.height as u32).to_be_bytes());
        header.extend([8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib_stored(&raw));
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

/// A zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if data.is_empty() { vec![&[]] } else { data.chunks(65535).collect() };
    for (i, block) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend((b << 16 | a).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing() {
        let mut s = Surface::new(20, 20, 0xffffff);
        let all = s.bounds();
        s.fill(&Rect::new(2, 2, 6, 6), 0xff0000, &all);
        assert_eq!(s.get(2, 2), Some(0xff0000));
        assert_eq!(s.get(6, 6), Some(0xffffff));
        s.line((0, 0), (19, 19), 0x0000ff, &Rect::new(0, 0, 10, 10));
        assert_eq!(s.get(9, 9), Some(0x0000ff));
        assert_eq!(s.get(10, 10), Some(0xffffff));
        s.text(0, 0, "I", 0x00ff00, None, &all);
        // 'I' has its top bar's middle at column 2, on the cell's row ASCENT - 7.
        assert_eq!(s.get(2, font::ASCENT - 7), Some(0x00ff00));
        s.resize(30, 10, 0x123456);
        assert_eq!((s.get(4, 3), s.get(25, 5)), (Some(0xff0000), Some(0x123456)));
    }

    #[test]
    fn png_is_well_formed() {
        let png = Surface::new(3, 2, 0x112233).png();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert!(png.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }
}
