//! A program's system calls, rewritten to skip the trap, where its backend has a jump gate (`Layout::jump_gate`: the
//! Linux sandbox, whose traps are signals to its stub).
//!
//! The third time a `syscall` stops lambda from the common seven bytes `mov $nr, %eax; syscall`, those bytes become
//! `jmp thunk; nop; nop`, and the thunk `mov $nr, %eax; lea rcx, [after]; jmp *[gate]`: the same stop, with the
//! registers `syscall` would leave (`rcx` the address after it, `r11` the flags), and nothing on the program's stack.
//! Thunks live in pages of their own (`[syscalls]` in `/proc/self/maps`), each within a jump's ±2 GiB of its sites.
//!
//! What keeps it safe:
//!
//! - **One thread.** A site is rewritten only while its process has a single thread, so no other CPU runs in its
//!   space and none can see the seven bytes half written. (A forked child has a space of its own.) A multithreaded
//!   process's calls keep their traps, until it's down to one thread again.
//! - **Private pages only.** The bytes are written as a debugger's poke is, to the process's own copy of the page, in
//!   an anonymous or private file mapping that's executable and not writable: never a shared mapping, never the vDSO,
//!   never code the program could be writing itself.
//! - **Unseen.** Reading the program's memory from outside (`ptrace`, `process_vm_readv`) shows the original bytes;
//!   writing it, or making the page writable, puts them back first. Unmapping or moving a site forgets it; unmapping a
//!   thunk page puts back every site that used it.
//! - **Equivalent.** Code that jumps to the `syscall` itself, past its `mov`, would land on the `nop`s: compilers and
//!   libcs don't make such jumps. `CARBON_SANDBOX_PATCH=0` turns rewriting off.

use std::collections::BTreeMap;
use std::ops::Range;

use kernels_core::thread::Cx;
use kernels_core::user;

use exec::Prot;

use super::{Area, Backing, page_down};
use crate::process::ProcState;

const PAGE: u64 = 4096;
/// `mov $nr, %eax; syscall`.
const SITE: u64 = 7;
/// Each thunk's room in its page; the page's first slot holds the gate's address.
const THUNK: u64 = 32;
/// How far a `jmp rel32` reaches, less a margin.
const REACH: u64 = (2 << 30) - (1 << 20);
/// The lowest a thunk page goes.
const LOWEST: u64 = 0x10000;
pub(crate) const NAME: &str = "[syscalls]";

/// A process's rewritten sites and thunk pages.
#[derive(Debug, Clone, Default)]
pub(crate) struct Patches {
    /// Each site: its seven original bytes, and its thunk.
    sites: BTreeMap<u64, ([u8; SITE as usize], u64)>,
    /// Each thunk page: where its next thunk goes.
    pages: BTreeMap<u64, u64>,
    /// Where `syscall`s that aren't sites to rewrite came from (`syscall()`'s, say), by the address after each.
    refused: std::collections::BTreeSet<u64>,
    /// How many times each place has trapped, until it's rewritten.
    traps: BTreeMap<u64, u8>,
}

/// The most places kept as refused, or counted.
const MOST_REFUSED: usize = 4096;
/// A site is rewritten at its third trap: rewriting costs a private copy of its page and a thunk, which a call made
/// once or twice (most of a short-lived process's) doesn't earn back.
const TRAPS_BEFORE: u8 = 3;

/// The bytes a site has, if it's one that can be rewritten for system call `nr`.
fn matches(bytes: &[u8], nr: u64) -> bool {
    bytes.len() == SITE as usize && bytes[0] == 0xb8 && bytes[5..7] == [0x0f, 0x05] && u64::from(u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]])) == nr
}

fn rel32(from_end: u64, to: u64) -> Option<[u8; 4]> {
    let delta = to.wrapping_sub(from_end) as i64;
    i32::try_from(delta).ok().map(i32::to_le_bytes)
}

/// Writes `bytes` to the process's own memory as a debugger's poke would: to its private copy of a page it may not
/// write.
fn poke(k: &crate::Kernel, state: &ProcState, cx: &mut Cx<'_>, at: u64, bytes: &[u8]) -> bool {
    user::with_faults(cx, true, |cx, a, access| super::fill(k, state, cx, a, access, true).is_ok(), |t| user::write(&mut t.space(), at, bytes)).is_ok()
}

/// Whether code in `area` may be rewritten.
fn rewritable(area: &Area) -> bool {
    area.prot.exec() && !area.prot.write() && !area.sealed && area.name != "[vdso]" && area.name != NAME && matches!(area.backing, Backing::Anon | Backing::File { .. })
}

/// Rewrites the site a `syscall` with number `nr` just stopped from, `after` being the address after it, if it can
/// be: the process has one thread, and its backend a jump gate at `gate`. Run in the process's own space.
pub(crate) fn rewrite(k: &crate::Kernel, state: &mut ProcState, cx: &mut Cx<'_>, gate: u64, after: u64, nr: u64) {
    let Some(site) = after.checked_sub(SITE) else { return };
    // One already rewritten (this call came by the jump gate), or a place found not to be a site before (its call
    // keeps its trap, as any other does), isn't read again.
    if state.patches.sites.contains_key(&site) || state.patches.refused.contains(&after) {
        return;
    }
    let traps = &mut state.patches.traps;
    if traps.len() >= MOST_REFUSED && !traps.contains_key(&after) {
        return;
    }
    let seen = traps.entry(after).or_insert(0);
    *seen += 1;
    if *seen < TRAPS_BEFORE {
        return;
    }
    traps.remove(&after);
    // The seven bytes on one page, which the program can't write, in a mapping that's its own.
    if page_down(site) != page_down(after - 1) || state.threads.len() != 1 || !state.regions.find(site).is_some_and(|(_, a)| rewritable(a)) {
        return;
    }
    let Ok(bytes) = cx.turn(|t| user::read_vec(&t.space(), site, SITE as usize)) else { return };
    if !matches(&bytes, nr) {
        if state.patches.refused.len() < MOST_REFUSED {
            state.patches.refused.insert(after);
        }
        return;
    }
    let mut patches = std::mem::take(&mut state.patches);
    if let Some(thunk) = patches.thunk(k, state, cx, gate, site, after, nr) {
        let jump = rel32(site + 5, thunk).map(|to| [0xe9, to[0], to[1], to[2], to[3], 0x90, 0x90]);
        if let Some(jump) = jump
            && poke(k, state, cx, site, &jump)
        {
            let mut original = [0; SITE as usize];
            original.copy_from_slice(&bytes);
            patches.sites.insert(site, (original, thunk));
        }
    }
    state.patches = patches;
}

impl Patches {
    /// A thunk for the site at `site`, written: in a thunk page within reach, or a new one.
    #[allow(clippy::too_many_arguments)]
    fn thunk(&mut self, k: &crate::Kernel, state: &mut ProcState, cx: &mut Cx<'_>, gate: u64, site: u64, after: u64, nr: u64) -> Option<u64> {
        let near = |page: u64| page.abs_diff(site) < REACH;
        let page = match self.pages.iter().find(|&(&page, &next)| near(page) && next + THUNK <= PAGE) {
            Some((&page, _)) => page,
            None => {
                let below = state.regions.free_below(site.saturating_sub(REACH).max(LOWEST)..page_down(site), PAGE, PAGE);
                let page = below.or_else(|| state.regions.free_above(page_down(site) + PAGE..(site + REACH).min(state.mmap.end), PAGE, PAGE))?;
                state.regions.insert(page..page + PAGE, Area::new(Prot::RX, Backing::Anon, NAME));
                if !poke(k, state, cx, page, &gate.to_le_bytes()) {
                    state.regions.remove(page..page + PAGE);
                    return None;
                }
                self.pages.insert(page, THUNK);
                page
            }
        };
        let thunk = page + self.pages[&page];
        let back = rel32(thunk + 12, after)?;
        let slot = rel32(thunk + 18, page)?;
        // mov eax, nr; lea rcx, [rip + back]; jmp [rip + slot]; then int3s
        let mut code = vec![0xb8];
        code.extend((nr as u32).to_le_bytes());
        code.extend([0x48, 0x8d, 0x0d]);
        code.extend(back);
        code.extend([0xff, 0x25]);
        code.extend(slot);
        code.resize(THUNK as usize, 0xcc);
        if !poke(k, state, cx, thunk, &code) {
            return None;
        }
        *self.pages.get_mut(&page)? += THUNK;
        Some(thunk)
    }

    /// Shows the original bytes, in `bytes` read from `at`, wherever a site was rewritten.
    pub(crate) fn unseen(&self, at: u64, bytes: &mut [u8]) {
        let end = at + bytes.len() as u64;
        for (&site, (original, _)) in self.sites.range(at.saturating_sub(SITE - 1)..end) {
            for (i, &b) in original.iter().enumerate() {
                let addr = site + i as u64;
                if (at..end).contains(&addr) {
                    bytes[(addr - at) as usize] = b;
                }
            }
        }
    }
}

/// Puts the original bytes back at every site in `range` that's still there to write (before it's written, made
/// writable or moved), and at every site whose thunk is in a page in `range`; forgets those sites, and the thunk
/// pages in `range`.
pub(crate) fn restore(k: &crate::Kernel, state: &mut ProcState, cx: &mut Cx<'_>, range: Range<u64>) {
    let p = &state.patches;
    let sites_in = p.sites.range(range.start.saturating_sub(SITE - 1)..range.end).next().is_some();
    let pages_in = p.pages.range(range.start.saturating_sub(PAGE - 1)..range.end).next().is_some();
    if !sites_in && !pages_in {
        return;
    }
    let patches = std::mem::take(&mut state.patches);
    let mut kept = Patches::default();
    let in_range = |a: u64, len: u64| a < range.end && range.start < a + len;
    for (site, (original, thunk)) in patches.sites {
        if in_range(site, SITE) || in_range(page_down(thunk), PAGE) {
            poke(k, state, cx, site, &original);
        } else {
            kept.sites.insert(site, (original, thunk));
        }
    }
    kept.pages = patches.pages.into_iter().filter(|&(page, _)| !in_range(page, PAGE)).collect();
    kept.refused = patches.refused.into_iter().filter(|&at| !in_range(at - SITE, SITE)).collect();
    kept.traps = patches.traps;
    state.patches = kept;
}

/// Forgets the sites in `range`, which is going (unmapped, or mapped over), and puts back those whose thunk page is in
/// it.
pub(crate) fn forget(k: &crate::Kernel, state: &mut ProcState, cx: &mut Cx<'_>, range: Range<u64>) {
    if state.patches.sites.is_empty() {
        return;
    }
    let gone: Vec<u64> = state.patches.sites.range(range.start.saturating_sub(SITE - 1)..range.end).map(|(&s, _)| s).collect();
    for site in gone {
        state.patches.sites.remove(&site);
    }
    state.patches.refused.retain(|&at| !(at < range.end + SITE && range.start < at));
    // Thunk pages in the range: their sites, elsewhere, are put back.
    let pages: Vec<u64> = state.patches.pages.keys().copied().filter(|&p| p < range.end && range.start < p + PAGE).collect();
    for page in pages {
        restore(k, state, cx, page..page + PAGE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_mov_eax_then_syscall_with_the_same_number() {
        assert!(matches(&[0xb8, 39, 0, 0, 0, 0x0f, 0x05], 39));
        assert!(!matches(&[0xb8, 39, 0, 0, 0, 0x0f, 0x05], 40));
        assert!(!matches(&[0xb9, 39, 0, 0, 0, 0x0f, 0x05], 39));
        assert!(!matches(&[0xb8, 39, 0, 0, 0, 0xcd, 0x80], 39));
    }

    #[test]
    fn reads_see_the_original_bytes() {
        let mut p = Patches::default();
        p.sites.insert(0x1003, ([0xb8, 39, 0, 0, 0, 0x0f, 0x05], 0x2000));
        let mut bytes = [0xe9, 1, 2, 3, 4, 0x90, 0x90, 0xaa];
        p.unseen(0x1003, &mut bytes);
        assert_eq!(bytes, [0xb8, 39, 0, 0, 0, 0x0f, 0x05, 0xaa]);
        // A read that starts inside the site.
        let mut bytes = [0x90, 0x90, 0xaa];
        p.unseen(0x1008, &mut bytes);
        assert_eq!(bytes, [0x0f, 0x05, 0xaa]);
    }
}
