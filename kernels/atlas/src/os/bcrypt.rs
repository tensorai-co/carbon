//! bcrypt.dll: random numbers, from the host's.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "bcrypt.dll";
    host BCryptGenRandom(4) => b_crypt_gen_random;
}

const STATUS_INVALID_HANDLE: u64 = 0xc000_0008;
const STATUS_INVALID_PARAMETER: u64 = 0xc000_000d;
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u64 = 0x2;

/// `BCryptGenRandom(algorithm, buffer, length, flags)`: the system's generator, which is all there is, whether asked
/// for by BCRYPT_USE_SYSTEM_PREFERRED_RNG or by a handle.
fn b_crypt_gen_random(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 && a[3] & BCRYPT_USE_SYSTEM_PREFERRED_RNG == 0 {
        return Reply::Value(STATUS_INVALID_HANDLE);
    }
    let len = a[2] as u32 as usize;
    if (a[1] == 0 && len != 0) || !task.random_into(cx, a[1], len) {
        return Reply::Value(STATUS_INVALID_PARAMETER);
    }
    Reply::Value(0)
}
