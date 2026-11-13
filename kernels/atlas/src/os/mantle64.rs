//! mantle64.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "MANTLE64.dll";
    missing grAllocMemory;
    missing grAttachImageViewDescriptors;
    missing grAttachMemoryViewDescriptors;
    missing grAttachSamplerDescriptors;
    missing grBeginCommandBuffer;
    missing grBeginDescriptorSetUpdate;
    missing grBindObjectMemory;
    missing grCmdBindDescriptorSet;
    missing grCmdBindPipeline;
    missing grCmdBindStateObject;
    missing grCmdBindTargets;
    missing grCmdCopyImage;
    missing grCmdDraw;
    missing grCmdPrepareImages;
    missing grCmdPrepareMemoryRegions;
    missing grCmdUpdateMemory;
    missing grCreateColorBlendState;
    missing grCreateColorTargetView;
    missing grCreateCommandBuffer;
    missing grCreateDepthStencilState;
    missing grCreateDescriptorSet;
    missing grCreateFence;
    missing grCreateGraphicsPipeline;
    missing grCreateImage;
    missing grCreateImageView;
    missing grCreateMsaaState;
    missing grCreateQueueSemaphore;
    missing grCreateRasterState;
    missing grCreateSampler;
    missing grCreateShader;
    missing grCreateViewportState;
    missing grDestroyObject;
    missing grEndCommandBuffer;
    missing grEndDescriptorSetUpdate;
    missing grFreeMemory;
    missing grGetDeviceQueue;
    missing grGetFenceStatus;
    missing grGetImageSubresourceInfo;
    missing grGetMemoryHeapCount;
    missing grGetMemoryHeapInfo;
    missing grGetObjectInfo;
    missing grMapMemory;
    missing grQueueSubmit;
    missing grQueueWaitIdle;
    missing grSignalQueueSemaphore;
    missing grUnmapMemory;
    missing grWaitQueueSemaphore;
}
