//! Code pages: what `*A` functions' bytes mean, and `MultiByteToWideChar` and `WideCharToMultiByte`. atlas is a US
//! English Windows: its ANSI code page is Windows-1252 and its OEM one (the console's) 437. UTF-8 (65001), US-ASCII
//! (20127) and Latin-1 (28591) convert too.

use atlas_abi::ansi::{from_1252, to_1252};

pub const ACP: u32 = 1252;
pub const OEMCP: u32 = 437;
pub const UTF8: u32 = 65001;

/// 437's upper half: accented letters, box drawing, Greek and mathematics.
const OEM_HIGH: [u16; 128] = [
    0x00c7, 0x00fc, 0x00e9, 0x00e2, 0x00e4, 0x00e0, 0x00e5, 0x00e7, 0x00ea, 0x00eb, 0x00e8, 0x00ef, 0x00ee, 0x00ec, 0x00c4, 0x00c5,
    0x00c9, 0x00e6, 0x00c6, 0x00f4, 0x00f6, 0x00f2, 0x00fb, 0x00f9, 0x00ff, 0x00d6, 0x00dc, 0x00a2, 0x00a3, 0x00a5, 0x20a7, 0x0192,
    0x00e1, 0x00ed, 0x00f3, 0x00fa, 0x00f1, 0x00d1, 0x00aa, 0x00ba, 0x00bf, 0x2310, 0x00ac, 0x00bd, 0x00bc, 0x00a1, 0x00ab, 0x00bb,
    0x2591, 0x2592, 0x2593, 0x2502, 0x2524, 0x2561, 0x2562, 0x2556, 0x2555, 0x2563, 0x2551, 0x2557, 0x255d, 0x255c, 0x255b, 0x2510,
    0x2514, 0x2534, 0x252c, 0x251c, 0x2500, 0x253c, 0x255e, 0x255f, 0x255a, 0x2554, 0x2569, 0x2566, 0x2560, 0x2550, 0x256c, 0x2567,
    0x2568, 0x2564, 0x2565, 0x2559, 0x2558, 0x2552, 0x2553, 0x256b, 0x256a, 0x2518, 0x250c, 0x2588, 0x2584, 0x258c, 0x2590, 0x2580,
    0x03b1, 0x00df, 0x0393, 0x03c0, 0x03a3, 0x03c3, 0x00b5, 0x03c4, 0x03a6, 0x0398, 0x03a9, 0x03b4, 0x221e, 0x03c6, 0x03b5, 0x2229,
    0x2261, 0x00b1, 0x2265, 0x2264, 0x2320, 0x2321, 0x00f7, 0x2248, 0x00b0, 0x2219, 0x00b7, 0x221a, 0x207f, 0x00b2, 0x25a0, 0x00a0,
];

/// A string that doesn't convert, as MB_ERR_INVALID_CHARS and WC_ERR_INVALID_CHARS ask to hear:
/// ERROR_NO_UNICODE_TRANSLATION.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Untranslatable;

/// The code page a function's `CodePage` names: CP_ACP and CP_THREAD_ACP are 1252, CP_OEMCP 437. None for one atlas
/// doesn't have.
pub fn resolve(cp: u32) -> Option<u32> {
    match cp {
        0 | 3 => Some(ACP),
        1 => Some(OEMCP),
        1252 | 437 | UTF8 | 20127 | 28591 => Some(cp),
        _ => None,
    }
}

/// The most bytes a character takes.
pub fn max_char_size(cp: u32) -> u32 {
    if cp == UTF8 { 4 } else { 1 }
}

/// Bytes in a code page as UTF-16. With `strict`, UTF-8 that isn't valid is an error, as MB_ERR_INVALID_CHARS asks;
/// otherwise it's U+FFFD.
pub fn decode(cp: u32, bytes: &[u8], strict: bool) -> Result<Vec<u16>, Untranslatable> {
    Ok(match cp {
        UTF8 => {
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => std::borrow::Cow::Borrowed(text),
                Err(_) if strict => return Err(Untranslatable),
                Err(_) => String::from_utf8_lossy(bytes),
            };
            text.encode_utf16().collect()
        }
        437 => bytes.iter().map(|&b| if b < 0x80 { u16::from(b) } else { OEM_HIGH[(b - 0x80) as usize] }).collect(),
        20127 => bytes.iter().map(|&b| u16::from(b & 0x7f)).collect(),
        28591 => bytes.iter().map(|&b| u16::from(b)).collect(),
        _ => bytes.iter().map(|&b| from_1252(b)).collect(),
    })
}

/// UTF-16 in a code page: each character it hasn't is `default`. The bytes, and whether a default was used. In
/// UTF-8, a lone surrogate is U+FFFD, or with `strict` (WC_ERR_INVALID_CHARS) an error.
pub fn encode(cp: u32, units: &[u16], default: u8, strict: bool) -> Result<(Vec<u8>, bool), Untranslatable> {
    if cp == UTF8 {
        let mut out = String::new();
        for c in char::decode_utf16(units.iter().copied()) {
            match c {
                Ok(c) => out.push(c),
                Err(_) if strict => return Err(Untranslatable),
                Err(_) => out.push('\u{fffd}'),
            }
        }
        return Ok((out.into_bytes(), false));
    }
    let mut used = false;
    let bytes = units
        .iter()
        .map(|&u| {
            let b = match cp {
                437 => if u < 0x80 { Some(u as u8) } else { OEM_HIGH.iter().position(|&h| h == u).map(|i| 0x80 + i as u8) },
                20127 => (u < 0x80).then_some(u as u8),
                28591 => (u < 0x100).then_some(u as u8),
                _ => to_1252(u),
            };
            b.unwrap_or_else(|| {
                used = true;
                default
            })
        })
        .collect();
    Ok((bytes, used))
}

/// A program's ANSI string, as text.
pub fn text(bytes: &[u8]) -> String {
    String::from_utf16_lossy(&decode(ACP, bytes, false).unwrap_or_default())
}

/// Text as a program's ANSI string, `?` for what 1252 hasn't (no NUL).
pub fn bytes(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    encode(ACP, &units, b'?', false).map(|(b, _)| b).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_pages_both_ways() {
        assert_eq!(text(b"caf\xe9 \x80 \x93q\x94"), "café € “q”");
        assert_eq!(bytes("café € ✓"), b"caf\xe9 \x80 ?");
        assert_eq!(decode(OEMCP, b"\x82\xc9\xe1", false).unwrap(), [0xe9, 0x2554, 0xdf]);
        assert_eq!(encode(OEMCP, &[0xe9, 0x2554, 0x20ac], b'?', false).unwrap(), (b"\x82\xc9?".to_vec(), true));
        assert_eq!(decode(UTF8, "é✓".as_bytes(), true).unwrap(), "é✓".encode_utf16().collect::<Vec<_>>());
        assert!(decode(UTF8, b"\xff", true).is_err());
        assert_eq!(decode(UTF8, b"a\xff", false).unwrap(), [u16::from(b'a'), 0xfffd]);
        assert!(encode(UTF8, &[0xd800], b'?', true).is_err());
        for b in 0..=255u8 {
            assert_eq!(to_1252(from_1252(b)), Some(b));
        }
        assert_eq!(resolve(0), Some(1252));
        assert_eq!(resolve(1), Some(437));
        assert_eq!(resolve(932), None);
    }
}
