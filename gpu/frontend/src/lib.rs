//! gpu's frontends: the GPU code each API hands its driver, turned into gpu's IR.
//!
//! Built so far: SPIR-V compute shaders (`spirv`), as Vulkan programs and glslang write them. DXBC, DXIL and PTX are
//! to come, each a module beside it.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use core::fmt;

pub mod spirv;

/// Why a program couldn't be read: malformed, or using something not translated yet. Either way it names what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<gpu_ir::Error> for Error {
    fn from(e: gpu_ir::Error) -> Self {
        Error { message: alloc::format!("the IR made isn't valid: {}", e.message) }
    }
}

#[cfg(test)]
mod tests;
