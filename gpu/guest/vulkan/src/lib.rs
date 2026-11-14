//! lambda's Vulkan driver: the ICD the image's own Vulkan loader finds through the system layer's manifest and loads
//! into a program. It opens the machine's render node (`/dev/dri/renderD128`), asks it what the device is
//! (`gpu_wire::INFO`), and gives programs a **Carbon GPU**: a vendor and device ID of Carbon's own, with the host
//! device's real name and limits. No render node (a machine with `--gpu none`) means no device: `vkCreateInstance`
//! says the driver doesn't fit, and the loader goes on to the image's other drivers.
//!
//! Built so far: Vulkan 1.0's instance, physical device and device (what `vulkaninfo` reads), compute and graphics
//! (`device`): memory, buffers, images, samplers, descriptor sets, compute and graphics pipelines, render passes,
//! command buffers, the queue and fences; and surfaces and swapchains on Wayland and X11 (`wsi`). The driver keeps buffers,
//! descriptor sets and command buffers itself and hands the host memory, pipelines and work as `gpu_wire::records`,
//! through the render node's doorbell; `gpu-api` lowers them onto the device.
//!
//! It's `no_std`, over the C library the program has already (`malloc`, `open`, `ioctl`), bound by the dynamic loader
//! when the driver's loaded.
#![no_std]
#![allow(non_snake_case, clippy::missing_safety_doc)]

extern crate alloc;

mod device;
mod sync;
mod wsi;

use alloc::boxed::Box;
use core::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use core::ptr;

use gpu_wire::vk::*;

// The C library's, from the program.
unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn posix_memalign(out: *mut *mut c_void, align: usize, size: usize) -> c_int;
    fn free(p: *mut c_void);
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
    pub(crate) fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub(crate) fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    pub(crate) fn munmap(addr: *mut c_void, len: usize) -> c_int;
    pub(crate) fn write(fd: c_int, p: *const c_void, n: usize) -> isize;
    fn abort() -> !;
}

struct Malloc;

unsafe impl core::alloc::GlobalAlloc for Malloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() <= 16 {
            return unsafe { malloc(layout.size().max(1)) as *mut u8 };
        }
        let mut p = ptr::null_mut();
        if unsafe { posix_memalign(&mut p, layout.align(), layout.size().max(1)) } != 0 {
            return ptr::null_mut();
        }
        p as *mut u8
    }

    unsafe fn dealloc(&self, p: *mut u8, _: core::alloc::Layout) {
        unsafe { free(p as *mut c_void) }
    }
}

#[global_allocator]
static ALLOCATOR: Malloc = Malloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { abort() }
}

/// What the loader finds at the start of every dispatchable object, until it writes its own table there.
pub(crate) const ICD_LOADER_MAGIC: usize = 0x01CD_C0DE;
/// The loader–driver interface this driver speaks: 5 takes any `apiVersion` an application asks for.
const LOADER_INTERFACE: u32 = 5;
const O_RDWR: c_int = 2;
const O_CLOEXEC: c_int = 0o2000000;

const fn api_version(major: u32, minor: u32, patch: u32) -> u32 {
    major << 22 | minor << 12 | patch
}
/// The Vulkan this driver implements.
const API_VERSION: u32 = api_version(1, 0, 0);
const DRIVER_VERSION: u32 = api_version(0, 2, 0);

/// Dispatchable: the loader's slot first.
#[repr(C)]
struct Instance {
    #[allow(dead_code)]
    loader: usize,
    /// The machine's device, or null where the render node's description couldn't be read.
    physical: *mut Physical,
}

/// Dispatchable: the loader's slot first.
#[repr(C)]
pub(crate) struct Physical {
    #[allow(dead_code)]
    loader: usize,
    pub(crate) fd: c_int,
    info: gpu_wire::Info,
}

/// The render node's device description, or None if there's no node or it isn't this driver's.
fn open_render_node() -> Option<(c_int, gpu_wire::Info)> {
    let fd = unsafe { open(c"/dev/dri/renderD128".as_ptr(), O_RDWR | O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let mut bytes = [0u8; gpu_wire::INFO_SIZE];
    if unsafe { ioctl(fd, gpu_wire::INFO as c_ulong, bytes.as_mut_ptr()) } != 0 {
        unsafe { close(fd) };
        return None;
    }
    let info = gpu_wire::Info::from_bytes(&bytes);
    if info.version != gpu_wire::VERSION {
        unsafe { close(fd) };
        return None;
    }
    Some((fd, info))
}

/// Writes a list the Vulkan way: the count if `out` is null, else as many as fit, and `VK_INCOMPLETE` if not all did.
pub(crate) unsafe fn write_list<T: Copy>(items: &[T], count: *mut u32, out: *mut T) -> VkResult {
    unsafe {
        if out.is_null() {
            *count = items.len() as u32;
            return VK_SUCCESS;
        }
        let n = (*count as usize).min(items.len());
        ptr::copy_nonoverlapping(items.as_ptr(), out, n);
        *count = n as u32;
        if n < items.len() { VK_INCOMPLETE } else { VK_SUCCESS }
    }
}

/// A NUL-terminated text into a fixed `char` array, cut to fit.
fn text<const N: usize>(out: &mut [c_char; N], s: &[u8]) {
    let n = s.len().min(N - 1);
    for (o, b) in out.iter_mut().zip(&s[..n]) {
        *o = *b as c_char;
    }
    out[n] = 0;
}

fn info_name(info: &gpu_wire::Info) -> &[u8] {
    let end = info.name.iter().position(|b| *b == 0).unwrap_or(info.name.len());
    &info.name[..end]
}

// The loader's way in.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vk_icdNegotiateLoaderICDInterfaceVersion(version: *mut u32) -> VkResult {
    unsafe {
        if *version < 1 {
            return VK_ERROR_INCOMPATIBLE_DRIVER;
        }
        *version = (*version).min(LOADER_INTERFACE);
    }
    VK_SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vk_icdGetInstanceProcAddr(_instance: VkInstance, name: *const c_char) -> PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    lookup(unsafe { CStr::from_ptr(name) }.to_bytes(), Level::Instance)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vk_icdGetPhysicalDeviceProcAddr(_instance: VkInstance, name: *const c_char) -> PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    lookup(unsafe { CStr::from_ptr(name) }.to_bytes(), Level::Physical)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Physical,
    Instance,
}

macro_rules! entry {
    ($name:literal, $level:expr, $f:expr) => {
        ($name.as_bytes(), $level, $f as *const ())
    };
}

/// Every command this driver has, by name, and the narrowest level it's reached at: a device's commands (`device`)
/// are reached from the instance too, a physical device's from `vk_icdGetPhysicalDeviceProcAddr`.
fn lookup(name: &[u8], at: Level) -> PFN_vkVoidFunction {
    use Level::*;
    let table: &[(&[u8], Level, *const ())] = &[
        entry!("vkCreateInstance", Instance, vkCreateInstance as PFN_vkCreateInstance),
        entry!("vkDestroyInstance", Instance, vkDestroyInstance as PFN_vkDestroyInstance),
        entry!("vkEnumerateInstanceExtensionProperties", Instance, vkEnumerateInstanceExtensionProperties as PFN_vkEnumerateInstanceExtensionProperties),
        entry!("vkEnumerateInstanceVersion", Instance, vkEnumerateInstanceVersion as PFN_vkEnumerateInstanceVersion),
        entry!("vkEnumeratePhysicalDevices", Instance, vkEnumeratePhysicalDevices as PFN_vkEnumeratePhysicalDevices),
        entry!("vkGetInstanceProcAddr", Instance, vk_icdGetInstanceProcAddr as PFN_vkGetInstanceProcAddr),
        entry!("vkCreateDevice", Physical, device::vkCreateDevice as PFN_vkCreateDevice),
        entry!("vkGetPhysicalDeviceSurfaceSupportKHR", Physical, wsi::vkGetPhysicalDeviceSurfaceSupportKHR as PFN_vkGetPhysicalDeviceSurfaceSupportKHR),
        entry!("vkGetPhysicalDeviceSurfaceCapabilitiesKHR", Physical, wsi::vkGetPhysicalDeviceSurfaceCapabilitiesKHR as PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR),
        entry!("vkGetPhysicalDeviceSurfaceFormatsKHR", Physical, wsi::vkGetPhysicalDeviceSurfaceFormatsKHR as PFN_vkGetPhysicalDeviceSurfaceFormatsKHR),
        entry!("vkGetPhysicalDeviceSurfacePresentModesKHR", Physical, wsi::vkGetPhysicalDeviceSurfacePresentModesKHR as PFN_vkGetPhysicalDeviceSurfacePresentModesKHR),
        entry!("vkGetPhysicalDeviceWaylandPresentationSupportKHR", Physical, wsi::vkGetPhysicalDeviceWaylandPresentationSupportKHR as unsafe extern "C" fn(VkPhysicalDevice, u32, *mut c_void) -> VkBool32),
        entry!("vkGetPhysicalDeviceXcbPresentationSupportKHR", Physical, wsi::vkGetPhysicalDeviceXcbPresentationSupportKHR as unsafe extern "C" fn(VkPhysicalDevice, u32, *mut c_void, u32) -> VkBool32),
        entry!("vkGetPhysicalDeviceXlibPresentationSupportKHR", Physical, wsi::vkGetPhysicalDeviceXlibPresentationSupportKHR as unsafe extern "C" fn(VkPhysicalDevice, u32, *mut c_void, c_ulong) -> VkBool32),
        entry!("vkGetPhysicalDeviceProperties", Physical, vkGetPhysicalDeviceProperties as PFN_vkGetPhysicalDeviceProperties),
        entry!("vkGetPhysicalDeviceProperties2", Physical, vkGetPhysicalDeviceProperties2 as PFN_vkGetPhysicalDeviceProperties2),
        entry!("vkGetPhysicalDeviceProperties2KHR", Physical, vkGetPhysicalDeviceProperties2 as PFN_vkGetPhysicalDeviceProperties2),
        entry!("vkGetPhysicalDeviceFeatures", Physical, vkGetPhysicalDeviceFeatures as PFN_vkGetPhysicalDeviceFeatures),
        entry!("vkGetPhysicalDeviceFeatures2", Physical, vkGetPhysicalDeviceFeatures2 as PFN_vkGetPhysicalDeviceFeatures2),
        entry!("vkGetPhysicalDeviceFeatures2KHR", Physical, vkGetPhysicalDeviceFeatures2 as PFN_vkGetPhysicalDeviceFeatures2),
        entry!("vkGetPhysicalDeviceMemoryProperties", Physical, vkGetPhysicalDeviceMemoryProperties as PFN_vkGetPhysicalDeviceMemoryProperties),
        entry!("vkGetPhysicalDeviceMemoryProperties2", Physical, vkGetPhysicalDeviceMemoryProperties2 as PFN_vkGetPhysicalDeviceMemoryProperties2),
        entry!("vkGetPhysicalDeviceMemoryProperties2KHR", Physical, vkGetPhysicalDeviceMemoryProperties2 as PFN_vkGetPhysicalDeviceMemoryProperties2),
        entry!("vkGetPhysicalDeviceQueueFamilyProperties", Physical, vkGetPhysicalDeviceQueueFamilyProperties as PFN_vkGetPhysicalDeviceQueueFamilyProperties),
        entry!("vkGetPhysicalDeviceQueueFamilyProperties2", Physical, vkGetPhysicalDeviceQueueFamilyProperties2 as PFN_vkGetPhysicalDeviceQueueFamilyProperties2),
        entry!("vkGetPhysicalDeviceQueueFamilyProperties2KHR", Physical, vkGetPhysicalDeviceQueueFamilyProperties2 as PFN_vkGetPhysicalDeviceQueueFamilyProperties2),
        entry!("vkGetPhysicalDeviceFormatProperties", Physical, vkGetPhysicalDeviceFormatProperties as PFN_vkGetPhysicalDeviceFormatProperties),
        entry!("vkGetPhysicalDeviceFormatProperties2", Physical, vkGetPhysicalDeviceFormatProperties2 as PFN_vkGetPhysicalDeviceFormatProperties2),
        entry!("vkGetPhysicalDeviceFormatProperties2KHR", Physical, vkGetPhysicalDeviceFormatProperties2 as PFN_vkGetPhysicalDeviceFormatProperties2),
        entry!("vkGetPhysicalDeviceImageFormatProperties", Physical, vkGetPhysicalDeviceImageFormatProperties as PFN_vkGetPhysicalDeviceImageFormatProperties),
        entry!("vkGetPhysicalDeviceImageFormatProperties2", Physical, vkGetPhysicalDeviceImageFormatProperties2 as PFN_vkGetPhysicalDeviceImageFormatProperties2),
        entry!("vkGetPhysicalDeviceImageFormatProperties2KHR", Physical, vkGetPhysicalDeviceImageFormatProperties2 as PFN_vkGetPhysicalDeviceImageFormatProperties2),
        entry!("vkGetPhysicalDeviceSparseImageFormatProperties", Physical, vkGetPhysicalDeviceSparseImageFormatProperties as PFN_vkGetPhysicalDeviceSparseImageFormatProperties),
        entry!("vkGetPhysicalDeviceSparseImageFormatProperties2", Physical, vkGetPhysicalDeviceSparseImageFormatProperties2 as PFN_vkGetPhysicalDeviceSparseImageFormatProperties2),
        entry!("vkGetPhysicalDeviceSparseImageFormatProperties2KHR", Physical, vkGetPhysicalDeviceSparseImageFormatProperties2 as PFN_vkGetPhysicalDeviceSparseImageFormatProperties2),
        entry!("vkEnumerateDeviceExtensionProperties", Physical, vkEnumerateDeviceExtensionProperties as PFN_vkEnumerateDeviceExtensionProperties),
        entry!("vkEnumerateDeviceLayerProperties", Physical, vkEnumerateDeviceLayerProperties as PFN_vkEnumerateDeviceLayerProperties),
    ];
    let found = table.iter().find(|(n, level, _)| *n == name && *level <= at).map(|(_, _, f)| *f).or_else(|| device::lookup(name));
    found.map(|f| unsafe { core::mem::transmute::<*const (), unsafe extern "C" fn()>(f) })
}

// The instance.

/// The instance extensions: getting physical devices' properties through `pNext` chains, which the `*2` commands do,
/// and surfaces on Wayland and X11 (`wsi`), which the loader makes and the driver reads.
const INSTANCE_EXTENSIONS: [(&[u8], u32); 5] = [
    (b"VK_KHR_get_physical_device_properties2", 2),
    (b"VK_KHR_surface", 25),
    (b"VK_KHR_wayland_surface", 6),
    (b"VK_KHR_xcb_surface", 6),
    (b"VK_KHR_xlib_surface", 6),
];

/// The device extensions: swapchains (`wsi`).
pub(crate) const DEVICE_EXTENSIONS: [(&[u8], u32); 1] = [(b"VK_KHR_swapchain", 70)];

pub unsafe extern "C" fn vkEnumerateInstanceExtensionProperties(layer: *const c_char, count: *mut u32, out: *mut VkExtensionProperties) -> VkResult {
    if !layer.is_null() {
        return VK_ERROR_LAYER_NOT_PRESENT;
    }
    let mut list = [VkExtensionProperties { extensionName: [0; VK_MAX_EXTENSION_NAME_SIZE as usize], specVersion: 0 }; INSTANCE_EXTENSIONS.len()];
    for (slot, (name, version)) in list.iter_mut().zip(INSTANCE_EXTENSIONS) {
        text(&mut slot.extensionName, name);
        slot.specVersion = version;
    }
    unsafe { write_list(&list, count, out) }
}

pub unsafe extern "C" fn vkEnumerateInstanceVersion(version: *mut u32) -> VkResult {
    unsafe { *version = API_VERSION };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkCreateInstance(_info: *const VkInstanceCreateInfo, _allocator: *const VkAllocationCallbacks, out: *mut VkInstance) -> VkResult {
    // No render node, or not this driver's: the loader takes another driver, or says there's no device.
    let Some((fd, info)) = open_render_node() else { return VK_ERROR_INCOMPATIBLE_DRIVER };
    let physical = Box::into_raw(Box::new(Physical { loader: ICD_LOADER_MAGIC, fd, info }));
    let instance = Box::into_raw(Box::new(Instance { loader: ICD_LOADER_MAGIC, physical }));
    unsafe { *out = instance as VkInstance };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyInstance(instance: VkInstance, _allocator: *const VkAllocationCallbacks) {
    if instance.is_null() {
        return;
    }
    let instance = unsafe { Box::from_raw(instance as *mut Instance) };
    if !instance.physical.is_null() {
        let physical = unsafe { Box::from_raw(instance.physical) };
        unsafe { close(physical.fd) };
    }
}

pub unsafe extern "C" fn vkEnumeratePhysicalDevices(instance: VkInstance, count: *mut u32, out: *mut VkPhysicalDevice) -> VkResult {
    let instance = unsafe { &*(instance as *const Instance) };
    let list = [instance.physical as VkPhysicalDevice];
    unsafe { write_list(if instance.physical.is_null() { &[] } else { &list }, count, out) }
}

// The physical device.

unsafe fn physical<'a>(p: VkPhysicalDevice) -> &'a Physical {
    unsafe { &*(p as *const Physical) }
}

/// Carbon's GPU, as a program sees it: our vendor and device ID, the host device's name, and limits every device
/// lambda's driver runs on meets.
fn properties(p: &Physical) -> VkPhysicalDeviceProperties {
    let host = p.info.kind == gpu_wire::KIND_HOST;
    let mut out = VkPhysicalDeviceProperties {
        apiVersion: API_VERSION,
        driverVersion: DRIVER_VERSION,
        vendorID: gpu_wire::VENDOR_ID,
        deviceID: gpu_wire::DEVICE_ID,
        deviceType: if host { VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU } else { VK_PHYSICAL_DEVICE_TYPE_CPU },
        deviceName: [0; VK_MAX_PHYSICAL_DEVICE_NAME_SIZE as usize],
        pipelineCacheUUID: *b"carbon-gpu-v0002",
        limits: limits(p),
        sparseProperties: VkPhysicalDeviceSparseProperties {
            residencyStandard2DBlockShape: VK_FALSE,
            residencyStandard2DMultisampleBlockShape: VK_FALSE,
            residencyStandard3DBlockShape: VK_FALSE,
            residencyAlignedMipSize: VK_FALSE,
            residencyNonResidentStrict: VK_FALSE,
        },
    };
    let mut name = [0u8; 96];
    let prefix = b"Carbon GPU (";
    let host_name = info_name(&p.info);
    let mut n = 0;
    for part in [prefix.as_slice(), host_name, b")"] {
        for b in part {
            if n < name.len() {
                name[n] = *b;
                n += 1;
            }
        }
    }
    text(&mut out.deviceName, &name[..n]);
    out
}

/// Vulkan 1.0's required limits, and the device's own where it reports them (its workgroups).
fn limits(p: &Physical) -> VkPhysicalDeviceLimits {
    let invocations = p.info.max_workgroup_invocations.max(128);
    VkPhysicalDeviceLimits {
        maxImageDimension1D: 8192,
        maxImageDimension2D: 8192,
        maxImageDimension3D: 2048,
        maxImageDimensionCube: 8192,
        maxImageArrayLayers: 2048,
        maxTexelBufferElements: 65536,
        maxUniformBufferRange: 65536,
        maxStorageBufferRange: 1 << 30,
        maxPushConstantsSize: 128,
        maxMemoryAllocationCount: 4096,
        maxSamplerAllocationCount: 4000,
        bufferImageGranularity: 1,
        sparseAddressSpaceSize: 0,
        maxBoundDescriptorSets: 4,
        maxPerStageDescriptorSamplers: 16,
        maxPerStageDescriptorUniformBuffers: 12,
        maxPerStageDescriptorStorageBuffers: 31,
        maxPerStageDescriptorSampledImages: 16,
        maxPerStageDescriptorStorageImages: 4,
        maxPerStageDescriptorInputAttachments: 4,
        maxPerStageResources: 128,
        maxDescriptorSetSamplers: 96,
        maxDescriptorSetUniformBuffers: 72,
        maxDescriptorSetUniformBuffersDynamic: 8,
        maxDescriptorSetStorageBuffers: 24,
        maxDescriptorSetStorageBuffersDynamic: 4,
        maxDescriptorSetSampledImages: 96,
        maxDescriptorSetStorageImages: 24,
        maxDescriptorSetInputAttachments: 4,
        maxVertexInputAttributes: 16,
        maxVertexInputBindings: 16,
        maxVertexInputAttributeOffset: 2047,
        maxVertexInputBindingStride: 2048,
        maxVertexOutputComponents: 64,
        maxTessellationGenerationLevel: 0,
        maxTessellationPatchSize: 0,
        maxTessellationControlPerVertexInputComponents: 0,
        maxTessellationControlPerVertexOutputComponents: 0,
        maxTessellationControlPerPatchOutputComponents: 0,
        maxTessellationControlTotalOutputComponents: 0,
        maxTessellationEvaluationInputComponents: 0,
        maxTessellationEvaluationOutputComponents: 0,
        maxGeometryShaderInvocations: 0,
        maxGeometryInputComponents: 0,
        maxGeometryOutputComponents: 0,
        maxGeometryOutputVertices: 0,
        maxGeometryTotalOutputComponents: 0,
        maxFragmentInputComponents: 64,
        maxFragmentOutputAttachments: 4,
        maxFragmentDualSrcAttachments: 0,
        maxFragmentCombinedOutputResources: 4,
        maxComputeSharedMemorySize: 16384,
        maxComputeWorkGroupCount: [65535; 3],
        maxComputeWorkGroupInvocations: invocations,
        maxComputeWorkGroupSize: [invocations, invocations, 64],
        subPixelPrecisionBits: 8,
        subTexelPrecisionBits: 4,
        mipmapPrecisionBits: 4,
        maxDrawIndexedIndexValue: u32::MAX,
        maxDrawIndirectCount: 1,
        maxSamplerLodBias: 2.0,
        maxSamplerAnisotropy: 1.0,
        maxViewports: 1,
        maxViewportDimensions: [8192, 8192],
        viewportBoundsRange: [-16384.0, 16383.0],
        viewportSubPixelBits: 0,
        minMemoryMapAlignment: 64,
        minTexelBufferOffsetAlignment: 256,
        minUniformBufferOffsetAlignment: 256,
        minStorageBufferOffsetAlignment: 256,
        minTexelOffset: -8,
        maxTexelOffset: 7,
        minTexelGatherOffset: 0,
        maxTexelGatherOffset: 0,
        minInterpolationOffset: 0.0,
        maxInterpolationOffset: 0.0,
        subPixelInterpolationOffsetBits: 0,
        maxFramebufferWidth: 8192,
        maxFramebufferHeight: 8192,
        maxFramebufferLayers: 1,
        framebufferColorSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        framebufferDepthSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        framebufferStencilSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        framebufferNoAttachmentsSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        maxColorAttachments: 4,
        sampledImageColorSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        sampledImageIntegerSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        sampledImageDepthSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        sampledImageStencilSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        storageImageSampleCounts: VK_SAMPLE_COUNT_1_BIT,
        maxSampleMaskWords: 1,
        timestampComputeAndGraphics: VK_FALSE,
        timestampPeriod: 1.0,
        maxClipDistances: 0,
        maxCullDistances: 0,
        maxCombinedClipAndCullDistances: 0,
        discreteQueuePriorities: 2,
        pointSizeRange: [1.0, 1.0],
        lineWidthRange: [1.0, 1.0],
        pointSizeGranularity: 0.0,
        lineWidthGranularity: 0.0,
        strictLines: VK_FALSE,
        standardSampleLocations: VK_TRUE,
        optimalBufferCopyOffsetAlignment: 4,
        optimalBufferCopyRowPitchAlignment: 4,
        nonCoherentAtomSize: 64,
    }
}

/// Walks a `pNext` chain, calling `f` with each structure's type and address.
unsafe fn each_next(mut next: *mut c_void, mut f: impl FnMut(VkStructureType, *mut c_void)) {
    while !next.is_null() {
        let base = next as *mut VkBaseOutStructure;
        let (ty, after) = unsafe { ((*base).sType, (*base).pNext as *mut c_void) };
        f(ty, next);
        next = after;
    }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceProperties(p: VkPhysicalDevice, out: *mut VkPhysicalDeviceProperties) {
    unsafe { *out = properties(physical(p)) };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceProperties2(p: VkPhysicalDevice, out: *mut VkPhysicalDeviceProperties2) {
    unsafe {
        let p = physical(p);
        (*out).properties = properties(p);
        each_next((*out).pNext, |ty, at| match ty {
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES => {
                let id = &mut *(at as *mut VkPhysicalDeviceIDProperties);
                id.deviceUUID = *b"carbon-gpu-dev-1";
                id.driverUUID = *b"carbon-vk-driver";
                id.deviceLUIDValid = VK_FALSE;
            }
            VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES => {
                let d = &mut *(at as *mut VkPhysicalDeviceDriverProperties);
                // No ID of Khronos's names this driver; 0 is no known driver.
                d.driverID = 0;
                text(&mut d.driverName, b"Carbon");
                text(&mut d.driverInfo, b"lambda's Vulkan driver, lowered onto Carbon's gpu");
                d.conformanceVersion = VkConformanceVersion { major: 0, minor: 0, subminor: 0, patch: 0 };
            }
            _ => {}
        });
    }
}

/// Vulkan 1.0's one required feature; the rest come as the device does them.
fn features() -> VkPhysicalDeviceFeatures {
    // Every member is a VkBool32: all false, then those it has.
    let mut f: VkPhysicalDeviceFeatures = unsafe { core::mem::zeroed() };
    f.robustBufferAccess = VK_TRUE;
    f.independentBlend = VK_TRUE;
    f.fullDrawIndexUint32 = VK_TRUE;
    f
}

pub unsafe extern "C" fn vkGetPhysicalDeviceFeatures(_p: VkPhysicalDevice, out: *mut VkPhysicalDeviceFeatures) {
    unsafe { *out = features() };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceFeatures2(_p: VkPhysicalDevice, out: *mut VkPhysicalDeviceFeatures2) {
    unsafe { (*out).features = features() };
}

/// One queue, which does everything.
fn queue_family() -> VkQueueFamilyProperties {
    VkQueueFamilyProperties {
        queueFlags: VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT | VK_QUEUE_TRANSFER_BIT,
        queueCount: 1,
        timestampValidBits: 0,
        minImageTransferGranularity: VkExtent3D { width: 1, height: 1, depth: 1 },
    }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceQueueFamilyProperties(_p: VkPhysicalDevice, count: *mut u32, out: *mut VkQueueFamilyProperties) {
    unsafe { write_list(&[queue_family()], count, out) };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceQueueFamilyProperties2(_p: VkPhysicalDevice, count: *mut u32, out: *mut VkQueueFamilyProperties2) {
    unsafe {
        if out.is_null() {
            *count = 1;
        } else if *count >= 1 {
            (*out).queueFamilyProperties = queue_family();
            *count = 1;
        }
    }
}

/// One heap of memory both the device and the program see, coherent: Apple silicon's, and the CPU device's.
fn memory() -> VkPhysicalDeviceMemoryProperties {
    let mut m: VkPhysicalDeviceMemoryProperties = unsafe { core::mem::zeroed() };
    m.memoryTypeCount = 1;
    m.memoryTypes[0] = VkMemoryType { propertyFlags: VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT | VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT | VK_MEMORY_PROPERTY_HOST_CACHED_BIT, heapIndex: 0 };
    m.memoryHeapCount = 1;
    m.memoryHeaps[0] = VkMemoryHeap { size: 2 << 30, flags: VK_MEMORY_HEAP_DEVICE_LOCAL_BIT };
    m
}

pub unsafe extern "C" fn vkGetPhysicalDeviceMemoryProperties(_p: VkPhysicalDevice, out: *mut VkPhysicalDeviceMemoryProperties) {
    unsafe { *out = memory() };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceMemoryProperties2(_p: VkPhysicalDevice, out: *mut VkPhysicalDeviceMemoryProperties2) {
    unsafe { (*out).memoryProperties = memory() };
}

/// What the device does with each format it has: the device model's image formats, and its vertex attributes'.
pub(crate) fn format(format: VkFormat) -> VkFormatProperties {
    let color = VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT
        | VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT
        | VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT
        | VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT
        | VK_FORMAT_FEATURE_BLIT_SRC_BIT
        | VK_FORMAT_FEATURE_BLIT_DST_BIT
        | VK_FORMAT_FEATURE_TRANSFER_SRC_BIT
        | VK_FORMAT_FEATURE_TRANSFER_DST_BIT;
    let depth = VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT | VK_FORMAT_FEATURE_TRANSFER_SRC_BIT | VK_FORMAT_FEATURE_TRANSFER_DST_BIT;
    let vertex = VK_FORMAT_FEATURE_VERTEX_BUFFER_BIT;
    let (linear, optimal, buffer) = match format {
        VK_FORMAT_R8G8B8A8_UNORM | VK_FORMAT_B8G8R8A8_UNORM => (color, color, vertex),
        VK_FORMAT_R8G8B8A8_SRGB | VK_FORMAT_B8G8R8A8_SRGB => (color, color, 0),
        VK_FORMAT_R32G32B32A32_SFLOAT => (color & !VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT, color & !VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT, vertex),
        VK_FORMAT_D32_SFLOAT | VK_FORMAT_D16_UNORM => (0, depth, 0),
        VK_FORMAT_R32_SFLOAT | VK_FORMAT_R32G32_SFLOAT | VK_FORMAT_R32G32B32_SFLOAT => (0, 0, vertex),
        VK_FORMAT_R32_UINT | VK_FORMAT_R32G32_UINT | VK_FORMAT_R32G32B32_UINT | VK_FORMAT_R32G32B32A32_UINT => (0, 0, vertex),
        VK_FORMAT_R32_SINT | VK_FORMAT_R32G32_SINT | VK_FORMAT_R32G32B32_SINT | VK_FORMAT_R32G32B32A32_SINT => (0, 0, vertex),
        _ => (0, 0, 0),
    };
    VkFormatProperties { linearTilingFeatures: linear, optimalTilingFeatures: optimal, bufferFeatures: buffer }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceFormatProperties(_p: VkPhysicalDevice, f: VkFormat, out: *mut VkFormatProperties) {
    unsafe { *out = format(f) };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceFormatProperties2(_p: VkPhysicalDevice, f: VkFormat, out: *mut VkFormatProperties2) {
    unsafe { (*out).formatProperties = format(f) };
}

/// 2D images of the formats the device has, one level and layer, one sample: what the device model makes.
pub(crate) fn image_format(f: VkFormat, ty: VkImageType, tiling: VkImageTiling, usage: VkImageUsageFlags) -> Result<VkImageFormatProperties, VkResult> {
    let props = format(f);
    let features = if tiling == VK_IMAGE_TILING_LINEAR { props.linearTilingFeatures } else { props.optimalTilingFeatures };
    if features == 0 || ty != VK_IMAGE_TYPE_2D {
        return Err(VK_ERROR_FORMAT_NOT_SUPPORTED);
    }
    let needs = [
        (VK_IMAGE_USAGE_SAMPLED_BIT, VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT),
        (VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT, VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT),
        (VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT, VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT),
        (VK_IMAGE_USAGE_TRANSFER_SRC_BIT, VK_FORMAT_FEATURE_TRANSFER_SRC_BIT),
        (VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_FORMAT_FEATURE_TRANSFER_DST_BIT),
    ];
    if needs.iter().any(|(u, f)| usage & u != 0 && features & f == 0) || usage & VK_IMAGE_USAGE_STORAGE_BIT != 0 {
        return Err(VK_ERROR_FORMAT_NOT_SUPPORTED);
    }
    Ok(VkImageFormatProperties { maxExtent: VkExtent3D { width: 8192, height: 8192, depth: 1 }, maxMipLevels: 1, maxArrayLayers: 1, sampleCounts: VK_SAMPLE_COUNT_1_BIT, maxResourceSize: 1 << 31 })
}

pub unsafe extern "C" fn vkGetPhysicalDeviceImageFormatProperties(
    _p: VkPhysicalDevice,
    f: VkFormat,
    ty: VkImageType,
    tiling: VkImageTiling,
    usage: VkImageUsageFlags,
    _flags: VkImageCreateFlags,
    out: *mut VkImageFormatProperties,
) -> VkResult {
    match image_format(f, ty, tiling, usage) {
        Ok(props) => {
            unsafe { *out = props };
            VK_SUCCESS
        }
        Err(e) => e,
    }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceImageFormatProperties2(_p: VkPhysicalDevice, info: *const VkPhysicalDeviceImageFormatInfo2, out: *mut VkImageFormatProperties2) -> VkResult {
    let info = unsafe { &*info };
    match image_format(info.format, info.type_, info.tiling, info.usage) {
        Ok(props) => {
            unsafe { (*out).imageFormatProperties = props };
            VK_SUCCESS
        }
        Err(e) => e,
    }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceSparseImageFormatProperties(
    _p: VkPhysicalDevice,
    _f: VkFormat,
    _ty: VkImageType,
    _samples: VkSampleCountFlagBits,
    _usage: VkImageUsageFlags,
    _tiling: VkImageTiling,
    count: *mut u32,
    _out: *mut VkSparseImageFormatProperties,
) {
    unsafe { *count = 0 };
}

pub unsafe extern "C" fn vkGetPhysicalDeviceSparseImageFormatProperties2(_p: VkPhysicalDevice, _info: *const VkPhysicalDeviceSparseImageFormatInfo2, count: *mut u32, _out: *mut VkSparseImageFormatProperties2) {
    unsafe { *count = 0 };
}

pub unsafe extern "C" fn vkEnumerateDeviceExtensionProperties(_p: VkPhysicalDevice, layer: *const c_char, count: *mut u32, out: *mut VkExtensionProperties) -> VkResult {
    if !layer.is_null() {
        return VK_ERROR_LAYER_NOT_PRESENT;
    }
    let mut list = [VkExtensionProperties { extensionName: [0; VK_MAX_EXTENSION_NAME_SIZE as usize], specVersion: 0 }; DEVICE_EXTENSIONS.len()];
    for (slot, (name, version)) in list.iter_mut().zip(DEVICE_EXTENSIONS) {
        text(&mut slot.extensionName, name);
        slot.specVersion = version;
    }
    unsafe { write_list(&list, count, out) }
}

pub unsafe extern "C" fn vkEnumerateDeviceLayerProperties(_p: VkPhysicalDevice, count: *mut u32, out: *mut VkLayerProperties) -> VkResult {
    unsafe { write_list::<VkLayerProperties>(&[], count, out) }
}
