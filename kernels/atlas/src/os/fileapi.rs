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
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).ok().map(|b| crate::ansi::text(&b))
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

    /// A string written back as a `*W` (UTF-16) or `*A` (ANSI) function does.
    pub(crate) fn put_text(&mut self, cx: &mut Cx<'_>, buffer: u64, size: u64, s: &str, wide: bool) -> Reply {
        if wide { self.put_wide(cx, buffer, size, s) } else { self.put_ansi(cx, buffer, size, s) }
    }

    /// The same, in the ANSI code page.
    pub(crate) fn put_ansi(&mut self, cx: &mut Cx<'_>, buffer: u64, size: u64, s: &str) -> Reply {
        let bytes = &crate::ansi::bytes(s)[..];
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

/// Three FILETIMEs in a row, as Windows' structures have them: made, last read, last written.
fn times(b: &mut [u8], at: usize, created: u64, accessed: u64, modified: u64) {
    for (o, t) in [(at, created), (at + 8, accessed), (at + 16, modified)] {
        b[o..o + 8].copy_from_slice(&filetime(t).to_le_bytes());
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
    // A file given a descriptor of its own is opened only as it allows.
    let full = task.security_path(&name);
    let kept = full.as_ref().and_then(|p| lock(&task.k.file_security).get(p).cloned());
    if let Some(d) = kept
        && crate::security::access_check(&d, &lock(&task.effective_token().state), a[1] as u32, &crate::security::FILE).is_err()
    {
        task.set_last_error(cx, 5); // ERROR_ACCESS_DENIED
        return Reply::Value(INVALID_HANDLE_VALUE);
    }
    match task.open_file(&name, a[1] as u32, a[2] as u32, a[4] as u32, a[5] as u32) {
        Ok((file, existed)) => {
            if !existed && let Some(d) = task.attributes_descriptor(cx, a[3]) {
                let d = d.filled(&lock(&task.effective_token().state)).mapped(&crate::security::FILE);
                lock(&task.k.file_security).insert(file.path.to_ascii_lowercase(), d);
            }
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

/// An OVERLAPPED's offset and event, for a read or write at a place of its own (`Internal`, `InternalHigh`,
/// `Offset`, `OffsetHigh`, `hEvent`, the first two and the last pointer-sized).
fn overlapped(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<(u64, u64)> {
    if at == 0 {
        return None;
    }
    let abi = task.abi();
    task.mem(cx, false, |t| {
        let offset = at + abi.overlapped_offset();
        let (lo, hi) = (user::read_u32(&t.space(), offset)?, user::read_u32(&t.space(), offset + 4)?);
        Ok((u64::from(hi) << 32 | u64::from(lo), user::read_ptr(&t.space(), at + abi.overlapped_event(), abi.width)?))
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
            let pending = task.abi().overlapped_result(ports::STATUS_PENDING, 0);
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
    let result = task.abi().overlapped_result(ports::status(error), n as u64);
    let wrote = task.mem(cx, true, |t| {
        if !data.is_empty() {
            user::write(&mut t.space(), buffer, &data)?;
        }
        if done_out != 0 {
            user::write_u32(&mut t.space(), done_out, n as u32)?;
        }
        if ov != 0 {
            user::write(&mut t.space(), ov, &result)?;
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
    times(&mut b, 4, info.created, info.accessed, info.modified);
    b[28..32].copy_from_slice(&((info.size >> 32) as u32).to_le_bytes());
    b[32..36].copy_from_slice(&(info.size as u32).to_le_bytes());
    b
}

pub fn get_file_attributes_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_attributes_ex(task, cx, a, true)
}

pub fn get_file_attributes_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_file_attributes_ex(task, cx, a, false)
}

fn get_file_attributes_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], wide) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.path_info(&path) {
        Ok(info) => {
            let b = attribute_data(&info);
            if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &b)).is_err() { task.fail(cx, 998) } else { Reply::Value(TRUE) }
        }
        Err(e) => task.fail(cx, e),
    }
}

pub fn set_file_attributes_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_file_attributes(task, cx, a, true)
}

pub fn set_file_attributes_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_file_attributes(task, cx, a, false)
}

fn set_file_attributes(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], wide) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
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
            times(&mut b, 4, info.created, info.accessed, info.modified);
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
    let path = task.path_arg(cx, a[0], true);
    let done = path_op(task, cx, a[0], true, Task::delete_file);
    task.forget_security(path.as_deref(), done);
    done
}

pub fn delete_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], false, Task::delete_file)
}

pub fn create_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let done = path_op(task, cx, a[0], true, Task::create_directory);
    if done == Reply::Value(TRUE)
        && let Some(d) = task.attributes_descriptor(cx, a[1])
        && let Some(path) = task.path_arg(cx, a[0], true).and_then(|p| task.security_path(&p))
    {
        let d = d.filled(&lock(&task.effective_token().state)).mapped(&crate::security::FILE);
        lock(&task.k.file_security).insert(path, d);
    }
    done
}

pub fn create_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], false, Task::create_directory)
}

pub fn remove_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    remove_directory(task, cx, a, true)
}

pub fn remove_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    remove_directory(task, cx, a, false)
}

fn remove_directory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let path = task.path_arg(cx, a[0], wide);
    let done = path_op(task, cx, a[0], wide, Task::remove_directory);
    task.forget_security(path.as_deref(), done);
    done
}

pub fn move_file_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    move_file(task, cx, a, true)
}

pub fn move_file_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    move_file(task, cx, a, false)
}

pub fn move_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    move_file(task, cx, &[a[0], a[1], 0], false)
}

fn move_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (Some(from), Some(to)) = (task.path_arg(cx, a[0], wide), task.path_arg(cx, a[1], wide)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.move_file(&from, &to, a[2] & 1 != 0) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn move_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    move_file_ex_w(task, cx, &[a[0], a[1], 0])
}

/// `CreateHardLink(new, existing, attributes)`.
fn create_hard_link(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (Some(new), Some(existing)) = (task.path_arg(cx, a[0], wide), task.path_arg(cx, a[1], wide)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.create_hard_link(&new, &existing) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn create_hard_link_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_hard_link(task, cx, a, true)
}

pub fn create_hard_link_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_hard_link(task, cx, a, false)
}

/// `CreateSymbolicLink(link, target, flags)`: SYMBOLIC_LINK_FLAG_DIRECTORY (1) for a folder's.
fn create_symbolic_link(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (Some(link), Some(target)) = (task.path_arg(cx, a[0], wide), task.path_arg(cx, a[1], wide)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if a[2] as u32 & !3 != 0 {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    match task.create_symbolic_link(&link, &target, a[2] as u32 & 1 != 0) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn create_symbolic_link_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_symbolic_link(task, cx, a, true)
}

pub fn create_symbolic_link_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_symbolic_link(task, cx, a, false)
}

/// `GetFileTime(file, &created, &accessed, &written)`: any it's given somewhere to put.
pub fn get_file_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let info = task.file_handle(a[0]).ok_or(ERROR_INVALID_HANDLE).and_then(|f| task.file_info(&f));
    match info {
        Ok(info) => {
            for (at, t) in [(a[1], info.created), (a[2], info.accessed), (a[3], info.modified)] {
                if at != 0 && task.mem(cx, true, |t2| user::write_u64(&mut t2.space(), at, filetime(t))).is_err() {
                    return task.fail(cx, 998);
                }
            }
            Reply::Value(TRUE)
        }
        Err(e) => task.fail(cx, e),
    }
}

/// `SetFileTime(file, &created, &accessed, &written)`: each that isn't null, or 0, or -1 (which only stops Windows
/// changing it itself).
pub fn set_file_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let mut times = [None; 3];
    for (i, at) in a[1..4].iter().enumerate() {
        if *at != 0 {
            let Ok(ft) = task.mem(cx, false, |t| user::read_u64(&t.space(), *at)) else { return task.fail(cx, 998) };
            if ft != 0 && ft != u64::MAX {
                times[i] = Some(files::from_filetime(ft));
            }
        }
    }
    match task.set_file_times(&file, times[0], times[1], times[2]) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn copy_file_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    copy_file(task, cx, a, true)
}

pub fn copy_file_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    copy_file(task, cx, a, false)
}

fn copy_file(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (Some(from), Some(to)) = (task.path_arg(cx, a[0], wide), task.path_arg(cx, a[1], wide)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    match task.copy_file(&from, &to, a[2] as u32 != 0) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

/// WIN32_FIND_DATAW (592 bytes), or WIN32_FIND_DATAA (320).
fn find_data(found: &Found, wide: bool) -> Vec<u8> {
    let mut b = vec![0u8; if wide { 592 } else { 320 }];
    b[0..4].copy_from_slice(&found.attributes.to_le_bytes());
    times(&mut b, 4, found.created, found.accessed, found.modified);
    b[28..32].copy_from_slice(&((found.size >> 32) as u32).to_le_bytes());
    b[32..36].copy_from_slice(&(found.size as u32).to_le_bytes());
    // dwReserved0: a reparse point's tag.
    if found.attributes & files::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        b[36..40].copy_from_slice(&files::IO_REPARSE_TAG_SYMLINK.to_le_bytes());
    }
    if wide {
        for (i, unit) in found.name.encode_utf16().take(259).enumerate() {
            b[44 + i * 2..46 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
    } else {
        for (i, byte) in crate::ansi::bytes(&found.name).into_iter().take(259).enumerate() {
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

pub fn set_current_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    path_op(task, cx, a[0], false, Task::set_current_directory)
}

/// `GetFullPathNameW`: the full path, and where its file name starts.
pub fn get_full_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_full_path_name(task, cx, a, true)
}

pub fn get_full_path_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_full_path_name(task, cx, a, false)
}

fn get_full_path_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(name) = task.path_arg(cx, a[0], wide) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let full = match task.resolve(&name) {
        Ok((full, _)) => full,
        Err(e) => return task.fail(cx, e),
    };
    // A trailing separator stays: "C:\dir\" names the folder.
    let full = if name.ends_with(['\\', '/']) && !full.ends_with('\\') { format!("{full}\\") } else { full };
    let reply = task.put_text(cx, a[2], a[1] & 0xffff_ffff, &full, wide);
    if let Reply::Value(n) = reply
        && n < a[1] & 0xffff_ffff
        && a[3] != 0
    {
        let file_part = full.rfind('\\').map(|i| i + 1).filter(|&i| i < full.len());
        let units = |s: &str| if wide { s.encode_utf16().count() as u64 * 2 } else { crate::ansi::bytes(s).len() as u64 };
        let at = file_part.map_or(0, |i| a[2] + units(&full[..i]));
        // lpFilePart: a pointer.
        let _ = task.write_ptr(cx, a[3], at);
    }
    reply
}

pub fn get_long_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_long_path_name(task, cx, a, true)
}

pub fn get_long_path_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_long_path_name(task, cx, a, false)
}

fn get_long_path_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(name) = task.path_arg(cx, a[0], wide) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if let Err(e) = task.path_info(&name) {
        task.set_last_error(cx, e);
        return Reply::Value(0);
    }
    task.put_text(cx, a[1], a[2] & 0xffff_ffff, &name, wide)
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

pub fn get_final_path_name_by_handle_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]).filter(|f| !f.path.is_empty()) else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(0);
    };
    let path = format!("\\\\?\\{}", file.path);
    task.put_ansi(cx, a[1], a[2] & 0xffff_ffff, &path)
}

pub fn get_temp_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[1], a[0] & 0xffff_ffff, "C:\\Windows\\Temp\\")
}

pub fn get_temp_path_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_ansi(cx, a[1], a[0] & 0xffff_ffff, "C:\\Windows\\Temp\\")
}

pub fn get_windows_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows")
}

pub fn get_windows_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_ansi(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows")
}

pub fn get_system_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows\\System32")
}

pub fn get_system_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_ansi(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows\\System32")
}

/// `GetSystemWow64DirectoryW(buffer, size)`: SysWOW64, where 32-bit programs' system DLLs are, for programs of either
/// kind, as 64-bit Windows says.
pub fn get_system_wow64_directory_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_wide(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows\\SysWOW64")
}

pub fn get_system_wow64_directory_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.put_ansi(cx, a[0], a[1] & 0xffff_ffff, "C:\\Windows\\SysWOW64")
}

/// `Wow64DisableWow64FsRedirection(&old)`: this thread's System32 is System32 until it reverts, and `old` is what to
/// revert to. Only a 32-bit program has redirection to turn off; a 64-bit one is told ERROR_INVALID_FUNCTION.
pub fn wow64_disable_wow64_fs_redirection(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.isa != exec::Isa::X86_32 {
        return task.fail(cx, 1);
    }
    let was_off = task.control.fs_redirection_off.swap(true, std::sync::atomic::Ordering::Relaxed);
    let _ = task.write_ptr(cx, a[0], u64::from(was_off));
    Reply::Value(1)
}

/// `Wow64RevertWow64FsRedirection(old)`: back as it was when it was turned off.
pub fn wow64_revert_wow64_fs_redirection(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.isa != exec::Isa::X86_32 {
        return task.fail(cx, 1);
    }
    task.control.fs_redirection_off.store(a[0] & 1 != 0, std::sync::atomic::Ordering::Relaxed);
    Reply::Value(1)
}

/// `Wow64EnableWow64FsRedirection(enable)`: on or off, BOOLEAN.
pub fn wow64_enable_wow64_fs_redirection(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.isa != exec::Isa::X86_32 {
        return Reply::Value(0);
    }
    task.control.fs_redirection_off.store(a[0] as u8 == 0, std::sync::atomic::Ordering::Relaxed);
    Reply::Value(1)
}

// --- What installers ask for: temporary names, short names, disk space, searching, and .ini files ---

/// `GetTempFileNameW(folder, prefix, unique, &name)`: `folder\PRE<hex>.tmp`. With `unique` 0, a number no file has yet,
/// whose empty file is made then, as Windows does; otherwise that number, and nothing made. Its number.
pub fn get_temp_file_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const CREATE_NEW: u32 = 1;
    let Some(folder) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let prefix: String = task.path_arg(cx, a[1], true).unwrap_or_default().chars().take(3).collect();
    let folder = folder.trim_end_matches('\\').to_string();
    let name = |n: u32| format!("{folder}\\{prefix}{n:X}.tmp");
    let mut n = a[2] as u32 & 0xffff;
    if n == 0 {
        let start = (task.k.host.monotonic() as u32 & 0xffff).max(1);
        n = start;
        loop {
            if task.open_file(&name(n), GENERIC_WRITE, 0, CREATE_NEW, 0x80).is_ok() {
                break;
            }
            n = n % 0xffff + 1;
            if n == start {
                return task.fail(cx, 80); // ERROR_FILE_EXISTS
            }
        }
    }
    let _ = task.put_wide(cx, a[3], 260, &name(n));
    Reply::Value(u64::from(n))
}

/// `GetShortPathNameW(long, &short, size)`: the long name, as Windows gives it where 8.3 names are turned off; the
/// file must be there.
pub fn get_short_path_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(path) = task.path_arg(cx, a[0], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if let Err(e) = task.path_info(&path) {
        task.set_last_error(cx, e);
        return Reply::Value(0);
    }
    task.put_wide(cx, a[1], a[2] & 0xffff_ffff, &path)
}

/// `GetDiskFreeSpaceW(root, &sectors per cluster, &bytes per sector, &free clusters, &clusters)`: a 128 GiB disk with
/// 96 GiB free, in 4 KiB clusters.
pub fn get_disk_free_space_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (cluster, total, free) = (4096u64, 128u64 << 30, 96u64 << 30);
    for (at, v) in [(a[1], 8u32), (a[2], 512), (a[3], (free / cluster) as u32), (a[4], (total / cluster) as u32)] {
        if at != 0 && task.mem(cx, true, |t| user::write_u32(&mut t.space(), at, v)).is_err() {
            return task.fail(cx, 998);
        }
    }
    Reply::Value(TRUE)
}

/// `SearchPathW(path, file, extension, size, &buffer, &file part)`: the first folder of `path` (or of the program's
/// folder, the current one, System32 and Windows) with the file, `extension` added if its name has none. Its full
/// name's length, or the size it needs; 0 with ERROR_FILE_NOT_FOUND.
pub fn search_path_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.path_arg(cx, a[1], true) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let extension = if a[2] != 0 { task.path_arg(cx, a[2], true).unwrap_or_default() } else { String::new() };
    let last = file.rsplit(['\\', '/']).next().unwrap_or(&file);
    let name = if last.contains('.') { file.clone() } else { format!("{file}{extension}") };
    let folders: Vec<String> = if file.contains(['\\', '/', ':']) {
        vec![String::new()]
    } else if a[0] != 0 {
        task.path_arg(cx, a[0], true).unwrap_or_default().split(';').filter(|d| !d.is_empty()).map(str::to_string).collect()
    } else {
        let exe_dir = lock(&task.process.state).exe_dir.clone();
        vec![format!("C:{}", exe_dir.replace('/', "\\")), task.cwd(), "C:\\Windows\\System32".into(), "C:\\Windows".into()]
    };
    for folder in folders {
        let candidate = if folder.is_empty() { name.clone() } else { format!("{}\\{name}", folder.trim_end_matches('\\')) };
        let Ok(info) = task.path_info(&candidate) else { continue };
        if info.attributes & crate::files::FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let Ok((full, _)) = task.resolve(&candidate) else { continue };
        let reply = task.put_wide(cx, a[4], a[3] & 0xffff_ffff, &full);
        if let Reply::Value(n) = reply
            && n < a[3] & 0xffff_ffff
            && a[5] != 0
        {
            let part = full.rfind('\\').map_or(0, |i| full[..=i].encode_utf16().count() as u64);
            let _ = task.write_ptr(cx, a[5], a[4] + part * 2);
        }
        return reply;
    }
    task.set_last_error(cx, 2);
    Reply::Value(0)
}

/// An .ini file: its sections in order, each with its keys and values in order, and whether it was UTF-16.
struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
    wide: bool,
}

impl Ini {
    fn parse(bytes: &[u8]) -> Ini {
        let wide = bytes.starts_with(&[0xff, 0xfe]);
        let text = if wide {
            String::from_utf16_lossy(&bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>())
        } else {
            crate::ansi::text(bytes)
        };
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                sections.push((name.trim().to_string(), Vec::new()));
            } else if let Some((key, value)) = line.split_once('=')
                && !line.starts_with(';')
                && let Some((_, keys)) = sections.last_mut()
            {
                keys.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
        Ini { sections, wide }
    }

    fn render(&self) -> Vec<u8> {
        let mut text = String::new();
        for (name, keys) in &self.sections {
            text.push_str(&format!("[{name}]\r\n"));
            for (k, v) in keys {
                text.push_str(&format!("{k}={v}\r\n"));
            }
        }
        if self.wide {
            [0xff, 0xfe].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()
        } else {
            crate::ansi::bytes(&text)
        }
    }

    fn section(&self, name: &str) -> Option<&Vec<(String, String)>> {
        self.sections.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, k)| k)
    }
}

impl Task {
    /// Where an .ini file is: a bare name is in the Windows folder, as Windows has it.
    fn ini_path(&mut self, cx: &mut Cx<'_>, at: u64) -> String {
        let name = self.path_arg(cx, at, true).unwrap_or_else(|| "win.ini".to_string());
        if name.contains(['\\', '/']) { name } else { format!("C:\\Windows\\{name}") }
    }

    fn ini_read(&self, path: &str) -> Ini {
        let bytes = self.resolve(path).ok().and_then(|(_, disk_path)| {
            let disk = lock(&self.k.disk);
            let ino = disk.resolve(layers::ROOT, &disk_path, true).ok()?;
            disk.contents(ino).ok()
        });
        Ini::parse(&bytes.unwrap_or_default())
    }

    fn ini_write(&self, path: &str, ini: &Ini) -> bool {
        let Ok((_, disk_path)) = self.resolve(path) else { return false };
        let now = self.k.host.wall();
        let bytes = ini.render();
        let mut disk = lock(&self.k.disk);
        let dir = crate::Meta { attributes: crate::files::FILE_ATTRIBUTE_DIRECTORY };
        let Ok(ino) = disk.put(&disk_path, layers::New::File, &[], crate::Meta { attributes: 0x20 }, dir) else { return false };
        disk.truncate(ino, 0, now).is_ok() && disk.write(ino, 0, &bytes, now).is_ok()
    }
}

/// `GetPrivateProfileStringW(section, key, default, &buffer, size, file)`: a key's value (or the default); with no
/// key, the section's keys, and with no section, the sections' names, each with a NUL and then another. What it
/// wrote, without the last NUL; cut short, `size` - 1 (or - 2 for a list).
pub fn get_private_profile_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let size = (a[4] & 0xffff_ffff) as usize;
    if size == 0 || a[3] == 0 {
        return Reply::Value(0);
    }
    let path = task.ini_path(cx, a[5]);
    let ini = task.ini_read(&path);
    let (section, key) = (if a[0] != 0 { task.path_arg(cx, a[0], true) } else { None }, if a[1] != 0 { task.path_arg(cx, a[1], true) } else { None });
    let list = |names: Vec<String>| -> (Vec<u16>, usize) {
        let mut units: Vec<u16> = Vec::new();
        for n in names {
            let n: Vec<u16> = n.encode_utf16().collect();
            if units.len() + n.len() + 2 > size {
                let cut = size.saturating_sub(2);
                units.truncate(cut);
                units.extend(n.iter().take(cut.saturating_sub(units.len())));
                units.truncate(cut);
                units.extend([0, 0]);
                return (units, cut);
            }
            units.extend(n);
            units.push(0);
        }
        let n = units.len();
        units.push(0);
        (units, n.saturating_sub(0))
    };
    let (units, n) = match (&section, &key) {
        (None, _) => list(ini.sections.iter().map(|(n, _)| n.clone()).collect()),
        (Some(s), None) => list(ini.section(s).map(|k| k.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default()),
        (Some(s), Some(k)) => {
            let default = if a[2] != 0 { task.path_arg(cx, a[2], true).unwrap_or_default().trim_end().to_string() } else { String::new() };
            let value = ini.section(s).and_then(|keys| keys.iter().find(|(name, _)| name.eq_ignore_ascii_case(k))).map(|(_, v)| {
                // A value in matching quotes is given without them.
                let q = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
                q.unwrap_or(v).to_string()
            });
            let mut units: Vec<u16> = value.unwrap_or(default).encode_utf16().collect();
            units.truncate(size - 1);
            let n = units.len();
            units.push(0);
            (units, n)
        }
    };
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[3], &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(n as u64)
}

/// `WritePrivateProfileStringW(section, key, value, file)`: sets a key; with no value, deletes it; with no key,
/// deletes the section. The file is made if it isn't there.
pub fn write_private_profile_string_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(section) = (if a[0] != 0 { task.path_arg(cx, a[0], true) } else { None }) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let path = task.ini_path(cx, a[3]);
    let mut ini = task.ini_read(&path);
    let key = if a[1] != 0 { task.path_arg(cx, a[1], true) } else { None };
    let value = if a[2] != 0 { task.path_arg(cx, a[2], true) } else { None };
    match key {
        None => ini.sections.retain(|(n, _)| !n.eq_ignore_ascii_case(&section)),
        Some(key) => {
            if !ini.sections.iter().any(|(n, _)| n.eq_ignore_ascii_case(&section)) {
                ini.sections.push((section.clone(), Vec::new()));
            }
            let keys = &mut ini.sections.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&section)).expect("added above").1;
            match value {
                None => keys.retain(|(k, _)| !k.eq_ignore_ascii_case(&key)),
                Some(value) => match keys.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&key)) {
                    Some(entry) => entry.1 = value,
                    None => keys.push((key, value)),
                },
            }
        }
    }
    if task.ini_write(&path, &ini) { Reply::Value(TRUE) } else { task.fail(cx, 5) }
}
