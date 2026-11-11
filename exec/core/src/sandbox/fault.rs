//! What a host's fault means to a kernel. Every host catches faults its own way and reports a `HostFault`; turning it
//! into a `Stop` happens only here.

use crate::Stop;
use crate::contract::{FaultKind, HostFault};

pub(crate) fn to_stop(fault: HostFault) -> Stop {
    match fault.kind {
        FaultKind::Segv => Stop::Fault { addr: fault.addr, access: fault.access, pc: fault.pc },
        FaultKind::Ill => Stop::Undefined { pc: fault.pc },
        FaultKind::Trap => Stop::Breakpoint { pc: fault.pc },
        FaultKind::Bus => Stop::Alignment { pc: fault.pc },
        FaultKind::DivZero => Stop::DivideError { pc: fault.pc },
        FaultKind::FastFail => Stop::FastFail { pc: fault.pc, code: fault.addr },
    }
}
