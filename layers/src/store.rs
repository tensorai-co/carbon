//! Where a disk's bytes go: chunks of file contents, named by what's in them, and the machine's log. What's behind a
//! store is the host's business (`images` keeps one on the host's disk); `layers` only asks it for bytes.
//!
//! A machine's store may keep the chunks the machine writes as its own, apart from the chunks images share: nothing
//! else can name those, so once the machine's files and its log stop naming one, it can be freed, as deleting a file
//! frees its space on any disk. Freezing the machine into an image shares the chunks the image names.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::{Error, Result};

/// A chunk's name, from its contents.
pub type Hash = [u8; 32];

/// Files are kept in chunks of this size, aligned to their offset in the file.
pub const CHUNK: u64 = 64 * 1024;

pub trait Store: Send {
    /// A chunk's contents.
    fn get(&mut self, hash: &Hash) -> Result<Vec<u8>>;
    /// Keeps a chunk, unless it's there already, and gives its name.
    fn put(&mut self, data: &[u8]) -> Result<Hash>;
    /// Adds to the end of the machine's log.
    fn log_append(&mut self, data: &[u8]) -> Result<()>;
    /// The whole log, as it is.
    fn log_read(&mut self) -> Result<Vec<u8>>;
    /// Drops the end of the log, from `len` on: what a crash left half-written.
    fn log_truncate(&mut self, len: u64) -> Result<()>;
    /// Makes what's in the log, and every chunk it names, survive a crash.
    fn log_sync(&mut self) -> Result<()>;
    /// The size of the disk the store is on, and how much of it is free, in bytes: what `df` shows for a machine's
    /// disk. None for a store that isn't on a disk (one in memory), whose machine shows its memory instead.
    fn space(&mut self) -> Option<(u64, u64)> {
        None
    }
    /// The chunks this store keeps as the machine's own: ones it wrote that no image names. None for a store that
    /// shares every chunk it keeps.
    fn own_chunks(&mut self) -> Vec<Hash> {
        Vec::new()
    }
    /// Frees one of the machine's own chunks, which neither its files nor anything in its log names any more.
    fn free(&mut self, _hash: &Hash) -> Result<()> {
        Ok(())
    }
    /// Shares the machine's own chunks among `hashes` (the rest are shared already), as freezing does with every chunk
    /// the image names: from then on they're the image's too, and never freed with the machine's files.
    fn share(&mut self, _hashes: &[Hash]) -> Result<()> {
        Ok(())
    }
}

/// A store in memory: for tests, and for disks that needn't outlive the machine.
#[derive(Default, Clone)]
pub struct MemoryStore {
    chunks: BTreeMap<Hash, Vec<u8>>,
    /// With [`MemoryStore::machine`], the chunks it keeps as the machine's own.
    own: Option<BTreeSet<Hash>>,
    log: Vec<u8>,
    /// How much of the log survives a crash, for tests.
    pub synced: usize,
}

impl MemoryStore {
    /// A machine's store, which keeps the chunks it's given as the machine's own, until they're shared.
    pub fn machine() -> MemoryStore {
        MemoryStore { own: Some(BTreeSet::new()), ..MemoryStore::default() }
    }

    pub fn chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn log(&self) -> &[u8] {
        &self.log
    }

    /// What a crash would leave: the log as far as it was synced.
    pub fn crashed(&self) -> MemoryStore {
        MemoryStore { chunks: self.chunks.clone(), own: self.own.clone(), log: self.log[..self.synced].to_vec(), synced: self.synced }
    }

    /// A name for telling apart what's in one machine, not anything trying to collide.
    pub fn name(data: &[u8]) -> Hash {
        let mut hash = [0u8; 32];
        for (i, slot) in hash.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (i as u64).wrapping_mul(0x1000_0000_01b3);
            for byte in data {
                h = (h ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
            }
            slot.copy_from_slice(&h.to_le_bytes());
        }
        hash
    }
}

impl Store for MemoryStore {
    fn get(&mut self, hash: &Hash) -> Result<Vec<u8>> {
        self.chunks.get(hash).cloned().ok_or(Error::Io)
    }

    fn put(&mut self, data: &[u8]) -> Result<Hash> {
        let hash = Self::name(data);
        if let alloc::collections::btree_map::Entry::Vacant(slot) = self.chunks.entry(hash) {
            slot.insert(data.to_vec());
            if let Some(own) = &mut self.own {
                own.insert(hash);
            }
        }
        Ok(hash)
    }

    fn log_append(&mut self, data: &[u8]) -> Result<()> {
        self.log.extend_from_slice(data);
        Ok(())
    }

    fn log_read(&mut self) -> Result<Vec<u8>> {
        Ok(self.log.clone())
    }

    fn log_truncate(&mut self, len: u64) -> Result<()> {
        self.log.truncate(len as usize);
        self.synced = self.synced.min(self.log.len());
        Ok(())
    }

    fn log_sync(&mut self) -> Result<()> {
        self.synced = self.log.len();
        Ok(())
    }

    fn own_chunks(&mut self) -> Vec<Hash> {
        self.own.iter().flatten().copied().collect()
    }

    fn free(&mut self, hash: &Hash) -> Result<()> {
        if self.own.as_mut().is_some_and(|own| own.remove(hash)) {
            self.chunks.remove(hash);
        }
        Ok(())
    }

    fn share(&mut self, hashes: &[Hash]) -> Result<()> {
        if let Some(own) = &mut self.own {
            for hash in hashes {
                own.remove(hash);
            }
        }
        Ok(())
    }
}
