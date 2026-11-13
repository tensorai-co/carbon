//! mfplat.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "MFPLAT.dll";
    missing MFCreateAlignedMemoryBuffer;
    missing MFCreateAsyncResult;
    missing MFCreateAttributes;
    missing MFCreateDXGIDeviceManager;
    missing MFCreateDXGISurfaceBuffer;
    missing MFCreateEventQueue;
    missing MFCreateMediaBufferWrapper;
    missing MFCreateMediaEvent;
    missing MFCreateMediaType;
    missing MFCreateMemoryBuffer;
    missing MFCreatePresentationDescriptor;
    missing MFCreateSample;
    missing MFCreateStreamDescriptor;
    missing MFCreateWaveFormatExFromMFMediaType;
    missing MFFrameRateToAverageTimePerFrame;
    missing MFGetSystemTime;
    missing MFInitMediaTypeFromWaveFormatEx;
    missing MFLockDXGIDeviceManager;
    missing MFShutdown;
    missing MFStartup;
    missing MFTEnumEx;
    missing MFUnlockDXGIDeviceManager;
}
