//! mf.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "MF.dll";
    missing MFCreateDeviceSource;
    missing MFEnumDeviceSources;
    missing MFGetService;
}
