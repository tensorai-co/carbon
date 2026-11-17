//! exec-core on a mock sandbox whose CPUs each have a stub of their own (`Sandbox::space_per_cpu`), as the Linux
//! sandbox's do: each CPU's stub shows its own space, a page as its program first faults on it, what a turn changes
//! reaches the stubs in merged runs, a CPU whose space another CPU changed is stopped until it's caught up, and a
//! dropped CPU's slot is used again.
//!
//! Each mock stub is a map from page to the guest-RAM offset and protection core told it to show, plus every run it
//! was told, in order. Everything runs on the test's own thread.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use exec_core::contract::{FaultKind, GuestRam, HostFault, MirrorRun, RawPreempt, Reach, Sandbox, SandboxCpu, SandboxExit};
use exec_core::isa::x86::{X86Fp, X86Regs};
use exec_core::shim::x86::X86Shim;
use exec_core::{Core, Cpu, FpRegs, Isa, Isolation, Layout, NoVm, Options, PAGE, Prot, Regs, Stop, Why};

struct NoHypervisor;
impl Why for NoHypervisor {
    const NAME: &'static str = "none";
    const WHY: &'static str = "the mock has no vm";
}

type Exec = Core<NoVm<X86Shim, NoHypervisor>, Mock>;
type MockCpuOf = Cpu<NoVm<X86Shim, NoHypervisor>, Mock>;

#[derive(Default)]
struct Stub {
    pages: BTreeMap<u64, (u64, Prot)>,
    runs: Vec<MirrorRun>,
}

type Stubs = Arc<Mutex<Vec<Arc<Mutex<Stub>>>>>;

/// What each CPU's next `resume` does, by CPU index: nothing scripted is a preempt.
type Script = Arc<Mutex<BTreeMap<usize, VecDeque<SandboxExit>>>>;

struct Mock {
    stubs: Stubs,
    script: Script,
}

struct MockCpu {
    index: usize,
    regs: Regs,
    fp: FpRegs,
    stub: Arc<Mutex<Stub>>,
    script: Script,
}

thread_local! {
    static OPENED: std::cell::RefCell<Option<(Stubs, Script)>> = const { std::cell::RefCell::new(None) };
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
    const NAME: &'static str = "mock-cpus";

    fn alloc_ram(bytes: u64) -> Result<GuestRam, String> {
        let layout = std::alloc::Layout::from_size_align(bytes as usize, PAGE as usize).map_err(|e| e.to_string())?;
        // SAFETY: a non-zero size.
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        Ok(GuestRam { ptr, size: bytes, stride: PAGE, granule: PAGE, handle: -1, discard, release })
    }

    fn open(_: &GuestRam, _: &Options) -> Result<Mock, String> {
        let (stubs, script) = (Stubs::default(), Script::default());
        OPENED.with(|o| *o.borrow_mut() = Some((stubs.clone(), script.clone())));
        Ok(Mock { stubs, script })
    }

    fn isas(&self) -> &'static [Isa] {
        &[Isa::X86_64]
    }

    fn reach(&self) -> Reach {
        Reach::Dispatch
    }

    fn layout(&self, _: Isa) -> Layout {
        Layout { top: 1 << 47, kept: Vec::new(), gate: None, traps: true, jump_gate: None }
    }

    fn max_cpus(&self) -> usize {
        4
    }

    fn space_per_cpu(&self) -> bool {
        true
    }

    fn new_cpu(&self, index: usize) -> Result<MockCpu, String> {
        // A new stub, showing nothing, in the CPU's slot.
        let stub = Arc::new(Mutex::new(Stub::default()));
        let mut stubs = self.stubs.lock().expect("stubs");
        if index < stubs.len() {
            stubs[index] = stub.clone();
        } else {
            stubs.push(stub.clone());
        }
        Ok(MockCpu { index, regs: Regs::X86(X86Regs::default()), fp: FpRegs::X86(X86Fp::default()), stub, script: self.script.clone() })
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
        self.script.lock().expect("script").get_mut(&self.index).and_then(VecDeque::pop_front).unwrap_or(SandboxExit::Preempted)
    }

    fn preempt_handle(&self) -> RawPreempt {
        Arc::new(|| {})
    }

    fn mirror(&mut self, runs: &[MirrorRun]) -> Result<(), String> {
        let mut stub = self.stub.lock().expect("stub");
        for run in runs {
            stub.runs.push(*run);
            match *run {
                MirrorRun::Map { page, len, offset, prot } => {
                    for i in (0..len).step_by(PAGE as usize) {
                        stub.pages.insert(page + i, (offset + i, prot));
                    }
                }
                MirrorRun::Protect { page, len, prot } => {
                    for i in (0..len).step_by(PAGE as usize) {
                        let entry = stub.pages.get_mut(&(page + i)).ok_or_else(|| format!("protecting {:#x}, which isn't shown", page + i))?;
                        entry.1 = prot;
                    }
                }
                MirrorRun::Unmap { page, len } => {
                    let gone: Vec<u64> = stub.pages.range(page..page + len).map(|(&p, _)| p).collect();
                    for page in gone {
                        stub.pages.remove(&page);
                    }
                }
            }
        }
        Ok(())
    }
}

fn open() -> (Exec, MockCpuOf, Stubs) {
    let (exec, cpu) = Exec::open(Options { memory: 16 << 20, isolation: Isolation::Sandbox, ..Options::default() }).expect("the mock opens");
    let (stubs, _) = OPENED.with(|o| o.borrow().clone()).expect("the mock's stubs");
    (exec, cpu, stubs)
}

fn stub(stubs: &Stubs, cpu: usize) -> Arc<Mutex<Stub>> {
    stubs.lock().expect("stubs")[cpu].clone()
}

fn shown(stubs: &Stubs, cpu: usize, page: u64) -> Option<(u64, Prot)> {
    stub(stubs, cpu).lock().expect("stub").pages.get(&page).copied()
}

const A: u64 = 0x10_0000;

/// The program on `cpu` (index `index`) touches `addr`: a page its stub doesn't show yet faults, and core shows it,
/// with the pages around it, without the kernel hearing of it.
fn touch(exec: &mut Exec, cpu: &mut MockCpuOf, index: usize, addr: u64) {
    let (_, script) = OPENED.with(|o| o.borrow().clone()).expect("the mock's script");
    script.lock().expect("script").entry(index).or_default().push_back(SandboxExit::Fault(HostFault { kind: FaultKind::Segv, addr, access: Prot::R, pc: 0x1000 }));
    let exit = cpu.run();
    assert_eq!(exec.enter(cpu).stop(exit), None, "a fault on a page the tables have is core's");
}

#[test]
fn a_parent_and_its_child_show_their_own_spaces_at_once() {
    let (mut exec, mut parent_cpu, stubs) = open();
    let child = {
        let mut turn = exec.enter(&mut parent_cpu);
        let parent = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(parent);
        turn.space().map(A..A + PAGE, Prot::RW).with(&5u32.to_le_bytes()).expect("map");
        turn.fork_space().expect("fork")
    };
    let mut child_cpu = exec.add_cpu().expect("a second CPU");
    exec.enter(&mut child_cpu).use_space(child);
    assert!(shown(&stubs, 1, A).is_none(), "a new stub shows nothing until it's faulted on");
    touch(&mut exec, &mut parent_cpu, 0, A);
    touch(&mut exec, &mut child_cpu, 1, A);
    // Both share the frame, read-only in both stubs.
    let (frame, prot) = shown(&stubs, 0, A).expect("the parent's");
    assert!(!prot.contains(Prot::W));
    assert_eq!(shown(&stubs, 1, A), Some((frame, prot)));

    // The child gets its own copy: only its stub changes, and the parent's still shows the shared frame.
    assert_eq!(exec.enter(&mut child_cpu).space().make_writable(A, Prot::RW), Ok(true));
    let (copy, prot) = shown(&stubs, 1, A).expect("the child's");
    assert!(copy != frame && prot.contains(Prot::W));
    assert_eq!(shown(&stubs, 0, A).map(|(f, _)| f), Some(frame));
}

#[test]
fn a_cpu_whose_space_another_changed_catches_up_before_it_runs() {
    let (mut exec, mut one, stubs) = open();
    let space = {
        let mut turn = exec.enter(&mut one);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + 2 * PAGE, Prot::RW).zeroed().expect("map");
        space
    };
    // A second thread of the same program, on a CPU of its own.
    let mut two = exec.add_cpu().expect("a second CPU");
    exec.enter(&mut two).use_space(space);
    touch(&mut exec, &mut one, 0, A);
    touch(&mut exec, &mut two, 1, A + PAGE);
    assert!(shown(&stubs, 1, A).is_some() && shown(&stubs, 1, A + PAGE).is_some(), "the pages around a fault come with it");

    // The first unmaps a page: the second's stub still shows it, so it mustn't run until it's caught up.
    exec.enter(&mut one).space().unmap(A + PAGE..A + 2 * PAGE);
    assert!(shown(&stubs, 0, A + PAGE).is_none());
    assert!(shown(&stubs, 1, A + PAGE).is_some(), "not yet: only its own thread can change its stub");
    let exit = two.run();
    assert_eq!(exec.enter(&mut two).stop(exit), Some(Stop::Preempted), "it stops before running, for a turn");
    assert!(shown(&stubs, 1, A + PAGE).is_none(), "and that turn caught its stub up");
    assert!(shown(&stubs, 1, A).is_some());
}

#[test]
fn changes_reach_a_stub_in_merged_runs() {
    let (mut exec, mut cpu, stubs) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
    }
    // A small one, as a fault's own pages, is shown with its turn: sixteen pages next to each other on frames next to
    // each other, as one mapping.
    let before = stub(&stubs, 0).lock().expect("stub").runs.len();
    exec.enter(&mut cpu).space().map(A..A + 16 * PAGE, Prot::RW).zeroed().expect("map");
    let runs = stub(&stubs, 0).lock().expect("stub").runs[before..].to_vec();
    let mapped: u64 = runs.iter().map(|r| if let MirrorRun::Map { len, .. } = r { *len } else { 0 }).sum();
    assert_eq!(mapped, 16 * PAGE, "{runs:?}");
    assert!(runs.len() <= 2, "frames from a fresh machine are mostly in a row: {runs:?}");

    // A protection change over all of them is one run, with no remapping.
    let before = stub(&stubs, 0).lock().expect("stub").runs.len();
    exec.enter(&mut cpu).space().protect(A..A + 16 * PAGE, Prot::R);
    let runs = stub(&stubs, 0).lock().expect("stub").runs[before..].to_vec();
    assert_eq!(runs, vec![MirrorRun::Protect { page: A, len: 16 * PAGE, prot: Prot::R }]);

    // A page that didn't change costs nothing.
    let before = stub(&stubs, 0).lock().expect("stub").runs.len();
    exec.enter(&mut cpu).space().protect(A..A + 16 * PAGE, Prot::R);
    assert_eq!(stub(&stubs, 0).lock().expect("stub").runs.len(), before);

    // Taking every other page away, then the rest: the second unmapping spans the gaps the first left.
    exec.enter(&mut cpu).space().unmap(A + PAGE..A + 2 * PAGE);
    let before = stub(&stubs, 0).lock().expect("stub").runs.len();
    exec.enter(&mut cpu).space().unmap(A..A + 16 * PAGE);
    let runs = stub(&stubs, 0).lock().expect("stub").runs[before..].to_vec();
    assert_eq!(runs, vec![MirrorRun::Unmap { page: A, len: 16 * PAGE }]);

    // A big mapping waits for faults, and a fault brings in the pages around it.
    exec.enter(&mut cpu).space().map(A + 64 * PAGE..A + 128 * PAGE, Prot::RW).zeroed().expect("map");
    assert!(shown(&stubs, 0, A + 64 * PAGE).is_none(), "mapped, but not faulted on yet");
    touch(&mut exec, &mut cpu, 0, A + 70 * PAGE);
    assert!(shown(&stubs, 0, A + 64 * PAGE).is_some() && shown(&stubs, 0, A + 79 * PAGE).is_some() && shown(&stubs, 0, A + 80 * PAGE).is_none());
}

#[test]
fn a_dropped_cpus_slot_is_used_again_with_a_new_stub() {
    let (mut exec, mut cpu, stubs) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + PAGE, Prot::RW).zeroed().expect("map");
    }
    let two = exec.add_cpu().expect("a second CPU");
    assert_eq!(two.id().0, 1);
    drop(two);
    for _ in 0..10 {
        let mut again = exec.add_cpu().expect("the slot, again");
        assert_eq!(again.id().0, 1, "the slot is used again");
        assert!(shown(&stubs, 1, A).is_none(), "a new stub shows nothing");
        let space = exec.enter(&mut cpu).space().id();
        exec.enter(&mut again).use_space(space);
        touch(&mut exec, &mut again, 1, A);
        assert!(shown(&stubs, 1, A).is_some());
    }
}

#[test]
fn a_fault_on_a_shown_page_or_none_at_all_is_the_kernels() {
    let (mut exec, mut cpu, _stubs) = open();
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64).expect("space");
        turn.use_space(space);
        turn.space().map(A..A + PAGE, Prot::R).zeroed().expect("map");
    }
    touch(&mut exec, &mut cpu, 0, A);
    let (_, script) = OPENED.with(|o| o.borrow().clone()).expect("script");
    // A write to the read-only page it now shows, and a read of a page there isn't: both the kernel's.
    for (addr, access) in [(A, Prot::W), (A + 64 * PAGE, Prot::R)] {
        script.lock().expect("script").entry(0).or_default().push_back(SandboxExit::Fault(HostFault { kind: FaultKind::Segv, addr, access, pc: 0x1000 }));
        let exit = cpu.run();
        assert_eq!(exec.enter(&mut cpu).stop(exit), Some(Stop::Fault { addr, access, pc: 0x1000 }));
    }
}
