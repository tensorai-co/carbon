//! kernel32's locales and messages: the user's locale (English, United States, 0x409) and what `GetLocaleInfo` says
//! of it, and `FormatMessage`, from a string or the system's messages for the common errors, with its inserts.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::Task;

const EN_US: u64 = 0x409;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INVALID_FLAGS: u32 = 1004;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_MR_MID_NOT_FOUND: u32 = 317;
const LOCALE_RETURN_NUMBER: u32 = 0x2000_0000;
const LOCALE_NOUSEROVERRIDE: u32 = 0x8000_0000;
const LOCALE_USE_CP_ACP: u32 = 0x4000_0000;

pub fn get_user_default_lcid(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(EN_US)
}

/// `IsValidLocale(lcid, flags)`: the locales atlas knows: the user's and system's defaults, English (United States
/// and United Kingdom), French, German and the invariant one.
pub fn is_valid_locale(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(matches!(a[0] as u32, 0x400 | 0x800 | 0x409 | 0x809 | 0x40c | 0x407 | 0x7f)))
}

/// What `GetLocaleInfo` says for an LCTYPE, as text: English (United States)'s, which is every locale's here.
fn locale_info(lctype: u32) -> Option<String> {
    const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let s = |v: &str| Some(v.to_string());
    match lctype {
        0x1 | 0x9 => s("0409"),
        0x2 => s("English (United States)"),
        0x3 => s("ENU"),
        0x4 | 0x1001 | 0x6f => s("English"),
        0x5 | 0xa => s("1"),
        0x6 | 0x1002 | 0x8 | 0x6a | 0x6b => s("United States"),
        0x7 => s("USA"),
        0xb => s("437"),
        0xc => s(","),
        0xd => s("1"),
        0xe | 0x16 => s("."),
        0xf | 0x17 => s(","),
        0x10 | 0x18 => s("3;0"),
        0x11 | 0x19 => s("2"),
        0x12 => s("1"),
        0x13 => s("0123456789"),
        0x14 => s("$"),
        0x15 => s("USD"),
        0x1b | 0x1c | 0x21 | 0x22 | 0x23 => s("0"),
        0x1d => s("/"),
        0x1e => s(":"),
        0x1f => s("M/d/yyyy"),
        0x20 => s("dddd, MMMM d, yyyy"),
        0x28 => s("AM"),
        0x29 => s("PM"),
        0x2a..=0x30 => s(DAYS[(lctype - 0x2a) as usize]),
        0x31..=0x37 => s(&DAYS[(lctype - 0x31) as usize][..3]),
        0x38..=0x43 => s(MONTHS[(lctype - 0x38) as usize]),
        0x44..=0x4f => s(&MONTHS[(lctype - 0x44) as usize][..3]),
        0x50 => s(""),
        0x51 => s("-"),
        0x59 => s("en"),
        0x5a => s("US"),
        0x5c => s("en-US"),
        0x1003 => s("h:mm:ss tt"),
        0x1004 => s("1252"),
        0x100c => s("6"),
        0x1011 => s("10000"),
        0x1014 => s("1"),
        _ => None,
    }
}

/// The number an LCTYPE's text is, for LOCALE_RETURN_NUMBER.
fn locale_number(lctype: u32) -> Option<u32> {
    match lctype {
        0x1 | 0x9 => Some(0x409),
        _ => locale_info(lctype)?.parse().ok(),
    }
}

fn get_locale_info(task: &mut Task, cx: &mut Cx<'_>, lctype: u32, out: u64, room: u64, wide: bool) -> Reply {
    let kind = lctype & !(LOCALE_RETURN_NUMBER | LOCALE_NOUSEROVERRIDE | LOCALE_USE_CP_ACP);
    if lctype & LOCALE_RETURN_NUMBER != 0 {
        let Some(n) = locale_number(kind) else { return task.fail(cx, ERROR_INVALID_FLAGS) };
        let units = if wide { 2 } else { 4 };
        if room == 0 {
            return Reply::Value(units);
        }
        if room < units {
            return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
        }
        return if task.mem(cx, true, |t| user::write_u32(&mut t.space(), out, n)).is_ok() { Reply::Value(units) } else { task.fail(cx, 998) };
    }
    let Some(text) = locale_info(kind) else { return task.fail(cx, ERROR_INVALID_FLAGS) };
    let bytes: Vec<u8> = if wide { text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { crate::ansi::bytes(&text).iter().copied().chain([0]).collect() };
    let count = (bytes.len() / if wide { 2 } else { 1 }) as u64;
    if room == 0 {
        return Reply::Value(count);
    }
    if room < count {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(count)
}

/// `GetLocaleInfoW(lcid, type, out, room)` and `A`: its length with the NUL, or with no room, the room it needs.
pub fn get_locale_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_locale_info(task, cx, a[1] as u32, a[2], a[3] as u32 as u64, true)
}

pub fn get_locale_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_locale_info(task, cx, a[1] as u32, a[2], a[3] as u32 as u64, false)
}

/// `GetLocaleInfoEx(name, type, out, room)`.
pub fn get_locale_info_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_locale_info(task, cx, a[1] as u32, a[2], a[3] as u32 as u64, true)
}

// ---- FormatMessage ----

const ALLOCATE_BUFFER: u32 = 0x100;
const IGNORE_INSERTS: u32 = 0x200;
const FROM_STRING: u32 = 0x400;
const FROM_SYSTEM: u32 = 0x1000;
const ARGUMENT_ARRAY: u32 = 0x2000;

/// The system's message for an error, as Windows words it (and ends it, with CRLF).
fn system_message(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "The operation completed successfully.",
        1 => "Incorrect function.",
        2 => "The system cannot find the file specified.",
        3 => "The system cannot find the path specified.",
        4 => "The system cannot open the file.",
        5 => "Access is denied.",
        6 => "The handle is invalid.",
        8 => "Not enough memory resources are available to process this command.",
        14 => "Not enough memory resources are available to complete this operation.",
        18 => "There are no more files.",
        32 => "The process cannot access the file because it is being used by another process.",
        38 => "Reached the end of the file.",
        50 => "The request is not supported.",
        80 => "The file exists.",
        87 => "The parameter is incorrect.",
        109 => "The pipe has been ended.",
        112 => "There is not enough space on the disk.",
        122 => "The data area passed to a system call is too small.",
        126 => "The specified module could not be found.",
        127 => "The specified procedure could not be found.",
        183 => "Cannot create a file when that file already exists.",
        203 => "The system could not find the environment option that was entered.",
        234 => "More data is available.",
        258 => "The wait operation timed out.",
        259 => "No more data is available.",
        267 => "The directory name is invalid.",
        995 => "The I/O operation has been aborted because of either a thread exit or an application request.",
        996 => "Overlapped I/O event is not in a signaled state.",
        997 => "Overlapped I/O operation is in progress.",
        998 => "Invalid access to memory location.",
        1060 => "The specified service does not exist as an installed service.",
        1168 => "Element not found.",
        1223 => "The operation was canceled by the user.",
        1460 => "This operation returned because the timeout period expired.",
        10035 => "A non-blocking socket operation could not be completed immediately.",
        10038 => "An operation was attempted on something that is not a socket.",
        10048 => "Only one usage of each socket address (protocol/network address/port) is normally permitted.",
        10053 => "An established connection was aborted by the software in your host machine.",
        10054 => "An existing connection was forcibly closed by the remote host.",
        10057 => "A request to send or receive data was disallowed because the socket is not connected and (when sending on a datagram socket using a sendto call) no address was supplied.",
        10060 => "A connection attempt failed because the connected party did not properly respond after a period of time, or established connection failed because connected host has failed to respond.",
        10061 => "No connection could be made because the target machine actively refused it.",
        11001 => "No such host is known.",
        _ => return None,
    })
}

/// One insert's value, from the arguments' array of pointer-sized slots, as its `!format!` says (a string by default).
fn insert(task: &mut Task, cx: &mut Cx<'_>, args: u64, n: usize, format: &str, wide: bool) -> String {
    let w = task.abi().width;
    let Ok(v) = task.mem(cx, false, |t| user::read_ptr(&t.space(), args + (n as u64 - 1) * w, w)) else { return String::new() };
    let narrow = format.contains('h') || (format.contains('S') && wide) || (!wide && !format.contains('l') && !format.contains('S'));
    match format.trim_start_matches(['l', 'h', 'I']).chars().last().unwrap_or('s') {
        's' | 'S' => task.text_arg(cx, v, !narrow),
        'd' | 'i' => (v as u32 as i32).to_string(),
        'u' => (v as u32).to_string(),
        'x' => format!("{:x}", v as u32),
        'X' => format!("{:X}", v as u32),
        'c' => char::from_u32(v as u32).map(String::from).unwrap_or_default(),
        'p' => format!("{v:0width$X}", width = 2 * w as usize),
        _ => (v as u32).to_string(),
    }
}

/// A message's inserts and escapes, as `FormatMessage` makes them: `%1`–`%99` (with `!format!`), `%n` a line end,
/// `%0` the end with none, `%%`, `%!`, `%.`, `%t` and `%r`. (FORMAT_MESSAGE_MAX_WIDTH_MASK's wrapping isn't done.)
fn expand(task: &mut Task, cx: &mut Cx<'_>, text: &str, args: Option<u64>, wide: bool) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] != '%' || i + 1 >= c.len() {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let next = c[i + 1];
        i += 2;
        match next {
            'n' => out.push_str("\r\n"),
            '0' => return out,
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '1'..='9' => {
                let mut n = next.to_digit(10).unwrap_or(0) as usize;
                if i < c.len() && c[i].is_ascii_digit() {
                    n = n * 10 + c[i].to_digit(10).unwrap_or(0) as usize;
                    i += 1;
                }
                let mut format = String::from("s");
                if i < c.len() && c[i] == '!'
                    && let Some(end) = c[i + 1..].iter().position(|&x| x == '!')
                {
                    format = c[i + 1..i + 1 + end].iter().collect();
                    i += end + 2;
                }
                match args {
                    Some(at) => out.push_str(&insert(task, cx, at, n, &format, wide)),
                    None => out.push_str(&format!("%{n}")),
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `FormatMessageW(flags, source, id, language, out, size, args)` and `A`: from FORMAT_MESSAGE_FROM_STRING's string or
/// the system's message for an error (a module's message table isn't read yet), its inserts filled (unless
/// IGNORE_INSERTS) from a va_list or, with ARGUMENT_ARRAY, an array; into the buffer, or with ALLOCATE_BUFFER into
/// one `LocalAlloc` gives. Its length without the NUL; 0 with ERROR_MR_MID_NOT_FOUND for a message there isn't.
fn format_message(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let flags = a[0] as u32;
    let template = if flags & FROM_STRING != 0 {
        task.text_arg(cx, a[1], wide)
    } else if flags & FROM_SYSTEM != 0 {
        match system_message(a[2] as u32) {
            Some(m) => format!("{m}\r\n"),
            None => return task.fail(cx, ERROR_MR_MID_NOT_FOUND),
        }
    } else {
        return task.fail(cx, ERROR_MR_MID_NOT_FOUND);
    };
    // A va_list is a pointer to its arguments' slots, as an array is, on every ISA here; `args` points at the va_list.
    let args = if flags & IGNORE_INSERTS != 0 || a[6] == 0 {
        None
    } else if flags & ARGUMENT_ARRAY != 0 {
        Some(a[6])
    } else {
        let w = task.abi().width;
        task.mem(cx, false, |t| user::read_ptr(&t.space(), a[6], w)).ok()
    };
    let text = expand(task, cx, &template, args, wide);
    let bytes: Vec<u8> = if wide { text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() } else { crate::ansi::bytes(&text).iter().copied().chain([0]).collect() };
    let units = (bytes.len() / if wide { 2 } else { 1 }) as u64;
    if flags & ALLOCATE_BUFFER != 0 {
        let block = super::kernel32::global_new(task, cx, &bytes);
        if block == 0 || task.write_ptr(cx, a[4], block).is_err() {
            return task.fail(cx, 8);
        }
        return Reply::Value(units - 1);
    }
    if u64::from(a[5] as u32) < units {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[4], &bytes)).is_err() {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(units - 1)
}

pub fn format_message_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_message(task, cx, a, true)
}

pub fn format_message_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_message(task, cx, a, false)
}

/// Writes "en-US" as the locale-name calls write a name: its length with the NUL, or 0 with
/// ERROR_INSUFFICIENT_BUFFER; with no room, the room it needs.
fn put_locale_name(task: &mut Task, cx: &mut Cx<'_>, out: u64, room: u64) -> Reply {
    let name: Vec<u8> = "en-US".encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    if room == 0 {
        return Reply::Value(6);
    }
    if room < 6 {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), out, &name)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(6)
}

/// `GetUserDefaultLocaleName(out, room)`.
pub fn get_user_default_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_locale_name(task, cx, a[0], u64::from(a[1] as u32))
}

/// `LCIDToLocaleName(lcid, out, room, flags)`.
pub fn lcid_to_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_locale_name(task, cx, a[1], u64::from(a[2] as u32))
}

/// `ResolveLocaleName(name, out, room)`: every name resolves to en-US, the locale there is.
pub fn resolve_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    put_locale_name(task, cx, a[1], u64::from(a[2] as u32))
}

/// `LocaleNameToLCID(name, flags)`.
pub fn locale_name_to_lcid(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(EN_US)
}

/// `GetUserGeoID(class)`: the United States (244).
pub fn get_user_geo_id(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(244)
}

/// `GetThreadPreferredUILanguages(flags, &count, out, &room)` (and the user's and process's forms): en-US, one.
pub fn get_preferred_ui_languages(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let list: Vec<u8> = "en-US\0\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let Ok(room) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[3])) else { return task.fail(cx, 87) };
    let wrote = task.mem(cx, true, |t| {
        user::write_u32(&mut t.space(), a[1], 1)?;
        user::write_u32(&mut t.space(), a[3], 7)
    });
    if wrote.is_err() {
        return task.fail(cx, 998);
    }
    if a[2] != 0 {
        if room < 7 {
            return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
        }
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &list));
    }
    Reply::Value(1)
}

/// `CompareStringOrdinal(a, length, b, length, ignore case)`: by code unit (uppercased first if asked), as
/// CSTR_LESS_THAN, CSTR_EQUAL or CSTR_GREATER_THAN.
pub fn compare_string_ordinal(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let read = |task: &mut Task, cx: &mut Cx<'_>, at: u64, len: i32| -> Option<Vec<u16>> {
        let text = if len < 0 { task.wstr(cx, at) } else { String::from_utf16_lossy(&task.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize * 2)).ok()?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>()) };
        Some(text.encode_utf16().collect())
    };
    let (Some(mut x), Some(mut y)) = (read(task, cx, a[0], a[1] as u32 as i32), read(task, cx, a[2], a[3] as u32 as i32)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if a[4] as u32 != 0 {
        let upper = |s: &mut Vec<u16>| *s = String::from_utf16_lossy(s).to_uppercase().encode_utf16().collect();
        upper(&mut x);
        upper(&mut y);
    }
    Reply::Value((x.cmp(&y) as i8 + 2) as u64)
}
