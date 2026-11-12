//! Waiting on files: `poll`, `ppoll`, `select` and `pselect6`, and what `epoll` and the blocking reads of eventfds and
//! timerfds share with them. A file says what's ready on it (`ready`) and who to tell when that may change (its
//! watchers: see `kernels_core::wait`). A thread waiting on files is among the watchers of each while it waits, so
//! nothing is checked again until something says it changed, a timer it watches expires, or a signal comes.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Wake, Watchers, Woke};

use super::{Node, Open};
use crate::abi::*;
use crate::process::Task;

pub const POLLIN: u32 = 0x1;
pub const POLLOUT: u32 = 0x4;
pub const POLLERR: u32 = 0x8;
pub const POLLHUP: u32 = 0x10;
const POLLNVAL: u32 = 0x20;
const POLLRDNORM: u32 = 0x40;
const POLLWRNORM: u32 = 0x100;
const POLLRDHUP: u32 = 0x2000;
/// How deep epoll instances inside epoll instances are looked into.
const MOST_NESTED: usize = 4;

impl Task {
    /// What's ready on a file now, of `wanted` (hangup and error always count). `depth` is how deep in epoll
    /// instances this file is.
    pub(crate) fn ready(&self, open: &Open, wanted: u32, depth: usize) -> u32 {
        if let Node::Socket(s) = &open.node {
            let ready = s.socket.ready();
            let mut got = 0;
            if ready.read {
                got |= POLLIN | POLLRDNORM;
            }
            if ready.write {
                got |= POLLOUT | POLLWRNORM;
            }
            if ready.read_closed {
                got |= POLLRDHUP;
            }
            if ready.hangup {
                got |= POLLHUP;
            }
            if ready.error {
                got |= POLLERR;
            }
            return got & (wanted | POLLHUP | POLLERR);
        }
        let (readable, writable, hangup, error) = match &open.node {
            Node::Stdin => (self.k.host.wait_input(Some(Duration::ZERO)), false, false, false),
            Node::Pipe(pipe, false) => {
                let (readable, no_writers) = pipe.readable();
                (readable, false, no_writers, false)
            }
            Node::Pipe(pipe, true) => {
                let (room, no_readers) = pipe.writable();
                (false, room && !no_readers, false, no_readers)
            }
            Node::Tty(tty) => (tty.readable(), true, false, false),
            Node::PtyMaster(tty) => (tty.master_readable(), true, false, false),
            Node::Signalfd(mask) => (self.has_signal(*kernels_core::lock(mask)), false, false, false),
            Node::Eventfd(eventfd) => (eventfd.readable(), eventfd.writable(), false, false),
            Node::Netlink(netlink) => (netlink.readable(), true, false, false),
            Node::Timerfd(timer) => (timer.readable(self.timer_now(timer.realtime)), false, false, false),
            Node::Pidfd(process) => (kernels_core::lock(&process.state).zombie.is_some(), false, false, false),
            Node::Inotify(inotify) => (inotify.readable(), false, false, false),
            Node::Fanotify(group) => (group.readable(), false, false, false),
            Node::Mqueue(queue) => {
                let (readable, writable) = queue.ready();
                (readable, writable, false, false)
            }
            Node::Epoll(epoll) => {
                let any = depth < MOST_NESTED
                    && epoll.live().iter().any(|(interest, file)| Task::interest_report(interest, self.ready(file, u32::MAX, depth + 1), false) != 0);
                (any, false, false, false)
            }
            _ => (true, true, false, false),
        };
        let mut got = 0;
        if readable {
            got |= POLLIN | POLLRDNORM;
        }
        if writable {
            got |= POLLOUT | POLLWRNORM;
        }
        if hangup {
            got |= POLLHUP | POLLRDHUP;
        }
        if error {
            got |= POLLERR;
        }
        got & (wanted | POLLHUP | POLLERR)
    }

    /// Calls `f` with whoever a file tells when its readiness may change. Files that are always ready have none.
    pub(crate) fn with_watchers(&self, open: &Open, f: impl FnOnce(&Watchers)) {
        match &open.node {
            Node::Stdin => f(&self.k.stdin_watchers),
            Node::Pipe(pipe, _) => f(&pipe.watchers),
            Node::Tty(tty) | Node::PtyMaster(tty) => f(&tty.watchers),
            Node::Signalfd(_) => f(&self.process.signal_watchers),
            Node::Eventfd(eventfd) => f(&eventfd.watchers),
            Node::Timerfd(timer) => f(&timer.watchers),
            Node::Epoll(epoll) => f(&epoll.watchers),
            Node::Socket(s) => f(&s.watchers),
            Node::Netlink(netlink) => f(&netlink.watchers),
            Node::Pidfd(process) => f(&process.exit_watchers),
            Node::Mqueue(queue) => f(&queue.watchers),
            Node::Inotify(inotify) => f(&inotify.watchers),
            Node::Fanotify(group) => f(&group.watchers),
            _ => {}
        }
    }

    /// Now, on a timer's clock: the wall clock or the monotonic one, in nanoseconds.
    pub(crate) fn timer_now(&self, realtime: bool) -> u64 {
        if realtime { self.k.host.wall() } else { self.k.host.monotonic() }
    }

    /// How long until a file might become ready of its own accord: a timer's next expiry, or the soonest in an epoll.
    fn next_change(&self, open: &Open, depth: usize) -> Option<Duration> {
        match &open.node {
            Node::Timerfd(timer) => timer.until_next(self.timer_now(timer.realtime)),
            Node::Epoll(epoll) if depth < MOST_NESTED => epoll.live().iter().filter_map(|(_, file)| self.next_change(file, depth + 1)).min(),
            _ => None,
        }
    }

    /// Waits until `check` finds something, `timeout` passes, or a signal interrupts. While it waits this thread is
    /// among the watchers of every file in `watched`, so it's woken when one may have changed; a timer among them wakes
    /// it when it expires.
    pub(crate) fn wait_ready<R>(&self, timeout: Option<Duration>, watched: &[&Open], mut check: impl FnMut(&Task) -> Result<Option<R>, Errno>) -> Result<Option<R>, Errno> {
        let deadline = timeout.map(|t| Instant::now() + t);
        let me: Weak<dyn Wake> = Arc::downgrade(&self.waiter) as Weak<dyn Wake>;
        for open in watched {
            self.with_watchers(open, |w| w.add(me.clone()));
        }
        let result = loop {
            // Reset first: whatever changes after the check wakes the wait that follows it.
            self.waiter.reset();
            match check(self) {
                Ok(None) => {}
                other => break other,
            }
            let now = Instant::now();
            if deadline.is_some_and(|d| now >= d) {
                break Ok(None);
            }
            let soonest = watched.iter().filter_map(|open| self.next_change(open, 0)).min().map(|d| now + d);
            let wake_at = match (deadline, soonest) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if self.waiter.wait(wake_at) == Woke::Interrupted {
                break Err(EINTR);
            }
        };
        for open in watched {
            self.with_watchers(open, |w| w.remove(&me));
        }
        result
    }

    /// Runs a wait with the signal mask at `mask_at` (if it isn't 0), as `ppoll`, `pselect6` and `epoll_pwait` do:
    /// the old mask comes back after, or, if a signal ends the wait, after its handler, as with `rt_sigsuspend`.
    pub(crate) fn with_mask<R>(&mut self, cx: &mut Cx<'_>, mask_at: u64, wait: impl FnOnce(&mut Task, &mut Cx<'_>) -> Result<R, Errno>) -> Result<R, Errno> {
        if mask_at == 0 {
            return wait(self, cx);
        }
        let mask = self.mem(cx, false, |t| user::read_u64(&t.space(), mask_at))?;
        let old = self.mask();
        self.set_mask(mask);
        // A signal the new mask lets through may be waiting already.
        let result = if self.deliverable() != 0 { Err(EINTR) } else { wait(self, cx) };
        if result.as_ref().err() == Some(&EINTR) {
            self.suspend_mask = Some(old);
        } else {
            self.set_mask(old);
        }
        result
    }

    pub(crate) fn timespec_timeout(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Option<Duration>, Errno> {
        if at == 0 {
            return Ok(None);
        }
        let (sec, nsec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?;
        Ok(Some(Duration::new(sec, nsec.min(999_999_999) as u32)))
    }

    pub(crate) fn timeval_timeout(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<Option<Duration>, Errno> {
        if at == 0 {
            return Ok(None);
        }
        let (sec, usec) = self.mem(cx, false, |t| Ok((user::read_u64(&t.space(), at)?, user::read_u64(&t.space(), at + 8)?)))?;
        Ok(Some(Duration::new(sec, (usec.min(999_999) * 1000) as u32)))
    }

    pub(crate) fn ppoll(&mut self, cx: &mut Cx<'_>, at: u64, count: u64, timeout: Option<Duration>) -> Result<u64, Errno> {
        if count > 4096 {
            return Err(EINVAL);
        }
        let raw = self.mem(cx, false, |t| user::read_vec(&t.space(), at, count as usize * 8))?;
        let wanted: Vec<(i32, u32)> = raw.chunks(8).map(|c| (i32::from_le_bytes([c[0], c[1], c[2], c[3]]), u32::from(u16::from_le_bytes([c[4], c[5]])))).collect();
        let files: Vec<Option<Arc<Open>>> = wanted.iter().map(|&(fd, _)| if fd < 0 { None } else { self.file(fd).ok() }).collect();
        let watched: Vec<&Open> = files.iter().flatten().map(|open| &**open).collect();
        let found = self.wait_ready(timeout, &watched, |task| {
            let got: Vec<u32> = wanted
                .iter()
                .zip(&files)
                .map(|(&(fd, events), open)| match (fd, open) {
                    (..0, _) => 0,
                    (_, None) => POLLNVAL,
                    (_, Some(open)) => task.ready(open, events, 0),
                })
                .collect();
            Ok(got.iter().any(|&g| g != 0).then_some(got))
        })?;
        let got = found.unwrap_or_else(|| vec![0; wanted.len()]);
        let mut out = raw.clone();
        for (i, g) in got.iter().enumerate() {
            out[i * 8 + 6..i * 8 + 8].copy_from_slice(&(*g as u16).to_le_bytes());
        }
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &out))?;
        Ok(got.iter().filter(|&&g| g != 0).count() as u64)
    }

    /// `select` and `pselect6`, over read, write and exception sets of `nfds` descriptors.
    pub(crate) fn select(&mut self, cx: &mut Cx<'_>, nfds: u64, sets: [u64; 3], timeout: Option<Duration>) -> Result<u64, Errno> {
        if nfds > 1024 {
            return Err(EINVAL);
        }
        let bytes = nfds.div_ceil(64) as usize * 8;
        let mut wanted = [vec![], vec![], vec![]];
        for (i, &at) in sets.iter().enumerate() {
            if at != 0 {
                wanted[i] = self.mem(cx, false, |t| user::read_vec(&t.space(), at, bytes))?;
            }
        }
        let is_set = |set: &[u8], fd: usize| set.get(fd / 8).is_some_and(|b| b & (1 << (fd % 8)) != 0);
        let mut files = Vec::new();
        for fd in 0..nfds as usize {
            if wanted.iter().any(|s| is_set(s, fd)) {
                files.push((fd, self.file(fd as i32)?));
            }
        }
        let watched: Vec<&Open> = files.iter().map(|(_, open)| &**open).collect();
        let found = self.wait_ready(timeout, &watched, |task| {
            let mut got = [vec![0u8; bytes], vec![0u8; bytes], vec![0u8; bytes]];
            let mut count = 0;
            for (fd, open) in &files {
                // Readable for select includes hung up and in error; writable includes in error.
                for (i, event, counts) in [(0, POLLIN, POLLIN | POLLHUP | POLLERR), (1, POLLOUT, POLLOUT | POLLERR)] {
                    if is_set(&wanted[i], *fd) && task.ready(open, event, 0) & counts != 0 {
                        got[i][fd / 8] |= 1 << (fd % 8);
                        count += 1;
                    }
                }
            }
            Ok((count > 0).then_some((got, count)))
        })?;
        let (got, count) = found.unwrap_or_else(|| ([vec![0; bytes], vec![0; bytes], vec![0; bytes]], 0));
        for (i, &at) in sets.iter().enumerate() {
            if at != 0 {
                self.mem(cx, true, |t| user::write(&mut t.space(), at, &got[i]))?;
            }
        }
        Ok(count)
    }
}
