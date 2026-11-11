//! What memory is being used for: per space, per range, and across the machine.

use std::collections::BTreeSet;
use std::ops::Range;

use exec_mmu::{Format, Native, PAGE};

use crate::contract::{Sandbox, Vm};
use crate::exec::Core;
use crate::{MemoryStats, PageCensus, Resident, SpaceUsage};

impl<V: Vm, S: Sandbox> Core<V, S> {
    pub(crate) fn stats(&self) -> MemoryStats {
        let mut tables = 0;
        for root in self.spaces.keys() {
            self.mmu.walk(*root, &mut |_, _, _| {}, &mut tables);
        }
        let frames = &self.mmu.frames;
        MemoryStats {
            limit: frames.limit_bytes(),
            touched: frames.touched_bytes(),
            free: frames.free_bytes(),
            given_back: frames.given_back_bytes(),
            tables: tables * PAGE,
        }
    }

    pub(crate) fn usage(&self, root: u64) -> SpaceUsage {
        let mut usage = SpaceUsage::default();
        if !self.spaces.contains_key(&root) {
            return usage;
        }
        let mut tables = 0;
        self.mmu.walk(
            root,
            &mut |_, frame, entry| {
                let prot = Native::prot(entry);
                if prot.exec() {
                    usage.code += PAGE;
                } else if prot.write() {
                    usage.writable += PAGE;
                } else {
                    usage.read_only += PAGE;
                }
                if self.mmu.refs(frame) > 1 {
                    usage.shared += PAGE;
                }
            },
            &mut tables,
        );
        usage.tables = tables * PAGE;
        usage
    }

    pub(crate) fn resident(&self, root: u64, range: Range<u64>) -> Resident {
        let mut resident = Resident::default();
        self.mmu.visit(root, range, &mut |_, entry| {
            resident.mapped += 1;
            if entry & Native::SHARED != 0 {
                resident.shared += 1;
            }
        });
        resident
    }

    pub(crate) fn census(&self) -> PageCensus {
        let mut census = PageCensus::default();
        let mut frames = BTreeSet::new();
        let mut contents = BTreeSet::new();
        for root in self.spaces.keys() {
            let mut tables = 0;
            self.mmu.walk(
                *root,
                &mut |_, frame, _| {
                    census.mapped += PAGE;
                    if frames.insert(frame) {
                        let hash = self.mmu.hash_frame(frame);
                        if hash == 0 {
                            census.zero += PAGE;
                        }
                        contents.insert(hash);
                    }
                },
                &mut tables,
            );
        }
        census.frames = frames.len() as u64 * PAGE;
        census.contents = contents.len() as u64 * PAGE;
        census
    }
}
