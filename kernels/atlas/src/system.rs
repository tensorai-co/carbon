//! The machine as Windows describes it: which Windows it is (one version, the same in the registry, the PEB, the
//! shared data page and every call that asks), and `KUSER_SHARED_DATA`, the read-only page at `0x7ffe0000` in every
//! process that programs and runtimes read the time, the version and the processor's features from with no call.
//!
//! There's a page for each ISA, as an ARM64 program and an x86-64 one see different processors, with the same clocks.
//! The timer thread brings the clocks up to date 64 times a second, as Windows' clock interrupt does, writing each
//! `KSYSTEM_TIME` the way readers expect (`High2Time`, then `LowPart`, then `High1Time`).

use std::sync::atomic::{Ordering, fence};
use std::sync::Weak;
use std::time::{Duration, Instant};

use exec::{Frame, Isa};
use kernels_core::lock;
use kernels_core::thread::Cx;

use crate::Kernel;

/// Windows 10 22H2.
pub const MAJOR: u32 = 10;
pub const MINOR: u32 = 0;
pub const BUILD: u32 = 19045;
/// What `GetVersionEx` says to a program whose manifest doesn't say it knows Windows 10: Windows 8, as Windows does.
pub const LEGACY: (u32, u32, u32) = (6, 2, 9200);
/// The manifest's `supportedOS` for Windows 10 and 11.
pub const WINDOWS_10_GUID: &str = "8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a";

/// Where the shared data page is, in every process.
pub const SHARED_DATA: u64 = 0x7ffe_0000;
/// Windows' clock interrupt: 15.625 ms, in 100 ns units.
const TICK: u64 = 156_250;

/// `PROCESSOR_ARCHITECTURE_*`.
pub fn architecture(isa: Isa) -> u16 {
    match isa {
        Isa::Arm64 => 12,
        Isa::X86_64 => 9,
        Isa::X86_32 => 0,
    }
}

/// `IMAGE_FILE_MACHINE_*`.
pub fn machine(isa: Isa) -> u16 {
    match isa {
        Isa::Arm64 => 0xaa64,
        Isa::X86_64 => 0x8664,
        Isa::X86_32 => 0x14c,
    }
}

/// `IsProcessorFeaturePresent`'s features, as each ISA's programs see the CPU: x86-64 as exec's translator gives it
/// (SSE2, no more), ARM64 as Apple silicon has it.
pub fn features(isa: Isa) -> [u8; 64] {
    let present: &[usize] = match isa {
        // CMPXCHG8B, MMX, SSE, RDTSC, PAE, SSE2, NX, fast fail.
        Isa::X86_64 | Isa::X86_32 => &[2, 3, 6, 8, 9, 10, 12, 23],
        // CMPXCHG16B's equivalent, NX, fast fail, divide, 64-bit atomics, FMAC, ARMv8, crypto, CRC32, LSE atomics,
        // dot product, JSCVT, LRCPC.
        Isa::Arm64 => &[2, 12, 14, 23, 24, 25, 27, 29, 30, 31, 34, 43, 44, 45],
    };
    let mut f = [0u8; 64];
    for &i in present {
        f[i] = 1;
    }
    f
}

/// The shared data pages, a frame for each ISA.
pub struct SharedData {
    pub frames: Vec<(Isa, Frame)>,
}

impl SharedData {
    pub fn frame(&self, isa: Isa) -> Option<Frame> {
        self.frames.iter().find(|(i, _)| *i == isa).map(|(_, f)| *f)
    }
}

/// Makes the pages, with what doesn't change: the version, the system root, the processors and memory.
pub fn shared_data(k: &Kernel, cx: &mut Cx<'_>) -> SharedData {
    let cpus = k.host.cpus() as u32;
    let cookie = (k.host.wall() as u32).wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995;
    let frames = [Isa::Arm64, Isa::X86_64]
        .into_iter()
        .filter_map(|isa| {
            cx.turn(|t| {
                let pages = t.memory().size().limit / 4096;
                let frame = t.memory().new_frame().ok()?;
                let mut page = vec![0u8; 0x1000];
                let mut put = |at: usize, bytes: &[u8]| page[at..at + bytes.len()].copy_from_slice(bytes);
                put(0x004, &0x0fa0_0000u32.to_le_bytes()); // TickCountMultiplier: 15.625 ms in 24 bits of fraction
                put(0x02c, &machine(isa).to_le_bytes());
                put(0x02e, &machine(isa).to_le_bytes());
                let root: Vec<u8> = "C:\\Windows".encode_utf16().flat_map(u16::to_le_bytes).collect();
                put(0x030, &root);
                put(0x244, &0x20_0000u32.to_le_bytes()); // LargePageMinimum
                put(0x260, &BUILD.to_le_bytes());
                put(0x264, &1u32.to_le_bytes()); // NtProductType: a workstation
                put(0x268, &[1]);
                put(0x26a, &architecture(isa).to_le_bytes());
                put(0x26c, &MAJOR.to_le_bytes());
                put(0x270, &MINOR.to_le_bytes());
                put(0x274, &features(isa));
                put(0x2d0, &0x100u32.to_le_bytes()); // SuiteMask: VER_SUITE_SINGLEUSERTS
                put(0x2d8, &1u32.to_le_bytes()); // ActiveConsoleId
                put(0x2e8, &(pages as u32).to_le_bytes());
                put(0x300, &10_000_000u64.to_le_bytes()); // QpcFrequency, as QueryPerformanceFrequency gives
                put(0x330, &cookie.to_le_bytes());
                put(0x3c0, &cpus.to_le_bytes());
                put(0x3c4, &1u16.to_le_bytes()); // ActiveGroupCount
                t.memory().frame_write(frame, 0, &page);
                Some((isa, frame))
            })
        })
        .collect();
    let shared = SharedData { frames };
    tick(k, &shared);
    shared
}

/// Brings the pages' clocks up to now: interrupt time, system time, and the tick count.
fn tick(k: &Kernel, shared: &SharedData) {
    let interrupt = k.host.monotonic() / 100;
    let system = crate::files::filetime(k.host.wall());
    let ticks = interrupt / TICK;
    let mut exec = lock(&k.exec);
    let mut memory = exec.memory();
    for (_, frame) in &shared.frames {
        for (at, value) in [(0x008, interrupt), (0x014, system), (0x320, ticks)] {
            memory.frame_write(*frame, at + 8, &((value >> 32) as u32).to_le_bytes());
            fence(Ordering::Release);
            memory.frame_write(*frame, at, &(value as u32).to_le_bytes());
            fence(Ordering::Release);
            memory.frame_write(*frame, at + 4, &((value >> 32) as u32).to_le_bytes());
        }
    }
}

/// Keeps the clocks up to date, a tick at a time, for as long as the machine runs.
pub fn keep_time(k: Weak<Kernel>, when: Instant) {
    let Some(kernel) = k.upgrade() else { return };
    let timers = kernel.timers.clone();
    drop(kernel);
    let next = when + Duration::from_nanos(TICK * 100);
    timers.at(next, move || {
        let Some(kernel) = k.upgrade() else { return };
        if let Some(shared) = kernel.shared.get() {
            tick(&kernel, shared);
        }
        drop(kernel);
        // A tick that's fallen behind starts again from now rather than catching up.
        keep_time(k, next.max(Instant::now() - Duration::from_millis(1)));
    });
}

/// Days from 0000-03-01 (the start of Hinnant's civil calendar) to 1601-01-01 (FILETIME's).
const EPOCH: i64 = 584_694;

/// A FILETIME as SYSTEMTIME's eight fields: year, month, day of the week, day, hour, minute, second, millisecond.
pub fn system_time(filetime: u64) -> [u16; 8] {
    let ms = filetime / 10_000;
    let (days, ms) = (ms / 86_400_000, ms % 86_400_000);
    // Days since 1601-01-01 (a Monday) to a civil date, by Howard Hinnant's algorithm from 0000-03-01.
    let z = days as i64 + EPOCH; // to days since 0000-03-01
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = ((days + 1) % 7) as u16;
    [year as u16, month as u16, weekday, day as u16, (ms / 3_600_000) as u16, (ms / 60_000 % 60) as u16, (ms / 1000 % 60) as u16, (ms % 1000) as u16]
}

/// SYSTEMTIME's fields as a FILETIME, or None for a date that isn't one (the day of the week is ignored).
pub fn file_time(st: [u16; 8]) -> Option<u64> {
    let [year, month, _, day, hour, minute, second, ms] = st.map(i64::from);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1601..=30827).contains(&year) || !(1..=12).contains(&month) || day < 1 || day > days_in[(month - 1) as usize] || hour > 23 || minute > 59 || second > 59 || ms > 999 {
        return None;
    }
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - EPOCH;
    let ms_total = days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000 + ms;
    Some(ms_total as u64 * 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips() {
        // 1601-01-01 00:00 was a Monday.
        assert_eq!(system_time(0), [1601, 1, 1, 1, 0, 0, 0, 0]);
        // 1970-01-01 was a Thursday.
        assert_eq!(system_time(116_444_736_000_000_000), [1970, 1, 4, 1, 0, 0, 0, 0]);
        // 2000-02-29 12:34:56.789, a Tuesday.
        let leap = [2000, 2, 2, 29, 12, 34, 56, 789];
        let ft = file_time(leap).unwrap();
        assert_eq!(system_time(ft), leap);
        assert_eq!(file_time([2023, 2, 0, 29, 0, 0, 0, 0]), None);
        assert_eq!(file_time([1600, 12, 0, 31, 0, 0, 0, 0]), None);
    }
}
