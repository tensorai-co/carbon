//! SysV semaphores: `semget`, `semop`, `semtimedop` and `semctl`. A set's operations happen all at once or not at
//! all: a process waits until every one can (unless one says `IPC_NOWAIT`). With `SEM_UNDO`, what an operation did is
//! undone when the process ends, so a lock it held is let go.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use exec::Isa;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};

use super::*;
use crate::abi::*;

/// The most semaphores in a set, operations in one call, and sets, and the highest value and undo, as Linux's
/// defaults.
const SEMMSL: u64 = 32000;
const SEMOPM: u64 = 500;
const SEMMNI: usize = 32000;
const SEMVMX: i32 = 32767;
const SEMAEM: i32 = 32767;
const SEM_UNDO: i16 = 0x1000;
const GETPID: u64 = 11;
const GETVAL: u64 = 12;
const GETALL: u64 = 13;
const GETNCNT: u64 = 14;
const GETZCNT: u64 = 15;
const SETVAL: u64 = 16;
const SETALL: u64 = 17;
const SEM_STAT: u64 = 18;
const SEM_INFO: u64 = 19;
const SEM_STAT_ANY: u64 = 20;

struct Set {
    perm: Perm,
    /// Each semaphore's value, and who last changed it.
    sems: Vec<(i32, i32)>,
    /// Last operated on and changed, in seconds.
    times: [u64; 2],
    /// The threads waiting on it, by thread ID: on which semaphore, and for zero (else for more).
    blocked: Vec<(i32, u16, bool)>,
}

/// The machine's semaphore sets, by ID.
#[derive(Default)]
pub(crate) struct SemTable {
    sets: BTreeMap<i32, Set>,
    next: i32,
    waiting: Vec<Arc<Waiter>>,
}

/// One operation of `semop`: which semaphore, by how much (0: wait for zero), and its flags.
#[derive(Clone, Copy)]
struct Op {
    num: u16,
    by: i16,
    flags: i16,
}

/// Semaphore `num` of set `id`: its index, and the set.
fn one(table: &SemTable, id: i32, num: u64) -> Result<(usize, &Set), Errno> {
    let set = table.sets.get(&id).ok_or(EINVAL)?;
    let num = usize::try_from(num).ok().filter(|n| *n < set.sems.len()).ok_or(EINVAL)?;
    Ok((num, set))
}

/// A process has ended: what its `SEM_UNDO` operations did is undone.
pub(crate) fn apply_undo(ns: &crate::ns::IpcNs, pid: i32, undo: BTreeMap<(i32, u16), i32>) {
    let mut table = lock(&ns.sem);
    for ((id, num), adjust) in undo {
        if let Some(sem) = table.sets.get_mut(&id).and_then(|s| s.sems.get_mut(num as usize)) {
            *sem = ((sem.0 + adjust).clamp(0, SEMVMX), pid);
        }
    }
    wake_all(&mut table.waiting);
}

impl Task {
    pub(crate) fn semget(&mut self, key: i32, count: u64, flags: u64) -> Result<u64, Errno> {
        if count > SEMMSL {
            return Err(EINVAL);
        }
        let perm = Perm::new(self, key, flags);
        let now = seconds(self);
        let __ipc = self.ipc_ns();
        let mut table = lock(&__ipc.sem);
        if key != IPC_PRIVATE
            && let Some((id, set)) = table.sets.iter().find(|(_, s)| s.perm.key == key)
        {
            if flags & IPC_CREAT != 0 && flags & IPC_EXCL != 0 {
                return Err(EEXIST);
            }
            return if count as usize > set.sems.len() { Err(EINVAL) } else { Ok(*id as u64) };
        }
        if key != IPC_PRIVATE && flags & IPC_CREAT == 0 {
            return Err(ENOENT);
        }
        if count == 0 {
            return Err(EINVAL);
        }
        if table.sets.len() >= SEMMNI {
            return Err(ENOSPC);
        }
        let id = table.next;
        table.next += 1;
        table.sets.insert(id, Set { perm, sems: vec![(0, 0); count as usize], times: [0, now], blocked: Vec::new() });
        Ok(id as u64)
    }

    /// `semop` and `semtimedop`: every operation at once, waiting until they can all happen (until the timeout, then
    /// EAGAIN), unless the one that can't says `IPC_NOWAIT`.
    pub(crate) fn semtimedop(&mut self, cx: &mut Cx<'_>, id: i32, at: u64, count: u64, timeout_at: u64) -> Result<u64, Errno> {
        if count == 0 {
            return Err(EINVAL);
        }
        if count > SEMOPM {
            return Err(E2BIG);
        }
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, count as usize * 6))?;
        let ops: Vec<Op> = bytes
            .chunks(6)
            .map(|b| Op { num: u16::from_le_bytes([b[0], b[1]]), by: i16::from_le_bytes([b[2], b[3]]), flags: i16::from_le_bytes([b[4], b[5]]) })
            .collect();
        let deadline = self.timespec_timeout(cx, timeout_at)?.map(|d: Duration| Instant::now() + d);
        let pid = self.process.pid;
        let mut waited = false;
        loop {
            {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.sem);
                let now = seconds(self);
                let Some(set) = table.sets.get_mut(&id) else { return Err(if waited { EIDRM } else { EINVAL }) };
                set.blocked.retain(|(tid, _, _)| *tid != self.tid);
                if ops.iter().any(|op| op.num as usize >= set.sems.len()) {
                    return Err(EFBIG);
                }
                // Tried on a copy: all of them, or the first that would have to wait.
                let mut values: Vec<i32> = set.sems.iter().map(|s| s.0).collect();
                let mut blocked = None;
                for op in &ops {
                    let value = &mut values[op.num as usize];
                    match op.by {
                        0 if *value != 0 => blocked = Some((*op, true)),
                        by if by < 0 && *value + i32::from(by) < 0 => blocked = Some((*op, false)),
                        by if *value + i32::from(by) > SEMVMX => return Err(ERANGE),
                        by => *value += i32::from(by),
                    }
                    if blocked.is_some() {
                        break;
                    }
                }
                let Some((op, zero)) = blocked else {
                    // What SEM_UNDO keeps to undo, checked before anything changes.
                    let mut state = lock(&self.process.state);
                    let mut undo = state.sem_undo.clone();
                    for op in ops.iter().filter(|op| op.flags & SEM_UNDO != 0) {
                        let adjust = undo.entry((id, op.num)).or_insert(0);
                        *adjust -= i32::from(op.by);
                        if adjust.abs() > SEMAEM {
                            return Err(ERANGE);
                        }
                    }
                    undo.retain(|_, adjust| *adjust != 0);
                    state.sem_undo = undo;
                    drop(state);
                    for op in &ops {
                        set.sems[op.num as usize] = (values[op.num as usize], pid);
                    }
                    set.times[0] = now;
                    wake_all(&mut table.waiting);
                    return Ok(0);
                };
                if op.flags & IPC_NOWAIT as i16 != 0 {
                    return Err(EAGAIN);
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Err(EAGAIN);
                }
                set.blocked.push((self.tid, op.num, zero));
                self.waiter.reset();
                table.waiting.push(self.waiter.clone());
            }
            let woke = self.waiter.wait(deadline);
            waited = true;
            if woke != Woke::Woken {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.sem);
                leave(&mut table.waiting, &self.waiter);
                if let Some(set) = table.sets.get_mut(&id) {
                    set.blocked.retain(|(tid, _, _)| *tid != self.tid);
                }
                return Err(if woke == Woke::Interrupted { EINTR } else { EAGAIN });
            }
        }
    }

    /// Forgets what `SEM_UNDO` would undo of a semaphore given a value (`SETVAL`, `SETALL`), in every process.
    fn forget_undo(&self, id: i32, nums: std::ops::Range<u16>) {
        let ipc = self.ipc_ns();
        for process in lock(&self.k.processes).values() {
            let mut state = lock(&process.state);
            if std::sync::Arc::ptr_eq(&state.ns.ipc, &ipc) {
                state.sem_undo.retain(|(i, n), _| *i != id || !nums.contains(n));
            }
        }
    }

    /// `semctl`: one semaphore's value, pid or waiters, all their values, the set's details, or removing it. `arg` is
    /// `union semun`: a value or an address.
    pub(crate) fn semctl(&mut self, cx: &mut Cx<'_>, id: i32, num: u64, cmd: u64, arg: u64) -> Result<u64, Errno> {
        let cmd = cmd & !IPC_64;
        let now = seconds(self);
        match cmd {
            GETVAL => one(&lock(&self.ipc_ns().sem), id, num).map(|(n, s)| s.sems[n].0 as u64),
            GETPID => one(&lock(&self.ipc_ns().sem), id, num).map(|(n, s)| s.sems[n].1 as u64),
            GETNCNT | GETZCNT => one(&lock(&self.ipc_ns().sem), id, num).map(|(n, s)| s.blocked.iter().filter(|(_, b, zero)| *b as usize == n && *zero == (cmd == GETZCNT)).count() as u64),
            GETALL => {
                let values: Vec<u8> = lock(&self.ipc_ns().sem).sets.get(&id).ok_or(EINVAL)?.sems.iter().flat_map(|s| (s.0 as u16).to_le_bytes()).collect();
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &values))?;
                Ok(0)
            }
            SETVAL => {
                let value = arg as i32;
                if !(0..=SEMVMX).contains(&value) {
                    return Err(ERANGE);
                }
                {
                    let __ipc = self.ipc_ns();
                    let mut table = lock(&__ipc.sem);
                    let n = one(&table, id, num)?.0;
                    let set = table.sets.get_mut(&id).ok_or(EINVAL)?;
                    set.sems[n] = (value, self.process.pid);
                    set.times[1] = now;
                    wake_all(&mut table.waiting);
                }
                self.forget_undo(id, num as u16..num as u16 + 1);
                Ok(0)
            }
            SETALL => {
                let count = lock(&self.ipc_ns().sem).sets.get(&id).ok_or(EINVAL)?.sems.len();
                let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, count * 2))?;
                let values: Vec<i32> = bytes.chunks(2).map(|b| i32::from(u16::from_le_bytes([b[0], b[1]]))).collect();
                if values.iter().any(|v| *v > SEMVMX) {
                    return Err(ERANGE);
                }
                {
                    let __ipc = self.ipc_ns();
                    let mut table = lock(&__ipc.sem);
                    let set = table.sets.get_mut(&id).ok_or(EINVAL)?;
                    for (sem, value) in set.sems.iter_mut().zip(values) {
                        *sem = (value, self.process.pid);
                    }
                    set.times[1] = now;
                    wake_all(&mut table.waiting);
                }
                self.forget_undo(id, 0..count as u16);
                Ok(0)
            }
            IPC_INFO | SEM_INFO => {
                let (highest, bytes) = {
                    let __ipc = self.ipc_ns();
                    let table = lock(&__ipc.sem);
                    let highest = table.sets.keys().next_back().copied().unwrap_or(0);
                    let total: usize = table.sets.values().map(|s| s.sems.len()).sum();
                    // struct seminfo: semmap, semmni, semmns, semmnu, semmsl, semopm, semume, semusz, semvmx, semaem;
                    // SEM_INFO puts the sets in use in semusz and the semaphores in semaem.
                    let (usz, aem) = if cmd == IPC_INFO { (20, SEMAEM) } else { (table.sets.len() as i32, total as i32) };
                    let words = [SEMMNI as i32, SEMMNI as i32, (SEMMNI as u64 * SEMMSL).min(i32::MAX as u64) as i32, SEMMNI as i32, SEMMSL as i32, SEMOPM as i32, SEMOPM as i32, usz, SEMVMX, aem];
                    (highest, words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>())
                };
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &bytes))?;
                Ok(highest as u64)
            }
            IPC_STAT | SEM_STAT | SEM_STAT_ANY => {
                let bytes = {
                    let __ipc = self.ipc_ns();
                    let table = lock(&__ipc.sem);
                    let s = table.sets.get(&id).ok_or(EINVAL)?;
                    // struct semid64_ds: ipc64_perm, then the times and the count. x86-64 has a word after each time.
                    let mut b = s.perm.encode().to_vec();
                    let words: Vec<u64> = if self.isa == Isa::Arm64 { vec![s.times[0], s.times[1], s.sems.len() as u64, 0, 0] } else { vec![s.times[0], 0, s.times[1], 0, s.sems.len() as u64, 0, 0] };
                    b.extend(words.iter().flat_map(|w| w.to_le_bytes()));
                    b
                };
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &bytes))?;
                Ok(if cmd == IPC_STAT { 0 } else { id as u64 })
            }
            IPC_SET => {
                let b = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, 48))?;
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.sem);
                let set = table.sets.get_mut(&id).ok_or(EINVAL)?;
                if !set.perm.may_change(self) {
                    return Err(EPERM);
                }
                set.perm.set(&b);
                set.times[1] = now;
                Ok(0)
            }
            IPC_RMID => {
                let __ipc = self.ipc_ns();
                let mut table = lock(&__ipc.sem);
                if !table.sets.get(&id).ok_or(EINVAL)?.perm.may_change(self) {
                    return Err(EPERM);
                }
                table.sets.remove(&id);
                wake_all(&mut table.waiting);
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }
}
