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
use crate::{pipes, ports};

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

    pub(crate) fn file_handle(&self, handle: u64) -> Option<Arc<File>> {
        match lock(&self.process.state).handles.get(handle)? {
            Handle::File(file) => Some(file),
            Handle::Console(stream) => Some(Arc::new(File::device(Node::Console(stream), false, true))),
            Handle::Input => Some(Arc::new(File::device(Node::Input, true, false))),
            _ => None,
        }
    }

    /// FALSE with the last error set.
    pub(crate) fn fail(&mut self, cx: &mut Cx<'_>, error: u32) -> Reply {
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
    // `\\.\pipe\NAME`: a client connecting to a named pipe.
    if let Some(key) = crate::named_pipes::key(&name) {
        return match super::pipeapi::open_client(task, cx, &key, &name, a[1] as u32, a[5] as u32) {
            Ok(file) => {
                let inheritable = task.inheritable_by(cx, a[3]);
                task.set_last_error(cx, 0);
                Reply::Value(lock(&task.process.state).handles.add_with(Handle::File(file), inheritable))
            }
            Err(e) => {
                task.set_last_error(cx, e);
                Reply::Value(INVALID_HANDLE_VALUE)
            }
        };
    }
    match task.open_file(&name, a[1] as u32, a[4] as u32, a[5] as u32) {
        Ok((file, existed)) => {
            // CREATE_ALWAYS and OPEN_ALWAYS say whether it was already there.
            let said = if existed && matches!(a[4], 2 | 4) { ERROR_ALREADY_EXISTS } else { 0 };
            task.set_last_error(cx, said);
            let inheritable = task.inheritable_by(cx, a[3]);
            Reply::Value(lock(&task.process.state).handles.add_with(Handle::File(file), inheritable))
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
/// An OVERLAPPED's offset and event (64-bit: `Internal`, `InternalHigh`, `Offset`, `OffsetHigh`, `hEvent`).
fn overlapped(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<(u64, u64)> {
    if at == 0 {
        return None;
    }
    task.mem(cx, false, |t| {
        let (lo, hi) = (user::read_u32(&t.space(), at + 16)?, user::read_u32(&t.space(), at + 20)?);
        Ok((u64::from(hi) << 32 | u64::from(lo), user::read_u64(&t.space(), at + 24)?))
    })
    .ok()
}

/// How a transfer says it's done, besides its return value.
#[derive(Clone, Copy)]
pub(crate) enum Notify {
    /// `ReadFile`/`WriteFile`: the OVERLAPPED's event, and the file's port.
    Plain,
    /// `ReadFileEx`/`WriteFileEx`: an APC to the thread, with the routine.
    Routine(u64),
}

/// What a transfer did.
enum Outcome {
    /// Done: what it read (empty for a write), how many bytes, and the error (ERROR_MORE_DATA comes with data).
    Done(Vec<u8>, usize, u32),
    /// An overlapped read that's waiting: another thread completes it.
    Pending,
}

impl Task {
    /// Where an I/O that's pending completes to.
    fn new_op(&self, file: &File, overlapped: u64, buffer: u64, event: Option<Arc<crate::objects::Object>>, notify: Notify) -> Arc<ports::Op> {
        let (port, routine) = match notify {
            Notify::Plain => (lock(&file.port).clone(), None),
            Notify::Routine(r) => (None, Some(r)),
        };
        Arc::new(ports::Op { process: Arc::downgrade(&self.process), thread: Arc::downgrade(&self.control), overlapped, buffer, event, port, routine })
    }
}

/// `ReadFile`/`WriteFile` and their `Ex` forms: the transfer, then its completion as the handle and the call ask.
#[allow(clippy::too_many_arguments)]
pub(crate) fn transfer(task: &mut Task, cx: &mut Cx<'_>, handle: u64, buffer: u64, len: u32, done_out: u64, ov: u64, write: bool, notify: Notify) -> Reply {
    let Some(file) = task.file_handle(handle) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let len = (len as usize).min(1 << 24);
    let (offset, event) = match overlapped(task, cx, ov) {
        Some((offset, event)) => (Some(offset), if matches!(notify, Notify::Plain) { event } else { 0 }),
        None => (None, 0),
    };
    // The event is unset while the I/O is under way.
    let event = (event != 0).then(|| task.object(event)).flatten();
    if let Some(event) = &event {
        task.k.objects.signal(event, |s| s.set = false);
    }
    // Overlapped I/O can wait as pending: only an overlapped handle's, with an OVERLAPPED.
    let asynchronous = file.overlapped && ov != 0;
    let on_disk = matches!(file.node, Node::File(_));
    let finished = |r: Result<usize, u32>| match r {
        Ok(n) => Outcome::Done(Vec::new(), n, 0),
        Err(e) => Outcome::Done(Vec::new(), 0, e),
    };
    let outcome = if write {
        let data = match task.mem(cx, false, |t| user::read_vec(&t.space(), buffer, len)) {
            Ok(data) => data,
            Err(_) => return task.fail(cx, 998),
        };
        let pipe = match &file.node {
            Node::Pipe(end) if file.write => Some(Ok(end.pipe.clone())),
            Node::NamedPipe(end) if file.write => Some(end.outgoing()),
            _ => None,
        };
        match pipe {
            Some(Err(e)) => finished(Err(e)),
            // An overlapped write always goes in at once: the pending reads it completes are completed here.
            Some(Ok(pipe)) if asynchronous => match pipe.write_all(&data) {
                Ok(done) => {
                    for c in done {
                        let k = task.k.clone();
                        k.complete(cx, c);
                    }
                    finished(Ok(data.len()))
                }
                Err(e) => finished(Err(e)),
            },
            Some(Ok(pipe)) => match task.write_pipe(cx, &pipe, &data) {
                Ok(n) => finished(Ok(n)),
                Err(pipes::Failed::Error(e)) => finished(Err(e)),
                Err(pipes::Failed::Ended(reply)) => return reply,
            },
            None => finished(task.write_to_file(&file, &data, offset)),
        }
    } else {
        let pipe = match &file.node {
            Node::Pipe(end) if file.read => Some(Ok((end.pipe.clone(), false, false))),
            Node::NamedPipe(end) if file.read => Some(end.incoming().map(|p| (p, end.message_mode(), end.nowait()))),
            _ => None,
        };
        let read = |data: Vec<u8>, more: bool| {
            let n = data.len();
            Outcome::Done(data, n, if more { ports::ERROR_MORE_DATA } else { 0 })
        };
        match pipe {
            Some(Err(e)) => finished(Err(e)),
            Some(Ok((pipe, message_mode, nowait))) if asynchronous || nowait => {
                let op = asynchronous.then(|| task.new_op(&file, ov, buffer, event.clone(), notify));
                match pipe.read(len, message_mode, op) {
                    pipes::Read::Data(data, more) => read(data, more),
                    pipes::Read::Ended(e) => finished(Err(e)),
                    pipes::Read::Empty if asynchronous => Outcome::Pending,
                    pipes::Read::Empty => finished(Err(pipes::ERROR_NO_DATA)),
                }
            }
            Some(Ok((pipe, message_mode, _))) => match task.read_pipe(&pipe, len, message_mode) {
                Ok((data, more)) => read(data, more),
                Err(pipes::Failed::Error(e)) => finished(Err(e)),
                Err(pipes::Failed::Ended(reply)) => return reply,
            },
            None => match task.read_from(&file, len, offset) {
                // Reading at or past the end, at an offset the OVERLAPPED gave, is ERROR_HANDLE_EOF.
                Ok(data) if data.is_empty() && len > 0 && offset.is_some() && on_disk => finished(Err(ports::ERROR_HANDLE_EOF)),
                Ok(data) => read(data, false),
                Err(e) => finished(Err(e)),
            },
        }
    };
    let (data, n, error) = match outcome {
        Outcome::Pending => {
            let pending = [u64::from(ports::STATUS_PENDING).to_le_bytes(), 0u64.to_le_bytes()].concat();
            let _ = task.mem(cx, true, |t| user::write(&mut t.space(), ov, &pending));
            // `ReadFileEx` says it's under way by succeeding; `ReadFile`, by ERROR_IO_PENDING.
            return match notify {
                Notify::Routine(_) => Reply::Value(TRUE),
                Notify::Plain => task.fail(cx, ports::ERROR_IO_PENDING),
            };
        }
        Outcome::Done(data, n, error) => (data, n, error),
    };
    // What was read, the count, and the OVERLAPPED's status and count, whatever happened.
    let wrote = task.mem(cx, true, |t| {
        if !data.is_empty() {
            user::write(&mut t.space(), buffer, &data)?;
        }
        if done_out != 0 {
            user::write_u32(&mut t.space(), done_out, n as u32)?;
        }
        if ov != 0 {
            user::write_u64(&mut t.space(), ov, u64::from(ports::status(error)))?;
            user::write_u64(&mut t.space(), ov + 8, n as u64)?;
        }
        Ok(())
    });
    if wrote.is_err() {
        return task.fail(cx, 998);
    }
    // A failure (but a message that didn't all fit) says so at once, with nothing else told.
    if error != 0 && error != ports::ERROR_MORE_DATA {
        return task.fail(cx, error);
    }
    match notify {
        Notify::Plain => {
            const SKIP_PORT_ON_SUCCESS: u32 = 1;
            if let Some(event) = &event {
                task.k.objects.signal(event, |s| s.set = true);
            }
            let port = lock(&file.port).clone();
            if let Some((port, key)) = port
                && asynchronous
                && (error != 0 || file.modes.load(std::sync::atomic::Ordering::Relaxed) & SKIP_PORT_ON_SUCCESS == 0)
            {
                port.post(ports::Packet { bytes: n as u32, key, overlapped: ov, status: ports::status(error) });
            }
        }
        Notify::Routine(routine) => {
            // The routine runs in this thread's next alertable wait: (error, bytes, OVERLAPPED).
            lock(&task.control.apcs).push_back((routine, vec![u64::from(error), n as u64, ov]));
        }
    }
    if error != 0 { task.fail(cx, error) } else { Reply::Value(TRUE) }
}

pub fn read_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    transfer(task, cx, a[0], a[1], a[2] as u32, a[3], a[4], false, Notify::Plain)
}

pub fn write_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    transfer(task, cx, a[0], a[1], a[2] as u32, a[3], a[4], true, Notify::Plain)
}

/// `ReadFileEx(file, buffer, size, overlapped, routine)`.
pub fn read_file_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] == 0 {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    transfer(task, cx, a[0], a[1], a[2] as u32, 0, a[3], false, Notify::Routine(a[4]))
}

/// `WriteFileEx(file, buffer, size, overlapped, routine)`.
pub fn write_file_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] == 0 {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    transfer(task, cx, a[0], a[1], a[2] as u32, 0, a[3], true, Notify::Routine(a[4]))
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
        Some(Handle::File(file)) => Reply::Value(match file.node {
            Node::Null | Node::Console(_) | Node::Input => 2,
            Node::Pipe(_) | Node::NamedPipe(_) => 3,
            _ => 1,
        }),
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
