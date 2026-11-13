//! kernel32's asynchronous I/O, in host Rust over `ports`: completion ports (`CreateIoCompletionPort`,
//! `PostQueuedCompletionStatus`, `GetQueuedCompletionStatus(Ex)`), an I/O's result (`GetOverlappedResult(Ex)`),
//! cancelling, and `SetFileCompletionNotificationModes`. The I/O itself, and its completion, is `fileapi`'s.
//!
//! I/O on the disk completes as it's asked for, as Windows' cached I/O usually does: the call succeeds, and its
//! completion still reaches the OVERLAPPED, its event and the file's port. I/O that stays pending comes with named
//! pipes and sockets.

use std::sync::Arc;
use std::time::{Duration, Instant};

use kernels_core::thread::Cx;
use kernels_core::{lock, user};

use super::Reply;
use crate::files::{ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER};
use crate::handles::Handle;
use crate::objects::INFINITE;
use crate::ports::{self, Empty, Packet, Port};
use crate::process::Task;

const TRUE: u64 = 1;
const ERROR_NOT_FOUND: u32 = 1168;

impl Task {
    fn port_handle(&self, handle: u64) -> Option<Arc<Port>> {
        match lock(&self.process.state).handles.get(handle)? {
            Handle::Port(port) => Some(port),
            _ => None,
        }
    }
}

fn deadline(timeout_ms: u32) -> Option<Instant> {
    (timeout_ms != INFINITE).then(|| Instant::now() + Duration::from_millis(u64::from(timeout_ms)))
}

/// `CreateIoCompletionPort(file, port, key, threads)`: a new port (with no file), or a file associated with a port
/// (a new one, or the one given) under a key. Gives the port's handle.
pub fn create_io_completion_port(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let [file, existing, key] = [a[0], a[1], a[2]];
    let port = match existing {
        0 => None,
        h => match task.port_handle(h) {
            Some(port) => Some(port),
            None => {
                task.set_last_error(cx, ERROR_INVALID_HANDLE);
                return Reply::Value(0);
            }
        },
    };
    if file == u64::MAX {
        if port.is_some() {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Reply::Value(0);
        }
        return Reply::Value(lock(&task.process.state).handles.add(Handle::Port(Arc::default())));
    }
    let handle = lock(&task.process.state).handles.get(file);
    let Some(Handle::File(open)) = handle else {
        task.set_last_error(cx, ERROR_INVALID_HANDLE);
        return Reply::Value(0);
    };
    let mut associated = lock(&open.port);
    if associated.is_some() {
        drop(associated);
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Reply::Value(0);
    }
    let (port, handle) = match port {
        Some(port) => (port, existing),
        None => {
            let port: Arc<Port> = Arc::default();
            let handle = lock(&task.process.state).handles.add(Handle::Port(port.clone()));
            (port, handle)
        }
    };
    *associated = Some((port, key));
    Reply::Value(handle)
}

/// `PostQueuedCompletionStatus(port, bytes, key, overlapped)`.
pub fn post_queued_completion_status(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(port) = task.port_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    port.post(Packet { bytes: a[1] as u32, key: a[2], overlapped: a[3], status: 0 });
    Reply::Value(TRUE)
}

/// `GetQueuedCompletionStatus(port, &bytes, &key, &overlapped, timeout)`: TRUE for a packet of an I/O that
/// succeeded (or a posted one); FALSE with the OVERLAPPED set for one that failed, and with it NULL if none came.
pub fn get_queued_completion_status(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(port) = task.port_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let (bytes_out, key_out, overlapped_out) = (a[1], a[2], a[3]);
    match task.dequeue(cx, &port, 1, deadline(a[4] as u32), false) {
        Ok(packets) => {
            let p = packets[0];
            let abi = task.abi();
            let _ = task.mem(cx, true, |t| {
                user::write_u32(&mut t.space(), bytes_out, p.bytes)?;
                user::write(&mut t.space(), key_out, &abi.ptr(p.key))?;
                user::write(&mut t.space(), overlapped_out, &abi.ptr(p.overlapped))
            });
            if p.status != 0 { task.fail(cx, ports::error(p.status)) } else { Reply::Value(TRUE) }
        }
        Err(Empty::Ended) => Reply::Ended,
        Err(_) => {
            let _ = task.write_ptr(cx, overlapped_out, 0);
            task.fail(cx, ports::WAIT_TIMEOUT)
        }
    }
}

/// `GetQueuedCompletionStatusEx(port, entries, count, &removed, timeout, alertable)`: up to `count` packets, as
/// OVERLAPPED_ENTRY records (key, OVERLAPPED, status, bytes).
pub fn get_queued_completion_status_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(port) = task.port_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    let (entries, count, removed_out) = (a[1], a[2] as u32 as usize, a[3]);
    if count == 0 {
        return task.fail(cx, ERROR_INVALID_PARAMETER);
    }
    match task.dequeue(cx, &port, count, deadline(a[4] as u32), a[5] as u32 != 0) {
        Ok(packets) => {
            // OVERLAPPED_ENTRY: three pointer-sized fields and a DWORD, padded to a pointer's size.
            let abi = task.abi();
            let mut b = Vec::new();
            for p in &packets {
                b.extend(abi.ptr(p.key));
                b.extend(abi.ptr(p.overlapped));
                b.extend(abi.ptr(u64::from(p.status)));
                b.extend(p.bytes.to_le_bytes());
                b.resize(b.len().next_multiple_of(abi.width as usize), 0);
            }
            let n = packets.len() as u32;
            let _ = task.mem(cx, true, |t| {
                user::write(&mut t.space(), entries, &b)?;
                user::write_u32(&mut t.space(), removed_out, n)
            });
            Reply::Value(TRUE)
        }
        Err(Empty::Ended) => Reply::Ended,
        Err(Empty::Alerted) => {
            let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), removed_out, 0));
            task.fail(cx, ports::IO_COMPLETION)
        }
        Err(Empty::TimedOut) => {
            let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), removed_out, 0));
            task.fail(cx, ports::WAIT_TIMEOUT)
        }
    }
}

/// `GetOverlappedResultEx(file, overlapped, &bytes, timeout, alertable)`: what an I/O did, waiting for it on its event
/// if it's still under way and the caller will wait.
pub fn get_overlapped_result_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let k = task.k.clone();
    k.drain(cx);
    let (ov, bytes_out, timeout, alertable) = (a[1], a[2], a[3] as u32, a[4] as u32 != 0);
    let abi = task.abi();
    let read = |task: &mut Task, cx: &mut Cx<'_>| {
        task.mem(cx, false, |t| Ok((user::read_ptr(&t.space(), ov, abi.width)?, user::read_ptr(&t.space(), ov + abi.width, abi.width)?, user::read_ptr(&t.space(), ov + abi.overlapped_event(), abi.width)?))).ok()
    };
    let Some((mut status, mut bytes, event)) = read(task, cx) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
    if status as u32 == ports::STATUS_PENDING {
        if timeout == 0 {
            return task.fail(cx, ports::ERROR_IO_INCOMPLETE);
        }
        let waited = match task.object(if event != 0 { event } else { a[0] }) {
            Some(object) => task.wait_for(cx, &[object], false, timeout, alertable),
            None => return task.fail(cx, ERROR_INVALID_HANDLE),
        };
        match waited {
            Ok(0) => {}
            Ok(code) => return task.fail(cx, code),
            Err(reply) => return reply,
        }
        let Some(now) = read(task, cx) else { return task.fail(cx, ERROR_INVALID_PARAMETER) };
        (status, bytes) = (now.0, now.1);
    }
    let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), bytes_out, bytes as u32));
    if status != 0 { task.fail(cx, ports::error(status as u32)) } else { Reply::Value(TRUE) }
}

/// `GetOverlappedResult(file, overlapped, &bytes, wait)`.
pub fn get_overlapped_result(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let timeout = if a[3] as u32 != 0 { INFINITE } else { 0 };
    get_overlapped_result_ex(task, cx, &[a[0], a[1], a[2], u64::from(timeout), 0])
}

/// Cancels this process's pending I/O on a handle (just the one at `overlapped`, if it's given): each completes with
/// ERROR_OPERATION_ABORTED. How many there were.
fn cancel(task: &mut Task, cx: &mut Cx<'_>, handle: u64, overlapped: Option<u64>) -> Result<usize, u32> {
    let file = task.file_handle(handle).ok_or(ERROR_INVALID_HANDLE)?;
    let mut done = Vec::new();
    if let crate::files::Node::NamedPipe(end) = &file.node {
        if let Ok(pipe) = end.incoming() {
            done.extend(pipe.cancel(&task.process, overlapped));
        }
        let mut s = lock(&end.instance.state);
        let mine = s.connect.as_ref().is_some_and(|op| op.process.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &task.process)) && overlapped.is_none_or(|o| o == op.overlapped));
        if mine && let Some(op) = s.connect.take() {
            done.push(ports::Completion { op, data: Vec::new(), error: ports::ERROR_OPERATION_ABORTED });
        }
    }
    let n = done.len();
    for c in done {
        let k = task.k.clone();
        k.complete(cx, c);
    }
    Ok(n)
}

/// `CancelIo(file)`: this process's I/O under way on it.
pub fn cancel_io(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match cancel(task, cx, a[0], None) {
        Ok(_) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

/// `CancelIoEx(file, overlapped)`: the one I/O at `overlapped`, or all of them; ERROR_NOT_FOUND if there were none.
pub fn cancel_io_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match cancel(task, cx, a[0], (a[1] != 0).then_some(a[1])) {
        Ok(0) => task.fail(cx, ERROR_NOT_FOUND),
        Ok(_) => Reply::Value(TRUE),
        Err(e) => task.fail(cx, e),
    }
}

/// `CancelSynchronousIo(thread)`: a thread's I/O that waits can't be cancelled yet, so there's none found.
pub fn cancel_synchronous_io(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.fail(cx, ERROR_NOT_FOUND)
}

/// `SetFileCompletionNotificationModes(file, flags)`: `FILE_SKIP_COMPLETION_PORT_ON_SUCCESS` (1) and
/// `FILE_SKIP_SET_EVENT_ON_HANDLE` (2).
pub fn set_file_completion_notification_modes(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(file) = task.file_handle(a[0]) else { return task.fail(cx, ERROR_INVALID_HANDLE) };
    file.modes.store(a[1] as u32 & 3, std::sync::atomic::Ordering::Relaxed);
    Reply::Value(TRUE)
}
