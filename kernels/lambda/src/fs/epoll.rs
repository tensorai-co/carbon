//! `epoll`: a set of files to wait on, kept between waits. Each file in it has an [`Interest`]: the events wanted, the
//! program's data to give back, and whether the file has said anything since the last time it was reported. The
//! interest is among the file's watchers, so the file becoming ready marks it and wakes the epoll instance, which wakes
//! whoever is in `epoll_wait` on it (or polling it).
//!
//! Level-triggered interests are reported while they're ready; edge-triggered ones (`EPOLLET`) only once each time
//! their file says something; one-shot ones (`EPOLLONESHOT`) once, until `EPOLL_CTL_MOD` arms them again. An interest
//! holds its file weakly: closing the file's last descriptor takes it out, as on Linux.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use exec::Isa;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Wake, Watchers};

use super::{Node, Open};
use crate::abi::*;
use crate::process::Task;

pub const EPOLLERR: u32 = 0x8;
pub const EPOLLHUP: u32 = 0x10;
const EPOLLEXCLUSIVE: u32 = 1 << 28;
const EPOLLWAKEUP: u32 = 1 << 29;
const EPOLLONESHOT: u32 = 1 << 30;
const EPOLLET: u32 = 1 << 31;
/// The bits that say how to report, not what.
const CONTROL: u32 = EPOLLEXCLUSIVE | EPOLLWAKEUP | EPOLLONESHOT | EPOLLET;
const EPOLL_CTL_ADD: u64 = 1;
const EPOLL_CTL_DEL: u64 = 2;
const EPOLL_CTL_MOD: u64 = 3;

pub(crate) struct Interest {
    pub open: Weak<Open>,
    /// What's wanted, with the control bits; a one-shot interest that has fired keeps only those.
    pub events: Mutex<u32>,
    pub data: Mutex<u64>,
    /// The file has said something since this was last reported.
    pub edge: AtomicBool,
    epoll: Weak<Epoll>,
}

impl Wake for Interest {
    fn wake(&self) {
        self.edge.store(true, Ordering::Release);
        if let Some(epoll) = self.epoll.upgrade() {
            epoll.watchers.notify();
        }
    }
}

pub(crate) struct Epoll {
    /// By descriptor.
    pub interests: Mutex<BTreeMap<i32, Arc<Interest>>>,
    /// Whoever's waiting on this instance: threads in `epoll_wait`, and pollers of its descriptor.
    pub watchers: Watchers,
    me: Weak<Epoll>,
}

impl Epoll {
    pub fn new() -> Arc<Epoll> {
        Arc::new_cyclic(|me| Epoll { interests: Mutex::new(BTreeMap::new()), watchers: Watchers::default(), me: me.clone() })
    }

    /// The live interests, dropping those whose file has closed.
    pub fn live(&self) -> Vec<(Arc<Interest>, Arc<Open>)> {
        let mut interests = lock(&self.interests);
        interests.retain(|_, i| i.open.strong_count() > 0);
        interests.values().filter_map(|i| i.open.upgrade().map(|open| (i.clone(), open))).collect()
    }
}

impl Task {
    /// What an interest would report now, of `ready` (what its file has ready). Taking it clears its edge and, if
    /// it's one-shot, disarms it.
    pub(crate) fn interest_report(interest: &Interest, ready: u32, take: bool) -> u32 {
        let events = *lock(&interest.events);
        let report = ready & (events & !CONTROL | EPOLLERR | EPOLLHUP);
        if report == 0 || events & !CONTROL == 0 {
            return 0;
        }
        if events & EPOLLET != 0 && !interest.edge.load(Ordering::Acquire) {
            return 0;
        }
        if take {
            interest.edge.store(false, Ordering::Release);
            if events & EPOLLONESHOT != 0 {
                *lock(&interest.events) = events & CONTROL;
            }
        }
        report
    }

    pub(crate) fn epoll_create(&mut self, flags: u32) -> Result<u64, Errno> {
        if flags & !O_CLOEXEC != 0 {
            return Err(EINVAL);
        }
        let open = Open::new(Node::Epoll(Epoll::new()), O_RDWR, "anon_inode:[eventpoll]");
        lock(&self.process.state).files.add(open, flags & O_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    /// `struct epoll_event`'s size, and where its data is: packed on x86-64, padded on ARM64.
    fn event_layout(&self) -> (usize, usize) {
        if self.isa == Isa::X86_64 { (12, 4) } else { (16, 8) }
    }

    pub(crate) fn epoll_ctl(&mut self, cx: &mut Cx<'_>, epfd: i32, op: u64, fd: i32, at: u64) -> Result<u64, Errno> {
        let epoll_open = self.file(epfd)?;
        let Node::Epoll(epoll) = &epoll_open.node else { return Err(EINVAL) };
        let target = self.file(fd)?;
        if Arc::ptr_eq(&epoll_open, &target) {
            return Err(EINVAL);
        }
        // What can't be waited on (files on the disk, directories) can't be added, as Linux has it.
        if matches!(target.node, Node::File(_) | Node::Dir(_)) {
            return Err(EPERM);
        }
        let (events, data) = if op == EPOLL_CTL_DEL {
            (0, 0)
        } else {
            let (size, data_at) = self.event_layout();
            let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, size))?;
            (u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), u64::from_le_bytes(bytes[data_at..data_at + 8].try_into().unwrap_or_default()))
        };
        let existing = lock(&epoll.interests).get(&fd).filter(|i| i.open.upgrade().is_some_and(|o| Arc::ptr_eq(&o, &target))).cloned();
        match (op, existing) {
            (EPOLL_CTL_ADD, Some(_)) => Err(EEXIST),
            (EPOLL_CTL_ADD, None) => {
                let interest = Arc::new(Interest {
                    open: Arc::downgrade(&target),
                    events: Mutex::new(events),
                    data: Mutex::new(data),
                    // Reported once to begin with, if it's ready, even when edge-triggered.
                    edge: AtomicBool::new(true),
                    epoll: epoll.me.clone(),
                });
                let watcher: Weak<dyn Wake> = Arc::downgrade(&interest) as Weak<dyn Wake>;
                self.with_watchers(&target, |w| w.add(watcher));
                lock(&epoll.interests).insert(fd, interest);
                epoll.watchers.notify();
                Ok(0)
            }
            (EPOLL_CTL_MOD, Some(interest)) => {
                *lock(&interest.events) = events;
                *lock(&interest.data) = data;
                interest.edge.store(true, Ordering::Release);
                epoll.watchers.notify();
                Ok(0)
            }
            (EPOLL_CTL_DEL, Some(interest)) => {
                let watcher: Weak<dyn Wake> = Arc::downgrade(&interest) as Weak<dyn Wake>;
                self.with_watchers(&target, |w| w.remove(&watcher));
                lock(&epoll.interests).remove(&fd);
                Ok(0)
            }
            (EPOLL_CTL_MOD | EPOLL_CTL_DEL, None) => Err(ENOENT),
            _ => Err(EINVAL),
        }
    }

    /// `epoll_wait`, `epoll_pwait` and `epoll_pwait2`: up to `most` events into the array at `at`.
    pub(crate) fn epoll_wait(&mut self, cx: &mut Cx<'_>, epfd: i32, at: u64, most: i32, timeout: Option<Duration>) -> Result<u64, Errno> {
        if most <= 0 || most > 1 << 20 {
            return Err(EINVAL);
        }
        let epoll_open = self.file(epfd)?;
        let Node::Epoll(epoll) = &epoll_open.node else { return Err(EINVAL) };
        let epoll = epoll.clone();
        let found = self.wait_ready(timeout, &[&*epoll_open], |task| {
            let mut events = Vec::new();
            for (interest, open) in epoll.live() {
                if events.len() == most as usize {
                    break;
                }
                let ready = task.ready(&open, u32::MAX, 0);
                let report = Task::interest_report(&interest, ready, true);
                if report != 0 {
                    events.push((report, *lock(&interest.data)));
                }
            }
            Ok((!events.is_empty()).then_some(events))
        })?;
        let events = found.unwrap_or_default();
        let (size, data_at) = self.event_layout();
        let mut out = vec![0u8; events.len() * size];
        for (i, (report, data)) in events.iter().enumerate() {
            out[i * size..i * size + 4].copy_from_slice(&report.to_le_bytes());
            out[i * size + data_at..i * size + data_at + 8].copy_from_slice(&data.to_le_bytes());
        }
        if !out.is_empty() {
            self.mem(cx, true, |t| user::write(&mut t.space(), at, &out))?;
        }
        Ok(events.len() as u64)
    }
}
