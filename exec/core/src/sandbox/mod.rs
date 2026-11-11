//! Driving a sandbox backend: the parts of a sandbox that aren't about any host.
//!
//! A host's sandbox only starts its stub, maps one page into it, moves registers, resumes it and preempts it. Keeping
//! the stub's memory matching core's tables (`mirror`), what the registers mean at the gate (`gate`), what a host's
//! fault means to a kernel (`fault`) and where programs may live (`layout`) are all here, once.

pub(crate) mod fault;
pub(crate) mod gate;
pub(crate) mod layout;
mod mirror;

pub(crate) use mirror::Mirror;

use crate::contract::SandboxExit;
use crate::isa::{arm64, x86};
use crate::{Regs, Stop};

/// What a sandbox's exit means to a kernel.
pub(crate) fn stop(exit: SandboxExit, regs: &Regs) -> Stop {
    match exit {
        SandboxExit::Gate(trap) => match regs {
            Regs::X86(r) => x86::syscall(trap, r),
            Regs::Arm64(r) => arm64::syscall(trap, r),
        },
        SandboxExit::Fault(fault) => fault::to_stop(fault),
        SandboxExit::Preempted => Stop::Preempted,
        SandboxExit::Failed(code) => Stop::Internal { pc: regs.pc(), detail: code },
    }
}
