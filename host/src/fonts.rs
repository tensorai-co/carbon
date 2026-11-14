//! The host's installed fonts, for kernels whose programs ask the OS for fonts (atlas's DirectWrite). The host lists
//! its faces, and for each gives its OpenType tables as they are and its glyphs' outlines, which is all it does:
//! metrics, character maps and rasterizing are read from those by the kernels' shared code, the same on every host.

/// One face of a font the host has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFace {
    /// How to open it again: its PostScript name.
    pub id: String,
    pub family: String,
    /// Its style within the family, as "Bold Italic".
    pub style: String,
    pub full_name: String,
    /// 100 (thin) to 900 (black), 400 regular, as OpenType's `usWeightClass`.
    pub weight: u16,
    /// 1 (ultra-condensed) to 9 (ultra-expanded), 5 normal, as `usWidthClass`.
    pub stretch: u16,
    pub italic: bool,
    pub monospace: bool,
}

/// A step of a glyph's outline, in font units with y up, as its font has it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    Move(f32, f32),
    Line(f32, f32),
    /// A quadratic Bézier: its control point, then where it ends.
    Quad(f32, f32, f32, f32),
    /// A cubic Bézier: its two control points, then where it ends.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// One face, open.
pub trait HostFont: Send + Sync {
    /// The tags of the tables it has.
    fn tables(&self) -> Vec<[u8; 4]>;
    /// A table's bytes, as the font has them.
    fn table(&self, tag: [u8; 4]) -> Option<Vec<u8>>;
    /// A glyph's outline, in font units (its `head` table's units per em), y up: empty for a glyph that draws nothing,
    /// None for a glyph it hasn't.
    fn outline(&self, glyph: u16) -> Option<Vec<PathOp>>;
}

/// The weight OpenType numbers a face's weight with, for a weight on the scale Core Text and others use (-1 to 1, 0
/// regular), by its nearest named weight.
pub fn weight_class(trait_weight: f64) -> u16 {
    const NAMED: [(f64, u16); 9] = [(-0.8, 100), (-0.6, 200), (-0.4, 300), (0.0, 400), (0.23, 500), (0.3, 600), (0.4, 700), (0.56, 800), (0.62, 900)];
    NAMED.iter().min_by(|a, b| (a.0 - trait_weight).abs().total_cmp(&(b.0 - trait_weight).abs())).map_or(400, |n| n.1)
}

/// The width class for a width on that scale (-1 condensed to 1 expanded, 0 normal).
pub fn width_class(trait_width: f64) -> u16 {
    (5.0 + trait_width * 4.0).round().clamp(1.0, 9.0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_and_widths_by_their_nearest() {
        assert_eq!(weight_class(0.0), 400);
        assert_eq!(weight_class(0.4), 700);
        assert_eq!(weight_class(0.35), 600);
        assert_eq!(weight_class(-1.0), 100);
        assert_eq!(width_class(0.0), 5);
        assert_eq!(width_class(-0.2), 4);
        assert_eq!(width_class(2.0), 9);
    }
}
