//! The fonts macOS has, through Core Text: every face of every family (but the system's private ones, whose names
//! start with a dot), each face's tables as it has them, and its glyphs' outlines in font units.

use std::ffi::{c_char, c_void};
use std::sync::Arc;

use crate::fonts::{FontFace, HostFont, PathOp, weight_class, width_class};

type CFTypeRef = *const c_void;
type CFIndex = isize;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct CGPathElement {
    kind: i32,
    points: *const CGPoint,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: CFTypeRef);
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFStringCreateWithBytes(allocator: CFTypeRef, bytes: *const u8, length: CFIndex, encoding: u32, external: u8) -> CFTypeRef;
    fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
    fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    fn CFNumberGetValue(number: CFTypeRef, kind: CFIndex, value: *mut c_void) -> u8;
    fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    static kCTFontNameAttribute: CFTypeRef;
    static kCTFontFamilyNameAttribute: CFTypeRef;
    static kCTFontStyleNameAttribute: CFTypeRef;
    static kCTFontDisplayNameAttribute: CFTypeRef;
    static kCTFontTraitsAttribute: CFTypeRef;
    static kCTFontWeightTrait: CFTypeRef;
    static kCTFontWidthTrait: CFTypeRef;
    static kCTFontSymbolicTrait: CFTypeRef;
    fn CTFontCollectionCreateFromAvailableFonts(options: CFTypeRef) -> CFTypeRef;
    fn CTFontCollectionCreateMatchingFontDescriptors(collection: CFTypeRef) -> CFTypeRef;
    fn CTFontDescriptorCopyAttribute(descriptor: CFTypeRef, attribute: CFTypeRef) -> CFTypeRef;
    fn CTFontCreateWithName(name: CFTypeRef, size: f64, matrix: *const c_void) -> CFTypeRef;
    fn CTFontCreateCopyWithAttributes(font: CFTypeRef, size: f64, matrix: *const c_void, attributes: CFTypeRef) -> CFTypeRef;
    fn CTFontCopyPostScriptName(font: CFTypeRef) -> CFTypeRef;
    fn CTFontGetUnitsPerEm(font: CFTypeRef) -> u32;
    fn CTFontGetGlyphCount(font: CFTypeRef) -> CFIndex;
    fn CTFontCopyAvailableTables(font: CFTypeRef, options: u32) -> CFTypeRef;
    fn CTFontCopyTable(font: CFTypeRef, tag: u32, options: u32) -> CFTypeRef;
    fn CTFontCreatePathForGlyph(font: CFTypeRef, glyph: u16, matrix: *const c_void) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPathApply(path: CFTypeRef, info: *mut c_void, function: extern "C" fn(*mut c_void, *const CGPathElement));
}

const UTF8: u32 = 0x0800_0100;
const NUMBER_DOUBLE: CFIndex = 13;
const NUMBER_SINT32: CFIndex = 3;
/// kCTFontTraitItalic and kCTFontTraitMonoSpace.
const ITALIC: u32 = 1 << 0;
const MONOSPACE: u32 = 1 << 10;
/// kCTFontTableOptionNoOptions.
const NO_OPTIONS: u32 = 0;

/// Something of Core Foundation's that's ours to release.
struct Owned(CFTypeRef);

impl Owned {
    fn new(object: CFTypeRef) -> Option<Owned> {
        (!object.is_null()).then(|| Owned(object))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: it's ours, and not null.
        unsafe { CFRelease(self.0) };
    }
}

fn string(s: CFTypeRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: a CFString; the buffer has room for every character in UTF-8 and the NUL.
    unsafe {
        let room = CFStringGetLength(s) * 4 + 1;
        let mut buffer = vec![0u8; room as usize];
        if CFStringGetCString(s, buffer.as_mut_ptr().cast(), room, UTF8) == 0 {
            return None;
        }
        let n = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
        buffer.truncate(n);
        String::from_utf8(buffer).ok()
    }
}

fn cf_string(s: &str) -> Option<Owned> {
    // SAFETY: the bytes are UTF-8, and copied.
    Owned::new(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as CFIndex, UTF8, 0) })
}

fn attribute(descriptor: CFTypeRef, key: CFTypeRef) -> Option<Owned> {
    // SAFETY: a descriptor from the array that's still kept; the copy is ours.
    Owned::new(unsafe { CTFontDescriptorCopyAttribute(descriptor, key) })
}

fn number<T: Default>(n: CFTypeRef, kind: CFIndex) -> Option<T> {
    if n.is_null() {
        return None;
    }
    let mut v = T::default();
    // SAFETY: a CFNumber, read as the kind `T` is.
    (unsafe { CFNumberGetValue(n, kind, (&raw mut v).cast()) } != 0).then_some(v)
}

/// Every face macOS has, but the system's private ones.
pub fn fonts() -> Vec<FontFace> {
    // SAFETY: Core Text's calls, on objects kept for as long as they're used.
    unsafe {
        let Some(collection) = Owned::new(CTFontCollectionCreateFromAvailableFonts(std::ptr::null())) else { return Vec::new() };
        let Some(descriptors) = Owned::new(CTFontCollectionCreateMatchingFontDescriptors(collection.0)) else { return Vec::new() };
        let mut faces = Vec::new();
        for i in 0..CFArrayGetCount(descriptors.0) {
            let d = CFArrayGetValueAtIndex(descriptors.0, i);
            let text = |key| attribute(d, key).and_then(|s| string(s.0));
            let (Some(id), Some(family)) = (text(kCTFontNameAttribute), text(kCTFontFamilyNameAttribute)) else { continue };
            if id.starts_with('.') || family.starts_with('.') {
                continue;
            }
            let style = text(kCTFontStyleNameAttribute).unwrap_or_else(|| "Regular".into());
            let full_name = text(kCTFontDisplayNameAttribute).unwrap_or_else(|| format!("{family} {style}"));
            let traits = attribute(d, kCTFontTraitsAttribute);
            let get = |key| traits.as_ref().map_or(std::ptr::null(), |t| CFDictionaryGetValue(t.0, key));
            let weight = number::<f64>(get(kCTFontWeightTrait), NUMBER_DOUBLE).unwrap_or(0.0);
            let width = number::<f64>(get(kCTFontWidthTrait), NUMBER_DOUBLE).unwrap_or(0.0);
            let symbolic = number::<i32>(get(kCTFontSymbolicTrait), NUMBER_SINT32).unwrap_or(0) as u32;
            faces.push(FontFace {
                id,
                family,
                style,
                full_name,
                weight: weight_class(weight),
                stretch: width_class(width),
                italic: symbolic & ITALIC != 0,
                monospace: symbolic & MONOSPACE != 0,
            });
        }
        faces
    }
}

/// A face open, at its units per em, so its outlines come in font units.
struct Font(Owned);

// SAFETY: Core Text's fonts are immutable, and may be used from any thread.
unsafe impl Send for Font {}
// SAFETY: as above.
unsafe impl Sync for Font {}

/// A face by its PostScript name, if macOS has one by that name exactly (Core Text gives a fallback for a name it
/// hasn't, which isn't the face asked for).
pub fn open_font(id: &str) -> Option<Arc<dyn HostFont>> {
    let name = cf_string(id)?;
    // SAFETY: Core Text's calls, on objects kept for as long as they're used.
    unsafe {
        let probe = Owned::new(CTFontCreateWithName(name.0, 12.0, std::ptr::null()))?;
        let got = Owned::new(CTFontCopyPostScriptName(probe.0)).and_then(|s| string(s.0));
        if got.as_deref() != Some(id) {
            return None;
        }
        let units = f64::from(CTFontGetUnitsPerEm(probe.0).max(16));
        let font = Owned::new(CTFontCreateCopyWithAttributes(probe.0, units, std::ptr::null(), std::ptr::null()))?;
        Some(Arc::new(Font(font)))
    }
}

/// Collects a CGPath's elements, y up as Core Graphics has it.
extern "C" fn element(info: *mut c_void, e: *const CGPathElement) {
    // SAFETY: `info` is the Vec `outline` passes; `e` and its points are Core Graphics', valid for this call.
    unsafe {
        let ops = &mut *info.cast::<Vec<PathOp>>();
        let e = &*e;
        let p = |i: usize| {
            let p = *e.points.add(i);
            (p.x as f32, p.y as f32)
        };
        ops.push(match e.kind {
            0 => PathOp::Move(p(0).0, p(0).1),
            1 => PathOp::Line(p(0).0, p(0).1),
            2 => PathOp::Quad(p(0).0, p(0).1, p(1).0, p(1).1),
            3 => PathOp::Cubic(p(0).0, p(0).1, p(1).0, p(1).1, p(2).0, p(2).1),
            _ => PathOp::Close,
        });
    }
}

impl HostFont for Font {
    fn tables(&self) -> Vec<[u8; 4]> {
        // SAFETY: the array holds tags as its values, not objects.
        unsafe {
            let Some(tags) = Owned::new(CTFontCopyAvailableTables(self.0.0, NO_OPTIONS)) else { return Vec::new() };
            (0..CFArrayGetCount(tags.0)).map(|i| (CFArrayGetValueAtIndex(tags.0, i) as usize as u32).to_be_bytes()).collect()
        }
    }

    fn table(&self, tag: [u8; 4]) -> Option<Vec<u8>> {
        // SAFETY: the data is ours, and read for its length.
        unsafe {
            let data = Owned::new(CTFontCopyTable(self.0.0, u32::from_be_bytes(tag), NO_OPTIONS))?;
            let n = CFDataGetLength(data.0) as usize;
            let at = CFDataGetBytePtr(data.0);
            Some(if n == 0 || at.is_null() { Vec::new() } else { std::slice::from_raw_parts(at, n).to_vec() })
        }
    }

    fn outline(&self, glyph: u16) -> Option<Vec<PathOp>> {
        // SAFETY: the path is ours; the callback is given the Vec it fills.
        unsafe {
            if glyph as CFIndex >= CTFontGetGlyphCount(self.0.0) {
                return None;
            }
            let mut ops: Vec<PathOp> = Vec::new();
            if let Some(path) = Owned::new(CTFontCreatePathForGlyph(self.0.0, glyph, std::ptr::null())) {
                CGPathApply(path.0, (&raw mut ops).cast(), element);
            }
            Some(ops)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_tables_and_outlines() {
        let faces = fonts();
        let helvetica = faces.iter().find(|f| f.family == "Helvetica" && f.style == "Regular").expect("Helvetica, which every Mac has");
        assert_eq!((helvetica.weight, helvetica.stretch, helvetica.italic), (400, 5, false));
        assert!(faces.iter().any(|f| f.family == "Helvetica" && f.weight == 700));
        assert!(faces.iter().all(|f| !f.id.starts_with('.')));
        let font = open_font(&helvetica.id).expect("it opens");
        assert!(font.tables().contains(b"head") && font.tables().contains(b"cmap"));
        let head = font.table(*b"head").unwrap();
        assert_eq!(&head[12..16], &[0x5f, 0x0f, 0x3c, 0xf5], "head's magic number");
        // Glyph 0 is .notdef; some glyph past it draws something.
        assert!((1..64).any(|g| font.outline(g).is_some_and(|o| o.len() > 2 && matches!(o[0], PathOp::Move(..)))));
        assert!(font.outline(u16::MAX).is_none());
        assert!(open_font("NoSuchFont-AtAll").is_none());
    }
}
