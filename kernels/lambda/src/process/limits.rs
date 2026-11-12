//! Resource limits (`prlimit64`, `getrlimit`, `setrlimit`): each process's soft and hard limit for each of Linux's
//! sixteen resources, inherited by its children and kept across `execve`. `RLIMIT_NOFILE` is enforced, as the size of
//! the process's descriptor table. The rest are kept and reported: nothing measures what they limit yet.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Task;
use crate::abi::*;

pub(crate) const RLIMIT_NOFILE: usize = 7;
const RLIM_NLIMITS: usize = 16;
const INFINITY: u64 = u64::MAX;
/// The most `RLIMIT_NOFILE` may be raised to (Linux's `/proc/sys/fs/nr_open`).
pub(crate) const NR_OPEN: u64 = 1 << 20;

/// Soft and hard, for each resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits(pub [(u64, u64); RLIM_NLIMITS]);

impl Default for Limits {
    /// What a Debian machine's first process has.
    fn default() -> Limits {
        let mut l = [(INFINITY, INFINITY); RLIM_NLIMITS];
        l[3] = (8 << 20, INFINITY); // RLIMIT_STACK
        l[4] = (0, INFINITY); // RLIMIT_CORE
        l[6] = (63_488, 63_488); // RLIMIT_NPROC
        l[RLIMIT_NOFILE] = (1024, 524_288);
        l[8] = (8 << 20, 8 << 20); // RLIMIT_MEMLOCK
        l[11] = (63_488, 63_488); // RLIMIT_SIGPENDING
        l[12] = (819_200, 819_200); // RLIMIT_MSGQUEUE
        l[13] = (0, 0); // RLIMIT_NICE
        l[14] = (0, 0); // RLIMIT_RTPRIO
        Limits(l)
    }
}

impl Limits {
    pub fn get(&self, resource: usize) -> (u64, u64) {
        self.0[resource]
    }
}

impl Task {
    /// `prlimit64`, and `getrlimit` and `setrlimit` for this process: writes the old limit to `old_at` and sets the
    /// one at `new_at`, either or both. Only root may raise a hard limit.
    pub(crate) fn prlimit(&mut self, cx: &mut Cx<'_>, pid: i32, resource: u64, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        let resource = usize::try_from(resource).ok().filter(|r| *r < RLIM_NLIMITS).ok_or(EINVAL)?;
        let process = if pid == 0 { self.process.clone() } else { self.k.process(pid).ok_or(ESRCH)? };
        let new = match new_at {
            0 => None,
            at => Some(self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?),
        };
        if new.is_some_and(|(soft, hard)| soft > hard) {
            return Err(EINVAL);
        }
        let privileged = lock(&self.process.state).creds.uid[1] == 0;
        let old = {
            let mut state = lock(&process.state);
            let old = state.limits.get(resource);
            if let Some((soft, hard)) = new {
                if (hard > old.1 && !privileged) || (resource == RLIMIT_NOFILE && hard > NR_OPEN) {
                    return Err(EPERM);
                }
                state.limits.0[resource] = (soft, hard);
                if resource == RLIMIT_NOFILE {
                    state.files.set_limit(soft.min(NR_OPEN) as i32);
                }
            }
            old
        };
        if old_at != 0 {
            self.mem(cx, true, |t| {
                user::write_u64(&mut t.space(), old_at, old.0)?;
                user::write_u64(&mut t.space(), old_at + 8, old.1)
            })?;
        }
        Ok(0)
    }
}
