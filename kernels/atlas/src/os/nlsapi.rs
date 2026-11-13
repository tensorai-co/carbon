//! Code pages, kernel32's face: `MultiByteToWideChar`, `WideCharToMultiByte`, the ANSI and OEM code pages and what
//! they are, over [`crate::ansi`].

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::ansi;
use crate::process::Task;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INVALID_FLAGS: u32 = 1004;
const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;
const ERROR_NOACCESS: u32 = 998;
const MB_ERR_INVALID_CHARS: u32 = 0x8;
const WC_ERR_INVALID_CHARS: u32 = 0x80;
/// The longest string a call takes, NUL-terminated.
const MOST: usize = 1 << 24;

impl Task {
    /// A string of the program's: `len` units, or with -1, up to its NUL and with it. Bytes, or UTF-16's units as
    /// bytes, two each.
    fn counted(&mut self, cx: &mut Cx<'_>, at: u64, len: i32, wide: bool) -> Result<Vec<u8>, u32> {
        let unit = if wide { 2 } else { 1 };
        let read = if len >= 0 {
            self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize * unit))
        } else if wide {
            self.mem(cx, false, |t| {
                let mut out = Vec::new();
                let mut b = [0u8; 2];
                loop {
                    user::read(&t.space(), at + out.len() as u64, &mut b)?;
                    out.extend(b);
                    if b == [0, 0] || out.len() >= MOST {
                        return Ok(out);
                    }
                }
            })
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, MOST)).map(|mut b| {
                b.push(0);
                b
            })
        };
        read.map_err(|_| ERROR_NOACCESS)
    }

    /// Writes a converted string where the program asked, as the conversions do: with no room given, just how much
    /// it would take; with too little, ERROR_INSUFFICIENT_BUFFER.
    fn give_back(&mut self, cx: &mut Cx<'_>, out: u64, room: i32, bytes: &[u8], unit: usize) -> Reply {
        let count = bytes.len() / unit;
        if room == 0 {
            return Reply::Value(count as u64);
        }
        if (room as usize) < count {
            self.set_last_error(cx, ERROR_INSUFFICIENT_BUFFER);
            return Reply::Value(0);
        }
        if self.mem(cx, true, |t| user::write(&mut t.space(), out, bytes)).is_err() {
            self.set_last_error(cx, ERROR_NOACCESS);
            return Reply::Value(0);
        }
        Reply::Value(count as u64)
    }
}

fn failed(task: &mut Task, cx: &mut Cx<'_>, error: u32) -> Reply {
    task.set_last_error(cx, error);
    Reply::Value(0)
}

/// `MultiByteToWideChar(code page, flags, bytes, length, out, room)`.
pub fn multi_byte_to_wide_char(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (flags, len, room) = (a[1] as u32, a[3] as i32, a[5] as i32);
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    if a[2] == 0 || len == 0 || len < -1 || room < 0 || (room > 0 && a[4] == 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    // UTF-8 takes no flags but MB_ERR_INVALID_CHARS.
    if cp == ansi::UTF8 && flags & !MB_ERR_INVALID_CHARS != 0 {
        return failed(task, cx, ERROR_INVALID_FLAGS);
    }
    let bytes = match task.counted(cx, a[2], len, false) {
        Ok(bytes) => bytes,
        Err(e) => return failed(task, cx, e),
    };
    let Ok(units) = ansi::decode(cp, &bytes, flags & MB_ERR_INVALID_CHARS != 0) else { return failed(task, cx, ERROR_NO_UNICODE_TRANSLATION) };
    let out: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    task.give_back(cx, a[4], room, &out, 2)
}

/// `WideCharToMultiByte(code page, flags, units, length, out, room, &default char, &used default)`.
pub fn wide_char_to_multi_byte(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (flags, len, room) = (a[1] as u32, a[3] as i32, a[5] as i32);
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    if a[2] == 0 || len == 0 || len < -1 || room < 0 || (room > 0 && a[4] == 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    // UTF-8 has no default character to give.
    if cp == ansi::UTF8 && (a[6] != 0 || a[7] != 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    let raw = match task.counted(cx, a[2], len, true) {
        Ok(raw) => raw,
        Err(e) => return failed(task, cx, e),
    };
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let default = if a[6] != 0 { task.mem(cx, false, |t| user::read_vec(&t.space(), a[6], 1)).map_or(b'?', |b| b[0]) } else { b'?' };
    let Ok((bytes, used)) = ansi::encode(cp, &units, default, flags & WC_ERR_INVALID_CHARS != 0) else { return failed(task, cx, ERROR_NO_UNICODE_TRANSLATION) };
    if a[7] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[7], u32::from(used)));
    }
    task.give_back(cx, a[4], room, &bytes, 1)
}

pub fn get_acp(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(ansi::ACP))
}

pub fn get_oem_cp(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(ansi::OEMCP))
}

pub fn is_valid_code_page(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    // CP_ACP and the rest aren't code pages themselves.
    Reply::Value(u64::from(a[0] as u32 > 3 && ansi::resolve(a[0] as u32).is_some()))
}

/// CPINFO: the most bytes a character takes, the default character, and no lead bytes.
fn cp_info(cp: u32) -> Vec<u8> {
    let mut b = vec![0u8; 18];
    b[0..4].copy_from_slice(&ansi::max_char_size(cp).to_le_bytes());
    b[4] = b'?';
    b
}

/// `GetCPInfo(code page, &info)`.
pub fn get_cp_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    let b = cp_info(cp);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() { failed(task, cx, ERROR_NOACCESS) } else { Reply::Value(TRUE) }
}

/// `GetCPInfoExW(code page, flags, &info)`: CPINFO, then the Unicode default character, the code page and its name.
pub fn get_cp_info_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    let name = match cp {
        1252 => "1252  (ANSI - Latin I)",
        437 => "437   (OEM - United States)",
        ansi::UTF8 => "65001 (UTF-8)",
        20127 => "20127 (US-ASCII)",
        _ => "28591 (ISO 8859-1 Latin I)",
    };
    let mut b = cp_info(cp);
    b.extend(0x3fu16.to_le_bytes());
    b.extend(cp.to_le_bytes());
    let mut n: Vec<u8> = name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    n.resize(520, 0);
    b.extend(n);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() { failed(task, cx, ERROR_NOACCESS) } else { Reply::Value(TRUE) }
}

/// `IsDBCSLeadByte(Ex)`: none of atlas's code pages has lead bytes.
pub fn is_dbcs_lead_byte(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(FALSE)
}

/// `AreFileApisANSI`: file functions' `*A` strings are ANSI, not OEM.
pub fn are_file_apis_ansi(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(TRUE)
}

/// `SetFileApisToANSI`, `SetFileApisToOEM`: taken, and the ANSI code page kept.
pub fn set_file_apis(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `GetConsoleCP` and `GetConsoleOutputCP`: the console's code page, the OEM one unless it's been set.
pub fn get_console_cp(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(task.k.console_cp.load(std::sync::atomic::Ordering::Relaxed)))
}

/// `SetConsoleCP` and `SetConsoleOutputCP`: a code page atlas has.
pub fn set_console_cp(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match ansi::resolve(a[0] as u32).filter(|_| a[0] as u32 > 3) {
        Some(cp) => {
            task.k.console_cp.store(cp, std::sync::atomic::Ordering::Relaxed);
            Reply::Value(TRUE)
        }
        None => {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            Reply::Value(FALSE)
        }
    }
}

// --- Comparing, mapping and classifying text, as the user's locale (English) does it ---

const NORM_IGNORECASE: u32 = 0x1;
const NORM_IGNORENONSPACE: u32 = 0x2;
const NORM_IGNORESYMBOLS: u32 = 0x4;
const LINGUISTIC_IGNORECASE: u32 = 0x10;
const SORT_STRINGSORT: u32 = 0x1000;
const SORT_DIGITSASNUMBERS: u32 = 0x8;
const LCMAP_LOWERCASE: u32 = 0x100;
const LCMAP_UPPERCASE: u32 = 0x200;
const LCMAP_SORTKEY: u32 = 0x400;
const LCMAP_BYTEREV: u32 = 0x800;

impl Task {
    /// A UTF-16 string the program passed: `len` units, or with -1 up to its NUL (left off).
    fn units_arg(&mut self, cx: &mut Cx<'_>, at: u64, len: i32) -> Result<Vec<u16>, u32> {
        if at == 0 || len < -1 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let raw = self.counted(cx, at, len, true)?;
        let mut units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        if len < 0 {
            units.pop();
        }
        Ok(units)
    }

    /// An ANSI string, the same way, made UTF-16.
    fn ansi_units_arg(&mut self, cx: &mut Cx<'_>, at: u64, len: i32) -> Result<Vec<u16>, u32> {
        if at == 0 || len < -1 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut bytes = self.counted(cx, at, len, false)?;
        if len < 0 {
            bytes.pop();
        }
        Ok(ansi::text(&bytes).encode_utf16().collect())
    }
}

/// Where a character sorts, first: punctuation and spaces, then digits, then letters, then everything else, each by
/// its lowercase form. (Windows' tables go further, putting accented letters with their plain ones; this doesn't.)
fn primary(c: char) -> (u8, u32) {
    let lower = c.to_lowercase().next().unwrap_or(c);
    let class = if c.is_alphabetic() {
        2
    } else if c.is_numeric() {
        1
    } else if c.is_whitespace() || c.is_ascii_punctuation() || (!c.is_alphanumeric() && (c as u32) < 0x3000) {
        0
    } else {
        3
    };
    (class, lower as u32)
}

/// Whether a character counts when comparing with `flags`: word sort (the default) passes over hyphens and
/// apostrophes; NORM_IGNORESYMBOLS over all but letters and digits; NORM_IGNORENONSPACE over combining marks.
fn counts(c: char, flags: u32) -> bool {
    if flags & NORM_IGNORESYMBOLS != 0 && !c.is_alphanumeric() {
        return false;
    }
    if flags & NORM_IGNORENONSPACE != 0 && ('\u{300}'..='\u{36f}').contains(&c) {
        return false;
    }
    flags & SORT_STRINGSORT != 0 || !matches!(c, '-' | '\'' | '\u{2010}' | '\u{2011}' | '\u{2019}')
}

fn chars(units: &[u16], flags: u32) -> Vec<char> {
    char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).filter(|&c| counts(c, flags)).collect()
}

/// A run of digits at the start, as a number, for SORT_DIGITSASNUMBERS: its digits, leading zeros gone.
fn number_at(s: &[char]) -> Option<(String, usize)> {
    let n = s.iter().take_while(|c| c.is_ascii_digit()).count();
    (n > 0).then(|| (s[..n].iter().collect::<String>().trim_start_matches('0').to_string(), n))
}

/// Compares as `CompareString` does: by where the characters sort, and only if that's equal, by case (lowercase
/// first) unless case is ignored, and then by what was passed over.
fn compare(a: &[u16], b: &[u16], flags: u32) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (x, y) = (chars(a, flags), chars(b, flags));
    let (mut i, mut j) = (0, 0);
    while i < x.len() && j < y.len() {
        if flags & SORT_DIGITSASNUMBERS != 0
            && let (Some((m, mi)), Some((n, nj))) = (number_at(&x[i..]), number_at(&y[j..]))
        {
            let o = m.len().cmp(&n.len()).then_with(|| m.cmp(&n));
            if o != Ordering::Equal {
                return o;
            }
            i += mi;
            j += nj;
            continue;
        }
        let o = primary(x[i]).cmp(&primary(y[j]));
        if o != Ordering::Equal {
            return o;
        }
        i += 1;
        j += 1;
    }
    let o = (x.len() - i).cmp(&(y.len() - j));
    if o != Ordering::Equal {
        return o;
    }
    if flags & (NORM_IGNORECASE | LINGUISTIC_IGNORECASE) == 0 {
        for (c, d) in x.iter().zip(&y) {
            if c != d {
                // Lowercase before uppercase.
                return c.is_uppercase().cmp(&d.is_uppercase()).then(c.cmp(d));
            }
        }
    }
    // Equal as far as counted: what word sort passed over (not what the flags said to ignore) decides.
    let all = |s: &[u16]| chars(s, flags | SORT_STRINGSORT).len();
    all(a).cmp(&all(b))
}

fn compare_reply(task: &mut Task, cx: &mut Cx<'_>, a: Result<Vec<u16>, u32>, b: Result<Vec<u16>, u32>, flags: u32) -> Reply {
    match (a, b) {
        (Ok(a), Ok(b)) => Reply::Value((compare(&a, &b, flags) as i8 + 2) as u64),
        (Err(e), _) | (_, Err(e)) => failed(task, cx, e),
    }
}

/// `CompareStringW(locale, flags, a, a's length, b, b's length)`: CSTR_LESS_THAN (1), CSTR_EQUAL (2) or
/// CSTR_GREATER_THAN (3).
pub fn compare_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.units_arg(cx, a[2], a[3] as i32);
    let y = task.units_arg(cx, a[4], a[5] as i32);
    compare_reply(task, cx, x, y, a[1] as u32)
}

/// `CompareStringEx(locale name, flags, a, length, b, length, version, reserved, param)`.
pub fn compare_string_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.units_arg(cx, a[2], a[3] as i32);
    let y = task.units_arg(cx, a[4], a[5] as i32);
    compare_reply(task, cx, x, y, a[1] as u32)
}

pub fn compare_string_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let x = task.ansi_units_arg(cx, a[2], a[3] as i32);
    let y = task.ansi_units_arg(cx, a[4], a[5] as i32);
    compare_reply(task, cx, x, y, a[1] as u32)
}

/// A sort key for `units`: bytes whose order is `compare`'s (case aside), as LCMAP_SORTKEY gives, with no zero byte
/// but the last, so it compares as a C string too.
fn sort_key(units: &[u16], flags: u32) -> Vec<u8> {
    let x = chars(units, flags);
    let mut key = Vec::new();
    for &c in &x {
        let (class, lower) = primary(c);
        key.push(class + 2);
        // The character in base 255, each digit one up: never 0.
        key.extend([(lower / 65025) as u8 + 1, (lower / 255 % 255) as u8 + 1, (lower % 255) as u8 + 1]);
    }
    key.push(1);
    if flags & (NORM_IGNORECASE | LINGUISTIC_IGNORECASE) == 0 {
        key.extend(x.iter().map(|c| if c.is_uppercase() { 0x12 } else { 2 }));
    }
    key.push(1);
    key.push(0);
    key
}

fn lc_map(task: &mut Task, cx: &mut Cx<'_>, flags: u32, src: Result<Vec<u16>, u32>, out: u64, room: i32, with_nul: bool) -> Reply {
    let units = match src {
        Ok(u) => u,
        Err(e) => return failed(task, cx, e),
    };
    if room < 0 || (room > 0 && out == 0) || flags & LCMAP_LOWERCASE != 0 && flags & LCMAP_UPPERCASE != 0 {
        return failed(task, cx, ERROR_INVALID_FLAGS);
    }
    if flags & LCMAP_SORTKEY != 0 {
        return task.give_back(cx, out, room, &sort_key(&units, flags), 1);
    }
    // Character by character, each to one: a character whose case is more than one (ß's uppercase is SS) stays.
    let one = |c: char, mut m: std::iter::Peekable<std::vec::IntoIter<char>>| match (m.next(), m.peek()) {
        (Some(d), None) if d.len_utf16() == c.len_utf16() => d,
        _ => c,
    };
    let mut mapped: Vec<u16> = Vec::with_capacity(units.len());
    for c in char::decode_utf16(units.iter().copied()) {
        let Ok(c) = c else {
            mapped.push(0xfffd);
            continue;
        };
        let c = if flags & LCMAP_LOWERCASE != 0 {
            one(c, c.to_lowercase().collect::<Vec<_>>().into_iter().peekable())
        } else if flags & LCMAP_UPPERCASE != 0 {
            one(c, c.to_uppercase().collect::<Vec<_>>().into_iter().peekable())
        } else {
            c
        };
        let mut b = [0u16; 2];
        mapped.extend_from_slice(c.encode_utf16(&mut b));
    }
    if flags & LCMAP_BYTEREV != 0 {
        mapped.iter_mut().for_each(|u| *u = u.swap_bytes());
    }
    if with_nul {
        mapped.push(0);
    }
    let bytes: Vec<u8> = mapped.iter().flat_map(|u| u.to_le_bytes()).collect();
    task.give_back(cx, out, room, &bytes, 2)
}

/// `LCMapStringW(locale, flags, source, length, out, room)`: lowercased, uppercased, byte-reversed, or a sort key
/// (whose room is in bytes); the other mappings (kana, widths, Chinese scripts) leave it as it is.
pub fn lc_map_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.units_arg(cx, a[2], a[3] as i32);
    lc_map(task, cx, a[1] as u32, src, a[4], a[5] as i32, a[3] as i32 == -1)
}

/// `LCMapStringEx(locale name, flags, source, length, out, room, version, reserved, sort handle)`.
pub fn lc_map_string_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.units_arg(cx, a[2], a[3] as i32);
    lc_map(task, cx, a[1] as u32, src, a[4], a[5] as i32, a[3] as i32 == -1)
}

/// A character's CT_CTYPE1 bits: C1_UPPER, C1_LOWER, C1_DIGIT, C1_SPACE, C1_PUNCT, C1_CNTRL, C1_BLANK, C1_XDIGIT,
/// C1_ALPHA, C1_DEFINED.
fn ctype1(c: char) -> u16 {
    let mut bits = 0x200;
    for (yes, bit) in [
        (c.is_uppercase(), 0x1),
        (c.is_lowercase(), 0x2),
        (c.is_ascii_digit(), 0x4),
        (c.is_whitespace(), 0x8),
        (c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && (c as u32) > 0x7f), 0x10),
        (c.is_control(), 0x20),
        (c == ' ' || c == '\t', 0x40),
        (c.is_ascii_hexdigit(), 0x80),
        (c.is_alphabetic(), 0x100),
    ] {
        if yes {
            bits |= bit;
        }
    }
    bits
}

/// CT_CTYPE2: the character's direction: C2_LEFTTORIGHT for letters, C2_EUROPENUMBER for digits, C2_WHITESPACE,
/// C2_SEGMENTSEPARATOR for tabs, C2_BLOCKSEPARATOR for line ends, else C2_OTHERNEUTRAL.
fn ctype2(c: char) -> u16 {
    match c {
        '\t' => 8,
        '\n' | '\r' => 7,
        c if c.is_ascii_digit() => 3,
        c if c.is_whitespace() => 10,
        c if c.is_alphabetic() => 1,
        _ => 11,
    }
}

/// CT_CTYPE3: C3_ALPHA for letters, C3_SYMBOL for symbols, C3_NONSPACING for combining marks.
fn ctype3(c: char) -> u16 {
    if c.is_alphabetic() {
        0x8000
    } else if ('\u{300}'..='\u{36f}').contains(&c) {
        1
    } else if !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() {
        8
    } else {
        0
    }
}

fn string_type(task: &mut Task, cx: &mut Cx<'_>, info: u32, src: Result<Vec<u16>, u32>, out: u64) -> Reply {
    let units = match src {
        Ok(u) => u,
        Err(e) => return failed(task, cx, e),
    };
    let f = match info {
        1 => ctype1,
        2 => ctype2,
        4 => ctype3,
        _ => return failed(task, cx, ERROR_INVALID_FLAGS),
    };
    // One entry per UTF-16 unit; a surrogate pair's halves both say what the pair is.
    let mut types = Vec::with_capacity(units.len() * 2);
    for c in char::decode_utf16(units.iter().copied()) {
        let c = c.unwrap_or('\u{fffd}');
        for _ in 0..c.len_utf16() {
            types.extend(f(c).to_le_bytes());
        }
    }
    if out == 0 || task.mem(cx, true, |t| user::write(&mut t.space(), out, &types)).is_err() {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(TRUE)
}

/// `GetStringTypeW(info type, source, length, &types)`.
pub fn get_string_type_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.units_arg(cx, a[1], a[2] as i32);
    string_type(task, cx, a[0] as u32, src, a[3])
}

/// `GetStringTypeExW(locale, info type, source, length, &types)`.
pub fn get_string_type_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.units_arg(cx, a[2], a[3] as i32);
    string_type(task, cx, a[1] as u32, src, a[4])
}


/// `FoldStringW(flags, source, length, out, room)`: the text as it is (its compatibility forms, ligatures and
/// digits are folded already, being what English text has).
pub fn fold_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.units_arg(cx, a[1], a[2] as i32);
    let units = match src {
        Ok(u) => u,
        Err(e) => return failed(task, cx, e),
    };
    let mut out = units;
    if a[2] as i32 == -1 {
        out.push(0);
    }
    let bytes: Vec<u8> = out.iter().flat_map(|u| u.to_le_bytes()).collect();
    task.give_back(cx, a[3], a[4] as i32, &bytes, 2)
}

/// `LCMapStringA(locale, flags, source, length, out, room)`.
pub fn lc_map_string_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let flags = a[1] as u32;
    let src = task.ansi_units_arg(cx, a[2], a[3] as i32);
    if flags & LCMAP_SORTKEY != 0 {
        return lc_map(task, cx, flags, src, a[4], a[5] as i32, false);
    }
    let units = match src {
        Ok(u) => u,
        Err(e) => return failed(task, cx, e),
    };
    let text = String::from_utf16_lossy(&units);
    let mapped = if flags & LCMAP_LOWERCASE != 0 { text.to_lowercase() } else if flags & LCMAP_UPPERCASE != 0 { text.to_uppercase() } else { text };
    let mut bytes = crate::ansi::bytes(&mapped).to_vec();
    if a[3] as i32 == -1 {
        bytes.push(0);
    }
    task.give_back(cx, a[4], a[5] as i32, &bytes, 1)
}

/// `GetStringTypeA(locale, info type, source, length, &types)`.
pub fn get_string_type_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let src = task.ansi_units_arg(cx, a[2], a[3] as i32);
    string_type(task, cx, a[1] as u32, src, a[4])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn compares_as_windows_sorts() {
        assert_eq!(compare(&w("apple"), &w("Banana"), 0), Less);
        assert_eq!(compare(&w("a"), &w("A"), 0), Less);
        assert_eq!(compare(&w("abc"), &w("ABC"), NORM_IGNORECASE), Equal);
        assert_eq!(compare(&w("co-op"), &w("coop"), 0), Greater);
        assert_eq!(compare(&w("co-op"), &w("cop"), 0), Less);
        assert_eq!(compare(&w("co-op"), &w("coop"), SORT_STRINGSORT), Less);
        assert_eq!(compare(&w("_x"), &w("1"), 0), Less);
        assert_eq!(compare(&w("9"), &w("a"), 0), Less);
        assert_eq!(compare(&w("file10"), &w("file9"), SORT_DIGITSASNUMBERS), Greater);
        assert_eq!(compare(&w("a.b"), &w("ab"), NORM_IGNORESYMBOLS), Equal);
        let key = |s: &str| sort_key(&w(s), 0);
        assert!(primary('ß') > primary('s'));
        assert!(key("apple") < key("banana") && key("a") < key("A") && key("Ab") < key("b"));
    }
}
