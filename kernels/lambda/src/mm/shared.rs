//! Shared memory: pages that every mapping of them sees, in one process or several. `MAP_SHARED | MAP_ANONYMOUS` (and
//! `/dev/zero` mapped shared), `memfd_create` and `memfd_secret`, and SysV shared memory (`shmget`, `shmat`, `shmdt`,
//! `shmctl`) are each a [`SharedMem`]: frames lambda holds, made as they're first touched, which a fault maps
//! writable as they are (`Map::frame`) rather than copy-on-write. exec keeps such a page shared through `fork`, and a
//! page faulted later maps the same frame, so parent and child go on sharing. A memfd's `read` and `write` go to the
//! same frames, so what's written either way is seen both ways.
//!
//! Not yet: `MAP_SHARED` of a file on the disk still acts as a private mapping (see `Backing::File`). And the
//! translator doesn't see x86-64 code written through another mapping of a shared page (exec's to-do list).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use exec::{Frame, PAGE, Prot};
use kernels_core::lock;
use kernels_core::thread::{Cx, SharedExec};
use kernels_core::user;

use super::{Area, Backing, page_up};
use crate::abi::*;
use crate::fs::{Node, Open};
use crate::process::Task;

// Seals (`fcntl`'s `F_ADD_SEALS`): no more seals, no shrinking, no growing, no writing, no new writable mappings,
// no changing the mode's execute bits.
pub(crate) const F_SEAL_SEAL: u32 = 0x01;
pub(crate) const F_SEAL_SHRINK: u32 = 0x02;
pub(crate) const F_SEAL_GROW: u32 = 0x04;
pub(crate) const F_SEAL_WRITE: u32 = 0x08;
pub(crate) const F_SEAL_FUTURE_WRITE: u32 = 0x10;
pub(crate) const F_SEAL_EXEC: u32 = 0x20;
const ALL_SEALS: u32 = 0x3f;

/// Each shared memory object's inode number, for `stat`.
static NEXT_INO: AtomicU64 = AtomicU64::new(1);

/// Pages shared by every mapping of them, and a size, for a memfd or a SysV segment.
pub(crate) struct SharedMem {
    exec: SharedExec,
    inner: Mutex<Inner>,
    pub ino: u64,
}

struct Inner {
    /// Its frames by page, made as they're first wanted.
    frames: BTreeMap<u64, Frame>,
    size: u64,
    seals: u32,
}

impl std::fmt::Debug for SharedMem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SharedMem({})", self.ino)
    }
}

impl Drop for SharedMem {
    fn drop(&mut self) {
        let inner = self.inner.get_mut().unwrap_or_else(|p| p.into_inner());
        if inner.frames.is_empty() {
            return;
        }
        let mut exec = lock(&self.exec);
        let mut memory = exec.memory();
        for frame in std::mem::take(&mut inner.frames).into_values() {
            memory.free_frame(frame);
        }
    }
}

impl SharedMem {
    pub fn new(exec: SharedExec, size: u64, seals: u32) -> Arc<SharedMem> {
        Arc::new(SharedMem { exec, inner: Mutex::new(Inner { frames: BTreeMap::new(), size, seals }), ino: NEXT_INO.fetch_add(1, Ordering::Relaxed) })
    }

    pub fn size(&self) -> u64 {
        lock(&self.inner).size
    }

    pub fn seals(&self) -> u32 {
        lock(&self.inner).seals
    }

    /// Adds seals, unless `F_SEAL_SEAL` is one already.
    pub fn add_seals(&self, seals: u32) -> Result<(), Errno> {
        if seals & !ALL_SEALS != 0 {
            return Err(EINVAL);
        }
        let mut inner = lock(&self.inner);
        if inner.seals & F_SEAL_SEAL != 0 {
            return Err(EPERM);
        }
        inner.seals |= seals;
        Ok(())
    }

    /// How many of its pages from `first`, `count` of them, hold a frame.
    pub fn resident(&self, first: u64, count: u64) -> u64 {
        lock(&self.inner).frames.range(first..first.saturating_add(count)).count() as u64
    }

    /// Up to `buffer`'s length from `offset`, stopping at its size: how much was read.
    pub fn read(&self, offset: u64, buffer: &mut [u8]) -> usize {
        let inner = lock(&self.inner);
        let n = buffer.len().min(inner.size.saturating_sub(offset) as usize);
        buffer[..n].fill(0);
        let mut exec = lock(&self.exec);
        let memory = exec.memory();
        let mut done = 0;
        while done < n {
            let at = offset + done as u64;
            let (page, within) = (at / PAGE, (at % PAGE) as usize);
            let part = (n - done).min(PAGE as usize - within);
            if let Some(frame) = inner.frames.get(&page) {
                memory.frame_read(*frame, within, &mut buffer[done..done + part]);
            }
            done += part;
        }
        n
    }

    /// Writes `data` at `offset`, growing it to fit: how much was written. Sealed against writing or growing, EPERM.
    pub fn write(&self, offset: u64, data: &[u8]) -> Result<usize, Errno> {
        let mut inner = lock(&self.inner);
        let end = offset.checked_add(data.len() as u64).ok_or(EFBIG)?;
        if inner.seals & (F_SEAL_WRITE | F_SEAL_FUTURE_WRITE) != 0 || (end > inner.size && inner.seals & F_SEAL_GROW != 0) {
            return Err(EPERM);
        }
        let mut exec = lock(&self.exec);
        let mut memory = exec.memory();
        let mut done = 0;
        while done < data.len() {
            let at = offset + done as u64;
            let (page, within) = (at / PAGE, (at % PAGE) as usize);
            let part = (data.len() - done).min(PAGE as usize - within);
            let frame = match inner.frames.get(&page) {
                Some(frame) => *frame,
                None => {
                    let frame = memory.new_frame().map_err(|_| ENOSPC)?;
                    inner.frames.insert(page, frame);
                    frame
                }
            };
            memory.frame_write(frame, within, &data[done..done + part]);
            done += part;
        }
        inner.size = inner.size.max(end);
        Ok(data.len())
    }

    /// A new size: pages past it go (a mapping that still has one keeps it), and what's left of the last is zeroed
    /// past the end, so growing again shows zeroes.
    pub fn truncate(&self, size: u64) -> Result<(), Errno> {
        let mut inner = lock(&self.inner);
        if (size < inner.size && inner.seals & F_SEAL_SHRINK != 0) || (size > inner.size && inner.seals & F_SEAL_GROW != 0) {
            return Err(EPERM);
        }
        if size < inner.size {
            let mut exec = lock(&self.exec);
            let mut memory = exec.memory();
            for frame in inner.frames.split_off(&size.div_ceil(PAGE)).into_values() {
                memory.free_frame(frame);
            }
            if let Some(frame) = inner.frames.get(&(size / PAGE))
                && !size.is_multiple_of(PAGE)
            {
                let within = (size % PAGE) as usize;
                memory.frame_write(*frame, within, &vec![0; PAGE as usize - within]);
            }
        }
        inner.size = size;
        Ok(())
    }

    /// Zeroes a range within its size, letting whole pages go (`fallocate`'s punched hole).
    pub fn zero(&self, offset: u64, len: u64) {
        let mut inner = lock(&self.inner);
        let end = offset.saturating_add(len).min(inner.size);
        let mut exec = lock(&self.exec);
        let mut memory = exec.memory();
        let mut at = offset;
        while at < end {
            let (page, within) = (at / PAGE, at % PAGE);
            let part = (end - at).min(PAGE - within);
            if within == 0 && part == PAGE {
                if let Some(frame) = inner.frames.remove(&page) {
                    memory.free_frame(frame);
                }
            } else if let Some(frame) = inner.frames.get(&page) {
                memory.frame_write(*frame, within as usize, &vec![0; part as usize]);
            }
            at += part;
        }
    }
}

/// Faults page `index` of `mem` into `page` of the space: the frame itself for a shared mapping, or a copy-on-write
/// view of it for a private one (a copy once written).
pub(crate) fn fault_shared(cx: &mut Cx<'_>, mem: &SharedMem, index: u64, page: u64, prot: Prot, private: bool, write: bool) -> Result<(), exec::Error> {
    // Its lock first, then exec's, as `read` and `write` take them.
    let mut inner = lock(&mem.inner);
    cx.turn(|t| {
        let frame = match inner.frames.get(&index) {
            Some(frame) => *frame,
            None => {
                let frame = t.memory().new_frame()?;
                inner.frames.insert(index, frame);
                frame
            }
        };
        if !private {
            return t.space().map(page..page + PAGE, prot).frame(frame);
        }
        t.space().map(page..page + PAGE, prot).copy_on_write(frame)?;
        if write {
            t.space().make_writable(page, prot)?;
        }
        Ok(())
    })
}

// memfd_create's flags.
const MFD_CLOEXEC: u64 = 0x01;
const MFD_ALLOW_SEALING: u64 = 0x02;
const MFD_HUGETLB: u64 = 0x04;
const MFD_NOEXEC_SEAL: u64 = 0x08;
const MFD_EXEC: u64 = 0x10;
/// The longest name a memfd may have, before `memfd:` goes in front.
const MFD_NAME_MAX: usize = 249;

// SysV shared memory.
const IPC_PRIVATE: i32 = 0;
const IPC_CREAT: u64 = 0o1000;
const IPC_EXCL: u64 = 0o2000;
const IPC_RMID: u64 = 0;
const IPC_SET: u64 = 1;
const IPC_STAT: u64 = 2;
const IPC_INFO: u64 = 3;
const SHM_LOCK: u64 = 11;
const SHM_UNLOCK: u64 = 12;
const SHM_STAT: u64 = 13;
const SHM_INFO: u64 = 14;
const SHM_STAT_ANY: u64 = 15;
/// Or'd into a command by some C libraries, for the 64-bit structures, which are the only ones here.
const IPC_64: u64 = 0x100;
const SHM_RDONLY: u64 = 0o10000;
const SHM_RND: u64 = 0o20000;
const SHM_REMAP: u64 = 0o40000;
const SHM_EXEC: u64 = 0o100000;
/// In a segment's mode: removed, to go when nothing has it attached; locked in memory.
const SHM_DEST: u32 = 0o1000;
const SHM_LOCKED: u32 = 0o2000;
/// The most segments, and the biggest, as Linux's defaults have them.
const SHMMNI: usize = 4096;
const SHMMAX: u64 = u64::MAX - (1 << 24);

/// A SysV shared memory segment.
pub(crate) struct Segment {
    key: i32,
    mem: Arc<SharedMem>,
    size: u64,
    /// Permission bits, `SHM_DEST` and `SHM_LOCKED`.
    mode: u32,
    owner: (u32, u32),
    creator: (u32, u32),
    cpid: i32,
    lpid: i32,
    /// Attached, detached and changed, in seconds.
    times: [u64; 3],
}

/// The machine's SysV shared memory segments, by ID.
#[derive(Default)]
pub(crate) struct ShmTable {
    segments: BTreeMap<i32, Segment>,
    next: i32,
}

impl Task {
    /// `MAP_SHARED | MAP_ANONYMOUS`: new shared pages, as many as `len` covers.
    pub(crate) fn shared_anon(&self, len: u64) -> Backing {
        Backing::Shared { mem: SharedMem::new(self.k.exec.clone(), len, F_SEAL_SEAL), offset: 0, private: false }
    }

    /// `memfd_create`: a file in memory with no name on any disk, closed at `execve` with `MFD_CLOEXEC`; with
    /// `MFD_ALLOW_SEALING` it may be sealed, else it's sealed against sealing.
    pub(crate) fn memfd_create(&mut self, cx: &mut Cx<'_>, name_at: u64, flags: u64) -> Result<u64, Errno> {
        let hugetlb_size = 0x3f << 26;
        if flags & !(MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_HUGETLB | MFD_NOEXEC_SEAL | MFD_EXEC | hugetlb_size) != 0
            || (flags & MFD_NOEXEC_SEAL != 0 && flags & MFD_EXEC != 0)
            || (flags & hugetlb_size != 0 && flags & MFD_HUGETLB == 0)
        {
            return Err(EINVAL);
        }
        let name = match self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, MFD_NAME_MAX + 1)) {
            Ok(name) => name,
            // No end within the longest name, in memory that can be read: it's too long.
            Err(_) => return Err(if self.mem(cx, false, |t| user::read_vec(&t.space(), name_at, 1)).is_ok() { EINVAL } else { EFAULT }),
        };
        let mut seals = if flags & (MFD_ALLOW_SEALING | MFD_NOEXEC_SEAL) != 0 { 0 } else { F_SEAL_SEAL };
        if flags & MFD_NOEXEC_SEAL != 0 {
            seals |= F_SEAL_EXEC;
        }
        let mem = SharedMem::new(self.k.exec.clone(), 0, seals);
        let path = format!("/memfd:{} (deleted)", String::from_utf8_lossy(&name));
        let open = Open::new(Node::Memfd(mem, false), O_RDWR | O_LARGEFILE, path);
        lock(&self.process.state).files.add(open, flags & MFD_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    /// `memfd_secret`: memory only its mappings reach, never `read` or `write`.
    pub(crate) fn memfd_secret(&mut self, flags: u64) -> Result<u64, Errno> {
        if flags & !u64::from(O_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let open = Open::new(Node::Memfd(SharedMem::new(self.k.exec.clone(), 0, 0), true), O_RDWR, "/secretmem (deleted)");
        lock(&self.process.state).files.add(open, flags != 0, 0).map(|fd| fd as u64)
    }

    /// The process's IDs, for a segment's creator and owner.
    fn ids(&self) -> (u32, u32) {
        let state = lock(&self.process.state);
        (state.creds.uid[1], state.creds.gid[1])
    }

    /// How many attachments each segment has, across every process.
    fn attachments(&self) -> BTreeMap<i32, u64> {
        let mut counts = BTreeMap::new();
        for process in lock(&self.k.processes).values() {
            let state = lock(&process.state);
            if state.zombie.is_some() {
                continue;
            }
            for (id, _) in state.shm_attached.values() {
                *counts.entry(*id).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Lets go of removed segments nothing has attached any more.
    pub(crate) fn shm_tidy(&self) {
        let counts = self.attachments();
        lock(&self.ipc_ns().shm).segments.retain(|id, s| s.mode & SHM_DEST == 0 || counts.get(id).copied().unwrap_or(0) > 0);
    }

    pub(crate) fn shmget(&mut self, key: i32, size: u64, flags: u64) -> Result<u64, Errno> {
        let (uid, gid) = self.ids();
        let now = self.k.host.wall() / 1_000_000_000;
        let __ipc = self.ipc_ns();
        let mut table = lock(&__ipc.shm);
        if key != IPC_PRIVATE
            && let Some((id, segment)) = table.segments.iter().find(|(_, s)| s.key == key && s.mode & SHM_DEST == 0)
        {
            if flags & IPC_CREAT != 0 && flags & IPC_EXCL != 0 {
                return Err(EEXIST);
            }
            if size > segment.size {
                return Err(EINVAL);
            }
            return Ok(*id as u64);
        }
        if key != IPC_PRIVATE && flags & IPC_CREAT == 0 {
            return Err(ENOENT);
        }
        if size == 0 || size > SHMMAX {
            return Err(EINVAL);
        }
        if table.segments.len() >= SHMMNI {
            return Err(ENOSPC);
        }
        let id = table.next;
        table.next += 1;
        let mem = SharedMem::new(self.k.exec.clone(), page_up(size), F_SEAL_SEAL);
        table.segments.insert(id, Segment { key, mem, size, mode: flags as u32 & 0o777, owner: (uid, gid), creator: (uid, gid), cpid: self.process.pid, lpid: 0, times: [0, 0, now] });
        Ok(id as u64)
    }

    /// `shmat`: maps a segment, where it's asked (rounded down to a page with `SHM_RND`) or where there's room.
    pub(crate) fn shmat(&mut self, cx: &mut Cx<'_>, id: i32, addr: u64, flags: u64) -> Result<u64, Errno> {
        let (mem, size) = {
            let __ipc = self.ipc_ns();
            let table = lock(&__ipc.shm);
            let segment = table.segments.get(&id).ok_or(EINVAL)?;
            (segment.mem.clone(), segment.size)
        };
        let addr = if flags & SHM_RND != 0 { addr & !(PAGE - 1) } else { addr };
        if !addr.is_multiple_of(PAGE) {
            return Err(EINVAL);
        }
        let mut prot = if flags & SHM_RDONLY != 0 { PROT_READ } else { PROT_READ | PROT_WRITE };
        if flags & SHM_EXEC != 0 {
            prot |= PROT_EXEC;
        }
        let len = page_up(size);
        let area = Area::new(super::prot_from(prot), Backing::Shared { mem, offset: 0, private: false }, format!("/SYSV{:08x} (deleted)", lock(&self.ipc_ns().shm).segments.get(&id).map_or(0, |s| s.key)));
        let placement = if addr == 0 {
            0
        } else if flags & SHM_REMAP != 0 {
            MAP_FIXED
        } else {
            MAP_FIXED_NOREPLACE
        };
        let at = self.place(cx, addr, len, area, placement).map_err(|e| if e == EEXIST { EINVAL } else { e })?;
        lock(&self.process.state).shm_attached.insert(at, (id, len));
        let now = self.k.host.wall() / 1_000_000_000;
        if let Some(segment) = lock(&self.ipc_ns().shm).segments.get_mut(&id) {
            segment.times[0] = now;
            segment.lpid = self.process.pid;
        }
        Ok(at)
    }

    pub(crate) fn shmdt(&mut self, cx: &mut Cx<'_>, addr: u64) -> Result<u64, Errno> {
        let (id, len) = *lock(&self.process.state).shm_attached.get(&addr).ok_or(EINVAL)?;
        self.munmap(cx, addr, len)?;
        lock(&self.process.state).shm_attached.remove(&addr);
        let now = self.k.host.wall() / 1_000_000_000;
        if let Some(segment) = lock(&self.ipc_ns().shm).segments.get_mut(&id) {
            segment.times[1] = now;
            segment.lpid = self.process.pid;
        }
        self.shm_tidy();
        Ok(0)
    }

    pub(crate) fn shmctl(&mut self, cx: &mut Cx<'_>, id: i32, cmd: u64, at: u64) -> Result<u64, Errno> {
        let cmd = cmd & !IPC_64;
        let now = self.k.host.wall() / 1_000_000_000;
        match cmd {
            IPC_INFO | SHM_INFO => {
                let __ipc = self.ipc_ns();
                let table = lock(&__ipc.shm);
                let highest = table.segments.keys().next_back().copied().unwrap_or(0);
                let words: Vec<u64> = if cmd == IPC_INFO {
                    // shmmax, shmmin, shmmni, shmseg, shmall, and four unused.
                    vec![SHMMAX, 1, SHMMNI as u64, SHMMNI as u64, SHMMAX / PAGE, 0, 0, 0, 0]
                } else {
                    // used_ids (an int, padded), shm_tot, shm_rss, shm_swp, and two swap counts.
                    let pages: u64 = table.segments.values().map(|s| s.size.div_ceil(PAGE)).sum();
                    let resident: u64 = table.segments.values().map(|s| s.mem.resident(0, u64::MAX)).sum();
                    vec![table.segments.len() as u64, pages, resident, 0, 0, 0]
                };
                drop(table);
                let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
                Ok(highest as u64)
            }
            IPC_STAT | SHM_STAT | SHM_STAT_ANY => {
                let nattch = self.attachments().get(&id).copied().unwrap_or(0);
                let bytes = {
                    let __ipc = self.ipc_ns();
                    let table = lock(&__ipc.shm);
                    let s = table.segments.get(&id).ok_or(EINVAL)?;
                    // struct shmid64_ds: ipc64_perm (key, uid, gid, cuid, cgid, mode, seq), then the segment's own.
                    let mut b = vec![0u8; 112];
                    for (i, v) in [s.key as u32, s.owner.0, s.owner.1, s.creator.0, s.creator.1, s.mode].into_iter().enumerate() {
                        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                    }
                    for (i, v) in [s.size, s.times[0], s.times[1], s.times[2]].into_iter().enumerate() {
                        b[48 + i * 8..56 + i * 8].copy_from_slice(&v.to_le_bytes());
                    }
                    b[80..84].copy_from_slice(&s.cpid.to_le_bytes());
                    b[84..88].copy_from_slice(&s.lpid.to_le_bytes());
                    b[88..96].copy_from_slice(&nattch.to_le_bytes());
                    b
                };
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
                Ok(if cmd == IPC_STAT { 0 } else { id as u64 })
            }
            IPC_SET => {
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, 24))?;
                let word = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or_default());
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.shm);
                let s = table.segments.get_mut(&id).ok_or(EINVAL)?;
                s.owner = (word(1), word(2));
                s.mode = s.mode & !0o777 | word(5) & 0o777;
                s.times[2] = now;
                Ok(0)
            }
            IPC_RMID => {
                {
                    let __ipc = self.ipc_ns();
                    let mut table = lock(&__ipc.shm);
                    let s = table.segments.get_mut(&id).ok_or(EINVAL)?;
                    s.mode |= SHM_DEST;
                    s.key = IPC_PRIVATE;
                    s.times[2] = now;
                }
                self.shm_tidy();
                Ok(0)
            }
            SHM_LOCK | SHM_UNLOCK => {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.shm);
                let s = table.segments.get_mut(&id).ok_or(EINVAL)?;
                s.mode = if cmd == SHM_LOCK { s.mode | SHM_LOCKED } else { s.mode & !SHM_LOCKED };
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }
}

/// A SysV segment's memory, by its ID, in an IPC namespace: what an X client's `shmid` names to MIT-SHM.
pub(crate) fn shm_segment(ipc: &crate::ns::IpcNs, id: i32) -> Option<Arc<SharedMem>> {
    lock(&ipc.shm).segments.get(&id).map(|s| s.mem.clone())
}
