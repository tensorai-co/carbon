//! Resources from the program's side: kernel32's `FindResource`, `LoadResource` and the enumerations, user32's
//! `LoadString`, and version.dll's file version information, over `resources`.
//!
//! A module's resources are read where it's loaded, as Windows reads them: an `HRSRC` is the address of its
//! `IMAGE_RESOURCE_DATA_ENTRY`, and a loaded resource is its bytes in the module. Version information is read from a
//! file's bytes, loaded or not; atlas's own DLLs have theirs too.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::files::Node;
use crate::images::pe::Pe;
use crate::process::Task;
use crate::resources::{self, Id, RT_STRING, RT_VERSION, Tree};

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOACCESS: u32 = 998;
const ERROR_RESOURCE_ENUM_USER_STOP: u32 = 15106;

impl Task {
    /// A module handle as its base: NULL is the program's own; a datafile handle's low bits go.
    fn module_base(&mut self, cx: &mut Cx<'_>, handle: u64) -> u64 {
        if handle != 0 {
            return handle & !3;
        }
        let peb = kernels_core::lock(&self.process.state).peb;
        let abi = self.abi();
        self.mem(cx, false, |t| user::read_ptr(&t.space(), peb + abi.peb.image_base, abi.width)).unwrap_or(0)
    }

    fn read_module(&mut self, cx: &mut Cx<'_>, at: u64, len: usize) -> Option<Vec<u8>> {
        self.mem(cx, false, |t| user::read_vec(&t.space(), at, len)).ok()
    }

    /// Where a loaded module's resource directory is (an RVA), from its headers in memory.
    fn resource_root(&mut self, cx: &mut Cx<'_>, base: u64) -> Result<u32, u32> {
        let missing = resources::ERROR_RESOURCE_DATA_NOT_FOUND;
        let dos = self.read_module(cx, base, 64).filter(|d| d.starts_with(b"MZ")).ok_or(missing)?;
        let pe = base + u64::from(u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()));
        let head = self.read_module(cx, pe, 24 + 2).filter(|h| h.starts_with(b"PE\0\0")).ok_or(missing)?;
        // PE32+'s data directories start 112 bytes into the optional header, PE32's 96; the resources' is the third.
        let dirs = pe + 24 + if u16::from_le_bytes([head[24], head[25]]) == 0x20b { 112 } else { 96 };
        let dir = self.read_module(cx, dirs + 16, 8).ok_or(missing)?;
        let (rva, size) = (u32::from_le_bytes(dir[0..4].try_into().unwrap()), u32::from_le_bytes(dir[4..8].try_into().unwrap()));
        if rva == 0 || size == 0 { Err(missing) } else { Ok(rva) }
    }

    /// A type or name as a program passes it: a number below 0x10000, else a string (`#123` being a number).
    fn resource_id(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<Id> {
        if at >> 16 == 0 {
            return Some(Id::Num(at as u16));
        }
        let s = if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 1024)).ok()?
        } else {
            crate::ansi::text(&self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 1024)).ok()?)
        };
        Some(Id::named(&s))
    }

    /// Finds a resource in a loaded module: its `IMAGE_RESOURCE_DATA_ENTRY`'s address, its bytes' and their size.
    fn find_resource(&mut self, cx: &mut Cx<'_>, module: u64, kind: &Id, name: &Id, lang: Option<u16>) -> Result<(u64, u64, u32), u32> {
        let base = self.module_base(cx, module);
        let root = self.resource_root(cx, base)?;
        let mut tree = Tree::new(root, |at, len| self.mem(cx, false, |t| user::read_vec(&t.space(), base + u64::from(at), len)).ok());
        let found = tree.find(kind, name, lang)?;
        Ok((base + u64::from(found.entry), base + u64::from(found.data), found.size))
    }

    /// A file's bytes, whether it's on the disk or one of atlas's own DLLs.
    fn file_bytes(&mut self, name: &str) -> Result<Vec<u8>, u32> {
        let (file, _) = self.open_file(name, 0x8000_0000, crate::files::FILE_SHARE_ALL, 3, 0)?;
        match &file.node {
            Node::File(ino) => kernels_core::lock(&self.k.disk).contents(*ino).map(|c| c.to_vec()).map_err(|_| 5),
            Node::System(image) => Ok(image.to_vec()),
            _ => Err(5),
        }
    }

    /// A file's version information (`RT_VERSION` 1), found through its sections as they are in the file.
    fn version_block(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Result<Vec<u8>, u32> {
        let name = if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 32768))
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32768)).map(|b| crate::ansi::text(&b))
        }
        .map_err(|_| ERROR_NOACCESS)?;
        let bytes = self.file_bytes(&name)?;
        let pe = Pe::parse(&bytes).map_err(|_| 193u32)?; // ERROR_BAD_EXE_FORMAT
        let (root, _) = pe.directory(2).ok_or(resources::ERROR_RESOURCE_DATA_NOT_FOUND)?;
        let offset = |rva: u32| -> Option<usize> {
            if rva < pe.size_of_headers {
                return Some(rva as usize);
            }
            let s = pe.sections.iter().find(|s| (s.rva..s.rva + s.virtual_size.max(s.raw_size)).contains(&rva))?;
            Some((s.raw_offset + (rva - s.rva)) as usize)
        };
        let read = |at: u32, len: usize| offset(at).and_then(|o| bytes.get(o..o + len)).map(<[u8]>::to_vec);
        let found = Tree::new(root, read).find(&Id::Num(RT_VERSION), &Id::Num(1), None).map_err(|_| resources::ERROR_RESOURCE_TYPE_NOT_FOUND)?;
        offset(found.data).and_then(|o| bytes.get(o..o + found.size as usize)).map(<[u8]>::to_vec).ok_or(resources::ERROR_RESOURCE_TYPE_NOT_FOUND)
    }
}

// --- kernel32 ---

fn find(task: &mut Task, cx: &mut Cx<'_>, module: u64, kind: u64, name: u64, lang: Option<u16>, wide: bool) -> Reply {
    let (Some(kind), Some(name)) = (task.resource_id(cx, kind, wide), task.resource_id(cx, name, wide)) else {
        task.set_last_error(cx, ERROR_NOACCESS);
        return Reply::Value(0);
    };
    match task.find_resource(cx, module, &kind, &name, lang) {
        Ok((entry, _, _)) => Reply::Value(entry),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0)
        }
    }
}

/// `FindResourceW(module, name, type)` and `A`: the name before the type.
pub fn find_resource_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find(task, cx, a[0], a[2], a[1], None, true)
}

pub fn find_resource_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find(task, cx, a[0], a[2], a[1], None, false)
}

/// `FindResourceExW(module, type, name, language)` and `A`: the type first, then a language.
pub fn find_resource_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find(task, cx, a[0], a[1], a[2], Some(a[3] as u16), true)
}

pub fn find_resource_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find(task, cx, a[0], a[1], a[2], Some(a[3] as u16), false)
}

/// `LoadResource(module, resource)`: where its bytes are, in the module.
pub fn load_resource(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let base = task.module_base(cx, a[0]);
    match task.read_module(cx, a[1], 4).filter(|_| a[1] != 0) {
        Some(b) => Reply::Value(base + u64::from(u32::from_le_bytes(b[..4].try_into().unwrap()))),
        None => {
            task.set_last_error(cx, ERROR_INVALID_HANDLE);
            Reply::Value(0)
        }
    }
}

/// `LockResource(loaded)`: the same address; `FreeResource`: nothing to free.
pub fn lock_resource(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(a[0])
}

pub fn free_resource(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(FALSE)
}

/// `SizeofResource(module, resource)`.
pub fn sizeof_resource(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.read_module(cx, a[1] + 4, 4).filter(|_| a[1] != 0) {
        Some(b) => Reply::Value(u64::from(u32::from_le_bytes(b[..4].try_into().unwrap()))),
        None => {
            task.set_last_error(cx, ERROR_INVALID_HANDLE);
            Reply::Value(0)
        }
    }
}

/// Calls an enumeration's callback with `before`, then an ID (a number, or a string on the stack), then `after`:
/// Ok(false) if it said to stop.
fn call_with_id(task: &mut Task, cx: &mut Cx<'_>, callback: u64, before: &[u64], id: &Id, after: &[u64], wide: bool) -> Result<bool, Reply> {
    let (data, number) = match id {
        Id::Num(n) => (Vec::new(), Some(u64::from(*n))),
        Id::Name(s) if wide => (s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect(), None),
        Id::Name(s) => (crate::ansi::bytes(s).into_iter().chain([0]).collect(), None),
    };
    let args = |at: u64| before.iter().copied().chain([number.unwrap_or(at)]).chain(after.iter().copied()).collect();
    task.upcall_data(cx, callback, &data, args).map(|r| r as u32 != 0)
}

/// Runs an enumeration over `ids`: TRUE when it's done, FALSE with ERROR_RESOURCE_ENUM_USER_STOP if the callback
/// stopped it.
fn enumerate(task: &mut Task, cx: &mut Cx<'_>, ids: Result<Vec<Id>, u32>, mut each: impl FnMut(&mut Task, &mut Cx<'_>, &Id) -> Result<bool, Reply>) -> Reply {
    let ids = match ids {
        Ok(ids) => ids,
        Err(e) => {
            task.set_last_error(cx, e);
            return Reply::Value(FALSE);
        }
    };
    for id in &ids {
        match each(task, cx, id) {
            Ok(true) => {}
            Ok(false) => {
                task.set_last_error(cx, ERROR_RESOURCE_ENUM_USER_STOP);
                return Reply::Value(FALSE);
            }
            Err(ended) => return ended,
        }
    }
    Reply::Value(TRUE)
}

/// A loaded module's types, a type's names, or a name's languages.
fn listing(task: &mut Task, cx: &mut Cx<'_>, module: u64, kind: Option<&Id>, name: Option<&Id>) -> Result<Vec<Id>, u32> {
    let base = task.module_base(cx, module);
    let root = task.resource_root(cx, base)?;
    let mut tree = Tree::new(root, |at, len| task.mem(cx, false, |t| user::read_vec(&t.space(), base + u64::from(at), len)).ok());
    let entries = match (kind, name) {
        (None, _) => tree.types().ok_or(resources::ERROR_RESOURCE_DATA_NOT_FOUND)?,
        (Some(k), None) => tree.names(k).ok_or(resources::ERROR_RESOURCE_TYPE_NOT_FOUND)?,
        (Some(k), Some(n)) => tree.languages(k, n).ok_or(resources::ERROR_RESOURCE_NAME_NOT_FOUND)?,
    };
    Ok(entries.into_iter().map(|e| e.id).collect())
}

/// `EnumResourceTypesW(module, callback(module, type, param), param)`.
pub fn enum_resource_types_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_types(task, cx, a, true)
}

pub fn enum_resource_types_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_types(task, cx, a, false)
}

fn enum_resource_types(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let ids = listing(task, cx, a[0], None, None);
    let (module, callback, param) = (a[0], a[1], a[2]);
    enumerate(task, cx, ids, |task, cx, id| call_with_id(task, cx, callback, &[module], id, &[param], wide))
}

/// `EnumResourceNamesW(module, type, callback(module, type, name, param), param)`: the type as the program gave it.
pub fn enum_resource_names_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_names(task, cx, a, true)
}

pub fn enum_resource_names_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_names(task, cx, a, false)
}

fn enum_resource_names(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(kind) = task.resource_id(cx, a[1], wide) else { return Reply::Value(FALSE) };
    let ids = listing(task, cx, a[0], Some(&kind), None);
    let (module, kind_arg, callback, param) = (a[0], a[1], a[2], a[3]);
    enumerate(task, cx, ids, |task, cx, id| call_with_id(task, cx, callback, &[module, kind_arg], id, &[param], wide))
}

/// `EnumResourceLanguagesW(module, type, name, callback(module, type, name, language, param), param)`.
pub fn enum_resource_languages_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_languages(task, cx, a, true)
}

pub fn enum_resource_languages_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    enum_resource_languages(task, cx, a, false)
}

fn enum_resource_languages(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (Some(kind), Some(name)) = (task.resource_id(cx, a[1], wide), task.resource_id(cx, a[2], wide)) else { return Reply::Value(FALSE) };
    let ids = listing(task, cx, a[0], Some(&kind), Some(&name));
    let (module, kind_arg, name_arg, callback, param) = (a[0], a[1], a[2], a[3], a[4]);
    enumerate(task, cx, ids, |task, cx, id| {
        let Id::Num(lang) = id else { return Ok(true) };
        task.upcall(cx, callback, &[module, kind_arg, name_arg, u64::from(*lang), param]).map(|r| r as u32 != 0)
    })
}

// --- user32 ---

/// String `id` of a module's string tables: where its UTF-16 is in memory, and its units.
fn table_string(task: &mut Task, cx: &mut Cx<'_>, module: u64, id: u32) -> Result<(u64, Vec<u16>), u32> {
    let (_, data, size) = task.find_resource(cx, module, &Id::Num(RT_STRING), &Id::Num((id >> 4) as u16 + 1), None)?;
    let block = task.read_module(cx, data, size as usize).ok_or(ERROR_NOACCESS)?;
    let (start, units) = resources::table_string(&block, (id & 15) as usize).ok_or(resources::ERROR_RESOURCE_NAME_NOT_FOUND)?;
    Ok((data + start as u64, units))
}

/// `LoadStringW(module, id, buffer, size)`: how many characters it copied, with a NUL; with a size of 0, the string's
/// address (read-only, not NUL-terminated) in `buffer`, and its length.
pub fn load_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (buffer, room) = (a[2], a[3] as u32 as usize);
    let (at, units) = match table_string(task, cx, a[0], a[1] as u32) {
        Ok(found) => found,
        Err(e) => {
            if room > 0 && buffer != 0 {
                let _ = task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &[0, 0]));
            }
            task.set_last_error(cx, e);
            return Reply::Value(0);
        }
    };
    if room == 0 {
        let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), buffer, if units.is_empty() { 0 } else { at }));
        return Reply::Value(units.len() as u64);
    }
    let n = units.len().min(room - 1);
    let bytes: Vec<u8> = units[..n].iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &bytes));
    Reply::Value(n as u64)
}

/// `LoadStringA(module, id, buffer, size)`: the same, as ANSI.
pub fn load_string_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (buffer, room) = (a[2], a[3] as u32 as usize);
    if room == 0 || buffer == 0 {
        return Reply::Value(0);
    }
    let text = match table_string(task, cx, a[0], a[1] as u32) {
        Ok((_, units)) => String::from_utf16_lossy(&units),
        Err(e) => {
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &[0]));
            task.set_last_error(cx, e);
            return Reply::Value(0);
        }
    };
    let ansi = crate::ansi::bytes(&text);
    let n = ansi.len().min(room - 1);
    let mut bytes = ansi[..n].to_vec();
    bytes.push(0);
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), buffer, &bytes));
    Reply::Value(n as u64)
}

// --- version.dll ---

/// `GetFileVersionInfoSizeW(file, &handle)`: room for its version information, with space to spare as Windows gives.
pub fn get_file_version_info_size_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info_size(task, cx, a, true)
}

pub fn get_file_version_info_size_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info_size(task, cx, a, false)
}

pub fn get_file_version_info_size_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info_size(task, cx, &a[1..], false)
}

/// The room asked for is twice the block's and more: the half after it is where `VerQueryValueA` puts strings it
/// converts.
fn version_info_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    if a[1] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], 0));
    }
    match task.version_block(cx, a[0], wide) {
        Ok(block) => Reply::Value(block.len() as u64 * 2 + 4),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0)
        }
    }
}

/// `GetFileVersionInfoSizeExW(flags, file, &handle)`.
pub fn get_file_version_info_size_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_version_info_size_w(task, cx, &a[1..])
}

/// `GetFileVersionInfoW(file, handle, size, buffer)`: its version information, as much as fits.
pub fn get_file_version_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info(task, cx, a, true)
}

pub fn get_file_version_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info(task, cx, a, false)
}

pub fn get_file_version_info_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    version_info(task, cx, &a[1..], false)
}

fn version_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let block = match task.version_block(cx, a[0], wide) {
        Ok(block) => block,
        Err(e) => {
            task.set_last_error(cx, e);
            return Reply::Value(FALSE);
        }
    };
    let n = block.len().min(a[2] as u32 as usize);
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &block[..n])).is_err() {
        task.set_last_error(cx, ERROR_NOACCESS);
        return Reply::Value(FALSE);
    }
    Reply::Value(TRUE)
}

/// `GetFileVersionInfoExW(flags, file, handle, size, buffer)`.
pub fn get_file_version_info_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_version_info_w(task, cx, &a[1..])
}

/// `VerQueryValueW(block, path, &value, &length)`: where a value is in the block, and its length (bytes for binary
/// values, characters for strings).
pub fn ver_query_value_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    ver_query_value(task, cx, a, true)
}

pub fn ver_query_value_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    ver_query_value(task, cx, a, false)
}

/// For `VerQueryValueA`, a string value (under `\StringFileInfo`) is converted to ANSI, in the room after the block,
/// at half its own offset, so each has a place of its own.
fn ver_query_value(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let found = (|| {
        let len = task.read_module(cx, a[0], 2)?;
        let block = task.read_module(cx, a[0], usize::from(u16::from_le_bytes([len[0], len[1]])))?;
        let path = if wide {
            task.mem(cx, false, |t| user::read_wstr(&t.space(), a[1], 1024)).ok()?
        } else {
            crate::ansi::text(&task.mem(cx, false, |t| user::read_cstr(&t.space(), a[1], 1024)).ok()?)
        };
        let (at, len) = resources::query(&block, &path)?;
        let string = path.trim_start_matches('\\').to_ascii_lowercase().starts_with("stringfileinfo\\");
        Some((at, len, block, string))
    })();
    let Some((at, len, block, string)) = found else {
        task.set_last_error(cx, resources::ERROR_RESOURCE_TYPE_NOT_FOUND);
        return Reply::Value(FALSE);
    };
    let mut value = a[0] + at as u64;
    if !wide && string {
        let units: Vec<u16> = block.get(at..at + usize::from(len) * 2).unwrap_or_default().as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let ansi = crate::ansi::encode(crate::ansi::ACP, &units, b'?', false).map(|(b, _)| b).unwrap_or_default();
        value = a[0] + block.len() as u64 + at as u64 / 2;
        if task.mem(cx, true, |t| user::write(&mut t.space(), value, &ansi)).is_err() {
            task.set_last_error(cx, ERROR_NOACCESS);
            return Reply::Value(FALSE);
        }
    }
    let ok = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[2], value)).is_ok()
        && task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], u32::from(len))).is_ok();
    Reply::Value(u64::from(ok))
}
