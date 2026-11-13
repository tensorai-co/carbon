//! kernel32's and user32's small string and number functions, in the program as Windows' are, since programs
//! (installers most of all) call them all the time: `lstrlen`, `lstrcpy(n)`, `lstrcat`, `lstrcmp(i)` in their A and W
//! forms, `CharNext` and `CharPrev`, `MulDiv`, and `CompareFileTime`.
//!
//! `lstrcmp` and `lstrcmpi` compare as Windows' word sort does for the common case: letters by their lower-case form,
//! then (for `lstrcmp`) a lower-case letter before its upper-case one. Locales' other rules aren't here.

/// A NUL-terminated string's length, in units.
///
/// # Safety
///
/// `s` is null or a NUL-terminated string.
unsafe fn len<T: Copy + Default + PartialEq>(s: *const T) -> usize {
    if s.is_null() {
        return 0;
    }
    let mut n = 0;
    // SAFETY: as the caller says.
    while unsafe { *s.add(n) } != T::default() {
        n += 1;
    }
    n
}

/// A unit as UTF-16: ANSI's is Windows-1252.
trait Unit: Copy + Default + PartialEq {
    fn wide(self) -> u16;
}

impl Unit for u16 {
    fn wide(self) -> u16 {
        self
    }
}

impl Unit for u8 {
    fn wide(self) -> u16 {
        atlas_abi::ansi::from_1252(self)
    }
}

/// A unit's lower-case form, where it has one of its own.
fn fold(u: u16) -> u16 {
    match char::from_u32(u32::from(u)) {
        Some(c) if c.is_ascii() => c.to_ascii_lowercase() as u16,
        Some(c) => {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) if (l as u32) < 0x1_0000 => l as u16,
                _ => u,
            }
        }
        None => u,
    }
}

/// -1, 0 or 1, as `lstrcmp(i)` gives; null is less than anything else.
///
/// # Safety
///
/// Each is null or a NUL-terminated string.
unsafe fn compare<T: Unit>(a: *const T, b: *const T, ignore_case: bool) -> i32 {
    match (a.is_null(), b.is_null()) {
        (true, true) => return 0,
        (true, false) => return -1,
        (false, true) => return 1,
        _ => {}
    }
    // SAFETY: as the caller says.
    let (a, b) = unsafe { (core::slice::from_raw_parts(a, len(a)), core::slice::from_raw_parts(b, len(b))) };
    let order = |x: core::cmp::Ordering| x as i32;
    let folded = a.iter().map(|u| fold(u.wide())).cmp(b.iter().map(|u| fold(u.wide())));
    if folded != core::cmp::Ordering::Equal || ignore_case {
        return order(folded);
    }
    // The same but for case: the first that differs, lower-case first.
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x.wide(), y.wide());
        if x != y {
            return if fold(x) == x { -1 } else { 1 };
        }
    }
    0
}

/// # Safety
///
/// `dst` has room for `src` and its NUL, or at least `most` units; `src` is a NUL-terminated string.
unsafe fn copy<T: Unit>(dst: *mut T, src: *const T, most: Option<usize>) -> *mut T {
    if dst.is_null() || src.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: as the caller says.
    unsafe {
        let n = len(src);
        let n = most.map_or(n, |m| n.min(m.saturating_sub(1)));
        if most == Some(0) {
            return dst;
        }
        core::ptr::copy(src, dst, n);
        *dst.add(n) = T::default();
    }
    dst
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrlenW(s: *const u16) -> i32 {
    // SAFETY: the program's string.
    unsafe { len(s) as i32 }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrlenA(s: *const u8) -> i32 {
    // SAFETY: the program's string.
    unsafe { len(s) as i32 }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcpyW(dst: *mut u16, src: *const u16) -> *mut u16 {
    // SAFETY: the program's buffers.
    unsafe { copy(dst, src, None) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcpyA(dst: *mut u8, src: *const u8) -> *mut u8 {
    // SAFETY: the program's buffers.
    unsafe { copy(dst, src, None) }
}

/// `lstrcpyn(dst, src, most)`: at most `most` units, its NUL among them.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcpynW(dst: *mut u16, src: *const u16, most: i32) -> *mut u16 {
    // SAFETY: the program's buffers.
    unsafe { copy(dst, src, Some(most.max(0) as usize)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcpynA(dst: *mut u8, src: *const u8, most: i32) -> *mut u8 {
    // SAFETY: the program's buffers.
    unsafe { copy(dst, src, Some(most.max(0) as usize)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcatW(dst: *mut u16, src: *const u16) -> *mut u16 {
    if dst.is_null() {
        return dst;
    }
    // SAFETY: the program's buffers: `dst` has room for both.
    unsafe {
        copy(dst.add(len(dst)), src, None);
    }
    dst
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcatA(dst: *mut u8, src: *const u8) -> *mut u8 {
    if dst.is_null() {
        return dst;
    }
    // SAFETY: as above.
    unsafe {
        copy(dst.add(len(dst)), src, None);
    }
    dst
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcmpW(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the program's strings.
    unsafe { compare(a, b, false) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcmpA(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the program's strings.
    unsafe { compare(a, b, false) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcmpiW(a: *const u16, b: *const u16) -> i32 {
    // SAFETY: the program's strings.
    unsafe { compare(a, b, true) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLstrcmpiA(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the program's strings.
    unsafe { compare(a, b, true) }
}

/// `CharNextW(p)`: the next character, or `p` at the NUL.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCharNextW(p: *const u16) -> *const u16 {
    // SAFETY: the program's string.
    if p.is_null() || unsafe { *p } == 0 { p } else { p.wrapping_add(1) }
}

/// `CharNextA(p)`: Windows-1252 has a byte a character.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCharNextA(p: *const u8) -> *const u8 {
    // SAFETY: the program's string.
    if p.is_null() || unsafe { *p } == 0 { p } else { p.wrapping_add(1) }
}

/// `CharPrevW(start, p)`: the character before `p`, or `start`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCharPrevW(start: *const u16, p: *const u16) -> *const u16 {
    if p <= start { start } else { p.wrapping_sub(1) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCharPrevA(start: *const u8, p: *const u8) -> *const u8 {
    if p <= start { start } else { p.wrapping_sub(1) }
}

/// `MulDiv(a, b, c)`: a * b / c in 64 bits, rounded half away from zero; -1 if c is 0 or it doesn't fit.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasMulDiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let product = i64::from(a) * i64::from(b);
    let (c, half) = (i64::from(c), i64::from(c).abs() / 2);
    let rounded = if (product < 0) == (c < 0) { (product.abs() + half) / c.abs() } else { -((product.abs() + half) / c.abs()) };
    i32::try_from(rounded).unwrap_or(-1)
}

/// `CompareFileTime(&a, &b)`: -1, 0 or 1.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCompareFileTime(a: *const u64, b: *const u64) -> i32 {
    // SAFETY: the program's FILETIMEs.
    let (a, b) = unsafe { (a.read_unaligned(), b.read_unaligned()) };
    a.cmp(&b) as i32
}
