//! Keeping a stub's host mappings matching core's page tables.
//!
//! The stub has one address space, so it shows one space at a time: switching spaces takes everything out and puts
//! the new space in. Within a space, every change a turn makes is noted as a range, and the stub is brought up to date
//! when the turn ends, page by page. The host can map in larger pieces than a page; frames are spaced so each page's
//! bytes start a piece of their own (`GuestRam::stride`), and the host's `mirror` maps each.

use std::collections::BTreeSet;
use std::ops::Range;

use exec_mmu::{Format, Frame, Mmu, Native, PAGE, Prot};

use crate::contract::Sandbox;

#[derive(Default)]
pub(crate) struct Mirror {
    /// The space the stub shows.
    root: u64,
    /// The pages it shows.
    shown: BTreeSet<u64>,
    /// Ranges changed since the stub was last brought up to date.
    changed: Vec<Range<u64>>,
    /// The host couldn't map something, so the stub doesn't match.
    pub broken: Option<String>,
}

impl Mirror {
    /// A turn changed `range` of a space.
    pub fn changed(&mut self, root: u64, range: Range<u64>) {
        if root == self.root {
            self.changed.push(range);
        }
    }

    /// A space has gone.
    pub fn forget(&mut self, root: u64) {
        if root == self.root {
            self.root = u64::MAX;
        }
    }

    /// Makes the stub show `root` as it is now.
    pub fn apply<S: Sandbox>(&mut self, sandbox: &mut S, mmu: &Mmu<Native>, root: u64) {
        if root != self.root {
            for page in std::mem::take(&mut self.shown) {
                self.show(sandbox, page, None, Prot::NONE);
            }
            self.root = root;
            self.changed.clear();
            if root != 0 {
                let mut pages = Vec::new();
                mmu.visit(root, 0..u64::MAX, &mut |va, entry| pages.push((va, entry)));
                for (va, entry) in pages {
                    self.show_entry(sandbox, mmu, va, entry);
                }
            }
            return;
        }
        for range in std::mem::take(&mut self.changed) {
            let mut pages: BTreeSet<u64> = self.shown.range(range.clone()).copied().collect();
            mmu.visit(root, range, &mut |va, _| {
                pages.insert(va);
            });
            for page in pages {
                let entry = mmu.descriptor(root, page);
                if Native::is_valid(entry) {
                    self.show_entry(sandbox, mmu, page, entry);
                } else if self.shown.remove(&page) {
                    self.show(sandbox, page, None, Prot::NONE);
                }
            }
        }
    }

    fn show_entry<S: Sandbox>(&mut self, sandbox: &mut S, mmu: &Mmu<Native>, page: u64, entry: u64) {
        let offset = mmu.offset(Frame(entry & Native::ADDRESS));
        self.show(sandbox, page, Some(offset), Native::prot(entry));
        self.shown.insert(page);
    }

    fn show<S: Sandbox>(&mut self, sandbox: &mut S, page: u64, offset: Option<u64>, prot: Prot) {
        debug_assert!(page.is_multiple_of(PAGE));
        if let Err(why) = sandbox.mirror(page, offset, prot) {
            self.broken.get_or_insert(why);
        }
    }
}
