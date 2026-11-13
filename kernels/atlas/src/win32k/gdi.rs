//! GDI: device contexts, brushes, pens, fonts, bitmaps and icons, and drawing with them, host-side, into a window's
//! top-level surface or (a memory DC's) the bitmap selected into it. There's one font (`display::font`), whatever a
//! program asks for.

use std::collections::HashMap;

use display::{Rect, Rgb, Surface, font};

use super::image::{self, Image};

use super::Win32k;

/// A COLORREF (`0x00BBGGRR`) as `0x00RRGGBB`, and back: the same swap.
pub fn swap(color: u32) -> u32 {
    (color & 0xff) << 16 | color & 0xff00 | (color >> 16) & 0xff
}

/// A system colour, as `0x00RRGGBB`.
pub fn sys_color(index: u32) -> Rgb {
    match index {
        1 | 7 => 0x000000,   // COLOR_DESKTOP, COLOR_WINDOWFRAME
        5 => 0xffffff,       // COLOR_WINDOW
        6 | 8 | 18 => 0x000000, // COLOR_WINDOWFRAME, COLOR_WINDOWTEXT, COLOR_BTNTEXT
        13 => 0x0078d7,      // COLOR_HIGHLIGHT
        14 => 0xffffff,      // COLOR_HIGHLIGHTTEXT
        15 => 0xf0f0f0,      // COLOR_BTNFACE
        16 => 0xa0a0a0,      // COLOR_BTNSHADOW
        17 => 0x6d6d6d,      // COLOR_GRAYTEXT
        20 => 0xffffff,      // COLOR_BTNHIGHLIGHT
        _ => 0xc0c0c0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    /// A brush's colour, or none (NULL_BRUSH).
    Brush(Option<Rgb>),
    /// A pen's colour, or none (NULL_PEN).
    Pen(Option<Rgb>),
    Font,
    /// Its pixels are `Gdi::bitmaps`'.
    Bitmap,
    /// An icon or a cursor: its pixels, with alpha, are `Gdi::icons`'.
    Icon,
    /// A region, as its bounding rectangle (regions of other shapes aren't kept yet).
    Region(Rect),
}

const STOCK: u64 = 0x0080_0010;
/// The 1 by 1 bitmap a memory DC has before one's selected into it, which `SelectObject` gives back.
pub const DEFAULT_BITMAP: u64 = STOCK + 30;
const SYS_BRUSHES: u64 = 0x0080_1000;
pub const OPAQUE: u32 = 2;

/// GetStockObject's objects, by index.
fn stock(index: u64) -> Option<Object> {
    Some(match index {
        0 | 18 => Object::Brush(Some(0xffffff)),
        1 => Object::Brush(Some(0xc0c0c0)),
        2 => Object::Brush(Some(0x808080)),
        3 => Object::Brush(Some(0x404040)),
        4 => Object::Brush(Some(0x000000)),
        5 => Object::Brush(None),
        6 => Object::Pen(Some(0xffffff)),
        7 | 19 => Object::Pen(Some(0x000000)),
        8 => Object::Pen(None),
        10..=17 => Object::Font,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Dc {
    /// Its window, or 0 for the screen, where drawing goes nowhere yet.
    pub hwnd: u64,
    /// Where it may draw, in its window's client coordinates.
    pub clip: Rect,
    pub text: Rgb,
    pub background: Rgb,
    pub mode: u32,
    pub brush: u64,
    pub pen: u64,
    pub font: u64,
    pub position: (i32, i32),
    /// A memory DC (`CreateCompatibleDC`): it draws into its selected bitmap.
    pub memory: bool,
    pub bitmap: u64,
}

#[derive(Default)]
pub struct Gdi {
    objects: HashMap<u64, Object>,
    pub dcs: HashMap<u64, Dc>,
    next: u64,
    /// Bitmaps' pixels, and icons' and cursors'.
    pub bitmaps: HashMap<u64, Surface>,
    pub icons: HashMap<u64, super::image::Image>,
}

impl Gdi {
    pub fn stock_object(index: u64) -> u64 {
        if stock(index).is_some() { STOCK + index } else { 0 }
    }

    pub fn sys_brush(index: u64) -> u64 {
        SYS_BRUSHES + index
    }

    pub fn object(&self, handle: u64) -> Option<Object> {
        if handle == DEFAULT_BITMAP {
            return Some(Object::Bitmap);
        }
        if (STOCK..STOCK + 32).contains(&handle) {
            return stock(handle - STOCK);
        }
        if (SYS_BRUSHES..SYS_BRUSHES + 64).contains(&handle) {
            return Some(Object::Brush(Some(sys_color((handle - SYS_BRUSHES) as u32))));
        }
        self.objects.get(&handle).copied()
    }

    pub fn brush_color(&self, handle: u64) -> Option<Rgb> {
        match self.object(handle)? {
            Object::Brush(color) => color,
            _ => None,
        }
    }

    /// A region's rectangle.
    pub fn region(&self, handle: u64) -> Option<Rect> {
        match self.objects.get(&handle)? {
            Object::Region(r) => Some(*r),
            _ => None,
        }
    }

    /// Changes a region's rectangle: whether it's one.
    pub fn set_region(&mut self, handle: u64, rect: Rect) -> bool {
        match self.objects.get_mut(&handle) {
            Some(Object::Region(r)) => {
                *r = rect;
                true
            }
            _ => false,
        }
    }

    pub fn create(&mut self, object: Object) -> u64 {
        self.next = self.next.max(0x0030_0000) + 4;
        self.objects.insert(self.next, object);
        self.next
    }

    /// Deletes an object made by `create`, and a bitmap's or icon's pixels; stock objects stay.
    pub fn delete(&mut self, handle: u64) -> bool {
        self.bitmaps.remove(&handle);
        self.icons.remove(&handle);
        self.object(handle).is_some() && (self.objects.remove(&handle).is_some() || handle >= STOCK)
    }

    /// A bitmap of these pixels.
    pub fn create_bitmap(&mut self, pixels: Surface) -> u64 {
        let handle = self.create(Object::Bitmap);
        self.bitmaps.insert(handle, pixels);
        handle
    }

    /// An icon (or cursor) of this picture.
    pub fn create_icon(&mut self, image: super::image::Image) -> u64 {
        let handle = self.create(Object::Icon);
        self.icons.insert(handle, image);
        handle
    }

    /// `CreateCompatibleDC`: a memory DC, with no bitmap of its own yet.
    pub fn new_memory_dc(&mut self) -> u64 {
        let hdc = self.new_dc(0, Rect::new(0, 0, i32::MAX / 2, i32::MAX / 2));
        if let Some(dc) = self.dcs.get_mut(&hdc) {
            dc.memory = true;
            dc.bitmap = DEFAULT_BITMAP;
        }
        hdc
    }

    pub fn new_dc(&mut self, hwnd: u64, clip: Rect) -> u64 {
        self.next = self.next.max(0x0030_0000) + 4;
        let dc = Dc {
            hwnd,
            clip,
            text: 0,
            background: 0xffffff,
            mode: OPAQUE,
            brush: STOCK,
            pen: STOCK + 7,
            font: STOCK + 13,
            position: (0, 0),
            memory: false,
            bitmap: 0,
        };
        self.dcs.insert(self.next, dc);
        self.next
    }

    /// `SelectObject`: the object of that kind it had before.
    pub fn select(&mut self, hdc: u64, handle: u64) -> Option<u64> {
        let kind = self.object(handle)?;
        let dc = self.dcs.get_mut(&hdc)?;
        let slot = match kind {
            Object::Brush(_) => &mut dc.brush,
            Object::Pen(_) => &mut dc.pen,
            Object::Font => &mut dc.font,
            // A bitmap goes into a memory DC only.
            Object::Bitmap if dc.memory => &mut dc.bitmap,
            Object::Bitmap | Object::Icon => return None,
            // A region selected is the clip (SelectClipRgn's), and what comes back is SIMPLEREGION.
            Object::Region(r) => {
                dc.clip = r;
                return Some(2);
            }
        };
        Some(std::mem::replace(slot, handle))
    }
}

impl Win32k {
    /// Draws with a DC: `f` gets its top-level window's pixels, on the desktop, where its client area's origin is in it, the clip
    /// there, and the DC. False for a DC that isn't there; true, drawing nothing, for the screen's.
    pub fn draw(&mut self, hdc: u64, f: impl FnOnce(&mut Surface, (i32, i32), &Rect, &Dc)) -> bool {
        let Some(dc) = self.gdi.dcs.get(&hdc).cloned() else { return false };
        // A memory DC's selected bitmap; the default one is 1 by 1 and draws nothing worth keeping.
        if dc.memory {
            if let Some(bitmap) = self.gdi.bitmaps.get_mut(&dc.bitmap) {
                let clip = dc.clip.intersect(&bitmap.bounds());
                f(bitmap, (0, 0), &clip, &dc);
            }
            return true;
        }
        let Some((top, dx, dy)) = self.origin(dc.hwnd) else { return dc.hwnd == 0 };
        let Some(client) = self.windows.get(&dc.hwnd).map(|w| w.client()) else { return false };
        let clip = dc.clip.intersect(&client).offset(dx, dy);
        let Some(shown) = self.windows.get(&top).map(|w| w.shown).filter(|&s| s != 0) else { return false };
        let Some(canvas) = self.canvases.get(&shown) else { return false };
        // Drawn with only the canvas's lock, then committed with what changed: the desktop's lock comes after.
        canvas.draw(|surface| f(surface, (dx, dy), &clip, &dc));
        let mut d = display::desktop::lock(&self.display);
        d.damage(shown, clip);
        d.commit(shown);
        true
    }

    pub fn fill_rect(&mut self, hdc: u64, rect: Rect, brush: u64) -> bool {
        let Some(color) = self.gdi.brush_color(brush) else { return self.gdi.object(brush).is_some() };
        self.draw(hdc, |s, (dx, dy), clip, _| s.fill(&rect.offset(dx, dy), color, clip))
    }

    /// `Rectangle`: filled with the DC's brush, outlined with its pen.
    pub fn rectangle(&mut self, hdc: u64, rect: Rect) -> bool {
        let Some(dc) = self.gdi.dcs.get(&hdc) else { return false };
        let (fill, outline) = (self.gdi.brush_color(dc.brush), match self.gdi.object(dc.pen) {
            Some(Object::Pen(color)) => color,
            _ => None,
        });
        self.draw(hdc, |s, (dx, dy), clip, _| {
            let r = rect.offset(dx, dy);
            if let Some(c) = fill {
                s.fill(&r, c, clip);
            }
            if let Some(c) = outline {
                s.frame(&r, c, clip);
            }
        })
    }

    /// `LineTo`: from the DC's position to here, not including here, as GDI draws lines; then it's the position.
    pub fn line_to(&mut self, hdc: u64, x: i32, y: i32) -> bool {
        let Some(dc) = self.gdi.dcs.get(&hdc) else { return false };
        let color = match self.gdi.object(dc.pen) {
            Some(Object::Pen(color)) => color,
            _ => None,
        };
        let from = dc.position;
        let drawn = self.draw(hdc, |s, (dx, dy), clip, _| {
            if let Some(c) = color
                && from != (x, y)
            {
                let last = s.get(x + dx, y + dy);
                s.line((from.0 + dx, from.1 + dy), (x + dx, y + dy), c, clip);
                if let Some(last) = last {
                    s.set(x + dx, y + dy, last, clip);
                }
            }
        });
        if let Some(dc) = self.gdi.dcs.get_mut(&hdc) {
            dc.position = (x, y);
        }
        drawn
    }

    pub fn set_pixel(&mut self, hdc: u64, x: i32, y: i32, color: Rgb) -> bool {
        self.draw(hdc, |s, (dx, dy), clip, _| s.set(x + dx, y + dy, color, clip))
    }

    pub fn get_pixel(&mut self, hdc: u64, x: i32, y: i32) -> Option<Rgb> {
        let mut found = None;
        self.draw(hdc, |s, (dx, dy), clip, _| {
            if clip.contains(x + dx, y + dy) {
                found = s.get(x + dx, y + dy);
            }
        });
        found
    }

    /// `TextOutW`: in the DC's text colour, its cells filled with its background colour if it's OPAQUE.
    pub fn text_out(&mut self, hdc: u64, x: i32, y: i32, text: &str) -> bool {
        self.draw(hdc, |s, (dx, dy), clip, dc| {
            let background = (dc.mode == OPAQUE).then_some(dc.background);
            s.text(x + dx, y + dy, text, dc.text, background, clip);
        })
    }

    /// `DrawTextW`: lines in a rectangle, as `format` places them. With DT_CALCRECT, only measures, into the rectangle.
    /// Gives the text's height.
    pub fn draw_text(&mut self, hdc: u64, text: &str, rect: &mut Rect, format: u32) -> i32 {
        const CENTER: u32 = 1;
        const RIGHT: u32 = 2;
        const VCENTER: u32 = 4;
        const BOTTOM: u32 = 8;
        const SINGLELINE: u32 = 0x20;
        const CALCRECT: u32 = 0x400;
        let lines: Vec<String> = if format & SINGLELINE != 0 { vec![text.replace(['\r', '\n'], " ")] } else { text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect() };
        let height = lines.len() as i32 * font::CELL_HEIGHT;
        let widest = lines.iter().map(|l| l.chars().count() as i32 * font::CELL_WIDTH).max().unwrap_or(0);
        if format & CALCRECT != 0 {
            rect.right = rect.left + widest;
            rect.bottom = rect.top + height;
            return height;
        }
        let top = if format & SINGLELINE != 0 && format & VCENTER != 0 {
            rect.top + (rect.height() - height) / 2
        } else if format & SINGLELINE != 0 && format & BOTTOM != 0 {
            rect.bottom - height
        } else {
            rect.top
        };
        for (i, line) in lines.iter().enumerate() {
            let width = line.chars().count() as i32 * font::CELL_WIDTH;
            let left = if format & CENTER != 0 {
                rect.left + (rect.width() - width) / 2
            } else if format & RIGHT != 0 {
                rect.right - width
            } else {
                rect.left
            };
            let y = top + i as i32 * font::CELL_HEIGHT;
            let bounds = *rect;
            self.draw(hdc, |s, (dx, dy), clip, dc| {
                let clip = clip.intersect(&bounds.offset(dx, dy));
                let background = (dc.mode == OPAQUE).then_some(dc.background);
                s.text(left + dx, y + dy, line, dc.text, background, &clip);
            });
        }
        height
    }
}

impl Win32k {
    /// What a DC shows in `rect` (its own coordinates): a memory DC's bitmap's pixels, or a window's.
    pub fn read_dc(&self, hdc: u64, rect: Rect) -> Option<Surface> {
        let dc = self.gdi.dcs.get(&hdc)?;
        let mut out = Surface::new(rect.width().max(0), rect.height().max(0), 0);
        let from = |s: &Surface, dx: i32, dy: i32, out: &mut Surface| {
            let all = out.bounds();
            image::copy(out, 0, 0, s, &rect.offset(dx, dy), 0xcc_0020, &all);
        };
        if dc.memory {
            from(self.gdi.bitmaps.get(&dc.bitmap)?, 0, 0, &mut out);
            return Some(out);
        }
        let (top, dx, dy) = self.origin(dc.hwnd)?;
        let shown = self.windows.get(&top).map(|w| w.shown).filter(|&s| s != 0)?;
        self.canvases.get(&shown)?.draw(|s| from(s, dx, dy, &mut out));
        Some(out)
    }

    /// `BitBlt` and `StretchBlt`: `from` in `src` (none for the raster operations that don't read it) onto `to` in
    /// `dst`, scaled to fit, as `rop` combines them.
    pub fn blit(&mut self, dst: u64, to: Rect, src: u64, from: Rect, rop: u32) -> bool {
        const BLACKNESS: u32 = 0x00_0042;
        const WHITENESS: u32 = 0xff_0062;
        const PATCOPY: u32 = 0xf0_0021;
        const DSTINVERT: u32 = 0x55_0009;
        let fill = match rop {
            BLACKNESS => Some(Some(0)),
            WHITENESS => Some(Some(0xff_ffff)),
            PATCOPY => Some(self.gdi.dcs.get(&dst).and_then(|d| self.gdi.brush_color(d.brush))),
            _ => None,
        };
        if let Some(color) = fill {
            return self.draw(dst, |s, (dx, dy), clip, _| {
                if let Some(c) = color {
                    s.fill(&to.offset(dx, dy), c, clip);
                }
            });
        }
        if rop == DSTINVERT {
            return self.draw(dst, |s, (dx, dy), clip, _| {
                let r = to.offset(dx, dy).intersect(clip).intersect(&s.bounds());
                for y in r.top..r.bottom {
                    for x in r.left..r.right {
                        let at = (y * s.width + x) as usize;
                        s.pixels[at] = !s.pixels[at] & 0xff_ffff;
                    }
                }
            });
        }
        let Some(source) = self.read_dc(src, from) else { return false };
        let source = if (to.width(), to.height()) != (from.width(), from.height()) {
            Image::from_surface(&source).scaled(to.width().abs(), to.height().abs()).to_surface()
        } else {
            source
        };
        self.draw(dst, |s, (dx, dy), clip, _| {
            let all = source.bounds();
            image::copy(s, to.left + dx, to.top + dy, &source, &all, rop, clip);
        })
    }

    /// `DrawIconEx`: an icon at a place in a DC, at a size (its own for 0), blended by its alpha.
    pub fn draw_icon(&mut self, hdc: u64, x: i32, y: i32, icon: u64, width: i32, height: i32) -> bool {
        let Some(image) = self.gdi.icons.get(&icon).cloned() else { return false };
        let image = if width > 0 && height > 0 { image.scaled(width, height) } else { image };
        self.draw(hdc, |s, (dx, dy), clip, _| image::blend(s, &image, x + dx, y + dy, clip))
    }
}
