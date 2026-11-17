//! A folder of the host's, as a machine's mount sees it: its files by paths within it (`""` for its top, `a/b`
//! below), read and changed as they're asked for, so the host and the machine see each other's changes at once.
//!
//! Nothing a path names outside the folder is reached: a path's parts are never `..`, and a symbolic link in it is
//! read, never followed here (the machine's kernel follows it, in the machine). A volume's folder keeps each file's
//! kernel metadata (owner, mode, attributes) in an extended attribute, `user.carbon.meta`, so a file made by uid 999
//! is 999's when the volume is mounted again; a bound folder of the user's is left as it is.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// What a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
}

/// What the host says of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub kind: FileKind,
    pub size: u64,
    /// Nanoseconds since 1970.
    pub modified: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub links: u32,
    /// Kept kernel metadata, for a folder that keeps it.
    pub meta: Option<[u32; 4]>,
}

/// A host folder, reached by paths within it.
#[derive(Debug, Clone)]
pub struct Folder {
    root: PathBuf,
    keep_meta: bool,
}

const META: &str = "user.carbon.meta";

fn bad_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "a path that leaves the folder")
}

impl Folder {
    /// The folder at `root`, which must be one. With `keep_meta`, each file's kernel metadata is kept with it.
    pub fn open(root: impl Into<PathBuf>, keep_meta: bool) -> io::Result<Folder> {
        let root = root.into();
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("{}: not a folder", root.display())));
        }
        Ok(Folder { root: fs::canonicalize(&root)?, keep_meta })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether names that differ only in case are one, as on a Mac's or Windows' disk by default.
    pub fn folds_case(&self) -> bool {
        let probe = self.root.join(".CARBON-CASE-PROBE");
        let made = fs::write(&probe, b"").is_ok();
        let folds = made && fs::symlink_metadata(self.root.join(".carbon-case-probe")).is_ok();
        if made {
            let _ = fs::remove_file(&probe);
        }
        folds
    }

    fn path(&self, rel: &str) -> io::Result<PathBuf> {
        let mut out = self.root.clone();
        for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." || part.contains('\\') && cfg!(windows) {
                return Err(bad_path());
            }
            out.push(part);
        }
        Ok(out)
    }

    pub fn stat(&self, rel: &str) -> io::Result<FileStat> {
        let path = self.path(rel)?;
        let m = fs::symlink_metadata(&path)?;
        let kind = if m.file_type().is_symlink() {
            FileKind::Symlink
        } else if m.is_dir() {
            FileKind::Dir
        } else {
            FileKind::File
        };
        let modified = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
        let (mode, uid, gid, links) = owner(&m);
        let meta = if self.keep_meta && kind != FileKind::Symlink { sys::xattr_get(&path, META).and_then(|v| words(&v)) } else { None };
        Ok(FileStat { kind, size: if kind == FileKind::Dir { 0 } else { m.len() }, modified, mode, uid, gid, links, meta })
    }

    pub fn list(&self, rel: &str) -> io::Result<Vec<(String, FileKind)>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.path(rel)?)? {
            let entry = entry?;
            let t = entry.file_type()?;
            let kind = if t.is_symlink() {
                FileKind::Symlink
            } else if t.is_dir() {
                FileKind::Dir
            } else {
                FileKind::File
            };
            out.push((entry.file_name().to_string_lossy().into_owned(), kind));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    fn open_file(&self, rel: &str, write: bool) -> io::Result<fs::File> {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(write);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
        options.open(self.path(rel)?)
    }

    pub fn read(&self, rel: &str, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let mut file = self.open_file(rel, false)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut done = 0;
        while done < buffer.len() {
            match file.read(&mut buffer[done..])? {
                0 => break,
                n => done += n,
            }
        }
        Ok(done)
    }

    pub fn write(&self, rel: &str, offset: u64, data: &[u8]) -> io::Result<usize> {
        let mut file = self.open_file(rel, true)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;
        Ok(data.len())
    }

    pub fn truncate(&self, rel: &str, size: u64) -> io::Result<()> {
        self.open_file(rel, true)?.set_len(size)
    }

    pub fn make_file(&self, rel: &str, mode: u32) -> io::Result<()> {
        let path = self.path(rel)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode & 0o7777);
        options.open(&path)?;
        let _ = mode;
        Ok(())
    }

    pub fn make_dir(&self, rel: &str, mode: u32) -> io::Result<()> {
        let path = self.path(rel)?;
        fs::create_dir(&path)?;
        self.set_mode(rel, mode)
    }

    pub fn make_symlink(&self, rel: &str, target: &str) -> io::Result<()> {
        let path = self.path(rel)?;
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, path);
        #[cfg(windows)]
        return std::os::windows::fs::symlink_file(target, path);
    }

    pub fn link(&self, existing: &str, rel: &str) -> io::Result<()> {
        fs::hard_link(self.path(existing)?, self.path(rel)?)
    }

    pub fn remove(&self, rel: &str, dir: bool) -> io::Result<()> {
        let path = self.path(rel)?;
        if dir { fs::remove_dir(path) } else { fs::remove_file(path) }
    }

    pub fn rename(&self, from: &str, to: &str) -> io::Result<()> {
        fs::rename(self.path(from)?, self.path(to)?)
    }

    pub fn read_link(&self, rel: &str) -> io::Result<String> {
        Ok(fs::read_link(self.path(rel)?)?.to_string_lossy().into_owned())
    }

    /// Sets a file's permission bits (not a link's, which have none worth setting).
    pub fn set_mode(&self, rel: &str, mode: u32) -> io::Result<()> {
        let path = self.path(rel)?;
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o7777))?;
        }
        #[cfg(windows)]
        {
            let mut p = fs::metadata(&path)?.permissions();
            p.set_readonly(mode & 0o200 == 0);
            fs::set_permissions(&path, p)?;
        }
        Ok(())
    }

    pub fn set_modified(&self, rel: &str, modified: u64) -> io::Result<()> {
        let path = self.path(rel)?;
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Ok(());
        }
        let at = UNIX_EPOCH + std::time::Duration::from_nanos(modified);
        let file = fs::OpenOptions::new().write(!fs::metadata(&path)?.is_dir()).read(true).open(&path)?;
        file.set_times(fs::FileTimes::new().set_modified(at).set_accessed(SystemTime::now()))
    }

    /// Keeps a file's kernel metadata with it, if this folder keeps it. Whether it did.
    pub fn keep_meta(&self, rel: &str, words: [u32; 4]) -> io::Result<bool> {
        if !self.keep_meta {
            return Ok(false);
        }
        let path = self.path(rel)?;
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            // A link's own attributes can't be set everywhere: its metadata is made from what it is.
            return Ok(true);
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        sys::xattr_set(&path, META, &bytes).map(|()| true)
    }
}

fn words(v: &[u8]) -> Option<[u32; 4]> {
    (v.len() == 16).then(|| core::array::from_fn(|i| u32::from_le_bytes([v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]])))
}

#[cfg(unix)]
fn owner(m: &fs::Metadata) -> (u32, u32, u32, u32) {
    use std::os::unix::fs::MetadataExt;
    (m.mode() & 0o7777, m.uid(), m.gid(), m.nlink() as u32)
}

#[cfg(windows)]
fn owner(m: &fs::Metadata) -> (u32, u32, u32, u32) {
    let mode = if m.is_dir() { 0o755 } else if m.permissions().readonly() { 0o444 } else { 0o644 };
    (mode, 0, 0, 1)
}

use crate::sys;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_read_and_changed_by_paths() {
        let dir = std::env::temp_dir().join(format!("carbon-folder-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        let f = Folder::open(&dir, true).unwrap();
        f.make_file("a/x.txt", 0o640).unwrap();
        f.write("a/x.txt", 0, b"hello").unwrap();
        f.write("a/x.txt", 5, b" world").unwrap();
        let mut buf = [0u8; 32];
        let n = f.read("a/x.txt", 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello world");
        let st = f.stat("a/x.txt").unwrap();
        assert_eq!((st.kind, st.size), (FileKind::File, 11));
        assert!(f.keep_meta("a/x.txt", [1, 2, 3, 4]).unwrap());
        assert_eq!(f.stat("a/x.txt").unwrap().meta, Some([1, 2, 3, 4]));
        f.make_symlink("a/l", "x.txt").unwrap();
        assert_eq!(f.read_link("a/l").unwrap(), "x.txt");
        assert_eq!(f.stat("a/l").unwrap().kind, FileKind::Symlink);
        f.rename("a/x.txt", "a/y.txt").unwrap();
        assert_eq!(f.list("a").unwrap(), vec![("l".to_string(), FileKind::Symlink), ("y.txt".to_string(), FileKind::File)]);
        assert!(f.stat("../escape").is_err());
        f.truncate("a/y.txt", 2).unwrap();
        assert_eq!(f.stat("a/y.txt").unwrap().size, 2);
        f.remove("a/y.txt", false).unwrap();
        f.remove("a/l", false).unwrap();
        f.remove("a", true).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
