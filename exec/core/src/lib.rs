//! The heart of `exec`: its types, the facts about each CPU type, the contract a backend answers, the shims a vm
//! backend puts in its guest, and the one implementation of everything else, for any host's two backends.
//!
//! Nothing here names a host. The `exec` crate picks this host's backends and gives them to `Core`.

pub mod contract;
pub mod isa;
pub mod shim;

mod census;
mod cpu;
mod debug;
mod exec;
mod regs;
mod sandbox;
mod ticker;
mod translated;
mod tuning;
mod turn;
mod types;
mod unavailable;

pub use cpu::{Cpu, CpuHandle, Exit};
pub use exec::{Core, Opened};
pub use exec_mmu::{Frame, PAGE, Prot};
pub use regs::{Context, FpRegs, Regs};
pub use tuning::Tuning;
pub use turn::{Map, Memory, Space, Thread, Turn};
pub use types::{
    Backend, Caps, CpuId, Error, Isa, Isolation, Layout, MemorySize, MemoryStats, OpenError, Options, Page, PageCensus, Pressure,
    Resident, SpaceId, SpaceUsage, Stop,
};
pub use unavailable::{NoSandbox, NoSandboxCpu, NoVm, NoVmCpu, Why};
