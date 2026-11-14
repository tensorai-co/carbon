//! The loader: what Windows' ntdll does to start a process and load DLLs, inside the program. atlas maps the program
//! and ntdll, and maps each DLL as the loader asks (relocating it, as NT's kernel does, and following API sets to
//! their DLL); the loader resolves imports and exports (forwarders too), sets up static TLS, and runs TLS callbacks
//! and each DLL's `DllMain`, dependencies first.
//!
//! It keeps the PEB's `PEB_LDR_DATA` as Windows does, with an `LDR_DATA_TABLE_ENTRY` for each module on its three
//! lists, since programs walk them themselves. A DLL loaded at run time counts its references (`LoadLibrary`, and
//! each module loaded at run time that imports it), and `FreeLibrary` that takes the last one away detaches it and
//! whatever only it held, and unmaps them. What the process started with stays. DLLs hear about threads ending and
//! the process ending (`DLL_THREAD_DETACH`, `DLL_PROCESS_DETACH`), in the reverse of the order they started.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use atlas_abi::{ldr, peb, status, teb};

use crate::pe::{self, Export, Image, ORDINAL, PTR, Wanted, cstr, ptr_at, put_ptr, u16_at, u32_at};
use crate::strings::{Message, Name};
use crate::sys;

const MOST_MODULES: usize = 128;
const MOST_TLS: usize = 64;
const MOST_DEPS: usize = 16;
const MOST_NOTIFICATIONS: usize = 8;
/// The most threads whose TEBs the loader keeps, to give a DLL loaded later its static TLS on each.
const MOST_THREADS: usize = 1024;
const DLL_PROCESS_DETACH: u32 = 0;
const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_THREAD_ATTACH: u32 = 2;
const DLL_THREAD_DETACH: u32 = 3;
const PAGE_READWRITE: u32 = 4;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_PROC_NOT_FOUND: u32 = 127;
const ERROR_DLL_INIT_FAILED: u32 = 1114;
/// A module that's never unloaded: what the process started with, atlas's own DLLs, and anything pinned.
const PINNED: u32 = u32::MAX;
/// Where atlas's own DLLs go, which are never unmapped.
const SYSTEM_BASE: u64 = atlas_abi::call::ATLAS_PAGE + 0x1_0000;
/// What room each LDR_DATA_TABLE_ENTRY has after it for its two names.
const FULL_NAME_ROOM: u64 = 520;
const BASE_NAME_ROOM: u64 = 128;

#[derive(Clone, Copy)]
pub struct Module {
    pub base: u64,
    pub size: u64,
    pub name: Name,
    pub path: [u16; 260],
    /// Its `DllMain` has been called with DLL_PROCESS_ATTACH (or it isn't a DLL, or is ntdll).
    pub ready: bool,
    /// Loaded at start, rather than by `LoadLibrary`.
    pub static_load: bool,
    /// Its static TLS index and TLS directory, if it has one.
    pub tls: Option<(usize, u64)>,
    /// References to it, or `PINNED`.
    count: u32,
    /// When its imports were resolved, counting from 1: the order modules are initialized in. 0 while it's loading.
    order: u32,
    /// Modules it holds a reference to, as their bases: what it imports, if it was loaded at run time.
    deps: [u64; MOST_DEPS],
    dep_count: usize,
    /// Its LDR_DATA_TABLE_ENTRY, in the program's memory.
    entry: u64,
    /// `DisableThreadLibraryCalls`: no DLL_THREAD_ATTACH or DLL_THREAD_DETACH.
    no_thread_calls: bool,
}

struct Table {
    modules: [Module; MOST_MODULES],
    count: usize,
    /// Static TLS: each module's index, given out in turn.
    tls_count: usize,
    /// The next initialization order to give.
    next_order: u32,
    /// The PEB's PEB_LDR_DATA.
    ldr: u64,
    /// `LdrRegisterDllNotification`'s callbacks and their contexts; a cookie is its slot plus one.
    notifications: [(u64, u64); MOST_NOTIFICATIONS],
    /// Every thread's TEB (0 where a slot is free): a DLL with static TLS loaded later gives each its block.
    threads: [u64; MOST_THREADS],
}

struct Loader {
    busy: AtomicBool,
    table: UnsafeCell<Table>,
}

// SAFETY: `table` is only touched while `busy` is held.
unsafe impl Sync for Loader {}

const EMPTY: Module = Module {
    base: 0,
    size: 0,
    name: Name::EMPTY,
    path: [0; 260],
    ready: false,
    static_load: false,
    tls: None,
    count: 0,
    order: 0,
    deps: [0; MOST_DEPS],
    dep_count: 0,
    entry: 0,
    no_thread_calls: false,
};
static LOADER: Loader = Loader {
    busy: AtomicBool::new(false),
    table: UnsafeCell::new(Table { modules: [EMPTY; MOST_MODULES], count: 0, tls_count: 0, next_order: 1, ldr: 0, notifications: [(0, 0); MOST_NOTIFICATIONS], threads: [0; MOST_THREADS] }),
};
/// Set once the process has begun to end: DLLs have been, or are being, told.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Runs `f` with the module table. It mustn't call anything that loads, or call into a module.
fn with<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    while LOADER.busy.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: held exclusively, as `busy` says.
    let result = f(unsafe { &mut *LOADER.table.get() });
    LOADER.busy.store(false, Ordering::Release);
    result
}

impl Table {
    fn by_base(&mut self, base: u64) -> Option<&mut Module> {
        self.modules[..self.count].iter_mut().find(|m| m.base == base)
    }
}

fn find(name: &Name) -> Option<u64> {
    with(|t| t.modules[..t.count].iter().find(|m| m.name == *name).map(|m| m.base))
}

/// The module `addr` is in: its base and size.
/// The module an address is in: its name, and the address's offset in it.
pub fn whose(addr: u64) -> Option<(Name, u64)> {
    with(|t| t.modules[..t.count].iter().find(|m| addr >= m.base && addr < m.base + m.size).map(|m| (m.name, addr - m.base)))
}

pub fn containing(addr: u64) -> Option<(u64, u64)> {
    with(|t| t.modules[..t.count].iter().find(|m| addr >= m.base && addr < m.base + m.size).map(|m| (m.base, m.size)))
}

// ---- The PEB's lists ----

/// # Safety
///
/// `at` is writable.
unsafe fn put32(at: u64, v: u32) {
    // SAFETY: as the caller says.
    unsafe { core::ptr::write_unaligned(at as *mut u32, v) }
}

/// # Safety
///
/// `at` is writable.
unsafe fn put16(at: u64, v: u16) {
    // SAFETY: as the caller says.
    unsafe { core::ptr::write_unaligned(at as *mut u16, v) }
}

/// A LIST_ENTRY that's empty: pointing at itself both ways.
///
/// # Safety
///
/// `head` is a writable LIST_ENTRY.
unsafe fn list_init(head: u64) {
    // SAFETY: as the caller says.
    unsafe {
        put_ptr(head, head);
        put_ptr(head + PTR, head);
    }
}

/// Adds `links` at the end of the list at `head`.
///
/// # Safety
///
/// Both are writable LIST_ENTRYs, and `head`'s list is well formed.
unsafe fn list_append(head: u64, links: u64) {
    // SAFETY: as the caller says.
    unsafe {
        let last = ptr_at(head + PTR);
        put_ptr(links, head);
        put_ptr(links + PTR, last);
        put_ptr(last, links);
        put_ptr(head + PTR, links);
    }
}

/// Takes `links` out of whatever list it's in, if it's in one.
///
/// # Safety
///
/// `links` is a writable LIST_ENTRY that's empty or in a well-formed list.
unsafe fn list_remove(links: u64) {
    // SAFETY: as the caller says.
    unsafe {
        let (next, prev) = (ptr_at(links), ptr_at(links + PTR));
        if next == 0 || next == links {
            return;
        }
        put_ptr(prev, next);
        put_ptr(next + PTR, prev);
        list_init(links);
    }
}

/// Makes the PEB's PEB_LDR_DATA, with its three lists empty.
fn ldr_init() {
    let at = crate::heap::allocate(ldr::DATA_SIZE as usize, true) as u64;
    if at == 0 {
        return;
    }
    // SAFETY: the block just allocated, and this process's PEB.
    unsafe {
        put32(at, ldr::DATA_SIZE as u32);
        *((at + ldr::DATA_INITIALIZED) as *mut u8) = 1;
        for head in [ldr::IN_LOAD_ORDER, ldr::IN_MEMORY_ORDER, ldr::IN_INITIALIZATION_ORDER] {
            list_init(at + head);
        }
        put_ptr(crate::peb() as u64 + peb::LDR, at);
    }
    with(|t| t.ldr = at);
}

/// A module's LDR_DATA_TABLE_ENTRY, its names after it, on the load and memory order lists: its address, or 0.
fn ldr_entry(ldr_at: u64, base: u64, path: &[u16; 260], count: u32, static_load: bool) -> u64 {
    if ldr_at == 0 {
        return 0;
    }
    let at = crate::heap::allocate((ldr::ENTRY_SIZE + FULL_NAME_ROOM + BASE_NAME_ROOM) as usize, true) as u64;
    if at == 0 {
        return 0;
    }
    let image = Image(base);
    let len = path.iter().position(|&c| c == 0).unwrap_or(259);
    let file = path[..len].iter().rposition(|&c| c == u16::from(b'\\') || c == u16::from(b'/')).map_or(0, |i| i + 1);
    let base_len = (len - file).min(BASE_NAME_ROOM as usize / 2 - 1);
    let (full_at, base_name_at) = (at + ldr::ENTRY_SIZE, at + ldr::ENTRY_SIZE + FULL_NAME_ROOM);
    // SAFETY: the entry just allocated, with room for both names; the module's mapped headers.
    unsafe {
        core::ptr::copy_nonoverlapping(path.as_ptr(), full_at as *mut u16, len);
        core::ptr::copy_nonoverlapping(path[file..].as_ptr(), base_name_at as *mut u16, base_len);
        put_ptr(at + ldr::DLL_BASE, base);
        put_ptr(at + ldr::ENTRY_POINT, image.entry().unwrap_or(0));
        put32(at + ldr::SIZE_OF_IMAGE, image.size() as u32);
        for (string, buffer, units) in [(ldr::FULL_DLL_NAME, full_at, len), (ldr::BASE_DLL_NAME, base_name_at, base_len)] {
            put16(at + string, (units * 2) as u16);
            put16(at + string + 2, (units * 2 + 2) as u16);
            put_ptr(at + string + PTR, buffer);
        }
        put32(at + ldr::FLAGS, if image.is_dll() { ldr::IMAGE_DLL } else { 0 });
        put32(at + ldr::TIME_DATE_STAMP, image.time_date_stamp());
        put_ptr(at + ldr::ORIGINAL_BASE, image.preferred_base());
        put32(at + ldr::LOAD_REASON, if static_load { ldr::REASON_STATIC } else { ldr::REASON_DYNAMIC });
        list_init(at + ldr::INITIALIZATION_LINKS);
        list_append(ldr_at + ldr::IN_LOAD_ORDER, at + ldr::LOAD_LINKS);
        list_append(ldr_at + ldr::IN_MEMORY_ORDER, at + ldr::MEMORY_LINKS);
    }
    set_counts(at, count);
    at
}

/// An entry's reference counts, as Windows keeps them: the old 16-bit count is 0xffff for a pinned module.
fn set_counts(entry: u64, count: u32) {
    if entry == 0 {
        return;
    }
    // SAFETY: an entry the loader made.
    unsafe {
        put16(entry + ldr::OBSOLETE_LOAD_COUNT, if count == PINNED { 0xffff } else { count.min(0xfffe) as u16 });
        put32(entry + ldr::REFERENCE_COUNT, count);
    }
}

/// Adds a module to the table and the PEB's lists, with a reference count.
fn add(base: u64, name: Name, path: [u16; 260], static_load: bool, count: u32) {
    with(|t| {
        if t.count < MOST_MODULES {
            let entry = ldr_entry(t.ldr, base, &path, count, static_load);
            t.modules[t.count] = Module { base, size: Image(base).size(), name, path, static_load, count, entry, ..EMPTY };
            t.count += 1;
        }
    });
}

/// Gives a module the next place in initialization order: its imports are resolved.
fn loaded(base: u64) {
    with(|t| {
        let order = t.next_order;
        if let Some(m) = t.by_base(base) {
            m.order = order;
            t.next_order += 1;
        }
    });
}

fn fail(what: &[u8], name: &[u8], status: u32) -> u32 {
    Message::new().text(b"atlas-guest: ").text(what).text(b" ").text(name).text(b" (").hex(u64::from(status)).text(b")\n").print();
    status
}

// ---- Loading ----

/// Why a module is being loaded, which says what reference it gets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Why {
    /// What the process starts with: pinned.
    Static,
    /// `LoadLibrary`: a reference of the caller's.
    Dynamic,
    /// An import of a module loaded at run time, which holds a reference to it.
    Import(u64),
    /// A forwarder's DLL: pinned if this loads it, since nothing would let it go.
    Forward,
}

/// Takes a reference to a module already loaded, for `why`.
fn reference(base: u64, why: Why) {
    with(|t| {
        let Some(m) = t.by_base(base) else { return };
        if m.count == PINNED {
            return;
        }
        match why {
            Why::Static => m.count = PINNED,
            Why::Dynamic => m.count += 1,
            Why::Forward => {}
            Why::Import(parent) => {
                m.count += 1;
                let (entry, count) = (m.entry, m.count);
                set_counts(entry, count);
                let pinned = match t.by_base(parent) {
                    Some(p) if p.dep_count < MOST_DEPS => {
                        p.deps[p.dep_count] = base;
                        p.dep_count += 1;
                        false
                    }
                    _ => true,
                };
                if pinned && let Some(m) = t.by_base(base) {
                    m.count = PINNED;
                    set_counts(m.entry, PINNED);
                }
                return;
            }
        }
        set_counts(m.entry, m.count);
    });
}

/// The ASCII of a UTF-16 path, for a module's name.
fn ascii_name(path: &[u16; 260]) -> Name {
    let mut ascii = [0u8; 260];
    let n = path.iter().position(|&c| c == 0).unwrap_or(0);
    for i in 0..n {
        ascii[i] = if path[i] < 128 { path[i] as u8 } else { b'?' };
    }
    Name::module(&ascii[..n])
}

/// Loads a DLL and what it imports: its base. `Why::Static` is for a process's start, where any failure ends it. A
/// DLL loaded for anything else that fails to load is let go of again.
fn load(raw: &[u8], why: Why, depth: usize) -> Result<u64, u32> {
    let static_load = why == Why::Static;
    if let Some(base) = find(&Name::module(raw)) {
        reference(base, why);
        return Ok(base);
    }
    if depth > 32 {
        return Err(status::DLL_NOT_FOUND);
    }
    let mut path = [0u16; 260];
    let base = sys::map_image(raw, &mut path).map_err(|s| if static_load { fail(b"can't find", raw, s) } else { s })?;
    // The same module by another name: an API set's, or a path.
    if containing(base).is_some_and(|(b, _)| b == base) {
        reference(base, why);
        return Ok(base);
    }
    let count = if matches!(why, Why::Static | Why::Forward) || base >= SYSTEM_BASE { PINNED } else { 1 };
    add(base, ascii_name(&path), path, static_load, count);
    if let Why::Import(parent) = why {
        // The parent holds the reference this one starts with.
        with(|t| {
            if let Some(p) = t.by_base(parent)
                && p.dep_count < MOST_DEPS
            {
                p.deps[p.dep_count] = base;
                p.dep_count += 1;
            } else if let Some(m) = t.by_base(base) {
                m.count = PINNED;
                set_counts(m.entry, PINNED);
            }
        });
    }
    let children = if static_load { Why::Static } else { Why::Import(base) };
    if let Err(e) = resolve_imports(Image(base), children, static_load, depth) {
        if !static_load {
            release(base);
        }
        return Err(e);
    }
    set_up_tls(Image(base));
    loaded(base);
    notify(base, 1);
    Ok(base)
}

/// Fills in a module's import address table, loading each DLL it imports for `why`.
fn resolve_imports(image: Image, why: Why, static_load: bool, depth: usize) -> Result<(), u32> {
    let Some((imports, _)) = image.dir(pe::DIR_IMPORT) else { return Ok(()) };
    // The table is usually read-only: open it while it's filled in.
    let iat = image.dir(pe::DIR_IAT);
    let old = iat.and_then(|(at, size)| sys::protect(at, size, PAGE_READWRITE));
    let mut result = Ok(());
    let mut descriptor = imports;
    // SAFETY: the import directory and its tables, in a mapped image.
    unsafe {
        loop {
            let (lookup, name_rva, iat_rva) = (u32_at(descriptor), u32_at(descriptor + 12), u32_at(descriptor + 16));
            if name_rva == 0 && iat_rva == 0 {
                break;
            }
            let dll = cstr(image.0 + u64::from(name_rva));
            let dll_base = match load(dll, why, depth + 1) {
                Ok(base) => base,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            };
            let lookup = image.0 + u64::from(if lookup != 0 { lookup } else { iat_rva });
            let mut slot = image.0 + u64::from(iat_rva);
            let mut entry = lookup;
            loop {
                let thunk = ptr_at(entry);
                if thunk == 0 {
                    break;
                }
                let wanted = if thunk & ORDINAL != 0 { Wanted::Ordinal(thunk as u16) } else { Wanted::Name(cstr(image.0 + (thunk & 0x7fff_ffff) + 2)) };
                match resolve(dll_base, wanted, depth) {
                    Some(address) => {
                        if iat.is_none() {
                            sys::protect(slot, PTR, PAGE_READWRITE);
                        }
                        core::ptr::write_volatile(slot as usize as *mut usize, address as usize);
                    }
                    None => {
                        let shown = match wanted {
                            Wanted::Name(n) => n,
                            Wanted::Ordinal(_) => b"(an ordinal)",
                        };
                        if static_load {
                            // What's missing, from which DLL, for which module.
                            let mut m = Message::new();
                            m.text(b"atlas-guest: no export ").text(shown).text(b" in ").text(dll);
                            if let Some((importer, _)) = whose(image.0) {
                                m.text(b", for ").text(importer.as_bytes());
                            }
                            m.text(b"\n").print();
                        }
                        result = Err(status::ENTRYPOINT_NOT_FOUND);
                        break;
                    }
                }
                entry += PTR;
                slot += PTR;
            }
            if result.is_err() {
                break;
            }
            descriptor += 20;
        }
    }
    if let (Some((at, size)), Some(old)) = (iat, old) {
        sys::protect(at, size, old);
    }
    result
}

/// An export's address, following forwarders (`dll.name`, `dll.#ordinal`) to the end.
fn resolve(base: u64, wanted: Wanted<'_>, depth: usize) -> Option<u64> {
    match pe::export(Image(base), wanted)? {
        Export::Address(address) => Some(address),
        Export::Forward(text) => {
            let dot = text.iter().rposition(|&b| b == b'.')?;
            let (dll, name) = (&text[..dot], &text[dot + 1..]);
            let target = load(dll, Why::Forward, depth + 1).ok()?;
            let wanted = match name.strip_prefix(b"#") {
                Some(digits) => Wanted::Ordinal(digits.iter().try_fold(0u16, |n, &d| (d as char).to_digit(10).map(|d| n * 10 + d as u16))?),
                None => Wanted::Name(name),
            };
            resolve(target, wanted, depth + 1)
        }
    }
}

// ---- Static TLS ----

/// Static TLS: gives a module with a TLS directory its index, and this thread its block.
fn set_up_tls(image: Image) {
    let Some((dir, _)) = image.dir(pe::DIR_TLS) else { return };
    let Some(index) = with(|t| {
        if t.tls_count >= MOST_TLS {
            return None;
        }
        t.tls_count += 1;
        let index = t.tls_count - 1;
        if let Some(m) = t.by_base(image.0) {
            m.tls = Some((index, dir));
            if m.entry != 0 {
                // SAFETY: an entry the loader made.
                unsafe { put16(m.entry + ldr::TLS_INDEX, index as u16) };
            }
        }
        Some(index)
    }) else {
        return;
    };
    // SAFETY: the TLS directory's index field, in a mapped image.
    unsafe {
        let index_at = ptr_at(dir + 2 * PTR);
        if index_at != 0 {
            core::ptr::write_volatile(index_at as *mut u32, index as u32);
        }
    }
    // Every thread there is gets its block, as Windows gives a DLL loaded later its TLS on each: under the loader's
    // lock, so none ends meanwhile or starts without seeing it (a thread starting gives itself the blocks it lacks).
    with(|t| {
        for &teb in t.threads.iter().filter(|&&teb| teb != 0) {
            tls_block_of(teb, index, dir);
        }
    });
    tls_block(index, dir);
}

/// A thread's static TLS slots, by its TEB, made if it has none yet.
fn tls_slots_of(teb: u64) -> *mut usize {
    // SAFETY: a live thread's TEB, which the loader's lock keeps from ending, or this thread's.
    unsafe {
        let slots = (teb as usize as *mut u8).add(teb::THREAD_LOCAL_STORAGE as usize).cast::<*mut usize>();
        if (*slots).is_null() {
            *slots = crate::heap::allocate(MOST_TLS * PTR as usize, true).cast();
        }
        *slots
    }
}

/// This thread's block for one module's static TLS, unless it has one: a copy of its template, then zeros.
fn tls_block(index: usize, dir: u64) {
    tls_block_of(crate::teb() as u64, index, dir);
}

/// A thread's block for one module's static TLS, by its TEB, unless it has one already.
fn tls_block_of(teb: u64, index: usize, dir: u64) {
    // SAFETY: the TLS directory and its template in a mapped image, and the thread's slots.
    unsafe {
        let slot = tls_slots_of(teb).add(index);
        if *slot != 0 {
            return;
        }
        let (start, end) = (ptr_at(dir), ptr_at(dir + PTR));
        let zero_fill = u64::from(u32_at(dir + 4 * PTR));
        let block = crate::heap::allocate(((end - start + zero_fill) as usize).max(16), true);
        core::ptr::copy_nonoverlapping(start as *const u8, block, (end - start) as usize);
        *slot = (block as u64) as usize;
    }
}

/// This thread, among those a DLL loaded later gives static TLS to.
fn register_thread() {
    let teb = crate::teb() as u64;
    with(|t| {
        if !t.threads.contains(&teb)
            && let Some(free) = t.threads.iter_mut().find(|s| **s == 0)
        {
            *free = teb;
        }
    });
}

/// Runs `f` on every thread's TEB there is, under the loader's lock, so none ends meanwhile: how `TlsFree` clears
/// a slot in every thread.
pub fn for_each_thread(mut f: impl FnMut(u64)) {
    with(|t| {
        for &teb in t.threads.iter().filter(|&&teb| teb != 0) {
            f(teb);
        }
    });
}

/// This thread, ending: no DLL loaded from now on gives it TLS.
fn unregister_thread() {
    let teb = crate::teb() as u64;
    with(|t| {
        if let Some(slot) = t.threads.iter_mut().find(|s| **s == teb) {
            *slot = 0;
        }
    });
}

/// Lets go of this thread's static TLS blocks, as it ends.
fn free_tls_blocks() {
    // SAFETY: this thread's TEB and slots, which nothing else uses.
    unsafe {
        let slots = crate::teb().add(teb::THREAD_LOCAL_STORAGE as usize).cast::<*mut usize>();
        if (*slots).is_null() {
            return;
        }
        for i in 0..MOST_TLS {
            let block = *(*slots).add(i);
            if block != 0 {
                crate::heap::free(block as *mut u8);
            }
        }
        crate::heap::free((*slots).cast());
        *slots = core::ptr::null_mut();
    }
}

fn run_tls_callbacks(image: Image, reason: u32) {
    let Some((dir, _)) = image.dir(pe::DIR_TLS) else { return };
    // SAFETY: the TLS directory's null-terminated list of callbacks.
    unsafe {
        let mut at = ptr_at(dir + 3 * PTR);
        if at == 0 {
            return;
        }
        loop {
            let callback = ptr_at(at);
            if callback == 0 {
                break;
            }
            let callback: extern "system" fn(usize, u32, usize) = core::mem::transmute(callback as usize);
            callback(image.0 as usize, reason, 0);
            at += PTR;
        }
    }
}

/// Calls a DLL's entry point, as Windows does: its result.
fn dll_main(base: u64, reason: u32, reserved: u64) -> bool {
    let Some(entry) = Image(base).entry() else { return true };
    // SAFETY: a DLL's entry point, as Windows calls it.
    let main: extern "system" fn(usize, u32, usize) -> i32 = unsafe { core::mem::transmute(entry as usize) };
    main(base as usize, reason, reserved as usize) != 0
}

// ---- Starting and ending ----

/// The modules that have started, newest first: for telling them a thread or the process is ending. With
/// `threads`, only those that want to hear about threads.
fn started_newest_first(out: &mut [u64; MOST_MODULES], threads: bool) -> usize {
    with(|t| {
        let mut list: [(u32, u64); MOST_MODULES] = [(0, 0); MOST_MODULES];
        let mut n = 0;
        for m in &t.modules[..t.count] {
            if m.ready && m.order != 0 && Image(m.base).is_dll() && m.name != Name::module(b"ntdll.dll") && !(threads && m.no_thread_calls) {
                list[n] = (m.order, m.base);
                n += 1;
            }
        }
        list[..n].sort_unstable_by_key(|m| core::cmp::Reverse(m.0));
        for (i, (_, base)) in list[..n].iter().enumerate() {
            out[i] = *base;
        }
        n
    })
}

fn exe() -> Image {
    // SAFETY: the PEB's image base.
    Image(unsafe { ptr_at(exe_base()) })
}

/// Where a new thread starts: it waits here if it was made suspended, gets its own static TLS, tells each DLL it's
/// started, then runs `start(parameter)` and ends with what it returns.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasThreadStart(start: usize, parameter: usize) -> ! {
    let (start, parameter) = (start as u64, parameter as u64);
    sys::call4(atlas_abi::call::THREAD_STARTED, 0, 0, 0, 0);
    register_thread();
    let mut at = 0;
    loop {
        let next = with(|t| {
            let m = t.modules[..t.count].get(at).copied();
            at += 1;
            m
        });
        let Some(m) = next else { break };
        if let Some((index, dir)) = m.tls {
            tls_block(index, dir);
        }
    }
    // Oldest first, as they started.
    let mut started = [0u64; MOST_MODULES];
    let n = started_newest_first(&mut started, true);
    for &base in started[..n].iter().rev() {
        run_tls_callbacks(Image(base), DLL_THREAD_ATTACH);
        dll_main(base, DLL_THREAD_ATTACH, 0);
    }
    run_tls_callbacks(exe(), DLL_THREAD_ATTACH);
    // SAFETY: the program's thread routine, as Windows calls it.
    let start: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(start as usize) };
    AtlasExitThread(start(parameter as usize))
}

/// `ExitThread`, `RtlExitUserThread`: the process's last thread ends the process, as Windows has it. Otherwise each
/// DLL hears DLL_THREAD_DETACH, newest first, and the thread lets go of its TLS and ends.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasExitThread(code: u32) -> ! {
    if sys::call4(atlas_abi::call::LAST_THREAD, 0, 0, 0, 0) != 0 {
        RtlExitUserProcess(code)
    }
    if !SHUTTING_DOWN.load(Ordering::Acquire) {
        // FLS's callbacks first, as Windows runs them: a C runtime's per-thread data goes there.
        crate::sync::release_thread_fls();
        run_tls_callbacks(exe(), DLL_THREAD_DETACH);
        let mut started = [0u64; MOST_MODULES];
        let n = started_newest_first(&mut started, true);
        for &base in &started[..n] {
            run_tls_callbacks(Image(base), DLL_THREAD_DETACH);
            dll_main(base, DLL_THREAD_DETACH, 0);
        }
        unregister_thread();
        free_tls_blocks();
        crate::sync::release_tls_expansion();
    }
    sys::call4(atlas_abi::call::EXIT_THREAD, u64::from(code), 0, 0, 0);
    sys::terminate(code)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlExitUserThread(code: u32) -> ! {
    AtlasExitThread(code)
}

/// `ExitProcess`, `RtlExitUserProcess`: every other thread ends, then each DLL hears DLL_PROCESS_DETACH (with a
/// non-null `lpReserved`, as for a process ending), newest first, and the process ends.
#[unsafe(no_mangle)]
pub extern "system" fn RtlExitUserProcess(code: u32) -> ! {
    // A DllMain that ends the process as it's told it's ending: it just ends.
    if SHUTTING_DOWN.swap(true, Ordering::AcqRel) {
        sys::terminate(code)
    }
    sys::call4(atlas_abi::call::TERMINATE_PROCESS, 0, u64::from(code), 0, 0);
    crate::sync::release_thread_fls();
    let ldr_at = with(|t| t.ldr);
    if ldr_at != 0 {
        // SAFETY: the PEB_LDR_DATA the loader made.
        unsafe { *((ldr_at + ldr::DATA_SHUTDOWN_IN_PROGRESS) as *mut u8) = 1 };
    }
    run_tls_callbacks(exe(), DLL_PROCESS_DETACH);
    let mut started = [0u64; MOST_MODULES];
    let n = started_newest_first(&mut started, false);
    for &base in &started[..n] {
        run_tls_callbacks(Image(base), DLL_PROCESS_DETACH);
        dll_main(base, DLL_PROCESS_DETACH, 1);
    }
    sys::terminate(code)
}

/// Runs each module that isn't yet, of those given an order from `first` on (what one load brought in), in the order
/// their imports were resolved (so dependencies first): its TLS callbacks, then `DllMain`. Each goes on the PEB's
/// initialization order list as it starts. False if a `DllMain` failed.
///
/// A `DllMain` that loads a library (as C runtimes' do) starts only what that load brought in: a module the outer
/// load is still to start waits for it, as on Windows, rather than starting before the DLLs it depends on are done.
fn attach_all(first: u32) -> bool {
    loop {
        let next = with(|t| {
            let ldr_at = t.ldr;
            let m = t.modules[..t.count].iter_mut().filter(|m| !m.ready && m.order >= first.max(1)).min_by_key(|m| m.order)?;
            m.ready = true;
            if m.entry != 0 && ldr_at != 0 {
                // SAFETY: an entry the loader made, and the PEB_LDR_DATA.
                unsafe { list_append(ldr_at + ldr::IN_INITIALIZATION_ORDER, m.entry + ldr::INITIALIZATION_LINKS) };
            }
            Some((m.base, m.static_load, m.entry))
        });
        let Some((base, static_load, entry)) = next else { return true };
        let image = Image(base);
        if !image.is_dll() {
            continue;
        }
        run_tls_callbacks(image, DLL_PROCESS_ATTACH);
        let ok = dll_main(base, DLL_PROCESS_ATTACH, u64::from(static_load));
        if entry != 0 {
            // SAFETY: an entry the loader made.
            unsafe { put32(entry + ldr::FLAGS, u32_at(entry + ldr::FLAGS) | ldr::ENTRY_PROCESSED | ldr::PROCESS_ATTACH_CALLED) };
        }
        if !ok {
            Message::new().text(b"atlas-guest: a DLL's DllMain failed, at ").hex(base).text(b"\n").print();
            return false;
        }
    }
}

/// Where the first thread starts: loads what the program imports, runs every DLL's start, then the program.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasProcessStart(peb_at: usize, ntdll: usize) -> ! {
    let (peb_at, ntdll) = (peb_at as u64, ntdll as u64);
    // A process made suspended waits here for `ResumeThread`, before anything of it has run.
    sys::call4(atlas_abi::call::THREAD_STARTED, 0, 0, 0, 0);
    ldr_init();
    // SAFETY: the PEB atlas made, and its process parameters' image path.
    let (exe, path) = unsafe {
        let exe = ptr_at(peb_at + peb::IMAGE_BASE);
        let params = ptr_at(peb_at + peb::PROCESS_PARAMETERS);
        let string = params + atlas_abi::params::IMAGE_PATH_NAME;
        let (len, buffer) = (u16_at(string) as usize / 2, ptr_at(string + PTR));
        let mut path = [0u16; 260];
        for (i, unit) in path.iter_mut().enumerate().take(len.min(259)) {
            *unit = u16_at(buffer + i as u64 * 2);
        }
        (exe, path)
    };
    // As on Windows, the program comes first in load order, then ntdll, then kernel32, whatever the program imports.
    register_thread();
    add(exe, ascii_name(&path), path, true, PINNED);
    let mut ntdll_path = [0u16; 260];
    for (i, b) in b"C:\\Windows\\System32\\ntdll.dll".iter().enumerate() {
        ntdll_path[i] = u16::from(*b);
    }
    add(ntdll, Name::module(b"ntdll.dll"), ntdll_path, true, PINNED);
    loaded(ntdll);
    let ldr_at = with(|t| t.ldr);
    with(|t| {
        if let Some(m) = t.by_base(ntdll) {
            m.ready = true;
            if m.entry != 0 && ldr_at != 0 {
                // SAFETY: as in `attach_all`.
                unsafe { list_append(ldr_at + ldr::IN_INITIALIZATION_ORDER, m.entry + ldr::INITIALIZATION_LINKS) };
            }
        }
        if let Some(m) = t.by_base(exe) {
            m.ready = true;
        }
    });
    if let Err(e) = load(b"kernel32.dll", Why::Static, 0) {
        sys::terminate(e);
    }
    if let Err(e) = resolve_imports(Image(exe), Why::Static, true, 0) {
        sys::terminate(e);
    }
    set_up_tls(Image(exe));
    if !attach_all(0) {
        sys::terminate(status::DLL_INIT_FAILED);
    }
    run_tls_callbacks(Image(exe), DLL_PROCESS_ATTACH);
    let Some(entry) = Image(exe).entry() else { sys::terminate(status::INVALID_IMAGE_FORMAT) };
    // SAFETY: the program's entry point, as Windows calls it.
    let main: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(entry as usize) };
    let code = main(peb_at as usize);
    RtlExitUserProcess(code)
}

// ---- Unloading ----

/// Lets go of a reference to a module: if it was the last, the module is detached and unmapped, with whatever only
/// it held. False if `base` isn't a module.
fn release(base: u64) -> bool {
    let mut gone = [(0u32, 0u64, false); MOST_MODULES];
    let mut n = 0;
    let known = with(|t| {
        if t.by_base(base).is_none() {
            return false;
        }
        let mut stack = [0u64; MOST_MODULES];
        stack[0] = base;
        let mut depth = 1;
        while depth > 0 {
            depth -= 1;
            let at = stack[depth];
            let Some(m) = t.by_base(at) else { continue };
            if m.count == PINNED || m.count == 0 {
                continue;
            }
            m.count -= 1;
            set_counts(m.entry, m.count);
            if m.count == 0 && n < MOST_MODULES {
                gone[n] = (m.order, m.base, m.ready);
                n += 1;
                for &dep in &m.deps[..m.dep_count] {
                    if depth < MOST_MODULES {
                        stack[depth] = dep;
                        depth += 1;
                    }
                }
            }
        }
        true
    });
    if !known {
        return false;
    }
    // Newest first: what holds a DLL hears it's ending before the DLL does.
    gone[..n].sort_unstable_by_key(|m| core::cmp::Reverse(m.0));
    for &(_, at, ready) in &gone[..n] {
        if ready && Image(at).is_dll() {
            run_tls_callbacks(Image(at), DLL_PROCESS_DETACH);
            dll_main(at, DLL_PROCESS_DETACH, 0);
        }
    }
    for &(_, at, _) in &gone[..n] {
        notify(at, 2);
        let removed = with(|t| {
            let i = t.modules[..t.count].iter().position(|m| m.base == at)?;
            let m = t.modules[i];
            t.modules.copy_within(i + 1..t.count, i);
            t.count -= 1;
            if m.entry != 0 {
                // SAFETY: an entry the loader made, on the PEB's lists.
                unsafe {
                    list_remove(m.entry + ldr::LOAD_LINKS);
                    list_remove(m.entry + ldr::MEMORY_LINKS);
                    list_remove(m.entry + ldr::INITIALIZATION_LINKS);
                }
            }
            Some(m)
        });
        if let Some(m) = removed {
            if m.entry != 0 {
                crate::heap::free(m.entry as *mut u8);
            }
            sys::call4(atlas_abi::call::UNMAP_IMAGE, m.base, m.size, 0, 0);
        }
    }
    true
}

// ---- Notifications ----

/// Tells `LdrRegisterDllNotification`'s callbacks a module was loaded (1) or is about to be unloaded (2).
fn notify(base: u64, reason: u32) {
    let (callbacks, entry, size) = with(|t| {
        let m = t.modules[..t.count].iter().find(|m| m.base == base);
        (t.notifications, m.map_or(0, |m| m.entry), m.map_or(0, |m| m.size))
    });
    if entry == 0 || callbacks.iter().all(|c| c.0 == 0) {
        return;
    }
    // LDR_DLL_NOTIFICATION_DATA: flags, the full and base names, the base and the size.
    let data: [usize; 5] = [0, (entry + ldr::FULL_DLL_NAME) as usize, (entry + ldr::BASE_DLL_NAME) as usize, base as usize, size as usize];
    for (callback, context) in callbacks {
        if callback != 0 {
            // SAFETY: the program's callback, as Windows calls it.
            let f: extern "system" fn(u32, *const usize, usize) = unsafe { core::mem::transmute(callback as usize) };
            f(reason, data.as_ptr(), context as usize);
        }
    }
}

/// `LdrRegisterDllNotification(flags, callback, context, &cookie)`.
#[unsafe(no_mangle)]
pub extern "system" fn LdrRegisterDllNotification(flags: u32, callback: usize, context: usize, cookie: *mut usize) -> u32 {
    let (callback, context) = (callback as u64, context as u64);
    if flags != 0 || callback == 0 || cookie.is_null() {
        return status::INVALID_PARAMETER;
    }
    let slot = with(|t| {
        let i = t.notifications.iter().position(|c| c.0 == 0)?;
        t.notifications[i] = (callback, context);
        Some(i)
    });
    match slot {
        Some(i) => {
            // SAFETY: the program's cookie.
            unsafe { cookie.write_unaligned((i as u64 + 1) as usize) };
            status::SUCCESS
        }
        None => status::NO_MEMORY,
    }
}

/// `LdrUnregisterDllNotification(cookie)`.
#[unsafe(no_mangle)]
pub extern "system" fn LdrUnregisterDllNotification(cookie: usize) -> u32 {
    let cookie = cookie as u64;
    with(|t| match t.notifications.get_mut((cookie as usize).wrapping_sub(1)) {
        Some(slot) if slot.0 != 0 => {
            *slot = (0, 0);
            status::SUCCESS
        }
        _ => status::INVALID_PARAMETER,
    })
}

// ---- kernel32's face ----

fn set_last_error(error: u32) {
    crate::RtlSetLastWin32Error(error);
}

/// A string the program passed, as ASCII: UTF-16 if `wide`, at most `limit` units if it says.
///
/// # Safety
///
/// `at` is a readable string, NUL-terminated or `limit` long.
unsafe fn read_name(at: u64, wide: bool, limit: usize, out: &mut [u8; 260]) -> usize {
    let mut n = 0;
    while n < 259 && n < limit {
        // SAFETY: as the caller says.
        let c = unsafe { if wide { u16_at(at + n as u64 * 2) } else { u16::from(*((at + n as u64) as *const u8)) } };
        if c == 0 {
            break;
        }
        out[n] = if c < 128 { c as u8 } else { b'?' };
        n += 1;
    }
    n
}

/// Loads a DLL for the program, and starts it: its base, or the status.
fn load_dynamic(name: &[u8]) -> Result<u64, u32> {
    let first = with(|t| t.next_order);
    let base = load(name, Why::Dynamic, 0)?;
    if attach_all(first) {
        Ok(base)
    } else {
        // Windows tells a DLL whose DllMain failed that it's detaching, and unloads it.
        release(base);
        Err(status::DLL_INIT_FAILED)
    }
}

fn load_library(at: u64, wide: bool) -> u64 {
    if at == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut name = [0u8; 260];
    // SAFETY: the program's string.
    let n = unsafe { read_name(at, wide, usize::MAX, &mut name) };
    match load_dynamic(&name[..n]) {
        Ok(base) => base,
        Err(status::DLL_INIT_FAILED) => {
            set_last_error(ERROR_DLL_INIT_FAILED);
            0
        }
        Err(_) => {
            set_last_error(ERROR_MOD_NOT_FOUND);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryA(name: usize) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        load_library(name, false)
    })();
    result as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryW(name: usize) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        load_library(name, true)
    })();
    result as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryExA(name: usize, _file: usize, _flags: u32) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        load_library(name, false)
    })();
    result as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryExW(name: usize, _file: usize, _flags: u32) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        load_library(name, true)
    })();
    result as usize
}

/// `FreeLibrary`: one reference fewer, and the DLL unloaded if that was the last.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasFreeLibrary(module: usize) -> i32 {
    let module = module as u64;
    if release(module) {
        1
    } else {
        set_last_error(ERROR_MOD_NOT_FOUND);
        0
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFreeLibraryAndExitThread(module: usize, code: u32) -> ! {
    let module = module as u64;
    release(module);
    AtlasExitThread(code)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasDisableThreadLibraryCalls(module: usize) -> i32 {
    let module = module as u64;
    let found = with(|t| t.by_base(module).map(|m| m.no_thread_calls = true).is_some());
    if !found {
        set_last_error(ERROR_INVALID_PARAMETER);
    }
    i32::from(found)
}

fn exe_base() -> u64 {
    // SAFETY: this thread's TEB and its PEB.
    unsafe { ptr_at(crate::teb() as u64 + teb::PEB) }.wrapping_add(peb::IMAGE_BASE)
}

fn module_handle(at: u64, wide: bool) -> u64 {
    if at == 0 {
        // SAFETY: the PEB's image base.
        return unsafe { ptr_at(exe_base()) };
    }
    let mut name = [0u8; 260];
    // SAFETY: the program's string.
    let n = unsafe { read_name(at, wide, usize::MAX, &mut name) };
    find(&Name::module(&name[..n])).unwrap_or_else(|| {
        set_last_error(ERROR_MOD_NOT_FOUND);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleA(name: usize) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        module_handle(name, false)
    })();
    result as usize
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleW(name: usize) -> usize {
    let name = name as u64;
    let result = (|| -> u64 {
        module_handle(name, true)
    })();
    result as usize
}

/// `GetModuleHandleEx(flags, name or address, &module)`: by name, or by an address in it
/// (GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS); with a reference taken, unless UNCHANGED_REFCOUNT, or pinned (PIN).
fn module_handle_ex(flags: u32, name: u64, out: *mut usize, wide: bool) -> i32 {
    const PIN: u32 = 1;
    const UNCHANGED_REFCOUNT: u32 = 2;
    const FROM_ADDRESS: u32 = 4;
    if out.is_null() || flags & !(PIN | UNCHANGED_REFCOUNT | FROM_ADDRESS) != 0 || flags & (PIN | UNCHANGED_REFCOUNT) == PIN | UNCHANGED_REFCOUNT {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let base = if flags & FROM_ADDRESS != 0 {
        containing(name).map_or(0, |(base, _)| base)
    } else {
        module_handle(name, wide)
    };
    // SAFETY: the program's out pointer.
    unsafe { out.write_unaligned(base as usize) };
    if base == 0 {
        set_last_error(ERROR_MOD_NOT_FOUND);
        return 0;
    }
    if flags & PIN != 0 {
        reference(base, Why::Static);
    } else if flags & UNCHANGED_REFCOUNT == 0 {
        reference(base, Why::Dynamic);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleExA(flags: u32, name: usize, out: *mut usize) -> i32 {
    let name = name as u64;
    module_handle_ex(flags, name, out, false)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleExW(flags: u32, name: usize, out: *mut usize) -> i32 {
    let name = name as u64;
    module_handle_ex(flags, name, out, true)
}

/// `RtlPcToFileHeader(pc, &base)`: the base of the module `pc` is in, also where `base` says; null for none.
#[unsafe(no_mangle)]
pub extern "system" fn RtlPcToFileHeader(pc: usize, base: *mut usize) -> usize {
    let found = containing(pc as u64).map_or(0, |(b, _)| b as usize);
    if !base.is_null() {
        // SAFETY: the program's pointer.
        unsafe { base.write(found) };
    }
    found
}

/// `RtlCaptureStackBackTrace(skip, count, &frames, &hash)`: no frames (a stack walk isn't kept for this yet), which
/// callers take as a trace they can't have.
#[unsafe(no_mangle)]
pub extern "system" fn RtlCaptureStackBackTrace(_skip: u32, _count: u32, _frames: *mut usize, hash: *mut u32) -> u16 {
    if !hash.is_null() {
        // SAFETY: the program's pointer.
        unsafe { hash.write(0) };
    }
    0
}

/// Traced, a lookup by name that found nothing: what the program will get NULL for, and may call anyway. The module
/// is by its base, as the host's `maps` lines give it.
fn traced_miss(base: u64, name: &[u8]) {
    if crate::sys::tracing() {
        Message::new().text(b"atlas-guest: no export ").text(name).text(b" in the module at ").hex(base).text(b"\n").print();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetProcAddress(module: usize, name: usize) -> usize {
    let (module, name) = (module as u64, name as u64);
    let result = (|| -> u64 {
        // SAFETY: the PEB's image base.
        let base = if module == 0 { unsafe { ptr_at(exe_base()) } } else { module };
        if containing(base).is_none() {
            set_last_error(ERROR_MOD_NOT_FOUND);
            return 0;
        }
        // An ordinal is a "pointer" below 64 KiB.
        // SAFETY: the program's string, if it isn't an ordinal.
        let wanted = if name < 0x1_0000 { Wanted::Ordinal(name as u16) } else { Wanted::Name(unsafe { cstr(name) }) };
        resolve(base, wanted, 0).unwrap_or_else(|| {
            if name >= 0x1_0000 {
                // SAFETY: as above.
                traced_miss(base, unsafe { cstr(name) });
            }
            set_last_error(ERROR_PROC_NOT_FOUND);
            0
        })
    })();
    result as usize
}

/// `GetModuleFileNameA`: the path in the ANSI code page.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleFileNameA(module: usize, buffer: *mut u8, size: u32) -> u32 {
    let module = module as u64;
    // SAFETY: the PEB's image base.
    let base = if module == 0 { unsafe { ptr_at(exe_base()) } } else { module };
    let Some(path) = with(|t| t.modules[..t.count].iter().find(|m| m.base == base).map(|m| m.path)) else {
        set_last_error(ERROR_MOD_NOT_FOUND);
        return 0;
    };
    let len = path.iter().position(|&c| c == 0).unwrap_or(0);
    if size == 0 {
        return 0;
    }
    let n = len.min(size as usize - 1);
    // SAFETY: the program's buffer, of `size` bytes.
    unsafe {
        for (i, &unit) in path[..n].iter().enumerate() {
            *buffer.add(i) = atlas_abi::ansi::to_1252(unit).unwrap_or(b'?');
        }
        *buffer.add(n) = 0;
    }
    n as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleFileNameW(module: usize, buffer: *mut u16, size: u32) -> u32 {
    let module = module as u64;
    // SAFETY: the PEB's image base.
    let base = if module == 0 { unsafe { ptr_at(exe_base()) } } else { module };
    let Some(path) = with(|t| t.modules[..t.count].iter().find(|m| m.base == base).map(|m| m.path)) else {
        set_last_error(ERROR_MOD_NOT_FOUND);
        return 0;
    };
    let len = path.iter().position(|&c| c == 0).unwrap_or(0);
    if size == 0 {
        return 0;
    }
    let n = len.min(size as usize - 1);
    // SAFETY: the program's buffer, of `size` units.
    unsafe {
        core::ptr::copy_nonoverlapping(path.as_ptr(), buffer, n);
        *buffer.add(n) = 0;
    }
    n as u32
}

// ---- ntdll's own: Ldr* ----

/// A UNICODE_STRING (or, not `wide`, an ANSI_STRING) the program passed, as ASCII.
///
/// # Safety
///
/// `string` is null or a readable counted string.
unsafe fn counted(string: u64, wide: bool, out: &mut [u8; 260]) -> Option<usize> {
    if string == 0 {
        return None;
    }
    // SAFETY: as the caller says.
    unsafe {
        let (bytes, buffer) = (usize::from(u16_at(string)), ptr_at(string + PTR));
        if buffer == 0 {
            return None;
        }
        Some(read_name(buffer, wide, if wide { bytes / 2 } else { bytes }, out))
    }
}

/// `LdrLoadDll(search path, &flags, name, &base)`.
#[unsafe(no_mangle)]
pub extern "system" fn LdrLoadDll(_path: usize, _flags: usize, name: usize, base: *mut usize) -> u32 {
    let name = name as u64;
    let mut raw = [0u8; 260];
    // SAFETY: the program's UNICODE_STRING.
    let Some(n) = (unsafe { counted(name, true, &mut raw) }) else { return status::INVALID_PARAMETER };
    if base.is_null() {
        return status::INVALID_PARAMETER;
    }
    match load_dynamic(&raw[..n]) {
        Ok(at) => {
            // SAFETY: the program's out pointer.
            unsafe { base.write_unaligned(at as usize) };
            status::SUCCESS
        }
        Err(code) => code,
    }
}

/// `LdrUnloadDll(base)`.
#[unsafe(no_mangle)]
pub extern "system" fn LdrUnloadDll(base: usize) -> u32 {
    let base = base as u64;
    if release(base) { status::SUCCESS } else { status::DLL_NOT_FOUND }
}

/// `LdrGetDllHandle(search path, characteristics, name, &base)`.
#[unsafe(no_mangle)]
pub extern "system" fn LdrGetDllHandle(_path: usize, _characteristics: usize, name: usize, base: *mut usize) -> u32 {
    let name = name as u64;
    let mut raw = [0u8; 260];
    // SAFETY: the program's UNICODE_STRING.
    let Some(n) = (unsafe { counted(name, true, &mut raw) }) else { return status::INVALID_PARAMETER };
    if base.is_null() {
        return status::INVALID_PARAMETER;
    }
    let found = find(&Name::module(&raw[..n]));
    // SAFETY: the program's out pointer.
    unsafe { base.write_unaligned((found.unwrap_or(0)) as usize) };
    if found.is_some() { status::SUCCESS } else { status::DLL_NOT_FOUND }
}

/// `LdrGetProcedureAddress(base, name, ordinal, &address)`: by an ANSI_STRING's name, or with none, the ordinal.
#[unsafe(no_mangle)]
pub extern "system" fn LdrGetProcedureAddress(base: usize, name: usize, ordinal: u32, address: *mut usize) -> u32 {
    let (base, name) = (base as u64, name as u64);
    if address.is_null() {
        return status::INVALID_PARAMETER;
    }
    if containing(base).is_none_or(|(b, _)| b != base) {
        return status::DLL_NOT_FOUND;
    }
    let found = if name == 0 {
        resolve(base, Wanted::Ordinal(ordinal as u16), 0).ok_or(status::ORDINAL_NOT_FOUND)
    } else {
        // SAFETY: the program's ANSI_STRING, whose buffer needn't end in a NUL.
        let wanted = unsafe { core::slice::from_raw_parts(ptr_at(name + PTR) as usize as *const u8, usize::from(u16_at(name))) };
        resolve(base, Wanted::Name(wanted), 0).ok_or_else(|| {
            traced_miss(base, wanted);
            status::PROCEDURE_NOT_FOUND
        })
    };
    match found {
        Ok(at) => {
            // SAFETY: the program's out pointer.
            unsafe { address.write_unaligned(at as usize) };
            status::SUCCESS
        }
        Err(code) => code,
    }
}

/// `LdrFindEntryForAddress(address, &entry)`: the LDR_DATA_TABLE_ENTRY of the module it's in.
#[unsafe(no_mangle)]
pub extern "system" fn LdrFindEntryForAddress(address: usize, out: *mut usize) -> u32 {
    let address = address as u64;
    let entry = with(|t| t.modules[..t.count].iter().find(|m| address >= m.base && address < m.base + m.size).map(|m| m.entry));
    match entry {
        Some(entry) if entry != 0 && !out.is_null() => {
            // SAFETY: the program's out pointer.
            unsafe { out.write_unaligned(entry as usize) };
            status::SUCCESS
        }
        _ => status::DLL_NOT_FOUND,
    }
}

// ---- Delay loads ----

/// `ResolveDelayLoadedAPI(parent base, IMAGE_DELAYLOAD_DESCRIPTOR, failure DLL hook, failure system hook, thunk,
/// flags)`, which MSVC's delay-load helper and Windows' own DLLs call the first time a delay-loaded import is: loads
/// the DLL if it isn't yet (keeping its handle where the descriptor says), finds the export, and writes it in the
/// thunk. If either fails, the hooks are asked for something to use instead (`DELAYLOAD_INFO`); with nothing,
/// it raises the delay-load helper's exception for the module (0xC06D007E) or the procedure (0xC06D007F).
#[unsafe(no_mangle)]
pub extern "system" fn LdrResolveDelayLoadedAPI(parent: usize, descriptor: usize, dll_hook: usize, system_hook: usize, thunk: usize, _flags: u32) -> usize {
    let (parent, descriptor, dll_hook, system_hook, thunk) = (parent as u64, descriptor as u64, dll_hook as u64, system_hook as u64, thunk as u64);
    let result = (|| -> u64 {
        // SAFETY: the parent's delay-load descriptor and its tables, all RVAs from its base.
        unsafe {
            let rva = |at: u64| parent + u64::from(u32_at(descriptor + at));
            let dll = cstr(rva(4));
            let (handle_at, iat, names) = (rva(8), rva(12), rva(16));
            // Thunks are pointers: 8 bytes, or 4 for a 32-bit program.
            let index = thunk.wrapping_sub(iat) / PTR;
            let entry = ptr_at(names + index * PTR);
            let (wanted, by_name, name_at) = if entry & ORDINAL != 0 {
                (Wanted::Ordinal(entry as u16), false, entry & 0xffff)
            } else {
                let at = parent + (entry & 0x7fff_ffff) + 2;
                (Wanted::Name(cstr(at)), true, at)
            };
            let mut module = ptr_at(handle_at);
            if module == 0 {
                match load_dynamic(dll) {
                    Ok(base) => {
                        // The handle's slot is pointer-sized, as the descriptor's module handle is.
                        let slot = &*(handle_at as usize as *const core::sync::atomic::AtomicUsize);
                        // Another thread got here first: its reference is the one kept.
                        if slot.compare_exchange(0, base as usize, Ordering::AcqRel, Ordering::Acquire).is_err() {
                            release(base);
                        }
                        module = slot.load(Ordering::Acquire) as u64;
                    }
                    Err(_) => {
                        set_last_error(ERROR_MOD_NOT_FOUND);
                        return delay_failed(3, descriptor, thunk, dll, by_name, name_at, 0, dll_hook, system_hook);
                    }
                }
            }
            match resolve(module, wanted, 0) {
                Some(address) => {
                    write_thunk(thunk, address);
                    address
                }
                None => {
                    set_last_error(ERROR_PROC_NOT_FOUND);
                    delay_failed(4, descriptor, thunk, dll, by_name, name_at, module, dll_hook, system_hook)
                }
            }
        }
    })();
    result as usize
}

/// A delay-load thunk, written whatever its page's protection.
fn write_thunk(thunk: u64, address: u64) {
    let old = sys::protect(thunk, PTR, PAGE_READWRITE);
    // SAFETY: the thunk, writable now.
    unsafe { core::ptr::write_volatile(thunk as usize as *mut usize, address as usize) };
    if let Some(old) = old {
        sys::protect(thunk, PTR, old);
    }
}

/// What a delay load that failed (`reason`: 3, the DLL; 4, the export) uses instead, from the hooks.
#[allow(clippy::too_many_arguments)]
fn delay_failed(reason: u32, descriptor: u64, thunk: u64, dll: &[u8], by_name: bool, name: u64, module: u64, dll_hook: u64, system_hook: u64) -> u64 {
    // DELAYLOAD_INFO: size, descriptor, thunk, the DLL's name, the export (by name or ordinal), the module, unused,
    // and the last error.
    let info: [usize; 9] =
        [9 * PTR as usize, descriptor as usize, thunk as usize, dll.as_ptr() as usize, usize::from(by_name), name as usize, module as usize, 0, crate::RtlGetLastWin32Error() as usize];
    if dll_hook != 0 {
        // SAFETY: the program's hook, as Windows calls it.
        let hook: extern "system" fn(u32, *const usize) -> usize = unsafe { core::mem::transmute(dll_hook as usize) };
        let instead = hook(reason, info.as_ptr()) as u64;
        if instead != 0 {
            write_thunk(thunk, instead);
            return instead;
        }
    }
    if system_hook != 0 {
        // SAFETY: as above.
        let hook: extern "system" fn(usize, usize) -> usize = unsafe { core::mem::transmute(system_hook as usize) };
        let instead = hook(dll.as_ptr() as usize, name as usize) as u64;
        if instead != 0 {
            write_thunk(thunk, instead);
            return instead;
        }
    }
    let code = if reason == 3 { 0xc06d_007e } else { 0xc06d_007f };
    let arguments = [info.as_ptr() as usize];
    crate::exception::AtlasRaiseException(code, 0, 1, arguments.as_ptr());
    0
}
