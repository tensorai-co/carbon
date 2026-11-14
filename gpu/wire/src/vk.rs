//! Vulkan's types for the guest driver's commands, generated from `vk.xml` (header version 341) by
//! `gpu-wire-gen`: not edited by hand.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code, clippy::all)]

use core::ffi::{c_char, c_void};

pub const VK_MAX_PHYSICAL_DEVICE_NAME_SIZE: u32 = 256;
pub const VK_UUID_SIZE: u32 = 16;
pub const VK_LUID_SIZE: u32 = 8;
pub const VK_MAX_EXTENSION_NAME_SIZE: u32 = 256;
pub const VK_MAX_DESCRIPTION_SIZE: u32 = 256;
pub const VK_MAX_MEMORY_TYPES: u32 = 32;
pub const VK_MAX_MEMORY_HEAPS: u32 = 16;
pub const VK_LOD_CLAMP_NONE: f32 = 1000.0;
pub const VK_REMAINING_MIP_LEVELS: u32 = !0;
pub const VK_REMAINING_ARRAY_LAYERS: u32 = !0;
pub const VK_REMAINING_3D_SLICES_EXT: u32 = !0;
pub const VK_WHOLE_SIZE: u64 = !0;
pub const VK_ATTACHMENT_UNUSED: u32 = !0;
pub const VK_TRUE: u32 = 1;
pub const VK_FALSE: u32 = 0;
pub const VK_QUEUE_FAMILY_IGNORED: u32 = !0;
pub const VK_QUEUE_FAMILY_EXTERNAL: u32 = !1;
pub const VK_QUEUE_FAMILY_FOREIGN_EXT: u32 = !2;
pub const VK_SUBPASS_EXTERNAL: u32 = !0;
pub const VK_MAX_DEVICE_GROUP_SIZE: u32 = 32;
pub const VK_MAX_DRIVER_NAME_SIZE: u32 = 256;
pub const VK_MAX_DRIVER_INFO_SIZE: u32 = 256;
pub const VK_SHADER_UNUSED_KHR: u32 = !0;
pub const VK_MAX_GLOBAL_PRIORITY_SIZE: u32 = 16;
pub const VK_MAX_SHADER_MODULE_IDENTIFIER_SIZE_EXT: u32 = 32;
pub const VK_MAX_PIPELINE_BINARY_KEY_SIZE_KHR: u32 = 32;
pub const VK_MAX_VIDEO_AV1_REFERENCES_PER_FRAME_KHR: u32 = 7;
pub const VK_MAX_VIDEO_VP9_REFERENCES_PER_FRAME_KHR: u32 = 3;
pub const VK_SHADER_INDEX_UNUSED_AMDX: u32 = !0;
pub const VK_PARTITIONED_ACCELERATION_STRUCTURE_PARTITION_INDEX_GLOBAL_NV: u32 = !0;
pub const VK_COMPRESSED_TRIANGLE_FORMAT_DGF1_BYTE_ALIGNMENT_AMDX: u32 = 128;
pub const VK_COMPRESSED_TRIANGLE_FORMAT_DGF1_BYTE_STRIDE_AMDX: u32 = 128;
pub const VK_MAX_PHYSICAL_DEVICE_DATA_GRAPH_OPERATION_SET_NAME_SIZE_ARM: u32 = 128;
pub const VK_DATA_GRAPH_MODEL_TOOLCHAIN_VERSION_LENGTH_QCOM: u32 = 3;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_LOW_NV: f32 = 0.25;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_NORMAL_NV: f32 = 0.50;
pub const VK_COMPUTE_OCCUPANCY_PRIORITY_HIGH_NV: f32 = 0.75;

pub type PFN_vkAllocationFunction = Option<unsafe extern "C" fn()>;
pub type PFN_vkFreeFunction = Option<unsafe extern "C" fn()>;
pub type PFN_vkInternalAllocationNotification = Option<unsafe extern "C" fn()>;
pub type PFN_vkInternalFreeNotification = Option<unsafe extern "C" fn()>;
pub type PFN_vkReallocationFunction = Option<unsafe extern "C" fn()>;
pub type PFN_vkVoidFunction = Option<unsafe extern "C" fn()>;
pub type VkAccessFlagBits = u32;
pub const VK_ACCESS_INDIRECT_COMMAND_READ_BIT: VkAccessFlagBits = 0x1;
pub const VK_ACCESS_INDEX_READ_BIT: VkAccessFlagBits = 0x2;
pub const VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT: VkAccessFlagBits = 0x4;
pub const VK_ACCESS_UNIFORM_READ_BIT: VkAccessFlagBits = 0x8;
pub const VK_ACCESS_INPUT_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x10;
pub const VK_ACCESS_SHADER_READ_BIT: VkAccessFlagBits = 0x20;
pub const VK_ACCESS_SHADER_WRITE_BIT: VkAccessFlagBits = 0x40;
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x80;
pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: VkAccessFlagBits = 0x100;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: VkAccessFlagBits = 0x200;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: VkAccessFlagBits = 0x400;
pub const VK_ACCESS_TRANSFER_READ_BIT: VkAccessFlagBits = 0x800;
pub const VK_ACCESS_TRANSFER_WRITE_BIT: VkAccessFlagBits = 0x1000;
pub const VK_ACCESS_HOST_READ_BIT: VkAccessFlagBits = 0x2000;
pub const VK_ACCESS_HOST_WRITE_BIT: VkAccessFlagBits = 0x4000;
pub const VK_ACCESS_MEMORY_READ_BIT: VkAccessFlagBits = 0x8000;
pub const VK_ACCESS_MEMORY_WRITE_BIT: VkAccessFlagBits = 0x10000;
pub const VK_ACCESS_NONE: VkAccessFlagBits = 0x0;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_WRITE_BIT_EXT: VkAccessFlagBits = 0x2000000;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_COUNTER_READ_BIT_EXT: VkAccessFlagBits = 0x4000000;
pub const VK_ACCESS_TRANSFORM_FEEDBACK_COUNTER_WRITE_BIT_EXT: VkAccessFlagBits = 0x8000000;
pub const VK_ACCESS_CONDITIONAL_RENDERING_READ_BIT_EXT: VkAccessFlagBits = 0x100000;
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_NONCOHERENT_BIT_EXT: VkAccessFlagBits = 0x80000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR: VkAccessFlagBits = 0x200000;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR: VkAccessFlagBits = 0x400000;
pub const VK_ACCESS_SHADING_RATE_IMAGE_READ_BIT_NV: VkAccessFlagBits = VK_ACCESS_FRAGMENT_SHADING_RATE_ATTACHMENT_READ_BIT_KHR;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_NV: VkAccessFlagBits = VK_ACCESS_ACCELERATION_STRUCTURE_READ_BIT_KHR;
pub const VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_NV: VkAccessFlagBits = VK_ACCESS_ACCELERATION_STRUCTURE_WRITE_BIT_KHR;
pub const VK_ACCESS_FRAGMENT_DENSITY_MAP_READ_BIT_EXT: VkAccessFlagBits = 0x1000000;
pub const VK_ACCESS_FRAGMENT_SHADING_RATE_ATTACHMENT_READ_BIT_KHR: VkAccessFlagBits = 0x800000;
pub const VK_ACCESS_COMMAND_PREPROCESS_READ_BIT_NV: VkAccessFlagBits = VK_ACCESS_COMMAND_PREPROCESS_READ_BIT_EXT;
pub const VK_ACCESS_COMMAND_PREPROCESS_WRITE_BIT_NV: VkAccessFlagBits = VK_ACCESS_COMMAND_PREPROCESS_WRITE_BIT_EXT;
pub const VK_ACCESS_NONE_KHR: VkAccessFlagBits = VK_ACCESS_NONE;
pub const VK_ACCESS_COMMAND_PREPROCESS_READ_BIT_EXT: VkAccessFlagBits = 0x20000;
pub const VK_ACCESS_COMMAND_PREPROCESS_WRITE_BIT_EXT: VkAccessFlagBits = 0x40000;
pub type VkAccessFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkAllocationCallbacks {
    pub pUserData: *mut c_void,
    pub pfnAllocation: PFN_vkAllocationFunction,
    pub pfnReallocation: PFN_vkReallocationFunction,
    pub pfnFree: PFN_vkFreeFunction,
    pub pfnInternalAllocation: PFN_vkInternalAllocationNotification,
    pub pfnInternalFree: PFN_vkInternalFreeNotification,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkApplicationInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub pApplicationName: *const c_char,
    pub applicationVersion: u32,
    pub pEngineName: *const c_char,
    pub engineVersion: u32,
    pub apiVersion: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkAttachmentDescription {
    pub flags: VkAttachmentDescriptionFlags,
    pub format: VkFormat,
    pub samples: VkSampleCountFlagBits,
    pub loadOp: VkAttachmentLoadOp,
    pub storeOp: VkAttachmentStoreOp,
    pub stencilLoadOp: VkAttachmentLoadOp,
    pub stencilStoreOp: VkAttachmentStoreOp,
    pub initialLayout: VkImageLayout,
    pub finalLayout: VkImageLayout,
}
pub type VkAttachmentDescriptionFlagBits = u32;
pub const VK_ATTACHMENT_DESCRIPTION_MAY_ALIAS_BIT: VkAttachmentDescriptionFlagBits = 0x1;
pub const VK_ATTACHMENT_DESCRIPTION_RESOLVE_SKIP_TRANSFER_FUNCTION_BIT_KHR: VkAttachmentDescriptionFlagBits = 0x2;
pub const VK_ATTACHMENT_DESCRIPTION_RESOLVE_ENABLE_TRANSFER_FUNCTION_BIT_KHR: VkAttachmentDescriptionFlagBits = 0x4;
pub type VkAttachmentDescriptionFlags = u32;
pub type VkAttachmentLoadOp = i32;
pub const VK_ATTACHMENT_LOAD_OP_LOAD: VkAttachmentLoadOp = 0;
pub const VK_ATTACHMENT_LOAD_OP_CLEAR: VkAttachmentLoadOp = 1;
pub const VK_ATTACHMENT_LOAD_OP_DONT_CARE: VkAttachmentLoadOp = 2;
pub const VK_ATTACHMENT_LOAD_OP_NONE: VkAttachmentLoadOp = 1000400000;
pub const VK_ATTACHMENT_LOAD_OP_NONE_EXT: VkAttachmentLoadOp = VK_ATTACHMENT_LOAD_OP_NONE;
pub const VK_ATTACHMENT_LOAD_OP_NONE_KHR: VkAttachmentLoadOp = VK_ATTACHMENT_LOAD_OP_NONE;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkAttachmentReference {
    pub attachment: u32,
    pub layout: VkImageLayout,
}
pub type VkAttachmentStoreOp = i32;
pub const VK_ATTACHMENT_STORE_OP_STORE: VkAttachmentStoreOp = 0;
pub const VK_ATTACHMENT_STORE_OP_DONT_CARE: VkAttachmentStoreOp = 1;
pub const VK_ATTACHMENT_STORE_OP_NONE: VkAttachmentStoreOp = 1000301000;
pub const VK_ATTACHMENT_STORE_OP_NONE_KHR: VkAttachmentStoreOp = VK_ATTACHMENT_STORE_OP_NONE;
pub const VK_ATTACHMENT_STORE_OP_NONE_QCOM: VkAttachmentStoreOp = VK_ATTACHMENT_STORE_OP_NONE;
pub const VK_ATTACHMENT_STORE_OP_NONE_EXT: VkAttachmentStoreOp = VK_ATTACHMENT_STORE_OP_NONE;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBaseInStructure {
    pub sType: VkStructureType,
    pub pNext: *const VkBaseInStructure,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBaseOutStructure {
    pub sType: VkStructureType,
    pub pNext: *mut VkBaseOutStructure,
}
pub type VkBlendFactor = i32;
pub const VK_BLEND_FACTOR_ZERO: VkBlendFactor = 0;
pub const VK_BLEND_FACTOR_ONE: VkBlendFactor = 1;
pub const VK_BLEND_FACTOR_SRC_COLOR: VkBlendFactor = 2;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR: VkBlendFactor = 3;
pub const VK_BLEND_FACTOR_DST_COLOR: VkBlendFactor = 4;
pub const VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR: VkBlendFactor = 5;
pub const VK_BLEND_FACTOR_SRC_ALPHA: VkBlendFactor = 6;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: VkBlendFactor = 7;
pub const VK_BLEND_FACTOR_DST_ALPHA: VkBlendFactor = 8;
pub const VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA: VkBlendFactor = 9;
pub const VK_BLEND_FACTOR_CONSTANT_COLOR: VkBlendFactor = 10;
pub const VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR: VkBlendFactor = 11;
pub const VK_BLEND_FACTOR_CONSTANT_ALPHA: VkBlendFactor = 12;
pub const VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_ALPHA: VkBlendFactor = 13;
pub const VK_BLEND_FACTOR_SRC_ALPHA_SATURATE: VkBlendFactor = 14;
pub const VK_BLEND_FACTOR_SRC1_COLOR: VkBlendFactor = 15;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC1_COLOR: VkBlendFactor = 16;
pub const VK_BLEND_FACTOR_SRC1_ALPHA: VkBlendFactor = 17;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC1_ALPHA: VkBlendFactor = 18;
pub type VkBlendOp = i32;
pub const VK_BLEND_OP_ADD: VkBlendOp = 0;
pub const VK_BLEND_OP_SUBTRACT: VkBlendOp = 1;
pub const VK_BLEND_OP_REVERSE_SUBTRACT: VkBlendOp = 2;
pub const VK_BLEND_OP_MIN: VkBlendOp = 3;
pub const VK_BLEND_OP_MAX: VkBlendOp = 4;
pub const VK_BLEND_OP_ZERO_EXT: VkBlendOp = 1000148000;
pub const VK_BLEND_OP_SRC_EXT: VkBlendOp = 1000148001;
pub const VK_BLEND_OP_DST_EXT: VkBlendOp = 1000148002;
pub const VK_BLEND_OP_SRC_OVER_EXT: VkBlendOp = 1000148003;
pub const VK_BLEND_OP_DST_OVER_EXT: VkBlendOp = 1000148004;
pub const VK_BLEND_OP_SRC_IN_EXT: VkBlendOp = 1000148005;
pub const VK_BLEND_OP_DST_IN_EXT: VkBlendOp = 1000148006;
pub const VK_BLEND_OP_SRC_OUT_EXT: VkBlendOp = 1000148007;
pub const VK_BLEND_OP_DST_OUT_EXT: VkBlendOp = 1000148008;
pub const VK_BLEND_OP_SRC_ATOP_EXT: VkBlendOp = 1000148009;
pub const VK_BLEND_OP_DST_ATOP_EXT: VkBlendOp = 1000148010;
pub const VK_BLEND_OP_XOR_EXT: VkBlendOp = 1000148011;
pub const VK_BLEND_OP_MULTIPLY_EXT: VkBlendOp = 1000148012;
pub const VK_BLEND_OP_SCREEN_EXT: VkBlendOp = 1000148013;
pub const VK_BLEND_OP_OVERLAY_EXT: VkBlendOp = 1000148014;
pub const VK_BLEND_OP_DARKEN_EXT: VkBlendOp = 1000148015;
pub const VK_BLEND_OP_LIGHTEN_EXT: VkBlendOp = 1000148016;
pub const VK_BLEND_OP_COLORDODGE_EXT: VkBlendOp = 1000148017;
pub const VK_BLEND_OP_COLORBURN_EXT: VkBlendOp = 1000148018;
pub const VK_BLEND_OP_HARDLIGHT_EXT: VkBlendOp = 1000148019;
pub const VK_BLEND_OP_SOFTLIGHT_EXT: VkBlendOp = 1000148020;
pub const VK_BLEND_OP_DIFFERENCE_EXT: VkBlendOp = 1000148021;
pub const VK_BLEND_OP_EXCLUSION_EXT: VkBlendOp = 1000148022;
pub const VK_BLEND_OP_INVERT_EXT: VkBlendOp = 1000148023;
pub const VK_BLEND_OP_INVERT_RGB_EXT: VkBlendOp = 1000148024;
pub const VK_BLEND_OP_LINEARDODGE_EXT: VkBlendOp = 1000148025;
pub const VK_BLEND_OP_LINEARBURN_EXT: VkBlendOp = 1000148026;
pub const VK_BLEND_OP_VIVIDLIGHT_EXT: VkBlendOp = 1000148027;
pub const VK_BLEND_OP_LINEARLIGHT_EXT: VkBlendOp = 1000148028;
pub const VK_BLEND_OP_PINLIGHT_EXT: VkBlendOp = 1000148029;
pub const VK_BLEND_OP_HARDMIX_EXT: VkBlendOp = 1000148030;
pub const VK_BLEND_OP_HSL_HUE_EXT: VkBlendOp = 1000148031;
pub const VK_BLEND_OP_HSL_SATURATION_EXT: VkBlendOp = 1000148032;
pub const VK_BLEND_OP_HSL_COLOR_EXT: VkBlendOp = 1000148033;
pub const VK_BLEND_OP_HSL_LUMINOSITY_EXT: VkBlendOp = 1000148034;
pub const VK_BLEND_OP_PLUS_EXT: VkBlendOp = 1000148035;
pub const VK_BLEND_OP_PLUS_CLAMPED_EXT: VkBlendOp = 1000148036;
pub const VK_BLEND_OP_PLUS_CLAMPED_ALPHA_EXT: VkBlendOp = 1000148037;
pub const VK_BLEND_OP_PLUS_DARKER_EXT: VkBlendOp = 1000148038;
pub const VK_BLEND_OP_MINUS_EXT: VkBlendOp = 1000148039;
pub const VK_BLEND_OP_MINUS_CLAMPED_EXT: VkBlendOp = 1000148040;
pub const VK_BLEND_OP_CONTRAST_EXT: VkBlendOp = 1000148041;
pub const VK_BLEND_OP_INVERT_OVG_EXT: VkBlendOp = 1000148042;
pub const VK_BLEND_OP_RED_EXT: VkBlendOp = 1000148043;
pub const VK_BLEND_OP_GREEN_EXT: VkBlendOp = 1000148044;
pub const VK_BLEND_OP_BLUE_EXT: VkBlendOp = 1000148045;
pub type VkBool32 = u32;
pub type VkBorderColor = i32;
pub const VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK: VkBorderColor = 0;
pub const VK_BORDER_COLOR_INT_TRANSPARENT_BLACK: VkBorderColor = 1;
pub const VK_BORDER_COLOR_FLOAT_OPAQUE_BLACK: VkBorderColor = 2;
pub const VK_BORDER_COLOR_INT_OPAQUE_BLACK: VkBorderColor = 3;
pub const VK_BORDER_COLOR_FLOAT_OPAQUE_WHITE: VkBorderColor = 4;
pub const VK_BORDER_COLOR_INT_OPAQUE_WHITE: VkBorderColor = 5;
pub const VK_BORDER_COLOR_FLOAT_CUSTOM_EXT: VkBorderColor = 1000287003;
pub const VK_BORDER_COLOR_INT_CUSTOM_EXT: VkBorderColor = 1000287004;
pub type VkBuffer = u64;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBufferCopy {
    pub srcOffset: VkDeviceSize,
    pub dstOffset: VkDeviceSize,
    pub size: VkDeviceSize,
}
pub type VkBufferCreateFlagBits = u32;
pub const VK_BUFFER_CREATE_SPARSE_BINDING_BIT: VkBufferCreateFlagBits = 0x1;
pub const VK_BUFFER_CREATE_SPARSE_RESIDENCY_BIT: VkBufferCreateFlagBits = 0x2;
pub const VK_BUFFER_CREATE_SPARSE_ALIASED_BIT: VkBufferCreateFlagBits = 0x4;
pub const VK_BUFFER_CREATE_PROTECTED_BIT: VkBufferCreateFlagBits = 0x8;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT: VkBufferCreateFlagBits = 0x10;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT_EXT: VkBufferCreateFlagBits = VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT;
pub const VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT_KHR: VkBufferCreateFlagBits = VK_BUFFER_CREATE_DEVICE_ADDRESS_CAPTURE_REPLAY_BIT;
pub const VK_BUFFER_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkBufferCreateFlagBits = 0x20;
pub const VK_BUFFER_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR: VkBufferCreateFlagBits = 0x40;
pub type VkBufferCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBufferCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkBufferCreateFlags,
    pub size: VkDeviceSize,
    pub usage: VkBufferUsageFlags,
    pub sharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBufferImageCopy {
    pub bufferOffset: VkDeviceSize,
    pub bufferRowLength: u32,
    pub bufferImageHeight: u32,
    pub imageSubresource: VkImageSubresourceLayers,
    pub imageOffset: VkOffset3D,
    pub imageExtent: VkExtent3D,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkBufferMemoryBarrier {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub srcQueueFamilyIndex: u32,
    pub dstQueueFamilyIndex: u32,
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
}
pub type VkBufferUsageFlagBits = u32;
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: VkBufferUsageFlagBits = 0x1;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: VkBufferUsageFlagBits = 0x2;
pub const VK_BUFFER_USAGE_UNIFORM_TEXEL_BUFFER_BIT: VkBufferUsageFlagBits = 0x4;
pub const VK_BUFFER_USAGE_STORAGE_TEXEL_BUFFER_BIT: VkBufferUsageFlagBits = 0x8;
pub const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: VkBufferUsageFlagBits = 0x10;
pub const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: VkBufferUsageFlagBits = 0x20;
pub const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: VkBufferUsageFlagBits = 0x40;
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: VkBufferUsageFlagBits = 0x80;
pub const VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT: VkBufferUsageFlagBits = 0x100;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT: VkBufferUsageFlagBits = 0x20000;
pub const VK_BUFFER_USAGE_VIDEO_DECODE_SRC_BIT_KHR: VkBufferUsageFlagBits = 0x2000;
pub const VK_BUFFER_USAGE_VIDEO_DECODE_DST_BIT_KHR: VkBufferUsageFlagBits = 0x4000;
pub const VK_BUFFER_USAGE_TRANSFORM_FEEDBACK_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x800;
pub const VK_BUFFER_USAGE_TRANSFORM_FEEDBACK_COUNTER_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x1000;
pub const VK_BUFFER_USAGE_CONDITIONAL_RENDERING_BIT_EXT: VkBufferUsageFlagBits = 0x200;
pub const VK_BUFFER_USAGE_EXECUTION_GRAPH_SCRATCH_BIT_AMDX: VkBufferUsageFlagBits = 0x2000000;
pub const VK_BUFFER_USAGE_DESCRIPTOR_HEAP_BIT_EXT: VkBufferUsageFlagBits = 0x10000000;
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_BIT_KHR: VkBufferUsageFlagBits = 0x80000;
pub const VK_BUFFER_USAGE_ACCELERATION_STRUCTURE_STORAGE_BIT_KHR: VkBufferUsageFlagBits = 0x100000;
pub const VK_BUFFER_USAGE_SHADER_BINDING_TABLE_BIT_KHR: VkBufferUsageFlagBits = 0x400;
pub const VK_BUFFER_USAGE_RAY_TRACING_BIT_NV: VkBufferUsageFlagBits = VK_BUFFER_USAGE_SHADER_BINDING_TABLE_BIT_KHR;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_EXT: VkBufferUsageFlagBits = VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT;
pub const VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT_KHR: VkBufferUsageFlagBits = VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT;
pub const VK_BUFFER_USAGE_VIDEO_ENCODE_DST_BIT_KHR: VkBufferUsageFlagBits = 0x8000;
pub const VK_BUFFER_USAGE_VIDEO_ENCODE_SRC_BIT_KHR: VkBufferUsageFlagBits = 0x10000;
pub const VK_BUFFER_USAGE_SAMPLER_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x200000;
pub const VK_BUFFER_USAGE_RESOURCE_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x400000;
pub const VK_BUFFER_USAGE_PUSH_DESCRIPTORS_DESCRIPTOR_BUFFER_BIT_EXT: VkBufferUsageFlagBits = 0x4000000;
pub const VK_BUFFER_USAGE_MICROMAP_BUILD_INPUT_READ_ONLY_BIT_EXT: VkBufferUsageFlagBits = 0x800000;
pub const VK_BUFFER_USAGE_MICROMAP_STORAGE_BIT_EXT: VkBufferUsageFlagBits = 0x1000000;
pub const VK_BUFFER_USAGE_TILE_MEMORY_BIT_QCOM: VkBufferUsageFlagBits = 0x8000000;
pub type VkBufferUsageFlags = u32;
pub type VkBufferView = u64;
#[repr(C)]
#[derive(Clone, Copy)]
pub union VkClearColorValue {
    pub float32: [f32; 4 as usize],
    pub int32: [i32; 4 as usize],
    pub uint32: [u32; 4 as usize],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkClearDepthStencilValue {
    pub depth: f32,
    pub stencil: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union VkClearValue {
    pub color: VkClearColorValue,
    pub depthStencil: VkClearDepthStencilValue,
}
pub type VkColorComponentFlagBits = u32;
pub const VK_COLOR_COMPONENT_R_BIT: VkColorComponentFlagBits = 0x1;
pub const VK_COLOR_COMPONENT_G_BIT: VkColorComponentFlagBits = 0x2;
pub const VK_COLOR_COMPONENT_B_BIT: VkColorComponentFlagBits = 0x4;
pub const VK_COLOR_COMPONENT_A_BIT: VkColorComponentFlagBits = 0x8;
pub type VkColorComponentFlags = u32;
pub type VkColorSpaceKHR = i32;
pub const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = 0;
pub const VK_COLORSPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
pub const VK_COLOR_SPACE_DISPLAY_P3_NONLINEAR_EXT: VkColorSpaceKHR = 1000104001;
pub const VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT: VkColorSpaceKHR = 1000104002;
pub const VK_COLOR_SPACE_DISPLAY_P3_LINEAR_EXT: VkColorSpaceKHR = 1000104003;
pub const VK_COLOR_SPACE_DCI_P3_NONLINEAR_EXT: VkColorSpaceKHR = 1000104004;
pub const VK_COLOR_SPACE_BT709_LINEAR_EXT: VkColorSpaceKHR = 1000104005;
pub const VK_COLOR_SPACE_BT709_NONLINEAR_EXT: VkColorSpaceKHR = 1000104006;
pub const VK_COLOR_SPACE_BT2020_LINEAR_EXT: VkColorSpaceKHR = 1000104007;
pub const VK_COLOR_SPACE_HDR10_ST2084_EXT: VkColorSpaceKHR = 1000104008;
pub const VK_COLOR_SPACE_DOLBYVISION_EXT: VkColorSpaceKHR = 1000104009;
pub const VK_COLOR_SPACE_HDR10_HLG_EXT: VkColorSpaceKHR = 1000104010;
pub const VK_COLOR_SPACE_ADOBERGB_LINEAR_EXT: VkColorSpaceKHR = 1000104011;
pub const VK_COLOR_SPACE_ADOBERGB_NONLINEAR_EXT: VkColorSpaceKHR = 1000104012;
pub const VK_COLOR_SPACE_PASS_THROUGH_EXT: VkColorSpaceKHR = 1000104013;
pub const VK_COLOR_SPACE_EXTENDED_SRGB_NONLINEAR_EXT: VkColorSpaceKHR = 1000104014;
pub const VK_COLOR_SPACE_DCI_P3_LINEAR_EXT: VkColorSpaceKHR = VK_COLOR_SPACE_DISPLAY_P3_LINEAR_EXT;
pub const VK_COLOR_SPACE_DISPLAY_NATIVE_AMD: VkColorSpaceKHR = 1000213000;
pub type VkCommandBuffer = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkCommandBufferAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub commandPool: VkCommandPool,
    pub level: VkCommandBufferLevel,
    pub commandBufferCount: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkCommandBufferBeginInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkCommandBufferUsageFlags,
    pub pInheritanceInfo: *const VkCommandBufferInheritanceInfo,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkCommandBufferInheritanceInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub renderPass: VkRenderPass,
    pub subpass: u32,
    pub framebuffer: VkFramebuffer,
    pub occlusionQueryEnable: VkBool32,
    pub queryFlags: VkQueryControlFlags,
    pub pipelineStatistics: VkQueryPipelineStatisticFlags,
}
pub type VkCommandBufferLevel = i32;
pub const VK_COMMAND_BUFFER_LEVEL_PRIMARY: VkCommandBufferLevel = 0;
pub const VK_COMMAND_BUFFER_LEVEL_SECONDARY: VkCommandBufferLevel = 1;
pub type VkCommandBufferResetFlagBits = u32;
pub const VK_COMMAND_BUFFER_RESET_RELEASE_RESOURCES_BIT: VkCommandBufferResetFlagBits = 0x1;
pub type VkCommandBufferResetFlags = u32;
pub type VkCommandBufferUsageFlagBits = u32;
pub const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkCommandBufferUsageFlagBits = 0x1;
pub const VK_COMMAND_BUFFER_USAGE_RENDER_PASS_CONTINUE_BIT: VkCommandBufferUsageFlagBits = 0x2;
pub const VK_COMMAND_BUFFER_USAGE_SIMULTANEOUS_USE_BIT: VkCommandBufferUsageFlagBits = 0x4;
pub type VkCommandBufferUsageFlags = u32;
pub type VkCommandPool = u64;
pub type VkCommandPoolCreateFlagBits = u32;
pub const VK_COMMAND_POOL_CREATE_TRANSIENT_BIT: VkCommandPoolCreateFlagBits = 0x1;
pub const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkCommandPoolCreateFlagBits = 0x2;
pub const VK_COMMAND_POOL_CREATE_PROTECTED_BIT: VkCommandPoolCreateFlagBits = 0x4;
pub type VkCommandPoolCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkCommandPoolCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkCommandPoolCreateFlags,
    pub queueFamilyIndex: u32,
}
pub type VkCommandPoolResetFlagBits = u32;
pub const VK_COMMAND_POOL_RESET_RELEASE_RESOURCES_BIT: VkCommandPoolResetFlagBits = 0x1;
pub type VkCommandPoolResetFlags = u32;
pub type VkCompareOp = i32;
pub const VK_COMPARE_OP_NEVER: VkCompareOp = 0;
pub const VK_COMPARE_OP_LESS: VkCompareOp = 1;
pub const VK_COMPARE_OP_EQUAL: VkCompareOp = 2;
pub const VK_COMPARE_OP_LESS_OR_EQUAL: VkCompareOp = 3;
pub const VK_COMPARE_OP_GREATER: VkCompareOp = 4;
pub const VK_COMPARE_OP_NOT_EQUAL: VkCompareOp = 5;
pub const VK_COMPARE_OP_GREATER_OR_EQUAL: VkCompareOp = 6;
pub const VK_COMPARE_OP_ALWAYS: VkCompareOp = 7;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkComponentMapping {
    pub r: VkComponentSwizzle,
    pub g: VkComponentSwizzle,
    pub b: VkComponentSwizzle,
    pub a: VkComponentSwizzle,
}
pub type VkComponentSwizzle = i32;
pub const VK_COMPONENT_SWIZZLE_IDENTITY: VkComponentSwizzle = 0;
pub const VK_COMPONENT_SWIZZLE_ZERO: VkComponentSwizzle = 1;
pub const VK_COMPONENT_SWIZZLE_ONE: VkComponentSwizzle = 2;
pub const VK_COMPONENT_SWIZZLE_R: VkComponentSwizzle = 3;
pub const VK_COMPONENT_SWIZZLE_G: VkComponentSwizzle = 4;
pub const VK_COMPONENT_SWIZZLE_B: VkComponentSwizzle = 5;
pub const VK_COMPONENT_SWIZZLE_A: VkComponentSwizzle = 6;
pub type VkCompositeAlphaFlagBitsKHR = u32;
pub const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x1;
pub const VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x2;
pub const VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x4;
pub const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x8;
pub type VkCompositeAlphaFlagsKHR = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkComputePipelineCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineCreateFlags,
    pub stage: VkPipelineShaderStageCreateInfo,
    pub layout: VkPipelineLayout,
    pub basePipelineHandle: VkPipeline,
    pub basePipelineIndex: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkConformanceVersion {
    pub major: u8,
    pub minor: u8,
    pub subminor: u8,
    pub patch: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkCopyDescriptorSet {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcSet: VkDescriptorSet,
    pub srcBinding: u32,
    pub srcArrayElement: u32,
    pub dstSet: VkDescriptorSet,
    pub dstBinding: u32,
    pub dstArrayElement: u32,
    pub descriptorCount: u32,
}
pub type VkCullModeFlagBits = u32;
pub const VK_CULL_MODE_NONE: VkCullModeFlagBits = 0x0;
pub const VK_CULL_MODE_FRONT_BIT: VkCullModeFlagBits = 0x1;
pub const VK_CULL_MODE_BACK_BIT: VkCullModeFlagBits = 0x2;
pub const VK_CULL_MODE_FRONT_AND_BACK: VkCullModeFlagBits = 0x3;
pub type VkCullModeFlags = u32;
pub type VkDependencyFlagBits = u32;
pub const VK_DEPENDENCY_BY_REGION_BIT: VkDependencyFlagBits = 0x1;
pub const VK_DEPENDENCY_DEVICE_GROUP_BIT: VkDependencyFlagBits = 0x4;
pub const VK_DEPENDENCY_VIEW_LOCAL_BIT: VkDependencyFlagBits = 0x2;
pub const VK_DEPENDENCY_VIEW_LOCAL_BIT_KHR: VkDependencyFlagBits = VK_DEPENDENCY_VIEW_LOCAL_BIT;
pub const VK_DEPENDENCY_DEVICE_GROUP_BIT_KHR: VkDependencyFlagBits = VK_DEPENDENCY_DEVICE_GROUP_BIT;
pub const VK_DEPENDENCY_FEEDBACK_LOOP_BIT_EXT: VkDependencyFlagBits = 0x8;
pub const VK_DEPENDENCY_QUEUE_FAMILY_OWNERSHIP_TRANSFER_USE_ALL_STAGES_BIT_KHR: VkDependencyFlagBits = 0x20;
pub const VK_DEPENDENCY_ASYMMETRIC_EVENT_BIT_KHR: VkDependencyFlagBits = 0x40;
pub type VkDependencyFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorBufferInfo {
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub range: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorImageInfo {
    pub sampler: VkSampler,
    pub imageView: VkImageView,
    pub imageLayout: VkImageLayout,
}
pub type VkDescriptorPool = u64;
pub type VkDescriptorPoolCreateFlagBits = u32;
pub const VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: VkDescriptorPoolCreateFlagBits = 0x1;
pub const VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT: VkDescriptorPoolCreateFlagBits = 0x2;
pub const VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT_EXT: VkDescriptorPoolCreateFlagBits = VK_DESCRIPTOR_POOL_CREATE_UPDATE_AFTER_BIND_BIT;
pub const VK_DESCRIPTOR_POOL_CREATE_HOST_ONLY_BIT_VALVE: VkDescriptorPoolCreateFlagBits = VK_DESCRIPTOR_POOL_CREATE_HOST_ONLY_BIT_EXT;
pub const VK_DESCRIPTOR_POOL_CREATE_HOST_ONLY_BIT_EXT: VkDescriptorPoolCreateFlagBits = 0x4;
pub const VK_DESCRIPTOR_POOL_CREATE_ALLOW_OVERALLOCATION_SETS_BIT_NV: VkDescriptorPoolCreateFlagBits = 0x8;
pub const VK_DESCRIPTOR_POOL_CREATE_ALLOW_OVERALLOCATION_POOLS_BIT_NV: VkDescriptorPoolCreateFlagBits = 0x10;
pub type VkDescriptorPoolCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorPoolCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkDescriptorPoolCreateFlags,
    pub maxSets: u32,
    pub poolSizeCount: u32,
    pub pPoolSizes: *const VkDescriptorPoolSize,
}
pub type VkDescriptorPoolResetFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorPoolSize {
    pub type_: VkDescriptorType,
    pub descriptorCount: u32,
}
pub type VkDescriptorSet = u64;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorSetAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub descriptorPool: VkDescriptorPool,
    pub descriptorSetCount: u32,
    pub pSetLayouts: *const VkDescriptorSetLayout,
}
pub type VkDescriptorSetLayout = u64;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorSetLayoutBinding {
    pub binding: u32,
    pub descriptorType: VkDescriptorType,
    pub descriptorCount: u32,
    pub stageFlags: VkShaderStageFlags,
    pub pImmutableSamplers: *const VkSampler,
}
pub type VkDescriptorSetLayoutCreateFlagBits = u32;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT: VkDescriptorSetLayoutCreateFlagBits = 0x2;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT: VkDescriptorSetLayoutCreateFlagBits = 0x1;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT_KHR: VkDescriptorSetLayoutCreateFlagBits = VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = VK_DESCRIPTOR_SET_LAYOUT_CREATE_UPDATE_AFTER_BIND_POOL_BIT;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_DESCRIPTOR_BUFFER_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 0x10;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_EMBEDDED_IMMUTABLE_SAMPLERS_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 0x20;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_HOST_ONLY_POOL_BIT_VALVE: VkDescriptorSetLayoutCreateFlagBits = VK_DESCRIPTOR_SET_LAYOUT_CREATE_HOST_ONLY_POOL_BIT_EXT;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_INDIRECT_BINDABLE_BIT_NV: VkDescriptorSetLayoutCreateFlagBits = 0x80;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_HOST_ONLY_POOL_BIT_EXT: VkDescriptorSetLayoutCreateFlagBits = 0x4;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PER_STAGE_BIT_NV: VkDescriptorSetLayoutCreateFlagBits = 0x40;
pub type VkDescriptorSetLayoutCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDescriptorSetLayoutCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkDescriptorSetLayoutCreateFlags,
    pub bindingCount: u32,
    pub pBindings: *const VkDescriptorSetLayoutBinding,
}
pub type VkDescriptorType = i32;
pub const VK_DESCRIPTOR_TYPE_SAMPLER: VkDescriptorType = 0;
pub const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: VkDescriptorType = 1;
pub const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: VkDescriptorType = 2;
pub const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: VkDescriptorType = 3;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_TEXEL_BUFFER: VkDescriptorType = 4;
pub const VK_DESCRIPTOR_TYPE_STORAGE_TEXEL_BUFFER: VkDescriptorType = 5;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: VkDescriptorType = 6;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: VkDescriptorType = 7;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC: VkDescriptorType = 8;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC: VkDescriptorType = 9;
pub const VK_DESCRIPTOR_TYPE_INPUT_ATTACHMENT: VkDescriptorType = 10;
pub const VK_DESCRIPTOR_TYPE_INLINE_UNIFORM_BLOCK: VkDescriptorType = 1000138000;
pub const VK_DESCRIPTOR_TYPE_INLINE_UNIFORM_BLOCK_EXT: VkDescriptorType = VK_DESCRIPTOR_TYPE_INLINE_UNIFORM_BLOCK;
pub const VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_KHR: VkDescriptorType = 1000150000;
pub const VK_DESCRIPTOR_TYPE_ACCELERATION_STRUCTURE_NV: VkDescriptorType = 1000165000;
pub const VK_DESCRIPTOR_TYPE_MUTABLE_VALVE: VkDescriptorType = VK_DESCRIPTOR_TYPE_MUTABLE_EXT;
pub const VK_DESCRIPTOR_TYPE_SAMPLE_WEIGHT_IMAGE_QCOM: VkDescriptorType = 1000440000;
pub const VK_DESCRIPTOR_TYPE_BLOCK_MATCH_IMAGE_QCOM: VkDescriptorType = 1000440001;
pub const VK_DESCRIPTOR_TYPE_TENSOR_ARM: VkDescriptorType = 1000460000;
pub const VK_DESCRIPTOR_TYPE_MUTABLE_EXT: VkDescriptorType = 1000351000;
pub const VK_DESCRIPTOR_TYPE_PARTITIONED_ACCELERATION_STRUCTURE_NV: VkDescriptorType = 1000570000;
pub type VkDevice = *mut c_void;
pub type VkDeviceCreateFlagBits = u32;
pub type VkDeviceCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDeviceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkDeviceCreateFlags,
    pub queueCreateInfoCount: u32,
    pub pQueueCreateInfos: *const VkDeviceQueueCreateInfo,
    pub enabledLayerCount: u32,
    pub ppEnabledLayerNames: *const *const c_char,
    pub enabledExtensionCount: u32,
    pub ppEnabledExtensionNames: *const *const c_char,
    pub pEnabledFeatures: *const VkPhysicalDeviceFeatures,
}
pub type VkDeviceMemory = u64;
pub type VkDeviceQueueCreateFlagBits = u32;
pub const VK_DEVICE_QUEUE_CREATE_PROTECTED_BIT: VkDeviceQueueCreateFlagBits = 0x1;
pub const VK_DEVICE_QUEUE_CREATE_INTERNALLY_SYNCHRONIZED_BIT_KHR: VkDeviceQueueCreateFlagBits = 0x4;
pub type VkDeviceQueueCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkDeviceQueueCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkDeviceQueueCreateFlags,
    pub queueFamilyIndex: u32,
    pub queueCount: u32,
    pub pQueuePriorities: *const f32,
}
pub type VkDeviceSize = u64;
pub type VkDriverId = i32;
pub const VK_DRIVER_ID_AMD_PROPRIETARY: VkDriverId = 1;
pub const VK_DRIVER_ID_AMD_OPEN_SOURCE: VkDriverId = 2;
pub const VK_DRIVER_ID_MESA_RADV: VkDriverId = 3;
pub const VK_DRIVER_ID_NVIDIA_PROPRIETARY: VkDriverId = 4;
pub const VK_DRIVER_ID_INTEL_PROPRIETARY_WINDOWS: VkDriverId = 5;
pub const VK_DRIVER_ID_INTEL_OPEN_SOURCE_MESA: VkDriverId = 6;
pub const VK_DRIVER_ID_IMAGINATION_PROPRIETARY: VkDriverId = 7;
pub const VK_DRIVER_ID_QUALCOMM_PROPRIETARY: VkDriverId = 8;
pub const VK_DRIVER_ID_ARM_PROPRIETARY: VkDriverId = 9;
pub const VK_DRIVER_ID_GOOGLE_SWIFTSHADER: VkDriverId = 10;
pub const VK_DRIVER_ID_GGP_PROPRIETARY: VkDriverId = 11;
pub const VK_DRIVER_ID_BROADCOM_PROPRIETARY: VkDriverId = 12;
pub const VK_DRIVER_ID_MESA_LLVMPIPE: VkDriverId = 13;
pub const VK_DRIVER_ID_MOLTENVK: VkDriverId = 14;
pub const VK_DRIVER_ID_COREAVI_PROPRIETARY: VkDriverId = 15;
pub const VK_DRIVER_ID_JUICE_PROPRIETARY: VkDriverId = 16;
pub const VK_DRIVER_ID_VERISILICON_PROPRIETARY: VkDriverId = 17;
pub const VK_DRIVER_ID_MESA_TURNIP: VkDriverId = 18;
pub const VK_DRIVER_ID_MESA_V3DV: VkDriverId = 19;
pub const VK_DRIVER_ID_MESA_PANVK: VkDriverId = 20;
pub const VK_DRIVER_ID_SAMSUNG_PROPRIETARY: VkDriverId = 21;
pub const VK_DRIVER_ID_MESA_VENUS: VkDriverId = 22;
pub const VK_DRIVER_ID_MESA_DOZEN: VkDriverId = 23;
pub const VK_DRIVER_ID_MESA_NVK: VkDriverId = 24;
pub const VK_DRIVER_ID_IMAGINATION_OPEN_SOURCE_MESA: VkDriverId = 25;
pub const VK_DRIVER_ID_MESA_HONEYKRISP: VkDriverId = 26;
pub const VK_DRIVER_ID_VULKAN_SC_EMULATION_ON_VULKAN: VkDriverId = 27;
pub const VK_DRIVER_ID_MESA_KOSMICKRISP: VkDriverId = 28;
pub const VK_DRIVER_ID_AMD_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_AMD_PROPRIETARY;
pub const VK_DRIVER_ID_AMD_OPEN_SOURCE_KHR: VkDriverId = VK_DRIVER_ID_AMD_OPEN_SOURCE;
pub const VK_DRIVER_ID_MESA_RADV_KHR: VkDriverId = VK_DRIVER_ID_MESA_RADV;
pub const VK_DRIVER_ID_NVIDIA_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_NVIDIA_PROPRIETARY;
pub const VK_DRIVER_ID_INTEL_PROPRIETARY_WINDOWS_KHR: VkDriverId = VK_DRIVER_ID_INTEL_PROPRIETARY_WINDOWS;
pub const VK_DRIVER_ID_INTEL_OPEN_SOURCE_MESA_KHR: VkDriverId = VK_DRIVER_ID_INTEL_OPEN_SOURCE_MESA;
pub const VK_DRIVER_ID_IMAGINATION_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_IMAGINATION_PROPRIETARY;
pub const VK_DRIVER_ID_QUALCOMM_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_QUALCOMM_PROPRIETARY;
pub const VK_DRIVER_ID_ARM_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_ARM_PROPRIETARY;
pub const VK_DRIVER_ID_GOOGLE_SWIFTSHADER_KHR: VkDriverId = VK_DRIVER_ID_GOOGLE_SWIFTSHADER;
pub const VK_DRIVER_ID_GGP_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_GGP_PROPRIETARY;
pub const VK_DRIVER_ID_BROADCOM_PROPRIETARY_KHR: VkDriverId = VK_DRIVER_ID_BROADCOM_PROPRIETARY;
pub type VkDynamicState = i32;
pub const VK_DYNAMIC_STATE_VIEWPORT: VkDynamicState = 0;
pub const VK_DYNAMIC_STATE_SCISSOR: VkDynamicState = 1;
pub const VK_DYNAMIC_STATE_LINE_WIDTH: VkDynamicState = 2;
pub const VK_DYNAMIC_STATE_DEPTH_BIAS: VkDynamicState = 3;
pub const VK_DYNAMIC_STATE_BLEND_CONSTANTS: VkDynamicState = 4;
pub const VK_DYNAMIC_STATE_DEPTH_BOUNDS: VkDynamicState = 5;
pub const VK_DYNAMIC_STATE_STENCIL_COMPARE_MASK: VkDynamicState = 6;
pub const VK_DYNAMIC_STATE_STENCIL_WRITE_MASK: VkDynamicState = 7;
pub const VK_DYNAMIC_STATE_STENCIL_REFERENCE: VkDynamicState = 8;
pub const VK_DYNAMIC_STATE_CULL_MODE: VkDynamicState = 1000267000;
pub const VK_DYNAMIC_STATE_FRONT_FACE: VkDynamicState = 1000267001;
pub const VK_DYNAMIC_STATE_PRIMITIVE_TOPOLOGY: VkDynamicState = 1000267002;
pub const VK_DYNAMIC_STATE_VIEWPORT_WITH_COUNT: VkDynamicState = 1000267003;
pub const VK_DYNAMIC_STATE_SCISSOR_WITH_COUNT: VkDynamicState = 1000267004;
pub const VK_DYNAMIC_STATE_VERTEX_INPUT_BINDING_STRIDE: VkDynamicState = 1000267005;
pub const VK_DYNAMIC_STATE_DEPTH_TEST_ENABLE: VkDynamicState = 1000267006;
pub const VK_DYNAMIC_STATE_DEPTH_WRITE_ENABLE: VkDynamicState = 1000267007;
pub const VK_DYNAMIC_STATE_DEPTH_COMPARE_OP: VkDynamicState = 1000267008;
pub const VK_DYNAMIC_STATE_DEPTH_BOUNDS_TEST_ENABLE: VkDynamicState = 1000267009;
pub const VK_DYNAMIC_STATE_STENCIL_TEST_ENABLE: VkDynamicState = 1000267010;
pub const VK_DYNAMIC_STATE_STENCIL_OP: VkDynamicState = 1000267011;
pub const VK_DYNAMIC_STATE_RASTERIZER_DISCARD_ENABLE: VkDynamicState = 1000377001;
pub const VK_DYNAMIC_STATE_DEPTH_BIAS_ENABLE: VkDynamicState = 1000377002;
pub const VK_DYNAMIC_STATE_PRIMITIVE_RESTART_ENABLE: VkDynamicState = 1000377004;
pub const VK_DYNAMIC_STATE_LINE_STIPPLE: VkDynamicState = 1000259000;
pub const VK_DYNAMIC_STATE_VIEWPORT_W_SCALING_NV: VkDynamicState = 1000087000;
pub const VK_DYNAMIC_STATE_DISCARD_RECTANGLE_EXT: VkDynamicState = 1000099000;
pub const VK_DYNAMIC_STATE_DISCARD_RECTANGLE_ENABLE_EXT: VkDynamicState = 1000099001;
pub const VK_DYNAMIC_STATE_DISCARD_RECTANGLE_MODE_EXT: VkDynamicState = 1000099002;
pub const VK_DYNAMIC_STATE_SAMPLE_LOCATIONS_EXT: VkDynamicState = 1000143000;
pub const VK_DYNAMIC_STATE_RAY_TRACING_PIPELINE_STACK_SIZE_KHR: VkDynamicState = 1000347000;
pub const VK_DYNAMIC_STATE_VIEWPORT_SHADING_RATE_PALETTE_NV: VkDynamicState = 1000164004;
pub const VK_DYNAMIC_STATE_VIEWPORT_COARSE_SAMPLE_ORDER_NV: VkDynamicState = 1000164006;
pub const VK_DYNAMIC_STATE_EXCLUSIVE_SCISSOR_ENABLE_NV: VkDynamicState = 1000205000;
pub const VK_DYNAMIC_STATE_EXCLUSIVE_SCISSOR_NV: VkDynamicState = 1000205001;
pub const VK_DYNAMIC_STATE_FRAGMENT_SHADING_RATE_KHR: VkDynamicState = 1000226000;
pub const VK_DYNAMIC_STATE_LINE_STIPPLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_LINE_STIPPLE;
pub const VK_DYNAMIC_STATE_CULL_MODE_EXT: VkDynamicState = VK_DYNAMIC_STATE_CULL_MODE;
pub const VK_DYNAMIC_STATE_FRONT_FACE_EXT: VkDynamicState = VK_DYNAMIC_STATE_FRONT_FACE;
pub const VK_DYNAMIC_STATE_PRIMITIVE_TOPOLOGY_EXT: VkDynamicState = VK_DYNAMIC_STATE_PRIMITIVE_TOPOLOGY;
pub const VK_DYNAMIC_STATE_VIEWPORT_WITH_COUNT_EXT: VkDynamicState = VK_DYNAMIC_STATE_VIEWPORT_WITH_COUNT;
pub const VK_DYNAMIC_STATE_SCISSOR_WITH_COUNT_EXT: VkDynamicState = VK_DYNAMIC_STATE_SCISSOR_WITH_COUNT;
pub const VK_DYNAMIC_STATE_VERTEX_INPUT_BINDING_STRIDE_EXT: VkDynamicState = VK_DYNAMIC_STATE_VERTEX_INPUT_BINDING_STRIDE;
pub const VK_DYNAMIC_STATE_DEPTH_TEST_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_DEPTH_TEST_ENABLE;
pub const VK_DYNAMIC_STATE_DEPTH_WRITE_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_DEPTH_WRITE_ENABLE;
pub const VK_DYNAMIC_STATE_DEPTH_COMPARE_OP_EXT: VkDynamicState = VK_DYNAMIC_STATE_DEPTH_COMPARE_OP;
pub const VK_DYNAMIC_STATE_DEPTH_BOUNDS_TEST_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_DEPTH_BOUNDS_TEST_ENABLE;
pub const VK_DYNAMIC_STATE_STENCIL_TEST_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_STENCIL_TEST_ENABLE;
pub const VK_DYNAMIC_STATE_STENCIL_OP_EXT: VkDynamicState = VK_DYNAMIC_STATE_STENCIL_OP;
pub const VK_DYNAMIC_STATE_VERTEX_INPUT_EXT: VkDynamicState = 1000352000;
pub const VK_DYNAMIC_STATE_PATCH_CONTROL_POINTS_EXT: VkDynamicState = 1000377000;
pub const VK_DYNAMIC_STATE_RASTERIZER_DISCARD_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_RASTERIZER_DISCARD_ENABLE;
pub const VK_DYNAMIC_STATE_DEPTH_BIAS_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_DEPTH_BIAS_ENABLE;
pub const VK_DYNAMIC_STATE_LOGIC_OP_EXT: VkDynamicState = 1000377003;
pub const VK_DYNAMIC_STATE_PRIMITIVE_RESTART_ENABLE_EXT: VkDynamicState = VK_DYNAMIC_STATE_PRIMITIVE_RESTART_ENABLE;
pub const VK_DYNAMIC_STATE_COLOR_WRITE_ENABLE_EXT: VkDynamicState = 1000381000;
pub const VK_DYNAMIC_STATE_DEPTH_CLAMP_ENABLE_EXT: VkDynamicState = 1000455003;
pub const VK_DYNAMIC_STATE_POLYGON_MODE_EXT: VkDynamicState = 1000455004;
pub const VK_DYNAMIC_STATE_RASTERIZATION_SAMPLES_EXT: VkDynamicState = 1000455005;
pub const VK_DYNAMIC_STATE_SAMPLE_MASK_EXT: VkDynamicState = 1000455006;
pub const VK_DYNAMIC_STATE_ALPHA_TO_COVERAGE_ENABLE_EXT: VkDynamicState = 1000455007;
pub const VK_DYNAMIC_STATE_ALPHA_TO_ONE_ENABLE_EXT: VkDynamicState = 1000455008;
pub const VK_DYNAMIC_STATE_LOGIC_OP_ENABLE_EXT: VkDynamicState = 1000455009;
pub const VK_DYNAMIC_STATE_COLOR_BLEND_ENABLE_EXT: VkDynamicState = 1000455010;
pub const VK_DYNAMIC_STATE_COLOR_BLEND_EQUATION_EXT: VkDynamicState = 1000455011;
pub const VK_DYNAMIC_STATE_COLOR_WRITE_MASK_EXT: VkDynamicState = 1000455012;
pub const VK_DYNAMIC_STATE_TESSELLATION_DOMAIN_ORIGIN_EXT: VkDynamicState = 1000455002;
pub const VK_DYNAMIC_STATE_RASTERIZATION_STREAM_EXT: VkDynamicState = 1000455013;
pub const VK_DYNAMIC_STATE_CONSERVATIVE_RASTERIZATION_MODE_EXT: VkDynamicState = 1000455014;
pub const VK_DYNAMIC_STATE_EXTRA_PRIMITIVE_OVERESTIMATION_SIZE_EXT: VkDynamicState = 1000455015;
pub const VK_DYNAMIC_STATE_DEPTH_CLIP_ENABLE_EXT: VkDynamicState = 1000455016;
pub const VK_DYNAMIC_STATE_SAMPLE_LOCATIONS_ENABLE_EXT: VkDynamicState = 1000455017;
pub const VK_DYNAMIC_STATE_COLOR_BLEND_ADVANCED_EXT: VkDynamicState = 1000455018;
pub const VK_DYNAMIC_STATE_PROVOKING_VERTEX_MODE_EXT: VkDynamicState = 1000455019;
pub const VK_DYNAMIC_STATE_LINE_RASTERIZATION_MODE_EXT: VkDynamicState = 1000455020;
pub const VK_DYNAMIC_STATE_LINE_STIPPLE_ENABLE_EXT: VkDynamicState = 1000455021;
pub const VK_DYNAMIC_STATE_DEPTH_CLIP_NEGATIVE_ONE_TO_ONE_EXT: VkDynamicState = 1000455022;
pub const VK_DYNAMIC_STATE_VIEWPORT_W_SCALING_ENABLE_NV: VkDynamicState = 1000455023;
pub const VK_DYNAMIC_STATE_VIEWPORT_SWIZZLE_NV: VkDynamicState = 1000455024;
pub const VK_DYNAMIC_STATE_COVERAGE_TO_COLOR_ENABLE_NV: VkDynamicState = 1000455025;
pub const VK_DYNAMIC_STATE_COVERAGE_TO_COLOR_LOCATION_NV: VkDynamicState = 1000455026;
pub const VK_DYNAMIC_STATE_COVERAGE_MODULATION_MODE_NV: VkDynamicState = 1000455027;
pub const VK_DYNAMIC_STATE_COVERAGE_MODULATION_TABLE_ENABLE_NV: VkDynamicState = 1000455028;
pub const VK_DYNAMIC_STATE_COVERAGE_MODULATION_TABLE_NV: VkDynamicState = 1000455029;
pub const VK_DYNAMIC_STATE_SHADING_RATE_IMAGE_ENABLE_NV: VkDynamicState = 1000455030;
pub const VK_DYNAMIC_STATE_REPRESENTATIVE_FRAGMENT_TEST_ENABLE_NV: VkDynamicState = 1000455031;
pub const VK_DYNAMIC_STATE_COVERAGE_REDUCTION_MODE_NV: VkDynamicState = 1000455032;
pub const VK_DYNAMIC_STATE_ATTACHMENT_FEEDBACK_LOOP_ENABLE_EXT: VkDynamicState = 1000524000;
pub const VK_DYNAMIC_STATE_LINE_STIPPLE_KHR: VkDynamicState = VK_DYNAMIC_STATE_LINE_STIPPLE;
pub const VK_DYNAMIC_STATE_DEPTH_CLAMP_RANGE_EXT: VkDynamicState = 1000582000;
pub type VkEvent = u64;
pub type VkEventCreateFlagBits = u32;
pub const VK_EVENT_CREATE_DEVICE_ONLY_BIT: VkEventCreateFlagBits = 0x1;
pub const VK_EVENT_CREATE_DEVICE_ONLY_BIT_KHR: VkEventCreateFlagBits = VK_EVENT_CREATE_DEVICE_ONLY_BIT;
pub type VkEventCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkEventCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkEventCreateFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkExtensionProperties {
    pub extensionName: [c_char; VK_MAX_EXTENSION_NAME_SIZE as usize],
    pub specVersion: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkExtent2D {
    pub width: u32,
    pub height: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}
pub type VkFence = u64;
pub type VkFenceCreateFlagBits = u32;
pub const VK_FENCE_CREATE_SIGNALED_BIT: VkFenceCreateFlagBits = 0x1;
pub type VkFenceCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkFenceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFenceCreateFlags,
}
pub type VkFilter = i32;
pub const VK_FILTER_NEAREST: VkFilter = 0;
pub const VK_FILTER_LINEAR: VkFilter = 1;
pub const VK_FILTER_CUBIC_IMG: VkFilter = VK_FILTER_CUBIC_EXT;
pub const VK_FILTER_CUBIC_EXT: VkFilter = 1000015000;
pub type VkFlags = u32;
pub type VkFlags64 = u64;
pub type VkFormat = i32;
pub const VK_FORMAT_UNDEFINED: VkFormat = 0;
pub const VK_FORMAT_R4G4_UNORM_PACK8: VkFormat = 1;
pub const VK_FORMAT_R4G4B4A4_UNORM_PACK16: VkFormat = 2;
pub const VK_FORMAT_B4G4R4A4_UNORM_PACK16: VkFormat = 3;
pub const VK_FORMAT_R5G6B5_UNORM_PACK16: VkFormat = 4;
pub const VK_FORMAT_B5G6R5_UNORM_PACK16: VkFormat = 5;
pub const VK_FORMAT_R5G5B5A1_UNORM_PACK16: VkFormat = 6;
pub const VK_FORMAT_B5G5R5A1_UNORM_PACK16: VkFormat = 7;
pub const VK_FORMAT_A1R5G5B5_UNORM_PACK16: VkFormat = 8;
pub const VK_FORMAT_R8_UNORM: VkFormat = 9;
pub const VK_FORMAT_R8_SNORM: VkFormat = 10;
pub const VK_FORMAT_R8_USCALED: VkFormat = 11;
pub const VK_FORMAT_R8_SSCALED: VkFormat = 12;
pub const VK_FORMAT_R8_UINT: VkFormat = 13;
pub const VK_FORMAT_R8_SINT: VkFormat = 14;
pub const VK_FORMAT_R8_SRGB: VkFormat = 15;
pub const VK_FORMAT_R8G8_UNORM: VkFormat = 16;
pub const VK_FORMAT_R8G8_SNORM: VkFormat = 17;
pub const VK_FORMAT_R8G8_USCALED: VkFormat = 18;
pub const VK_FORMAT_R8G8_SSCALED: VkFormat = 19;
pub const VK_FORMAT_R8G8_UINT: VkFormat = 20;
pub const VK_FORMAT_R8G8_SINT: VkFormat = 21;
pub const VK_FORMAT_R8G8_SRGB: VkFormat = 22;
pub const VK_FORMAT_R8G8B8_UNORM: VkFormat = 23;
pub const VK_FORMAT_R8G8B8_SNORM: VkFormat = 24;
pub const VK_FORMAT_R8G8B8_USCALED: VkFormat = 25;
pub const VK_FORMAT_R8G8B8_SSCALED: VkFormat = 26;
pub const VK_FORMAT_R8G8B8_UINT: VkFormat = 27;
pub const VK_FORMAT_R8G8B8_SINT: VkFormat = 28;
pub const VK_FORMAT_R8G8B8_SRGB: VkFormat = 29;
pub const VK_FORMAT_B8G8R8_UNORM: VkFormat = 30;
pub const VK_FORMAT_B8G8R8_SNORM: VkFormat = 31;
pub const VK_FORMAT_B8G8R8_USCALED: VkFormat = 32;
pub const VK_FORMAT_B8G8R8_SSCALED: VkFormat = 33;
pub const VK_FORMAT_B8G8R8_UINT: VkFormat = 34;
pub const VK_FORMAT_B8G8R8_SINT: VkFormat = 35;
pub const VK_FORMAT_B8G8R8_SRGB: VkFormat = 36;
pub const VK_FORMAT_R8G8B8A8_UNORM: VkFormat = 37;
pub const VK_FORMAT_R8G8B8A8_SNORM: VkFormat = 38;
pub const VK_FORMAT_R8G8B8A8_USCALED: VkFormat = 39;
pub const VK_FORMAT_R8G8B8A8_SSCALED: VkFormat = 40;
pub const VK_FORMAT_R8G8B8A8_UINT: VkFormat = 41;
pub const VK_FORMAT_R8G8B8A8_SINT: VkFormat = 42;
pub const VK_FORMAT_R8G8B8A8_SRGB: VkFormat = 43;
pub const VK_FORMAT_B8G8R8A8_UNORM: VkFormat = 44;
pub const VK_FORMAT_B8G8R8A8_SNORM: VkFormat = 45;
pub const VK_FORMAT_B8G8R8A8_USCALED: VkFormat = 46;
pub const VK_FORMAT_B8G8R8A8_SSCALED: VkFormat = 47;
pub const VK_FORMAT_B8G8R8A8_UINT: VkFormat = 48;
pub const VK_FORMAT_B8G8R8A8_SINT: VkFormat = 49;
pub const VK_FORMAT_B8G8R8A8_SRGB: VkFormat = 50;
pub const VK_FORMAT_A8B8G8R8_UNORM_PACK32: VkFormat = 51;
pub const VK_FORMAT_A8B8G8R8_SNORM_PACK32: VkFormat = 52;
pub const VK_FORMAT_A8B8G8R8_USCALED_PACK32: VkFormat = 53;
pub const VK_FORMAT_A8B8G8R8_SSCALED_PACK32: VkFormat = 54;
pub const VK_FORMAT_A8B8G8R8_UINT_PACK32: VkFormat = 55;
pub const VK_FORMAT_A8B8G8R8_SINT_PACK32: VkFormat = 56;
pub const VK_FORMAT_A8B8G8R8_SRGB_PACK32: VkFormat = 57;
pub const VK_FORMAT_A2R10G10B10_UNORM_PACK32: VkFormat = 58;
pub const VK_FORMAT_A2R10G10B10_SNORM_PACK32: VkFormat = 59;
pub const VK_FORMAT_A2R10G10B10_USCALED_PACK32: VkFormat = 60;
pub const VK_FORMAT_A2R10G10B10_SSCALED_PACK32: VkFormat = 61;
pub const VK_FORMAT_A2R10G10B10_UINT_PACK32: VkFormat = 62;
pub const VK_FORMAT_A2R10G10B10_SINT_PACK32: VkFormat = 63;
pub const VK_FORMAT_A2B10G10R10_UNORM_PACK32: VkFormat = 64;
pub const VK_FORMAT_A2B10G10R10_SNORM_PACK32: VkFormat = 65;
pub const VK_FORMAT_A2B10G10R10_USCALED_PACK32: VkFormat = 66;
pub const VK_FORMAT_A2B10G10R10_SSCALED_PACK32: VkFormat = 67;
pub const VK_FORMAT_A2B10G10R10_UINT_PACK32: VkFormat = 68;
pub const VK_FORMAT_A2B10G10R10_SINT_PACK32: VkFormat = 69;
pub const VK_FORMAT_R16_UNORM: VkFormat = 70;
pub const VK_FORMAT_R16_SNORM: VkFormat = 71;
pub const VK_FORMAT_R16_USCALED: VkFormat = 72;
pub const VK_FORMAT_R16_SSCALED: VkFormat = 73;
pub const VK_FORMAT_R16_UINT: VkFormat = 74;
pub const VK_FORMAT_R16_SINT: VkFormat = 75;
pub const VK_FORMAT_R16_SFLOAT: VkFormat = 76;
pub const VK_FORMAT_R16G16_UNORM: VkFormat = 77;
pub const VK_FORMAT_R16G16_SNORM: VkFormat = 78;
pub const VK_FORMAT_R16G16_USCALED: VkFormat = 79;
pub const VK_FORMAT_R16G16_SSCALED: VkFormat = 80;
pub const VK_FORMAT_R16G16_UINT: VkFormat = 81;
pub const VK_FORMAT_R16G16_SINT: VkFormat = 82;
pub const VK_FORMAT_R16G16_SFLOAT: VkFormat = 83;
pub const VK_FORMAT_R16G16B16_UNORM: VkFormat = 84;
pub const VK_FORMAT_R16G16B16_SNORM: VkFormat = 85;
pub const VK_FORMAT_R16G16B16_USCALED: VkFormat = 86;
pub const VK_FORMAT_R16G16B16_SSCALED: VkFormat = 87;
pub const VK_FORMAT_R16G16B16_UINT: VkFormat = 88;
pub const VK_FORMAT_R16G16B16_SINT: VkFormat = 89;
pub const VK_FORMAT_R16G16B16_SFLOAT: VkFormat = 90;
pub const VK_FORMAT_R16G16B16A16_UNORM: VkFormat = 91;
pub const VK_FORMAT_R16G16B16A16_SNORM: VkFormat = 92;
pub const VK_FORMAT_R16G16B16A16_USCALED: VkFormat = 93;
pub const VK_FORMAT_R16G16B16A16_SSCALED: VkFormat = 94;
pub const VK_FORMAT_R16G16B16A16_UINT: VkFormat = 95;
pub const VK_FORMAT_R16G16B16A16_SINT: VkFormat = 96;
pub const VK_FORMAT_R16G16B16A16_SFLOAT: VkFormat = 97;
pub const VK_FORMAT_R32_UINT: VkFormat = 98;
pub const VK_FORMAT_R32_SINT: VkFormat = 99;
pub const VK_FORMAT_R32_SFLOAT: VkFormat = 100;
pub const VK_FORMAT_R32G32_UINT: VkFormat = 101;
pub const VK_FORMAT_R32G32_SINT: VkFormat = 102;
pub const VK_FORMAT_R32G32_SFLOAT: VkFormat = 103;
pub const VK_FORMAT_R32G32B32_UINT: VkFormat = 104;
pub const VK_FORMAT_R32G32B32_SINT: VkFormat = 105;
pub const VK_FORMAT_R32G32B32_SFLOAT: VkFormat = 106;
pub const VK_FORMAT_R32G32B32A32_UINT: VkFormat = 107;
pub const VK_FORMAT_R32G32B32A32_SINT: VkFormat = 108;
pub const VK_FORMAT_R32G32B32A32_SFLOAT: VkFormat = 109;
pub const VK_FORMAT_R64_UINT: VkFormat = 110;
pub const VK_FORMAT_R64_SINT: VkFormat = 111;
pub const VK_FORMAT_R64_SFLOAT: VkFormat = 112;
pub const VK_FORMAT_R64G64_UINT: VkFormat = 113;
pub const VK_FORMAT_R64G64_SINT: VkFormat = 114;
pub const VK_FORMAT_R64G64_SFLOAT: VkFormat = 115;
pub const VK_FORMAT_R64G64B64_UINT: VkFormat = 116;
pub const VK_FORMAT_R64G64B64_SINT: VkFormat = 117;
pub const VK_FORMAT_R64G64B64_SFLOAT: VkFormat = 118;
pub const VK_FORMAT_R64G64B64A64_UINT: VkFormat = 119;
pub const VK_FORMAT_R64G64B64A64_SINT: VkFormat = 120;
pub const VK_FORMAT_R64G64B64A64_SFLOAT: VkFormat = 121;
pub const VK_FORMAT_B10G11R11_UFLOAT_PACK32: VkFormat = 122;
pub const VK_FORMAT_E5B9G9R9_UFLOAT_PACK32: VkFormat = 123;
pub const VK_FORMAT_D16_UNORM: VkFormat = 124;
pub const VK_FORMAT_X8_D24_UNORM_PACK32: VkFormat = 125;
pub const VK_FORMAT_D32_SFLOAT: VkFormat = 126;
pub const VK_FORMAT_S8_UINT: VkFormat = 127;
pub const VK_FORMAT_D16_UNORM_S8_UINT: VkFormat = 128;
pub const VK_FORMAT_D24_UNORM_S8_UINT: VkFormat = 129;
pub const VK_FORMAT_D32_SFLOAT_S8_UINT: VkFormat = 130;
pub const VK_FORMAT_BC1_RGB_UNORM_BLOCK: VkFormat = 131;
pub const VK_FORMAT_BC1_RGB_SRGB_BLOCK: VkFormat = 132;
pub const VK_FORMAT_BC1_RGBA_UNORM_BLOCK: VkFormat = 133;
pub const VK_FORMAT_BC1_RGBA_SRGB_BLOCK: VkFormat = 134;
pub const VK_FORMAT_BC2_UNORM_BLOCK: VkFormat = 135;
pub const VK_FORMAT_BC2_SRGB_BLOCK: VkFormat = 136;
pub const VK_FORMAT_BC3_UNORM_BLOCK: VkFormat = 137;
pub const VK_FORMAT_BC3_SRGB_BLOCK: VkFormat = 138;
pub const VK_FORMAT_BC4_UNORM_BLOCK: VkFormat = 139;
pub const VK_FORMAT_BC4_SNORM_BLOCK: VkFormat = 140;
pub const VK_FORMAT_BC5_UNORM_BLOCK: VkFormat = 141;
pub const VK_FORMAT_BC5_SNORM_BLOCK: VkFormat = 142;
pub const VK_FORMAT_BC6H_UFLOAT_BLOCK: VkFormat = 143;
pub const VK_FORMAT_BC6H_SFLOAT_BLOCK: VkFormat = 144;
pub const VK_FORMAT_BC7_UNORM_BLOCK: VkFormat = 145;
pub const VK_FORMAT_BC7_SRGB_BLOCK: VkFormat = 146;
pub const VK_FORMAT_ETC2_R8G8B8_UNORM_BLOCK: VkFormat = 147;
pub const VK_FORMAT_ETC2_R8G8B8_SRGB_BLOCK: VkFormat = 148;
pub const VK_FORMAT_ETC2_R8G8B8A1_UNORM_BLOCK: VkFormat = 149;
pub const VK_FORMAT_ETC2_R8G8B8A1_SRGB_BLOCK: VkFormat = 150;
pub const VK_FORMAT_ETC2_R8G8B8A8_UNORM_BLOCK: VkFormat = 151;
pub const VK_FORMAT_ETC2_R8G8B8A8_SRGB_BLOCK: VkFormat = 152;
pub const VK_FORMAT_EAC_R11_UNORM_BLOCK: VkFormat = 153;
pub const VK_FORMAT_EAC_R11_SNORM_BLOCK: VkFormat = 154;
pub const VK_FORMAT_EAC_R11G11_UNORM_BLOCK: VkFormat = 155;
pub const VK_FORMAT_EAC_R11G11_SNORM_BLOCK: VkFormat = 156;
pub const VK_FORMAT_ASTC_4x4_UNORM_BLOCK: VkFormat = 157;
pub const VK_FORMAT_ASTC_4x4_SRGB_BLOCK: VkFormat = 158;
pub const VK_FORMAT_ASTC_5x4_UNORM_BLOCK: VkFormat = 159;
pub const VK_FORMAT_ASTC_5x4_SRGB_BLOCK: VkFormat = 160;
pub const VK_FORMAT_ASTC_5x5_UNORM_BLOCK: VkFormat = 161;
pub const VK_FORMAT_ASTC_5x5_SRGB_BLOCK: VkFormat = 162;
pub const VK_FORMAT_ASTC_6x5_UNORM_BLOCK: VkFormat = 163;
pub const VK_FORMAT_ASTC_6x5_SRGB_BLOCK: VkFormat = 164;
pub const VK_FORMAT_ASTC_6x6_UNORM_BLOCK: VkFormat = 165;
pub const VK_FORMAT_ASTC_6x6_SRGB_BLOCK: VkFormat = 166;
pub const VK_FORMAT_ASTC_8x5_UNORM_BLOCK: VkFormat = 167;
pub const VK_FORMAT_ASTC_8x5_SRGB_BLOCK: VkFormat = 168;
pub const VK_FORMAT_ASTC_8x6_UNORM_BLOCK: VkFormat = 169;
pub const VK_FORMAT_ASTC_8x6_SRGB_BLOCK: VkFormat = 170;
pub const VK_FORMAT_ASTC_8x8_UNORM_BLOCK: VkFormat = 171;
pub const VK_FORMAT_ASTC_8x8_SRGB_BLOCK: VkFormat = 172;
pub const VK_FORMAT_ASTC_10x5_UNORM_BLOCK: VkFormat = 173;
pub const VK_FORMAT_ASTC_10x5_SRGB_BLOCK: VkFormat = 174;
pub const VK_FORMAT_ASTC_10x6_UNORM_BLOCK: VkFormat = 175;
pub const VK_FORMAT_ASTC_10x6_SRGB_BLOCK: VkFormat = 176;
pub const VK_FORMAT_ASTC_10x8_UNORM_BLOCK: VkFormat = 177;
pub const VK_FORMAT_ASTC_10x8_SRGB_BLOCK: VkFormat = 178;
pub const VK_FORMAT_ASTC_10x10_UNORM_BLOCK: VkFormat = 179;
pub const VK_FORMAT_ASTC_10x10_SRGB_BLOCK: VkFormat = 180;
pub const VK_FORMAT_ASTC_12x10_UNORM_BLOCK: VkFormat = 181;
pub const VK_FORMAT_ASTC_12x10_SRGB_BLOCK: VkFormat = 182;
pub const VK_FORMAT_ASTC_12x12_UNORM_BLOCK: VkFormat = 183;
pub const VK_FORMAT_ASTC_12x12_SRGB_BLOCK: VkFormat = 184;
pub const VK_FORMAT_G8B8G8R8_422_UNORM: VkFormat = 1000156000;
pub const VK_FORMAT_B8G8R8G8_422_UNORM: VkFormat = 1000156001;
pub const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM: VkFormat = 1000156002;
pub const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM: VkFormat = 1000156003;
pub const VK_FORMAT_G8_B8_R8_3PLANE_422_UNORM: VkFormat = 1000156004;
pub const VK_FORMAT_G8_B8R8_2PLANE_422_UNORM: VkFormat = 1000156005;
pub const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM: VkFormat = 1000156006;
pub const VK_FORMAT_R10X6_UNORM_PACK16: VkFormat = 1000156007;
pub const VK_FORMAT_R10X6G10X6_UNORM_2PACK16: VkFormat = 1000156008;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UNORM_4PACK16: VkFormat = 1000156009;
pub const VK_FORMAT_G10X6B10X6G10X6R10X6_422_UNORM_4PACK16: VkFormat = 1000156010;
pub const VK_FORMAT_B10X6G10X6R10X6G10X6_422_UNORM_4PACK16: VkFormat = 1000156011;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_420_UNORM_3PACK16: VkFormat = 1000156012;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156013;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_422_UNORM_3PACK16: VkFormat = 1000156014;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_422_UNORM_3PACK16: VkFormat = 1000156015;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_444_UNORM_3PACK16: VkFormat = 1000156016;
pub const VK_FORMAT_R12X4_UNORM_PACK16: VkFormat = 1000156017;
pub const VK_FORMAT_R12X4G12X4_UNORM_2PACK16: VkFormat = 1000156018;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UNORM_4PACK16: VkFormat = 1000156019;
pub const VK_FORMAT_G12X4B12X4G12X4R12X4_422_UNORM_4PACK16: VkFormat = 1000156020;
pub const VK_FORMAT_B12X4G12X4R12X4G12X4_422_UNORM_4PACK16: VkFormat = 1000156021;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_420_UNORM_3PACK16: VkFormat = 1000156022;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16: VkFormat = 1000156023;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_422_UNORM_3PACK16: VkFormat = 1000156024;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_422_UNORM_3PACK16: VkFormat = 1000156025;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_444_UNORM_3PACK16: VkFormat = 1000156026;
pub const VK_FORMAT_G16B16G16R16_422_UNORM: VkFormat = 1000156027;
pub const VK_FORMAT_B16G16R16G16_422_UNORM: VkFormat = 1000156028;
pub const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM: VkFormat = 1000156029;
pub const VK_FORMAT_G16_B16R16_2PLANE_420_UNORM: VkFormat = 1000156030;
pub const VK_FORMAT_G16_B16_R16_3PLANE_422_UNORM: VkFormat = 1000156031;
pub const VK_FORMAT_G16_B16R16_2PLANE_422_UNORM: VkFormat = 1000156032;
pub const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM: VkFormat = 1000156033;
pub const VK_FORMAT_G8_B8R8_2PLANE_444_UNORM: VkFormat = 1000330000;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_444_UNORM_3PACK16: VkFormat = 1000330001;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_444_UNORM_3PACK16: VkFormat = 1000330002;
pub const VK_FORMAT_G16_B16R16_2PLANE_444_UNORM: VkFormat = 1000330003;
pub const VK_FORMAT_A4R4G4B4_UNORM_PACK16: VkFormat = 1000340000;
pub const VK_FORMAT_A4B4G4R4_UNORM_PACK16: VkFormat = 1000340001;
pub const VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK: VkFormat = 1000066000;
pub const VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK: VkFormat = 1000066001;
pub const VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK: VkFormat = 1000066002;
pub const VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK: VkFormat = 1000066003;
pub const VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK: VkFormat = 1000066004;
pub const VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK: VkFormat = 1000066005;
pub const VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK: VkFormat = 1000066006;
pub const VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK: VkFormat = 1000066007;
pub const VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK: VkFormat = 1000066008;
pub const VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK: VkFormat = 1000066009;
pub const VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK: VkFormat = 1000066010;
pub const VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK: VkFormat = 1000066011;
pub const VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK: VkFormat = 1000066012;
pub const VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK: VkFormat = 1000066013;
pub const VK_FORMAT_A1B5G5R5_UNORM_PACK16: VkFormat = 1000470000;
pub const VK_FORMAT_A8_UNORM: VkFormat = 1000470001;
pub const VK_FORMAT_PVRTC1_2BPP_UNORM_BLOCK_IMG: VkFormat = 1000054000;
pub const VK_FORMAT_PVRTC1_4BPP_UNORM_BLOCK_IMG: VkFormat = 1000054001;
pub const VK_FORMAT_PVRTC2_2BPP_UNORM_BLOCK_IMG: VkFormat = 1000054002;
pub const VK_FORMAT_PVRTC2_4BPP_UNORM_BLOCK_IMG: VkFormat = 1000054003;
pub const VK_FORMAT_PVRTC1_2BPP_SRGB_BLOCK_IMG: VkFormat = 1000054004;
pub const VK_FORMAT_PVRTC1_4BPP_SRGB_BLOCK_IMG: VkFormat = 1000054005;
pub const VK_FORMAT_PVRTC2_2BPP_SRGB_BLOCK_IMG: VkFormat = 1000054006;
pub const VK_FORMAT_PVRTC2_4BPP_SRGB_BLOCK_IMG: VkFormat = 1000054007;
pub const VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_4x4_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_5x4_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_5x5_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_6x5_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_6x6_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_8x5_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_8x6_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_8x8_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_10x5_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_10x6_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_10x8_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_10x10_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_12x10_SFLOAT_BLOCK;
pub const VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK_EXT: VkFormat = VK_FORMAT_ASTC_12x12_SFLOAT_BLOCK;
pub const VK_FORMAT_G8B8G8R8_422_UNORM_KHR: VkFormat = VK_FORMAT_G8B8G8R8_422_UNORM;
pub const VK_FORMAT_B8G8R8G8_422_UNORM_KHR: VkFormat = VK_FORMAT_B8G8R8G8_422_UNORM;
pub const VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM_KHR: VkFormat = VK_FORMAT_G8_B8_R8_3PLANE_420_UNORM;
pub const VK_FORMAT_G8_B8R8_2PLANE_420_UNORM_KHR: VkFormat = VK_FORMAT_G8_B8R8_2PLANE_420_UNORM;
pub const VK_FORMAT_G8_B8_R8_3PLANE_422_UNORM_KHR: VkFormat = VK_FORMAT_G8_B8_R8_3PLANE_422_UNORM;
pub const VK_FORMAT_G8_B8R8_2PLANE_422_UNORM_KHR: VkFormat = VK_FORMAT_G8_B8R8_2PLANE_422_UNORM;
pub const VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM_KHR: VkFormat = VK_FORMAT_G8_B8_R8_3PLANE_444_UNORM;
pub const VK_FORMAT_R10X6_UNORM_PACK16_KHR: VkFormat = VK_FORMAT_R10X6_UNORM_PACK16;
pub const VK_FORMAT_R10X6G10X6_UNORM_2PACK16_KHR: VkFormat = VK_FORMAT_R10X6G10X6_UNORM_2PACK16;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_R10X6G10X6B10X6A10X6_UNORM_4PACK16;
pub const VK_FORMAT_G10X6B10X6G10X6R10X6_422_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_G10X6B10X6G10X6R10X6_422_UNORM_4PACK16;
pub const VK_FORMAT_B10X6G10X6R10X6G10X6_422_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_B10X6G10X6R10X6G10X6_422_UNORM_4PACK16;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_420_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_420_UNORM_3PACK16;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_422_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_422_UNORM_3PACK16;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_422_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G10X6_B10X6R10X6_2PLANE_422_UNORM_3PACK16;
pub const VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_444_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G10X6_B10X6_R10X6_3PLANE_444_UNORM_3PACK16;
pub const VK_FORMAT_R12X4_UNORM_PACK16_KHR: VkFormat = VK_FORMAT_R12X4_UNORM_PACK16;
pub const VK_FORMAT_R12X4G12X4_UNORM_2PACK16_KHR: VkFormat = VK_FORMAT_R12X4G12X4_UNORM_2PACK16;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_R12X4G12X4B12X4A12X4_UNORM_4PACK16;
pub const VK_FORMAT_G12X4B12X4G12X4R12X4_422_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_G12X4B12X4G12X4R12X4_422_UNORM_4PACK16;
pub const VK_FORMAT_B12X4G12X4R12X4G12X4_422_UNORM_4PACK16_KHR: VkFormat = VK_FORMAT_B12X4G12X4R12X4G12X4_422_UNORM_4PACK16;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_420_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_420_UNORM_3PACK16;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G12X4_B12X4R12X4_2PLANE_420_UNORM_3PACK16;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_422_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_422_UNORM_3PACK16;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_422_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G12X4_B12X4R12X4_2PLANE_422_UNORM_3PACK16;
pub const VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_444_UNORM_3PACK16_KHR: VkFormat = VK_FORMAT_G12X4_B12X4_R12X4_3PLANE_444_UNORM_3PACK16;
pub const VK_FORMAT_G16B16G16R16_422_UNORM_KHR: VkFormat = VK_FORMAT_G16B16G16R16_422_UNORM;
pub const VK_FORMAT_B16G16R16G16_422_UNORM_KHR: VkFormat = VK_FORMAT_B16G16R16G16_422_UNORM;
pub const VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM_KHR: VkFormat = VK_FORMAT_G16_B16_R16_3PLANE_420_UNORM;
pub const VK_FORMAT_G16_B16R16_2PLANE_420_UNORM_KHR: VkFormat = VK_FORMAT_G16_B16R16_2PLANE_420_UNORM;
pub const VK_FORMAT_G16_B16_R16_3PLANE_422_UNORM_KHR: VkFormat = VK_FORMAT_G16_B16_R16_3PLANE_422_UNORM;
pub const VK_FORMAT_G16_B16R16_2PLANE_422_UNORM_KHR: VkFormat = VK_FORMAT_G16_B16R16_2PLANE_422_UNORM;
pub const VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM_KHR: VkFormat = VK_FORMAT_G16_B16_R16_3PLANE_444_UNORM;
pub const VK_FORMAT_ASTC_3x3x3_UNORM_BLOCK_EXT: VkFormat = 1000288000;
pub const VK_FORMAT_ASTC_3x3x3_SRGB_BLOCK_EXT: VkFormat = 1000288001;
pub const VK_FORMAT_ASTC_3x3x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288002;
pub const VK_FORMAT_ASTC_4x3x3_UNORM_BLOCK_EXT: VkFormat = 1000288003;
pub const VK_FORMAT_ASTC_4x3x3_SRGB_BLOCK_EXT: VkFormat = 1000288004;
pub const VK_FORMAT_ASTC_4x3x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288005;
pub const VK_FORMAT_ASTC_4x4x3_UNORM_BLOCK_EXT: VkFormat = 1000288006;
pub const VK_FORMAT_ASTC_4x4x3_SRGB_BLOCK_EXT: VkFormat = 1000288007;
pub const VK_FORMAT_ASTC_4x4x3_SFLOAT_BLOCK_EXT: VkFormat = 1000288008;
pub const VK_FORMAT_ASTC_4x4x4_UNORM_BLOCK_EXT: VkFormat = 1000288009;
pub const VK_FORMAT_ASTC_4x4x4_SRGB_BLOCK_EXT: VkFormat = 1000288010;
pub const VK_FORMAT_ASTC_4x4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288011;
pub const VK_FORMAT_ASTC_5x4x4_UNORM_BLOCK_EXT: VkFormat = 1000288012;
pub const VK_FORMAT_ASTC_5x4x4_SRGB_BLOCK_EXT: VkFormat = 1000288013;
pub const VK_FORMAT_ASTC_5x4x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288014;
pub const VK_FORMAT_ASTC_5x5x4_UNORM_BLOCK_EXT: VkFormat = 1000288015;
pub const VK_FORMAT_ASTC_5x5x4_SRGB_BLOCK_EXT: VkFormat = 1000288016;
pub const VK_FORMAT_ASTC_5x5x4_SFLOAT_BLOCK_EXT: VkFormat = 1000288017;
pub const VK_FORMAT_ASTC_5x5x5_UNORM_BLOCK_EXT: VkFormat = 1000288018;
pub const VK_FORMAT_ASTC_5x5x5_SRGB_BLOCK_EXT: VkFormat = 1000288019;
pub const VK_FORMAT_ASTC_5x5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288020;
pub const VK_FORMAT_ASTC_6x5x5_UNORM_BLOCK_EXT: VkFormat = 1000288021;
pub const VK_FORMAT_ASTC_6x5x5_SRGB_BLOCK_EXT: VkFormat = 1000288022;
pub const VK_FORMAT_ASTC_6x5x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288023;
pub const VK_FORMAT_ASTC_6x6x5_UNORM_BLOCK_EXT: VkFormat = 1000288024;
pub const VK_FORMAT_ASTC_6x6x5_SRGB_BLOCK_EXT: VkFormat = 1000288025;
pub const VK_FORMAT_ASTC_6x6x5_SFLOAT_BLOCK_EXT: VkFormat = 1000288026;
pub const VK_FORMAT_ASTC_6x6x6_UNORM_BLOCK_EXT: VkFormat = 1000288027;
pub const VK_FORMAT_ASTC_6x6x6_SRGB_BLOCK_EXT: VkFormat = 1000288028;
pub const VK_FORMAT_ASTC_6x6x6_SFLOAT_BLOCK_EXT: VkFormat = 1000288029;
pub const VK_FORMAT_G8_B8R8_2PLANE_444_UNORM_EXT: VkFormat = VK_FORMAT_G8_B8R8_2PLANE_444_UNORM;
pub const VK_FORMAT_G10X6_B10X6R10X6_2PLANE_444_UNORM_3PACK16_EXT: VkFormat = VK_FORMAT_G10X6_B10X6R10X6_2PLANE_444_UNORM_3PACK16;
pub const VK_FORMAT_G12X4_B12X4R12X4_2PLANE_444_UNORM_3PACK16_EXT: VkFormat = VK_FORMAT_G12X4_B12X4R12X4_2PLANE_444_UNORM_3PACK16;
pub const VK_FORMAT_G16_B16R16_2PLANE_444_UNORM_EXT: VkFormat = VK_FORMAT_G16_B16R16_2PLANE_444_UNORM;
pub const VK_FORMAT_A4R4G4B4_UNORM_PACK16_EXT: VkFormat = VK_FORMAT_A4R4G4B4_UNORM_PACK16;
pub const VK_FORMAT_A4B4G4R4_UNORM_PACK16_EXT: VkFormat = VK_FORMAT_A4B4G4R4_UNORM_PACK16;
pub const VK_FORMAT_R8_BOOL_ARM: VkFormat = 1000460000;
pub const VK_FORMAT_R16G16_SFIXED5_NV: VkFormat = 1000464000;
pub const VK_FORMAT_R16G16_S10_5_NV: VkFormat = VK_FORMAT_R16G16_SFIXED5_NV;
pub const VK_FORMAT_A1B5G5R5_UNORM_PACK16_KHR: VkFormat = VK_FORMAT_A1B5G5R5_UNORM_PACK16;
pub const VK_FORMAT_A8_UNORM_KHR: VkFormat = VK_FORMAT_A8_UNORM;
pub const VK_FORMAT_R10X6_UINT_PACK16_ARM: VkFormat = 1000609000;
pub const VK_FORMAT_R10X6G10X6_UINT_2PACK16_ARM: VkFormat = 1000609001;
pub const VK_FORMAT_R10X6G10X6B10X6A10X6_UINT_4PACK16_ARM: VkFormat = 1000609002;
pub const VK_FORMAT_R12X4_UINT_PACK16_ARM: VkFormat = 1000609003;
pub const VK_FORMAT_R12X4G12X4_UINT_2PACK16_ARM: VkFormat = 1000609004;
pub const VK_FORMAT_R12X4G12X4B12X4A12X4_UINT_4PACK16_ARM: VkFormat = 1000609005;
pub const VK_FORMAT_R14X2_UINT_PACK16_ARM: VkFormat = 1000609006;
pub const VK_FORMAT_R14X2G14X2_UINT_2PACK16_ARM: VkFormat = 1000609007;
pub const VK_FORMAT_R14X2G14X2B14X2A14X2_UINT_4PACK16_ARM: VkFormat = 1000609008;
pub const VK_FORMAT_R14X2_UNORM_PACK16_ARM: VkFormat = 1000609009;
pub const VK_FORMAT_R14X2G14X2_UNORM_2PACK16_ARM: VkFormat = 1000609010;
pub const VK_FORMAT_R14X2G14X2B14X2A14X2_UNORM_4PACK16_ARM: VkFormat = 1000609011;
pub const VK_FORMAT_G14X2_B14X2R14X2_2PLANE_420_UNORM_3PACK16_ARM: VkFormat = 1000609012;
pub const VK_FORMAT_G14X2_B14X2R14X2_2PLANE_422_UNORM_3PACK16_ARM: VkFormat = 1000609013;
pub type VkFormatFeatureFlagBits = u32;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT: VkFormatFeatureFlagBits = 0x1;
pub const VK_FORMAT_FEATURE_STORAGE_IMAGE_BIT: VkFormatFeatureFlagBits = 0x2;
pub const VK_FORMAT_FEATURE_STORAGE_IMAGE_ATOMIC_BIT: VkFormatFeatureFlagBits = 0x4;
pub const VK_FORMAT_FEATURE_UNIFORM_TEXEL_BUFFER_BIT: VkFormatFeatureFlagBits = 0x8;
pub const VK_FORMAT_FEATURE_STORAGE_TEXEL_BUFFER_BIT: VkFormatFeatureFlagBits = 0x10;
pub const VK_FORMAT_FEATURE_STORAGE_TEXEL_BUFFER_ATOMIC_BIT: VkFormatFeatureFlagBits = 0x20;
pub const VK_FORMAT_FEATURE_VERTEX_BUFFER_BIT: VkFormatFeatureFlagBits = 0x40;
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT: VkFormatFeatureFlagBits = 0x80;
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BLEND_BIT: VkFormatFeatureFlagBits = 0x100;
pub const VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT: VkFormatFeatureFlagBits = 0x200;
pub const VK_FORMAT_FEATURE_BLIT_SRC_BIT: VkFormatFeatureFlagBits = 0x400;
pub const VK_FORMAT_FEATURE_BLIT_DST_BIT: VkFormatFeatureFlagBits = 0x800;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_LINEAR_BIT: VkFormatFeatureFlagBits = 0x1000;
pub const VK_FORMAT_FEATURE_TRANSFER_SRC_BIT: VkFormatFeatureFlagBits = 0x4000;
pub const VK_FORMAT_FEATURE_TRANSFER_DST_BIT: VkFormatFeatureFlagBits = 0x8000;
pub const VK_FORMAT_FEATURE_MIDPOINT_CHROMA_SAMPLES_BIT: VkFormatFeatureFlagBits = 0x20000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER_BIT: VkFormatFeatureFlagBits = 0x40000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_SEPARATE_RECONSTRUCTION_FILTER_BIT: VkFormatFeatureFlagBits = 0x80000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_BIT: VkFormatFeatureFlagBits = 0x100000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_FORCEABLE_BIT: VkFormatFeatureFlagBits = 0x200000;
pub const VK_FORMAT_FEATURE_DISJOINT_BIT: VkFormatFeatureFlagBits = 0x400000;
pub const VK_FORMAT_FEATURE_COSITED_CHROMA_SAMPLES_BIT: VkFormatFeatureFlagBits = 0x800000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_MINMAX_BIT: VkFormatFeatureFlagBits = 0x10000;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_CUBIC_BIT_IMG: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_CUBIC_BIT_EXT;
pub const VK_FORMAT_FEATURE_VIDEO_DECODE_OUTPUT_BIT_KHR: VkFormatFeatureFlagBits = 0x2000000;
pub const VK_FORMAT_FEATURE_VIDEO_DECODE_DPB_BIT_KHR: VkFormatFeatureFlagBits = 0x4000000;
pub const VK_FORMAT_FEATURE_TRANSFER_SRC_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_TRANSFER_SRC_BIT;
pub const VK_FORMAT_FEATURE_TRANSFER_DST_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_TRANSFER_DST_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_MINMAX_BIT_EXT: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_MINMAX_BIT;
pub const VK_FORMAT_FEATURE_ACCELERATION_STRUCTURE_VERTEX_BUFFER_BIT_KHR: VkFormatFeatureFlagBits = 0x20000000;
pub const VK_FORMAT_FEATURE_MIDPOINT_CHROMA_SAMPLES_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_MIDPOINT_CHROMA_SAMPLES_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_LINEAR_FILTER_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_SEPARATE_RECONSTRUCTION_FILTER_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_SEPARATE_RECONSTRUCTION_FILTER_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_FORCEABLE_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_SAMPLED_IMAGE_YCBCR_CONVERSION_CHROMA_RECONSTRUCTION_EXPLICIT_FORCEABLE_BIT;
pub const VK_FORMAT_FEATURE_DISJOINT_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_DISJOINT_BIT;
pub const VK_FORMAT_FEATURE_COSITED_CHROMA_SAMPLES_BIT_KHR: VkFormatFeatureFlagBits = VK_FORMAT_FEATURE_COSITED_CHROMA_SAMPLES_BIT;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_FILTER_CUBIC_BIT_EXT: VkFormatFeatureFlagBits = 0x2000;
pub const VK_FORMAT_FEATURE_FRAGMENT_DENSITY_MAP_BIT_EXT: VkFormatFeatureFlagBits = 0x1000000;
pub const VK_FORMAT_FEATURE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkFormatFeatureFlagBits = 0x40000000;
pub const VK_FORMAT_FEATURE_VIDEO_ENCODE_INPUT_BIT_KHR: VkFormatFeatureFlagBits = 0x8000000;
pub const VK_FORMAT_FEATURE_VIDEO_ENCODE_DPB_BIT_KHR: VkFormatFeatureFlagBits = 0x10000000;
pub type VkFormatFeatureFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkFormatProperties {
    pub linearTilingFeatures: VkFormatFeatureFlags,
    pub optimalTilingFeatures: VkFormatFeatureFlags,
    pub bufferFeatures: VkFormatFeatureFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkFormatProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub formatProperties: VkFormatProperties,
}
pub type VkFramebuffer = u64;
pub type VkFramebufferCreateFlagBits = u32;
pub const VK_FRAMEBUFFER_CREATE_IMAGELESS_BIT: VkFramebufferCreateFlagBits = 0x1;
pub const VK_FRAMEBUFFER_CREATE_IMAGELESS_BIT_KHR: VkFramebufferCreateFlagBits = VK_FRAMEBUFFER_CREATE_IMAGELESS_BIT;
pub type VkFramebufferCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkFramebufferCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFramebufferCreateFlags,
    pub renderPass: VkRenderPass,
    pub attachmentCount: u32,
    pub pAttachments: *const VkImageView,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
}
pub type VkFrontFace = i32;
pub const VK_FRONT_FACE_COUNTER_CLOCKWISE: VkFrontFace = 0;
pub const VK_FRONT_FACE_CLOCKWISE: VkFrontFace = 1;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkGraphicsPipelineCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineCreateFlags,
    pub stageCount: u32,
    pub pStages: *const VkPipelineShaderStageCreateInfo,
    pub pVertexInputState: *const VkPipelineVertexInputStateCreateInfo,
    pub pInputAssemblyState: *const VkPipelineInputAssemblyStateCreateInfo,
    pub pTessellationState: *const VkPipelineTessellationStateCreateInfo,
    pub pViewportState: *const VkPipelineViewportStateCreateInfo,
    pub pRasterizationState: *const VkPipelineRasterizationStateCreateInfo,
    pub pMultisampleState: *const VkPipelineMultisampleStateCreateInfo,
    pub pDepthStencilState: *const VkPipelineDepthStencilStateCreateInfo,
    pub pColorBlendState: *const VkPipelineColorBlendStateCreateInfo,
    pub pDynamicState: *const VkPipelineDynamicStateCreateInfo,
    pub layout: VkPipelineLayout,
    pub renderPass: VkRenderPass,
    pub subpass: u32,
    pub basePipelineHandle: VkPipeline,
    pub basePipelineIndex: i32,
}
pub type VkImage = u64;
pub type VkImageAspectFlagBits = u32;
pub const VK_IMAGE_ASPECT_COLOR_BIT: VkImageAspectFlagBits = 0x1;
pub const VK_IMAGE_ASPECT_DEPTH_BIT: VkImageAspectFlagBits = 0x2;
pub const VK_IMAGE_ASPECT_STENCIL_BIT: VkImageAspectFlagBits = 0x4;
pub const VK_IMAGE_ASPECT_METADATA_BIT: VkImageAspectFlagBits = 0x8;
pub const VK_IMAGE_ASPECT_PLANE_0_BIT: VkImageAspectFlagBits = 0x10;
pub const VK_IMAGE_ASPECT_PLANE_1_BIT: VkImageAspectFlagBits = 0x20;
pub const VK_IMAGE_ASPECT_PLANE_2_BIT: VkImageAspectFlagBits = 0x40;
pub const VK_IMAGE_ASPECT_NONE: VkImageAspectFlagBits = 0x0;
pub const VK_IMAGE_ASPECT_PLANE_0_BIT_KHR: VkImageAspectFlagBits = VK_IMAGE_ASPECT_PLANE_0_BIT;
pub const VK_IMAGE_ASPECT_PLANE_1_BIT_KHR: VkImageAspectFlagBits = VK_IMAGE_ASPECT_PLANE_1_BIT;
pub const VK_IMAGE_ASPECT_PLANE_2_BIT_KHR: VkImageAspectFlagBits = VK_IMAGE_ASPECT_PLANE_2_BIT;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_0_BIT_EXT: VkImageAspectFlagBits = 0x80;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_1_BIT_EXT: VkImageAspectFlagBits = 0x100;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_2_BIT_EXT: VkImageAspectFlagBits = 0x200;
pub const VK_IMAGE_ASPECT_MEMORY_PLANE_3_BIT_EXT: VkImageAspectFlagBits = 0x400;
pub const VK_IMAGE_ASPECT_NONE_KHR: VkImageAspectFlagBits = VK_IMAGE_ASPECT_NONE;
pub type VkImageAspectFlags = u32;
pub type VkImageCreateFlagBits = u32;
pub const VK_IMAGE_CREATE_SPARSE_BINDING_BIT: VkImageCreateFlagBits = 0x1;
pub const VK_IMAGE_CREATE_SPARSE_RESIDENCY_BIT: VkImageCreateFlagBits = 0x2;
pub const VK_IMAGE_CREATE_SPARSE_ALIASED_BIT: VkImageCreateFlagBits = 0x4;
pub const VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT: VkImageCreateFlagBits = 0x8;
pub const VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x10;
pub const VK_IMAGE_CREATE_ALIAS_BIT: VkImageCreateFlagBits = 0x400;
pub const VK_IMAGE_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT: VkImageCreateFlagBits = 0x40;
pub const VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x20;
pub const VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT: VkImageCreateFlagBits = 0x80;
pub const VK_IMAGE_CREATE_EXTENDED_USAGE_BIT: VkImageCreateFlagBits = 0x100;
pub const VK_IMAGE_CREATE_PROTECTED_BIT: VkImageCreateFlagBits = 0x800;
pub const VK_IMAGE_CREATE_DISJOINT_BIT: VkImageCreateFlagBits = 0x200;
pub const VK_IMAGE_CREATE_CORNER_SAMPLED_BIT_NV: VkImageCreateFlagBits = 0x2000;
pub const VK_IMAGE_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT;
pub const VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_2D_ARRAY_COMPATIBLE_BIT;
pub const VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_BLOCK_TEXEL_VIEW_COMPATIBLE_BIT;
pub const VK_IMAGE_CREATE_EXTENDED_USAGE_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_EXTENDED_USAGE_BIT;
pub const VK_IMAGE_CREATE_DESCRIPTOR_HEAP_CAPTURE_REPLAY_BIT_EXT: VkImageCreateFlagBits = 0x10000;
pub const VK_IMAGE_CREATE_SAMPLE_LOCATIONS_COMPATIBLE_DEPTH_BIT_EXT: VkImageCreateFlagBits = 0x1000;
pub const VK_IMAGE_CREATE_DISJOINT_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_DISJOINT_BIT;
pub const VK_IMAGE_CREATE_ALIAS_BIT_KHR: VkImageCreateFlagBits = VK_IMAGE_CREATE_ALIAS_BIT;
pub const VK_IMAGE_CREATE_SUBSAMPLED_BIT_EXT: VkImageCreateFlagBits = 0x4000;
pub const VK_IMAGE_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkImageCreateFlagBits = VK_IMAGE_CREATE_DESCRIPTOR_HEAP_CAPTURE_REPLAY_BIT_EXT;
pub const VK_IMAGE_CREATE_MULTISAMPLED_RENDER_TO_SINGLE_SAMPLED_BIT_EXT: VkImageCreateFlagBits = 0x40000;
pub const VK_IMAGE_CREATE_2D_VIEW_COMPATIBLE_BIT_EXT: VkImageCreateFlagBits = 0x20000;
pub const VK_IMAGE_CREATE_FRAGMENT_DENSITY_MAP_OFFSET_BIT_QCOM: VkImageCreateFlagBits = VK_IMAGE_CREATE_FRAGMENT_DENSITY_MAP_OFFSET_BIT_EXT;
pub const VK_IMAGE_CREATE_VIDEO_PROFILE_INDEPENDENT_BIT_KHR: VkImageCreateFlagBits = 0x100000;
pub const VK_IMAGE_CREATE_FRAGMENT_DENSITY_MAP_OFFSET_BIT_EXT: VkImageCreateFlagBits = 0x8000;
pub type VkImageCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkImageCreateFlags,
    pub imageType: VkImageType,
    pub format: VkFormat,
    pub extent: VkExtent3D,
    pub mipLevels: u32,
    pub arrayLayers: u32,
    pub samples: VkSampleCountFlagBits,
    pub tiling: VkImageTiling,
    pub usage: VkImageUsageFlags,
    pub sharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
    pub initialLayout: VkImageLayout,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageFormatProperties {
    pub maxExtent: VkExtent3D,
    pub maxMipLevels: u32,
    pub maxArrayLayers: u32,
    pub sampleCounts: VkSampleCountFlags,
    pub maxResourceSize: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageFormatProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub imageFormatProperties: VkImageFormatProperties,
}
pub type VkImageLayout = i32;
pub const VK_IMAGE_LAYOUT_UNDEFINED: VkImageLayout = 0;
pub const VK_IMAGE_LAYOUT_GENERAL: VkImageLayout = 1;
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: VkImageLayout = 2;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 3;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 4;
pub const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: VkImageLayout = 5;
pub const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: VkImageLayout = 6;
pub const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: VkImageLayout = 7;
pub const VK_IMAGE_LAYOUT_PREINITIALIZED: VkImageLayout = 8;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 1000117000;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 1000117001;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL: VkImageLayout = 1000241000;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL: VkImageLayout = 1000241001;
pub const VK_IMAGE_LAYOUT_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 1000241002;
pub const VK_IMAGE_LAYOUT_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 1000241003;
pub const VK_IMAGE_LAYOUT_READ_ONLY_OPTIMAL: VkImageLayout = 1000314000;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_OPTIMAL: VkImageLayout = 1000314001;
pub const VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ: VkImageLayout = 1000232000;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: VkImageLayout = 1000001002;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_DST_KHR: VkImageLayout = 1000024000;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_SRC_KHR: VkImageLayout = 1000024001;
pub const VK_IMAGE_LAYOUT_VIDEO_DECODE_DPB_KHR: VkImageLayout = 1000024002;
pub const VK_IMAGE_LAYOUT_SHARED_PRESENT_KHR: VkImageLayout = 1000111000;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_STENCIL_READ_ONLY_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_STENCIL_READ_ONLY_OPTIMAL;
pub const VK_IMAGE_LAYOUT_SHADING_RATE_OPTIMAL_NV: VkImageLayout = VK_IMAGE_LAYOUT_FRAGMENT_SHADING_RATE_ATTACHMENT_OPTIMAL_KHR;
pub const VK_IMAGE_LAYOUT_FRAGMENT_DENSITY_MAP_OPTIMAL_EXT: VkImageLayout = 1000218000;
pub const VK_IMAGE_LAYOUT_FRAGMENT_SHADING_RATE_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = 1000164003;
pub const VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ_KHR: VkImageLayout = VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ;
pub const VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL;
pub const VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL;
pub const VK_IMAGE_LAYOUT_STENCIL_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_STENCIL_ATTACHMENT_OPTIMAL;
pub const VK_IMAGE_LAYOUT_STENCIL_READ_ONLY_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_STENCIL_READ_ONLY_OPTIMAL;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_DST_KHR: VkImageLayout = 1000299000;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_SRC_KHR: VkImageLayout = 1000299001;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_DPB_KHR: VkImageLayout = 1000299002;
pub const VK_IMAGE_LAYOUT_READ_ONLY_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_READ_ONLY_OPTIMAL;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_OPTIMAL_KHR: VkImageLayout = VK_IMAGE_LAYOUT_ATTACHMENT_OPTIMAL;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_FEEDBACK_LOOP_OPTIMAL_EXT: VkImageLayout = 1000339000;
pub const VK_IMAGE_LAYOUT_TENSOR_ALIASING_ARM: VkImageLayout = 1000460000;
pub const VK_IMAGE_LAYOUT_VIDEO_ENCODE_QUANTIZATION_MAP_KHR: VkImageLayout = 1000553000;
pub const VK_IMAGE_LAYOUT_ZERO_INITIALIZED_EXT: VkImageLayout = 1000620000;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageMemoryBarrier {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub oldLayout: VkImageLayout,
    pub newLayout: VkImageLayout,
    pub srcQueueFamilyIndex: u32,
    pub dstQueueFamilyIndex: u32,
    pub image: VkImage,
    pub subresourceRange: VkImageSubresourceRange,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageSubresource {
    pub aspectMask: VkImageAspectFlags,
    pub mipLevel: u32,
    pub arrayLayer: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageSubresourceLayers {
    pub aspectMask: VkImageAspectFlags,
    pub mipLevel: u32,
    pub baseArrayLayer: u32,
    pub layerCount: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageSubresourceRange {
    pub aspectMask: VkImageAspectFlags,
    pub baseMipLevel: u32,
    pub levelCount: u32,
    pub baseArrayLayer: u32,
    pub layerCount: u32,
}
pub type VkImageTiling = i32;
pub const VK_IMAGE_TILING_OPTIMAL: VkImageTiling = 0;
pub const VK_IMAGE_TILING_LINEAR: VkImageTiling = 1;
pub const VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT: VkImageTiling = 1000158000;
pub type VkImageType = i32;
pub const VK_IMAGE_TYPE_1D: VkImageType = 0;
pub const VK_IMAGE_TYPE_2D: VkImageType = 1;
pub const VK_IMAGE_TYPE_3D: VkImageType = 2;
pub type VkImageUsageFlagBits = u32;
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: VkImageUsageFlagBits = 0x1;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: VkImageUsageFlagBits = 0x2;
pub const VK_IMAGE_USAGE_SAMPLED_BIT: VkImageUsageFlagBits = 0x4;
pub const VK_IMAGE_USAGE_STORAGE_BIT: VkImageUsageFlagBits = 0x8;
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x10;
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x20;
pub const VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x40;
pub const VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT: VkImageUsageFlagBits = 0x80;
pub const VK_IMAGE_USAGE_HOST_TRANSFER_BIT: VkImageUsageFlagBits = 0x400000;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR: VkImageUsageFlagBits = 0x400;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_SRC_BIT_KHR: VkImageUsageFlagBits = 0x800;
pub const VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR: VkImageUsageFlagBits = 0x1000;
pub const VK_IMAGE_USAGE_SHADING_RATE_IMAGE_BIT_NV: VkImageUsageFlagBits = VK_IMAGE_USAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR;
pub const VK_IMAGE_USAGE_FRAGMENT_DENSITY_MAP_BIT_EXT: VkImageUsageFlagBits = 0x200;
pub const VK_IMAGE_USAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkImageUsageFlagBits = 0x100;
pub const VK_IMAGE_USAGE_HOST_TRANSFER_BIT_EXT: VkImageUsageFlagBits = VK_IMAGE_USAGE_HOST_TRANSFER_BIT;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_DST_BIT_KHR: VkImageUsageFlagBits = 0x2000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR: VkImageUsageFlagBits = 0x4000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR: VkImageUsageFlagBits = 0x8000;
pub const VK_IMAGE_USAGE_ATTACHMENT_FEEDBACK_LOOP_BIT_EXT: VkImageUsageFlagBits = 0x80000;
pub const VK_IMAGE_USAGE_INVOCATION_MASK_BIT_HUAWEI: VkImageUsageFlagBits = 0x40000;
pub const VK_IMAGE_USAGE_SAMPLE_WEIGHT_BIT_QCOM: VkImageUsageFlagBits = 0x100000;
pub const VK_IMAGE_USAGE_SAMPLE_BLOCK_MATCH_BIT_QCOM: VkImageUsageFlagBits = 0x200000;
pub const VK_IMAGE_USAGE_TENSOR_ALIASING_BIT_ARM: VkImageUsageFlagBits = 0x800000;
pub const VK_IMAGE_USAGE_TILE_MEMORY_BIT_QCOM: VkImageUsageFlagBits = 0x8000000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_QUANTIZATION_DELTA_MAP_BIT_KHR: VkImageUsageFlagBits = 0x2000000;
pub const VK_IMAGE_USAGE_VIDEO_ENCODE_EMPHASIS_MAP_BIT_KHR: VkImageUsageFlagBits = 0x4000000;
pub type VkImageUsageFlags = u32;
pub type VkImageView = u64;
pub type VkImageViewCreateFlagBits = u32;
pub const VK_IMAGE_VIEW_CREATE_FRAGMENT_DENSITY_MAP_DYNAMIC_BIT_EXT: VkImageViewCreateFlagBits = 0x1;
pub const VK_IMAGE_VIEW_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkImageViewCreateFlagBits = 0x4;
pub const VK_IMAGE_VIEW_CREATE_FRAGMENT_DENSITY_MAP_DEFERRED_BIT_EXT: VkImageViewCreateFlagBits = 0x2;
pub type VkImageViewCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkImageViewCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkImageViewCreateFlags,
    pub image: VkImage,
    pub viewType: VkImageViewType,
    pub format: VkFormat,
    pub components: VkComponentMapping,
    pub subresourceRange: VkImageSubresourceRange,
}
pub type VkImageViewType = i32;
pub const VK_IMAGE_VIEW_TYPE_1D: VkImageViewType = 0;
pub const VK_IMAGE_VIEW_TYPE_2D: VkImageViewType = 1;
pub const VK_IMAGE_VIEW_TYPE_3D: VkImageViewType = 2;
pub const VK_IMAGE_VIEW_TYPE_CUBE: VkImageViewType = 3;
pub const VK_IMAGE_VIEW_TYPE_1D_ARRAY: VkImageViewType = 4;
pub const VK_IMAGE_VIEW_TYPE_2D_ARRAY: VkImageViewType = 5;
pub const VK_IMAGE_VIEW_TYPE_CUBE_ARRAY: VkImageViewType = 6;
pub type VkIndexType = i32;
pub const VK_INDEX_TYPE_UINT16: VkIndexType = 0;
pub const VK_INDEX_TYPE_UINT32: VkIndexType = 1;
pub const VK_INDEX_TYPE_UINT8: VkIndexType = 1000265000;
pub const VK_INDEX_TYPE_NONE_KHR: VkIndexType = 1000165000;
pub const VK_INDEX_TYPE_NONE_NV: VkIndexType = VK_INDEX_TYPE_NONE_KHR;
pub const VK_INDEX_TYPE_UINT8_EXT: VkIndexType = VK_INDEX_TYPE_UINT8;
pub const VK_INDEX_TYPE_UINT8_KHR: VkIndexType = VK_INDEX_TYPE_UINT8;
pub type VkInstance = *mut c_void;
pub type VkInstanceCreateFlagBits = u32;
pub const VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR: VkInstanceCreateFlagBits = 0x1;
pub type VkInstanceCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkInstanceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkInstanceCreateFlags,
    pub pApplicationInfo: *const VkApplicationInfo,
    pub enabledLayerCount: u32,
    pub ppEnabledLayerNames: *const *const c_char,
    pub enabledExtensionCount: u32,
    pub ppEnabledExtensionNames: *const *const c_char,
}
pub type VkInternalAllocationType = i32;
pub const VK_INTERNAL_ALLOCATION_TYPE_EXECUTABLE: VkInternalAllocationType = 0;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkLayerProperties {
    pub layerName: [c_char; VK_MAX_EXTENSION_NAME_SIZE as usize],
    pub specVersion: u32,
    pub implementationVersion: u32,
    pub description: [c_char; VK_MAX_DESCRIPTION_SIZE as usize],
}
pub type VkLogicOp = i32;
pub const VK_LOGIC_OP_CLEAR: VkLogicOp = 0;
pub const VK_LOGIC_OP_AND: VkLogicOp = 1;
pub const VK_LOGIC_OP_AND_REVERSE: VkLogicOp = 2;
pub const VK_LOGIC_OP_COPY: VkLogicOp = 3;
pub const VK_LOGIC_OP_AND_INVERTED: VkLogicOp = 4;
pub const VK_LOGIC_OP_NO_OP: VkLogicOp = 5;
pub const VK_LOGIC_OP_XOR: VkLogicOp = 6;
pub const VK_LOGIC_OP_OR: VkLogicOp = 7;
pub const VK_LOGIC_OP_NOR: VkLogicOp = 8;
pub const VK_LOGIC_OP_EQUIVALENT: VkLogicOp = 9;
pub const VK_LOGIC_OP_INVERT: VkLogicOp = 10;
pub const VK_LOGIC_OP_OR_REVERSE: VkLogicOp = 11;
pub const VK_LOGIC_OP_COPY_INVERTED: VkLogicOp = 12;
pub const VK_LOGIC_OP_OR_INVERTED: VkLogicOp = 13;
pub const VK_LOGIC_OP_NAND: VkLogicOp = 14;
pub const VK_LOGIC_OP_SET: VkLogicOp = 15;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMappedMemoryRange {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub memory: VkDeviceMemory,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub allocationSize: VkDeviceSize,
    pub memoryTypeIndex: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryBarrier {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryHeap {
    pub size: VkDeviceSize,
    pub flags: VkMemoryHeapFlags,
}
pub type VkMemoryHeapFlagBits = u32;
pub const VK_MEMORY_HEAP_DEVICE_LOCAL_BIT: VkMemoryHeapFlagBits = 0x1;
pub const VK_MEMORY_HEAP_MULTI_INSTANCE_BIT: VkMemoryHeapFlagBits = 0x2;
pub const VK_MEMORY_HEAP_MULTI_INSTANCE_BIT_KHR: VkMemoryHeapFlagBits = VK_MEMORY_HEAP_MULTI_INSTANCE_BIT;
pub const VK_MEMORY_HEAP_TILE_MEMORY_BIT_QCOM: VkMemoryHeapFlagBits = 0x8;
pub type VkMemoryHeapFlags = u32;
pub type VkMemoryMapFlagBits = u32;
pub const VK_MEMORY_MAP_PLACED_BIT_EXT: VkMemoryMapFlagBits = 0x1;
pub type VkMemoryMapFlags = u32;
pub type VkMemoryPropertyFlagBits = u32;
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: VkMemoryPropertyFlagBits = 0x1;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: VkMemoryPropertyFlagBits = 0x2;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: VkMemoryPropertyFlagBits = 0x4;
pub const VK_MEMORY_PROPERTY_HOST_CACHED_BIT: VkMemoryPropertyFlagBits = 0x8;
pub const VK_MEMORY_PROPERTY_LAZILY_ALLOCATED_BIT: VkMemoryPropertyFlagBits = 0x10;
pub const VK_MEMORY_PROPERTY_PROTECTED_BIT: VkMemoryPropertyFlagBits = 0x20;
pub const VK_MEMORY_PROPERTY_DEVICE_COHERENT_BIT_AMD: VkMemoryPropertyFlagBits = 0x40;
pub const VK_MEMORY_PROPERTY_DEVICE_UNCACHED_BIT_AMD: VkMemoryPropertyFlagBits = 0x80;
pub const VK_MEMORY_PROPERTY_RDMA_CAPABLE_BIT_NV: VkMemoryPropertyFlagBits = 0x100;
pub type VkMemoryPropertyFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryRequirements {
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    pub memoryTypeBits: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkMemoryType {
    pub propertyFlags: VkMemoryPropertyFlags,
    pub heapIndex: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkOffset2D {
    pub x: i32,
    pub y: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkOffset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}
pub type VkPhysicalDevice = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceDriverProperties {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub driverID: VkDriverId,
    pub driverName: [c_char; VK_MAX_DRIVER_NAME_SIZE as usize],
    pub driverInfo: [c_char; VK_MAX_DRIVER_INFO_SIZE as usize],
    pub conformanceVersion: VkConformanceVersion,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceFeatures {
    pub robustBufferAccess: VkBool32,
    pub fullDrawIndexUint32: VkBool32,
    pub imageCubeArray: VkBool32,
    pub independentBlend: VkBool32,
    pub geometryShader: VkBool32,
    pub tessellationShader: VkBool32,
    pub sampleRateShading: VkBool32,
    pub dualSrcBlend: VkBool32,
    pub logicOp: VkBool32,
    pub multiDrawIndirect: VkBool32,
    pub drawIndirectFirstInstance: VkBool32,
    pub depthClamp: VkBool32,
    pub depthBiasClamp: VkBool32,
    pub fillModeNonSolid: VkBool32,
    pub depthBounds: VkBool32,
    pub wideLines: VkBool32,
    pub largePoints: VkBool32,
    pub alphaToOne: VkBool32,
    pub multiViewport: VkBool32,
    pub samplerAnisotropy: VkBool32,
    pub textureCompressionETC2: VkBool32,
    pub textureCompressionASTC_LDR: VkBool32,
    pub textureCompressionBC: VkBool32,
    pub occlusionQueryPrecise: VkBool32,
    pub pipelineStatisticsQuery: VkBool32,
    pub vertexPipelineStoresAndAtomics: VkBool32,
    pub fragmentStoresAndAtomics: VkBool32,
    pub shaderTessellationAndGeometryPointSize: VkBool32,
    pub shaderImageGatherExtended: VkBool32,
    pub shaderStorageImageExtendedFormats: VkBool32,
    pub shaderStorageImageMultisample: VkBool32,
    pub shaderStorageImageReadWithoutFormat: VkBool32,
    pub shaderStorageImageWriteWithoutFormat: VkBool32,
    pub shaderUniformBufferArrayDynamicIndexing: VkBool32,
    pub shaderSampledImageArrayDynamicIndexing: VkBool32,
    pub shaderStorageBufferArrayDynamicIndexing: VkBool32,
    pub shaderStorageImageArrayDynamicIndexing: VkBool32,
    pub shaderClipDistance: VkBool32,
    pub shaderCullDistance: VkBool32,
    pub shaderFloat64: VkBool32,
    pub shaderInt64: VkBool32,
    pub shaderInt16: VkBool32,
    pub shaderResourceResidency: VkBool32,
    pub shaderResourceMinLod: VkBool32,
    pub sparseBinding: VkBool32,
    pub sparseResidencyBuffer: VkBool32,
    pub sparseResidencyImage2D: VkBool32,
    pub sparseResidencyImage3D: VkBool32,
    pub sparseResidency2Samples: VkBool32,
    pub sparseResidency4Samples: VkBool32,
    pub sparseResidency8Samples: VkBool32,
    pub sparseResidency16Samples: VkBool32,
    pub sparseResidencyAliased: VkBool32,
    pub variableMultisampleRate: VkBool32,
    pub inheritedQueries: VkBool32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceFeatures2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub features: VkPhysicalDeviceFeatures,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceIDProperties {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub deviceUUID: [u8; VK_UUID_SIZE as usize],
    pub driverUUID: [u8; VK_UUID_SIZE as usize],
    pub deviceLUID: [u8; VK_LUID_SIZE as usize],
    pub deviceNodeMask: u32,
    pub deviceLUIDValid: VkBool32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceImageFormatInfo2 {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub format: VkFormat,
    pub type_: VkImageType,
    pub tiling: VkImageTiling,
    pub usage: VkImageUsageFlags,
    pub flags: VkImageCreateFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceLimits {
    pub maxImageDimension1D: u32,
    pub maxImageDimension2D: u32,
    pub maxImageDimension3D: u32,
    pub maxImageDimensionCube: u32,
    pub maxImageArrayLayers: u32,
    pub maxTexelBufferElements: u32,
    pub maxUniformBufferRange: u32,
    pub maxStorageBufferRange: u32,
    pub maxPushConstantsSize: u32,
    pub maxMemoryAllocationCount: u32,
    pub maxSamplerAllocationCount: u32,
    pub bufferImageGranularity: VkDeviceSize,
    pub sparseAddressSpaceSize: VkDeviceSize,
    pub maxBoundDescriptorSets: u32,
    pub maxPerStageDescriptorSamplers: u32,
    pub maxPerStageDescriptorUniformBuffers: u32,
    pub maxPerStageDescriptorStorageBuffers: u32,
    pub maxPerStageDescriptorSampledImages: u32,
    pub maxPerStageDescriptorStorageImages: u32,
    pub maxPerStageDescriptorInputAttachments: u32,
    pub maxPerStageResources: u32,
    pub maxDescriptorSetSamplers: u32,
    pub maxDescriptorSetUniformBuffers: u32,
    pub maxDescriptorSetUniformBuffersDynamic: u32,
    pub maxDescriptorSetStorageBuffers: u32,
    pub maxDescriptorSetStorageBuffersDynamic: u32,
    pub maxDescriptorSetSampledImages: u32,
    pub maxDescriptorSetStorageImages: u32,
    pub maxDescriptorSetInputAttachments: u32,
    pub maxVertexInputAttributes: u32,
    pub maxVertexInputBindings: u32,
    pub maxVertexInputAttributeOffset: u32,
    pub maxVertexInputBindingStride: u32,
    pub maxVertexOutputComponents: u32,
    pub maxTessellationGenerationLevel: u32,
    pub maxTessellationPatchSize: u32,
    pub maxTessellationControlPerVertexInputComponents: u32,
    pub maxTessellationControlPerVertexOutputComponents: u32,
    pub maxTessellationControlPerPatchOutputComponents: u32,
    pub maxTessellationControlTotalOutputComponents: u32,
    pub maxTessellationEvaluationInputComponents: u32,
    pub maxTessellationEvaluationOutputComponents: u32,
    pub maxGeometryShaderInvocations: u32,
    pub maxGeometryInputComponents: u32,
    pub maxGeometryOutputComponents: u32,
    pub maxGeometryOutputVertices: u32,
    pub maxGeometryTotalOutputComponents: u32,
    pub maxFragmentInputComponents: u32,
    pub maxFragmentOutputAttachments: u32,
    pub maxFragmentDualSrcAttachments: u32,
    pub maxFragmentCombinedOutputResources: u32,
    pub maxComputeSharedMemorySize: u32,
    pub maxComputeWorkGroupCount: [u32; 3 as usize],
    pub maxComputeWorkGroupInvocations: u32,
    pub maxComputeWorkGroupSize: [u32; 3 as usize],
    pub subPixelPrecisionBits: u32,
    pub subTexelPrecisionBits: u32,
    pub mipmapPrecisionBits: u32,
    pub maxDrawIndexedIndexValue: u32,
    pub maxDrawIndirectCount: u32,
    pub maxSamplerLodBias: f32,
    pub maxSamplerAnisotropy: f32,
    pub maxViewports: u32,
    pub maxViewportDimensions: [u32; 2 as usize],
    pub viewportBoundsRange: [f32; 2 as usize],
    pub viewportSubPixelBits: u32,
    pub minMemoryMapAlignment: usize,
    pub minTexelBufferOffsetAlignment: VkDeviceSize,
    pub minUniformBufferOffsetAlignment: VkDeviceSize,
    pub minStorageBufferOffsetAlignment: VkDeviceSize,
    pub minTexelOffset: i32,
    pub maxTexelOffset: u32,
    pub minTexelGatherOffset: i32,
    pub maxTexelGatherOffset: u32,
    pub minInterpolationOffset: f32,
    pub maxInterpolationOffset: f32,
    pub subPixelInterpolationOffsetBits: u32,
    pub maxFramebufferWidth: u32,
    pub maxFramebufferHeight: u32,
    pub maxFramebufferLayers: u32,
    pub framebufferColorSampleCounts: VkSampleCountFlags,
    pub framebufferDepthSampleCounts: VkSampleCountFlags,
    pub framebufferStencilSampleCounts: VkSampleCountFlags,
    pub framebufferNoAttachmentsSampleCounts: VkSampleCountFlags,
    pub maxColorAttachments: u32,
    pub sampledImageColorSampleCounts: VkSampleCountFlags,
    pub sampledImageIntegerSampleCounts: VkSampleCountFlags,
    pub sampledImageDepthSampleCounts: VkSampleCountFlags,
    pub sampledImageStencilSampleCounts: VkSampleCountFlags,
    pub storageImageSampleCounts: VkSampleCountFlags,
    pub maxSampleMaskWords: u32,
    pub timestampComputeAndGraphics: VkBool32,
    pub timestampPeriod: f32,
    pub maxClipDistances: u32,
    pub maxCullDistances: u32,
    pub maxCombinedClipAndCullDistances: u32,
    pub discreteQueuePriorities: u32,
    pub pointSizeRange: [f32; 2 as usize],
    pub lineWidthRange: [f32; 2 as usize],
    pub pointSizeGranularity: f32,
    pub lineWidthGranularity: f32,
    pub strictLines: VkBool32,
    pub standardSampleLocations: VkBool32,
    pub optimalBufferCopyOffsetAlignment: VkDeviceSize,
    pub optimalBufferCopyRowPitchAlignment: VkDeviceSize,
    pub nonCoherentAtomSize: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memoryTypeCount: u32,
    pub memoryTypes: [VkMemoryType; VK_MAX_MEMORY_TYPES as usize],
    pub memoryHeapCount: u32,
    pub memoryHeaps: [VkMemoryHeap; VK_MAX_MEMORY_HEAPS as usize],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceMemoryProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub memoryProperties: VkPhysicalDeviceMemoryProperties,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceProperties {
    pub apiVersion: u32,
    pub driverVersion: u32,
    pub vendorID: u32,
    pub deviceID: u32,
    pub deviceType: VkPhysicalDeviceType,
    pub deviceName: [c_char; VK_MAX_PHYSICAL_DEVICE_NAME_SIZE as usize],
    pub pipelineCacheUUID: [u8; VK_UUID_SIZE as usize],
    pub limits: VkPhysicalDeviceLimits,
    pub sparseProperties: VkPhysicalDeviceSparseProperties,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub properties: VkPhysicalDeviceProperties,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceSparseImageFormatInfo2 {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub format: VkFormat,
    pub type_: VkImageType,
    pub samples: VkSampleCountFlagBits,
    pub usage: VkImageUsageFlags,
    pub tiling: VkImageTiling,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPhysicalDeviceSparseProperties {
    pub residencyStandard2DBlockShape: VkBool32,
    pub residencyStandard2DMultisampleBlockShape: VkBool32,
    pub residencyStandard3DBlockShape: VkBool32,
    pub residencyAlignedMipSize: VkBool32,
    pub residencyNonResidentStrict: VkBool32,
}
pub type VkPhysicalDeviceType = i32;
pub const VK_PHYSICAL_DEVICE_TYPE_OTHER: VkPhysicalDeviceType = 0;
pub const VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU: VkPhysicalDeviceType = 1;
pub const VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU: VkPhysicalDeviceType = 2;
pub const VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU: VkPhysicalDeviceType = 3;
pub const VK_PHYSICAL_DEVICE_TYPE_CPU: VkPhysicalDeviceType = 4;
pub type VkPipeline = u64;
pub type VkPipelineBindPoint = i32;
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: VkPipelineBindPoint = 0;
pub const VK_PIPELINE_BIND_POINT_COMPUTE: VkPipelineBindPoint = 1;
pub const VK_PIPELINE_BIND_POINT_EXECUTION_GRAPH_AMDX: VkPipelineBindPoint = 1000134000;
pub const VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR: VkPipelineBindPoint = 1000165000;
pub const VK_PIPELINE_BIND_POINT_RAY_TRACING_NV: VkPipelineBindPoint = VK_PIPELINE_BIND_POINT_RAY_TRACING_KHR;
pub const VK_PIPELINE_BIND_POINT_SUBPASS_SHADING_HUAWEI: VkPipelineBindPoint = 1000369003;
pub const VK_PIPELINE_BIND_POINT_DATA_GRAPH_ARM: VkPipelineBindPoint = 1000507000;
pub type VkPipelineCache = u64;
pub type VkPipelineCacheCreateFlagBits = u32;
pub const VK_PIPELINE_CACHE_CREATE_EXTERNALLY_SYNCHRONIZED_BIT: VkPipelineCacheCreateFlagBits = 0x1;
pub const VK_PIPELINE_CACHE_CREATE_EXTERNALLY_SYNCHRONIZED_BIT_EXT: VkPipelineCacheCreateFlagBits = VK_PIPELINE_CACHE_CREATE_EXTERNALLY_SYNCHRONIZED_BIT;
pub const VK_PIPELINE_CACHE_CREATE_INTERNALLY_SYNCHRONIZED_MERGE_BIT_KHR: VkPipelineCacheCreateFlagBits = 0x8;
pub type VkPipelineCacheCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineCacheCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineCacheCreateFlags,
    pub initialDataSize: usize,
    pub pInitialData: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineColorBlendAttachmentState {
    pub blendEnable: VkBool32,
    pub srcColorBlendFactor: VkBlendFactor,
    pub dstColorBlendFactor: VkBlendFactor,
    pub colorBlendOp: VkBlendOp,
    pub srcAlphaBlendFactor: VkBlendFactor,
    pub dstAlphaBlendFactor: VkBlendFactor,
    pub alphaBlendOp: VkBlendOp,
    pub colorWriteMask: VkColorComponentFlags,
}
pub type VkPipelineColorBlendStateCreateFlagBits = u32;
pub const VK_PIPELINE_COLOR_BLEND_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_BIT_ARM: VkPipelineColorBlendStateCreateFlagBits = VK_PIPELINE_COLOR_BLEND_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_BIT_EXT;
pub const VK_PIPELINE_COLOR_BLEND_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_BIT_EXT: VkPipelineColorBlendStateCreateFlagBits = 0x1;
pub type VkPipelineColorBlendStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineColorBlendStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineColorBlendStateCreateFlags,
    pub logicOpEnable: VkBool32,
    pub logicOp: VkLogicOp,
    pub attachmentCount: u32,
    pub pAttachments: *const VkPipelineColorBlendAttachmentState,
    pub blendConstants: [f32; 4 as usize],
}
pub type VkPipelineCreateFlagBits = u32;
pub const VK_PIPELINE_CREATE_DISABLE_OPTIMIZATION_BIT: VkPipelineCreateFlagBits = 0x1;
pub const VK_PIPELINE_CREATE_ALLOW_DERIVATIVES_BIT: VkPipelineCreateFlagBits = 0x2;
pub const VK_PIPELINE_CREATE_DERIVATIVE_BIT: VkPipelineCreateFlagBits = 0x4;
pub const VK_PIPELINE_CREATE_DISPATCH_BASE_BIT: VkPipelineCreateFlagBits = 0x10;
pub const VK_PIPELINE_CREATE_DISPATCH_BASE: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_DISPATCH_BASE_BIT;
pub const VK_PIPELINE_CREATE_VIEW_INDEX_FROM_DEVICE_INDEX_BIT: VkPipelineCreateFlagBits = 0x8;
pub const VK_PIPELINE_CREATE_FAIL_ON_PIPELINE_COMPILE_REQUIRED_BIT: VkPipelineCreateFlagBits = 0x100;
pub const VK_PIPELINE_CREATE_EARLY_RETURN_ON_FAILURE_BIT: VkPipelineCreateFlagBits = 0x200;
pub const VK_PIPELINE_CREATE_NO_PROTECTED_ACCESS_BIT: VkPipelineCreateFlagBits = 0x8000000;
pub const VK_PIPELINE_CREATE_PROTECTED_ACCESS_ONLY_BIT: VkPipelineCreateFlagBits = 0x40000000;
pub const VK_PIPELINE_CREATE_VIEW_INDEX_FROM_DEVICE_INDEX_BIT_KHR: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_VIEW_INDEX_FROM_DEVICE_INDEX_BIT;
pub const VK_PIPELINE_CREATE_DISPATCH_BASE_BIT_KHR: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_DISPATCH_BASE_BIT;
pub const VK_PIPELINE_CREATE_DISPATCH_BASE_KHR: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_DISPATCH_BASE_BIT;
pub const VK_PIPELINE_CREATE_RAY_TRACING_NO_NULL_ANY_HIT_SHADERS_BIT_KHR: VkPipelineCreateFlagBits = 0x4000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_NO_NULL_CLOSEST_HIT_SHADERS_BIT_KHR: VkPipelineCreateFlagBits = 0x8000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_NO_NULL_MISS_SHADERS_BIT_KHR: VkPipelineCreateFlagBits = 0x10000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_NO_NULL_INTERSECTION_SHADERS_BIT_KHR: VkPipelineCreateFlagBits = 0x20000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_SKIP_TRIANGLES_BIT_KHR: VkPipelineCreateFlagBits = 0x1000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_SKIP_AABBS_BIT_KHR: VkPipelineCreateFlagBits = 0x2000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_SHADER_GROUP_HANDLE_CAPTURE_REPLAY_BIT_KHR: VkPipelineCreateFlagBits = 0x80000;
pub const VK_PIPELINE_CREATE_DEFER_COMPILE_BIT_NV: VkPipelineCreateFlagBits = 0x20;
pub const VK_PIPELINE_CREATE_RENDERING_FRAGMENT_DENSITY_MAP_ATTACHMENT_BIT_EXT: VkPipelineCreateFlagBits = 0x400000;
pub const VK_PIPELINE_RASTERIZATION_STATE_CREATE_FRAGMENT_DENSITY_MAP_ATTACHMENT_BIT_EXT: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_RENDERING_FRAGMENT_DENSITY_MAP_ATTACHMENT_BIT_EXT;
pub const VK_PIPELINE_CREATE_RENDERING_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkPipelineCreateFlagBits = 0x200000;
pub const VK_PIPELINE_RASTERIZATION_STATE_CREATE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_RENDERING_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR;
pub const VK_PIPELINE_CREATE_CAPTURE_STATISTICS_BIT_KHR: VkPipelineCreateFlagBits = 0x40;
pub const VK_PIPELINE_CREATE_CAPTURE_INTERNAL_REPRESENTATIONS_BIT_KHR: VkPipelineCreateFlagBits = 0x80;
pub const VK_PIPELINE_CREATE_INDIRECT_BINDABLE_BIT_NV: VkPipelineCreateFlagBits = 0x40000;
pub const VK_PIPELINE_CREATE_LIBRARY_BIT_KHR: VkPipelineCreateFlagBits = 0x800;
pub const VK_PIPELINE_CREATE_FAIL_ON_PIPELINE_COMPILE_REQUIRED_BIT_EXT: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_FAIL_ON_PIPELINE_COMPILE_REQUIRED_BIT;
pub const VK_PIPELINE_CREATE_EARLY_RETURN_ON_FAILURE_BIT_EXT: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_EARLY_RETURN_ON_FAILURE_BIT;
pub const VK_PIPELINE_CREATE_DESCRIPTOR_BUFFER_BIT_EXT: VkPipelineCreateFlagBits = 0x20000000;
pub const VK_PIPELINE_CREATE_RETAIN_LINK_TIME_OPTIMIZATION_INFO_BIT_EXT: VkPipelineCreateFlagBits = 0x800000;
pub const VK_PIPELINE_CREATE_LINK_TIME_OPTIMIZATION_BIT_EXT: VkPipelineCreateFlagBits = 0x400;
pub const VK_PIPELINE_CREATE_RAY_TRACING_ALLOW_MOTION_BIT_NV: VkPipelineCreateFlagBits = 0x100000;
pub const VK_PIPELINE_CREATE_COLOR_ATTACHMENT_FEEDBACK_LOOP_BIT_EXT: VkPipelineCreateFlagBits = 0x2000000;
pub const VK_PIPELINE_CREATE_DEPTH_STENCIL_ATTACHMENT_FEEDBACK_LOOP_BIT_EXT: VkPipelineCreateFlagBits = 0x4000000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_OPACITY_MICROMAP_BIT_EXT: VkPipelineCreateFlagBits = 0x1000000;
pub const VK_PIPELINE_CREATE_RAY_TRACING_DISPLACEMENT_MICROMAP_BIT_NV: VkPipelineCreateFlagBits = 0x10000000;
pub const VK_PIPELINE_CREATE_NO_PROTECTED_ACCESS_BIT_EXT: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_NO_PROTECTED_ACCESS_BIT;
pub const VK_PIPELINE_CREATE_PROTECTED_ACCESS_ONLY_BIT_EXT: VkPipelineCreateFlagBits = VK_PIPELINE_CREATE_PROTECTED_ACCESS_ONLY_BIT;
pub type VkPipelineCreateFlags = u32;
pub type VkPipelineDepthStencilStateCreateFlagBits = u32;
pub const VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_ARM: VkPipelineDepthStencilStateCreateFlagBits = VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_EXT;
pub const VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_ARM: VkPipelineDepthStencilStateCreateFlagBits = VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_EXT;
pub const VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_EXT: VkPipelineDepthStencilStateCreateFlagBits = 0x1;
pub const VK_PIPELINE_DEPTH_STENCIL_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_EXT: VkPipelineDepthStencilStateCreateFlagBits = 0x2;
pub type VkPipelineDepthStencilStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineDepthStencilStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineDepthStencilStateCreateFlags,
    pub depthTestEnable: VkBool32,
    pub depthWriteEnable: VkBool32,
    pub depthCompareOp: VkCompareOp,
    pub depthBoundsTestEnable: VkBool32,
    pub stencilTestEnable: VkBool32,
    pub front: VkStencilOpState,
    pub back: VkStencilOpState,
    pub minDepthBounds: f32,
    pub maxDepthBounds: f32,
}
pub type VkPipelineDynamicStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineDynamicStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineDynamicStateCreateFlags,
    pub dynamicStateCount: u32,
    pub pDynamicStates: *const VkDynamicState,
}
pub type VkPipelineInputAssemblyStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineInputAssemblyStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineInputAssemblyStateCreateFlags,
    pub topology: VkPrimitiveTopology,
    pub primitiveRestartEnable: VkBool32,
}
pub type VkPipelineLayout = u64;
pub type VkPipelineLayoutCreateFlagBits = u32;
pub const VK_PIPELINE_LAYOUT_CREATE_INDEPENDENT_SETS_BIT_EXT: VkPipelineLayoutCreateFlagBits = 0x2;
pub type VkPipelineLayoutCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineLayoutCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineLayoutCreateFlags,
    pub setLayoutCount: u32,
    pub pSetLayouts: *const VkDescriptorSetLayout,
    pub pushConstantRangeCount: u32,
    pub pPushConstantRanges: *const VkPushConstantRange,
}
pub type VkPipelineMultisampleStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineMultisampleStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineMultisampleStateCreateFlags,
    pub rasterizationSamples: VkSampleCountFlagBits,
    pub sampleShadingEnable: VkBool32,
    pub minSampleShading: f32,
    pub pSampleMask: *const VkSampleMask,
    pub alphaToCoverageEnable: VkBool32,
    pub alphaToOneEnable: VkBool32,
}
pub type VkPipelineRasterizationStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineRasterizationStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineRasterizationStateCreateFlags,
    pub depthClampEnable: VkBool32,
    pub rasterizerDiscardEnable: VkBool32,
    pub polygonMode: VkPolygonMode,
    pub cullMode: VkCullModeFlags,
    pub frontFace: VkFrontFace,
    pub depthBiasEnable: VkBool32,
    pub depthBiasConstantFactor: f32,
    pub depthBiasClamp: f32,
    pub depthBiasSlopeFactor: f32,
    pub lineWidth: f32,
}
pub type VkPipelineShaderStageCreateFlagBits = u32;
pub const VK_PIPELINE_SHADER_STAGE_CREATE_ALLOW_VARYING_SUBGROUP_SIZE_BIT: VkPipelineShaderStageCreateFlagBits = 0x1;
pub const VK_PIPELINE_SHADER_STAGE_CREATE_REQUIRE_FULL_SUBGROUPS_BIT: VkPipelineShaderStageCreateFlagBits = 0x2;
pub const VK_PIPELINE_SHADER_STAGE_CREATE_ALLOW_VARYING_SUBGROUP_SIZE_BIT_EXT: VkPipelineShaderStageCreateFlagBits = VK_PIPELINE_SHADER_STAGE_CREATE_ALLOW_VARYING_SUBGROUP_SIZE_BIT;
pub const VK_PIPELINE_SHADER_STAGE_CREATE_REQUIRE_FULL_SUBGROUPS_BIT_EXT: VkPipelineShaderStageCreateFlagBits = VK_PIPELINE_SHADER_STAGE_CREATE_REQUIRE_FULL_SUBGROUPS_BIT;
pub type VkPipelineShaderStageCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineShaderStageCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineShaderStageCreateFlags,
    pub stage: VkShaderStageFlagBits,
    pub module: VkShaderModule,
    pub pName: *const c_char,
    pub pSpecializationInfo: *const VkSpecializationInfo,
}
pub type VkPipelineStageFlagBits = u32;
pub const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: VkPipelineStageFlagBits = 0x1;
pub const VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT: VkPipelineStageFlagBits = 0x2;
pub const VK_PIPELINE_STAGE_VERTEX_INPUT_BIT: VkPipelineStageFlagBits = 0x4;
pub const VK_PIPELINE_STAGE_VERTEX_SHADER_BIT: VkPipelineStageFlagBits = 0x8;
pub const VK_PIPELINE_STAGE_TESSELLATION_CONTROL_SHADER_BIT: VkPipelineStageFlagBits = 0x10;
pub const VK_PIPELINE_STAGE_TESSELLATION_EVALUATION_SHADER_BIT: VkPipelineStageFlagBits = 0x20;
pub const VK_PIPELINE_STAGE_GEOMETRY_SHADER_BIT: VkPipelineStageFlagBits = 0x40;
pub const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: VkPipelineStageFlagBits = 0x80;
pub const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: VkPipelineStageFlagBits = 0x100;
pub const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: VkPipelineStageFlagBits = 0x200;
pub const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: VkPipelineStageFlagBits = 0x400;
pub const VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT: VkPipelineStageFlagBits = 0x800;
pub const VK_PIPELINE_STAGE_TRANSFER_BIT: VkPipelineStageFlagBits = 0x1000;
pub const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: VkPipelineStageFlagBits = 0x2000;
pub const VK_PIPELINE_STAGE_HOST_BIT: VkPipelineStageFlagBits = 0x4000;
pub const VK_PIPELINE_STAGE_ALL_GRAPHICS_BIT: VkPipelineStageFlagBits = 0x8000;
pub const VK_PIPELINE_STAGE_ALL_COMMANDS_BIT: VkPipelineStageFlagBits = 0x10000;
pub const VK_PIPELINE_STAGE_NONE: VkPipelineStageFlagBits = 0x0;
pub const VK_PIPELINE_STAGE_TRANSFORM_FEEDBACK_BIT_EXT: VkPipelineStageFlagBits = 0x1000000;
pub const VK_PIPELINE_STAGE_CONDITIONAL_RENDERING_BIT_EXT: VkPipelineStageFlagBits = 0x40000;
pub const VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR: VkPipelineStageFlagBits = 0x2000000;
pub const VK_PIPELINE_STAGE_RAY_TRACING_SHADER_BIT_KHR: VkPipelineStageFlagBits = 0x200000;
pub const VK_PIPELINE_STAGE_SHADING_RATE_IMAGE_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR;
pub const VK_PIPELINE_STAGE_RAY_TRACING_SHADER_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_RAY_TRACING_SHADER_BIT_KHR;
pub const VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR;
pub const VK_PIPELINE_STAGE_TASK_SHADER_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_TASK_SHADER_BIT_EXT;
pub const VK_PIPELINE_STAGE_MESH_SHADER_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_MESH_SHADER_BIT_EXT;
pub const VK_PIPELINE_STAGE_FRAGMENT_DENSITY_PROCESS_BIT_EXT: VkPipelineStageFlagBits = 0x800000;
pub const VK_PIPELINE_STAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR: VkPipelineStageFlagBits = 0x400000;
pub const VK_PIPELINE_STAGE_COMMAND_PREPROCESS_BIT_NV: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_COMMAND_PREPROCESS_BIT_EXT;
pub const VK_PIPELINE_STAGE_NONE_KHR: VkPipelineStageFlagBits = VK_PIPELINE_STAGE_NONE;
pub const VK_PIPELINE_STAGE_TASK_SHADER_BIT_EXT: VkPipelineStageFlagBits = 0x80000;
pub const VK_PIPELINE_STAGE_MESH_SHADER_BIT_EXT: VkPipelineStageFlagBits = 0x100000;
pub const VK_PIPELINE_STAGE_COMMAND_PREPROCESS_BIT_EXT: VkPipelineStageFlagBits = 0x20000;
pub type VkPipelineStageFlags = u32;
pub type VkPipelineTessellationStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineTessellationStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineTessellationStateCreateFlags,
    pub patchControlPoints: u32,
}
pub type VkPipelineVertexInputStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineVertexInputStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineVertexInputStateCreateFlags,
    pub vertexBindingDescriptionCount: u32,
    pub pVertexBindingDescriptions: *const VkVertexInputBindingDescription,
    pub vertexAttributeDescriptionCount: u32,
    pub pVertexAttributeDescriptions: *const VkVertexInputAttributeDescription,
}
pub type VkPipelineViewportStateCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPipelineViewportStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineViewportStateCreateFlags,
    pub viewportCount: u32,
    pub pViewports: *const VkViewport,
    pub scissorCount: u32,
    pub pScissors: *const VkRect2D,
}
pub type VkPolygonMode = i32;
pub const VK_POLYGON_MODE_FILL: VkPolygonMode = 0;
pub const VK_POLYGON_MODE_LINE: VkPolygonMode = 1;
pub const VK_POLYGON_MODE_POINT: VkPolygonMode = 2;
pub const VK_POLYGON_MODE_FILL_RECTANGLE_NV: VkPolygonMode = 1000153000;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPresentInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub waitSemaphoreCount: u32,
    pub pWaitSemaphores: *const VkSemaphore,
    pub swapchainCount: u32,
    pub pSwapchains: *const VkSwapchainKHR,
    pub pImageIndices: *const u32,
    pub pResults: *mut VkResult,
}
pub type VkPresentModeKHR = i32;
pub const VK_PRESENT_MODE_IMMEDIATE_KHR: VkPresentModeKHR = 0;
pub const VK_PRESENT_MODE_MAILBOX_KHR: VkPresentModeKHR = 1;
pub const VK_PRESENT_MODE_FIFO_KHR: VkPresentModeKHR = 2;
pub const VK_PRESENT_MODE_FIFO_RELAXED_KHR: VkPresentModeKHR = 3;
pub const VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR: VkPresentModeKHR = 1000111000;
pub const VK_PRESENT_MODE_SHARED_CONTINUOUS_REFRESH_KHR: VkPresentModeKHR = 1000111001;
pub const VK_PRESENT_MODE_FIFO_LATEST_READY_EXT: VkPresentModeKHR = VK_PRESENT_MODE_FIFO_LATEST_READY_KHR;
pub const VK_PRESENT_MODE_FIFO_LATEST_READY_KHR: VkPresentModeKHR = 1000361000;
pub type VkPrimitiveTopology = i32;
pub const VK_PRIMITIVE_TOPOLOGY_POINT_LIST: VkPrimitiveTopology = 0;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_LIST: VkPrimitiveTopology = 1;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_STRIP: VkPrimitiveTopology = 2;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: VkPrimitiveTopology = 3;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP: VkPrimitiveTopology = 4;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN: VkPrimitiveTopology = 5;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_LIST_WITH_ADJACENCY: VkPrimitiveTopology = 6;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_STRIP_WITH_ADJACENCY: VkPrimitiveTopology = 7;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST_WITH_ADJACENCY: VkPrimitiveTopology = 8;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP_WITH_ADJACENCY: VkPrimitiveTopology = 9;
pub const VK_PRIMITIVE_TOPOLOGY_PATCH_LIST: VkPrimitiveTopology = 10;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkPushConstantRange {
    pub stageFlags: VkShaderStageFlags,
    pub offset: u32,
    pub size: u32,
}
pub type VkQueryControlFlagBits = u32;
pub const VK_QUERY_CONTROL_PRECISE_BIT: VkQueryControlFlagBits = 0x1;
pub type VkQueryControlFlags = u32;
pub type VkQueryPipelineStatisticFlagBits = u32;
pub const VK_QUERY_PIPELINE_STATISTIC_INPUT_ASSEMBLY_VERTICES_BIT: VkQueryPipelineStatisticFlagBits = 0x1;
pub const VK_QUERY_PIPELINE_STATISTIC_INPUT_ASSEMBLY_PRIMITIVES_BIT: VkQueryPipelineStatisticFlagBits = 0x2;
pub const VK_QUERY_PIPELINE_STATISTIC_VERTEX_SHADER_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x4;
pub const VK_QUERY_PIPELINE_STATISTIC_GEOMETRY_SHADER_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x8;
pub const VK_QUERY_PIPELINE_STATISTIC_GEOMETRY_SHADER_PRIMITIVES_BIT: VkQueryPipelineStatisticFlagBits = 0x10;
pub const VK_QUERY_PIPELINE_STATISTIC_CLIPPING_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x20;
pub const VK_QUERY_PIPELINE_STATISTIC_CLIPPING_PRIMITIVES_BIT: VkQueryPipelineStatisticFlagBits = 0x40;
pub const VK_QUERY_PIPELINE_STATISTIC_FRAGMENT_SHADER_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x80;
pub const VK_QUERY_PIPELINE_STATISTIC_TESSELLATION_CONTROL_SHADER_PATCHES_BIT: VkQueryPipelineStatisticFlagBits = 0x100;
pub const VK_QUERY_PIPELINE_STATISTIC_TESSELLATION_EVALUATION_SHADER_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x200;
pub const VK_QUERY_PIPELINE_STATISTIC_COMPUTE_SHADER_INVOCATIONS_BIT: VkQueryPipelineStatisticFlagBits = 0x400;
pub const VK_QUERY_PIPELINE_STATISTIC_TASK_SHADER_INVOCATIONS_BIT_EXT: VkQueryPipelineStatisticFlagBits = 0x800;
pub const VK_QUERY_PIPELINE_STATISTIC_MESH_SHADER_INVOCATIONS_BIT_EXT: VkQueryPipelineStatisticFlagBits = 0x1000;
pub const VK_QUERY_PIPELINE_STATISTIC_CLUSTER_CULLING_SHADER_INVOCATIONS_BIT_HUAWEI: VkQueryPipelineStatisticFlagBits = 0x2000;
pub type VkQueryPipelineStatisticFlags = u32;
pub type VkQueue = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkQueueFamilyProperties {
    pub queueFlags: VkQueueFlags,
    pub queueCount: u32,
    pub timestampValidBits: u32,
    pub minImageTransferGranularity: VkExtent3D,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkQueueFamilyProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub queueFamilyProperties: VkQueueFamilyProperties,
}
pub type VkQueueFlagBits = u32;
pub const VK_QUEUE_GRAPHICS_BIT: VkQueueFlagBits = 0x1;
pub const VK_QUEUE_COMPUTE_BIT: VkQueueFlagBits = 0x2;
pub const VK_QUEUE_TRANSFER_BIT: VkQueueFlagBits = 0x4;
pub const VK_QUEUE_SPARSE_BINDING_BIT: VkQueueFlagBits = 0x8;
pub const VK_QUEUE_PROTECTED_BIT: VkQueueFlagBits = 0x10;
pub const VK_QUEUE_VIDEO_DECODE_BIT_KHR: VkQueueFlagBits = 0x20;
pub const VK_QUEUE_VIDEO_ENCODE_BIT_KHR: VkQueueFlagBits = 0x40;
pub const VK_QUEUE_OPTICAL_FLOW_BIT_NV: VkQueueFlagBits = 0x100;
pub const VK_QUEUE_DATA_GRAPH_BIT_ARM: VkQueueFlagBits = 0x400;
pub type VkQueueFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkRect2D {
    pub offset: VkOffset2D,
    pub extent: VkExtent2D,
}
pub type VkRenderPass = u64;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkRenderPassBeginInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub renderPass: VkRenderPass,
    pub framebuffer: VkFramebuffer,
    pub renderArea: VkRect2D,
    pub clearValueCount: u32,
    pub pClearValues: *const VkClearValue,
}
pub type VkRenderPassCreateFlagBits = u32;
pub const VK_RENDER_PASS_CREATE_TRANSFORM_BIT_QCOM: VkRenderPassCreateFlagBits = 0x2;
pub const VK_RENDER_PASS_CREATE_PER_LAYER_FRAGMENT_DENSITY_BIT_VALVE: VkRenderPassCreateFlagBits = 0x4;
pub type VkRenderPassCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkRenderPassCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkRenderPassCreateFlags,
    pub attachmentCount: u32,
    pub pAttachments: *const VkAttachmentDescription,
    pub subpassCount: u32,
    pub pSubpasses: *const VkSubpassDescription,
    pub dependencyCount: u32,
    pub pDependencies: *const VkSubpassDependency,
}
pub type VkResult = i32;
pub const VK_SUCCESS: VkResult = 0;
pub const VK_NOT_READY: VkResult = 1;
pub const VK_TIMEOUT: VkResult = 2;
pub const VK_EVENT_SET: VkResult = 3;
pub const VK_EVENT_RESET: VkResult = 4;
pub const VK_INCOMPLETE: VkResult = 5;
pub const VK_ERROR_OUT_OF_HOST_MEMORY: VkResult = -1;
pub const VK_ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub const VK_ERROR_INITIALIZATION_FAILED: VkResult = -3;
pub const VK_ERROR_DEVICE_LOST: VkResult = -4;
pub const VK_ERROR_MEMORY_MAP_FAILED: VkResult = -5;
pub const VK_ERROR_LAYER_NOT_PRESENT: VkResult = -6;
pub const VK_ERROR_EXTENSION_NOT_PRESENT: VkResult = -7;
pub const VK_ERROR_FEATURE_NOT_PRESENT: VkResult = -8;
pub const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
pub const VK_ERROR_TOO_MANY_OBJECTS: VkResult = -10;
pub const VK_ERROR_FORMAT_NOT_SUPPORTED: VkResult = -11;
pub const VK_ERROR_FRAGMENTED_POOL: VkResult = -12;
pub const VK_ERROR_UNKNOWN: VkResult = -13;
pub const VK_ERROR_VALIDATION_FAILED: VkResult = -1000011001;
pub const VK_ERROR_OUT_OF_POOL_MEMORY: VkResult = -1000069000;
pub const VK_ERROR_INVALID_EXTERNAL_HANDLE: VkResult = -1000072003;
pub const VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS: VkResult = -1000257000;
pub const VK_ERROR_FRAGMENTATION: VkResult = -1000161000;
pub const VK_PIPELINE_COMPILE_REQUIRED: VkResult = 1000297000;
pub const VK_ERROR_NOT_PERMITTED: VkResult = -1000174001;
pub const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub const VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;
pub const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
pub const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;
pub const VK_ERROR_INCOMPATIBLE_DISPLAY_KHR: VkResult = -1000003001;
pub const VK_ERROR_VALIDATION_FAILED_EXT: VkResult = VK_ERROR_VALIDATION_FAILED;
pub const VK_ERROR_INVALID_SHADER_NV: VkResult = -1000012000;
pub const VK_ERROR_IMAGE_USAGE_NOT_SUPPORTED_KHR: VkResult = -1000023000;
pub const VK_ERROR_VIDEO_PICTURE_LAYOUT_NOT_SUPPORTED_KHR: VkResult = -1000023001;
pub const VK_ERROR_VIDEO_PROFILE_OPERATION_NOT_SUPPORTED_KHR: VkResult = -1000023002;
pub const VK_ERROR_VIDEO_PROFILE_FORMAT_NOT_SUPPORTED_KHR: VkResult = -1000023003;
pub const VK_ERROR_VIDEO_PROFILE_CODEC_NOT_SUPPORTED_KHR: VkResult = -1000023004;
pub const VK_ERROR_VIDEO_STD_VERSION_NOT_SUPPORTED_KHR: VkResult = -1000023005;
pub const VK_ERROR_OUT_OF_POOL_MEMORY_KHR: VkResult = VK_ERROR_OUT_OF_POOL_MEMORY;
pub const VK_ERROR_INVALID_EXTERNAL_HANDLE_KHR: VkResult = VK_ERROR_INVALID_EXTERNAL_HANDLE;
pub const VK_ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT: VkResult = -1000158000;
pub const VK_ERROR_FRAGMENTATION_EXT: VkResult = VK_ERROR_FRAGMENTATION;
pub const VK_ERROR_NOT_PERMITTED_EXT: VkResult = VK_ERROR_NOT_PERMITTED;
pub const VK_ERROR_NOT_PERMITTED_KHR: VkResult = VK_ERROR_NOT_PERMITTED;
pub const VK_ERROR_PRESENT_TIMING_QUEUE_FULL_EXT: VkResult = -1000208000;
pub const VK_ERROR_INVALID_DEVICE_ADDRESS_EXT: VkResult = VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS;
pub const VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT: VkResult = -1000255000;
pub const VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS_KHR: VkResult = VK_ERROR_INVALID_OPAQUE_CAPTURE_ADDRESS;
pub const VK_THREAD_IDLE_KHR: VkResult = 1000268000;
pub const VK_THREAD_DONE_KHR: VkResult = 1000268001;
pub const VK_OPERATION_DEFERRED_KHR: VkResult = 1000268002;
pub const VK_OPERATION_NOT_DEFERRED_KHR: VkResult = 1000268003;
pub const VK_PIPELINE_COMPILE_REQUIRED_EXT: VkResult = VK_PIPELINE_COMPILE_REQUIRED;
pub const VK_ERROR_PIPELINE_COMPILE_REQUIRED_EXT: VkResult = VK_PIPELINE_COMPILE_REQUIRED;
pub const VK_ERROR_INVALID_VIDEO_STD_PARAMETERS_KHR: VkResult = -1000299000;
pub const VK_ERROR_COMPRESSION_EXHAUSTED_EXT: VkResult = -1000338000;
pub const VK_INCOMPATIBLE_SHADER_BINARY_EXT: VkResult = 1000482000;
pub const VK_ERROR_INCOMPATIBLE_SHADER_BINARY_EXT: VkResult = VK_INCOMPATIBLE_SHADER_BINARY_EXT;
pub const VK_PIPELINE_BINARY_MISSING_KHR: VkResult = 1000483000;
pub const VK_ERROR_NOT_ENOUGH_SPACE_KHR: VkResult = -1000483000;
pub type VkSampleCountFlagBits = u32;
pub const VK_SAMPLE_COUNT_1_BIT: VkSampleCountFlagBits = 0x1;
pub const VK_SAMPLE_COUNT_2_BIT: VkSampleCountFlagBits = 0x2;
pub const VK_SAMPLE_COUNT_4_BIT: VkSampleCountFlagBits = 0x4;
pub const VK_SAMPLE_COUNT_8_BIT: VkSampleCountFlagBits = 0x8;
pub const VK_SAMPLE_COUNT_16_BIT: VkSampleCountFlagBits = 0x10;
pub const VK_SAMPLE_COUNT_32_BIT: VkSampleCountFlagBits = 0x20;
pub const VK_SAMPLE_COUNT_64_BIT: VkSampleCountFlagBits = 0x40;
pub type VkSampleCountFlags = u32;
pub type VkSampleMask = u32;
pub type VkSampler = u64;
pub type VkSamplerAddressMode = i32;
pub const VK_SAMPLER_ADDRESS_MODE_REPEAT: VkSamplerAddressMode = 0;
pub const VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT: VkSamplerAddressMode = 1;
pub const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: VkSamplerAddressMode = 2;
pub const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_BORDER: VkSamplerAddressMode = 3;
pub const VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE: VkSamplerAddressMode = 4;
pub const VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE_KHR: VkSamplerAddressMode = VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE;
pub type VkSamplerCreateFlagBits = u32;
pub const VK_SAMPLER_CREATE_SUBSAMPLED_BIT_EXT: VkSamplerCreateFlagBits = 0x1;
pub const VK_SAMPLER_CREATE_SUBSAMPLED_COARSE_RECONSTRUCTION_BIT_EXT: VkSamplerCreateFlagBits = 0x2;
pub const VK_SAMPLER_CREATE_DESCRIPTOR_BUFFER_CAPTURE_REPLAY_BIT_EXT: VkSamplerCreateFlagBits = 0x8;
pub const VK_SAMPLER_CREATE_NON_SEAMLESS_CUBE_MAP_BIT_EXT: VkSamplerCreateFlagBits = 0x4;
pub const VK_SAMPLER_CREATE_IMAGE_PROCESSING_BIT_QCOM: VkSamplerCreateFlagBits = 0x10;
pub type VkSamplerCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSamplerCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkSamplerCreateFlags,
    pub magFilter: VkFilter,
    pub minFilter: VkFilter,
    pub mipmapMode: VkSamplerMipmapMode,
    pub addressModeU: VkSamplerAddressMode,
    pub addressModeV: VkSamplerAddressMode,
    pub addressModeW: VkSamplerAddressMode,
    pub mipLodBias: f32,
    pub anisotropyEnable: VkBool32,
    pub maxAnisotropy: f32,
    pub compareEnable: VkBool32,
    pub compareOp: VkCompareOp,
    pub minLod: f32,
    pub maxLod: f32,
    pub borderColor: VkBorderColor,
    pub unnormalizedCoordinates: VkBool32,
}
pub type VkSamplerMipmapMode = i32;
pub const VK_SAMPLER_MIPMAP_MODE_NEAREST: VkSamplerMipmapMode = 0;
pub const VK_SAMPLER_MIPMAP_MODE_LINEAR: VkSamplerMipmapMode = 1;
pub type VkSemaphore = u64;
pub type VkSemaphoreCreateFlagBits = u32;
pub type VkSemaphoreCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSemaphoreCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkSemaphoreCreateFlags,
}
pub type VkShaderModule = u64;
pub type VkShaderModuleCreateFlagBits = u32;
pub type VkShaderModuleCreateFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkShaderModuleCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkShaderModuleCreateFlags,
    pub codeSize: usize,
    pub pCode: *const u32,
}
pub type VkShaderStageFlagBits = u32;
pub const VK_SHADER_STAGE_VERTEX_BIT: VkShaderStageFlagBits = 0x1;
pub const VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT: VkShaderStageFlagBits = 0x2;
pub const VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT: VkShaderStageFlagBits = 0x4;
pub const VK_SHADER_STAGE_GEOMETRY_BIT: VkShaderStageFlagBits = 0x8;
pub const VK_SHADER_STAGE_FRAGMENT_BIT: VkShaderStageFlagBits = 0x10;
pub const VK_SHADER_STAGE_COMPUTE_BIT: VkShaderStageFlagBits = 0x20;
pub const VK_SHADER_STAGE_ALL_GRAPHICS: VkShaderStageFlagBits = 0x1;
pub const VK_SHADER_STAGE_ALL: VkShaderStageFlagBits = 0x7;
pub const VK_SHADER_STAGE_RAYGEN_BIT_KHR: VkShaderStageFlagBits = 0x100;
pub const VK_SHADER_STAGE_ANY_HIT_BIT_KHR: VkShaderStageFlagBits = 0x200;
pub const VK_SHADER_STAGE_CLOSEST_HIT_BIT_KHR: VkShaderStageFlagBits = 0x400;
pub const VK_SHADER_STAGE_MISS_BIT_KHR: VkShaderStageFlagBits = 0x800;
pub const VK_SHADER_STAGE_INTERSECTION_BIT_KHR: VkShaderStageFlagBits = 0x1000;
pub const VK_SHADER_STAGE_CALLABLE_BIT_KHR: VkShaderStageFlagBits = 0x2000;
pub const VK_SHADER_STAGE_RAYGEN_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_RAYGEN_BIT_KHR;
pub const VK_SHADER_STAGE_ANY_HIT_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_ANY_HIT_BIT_KHR;
pub const VK_SHADER_STAGE_CLOSEST_HIT_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_CLOSEST_HIT_BIT_KHR;
pub const VK_SHADER_STAGE_MISS_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_MISS_BIT_KHR;
pub const VK_SHADER_STAGE_INTERSECTION_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_INTERSECTION_BIT_KHR;
pub const VK_SHADER_STAGE_CALLABLE_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_CALLABLE_BIT_KHR;
pub const VK_SHADER_STAGE_TASK_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_TASK_BIT_EXT;
pub const VK_SHADER_STAGE_MESH_BIT_NV: VkShaderStageFlagBits = VK_SHADER_STAGE_MESH_BIT_EXT;
pub const VK_SHADER_STAGE_TASK_BIT_EXT: VkShaderStageFlagBits = 0x40;
pub const VK_SHADER_STAGE_MESH_BIT_EXT: VkShaderStageFlagBits = 0x80;
pub const VK_SHADER_STAGE_SUBPASS_SHADING_BIT_HUAWEI: VkShaderStageFlagBits = 0x4000;
pub const VK_SHADER_STAGE_CLUSTER_CULLING_BIT_HUAWEI: VkShaderStageFlagBits = 0x80000;
pub type VkShaderStageFlags = u32;
pub type VkSharingMode = i32;
pub const VK_SHARING_MODE_EXCLUSIVE: VkSharingMode = 0;
pub const VK_SHARING_MODE_CONCURRENT: VkSharingMode = 1;
pub type VkSparseImageFormatFlagBits = u32;
pub const VK_SPARSE_IMAGE_FORMAT_SINGLE_MIPTAIL_BIT: VkSparseImageFormatFlagBits = 0x1;
pub const VK_SPARSE_IMAGE_FORMAT_ALIGNED_MIP_SIZE_BIT: VkSparseImageFormatFlagBits = 0x2;
pub const VK_SPARSE_IMAGE_FORMAT_NONSTANDARD_BLOCK_SIZE_BIT: VkSparseImageFormatFlagBits = 0x4;
pub type VkSparseImageFormatFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSparseImageFormatProperties {
    pub aspectMask: VkImageAspectFlags,
    pub imageGranularity: VkExtent3D,
    pub flags: VkSparseImageFormatFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSparseImageFormatProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub properties: VkSparseImageFormatProperties,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSparseImageMemoryRequirements {
    pub formatProperties: VkSparseImageFormatProperties,
    pub imageMipTailFirstLod: u32,
    pub imageMipTailSize: VkDeviceSize,
    pub imageMipTailOffset: VkDeviceSize,
    pub imageMipTailStride: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSpecializationInfo {
    pub mapEntryCount: u32,
    pub pMapEntries: *const VkSpecializationMapEntry,
    pub dataSize: usize,
    pub pData: *const c_void,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSpecializationMapEntry {
    pub constantID: u32,
    pub offset: u32,
    pub size: usize,
}
pub type VkStencilFaceFlagBits = u32;
pub const VK_STENCIL_FACE_FRONT_BIT: VkStencilFaceFlagBits = 0x1;
pub const VK_STENCIL_FACE_BACK_BIT: VkStencilFaceFlagBits = 0x2;
pub const VK_STENCIL_FACE_FRONT_AND_BACK: VkStencilFaceFlagBits = 0x3;
pub const VK_STENCIL_FRONT_AND_BACK: VkStencilFaceFlagBits = VK_STENCIL_FACE_FRONT_AND_BACK;
pub type VkStencilFaceFlags = u32;
pub type VkStencilOp = i32;
pub const VK_STENCIL_OP_KEEP: VkStencilOp = 0;
pub const VK_STENCIL_OP_ZERO: VkStencilOp = 1;
pub const VK_STENCIL_OP_REPLACE: VkStencilOp = 2;
pub const VK_STENCIL_OP_INCREMENT_AND_CLAMP: VkStencilOp = 3;
pub const VK_STENCIL_OP_DECREMENT_AND_CLAMP: VkStencilOp = 4;
pub const VK_STENCIL_OP_INVERT: VkStencilOp = 5;
pub const VK_STENCIL_OP_INCREMENT_AND_WRAP: VkStencilOp = 6;
pub const VK_STENCIL_OP_DECREMENT_AND_WRAP: VkStencilOp = 7;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkStencilOpState {
    pub failOp: VkStencilOp,
    pub passOp: VkStencilOp,
    pub depthFailOp: VkStencilOp,
    pub compareOp: VkCompareOp,
    pub compareMask: u32,
    pub writeMask: u32,
    pub reference: u32,
}
pub type VkStructureType = i32;
pub const VK_STRUCTURE_TYPE_APPLICATION_INFO: VkStructureType = 0;
pub const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: VkStructureType = 1;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: VkStructureType = 2;
pub const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: VkStructureType = 3;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO: VkStructureType = 4;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: VkStructureType = 5;
pub const VK_STRUCTURE_TYPE_MAPPED_MEMORY_RANGE: VkStructureType = 6;
pub const VK_STRUCTURE_TYPE_BIND_SPARSE_INFO: VkStructureType = 7;
pub const VK_STRUCTURE_TYPE_FENCE_CREATE_INFO: VkStructureType = 8;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO: VkStructureType = 9;
pub const VK_STRUCTURE_TYPE_EVENT_CREATE_INFO: VkStructureType = 10;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO: VkStructureType = 11;
pub const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: VkStructureType = 12;
pub const VK_STRUCTURE_TYPE_BUFFER_VIEW_CREATE_INFO: VkStructureType = 13;
pub const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: VkStructureType = 14;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: VkStructureType = 15;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: VkStructureType = 16;
pub const VK_STRUCTURE_TYPE_PIPELINE_CACHE_CREATE_INFO: VkStructureType = 17;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: VkStructureType = 18;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: VkStructureType = 19;
pub const VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: VkStructureType = 20;
pub const VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_STATE_CREATE_INFO: VkStructureType = 21;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO: VkStructureType = 22;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: VkStructureType = 23;
pub const VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: VkStructureType = 24;
pub const VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO: VkStructureType = 25;
pub const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: VkStructureType = 26;
pub const VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO: VkStructureType = 27;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: VkStructureType = 28;
pub const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: VkStructureType = 29;
pub const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: VkStructureType = 30;
pub const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: VkStructureType = 31;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: VkStructureType = 32;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: VkStructureType = 33;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: VkStructureType = 34;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: VkStructureType = 35;
pub const VK_STRUCTURE_TYPE_COPY_DESCRIPTOR_SET: VkStructureType = 36;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: VkStructureType = 37;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: VkStructureType = 38;
pub const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: VkStructureType = 39;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: VkStructureType = 40;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_INFO: VkStructureType = 41;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: VkStructureType = 42;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: VkStructureType = 43;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER: VkStructureType = 44;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: VkStructureType = 45;
pub const VK_STRUCTURE_TYPE_MEMORY_BARRIER: VkStructureType = 46;
pub const VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO: VkStructureType = 47;
pub const VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO: VkStructureType = 48;
pub const VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_INFO: VkStructureType = 1000157000;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_INFO: VkStructureType = 1000157001;
pub const VK_STRUCTURE_TYPE_MEMORY_DEDICATED_REQUIREMENTS: VkStructureType = 1000127000;
pub const VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO: VkStructureType = 1000127001;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO: VkStructureType = 1000060000;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_COMMAND_BUFFER_BEGIN_INFO: VkStructureType = 1000060004;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_SUBMIT_INFO: VkStructureType = 1000060005;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_BIND_SPARSE_INFO: VkStructureType = 1000060006;
pub const VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_DEVICE_GROUP_INFO: VkStructureType = 1000060013;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_DEVICE_GROUP_INFO: VkStructureType = 1000060014;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GROUP_PROPERTIES: VkStructureType = 1000070000;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_DEVICE_CREATE_INFO: VkStructureType = 1000070001;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_REQUIREMENTS_INFO_2: VkStructureType = 1000146000;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2: VkStructureType = 1000146001;
pub const VK_STRUCTURE_TYPE_IMAGE_SPARSE_MEMORY_REQUIREMENTS_INFO_2: VkStructureType = 1000146002;
pub const VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2: VkStructureType = 1000146003;
pub const VK_STRUCTURE_TYPE_SPARSE_IMAGE_MEMORY_REQUIREMENTS_2: VkStructureType = 1000146004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2: VkStructureType = 1000059000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2: VkStructureType = 1000059001;
pub const VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2: VkStructureType = 1000059002;
pub const VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2: VkStructureType = 1000059003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2: VkStructureType = 1000059004;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_PROPERTIES_2: VkStructureType = 1000059005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PROPERTIES_2: VkStructureType = 1000059006;
pub const VK_STRUCTURE_TYPE_SPARSE_IMAGE_FORMAT_PROPERTIES_2: VkStructureType = 1000059007;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SPARSE_IMAGE_FORMAT_INFO_2: VkStructureType = 1000059008;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO: VkStructureType = 1000117002;
pub const VK_STRUCTURE_TYPE_PROTECTED_SUBMIT_INFO: VkStructureType = 1000145000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES: VkStructureType = 1000145001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROTECTED_MEMORY_PROPERTIES: VkStructureType = 1000145002;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_INFO_2: VkStructureType = 1000145003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO: VkStructureType = 1000071000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES: VkStructureType = 1000071001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO: VkStructureType = 1000071002;
pub const VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES: VkStructureType = 1000071003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES: VkStructureType = 1000071004;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO: VkStructureType = 1000072000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO: VkStructureType = 1000072001;
pub const VK_STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO: VkStructureType = 1000072002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_FENCE_INFO: VkStructureType = 1000112000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_FENCE_PROPERTIES: VkStructureType = 1000112001;
pub const VK_STRUCTURE_TYPE_EXPORT_FENCE_CREATE_INFO: VkStructureType = 1000113000;
pub const VK_STRUCTURE_TYPE_EXPORT_SEMAPHORE_CREATE_INFO: VkStructureType = 1000077000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_SEMAPHORE_INFO: VkStructureType = 1000076000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_SEMAPHORE_PROPERTIES: VkStructureType = 1000076001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_PROPERTIES: VkStructureType = 1000094000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES: VkStructureType = 1000083000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES: VkStructureType = 1000120000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTER_FEATURES: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_UPDATE_TEMPLATE_CREATE_INFO: VkStructureType = 1000085000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_3_PROPERTIES: VkStructureType = 1000168000;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_SUPPORT: VkStructureType = 1000168001;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_CREATE_INFO: VkStructureType = 1000156000;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_INFO: VkStructureType = 1000156001;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_PLANE_MEMORY_INFO: VkStructureType = 1000156002;
pub const VK_STRUCTURE_TYPE_IMAGE_PLANE_MEMORY_REQUIREMENTS_INFO: VkStructureType = 1000156003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES: VkStructureType = 1000156004;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_IMAGE_FORMAT_PROPERTIES: VkStructureType = 1000156005;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_RENDER_PASS_BEGIN_INFO: VkStructureType = 1000060003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_POINT_CLIPPING_PROPERTIES: VkStructureType = 1000117000;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_INPUT_ATTACHMENT_ASPECT_CREATE_INFO: VkStructureType = 1000117001;
pub const VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_DOMAIN_ORIGIN_STATE_CREATE_INFO: VkStructureType = 1000117003;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_MULTIVIEW_CREATE_INFO: VkStructureType = 1000053000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES: VkStructureType = 1000053001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PROPERTIES: VkStructureType = 1000053002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES: VkStructureType = 1000063000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETER_FEATURES: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES: VkStructureType = 49;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_PROPERTIES: VkStructureType = 50;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES: VkStructureType = 51;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_PROPERTIES: VkStructureType = 52;
pub const VK_STRUCTURE_TYPE_IMAGE_FORMAT_LIST_CREATE_INFO: VkStructureType = 1000147000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES: VkStructureType = 1000196000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_MEMORY_MODEL_FEATURES: VkStructureType = 1000211000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES: VkStructureType = 1000261000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES: VkStructureType = 1000207000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_PROPERTIES: VkStructureType = 1000207001;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO: VkStructureType = 1000207002;
pub const VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO: VkStructureType = 1000207003;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO: VkStructureType = 1000207004;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_SIGNAL_INFO: VkStructureType = 1000207005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES: VkStructureType = 1000257000;
pub const VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO: VkStructureType = 1000244001;
pub const VK_STRUCTURE_TYPE_BUFFER_OPAQUE_CAPTURE_ADDRESS_CREATE_INFO: VkStructureType = 1000257002;
pub const VK_STRUCTURE_TYPE_MEMORY_OPAQUE_CAPTURE_ADDRESS_ALLOCATE_INFO: VkStructureType = 1000257003;
pub const VK_STRUCTURE_TYPE_DEVICE_MEMORY_OPAQUE_CAPTURE_ADDRESS_INFO: VkStructureType = 1000257004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_8BIT_STORAGE_FEATURES: VkStructureType = 1000177000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_INT64_FEATURES: VkStructureType = 1000180000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES: VkStructureType = 1000082000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FLOAT_CONTROLS_PROPERTIES: VkStructureType = 1000197000;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO: VkStructureType = 1000161000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES: VkStructureType = 1000161001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_PROPERTIES: VkStructureType = 1000161002;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_ALLOCATE_INFO: VkStructureType = 1000161003;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_LAYOUT_SUPPORT: VkStructureType = 1000161004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCALAR_BLOCK_LAYOUT_FEATURES: VkStructureType = 1000221000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_FILTER_MINMAX_PROPERTIES: VkStructureType = 1000130000;
pub const VK_STRUCTURE_TYPE_SAMPLER_REDUCTION_MODE_CREATE_INFO: VkStructureType = 1000130001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFORM_BUFFER_STANDARD_LAYOUT_FEATURES: VkStructureType = 1000253000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_EXTENDED_TYPES_FEATURES: VkStructureType = 1000175000;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_2: VkStructureType = 1000109000;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_2: VkStructureType = 1000109001;
pub const VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_2: VkStructureType = 1000109002;
pub const VK_STRUCTURE_TYPE_SUBPASS_DEPENDENCY_2: VkStructureType = 1000109003;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO_2: VkStructureType = 1000109004;
pub const VK_STRUCTURE_TYPE_SUBPASS_BEGIN_INFO: VkStructureType = 1000109005;
pub const VK_STRUCTURE_TYPE_SUBPASS_END_INFO: VkStructureType = 1000109006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_STENCIL_RESOLVE_PROPERTIES: VkStructureType = 1000199000;
pub const VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_DEPTH_STENCIL_RESOLVE: VkStructureType = 1000199001;
pub const VK_STRUCTURE_TYPE_IMAGE_STENCIL_USAGE_CREATE_INFO: VkStructureType = 1000246000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGELESS_FRAMEBUFFER_FEATURES: VkStructureType = 1000108000;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENTS_CREATE_INFO: VkStructureType = 1000108001;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENT_IMAGE_INFO: VkStructureType = 1000108002;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_ATTACHMENT_BEGIN_INFO: VkStructureType = 1000108003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SEPARATE_DEPTH_STENCIL_LAYOUTS_FEATURES: VkStructureType = 1000241000;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_STENCIL_LAYOUT: VkStructureType = 1000241001;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_STENCIL_LAYOUT: VkStructureType = 1000241002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES: VkStructureType = 53;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_PROPERTIES: VkStructureType = 54;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TOOL_PROPERTIES: VkStructureType = 1000245000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIVATE_DATA_FEATURES: VkStructureType = 1000295000;
pub const VK_STRUCTURE_TYPE_DEVICE_PRIVATE_DATA_CREATE_INFO: VkStructureType = 1000295001;
pub const VK_STRUCTURE_TYPE_PRIVATE_DATA_SLOT_CREATE_INFO: VkStructureType = 1000295002;
pub const VK_STRUCTURE_TYPE_MEMORY_BARRIER_2: VkStructureType = 1000314000;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER_2: VkStructureType = 1000314001;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2: VkStructureType = 1000314002;
pub const VK_STRUCTURE_TYPE_DEPENDENCY_INFO: VkStructureType = 1000314003;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO_2: VkStructureType = 1000314004;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO: VkStructureType = 1000314005;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_SUBMIT_INFO: VkStructureType = 1000314006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES: VkStructureType = 1000314007;
pub const VK_STRUCTURE_TYPE_COPY_BUFFER_INFO_2: VkStructureType = 1000337000;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_INFO_2: VkStructureType = 1000337001;
pub const VK_STRUCTURE_TYPE_COPY_BUFFER_TO_IMAGE_INFO_2: VkStructureType = 1000337002;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_BUFFER_INFO_2: VkStructureType = 1000337003;
pub const VK_STRUCTURE_TYPE_BUFFER_COPY_2: VkStructureType = 1000337006;
pub const VK_STRUCTURE_TYPE_IMAGE_COPY_2: VkStructureType = 1000337007;
pub const VK_STRUCTURE_TYPE_BUFFER_IMAGE_COPY_2: VkStructureType = 1000337009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_HDR_FEATURES: VkStructureType = 1000066000;
pub const VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_3: VkStructureType = 1000360000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_FEATURES: VkStructureType = 1000413000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_PROPERTIES: VkStructureType = 1000413001;
pub const VK_STRUCTURE_TYPE_DEVICE_BUFFER_MEMORY_REQUIREMENTS: VkStructureType = 1000413002;
pub const VK_STRUCTURE_TYPE_DEVICE_IMAGE_MEMORY_REQUIREMENTS: VkStructureType = 1000413003;
pub const VK_STRUCTURE_TYPE_PIPELINE_CREATION_FEEDBACK_CREATE_INFO: VkStructureType = 1000192000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TERMINATE_INVOCATION_FEATURES: VkStructureType = 1000215000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DEMOTE_TO_HELPER_INVOCATION_FEATURES: VkStructureType = 1000276000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES: VkStructureType = 1000297000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ZERO_INITIALIZE_WORKGROUP_MEMORY_FEATURES: VkStructureType = 1000325000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ROBUSTNESS_FEATURES: VkStructureType = 1000335000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_PROPERTIES: VkStructureType = 1000225000;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO: VkStructureType = 1000225001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_FEATURES: VkStructureType = 1000225002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_FEATURES: VkStructureType = 1000138000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_PROPERTIES: VkStructureType = 1000138001;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_INLINE_UNIFORM_BLOCK: VkStructureType = 1000138002;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_INLINE_UNIFORM_BLOCK_CREATE_INFO: VkStructureType = 1000138003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_FEATURES: VkStructureType = 1000280000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_PROPERTIES: VkStructureType = 1000280001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXEL_BUFFER_ALIGNMENT_PROPERTIES: VkStructureType = 1000281001;
pub const VK_STRUCTURE_TYPE_BLIT_IMAGE_INFO_2: VkStructureType = 1000337004;
pub const VK_STRUCTURE_TYPE_RESOLVE_IMAGE_INFO_2: VkStructureType = 1000337005;
pub const VK_STRUCTURE_TYPE_IMAGE_BLIT_2: VkStructureType = 1000337008;
pub const VK_STRUCTURE_TYPE_IMAGE_RESOLVE_2: VkStructureType = 1000337010;
pub const VK_STRUCTURE_TYPE_RENDERING_INFO: VkStructureType = 1000044000;
pub const VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO: VkStructureType = 1000044001;
pub const VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO: VkStructureType = 1000044002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES: VkStructureType = 1000044003;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_RENDERING_INFO: VkStructureType = 1000044004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_4_FEATURES: VkStructureType = 55;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_4_PROPERTIES: VkStructureType = 56;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_GLOBAL_PRIORITY_CREATE_INFO: VkStructureType = 1000174000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GLOBAL_PRIORITY_QUERY_FEATURES: VkStructureType = 1000388000;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_GLOBAL_PRIORITY_PROPERTIES: VkStructureType = 1000388001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INDEX_TYPE_UINT8_FEATURES: VkStructureType = 1000265000;
pub const VK_STRUCTURE_TYPE_MEMORY_MAP_INFO: VkStructureType = 1000271000;
pub const VK_STRUCTURE_TYPE_MEMORY_UNMAP_INFO: VkStructureType = 1000271001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_FEATURES: VkStructureType = 1000470000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_PROPERTIES: VkStructureType = 1000470001;
pub const VK_STRUCTURE_TYPE_DEVICE_IMAGE_SUBRESOURCE_INFO: VkStructureType = 1000470004;
pub const VK_STRUCTURE_TYPE_SUBRESOURCE_LAYOUT_2: VkStructureType = 1000338002;
pub const VK_STRUCTURE_TYPE_IMAGE_SUBRESOURCE_2: VkStructureType = 1000338003;
pub const VK_STRUCTURE_TYPE_BUFFER_USAGE_FLAGS_2_CREATE_INFO: VkStructureType = 1000470006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_FEATURES: VkStructureType = 1000545000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_PROPERTIES: VkStructureType = 1000545001;
pub const VK_STRUCTURE_TYPE_BIND_MEMORY_STATUS: VkStructureType = 1000545002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_FEATURES: VkStructureType = 1000270000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_PROPERTIES: VkStructureType = 1000270001;
pub const VK_STRUCTURE_TYPE_MEMORY_TO_IMAGE_COPY: VkStructureType = 1000270002;
pub const VK_STRUCTURE_TYPE_IMAGE_TO_MEMORY_COPY: VkStructureType = 1000270003;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_MEMORY_INFO: VkStructureType = 1000270004;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_TO_IMAGE_INFO: VkStructureType = 1000270005;
pub const VK_STRUCTURE_TYPE_HOST_IMAGE_LAYOUT_TRANSITION_INFO: VkStructureType = 1000270006;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_IMAGE_INFO: VkStructureType = 1000270007;
pub const VK_STRUCTURE_TYPE_SUBRESOURCE_HOST_MEMCPY_SIZE: VkStructureType = 1000270008;
pub const VK_STRUCTURE_TYPE_HOST_IMAGE_COPY_DEVICE_PERFORMANCE_QUERY: VkStructureType = 1000270009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_ROTATE_FEATURES: VkStructureType = 1000416000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT_CONTROLS_2_FEATURES: VkStructureType = 1000528000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_EXPECT_ASSUME_FEATURES: VkStructureType = 1000544000;
pub const VK_STRUCTURE_TYPE_PIPELINE_CREATE_FLAGS_2_CREATE_INFO: VkStructureType = 1000470005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_DESCRIPTOR_PROPERTIES: VkStructureType = 1000080000;
pub const VK_STRUCTURE_TYPE_BIND_DESCRIPTOR_SETS_INFO: VkStructureType = 1000545003;
pub const VK_STRUCTURE_TYPE_PUSH_CONSTANTS_INFO: VkStructureType = 1000545004;
pub const VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_INFO: VkStructureType = 1000545005;
pub const VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_WITH_TEMPLATE_INFO: VkStructureType = 1000545006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_PROTECTED_ACCESS_FEATURES: VkStructureType = 1000466000;
pub const VK_STRUCTURE_TYPE_PIPELINE_ROBUSTNESS_CREATE_INFO: VkStructureType = 1000068000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_FEATURES: VkStructureType = 1000068001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_PROPERTIES: VkStructureType = 1000068002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES: VkStructureType = 1000259000;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_LINE_STATE_CREATE_INFO: VkStructureType = 1000259001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_PROPERTIES: VkStructureType = 1000259002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_PROPERTIES: VkStructureType = 1000525000;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_DIVISOR_STATE_CREATE_INFO: VkStructureType = 1000190001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_FEATURES: VkStructureType = 1000190002;
pub const VK_STRUCTURE_TYPE_RENDERING_AREA_INFO: VkStructureType = 1000470003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_LOCAL_READ_FEATURES: VkStructureType = 1000232000;
pub const VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_LOCATION_INFO: VkStructureType = 1000232001;
pub const VK_STRUCTURE_TYPE_RENDERING_INPUT_ATTACHMENT_INDEX_INFO: VkStructureType = 1000232002;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000001000;
pub const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: VkStructureType = 1000001001;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_PRESENT_CAPABILITIES_KHR: VkStructureType = 1000060007;
pub const VK_STRUCTURE_TYPE_IMAGE_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000060008;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_SWAPCHAIN_INFO_KHR: VkStructureType = 1000060009;
pub const VK_STRUCTURE_TYPE_ACQUIRE_NEXT_IMAGE_INFO_KHR: VkStructureType = 1000060010;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_PRESENT_INFO_KHR: VkStructureType = 1000060011;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000060012;
pub const VK_STRUCTURE_TYPE_DISPLAY_MODE_CREATE_INFO_KHR: VkStructureType = 1000002000;
pub const VK_STRUCTURE_TYPE_DISPLAY_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000002001;
pub const VK_STRUCTURE_TYPE_DISPLAY_PRESENT_INFO_KHR: VkStructureType = 1000003000;
pub const VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000004000;
pub const VK_STRUCTURE_TYPE_XCB_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000005000;
pub const VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000006000;
pub const VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000008000;
pub const VK_STRUCTURE_TYPE_WIN32_SURFACE_CREATE_INFO_KHR: VkStructureType = 1000009000;
pub const VK_STRUCTURE_TYPE_DEBUG_REPORT_CALLBACK_CREATE_INFO_EXT: VkStructureType = 1000011000;
pub const VK_STRUCTURE_TYPE_DEBUG_REPORT_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DEBUG_REPORT_CALLBACK_CREATE_INFO_EXT;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_RASTERIZATION_ORDER_AMD: VkStructureType = 1000018000;
pub const VK_STRUCTURE_TYPE_DEBUG_MARKER_OBJECT_NAME_INFO_EXT: VkStructureType = 1000022000;
pub const VK_STRUCTURE_TYPE_DEBUG_MARKER_OBJECT_TAG_INFO_EXT: VkStructureType = 1000022001;
pub const VK_STRUCTURE_TYPE_DEBUG_MARKER_MARKER_INFO_EXT: VkStructureType = 1000022002;
pub const VK_STRUCTURE_TYPE_VIDEO_PROFILE_INFO_KHR: VkStructureType = 1000023000;
pub const VK_STRUCTURE_TYPE_VIDEO_CAPABILITIES_KHR: VkStructureType = 1000023001;
pub const VK_STRUCTURE_TYPE_VIDEO_PICTURE_RESOURCE_INFO_KHR: VkStructureType = 1000023002;
pub const VK_STRUCTURE_TYPE_VIDEO_SESSION_MEMORY_REQUIREMENTS_KHR: VkStructureType = 1000023003;
pub const VK_STRUCTURE_TYPE_BIND_VIDEO_SESSION_MEMORY_INFO_KHR: VkStructureType = 1000023004;
pub const VK_STRUCTURE_TYPE_VIDEO_SESSION_CREATE_INFO_KHR: VkStructureType = 1000023005;
pub const VK_STRUCTURE_TYPE_VIDEO_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000023006;
pub const VK_STRUCTURE_TYPE_VIDEO_SESSION_PARAMETERS_UPDATE_INFO_KHR: VkStructureType = 1000023007;
pub const VK_STRUCTURE_TYPE_VIDEO_BEGIN_CODING_INFO_KHR: VkStructureType = 1000023008;
pub const VK_STRUCTURE_TYPE_VIDEO_END_CODING_INFO_KHR: VkStructureType = 1000023009;
pub const VK_STRUCTURE_TYPE_VIDEO_CODING_CONTROL_INFO_KHR: VkStructureType = 1000023010;
pub const VK_STRUCTURE_TYPE_VIDEO_REFERENCE_SLOT_INFO_KHR: VkStructureType = 1000023011;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_VIDEO_PROPERTIES_KHR: VkStructureType = 1000023012;
pub const VK_STRUCTURE_TYPE_VIDEO_PROFILE_LIST_INFO_KHR: VkStructureType = 1000023013;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_FORMAT_INFO_KHR: VkStructureType = 1000023014;
pub const VK_STRUCTURE_TYPE_VIDEO_FORMAT_PROPERTIES_KHR: VkStructureType = 1000023015;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_QUERY_RESULT_STATUS_PROPERTIES_KHR: VkStructureType = 1000023016;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_INFO_KHR: VkStructureType = 1000024000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_CAPABILITIES_KHR: VkStructureType = 1000024001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_USAGE_INFO_KHR: VkStructureType = 1000024002;
pub const VK_STRUCTURE_TYPE_DEDICATED_ALLOCATION_IMAGE_CREATE_INFO_NV: VkStructureType = 1000026000;
pub const VK_STRUCTURE_TYPE_DEDICATED_ALLOCATION_BUFFER_CREATE_INFO_NV: VkStructureType = 1000026001;
pub const VK_STRUCTURE_TYPE_DEDICATED_ALLOCATION_MEMORY_ALLOCATE_INFO_NV: VkStructureType = 1000026002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TRANSFORM_FEEDBACK_FEATURES_EXT: VkStructureType = 1000028000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TRANSFORM_FEEDBACK_PROPERTIES_EXT: VkStructureType = 1000028001;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_STREAM_CREATE_INFO_EXT: VkStructureType = 1000028002;
pub const VK_STRUCTURE_TYPE_CU_MODULE_CREATE_INFO_NVX: VkStructureType = 1000029000;
pub const VK_STRUCTURE_TYPE_CU_FUNCTION_CREATE_INFO_NVX: VkStructureType = 1000029001;
pub const VK_STRUCTURE_TYPE_CU_LAUNCH_INFO_NVX: VkStructureType = 1000029002;
pub const VK_STRUCTURE_TYPE_CU_MODULE_TEXTURING_MODE_CREATE_INFO_NVX: VkStructureType = 1000029004;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_HANDLE_INFO_NVX: VkStructureType = 1000030000;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_ADDRESS_PROPERTIES_NVX: VkStructureType = 1000030001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_CAPABILITIES_KHR: VkStructureType = 1000038000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000038001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_SESSION_PARAMETERS_ADD_INFO_KHR: VkStructureType = 1000038002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_PICTURE_INFO_KHR: VkStructureType = 1000038003;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_DPB_SLOT_INFO_KHR: VkStructureType = 1000038004;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_NALU_SLICE_INFO_KHR: VkStructureType = 1000038005;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_GOP_REMAINING_FRAME_INFO_KHR: VkStructureType = 1000038006;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_PROFILE_INFO_KHR: VkStructureType = 1000038007;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_RATE_CONTROL_INFO_KHR: VkStructureType = 1000038008;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_RATE_CONTROL_LAYER_INFO_KHR: VkStructureType = 1000038009;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_SESSION_CREATE_INFO_KHR: VkStructureType = 1000038010;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_QUALITY_LEVEL_PROPERTIES_KHR: VkStructureType = 1000038011;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_SESSION_PARAMETERS_GET_INFO_KHR: VkStructureType = 1000038012;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_SESSION_PARAMETERS_FEEDBACK_INFO_KHR: VkStructureType = 1000038013;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_CAPABILITIES_KHR: VkStructureType = 1000039000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000039001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_SESSION_PARAMETERS_ADD_INFO_KHR: VkStructureType = 1000039002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_PICTURE_INFO_KHR: VkStructureType = 1000039003;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_DPB_SLOT_INFO_KHR: VkStructureType = 1000039004;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_NALU_SLICE_SEGMENT_INFO_KHR: VkStructureType = 1000039005;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_GOP_REMAINING_FRAME_INFO_KHR: VkStructureType = 1000039006;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_PROFILE_INFO_KHR: VkStructureType = 1000039007;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_RATE_CONTROL_INFO_KHR: VkStructureType = 1000039009;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_RATE_CONTROL_LAYER_INFO_KHR: VkStructureType = 1000039010;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_SESSION_CREATE_INFO_KHR: VkStructureType = 1000039011;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_QUALITY_LEVEL_PROPERTIES_KHR: VkStructureType = 1000039012;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_SESSION_PARAMETERS_GET_INFO_KHR: VkStructureType = 1000039013;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_SESSION_PARAMETERS_FEEDBACK_INFO_KHR: VkStructureType = 1000039014;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_CAPABILITIES_KHR: VkStructureType = 1000040000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_PICTURE_INFO_KHR: VkStructureType = 1000040001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_PROFILE_INFO_KHR: VkStructureType = 1000040003;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000040004;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_SESSION_PARAMETERS_ADD_INFO_KHR: VkStructureType = 1000040005;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_DPB_SLOT_INFO_KHR: VkStructureType = 1000040006;
pub const VK_STRUCTURE_TYPE_TEXTURE_LOD_GATHER_FORMAT_PROPERTIES_AMD: VkStructureType = 1000041000;
pub const VK_STRUCTURE_TYPE_RENDERING_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_INFO;
pub const VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO;
pub const VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_RENDERING_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_RENDERING_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_RENDERING_INFO;
pub const VK_STRUCTURE_TYPE_STREAM_DESCRIPTOR_SURFACE_CREATE_INFO_GGP: VkStructureType = 1000049000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CORNER_SAMPLED_IMAGE_FEATURES_NV: VkStructureType = 1000050000;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_MULTIVIEW_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDER_PASS_MULTIVIEW_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PROPERTIES;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO_NV: VkStructureType = 1000056000;
pub const VK_STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO_NV: VkStructureType = 1000056001;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_WIN32_HANDLE_INFO_NV: VkStructureType = 1000057000;
pub const VK_STRUCTURE_TYPE_EXPORT_MEMORY_WIN32_HANDLE_INFO_NV: VkStructureType = 1000057001;
pub const VK_STRUCTURE_TYPE_WIN32_KEYED_MUTEX_ACQUIRE_RELEASE_INFO_NV: VkStructureType = 1000058000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_QUEUE_FAMILY_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_SPARSE_IMAGE_FORMAT_PROPERTIES_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SPARSE_IMAGE_FORMAT_PROPERTIES_2;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SPARSE_IMAGE_FORMAT_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SPARSE_IMAGE_FORMAT_INFO_2;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_FLAGS_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_RENDER_PASS_BEGIN_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_GROUP_RENDER_PASS_BEGIN_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_COMMAND_BUFFER_BEGIN_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_GROUP_COMMAND_BUFFER_BEGIN_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_SUBMIT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_GROUP_SUBMIT_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_BIND_SPARSE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_GROUP_BIND_SPARSE_INFO;
pub const VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_DEVICE_GROUP_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_DEVICE_GROUP_INFO;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_DEVICE_GROUP_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_DEVICE_GROUP_INFO;
pub const VK_STRUCTURE_TYPE_VALIDATION_FLAGS_EXT: VkStructureType = 1000061000;
pub const VK_STRUCTURE_TYPE_VI_SURFACE_CREATE_INFO_NN: VkStructureType = 1000062000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_HDR_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_HDR_FEATURES;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_ASTC_DECODE_MODE_EXT: VkStructureType = 1000067000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ASTC_DECODE_FEATURES_EXT: VkStructureType = 1000067001;
pub const VK_STRUCTURE_TYPE_PIPELINE_ROBUSTNESS_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_ROBUSTNESS_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_ROBUSTNESS_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GROUP_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GROUP_PROPERTIES;
pub const VK_STRUCTURE_TYPE_DEVICE_GROUP_DEVICE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_GROUP_DEVICE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO;
pub const VK_STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_IMAGE_FORMAT_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO;
pub const VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_BUFFER_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXPORT_MEMORY_ALLOCATE_INFO;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000073000;
pub const VK_STRUCTURE_TYPE_EXPORT_MEMORY_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000073001;
pub const VK_STRUCTURE_TYPE_MEMORY_WIN32_HANDLE_PROPERTIES_KHR: VkStructureType = 1000073002;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000073003;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_FD_INFO_KHR: VkStructureType = 1000074000;
pub const VK_STRUCTURE_TYPE_MEMORY_FD_PROPERTIES_KHR: VkStructureType = 1000074001;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_FD_INFO_KHR: VkStructureType = 1000074002;
pub const VK_STRUCTURE_TYPE_WIN32_KEYED_MUTEX_ACQUIRE_RELEASE_INFO_KHR: VkStructureType = 1000075000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_SEMAPHORE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_SEMAPHORE_INFO;
pub const VK_STRUCTURE_TYPE_EXTERNAL_SEMAPHORE_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_SEMAPHORE_PROPERTIES;
pub const VK_STRUCTURE_TYPE_EXPORT_SEMAPHORE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXPORT_SEMAPHORE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_IMPORT_SEMAPHORE_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000078000;
pub const VK_STRUCTURE_TYPE_EXPORT_SEMAPHORE_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000078001;
pub const VK_STRUCTURE_TYPE_D3D12_FENCE_SUBMIT_INFO_KHR: VkStructureType = 1000078002;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_GET_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000078003;
pub const VK_STRUCTURE_TYPE_IMPORT_SEMAPHORE_FD_INFO_KHR: VkStructureType = 1000079000;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_GET_FD_INFO_KHR: VkStructureType = 1000079001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_DESCRIPTOR_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_DESCRIPTOR_PROPERTIES;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_CONDITIONAL_RENDERING_INFO_EXT: VkStructureType = 1000081000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CONDITIONAL_RENDERING_FEATURES_EXT: VkStructureType = 1000081001;
pub const VK_STRUCTURE_TYPE_CONDITIONAL_RENDERING_BEGIN_INFO_EXT: VkStructureType = 1000081002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FLOAT16_INT8_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES;
pub const VK_STRUCTURE_TYPE_PRESENT_REGIONS_KHR: VkStructureType = 1000084000;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_UPDATE_TEMPLATE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_UPDATE_TEMPLATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_W_SCALING_STATE_CREATE_INFO_NV: VkStructureType = 1000087000;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_2_EXT: VkStructureType = 1000090000;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES2_EXT: VkStructureType = VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_2_EXT;
pub const VK_STRUCTURE_TYPE_DISPLAY_POWER_INFO_EXT: VkStructureType = 1000091000;
pub const VK_STRUCTURE_TYPE_DEVICE_EVENT_INFO_EXT: VkStructureType = 1000091001;
pub const VK_STRUCTURE_TYPE_DISPLAY_EVENT_INFO_EXT: VkStructureType = 1000091002;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_COUNTER_CREATE_INFO_EXT: VkStructureType = 1000091003;
pub const VK_STRUCTURE_TYPE_PRESENT_TIMES_INFO_GOOGLE: VkStructureType = 1000092000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PER_VIEW_ATTRIBUTES_PROPERTIES_NVX: VkStructureType = 1000097000;
pub const VK_STRUCTURE_TYPE_MULTIVIEW_PER_VIEW_ATTRIBUTES_INFO_NVX: VkStructureType = 1000044009;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_SWIZZLE_STATE_CREATE_INFO_NV: VkStructureType = 1000098000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DISCARD_RECTANGLE_PROPERTIES_EXT: VkStructureType = 1000099000;
pub const VK_STRUCTURE_TYPE_PIPELINE_DISCARD_RECTANGLE_STATE_CREATE_INFO_EXT: VkStructureType = 1000099001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CONSERVATIVE_RASTERIZATION_PROPERTIES_EXT: VkStructureType = 1000101000;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_CONSERVATIVE_STATE_CREATE_INFO_EXT: VkStructureType = 1000101001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLIP_ENABLE_FEATURES_EXT: VkStructureType = 1000102000;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_DEPTH_CLIP_STATE_CREATE_INFO_EXT: VkStructureType = 1000102001;
pub const VK_STRUCTURE_TYPE_HDR_METADATA_EXT: VkStructureType = 1000105000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGELESS_FRAMEBUFFER_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGELESS_FRAMEBUFFER_FEATURES;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENTS_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENTS_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENT_IMAGE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_FRAMEBUFFER_ATTACHMENT_IMAGE_INFO;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_ATTACHMENT_BEGIN_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDER_PASS_ATTACHMENT_BEGIN_INFO;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_2;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_2;
pub const VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_2;
pub const VK_STRUCTURE_TYPE_SUBPASS_DEPENDENCY_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBPASS_DEPENDENCY_2;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO_2;
pub const VK_STRUCTURE_TYPE_SUBPASS_BEGIN_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBPASS_BEGIN_INFO;
pub const VK_STRUCTURE_TYPE_SUBPASS_END_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBPASS_END_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RELAXED_LINE_RASTERIZATION_FEATURES_IMG: VkStructureType = 1000110000;
pub const VK_STRUCTURE_TYPE_SHARED_PRESENT_SURFACE_CAPABILITIES_KHR: VkStructureType = 1000111000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_FENCE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_FENCE_INFO;
pub const VK_STRUCTURE_TYPE_EXTERNAL_FENCE_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXTERNAL_FENCE_PROPERTIES;
pub const VK_STRUCTURE_TYPE_EXPORT_FENCE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_EXPORT_FENCE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_IMPORT_FENCE_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000114000;
pub const VK_STRUCTURE_TYPE_EXPORT_FENCE_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000114001;
pub const VK_STRUCTURE_TYPE_FENCE_GET_WIN32_HANDLE_INFO_KHR: VkStructureType = 1000114002;
pub const VK_STRUCTURE_TYPE_IMPORT_FENCE_FD_INFO_KHR: VkStructureType = 1000115000;
pub const VK_STRUCTURE_TYPE_FENCE_GET_FD_INFO_KHR: VkStructureType = 1000115001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PERFORMANCE_QUERY_FEATURES_KHR: VkStructureType = 1000116000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PERFORMANCE_QUERY_PROPERTIES_KHR: VkStructureType = 1000116001;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_PERFORMANCE_CREATE_INFO_KHR: VkStructureType = 1000116002;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_QUERY_SUBMIT_INFO_KHR: VkStructureType = 1000116003;
pub const VK_STRUCTURE_TYPE_ACQUIRE_PROFILING_LOCK_INFO_KHR: VkStructureType = 1000116004;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_COUNTER_KHR: VkStructureType = 1000116005;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_COUNTER_DESCRIPTION_KHR: VkStructureType = 1000116006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_POINT_CLIPPING_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_POINT_CLIPPING_PROPERTIES;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_INPUT_ATTACHMENT_ASPECT_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDER_PASS_INPUT_ATTACHMENT_ASPECT_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_VIEW_USAGE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_DOMAIN_ORIGIN_STATE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_TESSELLATION_DOMAIN_ORIGIN_STATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SURFACE_INFO_2_KHR: VkStructureType = 1000119000;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_2_KHR: VkStructureType = 1000119001;
pub const VK_STRUCTURE_TYPE_SURFACE_FORMAT_2_KHR: VkStructureType = 1000119002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTER_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_DISPLAY_PROPERTIES_2_KHR: VkStructureType = 1000121000;
pub const VK_STRUCTURE_TYPE_DISPLAY_PLANE_PROPERTIES_2_KHR: VkStructureType = 1000121001;
pub const VK_STRUCTURE_TYPE_DISPLAY_MODE_PROPERTIES_2_KHR: VkStructureType = 1000121002;
pub const VK_STRUCTURE_TYPE_DISPLAY_PLANE_INFO_2_KHR: VkStructureType = 1000121003;
pub const VK_STRUCTURE_TYPE_DISPLAY_PLANE_CAPABILITIES_2_KHR: VkStructureType = 1000121004;
pub const VK_STRUCTURE_TYPE_IOS_SURFACE_CREATE_INFO_MVK: VkStructureType = 1000122000;
pub const VK_STRUCTURE_TYPE_MACOS_SURFACE_CREATE_INFO_MVK: VkStructureType = 1000123000;
pub const VK_STRUCTURE_TYPE_MEMORY_DEDICATED_REQUIREMENTS_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_DEDICATED_REQUIREMENTS;
pub const VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_NAME_INFO_EXT: VkStructureType = 1000128000;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_TAG_INFO_EXT: VkStructureType = 1000128001;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT: VkStructureType = 1000128002;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CALLBACK_DATA_EXT: VkStructureType = 1000128003;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT: VkStructureType = 1000128004;
pub const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_USAGE_ANDROID: VkStructureType = 1000129000;
pub const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_PROPERTIES_ANDROID: VkStructureType = 1000129001;
pub const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_FORMAT_PROPERTIES_ANDROID: VkStructureType = 1000129002;
pub const VK_STRUCTURE_TYPE_IMPORT_ANDROID_HARDWARE_BUFFER_INFO_ANDROID: VkStructureType = 1000129003;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_ANDROID_HARDWARE_BUFFER_INFO_ANDROID: VkStructureType = 1000129004;
pub const VK_STRUCTURE_TYPE_EXTERNAL_FORMAT_ANDROID: VkStructureType = 1000129005;
pub const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_FORMAT_PROPERTIES_2_ANDROID: VkStructureType = 1000129006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_FILTER_MINMAX_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_FILTER_MINMAX_PROPERTIES;
pub const VK_STRUCTURE_TYPE_SAMPLER_REDUCTION_MODE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_SAMPLER_REDUCTION_MODE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ENQUEUE_FEATURES_AMDX: VkStructureType = 1000134000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ENQUEUE_PROPERTIES_AMDX: VkStructureType = 1000134001;
pub const VK_STRUCTURE_TYPE_EXECUTION_GRAPH_PIPELINE_SCRATCH_SIZE_AMDX: VkStructureType = 1000134002;
pub const VK_STRUCTURE_TYPE_EXECUTION_GRAPH_PIPELINE_CREATE_INFO_AMDX: VkStructureType = 1000134003;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_NODE_CREATE_INFO_AMDX: VkStructureType = 1000134004;
pub const VK_STRUCTURE_TYPE_TEXEL_BUFFER_DESCRIPTOR_INFO_EXT: VkStructureType = 1000135000;
pub const VK_STRUCTURE_TYPE_IMAGE_DESCRIPTOR_INFO_EXT: VkStructureType = 1000135001;
pub const VK_STRUCTURE_TYPE_RESOURCE_DESCRIPTOR_INFO_EXT: VkStructureType = 1000135002;
pub const VK_STRUCTURE_TYPE_BIND_HEAP_INFO_EXT: VkStructureType = 1000135003;
pub const VK_STRUCTURE_TYPE_PUSH_DATA_INFO_EXT: VkStructureType = 1000135004;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_AND_BINDING_MAPPING_EXT: VkStructureType = 1000135005;
pub const VK_STRUCTURE_TYPE_SHADER_DESCRIPTOR_SET_AND_BINDING_MAPPING_INFO_EXT: VkStructureType = 1000135006;
pub const VK_STRUCTURE_TYPE_OPAQUE_CAPTURE_DATA_CREATE_INFO_EXT: VkStructureType = 1000135007;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_HEAP_PROPERTIES_EXT: VkStructureType = 1000135008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_HEAP_FEATURES_EXT: VkStructureType = 1000135009;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_DESCRIPTOR_HEAP_INFO_EXT: VkStructureType = 1000135010;
pub const VK_STRUCTURE_TYPE_SAMPLER_CUSTOM_BORDER_COLOR_INDEX_CREATE_INFO_EXT: VkStructureType = 1000135011;
pub const VK_STRUCTURE_TYPE_INDIRECT_COMMANDS_LAYOUT_PUSH_DATA_TOKEN_NV: VkStructureType = 1000135012;
pub const VK_STRUCTURE_TYPE_SUBSAMPLED_IMAGE_FORMAT_PROPERTIES_EXT: VkStructureType = 1000135013;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_HEAP_TENSOR_PROPERTIES_ARM: VkStructureType = 1000135014;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_SAMPLE_COUNT_INFO_AMD: VkStructureType = 1000044008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_PROPERTIES;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_INLINE_UNIFORM_BLOCK_EXT: VkStructureType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_INLINE_UNIFORM_BLOCK;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_INLINE_UNIFORM_BLOCK_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_INLINE_UNIFORM_BLOCK_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_BFLOAT16_FEATURES_KHR: VkStructureType = 1000141000;
pub const VK_STRUCTURE_TYPE_SAMPLE_LOCATIONS_INFO_EXT: VkStructureType = 1000143000;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_SAMPLE_LOCATIONS_BEGIN_INFO_EXT: VkStructureType = 1000143001;
pub const VK_STRUCTURE_TYPE_PIPELINE_SAMPLE_LOCATIONS_STATE_CREATE_INFO_EXT: VkStructureType = 1000143002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLE_LOCATIONS_PROPERTIES_EXT: VkStructureType = 1000143003;
pub const VK_STRUCTURE_TYPE_MULTISAMPLE_PROPERTIES_EXT: VkStructureType = 1000143004;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_REQUIREMENTS_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_REQUIREMENTS_INFO_2;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_REQUIREMENTS_INFO_2;
pub const VK_STRUCTURE_TYPE_IMAGE_SPARSE_MEMORY_REQUIREMENTS_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_SPARSE_MEMORY_REQUIREMENTS_INFO_2;
pub const VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_REQUIREMENTS_2;
pub const VK_STRUCTURE_TYPE_SPARSE_IMAGE_MEMORY_REQUIREMENTS_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SPARSE_IMAGE_MEMORY_REQUIREMENTS_2;
pub const VK_STRUCTURE_TYPE_IMAGE_FORMAT_LIST_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_FORMAT_LIST_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BLEND_OPERATION_ADVANCED_FEATURES_EXT: VkStructureType = 1000148000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BLEND_OPERATION_ADVANCED_PROPERTIES_EXT: VkStructureType = 1000148001;
pub const VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_ADVANCED_STATE_CREATE_INFO_EXT: VkStructureType = 1000148002;
pub const VK_STRUCTURE_TYPE_PIPELINE_COVERAGE_TO_COLOR_STATE_CREATE_INFO_NV: VkStructureType = 1000149000;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_KHR: VkStructureType = 1000150007;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_GEOMETRY_INFO_KHR: VkStructureType = 1000150000;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_DEVICE_ADDRESS_INFO_KHR: VkStructureType = 1000150002;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_AABBS_DATA_KHR: VkStructureType = 1000150003;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_INSTANCES_DATA_KHR: VkStructureType = 1000150004;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_TRIANGLES_DATA_KHR: VkStructureType = 1000150005;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_KHR: VkStructureType = 1000150006;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_VERSION_INFO_KHR: VkStructureType = 1000150009;
pub const VK_STRUCTURE_TYPE_COPY_ACCELERATION_STRUCTURE_INFO_KHR: VkStructureType = 1000150010;
pub const VK_STRUCTURE_TYPE_COPY_ACCELERATION_STRUCTURE_TO_MEMORY_INFO_KHR: VkStructureType = 1000150011;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_TO_ACCELERATION_STRUCTURE_INFO_KHR: VkStructureType = 1000150012;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_FEATURES_KHR: VkStructureType = 1000150013;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ACCELERATION_STRUCTURE_PROPERTIES_KHR: VkStructureType = 1000150014;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_KHR: VkStructureType = 1000150017;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_BUILD_SIZES_INFO_KHR: VkStructureType = 1000150020;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_FEATURES_KHR: VkStructureType = 1000347000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PIPELINE_PROPERTIES_KHR: VkStructureType = 1000347001;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_PIPELINE_CREATE_INFO_KHR: VkStructureType = 1000150015;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_SHADER_GROUP_CREATE_INFO_KHR: VkStructureType = 1000150016;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_PIPELINE_INTERFACE_CREATE_INFO_KHR: VkStructureType = 1000150018;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_QUERY_FEATURES_KHR: VkStructureType = 1000348013;
pub const VK_STRUCTURE_TYPE_PIPELINE_COVERAGE_MODULATION_STATE_CREATE_INFO_NV: VkStructureType = 1000152000;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_SAMPLE_COUNT_INFO_NV: VkStructureType = VK_STRUCTURE_TYPE_ATTACHMENT_SAMPLE_COUNT_INFO_AMD;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SM_BUILTINS_FEATURES_NV: VkStructureType = 1000154000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SM_BUILTINS_PROPERTIES_NV: VkStructureType = 1000154001;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_INFO;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_PLANE_MEMORY_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_IMAGE_PLANE_MEMORY_INFO;
pub const VK_STRUCTURE_TYPE_IMAGE_PLANE_MEMORY_REQUIREMENTS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_PLANE_MEMORY_REQUIREMENTS_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_IMAGE_FORMAT_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_IMAGE_FORMAT_PROPERTIES;
pub const VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_BUFFER_MEMORY_INFO;
pub const VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_IMAGE_MEMORY_INFO;
pub const VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT: VkStructureType = 1000158000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT: VkStructureType = 1000158002;
pub const VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_LIST_CREATE_INFO_EXT: VkStructureType = 1000158003;
pub const VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_EXPLICIT_CREATE_INFO_EXT: VkStructureType = 1000158004;
pub const VK_STRUCTURE_TYPE_IMAGE_DRM_FORMAT_MODIFIER_PROPERTIES_EXT: VkStructureType = 1000158005;
pub const VK_STRUCTURE_TYPE_DRM_FORMAT_MODIFIER_PROPERTIES_LIST_2_EXT: VkStructureType = 1000158006;
pub const VK_STRUCTURE_TYPE_VALIDATION_CACHE_CREATE_INFO_EXT: VkStructureType = 1000160000;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_VALIDATION_CACHE_CREATE_INFO_EXT: VkStructureType = 1000160001;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_BINDING_FLAGS_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_PROPERTIES;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_ALLOCATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_ALLOCATE_INFO;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_LAYOUT_SUPPORT_EXT: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_VARIABLE_DESCRIPTOR_COUNT_LAYOUT_SUPPORT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PORTABILITY_SUBSET_FEATURES_KHR: VkStructureType = 1000163000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PORTABILITY_SUBSET_PROPERTIES_KHR: VkStructureType = 1000163001;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_SHADING_RATE_IMAGE_STATE_CREATE_INFO_NV: VkStructureType = 1000164000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADING_RATE_IMAGE_FEATURES_NV: VkStructureType = 1000164001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADING_RATE_IMAGE_PROPERTIES_NV: VkStructureType = 1000164002;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_COARSE_SAMPLE_ORDER_STATE_CREATE_INFO_NV: VkStructureType = 1000164005;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_PIPELINE_CREATE_INFO_NV: VkStructureType = 1000165000;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CREATE_INFO_NV: VkStructureType = 1000165001;
pub const VK_STRUCTURE_TYPE_GEOMETRY_NV: VkStructureType = 1000165003;
pub const VK_STRUCTURE_TYPE_GEOMETRY_TRIANGLES_NV: VkStructureType = 1000165004;
pub const VK_STRUCTURE_TYPE_GEOMETRY_AABB_NV: VkStructureType = 1000165005;
pub const VK_STRUCTURE_TYPE_BIND_ACCELERATION_STRUCTURE_MEMORY_INFO_NV: VkStructureType = 1000165006;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_ACCELERATION_STRUCTURE_NV: VkStructureType = 1000165007;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_MEMORY_REQUIREMENTS_INFO_NV: VkStructureType = 1000165008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_PROPERTIES_NV: VkStructureType = 1000165009;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_SHADER_GROUP_CREATE_INFO_NV: VkStructureType = 1000165011;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_INFO_NV: VkStructureType = 1000165012;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_REPRESENTATIVE_FRAGMENT_TEST_FEATURES_NV: VkStructureType = 1000166000;
pub const VK_STRUCTURE_TYPE_PIPELINE_REPRESENTATIVE_FRAGMENT_TEST_STATE_CREATE_INFO_NV: VkStructureType = 1000166001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_3_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_3_PROPERTIES;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_SUPPORT_KHR: VkStructureType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_SUPPORT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_VIEW_IMAGE_FORMAT_INFO_EXT: VkStructureType = 1000170000;
pub const VK_STRUCTURE_TYPE_FILTER_CUBIC_IMAGE_VIEW_IMAGE_FORMAT_PROPERTIES_EXT: VkStructureType = 1000170001;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_GLOBAL_PRIORITY_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_GLOBAL_PRIORITY_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_EXTENDED_TYPES_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_EXTENDED_TYPES_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_8BIT_STORAGE_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_8BIT_STORAGE_FEATURES;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_HOST_POINTER_INFO_EXT: VkStructureType = 1000178000;
pub const VK_STRUCTURE_TYPE_MEMORY_HOST_POINTER_PROPERTIES_EXT: VkStructureType = 1000178001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_MEMORY_HOST_PROPERTIES_EXT: VkStructureType = 1000178002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_INT64_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_INT64_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CLOCK_FEATURES_KHR: VkStructureType = 1000181000;
pub const VK_STRUCTURE_TYPE_PIPELINE_COMPILER_CONTROL_CREATE_INFO_AMD: VkStructureType = 1000183000;
pub const VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CORE_PROPERTIES_AMD: VkStructureType = 1000185000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_CAPABILITIES_KHR: VkStructureType = 1000187000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000187001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_SESSION_PARAMETERS_ADD_INFO_KHR: VkStructureType = 1000187002;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_PROFILE_INFO_KHR: VkStructureType = 1000187003;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_PICTURE_INFO_KHR: VkStructureType = 1000187004;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_DPB_SLOT_INFO_KHR: VkStructureType = 1000187005;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_GLOBAL_PRIORITY_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_GLOBAL_PRIORITY_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GLOBAL_PRIORITY_QUERY_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GLOBAL_PRIORITY_QUERY_FEATURES;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_GLOBAL_PRIORITY_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_QUEUE_FAMILY_GLOBAL_PRIORITY_PROPERTIES;
pub const VK_STRUCTURE_TYPE_DEVICE_MEMORY_OVERALLOCATION_CREATE_INFO_AMD: VkStructureType = 1000189000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_PROPERTIES_EXT: VkStructureType = 1000190000;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_DIVISOR_STATE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_DIVISOR_STATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_FEATURES;
pub const VK_STRUCTURE_TYPE_PRESENT_FRAME_TOKEN_GGP: VkStructureType = 1000191000;
pub const VK_STRUCTURE_TYPE_PIPELINE_CREATION_FEEDBACK_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_CREATION_FEEDBACK_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FLOAT_CONTROLS_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FLOAT_CONTROLS_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_STENCIL_RESOLVE_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_STENCIL_RESOLVE_PROPERTIES;
pub const VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_DEPTH_STENCIL_RESOLVE_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBPASS_DESCRIPTION_DEPTH_STENCIL_RESOLVE;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMPUTE_SHADER_DERIVATIVES_FEATURES_NV: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMPUTE_SHADER_DERIVATIVES_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MESH_SHADER_FEATURES_NV: VkStructureType = 1000202000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MESH_SHADER_PROPERTIES_NV: VkStructureType = 1000202001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_BARYCENTRIC_FEATURES_NV: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_BARYCENTRIC_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_FOOTPRINT_FEATURES_NV: VkStructureType = 1000204000;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_EXCLUSIVE_SCISSOR_STATE_CREATE_INFO_NV: VkStructureType = 1000205000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXCLUSIVE_SCISSOR_FEATURES_NV: VkStructureType = 1000205002;
pub const VK_STRUCTURE_TYPE_CHECKPOINT_DATA_NV: VkStructureType = 1000206000;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_CHECKPOINT_PROPERTIES_NV: VkStructureType = 1000206001;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_CHECKPOINT_PROPERTIES_2_NV: VkStructureType = 1000314008;
pub const VK_STRUCTURE_TYPE_CHECKPOINT_DATA_2_NV: VkStructureType = 1000314009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_PROPERTIES;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SEMAPHORE_WAIT_INFO;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_SIGNAL_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SEMAPHORE_SIGNAL_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_TIMING_FEATURES_EXT: VkStructureType = 1000208000;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_TIMING_PROPERTIES_EXT: VkStructureType = 1000208001;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_TIME_DOMAIN_PROPERTIES_EXT: VkStructureType = 1000208002;
pub const VK_STRUCTURE_TYPE_PRESENT_TIMINGS_INFO_EXT: VkStructureType = 1000208003;
pub const VK_STRUCTURE_TYPE_PRESENT_TIMING_INFO_EXT: VkStructureType = 1000208004;
pub const VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_INFO_EXT: VkStructureType = 1000208005;
pub const VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_PROPERTIES_EXT: VkStructureType = 1000208006;
pub const VK_STRUCTURE_TYPE_PAST_PRESENTATION_TIMING_EXT: VkStructureType = 1000208007;
pub const VK_STRUCTURE_TYPE_PRESENT_TIMING_SURFACE_CAPABILITIES_EXT: VkStructureType = 1000208008;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_CALIBRATED_TIMESTAMP_INFO_EXT: VkStructureType = 1000208009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_FUNCTIONS_2_FEATURES_INTEL: VkStructureType = 1000209000;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_PERFORMANCE_QUERY_CREATE_INFO_INTEL: VkStructureType = 1000210000;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO_INTEL: VkStructureType = VK_STRUCTURE_TYPE_QUERY_POOL_PERFORMANCE_QUERY_CREATE_INFO_INTEL;
pub const VK_STRUCTURE_TYPE_INITIALIZE_PERFORMANCE_API_INFO_INTEL: VkStructureType = 1000210001;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_MARKER_INFO_INTEL: VkStructureType = 1000210002;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_STREAM_MARKER_INFO_INTEL: VkStructureType = 1000210003;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_OVERRIDE_INFO_INTEL: VkStructureType = 1000210004;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_CONFIGURATION_ACQUIRE_INFO_INTEL: VkStructureType = 1000210005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_MEMORY_MODEL_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_MEMORY_MODEL_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PCI_BUS_INFO_PROPERTIES_EXT: VkStructureType = 1000212000;
pub const VK_STRUCTURE_TYPE_DISPLAY_NATIVE_HDR_SURFACE_CAPABILITIES_AMD: VkStructureType = 1000213000;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_DISPLAY_NATIVE_HDR_CREATE_INFO_AMD: VkStructureType = 1000213001;
pub const VK_STRUCTURE_TYPE_IMAGEPIPE_SURFACE_CREATE_INFO_FUCHSIA: VkStructureType = 1000214000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TERMINATE_INVOCATION_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TERMINATE_INVOCATION_FEATURES;
pub const VK_STRUCTURE_TYPE_METAL_SURFACE_CREATE_INFO_EXT: VkStructureType = 1000217000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_FEATURES_EXT: VkStructureType = 1000218000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_PROPERTIES_EXT: VkStructureType = 1000218001;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_FRAGMENT_DENSITY_MAP_CREATE_INFO_EXT: VkStructureType = 1000218002;
pub const VK_STRUCTURE_TYPE_RENDERING_FRAGMENT_DENSITY_MAP_ATTACHMENT_INFO_EXT: VkStructureType = 1000044007;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCALAR_BLOCK_LAYOUT_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCALAR_BLOCK_LAYOUT_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_FEATURES;
pub const VK_STRUCTURE_TYPE_FRAGMENT_SHADING_RATE_ATTACHMENT_INFO_KHR: VkStructureType = 1000226000;
pub const VK_STRUCTURE_TYPE_PIPELINE_FRAGMENT_SHADING_RATE_STATE_CREATE_INFO_KHR: VkStructureType = 1000226001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADING_RATE_PROPERTIES_KHR: VkStructureType = 1000226002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADING_RATE_FEATURES_KHR: VkStructureType = 1000226003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADING_RATE_KHR: VkStructureType = 1000226004;
pub const VK_STRUCTURE_TYPE_RENDERING_FRAGMENT_SHADING_RATE_ATTACHMENT_INFO_KHR: VkStructureType = 1000044006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CORE_PROPERTIES_2_AMD: VkStructureType = 1000227000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COHERENT_MEMORY_FEATURES_AMD: VkStructureType = 1000229000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_LOCAL_READ_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_LOCAL_READ_FEATURES;
pub const VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_LOCATION_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_LOCATION_INFO;
pub const VK_STRUCTURE_TYPE_RENDERING_INPUT_ATTACHMENT_INDEX_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_INPUT_ATTACHMENT_INDEX_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_IMAGE_ATOMIC_INT64_FEATURES_EXT: VkStructureType = 1000234000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_QUAD_CONTROL_FEATURES_KHR: VkStructureType = 1000235000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_BUDGET_PROPERTIES_EXT: VkStructureType = 1000237000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_PRIORITY_FEATURES_EXT: VkStructureType = 1000238000;
pub const VK_STRUCTURE_TYPE_MEMORY_PRIORITY_ALLOCATE_INFO_EXT: VkStructureType = 1000238001;
pub const VK_STRUCTURE_TYPE_SURFACE_PROTECTED_CAPABILITIES_KHR: VkStructureType = 1000239000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEDICATED_ALLOCATION_IMAGE_ALIASING_FEATURES_NV: VkStructureType = 1000240000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SEPARATE_DEPTH_STENCIL_LAYOUTS_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SEPARATE_DEPTH_STENCIL_LAYOUTS_FEATURES;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_STENCIL_LAYOUT_KHR: VkStructureType = VK_STRUCTURE_TYPE_ATTACHMENT_REFERENCE_STENCIL_LAYOUT;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_STENCIL_LAYOUT_KHR: VkStructureType = VK_STRUCTURE_TYPE_ATTACHMENT_DESCRIPTION_STENCIL_LAYOUT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES_EXT: VkStructureType = 1000244000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_ADDRESS_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES_EXT;
pub const VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO;
pub const VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_CREATE_INFO_EXT: VkStructureType = 1000244002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TOOL_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TOOL_PROPERTIES;
pub const VK_STRUCTURE_TYPE_IMAGE_STENCIL_USAGE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_STENCIL_USAGE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_VALIDATION_FEATURES_EXT: VkStructureType = 1000247000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_WAIT_FEATURES_KHR: VkStructureType = 1000248000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_FEATURES_NV: VkStructureType = 1000249000;
pub const VK_STRUCTURE_TYPE_COOPERATIVE_MATRIX_PROPERTIES_NV: VkStructureType = 1000249001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_PROPERTIES_NV: VkStructureType = 1000249002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COVERAGE_REDUCTION_MODE_FEATURES_NV: VkStructureType = 1000250000;
pub const VK_STRUCTURE_TYPE_PIPELINE_COVERAGE_REDUCTION_STATE_CREATE_INFO_NV: VkStructureType = 1000250001;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_MIXED_SAMPLES_COMBINATION_NV: VkStructureType = 1000250002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_INTERLOCK_FEATURES_EXT: VkStructureType = 1000251000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_YCBCR_IMAGE_ARRAYS_FEATURES_EXT: VkStructureType = 1000252000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFORM_BUFFER_STANDARD_LAYOUT_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFORM_BUFFER_STANDARD_LAYOUT_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROVOKING_VERTEX_FEATURES_EXT: VkStructureType = 1000254000;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_PROVOKING_VERTEX_STATE_CREATE_INFO_EXT: VkStructureType = 1000254001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROVOKING_VERTEX_PROPERTIES_EXT: VkStructureType = 1000254002;
pub const VK_STRUCTURE_TYPE_SURFACE_FULL_SCREEN_EXCLUSIVE_INFO_EXT: VkStructureType = 1000255000;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_FULL_SCREEN_EXCLUSIVE_EXT: VkStructureType = 1000255002;
pub const VK_STRUCTURE_TYPE_SURFACE_FULL_SCREEN_EXCLUSIVE_WIN32_INFO_EXT: VkStructureType = 1000255001;
pub const VK_STRUCTURE_TYPE_HEADLESS_SURFACE_CREATE_INFO_EXT: VkStructureType = 1000256000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES;
pub const VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO;
pub const VK_STRUCTURE_TYPE_BUFFER_OPAQUE_CAPTURE_ADDRESS_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_OPAQUE_CAPTURE_ADDRESS_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_MEMORY_OPAQUE_CAPTURE_ADDRESS_ALLOCATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_OPAQUE_CAPTURE_ADDRESS_ALLOCATE_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_MEMORY_OPAQUE_CAPTURE_ADDRESS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_MEMORY_OPAQUE_CAPTURE_ADDRESS_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_LINE_STATE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_LINE_STATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_FLOAT_FEATURES_EXT: VkStructureType = 1000260000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INDEX_TYPE_UINT8_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INDEX_TYPE_UINT8_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_DYNAMIC_STATE_FEATURES_EXT: VkStructureType = 1000267000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_EXECUTABLE_PROPERTIES_FEATURES_KHR: VkStructureType = 1000269000;
pub const VK_STRUCTURE_TYPE_PIPELINE_INFO_KHR: VkStructureType = 1000269001;
pub const VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_PROPERTIES_KHR: VkStructureType = 1000269002;
pub const VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_INFO_KHR: VkStructureType = 1000269003;
pub const VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_STATISTIC_KHR: VkStructureType = 1000269004;
pub const VK_STRUCTURE_TYPE_PIPELINE_EXECUTABLE_INTERNAL_REPRESENTATION_KHR: VkStructureType = 1000269005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HOST_IMAGE_COPY_PROPERTIES;
pub const VK_STRUCTURE_TYPE_MEMORY_TO_IMAGE_COPY_EXT: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_TO_IMAGE_COPY;
pub const VK_STRUCTURE_TYPE_IMAGE_TO_MEMORY_COPY_EXT: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_TO_MEMORY_COPY;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_MEMORY_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_COPY_IMAGE_TO_MEMORY_INFO;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_TO_IMAGE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_COPY_MEMORY_TO_IMAGE_INFO;
pub const VK_STRUCTURE_TYPE_HOST_IMAGE_LAYOUT_TRANSITION_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_HOST_IMAGE_LAYOUT_TRANSITION_INFO;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_IMAGE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_COPY_IMAGE_TO_IMAGE_INFO;
pub const VK_STRUCTURE_TYPE_SUBRESOURCE_HOST_MEMCPY_SIZE_EXT: VkStructureType = VK_STRUCTURE_TYPE_SUBRESOURCE_HOST_MEMCPY_SIZE;
pub const VK_STRUCTURE_TYPE_HOST_IMAGE_COPY_DEVICE_PERFORMANCE_QUERY_EXT: VkStructureType = VK_STRUCTURE_TYPE_HOST_IMAGE_COPY_DEVICE_PERFORMANCE_QUERY;
pub const VK_STRUCTURE_TYPE_MEMORY_MAP_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_MAP_INFO;
pub const VK_STRUCTURE_TYPE_MEMORY_UNMAP_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_UNMAP_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAP_MEMORY_PLACED_FEATURES_EXT: VkStructureType = 1000272000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAP_MEMORY_PLACED_PROPERTIES_EXT: VkStructureType = 1000272001;
pub const VK_STRUCTURE_TYPE_MEMORY_MAP_PLACED_INFO_EXT: VkStructureType = 1000272002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_FLOAT_2_FEATURES_EXT: VkStructureType = 1000273000;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_EXT: VkStructureType = VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_KHR;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_SCALING_CAPABILITIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_SURFACE_PRESENT_SCALING_CAPABILITIES_KHR;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_COMPATIBILITY_EXT: VkStructureType = VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_COMPATIBILITY_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SWAPCHAIN_MAINTENANCE_1_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SWAPCHAIN_MAINTENANCE_1_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_FENCE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_FENCE_INFO_KHR;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODES_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODES_CREATE_INFO_KHR;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODE_INFO_KHR;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_SCALING_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_SCALING_CREATE_INFO_KHR;
pub const VK_STRUCTURE_TYPE_RELEASE_SWAPCHAIN_IMAGES_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_RELEASE_SWAPCHAIN_IMAGES_INFO_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DEMOTE_TO_HELPER_INVOCATION_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_DEMOTE_TO_HELPER_INVOCATION_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_GENERATED_COMMANDS_PROPERTIES_NV: VkStructureType = 1000277000;
pub const VK_STRUCTURE_TYPE_GRAPHICS_SHADER_GROUP_CREATE_INFO_NV: VkStructureType = 1000277001;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_SHADER_GROUPS_CREATE_INFO_NV: VkStructureType = 1000277002;
pub const VK_STRUCTURE_TYPE_INDIRECT_COMMANDS_LAYOUT_TOKEN_NV: VkStructureType = 1000277003;
pub const VK_STRUCTURE_TYPE_INDIRECT_COMMANDS_LAYOUT_CREATE_INFO_NV: VkStructureType = 1000277004;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_INFO_NV: VkStructureType = 1000277005;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_MEMORY_REQUIREMENTS_INFO_NV: VkStructureType = 1000277006;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_GENERATED_COMMANDS_FEATURES_NV: VkStructureType = 1000277007;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INHERITED_VIEWPORT_SCISSOR_FEATURES_NV: VkStructureType = 1000278000;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_VIEWPORT_SCISSOR_INFO_NV: VkStructureType = 1000278001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXEL_BUFFER_ALIGNMENT_FEATURES_EXT: VkStructureType = 1000281000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXEL_BUFFER_ALIGNMENT_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXEL_BUFFER_ALIGNMENT_PROPERTIES;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_INHERITANCE_RENDER_PASS_TRANSFORM_INFO_QCOM: VkStructureType = 1000282000;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_TRANSFORM_BEGIN_INFO_QCOM: VkStructureType = 1000282001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_BIAS_CONTROL_FEATURES_EXT: VkStructureType = 1000283000;
pub const VK_STRUCTURE_TYPE_DEPTH_BIAS_INFO_EXT: VkStructureType = 1000283001;
pub const VK_STRUCTURE_TYPE_DEPTH_BIAS_REPRESENTATION_INFO_EXT: VkStructureType = 1000283002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_MEMORY_REPORT_FEATURES_EXT: VkStructureType = 1000284000;
pub const VK_STRUCTURE_TYPE_DEVICE_DEVICE_MEMORY_REPORT_CREATE_INFO_EXT: VkStructureType = 1000284001;
pub const VK_STRUCTURE_TYPE_DEVICE_MEMORY_REPORT_CALLBACK_DATA_EXT: VkStructureType = 1000284002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_PROPERTIES_KHR;
pub const VK_STRUCTURE_TYPE_SAMPLER_CUSTOM_BORDER_COLOR_CREATE_INFO_EXT: VkStructureType = 1000287000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUSTOM_BORDER_COLOR_PROPERTIES_EXT: VkStructureType = 1000287001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUSTOM_BORDER_COLOR_FEATURES_EXT: VkStructureType = 1000287002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_3D_FEATURES_EXT: VkStructureType = 1000288000;
pub const VK_STRUCTURE_TYPE_PIPELINE_LIBRARY_CREATE_INFO_KHR: VkStructureType = 1000290000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_BARRIER_FEATURES_NV: VkStructureType = 1000292000;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_BARRIER_NV: VkStructureType = 1000292001;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_BARRIER_CREATE_INFO_NV: VkStructureType = 1000292002;
pub const VK_STRUCTURE_TYPE_PRESENT_ID_KHR: VkStructureType = 1000294000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_FEATURES_KHR: VkStructureType = 1000294001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIVATE_DATA_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIVATE_DATA_FEATURES;
pub const VK_STRUCTURE_TYPE_DEVICE_PRIVATE_DATA_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_PRIVATE_DATA_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PRIVATE_DATA_SLOT_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PRIVATE_DATA_SLOT_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_INFO_KHR: VkStructureType = 1000299000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_RATE_CONTROL_INFO_KHR: VkStructureType = 1000299001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_RATE_CONTROL_LAYER_INFO_KHR: VkStructureType = 1000299002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_CAPABILITIES_KHR: VkStructureType = 1000299003;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_USAGE_INFO_KHR: VkStructureType = 1000299004;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_VIDEO_ENCODE_FEEDBACK_CREATE_INFO_KHR: VkStructureType = 1000299005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_ENCODE_QUALITY_LEVEL_INFO_KHR: VkStructureType = 1000299006;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_QUALITY_LEVEL_PROPERTIES_KHR: VkStructureType = 1000299007;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_QUALITY_LEVEL_INFO_KHR: VkStructureType = 1000299008;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_SESSION_PARAMETERS_GET_INFO_KHR: VkStructureType = 1000299009;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_SESSION_PARAMETERS_FEEDBACK_INFO_KHR: VkStructureType = 1000299010;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DIAGNOSTICS_CONFIG_FEATURES_NV: VkStructureType = 1000300000;
pub const VK_STRUCTURE_TYPE_DEVICE_DIAGNOSTICS_CONFIG_CREATE_INFO_NV: VkStructureType = 1000300001;
pub const VK_STRUCTURE_TYPE_CUDA_MODULE_CREATE_INFO_NV: VkStructureType = 1000307000;
pub const VK_STRUCTURE_TYPE_CUDA_FUNCTION_CREATE_INFO_NV: VkStructureType = 1000307001;
pub const VK_STRUCTURE_TYPE_CUDA_LAUNCH_INFO_NV: VkStructureType = 1000307002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUDA_KERNEL_LAUNCH_FEATURES_NV: VkStructureType = 1000307003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUDA_KERNEL_LAUNCH_PROPERTIES_NV: VkStructureType = 1000307004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TILE_SHADING_FEATURES_QCOM: VkStructureType = 1000309000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TILE_SHADING_PROPERTIES_QCOM: VkStructureType = 1000309001;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_TILE_SHADING_CREATE_INFO_QCOM: VkStructureType = 1000309002;
pub const VK_STRUCTURE_TYPE_PER_TILE_BEGIN_INFO_QCOM: VkStructureType = 1000309003;
pub const VK_STRUCTURE_TYPE_PER_TILE_END_INFO_QCOM: VkStructureType = 1000309004;
pub const VK_STRUCTURE_TYPE_DISPATCH_TILE_INFO_QCOM: VkStructureType = 1000309005;
pub const VK_STRUCTURE_TYPE_QUERY_LOW_LATENCY_SUPPORT_NV: VkStructureType = 1000310000;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_OBJECT_CREATE_INFO_EXT: VkStructureType = 1000311000;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_OBJECTS_INFO_EXT: VkStructureType = 1000311001;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_DEVICE_INFO_EXT: VkStructureType = 1000311002;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_COMMAND_QUEUE_INFO_EXT: VkStructureType = 1000311003;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_BUFFER_INFO_EXT: VkStructureType = 1000311004;
pub const VK_STRUCTURE_TYPE_IMPORT_METAL_BUFFER_INFO_EXT: VkStructureType = 1000311005;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_TEXTURE_INFO_EXT: VkStructureType = 1000311006;
pub const VK_STRUCTURE_TYPE_IMPORT_METAL_TEXTURE_INFO_EXT: VkStructureType = 1000311007;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_IO_SURFACE_INFO_EXT: VkStructureType = 1000311008;
pub const VK_STRUCTURE_TYPE_IMPORT_METAL_IO_SURFACE_INFO_EXT: VkStructureType = 1000311009;
pub const VK_STRUCTURE_TYPE_EXPORT_METAL_SHARED_EVENT_INFO_EXT: VkStructureType = 1000311010;
pub const VK_STRUCTURE_TYPE_IMPORT_METAL_SHARED_EVENT_INFO_EXT: VkStructureType = 1000311011;
pub const VK_STRUCTURE_TYPE_MEMORY_BARRIER_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_MEMORY_BARRIER_2;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER_2;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2;
pub const VK_STRUCTURE_TYPE_DEPENDENCY_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEPENDENCY_INFO;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBMIT_INFO_2;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_SEMAPHORE_SUBMIT_INFO;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_SUBMIT_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_SUBMIT_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_BUFFER_PROPERTIES_EXT: VkStructureType = 1000316000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_BUFFER_DENSITY_MAP_PROPERTIES_EXT: VkStructureType = 1000316001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_BUFFER_FEATURES_EXT: VkStructureType = 1000316002;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_ADDRESS_INFO_EXT: VkStructureType = 1000316003;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_GET_INFO_EXT: VkStructureType = 1000316004;
pub const VK_STRUCTURE_TYPE_BUFFER_CAPTURE_DESCRIPTOR_DATA_INFO_EXT: VkStructureType = 1000316005;
pub const VK_STRUCTURE_TYPE_IMAGE_CAPTURE_DESCRIPTOR_DATA_INFO_EXT: VkStructureType = 1000316006;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_CAPTURE_DESCRIPTOR_DATA_INFO_EXT: VkStructureType = 1000316007;
pub const VK_STRUCTURE_TYPE_SAMPLER_CAPTURE_DESCRIPTOR_DATA_INFO_EXT: VkStructureType = 1000316008;
pub const VK_STRUCTURE_TYPE_OPAQUE_CAPTURE_DESCRIPTOR_DATA_CREATE_INFO_EXT: VkStructureType = 1000316010;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_BUFFER_BINDING_INFO_EXT: VkStructureType = 1000316011;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_BUFFER_BINDING_PUSH_DESCRIPTOR_BUFFER_HANDLE_EXT: VkStructureType = 1000316012;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_CAPTURE_DESCRIPTOR_DATA_INFO_EXT: VkStructureType = 1000316009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GRAPHICS_PIPELINE_LIBRARY_FEATURES_EXT: VkStructureType = 1000320000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GRAPHICS_PIPELINE_LIBRARY_PROPERTIES_EXT: VkStructureType = 1000320001;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_LIBRARY_CREATE_INFO_EXT: VkStructureType = 1000320002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_EARLY_AND_LATE_FRAGMENT_TESTS_FEATURES_AMD: VkStructureType = 1000321000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_BARYCENTRIC_FEATURES_KHR: VkStructureType = 1000203000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_BARYCENTRIC_PROPERTIES_KHR: VkStructureType = 1000322000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_UNIFORM_CONTROL_FLOW_FEATURES_KHR: VkStructureType = 1000323000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ZERO_INITIALIZE_WORKGROUP_MEMORY_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ZERO_INITIALIZE_WORKGROUP_MEMORY_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADING_RATE_ENUMS_PROPERTIES_NV: VkStructureType = 1000326000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADING_RATE_ENUMS_FEATURES_NV: VkStructureType = 1000326001;
pub const VK_STRUCTURE_TYPE_PIPELINE_FRAGMENT_SHADING_RATE_ENUM_STATE_CREATE_INFO_NV: VkStructureType = 1000326002;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_MOTION_TRIANGLES_DATA_NV: VkStructureType = 1000327000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_MOTION_BLUR_FEATURES_NV: VkStructureType = 1000327001;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_MOTION_INFO_NV: VkStructureType = 1000327002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MESH_SHADER_FEATURES_EXT: VkStructureType = 1000328000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MESH_SHADER_PROPERTIES_EXT: VkStructureType = 1000328001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_YCBCR_2_PLANE_444_FORMATS_FEATURES_EXT: VkStructureType = 1000330000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_2_FEATURES_EXT: VkStructureType = 1000332000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_2_PROPERTIES_EXT: VkStructureType = 1000332001;
pub const VK_STRUCTURE_TYPE_COPY_COMMAND_TRANSFORM_INFO_QCOM: VkStructureType = 1000333000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ROBUSTNESS_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ROBUSTNESS_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_WORKGROUP_MEMORY_EXPLICIT_LAYOUT_FEATURES_KHR: VkStructureType = 1000336000;
pub const VK_STRUCTURE_TYPE_COPY_BUFFER_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_COPY_BUFFER_INFO_2;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_COPY_IMAGE_INFO_2;
pub const VK_STRUCTURE_TYPE_COPY_BUFFER_TO_IMAGE_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_COPY_BUFFER_TO_IMAGE_INFO_2;
pub const VK_STRUCTURE_TYPE_COPY_IMAGE_TO_BUFFER_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_COPY_IMAGE_TO_BUFFER_INFO_2;
pub const VK_STRUCTURE_TYPE_BLIT_IMAGE_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_BLIT_IMAGE_INFO_2;
pub const VK_STRUCTURE_TYPE_RESOLVE_IMAGE_INFO_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_RESOLVE_IMAGE_INFO_2;
pub const VK_STRUCTURE_TYPE_BUFFER_COPY_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_COPY_2;
pub const VK_STRUCTURE_TYPE_IMAGE_COPY_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_COPY_2;
pub const VK_STRUCTURE_TYPE_IMAGE_BLIT_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_BLIT_2;
pub const VK_STRUCTURE_TYPE_BUFFER_IMAGE_COPY_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_IMAGE_COPY_2;
pub const VK_STRUCTURE_TYPE_IMAGE_RESOLVE_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_RESOLVE_2;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_COMPRESSION_CONTROL_FEATURES_EXT: VkStructureType = 1000338000;
pub const VK_STRUCTURE_TYPE_IMAGE_COMPRESSION_CONTROL_EXT: VkStructureType = 1000338001;
pub const VK_STRUCTURE_TYPE_SUBRESOURCE_LAYOUT_2_EXT: VkStructureType = VK_STRUCTURE_TYPE_SUBRESOURCE_LAYOUT_2;
pub const VK_STRUCTURE_TYPE_IMAGE_SUBRESOURCE_2_EXT: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_SUBRESOURCE_2;
pub const VK_STRUCTURE_TYPE_IMAGE_COMPRESSION_PROPERTIES_EXT: VkStructureType = 1000338004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ATTACHMENT_FEEDBACK_LOOP_LAYOUT_FEATURES_EXT: VkStructureType = 1000339000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_4444_FORMATS_FEATURES_EXT: VkStructureType = 1000340000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FAULT_FEATURES_EXT: VkStructureType = 1000341000;
pub const VK_STRUCTURE_TYPE_DEVICE_FAULT_COUNTS_EXT: VkStructureType = 1000341001;
pub const VK_STRUCTURE_TYPE_DEVICE_FAULT_INFO_EXT: VkStructureType = 1000341002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_FEATURES_ARM: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_FEATURES_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RGBA10X6_FORMATS_FEATURES_EXT: VkStructureType = 1000344000;
pub const VK_STRUCTURE_TYPE_DIRECTFB_SURFACE_CREATE_INFO_EXT: VkStructureType = 1000346000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MUTABLE_DESCRIPTOR_TYPE_FEATURES_VALVE: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MUTABLE_DESCRIPTOR_TYPE_FEATURES_EXT;
pub const VK_STRUCTURE_TYPE_MUTABLE_DESCRIPTOR_TYPE_CREATE_INFO_VALVE: VkStructureType = VK_STRUCTURE_TYPE_MUTABLE_DESCRIPTOR_TYPE_CREATE_INFO_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_INPUT_DYNAMIC_STATE_FEATURES_EXT: VkStructureType = 1000352000;
pub const VK_STRUCTURE_TYPE_VERTEX_INPUT_BINDING_DESCRIPTION_2_EXT: VkStructureType = 1000352001;
pub const VK_STRUCTURE_TYPE_VERTEX_INPUT_ATTRIBUTE_DESCRIPTION_2_EXT: VkStructureType = 1000352002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT: VkStructureType = 1000353000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ADDRESS_BINDING_REPORT_FEATURES_EXT: VkStructureType = 1000354000;
pub const VK_STRUCTURE_TYPE_DEVICE_ADDRESS_BINDING_CALLBACK_DATA_EXT: VkStructureType = 1000354001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLIP_CONTROL_FEATURES_EXT: VkStructureType = 1000355000;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_DEPTH_CLIP_CONTROL_CREATE_INFO_EXT: VkStructureType = 1000355001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIMITIVE_TOPOLOGY_LIST_RESTART_FEATURES_EXT: VkStructureType = 1000356000;
pub const VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_3_KHR: VkStructureType = VK_STRUCTURE_TYPE_FORMAT_PROPERTIES_3;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_MODE_FIFO_LATEST_READY_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_MODE_FIFO_LATEST_READY_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_ZIRCON_HANDLE_INFO_FUCHSIA: VkStructureType = 1000364000;
pub const VK_STRUCTURE_TYPE_MEMORY_ZIRCON_HANDLE_PROPERTIES_FUCHSIA: VkStructureType = 1000364001;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_ZIRCON_HANDLE_INFO_FUCHSIA: VkStructureType = 1000364002;
pub const VK_STRUCTURE_TYPE_IMPORT_SEMAPHORE_ZIRCON_HANDLE_INFO_FUCHSIA: VkStructureType = 1000365000;
pub const VK_STRUCTURE_TYPE_SEMAPHORE_GET_ZIRCON_HANDLE_INFO_FUCHSIA: VkStructureType = 1000365001;
pub const VK_STRUCTURE_TYPE_BUFFER_COLLECTION_CREATE_INFO_FUCHSIA: VkStructureType = 1000366000;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_BUFFER_COLLECTION_FUCHSIA: VkStructureType = 1000366001;
pub const VK_STRUCTURE_TYPE_BUFFER_COLLECTION_IMAGE_CREATE_INFO_FUCHSIA: VkStructureType = 1000366002;
pub const VK_STRUCTURE_TYPE_BUFFER_COLLECTION_PROPERTIES_FUCHSIA: VkStructureType = 1000366003;
pub const VK_STRUCTURE_TYPE_BUFFER_CONSTRAINTS_INFO_FUCHSIA: VkStructureType = 1000366004;
pub const VK_STRUCTURE_TYPE_BUFFER_COLLECTION_BUFFER_CREATE_INFO_FUCHSIA: VkStructureType = 1000366005;
pub const VK_STRUCTURE_TYPE_IMAGE_CONSTRAINTS_INFO_FUCHSIA: VkStructureType = 1000366006;
pub const VK_STRUCTURE_TYPE_IMAGE_FORMAT_CONSTRAINTS_INFO_FUCHSIA: VkStructureType = 1000366007;
pub const VK_STRUCTURE_TYPE_SYSMEM_COLOR_SPACE_FUCHSIA: VkStructureType = 1000366008;
pub const VK_STRUCTURE_TYPE_BUFFER_COLLECTION_CONSTRAINTS_INFO_FUCHSIA: VkStructureType = 1000366009;
pub const VK_STRUCTURE_TYPE_SUBPASS_SHADING_PIPELINE_CREATE_INFO_HUAWEI: VkStructureType = 1000369000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBPASS_SHADING_FEATURES_HUAWEI: VkStructureType = 1000369001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBPASS_SHADING_PROPERTIES_HUAWEI: VkStructureType = 1000369002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INVOCATION_MASK_FEATURES_HUAWEI: VkStructureType = 1000370000;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_REMOTE_ADDRESS_INFO_NV: VkStructureType = 1000371000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_MEMORY_RDMA_FEATURES_NV: VkStructureType = 1000371001;
pub const VK_STRUCTURE_TYPE_PIPELINE_PROPERTIES_IDENTIFIER_EXT: VkStructureType = 1000372000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_PROPERTIES_FEATURES_EXT: VkStructureType = 1000372001;
pub const VK_STRUCTURE_TYPE_PIPELINE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_INFO_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAME_BOUNDARY_FEATURES_EXT: VkStructureType = 1000375000;
pub const VK_STRUCTURE_TYPE_FRAME_BOUNDARY_EXT: VkStructureType = 1000375001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTISAMPLED_RENDER_TO_SINGLE_SAMPLED_FEATURES_EXT: VkStructureType = 1000376000;
pub const VK_STRUCTURE_TYPE_SUBPASS_RESOLVE_PERFORMANCE_QUERY_EXT: VkStructureType = 1000376001;
pub const VK_STRUCTURE_TYPE_MULTISAMPLED_RENDER_TO_SINGLE_SAMPLED_INFO_EXT: VkStructureType = 1000376002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_DYNAMIC_STATE_2_FEATURES_EXT: VkStructureType = 1000377000;
pub const VK_STRUCTURE_TYPE_SCREEN_SURFACE_CREATE_INFO_QNX: VkStructureType = 1000378000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COLOR_WRITE_ENABLE_FEATURES_EXT: VkStructureType = 1000381000;
pub const VK_STRUCTURE_TYPE_PIPELINE_COLOR_WRITE_CREATE_INFO_EXT: VkStructureType = 1000381001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRIMITIVES_GENERATED_QUERY_FEATURES_EXT: VkStructureType = 1000382000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_MAINTENANCE_1_FEATURES_KHR: VkStructureType = 1000386000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_UNTYPED_POINTERS_FEATURES_KHR: VkStructureType = 1000387000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GLOBAL_PRIORITY_QUERY_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_GLOBAL_PRIORITY_QUERY_FEATURES;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_GLOBAL_PRIORITY_PROPERTIES_EXT: VkStructureType = VK_STRUCTURE_TYPE_QUEUE_FAMILY_GLOBAL_PRIORITY_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_ENCODE_RGB_CONVERSION_FEATURES_VALVE: VkStructureType = 1000390000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_RGB_CONVERSION_CAPABILITIES_VALVE: VkStructureType = 1000390001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_PROFILE_RGB_CONVERSION_INFO_VALVE: VkStructureType = 1000390002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_SESSION_RGB_CONVERSION_CREATE_INFO_VALVE: VkStructureType = 1000390003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_VIEW_MIN_LOD_FEATURES_EXT: VkStructureType = 1000391000;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_MIN_LOD_CREATE_INFO_EXT: VkStructureType = 1000391001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTI_DRAW_FEATURES_EXT: VkStructureType = 1000392000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTI_DRAW_PROPERTIES_EXT: VkStructureType = 1000392001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_2D_VIEW_OF_3D_FEATURES_EXT: VkStructureType = 1000393000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TILE_IMAGE_FEATURES_EXT: VkStructureType = 1000395000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_TILE_IMAGE_PROPERTIES_EXT: VkStructureType = 1000395001;
pub const VK_STRUCTURE_TYPE_MICROMAP_BUILD_INFO_EXT: VkStructureType = 1000396000;
pub const VK_STRUCTURE_TYPE_MICROMAP_VERSION_INFO_EXT: VkStructureType = 1000396001;
pub const VK_STRUCTURE_TYPE_COPY_MICROMAP_INFO_EXT: VkStructureType = 1000396002;
pub const VK_STRUCTURE_TYPE_COPY_MICROMAP_TO_MEMORY_INFO_EXT: VkStructureType = 1000396003;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_TO_MICROMAP_INFO_EXT: VkStructureType = 1000396004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPACITY_MICROMAP_FEATURES_EXT: VkStructureType = 1000396005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPACITY_MICROMAP_PROPERTIES_EXT: VkStructureType = 1000396006;
pub const VK_STRUCTURE_TYPE_MICROMAP_CREATE_INFO_EXT: VkStructureType = 1000396007;
pub const VK_STRUCTURE_TYPE_MICROMAP_BUILD_SIZES_INFO_EXT: VkStructureType = 1000396008;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_TRIANGLES_OPACITY_MICROMAP_EXT: VkStructureType = 1000396009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DISPLACEMENT_MICROMAP_FEATURES_NV: VkStructureType = 1000397000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DISPLACEMENT_MICROMAP_PROPERTIES_NV: VkStructureType = 1000397001;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_TRIANGLES_DISPLACEMENT_MICROMAP_NV: VkStructureType = 1000397002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CLUSTER_CULLING_SHADER_FEATURES_HUAWEI: VkStructureType = 1000404000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CLUSTER_CULLING_SHADER_PROPERTIES_HUAWEI: VkStructureType = 1000404001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CLUSTER_CULLING_SHADER_VRS_FEATURES_HUAWEI: VkStructureType = 1000404002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_BORDER_COLOR_SWIZZLE_FEATURES_EXT: VkStructureType = 1000411000;
pub const VK_STRUCTURE_TYPE_SAMPLER_BORDER_COLOR_COMPONENT_MAPPING_CREATE_INFO_EXT: VkStructureType = 1000411001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PAGEABLE_DEVICE_LOCAL_MEMORY_FEATURES_EXT: VkStructureType = 1000412000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_4_PROPERTIES;
pub const VK_STRUCTURE_TYPE_DEVICE_BUFFER_MEMORY_REQUIREMENTS_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_BUFFER_MEMORY_REQUIREMENTS;
pub const VK_STRUCTURE_TYPE_DEVICE_IMAGE_MEMORY_REQUIREMENTS_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_IMAGE_MEMORY_REQUIREMENTS;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CORE_PROPERTIES_ARM: VkStructureType = 1000415000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_ROTATE_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_ROTATE_FEATURES;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_SHADER_CORE_CONTROL_CREATE_INFO_ARM: VkStructureType = 1000417000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCHEDULING_CONTROLS_FEATURES_ARM: VkStructureType = 1000417001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SCHEDULING_CONTROLS_PROPERTIES_ARM: VkStructureType = 1000417002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_SLICED_VIEW_OF_3D_FEATURES_EXT: VkStructureType = 1000418000;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_SLICED_CREATE_INFO_EXT: VkStructureType = 1000418001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_SET_HOST_MAPPING_FEATURES_VALVE: VkStructureType = 1000420000;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_BINDING_REFERENCE_VALVE: VkStructureType = 1000420001;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_HOST_MAPPING_INFO_VALVE: VkStructureType = 1000420002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLAMP_ZERO_ONE_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLAMP_ZERO_ONE_FEATURES_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_NON_SEAMLESS_CUBE_MAP_FEATURES_EXT: VkStructureType = 1000422000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RENDER_PASS_STRIPED_FEATURES_ARM: VkStructureType = 1000424000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RENDER_PASS_STRIPED_PROPERTIES_ARM: VkStructureType = 1000424001;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_STRIPE_BEGIN_INFO_ARM: VkStructureType = 1000424002;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_STRIPE_INFO_ARM: VkStructureType = 1000424003;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_STRIPE_SUBMIT_INFO_ARM: VkStructureType = 1000424004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_FEATURES_QCOM: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_FEATURES_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_PROPERTIES_QCOM: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_PROPERTIES_EXT;
pub const VK_STRUCTURE_TYPE_SUBPASS_FRAGMENT_DENSITY_MAP_OFFSET_END_INFO_QCOM: VkStructureType = VK_STRUCTURE_TYPE_RENDER_PASS_FRAGMENT_DENSITY_MAP_OFFSET_END_INFO_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COPY_MEMORY_INDIRECT_FEATURES_NV: VkStructureType = 1000426000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COPY_MEMORY_INDIRECT_PROPERTIES_NV: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COPY_MEMORY_INDIRECT_PROPERTIES_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_FEATURES_NV: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_FEATURES_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_PROPERTIES_NV: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_PROPERTIES_EXT;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_GENERATED_COMMANDS_COMPUTE_FEATURES_NV: VkStructureType = 1000428000;
pub const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_INDIRECT_BUFFER_INFO_NV: VkStructureType = 1000428001;
pub const VK_STRUCTURE_TYPE_PIPELINE_INDIRECT_DEVICE_ADDRESS_INFO_NV: VkStructureType = 1000428002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_LINEAR_SWEPT_SPHERES_FEATURES_NV: VkStructureType = 1000429008;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_LINEAR_SWEPT_SPHERES_DATA_NV: VkStructureType = 1000429009;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_GEOMETRY_SPHERES_DATA_NV: VkStructureType = 1000429010;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINEAR_COLOR_ATTACHMENT_FEATURES_NV: VkStructureType = 1000430000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_MAXIMAL_RECONVERGENCE_FEATURES_KHR: VkStructureType = 1000434000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_COMPRESSION_CONTROL_SWAPCHAIN_FEATURES_EXT: VkStructureType = 1000437000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_PROCESSING_FEATURES_QCOM: VkStructureType = 1000440000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_PROCESSING_PROPERTIES_QCOM: VkStructureType = 1000440001;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_SAMPLE_WEIGHT_CREATE_INFO_QCOM: VkStructureType = 1000440002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_NESTED_COMMAND_BUFFER_FEATURES_EXT: VkStructureType = 1000451000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_NESTED_COMMAND_BUFFER_PROPERTIES_EXT: VkStructureType = 1000451001;
pub const VK_STRUCTURE_TYPE_NATIVE_BUFFER_USAGE_OHOS: VkStructureType = 1000452000;
pub const VK_STRUCTURE_TYPE_NATIVE_BUFFER_PROPERTIES_OHOS: VkStructureType = 1000452001;
pub const VK_STRUCTURE_TYPE_NATIVE_BUFFER_FORMAT_PROPERTIES_OHOS: VkStructureType = 1000452002;
pub const VK_STRUCTURE_TYPE_IMPORT_NATIVE_BUFFER_INFO_OHOS: VkStructureType = 1000452003;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_NATIVE_BUFFER_INFO_OHOS: VkStructureType = 1000452004;
pub const VK_STRUCTURE_TYPE_EXTERNAL_FORMAT_OHOS: VkStructureType = 1000452005;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_ACQUIRE_UNMODIFIED_EXT: VkStructureType = 1000453000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_DYNAMIC_STATE_3_FEATURES_EXT: VkStructureType = 1000455000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_DYNAMIC_STATE_3_PROPERTIES_EXT: VkStructureType = 1000455001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBPASS_MERGE_FEEDBACK_FEATURES_EXT: VkStructureType = 1000458000;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATION_CONTROL_EXT: VkStructureType = 1000458001;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATION_FEEDBACK_CREATE_INFO_EXT: VkStructureType = 1000458002;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_SUBPASS_FEEDBACK_CREATE_INFO_EXT: VkStructureType = 1000458003;
pub const VK_STRUCTURE_TYPE_DIRECT_DRIVER_LOADING_INFO_LUNARG: VkStructureType = 1000459000;
pub const VK_STRUCTURE_TYPE_DIRECT_DRIVER_LOADING_LIST_LUNARG: VkStructureType = 1000459001;
pub const VK_STRUCTURE_TYPE_TENSOR_CREATE_INFO_ARM: VkStructureType = 1000460000;
pub const VK_STRUCTURE_TYPE_TENSOR_VIEW_CREATE_INFO_ARM: VkStructureType = 1000460001;
pub const VK_STRUCTURE_TYPE_BIND_TENSOR_MEMORY_INFO_ARM: VkStructureType = 1000460002;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_TENSOR_ARM: VkStructureType = 1000460003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TENSOR_PROPERTIES_ARM: VkStructureType = 1000460004;
pub const VK_STRUCTURE_TYPE_TENSOR_FORMAT_PROPERTIES_ARM: VkStructureType = 1000460005;
pub const VK_STRUCTURE_TYPE_TENSOR_DESCRIPTION_ARM: VkStructureType = 1000460006;
pub const VK_STRUCTURE_TYPE_TENSOR_MEMORY_REQUIREMENTS_INFO_ARM: VkStructureType = 1000460007;
pub const VK_STRUCTURE_TYPE_TENSOR_MEMORY_BARRIER_ARM: VkStructureType = 1000460008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TENSOR_FEATURES_ARM: VkStructureType = 1000460009;
pub const VK_STRUCTURE_TYPE_DEVICE_TENSOR_MEMORY_REQUIREMENTS_ARM: VkStructureType = 1000460010;
pub const VK_STRUCTURE_TYPE_COPY_TENSOR_INFO_ARM: VkStructureType = 1000460011;
pub const VK_STRUCTURE_TYPE_TENSOR_COPY_ARM: VkStructureType = 1000460012;
pub const VK_STRUCTURE_TYPE_TENSOR_DEPENDENCY_INFO_ARM: VkStructureType = 1000460013;
pub const VK_STRUCTURE_TYPE_MEMORY_DEDICATED_ALLOCATE_INFO_TENSOR_ARM: VkStructureType = 1000460014;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_TENSOR_INFO_ARM: VkStructureType = 1000460015;
pub const VK_STRUCTURE_TYPE_EXTERNAL_TENSOR_PROPERTIES_ARM: VkStructureType = 1000460016;
pub const VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_TENSOR_CREATE_INFO_ARM: VkStructureType = 1000460017;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_BUFFER_TENSOR_FEATURES_ARM: VkStructureType = 1000460018;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_BUFFER_TENSOR_PROPERTIES_ARM: VkStructureType = 1000460019;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_GET_TENSOR_INFO_ARM: VkStructureType = 1000460020;
pub const VK_STRUCTURE_TYPE_TENSOR_CAPTURE_DESCRIPTOR_DATA_INFO_ARM: VkStructureType = 1000460021;
pub const VK_STRUCTURE_TYPE_TENSOR_VIEW_CAPTURE_DESCRIPTOR_DATA_INFO_ARM: VkStructureType = 1000460022;
pub const VK_STRUCTURE_TYPE_FRAME_BOUNDARY_TENSORS_ARM: VkStructureType = 1000460023;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_MODULE_IDENTIFIER_FEATURES_EXT: VkStructureType = 1000462000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_MODULE_IDENTIFIER_PROPERTIES_EXT: VkStructureType = 1000462001;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_MODULE_IDENTIFIER_CREATE_INFO_EXT: VkStructureType = 1000462002;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_IDENTIFIER_EXT: VkStructureType = 1000462003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_FEATURES_EXT: VkStructureType = 1000342000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPTICAL_FLOW_FEATURES_NV: VkStructureType = 1000464000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_OPTICAL_FLOW_PROPERTIES_NV: VkStructureType = 1000464001;
pub const VK_STRUCTURE_TYPE_OPTICAL_FLOW_IMAGE_FORMAT_INFO_NV: VkStructureType = 1000464002;
pub const VK_STRUCTURE_TYPE_OPTICAL_FLOW_IMAGE_FORMAT_PROPERTIES_NV: VkStructureType = 1000464003;
pub const VK_STRUCTURE_TYPE_OPTICAL_FLOW_SESSION_CREATE_INFO_NV: VkStructureType = 1000464004;
pub const VK_STRUCTURE_TYPE_OPTICAL_FLOW_EXECUTE_INFO_NV: VkStructureType = 1000464005;
pub const VK_STRUCTURE_TYPE_OPTICAL_FLOW_SESSION_CREATE_PRIVATE_DATA_INFO_NV: VkStructureType = 1000464010;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LEGACY_DITHERING_FEATURES_EXT: VkStructureType = 1000465000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_PROTECTED_ACCESS_FEATURES_EXT: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_PROTECTED_ACCESS_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_FORMAT_RESOLVE_FEATURES_ANDROID: VkStructureType = 1000468000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_FORMAT_RESOLVE_PROPERTIES_ANDROID: VkStructureType = 1000468001;
pub const VK_STRUCTURE_TYPE_ANDROID_HARDWARE_BUFFER_FORMAT_RESOLVE_PROPERTIES_ANDROID: VkStructureType = 1000468002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_5_PROPERTIES;
pub const VK_STRUCTURE_TYPE_RENDERING_AREA_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_AREA_INFO;
pub const VK_STRUCTURE_TYPE_DEVICE_IMAGE_SUBRESOURCE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_DEVICE_IMAGE_SUBRESOURCE_INFO;
pub const VK_STRUCTURE_TYPE_SUBRESOURCE_LAYOUT_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_SUBRESOURCE_LAYOUT_2;
pub const VK_STRUCTURE_TYPE_IMAGE_SUBRESOURCE_2_KHR: VkStructureType = VK_STRUCTURE_TYPE_IMAGE_SUBRESOURCE_2;
pub const VK_STRUCTURE_TYPE_PIPELINE_CREATE_FLAGS_2_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_CREATE_FLAGS_2_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_BUFFER_USAGE_FLAGS_2_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BUFFER_USAGE_FLAGS_2_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ANTI_LAG_FEATURES_AMD: VkStructureType = 1000476000;
pub const VK_STRUCTURE_TYPE_ANTI_LAG_DATA_AMD: VkStructureType = 1000476001;
pub const VK_STRUCTURE_TYPE_ANTI_LAG_PRESENTATION_INFO_AMD: VkStructureType = 1000476002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DENSE_GEOMETRY_FORMAT_FEATURES_AMDX: VkStructureType = 1000478000;
pub const VK_STRUCTURE_TYPE_ACCELERATION_STRUCTURE_DENSE_GEOMETRY_FORMAT_TRIANGLES_DATA_AMDX: VkStructureType = 1000478001;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_ID_2_KHR: VkStructureType = 1000479000;
pub const VK_STRUCTURE_TYPE_PRESENT_ID_2_KHR: VkStructureType = 1000479001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_ID_2_FEATURES_KHR: VkStructureType = 1000479002;
pub const VK_STRUCTURE_TYPE_SURFACE_CAPABILITIES_PRESENT_WAIT_2_KHR: VkStructureType = 1000480000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_WAIT_2_FEATURES_KHR: VkStructureType = 1000480001;
pub const VK_STRUCTURE_TYPE_PRESENT_WAIT_2_INFO_KHR: VkStructureType = 1000480002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_POSITION_FETCH_FEATURES_KHR: VkStructureType = 1000481000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_OBJECT_FEATURES_EXT: VkStructureType = 1000482000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_OBJECT_PROPERTIES_EXT: VkStructureType = 1000482001;
pub const VK_STRUCTURE_TYPE_SHADER_CREATE_INFO_EXT: VkStructureType = 1000482002;
pub const VK_STRUCTURE_TYPE_SHADER_REQUIRED_SUBGROUP_SIZE_CREATE_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_REQUIRED_SUBGROUP_SIZE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_BINARY_FEATURES_KHR: VkStructureType = 1000483000;
pub const VK_STRUCTURE_TYPE_PIPELINE_BINARY_CREATE_INFO_KHR: VkStructureType = 1000483001;
pub const VK_STRUCTURE_TYPE_PIPELINE_BINARY_INFO_KHR: VkStructureType = 1000483002;
pub const VK_STRUCTURE_TYPE_PIPELINE_BINARY_KEY_KHR: VkStructureType = 1000483003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_BINARY_PROPERTIES_KHR: VkStructureType = 1000483004;
pub const VK_STRUCTURE_TYPE_RELEASE_CAPTURED_PIPELINE_DATA_INFO_KHR: VkStructureType = 1000483005;
pub const VK_STRUCTURE_TYPE_PIPELINE_BINARY_DATA_INFO_KHR: VkStructureType = 1000483006;
pub const VK_STRUCTURE_TYPE_PIPELINE_CREATE_INFO_KHR: VkStructureType = 1000483007;
pub const VK_STRUCTURE_TYPE_DEVICE_PIPELINE_BINARY_INTERNAL_CACHE_CONTROL_KHR: VkStructureType = 1000483008;
pub const VK_STRUCTURE_TYPE_PIPELINE_BINARY_HANDLES_INFO_KHR: VkStructureType = 1000483009;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TILE_PROPERTIES_FEATURES_QCOM: VkStructureType = 1000484000;
pub const VK_STRUCTURE_TYPE_TILE_PROPERTIES_QCOM: VkStructureType = 1000484001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_AMIGO_PROFILING_FEATURES_SEC: VkStructureType = 1000485000;
pub const VK_STRUCTURE_TYPE_AMIGO_PROFILING_SUBMIT_INFO_SEC: VkStructureType = 1000485001;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_KHR: VkStructureType = 1000274000;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_SCALING_CAPABILITIES_KHR: VkStructureType = 1000274001;
pub const VK_STRUCTURE_TYPE_SURFACE_PRESENT_MODE_COMPATIBILITY_KHR: VkStructureType = 1000274002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SWAPCHAIN_MAINTENANCE_1_FEATURES_KHR: VkStructureType = 1000275000;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_FENCE_INFO_KHR: VkStructureType = 1000275001;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODES_CREATE_INFO_KHR: VkStructureType = 1000275002;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODE_INFO_KHR: VkStructureType = 1000275003;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_SCALING_CREATE_INFO_KHR: VkStructureType = 1000275004;
pub const VK_STRUCTURE_TYPE_RELEASE_SWAPCHAIN_IMAGES_INFO_KHR: VkStructureType = 1000275005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PER_VIEW_VIEWPORTS_FEATURES_QCOM: VkStructureType = 1000488000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_INVOCATION_REORDER_FEATURES_NV: VkStructureType = 1000490000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_INVOCATION_REORDER_PROPERTIES_NV: VkStructureType = 1000490001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_VECTOR_FEATURES_NV: VkStructureType = 1000491000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_VECTOR_PROPERTIES_NV: VkStructureType = 1000491001;
pub const VK_STRUCTURE_TYPE_COOPERATIVE_VECTOR_PROPERTIES_NV: VkStructureType = 1000491002;
pub const VK_STRUCTURE_TYPE_CONVERT_COOPERATIVE_VECTOR_MATRIX_INFO_NV: VkStructureType = 1000491004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_SPARSE_ADDRESS_SPACE_FEATURES_NV: VkStructureType = 1000492000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTENDED_SPARSE_ADDRESS_SPACE_PROPERTIES_NV: VkStructureType = 1000492001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MUTABLE_DESCRIPTOR_TYPE_FEATURES_EXT: VkStructureType = 1000351000;
pub const VK_STRUCTURE_TYPE_MUTABLE_DESCRIPTOR_TYPE_CREATE_INFO_EXT: VkStructureType = 1000351002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LEGACY_VERTEX_ATTRIBUTES_FEATURES_EXT: VkStructureType = 1000495000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LEGACY_VERTEX_ATTRIBUTES_PROPERTIES_EXT: VkStructureType = 1000495001;
pub const VK_STRUCTURE_TYPE_LAYER_SETTINGS_CREATE_INFO_EXT: VkStructureType = 1000496000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CORE_BUILTINS_FEATURES_ARM: VkStructureType = 1000497000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_CORE_BUILTINS_PROPERTIES_ARM: VkStructureType = 1000497001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_LIBRARY_GROUP_HANDLES_FEATURES_EXT: VkStructureType = 1000498000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_UNUSED_ATTACHMENTS_FEATURES_EXT: VkStructureType = 1000499000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INTERNALLY_SYNCHRONIZED_QUEUES_FEATURES_KHR: VkStructureType = 1000504000;
pub const VK_STRUCTURE_TYPE_LATENCY_SLEEP_MODE_INFO_NV: VkStructureType = 1000505000;
pub const VK_STRUCTURE_TYPE_LATENCY_SLEEP_INFO_NV: VkStructureType = 1000505001;
pub const VK_STRUCTURE_TYPE_SET_LATENCY_MARKER_INFO_NV: VkStructureType = 1000505002;
pub const VK_STRUCTURE_TYPE_GET_LATENCY_MARKER_INFO_NV: VkStructureType = 1000505003;
pub const VK_STRUCTURE_TYPE_LATENCY_TIMINGS_FRAME_REPORT_NV: VkStructureType = 1000505004;
pub const VK_STRUCTURE_TYPE_LATENCY_SUBMISSION_PRESENT_ID_NV: VkStructureType = 1000505005;
pub const VK_STRUCTURE_TYPE_OUT_OF_BAND_QUEUE_TYPE_INFO_NV: VkStructureType = 1000505006;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_LATENCY_CREATE_INFO_NV: VkStructureType = 1000505007;
pub const VK_STRUCTURE_TYPE_LATENCY_SURFACE_CAPABILITIES_NV: VkStructureType = 1000505008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_FEATURES_KHR: VkStructureType = 1000506000;
pub const VK_STRUCTURE_TYPE_COOPERATIVE_MATRIX_PROPERTIES_KHR: VkStructureType = 1000506001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_PROPERTIES_KHR: VkStructureType = 1000506002;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_CREATE_INFO_ARM: VkStructureType = 1000507000;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_SESSION_CREATE_INFO_ARM: VkStructureType = 1000507001;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_RESOURCE_INFO_ARM: VkStructureType = 1000507002;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_CONSTANT_ARM: VkStructureType = 1000507003;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_SESSION_MEMORY_REQUIREMENTS_INFO_ARM: VkStructureType = 1000507004;
pub const VK_STRUCTURE_TYPE_BIND_DATA_GRAPH_PIPELINE_SESSION_MEMORY_INFO_ARM: VkStructureType = 1000507005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DATA_GRAPH_FEATURES_ARM: VkStructureType = 1000507006;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_SHADER_MODULE_CREATE_INFO_ARM: VkStructureType = 1000507007;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_PROPERTY_QUERY_RESULT_ARM: VkStructureType = 1000507008;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_INFO_ARM: VkStructureType = 1000507009;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_COMPILER_CONTROL_CREATE_INFO_ARM: VkStructureType = 1000507010;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_SESSION_BIND_POINT_REQUIREMENTS_INFO_ARM: VkStructureType = 1000507011;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_SESSION_BIND_POINT_REQUIREMENT_ARM: VkStructureType = 1000507012;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_IDENTIFIER_CREATE_INFO_ARM: VkStructureType = 1000507013;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_DISPATCH_INFO_ARM: VkStructureType = 1000507014;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PROCESSING_ENGINE_CREATE_INFO_ARM: VkStructureType = 1000507016;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_DATA_GRAPH_PROCESSING_ENGINE_PROPERTIES_ARM: VkStructureType = 1000507017;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_DATA_GRAPH_PROPERTIES_ARM: VkStructureType = 1000507018;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_QUEUE_FAMILY_DATA_GRAPH_PROCESSING_ENGINE_INFO_ARM: VkStructureType = 1000507019;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_CONSTANT_TENSOR_SEMI_STRUCTURED_SPARSITY_INFO_ARM: VkStructureType = 1000507015;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MULTIVIEW_PER_VIEW_RENDER_AREAS_FEATURES_QCOM: VkStructureType = 1000510000;
pub const VK_STRUCTURE_TYPE_MULTIVIEW_PER_VIEW_RENDER_AREAS_RENDER_PASS_BEGIN_INFO_QCOM: VkStructureType = 1000510001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMPUTE_SHADER_DERIVATIVES_FEATURES_KHR: VkStructureType = 1000201000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMPUTE_SHADER_DERIVATIVES_PROPERTIES_KHR: VkStructureType = 1000511000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_CAPABILITIES_KHR: VkStructureType = 1000512000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_PICTURE_INFO_KHR: VkStructureType = 1000512001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_PROFILE_INFO_KHR: VkStructureType = 1000512003;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000512004;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_DPB_SLOT_INFO_KHR: VkStructureType = 1000512005;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_CAPABILITIES_KHR: VkStructureType = 1000513000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000513001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_PICTURE_INFO_KHR: VkStructureType = 1000513002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_DPB_SLOT_INFO_KHR: VkStructureType = 1000513003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_ENCODE_AV1_FEATURES_KHR: VkStructureType = 1000513004;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_PROFILE_INFO_KHR: VkStructureType = 1000513005;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_RATE_CONTROL_INFO_KHR: VkStructureType = 1000513006;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_RATE_CONTROL_LAYER_INFO_KHR: VkStructureType = 1000513007;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_QUALITY_LEVEL_PROPERTIES_KHR: VkStructureType = 1000513008;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_SESSION_CREATE_INFO_KHR: VkStructureType = 1000513009;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_GOP_REMAINING_FRAME_INFO_KHR: VkStructureType = 1000513010;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_DECODE_VP9_FEATURES_KHR: VkStructureType = 1000514000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_VP9_CAPABILITIES_KHR: VkStructureType = 1000514001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_VP9_PICTURE_INFO_KHR: VkStructureType = 1000514002;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_VP9_PROFILE_INFO_KHR: VkStructureType = 1000514003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_1_FEATURES_KHR: VkStructureType = 1000515000;
pub const VK_STRUCTURE_TYPE_VIDEO_INLINE_QUERY_INFO_KHR: VkStructureType = 1000515001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PER_STAGE_DESCRIPTOR_SET_FEATURES_NV: VkStructureType = 1000516000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_PROCESSING_2_FEATURES_QCOM: VkStructureType = 1000518000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_PROCESSING_2_PROPERTIES_QCOM: VkStructureType = 1000518001;
pub const VK_STRUCTURE_TYPE_SAMPLER_BLOCK_MATCH_WINDOW_CREATE_INFO_QCOM: VkStructureType = 1000518002;
pub const VK_STRUCTURE_TYPE_SAMPLER_CUBIC_WEIGHTS_CREATE_INFO_QCOM: VkStructureType = 1000519000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUBIC_WEIGHTS_FEATURES_QCOM: VkStructureType = 1000519001;
pub const VK_STRUCTURE_TYPE_BLIT_IMAGE_CUBIC_WEIGHTS_INFO_QCOM: VkStructureType = 1000519002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_YCBCR_DEGAMMA_FEATURES_QCOM: VkStructureType = 1000520000;
pub const VK_STRUCTURE_TYPE_SAMPLER_YCBCR_CONVERSION_YCBCR_DEGAMMA_CREATE_INFO_QCOM: VkStructureType = 1000520001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUBIC_CLAMP_FEATURES_QCOM: VkStructureType = 1000521000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ATTACHMENT_FEEDBACK_LOOP_DYNAMIC_STATE_FEATURES_EXT: VkStructureType = 1000524000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_DIVISOR_STATE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_DIVISOR_STATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_DIVISOR_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFIED_IMAGE_LAYOUTS_FEATURES_KHR: VkStructureType = 1000527000;
pub const VK_STRUCTURE_TYPE_ATTACHMENT_FEEDBACK_LOOP_INFO_EXT: VkStructureType = 1000527001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT_CONTROLS_2_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT_CONTROLS_2_FEATURES;
pub const VK_STRUCTURE_TYPE_SCREEN_BUFFER_PROPERTIES_QNX: VkStructureType = 1000529000;
pub const VK_STRUCTURE_TYPE_SCREEN_BUFFER_FORMAT_PROPERTIES_QNX: VkStructureType = 1000529001;
pub const VK_STRUCTURE_TYPE_IMPORT_SCREEN_BUFFER_INFO_QNX: VkStructureType = 1000529002;
pub const VK_STRUCTURE_TYPE_EXTERNAL_FORMAT_QNX: VkStructureType = 1000529003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_MEMORY_SCREEN_BUFFER_FEATURES_QNX: VkStructureType = 1000529004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_DRIVER_PROPERTIES_MSFT: VkStructureType = 1000530000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INDEX_TYPE_UINT8_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_INDEX_TYPE_UINT8_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES;
pub const VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_LINE_STATE_CREATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_LINE_STATE_CREATE_INFO;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_PROPERTIES;
pub const VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_KHR: VkStructureType = 1000184000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_EXPECT_ASSUME_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_EXPECT_ASSUME_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_FEATURES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_FEATURES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_6_PROPERTIES;
pub const VK_STRUCTURE_TYPE_BIND_MEMORY_STATUS_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_MEMORY_STATUS;
pub const VK_STRUCTURE_TYPE_BIND_DESCRIPTOR_SETS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_BIND_DESCRIPTOR_SETS_INFO;
pub const VK_STRUCTURE_TYPE_PUSH_CONSTANTS_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PUSH_CONSTANTS_INFO;
pub const VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_INFO;
pub const VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_WITH_TEMPLATE_INFO_KHR: VkStructureType = VK_STRUCTURE_TYPE_PUSH_DESCRIPTOR_SET_WITH_TEMPLATE_INFO;
pub const VK_STRUCTURE_TYPE_SET_DESCRIPTOR_BUFFER_OFFSETS_INFO_EXT: VkStructureType = 1000545007;
pub const VK_STRUCTURE_TYPE_BIND_DESCRIPTOR_BUFFER_EMBEDDED_SAMPLERS_INFO_EXT: VkStructureType = 1000545008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_POOL_OVERALLOCATION_FEATURES_NV: VkStructureType = 1000546000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TILE_MEMORY_HEAP_FEATURES_QCOM: VkStructureType = 1000547000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TILE_MEMORY_HEAP_PROPERTIES_QCOM: VkStructureType = 1000547001;
pub const VK_STRUCTURE_TYPE_TILE_MEMORY_REQUIREMENTS_QCOM: VkStructureType = 1000547002;
pub const VK_STRUCTURE_TYPE_TILE_MEMORY_BIND_INFO_QCOM: VkStructureType = 1000547003;
pub const VK_STRUCTURE_TYPE_TILE_MEMORY_SIZE_INFO_QCOM: VkStructureType = 1000547004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COPY_MEMORY_INDIRECT_FEATURES_KHR: VkStructureType = 1000549000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COPY_MEMORY_INDIRECT_PROPERTIES_KHR: VkStructureType = 1000426001;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_INDIRECT_INFO_KHR: VkStructureType = 1000549002;
pub const VK_STRUCTURE_TYPE_COPY_MEMORY_TO_IMAGE_INDIRECT_INFO_KHR: VkStructureType = 1000549003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_FEATURES_EXT: VkStructureType = 1000427000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MEMORY_DECOMPRESSION_PROPERTIES_EXT: VkStructureType = 1000427001;
pub const VK_STRUCTURE_TYPE_DECOMPRESS_MEMORY_INFO_EXT: VkStructureType = 1000550002;
pub const VK_STRUCTURE_TYPE_DISPLAY_SURFACE_STEREO_CREATE_INFO_NV: VkStructureType = 1000551000;
pub const VK_STRUCTURE_TYPE_DISPLAY_MODE_STEREO_PROPERTIES_NV: VkStructureType = 1000551001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_INTRA_REFRESH_CAPABILITIES_KHR: VkStructureType = 1000552000;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_SESSION_INTRA_REFRESH_CREATE_INFO_KHR: VkStructureType = 1000552001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_INTRA_REFRESH_INFO_KHR: VkStructureType = 1000552002;
pub const VK_STRUCTURE_TYPE_VIDEO_REFERENCE_INTRA_REFRESH_INFO_KHR: VkStructureType = 1000552003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_ENCODE_INTRA_REFRESH_FEATURES_KHR: VkStructureType = 1000552004;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_QUANTIZATION_MAP_CAPABILITIES_KHR: VkStructureType = 1000553000;
pub const VK_STRUCTURE_TYPE_VIDEO_FORMAT_QUANTIZATION_MAP_PROPERTIES_KHR: VkStructureType = 1000553001;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_QUANTIZATION_MAP_INFO_KHR: VkStructureType = 1000553002;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_QUANTIZATION_MAP_SESSION_PARAMETERS_CREATE_INFO_KHR: VkStructureType = 1000553005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_ENCODE_QUANTIZATION_MAP_FEATURES_KHR: VkStructureType = 1000553009;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H264_QUANTIZATION_MAP_CAPABILITIES_KHR: VkStructureType = 1000553003;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_H265_QUANTIZATION_MAP_CAPABILITIES_KHR: VkStructureType = 1000553004;
pub const VK_STRUCTURE_TYPE_VIDEO_FORMAT_H265_QUANTIZATION_MAP_PROPERTIES_KHR: VkStructureType = 1000553006;
pub const VK_STRUCTURE_TYPE_VIDEO_ENCODE_AV1_QUANTIZATION_MAP_CAPABILITIES_KHR: VkStructureType = 1000553007;
pub const VK_STRUCTURE_TYPE_VIDEO_FORMAT_AV1_QUANTIZATION_MAP_PROPERTIES_KHR: VkStructureType = 1000553008;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAW_ACCESS_CHAINS_FEATURES_NV: VkStructureType = 1000555000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_COMPUTE_QUEUE_DEVICE_CREATE_INFO_NV: VkStructureType = 1000556000;
pub const VK_STRUCTURE_TYPE_EXTERNAL_COMPUTE_QUEUE_CREATE_INFO_NV: VkStructureType = 1000556001;
pub const VK_STRUCTURE_TYPE_EXTERNAL_COMPUTE_QUEUE_DATA_PARAMS_NV: VkStructureType = 1000556002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_COMPUTE_QUEUE_PROPERTIES_NV: VkStructureType = 1000556003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_RELAXED_EXTENDED_INSTRUCTION_FEATURES_KHR: VkStructureType = 1000558000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMMAND_BUFFER_INHERITANCE_FEATURES_NV: VkStructureType = 1000559000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_7_FEATURES_KHR: VkStructureType = 1000562000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_7_PROPERTIES_KHR: VkStructureType = 1000562001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_API_PROPERTIES_LIST_KHR: VkStructureType = 1000562002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_API_PROPERTIES_KHR: VkStructureType = 1000562003;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LAYERED_API_VULKAN_PROPERTIES_KHR: VkStructureType = 1000562004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_ATOMIC_FLOAT16_VECTOR_FEATURES_NV: VkStructureType = 1000563000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_REPLICATED_COMPOSITES_FEATURES_EXT: VkStructureType = 1000564000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FLOAT8_FEATURES_EXT: VkStructureType = 1000567000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_VALIDATION_FEATURES_NV: VkStructureType = 1000568000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CLUSTER_ACCELERATION_STRUCTURE_FEATURES_NV: VkStructureType = 1000569000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CLUSTER_ACCELERATION_STRUCTURE_PROPERTIES_NV: VkStructureType = 1000569001;
pub const VK_STRUCTURE_TYPE_CLUSTER_ACCELERATION_STRUCTURE_CLUSTERS_BOTTOM_LEVEL_INPUT_NV: VkStructureType = 1000569002;
pub const VK_STRUCTURE_TYPE_CLUSTER_ACCELERATION_STRUCTURE_TRIANGLE_CLUSTER_INPUT_NV: VkStructureType = 1000569003;
pub const VK_STRUCTURE_TYPE_CLUSTER_ACCELERATION_STRUCTURE_MOVE_OBJECTS_INPUT_NV: VkStructureType = 1000569004;
pub const VK_STRUCTURE_TYPE_CLUSTER_ACCELERATION_STRUCTURE_INPUT_INFO_NV: VkStructureType = 1000569005;
pub const VK_STRUCTURE_TYPE_CLUSTER_ACCELERATION_STRUCTURE_COMMANDS_INFO_NV: VkStructureType = 1000569006;
pub const VK_STRUCTURE_TYPE_RAY_TRACING_PIPELINE_CLUSTER_ACCELERATION_STRUCTURE_CREATE_INFO_NV: VkStructureType = 1000569007;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PARTITIONED_ACCELERATION_STRUCTURE_FEATURES_NV: VkStructureType = 1000570000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PARTITIONED_ACCELERATION_STRUCTURE_PROPERTIES_NV: VkStructureType = 1000570001;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET_PARTITIONED_ACCELERATION_STRUCTURE_NV: VkStructureType = 1000570002;
pub const VK_STRUCTURE_TYPE_PARTITIONED_ACCELERATION_STRUCTURE_INSTANCES_INPUT_NV: VkStructureType = 1000570003;
pub const VK_STRUCTURE_TYPE_BUILD_PARTITIONED_ACCELERATION_STRUCTURE_INFO_NV: VkStructureType = 1000570004;
pub const VK_STRUCTURE_TYPE_PARTITIONED_ACCELERATION_STRUCTURE_FLAGS_NV: VkStructureType = 1000570005;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_GENERATED_COMMANDS_FEATURES_EXT: VkStructureType = 1000572000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEVICE_GENERATED_COMMANDS_PROPERTIES_EXT: VkStructureType = 1000572001;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_MEMORY_REQUIREMENTS_INFO_EXT: VkStructureType = 1000572002;
pub const VK_STRUCTURE_TYPE_INDIRECT_EXECUTION_SET_CREATE_INFO_EXT: VkStructureType = 1000572003;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_INFO_EXT: VkStructureType = 1000572004;
pub const VK_STRUCTURE_TYPE_INDIRECT_COMMANDS_LAYOUT_CREATE_INFO_EXT: VkStructureType = 1000572006;
pub const VK_STRUCTURE_TYPE_INDIRECT_COMMANDS_LAYOUT_TOKEN_EXT: VkStructureType = 1000572007;
pub const VK_STRUCTURE_TYPE_WRITE_INDIRECT_EXECUTION_SET_PIPELINE_EXT: VkStructureType = 1000572008;
pub const VK_STRUCTURE_TYPE_WRITE_INDIRECT_EXECUTION_SET_SHADER_EXT: VkStructureType = 1000572009;
pub const VK_STRUCTURE_TYPE_INDIRECT_EXECUTION_SET_PIPELINE_INFO_EXT: VkStructureType = 1000572010;
pub const VK_STRUCTURE_TYPE_INDIRECT_EXECUTION_SET_SHADER_INFO_EXT: VkStructureType = 1000572011;
pub const VK_STRUCTURE_TYPE_INDIRECT_EXECUTION_SET_SHADER_LAYOUT_INFO_EXT: VkStructureType = 1000572012;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_PIPELINE_INFO_EXT: VkStructureType = 1000572013;
pub const VK_STRUCTURE_TYPE_GENERATED_COMMANDS_SHADER_INFO_EXT: VkStructureType = 1000572014;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_8_FEATURES_KHR: VkStructureType = 1000574000;
pub const VK_STRUCTURE_TYPE_MEMORY_BARRIER_ACCESS_FLAGS_3_KHR: VkStructureType = 1000574002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ALIGNMENT_CONTROL_FEATURES_MESA: VkStructureType = 1000575000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_IMAGE_ALIGNMENT_CONTROL_PROPERTIES_MESA: VkStructureType = 1000575001;
pub const VK_STRUCTURE_TYPE_IMAGE_ALIGNMENT_CONTROL_CREATE_INFO_MESA: VkStructureType = 1000575002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_FMA_FEATURES_KHR: VkStructureType = 1000579000;
pub const VK_STRUCTURE_TYPE_PUSH_CONSTANT_BANK_INFO_NV: VkStructureType = 1000580000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_CONSTANT_BANK_FEATURES_NV: VkStructureType = 1000580001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_CONSTANT_BANK_PROPERTIES_NV: VkStructureType = 1000580002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_INVOCATION_REORDER_FEATURES_EXT: VkStructureType = 1000581000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RAY_TRACING_INVOCATION_REORDER_PROPERTIES_EXT: VkStructureType = 1000581001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLAMP_CONTROL_FEATURES_EXT: VkStructureType = 1000582000;
pub const VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_DEPTH_CLAMP_CONTROL_CREATE_INFO_EXT: VkStructureType = 1000582001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_9_FEATURES_KHR: VkStructureType = 1000584000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_9_PROPERTIES_KHR: VkStructureType = 1000584001;
pub const VK_STRUCTURE_TYPE_QUEUE_FAMILY_OWNERSHIP_TRANSFER_PROPERTIES_KHR: VkStructureType = 1000584002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VIDEO_MAINTENANCE_2_FEATURES_KHR: VkStructureType = 1000586000;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H264_INLINE_SESSION_PARAMETERS_INFO_KHR: VkStructureType = 1000586001;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_H265_INLINE_SESSION_PARAMETERS_INFO_KHR: VkStructureType = 1000586002;
pub const VK_STRUCTURE_TYPE_VIDEO_DECODE_AV1_INLINE_SESSION_PARAMETERS_INFO_KHR: VkStructureType = 1000586003;
pub const VK_STRUCTURE_TYPE_SURFACE_CREATE_INFO_OHOS: VkStructureType = 1000685000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_HDR_VIVID_FEATURES_HUAWEI: VkStructureType = 1000590000;
pub const VK_STRUCTURE_TYPE_HDR_VIVID_DYNAMIC_METADATA_HUAWEI: VkStructureType = 1000590001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_2_FEATURES_NV: VkStructureType = 1000593000;
pub const VK_STRUCTURE_TYPE_COOPERATIVE_MATRIX_FLEXIBLE_DIMENSIONS_PROPERTIES_NV: VkStructureType = 1000593001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COOPERATIVE_MATRIX_2_PROPERTIES_NV: VkStructureType = 1000593002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_OPACITY_MICROMAP_FEATURES_ARM: VkStructureType = 1000596000;
pub const VK_STRUCTURE_TYPE_IMPORT_MEMORY_METAL_HANDLE_INFO_EXT: VkStructureType = 1000602000;
pub const VK_STRUCTURE_TYPE_MEMORY_METAL_HANDLE_PROPERTIES_EXT: VkStructureType = 1000602001;
pub const VK_STRUCTURE_TYPE_MEMORY_GET_METAL_HANDLE_INFO_EXT: VkStructureType = 1000602002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DEPTH_CLAMP_ZERO_ONE_FEATURES_KHR: VkStructureType = 1000421000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PERFORMANCE_COUNTERS_BY_REGION_FEATURES_ARM: VkStructureType = 1000605000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PERFORMANCE_COUNTERS_BY_REGION_PROPERTIES_ARM: VkStructureType = 1000605001;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_COUNTER_ARM: VkStructureType = 1000605002;
pub const VK_STRUCTURE_TYPE_PERFORMANCE_COUNTER_DESCRIPTION_ARM: VkStructureType = 1000605003;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_PERFORMANCE_COUNTERS_BY_REGION_BEGIN_INFO_ARM: VkStructureType = 1000605004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_VERTEX_ATTRIBUTE_ROBUSTNESS_FEATURES_EXT: VkStructureType = 1000608000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FORMAT_PACK_FEATURES_ARM: VkStructureType = 1000609000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_LAYERED_FEATURES_VALVE: VkStructureType = 1000611000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_LAYERED_PROPERTIES_VALVE: VkStructureType = 1000611001;
pub const VK_STRUCTURE_TYPE_PIPELINE_FRAGMENT_DENSITY_MAP_LAYERED_CREATE_INFO_VALVE: VkStructureType = 1000611002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_KHR: VkStructureType = 1000286000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ROBUSTNESS_2_PROPERTIES_KHR: VkStructureType = 1000286001;
pub const VK_STRUCTURE_TYPE_SET_PRESENT_CONFIG_NV: VkStructureType = 1000613000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_METERING_FEATURES_NV: VkStructureType = 1000613001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_FEATURES_EXT: VkStructureType = 1000425000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_DENSITY_MAP_OFFSET_PROPERTIES_EXT: VkStructureType = 1000425001;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_FRAGMENT_DENSITY_MAP_OFFSET_END_INFO_EXT: VkStructureType = 1000425002;
pub const VK_STRUCTURE_TYPE_RENDERING_END_INFO_EXT: VkStructureType = VK_STRUCTURE_TYPE_RENDERING_END_INFO_KHR;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ZERO_INITIALIZE_DEVICE_MEMORY_FEATURES_EXT: VkStructureType = 1000620000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PRESENT_MODE_FIFO_LATEST_READY_FEATURES_KHR: VkStructureType = 1000361000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_64_BIT_INDEXING_FEATURES_EXT: VkStructureType = 1000627000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_CUSTOM_RESOLVE_FEATURES_EXT: VkStructureType = 1000628000;
pub const VK_STRUCTURE_TYPE_BEGIN_CUSTOM_RESOLVE_INFO_EXT: VkStructureType = 1000628001;
pub const VK_STRUCTURE_TYPE_CUSTOM_RESOLVE_CREATE_INFO_EXT: VkStructureType = 1000628002;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DATA_GRAPH_MODEL_FEATURES_QCOM: VkStructureType = 1000629000;
pub const VK_STRUCTURE_TYPE_DATA_GRAPH_PIPELINE_BUILTIN_MODEL_CREATE_INFO_QCOM: VkStructureType = 1000629001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_10_FEATURES_KHR: VkStructureType = 1000630000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_MAINTENANCE_10_PROPERTIES_KHR: VkStructureType = 1000630001;
pub const VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_FLAGS_INFO_KHR: VkStructureType = 1000630002;
pub const VK_STRUCTURE_TYPE_RENDERING_END_INFO_KHR: VkStructureType = 1000619003;
pub const VK_STRUCTURE_TYPE_RESOLVE_IMAGE_MODE_INFO_KHR: VkStructureType = 1000630004;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_LONG_VECTOR_FEATURES_EXT: VkStructureType = 1000635000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_LONG_VECTOR_PROPERTIES_EXT: VkStructureType = 1000635001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PIPELINE_CACHE_INCREMENTAL_MODE_FEATURES_SEC: VkStructureType = 1000637000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_UNIFORM_BUFFER_UNSIZED_ARRAY_FEATURES_EXT: VkStructureType = 1000642000;
pub const VK_STRUCTURE_TYPE_COMPUTE_OCCUPANCY_PRIORITY_PARAMETERS_NV: VkStructureType = 1000645000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COMPUTE_OCCUPANCY_PRIORITY_FEATURES_NV: VkStructureType = 1000645001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SHADER_SUBGROUP_PARTITIONED_FEATURES_EXT: VkStructureType = 1000662000;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSubmitInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub waitSemaphoreCount: u32,
    pub pWaitSemaphores: *const VkSemaphore,
    pub pWaitDstStageMask: *const VkPipelineStageFlags,
    pub commandBufferCount: u32,
    pub pCommandBuffers: *const VkCommandBuffer,
    pub signalSemaphoreCount: u32,
    pub pSignalSemaphores: *const VkSemaphore,
}
pub type VkSubpassContents = i32;
pub const VK_SUBPASS_CONTENTS_INLINE: VkSubpassContents = 0;
pub const VK_SUBPASS_CONTENTS_SECONDARY_COMMAND_BUFFERS: VkSubpassContents = 1;
pub const VK_SUBPASS_CONTENTS_INLINE_AND_SECONDARY_COMMAND_BUFFERS_EXT: VkSubpassContents = VK_SUBPASS_CONTENTS_INLINE_AND_SECONDARY_COMMAND_BUFFERS_KHR;
pub const VK_SUBPASS_CONTENTS_INLINE_AND_SECONDARY_COMMAND_BUFFERS_KHR: VkSubpassContents = 1000451000;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSubpassDependency {
    pub srcSubpass: u32,
    pub dstSubpass: u32,
    pub srcStageMask: VkPipelineStageFlags,
    pub dstStageMask: VkPipelineStageFlags,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub dependencyFlags: VkDependencyFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSubpassDescription {
    pub flags: VkSubpassDescriptionFlags,
    pub pipelineBindPoint: VkPipelineBindPoint,
    pub inputAttachmentCount: u32,
    pub pInputAttachments: *const VkAttachmentReference,
    pub colorAttachmentCount: u32,
    pub pColorAttachments: *const VkAttachmentReference,
    pub pResolveAttachments: *const VkAttachmentReference,
    pub pDepthStencilAttachment: *const VkAttachmentReference,
    pub preserveAttachmentCount: u32,
    pub pPreserveAttachments: *const u32,
}
pub type VkSubpassDescriptionFlagBits = u32;
pub const VK_SUBPASS_DESCRIPTION_PER_VIEW_ATTRIBUTES_BIT_NVX: VkSubpassDescriptionFlagBits = 0x1;
pub const VK_SUBPASS_DESCRIPTION_PER_VIEW_POSITION_X_ONLY_BIT_NVX: VkSubpassDescriptionFlagBits = 0x2;
pub const VK_SUBPASS_DESCRIPTION_FRAGMENT_REGION_BIT_QCOM: VkSubpassDescriptionFlagBits = VK_SUBPASS_DESCRIPTION_FRAGMENT_REGION_BIT_EXT;
pub const VK_SUBPASS_DESCRIPTION_SHADER_RESOLVE_BIT_QCOM: VkSubpassDescriptionFlagBits = VK_SUBPASS_DESCRIPTION_CUSTOM_RESOLVE_BIT_EXT;
pub const VK_SUBPASS_DESCRIPTION_TILE_SHADING_APRON_BIT_QCOM: VkSubpassDescriptionFlagBits = 0x100;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_COLOR_ACCESS_BIT_ARM: VkSubpassDescriptionFlagBits = VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_COLOR_ACCESS_BIT_EXT;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_ARM: VkSubpassDescriptionFlagBits = VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_EXT;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_ARM: VkSubpassDescriptionFlagBits = VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_EXT;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_COLOR_ACCESS_BIT_EXT: VkSubpassDescriptionFlagBits = 0x10;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_DEPTH_ACCESS_BIT_EXT: VkSubpassDescriptionFlagBits = 0x20;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_STENCIL_ACCESS_BIT_EXT: VkSubpassDescriptionFlagBits = 0x40;
pub const VK_SUBPASS_DESCRIPTION_ENABLE_LEGACY_DITHERING_BIT_EXT: VkSubpassDescriptionFlagBits = 0x80;
pub const VK_SUBPASS_DESCRIPTION_FRAGMENT_REGION_BIT_EXT: VkSubpassDescriptionFlagBits = 0x4;
pub const VK_SUBPASS_DESCRIPTION_CUSTOM_RESOLVE_BIT_EXT: VkSubpassDescriptionFlagBits = 0x8;
pub type VkSubpassDescriptionFlags = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSubresourceLayout {
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
    pub rowPitch: VkDeviceSize,
    pub arrayPitch: VkDeviceSize,
    pub depthPitch: VkDeviceSize,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSurfaceCapabilitiesKHR {
    pub minImageCount: u32,
    pub maxImageCount: u32,
    pub currentExtent: VkExtent2D,
    pub minImageExtent: VkExtent2D,
    pub maxImageExtent: VkExtent2D,
    pub maxImageArrayLayers: u32,
    pub supportedTransforms: VkSurfaceTransformFlagsKHR,
    pub currentTransform: VkSurfaceTransformFlagBitsKHR,
    pub supportedCompositeAlpha: VkCompositeAlphaFlagsKHR,
    pub supportedUsageFlags: VkImageUsageFlags,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSurfaceFormatKHR {
    pub format: VkFormat,
    pub colorSpace: VkColorSpaceKHR,
}
pub type VkSurfaceKHR = u64;
pub type VkSurfaceTransformFlagBitsKHR = u32;
pub const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x1;
pub const VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x2;
pub const VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x4;
pub const VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x8;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x10;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x20;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x40;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x80;
pub const VK_SURFACE_TRANSFORM_INHERIT_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x100;
pub type VkSurfaceTransformFlagsKHR = u32;
pub type VkSwapchainCreateFlagBitsKHR = u32;
pub const VK_SWAPCHAIN_CREATE_SPLIT_INSTANCE_BIND_REGIONS_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x1;
pub const VK_SWAPCHAIN_CREATE_PROTECTED_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x2;
pub const VK_SWAPCHAIN_CREATE_MUTABLE_FORMAT_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x4;
pub const VK_SWAPCHAIN_CREATE_PRESENT_TIMING_BIT_EXT: VkSwapchainCreateFlagBitsKHR = 0x200;
pub const VK_SWAPCHAIN_CREATE_DEFERRED_MEMORY_ALLOCATION_BIT_EXT: VkSwapchainCreateFlagBitsKHR = VK_SWAPCHAIN_CREATE_DEFERRED_MEMORY_ALLOCATION_BIT_KHR;
pub const VK_SWAPCHAIN_CREATE_PRESENT_ID_2_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x40;
pub const VK_SWAPCHAIN_CREATE_PRESENT_WAIT_2_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x80;
pub const VK_SWAPCHAIN_CREATE_DEFERRED_MEMORY_ALLOCATION_BIT_KHR: VkSwapchainCreateFlagBitsKHR = 0x8;
pub type VkSwapchainCreateFlagsKHR = u32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkSwapchainCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkSwapchainCreateFlagsKHR,
    pub surface: VkSurfaceKHR,
    pub minImageCount: u32,
    pub imageFormat: VkFormat,
    pub imageColorSpace: VkColorSpaceKHR,
    pub imageExtent: VkExtent2D,
    pub imageArrayLayers: u32,
    pub imageUsage: VkImageUsageFlags,
    pub imageSharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
    pub preTransform: VkSurfaceTransformFlagBitsKHR,
    pub compositeAlpha: VkCompositeAlphaFlagBitsKHR,
    pub presentMode: VkPresentModeKHR,
    pub clipped: VkBool32,
    pub oldSwapchain: VkSwapchainKHR,
}
pub type VkSwapchainKHR = u64;
pub type VkSystemAllocationScope = i32;
pub const VK_SYSTEM_ALLOCATION_SCOPE_COMMAND: VkSystemAllocationScope = 0;
pub const VK_SYSTEM_ALLOCATION_SCOPE_OBJECT: VkSystemAllocationScope = 1;
pub const VK_SYSTEM_ALLOCATION_SCOPE_CACHE: VkSystemAllocationScope = 2;
pub const VK_SYSTEM_ALLOCATION_SCOPE_DEVICE: VkSystemAllocationScope = 3;
pub const VK_SYSTEM_ALLOCATION_SCOPE_INSTANCE: VkSystemAllocationScope = 4;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkVertexInputAttributeDescription {
    pub location: u32,
    pub binding: u32,
    pub format: VkFormat,
    pub offset: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkVertexInputBindingDescription {
    pub binding: u32,
    pub stride: u32,
    pub inputRate: VkVertexInputRate,
}
pub type VkVertexInputRate = i32;
pub const VK_VERTEX_INPUT_RATE_VERTEX: VkVertexInputRate = 0;
pub const VK_VERTEX_INPUT_RATE_INSTANCE: VkVertexInputRate = 1;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub minDepth: f32,
    pub maxDepth: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VkWriteDescriptorSet {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub dstSet: VkDescriptorSet,
    pub dstBinding: u32,
    pub dstArrayElement: u32,
    pub descriptorCount: u32,
    pub descriptorType: VkDescriptorType,
    pub pImageInfo: *const VkDescriptorImageInfo,
    pub pBufferInfo: *const VkDescriptorBufferInfo,
    pub pTexelBufferView: *const VkBufferView,
}
pub type VkConformanceVersionKHR = VkConformanceVersion;
pub type VkDriverIdKHR = VkDriverId;
pub type VkFormatProperties2KHR = VkFormatProperties2;
pub type VkImageFormatProperties2KHR = VkImageFormatProperties2;
pub type VkPhysicalDeviceDriverPropertiesKHR = VkPhysicalDeviceDriverProperties;
pub type VkPhysicalDeviceFeatures2KHR = VkPhysicalDeviceFeatures2;
pub type VkPhysicalDeviceIDPropertiesKHR = VkPhysicalDeviceIDProperties;
pub type VkPhysicalDeviceImageFormatInfo2KHR = VkPhysicalDeviceImageFormatInfo2;
pub type VkPhysicalDeviceMemoryProperties2KHR = VkPhysicalDeviceMemoryProperties2;
pub type VkPhysicalDeviceProperties2KHR = VkPhysicalDeviceProperties2;
pub type VkPhysicalDeviceSparseImageFormatInfo2KHR = VkPhysicalDeviceSparseImageFormatInfo2;
pub type VkQueueFamilyProperties2KHR = VkQueueFamilyProperties2;
pub type VkSparseImageFormatProperties2KHR = VkSparseImageFormatProperties2;

pub type PFN_vkCreateInstance = unsafe extern "C" fn(pCreateInfo: *const VkInstanceCreateInfo, pAllocator: *const VkAllocationCallbacks, pInstance: *mut VkInstance) -> VkResult;
pub type PFN_vkDestroyInstance = unsafe extern "C" fn(instance: VkInstance, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkEnumerateInstanceExtensionProperties = unsafe extern "C" fn(pLayerName: *const c_char, pPropertyCount: *mut u32, pProperties: *mut VkExtensionProperties) -> VkResult;
pub type PFN_vkEnumerateInstanceLayerProperties = unsafe extern "C" fn(pPropertyCount: *mut u32, pProperties: *mut VkLayerProperties) -> VkResult;
pub type PFN_vkEnumerateInstanceVersion = unsafe extern "C" fn(pApiVersion: *mut u32) -> VkResult;
pub type PFN_vkEnumeratePhysicalDevices = unsafe extern "C" fn(instance: VkInstance, pPhysicalDeviceCount: *mut u32, pPhysicalDevices: *mut VkPhysicalDevice) -> VkResult;
pub type PFN_vkGetInstanceProcAddr = unsafe extern "C" fn(instance: VkInstance, pName: *const c_char) -> PFN_vkVoidFunction;
pub type PFN_vkGetDeviceProcAddr = unsafe extern "C" fn(device: VkDevice, pName: *const c_char) -> PFN_vkVoidFunction;
pub type PFN_vkGetPhysicalDeviceProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pProperties: *mut VkPhysicalDeviceProperties);
pub type PFN_vkGetPhysicalDeviceProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pProperties: *mut VkPhysicalDeviceProperties2);
pub type PFN_vkGetPhysicalDeviceFeatures = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pFeatures: *mut VkPhysicalDeviceFeatures);
pub type PFN_vkGetPhysicalDeviceFeatures2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pFeatures: *mut VkPhysicalDeviceFeatures2);
pub type PFN_vkGetPhysicalDeviceMemoryProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pMemoryProperties: *mut VkPhysicalDeviceMemoryProperties);
pub type PFN_vkGetPhysicalDeviceMemoryProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pMemoryProperties: *mut VkPhysicalDeviceMemoryProperties2);
pub type PFN_vkGetPhysicalDeviceQueueFamilyProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pQueueFamilyPropertyCount: *mut u32, pQueueFamilyProperties: *mut VkQueueFamilyProperties);
pub type PFN_vkGetPhysicalDeviceQueueFamilyProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pQueueFamilyPropertyCount: *mut u32, pQueueFamilyProperties: *mut VkQueueFamilyProperties2);
pub type PFN_vkGetPhysicalDeviceFormatProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, format: VkFormat, pFormatProperties: *mut VkFormatProperties);
pub type PFN_vkGetPhysicalDeviceFormatProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, format: VkFormat, pFormatProperties: *mut VkFormatProperties2);
pub type PFN_vkGetPhysicalDeviceImageFormatProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, format: VkFormat, type_: VkImageType, tiling: VkImageTiling, usage: VkImageUsageFlags, flags: VkImageCreateFlags, pImageFormatProperties: *mut VkImageFormatProperties) -> VkResult;
pub type PFN_vkGetPhysicalDeviceImageFormatProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pImageFormatInfo: *const VkPhysicalDeviceImageFormatInfo2, pImageFormatProperties: *mut VkImageFormatProperties2) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSparseImageFormatProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, format: VkFormat, type_: VkImageType, samples: VkSampleCountFlagBits, usage: VkImageUsageFlags, tiling: VkImageTiling, pPropertyCount: *mut u32, pProperties: *mut VkSparseImageFormatProperties);
pub type PFN_vkGetPhysicalDeviceSparseImageFormatProperties2 = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pFormatInfo: *const VkPhysicalDeviceSparseImageFormatInfo2, pPropertyCount: *mut u32, pProperties: *mut VkSparseImageFormatProperties2);
pub type PFN_vkEnumerateDeviceExtensionProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pLayerName: *const c_char, pPropertyCount: *mut u32, pProperties: *mut VkExtensionProperties) -> VkResult;
pub type PFN_vkEnumerateDeviceLayerProperties = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pPropertyCount: *mut u32, pProperties: *mut VkLayerProperties) -> VkResult;
pub type PFN_vkCreateDevice = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, pCreateInfo: *const VkDeviceCreateInfo, pAllocator: *const VkAllocationCallbacks, pDevice: *mut VkDevice) -> VkResult;
pub type PFN_vkDestroyDevice = unsafe extern "C" fn(device: VkDevice, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetDeviceQueue = unsafe extern "C" fn(device: VkDevice, queueFamilyIndex: u32, queueIndex: u32, pQueue: *mut VkQueue);
pub type PFN_vkDeviceWaitIdle = unsafe extern "C" fn(device: VkDevice) -> VkResult;
pub type PFN_vkCreateImage = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkImageCreateInfo, pAllocator: *const VkAllocationCallbacks, pImage: *mut VkImage) -> VkResult;
pub type PFN_vkDestroyImage = unsafe extern "C" fn(device: VkDevice, image: VkImage, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetImageMemoryRequirements = unsafe extern "C" fn(device: VkDevice, image: VkImage, pMemoryRequirements: *mut VkMemoryRequirements);
pub type PFN_vkCreateBuffer = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkBufferCreateInfo, pAllocator: *const VkAllocationCallbacks, pBuffer: *mut VkBuffer) -> VkResult;
pub type PFN_vkDestroyBuffer = unsafe extern "C" fn(device: VkDevice, buffer: VkBuffer, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetBufferMemoryRequirements = unsafe extern "C" fn(device: VkDevice, buffer: VkBuffer, pMemoryRequirements: *mut VkMemoryRequirements);
pub type PFN_vkAllocateMemory = unsafe extern "C" fn(device: VkDevice, pAllocateInfo: *const VkMemoryAllocateInfo, pAllocator: *const VkAllocationCallbacks, pMemory: *mut VkDeviceMemory) -> VkResult;
pub type PFN_vkFreeMemory = unsafe extern "C" fn(device: VkDevice, memory: VkDeviceMemory, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkMapMemory = unsafe extern "C" fn(device: VkDevice, memory: VkDeviceMemory, offset: VkDeviceSize, size: VkDeviceSize, flags: VkMemoryMapFlags, ppData: *mut *mut c_void) -> VkResult;
pub type PFN_vkUnmapMemory = unsafe extern "C" fn(device: VkDevice, memory: VkDeviceMemory);
pub type PFN_vkFlushMappedMemoryRanges = unsafe extern "C" fn(device: VkDevice, memoryRangeCount: u32, pMemoryRanges: *const VkMappedMemoryRange) -> VkResult;
pub type PFN_vkInvalidateMappedMemoryRanges = unsafe extern "C" fn(device: VkDevice, memoryRangeCount: u32, pMemoryRanges: *const VkMappedMemoryRange) -> VkResult;
pub type PFN_vkBindBufferMemory = unsafe extern "C" fn(device: VkDevice, buffer: VkBuffer, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult;
pub type PFN_vkBindImageMemory = unsafe extern "C" fn(device: VkDevice, image: VkImage, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult;
pub type PFN_vkGetDeviceMemoryCommitment = unsafe extern "C" fn(device: VkDevice, memory: VkDeviceMemory, pCommittedMemoryInBytes: *mut VkDeviceSize);
pub type PFN_vkGetImageSubresourceLayout = unsafe extern "C" fn(device: VkDevice, image: VkImage, pSubresource: *const VkImageSubresource, pLayout: *mut VkSubresourceLayout);
pub type PFN_vkGetImageSparseMemoryRequirements = unsafe extern "C" fn(device: VkDevice, image: VkImage, pSparseMemoryRequirementCount: *mut u32, pSparseMemoryRequirements: *mut VkSparseImageMemoryRequirements);
pub type PFN_vkCreateShaderModule = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkShaderModuleCreateInfo, pAllocator: *const VkAllocationCallbacks, pShaderModule: *mut VkShaderModule) -> VkResult;
pub type PFN_vkDestroyShaderModule = unsafe extern "C" fn(device: VkDevice, shaderModule: VkShaderModule, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreatePipelineCache = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkPipelineCacheCreateInfo, pAllocator: *const VkAllocationCallbacks, pPipelineCache: *mut VkPipelineCache) -> VkResult;
pub type PFN_vkDestroyPipelineCache = unsafe extern "C" fn(device: VkDevice, pipelineCache: VkPipelineCache, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetPipelineCacheData = unsafe extern "C" fn(device: VkDevice, pipelineCache: VkPipelineCache, pDataSize: *mut usize, pData: *mut c_void) -> VkResult;
pub type PFN_vkMergePipelineCaches = unsafe extern "C" fn(device: VkDevice, dstCache: VkPipelineCache, srcCacheCount: u32, pSrcCaches: *const VkPipelineCache) -> VkResult;
pub type PFN_vkCreateComputePipelines = unsafe extern "C" fn(device: VkDevice, pipelineCache: VkPipelineCache, createInfoCount: u32, pCreateInfos: *const VkComputePipelineCreateInfo, pAllocator: *const VkAllocationCallbacks, pPipelines: *mut VkPipeline) -> VkResult;
pub type PFN_vkDestroyPipeline = unsafe extern "C" fn(device: VkDevice, pipeline: VkPipeline, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreatePipelineLayout = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkPipelineLayoutCreateInfo, pAllocator: *const VkAllocationCallbacks, pPipelineLayout: *mut VkPipelineLayout) -> VkResult;
pub type PFN_vkDestroyPipelineLayout = unsafe extern "C" fn(device: VkDevice, pipelineLayout: VkPipelineLayout, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateDescriptorSetLayout = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkDescriptorSetLayoutCreateInfo, pAllocator: *const VkAllocationCallbacks, pSetLayout: *mut VkDescriptorSetLayout) -> VkResult;
pub type PFN_vkDestroyDescriptorSetLayout = unsafe extern "C" fn(device: VkDevice, descriptorSetLayout: VkDescriptorSetLayout, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateDescriptorPool = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkDescriptorPoolCreateInfo, pAllocator: *const VkAllocationCallbacks, pDescriptorPool: *mut VkDescriptorPool) -> VkResult;
pub type PFN_vkDestroyDescriptorPool = unsafe extern "C" fn(device: VkDevice, descriptorPool: VkDescriptorPool, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkResetDescriptorPool = unsafe extern "C" fn(device: VkDevice, descriptorPool: VkDescriptorPool, flags: VkDescriptorPoolResetFlags) -> VkResult;
pub type PFN_vkAllocateDescriptorSets = unsafe extern "C" fn(device: VkDevice, pAllocateInfo: *const VkDescriptorSetAllocateInfo, pDescriptorSets: *mut VkDescriptorSet) -> VkResult;
pub type PFN_vkFreeDescriptorSets = unsafe extern "C" fn(device: VkDevice, descriptorPool: VkDescriptorPool, descriptorSetCount: u32, pDescriptorSets: *const VkDescriptorSet) -> VkResult;
pub type PFN_vkUpdateDescriptorSets = unsafe extern "C" fn(device: VkDevice, descriptorWriteCount: u32, pDescriptorWrites: *const VkWriteDescriptorSet, descriptorCopyCount: u32, pDescriptorCopies: *const VkCopyDescriptorSet);
pub type PFN_vkCreateCommandPool = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkCommandPoolCreateInfo, pAllocator: *const VkAllocationCallbacks, pCommandPool: *mut VkCommandPool) -> VkResult;
pub type PFN_vkDestroyCommandPool = unsafe extern "C" fn(device: VkDevice, commandPool: VkCommandPool, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkResetCommandPool = unsafe extern "C" fn(device: VkDevice, commandPool: VkCommandPool, flags: VkCommandPoolResetFlags) -> VkResult;
pub type PFN_vkAllocateCommandBuffers = unsafe extern "C" fn(device: VkDevice, pAllocateInfo: *const VkCommandBufferAllocateInfo, pCommandBuffers: *mut VkCommandBuffer) -> VkResult;
pub type PFN_vkFreeCommandBuffers = unsafe extern "C" fn(device: VkDevice, commandPool: VkCommandPool, commandBufferCount: u32, pCommandBuffers: *const VkCommandBuffer);
pub type PFN_vkBeginCommandBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, pBeginInfo: *const VkCommandBufferBeginInfo) -> VkResult;
pub type PFN_vkEndCommandBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer) -> VkResult;
pub type PFN_vkResetCommandBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, flags: VkCommandBufferResetFlags) -> VkResult;
pub type PFN_vkCmdBindPipeline = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, pipeline: VkPipeline);
pub type PFN_vkCmdBindDescriptorSets = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, layout: VkPipelineLayout, firstSet: u32, descriptorSetCount: u32, pDescriptorSets: *const VkDescriptorSet, dynamicOffsetCount: u32, pDynamicOffsets: *const u32);
pub type PFN_vkCmdPushConstants = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, layout: VkPipelineLayout, stageFlags: VkShaderStageFlags, offset: u32, size: u32, pValues: *const c_void);
pub type PFN_vkCmdDispatch = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, groupCountX: u32, groupCountY: u32, groupCountZ: u32);
pub type PFN_vkCmdCopyBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, srcBuffer: VkBuffer, dstBuffer: VkBuffer, regionCount: u32, pRegions: *const VkBufferCopy);
pub type PFN_vkCmdFillBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, dstBuffer: VkBuffer, dstOffset: VkDeviceSize, size: VkDeviceSize, data: u32);
pub type PFN_vkCmdUpdateBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, dstBuffer: VkBuffer, dstOffset: VkDeviceSize, dataSize: VkDeviceSize, pData: *const c_void);
pub type PFN_vkCmdPipelineBarrier = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, srcStageMask: VkPipelineStageFlags, dstStageMask: VkPipelineStageFlags, dependencyFlags: VkDependencyFlags, memoryBarrierCount: u32, pMemoryBarriers: *const VkMemoryBarrier, bufferMemoryBarrierCount: u32, pBufferMemoryBarriers: *const VkBufferMemoryBarrier, imageMemoryBarrierCount: u32, pImageMemoryBarriers: *const VkImageMemoryBarrier);
pub type PFN_vkQueueSubmit = unsafe extern "C" fn(queue: VkQueue, submitCount: u32, pSubmits: *const VkSubmitInfo, fence: VkFence) -> VkResult;
pub type PFN_vkQueueWaitIdle = unsafe extern "C" fn(queue: VkQueue) -> VkResult;
pub type PFN_vkCreateFence = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkFenceCreateInfo, pAllocator: *const VkAllocationCallbacks, pFence: *mut VkFence) -> VkResult;
pub type PFN_vkDestroyFence = unsafe extern "C" fn(device: VkDevice, fence: VkFence, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkResetFences = unsafe extern "C" fn(device: VkDevice, fenceCount: u32, pFences: *const VkFence) -> VkResult;
pub type PFN_vkGetFenceStatus = unsafe extern "C" fn(device: VkDevice, fence: VkFence) -> VkResult;
pub type PFN_vkWaitForFences = unsafe extern "C" fn(device: VkDevice, fenceCount: u32, pFences: *const VkFence, waitAll: VkBool32, timeout: u64) -> VkResult;
pub type PFN_vkCreateSemaphore = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkSemaphoreCreateInfo, pAllocator: *const VkAllocationCallbacks, pSemaphore: *mut VkSemaphore) -> VkResult;
pub type PFN_vkDestroySemaphore = unsafe extern "C" fn(device: VkDevice, semaphore: VkSemaphore, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateEvent = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkEventCreateInfo, pAllocator: *const VkAllocationCallbacks, pEvent: *mut VkEvent) -> VkResult;
pub type PFN_vkDestroyEvent = unsafe extern "C" fn(device: VkDevice, event: VkEvent, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetEventStatus = unsafe extern "C" fn(device: VkDevice, event: VkEvent) -> VkResult;
pub type PFN_vkSetEvent = unsafe extern "C" fn(device: VkDevice, event: VkEvent) -> VkResult;
pub type PFN_vkResetEvent = unsafe extern "C" fn(device: VkDevice, event: VkEvent) -> VkResult;
pub type PFN_vkCreateImageView = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkImageViewCreateInfo, pAllocator: *const VkAllocationCallbacks, pView: *mut VkImageView) -> VkResult;
pub type PFN_vkDestroyImageView = unsafe extern "C" fn(device: VkDevice, imageView: VkImageView, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateSampler = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkSamplerCreateInfo, pAllocator: *const VkAllocationCallbacks, pSampler: *mut VkSampler) -> VkResult;
pub type PFN_vkDestroySampler = unsafe extern "C" fn(device: VkDevice, sampler: VkSampler, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateRenderPass = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkRenderPassCreateInfo, pAllocator: *const VkAllocationCallbacks, pRenderPass: *mut VkRenderPass) -> VkResult;
pub type PFN_vkDestroyRenderPass = unsafe extern "C" fn(device: VkDevice, renderPass: VkRenderPass, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetRenderAreaGranularity = unsafe extern "C" fn(device: VkDevice, renderPass: VkRenderPass, pGranularity: *mut VkExtent2D);
pub type PFN_vkCreateFramebuffer = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkFramebufferCreateInfo, pAllocator: *const VkAllocationCallbacks, pFramebuffer: *mut VkFramebuffer) -> VkResult;
pub type PFN_vkDestroyFramebuffer = unsafe extern "C" fn(device: VkDevice, framebuffer: VkFramebuffer, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkCreateGraphicsPipelines = unsafe extern "C" fn(device: VkDevice, pipelineCache: VkPipelineCache, createInfoCount: u32, pCreateInfos: *const VkGraphicsPipelineCreateInfo, pAllocator: *const VkAllocationCallbacks, pPipelines: *mut VkPipeline) -> VkResult;
pub type PFN_vkCmdBeginRenderPass = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, pRenderPassBegin: *const VkRenderPassBeginInfo, contents: VkSubpassContents);
pub type PFN_vkCmdEndRenderPass = unsafe extern "C" fn(commandBuffer: VkCommandBuffer);
pub type PFN_vkCmdNextSubpass = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, contents: VkSubpassContents);
pub type PFN_vkCmdSetViewport = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, firstViewport: u32, viewportCount: u32, pViewports: *const VkViewport);
pub type PFN_vkCmdSetScissor = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, firstScissor: u32, scissorCount: u32, pScissors: *const VkRect2D);
pub type PFN_vkCmdBindVertexBuffers = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, firstBinding: u32, bindingCount: u32, pBuffers: *const VkBuffer, pOffsets: *const VkDeviceSize);
pub type PFN_vkCmdBindIndexBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, buffer: VkBuffer, offset: VkDeviceSize, indexType: VkIndexType);
pub type PFN_vkCmdDraw = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, vertexCount: u32, instanceCount: u32, firstVertex: u32, firstInstance: u32);
pub type PFN_vkCmdDrawIndexed = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, indexCount: u32, instanceCount: u32, firstIndex: u32, vertexOffset: i32, firstInstance: u32);
pub type PFN_vkCmdCopyBufferToImage = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, srcBuffer: VkBuffer, dstImage: VkImage, dstImageLayout: VkImageLayout, regionCount: u32, pRegions: *const VkBufferImageCopy);
pub type PFN_vkCmdCopyImageToBuffer = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, srcImage: VkImage, srcImageLayout: VkImageLayout, dstBuffer: VkBuffer, regionCount: u32, pRegions: *const VkBufferImageCopy);
pub type PFN_vkCmdSetLineWidth = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, lineWidth: f32);
pub type PFN_vkCmdSetDepthBias = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, depthBiasConstantFactor: f32, depthBiasClamp: f32, depthBiasSlopeFactor: f32);
pub type PFN_vkCmdSetBlendConstants = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, blendConstants: *const f32);
pub type PFN_vkCmdSetDepthBounds = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, minDepthBounds: f32, maxDepthBounds: f32);
pub type PFN_vkCmdSetStencilCompareMask = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, faceMask: VkStencilFaceFlags, compareMask: u32);
pub type PFN_vkCmdSetStencilWriteMask = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, faceMask: VkStencilFaceFlags, writeMask: u32);
pub type PFN_vkCmdSetStencilReference = unsafe extern "C" fn(commandBuffer: VkCommandBuffer, faceMask: VkStencilFaceFlags, reference: u32);
pub type PFN_vkDestroySurfaceKHR = unsafe extern "C" fn(instance: VkInstance, surface: VkSurfaceKHR, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetPhysicalDeviceSurfaceSupportKHR = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, queueFamilyIndex: u32, surface: VkSurfaceKHR, pSupported: *mut VkBool32) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceCapabilities: *mut VkSurfaceCapabilitiesKHR) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfaceFormatsKHR = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceFormatCount: *mut u32, pSurfaceFormats: *mut VkSurfaceFormatKHR) -> VkResult;
pub type PFN_vkGetPhysicalDeviceSurfacePresentModesKHR = unsafe extern "C" fn(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pPresentModeCount: *mut u32, pPresentModes: *mut VkPresentModeKHR) -> VkResult;
pub type PFN_vkCreateSwapchainKHR = unsafe extern "C" fn(device: VkDevice, pCreateInfo: *const VkSwapchainCreateInfoKHR, pAllocator: *const VkAllocationCallbacks, pSwapchain: *mut VkSwapchainKHR) -> VkResult;
pub type PFN_vkDestroySwapchainKHR = unsafe extern "C" fn(device: VkDevice, swapchain: VkSwapchainKHR, pAllocator: *const VkAllocationCallbacks);
pub type PFN_vkGetSwapchainImagesKHR = unsafe extern "C" fn(device: VkDevice, swapchain: VkSwapchainKHR, pSwapchainImageCount: *mut u32, pSwapchainImages: *mut VkImage) -> VkResult;
pub type PFN_vkAcquireNextImageKHR = unsafe extern "C" fn(device: VkDevice, swapchain: VkSwapchainKHR, timeout: u64, semaphore: VkSemaphore, fence: VkFence, pImageIndex: *mut u32) -> VkResult;
pub type PFN_vkQueuePresentKHR = unsafe extern "C" fn(queue: VkQueue, pPresentInfo: *const VkPresentInfoKHR) -> VkResult;
