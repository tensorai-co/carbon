//! The machine's administration, as a container's root sees it: its own host and domain names (`sethostname`,
//! `setdomainname`, as `uname` gives them), `reboot` ending the machine (as it ends a PID namespace: halt and power-off
//! as SIGINT, restart as SIGHUP), the kernel's log (`syslog`, which `dmesg` reads), `vhangup`, and no quotas.
//!
//! What would reach past the machine is refused with EPERM, as in a container: loading kernel modules, `kexec`,
//! process accounting, and x86's I/O ports (`iopl` and `ioperm`, which may only lower what's there: nothing).

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use crate::abi::*;
use crate::process::Task;

use crate::process::creds::{CAP_SYS_ADMIN, CAP_SYS_BOOT, CAP_SYS_TTY_CONFIG, CAP_SYSLOG};
use crate::signal::{SIGHUP, SIGINT};
use crate::{Kernel, Status};

/// `__NEW_UTS_LEN`: the longest host or domain name.
const UTS_LEN: u64 = 64;
const LINUX_REBOOT_MAGIC1: u64 = 0xfee1_dead;
const LINUX_REBOOT_MAGIC2: [u64; 4] = [0x2812_1969, 0x0512_1996, 0x1604_1998, 0x2011_2000];
const LINUX_REBOOT_CMD_CAD_OFF: u32 = 0;
const LINUX_REBOOT_CMD_CAD_ON: u32 = 0x89ab_cdef;
const LINUX_REBOOT_CMD_RESTART: u32 = 0x0123_4567;
const LINUX_REBOOT_CMD_RESTART2: u32 = 0xa1b2_c3d4;
const LINUX_REBOOT_CMD_HALT: u32 = 0xcdef_0123;
const LINUX_REBOOT_CMD_POWER_OFF: u32 = 0x4321_fedc;
/// The kernel log's size, as `SYSLOG_ACTION_SIZE_BUFFER` gives it.
const LOG_SIZE: u64 = 1 << 17;
/// quotactl's subcommands: `cmd >> 8` (its low byte is the quota type).
const Q_SYNC: u64 = 0x80_0001;
const Q_QUOTAON: u64 = 0x80_0002;

/// The machine's names, as `uname` gives them.
#[derive(Debug, Clone)]
pub(crate) struct Names {
    pub host: String,
    pub domain: String,
}

/// The kernel's log: what it said as the machine started, where `SYSLOG_ACTION_READ` has read to, and where
/// `SYSLOG_ACTION_CLEAR` cleared it from.
#[derive(Debug, Default)]
pub(crate) struct Klog {
    text: String,
    read: usize,
    cleared: usize,
}

impl Klog {
    pub fn boot(cpus: usize, memory: u64) -> Klog {
        let line = |level: u8, text: String| format!("<{level}>[    0.000000] {text}\n");
        let text = [
            line(5, format!("Linux version {} (lambda) {}", RELEASE, VERSION)),
            line(6, format!("lambda: {cpus} CPUs, {} MiB", memory >> 20)),
            line(6, "Run /sbin/init as init process".to_string()),
        ]
        .concat();
        Klog { text, read: 0, cleared: 0 }
    }
}

impl Kernel {
    /// Ends the machine, as `reboot` ends a PID namespace: its first process by `signal`, and the rest killed.
    fn shut_down(&self, signal: i32) {
        if let Some(init) = self.process(1) {
            init.end(Status::Killed(signal));
        }
        self.end_everything();
    }
}

impl Task {
    fn capable(&self, cap: u32) -> bool {
        lock(&self.process.state).creds.capable(cap)
    }

    /// `sethostname` and `setdomainname`.
    pub(crate) fn set_name(&mut self, cx: &mut Cx<'_>, domain: bool, at: u64, len: u64) -> Result<u64, Errno> {
        if !self.capable(CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        if len as i64 > UTS_LEN as i64 || (len as i64) < 0 {
            return Err(EINVAL);
        }
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize))?;
        let name = String::from_utf8_lossy(&bytes).into_owned();
        let uts = self.uts_ns();
        let mut names = lock(&uts.names);
        if domain {
            names.domain = name;
        } else {
            names.host = name;
        }
        Ok(0)
    }

    pub(crate) fn reboot(&mut self, magic1: u64, magic2: u64, cmd: u64) -> Result<u64, Errno> {
        if magic1 as u32 as u64 != LINUX_REBOOT_MAGIC1 || !LINUX_REBOOT_MAGIC2.contains(&(magic2 as u32 as u64)) {
            return Err(EINVAL);
        }
        if !self.capable(CAP_SYS_BOOT) {
            return Err(EPERM);
        }
        match cmd as u32 {
            LINUX_REBOOT_CMD_CAD_ON | LINUX_REBOOT_CMD_CAD_OFF => Ok(0),
            LINUX_REBOOT_CMD_RESTART | LINUX_REBOOT_CMD_RESTART2 => {
                self.k.shut_down(SIGHUP);
                Ok(0)
            }
            LINUX_REBOOT_CMD_HALT | LINUX_REBOOT_CMD_POWER_OFF => {
                self.k.shut_down(SIGINT);
                Ok(0)
            }
            // Suspending, or kexec: not a machine's to do.
            _ => Err(EINVAL),
        }
    }

    /// `syslog` (`klogctl`): anyone may read the whole log and ask its size (`dmesg_restrict` is 0); the rest takes
    /// `CAP_SYSLOG` or `CAP_SYS_ADMIN`.
    pub(crate) fn syslog(&mut self, cx: &mut Cx<'_>, action: u64, buf: u64, len: u64) -> Result<u64, Errno> {
        let (action, len) = (action as i32, len as i32);
        if !matches!(action, 3 | 10) && !self.capable(CAP_SYSLOG) && !self.capable(CAP_SYS_ADMIN) {
            return Err(EPERM);
        }
        let reads = matches!(action, 2..=4);
        if reads && (buf == 0 || len < 0) {
            return Err(EINVAL);
        }
        if reads && len == 0 {
            return Ok(0);
        }
        let out = {
            let mut log = lock(&self.k.klog);
            match action {
                // Close, open, console off and on.
                0 | 1 | 6 | 7 => return Ok(0),
                // Read what's not been read: the log never grows after the machine starts, so once it's all read,
                // a reader waits for nothing, as on Linux with nothing new, until a signal.
                2 => {
                    let start = log.read.max(log.cleared);
                    if start == log.text.len() {
                        drop(log);
                        self.waiter.reset();
                        self.waiter.wait(None);
                        return Err(EINTR);
                    }
                    let end = (start + len as usize).min(log.text.len());
                    log.read = end;
                    log.text.as_bytes()[start..end].to_vec()
                }
                // The latest that fits, since it was last cleared; and clearing it.
                3 | 4 => {
                    let all = &log.text.as_bytes()[log.cleared..];
                    let out = all[all.len().saturating_sub(len as usize)..].to_vec();
                    if action == 4 {
                        log.cleared = log.text.len();
                    }
                    out
                }
                5 => {
                    log.cleared = log.text.len();
                    return Ok(0);
                }
                8 => return if (1..=8).contains(&len) { Ok(0) } else { Err(EINVAL) },
                9 => return Ok((log.text.len() - log.read.max(log.cleared)) as u64),
                10 => return Ok(LOG_SIZE),
                _ => return Err(EINVAL),
            }
        };
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &out))?;
        Ok(out.len() as u64)
    }

    /// `vhangup`: the controlling terminal hangs up, as if the line dropped: its foreground group gets SIGHUP and
    /// SIGCONT, and its session loses it.
    pub(crate) fn vhangup(&mut self) -> Result<u64, Errno> {
        if !self.capable(CAP_SYS_TTY_CONFIG) {
            return Err(EPERM);
        }
        let Some(tty) = lock(&self.process.state).ctty.clone() else { return Ok(0) };
        let foreground = tty.foreground();
        tty.set_session(None, 0);
        for process in lock(&self.k.processes).values() {
            let mut state = lock(&process.state);
            if state.ctty.as_ref().is_some_and(|t| std::sync::Arc::ptr_eq(t, &tty)) {
                state.ctty = None;
            }
        }
        if foreground > 0 {
            self.k.signal_group(foreground, SIGHUP);
            self.k.signal_group(foreground, crate::signal::SIGCONT);
        }
        Ok(0)
    }

    /// `quotactl` and `quotactl_fd`: the disk keeps no quotas. Syncing them is nothing to do; turning them on can't be
    /// done; anything else finds them off (ESRCH).
    pub(crate) fn quotactl(&mut self, cmd: u64, fd: Option<i32>) -> Result<u64, Errno> {
        if let Some(fd) = fd {
            self.file(fd)?;
        }
        match (cmd as u32 as u64) >> 8 {
            Q_SYNC => Ok(0),
            Q_QUOTAON => Err(EINVAL),
            _ => Err(ESRCH),
        }
    }

    /// x86-64's `iopl`: lowering it (to 0, where it is) is allowed; anything else reaches the host's ports.
    pub(crate) fn iopl(&mut self, level: u64) -> Result<u64, Errno> {
        match level {
            0 => Ok(0),
            1..=3 => Err(EPERM),
            _ => Err(EINVAL),
        }
    }

    /// x86-64's `ioperm`: turning ports off is allowed (they're off); turning them on isn't.
    pub(crate) fn ioperm(&mut self, from: u64, count: u64, on: u64) -> Result<u64, Errno> {
        if from.checked_add(count).is_none_or(|end| end > 0x10000) {
            return Err(EINVAL);
        }
        if on == 0 { Ok(0) } else { Err(EPERM) }
    }
}

/// The file system types lambda can mount, as `sysfs` lists them.
const FS_TYPES: [&str; 5] = ["ext4", "tmpfs", "proc", "devpts", "mqueue"];

impl Task {
    /// x86-64's `ustat`: the free blocks and inodes of the mounted file system with device `dev`, in a `struct ustat`.
    pub(crate) fn ustat(&mut self, cx: &mut Cx<'_>, dev: u32, at: u64) -> Result<u64, Errno> {
        let (major, minor) = ((dev & 0xfff00) >> 8, (dev & 0xff) | ((dev >> 12) & 0xfff00));
        let mounts = self.view().mounts;
        if !mounts.list.iter().any(|m| m.dev() == (major, minor)) {
            return Err(EINVAL);
        }
        let free = lock(&self.k.exec).memory().size().available / 4096;
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&(free.min(i32::MAX as u64) as i32).to_le_bytes());
        b[8..16].copy_from_slice(&(1u64 << 20).to_le_bytes());
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }

    /// x86-64's `sysfs`: the file system types the kernel knows: a name's index (1), the name at an index (2), or
    /// how many there are (3).
    pub(crate) fn sysfs(&mut self, cx: &mut Cx<'_>, option: u64, a: u64, b: u64) -> Result<u64, Errno> {
        match option {
            1 => {
                let name = self.mem(cx, false, |t| user::read_cstr(&t.space(), a, 256))?;
                FS_TYPES.iter().position(|t| t.as_bytes() == name.as_slice()).map(|i| i as u64).ok_or(EINVAL)
            }
            2 => {
                let name = FS_TYPES.get(a as u32 as usize).ok_or(EINVAL)?;
                let mut bytes = name.as_bytes().to_vec();
                bytes.push(0);
                self.mem(cx, true, |t| user::write(&mut t.space(), b, &bytes))?;
                Ok(0)
            }
            3 => Ok(FS_TYPES.len() as u64),
            _ => Err(EINVAL),
        }
    }

    /// x86-64's `modify_ldt`. No process has an LDT: reading one gives nothing, and the default one zeros. Setting an
    /// entry is EINVAL, as lambda's x86-64 has no segments for a selector to load.
    pub(crate) fn modify_ldt(&mut self, cx: &mut Cx<'_>, func: u64, at: u64, count: u64) -> Result<u64, Errno> {
        match func as u32 {
            0 => Ok(0),
            2 => {
                let n = count.min(5 * 8) as usize;
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &vec![0; n]))?;
                Ok(n as u64)
            }
            1 | 0x11 => Err(EINVAL),
            _ => Err(ENOSYS),
        }
    }
}
