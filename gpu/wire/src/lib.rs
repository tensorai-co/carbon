//! What gpu's guest drivers (in programs, built for the guest) and the host share.
//!
//! - [`vk`]: Vulkan's types, constants and command signatures, generated from Khronos's `vk.xml` by `gpu-wire-gen`,
//!   for the commands lambda's Vulkan driver implements.
//! - The render node's protocol: lambda's `/dev/dri/renderD128` answers DRM's version query, as any DRM driver
//!   does, and the driver's own commands from DRM's driver range: [`INFO`], the device's description, and
//!   [`SUBMIT_IOCTL`], the doorbell that hands the host the [`records`] the driver has collected.
//!
//! `no_std`, with no `unsafe`: plain data both sides agree on, in the C ABI's layout, the same on ARM64 and x86-64.
#![no_std]
#![forbid(unsafe_code)]

pub mod records;
pub mod vk;

/// An ioctl number, as Linux makes them on ARM64 and x86-64 alike: direction, size, type and number.
pub const fn ioc(dir: u64, ty: u8, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((ty as u64) << 8) | nr as u64
}

pub const IOC_WRITE: u64 = 1;
pub const IOC_READ: u64 = 2;

/// DRM's ioctls are type `'d'`; a driver's own start at 0x40.
pub const DRM_TYPE: u8 = b'd';
pub const DRM_COMMAND_BASE: u8 = 0x40;

/// `struct drm_version`, on a 64-bit ABI: the driver's version and the user's buffers for its name, date and
/// description, each given back with its length (the user asks twice: once for the lengths, once with buffers).
pub const DRM_VERSION_SIZE: usize = 64;
pub const DRM_IOCTL_VERSION: u64 = ioc(IOC_READ | IOC_WRITE, DRM_TYPE, 0x00, DRM_VERSION_SIZE);

/// The render node's driver: what `drmGetVersion` says.
pub const DRIVER_NAME: &str = "carbon";
pub const DRIVER_DESCRIPTION: &str = "Carbon GPU";
pub const DRIVER_DATE: &str = "20261114";

/// The protocol's version: a driver and a kernel that disagree say so instead of misreading each other.
pub const VERSION: u32 = 1;

/// The guest's ID for the Carbon GPU: Vulkan's vendor and device IDs. Above 0xffff, so it's in Khronos's range
/// rather than PCI's, where no real driver's workarounds match it.
pub const VENDOR_ID: u32 = 0x1_c0de;
pub const DEVICE_ID: u32 = 0x0001;

/// What kind of device the guest has behind the render node.
pub const KIND_HOST: u32 = 1;
pub const KIND_SOFTWARE: u32 = 2;

/// The device's description, as [`INFO`] gives it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// [`VERSION`], the kernel's.
    pub version: u32,
    /// [`KIND_HOST`] or [`KIND_SOFTWARE`].
    pub kind: u32,
    pub subgroup_size: u32,
    pub max_workgroup_invocations: u32,
    /// The host's name for it (`Apple M1`, `software`), NUL-padded.
    pub name: [u8; 64],
    /// The backend driving it (`metal`, `software`), NUL-padded.
    pub backend: [u8; 16],
}

impl Info {
    pub const fn zeroed() -> Info {
        Info { version: 0, kind: 0, subgroup_size: 0, max_workgroup_invocations: 0, name: [0; 64], backend: [0; 16] }
    }

    /// The struct's bytes, as the ioctl copies them.
    pub fn to_bytes(&self) -> [u8; INFO_SIZE] {
        let mut b = [0u8; INFO_SIZE];
        b[0..4].copy_from_slice(&self.version.to_le_bytes());
        b[4..8].copy_from_slice(&self.kind.to_le_bytes());
        b[8..12].copy_from_slice(&self.subgroup_size.to_le_bytes());
        b[12..16].copy_from_slice(&self.max_workgroup_invocations.to_le_bytes());
        b[16..80].copy_from_slice(&self.name);
        b[80..96].copy_from_slice(&self.backend);
        b
    }

    pub fn from_bytes(b: &[u8; INFO_SIZE]) -> Info {
        let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut name = [0; 64];
        name.copy_from_slice(&b[16..80]);
        let mut backend = [0; 16];
        backend.copy_from_slice(&b[80..96]);
        Info { version: word(0), kind: word(4), subgroup_size: word(8), max_workgroup_invocations: word(12), name, backend }
    }
}

/// A text into a fixed NUL-padded field, cut to fit (leaving a NUL).
pub fn padded<const N: usize>(text: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let n = text.len().min(N - 1);
    out[..n].copy_from_slice(&text.as_bytes()[..n]);
    out
}

pub const INFO_SIZE: usize = core::mem::size_of::<Info>();
/// The device's description: `DRM_IOR(DRM_COMMAND_BASE + 0, Info)`.
pub const INFO: u64 = ioc(IOC_READ, DRM_TYPE, DRM_COMMAND_BASE, INFO_SIZE);

/// The doorbell: `struct { records, length, reply, reply_capacity }`, four 64-bit words. The host runs the records at
/// `records` (`length` bytes) and returns 0, or the length of a message saying what failed, written at `reply` as far
/// as `reply_capacity` allows. Work submitted is finished when it returns.
pub const SUBMIT_SIZE: usize = 32;
pub const SUBMIT_IOCTL: u64 = ioc(IOC_READ | IOC_WRITE, DRM_TYPE, DRM_COMMAND_BASE + 1, SUBMIT_SIZE);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(INFO_SIZE, 96);
        assert_eq!(DRM_IOCTL_VERSION, 0xc040_6400);
        let i = Info { version: 1, kind: KIND_HOST, subgroup_size: 32, max_workgroup_invocations: 1024, name: padded("Apple M1"), backend: padded("metal") };
        assert_eq!(Info::from_bytes(&i.to_bytes()), i);
    }

    #[test]
    fn records_read_back() {
        let mut payload = [0u8; 64];
        let mut n = 0;
        struct At<'a>(&'a mut [u8], &'a mut usize);
        impl Extend<u8> for At<'_> {
            fn extend<I: IntoIterator<Item = u8>>(&mut self, it: I) {
                for b in it {
                    self.0[*self.1] = b;
                    *self.1 += 1;
                }
            }
        }
        let b = records::Bound { set: 1, binding: 2, memory: 0xdead_beef_0000, offset: 16, size: 48 };
        records::put_bound(&mut At(&mut payload, &mut n), &b);
        records::put_bytes(&mut At(&mut payload, &mut n), b"main");
        let mut whole = [0u8; 128];
        let mut m = 0;
        records::record(&mut At(&mut whole, &mut m), records::DISPATCH, &payload[..n]);
        let mut r = records::Reader::new(&whole[..m]);
        let (kind, mut p) = r.record().unwrap();
        assert_eq!(kind, records::DISPATCH);
        assert_eq!(p.bound().unwrap(), b);
        assert_eq!(p.bytes().unwrap(), b"main");
        assert!(p.is_empty() && r.is_empty());
    }

    /// The generated structs' sizes, as `vulkan_core.h` compiled by clang has them on a 64-bit ABI.
    #[test]
    fn vulkan_layouts_are_c_layouts() {
        use core::mem::{offset_of, size_of};
        use vk::*;
        assert_eq!(size_of::<VkPhysicalDeviceProperties>(), 824);
        assert_eq!(offset_of!(VkPhysicalDeviceProperties, limits), 296);
        assert_eq!(size_of::<VkPhysicalDeviceLimits>(), 504);
        assert_eq!(size_of::<VkPhysicalDeviceFeatures>(), 220);
        assert_eq!(size_of::<VkPhysicalDeviceMemoryProperties>(), 520);
        assert_eq!(size_of::<VkQueueFamilyProperties>(), 24);
        assert_eq!(size_of::<VkExtensionProperties>(), 260);
        assert_eq!(size_of::<VkInstanceCreateInfo>(), 64);
        assert_eq!(size_of::<VkDeviceCreateInfo>(), 72);
        assert_eq!(size_of::<VkPhysicalDeviceProperties2>(), 840);
        assert_eq!(size_of::<VkFormatProperties>(), 12);
        assert_eq!(size_of::<VkImageFormatProperties>(), 32);
        assert_eq!(size_of::<VkPhysicalDeviceIDProperties>(), 64);
        assert_eq!(size_of::<VkPhysicalDeviceDriverProperties>(), 536);
    }
}
