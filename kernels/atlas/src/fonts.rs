//! The machine's fonts: the host's, grouped into families as Windows lists them, with the families Windows programs
//! ask for by name (Segoe UI, Consolas, …) found as the host's nearest where the host hasn't them. A face's tables,
//! metrics, character map and advances are read once, the first time any process uses it, and kept for every process.
//! The list itself is made the first time a program asks for fonts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use display::raster::Step;
use display::sfnt::{self, Cmap, Metrics};
use host::{FontFace, Host, HostFont, PathOp};
use kernels_core::lock;

/// Windows' families that hosts often haven't, each with the host families that stand in for it, best first.
const STAND_INS: &[(&str, &[&str])] = &[
    ("Segoe UI", &["Segoe UI", "Helvetica Neue", "Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"]),
    ("Segoe UI Variable", &["Segoe UI", "Helvetica Neue", "Helvetica", "Arial"]),
    ("Segoe UI Symbol", &["Segoe UI Symbol", "Apple Symbols", "DejaVu Sans"]),
    ("MS Shell Dlg", &["Tahoma", "Microsoft Sans Serif", "Helvetica Neue", "Arial"]),
    ("MS Shell Dlg 2", &["Tahoma", "Helvetica Neue", "Arial"]),
    ("Microsoft Sans Serif", &["Microsoft Sans Serif", "Tahoma", "Helvetica Neue", "Arial"]),
    ("Calibri", &["Calibri", "Helvetica Neue", "Carlito", "Arial"]),
    ("Cambria", &["Cambria", "Georgia", "Caladea", "Times New Roman"]),
    ("Consolas", &["Consolas", "Menlo", "DejaVu Sans Mono", "Courier New"]),
    ("Lucida Console", &["Lucida Console", "Menlo", "Monaco", "Courier New"]),
    ("Courier New", &["Courier New", "Courier", "Liberation Mono", "Menlo"]),
    ("Arial", &["Arial", "Helvetica", "Liberation Sans", "DejaVu Sans"]),
    ("Times New Roman", &["Times New Roman", "Times", "Liberation Serif", "DejaVu Serif"]),
    ("Microsoft YaHei", &["Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC"]),
    ("Microsoft JhengHei", &["Microsoft JhengHei", "PingFang TC", "Noto Sans CJK TC"]),
    ("SimSun", &["SimSun", "Songti SC", "PingFang SC"]),
    ("Yu Gothic", &["Yu Gothic", "Hiragino Sans", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP"]),
    ("Meiryo", &["Meiryo", "Hiragino Sans", "Hiragino Kaku Gothic ProN"]),
    ("MS Gothic", &["MS Gothic", "Hiragino Sans", "Osaka"]),
    ("Malgun Gothic", &["Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans CJK KR"]),
    ("Nirmala UI", &["Nirmala UI", "Kohinoor Devanagari", "Noto Sans Devanagari"]),
    ("Leelawadee UI", &["Leelawadee UI", "Thonburi", "Noto Sans Thai"]),
    ("Ebrima", &["Ebrima", "Noto Sans"]),
    ("Gadugi", &["Gadugi", "Noto Sans"]),
];

/// A family: its name and its faces' places in the list.
#[derive(Debug, Clone)]
pub struct Family {
    pub name: String,
    pub faces: Vec<usize>,
}

/// The machine's fonts, as listed.
pub struct Fonts {
    pub faces: Vec<FontFace>,
    pub families: Vec<Family>,
    open: Mutex<HashMap<usize, Option<Arc<Face>>>>,
}

/// A face, open: what its tables say.
pub struct Face {
    pub host: Arc<dyn HostFont>,
    pub metrics: Metrics,
    pub cmap: Option<Cmap>,
    hmtx: Vec<u8>,
    vmtx: Option<(Vec<u8>, u16)>,
    tables: Mutex<HashMap<[u8; 4], Option<Arc<Vec<u8>>>>>,
    file: OnceLock<Arc<Vec<u8>>>,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();

/// The machine's fonts, listed from the host the first time.
pub fn get(host: &Arc<dyn Host>) -> &'static Fonts {
    FONTS.get_or_init(|| Fonts::new(host.fonts()))
}

impl Fonts {
    fn new(mut faces: Vec<FontFace>) -> Fonts {
        faces.sort_by_key(|f| (f.family.to_lowercase(), f.weight, f.stretch, f.italic));
        let mut families: Vec<Family> = Vec::new();
        for (i, f) in faces.iter().enumerate() {
            match families.last_mut() {
                Some(last) if last.name.eq_ignore_ascii_case(&f.family) => last.faces.push(i),
                _ => families.push(Family { name: f.family.clone(), faces: vec![i] }),
            }
        }
        Fonts { faces, families, open: Mutex::new(HashMap::new()) }
    }

    /// A family by name, any case: the host's own, else a stand-in for a Windows family the host hasn't.
    pub fn find(&self, name: &str) -> Option<usize> {
        let exact = |n: &str| self.families.iter().position(|f| f.name.eq_ignore_ascii_case(n));
        exact(name).or_else(|| {
            let (_, stand_ins) = STAND_INS.iter().find(|(w, _)| w.eq_ignore_ascii_case(name))?;
            stand_ins.iter().find_map(|n| exact(n))
        })
    }

    /// The face of a family nearest a weight, stretch and slant, as DirectWrite matches them: the stretch first, then
    /// the slant (oblique and italic alike), then the weight, as CSS orders them.
    pub fn matching(&self, family: usize, weight: u16, stretch: u16, italic: bool) -> Option<usize> {
        let faces = &self.families.get(family)?.faces;
        faces.iter().copied().min_by_key(|&i| {
            let f = &self.faces[i];
            let stretch_distance = (i32::from(f.stretch) - i32::from(stretch)).unsigned_abs();
            let slant = u32::from(f.italic != italic);
            let w = i32::from(f.weight);
            let want = i32::from(weight);
            // 400 to 500 asked: those up to 500, then lighter, then heavier. Lighter asked: lighter first. Heavier:
            // heavier first.
            let weight_distance = match want {
                400..=500 if w >= want && w <= 500 => (w - want) as u32,
                400..=500 if w < want => 1000 + (want - w) as u32,
                400..=500 => 2000 + (w - want) as u32,
                _ if want < 400 && w <= want => (want - w) as u32,
                _ if want < 400 => 1000 + (w - want) as u32,
                _ if w >= want => (w - want) as u32,
                _ => 1000 + (want - w) as u32,
            };
            (stretch_distance, slant, weight_distance)
        })
    }

    /// A face open (its tables read), or None if the host can't open it.
    pub fn open(&self, host: &Arc<dyn Host>, face: usize) -> Option<Arc<Face>> {
        if let Some(open) = lock(&self.open).get(&face) {
            return open.clone();
        }
        let opened = self.faces.get(face).and_then(|f| host.open_font(&f.id)).and_then(Face::new);
        lock(&self.open).insert(face, opened.clone());
        opened
    }
}

impl Face {
    fn new(host: Arc<dyn HostFont>) -> Option<Arc<Face>> {
        let head = host.table(*b"head")?;
        let hhea = host.table(*b"hhea");
        let metrics = Metrics::read(&head, hhea.as_deref(), host.table(*b"OS/2").as_deref(), host.table(*b"post").as_deref(), host.table(*b"maxp").as_deref())?;
        let cmap = host.table(*b"cmap").and_then(|c| Cmap::read(&c));
        let hmtx = host.table(*b"hmtx").unwrap_or_default();
        let vmtx = host.table(*b"vmtx").zip(host.table(*b"vhea")).map(|(v, h)| (v, u16::from_be_bytes([h.get(34).copied().unwrap_or(0), h.get(35).copied().unwrap_or(0)])));
        Some(Arc::new(Face { host, metrics, cmap, hmtx, vmtx, tables: Mutex::new(HashMap::new()), file: OnceLock::new() }))
    }

    pub fn glyph(&self, c: u32) -> u16 {
        self.cmap.as_ref().map_or(0, |m| m.glyph(c))
    }

    /// A glyph's horizontal advance and left side bearing, in font units.
    pub fn h_metrics(&self, glyph: u16) -> (u16, i16) {
        sfnt::advance(&self.hmtx, self.metrics.h_metrics, glyph)
    }

    /// Its vertical advance and top side bearing: `vmtx`'s, else the line's height and what's above its box.
    pub fn v_metrics(&self, glyph: u16) -> (u16, i16) {
        match &self.vmtx {
            Some((v, n)) => sfnt::advance(v, *n, glyph),
            None => {
                let top = self.bounds(glyph).map_or(0, |b| b.3.ceil() as i32);
                ((self.metrics.ascent + self.metrics.descent), (i32::from(self.metrics.ascent) - top).clamp(-0x8000, 0x7fff) as i16)
            }
        }
    }

    /// A glyph's box in font units, y up: left, bottom, right, top. None for a glyph that draws nothing.
    pub fn bounds(&self, glyph: u16) -> Option<(f32, f32, f32, f32)> {
        let outline = self.host.outline(glyph)?;
        let mut b: Option<(f32, f32, f32, f32)> = None;
        for op in &outline {
            let points: &[(f32, f32)] = &match *op {
                PathOp::Move(x, y) | PathOp::Line(x, y) => vec![(x, y)],
                PathOp::Quad(a, b, x, y) => vec![(a, b), (x, y)],
                PathOp::Cubic(a, b, c, d, x, y) => vec![(a, b), (c, d), (x, y)],
                PathOp::Close => vec![],
            };
            for &(x, y) in points {
                b = Some(b.map_or((x, y, x, y), |(l, bo, r, t)| (l.min(x), bo.min(y), r.max(x), t.max(y))));
            }
        }
        b
    }

    /// A glyph's outline in pixels, y down: scaled by `scale` pixels a font unit, through `transform` (DirectWrite's
    /// m11, m12, m21, m22, dx, dy), from the origin `at`.
    pub fn steps(&self, glyph: u16, scale: f32, transform: [f32; 6], at: (f32, f32)) -> Vec<Step> {
        let Some(outline) = self.host.outline(glyph) else { return Vec::new() };
        let [m11, m12, m21, m22, dx, dy] = transform;
        let p = |x: f32, y: f32| {
            // Font units y up, to the glyph run's space y down, then through the transform.
            let (gx, gy) = (at.0 + x * scale, at.1 - y * scale);
            (gx * m11 + gy * m21 + dx, gx * m12 + gy * m22 + dy)
        };
        outline
            .iter()
            .map(|op| match *op {
                PathOp::Move(x, y) => {
                    let (x, y) = p(x, y);
                    Step::Move(x, y)
                }
                PathOp::Line(x, y) => {
                    let (x, y) = p(x, y);
                    Step::Line(x, y)
                }
                PathOp::Quad(a, b, x, y) => {
                    let ((a, b), (x, y)) = (p(a, b), p(x, y));
                    Step::Quad(a, b, x, y)
                }
                PathOp::Cubic(a, b, c, d, x, y) => {
                    let ((a, b), (c, d), (x, y)) = (p(a, b), p(c, d), p(x, y));
                    Step::Cubic(a, b, c, d, x, y)
                }
                PathOp::Close => Step::Close,
            })
            .collect()
    }

    /// A table's bytes, kept once read.
    pub fn table(&self, tag: [u8; 4]) -> Option<Arc<Vec<u8>>> {
        if let Some(t) = lock(&self.tables).get(&tag) {
            return t.clone();
        }
        let t = self.host.table(tag).map(Arc::new);
        lock(&self.tables).insert(tag, t.clone());
        t
    }

    /// The face as a font file, made of its tables.
    pub fn file(&self) -> Arc<Vec<u8>> {
        self.file
            .get_or_init(|| {
                let tables: Vec<([u8; 4], Vec<u8>)> = self.host.tables().into_iter().filter_map(|tag| Some((tag, self.table(tag)?.to_vec()))).collect();
                Arc::new(sfnt::build(&tables))
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, weight: u16, italic: bool) -> FontFace {
        FontFace { id: format!("{family}-{weight}"), family: family.into(), style: String::new(), full_name: String::new(), weight, stretch: 5, italic, monospace: false }
    }

    #[test]
    fn families_stand_ins_and_matching() {
        let fonts = Fonts::new(vec![face("Helvetica Neue", 700, false), face("Menlo", 400, false), face("Helvetica Neue", 400, false), face("Helvetica Neue", 300, false), face("helvetica neue", 400, true)]);
        assert_eq!(fonts.families.len(), 2);
        let hn = fonts.find("HELVETICA NEUE").unwrap();
        assert_eq!(fonts.families[hn].faces.len(), 4);
        assert_eq!(fonts.find("Segoe UI"), Some(hn));
        assert_eq!(fonts.find("Consolas"), fonts.find("Menlo"));
        assert_eq!(fonts.find("Wingdings"), None);
        let w = |weight, italic| fonts.faces[fonts.matching(hn, weight, 5, italic).unwrap()].weight;
        assert_eq!((w(400, false), w(700, false), w(600, false), w(350, false), w(900, false)), (400, 700, 700, 300, 700));
        assert!(fonts.faces[fonts.matching(hn, 400, 5, true).unwrap()].italic);
    }
}
