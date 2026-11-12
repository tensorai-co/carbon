//! The loader: what Windows' ntdll does to start a process and load DLLs, inside the program. atlas maps the program
//! and ntdll, and maps each DLL as the loader asks (relocating it, as NT's kernel does); the loader resolves imports
//! and exports (forwarders too), sets up static TLS, and runs TLS callbacks and each DLL's `DllMain`.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use atlas_abi::{peb, status, teb};

use crate::pe::{self, Export, Image, Wanted, cstr, u16_at, u32_at, u64_at};
use crate::strings::{Message, Name};
use crate::sys;

const MOST_MODULES: usize = 128;
const MOST_TLS: usize = 64;
const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_THREAD_ATTACH: u32 = 2;
const PAGE_READWRITE: u32 = 4;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_PROC_NOT_FOUND: u32 = 127;

#[derive(Clone, Copy)]
pub struct Module {
    pub base: u64,
    pub size: u64,
    pub name: Name,
    pub path: [u16; 260],
    /// Its imports are resolved, its TLS set up, and its `DllMain` called.
    pub ready: bool,
    /// Loaded at start, rather than by `LoadLibrary`.
    pub static_load: bool,
    /// Its static TLS index and TLS directory, if it has one.
    pub tls: Option<(usize, u64)>,
}

struct Table {
    modules: [Module; MOST_MODULES],
    count: usize,
    /// Static TLS: each module's index, given out in turn.
    tls_count: usize,
}

struct Loader {
    busy: AtomicBool,
    table: UnsafeCell<Table>,
}

// SAFETY: `table` is only touched while `busy` is held.
unsafe impl Sync for Loader {}

const EMPTY: Module = Module { base: 0, size: 0, name: Name::EMPTY, path: [0; 260], ready: false, static_load: false, tls: None };
static LOADER: Loader = Loader { busy: AtomicBool::new(false), table: UnsafeCell::new(Table { modules: [EMPTY; MOST_MODULES], count: 0, tls_count: 0 }) };

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

fn find(name: &Name) -> Option<u64> {
    with(|t| t.modules[..t.count].iter().find(|m| m.name == *name).map(|m| m.base))
}

/// The module `addr` is in: its base and size.
pub fn containing(addr: u64) -> Option<(u64, u64)> {
    with(|t| t.modules[..t.count].iter().find(|m| addr >= m.base && addr < m.base + m.size).map(|m| (m.base, m.size)))
}

fn add(base: u64, name: Name, path: [u16; 260], static_load: bool) {
    with(|t| {
        if t.count < MOST_MODULES {
            t.modules[t.count] = Module { base, size: Image(base).size(), name, path, ready: false, static_load, tls: None };
            t.count += 1;
        }
    });
}

fn fail(what: &[u8], name: &[u8], status: u32) -> u32 {
    Message::new().text(b"atlas-guest: ").text(what).text(b" ").text(name).text(b" (").hex(u64::from(status)).text(b")\n").print();
    status
}

/// Loads a DLL and what it imports: its base. `static_load` is for a process's start, where any failure ends it.
fn load(raw: &[u8], static_load: bool, depth: usize) -> Result<u64, u32> {
    let name = Name::module(raw);
    if let Some(base) = find(&name) {
        return Ok(base);
    }
    if depth > 32 {
        return Err(status::DLL_NOT_FOUND);
    }
    let mut path = [0u16; 260];
    let base = sys::map_image(raw, &mut path).map_err(|s| if static_load { fail(b"can't find", raw, s) } else { s })?;
    add(base, name, path, static_load);
    resolve_imports(Image(base), static_load, depth)?;
    set_up_tls(Image(base));
    Ok(base)
}

/// Fills in a module's import address table.
fn resolve_imports(image: Image, static_load: bool, depth: usize) -> Result<(), u32> {
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
            let dll_base = match load(dll, static_load, depth + 1) {
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
                let thunk = u64_at(entry);
                if thunk == 0 {
                    break;
                }
                let wanted = if thunk & (1 << 63) != 0 { Wanted::Ordinal(thunk as u16) } else { Wanted::Name(cstr(image.0 + (thunk & 0x7fff_ffff) + 2)) };
                match resolve(dll_base, wanted, depth) {
                    Some(address) => {
                        if iat.is_none() {
                            sys::protect(slot, 8, PAGE_READWRITE);
                        }
                        core::ptr::write_volatile(slot as *mut u64, address);
                    }
                    None => {
                        let shown = match wanted {
                            Wanted::Name(n) => n,
                            Wanted::Ordinal(_) => b"(an ordinal)",
                        };
                        result = Err(if static_load { fail(b"no export", shown, status::ENTRYPOINT_NOT_FOUND) } else { status::ENTRYPOINT_NOT_FOUND });
                        break;
                    }
                }
                entry += 8;
                slot += 8;
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
            let target = load(dll, false, depth + 1).ok()?;
            let wanted = match name.strip_prefix(b"#") {
                Some(digits) => Wanted::Ordinal(digits.iter().try_fold(0u16, |n, &d| (d as char).to_digit(10).map(|d| n * 10 + d as u16))?),
                None => Wanted::Name(name),
            };
            resolve(target, wanted, depth + 1)
        }
    }
}

/// Static TLS: gives a module with a TLS directory its index, and this thread its block.
fn set_up_tls(image: Image) {
    let Some((dir, _)) = image.dir(pe::DIR_TLS) else { return };
    let Some(index) = with(|t| {
        if t.tls_count >= MOST_TLS {
            return None;
        }
        t.tls_count += 1;
        let index = t.tls_count - 1;
        if let Some(m) = t.modules[..t.count].iter_mut().find(|m| m.base == image.0) {
            m.tls = Some((index, dir));
        }
        Some(index)
    }) else {
        return;
    };
    // SAFETY: the TLS directory's index field, in a mapped image.
    unsafe {
        let index_at = u64_at(dir + 16);
        if index_at != 0 {
            core::ptr::write_volatile(index_at as *mut u32, index as u32);
        }
    }
    tls_block(index, dir);
}

/// This thread's block for one module's static TLS: a copy of its template, then zeros.
fn tls_block(index: usize, dir: u64) {
    // SAFETY: the TLS directory and its template in a mapped image, and this thread's TEB.
    unsafe {
        let (start, end) = (u64_at(dir), u64_at(dir + 8));
        let zero_fill = u64::from(u32_at(dir + 32));
        let slots = crate::teb().add(teb::THREAD_LOCAL_STORAGE as usize).cast::<*mut u64>();
        if (*slots).is_null() {
            *slots = crate::heap::allocate(MOST_TLS * 8, true).cast();
        }
        let block = crate::heap::allocate(((end - start + zero_fill) as usize).max(16), true);
        core::ptr::copy_nonoverlapping(start as *const u8, block, (end - start) as usize);
        *(*slots).add(index) = block as u64;
    }
}

/// Where a new thread starts: it waits here if it was made suspended, gets its own static TLS, tells each DLL it's
/// started, then runs `start(parameter)` and ends with what it returns.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasThreadStart(start: u64, parameter: u64) -> ! {
    sys::call4(atlas_abi::call::THREAD_STARTED, 0, 0, 0, 0);
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
        let image = Image(m.base);
        if m.ready && image.is_dll() && m.name != Name::module(b"ntdll.dll") {
            run_tls_callbacks(image, DLL_THREAD_ATTACH);
            if let Some(entry) = image.entry() {
                // SAFETY: a DLL's entry point, as Windows calls it.
                let main: extern "system" fn(u64, u32, u64) -> i32 = unsafe { core::mem::transmute(entry) };
                main(m.base, DLL_THREAD_ATTACH, 0);
            }
        }
    }
    // SAFETY: the program's thread routine, as Windows calls it.
    let start: extern "system" fn(u64) -> u32 = unsafe { core::mem::transmute(start) };
    AtlasExitThread(start(parameter))
}

/// `ExitThread`.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasExitThread(code: u32) -> ! {
    sys::call4(atlas_abi::call::EXIT_THREAD, u64::from(code), 0, 0, 0);
    sys::terminate(code)
}

/// Runs each module that isn't yet: its TLS callbacks, then `DllMain`. Dependencies were added first, so they run
/// first.
fn attach_all() -> bool {
    let mut at = 0;
    loop {
        let next = with(|t| {
            while at < t.count && t.modules[at].ready {
                at += 1;
            }
            (at < t.count).then(|| {
                t.modules[at].ready = true;
                (t.modules[at].base, t.modules[at].static_load)
            })
        });
        let Some((base, static_load)) = next else { return true };
        let image = Image(base);
        if !image.is_dll() {
            continue;
        }
        run_tls_callbacks(image, DLL_PROCESS_ATTACH);
        if let Some(entry) = image.entry() {
            // SAFETY: a DLL's entry point, as Windows calls it.
            let main: extern "system" fn(u64, u32, u64) -> i32 = unsafe { core::mem::transmute(entry) };
            if main(base, DLL_PROCESS_ATTACH, u64::from(static_load)) == 0 {
                Message::new().text(b"atlas-guest: a DLL's DllMain failed, at ").hex(base).text(b"\n").print();
                return false;
            }
        }
    }
}

fn run_tls_callbacks(image: Image, reason: u32) {
    let Some((dir, _)) = image.dir(pe::DIR_TLS) else { return };
    // SAFETY: the TLS directory's null-terminated list of callbacks.
    unsafe {
        let mut at = u64_at(dir + 24);
        if at == 0 {
            return;
        }
        loop {
            let callback = u64_at(at);
            if callback == 0 {
                break;
            }
            let callback: extern "system" fn(u64, u32, u64) = core::mem::transmute(callback);
            callback(image.0, reason, 0);
            at += 8;
        }
    }
}

/// Where the first thread starts: loads what the program imports, runs every DLL's start, then the program.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasProcessStart(peb_at: u64, ntdll: u64) -> ! {
    // A process made suspended waits here for `ResumeThread`, before anything of it has run.
    sys::call4(atlas_abi::call::THREAD_STARTED, 0, 0, 0, 0);
    let mut ntdll_path = [0u16; 260];
    for (i, b) in b"C:\\Windows\\System32\\ntdll.dll".iter().enumerate() {
        ntdll_path[i] = u16::from(*b);
    }
    add(ntdll, Name::module(b"ntdll.dll"), ntdll_path, true);
    // SAFETY: the PEB atlas made, and its process parameters' image path.
    let (exe, path) = unsafe {
        let exe = u64_at(peb_at + peb::IMAGE_BASE);
        let params = u64_at(peb_at + peb::PROCESS_PARAMETERS);
        let string = params + atlas_abi::params::IMAGE_PATH_NAME;
        let (len, buffer) = (u16_at(string) as usize / 2, u64_at(string + 8));
        let mut path = [0u16; 260];
        for (i, unit) in path.iter_mut().enumerate().take(len.min(259)) {
            *unit = u16_at(buffer + i as u64 * 2);
        }
        (exe, path)
    };
    let mut ascii = [0u8; 260];
    let n = path.iter().position(|&c| c == 0).unwrap_or(0);
    for i in 0..n {
        ascii[i] = path[i] as u8;
    }
    add(exe, Name::module(&ascii[..n]), path, true);
    if let Err(e) = resolve_imports(Image(exe), true, 0) {
        sys::terminate(e);
    }
    set_up_tls(Image(exe));
    with(|t| t.modules[..t.count].iter_mut().filter(|m| m.base == exe || m.base == ntdll).for_each(|m| m.ready = true));
    if !attach_all() {
        sys::terminate(status::DLL_NOT_FOUND + 7); // STATUS_DLL_INIT_FAILED
    }
    run_tls_callbacks(Image(exe), DLL_PROCESS_ATTACH);
    let Some(entry) = Image(exe).entry() else { sys::terminate(status::INVALID_IMAGE_FORMAT) };
    // SAFETY: the program's entry point, as Windows calls it.
    let main: extern "system" fn(u64) -> u32 = unsafe { core::mem::transmute(entry) };
    let code = main(peb_at);
    sys::terminate(code)
}

fn set_last_error(error: u32) {
    crate::RtlSetLastWin32Error(error);
}

/// A string the program passed, as ASCII: UTF-16 if `wide`.
///
/// # Safety
///
/// `at` is a readable NUL-terminated string.
unsafe fn read_name(at: u64, wide: bool, out: &mut [u8; 260]) -> usize {
    let mut n = 0;
    while n < 259 {
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

fn load_library(at: u64, wide: bool) -> u64 {
    if at == 0 {
        set_last_error(87);
        return 0;
    }
    let mut name = [0u8; 260];
    // SAFETY: the program's string.
    let n = unsafe { read_name(at, wide, &mut name) };
    match load(&name[..n], false, 0) {
        Ok(base) => {
            if attach_all() {
                base
            } else {
                set_last_error(1114); // ERROR_DLL_INIT_FAILED
                0
            }
        }
        Err(_) => {
            set_last_error(ERROR_MOD_NOT_FOUND);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryA(name: u64) -> u64 {
    load_library(name, false)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryW(name: u64) -> u64 {
    load_library(name, true)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryExA(name: u64, _file: u64, _flags: u32) -> u64 {
    load_library(name, false)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasLoadLibraryExW(name: u64, _file: u64, _flags: u32) -> u64 {
    load_library(name, true)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasFreeLibrary(_module: u64) -> i32 {
    1
}

fn exe_base() -> u64 {
    // SAFETY: this thread's TEB and its PEB.
    unsafe { u64_at(crate::teb() as u64 + teb::PEB) }.wrapping_add(peb::IMAGE_BASE)
}

fn module_handle(at: u64, wide: bool) -> u64 {
    if at == 0 {
        // SAFETY: the PEB's image base.
        return unsafe { u64_at(exe_base()) };
    }
    let mut name = [0u8; 260];
    // SAFETY: the program's string.
    let n = unsafe { read_name(at, wide, &mut name) };
    find(&Name::module(&name[..n])).unwrap_or_else(|| {
        set_last_error(ERROR_MOD_NOT_FOUND);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleA(name: u64) -> u64 {
    module_handle(name, false)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleHandleW(name: u64) -> u64 {
    module_handle(name, true)
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetProcAddress(module: u64, name: u64) -> u64 {
    // SAFETY: the PEB's image base.
    let base = if module == 0 { unsafe { u64_at(exe_base()) } } else { module };
    if containing(base).is_none() {
        set_last_error(ERROR_MOD_NOT_FOUND);
        return 0;
    }
    // An ordinal is a "pointer" below 64 KiB.
    // SAFETY: the program's string, if it isn't an ordinal.
    let wanted = if name < 0x1_0000 { Wanted::Ordinal(name as u16) } else { Wanted::Name(unsafe { cstr(name) }) };
    resolve(base, wanted, 0).unwrap_or_else(|| {
        set_last_error(ERROR_PROC_NOT_FOUND);
        0
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn AtlasGetModuleFileNameW(module: u64, buffer: *mut u16, size: u32) -> u32 {
    // SAFETY: the PEB's image base.
    let base = if module == 0 { unsafe { u64_at(exe_base()) } } else { module };
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
