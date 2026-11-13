//! kernel32's system information: which Windows it is (`GetVersionEx`, `VerifyVersionInfo`), the machine
//! (`GetSystemInfo`, memory, the processor's features and how its cores and caches are laid out), its name, and the
//! calendar and time zone. Everything here says the same as the shared data page and the PEB (`system`).
//!
//! The time zone is UTC: the host doesn't say which zone it's in, and a program that's given one converts by its rules.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::Reply;
use crate::process::{ALLOC, GRANULE, Task};
use crate::system;

const TRUE: u64 = 1;
const FALSE: u64 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_BAD_ARGUMENTS: u32 = 160;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NOACCESS: u32 = 998;
const ERROR_OLD_WIN_VERSION: u32 = 1150;

impl Task {
    /// Writes `bytes` at `at`, or says why it couldn't (`ERROR_NOACCESS`).
    pub(crate) fn put(&mut self, cx: &mut Cx<'_>, at: u64, bytes: &[u8]) -> Result<(), u64> {
        match self.mem(cx, true, |t| user::write(&mut t.space(), at, bytes)) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.set_last_error(cx, ERROR_NOACCESS);
                Err(FALSE)
            }
        }
    }

    pub(crate) fn get(&mut self, cx: &mut Cx<'_>, at: u64, len: usize) -> Result<Vec<u8>, u64> {
        match self.mem(cx, false, |t| user::read_vec(&t.space(), at, len)) {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                self.set_last_error(cx, ERROR_NOACCESS);
                Err(FALSE)
            }
        }
    }

    /// The version this program is told: Windows 10, or Windows 8 if its manifest doesn't say it knows 10.
    fn told_version(&self) -> (u32, u32, u32) {
        if kernels_core::lock(&self.process.state).windows10 { (system::MAJOR, system::MINOR, system::BUILD) } else { system::LEGACY }
    }
}

fn reply(r: Result<u64, u64>) -> Reply {
    Reply::Value(r.unwrap_or_else(|e| e))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

// --- The version ---

/// `GetVersion()`: the build, minor and major version in one DWORD.
pub fn get_version(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    let (major, minor, build) = task.told_version();
    Reply::Value(u64::from((build & 0x7fff) << 16 | minor << 8 | major))
}

/// `GetVersionExW(info)` and `GetVersionExA`: `OSVERSIONINFO(EX)`, by the size the program put in it.
fn get_version_ex(task: &mut Task, cx: &mut Cx<'_>, info: u64, chars: usize) -> Result<u64, u64> {
    let size = u32_at(&task.get(cx, info, 4)?, 0) as usize;
    let (base, ex) = (20 + chars, 20 + chars + 8);
    if size != base && size != ex {
        task.set_last_error(cx, ERROR_INSUFFICIENT_BUFFER);
        return Err(FALSE);
    }
    let (major, minor, build) = task.told_version();
    let mut b = vec![0u8; size];
    for (at, v) in [(0, size as u32), (4, major), (8, minor), (12, build), (16, 2)] {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    if size == ex {
        // No service pack; VER_SUITE_SINGLEUSERTS; VER_NT_WORKSTATION.
        b[base + 4..base + 6].copy_from_slice(&0x100u16.to_le_bytes());
        b[base + 6] = 1;
    }
    task.put(cx, info, &b)?;
    Ok(TRUE)
}

pub fn get_version_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_version_ex(task, cx, a[0], 256))
}

pub fn get_version_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_version_ex(task, cx, a[0], 128))
}

/// `VerSetConditionMask(mask, types, condition)`: three bits of condition for each type.
pub fn ver_set_condition_mask(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (mut mask, types, condition) = (a[0], a[1] as u32, a[2] & 7);
    if condition != 0 {
        for i in 0..8 {
            if types & (1 << i) != 0 {
                mask = mask & !(7 << (i * 3)) | condition << (i * 3);
            }
        }
    }
    Reply::Value(mask)
}

/// Whether `have` stands in relation `condition` (VER_EQUAL … VER_LESS_EQUAL) to `want`.
fn holds<T: Ord>(condition: u64, have: T, want: T) -> bool {
    use std::cmp::Ordering::*;
    match (condition, have.cmp(&want)) {
        (1, o) => o == Equal,
        (2, o) => o == Greater,
        (3, o) => o != Less,
        (4, o) => o == Less,
        (5, o) => o != Greater,
        _ => false,
    }
}

/// `VerifyVersionInfoW(info, types, conditions)` and `A`: whether this Windows is as `info` asks, as the program is
/// told it (so a program without the manifest is Windows 8 here too, as on Windows).
fn verify_version_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], chars: usize) -> Result<u64, u64> {
    let (types, conditions) = (a[1] as u32, a[2]);
    if types == 0 || conditions == 0 {
        task.set_last_error(cx, ERROR_BAD_ARGUMENTS);
        return Err(FALSE);
    }
    let ex = 20 + chars;
    let b = task.get(cx, a[0], ex + 8)?;
    let condition = |bit: u32| conditions >> (bit.trailing_zeros() * 3) & 7;
    let (major, minor, build) = task.told_version();
    // The version and service pack are one number, compared by the condition of its highest part asked for.
    let version = [(2, major, u32_at(&b, 4)), (1, minor, u32_at(&b, 8)), (0x20, 0, u32::from(u16_at(&b, ex))), (0x10, 0, u32::from(u16_at(&b, ex + 2)))];
    let asked: Vec<_> = version.iter().filter(|(bit, _, _)| types & bit != 0).collect();
    let mut ok = asked.first().is_none_or(|(bit, _, _)| {
        holds(condition(*bit), asked.iter().map(|v| v.1).collect::<Vec<_>>(), asked.iter().map(|v| v.2).collect::<Vec<_>>())
    });
    if types & 4 != 0 {
        ok &= holds(condition(4), build, u32_at(&b, 12));
    }
    if types & 8 != 0 {
        ok &= holds(condition(8), 2u32, u32_at(&b, 16));
    }
    if types & 0x40 != 0 {
        let want = u32::from(u16_at(&b, ex + 4));
        ok &= match condition(0x40) {
            6 => 0x100 & want == want,
            7 => 0x100 & want != 0,
            _ => false,
        };
    }
    if types & 0x80 != 0 {
        ok &= holds(condition(0x80), 1u8, b[ex + 6]);
    }
    if ok {
        Ok(TRUE)
    } else {
        task.set_last_error(cx, ERROR_OLD_WIN_VERSION);
        Err(FALSE)
    }
}

pub fn verify_version_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(verify_version_info(task, cx, a, 256))
}

pub fn verify_version_info_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(verify_version_info(task, cx, a, 128))
}

/// `GetProductInfo(major, minor, sp major, sp minor, &type)`: Windows 10 Pro.
pub fn get_product_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(task.put(cx, a[4], &0x30u32.to_le_bytes()).map(|_| TRUE))
}

// --- The machine ---

/// `GetSystemInfo(&info)` and `GetNativeSystemInfo`: the same here, as every program runs as its own ISA.
pub fn get_system_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cpus = task.cpus();
    let mut b = [0u8; 48];
    b[0..2].copy_from_slice(&system::architecture(task.isa).to_le_bytes());
    b[4..8].copy_from_slice(&4096u32.to_le_bytes());
    b[8..16].copy_from_slice(&GRANULE.to_le_bytes());
    b[16..24].copy_from_slice(&(ALLOC.end - GRANULE - 1).to_le_bytes());
    b[24..32].copy_from_slice(&(u64::MAX >> (64 - cpus)).to_le_bytes());
    b[32..36].copy_from_slice(&cpus.to_le_bytes());
    // PROCESSOR_AMD_X8664, family 6; ARM64 has no processor type.
    let (kind, level) = if task.isa == exec::Isa::Arm64 { (0u32, 8u16) } else { (8664, 6) };
    b[36..40].copy_from_slice(&kind.to_le_bytes());
    b[40..44].copy_from_slice(&(GRANULE as u32).to_le_bytes());
    b[44..46].copy_from_slice(&level.to_le_bytes());
    let _ = task.put(cx, a[0], &b);
    Reply::Value(0)
}

/// Physical memory: the machine's, and how much of it is free.
fn physical(cx: &mut Cx<'_>) -> (u64, u64) {
    let size = cx.turn(|t| t.memory().size());
    (size.limit, size.available.min(size.limit))
}

/// `GlobalMemoryStatusEx(&status)`: `MEMORYSTATUSEX`, which must say it's 64 bytes.
pub fn global_memory_status_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        if u32_at(&task.get(cx, a[0], 4)?, 0) != 64 {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Err(FALSE);
        }
        let (total, free) = physical(cx);
        let virt = ALLOC.end - GRANULE;
        let load = ((total - free) * 100).checked_div(total).unwrap_or(0) as u32;
        let mut b = Vec::with_capacity(64);
        b.extend(64u32.to_le_bytes());
        b.extend(load.to_le_bytes());
        for v in [total, free, total, free, virt, virt, 0] {
            b.extend(v.to_le_bytes());
        }
        task.put(cx, a[0], &b)?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `GlobalMemoryStatus(&status)`: the older `MEMORYSTATUS`, which has no result.
pub fn global_memory_status(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (total, free) = physical(cx);
    let virt = ALLOC.end - GRANULE;
    let load = ((total - free) * 100).checked_div(total).unwrap_or(0) as u32;
    let mut b = Vec::with_capacity(56);
    b.extend(56u32.to_le_bytes());
    b.extend(load.to_le_bytes());
    for v in [total, free, total, free, virt, virt] {
        b.extend(v.to_le_bytes());
    }
    let _ = task.put(cx, a[0], &b);
    Reply::Value(0)
}

/// `IsProcessorFeaturePresent(feature)`, as the shared data page has them.
pub fn is_processor_feature_present(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let feature = a[0] as u32 as usize;
    Reply::Value(u64::from(feature < 64 && system::features(task.isa)[feature] != 0))
}

/// `GetSystemTimes(&idle, &kernel, &user)`: every processor's, as `NtQuerySystemInformation` gives them.
pub fn get_system_times(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (idle, kernel, user) = crate::system_info::times(task);
    let mut r = Ok(());
    for (at, v) in [(a[0], idle), (a[1], kernel), (a[2], user)] {
        if at != 0 {
            r = r.and_then(|_| task.put(cx, at, &v.to_le_bytes()));
        }
    }
    reply(r.map(|_| TRUE))
}

/// `IsWow64Process(process, &wow64)`: never, as there's no 32-bit Windows under a 64-bit one here yet.
pub fn is_wow64_process(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(task.put(cx, a[1], &0u32.to_le_bytes()).map(|_| TRUE))
}

/// `IsWow64Process2(process, &process machine, &native machine)`: not WOW64, on its own ISA's machine.
pub fn is_wow64_process2(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let native = system::machine(task.isa);
    let r = task.put(cx, a[1], &0u16.to_le_bytes()).and_then(|_| if a[2] != 0 { task.put(cx, a[2], &native.to_le_bytes()) } else { Ok(()) });
    reply(r.map(|_| TRUE))
}

/// `GetActiveProcessorCount(group)` and `GetMaximumProcessorCount`: one group of every processor.
pub fn get_active_processor_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let group = a[0] as u16;
    Reply::Value(if group == 0 || group == 0xffff { u64::from(task.cpus()) } else { 0 })
}

pub fn get_active_processor_group_count(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

/// The caches each processor sees: (level, line size, size, type, one per core or shared), type 0 unified, 1
/// instruction, 2 data. Apple silicon's lines are 128 bytes; x86-64 programs are told 64, as their processors have.
fn caches(isa: exec::Isa) -> [(u8, u16, u32, u32, bool); 3] {
    let line = if isa == exec::Isa::Arm64 { 128 } else { 64 };
    [(1, line, 64 << 10, 2, true), (1, line, 128 << 10, 1, true), (2, line, 4 << 20, 0, false)]
}

/// `GetLogicalProcessorInformation(buffer, &length)`: 32-byte records, a core for each processor, its caches, the
/// package and the one NUMA node.
pub fn get_logical_processor_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cpus = task.cpus();
    let all = u64::MAX >> (64 - cpus);
    let record = |mask: u64, relation: u32, rest: &[u8]| {
        let mut r = [0u8; 32];
        r[0..8].copy_from_slice(&mask.to_le_bytes());
        r[8..12].copy_from_slice(&relation.to_le_bytes());
        r[16..16 + rest.len()].copy_from_slice(rest);
        r
    };
    let cache = |level: u8, line: u16, size: u32, kind: u32| {
        let mut c = [0u8; 12];
        c[0] = level;
        c[1] = 8; // 8-way
        c[2..4].copy_from_slice(&line.to_le_bytes());
        c[4..8].copy_from_slice(&size.to_le_bytes());
        c[8..12].copy_from_slice(&kind.to_le_bytes());
        c
    };
    let mut out = Vec::new();
    for cpu in 0..cpus {
        out.extend(record(1 << cpu, 0, &[]));
        for (level, line, size, kind, own) in caches(task.isa) {
            if own {
                out.extend(record(1 << cpu, 2, &cache(level, line, size, kind)));
            }
        }
    }
    for (level, line, size, kind, own) in caches(task.isa) {
        if !own {
            out.extend(record(all, 2, &cache(level, line, size, kind)));
        }
    }
    out.extend(record(all, 3, &[]));
    out.extend(record(all, 1, &[]));
    reply(put_sized(task, cx, a[0], a[1], &out, 4))
}

/// `GetLogicalProcessorInformationEx(relation, buffer, &length)`: variable-length records of the relation asked for
/// (0xffff all): cores, NUMA nodes, caches, the package and processor groups.
pub fn get_logical_processor_information_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let relation = a[0] as u32;
    let cpus = task.cpus();
    let all = u64::MAX >> (64 - cpus);
    let group_affinity = |mask: u64| {
        let mut g = [0u8; 16];
        g[0..8].copy_from_slice(&mask.to_le_bytes());
        g
    };
    let record = |relation: u32, body: Vec<u8>| {
        let mut r = Vec::with_capacity(8 + body.len());
        r.extend(relation.to_le_bytes());
        r.extend((8 + body.len() as u32).to_le_bytes());
        r.extend(body);
        r
    };
    // PROCESSOR_RELATIONSHIP and NUMA_NODE_RELATIONSHIP: 22 bytes, GroupCount at 30, the group's mask at 32.
    let with_group = |head: &[u8], mask: u64| {
        let mut b = vec![0u8; 24];
        b[..head.len()].copy_from_slice(head);
        b[22..24].copy_from_slice(&1u16.to_le_bytes());
        b.extend(group_affinity(mask));
        b
    };
    let mut out = Vec::new();
    let wants = |r: u32| relation == 0xffff || relation == r;
    if wants(0) {
        for cpu in 0..cpus {
            out.extend(record(0, with_group(&[], 1 << cpu)));
        }
    }
    if wants(1) {
        out.extend(record(1, with_group(&[], all)));
    }
    if wants(2) {
        for (level, line, size, kind, own) in caches(task.isa) {
            let masks: Vec<u64> = if own { (0..cpus).map(|c| 1 << c).collect() } else { vec![all] };
            for mask in masks {
                let mut b = vec![0u8; 32];
                b[0] = level;
                b[1] = 8;
                b[2..4].copy_from_slice(&line.to_le_bytes());
                b[4..8].copy_from_slice(&size.to_le_bytes());
                b[8..12].copy_from_slice(&kind.to_le_bytes());
                b[30..32].copy_from_slice(&1u16.to_le_bytes());
                b.extend(group_affinity(mask));
                out.extend(record(2, b));
            }
        }
    }
    if wants(3) {
        out.extend(record(3, with_group(&[], all)));
    }
    if wants(4) {
        let mut b = vec![0u8; 24 + 48];
        b[0..2].copy_from_slice(&1u16.to_le_bytes());
        b[2..4].copy_from_slice(&1u16.to_le_bytes());
        b[24] = cpus as u8;
        b[25] = cpus as u8;
        b[64..72].copy_from_slice(&all.to_le_bytes());
        out.extend(record(4, b));
    }
    reply(put_sized(task, cx, a[1], a[2], &out, 4))
}

/// Writes `data` to `buffer` if `*length` (of `width` bytes) has room for it, else says how much it needs.
fn put_sized(task: &mut Task, cx: &mut Cx<'_>, buffer: u64, length: u64, data: &[u8], width: usize) -> Result<u64, u64> {
    let room = task.get(cx, length, width)?;
    let room = if width == 4 { u64::from(u32_at(&room, 0)) } else { u64::from_le_bytes(room[..8].try_into().unwrap()) };
    task.put(cx, length, &(data.len() as u64).to_le_bytes()[..width])?;
    if room < data.len() as u64 || buffer == 0 {
        task.set_last_error(cx, ERROR_INSUFFICIENT_BUFFER);
        return Err(FALSE);
    }
    task.put(cx, buffer, data)?;
    Ok(TRUE)
}

// --- The machine's name ---

fn computer_name() -> &'static str {
    crate::registry::variable("computername").unwrap_or("CARBON")
}

/// Writes `name` into a buffer of `*size` characters, as the computer name calls do: its length without the NUL if it
/// fits, else the size it needs with it, failing with `too_small`.
fn put_name(task: &mut Task, cx: &mut Cx<'_>, buffer: u64, size: u64, name: &str, ansi: bool, too_small: u32) -> Result<u64, u64> {
    let room = u32_at(&task.get(cx, size, 4)?, 0) as usize;
    let mut bytes = if ansi { crate::ansi::bytes(name) } else { wide(name) };
    let len = if ansi { bytes.len() } else { bytes.len() / 2 };
    if room <= len || buffer == 0 {
        task.put(cx, size, &(len as u32 + 1).to_le_bytes())?;
        task.set_last_error(cx, too_small);
        return Err(FALSE);
    }
    bytes.extend(if ansi { &[0u8][..] } else { &[0, 0] });
    task.put(cx, buffer, &bytes)?;
    task.put(cx, size, &(len as u32).to_le_bytes())?;
    Ok(TRUE)
}

pub fn get_computer_name_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(put_name(task, cx, a[0], a[1], computer_name(), false, ERROR_BUFFER_OVERFLOW))
}

pub fn get_computer_name_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(put_name(task, cx, a[0], a[1], computer_name(), true, ERROR_BUFFER_OVERFLOW))
}

/// `GetComputerNameExW(format, buffer, &size)`: NetBIOS and DNS names the same, and no domain.
fn get_computer_name_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], ansi: bool) -> Result<u64, u64> {
    let name = match a[0] as u32 {
        0 | 1 | 3 | 4 | 5 | 7 => computer_name(),
        2 | 6 => "",
        _ => {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Err(FALSE);
        }
    };
    put_name(task, cx, a[1], a[2], name, ansi, ERROR_MORE_DATA)
}

pub fn get_computer_name_ex_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_computer_name_ex(task, cx, a, false))
}

pub fn get_computer_name_ex_a(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(get_computer_name_ex(task, cx, a, true))
}

// --- Time ---

fn systemtime_bytes(st: [u16; 8]) -> Vec<u8> {
    st.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn read_systemtime(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Result<[u16; 8], u64> {
    let b = task.get(cx, at, 16)?;
    Ok(std::array::from_fn(|i| u16_at(&b, i * 2)))
}

fn read_filetime(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Result<u64, u64> {
    Ok(u64::from_le_bytes(task.get(cx, at, 8)?[..8].try_into().unwrap()))
}

/// `GetSystemTime(&st)` and `GetLocalTime`, the same in UTC.
pub fn get_system_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let now = crate::files::filetime(task.k.host.wall());
    let _ = task.put(cx, a[0], &systemtime_bytes(system::system_time(now)));
    Reply::Value(0)
}

/// `FileTimeToSystemTime(&ft, &st)`.
pub fn file_time_to_system_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let ft = read_filetime(task, cx, a[0])?;
        if ft >> 63 != 0 {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Err(FALSE);
        }
        task.put(cx, a[1], &systemtime_bytes(system::system_time(ft)))?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `SystemTimeToFileTime(&st, &ft)`.
pub fn system_time_to_file_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| {
        let st = read_systemtime(task, cx, a[0])?;
        let Some(ft) = system::file_time(st) else {
            task.set_last_error(cx, ERROR_INVALID_PARAMETER);
            return Err(FALSE);
        };
        task.put(cx, a[1], &ft.to_le_bytes())?;
        Ok(TRUE)
    })();
    reply(r)
}

/// `FileTimeToLocalFileTime(&ft, &local)` and back: the same, in UTC.
pub fn file_time_to_local_file_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = read_filetime(task, cx, a[0]).and_then(|ft| task.put(cx, a[1], &ft.to_le_bytes()));
    reply(r.map(|_| TRUE))
}

/// `TIME_ZONE_INFORMATION` for UTC: no bias and no daylight saving.
fn utc_zone() -> Vec<u8> {
    let mut b = vec![0u8; 172];
    let name = wide("Coordinated Universal Time");
    b[4..4 + name.len()].copy_from_slice(&name);
    b[88..88 + name.len()].copy_from_slice(&name);
    b
}

/// `GetTimeZoneInformation(&tz)`: `TIME_ZONE_ID_UNKNOWN`, as a zone without daylight saving has.
pub fn get_time_zone_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(task.put(cx, a[0], &utc_zone()).map(|_| 0).map_err(|_| u64::from(u32::MAX)))
}

/// `GetDynamicTimeZoneInformation(&tz)`: the same, with its key name.
pub fn get_dynamic_time_zone_information(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut b = utc_zone();
    b.resize(432, 0);
    let key = wide("UTC");
    b[172..172 + key.len()].copy_from_slice(&key);
    reply(task.put(cx, a[0], &b).map(|_| 0).map_err(|_| u64::from(u32::MAX)))
}

/// A zone's rules, from its `TIME_ZONE_INFORMATION`: biases in minutes (UTC = local + bias), and when daylight
/// saving starts and ends, as SYSTEMTIMEs.
struct Zone {
    bias: i64,
    standard_bias: i64,
    daylight_bias: i64,
    standard: [u16; 8],
    daylight: [u16; 8],
}

impl Zone {
    fn of(b: &[u8]) -> Zone {
        let i = |at: usize| i64::from(u32_at(b, at) as i32);
        let st = |at: usize| std::array::from_fn(|k| u16_at(b, at + k * 2));
        Zone { bias: i(0), standard_bias: i(84), daylight_bias: i(168), standard: st(68), daylight: st(152) }
    }

    /// When a rule (`StandardDate` or `DaylightDate`) falls in `year`, in the local time it's given in: the `wDay`th
    /// (5 the last) `wDayOfWeek` of `wMonth`, or a date of its own if it has a year.
    fn when(rule: [u16; 8], year: u16) -> Option<u64> {
        if rule[0] != 0 {
            return system::file_time(rule);
        }
        let first = system::file_time([year, rule[1], 0, 1, 0, 0, 0, 0])?;
        let weekday = system::system_time(first)[2];
        let mut day = 1 + (rule[2] + 7 - weekday) % 7 + (rule[3].clamp(1, 5) - 1) * 7;
        loop {
            if let Some(ft) = system::file_time([year, rule[1], 0, day, rule[4], rule[5], rule[6], rule[7]]) {
                return Some(ft);
            }
            day = day.checked_sub(7)?;
        }
    }

    /// Whether `utc` is in daylight saving time.
    fn daylight(&self, utc: u64) -> bool {
        if self.daylight[1] == 0 || self.standard[1] == 0 {
            return false;
        }
        let year = system::system_time(utc)[0];
        let minutes = |m: i64| (m * 600_000_000) as u64;
        let (Some(start), Some(end)) = (Zone::when(self.daylight, year), Zone::when(self.standard, year)) else { return false };
        // Each starts in the local time of the one before it.
        let start = start.wrapping_add(minutes(self.bias + self.standard_bias));
        let end = end.wrapping_add(minutes(self.bias + self.daylight_bias));
        if start < end { (start..end).contains(&utc) } else { !(end..start).contains(&utc) }
    }

    fn to_local(&self, utc: u64) -> u64 {
        let bias = self.bias + if self.daylight(utc) { self.daylight_bias } else { self.standard_bias };
        utc.wrapping_sub((bias * 600_000_000) as u64)
    }

    fn to_utc(&self, local: u64) -> u64 {
        let daylight = local.wrapping_add(((self.bias + self.daylight_bias) * 600_000_000) as u64);
        if self.daylight(daylight) { daylight } else { local.wrapping_add(((self.bias + self.standard_bias) * 600_000_000) as u64) }
    }
}

/// `SystemTimeToTzSpecificLocalTime(zone, &utc, &local)` and `TzSpecificLocalTimeToSystemTime`: by the zone given, or
/// this machine's (UTC).
fn convert(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], to_local: bool) -> Result<u64, u64> {
    let zone = if a[0] == 0 { utc_zone() } else { task.get(cx, a[0], 172)? };
    let zone = Zone::of(&zone);
    let st = read_systemtime(task, cx, a[1])?;
    let Some(ft) = system::file_time(st) else {
        task.set_last_error(cx, ERROR_INVALID_PARAMETER);
        return Err(FALSE);
    };
    let out = if to_local { zone.to_local(ft) } else { zone.to_utc(ft) };
    task.put(cx, a[2], &systemtime_bytes(system::system_time(out)))?;
    Ok(TRUE)
}

pub fn system_time_to_tz_specific_local_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(convert(task, cx, a, true))
}

pub fn tz_specific_local_time_to_system_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    reply(convert(task, cx, a, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daylight_saving_by_a_zones_rules() {
        // US Eastern: UTC+5 standard, an hour less from the second Sunday of March at 2:00 to the first of November.
        let mut b = vec![0u8; 172];
        b[0..4].copy_from_slice(&300i32.to_le_bytes());
        let put = |b: &mut Vec<u8>, at: usize, st: [u16; 8]| b[at..at + 16].copy_from_slice(&systemtime_bytes(st));
        put(&mut b, 68, [0, 11, 0, 1, 2, 0, 0, 0]);
        put(&mut b, 152, [0, 3, 0, 2, 2, 0, 0, 0]);
        b[168..172].copy_from_slice(&(-60i32).to_le_bytes());
        let zone = Zone::of(&b);
        let at = |st| system::file_time(st).unwrap();
        // 2024-07-01 16:00 UTC is 12:00 EDT; 2024-01-15 17:00 UTC is 12:00 EST.
        assert_eq!(system::system_time(zone.to_local(at([2024, 7, 0, 1, 16, 0, 0, 0])))[4], 12);
        assert_eq!(system::system_time(zone.to_local(at([2024, 1, 0, 15, 17, 0, 0, 0])))[4], 12);
        // 2024's change was 10 March, 07:00 UTC.
        assert!(!zone.daylight(at([2024, 3, 0, 10, 6, 59, 0, 0])));
        assert!(zone.daylight(at([2024, 3, 0, 10, 7, 0, 0, 0])));
        // And back on 3 November, 06:00 UTC.
        assert!(zone.daylight(at([2024, 11, 0, 3, 5, 59, 0, 0])));
        assert!(!zone.daylight(at([2024, 11, 0, 3, 6, 0, 0, 0])));
        assert_eq!(zone.to_utc(at([2024, 7, 0, 1, 12, 0, 0, 0])), at([2024, 7, 0, 1, 16, 0, 0, 0]));
    }
}
