//! d3d11.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "D3D11.dll";
    missing D3D11CreateDevice;
}
