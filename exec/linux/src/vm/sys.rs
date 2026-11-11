//! KVM's interface on x86-64, as `linux/kvm.h` defines it: the ioctls used, and the structures they pass.

pub const API_VERSION: u64 = 0xae00;
pub const CREATE_VM: u64 = 0xae01;
pub const CHECK_EXTENSION: u64 = 0xae03;
pub const GET_VCPU_MMAP_SIZE: u64 = 0xae04;
pub const GET_SUPPORTED_CPUID: u64 = 0xc008_ae05;
pub const CREATE_VCPU: u64 = 0xae41;
pub const SET_USER_MEMORY_REGION: u64 = 0x4020_ae46;
pub const SET_TSS_ADDR: u64 = 0xae47;
pub const SET_IDENTITY_MAP_ADDR: u64 = 0x4008_ae48;
pub const RUN: u64 = 0xae80;
pub const SET_REGS: u64 = 0x4090_ae82;
pub const GET_SREGS: u64 = 0x8138_ae83;
pub const SET_SREGS: u64 = 0x4138_ae84;
pub const SET_MSRS: u64 = 0x4008_ae89;
pub const GET_FPU: u64 = 0x81a0_ae8c;
pub const SET_FPU: u64 = 0x41a0_ae8d;
pub const SET_CPUID2: u64 = 0x4008_ae90;
pub const SET_XCRS: u64 = 0x4188_aea7;

pub const CAP_MAX_VCPUS: u64 = 66;
pub const CAP_SYNC_REGS: u64 = 74;
pub const CAP_IMMEDIATE_EXIT: u64 = 136;

pub const EXIT_HLT: u32 = 5;
pub const EXIT_INTR: u32 = 10;
pub const EXIT_FAIL_ENTRY: u32 = 9;

/// Registers shared through `kvm_run`, with no ioctl each time.
pub const SYNC_REGS: u64 = 1;
pub const SYNC_SREGS: u64 = 2;

/// Where things are in `kvm_run`.
pub const RUN_IMMEDIATE_EXIT: usize = 1;
pub const RUN_EXIT_REASON: usize = 8;
/// `fail_entry.hardware_entry_failure_reason`, for `EXIT_FAIL_ENTRY`.
pub const RUN_FAIL_ENTRY_REASON: usize = 32;
pub const RUN_VALID_REGS: usize = 288;
pub const RUN_DIRTY_REGS: usize = 296;
pub const RUN_REGS: usize = 304;
pub const RUN_SREGS: usize = RUN_REGS + size_of::<Regs>();

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Regs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl Regs {
    /// From x86's encoding order: rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8–r15.
    pub fn from_gpr(gpr: &[u64; 16], rip: u64, rflags: u64) -> Regs {
        let g = gpr;
        Regs {
            rax: g[0], rcx: g[1], rdx: g[2], rbx: g[3], rsp: g[4], rbp: g[5], rsi: g[6], rdi: g[7],
            r8: g[8], r9: g[9], r10: g[10], r11: g[11], r12: g[12], r13: g[13], r14: g[14], r15: g[15],
            rip, rflags,
        }
    }

    pub fn gpr(&self) -> [u64; 16] {
        [self.rax, self.rcx, self.rdx, self.rbx, self.rsp, self.rbp, self.rsi, self.rdi, self.r8, self.r9, self.r10, self.r11, self.r12, self.r13, self.r14, self.r15]
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub kind: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DescriptorTable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Sregs {
    pub cs: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub ss: Segment,
    pub tr: Segment,
    pub ldt: Segment,
    pub gdt: DescriptorTable,
    pub idt: DescriptorTable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Fpu {
    pub fpr: [[u8; 16]; 8],
    pub fcw: u16,
    pub fsw: u16,
    pub ftwx: u8,
    pub pad1: u8,
    pub last_opcode: u16,
    pub last_ip: u64,
    pub last_dp: u64,
    pub xmm: [[u8; 16]; 16],
    pub mxcsr: u32,
    pub pad2: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

#[repr(C)]
pub struct MemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

#[repr(C)]
pub struct Xcr {
    pub xcr: u32,
    pub reserved: u32,
    pub value: u64,
}

#[repr(C)]
pub struct Xcrs {
    pub count: u32,
    pub flags: u32,
    pub xcrs: [Xcr; 16],
    pub padding: [u64; 16],
}

const _: () = assert!(size_of::<Regs>() == 144 && size_of::<Sregs>() == 312 && size_of::<Fpu>() == 416 && size_of::<Xcrs>() == 392);

/// An ioctl with a pointer argument: its result, or the error number.
///
/// # Safety
///
/// `argument` is what this ioctl takes.
pub unsafe fn ioctl<T>(fd: i32, request: u64, argument: *mut T) -> Result<i32, i32> {
    // SAFETY: as the caller says.
    let result = unsafe { libc::ioctl(fd, request as libc::Ioctl, argument) };
    if result < 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) } else { Ok(result) }
}

/// An ioctl that takes a number.
pub fn ioctl_value(fd: i32, request: u64, value: u64) -> Result<i32, i32> {
    // SAFETY: these ioctls take a plain number.
    let result = unsafe { libc::ioctl(fd, request as libc::Ioctl, value) };
    if result < 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) } else { Ok(result) }
}
