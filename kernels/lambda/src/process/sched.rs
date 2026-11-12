//! Scheduling and priorities: each process's nice value (`getpriority`, `setpriority`), scheduling policy and
//! priority (`sched_setscheduler`, `sched_setparam`, `sched_setattr` and their `get` forms), CPU affinity
//! (`sched_setaffinity`, and `getcpu` within it) and I/O priority (`ioprio_set`, `ioprio_get`).
//!
//! They're kept, checked as Linux checks them, inherited by children (`SCHED_RESET_ON_FORK` aside) and reported, but
//! change nothing: every guest thread is a host thread, and the host schedules them as it does its own. On Linux a
//! thread has its own of each; here a process's threads share one, and a thread's ID names its process.

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::Task;
use crate::abi::*;

// Scheduling policies.
const SCHED_OTHER: u32 = 0;
const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
const SCHED_BATCH: u32 = 3;
const SCHED_IDLE: u32 = 5;
const SCHED_DEADLINE: u32 = 6;
const SCHED_EXT: u32 = 7;
/// Or'd into a policy: children start with the default policy and no negative nice.
const SCHED_RESET_ON_FORK: u32 = 0x4000_0000;
// `sched_attr`'s flags.
const SCHED_FLAG_RESET_ON_FORK: u64 = 0x01;
const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;
const SCHED_FLAG_ALL: u64 = 0x7f;
/// `sched_attr`'s first size, and its size now (with the utilization clamps).
const SCHED_ATTR_SIZE_VER0: u32 = 48;
const SCHED_ATTR_SIZE_VER1: u32 = 56;
/// `sched_rr_get_interval`'s answer for `SCHED_RR`, as Linux's default.
const RR_INTERVAL: u64 = 100_000_000;

// Whose priority, for `getpriority` and `setpriority`.
const PRIO_PROCESS: u64 = 0;
const PRIO_PGRP: u64 = 1;
const PRIO_USER: u64 = 2;

// Whose I/O priority: one process, a process group (2), or a user's processes; `PRIO_*` plus one.
const IOPRIO_WHO_PROCESS: u64 = 1;
const IOPRIO_WHO_USER: u64 = 3;
/// The class, in the top three of an I/O priority's 16 bits: none, real time, best effort, idle.
const IOPRIO_CLASS_SHIFT: u32 = 13;

/// A process's scheduling, as it was last set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sched {
    /// -20 (most favoured) to 19.
    pub nice: i32,
    pub policy: u32,
    /// 1 to 99 for `SCHED_FIFO` and `SCHED_RR`, else 0.
    pub priority: u32,
    pub reset_on_fork: bool,
    /// `SCHED_DEADLINE`'s runtime, deadline and period, in nanoseconds.
    pub deadline: [u64; 3],
    /// The utilization clamps, 0 to 1024.
    pub util: [u32; 2],
    /// The CPUs it may run on, one bit each.
    pub affinity: u64,
    pub ioprio: u16,
}

impl Sched {
    /// A machine's first process: the default policy, on every CPU it has.
    pub fn new(cpus: usize) -> Sched {
        Sched { nice: 0, policy: SCHED_OTHER, priority: 0, reset_on_fork: false, deadline: [0; 3], util: [0, 1024], affinity: all_cpus(cpus), ioprio: 0 }
    }

    /// What a child starts with: the same, unless `SCHED_RESET_ON_FORK` asked for the defaults.
    pub fn forked(&self) -> Sched {
        let mut child = *self;
        if self.reset_on_fork {
            if matches!(self.policy, SCHED_FIFO | SCHED_RR | SCHED_DEADLINE) {
                child.policy = SCHED_OTHER;
                child.priority = 0;
            }
            child.nice = child.nice.max(0);
            child.reset_on_fork = false;
        }
        child
    }
}

/// One bit for each of the host's CPUs, up to 64.
fn all_cpus(cpus: usize) -> u64 {
    match cpus.min(64) {
        64 => u64::MAX,
        n => (1 << n) - 1,
    }
}

/// The lowest and highest priority `policy` takes, or None for one Linux doesn't have.
fn priority_range(policy: u32) -> Option<(u32, u32)> {
    match policy {
        SCHED_FIFO | SCHED_RR => Some((1, 99)),
        SCHED_OTHER | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE | SCHED_EXT => Some((0, 0)),
        _ => None,
    }
}

impl Task {
    fn privileged(&self) -> bool {
        lock(&self.process.state).creds.uid[1] == 0
    }

    /// The process `pid` names for the `sched_*` calls: this one for 0, else the one it is or has as a thread.
    fn sched_target(&self, pid: i32) -> Result<std::sync::Arc<super::Process>, Errno> {
        if pid < 0 {
            return Err(EINVAL);
        }
        self.process_of(pid).ok_or(ESRCH)
    }

    /// The processes `which` and `who` mean, for `getpriority`, `setpriority` and the `ioprio` calls (whose `which`
    /// counts from 1); ESRCH if there are none.
    fn whose(&self, which: u64, who: i32) -> Result<Vec<std::sync::Arc<super::Process>>, Errno> {
        let (pgid, uid) = {
            let state = lock(&self.process.state);
            (state.pgid, state.creds.uid[0])
        };
        let found: Vec<_> = match which {
            PRIO_PROCESS => vec![self.sched_target(who)?],
            PRIO_PGRP => {
                let group = if who == 0 { pgid } else { who };
                lock(&self.k.processes).values().filter(|p| lock(&p.state).pgid == group).cloned().collect()
            }
            PRIO_USER => {
                let user = if who == 0 { uid } else { who as u32 };
                lock(&self.k.processes).values().filter(|p| lock(&p.state).creds.uid[0] == user).cloned().collect()
            }
            _ => return Err(EINVAL),
        };
        if found.is_empty() { Err(ESRCH) } else { Ok(found) }
    }

    /// `getpriority`: the most favoured nice among those meant, as Linux returns it, 20 minus nice (1 to 40).
    pub(crate) fn getpriority(&mut self, which: u64, who: i32) -> Result<u64, Errno> {
        let best = self.whose(which, who)?.iter().map(|p| lock(&p.state).sched.nice).min().unwrap_or(0);
        Ok((20 - best) as u64)
    }

    /// `setpriority`: a nice for each process meant, clamped to -20 to 19. Only root may make one more favoured.
    pub(crate) fn setpriority(&mut self, which: u64, who: i32, nice: i32) -> Result<u64, Errno> {
        let nice = nice.clamp(-20, 19);
        let privileged = self.privileged();
        for process in self.whose(which, who)? {
            let mut state = lock(&process.state);
            if nice < state.sched.nice && !privileged {
                return Err(EACCES);
            }
            state.sched.nice = nice;
        }
        Ok(0)
    }

    /// A `sched_param`'s priority.
    fn read_param(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u32, Errno> {
        if at == 0 {
            return Err(EINVAL);
        }
        self.mem(cx, false, |t| user::read_u32(&t.space(), at))
    }

    /// Sets `policy` and `priority` on `pid`'s process, as `sched_setscheduler` and `sched_setparam` do, checking the
    /// priority is in the policy's range. Only root may choose a real-time policy.
    fn set_sched(&self, pid: i32, policy: u32, priority: u32, reset_on_fork: Option<bool>) -> Result<(), Errno> {
        let (low, high) = priority_range(policy).ok_or(EINVAL)?;
        if priority < low || priority > high {
            return Err(EINVAL);
        }
        if matches!(policy, SCHED_FIFO | SCHED_RR | SCHED_DEADLINE) && !self.privileged() {
            return Err(EPERM);
        }
        let process = self.sched_target(pid)?;
        let mut state = lock(&process.state);
        state.sched.policy = policy;
        state.sched.priority = priority;
        if let Some(reset) = reset_on_fork {
            state.sched.reset_on_fork = reset;
        }
        Ok(())
    }

    pub(crate) fn sched_setscheduler(&mut self, cx: &mut Cx<'_>, pid: i32, policy: u64, param_at: u64) -> Result<u64, Errno> {
        let policy = policy as u32;
        let priority = self.read_param(cx, param_at)?;
        self.set_sched(pid, policy & !SCHED_RESET_ON_FORK, priority, Some(policy & SCHED_RESET_ON_FORK != 0))?;
        Ok(0)
    }

    pub(crate) fn sched_getscheduler(&mut self, pid: i32) -> Result<u64, Errno> {
        let process = self.sched_target(pid)?;
        let sched = lock(&process.state).sched;
        Ok(u64::from(sched.policy | if sched.reset_on_fork { SCHED_RESET_ON_FORK } else { 0 }))
    }

    pub(crate) fn sched_setparam(&mut self, cx: &mut Cx<'_>, pid: i32, param_at: u64) -> Result<u64, Errno> {
        let priority = self.read_param(cx, param_at)?;
        let policy = lock(&self.sched_target(pid)?.state).sched.policy;
        self.set_sched(pid, policy, priority, None)?;
        Ok(0)
    }

    pub(crate) fn sched_getparam(&mut self, cx: &mut Cx<'_>, pid: i32, param_at: u64) -> Result<u64, Errno> {
        if param_at == 0 {
            return Err(EINVAL);
        }
        let priority = lock(&self.sched_target(pid)?.state).sched.priority;
        self.mem(cx, true, |t| user::write_u32(&mut t.space(), param_at, priority))?;
        Ok(0)
    }

    pub(crate) fn sched_priority_bound(&mut self, policy: u64, max: bool) -> Result<u64, Errno> {
        let (low, high) = priority_range(u32::try_from(policy).map_err(|_| EINVAL)?).ok_or(EINVAL)?;
        Ok(u64::from(if max { high } else { low }))
    }

    /// `sched_rr_get_interval`: a time slice for `SCHED_RR`, and none for the rest.
    pub(crate) fn sched_rr_get_interval(&mut self, cx: &mut Cx<'_>, pid: i32, at: u64) -> Result<u64, Errno> {
        let policy = lock(&self.sched_target(pid)?.state).sched.policy;
        let slice = if policy == SCHED_RR { RR_INTERVAL } else { 0 };
        self.mem(cx, true, |t| {
            user::write_u64(&mut t.space(), at, slice / 1_000_000_000)?;
            user::write_u64(&mut t.space(), at + 8, slice % 1_000_000_000)
        })?;
        Ok(0)
    }

    /// `sched_setattr`: a policy with all its parameters at once, in a `sched_attr` that says its own size.
    pub(crate) fn sched_setattr(&mut self, cx: &mut Cx<'_>, pid: i32, at: u64, flags: u64) -> Result<u64, Errno> {
        if at == 0 || flags != 0 || pid < 0 {
            return Err(EINVAL);
        }
        let mut size = self.mem(cx, false, |t| user::read_u32(&t.space(), at))?;
        if size == 0 {
            size = SCHED_ATTR_SIZE_VER0;
        }
        // Too small, or bigger with something set past what's known: E2BIG, with the size known written back.
        let too_big = size > SCHED_ATTR_SIZE_VER1 && {
            let rest = self.mem(cx, false, |t| user::read_vec(&t.space(), at + u64::from(SCHED_ATTR_SIZE_VER1), (size - SCHED_ATTR_SIZE_VER1).min(4096) as usize))?;
            rest.iter().any(|b| *b != 0)
        };
        if !(SCHED_ATTR_SIZE_VER0..=4096).contains(&size) || too_big {
            self.mem(cx, true, |t| user::write_u32(&mut t.space(), at, SCHED_ATTR_SIZE_VER1))?;
            return Err(E2BIG);
        }
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, size.min(SCHED_ATTR_SIZE_VER1) as usize))?;
        let (u32_at, u64_at) = (|i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default()), |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or_default()));
        let (policy, sched_flags, nice, priority) = (u32_at(4), u64_at(8), u32_at(16) as i32, u32_at(20));
        let deadline = [u64_at(24), u64_at(32), u64_at(40)];
        let util = if b.len() >= 56 { [u32_at(48), u32_at(52)] } else { [0, 1024] };
        if sched_flags & !SCHED_FLAG_ALL != 0 || util[0] > 1024 || util[1] > 1024 || util[0] > util[1] {
            return Err(EINVAL);
        }
        let process = self.sched_target(pid)?;
        let current = lock(&process.state).sched;
        let policy = if sched_flags & SCHED_FLAG_KEEP_POLICY != 0 { current.policy } else { policy };
        if policy == SCHED_DEADLINE && sched_flags & SCHED_FLAG_KEEP_PARAMS == 0 {
            // At least 1024 ns of runtime, within the deadline, within the period (0: the deadline's).
            let [runtime, deadline_ns, period] = deadline;
            let period = if period == 0 { deadline_ns } else { period };
            if runtime < 1024 || runtime > deadline_ns || deadline_ns > period {
                return Err(EINVAL);
            }
        }
        if sched_flags & SCHED_FLAG_KEEP_PARAMS == 0 {
            self.set_sched(pid, policy, priority, Some(sched_flags & SCHED_FLAG_RESET_ON_FORK != 0))?;
        } else {
            self.set_sched(pid, policy, current.priority, Some(sched_flags & SCHED_FLAG_RESET_ON_FORK != 0))?;
        }
        let privileged = self.privileged();
        let mut state = lock(&process.state);
        if matches!(policy, SCHED_OTHER | SCHED_BATCH | SCHED_EXT) && sched_flags & SCHED_FLAG_KEEP_PARAMS == 0 {
            let nice = nice.clamp(-20, 19);
            if nice < state.sched.nice && !privileged {
                return Err(EPERM);
            }
            state.sched.nice = nice;
        }
        if policy == SCHED_DEADLINE && sched_flags & SCHED_FLAG_KEEP_PARAMS == 0 {
            state.sched.deadline = deadline;
        }
        if sched_flags & SCHED_FLAG_UTIL_CLAMP_MIN != 0 {
            state.sched.util[0] = util[0];
        }
        if sched_flags & SCHED_FLAG_UTIL_CLAMP_MAX != 0 {
            state.sched.util[1] = util[1];
        }
        Ok(0)
    }

    /// `sched_getattr`: as much of a `sched_attr` as `size` has room for, up to the size known, which it says.
    pub(crate) fn sched_getattr(&mut self, cx: &mut Cx<'_>, pid: i32, at: u64, size: u64, flags: u64) -> Result<u64, Errno> {
        if at == 0 || flags != 0 || pid < 0 || size < u64::from(SCHED_ATTR_SIZE_VER0) || size > 4096 {
            return Err(EINVAL);
        }
        let sched = lock(&self.sched_target(pid)?.state).sched;
        let size = (size as u32).min(SCHED_ATTR_SIZE_VER1);
        let mut b = Vec::with_capacity(56);
        b.extend(size.to_le_bytes());
        b.extend(sched.policy.to_le_bytes());
        b.extend((if sched.reset_on_fork { SCHED_FLAG_RESET_ON_FORK } else { 0 } | SCHED_FLAG_UTIL_CLAMP_MIN | SCHED_FLAG_UTIL_CLAMP_MAX).to_le_bytes());
        b.extend(sched.nice.to_le_bytes());
        b.extend(sched.priority.to_le_bytes());
        for v in sched.deadline {
            b.extend(v.to_le_bytes());
        }
        b.extend(sched.util[0].to_le_bytes());
        b.extend(sched.util[1].to_le_bytes());
        b.truncate(size as usize);
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &b))?;
        Ok(0)
    }

    /// `sched_setaffinity`: the CPUs it may run on, kept to those the machine has; none of them is EINVAL.
    pub(crate) fn sched_setaffinity(&mut self, cx: &mut Cx<'_>, pid: i32, len: u64, at: u64) -> Result<u64, Errno> {
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len.min(8) as usize))?;
        let mut mask = [0u8; 8];
        mask[..bytes.len()].copy_from_slice(&bytes);
        let mask = u64::from_le_bytes(mask) & all_cpus(self.k.host.cpus());
        if mask == 0 {
            return Err(EINVAL);
        }
        lock(&self.sched_target(pid)?.state).sched.affinity = mask;
        Ok(0)
    }

    /// `sched_getaffinity`: the mask, in a buffer that must hold the machine's CPUs in whole longs. Gives its size.
    pub(crate) fn sched_getaffinity(&mut self, cx: &mut Cx<'_>, pid: i32, len: u64, at: u64) -> Result<u64, Errno> {
        if len * 8 < self.k.host.cpus().min(64) as u64 || !len.is_multiple_of(8) {
            return Err(EINVAL);
        }
        let mask = lock(&self.sched_target(pid)?.state).sched.affinity;
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &mask.to_le_bytes()))?;
        Ok(8)
    }

    /// `getcpu`: a CPU it may run on, the same one each time for a thread, spread across them by thread ID, so that
    /// per-CPU caches (glibc's malloc arenas, rseq users) spread as they would; and node 0.
    pub(crate) fn getcpu(&mut self, cx: &mut Cx<'_>, cpu_at: u64, node_at: u64) -> Result<u64, Errno> {
        let mask = lock(&self.process.state).sched.affinity;
        let nth = self.tid as u32 % mask.count_ones().max(1);
        let cpu = (0..64).filter(|i| mask & 1 << i != 0).nth(nth as usize).unwrap_or(0);
        self.mem(cx, true, |t| {
            if cpu_at != 0 {
                user::write_u32(&mut t.space(), cpu_at, cpu)?;
            }
            if node_at != 0 {
                user::write_u32(&mut t.space(), node_at, 0)?;
            }
            Ok(())
        })?;
        Ok(0)
    }

    /// `ioprio_get`: the highest I/O priority among the processes meant. A process that never set one has 0 (no
    /// class).
    pub(crate) fn ioprio_get(&mut self, which: u64, who: i32) -> Result<u64, Errno> {
        if !(IOPRIO_WHO_PROCESS..=IOPRIO_WHO_USER).contains(&which) {
            return Err(EINVAL);
        }
        // Lower classes rank higher, but no class at all ranks below every one.
        let rank = |prio: u16| if prio >> IOPRIO_CLASS_SHIFT == 0 { u16::MAX } else { prio };
        let best = self.whose(which - 1, who)?.iter().map(|p| lock(&p.state).sched.ioprio).min_by_key(|p| rank(*p));
        Ok(u64::from(best.unwrap_or(0)))
    }

    /// `ioprio_set`: a class and a level within it (0 to 7 for real time and best effort).
    pub(crate) fn ioprio_set(&mut self, which: u64, who: i32, prio: u64) -> Result<u64, Errno> {
        let prio = u16::try_from(prio).map_err(|_| EINVAL)?;
        let (class, level) = (prio >> IOPRIO_CLASS_SHIFT, prio & ((1 << IOPRIO_CLASS_SHIFT) - 1));
        match class {
            0 | 3 => {}
            1 | 2 if level < 8 => {}
            _ => return Err(EINVAL),
        }
        if !(IOPRIO_WHO_PROCESS..=IOPRIO_WHO_USER).contains(&which) {
            return Err(EINVAL);
        }
        for process in self.whose(which - 1, who)? {
            lock(&process.state).sched.ioprio = prio;
        }
        Ok(0)
    }
}
