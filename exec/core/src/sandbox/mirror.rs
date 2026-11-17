//! Keeping each CPU's stub's host mappings matching core's page tables.
//!
//! Every CPU has a mirror: the space its stub shows and the pages it shows there. A turn's changes to a space are
//! noted, as ranges, on every mirror that shows that space, and each stub is brought up to date by its own CPU's
//! thread as that CPU's turn ends. A CPU whose stub has fallen behind (another CPU changed its space) is stopped and
//! made to take a turn before it runs again (`Core::settle`), so no program ever runs on with a page that's gone.
//!
//! Bringing a stub up to date compares what core's tables say with what the stub shows, so a page that didn't change
//! costs the host nothing, and gives the host only what did: page by page for a shared stub (`Sandbox::mirror`), or
//! merged into as few runs as it can for a stub per CPU (`SandboxCpu::mirror`): pages next to each other on frames next
//! to each other are one mapping, a protection change is one `mprotect`, and taking a space out is one unmapping for
//! every stretch of it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use exec_mmu::{Format, Frame, Mmu, Native, PAGE, Prot};

use crate::contract::MirrorRun;

/// A page's guest-RAM offset and protection.
type Shown = (u64, Prot);

/// One page whose host mapping changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Change {
    pub page: u64,
    /// What it shows now, or nothing.
    pub now: Option<Shown>,
    /// What it showed before.
    pub before: Option<Shown>,
}

#[derive(Default)]
pub(crate) struct Mirror {
    /// The space the stub shows: 0 for none, `u64::MAX` for one that's gone.
    root: u64,
    /// The pages it shows.
    shown: BTreeMap<u64, Shown>,
    /// Ranges changed since the stub was last brought up to date.
    changed: Vec<Range<u64>>,
    /// Its CPU has been asked to stop and catch up, and hasn't yet.
    pub asked: bool,
    /// The host couldn't map something, so the stub doesn't match.
    pub broken: Option<String>,
}

impl Mirror {
    /// A turn changed `range` of a space.
    pub fn changed(&mut self, root: u64, range: Range<u64>) {
        if root == self.root && root != 0 {
            self.changed.push(range);
        }
    }

    /// A space has gone.
    pub fn forget(&mut self, root: u64) {
        if root == self.root && root != 0 {
            self.root = u64::MAX;
        }
    }

    /// Whether the stub may show something core's tables no longer say.
    pub fn behind(&self) -> bool {
        !self.changed.is_empty() || self.root == u64::MAX
    }

    /// Brings what the stub shows up to `root` as it is now, and says which pages changed, in order.
    pub fn apply(&mut self, mmu: &Mmu<Native>, root: u64) -> Vec<Change> {
        self.asked = false;
        let mut changes = Vec::new();
        if root != self.root {
            // Another space: everything it shows that the new one doesn't goes, and the new one's pages come.
            let mut target = BTreeMap::new();
            if root != 0 {
                mmu.visit(root, 0..u64::MAX, &mut |va, entry| {
                    target.insert(va, shown(mmu, entry));
                });
            }
            let pages: BTreeSet<u64> = self.shown.keys().chain(target.keys()).copied().collect();
            for page in pages {
                self.update(page, target.get(&page).copied(), &mut changes);
            }
            self.root = root;
            self.changed.clear();
            return changes;
        }
        let ranges = std::mem::take(&mut self.changed);
        if ranges.is_empty() || root == 0 {
            return changes;
        }
        let mut pages = BTreeSet::new();
        for range in ranges {
            pages.extend(self.shown.range(range.clone()).map(|(&page, _)| page));
            mmu.visit(root, range, &mut |va, _| {
                pages.insert(va);
            });
        }
        for page in pages {
            let entry = mmu.descriptor(root, page);
            let now = Native::is_valid(entry).then(|| shown(mmu, entry));
            self.update(page, now, &mut changes);
        }
        changes
    }

    fn update(&mut self, page: u64, now: Option<Shown>, changes: &mut Vec<Change>) {
        debug_assert!(page.is_multiple_of(PAGE));
        let before = match now {
            Some(now) => self.shown.insert(page, now),
            None => self.shown.remove(&page),
        };
        if before != now {
            changes.push(Change { page, now, before });
        }
    }

    /// `changes`, from `apply`, as few runs as they make: what a stub per CPU is given.
    pub fn runs(&self, changes: &[Change]) -> Vec<MirrorRun> {
        let mut runs: Vec<MirrorRun> = Vec::new();
        for change in changes {
            let page = change.page;
            let next = match (change.now, change.before) {
                (None, _) => MirrorRun::Unmap { page, len: PAGE },
                (Some((offset, prot)), Some((was, _))) if was == offset => MirrorRun::Protect { page, len: PAGE, prot },
                (Some((offset, prot)), _) => MirrorRun::Map { page, len: PAGE, offset, prot },
            };
            let merged = match (runs.last_mut(), next) {
                // Unmapping goes across a gap the stub shows nothing in.
                (Some(MirrorRun::Unmap { page: at, len }), MirrorRun::Unmap { page, .. })
                    if self.shown.range(*at + *len..page).next().is_none() =>
                {
                    *len = page + PAGE - *at;
                    true
                }
                (Some(MirrorRun::Protect { page: at, len, prot: was }), MirrorRun::Protect { page, prot, .. }) if *at + *len == page && *was == prot => {
                    *len += PAGE;
                    true
                }
                (Some(MirrorRun::Map { page: at, len, offset: from, prot: was }), MirrorRun::Map { page, offset, prot, .. })
                    if *at + *len == page && *from + *len == offset && *was == prot =>
                {
                    *len += PAGE;
                    true
                }
                _ => false,
            };
            if !merged {
                runs.push(next);
            }
        }
        runs
    }
}

fn shown(mmu: &Mmu<Native>, entry: u64) -> Shown {
    (mmu.offset(Frame(entry & Native::ADDRESS)), Native::prot(entry))
}
