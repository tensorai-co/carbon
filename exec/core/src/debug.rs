//! `CARBON_EXEC_DEBUG=1`: every stop, on every backend, logged the same way.

use crate::{Backend, Stop};

pub(crate) fn stop(backend: Backend, cpu: usize, stop: &Stop) {
    if !matches!(stop, Stop::Preempted) {
        eprintln!("exec [{backend} cpu {cpu}] {stop:x?}");
    }
}
