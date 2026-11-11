//! Policy, not facts: numbers someone might want to change, with the defaults Carbon runs with.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Tuning {
    /// How long a program runs before it's preempted, once time slices are on.
    pub time_slice: Duration,
    /// Free memory to keep backed, so a program that allocates and frees in a loop doesn't go to the host each time.
    /// Under memory pressure it's 0.
    pub keep_free: u64,
    /// Don't give back less than this at once.
    pub give_back_min: u64,
    /// The most pieces to give back at once, as each holds the kernel's lock.
    pub give_back_batch: usize,
    /// How much translated x86 code to write before a translated space's code cache moves to its other half: at most
    /// 448 MiB, which leaves 64 MiB of the half for while a CPU is still finishing in the other one. Smaller only to
    /// test moving between halves.
    pub code_cache: u64,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning { time_slice: Duration::from_millis(10), keep_free: 64 << 20, give_back_min: 16 << 20, give_back_batch: 1024, code_cache: 448 << 20 }
    }
}
