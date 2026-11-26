//! Extended UCRT exports: locale (`*_l`, `localeconv`, multibyte/wide conversions, `_Getdays`/`_Getmonths`),
//! aligned allocations, time (`_gmtime64`, `_localtime64`, `_mktime64`, `strftime`), threading (`_beginthreadex`),
//! filesystem (`_stat64`, `_findfirst64i32`, `_chdir`, `_getcwd`, `_fullpath`, `remove`, `rename`), extra string/number
//! conversions, and `setjmp`/`longjmp`.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

use atlas_abi::ansi::{from_1252, to_1252};

use crate::crt::{
    self, AtlasCrtAbort, AtlasCrtFree, AtlasCrtGetenv, AtlasCrtIswalpha, AtlasCrtIswcntrl, AtlasCrtIswdigit,
    AtlasCrtIswlower, AtlasCrtIswprint, AtlasCrtIswpunct, AtlasCrtIswspace, AtlasCrtIswupper,
    AtlasCrtIswxdigit, AtlasCrtMalloc, AtlasCrtStrchr, AtlasCrtStrcmp, AtlasCrtStrerror, AtlasCrtStrlen,
    AtlasCrtStrncmp, AtlasCrtStrncpy, AtlasCrtStrnlen, AtlasCrtStrpbrk, AtlasCrtStrtod, AtlasCrtTime64,
    AtlasCrtTolower, AtlasCrtToupper, AtlasCrtTowlower, AtlasCrtTowupper, AtlasCrtWcscmp, AtlasCrtWcslen,
    AtlasCrtWcstoll, set_errno,
};
use crate::sys::host;

host!(create_thread, b"CreateThread", (sec, stack, start, param, flags, tid));
host!(get_current_process_id, b"GetCurrentProcessId", ());
host!(get_command_line_w, b"GetCommandLineW", ());
host!(get_file_attributes_ex_a, b"GetFileAttributesExA", (name, level, info));
host!(get_file_attributes_ex_w, b"GetFileAttributesExW", (name, level, info));
host!(find_first_file_a, b"FindFirstFileA", (name, data));
host!(find_next_file_a, b"FindNextFileA", (handle, data));
host!(find_close, b"FindClose", (handle));
host!(set_current_directory_a, b"SetCurrentDirectoryA", (path));
host!(set_current_directory_w, b"SetCurrentDirectoryW", (path));
host!(get_current_directory_a, b"GetCurrentDirectoryA", (len, buf));
host!(get_current_directory_w, b"GetCurrentDirectoryW", (len, buf));
host!(get_full_path_name_a, b"GetFullPathNameA", (name, len, buf, part));
host!(delete_file_a, b"DeleteFileA", (name));
host!(delete_file_w, b"DeleteFileW", (name));
host!(move_file_ex_a, b"MoveFileExA", (from, to, flags));
host!(move_file_ex_w, b"MoveFileExW", (from, to, flags));
host!(remove_directory_a, b"RemoveDirectoryA", (name));
host!(remove_directory_w, b"RemoveDirectoryW", (name));

const ENOENT: i32 = 2;
const EACCES: i32 = 13;
const EEXIST: i32 = 17;
const EINVAL: i32 = 22;
const ERANGE: i32 = 34;
const EILSEQ: i32 = 42;
const ENOMEM: i32 = 12;

pub(crate) fn map_win32_errno() -> i32 {
    match crate::RtlGetLastWin32Error() {
        2 | 3 => ENOENT,
        5 => EACCES,
        80 | 183 => EEXIST,
        87 => EINVAL,
        _ => EACCES,
    }
}

// ---- Locale & *_l helpers ----

/// Minimal `_locale_t` object (struct `__crt_locale_pointers`: `{ locinfo, mbcinfo }`).
#[repr(C)]
struct LocaleData {
    locinfo: *const c_void,
    mbcinfo: *const c_void,
}

unsafe impl Sync for LocaleData {}

static C_LOCINFO: [usize; 16] = [0; 16];
static C_MBCINFO: [usize; 16] = [0; 16];
static C_LOCALE: LocaleData = LocaleData {
    locinfo: C_LOCINFO.as_ptr().cast(),
    mbcinfo: C_MBCINFO.as_ptr().cast(),
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtCreateLocale(_category: i32, locale: *const u8) -> *const c_void {
    if locale.is_null() {
        return (&raw const C_LOCALE).cast();
    }
    // Accept "C", "POSIX", "", and UTF-8 / English names as the C locale.
    (&raw const C_LOCALE).cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFreeLocale(_locale: *const c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLockLocales() {}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtUnlockLocales() {}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLcCollateCp() -> u32 {
    0
}

static LOCALE_NAMES: [usize; 6] = [0; 6];

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLcLocaleName() -> *const *const u16 {
    LOCALE_NAMES.as_ptr().cast()
}

/// Windows UCRT `struct lconv` for the C locale.
#[repr(C)]
struct Lconv {
    decimal_point: *const u8,
    thousands_sep: *const u8,
    grouping: *const u8,
    int_curr_symbol: *const u8,
    currency_symbol: *const u8,
    mon_decimal_point: *const u8,
    mon_thousands_sep: *const u8,
    mon_grouping: *const u8,
    positive_sign: *const u8,
    negative_sign: *const u8,
    int_frac_digits: i8,
    frac_digits: i8,
    p_cs_precedes: i8,
    p_sep_by_space: i8,
    n_cs_precedes: i8,
    n_sep_by_space: i8,
    p_sign_posn: i8,
    n_sign_posn: i8,
    w_decimal_point: *const u16,
    w_thousands_sep: *const u16,
    w_int_curr_symbol: *const u16,
    w_currency_symbol: *const u16,
    w_mon_decimal_point: *const u16,
    w_mon_thousands_sep: *const u16,
    w_mon_grouping: *const u16,
    w_positive_sign: *const u16,
    w_negative_sign: *const u16,
}

unsafe impl Sync for Lconv {}

static W_DOT: [u16; 2] = [b'.' as u16, 0];
static W_EMPTY: [u16; 1] = [0];

static C_LCONV: Lconv = Lconv {
    decimal_point: c".".as_ptr().cast(),
    thousands_sep: c"".as_ptr().cast(),
    grouping: c"".as_ptr().cast(),
    int_curr_symbol: c"".as_ptr().cast(),
    currency_symbol: c"".as_ptr().cast(),
    mon_decimal_point: c"".as_ptr().cast(),
    mon_thousands_sep: c"".as_ptr().cast(),
    mon_grouping: c"".as_ptr().cast(),
    positive_sign: c"".as_ptr().cast(),
    negative_sign: c"".as_ptr().cast(),
    int_frac_digits: 127,
    frac_digits: 127,
    p_cs_precedes: 127,
    p_sep_by_space: 127,
    n_cs_precedes: 127,
    n_sep_by_space: 127,
    p_sign_posn: 127,
    n_sign_posn: 127,
    w_decimal_point: W_DOT.as_ptr(),
    w_thousands_sep: W_EMPTY.as_ptr(),
    w_int_curr_symbol: W_EMPTY.as_ptr(),
    w_currency_symbol: W_EMPTY.as_ptr(),
    w_mon_decimal_point: W_EMPTY.as_ptr(),
    w_mon_thousands_sep: W_EMPTY.as_ptr(),
    w_mon_grouping: W_EMPTY.as_ptr(),
    w_positive_sign: W_EMPTY.as_ptr(),
    w_negative_sign: W_EMPTY.as_ptr(),
};

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLocaleconv() -> *const c_void {
    (&raw const C_LCONV).cast()
}

macro_rules! isw_l {
    ($($name:ident => $inner:ident;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(c: u32, _loc: usize) -> i32 {
            $inner(c)
        }
    )*};
}

isw_l! {
    AtlasCrtIswalphaL => AtlasCrtIswalpha;
    AtlasCrtIswcntrlL => AtlasCrtIswcntrl;
    AtlasCrtIswdigitL => AtlasCrtIswdigit;
    AtlasCrtIswlowerL => AtlasCrtIswlower;
    AtlasCrtIswprintL => AtlasCrtIswprint;
    AtlasCrtIswpunctL => AtlasCrtIswpunct;
    AtlasCrtIswspaceL => AtlasCrtIswspace;
    AtlasCrtIswupperL => AtlasCrtIswupper;
    AtlasCrtIswxdigitL => AtlasCrtIswxdigit;
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTolowerL(c: i32, _loc: usize) -> i32 {
    AtlasCrtTolower(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtToupperL(c: i32, _loc: usize) -> i32 {
    AtlasCrtToupper(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTowlowerL(c: u32, _loc: usize) -> u32 {
    AtlasCrtTowlower(c)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTowupperL(c: u32, _loc: usize) -> u32 {
    AtlasCrtTowupper(c)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrcollL(a: *const u8, b: *const u8, _loc: usize) -> i32 {
    unsafe { AtlasCrtStrcmp(a, b) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcscollL(a: *const u16, b: *const u16, _loc: usize) -> i32 {
    unsafe { AtlasCrtWcscmp(a, b) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrxfrmL(dst: *mut u8, src: *const u8, count: usize, _loc: usize) -> usize {
    if src.is_null() {
        return 0;
    }
    let n = unsafe { AtlasCrtStrlen(src) };
    if !dst.is_null() && count > 0 {
        let copy_n = n.min(count - 1);
        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, copy_n);
            *dst.add(copy_n) = 0;
        }
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcsxfrmL(dst: *mut u16, src: *const u16, count: usize, _loc: usize) -> usize {
    if src.is_null() {
        return 0;
    }
    let n = unsafe { AtlasCrtWcslen(src) };
    if !dst.is_null() && count > 0 {
        let copy_n = n.min(count - 1);
        unsafe {
            core::ptr::copy_nonoverlapping(src, dst, copy_n);
            *dst.add(copy_n) = 0;
        }
    }
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrtodL(s: *const u8, end: *mut *const u8, _loc: usize) -> f64 {
    unsafe { AtlasCrtStrtod(s, end) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrtof(s: *const u8, end: *mut *const u8) -> f32 {
    unsafe { AtlasCrtStrtod(s, end) as f32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcstod(s: *const u16, end: *mut *const u16) -> f64 {
    if s.is_null() {
        return 0.0;
    }
    // Convert ASCII prefix into a stack buffer and parse with AtlasCrtStrtod.
    let mut buf = [0u8; 256];
    let mut n = 0usize;
    unsafe {
        while n + 1 < buf.len() {
            let u = *s.add(n);
            if u == 0 || u >= 0x80 {
                break;
            }
            buf[n] = u as u8;
            n += 1;
        }
        buf[n] = 0;
        let mut narrow_end = core::ptr::null();
        let v = AtlasCrtStrtod(buf.as_ptr(), &mut narrow_end);
        if !end.is_null() {
            let consumed = if narrow_end.is_null() { 0 } else { narrow_end.offset_from(buf.as_ptr()) as usize };
            *end = s.add(consumed);
        }
        v
    }
}

/// Helper: allocate a heap copy of a byte slice + NUL (callers free with `free`).
fn dup_bytes(s: &[u8]) -> *mut u8 {
    let p = AtlasCrtMalloc(s.len() + 1);
    if !p.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
            *p.add(s.len()) = 0;
        }
    }
    p
}

fn dup_wide(s: &[u8]) -> *mut u16 {
    let p = AtlasCrtMalloc((s.len() + 1) * 2).cast::<u16>();
    if !p.is_null() {
        unsafe {
            for (i, &b) in s.iter().enumerate() {
                *p.add(i) = u16::from(b);
            }
            *p.add(s.len()) = 0;
        }
    }
    p
}

const DAYS_STR: &[u8] = b":Sun:Sunday:Mon:Monday:Tue:Tuesday:Wed:Wednesday:Thu:Thursday:Fri:Friday:Sat:Saturday";
const MONTHS_STR: &[u8] = b":Jan:January:Feb:February:Mar:March:Apr:April:May:May:Jun:June:Jul:July:Aug:August:Sep:September:Oct:October:Nov:November:Dec:December";

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetdays() -> *mut u8 {
    dup_bytes(DAYS_STR)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetmonths() -> *mut u8 {
    dup_bytes(MONTHS_STR)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGettnames() -> *mut c_void {
    AtlasCrtMalloc(64).cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWGetdays() -> *mut u16 {
    dup_wide(DAYS_STR)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWGetmonths() -> *mut u16 {
    dup_wide(MONTHS_STR)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWGettnames() -> *mut c_void {
    AtlasCrtMalloc(64).cast()
}

// ---- Multibyte / Wide conversions (single-byte CP1252 in C locale) ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbtowcL(pwc: *mut u16, s: *const u8, n: usize, _loc: usize) -> i32 {
    if s.is_null() || n == 0 {
        return 0;
    }
    let b = unsafe { *s };
    if !pwc.is_null() {
        unsafe { *pwc = from_1252(b) };
    }
    if b == 0 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbrtowc(pwc: *mut u16, s: *const u8, n: usize, _ps: *mut c_void) -> usize {
    if s.is_null() {
        return 0;
    }
    if n == 0 {
        return (-2isize) as usize;
    }
    let b = unsafe { *s };
    if !pwc.is_null() {
        unsafe { *pwc = from_1252(b) };
    }
    if b == 0 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbrlen(s: *const u8, n: usize, ps: *mut c_void) -> usize {
    unsafe { AtlasCrtMbrtowc(core::ptr::null_mut(), s, n, ps) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcrtomb(s: *mut u8, wc: u16, _ps: *mut c_void) -> usize {
    if s.is_null() {
        return 1;
    }
    if let Some(b) = to_1252(wc) {
        unsafe { *s = b };
        1
    } else {
        set_errno(EILSEQ);
        usize::MAX
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcrtombS(retval: *mut usize, dst: *mut u8, size: usize, wc: u16, ps: *mut c_void) -> i32 {
    if dst.is_null() {
        if !retval.is_null() {
            unsafe { *retval = 1 };
        }
        return 0;
    }
    if size == 0 {
        set_errno(ERANGE);
        return ERANGE;
    }
    let r = unsafe { AtlasCrtWcrtomb(dst, wc, ps) };
    if r == usize::MAX {
        if !retval.is_null() {
            unsafe { *retval = usize::MAX };
        }
        return EILSEQ;
    }
    if !retval.is_null() {
        unsafe { *retval = r };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbstowcs(dst: *mut u16, src: *const u8, max: usize) -> usize {
    if src.is_null() {
        set_errno(EINVAL);
        return usize::MAX;
    }
    let mut i = 0;
    unsafe {
        if dst.is_null() {
            while *src.add(i) != 0 {
                i += 1;
            }
            return i;
        }
        while i < max {
            let b = *src.add(i);
            *dst.add(i) = from_1252(b);
            if b == 0 {
                break;
            }
            i += 1;
        }
    }
    i
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbstowcsS(retval: *mut usize, dst: *mut u16, size: usize, src: *const u8, max: usize) -> i32 {
    if !retval.is_null() {
        unsafe { *retval = 0 };
    }
    if src.is_null() || (dst.is_null() && size != 0) || (!dst.is_null() && size == 0) {
        set_errno(EINVAL);
        return EINVAL;
    }
    let src_len = unsafe { AtlasCrtStrlen(src) };
    if dst.is_null() {
        if !retval.is_null() {
            unsafe { *retval = src_len + 1 };
        }
        return 0;
    }
    let limit = if max == usize::MAX { size - 1 } else { max.min(size - 1) };
    let copy_n = src_len.min(limit);
    unsafe {
        for i in 0..copy_n {
            *dst.add(i) = from_1252(*src.add(i));
        }
        *dst.add(copy_n) = 0;
        if !retval.is_null() {
            *retval = copy_n + 1;
        }
    }
    if copy_n < src_len && max != usize::MAX && copy_n < max {
        set_errno(ERANGE);
        return ERANGE;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbsrtowcs(dst: *mut u16, src_ptr: *mut *const u8, len: usize, _ps: *mut c_void) -> usize {
    if src_ptr.is_null() || unsafe { (*src_ptr).is_null() } {
        set_errno(EINVAL);
        return usize::MAX;
    }
    let src = unsafe { *src_ptr };
    if dst.is_null() {
        return unsafe { AtlasCrtStrlen(src) };
    }
    let mut i = 0;
    unsafe {
        while i < len {
            let b = *src.add(i);
            *dst.add(i) = from_1252(b);
            if b == 0 {
                *src_ptr = core::ptr::null();
                return i;
            }
            i += 1;
        }
        *src_ptr = src.add(i);
    }
    i
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcstombs(dst: *mut u8, src: *const u16, max: usize) -> usize {
    if src.is_null() {
        set_errno(EINVAL);
        return usize::MAX;
    }
    let mut i = 0;
    unsafe {
        if dst.is_null() {
            while *src.add(i) != 0 {
                i += 1;
            }
            return i;
        }
        while i < max {
            let u = *src.add(i);
            *dst.add(i) = to_1252(u).unwrap_or(b'?');
            if u == 0 {
                break;
            }
            i += 1;
        }
    }
    i
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcstombsS(retval: *mut usize, dst: *mut u8, size: usize, src: *const u16, max: usize) -> i32 {
    if !retval.is_null() {
        unsafe { *retval = 0 };
    }
    if src.is_null() || (dst.is_null() && size != 0) || (!dst.is_null() && size == 0) {
        set_errno(EINVAL);
        return EINVAL;
    }
    let src_len = unsafe { AtlasCrtWcslen(src) };
    if dst.is_null() {
        if !retval.is_null() {
            unsafe { *retval = src_len + 1 };
        }
        return 0;
    }
    let limit = if max == usize::MAX { size - 1 } else { max.min(size - 1) };
    let copy_n = src_len.min(limit);
    unsafe {
        for i in 0..copy_n {
            *dst.add(i) = to_1252(*src.add(i)).unwrap_or(b'?');
        }
        *dst.add(copy_n) = 0;
        if !retval.is_null() {
            *retval = copy_n + 1;
        }
    }
    0
}

// ---- Aligned heap allocations ----

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtAlignedOffsetMalloc(size: usize, alignment: usize, offset: usize) -> *mut u8 {
    let align = alignment.max(size_of::<usize>());
    if !align.is_power_of_two() || (offset != 0 && offset >= size) {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let Some(total) = size.checked_add(align).and_then(|v| v.checked_add(size_of::<usize>())) else {
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    };
    let raw = AtlasCrtMalloc(total);
    if raw.is_null() {
        return core::ptr::null_mut();
    }
    let base = raw as usize + size_of::<usize>() + offset;
    let aligned_off = (base + (align - 1)) & !(align - 1);
    let ptr = (aligned_off - offset) as *mut u8;
    // SAFETY: `ptr - size_of::<usize>() >= raw`, within the allocated block.
    unsafe {
        ptr.sub(size_of::<usize>()).cast::<*mut u8>().write_unaligned(raw);
    }
    ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtAlignedMalloc(size: usize, alignment: usize) -> *mut u8 {
    AtlasCrtAlignedOffsetMalloc(size, alignment, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtAlignedFree(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: allocated by `AtlasCrtAlignedOffsetMalloc`.
    let raw = unsafe { ptr.sub(size_of::<usize>()).cast::<*mut u8>().read_unaligned() };
    AtlasCrtFree(raw);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAlignedRealloc(ptr: *mut u8, size: usize, alignment: usize) -> *mut u8 {
    if ptr.is_null() {
        return AtlasCrtAlignedMalloc(size, alignment);
    }
    if size == 0 {
        AtlasCrtAlignedFree(ptr);
        return core::ptr::null_mut();
    }
    let old_raw = unsafe { ptr.sub(size_of::<usize>()).cast::<*mut u8>().read_unaligned() };
    let old_total = crate::RtlSizeHeap(crate::RtlGetProcessHeap(), 0, old_raw.cast());
    let old_usable = if old_total != usize::MAX {
        old_total.saturating_sub(ptr as usize - old_raw as usize)
    } else {
        size
    };
    let new_ptr = AtlasCrtAlignedMalloc(size, alignment);
    if new_ptr.is_null() {
        return core::ptr::null_mut();
    }
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, new_ptr, old_usable.min(size));
    }
    AtlasCrtFree(old_raw);
    new_ptr
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetHeapHandle() -> usize {
    crate::RtlGetProcessHeap() as usize
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtHeapchk() -> i32 {
    -2 // _HEAPOK
}

static mut SYS_ERRLIST: [*const u8; 43] = [core::ptr::null(); 43];
static SYS_ERRLIST_INIT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSysErrlist() -> *mut *const u8 {
    if !SYS_ERRLIST_INIT.load(Ordering::Acquire) {
        for i in 0..43 {
            unsafe {
                SYS_ERRLIST[i] = AtlasCrtStrerror(i as i32);
            }
        }
        SYS_ERRLIST_INIT.store(true, Ordering::Release);
    }
    (&raw mut SYS_ERRLIST).cast()
}

static WCMDLN: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtWideWinmainCommandLine() -> *const u16 {
    if WCMDLN.load(Ordering::Acquire) == 0 {
        let p = get_command_line_w() as usize as *const u16;
        if p.is_null() {
            static EMPTY: [u16; 1] = [0];
            return EMPTY.as_ptr();
        }
        let mut i = 0usize;
        let mut quoted = false;
        unsafe {
            while *p.add(i) != 0 && (quoted || (*p.add(i) != b' ' as u16 && *p.add(i) != b'\t' as u16)) {
                if *p.add(i) == b'"' as u16 {
                    quoted = !quoted;
                }
                i += 1;
            }
            while *p.add(i) == b' ' as u16 || *p.add(i) == b'\t' as u16 {
                i += 1;
            }
            WCMDLN.store(p.add(i) as u64, Ordering::Release);
        }
    }
    WCMDLN.load(Ordering::Acquire) as usize as *const u16
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWtof(s: *const u16) -> f64 {
    unsafe { AtlasCrtWcstod(s, core::ptr::null_mut()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWtoi64(s: *const u16) -> i64 {
    unsafe { AtlasCrtWcstoll(s, core::ptr::null_mut(), 10) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtGetenvS(
    ret_sz: *mut usize,
    buf: *mut u8,
    cap: usize,
    name: *const u8,
) -> i32 {
    if ret_sz.is_null() || name.is_null() || (buf.is_null() && cap > 0) {
        set_errno(EINVAL);
        return EINVAL;
    }
    let val = unsafe { AtlasCrtGetenv(name) };
    if val.is_null() {
        unsafe { *ret_sz = 0 };
        if !buf.is_null() && cap > 0 {
            unsafe { *buf = 0 };
        }
        return 0;
    }
    let need = unsafe { AtlasCrtStrlen(val) } + 1;
    unsafe { *ret_sz = need };
    if buf.is_null() || cap == 0 {
        return 0;
    }
    if need > cap {
        unsafe { *buf = 0 };
        set_errno(ERANGE);
        return ERANGE;
    }
    unsafe { core::ptr::copy_nonoverlapping(val, buf, need) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWgetenvS(
    ret_sz: *mut usize,
    buf: *mut u16,
    cap: usize,
    name: *const u16,
) -> i32 {
    if ret_sz.is_null() || name.is_null() || (buf.is_null() && cap > 0) {
        set_errno(EINVAL);
        return EINVAL;
    }
    let val = unsafe { AtlasCrtWgetenv(name) };
    if val.is_null() {
        unsafe { *ret_sz = 0 };
        if !buf.is_null() && cap > 0 {
            unsafe { *buf = 0 };
        }
        return 0;
    }
    let need = unsafe { AtlasCrtWcslen(val) } + 1;
    unsafe { *ret_sz = need };
    if buf.is_null() || cap == 0 {
        return 0;
    }
    if need > cap {
        unsafe { *buf = 0 };
        set_errno(ERANGE);
        return ERANGE;
    }
    unsafe { core::ptr::copy_nonoverlapping(val, buf, need) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAccess(path: *const u8, _mode: i32) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut info = [0u32; 9];
    if get_file_attributes_ex_a(path as usize as u64, 0, info.as_mut_ptr() as usize as u64) == 0 {
        set_errno(map_win32_errno());
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWaccess(path: *const u16, _mode: i32) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut info = [0u32; 9];
    if get_file_attributes_ex_w(path as usize as u64, 0, info.as_mut_ptr() as usize as u64) == 0 {
        set_errno(map_win32_errno());
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetpid() -> i32 {
    get_current_process_id() as i32
}

#[repr(C)]
pub struct DivT {
    pub quot: i32,
    pub rem: i32,
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtDiv(numer: i32, denom: i32) -> DivT {
    DivT { quot: numer / denom, rem: numer % denom }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtCputs(s: *const u8) -> i32 {
    if s.is_null() {
        return -1;
    }
    let n = unsafe { AtlasCrtStrlen(s) };
    let out = crate::stdio::AtlasCrtIob(1);
    unsafe { crate::stdio::AtlasCrtFwrite(s.cast(), 1, n, out) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbschr(s: *const u8, c: u32) -> *const u8 {
    unsafe { AtlasCrtStrchr(s, c as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbsnbcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    unsafe { AtlasCrtStrncmp(s1, s2, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbsnbcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    unsafe { AtlasCrtStrncpy(dst, src, n) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMbspbrk(s: *const u8, accept: *const u8) -> *const u8 {
    unsafe { AtlasCrtStrpbrk(s, accept) }
}

#[repr(C)]
pub struct Timespec32 {
    pub tv_sec: i32,
    pub tv_nsec: i32,
}

#[repr(C)]
pub struct Timespec64 {
    pub tv_sec: i64,
    pub tv_nsec: i32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtTimespec32Get(ts: *mut Timespec32, base: i32) -> i32 {
    if ts.is_null() || base != 1 {
        return 0;
    }
    let sec = unsafe { AtlasCrtTime64(core::ptr::null_mut()) };
    unsafe {
        (*ts).tv_sec = sec as i32;
        (*ts).tv_nsec = 0;
    }
    base
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtTimespec64Get(ts: *mut Timespec64, base: i32) -> i32 {
    if ts.is_null() || base != 1 {
        return 0;
    }
    let sec = unsafe { AtlasCrtTime64(core::ptr::null_mut()) };
    unsafe {
        (*ts).tv_sec = sec;
        (*ts).tv_nsec = 0;
    }
    base
}

// ---- Errno, strings, numbers, assertions, terminate ----

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetErrno(value: i32) -> i32 {
    set_errno(value);
    0
}

static mut SYS_NERR: i32 = 43;

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSysNerr() -> *mut i32 {
    &raw mut SYS_NERR
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrerrorS(buf: *mut u8, size: usize, errnum: i32) -> i32 {
    if buf.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    let msg = AtlasCrtStrerror(errnum);
    let n = unsafe { AtlasCrtStrlen(msg) }.min(size - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(msg, buf, n);
        *buf.add(n) = 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrncnt(s: *const u8, count: usize) -> usize {
    if s.is_null() { 0 } else { unsafe { AtlasCrtStrnlen(s, count) } }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrncpyS(d: *mut u8, size: usize, s: *const u8, count: usize) -> i32 {
    if d.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    if count == 0 {
        unsafe { *d = 0 };
        return 0;
    }
    if s.is_null() {
        unsafe { *d = 0 };
        set_errno(EINVAL);
        return EINVAL;
    }
    let limit = if count == usize::MAX { size - 1 } else { count.min(size - 1) };
    let mut i = 0;
    unsafe {
        while i < limit && *s.add(i) != 0 {
            *d.add(i) = *s.add(i);
            i += 1;
        }
        if count != usize::MAX && i == size - 1 && i < count && *s.add(i) != 0 {
            *d = 0;
            set_errno(ERANGE);
            return ERANGE;
        }
        *d.add(i) = 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcscpyS(d: *mut u16, size: usize, s: *const u16) -> i32 {
    if d.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    if s.is_null() {
        unsafe { *d = 0 };
        set_errno(EINVAL);
        return EINVAL;
    }
    let n = unsafe { AtlasCrtWcslen(s) };
    if n >= size {
        unsafe { *d = 0 };
        set_errno(ERANGE);
        return ERANGE;
    }
    unsafe { core::ptr::copy_nonoverlapping(s, d, n + 1) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcsncpyS(d: *mut u16, size: usize, s: *const u16, count: usize) -> i32 {
    if d.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    if count == 0 {
        unsafe { *d = 0 };
        return 0;
    }
    if s.is_null() {
        unsafe { *d = 0 };
        set_errno(EINVAL);
        return EINVAL;
    }
    let limit = if count == usize::MAX { size - 1 } else { count.min(size - 1) };
    let mut i = 0;
    unsafe {
        while i < limit && *s.add(i) != 0 {
            *d.add(i) = *s.add(i);
            i += 1;
        }
        if count != usize::MAX && i == size - 1 && i < count && *s.add(i) != 0 {
            *d = 0;
            set_errno(ERANGE);
            return ERANGE;
        }
        *d.add(i) = 0;
    }
    0
}

unsafe fn format_radix(mut val: u64, negative: bool, buf: *mut u8, radix: i32) -> *mut u8 {
    if buf.is_null() || !(2..=36).contains(&radix) {
        return buf;
    }
    let mut tmp = [0u8; 66];
    let mut n = 0usize;
    let r = radix as u64;
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if val == 0 {
        tmp[0] = b'0';
        n = 1;
    } else {
        while val != 0 {
            tmp[n] = DIGITS[(val % r) as usize];
            val /= r;
            n += 1;
        }
    }
    let mut at = 0usize;
    unsafe {
        if negative {
            *buf.add(at) = b'-';
            at += 1;
        }
        while n > 0 {
            n -= 1;
            *buf.add(at) = tmp[n];
            at += 1;
        }
        *buf.add(at) = 0;
    }
    buf
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtItoa(val: i32, buf: *mut u8, radix: i32) -> *mut u8 {
    if radix == 10 && val < 0 {
        unsafe { format_radix(val.unsigned_abs() as u64, true, buf, radix) }
    } else {
        unsafe { format_radix(val as u32 as u64, false, buf, radix) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtUltoa(val: u32, buf: *mut u8, radix: i32) -> *mut u8 {
    unsafe { format_radix(val as u64, false, buf, radix) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtI64toa(val: i64, buf: *mut u8, radix: i32) -> *mut u8 {
    if radix == 10 && val < 0 {
        unsafe { format_radix(val.unsigned_abs(), true, buf, radix) }
    } else {
        unsafe { format_radix(val as u64, false, buf, radix) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtUi64toa(val: u64, buf: *mut u8, radix: i32) -> *mut u8 {
    unsafe { format_radix(val, false, buf, radix) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrlwr(s: *mut u8) -> *mut u8 {
    if !s.is_null() {
        let mut i = 0;
        unsafe {
            while *s.add(i) != 0 {
                *s.add(i) = AtlasCrtTolower(i32::from(*s.add(i))) as u8;
                i += 1;
            }
        }
    }
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrlwrS(s: *mut u8, size: usize) -> i32 {
    if s.is_null() || size == 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    unsafe { AtlasCrtStrlwr(s) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrupr(s: *mut u8) -> *mut u8 {
    if !s.is_null() {
        let mut i = 0;
        unsafe {
            while *s.add(i) != 0 {
                *s.add(i) = AtlasCrtToupper(i32::from(*s.add(i))) as u8;
                i += 1;
            }
        }
    }
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrrev(s: *mut u8) -> *mut u8 {
    if s.is_null() {
        return s;
    }
    unsafe {
        let n = AtlasCrtStrlen(s);
        let (mut a, mut b) = (0, n);
        while a + 1 < b {
            b -= 1;
            let t = *s.add(a);
            *s.add(a) = *s.add(b);
            *s.add(b) = t;
            a += 1;
        }
    }
    s
}

unsafe fn copy_part_u8(dst: *mut u8, dst_sz: usize, src: &[u8]) -> i32 {
    if dst.is_null() || dst_sz == 0 {
        return 0;
    }
    if src.len() >= dst_sz {
        unsafe { *dst = 0 };
        return ERANGE;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        *dst.add(src.len()) = 0;
    }
    0
}

unsafe fn copy_part_u16(dst: *mut u16, dst_sz: usize, src: &[u16]) -> i32 {
    if dst.is_null() || dst_sz == 0 {
        return 0;
    }
    if src.len() >= dst_sz {
        unsafe { *dst = 0 };
        return ERANGE;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        *dst.add(src.len()) = 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtSplitpathS(
    path: *const u8,
    drive: *mut u8, drive_sz: usize,
    dir: *mut u8, dir_sz: usize,
    fname: *mut u8, fname_sz: usize,
    ext: *mut u8, ext_sz: usize,
) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    let s = unsafe { core::slice::from_raw_parts(path, AtlasCrtStrlen(path)) };
    let (drv, rest) = if s.len() >= 2 && s[1] == b':' { (&s[..2], &s[2..]) } else { (&s[..0], s) };
    let slash = rest.iter().rposition(|&c| c == b'\\' || c == b'/').map(|i| i + 1).unwrap_or(0);
    let (d, file) = (&rest[..slash], &rest[slash..]);
    let dot = file.iter().rposition(|&c| c == b'.').unwrap_or(file.len());
    let (name, ex) = (&file[..dot], &file[dot..]);
    unsafe {
        let e = copy_part_u8(drive, drive_sz, drv)
            | copy_part_u8(dir, dir_sz, d)
            | copy_part_u8(fname, fname_sz, name)
            | copy_part_u8(ext, ext_sz, ex);
        if e != 0 {
            set_errno(ERANGE);
            return ERANGE;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWsplitpathS(
    path: *const u16,
    drive: *mut u16, drive_sz: usize,
    dir: *mut u16, dir_sz: usize,
    fname: *mut u16, fname_sz: usize,
    ext: *mut u16, ext_sz: usize,
) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    let s = unsafe { core::slice::from_raw_parts(path, AtlasCrtWcslen(path)) };
    let (drv, rest) = if s.len() >= 2 && s[1] == u16::from(b':') { (&s[..2], &s[2..]) } else { (&s[..0], s) };
    let slash = rest.iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/')).map(|i| i + 1).unwrap_or(0);
    let (d, file) = (&rest[..slash], &rest[slash..]);
    let dot = file.iter().rposition(|&c| c == u16::from(b'.')).unwrap_or(file.len());
    let (name, ex) = (&file[..dot], &file[dot..]);
    unsafe {
        let e = copy_part_u16(drive, drive_sz, drv)
            | copy_part_u16(dir, dir_sz, d)
            | copy_part_u16(fname, fname_sz, name)
            | copy_part_u16(ext, ext_sz, ex);
        if e != 0 {
            set_errno(ERANGE);
            return ERANGE;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWgetenv(name: *const u16) -> *const u16 {
    if name.is_null() {
        return core::ptr::null();
    }
    let table = crt::AtlasCrtGetInitialWideEnvironment() as *const *const u16;
    if table.is_null() {
        return core::ptr::null();
    }
    unsafe {
        let n = AtlasCrtWcslen(name);
        let mut i = 0;
        while !(*table.add(i)).is_null() {
            let entry = *table.add(i);
            if crt::AtlasCrtWcsnicmp(entry, name, n) == 0 && *entry.add(n) == u16::from(b'=') {
                return entry.add(n + 1);
            }
            i += 1;
        }
    }
    core::ptr::null()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtPutenvS(name: *const u8, value: *const u8) -> i32 {
    if name.is_null() || value.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    let val = if unsafe { *value } == 0 { core::ptr::null() } else { value };
    crate::env::AtlasSetEnvironmentVariableA(name, val);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtTerminate() -> ! {
    AtlasCrtAbort()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtAssert(msg: *const u8, file: *const u8, _line: u32) -> ! {
    crate::stdio::write_err(b"Assertion failed: ");
    if !msg.is_null() {
        let s = unsafe { core::slice::from_raw_parts(msg, AtlasCrtStrlen(msg)) };
        crate::stdio::write_err(s);
    }
    if !file.is_null() {
        crate::stdio::write_err(b", file ");
        let s = unsafe { core::slice::from_raw_parts(file, AtlasCrtStrlen(file)) };
        crate::stdio::write_err(s);
    }
    crate::stdio::write_err(b"\n");
    AtlasCrtAbort()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWassert(_msg: *const u16, _file: *const u16, _line: u32) -> ! {
    crate::stdio::write_err(b"Assertion failed\n");
    AtlasCrtAbort()
}

static NEW_HANDLER: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetNewHandler(handler: usize) -> usize {
    NEW_HANDLER.swap(handler as u64, Ordering::AcqRel) as usize
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtInvokeWatson() -> ! {
    crate::sys::terminate(0xc000_0417)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtPurecall() -> ! {
    crate::stdio::write_err(b"pure virtual function call\n");
    AtlasCrtAbort()
}

// ---- Time (UTC) ----

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Tm {
    pub tm_sec: i32,
    pub tm_min: i32,
    pub tm_hour: i32,
    pub tm_mday: i32,
    pub tm_mon: i32,
    pub tm_year: i32,
    pub tm_wday: i32,
    pub tm_yday: i32,
    pub tm_isdst: i32,
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn unix_to_tm(t: i64) -> Tm {
    let days = t.div_euclid(86400);
    let rem = t.rem_euclid(86400);
    let hour = (rem / 3600) as i32;
    let min = ((rem % 3600) / 60) as i32;
    let sec = (rem % 60) as i32;
    let wday = ((days + 4).rem_euclid(7)) as i32;

    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as i32;
    let year = y + i64::from(m <= 2);

    const CUM_DAYS: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = CUM_DAYS[(m - 1) as usize] + (d - 1) + i32::from(m > 2 && is_leap(year));

    Tm {
        tm_sec: sec,
        tm_min: min,
        tm_hour: hour,
        tm_mday: d,
        tm_mon: m - 1,
        tm_year: (year - 1900) as i32,
        tm_wday: wday,
        tm_yday: yday,
        tm_isdst: 0,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m_adj = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * m_adj + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + (doe as i64) - 719_468
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtGmtime64S(out: *mut Tm, timer: *const i64) -> i32 {
    if out.is_null() || timer.is_null() {
        set_errno(EINVAL);
        return EINVAL;
    }
    let t = unsafe { *timer };
    if t < 0 {
        set_errno(EINVAL);
        return EINVAL;
    }
    unsafe { out.write(unix_to_tm(t)) };
    0
}

static mut STATIC_TM: Tm = Tm {
    tm_sec: 0, tm_min: 0, tm_hour: 0, tm_mday: 1, tm_mon: 0, tm_year: 70, tm_wday: 4, tm_yday: 0, tm_isdst: 0,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtGmtime64(timer: *const i64) -> *mut Tm {
    if timer.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let out = &raw mut STATIC_TM;
    if unsafe { AtlasCrtGmtime64S(out, timer) } == 0 { out } else { core::ptr::null_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtMktime64(tm_ptr: *mut Tm) -> i64 {
    if tm_ptr.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let tm = unsafe { *tm_ptr };
    let mut year = i64::from(tm.tm_year) + 1900;
    let mut mon = i64::from(tm.tm_mon);
    year += mon.div_euclid(12);
    mon = mon.rem_euclid(12) + 1;
    let days = days_from_civil(year, mon, 1) + i64::from(tm.tm_mday - 1);
    let secs = days * 86400 + i64::from(tm.tm_hour) * 3600 + i64::from(tm.tm_min) * 60 + i64::from(tm.tm_sec);
    if secs < 0 {
        return -1;
    }
    unsafe { tm_ptr.write(unix_to_tm(secs)) };
    secs
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtDifftime64(t1: i64, t0: i64) -> f64 {
    (t1 - t0) as f64
}

struct StrfOut<F> {
    put: F,
    written: usize,
}

impl<F: FnMut(u8) -> bool> StrfOut<F> {
    fn byte(&mut self, b: u8) -> bool {
        if (self.put)(b) {
            self.written += 1;
            true
        } else {
            false
        }
    }

    fn slice(&mut self, s: &[u8]) -> bool {
        for &b in s {
            if !self.byte(b) {
                return false;
            }
        }
        true
    }

    fn num(&mut self, mut v: i32, width: usize, pad: u8) -> bool {
        if v < 0 {
            if !self.byte(b'-') {
                return false;
            }
            v = -v;
        }
        let mut digits = [0u8; 16];
        let mut n = 0;
        let mut u = v as u32;
        if u == 0 {
            digits[0] = b'0';
            n = 1;
        } else {
            while u > 0 {
                digits[n] = b'0' + (u % 10) as u8;
                u /= 10;
                n += 1;
            }
        }
        for _ in n..width {
            if !self.byte(pad) {
                return false;
            }
        }
        while n > 0 {
            n -= 1;
            if !self.byte(digits[n]) {
                return false;
            }
        }
        true
    }
}

fn strftime_core(put: impl FnMut(u8) -> bool, fmt: &[u8], tm: &Tm) -> Option<usize> {
    const WDAY_SHORT: [&[u8]; 7] = [b"Sun", b"Mon", b"Tue", b"Wed", b"Thu", b"Fri", b"Sat"];
    const WDAY_LONG: [&[u8]; 7] = [b"Sunday", b"Monday", b"Tuesday", b"Wednesday", b"Thursday", b"Friday", b"Saturday"];
    const MON_SHORT: [&[u8]; 12] = [b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec"];
    const MON_LONG: [&[u8]; 12] = [b"January", b"February", b"March", b"April", b"May", b"June", b"July", b"August", b"September", b"October", b"November", b"December"];

    let mut out = StrfOut { put, written: 0 };
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' {
            if !out.byte(c) {
                return None;
            }
            continue;
        }
        if i >= fmt.len() {
            break;
        }
        let mut spec = fmt[i];
        i += 1;
        if (spec == b'#' || spec == b'E' || spec == b'O') && i < fmt.len() {
            spec = fmt[i];
            i += 1;
        }
        let wday = (tm.tm_wday as usize).min(6);
        let mon = (tm.tm_mon as usize).min(11);
        let year = tm.tm_year + 1900;
        let hour12 = match tm.tm_hour % 12 { 0 => 12, h => h };
        let ok = match spec {
            b'%' => out.byte(b'%'),
            b'n' => out.byte(b'\n'),
            b't' => out.byte(b'\t'),
            b'a' => out.slice(WDAY_SHORT[wday]),
            b'A' => out.slice(WDAY_LONG[wday]),
            b'b' | b'h' => out.slice(MON_SHORT[mon]),
            b'B' => out.slice(MON_LONG[mon]),
            b'Y' => out.num(year, 4, b'0'),
            b'y' => out.num(year.rem_euclid(100), 2, b'0'),
            b'C' => out.num(year / 100, 2, b'0'),
            b'm' => out.num(tm.tm_mon + 1, 2, b'0'),
            b'd' => out.num(tm.tm_mday, 2, b'0'),
            b'e' => out.num(tm.tm_mday, 2, b' '),
            b'H' => out.num(tm.tm_hour, 2, b'0'),
            b'I' => out.num(hour12, 2, b'0'),
            b'M' => out.num(tm.tm_min, 2, b'0'),
            b'S' => out.num(tm.tm_sec, 2, b'0'),
            b'p' => out.slice(if tm.tm_hour < 12 { b"AM" } else { b"PM" }),
            b'j' => out.num(tm.tm_yday + 1, 3, b'0'),
            b'w' => out.num(tm.tm_wday, 1, b'0'),
            b'u' => out.num(if tm.tm_wday == 0 { 7 } else { tm.tm_wday }, 1, b'0'),
            b'Z' => out.slice(b"UTC"),
            b'z' => out.slice(b"+0000"),
            b'F' => out.num(year, 4, b'0') && out.byte(b'-') && out.num(tm.tm_mon + 1, 2, b'0') && out.byte(b'-') && out.num(tm.tm_mday, 2, b'0'),
            b'T' | b'X' => out.num(tm.tm_hour, 2, b'0') && out.byte(b':') && out.num(tm.tm_min, 2, b'0') && out.byte(b':') && out.num(tm.tm_sec, 2, b'0'),
            b'R' => out.num(tm.tm_hour, 2, b'0') && out.byte(b':') && out.num(tm.tm_min, 2, b'0'),
            b'D' | b'x' => out.num(tm.tm_mon + 1, 2, b'0') && out.byte(b'/') && out.num(tm.tm_mday, 2, b'0') && out.byte(b'/') && out.num(year.rem_euclid(100), 2, b'0'),
            b'c' => {
                out.slice(WDAY_SHORT[wday]) && out.byte(b' ') && out.slice(MON_SHORT[mon]) && out.byte(b' ')
                    && out.num(tm.tm_mday, 2, b' ') && out.byte(b' ')
                    && out.num(tm.tm_hour, 2, b'0') && out.byte(b':') && out.num(tm.tm_min, 2, b'0') && out.byte(b':') && out.num(tm.tm_sec, 2, b'0')
                    && out.byte(b' ') && out.num(year, 4, b'0')
            }
            other => out.byte(b'%') && out.byte(other),
        };
        if !ok {
            return None;
        }
    }
    Some(out.written)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrftime(buf: *mut u8, max: usize, fmt: *const u8, tm: *const Tm) -> usize {
    if buf.is_null() || max == 0 || fmt.is_null() || tm.is_null() {
        return 0;
    }
    let fmt_slice = unsafe { core::slice::from_raw_parts(fmt, AtlasCrtStrlen(fmt)) };
    let tm_ref = unsafe { &*tm };
    let mut pos = 0usize;
    let res = strftime_core(|b| {
        if pos + 1 >= max {
            return false;
        }
        unsafe { *buf.add(pos) = b };
        pos += 1;
        true
    }, fmt_slice, tm_ref);
    match res {
        Some(n) => {
            unsafe { *buf.add(n) = 0 };
            n
        }
        None => {
            unsafe { *buf = 0 };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStrftimeL(buf: *mut u8, max: usize, fmt: *const u8, tm: *const Tm, _loc: usize) -> usize {
    unsafe { AtlasCrtStrftime(buf, max, fmt, tm) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWcsftime(buf: *mut u16, max: usize, fmt: *const u16, tm: *const Tm, _lc: usize) -> usize {
    if buf.is_null() || max == 0 || fmt.is_null() || tm.is_null() {
        return 0;
    }
    let mut narrow = [0u8; 256];
    let flen = unsafe { AtlasCrtWcslen(fmt) }.min(narrow.len() - 1);
    for i in 0..flen {
        narrow[i] = to_1252(unsafe { *fmt.add(i) }).unwrap_or(b'?');
    }
    let tm_ref = unsafe { &*tm };
    let mut pos = 0usize;
    let res = strftime_core(|b| {
        if pos + 1 >= max {
            return false;
        }
        unsafe { *buf.add(pos) = from_1252(b) };
        pos += 1;
        true
    }, &narrow[..flen], tm_ref);
    match res {
        Some(n) => {
            unsafe { *buf.add(n) = 0 };
            n
        }
        None => {
            unsafe { *buf = 0 };
            0
        }
    }
}

// ---- Threads ----

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtBeginthreadex(
    security: usize,
    stack_size: u32,
    start_address: usize,
    arglist: usize,
    initflag: u32,
    thrdaddr: *mut u32,
) -> usize {
    let h = create_thread(
        security as u64,
        u64::from(stack_size),
        start_address as u64,
        arglist as u64,
        u64::from(initflag),
        thrdaddr as usize as u64,
    );
    if h == 0 {
        set_errno(EACCES);
    }
    h as usize
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtEndthreadex(retval: u32) -> ! {
    crate::loader::RtlExitUserThread(retval)
}

// ---- Filesystem ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtChdir(path: *const u8) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if set_current_directory_a(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWchdir(path: *const u16) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if set_current_directory_w(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtGetcwd(buf: *mut u8, maxlen: i32) -> *mut u8 {
    let need = get_current_directory_a(0, 0) as usize;
    if need == 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let out = if buf.is_null() {
        let alloc_sz = need.max(maxlen.max(0) as usize);
        let p = AtlasCrtMalloc(alloc_sz);
        if p.is_null() {
            return core::ptr::null_mut();
        }
        let n = get_current_directory_a(alloc_sz as u64, p as usize as u64) as usize;
        if n == 0 || n >= alloc_sz {
            AtlasCrtFree(p);
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        p
    } else {
        if maxlen <= 0 || need > maxlen as usize {
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        let n = get_current_directory_a(maxlen as u64, buf as usize as u64) as usize;
        if n == 0 || n >= maxlen as usize {
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        buf
    };
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWgetcwd(buf: *mut u16, maxlen: i32) -> *mut u16 {
    let need = get_current_directory_w(0, 0) as usize;
    if need == 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    if buf.is_null() {
        let alloc_sz = need.max(maxlen.max(0) as usize);
        let p = AtlasCrtMalloc(alloc_sz * 2).cast::<u16>();
        if p.is_null() {
            return core::ptr::null_mut();
        }
        let n = get_current_directory_w(alloc_sz as u64, p as usize as u64) as usize;
        if n == 0 || n >= alloc_sz {
            AtlasCrtFree(p.cast());
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        p
    } else {
        if maxlen <= 0 || need > maxlen as usize {
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        let n = get_current_directory_w(maxlen as u64, buf as usize as u64) as usize;
        if n == 0 || n >= maxlen as usize {
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        buf
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFullpath(abs_path: *mut u8, rel_path: *const u8, max_length: usize) -> *mut u8 {
    if rel_path.is_null() {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    if abs_path.is_null() {
        let need = get_full_path_name_a(rel_path as usize as u64, 0, 0, 0) as usize;
        if need == 0 {
            return core::ptr::null_mut();
        }
        let alloc_sz = need.max(max_length);
        let p = AtlasCrtMalloc(alloc_sz);
        if p.is_null() {
            return core::ptr::null_mut();
        }
        let n = get_full_path_name_a(rel_path as usize as u64, alloc_sz as u64, p as usize as u64, 0) as usize;
        if n == 0 || n >= alloc_sz {
            AtlasCrtFree(p);
            return core::ptr::null_mut();
        }
        p
    } else {
        let n = get_full_path_name_a(rel_path as usize as u64, max_length as u64, abs_path as usize as u64, 0) as usize;
        if n == 0 || n >= max_length {
            set_errno(ERANGE);
            return core::ptr::null_mut();
        }
        abs_path
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtUnlink(path: *const u8) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if delete_file_a(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWremove(path: *const u16) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if delete_file_w(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtRename(old: *const u8, new: *const u8) -> i32 {
    if old.is_null() || new.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    // MOVEFILE_REPLACE_EXISTING = 1
    if move_file_ex_a(old as usize as u64, new as usize as u64, 1) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWrename(old: *const u16, new: *const u16) -> i32 {
    if old.is_null() || new.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if move_file_ex_w(old as usize as u64, new as usize as u64, 1) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtRmdir(path: *const u8) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if remove_directory_a(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWrmdir(path: *const u16) -> i32 {
    if path.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if remove_directory_w(path as usize as u64) != 0 {
        0
    } else {
        set_errno(map_win32_errno());
        -1
    }
}

pub(crate) fn filetime_to_unix(lo: u32, hi: u32) -> i64 {
    let ft = u64::from(lo) | (u64::from(hi) << 32);
    if ft == 0 {
        return 0;
    }
    (ft / 10_000_000) as i64 - 11_644_473_600
}

#[repr(C)]
pub struct Stat64 {
    pub st_dev: u32,
    pub st_ino: u16,
    pub st_mode: u16,
    pub st_nlink: i16,
    pub st_uid: i16,
    pub st_gid: i16,
    pub _pad0: u16,
    pub st_rdev: u32,
    pub _pad1: u32,
    pub st_size: i64,
    pub st_atime: i64,
    pub st_mtime: i64,
    pub st_ctime: i64,
}

#[repr(C)]
pub struct Stat64i32 {
    pub st_dev: u32,
    pub st_ino: u16,
    pub st_mode: u16,
    pub st_nlink: i16,
    pub st_uid: i16,
    pub st_gid: i16,
    pub _pad0: u16,
    pub st_rdev: u32,
    pub st_size: i32,
    pub st_atime: i64,
    pub st_mtime: i64,
    pub st_ctime: i64,
}

pub(crate) fn fill_stat64(attrs: u32, size: u64, ctime: i64, atime: i64, mtime: i64) -> Stat64 {
    let is_dir = (attrs & 0x10) != 0;
    let mode = if is_dir { 0x4000 | 0o777 } else { 0x8000 | 0o666 };
    Stat64 {
        st_dev: 2,
        st_ino: 0,
        st_mode: mode,
        st_nlink: 1,
        st_uid: 0,
        st_gid: 0,
        _pad0: 0,
        st_rdev: 2,
        _pad1: 0,
        st_size: size as i64,
        st_atime: atime,
        st_mtime: mtime,
        st_ctime: ctime,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStat64(path: *const u8, buf: *mut Stat64) -> i32 {
    if path.is_null() || buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut info = [0u32; 9];
    if get_file_attributes_ex_a(path as usize as u64, 0, info.as_mut_ptr() as usize as u64) == 0 {
        set_errno(map_win32_errno());
        return -1;
    }
    let size = (u64::from(info[7]) << 32) | u64::from(info[8]);
    let st = fill_stat64(
        info[0],
        size,
        filetime_to_unix(info[1], info[2]),
        filetime_to_unix(info[3], info[4]),
        filetime_to_unix(info[5], info[6]),
    );
    unsafe { buf.write(st) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStat64i32(path: *const u8, buf: *mut Stat64i32) -> i32 {
    if buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut st = core::mem::MaybeUninit::<Stat64>::uninit();
    if unsafe { AtlasCrtStat64(path, st.as_mut_ptr()) } != 0 {
        return -1;
    }
    let s = unsafe { st.assume_init() };
    unsafe {
        buf.write(Stat64i32 {
            st_dev: s.st_dev,
            st_ino: s.st_ino,
            st_mode: s.st_mode,
            st_nlink: s.st_nlink,
            st_uid: s.st_uid,
            st_gid: s.st_gid,
            _pad0: 0,
            st_rdev: s.st_rdev,
            st_size: s.st_size as i32,
            st_atime: s.st_atime,
            st_mtime: s.st_mtime,
            st_ctime: s.st_ctime,
        });
    }
    0
}

#[repr(C)]
pub struct Stat32i64 {
    pub st_dev: u32,
    pub st_ino: u16,
    pub st_mode: u16,
    pub st_nlink: i16,
    pub st_uid: i16,
    pub st_gid: i16,
    pub _pad0: u16,
    pub st_rdev: u32,
    pub _pad1: u32,
    pub st_size: i64,
    pub st_atime: i32,
    pub st_mtime: i32,
    pub st_ctime: i32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtStat32i64(path: *const u8, buf: *mut Stat32i64) -> i32 {
    if buf.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut st = core::mem::MaybeUninit::<Stat64>::uninit();
    if unsafe { AtlasCrtStat64(path, st.as_mut_ptr()) } != 0 {
        return -1;
    }
    let s = unsafe { st.assume_init() };
    unsafe {
        buf.write(Stat32i64 {
            st_dev: s.st_dev,
            st_ino: s.st_ino,
            st_mode: s.st_mode,
            st_nlink: s.st_nlink,
            st_uid: s.st_uid,
            st_gid: s.st_gid,
            _pad0: 0,
            st_rdev: s.st_rdev,
            _pad1: 0,
            st_size: s.st_size,
            st_atime: s.st_atime as i32,
            st_mtime: s.st_mtime as i32,
            st_ctime: s.st_ctime as i32,
        });
    }
    0
}

#[repr(C)]
pub struct FindData64i32 {
    attrib: u32,
    _pad0: u32,
    time_create: i64,
    time_access: i64,
    time_write: i64,
    size: u32,
    name: [u8; 260],
}

fn fill_find_data(win32: &[u8; 320], out: &mut FindData64i32) {
    let rd = |off: usize| u32::from_le_bytes([win32[off], win32[off + 1], win32[off + 2], win32[off + 3]]);
    out.attrib = rd(0) & 0x37;
    out._pad0 = 0;
    out.time_create = filetime_to_unix(rd(4), rd(8));
    out.time_access = filetime_to_unix(rd(12), rd(16));
    out.time_write = filetime_to_unix(rd(20), rd(24));
    out.size = rd(32);
    out.name.copy_from_slice(&win32[44..304]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFindfirst64i32(filespec: *const u8, info: *mut FindData64i32) -> isize {
    if filespec.is_null() || info.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut win32 = [0u8; 320];
    let h = find_first_file_a(filespec as usize as u64, win32.as_mut_ptr() as usize as u64);
    if h == crate::pe::INVALID_HANDLE || h == 0 {
        set_errno(map_win32_errno());
        return -1;
    }
    fill_find_data(&win32, unsafe { &mut *info });
    h as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFindnext64i32(handle: isize, info: *mut FindData64i32) -> i32 {
    if handle == -1 || info.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut win32 = [0u8; 320];
    if find_next_file_a(handle as usize as u64, win32.as_mut_ptr() as usize as u64) == 0 {
        set_errno(ENOENT);
        return -1;
    }
    fill_find_data(&win32, unsafe { &mut *info });
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFindclose(handle: isize) -> i32 {
    if handle == -1 {
        set_errno(EINVAL);
        return -1;
    }
    if find_close(handle as usize as u64) != 0 { 0 } else { -1 }
}

// ---- setjmp / longjmp ----

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl AtlasCrtSetjmp",
    "AtlasCrtSetjmp:",
    "mov [rcx + 0x00], rdx",
    "mov [rcx + 0x08], rbx",
    "lea rax, [rsp + 8]",
    "mov [rcx + 0x10], rax",
    "mov [rcx + 0x18], rbp",
    "mov [rcx + 0x20], rsi",
    "mov [rcx + 0x28], rdi",
    "mov [rcx + 0x30], r12",
    "mov [rcx + 0x38], r13",
    "mov [rcx + 0x40], r14",
    "mov [rcx + 0x48], r15",
    "mov rax, [rsp]",
    "mov [rcx + 0x50], rax",
    "movdqu [rcx + 0x60], xmm6",
    "movdqu [rcx + 0x70], xmm7",
    "movdqu [rcx + 0x80], xmm8",
    "movdqu [rcx + 0x90], xmm9",
    "movdqu [rcx + 0xa0], xmm10",
    "movdqu [rcx + 0xb0], xmm11",
    "movdqu [rcx + 0xc0], xmm12",
    "movdqu [rcx + 0xd0], xmm13",
    "movdqu [rcx + 0xe0], xmm14",
    "movdqu [rcx + 0xf0], xmm15",
    "xor eax, eax",
    "ret",

    ".globl AtlasCrtLongjmp",
    "AtlasCrtLongjmp:",
    "mov eax, edx",
    "test eax, eax",
    "jnz 1f",
    "mov eax, 1",
    "1:",
    "mov rbx, [rcx + 0x08]",
    "mov rbp, [rcx + 0x18]",
    "mov rsi, [rcx + 0x20]",
    "mov rdi, [rcx + 0x28]",
    "mov r12, [rcx + 0x30]",
    "mov r13, [rcx + 0x38]",
    "mov r14, [rcx + 0x40]",
    "mov r15, [rcx + 0x48]",
    "movdqu xmm6,  [rcx + 0x60]",
    "movdqu xmm7,  [rcx + 0x70]",
    "movdqu xmm8,  [rcx + 0x80]",
    "movdqu xmm9,  [rcx + 0x90]",
    "movdqu xmm10, [rcx + 0xa0]",
    "movdqu xmm11, [rcx + 0xb0]",
    "movdqu xmm12, [rcx + 0xc0]",
    "movdqu xmm13, [rcx + 0xd0]",
    "movdqu xmm14, [rcx + 0xe0]",
    "movdqu xmm15, [rcx + 0xf0]",
    "mov rsp, [rcx + 0x10]",
    "jmp qword ptr [rcx + 0x50]",
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".globl AtlasCrtSetjmp",
    ".p2align 2",
    "AtlasCrtSetjmp:",
    "str x1, [x0, #0x00]",
    "stp x19, x20, [x0, #0x08]",
    "stp x21, x22, [x0, #0x18]",
    "stp x23, x24, [x0, #0x28]",
    "stp x25, x26, [x0, #0x38]",
    "stp x27, x28, [x0, #0x48]",
    "stp x29, x30, [x0, #0x58]",
    "mov x9, sp",
    "str x9, [x0, #0x68]",
    "stp d8, d9, [x0, #0x70]",
    "stp d10, d11, [x0, #0x80]",
    "stp d12, d13, [x0, #0x90]",
    "stp d14, d15, [x0, #0xa0]",
    "mov x0, #0",
    "ret",

    ".globl AtlasCrtLongjmp",
    ".p2align 2",
    "AtlasCrtLongjmp:",
    "ldp x19, x20, [x0, #0x08]",
    "ldp x21, x22, [x0, #0x18]",
    "ldp x23, x24, [x0, #0x28]",
    "ldp x25, x26, [x0, #0x38]",
    "ldp x27, x28, [x0, #0x48]",
    "ldp x29, x30, [x0, #0x58]",
    "ldr x9, [x0, #0x68]",
    "mov sp, x9",
    "ldp d8, d9, [x0, #0x70]",
    "ldp d10, d11, [x0, #0x80]",
    "ldp d12, d13, [x0, #0x90]",
    "ldp d14, d15, [x0, #0xa0]",
    "cmp w1, #0",
    "csinc w0, w1, wzr, ne",
    "ret",
);

#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl _AtlasCrtSetjmp",
    "_AtlasCrtSetjmp:",
    "mov ecx, [esp + 4]",
    "mov [ecx + 0x00], ebp",
    "mov [ecx + 0x04], ebx",
    "mov [ecx + 0x08], edi",
    "mov [ecx + 0x0c], esi",
    "lea eax, [esp + 4]",
    "mov [ecx + 0x10], eax",
    "mov eax, [esp]",
    "mov [ecx + 0x14], eax",
    "xor eax, eax",
    "ret",

    ".globl _AtlasCrtLongjmp",
    "_AtlasCrtLongjmp:",
    "mov ecx, [esp + 4]",
    "mov eax, [esp + 8]",
    "test eax, eax",
    "jnz 1f",
    "mov eax, 1",
    "1:",
    "mov ebp, [ecx + 0x00]",
    "mov ebx, [ecx + 0x04]",
    "mov edi, [ecx + 0x08]",
    "mov esi, [ecx + 0x0c]",
    "mov esp, [ecx + 0x10]",
    "jmp dword ptr [ecx + 0x14]",
);
