//! A map of an address space's regions. Each kernel keeps one per process with its own rules (`mmap`'s for lambda,
//! `VirtualAlloc`'s for atlas) and its own `R` for what a region is. exec holds what's actually mapped; this holds
//! what the program was promised, so a fault inside a region is the kernel's to fill, and outside one is the program's.

use std::collections::BTreeMap;
use std::ops::Range;

/// What a region is, to its kernel.
pub trait Region: Clone {
    /// This region's description for the part of it that starts `by` bytes in: a file mapping's offset moves along.
    fn advanced(&self, by: u64) -> Self;
}

#[derive(Debug, Clone)]
pub struct Regions<R> {
    map: BTreeMap<u64, (u64, R)>,
}

impl<R: Region> Default for Regions<R> {
    fn default() -> Regions<R> {
        Regions { map: BTreeMap::new() }
    }
}

impl<R: Region> Regions<R> {
    /// The region `addr` is in.
    pub fn find(&self, addr: u64) -> Option<(Range<u64>, &R)> {
        let (start, (end, region)) = self.map.range(..=addr).next_back()?;
        (addr < *end).then_some((*start..*end, region))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Range<u64>, &R)> {
        self.map.iter().map(|(start, (end, region))| (*start..*end, region))
    }

    /// Whether nothing is in `range`.
    pub fn is_free(&self, range: Range<u64>) -> bool {
        self.map.range(..range.end).next_back().is_none_or(|(_, (end, _))| *end <= range.start)
    }

    /// Cuts the regions at `at`, so that one starts there if any covers it.
    fn split(&mut self, at: u64) {
        if let Some((start, (end, region))) = self.map.range(..at).next_back().map(|(s, (e, r))| (*s, (*e, r.clone())))
            && at < end
        {
            self.map.insert(start, (at, region.clone()));
            self.map.insert(at, (end, region.advanced(at - start)));
        }
    }

    /// Takes everything in `range` out, giving what was there.
    pub fn remove(&mut self, range: Range<u64>) -> Vec<(Range<u64>, R)> {
        self.split(range.start);
        self.split(range.end);
        let starts: Vec<u64> = self.map.range(range.start..range.end).map(|(s, _)| *s).collect();
        starts.into_iter().filter_map(|s| self.map.remove(&s).map(|(e, r)| (s..e, r))).collect()
    }

    /// Puts `region` at `range`, replacing whatever was there.
    pub fn insert(&mut self, range: Range<u64>, region: R) {
        self.remove(range.clone());
        self.map.insert(range.start, (range.end, region));
    }

    /// Changes every region in `range`, cut at its edges.
    pub fn update(&mut self, range: Range<u64>, mut change: impl FnMut(&mut R)) {
        self.split(range.start);
        self.split(range.end);
        for (_, (_, region)) in self.map.range_mut(range.start..range.end) {
            change(region);
        }
    }

    /// The highest free `len` bytes, aligned to `align`, within `within`.
    pub fn free_below(&self, within: Range<u64>, len: u64, align: u64) -> Option<u64> {
        let mut top = within.end;
        for (start, (end, _)) in self.map.range(..within.end).rev() {
            if let Some(at) = fit(*end.max(&within.start)..top, len, align) {
                return Some(at);
            }
            top = top.min(*start);
            if top <= within.start {
                return None;
            }
        }
        fit(within.start..top, len, align)
    }

    /// The lowest free `len` bytes, aligned to `align`, within `within`.
    pub fn free_above(&self, within: Range<u64>, len: u64, align: u64) -> Option<u64> {
        let mut bottom = within.start.next_multiple_of(align);
        loop {
            let end = bottom.checked_add(len)?;
            if end > within.end {
                return None;
            }
            match self.map.range(..end).next_back() {
                Some((start, (stop, _))) if *stop > bottom => bottom = stop.max(start).next_multiple_of(align),
                _ => return Some(bottom),
            }
        }
    }
}

/// The highest `len` bytes aligned to `align` inside `gap`.
fn fit(gap: Range<u64>, len: u64, align: u64) -> Option<u64> {
    let at = gap.end.checked_sub(len)? / align * align;
    (at >= gap.start && gap.end > gap.start).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct File(u64);
    impl Region for File {
        fn advanced(&self, by: u64) -> File {
            File(self.0 + by)
        }
    }

    #[test]
    fn splits_and_finds_room() {
        let mut regions = Regions::default();
        regions.insert(0x1000..0x5000, File(0));
        regions.insert(0x2000..0x3000, File(100));
        assert_eq!(regions.find(0x1800).unwrap().0, 0x1000..0x2000);
        assert_eq!(*regions.find(0x3800).unwrap().1, File(0x2000));
        assert_eq!(regions.find(0x5000), None);
        let removed = regions.remove(0x2800..0x4000);
        assert_eq!(removed.len(), 2);
        assert!(regions.is_free(0x2800..0x4000));
        assert!(!regions.is_free(0x2000..0x2900));
        assert_eq!(regions.free_below(0..0x10000, 0x1000, 0x1000), Some(0xf000));
        assert_eq!(regions.free_below(0..0x5000, 0x1000, 0x1000), Some(0x3000));
        assert_eq!(regions.free_below(0..0x5000, 0x2000, 0x1000), None);
        assert_eq!(regions.free_above(0x1000..0x10000, 0x1000, 0x1000), Some(0x3000));
    }
}
