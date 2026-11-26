//! mfplat.dll: Media Foundation platform stubs; `MFStartup` reports `E_NOTIMPL` (as on Windows N editions without
//! the Media Feature Pack), `MFShutdown` succeeds, and `MFTEnumEx` returns zero transforms.

use kernels_core::thread::Cx;
use kernels_core::user;

use super::{Reply, dll};
use crate::process::Task;

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
    host MFShutdown(0) => mf_shutdown;
    host MFStartup(2) => mf_startup;
    host MFTEnum(7) => mft_enum;
    host MFTEnumEx(6) => mft_enum_ex;
    missing MFUnlockDXGIDeviceManager;
}

const E_NOTIMPL: u64 = 0x8000_4001;

fn mf_startup(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(E_NOTIMPL)
}

fn mf_shutdown(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `MFTEnum(category, flags, in_type, out_type, attrs, &clsids, &count)`: zero transforms.
fn mft_enum(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[5] != 0 {
        let _ = task.write_ptr(cx, a[5], 0);
    }
    if a[6] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[6], 0));
    }
    Reply::Value(0)
}

/// `MFTEnumEx(category, flags, input_type, output_type, &activates, &count)`: no Media Foundation transforms.
fn mft_enum_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[4] != 0 {
        let _ = task.write_ptr(cx, a[4], 0);
    }
    if a[5] != 0 {
        let _ = task.mem(cx, true, |t| user::write_u32(&mut t.space(), a[5], 0));
    }
    Reply::Value(0)
}
