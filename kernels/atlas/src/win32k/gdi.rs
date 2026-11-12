//! GDI: device contexts, brushes, pens and fonts, and drawing with them, host-side, into a window's top-level
//! surface. There's one font (`display::font`), whatever a program asks for.

use std::collections::HashMap;

use display::{Rect, Rgb, Surface, font};

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
}

const STOCK: u64 = 0x0080_0010;
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
}

#[derive(Default)]
pub struct Gdi {
    objects: HashMap<u64, Object>,
    pub dcs: HashMap<u64, Dc>,
    next: u64,
}

impl Gdi {
    pub fn stock_object(index: u64) -> u64 {
        if stock(index).is_some() { STOCK + index } else { 0 }
    }

    pub fn sys_brush(index: u64) -> u64 {
        SYS_BRUSHES + index
    }

    pub fn object(&self, handle: u64) -> Option<Object> {
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

    pub fn create(&mut self, object: Object) -> u64 {
        self.next = self.next.max(0x0030_0000) + 4;
        self.objects.insert(self.next, object);
        self.next
    }

    /// Deletes an object made by `create`; stock objects stay.
    pub fn delete(&mut self, handle: u64) -> bool {
        self.object(handle).is_some() && (self.objects.remove(&handle).is_some() || handle >= STOCK)
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
        };
        Some(std::mem::replace(slot, handle))
    }
}

impl Win32k {
    /// Draws with a DC: `f` gets its top-level window's pixels, on the desktop, where its client area's origin is in it, the clip
    /// there, and the DC. False for a DC that isn't there; true, drawing nothing, for the screen's.
    pub fn draw(&mut self, hdc: u64, f: impl FnOnce(&mut Surface, (i32, i32), &Rect, &Dc)) -> bool {
        let Some(dc) = self.gdi.dcs.get(&hdc).cloned() else { return false };
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
