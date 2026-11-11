//! Which frames of guest memory are in use, and by how many: reference counts, a free list, and, per granule (the
//! host's own page), how many of its frames are used, so a granule with none can go back to the host.

use crate::{Frame, OutOfMemory, PAGE};

/// A run of whole granules, nothing in which is used, that the host could have back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub start: Frame,
    pub bytes: u64,
}

pub struct Frames {
    base: u64,
    pages: u64,
    granule_pages: u64,
    refs: Vec<u32>,
    /// Frames that have been used and freed, by index.
    free: Vec<u32>,
    /// Frames from here on have never been used.
    next: u64,
    /// In use, per granule.
    used: Vec<u32>,
    /// Whether the host has backed each granule, as far as anyone knows.
    touched: Vec<bool>,
    /// Granules touched, and of those, ones nothing uses: kept as they change, so a turn's end needn't count.
    touched_count: u64,
    idle_count: u64,
    in_use: u64,
    given_back: u64,
}

impl Frames {
    /// `bytes` of guest memory from `base`, which the host backs in granules of `granule` bytes.
    pub fn new(base: u64, bytes: u64, granule: u64) -> Frames {
        let pages = bytes / PAGE;
        let granule_pages = (granule / PAGE).max(1);
        let granules = pages.div_ceil(granule_pages) as usize;
        Frames {
            base,
            pages,
            granule_pages,
            refs: vec![0; pages as usize],
            free: Vec::new(),
            next: 0,
            used: vec![0; granules],
            touched: vec![false; granules],
            touched_count: 0,
            idle_count: 0,
            in_use: 0,
            given_back: 0,
        }
    }

    fn index(&self, frame: Frame) -> usize {
        debug_assert!(self.contains(frame));
        ((frame.0 - self.base) / PAGE) as usize
    }

    fn granule(&self, index: usize) -> usize {
        index / self.granule_pages as usize
    }

    pub fn contains(&self, frame: Frame) -> bool {
        frame.0 >= self.base && frame.0 < self.base + self.pages * PAGE
    }

    /// A frame for one holder. Its contents are whatever they were: `Mmu::new_frame` zeroes it.
    pub fn take(&mut self) -> Result<Frame, OutOfMemory> {
        let index = if let Some(index) = self.free.pop() {
            index as usize
        } else if self.next < self.pages {
            self.next += 1;
            (self.next - 1) as usize
        } else {
            return Err(OutOfMemory);
        };
        self.refs[index] = 1;
        let granule = self.granule(index);
        if self.used[granule] == 0 && self.touched[granule] {
            self.idle_count -= 1;
        }
        self.used[granule] += 1;
        if !self.touched[granule] {
            self.touched[granule] = true;
            self.touched_count += 1;
        }
        self.in_use += 1;
        Ok(Frame(self.base + index as u64 * PAGE))
    }

    /// One more holder.
    pub fn hold(&mut self, frame: Frame) {
        let index = self.index(frame);
        self.refs[index] += 1;
    }

    /// One fewer holder. True if that was the last, and the frame is free again.
    pub fn release(&mut self, frame: Frame) -> bool {
        let index = self.index(frame);
        if self.refs[index] == 0 {
            return false;
        }
        self.refs[index] -= 1;
        if self.refs[index] > 0 {
            return false;
        }
        self.free.push(index as u32);
        let granule = self.granule(index);
        self.used[granule] -= 1;
        if self.used[granule] == 0 && self.touched[granule] {
            self.idle_count += 1;
        }
        self.in_use -= 1;
        true
    }

    pub fn refs(&self, frame: Frame) -> u32 {
        self.refs[self.index(frame)]
    }

    /// Runs of granules that nothing uses and the host has backed, keeping at least `keep` bytes of free memory
    /// backed, and at most `most` runs.
    pub fn excess(&self, keep: u64, most: usize) -> Vec<Run> {
        let granule_bytes = self.granule_pages * PAGE;
        let mut spare = self.free_bytes();
        let mut runs: Vec<Run> = Vec::new();
        if self.idle_count == 0 || spare < keep + granule_bytes {
            return runs;
        }
        for (granule, (&used, &touched)) in self.used.iter().zip(&self.touched).enumerate() {
            if used != 0 || !touched {
                continue;
            }
            if spare < keep + granule_bytes {
                break;
            }
            spare -= granule_bytes;
            let start = Frame(self.base + granule as u64 * granule_bytes);
            let joins = runs.last().is_some_and(|last| last.start.0 + last.bytes == start.0);
            if joins {
                if let Some(last) = runs.last_mut() {
                    last.bytes += granule_bytes;
                }
            } else if runs.len() == most {
                break;
            } else {
                runs.push(Run { start, bytes: granule_bytes });
            }
        }
        runs
    }

    /// The host has had a run back.
    pub fn returned(&mut self, run: Run) {
        let granule_bytes = self.granule_pages * PAGE;
        let first = ((run.start.0 - self.base) / granule_bytes) as usize;
        for granule in first..first + (run.bytes / granule_bytes) as usize {
            if self.touched[granule] {
                self.touched[granule] = false;
                self.touched_count -= 1;
                if self.used[granule] == 0 {
                    self.idle_count -= 1;
                }
            }
        }
        self.given_back += run.bytes;
    }

    pub fn limit_bytes(&self) -> u64 {
        self.pages * PAGE
    }

    /// What the host has backed.
    pub fn touched_bytes(&self) -> u64 {
        self.touched_count * self.granule_pages * PAGE
    }

    /// Backed, with nothing in use in them: the most `excess` could ever give back.
    pub fn idle_bytes(&self) -> u64 {
        self.idle_count * self.granule_pages * PAGE
    }

    pub fn in_use_bytes(&self) -> u64 {
        self.in_use * PAGE
    }

    /// Backed but not in use.
    pub fn free_bytes(&self) -> u64 {
        self.touched_bytes().saturating_sub(self.in_use_bytes())
    }

    /// What can still be handed out, backed or not.
    pub fn available_bytes(&self) -> u64 {
        (self.pages - self.in_use) * PAGE
    }

    pub fn given_back_bytes(&self) -> u64 {
        self.given_back
    }
}

#[cfg(test)]
mod tests {
    use super::{Frames, PAGE};

    /// The counts kept as frames come and go equal a recount, through takes, releases and give-backs.
    #[test]
    fn kept_counts_match_a_recount() {
        let granule = 4 * PAGE;
        let mut frames = Frames::new(0x1000_0000, 64 * granule, granule);
        let mut held = Vec::new();
        let mut seed = 12345u64;
        for step in 0..20_000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let roll = seed >> 33;
            if !roll.is_multiple_of(3) || held.is_empty() {
                if let Ok(frame) = frames.take() {
                    held.push(frame);
                }
            } else {
                let frame = held.swap_remove((roll as usize / 3) % held.len());
                frames.release(frame);
            }
            if step % 97 == 0 {
                for run in frames.excess(0, 8) {
                    frames.returned(run);
                }
            }
            let touched = frames.touched.iter().filter(|t| **t).count() as u64;
            let idle = frames.used.iter().zip(&frames.touched).filter(|(u, t)| **u == 0 && **t).count() as u64;
            assert_eq!(frames.touched_count, touched, "step {step}");
            assert_eq!(frames.idle_count, idle, "step {step}");
        }
    }
}
