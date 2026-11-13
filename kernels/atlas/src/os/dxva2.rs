//! dxva2.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "DXVA2.dll";
    missing DXVA2CreateVideoService;
    // Not written yet: what larger programs import.
    missing DXVA2CreateDirect3DDeviceManager9;
}
