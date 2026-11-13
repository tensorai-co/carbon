//! ole32's task allocator: `CoTaskMemAlloc`, `CoTaskMemRealloc` and `CoTaskMemFree`, on the process heap as the C
//! runtime's `malloc` is. They are stdcall, unlike `malloc`, so they're functions of their own rather than forwards.

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCoTaskMemAlloc(size: usize) -> *mut u8 {
    crate::heap::allocate(size.max(1), false)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCoTaskMemRealloc(p: *mut u8, size: usize) -> *mut u8 {
    crate::crt::AtlasCrtRealloc(p, size)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasCoTaskMemFree(p: *mut u8) {
    crate::heap::free(p);
}
