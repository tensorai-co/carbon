//! `NtQuerySystemInformation`: the classes programs ask for, as Windows 10 lays them out for 64-bit programs.
//!
//! - basic and processor information (and their emulation twins), the same machine `GetSystemInfo` describes;
//! - performance, and each processor's times, from the clocks (atlas doesn't count the CPU time programs take, so the
//!   machine looks idle);
//! - the time of day: boot time, now, and UTC's bias;
//! - the process list: an idle process, then every process with its threads, as `CreateToolhelp32Snapshot` and task
//!   managers read it;
//! - the handle list, both forms, every process's handles with a type and an object address that's the same for the
//!   same object in every process;
//! - the kernel's modules (`ntoskrnl.exe` and `hal.dll`, at kernel addresses), and the kernel debugger (none).
//!
//! Each writes what fits, says in `ReturnLength` how much it wrote or needs, and gives STATUS_INFO_LENGTH_MISMATCH
//! when the buffer's too small.

use std::sync::Arc;

use atlas_abi::status;
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use crate::handles::Handle;
use crate::objects::Kind;
use crate::process::{AreaKind, GRANULE, Task};
use crate::system;

const INFO_LENGTH_MISMATCH: u32 = 0xc000_0004;
const INVALID_INFO_CLASS: u32 = 0xc000_0003;

/// Where the kernel is said to be: its address space, and the modules in it.
const SYSTEM_RANGE_START: u64 = 0xffff_8000_0000_0000;
const KERNEL_MODULES: &[(&str, u64, u32)] = &[("ntoskrnl.exe", 0xffff_f800_0000_0000, 0x0104_6000), ("hal.dll", 0xffff_f800_0110_0000, 0x0006_8000)];

/// A buffer being filled: little-endian fields at offsets.
struct Out(Vec<u8>);

impl Out {
    fn new(len: usize) -> Out {
        Out(vec![0; len])
    }
    fn u8(&mut self, at: usize, v: u8) {
        self.0[at] = v;
    }
    fn u16(&mut self, at: usize, v: u16) {
        self.0[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, at: usize, v: u64) {
        self.0[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
}

/// One process, as the list classes see it.
struct Snapshot {
    pid: u64,
    parent: u64,
    created: u64,
    name: String,
    threads: Vec<u64>,
    thread_start: u64,
    handles: Vec<(u64, Handle)>,
    virtual_size: u64,
    committed: u64,
}

impl Task {
    /// `NtQuerySystemInformation(class, buffer, length, &return length)`: an NTSTATUS.
    pub fn query_system_information(&mut self, cx: &mut Cx<'_>, class: u32, buffer: u64, length: u64, returned: u64) -> u32 {
        let length = length as u32 as usize;
        // A 32-bit program gets the classes with pointers in them in its own layout; the kernel's module list and
        // the handle lists aren't written for it yet.
        let x32 = self.isa == exec::Isa::X86_32;
        let data = match class {
            0 | 0x3e => Some(self.basic(cx)),
            1 | 0x3f => Some(self.processor()),
            2 => Some(self.performance(cx)),
            3 => Some(self.time_of_day()),
            5 if x32 => Some(self.processes32(buffer)),
            5 => Some(self.processes(buffer)),
            8 => Some(self.processor_times(length)),
            11 | 16 | 64 if x32 => None,
            11 => Some(modules()),
            16 => Some(self.handle_list(false)),
            35 => Some(vec![0, 1]),
            50 if x32 => Some(0x8000_0000u32.to_le_bytes().to_vec()),
            50 => Some(SYSTEM_RANGE_START.to_le_bytes().to_vec()),
            64 => Some(self.handle_list(true)),
            _ => None,
        };
        let Some(mut data) = data else { return INVALID_INFO_CLASS };
        // Performance information grew in Windows 7: the older, shorter size is still taken, and gets what fits.
        if class == 2 && (0x138..data.len()).contains(&length) {
            data.truncate(length);
        }
        // So does the time of day's, which takes any length up to its own.
        if class == 3 && length < data.len() {
            data.truncate(length);
        }
        let fits = length >= data.len() && (class != 50 || length == data.len());
        if returned != 0 && self.mem(cx, true, |t| user::write_u32(&mut t.space(), returned, data.len() as u32)).is_err() {
            return status::ACCESS_VIOLATION;
        }
        if !fits {
            return INFO_LENGTH_MISMATCH;
        }
        if !data.is_empty() && self.mem(cx, true, |t| user::write(&mut t.space(), buffer, &data)).is_err() {
            return status::ACCESS_VIOLATION;
        }
        status::SUCCESS
    }

    pub(crate) fn cpus(&self) -> u32 {
        (self.k.host.cpus() as u32).clamp(1, 64)
    }

    /// Interrupt time: 100 ns units since the machine started, as the shared data page has it.
    fn uptime(&self) -> u64 {
        self.k.host.monotonic() / 100
    }

    /// `SYSTEM_BASIC_INFORMATION`: 64 bytes, or 44 for a 32-bit program, whose two addresses and mask are 4 bytes.
    fn basic(&self, cx: &mut Cx<'_>) -> Vec<u8> {
        let pages = cx.turn(|t| t.memory().size().limit) / 4096;
        let abi = self.abi();
        let cpus = self.cpus().min(abi.width as u32 * 8);
        let mut o = Out::new(if abi.width == 8 { 64 } else { 44 });
        o.u32(4, 156_250); // the timer's resolution, 15.625 ms
        o.u32(8, 4096);
        o.u32(12, pages as u32);
        o.u32(16, 1);
        o.u32(20, pages as u32);
        o.u32(24, GRANULE as u32);
        let addresses = if abi.width == 8 { 32 } else { 28 };
        let w = abi.width as usize;
        for (i, v) in [GRANULE, self.layout().alloc.end - GRANULE - 1, u64::MAX >> (64 - cpus)].into_iter().enumerate() {
            let at = addresses + i * w;
            o.0[at..at + w].copy_from_slice(&abi.ptr(v));
        }
        o.u8(addresses + 3 * w, cpus as u8);
        o.0
    }

    /// `SYSTEM_PROCESSOR_INFORMATION`: the architecture, level and the most processors there can be.
    fn processor(&self) -> Vec<u8> {
        let mut o = Out::new(12);
        o.u16(0, system::architecture(self.isa));
        o.u16(2, if self.isa == exec::Isa::Arm64 { 8 } else { 6 });
        o.u16(6, self.cpus() as u16);
        o.0
    }

    /// `SYSTEM_PERFORMANCE_INFORMATION`, Windows 7's and later's size: idle time, and memory in pages.
    fn performance(&self, cx: &mut Cx<'_>) -> Vec<u8> {
        let size = cx.turn(|t| t.memory().size());
        let (total, free) = (size.limit / 4096, size.available.min(size.limit) / 4096);
        let mut o = Out::new(0x158);
        o.u64(0, self.uptime() * u64::from(self.cpus()));
        o.u32(0x2c, free as u32);
        o.u32(0x30, (total - free) as u32);
        o.u32(0x34, total as u32);
        o.u32(0x38, (total - free) as u32);
        o.u64(0x150, (total - free) / 4);
        o.0
    }

    /// `SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION` for as many processors as fit (at least one must): each idle all
    /// along, its kernel time counting its idle time, as Windows' does.
    fn processor_times(&self, length: usize) -> Vec<u8> {
        let count = (length / 48).clamp(1, self.cpus() as usize);
        let up = self.uptime();
        let mut o = Out::new(48 * count);
        for i in 0..count {
            o.u64(i * 48, up);
            o.u64(i * 48 + 8, up);
        }
        o.0
    }

    /// `SYSTEM_TIMEOFDAY_INFORMATION`: when the machine started, now, and UTC's bias (none).
    fn time_of_day(&self) -> Vec<u8> {
        let now = crate::files::filetime(self.k.host.wall());
        let mut o = Out::new(48);
        o.u64(0, now - self.uptime());
        o.u64(8, now);
        o.0
    }

    /// Every process that hasn't ended, by ID, as it is now.
    fn snapshot(&self) -> Vec<Snapshot> {
        let processes: Vec<Arc<crate::process::Process>> = lock(&self.k.processes).values().filter_map(std::sync::Weak::upgrade).collect();
        processes
            .iter()
            .map(|p| {
                let state = lock(&p.state);
                let (mut virtual_size, mut committed) = (0, 0);
                for (range, area) in state.regions.iter() {
                    let len = range.end - range.start;
                    virtual_size += len;
                    if !matches!(area.kind, AreaKind::Reserved | AreaKind::Kept) {
                        committed += len;
                    }
                }
                Snapshot {
                    pid: p.pid,
                    parent: state.parent,
                    created: state.created,
                    name: state.image.rsplit('\\').next().unwrap_or_default().to_string(),
                    threads: state.threads.keys().copied().collect(),
                    thread_start: state.thread_start,
                    handles: state.handles.iter().map(|(v, h)| (v, h.clone())).collect(),
                    virtual_size,
                    committed,
                }
            })
            .collect()
    }

    /// `SYSTEM_PROCESS_INFORMATION` records, each followed by its threads' `SYSTEM_THREAD_INFORMATION` and its name,
    /// whose `UNICODE_STRING` points into the buffer at `base`. First the idle process: ID 0, a thread a processor.
    fn processes(&self, base: u64) -> Vec<u8> {
        const PROCESS: usize = 0x100;
        const THREAD: usize = 0x50;
        let up = self.uptime();
        let booted = crate::files::filetime(self.k.host.wall()) - up;
        let cpus = self.cpus() as usize;
        let mut out: Vec<u8> = Vec::new();
        let mut entries: Vec<(Snapshot, bool)> = vec![(
            Snapshot { pid: 0, parent: 0, created: 0, name: String::new(), threads: vec![0; cpus], thread_start: 0, handles: Vec::new(), virtual_size: 0, committed: 0 },
            true,
        )];
        entries.extend(self.snapshot().into_iter().map(|s| (s, false)));
        let last = entries.len() - 1;
        for (index, (p, idle)) in entries.into_iter().enumerate() {
            let start = out.len();
            let name: Vec<u8> = p.name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let size = (PROCESS + THREAD * p.threads.len() + name.len() + 2).next_multiple_of(8);
            let mut o = Out::new(size);
            if index != last {
                o.u32(0, size as u32);
            }
            o.u32(4, p.threads.len() as u32);
            o.u64(0x20, if idle { 0 } else { p.created });
            if idle {
                o.u64(0x30, up * cpus as u64);
            }
            if !name.is_empty() {
                let at = PROCESS + THREAD * p.threads.len();
                o.u16(0x38, name.len() as u16);
                o.u16(0x3a, name.len() as u16 + 2);
                o.u64(0x40, base + (start + at) as u64);
                o.0[at..at + name.len()].copy_from_slice(&name);
            }
            o.u32(0x48, if idle { 0 } else { 8 });
            o.u64(0x50, p.pid);
            o.u64(0x58, p.parent);
            o.u32(0x60, p.handles.len() as u32);
            o.u32(0x64, if idle { 0 } else { 1 });
            o.u64(0x68, p.pid);
            for at in [0x70, 0x78] {
                o.u64(at, p.virtual_size);
            }
            for at in [0x88, 0x90, 0xb8, 0xc0, 0xc8, 0x08] {
                o.u64(at, p.committed);
            }
            for (i, &tid) in p.threads.iter().enumerate() {
                let t = PROCESS + THREAD * i;
                if idle {
                    o.u64(t, up);
                }
                o.u64(t + 0x10, if idle { booted } else { p.created });
                o.u64(t + 0x20, p.thread_start);
                o.u64(t + 0x28, p.pid);
                o.u64(t + 0x30, tid);
                o.u32(t + 0x38, if idle { 0 } else { 8 });
                o.u32(t + 0x3c, if idle { 0 } else { 8 });
                // Running, for the thread asking; else waiting, as the program asked (UserRequest).
                let running = tid == self.tid && p.pid == self.process.pid;
                o.u32(t + 0x44, if running { 2 } else { 5 });
                o.u32(t + 0x48, if running { 0 } else { 6 });
            }
            out.extend(o.0);
        }
        out
    }

    /// The same, for a 32-bit program: `SYSTEM_PROCESS_INFORMATION` is 0xb8 bytes, with its handles, sizes and name's
    /// pointer 4 bytes each, and `SYSTEM_THREAD_INFORMATION` 0x40.
    fn processes32(&self, base: u64) -> Vec<u8> {
        const PROCESS: usize = 0xb8;
        const THREAD: usize = 0x40;
        let up = self.uptime();
        let booted = crate::files::filetime(self.k.host.wall()) - up;
        let cpus = self.cpus() as usize;
        let small = |v: u64| v.min(u64::from(u32::MAX)) as u32;
        let mut out: Vec<u8> = Vec::new();
        let mut entries: Vec<(Snapshot, bool)> = vec![(
            Snapshot { pid: 0, parent: 0, created: 0, name: String::new(), threads: vec![0; cpus], thread_start: 0, handles: Vec::new(), virtual_size: 0, committed: 0 },
            true,
        )];
        entries.extend(self.snapshot().into_iter().map(|s| (s, false)));
        let last = entries.len() - 1;
        for (index, (p, idle)) in entries.into_iter().enumerate() {
            let start = out.len();
            let name: Vec<u8> = p.name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let size = (PROCESS + THREAD * p.threads.len() + name.len() + 2).next_multiple_of(8);
            let mut o = Out::new(size);
            if index != last {
                o.u32(0, size as u32);
            }
            o.u32(4, p.threads.len() as u32);
            o.u64(0x20, if idle { 0 } else { p.created });
            if idle {
                o.u64(0x30, up * cpus as u64);
            }
            if !name.is_empty() {
                let at = PROCESS + THREAD * p.threads.len();
                o.u16(0x38, name.len() as u16);
                o.u16(0x3a, name.len() as u16 + 2);
                o.u32(0x3c, (base + (start + at) as u64) as u32);
                o.0[at..at + name.len()].copy_from_slice(&name);
            }
            o.u32(0x40, if idle { 0 } else { 8 });
            o.u32(0x44, p.pid as u32);
            o.u32(0x48, p.parent as u32);
            o.u32(0x4c, p.handles.len() as u32);
            o.u32(0x50, if idle { 0 } else { 1 });
            o.u32(0x54, p.pid as u32);
            for at in [0x58, 0x5c] {
                o.u32(at, small(p.virtual_size));
            }
            for at in [0x64, 0x68, 0x7c, 0x80, 0x84, 0x08] {
                o.u32(at, small(p.committed));
            }
            for (i, &tid) in p.threads.iter().enumerate() {
                let t = PROCESS + THREAD * i;
                if idle {
                    o.u64(t, up);
                }
                o.u64(t + 0x10, if idle { booted } else { p.created });
                o.u32(t + 0x1c, p.thread_start as u32);
                o.u32(t + 0x20, p.pid as u32);
                o.u32(t + 0x24, tid as u32);
                o.u32(t + 0x28, if idle { 0 } else { 8 });
                o.u32(t + 0x2c, if idle { 0 } else { 8 });
                let running = tid == self.tid && p.pid == self.process.pid;
                o.u32(t + 0x34, if running { 2 } else { 5 });
                o.u32(t + 0x38, if running { 0 } else { 6 });
            }
            out.extend(o.0);
        }
        out
    }

    /// `SYSTEM_HANDLE_INFORMATION`, or with `extended` `SYSTEM_HANDLE_INFORMATION_EX`: every process's handles.
    fn handle_list(&self, extended: bool) -> Vec<u8> {
        let handles: Vec<(u64, u64, Handle)> = self.snapshot().into_iter().flat_map(|p| p.handles.into_iter().map(move |(v, h)| (p.pid, v, h))).collect();
        let (head, each) = if extended { (16, 40) } else { (8, 24) };
        let mut o = Out::new(head + each * handles.len());
        o.u64(0, handles.len() as u64);
        for (i, (pid, value, handle)) in handles.iter().enumerate() {
            let at = head + each * i;
            let (kind, access) = type_of(handle);
            let object = object_address(handle);
            if extended {
                o.u64(at, object);
                o.u64(at + 8, *pid);
                o.u64(at + 16, *value);
                o.u32(at + 24, access);
                o.u16(at + 30, u16::from(kind));
            } else {
                o.u16(at, *pid as u16);
                o.u8(at + 4, kind);
                o.u16(at + 6, *value as u16);
                o.u64(at + 8, object);
                o.u32(at + 16, access);
            }
        }
        o.0
    }
}

/// Windows 10's object type names, by the numbers `type_of` gives.
fn type_name(kind: u8) -> &'static str {
    match kind {
        5 => "Token",
        7 => "Process",
        8 => "Thread",
        16 => "Event",
        17 => "Mutant",
        19 => "Semaphore",
        20 => "Timer",
        36 => "IoCompletion",
        42 => "Section",
        44 => "Key",
        _ => "File",
    }
}

const INFO_LENGTH_MISMATCH_STATUS: u32 = 0xc000_0004;
/// OBJ_INHERIT, as handle attributes give it.
const OBJ_INHERIT: u32 = 0x2;

impl Task {
    /// `NtQueryInformationProcess`'s ProcessHandleInformation (51) for this process: PROCESS_HANDLE_SNAPSHOT_INFORMATION,
    /// each handle's value, counts, access, type and attributes. An NTSTATUS; the length needed is written either way.
    /// Or ProcessHandleTable (58): each handle's value, as a ULONG.
    pub fn process_handles(&mut self, cx: &mut Cx<'_>, class: u32, out: u64, length: u64, returned: u64) -> u32 {
        let handles: Vec<(u64, Handle, bool)> = {
            let state = lock(&self.process.state);
            state.handles.iter().map(|(v, h)| (v, h.clone(), state.handles.inheritable(v).unwrap_or(false))).collect()
        };
        if class == 58 {
            let table: Vec<u8> = handles.iter().flat_map(|(v, _, _)| (*v as u32).to_le_bytes()).collect();
            return self.answer_info(cx, &table, out, length, returned);
        }
        let w = self.abi().width as usize;
        let (head, each) = (2 * w, 3 * w + 16);
        let mut o = Out::new(head + each * handles.len());
        let put_ptr = |o: &mut Out, at: usize, v: u64| if w == 8 { o.u64(at, v) } else { o.u32(at, v as u32) };
        put_ptr(&mut o, 0, handles.len() as u64);
        for (i, (value, handle, inherit)) in handles.iter().enumerate() {
            let at = head + each * i;
            let (kind, access) = type_of(handle);
            put_ptr(&mut o, at, *value);
            put_ptr(&mut o, at + w, 1);
            put_ptr(&mut o, at + 2 * w, 1);
            o.u32(at + 3 * w, access);
            o.u32(at + 3 * w + 4, u32::from(kind));
            o.u32(at + 3 * w + 8, if *inherit { OBJ_INHERIT } else { 0 });
        }
        self.answer_info(cx, &o.0, out, length, returned)
    }

    /// Writes `data` to the program's buffer if it fits, and its length to `returned`: STATUS_SUCCESS, or
    /// STATUS_INFO_LENGTH_MISMATCH when the buffer's too small.
    fn answer_info(&mut self, cx: &mut Cx<'_>, data: &[u8], out: u64, length: u64, returned: u64) -> u32 {
        if returned != 0 {
            let _ = self.mem(cx, true, |t| user::write_u32(&mut t.space(), returned, data.len() as u32));
        }
        if (length as usize) < data.len() {
            return INFO_LENGTH_MISMATCH_STATUS;
        }
        match self.mem(cx, true, |t| user::write(&mut t.space(), out, data)) {
            Ok(()) => status::SUCCESS,
            Err(_) => status::ACCESS_VIOLATION,
        }
    }

    /// A handle's object's name, as Windows' object manager gives it: a file's on its volume's device, a named
    /// object's in the session's BaseNamedObjects; empty for one without a name.
    fn object_name(&self, handle: &Handle) -> String {
        match handle {
            Handle::File(f) if f.path.starts_with("\\") => f.path.clone(),
            Handle::File(f) => {
                let rest = f.path.strip_prefix("C:").or_else(|| f.path.strip_prefix("c:")).unwrap_or(&f.path);
                format!("\\Device\\HarddiskVolume3{rest}")
            }
            Handle::Console(_) | Handle::Input => "\\Device\\ConDrv".to_string(),
            Handle::Object(o) => self.k.objects.name_of(o).map(|n| format!("\\Sessions\\1\\BaseNamedObjects\\{n}")).unwrap_or_default(),
            Handle::Section(s, _) => lock(&self.k.sections)
                .iter()
                .find(|(_, w)| w.upgrade().is_some_and(|x| Arc::ptr_eq(&x, s)))
                .map(|(n, _)| format!("\\Sessions\\1\\BaseNamedObjects\\{n}"))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// `NtQueryObject(handle, class, out, length, &returned)`: ObjectBasicInformation (0: its attributes and access),
    /// ObjectNameInformation (1: a UNICODE_STRING and the name after it) and ObjectTypeInformation (2: the type's
    /// name, number and access mask). An NTSTATUS.
    pub fn query_object(&mut self, cx: &mut Cx<'_>, handle: u64, class: u32, out: u64, length: u64, returned: u64) -> u32 {
        let found = {
            let state = lock(&self.process.state);
            state.handles.get(handle).map(|h| (h, state.handles.inheritable(handle).unwrap_or(false)))
        };
        let (handle, inherit) = match (handle, found) {
            (crate::handles::CURRENT_PROCESS, _) => (Handle::Object(self.process.object.clone()), false),
            (crate::handles::CURRENT_THREAD, _) => (Handle::Object(self.control.object.clone()), false),
            (_, Some(found)) => found,
            _ => return status::INVALID_HANDLE,
        };
        let (kind, access) = type_of(&handle);
        let w = self.abi().width as usize;
        // A UNICODE_STRING (lengths, then the buffer's pointer) at the start, its text after `fixed` bytes.
        let unicode = |fixed: usize, text: &str| -> Vec<u8> {
            let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut b = vec![0u8; fixed];
            let len = units.len() as u16;
            b[0..2].copy_from_slice(&len.to_le_bytes());
            b[2..4].copy_from_slice(&(len + 2).to_le_bytes());
            let at = out + fixed as u64;
            b[w..2 * w].copy_from_slice(&at.to_le_bytes()[..w]);
            b.extend(units);
            b.extend([0, 0]);
            b
        };
        let data = match class {
            0 => {
                let mut o = Out::new(56);
                o.u32(0, if inherit { OBJ_INHERIT } else { 0 });
                o.u32(4, access);
                o.u32(8, 1);
                o.u32(12, 1);
                o.0
            }
            1 => {
                let name = self.object_name(&handle);
                if name.is_empty() { vec![0u8; 2 * w] } else { unicode(2 * w, &name) }
            }
            2 => {
                let fixed = if w == 8 { 104 } else { 96 };
                let mut b = unicode(fixed, type_name(kind));
                let base = 2 * w;
                b[base + 48..base + 52].copy_from_slice(&0u32.to_le_bytes());
                b[base + 68..base + 72].copy_from_slice(&access.to_le_bytes());
                b[base + 74] = kind;
                b
            }
            _ => return 0xc000_0003, // STATUS_INVALID_INFO_CLASS
        };
        self.answer_info(cx, &data, out, length, returned)
    }
}

/// A handle's object type (by Windows 10's numbers) and the access it's given (all of its type's).
fn type_of(handle: &Handle) -> (u8, u32) {
    match handle {
        Handle::Object(object) => match object.kind {
            Kind::Process(_) => (7, 0x1f_ffff),
            Kind::Thread(_) => (8, 0x1f_ffff),
            Kind::Event { .. } => (16, 0x1f_0003),
            Kind::Mutant => (17, 0x1f_0001),
            Kind::Semaphore { .. } => (19, 0x1f_0003),
            Kind::Timer { .. } => (20, 0x1f_0003),
        },
        Handle::Port(_) => (36, 0x1f_0003),
        Handle::Token(_) => (5, 0xf_01ff),
        Handle::Console(_) | Handle::Input | Handle::File(_) | Handle::Find(_) | Handle::Snapshot(_) => (37, 0x12_019f),
        Handle::Section(_, access) => (42, *access),
        Handle::Key(_) => (44, 0xf_003f),
    }
}

/// An address for a handle's object, the same in every process with a handle to it, in the kernel's half.
fn object_address(handle: &Handle) -> u64 {
    let host = match handle {
        Handle::Object(o) => Arc::as_ptr(o) as u64,
        Handle::File(f) => Arc::as_ptr(f) as u64,
        Handle::Find(f) => Arc::as_ptr(f) as u64,
        Handle::Section(s, _) => Arc::as_ptr(s) as u64,
        Handle::Port(p) => Arc::as_ptr(p) as u64,
        Handle::Token(t) => Arc::as_ptr(t) as u64,
        Handle::Snapshot(s) => Arc::as_ptr(s) as u64,
        Handle::Key(id) => 0x10_0000 + id * 0x40,
        Handle::Console(host::Stream::Out) => 0x1000,
        Handle::Console(_) => 0x1040,
        Handle::Input => 0x1080,
    };
    SYSTEM_RANGE_START | 0x7000_0000_0000 | host & 0x0fff_ffff_fff0
}

/// `RTL_PROCESS_MODULES`: the kernel's modules, with their paths as `\SystemRoot\system32\NAME`.
fn modules() -> Vec<u8> {
    const EACH: usize = 0x128;
    let mut o = Out::new(8 + EACH * KERNEL_MODULES.len());
    o.u32(0, KERNEL_MODULES.len() as u32);
    for (i, (name, base, size)) in KERNEL_MODULES.iter().enumerate() {
        let at = 8 + EACH * i;
        o.u64(at + 0x10, *base);
        o.u32(at + 0x18, *size);
        o.u32(at + 0x1c, 0x0800_4000);
        o.u16(at + 0x20, i as u16);
        o.u16(at + 0x24, 1);
        let dir = "\\SystemRoot\\system32\\";
        o.u16(at + 0x26, dir.len() as u16);
        let path = format!("{dir}{name}");
        o.0[at + 0x28..at + 0x28 + path.len()].copy_from_slice(path.as_bytes());
    }
    o.0
}

/// `GetSystemTimes`' idle, kernel and user times: every processor's, idle all along (kernel time counts idle time).
pub fn times(task: &Task) -> (u64, u64, u64) {
    let all = task.uptime() * u64::from(task.cpus());
    (all, all, 0)
}
