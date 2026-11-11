//! Memory: `mmap`, `munmap`, `mprotect`, `mremap`, `madvise` and `brk`, and filling pages when the program touches
//! them. A mapping only makes a region; exec maps a page the first time the program faults on it, zeroed for anonymous
//! memory or from the page cache, copy-on-write, for a file.

use exec::{PAGE, Prot};
use kernels_core::cache::Key;
use kernels_core::lock;
use kernels_core::mm::Region;
use kernels_core::thread::Cx;

use crate::abi::*;
use crate::fs::Node;
use crate::process::Task;

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
}

#[derive(Debug, Clone)]
pub(crate) enum Backing {
    /// Zeros until written. Also an ELF image's pages, which are mapped when it's loaded.
    Anon,
    /// A file's pages from `offset`. Shared mappings aren't written back yet, so for now they act as private ones.
    File { ino: layers::Ino, offset: u64 },
    /// Never the program's: what the backend keeps for itself.
    Reserved,
}

impl Region for Area {
    fn advanced(&self, by: u64) -> Area {
        let backing = match self.backing {
            Backing::File { ino, offset } => Backing::File { ino, offset: offset + by },
            ref other => other.clone(),
        };
        Area { prot: self.prot, backing, name: self.name.clone() }
    }
}

fn prot_from(bits: u64) -> Prot {
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
        let page = page_down(addr);
        let state = lock(&self.process.state);
        let (range, area) = state.regions.find(addr).map(|(r, a)| (r, a.clone())).ok_or(())?;
        if !area.prot.contains(access) {
            return Err(());
        }
        let file = match area.backing {
            Backing::Reserved => return Err(()),
            Backing::Anon => None,
            Backing::File { ino, offset } => Some((ino, offset + (page - range.start))),
        };
        let disk = file.map(|_| lock(&self.k.disk));
        let mut cache = file.map(|_| lock(&self.k.cache));
        cx.turn(|t| {
            if let Some(present) = t.space().page(page) {
                // Already mapped: a copy-on-write page being written; one another thread touched first, which is
                // there now, so the program just runs again; or nothing exec can fix.
                if access.write() && present.shared {
                    return t.space().make_writable(page, area.prot).map(|_| ());
                }
                if present.prot.contains(access) {
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mmap(&mut self, cx: &mut Cx<'_>, addr: u64, len: u64, prot: u64, flags: u64, fd: i32, offset: u64) -> Result<u64, Errno> {
        if len == 0 || !offset.is_multiple_of(PAGE) || !addr.is_multiple_of(PAGE) && flags & MAP_FIXED != 0 {
            return Err(EINVAL);
        }
        let len = page_up(len);
        let (backing, name) = if flags & MAP_ANONYMOUS != 0 {
            (Backing::Anon, String::new())
        } else {
            let open = self.file(fd)?;
            match open.node {
                Node::File(ino) => (Backing::File { ino, offset }, open.path.clone()),
                Node::Zero | Node::Null => (Backing::Anon, String::new()),
                _ => return Err(Errno(19)), // ENODEV
            }
        };
        let area = Area { prot: prot_from(prot), backing, name };
        let mut state = lock(&self.process.state);
        let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;
        let at = if fixed {
            if flags & MAP_FIXED_NOREPLACE != 0 && !state.regions.is_free(addr..addr + len) {
                return Err(EEXIST);
            }
            if state.regions.iter().any(|(r, a)| matches!(a.backing, Backing::Reserved) && r.start < addr + len && addr < r.end) {
                return Err(ENOMEM);
            }
            addr
        } else if addr != 0 && addr >= state.mmap.start && addr + len <= state.mmap.end && state.regions.is_free(addr..addr + len) {
            addr
        } else {
            state.regions.free_below(state.mmap.clone(), len, PAGE).ok_or(ENOMEM)?
        };
        state.regions.insert(at..at + len, area);
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
            // Forget the pages: the next touch fills them again, with zeros or from the file.
            let range = addr..addr + page_up(len);
            let state = lock(&self.process.state);
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
            state.regions.insert(old_top..new_top, Area { prot: Prot::RW, backing: Backing::Anon, name: "[heap]".to_string() });
        } else if new_top < old_top {
            state.regions.remove(new_top..old_top);
            cx.turn(|t| t.space().unmap(new_top..old_top));
        }
        state.brk.1 = wanted;
        Ok(wanted)
    }
}
