//! Inter-process communication: SysV message queues (`msg`) and semaphores (`sem`), and POSIX message queues
//! (`mqueue`). SysV shared memory is with the rest of memory (`mm::shared`).
//!
//! Each SysV kind is one table for the machine, by ID, with a key that names an object to every process that knows it.
//! A process that has to wait (for room, a message, a semaphore) waits on the table, and anything that changes it wakes
//! every waiter to look again: simple, and with queues and semaphore sets this small, cheap.

pub(crate) mod mqueue;
mod msg;
mod sem;

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::wait::Waiter;

pub(crate) use msg::MsgTable;
pub(crate) use sem::{SemTable, apply_undo};

use crate::process::Task;

const IPC_PRIVATE: i32 = 0;
const IPC_CREAT: u64 = 0o1000;
const IPC_EXCL: u64 = 0o2000;
const IPC_NOWAIT: u64 = 0o4000;
const IPC_RMID: u64 = 0;
const IPC_SET: u64 = 1;
const IPC_STAT: u64 = 2;
const IPC_INFO: u64 = 3;
/// Or'd into a command by some C libraries, for the 64-bit structures, which are the only ones here.
const IPC_64: u64 = 0x100;
/// `msgrcv`'s and `semop`'s "the object went while waiting".
const EIDRM: crate::abi::Errno = crate::abi::Errno(43);

/// An IPC object's key, owner, creator and permissions, as `struct ipc64_perm` has them.
#[derive(Debug, Clone, Copy)]
struct Perm {
    key: i32,
    owner: (u32, u32),
    creator: (u32, u32),
    mode: u32,
}

impl Perm {
    fn new(task: &Task, key: i32, flags: u64) -> Perm {
        let (uid, gid) = {
            let state = lock(&task.process.state);
            (state.creds.uid[1], state.creds.gid[1])
        };
        Perm { key, owner: (uid, gid), creator: (uid, gid), mode: flags as u32 & 0o777 }
    }

    /// `struct ipc64_perm`: key, uid, gid, cuid, cgid, mode, then the sequence number and padding.
    fn encode(&self) -> [u8; 48] {
        let mut b = [0u8; 48];
        for (i, v) in [self.key as u32, self.owner.0, self.owner.1, self.creator.0, self.creator.1, self.mode].into_iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// `IPC_SET`: a new owner, and permission bits, from an `ipc64_perm`.
    fn set(&mut self, b: &[u8]) {
        let word = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or_default());
        self.owner = (word(1), word(2));
        self.mode = self.mode & !0o777 | word(5) & 0o777;
    }

    /// Whether `task` may change or remove the object: root, its owner or its creator.
    fn may_change(&self, task: &Task) -> bool {
        let euid = lock(&task.process.state).creds.uid[1];
        euid == 0 || euid == self.owner.0 || euid == self.creator.0
    }
}

/// The time now, in seconds, for an object's times.
fn seconds(task: &Task) -> u64 {
    task.k.host.wall() / 1_000_000_000
}

/// Takes a thread that stopped waiting without being woken (a signal, a timeout) off a waiting list, so a wake meant
/// for this list can't end a later, unrelated wait of its.
fn leave(waiting: &mut Vec<Arc<Waiter>>, me: &Arc<Waiter>) {
    waiting.retain(|w| !Arc::ptr_eq(w, me));
}

/// Wakes every thread waiting on a table, to look again.
fn wake_all(waiting: &mut Vec<Arc<Waiter>>) {
    for waiter in waiting.drain(..) {
        waiter.wake();
    }
}
