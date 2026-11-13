//! The environment, in the program, as Windows keeps it: the process parameters' block of UTF-16 `NAME=VALUE`
//! strings, sorted by name, which atlas made at the process's start. Names fold case. Setting a variable makes a new
//! block on the process heap.

use core::sync::atomic::{AtomicBool, Ordering};

use atlas_abi::{params, peb, teb};

const ERROR_ENVVAR_NOT_FOUND: u32 = 203;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;

static BUSY: AtomicBool = AtomicBool::new(false);
/// Whether the block is one this made, on the heap, which a new one replaces.
static OWNED: AtomicBool = AtomicBool::new(false);

fn with<R>(f: impl FnOnce() -> R) -> R {
    while BUSY.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    let result = f();
    BUSY.store(false, Ordering::Release);
    result
}

fn parameters() -> *mut u8 {
    // SAFETY: this thread's TEB, its PEB, and the process parameters atlas made.
    unsafe {
        let peb_at = crate::teb().add(teb::PEB as usize).cast::<*mut u8>().read();
        peb_at.add(peb::PROCESS_PARAMETERS as usize).cast::<*mut u8>().read()
    }
}

/// The block, as one slice to its last NUL.
fn block() -> &'static [u16] {
    // SAFETY: the block the process parameters point at, which ends in two NULs.
    unsafe {
        let at = parameters().add(params::ENVIRONMENT as usize).cast::<*const u16>().read();
        if at.is_null() {
            return &[0];
        }
        let mut i = 0;
        while *at.add(i) != 0 {
            while *at.add(i) != 0 {
                i += 1;
            }
            i += 1;
        }
        core::slice::from_raw_parts(at, i + 1)
    }
}

/// Each `NAME=VALUE` entry, without its NUL.
fn entries(b: &[u16]) -> impl Iterator<Item = &[u16]> {
    b.split(|&u| u == 0).take_while(|e| !e.is_empty())
}

fn upper(u: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&u) { u - 32 } else { u }
}

/// An entry's name and value. An entry may start with `=` (`=C:=C:\`), which is part of its name.
fn split(entry: &[u16]) -> (&[u16], &[u16]) {
    match entry.iter().skip(1).position(|&u| u == u16::from(b'=')) {
        Some(at) => (&entry[..at + 1], &entry[at + 2..]),
        None => (entry, &[]),
    }
}

fn same(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| upper(x) == upper(y))
}

fn value_of<'b>(b: &'b [u16], name: &[u16]) -> Option<&'b [u16]> {
    entries(b).map(split).find(|(n, _)| same(n, name)).map(|(_, v)| v)
}

/// A NUL-terminated UTF-16 string the program passed.
///
/// # Safety
///
/// `at` is a readable NUL-terminated string.
unsafe fn wide<'a>(at: *const u16) -> &'a [u16] {
    let mut n = 0;
    // SAFETY: as the caller says.
    unsafe {
        while *at.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(at, n)
    }
}

/// An ANSI string the program passed, as UTF-16 on the heap (free it with `crate::heap::free`).
///
/// # Safety
///
/// `at` is a readable NUL-terminated string.
unsafe fn widen(at: *const u8) -> (*mut u16, usize) {
    let mut n = 0;
    // SAFETY: as the caller says, and a heap block of n + 1 units.
    unsafe {
        while *at.add(n) != 0 {
            n += 1;
        }
        let out = crate::heap::allocate((n + 1) * 2, false).cast::<u16>();
        if out.is_null() {
            return (out, 0);
        }
        for i in 0..=n {
            *out.add(i) = atlas_abi::ansi::from_1252(*at.add(i));
        }
        (out, n)
    }
}

fn narrow(u: u16) -> u8 {
    atlas_abi::ansi::to_1252(u).unwrap_or(b'?')
}

/// Copies `value` and a NUL into a buffer of `size` units, as `GetEnvironmentVariable` does: its length without the
/// NUL if it fits, else the size it needs with it.
///
/// # Safety
///
/// `out` has room for `size` units of `T`.
unsafe fn give<T: Copy>(value: &[u16], out: *mut T, size: u32, convert: impl Fn(u16) -> T, nul: T) -> u32 {
    if (value.len() as u32) < size && !out.is_null() {
        // SAFETY: as the caller says.
        unsafe {
            for (i, &u) in value.iter().enumerate() {
                *out.add(i) = convert(u);
            }
            *out.add(value.len()) = nul;
        }
        value.len() as u32
    } else {
        value.len() as u32 + 1
    }
}

fn get(name: &[u16], out: impl FnOnce(&[u16]) -> u32) -> u32 {
    let found = with(|| value_of(block(), name).map(out));
    found.unwrap_or_else(|| {
        crate::RtlSetLastWin32Error(ERROR_ENVVAR_NOT_FOUND);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetEnvironmentVariableW(name: *const u16, out: *mut u16, size: u32) -> u32 {
    // SAFETY: the program's name, and its buffer of `size` units.
    unsafe { get(wide(name), |v| give(v, out, size, |u| u, 0)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetEnvironmentVariableA(name: *const u8, out: *mut u8, size: u32) -> u32 {
    // SAFETY: as above, for ANSI.
    unsafe {
        let (name, n) = widen(name);
        if name.is_null() {
            return 0;
        }
        let result = get(core::slice::from_raw_parts(name, n), |v| give(v, out, size, narrow, 0));
        crate::heap::free(name.cast());
        result
    }
}

/// Sets (or with no value, removes) a variable: a new block, in order, on the heap.
fn set(name: &[u16], value: Option<&[u16]>) -> i32 {
    if name.is_empty() || name.iter().skip(1).any(|&u| u == u16::from(b'=')) {
        crate::RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    with(|| {
        let old = block();
        let kept = || entries(old).filter(|e| !same(split(e).0, name));
        let mut units: usize = kept().map(|e| e.len() + 1).sum::<usize>() + 1;
        if let Some(v) = value {
            units += name.len() + 1 + v.len() + 1;
        }
        let new = crate::heap::allocate(units * 2, false).cast::<u16>();
        if new.is_null() {
            crate::RtlSetLastWin32Error(ERROR_NOT_ENOUGH_MEMORY);
            return 0;
        }
        let mut at = 0;
        let mut put = |s: &[u16]| {
            for &u in s {
                // SAFETY: within the `units` the new block has.
                unsafe { *new.add(at) = u };
                at += 1;
            }
        };
        let mut placed = value.is_none();
        let goes_before = |e: &[u16]| -> bool {
            let n = split(e).0;
            !n.starts_with(&[u16::from(b'=')]) && n.iter().map(|&u| upper(u)).cmp(name.iter().map(|&u| upper(u))) == core::cmp::Ordering::Greater
        };
        for e in kept() {
            if !placed && goes_before(e) {
                put(name);
                put(&[u16::from(b'=')]);
                put(value.unwrap_or(&[]));
                put(&[0]);
                placed = true;
            }
            put(e);
            put(&[0]);
        }
        if !placed {
            put(name);
            put(&[u16::from(b'=')]);
            put(value.unwrap_or(&[]));
            put(&[0]);
        }
        put(&[0]);
        // SAFETY: the process parameters' environment fields.
        unsafe {
            let p = parameters();
            p.add(params::ENVIRONMENT as usize).cast::<*mut u16>().write(new);
            p.add(params::ENVIRONMENT_SIZE as usize).cast::<u64>().write(units as u64 * 2);
        }
        if OWNED.swap(true, Ordering::AcqRel) {
            crate::heap::free(old.as_ptr().cast_mut().cast());
        }
        1
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32 {
    // SAFETY: the program's strings.
    unsafe { set(wide(name), (!value.is_null()).then(|| wide(value))) }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSetEnvironmentVariableA(name: *const u8, value: *const u8) -> i32 {
    // SAFETY: the program's strings, widened onto the heap.
    unsafe {
        let (n, nl) = widen(name);
        let v = (!value.is_null()).then(|| widen(value));
        let result = set(core::slice::from_raw_parts(n, nl), v.map(|(p, l)| core::slice::from_raw_parts(p.cast_const(), l)));
        crate::heap::free(n.cast());
        if let Some((p, _)) = v {
            crate::heap::free(p.cast());
        }
        result
    }
}

/// Expands `%NAME%`s in `src`, writing to `emit`; ones not set stay as they are. Gives the length.
fn expand(src: &[u16], mut emit: impl FnMut(u16)) -> usize {
    let percent = u16::from(b'%');
    with(|| {
        let b = block();
        let mut n = 0;
        let mut i = 0;
        let mut out = |u: u16| {
            emit(u);
            n += 1;
        };
        while i < src.len() {
            if src[i] == percent
                && let Some(end) = src[i + 1..].iter().position(|&u| u == percent)
                && let Some(v) = value_of(b, &src[i + 1..i + 1 + end]).filter(|_| end > 0)
            {
                v.iter().for_each(|&u| out(u));
                i += end + 2;
                continue;
            }
            out(src[i]);
            i += 1;
        }
        n
    })
}

/// `ExpandEnvironmentStrings`: the size the result needs, with its NUL, written if it fits in `size`.
fn expand_into<T: Copy>(src: &[u16], out: *mut T, size: u32, convert: impl Fn(u16) -> T, nul: T) -> u32 {
    let room = if out.is_null() { 0 } else { size as usize };
    let mut at = 0;
    let n = expand(src, |u| {
        if at < room {
            // SAFETY: the program's buffer of `size` units.
            unsafe { *out.add(at) = convert(u) };
        }
        at += 1;
    });
    if n < room {
        // SAFETY: as above.
        unsafe { *out.add(n) = nul };
    }
    n as u32 + 1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasExpandEnvironmentStringsW(src: *const u16, out: *mut u16, size: u32) -> u32 {
    // SAFETY: the program's string.
    expand_into(unsafe { wide(src) }, out, size, |u| u, 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasExpandEnvironmentStringsA(src: *const u8, out: *mut u8, size: u32) -> u32 {
    // SAFETY: the program's string, widened onto the heap.
    unsafe {
        let (s, n) = widen(src);
        let result = expand_into(core::slice::from_raw_parts(s, n), out, size, narrow, 0);
        crate::heap::free(s.cast());
        result
    }
}

/// A copy of the whole block, for the program to read and give back with `FreeEnvironmentStrings`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetEnvironmentStringsW() -> *mut u16 {
    with(|| {
        let b = block();
        let copy = crate::heap::allocate(b.len() * 2, false).cast::<u16>();
        if !copy.is_null() {
            // SAFETY: a heap block of the block's size.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), copy, b.len()) };
        }
        copy
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetEnvironmentStringsA() -> *mut u8 {
    with(|| {
        let b = block();
        let copy = crate::heap::allocate(b.len(), false);
        if !copy.is_null() {
            for (i, &u) in b.iter().enumerate() {
                // SAFETY: as above.
                unsafe { *copy.add(i) = narrow(u) };
            }
        }
        copy
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFreeEnvironmentStrings(block: *mut u8) -> i32 {
    crate::heap::free(block);
    1
}
