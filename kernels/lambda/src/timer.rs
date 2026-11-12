//! Timers that send signals: `alarm` and the interval timers (`setitimer`, `getitimer`), and POSIX timers
//! (`timer_create`, `timer_settime`, `timer_gettime`, `timer_getoverrun`, `timer_delete`).
//!
//! A timer on a clock that runs by itself (real time, monotonic, boot time, TAI) is kept by the machine's timer thread
//! (`kernels_core::timers`), which sends its signal when it's due, whatever the program is doing. One on CPU time
//! (`ITIMER_VIRTUAL`, `ITIMER_PROF`, a CPU-time clock) is checked each time one of its process's threads stops, which
//! is at least once a time slice while it runs. A POSIX timer's signal is queued once: expiries while it's still
//! pending are counted as its overrun, which the signal reports when it's taken.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::timers::TimerId;
use kernels_core::user;

use crate::Kernel;
use crate::abi::*;
use crate::process::{ProcState, Task};
use crate::signal::{Info, SI_TIMER};

const SIGALRM: i32 = 14;
const SIGVTALRM: i32 = 26;
const SIGPROF: i32 = 27;
const ITIMER_REAL: usize = 0;
const ITIMER_VIRTUAL: usize = 1;
const ITIMER_PROF: usize = 2;
// How a POSIX timer says it expired: a signal, nothing, or a signal to one thread (`SIGEV_THREAD`, glibc's, is that).
const SIGEV_SIGNAL: i32 = 0;
const SIGEV_NONE: i32 = 1;
const SIGEV_THREAD_ID: i32 = 4;
/// The most a timer's overrun counts, as Linux's `DELAYTIMER_MAX` (adding to it saturates there).
const MOST_OVERRUN: i32 = i32::MAX;
/// TAI's lead on real time.
const TAI_OFFSET: u64 = 37_000_000_000;

/// What a CPU-time timer counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpuClock {
    /// The process's threads together: user time only, or user and system.
    Process { user_only: bool },
    Thread(i32),
}

/// The clock a timer runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clock {
    /// Real time, ahead by this many nanoseconds (TAI's 37 s, or none).
    Wall(u64),
    Monotonic,
    Cpu(CpuClock),
}

/// When an armed timer next expires.
#[derive(Debug, Clone, Copy)]
enum Due {
    /// At this instant, by the timer thread's timer.
    At(Instant, TimerId),
    /// When its CPU-time clock reaches this.
    Cpu(u64),
}

#[derive(Debug, Clone, Copy)]
struct Armed {
    due: Due,
    /// Nanoseconds between expiries; 0 for once.
    interval: u64,
    /// Which arming this is, so a timer thread's callback for an earlier one does nothing.
    generation: u64,
}

#[derive(Debug, Clone, Copy)]
enum Notify {
    Nothing,
    /// A signal and its value, to the process or to one thread.
    Signal { signal: i32, value: u64, tid: Option<i32> },
}

#[derive(Debug)]
struct Posix {
    clock: Clock,
    notify: Notify,
    armed: Option<Armed>,
    /// Expiries since its signal was queued, beyond the first; and as of the last one taken.
    overrun: i32,
    last_overrun: i32,
    /// Its signal is queued and not yet taken.
    queued: bool,
}

/// Which of a process's timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Itimer(usize),
    Posix(i32),
}

/// A process's timers. A child starts with none; `execve` keeps the interval timers and deletes the POSIX ones.
#[derive(Debug, Default)]
pub(crate) struct ProcTimers {
    itimers: [Option<Armed>; 3],
    posix: BTreeMap<i32, Posix>,
    next_id: i32,
    generation: u64,
}

impl ProcTimers {
    fn armed(&mut self, which: Which) -> Option<&mut Option<Armed>> {
        match which {
            Which::Itimer(n) => self.itimers.get_mut(n),
            Which::Posix(id) => self.posix.get_mut(&id).map(|p| &mut p.armed),
        }
    }

    fn clock(&self, which: Which) -> Option<Clock> {
        match which {
            Which::Itimer(ITIMER_REAL) => Some(Clock::Wall(0)),
            Which::Itimer(ITIMER_VIRTUAL) => Some(Clock::Cpu(CpuClock::Process { user_only: true })),
            Which::Itimer(ITIMER_PROF) => Some(Clock::Cpu(CpuClock::Process { user_only: false })),
            Which::Itimer(_) => None,
            Which::Posix(id) => self.posix.get(&id).map(|p| p.clock),
        }
    }

    /// Whether any timer runs on CPU time, for the check at each stop.
    fn any_cpu(&self) -> bool {
        self.itimers.iter().chain(self.posix.values().map(|p| &p.armed)).any(|a| matches!(a, Some(Armed { due: Due::Cpu(_), .. })))
    }

    /// Every armed timer the timer thread keeps, to cancel.
    fn thread_timers(&self) -> Vec<TimerId> {
        self.itimers
            .iter()
            .chain(self.posix.values().map(|p| &p.armed))
            .filter_map(|a| match a {
                Some(Armed { due: Due::At(_, id), .. }) => Some(*id),
                _ => None,
            })
            .collect()
    }
}

/// A timer's signal has been taken: it says how many expiries it stands for, and the timer may queue another.
pub(crate) fn delivered(state: &mut ProcState, info: &mut Info) {
    if info.code != SI_TIMER {
        return;
    }
    if let Some(timer) = state.timers.posix.get_mut(&info.timer) {
        info.overrun = timer.overrun;
        timer.last_overrun = timer.overrun;
        timer.overrun = 0;
        timer.queued = false;
    }
}

/// The process is ending: its timers go with it.
pub(crate) fn cancel_all(k: &Kernel, state: &mut ProcState) {
    for id in state.timers.thread_timers() {
        k.timers.cancel(id);
    }
    state.timers = ProcTimers::default();
}

/// `execve`: the POSIX timers are deleted, the interval timers kept.
pub(crate) fn on_exec(k: &Kernel, state: &mut ProcState) {
    let itimers = state.timers.itimers;
    for (_, timer) in std::mem::take(&mut state.timers.posix) {
        if let Some(Armed { due: Due::At(_, id), .. }) = timer.armed {
            k.timers.cancel(id);
        }
    }
    crate::signal::purge(state, |info| info.code == SI_TIMER);
    state.timers.itimers = itimers;
}

/// What an expiry does: the signal to send, if any.
type Send = Option<(i32, Option<i32>, Info)>;

/// `which` expired: count it, arm its next expiry if it has an interval, and say what to send.
fn expire(timers: &mut ProcTimers, which: Which, missed: i32) -> Send {
    match which {
        Which::Itimer(n) => Some(([SIGALRM, SIGVTALRM, SIGPROF][n], None, Info { code: 0x80, ..Info::default() })),
        Which::Posix(id) => {
            let timer = timers.posix.get_mut(&id)?;
            let Notify::Signal { signal, value, tid } = timer.notify else { return None };
            if timer.queued {
                timer.overrun = timer.overrun.saturating_add(1 + missed);
                return None;
            }
            timer.queued = true;
            timer.overrun = timer.overrun.saturating_add(missed);
            Some((signal, tid, Info { code: SI_TIMER, timer: id, value, ..Info::default() }))
        }
    }
}

/// Arms the timer thread's timer for `which` of process `pid`, at `when`.
fn arm_at(k: &Arc<Kernel>, pid: i32, which: Which, generation: u64, when: Instant) -> TimerId {
    let kernel = Arc::downgrade(k);
    k.timers.at(when, move || {
        if let Some(k) = kernel.upgrade() {
            fired(&k, pid, which, generation);
        }
    })
}

/// The timer thread's timer for `which` of process `pid` is due.
fn fired(k: &Arc<Kernel>, pid: i32, which: Which, generation: u64) {
    let Some(process) = k.process(pid) else { return };
    let send = {
        let mut state = lock(&process.state);
        let timers = &mut state.timers;
        let Some(slot) = timers.armed(which) else { return };
        let Some(Armed { due: Due::At(when, _), interval, generation: g }) = *slot else { return };
        if g != generation {
            return;
        }
        let mut missed = 0;
        if interval == 0 {
            *slot = None;
        } else {
            // The next expiry after now, counting those already gone by as missed.
            let step = Duration::from_nanos(interval);
            let mut next = when + step;
            let now = Instant::now();
            if next <= now {
                let behind = ((now - next).as_nanos() / u128::from(interval)) as u32 + 1;
                next += step * behind;
                missed = behind.min(MOST_OVERRUN as u32) as i32;
            }
            let id = arm_at(k, pid, which, generation, next);
            *slot = Some(Armed { due: Due::At(next, id), interval, generation });
        }
        expire(timers, which, missed)
    };
    if let Some((signal, tid, info)) = send {
        k.send(&process, tid, signal, info);
    }
}

/// An `itimerval` (two `timeval`s) or `itimerspec` (two `timespec`s): interval and value, in nanoseconds.
fn decode_pair(words: &[u64], micro: bool) -> Result<(u64, u64), Errno> {
    let (unit, most) = if micro { (1000, 1_000_000) } else { (1, 1_000_000_000) };
    if words.iter().any(|w| (*w as i64) < 0) || words[1] >= most || words[3] >= most {
        return Err(EINVAL);
    }
    let ns = |sec: u64, frac: u64| sec.saturating_mul(1_000_000_000).saturating_add(frac * unit);
    Ok((ns(words[0], words[1]), ns(words[2], words[3])))
}

fn encode_pair((interval, value): (u64, u64), micro: bool) -> Vec<u8> {
    let unit = if micro { 1000 } else { 1 };
    [interval / 1_000_000_000, interval % 1_000_000_000 / unit, value / 1_000_000_000, value % 1_000_000_000 / unit].iter().flat_map(|w| w.to_le_bytes()).collect()
}

impl Task {
    /// A CPU-time clock's reading now.
    fn cpu_now(&self, clock: CpuClock) -> u64 {
        match clock {
            CpuClock::Process { user_only } => {
                let (user, system) = self.process.cpu_times();
                if user_only { user } else { user + system }
            }
            CpuClock::Thread(tid) => {
                let state = lock(&self.process.state);
                state.threads.get(&tid).map_or(0, |live| {
                    let (user, system) = live.times.get();
                    user + system
                })
            }
        }
    }

    /// A clock's reading now, for timers.
    fn timer_clock_now(&self, clock: Clock) -> u64 {
        match clock {
            Clock::Wall(ahead) => self.k.host.wall() + ahead,
            Clock::Monotonic => self.k.host.monotonic(),
            Clock::Cpu(cpu) => self.cpu_now(cpu),
        }
    }

    /// Arms `which` to expire `value` nanoseconds from now (or at `value` on its clock, if `absolute`), then every
    /// `interval`; a `value` of 0 disarms it. Gives what was left of it before, and its interval.
    fn set_timer(&mut self, which: Which, value: u64, interval: u64, absolute: bool) -> Result<(u64, u64), Errno> {
        let clock = lock(&self.process.state).timers.clock(which).ok_or(EINVAL)?;
        let now = self.timer_clock_now(clock);
        let instant = Instant::now();
        let pid = self.process.pid;
        let mut state = lock(&self.process.state);
        let timers = &mut state.timers;
        timers.generation += 1;
        let generation = timers.generation;
        let slot = timers.armed(which).ok_or(EINVAL)?;
        let old = slot.take().map_or((0, 0), |armed| {
            let left = match armed.due {
                Due::At(when, id) => {
                    self.k.timers.cancel(id);
                    when.saturating_duration_since(instant).as_nanos() as u64
                }
                Due::Cpu(at) => at.saturating_sub(now),
            };
            (left, armed.interval)
        });
        if value != 0 {
            let wait = if absolute { value.saturating_sub(now) } else { value };
            let due = match clock {
                Clock::Cpu(_) => Due::Cpu(now.saturating_add(wait)),
                _ => {
                    let when = instant + Duration::from_nanos(wait);
                    Due::At(when, arm_at(&self.k, pid, which, generation, when))
                }
            };
            *slot = Some(Armed { due, interval, generation });
        }
        if let Which::Posix(id) = which
            && let Some(timer) = timers.posix.get_mut(&id)
        {
            timer.overrun = 0;
        }
        self.process.cpu_timers.store(timers.any_cpu(), Ordering::Release);
        Ok(old)
    }

    /// What's left of `which`, and its interval.
    fn get_timer(&self, which: Which) -> Result<(u64, u64), Errno> {
        let clock = lock(&self.process.state).timers.clock(which).ok_or(EINVAL)?;
        let now = self.timer_clock_now(clock);
        let mut state = lock(&self.process.state);
        let slot = state.timers.armed(which).ok_or(EINVAL)?;
        Ok(slot.map_or((0, 0), |armed| {
            let left = match armed.due {
                Due::At(when, _) => when.saturating_duration_since(Instant::now()).as_nanos() as u64,
                Due::Cpu(at) => at.saturating_sub(now),
            };
            (left.max(1), armed.interval)
        }))
    }

    /// At a stop: sends the signals of CPU-time timers that are due, if the process has any.
    pub(crate) fn check_cpu_timers(&mut self) {
        if !self.process.cpu_timers.load(Ordering::Acquire) {
            return;
        }
        let (user, system) = self.process.cpu_times();
        let mut sends = Vec::new();
        {
            let mut state = lock(&self.process.state);
            let thread_times: BTreeMap<i32, u64> = state.threads.iter().map(|(tid, live)| (*tid, live.times.get().0 + live.times.get().1)).collect();
            let timers = &mut state.timers;
            let due: Vec<(Which, Clock)> = (0..3).map(Which::Itimer).chain(timers.posix.keys().map(|id| Which::Posix(*id))).filter_map(|w| Some((w, timers.clock(w)?))).collect();
            for (which, clock) in due {
                let Clock::Cpu(cpu) = clock else { continue };
                let now = match cpu {
                    CpuClock::Process { user_only: true } => user,
                    CpuClock::Process { user_only: false } => user + system,
                    CpuClock::Thread(tid) => thread_times.get(&tid).copied().unwrap_or(0),
                };
                let Some(slot) = timers.armed(which) else { continue };
                let Some(Armed { due: Due::Cpu(at), interval, generation }) = *slot else { continue };
                if now < at {
                    continue;
                }
                let missed = match (now - at).checked_div(interval) {
                    None => {
                        *slot = None;
                        0
                    }
                    Some(behind) => {
                        *slot = Some(Armed { due: Due::Cpu(at + (behind + 1) * interval), interval, generation });
                        behind.min(MOST_OVERRUN as u64) as i32
                    }
                };
                sends.extend(expire(timers, which, missed));
            }
            self.process.cpu_timers.store(timers.any_cpu(), Ordering::Release);
        }
        for (signal, tid, info) in sends {
            self.k.send(&self.process, tid, signal, info);
        }
    }

    pub(crate) fn setitimer(&mut self, cx: &mut Cx<'_>, which: u64, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        let which = usize::try_from(which).ok().filter(|w| *w < 3).ok_or(EINVAL)?;
        // No new value is a zero one: disarmed.
        let (interval, value) = match new_at {
            0 => (0, 0),
            at => decode_pair(&self.mem(cx, false, |t| (0..4).map(|i| user::read_u64(&t.space(), at + i * 8)).collect::<Result<Vec<u64>, _>>())?, true)?,
        };
        let old = self.set_timer(Which::Itimer(which), value, interval, false)?;
        if old_at != 0 {
            let (left, interval) = old;
            self.mem(cx, true, |t| user::write(&mut t.space(), old_at, &encode_pair((interval, left.div_ceil(1000) * 1000), true)))?;
        }
        Ok(0)
    }

    pub(crate) fn getitimer(&mut self, cx: &mut Cx<'_>, which: u64, at: u64) -> Result<u64, Errno> {
        let which = usize::try_from(which).ok().filter(|w| *w < 3).ok_or(EINVAL)?;
        let (left, interval) = self.get_timer(Which::Itimer(which))?;
        let left = if left == 0 { 0 } else { left.div_ceil(1000) * 1000 };
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &encode_pair((interval, left), true)))?;
        Ok(0)
    }

    /// x86-64's `alarm`: SIGALRM in `seconds` (0: none), giving the seconds the last one had left, rounded.
    pub(crate) fn alarm(&mut self, seconds: u64) -> Result<u64, Errno> {
        let (left, _) = self.set_timer(Which::Itimer(ITIMER_REAL), (seconds as u32 as u64) * 1_000_000_000, 0, false)?;
        Ok(if left == 0 { 0 } else { ((left + 500_000_000) / 1_000_000_000).max(1) })
    }

    /// `timer_create`: a timer on `clock`, disarmed, saying it expired as `sevp_at`'s `sigevent` asks (none:
    /// SIGALRM, with the timer's ID as its value). Writes its ID at `id_at`.
    pub(crate) fn timer_create(&mut self, cx: &mut Cx<'_>, clock: u64, sevp_at: u64, id_at: u64) -> Result<u64, Errno> {
        let clock = match clock as i32 {
            0 | 8 => Clock::Wall(0),
            11 => Clock::Wall(TAI_OFFSET),
            1 | 7 | 9 => Clock::Monotonic,
            2 => Clock::Cpu(CpuClock::Process { user_only: false }),
            3 => Clock::Cpu(CpuClock::Thread(self.tid)),
            c @ ..0 => {
                // A CPU-time clock by ID, for this process or one of its threads.
                let (id, thread, kind) = (!(c >> 3), c & 4 != 0, c & 3);
                match (thread, id) {
                    _ if kind == 3 => return Err(EINVAL),
                    (true, tid) if tid == 0 || lock(&self.process.state).threads.contains_key(&tid) => Clock::Cpu(CpuClock::Thread(if tid == 0 { self.tid } else { tid })),
                    (false, pid) if pid == 0 || pid == self.process.pid => Clock::Cpu(CpuClock::Process { user_only: kind == 1 }),
                    (false, pid) if self.k.process(pid).is_some() => return Err(EPERM),
                    _ => return Err(EINVAL),
                }
            }
            _ => return Err(EINVAL),
        };
        let mut state = lock(&self.process.state);
        let id = state.timers.next_id;
        drop(state);
        let notify = if sevp_at == 0 {
            Notify::Signal { signal: SIGALRM, value: id as u64, tid: None }
        } else {
            let b = self.mem(cx, false, |t| user::read_vec(&t.space(), sevp_at, 20))?;
            let i32_at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
            let (value, signal, how, tid) = (u64::from_le_bytes(b[0..8].try_into().unwrap_or_default()), i32_at(8), i32_at(12), i32_at(16));
            let valid = (1..=64).contains(&signal);
            match how {
                SIGEV_NONE => Notify::Nothing,
                SIGEV_SIGNAL if valid => Notify::Signal { signal, value, tid: None },
                SIGEV_THREAD_ID if valid && lock(&self.process.state).threads.contains_key(&tid) => Notify::Signal { signal, value, tid: Some(tid) },
                _ => return Err(EINVAL),
            }
        };
        self.mem(cx, true, |t| user::write_u32(&mut t.space(), id_at, id as u32))?;
        state = lock(&self.process.state);
        state.timers.next_id += 1;
        state.timers.posix.insert(id, Posix { clock, notify, armed: None, overrun: 0, last_overrun: 0, queued: false });
        Ok(0)
    }

    fn posix_timer(&self, id: i32) -> Result<Which, Errno> {
        if lock(&self.process.state).timers.posix.contains_key(&id) { Ok(Which::Posix(id)) } else { Err(EINVAL) }
    }

    /// `timer_settime`: arms or disarms it, `TIMER_ABSTIME` for a time on its clock rather than from now.
    pub(crate) fn timer_settime(&mut self, cx: &mut Cx<'_>, id: i32, flags: u64, new_at: u64, old_at: u64) -> Result<u64, Errno> {
        if flags & !TIMER_ABSTIME != 0 || new_at == 0 {
            return Err(EINVAL);
        }
        let which = self.posix_timer(id)?;
        let words = self.mem(cx, false, |t| (0..4).map(|i| user::read_u64(&t.space(), new_at + i * 8)).collect::<Result<Vec<u64>, _>>())?;
        let (interval, value) = decode_pair(&words, false)?;
        let old = self.set_timer(which, value, interval, flags & TIMER_ABSTIME != 0)?;
        if old_at != 0 {
            self.mem(cx, true, |t| user::write(&mut t.space(), old_at, &encode_pair((old.1, old.0), false)))?;
        }
        Ok(0)
    }

    pub(crate) fn timer_gettime(&mut self, cx: &mut Cx<'_>, id: i32, at: u64) -> Result<u64, Errno> {
        let (left, interval) = self.get_timer(self.posix_timer(id)?)?;
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &encode_pair((interval, left), false)))?;
        Ok(0)
    }

    pub(crate) fn timer_getoverrun(&mut self, id: i32) -> Result<u64, Errno> {
        let state = lock(&self.process.state);
        state.timers.posix.get(&id).map(|t| t.last_overrun as u64).ok_or(EINVAL)
    }

    /// `timer_delete`: disarmed and gone, with its signal if it's still queued.
    pub(crate) fn timer_delete(&mut self, id: i32) -> Result<u64, Errno> {
        let mut state = lock(&self.process.state);
        let timer = state.timers.posix.remove(&id).ok_or(EINVAL)?;
        if let Some(Armed { due: Due::At(_, timer_id), .. }) = timer.armed {
            self.k.timers.cancel(timer_id);
        }
        crate::signal::purge(&mut state, |info| info.code == SI_TIMER && info.timer == id);
        self.process.cpu_timers.store(state.timers.any_cpu(), Ordering::Release);
        Ok(0)
    }
}
