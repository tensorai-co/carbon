//! The part of translation that's the same in every direction.
//!
//! The code cache lives in guest memory, as pages core maps for it; this only keeps the books: which program addresses
//! have translations and where, which program pages they came from (so changing one forgets them), and how to map a
//! translated address back to the program's.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::ops::Range;

use crate::Error;
use crate::decode::{Decoder, Flow, Insn};

const PAGE: u64 = 4096;

/// One translated block.
#[derive(Debug, Clone)]
pub struct Block {
    /// Where it starts in the program.
    pub start: u64,
    /// Just past the last program byte it was translated from.
    pub end: u64,
    /// Where its translation is in the code cache.
    pub at: u64,
    pub len: u32,
    /// (offset in the translation, program address) per instruction.
    pub map: Vec<(u32, u64)>,
}

/// The books for one space's translations.
///
/// A block that's forgotten (its program bytes changed) can't be found by its program address any more, but its
/// translation stays where it is until the cache is emptied: a CPU still inside it finishes it, and its addresses
/// still map back to the program's.
#[derive(Default)]
pub struct Cache {
    /// The live blocks: program start → where its translation is.
    live: BTreeMap<u64, u64>,
    /// Every block placed since the cache was last emptied, live or forgotten, by where its translation is.
    placed: BTreeMap<u64, Block>,
    /// Program page → the live blocks translated from it.
    pages: BTreeMap<u64, BTreeSet<u64>>,
}

impl Cache {
    pub fn find(&self, start: u64) -> Option<&Block> {
        self.live.get(&start).and_then(|at| self.placed.get(at))
    }

    pub fn insert(&mut self, block: Block) {
        if let Some(old) = self.live.insert(block.start, block.at) {
            self.unindex(block.start, old);
        }
        for page in Self::pages_of(block.start, block.end) {
            self.pages.entry(page).or_default().insert(block.start);
        }
        self.placed.insert(block.at, block);
    }

    /// The program pages a block's bytes are on.
    fn pages_of(start: u64, end: u64) -> impl Iterator<Item = u64> {
        let last = end.max(start + 1) - 1;
        ((start / PAGE)..=(last / PAGE)).map(|page| page * PAGE)
    }

    fn unindex(&mut self, start: u64, at: u64) {
        let Some(block) = self.placed.get(&at) else { return };
        for page in Self::pages_of(block.start, block.end) {
            if let Some(starts) = self.pages.get_mut(&page) {
                starts.remove(&start);
                if starts.is_empty() {
                    self.pages.remove(&page);
                }
            }
        }
    }

    /// Whether any live block came from the program bytes in `range`.
    pub fn has_code(&self, range: Range<u64>) -> bool {
        let first = range.start & !(PAGE - 1);
        range.start < range.end && self.pages.range(first..range.end).next().is_some()
    }

    /// The program pages in `range` that live blocks came from.
    pub fn code_pages(&self, range: Range<u64>) -> Vec<u64> {
        let first = range.start & !(PAGE - 1);
        self.pages.range(first..range.end).map(|(page, _)| *page).collect()
    }

    /// The program changed (or lost) the bytes in `range`: every live block translated from a page of it is forgotten.
    /// Returns them, as (program start, where its translation is), for the runtime to unlink.
    pub fn forget(&mut self, range: Range<u64>) -> Vec<(u64, u64)> {
        let mut gone = Vec::new();
        for page in self.code_pages(range) {
            let Some(starts) = self.pages.get(&page).cloned() else { continue };
            for start in starts {
                if let Some(at) = self.live.remove(&start) {
                    self.unindex(start, at);
                    gone.push((start, at));
                }
            }
        }
        gone
    }

    /// The program wrote to a page it had run: its translations go. Returns how many.
    pub fn forget_page(&mut self, page: u64) -> usize {
        let page = page & !(PAGE - 1);
        self.forget(page..page + PAGE).len()
    }

    /// Where a translated address is in the program, as a fault or a preempt there sees it.
    pub fn program_pc(&self, host: u64) -> Option<u64> {
        let (&at, block) = self.placed.range(..=host).next_back()?;
        let offset = u32::try_from(host - at).ok()?;
        if offset >= block.len {
            return None;
        }
        block.map.iter().take_while(|(o, _)| *o <= offset).last().map(|(_, pc)| *pc)
    }

    /// The program address a translated address starts, if it's exactly where one of the program's instructions
    /// begins: there every instruction before it is done and none after it has started.
    pub fn boundary(&self, host: u64) -> Option<u64> {
        let (&at, block) = self.placed.range(..=host).next_back()?;
        let offset = u32::try_from(host - at).ok()?;
        block.map.iter().find(|(o, _)| *o == offset).map(|(_, pc)| *pc)
    }
}

/// Decodes a block from `start`: up to and including the first instruction that leaves it.
pub fn decode_block(decoder: &dyn Decoder, start: u64, bytes: &[u8], most: usize) -> Result<Vec<Insn>, Error> {
    let mut block = Vec::new();
    let mut offset = 0usize;
    while block.len() < most && offset < bytes.len() {
        let insn = decoder.decode(start + offset as u64, &bytes[offset..])?;
        offset += usize::from(insn.len);
        let ends = !matches!(insn.flow, Flow::Next);
        block.push(insn);
        if ends {
            break;
        }
    }
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::{Block, Cache};

    fn block(start: u64, end: u64, at: u64) -> Block {
        Block { start, end, at, len: 16, map: vec![(0, start), (8, start + 3)] }
    }

    #[test]
    fn forgetting_a_page_forgets_its_blocks_and_only_them() {
        let mut cache = Cache::default();
        cache.insert(block(0x1000, 0x1010, 0x10_0000));
        // Starts on one page and ends on the next.
        cache.insert(block(0x1ffc, 0x2004, 0x10_0100));
        cache.insert(block(0x3000, 0x3010, 0x10_0200));
        assert!(cache.has_code(0x2000..0x2001));
        let mut gone = cache.forget(0x2000..0x3000);
        gone.sort();
        assert_eq!(gone, vec![(0x1ffc, 0x10_0100)]);
        assert!(cache.find(0x1ffc).is_none());
        assert!(cache.find(0x1000).is_some() && cache.find(0x3000).is_some());
        assert!(!cache.has_code(0x2000..0x3000));
        // Its own first page still holds the block at 0x1000.
        assert!(cache.has_code(0x1000..0x2000));
        assert_eq!(cache.forget_page(0x1234), 1);
        assert!(cache.find(0x1000).is_none());
    }

    #[test]
    fn a_forgotten_block_still_maps_back() {
        let mut cache = Cache::default();
        cache.insert(block(0x1000, 0x1010, 0x10_0000));
        cache.forget(0x1000..0x2000);
        assert_eq!(cache.program_pc(0x10_0004), Some(0x1000));
        assert_eq!(cache.program_pc(0x10_000c), Some(0x1003));
        assert_eq!(cache.program_pc(0x10_0010), None);
        assert_eq!(cache.boundary(0x10_0008), Some(0x1003));
        assert_eq!(cache.boundary(0x10_0004), None);
    }

    #[test]
    fn translating_a_block_again_replaces_it() {
        let mut cache = Cache::default();
        cache.insert(block(0x1000, 0x1010, 0x10_0000));
        cache.insert(block(0x1000, 0x1010, 0x10_0100));
        assert_eq!(cache.find(0x1000).map(|b| b.at), Some(0x10_0100));
        assert_eq!(cache.forget(0x1000..0x1001), vec![(0x1000, 0x10_0100)]);
        // The old translation's addresses still map back.
        assert_eq!(cache.program_pc(0x10_0000), Some(0x1000));
    }
}
