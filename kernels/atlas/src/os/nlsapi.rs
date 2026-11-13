//! Code pages, kernel32's face: `MultiByteToWideChar`, `WideCharToMultiByte`, the ANSI and OEM code pages and what
//! they are, over [`crate::ansi`].

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::ansi;
use crate::process::Task;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INVALID_FLAGS: u32 = 1004;
const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;
const ERROR_NOACCESS: u32 = 998;
const MB_ERR_INVALID_CHARS: u32 = 0x8;
const WC_ERR_INVALID_CHARS: u32 = 0x80;
/// The longest string a call takes, NUL-terminated.
const MOST: usize = 1 << 24;

impl Task {
    /// A string of the program's: `len` units, or with -1, up to its NUL and with it. Bytes, or UTF-16's units as
    /// bytes, two each.
    fn counted(&mut self, cx: &mut Cx<'_>, at: u64, len: i32, wide: bool) -> Result<Vec<u8>, u32> {
        let unit = if wide { 2 } else { 1 };
        let read = if len >= 0 {
            self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize * unit))
        } else if wide {
            self.mem(cx, false, |t| {
                let mut out = Vec::new();
                let mut b = [0u8; 2];
                loop {
                    user::read(&t.space(), at + out.len() as u64, &mut b)?;
                    out.extend(b);
                    if b == [0, 0] || out.len() >= MOST {
                        return Ok(out);
                    }
                }
            })
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, MOST)).map(|mut b| {
                b.push(0);
                b
            })
        };
        read.map_err(|_| ERROR_NOACCESS)
    }

    /// Writes a converted string where the program asked, as the conversions do: with no room given, just how much
    /// it would take; with too little, ERROR_INSUFFICIENT_BUFFER.
    fn give_back(&mut self, cx: &mut Cx<'_>, out: u64, room: i32, bytes: &[u8], unit: usize) -> Reply {
        let count = bytes.len() / unit;
        if room == 0 {
            return Reply::Value(count as u64);
        }
        if (room as usize) < count {
            self.set_last_error(cx, ERROR_INSUFFICIENT_BUFFER);
            return Reply::Value(0);
        }
        if self.mem(cx, true, |t| user::write(&mut t.space(), out, bytes)).is_err() {
            self.set_last_error(cx, ERROR_NOACCESS);
            return Reply::Value(0);
        }
        Reply::Value(count as u64)
    }
}

fn failed(task: &mut Task, cx: &mut Cx<'_>, error: u32) -> Reply {
    task.set_last_error(cx, error);
    Reply::Value(0)
}

/// `MultiByteToWideChar(code page, flags, bytes, length, out, room)`.
pub fn multi_byte_to_wide_char(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (flags, len, room) = (a[1] as u32, a[3] as i32, a[5] as i32);
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    if a[2] == 0 || len == 0 || len < -1 || room < 0 || (room > 0 && a[4] == 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    // UTF-8 takes no flags but MB_ERR_INVALID_CHARS.
    if cp == ansi::UTF8 && flags & !MB_ERR_INVALID_CHARS != 0 {
        return failed(task, cx, ERROR_INVALID_FLAGS);
    }
    let bytes = match task.counted(cx, a[2], len, false) {
        Ok(bytes) => bytes,
        Err(e) => return failed(task, cx, e),
    };
    let Ok(units) = ansi::decode(cp, &bytes, flags & MB_ERR_INVALID_CHARS != 0) else { return failed(task, cx, ERROR_NO_UNICODE_TRANSLATION) };
    let out: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    task.give_back(cx, a[4], room, &out, 2)
}

/// `WideCharToMultiByte(code page, flags, units, length, out, room, &default char, &used default)`.
pub fn wide_char_to_multi_byte(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (flags, len, room) = (a[1] as u32, a[3] as i32, a[5] as i32);
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    if a[2] == 0 || len == 0 || len < -1 || room < 0 || (room > 0 && a[4] == 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    // UTF-8 has no default character to give.
    if cp == ansi::UTF8 && (a[6] != 0 || a[7] != 0) {
        return failed(task, cx, ERROR_INVALID_PARAMETER);
    }
    let raw = match task.counted(cx, a[2], len, true) {
        Ok(raw) => raw,
        Err(e) => return failed(task, cx, e),
    };
    let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let default = if a[6] != 0 { task.mem(cx, false, |t| user::read_vec(&t.space(), a[6], 1)).map_or(b'?', |b| b[0]) } else { b'?' };
    let Ok((bytes, used)) = ansi::encode(cp, &units, default, flags & WC_ERR_INVALID_CHARS != 0) else { return failed(task, cx, ERROR_NO_UNICODE_TRANSLATION) };
    if a[7] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[7], u32::from(used)));
    }
    task.give_back(cx, a[4], room, &bytes, 1)
}

pub fn get_acp(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(ansi::ACP))
}

pub fn get_oem_cp(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(ansi::OEMCP))
}

pub fn is_valid_code_page(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    // CP_ACP and the rest aren't code pages themselves.
    Reply::Value(u64::from(a[0] as u32 > 3 && ansi::resolve(a[0] as u32).is_some()))
}

/// CPINFO: the most bytes a character takes, the default character, and no lead bytes.
fn cp_info(cp: u32) -> Vec<u8> {
    let mut b = vec![0u8; 18];
    b[0..4].copy_from_slice(&ansi::max_char_size(cp).to_le_bytes());
    b[4] = b'?';
    b
}

/// `GetCPInfo(code page, &info)`.
pub fn get_cp_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    let b = cp_info(cp);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() { failed(task, cx, ERROR_NOACCESS) } else { Reply::Value(TRUE) }
}

/// `GetCPInfoExW(code page, flags, &info)`: CPINFO, then the Unicode default character, the code page and its name.
pub fn get_cp_info_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(cp) = ansi::resolve(a[0] as u32) else { return failed(task, cx, ERROR_INVALID_PARAMETER) };
    let name = match cp {
        1252 => "1252  (ANSI - Latin I)",
        437 => "437   (OEM - United States)",
        ansi::UTF8 => "65001 (UTF-8)",
        20127 => "20127 (US-ASCII)",
        _ => "28591 (ISO 8859-1 Latin I)",
    };
    let mut b = cp_info(cp);
    b.extend(0x3fu16.to_le_bytes());
    b.extend(cp.to_le_bytes());
    let mut n: Vec<u8> = name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    n.resize(520, 0);
    b.extend(n);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() { failed(task, cx, ERROR_NOACCESS) } else { Reply::Value(TRUE) }
}

/// `IsDBCSLeadByte(Ex)`: none of atlas's code pages has lead bytes.
pub fn is_dbcs_lead_byte(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(FALSE)
}

/// `AreFileApisANSI`: file functions' `*A` strings are ANSI, not OEM.
pub fn are_file_apis_ansi(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(TRUE)
}

/// `SetFileApisToANSI`, `SetFileApisToOEM`: taken, and the ANSI code page kept.
pub fn set_file_apis(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `GetConsoleCP` and `GetConsoleOutputCP`: the console's code page, the OEM one unless it's been set.
pub fn get_console_cp(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(u64::from(task.k.console_cp.load(std::sync::atomic::Ordering::Relaxed)))
}

/// `SetConsoleCP` and `SetConsoleOutputCP`: a code page atlas has.
pub fn set_console_cp(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match ansi::resolve(a[0] as u32).filter(|_| a[0] as u32 > 3) {
        Some(cp) => {
            task.k.console_cp.store(cp, std::sync::atomic::Ordering::Relaxed);
            Reply::Value(TRUE)
        }
        None => {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            Reply::Value(FALSE)
        }
    }
}
