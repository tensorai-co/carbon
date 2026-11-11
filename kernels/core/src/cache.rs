//! The page cache: one frame for each page of each file, however many processes map it. A kernel maps a cached frame
//! copy-on-write, so a write makes a private copy and everyone else keeps this one. A file that's written to has a new
//! generation, which is part of the key, so nothing stale can be found.
//!
//! The cache holds the frames. Frames nothing maps any more go back, least recently used first, when it's trimmed.

use std::collections::HashMap;

use exec::{Error, Frame, Memory, PAGE};

/// One page of one file, as it was at one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// The kernel's own number for the file: a `layers` inode, or anything else that names it.
    pub file: u64,
    pub generation: u64,
    /// Which page of the file.
    pub page: u64,
}

#[derive(Default)]
pub struct PageCache {
    pages: HashMap<Key, (Frame, u64)>,
    clock: u64,
}

impl PageCache {
    /// The frame for `key`, filling a new one with `fill` (a page of zeros to fill in) if it isn't cached.
    pub fn get(&mut self, memory: &mut Memory<'_>, key: Key, fill: impl FnOnce(&mut [u8])) -> Result<Frame, Error> {
        self.clock += 1;
        if let Some((frame, used)) = self.pages.get_mut(&key) {
            *used = self.clock;
            return Ok(*frame);
        }
        let frame = memory.new_frame()?;
        let mut page = vec![0; PAGE as usize];
        fill(&mut page);
        memory.frame_write(frame, 0, &page);
        self.pages.insert(key, (frame, self.clock));
        Ok(frame)
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Gives back frames nothing maps, oldest first, until at most `keep` pages are cached.
    pub fn trim(&mut self, memory: &mut Memory<'_>, keep: usize) {
        if self.pages.len() <= keep {
            return;
        }
        let mut by_age: Vec<(u64, Key, Frame)> = self.pages.iter().map(|(k, (f, used))| (*used, *k, *f)).collect();
        by_age.sort_unstable_by_key(|(used, ..)| *used);
        for (_, key, frame) in by_age {
            if self.pages.len() <= keep {
                break;
            }
            if !memory.frame_is_mapped(frame) {
                self.pages.remove(&key);
                memory.free_frame(frame);
            }
        }
    }

    /// Forgets every page of `file` (it's gone), giving back those nothing maps.
    pub fn forget(&mut self, memory: &mut Memory<'_>, file: u64) {
        let keys: Vec<Key> = self.pages.keys().filter(|k| k.file == file).copied().collect();
        for key in keys {
            if let Some((frame, _)) = self.pages.remove(&key)
                && !memory.frame_is_mapped(frame)
            {
                memory.free_frame(frame);
            }
        }
    }
}
