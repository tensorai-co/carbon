//! dxgi.dll: declared so the programs that import it load; each export says it isn't written yet when called.

use super::dll;

dll! {
    "DXGI.dll";
    missing CreateDXGIFactory;
    // Not written yet: what larger programs import.
    missing CreateDXGIFactory1;
}
