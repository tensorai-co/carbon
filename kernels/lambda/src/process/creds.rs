//! A process's credentials: its real, effective, saved and filesystem user and group IDs, and its supplementary
//! groups, with Linux's rules for changing them. Effective root may set them to anything; anyone else may only move
//! among the IDs it already has, so a program that drops root (as `apt` does for its downloads, to `_apt`) can't get
//! it back, and can check that it can't. Files' permissions aren't checked against them yet.

use crate::abi::*;

/// "Leave it as it is", for the `setres*id` and `setre*id` calls.
const KEEP: u32 = u32::MAX;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Creds {
    /// Real, effective, saved and filesystem user IDs.
    pub uid: [u32; 4],
    pub gid: [u32; 4],
    pub groups: Vec<u32>,
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
    /// Whether it may change user IDs freely (`CAP_SETUID`, which effective root has).
    fn root(&self) -> bool {
        self.uid[1] == 0
    }

    pub fn setuid(&mut self, v: u32) -> Result<(), Errno> {
        let root = self.root();
        set_one(&mut self.uid, root, v)
    }

    pub fn setgid(&mut self, v: u32) -> Result<(), Errno> {
        let root = self.root();
        set_one(&mut self.gid, root, v)
    }

    pub fn setreuid(&mut self, r: u32, e: u32) -> Result<(), Errno> {
        let root = self.root();
        set_re(&mut self.uid, root, r, e)
    }

    pub fn setregid(&mut self, r: u32, e: u32) -> Result<(), Errno> {
        let root = self.root();
        set_re(&mut self.gid, root, r, e)
    }

    pub fn setresuid(&mut self, r: u32, e: u32, s: u32) -> Result<(), Errno> {
        let root = self.root();
        set_res(&mut self.uid, root, r, e, s)
    }

    pub fn setresgid(&mut self, r: u32, e: u32, s: u32) -> Result<(), Errno> {
        let root = self.root();
        set_res(&mut self.gid, root, r, e, s)
    }

    pub fn setgroups(&mut self, groups: Vec<u32>) -> Result<(), Errno> {
        if !self.root() {
            return Err(EPERM);
        }
        self.groups = groups;
        Ok(())
    }

    /// `setfsuid` and `setfsgid`: the old filesystem ID, always; the new one only if it's one of the others (or by
    /// root).
    pub fn setfs(&mut self, group: bool, v: u32) -> u32 {
        let root = self.root();
        let ids = if group { &mut self.gid } else { &mut self.uid };
        let old = ids[3];
        if v != KEEP && (root || ids[..3].contains(&v)) {
            ids[3] = v;
        }
        old
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
