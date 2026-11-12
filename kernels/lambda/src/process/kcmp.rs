//! `kcmp`: whether two processes share something (an open file, their memory, their file table), for checkpointing
//! tools and systemd. Here each process has its own memory and file table, and its threads share them, so most
//! kinds compare the processes themselves.

use std::cmp::Ordering;
use std::sync::Arc;

use kernels_core::lock;

use super::Task;
use crate::abi::*;

const KCMP_FILE: u64 = 0;
const KCMP_VM: u64 = 1;
const KCMP_SYSVSEM: u64 = 6;

/// 0 for the same, else 1 or 2 for one before or after the other, as Linux orders them.
fn order(a: usize, b: usize) -> u64 {
    match a.cmp(&b) {
        Ordering::Equal => 0,
        Ordering::Less => 1,
        Ordering::Greater => 2,
    }
}

impl Task {
    pub(crate) fn kcmp(&mut self, pid1: i32, pid2: i32, kind: u64, idx1: u64, idx2: u64) -> Result<u64, Errno> {
        let (p1, p2) = (self.process_of(pid1).ok_or(ESRCH)?, self.process_of(pid2).ok_or(ESRCH)?);
        match kind {
            KCMP_FILE => {
                let f1 = lock(&p1.state).files.get(idx1 as i32).ok_or(EBADF)?;
                let f2 = lock(&p2.state).files.get(idx2 as i32).ok_or(EBADF)?;
                Ok(order(Arc::as_ptr(&f1) as usize, Arc::as_ptr(&f2) as usize))
            }
            KCMP_VM => {
                let s1 = lock(&p1.state).space.0;
                let s2 = lock(&p2.state).space.0;
                Ok(order(s1 as usize, s2 as usize))
            }
            // The file table, working directory, signal handlers, I/O context and SysV semaphore undo list: each
            // process's own.
            2..=KCMP_SYSVSEM => Ok(order(Arc::as_ptr(&p1) as usize, Arc::as_ptr(&p2) as usize)),
            _ => Err(EINVAL),
        }
    }
}
