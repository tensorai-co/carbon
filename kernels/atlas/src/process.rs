//! Processes and their threads: starting a program, what happens at each stop, host exports, and memory.
//!
//! atlas maps the program and ntdll, lays out the PEB, TEB and stack, and starts the first thread in ntdll's
//! `AtlasProcessStart`, whose loader does the rest inside the program.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};

use atlas_abi::{call, params, peb, status, teb};
use exec::isa::x86::{R10, RCX};
use exec::{FpRegs, Isa, PAGE, Prot, Regs, SpaceId, Stop, Turn};
use kernels_core::call::Convention;
use kernels_core::lock;
use kernels_core::mm::{Region, Regions};
use kernels_core::thread::{Cx, Flow, Guest};
use kernels_core::user::{self, Fault};
use kernels_core::wait::Waiter;

use crate::Kernel;
use crate::handles::{Handle, Handles};
use crate::exceptions::Raised;
use crate::images::pe::{Found, Pe, Wanted};
use crate::images::{self, ATLAS_PAGE, THREAD_RETURN_AT};
use crate::mapping::{map_image, map_ntdll, module_name, place};
use crate::objects::{self as objects_mod, Object, State};
use crate::os::{self, Kind, Reply};
use crate::threads::ThreadControl;

/// Where `VirtualAlloc` finds room, bottom up as on Windows: above the macOS sandbox's empty bottom 4 GiB.
pub(crate) const ALLOC: std::ops::Range<u64> = 0x1_0000_0000..images::SYSTEM_BASE - (1 << 30);
/// Windows' allocation granularity.
pub(crate) const GRANULE: u64 = 0x1_0000;
const STACK: u64 = 1 << 20;
const STACK_MAPPED: u64 = 64 << 10;

pub(crate) struct Process {
    pub pid: u64,
    pub state: Mutex<ProcState>,
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
}

/// What the program was promised in a region.
#[derive(Debug, Clone)]
pub(crate) struct Area {
    pub prot: Prot,
    pub kind: AreaKind,
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
}

impl Region for Area {
    fn advanced(&self, _: u64) -> Area {
        self.clone()
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
                let code = self.args(cx, 2)[1];
                self.end(code as u32)
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
                        let a = self.args(cx, args);
                        let reply = run(self, cx, &a);
                        if trace {
                            let shown = match &reply {
                                Reply::Value(v) => format!("{v:#x}"),
                                Reply::ExitProcess(code) => format!("(the process exits, {code})"),
                                Reply::ExitThread(code) => format!("(the thread exits, {code})"),
                                Reply::Ended => "(ending)".to_string(),
                            };
                            self.k.host.write(host::Stream::Err, format!("[{}] {}!{}({a:x?}) = {shown}\n", self.tid, dll.name, export.name).as_bytes());
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

    /// Sets the call's result, and carries on.
    fn answer(&mut self, cx: &mut Cx<'_>, value: u64) -> Flow {
        cx.turn(|t| t.thread().set_return(value));
        if self.ending() { Flow::End } else { Flow::Run }
    }

    /// `WAIT_ADDRESS`: waits while the `size` bytes at `addr` are what's at `compare`, up to `timeout_ms`. 0 when
    /// woken (or it had changed), STATUS_TIMEOUT when the time's up.
    fn wait_address(&mut self, cx: &mut Cx<'_>, addr: u64, compare: u64, size: u64, timeout_ms: u64) -> Result<u32, Reply> {
        let size = size.clamp(1, 8) as usize;
        let Ok(expected) = self.mem(cx, false, |t| user::read_vec(&t.space(), compare, size)) else { return Ok(status::ACCESS_VIOLATION) };
        let deadline = (timeout_ms != u64::from(u32::MAX)).then(|| std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms));
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

    /// What a function returned to one of the atlas page's stubs: `x0`, or `rax` (moved to `r10`).
    pub fn return_value(&mut self, cx: &mut Cx<'_>) -> u64 {
        match cx.turn(|t| t.thread().regs()) {
            Regs::Arm64(r) => r.x[0],
            Regs::X86(r) => r.gpr[R10],
        }
    }

    /// The first `count` arguments of the call that trapped, by this ISA's convention. x86-64's stubs keep the first
    /// in `r10`, as Windows' own do, since `syscall` takes `rcx`.
    fn args(&mut self, cx: &mut Cx<'_>, count: usize) -> Vec<u64> {
        let mut regs = cx.turn(|t| t.thread().regs());
        if let Regs::X86(r) = &mut regs {
            r.gpr[RCX] = r.gpr[R10];
        }
        let convention = self.convention();
        let width = convention.width();
        (0..count)
            .map(|i| {
                let mut stack = |addr: u64| self.mem(cx, false, |t| user::read_ptr(&t.space(), addr, width)).ok();
                convention.arg(&regs, i, &mut stack).unwrap_or(0)
            })
            .collect()
    }

    /// Copies in or out of the program's memory in a turn, committing pages it may use as its own touch would.
    pub fn mem<R>(&mut self, cx: &mut Cx<'_>, write: bool, copy: impl FnMut(&mut Turn<'_>) -> Result<R, Fault>) -> Result<R, Fault> {
        user::with_faults(cx, write, |cx, addr, access| self.fault(cx, addr, access), copy)
    }

    /// The program touched a page exec hasn't mapped: zeros, if it's committed memory it may touch that way.
    fn fault(&mut self, cx: &mut Cx<'_>, addr: u64, access: Prot) -> bool {
        let page = addr & !(PAGE - 1);
        let state = lock(&self.process.state);
        let Some((_, area)) = state.regions.find(addr) else { return false };
        if area.kind != AreaKind::Committed || !area.prot.contains(access) {
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

    /// `VirtualAlloc`: reserves, commits, or both. Gives the base, or None.
    pub fn virtual_alloc(&mut self, cx: &mut Cx<'_>, addr: u64, size: u64, kind: u32, protect: u32) -> Option<u64> {
        const MEM_COMMIT: u32 = 0x1000;
        const MEM_RESERVE: u32 = 0x2000;
        let prot = prot_from(protect)?;
        if size == 0 {
            return None;
        }
        let mut state = lock(&self.process.state);
        let commit = kind & MEM_COMMIT != 0;
        let area = Area { prot, kind: if commit { AreaKind::Committed } else { AreaKind::Reserved } };
        if addr == 0 || kind & MEM_RESERVE != 0 {
            let size = size.next_multiple_of(GRANULE);
            let base = if addr == 0 {
                state.regions.free_above(ALLOC, size, GRANULE)?
            } else {
                let base = addr & !(GRANULE - 1);
                if !state.regions.is_free(base..base + size) {
                    return None;
                }
                base
            };
            state.regions.insert(base..base + size, area);
            return Some(base);
        }
        // Committing part of what's reserved.
        let range = addr & !(PAGE - 1)..(addr + size).next_multiple_of(PAGE);
        let mut at = range.start;
        while at < range.end {
            let (r, a) = state.regions.find(at)?;
            if matches!(a.kind, AreaKind::Kept | AreaKind::Image) {
                return None;
            }
            at = r.end;
        }
        state.regions.update(range.clone(), |a| *a = area.clone());
        cx.turn(|t| t.space().protect(range.clone(), prot));
        Some(range.start)
    }

    /// `VirtualFree`: decommits, or releases a whole reservation.
    pub fn virtual_free(&mut self, cx: &mut Cx<'_>, addr: u64, size: u64, kind: u32) -> bool {
        const MEM_DECOMMIT: u32 = 0x4000;
        const MEM_RELEASE: u32 = 0x8000;
        let mut state = lock(&self.process.state);
        let Some((range, area)) = state.regions.find(addr).map(|(r, a)| (r, a.clone())) else { return false };
        if matches!(area.kind, AreaKind::Kept | AreaKind::Image) {
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

/// Starts process 1: `argv[0]` from the disk, loaded with its system DLLs, with the host's console as its standard
/// handles.
/// Windows starts every x86 thread with x87 at double precision (control word 0x27F), where exec's fresh thread has
/// `fninit`'s extended precision (0x37F), as Linux's do.
pub(crate) fn windows_fp(t: &mut Turn<'_>, isa: Isa) {
    if let (true, FpRegs::X86(mut fp)) = (isa.is_x86(), t.thread().fp()) {
        fp.fxsave[0..2].copy_from_slice(&0x27Fu16.to_le_bytes());
        t.thread().set_fp(&FpRegs::X86(fp));
    }
}

pub(crate) fn start(k: &Arc<Kernel>, cx: &mut Cx<'_>, argv: &[String]) -> Result<Task, String> {
    let path = disk_path(argv.first().ok_or("nothing to run")?);
    let bytes = {
        let disk = lock(&k.disk);
        let ino = disk.resolve(layers::ROOT, &path, true).map_err(|e| format!("{path}: {e}"))?;
        disk.contents(ino).map_err(|e| format!("{path}: {e}"))?.to_vec()
    };
    let pe = Pe::parse(&bytes).map_err(|_| format!("{path} isn't a Windows program"))?;
    if pe.is_dll {
        return Err(format!("{path} is a DLL"));
    }
    let isa = pe.isa;
    let mut image = pe.layout().map_err(|_| format!("{path}: its sections don't fit its image"))?;
    let windows_path = format!("C:{}", path.replace('/', "\\"));
    let mut handles = Handles::default();
    let std_handles = [handles.add(Handle::Input), handles.add(Handle::Console(host::Stream::Out)), handles.add(Handle::Console(host::Stream::Err))];
    let pid = k.new_id();
    let environment = crate::environment::block(&crate::environment::first(k, isa));
    let tid = k.new_id();

    let mut regions: Regions<Area> = Regions::default();
    let started = cx
        .turn(|t| -> Result<_, String> {
            let space = t.new_space(isa).map_err(|e| format!("exec can't run {isa:?} programs here: {e:?}"))?;
            t.use_space(space);
            let layout = t.space().layout();
            for kept in &layout.kept {
                regions.insert(kept.clone(), Area { prot: Prot::NONE, kind: AreaKind::Kept });
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
            t.space().map(ATLAS_PAGE..ATLAS_PAGE + PAGE, Prot::RX).with(&images::atlas_page(isa, gate)).map_err(|_| "no memory")?;
            regions.insert(ATLAS_PAGE..ATLAS_PAGE + PAGE, Area { prot: Prot::RX, kind: AreaKind::Image });

            // The PEB, its process parameters and strings, and the TEB, in one committed block.
            let block = regions.free_above(ALLOC, GRANULE * 2, GRANULE).ok_or("no room")?;
            regions.insert(block..block + GRANULE * 2, Area { prot: Prot::RW, kind: AreaKind::Committed });
            t.space().map(block..block + GRANULE * 2, Prot::RW).zeroed().map_err(|_| "no memory")?;
            let (peb_at, params_at, mut teb_at, strings_at) = (block, block + 0x1000, block + 0x2000, block + 0x4000);
            // Where the host decides the thread's block goes (Windows' sandbox: the stub thread's own TEB, since
            // Windows owns gs), the TEB goes there, and the host copies it in and out around each run.
            if let Some(at) = t.space().thread_block(0x2000) {
                regions.insert(at..at + 0x2000, Area { prot: Prot::RW, kind: AreaKind::Committed });
                t.space().map(at..at + 0x2000, Prot::RW).zeroed().map_err(|_| "no memory for the thread block")?;
                teb_at = at;
            }
            let line = command_line(argv);
            let wide = utf16z(&line);
            let ansi: Vec<u8> = line.bytes().chain([0]).collect();
            let image_path = utf16z(&windows_path);
            let (wide_at, ansi_at) = (strings_at, strings_at + wide.len() as u64 + 16);
            let path_at = ansi_at + ansi.len() as u64 + 16;
            let env_at = (path_at + image_path.len() as u64 + 16).next_multiple_of(16);
            if env_at + environment.len() as u64 > block + GRANULE * 2 {
                return Err("the environment is too big".to_string());
            }
            let unicode_string = |len: usize, at: u64| -> Vec<u8> {
                let mut s = ((len - 2) as u16).to_le_bytes().to_vec();
                s.extend((len as u16).to_le_bytes());
                s.extend([0; 4]);
                s.extend(at.to_le_bytes());
                s
            };
            let mut memory = t.space();
            let mut put = |at: u64, bytes: &[u8]| user::write(&mut memory, at, bytes).map_err(|_| "writing the PEB".to_string());
            put(wide_at, &wide)?;
            put(ansi_at, &ansi)?;
            put(path_at, &image_path)?;
            put(peb_at + peb::IMAGE_BASE, &base.to_le_bytes())?;
            put(peb_at + peb::PROCESS_PARAMETERS, &params_at.to_le_bytes())?;
            put(params_at, &(params::SIZE as u32).to_le_bytes())?;
            for (i, at) in [params::STANDARD_INPUT, params::STANDARD_OUTPUT, params::STANDARD_ERROR].into_iter().enumerate() {
                put(params_at + at, &std_handles[i].to_le_bytes())?;
            }
            put(params_at + params::IMAGE_PATH_NAME, &unicode_string(image_path.len(), path_at))?;
            put(params_at + params::COMMAND_LINE, &unicode_string(wide.len(), wide_at))?;
            put(env_at, &environment)?;
            put(params_at + params::ENVIRONMENT, &env_at.to_le_bytes())?;
            put(params_at + params::ENVIRONMENT_SIZE, &(environment.len() as u64).to_le_bytes())?;

            // The stack.
            let stack = regions.free_above(ALLOC, STACK, GRANULE).ok_or("no room for the stack")?;
            let stack_top = stack + STACK;
            regions.insert(stack..stack_top, Area { prot: Prot::RW, kind: AreaKind::Committed });
            t.space().map(stack_top - STACK_MAPPED..stack_top, Prot::RW).zeroed().map_err(|_| "no memory")?;
            let mut memory = t.space();
            let mut put = |at: u64, bytes: &[u8]| user::write(&mut memory, at, bytes).map_err(|_| "writing the TEB".to_string());
            put(teb_at + teb::STACK_BASE, &stack_top.to_le_bytes())?;
            put(teb_at + teb::STACK_LIMIT, &(stack_top - STACK_MAPPED).to_le_bytes())?;
            put(teb_at + teb::SELF, &teb_at.to_le_bytes())?;
            put(teb_at + teb::CLIENT_ID, &pid.to_le_bytes())?;
            put(teb_at + teb::CLIENT_ID + 8, &tid.to_le_bytes())?;
            put(teb_at + teb::PEB, &peb_at.to_le_bytes())?;

            // The first thread: ntdll's process start, which never returns, with the TEB where Windows keeps it (x18
            // on ARM64, gs on x86-64).
            let entry = process_start;
            t.thread().start(entry, stack_top);
            windows_fp(t, isa);
            let mut regs = t.thread().regs();
            let convention = match isa {
                Isa::Arm64 => Convention::Arm64,
                Isa::X86_64 => Convention::Win64,
                Isa::X86_32 => Convention::Stdcall,
            };
            let (sp, frame) = convention.prepare(&mut regs, entry, &[peb_at, ntdll], THREAD_RETURN_AT);
            user::write(&mut t.space(), sp, &frame).map_err(|_| "writing the first frame")?;
            match &mut regs {
                Regs::Arm64(r) => r.x[18] = teb_at,
                Regs::X86(r) => r.gs_base = teb_at,
            }
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
    let control = ThreadControl::new(tid, teb_at, waiter.clone(), cx.handle(), false);
    let mut threads = BTreeMap::new();
    threads.insert(tid, control.clone());
    let cwd = format!("C:\\{}", exe_dir.trim_start_matches('/').replace('/', "\\"));
    let state = ProcState { space, dispatcher, thread_start, peb, threads, mapped, exe_dir, cwd, regions, handles, std_handles, command_line: command, exiting: None };
    let object = Object::new(objects_mod::Kind::Process(pid), State::default());
    let process = Arc::new(Process { pid, state: Mutex::new(state), object, ended: (Mutex::new(None), Condvar::new()) });
    Ok(Task { k: k.clone(), process, tid, waiter, isa, depth: 0, nested: 0, control, ending: None })
}

