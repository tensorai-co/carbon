//! exec on macOS, Apple silicon: `vm` is Hypervisor.framework, `sandbox` is a stub task that runs only rewritten code,
//! so its system calls reach Carbon. A binary using the sandbox calls `run_if_stub()` first thing in `main`.
#![cfg(target_os = "macos")]

mod ram;
pub mod sandbox;
mod sys;
pub mod vm;

pub type Vm = vm::Hvf;
pub type Sandbox = sandbox::Stub;
pub use sandbox::run_if_stub;
