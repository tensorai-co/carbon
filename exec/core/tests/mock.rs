//! exec-core on a mock sandbox, with no hypervisor and no stub process: spaces, mapping, fork and copy-on-write,
//! unmapping, switching spaces, stops, and giving memory back, checked against what the mock's "stub" was told to show.
//!
//! The mock is an x86-64 sandbox. Its stub is a map from page to the guest frame and protection core asked it to show,
//! over core's own guest RAM; its CPU plays a script of steps, each reading or writing memory as a program would (with
//! the stub's protections), or arriving at the gate. Everything runs on the test's own thread.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use exec_core::contract::{FaultKind, GuestRam, HostFault, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::Trap;
use exec_core::isa::x86::{RAX, RDI, RSI, X86Fp, X86Regs};
use exec_core::shim::x86::X86Shim;
use exec_core::{Core, Cpu, FpRegs, Isa, Isolation, Layout, NoVm, Options, PAGE, Prot, Regs, Stop, Why};

struct NoHypervisor;
impl Why for NoHypervisor {
    const NAME: &'static str = "none";
    const WHY: &'static str = "the mock has no vm";
}

type Exec = Core<NoVm<X86Shim, NoHypervisor>, Mock>;
type MockCpuOf = Cpu<NoVm<X86Shim, NoHypervisor>, Mock>;

/// What the stub shows: each page's guest-RAM offset and protection, over guest RAM itself.
struct Stub {
    ram: usize,
    pages: BTreeMap<u64, (u64, Prot)>,
}

impl Stub {
    fn fault(addr: u64, access: Prot) -> SandboxExit {
        SandboxExit::Fault(HostFault { kind: FaultKind::Segv, addr, access, pc: 0x1000 })
    }

    /// Reads as a program would.
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, SandboxExit> {
        let page = addr & !(PAGE - 1);
        match self.pages.get(&page) {
            Some(&(offset, prot)) if prot.contains(Prot::R) => {
                let at = (self.ram as u64 + offset + (addr - page)) as *const u8;
                // SAFETY: inside a guest frame the stub shows, within guest RAM.
                Ok(unsafe { std::slice::from_raw_parts(at, len) }.to_vec())
            }
            _ => Err(Stub::fault(addr, Prot::R)),
        }
    }

    /// Writes as a program would.
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), SandboxExit> {
        let page = addr & !(PAGE - 1);
        match self.pages.get(&page) {
            Some(&(offset, prot)) if prot.contains(Prot::W) => {
                let at = (self.ram as u64 + offset + (addr - page)) as *mut u8;
                // SAFETY: as for `read`, and writable.
                unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), at, data.len()) };
                Ok(())
            }
            _ => Err(Stub::fault(addr, Prot::W)),
        }
    }
}

/// One step of a program: it may use memory and registers, and says why it stopped.
type Step = Box<dyn FnOnce(&mut X86Regs, &mut Stub) -> SandboxExit>;

thread_local! {
    static STUB: RefCell<Option<Arc<Mutex<Stub>>>> = const { RefCell::new(None) };
    static SCRIPT: RefCell<VecDeque<Step>> = const { RefCell::new(VecDeque::new()) };
}

struct Mock {
    stub: Arc<Mutex<Stub>>,
}

struct MockCpu {
    regs: Regs,
    fp: FpRegs,
    stub: Arc<Mutex<Stub>>,
}

fn release(ptr: *mut u8, size: u64, _: i64) {
    // SAFETY: allocated by `alloc_ram` with this size and alignment.
    unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(size as usize, PAGE as usize).expect("layout")) };
}

fn discard(ptr: *mut u8, len: usize) -> bool {
    // SAFETY: part of guest RAM, which core is giving back.
    unsafe { std::ptr::write_bytes(ptr, 0, len) };
    true
}

impl Sandbox for Mock {
    type Cpu = MockCpu;
    const NAME: &'static str = "mock";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        let layout = std::alloc::Layout::from_size_align(bytes as usize, PAGE as usize).map_err(|e| e.to_string())?;
        // SAFETY: a non-zero size.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        Ok(GuestRam { ptr, size: bytes, stride: PAGE, granule: PAGE, handle: -1, discard, release })
    }

    fn open(ram: &GuestRam, _: &Options) -> Result<Mock, String> {
        let stub = Arc::new(Mutex::new(Stub { ram: ram.ptr as usize, pages: BTreeMap::new() }));
        STUB.with(|s| *s.borrow_mut() = Some(stub.clone()));
        Ok(Mock { stub })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Dispatch
    }

    fn layout(&self, _: Isa) -> Layout {
        Layout { top: 1 << 47, kept: Vec::new(), gate: None }
    }

    fn max_cpus(&self) -> usize {
        1
    }

    fn mirror(&mut self, page: u64, ram_offset: Option<u64>, prot: Prot) -> Result<(), String> {
        let mut stub = self.stub.lock().expect("stub");
        match ram_offset {
            Some(offset) => stub.pages.insert(page, (offset, prot)),
            None => stub.pages.remove(&page),
        };
        Ok(())
    }

    fn new_cpu(&self, _: usize) -> Result<MockCpu, String> {
        Ok(MockCpu { regs: Regs::X86(X86Regs::default()), fp: FpRegs::X86(X86Fp::default()), stub: self.stub.clone() })
    }
}

impl SandboxCpu for MockCpu {
    fn regs(&mut self) -> &mut Regs {
        &mut self.regs
    }

    fn fp(&mut self) -> &mut FpRegs {
        &mut self.fp
    }

    fn resume(&mut self) -> SandboxExit {
        let Some(step) = SCRIPT.with(|s| s.borrow_mut().pop_front()) else { return SandboxExit::Failed(1) };
        let Regs::X86(regs) = &mut self.regs else { return SandboxExit::Failed(2) };
        step(regs, &mut self.stub.lock().expect("stub"))
    }

    fn preempt_handle(&self) -> RawPreempt {
        Arc::new(|| {})
    }
}

fn open() -> (Exec, MockCpuOf, Arc<Mutex<Stub>>) {
    SCRIPT.with(|s| s.borrow_mut().clear());
    let (exec, cpu) = Exec::open(Options { memory: 16 << 20, isolation: Isolation::Sandbox, ..Options::default() }).expect("the mock opens");
    let stub = STUB.with(|s| s.borrow().clone()).expect("the mock's stub");
    (exec, cpu, stub)
}

fn script(step: impl FnOnce(&mut X86Regs, &mut Stub) -> SandboxExit + 'static) {
    SCRIPT.with(|s| s.borrow_mut().push_back(Box::new(step)));
}

/// Runs the script's next step, as a kernel's loop would, to its stop.
fn step(exec: &mut Exec, cpu: &mut MockCpuOf) -> Stop {
    let exit = cpu.run();
    exec.enter(cpu).stop(exit).expect("a stop for the kernel")
}

const A: u64 = 0x10_0000;
const B: u64 = 0x20_0000;

/// A page shown by the stub: its frame's offset and protection.
fn shown(stub: &Arc<Mutex<Stub>>, page: u64) -> Option<(u64, Prot)> {
    stub.lock().expect("stub").pages.get(&page).copied()
}

#[test]
fn a_mapped_page_shows_in_the_stub_once_the_turn_ends() {
    let (mut exec, mut cpu, stub) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + PAGE, Prot::RW).with(b"hello").expect("map");
        assert_eq!(shown(&stub, A), None, "nothing's shown until the turn ends");
    }
    let (_, prot) = shown(&stub, A).expect("shown");
    assert_eq!(prot, Prot::RW);
    assert_eq!(stub.lock().expect("stub").read(A, 5).ok().as_deref(), Some(&b"hello"[..]));
}

#[test]
fn a_system_call_arrives_with_its_number_and_arguments() {
    let (mut exec, mut cpu, _) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.thread().start(0x40_0000, 0x50_0000);
    }
    script(|r, _| {
        (r.gpr[RAX], r.gpr[RDI], r.gpr[RSI]) = (60, 7, 8);
        SandboxExit::Gate(Trap::Syscall)
    });
    assert!(matches!(step(&mut exec, &mut cpu), Stop::Syscall { nr: 60, args: [7, 8, ..] }));
    // The kernel's answer goes in rax, for the program's next step.
    exec.enter(&mut cpu).thread().set_return(41);
    script(|r, _| {
        assert_eq!(r.gpr[RAX], 41);
        SandboxExit::Preempted
    });
    assert_eq!(step(&mut exec, &mut cpu), Stop::Preempted);
}

#[test]
fn host_faults_become_the_kernels_stops() {
    let (mut exec, mut cpu, _) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
    }
    let fault = |kind, addr| SandboxExit::Fault(HostFault { kind, addr, access: Prot::R, pc: 0x1234 });
    for (kind, want) in [
        (FaultKind::Ill, Stop::Undefined { pc: 0x1234 }),
        (FaultKind::Trap, Stop::Breakpoint { pc: 0x1234 }),
        (FaultKind::DivZero, Stop::DivideError { pc: 0x1234 }),
        (FaultKind::Segv, Stop::Fault { addr: 0x99, access: Prot::R, pc: 0x1234 }),
    ] {
        script(move |_, _| fault(kind, 0x99));
        assert_eq!(step(&mut exec, &mut cpu), want, "{kind:?}");
    }
}

#[test]
fn fork_shares_pages_until_one_side_writes() {
    let (mut exec, mut cpu, stub) = open();
    let (parent, child) = {
        let mut turn = exec.enter(&mut cpu);
        let parent = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(parent);
        turn.space().map(A..A + PAGE, Prot::RW).with(&5u32.to_le_bytes()).expect("map");
        let child = turn.fork_space().expect("fork");
        (parent, child)
    };
    // Both share one frame, read-only in the parent's stub.
    let (frame, prot) = shown(&stub, A).expect("shown");
    assert!(!prot.contains(Prot::W), "shared pages are read-only after a fork");

    // The parent writes: the stub refuses, and the kernel copies the page.
    script(|_, s| match s.write(A, &7u32.to_le_bytes()) {
        Ok(()) => SandboxExit::Failed(3),
        Err(fault) => fault,
    });
    let stop = step(&mut exec, &mut cpu);
    assert!(matches!(stop, Stop::Fault { addr: A, access, .. } if access.contains(Prot::W)), "{stop:?}");
    assert_eq!(exec.enter(&mut cpu).space().make_writable(A, Prot::RW), Ok(true));
    let (copy, prot) = shown(&stub, A).expect("shown");
    assert!(copy != frame && prot.contains(Prot::W), "the parent has its own writable copy now");
    script(|_, s| match s.write(A, &7u32.to_le_bytes()) {
        Ok(()) => SandboxExit::Preempted,
        Err(fault) => fault,
    });
    assert_eq!(step(&mut exec, &mut cpu), Stop::Preempted);

    // The child still sees the old value, in the old frame.
    exec.enter(&mut cpu).use_space(child);
    assert_eq!(shown(&stub, A).map(|(f, _)| f), Some(frame));
    assert_eq!(stub.lock().expect("stub").read(A, 4).ok(), Some(5u32.to_le_bytes().to_vec()));
    exec.enter(&mut cpu).use_space(parent);
    assert_eq!(stub.lock().expect("stub").read(A, 4).ok(), Some(7u32.to_le_bytes().to_vec()));
}

#[test]
fn unmapping_takes_pages_out_of_the_stub() {
    let (mut exec, mut cpu, stub) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + 3 * PAGE, Prot::RW).zeroed().expect("map");
    }
    assert!(shown(&stub, A + PAGE).is_some());
    exec.enter(&mut cpu).space().unmap(A + PAGE..A + 2 * PAGE);
    assert!(shown(&stub, A).is_some() && shown(&stub, A + PAGE).is_none() && shown(&stub, A + 2 * PAGE).is_some());
    exec.enter(&mut cpu).space().protect(A..A + PAGE, Prot::R);
    assert_eq!(shown(&stub, A).map(|(_, p)| p), Some(Prot::R));
}

#[test]
fn switching_spaces_switches_what_the_stub_shows() {
    let (mut exec, mut cpu, stub) = open();
    let (one, two) = {
        let mut turn = exec.enter(&mut cpu);
        let one = turn.new_space(Isa::X86_64).expect("space");
        let two = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(one);
        turn.space().map(A..A + PAGE, Prot::RW).zeroed().expect("map");
        turn.use_space(two);
        turn.space().map(B..B + PAGE, Prot::RW).zeroed().expect("map");
        (one, two)
    };
    assert!(shown(&stub, A).is_none() && shown(&stub, B).is_some());
    exec.enter(&mut cpu).use_space(one);
    assert!(shown(&stub, A).is_some() && shown(&stub, B).is_none());
    // Destroying the space it shows leaves nothing shown once another is used.
    {
        let mut turn = exec.enter(&mut cpu);
        turn.use_space(two);
        turn.destroy_space(one);
    }
    assert!(shown(&stub, A).is_none() && shown(&stub, B).is_some());
}

#[test]
fn a_destroyed_spaces_frames_are_free_again() {
    let (mut exec, mut cpu, _) = open();
    let before = exec.enter(&mut cpu).memory().stats();
    let space = {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + 256 * PAGE, Prot::RW).with(&[1; 256 * PAGE as usize]).expect("map");
        space
    };
    let used = exec.enter(&mut cpu).memory().stats();
    assert!(used.touched >= before.touched + 256 * PAGE, "{before:?} then {used:?}");
    {
        let mut turn = exec.enter(&mut cpu);
        let other = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(other);
        turn.destroy_space(space);
    }
    let after = exec.enter(&mut cpu).memory().stats();
    assert!(after.free + after.given_back >= 256 * PAGE + before.free + before.given_back, "{used:?} then {after:?}");
}
