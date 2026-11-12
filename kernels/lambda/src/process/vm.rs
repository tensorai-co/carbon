//! `process_vm_readv` and `process_vm_writev`: copying between this process's memory and another's, without
//! stopping it, for debuggers, strace's string reads, and checkpointing. The other's pages are filled as its own
//! touch would fill them (`ptrace`'s way in), and a copy stops at the first byte that can't be read or written.

use super::Task;
use crate::abi::*;
use kernels_core::thread::Cx;
use kernels_core::user;

/// What one copy moves at most at a time.
const CHUNK: u64 = 1 << 20;

impl Task {
    /// `process_vm_readv` (or with `write`, `process_vm_writev`): from `remote_iov`'s ranges of `pid`'s memory to
    /// `local_iov`'s of this process's, or back, in order, as far as both go. How many bytes moved; EFAULT if the
    /// first byte couldn't be, ESRCH for no such process, EPERM if this process may not trace it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process_vm(&mut self, cx: &mut Cx<'_>, write: bool, pid: i32, local_iov: u64, local_count: u64, remote_iov: u64, remote_count: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let local = self.iovecs(cx, local_iov, local_count)?;
        let remote = self.iovecs(cx, remote_iov, remote_count)?;
        if pid <= 0 {
            return Err(ESRCH);
        }
        let process = self.process_of(pid).ok_or(ESRCH)?;
        if !self.may_trace(&process) {
            return Err(EPERM);
        }
        let (mut l, mut r) = (local.into_iter().filter(|v| v.1 > 0), remote.into_iter().filter(|v| v.1 > 0));
        let (mut here, mut there) = (l.next(), r.next());
        let mut moved = 0u64;
        while let (Some((l_at, l_len)), Some((r_at, r_len))) = (here, there) {
            let len = l_len.min(r_len).min(CHUNK);
            let done = if write {
                let Ok(data) = self.mem(cx, false, |t| user::read_vec(&t.space(), l_at, len as usize)) else { break };
                self.write_remote(cx, &process, r_at, &data, false).unwrap_or(0) as u64
            } else {
                let data = self.peek(cx, &process, r_at, len as usize).unwrap_or_default();
                if self.mem(cx, true, |t| user::write(&mut t.space(), l_at, &data)).is_err() {
                    break;
                }
                data.len() as u64
            };
            moved += done;
            if done < len {
                break;
            }
            here = if len == l_len { l.next() } else { Some((l_at + len, l_len - len)) };
            there = if len == r_len { r.next() } else { Some((r_at + len, r_len - len)) };
        }
        let asked = here.is_some() && there.is_some();
        if moved == 0 && asked { Err(EFAULT) } else { Ok(moved) }
    }
}
