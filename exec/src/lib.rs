//! `exec` runs a program's machine code and stops it whenever the program needs its kernel: a system call, a fault, a
//! bad instruction, or a time slice running out. It owns the program's memory and CPUs, and knows nothing of any
//! kernel, file system or ABI.
//!
//! A host has two backends, chosen by isolation: `vm`, a hardware VM (hvf, kvm, winkvm), and `sandbox`, a process on
//! the host. Everything outside `exec` sees only this crate, with one concrete `Exec` and no engine type parameter.
//!
//! ```no_run
//! use exec::{Exec, Isa, Isolation, Options, Prot, Stop};
//!
//! let (mut exec, mut cpu) = Exec::open(Options { memory: 64 << 20, isolation: Isolation::Auto, ..Options::default() })?;
//! {
//!     let mut turn = exec.enter(&mut cpu);
//!     let space = turn.new_space(Isa::host())?;
//!     turn.use_space(space);
//!     turn.space().map(0x40_0000..0x40_1000, Prot::RX).with(&[0xd4, 0x00, 0x00, 0x01])?;
//!     turn.thread().start(0x40_0000, 0x80_0000);
//! }
//! loop {
//!     let exit = cpu.run();                         // no lock: programs run in parallel
//!     let mut turn = exec.enter(&mut cpu);
//!     match turn.stop(exit) {
//!         Some(Stop::Syscall { nr, .. }) => turn.thread().set_return(nr),
//!         Some(Stop::Preempted) | None => {}
//!         Some(other) => break,
//!     }
//! }                                                 // each turn settles as it ends
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg(target_os = "macos")]
use exec_macos as host;

#[cfg(target_os = "linux")]
use exec_linux as host;

#[cfg(windows)]
use exec_windows as host;

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
compile_error!("exec has backends for macOS, Linux and Windows");

pub use exec_core::isa;
pub use exec_core::{
    Backend, Caps, Context, CpuHandle, CpuId, Error, Exit, FpRegs, Frame, Isa, Isolation, Layout, MemorySize, MemoryStats, OpenError,
    Options, PAGE, Page, PageCensus, Pressure, Prot, Regs, Resident, SpaceId, SpaceUsage, Stop, Tuning,
};

/// A machine's memory and CPUs, on this host's `vm` or `sandbox` backend.
pub type Exec = exec_core::Core<host::Vm, host::Sandbox>;
/// One CPU, owned by its thread.
pub type Cpu = exec_core::Cpu<host::Vm, host::Sandbox>;
/// A CPU's turn in the kernel.
pub type Turn<'e> = exec_core::Turn<'e, host::Vm, host::Sandbox>;
pub type Space<'t> = exec_core::Space<'t, host::Vm, host::Sandbox>;
pub type Map<'s> = exec_core::Map<'s, host::Vm, host::Sandbox>;
pub type Thread<'t> = exec_core::Thread<'t, host::Vm, host::Sandbox>;
pub type Memory<'e> = exec_core::Memory<'e, host::Vm, host::Sandbox>;

/// If this process is a sandbox's stub, does a stub's job and never returns; otherwise returns at once. On macOS the
/// sandbox's stub is this same binary run again, so a binary that may use it calls this first thing in `main`.
pub fn run_if_stub() {
    #[cfg(target_os = "macos")]
    exec_macos::run_if_stub();
}

/// For tests and debugging: `code` in a fresh space, with a stack, run until its first stop that isn't a preempt.
pub fn run_once(isolation: Isolation, isa: Isa, code: &[u8]) -> Result<(Stop, Regs), Box<dyn std::error::Error>> {
    const STACK: u64 = 0x1_0000;

    // x86 translated on Apple silicon wants the CPU's own x86 memory ordering.
    let tso = isa.is_x86() && Isa::host() == Isa::Arm64;
    let (mut exec, mut cpu) = Exec::open(Options { memory: 64 << 20, isolation, tso, ..Options::default() })?;
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(isa)?;
        turn.use_space(space);
        // Somewhere the layout allows: low down, unless the backend keeps it (macOS's sandbox keeps the bottom 4 GiB).
        let layout = turn.space().layout();
        let free = |at: u64| at + 0x40_0000 <= layout.top && !layout.kept.iter().any(|r| r.start < at + 0x40_0000 && at < r.end);
        let base = [0x40_0000u64, 0x10_0000_0000, 0x1000_0000_0000].into_iter().find(|at| free(*at)).ok_or("no room for the program")?;
        let (code_at, stack_top) = (base, base + 0x40_0000);
        let end = code_at + (code.len() as u64).max(1).next_multiple_of(PAGE);
        turn.space().map(code_at..end, Prot::RX).with(code)?;
        turn.space().map(stack_top - STACK..stack_top, Prot::RW).zeroed()?;
        turn.thread().start(code_at, stack_top);
    }
    loop {
        let exit = cpu.run();
        let mut turn = exec.enter(&mut cpu);
        match turn.stop(exit) {
            Some(Stop::Preempted) | None => continue,
            Some(stop) => return Ok((stop, turn.thread().regs())),
        }
    }
}
