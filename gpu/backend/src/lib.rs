//! gpu's backends: its IR as a host GPU's own code, or run on the CPU.
//!
//! - `air`: AIR, the IR Apple's GPU drivers compile, written as the `.metallib` Metal loads. No source is
//!   compiled at pipeline creation.
//! - `cpu`: the IR interpreted, a workgroup at a time in lockstep, the way a GPU runs a subgroup. The software
//!   device is built on it.
//!
//! Unlike `gpu-ir` and `gpu-frontend` this crate uses `std`: the CPU backend needs the host's float functions, and both
//! backends only ever run host-side.
#![forbid(unsafe_code)]

pub mod air;
pub mod cpu;

use std::fmt;

/// Why a backend couldn't take a program or finish running one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error { message: message.into() })
}
