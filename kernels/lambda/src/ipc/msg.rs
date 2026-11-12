//! SysV message queues: `msgget`, `msgsnd`, `msgrcv` and `msgctl`. A queue holds typed messages up to its byte limit
//! (`msg_qbytes`); a receiver takes the first, the first of a type, the first not of a type, or the lowest type up to
//! one, and `MSG_COPY` looks at one without taking it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};

use super::*;
use crate::abi::*;

/// The biggest message, a queue's default byte limit, and the most queues, as Linux's defaults.
const MSGMAX: u64 = 8192;
const MSGMNB: u64 = 16384;
const MSGMNI: usize = 32000;
const MSG_STAT: u64 = 11;
const MSG_INFO: u64 = 12;
const MSG_STAT_ANY: u64 = 13;
const MSG_NOERROR: u64 = 0o10000;
const MSG_EXCEPT: u64 = 0o20000;
const MSG_COPY: u64 = 0o40000;
const ENOMSG: Errno = Errno(42);

struct Queue {
    perm: Perm,
    /// Each message's type and text, oldest first.
    messages: VecDeque<(i64, Vec<u8>)>,
    bytes: u64,
    /// The most bytes it holds (`msg_qbytes`).
    limit: u64,
    /// Last sent, received and changed, in seconds.
    times: [u64; 3],
    /// Who last sent and received.
    pids: (i32, i32),
}

/// The machine's message queues, by ID.
#[derive(Default)]
pub(crate) struct MsgTable {
    queues: BTreeMap<i32, Queue>,
    next: i32,
    waiting: Vec<Arc<Waiter>>,
}

impl Task {
    pub(crate) fn msgget(&mut self, key: i32, flags: u64) -> Result<u64, Errno> {
        let perm = Perm::new(self, key, flags);
        let now = seconds(self);
        let __ipc = self.ipc_ns();
        let mut table = lock(&__ipc.msg);
        if key != IPC_PRIVATE
            && let Some(id) = table.queues.iter().find(|(_, q)| q.perm.key == key).map(|(id, _)| *id)
        {
            return if flags & IPC_CREAT != 0 && flags & IPC_EXCL != 0 { Err(EEXIST) } else { Ok(id as u64) };
        }
        if key != IPC_PRIVATE && flags & IPC_CREAT == 0 {
            return Err(ENOENT);
        }
        if table.queues.len() >= MSGMNI {
            return Err(ENOSPC);
        }
        let id = table.next;
        table.next += 1;
        table.queues.insert(id, Queue { perm, messages: VecDeque::new(), bytes: 0, limit: MSGMNB, times: [0, 0, now], pids: (0, 0) });
        Ok(id as u64)
    }

    /// Waits for the table to change; `Err` if a signal or the process ending comes first.
    fn wait_msg(&self) -> Result<(), Errno> {
        if self.waiter.wait(None) == Woke::Interrupted {
            leave(&mut lock(&self.ipc_ns().msg).waiting, &self.waiter);
            return Err(EINTR);
        }
        Ok(())
    }

    /// `msgsnd`: a message of a type (above 0) and `size` bytes, waiting for room unless `IPC_NOWAIT`.
    pub(crate) fn msgsnd(&mut self, cx: &mut Cx<'_>, id: i32, at: u64, size: u64, flags: u64) -> Result<u64, Errno> {
        if size > MSGMAX {
            return Err(EINVAL);
        }
        let (kind, text) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)? as i64, user::read_vec(&t.space(), at + 8, size as usize)?)))?;
        if kind <= 0 {
            return Err(EINVAL);
        }
        let mut waited = false;
        loop {
            {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.msg);
                let now = seconds(self);
                let Some(queue) = table.queues.get_mut(&id) else { return Err(if waited { EIDRM } else { EINVAL }) };
                if queue.bytes + size <= queue.limit && (queue.messages.len() as u64) < queue.limit {
                    queue.messages.push_back((kind, text));
                    queue.bytes += size;
                    queue.times[0] = now;
                    queue.pids.0 = self.process.pid;
                    wake_all(&mut table.waiting);
                    return Ok(0);
                }
                if flags & IPC_NOWAIT != 0 {
                    return Err(EAGAIN);
                }
                self.waiter.reset();
                table.waiting.push(self.waiter.clone());
            }
            self.wait_msg()?;
            waited = true;
        }
    }

    /// `msgrcv`: takes a message, as `kind` picks it, into a buffer of `size` bytes (a longer one is E2BIG, or cut with
    /// `MSG_NOERROR`), waiting for one unless `IPC_NOWAIT`. Gives its length.
    pub(crate) fn msgrcv(&mut self, cx: &mut Cx<'_>, id: i32, at: u64, size: u64, kind: i64, flags: u64) -> Result<u64, Errno> {
        let copy = flags & MSG_COPY != 0;
        if (size as i64) < 0 || (copy && (flags & IPC_NOWAIT == 0 || flags & MSG_EXCEPT != 0 || kind < 0)) {
            return Err(EINVAL);
        }
        let mut waited = false;
        let (got_kind, text) = loop {
            {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.msg);
                let now = seconds(self);
                let Some(queue) = table.queues.get_mut(&id) else { return Err(if waited { EIDRM } else { EINVAL }) };
                let index = if copy {
                    ((kind as usize) < queue.messages.len()).then_some(kind as usize)
                } else if kind == 0 {
                    (!queue.messages.is_empty()).then_some(0)
                } else if kind > 0 && flags & MSG_EXCEPT == 0 {
                    queue.messages.iter().position(|(t, _)| *t == kind)
                } else if kind > 0 {
                    queue.messages.iter().position(|(t, _)| *t != kind)
                } else {
                    // The lowest type up to -kind, the oldest of them.
                    queue.messages.iter().enumerate().filter(|(_, (t, _))| *t <= -kind).min_by_key(|(i, (t, _))| (*t, *i)).map(|(i, _)| i)
                };
                if let Some(index) = index {
                    if queue.messages[index].1.len() as u64 > size && flags & MSG_NOERROR == 0 {
                        return Err(E2BIG);
                    }
                    if copy {
                        break queue.messages[index].clone();
                    }
                    let message = queue.messages.remove(index).unwrap_or_default();
                    queue.bytes -= message.1.len() as u64;
                    queue.times[1] = now;
                    queue.pids.1 = self.process.pid;
                    wake_all(&mut table.waiting);
                    break message;
                }
                if flags & IPC_NOWAIT != 0 {
                    return Err(ENOMSG);
                }
                self.waiter.reset();
                table.waiting.push(self.waiter.clone());
            }
            self.wait_msg()?;
            waited = true;
        };
        let n = text.len().min(size as usize);
        self.mem(cx, true, |t| {
            user::write_u64(&mut t.space(), at, got_kind as u64)?;
            user::write(&mut t.space(), at + 8, &text[..n])
        })?;
        Ok(n as u64)
    }

    pub(crate) fn msgctl(&mut self, cx: &mut Cx<'_>, id: i32, cmd: u64, at: u64) -> Result<u64, Errno> {
        let cmd = cmd & !IPC_64;
        let now = seconds(self);
        match cmd {
            IPC_INFO | MSG_INFO => {
                let (highest, bytes) = {
                    let __ipc = self.ipc_ns();
                    let table = lock(&__ipc.msg);
                    let highest = table.queues.keys().next_back().copied().unwrap_or(0);
                    // struct msginfo: msgpool, msgmap, msgmax, msgmnb, msgmni, msgssz, msgtql (ints), msgseg.
                    let words: [u32; 7] = if cmd == IPC_INFO {
                        [(MSGMNI as u64 * MSGMNB / 1024) as u32, MSGMNB as u32, MSGMAX as u32, MSGMNB as u32, MSGMNI as u32, 16, MSGMNB as u32]
                    } else {
                        let messages: usize = table.queues.values().map(|q| q.messages.len()).sum();
                        let bytes: u64 = table.queues.values().map(|q| q.bytes).sum();
                        [table.queues.len() as u32, messages as u32, MSGMAX as u32, MSGMNB as u32, MSGMNI as u32, 16, bytes as u32]
                    };
                    let mut b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                    b.extend(0xffffu16.to_le_bytes());
                    b.resize(32, 0);
                    (highest, b)
                };
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
                Ok(highest as u64)
            }
            IPC_STAT | MSG_STAT | MSG_STAT_ANY => {
                let bytes = {
                    let __ipc = self.ipc_ns();
                    let table = lock(&__ipc.msg);
                    let q = table.queues.get(&id).ok_or(EINVAL)?;
                    // struct msqid64_ds: ipc64_perm, the three times, bytes, messages and limit, the two pids.
                    let mut b = q.perm.encode().to_vec();
                    for v in [q.times[0], q.times[1], q.times[2], q.bytes, q.messages.len() as u64, q.limit] {
                        b.extend(v.to_le_bytes());
                    }
                    b.extend(q.pids.0.to_le_bytes());
                    b.extend(q.pids.1.to_le_bytes());
                    b.resize(120, 0);
                    b
                };
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
                Ok(if cmd == IPC_STAT { 0 } else { id as u64 })
            }
            IPC_SET => {
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, 96))?;
                let limit = u64::from_le_bytes(b[88..96].try_into().unwrap_or_default());
                let root = lock(&self.process.state).creds.uid[1] == 0;
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.msg);
                let q = table.queues.get_mut(&id).ok_or(EINVAL)?;
                if !q.perm.may_change(self) || (limit > MSGMNB && !root) {
                    return Err(EPERM);
                }
                q.perm.set(&b);
                q.limit = limit;
                q.times[2] = now;
                wake_all(&mut table.waiting);
                Ok(0)
            }
            IPC_RMID => {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.msg);
                let q = table.queues.get(&id).ok_or(EINVAL)?;
                if !q.perm.may_change(self) {
                    return Err(EPERM);
                }
                table.queues.remove(&id);
                wake_all(&mut table.waiting);
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }
}
