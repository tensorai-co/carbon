//! The rest of the memory calls: `msync`, `mincore`, locking (`mlock`, `mlock2`, `munlock`, `mlockall`,
//! `munlockall`), sealing (`mseal`), NUMA policy (`set_mempolicy`, `get_mempolicy`, `mbind`, `migrate_pages`,
//! `move_pages`, `set_mempolicy_home_node`), protection keys (`pkey_alloc`, `pkey_free`, `pkey_mprotect`),
//! `map_shadow_stack`, `process_madvise`, `process_mrelease`, `cachestat`, and `swapon` and `swapoff`.
//!
//! A machine's memory is never paged out (nothing swaps; the disk is memory too), so locking only checks and, for
//! `mlock`, brings the pages in as Linux does. It has one NUMA node, 0, which every policy names. There are no
//! protection keys or shadow stacks to give.

use exec::PAGE;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Backing, page_up};
use crate::abi::*;
use crate::fs::Node;
use crate::process::Task;

// msync's flags.
const MS_ASYNC: u64 = 1;
const MS_INVALIDATE: u64 = 2;
const MS_SYNC: u64 = 4;
// mlockall's and mlock2's.
const MCL_CURRENT: u64 = 1;
const MCL_FUTURE: u64 = 2;
const MCL_ONFAULT: u64 = 4;
const MLOCK_ONFAULT: u64 = 1;
/// The resource limit on locked memory.
const RLIMIT_MEMLOCK: usize = 8;
// Memory policies, and the flags that go with one.
const MPOL_DEFAULT: u32 = 0;
const MPOL_PREFERRED: u32 = 1;
const MPOL_LOCAL: u32 = 4;
const MPOL_MAX: u32 = 7;
const MPOL_F_STATIC_NODES: u32 = 1 << 15;
const MPOL_F_RELATIVE_NODES: u32 = 1 << 14;
const MPOL_F_NUMA_BALANCING: u32 = 1 << 13;
const MPOL_MODE_FLAGS: u32 = MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES | MPOL_F_NUMA_BALANCING;
// get_mempolicy's flags.
const MPOL_F_NODE: u64 = 1;
const MPOL_F_ADDR: u64 = 2;
const MPOL_F_MEMS_ALLOWED: u64 = 4;
// mbind's and move_pages's.
const MPOL_MF_STRICT: u64 = 1;
const MPOL_MF_MOVE: u64 = 2;
const MPOL_MF_MOVE_ALL: u64 = 4;
// process_madvise's advice: what may be said of another process's memory.
const MADV_WILLNEED: u64 = 3;
const MADV_COLD: u64 = 20;
const MADV_PAGEOUT: u64 = 21;
const MADV_COLLAPSE: u64 = 25;

impl Task {
    /// The pages of `[addr, addr + len)`, checked: `addr` on a page, and every page in a region. ENOMEM if one isn't.
    fn mapped_pages(&self, addr: u64, len: u64) -> Result<std::ops::Range<u64>, Errno> {
        if !addr.is_multiple_of(PAGE) {
            return Err(EINVAL);
        }
        let end = addr.checked_add(page_up(len)).ok_or(ENOMEM)?;
        let state = lock(&self.process.state);
        let mut at = addr;
        while at < end {
            let (r, area) = state.regions.find(at).ok_or(ENOMEM)?;
            if matches!(area.backing, Backing::Reserved) {
                return Err(ENOMEM);
            }
            at = r.end;
        }
        Ok(addr..end)
    }

    pub(crate) fn msync(&mut self, addr: u64, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(MS_ASYNC | MS_INVALIDATE | MS_SYNC) != 0 || (flags & MS_ASYNC != 0 && flags & MS_SYNC != 0) {
            return Err(EINVAL);
        }
        self.mapped_pages(addr, len)?;
        Ok(0)
    }

    /// `mincore`: a byte for each page, 1 if it's in memory: a page the program has touched, a shared page that
    /// exists, or any page of a file (the disk is memory).
    pub(crate) fn mincore(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, vec_at: u64) -> Result<u64, Errno> {
        let range = self.mapped_pages(addr, len)?;
        let areas: Vec<(std::ops::Range<u64>, Backing)> = {
            let state = lock(&self.process.state);
            state.regions.iter().filter(|(r, _)| r.start < range.end && range.start < r.end).map(|(r, a)| (r, a.backing.clone())).collect()
        };
        let mut bytes = Vec::with_capacity(((range.end - range.start) / PAGE) as usize);
        let mut page = range.start;
        while page < range.end {
            let Some((r, backing)) = areas.iter().find(|(r, _)| r.contains(&page)) else { break };
            let resident = match backing {
                Backing::File { .. } => true,
                Backing::Shared { mem, offset, .. } => mem.resident((offset + page - r.start) / PAGE, 1) > 0,
                _ => cx.turn(|t| t.space().page(page).is_some()),
            };
            bytes.push(u8::from(resident));
            page += PAGE;
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), vec_at, &bytes))?;
        Ok(0)
    }

    /// Whether this process may lock `len` more bytes: root may always; anyone else up to `RLIMIT_MEMLOCK`.
    fn may_lock(&self, len: u64) -> Result<(), Errno> {
        let state = lock(&self.process.state);
        let limit = state.limits.get(RLIMIT_MEMLOCK).0;
        match () {
            _ if state.creds.uid[1] == 0 || len <= limit => Ok(()),
            _ if limit == 0 => Err(EPERM),
            _ => Err(ENOMEM),
        }
    }

    /// `mlock` and `mlock2`: the range's pages, brought in now unless `MLOCK_ONFAULT`.
    pub(crate) fn mlock(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !MLOCK_ONFAULT != 0 {
            return Err(EINVAL);
        }
        let start = addr & !(PAGE - 1);
        let range = self.mapped_pages(start, len + (addr - start))?;
        self.may_lock(range.end - range.start)?;
        if flags & MLOCK_ONFAULT == 0 {
            let mut page = range.start;
            while page < range.end {
                // Pages it can't read stay out, as Linux leaves PROT_NONE ones.
                let readable = lock(&self.process.state).regions.find(page).is_some_and(|(_, a)| a.prot.read());
                if readable && cx.turn(|t| t.space().page(page).is_none()) {
                    let _ = self.fault(cx, page, exec::Prot::R);
                }
                page += PAGE;
            }
        }
        Ok(0)
    }

    pub(crate) fn munlock(&mut self, addr: u64, len: u64) -> Result<u64, Errno> {
        let start = addr & !(PAGE - 1);
        self.mapped_pages(start, len + (addr - start)).map(|_| 0)
    }

    /// `mlockall`: what it says is noted, not done, as nothing is paged out anyway.
    pub(crate) fn mlockall(&mut self, flags: u64) -> Result<u64, Errno> {
        if flags == 0 || flags & !(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT) != 0 || flags & (MCL_CURRENT | MCL_FUTURE) == 0 {
            return Err(EINVAL);
        }
        self.may_lock(0).map(|()| 0)
    }

    /// `mseal`: the range can no longer be unmapped, moved, mapped over or have its protection changed.
    pub(crate) fn mseal(&mut self, addr: u64, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 || !addr.is_multiple_of(PAGE) || addr.checked_add(page_up(len)).is_none() {
            return Err(EINVAL);
        }
        if len == 0 {
            return Ok(0);
        }
        let range = self.mapped_pages(addr, len)?;
        lock(&self.process.state).regions.update(range, |area| area.sealed = true);
        Ok(0)
    }

    /// A node mask from the program: `maxnode` bits at `at`. EINVAL if it names a node the machine doesn't have.
    fn read_nodes(&mut self, cx: &mut Cx<'_>, at: u64, maxnode: u64) -> Result<u64, Errno> {
        if at == 0 || maxnode == 0 {
            return Ok(0);
        }
        let bits = (maxnode - 1).min(1 << 16);
        let words = self.mem(cx, false, |t| (0..bits.div_ceil(64)).map(|i| user::read_u64(&t.space(), at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
        let mut mask = 0;
        for (i, word) in words.iter().enumerate() {
            let valid = if (i as u64 + 1) * 64 <= bits { u64::MAX } else { (1 << (bits % 64)) - 1 };
            let word = word & valid;
            if i == 0 {
                mask = word;
            } else if word != 0 {
                return Err(EINVAL);
            }
        }
        if mask & !1 != 0 {
            return Err(EINVAL);
        }
        Ok(mask)
    }

    /// A policy's mode and nodes, checked as `set_mempolicy` and `mbind` check them.
    fn policy(&mut self, cx: &mut Cx<'_>, mode: u64, at: u64, maxnode: u64) -> Result<(u32, u64), Errno> {
        let mode = u32::try_from(mode).map_err(|_| EINVAL)?;
        let (base, flags) = (mode & !MPOL_MODE_FLAGS, mode & MPOL_MODE_FLAGS);
        if base >= MPOL_MAX || (flags & MPOL_F_STATIC_NODES != 0 && flags & MPOL_F_RELATIVE_NODES != 0) {
            return Err(EINVAL);
        }
        let nodes = self.read_nodes(cx, at, maxnode)?;
        match base {
            MPOL_DEFAULT | MPOL_LOCAL if nodes != 0 => Err(EINVAL),
            MPOL_DEFAULT | MPOL_LOCAL | MPOL_PREFERRED => Ok((mode, nodes)),
            _ if nodes == 0 => Err(EINVAL),
            _ => Ok((mode, nodes)),
        }
    }

    pub(crate) fn set_mempolicy(&mut self, cx: &mut Cx<'_>, mode: u64, at: u64, maxnode: u64) -> Result<u64, Errno> {
        let policy = self.policy(cx, mode, at, maxnode)?;
        lock(&self.process.state).mempolicy = if policy.0 & !MPOL_MODE_FLAGS == MPOL_DEFAULT { (0, 0) } else { policy };
        Ok(0)
    }

    /// `get_mempolicy`: the process's policy, the nodes it may use (`MPOL_F_MEMS_ALLOWED`), or the node a page is on
    /// (`MPOL_F_NODE | MPOL_F_ADDR`): node 0, whichever.
    pub(crate) fn get_mempolicy(&mut self, cx: &mut Cx<'_>, mode_at: u64, nodes_at: u64, maxnode: u64, addr: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(MPOL_F_NODE | MPOL_F_ADDR | MPOL_F_MEMS_ALLOWED) != 0 || (flags & MPOL_F_MEMS_ALLOWED != 0 && flags != MPOL_F_MEMS_ALLOWED) {
            return Err(EINVAL);
        }
        if nodes_at != 0 && maxnode == 0 {
            return Err(EINVAL);
        }
        if flags & MPOL_F_ADDR != 0 {
            let page = addr & !(PAGE - 1);
            self.mapped_pages(page, PAGE).map_err(|_| EFAULT)?;
        }
        let (policy, nodes) = lock(&self.process.state).mempolicy;
        let (mode, nodes) = match flags {
            MPOL_F_MEMS_ALLOWED => (0, 1),
            f if f & MPOL_F_NODE != 0 => (0, nodes),
            _ => (policy, nodes),
        };
        self.mem(cx, true, |t| {
            if mode_at != 0 {
                user::write_u32(&mut t.space(), mode_at, mode)?;
            }
            if nodes_at != 0 {
                user::write(&mut t.space(), nodes_at, &vec![0; maxnode.div_ceil(64).min(1024) as usize * 8])?;
                user::write_u64(&mut t.space(), nodes_at, nodes)?;
            }
            Ok(())
        })?;
        Ok(0)
    }

    /// `mbind`: a policy for a range, checked; with one node, it's where the pages are already.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mbind(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, mode: u64, at: u64, maxnode: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(MPOL_MF_STRICT | MPOL_MF_MOVE | MPOL_MF_MOVE_ALL) != 0 {
            return Err(EINVAL);
        }
        self.policy(cx, mode, at, maxnode)?;
        self.mapped_pages(addr, len).map_err(|e| if e == ENOMEM { EFAULT } else { e })?;
        Ok(0)
    }

    /// `migrate_pages`: none moves, as there's only node 0 to be on.
    pub(crate) fn migrate_pages(&mut self, cx: &mut Cx<'_>, pid: i32, maxnode: u64, old_at: u64, new_at: u64) -> Result<u64, Errno> {
        self.process_of(pid).ok_or(ESRCH)?;
        self.read_nodes(cx, old_at, maxnode)?;
        self.read_nodes(cx, new_at, maxnode)?;
        Ok(0)
    }

    /// `move_pages`: each page's node (0) in `status`, or -EFAULT for one that isn't mapped; moving one to another
    /// node is -ENODEV.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn move_pages(&mut self, cx: &mut Cx<'_>, pid: i32, count: u64, pages_at: u64, nodes_at: u64, status_at: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(MPOL_MF_MOVE | MPOL_MF_MOVE_ALL) != 0 || count > 1 << 20 {
            return Err(EINVAL);
        }
        let process = self.process_of(pid).ok_or(ESRCH)?;
        let pages = self.mem(cx, false, |t| (0..count).map(|i| user::read_u64(&t.space(), pages_at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
        let nodes = if nodes_at == 0 { Vec::new() } else { self.mem(cx, false, |t| (0..count).map(|i| user::read_u32(&t.space(), nodes_at + i * 4)).collect::<Result<Vec<u32>, _>>())? };
        let status: Vec<u8> = {
            let state = lock(&process.state);
            pages
                .iter()
                .enumerate()
                .map(|(i, addr)| {
                    let mapped = state.regions.find(*addr).is_some_and(|(_, a)| !matches!(a.backing, Backing::Reserved));
                    match nodes.get(i) {
                        _ if !mapped => -EFAULT.0,
                        Some(node) if *node != 0 => -ENODEV.0,
                        _ => 0,
                    }
                })
                .flat_map(i32::to_le_bytes)
                .collect()
        };
        self.mem(cx, true, |t| user::write(&mut t.space(), status_at, &status))?;
        Ok(0)
    }

    pub(crate) fn set_mempolicy_home_node(&mut self, addr: u64, len: u64, node: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 || node != 0 {
            return Err(EINVAL);
        }
        self.mapped_pages(addr, len).map(|_| 0)
    }

    /// `pkey_alloc`: there are no protection keys to give, as on a CPU without them.
    pub(crate) fn pkey_alloc(&mut self, flags: u64, rights: u64) -> Result<u64, Errno> {
        if flags != 0 || rights & !3 != 0 {
            return Err(EINVAL);
        }
        Err(ENOSPC)
    }

    /// `pkey_mprotect`: with key -1 (none), `mprotect`; no other key was ever given.
    pub(crate) fn pkey_mprotect(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, prot: u64, key: i32) -> Result<u64, Errno> {
        if key != -1 {
            return Err(EINVAL);
        }
        self.mprotect(cx, addr, len, prot)
    }

    /// `process_madvise`: advice about another process's memory (by pidfd), taken, and nothing more. Gives the bytes
    /// it was about.
    pub(crate) fn process_madvise(&mut self, cx: &mut Cx<'_>, pidfd: i32, iov: u64, count: u64, advice: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 || count > 1024 || !matches!(advice, MADV_WILLNEED | MADV_COLD | MADV_PAGEOUT | MADV_COLLAPSE) {
            return Err(EINVAL);
        }
        let (process, _) = self.pidfd_process(pidfd)?;
        if lock(&process.state).zombie.is_some() {
            return Err(ESRCH);
        }
        let iovecs = self.iovecs(cx, iov, count)?;
        Ok(iovecs.iter().map(|(_, len)| len).sum())
    }

    /// `process_mrelease`: an ending process's memory goes as it ends, so there's nothing to hurry; one that isn't
    /// ending is EINVAL.
    pub(crate) fn process_mrelease(&mut self, pidfd: i32, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let (process, _) = self.pidfd_process(pidfd)?;
        let state = lock(&process.state);
        if state.exiting.is_some() || state.zombie.is_some() { Ok(0) } else { Err(EINVAL) }
    }

    /// `cachestat`: how much of a range of a file is in memory: all of a disk file's (the disk is memory), and a
    /// memfd's pages that exist. Nothing is ever dirty, written back or evicted.
    pub(crate) fn cachestat(&mut self, cx: &mut Cx<'_>, fd: i32, range_at: u64, at: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let open = self.file(fd)?;
        let (offset, len) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), range_at)?, user::read_u64(&t.space(), range_at + 8)?)))?;
        let first = offset / PAGE;
        let cached = match &open.node {
            Node::File(ino) => {
                let size = lock(&self.k.disk).stat(*ino)?.size;
                let end = if len == 0 { size } else { offset.saturating_add(len).min(size) };
                end.div_ceil(PAGE).saturating_sub(first)
            }
            Node::Memfd(mem, _) => {
                let end = if len == 0 { mem.size() } else { offset.saturating_add(len).min(mem.size()) };
                mem.resident(first, end.div_ceil(PAGE).saturating_sub(first))
            }
            _ => 0,
        };
        let mut b = vec![0u8; 40];
        b[..8].copy_from_slice(&cached.to_le_bytes());
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }
}

impl Task {
    /// `remap_file_pages`, as Linux has done it since 4.0: the pages of a shared mapping in `[start, start + size)`
    /// mapped again, at the same place, from page `pgoff` of what the mapping is of. `prot` must be 0. A disk file's
    /// mappings don't say whether they're shared yet (they act as private), so those are taken too.
    pub(crate) fn remap_file_pages(&mut self, cx: &mut Cx<'_>, start: u64, size: u64, prot: u64, pgoff: u64) -> Result<u64, Errno> {
        let start = start & !(PAGE - 1);
        let size = page_up(size);
        let end = start.checked_add(size).ok_or(EINVAL)?;
        if prot != 0 || size == 0 {
            return Err(EINVAL);
        }
        let offset = pgoff.checked_mul(PAGE).ok_or(EINVAL)?;
        let area = {
            let state = lock(&self.process.state);
            let (range, area) = state.regions.find(start).ok_or(EINVAL)?;
            if end > range.end {
                return Err(EINVAL);
            }
            let mut area = area.clone();
            area.backing = match area.backing {
                Backing::File { ino, .. } => Backing::File { ino, offset },
                Backing::Shared { mem, private: false, .. } => Backing::Shared { mem, offset, private: false },
                _ => return Err(EINVAL),
            };
            area
        };
        self.place(cx, start, size, area, MAP_FIXED).map(|_| 0)
    }
}
