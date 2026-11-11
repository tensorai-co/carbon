//! The chunk store on the host's disk, and a machine's log beside it: `layers::Store`, in files.
//!
//! ```text
//! <home>/chunks/ab/cdef…   one file per chunk, named by its SHA-256, so each is kept once however many images share it
//! <home>/machines/<name>/log   a machine's log: its writable layer
//! ```
//!
//! A chunk is written to a temporary file and renamed into place, so a crash never leaves half of one under its name.
//! Chunks written since the last sync are synced with the log, as `layers` expects: a record never names a chunk that
//! didn't survive.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use layers::{Error, Hash, Result};
use sha2::{Digest, Sha256};

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}

fn unhex(text: &str) -> Option<Hash> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// A hash from its hex, as image records keep it.
pub fn parse_hash(text: &str) -> Option<Hash> {
    unhex(text)
}

fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|_| Error::Io)
}

pub struct DiskStore {
    chunks: PathBuf,
    log: Option<File>,
    /// Chunks written since the last sync, to sync with the log.
    unsynced: Vec<PathBuf>,
}

impl DiskStore {
    /// Chunks only, in `chunks`: for building an image, which logs nothing.
    pub fn chunks(chunks: PathBuf) -> std::io::Result<DiskStore> {
        fs::create_dir_all(&chunks)?;
        Ok(DiskStore { chunks, log: None, unsynced: Vec::new() })
    }

    /// Chunks in `chunks`, and the log at `log`, made if it isn't there.
    pub fn with_log(chunks: PathBuf, log: &Path) -> std::io::Result<DiskStore> {
        let mut store = DiskStore::chunks(chunks)?;
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)?;
        }
        // Written at its end, not opened to append: a file opened to append can't be cut short on Windows.
        store.log = Some(OpenOptions::new().read(true).write(true).create(true).truncate(false).open(log)?);
        Ok(store)
    }

    fn path(&self, hash: &Hash) -> PathBuf {
        let name = hex(hash);
        self.chunks.join(&name[..2]).join(&name[2..])
    }

    /// Whether a chunk is here.
    pub fn has(&self, hash: &Hash) -> bool {
        self.path(hash).exists()
    }

    fn log(&mut self) -> Result<&mut File> {
        self.log.as_mut().ok_or(Error::Io)
    }
}

impl layers::Store for DiskStore {
    fn get(&mut self, hash: &Hash) -> Result<Vec<u8>> {
        let bytes = io(fs::read(self.path(hash)))?;
        // A chunk that isn't what its name says is a corrupt store, not data.
        if sha256(&bytes) != *hash {
            return Err(Error::Io);
        }
        Ok(bytes)
    }

    fn put(&mut self, data: &[u8]) -> Result<Hash> {
        let hash = sha256(data);
        let path = self.path(&hash);
        if path.exists() {
            return Ok(hash);
        }
        let dir = path.parent().ok_or(Error::Io)?;
        io(fs::create_dir_all(dir))?;
        let temporary = dir.join(format!(".{}.{}", hex(&hash[..8]), std::process::id()));
        io(fs::write(&temporary, data))?;
        io(fs::rename(&temporary, &path))?;
        self.unsynced.push(path);
        Ok(hash)
    }

    fn log_append(&mut self, data: &[u8]) -> Result<()> {
        let log = self.log()?;
        io(log.seek(SeekFrom::End(0)))?;
        io(log.write_all(data))
    }

    fn log_read(&mut self) -> Result<Vec<u8>> {
        // A store of chunks only has an empty log: an image opened to read, with nothing to replay.
        let Some(log) = self.log.as_mut() else { return Ok(Vec::new()) };
        let mut bytes = Vec::new();
        io(log.seek(SeekFrom::Start(0)))?;
        io(log.read_to_end(&mut bytes))?;
        Ok(bytes)
    }

    fn log_truncate(&mut self, len: u64) -> Result<()> {
        match self.log.as_mut() {
            Some(log) => io(log.set_len(len)),
            None if len == 0 => Ok(()),
            None => Err(Error::Io),
        }
    }

    fn log_sync(&mut self) -> Result<()> {
        for path in std::mem::take(&mut self.unsynced) {
            // Opened to write: Windows won't flush a file opened only to read.
            io(OpenOptions::new().write(true).open(&path).and_then(|f| f.sync_all()))?;
        }
        if let Some(dir) = Some(&self.chunks) {
            // The renames into place, too, where the host allows a directory to be synced.
            let _ = File::open(dir).and_then(|f| f.sync_all());
        }
        io(self.log()?.sync_data())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layers::Store;

    #[test]
    fn chunks_and_the_log_on_disk() {
        let home = std::env::temp_dir().join(format!("carbon-store-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let mut store = DiskStore::with_log(home.join("chunks"), &home.join("machines/m/log")).unwrap();
        let hash = store.put(b"hello").unwrap();
        assert_eq!(store.put(b"hello").unwrap(), hash);
        assert_eq!(store.get(&hash).unwrap(), b"hello");
        assert_eq!(parse_hash(&hex(&hash)), Some(hash));
        store.log_append(b"one").unwrap();
        store.log_append(b"two").unwrap();
        store.log_sync().unwrap();
        assert_eq!(store.log_read().unwrap(), b"onetwo");
        store.log_truncate(3).unwrap();
        store.log_append(b"!").unwrap();
        assert_eq!(store.log_read().unwrap(), b"one!");
        // A chunk changed behind the store's back is refused.
        fs::write(store.path(&hash), b"other").unwrap();
        assert_eq!(store.get(&hash), Err(Error::Io));
        fs::remove_dir_all(&home).unwrap();
    }
}
