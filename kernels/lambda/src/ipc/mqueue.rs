//! POSIX message queues: `mq_open`, `mq_unlink`, `mq_timedsend`, `mq_timedreceive`, `mq_notify` and `mq_getsetattr`.
//! A queue has a name for the machine and is open as a file: readable in `poll` and `epoll` while it holds a message,
//! writable while it has room, and `read` gives its status as Linux's mqueue file system does. Messages come out
//! highest priority first, oldest first within one. `mq_notify` tells one process, by a signal, when a message comes
//! to the queue empty and no one's waiting to take it.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Watchers, Woke};

use super::{leave, wake_all};
use crate::abi::*;
use crate::fs::{Node, Open};
use crate::process::Task;
use crate::signal::Info;

/// The most queues; the longest queue and message without privilege, and with it.
const QUEUES_MAX: usize = 256;
const MSG_MAX: u64 = 10;
const MSGSIZE_MAX: u64 = 8192;
const HARD_MSG_MAX: u64 = 65536;
const HARD_MSGSIZE_MAX: u64 = 16 << 20;
const MQ_PRIO_MAX: u64 = 32768;
/// A message queue's `si_code`.
const SI_MESGQ: i32 = -3;
const SIGEV_SIGNAL: i32 = 0;
const SIGEV_NONE: i32 = 1;
static NEXT_INO: AtomicU64 = AtomicU64::new(1);

/// Who to tell when a message comes: a process, and the signal and value, or nothing but the registration.
#[derive(Debug, Clone, Copy)]
struct Notify {
    pid: i32,
    signal: Option<(i32, u64)>,
}

#[derive(Default)]
struct State {
    /// Messages by priority (highest first), then by when they came.
    messages: BTreeMap<(Reverse<u32>, u64), Vec<u8>>,
    next: u64,
    bytes: u64,
    /// Threads waiting for a message or for room, and how many are waiting for a message.
    waiting: Vec<Arc<Waiter>>,
    receivers: usize,
    notify: Option<Notify>,
}

/// A message queue.
pub(crate) struct MQueue {
    state: std::sync::Mutex<State>,
    max_messages: u64,
    message_size: u64,
    pub mode: u32,
    pub ino: u64,
    /// Told when it gets a message or room, for `poll` and `epoll`.
    pub watchers: Watchers,
}

impl MQueue {
    /// Whether it holds a message, and whether it has room.
    pub fn ready(&self) -> (bool, bool) {
        let state = lock(&self.state);
        (!state.messages.is_empty(), (state.messages.len() as u64) < self.max_messages)
    }

    /// What `read` gives: how much it holds, and who's to be told.
    pub fn status(&self) -> Vec<u8> {
        let state = lock(&self.state);
        let (how, signal, pid) = match state.notify {
            Some(Notify { pid, signal: Some((signal, _)) }) => (SIGEV_SIGNAL, signal, pid),
            Some(Notify { pid, signal: None }) => (SIGEV_NONE, 0, pid),
            None => (0, 0, 0),
        };
        format!("QSIZE:{:<10} NOTIFY:{how:<5} SIGNO:{signal:<5} NOTIFY_PID:{pid:<6}\n", state.bytes).into_bytes()
    }
}

impl Task {
    /// A queue's name: no slash in it (the C library takes the leading one off), at most `NAME_MAX` bytes.
    fn mq_name(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<String, Errno> {
        let name = match self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 256)) {
            Ok(name) => name,
            Err(_) => return Err(if self.mem(cx, false, |t| user::read_vec(&t.space(), at, 1)).is_ok() { ENAMETOOLONG } else { EFAULT }),
        };
        let name = String::from_utf8(name).map_err(|_| EINVAL)?;
        match name.as_str() {
            "" | "." | ".." => Err(EINVAL),
            n if n.contains('/') => Err(EACCES),
            _ => Ok(name),
        }
    }

    /// `mq_open`: a queue by name, made with `O_CREAT` (and `attr_at`'s size, or 10 messages of 8 KiB).
    pub(crate) fn mq_open(&mut self, cx: &mut Cx<'_>, name_at: u64, flags: u32, mode: u32, attr_at: u64) -> Result<u64, Errno> {
        let name = self.mq_name(cx, name_at)?;
        let (umask, root) = {
            let state = lock(&self.process.state);
            (state.umask, state.creds.uid[1] == 0)
        };
        let existing = lock(&self.ipc_ns().mqueues).get(&name).cloned();
        let queue = match existing {
            Some(_) if flags & O_CREAT != 0 && flags & O_EXCL != 0 => return Err(EEXIST),
            Some(queue) => queue,
            None if flags & O_CREAT == 0 => return Err(ENOENT),
            None => {
                let (max_messages, message_size) = if attr_at == 0 {
                    (MSG_MAX, MSGSIZE_MAX)
                } else {
                    let (max, size) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), attr_at + 8)? as i64, user::read_u64(&t.space(), attr_at + 16)? as i64)))?;
                    let (most_messages, most_size) = if root { (HARD_MSG_MAX, HARD_MSGSIZE_MAX) } else { (MSG_MAX, MSGSIZE_MAX) };
                    if max <= 0 || size <= 0 || max as u64 > most_messages || size as u64 > most_size {
                        return Err(EINVAL);
                    }
                    (max as u64, size as u64)
                };
                let __ipc = self.ipc_ns();
                let mut queues = lock(&__ipc.mqueues);
                if queues.len() >= QUEUES_MAX {
                    return Err(ENOSPC);
                }
                let queue = Arc::new(MQueue {
                    state: Default::default(),
                    max_messages,
                    message_size,
                    mode: S_IFREG | mode & !umask & 0o777,
                    ino: NEXT_INO.fetch_add(1, Ordering::Relaxed),
                    watchers: Watchers::default(),
                });
                queues.entry(name.clone()).or_insert(queue).clone()
            }
        };
        let open = Open::new(Node::Mqueue(queue), flags & (O_ACCMODE | O_NONBLOCK), format!("/{name}"));
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    pub(crate) fn mq_unlink(&mut self, cx: &mut Cx<'_>, name_at: u64) -> Result<u64, Errno> {
        let name = self.mq_name(cx, name_at)?;
        lock(&self.ipc_ns().mqueues).remove(&name).map(|_| 0).ok_or(ENOENT)
    }

    /// The queue a descriptor is open on, and the file.
    fn mqueue(&self, fd: i32) -> Result<(Arc<MQueue>, Arc<Open>), Errno> {
        let open = self.file(fd)?;
        match &open.node {
            Node::Mqueue(queue) => Ok((queue.clone(), open.clone())),
            _ => Err(EBADF),
        }
    }

    /// An absolute timeout on the real-time clock, as an instant; none for no timeout.
    fn mq_deadline(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Option<Instant>, Errno> {
        if at == 0 {
            return Ok(None);
        }
        let (sec, nsec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?;
        if (sec as i64) < 0 || nsec >= 1_000_000_000 {
            return Err(EINVAL);
        }
        let at = sec.saturating_mul(1_000_000_000).saturating_add(nsec);
        Ok(Some(Instant::now() + Duration::from_nanos(at.saturating_sub(self.k.host.wall()))))
    }

    /// Waits on a queue until woken or `deadline`: ETIMEDOUT, or EINTR for a signal.
    fn mq_wait(&self, queue: &MQueue, deadline: Option<Instant>) -> Result<(), Errno> {
        let woke = self.waiter.wait(deadline);
        if woke != Woke::Woken {
            leave(&mut lock(&queue.state).waiting, &self.waiter);
        }
        match woke {
            Woke::Woken => Ok(()),
            Woke::Interrupted => Err(EINTR),
            Woke::TimedOut => Err(ETIMEDOUT),
        }
    }

    pub(crate) fn mq_timedsend(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len: u64, priority: u64, timeout_at: u64) -> Result<u64, Errno> {
        let (queue, open) = self.mqueue(fd)?;
        if *lock(&open.flags) & O_ACCMODE == 0 {
            return Err(EBADF);
        }
        if priority >= MQ_PRIO_MAX {
            return Err(EINVAL);
        }
        if len > queue.message_size {
            return Err(EMSGSIZE);
        }
        let message = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize))?;
        let mut deadline = None;
        loop {
            let notify = {
                let mut state = lock(&queue.state);
                if (state.messages.len() as u64) < queue.max_messages {
                    // Coming to it empty, with no one waiting to take it: whoever asked is told, once.
                    let notify = if state.messages.is_empty() && state.receivers == 0 { state.notify.take() } else { None };
                    let key = (Reverse(priority as u32), state.next);
                    state.next += 1;
                    state.bytes += len;
                    state.messages.insert(key, message);
                    wake_all(&mut state.waiting);
                    notify
                } else {
                    if open.nonblock() {
                        return Err(EAGAIN);
                    }
                    if deadline.is_none() {
                        deadline = Some(self.mq_deadline(cx, timeout_at)?);
                    }
                    self.waiter.reset();
                    state.waiting.push(self.waiter.clone());
                    drop(state);
                    self.mq_wait(&queue, deadline.flatten())?;
                    continue;
                }
            };
            queue.watchers.notify();
            if let Some(Notify { pid, signal: Some((signal, value)) }) = notify
                && let Some(process) = self.k.process(pid)
            {
                let uid = lock(&self.process.state).creds.uid[0];
                self.k.send(&process, None, signal, Info { code: SI_MESGQ, pid: self.process.pid, uid, value, ..Info::default() });
            }
            return Ok(0);
        }
    }

    pub(crate) fn mq_timedreceive(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len: u64, priority_at: u64, timeout_at: u64) -> Result<u64, Errno> {
        let (queue, open) = self.mqueue(fd)?;
        if *lock(&open.flags) & O_ACCMODE == O_WRONLY {
            return Err(EBADF);
        }
        if len < queue.message_size {
            return Err(EMSGSIZE);
        }
        let mut deadline = None;
        let ((Reverse(priority), _), message) = loop {
            let mut state = lock(&queue.state);
            if let Some(first) = state.messages.pop_first() {
                state.bytes -= first.1.len() as u64;
                wake_all(&mut state.waiting);
                break first;
            }
            if open.nonblock() {
                return Err(EAGAIN);
            }
            if deadline.is_none() {
                deadline = Some(self.mq_deadline(cx, timeout_at)?);
            }
            self.waiter.reset();
            state.waiting.push(self.waiter.clone());
            state.receivers += 1;
            drop(state);
            let woke = self.mq_wait(&queue, deadline.flatten());
            lock(&queue.state).receivers -= 1;
            woke?;
        };
        queue.watchers.notify();
        self.mem(cx, true, |t| {
            user::write(&mut t.space(), at, &message)?;
            if priority_at != 0 {
                user::write_u32(&mut t.space(), priority_at, priority)?;
            }
            Ok(())
        })?;
        Ok(message.len() as u64)
    }

    /// `mq_notify`: this process is to be told (by `sevp_at`'s signal, or not at all) when a message comes, if no
    /// other is; none says it's not to be any more.
    pub(crate) fn mq_notify(&mut self, cx: &mut Cx<'_>, fd: i32, sevp_at: u64) -> Result<u64, Errno> {
        let (queue, _) = self.mqueue(fd)?;
        let pid = self.process.pid;
        if sevp_at == 0 {
            let mut state = lock(&queue.state);
            if state.notify.is_some_and(|n| n.pid == pid) {
                state.notify = None;
            }
            return Ok(0);
        }
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), sevp_at, 16))?;
        let (value, signal, how) = (u64::from_le_bytes(b[0..8].try_into().unwrap_or_default()), i32::from_le_bytes(b[8..12].try_into().unwrap_or_default()), i32::from_le_bytes(b[12..16].try_into().unwrap_or_default()));
        let signal = match how {
            SIGEV_NONE => None,
            SIGEV_SIGNAL if (1..=64).contains(&signal) => Some((signal, value)),
            // SIGEV_THREAD's netlink socket isn't here.
            _ => return Err(EINVAL),
        };
        let mut state = lock(&queue.state);
        // A registration ends with its process.
        if let Some(n) = state.notify
            && self.k.process(n.pid).is_some()
        {
            return Err(EBUSY);
        }
        state.notify = Some(Notify { pid, signal });
        Ok(0)
    }

    /// `mq_getsetattr`: the queue's sizes and how many it holds, and whether this open file waits; only that
    /// (`O_NONBLOCK`) can be changed.
    pub(crate) fn mq_getsetattr(&mut self, cx: &mut Cx<'_>, fd: i32, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        let (queue, open) = self.mqueue(fd)?;
        let new_flags = if new_at == 0 { None } else { Some(self.mem(cx, false, |t| user::read_u64(&t.space(), new_at))? as u32) };
        if old_at != 0 {
            let count = lock(&queue.state).messages.len() as u64;
            let flags = u64::from(*lock(&open.flags) & O_NONBLOCK);
            let mut b: Vec<u8> = [flags, queue.max_messages, queue.message_size, count].iter().flat_map(|w| w.to_le_bytes()).collect();
            b.resize(64, 0);
            self.mem(cx, true, |t| user::write(&mut t.space(), old_at, &b))?;
        }
        if let Some(new) = new_flags {
            let mut flags = lock(&open.flags);
            *flags = *flags & !O_NONBLOCK | new & O_NONBLOCK;
        }
        Ok(0)
    }
}
