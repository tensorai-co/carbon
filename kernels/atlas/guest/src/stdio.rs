//! The C runtime's stdio, as `ucrtbase.dll` has it: streams (`FILE`) over kernel32's files, standard input, output
//! and error among them, in text mode (CR LF out for each LF, and back on reading) unless they're binary; and the
//! `printf` family, narrow and wide, as UCRT's `__stdio_common_*` entry points take it, with every conversion C has
//! but `%a` and `%n`.
//!
//! Streams aren't buffered on their way out: each write goes to the file then. Reading has a buffer, for `fgets` and
//! `ungetc`.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use atlas_abi::ansi::{from_1252, to_1252};

use crate::sys::host;

host!(get_std_handle, b"GetStdHandle", (which));
host!(write_file, b"WriteFile", (handle, buffer, count, written, overlapped));
host!(read_file, b"ReadFile", (handle, buffer, count, read, overlapped));
host!(create_file_a, b"CreateFileA", (name, access, share, attributes, disposition, flags, template));
host!(create_file_w, b"CreateFileW", (name, access, share, attributes, disposition, flags, template));
host!(close_handle, b"CloseHandle", (handle));
host!(get_file_type, b"GetFileType", (handle));
host!(get_file_information_by_handle, b"GetFileInformationByHandle", (handle, info));
host!(duplicate_handle, b"DuplicateHandle", (src_proc, src_h, dst_proc, dst_h, access, inherit, opts));
#[cfg(not(target_arch = "x86"))]
host!(set_file_pointer_ex, b"SetFilePointerEx", (handle, distance, new, method));
// On x86 its LARGE_INTEGER distance is two stack slots, low half first.
#[cfg(target_arch = "x86")]
host!(set_file_pointer_ex, b"SetFilePointerEx", (handle, low, high, new, method));

/// `SetFilePointerEx(handle, distance, &new, method)`.
fn seek(handle: u64, distance: i64, new: u64, method: u64) -> u64 {
    #[cfg(not(target_arch = "x86"))]
    let r = set_file_pointer_ex(handle, distance as u64, new, method);
    #[cfg(target_arch = "x86")]
    let r = set_file_pointer_ex(handle, distance as u64 & 0xffff_ffff, distance as u64 >> 32, new, method);
    r
}

const EOF: i32 = -1;
const BUFFER: usize = 4096;

/// A stream. The program only ever holds a pointer to it.
#[repr(C)]
pub struct File {
    handle: u64,
    /// Assigned POSIX fd (-1 if not yet registered in `FDS`).
    fd: i32,
    /// Reentrant stream lock (`_lock_file` / `_unlock_file`).
    lock_owner: AtomicU32,
    lock_count: AtomicU32,
    /// Opened for reading, writing, appending; in text mode; at its end; in error.
    read: bool,
    write: bool,
    append: bool,
    text: bool,
    eof: bool,
    error: bool,
    /// One of the standard three, whose handle comes from `GetStdHandle` the first time it's used.
    standard: u8,
    /// What's been read and not taken: `buffer[at..have]`.
    at: usize,
    have: usize,
    buffer: [u8; BUFFER],
}

impl File {
    const fn standard(which: u8) -> File {
        File {
            handle: 0,
            fd: which as i32,
            lock_owner: AtomicU32::new(0),
            lock_count: AtomicU32::new(0),
            read: which == 0,
            write: which != 0,
            append: false,
            text: true,
            eof: false,
            error: false,
            standard: which + 1,
            at: 0,
            have: 0,
            buffer: [0; BUFFER],
        }
    }

    fn handle(&mut self) -> u64 {
        if self.handle == 0 && self.standard != 0 {
            // STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE: -10, -11, -12.
            self.handle = get_std_handle((-10i64 - i64::from(self.standard - 1)) as u64);
        }
        self.handle
    }
}

static mut STANDARD: [File; 3] = [File::standard(0), File::standard(1), File::standard(2)];
static STREAMS: AtomicBool = AtomicBool::new(false);

/// `__acrt_iob_func(index)`: stdin, stdout or stderr.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtIob(index: u32) -> *mut File {
    STREAMS.store(true, Ordering::Relaxed);
    // SAFETY: the three standard streams, which live as long as the process.
    unsafe { (&raw mut STANDARD).cast::<File>().add(index.min(2) as usize) }
}

/// Writes bytes to a stream's file, in text mode with each LF as CR LF.
fn put(f: &mut File, bytes: &[u8]) -> bool {
    if !f.write {
        f.error = true;
        return false;
    }
    let handle = f.handle();
    let mut written = 0u32;
    let mut send = |chunk: &[u8]| -> bool {
        if chunk.is_empty() {
            return true;
        }
        if f.append {
            let mut end = 0i64;
            seek(handle, 0, (&raw mut end) as usize as u64, 2);
        }
        write_file(handle, chunk.as_ptr() as u64, chunk.len() as u64, (&raw mut written) as u64, 0) != 0
    };
    let ok = if f.text {
        let mut start = 0;
        let mut ok = true;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\n' {
                ok &= send(&bytes[start..i]) && send(b"\r\n");
                start = i + 1;
            }
        }
        ok && send(&bytes[start..])
    } else {
        send(bytes)
    };
    if !ok {
        f.error = true;
    }
    ok
}

/// The next byte of a stream, refilling its buffer from the file; in text mode, CR LF as LF.
fn get(f: &mut File) -> i32 {
    loop {
        if f.at == f.have {
            if !f.read || f.eof {
                f.eof = true;
                return EOF;
            }
            let handle = f.handle();
            let mut read = 0u32;
            let ok = read_file(handle, f.buffer.as_mut_ptr() as u64, BUFFER as u64, (&raw mut read) as u64, 0) != 0;
            f.at = 0;
            f.have = read as usize;
            if read == 0 {
                f.eof = true;
                if !ok {
                    f.error = true;
                }
                return EOF;
            }
        }
        let b = f.buffer[f.at];
        f.at += 1;
        if f.text && b == b'\r' {
            // A CR before LF is dropped; one alone stays.
            if f.at == f.have {
                let mut next = [0u8];
                let mut read = 0u32;
                let handle = f.handle();
                if read_file(handle, next.as_mut_ptr() as u64, 1, (&raw mut read) as u64, 0) != 0 && read == 1 {
                    if next[0] == b'\n' {
                        return i32::from(b'\n');
                    }
                    f.buffer[0] = next[0];
                    f.at = 0;
                    f.have = 1;
                }
                return i32::from(b'\r');
            }
            if f.buffer[f.at] == b'\n' {
                continue;
            }
        }
        return i32::from(b);
    }
}

/// Writes to standard error, as `abort` does.
pub fn write_err(bytes: &[u8]) {
    // SAFETY: the standard streams.
    put(unsafe { &mut *AtlasCrtIob(2) }, bytes);
}

/// Every stream's pending output, at exit: there's none, as nothing's buffered.
pub fn flush_all() {}

fn stream<'a>(f: *mut File) -> Option<&'a mut File> {
    // SAFETY: a stream the program was given.
    unsafe { f.as_mut() }
}

// ---- Streams ----

fn parse_mode_bytes(mode: &[u8]) -> Option<(bool, bool, bool, u64, bool)> {
    let plus = mode.contains(&b'+');
    let (read, write, append, disposition) = match mode.first() {
        Some(b'r') => (true, plus, false, 3),
        Some(b'w') if mode.contains(&b'x') => (plus, true, false, 1),
        Some(b'w') => (plus, true, false, 2),
        Some(b'a') => (plus, true, true, 4),
        _ => return None,
    };
    let text = !mode.contains(&b'b');
    Some((read, write, append, disposition, text))
}

fn alloc_file(handle: u64, fd: i32, read: bool, write: bool, append: bool, text: bool) -> *mut File {
    let f = crate::heap::allocate(size_of::<File>(), true).cast::<File>();
    if f.is_null() {
        return f;
    }
    // SAFETY: the new stream.
    unsafe {
        f.write(File {
            handle,
            fd,
            lock_owner: AtomicU32::new(0),
            lock_count: AtomicU32::new(0),
            read,
            write,
            append,
            text,
            eof: false,
            error: false,
            standard: 0,
            at: 0,
            have: 0,
            buffer: [0; BUFFER],
        });
    }
    f
}

/// `fopen(name, mode)`: `r`, `w`, `a`, each with `+`, and `b`, `t` and `x`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFopen(name: *const u8, mode: *const u8) -> *mut File {
    const GENERIC_READ: u64 = 0x8000_0000;
    const GENERIC_WRITE: u64 = 0x4000_0000;
    if name.is_null() || mode.is_null() {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    }
    // SAFETY: the program's mode string.
    let mode = unsafe { core::slice::from_raw_parts(mode, crate::crt::AtlasCrtStrlen(mode)) };
    let Some((read, write, append, disposition, text)) = parse_mode_bytes(mode) else {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    };
    let access = if read { GENERIC_READ } else { 0 } | if write { GENERIC_WRITE } else { 0 };
    let handle = create_file_a(name as u64, access, 3, 0, disposition, 0x80, 0);
    if handle == crate::pe::INVALID_HANDLE || handle == 0 {
        crate::crt::set_errno(crate::crt_ext::map_win32_errno());
        return core::ptr::null_mut();
    }
    let f = alloc_file(handle, -1, read, write, append, text);
    if f.is_null() {
        close_handle(handle);
    }
    f
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWfopen(name: *const u16, mode: *const u16) -> *mut File {
    const GENERIC_READ: u64 = 0x8000_0000;
    const GENERIC_WRITE: u64 = 0x4000_0000;
    if name.is_null() || mode.is_null() {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    }
    let mut mbuf = [0u8; 32];
    let mlen = unsafe { crate::crt::AtlasCrtWcslen(mode) }.min(mbuf.len());
    for i in 0..mlen {
        mbuf[i] = unsafe { *mode.add(i) } as u8;
    }
    let Some((read, write, append, disposition, text)) = parse_mode_bytes(&mbuf[..mlen]) else {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    };
    let access = if read { GENERIC_READ } else { 0 } | if write { GENERIC_WRITE } else { 0 };
    let handle = create_file_w(name as u64, access, 3, 0, disposition, 0x80, 0);
    if handle == crate::pe::INVALID_HANDLE || handle == 0 {
        crate::crt::set_errno(crate::crt_ext::map_win32_errno());
        return core::ptr::null_mut();
    }
    let f = alloc_file(handle, -1, read, write, append, text);
    if f.is_null() {
        close_handle(handle);
    }
    f
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFsopen(name: *const u8, mode: *const u8, _shflag: i32) -> *mut File {
    unsafe { AtlasCrtFopen(name, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWfsopen(name: *const u16, mode: *const u16, _shflag: i32) -> *mut File {
    unsafe { AtlasCrtWfopen(name, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFreopen(name: *const u8, mode: *const u8, f: *mut File) -> *mut File {
    let Some(file) = stream(f) else {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    };
    if name.is_null() || mode.is_null() {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    }
    let mode_s = unsafe { core::slice::from_raw_parts(mode, crate::crt::AtlasCrtStrlen(mode)) };
    let Some((read, write, append, disposition, text)) = parse_mode_bytes(mode_s) else {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    };
    const GENERIC_READ: u64 = 0x8000_0000;
    const GENERIC_WRITE: u64 = 0x4000_0000;
    let access = if read { GENERIC_READ } else { 0 } | if write { GENERIC_WRITE } else { 0 };
    let handle = create_file_a(name as u64, access, 3, 0, disposition, 0x80, 0);
    if handle == crate::pe::INVALID_HANDLE || handle == 0 {
        crate::crt::set_errno(crate::crt_ext::map_win32_errno());
        return core::ptr::null_mut();
    }
    if file.standard == 0 && file.handle != 0 {
        close_handle(file.handle);
    }
    file.handle = handle;
    file.read = read;
    file.write = write;
    file.append = append;
    file.text = text;
    file.eof = false;
    file.error = false;
    file.at = 0;
    file.have = 0;
    f
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFreopenS(pfile: *mut *mut File, name: *const u8, mode: *const u8, f: *mut File) -> i32 {
    if pfile.is_null() {
        crate::crt::set_errno(22);
        return 22;
    }
    let r = unsafe { AtlasCrtFreopen(name, mode, f) };
    unsafe { *pfile = r };
    if r.is_null() { unsafe { *crate::crt::AtlasCrtErrno() } } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFclose(f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    if file.standard != 0 {
        return 0;
    }
    if file.fd >= 3 {
        fd_release(file.fd);
    }
    close_handle(file.handle);
    crate::heap::free(f.cast());
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLockFile(f: *mut File) {
    let Some(file) = stream(f) else { return };
    let me = (crate::teb() as usize as u32) | 1;
    if file.lock_owner.load(Ordering::Relaxed) == me {
        file.lock_count.fetch_add(1, Ordering::Relaxed);
        return;
    }
    while file.lock_owner.compare_exchange_weak(0, me, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    file.lock_count.store(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtUnlockFile(f: *mut File) {
    let Some(file) = stream(f) else { return };
    let count = file.lock_count.load(Ordering::Relaxed);
    if count <= 1 {
        file.lock_count.store(0, Ordering::Relaxed);
        file.lock_owner.store(0, Ordering::Release);
    } else {
        file.lock_count.store(count - 1, Ordering::Relaxed);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetbuf(_f: *mut File, _buffer: *mut u8) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFgetpos(f: *mut File, pos: *mut i64) -> i32 {
    if pos.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let p = AtlasCrtFtelli64(f);
    if p < 0 {
        return -1;
    }
    unsafe { pos.write_unaligned(p) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFsetpos(f: *mut File, pos: *const i64) -> i32 {
    if pos.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    AtlasCrtFseeki64(f, unsafe { pos.read_unaligned() }, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFgetwc(f: *mut File) -> u32 {
    let c = AtlasCrtFgetc(f);
    if c == EOF { 0xffff } else { u32::from(from_1252(c as u8)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFputwc(c: u16, f: *mut File) -> u32 {
    let b = to_1252(c).unwrap_or(b'?');
    if AtlasCrtFputc(i32::from(b), f) == EOF { 0xffff } else { u32::from(c) }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtUngetwc(c: u32, f: *mut File) -> u32 {
    if c == 0xffff {
        return 0xffff;
    }
    let b = to_1252(c as u16).unwrap_or(b'?');
    if AtlasCrtUngetc(i32::from(b), f) == EOF { 0xffff } else { c }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetStreamBufferPointers(_f: *mut File, _base: *mut usize, _ptr: *mut usize, _cnt: *mut usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFflush(_f: *mut File) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetvbuf(_f: *mut File, _buffer: *mut u8, _mode: i32, _size: usize) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFwrite(p: *const u8, size: usize, count: usize, f: *mut File) -> usize {
    let Some(file) = stream(f) else { return 0 };
    let Some(total) = size.checked_mul(count) else { return 0 };
    if total == 0 {
        return 0;
    }
    // SAFETY: the program's buffer, `total` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(p, total) };
    if put(file, bytes) { count } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFread(p: *mut u8, size: usize, count: usize, f: *mut File) -> usize {
    let Some(file) = stream(f) else { return 0 };
    let Some(total) = size.checked_mul(count) else { return 0 };
    let mut n = 0;
    while n < total {
        let c = get(file);
        if c == EOF {
            break;
        }
        // SAFETY: the program's buffer, `total` bytes.
        unsafe { *p.add(n) = c as u8 };
        n += 1;
    }
    n.checked_div(size).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFputs(s: *const u8, f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    // SAFETY: the program's string.
    let bytes = unsafe { core::slice::from_raw_parts(s, crate::crt::AtlasCrtStrlen(s)) };
    if put(file, bytes) { 0 } else { EOF }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtPuts(s: *const u8) -> i32 {
    let out = AtlasCrtIob(1);
    // SAFETY: as above, then its newline.
    unsafe {
        if AtlasCrtFputs(s, out) == EOF || AtlasCrtFputs(c"\n".as_ptr().cast(), out) == EOF {
            return EOF;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFputc(c: i32, f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    if put(file, &[c as u8]) { c & 0xff } else { EOF }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtPutchar(c: i32) -> i32 {
    AtlasCrtFputc(c, AtlasCrtIob(1))
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFgetc(f: *mut File) -> i32 {
    stream(f).map_or(EOF, get)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetchar() -> i32 {
    AtlasCrtFgetc(AtlasCrtIob(0))
}

/// `ungetc`: one byte back, to be read next.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtUngetc(c: i32, f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    if c == EOF {
        return EOF;
    }
    if file.at == 0 {
        if file.have == BUFFER {
            return EOF;
        }
        file.buffer.copy_within(0..file.have, 1);
        file.have += 1;
    } else {
        file.at -= 1;
    }
    file.buffer[file.at] = c as u8;
    file.eof = false;
    c & 0xff
}

/// `fgets(buffer, size, stream)`: a line with its newline, or as much as fits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFgets(p: *mut u8, size: i32, f: *mut File) -> *mut u8 {
    let Some(file) = stream(f) else { return core::ptr::null_mut() };
    if size <= 0 || p.is_null() {
        return core::ptr::null_mut();
    }
    let mut n = 0usize;
    while n + 1 < size as usize {
        let c = get(file);
        if c == EOF {
            break;
        }
        // SAFETY: the program's buffer, `size` bytes.
        unsafe { *p.add(n) = c as u8 };
        n += 1;
        if c == i32::from(b'\n') {
            break;
        }
    }
    if n == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: as above.
    unsafe { *p.add(n) = 0 };
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFeof(f: *mut File) -> i32 {
    stream(f).map_or(0, |f| i32::from(f.eof && f.at == f.have))
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFerror(f: *mut File) -> i32 {
    stream(f).map_or(0, |f| i32::from(f.error))
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtClearerr(f: *mut File) {
    if let Some(f) = stream(f) {
        f.eof = false;
        f.error = false;
    }
}

/// `_fseeki64(stream, offset, whence)`: what's buffered is dropped.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFseeki64(f: *mut File, offset: i64, whence: i32) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    // A seek from where the program is: the file's position less what's buffered and not taken.
    let offset = if whence == 1 { offset - (file.have - file.at) as i64 } else { offset };
    file.at = 0;
    file.have = 0;
    file.eof = false;
    let handle = file.handle();
    if seek(handle, offset, 0, whence as u64) == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFseek(f: *mut File, offset: i32, whence: i32) -> i32 {
    AtlasCrtFseeki64(f, i64::from(offset), whence)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFtelli64(f: *mut File) -> i64 {
    let Some(file) = stream(f) else { return -1 };
    let mut at = 0i64;
    let handle = file.handle();
    if seek(handle, 0, (&raw mut at) as usize as u64, 1) == 0 {
        return -1;
    }
    at - (file.have - file.at) as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFtell(f: *mut File) -> i32 {
    AtlasCrtFtelli64(f) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtRewind(f: *mut File) {
    AtlasCrtFseeki64(f, 0, 0);
    AtlasCrtClearerr(f);
}

// ---- POSIX file descriptor table ----

const MAX_FDS: usize = 256;

#[derive(Clone, Copy)]
struct FdSlot {
    used: bool,
    handle: u64,
    text: bool,
    append: bool,
}

impl FdSlot {
    const EMPTY: FdSlot = FdSlot { used: false, handle: 0, text: false, append: false };
}

static FD_LOCK: AtomicBool = AtomicBool::new(false);
static mut FDS: [FdSlot; MAX_FDS] = [FdSlot::EMPTY; MAX_FDS];

fn with_fds<R>(f: impl FnOnce(&mut [FdSlot; MAX_FDS]) -> R) -> R {
    while FD_LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: protected by FD_LOCK.
    let r = f(unsafe { &mut *(&raw mut FDS) });
    FD_LOCK.store(false, Ordering::Release);
    r
}

fn fd_alloc(handle: u64, text: bool, append: bool) -> i32 {
    with_fds(|fds| {
        for i in 3..MAX_FDS {
            if !fds[i].used {
                fds[i] = FdSlot { used: true, handle, text, append };
                return i as i32;
            }
        }
        -1
    })
}

fn fd_release(fd: i32) -> Option<FdSlot> {
    if !(3..MAX_FDS as i32).contains(&fd) {
        return None;
    }
    with_fds(|fds| {
        let s = fds[fd as usize];
        if s.used {
            fds[fd as usize] = FdSlot::EMPTY;
            Some(s)
        } else {
            None
        }
    })
}

fn fd_get(fd: i32) -> Option<FdSlot> {
    if (0..3).contains(&fd) {
        // SAFETY: the three standard streams.
        let file = unsafe { &mut *AtlasCrtIob(fd as u32) };
        let handle = file.handle();
        return Some(FdSlot { used: true, handle, text: file.text, append: false });
    }
    if !(3..MAX_FDS as i32).contains(&fd) {
        return None;
    }
    with_fds(|fds| {
        let s = fds[fd as usize];
        s.used.then_some(s)
    })
}

/// `_fileno`: 0, 1 and 2 for the standard streams; others get a slot in the fd table.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFileno(f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    if file.standard != 0 {
        return i32::from(file.standard - 1);
    }
    if file.fd < 0 {
        file.fd = fd_alloc(file.handle, file.text, file.append);
    }
    file.fd
}

/// `_setmode(fd, mode)`: _O_TEXT (0x4000) or _O_BINARY (0x8000); the mode before.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetmode(fd: i32, mode: i32) -> i32 {
    if (0..3).contains(&fd) {
        // SAFETY: the standard streams.
        let file = unsafe { &mut *AtlasCrtIob(fd as u32) };
        let before = if file.text { 0x4000 } else { 0x8000 };
        file.text = mode & 0x8000 == 0;
        return before;
    }
    if !(3..MAX_FDS as i32).contains(&fd) {
        crate::crt::set_errno(9);
        return -1;
    }
    with_fds(|fds| {
        let s = &mut fds[fd as usize];
        if !s.used {
            crate::crt::set_errno(9);
            return -1;
        }
        let before = if s.text { 0x4000 } else { 0x8000 };
        s.text = mode & 0x8000 == 0;
        before
    })
}

fn open_flags(oflag: i32) -> (u64, u64, bool, bool) {
    const O_WRONLY: i32 = 0x0001;
    const O_RDWR: i32 = 0x0002;
    const O_APPEND: i32 = 0x0008;
    const O_CREAT: i32 = 0x0100;
    const O_TRUNC: i32 = 0x0200;
    const O_EXCL: i32 = 0x0400;
    const O_BINARY: i32 = 0x8000;
    const GENERIC_READ: u64 = 0x8000_0000;
    const GENERIC_WRITE: u64 = 0x4000_0000;
    let access = match oflag & 3 {
        O_WRONLY => GENERIC_WRITE,
        O_RDWR => GENERIC_READ | GENERIC_WRITE,
        _ => GENERIC_READ,
    };
    let disposition = match (oflag & O_CREAT != 0, oflag & O_EXCL != 0, oflag & O_TRUNC != 0) {
        (true, true, _) => 1,   // CREATE_NEW
        (true, false, true) => 2, // CREATE_ALWAYS
        (true, false, false) => 4, // OPEN_ALWAYS
        (false, _, true) => 5,  // TRUNCATE_EXISTING
        (false, _, false) => 3, // OPEN_EXISTING
    };
    let text = oflag & O_BINARY == 0;
    let append = oflag & O_APPEND != 0;
    (access, disposition, text, append)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtOpen(path: *const u8, oflag: i32, _pmode: i32) -> i32 {
    if path.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let (access, disp, text, append) = open_flags(oflag);
    let h = create_file_a(path as usize as u64, access, 3, 0, disp, 0x80, 0);
    if h == crate::pe::INVALID_HANDLE || h == 0 {
        crate::crt::set_errno(crate::crt_ext::map_win32_errno());
        return -1;
    }
    let fd = fd_alloc(h, text, append);
    if fd < 0 {
        close_handle(h);
        crate::crt::set_errno(24);
    }
    fd
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWopen(path: *const u16, oflag: i32, _pmode: i32) -> i32 {
    if path.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let (access, disp, text, append) = open_flags(oflag);
    let h = create_file_w(path as usize as u64, access, 3, 0, disp, 0x80, 0);
    if h == crate::pe::INVALID_HANDLE || h == 0 {
        crate::crt::set_errno(crate::crt_ext::map_win32_errno());
        return -1;
    }
    let fd = fd_alloc(h, text, append);
    if fd < 0 {
        close_handle(h);
        crate::crt::set_errno(24);
    }
    fd
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtSopenS(pfd: *mut i32, path: *const u8, oflag: i32, _shflag: i32, pmode: i32) -> i32 {
    if pfd.is_null() {
        crate::crt::set_errno(22);
        return 22;
    }
    let fd = unsafe { AtlasCrtOpen(path, oflag, pmode) };
    unsafe { *pfd = fd };
    if fd < 0 { unsafe { *crate::crt::AtlasCrtErrno() } } else { 0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtClose(fd: i32) -> i32 {
    if (0..3).contains(&fd) {
        return 0;
    }
    let Some(s) = fd_release(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    close_handle(s.handle);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtRead(fd: i32, buf: *mut u8, count: u32) -> i32 {
    if buf.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    let mut got = 0u32;
    if read_file(s.handle, buf as usize as u64, u64::from(count), (&raw mut got) as usize as u64, 0) == 0 {
        crate::crt::set_errno(5);
        return -1;
    }
    got as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtWrite(fd: i32, buf: *const u8, count: u32) -> i32 {
    if buf.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    if s.append {
        let mut end = 0i64;
        seek(s.handle, 0, (&raw mut end) as usize as u64, 2);
    }
    let mut wrote = 0u32;
    if write_file(s.handle, buf as usize as u64, u64::from(count), (&raw mut wrote) as usize as u64, 0) == 0 {
        crate::crt::set_errno(5);
        return -1;
    }
    wrote as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtLseeki64(fd: i32, offset: i64, origin: i32) -> i64 {
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    let mut new_pos = 0i64;
    if seek(s.handle, offset, (&raw mut new_pos) as usize as u64, origin as u64) == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    new_pos
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtGetOsfhandle(fd: i32) -> isize {
    match fd_get(fd) {
        Some(s) => s.handle as isize,
        None => {
            crate::crt::set_errno(9);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtIsatty(fd: i32) -> i32 {
    let Some(s) = fd_get(fd) else { return 0 };
    // FILE_TYPE_CHAR = 2
    i32::from(get_file_type(s.handle) == 2)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFdopen(fd: i32, mode: *const u8) -> *mut File {
    if mode.is_null() {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    }
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return core::ptr::null_mut();
    };
    let mode_s = unsafe { core::slice::from_raw_parts(mode, crate::crt::AtlasCrtStrlen(mode)) };
    let Some((read, write, append, _, text)) = parse_mode_bytes(mode_s) else {
        crate::crt::set_errno(22);
        return core::ptr::null_mut();
    };
    alloc_file(s.handle, fd, read, write, append, text)
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtDup(fd: i32) -> i32 {
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    let mut dup_h = 0usize;
    let cur = usize::MAX as u64; // GetCurrentProcess() = -1
    if duplicate_handle(cur, s.handle, cur, (&raw mut dup_h) as usize as u64, 0, 1, 2) == 0 {
        crate::crt::set_errno(9);
        return -1;
    }
    let new_fd = fd_alloc(dup_h as u64, s.text, s.append);
    if new_fd < 0 {
        close_handle(dup_h as u64);
        crate::crt::set_errno(24);
    }
    new_fd
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtDup2(fd1: i32, fd2: i32) -> i32 {
    if !(0..MAX_FDS as i32).contains(&fd2) {
        crate::crt::set_errno(9);
        return -1;
    }
    let Some(s) = fd_get(fd1) else {
        crate::crt::set_errno(9);
        return -1;
    };
    if fd1 == fd2 {
        return 0;
    }
    let mut dup_h = 0usize;
    let cur = usize::MAX as u64;
    if duplicate_handle(cur, s.handle, cur, (&raw mut dup_h) as usize as u64, 0, 1, 2) == 0 {
        crate::crt::set_errno(9);
        return -1;
    }
    if (0..3).contains(&fd2) {
        // SAFETY: standard stream.
        let file = unsafe { &mut *AtlasCrtIob(fd2 as u32) };
        file.handle = dup_h as u64;
        return 0;
    }
    with_fds(|fds| {
        if fds[fd2 as usize].used {
            close_handle(fds[fd2 as usize].handle);
        }
        fds[fd2 as usize] = FdSlot { used: true, handle: dup_h as u64, text: s.text, append: s.append };
    });
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFstat64(fd: i32, buf: *mut crate::crt_ext::Stat64) -> i32 {
    if buf.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let Some(s) = fd_get(fd) else {
        crate::crt::set_errno(9);
        return -1;
    };
    let mut info = [0u32; 13];
    if get_file_information_by_handle(s.handle, info.as_mut_ptr() as usize as u64) == 0 {
        // Pipe/char device fallback: minimal valid stat.
        let st = crate::crt_ext::fill_stat64(0, 0, 0, 0, 0);
        unsafe { buf.write(st) };
        return 0;
    }
    let size = (u64::from(info[8]) << 32) | u64::from(info[9]);
    let st = crate::crt_ext::fill_stat64(
        info[0],
        size,
        crate::crt_ext::filetime_to_unix(info[1], info[2]),
        crate::crt_ext::filetime_to_unix(info[3], info[4]),
        crate::crt_ext::filetime_to_unix(info[5], info[6]),
    );
    unsafe { buf.write(st) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtFstat64i32(fd: i32, buf: *mut crate::crt_ext::Stat64i32) -> i32 {
    if buf.is_null() {
        crate::crt::set_errno(22);
        return -1;
    }
    let mut st = core::mem::MaybeUninit::<crate::crt_ext::Stat64>::uninit();
    if unsafe { AtlasCrtFstat64(fd, st.as_mut_ptr()) } != 0 {
        return -1;
    }
    let s = unsafe { st.assume_init() };
    unsafe {
        buf.write(crate::crt_ext::Stat64i32 {
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

// ---- printf ----

/// Where formatted output goes, a unit at a time: a narrow sink takes bytes (and UTF-16, as 1252), a wide one
/// UTF-16 (and bytes, from 1252).
trait Sink {
    fn byte(&mut self, b: u8);
    fn unit(&mut self, u: u16);
    fn count(&self) -> usize;
}

/// Formatted output into a buffer of the program's: as much as fits, and how much there was.
struct Buffer<'a, T> {
    out: &'a mut [T],
    n: usize,
}

impl Sink for Buffer<'_, u8> {
    fn byte(&mut self, b: u8) {
        if let Some(slot) = self.out.get_mut(self.n) {
            *slot = b;
        }
        self.n += 1;
    }
    fn unit(&mut self, u: u16) {
        self.byte(to_1252(u).unwrap_or(b'?'));
    }
    fn count(&self) -> usize {
        self.n
    }
}

impl Sink for Buffer<'_, u16> {
    fn byte(&mut self, b: u8) {
        self.unit(from_1252(b));
    }
    fn unit(&mut self, u: u16) {
        if let Some(slot) = self.out.get_mut(self.n) {
            *slot = u;
        }
        self.n += 1;
    }
    fn count(&self) -> usize {
        self.n
    }
}

/// Formatted output to a stream, in chunks.
struct ToStream<'a> {
    file: &'a mut File,
    chunk: [u8; 256],
    used: usize,
    n: usize,
    failed: bool,
}

impl ToStream<'_> {
    fn flush(&mut self) {
        if self.used > 0 {
            let chunk = self.chunk;
            if !put(self.file, &chunk[..self.used]) {
                self.failed = true;
            }
            self.used = 0;
        }
    }
}

impl Sink for ToStream<'_> {
    fn byte(&mut self, b: u8) {
        if self.used == self.chunk.len() {
            self.flush();
        }
        self.chunk[self.used] = b;
        self.used += 1;
        self.n += 1;
    }
    fn unit(&mut self, u: u16) {
        self.byte(to_1252(u).unwrap_or(b'?'));
    }
    fn count(&self) -> usize {
        self.n
    }
}

/// The arguments after the format, as a `va_list` has them: on the 64-bit ISAs one 8-byte slot each, a double's
/// bits too; on x86 a 4-byte slot for anything up to a pointer's size, and 8 bytes for a `long long` or a double.
struct Args(*const u8);

const POINTER_BITS: u32 = usize::BITS;

impl Args {
    fn take<T: Copy>(&mut self) -> T {
        // SAFETY: the program's va_list, as long as its format says.
        let v = unsafe { self.0.cast::<T>().read_unaligned() };
        // SAFETY: as above.
        self.0 = unsafe { self.0.add(size_of::<T>()) };
        v
    }

    /// An int, a pointer or anything between: a slot of its own.
    fn next(&mut self) -> u64 {
        self.take::<usize>() as u64
    }

    /// A 64-bit integer, or a double's bits.
    fn quad(&mut self) -> u64 {
        self.take::<u64>()
    }

    /// An integer of this many bits.
    fn int(&mut self, bits: u32) -> u64 {
        if bits == 64 { self.quad() } else { self.next() }
    }
}

/// A format string's units, narrow or wide.
#[derive(Clone, Copy)]
enum Format {
    Narrow(*const u8),
    Wide(*const u16),
}

impl Format {
    fn at(self, i: usize) -> u32 {
        // SAFETY: the program's format string, read up to its NUL.
        unsafe {
            match self {
                Format::Narrow(p) => u32::from(*p.add(i)),
                Format::Wide(p) => u32::from(*p.add(i)),
            }
        }
    }

    fn wide(self) -> bool {
        matches!(self, Format::Wide(_))
    }
}

/// A conversion's flags, width and precision.
#[derive(Default, Clone, Copy)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Writes a field: `prefix` (a sign, `0x`) and `body`, padded to its width, zeros between them if `zero`.
fn field(sink: &mut dyn Sink, spec: &Spec, prefix: &[u8], body: &[u8], zero: bool) {
    let n = prefix.len() + body.len();
    let pad = spec.width.saturating_sub(n);
    if !spec.left && !zero {
        (0..pad).for_each(|_| sink.byte(b' '));
    }
    prefix.iter().for_each(|&b| sink.byte(b));
    if !spec.left && zero {
        (0..pad).for_each(|_| sink.byte(b'0'));
    }
    body.iter().for_each(|&b| sink.byte(b));
    if spec.left {
        (0..pad).for_each(|_| sink.byte(b' '));
    }
}

/// A buffer for formatting numbers into, as `core::fmt` writes them.
struct Digits {
    b: [u8; 400],
    n: usize,
}

impl core::fmt::Write for Digits {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

impl Digits {
    fn new() -> Digits {
        Digits { b: [0; 400], n: 0 }
    }

    fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

fn integer(sink: &mut dyn Sink, spec: &Spec, value: u64, negative: bool, base: u64, upper: bool, conversion: u8) {
    let mut digits = [0u8; 24];
    let mut n = 0;
    let mut v = value;
    let table = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    while v != 0 {
        digits[n] = table[(v % base) as usize];
        v /= base;
        n += 1;
    }
    digits[..n].reverse();
    let mut body = [0u8; 128];
    let precision = spec.precision.unwrap_or(1);
    let zeros = precision.saturating_sub(n).min(100);
    body[..zeros].fill(b'0');
    body[zeros..zeros + n].copy_from_slice(&digits[..n]);
    let mut len = zeros + n;
    // `%#o` starts with a 0.
    if conversion == b'o' && spec.alternate && body.first() != Some(&b'0') {
        body.copy_within(0..len, 1);
        body[0] = b'0';
        len += 1;
    }
    let mut prefix = [0u8; 2];
    let mut p = 0;
    if negative {
        prefix[0] = b'-';
        p = 1;
    } else if matches!(conversion, b'd' | b'i') && (spec.plus || spec.space) {
        prefix[0] = if spec.plus { b'+' } else { b' ' };
        p = 1;
    } else if base == 16 && spec.alternate && value != 0 {
        prefix = if upper { *b"0X" } else { *b"0x" };
        p = 2;
    }
    field(sink, spec, &prefix[..p], &body[..len], spec.zero && spec.precision.is_none());
}

/// `%e`'s form, from what `core::fmt` gives (`1.5e2`): a signed exponent of at least two digits.
fn exponential(value: f64, precision: usize, upper: bool, out: &mut Digits) {
    use core::fmt::Write;
    let mut raw = Digits::new();
    let _ = write!(raw, "{:.*e}", precision, value);
    let s = raw.get();
    let e = s.iter().position(|&c| c == b'e').unwrap_or(s.len());
    let _ = out.write_str(core::str::from_utf8(&s[..e]).unwrap_or(""));
    let exp: i32 = core::str::from_utf8(&s[(e + 1).min(s.len())..]).ok().and_then(|t| t.parse().ok()).unwrap_or(0);
    let _ = write!(out, "{}{}{:02}", if upper { 'E' } else { 'e' }, if exp < 0 { '-' } else { '+' }, exp.unsigned_abs());
}

/// `%g`'s trailing zeros (and a trailing point) gone, unless `#`.
fn trim(d: &mut Digits) {
    let s = d.get();
    let e = s.iter().position(|&c| c == b'e' || c == b'E').unwrap_or(s.len());
    if !s[..e].contains(&b'.') {
        return;
    }
    let mut end = e;
    while end > 0 && s[end - 1] == b'0' {
        end -= 1;
    }
    if end > 0 && s[end - 1] == b'.' {
        end -= 1;
    }
    let tail: [u8; 8] = {
        let mut t = [0u8; 8];
        let rest = &s[e..];
        t[..rest.len().min(8)].copy_from_slice(&rest[..rest.len().min(8)]);
        t
    };
    let rest = s.len() - e;
    d.n = end;
    for &c in &tail[..rest.min(8)] {
        d.b[d.n] = c;
        d.n += 1;
    }
}

fn floating(sink: &mut dyn Sink, spec: &Spec, value: f64, conversion: u8) {
    use core::fmt::Write;
    let upper = conversion.is_ascii_uppercase();
    let negative = value.is_sign_negative() && !value.is_nan();
    let magnitude = value.abs();
    let prefix: &[u8] = if negative {
        b"-"
    } else if spec.plus {
        b"+"
    } else if spec.space {
        b" "
    } else {
        b""
    };
    if !value.is_finite() {
        let text: &[u8] = match (value.is_nan(), upper) {
            (true, false) => b"nan",
            (true, true) => b"NAN",
            (false, false) => b"inf",
            (false, true) => b"INF",
        };
        field(sink, spec, prefix, text, false);
        return;
    }
    let precision = spec.precision.unwrap_or(6).min(300);
    let mut d = Digits::new();
    match conversion | 0x20 {
        b'f' => {
            let _ = write!(d, "{:.*}", precision, magnitude);
        }
        b'e' => exponential(magnitude, precision, upper, &mut d),
        _ => {
            // %g: %e if the exponent's below -4 or at least the precision, else %f, with as many significant digits.
            let p = precision.max(1);
            let mut probe = Digits::new();
            exponential(magnitude, p - 1, false, &mut probe);
            let s = probe.get();
            let e = s.iter().position(|&c| c == b'e').unwrap_or(0);
            let exp: i32 = core::str::from_utf8(&s[e + 1..]).ok().and_then(|t| t.parse().ok()).unwrap_or(0);
            if exp < -4 || exp >= p as i32 {
                exponential(magnitude, p - 1, upper, &mut d);
            } else {
                let _ = write!(d, "{:.*}", (p as i32 - 1 - exp).max(0) as usize, magnitude);
            }
            if !spec.alternate {
                trim(&mut d);
            }
        }
    }
    if spec.alternate && !d.get().contains(&b'.') {
        let s = d.get();
        let e = s.iter().position(|&c| c == b'e' || c == b'E').unwrap_or(s.len());
        let n = d.n;
        if n < d.b.len() {
            d.b.copy_within(e..n, e + 1);
            d.b[e] = b'.';
            d.n += 1;
        }
    }
    field(sink, spec, prefix, d.get(), spec.zero);
}

/// A string argument, narrow or wide, at most `precision` characters.
fn string(sink: &mut dyn Sink, spec: &Spec, at: u64, wide: bool) {
    let most = spec.precision.unwrap_or(usize::MAX);
    if at == 0 {
        let null: &[u8] = b"(null)";
        field(sink, spec, b"", &null[..most.min(null.len())], false);
        return;
    }
    // SAFETY: the program's string, up to its NUL or `most` units.
    let n = unsafe {
        let mut n = 0;
        if wide {
            while n < most && *(at as *const u16).add(n) != 0 {
                n += 1;
            }
        } else {
            while n < most && *(at as *const u8).add(n) != 0 {
                n += 1;
            }
        }
        n
    };
    let pad = spec.width.saturating_sub(n);
    if !spec.left {
        (0..pad).for_each(|_| sink.byte(b' '));
    }
    for i in 0..n {
        // SAFETY: as above.
        unsafe {
            if wide {
                sink.unit(*(at as *const u16).add(i));
            } else {
                sink.byte(*(at as *const u8).add(i));
            }
        }
    }
    if spec.left {
        (0..pad).for_each(|_| sink.byte(b' '));
    }
}

/// The printf engine: each conversion of the format, with its flags, width and precision (`*` from the arguments),
/// and its size (`hh`, `h`, `l`, `ll`, `L`, `I32`, `I64`, `I`, `j`, `z`, `t`, `w`). In a wide format `%s` and `%c`
/// are wide and `%S` and `%C` narrow; in a narrow one the other way round; `h` is always narrow and `l` and `w` wide.
fn format(sink: &mut dyn Sink, fmt: Format, args: &mut Args) {
    let wide = fmt.wide();
    let mut i = 0;
    loop {
        let c = fmt.at(i);
        if c == 0 {
            return;
        }
        i += 1;
        if c != u32::from(b'%') {
            if wide { sink.unit(c as u16) } else { sink.byte(c as u8) }
            continue;
        }
        let mut spec = Spec::default();
        loop {
            match fmt.at(i) as u8 {
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alternate = true,
                b'0' => spec.zero = true,
                _ => break,
            }
            i += 1;
        }
        if fmt.at(i) == u32::from(b'*') {
            let w = args.next() as i32;
            if w < 0 {
                spec.left = true;
            }
            spec.width = w.unsigned_abs() as usize;
            i += 1;
        } else {
            while (u32::from(b'0')..=u32::from(b'9')).contains(&fmt.at(i)) {
                spec.width = spec.width * 10 + (fmt.at(i) - u32::from(b'0')) as usize;
                i += 1;
            }
        }
        if fmt.at(i) == u32::from(b'.') {
            i += 1;
            if fmt.at(i) == u32::from(b'*') {
                let p = args.next() as i32;
                spec.precision = (p >= 0).then_some(p as usize);
                i += 1;
            } else {
                let mut p = 0;
                while (u32::from(b'0')..=u32::from(b'9')).contains(&fmt.at(i)) {
                    p = p * 10 + (fmt.at(i) - u32::from(b'0')) as usize;
                    i += 1;
                }
                spec.precision = Some(p);
            }
        }
        // The size: 8, 16, 32 or 64 bits; and, for strings and characters, narrow (false) or wide (true).
        let mut bits = 32;
        let mut text_wide = None;
        loop {
            match fmt.at(i) as u8 {
                b'h' if fmt.at(i + 1) == u32::from(b'h') => {
                    bits = 8;
                    i += 2;
                }
                b'h' => {
                    bits = 16;
                    text_wide = Some(false);
                    i += 1;
                }
                b'l' if fmt.at(i + 1) == u32::from(b'l') => {
                    bits = 64;
                    i += 2;
                }
                b'l' | b'w' => {
                    text_wide = Some(true);
                    i += 1;
                }
                b'L' => i += 1,
                b'j' => {
                    bits = 64;
                    i += 1;
                }
                // size_t and ptrdiff_t: a pointer's size.
                b'z' | b't' => {
                    bits = POINTER_BITS;
                    i += 1;
                }
                b'I' if fmt.at(i + 1) == u32::from(b'6') && fmt.at(i + 2) == u32::from(b'4') => {
                    bits = 64;
                    i += 3;
                }
                b'I' if fmt.at(i + 1) == u32::from(b'3') && fmt.at(i + 2) == u32::from(b'2') => {
                    bits = 32;
                    i += 3;
                }
                b'I' => {
                    bits = POINTER_BITS;
                    i += 1;
                }
                _ => break,
            }
        }
        let conversion = fmt.at(i) as u8;
        if conversion == 0 {
            return;
        }
        i += 1;
        let unsigned = |v: u64| if bits == 64 { v } else { v & ((1u64 << bits) - 1) };
        let signed = |v: u64| -> i64 {
            match bits {
                8 => i64::from(v as i8),
                16 => i64::from(v as i16),
                32 => i64::from(v as i32),
                _ => v as i64,
            }
        };
        match conversion {
            b'd' | b'i' => {
                let v = signed(args.int(bits));
                integer(sink, &spec, v.unsigned_abs(), v < 0, 10, false, conversion);
            }
            b'u' => integer(sink, &spec, unsigned(args.int(bits)), false, 10, false, conversion),
            b'x' | b'X' => integer(sink, &spec, unsigned(args.int(bits)), false, 16, conversion == b'X', conversion),
            b'o' => integer(sink, &spec, unsigned(args.int(bits)), false, 8, false, conversion),
            b'p' => {
                let p = Spec { precision: Some(POINTER_BITS as usize / 4), alternate: false, ..spec };
                integer(sink, &p, args.next(), false, 16, true, b'p');
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => floating(sink, &spec, f64::from_bits(args.quad()), if conversion | 0x20 == b'a' { b'e' } else { conversion }),
            b'c' | b'C' => {
                let as_wide = text_wide.unwrap_or(wide == (conversion == b'c'));
                let v = args.next();
                let pad = spec.width.saturating_sub(1);
                if !spec.left {
                    (0..pad).for_each(|_| sink.byte(b' '));
                }
                if as_wide { sink.unit(v as u16) } else { sink.byte(v as u8) }
                if spec.left {
                    (0..pad).for_each(|_| sink.byte(b' '));
                }
            }
            b's' | b'S' => {
                let as_wide = text_wide.unwrap_or(wide == (conversion == b's'));
                string(sink, &spec, args.next(), as_wide);
            }
            // %n writes nothing: UCRT refuses it, as it's how format strings are exploited.
            b'n' => {
                args.next();
            }
            b'%' => sink.byte(b'%'),
            other => {
                sink.byte(b'%');
                sink.byte(other);
            }
        }
    }
}

/// UCRT's options for the `sprintf` family: C99's `snprintf` (the length it would have been, and the buffer cut
/// and terminated) rather than the legacy -1.
const STANDARD_SNPRINTF: u64 = 0x2;

/// Formats into a buffer of `count` units as UCRT's `__stdio_common_vs(n)printf` and `vsw(n)printf` do: the full
/// length when it fits, or with standard behaviour (or no buffer); else -1, the buffer cut and terminated.
fn to_buffer<T: Copy + Default>(options: u64, buffer: *mut T, count: usize, sink_of: impl FnOnce(&mut [T]) -> usize) -> i32 {
    // SAFETY: the program's buffer, `count` units, if it gave one.
    let out: &mut [T] = if buffer.is_null() || count == 0 { &mut [] } else { unsafe { core::slice::from_raw_parts_mut(buffer, count) } };
    let have = out.len();
    let n = sink_of(out);
    if have == 0 {
        return n as i32;
    }
    if n < have {
        // SAFETY: inside the buffer.
        unsafe { *buffer.add(n) = T::default() };
        return n as i32;
    }
    // SAFETY: as above: its last unit.
    unsafe { *buffer.add(have - 1) = T::default() };
    if options & STANDARD_SNPRINTF != 0 { n as i32 } else { -1 }
}

/// `__stdio_common_vfprintf(options, stream, format, locale, args)`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVfprintf(_options: u64, f: *mut File, fmt: *const u8, _locale: usize, args: *const u8) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    let mut sink = ToStream { file, chunk: [0; 256], used: 0, n: 0, failed: false };
    format(&mut sink, Format::Narrow(fmt), &mut Args(args.cast()));
    sink.flush();
    if sink.failed { -1 } else { sink.n as i32 }
}

/// `__stdio_common_vfwprintf(options, stream, format, locale, args)`: to a stream, in the ANSI code page.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVfwprintf(_options: u64, f: *mut File, fmt: *const u16, _locale: usize, args: *const u8) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    let mut sink = ToStream { file, chunk: [0; 256], used: 0, n: 0, failed: false };
    format(&mut sink, Format::Wide(fmt), &mut Args(args.cast()));
    sink.flush();
    if sink.failed { -1 } else { sink.n as i32 }
}

/// `__stdio_common_vsprintf(options, buffer, count, format, locale, args)`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsprintf(options: u64, buffer: *mut u8, count: usize, fmt: *const u8, _locale: usize, args: *const u8) -> i32 {
    to_buffer(options, buffer, count, |out| {
        let mut sink = Buffer { out, n: 0 };
        format(&mut sink, Format::Narrow(fmt), &mut Args(args.cast()));
        sink.count()
    })
}

/// `__stdio_common_vswprintf(options, buffer, count, format, locale, args)`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVswprintf(options: u64, buffer: *mut u16, count: usize, fmt: *const u16, _locale: usize, args: *const u8) -> i32 {
    to_buffer(options, buffer, count, |out| {
        let mut sink = Buffer { out, n: 0 };
        format(&mut sink, Format::Wide(fmt), &mut Args(args.cast()));
        sink.count()
    })
}

/// `__stdio_common_vsprintf_s(options, buffer, size, format, locale, args)`: all of it, or -1 and the buffer emptied.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsprintfS(_options: u64, buffer: *mut u8, size: usize, fmt: *const u8, _locale: usize, args: *const u8) -> i32 {
    if buffer.is_null() || size == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    let n = AtlasCrtVsprintf(STANDARD_SNPRINTF, buffer, size, fmt, 0, args);
    if n < 0 || n as usize >= size {
        // SAFETY: the program's buffer.
        unsafe { *buffer = 0 };
        crate::crt::set_errno(34);
        return -1;
    }
    n
}

/// `__stdio_common_vsnprintf_s(options, buffer, size, max, format, locale, args)`: at most `max` characters
/// (`_TRUNCATE`, all that fits, -1 if that's not all).
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsnprintfS(_options: u64, buffer: *mut u8, size: usize, max: usize, fmt: *const u8, _locale: usize, args: *const u8) -> i32 {
    if buffer.is_null() || size == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    let room = if max == usize::MAX { size } else { (max + 1).min(size) };
    let n = AtlasCrtVsprintf(STANDARD_SNPRINTF, buffer, room, fmt, 0, args);
    if n < 0 || n as usize >= room {
        if max == usize::MAX || (n as usize) > max {
            return -1;
        }
        // SAFETY: as above.
        unsafe { *buffer = 0 };
        crate::crt::set_errno(34);
        return -1;
    }
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVswprintfS(_options: u64, buffer: *mut u16, size: usize, fmt: *const u16, _locale: usize, args: *const u8) -> i32 {
    if buffer.is_null() || size == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    let n = AtlasCrtVswprintf(STANDARD_SNPRINTF, buffer, size, fmt, 0, args);
    if n < 0 || n as usize >= size {
        unsafe { *buffer = 0 };
        crate::crt::set_errno(34);
        return -1;
    }
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsnwprintfS(_options: u64, buffer: *mut u16, size: usize, max: usize, fmt: *const u16, _locale: usize, args: *const u8) -> i32 {
    if buffer.is_null() || size == 0 {
        crate::crt::set_errno(22);
        return -1;
    }
    let room = if max == usize::MAX { size } else { (max + 1).min(size) };
    let n = AtlasCrtVswprintf(STANDARD_SNPRINTF, buffer, room, fmt, 0, args);
    if n < 0 || n as usize >= room {
        if max == usize::MAX || (n as usize) > max {
            return -1;
        }
        unsafe { *buffer = 0 };
        crate::crt::set_errno(34);
        return -1;
    }
    n
}

// ---- scanf ----

trait ScanSource {
    fn peek(&mut self) -> Option<u8>;
    fn next(&mut self) -> Option<u8>;
    fn consumed(&self) -> usize;
}

struct SliceSource<'a> {
    data: &'a [u8],
    pos: usize,
}

impl ScanSource for SliceSource<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let b = self.data.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }
    fn consumed(&self) -> usize {
        self.pos
    }
}

struct FileSource<'a> {
    file: &'a mut File,
    count: usize,
}

impl ScanSource for FileSource<'_> {
    fn peek(&mut self) -> Option<u8> {
        let c = get(self.file);
        if c == EOF {
            return None;
        }
        AtlasCrtUngetc(c, self.file);
        Some(c as u8)
    }
    fn next(&mut self) -> Option<u8> {
        let c = get(self.file);
        if c == EOF {
            return None;
        }
        self.count += 1;
        Some(c as u8)
    }
    fn consumed(&self) -> usize {
        self.count
    }
}

fn is_ascii_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn scan_core(src: &mut dyn ScanSource, fmt: &[u8], args: &mut Args) -> i32 {
    let mut assigned = 0i32;
    let mut fi = 0usize;
    while fi < fmt.len() {
        let fc = fmt[fi];
        fi += 1;
        if is_ascii_space(fc) {
            while src.peek().is_some_and(is_ascii_space) {
                src.next();
            }
            continue;
        }
        if fc != b'%' {
            if src.peek() == Some(fc) {
                src.next();
                continue;
            }
            return if assigned == 0 && src.peek().is_none() { EOF } else { assigned };
        }
        if fi >= fmt.len() {
            break;
        }
        if fmt[fi] == b'%' {
            fi += 1;
            if src.peek() == Some(b'%') {
                src.next();
                continue;
            }
            break;
        }
        let suppress = fmt[fi] == b'*';
        if suppress {
            fi += 1;
        }
        let mut width = 0usize;
        while fi < fmt.len() && fmt[fi].is_ascii_digit() {
            width = width * 10 + usize::from(fmt[fi] - b'0');
            fi += 1;
        }
        let max_w = if width == 0 { usize::MAX } else { width };
        let mut bits = 32u32;
        if fi < fmt.len() {
            match fmt[fi] {
                b'h' if fi + 1 < fmt.len() && fmt[fi + 1] == b'h' => { bits = 8; fi += 2; }
                b'h' => { bits = 16; fi += 1; }
                b'l' if fi + 1 < fmt.len() && fmt[fi + 1] == b'l' => { bits = 64; fi += 2; }
                b'l' => { bits = 33; fi += 1; } // 33 = `long` / `double`
                b'z' | b't' | b'I' => { bits = POINTER_BITS; fi += 1; }
                b'j' => { bits = 64; fi += 1; }
                _ => {}
            }
        }
        if fi >= fmt.len() {
            break;
        }
        let conv = fmt[fi];
        fi += 1;
        if conv == b'n' {
            if !suppress {
                let p = args.next() as usize as *mut i32;
                if !p.is_null() {
                    unsafe { p.write_unaligned(src.consumed() as i32) };
                }
            }
            continue;
        }
        if conv != b'c' && conv != b'[' {
            while src.peek().is_some_and(is_ascii_space) {
                src.next();
            }
        }
        if src.peek().is_none() {
            return if assigned == 0 { EOF } else { assigned };
        }
        match conv {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'p' => {
                let mut buf = [0u8; 64];
                let mut n = 0usize;
                if n < max_w && matches!(src.peek(), Some(b'+' | b'-')) {
                    buf[n] = src.next().unwrap();
                    n += 1;
                }
                let mut base = match conv {
                    b'd' | b'u' => 10,
                    b'o' => 8,
                    b'x' | b'X' | b'p' => 16,
                    _ => 0,
                };
                if n < max_w && src.peek() == Some(b'0') {
                    buf[n] = src.next().unwrap();
                    n += 1;
                    if (base == 0 || base == 16) && n < max_w && matches!(src.peek(), Some(b'x' | b'X')) {
                        buf[n] = src.next().unwrap();
                        n += 1;
                        base = 16;
                    } else if base == 0 {
                        base = 8;
                    }
                } else if base == 0 {
                    base = 10;
                }
                while n < max_w && n + 1 < buf.len() {
                    let Some(c) = src.peek() else { break };
                    let ok = match c {
                        b'0'..=b'7' => true,
                        b'8'..=b'9' => base >= 10,
                        b'a'..=b'f' | b'A'..=b'F' => base == 16,
                        _ => false,
                    };
                    if !ok {
                        break;
                    }
                    buf[n] = src.next().unwrap();
                    n += 1;
                }
                buf[n] = 0;
                let mut end = core::ptr::null();
                let val = if conv == b'd' || conv == b'i' {
                    unsafe { crate::crt::AtlasCrtStrtoll(buf.as_ptr(), &mut end, base) as u64 }
                } else {
                    unsafe { crate::crt::AtlasCrtStrtoull(buf.as_ptr(), &mut end, base) }
                };
                if end == buf.as_ptr() {
                    break;
                }
                if !suppress {
                    let p = args.next() as usize;
                    if p != 0 {
                        unsafe {
                            match if conv == b'p' { POINTER_BITS } else { bits } {
                                8 => (p as *mut u8).write_unaligned(val as u8),
                                16 => (p as *mut u16).write_unaligned(val as u16),
                                64 => (p as *mut u64).write_unaligned(val),
                                _ => (p as *mut u32).write_unaligned(val as u32),
                            }
                        }
                    }
                    assigned += 1;
                }
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let mut buf = [0u8; 128];
                let mut n = 0usize;
                while n < max_w && n + 1 < buf.len() {
                    let Some(c) = src.peek() else { break };
                    if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
                        buf[n] = src.next().unwrap();
                        n += 1;
                    } else {
                        break;
                    }
                }
                buf[n] = 0;
                let mut end = core::ptr::null();
                let val = unsafe { crate::crt::AtlasCrtStrtod(buf.as_ptr(), &mut end) };
                if end == buf.as_ptr() {
                    break;
                }
                if !suppress {
                    let p = args.next() as usize;
                    if p != 0 {
                        unsafe {
                            if bits == 33 || bits == 64 {
                                (p as *mut f64).write_unaligned(val);
                            } else {
                                (p as *mut f32).write_unaligned(val as f32);
                            }
                        }
                    }
                    assigned += 1;
                }
            }
            b's' => {
                let dst = if suppress { core::ptr::null_mut() } else { args.next() as usize as *mut u8 };
                let mut n = 0usize;
                while n < max_w {
                    let Some(c) = src.peek() else { break };
                    if is_ascii_space(c) {
                        break;
                    }
                    src.next();
                    if !dst.is_null() {
                        unsafe { *dst.add(n) = c };
                    }
                    n += 1;
                }
                if n == 0 {
                    break;
                }
                if !dst.is_null() {
                    unsafe { *dst.add(n) = 0 };
                }
                if !suppress {
                    assigned += 1;
                }
            }
            b'c' => {
                let need = if width == 0 { 1 } else { width };
                let dst = if suppress { core::ptr::null_mut() } else { args.next() as usize as *mut u8 };
                let mut n = 0usize;
                while n < need {
                    let Some(c) = src.next() else { break };
                    if !dst.is_null() {
                        unsafe { *dst.add(n) = c };
                    }
                    n += 1;
                }
                if n < need {
                    break;
                }
                if !suppress {
                    assigned += 1;
                }
            }
            b'[' => {
                let invert = fi < fmt.len() && fmt[fi] == b'^';
                if invert {
                    fi += 1;
                }
                let mut table = [false; 256];
                let mut first = true;
                while fi < fmt.len() && (first || fmt[fi] != b']') {
                    first = false;
                    let c = fmt[fi];
                    fi += 1;
                    if fi + 1 < fmt.len() && fmt[fi] == b'-' && fmt[fi + 1] != b']' {
                        let end = fmt[fi + 1];
                        fi += 2;
                        for b in c..=end {
                            table[usize::from(b)] = true;
                        }
                    } else {
                        table[usize::from(c)] = true;
                    }
                }
                if fi < fmt.len() && fmt[fi] == b']' {
                    fi += 1;
                }
                let dst = if suppress { core::ptr::null_mut() } else { args.next() as usize as *mut u8 };
                let mut n = 0usize;
                while n < max_w {
                    let Some(c) = src.peek() else { break };
                    if table[usize::from(c)] == invert {
                        break;
                    }
                    src.next();
                    if !dst.is_null() {
                        unsafe { *dst.add(n) = c };
                    }
                    n += 1;
                }
                if n == 0 {
                    break;
                }
                if !dst.is_null() {
                    unsafe { *dst.add(n) = 0 };
                }
                if !suppress {
                    assigned += 1;
                }
            }
            _ => break,
        }
    }
    assigned
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtVsscanf(_options: u64, buf: *const u8, count: usize, fmt: *const u8, _loc: usize, args: *const u8) -> i32 {
    if buf.is_null() || fmt.is_null() {
        return EOF;
    }
    let len = if count == usize::MAX { unsafe { crate::crt::AtlasCrtStrlen(buf) } } else { unsafe { crate::crt::AtlasCrtStrnlen(buf, count) } };
    let data = unsafe { core::slice::from_raw_parts(buf, len) };
    let fmt_s = unsafe { core::slice::from_raw_parts(fmt, crate::crt::AtlasCrtStrlen(fmt)) };
    let mut src = SliceSource { data, pos: 0 };
    scan_core(&mut src, fmt_s, &mut Args(args))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn AtlasCrtVfscanf(_options: u64, f: *mut File, fmt: *const u8, _loc: usize, args: *const u8) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    if fmt.is_null() {
        return EOF;
    }
    let fmt_s = unsafe { core::slice::from_raw_parts(fmt, crate::crt::AtlasCrtStrlen(fmt)) };
    let mut src = FileSource { file, count: 0 };
    scan_core(&mut src, fmt_s, &mut Args(args))
}

// ---- user32's wsprintf ----

/// The most `wsprintf` writes, as Windows' does: 1024 units and the NUL.
const WSPRINTF_MAX: usize = 1025;

/// `wvsprintfW(buffer, format, args)`: `format` into `buffer`, which Windows takes to hold 1025 units, cut there and
/// terminated. The units written, not counting the NUL.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasWvsprintfW(buffer: *mut u16, fmt: *const u16, args: *const u8) -> i32 {
    if buffer.is_null() || fmt.is_null() {
        return 0;
    }
    // SAFETY: the program's buffer, as large as Windows' contract says.
    let out = unsafe { core::slice::from_raw_parts_mut(buffer, WSPRINTF_MAX) };
    let mut sink = Buffer { out, n: 0 };
    format(&mut sink, Format::Wide(fmt), &mut Args(args));
    let n = sink.count().min(WSPRINTF_MAX - 1);
    // SAFETY: inside the buffer.
    unsafe { *buffer.add(n) = 0 };
    n as i32
}

/// `wvsprintfA(buffer, format, args)`: as `wvsprintfW`, in bytes.
#[unsafe(no_mangle)]
pub extern "system" fn AtlasWvsprintfA(buffer: *mut u8, fmt: *const u8, args: *const u8) -> i32 {
    if buffer.is_null() || fmt.is_null() {
        return 0;
    }
    // SAFETY: as above.
    let out = unsafe { core::slice::from_raw_parts_mut(buffer, WSPRINTF_MAX) };
    let mut sink = Buffer { out, n: 0 };
    format(&mut sink, Format::Narrow(fmt), &mut Args(args));
    let n = sink.count().min(WSPRINTF_MAX - 1);
    // SAFETY: inside the buffer.
    unsafe { *buffer.add(n) = 0 };
    n as i32
}

// `wsprintfW(buffer, format, ...)` and `wsprintfA`: C-variadic, so in assembly, each making a `va_list` of its
// arguments after the format and calling `wvsprintf` with it.

/// x86: cdecl, the arguments on the stack, which is a `va_list` already. `wvsprintf` is stdcall and pops its three.
#[cfg(target_arch = "x86")]
macro_rules! wsprintf {
    ($name:literal, $v:literal) => {
        concat!(
            ".globl _", $name, "\n",
            "_", $name, ":\n",
            "lea eax, [esp + 12]\n",
            "push eax\n",
            "push dword ptr [esp + 12]\n",
            "push dword ptr [esp + 12]\n",
            "call \"_", $v, "@12\"\n",
            "ret\n",
        )
    };
}

/// x86-64: the first four in registers, with home slots above the return address for each. The variadic ones in r8
/// and r9 go to their slots, so that the arguments from the third on lie in a row, and a frame of its own keeps
/// `wvsprintf` from spilling into them.
#[cfg(target_arch = "x86_64")]
macro_rules! wsprintf {
    ($name:literal, $v:literal) => {
        concat!(
            ".globl ", $name, "\n",
            $name, ":\n",
            "mov [rsp + 24], r8\n",
            "mov [rsp + 32], r9\n",
            "lea r8, [rsp + 24]\n",
            "sub rsp, 40\n",
            "call ", $v, "\n",
            "add rsp, 40\n",
            "ret\n",
        )
    };
}

/// ARM64: x0 to x7, then the stack. Windows' variadic convention: the eight are saved just below the arguments on
/// the stack, so the third on lie in a row from x2's slot.
#[cfg(target_arch = "aarch64")]
macro_rules! wsprintf {
    ($name:literal, $v:literal) => {
        concat!(
            ".globl ", $name, "\n",
            $name, ":\n",
            "stp x29, x30, [sp, #-80]!\n",
            "mov x29, sp\n",
            "stp x0, x1, [sp, #16]\n",
            "stp x2, x3, [sp, #32]\n",
            "stp x4, x5, [sp, #48]\n",
            "stp x6, x7, [sp, #64]\n",
            "add x2, sp, #32\n",
            "bl ", $v, "\n",
            "ldp x29, x30, [sp], #80\n",
            "ret\n",
        )
    };
}

core::arch::global_asm!(wsprintf!("AtlasWsprintfW", "AtlasWvsprintfW"), wsprintf!("AtlasWsprintfA", "AtlasWvsprintfA"));
