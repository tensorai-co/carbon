//! bcrypt.dll: random numbers, from the host's.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "bcrypt.dll";
    host BCryptGenRandom(4) => b_crypt_gen_random;
    // Not written yet: what larger programs import.
    host BCryptCloseAlgorithmProvider(2) => b_crypt_close_algorithm_provider;
    host BCryptOpenAlgorithmProvider(4) => b_crypt_open_algorithm_provider;
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

/// An algorithm provider's handle: there's one generator, so every provider is the same.
const PROVIDER: u64 = 0xbc00;

/// `BCryptOpenAlgorithmProvider(&handle, algorithm, implementation, flags)`.
fn b_crypt_open_algorithm_provider(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[0] == 0 || task.write_ptr(cx, a[0], PROVIDER).is_err() {
        return Reply::Value(STATUS_INVALID_PARAMETER);
    }
    Reply::Value(0)
}

fn b_crypt_close_algorithm_provider(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[0] == PROVIDER { 0 } else { STATUS_INVALID_HANDLE })
}
