//! A process's credentials: its real, effective, saved and filesystem user and group IDs, and its supplementary
//! groups, with Linux's rules for changing them. Effective root may set them to anything; anyone else may only move
//! among the IDs it already has, so a program that drops root (as `apt` does for its downloads, to `_apt`) can't get
//! it back, and can check that it can't. Files' permissions aren't checked against them yet.
//!
//! And its capabilities, as Linux keeps them: effective, permitted, inheritable, bounding and ambient sets, and the
//! securebits. What's privileged asks the effective set (`CAP_SETUID` to change user IDs freely, and so on). Changing
//! user IDs changes them as capabilities(7) says: leaving root behind clears the permitted and effective sets (unless
//! `SECBIT_KEEP_CAPS`), an effective ID leaving 0 clears the effective set, coming back to 0 restores it, and the
//! filesystem ID does the same for the file capabilities. `execve` works out the new sets as for a file with no
//! capabilities of its own: root's from its inheritable and bounding sets, anyone else's from its ambient set.

use crate::abi::*;

/// "Leave it as it is", for the `setres*id` and `setre*id` calls.
const KEEP: u32 = u32::MAX;

/// The highest capability Linux 7.4 has (`CAP_CHECKPOINT_RESTORE`), and all of them.
pub(crate) const CAP_LAST_CAP: u32 = 40;
pub(crate) const ALL_CAPS: u64 = (1 << (CAP_LAST_CAP + 1)) - 1;
pub(crate) const CAP_SETGID: u32 = 6;
pub(crate) const CAP_SETUID: u32 = 7;
pub(crate) const CAP_SETPCAP: u32 = 8;
pub(crate) const CAP_MKNOD: u32 = 27;
pub(crate) const CAP_SYS_CHROOT: u32 = 18;
pub(crate) const CAP_SYS_ADMIN: u32 = 21;
pub(crate) const CAP_SYS_BOOT: u32 = 22;
pub(crate) const CAP_SYS_TTY_CONFIG: u32 = 26;
pub(crate) const CAP_SYSLOG: u32 = 34;
/// What the filesystem user ID going to and from 0 takes and gives back: `CAP_CHOWN`, `CAP_DAC_OVERRIDE`,
/// `CAP_DAC_READ_SEARCH`, `CAP_FOWNER`, `CAP_FSETID`, `CAP_LINUX_IMMUTABLE`, `CAP_MKNOD`, `CAP_MAC_OVERRIDE`.
const FS_CAPS: u64 = 0b1_1111 | 1 << 9 | 1 << 27 | 1 << 32;
/// Securebits.
pub(crate) const SECBIT_NOROOT: u32 = 1 << 0;
pub(crate) const SECBIT_NO_SETUID_FIXUP: u32 = 1 << 2;
pub(crate) const SECBIT_KEEP_CAPS: u32 = 1 << 4;
pub(crate) const SECBIT_NO_CAP_AMBIENT_RAISE: u32 = 1 << 6;

/// A process's capability sets, each a bit for each capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Caps {
    pub effective: u64,
    pub permitted: u64,
    pub inheritable: u64,
    pub bounding: u64,
    pub ambient: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Creds {
    /// Real, effective, saved and filesystem user IDs.
    pub uid: [u32; 4],
    pub gid: [u32; 4],
    pub groups: Vec<u32>,
    pub caps: Caps,
    pub securebits: u32,
}

/// The first process's: root, with every capability.
impl Default for Creds {
    fn default() -> Creds {
        let caps = Caps { effective: ALL_CAPS, permitted: ALL_CAPS, inheritable: 0, bounding: ALL_CAPS, ambient: 0 };
        Creds { uid: [0; 4], gid: [0; 4], groups: Vec::new(), caps, securebits: 0 }
    }
}

/// One kind of ID's four: real, effective, saved, filesystem.
fn set_res(ids: &mut [u32; 4], privileged: bool, r: u32, e: u32, s: u32) -> Result<(), Errno> {
    let allowed = |v: u32| v == KEEP || privileged || ids[..3].contains(&v);
    if !(allowed(r) && allowed(e) && allowed(s)) {
        return Err(EPERM);
    }
    for (slot, v) in [(0, r), (1, e), (2, s)] {
        if v != KEEP {
            ids[slot] = v;
        }
    }
    ids[3] = ids[1];
    Ok(())
}

/// `setuid` and `setgid`: everything for root, else only the effective ID (to the real or saved one).
fn set_one(ids: &mut [u32; 4], privileged: bool, v: u32) -> Result<(), Errno> {
    if privileged {
        *ids = [v; 4];
    } else if v == ids[0] || v == ids[2] {
        ids[1] = v;
        ids[3] = v;
    } else {
        return Err(EPERM);
    }
    Ok(())
}

/// `setreuid` and `setregid`: the real ID to the real or effective one, the effective to any of the three; the saved
/// ID follows the new effective one when the real one's set or the effective one leaves the real.
fn set_re(ids: &mut [u32; 4], privileged: bool, r: u32, e: u32) -> Result<(), Errno> {
    let old_real = ids[0];
    if !privileged && ((r != KEEP && r != ids[0] && r != ids[1]) || (e != KEEP && !ids[..3].contains(&e))) {
        return Err(EPERM);
    }
    if r != KEEP {
        ids[0] = r;
    }
    if e != KEEP {
        ids[1] = e;
    }
    if r != KEEP || (e != KEEP && e != old_real) {
        ids[2] = ids[1];
    }
    ids[3] = ids[1];
    Ok(())
}

impl Creds {
    /// Whether its effective set has `cap`.
    pub fn capable(&self, cap: u32) -> bool {
        self.caps.effective & 1 << cap != 0
    }

    /// The user IDs were `old`: what that does to the capabilities, as Linux's `cap_emulate_setxuid`.
    fn uid_changed(&mut self, old: [u32; 4]) {
        if self.securebits & SECBIT_NO_SETUID_FIXUP != 0 {
            return;
        }
        let (new, caps) = (self.uid, &mut self.caps);
        if old[..3].contains(&0) && !new[..3].contains(&0) {
            if self.securebits & SECBIT_KEEP_CAPS == 0 {
                caps.permitted = 0;
                caps.effective = 0;
            }
            caps.ambient = 0;
        }
        if old[1] == 0 && new[1] != 0 {
            caps.effective = 0;
        }
        if old[1] != 0 && new[1] == 0 {
            caps.effective = caps.permitted;
        }
    }

    /// The user IDs, changed by `change` (given whether it may set any), and the capabilities after.
    fn change_uid(&mut self, change: impl FnOnce(&mut [u32; 4], bool) -> Result<(), Errno>) -> Result<(), Errno> {
        let (old, may) = (self.uid, self.capable(CAP_SETUID));
        change(&mut self.uid, may)?;
        self.uid_changed(old);
        Ok(())
    }

    pub fn setuid(&mut self, v: u32) -> Result<(), Errno> {
        self.change_uid(|ids, may| set_one(ids, may, v))
    }

    pub fn setgid(&mut self, v: u32) -> Result<(), Errno> {
        let may = self.capable(CAP_SETGID);
        set_one(&mut self.gid, may, v)
    }

    pub fn setreuid(&mut self, r: u32, e: u32) -> Result<(), Errno> {
        self.change_uid(|ids, may| set_re(ids, may, r, e))
    }

    pub fn setregid(&mut self, r: u32, e: u32) -> Result<(), Errno> {
        let may = self.capable(CAP_SETGID);
        set_re(&mut self.gid, may, r, e)
    }

    pub fn setresuid(&mut self, r: u32, e: u32, s: u32) -> Result<(), Errno> {
        self.change_uid(|ids, may| set_res(ids, may, r, e, s))
    }

    pub fn setresgid(&mut self, r: u32, e: u32, s: u32) -> Result<(), Errno> {
        let may = self.capable(CAP_SETGID);
        set_res(&mut self.gid, may, r, e, s)
    }

    pub fn setgroups(&mut self, groups: Vec<u32>) -> Result<(), Errno> {
        if !self.capable(CAP_SETGID) {
            return Err(EPERM);
        }
        self.groups = groups;
        Ok(())
    }

    /// `setfsuid` and `setfsgid`: the old filesystem ID, always; the new one only if it's one of the others (or with
    /// `CAP_SETUID` or `CAP_SETGID`). The filesystem user ID leaving 0 takes the file capabilities from the effective
    /// set, and coming back gives them back from the permitted one.
    pub fn setfs(&mut self, group: bool, v: u32) -> u32 {
        let may = self.capable(if group { CAP_SETGID } else { CAP_SETUID });
        let ids = if group { &mut self.gid } else { &mut self.uid };
        let old = ids[3];
        if v != KEEP && (may || ids[..4].contains(&v)) {
            ids[3] = v;
        }
        if !group && self.securebits & SECBIT_NO_SETUID_FIXUP == 0 {
            let caps = &mut self.caps;
            if old == 0 && self.uid[3] != 0 {
                caps.effective &= !FS_CAPS;
            } else if old != 0 && self.uid[3] == 0 {
                caps.effective |= caps.permitted & FS_CAPS;
            }
        }
        old
    }

    /// `execve` of a file with no capabilities of its own (there are no file capabilities here, nor setuid bits).
    pub fn exec(&mut self) {
        let root = self.securebits & SECBIT_NOROOT == 0 && (self.uid[0] == 0 || self.uid[1] == 0);
        let caps = &mut self.caps;
        if root {
            // As if the file had every capability, inheritable and permitted, and was effective for effective root.
            caps.permitted = caps.inheritable | caps.bounding;
            caps.ambient = 0;
            caps.effective = if self.uid[1] == 0 { caps.permitted } else { 0 };
        } else {
            caps.permitted = caps.ambient;
            caps.effective = caps.ambient;
        }
        self.securebits &= !SECBIT_KEEP_CAPS;
    }

    /// `capset`: new effective, permitted and inheritable sets, if Linux's rules allow them.
    pub fn capset(&mut self, effective: u64, permitted: u64, inheritable: u64) -> Result<(), Errno> {
        let old = self.caps;
        let capped = !self.capable(CAP_SETPCAP);
        if (capped && inheritable & !(old.inheritable | old.permitted) != 0)
            || inheritable & !(old.inheritable | old.bounding) != 0
            || permitted & !old.permitted != 0
            || effective & !permitted != 0
        {
            return Err(EPERM);
        }
        self.caps = Caps { effective, permitted, inheritable, ambient: old.ambient & permitted & inheritable, ..old };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `apt` does to drop root for its downloads, and checks afterwards.
    #[test]
    fn dropping_root_sticks() {
        let mut c = Creds::default();
        c.setgroups(vec![65534]).unwrap();
        c.setresgid(65534, 65534, 65534).unwrap();
        c.setresuid(42, 42, 42).unwrap();
        assert_eq!((c.uid, c.gid), ([42; 4], [65534; 4]));
        assert_eq!(c.setuid(0), Err(EPERM));
        assert_eq!(c.setresuid(KEEP, 0, KEEP), Err(EPERM));
        assert_eq!(c.setgid(0), Err(EPERM));
        assert_eq!(c.setgroups(vec![0]), Err(EPERM));
    }

    #[test]
    fn capabilities_follow_the_user_ids() {
        let mut c = Creds::default();
        // A temporary drop takes the effective set; coming back restores it from the permitted one.
        c.setresuid(KEEP, 1000, KEEP).unwrap();
        assert_eq!((c.caps.effective, c.caps.permitted), (0, ALL_CAPS));
        c.setresuid(KEEP, 0, KEEP).unwrap();
        assert_eq!(c.caps.effective, ALL_CAPS);
        // SECBIT_KEEP_CAPS keeps the permitted set through leaving root, but not the effective one.
        let mut kept = c.clone();
        kept.securebits |= SECBIT_KEEP_CAPS;
        kept.setresuid(1000, 1000, 1000).unwrap();
        assert_eq!((kept.caps.effective, kept.caps.permitted), (0, ALL_CAPS));
        assert_eq!(kept.capset(1 << CAP_SETUID, ALL_CAPS, 0), Ok(()));
        // Leaving root for good takes both, and with them CAP_SETUID.
        c.setresuid(1000, 1000, 1000).unwrap();
        assert_eq!((c.caps.effective, c.caps.permitted), (0, 0));
        assert_eq!(c.capset(0, 1, 0), Err(EPERM));
        // An unprivileged program's ambient capabilities are what it gets at execve.
        let mut a = Creds::default();
        a.caps.inheritable = 1 << 10;
        a.caps.ambient = 1 << 10;
        a.securebits |= SECBIT_KEEP_CAPS;
        a.setresuid(1000, 1000, 1000).unwrap();
        a.exec();
        assert_eq!((a.caps.effective, a.caps.permitted, a.securebits & SECBIT_KEEP_CAPS), (0, 0, 0));
        let mut root = Creds::default();
        root.capset(0, 1 << CAP_SETUID, 0).unwrap();
        root.exec();
        // Root's execve gives it its bounding set again.
        assert_eq!(root.caps.effective, ALL_CAPS);
        assert_eq!(root.capset(ALL_CAPS, ALL_CAPS, 1 << 63), Err(EPERM));
    }

    #[test]
    fn a_temporary_drop_can_come_back() {
        let mut c = Creds::default();
        c.setresuid(KEEP, 1000, KEEP).unwrap();
        assert_eq!(c.uid, [0, 1000, 0, 1000]);
        // The real and saved IDs are still root, so it may return.
        c.setresuid(KEEP, 0, KEEP).unwrap();
        assert_eq!(c.uid, [0; 4]);
        c.setreuid(1000, 1000).unwrap();
        assert_eq!(c.uid, [1000; 4]);
        assert_eq!(c.setreuid(KEEP, 0), Err(EPERM));
        assert_eq!(c.setfs(false, 0), 1000);
        assert_eq!(c.uid[3], 1000);
    }
}
