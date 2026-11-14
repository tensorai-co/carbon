//! d3d11.dll: Direct3D 11 over the machine's GPU (`gfx.rs`): devices, their immediate contexts and everything they
//! make. With no GPU, making a device fails with DXGI_ERROR_UNSUPPORTED.

use super::dll;
use super::gfx::*;

gfx_dll! {
    "D3D11.dll";
    host D3D11CreateDevice(10) => d3d11_create_device;
    host D3D11CreateDeviceAndSwapChain(12) => d3d11_create_device_and_swap_chain;
}
