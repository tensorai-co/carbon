//! Moving data: reads and writes scattered over several buffers, at an offset or not (`readv`, `writev`, `preadv`,
//! `pwritev` and their `2` forms), and from one file to another without passing through the program (`sendfile`,
//! `copy_file_range`, `splice`, `tee`, `vmsplice`). Also what changes a file's size or says how it'll be used:
//! `truncate`, `fallocate`, `readahead` and `sync_file_range`.
//!
//! A copy goes through lambda a piece at a time, so a big one never holds much in memory. A file read from keeps its
//! offset as it was until the write has taken the bytes, so a short write (a non-blocking socket's, a full pipe's)
//! loses nothing. A copy into a pipe moves at most what fits, as Linux's does, so a program that splices into a pipe
//! and then reads it itself never waits on itself.

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::{MOST_AT_ONCE, Node, Open};
use crate::abi::*;
use crate::process::Task;

// `preadv2` and `pwritev2`'s flags.
const RWF_HIPRI: u64 = 0x01;
const RWF_DSYNC: u64 = 0x02;
const RWF_SYNC: u64 = 0x04;
const RWF_NOWAIT: u64 = 0x08;
const RWF_APPEND: u64 = 0x10;
const RWF_NOAPPEND: u64 = 0x20;
const RWF_ATOMIC: u64 = 0x40;
const RWF_DONTCACHE: u64 = 0x80;
const RWF_ALL: u64 = RWF_HIPRI | RWF_DSYNC | RWF_SYNC | RWF_NOWAIT | RWF_APPEND | RWF_NOAPPEND | RWF_ATOMIC | RWF_DONTCACHE;

// `splice`'s flags: only `SPLICE_F_NONBLOCK` changes anything.
const SPLICE_F_NONBLOCK: u64 = 0x02;
const SPLICE_F_ALL: u64 = 0x0f;

// `fallocate`'s modes.
const FALLOC_FL_KEEP_SIZE: u64 = 0x01;
const FALLOC_FL_PUNCH_HOLE: u64 = 0x02;
const FALLOC_FL_ZERO_RANGE: u64 = 0x10;
const FALLOC_FL_WRITE_ZEROES: u64 = 0x80;

/// The most one `sendfile`, `splice` or `copy_file_range` moves, as on Linux.
const MOST_COPIED: u64 = 0x7fff_f000;
/// How much one pass of a copy moves.
const PIECE: u64 = 1 << 20;

/// Whether `open` has offsets to read and write at. Pipes, sockets and terminals don't: `pread` on them is ESPIPE.
pub(super) fn seekable(open: &Open) -> bool {
    matches!(open.node, Node::File(_) | Node::Dir(_) | Node::Made(_) | Node::Null | Node::Zero | Node::Random | Node::Memfd(..))
}

/// Where a copy reads or writes: at an offset the call keeps (its own, or the program's), or at the file's own.
#[derive(Clone, Copy)]
enum At {
    Own,
    Offset(u64),
}

impl Task {
    /// The `iovec`s at `at`, and what they hold in all.
    fn iovecs_total(&mut self, cx: &mut Cx<'_>, at: u64, count: u64) -> Result<(Vec<(u64, u64)>, u64), Errno> {
        let iovecs = self.iovecs(cx, at, count)?;
        let total = iovecs.iter().try_fold(0u64, |sum, &(_, len)| sum.checked_add(len).filter(|s| *s <= i64::MAX as u64)).ok_or(EINVAL)?;
        Ok((iovecs, total))
    }

    /// `readv`, and `preadv` with `at`: reads once, and spreads what came over the buffers in order.
    pub(crate) fn vread(&mut self, cx: &mut Cx<'_>, fd: i32, iov: u64, count: u64, at: Option<u64>) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let (iovecs, total) = self.iovecs_total(cx, iov, count)?;
        let data = self.read_from(&open, total, at)?;
        self.mem(cx, true, |t| {
            let mut done = 0;
            for &(base, len) in &iovecs {
                let n = (len as usize).min(data.len() - done);
                user::write(&mut t.space(), base, &data[done..done + n])?;
                done += n;
            }
            Ok(())
        })?;
        Ok(data.len() as u64)
    }

    /// The buffers' bytes, in order.
    fn gather(&mut self, cx: &mut Cx<'_>, iov: u64, count: u64) -> Result<Vec<u8>, Errno> {
        let (iovecs, _) = self.iovecs_total(cx, iov, count)?;
        self.mem(cx, false, |t| {
            let space = t.space();
            let mut data = Vec::new();
            for &(base, len) in &iovecs {
                let room = MOST_AT_ONCE as usize - data.len();
                data.extend(user::read_vec(&space, base, (len as usize).min(room))?);
                if data.len() == MOST_AT_ONCE as usize {
                    break;
                }
            }
            Ok(data)
        })
    }

    /// `writev`, and `pwritev` with `at`: the buffers' bytes, in order, in one write.
    pub(crate) fn vwrite(&mut self, cx: &mut Cx<'_>, fd: i32, iov: u64, count: u64, at: Option<u64>) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let data = self.gather(cx, iov, count)?;
        Ok(self.write_to(&open, &data, at)? as u64)
    }

    /// Whether a read (or a write) of a pipe would wait now, for `RWF_NOWAIT` and `SPLICE_F_NONBLOCK`. Nothing else
    /// they're used on waits.
    fn would_wait(open: &Open, write: bool) -> bool {
        match &open.node {
            Node::Pipe(pipe, true) if write => !pipe.writable().0,
            Node::Pipe(pipe, false) if !write => !pipe.readable().0,
            _ => false,
        }
    }

    /// `preadv`, and with `flags` `preadv2`: at `offset`, or for `preadv2` at the file's own if it's -1.
    pub(crate) fn preadv(&mut self, cx: &mut Cx<'_>, fd: i32, iov: u64, count: u64, offset: i64, flags: Option<u64>) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let at = self.rw_offset(&open, offset, flags)?;
        let flags = flags.unwrap_or(0);
        if flags & RWF_NOWAIT != 0 && Self::would_wait(&open, false) {
            return Err(EAGAIN);
        }
        self.vread(cx, fd, iov, count, at)
    }

    /// `pwritev` and `pwritev2`; `RWF_APPEND` writes at the end whatever the offset, moving the file's own offset
    /// past it if that's the one the call means.
    pub(crate) fn pwritev(&mut self, cx: &mut Cx<'_>, fd: i32, iov: u64, count: u64, offset: i64, flags: Option<u64>) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        let at = self.rw_offset(&open, offset, flags)?;
        let flags = flags.unwrap_or(0);
        if flags & RWF_NOWAIT != 0 && Self::would_wait(&open, true) {
            return Err(EAGAIN);
        }
        let Node::File(ino) = open.node else { return self.vwrite(cx, fd, iov, count, at) };
        if flags & RWF_APPEND == 0 {
            return self.vwrite(cx, fd, iov, count, at);
        }
        let data = self.gather(cx, iov, count)?;
        let end = lock(&self.k.disk).stat(ino)?.size;
        let n = self.write_to(&open, &data, Some(end))?;
        if at.is_none() {
            *lock(&open.offset) = end + n as u64;
        }
        Ok(n as u64)
    }

    /// The offset a `p*v` call means: None for the file's own (`-1` in a `2` form, the one with `flags`), checking
    /// its flags and that the file has offsets at all.
    fn rw_offset(&self, open: &Open, offset: i64, flags: Option<u64>) -> Result<Option<u64>, Errno> {
        if flags.unwrap_or(0) & !RWF_ALL != 0 {
            return Err(EOPNOTSUPP);
        }
        if flags.is_some() && offset == -1 {
            return Ok(None);
        }
        if !seekable(open) {
            return Err(ESPIPE);
        }
        u64::try_from(offset).map(Some).map_err(|_| EINVAL)
    }

    /// Copies up to `count` bytes from `from` to `to`, a piece at a time while `from` is a file on the disk, and
    /// gives how much was copied: less at `from`'s end, or when `to` took less. `from_at` and `to_at` are moved on by
    /// what was copied; with `At::Own`, the file's own offset is.
    fn pump(&mut self, from: &Open, from_at: &mut At, to: &Open, to_at: &mut At, count: u64, nonblock: bool) -> Result<u64, Errno> {
        let mut done = 0;
        while done < count {
            let mut want = (count - done).min(PIECE);
            if let Node::Pipe(pipe, true) = &to.node {
                // What fits now, or a page's worth to wait for room for.
                want = want.min(pipe.room().max(4096) as u64);
            }
            if nonblock && (Self::would_wait(from, false) || Self::would_wait(to, true)) {
                return if done > 0 { Ok(done) } else { Err(EAGAIN) };
            }
            // A file read from at its own offset is read at that offset, and moved on below by what was written.
            let read_at = match *from_at {
                At::Offset(at) => Some(at),
                At::Own if seekable(from) => Some(*lock(&from.offset)),
                At::Own => None,
            };
            let data = match self.read_from(from, want, read_at) {
                Ok(data) => data,
                Err(_) if done > 0 => break,
                Err(e) => return Err(e),
            };
            if data.is_empty() {
                break;
            }
            let write_at = match *to_at {
                At::Offset(at) => Some(at),
                At::Own => None,
            };
            let n = match self.write_to(to, &data, write_at) {
                Ok(n) => n as u64,
                Err(_) if done > 0 => break,
                Err(e) => return Err(e),
            };
            match from_at {
                At::Offset(at) => *at += n,
                At::Own if seekable(from) => *lock(&from.offset) = read_at.unwrap_or(0) + n,
                At::Own => {}
            }
            if let At::Offset(at) = to_at {
                *at += n;
            }
            done += n;
            let more = matches!(from.node, Node::File(_) | Node::Memfd(..)) && !matches!(to.node, Node::Pipe(..));
            if n < data.len() as u64 || (data.len() as u64) < want || !more {
                break;
            }
        }
        Ok(done)
    }

    /// An `loff_t` the program passes by address, or `At::Own` for none.
    fn read_offset(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<At, Errno> {
        if at == 0 {
            return Ok(At::Own);
        }
        let offset = self.mem(cx, false, |t| user::read_u64(&t.space(), at))? as i64;
        u64::try_from(offset).map(At::Offset).map_err(|_| EINVAL)
    }

    fn write_offset(&mut self, cx: &mut Cx<'_>, at: u64, offset: At) -> Result<(), Errno> {
        match offset {
            At::Offset(value) if at != 0 => self.mem(cx, true, |t| user::write_u64(&mut t.space(), at, value)),
            _ => Ok(()),
        }
    }

    /// `sendfile`: from `in_fd` (at `*offset_at` if given, leaving its own offset alone) to `out_fd`.
    pub(crate) fn sendfile(&mut self, cx: &mut Cx<'_>, out_fd: i32, in_fd: i32, offset_at: u64, count: u64) -> Result<u64, Errno> {
        let (to, from) = (self.file(out_fd)?, self.file(in_fd)?);
        if !from.readable() || !to.writable() {
            return Err(EBADF);
        }
        let mut from_at = self.read_offset(cx, offset_at)?;
        if matches!(from_at, At::Offset(_)) && !seekable(&from) {
            return Err(ESPIPE);
        }
        let done = self.pump(&from, &mut from_at, &to, &mut At::Own, count.min(MOST_COPIED), false)?;
        self.write_offset(cx, offset_at, from_at)?;
        Ok(done)
    }

    /// `copy_file_range`: between two files on the disk, each at the offset given or its own.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn copy_file_range(&mut self, cx: &mut Cx<'_>, in_fd: i32, in_at: u64, out_fd: i32, out_at: u64, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let (from, to) = (self.file(in_fd)?, self.file(out_fd)?);
        if !from.readable() || !to.writable() || *lock(&to.flags) & O_APPEND != 0 {
            return Err(EBADF);
        }
        // Two files on the disk or in memory; the same file is the same inode, or the same memfd.
        let same = match (&from.node, &to.node) {
            (Node::File(a), Node::File(b)) => a == b,
            (Node::Memfd(a, false), Node::Memfd(b, false)) => Arc::ptr_eq(a, b),
            (Node::File(_), Node::Memfd(_, false)) | (Node::Memfd(_, false), Node::File(_)) => false,
            (Node::Dir(_), _) | (_, Node::Dir(_)) => return Err(EISDIR),
            _ => return Err(EINVAL),
        };
        let mut from_at = self.read_offset(cx, in_at)?;
        let mut to_at = self.read_offset(cx, out_at)?;
        let len = len.min(MOST_COPIED);
        // Within one file, the ranges mustn't overlap.
        if same {
            let start = |at: At, open: &Open| match at {
                At::Offset(v) => v,
                At::Own => *lock(&open.offset),
            };
            let (a, b) = (start(from_at, &from), start(to_at, &to));
            if a < b.saturating_add(len) && b < a.saturating_add(len) {
                return Err(EINVAL);
            }
        }
        let done = self.pump(&from, &mut from_at, &to, &mut to_at, len, false)?;
        self.write_offset(cx, in_at, from_at)?;
        self.write_offset(cx, out_at, to_at)?;
        Ok(done)
    }

    /// `splice`: to or from a pipe; the end that isn't one may be at an offset.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn splice(&mut self, cx: &mut Cx<'_>, in_fd: i32, in_at: u64, out_fd: i32, out_at: u64, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !SPLICE_F_ALL != 0 {
            return Err(EINVAL);
        }
        let (from, to) = (self.file(in_fd)?, self.file(out_fd)?);
        if !from.readable() || !to.writable() {
            return Err(EBADF);
        }
        let pipe = |open: &Open| match &open.node {
            Node::Pipe(p, _) => Some(std::sync::Arc::as_ptr(p)),
            _ => None,
        };
        match (pipe(&from), pipe(&to)) {
            (None, None) => return Err(EINVAL),
            (Some(a), Some(b)) if a == b => return Err(EINVAL),
            _ => {}
        }
        if (in_at != 0 && pipe(&from).is_some()) || (out_at != 0 && pipe(&to).is_some()) {
            return Err(ESPIPE);
        }
        if (in_at != 0 && !seekable(&from)) || (out_at != 0 && !seekable(&to)) {
            return Err(ESPIPE);
        }
        if len == 0 {
            return Ok(0);
        }
        let mut from_at = self.read_offset(cx, in_at)?;
        let mut to_at = self.read_offset(cx, out_at)?;
        let nonblock = flags & SPLICE_F_NONBLOCK != 0;
        let done = self.pump(&from, &mut from_at, &to, &mut to_at, len.min(MOST_COPIED), nonblock)?;
        self.write_offset(cx, in_at, from_at)?;
        self.write_offset(cx, out_at, to_at)?;
        Ok(done)
    }

    /// `tee`: copies what's in one pipe into another, leaving it in the first.
    pub(crate) fn tee(&mut self, in_fd: i32, out_fd: i32, len: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !SPLICE_F_ALL != 0 {
            return Err(EINVAL);
        }
        let (from, to) = (self.file(in_fd)?, self.file(out_fd)?);
        let (Node::Pipe(source, false), Node::Pipe(sink, true)) = (&from.node, &to.node) else { return Err(EINVAL) };
        if std::sync::Arc::ptr_eq(source, sink) {
            return Err(EINVAL);
        }
        let nonblock = flags & SPLICE_F_NONBLOCK != 0;
        if nonblock && Self::would_wait(&to, true) {
            return Err(EAGAIN);
        }
        let want = (len.min(MOST_COPIED) as usize).min(sink.room().max(4096));
        let data = source.peek(&self.waiter, want, nonblock)?;
        if data.is_empty() {
            return Ok(0);
        }
        Ok(self.write_to(&to, &data, None)? as u64)
    }

    /// `vmsplice`: the program's buffers into a pipe's write end, or out of its read end.
    pub(crate) fn vmsplice(&mut self, cx: &mut Cx<'_>, fd: i32, iov: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !SPLICE_F_ALL != 0 {
            return Err(EINVAL);
        }
        let open = self.file(fd)?;
        let Node::Pipe(_, write_end) = open.node else { return Err(EBADF) };
        if flags & SPLICE_F_NONBLOCK != 0 && Self::would_wait(&open, write_end) {
            return Err(EAGAIN);
        }
        if write_end { self.vwrite(cx, fd, iov, count, None) } else { self.vread(cx, fd, iov, count, None) }
    }

    /// `truncate`: `ftruncate` by name, following symbolic links.
    pub(crate) fn truncate(&mut self, cx: &mut Cx<'_>, path_at: u64, len: u64) -> Result<u64, Errno> {
        if (len as i64) < 0 {
            return Err(EINVAL);
        }
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (dir, dir_path) = self.start_dir(AT_FDCWD)?;
        let absolute = crate::process::exe_path(&dir_path, &path);
        if crate::os::stat(&absolute).is_some() {
            return Err(EACCES);
        }
        let view = self.view();
        self.writable(&absolute)?;
        let mut disk = lock(&self.k.disk);
        let ino = view.resolve(&disk, dir, &path, true)?;
        if disk.stat(ino)?.kind == layers::Kind::Dir {
            return Err(EISDIR);
        }
        if !matches!(disk.meta(ino)?.mode & S_IFMT, 0 | S_IFREG) {
            return Err(EINVAL);
        }
        disk.truncate(ino, len, self.k.now())?;
        Ok(0)
    }

    /// `fallocate`: room for a range (the disk makes room as it's written, so only the size changes), or a range
    /// zeroed, punched out or not. Moving ranges (`COLLAPSE_RANGE`, `INSERT_RANGE`) isn't supported.
    pub(crate) fn fallocate(&mut self, fd: i32, mode: u64, offset: u64, len: u64) -> Result<u64, Errno> {
        let (offset_signed, len_signed) = (offset as i64, len as i64);
        if offset_signed < 0 || len_signed <= 0 {
            return Err(EINVAL);
        }
        let end = offset_signed.checked_add(len_signed).ok_or(EFBIG)? as u64;
        let open = self.file(fd)?;
        if let Node::Memfd(mem, _) = &open.node {
            if !open.writable() {
                return Err(EBADF);
            }
            match mode {
                0 if end > mem.size() => mem.truncate(end)?,
                0 | FALLOC_FL_KEEP_SIZE => {}
                m if m == FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE => mem.zero(offset, len),
                _ => return Err(EOPNOTSUPP),
            }
            return Ok(0);
        }
        let ino = match open.node {
            Node::File(ino) => ino,
            Node::Pipe(..) => return Err(ESPIPE),
            Node::Dir(_) => return Err(EISDIR),
            _ => return Err(ENODEV),
        };
        if !open.writable() {
            return Err(EBADF);
        }
        if mode & !FALLOC_FL_KEEP_SIZE != 0 && *lock(&open.flags) & O_APPEND != 0 {
            return Err(EPERM);
        }
        let now = self.k.now();
        let mut disk = lock(&self.k.disk);
        let grow = |disk: &mut crate::Disk| -> Result<(), Errno> {
            if end > disk.stat(ino)?.size {
                disk.truncate(ino, end, now)?;
            }
            Ok(())
        };
        match mode {
            0 => grow(&mut disk)?,
            FALLOC_FL_KEEP_SIZE => {}
            m if m == FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE => disk.zero(ino, offset, len, now)?,
            FALLOC_FL_ZERO_RANGE | FALLOC_FL_WRITE_ZEROES => {
                disk.zero(ino, offset, len, now)?;
                grow(&mut disk)?;
            }
            m if m == FALLOC_FL_ZERO_RANGE | FALLOC_FL_KEEP_SIZE => disk.zero(ino, offset, len, now)?,
            _ => return Err(EOPNOTSUPP),
        }
        Ok(0)
    }

    /// `readahead`: a file on the disk, open to read, is all it checks.
    pub(crate) fn readahead(&mut self, fd: i32) -> Result<u64, Errno> {
        let open = self.file(fd)?;
        if !open.readable() {
            return Err(EBADF);
        }
        match open.node {
            Node::File(_) | Node::Memfd(_, false) => Ok(0),
            _ => Err(EINVAL),
        }
    }

    /// `sync_file_range`: checked; with `SYNC_FILE_RANGE_WAIT_AFTER`, the disk is synced, as `fsync` does.
    pub(crate) fn sync_file_range(&mut self, fd: i32, offset: u64, nbytes: u64, flags: u64) -> Result<u64, Errno> {
        const WAIT_AFTER: u64 = 4;
        if flags & !7 != 0 || (offset as i64) < 0 || (nbytes as i64) < 0 {
            return Err(EINVAL);
        }
        match self.file(fd)?.node {
            Node::File(_) | Node::Dir(_) => {}
            _ => return Err(ESPIPE),
        }
        if flags & WAIT_AFTER != 0 {
            lock(&self.k.disk).sync()?;
        }
        Ok(0)
    }
}
