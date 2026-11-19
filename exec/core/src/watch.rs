//! `CARBON_EXEC_WATCH=1`: CPUs that make no progress, reported.
//!
//! - One that stays in its program for `LONG`: a thread looks every second, says so, and kicks it out, as a preempt
//!   would; when it comes out, `Cpu::run` says how long it was in, why it came out, and where it was. One that won't
//!   come out for a kick is stuck in its backend.
//! - One that only ever comes out for kicks (time slices, preempts) for `LONG`, never for a trap or a fault: its
//!   program isn't getting anywhere, or isn't running at all. Said once a second while it lasts, with where it is
//!   and how many of its runs didn't go in at all.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::cpu::CpuShared;

/// How long in its program counts as long: nanoseconds.
pub(crate) const LONG: u64 = 3_000_000_000;

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CARBON_EXEC_WATCH").is_some())
}

/// Nanoseconds since the watch's epoch, never 0.
pub(crate) fn now() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
}

/// Starts the watching thread, if the watch is on.
pub(crate) fn start(cpus: Arc<Mutex<Vec<Arc<CpuShared>>>>) {
    if !on() {
        return;
    }
    eprintln!("exec watch: on (cpus in their program for {}s, or only out for kicks that long, are reported)", LONG / 1_000_000_000);
    let _ = std::thread::Builder::new().name("exec-watch".into()).spawn(move || {
        // Per CPU slot, the entry last reported, and how many kicks it's had since.
        let mut seen: Vec<(u64, u32)> = Vec::new();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let cpus = cpus.lock().unwrap_or_else(PoisonError::into_inner).clone();
            seen.resize(cpus.len(), (0, 0));
            for (index, cpu) in cpus.iter().enumerate() {
                let entered = cpu.entered.load(Ordering::Acquire);
                if entered == 0 || cpu.gone.load(Ordering::Acquire) || !cpu.running.load(Ordering::SeqCst) {
                    continue;
                }
                let stayed = now().saturating_sub(entered);
                if stayed < LONG {
                    continue;
                }
                let (last, kicks) = &mut seen[index];
                if *last != entered {
                    (*last, *kicks) = (entered, 0);
                }
                *kicks += 1;
                // Said the first time, then every tenth: a CPU that won't come out would fill the log.
                if *kicks == 1 || kicks.is_multiple_of(10) {
                    eprintln!(
                        "exec watch: cpu {index} ({}) has been in its program {:.1}s; kick {kicks}",
                        cpu.thread.get().map_or("?", String::as_str),
                        stayed as f64 / 1e9
                    );
                }
                (cpu.preempt)();
            }
        }
    });
}

/// One CPU's progress, as the watch sees it.
#[derive(Default)]
pub(crate) struct Progress {
    since: u64,
    runs: u64,
    skipped: u64,
    reported: u64,
}

/// It came out for something other than a kick.
pub(crate) fn progressed(cpu: &CpuShared) {
    let mut p = cpu.progress.lock().unwrap_or_else(PoisonError::into_inner);
    *p = Progress { since: now(), ..Progress::default() };
}

/// It came out for a kick, or (`skipped`) didn't go in at all because it had been asked to stop. `regs` says where it
/// is, only called when there's a report to make.
pub(crate) fn kicked(cpu: &CpuShared, index: usize, skipped: bool, regs: impl FnOnce() -> String) {
    let mut p = cpu.progress.lock().unwrap_or_else(PoisonError::into_inner);
    let t = now();
    if p.since == 0 {
        p.since = t;
    }
    p.runs += 1;
    p.skipped += u64::from(skipped);
    if t.saturating_sub(p.since) >= LONG && t.saturating_sub(p.reported) >= 1_000_000_000 {
        p.reported = t;
        eprintln!(
            "exec watch: cpu {index} ({}) has only come out for kicks for {:.1}s: {} runs, {} not going in; {}",
            cpu.thread.get().map_or("?", String::as_str),
            t.saturating_sub(p.since) as f64 / 1e9,
            p.runs,
            p.skipped,
            regs()
        );
    }
}
