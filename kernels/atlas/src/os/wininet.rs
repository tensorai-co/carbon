//! wininet.dll: URL cracking and WinINet exports.

use exec::Isa;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WININET.dll";
    missing HttpAddRequestHeadersW;
    missing HttpOpenRequestW;
    missing HttpQueryInfoW;
    missing HttpSendRequestW;
    missing InternetCloseHandle;
    missing InternetConnectW;
    host InternetCrackUrlW(4) => internet_crack_url_w;
    missing InternetOpenW;
    missing InternetQueryDataAvailable;
    missing InternetReadFile;
    missing InternetSetOptionW;
    // Not written yet: what larger programs import.
    missing GetUrlCacheEntryInfoExW;
}

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INTERNET_UNRECOGNIZED_SCHEME: u32 = 12006;

const INTERNET_SCHEME_UNKNOWN: i32 = 0;
const INTERNET_SCHEME_FTP: i32 = 1;
const INTERNET_SCHEME_GOPHER: i32 = 2;
const INTERNET_SCHEME_HTTP: i32 = 3;
const INTERNET_SCHEME_HTTPS: i32 = 4;
const INTERNET_SCHEME_FILE: i32 = 5;

struct FieldOffsets {
    size: usize,
    scheme_ptr: usize,
    scheme_len: usize,
    scheme_id: usize,
    host_ptr: usize,
    host_len: usize,
    port: usize,
    user_ptr: usize,
    user_len: usize,
    pass_ptr: usize,
    pass_len: usize,
    path_ptr: usize,
    path_len: usize,
    extra_ptr: usize,
    extra_len: usize,
}

const OFF_32: FieldOffsets = FieldOffsets {
    size: 60,
    scheme_ptr: 4,
    scheme_len: 8,
    scheme_id: 12,
    host_ptr: 16,
    host_len: 20,
    port: 24,
    user_ptr: 28,
    user_len: 32,
    pass_ptr: 36,
    pass_len: 40,
    path_ptr: 44,
    path_len: 48,
    extra_ptr: 52,
    extra_len: 56,
};

const OFF_64: FieldOffsets = FieldOffsets {
    size: 104,
    scheme_ptr: 8,
    scheme_len: 16,
    scheme_id: 20,
    host_ptr: 24,
    host_len: 32,
    port: 36,
    user_ptr: 40,
    user_len: 48,
    pass_ptr: 56,
    pass_len: 64,
    path_ptr: 72,
    path_len: 80,
    extra_ptr: 88,
    extra_len: 96,
};

/// `InternetCrackUrlW(url, url_len, flags, &components)`.
fn internet_crack_url_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (url_at, url_len, _flags, comp_at) = (a[0], a[1] as u32, a[2] as u32, a[3]);
    if url_at == 0 || comp_at == 0 {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    }
    let x32 = task.isa == Isa::X86_32;
    let off = if x32 { &OFF_32 } else { &OFF_64 };
    let Ok(mut comp) = task.mem(cx, false, |t| user::read_vec(&t.space(), comp_at, off.size)) else {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    };
    let struct_size = u32::from_le_bytes(comp[0..4].try_into().unwrap()) as usize;
    if struct_size < off.size {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    }
    // Read the URL as UTF-16 code units so offsets back into `url_at` are exact.
    let units: Vec<u16> = if url_len == 0 {
        let Ok(s) = task.mem(cx, false, |t| user::read_wstr(&t.space(), url_at, 65536)) else {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Reply::Value(0);
        };
        s.encode_utf16().collect()
    } else {
        let Ok(raw) = task.mem(cx, false, |t| user::read_vec(&t.space(), url_at, (url_len as usize).min(65536) * 2)) else {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Reply::Value(0);
        };
        raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    };

    // Find `:` separating scheme.
    let Some(colon) = units.iter().position(|&c| c == b':' as u16) else {
        task.set_last_error(cx, ERROR_INTERNET_UNRECOGNIZED_SCHEME);
        return Reply::Value(0);
    };
    let scheme_slice = 0..colon;
    let scheme_str = String::from_utf16_lossy(&units[scheme_slice.clone()]).to_ascii_lowercase();
    let (scheme_id, default_port) = match scheme_str.as_str() {
        "http" => (INTERNET_SCHEME_HTTP, 80u16),
        "https" => (INTERNET_SCHEME_HTTPS, 443u16),
        "ftp" => (INTERNET_SCHEME_FTP, 21u16),
        "gopher" => (INTERNET_SCHEME_GOPHER, 70u16),
        "file" => (INTERNET_SCHEME_FILE, 0u16),
        _ => (INTERNET_SCHEME_UNKNOWN, 0u16),
    };

    let mut pos = colon + 1;
    let has_slashes = units.get(pos..pos + 2) == Some(&[b'/' as u16, b'/' as u16]);
    if has_slashes {
        pos += 2;
    }

    let (user_slice, pass_slice, host_slice, port, path_start) = if has_slashes {
        // Authority ends at '/', '?', '#' or end of string.
        let auth_end = units[pos..].iter().position(|&c| matches!(c as u8, b'/' | b'?' | b'#') && c < 128).map(|i| pos + i).unwrap_or(units.len());
        let auth = pos..auth_end;
        // Optional userinfo before '@'.
        let (u_slice, p_slice, hostport_start) = if let Some(at_rel) = units[auth.clone()].iter().position(|&c| c == b'@' as u16) {
            let at = auth.start + at_rel;
            if let Some(c_rel) = units[auth.start..at].iter().position(|&c| c == b':' as u16) {
                (auth.start..auth.start + c_rel, auth.start + c_rel + 1..at, at + 1)
            } else {
                (auth.start..at, at..at, at + 1)
            }
        } else {
            (auth.start..auth.start, auth.start..auth.start, auth.start)
        };
        // Host and optional :port (handling IPv6 `[...]`).
        let hp = &units[hostport_start..auth_end];
        let (h_slice, port) = if hp.first() == Some(&(b'[' as u16)) {
            if let Some(rb) = hp.iter().position(|&c| c == b']' as u16) {
                let port = if hp.get(rb + 1) == Some(&(b':' as u16)) {
                    String::from_utf16_lossy(&hp[rb + 2..]).parse::<u16>().unwrap_or(default_port)
                } else {
                    default_port
                };
                (hostport_start..hostport_start + rb + 1, port)
            } else {
                (hostport_start..auth_end, default_port)
            }
        } else if let Some(c_rel) = hp.iter().rposition(|&c| c == b':' as u16) {
            let port = String::from_utf16_lossy(&hp[c_rel + 1..]).parse::<u16>().unwrap_or(default_port);
            (hostport_start..hostport_start + c_rel, port)
        } else {
            (hostport_start..auth_end, default_port)
        };
        (u_slice, p_slice, h_slice, port, auth_end)
    } else {
        (pos..pos, pos..pos, pos..pos, default_port, pos)
    };

    // Split path vs extra info (? or #).
    let extra_pos = units[path_start..].iter().position(|&c| c == b'?' as u16 || c == b'#' as u16).map(|i| path_start + i).unwrap_or(units.len());
    let path_slice = path_start..extra_pos;
    let extra_slice = extra_pos..units.len();

    comp[off.scheme_id..off.scheme_id + 4].copy_from_slice(&scheme_id.to_le_bytes());
    comp[off.port..off.port + 2].copy_from_slice(&port.to_le_bytes());

    let read_ptr = |buf: &[u8], at: usize| -> u64 {
        if x32 {
            u64::from(u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()))
        } else {
            u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
        }
    };
    let write_ptr = |buf: &mut [u8], at: usize, v: u64| {
        if x32 {
            buf[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes());
        } else {
            buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
        }
    };

    let fields = [
        (off.scheme_ptr, off.scheme_len, scheme_slice),
        (off.host_ptr, off.host_len, host_slice),
        (off.user_ptr, off.user_len, user_slice),
        (off.pass_ptr, off.pass_len, pass_slice),
        (off.path_ptr, off.path_len, path_slice),
        (off.extra_ptr, off.extra_len, extra_slice),
    ];

    let mut insufficient = false;
    for (p_off, l_off, range) in fields {
        let ptr = read_ptr(&comp, p_off);
        let cap = u32::from_le_bytes(comp[l_off..l_off + 4].try_into().unwrap()) as usize;
        let len = range.end - range.start;
        if ptr != 0 {
            if cap <= len {
                comp[l_off..l_off + 4].copy_from_slice(&((len + 1) as u32).to_le_bytes());
                insufficient = true;
            } else {
                let mut bytes = Vec::with_capacity((len + 1) * 2);
                for &u in &units[range] {
                    bytes.extend_from_slice(&u.to_le_bytes());
                }
                bytes.extend_from_slice(&0u16.to_le_bytes());
                if task.mem(cx, true, |t| user::write(&mut t.space(), ptr, &bytes)).is_err() {
                    task.set_last_error(cx, ERROR_INVALID_PARAMETER);
                    return Reply::Value(0);
                }
                comp[l_off..l_off + 4].copy_from_slice(&(len as u32).to_le_bytes());
            }
        } else if cap != 0 {
            let out_ptr = if len == 0 { 0 } else { url_at + (range.start as u64) * 2 };
            write_ptr(&mut comp, p_off, out_ptr);
            comp[l_off..l_off + 4].copy_from_slice(&(len as u32).to_le_bytes());
        }
    }

    if task.mem(cx, true, |t| user::write(&mut t.space(), comp_at, &comp)).is_err() {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    }
    if insufficient {
        task.set_last_error(cx, ERROR_INSUFFICIENT_BUFFER);
        return Reply::Value(0);
    }
    Reply::Value(1)
}
