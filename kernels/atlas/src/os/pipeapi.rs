//! kernel32's named pipes, in host Rust over `named_pipes`: `CreateNamedPipe`, a client's `CreateFile` of
//! `\\.\pipe\NAME`, `ConnectNamedPipe` (waiting, or overlapped and pending until a client comes),
//! `DisconnectNamedPipe`, `WaitNamedPipe`, the handle state and information, the processes at each end, and
//! `TransactNamedPipe` and `CallNamedPipe`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kernels_core::thread::Cx;
use kernels_core::wait::Woke;
use kernels_core::{lock, user};

use super::Reply;
use super::fileapi::{Notify, transfer};
use crate::files::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE, ERROR_INVALID_NAME, ERROR_INVALID_PARAMETER, File, Node};
use crate::handles::Handle;
use crate::named_pipes::{self, Channel, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, Instance, InstanceState, Phase, PipeEnd, Server};
use crate::pipes::{self, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED, Pipe};
use crate::ports::{self, Completion};
use crate::process::Task;

const TRUE: u64 = 1;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;
const ERROR_BAD_PIPE: u32 = 230;
const ERROR_SEM_TIMEOUT: u32 = 121;

const PIPE_ACCESS_INBOUND: u32 = 1;
const PIPE_ACCESS_OUTBOUND: u32 = 2;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x8_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const PIPE_TYPE_MESSAGE: u32 = 4;
const PIPE_READMODE_MESSAGE: u32 = 2;
const PIPE_NOWAIT: u32 = 1;

impl Task {
    /// The named pipe end a handle is, with its file.
    pub(crate) fn pipe_end(&self, handle: u64) -> Option<Arc<File>> {
        match lock(&self.process.state).handles.get(handle)? {
            Handle::File(file) if matches!(file.node, Node::NamedPipe(_)) => Some(file),
            _ => None,
        }
    }

    fn pipe_name(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at == 0 {
            return None;
        }
        if wide {
            self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 260)).ok()
        } else {
            self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 260)).ok().map(|b| crate::ansi::text(&b))
        }
    }

    /// A client's connection to the pipe named `key`: a listening instance taken, and an overlapped
    /// `ConnectNamedPipe` waiting on it completed. ERROR_PIPE_BUSY if none is listening.
    pub(crate) fn connect_client(&mut self, cx: &mut Cx<'_>, key: &str) -> Result<PipeEnd, u32> {
        let server = lock(&self.k.named_pipes).get(key).and_then(Weak::upgrade).ok_or(ERROR_FILE_NOT_FOUND)?;
        for instance in server.live() {
            let mut s = lock(&instance.state);
            if s.phase != Phase::Listening {
                continue;
            }
            let channel = Arc::new(Channel { to_server: Pipe::new(server.in_size, server.message), to_client: Pipe::new(server.out_size, server.message) });
            s.phase = Phase::Connected;
            s.channel = Some(channel.clone());
            s.client_pid = self.process.pid;
            let connect = s.connect.take();
            for w in s.waiters.drain(..) {
                w.wake();
            }
            drop(s);
            if let Some(op) = connect {
                let k = self.k.clone();
                k.complete(cx, Completion { op, data: Vec::new(), error: 0 });
            }
            return Ok(PipeEnd { k: Arc::downgrade(&self.k), instance, channel: Some(channel), message_mode: AtomicBool::new(false), nowait: AtomicBool::new(false) });
        }
        Err(ERROR_PIPE_BUSY)
    }

    /// Waits up to `timeout_ms` for an instance of `key` to be listening.
    fn wait_named(&mut self, key: &str, timeout_ms: u32) -> Result<(), u32> {
        let server = lock(&self.k.named_pipes).get(key).and_then(Weak::upgrade).ok_or(ERROR_FILE_NOT_FOUND)?;
        let timeout = match timeout_ms {
            0 => server.timeout_ms.max(50),
            t => t,
        };
        let deadline = (timeout != u32::MAX).then(|| Instant::now() + Duration::from_millis(u64::from(timeout)));
        loop {
            self.waiter.reset();
            if server.live().iter().any(|i| lock(&i.state).phase == Phase::Listening) {
                return Ok(());
            }
            lock(&server.waiters).push(self.waiter.clone());
            match self.waiter.wait(deadline) {
                Woke::TimedOut => return Err(ERROR_SEM_TIMEOUT),
                Woke::Interrupted if self.ending() => return Err(ERROR_SEM_TIMEOUT),
                Woke::Interrupted => self.waiter.clear_interrupt(),
                Woke::Woken => {}
            }
        }
    }
}

/// A client's `CreateFile` of a pipe: its file.
pub(crate) fn open_client(task: &mut Task, cx: &mut Cx<'_>, key: &str, path: &str, access: u32, flags: u32) -> Result<Arc<File>, u32> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    let end = task.connect_client(cx, key)?;
    let read = access & (GENERIC_READ | GENERIC_ALL | 1) != 0;
    let write = access & (GENERIC_WRITE | GENERIC_ALL | 2) != 0;
    Ok(Arc::new(File::new(Node::NamedPipe(end), read, write, false, path.to_string(), String::new(), false, flags & FILE_FLAG_OVERLAPPED != 0)))
}

/// `CreateNamedPipeW/A(name, open mode, pipe mode, instances, out size, in size, timeout, attributes)`.
fn create_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let (open_mode, pipe_mode, max) = (a[1] as u32, a[2] as u32, a[3] as u32);
    let fail = |task: &mut Task, cx: &mut Cx<'_>, e: u32| {
        task.set_last_error(cx, e);
        Reply::Value(INVALID_HANDLE_VALUE)
    };
    let Some(name) = task.pipe_name(cx, a[0], wide) else { return fail(task, cx, ERROR_INVALID_PARAMETER) };
    let Some(key) = named_pipes::key(&name) else { return fail(task, cx, ERROR_INVALID_NAME) };
    let message = pipe_mode & PIPE_TYPE_MESSAGE != 0;
    if !(1..=255).contains(&max) || (!message && pipe_mode & PIPE_READMODE_MESSAGE != 0) || open_mode & (PIPE_ACCESS_INBOUND | PIPE_ACCESS_OUTBOUND) == 0 {
        return fail(task, cx, ERROR_INVALID_PARAMETER);
    }
    let inheritable = task.inheritable_by(cx, a[7]);
    let server = {
        let mut pipes = lock(&task.k.named_pipes);
        pipes.retain(|_, w| w.strong_count() > 0);
        match pipes.get(&key).and_then(Weak::upgrade) {
            Some(server) => server,
            None => {
                let server = Arc::new(Server {
                    message,
                    max,
                    out_size: a[4] as u32 as usize,
                    in_size: a[5] as u32 as usize,
                    timeout_ms: a[6] as u32,
                    instances: Mutex::new(Vec::new()),
                    waiters: Mutex::new(Vec::new()),
                });
                pipes.insert(key, Arc::downgrade(&server));
                server
            }
        }
    };
    let live = server.live().len();
    if live > 0 && (open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0 || server.message != message) {
        return fail(task, cx, ERROR_ACCESS_DENIED);
    }
    if live >= server.max as usize && server.max != 255 {
        return fail(task, cx, ERROR_PIPE_BUSY);
    }
    let state = InstanceState { phase: Phase::Listening, channel: None, connect: None, client_pid: 0, waiters: Vec::new() };
    let instance = Arc::new(Instance { server: server.clone(), server_pid: task.process.pid, state: Mutex::new(state) });
    lock(&server.instances).push(Arc::downgrade(&instance));
    server.listening();
    let end = PipeEnd {
        k: Arc::downgrade(&task.k),
        instance,
        channel: None,
        message_mode: AtomicBool::new(pipe_mode & PIPE_READMODE_MESSAGE != 0),
        nowait: AtomicBool::new(pipe_mode & PIPE_NOWAIT != 0),
    };
    let file = File::new(Node::NamedPipe(end), open_mode & PIPE_ACCESS_INBOUND != 0, open_mode & PIPE_ACCESS_OUTBOUND != 0, false, String::new(), String::new(), false, open_mode & FILE_FLAG_OVERLAPPED != 0);
    task.set_last_error(cx, 0);
    Reply::Value(lock(&task.process.state).handles.add_with(Handle::File(Arc::new(file)), inheritable))
}

pub fn create_named_pipe_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_named_pipe(task, cx, a, true)
}

pub fn create_named_pipe_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_named_pipe(task, cx, a, false)
}

/// `ConnectNamedPipe(pipe, overlapped)`: waits for a client, or with an overlapped handle, leaves it pending.
pub fn connect_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    if !end.is_server() {
        return task.fail(cx, 1); // ERROR_INVALID_FUNCTION
    }
    let instance = end.instance.clone();
    {
        let mut s = lock(&instance.state);
        match s.phase {
            Phase::Connected => {
                drop(s);
                return task.fail(cx, ERROR_PIPE_CONNECTED);
            }
            Phase::Broken => {
                drop(s);
                return task.fail(cx, ERROR_NO_DATA);
            }
            Phase::Disconnected => {
                s.phase = Phase::Listening;
                drop(s);
                instance.server.listening();
            }
            Phase::Listening => {}
        }
    }
    if end.nowait() {
        return task.fail(cx, ERROR_PIPE_LISTENING);
    }
    let ov = a[1];
    if file.overlapped && ov != 0 {
        let abi = task.abi();
        let event = task.mem(cx, false, |t| user::read_ptr(&t.space(), ov + abi.overlapped_event(), abi.width)).ok().filter(|&e| e != 0).and_then(|e| task.object(e));
        if let Some(event) = &event {
            task.k.objects.signal(event, |s| s.set = false);
        }
        let op = Arc::new(ports::Op {
            process: Arc::downgrade(&task.process),
            thread: Arc::downgrade(&task.control),
            overlapped: ov,
            buffer: 0,
            event,
            port: lock(&file.port).clone(),
            routine: None,
        });
        let pending = task.abi().overlapped_result(ports::STATUS_PENDING, 0);
        let _ = task.mem(cx, true, |t| user::write(&mut t.space(), ov, &pending));
        let mut s = lock(&instance.state);
        if s.phase == Phase::Listening {
            s.connect = Some(op);
            drop(s);
            return task.fail(cx, ports::ERROR_IO_PENDING);
        }
        // A client came meanwhile.
        drop(s);
        return task.fail(cx, ERROR_PIPE_CONNECTED);
    }
    loop {
        task.waiter.reset();
        {
            let mut s = lock(&instance.state);
            if s.phase != Phase::Listening {
                return Reply::Value(TRUE);
            }
            s.waiters.push(task.waiter.clone());
        }
        if task.waiter.wait(None) == Woke::Interrupted {
            if task.ending() {
                return Reply::Ended;
            }
            task.waiter.clear_interrupt();
        }
    }
}

/// `DisconnectNamedPipe(pipe)`: its client's connection ends (what's unread goes), and the instance waits for
/// `ConnectNamedPipe`.
pub fn disconnect_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    if !end.is_server() {
        return task.fail(cx, 1);
    }
    let mut done = Vec::new();
    {
        let mut s = lock(&end.instance.state);
        if let Some(c) = s.channel.take() {
            c.to_client.close_reader();
            done.extend(c.to_client.close_writer(ERROR_PIPE_NOT_CONNECTED));
            c.to_server.close_reader();
            done.extend(c.to_server.close_writer(ERROR_PIPE_NOT_CONNECTED));
        }
        if let Some(op) = s.connect.take() {
            done.push(Completion { op, data: Vec::new(), error: ERROR_PIPE_NOT_CONNECTED });
        }
        s.phase = Phase::Disconnected;
        s.client_pid = 0;
    }
    for c in done {
        let k = task.k.clone();
        k.complete(cx, c);
    }
    Reply::Value(TRUE)
}

/// `WaitNamedPipeW/A(name, timeout)`.
fn wait_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(key) = task.pipe_name(cx, a[0], wide).and_then(|n| named_pipes::key(&n)) else { return task.fail(cx, ERROR_INVALID_NAME) };
    match task.wait_named(&key, a[1] as u32) {
        Ok(()) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

pub fn wait_named_pipe_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wait_named_pipe(task, cx, a, true)
}

pub fn wait_named_pipe_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    wait_named_pipe(task, cx, a, false)
}

/// `SetNamedPipeHandleState(pipe, &mode, &max collection, &timeout)`: its read mode and whether it waits.
pub fn set_named_pipe_handle_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    if a[1] != 0 {
        let Ok(mode) = task.mem(cx, false, |t| user::read_u32(&t.space(), a[1])) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
        if mode & PIPE_READMODE_MESSAGE != 0 && !end.instance.server.message {
            return task.fail(cx, ERROR_INVALID_PARAMETER);
        }
        end.message_mode.store(mode & PIPE_READMODE_MESSAGE != 0, Ordering::Relaxed);
        end.nowait.store(mode & PIPE_NOWAIT != 0, Ordering::Relaxed);
    }
    Reply::Value(TRUE)
}

/// `GetNamedPipeInfo(pipe, &flags, &out size, &in size, &instances)`.
pub fn get_named_pipe_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let server = &end.instance.server;
    let flags = u32::from(end.is_server()) | if server.message { PIPE_TYPE_MESSAGE } else { 0 };
    let values = [(a[1], flags), (a[2], server.out_size as u32), (a[3], server.in_size as u32), (a[4], server.max)];
    let _ = task.mem(cx, true, |t| {
        for (at, v) in values {
            if at != 0 {
                user::write_u32(&mut t.space(), at, v)?;
            }
        }
        Ok(())
    });
    Reply::Value(TRUE)
}

fn end_process_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], client: bool) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let pid = if client { lock(&end.instance.state).client_pid } else { end.instance.server_pid };
    if client && pid == 0 {
        return task.fail(cx, ERROR_PIPE_NOT_CONNECTED);
    }
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[1], pid as u32));
    Reply::Value(TRUE)
}

/// `GetNamedPipeClientProcessId(pipe, &pid)`.
pub fn get_named_pipe_client_process_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    end_process_id(task, cx, a, true)
}

/// `GetNamedPipeServerProcessId(pipe, &pid)`.
pub fn get_named_pipe_server_process_id(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    end_process_id(task, cx, a, false)
}

/// `TransactNamedPipe(pipe, in, in size, out, out size, &read, overlapped)`: a message written, then one read.
pub fn transact_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.pipe_end(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let Node::NamedPipe(end) = &file.node else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    if !end.message_mode() {
        return task.fail(cx, ERROR_BAD_PIPE);
    }
    let wrote = transfer(task, cx, a[0], a[1], a[2] as u32, 0, 0, true, Notify::Plain);
    if wrote != Reply::Value(TRUE) {
        return wrote;
    }
    transfer(task, cx, a[0], a[3], a[4] as u32, a[5], a[6], false, Notify::Plain)
}

/// `CallNamedPipeW/A(name, in, in size, out, out size, &read, timeout)`: connects (waiting up to `timeout` for an
/// instance), sends a message, reads the reply, and closes.
fn call_named_pipe(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    let Some(key) = task.pipe_name(cx, a[0], wide).and_then(|n| named_pipes::key(&n)) else { return task.fail(cx, ERROR_INVALID_NAME) };
    let end = match task.connect_client(cx, &key) {
        Ok(end) => end,
        Err(ERROR_PIPE_BUSY) => match task.wait_named(&key, a[6] as u32).and_then(|()| task.connect_client(cx, &key)) {
            Ok(end) => end,
            Err(e) => return task.fail(cx, e),
        },
        Err(e) => return task.fail(cx, e),
    };
    if !end.instance.server.message {
        return task.fail(cx, ERROR_BAD_PIPE);
    }
    let Ok(message) = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], a[2] as u32 as usize)) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    let (Ok(outgoing), Ok(incoming)) = (end.outgoing(), end.incoming()) else { return task.fail(cx, ERROR_PIPE_NOT_CONNECTED) };
    match task.write_pipe(cx, &outgoing, &message) {
        Ok(_) => {}
        Err(pipes::Failed::Error(e)) => return task.fail(cx, e),
        Err(pipes::Failed::Ended(reply)) => return reply,
    }
    let (reply, more) = match task.read_pipe(&incoming, a[4] as u32 as usize, true) {
        Ok(read) => read,
        Err(pipes::Failed::Error(e)) => return task.fail(cx, e),
        Err(pipes::Failed::Ended(reply)) => return reply,
    };
    let n = reply.len() as u32;
    let _ = task.mem(cx, true, |t| {
        user::write(&mut t.space(), a[3], &reply)?;
        user::write_u32(&mut t.space(), a[5], n)
    });
    drop(end);
    if more { task.fail(cx, ports::ERROR_MORE_DATA) } else { Reply::Value(TRUE) }
}

pub fn call_named_pipe_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    call_named_pipe(task, cx, a, true)
}

pub fn call_named_pipe_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    call_named_pipe(task, cx, a, false)
}
