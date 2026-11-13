//! kernel32's processes and pipes, in host Rust: `CreateProcess` (finding the program as Windows does, its command
//! line, environment, current directory, inherited and standard handles, and starting suspended), `OpenProcess`,
//! `TerminateProcess`, anonymous pipes (`CreatePipe`, `PeekNamedPipe`) and handles' inheritance.
//!
//! A new process gets a host thread and a CPU of its own, and a space of its own; the objects and their names are the
//! machine's, so processes share them as on Windows.

use std::sync::Arc;

use exec::Isa;
use kernels_core::thread::Cx;
use kernels_core::{lock, user};

use super::Reply;
use crate::files::{ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, FILE_ATTRIBUTE_DIRECTORY, File, Node};
use crate::handles::{CURRENT_PROCESS, Handle};
use crate::images::pe::Pe;
use crate::objects::Kind;
use crate::pipes;
use crate::process::{Launch, Process, Task};

const TRUE: u64 = 1;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_BAD_EXE_FORMAT: u32 = 193;
const ERROR_DIRECTORY: u32 = 267;
const ERROR_INVALID_FUNCTION: u32 = 1;

const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const STARTF_USESTDHANDLES: u32 = 0x100;
const HANDLE_FLAG_INHERIT: u32 = 1;

/// STARTUPINFO, by ISA: its size, where its flags are, and where its three standard handles start (each pointer-sized).
/// A 32-bit one is 68 bytes, a 64-bit one 104.
pub(crate) fn startup(isa: Isa) -> (u64, u64, u64) {
    if isa == Isa::X86_32 { (68, 44, 56) } else { (104, 60, 80) }
}

impl Task {
    /// Whether a SECURITY_ATTRIBUTES (or none) says the handle made with it is inheritable.
    pub(crate) fn inheritable_by(&mut self, cx: &mut Cx<'_>, attributes: u64) -> bool {
        // nLength, then lpSecurityDescriptor, then bInheritHandle.
        let at = attributes + if self.isa == Isa::X86_32 { 8 } else { 16 };
        attributes != 0 && self.mem(cx, false, |t| user::read_u32(&t.space(), at)).is_ok_and(|v| v != 0)
    }

    /// The process a handle stands for: this one, by `GetCurrentProcess()` or a handle, or another that hasn't ended.
    pub(crate) fn process_of(&self, handle: u64) -> Option<Arc<Process>> {
        let pid = match self.object(handle)?.kind {
            Kind::Process(pid) => pid,
            _ => return None,
        };
        if pid == self.process.pid { Some(self.process.clone()) } else { self.k.process(pid) }
    }

    /// A string the program passed, as UTF-16 or ANSI: a command line may be 32,767 characters.
    fn string_arg(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at == 0 {
            return None;
        }
        if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 32767)).ok()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).ok().map(|b| crate::ansi::text(&b))
        }
    }

    /// A UTF-16 environment block in the program's memory, as bytes up to and with its closing NUL.
    fn wide_block(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<Vec<u8>> {
        let mut block = Vec::new();
        let mut previous = 1u16;
        for i in 0..(1u64 << 19) {
            let unit = self.mem(cx, false, |t| user::read_vec(&t.space(), at + i * 2, 2)).ok()?;
            let unit = u16::from_le_bytes([unit[0], unit[1]]);
            block.extend(unit.to_le_bytes());
            if unit == 0 && (previous == 0 || i == 0) {
                return Some(block);
            }
            previous = unit;
        }
        None
    }

    /// An ANSI environment block, made UTF-16.
    fn ansi_block(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<Vec<u8>> {
        let mut units: Vec<u16> = Vec::new();
        let mut at = at;
        loop {
            let s = self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).ok()?;
            if s.is_empty() {
                break;
            }
            at += s.len() as u64 + 1;
            units.extend(crate::ansi::text(&s).encode_utf16());
            units.push(0);
        }
        units.push(0);
        Some(units.iter().flat_map(|u| u.to_le_bytes()).collect())
    }

    /// The program's environment as it is now (ntdll keeps it, and its changes, in the process parameters).
    fn environment_now(&mut self, cx: &mut Cx<'_>) -> Vec<u8> {
        let peb_at = lock(&self.process.state).peb;
        let abi = self.abi();
        let at = self.mem(cx, false, |t| {
            let params_at = user::read_ptr(&t.space(), peb_at + abi.peb.process_parameters, abi.width)?;
            user::read_ptr(&t.space(), params_at + abi.params.environment, abi.width)
        });
        at.ok().and_then(|at| self.wide_block(cx, at)).unwrap_or_else(|| vec![0, 0])
    }

    /// A program at a Windows path: its full path, where it is on the disk, and its bytes.
    fn program_at(&self, raw: &str) -> Option<(String, String, Vec<u8>)> {
        let (windows, disk_path) = self.resolve(raw).ok()?;
        let disk = lock(&self.k.disk);
        let ino = disk.resolve(layers::ROOT, &disk_path, true).ok()?;
        let bytes = disk.contents(ino).ok()?.to_vec();
        Some((windows, disk_path, bytes))
    }

    /// Finds the program a command line starts with, as `CreateProcess` does with no application name: `.exe` added
    /// when it has no extension, then the parent's folder, the current directory, System32, Windows, and `Path`.
    fn find_program(&mut self, cx: &mut Cx<'_>, name: &str) -> Option<(String, String, Vec<u8>)> {
        let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let name = if file.contains('.') { name.to_string() } else { format!("{name}.exe") };
        if name.contains(['\\', '/', ':']) {
            return self.program_at(&name);
        }
        let exe_dir = lock(&self.process.state).exe_dir.clone();
        let mut dirs = vec![format!("C:{}", exe_dir.replace('/', "\\")), self.cwd(), "C:\\Windows\\System32".to_string(), "C:\\Windows".to_string()];
        let environment = self.environment_now(cx);
        let units: Vec<u16> = environment.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for var in units.split(|&u| u == 0).map(String::from_utf16_lossy) {
            if var.len() > 5 && var[..5].eq_ignore_ascii_case("path=") {
                dirs.extend(var[5..].split(';').filter(|d| !d.is_empty()).map(str::to_string));
            }
        }
        dirs.iter().find_map(|dir| self.program_at(&format!("{}\\{name}", dir.trim_end_matches('\\'))))
    }
}

/// The program a command line names: its first argument, quoted or up to the first space.
fn first_argument(line: &str) -> &str {
    let line = line.trim_start();
    match line.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => line.split([' ', '\t']).next().unwrap_or(line),
    }
}

fn create_process(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let [app, line, process_attributes, thread_attributes, environment, directory, startup_info, information] = [a[0], a[1], a[2], a[3], a[6], a[7], a[8], a[9]];
    let (inherit, flags) = (a[4] as u32 != 0, a[5] as u32);
    let app = task.string_arg(cx, app, wide);
    let line = task.string_arg(cx, line, wide);
    let command_line = match (&line, &app) {
        (Some(line), _) => line.clone(),
        (None, Some(app)) if app.contains(' ') => format!("\"{app}\""),
        (None, Some(app)) => app.clone(),
        (None, None) => return task.fail(cx, ERROR_INVALID_PARAMETER),
    };
    let found = match &app {
        Some(app) => task.program_at(app),
        None => task.find_program(cx, first_argument(&command_line)),
    };
    let Some((_, path, bytes)) = found else { return task.fail(cx, ERROR_FILE_NOT_FOUND) };
    if Pe::parse(&bytes).map_or(true, |pe| pe.is_dll) {
        return task.fail(cx, ERROR_BAD_EXE_FORMAT);
    }

    let environment = match environment {
        0 => Some(task.environment_now(cx)),
        at if wide || flags & CREATE_UNICODE_ENVIRONMENT != 0 => task.wide_block(cx, at),
        at => task.ansi_block(cx, at),
    };
    let Some(environment) = environment else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let cwd = match task.string_arg(cx, directory, wide) {
        None => task.cwd(),
        Some(dir) => match task.path_info(&dir) {
            Ok(info) if info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 => match task.resolve(&dir) {
                Ok((full, _)) => full,
                Err(e) => return task.fail(cx, e),
            },
            _ => return task.fail(cx, ERROR_DIRECTORY),
        },
    };

    // What it inherits, and its standard handles: STARTUPINFO's (which it has only if it inherited them, as on
    // Windows), else this process's, which share its console.
    let ((_, flags_at, std_at), width) = (startup(task.isa), task.abi().width);
    let Ok((startup_flags, given)) = task.mem(cx, false, |t| {
        let flags = user::read_u32(&t.space(), startup_info + flags_at)?;
        let mut given = [0u64; 3];
        for (i, h) in given.iter_mut().enumerate() {
            *h = user::read_ptr(&t.space(), startup_info + std_at + i as u64 * width, width)?;
        }
        Ok((flags, given))
    }) else {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    };
    let (handles, std_handles) = {
        let state = lock(&task.process.state);
        let mut handles = if inherit { state.handles.inherited() } else { Default::default() };
        let std_handles = if startup_flags & STARTF_USESTDHANDLES != 0 {
            given
        } else {
            for &h in &state.std_handles {
                if !handles.contains(h)
                    && let Some(handle) = state.handles.get(h)
                {
                    handles.put(h, handle, false);
                }
            }
            state.std_handles
        };
        (handles, std_handles)
    };

    let launch = Launch { path, bytes, command_line, environment, cwd: Some(cwd), handles, std_handles, suspended: flags & CREATE_SUSPENDED != 0, parent: task.process.pid };
    let k = task.k.clone();
    let (made, child) = std::sync::mpsc::channel();
    let started = kernels_core::thread::spawn(task.k.exec.clone(), "atlas process".to_string(), move |cx| {
        let first = crate::process::launch(&k, cx, launch)?;
        let _ = made.send((first.process.clone(), first.control.object.clone(), first.tid));
        Ok(first)
    });
    let (process, thread, tid) = match started.ok().and_then(|_| child.recv().ok()) {
        Some(child) => child,
        None => {
            task.k.host.write(host::Stream::Err, b"atlas: CreateProcess: the new process didn't start\n");
            return task.fail(cx, ERROR_NOT_ENOUGH_MEMORY);
        }
    };
    let process_inherit = task.inheritable_by(cx, process_attributes);
    let thread_inherit = task.inheritable_by(cx, thread_attributes);
    let (process_handle, thread_handle) = {
        let mut state = lock(&task.process.state);
        (state.handles.add_with(Handle::Object(process.object.clone()), process_inherit), state.handles.add_with(Handle::Object(thread), thread_inherit))
    };
    // PROCESS_INFORMATION: the two handles, then the two IDs.
    let abi = task.abi();
    let info = [abi.ptr(process_handle), abi.ptr(thread_handle), (process.pid as u32).to_le_bytes().to_vec(), (tid as u32).to_le_bytes().to_vec()].concat();
    let wrote = task.mem(cx, true, |t| user::write(&mut t.space(), information, &info));
    if wrote.is_err() {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(TRUE)
}

pub fn create_process_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_process(task, cx, a, true)
}

pub fn create_process_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_process(task, cx, a, false)
}

/// `OpenProcess(access, inherit, pid)`.
pub fn open_process(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let pid = a[2] as u32 as u64;
    let process = if pid == task.process.pid { Some(task.process.clone()) } else { task.k.process(pid) };
    match process {
        Some(process) => Reply::Value(lock(&task.process.state).handles.add_with(Handle::Object(process.object.clone()), a[1] as u32 != 0)),
        None => {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            Reply::Value(0)
        }
    }
}

/// `TerminateProcess(process, code)`: this one ends now; another ends at its threads' next chance.
pub fn terminate_process(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == CURRENT_PROCESS {
        return Reply::ExitProcess(a[1] as u32);
    }
    match task.object(a[0]).map(|o| o.kind) {
        Some(Kind::Process(pid)) if pid == task.process.pid => Reply::ExitProcess(a[1] as u32),
        Some(Kind::Process(pid)) => {
            // One that's ended already has nothing left to end.
            if let Some(process) = task.k.process(pid) {
                process.end(a[1] as u32);
            }
            Reply::Value(TRUE)
        }
        _ => task.fail(cx, ERROR_INVALID_HANDLE),
    }
}

/// `CreatePipe(&read, &write, attributes, size)`.
pub fn create_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let inherit = task.inheritable_by(cx, a[2]);
    let (reader, writer) = pipes::pair(a[3] as u32 as usize);
    let (read, write) = {
        let mut state = lock(&task.process.state);
        let read = state.handles.add_with(Handle::File(Arc::new(File::device(Node::Pipe(reader), true, false))), inherit);
        let write = state.handles.add_with(Handle::File(Arc::new(File::device(Node::Pipe(writer), false, true))), inherit);
        (read, write)
    };
    let abi = task.abi();
    let wrote = task.mem(cx, true, |t| {
        user::write(&mut t.space(), a[0], &abi.ptr(read))?;
        user::write(&mut t.space(), a[1], &abi.ptr(write))
    });
    if wrote.is_err() {
        let mut state = lock(&task.process.state);
        state.handles.close(read);
        state.handles.close(write);
        drop(state);
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(TRUE)
}

/// `PeekNamedPipe(pipe, buffer, size, &read, &available, &left)`: what's in a pipe, without taking it.
pub fn peek_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let handle = lock(&task.process.state).handles.get(a[0]);
    let file = match handle {
        Some(Handle::File(file)) => file,
        Some(_) => return task.fail(cx, ERROR_INVALID_FUNCTION),
        None => return task.fail(cx, ERROR_INVALID_HANDLE),
    };
    let pipe = match &file.node {
        Node::Pipe(end) => Ok(end.pipe.clone()),
        Node::NamedPipe(end) => end.incoming(),
        _ => return task.fail(cx, ERROR_INVALID_FUNCTION),
    };
    let size = if a[1] == 0 { 0 } else { a[2] as u32 as usize };
    let (data, available, left) = match pipe.and_then(|p| p.peek(size)) {
        Ok(peeked) => peeked,
        Err(e) => return task.fail(cx, e),
    };
    let wrote = task.mem(cx, true, |t| {
        let mut space = t.space();
        if !data.is_empty() {
            user::write(&mut space, a[1], &data)?;
        }
        for (at, value) in [(a[3], data.len()), (a[4], available), (a[5], left)] {
            if at != 0 {
                user::write_u32(&mut space, at, value as u32)?;
            }
        }
        Ok(())
    });
    if wrote.is_err() { task.fail(cx, ERROR_INVALID_PARAMETER) } else { Reply::Value(TRUE) }
}

/// `GetHandleInformation(handle, &flags)`: whether it's inheritable.
pub fn get_handle_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(inheritable) = lock(&task.process.state).handles.inheritable(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let flags = if inheritable { HANDLE_FLAG_INHERIT } else { 0 };
    if task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], flags)).is_err() {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(TRUE)
}

/// `SetHandleInformation(handle, mask, flags)`: `HANDLE_FLAG_INHERIT` (the only flag atlas keeps).
pub fn set_handle_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (mask, flags) = (a[1] as u32, a[2] as u32);
    let mut state = lock(&task.process.state);
    let there = if mask & HANDLE_FLAG_INHERIT != 0 { state.handles.set_inheritable(a[0], flags & HANDLE_FLAG_INHERIT != 0) } else { state.handles.contains(a[0]) };
    drop(state);
    if there { Reply::Value(TRUE) } else { task.fail(cx, ERROR_INVALID_HANDLE) }
}

/// `GetStartupInfoW`/`A`: the size, and nothing a parent asked for (atlas keeps none of it yet).
pub fn get_startup_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (size, _, _) = startup(task.isa);
    let mut info = vec![0u8; size as usize];
    info[..4].copy_from_slice(&(size as u32).to_le_bytes());
    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), a[0], &info));
    Reply::Value(0)
}

// --- Threads by ID, and their priorities ---

/// `OpenThread(access, inherit, tid)`: a handle to any process's thread that hasn't ended.
pub fn open_thread(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let tid = a[2] as u32 as u64;
    let own = lock(&task.process.state).threads.get(&tid).cloned();
    match own.or_else(|| task.k.thread(tid)) {
        Some(thread) => Reply::Value(lock(&task.process.state).handles.add_with(Handle::Object(thread.object.clone()), a[1] as u32 != 0)),
        None => {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            Reply::Value(0)
        }
    }
}

/// `SetThreadPriority(thread, priority)`: any of Windows' levels, for a thread; the host schedules atlas's threads
/// alike, so it's only checked.
pub fn set_thread_priority(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.thread_of(a[0]).is_none() {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    }
    let priority = a[1] as u32 as i32;
    // THREAD_PRIORITY_IDLE (-15) to TIME_CRITICAL (15), and THREAD_MODE_BACKGROUND_BEGIN and END.
    if !(matches!(priority, -2..=2 | -15 | 15 | 0x10000 | 0x20000)) {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    Reply::Value(TRUE)
}

/// `GetThreadPriority(thread)`: THREAD_PRIORITY_NORMAL, or THREAD_PRIORITY_ERROR_RETURN for what isn't a thread.
pub fn get_thread_priority(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if task.thread_of(a[0]).is_none() {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(0x7fff_ffff);
    }
    Reply::Value(0)
}

// --- psapi: a process's modules and memory, as kernel32's K32* functions (psapi.dll's are the same) ---

const ERROR_ACCESS_DENIED: u32 = 5;

/// A module of a process: its base (0 is its program's) and Windows path.
fn module_of(process: &Process, module: u64) -> Option<(u64, String)> {
    let state = lock(&process.state);
    if module == 0 {
        return Some((0, state.image.clone()));
    }
    state.mapped.values().find(|(base, _)| *base == module).cloned()
}

/// `K32GetModuleFileNameExW(process, module, out, room)` and `A`: the module's full path, cut to fit (with its NUL);
/// its length without the NUL.
fn get_module_file_name_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(process) = task.process_of(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let room = a[3] as u32 as usize;
    let Some((_, path)) = module_of(&process, a[1]).or_else(|| (a[1] == task.exe_base(cx) && Arc::ptr_eq(&process, &task.process)).then(|| (0, lock(&process.state).image.clone()))) else {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    };
    if room == 0 {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    let (bytes, count) = if wide {
        let mut units: Vec<u16> = path.encode_utf16().collect();
        units.truncate(room - 1);
        let n = units.len();
        (units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>(), n)
    } else {
        let mut b = crate::ansi::bytes(&path).to_vec();
        b.truncate(room - 1);
        let n = b.len();
        b.push(0);
        (b, n)
    };
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &bytes)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(count as u64)
}

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

pub fn get_module_file_name_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_module_file_name_ex(task, cx, a, true)
}

pub fn get_module_file_name_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    get_module_file_name_ex(task, cx, a, false)
}

impl Task {
    /// Where this process's program is, from its PEB.
    fn exe_base(&mut self, cx: &mut Cx<'_>) -> u64 {
        let abi = self.abi();
        let peb = lock(&self.process.state).peb;
        self.mem(cx, false, |t| user::read_ptr(&t.space(), peb + abi.peb.image_base, abi.width)).unwrap_or(0)
    }
}

/// `K32GetModuleInformation(process, module, &info, size)`: MODULEINFO's base, size of image and entry point, read
/// from the module's headers. Only this process's: another's memory isn't read from here.
pub fn get_module_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(process) = task.process_of(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    if !Arc::ptr_eq(&process, &task.process) {
        return task.fail(cx, ERROR_ACCESS_DENIED);
    }
    let abi = task.abi();
    let size = 2 * abi.width + if abi.width == 8 { 8 } else { 4 };
    if a[3] < size {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    let base = if a[1] == 0 { task.exe_base(cx) } else { a[1] };
    if base != task.exe_base(cx) && module_of(&process, base).is_none() {
        return task.fail(cx, ERROR_INVALID_HANDLE);
    }
    let headers = task.mem(cx, false, |t| {
        let lfanew = u64::from(user::read_u32(&t.space(), base + 0x3c)?);
        Ok((user::read_u32(&t.space(), base + lfanew + 0x28)?, user::read_u32(&t.space(), base + lfanew + 0x50)?))
    });
    let Ok((entry, image_size)) = headers else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let mut info = abi.ptr(base);
    info.extend(image_size.to_le_bytes());
    if abi.width == 8 {
        info.extend([0; 4]);
    }
    info.extend(abi.ptr(if entry == 0 { 0 } else { base + u64::from(entry) }));
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[2], &info)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}

/// `K32GetProcessMemoryInfo(process, &counters, size)`: PROCESS_MEMORY_COUNTERS(_EX), its working set and
/// commit both what the process has committed now (there's no paging to tell them apart), peaks the same.
pub fn get_process_memory_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(process) = task.process_of(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let abi = task.abi();
    let w = abi.width;
    let (plain, ex) = (8 + 8 * w, 8 + 9 * w);
    let size = a[2] as u32 as u64;
    if size < plain {
        return task.fail(cx, ERROR_INSUFFICIENT_BUFFER);
    }
    let committed: u64 = {
        let state = lock(&process.state);
        state.regions.iter().filter(|(_, area)| !matches!(area.kind, crate::process::AreaKind::Reserved | crate::process::AreaKind::Kept)).map(|(r, _)| r.end - r.start).sum()
    };
    let size = if size >= ex { ex } else { plain };
    let mut b = (size as u32).to_le_bytes().to_vec();
    b.extend(((committed / 4096) as u32).to_le_bytes());
    // Peak and current working set, the pool quotas (none), then the pagefile's, peak, and private bytes.
    for v in [committed, committed, 0, 0, 0, 0, committed, committed, committed].iter().take(((size - 8) / w) as usize) {
        b.extend(abi.ptr(*v));
    }
    if task.mem(cx, true, |t| user::write(&mut t.space(), a[1], &b)).is_err() {
        return task.fail(cx, 998);
    }
    Reply::Value(TRUE)
}
