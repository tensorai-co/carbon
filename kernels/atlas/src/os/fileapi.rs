//! kernel32's files and folders, in host Rust over `files`: opening, reading, writing, seeking, attributes, folders,
//! moving and copying, finding, and the current directory. `ReadFile`, `WriteFile` and `CloseHandle` are here too, for
//! files and the console alike.

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::files::{self, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, File, Found, Info, Node, filetime};
use crate::handles::Handle;
use crate::process::Task;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;
const INVALID_FILE_ATTRIBUTES: u64 = 0xffff_ffff;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_NEGATIVE_SEEK: u32 = 131;

impl Task {
    /// A path the program passed, as UTF-16 or ANSI.
    fn path_arg(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at == 0 {
            return None;
        }
        if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 32767)).ok()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
        }
    }

    fn file_handle(&self, handle: u64) -> Option<Arc<File>> {
        match lock(&self.process.state).handles.get(handle)? {
            Handle::File(file) => Some(file),
            Handle::Console(stream) => Some(Arc::new(File { node: Node::Console(stream), read: false, write: true, append: false, position: Default::default(), path: String::new(), disk: String::new(), delete_on_close: false })),
            Handle::Input => Some(Arc::new(File { node: Node::Input, read: true, write: false, append: false, position: Default::default(), path: String::new(), disk: String::new(), delete_on_close: false })),
            _ => None,
        }
    }

    /// FALSE with the last error set.
    fn fail(&mut self, cx: &mut Cx<'_>, error: u32) -> Reply {
        self.set_last_error(cx, error);
        Reply::Value(FALSE)
    }

    /// Writes a string as UTF-16 into a buffer of `size` units, as the `Get*PathW` functions do: its length without
    /// the NUL if it fits, else the size it needs with it.
    fn put_wide(&mut self, cx: &mut Cx<'_>, buffer: u64, size: u64, s: &str) -> Reply {
        let units: Vec<u16> = s.encode_utf16().collect();
        if (units.len() as u64) < size && buffer != 0 {
            let bytes: Vec<u8> = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
            if self.mem(cx, true, |t| user::write(&mut t.space(), buffer, &bytes)).is_err() {
                return self.fail(cx, 998); // ERROR_NOACCESS
            }
            Reply::Value(units.len() as u64)
        } else {
            Reply::Value(units.len() as u64 + 1)
        }
    }

    fn put_ansi(&mut self, cx: &mut Cx<'_>, buffer: u64, size: u64, s: &str) -> Reply {
        let bytes = s.as_bytes();
        if (bytes.len() as u64) < size && buffer != 0 {
            let mut z = bytes.to_vec();
            z.push(0);
            if self.mem(cx, true, |t| user::write(&mut t.space(), buffer, &z)).is_err() {
                return self.fail(cx, 998);
            }
            Reply::Value(bytes.len() as u64)
        } else {
            Reply::Value(bytes.len() as u64 + 1)
        }
    }
}

fn times(b: &mut [u8], at: usize, modified: u64) {
    let t = filetime(modified);
    for o in [at, at + 8, at + 16] {
        b[o..o + 8].copy_from_slice(&t.to_le_bytes());
    }
}

fn create_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(name) = task.path_arg(cx, a[0], wide) else {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(INVALID_HANDLE_VALUE);
    };
    match task.open_file(&name, a[1] as u32, a[4] as u32, a[5] as u32) {
        Ok((file, existed)) => {
            // CREATE_ALWAYS and OPEN_ALWAYS say whether it was already there.
            let said = if existed && matches!(a[4], 2 | 4) { ERROR_ALREADY_EXISTS } else { 0 };
            task.set_last_error(cx, said);
            Reply::Value(lock(&task.process.state).handles.add(Handle::File(file)))
        }
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(INVALID_HANDLE_VALUE)
        }
    }
}

pub fn create_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_file(task, cx, a, true)
}

pub fn create_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_file(task, cx, a, false)
}

/// An OVERLAPPED's offset, for a read or write at a place of its own.
fn overlapped(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<u64> {
    if at == 0 {
        return None;
    }
    let (lo, hi) = task.mem(cx, false, |t| Ok((user::read_u32(&t.space(), at + 16)?, user::read_u32(&t.space(), at + 20)?))).ok()?;
    Some(u64::from(hi) << 32 | u64::from(lo))
}

pub fn read_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let at = overlapped(task, cx, a[4]);
    match task.read_from(&file, (a[2] as u32 as usize).min(1 << 24), at) {
        Ok(data) => {
            let n = data.len() as u32;
            let wrote = task.mem(cx, true, |t| {
                user::write(&mut t.space(), a[1], &data)?;
                if a[3] != 0 { user::write_u32(&mut t.space(), a[3], n) } else { Ok(()) }
            });
            if wrote.is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
        }
        Err(e) => task.fail(cx, e),
    }
}

pub fn write_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Ok(data) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], (a[2] as u32 as usize).min(1 << 24))) else { return task.fail(cx, 998) };
    let at = overlapped(task, cx, a[4]);
    match task.write_to_file(&file, &data, at) {
        Ok(n) => {
            if a[3] != 0 && task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[3], n as u32)).is_err() {
                return task.fail(cx, 998);
            }
            Reply::Value(TRUE)
        }
        Err(e) => task.fail(cx, e),
    }
}

pub fn close_handle(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let handle = {
        let mut state = lock(&task.process.state);
        let handle = state.handles.get(a[0]);
        state.handles.close(a[0]);
        handle
    };
    match handle {
        Some(Handle::File(file)) => {
            task.closed_file(file);
            Reply::Value(TRUE)
        }
        Some(_) => Reply::Value(TRUE),
        None => task.fail(cx, ERROR_INVALID_HANDLE),
    }
}

/// Moves a file's position: from its start, where it is, or its end.
fn seek(task: &Task, file: &File, distance: i64, method: u32) -> Result<u64, u32> {
    let base = match method {
        0 => 0,
        1 => *lock(&file.position) as i64,
        2 => task.file_info(file)?.size as i64,
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let to = base.checked_add(distance).filter(|n| *n >= 0).ok_or(ERROR_NEGATIVE_SEEK)?;
    *lock(&file.position) = to as u64;
    Ok(to as u64)
}

pub fn set_file_pointer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(0xffff_ffff);
    };
    let distance = if a[2] != 0 {
        let Ok(high) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[2])) else { return Reply::Value(0xffff_ffff) };
        (u64::from(high) << 32 | (a[1] & 0xffff_ffff)) as i64
    } else {
        a[1] as i32 as i64
    };
    match seek(task, &file, distance, a[3] as u32) {
        Ok(to) => {
            if a[2] != 0 {
                let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[2], (to >> 32) as u32));
            }
            task.set_last_error(cx, 0);
            Reply::Value(to & 0xffff_ffff)
        }
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0xffff_ffff)
        }
    }
}

pub fn set_file_pointer_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    match seek(task, &file, a[1] as i64, a[3] as u32) {
        Ok(to) => {
            if a[2] != 0 {
                let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[2], to));
            }
            Reply::Value(TRUE)
        }
        Err(e) => task.fail(cx, e),
    }
}

pub fn get_file_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let info = task.file_handle(a[0]).ok_or(ERROR_INVALID_HANDLE).and_then(|f| task.file_info(&f));
    match info {
        Ok(info) => {
            if a[1] != 0 {
                let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], (info.size >> 32) as u32));
            }
            Reply::Value(info.size & 0xffff_ffff)
        }
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(0xffff_ffff)
        }
    }
}

pub fn get_file_size_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let info = task.file_handle(a[0]).ok_or(ERROR_INVALID_HANDLE).and_then(|f| task.file_info(&f));
    match info {
        Ok(info) => {
            let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[1], info.size));
            Reply::Value(TRUE)
        }
        Err(e) => task.fail(cx, e),
    }
}

/// FILE_TYPE_DISK, CHAR, or UNKNOWN.
pub fn get_file_type(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let handle = lock(&task.process.state).handles.get(a[0]);
    match handle {
        Some(Handle::File(file)) => Reply::Value(if matches!(file.node, Node::Null | Node::Console(_) | Node::Input) { 2 } else { 1 }),
        Some(Handle::Console(_) | Handle::Input) => Reply::Value(2),
        Some(_) => Reply::Value(0),
        None => {
            task.set_last_error(cx, ERROR_INVALID_HANDLE);
            Reply::Value(0)
        }
    }
}

pub fn set_end_of_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::File(ino) = file.node else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if !file.write {
        return task.fail(cx, files::ERROR_ACCESS_DENIED);
    }
    let at = *lock(&file.position);
    let _ = lock(&task.k.disk).truncate(ino, at, task.k.host.wall());
    Reply::Value(TRUE)
}

/// The disk is one, so flushing a file saves everything written so far.
pub fn flush_file_buffers(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    let synced = lock(&task.k.disk).sync();
    match synced {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, files::layers_error(e)),
    }
}

fn get_file_attributes(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], wide) else { return Reply::Value(INVALID_FILE_ATTRIBUTES) };
    match task.path_info(&path) {
        Ok(info) => Reply::Value(u64::from(info.attributes)),
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(INVALID_FILE_ATTRIBUTES)
        }
    }
}

pub fn get_file_attributes_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_attributes(task, cx, a, true)
}

pub fn get_file_attributes_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_attributes(task, cx, a, false)
}

/// WIN32_FILE_ATTRIBUTE_DATA.
fn attribute_data(info: &Info) -> Vec<u8> {
    let mut b = vec![0u8; 36];
    b[0..4].copy_from_slice(&info.attributes.to_le_bytes());
    times(&mut b, 4, info.modified);
    b[28..32].copy_from_slice(&((info.size >> 32) as u32).to_le_bytes());
    b[32..36].copy_from_slice(&(info.size as u32).to_le_bytes());
    b
}

pub fn get_file_attributes_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.path_info(&path) {
        Ok(info) => {
            let b = attribute_data(&info);
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
        }
        Err(e) => task.fail(cx, e),
    }
}

pub fn set_file_attributes_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let found = task.resolve(&path).and_then(|(_, disk_path)| {
        let mut disk = lock(&task.k.disk);
        let ino = disk.resolve(layers::ROOT, &disk_path, true).map_err(|_| files::ERROR_FILE_NOT_FOUND)?;
        let directory = disk.meta(ino).map(|m| m.attributes & files::FILE_ATTRIBUTE_DIRECTORY).unwrap_or(0);
        let wanted = (a[1] as u32) & !files::FILE_ATTRIBUTE_DIRECTORY & !files::FILE_ATTRIBUTE_NORMAL;
        disk.set_meta(ino, crate::Meta { attributes: wanted | directory }).map_err(|_| files::ERROR_FILE_NOT_FOUND)
    });
    match found {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn get_file_information_by_handle(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let info = task.file_handle(a[0]).ok_or(ERROR_INVALID_HANDLE).and_then(|f| task.file_info(&f));
    match info {
        Ok(info) => {
            let mut b = vec![0u8; 52];
            b[0..4].copy_from_slice(&info.attributes.to_le_bytes());
            times(&mut b, 4, info.modified);
            b[28..32].copy_from_slice(&0xca4b_0c00u32.to_le_bytes()); // the volume's serial number
            b[32..36].copy_from_slice(&((info.size >> 32) as u32).to_le_bytes());
            b[36..40].copy_from_slice(&(info.size as u32).to_le_bytes());
            b[40..44].copy_from_slice(&info.links.to_le_bytes());
            b[44..48].copy_from_slice(&((info.index >> 32) as u32).to_le_bytes());
            b[48..52].copy_from_slice(&(info.index as u32).to_le_bytes());
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
        }
        Err(e) => task.fail(cx, e),
    }
}

/// Runs a path operation, giving TRUE, or FALSE with its error.
fn path_op(task: &mut Task, cx: &mut Cx<'_>, at: u64, wide: bool, op: impl FnOnce(&Task, &str) -> Result<(), u32>) -> Reply {
    let Some(path) = task.path_arg(cx, at, wide) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match op(task, &path) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn delete_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], true, Task::delete_file)
}

pub fn delete_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], false, Task::delete_file)
}

pub fn create_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], true, Task::create_directory)
}

pub fn create_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], false, Task::create_directory)
}

pub fn remove_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], true, Task::remove_directory)
}

pub fn move_file_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(from), Some(to)) = (task.path_arg(cx, a[0], true), task.path_arg(cx, a[1], true)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.move_file(&from, &to, a[2] & 1 != 0) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn move_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    move_file_ex_w(task, cx, &[a[0], a[1], 0])
}

pub fn copy_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(from), Some(to)) = (task.path_arg(cx, a[0], true), task.path_arg(cx, a[1], true)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.copy_file(&from, &to, a[2] as u32 != 0) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

/// WIN32_FIND_DATAW (592 bytes), or WIN32_FIND_DATAA (320).
fn find_data(found: &Found, wide: bool) -> Vec<u8> {
    let mut b = vec![0u8; if wide { 592 } else { 320 }];
    b[0..4].copy_from_slice(&found.attributes.to_le_bytes());
    times(&mut b, 4, found.modified);
    b[28..32].copy_from_slice(&((found.size >> 32) as u32).to_le_bytes());
    b[32..36].copy_from_slice(&(found.size as u32).to_le_bytes());
    if wide {
        for (i, unit) in found.name.encode_utf16().take(259).enumerate() {
            b[44 + i * 2..46 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
    } else {
        for (i, byte) in found.name.bytes().take(259).enumerate() {
            b[44 + i] = byte;
        }
    }
    b
}

fn find_first(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(pattern) = task.path_arg(cx, a[0], wide) else {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(INVALID_HANDLE_VALUE);
    };
    match task.find(&pattern) {
        Ok(mut found) => {
            let first = found.pop_front().map(|f| find_data(&f, wide)).unwrap_or_default();
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &first)).is_err() {
                task.set_last_error(cx, 998);
                return Reply::Value(INVALID_HANDLE_VALUE);
            }
            Reply::Value(lock(&task.process.state).handles.add(Handle::Find(Arc::new(std::sync::Mutex::new(found)))))
        }
        Err(e) => {
            task.set_last_error(cx, e);
            Reply::Value(INVALID_HANDLE_VALUE)
        }
    }
}

pub fn find_first_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_first(task, cx, a, true)
}

pub fn find_first_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_first(task, cx, a, false)
}

fn find_next(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let handle = lock(&task.process.state).handles.get(a[0]);
    let Some(Handle::Find(find)) = handle else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let next = lock(&find).pop_front();
    match next {
        Some(found) => {
            let b = find_data(&found, wide);
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
        }
        None => task.fail(cx, ERROR_NO_MORE_FILES),
    }
}

pub fn find_next_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_next(task, cx, a, true)
}

pub fn find_next_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    find_next(task, cx, a, false)
}

pub fn find_close(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let closed = {
        let mut state = lock(&task.process.state);
        matches!(state.handles.get(a[0]), Some(Handle::Find(_))) && state.handles.close(a[0])
    };
    if closed { Reply::Value(TRUE) } else { task.fail(cx, ERROR_INVALID_HANDLE) }
}

pub fn get_current_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cwd = task.cwd();
    task.put_wide(cx, a[1], a[0] & 0xffff_ffff, &cwd)
}

pub fn get_current_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cwd = task.cwd();
    task.put_ansi(cx, a[1], a[0] & 0xffff_ffff, &cwd)
}

pub fn set_current_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], true, Task::set_current_directory)
}

/// `GetFullPathNameW`: the full path, and where its file name starts.
pub fn get_full_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(name) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let full = match task.resolve(&name) {
        Ok((full, _)) => full,
        Err(e) => return task.fail(cx, e),
    };
    // A trailing separator stays: "C:\dir\" names the folder.
    let full = if name.ends_with(['\\', '/']) && !full.ends_with('\\') { format!("{full}\\") } else { full };
    let reply = task.put_wide(cx, a[2], a[1] & 0xffff_ffff, &full);
    if let Reply::Value(n) = reply
        && n < a[1] & 0xffff_ffff
        && a[3] != 0
    {
        let file_part = full.rfind('\\').map(|i| i + 1).filter(|&i| i < full.len());
        let at = file_part.map_or(0, |i| a[2] + full[..i].encode_utf16().count() as u64 * 2);
        let _ = task.mem(cx, true, |t| user::write_u64(&mut t.space(), a[3], at));
    }
    reply
}

pub fn get_long_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(name) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if let Err(e) = task.path_info(&name) {
        task.set_last_error(cx, e);
        return Reply::Value(0);
    }
    task.put_wide(cx, a[1], a[2] & 0xffff_ffff, &name)
}

/// `GetFinalPathNameByHandleW`: the file's full path, as `\\?\C:\...`.
pub fn get_final_path_name_by_handle_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]).filter(|f| !f.path.is_empty()) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(0);
    };
    let path = format!("\\\\?\\{}", file.path);
    task.put_wide(cx, a[1], a[2] & 0xffff_ffff, &path)
}

pub fn get_temp_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[1], a[0] & 0xffff_ffff, "C:\\Windows\\Temp\\")
}

pub fn get_windows_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows")
}

pub fn get_system_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows\\System32")
}
