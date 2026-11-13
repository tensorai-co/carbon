//! d3d9.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "D3D9.dll";
    missing D3DPERF_BeginEvent;
    missing D3DPERF_EndEvent;
    missing D3DPERF_GetStatus;
    missing D3DPERF_SetMarker;
    missing Direct3DCreate9;
    missing Direct3DCreate9Ex;
}
