//! The device: its memory, buffers and images, shader modules, descriptor sets, compute pipelines, command buffers,
//! queue and fences.
//!
//! What the host needs it's told as records (`gpu_wire::records`), collected under the device's lock and sent through
//! the render node's doorbell when the driver has to wait for the host: a pipeline's compile and a queue submission.
//! The rest stays here: a buffer is its memory and offset, a descriptor set the buffers written to it, and a command
//! buffer the records its commands make, with descriptor sets resolved to buffers as each dispatch is recorded (Vulkan
//! doesn't let a set bound in a command buffer change before it runs). A submission runs to its end before
//! `vkQueueSubmit` returns, so fences are signaled then, and waits never wait.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use core::ptr;

use gpu_wire::records::{self, Bound, BoundImage};
use gpu_wire::vk::*;

use crate::sync::Spin;
use crate::{ICD_LOADER_MAGIC, Physical, image_format, ioctl, mmap, munmap, write};

const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;
/// The memory every image and buffer can be in: the device's one type.
const MEMORY_TYPES: u32 = 1;
/// The alignment of an image's or buffer's memory: enough for any texel, uniform or storage offset.
const ALIGNMENT: u64 = 256;
/// Vulkan's most push-constant bytes this driver takes (`maxPushConstantsSize` is 128).
const PUSH_BYTES: usize = 256;
/// The descriptor sets a pipeline layout can have (`maxBoundDescriptorSets`).
const SETS: usize = 4;

/// Dispatchable: the loader's slot first.
#[repr(C)]
pub(crate) struct Device {
    #[allow(dead_code)]
    loader: usize,
    queue: *mut Queue,
    fd: c_int,
    /// Records for the host not sent yet.
    pending: Spin<Vec<u8>>,
}

/// Dispatchable: the loader's slot first.
#[repr(C)]
pub(crate) struct Queue {
    #[allow(dead_code)]
    loader: usize,
    pub(crate) device: *mut Device,
}

pub(crate) unsafe fn device<'a>(d: VkDevice) -> &'a Device {
    unsafe { &*(d as *const Device) }
}

/// Says what the host found wrong, on the program's stderr, as Mesa's drivers do.
fn complain(message: &[u8]) {
    let prefix = b"carbon vulkan: ";
    unsafe {
        write(2, prefix.as_ptr() as *const c_void, prefix.len());
        write(2, message.as_ptr() as *const c_void, message.len());
        write(2, b"\n".as_ptr() as *const c_void, 1);
    }
}

impl Device {
    /// Adds a record to those to send.
    pub(crate) fn record(&self, kind: u32, payload: &[u8]) {
        records::record(&mut *self.pending.lock(), kind, payload);
    }

    /// Sends what's pending, and `then`, and waits for the host to have done it all.
    pub(crate) fn ring(&self, kind: u32, payload: &[u8]) -> Result<(), ()> {
        let mut out = core::mem::take(&mut *self.pending.lock());
        records::record(&mut out, kind, payload);
        let mut reply = [0u8; 512];
        let words: [u64; 4] = [out.as_ptr() as u64, out.len() as u64, reply.as_mut_ptr() as u64, reply.len() as u64];
        let answer = unsafe { ioctl(self.fd, gpu_wire::SUBMIT_IOCTL as c_ulong, words.as_ptr()) };
        match answer {
            0 => Ok(()),
            n if n > 0 => {
                complain(&reply[..(n as usize).min(reply.len())]);
                Err(())
            }
            _ => {
                complain(b"the render node's doorbell failed");
                Err(())
            }
        }
    }
}

// The device.

pub unsafe extern "C" fn vkCreateDevice(p: VkPhysicalDevice, info: *const VkDeviceCreateInfo, _allocator: *const VkAllocationCallbacks, out: *mut VkDevice) -> VkResult {
    let info = unsafe { &*info };
    for i in 0..info.enabledExtensionCount as usize {
        let name = unsafe { CStr::from_ptr(*info.ppEnabledExtensionNames.add(i)) }.to_bytes();
        if !crate::DEVICE_EXTENSIONS.iter().any(|(n, _)| *n == name) {
            return VK_ERROR_EXTENSION_NOT_PRESENT;
        }
    }
    let fd = unsafe { &*(p as *const Physical) }.fd;
    let device = Box::into_raw(Box::new(Device { loader: ICD_LOADER_MAGIC, queue: ptr::null_mut(), fd, pending: Spin::new(Vec::new()) }));
    let queue = Box::into_raw(Box::new(Queue { loader: ICD_LOADER_MAGIC, device }));
    unsafe {
        (*device).queue = queue;
        *out = device as VkDevice;
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyDevice(device: VkDevice, _allocator: *const VkAllocationCallbacks) {
    if device.is_null() {
        return;
    }
    let device = unsafe { Box::from_raw(device as *mut Device) };
    // What's pending only frees things; the render node's close frees everything.
    drop(unsafe { Box::from_raw(device.queue) });
}

pub unsafe extern "C" fn vkGetDeviceProcAddr(_device: VkDevice, name: *const c_char) -> PFN_vkVoidFunction {
    if name.is_null() {
        return None;
    }
    lookup(unsafe { CStr::from_ptr(name) }.to_bytes()).map(|f| unsafe { core::mem::transmute::<*const (), unsafe extern "C" fn()>(f) })
}

pub unsafe extern "C" fn vkGetDeviceQueue(d: VkDevice, family: u32, index: u32, out: *mut VkQueue) {
    let device = unsafe { device(d) };
    unsafe { *out = if family == 0 && index == 0 { device.queue as VkQueue } else { ptr::null_mut() } };
}

pub unsafe extern "C" fn vkDeviceWaitIdle(_device: VkDevice) -> VkResult {
    VK_SUCCESS
}

// Memory: the guest's copy, mapped by the program, which the host reads before work and writes after.

struct Memory {
    size: u64,
    at: *mut u8,
}

pub unsafe extern "C" fn vkAllocateMemory(d: VkDevice, info: *const VkMemoryAllocateInfo, _allocator: *const VkAllocationCallbacks, out: *mut VkDeviceMemory) -> VkResult {
    let info = unsafe { &*info };
    if info.memoryTypeIndex != 0 || info.allocationSize == 0 {
        return VK_ERROR_OUT_OF_DEVICE_MEMORY;
    }
    let at = unsafe { mmap(ptr::null_mut(), info.allocationSize as usize, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
    if at as isize == -1 {
        return VK_ERROR_OUT_OF_HOST_MEMORY;
    }
    let memory = Box::into_raw(Box::new(Memory { size: info.allocationSize, at: at as *mut u8 }));
    let mut p = Vec::new();
    records::put_u64(&mut p, memory as u64);
    records::put_u64(&mut p, info.allocationSize);
    records::put_u64(&mut p, at as u64);
    unsafe { device(d) }.record(records::MEMORY, &p);
    unsafe { *out = memory as VkDeviceMemory };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkFreeMemory(d: VkDevice, memory: VkDeviceMemory, _allocator: *const VkAllocationCallbacks) {
    if memory == 0 {
        return;
    }
    let mut p = Vec::new();
    records::put_u64(&mut p, memory);
    unsafe { device(d) }.record(records::FREE_MEMORY, &p);
    let m = unsafe { Box::from_raw(memory as *mut Memory) };
    unsafe { munmap(m.at as *mut c_void, m.size as usize) };
}

pub unsafe extern "C" fn vkMapMemory(_d: VkDevice, memory: VkDeviceMemory, offset: VkDeviceSize, _size: VkDeviceSize, _flags: VkMemoryMapFlags, out: *mut *mut c_void) -> VkResult {
    let m = unsafe { &*(memory as *const Memory) };
    unsafe { *out = m.at.add(offset as usize) as *mut c_void };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkUnmapMemory(_d: VkDevice, _memory: VkDeviceMemory) {}

/// The one memory type is coherent: nothing to flush or invalidate.
pub unsafe extern "C" fn vkFlushMappedMemoryRanges(_d: VkDevice, _count: u32, _ranges: *const VkMappedMemoryRange) -> VkResult {
    VK_SUCCESS
}

pub unsafe extern "C" fn vkInvalidateMappedMemoryRanges(_d: VkDevice, _count: u32, _ranges: *const VkMappedMemoryRange) -> VkResult {
    VK_SUCCESS
}

pub unsafe extern "C" fn vkGetDeviceMemoryCommitment(_d: VkDevice, memory: VkDeviceMemory, out: *mut VkDeviceSize) {
    unsafe { *out = (*(memory as *const Memory)).size };
}

// Buffers and images.

struct Buffer {
    size: u64,
    /// The memory it's bound to (its handle, which the host knows it by) and where in it.
    memory: u64,
    offset: u64,
}

pub(crate) struct Image {
    size: u64,
    width: u32,
    height: u32,
    texel: u64,
    format: VkFormat,
    linear: bool,
    /// Whether the host has it: once it's bound to memory (or made for a swapchain).
    made: bool,
}

impl Image {
    /// Tells the host of it, in `memory` (0 for none) at `offset`.
    pub(crate) fn make(&mut self, device: &Device, id: u64, memory: u64, offset: u64) {
        let mut p = Vec::new();
        records::put_u64(&mut p, id);
        records::put_u64(&mut p, memory);
        records::put_u64(&mut p, offset);
        for v in [self.format as u32, self.width, self.height, self.linear as u32] {
            records::put_u32(&mut p, v);
        }
        device.record(records::IMAGE, &p);
        self.made = true;
    }

    /// An optimal 2D image with nothing behind it on the guest, as a swapchain's are.
    pub(crate) fn swapchain(format: VkFormat, width: u32, height: u32) -> Image {
        Image { size: 0, width, height, texel: 4, format, linear: false, made: false }
    }
}

unsafe fn buffer<'a>(b: VkBuffer) -> &'a Buffer {
    unsafe { &*(b as *const Buffer) }
}

/// A texel's bytes, for the formats the device has.
fn texel_size(format: VkFormat) -> Option<u64> {
    Some(match format {
        VK_FORMAT_R8G8B8A8_UNORM | VK_FORMAT_R8G8B8A8_SRGB | VK_FORMAT_B8G8R8A8_UNORM | VK_FORMAT_B8G8R8A8_SRGB | VK_FORMAT_D32_SFLOAT => 4,
        VK_FORMAT_D16_UNORM => 2,
        VK_FORMAT_R32G32B32A32_SFLOAT => 16,
        _ => return None,
    })
}

pub unsafe extern "C" fn vkCreateImage(_device: VkDevice, info: *const VkImageCreateInfo, _allocator: *const VkAllocationCallbacks, out: *mut VkImage) -> VkResult {
    let info = unsafe { &*info };
    let Some(texel) = texel_size(info.format) else { return VK_ERROR_FORMAT_NOT_SUPPORTED };
    if image_format(info.format, info.imageType, info.tiling, info.usage).is_err() || info.mipLevels != 1 || info.arrayLayers != 1 {
        return VK_ERROR_FORMAT_NOT_SUPPORTED;
    }
    let e = info.extent;
    let size = (u64::from(e.width) * u64::from(e.height) * u64::from(e.depth.max(1)) * texel).next_multiple_of(ALIGNMENT);
    let image = Image { size, width: e.width, height: e.height, texel, format: info.format, linear: info.tiling == VK_IMAGE_TILING_LINEAR, made: false };
    unsafe { *out = Box::into_raw(Box::new(image)) as VkImage };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyImage(d: VkDevice, image: VkImage, _allocator: *const VkAllocationCallbacks) {
    if image != 0 {
        let i = unsafe { Box::from_raw(image as *mut Image) };
        if i.made {
            free(d, records::FREE_IMAGE, image);
        }
    }
}

/// Tells the host an object it has is gone.
fn free(d: VkDevice, kind: u32, id: u64) {
    let mut p = Vec::new();
    records::put_u64(&mut p, id);
    unsafe { device(d) }.record(kind, &p);
}

pub unsafe extern "C" fn vkGetImageMemoryRequirements(_device: VkDevice, image: VkImage, out: *mut VkMemoryRequirements) {
    let image = unsafe { &*(image as *const Image) };
    unsafe { *out = VkMemoryRequirements { size: image.size, alignment: ALIGNMENT, memoryTypeBits: MEMORY_TYPES } };
}

/// The host makes the image once it's bound: a linear one's texels are its memory's, an optimal one's the host's.
pub unsafe extern "C" fn vkBindImageMemory(d: VkDevice, image: VkImage, memory: VkDeviceMemory, offset: VkDeviceSize) -> VkResult {
    unsafe { &mut *(image as *mut Image) }.make(unsafe { device(d) }, image, memory, offset);
    VK_SUCCESS
}

/// A linear image's one level: its rows, tightly packed.
pub unsafe extern "C" fn vkGetImageSubresourceLayout(_device: VkDevice, image: VkImage, _sub: *const VkImageSubresource, out: *mut VkSubresourceLayout) {
    let image = unsafe { &*(image as *const Image) };
    let row = u64::from(image.width) * image.texel;
    unsafe { *out = VkSubresourceLayout { offset: 0, size: image.size, rowPitch: row, arrayPitch: image.size, depthPitch: image.size } };
}

pub unsafe extern "C" fn vkGetImageSparseMemoryRequirements(_device: VkDevice, _image: VkImage, count: *mut u32, _out: *mut VkSparseImageMemoryRequirements) {
    unsafe { *count = 0 };
}

pub unsafe extern "C" fn vkCreateBuffer(_device: VkDevice, info: *const VkBufferCreateInfo, _allocator: *const VkAllocationCallbacks, out: *mut VkBuffer) -> VkResult {
    let info = unsafe { &*info };
    unsafe { *out = Box::into_raw(Box::new(Buffer { size: info.size, memory: 0, offset: 0 })) as VkBuffer };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyBuffer(_device: VkDevice, buffer: VkBuffer, _allocator: *const VkAllocationCallbacks) {
    if buffer != 0 {
        drop(unsafe { Box::from_raw(buffer as *mut Buffer) });
    }
}

pub unsafe extern "C" fn vkGetBufferMemoryRequirements(_device: VkDevice, b: VkBuffer, out: *mut VkMemoryRequirements) {
    let b = unsafe { buffer(b) };
    unsafe { *out = VkMemoryRequirements { size: b.size.next_multiple_of(ALIGNMENT), alignment: ALIGNMENT, memoryTypeBits: MEMORY_TYPES } };
}

pub unsafe extern "C" fn vkBindBufferMemory(_device: VkDevice, b: VkBuffer, memory: VkDeviceMemory, offset: VkDeviceSize) -> VkResult {
    let b = unsafe { &mut *(b as *mut Buffer) };
    b.memory = memory;
    b.offset = offset;
    VK_SUCCESS
}

/// A buffer's range, as (memory, offset in it, size), with `VK_WHOLE_SIZE` the rest of the buffer.
fn span(b: &Buffer, offset: u64, size: u64) -> (u64, u64, u64) {
    let size = if size == VK_WHOLE_SIZE { b.size.saturating_sub(offset) } else { size };
    (b.memory, b.offset + offset, size)
}

// Shader modules, layouts and pipeline caches: kept here; only a pipeline goes to the host.

struct ShaderModule {
    code: Vec<u8>,
}

/// A handle with nothing behind it that matters: a layout, a cache, a semaphore.
fn token() -> u64 {
    Box::into_raw(Box::new(0u8)) as u64
}

fn drop_token(handle: u64) {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut u8) });
    }
}

pub unsafe extern "C" fn vkCreateShaderModule(_d: VkDevice, info: *const VkShaderModuleCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkShaderModule) -> VkResult {
    let info = unsafe { &*info };
    let code = unsafe { core::slice::from_raw_parts(info.pCode as *const u8, info.codeSize) }.to_vec();
    unsafe { *out = Box::into_raw(Box::new(ShaderModule { code })) as VkShaderModule };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyShaderModule(_d: VkDevice, module: VkShaderModule, _a: *const VkAllocationCallbacks) {
    if module != 0 {
        drop(unsafe { Box::from_raw(module as *mut ShaderModule) });
    }
}

pub unsafe extern "C" fn vkCreatePipelineLayout(_d: VkDevice, _info: *const VkPipelineLayoutCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkPipelineLayout) -> VkResult {
    unsafe { *out = token() };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyPipelineLayout(_d: VkDevice, layout: VkPipelineLayout, _a: *const VkAllocationCallbacks) {
    drop_token(layout);
}

pub unsafe extern "C" fn vkCreatePipelineCache(_d: VkDevice, _info: *const VkPipelineCacheCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkPipelineCache) -> VkResult {
    unsafe { *out = token() };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyPipelineCache(_d: VkDevice, cache: VkPipelineCache, _a: *const VkAllocationCallbacks) {
    drop_token(cache);
}

/// The cache keeps nothing yet: its data is empty.
pub unsafe extern "C" fn vkGetPipelineCacheData(_d: VkDevice, _cache: VkPipelineCache, size: *mut usize, _data: *mut c_void) -> VkResult {
    unsafe { *size = 0 };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkMergePipelineCaches(_d: VkDevice, _dst: VkPipelineCache, _count: u32, _src: *const VkPipelineCache) -> VkResult {
    VK_SUCCESS
}

pub unsafe extern "C" fn vkCreateComputePipelines(
    d: VkDevice,
    _cache: VkPipelineCache,
    count: u32,
    infos: *const VkComputePipelineCreateInfo,
    _a: *const VkAllocationCallbacks,
    out: *mut VkPipeline,
) -> VkResult {
    let device = unsafe { device(d) };
    let mut result = VK_SUCCESS;
    for i in 0..count as usize {
        let info = unsafe { &*infos.add(i) };
        let module = unsafe { &*(info.stage.module as *const ShaderModule) };
        let name = unsafe { CStr::from_ptr(info.stage.pName) }.to_bytes();
        let id = Box::into_raw(Box::new(Pipeline::default())) as u64;
        let mut p = Vec::new();
        records::put_u64(&mut p, id);
        records::put_bytes(&mut p, name);
        records::put_bytes(&mut p, &module.code);
        // Compiled now, so a shader the host can't take fails here, as Vulkan says it should.
        if device.ring(records::PIPELINE, &p).is_err() {
            drop(unsafe { Box::from_raw(id as *mut Pipeline) });
            unsafe { *out.add(i) = 0 };
            result = VK_ERROR_INITIALIZATION_FAILED;
            continue;
        }
        unsafe { *out.add(i) = id };
    }
    result
}

pub unsafe extern "C" fn vkDestroyPipeline(d: VkDevice, pipeline: VkPipeline, _a: *const VkAllocationCallbacks) {
    if pipeline == 0 {
        return;
    }
    free(d, records::FREE_PIPELINE, pipeline);
    drop(unsafe { Box::from_raw(pipeline as *mut Pipeline) });
}

/// What the driver keeps of a pipeline: for a graphics one, which vertex binding each of its vertex buffers is, and
/// its viewport and scissor if they aren't dynamic.
#[derive(Default)]
struct Pipeline {
    vertex_bindings: Vec<u32>,
    viewport: Option<VkViewport>,
    scissor: Option<VkRect2D>,
}

// Descriptor sets: each binding's buffer or image, written here and read as a dispatch or draw is recorded.

struct SetLayout {
    /// Each binding, its descriptor type and count, and its immutable samplers.
    bindings: Vec<(u32, VkDescriptorType, u32, Vec<u64>)>,
}

/// One descriptor: the binding and element it's at, whether it takes a dynamic offset, and a buffer's range or an
/// image and sampler.
#[derive(Clone, Copy)]
struct Descriptor {
    binding: u32,
    element: u32,
    dynamic: bool,
    image_kind: bool,
    memory: u64,
    offset: u64,
    size: u64,
    image: u64,
    sampler: u64,
}

struct DescriptorSet {
    /// In binding and element order, which is the order dynamic offsets are taken in.
    descriptors: Vec<Descriptor>,
}

struct DescriptorPool {
    sets: Vec<*mut DescriptorSet>,
}

fn is_buffer(ty: VkDescriptorType) -> bool {
    matches!(ty, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER | VK_DESCRIPTOR_TYPE_STORAGE_BUFFER | VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC | VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC)
}

fn is_image(ty: VkDescriptorType) -> bool {
    matches!(ty, VK_DESCRIPTOR_TYPE_SAMPLER | VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER | VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE)
}

pub unsafe extern "C" fn vkCreateDescriptorSetLayout(_d: VkDevice, info: *const VkDescriptorSetLayoutCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkDescriptorSetLayout) -> VkResult {
    let info = unsafe { &*info };
    let mut bindings: Vec<(u32, VkDescriptorType, u32, Vec<u64>)> = (0..info.bindingCount as usize)
        .map(|i| {
            let b = unsafe { &*info.pBindings.add(i) };
            let immutable = if b.pImmutableSamplers.is_null() || !is_image(b.descriptorType) {
                Vec::new()
            } else {
                unsafe { core::slice::from_raw_parts(b.pImmutableSamplers, b.descriptorCount as usize) }.to_vec()
            };
            (b.binding, b.descriptorType, b.descriptorCount, immutable)
        })
        .collect();
    bindings.sort_by_key(|b| b.0);
    unsafe { *out = Box::into_raw(Box::new(SetLayout { bindings })) as VkDescriptorSetLayout };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyDescriptorSetLayout(_d: VkDevice, layout: VkDescriptorSetLayout, _a: *const VkAllocationCallbacks) {
    if layout != 0 {
        drop(unsafe { Box::from_raw(layout as *mut SetLayout) });
    }
}

pub unsafe extern "C" fn vkCreateDescriptorPool(_d: VkDevice, _info: *const VkDescriptorPoolCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkDescriptorPool) -> VkResult {
    unsafe { *out = Box::into_raw(Box::new(DescriptorPool { sets: Vec::new() })) as VkDescriptorPool };
    VK_SUCCESS
}

fn free_sets(pool: &mut DescriptorPool) {
    for set in pool.sets.drain(..) {
        drop(unsafe { Box::from_raw(set) });
    }
}

pub unsafe extern "C" fn vkDestroyDescriptorPool(_d: VkDevice, pool: VkDescriptorPool, _a: *const VkAllocationCallbacks) {
    if pool != 0 {
        let mut pool = unsafe { Box::from_raw(pool as *mut DescriptorPool) };
        free_sets(&mut pool);
    }
}

pub unsafe extern "C" fn vkResetDescriptorPool(_d: VkDevice, pool: VkDescriptorPool, _flags: VkDescriptorPoolResetFlags) -> VkResult {
    free_sets(unsafe { &mut *(pool as *mut DescriptorPool) });
    VK_SUCCESS
}

pub unsafe extern "C" fn vkAllocateDescriptorSets(_d: VkDevice, info: *const VkDescriptorSetAllocateInfo, out: *mut VkDescriptorSet) -> VkResult {
    let info = unsafe { &*info };
    let pool = unsafe { &mut *(info.descriptorPool as *mut DescriptorPool) };
    for i in 0..info.descriptorSetCount as usize {
        let layout = unsafe { &*(*info.pSetLayouts.add(i) as *const SetLayout) };
        let mut descriptors = Vec::new();
        for (binding, ty, count, immutable) in layout.bindings.iter().filter(|b| is_buffer(b.1) || is_image(b.1)) {
            let dynamic = matches!(*ty, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC | VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC);
            descriptors.extend((0..*count).map(|element| Descriptor {
                binding: *binding,
                element,
                dynamic,
                image_kind: is_image(*ty),
                memory: 0,
                offset: 0,
                size: 0,
                image: 0,
                sampler: immutable.get(element as usize).copied().unwrap_or(0),
            }));
        }
        let set = Box::into_raw(Box::new(DescriptorSet { descriptors }));
        pool.sets.push(set);
        unsafe { *out.add(i) = set as VkDescriptorSet };
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkFreeDescriptorSets(_d: VkDevice, pool: VkDescriptorPool, count: u32, sets: *const VkDescriptorSet) -> VkResult {
    let pool = unsafe { &mut *(pool as *mut DescriptorPool) };
    for i in 0..count as usize {
        let set = unsafe { *sets.add(i) } as *mut DescriptorSet;
        if let Some(at) = pool.sets.iter().position(|s| *s == set) {
            pool.sets.swap_remove(at);
            drop(unsafe { Box::from_raw(set) });
        }
    }
    VK_SUCCESS
}

/// The descriptor at (binding, element), going on to the next binding as Vulkan's updates do past a binding's end.
fn descriptor(set: &mut DescriptorSet, binding: u32, element: u32, n: u32) -> Option<&mut Descriptor> {
    let start = set.descriptors.iter().position(|d| d.binding == binding && d.element == element)?;
    set.descriptors.get_mut(start + n as usize)
}

pub unsafe extern "C" fn vkUpdateDescriptorSets(_d: VkDevice, write_count: u32, writes: *const VkWriteDescriptorSet, copy_count: u32, copies: *const VkCopyDescriptorSet) {
    for i in 0..write_count as usize {
        let w = unsafe { &*writes.add(i) };
        let set = unsafe { &mut *(w.dstSet as *mut DescriptorSet) };
        if is_image(w.descriptorType) {
            for n in 0..w.descriptorCount {
                let info = unsafe { &*w.pImageInfo.add(n as usize) };
                let image = if info.imageView == 0 || w.descriptorType == VK_DESCRIPTOR_TYPE_SAMPLER { 0 } else { unsafe { (*(info.imageView as *const ImageView)).image } };
                if let Some(d) = descriptor(set, w.dstBinding, w.dstArrayElement, n) {
                    d.image = image;
                    // An immutable sampler stays.
                    if w.descriptorType != VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE && info.sampler != 0 {
                        d.sampler = info.sampler;
                    }
                }
            }
            continue;
        }
        if !is_buffer(w.descriptorType) {
            continue;
        }
        for n in 0..w.descriptorCount {
            let info = unsafe { &*w.pBufferInfo.add(n as usize) };
            let (memory, offset, size) = span(unsafe { buffer(info.buffer) }, info.offset, info.range);
            if let Some(d) = descriptor(set, w.dstBinding, w.dstArrayElement, n) {
                (d.memory, d.offset, d.size) = (memory, offset, size);
            }
        }
    }
    for i in 0..copy_count as usize {
        let c = unsafe { &*copies.add(i) };
        for n in 0..c.descriptorCount {
            let from = descriptor(unsafe { &mut *(c.srcSet as *mut DescriptorSet) }, c.srcBinding, c.srcArrayElement, n).map(|d| (d.memory, d.offset, d.size, d.image, d.sampler));
            if let (Some(from), Some(to)) = (from, descriptor(unsafe { &mut *(c.dstSet as *mut DescriptorSet) }, c.dstBinding, c.dstArrayElement, n)) {
                (to.memory, to.offset, to.size, to.image, to.sampler) = from;
            }
        }
    }
}

// Command buffers: the records their commands make, sent with each submission of them.

struct CommandPool {
    buffers: Vec<*mut CommandBuffer>,
}

/// Dispatchable: the loader's slot first.
#[repr(C)]
struct CommandBuffer {
    #[allow(dead_code)]
    loader: usize,
    records: Vec<u8>,
    /// What's bound for compute and for graphics: the pipeline and each set with its dynamic offsets; the push
    /// constants, which both share; and the vertex buffers (memory, offset, size, by binding) and index buffer.
    pipeline: u64,
    sets: [(*const DescriptorSet, Vec<u32>); SETS],
    graphics: u64,
    graphics_sets: [(*const DescriptorSet, Vec<u32>); SETS],
    push: [u8; PUSH_BYTES],
    push_len: usize,
    vertex: [(u64, u64, u64); VERTEX_BINDINGS],
    index: (u64, u64, u64, VkIndexType),
}

/// The vertex buffer bindings a pipeline can have (`maxVertexInputBindings`).
const VERTEX_BINDINGS: usize = 16;

unsafe fn command_buffer<'a>(c: VkCommandBuffer) -> &'a mut CommandBuffer {
    unsafe { &mut *(c as *mut CommandBuffer) }
}

impl CommandBuffer {
    fn new() -> CommandBuffer {
        CommandBuffer {
            loader: ICD_LOADER_MAGIC,
            records: Vec::new(),
            pipeline: 0,
            sets: Default::default(),
            graphics: 0,
            graphics_sets: Default::default(),
            push: [0; PUSH_BYTES],
            push_len: 0,
            vertex: [(0, 0, 0); VERTEX_BINDINGS],
            index: (0, 0, 0, 0),
        }
    }

    fn reset(&mut self) {
        self.records.clear();
        self.pipeline = 0;
        self.sets = Default::default();
        self.graphics = 0;
        self.graphics_sets = Default::default();
        self.push_len = 0;
        self.vertex = [(0, 0, 0); VERTEX_BINDINGS];
        self.index = (0, 0, 0, 0);
    }
}

pub unsafe extern "C" fn vkCreateCommandPool(_d: VkDevice, _info: *const VkCommandPoolCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkCommandPool) -> VkResult {
    unsafe { *out = Box::into_raw(Box::new(CommandPool { buffers: Vec::new() })) as VkCommandPool };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyCommandPool(_d: VkDevice, pool: VkCommandPool, _a: *const VkAllocationCallbacks) {
    if pool != 0 {
        let pool = unsafe { Box::from_raw(pool as *mut CommandPool) };
        for b in pool.buffers {
            drop(unsafe { Box::from_raw(b) });
        }
    }
}

pub unsafe extern "C" fn vkResetCommandPool(_d: VkDevice, pool: VkCommandPool, _flags: VkCommandPoolResetFlags) -> VkResult {
    let pool = unsafe { &mut *(pool as *mut CommandPool) };
    for b in &pool.buffers {
        unsafe { (**b).reset() };
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkAllocateCommandBuffers(_d: VkDevice, info: *const VkCommandBufferAllocateInfo, out: *mut VkCommandBuffer) -> VkResult {
    let info = unsafe { &*info };
    let pool = unsafe { &mut *(info.commandPool as *mut CommandPool) };
    for i in 0..info.commandBufferCount as usize {
        let b = Box::into_raw(Box::new(CommandBuffer::new()));
        pool.buffers.push(b);
        unsafe { *out.add(i) = b as VkCommandBuffer };
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkFreeCommandBuffers(_d: VkDevice, pool: VkCommandPool, count: u32, buffers: *const VkCommandBuffer) {
    let pool = unsafe { &mut *(pool as *mut CommandPool) };
    for i in 0..count as usize {
        let b = unsafe { *buffers.add(i) } as *mut CommandBuffer;
        if let Some(at) = pool.buffers.iter().position(|x| *x == b) {
            pool.buffers.swap_remove(at);
            drop(unsafe { Box::from_raw(b) });
        }
    }
}

pub unsafe extern "C" fn vkBeginCommandBuffer(c: VkCommandBuffer, _info: *const VkCommandBufferBeginInfo) -> VkResult {
    unsafe { command_buffer(c) }.reset();
    VK_SUCCESS
}

pub unsafe extern "C" fn vkEndCommandBuffer(_c: VkCommandBuffer) -> VkResult {
    VK_SUCCESS
}

pub unsafe extern "C" fn vkResetCommandBuffer(c: VkCommandBuffer, _flags: VkCommandBufferResetFlags) -> VkResult {
    unsafe { command_buffer(c) }.reset();
    VK_SUCCESS
}

pub unsafe extern "C" fn vkCmdBindPipeline(c: VkCommandBuffer, point: VkPipelineBindPoint, pipeline: VkPipeline) {
    let c = unsafe { command_buffer(c) };
    if point == VK_PIPELINE_BIND_POINT_COMPUTE {
        c.pipeline = pipeline;
        return;
    }
    c.graphics = pipeline;
    // A pipeline's own viewport and scissor, where they aren't dynamic, are set as it's bound.
    let p = unsafe { &*(pipeline as *const Pipeline) };
    if let Some(v) = p.viewport {
        viewport(c, &v);
    }
    if let Some(r) = p.scissor {
        scissor(c, &r);
    }
}

pub unsafe extern "C" fn vkCmdBindDescriptorSets(
    c: VkCommandBuffer,
    point: VkPipelineBindPoint,
    _layout: VkPipelineLayout,
    first: u32,
    count: u32,
    sets: *const VkDescriptorSet,
    dynamic_count: u32,
    dynamic: *const u32,
) {
    let c = unsafe { command_buffer(c) };
    let compute = point == VK_PIPELINE_BIND_POINT_COMPUTE;
    let offsets = if dynamic_count == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(dynamic, dynamic_count as usize) } };
    let mut taken = 0;
    for i in 0..count as usize {
        let set = unsafe { *sets.add(i) } as *const DescriptorSet;
        let wants = unsafe { &*set }.descriptors.iter().filter(|d| d.dynamic).count();
        let mine = offsets.get(taken..taken + wants).unwrap_or(&[]).to_vec();
        taken += wants;
        let sets = if compute { &mut c.sets } else { &mut c.graphics_sets };
        if let Some(slot) = sets.get_mut(first as usize + i) {
            *slot = (set, mine);
        }
    }
}

pub unsafe extern "C" fn vkCmdPushConstants(c: VkCommandBuffer, _layout: VkPipelineLayout, _stages: VkShaderStageFlags, offset: u32, size: u32, values: *const c_void) {
    let c = unsafe { command_buffer(c) };
    let (offset, size) = (offset as usize, size as usize);
    if offset + size > PUSH_BYTES {
        return;
    }
    let bytes = unsafe { core::slice::from_raw_parts(values as *const u8, size) };
    c.push[offset..offset + size].copy_from_slice(bytes);
    c.push_len = c.push_len.max(offset + size);
}

/// Every buffer and image the bound sets have, with dynamic offsets applied; the host picks those the pipeline uses.
fn bound(sets: &[(*const DescriptorSet, Vec<u32>); SETS]) -> (Vec<Bound>, Vec<BoundImage>) {
    let (mut buffers, mut images) = (Vec::new(), Vec::new());
    for (index, (set, offsets)) in sets.iter().enumerate() {
        if set.is_null() {
            continue;
        }
        let mut dynamic = offsets.iter();
        for d in &unsafe { &**set }.descriptors {
            let extra = if d.dynamic { u64::from(*dynamic.next().unwrap_or(&0)) } else { 0 };
            if d.element != 0 {
                continue;
            }
            if d.image_kind && (d.image != 0 || d.sampler != 0) {
                images.push(BoundImage { set: index as u32, binding: d.binding, image: d.image, sampler: d.sampler });
            } else if d.memory != 0 {
                buffers.push(Bound { set: index as u32, binding: d.binding, memory: d.memory, offset: d.offset + extra, size: d.size });
            }
        }
    }
    (buffers, images)
}

pub unsafe extern "C" fn vkCmdDispatch(c: VkCommandBuffer, x: u32, y: u32, z: u32) {
    let c = unsafe { command_buffer(c) };
    let (bound, _) = bound(&c.sets);
    let mut p = Vec::new();
    records::put_u64(&mut p, c.pipeline);
    for v in [x, y, z, bound.len() as u32, c.push_len as u32] {
        records::put_u32(&mut p, v);
    }
    for b in &bound {
        records::put_bound(&mut p, b);
    }
    records::put_bytes(&mut p, &c.push[..c.push_len]);
    records::record(&mut c.records, records::DISPATCH, &p);
}

pub unsafe extern "C" fn vkCmdCopyBuffer(c: VkCommandBuffer, src: VkBuffer, dst: VkBuffer, count: u32, regions: *const VkBufferCopy) {
    let c = unsafe { command_buffer(c) };
    let (src, dst) = unsafe { (buffer(src), buffer(dst)) };
    for i in 0..count as usize {
        let r = unsafe { &*regions.add(i) };
        let mut p = Vec::new();
        for v in [src.memory, src.offset + r.srcOffset, dst.memory, dst.offset + r.dstOffset, r.size] {
            records::put_u64(&mut p, v);
        }
        records::record(&mut c.records, records::COPY, &p);
    }
}

pub unsafe extern "C" fn vkCmdFillBuffer(c: VkCommandBuffer, dst: VkBuffer, offset: VkDeviceSize, size: VkDeviceSize, data: u32) {
    let c = unsafe { command_buffer(c) };
    let (memory, offset, size) = span(unsafe { buffer(dst) }, offset, size);
    let mut p = Vec::new();
    for v in [memory, offset, size & !3] {
        records::put_u64(&mut p, v);
    }
    records::put_u32(&mut p, data);
    records::record(&mut c.records, records::FILL, &p);
}

pub unsafe extern "C" fn vkCmdUpdateBuffer(c: VkCommandBuffer, dst: VkBuffer, offset: VkDeviceSize, size: VkDeviceSize, data: *const c_void) {
    let c = unsafe { command_buffer(c) };
    let (memory, offset, _) = span(unsafe { buffer(dst) }, offset, size);
    let mut p = Vec::new();
    records::put_u64(&mut p, memory);
    records::put_u64(&mut p, offset);
    records::put_bytes(&mut p, unsafe { core::slice::from_raw_parts(data as *const u8, size as usize) });
    records::record(&mut c.records, records::UPDATE, &p);
}

/// The device runs commands in order, each seeing everything before it: barriers have nothing to do.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn vkCmdPipelineBarrier(
    _c: VkCommandBuffer,
    _src: VkPipelineStageFlags,
    _dst: VkPipelineStageFlags,
    _flags: VkDependencyFlags,
    _memory_count: u32,
    _memory: *const VkMemoryBarrier,
    _buffer_count: u32,
    _buffers: *const VkBufferMemoryBarrier,
    _image_count: u32,
    _images: *const VkImageMemoryBarrier,
) {
}

// The queue, fences, semaphores and events.

struct Fence {
    signaled: bool,
}

struct Event {
    set: bool,
}

pub unsafe extern "C" fn vkQueueSubmit(queue: VkQueue, count: u32, submits: *const VkSubmitInfo, fence: VkFence) -> VkResult {
    let device = unsafe { &*(*(queue as *const Queue)).device };
    let mut work = Vec::new();
    for i in 0..count as usize {
        let s = unsafe { &*submits.add(i) };
        for j in 0..s.commandBufferCount as usize {
            work.extend_from_slice(&unsafe { command_buffer(*s.pCommandBuffers.add(j)) }.records);
        }
    }
    if device.ring(records::SUBMIT, &work).is_err() {
        return VK_ERROR_DEVICE_LOST;
    }
    if fence != 0 {
        unsafe { (*(fence as *mut Fence)).signaled = true };
    }
    VK_SUCCESS
}

/// Signals a fence from the host's side: work is finished when it's submitted, so an acquire's fence is at once.
pub(crate) fn signal_fence(fence: VkFence) {
    unsafe { (*(fence as *mut Fence)).signaled = true };
}

pub unsafe extern "C" fn vkQueueWaitIdle(_queue: VkQueue) -> VkResult {
    VK_SUCCESS
}

pub unsafe extern "C" fn vkCreateFence(_d: VkDevice, info: *const VkFenceCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkFence) -> VkResult {
    let signaled = unsafe { (*info).flags } & VK_FENCE_CREATE_SIGNALED_BIT != 0;
    unsafe { *out = Box::into_raw(Box::new(Fence { signaled })) as VkFence };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyFence(_d: VkDevice, fence: VkFence, _a: *const VkAllocationCallbacks) {
    if fence != 0 {
        drop(unsafe { Box::from_raw(fence as *mut Fence) });
    }
}

pub unsafe extern "C" fn vkResetFences(_d: VkDevice, count: u32, fences: *const VkFence) -> VkResult {
    for i in 0..count as usize {
        unsafe { (*(*fences.add(i) as *mut Fence)).signaled = false };
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkGetFenceStatus(_d: VkDevice, fence: VkFence) -> VkResult {
    if unsafe { (*(fence as *const Fence)).signaled } { VK_SUCCESS } else { VK_NOT_READY }
}

/// Submissions are finished when they return, so a fence not signaled now never will be by waiting.
pub unsafe extern "C" fn vkWaitForFences(_d: VkDevice, count: u32, fences: *const VkFence, all: VkBool32, _timeout: u64) -> VkResult {
    let signaled: Vec<bool> = (0..count as usize).map(|i| unsafe { (*(*fences.add(i) as *const Fence)).signaled }).collect();
    let done = if all != 0 { signaled.iter().all(|s| *s) } else { signaled.iter().any(|s| *s) };
    if done { VK_SUCCESS } else { VK_TIMEOUT }
}

pub unsafe extern "C" fn vkCreateSemaphore(_d: VkDevice, _info: *const VkSemaphoreCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkSemaphore) -> VkResult {
    unsafe { *out = token() };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroySemaphore(_d: VkDevice, semaphore: VkSemaphore, _a: *const VkAllocationCallbacks) {
    drop_token(semaphore);
}

pub unsafe extern "C" fn vkCreateEvent(_d: VkDevice, _info: *const VkEventCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkEvent) -> VkResult {
    unsafe { *out = Box::into_raw(Box::new(Event { set: false })) as VkEvent };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyEvent(_d: VkDevice, event: VkEvent, _a: *const VkAllocationCallbacks) {
    if event != 0 {
        drop(unsafe { Box::from_raw(event as *mut Event) });
    }
}

pub unsafe extern "C" fn vkGetEventStatus(_d: VkDevice, event: VkEvent) -> VkResult {
    if unsafe { (*(event as *const Event)).set } { VK_EVENT_SET } else { VK_EVENT_RESET }
}

pub unsafe extern "C" fn vkSetEvent(_d: VkDevice, event: VkEvent) -> VkResult {
    unsafe { (*(event as *mut Event)).set = true };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkResetEvent(_d: VkDevice, event: VkEvent) -> VkResult {
    unsafe { (*(event as *mut Event)).set = false };
    VK_SUCCESS
}

// Graphics: image views, samplers, render passes, framebuffers, graphics pipelines, and the commands that draw.

/// A view of a whole image: views of one level or layer, other formats or swizzles aren't there yet.
struct ImageView {
    image: u64,
}

pub unsafe extern "C" fn vkCreateImageView(_d: VkDevice, info: *const VkImageViewCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkImageView) -> VkResult {
    unsafe { *out = Box::into_raw(Box::new(ImageView { image: (*info).image })) as VkImageView };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyImageView(_d: VkDevice, view: VkImageView, _a: *const VkAllocationCallbacks) {
    if view != 0 {
        drop(unsafe { Box::from_raw(view as *mut ImageView) });
    }
}

pub unsafe extern "C" fn vkCreateSampler(d: VkDevice, info: *const VkSamplerCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkSampler) -> VkResult {
    let info = unsafe { &*info };
    let id = token();
    let mut p = Vec::new();
    records::put_u64(&mut p, id);
    for v in [info.magFilter, info.minFilter, info.addressModeU, info.addressModeV] {
        records::put_u32(&mut p, v as u32);
    }
    unsafe { device(d) }.record(records::SAMPLER, &p);
    unsafe { *out = id };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroySampler(d: VkDevice, sampler: VkSampler, _a: *const VkAllocationCallbacks) {
    if sampler != 0 {
        free(d, records::FREE_SAMPLER, sampler);
        drop_token(sampler);
    }
}

/// A render pass's first subpass, which is all that's drawn yet: its colour attachments and depth attachment, each
/// an attachment's index, and each attachment's format and load op.
struct RenderPass {
    attachments: Vec<(VkFormat, VkAttachmentLoadOp)>,
    colors: Vec<u32>,
    depth: Option<u32>,
}

pub unsafe extern "C" fn vkCreateRenderPass(_d: VkDevice, info: *const VkRenderPassCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkRenderPass) -> VkResult {
    let info = unsafe { &*info };
    let attachments = (0..info.attachmentCount as usize).map(|i| unsafe { ((*info.pAttachments.add(i)).format, (*info.pAttachments.add(i)).loadOp) }).collect();
    let (mut colors, mut depth) = (Vec::new(), None);
    if info.subpassCount > 0 {
        let sub = unsafe { &*info.pSubpasses };
        colors = (0..sub.colorAttachmentCount as usize).map(|i| unsafe { (*sub.pColorAttachments.add(i)).attachment }).collect();
        if !sub.pDepthStencilAttachment.is_null() && unsafe { (*sub.pDepthStencilAttachment).attachment } != VK_ATTACHMENT_UNUSED {
            depth = Some(unsafe { (*sub.pDepthStencilAttachment).attachment });
        }
    }
    unsafe { *out = Box::into_raw(Box::new(RenderPass { attachments, colors, depth })) as VkRenderPass };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyRenderPass(_d: VkDevice, pass: VkRenderPass, _a: *const VkAllocationCallbacks) {
    if pass != 0 {
        drop(unsafe { Box::from_raw(pass as *mut RenderPass) });
    }
}

pub unsafe extern "C" fn vkGetRenderAreaGranularity(_d: VkDevice, _pass: VkRenderPass, out: *mut VkExtent2D) {
    unsafe { *out = VkExtent2D { width: 1, height: 1 } };
}

/// Each attachment's image.
struct Framebuffer {
    images: Vec<u64>,
}

pub unsafe extern "C" fn vkCreateFramebuffer(_d: VkDevice, info: *const VkFramebufferCreateInfo, _a: *const VkAllocationCallbacks, out: *mut VkFramebuffer) -> VkResult {
    let info = unsafe { &*info };
    let images = (0..info.attachmentCount as usize).map(|i| unsafe { (*(*info.pAttachments.add(i) as *const ImageView)).image }).collect();
    unsafe { *out = Box::into_raw(Box::new(Framebuffer { images })) as VkFramebuffer };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroyFramebuffer(_d: VkDevice, framebuffer: VkFramebuffer, _a: *const VkAllocationCallbacks) {
    if framebuffer != 0 {
        drop(unsafe { Box::from_raw(framebuffer as *mut Framebuffer) });
    }
}

/// A graphics pipeline's record: its two stages, its fixed-function state, and its render pass's formats. Vulkan's
/// enums go as they are, for the host to read.
unsafe fn graphics_record(id: u64, info: &VkGraphicsPipelineCreateInfo, pipeline: &mut Pipeline) -> Result<Vec<u8>, ()> {
    let mut p = Vec::new();
    records::put_u64(&mut p, id);
    let stages = unsafe { core::slice::from_raw_parts(info.pStages, info.stageCount as usize) };
    for want in [VK_SHADER_STAGE_VERTEX_BIT, VK_SHADER_STAGE_FRAGMENT_BIT] {
        let stage = stages.iter().find(|s| s.stage == want).ok_or(())?;
        let module = unsafe { &*(stage.module as *const ShaderModule) };
        records::put_bytes(&mut p, unsafe { CStr::from_ptr(stage.pName) }.to_bytes());
        records::put_bytes(&mut p, &module.code);
    }
    let (bindings, attributes) = match unsafe { info.pVertexInputState.as_ref() } {
        Some(v) => unsafe {
            (
                core::slice::from_raw_parts(v.pVertexBindingDescriptions, v.vertexBindingDescriptionCount as usize),
                core::slice::from_raw_parts(v.pVertexAttributeDescriptions, v.vertexAttributeDescriptionCount as usize),
            )
        },
        None => (&[][..], &[][..]),
    };
    pipeline.vertex_bindings = bindings.iter().map(|b| b.binding).collect();
    records::put_u32(&mut p, bindings.len() as u32);
    for b in bindings {
        records::put_u32(&mut p, b.stride);
        records::put_u32(&mut p, (b.inputRate == VK_VERTEX_INPUT_RATE_INSTANCE) as u32);
    }
    records::put_u32(&mut p, attributes.len() as u32);
    for a in attributes {
        let buffer = pipeline.vertex_bindings.iter().position(|b| *b == a.binding).ok_or(())? as u32;
        for v in [a.location, buffer, a.offset, a.format as u32] {
            records::put_u32(&mut p, v);
        }
    }
    let topology = unsafe { info.pInputAssemblyState.as_ref() }.map_or(VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST, |a| a.topology);
    let (cull, front) = unsafe { info.pRasterizationState.as_ref() }.map_or((VK_CULL_MODE_NONE, VK_FRONT_FACE_COUNTER_CLOCKWISE), |r| (r.cullMode, r.frontFace));
    records::put_u32(&mut p, topology as u32);
    records::put_u32(&mut p, cull);
    records::put_u32(&mut p, front as u32);
    let pass = unsafe { &*(info.renderPass as *const RenderPass) };
    let depth_format = pass.depth.map_or(0, |a| pass.attachments[a as usize].0 as u32);
    match unsafe { info.pDepthStencilState.as_ref() } {
        Some(d) if depth_format != 0 => {
            for v in [d.depthTestEnable, d.depthCompareOp as u32, d.depthWriteEnable, depth_format] {
                records::put_u32(&mut p, v);
            }
        }
        _ => {
            for v in [0, 0, 0, depth_format] {
                records::put_u32(&mut p, v);
            }
        }
    }
    let blends = match unsafe { info.pColorBlendState.as_ref() } {
        Some(b) => unsafe { core::slice::from_raw_parts(b.pAttachments, b.attachmentCount as usize) },
        None => &[][..],
    };
    records::put_u32(&mut p, pass.colors.len() as u32);
    for (i, a) in pass.colors.iter().enumerate() {
        records::put_u32(&mut p, pass.attachments[*a as usize].0 as u32);
        match blends.get(i) {
            Some(b) => {
                for v in [b.blendEnable, b.srcColorBlendFactor as u32, b.dstColorBlendFactor as u32, b.colorBlendOp as u32, b.srcAlphaBlendFactor as u32, b.dstAlphaBlendFactor as u32, b.alphaBlendOp as u32, b.colorWriteMask] {
                    records::put_u32(&mut p, v);
                }
            }
            None => {
                for v in [0, 1, 0, 0, 1, 0, 0, 0xf] {
                    records::put_u32(&mut p, v);
                }
            }
        }
    }
    // A viewport and scissor that aren't dynamic are the pipeline's, set as it's bound.
    let dynamic = match unsafe { info.pDynamicState.as_ref() } {
        Some(d) => unsafe { core::slice::from_raw_parts(d.pDynamicStates, d.dynamicStateCount as usize) },
        None => &[][..],
    };
    if let Some(v) = unsafe { info.pViewportState.as_ref() } {
        if !dynamic.contains(&VK_DYNAMIC_STATE_VIEWPORT) && !v.pViewports.is_null() && v.viewportCount > 0 {
            pipeline.viewport = Some(unsafe { *v.pViewports });
        }
        if !dynamic.contains(&VK_DYNAMIC_STATE_SCISSOR) && !v.pScissors.is_null() && v.scissorCount > 0 {
            pipeline.scissor = Some(unsafe { *v.pScissors });
        }
    }
    Ok(p)
}

pub unsafe extern "C" fn vkCreateGraphicsPipelines(
    d: VkDevice,
    _cache: VkPipelineCache,
    count: u32,
    infos: *const VkGraphicsPipelineCreateInfo,
    _a: *const VkAllocationCallbacks,
    out: *mut VkPipeline,
) -> VkResult {
    let device = unsafe { device(d) };
    let mut result = VK_SUCCESS;
    for i in 0..count as usize {
        let info = unsafe { &*infos.add(i) };
        let pipeline = Box::into_raw(Box::new(Pipeline::default()));
        let id = pipeline as u64;
        let made = unsafe { graphics_record(id, info, &mut *pipeline) }.and_then(|p| device.ring(records::GRAPHICS, &p));
        if made.is_err() {
            drop(unsafe { Box::from_raw(pipeline) });
            unsafe { *out.add(i) = 0 };
            result = VK_ERROR_INITIALIZATION_FAILED;
            continue;
        }
        unsafe { *out.add(i) = id };
    }
    result
}

pub unsafe extern "C" fn vkCmdBeginRenderPass(c: VkCommandBuffer, info: *const VkRenderPassBeginInfo, _contents: VkSubpassContents) {
    let c = unsafe { command_buffer(c) };
    let info = unsafe { &*info };
    let pass = unsafe { &*(info.renderPass as *const RenderPass) };
    let framebuffer = unsafe { &*(info.framebuffer as *const Framebuffer) };
    let clear = |a: u32| -> VkClearValue {
        if a < info.clearValueCount { unsafe { *info.pClearValues.add(a as usize) } } else { VkClearValue { color: VkClearColorValue { float32: [0.0; 4] } } }
    };
    let mut p = Vec::new();
    records::put_u32(&mut p, pass.colors.len() as u32);
    for a in &pass.colors {
        records::put_u64(&mut p, framebuffer.images[*a as usize]);
        records::put_u32(&mut p, pass.attachments[*a as usize].1 as u32);
        for v in unsafe { clear(*a).color.float32 } {
            records::put_u32(&mut p, v.to_bits());
        }
    }
    match pass.depth {
        Some(a) => {
            records::put_u64(&mut p, framebuffer.images[a as usize]);
            records::put_u32(&mut p, pass.attachments[a as usize].1 as u32);
            records::put_u32(&mut p, unsafe { clear(a).depthStencil.depth }.to_bits());
        }
        None => {
            records::put_u64(&mut p, 0);
            records::put_u32(&mut p, 0);
            records::put_u32(&mut p, 0);
        }
    }
    records::record(&mut c.records, records::BEGIN_RENDER, &p);
}

pub unsafe extern "C" fn vkCmdEndRenderPass(c: VkCommandBuffer) {
    records::record(&mut unsafe { command_buffer(c) }.records, records::END_RENDER, &[]);
}

/// Only a render pass's first subpass is drawn yet.
pub unsafe extern "C" fn vkCmdNextSubpass(_c: VkCommandBuffer, _contents: VkSubpassContents) {}

fn viewport(c: &mut CommandBuffer, v: &VkViewport) {
    let mut p = Vec::new();
    for f in [v.x, v.y, v.width, v.height, v.minDepth, v.maxDepth] {
        records::put_u32(&mut p, f.to_bits());
    }
    records::record(&mut c.records, records::VIEWPORT, &p);
}

fn scissor(c: &mut CommandBuffer, r: &VkRect2D) {
    let mut p = Vec::new();
    for v in [r.offset.x.max(0) as u32, r.offset.y.max(0) as u32, r.extent.width, r.extent.height] {
        records::put_u32(&mut p, v);
    }
    records::record(&mut c.records, records::SCISSOR, &p);
}

pub unsafe extern "C" fn vkCmdSetViewport(c: VkCommandBuffer, first: u32, count: u32, viewports: *const VkViewport) {
    if first == 0 && count > 0 {
        viewport(unsafe { command_buffer(c) }, unsafe { &*viewports });
    }
}

pub unsafe extern "C" fn vkCmdSetScissor(c: VkCommandBuffer, first: u32, count: u32, scissors: *const VkRect2D) {
    if first == 0 && count > 0 {
        scissor(unsafe { command_buffer(c) }, unsafe { &*scissors });
    }
}

pub unsafe extern "C" fn vkCmdBindVertexBuffers(c: VkCommandBuffer, first: u32, count: u32, buffers: *const VkBuffer, offsets: *const VkDeviceSize) {
    let c = unsafe { command_buffer(c) };
    for i in 0..count as usize {
        let (b, offset) = unsafe { (buffer(*buffers.add(i)), *offsets.add(i)) };
        if let Some(slot) = c.vertex.get_mut(first as usize + i) {
            *slot = span(b, offset, VK_WHOLE_SIZE);
        }
    }
}

pub unsafe extern "C" fn vkCmdBindIndexBuffer(c: VkCommandBuffer, b: VkBuffer, offset: VkDeviceSize, ty: VkIndexType) {
    let (memory, offset, size) = span(unsafe { buffer(b) }, offset, VK_WHOLE_SIZE);
    unsafe { command_buffer(c) }.index = (memory, offset, size, ty);
}

/// A draw's record: what's bound, then the counts.
fn draw(c: &mut CommandBuffer, indexed: bool, first: u32, count: u32, vertex_offset: i32, first_instance: u32, instances: u32) {
    let (buffers, images) = bound(&c.graphics_sets);
    let pipeline = unsafe { &*(c.graphics as *const Pipeline) };
    let mut p = Vec::new();
    records::put_u64(&mut p, c.graphics);
    for v in [first, count, vertex_offset as u32, first_instance, instances, buffers.len() as u32] {
        records::put_u32(&mut p, v);
    }
    for b in &buffers {
        records::put_bound(&mut p, b);
    }
    records::put_bytes(&mut p, &c.push[..c.push_len]);
    records::put_u32(&mut p, images.len() as u32);
    for i in &images {
        records::put_bound_image(&mut p, i);
    }
    records::put_u32(&mut p, pipeline.vertex_bindings.len() as u32);
    for b in &pipeline.vertex_bindings {
        let (memory, offset, size) = c.vertex.get(*b as usize).copied().unwrap_or((0, 0, 0));
        for v in [memory, offset, size] {
            records::put_u64(&mut p, v);
        }
    }
    let (memory, offset, size, ty) = if indexed { c.index } else { (0, 0, 0, 0) };
    for v in [memory, offset, size] {
        records::put_u64(&mut p, v);
    }
    records::put_u32(&mut p, ty as u32);
    records::record(&mut c.records, records::DRAW, &p);
}

pub unsafe extern "C" fn vkCmdDraw(c: VkCommandBuffer, count: u32, instances: u32, first: u32, first_instance: u32) {
    draw(unsafe { command_buffer(c) }, false, first, count, 0, first_instance, instances);
}

pub unsafe extern "C" fn vkCmdDrawIndexed(c: VkCommandBuffer, count: u32, instances: u32, first: u32, vertex_offset: i32, first_instance: u32) {
    draw(unsafe { command_buffer(c) }, true, first, count, vertex_offset, first_instance, instances);
}

pub unsafe extern "C" fn vkCmdCopyBufferToImage(c: VkCommandBuffer, b: VkBuffer, image: VkImage, _layout: VkImageLayout, count: u32, regions: *const VkBufferImageCopy) {
    let c = unsafe { command_buffer(c) };
    let b = unsafe { buffer(b) };
    for i in 0..count as usize {
        let r = unsafe { &*regions.add(i) };
        let mut p = Vec::new();
        records::put_u64(&mut p, b.memory);
        records::put_u64(&mut p, b.offset + r.bufferOffset);
        records::put_u32(&mut p, r.bufferRowLength);
        records::put_u64(&mut p, image);
        for v in [r.imageOffset.x as u32, r.imageOffset.y as u32, r.imageExtent.width, r.imageExtent.height] {
            records::put_u32(&mut p, v);
        }
        records::record(&mut c.records, records::COPY_TO_IMAGE, &p);
    }
}

pub unsafe extern "C" fn vkCmdCopyImageToBuffer(c: VkCommandBuffer, image: VkImage, _layout: VkImageLayout, b: VkBuffer, count: u32, regions: *const VkBufferImageCopy) {
    let c = unsafe { command_buffer(c) };
    let b = unsafe { buffer(b) };
    for i in 0..count as usize {
        let r = unsafe { &*regions.add(i) };
        let mut p = Vec::new();
        records::put_u64(&mut p, image);
        for v in [r.imageOffset.x as u32, r.imageOffset.y as u32, r.imageExtent.width, r.imageExtent.height] {
            records::put_u32(&mut p, v);
        }
        records::put_u64(&mut p, b.memory);
        records::put_u64(&mut p, b.offset + r.bufferOffset);
        records::put_u32(&mut p, r.bufferRowLength);
        records::record(&mut c.records, records::COPY_FROM_IMAGE, &p);
    }
}

// Dynamic state the device doesn't have: lines are 1 wide, and there's no depth bias, blend constant, depth bounds
// or stencil yet.
pub unsafe extern "C" fn vkCmdSetLineWidth(_c: VkCommandBuffer, _width: f32) {}
pub unsafe extern "C" fn vkCmdSetDepthBias(_c: VkCommandBuffer, _constant: f32, _clamp: f32, _slope: f32) {}
pub unsafe extern "C" fn vkCmdSetBlendConstants(_c: VkCommandBuffer, _constants: *const f32) {}
pub unsafe extern "C" fn vkCmdSetDepthBounds(_c: VkCommandBuffer, _min: f32, _max: f32) {}
pub unsafe extern "C" fn vkCmdSetStencilCompareMask(_c: VkCommandBuffer, _face: VkStencilFaceFlags, _mask: u32) {}
pub unsafe extern "C" fn vkCmdSetStencilWriteMask(_c: VkCommandBuffer, _face: VkStencilFaceFlags, _mask: u32) {}
pub unsafe extern "C" fn vkCmdSetStencilReference(_c: VkCommandBuffer, _face: VkStencilFaceFlags, _reference: u32) {}

macro_rules! commands {
    ($name:expr, $($f:ident: $ty:ident),* $(,)?) => {
        match $name {
            $(n if n == stringify!($f).as_bytes() => Some($f as $ty as *const ()),)*
            _ => crate::wsi::lookup($name),
        }
    };
}

/// The device's commands, by name.
pub(crate) fn lookup(name: &[u8]) -> Option<*const ()> {
    commands!(name,
        vkGetDeviceProcAddr: PFN_vkGetDeviceProcAddr,
        vkDestroyDevice: PFN_vkDestroyDevice,
        vkGetDeviceQueue: PFN_vkGetDeviceQueue,
        vkDeviceWaitIdle: PFN_vkDeviceWaitIdle,
        vkAllocateMemory: PFN_vkAllocateMemory,
        vkFreeMemory: PFN_vkFreeMemory,
        vkMapMemory: PFN_vkMapMemory,
        vkUnmapMemory: PFN_vkUnmapMemory,
        vkFlushMappedMemoryRanges: PFN_vkFlushMappedMemoryRanges,
        vkInvalidateMappedMemoryRanges: PFN_vkInvalidateMappedMemoryRanges,
        vkGetDeviceMemoryCommitment: PFN_vkGetDeviceMemoryCommitment,
        vkCreateImage: PFN_vkCreateImage,
        vkDestroyImage: PFN_vkDestroyImage,
        vkGetImageMemoryRequirements: PFN_vkGetImageMemoryRequirements,
        vkBindImageMemory: PFN_vkBindImageMemory,
        vkGetImageSubresourceLayout: PFN_vkGetImageSubresourceLayout,
        vkGetImageSparseMemoryRequirements: PFN_vkGetImageSparseMemoryRequirements,
        vkCreateBuffer: PFN_vkCreateBuffer,
        vkDestroyBuffer: PFN_vkDestroyBuffer,
        vkGetBufferMemoryRequirements: PFN_vkGetBufferMemoryRequirements,
        vkBindBufferMemory: PFN_vkBindBufferMemory,
        vkCreateShaderModule: PFN_vkCreateShaderModule,
        vkDestroyShaderModule: PFN_vkDestroyShaderModule,
        vkCreatePipelineLayout: PFN_vkCreatePipelineLayout,
        vkDestroyPipelineLayout: PFN_vkDestroyPipelineLayout,
        vkCreatePipelineCache: PFN_vkCreatePipelineCache,
        vkDestroyPipelineCache: PFN_vkDestroyPipelineCache,
        vkGetPipelineCacheData: PFN_vkGetPipelineCacheData,
        vkMergePipelineCaches: PFN_vkMergePipelineCaches,
        vkCreateComputePipelines: PFN_vkCreateComputePipelines,
        vkDestroyPipeline: PFN_vkDestroyPipeline,
        vkCreateDescriptorSetLayout: PFN_vkCreateDescriptorSetLayout,
        vkDestroyDescriptorSetLayout: PFN_vkDestroyDescriptorSetLayout,
        vkCreateDescriptorPool: PFN_vkCreateDescriptorPool,
        vkDestroyDescriptorPool: PFN_vkDestroyDescriptorPool,
        vkResetDescriptorPool: PFN_vkResetDescriptorPool,
        vkAllocateDescriptorSets: PFN_vkAllocateDescriptorSets,
        vkFreeDescriptorSets: PFN_vkFreeDescriptorSets,
        vkUpdateDescriptorSets: PFN_vkUpdateDescriptorSets,
        vkCreateCommandPool: PFN_vkCreateCommandPool,
        vkDestroyCommandPool: PFN_vkDestroyCommandPool,
        vkResetCommandPool: PFN_vkResetCommandPool,
        vkAllocateCommandBuffers: PFN_vkAllocateCommandBuffers,
        vkFreeCommandBuffers: PFN_vkFreeCommandBuffers,
        vkBeginCommandBuffer: PFN_vkBeginCommandBuffer,
        vkEndCommandBuffer: PFN_vkEndCommandBuffer,
        vkResetCommandBuffer: PFN_vkResetCommandBuffer,
        vkCmdBindPipeline: PFN_vkCmdBindPipeline,
        vkCmdBindDescriptorSets: PFN_vkCmdBindDescriptorSets,
        vkCmdPushConstants: PFN_vkCmdPushConstants,
        vkCmdDispatch: PFN_vkCmdDispatch,
        vkCmdCopyBuffer: PFN_vkCmdCopyBuffer,
        vkCmdFillBuffer: PFN_vkCmdFillBuffer,
        vkCmdUpdateBuffer: PFN_vkCmdUpdateBuffer,
        vkCmdPipelineBarrier: PFN_vkCmdPipelineBarrier,
        vkQueueSubmit: PFN_vkQueueSubmit,
        vkQueueWaitIdle: PFN_vkQueueWaitIdle,
        vkCreateFence: PFN_vkCreateFence,
        vkDestroyFence: PFN_vkDestroyFence,
        vkResetFences: PFN_vkResetFences,
        vkGetFenceStatus: PFN_vkGetFenceStatus,
        vkWaitForFences: PFN_vkWaitForFences,
        vkCreateSemaphore: PFN_vkCreateSemaphore,
        vkDestroySemaphore: PFN_vkDestroySemaphore,
        vkCreateEvent: PFN_vkCreateEvent,
        vkDestroyEvent: PFN_vkDestroyEvent,
        vkGetEventStatus: PFN_vkGetEventStatus,
        vkSetEvent: PFN_vkSetEvent,
        vkResetEvent: PFN_vkResetEvent,
        vkCreateImageView: PFN_vkCreateImageView,
        vkDestroyImageView: PFN_vkDestroyImageView,
        vkCreateSampler: PFN_vkCreateSampler,
        vkDestroySampler: PFN_vkDestroySampler,
        vkCreateRenderPass: PFN_vkCreateRenderPass,
        vkDestroyRenderPass: PFN_vkDestroyRenderPass,
        vkGetRenderAreaGranularity: PFN_vkGetRenderAreaGranularity,
        vkCreateFramebuffer: PFN_vkCreateFramebuffer,
        vkDestroyFramebuffer: PFN_vkDestroyFramebuffer,
        vkCreateGraphicsPipelines: PFN_vkCreateGraphicsPipelines,
        vkCmdBeginRenderPass: PFN_vkCmdBeginRenderPass,
        vkCmdEndRenderPass: PFN_vkCmdEndRenderPass,
        vkCmdNextSubpass: PFN_vkCmdNextSubpass,
        vkCmdSetViewport: PFN_vkCmdSetViewport,
        vkCmdSetScissor: PFN_vkCmdSetScissor,
        vkCmdBindVertexBuffers: PFN_vkCmdBindVertexBuffers,
        vkCmdBindIndexBuffer: PFN_vkCmdBindIndexBuffer,
        vkCmdDraw: PFN_vkCmdDraw,
        vkCmdDrawIndexed: PFN_vkCmdDrawIndexed,
        vkCmdCopyBufferToImage: PFN_vkCmdCopyBufferToImage,
        vkCmdCopyImageToBuffer: PFN_vkCmdCopyImageToBuffer,
        vkCmdSetLineWidth: PFN_vkCmdSetLineWidth,
        vkCmdSetDepthBias: PFN_vkCmdSetDepthBias,
        vkCmdSetBlendConstants: PFN_vkCmdSetBlendConstants,
        vkCmdSetDepthBounds: PFN_vkCmdSetDepthBounds,
        vkCmdSetStencilCompareMask: PFN_vkCmdSetStencilCompareMask,
        vkCmdSetStencilWriteMask: PFN_vkCmdSetStencilWriteMask,
        vkCmdSetStencilReference: PFN_vkCmdSetStencilReference,
    )
}
