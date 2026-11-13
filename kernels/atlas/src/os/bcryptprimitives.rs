//! bcryptprimitives.dll: the process's random generator, from the host's.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "BCRYPTPRIMITIVES.dll";
    host ProcessPrng(2) => process_prng;
}

/// `ProcessPrng(buffer, length)`: random bytes; it always succeeds (TRUE), as Windows documents, unless the buffer
/// isn't the program's to write.
fn process_prng(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let len = a[1] as usize;
    Reply::Value(u64::from(len == 0 || (a[0] != 0 && task.random_into(cx, a[0], len))))
}
