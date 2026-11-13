//! Processes and their threads: starting a program, what happens at each stop, host exports, and memory.
//!
//! atlas maps the program and ntdll, lays out the PEB, TEB and stack, and starts the first thread in ntdll's
//! `AtlasProcessStart`, whose loader does the rest inside the program.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};

use atlas_abi::{call, status};
use exec::isa::x86::{R10, RCX, RDX};
use exec::{FpRegs, Isa, PAGE, Prot, Regs, SpaceId, Stop, Turn};
use kernels_core::call::Convention;
use kernels_core::lock;
use kernels_core::mm::{Region, Regions};
use kernels_core::thread::{Cx, Flow, Guest};
use kernels_core::user::{self, Fault};
use kernels_core::wait::Waiter;

use crate::Kernel;
use crate::abi::{self, Abi};
use crate::handles::{Handle, Handles};
use crate::exceptions::Raised;
use crate::images::pe::{Found, Pe, Wanted};
use crate::images::{self, Layout};
use crate::mapping::{map_image, map_ntdll, module_name, place};
use crate::objects::{self as objects_mod, Object, State};
use crate::os::{self, Kind, Reply};
use crate::threads::ThreadControl;

/// Windows' allocation granularity.
pub(crate) const GRANULE: u64 = 0x1_0000;
const STACK: u64 = 1 << 20;
const STACK_MAPPED: u64 = 64 << 10;
/// The guard pages at the bottom of every thread's stack: touched, they're the program's to handle its stack
/// overflow with.
pub(crate) const STACK_GUARD: u64 = 16 << 10;
/// PAGE_GUARD, a modifier of any protection.
pub(crate) const PAGE_GUARD: u32 = 0x100;

pub(crate) struct Process {
    pub pid: u64,
    /// Its program's ISA, which says how its structures are laid out.
    pub isa: Isa,
    pub state: Mutex<ProcState>,
    /// Set once its space is gone, under this lock, which other processes' threads hold while they write into it.
    pub remote: Mutex<bool>,
    /// Signalled when its last thread ends.
    pub object: Arc<Object>,
    /// Its exit code, once its last thread has ended.
    pub ended: (Mutex<Option<u32>>, Condvar),
}

pub(crate) struct ProcState {
    pub space: SpaceId,
    /// Where ntdll's exception dispatcher is, for exceptions atlas delivers, and where new threads start.
    pub dispatcher: u64,
    pub thread_start: u64,
    /// The PEB, which every TEB points at.
    pub peb: u64,
    /// Its threads, by ID.
    pub threads: BTreeMap<u64, Arc<ThreadControl>>,
    /// The images mapped, by module name: base and Windows path.
    pub mapped: std::collections::BTreeMap<String, (u64, String)>,
    /// The program's folder on the disk, where DLLs are looked for first.
    pub exe_dir: String,
    /// The current directory, as a full Windows path.
    pub cwd: String,
    pub regions: Regions<Area>,
    pub handles: Handles,
    /// Standard input, output and error.
    pub std_handles: [u64; 3],
    /// The command line, in the program's memory: as ANSI, and as UTF-16.
    pub command_line: (u64, u64),
    /// Set once the process is ending, with its exit code.
    pub exiting: Option<u32>,
    /// Its manifest says it supports Windows 10: version calls tell it the truth, not Windows 8.
    pub windows10: bool,
    /// The process that started it (0 for the first), when (a FILETIME), and its program's Windows path.
    pub parent: u64,
    pub created: u64,
    pub image: String,
    /// Its primary token: a copy of its parent's, or the user's.
    pub token: Arc<crate::security::Token>,
    /// `SetErrorMode`'s.
    pub error_mode: u32,
}

/// What the program was promised in a region.
#[derive(Debug, Clone)]
pub(crate) struct Area {
    pub prot: Prot,
    pub kind: AreaKind,
    /// Where the allocation it's part of starts (`VirtualQuery`'s `AllocationBase`), or 0 for the region's own start.
    pub base: u64,
    /// A view's section, and where in it this region starts.
    pub view: Option<crate::sections::View>,
    /// PAGE_GUARD: the first touch raises STATUS_GUARD_PAGE_VIOLATION (or, at the bottom of a thread's stack,
    /// STATUS_STACK_OVERFLOW), and takes the guard away. exec maps it with no access while it's guarded.
    pub guard: bool,
}

impl Area {
    pub fn of(prot: Prot, kind: AreaKind) -> Area {
        Area { prot, kind, base: 0, view: None, guard: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AreaKind {
    /// Committed memory: zeros until written.
    Committed,
    /// Reserved, not committed: any touch is an access violation.
    Reserved,
    /// An image's pages, mapped when it's loaded.
    Image,
    /// What the backend keeps for itself.
    Kept,
    /// A view of a section.
    View,
    /// A page of the system's own, the same in every process: the shared data page.
    System,
}

impl Region for Area {
    fn advanced(&self, by: u64) -> Area {
        let mut area = self.clone();
        if let Some(view) = &mut area.view {
            view.offset += by;
        }
        area
    }
}

/// One guest thread, as atlas runs it.
pub(crate) struct Task {
    pub k: Arc<Kernel>,
    pub process: Arc<Process>,
    pub tid: u64,
    pub waiter: Arc<Waiter>,
    pub isa: Isa,
    /// How deep in upcalls this thread is.
    pub depth: usize,
    /// Exceptions delivered since the program last carried on (`CONTINUE`).
    pub nested: u32,
    pub control: Arc<ThreadControl>,
    /// Set when this thread ends by itself, with its exit code.
    pub ending: Option<u32>,
}

impl Guest for Task {
    fn exit(&mut self, cx: &mut Cx<'_>, exit: exec::Exit) -> Flow {
        let stop = cx.turn(|t| t.stop(exit));
        let mut flow = self.handle(cx, stop);
        if flow == Flow::Run && !self.park_while_suspended() {
            flow = Flow::End;
        }
        if flow == Flow::End {
            self.finish_thread(cx);
        }
        flow
    }
}

impl Task {
    /// Where things are in this process's structures, by its ISA.
    pub fn abi(&self) -> Abi {
        Abi::of(self.isa)
    }

    /// Where things go in this process's space: `layout().alloc` is where `VirtualAlloc` finds room.
    pub fn layout(&self) -> Layout {
        images::layout(self.isa)
    }

    /// Writes a pointer-sized value: a pointer, a handle or a SIZE_T, as this process's ISA has them.
    pub fn write_ptr(&mut self, cx: &mut Cx<'_>, addr: u64, value: u64) -> Result<(), Fault> {
        let bytes = self.abi().ptr(value);
        self.mem(cx, true, |t| user::write(&mut t.space(), addr, &bytes))
    }

    /// The calling convention of this thread's ISA.
    pub fn convention(&self) -> Convention {
        match self.isa {
            Isa::Arm64 => Convention::Arm64,
            Isa::X86_64 => Convention::Win64,
            Isa::X86_32 => Convention::Stdcall,
        }
    }

    /// Deals with one stop. Also used while an upcall runs the program.
    pub fn handle(&mut self, cx: &mut Cx<'_>, stop: Option<Stop>) -> Flow {
        if self.ending() {
            return Flow::End;
        }
        match stop {
            None | Some(Stop::Preempted) => Flow::Run,
            Some(Stop::Syscall { nr, .. }) => self.call(cx, nr),
            Some(Stop::Fault { addr, access, pc }) => {
                if let Some(code) = self.guard_hit(cx, addr) {
                    let mut raised = Raised::access_violation(pc, addr, access);
                    raised.code = code;
                    return self.raise(cx, raised, &format!("a guard page at {addr:#x}, at {pc:#x}"));
                }
                if self.fault(cx, addr, access) {
                    Flow::Run
                } else {
                    self.raise(cx, Raised::access_violation(pc, addr, access), &format!("bad access to {addr:#x} ({access:?}) at {pc:#x}"))
                }
            }
            Some(Stop::Undefined { pc }) => self.raise(cx, Raised::simple(status::ILLEGAL_INSTRUCTION, pc), &format!("illegal instruction at {pc:#x}")),
            Some(Stop::Breakpoint { pc }) => self.raise(cx, Raised::simple(status::BREAKPOINT, pc), &format!("breakpoint at {pc:#x}")),
            Some(Stop::Alignment { pc }) => self.raise(cx, Raised::simple(status::DATATYPE_MISALIGNMENT, pc), &format!("misaligned at {pc:#x}")),
            Some(Stop::DivideError { pc }) => self.raise(cx, Raised::simple(status::INTEGER_DIVIDE_BY_ZERO, pc), &format!("divide by zero at {pc:#x}")),
            Some(Stop::FastFail { pc, code }) => self.die(status::STACK_BUFFER_OVERRUN, &format!("fast fail {code} at {pc:#x}")),
            Some(Stop::Internal { pc, detail }) => self.die(status::ILLEGAL_INSTRUCTION, &format!("exec couldn't run it at {pc:#x} ({detail:#x})")),
        }
    }

    /// Ends the process, saying why.
    pub fn die(&mut self, code: u32, what: &str) -> Flow {
        self.k.host.write(host::Stream::Err, format!("atlas: process {} ended with {code:#010x}: {what}\n", self.process.pid).as_bytes());
        self.end(code)
    }

    /// Ends the whole process.
    pub fn end(&mut self, code: u32) -> Flow {
        self.process.end(code);
        Flow::End
    }

    /// Ends this thread.
    pub fn end_thread(&mut self, code: u32) -> Flow {
        self.ending.get_or_insert(code);
        Flow::End
    }

    /// What a host export's reply means for the thread.
    pub fn reply(&mut self, cx: &mut Cx<'_>, reply: Reply) -> Flow {
        match reply {
            Reply::Value(v) => self.answer(cx, v),
            Reply::ExitProcess(code) => self.end(code),
            Reply::ExitThread(code) => self.end_thread(code),
            Reply::Ended => Flow::End,
        }
    }

    fn call(&mut self, cx: &mut Cx<'_>, nr: u64) -> Flow {
        let trace = self.k.options.trace;
        match nr {
            call::THREAD_RETURN => {
                let code = self.return_value(cx);
                self.end_thread(code as u32)
            }
            call::EXIT_THREAD => {
                let code = self.args(cx, 1)[0];
                self.end_thread(code as u32)
            }
            call::THREAD_STARTED => self.answer(cx, 0),
            call::WAIT_ADDRESS => {
                let a = self.args(cx, 4);
                match self.wait_address(cx, a[0], a[1], a[2], a[3]) {
                    Ok(code) => self.answer(cx, u64::from(code)),
                    Err(reply) => self.reply(cx, reply),
                }
            }
            call::WAKE_ADDRESS => {
                let a = self.args(cx, 2);
                self.k.futexes.wake(a[0], if a[1] != 0 { usize::MAX } else { 1 });
                self.answer(cx, 0)
            }
            call::UPCALL_RETURN => self.die(status::INVALID_PARAMETER, "returned to an upcall that isn't waiting"),
            call::TERMINATE_PROCESS => {
                let a = self.args(cx, 2);
                if a[0] == 0 {
                    self.end_other_threads(a[1] as u32);
                    return self.answer(cx, 0);
                }
                self.end(a[1] as u32)
            }
            call::LAST_THREAD => {
                let last = lock(&self.process.state).threads.len() <= 1;
                self.answer(cx, u64::from(last))
            }
            call::CREATE_THREAD => {
                let a = self.args(cx, 2);
                let made = self.create_thread(cx, 0, a[0], a[1], 0).is_ok();
                self.answer(cx, u64::from(made))
            }
            call::EXPORT_NUMBER => {
                let a = self.args(cx, 2);
                let name = self.mem(cx, false, |t| user::read_vec(&t.space(), a[0], a[1].min(64) as usize)).unwrap_or_default();
                let name = String::from_utf8_lossy(&name);
                let number = os::find("kernel32.dll")
                    .and_then(|dll| os::DLLS[dll].exports.iter().position(|e| e.name == name && matches!(e.kind, Kind::Host { .. })).map(|i| os::number(dll, i)))
                    .unwrap_or(0);
                self.answer(cx, number)
            }
            call::UNMAP_IMAGE => {
                let a = self.args(cx, 2);
                let code = self.unmap_image(cx, a[0], a[1]);
                self.answer(cx, u64::from(code))
            }
            call::ALLOCATE_VIRTUAL_MEMORY => {
                let a = self.args(cx, 4);
                let base = self.virtual_alloc(cx, a[0], a[1], a[2] as u32, a[3] as u32).unwrap_or(0);
                self.answer(cx, base)
            }
            call::MAP_IMAGE => {
                let a = self.args(cx, 4);
                let code = self.map_named(cx, a[0], a[1], a[2], a[3]);
                self.answer(cx, u64::from(code))
            }
            call::PROTECT_VIRTUAL_MEMORY => {
                let a = self.args(cx, 4);
                let code = self.virtual_protect(cx, a[0], a[1], a[2] as u32, a[3]);
                self.answer(cx, u64::from(code))
            }
            call::CONTINUE => {
                let at = self.args(cx, 1)[0];
                self.continue_from(cx, at)
            }
            call::FREE_VIRTUAL_MEMORY => {
                let a = self.args(cx, 3);
                let freed = self.virtual_free(cx, a[0], a[1], a[2] as u32);
                self.answer(cx, if freed { 0 } else { u64::from(status::INVALID_PARAMETER) })
            }
            call::QUERY_SYSTEM_INFORMATION => {
                let a = self.args(cx, 4);
                let code = self.query_system_information(cx, a[0] as u32, a[1], a[2], a[3]);
                self.answer(cx, u64::from(code))
            }
            call::DEBUG_PRINT => {
                let a = self.args(cx, 2);
                if let Ok(text) = self.mem(cx, false, |t| user::read_vec(&t.space(), a[0], a[1].min(4096) as usize)) {
                    self.k.host.write(host::Stream::Err, &text);
                }
                self.answer(cx, 0)
            }
            nr if nr >= call::HOST_EXPORTS => {
                let Some((dll, export)) = os::by_number(nr) else {
                    return self.die(status::INVALID_PARAMETER, &format!("no export has call number {nr:#x}"));
                };
                match export.kind {
                    Kind::Host { args, run } => {
                        let wide = if self.isa == Isa::X86_32 { images::x86_wide(export.name) } else { None };
                        let a = match wide {
                            Some((slots, join)) => join(&self.args(cx, slots)),
                            None => self.args(cx, args),
                        };
                        let reply = run(self, cx, &a);
                        // A 64-bit result's high half goes in edx on x86.
                        if let Reply::Value(v) = reply
                            && self.isa == Isa::X86_32
                            && images::x86_returns_wide(export.name)
                        {
                            cx.turn(|t| {
                                let mut regs = t.thread().regs();
                                if let Regs::X86(r) = &mut regs {
                                    r.gpr[RDX] = v >> 32;
                                }
                                t.thread().set_regs(&regs);
                            });
                        }
                        if trace {
                            let shown = match &reply {
                                Reply::Value(v) => format!("{v:#x}"),
                                Reply::ExitProcess(code) => format!("(the process exits, {code})"),
                                Reply::ExitThread(code) => format!("(the thread exits, {code})"),
                                Reply::Ended => "(ending)".to_string(),
                            };
                            self.k.host.write(host::Stream::Err, format!("[{}] {}!{}({a:x?}) = {shown}\n", self.tid, dll.name, export.name).as_bytes());
                        }
                        if !lock(&self.k.deferred).is_empty() {
                            let k = self.k.clone();
                            k.drain(cx);
                        }
                        // What a registry function changed, for the watches on it.
                        if dll.name == "ADVAPI32.dll" {
                            self.k.registry_changed();
                        }
                        self.reply(cx, reply)
                    }
                    Kind::Guest(target) => {
                        self.die(status::NOT_IMPLEMENTED, &format!("{}!{} is guest code (ntdll!{target}), and atlas-guest isn't built yet", dll.name, export.name))
                    }
                    Kind::Missing => self.die(status::NOT_IMPLEMENTED, &format!("{}!{} isn't written yet", dll.name, export.name)),
                }
            }
            _ => self.answer(cx, u64::from(status::NOT_IMPLEMENTED)),
        }
    }

    /// Sets the call's result, and carries on. A 32-bit program gets 32 bits of it: -1 is 0xffffffff, as it'd have
    /// it, and the top half of its registers stays clear.
    fn answer(&mut self, cx: &mut Cx<'_>, value: u64) -> Flow {
        let value = if self.isa == Isa::X86_32 { value & 0xffff_ffff } else { value };
        cx.turn(|t| t.thread().set_return(value));
        if self.ending() { Flow::End } else { Flow::Run }
    }

    /// `WAIT_ADDRESS`: waits while the `size` bytes at `addr` are what's at `compare`, up to `timeout_ms`. 0 when
    /// woken (or it had changed), STATUS_TIMEOUT when the time's up.
    fn wait_address(&mut self, cx: &mut Cx<'_>, addr: u64, compare: u64, size: u64, timeout_ms: u64) -> Result<u32, Reply> {
        let size = size.clamp(1, 8) as usize;
        let Ok(expected) = self.mem(cx, false, |t| user::read_vec(&t.space(), compare, size)) else { return Ok(status::ACCESS_VIOLATION) };
        // INFINITE is u32::MAX, however the argument was widened.
        let deadline = (timeout_ms as u32 != u32::MAX).then(|| std::time::Instant::now() + std::time::Duration::from_millis(u64::from(timeout_ms as u32)));
        let waiter = self.waiter.clone();
        let k = self.k.clone();
        let still = || self.mem(cx, false, |t| user::read_vec(&t.space(), addr, size)).is_ok_and(|now| now == expected);
        match k.futexes.wait(addr, &waiter, still, deadline) {
            Err(kernels_core::wait::Changed) | Ok(kernels_core::wait::Woke::Woken) => Ok(0),
            Ok(kernels_core::wait::Woke::TimedOut) => Ok(0x102),
            Ok(kernels_core::wait::Woke::Interrupted) if self.ending() => Err(Reply::Ended),
            Ok(kernels_core::wait::Woke::Interrupted) => Ok(0),
        }
    }

    /// What a function returned to one of the atlas page's stubs: `x0`, or `rax` (moved to `r10`), or on x86 `eax`
    /// (moved to `edx`).
    pub fn return_value(&mut self, cx: &mut Cx<'_>) -> u64 {
        let isa = self.isa;
        match cx.turn(|t| t.thread().regs()) {
            Regs::Arm64(r) => r.x[0],
            Regs::X86(r) if isa == Isa::X86_32 => r.gpr[RDX] & 0xffff_ffff,
            Regs::X86(r) => r.gpr[R10],
        }
    }

    /// The first `count` arguments of the call that trapped, by this ISA's convention. x86-64's stubs keep the first
    /// in `r10`, as Windows' own do, since `syscall` takes `rcx`. x86's are on the stack, 32 bits each: -1 to -16
    /// are widened as negative numbers, as WoW64 widens a handle, so the pseudo-handles (`GetCurrentProcess()`'s -1)
    /// and INVALID_HANDLE_VALUE are what they are for a 64-bit program; everything else is widened with zeros.
    fn args(&mut self, cx: &mut Cx<'_>, count: usize) -> Vec<u64> {
        let mut regs = cx.turn(|t| t.thread().regs());
        if let Regs::X86(r) = &mut regs
            && self.isa == Isa::X86_64
        {
            r.gpr[RCX] = r.gpr[R10];
        }
        let convention = self.convention();
        let width = convention.width();
        let x32 = self.isa == Isa::X86_32;
        (0..count)
            .map(|i| {
                let mut stack = |addr: u64| self.mem(cx, false, |t| user::read_ptr(&t.space(), addr, width)).ok();
                let v = convention.arg(&regs, i, &mut stack).unwrap_or(0);
                if x32 && v >= 0xffff_fff0 { v | 0xffff_ffff_0000_0000 } else { v }
            })
            .collect()
    }

    /// Copies in or out of the program's memory in a turn, committing pages it may use as its own touch would.
    pub fn mem<R>(&mut self, cx: &mut Cx<'_>, write: bool, copy: impl FnMut(&mut Turn<'_>) -> Result<R, Fault>) -> Result<R, Fault> {
        user::with_faults(cx, write, |cx, addr, access| self.fault(cx, addr, access), copy)
    }

    /// A touch of a guard page: it isn't one any more, and the exception it raises. At the bottom of this thread's
    /// stack, it's a stack overflow, and every guard page left there goes too, for the handler to run in.
    fn guard_hit(&mut self, cx: &mut Cx<'_>, addr: u64) -> Option<u32> {
        let page = addr & !(PAGE - 1);
        let (range, prot) = {
            let state = lock(&self.process.state);
            let (_, area) = state.regions.find(addr)?;
            if !area.guard {
                return None;
            }
            (page..page + PAGE, area.prot)
        };
        let (teb_at, abi) = (self.control.teb, self.abi());
        let stack = self
            .mem(cx, false, |t| Ok((user::read_ptr(&t.space(), teb_at + abi.teb.deallocation_stack, abi.width)?, user::read_ptr(&t.space(), teb_at + abi.teb.stack_base, abi.width)?)))
            .ok();
        let (range, code) = match stack {
            Some((bottom, top)) if bottom != 0 && addr >= bottom && addr < top => (bottom..range.end, status::STACK_OVERFLOW),
            _ => (range, status::GUARD_PAGE_VIOLATION),
        };
        lock(&self.process.state).regions.update(range.clone(), |a| a.guard = false);
        cx.turn(|t| t.space().protect(range.clone(), prot));
        if code == status::STACK_OVERFLOW {
            // As Windows has it, the stack's limit is the lowest page it may use now.
            let _ = self.write_ptr(cx, teb_at + abi.teb.stack_limit, range.start);
        }
        Some(code)
    }

    /// The program touched a page exec hasn't mapped: zeros, if it's committed memory it may touch that way.
    fn fault(&mut self, cx: &mut Cx<'_>, addr: u64, access: Prot) -> bool {
        fault_in(&self.k, &self.process, cx, addr, access)
    }

    /// `VirtualAlloc`: reserves, commits, or both. Gives the base, or None.
    pub fn virtual_alloc(&mut self, cx: &mut Cx<'_>, addr: u64, size: u64, kind: u32, protect: u32) -> Option<u64> {
        const MEM_COMMIT: u32 = 0x1000;
        const MEM_RESERVE: u32 = 0x2000;
        let prot = prot_from(protect)?;
        if size == 0 {
            return None;
        }
        let alloc = self.layout().alloc;
        let mut state = lock(&self.process.state);
        let commit = kind & MEM_COMMIT != 0;
        let guard = commit && protect & PAGE_GUARD != 0;
        let area = Area { guard, ..Area::of(prot, if commit { AreaKind::Committed } else { AreaKind::Reserved }) };
        if addr == 0 || kind & MEM_RESERVE != 0 {
            let size = size.next_multiple_of(GRANULE);
            let base = if addr == 0 {
                state.regions.free_above(alloc, size, GRANULE)?
            } else {
                let base = addr & !(GRANULE - 1);
                if !state.regions.is_free(base..base + size) {
                    return None;
                }
                base
            };
            state.regions.insert(base..base + size, Area { base, ..area });
            return Some(base);
        }
        // Committing part of what's reserved.
        let range = addr & !(PAGE - 1)..(addr + size).next_multiple_of(PAGE);
        let mut at = range.start;
        while at < range.end {
            let (r, a) = state.regions.find(at)?;
            if matches!(a.kind, AreaKind::Kept | AreaKind::Image | AreaKind::View | AreaKind::System) {
                return None;
            }
            at = r.end;
        }
        state.regions.update(range.clone(), |a| {
            a.prot = area.prot;
            a.kind = area.kind;
            a.guard = guard;
        });
        cx.turn(|t| t.space().protect(range.clone(), if guard { Prot::NONE } else { prot }));
        Some(range.start)
    }

    /// `VirtualFree`: decommits, or releases a whole reservation.
    pub fn virtual_free(&mut self, cx: &mut Cx<'_>, addr: u64, size: u64, kind: u32) -> bool {
        const MEM_DECOMMIT: u32 = 0x4000;
        const MEM_RELEASE: u32 = 0x8000;
        let mut state = lock(&self.process.state);
        let Some((range, area)) = state.regions.find(addr).map(|(r, a)| (r, a.clone())) else { return false };
        if matches!(area.kind, AreaKind::Kept | AreaKind::Image | AreaKind::View | AreaKind::System) {
            return false;
        }
        let range = match kind {
            MEM_RELEASE if size == 0 => {
                state.regions.remove(range.clone());
                range
            }
            MEM_DECOMMIT => {
                let range = addr & !(PAGE - 1)..(addr + size).next_multiple_of(PAGE);
                state.regions.update(range.clone(), |a| a.kind = AreaKind::Reserved);
                range
            }
            _ => return false,
        };
        cx.turn(|t| t.space().unmap(range));
        true
    }
}

/// A page of `process` that isn't mapped, touched by `access`: brought in if the process may touch it so (committed
/// memory as zeros, a view's page of its section), in whichever space the turns of `cx` are in, the process's own or
/// (from another process's thread, completing its I/O) through `Cx::elsewhere`.
pub(crate) fn fault_in(k: &Kernel, process: &Process, cx: &mut Cx<'_>, addr: u64, access: Prot) -> bool {
    let page = addr & !(PAGE - 1);
    let state = lock(&process.state);
    let Some((range, area)) = state.regions.find(addr) else { return false };
    if area.kind == AreaKind::View && area.prot.contains(access) {
        let Some(mut view) = area.view.clone() else { return false };
        view.offset += page - range.start;
        let prot = area.prot;
        drop(state);
        // A copy-on-write page that's there already, written: this view's own copy now.
        let there = cx.turn(|t| t.space().page(page));
        if let Some(present) = there {
            if access.write() && present.shared {
                return cx.turn(|t| t.space().make_writable(page, prot).is_ok());
            }
            return present.prot.contains(access);
        }
        return crate::sections::fault_view(k, cx, &view, page, prot, access.write());
    }
    if area.kind != AreaKind::Committed || area.guard || !area.prot.contains(access) {
        return false;
    }
    let prot = area.prot;
    cx.turn(|t| match t.space().page(page) {
        Some(present) if access.write() && present.shared => t.space().make_writable(page, prot).is_ok(),
        // Another thread touched it first, and it's there now: just run again.
        Some(present) => present.prot.contains(access),
        None => t.space().map(page..page + PAGE, prot).zeroed().is_ok(),
    })
}

/// Writes into another process's memory (or this one's), bringing its pages in as its own touch would: how one thread
/// completes I/O another process is waiting for. False if the process has ended or the memory isn't writable.
pub(crate) fn write_remote(k: &Kernel, cx: &mut Cx<'_>, process: &Process, writes: &[(u64, &[u8])]) -> bool {
    // Its space stays while this is held: it goes only once `gone` is set, under the same lock.
    let gone = lock(&process.remote);
    if *gone {
        return false;
    }
    let space = lock(&process.state).space;
    cx.elsewhere(space, |cx| {
        writes.iter().all(|(at, data)| user::with_faults(cx, true, |cx, a, access| fault_in(k, process, cx, a, access), |t| user::write(&mut t.space(), *at, data)).is_ok())
    })
}

/// A `PAGE_*` protection, or None for one atlas doesn't take.
pub(crate) fn prot_from(protect: u32) -> Option<Prot> {
    Some(match protect & 0xff {
        0x01 => Prot::NONE,
        0x02 => Prot::R,
        0x04 | 0x08 => Prot::RW,
        0x10 | 0x20 => Prot::RX,
        0x40 | 0x80 => Prot::RWX,
        _ => return None,
    })
}

/// A Windows path on the machine's disk: `C:\Users\me\app.exe` is `/Users/me/app.exe`.
pub(crate) fn disk_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("C:").or_else(|| path.strip_prefix("c:")).unwrap_or(&path);
    if path.starts_with('/') { path.to_string() } else { format!("/{path}") }
}

/// A command line, as Windows programs parse it back: each argument quoted if it has spaces.
fn command_line(argv: &[String]) -> String {
    argv.iter().map(|a| if a.contains(' ') || a.is_empty() { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// Windows starts every x86 thread with x87 at double precision (control word 0x27F), where exec's fresh thread has
/// `fninit`'s extended precision (0x37F), as Linux's do.
pub(crate) fn windows_fp(t: &mut Turn<'_>, isa: Isa) {
    if let (true, FpRegs::X86(mut fp)) = (isa.is_x86(), t.thread().fp()) {
        fp.fxsave[0..2].copy_from_slice(&0x27Fu16.to_le_bytes());
        t.thread().set_fp(&FpRegs::X86(fp));
    }
}

/// What a new process starts with: its program, command line, environment, current directory and handles.
pub(crate) struct Launch {
    /// The program, on the disk (`/app.exe`), and its bytes.
    pub path: String,
    pub bytes: Vec<u8>,
    pub command_line: String,
    /// Its environment block: UTF-16 `NAME=VALUE` strings, each with its NUL, then a NUL.
    pub environment: Vec<u8>,
    /// Its current directory, as a full Windows path; else the program's folder.
    pub cwd: Option<String>,
    pub handles: Handles,
    pub std_handles: [u64; 3],
    /// Made with `CREATE_SUSPENDED`: its first thread waits for `ResumeThread`.
    pub suspended: bool,
    /// The process starting it, or 0.
    pub parent: u64,
}

/// Starts process 1: `argv[0]` from the disk, loaded with its system DLLs, with the host's console as its standard
/// handles and the environment a fresh logon gives.
pub(crate) fn start(k: &Arc<Kernel>, cx: &mut Cx<'_>, argv: &[String]) -> Result<Task, String> {
    let path = disk_path(argv.first().ok_or("nothing to run")?);
    let bytes = {
        let disk = lock(&k.disk);
        let ino = disk.resolve(layers::ROOT, &path, true).map_err(|e| format!("{path}: {e}"))?;
        disk.contents(ino).map_err(|e| format!("{path}: {e}"))?.to_vec()
    };
    let isa = Pe::parse(&bytes).map_err(|_| format!("{path} isn't a Windows program"))?.isa;
    let mut handles = Handles::default();
    let std_handles = [handles.add(Handle::Input), handles.add(Handle::Console(host::Stream::Out)), handles.add(Handle::Console(host::Stream::Err))];
    let environment = crate::environment::block(&crate::environment::first(k, isa));
    launch(k, cx, Launch { path, bytes, command_line: command_line(argv), environment, cwd: None, handles, std_handles, suspended: false, parent: 0 })
}

/// Starts a process on this host thread (whose CPU becomes its first thread's), giving that thread's task to run.
pub(crate) fn launch(k: &Arc<Kernel>, cx: &mut Cx<'_>, launch: Launch) -> Result<Task, String> {
    let Launch { path, bytes, command_line: line, environment, cwd, handles, std_handles, suspended, parent } = launch;
    let pe = Pe::parse(&bytes).map_err(|_| format!("{path} isn't a Windows program"))?;
    if pe.is_dll {
        return Err(format!("{path} is a DLL"));
    }
    let isa = pe.isa;
    // The variables that say what kind of program it is are its own, whatever its parent's said.
    let mut vars = crate::environment::parse(&environment);
    crate::environment::for_isa(&mut vars, isa);
    let environment = crate::environment::block(&vars);
    let mut image = pe.layout().map_err(|_| format!("{path}: its sections don't fit its image"))?;
    let windows_path = format!("C:{}", path.replace('/', "\\"));
    let pid = k.new_id();
    let tid = k.new_id();
    // Whether its manifest says it knows Windows 10: what `GetVersionEx` tells it.
    // A copy of its parent's token, so a change to either is its own.
    let token = Arc::new(k.process(parent).map_or_else(crate::security::Token::administrator, |p| {
        let token = lock(&p.state).token.clone();
        token.duplicate(1, 0)
    }));
    let windows10 = pe.resource(&image, 24).is_some_and(|m| String::from_utf8_lossy(m).to_ascii_lowercase().contains(crate::system::WINDOWS_10_GUID));

    let mut regions: Regions<Area> = Regions::default();
    let (abi, places) = (Abi::of(isa), images::layout(isa));
    let alloc = places.alloc.clone();
    let started = cx
        .turn(|t| -> Result<_, String> {
            let space = t.new_space(isa).map_err(|e| format!("exec can't run {isa:?} programs here: {e:?}"))?;
            t.use_space(space);
            let layout = t.space().layout();
            for kept in &layout.kept {
                regions.insert(kept.clone(), Area::of(Prot::NONE, AreaKind::Kept));
            }

            // The program, and ntdll, whose loader does the rest.
            let base = place(&regions, &pe, &mut image)?;
            map_image(t, &mut regions, &pe, base, &image)?;
            let (ntdll, ntdll_image) = map_ntdll(t, &mut regions, isa)?;
            let ntdll_pe = Pe::parse(&ntdll_image).map_err(|_| "ntdll doesn't read back")?;
            let export = |name: &str| match ntdll_pe.export(&ntdll_image, &Wanted::Name(name.to_string())) {
                Ok(Some(Found::Rva(rva))) => Ok(ntdll + u64::from(rva)),
                _ => Err(format!("ntdll has no {name}")),
            };
            let (process_start, dispatcher, thread_start) = (export("AtlasProcessStart")?, export("AtlasDispatchException")?, export("AtlasThreadStart")?);

            // The atlas page.
            // Where programs reach atlas through a gate rather than `syscall`, the stubs atlas makes do too.
            let gate = t.space().layout().gate;
            k.images.use_gate(gate.is_some());
            let atlas_page = places.atlas_page;
            t.space().map(atlas_page..atlas_page + PAGE, Prot::RX).with(&images::atlas_page(isa, gate)).map_err(|_| "no memory")?;
            regions.insert(atlas_page..atlas_page + PAGE, Area::of(Prot::RX, AreaKind::Image));

            // The shared data page, read-only, where Windows has it (where the host leaves that address free).
            let shared = crate::system::SHARED_DATA;
            if let Some(frame) = k.shared.get().and_then(|s| s.frame(isa))
                && regions.is_free(shared..shared + PAGE)
            {
                t.space().map(shared..shared + PAGE, Prot::R).frame(frame).map_err(|_| "no memory for the shared data page")?;
                regions.insert(shared..shared + PAGE, Area::of(Prot::R, AreaKind::System));
            }

            // The PEB, its process parameters and strings, and the TEB, in one committed block.
            let block = regions.free_above(alloc.clone(), GRANULE * 2, GRANULE).ok_or("no room")?;
            regions.insert(block..block + GRANULE * 2, Area::of(Prot::RW, AreaKind::Committed));
            t.space().map(block..block + GRANULE * 2, Prot::RW).zeroed().map_err(|_| "no memory")?;
            let (peb_at, params_at, mut teb_at, strings_at) = (block, block + 0x1000, block + 0x2000, block + 0x4000);
            // Where the host decides the thread's block goes (Windows' sandbox: the stub thread's own TEB, since
            // Windows owns gs), the TEB goes there, and the host copies it in and out around each run.
            if let Some(at) = t.space().thread_block(0x2000) {
                regions.insert(at..at + 0x2000, Area::of(Prot::RW, AreaKind::Committed));
                t.space().map(at..at + 0x2000, Prot::RW).zeroed().map_err(|_| "no memory for the thread block")?;
                teb_at = at;
            }
            let wide = utf16z(&line);
            let ansi: Vec<u8> = crate::ansi::bytes(&line).into_iter().chain([0]).collect();
            let image_path = utf16z(&windows_path);
            let (wide_at, ansi_at) = (strings_at, strings_at + wide.len() as u64 + 16);
            let path_at = ansi_at + ansi.len() as u64 + 16;
            let env_at = (path_at + image_path.len() as u64 + 16).next_multiple_of(16);
            if env_at + environment.len() as u64 > block + GRANULE * 2 {
                return Err("the environment is too big".to_string());
            }
            let (peb, params) = (abi.peb, abi.params);
            let mut memory = t.space();
            let mut put = |at: u64, bytes: &[u8]| user::write(&mut memory, at, bytes).map_err(|_| "writing the PEB".to_string());
            put(wide_at, &wide)?;
            put(ansi_at, &ansi)?;
            put(path_at, &image_path)?;
            put(peb_at + peb.image_base, &abi.ptr(base))?;
            put(peb_at + peb.number_of_processors, &(k.host.cpus() as u32).to_le_bytes())?;
            put(peb_at + peb.os_major_version, &crate::system::MAJOR.to_le_bytes())?;
            put(peb_at + peb.os_minor_version, &crate::system::MINOR.to_le_bytes())?;
            put(peb_at + peb.os_build_number, &(crate::system::BUILD as u16).to_le_bytes())?;
            put(peb_at + peb.os_platform_id, &2u32.to_le_bytes())?;
            let (subsystem, subsystem_major, subsystem_minor) = pe.subsystem();
            put(peb_at + peb.image_subsystem, &u32::from(subsystem).to_le_bytes())?;
            put(peb_at + peb.image_subsystem_major, &u32::from(subsystem_major).to_le_bytes())?;
            put(peb_at + peb.image_subsystem_minor, &u32::from(subsystem_minor).to_le_bytes())?;
            put(peb_at + peb.process_parameters, &abi.ptr(params_at))?;
            put(params_at, &(params.size as u32).to_le_bytes())?;
            for (i, at) in [params.standard_input, params.standard_output, params.standard_error].into_iter().enumerate() {
                put(params_at + at, &abi.ptr(std_handles[i]))?;
            }
            put(params_at + params.image_path_name, &abi.unicode_string(image_path.len(), path_at))?;
            put(params_at + params.command_line, &abi.unicode_string(wide.len(), wide_at))?;
            put(env_at, &environment)?;
            put(params_at + params.environment, &abi.ptr(env_at))?;
            put(params_at + params.environment_size, &abi.ptr(environment.len() as u64))?;

            // The stack.
            let stack = regions.free_above(alloc.clone(), STACK, GRANULE).ok_or("no room for the stack")?;
            let stack_top = stack + STACK;
            regions.insert(stack..stack_top, Area::of(Prot::RW, AreaKind::Committed));
            regions.update(stack..stack + STACK_GUARD, |a| a.guard = true);
            t.space().map(stack_top - STACK_MAPPED..stack_top, Prot::RW).zeroed().map_err(|_| "no memory")?;
            let mut memory = t.space();
            for (at, bytes) in abi.teb_fields(teb_at, stack_top, stack_top - STACK_MAPPED, stack, pid, tid, peb_at) {
                user::write(&mut memory, teb_at + at, &bytes).map_err(|_| "writing the TEB".to_string())?;
            }

            // The first thread: ntdll's process start, which never returns, with the TEB where Windows keeps it (x18
            // on ARM64, gs on x86-64, fs on x86).
            let entry = process_start;
            t.thread().start(entry, stack_top);
            windows_fp(t, isa);
            let mut regs = t.thread().regs();
            let convention = match isa {
                Isa::Arm64 => Convention::Arm64,
                Isa::X86_64 => Convention::Win64,
                Isa::X86_32 => Convention::Stdcall,
            };
            let (sp, frame) = convention.prepare(&mut regs, entry, &[peb_at, ntdll], places.thread_return());
            user::write(&mut t.space(), sp, &frame).map_err(|_| "writing the first frame")?;
            abi::set_teb(&mut regs, isa, teb_at);
            t.thread().set_regs(&regs);
            Ok((space, base, (ansi_at, wide_at), ntdll, (dispatcher, thread_start), (peb_at, teb_at)))
        })
        .map_err(|e| format!("starting {path}: {e}"))?;
    let (space, image_base, command, ntdll, (dispatcher, thread_start), (peb, teb_at)) = started;

    let mut mapped = std::collections::BTreeMap::new();
    mapped.insert(module_name(&path), (image_base, windows_path.clone()));
    mapped.insert("ntdll.dll".to_string(), (ntdll, "C:\\Windows\\System32\\ntdll.dll".to_string()));
    let exe_dir = path.rsplit_once('/').map(|(dir, _)| dir.to_string()).unwrap_or_default();
    let waiter = Waiter::new();
    let control = ThreadControl::new(tid, teb_at, waiter.clone(), cx.handle(), suspended);
    let mut threads = BTreeMap::new();
    threads.insert(tid, control.clone());
    let cwd = cwd.unwrap_or_else(|| format!("C:\\{}", exe_dir.trim_start_matches('/').replace('/', "\\")));
    let state = ProcState { space, dispatcher, thread_start, peb, threads, mapped, exe_dir, cwd, regions, handles, std_handles, command_line: command, exiting: None, windows10, parent, created: crate::files::filetime(k.host.wall()), image: windows_path.clone(), token, error_mode: 0 };
    let object = Object::new(objects_mod::Kind::Process(pid), State::default());
    let process = Arc::new(Process { pid, isa, state: Mutex::new(state), remote: Mutex::new(false), object, ended: (Mutex::new(None), Condvar::new()) });
    k.started(&process);
    Ok(Task { k: k.clone(), process, tid, waiter, isa, depth: 0, nested: 0, control, ending: None })
}

