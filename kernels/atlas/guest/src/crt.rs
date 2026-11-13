//! The C runtime, as `ucrtbase.dll` has it (atlas's own: on Windows 10 it's part of the OS), in the program: the heap
//! (`malloc` and the rest over ntdll's heap), memory and strings, narrow and wide; character classes; number
//! conversions; sorting and searching; `errno`; and what a program's startup and exit need (its arguments and
//! environment, `_initterm`, `atexit` and `exit`, `abort` and `signal`). Its stdio and `printf` are `stdio`'s.
//!
//! Every function here is C's: `extern "C"`, which on 64-bit Windows is the one calling convention.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use atlas_abi::ansi::{from_1252, to_1252};

use crate::sys::host;

host!(get_command_line_a, b"GetCommandLineA", ());
host!(get_system_time_as_file_time, b"GetSystemTimeAsFileTime", (out));

// ---- errno ----

/// This thread's `errno`, in atlas-guest's own room past the TEB.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtErrno() -> *mut i32 {
    // SAFETY: atlas-guest's own slot in this thread's TEB.
    unsafe { crate::teb().add(atlas_abi::teb::ATLAS_ERRNO as usize).cast() }
}

pub fn set_errno(e: i32) {
    // SAFETY: as above.
    unsafe { AtlasCrtErrno().write(e) };
}

const EINVAL: i32 = 22;
const ERANGE: i32 = 34;
const ENOMEM: i32 = 12;

// ---- The heap ----

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMalloc(size: usize) -> *mut u8 {
    let p = crate::heap::allocate(size.max(1), false);
    if p.is_null() {
        set_errno(ENOMEM);
    }
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtCalloc(count: usize, size: usize) -> *mut u8 {
    let Some(total) = count.checked_mul(size) else {
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    };
    let p = crate::heap::allocate(total.max(1), true);
    if p.is_null() {
        set_errno(ENOMEM);
    }
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFree(p: *mut u8) {
    crate::heap::free(p);
}

/// `realloc`: a block as big, with what fitted of the old one; with no block, `malloc`; with no size, `free`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtRealloc(p: *mut u8, size: usize) -> *mut u8 {
    if p.is_null() {
        return AtlasCrtMalloc(size);
    }
    if size == 0 {
        crate::heap::free(p);
        return core::ptr::null_mut();
    }
    let have = crate::heap::usable(p);
    if size <= have {
        return p;
    }
    let q = AtlasCrtMalloc(size);
    if !q.is_null() {
        // SAFETY: both blocks are this big at least.
        unsafe { core::ptr::copy_nonoverlapping(p, q, have) };
        crate::heap::free(p);
    }
    q
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMsize(p: *mut u8) -> usize {
    crate::heap::usable(p)
}

/// `_set_new_mode`: whether `malloc` calls the new handler; there's none, so it's only kept.
static NEW_MODE: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetNewMode(mode: i32) -> i32 {
    NEW_MODE.swap(mode as u32, Ordering::Relaxed) as i32
}

// ---- Memory ----

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the program's buffers, `n` bytes each, apart.
    unsafe { core::ptr::copy_nonoverlapping(s, d, n) };
    d
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemmove(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the program's buffers, `n` bytes each.
    unsafe { core::ptr::copy(s, d, n) };
    d
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemset(d: *mut u8, c: i32, n: usize) -> *mut u8 {
    // SAFETY: the program's buffer, `n` bytes.
    unsafe { core::ptr::write_bytes(d, c as u8, n) };
    d
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        // SAFETY: the program's buffers, `n` bytes each.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemchr(p: *const u8, c: i32, n: usize) -> *const u8 {
    for i in 0..n {
        // SAFETY: as above.
        if unsafe { *p.add(i) } == c as u8 {
            // SAFETY: inside the buffer.
            return unsafe { p.add(i) };
        }
    }
    core::ptr::null()
}

/// `memcpy_s(dest, size, src, count)`: 0, or EINVAL or ERANGE (and the destination zeroed) if it doesn't fit.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMemcpyS(d: *mut u8, size: usize, s: *const u8, n: usize) -> i32 {
    if n == 0 {
        return 0;
    }
    if d.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    if s.is_null() || size < n {
        AtlasCrtMemset(d, 0, size);
        let e = if s.is_null() { EINVAL } else { ERANGE };
        set_errno(e);
        return e;
    }
    AtlasCrtMemmove(d, s, n);
    0
}

// ---- Strings, narrow and wide, generic over their unit ----

trait Unit: Copy + Eq + Ord {
    const NUL: Self;
    fn lower(self) -> Self;
}

impl Unit for u8 {
    const NUL: u8 = 0;
    fn lower(self) -> u8 {
        to_lower(i32::from(self)) as u8
    }
}

impl Unit for u16 {
    const NUL: u16 = 0;
    fn lower(self) -> u16 {
        to_lower_wide(self)
    }
}

/// # Safety
///
/// `s` is a NUL-terminated string.
unsafe fn len<T: Unit>(s: *const T) -> usize {
    let mut n = 0;
    // SAFETY: as the caller says.
    while unsafe { *s.add(n) } != T::NUL {
        n += 1;
    }
    n
}

/// # Safety
///
/// Both are NUL-terminated, or `most` long at least.
unsafe fn compare<T: Unit>(a: *const T, b: *const T, most: usize, fold: bool) -> i32 {
    for i in 0..most {
        // SAFETY: as the caller says.
        let (mut x, mut y) = unsafe { (*a.add(i), *b.add(i)) };
        if fold {
            x = x.lower();
            y = y.lower();
        }
        if x != y {
            return if x < y { -1 } else { 1 };
        }
        if x == T::NUL {
            return 0;
        }
    }
    0
}

/// # Safety
///
/// `s` is NUL-terminated.
unsafe fn find<T: Unit>(s: *const T, c: T, last: bool) -> *const T {
    let mut found = core::ptr::null();
    let mut i = 0;
    loop {
        // SAFETY: as the caller says.
        let u = unsafe { *s.add(i) };
        if u == c {
            // SAFETY: inside the string, or its NUL.
            found = unsafe { s.add(i) };
            if !last {
                return found;
            }
        }
        if u == T::NUL {
            return found;
        }
        i += 1;
    }
}

/// # Safety
///
/// Both are NUL-terminated.
unsafe fn find_str<T: Unit>(s: *const T, needle: *const T) -> *const T {
    // SAFETY: as the caller says.
    unsafe {
        let n = len(needle);
        if n == 0 {
            return s;
        }
        let mut at = s;
        while *at != T::NUL {
            if compare(at, needle, n, false) == 0 {
                return at;
            }
            at = at.add(1);
        }
    }
    core::ptr::null()
}

/// # Safety
///
/// `d` has room for `s` and its NUL (or `most` units, with `bounded`).
unsafe fn copy<T: Unit>(d: *mut T, s: *const T, most: usize, bounded: bool) -> *mut T {
    let mut i = 0;
    // SAFETY: as the caller says.
    unsafe {
        while i < most {
            let u = *s.add(i);
            *d.add(i) = u;
            i += 1;
            if u == T::NUL {
                break;
            }
        }
        // strncpy pads with NULs to `most`.
        if bounded {
            while i < most {
                *d.add(i) = T::NUL;
                i += 1;
            }
        }
    }
    d
}

macro_rules! strings {
    ($t:ty, $len:ident, $nlen:ident, $cmp:ident, $ncmp:ident, $icmp:ident, $nicmp:ident, $cpy:ident, $ncpy:ident, $cat:ident, $ncat:ident,
     $chr:ident, $rchr:ident, $str:ident, $dup:ident, $spn:ident, $cspn:ident, $pbrk:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $len(s: *const $t) -> usize {
            // SAFETY: the program's string.
            unsafe { len(s) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $nlen(s: *const $t, most: usize) -> usize {
            let mut n = 0;
            // SAFETY: as above, at most `most` units.
            while n < most && unsafe { *s.add(n) } != 0 {
                n += 1;
            }
            n
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $cmp(a: *const $t, b: *const $t) -> i32 {
            // SAFETY: as above.
            unsafe { compare(a, b, usize::MAX, false) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ncmp(a: *const $t, b: *const $t, n: usize) -> i32 {
            // SAFETY: as above.
            unsafe { compare(a, b, n, false) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $icmp(a: *const $t, b: *const $t) -> i32 {
            // SAFETY: as above.
            unsafe { compare(a, b, usize::MAX, true) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $nicmp(a: *const $t, b: *const $t, n: usize) -> i32 {
            // SAFETY: as above.
            unsafe { compare(a, b, n, true) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $cpy(d: *mut $t, s: *const $t) -> *mut $t {
            // SAFETY: as above.
            unsafe { copy(d, s, usize::MAX, false) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ncpy(d: *mut $t, s: *const $t, n: usize) -> *mut $t {
            // SAFETY: as above.
            unsafe { copy(d, s, n, true) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $cat(d: *mut $t, s: *const $t) -> *mut $t {
            // SAFETY: as above.
            unsafe {
                copy(d.add(len(d)), s, usize::MAX, false);
            }
            d
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $ncat(d: *mut $t, s: *const $t, n: usize) -> *mut $t {
            // SAFETY: as above: at most `n` units, then a NUL.
            unsafe {
                let end = d.add(len(d));
                let mut i = 0;
                while i < n && *s.add(i) != 0 {
                    *end.add(i) = *s.add(i);
                    i += 1;
                }
                *end.add(i) = 0;
            }
            d
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $chr(s: *const $t, c: i32) -> *const $t {
            // SAFETY: as above.
            unsafe { find(s, c as $t, false) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $rchr(s: *const $t, c: i32) -> *const $t {
            // SAFETY: as above.
            unsafe { find(s, c as $t, true) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $str(s: *const $t, needle: *const $t) -> *const $t {
            // SAFETY: as above.
            unsafe { find_str(s, needle) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $dup(s: *const $t) -> *mut $t {
            if s.is_null() {
                return core::ptr::null_mut();
            }
            // SAFETY: as above, into a block as big as it and its NUL.
            unsafe {
                let n = len(s) + 1;
                let d = AtlasCrtMalloc(n * size_of::<$t>()).cast::<$t>();
                if !d.is_null() {
                    core::ptr::copy_nonoverlapping(s, d, n);
                }
                d
            }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $spn(s: *const $t, set: *const $t) -> usize {
            let mut n = 0;
            // SAFETY: as above.
            unsafe {
                while *s.add(n) != 0 && !find(set, *s.add(n), false).is_null() {
                    n += 1;
                }
            }
            n
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $cspn(s: *const $t, set: *const $t) -> usize {
            let mut n = 0;
            // SAFETY: as above.
            unsafe {
                while *s.add(n) != 0 && find(set, *s.add(n), false).is_null() {
                    n += 1;
                }
            }
            n
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $pbrk(s: *const $t, set: *const $t) -> *const $t {
            // SAFETY: as above.
            unsafe {
                let n = $cspn(s, set);
                if *s.add(n) == 0 { core::ptr::null() } else { s.add(n) }
            }
        }
    };
}

strings!(u8, AtlasCrtStrlen, AtlasCrtStrnlen, AtlasCrtStrcmp, AtlasCrtStrncmp, AtlasCrtStricmp, AtlasCrtStrnicmp, AtlasCrtStrcpy, AtlasCrtStrncpy,
    AtlasCrtStrcat, AtlasCrtStrncat, AtlasCrtStrchr, AtlasCrtStrrchr, AtlasCrtStrstr, AtlasCrtStrdup, AtlasCrtStrspn, AtlasCrtStrcspn, AtlasCrtStrpbrk);
strings!(u16, AtlasCrtWcslen, AtlasCrtWcsnlen, AtlasCrtWcscmp, AtlasCrtWcsncmp, AtlasCrtWcsicmp, AtlasCrtWcsnicmp, AtlasCrtWcscpy, AtlasCrtWcsncpy,
    AtlasCrtWcscat, AtlasCrtWcsncat, AtlasCrtWcschr, AtlasCrtWcsrchr, AtlasCrtWcsstr, AtlasCrtWcsdup, AtlasCrtWcsspn, AtlasCrtWcscspn, AtlasCrtWcspbrk);

/// `strcpy_s(dest, size, src)`: 0, or EINVAL or ERANGE with the destination emptied.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrcpyS(d: *mut u8, size: usize, s: *const u8) -> i32 {
    if d.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    if s.is_null() {
        // SAFETY: the program's buffer, `size` bytes.
        unsafe { *d = 0 };
        set_errno(EINVAL);
        return EINVAL;
    }
    // SAFETY: the program's string and buffer.
    unsafe {
        let n = len(s);
        if n >= size {
            *d = 0;
            set_errno(ERANGE);
            return ERANGE;
        }
        core::ptr::copy_nonoverlapping(s, d, n + 1);
    }
    0
}

/// `strcat_s(dest, size, src)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrcatS(d: *mut u8, size: usize, s: *const u8) -> i32 {
    if d.is_null() || s.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    // SAFETY: the program's string and buffer.
    unsafe {
        let have = AtlasCrtStrnlen(d, size);
        if have == size {
            set_errno(EINVAL);
            return EINVAL;
        }
        AtlasCrtStrcpyS(d.add(have), size - have, s)
    }
}

/// `strtok`: one tokenizer for the thread, as `strtok_s`'s context in the TEB's spare room isn't needed by much.
static TOKEN: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrtokS(s: *mut u8, delimiters: *const u8, context: *mut *mut u8) -> *mut u8 {
    // SAFETY: the program's strings and context.
    unsafe {
        let mut at = if s.is_null() { *context } else { s };
        if at.is_null() {
            return core::ptr::null_mut();
        }
        at = at.add(AtlasCrtStrspn(at, delimiters));
        if *at == 0 {
            *context = at;
            return core::ptr::null_mut();
        }
        let token = at;
        at = at.add(AtlasCrtStrcspn(at, delimiters));
        if *at != 0 {
            *at = 0;
            at = at.add(1);
        }
        *context = at;
        token
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrtok(s: *mut u8, delimiters: *const u8) -> *mut u8 {
    let mut context = TOKEN.load(Ordering::Relaxed) as *mut u8;
    // SAFETY: as above.
    let token = unsafe { AtlasCrtStrtokS(s, delimiters, &mut context) };
    TOKEN.store(context as u64, Ordering::Relaxed);
    token
}

// ---- Character classes, in the C locale (and 1252 for the wide ones' Latin-1 half) ----

/// `_pctype`'s bits: upper, lower, digit, space, punctuation, control, blank, hex digit; alpha is upper|lower|0x100.
const UPPER: u16 = 0x1;
const LOWER: u16 = 0x2;
const DIGIT: u16 = 0x4;
const SPACE: u16 = 0x8;
const PUNCT: u16 = 0x10;
const CONTROL: u16 = 0x20;
const BLANK: u16 = 0x40;
const HEX: u16 = 0x80;
const ALPHA: u16 = 0x100;

const fn class(c: u8) -> u16 {
    match c {
        b'A'..=b'F' => UPPER | HEX | ALPHA,
        b'G'..=b'Z' => UPPER | ALPHA,
        b'a'..=b'f' => LOWER | HEX | ALPHA,
        b'g'..=b'z' => LOWER | ALPHA,
        b'0'..=b'9' => DIGIT | HEX,
        b' ' => SPACE | BLANK,
        b'\t' => SPACE | CONTROL | BLANK,
        b'\n' | 0x0b | 0x0c | b'\r' => SPACE | CONTROL,
        0..=0x1f | 0x7f => CONTROL,
        0x21..=0x7e => PUNCT,
        _ => 0,
    }
}

/// The class table `_pctype` points into: 256 entries, after one for EOF.
static CLASSES: [u16; 257] = {
    let mut t = [0u16; 257];
    let mut i = 0;
    while i < 256 {
        t[i + 1] = class(i as u8);
        i += 1;
    }
    t
};

fn class_of(c: i32) -> u16 {
    if (0..256).contains(&c) { CLASSES[c as usize + 1] } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtPctype() -> *const *const u16 {
    static TABLE: AtomicU64 = AtomicU64::new(0);
    TABLE.store(CLASSES.as_ptr() as u64 + 2, Ordering::Relaxed);
    TABLE.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtIsctype(c: i32, mask: i32) -> i32 {
    i32::from(class_of(c) & mask as u16)
}

macro_rules! classes {
    ($($name:ident => $mask:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(c: i32) -> i32 {
            i32::from(class_of(c) & $mask)
        }
    )*};
}

classes! {
    AtlasCrtIsalpha => ALPHA;
    AtlasCrtIsupper => UPPER;
    AtlasCrtIslower => LOWER;
    AtlasCrtIsdigit => DIGIT;
    AtlasCrtIsxdigit => HEX;
    AtlasCrtIsspace => SPACE;
    AtlasCrtIspunct => PUNCT;
    AtlasCrtIscntrl => CONTROL;
    AtlasCrtIsblank => BLANK;
    AtlasCrtIsalnum => ALPHA | DIGIT;
    AtlasCrtIsprint => ALPHA | DIGIT | PUNCT | BLANK;
    AtlasCrtIsgraph => ALPHA | DIGIT | PUNCT;
}

fn to_lower(c: i32) -> i32 {
    if (i32::from(b'A')..=i32::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn to_upper(c: i32) -> i32 {
    if (i32::from(b'a')..=i32::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// Wide case: ASCII and Latin-1's letters (not × and ÷), as most text needs.
fn to_lower_wide(u: u16) -> u16 {
    match u {
        0x41..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde => u + 32,
        _ => u,
    }
}

fn to_upper_wide(u: u16) -> u16 {
    match u {
        0x61..=0x7a | 0xe0..=0xf6 | 0xf8..=0xfe => u - 32,
        0xff => 0x178,
        _ => u,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTolower(c: i32) -> i32 {
    to_lower(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtToupper(c: i32) -> i32 {
    to_upper(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTowlower(c: u32) -> u32 {
    u32::from(to_lower_wide(c as u16))
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTowupper(c: u32) -> u32 {
    u32::from(to_upper_wide(c as u16))
}

/// The wide classes: ASCII's as the narrow ones, Latin-1's letters alpha, the rest by what they are.
fn wide_class(u: u32) -> u16 {
    match u {
        0..0x80 => class(u as u8),
        0xc0..=0xd6 | 0xd8..=0xde => UPPER | ALPHA,
        0xdf..=0xf6 | 0xf8..=0xff => LOWER | ALPHA,
        0xa0 => SPACE | BLANK,
        0x80..0xa0 => CONTROL,
        0x2000..=0x200a | 0x3000 => SPACE | BLANK,
        0xa1..0xc0 | 0xd7 | 0xf7 => PUNCT,
        _ => ALPHA,
    }
}

macro_rules! wide_classes {
    ($($name:ident => $mask:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(c: u32) -> i32 {
            i32::from(wide_class(c) & $mask)
        }
    )*};
}

wide_classes! {
    AtlasCrtIswalpha => ALPHA;
    AtlasCrtIswupper => UPPER;
    AtlasCrtIswlower => LOWER;
    AtlasCrtIswdigit => DIGIT;
    AtlasCrtIswxdigit => HEX;
    AtlasCrtIswspace => SPACE;
    AtlasCrtIswpunct => PUNCT;
    AtlasCrtIswcntrl => CONTROL;
    AtlasCrtIswalnum => ALPHA | DIGIT;
    AtlasCrtIswprint => ALPHA | DIGIT | PUNCT | BLANK;
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtIswctype(c: u32, mask: u16) -> i32 {
    i32::from(wide_class(c) & mask)
}

// ---- Numbers from text ----

/// `strtoull`'s and friends' work: skipping space, a sign, a base's prefix (0x, 0), digits as far as they go. The
/// magnitude, whether it was negative, whether it overflowed 64 bits, and where it stopped (the start, if nothing
/// was read).
///
/// # Safety
///
/// `s` is NUL-terminated.
unsafe fn parse_integer<T: Unit + Into<u32>>(s: *const T, base: i32) -> (u64, bool, bool, *const T) {
    // SAFETY: as the caller says.
    unsafe {
        let at = |i: usize| -> u32 { (*s.add(i)).into() };
        let mut i = 0;
        while at(i) < 0x80 && class(at(i) as u8) & SPACE != 0 {
            i += 1;
        }
        let negative = at(i) == u32::from(b'-');
        if negative || at(i) == u32::from(b'+') {
            i += 1;
        }
        let mut base = base as u32;
        if (base == 0 || base == 16) && at(i) == u32::from(b'0') && (at(i + 1) | 0x20) == u32::from(b'x') && (at(i + 2) as u8 as char).is_ascii_hexdigit() {
            i += 2;
            base = 16;
        } else if base == 0 {
            base = if at(i) == u32::from(b'0') { 8 } else { 10 };
        }
        if !(2..=36).contains(&base) {
            return (0, false, false, s);
        }
        let (mut value, mut overflow, start) = (0u64, false, i);
        loop {
            let c = at(i);
            let digit = match c {
                0x30..=0x39 => c - 0x30,
                0x41..=0x5a => c - 0x41 + 10,
                0x61..=0x7a => c - 0x61 + 10,
                _ => break,
            };
            if digit >= base {
                break;
            }
            match value.checked_mul(u64::from(base)).and_then(|v| v.checked_add(u64::from(digit))) {
                Some(v) => value = v,
                None => overflow = true,
            }
            i += 1;
        }
        if i == start {
            return (0, false, false, s);
        }
        (value, negative, overflow, s.add(i))
    }
}

/// A signed or unsigned result in `bits`, clamped (with ERANGE) as C has it.
fn integer_result(value: u64, negative: bool, overflow: bool, signed: bool, bits: u32) -> u64 {
    let most = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    if signed {
        let limit = most >> 1;
        if overflow || (!negative && value > limit) {
            set_errno(ERANGE);
            return limit;
        }
        if negative && value > limit + 1 {
            set_errno(ERANGE);
            return (limit + 1).wrapping_neg();
        }
        if negative { value.wrapping_neg() } else { value }
    } else {
        if overflow || value > most {
            set_errno(ERANGE);
            return most;
        }
        if negative { value.wrapping_neg() & most } else { value }
    }
}

macro_rules! strto {
    ($($name:ident: $t:ty, $out:ty, $signed:expr, $bits:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(s: *const $t, end: *mut *const $t, base: i32) -> $out {
            // SAFETY: the program's string and end pointer.
            unsafe {
                let (value, negative, overflow, stop) = parse_integer(s, base);
                if !end.is_null() {
                    *end = stop;
                }
                integer_result(value, negative, overflow, $signed, $bits) as $out
            }
        }
    )*};
}

strto! {
    AtlasCrtStrtol: u8, i32, true, 32;
    AtlasCrtStrtoul: u8, u32, false, 32;
    AtlasCrtStrtoll: u8, i64, true, 64;
    AtlasCrtStrtoull: u8, u64, false, 64;
    AtlasCrtWcstol: u16, i32, true, 32;
    AtlasCrtWcstoul: u16, u32, false, 32;
    AtlasCrtWcstoll: u16, i64, true, 64;
    AtlasCrtWcstoull: u16, u64, false, 64;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAtoi(s: *const u8) -> i32 {
    // SAFETY: the program's string.
    unsafe { AtlasCrtStrtol(s, core::ptr::null_mut(), 10) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAtoll(s: *const u8) -> i64 {
    // SAFETY: as above.
    unsafe { AtlasCrtStrtoll(s, core::ptr::null_mut(), 10) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWtoi(s: *const u16) -> i32 {
    // SAFETY: as above.
    unsafe { AtlasCrtWcstol(s, core::ptr::null_mut(), 10) }
}

/// `strtod`: decimal, with a fraction and an exponent, and `inf` and `nan`; correctly rounded for the digits most
/// programs write, by scaling an integer of up to 19 digits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrtod(s: *const u8, end: *mut *const u8) -> f64 {
    // SAFETY: the program's string and end pointer.
    unsafe {
        let at = |i: usize| *s.add(i);
        let mut i = 0;
        while class(at(i)) & SPACE != 0 {
            i += 1;
        }
        let negative = at(i) == b'-';
        if negative || at(i) == b'+' {
            i += 1;
        }
        let word = |i: usize, w: &[u8]| w.iter().enumerate().all(|(k, &c)| at(i + k) | 0x20 == c);
        let sign = |v: f64| if negative { -v } else { v };
        if word(i, b"inf") {
            let n = if word(i, b"infinity") { 8 } else { 3 };
            if !end.is_null() {
                *end = s.add(i + n);
            }
            return sign(f64::INFINITY);
        }
        if word(i, b"nan") {
            if !end.is_null() {
                *end = s.add(i + 3);
            }
            return f64::NAN;
        }
        let (mut mantissa, mut digits, mut scale, mut any) = (0u64, 0u32, 0i32, false);
        while at(i).is_ascii_digit() {
            any = true;
            if digits < 19 {
                mantissa = mantissa * 10 + u64::from(at(i) - b'0');
                if mantissa != 0 {
                    digits += 1;
                }
            } else {
                scale += 1;
            }
            i += 1;
        }
        if at(i) == b'.' {
            i += 1;
            while at(i).is_ascii_digit() {
                any = true;
                if digits < 19 {
                    mantissa = mantissa * 10 + u64::from(at(i) - b'0');
                    if mantissa != 0 {
                        digits += 1;
                    }
                    scale -= 1;
                }
                i += 1;
            }
        }
        if !any {
            if !end.is_null() {
                *end = s;
            }
            return 0.0;
        }
        if at(i) | 0x20 == b'e' {
            let mut j = i + 1;
            let exp_negative = at(j) == b'-';
            if exp_negative || at(j) == b'+' {
                j += 1;
            }
            if at(j).is_ascii_digit() {
                let mut e = 0i32;
                while at(j).is_ascii_digit() {
                    e = (e * 10 + i32::from(at(j) - b'0')).min(100_000);
                    j += 1;
                }
                scale += if exp_negative { -e } else { e };
                i = j;
            }
        }
        if !end.is_null() {
            *end = s.add(i);
        }
        let value = scale_by_ten(mantissa as f64, scale);
        if value.is_infinite() {
            set_errno(ERANGE);
        }
        sign(value)
    }
}

/// `x` times ten to the `e`, in as few roundings as it can.
fn scale_by_ten(x: f64, e: i32) -> f64 {
    const POWERS: [f64; 23] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22];
    let mut x = x;
    let mut e = e;
    while e > 22 {
        x *= 1e22;
        e -= 22;
    }
    while e < -22 {
        x /= 1e22;
        e += 22;
    }
    if e >= 0 { x * POWERS[e as usize] } else { x / POWERS[(-e) as usize] }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAtof(s: *const u8) -> f64 {
    // SAFETY: the program's string.
    unsafe { AtlasCrtStrtod(s, core::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtAbs(v: i32) -> i32 {
    v.wrapping_abs()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLlabs(v: i64) -> i64 {
    v.wrapping_abs()
}

// ---- Sorting, searching, random numbers ----

type Compare = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
type CompareS = unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void) -> i32;

/// # Safety
///
/// Both point at `size` bytes, apart.
unsafe fn swap(a: *mut u8, b: *mut u8, size: usize) {
    // SAFETY: as the caller says.
    unsafe { core::ptr::swap_nonoverlapping(a, b, size) };
}

/// A heap sort: in place, with no allocation, `n log n` in the worst case.
///
/// # Safety
///
/// `base` has `count` elements of `size` bytes, and `less` compares two of them.
unsafe fn heap_sort(base: *mut u8, count: usize, size: usize, mut cmp: impl FnMut(*const u8, *const u8) -> i32) {
    if count < 2 || size == 0 {
        return;
    }
    let at = |i: usize| unsafe { base.add(i * size) };
    let sift = |cmp: &mut dyn FnMut(*const u8, *const u8) -> i32, mut root: usize, end: usize| loop {
        let mut child = 2 * root + 1;
        if child >= end {
            break;
        }
        if child + 1 < end && cmp(at(child), at(child + 1)) < 0 {
            child += 1;
        }
        if cmp(at(root), at(child)) >= 0 {
            break;
        }
        // SAFETY: two elements of the array.
        unsafe { swap(at(root), at(child), size) };
        root = child;
    };
    for root in (0..count / 2).rev() {
        sift(&mut cmp, root, count);
    }
    for end in (1..count).rev() {
        // SAFETY: as above.
        unsafe { swap(at(0), at(end), size) };
        sift(&mut cmp, 0, end);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtQsort(base: *mut u8, count: usize, size: usize, compare: Compare) {
    // SAFETY: the program's array and its comparison.
    unsafe { heap_sort(base, count, size, |a, b| compare(a.cast(), b.cast())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtQsortS(base: *mut u8, count: usize, size: usize, compare: CompareS, context: *mut c_void) {
    // SAFETY: as above.
    unsafe { heap_sort(base, count, size, |a, b| compare(context, a.cast(), b.cast())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtBsearch(key: *const c_void, base: *const u8, count: usize, size: usize, compare: Compare) -> *const u8 {
    let (mut lo, mut hi) = (0, count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        // SAFETY: an element of the program's sorted array.
        let at = unsafe { base.add(mid * size) };
        // SAFETY: the program's comparison.
        match unsafe { compare(key, at.cast()) } {
            0 => return at,
            c if c < 0 => hi = mid,
            _ => lo = mid + 1,
        }
    }
    core::ptr::null()
}

static SEED: AtomicU32 = AtomicU32::new(1);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSrand(seed: u32) {
    SEED.store(seed, Ordering::Relaxed);
}

/// `rand`: the C runtime's own generator, so a seed gives the sequence Windows' does.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtRand() -> i32 {
    let next = SEED.load(Ordering::Relaxed).wrapping_mul(214_013).wrapping_add(2_531_011);
    SEED.store(next, Ordering::Relaxed);
    ((next >> 16) & 0x7fff) as i32
}

// ---- Startup: arguments and environment ----

static ARGC: AtomicU32 = AtomicU32::new(0);
static ARGV: AtomicU64 = AtomicU64::new(0);
static ENVIRON: AtomicU64 = AtomicU64::new(0);
static WARGV: AtomicU64 = AtomicU64::new(0);
static WENVIRON: AtomicU64 = AtomicU64::new(0);
static ACMDLN: AtomicU64 = AtomicU64::new(0);

/// A command line split as the C runtime splits it: the program's name to its closing quote or the first space; then
/// each argument to unquoted white space, where 2n backslashes and a quote are n backslashes and a quote that
/// starts or ends quoting, 2n+1 and a quote are n and a literal quote, `""` in quotes is a literal quote, and other
/// backslashes are themselves. Each argument goes to `emit`, as units.
fn split_command_line<T: Unit + Into<u32>>(line: &[T], mut emit: impl FnMut(&[T])) {
    let is = |u: T, c: u8| u.into() == u32::from(c);
    let mut i = 0;
    let mut arg: [T; 1024] = [T::NUL; 1024];
    let mut n = 0;
    // The program's name.
    if i < line.len() && is(line[i], b'"') {
        i += 1;
        while i < line.len() && !is(line[i], b'"') {
            if n < 1023 {
                arg[n] = line[i];
                n += 1;
            }
            i += 1;
        }
        i += 1;
    } else {
        while i < line.len() && !is(line[i], b' ') && !is(line[i], b'\t') {
            if n < 1023 {
                arg[n] = line[i];
                n += 1;
            }
            i += 1;
        }
    }
    emit(&arg[..n]);
    loop {
        while i < line.len() && (is(line[i], b' ') || is(line[i], b'\t')) {
            i += 1;
        }
        if i >= line.len() {
            return;
        }
        n = 0;
        let mut quoted = false;
        while i < line.len() {
            let u = line[i];
            if is(u, b'\\') {
                let mut slashes = 0;
                while i < line.len() && is(line[i], b'\\') {
                    slashes += 1;
                    i += 1;
                }
                let before_quote = i < line.len() && is(line[i], b'"');
                let keep = if before_quote { slashes / 2 } else { slashes };
                for _ in 0..keep {
                    if n < 1023 {
                        arg[n] = u;
                        n += 1;
                    }
                }
                if before_quote && slashes % 2 == 1 {
                    if n < 1023 {
                        arg[n] = line[i];
                        n += 1;
                    }
                    i += 1;
                }
                continue;
            }
            if is(u, b'"') {
                if quoted && i + 1 < line.len() && is(line[i + 1], b'"') {
                    if n < 1023 {
                        arg[n] = u;
                        n += 1;
                    }
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            if !quoted && (is(u, b' ') || is(u, b'\t')) {
                break;
            }
            if n < 1023 {
                arg[n] = u;
                n += 1;
            }
            i += 1;
        }
        emit(&arg[..n]);
    }
}

/// An array of NUL-terminated strings on the heap, built from `fill`'s, with a null after the last.
fn string_array<T: Unit>(mut fill: impl FnMut(&mut dyn FnMut(&[T]))) -> (*mut *mut T, usize) {
    let mut count = 0;
    let mut units = 0;
    fill(&mut |s: &[T]| {
        count += 1;
        units += s.len() + 1;
    });
    let table = AtlasCrtMalloc((count + 1) * 8).cast::<*mut T>();
    let strings = AtlasCrtMalloc(units.max(1) * size_of::<T>()).cast::<T>();
    if table.is_null() || strings.is_null() {
        return (core::ptr::null_mut(), 0);
    }
    let (mut i, mut at) = (0, 0);
    fill(&mut |s: &[T]| {
        // SAFETY: room for every string and its NUL, and every pointer and the null.
        unsafe {
            let dst = strings.add(at);
            core::ptr::copy_nonoverlapping(s.as_ptr(), dst, s.len());
            *dst.add(s.len()) = T::NUL;
            *table.add(i) = dst;
        }
        i += 1;
        at += s.len() + 1;
    });
    // SAFETY: as above.
    unsafe { *table.add(i) = core::ptr::null_mut() };
    (table, count)
}

fn narrow_command_line() -> &'static [u8] {
    let line = get_command_line_a() as *const u8;
    if line.is_null() {
        return &[];
    }
    // SAFETY: the process's ANSI command line, which lasts as long as it does.
    unsafe { core::slice::from_raw_parts(line, len(line)) }
}

fn wide_command_line() -> &'static [u16] {
    // SAFETY: the PEB's process parameters' command line, a UNICODE_STRING.
    unsafe {
        let params = *(crate::peb().add(atlas_abi::peb::PROCESS_PARAMETERS as usize) as *const u64);
        let string = params + atlas_abi::params::COMMAND_LINE;
        let len = usize::from(*(string as *const u16)) / 2;
        let buffer = *((string + 8) as *const *const u16);
        if buffer.is_null() { &[] } else { core::slice::from_raw_parts(buffer, len) }
    }
}

/// `_configure_narrow_argv`: `__argc` and `__argv`, from the command line.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtConfigureNarrowArgv(_mode: i32) -> i32 {
    if ARGV.load(Ordering::Acquire) == 0 {
        let line = narrow_command_line();
        let (argv, argc) = string_array::<u8>(|emit| split_command_line(line, &mut *emit));
        ARGC.store(argc as u32, Ordering::Release);
        ARGV.store(argv as u64, Ordering::Release);
    }
    0
}

/// `_configure_wide_argv`: `__argc` and `__wargv`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtConfigureWideArgv(_mode: i32) -> i32 {
    if WARGV.load(Ordering::Acquire) == 0 {
        let line = wide_command_line();
        let (argv, argc) = string_array::<u16>(|emit| split_command_line(line, &mut *emit));
        ARGC.store(argc as u32, Ordering::Release);
        WARGV.store(argv as u64, Ordering::Release);
    }
    0
}

/// `_initialize_narrow_environment`: `_environ`, from the process's environment block.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtInitializeNarrowEnvironment() -> i32 {
    if ENVIRON.load(Ordering::Acquire) == 0 {
        let block = crate::env::AtlasGetEnvironmentStringsA();
        let (environ, _) = string_array::<u8>(|emit| {
            let mut at = block;
            // SAFETY: the block's strings, each NUL-terminated, then a NUL.
            unsafe {
                while !at.is_null() && *at != 0 {
                    let n = len(at);
                    if *at != b'=' {
                        emit(core::slice::from_raw_parts(at, n));
                    }
                    at = at.add(n + 1);
                }
            }
        });
        ENVIRON.store(environ as u64, Ordering::Release);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtInitializeWideEnvironment() -> i32 {
    if WENVIRON.load(Ordering::Acquire) == 0 {
        let block = crate::env::AtlasGetEnvironmentStringsW();
        let (environ, _) = string_array::<u16>(|emit| {
            let mut at = block;
            // SAFETY: as above, in UTF-16.
            unsafe {
                while !at.is_null() && *at != 0 {
                    let n = len(at);
                    if *at != u16::from(b'=') {
                        emit(core::slice::from_raw_parts(at, n));
                    }
                    at = at.add(n + 1);
                }
            }
        });
        WENVIRON.store(environ as u64, Ordering::Release);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtArgc() -> *mut i32 {
    AtlasCrtConfigureNarrowArgv(1);
    ARGC.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtArgv() -> *mut *mut *mut u8 {
    AtlasCrtConfigureNarrowArgv(1);
    ARGV.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWargv() -> *mut *mut *mut u16 {
    AtlasCrtConfigureWideArgv(1);
    WARGV.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtEnviron() -> *mut *mut *mut u8 {
    AtlasCrtInitializeNarrowEnvironment();
    ENVIRON.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWenviron() -> *mut *mut *mut u16 {
    AtlasCrtInitializeWideEnvironment();
    WENVIRON.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetInitialNarrowEnvironment() -> *mut *mut u8 {
    AtlasCrtInitializeNarrowEnvironment();
    ENVIRON.load(Ordering::Acquire) as *mut *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetInitialWideEnvironment() -> *mut *mut u16 {
    AtlasCrtInitializeWideEnvironment();
    WENVIRON.load(Ordering::Acquire) as *mut *mut u16
}

/// `_get_narrow_winmain_command_line`: the command line past the program's name, as `WinMain` gets it.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtNarrowWinmainCommandLine() -> *const u8 {
    if ACMDLN.load(Ordering::Acquire) == 0 {
        let line = narrow_command_line();
        let mut i = 0;
        let mut quoted = false;
        while i < line.len() && (quoted || (line[i] != b' ' && line[i] != b'\t')) {
            if line[i] == b'"' {
                quoted = !quoted;
            }
            i += 1;
        }
        while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
            i += 1;
        }
        // SAFETY: inside the command line, or its end.
        ACMDLN.store(unsafe { line.as_ptr().add(i) } as u64, Ordering::Release);
    }
    ACMDLN.load(Ordering::Acquire) as *const u8
}

/// `getenv`: in `_environ`, by name, folding case.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtGetenv(name: *const u8) -> *const u8 {
    AtlasCrtInitializeNarrowEnvironment();
    let table = ENVIRON.load(Ordering::Acquire) as *const *const u8;
    if table.is_null() || name.is_null() {
        return core::ptr::null();
    }
    // SAFETY: the table and its strings, and the program's name.
    unsafe {
        let n = len(name);
        let mut i = 0;
        while !(*table.add(i)).is_null() {
            let entry = *table.add(i);
            if compare(entry, name, n, true) == 0 && *entry.add(n) == b'=' {
                return entry.add(n + 1);
            }
            i += 1;
        }
    }
    core::ptr::null()
}

// ---- Startup: initializers, and what runs at exit ----

type Init = unsafe extern "C" fn();
type InitE = unsafe extern "C" fn() -> i32;

/// `_initterm(first, last)`: each initializer between, in order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtInitterm(first: *const Option<Init>, last: *const Option<Init>) {
    let mut at = first;
    while at < last {
        // SAFETY: the program's table of initializers.
        unsafe {
            if let Some(f) = *at {
                f();
            }
            at = at.add(1);
        }
    }
}

/// `_initterm_e`: the same, stopping at the first that fails, with its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtInittermE(first: *const Option<InitE>, last: *const Option<InitE>) -> i32 {
    let mut at = first;
    while at < last {
        // SAFETY: as above.
        unsafe {
            if let Some(f) = *at {
                let r = f();
                if r != 0 {
                    return r;
                }
            }
            at = at.add(1);
        }
    }
    0
}

/// `_onexit_table_t`: first, last, end, as the program's own CRT keeps one.
#[repr(C)]
pub struct OnexitTable {
    first: *mut Option<Init>,
    last: *mut Option<Init>,
    end: *mut Option<Init>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtInitializeOnexitTable(table: *mut OnexitTable) -> i32 {
    if table.is_null() {
        return -1;
    }
    // SAFETY: the program's table.
    unsafe { table.write(OnexitTable { first: core::ptr::null_mut(), last: core::ptr::null_mut(), end: core::ptr::null_mut() }) };
    0
}

/// `_register_onexit_function(table, function)`: run when the table is.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtRegisterOnexitFunction(table: *mut OnexitTable, function: Option<Init>) -> i32 {
    if table.is_null() {
        return -1;
    }
    with_exit_lock(|| {
        // SAFETY: the program's table, which only these functions change.
        unsafe {
            let t = &mut *table;
            if t.last == t.end {
                let have = if t.first.is_null() { 0 } else { t.end.offset_from(t.first) as usize };
                let room = (have * 2).max(32);
                let grown = AtlasCrtRealloc(t.first.cast(), room * 8).cast::<Option<Init>>();
                if grown.is_null() {
                    return -1;
                }
                let used = if t.first.is_null() { 0 } else { t.last.offset_from(t.first) as usize };
                t.first = grown;
                t.last = grown.add(used);
                t.end = grown.add(room);
            }
            *t.last = function;
            t.last = t.last.add(1);
        }
        0
    })
}

/// `_execute_onexit_table`: every function in it, last first, and the table empty after.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtExecuteOnexitTable(table: *mut OnexitTable) -> i32 {
    if table.is_null() {
        return -1;
    }
    loop {
        // One at a time, as a function may register another.
        let next = with_exit_lock(|| {
            // SAFETY: as above.
            unsafe {
                let t = &mut *table;
                if t.last == t.first || t.first.is_null() {
                    return None;
                }
                t.last = t.last.sub(1);
                Some(*t.last)
            }
        });
        match next {
            None => break,
            // SAFETY: the program's function.
            Some(Some(f)) => unsafe { f() },
            Some(None) => {}
        }
    }
    0
}

static EXIT_LOCK: AtomicBool = AtomicBool::new(false);

fn with_exit_lock<R>(f: impl FnOnce() -> R) -> R {
    while EXIT_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    let r = f();
    EXIT_LOCK.store(false, Ordering::Release);
    r
}

/// The process's table: `atexit` and `_crt_atexit` register into it, and `exit` runs it.
static mut PROCESS_TABLE: OnexitTable = OnexitTable { first: core::ptr::null_mut(), last: core::ptr::null_mut(), end: core::ptr::null_mut() };

fn process_table() -> *mut OnexitTable {
    &raw mut PROCESS_TABLE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAtexit(function: Option<Init>) -> i32 {
    // SAFETY: the process's table.
    if unsafe { AtlasCrtRegisterOnexitFunction(process_table(), function) } == 0 { 0 } else { -1 }
}

/// `_onexit`: as `atexit`, giving the function back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtOnexit(function: Option<Init>) -> Option<Init> {
    // SAFETY: as above.
    if unsafe { AtlasCrtAtexit(function) } == 0 { function } else { None }
}

/// What every way of ending runs first: the process's exit functions, then the streams flushed.
fn run_exit_functions() {
    // SAFETY: the process's table.
    unsafe { AtlasCrtExecuteOnexitTable(process_table()) };
    crate::stdio::flush_all();
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtExit(code: i32) -> ! {
    run_exit_functions();
    crate::loader::RtlExitUserProcess(code as u32)
}

/// `_exit`, `_Exit`, `quick_exit`: no exit functions.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtUnderscoreExit(code: i32) -> ! {
    crate::loader::RtlExitUserProcess(code as u32)
}

/// `_cexit`: the exit functions, without exiting.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtCexit() {
    run_exit_functions();
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtCExitNothing() {}

// ---- Signals, abort, and what's only kept ----

const SIGABRT: i32 = 22;
const SIGNALS: usize = 23;
static HANDLERS: [AtomicU64; SIGNALS] = [const { AtomicU64::new(0) }; SIGNALS];

/// `signal(sig, handler)`: the one before, or SIG_ERR (-1) for a signal there isn't.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSignal(sig: i32, handler: u64) -> u64 {
    match HANDLERS.get(sig as usize) {
        Some(slot) if sig > 0 => slot.swap(handler, Ordering::AcqRel),
        _ => {
            set_errno(EINVAL);
            u64::MAX
        }
    }
}

/// `raise(sig)`: its handler (reset to the default first, as C has it); with none, SIGABRT ends the process with 3,
/// and the rest are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtRaise(sig: i32) -> i32 {
    let Some(slot) = HANDLERS.get(sig as usize) else {
        set_errno(EINVAL);
        return -1;
    };
    match slot.swap(0, Ordering::AcqRel) {
        0 if sig == SIGABRT => AtlasCrtUnderscoreExit(3),
        0 | 1 => 0,
        handler => {
            // SAFETY: the program's handler.
            let f: extern "C" fn(i32) = unsafe { core::mem::transmute(handler) };
            f(sig);
            0
        }
    }
}

/// `abort`: SIGABRT, then the process ends with 3.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtAbort() -> ! {
    crate::stdio::write_err(b"\nabort() has been called\n");
    AtlasCrtRaise(SIGABRT);
    AtlasCrtUnderscoreExit(3)
}

/// `_invalid_parameter_noinfo(_noreturn)`: the handler `_set_invalid_parameter_handler` gave, if any, else the
/// process ends with STATUS_INVALID_CRUNTIME_PARAMETER.
static INVALID_PARAMETER: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetInvalidParameterHandler(handler: u64) -> u64 {
    INVALID_PARAMETER.swap(handler, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtInvalidParameterNoinfo() {
    let handler = INVALID_PARAMETER.load(Ordering::Acquire);
    if handler != 0 {
        // SAFETY: the program's handler: (expression, function, file, line, reserved), all null.
        let f: extern "C" fn(u64, u64, u64, u32, u64) = unsafe { core::mem::transmute(handler) };
        f(0, 0, 0, 0, 0);
        return;
    }
    crate::sys::terminate(0xc000_0417)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtInvalidParameterNoinfoNoreturn() -> ! {
    AtlasCrtInvalidParameterNoinfo();
    crate::sys::terminate(0xc000_0417)
}

static APP_TYPE: AtomicU32 = AtomicU32::new(0);
static FMODE: AtomicU32 = AtomicU32::new(0x4000);
static COMMODE: AtomicU32 = AtomicU32::new(0);
static THREAD_LOCALE: AtomicU32 = AtomicU32::new(2);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetAppType(kind: i32) {
    APP_TYPE.store(kind as u32, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFmode() -> *mut i32 {
    FMODE.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtCommode() -> *mut i32 {
    COMMODE.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetFmode(mode: i32) -> i32 {
    FMODE.store(mode as u32, Ordering::Relaxed);
    0
}

/// `_configthreadlocale`: the mode before; -1 asks without changing it.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtConfigThreadLocale(mode: i32) -> i32 {
    if mode <= 0 { THREAD_LOCALE.load(Ordering::Relaxed) as i32 } else { THREAD_LOCALE.swap(mode as u32, Ordering::Relaxed) as i32 }
}

/// `__setusermatherr`, `_set_abort_behavior` and the like: taken, and nothing to do.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtIgnore() -> i32 {
    0
}

/// `setlocale`: the C locale is the one there is: "C" for any query or for "C" and "", else null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtSetlocale(_category: i32, locale: *const u8) -> *const u8 {
    // SAFETY: the program's string, if it gives one.
    if locale.is_null() || unsafe { *locale == 0 || (*locale == b'C' && *locale.add(1) == 0) } { c"C".as_ptr().cast() } else { core::ptr::null() }
}

/// `___lc_codepage_func`: the locale's code page, the ANSI one.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLcCodepage() -> u32 {
    1252
}

/// `___mb_cur_max_func`: the C locale's.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtMbCurMax() -> i32 {
    1
}

/// `strerror`: the C runtime's message for an `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtStrerror(e: i32) -> *const u8 {
    let s: &'static core::ffi::CStr = match e {
        0 => c"No error",
        1 => c"Operation not permitted",
        2 => c"No such file or directory",
        5 => c"Input/output error",
        9 => c"Bad file descriptor",
        12 => c"Not enough space",
        13 => c"Permission denied",
        17 => c"File exists",
        22 => c"Invalid argument",
        24 => c"Too many open files",
        28 => c"No space left on device",
        33 => c"Domain error",
        34 => c"Result too large",
        _ => c"Unknown error",
    };
    s.as_ptr().cast()
}

/// `_time64(&t)`: seconds since 1970, from the system time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtTime64(out: *mut i64) -> i64 {
    let mut ft = 0u64;
    get_system_time_as_file_time((&raw mut ft) as u64);
    let t = (ft / 10_000_000) as i64 - 11_644_473_600;
    if !out.is_null() {
        // SAFETY: the program's out pointer.
        unsafe { out.write_unaligned(t) };
    }
    t
}

/// `clock`: milliseconds (CLOCKS_PER_SEC is 1000) since the process started, by the interrupt time.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtClock() -> i32 {
    static START: AtomicU64 = AtomicU64::new(0);
    // SAFETY: KUSER_SHARED_DATA's interrupt time, its low and high parts.
    let now = unsafe { u64::from(core::ptr::read_volatile(0x7ffe_0008 as *const u32)) | u64::from(core::ptr::read_volatile(0x7ffe_000c as *const u32)) << 32 };
    let start = match START.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => now,
        Err(s) => s,
    };
    ((now - start) / 10_000) as i32
}

/// A narrow character as UTF-16, for `btowc` and `mbtowc`: 1252.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtBtowc(c: i32) -> u32 {
    if c < 0 { 0xffff } else { u32::from(from_1252(c as u8)) }
}

/// `wctob`: a UTF-16 character as 1252, or EOF.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWctob(c: u32) -> i32 {
    u16::try_from(c).ok().and_then(to_1252).map_or(-1, i32::from)
}
