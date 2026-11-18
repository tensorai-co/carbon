//! Files' permissions, checked as Linux checks them (`generic_permission`): the owner's bits if the process's
//! filesystem user ID owns the file, else the group's if it's in the file's group, else everyone else's; then
//! `CAP_DAC_OVERRIDE` (reading and writing anything, and running anything with an execute bit, or searching any
//! folder) and `CAP_DAC_READ_SEARCH` (reading anything, searching any folder). A folder's sticky bit lets only a file's
//! owner, the folder's, or `CAP_FOWNER` remove or rename what's in it. A new file is its maker's, and in a folder with
//! the setgid bit takes the folder's group (and a new folder the bit too).
//!
//! A process with both DAC capabilities (root, as every machine's first program is unless it's given a user) is never
//! checked but for running a file with no execute bit at all, so a machine of root's pays nothing for this.

use layers::{Ino, Kind};

use crate::abi::*;
use crate::process::Task;
use crate::process::creds::Creds;
use crate::{Disk, Meta};

pub(crate) const MAY_READ: u32 = 4;
pub(crate) const MAY_WRITE: u32 = 2;
pub(crate) const MAY_EXEC: u32 = 1;

pub(crate) const S_ISUID: u32 = 0o4000;
pub(crate) const S_ISGID: u32 = 0o2000;
pub(crate) const S_ISVTX: u32 = 0o1000;

pub(crate) const CAP_CHOWN: u32 = 0;
const CAP_DAC_OVERRIDE: u32 = 1;
const CAP_DAC_READ_SEARCH: u32 = 2;
pub(crate) const CAP_FOWNER: u32 = 3;
pub(crate) const CAP_FSETID: u32 = 4;

/// Who a process is to its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Who {
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
    caps: u64,
}

impl Who {
    /// As the process's files see it: its filesystem IDs and effective capabilities.
    pub fn of(c: &Creds) -> Who {
        Who { uid: c.uid[3], gid: c.gid[3], groups: c.groups.clone(), caps: c.caps.effective }
    }

    /// As `access` asks: its real IDs, and capabilities only if its real user is root (as Linux's `access_override_creds`).
    pub fn real(c: &Creds) -> Who {
        Who { uid: c.uid[0], gid: c.gid[0], groups: c.groups.clone(), caps: if c.uid[0] == 0 { c.caps.permitted } else { 0 } }
    }

    pub fn can(&self, cap: u32) -> bool {
        self.caps & 1 << cap != 0
    }

    /// Whether nothing it does to a file needs checking (but for running one with no execute bit).
    pub fn unchecked(&self) -> bool {
        self.can(CAP_DAC_OVERRIDE) && self.can(CAP_DAC_READ_SEARCH)
    }

    pub fn in_group(&self, gid: u32) -> bool {
        self.gid == gid || self.groups.contains(&gid)
    }

    /// Whether it may do `want` (`MAY_READ`, `MAY_WRITE`, `MAY_EXEC`, together) to a file with `meta`.
    pub fn may(&self, meta: &Meta, dir: bool, want: u32) -> bool {
        let mode = meta.mode;
        let bits = if self.uid == meta.uid {
            mode >> 6 & 7
        } else if self.in_group(meta.gid) {
            mode >> 3 & 7
        } else {
            mode & 7
        };
        if bits & want == want {
            return true;
        }
        // Executing needs an execute bit somewhere, even for root; searching a folder doesn't.
        if self.can(CAP_DAC_OVERRIDE) && (want & MAY_EXEC == 0 || dir || mode & 0o111 != 0) {
            return true;
        }
        self.can(CAP_DAC_READ_SEARCH) && want & MAY_WRITE == 0 && (want & MAY_EXEC == 0 || dir)
    }

    /// Whether it may remove (or rename away) a file with `file` from a folder with `dir`: the sticky bit's rule.
    pub fn may_delete(&self, dir: &Meta, file: &Meta) -> bool {
        dir.mode & S_ISVTX == 0 || self.can(CAP_FOWNER) || self.uid == dir.uid || self.uid == file.uid
    }

    /// Whether it owns a file, or may act as if it did (`CAP_FOWNER`).
    pub fn owns(&self, meta: &Meta) -> bool {
        self.uid == meta.uid || self.can(CAP_FOWNER)
    }
}

/// Whether `who` may do `want` to the file `ino`; EACCES if not.
pub(crate) fn check(disk: &Disk, who: &Who, ino: Ino, want: u32) -> Result<(), Errno> {
    let dir = disk.stat(ino)?.kind == Kind::Dir;
    if who.may(&disk.meta(ino)?, dir, want) { Ok(()) } else { Err(EACCES) }
}

impl Task {
    /// Who this process is to its files. Taken before the disk is, as the process's state is locked first.
    pub(crate) fn who(&self) -> Who {
        Who::of(&kernels_core::lock(&self.process.state).creds)
    }
}

/// May `who` make or remove a name in the folder `dir`: write and search there.
pub(crate) fn may_change(disk: &Disk, who: &Who, dir: Ino) -> Result<(), Errno> {
    if who.unchecked() { Ok(()) } else { check(disk, who, dir, MAY_WRITE | MAY_EXEC) }
}

/// May `who` remove (or rename away) the file `ino` from `dir`: write and search there, and the sticky bit's rule.
pub(crate) fn may_remove(disk: &Disk, who: &Who, dir: Ino, ino: Ino) -> Result<(), Errno> {
    if who.unchecked() {
        return Ok(());
    }
    check(disk, who, dir, MAY_WRITE | MAY_EXEC)?;
    if !who.may_delete(&disk.meta(dir)?, &disk.meta(ino)?) {
        return Err(EPERM);
    }
    Ok(())
}

/// A new file's `Meta` in `parent`: `mode` (its type and permissions), `who`'s filesystem user, and its group, or a
/// setgid folder's (a new folder there keeps the bit).
pub(crate) fn new_meta(disk: &Disk, who: &Who, parent: Ino, mode: u32, dir: bool) -> Meta {
    let parent_meta = disk.meta(parent).unwrap_or_default();
    let setgid = parent_meta.mode & S_ISGID != 0;
    let gid = if setgid { parent_meta.gid } else { who.gid };
    let mode = if setgid && dir { mode | S_ISGID } else { mode };
    Meta { mode, uid: who.uid, gid, ..Meta::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::creds::Caps;

    fn user(uid: u32, gid: u32, groups: &[u32]) -> Who {
        Who::of(&Creds { uid: [uid; 4], gid: [gid; 4], groups: groups.to_vec(), caps: Caps { effective: 0, permitted: 0, inheritable: 0, bounding: 0, ambient: 0 }, securebits: 0 })
    }

    fn file(mode: u32, uid: u32, gid: u32) -> Meta {
        Meta { mode, uid, gid, ..Meta::default() }
    }

    #[test]
    fn owner_group_other_as_linux_checks() {
        let app = user(1000, 1000, &[50]);
        assert!(app.may(&file(0o600, 1000, 0), false, MAY_READ | MAY_WRITE));
        assert!(!app.may(&file(0o600, 0, 0), false, MAY_READ));
        assert!(app.may(&file(0o640, 0, 50), false, MAY_READ), "a supplementary group's bits");
        assert!(!app.may(&file(0o640, 0, 50), false, MAY_WRITE));
        assert!(app.may(&file(0o755, 0, 0), false, MAY_EXEC));
        assert!(!app.may(&file(0o750, 0, 0), true, MAY_EXEC), "a folder it can't search");
        // The owner's bits decide for the owner, even when the others' would allow it.
        assert!(!app.may(&file(0o077, 1000, 0), false, MAY_READ));
    }

    #[test]
    fn root_reads_and_writes_anything_but_runs_only_what_can_be_run() {
        let root = Who::of(&Creds::default());
        assert!(root.unchecked());
        assert!(root.may(&file(0o000, 1000, 1000), false, MAY_READ | MAY_WRITE));
        assert!(root.may(&file(0o000, 1000, 1000), true, MAY_EXEC), "any folder searched");
        assert!(!root.may(&file(0o644, 0, 0), false, MAY_EXEC), "no execute bit, no exec");
        assert!(root.may(&file(0o744, 1000, 0), false, MAY_EXEC));
    }

    #[test]
    fn sticky_folders() {
        let (app, other) = (user(1000, 1000, &[]), user(1001, 1001, &[]));
        let tmp = file(0o1777, 0, 0);
        assert!(app.may_delete(&tmp, &file(0o644, 1000, 1000)));
        assert!(!other.may_delete(&tmp, &file(0o644, 1000, 1000)));
        assert!(other.may_delete(&file(0o777, 0, 0), &file(0o644, 1000, 1000)));
    }
}
