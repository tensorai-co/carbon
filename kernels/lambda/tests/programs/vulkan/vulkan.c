// Vulkan compute through lambda's own driver, on the machine's GPU: the Carbon GPU found among the image's drivers,
// memory allocated and mapped, buffers at offsets in it, two descriptor sets (one with a dynamic offset), push
// constants, a compute pipeline from SPIR-V, a fill, a dispatch, an update and a copy in one command buffer, a fence,
// and the same command buffer submitted again after the program changed its input through the mapping. Every word is
// checked against what C computes.
//
// It's a real program, linked with the image's Vulkan loader, so it's built inside the machine:
//
//   carbon run --name vk --gpu host debian:13 -- apt-get install -y gcc libc6-dev libvulkan-dev
//   carbon cp kernels/lambda/tests/programs/vulkan vk:/root
//   carbon run --name vk -- sh -c 'cc -O2 -o /root/vulkan/vulkan /root/vulkan/vulkan.c -lvulkan && /root/vulkan/vulkan'
//   carbon run --name vk --gpu software -- /root/vulkan/vulkan
//
// compute.h is compute.comp as SPIR-V (build.sh).
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <vulkan/vulkan.h>

#include "compute.h"

#define CARBON_VENDOR 0x1c0de
#define COUNT 1000u
#define SPARE 24u
#define MUL 3u
#define BIAS 1000000u

static int checks, failures;

static void check(int ok, const char *what, uint32_t i, uint32_t got, uint32_t want) {
    checks++;
    if (!ok) {
        if (failures++ < 10) printf("FAIL %s [%u]: %#x, not %#x\n", what, i, got, want);
    }
}

#define VK(call)                                                       \
    do {                                                               \
        VkResult r_ = (call);                                          \
        if (r_ != VK_SUCCESS) {                                        \
            printf("FAIL %s: %d\n", #call, r_);                        \
            return 1;                                                  \
        }                                                              \
    } while (0)

static VkDevice device;

static VkBuffer buffer(VkDeviceMemory memory, VkDeviceSize offset, VkDeviceSize size, VkBufferUsageFlags usage) {
    VkBufferCreateInfo info = {VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, 0, 0, size, usage, VK_SHARING_MODE_EXCLUSIVE, 0, 0};
    VkBuffer b;
    if (vkCreateBuffer(device, &info, 0, &b) != VK_SUCCESS) return VK_NULL_HANDLE;
    VkMemoryRequirements req;
    vkGetBufferMemoryRequirements(device, b, &req);
    if (offset % req.alignment != 0 || vkBindBufferMemory(device, b, memory, offset) != VK_SUCCESS) return VK_NULL_HANDLE;
    return b;
}

int main(void) {
    VkApplicationInfo app = {VK_STRUCTURE_TYPE_APPLICATION_INFO, 0, "carbon-vulkan-test", 1, 0, 0, VK_API_VERSION_1_0};
    VkInstanceCreateInfo ici = {VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, 0, 0, &app, 0, 0, 0, 0};
    VkInstance instance;
    VK(vkCreateInstance(&ici, 0, &instance));

    // The Carbon GPU, among whatever else the image has (Mesa's llvmpipe, say).
    uint32_t n = 8;
    VkPhysicalDevice devices[8], physical = VK_NULL_HANDLE;
    vkEnumeratePhysicalDevices(instance, &n, devices);
    VkPhysicalDeviceProperties props;
    for (uint32_t i = 0; i < n; i++) {
        vkGetPhysicalDeviceProperties(devices[i], &props);
        if (props.vendorID == CARBON_VENDOR) {
            physical = devices[i];
            break;
        }
    }
    if (!physical) {
        printf("FAIL no Carbon GPU among %u devices\n", n);
        return 1;
    }
    printf("vulkan: %s\n", props.deviceName);

    float priority = 1;
    VkDeviceQueueCreateInfo qci = {VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO, 0, 0, 0, 1, &priority};
    VkDeviceCreateInfo dci = {VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, 0, 0, 1, &qci, 0, 0, 0, 0, 0};
    VK(vkCreateDevice(physical, &dci, 0, &device));
    VkQueue queue;
    vkGetDeviceQueue(device, 0, 0, &queue);

    // Two allocations: the input and the parameters in one, the output and a scratch buffer in the other, at offsets.
    VkMemoryAllocateInfo mai = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, 0, 1 << 16, 0};
    VkDeviceMemory in_memory, out_memory;
    VK(vkAllocateMemory(device, &mai, 0, &in_memory));
    VK(vkAllocateMemory(device, &mai, 0, &out_memory));
    uint8_t *in_map, *out_map;
    VK(vkMapMemory(device, in_memory, 0, VK_WHOLE_SIZE, 0, (void **)&in_map));
    VK(vkMapMemory(device, out_memory, 0, VK_WHOLE_SIZE, 0, (void **)&out_map));
    uint32_t *src = (uint32_t *)in_map;
    // The parameters 256 bytes past the descriptor's offset, reached by the dynamic offset.
    uint32_t *bias = (uint32_t *)(in_map + 8192 + 256);
    uint32_t *dst = (uint32_t *)(out_map + 256);
    uint32_t *scratch = (uint32_t *)(out_map + 16384);
    for (uint32_t i = 0; i < COUNT; i++) src[i] = i * 0x01010101u;
    *bias = BIAS;
    memset(out_map, 0x55, 1 << 16);

    VkBuffer src_buf = buffer(in_memory, 0, COUNT * 4, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT);
    VkBuffer params_buf = buffer(in_memory, 8192, 512, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT);
    VkBuffer dst_buf = buffer(out_memory, 256, (COUNT + SPARE) * 4, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    VkBuffer scratch_buf = buffer(out_memory, 16384, 256, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    if (!src_buf || !params_buf || !dst_buf || !scratch_buf) {
        printf("FAIL making buffers\n");
        return 1;
    }

    VkDescriptorSetLayoutBinding set0[2] = {
        {0, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, 0},
        {1, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1, VK_SHADER_STAGE_COMPUTE_BIT, 0},
    };
    VkDescriptorSetLayoutBinding set1[1] = {{0, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC, 1, VK_SHADER_STAGE_COMPUTE_BIT, 0}};
    VkDescriptorSetLayout layouts[2];
    VkDescriptorSetLayoutCreateInfo l0 = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, 0, 0, 2, set0};
    VkDescriptorSetLayoutCreateInfo l1 = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO, 0, 0, 1, set1};
    VK(vkCreateDescriptorSetLayout(device, &l0, 0, &layouts[0]));
    VK(vkCreateDescriptorSetLayout(device, &l1, 0, &layouts[1]));
    VkPushConstantRange push_range = {VK_SHADER_STAGE_COMPUTE_BIT, 0, 8};
    VkPipelineLayoutCreateInfo plci = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO, 0, 0, 2, layouts, 1, &push_range};
    VkPipelineLayout layout;
    VK(vkCreatePipelineLayout(device, &plci, 0, &layout));

    VkDescriptorPoolSize sizes[2] = {{VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 2}, {VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC, 1}};
    VkDescriptorPoolCreateInfo dpci = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO, 0, 0, 2, 2, sizes};
    VkDescriptorPool pool;
    VK(vkCreateDescriptorPool(device, &dpci, 0, &pool));
    VkDescriptorSetAllocateInfo dsai = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO, 0, pool, 2, layouts};
    VkDescriptorSet sets[2];
    VK(vkAllocateDescriptorSets(device, &dsai, sets));
    VkDescriptorBufferInfo infos[3] = {{src_buf, 0, VK_WHOLE_SIZE}, {dst_buf, 0, VK_WHOLE_SIZE}, {params_buf, 0, 16}};
    VkWriteDescriptorSet writes[2] = {
        {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, 0, sets[0], 0, 0, 2, VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 0, &infos[0], 0},
        {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, 0, sets[1], 0, 0, 1, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC, 0, &infos[2], 0},
    };
    vkUpdateDescriptorSets(device, 2, writes, 0, 0);

    VkShaderModuleCreateInfo smci = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO, 0, 0, sizeof compute_spv, compute_spv};
    VkShaderModule module;
    VK(vkCreateShaderModule(device, &smci, 0, &module));
    VkComputePipelineCreateInfo cpci = {
        VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO, 0, 0,
        {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, 0, 0, VK_SHADER_STAGE_COMPUTE_BIT, module, "main", 0},
        layout, VK_NULL_HANDLE, -1};
    VkPipeline pipeline;
    VK(vkCreateComputePipelines(device, VK_NULL_HANDLE, 1, &cpci, 0, &pipeline));

    VkCommandPoolCreateInfo cpi = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, 0, 0, 0};
    VkCommandPool cpool;
    VK(vkCreateCommandPool(device, &cpi, 0, &cpool));
    VkCommandBufferAllocateInfo cbai = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO, 0, cpool, VK_COMMAND_BUFFER_LEVEL_PRIMARY, 1};
    VkCommandBuffer cb;
    VK(vkAllocateCommandBuffers(device, &cbai, &cb));
    VkCommandBufferBeginInfo begin = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO, 0, 0, 0};
    VK(vkBeginCommandBuffer(cb, &begin));
    VkMemoryBarrier barrier = {VK_STRUCTURE_TYPE_MEMORY_BARRIER, 0, VK_ACCESS_SHADER_WRITE_BIT | VK_ACCESS_TRANSFER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_TRANSFER_READ_BIT};
    vkCmdFillBuffer(cb, dst_buf, 0, VK_WHOLE_SIZE, 0xdeadbeef);
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, 1, &barrier, 0, 0, 0, 0);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
    uint32_t dynamic_offset = 256;
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout, 0, 2, sets, 1, &dynamic_offset);
    uint32_t push[2] = {MUL, COUNT};
    vkCmdPushConstants(cb, layout, VK_SHADER_STAGE_COMPUTE_BIT, 0, 8, push);
    vkCmdDispatch(cb, (COUNT + 63) / 64, 1, 1);
    vkCmdPipelineBarrier(cb, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 1, &barrier, 0, 0, 0, 0);
    uint32_t words[4] = {0x11111111, 0x22222222, 0x33333333, 0x44444444};
    vkCmdUpdateBuffer(cb, scratch_buf, 64, sizeof words, words);
    VkBufferCopy region = {0, 0, 64};
    vkCmdCopyBuffer(cb, dst_buf, scratch_buf, 1, &region);
    VK(vkEndCommandBuffer(cb));

    VkFenceCreateInfo fci = {VK_STRUCTURE_TYPE_FENCE_CREATE_INFO, 0, 0};
    VkFence fence;
    VK(vkCreateFence(device, &fci, 0, &fence));
    VkSubmitInfo submit = {VK_STRUCTURE_TYPE_SUBMIT_INFO, 0, 0, 0, 0, 1, &cb, 0, 0};

    for (int round = 0; round < 2; round++) {
        // The second time, new input through the mapping, and the same command buffer.
        if (round == 1) {
            for (uint32_t i = 0; i < COUNT; i++) src[i] = i * 7u + 3u;
        }
        memset(scratch, 0, 256);
        VK(vkResetFences(device, 1, &fence));
        VK(vkQueueSubmit(queue, 1, &submit, fence));
        VK(vkWaitForFences(device, 1, &fence, VK_TRUE, UINT64_MAX));
        for (uint32_t i = 0; i < COUNT; i++) {
            uint32_t want = src[i] * MUL + BIAS + i;
            check(dst[i] == want, "dispatch", i, dst[i], want);
        }
        for (uint32_t i = COUNT; i < COUNT + SPARE; i++) check(dst[i] == 0xdeadbeef, "fill past the count", i, dst[i], 0xdeadbeef);
        for (uint32_t i = 0; i < 16; i++) check(scratch[i] == dst[i], "copy", i, scratch[i], dst[i]);
        for (uint32_t i = 0; i < 4; i++) check(scratch[16 + i] == words[i], "update", i, scratch[16 + i], words[i]);
        check(((uint32_t *)out_map)[0] == 0x55555555, "memory before the output untouched", 0, ((uint32_t *)out_map)[0], 0x55555555);
        check(scratch[20] == 0, "scratch past the update untouched", 20, scratch[20], 0);
    }

    vkDestroyFence(device, fence, 0);
    vkDestroyCommandPool(device, cpool, 0);
    vkDestroyPipeline(device, pipeline, 0);
    vkDestroyShaderModule(device, module, 0);
    vkDestroyDescriptorPool(device, pool, 0);
    vkDestroyPipelineLayout(device, layout, 0);
    vkDestroyDescriptorSetLayout(device, layouts[0], 0);
    vkDestroyDescriptorSetLayout(device, layouts[1], 0);
    vkDestroyBuffer(device, src_buf, 0);
    vkDestroyBuffer(device, params_buf, 0);
    vkDestroyBuffer(device, dst_buf, 0);
    vkDestroyBuffer(device, scratch_buf, 0);
    vkFreeMemory(device, in_memory, 0);
    vkFreeMemory(device, out_memory, 0);
    vkDestroyDevice(device, 0);
    vkDestroyInstance(instance, 0);
    printf("vulkan: %d checks, %d failed\n", checks, failures);
    return failures != 0;
}
