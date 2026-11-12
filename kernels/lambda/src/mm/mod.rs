//! Memory: `mmap`, `munmap`, `mprotect`, `mremap`, `madvise` and `brk`, and filling pages when the program touches
//! them. A mapping only makes a region; exec maps a page the first time the program faults on it, zeroed for anonymous
//! memory or from the page cache, copy-on-write, for a file; shared memory maps its own frames (`shared`). The rest
//! of the memory calls (`msync`, `mincore`, `mlock`, `mseal`, NUMA policy, protection keys) are in `control`.

mod control;
pub(crate) mod shared;

use std::sync::Arc;

use exec::{PAGE, Prot};
use kernels_core::cache::Key;
use kernels_core::lock;
use kernels_core::mm::Region;
use kernels_core::thread::Cx;

use crate::abi::*;
use crate::fs::Node;
use crate::process::Task;

/// Whether any region in `range` is sealed.
fn any_sealed(regions: &kernels_core::mm::Regions<Area>, range: std::ops::Range<u64>) -> bool {
    regions.iter().any(|(r, a)| a.sealed && r.start < range.end && range.start < r.end)
}

pub(crate) fn page_down(a: u64) -> u64 {
    a & !(PAGE - 1)
}

pub(crate) fn page_up(a: u64) -> u64 {
    a.next_multiple_of(PAGE)
}

/// What the program was promised in a region.
#[derive(Debug, Clone)]
pub(crate) struct Area {
    pub prot: Prot,
    pub backing: Backing,
    /// For `/proc/self/maps`.
    pub name: String,
    /// Sealed (`mseal`): it can't be unmapped, moved, remapped or have its protection changed.
    pub sealed: bool,
}

impl Area {
    pub fn new(prot: Prot, backing: Backing, name: impl Into<String>) -> Area {
        Area { prot, backing, name: name.into(), sealed: false }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Backing {
    /// Zeros until written. Also an ELF image's pages, which are mapped when it's loaded.
    Anon,
    /// A file's pages from `offset`. Shared mappings of a file on the disk aren't written back yet, so for now they
    /// act as private ones.
    File { ino: layers::Ino, offset: u64 },
    /// Shared memory's pages from `offset`: its frames themselves, or with `private` copy-on-write views of them.
    Shared { mem: Arc<shared::SharedMem>, offset: u64, private: bool },
    /// Never the program's: what the backend keeps for itself.
    Reserved,
}

impl Region for Area {
    fn advanced(&self, by: u64) -> Area {
        let backing = match &self.backing {
            Backing::File { ino, offset } => Backing::File { ino: *ino, offset: offset + by },
            Backing::Shared { mem, offset, private } => Backing::Shared { mem: mem.clone(), offset: offset + by, private: *private },
            other => other.clone(),
        };
        Area { prot: self.prot, backing, name: self.name.clone(), sealed: self.sealed }
    }
}

pub(crate) fn prot_from(bits: u64) -> Prot {
    let mut prot = Prot::NONE;
    for (bit, p) in [(PROT_READ, Prot::R), (PROT_WRITE, Prot::W), (PROT_EXEC, Prot::X)] {
        if bits & bit != 0 {
            prot = prot | p;
        }
    }
    prot
}

impl Task {
    /// The program touched `addr` in a way exec's tables don't allow: fill the page if it's within what the program
    /// was promised. Err means it wasn't, and the program gets SIGSEGV.
    pub(crate) fn fault(&mut self, cx: &mut Cx<'_>, addr: u64, access: Prot) -> Result<(), ()> {
        let state = lock(&self.process.state);
        fill(&self.k, &state, cx, addr, access, false)
    }
}

/// Fills the page at `addr` of the process `state` is, as its touch would: in the space each turn uses, which is that
/// process's (through `Cx::elsewhere`, for another's). With `force`, a write to a private mapping the program can't
/// write is let through (to a copy of its page, which stays read-only to the program), as `ptrace`'s pokes are.
pub(crate) fn fill(k: &crate::Kernel, state: &crate::process::ProcState, cx: &mut Cx<'_>, addr: u64, access: Prot, force: bool) -> Result<(), ()> {
    {
        let page = page_down(addr);
        let (range, area) = state.regions.find(addr).map(|(r, a)| (r, a.clone())).ok_or(())?;
        let forced = force && access == Prot::W && area.prot.contains(Prot::R) && !matches!(area.backing, Backing::Shared { private: false, .. });
        if !area.prot.contains(access) && !forced {
            return Err(());
        }
        let file = match area.backing {
            Backing::Reserved => return Err(()),
            Backing::Anon => None,
            Backing::File { ino, offset } => Some((ino, offset + (page - range.start))),
            Backing::Shared { ref mem, offset, private } => {
                let index = (offset + (page - range.start)) / PAGE;
                let mem = mem.clone();
                // Already there, as another thread may have made it: fine, unless it's a private copy to make.
                let present = cx.turn(|t| t.space().page(page));
                if let Some(present) = present {
                    if access.write() && present.shared {
                        return cx.turn(|t| t.space().make_writable(page, area.prot)).map(|_| ()).map_err(|_| ());
                    }
                    return if present.prot.contains(access) || forced { Ok(()) } else { Err(()) };
                }
                return shared::fault_shared(cx, &mem, index, page, area.prot, private, access.write()).map_err(|_| ());
            }
        };
        let disk = file.map(|_| lock(&k.disk));
        let mut cache = file.map(|_| lock(&k.cache));
        cx.turn(|t| {
            if let Some(present) = t.space().page(page) {
                // Already mapped: a copy-on-write page being written; one another thread touched first, which is
                // there now, so the program just runs again; or nothing exec can fix.
                if access.write() && present.shared {
                    return t.space().make_writable(page, area.prot).map(|_| ());
                }
                if present.prot.contains(access) || forced {
                    return Ok(());
                }
                return Err(exec::Error::Unmapped(addr));
            }
            match (file, &disk, &mut cache) {
                (Some((ino, offset)), Some(disk), Some(cache)) => {
                    let generation = disk.stat(ino).map(|s| s.generation).unwrap_or(0);
                    let key = Key { file: ino, generation, page: offset / PAGE };
                    let frame = cache.get(&mut t.memory(), key, |buffer| {
                        let _ = disk.read(ino, offset, buffer);
                    })?;
                    t.space().map(page..page + PAGE, area.prot).copy_on_write(frame)?;
                    if access.write() {
                        t.space().make_writable(page, area.prot)?;
                    }
                    Ok(())
                }
                _ => t.space().map(page..page + PAGE, area.prot).zeroed(),
            }
        })
        .map_err(|_| ())
    }
}

impl Task {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mmap(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, prot: u64, flags: u64, fd: i32, offset: u64) -> Result<u64, Errno> {
        if len == 0 || !offset.is_multiple_of(PAGE) || !addr.is_multiple_of(PAGE) && flags & MAP_FIXED != 0 {
            return Err(EINVAL);
        }
        let len = page_up(len);
        let shared = flags & MAP_SHARED != 0;
        let (backing, name) = if flags & MAP_ANONYMOUS != 0 {
            if shared { (self.shared_anon(len), "/dev/zero (deleted)".to_string()) } else { (Backing::Anon, String::new()) }
        } else {
            let open = self.file(fd)?;
            match &open.node {
                Node::File(ino) => (Backing::File { ino: *ino, offset }, open.path.clone()),
                Node::Zero | Node::Null if shared => (self.shared_anon(len), "/dev/zero (deleted)".to_string()),
                Node::Zero | Node::Null => (Backing::Anon, String::new()),
                Node::Memfd(mem, secret) => {
                    // A secret memfd is only ever shared; a sealed one can't be mapped to write.
                    if *secret && !shared {
                        return Err(EINVAL);
                    }
                    if shared && prot & PROT_WRITE != 0 && mem.seals() & (shared::F_SEAL_WRITE | shared::F_SEAL_FUTURE_WRITE) != 0 {
                        return Err(EPERM);
                    }
                    (Backing::Shared { mem: mem.clone(), offset, private: !shared }, open.path.clone())
                }
                _ => return Err(ENODEV),
            }
        };
        self.place(cx, addr, len, Area::new(prot_from(prot), backing, name), flags)
    }

    /// Puts `area` in the address space, `len` bytes of it: at `addr` with `MAP_FIXED` (replacing what's there) or
    /// `MAP_FIXED_NOREPLACE` (EEXIST if anything is), else at `addr` if it's free or wherever there's room.
    pub(crate) fn place(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, area: Area, flags: u64) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;
        let at = if fixed {
            if flags & MAP_FIXED_NOREPLACE != 0 && !state.regions.is_free(addr..addr + len) {
                return Err(EEXIST);
            }
            if state.regions.iter().any(|(r, a)| matches!(a.backing, Backing::Reserved) && r.start < addr + len && addr < r.end) {
                return Err(ENOMEM);
            }
            if any_sealed(&state.regions, addr..addr + len) {
                return Err(EPERM);
            }
            addr
        } else if addr != 0 && addr >= state.mmap.start && addr + len <= state.mmap.end && state.regions.is_free(addr..addr + len) {
            addr
        } else {
            state.regions.free_below(state.mmap.clone(), len, PAGE).ok_or(ENOMEM)?
        };
        let replaced = state.regions.remove(at..at + len);
        state.regions.insert(at..at + len, area);
        drop(state);
        drop(replaced);
        if fixed {
            cx.turn(|t| t.space().unmap(at..at + len));
        }
        Ok(at)
    }

    pub(crate) fn munmap(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64) -> Result<u64, Errno> {
        if !addr.is_multiple_of(PAGE) || len == 0 {
            return Err(EINVAL);
        }
        let range = addr..addr + page_up(len);
        let mut state = lock(&self.process.state);
        if any_sealed(&state.regions, range.clone()) {
            return Err(EPERM);
        }
        let removed = state.regions.remove(range.clone());
        // What the backend keeps stays.
        for (r, area) in removed {
            if matches!(area.backing, Backing::Reserved) {
                state.regions.insert(r, area);
            }
        }
        cx.turn(|t| t.space().unmap(range));
        Ok(0)
    }

    pub(crate) fn mprotect(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, prot: u64) -> Result<u64, Errno> {
        if !addr.is_multiple_of(PAGE) {
            return Err(EINVAL);
        }
        let range = addr..addr + page_up(len);
        let prot = prot_from(prot);
        let mut state = lock(&self.process.state);
        if any_sealed(&state.regions, range.clone()) {
            return Err(EPERM);
        }
        // Every page must be in a region.
        let mut at = range.start;
        while at < range.end {
            let (r, area) = state.regions.find(at).ok_or(ENOMEM)?;
            if matches!(area.backing, Backing::Reserved) {
                return Err(ENOMEM);
            }
            at = r.end;
        }
        state.regions.update(range.clone(), |area| area.prot = prot);
        cx.turn(|t| t.space().protect(range, prot));
        Ok(0)
    }

    pub(crate) fn mremap(&mut self, cx: &mut Cx<'_>, old: u64, old_len: u64, new_len: u64, flags: u64) -> Result<u64, Errno> {
        if !old.is_multiple_of(PAGE) || new_len == 0 || flags & MREMAP_FIXED != 0 {
            return Err(EINVAL);
        }
        let (old_len, new_len) = (page_up(old_len), page_up(new_len));
        let mut state = lock(&self.process.state);
        if any_sealed(&state.regions, old..old + old_len.max(PAGE)) {
            return Err(EPERM);
        }
        // The old range must be mapped throughout, in one area or several side by side (as a mapping that's grown
        // before is).
        let mut covered = old;
        for (r, a) in state.regions.iter() {
            if r.end <= covered {
                continue;
            }
            if r.start > covered || matches!(a.backing, Backing::Reserved) {
                break;
            }
            covered = r.end;
            if covered >= old + old_len {
                break;
            }
        }
        if covered < old + old_len {
            return Err(EFAULT);
        }
        // What it grows by continues its last area.
        let (range, area) = state.regions.find(old + old_len - PAGE).map(|(r, a)| (r, a.clone())).ok_or(EFAULT)?;
        if new_len <= old_len {
            state.regions.remove(old + new_len..old + old_len);
            cx.turn(|t| t.space().unmap(old + new_len..old + old_len));
            return Ok(old);
        }
        let tail = area.advanced(old + old_len - range.start);
        if state.regions.is_free(old + old_len..old + new_len) && old + new_len <= state.mmap.end.max(range.end) {
            state.regions.insert(old + old_len..old + new_len, tail);
            return Ok(old);
        }
        if flags & MREMAP_MAYMOVE == 0 {
            return Err(ENOMEM);
        }
        let to = state.regions.free_below(state.mmap.clone(), new_len, PAGE).ok_or(ENOMEM)?;
        for (r, a) in state.regions.remove(old..old + old_len) {
            state.regions.insert(r.start - old + to..r.end - old + to, a);
        }
        state.regions.insert(to + old_len..to + new_len, tail);
        cx.turn(|t| t.space().move_pages(old, to, old_len)).map_err(|_| ENOMEM)?;
        Ok(to)
    }

    pub(crate) fn madvise(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, advice: u64) -> Result<u64, Errno> {
        if advice == MADV_DONTNEED {
            // Forget the pages: the next touch fills them again, with zeros, from the file, or with the shared pages.
            let range = addr..addr + page_up(len);
            let state = lock(&self.process.state);
            // Discarding a sealed private page that can't be written would change what it holds.
            if state.regions.iter().any(|(r, a)| a.sealed && !a.prot.write() && !matches!(a.backing, Backing::Shared { private: false, .. }) && r.start < range.end && range.start < r.end) {
                return Err(EPERM);
            }
            if state.regions.iter().any(|(r, a)| matches!(a.backing, Backing::Reserved) && r.start < range.end && range.start < r.end) {
                return Err(EINVAL);
            }
            cx.turn(|t| t.space().unmap(range));
        }
        Ok(0)
    }

    pub(crate) fn brk(&mut self, cx: &mut Cx<'_>, wanted: u64) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let (start, end) = state.brk;
        if wanted < start {
            return Ok(end);
        }
        let (old_top, new_top) = (page_up(end), page_up(wanted));
        if new_top > old_top {
            if !state.regions.is_free(old_top..new_top) {
                return Ok(end);
            }
            state.regions.insert(old_top..new_top, Area::new(Prot::RW, Backing::Anon, "[heap]"));
        } else if new_top < old_top {
            state.regions.remove(new_top..old_top);
            cx.turn(|t| t.space().unmap(new_top..old_top));
        }
        state.brk.1 = wanted;
        Ok(wanted)
    }
}
