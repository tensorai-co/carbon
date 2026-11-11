//! exec on Linux: `vm` is KVM, `sandbox` is a stub process driven by `ptrace`, with Syscall User Dispatch sending its
//! system calls to the gate.
#![cfg(target_os = "linux")]

#[cfg(target_arch = "x86_64")]
mod preempt;
mod ram;
pub mod sandbox;
#[cfg(target_arch = "x86_64")]
pub mod vm;

#[cfg(target_arch = "x86_64")]
pub type Vm = vm::Kvm;

/// KVM on ARM64 hosts, with the ARM64 shim, comes later.
#[cfg(not(target_arch = "x86_64"))]
pub type Vm = exec_core::NoVm<exec_core::shim::NativeShim, NoKvmYet>;

#[cfg(not(target_arch = "x86_64"))]
pub struct NoKvmYet;

#[cfg(not(target_arch = "x86_64"))]
impl exec_core::Why for NoKvmYet {
    const NAME: &'static str = "kvm";
    const WHY: &'static str = "KVM on this CPU isn't built yet";
}

#[cfg(target_arch = "x86_64")]
pub type Sandbox = sandbox::Stub;

/// The sandbox on ARM64 hosts comes later.
#[cfg(not(target_arch = "x86_64"))]
pub type Sandbox = exec_core::NoSandbox<NoSandboxYet>;

#[cfg(not(target_arch = "x86_64"))]
pub struct NoSandboxYet;

#[cfg(not(target_arch = "x86_64"))]
impl exec_core::Why for NoSandboxYet {
    const NAME: &'static str = "linux";
    const WHY: &'static str = "the Linux sandbox on this CPU isn't built yet";
}
