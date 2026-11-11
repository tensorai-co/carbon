//! Hypervisor.framework, declared by hand from its headers, with only what's used.
#![allow(non_camel_case_types)]

use std::arch::global_asm;
use std::ffi::c_void;

pub type hv_return_t = i32;
pub type hv_vcpu_t = u64;
pub const HV_SUCCESS: hv_return_t = 0;

pub const HV_MEMORY_READ: u64 = 1 << 0;
pub const HV_MEMORY_WRITE: u64 = 1 << 1;
pub const HV_MEMORY_EXEC: u64 = 1 << 2;

pub const HV_EXIT_REASON_CANCELED: u32 = 0;
pub const HV_EXIT_REASON_EXCEPTION: u32 = 1;

pub const HV_REG_X0: u32 = 0;
pub const HV_REG_PC: u32 = 31;
pub const HV_REG_FPCR: u32 = 32;
pub const HV_REG_FPSR: u32 = 33;
pub const HV_REG_CPSR: u32 = 34;

pub const HV_SYS_REG_SCTLR_EL1: u16 = 0xc080;
/// From macOS 15, which lets only EnTSO be set.
pub const HV_SYS_REG_ACTLR_EL1: u16 = 0xc081;
pub const HV_SYS_REG_CPACR_EL1: u16 = 0xc082;
pub const HV_SYS_REG_TTBR0_EL1: u16 = 0xc100;
pub const HV_SYS_REG_TTBR1_EL1: u16 = 0xc101;
pub const HV_SYS_REG_TCR_EL1: u16 = 0xc102;
pub const HV_SYS_REG_SPSR_EL1: u16 = 0xc200;
pub const HV_SYS_REG_ELR_EL1: u16 = 0xc201;
pub const HV_SYS_REG_SP_EL0: u16 = 0xc208;
pub const HV_SYS_REG_ESR_EL1: u16 = 0xc290;
pub const HV_SYS_REG_FAR_EL1: u16 = 0xc300;
pub const HV_SYS_REG_MAIR_EL1: u16 = 0xc510;
pub const HV_SYS_REG_VBAR_EL1: u16 = 0xc600;
pub const HV_SYS_REG_CNTKCTL_EL1: u16 = 0xc708;
pub const HV_SYS_REG_TPIDR_EL0: u16 = 0xde82;

/// ESR's exception class for a hypercall, as the exit's syndrome reports it.
pub const EC_HVC: u64 = 0x16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct hv_vcpu_exit_exception_t {
    pub syndrome: u64,
    pub virtual_address: u64,
    pub physical_address: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct hv_vcpu_exit_t {
    pub reason: u32,
    pub exception: hv_vcpu_exit_exception_t,
}

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    pub fn hv_vm_create(config: *mut c_void) -> hv_return_t;
    pub fn hv_vm_destroy() -> hv_return_t;
    pub fn hv_vm_map(addr: *mut c_void, ipa: u64, size: usize, flags: u64) -> hv_return_t;
    pub fn hv_vm_unmap(ipa: u64, size: usize) -> hv_return_t;
    pub fn hv_vm_get_max_vcpu_count(count: *mut u32) -> hv_return_t;
    pub fn hv_vcpu_create(vcpu: *mut hv_vcpu_t, exit: *mut *const hv_vcpu_exit_t, config: *mut c_void) -> hv_return_t;
    pub fn hv_vcpu_destroy(vcpu: hv_vcpu_t) -> hv_return_t;
    pub fn hv_vcpu_run(vcpu: hv_vcpu_t) -> hv_return_t;
    pub fn hv_vcpus_exit(vcpus: *const hv_vcpu_t, count: u32) -> hv_return_t;
    pub fn hv_vcpu_get_reg(vcpu: hv_vcpu_t, reg: u32, value: *mut u64) -> hv_return_t;
    pub fn hv_vcpu_set_reg(vcpu: hv_vcpu_t, reg: u32, value: u64) -> hv_return_t;
    pub fn hv_vcpu_get_simd_fp_reg(vcpu: hv_vcpu_t, reg: u32, value: *mut u128) -> hv_return_t;
    pub fn hv_vcpu_get_sys_reg(vcpu: hv_vcpu_t, reg: u16, value: *mut u64) -> hv_return_t;
    pub fn hv_vcpu_set_sys_reg(vcpu: hv_vcpu_t, reg: u16, value: u64) -> hv_return_t;
}

// `hv_vcpu_set_simd_fp_reg` takes its value in v0, which Rust can't pass to C, so this loads it from a pointer and
// jumps to the real function.
global_asm!(
    ".global _exec_hv_set_simd_fp_reg",
    ".p2align 2",
    "_exec_hv_set_simd_fp_reg:",
    "ldr q0, [x2]",
    "b _hv_vcpu_set_simd_fp_reg",
);

unsafe extern "C" {
    pub fn exec_hv_set_simd_fp_reg(vcpu: hv_vcpu_t, reg: u32, value: *const u128) -> hv_return_t;
    /// From libSystem: makes newly written code safe to run.
    pub fn sys_icache_invalidate(start: *mut c_void, len: usize);
}

pub fn check(what: &str, result: hv_return_t) -> Result<(), String> {
    if result == HV_SUCCESS { Ok(()) } else { Err(format!("{what}: Hypervisor.framework error {result:#x}")) }
}
