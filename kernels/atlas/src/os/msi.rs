//! msi.dll: Windows Installer runtime — parses MS-CFB installer databases, extracts embedded and external
//! Microsoft Cabinet (`MSCF`, uncompressed and MSZIP) payloads into resolved MSI directory trees, applies
//! Registry table entries, and records installed product metadata in the Windows Registry.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use flate2::{Decompress, FlushDecompress, Status};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use layers::New;

use super::{Reply, dll};
use crate::files::{FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY};
use crate::process::Task;
use crate::registry::{self, Root};
use crate::Meta;

dll! {
    "MSI.dll";
    host MsiCloseHandle(1) @ 8 => msi_close_handle;
    host MsiDatabaseOpenViewW(3) @ 32 => msi_database_open_view_w;
    host MsiGetProductInfoW(4) @ 70 => msi_get_product_info_w;
    host MsiOpenDatabaseW(3) @ 92 => msi_open_database_w;
    host MsiQueryProductStateW(1) @ 113 => msi_query_product_state_w;
    host MsiRecordGetStringW(4) @ 118 => msi_record_get_string_w;
    host MsiSetInternalUI(2) @ 141 => msi_set_internal_ui;
    host MsiViewExecute(2) @ 159 => msi_view_execute;
    host MsiViewFetch(2) @ 160 => msi_view_fetch;
    host MsiEnumRelatedProductsW(4) @ 205 => msi_enum_related_products_w;
    host MsiConfigureProductExW(4) @ 210 => msi_configure_product_ex_w;
    host MsiConfigureProductW(3) => msi_configure_product_w;
    host MsiEnableLogA(3) => msi_enable_log_a;
    host MsiEnableLogW(3) => msi_enable_log_w;
    host MsiEnumProductsA(2) => msi_enum_products_a;
    host MsiEnumProductsW(2) => msi_enum_products_w;
    host MsiGetProductInfoA(4) => msi_get_product_info_a;
    host MsiGetPropertyA(4) => msi_get_property_a;
    host MsiGetPropertyW(4) => msi_get_property_w;
    host MsiInstallProductA(2) => msi_install_product_a;
    host MsiInstallProductW(2) => msi_install_product_w;
    host MsiOpenPackageW(2) => msi_open_package_w;
    host MsiQueryProductStateA(1) => msi_query_product_state_a;
    host MsiSetPropertyA(3) => msi_set_property_a;
    host MsiSetPropertyW(3) => msi_set_property_w;
}

const ERROR_SUCCESS: u64 = 0;
const ERROR_FILE_NOT_FOUND: u64 = 2;
const ERROR_INVALID_HANDLE: u64 = 6;
const ERROR_INVALID_PARAMETER: u64 = 87;
const ERROR_MORE_DATA: u64 = 234;
const ERROR_NO_MORE_ITEMS: u64 = 259;
const ERROR_UNKNOWN_PRODUCT: u64 = 1605;
const ERROR_UNKNOWN_PROPERTY: u64 = 1608;
const ERROR_INSTALL_FAILURE: u64 = 1603;
const ERROR_INSTALL_PACKAGE_OPEN_FAILED: u64 = 1619;
const ERROR_INSTALL_PACKAGE_INVALID: u64 = 1620;

const INSTALLSTATE_DEFAULT: i32 = 5;
const INSTALLSTATE_UNKNOWN: i32 = -1;

static UI_LEVEL: AtomicU32 = AtomicU32::new(5); // INSTALLUILEVEL_FULL
static LOG_FILE: Mutex<Option<String>> = Mutex::new(None);
static HANDLES: Mutex<Option<HandleTable>> = Mutex::new(None);

struct HandleTable {
    next: u32,
    packages: HashMap<u32, HashMap<String, String>>,
}

fn with_handles<R>(f: impl FnOnce(&mut HandleTable) -> R) -> R {
    let mut g = lock(&HANDLES);
    let table = g.get_or_insert_with(|| HandleTable { next: 1, packages: HashMap::new() });
    f(table)
}

// ---- Public MSI.dll exports ----

fn msi_set_internal_ui(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let level = a[0] as u32;
    let prev = if level == 0 {
        UI_LEVEL.load(Ordering::Relaxed)
    } else {
        UI_LEVEL.swap(level, Ordering::Relaxed)
    };
    if a[1] != 0 {
        let _ = task.write_ptr(cx, a[1], 0);
    }
    Reply::Value(u64::from(prev))
}

fn msi_enable_log(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let path = if a[1] == 0 { String::new() } else { task.text_arg(cx, a[1], wide) };
    let mut g = lock(&LOG_FILE);
    if path.is_empty() {
        *g = None;
    } else {
        *g = Some(path);
    }
    Reply::Value(ERROR_SUCCESS)
}

fn msi_enable_log_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_enable_log(task, cx, a, true)
}

fn msi_enable_log_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_enable_log(task, cx, a, false)
}

fn msi_install_product_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_install_product(task, cx, a, true)
}

fn msi_install_product_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_install_product(task, cx, a, false)
}

fn msi_install_product(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if a[0] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let package = task.text_arg(cx, a[0], wide);
    let cmdline = if a[1] == 0 { String::new() } else { task.text_arg(cx, a[1], wide) };
    if package.is_empty() {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }

    let mut log_lines = Vec::new();
    log_lines.push(format!("MSI: installing {package} ({cmdline})"));
    let code = match run_install(task, &package, &cmdline, &mut log_lines) {
        Ok(()) => ERROR_SUCCESS,
        Err((c, msg)) => {
            log_lines.push(format!("MSI error {c}: {msg}"));
            c
        }
    };
    flush_log(task, &log_lines);
    Reply::Value(code)
}

fn flush_log(task: &Task, lines: &[String]) {
    let path = lock(&LOG_FILE).clone();
    let Some(path) = path else { return };
    let mut text = lines.join("\r\n");
    text.push_str("\r\n");
    let _ = task.write_whole(&path, text.as_bytes(), false);
}

fn msi_configure_product_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_configure_product_ex_w(task, cx, &[a[0], a[1], a[2], 0])
}

fn msi_configure_product_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let product = task.text_arg(cx, a[0], true);
    let state = a[2] as u32 as i32;
    if state == 2 {
        // INSTALLSTATE_ABSENT -> uninstall
        let key_path = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}");
        let mut reg = lock(&task.k.registry);
        let root = reg.root(Root::LocalMachine);
        let _ = reg.delete_key(root, &key_path, true);
    }
    Reply::Value(ERROR_SUCCESS)
}

fn uninstall_subkey(task: &Task, product: &str, name: &str) -> Option<String> {
    let reg = lock(&task.k.registry);
    let root = {
        drop(reg);
        let mut r = lock(&task.k.registry);
        r.root(Root::LocalMachine)
    };
    let reg = lock(&task.k.registry);
    let key_path = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}");
    let k = reg.open(root, &key_path).ok()?;
    let v = reg.value(k, name).ok()?;
    Some(registry::text(&v.data))
}

fn msi_query_product_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if a[0] == 0 {
        return Reply::Value(INSTALLSTATE_UNKNOWN as i64 as u64);
    }
    let product = task.text_arg(cx, a[0], wide);
    let installed = uninstall_subkey(task, &product, "DisplayName").is_some();
    Reply::Value((if installed { INSTALLSTATE_DEFAULT } else { INSTALLSTATE_UNKNOWN }) as i64 as u64)
}

fn msi_query_product_state_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_query_product_state(task, cx, a, true)
}

fn msi_query_product_state_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_query_product_state(task, cx, a, false)
}

fn write_msi_string(task: &mut Task, cx: &mut Cx<'_>, out: u64, len_ptr: u64, val: &str, wide: bool) -> Reply {
    let char_len = if wide { val.encode_utf16().count() } else { val.len() };
    if len_ptr == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let Ok(cap) = task.mem(cx, false, |t| user::read_u32(&t.space(), len_ptr)) else {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    };
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), len_ptr, char_len as u32));
    if out == 0 {
        return Reply::Value(ERROR_SUCCESS);
    }
    if (cap as usize) <= char_len {
        return Reply::Value(ERROR_MORE_DATA);
    }
    let bytes: Vec<u8> = if wide {
        val.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    } else {
        val.bytes().chain([0]).collect()
    };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    Reply::Value(ERROR_SUCCESS)
}

fn msi_get_product_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if a[0] == 0 || a[1] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let product = task.text_arg(cx, a[0], wide);
    let prop = task.text_arg(cx, a[1], wide);
    if uninstall_subkey(task, &product, "DisplayName").is_none() {
        return Reply::Value(ERROR_UNKNOWN_PRODUCT);
    }
    let reg_name = match prop.as_str() {
        "InstalledProductName" | "ProductName" => "DisplayName",
        "VersionString" => "DisplayVersion",
        "Publisher" => "Publisher",
        "InstallLocation" => "InstallLocation",
        "LocalPackage" => "LocalPackage",
        _ => return Reply::Value(ERROR_UNKNOWN_PROPERTY),
    };
    let val = uninstall_subkey(task, &product, reg_name).unwrap_or_default();
    write_msi_string(task, cx, a[2], a[3], &val, wide)
}

fn msi_get_product_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_get_product_info(task, cx, a, true)
}

fn msi_get_product_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_get_product_info(task, cx, a, false)
}

fn msi_enum_products(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let idx = a[0] as u32 as usize;
    let out = a[1];
    if out == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let root = lock(&task.k.registry).root(Root::LocalMachine);
    let names: Vec<String> = {
        let reg = lock(&task.k.registry);
        match reg.open(root, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall") {
            Ok(k) => reg
                .key(k)
                .map(|key| key.subkeys.keys().filter(|n| n.starts_with('{') && n.ends_with('}')).cloned().collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    };
    let Some(code) = names.get(idx) else { return Reply::Value(ERROR_NO_MORE_ITEMS) };
    let bytes: Vec<u8> = if wide {
        code.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    } else {
        code.bytes().chain([0]).collect()
    };
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), out, &bytes));
    Reply::Value(ERROR_SUCCESS)
}

fn msi_enum_products_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_enum_products(task, cx, a, true)
}

fn msi_enum_products_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_enum_products(task, cx, a, false)
}

fn msi_open_database_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || a[2] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let path = task.text_arg(cx, a[0], true);
    let Ok(bytes) = task.read_whole(&path) else {
        return Reply::Value(ERROR_INSTALL_PACKAGE_OPEN_FAILED);
    };
    let Ok(db) = MsiDb::parse(&bytes) else {
        return Reply::Value(ERROR_INSTALL_PACKAGE_INVALID);
    };
    let props = db.properties();
    let h = with_handles(|t| {
        let id = t.next;
        t.next += 1;
        t.packages.insert(id, props);
        id
    });
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], h));
    Reply::Value(ERROR_SUCCESS)
}

fn msi_open_package_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_open_database_w(task, cx, &[a[0], 0, a[1]])
}

fn msi_close_handle(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let h = a[0] as u32;
    let removed = with_handles(|t| t.packages.remove(&h).is_some());
    Reply::Value(if removed { ERROR_SUCCESS } else { ERROR_INVALID_HANDLE })
}

fn msi_database_open_view_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], 0));
    }
    Reply::Value(ERROR_INVALID_HANDLE)
}

fn msi_view_execute(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(ERROR_INVALID_HANDLE)
}

fn msi_view_fetch(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 0));
    }
    Reply::Value(ERROR_NO_MORE_ITEMS)
}

fn msi_record_get_string_w(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(ERROR_INVALID_HANDLE)
}

fn msi_enum_related_products_w(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(ERROR_NO_MORE_ITEMS)
}

fn msi_get_property(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let h = a[0] as u32;
    if a[1] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let name = task.text_arg(cx, a[1], wide);
    let Some(val) = with_handles(|t| t.packages.get(&h).map(|p| p.get(&name).cloned().unwrap_or_default())) else {
        return Reply::Value(ERROR_INVALID_HANDLE);
    };
    write_msi_string(task, cx, a[2], a[3], &val, wide)
}

fn msi_get_property_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_get_property(task, cx, a, true)
}

fn msi_get_property_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_get_property(task, cx, a, false)
}

fn msi_set_property(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let h = a[0] as u32;
    if a[1] == 0 {
        return Reply::Value(ERROR_INVALID_PARAMETER);
    }
    let name = task.text_arg(cx, a[1], wide);
    let val = if a[2] == 0 { String::new() } else { task.text_arg(cx, a[2], wide) };
    let ok = with_handles(|t| {
        if let Some(p) = t.packages.get_mut(&h) {
            if val.is_empty() {
                p.remove(&name);
            } else {
                p.insert(name, val);
            }
            true
        } else {
            false
        }
    });
    Reply::Value(if ok { ERROR_SUCCESS } else { ERROR_INVALID_HANDLE })
}

fn msi_set_property_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_set_property(task, cx, a, true)
}

fn msi_set_property_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    msi_set_property(task, cx, a, false)
}

// ---- MS-CFB (Compound File Binary) reader ----

const CFBF_SIG: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const ENDOFCHAIN: u32 = 0xffff_fffe;
const FREESECT: u32 = 0xffff_ffff;
const MAXREGSECT: u32 = 0xffff_fffb;

const MSI_B64: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz._";

fn decode_msi_stream_name(u16s: &[u16]) -> String {
    let mut out = String::new();
    for &c in u16s {
        if (0x3800..0x4800).contains(&c) {
            let v = (c - 0x3800) as usize;
            out.push(MSI_B64[v & 63] as char);
            out.push(MSI_B64[v >> 6] as char);
        } else if (0x4800..0x4840).contains(&c) {
            out.push(MSI_B64[(c - 0x4800) as usize] as char);
        } else if c == 0x4840 {
            out.push('!');
        } else if let Some(ch) = char::from_u32(u32::from(c)) {
            out.push(ch);
        }
    }
    out
}

struct Cfbf<'a> {
    data: &'a [u8],
    sec_size: usize,
    mini_size: usize,
    mini_cutoff: u64,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini_stream: Vec<u8>,
    /// Root storage's direct child streams: decoded name -> (start_sector, size).
    streams: HashMap<String, (u32, u64)>,
}

impl<'a> Cfbf<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        if data.len() < 512 || data[..8] != CFBF_SIG {
            return Err("not a compound binary file".into());
        }
        let u16_at = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);

        let major = u16_at(26);
        let sec_shift = u16_at(30);
        let mini_shift = u16_at(32);
        if !(9..=16).contains(&sec_shift) || !(3..=12).contains(&mini_shift) {
            return Err("invalid sector shift".into());
        }
        let sec_size = 1usize << sec_shift;
        let mini_size = 1usize << mini_shift;
        let num_fat = u32_at(44) as usize;
        let first_dir = u32_at(48);
        let mini_cutoff = u64::from(u32_at(56));
        let first_minifat = u32_at(60);
        let first_difat = u32_at(68);
        let num_difat = u32_at(72) as usize;

        let mut difat = Vec::with_capacity(109 + num_difat * (sec_size / 4));
        for i in 0..109 {
            difat.push(u32_at(76 + i * 4));
        }
        let mut cur = first_difat;
        let per_sec = sec_size / 4;
        for _ in 0..num_difat {
            if cur > MAXREGSECT {
                break;
            }
            let off = (cur as usize + 1).checked_mul(sec_size).ok_or("difat overflow")?;
            if off + sec_size > data.len() {
                break;
            }
            for i in 0..per_sec - 1 {
                difat.push(u32_at(off + i * 4));
            }
            cur = u32_at(off + (per_sec - 1) * 4);
        }

        let mut fat = Vec::with_capacity(num_fat * per_sec);
        for &s in difat.iter().take(num_fat) {
            if s > MAXREGSECT {
                break;
            }
            let off = (s as usize + 1).checked_mul(sec_size).ok_or("fat overflow")?;
            if off + sec_size > data.len() {
                return Err("truncated FAT sector".into());
            }
            for i in 0..per_sec {
                fat.push(u32_at(off + i * 4));
            }
        }

        let read_chain = |start: u32, limit: Option<usize>| -> Result<Vec<u8>, String> {
            let mut out = Vec::with_capacity(limit.unwrap_or(sec_size));
            let mut s = start;
            let mut hops = 0usize;
            while s <= MAXREGSECT {
                let off = (s as usize + 1).checked_mul(sec_size).ok_or("chain overflow")?;
                if off + sec_size > data.len() {
                    return Err("truncated sector chain".into());
                }
                out.extend_from_slice(&data[off..off + sec_size]);
                if let Some(lim) = limit
                    && out.len() >= lim
                {
                    out.truncate(lim);
                    break;
                }
                hops += 1;
                if hops > fat.len() + 1 {
                    return Err("cycle in FAT chain".into());
                }
                s = *fat.get(s as usize).unwrap_or(&ENDOFCHAIN);
            }
            Ok(out)
        };

        let dir_bytes = read_chain(first_dir, None)?;
        let minifat_bytes = if first_minifat <= MAXREGSECT { read_chain(first_minifat, None)? } else { Vec::new() };
        let minifat: Vec<u32> = minifat_bytes.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();

        let dir_count = dir_bytes.len() / 128;
        if dir_count == 0 {
            return Err("empty CFBF directory".into());
        }
        let parse_entry = |idx: usize| -> Option<(String, u8, u32, u32, u32, u32, u64)> {
            let e = dir_bytes.get(idx * 128..(idx + 1) * 128)?;
            let nlen = u16::from_le_bytes([e[64], e[65]]) as usize;
            let etype = e[66];
            let left = u32::from_le_bytes(e[68..72].try_into().ok()?);
            let right = u32::from_le_bytes(e[72..76].try_into().ok()?);
            let child = u32::from_le_bytes(e[76..80].try_into().ok()?);
            let start = u32::from_le_bytes(e[116..120].try_into().ok()?);
            let size = if major == 4 {
                u64::from_le_bytes(e[120..128].try_into().ok()?)
            } else {
                u64::from(u32::from_le_bytes(e[120..124].try_into().ok()?))
            };
            let u16_count = (nlen / 2).saturating_sub(1).min(32);
            let mut u16s = Vec::with_capacity(u16_count);
            for i in 0..u16_count {
                u16s.push(u16::from_le_bytes([e[i * 2], e[i * 2 + 1]]));
            }
            Some((decode_msi_stream_name(&u16s), etype, left, right, child, start, size))
        };

        let (_, _, _, _, root_child, root_start, root_size) = parse_entry(0).ok_or("missing root entry")?;
        let mini_stream = if root_start <= MAXREGSECT && root_size > 0 {
            read_chain(root_start, Some(root_size as usize))?
        } else {
            Vec::new()
        };

        let mut streams = HashMap::new();
        let mut stack = Vec::new();
        if root_child != FREESECT {
            stack.push(root_child as usize);
        }
        let mut seen = vec![false; dir_count];
        while let Some(idx) = stack.pop() {
            if idx >= dir_count || seen[idx] {
                continue;
            }
            seen[idx] = true;
            if let Some((name, etype, left, right, _child, start, size)) = parse_entry(idx) {
                if left != FREESECT {
                    stack.push(left as usize);
                }
                if right != FREESECT {
                    stack.push(right as usize);
                }
                if etype == 2 {
                    streams.insert(name, (start, size));
                }
            }
        }

        Ok(Cfbf { data, sec_size, mini_size, mini_cutoff, fat, minifat, mini_stream, streams })
    }

    fn read_stream(&self, name: &str) -> Result<Vec<u8>, String> {
        let &(start, size) = self.streams.get(name).ok_or_else(|| format!("missing stream {name}"))?;
        let size = size as usize;
        if size == 0 {
            return Ok(Vec::new());
        }
        if (size as u64) < self.mini_cutoff {
            let mut out = Vec::with_capacity(size);
            let mut s = start;
            let mut hops = 0usize;
            while s <= MAXREGSECT && out.len() < size {
                let off = (s as usize).checked_mul(self.mini_size).ok_or("mini overflow")?;
                let end = (off + self.mini_size).min(self.mini_stream.len());
                if off >= end {
                    break;
                }
                out.extend_from_slice(&self.mini_stream[off..end]);
                hops += 1;
                if hops > self.minifat.len() + 1 {
                    return Err("cycle in MiniFAT".into());
                }
                s = *self.minifat.get(s as usize).unwrap_or(&ENDOFCHAIN);
            }
            out.truncate(size);
            return Ok(out);
        }
        let mut out = Vec::with_capacity(size);
        let mut s = start;
        let mut hops = 0usize;
        while s <= MAXREGSECT && out.len() < size {
            let off = (s as usize + 1).checked_mul(self.sec_size).ok_or("stream overflow")?;
            if off + self.sec_size > self.data.len() {
                return Err("truncated stream sector".into());
            }
            out.extend_from_slice(&self.data[off..off + self.sec_size]);
            hops += 1;
            if hops > self.fat.len() + 1 {
                return Err("cycle in FAT".into());
            }
            s = *self.fat.get(s as usize).unwrap_or(&ENDOFCHAIN);
        }
        out.truncate(size);
        Ok(out)
    }

    fn find_stream_case_insensitive(&self, wanted: &str) -> Option<String> {
        if self.streams.contains_key(wanted) {
            return Some(wanted.to_string());
        }
        let low = wanted.to_ascii_lowercase();
        self.streams.keys().find(|k| k.to_ascii_lowercase() == low).cloned()
    }
}

// ---- MSI relational table parser ----

#[derive(Clone, Debug)]
struct ColDef {
    num: i32,
    name: String,
    col_type: u16,
}

#[derive(Clone, Debug)]
enum Cell {
    Str(String),
    Int(i32),
    Null,
}

impl Cell {
    fn as_str(&self) -> &str {
        match self {
            Cell::Str(s) => s,
            _ => "",
        }
    }

    fn as_int(&self) -> Option<i32> {
        match self {
            Cell::Int(i) => Some(*i),
            _ => None,
        }
    }
}

struct MsiDb<'a> {
    cfb: Cfbf<'a>,
    strings: Vec<String>,
    str_id_bytes: usize,
    schema: HashMap<String, Vec<ColDef>>,
}

fn decode_cp1252(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from_u32(u32::from(atlas_abi::ansi::from_1252(b))).unwrap_or('\u{fffd}')).collect()
}

impl<'a> MsiDb<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        let cfb = Cfbf::parse(data)?;
        let sp = cfb.read_stream("!_StringPool")?;
        let sd = cfb.read_stream("!_StringData")?;
        if sp.len() < 4 {
            return Err("truncated _StringPool".into());
        }
        let flags = u16::from_le_bytes([sp[2], sp[3]]);
        let str_id_bytes = if flags & 0x8000 != 0 { 4 } else { 2 };

        let mut strings = Vec::with_capacity(sp.len() / 4 + 1);
        strings.push(String::new()); // ID 0 is empty/NULL
        let mut pos = 0usize;
        let mut i = 4usize;
        while i + 4 <= sp.len() {
            let mut len = u16::from_le_bytes([sp[i], sp[i + 1]]) as usize;
            let refs = u16::from_le_bytes([sp[i + 2], sp[i + 3]]);
            i += 4;
            if len == 0 && refs != 0 && i + 4 <= sp.len() {
                len = u32::from_le_bytes(sp[i..i + 4].try_into().unwrap()) as usize;
                i += 4;
            }
            let end = pos.saturating_add(len).min(sd.len());
            let s = if pos < end { decode_cp1252(&sd[pos..end]) } else { String::new() };
            pos = end;
            strings.push(s);
        }

        let cols_raw = cfb.read_stream("!_Columns")?;
        let col_row_w = str_id_bytes + 2 + str_id_bytes + 2;
        let nrows = cols_raw.len() / col_row_w;
        let mut o = 0usize;
        let read_ids = |buf: &[u8], off: &mut usize, n: usize, w: usize| -> Vec<usize> {
            let mut out = Vec::with_capacity(n);
            for k in 0..n {
                let at = *off + k * w;
                let v = if w == 2 {
                    u16::from_le_bytes([buf[at], buf[at + 1]]) as usize
                } else {
                    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize
                };
                out.push(v);
            }
            *off += n * w;
            out
        };
        let t_ids = read_ids(&cols_raw, &mut o, nrows, str_id_bytes);
        let nums = read_ids(&cols_raw, &mut o, nrows, 2);
        let n_ids = read_ids(&cols_raw, &mut o, nrows, str_id_bytes);
        let types = read_ids(&cols_raw, &mut o, nrows, 2);

        let mut schema: HashMap<String, Vec<ColDef>> = HashMap::new();
        for idx in 0..nrows {
            let tbl = strings.get(t_ids[idx]).cloned().unwrap_or_default();
            let name = strings.get(n_ids[idx]).cloned().unwrap_or_default();
            let num = (nums[idx] as u16).wrapping_sub(0x8000) as i16 as i32;
            let col_type = types[idx] as u16;
            schema.entry(tbl).or_default().push(ColDef { num, name, col_type });
        }
        for defs in schema.values_mut() {
            defs.sort_by_key(|c| c.num);
        }

        Ok(MsiDb { cfb, strings, str_id_bytes, schema })
    }

    fn read_table(&self, table: &str) -> Vec<HashMap<String, Cell>> {
        let Some(defs) = self.schema.get(table) else { return Vec::new() };
        let Ok(buf) = self.cfb.read_stream(&format!("!{table}")) else { return Vec::new() };
        let mut widths = Vec::with_capacity(defs.len());
        for d in defs {
            let w = if d.col_type & 0x0800 != 0 {
                self.str_id_bytes
            } else if d.col_type & 0x0400 != 0 {
                2
            } else if (d.col_type & 0xff) == 2 {
                2
            } else if (d.col_type & 0xff) == 4 {
                4
            } else {
                2
            };
            widths.push(w);
        }
        let row_w: usize = widths.iter().sum();
        if row_w == 0 {
            return Vec::new();
        }
        let nrows = buf.len() / row_w;
        let mut rows: Vec<HashMap<String, Cell>> = (0..nrows).map(|_| HashMap::with_capacity(defs.len())).collect();
        let mut off = 0usize;
        for (def, &w) in defs.iter().zip(widths.iter()) {
            let is_str = def.col_type & 0x0800 != 0;
            for r in &mut rows {
                let cell = if is_str {
                    let id = if w == 2 {
                        u16::from_le_bytes([buf[off], buf[off + 1]]) as usize
                    } else {
                        u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize
                    };
                    if id == 0 {
                        Cell::Null
                    } else {
                        Cell::Str(self.strings.get(id).cloned().unwrap_or_default())
                    }
                } else if w == 2 {
                    let raw = u16::from_le_bytes([buf[off], buf[off + 1]]);
                    if raw == 0 { Cell::Null } else { Cell::Int((raw ^ 0x8000) as i16 as i32) }
                } else {
                    let raw = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
                    if raw == 0 { Cell::Null } else { Cell::Int((raw ^ 0x8000_0000) as i32) }
                };
                off += w;
                r.insert(def.name.clone(), cell);
            }
        }
        rows
    }

    fn properties(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for row in self.read_table("Property") {
            let k = row.get("Property").map_or("", Cell::as_str);
            let v = row.get("Value").map_or("", Cell::as_str);
            if !k.is_empty() {
                out.insert(k.to_string(), v.to_string());
            }
        }
        out
    }
}

// ---- Microsoft Cabinet (MSCF) extractor (NONE & MSZIP) ----

struct CabFile {
    name: String,
    uoff: usize,
    usize_bytes: usize,
}

struct CabSpan {
    uoff: usize,
    size: usize,
    names: Vec<String>,
}

struct CabFolder {
    data_off: usize,
    num_blocks: usize,
    comp_type: u16,
}

fn extract_cab<F>(cab: &[u8], mut on_file: F) -> Result<usize, String>
where
    F: FnMut(&str, &[u8]) -> Result<(), String>,
{
    if cab.len() < 36 || &cab[..4] != b"MSCF" {
        return Err("invalid CAB signature".into());
    }
    let u16_at = |o: usize| u16::from_le_bytes([cab[o], cab[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([cab[o], cab[o + 1], cab[o + 2], cab[o + 3]]);

    let coff_files = u32_at(16) as usize;
    let cfolders = u16_at(26) as usize;
    let cfiles = u16_at(28) as usize;
    let flags = u16_at(30);

    let mut pos = 36usize;
    let (mut cb_cffolder, mut cb_cfdata) = (0usize, 0usize);
    if flags & 0x0004 != 0 {
        if pos + 4 > cab.len() {
            return Err("truncated CAB reserve header".into());
        }
        let cb_cfheader = u16_at(pos) as usize;
        cb_cffolder = cab[pos + 2] as usize;
        cb_cfdata = cab[pos + 3] as usize;
        pos += 4 + cb_cfheader;
    }
    // Skip optional prev/next cabinet names when present.
    let skip_cstr = |p: &mut usize| {
        while *p < cab.len() && cab[*p] != 0 {
            *p += 1;
        }
        if *p < cab.len() {
            *p += 1;
        }
    };
    if flags & 0x0001 != 0 {
        skip_cstr(&mut pos);
        skip_cstr(&mut pos);
    }
    if flags & 0x0002 != 0 {
        skip_cstr(&mut pos);
        skip_cstr(&mut pos);
    }

    let mut folders = Vec::with_capacity(cfolders);
    for _ in 0..cfolders {
        if pos + 8 + cb_cffolder > cab.len() {
            return Err("truncated CFFOLDER".into());
        }
        let data_off = u32_at(pos) as usize;
        let num_blocks = u16_at(pos + 4) as usize;
        let comp_type = u16_at(pos + 6) & 0x000f;
        folders.push(CabFolder { data_off, num_blocks, comp_type });
        pos += 8 + cb_cffolder;
    }

    let mut by_folder: Vec<Vec<CabFile>> = (0..cfolders).map(|_| Vec::new()).collect();
    let mut fpos = coff_files;
    for _ in 0..cfiles {
        if fpos + 16 > cab.len() {
            return Err("truncated CFFILE".into());
        }
        let usize_bytes = u32_at(fpos) as usize;
        let uoff = u32_at(fpos + 4) as usize;
        let folder = u16_at(fpos + 8) as usize;
        fpos += 16;
        let start = fpos;
        while fpos < cab.len() && cab[fpos] != 0 {
            fpos += 1;
        }
        let name = String::from_utf8_lossy(&cab[start..fpos]).into_owned();
        if fpos < cab.len() {
            fpos += 1;
        }
        if folder < cfolders {
            by_folder[folder].push(CabFile { name, uoff, usize_bytes });
        }
    }

    let mut extracted = 0usize;
    for (fi, folder) in folders.iter().enumerate() {
        let files = &mut by_folder[fi];
        if files.is_empty() {
            continue;
        }
        files.sort_by_key(|f| (f.uoff, f.usize_bytes));

        // Emit zero-byte files immediately and group duplicate (uoff, size) entries
        // (produced by smart-cabbing) into shared spans so each span is buffered once.
        let mut spans: Vec<CabSpan> = Vec::new();
        for f in files.drain(..) {
            if f.usize_bytes == 0 {
                on_file(&f.name, &[])?;
                extracted += 1;
                continue;
            }
            if let Some(last) = spans.last_mut()
                && last.uoff == f.uoff
                && last.size == f.usize_bytes
            {
                last.names.push(f.name);
            } else {
                spans.push(CabSpan { uoff: f.uoff, size: f.usize_bytes, names: vec![f.name] });
            }
        }
        if spans.is_empty() {
            continue;
        }

        let mut dpos = folder.data_off;
        let mut prev_block = Vec::<u8>::new();
        let mut folder_offset = 0usize;
        let mut next_span = 0usize;
        let mut active: Vec<(usize, Vec<u8>)> = Vec::new();

        for _ in 0..folder.num_blocks {
            if next_span >= spans.len() && active.is_empty() {
                break;
            }
            if dpos + 8 + cb_cfdata > cab.len() {
                return Err("truncated CFDATA header".into());
            }
            let cb_data = u16_at(dpos + 4) as usize;
            let cb_uncomp = u16_at(dpos + 6) as usize;
            dpos += 8 + cb_cfdata;
            if dpos + cb_data > cab.len() {
                return Err("truncated CFDATA payload".into());
            }
            let payload = &cab[dpos..dpos + cb_data];
            dpos += cb_data;
            if cb_uncomp == 0 {
                continue;
            }

            let uncomp = match folder.comp_type {
                0 => payload.to_vec(),
                1 => decompress_mszip(payload, cb_uncomp, &prev_block)?,
                other => return Err(format!("unsupported CAB compression type {other}")),
            };
            prev_block = uncomp;

            let block_start = folder_offset;
            let block_end = folder_offset + prev_block.len();

            while next_span < spans.len() && spans[next_span].uoff < block_end {
                active.push((next_span, Vec::with_capacity(spans[next_span].size)));
                next_span += 1;
            }

            let mut i = 0;
            while i < active.len() {
                let (si, ref mut buf) = active[i];
                let span = &spans[si];
                let need_off = span.uoff + buf.len();
                let span_end = span.uoff + span.size;
                if need_off < block_end && span_end > block_start {
                    let take_start = need_off.saturating_sub(block_start);
                    let take_end = span_end.min(block_end) - block_start;
                    if take_start < take_end && take_end <= prev_block.len() {
                        buf.extend_from_slice(&prev_block[take_start..take_end]);
                    }
                }
                if buf.len() >= span.size {
                    let (si, buf) = active.swap_remove(i);
                    let span = &spans[si];
                    for name in &span.names {
                        on_file(name, &buf[..span.size])?;
                        extracted += 1;
                    }
                } else {
                    i += 1;
                }
            }
            folder_offset = block_end;
        }
    }
    Ok(extracted)
}

fn decompress_mszip(payload: &[u8], expected: usize, prev_dict: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() < 2 || &payload[..2] != b"CK" {
        return Err("invalid MSZIP block signature".into());
    }
    let dict = if prev_dict.len() > 32768 { &prev_dict[prev_dict.len() - 32768..] } else { prev_dict };
    // Prime the raw-DEFLATE 32 KiB sliding window with `dict` via an uncompressed non-final stored block
    // (BFINAL=0, BTYPE=00), leaving the bitstream byte-aligned for the MSZIP block immediately following.
    let mut input = Vec::with_capacity(if dict.is_empty() { 0 } else { 5 + dict.len() } + payload.len() - 2);
    if !dict.is_empty() {
        let len = dict.len() as u16;
        input.push(0x00);
        input.extend_from_slice(&len.to_le_bytes());
        input.extend_from_slice(&(!len).to_le_bytes());
        input.extend_from_slice(dict);
    }
    input.extend_from_slice(&payload[2..]);

    let mut dec = Decompress::new(false);
    let mut out = vec![0u8; dict.len() + expected.max(32768) + 256];
    let status = dec
        .decompress(&input, &mut out, FlushDecompress::Finish)
        .map_err(|e| format!("MSZIP inflate error: {e}"))?;
    if matches!(status, Status::BufError) && (dec.total_out() as usize) < dict.len() + expected {
        let written = dec.total_out() as usize;
        let consumed = dec.total_in() as usize;
        out.resize(dict.len() + expected.max(65536) + 256, 0);
        let _ = dec.decompress(&input[consumed..], &mut out[written..], FlushDecompress::Finish);
    }
    let total = dec.total_out() as usize;
    if total < dict.len() {
        return Err("MSZIP dictionary priming truncated".into());
    }
    Ok(out[dict.len()..total].to_vec())
}

// ---- MSI installer execution ----

/// Parses MSI `KEY=VALUE` property pairs from a command-line string.
fn parse_properties(cmdline: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let bytes = cmdline.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        let key = &cmdline[start..i];
        i += 1; // skip '='
        let mut val = String::new();
        if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                        val.push('"');
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    val.push(bytes[i] as char);
                    i += 1;
                }
            }
        } else {
            let vstart = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            val.push_str(&cmdline[vstart..i]);
        }
        if !key.is_empty() {
            out.insert(key.to_string(), val);
        }
    }
    out
}

/// Extracts the target long name from MSI `DefaultDir` syntax (`target_short|target_long:source_short|source_long`).
fn default_dir_name(raw: &str) -> &str {
    let target = raw.split(':').next().unwrap_or(raw);
    let long = target.rsplit('|').next().unwrap_or(target);
    if long == "." || long.eq_ignore_ascii_case("SourceDir") { "" } else { long }
}

/// Extracts the long filename from MSI `FileName` syntax (`short|long`).
fn msi_long_name(raw: &str) -> &str {
    raw.rsplit('|').next().unwrap_or(raw)
}

fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('\\') { s.to_string() } else { format!("{s}\\") }
}

fn standard_dirs() -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in [
        ("TARGETDIR", "C:\\"),
        ("ROOTDRIVE", "C:\\"),
        ("ProgramFilesFolder", "C:\\Program Files (x86)\\"),
        ("ProgramFiles64Folder", "C:\\Program Files\\"),
        ("CommonFilesFolder", "C:\\Program Files (x86)\\Common Files\\"),
        ("CommonFiles64Folder", "C:\\Program Files\\Common Files\\"),
        ("CommonAppDataFolder", "C:\\ProgramData\\"),
        ("AppDataFolder", "C:\\Users\\User\\AppData\\Roaming\\"),
        ("APPDATA", "C:\\ProgramData\\"),
        ("LocalAppDataFolder", "C:\\Users\\User\\AppData\\Local\\"),
        ("WindowsFolder", "C:\\Windows\\"),
        ("SystemFolder", "C:\\Windows\\SysWOW64\\"),
        ("System64Folder", "C:\\Windows\\System32\\"),
        ("DesktopFolder", "C:\\Users\\User\\Desktop\\"),
        ("ProgramMenuFolder", "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\"),
        ("StartMenuFolder", "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\"),
        ("StartupFolder", "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\"),
        ("TempFolder", "C:\\Users\\User\\AppData\\Local\\Temp\\"),
        ("PersonalFolder", "C:\\Users\\User\\Documents\\"),
        ("FavoritesFolder", "C:\\Users\\User\\Favorites\\"),
        ("FontsFolder", "C:\\Windows\\Fonts\\"),
    ] {
        m.insert(k.to_string(), v.to_string());
    }
    m
}

fn resolve_directories(
    dir_rows: &[HashMap<String, Cell>],
    props: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut table: HashMap<String, (String, String)> = HashMap::new();
    for r in dir_rows {
        let id = r.get("Directory").map_or("", Cell::as_str);
        let parent = r.get("Directory_Parent").map_or("", Cell::as_str);
        let def = r.get("DefaultDir").map_or("", Cell::as_str);
        if !id.is_empty() {
            table.insert(id.to_string(), (parent.to_string(), def.to_string()));
        }
    }

    let std_dirs = standard_dirs();
    let mut cache: HashMap<String, String> = HashMap::new();

    fn resolve_one(
        id: &str,
        table: &HashMap<String, (String, String)>,
        std_dirs: &HashMap<String, String>,
        props: &HashMap<String, String>,
        cache: &mut HashMap<String, String>,
        depth: usize,
    ) -> String {
        if let Some(p) = cache.get(id) {
            return p.clone();
        }
        if depth > 64 {
            return "C:\\".to_string();
        }
        // Explicit property override (e.g. INSTALLDIR=C:\Custom or TARGETDIR=C:\Extract).
        if let Some(ov) = props.get(id)
            && (ov.contains(":\\") || ov.starts_with("\\\\"))
        {
            let p = ensure_trailing_slash(ov);
            cache.insert(id.to_string(), p.clone());
            return p;
        }
        let base_id = id.split('.').next().unwrap_or(id);
        if let Some(s) = std_dirs.get(base_id) {
            cache.insert(id.to_string(), s.clone());
            return s.clone();
        }
        let Some((parent, def)) = table.get(id) else {
            return "C:\\".to_string();
        };
        let base = if parent.is_empty() || parent == id {
            "C:\\".to_string()
        } else {
            resolve_one(parent, table, std_dirs, props, cache, depth + 1)
        };
        let seg = default_dir_name(def);
        let full = if seg.is_empty() { base } else { ensure_trailing_slash(&format!("{base}{seg}")) };
        cache.insert(id.to_string(), full.clone());
        full
    }

    for id in table.keys() {
        let _ = resolve_one(id, &table, &std_dirs, props, &mut cache, 0);
    }
    cache
}

/// Expands MSI formatted strings (`[Property]`, `[#FileKey]`, `[$CompKey]`, `[~]`).
fn format_msi_string(
    input: &str,
    props: &HashMap<String, String>,
    dirs: &HashMap<String, String>,
    file_paths: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            out.push('[');
            out.push_str(after);
            return out;
        };
        let token = &after[..close];
        if token == "~" {
            out.push('\0');
        } else if let Some(fkey) = token.strip_prefix('#') {
            if let Some(p) = file_paths.get(fkey) {
                out.push_str(p);
            }
        } else if let Some(val) = dirs.get(token).or_else(|| props.get(token)) {
            out.push_str(val);
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn win_to_unix(path: &str) -> String {
    let stripped = path
        .strip_prefix("C:\\")
        .or_else(|| path.strip_prefix("c:\\"))
        .or_else(|| path.strip_prefix("C:/"))
        .or_else(|| path.strip_prefix("c:/"))
        .unwrap_or(path);
    format!("/{}", stripped.replace('\\', "/").trim_start_matches('/'))
}

fn run_install(task: &mut Task, package: &str, cmdline: &str, log: &mut Vec<String>) -> Result<(), (u64, String)> {
    let bytes = task.read_whole(package).map_err(|_| (ERROR_INSTALL_PACKAGE_OPEN_FAILED, format!("cannot read {package}")))?;
    let db = MsiDb::parse(&bytes).map_err(|e| (ERROR_INSTALL_PACKAGE_INVALID, format!("invalid MSI database: {e}")))?;

    let mut props = db.properties();
    let overrides = parse_properties(cmdline);
    let is_uninstall = overrides.get("REMOVE").is_some_and(|v| v.eq_ignore_ascii_case("ALL"));
    for (k, v) in &overrides {
        props.insert(k.clone(), v.clone());
    }
    for (k, v) in standard_dirs() {
        props.entry(k).or_insert(v);
    }

    let product_code = props.get("ProductCode").cloned().unwrap_or_default();
    let product_name = props.get("ProductName").cloned().unwrap_or_default();
    let product_ver = props.get("ProductVersion").cloned().unwrap_or_default();
    let manufacturer = props.get("Manufacturer").cloned().unwrap_or_default();
    log.push(format!("Product: {product_name} {product_ver} {product_code}"));

    if is_uninstall {
        if !product_code.is_empty() {
            let mut reg = lock(&task.k.registry);
            let root = reg.root(Root::LocalMachine);
            let _ = reg.delete_key(root, &format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product_code}"), true);
        }
        return Ok(());
    }

    let dir_rows = db.read_table("Directory");
    let dirs = resolve_directories(&dir_rows, &props);
    for (k, v) in &dirs {
        props.insert(k.clone(), v.clone());
    }

    // Component -> Directory_
    let comp_rows = db.read_table("Component");
    let mut comp_dir: HashMap<String, String> = HashMap::new();
    for r in &comp_rows {
        let c = r.get("Component").map_or("", Cell::as_str);
        let d = r.get("Directory_").map_or("", Cell::as_str);
        if !c.is_empty() {
            comp_dir.insert(c.to_string(), d.to_string());
        }
    }

    // FileKey -> destination Windows path
    let file_rows = db.read_table("File");
    let mut file_dests: HashMap<String, String> = HashMap::with_capacity(file_rows.len());
    let mut file_dests_lower: HashMap<String, String> = HashMap::with_capacity(file_rows.len());
    for r in &file_rows {
        let fkey = r.get("File").map_or("", Cell::as_str);
        let comp = r.get("Component_").map_or("", Cell::as_str);
        let fname = msi_long_name(r.get("FileName").map_or("", Cell::as_str));
        if fkey.is_empty() || fname.is_empty() {
            continue;
        }
        let dir_id = comp_dir.get(comp).map(String::as_str).unwrap_or("TARGETDIR");
        let dir_path = dirs.get(dir_id).map(String::as_str).unwrap_or("C:\\");
        let full = format!("{}{}", ensure_trailing_slash(dir_path), fname);
        file_dests_lower.insert(fkey.to_ascii_lowercase(), full.clone());
        file_dests.insert(fkey.to_string(), full);
    }

    // Extract cabinets listed in Media table directly onto Atlas's disk.
    let media_rows = db.read_table("Media");
    let mut cabinets: BTreeMap<i32, String> = BTreeMap::new();
    for r in &media_rows {
        let disk_id = r.get("DiskId").and_then(Cell::as_int).unwrap_or(0);
        let cab = r.get("Cabinet").map_or("", Cell::as_str);
        if !cab.is_empty() {
            cabinets.insert(disk_id, cab.to_string());
        }
    }

    let pkg_dir = package.rsplit_once(['\\', '/']).map_or("", |(d, _)| d);
    let dir_meta = Meta { attributes: FILE_ATTRIBUTE_DIRECTORY };
    let file_meta = Meta { attributes: FILE_ATTRIBUTE_ARCHIVE };
    let mut total_written = 0usize;

    for cab_spec in cabinets.values() {
        let cab_bytes = if let Some(stream_name) = cab_spec.strip_prefix('#') {
            let actual = db
                .cfb
                .find_stream_case_insensitive(stream_name)
                .ok_or_else(|| (ERROR_INSTALL_PACKAGE_INVALID, format!("missing embedded cabinet {stream_name}")))?;
            db.cfb.read_stream(&actual).map_err(|e| (ERROR_INSTALL_PACKAGE_INVALID, e))?
        } else {
            let ext_path = if pkg_dir.is_empty() { cab_spec.clone() } else { format!("{pkg_dir}\\{cab_spec}") };
            task.read_whole(&ext_path).map_err(|_| (ERROR_FILE_NOT_FOUND, format!("missing external cabinet {ext_path}")))?
        };

        let mut disk = lock(&task.k.disk);
        let n = extract_cab(&cab_bytes, |cab_file_key, contents| {
            let Some(dest_win) = file_dests.get(cab_file_key).or_else(|| file_dests_lower.get(&cab_file_key.to_ascii_lowercase())) else {
                return Ok(());
            };
            let unix = win_to_unix(dest_win);
            disk.put(&unix, New::File, contents, file_meta, dir_meta).map_err(|e| format!("writing {dest_win}: {e}"))?;
            total_written += 1;
            Ok(())
        })
        .map_err(|e| (ERROR_INSTALL_FAILURE, format!("extracting {cab_spec}: {e}")))?;
        log.push(format!("Cabinet {cab_spec}: processed {n} entries ({total_written} files written)"));
    }

    // Create explicit empty directories from CreateFolder.
    {
        let mut disk = lock(&task.k.disk);
        for r in db.read_table("CreateFolder") {
            let dir_id = r.get("Directory_").map_or("", Cell::as_str);
            if let Some(path) = dirs.get(dir_id) {
                let unix = win_to_unix(path.trim_end_matches('\\'));
                if unix.len() > 1 {
                    let _ = disk.put(&unix, New::Dir, &[], dir_meta, dir_meta);
                }
            }
        }
    }

    // Apply Registry table entries.
    let now = crate::files::filetime(task.k.host.wall());
    let reg_rows = db.read_table("Registry");
    {
        let mut reg = lock(&task.k.registry);
        for r in &reg_rows {
            let root_num = r.get("Root").and_then(Cell::as_int).unwrap_or(-1);
            let root_enum = match root_num {
                0 => Root::ClassesRoot,
                1 => Root::CurrentUser,
                2 | -1 => Root::LocalMachine,
                3 => Root::Users,
                _ => continue,
            };
            let key_raw = r.get("Key").map_or("", Cell::as_str);
            let name_raw = r.get("Name").map_or("", Cell::as_str);
            let val_raw = r.get("Value").map_or("", Cell::as_str);
            let key_path = format_msi_string(key_raw, &props, &dirs, &file_dests);
            if key_path.is_empty() {
                continue;
            }
            let root_h = reg.root(root_enum);
            let Ok((k, _)) = reg.create(root_h, &key_path, false, now) else { continue };
            if matches!(name_raw, "+" | "-" | "*") {
                continue;
            }
            let val_name = format_msi_string(name_raw, &props, &dirs, &file_dests);
            let expanded = format_msi_string(val_raw, &props, &dirs, &file_dests);
            let (kind, data) = encode_msi_reg_value(&expanded);
            let _ = reg.set_value(k, &val_name, kind, data, now);
        }

        // Record installed product in Uninstall registry key.
        if !product_code.is_empty() {
            let root_h = reg.root(Root::LocalMachine);
            let uninstall_key = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product_code}");
            if let Ok((k, _)) = reg.create(root_h, &uninstall_key, false, now) {
                let put_sz = |reg: &mut registry::Registry, name: &str, s: &str| {
                    let b: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
                    let _ = reg.set_value(k, name, 1, b, now);
                };
                if !product_name.is_empty() {
                    put_sz(&mut reg, "DisplayName", &product_name);
                }
                if !product_ver.is_empty() {
                    put_sz(&mut reg, "DisplayVersion", &product_ver);
                }
                if !manufacturer.is_empty() {
                    put_sz(&mut reg, "Publisher", &manufacturer);
                }
                let install_loc = dirs
                    .get("INSTALLDIR")
                    .or_else(|| dirs.get("INSTALLLOCATION"))
                    .or_else(|| dirs.get("APPLICATIONFOLDER"))
                    .map(String::as_str)
                    .unwrap_or("C:\\");
                put_sz(&mut reg, "InstallLocation", install_loc);
                put_sz(&mut reg, "LocalPackage", package);
                let _ = reg.set_value(k, "WindowsInstaller", 4, 1u32.to_le_bytes().to_vec(), now);
            }
        }
    }

    log.push(format!("MSI: installation completed ({total_written} files)"));
    Ok(())
}

/// Encodes an MSI Registry Value string into `(REG_* type, bytes)`:
/// `#<int>` -> REG_DWORD (4), `#%<str>` -> REG_EXPAND_SZ (2), `#x<hex>` -> REG_BINARY (3),
/// `##<str>` -> literal `#<str>` REG_SZ (1), containing `\0` -> REG_MULTI_SZ (7), else REG_SZ (1).
fn encode_msi_reg_value(val: &str) -> (u32, Vec<u8>) {
    if let Some(rest) = val.strip_prefix("##") {
        let s = format!("#{rest}");
        return (1, s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect());
    }
    if let Some(rest) = val.strip_prefix("#%") {
        return (2, rest.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect());
    }
    if let Some(hex) = val.strip_prefix("#x").or_else(|| val.strip_prefix("#X")) {
        let mut out = Vec::with_capacity(hex.len() / 2);
        let bytes = hex.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&hex[i..i + 2], 16) {
                out.push(b);
            }
            i += 2;
        }
        return (3, out);
    }
    if let Some(num) = val.strip_prefix('#')
        && let Ok(n) = num.parse::<i64>()
    {
        return (4, (n as u32).to_le_bytes().to_vec());
    }
    if val.contains('\0') {
        let mut units: Vec<u16> = val.encode_utf16().collect();
        units.push(0);
        units.push(0);
        return (7, units.into_iter().flat_map(u16::to_le_bytes).collect());
    }
    (1, val.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    #[test]
    fn decodes_msi_stream_names() {
        // '!' (0x4840) + "Fi" + "le"
        let fi = 0x3800 + (MSI_B64.iter().position(|&b| b == b'F').unwrap() as u16) + ((MSI_B64.iter().position(|&b| b == b'i').unwrap() as u16) << 6);
        let le = 0x3800 + (MSI_B64.iter().position(|&b| b == b'l').unwrap() as u16) + ((MSI_B64.iter().position(|&b| b == b'e').unwrap() as u16) << 6);
        assert_eq!(decode_msi_stream_name(&[0x4840, fi, le]), "!File");
    }

    #[test]
    fn parses_default_dir_and_properties() {
        assert_eq!(default_dir_name("vcymgiuq|DirectXRedist"), "DirectXRedist");
        assert_eq!(default_dir_name("short|Long:src"), "Long");
        assert_eq!(default_dir_name("."), "");
        let p = parse_properties("INSTALLDIR=\"C:\\My App\" ALLUSERS=1 REMOVE=ALL");
        assert_eq!(p.get("INSTALLDIR").map(String::as_str), Some("C:\\My App"));
        assert_eq!(p.get("ALLUSERS").map(String::as_str), Some("1"));
        assert_eq!(p.get("REMOVE").map(String::as_str), Some("ALL"));
    }

    #[test]
    fn extracts_multiblock_mszip_cab() {
        // Compress two blocks where block 2 reuses block 1 as dictionary.
        let b1 = b"hello world from block one! ".repeat(100);
        let b2 = b"hello world from block one! and block two!".repeat(50);

        let compress_block = |data: &[u8]| -> Vec<u8> {
            let mut c = Compress::new(Compression::default(), false);
            let mut out = vec![0u8; data.len() + 64];
            let _ = c.compress(data, &mut out, FlushCompress::Finish);
            out.truncate(c.total_out() as usize);
            let mut mszip = vec![b'C', b'K'];
            mszip.extend_from_slice(&out);
            mszip
        };

        let c1 = compress_block(&b1);
        let c2 = compress_block(&b2);
        let mut combined = b1.clone();
        combined.extend_from_slice(&b2);

        // Build minimal MSCF cabinet with 1 folder (MSZIP), 3 files (including duplicate span).
        let split = b1.len() + 17;
        let f1_name = b"first.bin\0";
        let f2_name = b"second.bin\0";
        let f3_name = b"dup_first.bin\0";
        let coff_files: u32 = 36 + 8;
        let data_off: u32 = coff_files + (16 + f1_name.len() as u32) + (16 + f2_name.len() as u32) + (16 + f3_name.len() as u32);
        let mut cab = Vec::new();
        cab.extend_from_slice(b"MSCF");
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes()); // cbCabinet patched later
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&coff_files.to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&[3, 1]); // v1.3
        cab.extend_from_slice(&1u16.to_le_bytes()); // cFolders
        cab.extend_from_slice(&3u16.to_le_bytes()); // cFiles
        cab.extend_from_slice(&0u16.to_le_bytes()); // flags
        cab.extend_from_slice(&0u16.to_le_bytes()); // setID
        cab.extend_from_slice(&0u16.to_le_bytes()); // iCabinet
        // CFFOLDER
        cab.extend_from_slice(&data_off.to_le_bytes());
        cab.extend_from_slice(&2u16.to_le_bytes()); // cCFData
        cab.extend_from_slice(&1u16.to_le_bytes()); // MSZIP
        // CFFILE 1
        cab.extend_from_slice(&(split as u32).to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        cab.extend_from_slice(&[0u8; 6]);
        cab.extend_from_slice(f1_name);
        // CFFILE 2
        cab.extend_from_slice(&((combined.len() - split) as u32).to_le_bytes());
        cab.extend_from_slice(&(split as u32).to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        cab.extend_from_slice(&[0u8; 6]);
        cab.extend_from_slice(f2_name);
        // CFFILE 3 (deduplicated copy of CFFILE 1 at same offset/size)
        cab.extend_from_slice(&(split as u32).to_le_bytes());
        cab.extend_from_slice(&0u32.to_le_bytes());
        cab.extend_from_slice(&0u16.to_le_bytes());
        cab.extend_from_slice(&[0u8; 6]);
        cab.extend_from_slice(f3_name);
        // CFDATA 1 & 2
        for (c, u) in [(&c1, b1.len()), (&c2, b2.len())] {
            cab.extend_from_slice(&0u32.to_le_bytes());
            cab.extend_from_slice(&(c.len() as u16).to_le_bytes());
            cab.extend_from_slice(&(u as u16).to_le_bytes());
            cab.extend_from_slice(c);
        }
        let total = cab.len() as u32;
        cab[8..12].copy_from_slice(&total.to_le_bytes());

        let mut got: HashMap<String, Vec<u8>> = HashMap::new();
        let count = extract_cab(&cab, |name, data| {
            got.insert(name.to_string(), data.to_vec());
            Ok(())
        })
        .expect("extract");
        assert_eq!(count, 3);
        assert_eq!(got["first.bin"], combined[..split]);
        assert_eq!(got["dup_first.bin"], combined[..split]);
        assert_eq!(got["second.bin"], combined[split..]);
    }
}
