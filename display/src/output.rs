//! The output the desktop is shown on: its size in logical pixels (what windows are placed in), how many of the
//! screen's own pixels each is (its scale, in 120ths, as Wayland's fractional scale has it: 120 is 1×, 180 is 1.5×,
//! 240 is 2×), its refresh and its physical size. It changes while the machine runs: the host's monitor, a viewer's
//! window resized (`headless=auto`), `carbon display resize`.

/// A scale of 1×, in 120ths.
pub const SCALE_1X: u32 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    /// Its size, in logical pixels.
    pub width: i32,
    pub height: i32,
    /// Screen pixels per logical pixel, in 120ths.
    pub scale: u32,
    /// Refreshes a second, in thousandths.
    pub refresh_mhz: u32,
    /// Its physical size, in millimetres (0 for none: a screen in memory).
    pub physical_mm: (i32, i32),
}

impl Output {
    /// A screen in memory, at 60 Hz.
    pub fn headless(width: i32, height: i32, scale: u32) -> Output {
        Output { name: "headless".into(), width: width.max(1), height: height.max(1), scale: scale.max(1), refresh_mhz: 60_000, physical_mm: (0, 0) }
    }

    /// Dots per inch, as Windows and X11 count them: 96 at 1×.
    pub fn dpi(&self) -> u32 {
        (96 * self.scale).div_ceil(SCALE_1X)
    }

    /// The smallest whole scale at least as big: what a client that only knows whole scales draws at (Wayland's
    /// `wl_output.scale` and `preferred_buffer_scale`).
    pub fn whole_scale(&self) -> u32 {
        self.scale.div_ceil(SCALE_1X).max(1)
    }

    /// Its size in the screen's own pixels.
    pub fn pixels(&self) -> (i32, i32) {
        ((self.width as i64 * self.scale as i64 / SCALE_1X as i64) as i32, (self.height as i64 * self.scale as i64 / SCALE_1X as i64) as i32)
    }

    /// A coordinate in logical pixels, in the screen's own (rounded down, so neighbouring rectangles still meet).
    pub fn to_pixels(&self, v: i32) -> i32 {
        (v as i64 * self.scale as i64 / SCALE_1X as i64) as i32
    }

    /// A rectangle in logical pixels, in the screen's own.
    pub fn rect_to_pixels(&self, r: crate::Rect) -> crate::Rect {
        crate::Rect::new(self.to_pixels(r.left), self.to_pixels(r.top), self.to_pixels(r.right), self.to_pixels(r.bottom))
    }

    /// A scale as `1`, `1.5`, `2` (as `--display` takes it), in 120ths.
    pub fn parse_scale(text: &str) -> Option<u32> {
        let value: f64 = text.parse().ok()?;
        (value > 0.0 && value <= 8.0).then(|| (value * SCALE_1X as f64).round() as u32)
    }
}

/// `WxH`, or `WxH@SCALE`, as `--display headless=` and `carbon display resize` take it: logical pixels, and a scale
/// in 120ths (1× if none's given).
pub fn parse_size(text: &str) -> Option<(i32, i32, u32)> {
    let (size, scale) = match text.split_once('@') {
        Some((size, scale)) => (size, Output::parse_scale(scale)?),
        None => (text, SCALE_1X),
    };
    let (w, h) = size.split_once('x')?;
    let (w, h): (i32, i32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0 && w <= 16384 && h <= 16384).then_some((w, h, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales() {
        let o = Output::headless(1280, 800, 180);
        assert_eq!((o.dpi(), o.whole_scale(), o.pixels()), (144, 2, (1920, 1200)));
        assert_eq!(Output::parse_scale("1.5"), Some(180));
        assert_eq!(Output::parse_scale("2"), Some(240));
        assert_eq!(Output::parse_scale("0"), None);
        assert_eq!(parse_size("1280x800@1.5"), Some((1280, 800, 180)));
        assert_eq!(parse_size("800x600"), Some((800, 600, 120)));
        assert_eq!(parse_size("800x"), None);
    }
}
