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
        let absolute = crate::process::exe_path(&dir_path, &path);
        if crate::os::stat(&self.k, &absolute).is_some() {
            return Ok(None);
        }
        let view = self.view();
        let found = view.resolve(&lock(&self.k.disk), dir, &path, !nofollow).map_err(Errno::from);
        match found {
            Ok(ino) => {
                self.writable(&absolute)?;
                Ok(Some(ino))
            }
            // Through a link to somewhere lambda makes, as `/dev/stdin` is: nothing there to change.
            Err(e) if (e == ENOENT || e == ENOTDIR) && self.through_links(&absolute, !nofollow).is_some() => Ok(None),
            Err(e) => Err(e),
        }
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

    /// x86-64's `futimesat` and `utimes`: `utimensat` with two `timeval`s, microseconds rather than nanoseconds.
    pub(crate) fn futimesat(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, times_at: u64) -> Result<u64, Errno> {
        let modified = if times_at == 0 {
            self.k.now()
        } else {
            let t = self.mem(cx, false, |t| (0..4).map(|i| user::read_u64(&t.space(), times_at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
            if t[1] >= 1_000_000 || t[3] >= 1_000_000 {
                return Err(EINVAL);
            }
            t[2].saturating_mul(1_000_000_000).saturating_add(t[3] * 1000)
        };
        let Some(ino) = self.meta_target(cx, dirfd, path_at, false, path_at == 0)? else { return Ok(0) };
        lock(&self.k.disk).set_modified(ino, modified)?;
        Ok(0)
    }

    /// x86-64's `utime`: a `utimbuf` of access and modification seconds, or none for now.
    pub(crate) fn utime(&mut self, cx: &mut Cx<'_>, path_at: u64, times_at: u64) -> Result<u64, Errno> {
        let modified = if times_at == 0 { self.k.now() } else { self.mem(cx, false, |t| user::read_u64(&t.space(), times_at + 8))?.saturating_mul(1_000_000_000) };
        let Some(ino) = self.meta_target(cx, AT_FDCWD, path_at, false, false)? else { return Ok(0) };
        lock(&self.k.disk).set_modified(ino, modified)?;
        Ok(0)
    }

    /// `name_to_handle_at`: the file must be there, but the disk gives no handles, as a file system without
    /// export support doesn't. Programs (systemd's, `findmnt`) fall back to comparing `stat`s and mount IDs.
    pub(crate) fn name_to_handle_at(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<u64, Errno> {
        // AT_HANDLE_MNT_ID_UNIQUE, AT_HANDLE_CONNECTABLE, AT_HANDLE_FID, AT_SYMLINK_FOLLOW and AT_EMPTY_PATH.
        if flags & !(0x001 | 0x002 | 0x200 | AT_SYMLINK_FOLLOW | AT_EMPTY_PATH) != 0 {
            return Err(EINVAL);
        }
        self.meta_target(cx, dirfd, path_at, flags & AT_SYMLINK_FOLLOW == 0, flags & AT_EMPTY_PATH != 0)?;
        Err(EOPNOTSUPP)
    }

    /// `open_by_handle_at`: no handle was ever given out, so whatever it's passed is stale.
    pub(crate) fn open_by_handle_at(&mut self, cx: &mut Cx<'_>, mount_fd: i32, handle_at: u64) -> Result<u64, Errno> {
        const MAX_HANDLE_SZ: u32 = 128;
        if mount_fd != AT_FDCWD {
            self.file(mount_fd)?;
        }
        let bytes = self.mem(cx, false, |t| user::read_u32(&t.space(), handle_at))?;
        if bytes == 0 || bytes > MAX_HANDLE_SZ {
            return Err(EINVAL);
        }
        Err(ESTALE)
    }

    /// `linkat`: another name for a file.
    pub(crate) fn linkat(&mut self, cx: &mut Cx<'_>, olddirfd: i32, old_at: u64, newdirfd: i32, new_at: u64, flags: u64) -> Result<u64, Errno> {
        let ino = self.meta_target(cx, olddirfd, old_at, flags & AT_SYMLINK_FOLLOW == 0, flags & AT_EMPTY_PATH != 0)?.ok_or(EXDEV)?;
        let new = self.read_path(cx, new_at)?;
        let (dir, dir_path) = self.start_dir(newdirfd)?;
        self.writable(&crate::process::exe_path(&dir_path, &new))?;
        let view = self.view();
        let mut disk = lock(&self.k.disk);
        if disk.stat(ino)?.kind == Kind::Dir {
            return Err(EPERM);
        }
        let (parent, name) = view.resolve_parent(&disk, dir, &new)?;
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
