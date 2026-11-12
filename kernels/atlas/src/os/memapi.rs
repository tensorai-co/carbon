//! kernel32's sections and memory queries, in host Rust over `sections`: `CreateFileMapping` (of a file, or of the
//! page file), `OpenFileMapping`, `MapViewOfFile(Ex)` (shared, read-only or copy-on-write), `FlushViewOfFile`,
//! `UnmapViewOfFile`, and `VirtualQuery`, which says what any address is.

use std::sync::{Arc, Weak};

use exec::{PAGE, Prot};
use kernels_core::thread::Cx;
use kernels_core::{lock, user};

use super::Reply;
use crate::files::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, Node};
use crate::handles::Handle;
use crate::mapping::prot_to_page;
use crate::process::{ALLOC, Area, AreaKind, GRANULE, Task};
use crate::sections::{Section, View};

const TRUE: u64 = 1;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_INVALID_ADDRESS: u32 = 487;
const ERROR_FILE_INVALID: u32 = 1006;
const ERROR_MAPPED_ALIGNMENT: u32 = 1132;

const SEC_IMAGE: u32 = 0x0100_0000;
const FILE_MAP_COPY: u32 = 0x1;
const FILE_MAP_WRITE: u32 = 0x2;
const FILE_MAP_EXECUTE: u32 = 0x20;

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_FREE: u32 = 0x1_0000;
const MEM_PRIVATE: u32 = 0x2_0000;
const MEM_MAPPED: u32 = 0x4_0000;
const MEM_IMAGE: u32 = 0x100_0000;

impl Task {
    fn section_handle(&self, handle: u64) -> Option<Arc<Section>> {
        match lock(&self.process.state).handles.get(handle)? {
            Handle::Section(section) => Some(section),
            _ => None,
        }
    }

    /// NULL with the last error set.
    fn null(&mut self, cx: &mut Cx<'_>, error: u32) -> Reply {
        self.set_last_error(cx, error);
        Reply::Value(0)
    }

    fn section_name(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at == 0 {
            return None;
        }
        let name = if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 260)).ok()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 260)).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
        };
        // `Local\` is the session's, and atlas has one session.
        name.map(|n| n.strip_prefix("Local\\").or_else(|| n.strip_prefix("Global\\")).map(str::to_string).unwrap_or(n))
    }
}

/// `CreateFileMappingW/A(file, attributes, protect, size high, size low, name)`.
fn create_file_mapping(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (file, attributes, protect) = (a[0], a[1], a[2] as u32);
    let size = (a[3] << 32) | (a[4] & 0xffff_ffff);
    let name = task.section_name(cx, a[5], wide);
    if protect & SEC_IMAGE != 0 {
        return task.null(cx, ERROR_NOT_SUPPORTED);
    }
    let (writable, executable) = match protect & 0xff {
        0x02 | 0x08 => (false, false),
        0x04 => (true, false),
        0x20 | 0x80 => (false, true),
        0x40 => (true, true),
        _ => return task.null(cx, ERROR_INVALID_PARAMETER),
    };
    let inheritable = task.inheritable_by(cx, attributes);
    // A name that's taken gives the section that has it.
    let existing = name.as_ref().and_then(|name| lock(&task.k.sections).get(name).and_then(Weak::upgrade));
    if let Some(existing) = existing {
        task.set_last_error(cx, ERROR_ALREADY_EXISTS);
        return Reply::Value(lock(&task.process.state).handles.add_with(Handle::Section(existing), inheritable));
    }
    let section = if file == u64::MAX {
        if size == 0 {
            return task.null(cx, ERROR_INVALID_PARAMETER);
        }
        Section::new(task.k.exec.clone(), size, writable, executable, None)
    } else {
        let handle = lock(&task.process.state).handles.get(file);
        let open = match handle {
            Some(Handle::File(open)) => open,
            _ => return task.null(cx, ERROR_INVALID_HANDLE),
        };
        let Node::File(ino) = open.node else { return task.null(cx, ERROR_INVALID_HANDLE) };
        if !open.read || (writable && !open.write) {
            return task.null(cx, ERROR_ACCESS_DENIED);
        }
        let length = lock(&task.k.disk).stat(ino).map(|s| s.size).unwrap_or(0);
        let size = if size == 0 { length } else { size };
        if size == 0 {
            return task.null(cx, ERROR_FILE_INVALID);
        }
        // A writable mapping bigger than its file makes the file that big.
        if size > length {
            if !writable {
                return task.null(cx, ERROR_NOT_ENOUGH_MEMORY);
            }
            let now = task.k.host.wall();
            if lock(&task.k.disk).truncate(ino, size, now).is_err() {
                return task.null(cx, ERROR_NOT_ENOUGH_MEMORY);
            }
        }
        Section::new(task.k.exec.clone(), size, writable, executable, Some(ino))
    };
    if let Some(name) = name {
        let mut names = lock(&task.k.sections);
        names.retain(|_, w| w.strong_count() > 0);
        names.insert(name, Arc::downgrade(&section));
    }
    task.set_last_error(cx, 0);
    Reply::Value(lock(&task.process.state).handles.add_with(Handle::Section(section), inheritable))
}

pub fn create_file_mapping_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_file_mapping(task, cx, a, true)
}

pub fn create_file_mapping_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_file_mapping(task, cx, a, false)
}

/// `OpenFileMappingW/A(access, inherit, name)`.
fn open_file_mapping(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let found = task.section_name(cx, a[2], wide).and_then(|name| lock(&task.k.sections).get(&name).and_then(Weak::upgrade));
    match found {
        Some(section) => Reply::Value(lock(&task.process.state).handles.add_with(Handle::Section(section), a[1] as u32 != 0)),
        None => task.null(cx, ERROR_FILE_NOT_FOUND),
    }
}

pub fn open_file_mapping_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_file_mapping(task, cx, a, true)
}

pub fn open_file_mapping_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    open_file_mapping(task, cx, a, false)
}

/// `MapViewOfFileEx(section, access, offset high, offset low, bytes, base)`: a view at `base`, or wherever there's
/// room. Its pages come in as they're touched.
pub fn map_view_of_file_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(section) = task.section_handle(a[0]) else { return task.null(cx, ERROR_INVALID_HANDLE) };
    let access = a[1] as u32;
    let offset = (a[2] << 32) | (a[3] & 0xffff_ffff);
    if !offset.is_multiple_of(GRANULE) {
        return task.null(cx, ERROR_MAPPED_ALIGNMENT);
    }
    let bytes = if a[4] == 0 { section.size.saturating_sub(offset) } else { a[4] };
    if offset >= section.size || bytes == 0 || offset + bytes > section.size {
        return task.null(cx, ERROR_ACCESS_DENIED);
    }
    let copy = access & FILE_MAP_COPY != 0 && access & FILE_MAP_WRITE == 0;
    let write = access & FILE_MAP_WRITE != 0;
    let execute = access & FILE_MAP_EXECUTE != 0;
    if (write && !section.writable) || (execute && !section.executable) {
        return task.null(cx, ERROR_ACCESS_DENIED);
    }
    let prot = match (write || copy, execute) {
        (false, false) => Prot::R,
        (true, false) => Prot::RW,
        (false, true) => Prot::RX,
        (true, true) => Prot::RWX,
    };
    let size = bytes.next_multiple_of(PAGE);
    let mut state = lock(&task.process.state);
    let base = match a[5] {
        0 => state.regions.free_above(ALLOC, size.next_multiple_of(GRANULE), GRANULE),
        at if at.is_multiple_of(GRANULE) && state.regions.is_free(at..at + size) => Some(at),
        _ => None,
    };
    let Some(base) = base else {
        drop(state);
        return task.null(cx, ERROR_INVALID_ADDRESS);
    };
    state.regions.insert(base..base + size, Area { prot, kind: AreaKind::View, base, view: Some(View { section, offset, copy }) });
    drop(state);
    Reply::Value(base)
}

/// `MapViewOfFile(section, access, offset high, offset low, bytes)`.
pub fn map_view_of_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    map_view_of_file_ex(task, cx, &[a[0], a[1], a[2], a[3], a[4], 0])
}

/// `UnmapViewOfFile(base)`: its pages go back to the file, and it's gone.
pub fn unmap_view_of_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((range, section, copy)) = task.view_at(a[0]) else { return task.fail(cx, ERROR_INVALID_ADDRESS) };
    if !copy {
        let offset = lock(&task.process.state).regions.find(range.start).and_then(|(_, a)| a.view.as_ref().map(|v| v.offset)).unwrap_or(0);
        task.write_back(cx, &section, offset..offset + (range.end - range.start));
    }
    drop(section);
    let removed = lock(&task.process.state).regions.remove(range.clone());
    cx.turn(|t| t.space().unmap(range));
    drop(removed);
    Reply::Value(TRUE)
}

/// `FlushViewOfFile(address, bytes)`: the view's pages from there go back to its file.
pub fn flush_view_of_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let found = {
        let state = lock(&task.process.state);
        state.regions.find(a[0]).and_then(|(range, area)| area.view.clone().map(|v| (range, area.base, v)))
    };
    let Some((range, base, view)) = found else { return task.fail(cx, ERROR_INVALID_ADDRESS) };
    let Some((whole, _, _)) = task.view_at(base) else { return task.fail(cx, ERROR_INVALID_ADDRESS) };
    let start = a[0] & !(PAGE - 1);
    let end = if a[1] == 0 { whole.end } else { (a[0] + a[1]).next_multiple_of(PAGE).min(whole.end) };
    let offset = view.offset + (start - range.start);
    if !view.copy {
        task.write_back(cx, &view.section, offset..offset + (end - start));
    }
    Reply::Value(TRUE)
}

/// `VirtualQuery(address, &info, size)`: MEMORY_BASIC_INFORMATION for the run of pages from there that are alike.
pub fn virtual_query(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[2] < 48 {
        task.set_last_error(cx, 24); // ERROR_BAD_LENGTH
        return Reply::Value(0);
    }
    let page = a[0] & !(PAGE - 1);
    let info = {
        let state = lock(&task.process.state);
        match state.regions.find(page) {
            None => {
                // Free up to whatever comes next.
                let next = state.regions.iter().map(|(r, _)| r.start).filter(|&s| s > page).min().unwrap_or(crate::images::SYSTEM_BASE);
                (page, 0, 0, next.saturating_sub(page), MEM_FREE, 0, 0)
            }
            Some((range, area)) => {
                let alike = |a: &Area, b: &Area| a.kind == b.kind && a.prot == b.prot && a.base == b.base;
                let mut end = range.end;
                while let Some((next, b)) = state.regions.find(end) {
                    if !alike(area, b) {
                        break;
                    }
                    end = next.end;
                }
                let base = if area.base != 0 { area.base } else { range.start };
                let (state_bits, protect) = match area.kind {
                    AreaKind::Reserved | AreaKind::Kept => (MEM_RESERVE, 0),
                    _ => (MEM_COMMIT, prot_to_page(area.prot)),
                };
                let kind = match area.kind {
                    AreaKind::Image => MEM_IMAGE,
                    AreaKind::View => MEM_MAPPED,
                    _ => MEM_PRIVATE,
                };
                let allocation_protect = if area.kind == AreaKind::Reserved { 0x01 } else { prot_to_page(area.prot) };
                (page, base, allocation_protect, end - page, state_bits, protect, kind)
            }
        }
    };
    let (at, base, allocation_protect, size, state_bits, protect, kind) = info;
    let mut b = [0u8; 48];
    b[0..8].copy_from_slice(&at.to_le_bytes());
    b[8..16].copy_from_slice(&base.to_le_bytes());
    b[16..20].copy_from_slice(&allocation_protect.to_le_bytes());
    b[24..32].copy_from_slice(&size.to_le_bytes());
    b[32..36].copy_from_slice(&state_bits.to_le_bytes());
    b[36..40].copy_from_slice(&protect.to_le_bytes());
    b[40..44].copy_from_slice(&kind.to_le_bytes());
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() {
        task.set_last_error(cx, 998);
        return Reply::Value(0);
    }
    Reply::Value(48)
}

/// `GetSystemTimeAsFileTime(&time)` and its precise form: now, in 100 ns units since 1601.
pub fn get_system_time_as_file_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let now = crate::files::filetime(task.k.host.wall());
    let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[0], now));
    Reply::Value(0)
}
