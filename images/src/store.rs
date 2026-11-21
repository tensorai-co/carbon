//! The chunk store on the host's disk, and a machine's log beside it: `layers::Store`, in files.
//!
//! ```text
//! <home>/chunks/ab/cdef…   one file per chunk, named by its SHA-256, so each is kept once however many images share it
//! <home>/machines/<name>/log   a machine's log: its writable layer
//! <home>/machines/<name>/chunks/ab/cdef…   the chunks the machine wrote that no image has: its own
//! ```
//!
//! A chunk is written to a temporary file and renamed into place, so a crash never leaves half of one under its name.
//! Chunks written since the last sync are synced with the log, as `layers` expects: a record never names a chunk that
//! didn't survive.
//!
//! A machine's own chunks are kept apart because nothing else can name them: `layers` frees one when the machine's
//! files and log stop naming it, so deleting a file gives its space back, as Docker's overlay2 does with a container's
//! writable layer. Freezing the machine moves the chunks its image names into the shared store. Removing the machine
//! removes the rest with its folder.

use std::collections::HashSet;
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
    result.map_err(|e| if full(&e) { Error::NoSpace } else { Error::Io })
}

/// Whether the host's disk is full (or the user's quota on it is): ENOSPC or EDQUOT, or Windows'
/// ERROR_DISK_FULL or ERROR_HANDLE_DISK_FULL.
fn full(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded)
        || matches!(e.raw_os_error(), Some(n) if if cfg!(windows) { n == 112 || n == 39 } else { n == 28 || n == 122 || (cfg!(target_os = "macos") && n == 69) })
}

pub struct DiskStore {
    chunks: PathBuf,
    /// Where the machine's own chunks go, for a store with a log; and which chunks are there.
    own: Option<PathBuf>,
    owned: HashSet<Hash>,
    log: Option<File>,
    /// Chunks written since the last sync, to sync with the log.
    unsynced: Vec<PathBuf>,
}

impl DiskStore {
    /// Chunks only, in `chunks`: for building an image, which logs nothing.
    pub fn chunks(chunks: PathBuf) -> std::io::Result<DiskStore> {
        fs::create_dir_all(&chunks)?;
        Ok(DiskStore { chunks, own: None, owned: HashSet::new(), log: None, unsynced: Vec::new() })
    }

    /// Chunks in `chunks`, and the log at `log`, made if it isn't there.
    pub fn with_log(chunks: PathBuf, log: &Path) -> std::io::Result<DiskStore> {
        let mut store = DiskStore::chunks(chunks)?;
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)?;
            let own = dir.join("chunks");
            fs::create_dir_all(&own)?;
            store.owned = listed(&own)?;
            store.own = Some(own);
        }
        // Written at its end, not opened to append: a file opened to append can't be cut short on Windows.
        store.log = Some(OpenOptions::new().read(true).write(true).create(true).truncate(false).open(log)?);
        Ok(store)
    }

    fn path(&self, hash: &Hash) -> PathBuf {
        at(&self.chunks, hash)
    }

    /// Where a chunk of the machine's own is.
    fn own_path(&self, hash: &Hash) -> Option<PathBuf> {
        self.own.as_ref().map(|own| at(own, hash))
    }

    /// Whether a chunk is here: shared, or the machine's own.
    pub fn has(&self, hash: &Hash) -> bool {
        self.owned.contains(hash) || self.path(hash).exists()
    }

    fn log(&mut self) -> Result<&mut File> {
        self.log.as_mut().ok_or(Error::Io)
    }
}

/// Where a chunk is in a folder of chunks: in the folder named by its first byte.
fn at(dir: &Path, hash: &Hash) -> PathBuf {
    let name = hex(hash);
    dir.join(&name[..2]).join(&name[2..])
}

/// The chunks in a folder of them; not those still being written (`.`-named), nor anything else there.
fn listed(dir: &Path) -> std::io::Result<HashSet<Hash>> {
    let mut out = HashSet::new();
    for shard in fs::read_dir(dir)?.flatten() {
        let prefix = shard.file_name().to_string_lossy().into_owned();
        let Ok(entries) = fs::read_dir(shard.path()) else { continue };
        for entry in entries.flatten() {
            if let Some(hash) = unhex(&format!("{prefix}{}", entry.file_name().to_string_lossy())) {
                out.insert(hash);
            }
        }
    }
    Ok(out)
}

/// Writes a chunk at `path` by way of a temporary file beside it, so a crash never leaves half of one under its name.
fn write_chunk(path: &Path, hash: &Hash, data: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or(Error::Io)?;
    io(fs::create_dir_all(dir))?;
    let temporary = dir.join(format!(".{}.{}", hex(&hash[..8]), std::process::id()));
    if let Err(e) = io(fs::write(&temporary, data)) {
        let _ = fs::remove_file(&temporary);
        return Err(e);
    }
    io(fs::rename(&temporary, path))
}

impl layers::Store for DiskStore {
    fn get(&mut self, hash: &Hash) -> Result<Vec<u8>> {
        // The machine's own first, if it's one; then shared; then the machine's own after all, in case another process
        // wrote it there since this store was opened.
        let own = self.own_path(hash);
        let bytes = match own.as_ref().filter(|_| self.owned.contains(hash)) {
            Some(path) => io(fs::read(path))?,
            None => match (fs::read(self.path(hash)), &own) {
                (Ok(bytes), _) => bytes,
                (Err(e), Some(path)) if e.kind() == std::io::ErrorKind::NotFound => io(fs::read(path))?,
                (Err(e), _) => return io(Err(e)),
            },
        };
        // A chunk that isn't what its name says is a corrupt store, not data.
        if sha256(&bytes) != *hash {
            return Err(Error::Io);
        }
        Ok(bytes)
    }

    fn put(&mut self, data: &[u8]) -> Result<Hash> {
        let hash = sha256(data);
        if self.owned.contains(&hash) || self.path(&hash).exists() {
            return Ok(hash);
        }
        // New to the store: the machine's own, if this is a machine's store; else shared, as an image's chunks are.
        let path = self.own_path(&hash).unwrap_or_else(|| self.path(&hash));
        write_chunk(&path, &hash, data)?;
        if self.own.is_some() {
            self.owned.insert(hash);
        }
        self.unsynced.push(path);
        Ok(hash)
    }

    fn log_append(&mut self, data: &[u8]) -> Result<()> {
        let log = self.log()?;
        let end = io(log.seek(SeekFrom::End(0)))?;
        let written = io(log.write_all(data));
        if written.is_err() {
            // Half a record (a full disk) would hide every record after it when the log is read: it goes.
            let _ = log.set_len(end);
        }
        written
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
        for dir in [Some(&self.chunks), self.own.as_ref()].into_iter().flatten() {
            // The renames into place, too, where the host allows a directory to be synced.
            let _ = File::open(dir).and_then(|f| f.sync_all());
        }
        io(self.log()?.sync_data())
    }

    fn space(&mut self) -> Option<(u64, u64)> {
        host::disk_space(&self.chunks)
    }

    fn own_chunks(&mut self) -> Vec<Hash> {
        self.owned.iter().copied().collect()
    }

    fn free(&mut self, hash: &Hash) -> Result<()> {
        let Some(path) = self.own_path(hash).filter(|_| self.owned.contains(hash)) else { return Ok(()) };
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return io(Err(e)),
            _ => {}
        }
        self.owned.remove(hash);
        self.unsynced.retain(|p| *p != path);
        Ok(())
    }

    fn share(&mut self, hashes: &[Hash]) -> Result<()> {
        let (mut dirs, mut moved) = (HashSet::new(), Vec::new());
        for hash in hashes {
            let Some(from) = self.own_path(hash).filter(|_| self.owned.contains(hash)) else { continue };
            let to = self.path(hash);
            let dir = to.parent().ok_or(Error::Io)?.to_path_buf();
            io(fs::create_dir_all(&dir))?;
            io(fs::rename(&from, &to))?;
            self.owned.remove(hash);
            self.unsynced.retain(|p| *p != from);
            dirs.insert(dir);
            moved.push(to);
        }
        // The chunks, and their moves into place, survive a crash before the index naming them is saved.
        for path in moved {
            io(OpenOptions::new().write(true).open(&path).and_then(|f| f.sync_all()))?;
        }
        for dir in dirs {
            let _ = File::open(dir).and_then(|f| f.sync_all());
        }
        Ok(())
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
        fs::write(store.own_path(&hash).unwrap(), b"other").unwrap();
        assert_eq!(store.get(&hash), Err(Error::Io));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_machines_own_chunks_are_freed_or_shared() {
        let home = std::env::temp_dir().join(format!("carbon-store-own-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let shared = DiskStore::chunks(home.join("chunks")).unwrap().put(b"image's").unwrap();
        let mut store = DiskStore::with_log(home.join("chunks"), &home.join("machines/m/log")).unwrap();
        // One the image has stays shared; new ones are the machine's.
        assert_eq!(store.put(b"image's").unwrap(), shared);
        let (a, b) = (store.put(b"a").unwrap(), store.put(b"b").unwrap());
        assert!(store.own_path(&a).unwrap().exists() && !store.path(&a).exists());
        let mut own = store.own_chunks();
        own.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(own, expected);
        // Freed: gone. Shared: moved, and freeing it after does nothing.
        store.free(&a).unwrap();
        store.free(&shared).unwrap();
        assert!(!store.own_path(&a).unwrap().exists() && store.path(&shared).exists());
        store.share(&[b, shared]).unwrap();
        store.free(&b).unwrap();
        assert!(store.path(&b).exists() && store.own_chunks().is_empty());
        assert_eq!(store.get(&b).unwrap(), b"b");
        // Reopened, the machine's own are found again.
        let c = store.put(b"c").unwrap();
        let mut again = DiskStore::with_log(home.join("chunks"), &home.join("machines/m/log")).unwrap();
        assert_eq!(again.own_chunks(), vec![c]);
        assert_eq!(again.get(&c).unwrap(), b"c");
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn the_space_is_the_host_disks() {
        let home = std::env::temp_dir().join(format!("carbon-store-space-{}", std::process::id()));
        let mut store = DiskStore::chunks(home.join("chunks")).unwrap();
        let (total, free) = store.space().unwrap();
        assert!(total > 0 && free <= total);
        assert_eq!(layers::MemoryStore::default().space(), None);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_full_disk_is_no_space() {
        let full = std::io::Error::from_raw_os_error(if cfg!(windows) { 112 } else { 28 });
        assert_eq!(io::<()>(Err(full)), Err(Error::NoSpace));
        assert_eq!(io::<()>(Err(std::io::Error::other("else"))), Err(Error::Io));
    }
}
