//! Where a disk's bytes go: chunks of file contents, named by what's in them, and the machine's log. What's behind a
//! store is the host's business (`images` keeps one on the host's disk); `layers` only asks it for bytes.

use alloc::collections::BTreeMap;
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
}

/// A store in memory: for tests, and for disks that needn't outlive the machine.
#[derive(Default, Clone)]
pub struct MemoryStore {
    chunks: BTreeMap<Hash, Vec<u8>>,
    log: Vec<u8>,
    /// How much of the log survives a crash, for tests.
    pub synced: usize,
}

impl MemoryStore {
    pub fn chunks(&self) -> usize {
        self.chunks.len()
    }

    pub fn log(&self) -> &[u8] {
        &self.log
    }

    /// What a crash would leave: the log as far as it was synced.
    pub fn crashed(&self) -> MemoryStore {
        MemoryStore { chunks: self.chunks.clone(), log: self.log[..self.synced].to_vec(), synced: self.synced }
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
        self.chunks.entry(hash).or_insert_with(|| data.to_vec());
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
}
