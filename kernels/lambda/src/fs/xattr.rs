//! Extended attributes: `setxattr`, `getxattr`, `listxattr`, `removexattr`, their `l` (don't follow a symlink) and
//! `f` (by descriptor) forms, and the `*xattrat` calls. The attributes themselves are kept by `layers`, with the
//! file; this is Linux's rules over them — the `user.`, `trusted.`, `security.` and `system.` namespaces, who may
//! touch each, `XATTR_CREATE`/`XATTR_REPLACE`, and the sizing of `get` and `list`.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use layers::{Ino, Kind};

use super::Node;
use crate::abi::*;
use crate::process::Task;
use crate::process::creds::CAP_SYS_ADMIN;

const XATTR_CREATE: u64 = 1;
const XATTR_REPLACE: u64 = 2;
const XATTR_NAME_MAX: usize = 255;
const XATTR_SIZE_MAX: usize = 65536;
const XATTR_LIST_MAX: usize = 65536;

/// Which call form names the file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    /// A path, following a symlink at the end (`setxattr`).
    Path,
    /// A path, not following (`lsetxattr`).
    Link,
    /// An open descriptor (`fsetxattr`).
    Fd,
}

impl Task {
    /// The disk inode a form names, and its kind. A file lambda makes (under `/proc`) has no attributes (ENOTSUP).
    fn xattr_target(&mut self, cx: &mut Cx<'_>, form: Form, at: u64) -> Result<(Ino, Kind), Errno> {
        let ino = match form {
            Form::Fd => match self.file(at as i32)?.node {
                Node::File(ino) | Node::Dir(ino) => ino,
                _ => return Err(ENOTSUP),
            },
            Form::Path | Form::Link => {
                let path = self.read_path(cx, at)?;
                let (dir, dir_path) = self.start_dir(AT_FDCWD)?;
                if crate::os::stat(&self.k, &crate::process::exe_path(&dir_path, &path)).is_some() {
                    return Err(ENOTSUP);
                }
                self.view().resolve(&lock(&self.k.disk), dir, &path, form == Form::Path)?
            }
        };
        let kind = lock(&self.k.disk).stat(ino)?.kind;
        Ok((ino, kind))
    }

    /// An attribute name, checked: a known namespace, within length, and allowed on this file for `writing`.
    fn check_name(&self, name: &[u8], kind: Kind, writing: bool) -> Result<String, Errno> {
        if name.is_empty() || name.len() > XATTR_NAME_MAX {
            return Err(ERANGE);
        }
        let name = std::str::from_utf8(name).map_err(|_| EINVAL)?;
        let namespace = name.split('.').next().unwrap_or("");
        let root = lock(&self.process.state).creds.capable(CAP_SYS_ADMIN);
        match namespace {
            // `user.` only on regular files and directories (not symlinks or special files).
            "user" if name.len() > 5 => {
                if !matches!(kind, Kind::File | Kind::Dir) {
                    return Err(EPERM);
                }
            }
            "trusted" if name.len() > 8 => {
                if !root {
                    // Hidden from the unprivileged: ENODATA on read, EPERM on write.
                    return Err(if writing { EPERM } else { ENODATA });
                }
            }
            "security" if name.len() > 9 => {
                if writing && !root {
                    return Err(EPERM);
                }
            }
            "system" if name.len() > 7 => {}
            _ => return Err(ENOTSUP),
        }
        Ok(name.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn setxattr(&mut self, cx: &mut Cx<'_>, form: Form, at: u64, name_at: u64, value_at: u64, size: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 || flags & XATTR_CREATE != 0 && flags & XATTR_REPLACE != 0 {
            return Err(EINVAL);
        }
        if size as usize > XATTR_SIZE_MAX {
            return Err(E2BIG);
        }
        let (ino, kind) = self.xattr_target(cx, form, at)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, true)?;
        let value = self.mem(cx, false, |t| user::read_vec(&t.space(), value_at, size as usize))?;
        let present = lock(&self.k.disk).get_xattr(ino, &name)?.is_some();
        if flags & XATTR_CREATE != 0 && present {
            return Err(EEXIST);
        }
        if flags & XATTR_REPLACE != 0 && !present {
            return Err(ENODATA);
        }
        lock(&self.k.disk).set_xattr(ino, &name, &value)?;
        Ok(0)
    }

    pub(crate) fn getxattr(&mut self, cx: &mut Cx<'_>, form: Form, at: u64, name_at: u64, value_at: u64, size: u64) -> Result<u64, Errno> {
        let (ino, kind) = self.xattr_target(cx, form, at)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, false)?;
        let value = lock(&self.k.disk).get_xattr(ino, &name)?.ok_or(ENODATA)?;
        if size == 0 {
            return Ok(value.len() as u64);
        }
        if value.len() > size as usize {
            return Err(ERANGE);
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), value_at, &value))?;
        Ok(value.len() as u64)
    }

    pub(crate) fn listxattr(&mut self, cx: &mut Cx<'_>, form: Form, at: u64, list_at: u64, size: u64) -> Result<u64, Errno> {
        let (ino, _) = self.xattr_target(cx, form, at)?;
        let root = lock(&self.process.state).creds.capable(CAP_SYS_ADMIN);
        let names = lock(&self.k.disk).list_xattrs(ino)?;
        // `trusted.` attributes aren't shown to the unprivileged.
        let mut out = Vec::new();
        for name in names.iter().filter(|n| root || !n.starts_with("trusted.")) {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
        if out.len() > XATTR_LIST_MAX {
            return Err(E2BIG);
        }
        if size == 0 {
            return Ok(out.len() as u64);
        }
        if out.len() > size as usize {
            return Err(ERANGE);
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), list_at, &out))?;
        Ok(out.len() as u64)
    }

    pub(crate) fn removexattr(&mut self, cx: &mut Cx<'_>, form: Form, at: u64, name_at: u64) -> Result<u64, Errno> {
        let (ino, kind) = self.xattr_target(cx, form, at)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, true)?;
        if !lock(&self.k.disk).remove_xattr(ino, &name)? {
            return Err(ENODATA);
        }
        Ok(0)
    }

    // The 6.13 `*xattrat` calls: a dirfd and path (with `AT_EMPTY_PATH`, the fd itself), `at_flags`, the name, and a
    // `struct xattr_args { __u64 value; __u32 size; __u32 flags; }` (none for remove/list's value).

    fn at_target(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at_flags: u64) -> Result<(Ino, Kind), Errno> {
        if at_flags & !(AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH) != 0 {
            return Err(EINVAL);
        }
        let path = self.read_path(cx, path_at)?;
        if path.is_empty() {
            if at_flags & AT_EMPTY_PATH == 0 {
                return Err(ENOENT);
            }
            return match self.file(dfd)?.node {
                Node::File(ino) | Node::Dir(ino) => Ok((ino, lock(&self.k.disk).stat(ino)?.kind)),
                _ => Err(ENOTSUP),
            };
        }
        let (dir, dir_path) = self.start_dir(dfd)?;
        if crate::os::stat(&self.k, &crate::process::exe_path(&dir_path, &path)).is_some() {
            return Err(ENOTSUP);
        }
        let ino = self.view().resolve(&lock(&self.k.disk), dir, &path, at_flags & AT_SYMLINK_NOFOLLOW == 0)?;
        Ok((ino, lock(&self.k.disk).stat(ino)?.kind))
    }

    /// `struct xattr_args` at `args_at`: the value pointer, its size, and the flags.
    fn xattr_args(&mut self, cx: &mut Cx<'_>, args_at: u64, size: u64) -> Result<(u64, u64, u64), Errno> {
        if size < 16 {
            return Err(EINVAL);
        }
        self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), args_at)?, u64::from(user::read_u32(&t.space(), args_at + 8)?), u64::from(user::read_u32(&t.space(), args_at + 12)?))))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn setxattrat(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at_flags: u64, name_at: u64, args_at: u64, size: u64) -> Result<u64, Errno> {
        let (value_at, value_size, flags) = self.xattr_args(cx, args_at, size)?;
        if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0 || value_size as usize > XATTR_SIZE_MAX {
            return Err(if value_size as usize > XATTR_SIZE_MAX { E2BIG } else { EINVAL });
        }
        let (ino, kind) = self.at_target(cx, dfd, path_at, at_flags)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, true)?;
        let value = self.mem(cx, false, |t| user::read_vec(&t.space(), value_at, value_size as usize))?;
        let present = lock(&self.k.disk).get_xattr(ino, &name)?.is_some();
        if flags & XATTR_CREATE != 0 && present {
            return Err(EEXIST);
        }
        if flags & XATTR_REPLACE != 0 && !present {
            return Err(ENODATA);
        }
        lock(&self.k.disk).set_xattr(ino, &name, &value)?;
        Ok(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn getxattrat(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at_flags: u64, name_at: u64, args_at: u64, size: u64) -> Result<u64, Errno> {
        let (value_at, value_size, _) = self.xattr_args(cx, args_at, size)?;
        let (ino, kind) = self.at_target(cx, dfd, path_at, at_flags)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, false)?;
        let value = lock(&self.k.disk).get_xattr(ino, &name)?.ok_or(ENODATA)?;
        if value_size == 0 {
            return Ok(value.len() as u64);
        }
        if value.len() > value_size as usize {
            return Err(ERANGE);
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), value_at, &value))?;
        Ok(value.len() as u64)
    }

    pub(crate) fn listxattrat(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at_flags: u64, list_at: u64, size: u64) -> Result<u64, Errno> {
        let (ino, _) = self.at_target(cx, dfd, path_at, at_flags)?;
        let root = lock(&self.process.state).creds.capable(CAP_SYS_ADMIN);
        let names = lock(&self.k.disk).list_xattrs(ino)?;
        let mut out = Vec::new();
        for name in names.iter().filter(|n| root || !n.starts_with("trusted.")) {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
        if size == 0 {
            return Ok(out.len() as u64);
        }
        if out.len() > size as usize {
            return Err(ERANGE);
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), list_at, &out))?;
        Ok(out.len() as u64)
    }

    pub(crate) fn removexattrat(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at_flags: u64, name_at: u64) -> Result<u64, Errno> {
        let (ino, kind) = self.at_target(cx, dfd, path_at, at_flags)?;
        let raw = self.mem(cx, false, |t| user::read_cstr(&t.space(), name_at, XATTR_NAME_MAX + 1))?;
        let name = self.check_name(&raw, kind, true)?;
        if !lock(&self.k.disk).remove_xattr(ino, &name)? {
            return Err(ENODATA);
        }
        Ok(0)
    }
}

impl Task {
    /// `file_getattr` (6.17): a file's `struct file_attr` (XFS's `fsxattr` by path). lambda keeps none of these, so it
    /// reads back zeros — no flags, no project id, no extent hints.
    pub(crate) fn file_getattr(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at: u64, size: u64, at_flags: u64) -> Result<u64, Errno> {
        if !(8..=256).contains(&size) {
            return Err(EINVAL);
        }
        self.at_target(cx, dfd, path_at, at_flags)?;
        let zeros = vec![0u8; size as usize];
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &zeros))?;
        Ok(0)
    }

    /// `file_setattr` (6.17): nothing of a `file_attr` can be set here, so a request that would change something is
    /// EOPNOTSUPP; one that changes nothing is accepted.
    pub(crate) fn file_setattr(&mut self, cx: &mut Cx<'_>, dfd: i32, path_at: u64, at: u64, size: u64, at_flags: u64) -> Result<u64, Errno> {
        if !(8..=256).contains(&size) {
            return Err(EINVAL);
        }
        self.at_target(cx, dfd, path_at, at_flags)?;
        let given = self.mem(cx, false, |t| user::read_vec(&t.space(), at, size as usize))?;
        if given.iter().any(|b| *b != 0) {
            return Err(EOPNOTSUPP);
        }
        Ok(0)
    }
}
