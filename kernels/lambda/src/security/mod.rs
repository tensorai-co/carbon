//! Security: capabilities (`capget`, `capset`; the sets themselves are with the credentials, `process/creds.rs`),
//! seccomp (`seccomp.rs`), `no_new_privs`, and what lambda says of what it doesn't have.
//!
//! - **Linux Security Modules**: only `capability`, which has no attributes, as on a kernel with no SELinux,
//!   AppArmor or Smack. `lsm_list_modules` lists it; `lsm_get_self_attr` finds nothing; `lsm_set_self_attr` can't.
//! - **Landlock**: as on a kernel that has it but was booted with it off: EOPNOTSUPP, which programs that sandbox
//!   themselves "where they can" take as no Landlock, and go on.
//! - **BPF and performance counters**: refused (EPERM), as in a container.

pub(crate) mod seccomp;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use crate::Status;
use crate::abi::*;
use crate::process::{Outcome, Task};
use crate::signal::{Info, SIGSYS, SYS_SECCOMP};
use seccomp::{Seccomp, Verdict};

const LINUX_CAPABILITY_VERSION_1: u32 = 0x1998_0330;
const LINUX_CAPABILITY_VERSION_2: u32 = 0x2007_1026;
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
/// `LSM_ID_CAPABILITY`, and the attributes `lsm_get_self_attr` may be asked for (`LSM_ATTR_CURRENT` to
/// `LSM_ATTR_SOCKCREATE`).
const LSM_ID_CAPABILITY: u64 = 100;
const LSM_ATTRS: std::ops::RangeInclusive<u64> = 100..=105;
const LSM_FLAG_SINGLE: u64 = 1;

/// A process's security state, beyond its credentials: inherited by children, kept through `execve` (but for
/// `dumpable`, which `execve` sets again).
#[derive(Debug, Clone)]
pub(crate) struct Security {
    /// `PR_SET_NO_NEW_PRIVS`: nothing it runs can gain privileges, and it may install seccomp filters.
    pub no_new_privs: bool,
    pub seccomp: Option<Arc<Seccomp>>,
    /// `PR_SET_DUMPABLE`.
    pub dumpable: bool,
}

impl Default for Security {
    fn default() -> Security {
        Security { no_new_privs: false, seccomp: None, dumpable: true }
    }
}

impl Task {
    /// The process whose capabilities `capget` is asked about: 0 or its own pid is itself.
    fn cap_target(&self, pid: i32) -> Result<Arc<crate::process::Process>, Errno> {
        if pid < 0 {
            return Err(EINVAL);
        }
        if pid == 0 || pid == self.process.pid {
            return Ok(self.process.clone());
        }
        self.process_of(pid).ok_or(ESRCH)
    }

    /// `struct __user_cap_header_struct`: how many 32-bit halves of each set its version has, or, for one this isn't,
    /// EINVAL with the version this kernel prefers written back (`data_at` 0 asks only that: 0).
    fn cap_header(&mut self, cx: &mut Cx<'_>, at: u64, data_at: u64) -> Result<(usize, i32), Errno> {
        let (version, pid) = self.mem(cx, false, |t| Ok((user::read_u32(&t.space(), at)?, user::read_u32(&t.space(), at + 4)? as i32)))?;
        match version {
            LINUX_CAPABILITY_VERSION_1 => Ok((1, pid)),
            LINUX_CAPABILITY_VERSION_2 | LINUX_CAPABILITY_VERSION_3 => Ok((2, pid)),
            _ => {
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), at, LINUX_CAPABILITY_VERSION_3))?;
                Err(if data_at == 0 { Errno(0) } else { EINVAL })
            }
        }
    }

    pub(crate) fn capget(&mut self, cx: &mut Cx<'_>, header_at: u64, data_at: u64) -> Result<u64, Errno> {
        let (halves, pid) = self.cap_header(cx, header_at, data_at)?;
        let caps = lock(&self.cap_target(pid)?.state).creds.caps;
        if data_at == 0 {
            return Ok(0);
        }
        // struct __user_cap_data_struct: effective, permitted, inheritable; the low 32 bits, then the high.
        let mut b = Vec::new();
        for half in 0..halves {
            for set in [caps.effective, caps.permitted, caps.inheritable] {
                b.extend(((set >> (32 * half)) as u32).to_le_bytes());
            }
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), data_at, &b))?;
        Ok(0)
    }

    pub(crate) fn capset(&mut self, cx: &mut Cx<'_>, header_at: u64, data_at: u64) -> Result<u64, Errno> {
        let (halves, pid) = self.cap_header(cx, header_at, data_at)?;
        if pid != 0 && pid != self.process.pid {
            return Err(EPERM);
        }
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), data_at, halves * 12))?;
        let word = |i: usize| u64::from(u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or_default()));
        let set = |n: usize| (0..halves).fold(0u64, |v, half| v | word(half * 3 + n) << (32 * half));
        lock(&self.process.state).creds.capset(set(0), set(1), set(2)).map(|()| 0)
    }

    /// Runs the process's seccomp filters on a call about to be made: None if it's to go ahead, else what's become
    /// of it.
    pub(crate) fn seccomp_check(&mut self, cx: &mut Cx<'_>, nr: u64, args: &[u64; 6]) -> Option<Outcome> {
        let filters = lock(&self.process.state).security.seccomp.clone()?;
        let pc = cx.turn(|t| t.thread().regs().pc());
        let (verdict, log) = seccomp::judge(&filters, self.isa, nr, pc, args);
        if log && self.k.options.trace {
            let what = match &verdict {
                Verdict::Allow => "logged".to_string(),
                Verdict::Return(e) => format!("errno {}", e.0),
                Verdict::Trap(_) => "SIGSYS".to_string(),
                Verdict::KillThread => "thread killed".to_string(),
                Verdict::KillProcess | Verdict::Sigkill => "process killed".to_string(),
            };
            self.k.host.write(host::Stream::Err, format!("[{}] seccomp: system call {nr} at {pc:#x}: {what}\n", self.tid).as_bytes());
        }
        match verdict {
            Verdict::Allow => None,
            // An errno of 0 is a call that "succeeds" without being made.
            Verdict::Return(e) => Some(Outcome::Return((-i64::from(e.0)) as u64)),
            Verdict::Trap(errno) => {
                let info = Info { code: SYS_SECCOMP, addr: pc, errno: i32::from(errno), syscall: (nr as i32, seccomp::audit_arch(self.isa)), ..Info::default() };
                self.force_signal(SIGSYS, info);
                Some(Err(ENOSYS).into())
            }
            Verdict::KillThread if lock(&self.process.state).threads.len() > 1 => Some(Outcome::End),
            Verdict::KillThread | Verdict::KillProcess => {
                self.process.end(Status::Killed(SIGSYS));
                Some(Outcome::End)
            }
            Verdict::Sigkill => {
                self.process.end(Status::Killed(SIGKILL));
                Some(Outcome::End)
            }
        }
    }

    /// `PR_SET_NO_NEW_PRIVS`: once set, never cleared.
    pub(crate) fn set_no_new_privs(&mut self) {
        lock(&self.process.state).security.no_new_privs = true;
    }

    pub(crate) fn lsm_list_modules(&mut self, cx: &mut Cx<'_>, ids_at: u64, size_at: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 {
            return Err(EINVAL);
        }
        let room = self.mem(cx, false, |t| user::read_u32(&t.space(), size_at))?;
        self.mem(cx, true, |t| user::write_u32(&mut t.space(), size_at, 8))?;
        if room < 8 {
            return Err(E2BIG);
        }
        self.mem(cx, true, |t| user::write_u64(&mut t.space(), ids_at, LSM_ID_CAPABILITY))?;
        Ok(1)
    }

    /// `lsm_get_self_attr`: `capability` has no attributes, so there are none to give.
    pub(crate) fn lsm_get_self_attr(&mut self, cx: &mut Cx<'_>, attr: u64, size_at: u64, flags: u64) -> Result<u64, Errno> {
        if flags & !LSM_FLAG_SINGLE != 0 || !LSM_ATTRS.contains(&attr) {
            return Err(EINVAL);
        }
        // LSM_FLAG_SINGLE names a module that would have it: there's none.
        if flags & LSM_FLAG_SINGLE != 0 {
            return Err(EINVAL);
        }
        self.mem(cx, false, |t| user::read_u32(&t.space(), size_at))?;
        self.mem(cx, true, |t| user::write_u32(&mut t.space(), size_at, 0))?;
        Ok(0)
    }

    pub(crate) fn lsm_set_self_attr(&mut self, attr: u64, flags: u64) -> Result<u64, Errno> {
        if flags != 0 || !LSM_ATTRS.contains(&attr) {
            return Err(EINVAL);
        }
        Err(EOPNOTSUPP)
    }

    /// Whether the process has seccomp filters (or strict mode) to check each call against, without its lock.
    pub(crate) fn filtered(&self) -> bool {
        self.process.filtered.load(Ordering::Acquire)
    }
}
