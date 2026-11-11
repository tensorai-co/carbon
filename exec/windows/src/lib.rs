//! exec on Windows x86-64: `vm` is Carbon's own VT-x driver, `sandbox` is a locked-down stub process Carbon debugs.
#![cfg(windows)]

mod ram;
#[cfg(target_arch = "x86_64")]
pub mod sandbox;
mod win32;
#[cfg(target_arch = "x86_64")]
pub mod vm;

#[cfg(target_arch = "x86_64")]
pub type Vm = vm::WinKvm;

#[cfg(not(target_arch = "x86_64"))]
pub type Vm = exec_core::NoVm<exec_core::shim::NativeShim, NoVmHere>;

#[cfg(not(target_arch = "x86_64"))]
pub struct NoVmHere;

#[cfg(not(target_arch = "x86_64"))]
impl exec_core::Why for NoVmHere {
    const NAME: &'static str = "winkvm";
    const WHY: &'static str = "winkvm is for x86-64";
}

#[cfg(target_arch = "x86_64")]
pub type Sandbox = sandbox::Stub;

#[cfg(not(target_arch = "x86_64"))]
pub type Sandbox = exec_core::NoSandbox<NoSandboxHere>;

#[cfg(not(target_arch = "x86_64"))]
pub struct NoSandboxHere;

#[cfg(not(target_arch = "x86_64"))]
impl exec_core::Why for NoSandboxHere {
    const NAME: &'static str = "windows";
    const WHY: &'static str = "the Windows sandbox is for x86-64";
}
