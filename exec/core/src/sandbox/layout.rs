//! Where programs may live. A vm backend's program has a space to itself below the shim; a sandbox's shares the stub's
//! process with whatever the host keeps there, which the host's `Sandbox::layout` reports.

use crate::isa::space_top;
use crate::{Isa, Layout};

/// A vm backend's layout: everything below the user top is the program's.
pub(crate) fn vm(isa: Isa) -> Layout {
    Layout { top: space_top(isa), kept: Vec::new(), gate: None }
}
