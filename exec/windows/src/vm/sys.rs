//! `driver/winkvm.h`, in Rust: the device, its calls and the structures they pass.

use std::ffi::c_void;

use exec_core::shim::x86::X86Segment;

use crate::win32::*;

pub const API_VERSION: u32 = 0x0200;
const DEVICE_TYPE: u32 = 0x8000;

const fn ctl(function: u32) -> u32 {
    // CTL_CODE(DEVICE_TYPE, function, METHOD_BUFFERED, FILE_ANY_ACCESS)
    DEVICE_TYPE << 16 | function << 2
}

pub const CAPS: u32 = ctl(0x800);
pub const CREATE_VM: u32 = ctl(0x801);
pub const DESTROY_VM: u32 = ctl(0x802);
pub const SET_MEMORY: u32 = ctl(0x803);
pub const CREATE_VCPU: u32 = ctl(0x804);
pub const SET_SYSTEM: u32 = ctl(0x805);
pub const GET_FP: u32 = ctl(0x806);
pub const SET_FP: u32 = ctl(0x807);
pub const RUN: u32 = ctl(0x808);
pub const CANCEL: u32 = ctl(0x809);

pub const EXIT_HLT: u32 = 1;
pub const EXIT_CANCELED: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Caps {
    pub version: u32,
    pub vmx: u32,
    pub ept: u32,
    pub max_vcpus: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VcpuId {
    pub vm: u32,
    pub vcpu: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Memory {
    pub vm: u32,
    pub reserved: u32,
    pub guest_phys: u64,
    pub size: u64,
    pub user_address: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub kind: u8,
    /// Bit 0 present, bits 1–2 DPL, bit 3 D/B, bit 4 S, bit 5 L, bit 6 G.
    pub flags: u8,
}

impl From<X86Segment> for Segment {
    fn from(s: X86Segment) -> Segment {
        let flags = u8::from(s.present) | (s.dpl & 3) << 1 | u8::from(s.db) << 3 | u8::from(s.s) << 4 | u8::from(s.l) << 5 | u8::from(s.g) << 6;
        Segment { base: s.base, limit: s.limit, selector: s.selector, kind: s.kind, flags }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Msr {
    pub index: u32,
    pub reserved: u32,
    pub value: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct System {
    pub vm: u32,
    pub vcpu: u32,
    pub cr0: u64,
    pub cr4: u64,
    pub efer: u64,
    pub xcr0: u64,
    pub gdt_base: u64,
    pub idt_base: u64,
    pub gdt_limit: u16,
    pub idt_limit: u16,
    pub msr_count: u32,
    pub tr: Segment,
    pub msrs: [Msr; 8],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Regs {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cs: Segment,
    pub ss: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Fp {
    pub vm: u32,
    pub vcpu: u32,
    pub fxsave: [u8; 512],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Run {
    pub vm: u32,
    pub vcpu: u32,
    pub exit_reason: u32,
    pub reserved: u32,
    pub qualification: u64,
    pub regs: Regs,
}

// The same sizes as `driver/winkvm.h`'s structures.
const _: () = assert!(
    size_of::<Caps>() == 16
        && size_of::<VcpuId>() == 8
        && size_of::<Memory>() == 32
        && size_of::<Segment>() == 16
        && size_of::<System>() == 208
        && size_of::<Regs>() == 256
        && size_of::<Fp>() == 520
        && size_of::<Run>() == 280
);

/// `\\.\WinKvm`, open for overlapped calls. On a handle without `FILE_FLAG_OVERLAPPED` the I/O manager runs one call
/// at a time, so a `CANCEL` from another thread would wait behind the `RUN` it's meant to stop, forever. Each call still
/// waits for its own result.
pub struct Device(HANDLE);

/// A thread's event for its overlapped calls: Windows resets it as each call starts.
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the event `CreateEventW` made for this thread.
        unsafe { CloseHandle(self.0) };
    }
}

thread_local! {
    // SAFETY: plain arguments: a manual-reset event, unsignalled, unnamed.
    static EVENT: Event = Event(unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, std::ptr::null()) });
}

// SAFETY: a device handle, which any thread may use.
unsafe impl Send for Device {}
// SAFETY: as above.
unsafe impl Sync for Device {}

impl Device {
    pub fn open() -> Result<Device, String> {
        let name = wide(r"\\.\WinKvm");
        // SAFETY: a NUL-terminated name and plain flags.
        let handle = unsafe { CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, std::ptr::null_mut()) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("\\\\.\\WinKvm (is winkvm.sys installed?): {}", std::io::Error::last_os_error()));
        }
        Ok(Device(handle))
    }

    /// A call with `input` in and `output` out.
    pub fn call<I, O>(&self, code: u32, input: &I, output: &mut O) -> Result<(), String> {
        EVENT.with(|event| {
            let mut returned = 0;
            let mut overlapped = OVERLAPPED { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: event.0 };
            // SAFETY: plain structures of the sizes given, as the driver's interface says, and an OVERLAPPED that
            // lives until the call has finished: it's waited for below before returning.
            let mut ok = unsafe {
                DeviceIoControl(
                    self.0,
                    code,
                    (input as *const I).cast::<c_void>(),
                    size_of::<I>() as u32,
                    (output as *mut O).cast::<c_void>(),
                    size_of::<O>() as u32,
                    &mut returned,
                    (&mut overlapped as *mut OVERLAPPED).cast::<c_void>(),
                )
            };
            if ok == 0 && std::io::Error::last_os_error().raw_os_error() == Some(ERROR_IO_PENDING) {
                // SAFETY: the call just started, with this OVERLAPPED.
                ok = unsafe { GetOverlappedResult(self.0, &mut overlapped, &mut returned, 1) };
            }
            if ok == 0 { Err(format!("winkvm call {code:#x}: {}", std::io::Error::last_os_error())) } else { Ok(()) }
        })
    }

    /// A call with only `input`.
    pub fn send<I>(&self, code: u32, input: &I) -> Result<(), String> {
        self.call(code, input, &mut ())
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: the handle `open` made.
        unsafe { CloseHandle(self.0) };
    }
}
