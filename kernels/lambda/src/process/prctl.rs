//! `prctl`: the options that have real answers here — a process's name, whether it's dumpable, its capability
//! bounding and ambient sets, securebits and keep-caps, `no_new_privs`, and seccomp. Options not listed are still
//! accepted and do nothing (returning 0), as before they were looked at one by one.
//!
//! A name is a process's here, not each thread's: `PR_SET_NAME` from any thread names the process, as `/proc`'s
//! `status` shows it.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Task;
use super::creds::{CAP_LAST_CAP, CAP_SETPCAP, SECBIT_KEEP_CAPS, SECBIT_NO_CAP_AMBIENT_RAISE};
use crate::abi::*;
use crate::security::seccomp::Seccomp;

const PR_GET_DUMPABLE: u64 = 3;
const PR_SET_DUMPABLE: u64 = 4;
const PR_GET_KEEPCAPS: u64 = 7;
const PR_SET_KEEPCAPS: u64 = 8;
const PR_SET_NAME: u64 = 15;
const PR_GET_NAME: u64 = 16;
const PR_GET_SECCOMP: u64 = 21;
const PR_SET_SECCOMP: u64 = 22;
const PR_CAPBSET_READ: u64 = 23;
const PR_CAPBSET_DROP: u64 = 24;
const PR_GET_SECUREBITS: u64 = 27;
const PR_SET_SECUREBITS: u64 = 28;
const PR_SET_NO_NEW_PRIVS: u64 = 38;
const PR_GET_NO_NEW_PRIVS: u64 = 39;
const PR_CAP_AMBIENT: u64 = 47;
const PR_CAP_AMBIENT_IS_SET: u64 = 1;
const PR_CAP_AMBIENT_RAISE: u64 = 2;
const PR_CAP_AMBIENT_LOWER: u64 = 3;
const PR_CAP_AMBIENT_CLEAR_ALL: u64 = 4;
const SECCOMP_MODE_STRICT: u64 = 1;
const SECCOMP_MODE_FILTER: u64 = 2;
/// Securebits and the bits that lock them (`SECURE_ALL_BITS`, `SECURE_ALL_LOCKS`).
const SECURE_ALL_BITS: u32 = 0x555;
const SECURE_ALL_LOCKS: u32 = 0xaaa;
const SECBIT_KEEP_CAPS_LOCKED: u32 = 1 << 5;
/// `TASK_COMM_LEN`.
const COMM: usize = 16;

/// A capability number `prctl` was given, if it's one.
fn cap(n: u64) -> Result<u64, Errno> {
    if n > u64::from(CAP_LAST_CAP) { Err(EINVAL) } else { Ok(1 << n) }
}

impl Task {
    /// The process's name: what `PR_SET_NAME` set, or its program's file name, at most 15 bytes.
    pub(crate) fn comm(state: &super::ProcState) -> String {
        let name = state.comm.clone().unwrap_or_else(|| state.exe.rsplit('/').next().unwrap_or("").to_string());
        let mut end = name.len().min(COMM - 1);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name[..end].to_string()
    }

    pub(crate) fn prctl(&mut self, cx: &mut Cx<'_>, option: u64, a: [u64; 4]) -> Result<u64, Errno> {
        match option {
            PR_GET_DUMPABLE => Ok(u64::from(lock(&self.process.state).security.dumpable)),
            PR_SET_DUMPABLE => {
                if a[0] > 1 {
                    return Err(EINVAL);
                }
                lock(&self.process.state).security.dumpable = a[0] == 1;
                Ok(0)
            }
            PR_GET_KEEPCAPS => Ok(u64::from(lock(&self.process.state).creds.securebits & SECBIT_KEEP_CAPS != 0)),
            PR_SET_KEEPCAPS => {
                if a[0] > 1 {
                    return Err(EINVAL);
                }
                let creds = &mut lock(&self.process.state).creds;
                if creds.securebits & SECBIT_KEEP_CAPS_LOCKED != 0 {
                    return Err(EPERM);
                }
                creds.securebits = if a[0] == 1 { creds.securebits | SECBIT_KEEP_CAPS } else { creds.securebits & !SECBIT_KEEP_CAPS };
                Ok(0)
            }
            PR_SET_NAME => {
                // At most 15 bytes and a NUL; a longer one is cut.
                let name = match self.mem(cx, false, |t| user::read_cstr(&t.space(), a[0], COMM)) {
                    Ok(name) => name,
                    Err(_) => self.mem(cx, false, |t| user::read_vec(&t.space(), a[0], COMM - 1))?,
                };
                let name: Vec<u8> = name.into_iter().take(COMM - 1).collect();
                lock(&self.process.state).comm = Some(String::from_utf8_lossy(&name).into_owned());
                Ok(0)
            }
            PR_GET_NAME => {
                let mut name = Task::comm(&lock(&self.process.state)).into_bytes();
                name.resize(COMM, 0);
                self.mem(cx, true, |t| user::write(&mut t.space(), a[0], &name))?;
                Ok(0)
            }
            PR_GET_SECCOMP => Ok(Seccomp::mode(&lock(&self.process.state).security.seccomp)),
            PR_SET_SECCOMP => match a[0] {
                SECCOMP_MODE_STRICT => self.set_seccomp(None),
                SECCOMP_MODE_FILTER => {
                    let filter = self.read_filter(cx, a[1], false)?;
                    self.set_seccomp(Some(filter))
                }
                _ => Err(EINVAL),
            },
            PR_CAPBSET_READ => Ok(u64::from(lock(&self.process.state).creds.caps.bounding & cap(a[0])? != 0)),
            PR_CAPBSET_DROP => {
                let bit = cap(a[0])?;
                let creds = &mut lock(&self.process.state).creds;
                if !creds.capable(CAP_SETPCAP) {
                    return Err(EPERM);
                }
                creds.caps.bounding &= !bit;
                Ok(0)
            }
            PR_GET_SECUREBITS => Ok(u64::from(lock(&self.process.state).creds.securebits)),
            PR_SET_SECUREBITS => {
                let creds = &mut lock(&self.process.state).creds;
                let (old, new) = (creds.securebits, a[0]);
                // A locked bit can't change, a lock can't be undone, and only bits and locks there are.
                if u64::from((old & SECURE_ALL_LOCKS) >> 1) & (u64::from(old) ^ new) != 0
                    || u64::from(old & SECURE_ALL_LOCKS) & !new != 0
                    || new & !u64::from(SECURE_ALL_BITS | SECURE_ALL_LOCKS) != 0
                    || !creds.capable(CAP_SETPCAP)
                {
                    return Err(EPERM);
                }
                creds.securebits = new as u32;
                Ok(0)
            }
            PR_SET_NO_NEW_PRIVS => {
                if a[0] != 1 || a[1..].iter().any(|v| *v != 0) {
                    return Err(EINVAL);
                }
                self.set_no_new_privs();
                Ok(0)
            }
            PR_GET_NO_NEW_PRIVS => {
                if a.iter().any(|v| *v != 0) {
                    return Err(EINVAL);
                }
                Ok(u64::from(lock(&self.process.state).security.no_new_privs))
            }
            PR_CAP_AMBIENT => {
                let creds = &mut lock(&self.process.state).creds;
                if a[0] == PR_CAP_AMBIENT_CLEAR_ALL {
                    if a[1..].iter().any(|v| *v != 0) {
                        return Err(EINVAL);
                    }
                    creds.caps.ambient = 0;
                    return Ok(0);
                }
                if a[2] != 0 || a[3] != 0 {
                    return Err(EINVAL);
                }
                let bit = cap(a[1])?;
                match a[0] {
                    PR_CAP_AMBIENT_IS_SET => Ok(u64::from(creds.caps.ambient & bit != 0)),
                    PR_CAP_AMBIENT_RAISE => {
                        if creds.caps.permitted & bit == 0 || creds.caps.inheritable & bit == 0 || creds.securebits & SECBIT_NO_CAP_AMBIENT_RAISE != 0 {
                            return Err(EPERM);
                        }
                        creds.caps.ambient |= bit;
                        Ok(0)
                    }
                    PR_CAP_AMBIENT_LOWER => {
                        creds.caps.ambient &= !bit;
                        Ok(0)
                    }
                    _ => Err(EINVAL),
                }
            }
            _ => Ok(0),
        }
    }
}
