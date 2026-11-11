//! What's kept about a file beside its contents, and changed by name or by descriptor: its mode (`chmod`), owner
//! (`chown`) and modification time (`utimensat`); hard links (`linkat`); and `flock`'s locks.
//!
//! Locks (`flock`, and `fcntl`'s record locks) are granted at once: nothing checks one process's against another's
//! yet, so they only keep programs that take them going (dpkg's and apt's, debconf's).

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use layers::{Ino, Kind};

use super::Node;
use crate::abi::*;
use crate::process::Task;

const AT_SYMLINK_FOLLOW: u64 = 0x400;
/// `utimensat`'s "now" and "leave it".
const UTIME_NOW: u64 = (1 << 30) - 1;
const UTIME_OMIT: u64 = (1 << 30) - 2;
/// `fcntl`'s `F_GETLK`, and the "unlocked" it answers with.
pub(crate) const F_GETLK: u64 = 5;
const F_UNLCK: u16 = 2;

impl Task {
    /// The file a call means: `path` from `dirfd` (following a last symbolic link unless `nofollow`), or, with an
    /// empty path and `AT_EMPTY_PATH` (or no path at all), the file `dirfd` is open on. None for one that isn't on the
    /// disk (a pipe, a socket, `/proc`), which has nothing to change.
    fn meta_target(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, nofollow: bool, empty_ok: bool) -> Result<Option<Ino>, Errno> {
        let path = if path_at == 0 { String::new() } else { self.read_path(cx, path_at)? };
        if path.is_empty() {
            if !empty_ok {
                return Err(ENOENT);
            }
            return Ok(match self.file(dirfd)?.node {
                Node::File(ino) | Node::Dir(ino) => Some(ino),
                _ => None,
            });
        }
        let (dir, dir_path) = self.start_dir(dirfd)?;
        if crate::os::stat(&crate::process::exe_path(&dir_path, &path)).is_some() {
            return Ok(None);
        }
        Ok(Some(lock(&self.k.disk).resolve(dir, &path, !nofollow)?))
    }

    /// `fchmodat` and `fchmod`: the permission bits, keeping what kind of file it is.
    pub(crate) fn chmod(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, mode: u32, flags: u64) -> Result<u64, Errno> {
        let Some(ino) = self.meta_target(cx, dirfd, path_at, flags & AT_SYMLINK_NOFOLLOW != 0, flags & AT_EMPTY_PATH != 0 || path_at == 0)? else { return Ok(0) };
        let mut disk = lock(&self.k.disk);
        // A link's own mode can't be changed, as on Linux.
        if disk.stat(ino)?.kind == Kind::Symlink {
            return Err(EOPNOTSUPP);
        }
        let mut meta = *disk.meta(ino)?;
        meta.mode = meta.mode & S_IFMT | mode & 0o7777;
        disk.set_meta(ino, meta)?;
        Ok(0)
    }

    /// `fchownat`, `fchown` and `lchown`: -1 leaves an ID as it is.
    pub(crate) fn chown(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, uid: u32, gid: u32, flags: u64) -> Result<u64, Errno> {
        let Some(ino) = self.meta_target(cx, dirfd, path_at, flags & AT_SYMLINK_NOFOLLOW != 0, flags & AT_EMPTY_PATH != 0 || path_at == 0)? else { return Ok(0) };
        let mut disk = lock(&self.k.disk);
        let mut meta = *disk.meta(ino)?;
        if uid != u32::MAX {
            meta.uid = uid;
        }
        if gid != u32::MAX {
            meta.gid = gid;
        }
        disk.set_meta(ino, meta)?;
        Ok(0)
    }

    /// `utimensat` (and `futimens`, with no path): the modification time, which is the one the disk keeps.
    pub(crate) fn utimensat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, times_at: u64, flags: u64) -> Result<u64, Errno> {
        let Some(ino) = self.meta_target(cx, dirfd, path_at, flags & AT_SYMLINK_NOFOLLOW != 0, true)? else { return Ok(0) };
        let now = self.k.now();
        let modified = if times_at == 0 {
            Some(now)
        } else {
            let (sec, nsec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), times_at + 16)?, user::read_u64(&t.space(), times_at + 24)?)))?;
            match nsec {
                UTIME_OMIT => None,
                UTIME_NOW => Some(now),
                n if n < 1_000_000_000 => Some(sec.saturating_mul(1_000_000_000).saturating_add(n)),
                _ => return Err(EINVAL),
            }
        };
        if let Some(modified) = modified {
            lock(&self.k.disk).set_modified(ino, modified)?;
        }
        Ok(0)
    }

    /// `linkat`: another name for a file.
    pub(crate) fn linkat(&mut self, cx: &mut Cx<'_>, olddirfd: i32, old_at: u64, newdirfd: i32, new_at: u64, flags: u64) -> Result<u64, Errno> {
        let ino = self.meta_target(cx, olddirfd, old_at, flags & AT_SYMLINK_FOLLOW == 0, flags & AT_EMPTY_PATH != 0)?.ok_or(EXDEV)?;
        let new = self.read_path(cx, new_at)?;
        let (dir, _) = self.start_dir(newdirfd)?;
        let mut disk = lock(&self.k.disk);
        if disk.stat(ino)?.kind == Kind::Dir {
            return Err(EPERM);
        }
        let (parent, name) = disk.resolve_parent(dir, &new)?;
        disk.link(parent, name, ino, self.k.now())?;
        Ok(0)
    }

    /// `flock`: granted (see the module's note), on any open file.
    pub(crate) fn flock(&mut self, fd: i32, operation: u64) -> Result<u64, Errno> {
        self.file(fd)?;
        // LOCK_SH, LOCK_EX or LOCK_UN, perhaps with LOCK_NB.
        if !matches!(operation & !4, 1 | 2 | 8) {
            return Err(EINVAL);
        }
        Ok(0)
    }

    /// `fcntl(F_GETLK)`: nothing holds a lock that would stop this one, so the `flock` structure says unlocked.
    pub(crate) fn get_lock(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64) -> Result<u64, Errno> {
        self.file(fd)?;
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &F_UNLCK.to_le_bytes()))?;
        Ok(0)
    }
}
