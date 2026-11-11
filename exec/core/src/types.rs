//! The words everything in `exec` speaks.

use std::fmt;
use std::ops::Range;

use exec_mmu::{OutOfMemory, Prot, Unmapped};

use crate::Tuning;

/// An instruction set a program can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Isa {
    Arm64,
    X86_32,
    X86_64,
}

impl Isa {
    /// The host CPU's own.
    pub const fn host() -> Isa {
        if cfg!(target_arch = "aarch64") { Isa::Arm64 } else { Isa::X86_64 }
    }

    pub const fn is_x86(self) -> bool {
        matches!(self, Isa::X86_32 | Isa::X86_64)
    }
}

/// How strongly a program is walled off from the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Isolation {
    /// The best this host has: `vm` if it can, else `sandbox`.
    #[default]
    Auto,
    /// A hardware VM.
    Vm,
    /// A process on the host.
    Sandbox,
}

impl fmt::Display for Isolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Isolation::Auto => "auto",
            Isolation::Vm => "vm",
            Isolation::Sandbox => "sandbox",
        })
    }
}

/// Which backend is running: `vm/kvm`, `sandbox/linux`, …
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backend {
    /// `Vm` or `Sandbox`, never `Auto`.
    pub isolation: Isolation,
    pub name: &'static str,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.isolation, self.name)
    }
}

/// What a backend can do here.
#[derive(Debug, Clone)]
pub struct Caps {
    /// What `new_space` takes.
    pub isas: &'static [Isa],
    pub max_cpus: usize,
    /// Whether x86 memory ordering can be switched on.
    pub tso: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Guest memory, in bytes. It's a limit: the host backs only what's touched.
    pub memory: u64,
    pub isolation: Isolation,
    /// x86 memory ordering, for translated x86 on Apple silicon.
    pub tso: bool,
    pub tuning: Tuning,
}

/// Why `open` failed, for each backend it tried.
#[derive(Debug, Clone)]
pub struct OpenError {
    pub wanted: Isolation,
    pub tried: Vec<(&'static str, String)>,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = self.tried.iter().map(|(name, why)| format!("{name} ({why})")).collect::<Vec<_>>().join("; ");
        match self.wanted {
            Isolation::Vm => write!(f, "--isolation vm needs a hypervisor: {why}"),
            Isolation::Sandbox => write!(f, "--isolation sandbox can't start here: {why}"),
            Isolation::Auto => write!(f, "nothing can run programs here: {why}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Why a program stopped, as its kernel needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Syscall { nr: u64, args: [u64; 6] },
    /// It touched `addr` in a way its pages don't allow, or that isn't mapped. `access` is what it tried.
    Fault { addr: u64, access: Prot, pc: u64 },
    /// An instruction it isn't allowed, or that doesn't exist.
    Undefined { pc: u64 },
    /// A misaligned pc or stack.
    Alignment { pc: u64 },
    /// A breakpoint instruction.
    Breakpoint { pc: u64 },
    /// x86's `int 0x29`, which Windows programs use to end at once.
    FastFail { pc: u64, code: u64 },
    /// x86's `#DE`.
    DivideError { pc: u64 },
    /// It was preempted, and can carry on from where it was.
    Preempted,
    /// `exec` itself went wrong, not the program.
    Internal { pc: u64, detail: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    OutOfMemory,
    /// Nothing usable is mapped here.
    Unmapped(u64),
    NoMoreCpus,
    /// This CPU hasn't been given a space to use yet.
    NoSpace,
    /// This backend can't run this instruction set.
    Unsupported(Isa),
}

impl From<OutOfMemory> for Error {
    fn from(_: OutOfMemory) -> Error {
        Error::OutOfMemory
    }
}

impl From<Unmapped> for Error {
    fn from(unmapped: Unmapped) -> Error {
        Error::Unmapped(unmapped.0)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfMemory => f.write_str("out of guest memory"),
            Error::Unmapped(at) => write!(f, "nothing mapped at {at:#x}"),
            Error::NoMoreCpus => f.write_str("no more CPUs"),
            Error::NoSpace => f.write_str("this CPU has no space in use"),
            Error::Unsupported(isa) => write!(f, "this backend can't run {isa:?} programs"),
        }
    }
}

impl std::error::Error for Error {}

/// A CPU, by its index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuId(pub usize);

/// An address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpaceId(pub u64);

/// Where programs may live in a space.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// Programs live below this.
    pub top: u64,
    /// Ranges the backend keeps for itself, which the kernel must never give a program.
    pub kept: Vec<Range<u64>>,
    /// Where a program calls instead of trapping, for a sandbox that can't stop it at the trap itself.
    pub gate: Option<u64>,
}

/// A mapped page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub prot: Prot,
    /// Shared copy-on-write: read-only until `make_writable`.
    pub shared: bool,
}

/// How much of a range is mapped, and how much of that is shared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resident {
    pub mapped: u64,
    pub shared: u64,
}

/// What a space is using, in bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpaceUsage {
    pub code: u64,
    pub read_only: u64,
    pub writable: u64,
    /// Of those, mapped somewhere else too.
    pub shared: u64,
    pub tables: u64,
}

/// What the machine's memory is doing, in bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryStats {
    pub limit: u64,
    /// What the host has backed.
    pub touched: u64,
    /// Backed, but not in use.
    pub free: u64,
    /// Given back to the host so far.
    pub given_back: u64,
    pub tables: u64,
}

/// Every mapped page, and how many are the same.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageCensus {
    /// Counting a page once for each space it's in.
    pub mapped: u64,
    /// Counting each frame once.
    pub frames: u64,
    /// Counting pages with the same contents once.
    pub contents: u64,
    /// Frames holding only zeroes.
    pub zero: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySize {
    pub limit: u64,
    pub available: u64,
}

/// How short of memory the host says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}
