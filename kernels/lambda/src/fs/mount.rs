//! Mounts, a process's root, and the path walk that knows both: `mount`, `umount2`, `pivot_root`, `chroot`, `fchroot`,
//! and `/proc`'s `mounts` and `mountinfo`.
//!
//! A mount covers a directory (or a file) on the disk with the root of what's mounted: a directory elsewhere on it
//! (a bind mount), or a tmpfs, which is a volatile tree of the disk's own (`layers` never logs or saves it). The walk
//! goes component by component: a name that's covered leads into what covers it, `..` at a mount's root leads out to
//! where it's mounted, and neither `..` nor an absolute link climbs above the process's root. `proc`, `devpts` and
//! `mqueue` mounts are listed, and what's under them is made by lambda, found by path as it always was.
//!
//! The table is a mount namespace's, held by each process: today every process has the machine's one. A mount's
//! `path` is where it is in the namespace; a chrooted process sees the ones under its root, from there. Whether a
//! path is on a read-only mount is told by the deepest mount its path is under, as `/proc/self/mountinfo` would say.

use std::sync::{Arc, Mutex};

use kernels_core::lock;
use kernels_core::thread::Cx;
use layers::{Error, Ino, Kind, ROOT};

use super::{Node, Open};
use crate::abi::*;
use crate::process::creds::{CAP_SYS_ADMIN, CAP_SYS_CHROOT};
use crate::process::{Task, exe_path};
use crate::{Disk, Meta};

pub(crate) const MS_RDONLY: u64 = 1;
const MS_NOSUID: u64 = 2;
const MS_NODEV: u64 = 4;
const MS_NOEXEC: u64 = 8;
const MS_REMOUNT: u64 = 32;
const MS_NOSYMFOLLOW: u64 = 256;
const MS_NOATIME: u64 = 1024;
const MS_NODIRATIME: u64 = 2048;
const MS_BIND: u64 = 4096;
const MS_MOVE: u64 = 8192;
const MS_REC: u64 = 16384;
const MS_UNBINDABLE: u64 = 1 << 17;
const MS_PRIVATE: u64 = 1 << 18;
const MS_SLAVE: u64 = 1 << 19;
const MS_SHARED: u64 = 1 << 20;
const MS_RELATIME: u64 = 1 << 21;
const MS_STRICTATIME: u64 = 1 << 24;
/// What a mount itself keeps (the rest are its file system's, or the call's).
const PER_MOUNT: u64 = MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC | MS_NOSYMFOLLOW | MS_NOATIME | MS_NODIRATIME | MS_RELATIME | MS_STRICTATIME;
const PROPAGATION: u64 = MS_UNBINDABLE | MS_PRIVATE | MS_SLAVE | MS_SHARED;
/// What old `mount` callers put in the high half of the flags.
const MS_MGC_MSK: u64 = 0xffff_0000;
const MS_MGC_VAL: u64 = 0xc0ed_0000;
const MNT_FORCE: u64 = 1;
const MNT_DETACH: u64 = 2;
const MNT_EXPIRE: u64 = 4;
const UMOUNT_NOFOLLOW: u64 = 8;
/// How many links one walk follows.
const MOST_LINKS: usize = 40;

/// What a mount has: the disk (or part of it), a tmpfs, or one of the file systems lambda makes as they're read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum What {
    Disk,
    Tmpfs,
    Proc,
    Devpts,
    Mqueue,
}

impl What {
    /// `statfs`'s `f_type`.
    pub fn magic(self) -> u64 {
        match self {
            What::Disk => 0xef53,
            What::Tmpfs => 0x0102_1994,
            What::Proc => 0x9fa0,
            What::Devpts => 0x1cd1,
            What::Mqueue => 0x1980_0202,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Mount {
    pub id: u32,
    pub parent: u32,
    /// What it covers, and where that is in the namespace.
    pub point: Ino,
    pub path: String,
    /// The root of what's mounted, and where that is in its own file system.
    pub root: Ino,
    pub root_path: String,
    pub what: What,
    pub source: String,
    pub fstype: String,
    /// `MS_RDONLY`, `MS_NOSUID` and the rest a mount keeps; and its propagation (`MS_SHARED`, ...).
    pub flags: u64,
    pub propagation: u64,
    /// Its file system's own options, as `mounts` shows them after the mount's.
    pub options: String,
    /// A tmpfs's volatile tree, let go when no mount has it any more.
    pub tree: Option<Ino>,
}

/// A mount namespace's mounts, in the order they were made.
#[derive(Debug, Clone)]
pub(crate) struct Mounts {
    pub list: Vec<Mount>,
    next: u32,
}

/// A mount namespace: its mounts, replaced whole as they change, so a walk takes them without a lock held.
#[derive(Debug)]
pub(crate) struct MountNs {
    id: u64,
    mounts: Mutex<Arc<Mounts>>,
}

impl MountNs {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// A copy, with its own inode: `clone`, `unshare` and `setns` of a mount namespace.
    pub fn duplicate(&self) -> Arc<MountNs> {
        Arc::new(MountNs { id: crate::ns::next_ns_ino(), mounts: Mutex::new(self.current()) })
    }

    pub fn current(&self) -> Arc<Mounts> {
        lock(&self.mounts).clone()
    }

    fn change<R>(&self, f: impl FnOnce(&mut Mounts) -> Result<R, Errno>) -> Result<R, Errno> {
        let mut mounts = lock(&self.mounts);
        let mut next = (**mounts).clone();
        let result = f(&mut next)?;
        *mounts = Arc::new(next);
        Ok(result)
    }
}

/// Whether `path` is `under` or under it, component by component.
fn under(path: &str, top: &str) -> bool {
    top == "/" || path == top || path.strip_prefix(top).is_some_and(|rest| rest.starts_with('/'))
}

/// `path` with `from` at its start replaced by `to`.
fn rebase(path: &str, from: &str, to: &str) -> String {
    let rest = match from {
        "/" if path == "/" => "",
        "/" => path,
        _ => &path[from.len()..],
    };
    match (to, rest) {
        (_, "") => to.to_string(),
        ("/", rest) => rest.to_string(),
        (to, rest) => format!("{to}{rest}"),
    }
}

/// `/proc`'s escaping of a path in `mounts`: space, tab, newline and backslash as octal.
fn escape(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            ' ' => "\\040".to_string(),
            '\t' => "\\011".to_string(),
            '\n' => "\\012".to_string(),
            '\\' => "\\134".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// A tmpfs's device number, as its files' `st_dev` and `mountinfo` give it.
pub(crate) fn tmpfs_dev(tree: Ino) -> u64 {
    40 + tree % 200
}

impl Mounts {
    /// The machine's at boot: the disk at `/`, and what lambda makes at `/proc` and `/dev/pts`.
    pub fn boot(disk: &Disk) -> Mounts {
        let mut mounts = Mounts { list: Vec::new(), next: 1 };
        let root = Mount {
            id: 0,
            parent: 0,
            point: ROOT,
            path: "/".to_string(),
            root: ROOT,
            root_path: "/".to_string(),
            what: What::Disk,
            source: "/dev/root".to_string(),
            fstype: "ext4".to_string(),
            flags: MS_RELATIME,
            propagation: MS_PRIVATE,
            options: String::new(),
            tree: None,
        };
        mounts.add(root);
        for (path, what, fstype, source, flags, options) in [
            ("/proc", What::Proc, "proc", "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC | MS_RELATIME, ""),
            ("/dev/pts", What::Devpts, "devpts", "devpts", MS_NOSUID | MS_NOEXEC | MS_RELATIME, "gid=5,mode=620,ptmxmode=666"),
        ] {
            if let Ok(point) = disk.resolve(ROOT, path, true) {
                let mount = Mount { id: 0, parent: 1, point, path: path.to_string(), root: point, root_path: "/".to_string(), what, source: source.to_string(), fstype: fstype.to_string(), flags, propagation: MS_PRIVATE, options: options.to_string(), tree: None };
                mounts.add(mount);
            }
        }
        mounts
    }

    fn add(&mut self, mut mount: Mount) -> u32 {
        mount.id = self.next;
        self.next += 1;
        let id = mount.id;
        self.list.push(mount);
        id
    }

    /// What's mounted over `ino`, all the way up a stack of mounts on it; `ino` if nothing is.
    pub fn cover(&self, ino: Ino) -> Ino {
        let mut at = ino;
        for _ in 0..=self.list.len() {
            match self.list.iter().rev().find(|m| m.point == at && m.root != at && m.what != What::Proc && m.what != What::Devpts && m.what != What::Mqueue) {
                Some(m) => at = m.root,
                None => break,
            }
        }
        at
    }

    /// Where the mount whose root `ino` is is mounted, for `..` there: the latest, for one mounted in two places.
    pub fn up(&self, ino: Ino) -> Option<Ino> {
        self.list.iter().rev().find(|m| m.root == ino && m.point != ino).map(|m| m.point)
    }

    /// The mount a path (in the namespace) is on: the deepest one it's under, the latest of those.
    pub fn containing(&self, path: &str) -> &Mount {
        self.list.iter().rev().filter(|m| under(path, &m.path)).max_by_key(|m| m.path.len()).unwrap_or(&self.list[0])
    }

    /// The mount at exactly `path` (the latest there), if there's one.
    fn at(&self, path: &str) -> Option<usize> {
        self.list.iter().rposition(|m| m.path == path)
    }

    /// `/proc/<pid>/mounts`, as a process with its root at `root` (a path in the namespace) sees them.
    pub fn mounts_text(&self, root: &str) -> String {
        self.visible(root)
            .map(|(m, path)| format!("{} {} {} {} 0 0\n", escape(&m.source), escape(&path), m.fstype, Mount::options(m.flags, &m.options)))
            .collect()
    }

    /// `/proc/<pid>/mountinfo`.
    pub fn mountinfo_text(&self, root: &str) -> String {
        self.visible(root)
            .map(|(m, path)| {
                let (major, minor) = m.dev();
                let dev = format!("{major}:{minor}");
                let optional = match m.propagation {
                    MS_SHARED => format!(" shared:{}", m.id),
                    MS_SLAVE => " master:1".to_string(),
                    MS_UNBINDABLE => " unbindable".to_string(),
                    _ => String::new(),
                };
                let flags = Mount::options(m.flags, "");
                let sup = Mount::options(m.flags & MS_RDONLY, &m.options);
                format!("{} {} {dev} {} {} {flags}{optional} - {} {} {sup}\n", m.id, m.parent, escape(&m.root_path), escape(&path), m.fstype, escape(&m.source))
            })
            .collect()
    }

    /// The mounts under `root`, each with its path as seen from there.
    fn visible<'a>(&'a self, root: &'a str) -> impl Iterator<Item = (&'a Mount, String)> + 'a {
        self.list.iter().filter(move |m| under(&m.path, root)).map(move |m| (m, rebase(&m.path, root, "/")))
    }
}

impl Mount {
    /// Its file system's device, as `mountinfo` and `statmount` give it.
    pub fn dev(&self) -> (u32, u32) {
        match self.what {
            What::Disk => (8, 3),
            What::Tmpfs => (0, tmpfs_dev(self.tree.unwrap_or(0)) as u32),
            What::Proc => (0, 22),
            What::Devpts => (0, 24),
            What::Mqueue => (0, 25),
        }
    }

    /// Its 64-bit ID, as `statmount` and `listmount` know it: past every old 32-bit one, as Linux's are.
    pub fn unique_id(&self) -> u64 {
        MNT_UNIQUE_ID_OFFSET + u64::from(self.id)
    }

    /// `MOUNT_ATTR_*`, from the `MS_*` flags it keeps.
    fn attr(&self) -> u64 {
        let mut attr = self.flags & (MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC);
        if self.flags & MS_NOATIME != 0 {
            attr |= 0x10;
        } else if self.flags & (1 << 24) != 0 {
            attr |= 0x20;
        }
        if self.flags & MS_NODIRATIME != 0 {
            attr |= 0x80;
        }
        if self.flags & MS_NOSYMFOLLOW != 0 {
            attr |= 0x0020_0000;
        }
        attr
    }
    /// A mount's flags as `mounts` writes them, then its file system's options.
    fn options(flags: u64, fs: &str) -> String {
        let mut words = vec![if flags & MS_RDONLY != 0 { "ro" } else { "rw" }];
        for (bit, word) in [(MS_NOSUID, "nosuid"), (MS_NODEV, "nodev"), (MS_NOEXEC, "noexec"), (MS_NOSYMFOLLOW, "nosymfollow"), (MS_NOATIME, "noatime"), (MS_NODIRATIME, "nodiratime"), (MS_RELATIME, "relatime")] {
            if flags & bit != 0 {
                words.push(word);
            }
        }
        let mut out = words.join(",");
        if !fs.is_empty() {
            out.push(',');
            out.push_str(fs);
        }
        out
    }
}

/// How a process sees paths: from its root, through its namespace's mounts.
#[derive(Clone)]
pub(crate) struct View {
    pub root: Ino,
    pub mounts: Arc<Mounts>,
}

impl View {
    /// Follows `path` from `start` (or the root, for an absolute one); links along the way are followed, and the last
    /// one too if `follow`. As `layers`' own, but for mounts and the process's root.
    pub fn resolve(&self, disk: &Disk, start: Ino, path: &str, follow: bool) -> layers::Result<Ino> {
        let mut links = 0;
        self.walk(disk, start, path, follow, &mut links)
    }

    /// Like `resolve`, but stops at the last component's directory, giving it and the last name.
    pub fn resolve_parent<'p>(&self, disk: &Disk, start: Ino, path: &'p str) -> layers::Result<(Ino, &'p str)> {
        let trimmed = path.trim_end_matches('/');
        let (dir, name) = match trimmed.rfind('/') {
            None => (start, trimmed),
            Some(0) => (self.root, &trimmed[1..]),
            Some(at) => (self.resolve(disk, start, &trimmed[..at], true)?, &trimmed[at + 1..]),
        };
        if name.is_empty() {
            return Err(Error::BadName);
        }
        Ok((dir, name))
    }

    fn walk(&self, disk: &Disk, start: Ino, path: &str, follow: bool, links: &mut usize) -> layers::Result<Ino> {
        self.walk_how(disk, start, path, follow, links, &Resolve::default()).map_err(|e| e.0)
    }

    /// `openat2`'s resolution: as `walk`, but honouring `RESOLVE_*`. `how.start` is the floor for `RESOLVE_BENEATH`
    /// and the root for `RESOLVE_IN_ROOT`.
    pub fn resolve_how(&self, disk: &Disk, start: Ino, path: &str, follow: bool, how: &Resolve) -> Result<Ino, Errno> {
        let mut links = 0;
        self.walk_how(disk, start, path, follow, &mut links, how).map_err(|e| e.into())
    }

    fn walk_how(&self, disk: &Disk, start: Ino, path: &str, follow: bool, links: &mut usize, how: &Resolve) -> Result<Ino, Walk> {
        // RESOLVE_IN_ROOT makes the start the root; an absolute path and `..` stay within it.
        let root = if how.in_root { start } else { self.root };
        let mut at = if path.starts_with('/') { root } else { start };
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
        for (i, part) in parts.iter().enumerate() {
            let last = i + 1 == parts.len();
            if *part == ".." {
                // RESOLVE_BENEATH forbids climbing above the start; RESOLVE_IN_ROOT pins `..` at the root.
                if how.beneath && at == start {
                    return Err(Walk(Error::NotFound, Some(EXDEV)));
                }
                if at == root {
                    continue;
                }
                for _ in 0..=self.mounts.list.len() {
                    match self.mounts.up(at) {
                        Some(point) if at != root => at = point,
                        _ => break,
                    }
                }
                if at != root {
                    at = disk.lookup(at, "..").map_err(Walk::from)?;
                }
                continue;
            }
            let found = disk.lookup(at, part).map_err(Walk::from)?;
            let next = self.mounts.cover(found);
            // RESOLVE_NO_XDEV: neither crossing into a mount nor following `..` out of one (the `..` above is covered).
            if how.no_xdev && next != found {
                return Err(Walk(Error::NotFound, Some(EXDEV)));
            }
            if disk.stat(next).map_err(Walk::from)?.kind == Kind::Symlink && (!last || follow) {
                if how.no_symlinks {
                    return Err(Walk(Error::Loop, Some(ELOOP)));
                }
                *links += 1;
                if *links > MOST_LINKS {
                    return Err(Walk(Error::Loop, None));
                }
                // An absolute symlink under RESOLVE_IN_ROOT is taken from the root, not the machine's.
                let target = disk.read_link(next).map_err(Walk::from)?.to_string();
                at = self.walk_how(disk, at, &target, true, links, how)?;
            } else {
                at = next;
            }
        }
        Ok(at)
    }
}

/// What a resolve-with-options walk can fail with: a `layers` error, and the errno to prefer for it (for `RESOLVE_*`
/// violations that `layers` has no error of its own for).
struct Walk(Error, Option<Errno>);

impl From<Error> for Walk {
    fn from(e: Error) -> Walk {
        Walk(e, None)
    }
}

impl From<Walk> for Errno {
    fn from(w: Walk) -> Errno {
        w.1.unwrap_or_else(|| w.0.into())
    }
}

/// `openat2`'s `RESOLVE_*` restrictions.
#[derive(Default, Clone, Copy)]
pub(crate) struct Resolve {
    pub no_xdev: bool,
    pub no_symlinks: bool,
    pub beneath: bool,
    pub in_root: bool,
}

/// A file system lambda can mount, from its `mount`/`fsopen` type name.
fn what_of(fstype: &str) -> Option<What> {
    match fstype {
        "tmpfs" => Some(What::Tmpfs),
        "proc" => Some(What::Proc),
        "devpts" => Some(What::Devpts),
        "mqueue" => Some(What::Mqueue),
        // sysfs, cgroups, overlay and disks aren't here.
        _ => None,
    }
}

/// The per-mount flags a `mount`/`fsmount` call keeps, with an atime default of `relatime`.
fn mount_flags(flags: u64) -> u64 {
    let f = flags & PER_MOUNT;
    if f & (MS_NOATIME | MS_STRICTATIME) == 0 { f | MS_RELATIME } else { f & !MS_STRICTATIME }
}

impl Task {
    /// How this process sees paths now.
    pub(crate) fn view(&self) -> View {
        let state = lock(&self.process.state);
        View { root: state.root.0, mounts: state.ns.mnt.current() }
    }

    /// A path as this process sees it (absolute, from its root), as it is in the namespace.
    pub(crate) fn ns_path(&self, visible: &str) -> String {
        let root = lock(&self.process.state).root.1.clone();
        if root == "/" { visible.to_string() } else { rebase(visible, "/", &root) }
    }

    /// EROFS if `visible` (an absolute path as this process sees it) is on a read-only mount.
    pub(crate) fn writable(&self, visible: &str) -> Result<(), Errno> {
        let path = self.ns_path(visible);
        if self.view().mounts.containing(&path).flags & MS_RDONLY != 0 { Err(EROFS) } else { Ok(()) }
    }

    /// The mount `visible` is on, its flags, and for a tmpfs its tree and `size=`: for `statfs`.
    pub(crate) fn mount_of(&self, visible: &str) -> (What, u64, Option<(Ino, Option<u64>)>) {
        let path = self.ns_path(visible);
        let mounts = self.view().mounts;
        let m = mounts.containing(&path);
        let memory = lock(&self.k.exec).memory().size().limit;
        let size = m.options.split(',').find_map(|o| o.strip_prefix("size=")).and_then(|s| size_of(s, memory));
        (m.what, m.flags, m.tree.map(|t| (t, size)))
    }

    fn may_mount(&self) -> Result<(), Errno> {
        if lock(&self.process.state).creds.capable(CAP_SYS_ADMIN) { Ok(()) } else { Err(EPERM) }
    }

    /// A path argument: the inode it leads to (followed, or not), and where it is, as seen and in the namespace.
    fn mount_path(&mut self, cx: &mut Cx<'_>, at: u64, follow: bool) -> Result<(Ino, String), Errno> {
        let path = self.read_path(cx, at)?;
        if path.is_empty() {
            return Err(ENOENT);
        }
        let (dir, dir_path) = self.start_dir(AT_FDCWD)?;
        let view = self.view();
        let ino = view.resolve(&lock(&self.k.disk), dir, &path, follow)?;
        Ok((ino, self.ns_path(&exe_path(&dir_path, &path))))
    }

    pub(crate) fn mount(&mut self, cx: &mut Cx<'_>, source_at: u64, target_at: u64, type_at: u64, flags: u64, data_at: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        let flags = if flags & MS_MGC_MSK == MS_MGC_VAL { flags & !MS_MGC_MSK } else { flags };
        let (target, target_path) = self.mount_path(cx, target_at, true)?;
        let ns = lock(&self.process.state).ns.mnt.clone();
        if flags & MS_REMOUNT != 0 {
            let data = if data_at == 0 { String::new() } else { String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), data_at, 4096))?).into_owned() };
            return ns.change(|mounts| {
                let i = mounts.at(&target_path).ok_or(EINVAL)?;
                let m = &mut mounts.list[i];
                m.flags = mount_flags(flags);
                if flags & MS_BIND == 0 && !data.is_empty() && m.what == What::Tmpfs {
                    m.options = tmpfs_options(&data, &m.options);
                }
                Ok(0)
            });
        }
        if flags & PROPAGATION != 0 {
            let kind = flags & PROPAGATION;
            if !kind.is_power_of_two() {
                return Err(EINVAL);
            }
            return ns.change(|mounts| {
                let i = mounts.at(&target_path).ok_or(EINVAL)?;
                let top = mounts.list[i].path.clone();
                for m in &mut mounts.list {
                    if m.path == top || (flags & MS_REC != 0 && under(&m.path, &top)) {
                        m.propagation = kind;
                    }
                }
                Ok(0)
            });
        }
        if flags & MS_MOVE != 0 {
            let (_, source_path) = self.mount_path(cx, source_at, true)?;
            if under(&target_path, &source_path) {
                return Err(ELOOP);
            }
            return ns.change(|mounts| {
                let i = mounts.at(&source_path).ok_or(EINVAL)?;
                if i == 0 {
                    return Err(EINVAL);
                }
                let parent = mounts.containing(&target_path).id;
                mounts.list[i].point = target;
                mounts.list[i].parent = parent;
                for m in &mut mounts.list {
                    if under(&m.path, &source_path) {
                        m.path = rebase(&m.path, &source_path, &target_path);
                    }
                }
                Ok(0)
            });
        }
        let target_kind = lock(&self.k.disk).stat(target)?.kind;
        if flags & MS_BIND != 0 {
            let (source, source_path) = self.mount_path(cx, source_at, true)?;
            let source_kind = lock(&self.k.disk).stat(source)?.kind;
            if (source_kind == Kind::Dir) != (target_kind == Kind::Dir) {
                return Err(ENOTDIR);
            }
            return ns.change(|mounts| {
                let from = mounts.containing(&source_path).clone();
                let parent = mounts.containing(&target_path).id;
                let root_path = rebase(&source_path, &from.path, &from.root_path);
                let mut copies = vec![Mount { id: 0, parent, point: target, path: target_path.clone(), root: source, root_path, flags: from.flags, propagation: MS_PRIVATE, ..from }];
                // MS_REC: what's mounted under the source is under the target too.
                if flags & MS_REC != 0 {
                    for m in mounts.list.iter().filter(|m| m.path != source_path && under(&m.path, &source_path)) {
                        copies.push(Mount { path: rebase(&m.path, &source_path, &target_path), propagation: MS_PRIVATE, ..m.clone() });
                    }
                }
                let first = mounts.next;
                for (n, mut m) in copies.into_iter().enumerate() {
                    if n > 0 {
                        m.parent = first;
                    }
                    mounts.add(m);
                }
                Ok(0)
            });
        }
        // A new mount of a file system.
        if type_at == 0 {
            return Err(EINVAL);
        }
        let fstype = String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), type_at, 256))?).into_owned();
        let source = if source_at == 0 { "none".to_string() } else { String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), source_at, 4096))?).into_owned() };
        let data = if data_at == 0 { String::new() } else { String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), data_at, 4096))?).into_owned() };
        let what = what_of(&fstype).ok_or(ENODEV)?;
        if target_kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        let (root, tree, options) = if what == What::Tmpfs {
            let options = tmpfs_options(&data, "");
            let mode = options.split(',').find_map(|o| o.strip_prefix("mode=")).and_then(|m| u32::from_str_radix(m, 8).ok()).unwrap_or(0o1777);
            let tree = lock(&self.k.disk).volatile_root(Meta { mode: mode & 0o7777, ..Meta::dir(0) }, self.k.now());
            (tree, Some(tree), options)
        } else {
            (target, None, data.clone())
        };
        ns.change(|mounts| {
            let parent = mounts.containing(&target_path).id;
            mounts.add(Mount { id: 0, parent, point: target, path: target_path.clone(), root, root_path: "/".to_string(), what, source, fstype, flags: mount_flags(flags), propagation: MS_PRIVATE, options, tree });
            Ok(0)
        })
    }

    pub(crate) fn umount2(&mut self, cx: &mut Cx<'_>, target_at: u64, flags: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        if flags & !(MNT_FORCE | MNT_DETACH | MNT_EXPIRE | UMOUNT_NOFOLLOW) != 0 || (flags & MNT_EXPIRE != 0 && flags & (MNT_FORCE | MNT_DETACH) != 0) {
            return Err(EINVAL);
        }
        let (_, target_path) = self.mount_path(cx, target_at, flags & UMOUNT_NOFOLLOW == 0)?;
        let ns = lock(&self.process.state).ns.mnt.clone();
        let (gone, left) = ns.change(|mounts| {
            let i = mounts.at(&target_path).ok_or(EINVAL)?;
            if i == 0 {
                return Err(EBUSY);
            }
            if flags & MNT_EXPIRE != 0 {
                return Err(EAGAIN);
            }
            let id = mounts.list[i].id;
            // Mounts on it go with it, lazily (MNT_DETACH), or it's busy.
            let below: Vec<u32> = mounts.list.iter().filter(|m| m.id != id && m.path != target_path && under(&m.path, &target_path)).map(|m| m.id).collect();
            if !below.is_empty() && flags & MNT_DETACH == 0 {
                return Err(EBUSY);
            }
            let gone: Vec<Mount> = mounts.list.iter().filter(|m| m.id == id || below.contains(&m.id)).cloned().collect();
            mounts.list.retain(|m| m.id != id && !below.contains(&m.id));
            Ok((gone, mounts.list.iter().filter_map(|m| m.tree).collect::<Vec<_>>()))
        })?;
        // A tmpfs no mount has any more goes.
        let mut disk = lock(&self.k.disk);
        for tree in gone.iter().filter_map(|m| m.tree) {
            if !left.contains(&tree) {
                let _ = disk.drop_tree(tree);
            }
        }
        Ok(0)
    }

    /// `pivot_root`: the mount at `new_root` becomes the namespace's root, and the old root is mounted at `put_old`
    /// (under it). Every process whose root or working directory was the old root's has the new one's.
    pub(crate) fn pivot_root(&mut self, cx: &mut Cx<'_>, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        let (new_root, new_path) = self.mount_path(cx, new_at, true)?;
        let (put_old, old_path) = self.mount_path(cx, old_at, true)?;
        if lock(&self.k.disk).stat(new_root)?.kind != Kind::Dir || lock(&self.k.disk).stat(put_old)?.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        if new_path == "/" || !under(&old_path, &new_path) {
            return Err(EINVAL);
        }
        let (ns, old_root) = {
            let state = lock(&self.process.state);
            (state.ns.mnt.clone(), state.root.0)
        };
        ns.change(|mounts| {
            let i = mounts.at(&new_path).ok_or(EINVAL)?;
            if i == 0 {
                return Err(EBUSY);
            }
            let new_id = mounts.list[i].id;
            let old_id = mounts.list[0].id;
            let put_old_path = rebase(&old_path, &new_path, "/");
            for m in &mut mounts.list {
                m.path = if under(&m.path, &new_path) { rebase(&m.path, &new_path, "/") } else { rebase(&m.path, "/", &put_old_path) };
            }
            // The new root first, as the namespace's own; the old one on put_old.
            let mut new = mounts.list.remove(i);
            new.point = new.root;
            new.parent = new.id;
            mounts.list[0].point = put_old;
            mounts.list[0].parent = new_id;
            mounts.list.insert(0, new);
            for m in &mut mounts.list {
                if m.parent == old_id && m.id != old_id && m.id != new_id && !under(&m.path, &put_old_path) {
                    m.parent = new_id;
                }
            }
            Ok(())
        })?;
        for process in lock(&self.k.processes).values() {
            let mut state = lock(&process.state);
            if state.root.0 == old_root {
                state.root = (new_root, "/".to_string());
            }
            if state.cwd.0 == old_root {
                state.cwd = (new_root, "/".to_string());
            }
        }
        Ok(0)
    }

    /// `chroot` and `fchroot`: the root this process (and what it starts) sees.
    pub(crate) fn chroot(&mut self, cx: &mut Cx<'_>, path_at: Option<u64>, fd: i32) -> Result<u64, Errno> {
        if !lock(&self.process.state).creds.capable(CAP_SYS_CHROOT) {
            return Err(EPERM);
        }
        let (ino, path) = match path_at {
            Some(at) => self.mount_path(cx, at, true)?,
            None => {
                let open = self.file(fd)?;
                match open.node {
                    Node::Dir(ino) => (ino, self.ns_path(&open.path)),
                    _ => return Err(ENOTDIR),
                }
            }
        };
        if lock(&self.k.disk).stat(ino)?.kind != Kind::Dir {
            return Err(ENOTDIR);
        }
        lock(&self.process.state).root = (ino, path);
        Ok(0)
    }
}

/// A tmpfs's `size=`: bytes, with k, m, g or t, or a percentage of the machine's memory.
fn size_of(text: &str, memory: u64) -> Option<u64> {
    if let Some(percent) = text.strip_suffix('%') {
        return percent.parse::<u64>().ok().map(|p| memory / 100 * p);
    }
    let (digits, shift) = match text.chars().last()?.to_ascii_lowercase() {
        'k' => (&text[..text.len() - 1], 10),
        'm' => (&text[..text.len() - 1], 20),
        'g' => (&text[..text.len() - 1], 30),
        't' => (&text[..text.len() - 1], 40),
        _ => (text, 0),
    };
    digits.parse::<u64>().ok().map(|n| n << shift)
}

/// A tmpfs's options, as `mounts` shows them: what was given that tmpfs takes (`size`, `nr_inodes`, `mode`, `uid`,
/// `gid`), over what it had.
fn tmpfs_options(data: &str, old: &str) -> String {
    let mut kept: Vec<String> = old.split(',').filter(|o| !o.is_empty()).map(String::from).collect();
    for option in data.split(',').filter(|o| !o.is_empty()) {
        let key = option.split('=').next().unwrap_or("");
        if matches!(key, "size" | "nr_inodes" | "mode" | "uid" | "gid") {
            kept.retain(|o| o.split('=').next() != Some(key));
            kept.push(option.to_string());
        }
    }
    kept.join(",")
}

impl MountNs {
    pub fn new(mounts: Mounts) -> Arc<MountNs> {
        Arc::new(MountNs { id: crate::ns::next_ns_ino(), mounts: Mutex::new(Arc::new(mounts)) })
    }
}

// ---- The new mount API (Linux 5.2+): fsopen, fsconfig, fsmount, fspick, move_mount, open_tree, mount_setattr. ----

const FSOPEN_CLOEXEC: u64 = 1;
const FSCONFIG_SET_FLAG: u64 = 0;
const FSCONFIG_SET_STRING: u64 = 1;
const FSCONFIG_SET_PATH: u64 = 3;
const FSCONFIG_SET_PATH_EMPTY: u64 = 4;
const FSCONFIG_SET_FD: u64 = 5;
const FSCONFIG_CMD_CREATE: u64 = 6;
const FSCONFIG_CMD_RECONFIGURE: u64 = 7;
const FSCONFIG_CMD_CREATE_EXCL: u64 = 8;
const FSMOUNT_CLOEXEC: u64 = 1;
const MOVE_MOUNT_F_EMPTY_PATH: u64 = 4;
const MOVE_MOUNT_T_EMPTY_PATH: u64 = 0x40;
const MOVE_MOUNT_MASK: u64 = 0x77;
const OPEN_TREE_CLONE: u64 = 1;
const OPEN_TREE_CLOEXEC: u64 = 0o2000000;
const AT_RECURSIVE: u64 = 0x8000;
const AT_EMPTY_PATH_: u64 = 0x1000;
/// `struct mount_attr`'s bits, which are the `MS_*` with the same meaning but their own numbers.
const MOUNT_ATTR_RDONLY: u64 = 1;
const MOUNT_ATTR_NOSUID: u64 = 2;
const MOUNT_ATTR_NODEV: u64 = 4;
const MOUNT_ATTR_NOEXEC: u64 = 8;
const MOUNT_ATTR_NODIRATIME: u64 = 0x80;
const MOUNT_ATTR_NOSYMFOLLOW: u64 = 0x200000;
const MOUNT_ATTR__ATIME: u64 = 0x70;
const MOUNT_ATTR_RELATIME: u64 = 0;
const MOUNT_ATTR_NOATIME: u64 = 0x10;

/// A filesystem context being built (`fsopen`) or an existing mount being reconfigured (`fspick`).
pub(crate) struct FsContext {
    what: What,
    source: String,
    /// Options as given (`size=1m`, `mode=700`), for tmpfs.
    options: String,
    flags: u64,
    /// For `fspick`: the mount it reconfigures, by path in the namespace.
    pick: Option<String>,
    created: bool,
}

/// A mount made by `fsmount` or `open_tree(OPEN_TREE_CLONE)` but not yet in a namespace, until `move_mount`.
pub(crate) struct Detached {
    what: What,
    root: Ino,
    root_path: String,
    source: String,
    fstype: String,
    flags: u64,
    options: String,
    tree: Option<Ino>,
}

/// `struct mount_attr` turned into the `MS_*` to set and clear, and the propagation to set (0 for none).
fn mount_attr(set: u64, clr: u64) -> Result<(u64, u64), Errno> {
    let known = MOUNT_ATTR_RDONLY | MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOEXEC | MOUNT_ATTR_NODIRATIME | MOUNT_ATTR_NOSYMFOLLOW | MOUNT_ATTR__ATIME;
    if set & !known != 0 || clr & !known != 0 || set & clr != 0 {
        return Err(EINVAL);
    }
    let map = |a: u64| {
        let mut f = 0;
        for (attr, ms) in [(MOUNT_ATTR_RDONLY, MS_RDONLY), (MOUNT_ATTR_NOSUID, MS_NOSUID), (MOUNT_ATTR_NODEV, MS_NODEV), (MOUNT_ATTR_NOEXEC, MS_NOEXEC), (MOUNT_ATTR_NODIRATIME, MS_NODIRATIME), (MOUNT_ATTR_NOSYMFOLLOW, MS_NOSYMFOLLOW)] {
            if a & attr != 0 {
                f |= ms;
            }
        }
        f
    };
    // The atime field is a value, not a flag: clearing it clears all three, setting picks one.
    let mut set_ms = map(set);
    let clr_ms = map(clr) | if clr & MOUNT_ATTR__ATIME != 0 { MS_NOATIME | MS_RELATIME } else { 0 };
    if set & MOUNT_ATTR__ATIME == MOUNT_ATTR_NOATIME {
        set_ms |= MS_NOATIME;
    } else if set & MOUNT_ATTR__ATIME == MOUNT_ATTR_RELATIME {
        set_ms |= MS_RELATIME;
    }
    Ok((set_ms, clr_ms))
}

impl Task {
    fn fscontext(&self, fd: i32) -> Result<Arc<Mutex<FsContext>>, Errno> {
        match &self.file(fd)?.node {
            Node::FsContext(c) => Ok(c.clone()),
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn fsopen(&mut self, cx: &mut Cx<'_>, type_at: u64, flags: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        if flags & !FSOPEN_CLOEXEC != 0 {
            return Err(EINVAL);
        }
        let fstype = String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), type_at, 256))?).into_owned();
        let what = what_of(&fstype).ok_or(ENODEV)?;
        let context = FsContext { what, source: "none".to_string(), options: String::new(), flags: 0, pick: None, created: false };
        let open = Open::new(Node::FsContext(Arc::new(Mutex::new(context))), O_RDWR, "anon_inode:[fscontext]");
        lock(&self.process.state).files.add(open, flags & FSOPEN_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    pub(crate) fn fspick(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        let (_, path) = self.at_mount_path(cx, dirfd, path_at, flags)?;
        let m = {
            let mounts = self.view().mounts;
            let m = mounts.containing(&path);
            FsContext { what: m.what, source: m.source.clone(), options: m.options.clone(), flags: m.flags, pick: Some(m.path.clone()), created: true }
        };
        let open = Open::new(Node::FsContext(Arc::new(Mutex::new(m))), O_RDWR, "anon_inode:[fscontext]");
        lock(&self.process.state).files.add(open, flags & 1 != 0, 0).map(|fd| fd as u64)
    }

    pub(crate) fn fsconfig(&mut self, cx: &mut Cx<'_>, fd: i32, cmd: u64, key_at: u64, value_at: u64, aux: u64) -> Result<u64, Errno> {
        let context = self.fscontext(fd)?;
        let key = if key_at == 0 { String::new() } else { String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), key_at, 256))?).into_owned() };
        match cmd {
            FSCONFIG_SET_FLAG => {
                lock(&context).options = tmpfs_options(&key, &lock(&context).options);
                Ok(0)
            }
            FSCONFIG_SET_STRING | FSCONFIG_SET_PATH | FSCONFIG_SET_PATH_EMPTY => {
                let value = if value_at == 0 { String::new() } else { String::from_utf8_lossy(&self.mem(cx, false, |t| kernels_core::user::read_cstr(&t.space(), value_at, 4096))?).into_owned() };
                let mut c = lock(&context);
                if key == "source" {
                    c.source = value;
                } else {
                    let pair = if value.is_empty() { key } else { format!("{key}={value}") };
                    c.options = tmpfs_options(&pair, &c.options);
                }
                Ok(0)
            }
            FSCONFIG_SET_FD => {
                let _ = (value_at, aux);
                Err(EINVAL)
            }
            FSCONFIG_CMD_CREATE | FSCONFIG_CMD_CREATE_EXCL => {
                lock(&context).created = true;
                Ok(0)
            }
            FSCONFIG_CMD_RECONFIGURE => {
                let (pick, flags) = {
                    let c = lock(&context);
                    (c.pick.clone(), c.flags)
                };
                let path = pick.ok_or(EBUSY)?;
                let ns = lock(&self.process.state).ns.mnt.clone();
                ns.change(|mounts| {
                    let i = mounts.at(&path).ok_or(EINVAL)?;
                    mounts.list[i].flags = mount_flags(flags);
                    Ok(0)
                })
            }
            _ => Err(EINVAL),
        }
    }

    pub(crate) fn fsmount(&mut self, fd: i32, flags: u64, attr: u64) -> Result<u64, Errno> {
        if flags & !FSMOUNT_CLOEXEC != 0 || attr & !0xff != 0 {
            return Err(EINVAL);
        }
        let context = self.fscontext(fd)?;
        let (what, source, options, created) = {
            let c = lock(&context);
            (c.what, c.source.clone(), c.options.clone(), c.created)
        };
        if !created {
            return Err(EINVAL);
        }
        let (root, tree, root_path) = if what == What::Tmpfs {
            let mode = options.split(',').find_map(|o| o.strip_prefix("mode=")).and_then(|m| u32::from_str_radix(m, 8).ok()).unwrap_or(0o1777);
            let tree = lock(&self.k.disk).volatile_root(Meta { mode: mode & 0o7777, ..Meta::dir(0) }, self.k.now());
            (tree, Some(tree), "/".to_string())
        } else {
            (layers::ROOT, None, "/".to_string())
        };
        let fstype = match what {
            What::Tmpfs => "tmpfs",
            What::Proc => "proc",
            What::Devpts => "devpts",
            What::Mqueue => "mqueue",
            What::Disk => "ext4",
        };
        let (set_ms, _) = mount_attr(attr, 0)?;
        let detached = Detached { what, root, root_path, source, fstype: fstype.to_string(), flags: mount_flags(set_ms), options, tree };
        let open = Open::new(Node::MountFd(Arc::new(Mutex::new(Some(detached)))), O_RDWR, "anon_inode:[fsmount]");
        lock(&self.process.state).files.add(open, flags & FSMOUNT_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    /// `open_tree`: with `OPEN_TREE_CLONE`, a detached copy of the mount at the path; without, an O_PATH handle on it.
    pub(crate) fn open_tree(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<u64, Errno> {
        let clone = flags & OPEN_TREE_CLONE != 0;
        let (ino, path) = self.at_mount_path(cx, dirfd, path_at, flags)?;
        if !clone {
            // An O_PATH handle on the directory.
            let open = Open::new(if lock(&self.k.disk).stat(ino)?.kind == Kind::Dir { Node::Dir(ino) } else { Node::File(ino) }, O_PATH, path);
            return lock(&self.process.state).files.add(open, flags & OPEN_TREE_CLOEXEC != 0, 0).map(|fd| fd as u64);
        }
        self.may_mount()?;
        let m = {
            let mounts = self.view().mounts;
            mounts.containing(&path).clone()
        };
        let root_path = rebase(&path, &m.path, &m.root_path);
        let detached = Detached { what: m.what, root: ino, root_path, source: m.source, fstype: m.fstype, flags: m.flags, options: m.options, tree: m.tree };
        let open = Open::new(Node::MountFd(Arc::new(Mutex::new(Some(detached)))), O_RDWR, "anon_inode:[open_tree]");
        lock(&self.process.state).files.add(open, flags & OPEN_TREE_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    pub(crate) fn move_mount(&mut self, cx: &mut Cx<'_>, from_dfd: i32, from_at: u64, to_dfd: i32, to_at: u64, flags: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        if flags & !MOVE_MOUNT_MASK != 0 {
            return Err(EINVAL);
        }
        let (_, to_path) = self.at_mount_path(cx, to_dfd, to_at, if flags & MOVE_MOUNT_T_EMPTY_PATH != 0 { AT_EMPTY_PATH_ } else { 0 })?;
        // From a detached mount (fsmount / open_tree): attach it here.
        if flags & MOVE_MOUNT_F_EMPTY_PATH != 0 {
            let detached = match &self.file(from_dfd)?.node {
                Node::MountFd(m) => lock(m).take().ok_or(EINVAL)?,
                _ => return Err(EINVAL),
            };
            let (point, kind) = {
                let view = self.view();
                let disk = lock(&self.k.disk);
                let ino = view.resolve(&disk, view.root, &self.ns_to_visible(&to_path), true)?;
                (ino, disk.stat(ino)?.kind)
            };
            if kind != Kind::Dir {
                return Err(ENOTDIR);
            }
            let ns = lock(&self.process.state).ns.mnt.clone();
            return ns.change(|mounts| {
                let parent = mounts.containing(&to_path).id;
                mounts.add(Mount { id: 0, parent, point, path: to_path.clone(), root: detached.root, root_path: detached.root_path.clone(), what: detached.what, source: detached.source.clone(), fstype: detached.fstype.clone(), flags: detached.flags, propagation: MS_PRIVATE, options: detached.options.clone(), tree: detached.tree });
                Ok(0)
            });
        }
        // Moving an existing mount, like `mount --move`.
        let (_, from_path) = self.at_mount_path(cx, from_dfd, from_at, 0)?;
        let ns = lock(&self.process.state).ns.mnt.clone();
        let point = {
            let view = self.view();
            view.resolve(&lock(&self.k.disk), view.root, &self.ns_to_visible(&to_path), true)?
        };
        ns.change(|mounts| {
            let i = mounts.at(&from_path).ok_or(EINVAL)?;
            if i == 0 || mounts.at(&to_path).is_some() {
                return Err(EINVAL);
            }
            let parent = mounts.containing(&to_path).id;
            mounts.list[i].point = point;
            mounts.list[i].parent = parent;
            let (from, to) = (from_path.clone(), to_path.clone());
            for m in &mut mounts.list {
                if under(&m.path, &from) {
                    m.path = rebase(&m.path, &from, &to);
                }
            }
            Ok(0)
        })
    }

    pub(crate) fn mount_setattr(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64, attr_at: u64, size: u64) -> Result<u64, Errno> {
        self.may_mount()?;
        if size != 32 {
            return Err(EINVAL);
        }
        let b = self.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), attr_at, 32))?;
        let word = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or_default());
        let (set, clr, prop) = (word(0), word(1), word(2));
        let (set_ms, clr_ms) = mount_attr(set, clr)?;
        let (_, path) = self.at_mount_path(cx, dirfd, path_at, flags)?;
        let recursive = flags & AT_RECURSIVE != 0;
        let ns = lock(&self.process.state).ns.mnt.clone();
        ns.change(|mounts| {
            let i = mounts.at(&path).ok_or(EINVAL)?;
            let top = mounts.list[i].path.clone();
            let mut touched = false;
            for m in &mut mounts.list {
                if m.path == top || (recursive && under(&m.path, &top)) {
                    m.flags = m.flags & !clr_ms | set_ms;
                    if prop != 0 && prop.is_power_of_two() && prop & PROPAGATION != 0 {
                        m.propagation = prop;
                    }
                    touched = true;
                }
            }
            if touched { Ok(0) } else { Err(EINVAL) }
        })
    }

    /// A path from `dirfd` (`AT_FDCWD`, an O_PATH directory, or empty with a mount fd's/dir fd's own path), as it is in
    /// the namespace.
    fn at_mount_path(&mut self, cx: &mut Cx<'_>, dirfd: i32, path_at: u64, flags: u64) -> Result<(Ino, String), Errno> {
        let path = self.read_path_or_empty(cx, path_at)?;
        if path.is_empty() && flags & (AT_EMPTY_PATH_ | MOVE_MOUNT_T_EMPTY_PATH | MOVE_MOUNT_F_EMPTY_PATH) != 0 {
            let open = self.file(dirfd)?;
            let visible = open.path.clone();
            let view = self.view();
            let ino = view.resolve(&lock(&self.k.disk), view.root, &visible, true)?;
            return Ok((ino, self.ns_path(&visible)));
        }
        let (dir, dir_path) = if dirfd == AT_FDCWD {
            self.start_dir(dirfd)?
        } else {
            let open = self.file(dirfd)?;
            match open.node {
                Node::Dir(ino) => (ino, open.path.clone()),
                _ => return Err(ENOTDIR),
            }
        };
        let view = self.view();
        let ino = view.resolve(&lock(&self.k.disk), dir, &path, true)?;
        Ok((ino, self.ns_path(&exe_path(&dir_path, &path))))
    }

    fn read_path_or_empty(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<String, Errno> {
        if at == 0 {
            return Ok(String::new());
        }
        self.read_path(cx, at)
    }

    /// A namespace path back to a path this process can resolve from its root (the inverse of `ns_path`).
    fn ns_to_visible(&self, ns_path: &str) -> String {
        let root = lock(&self.process.state).root.1.clone();
        if root == "/" { ns_path.to_string() } else { rebase(ns_path, &root, "/") }
    }
}

/// Where Linux's unique mount IDs start; `mountinfo`'s old ones are below it.
const MNT_UNIQUE_ID_OFFSET: u64 = 1 << 31;
/// `listmount`'s "the caller's root".
const LSMT_ROOT: u64 = u64::MAX;
const STATMOUNT_SB_BASIC: u64 = 0x1;
const STATMOUNT_MNT_BASIC: u64 = 0x2;
const STATMOUNT_PROPAGATE_FROM: u64 = 0x4;
const STATMOUNT_MNT_ROOT: u64 = 0x8;
const STATMOUNT_MNT_POINT: u64 = 0x10;
const STATMOUNT_FS_TYPE: u64 = 0x20;
const STATMOUNT_MNT_NS_ID: u64 = 0x40;
const STATMOUNT_MNT_OPTS: u64 = 0x80;
const STATMOUNT_SB_SOURCE: u64 = 0x200;
const STATMOUNT_OPT_ARRAY: u64 = 0x400;
const STATMOUNT_OPT_SEC_ARRAY: u64 = 0x800;
const STATMOUNT_SUPPORTED_MASK: u64 = 0x1000;
/// What this `statmount` can say (no subtypes, no ID maps).
const STATMOUNT_SUPPORTED: u64 = STATMOUNT_SB_BASIC
    | STATMOUNT_MNT_BASIC
    | STATMOUNT_PROPAGATE_FROM
    | STATMOUNT_MNT_ROOT
    | STATMOUNT_MNT_POINT
    | STATMOUNT_FS_TYPE
    | STATMOUNT_MNT_NS_ID
    | STATMOUNT_MNT_OPTS
    | STATMOUNT_SB_SOURCE
    | STATMOUNT_OPT_ARRAY
    | STATMOUNT_OPT_SEC_ARRAY
    | STATMOUNT_SUPPORTED_MASK;
/// `struct statmount` without its strings.
const STATMOUNT_HEADER: usize = 512;

impl Task {
    /// `struct mnt_id_req`: the mount (or `LSMT_ROOT`), `param`, and the namespace, which must be this process's.
    fn mnt_id_req(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<(u64, u64), Errno> {
        let b = self.mem(cx, false, |t| kernels_core::user::read_vec(&t.space(), at, 32))?;
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or_default());
        let size = u32_at(0);
        if size < 24 || u32_at(4) != 0 {
            return Err(EINVAL);
        }
        let ns = lock(&self.process.state).ns.mnt.id();
        if size >= 32 && u64_at(24) != 0 && u64_at(24) != ns {
            return Err(ENOENT);
        }
        Ok((u64_at(8), u64_at(16)))
    }

    /// The mount with this ID, old or unique, that this process can see from its root, with its path from there.
    fn mount_by_id(&self, id: u64) -> Result<(Mount, String), Errno> {
        let root = lock(&self.process.state).root.1.clone();
        let mounts = self.view().mounts;
        mounts.visible(&root).find(|(m, _)| m.unique_id() == id || u64::from(m.id) == id).map(|(m, path)| (m.clone(), path)).ok_or(ENOENT)
    }

    /// `statmount`: what `param` asks of a mount, in a `struct statmount` and the strings after it.
    pub(crate) fn statmount(&mut self, cx: &mut Cx<'_>, req_at: u64, buf: u64, size: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let (id, mask) = self.mnt_id_req(cx, req_at)?;
        if (size as usize) < STATMOUNT_HEADER {
            return Err(EINVAL);
        }
        let (m, point) = self.mount_by_id(id)?;
        let mounts = self.view().mounts;
        let parent = mounts.list.iter().find(|p| p.id == m.parent && p.id != m.id).map_or(&m, |p| p);
        let ns = lock(&self.process.state).ns.mnt.id();
        let mask = mask & STATMOUNT_SUPPORTED;
        let mut h = vec![0u8; STATMOUNT_HEADER];
        let mut strings: Vec<u8> = Vec::new();
        let put32 = |h: &mut Vec<u8>, at: usize, v: u32| h[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let put64 = |h: &mut Vec<u8>, at: usize, v: u64| h[at..at + 8].copy_from_slice(&v.to_le_bytes());
        // A string's offset in what follows the header, and the string with its NUL.
        let mut string = |h: &mut Vec<u8>, at: usize, text: &[u8]| {
            h[at..at + 4].copy_from_slice(&(strings.len() as u32).to_le_bytes());
            strings.extend(text);
            strings.push(0);
        };
        if mask & STATMOUNT_SB_BASIC != 0 {
            let (major, minor) = m.dev();
            put32(&mut h, 16, major);
            put32(&mut h, 20, minor);
            put64(&mut h, 24, m.what.magic());
            put32(&mut h, 32, (m.flags & MS_RDONLY) as u32);
        }
        if mask & STATMOUNT_MNT_BASIC != 0 {
            put64(&mut h, 40, m.unique_id());
            put64(&mut h, 48, parent.unique_id());
            put32(&mut h, 56, m.id);
            put32(&mut h, 60, parent.id);
            put64(&mut h, 64, m.attr());
            put64(&mut h, 72, m.propagation);
            put64(&mut h, 80, if m.propagation == MS_SHARED { u64::from(m.id) } else { 0 });
            put64(&mut h, 88, if m.propagation == MS_SLAVE { 1 } else { 0 });
        }
        if mask & STATMOUNT_FS_TYPE != 0 {
            string(&mut h, 36, m.fstype.as_bytes());
        }
        if mask & STATMOUNT_MNT_ROOT != 0 {
            string(&mut h, 104, m.root_path.as_bytes());
        }
        if mask & STATMOUNT_MNT_POINT != 0 {
            string(&mut h, 108, point.as_bytes());
        }
        if mask & STATMOUNT_MNT_NS_ID != 0 {
            put64(&mut h, 112, ns);
        }
        if mask & STATMOUNT_SB_SOURCE != 0 {
            string(&mut h, 124, m.source.as_bytes());
        }
        if mask & STATMOUNT_MNT_OPTS != 0 {
            string(&mut h, 4, m.options.as_bytes());
        }
        if mask & STATMOUNT_OPT_ARRAY != 0 {
            let options: Vec<&str> = m.options.split(',').filter(|o| !o.is_empty()).collect();
            put32(&mut h, 128, options.len() as u32);
            string(&mut h, 132, options.join("\0").as_bytes());
        }
        if mask & STATMOUNT_OPT_SEC_ARRAY != 0 {
            put32(&mut h, 136, 0);
            string(&mut h, 140, b"");
        }
        if mask & STATMOUNT_SUPPORTED_MASK != 0 {
            put64(&mut h, 144, STATMOUNT_SUPPORTED);
        }
        put64(&mut h, 8, mask);
        let total = STATMOUNT_HEADER + strings.len();
        if total > size as usize {
            return Err(EOVERFLOW);
        }
        put32(&mut h, 0, total as u32);
        h.extend(strings);
        self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), buf, &h))?;
        Ok(0)
    }

    /// `listmount`: the IDs of the mounts at and under a mount (or this process's root), in order of ID, after
    /// `param` (or with `LISTMOUNT_REVERSE`, before it), as many as fit.
    pub(crate) fn listmount(&mut self, cx: &mut Cx<'_>, req_at: u64, ids_at: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        const LISTMOUNT_REVERSE: u64 = 1;
        if flags & !LISTMOUNT_REVERSE != 0 {
            return Err(EINVAL);
        }
        let (id, last) = self.mnt_id_req(cx, req_at)?;
        let root = lock(&self.process.state).root.1.clone();
        let top = if id == LSMT_ROOT { root.clone() } else { self.mount_by_id(id)?.0.path };
        let mounts = self.view().mounts;
        let mut ids: Vec<u64> = mounts.visible(&root).filter(|(m, _)| under(&m.path, &top)).map(|(m, _)| m.unique_id()).collect();
        ids.sort_unstable();
        if flags & LISTMOUNT_REVERSE != 0 {
            ids.reverse();
            ids.retain(|&i| last == 0 || i < last);
        } else {
            ids.retain(|&i| i > last);
        }
        ids.truncate(count.min(1 << 20) as usize);
        let bytes: Vec<u8> = ids.iter().flat_map(|i| i.to_le_bytes()).collect();
        self.mem(cx, true, |t| kernels_core::user::write(&mut t.space(), ids_at, &bytes))?;
        Ok(ids.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_under_and_rebased() {
        assert!(under("/a/b", "/a") && under("/a", "/a") && under("/x", "/") && !under("/ab", "/a"));
        assert_eq!(rebase("/new/proc", "/new", "/"), "/proc");
        assert_eq!(rebase("/new", "/new", "/"), "/");
        assert_eq!(rebase("/proc", "/", "/old"), "/old/proc");
        assert_eq!(rebase("/", "/", "/old"), "/old");
        assert_eq!(escape("a b\\c"), "a\\040b\\134c");
        assert_eq!((size_of("8m", 0), size_of("512k", 0), size_of("50%", 1000), size_of("x", 0)), (Some(8 << 20), Some(512 << 10), Some(500), None));
        assert_eq!(tmpfs_options("size=1m,mode=755,noexec", "mode=1777"), "size=1m,mode=755");
    }

    #[test]
    fn a_walk_crosses_mounts_and_stays_in_its_root() {
        let mut disk = Disk::new(Meta::dir(0o755), false);
        let a = disk.create(ROOT, "a", layers::New::Dir, Meta::dir(0o755), 0).unwrap();
        let b = disk.create(ROOT, "b", layers::New::Dir, Meta::dir(0o755), 0).unwrap();
        let inside = disk.create(b, "inside", layers::New::File, Meta::file(0o644), 0).unwrap();
        disk.create(a, "up", layers::New::Symlink("/b/inside".into()), Meta::file(0o777), 0).unwrap();
        let mut mounts = Mounts::boot(&disk);
        // b bound onto a: a's own "up" is hidden; ".." from a's mount goes to the root.
        mounts.add(Mount { id: 0, parent: 1, point: a, path: "/a".into(), root: b, root_path: "/b".into(), ..mounts.list[0].clone() });
        let view = View { root: ROOT, mounts: Arc::new(mounts) };
        assert_eq!(view.resolve(&disk, ROOT, "/a/inside", true), Ok(inside));
        assert_eq!(view.resolve(&disk, ROOT, "/a/up", true), Err(Error::NotFound));
        assert_eq!(view.resolve(&disk, ROOT, "/a/../b/inside", true), Ok(inside));
        // Chrooted to b: nothing above it.
        let jail = View { root: b, ..view.clone() };
        assert_eq!(jail.resolve(&disk, b, "/../../inside", true), Ok(inside));
        assert_eq!(jail.resolve(&disk, b, "/b/inside", true), Err(Error::NotFound));
        assert_eq!(jail.resolve_parent(&disk, b, "/new"), Ok((b, "new")));
        assert_eq!(view.mounts.containing("/a/x").path, "/a");
        assert_eq!(view.mounts.containing("/ab").path, "/");
    }
}
