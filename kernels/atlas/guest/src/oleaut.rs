//! oleaut32's strings and variants: BSTRs (a 32-bit byte count, then the UTF-16, then a NUL, on the process heap) and
//! `VariantInit`, `VariantClear` and `VariantCopy`, which free or copy what a VARIANT holds.

use core::ffi::c_void;

/// A BSTR's count, just before its characters.
const PREFIX: usize = 4;

const VT_DISPATCH: u16 = 9;
const VT_BSTR: u16 = 8;
const VT_UNKNOWN: u16 = 13;
const VT_BYREF: u16 = 0x4000;
const E_OUTOFMEMORY: u32 = 0x8007_000e;
/// A VARIANT's size: its type, three reserved words, then two pointers' worth (a DECIMAL fills all of it).
const VARIANT: usize = 8 + 2 * core::mem::size_of::<usize>();

/// A BSTR of `bytes` bytes, from `src` if given, else zeroed; NUL after.
fn bstr(src: *const u8, bytes: usize) -> *mut u16 {
    let Some(total) = bytes.checked_add(PREFIX + 2).filter(|&t| t <= u32::MAX as usize) else { return core::ptr::null_mut() };
    let block = crate::heap::allocate(total, src.is_null());
    if block.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: a block `total` long, ours; `src` is the program's, `bytes` long, if given.
    unsafe {
        block.cast::<u32>().write(bytes as u32);
        if !src.is_null() {
            core::ptr::copy_nonoverlapping(src, block.add(PREFIX), bytes);
        }
        block.add(PREFIX + bytes).cast::<[u8; 2]>().write_unaligned([0, 0]);
        block.add(PREFIX).cast()
    }
}

fn units(s: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the program's NUL-terminated string.
    while unsafe { s.add(n).read() } != 0 {
        n += 1;
    }
    n
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysAllocString(s: *const u16) -> *mut u16 {
    if s.is_null() {
        return core::ptr::null_mut();
    }
    bstr(s.cast(), units(s) * 2)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysAllocStringLen(s: *const u16, len: u32) -> *mut u16 {
    bstr(s.cast(), len as usize * 2)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysAllocStringByteLen(s: *const u8, len: u32) -> *mut u16 {
    bstr(s, len as usize)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysFreeString(b: *mut u16) {
    if !b.is_null() {
        // SAFETY: a BSTR, its block starting at its count.
        crate::heap::free(unsafe { b.cast::<u8>().sub(PREFIX) });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysStringByteLen(b: *const u16) -> u32 {
    // SAFETY: a BSTR, its count just before it.
    if b.is_null() { 0 } else { unsafe { b.cast::<u8>().sub(PREFIX).cast::<u32>().read() } }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysStringLen(b: *const u16) -> u32 {
    AtlasSysStringByteLen(b) / 2
}

/// `SysReAllocStringLen(&bstr, s, len)`: a new BSTR in place of the old (which `s` may be part of).
#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysReAllocStringLen(at: *mut *mut u16, s: *const u16, len: u32) -> i32 {
    if at.is_null() {
        return 0;
    }
    let new = AtlasSysAllocStringLen(s, len);
    if new.is_null() {
        return 0;
    }
    // SAFETY: the program's BSTR pointer.
    unsafe { AtlasSysFreeString(at.replace(new)) };
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasSysReAllocString(at: *mut *mut u16, s: *const u16) -> i32 {
    let len = if s.is_null() { 0 } else { units(s) as u32 };
    AtlasSysReAllocStringLen(at, s, len)
}

/// Calls an interface's `AddRef` (slot 1) or `Release` (slot 2).
fn unknown(object: *mut c_void, slot: usize) {
    if object.is_null() {
        return;
    }
    // SAFETY: a COM object, whose first word is its vtable, with IUnknown's three methods first.
    unsafe {
        let vtable = object.cast::<*const usize>().read();
        let method: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vtable.add(slot).read());
        method(object);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasVariantInit(v: *mut u8) {
    // SAFETY: the program's VARIANT: its type and reserved words.
    unsafe { core::ptr::write_bytes(v, 0, 8) }
}

/// `VariantClear(&variant)`: what it owns freed (a BSTR) or released (an interface), and VT_EMPTY.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasVariantClear(v: *mut u8) -> u32 {
    // SAFETY: the program's VARIANT.
    unsafe {
        let vt = v.cast::<u16>().read_unaligned();
        let data = v.add(8).cast::<*mut c_void>().read_unaligned();
        if vt & VT_BYREF == 0 {
            match vt {
                VT_BSTR => AtlasSysFreeString(data.cast()),
                VT_UNKNOWN | VT_DISPATCH => unknown(data, 2),
                _ => {}
            }
        }
        core::ptr::write_bytes(v, 0, 8);
    }
    0
}

/// `VariantCopy(&dest, &src)`: `dest` cleared, then a copy of `src` that owns its own BSTR or reference.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasVariantCopy(dest: *mut u8, src: *const u8) -> u32 {
    if core::ptr::eq(dest, src) {
        return 0;
    }
    AtlasVariantClear(dest);
    // SAFETY: the program's VARIANTs.
    unsafe {
        core::ptr::copy_nonoverlapping(src, dest, VARIANT);
        let vt = src.cast::<u16>().read_unaligned();
        let data = src.add(8).cast::<*mut c_void>().read_unaligned();
        if vt & VT_BYREF == 0 {
            match vt {
                VT_BSTR if !data.is_null() => {
                    let copy = bstr(data.cast(), AtlasSysStringByteLen(data.cast()) as usize);
                    if copy.is_null() {
                        core::ptr::write_bytes(dest, 0, 8);
                        return E_OUTOFMEMORY;
                    }
                    dest.add(8).cast::<*mut u16>().write_unaligned(copy);
                }
                VT_UNKNOWN | VT_DISPATCH => unknown(data, 1),
                _ => {}
            }
        }
    }
    0
}

/// `PropVariantClear(&value)`: a PROPVARIANT's string (VT_LPSTR, VT_LPWSTR: from `CoTaskMemAlloc`) freed, or what
/// a VARIANT owns, then VT_EMPTY.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasPropVariantClear(v: *mut u8) -> u32 {
    const VT_LPSTR: u16 = 30;
    const VT_LPWSTR: u16 = 31;
    if v.is_null() {
        return 0;
    }
    // SAFETY: the program's PROPVARIANT.
    unsafe {
        let vt = v.cast::<u16>().read_unaligned();
        if matches!(vt, VT_LPSTR | VT_LPWSTR) {
            crate::heap::free(v.add(8).cast::<*mut u8>().read_unaligned());
            core::ptr::write_bytes(v, 0, 8);
            return 0;
        }
    }
    AtlasVariantClear(v)
}
