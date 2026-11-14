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

// ---- IMalloc ----

/// The task allocator as an `IMalloc`, which `CoGetMalloc` gives: one object for the process, as Windows' is, that
/// `AddRef` and `Release` don't count (it's never freed), its methods the `CoTaskMem*` functions.
#[repr(C)]
struct Malloc {
    vtable: &'static MallocVtable,
}

#[repr(C)]
struct MallocVtable {
    query_interface: extern "system" fn(*mut Malloc, *const [u8; 16], *mut *mut Malloc) -> u32,
    add_ref: extern "system" fn(*mut Malloc) -> u32,
    release: extern "system" fn(*mut Malloc) -> u32,
    alloc: extern "system" fn(*mut Malloc, usize) -> *mut u8,
    realloc: extern "system" fn(*mut Malloc, *mut u8, usize) -> *mut u8,
    free: extern "system" fn(*mut Malloc, *mut u8),
    get_size: extern "system" fn(*mut Malloc, *mut u8) -> usize,
    did_alloc: extern "system" fn(*mut Malloc, *mut u8) -> i32,
    heap_minimize: extern "system" fn(*mut Malloc),
}

/// IID_IUnknown and IID_IMalloc: {00000000-…} and {00000002-0000-0000-C000-000000000046}.
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IMALLOC: [u8; 16] = [2, 0, 0, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];

extern "system" fn malloc_query_interface(this: *mut Malloc, iid: *const [u8; 16], out: *mut *mut Malloc) -> u32 {
    if out.is_null() {
        return 0x8000_4003; // E_POINTER
    }
    // SAFETY: the program's IID and out pointer.
    unsafe {
        if !iid.is_null() && (*iid == IID_IUNKNOWN || *iid == IID_IMALLOC) {
            out.write(this);
            return 0;
        }
        out.write(core::ptr::null_mut());
    }
    0x8000_4002 // E_NOINTERFACE
}

extern "system" fn malloc_count(_: *mut Malloc) -> u32 {
    1
}

extern "system" fn malloc_alloc(_: *mut Malloc, size: usize) -> *mut u8 {
    AtlasCoTaskMemAlloc(size)
}

extern "system" fn malloc_realloc(_: *mut Malloc, p: *mut u8, size: usize) -> *mut u8 {
    AtlasCoTaskMemRealloc(p, size)
}

extern "system" fn malloc_free(_: *mut Malloc, p: *mut u8) {
    if !p.is_null() {
        AtlasCoTaskMemFree(p);
    }
}

/// `GetSize`: what the block was asked for, or -1 for NULL, as Windows has it.
extern "system" fn malloc_get_size(_: *mut Malloc, p: *mut u8) -> usize {
    if p.is_null() { usize::MAX } else { crate::heap::size(p) }
}

/// `DidAlloc`: -1, "can't tell", which Windows' own allocator may say too.
extern "system" fn malloc_did_alloc(_: *mut Malloc, p: *mut u8) -> i32 {
    let _ = p;
    -1
}

extern "system" fn malloc_heap_minimize(_: *mut Malloc) {}

static MALLOC_VTABLE: MallocVtable = MallocVtable {
    query_interface: malloc_query_interface,
    add_ref: malloc_count,
    release: malloc_count,
    alloc: malloc_alloc,
    realloc: malloc_realloc,
    free: malloc_free,
    get_size: malloc_get_size,
    did_alloc: malloc_did_alloc,
    heap_minimize: malloc_heap_minimize,
};

struct SyncMalloc(core::cell::UnsafeCell<Malloc>);
// SAFETY: the object is never written after it's made; its methods keep nothing of their own.
unsafe impl Sync for SyncMalloc {}
static MALLOC: SyncMalloc = SyncMalloc(core::cell::UnsafeCell::new(Malloc { vtable: &MALLOC_VTABLE }));

/// `CoGetMalloc(context, &malloc)`: the task allocator (MEMCTX_TASK, 1, the only context there is).
#[unsafe(no_mangle)]
pub extern "system" fn AtlasCoGetMalloc(context: u32, out: *mut *mut Malloc) -> u32 {
    if out.is_null() {
        return 0x8007_0057; // E_INVALIDARG
    }
    // SAFETY: the program's out pointer.
    unsafe { out.write(if context == 1 { MALLOC.0.get() } else { core::ptr::null_mut() }) };
    if context == 1 { 0 } else { 0x8007_0057 }
}
