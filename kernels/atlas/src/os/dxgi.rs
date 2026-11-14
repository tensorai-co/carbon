//! dxgi.dll: DXGI over the machine's GPU (`gfx.rs`): its factory, adapter, output and swap chains. With no GPU
//! (`--gpu none`), making a factory fails with DXGI_ERROR_UNSUPPORTED, as on a Windows without one, and programs
//! draw without it.

use super::dll;
use super::gfx::*;

gfx_dll! {
    "DXGI.dll";
    host CreateDXGIFactory(2) => create_factory;
    host CreateDXGIFactory1(2) => create_factory;
    host CreateDXGIFactory2(3) => create_factory2;
}
