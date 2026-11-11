//! The Mach calls the macOS sandbox makes, declared by hand, and the messages it sends and receives.
#![allow(non_camel_case_types, non_upper_case_globals)]

pub type kern_return_t = i32;
pub type mach_port_t = u32;

pub const KERN_SUCCESS: kern_return_t = 0;
pub const MACH_PORT_NULL: mach_port_t = 0;
pub const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
pub const MACH_MSG_TYPE_MOVE_SEND_ONCE: u32 = 18;
pub const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
pub const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
pub const MACH_SEND_MSG: i32 = 1;
pub const MACH_RCV_MSG: i32 = 2;

pub const VM_FLAGS_FIXED: i32 = 0;
pub const VM_PROT_READ: i32 = 1;
pub const VM_PROT_WRITE: i32 = 2;
pub const VM_PROT_EXECUTE: i32 = 4;
pub const VM_REGION_BASIC_INFO_64: i32 = 9;

pub const ARM_THREAD_STATE64: i32 = 6;
pub const ARM_EXCEPTION_STATE64: i32 = 7;
pub const ARM_NEON_STATE64: i32 = 17;

pub const EXC_BAD_ACCESS: i32 = 1;
pub const EXC_BAD_INSTRUCTION: i32 = 2;
pub const EXC_ARITHMETIC: i32 = 3;
pub const EXC_BREAKPOINT: i32 = 6;
pub const EXC_MASK_ALL_FAULTS: u32 = 1 << 1 | 1 << 2 | 1 << 3 | 1 << 6;
pub const EXCEPTION_DEFAULT: i32 = 1;
pub const MACH_EXCEPTION_CODES: i32 = 0x8000_0000_u32 as i32;
/// `mach_exception_raise`'s message id; its reply's is 100 more.
pub const MACH_EXCEPTION_RAISE: i32 = 2405;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Header {
    pub bits: u32,
    pub size: u32,
    pub remote: mach_port_t,
    pub local: mach_port_t,
    pub voucher: mach_port_t,
    pub id: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PortDescriptor {
    pub name: mach_port_t,
    pub pad1: u32,
    pub pad2: u16,
    pub disposition: u8,
    pub kind: u8,
}

/// `mach_exception_raise`, with 64-bit codes, as it arrives (packed to 4 bytes).
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct ExceptionRequest {
    pub header: Header,
    pub count: u32,
    pub thread: PortDescriptor,
    pub task: PortDescriptor,
    pub ndr: [u8; 8],
    pub exception: i32,
    pub code_count: u32,
    pub code: [i64; 2],
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct ExceptionReply {
    pub header: Header,
    pub ndr: [u8; 8],
    pub result: kern_return_t,
}

/// The little-endian, ASCII, IEEE NDR record every MIG reply carries.
pub const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];

/// `arm_thread_state64_t`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadState {
    pub x: [u64; 29],
    pub fp: u64,
    pub lr: u64,
    pub sp: u64,
    pub pc: u64,
    pub cpsr: u32,
    pub flags: u32,
}

/// `arm_neon_state64_t`.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct NeonState {
    pub q: [u128; 32],
    pub fpsr: u32,
    pub fpcr: u32,
}

/// `arm_exception_state64_t`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ExceptionState {
    pub far: u64,
    pub esr: u32,
    pub exception: u32,
}

/// The size of a state, in the `natural_t`s Mach counts it in.
pub const fn count<T>() -> u32 {
    (size_of::<T>() / 4) as u32
}

const _: () = assert!(count::<ThreadState>() == 68 && size_of::<ExceptionRequest>() == 84 && size_of::<ExceptionReply>() == 36);

unsafe extern "C" {
    static mach_task_self_: mach_port_t;
    pub fn mach_port_allocate(task: mach_port_t, right: u32, name: *mut mach_port_t) -> kern_return_t;
    pub fn mach_port_insert_right(task: mach_port_t, name: mach_port_t, right: mach_port_t, kind: u32) -> kern_return_t;
    pub fn mach_port_deallocate(task: mach_port_t, name: mach_port_t) -> kern_return_t;
    pub fn mach_port_mod_refs(task: mach_port_t, name: mach_port_t, right: u32, delta: i32) -> kern_return_t;
    pub fn task_for_pid(task: mach_port_t, pid: i32, target: *mut mach_port_t) -> kern_return_t;
    pub fn mach_msg(msg: *mut Header, option: i32, send_size: u32, receive_size: u32, receive: mach_port_t, timeout: u32, notify: mach_port_t) -> kern_return_t;
    pub fn mach_vm_allocate(task: mach_port_t, addr: *mut u64, size: u64, flags: i32) -> kern_return_t;
    pub fn mach_vm_deallocate(task: mach_port_t, addr: u64, size: u64) -> kern_return_t;
    pub fn mach_vm_protect(task: mach_port_t, addr: u64, size: u64, set_max: i32, prot: i32) -> kern_return_t;
    pub fn mach_vm_write(task: mach_port_t, addr: u64, data: usize, count: u32) -> kern_return_t;
    pub fn mach_vm_read_overwrite(task: mach_port_t, addr: u64, size: u64, data: u64, out: *mut u64) -> kern_return_t;
    pub fn mach_vm_region(task: mach_port_t, addr: *mut u64, size: *mut u64, flavor: i32, info: *mut u32, count: *mut u32, object: *mut mach_port_t) -> kern_return_t;
    pub fn thread_create(task: mach_port_t, thread: *mut mach_port_t) -> kern_return_t;
    pub fn thread_get_state(thread: mach_port_t, flavor: i32, state: *mut u32, count: *mut u32) -> kern_return_t;
    pub fn thread_set_state(thread: mach_port_t, flavor: i32, state: *const u32, count: u32) -> kern_return_t;
    pub fn thread_suspend(thread: mach_port_t) -> kern_return_t;
    pub fn thread_resume(thread: mach_port_t) -> kern_return_t;
    pub fn thread_set_exception_ports(thread: mach_port_t, mask: u32, port: mach_port_t, behavior: i32, flavor: i32) -> kern_return_t;
}

pub fn task_self() -> mach_port_t {
    // SAFETY: libSystem's own copy of this task's port name, set before main.
    unsafe { mach_task_self_ }
}

pub fn check(what: &str, result: kern_return_t) -> Result<(), String> {
    if result == KERN_SUCCESS { Ok(()) } else { Err(format!("{what}: Mach error {result:#x}")) }
}
