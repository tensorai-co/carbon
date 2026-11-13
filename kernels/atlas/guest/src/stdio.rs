//! The C runtime's stdio, as `ucrtbase.dll` has it: streams (`FILE`) over kernel32's files, standard input, output
//! and error among them, in text mode (CR LF out for each LF, and back on reading) unless they're binary; and the
//! `printf` family, narrow and wide, as UCRT's `__stdio_common_*` entry points take it, with every conversion C has
//! but `%a` and `%n`.
//!
//! Streams aren't buffered on their way out: each write goes to the file then. Reading has a buffer, for `fgets` and
//! `ungetc`.

use core::sync::atomic::{AtomicBool, Ordering};

use atlas_abi::ansi::{from_1252, to_1252};

use crate::sys::host;

host!(get_std_handle, b"GetStdHandle", (which));
host!(write_file, b"WriteFile", (handle, buffer, count, written, overlapped));
host!(read_file, b"ReadFile", (handle, buffer, count, read, overlapped));
host!(create_file_a, b"CreateFileA", (name, access, share, attributes, disposition, flags, template));
host!(close_handle, b"CloseHandle", (handle));
host!(set_file_pointer_ex, b"SetFilePointerEx", (handle, distance, new, method));

const EOF: i32 = -1;
const BUFFER: usize = 4096;

/// A stream. The program only ever holds a pointer to it.
#[repr(C)]
pub struct File {
    handle: u64,
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
        File { handle: 0, read: which == 0, write: which != 0, append: false, text: true, eof: false, error: false, standard: which + 1, at: 0, have: 0, buffer: [0; BUFFER] }
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
            set_file_pointer_ex(handle, 0, (&raw mut end) as u64, 2);
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
    let plus = mode.contains(&b'+');
    let (read, write, append, disposition) = match mode.first() {
        Some(b'r') => (true, plus, false, 3),
        Some(b'w') if mode.contains(&b'x') => (plus, true, false, 1),
        Some(b'w') => (plus, true, false, 2),
        Some(b'a') => (plus, true, true, 4),
        _ => {
            crate::crt::set_errno(22);
            return core::ptr::null_mut();
        }
    };
    let access = if read { GENERIC_READ } else { 0 } | if write { GENERIC_WRITE } else { 0 };
    let handle = create_file_a(name as u64, access, 3, 0, disposition, 0x80, 0);
    if handle == u64::MAX || handle == 0 {
        // ENOENT, EEXIST, EACCES, as the error was.
        crate::crt::set_errno(match crate::RtlGetLastWin32Error() {
            2 | 3 => 2,
            80 | 183 => 17,
            _ => 13,
        });
        return core::ptr::null_mut();
    }
    let f = crate::heap::allocate(size_of::<File>(), true).cast::<File>();
    if f.is_null() {
        close_handle(handle);
        return f;
    }
    let text = !mode.contains(&b'b');
    // SAFETY: the new stream.
    unsafe { f.write(File { handle, read, write, append, text, eof: false, error: false, standard: 0, at: 0, have: 0, buffer: [0; BUFFER] }) };
    f
}

#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFclose(f: *mut File) -> i32 {
    let Some(file) = stream(f) else { return EOF };
    if file.standard != 0 {
        return 0;
    }
    close_handle(file.handle);
    crate::heap::free(f.cast());
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
    if set_file_pointer_ex(handle, offset as u64, 0, whence as u64) == 0 {
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
    if set_file_pointer_ex(handle, 0, (&raw mut at) as u64, 1) == 0 {
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

/// `_fileno`: 0, 1 and 2 for the standard streams; others by their handle.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtFileno(f: *mut File) -> i32 {
    stream(f).map_or(-1, |f| if f.standard != 0 { i32::from(f.standard - 1) } else { (f.handle & 0x7fff_ffff) as i32 })
}

/// `_setmode(fd, mode)`: _O_TEXT (0x4000) or _O_BINARY (0x8000) for a standard stream; the mode before.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtSetmode(fd: i32, mode: i32) -> i32 {
    if !(0..3).contains(&fd) {
        crate::crt::set_errno(9);
        return -1;
    }
    // SAFETY: the standard streams.
    let file = unsafe { &mut *AtlasCrtIob(fd as u32) };
    let before = if file.text { 0x4000 } else { 0x8000 };
    file.text = mode & 0x8000 == 0;
    before
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

/// The arguments after the format, as a `va_list` has them on both ISAs: one 8-byte slot each, a double's bits
/// too.
struct Args(*const u64);

impl Args {
    fn next(&mut self) -> u64 {
        // SAFETY: the program's va_list, as long as its format says.
        let v = unsafe { self.0.read_unaligned() };
        // SAFETY: as above.
        self.0 = unsafe { self.0.add(1) };
        v
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
                b'j' | b'z' | b't' => {
                    bits = 64;
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
                    bits = 64;
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
                let v = signed(args.next());
                integer(sink, &spec, v.unsigned_abs(), v < 0, 10, false, conversion);
            }
            b'u' => integer(sink, &spec, unsigned(args.next()), false, 10, false, conversion),
            b'x' | b'X' => integer(sink, &spec, unsigned(args.next()), false, 16, conversion == b'X', conversion),
            b'o' => integer(sink, &spec, unsigned(args.next()), false, 8, false, conversion),
            b'p' => {
                let p = Spec { precision: Some(16), alternate: false, ..spec };
                integer(sink, &p, args.next(), false, 16, true, b'p');
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => floating(sink, &spec, f64::from_bits(args.next()), if conversion | 0x20 == b'a' { b'e' } else { conversion }),
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
pub extern "C" fn AtlasCrtVfprintf(_options: u64, f: *mut File, fmt: *const u8, _locale: u64, args: *const u64) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    let mut sink = ToStream { file, chunk: [0; 256], used: 0, n: 0, failed: false };
    format(&mut sink, Format::Narrow(fmt), &mut Args(args));
    sink.flush();
    if sink.failed { -1 } else { sink.n as i32 }
}

/// `__stdio_common_vfwprintf(options, stream, format, locale, args)`: to a stream, in the ANSI code page.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVfwprintf(_options: u64, f: *mut File, fmt: *const u16, _locale: u64, args: *const u64) -> i32 {
    let Some(file) = stream(f) else { return -1 };
    let mut sink = ToStream { file, chunk: [0; 256], used: 0, n: 0, failed: false };
    format(&mut sink, Format::Wide(fmt), &mut Args(args));
    sink.flush();
    if sink.failed { -1 } else { sink.n as i32 }
}

/// `__stdio_common_vsprintf(options, buffer, count, format, locale, args)`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsprintf(options: u64, buffer: *mut u8, count: usize, fmt: *const u8, _locale: u64, args: *const u64) -> i32 {
    to_buffer(options, buffer, count, |out| {
        let mut sink = Buffer { out, n: 0 };
        format(&mut sink, Format::Narrow(fmt), &mut Args(args));
        sink.count()
    })
}

/// `__stdio_common_vswprintf(options, buffer, count, format, locale, args)`.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVswprintf(options: u64, buffer: *mut u16, count: usize, fmt: *const u16, _locale: u64, args: *const u64) -> i32 {
    to_buffer(options, buffer, count, |out| {
        let mut sink = Buffer { out, n: 0 };
        format(&mut sink, Format::Wide(fmt), &mut Args(args));
        sink.count()
    })
}

/// `__stdio_common_vsprintf_s(options, buffer, size, format, locale, args)`: all of it, or -1 and the buffer emptied.
#[unsafe(no_mangle)]
pub extern "C" fn AtlasCrtVsprintfS(_options: u64, buffer: *mut u8, size: usize, fmt: *const u8, _locale: u64, args: *const u64) -> i32 {
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
pub extern "C" fn AtlasCrtVsnprintfS(_options: u64, buffer: *mut u8, size: usize, max: usize, fmt: *const u8, _locale: u64, args: *const u64) -> i32 {
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
