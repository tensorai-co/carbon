//! Robust futexes (`set_robust_list`, `get_robust_list`), and what Linux does with a thread's list when it ends: each
//! futex it still holds is marked `FUTEX_OWNER_DIED` and one waiter woken, so a robust mutex whose owner died goes to
//! the next thread with `EOWNERDEAD` rather than being held forever. Priority-inheritance futexes aren't supported,
//! so a list's entries are all plain ones.

use std::sync::atomic::Ordering;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Task;
use crate::abi::*;

/// `struct robust_list_head`: the list, the futex's offset from each entry, and the entry being taken or let go.
const HEAD_SIZE: u64 = 24;
/// The most entries walked, as on Linux, so a list that loops ends.
const LIST_LIMIT: usize = 2048;
const FUTEX_WAITERS: u32 = 0x8000_0000;
const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
const FUTEX_TID_MASK: u32 = 0x3fff_ffff;

impl Task {
    pub(crate) fn set_robust_list(&mut self, head: u64, len: u64) -> Result<u64, Errno> {
        if len != HEAD_SIZE {
            return Err(EINVAL);
        }
        self.robust.store(head, Ordering::Release);
        Ok(0)
    }

    /// `get_robust_list`: this thread's list, or another's by its ID.
    pub(crate) fn get_robust_list(&mut self, cx: &mut Cx<'_>, tid: i32, head_at: u64, len_at: u64) -> Result<u64, Errno> {
        let head = if tid == 0 || tid == self.tid {
            self.robust.load(Ordering::Acquire)
        } else {
            lock(&self.k.processes).values().find_map(|p| lock(&p.state).threads.get(&tid).map(|live| live.robust.load(Ordering::Acquire))).ok_or(ESRCH)?
        };
        self.mem(cx, true, |t| {
            user::write_u64(&mut t.space(), head_at, head)?;
            user::write_u64(&mut t.space(), len_at, HEAD_SIZE)
        })?;
        Ok(0)
    }

    /// This thread is ending: every robust futex it holds is its owner's no more.
    pub(crate) fn release_robust(&mut self, cx: &mut Cx<'_>) {
        let head = self.robust.swap(0, Ordering::AcqRel);
        if head == 0 {
            return;
        }
        let Ok((first, offset, pending)) = self.mem(cx, false, |t| {
            let space = t.space();
            Ok((user::read_u64(&space, head)?, user::read_u64(&space, head + 8)? as i64, user::read_u64(&space, head + 16)?))
        }) else {
            return;
        };
        // The low bit of an entry marks a priority-inheritance futex.
        let mut entry = first;
        for _ in 0..LIST_LIMIT {
            if entry == head || entry == 0 {
                break;
            }
            let Ok(next) = self.mem(cx, false, |t| user::read_u64(&t.space(), entry & !1)) else { break };
            // The pending entry is dealt with last, once.
            if entry != pending {
                self.owner_died(cx, (entry & !1).wrapping_add_signed(offset));
            }
            entry = next;
        }
        if pending != 0 {
            self.owner_died(cx, (pending & !1).wrapping_add_signed(offset));
        }
    }

    /// Marks the futex at `addr` `FUTEX_OWNER_DIED` if this thread holds it, and wakes a waiter if it had any.
    fn owner_died(&mut self, cx: &mut Cx<'_>, addr: u64) {
        let tid = self.tid as u32;
        let old = self.mem(cx, true, |t| {
            t.space().update_u32(addr, |v| if v & FUTEX_TID_MASK == tid { (v & FUTEX_WAITERS) | FUTEX_OWNER_DIED } else { v }).map_err(|_| user::Fault(addr))
        });
        if let Ok(old) = old
            && old & FUTEX_TID_MASK == tid
            && old & FUTEX_WAITERS != 0
        {
            let key = self.futex_key(addr, false);
            self.k.futexes.wake(key, 1);
        }
    }
}
