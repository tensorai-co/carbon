//! pidfds: a descriptor for a process (`pidfd_open`, or `clone`'s `CLONE_PIDFD`), readable in `poll` and `epoll`
//! once it's ended, that signals it (`pidfd_send_signal`), copies its open files (`pidfd_getfd`) and waits for it
//! (`waitid`'s `P_PIDFD`) without the races of a pid that may have been reused.

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::thread::Cx;

use super::{Process, Task};
use crate::abi::*;
use crate::fs::{Node, Open};
use crate::signal::Info;

/// `pidfd_open`'s flags: a pidfd that doesn't wait (`O_NONBLOCK`), and one for a thread rather than a process
/// (`O_EXCL`).
const PIDFD_NONBLOCK: u64 = O_NONBLOCK as u64;
const PIDFD_THREAD: u64 = O_EXCL as u64;
// `pidfd_send_signal`'s flags: to the thread, the process, or its process group.
const PIDFD_SIGNAL_THREAD: u64 = 1;
const PIDFD_SIGNAL_THREAD_GROUP: u64 = 2;
const PIDFD_SIGNAL_PROCESS_GROUP: u64 = 4;

impl Task {
    /// A new pidfd for `process`, closed at `execve`.
    pub(crate) fn add_pidfd(&self, process: Arc<Process>, nonblock: bool) -> Result<i32, Errno> {
        let open = Open::new(Node::Pidfd(process), O_RDWR | if nonblock { O_NONBLOCK } else { 0 }, "anon_inode:[pidfd]");
        lock(&self.process.state).files.add(open, true, 0)
    }

    /// The process a pidfd is for.
    pub(crate) fn pidfd_process(&self, fd: i32) -> Result<(Arc<Process>, bool), Errno> {
        let open = self.file(fd)?;
        match &open.node {
            Node::Pidfd(process) => Ok((process.clone(), open.nonblock())),
            _ => Err(EBADF),
        }
    }

    pub(crate) fn pidfd_open(&mut self, pid: i32, flags: u64) -> Result<u64, Errno> {
        if flags & !(PIDFD_NONBLOCK | PIDFD_THREAD) != 0 || pid <= 0 {
            return Err(EINVAL);
        }
        let process = self.process_of(pid).ok_or(ESRCH)?;
        // A thread's ID names its process only with PIDFD_THREAD.
        if process.pid != pid && flags & PIDFD_THREAD == 0 {
            return Err(ENOENT);
        }
        self.add_pidfd(process, flags & PIDFD_NONBLOCK != 0).map(|fd| fd as u64)
    }

    /// `pidfd_send_signal`: like `kill`, or with a `siginfo` like `rt_sigqueueinfo`.
    pub(crate) fn pidfd_send_signal(&mut self, cx: &mut Cx<'_>, fd: i32, signal: i32, info_at: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(PIDFD_SIGNAL_THREAD | PIDFD_SIGNAL_THREAD_GROUP | PIDFD_SIGNAL_PROCESS_GROUP) != 0 || flags.count_ones() > 1 || !(0..=64).contains(&signal) {
            return Err(EINVAL);
        }
        let (process, _) = self.pidfd_process(fd)?;
        if lock(&process.state).zombie.is_some() {
            return Err(ESRCH);
        }
        if flags == PIDFD_SIGNAL_PROCESS_GROUP {
            let pgid = lock(&process.state).pgid;
            self.k.signal_group(pgid, signal);
            return Ok(0);
        }
        let info = match info_at {
            0 => Info { uid: lock(&self.process.state).creds.uid[0], ..Info::user(self.process.pid) },
            at => self.read_siginfo(cx, at)?,
        };
        let tid = (flags == PIDFD_SIGNAL_THREAD).then_some(process.pid);
        if info_at == 0 {
            self.k.send(&process, tid, signal, info);
            return Ok(0);
        }
        self.queue_to(&process, tid, signal, info)
    }

    /// `pidfd_getfd`: a copy of another process's open file, closed at `execve`.
    pub(crate) fn pidfd_getfd(&mut self, fd: i32, target: i32, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let (process, _) = self.pidfd_process(fd)?;
        let open = {
            let state = lock(&process.state);
            if state.zombie.is_some() || state.exiting.is_some() {
                return Err(ESRCH);
            }
            state.files.get(target).ok_or(EBADF)?
        };
        lock(&self.process.state).files.add(open, true, 0).map(|fd| fd as u64)
    }
}
