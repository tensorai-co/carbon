//! DXGI and Direct3D 11's objects: COM objects of atlas's (`com.rs`) whose methods read their arguments from the
//! program's memory and call `gpu_api::d3d11`, which does Direct3D 11 on the machine's GPU.
//!
//! - **DXGI**: a factory (`IDXGIFactory` to `IDXGIFactory6`) with one adapter, the Carbon GPU (its vendor and device
//!   IDs, the host device's name, a software adapter's flag for the CPU device), which has one output, the desktop's
//!   screen; swap chains for a window, blit or flip model, whose back buffer is a Direct3D 11 texture and whose
//!   `Present` reads it back into the window's canvas, paced at 60 Hz when asked to wait. Full screen is refused.
//! - **Direct3D 11**: `D3D11CreateDevice` and `D3D11CreateDeviceAndSwapChain` at feature levels 10_0 to 11_0; the
//!   device (also an `IDXGIDevice`), its immediate context, buffers, 2D textures (also `IDXGISurface`s), views,
//!   states, shaders, input layouts and queries (which are done as soon as they're asked, since the context waits for
//!   its work).
//!
//! Each object has a vtable for each interface it is, of exports in the DLL that made it. Both DLLs declare every
//! method (`gfx_dll!`), so an object made through DXGI or Direct3D has its vtables in a DLL the program has loaded,
//! and its children have their parent's. A method that isn't written is `missing`, and ends the process naming it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpu_api::d3d11::{self as d3d, Id};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

#[allow(unused_imports)]
pub use super::com::{com_add_ref, com_query_interface, com_release};
use super::Reply;
use super::com::{Guid, Object, guid};
use crate::process::Task;

const S_OK: u64 = 0;
const S_FALSE: u64 = 1;
const E_NOTIMPL: u64 = 0x8000_4001;
const E_NOINTERFACE: u64 = 0x8000_4002;
const E_POINTER: u64 = 0x8000_4003;
const E_FAIL: u64 = 0x8000_4005;
const E_INVALIDARG: u64 = 0x8007_0057;
const E_OUTOFMEMORY: u64 = 0x8007_000e;
const DXGI_ERROR_NOT_FOUND: u64 = 0x887a_0002;
const DXGI_ERROR_MORE_DATA: u64 = 0x887a_0003;
pub const DXGI_ERROR_UNSUPPORTED: u64 = 0x887a_0004;
const DXGI_ERROR_FRAME_STATISTICS_DISJOINT: u64 = 0x887a_000b;
const DXGI_ERROR_NOT_CURRENTLY_AVAILABLE: u64 = 0x887a_0022;

// ---- Interface IDs ----

const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_DXGI_OBJECT: Guid = guid(0xaec22fb8, 0x76f3, 0x4639, [0x9b, 0xe0, 0x28, 0xeb, 0x43, 0xa6, 0x7a, 0x2e]);
const IID_DXGI_DEVICE_SUB_OBJECT: Guid = guid(0x3d3e0379, 0xf9de, 0x4d58, [0xbb, 0x6c, 0x18, 0xd6, 0x29, 0x92, 0xf1, 0xa6]);
const IID_DXGI_SURFACE: Guid = guid(0xcafcb56c, 0x6ac3, 0x4889, [0xbf, 0x47, 0x9e, 0x23, 0xbb, 0xd2, 0x60, 0xec]);
const IID_DXGI_ADAPTER: Guid = guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
const IID_DXGI_ADAPTER1: Guid = guid(0x29038f61, 0x3839, 0x4626, [0x91, 0xfd, 0x08, 0x68, 0x79, 0x01, 0x1a, 0x05]);
const IID_DXGI_ADAPTER2: Guid = guid(0x0aa1ae0a, 0xfa0e, 0x4b84, [0x86, 0x44, 0xe0, 0x5f, 0xf8, 0xe5, 0xac, 0xb5]);
const IID_DXGI_ADAPTER3: Guid = guid(0x645967a4, 0x1392, 0x4310, [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd]);
const IID_DXGI_ADAPTER4: Guid = guid(0x3c8d99d1, 0x4fbf, 0x4181, [0xa8, 0x2c, 0xaf, 0x66, 0xbf, 0x7b, 0xd2, 0x4e]);
const IID_DXGI_OUTPUT: Guid = guid(0xae02eedb, 0xc735, 0x4690, [0x8d, 0x52, 0x5a, 0x8d, 0xc2, 0x02, 0x13, 0xaa]);
const IID_DXGI_SWAP_CHAIN: Guid = guid(0x310d36a0, 0xd2e7, 0x4c0a, [0xaa, 0x04, 0x6a, 0x9d, 0x23, 0xb8, 0x88, 0x6a]);
const IID_DXGI_SWAP_CHAIN1: Guid = guid(0x790a45f7, 0x0d42, 0x4876, [0x98, 0x3a, 0x0a, 0x55, 0xcf, 0xe6, 0xf4, 0xaa]);
const IID_DXGI_SWAP_CHAIN2: Guid = guid(0xa8be2ac4, 0x199f, 0x4946, [0xb3, 0x31, 0x79, 0x59, 0x9f, 0xb9, 0x8d, 0xe7]);
const IID_DXGI_SWAP_CHAIN3: Guid = guid(0x94d99bdb, 0xf1f8, 0x4ab0, [0xb2, 0x36, 0x7d, 0xa0, 0x17, 0x0e, 0xda, 0xb1]);
const IID_DXGI_FACTORY: Guid = guid(0x7b7166ec, 0x21c7, 0x44ae, [0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69]);
const IID_DXGI_FACTORY1: Guid = guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
const IID_DXGI_FACTORY2: Guid = guid(0x50c83a1c, 0xe072, 0x4c48, [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0]);
const IID_DXGI_FACTORY3: Guid = guid(0x25483823, 0xcd46, 0x4c7d, [0x86, 0xca, 0x47, 0xaa, 0x95, 0xb8, 0x37, 0xbd]);
const IID_DXGI_FACTORY4: Guid = guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
const IID_DXGI_FACTORY5: Guid = guid(0x7632e1f5, 0xee65, 0x4dca, [0x87, 0xfd, 0x84, 0xcd, 0x75, 0xf8, 0x83, 0x8d]);
const IID_DXGI_FACTORY6: Guid = guid(0xc1b6694f, 0xff09, 0x44a9, [0xb0, 0x3c, 0x77, 0x90, 0x0a, 0x0a, 0x1d, 0x17]);
const IID_DXGI_DEVICE: Guid = guid(0x54ec77fa, 0x1377, 0x44e6, [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c]);
const IID_DXGI_DEVICE1: Guid = guid(0x77db970f, 0x6276, 0x48ba, [0xba, 0x28, 0x07, 0x01, 0x43, 0xb4, 0x39, 0x2c]);
const IID_DXGI_DEVICE2: Guid = guid(0x05008617, 0xfbfd, 0x4051, [0xa7, 0x90, 0x14, 0x48, 0x84, 0xb4, 0xf6, 0xa9]);
const IID_DXGI_DEVICE3: Guid = guid(0x6007896c, 0x3244, 0x4afd, [0xbf, 0x18, 0xa6, 0xd3, 0xbe, 0xda, 0x50, 0x23]);
const IID_D3D11_DEVICE_CHILD: Guid = guid(0x1841e5c8, 0x16b0, 0x489b, [0xbc, 0xc8, 0x44, 0xcf, 0xb0, 0xd5, 0xde, 0xae]);
const IID_D3D11_DEPTH_STENCIL_STATE: Guid = guid(0x03823efb, 0x8d8f, 0x4e1c, [0x9a, 0xa2, 0xf6, 0x4b, 0xb2, 0xcb, 0xfd, 0xf1]);
const IID_D3D11_BLEND_STATE: Guid = guid(0x75b68faa, 0x347d, 0x4159, [0x8f, 0x45, 0xa0, 0x64, 0x0f, 0x01, 0xcd, 0x9a]);
const IID_D3D11_RASTERIZER_STATE: Guid = guid(0x9bb4ab81, 0xab1a, 0x4d8f, [0xb5, 0x06, 0xfc, 0x04, 0x20, 0x0b, 0x6e, 0xe7]);
const IID_D3D11_RESOURCE: Guid = guid(0xdc8e63f3, 0xd12b, 0x4952, [0xb4, 0x7b, 0x5e, 0x45, 0x02, 0x6a, 0x86, 0x2d]);
const IID_D3D11_BUFFER: Guid = guid(0x48570b85, 0xd1ee, 0x4fcd, [0xa2, 0x50, 0xeb, 0x35, 0x07, 0x22, 0xb0, 0x37]);
const IID_D3D11_TEXTURE2D: Guid = guid(0x6f15aaf2, 0xd208, 0x4e89, [0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c]);
const IID_D3D11_VIEW: Guid = guid(0x839d1216, 0xbb2e, 0x412b, [0xb7, 0xf4, 0xa9, 0xdb, 0xeb, 0xe0, 0x8e, 0xd1]);
const IID_D3D11_SRV: Guid = guid(0xb0e06fe0, 0x8192, 0x4e1a, [0xb1, 0xca, 0x36, 0xd7, 0x41, 0x47, 0x10, 0xb2]);
const IID_D3D11_RTV: Guid = guid(0xdfdba067, 0x0b8d, 0x4865, [0x87, 0x5b, 0xd7, 0xb4, 0x51, 0x6c, 0xc1, 0x64]);
const IID_D3D11_DSV: Guid = guid(0x9fdac92a, 0x1876, 0x48c3, [0xaf, 0xad, 0x25, 0xb9, 0x4f, 0x84, 0xa9, 0xb6]);
const IID_D3D11_UAV: Guid = guid(0x28acf509, 0x7f5c, 0x48f6, [0x86, 0x11, 0xf3, 0x16, 0x01, 0x0a, 0x63, 0x80]);
const IID_D3D11_VERTEX_SHADER: Guid = guid(0x3b301d64, 0xd678, 0x4289, [0x88, 0x97, 0x22, 0xf8, 0x92, 0x8b, 0x72, 0xf3]);
const IID_D3D11_PIXEL_SHADER: Guid = guid(0xea82e40d, 0x51dc, 0x4f33, [0x93, 0xd4, 0xdb, 0x7c, 0x91, 0x25, 0xae, 0x8c]);
const IID_D3D11_COMPUTE_SHADER: Guid = guid(0x4f5b196e, 0xc2bd, 0x495e, [0xbd, 0x01, 0x1f, 0xde, 0xd3, 0x8e, 0x49, 0x69]);
const IID_D3D11_INPUT_LAYOUT: Guid = guid(0xe4819ddc, 0x4cf0, 0x4025, [0xbd, 0x26, 0x5d, 0xe8, 0x2a, 0x3e, 0x07, 0xb7]);
const IID_D3D11_SAMPLER_STATE: Guid = guid(0xda6fea51, 0x564c, 0x4487, [0x98, 0x10, 0xf0, 0xd0, 0xf9, 0xb4, 0xe3, 0xa5]);
const IID_D3D11_ASYNCHRONOUS: Guid = guid(0x4b35d0cd, 0x1e15, 0x4258, [0x9c, 0x98, 0x1b, 0x13, 0x33, 0xf6, 0xdd, 0x3b]);
const IID_D3D11_QUERY: Guid = guid(0xd6c00747, 0x87b7, 0x425e, [0xb8, 0x4d, 0x44, 0xd1, 0x08, 0x56, 0x0a, 0xfd]);
const IID_D3D11_DEVICE_CONTEXT: Guid = guid(0xc0bfa96c, 0xe089, 0x44fb, [0x8e, 0xaf, 0x26, 0xf8, 0x79, 0x61, 0x90, 0xda]);
const IID_D3D11_DEVICE: Guid = guid(0xdb6f6ddb, 0xac77, 0x4e88, [0x82, 0x53, 0x81, 0x9d, 0xf9, 0xbb, 0xf1, 0x40]);

// ---- Objects ----

/// A device and its immediate context, shared by every object of the device's.
pub struct Dev {
    pub d: d3d::Device,
    /// Its feature level (`D3D_FEATURE_LEVEL`).
    pub level: u32,
    /// The device's and immediate context's objects.
    pub device: u64,
    pub context: u64,
    /// Where each mapped subresource is in the program's memory, how big, and how it was mapped.
    pub maps: HashMap<(Id, u32), (u64, u32, u32)>,
}

impl std::fmt::Debug for Dev {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dev").field("level", &self.level).finish()
    }
}

pub type Shared = Arc<Mutex<Dev>>;

/// A swap chain: its window, back buffer and description.
#[derive(Debug, Clone)]
pub struct Swap {
    pub hwnd: u64,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub count: u32,
    pub usage: u32,
    pub effect: u32,
    pub flags: u32,
    /// The back buffer's object and texture.
    pub back: u64,
    pub texture: Id,
    pub presents: u32,
    pub last: Option<Instant>,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Factory,
    Adapter,
    Output,
    Device,
    Context,
    Buffer(Id),
    Texture(Id),
    /// A view: its resource's object, its interface, and its description as given (for `GetDesc`).
    View { id: Id, resource: u64, iid: Guid, desc: Vec<u8> },
    /// A blend, depth-stencil, rasterizer or sampler state.
    State { id: Id, iid: Guid, desc: Vec<u8> },
    Shader { id: Id, iid: Guid },
    Layout(Id),
    Query { kind: u32, desc: Vec<u8> },
    SwapChain(Box<Swap>),
}

#[derive(Debug, Clone)]
pub struct Gfx {
    pub kind: Kind,
    /// The DLL its vtables are in.
    pub dll: &'static str,
    pub dev: Option<Shared>,
    /// `SetPrivateData`'s.
    pub private: Vec<(Guid, Vec<u8>)>,
}

/// An object's interfaces, by their pointers' order.
pub fn tables_of(kind: &Kind) -> Vec<&'static [&'static str]> {
    match kind {
        Kind::Factory => vec![&FACTORY],
        Kind::Adapter => vec![&ADAPTER],
        Kind::Output => vec![&OUTPUT],
        Kind::Device => vec![&DEVICE, &DXGI_DEVICE],
        Kind::Context => vec![&CONTEXT],
        Kind::Buffer(_) => vec![&RESOURCE],
        Kind::Texture(_) => vec![&RESOURCE, &SURFACE],
        Kind::View { .. } => vec![&VIEW],
        Kind::State { .. } => vec![&STATE],
        Kind::Shader { .. } | Kind::Layout(_) => vec![&CHILD],
        Kind::Query { .. } => vec![&QUERY],
        Kind::SwapChain(_) => vec![&SWAP_CHAIN],
    }
}

/// Which of an object's interface pointers is the interface asked for.
pub fn answers(g: &Gfx, iid: &Guid) -> Option<usize> {
    let child = [IID_IUNKNOWN, IID_D3D11_DEVICE_CHILD];
    let object = [IID_IUNKNOWN, IID_DXGI_OBJECT];
    let is = |list: &[Guid]| list.contains(iid);
    match &g.kind {
        Kind::Factory => (is(&object) || is(&[IID_DXGI_FACTORY, IID_DXGI_FACTORY1, IID_DXGI_FACTORY2, IID_DXGI_FACTORY3, IID_DXGI_FACTORY4, IID_DXGI_FACTORY5, IID_DXGI_FACTORY6])).then_some(0),
        Kind::Adapter => (is(&object) || is(&[IID_DXGI_ADAPTER, IID_DXGI_ADAPTER1, IID_DXGI_ADAPTER2, IID_DXGI_ADAPTER3, IID_DXGI_ADAPTER4])).then_some(0),
        Kind::Output => (is(&object) || *iid == IID_DXGI_OUTPUT).then_some(0),
        Kind::Device => {
            if *iid == IID_IUNKNOWN || *iid == IID_D3D11_DEVICE {
                Some(0)
            } else {
                is(&[IID_DXGI_OBJECT, IID_DXGI_DEVICE, IID_DXGI_DEVICE1, IID_DXGI_DEVICE2, IID_DXGI_DEVICE3]).then_some(1)
            }
        }
        Kind::Context => (is(&child) || *iid == IID_D3D11_DEVICE_CONTEXT).then_some(0),
        Kind::Buffer(_) => (is(&child) || is(&[IID_D3D11_RESOURCE, IID_D3D11_BUFFER])).then_some(0),
        Kind::Texture(_) => {
            if is(&child) || is(&[IID_D3D11_RESOURCE, IID_D3D11_TEXTURE2D]) {
                Some(0)
            } else {
                is(&[IID_DXGI_OBJECT, IID_DXGI_DEVICE_SUB_OBJECT, IID_DXGI_SURFACE]).then_some(1)
            }
        }
        Kind::View { iid: own, .. } => (is(&child) || *iid == IID_D3D11_VIEW || iid == own).then_some(0),
        Kind::State { iid: own, .. } | Kind::Shader { iid: own, .. } => (is(&child) || iid == own).then_some(0),
        Kind::Layout(_) => (is(&child) || *iid == IID_D3D11_INPUT_LAYOUT).then_some(0),
        Kind::Query { .. } => (is(&child) || is(&[IID_D3D11_ASYNCHRONOUS, IID_D3D11_QUERY])).then_some(0),
        Kind::SwapChain(_) => (is(&object) || is(&[IID_DXGI_DEVICE_SUB_OBJECT, IID_DXGI_SWAP_CHAIN, IID_DXGI_SWAP_CHAIN1, IID_DXGI_SWAP_CHAIN2, IID_DXGI_SWAP_CHAIN3])).then_some(0),
    }
}

/// What an object's last release lets go of: its resource on the device, what a view or swap chain holds.
pub fn released(task: &mut Task, cx: &mut Cx<'_>, g: Gfx) -> Result<(), Reply> {
    let id = match &g.kind {
        Kind::Buffer(id) | Kind::Texture(id) | Kind::Layout(id) => Some(*id),
        Kind::View { id, .. } | Kind::State { id, .. } | Kind::Shader { id, .. } => Some(*id),
        _ => None,
    };
    if let (Some(id), Some(dev)) = (id, &g.dev) {
        let mut dev = lock(dev);
        dev.maps.retain(|k, _| k.0 != id);
        dev.d.release(id);
    }
    match g.kind {
        Kind::View { resource, .. } => {
            task.release(cx, resource)?;
        }
        Kind::SwapChain(s) => {
            task.release(cx, s.back)?;
        }
        _ => {}
    }
    Ok(())
}

impl Task {
    fn make_gfx(&mut self, cx: &mut Cx<'_>, dll: &'static str, kind: Kind, dev: Option<Shared>) -> Result<u64, Reply> {
        let tables = tables_of(&kind);
        self.make_object(cx, dll, &tables, Object::Gfx(Gfx { kind, dll, dev, private: Vec::new() }))
    }

    fn gfx_read(&mut self, cx: &mut Cx<'_>, at: u64, len: usize) -> Option<Vec<u8>> {
        if at == 0 {
            return None;
        }
        self.mem(cx, false, |t| user::read_vec(&t.space(), at, len)).ok()
    }

    fn gfx_put(&mut self, cx: &mut Cx<'_>, at: u64, bytes: &[u8]) -> bool {
        at != 0 && self.mem(cx, true, |t| user::write(&mut t.space(), at, bytes)).is_ok()
    }

    fn gfx_put_u32(&mut self, cx: &mut Cx<'_>, at: u64, v: u32) -> bool {
        self.gfx_put(cx, at, &v.to_le_bytes())
    }

    /// `n` pointers from an array in the program's memory.
    fn gfx_ptrs(&mut self, cx: &mut Cx<'_>, at: u64, n: u64) -> Vec<u64> {
        let abi = self.abi();
        let n = n.min(256);
        match self.gfx_read(cx, at, (n * abi.width) as usize) {
            Some(b) => (0..n as usize).map(|i| abi.ptr_at(&b, i * abi.width as usize)).collect(),
            None => vec![0; n as usize],
        }
    }

    /// Gives `base`'s interface `iid` to `out`, and lets go of the reference made with it.
    fn gfx_give_as(&mut self, cx: &mut Cx<'_>, base: u64, iid: u64, out: u64) -> Reply {
        if base == 0 {
            let _ = self.write_ptr(cx, out, 0);
            return Reply::Value(E_OUTOFMEMORY);
        }
        let Some(iid) = self.read_guid(cx, iid) else { return Reply::Value(E_POINTER) };
        let hr = match self.query_interface(cx, base, &iid, out) {
            Ok(hr) => hr,
            Err(reply) => return reply,
        };
        if let Err(reply) = self.release(cx, base) {
            return reply;
        }
        Reply::Value(hr)
    }

    fn gfx_give(&mut self, cx: &mut Cx<'_>, base: u64, out: u64) -> Reply {
        if out == 0 {
            return Reply::Value(S_FALSE);
        }
        let _ = self.write_ptr(cx, out, base);
        Reply::Value(if base == 0 { E_OUTOFMEMORY } else { S_OK })
    }

    fn gfx_id(&self, ptr: u64) -> Option<Id> {
        if ptr == 0 {
            return None;
        }
        match self.gfx(ptr)?.kind {
            Kind::Buffer(id) | Kind::Texture(id) | Kind::Layout(id) => Some(id),
            Kind::View { id, .. } | Kind::State { id, .. } | Kind::Shader { id, .. } => Some(id),
            _ => None,
        }
    }

    fn gfx_ids(&mut self, cx: &mut Cx<'_>, at: u64, n: u64) -> Vec<Option<Id>> {
        self.gfx_ptrs(cx, at, n).into_iter().map(|p| self.gfx_id(p)).collect()
    }

    fn trace_gfx(&self, what: &str) {
        if self.k.options.trace || std::env::var_os("CARBON_TRACE_GFX").is_some() {
            self.k.host.write(host::Stream::Err, format!("atlas: d3d11: {what}\n").as_bytes());
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(b, at))
}

fn hr(e: d3d::Error) -> u64 {
    u64::from(e.hresult)
}

/// The device of the object a method's `this` is, with the object.
fn dev_of(task: &Task, this: u64) -> Option<(Gfx, Shared)> {
    let g = task.gfx(this)?;
    let dev = g.dev.clone()?;
    Some((g, dev))
}

macro_rules! dev_or {
    ($task:expr, $this:expr, $fail:expr) => {
        match dev_of($task, $this) {
            Some(x) => x,
            None => return Reply::Value($fail),
        }
    };
}

// ---- Shared ----

pub fn s_ok(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

pub fn true_(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(1)
}

pub fn not_implemented(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(E_NOTIMPL)
}

/// A method that gives back a pointer, which is none.
pub fn zero_out(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let _ = task.write_ptr(cx, a[1], 0);
    Reply::Value(S_OK)
}

pub fn zero_out32(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[1], 0);
    Reply::Value(S_OK)
}

pub fn frame_statistics(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(DXGI_ERROR_FRAME_STATISTICS_DISJOINT)
}

pub fn get_maximum_frame_latency(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[1], 3);
    Reply::Value(S_OK)
}

/// `SetPrivateData(guid, size, data)`.
pub fn set_private_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.read_guid(cx, a[1]) else { return Reply::Value(E_POINTER) };
    let data = task.gfx_read(cx, a[3], (a[2] as u32 as usize).min(1 << 16)).unwrap_or_default();
    let done = task.gfx_mut(a[0], |o| {
        o.private.retain(|(k, _)| *k != g);
        if a[3] != 0 {
            o.private.push((g, data));
        }
    });
    Reply::Value(if done.is_some() { S_OK } else { E_INVALIDARG })
}

pub fn set_private_data_interface(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

/// `GetPrivateData(guid, &size, data)`.
pub fn get_private_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.read_guid(cx, a[1]) else { return Reply::Value(E_POINTER) };
    let found = task.gfx(a[0]).and_then(|o| o.private.iter().find(|(k, _)| *k == g).map(|(_, v)| v.clone()));
    let Some(data) = found else {
        task.gfx_put_u32(cx, a[2], 0);
        return Reply::Value(DXGI_ERROR_NOT_FOUND);
    };
    let room = task.gfx_read(cx, a[2], 4).map_or(0, |b| u32_at(&b, 0)) as usize;
    task.gfx_put_u32(cx, a[2], data.len() as u32);
    if a[3] == 0 {
        return Reply::Value(S_OK);
    }
    if room < data.len() {
        return Reply::Value(DXGI_ERROR_MORE_DATA);
    }
    task.gfx_put(cx, a[3], &data);
    Reply::Value(S_OK)
}

/// `GetParent(riid, &parent)`: an adapter's factory, an output's or a DXGI device's adapter, a swap chain's factory,
/// a surface's device.
pub fn get_parent(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.gfx(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let base = match &g.kind {
        Kind::Adapter | Kind::SwapChain(_) => task.make_gfx(cx, g.dll, Kind::Factory, None),
        Kind::Output | Kind::Device => task.make_gfx(cx, g.dll, Kind::Adapter, None),
        Kind::Texture(_) => {
            let Some(dev) = &g.dev else { return Reply::Value(E_INVALIDARG) };
            let device = lock(dev).device;
            task.add_ref(device);
            Ok(device)
        }
        _ => return Reply::Value(E_NOINTERFACE),
    };
    match base {
        Ok(base) => task.gfx_give_as(cx, base, a[1], a[2]),
        Err(reply) => reply,
    }
}

/// `IDXGIDeviceSubObject::GetDevice(riid, &device)`.
pub fn sub_get_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (_, dev) = dev_or!(task, a[0], E_INVALIDARG);
    let device = lock(&dev).device;
    task.add_ref(device);
    task.gfx_give_as(cx, device, a[1], a[2])
}

/// `ID3D11DeviceChild::GetDevice(&device)`.
pub fn child_get_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (_, dev) = dev_or!(task, a[0], 0);
    let device = lock(&dev).device;
    task.add_ref(device);
    let _ = task.write_ptr(cx, a[1], device);
    Reply::Value(0)
}

/// A view's, state's or query's `GetDesc(&desc)`: the description it was made with.
pub fn object_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let desc = match task.gfx(a[0]).map(|g| g.kind) {
        Some(Kind::View { desc, .. } | Kind::State { desc, .. } | Kind::Query { desc, .. }) => desc,
        _ => return Reply::Value(0),
    };
    task.gfx_put(cx, a[1], &desc);
    Reply::Value(0)
}

// ---- DXGI: the factory ----

/// `CreateDXGIFactory(riid, &factory)` and `CreateDXGIFactory1`: the factory, if the machine has a GPU.
pub fn create_factory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    factory(task, cx, a[0], a[1])
}

/// `CreateDXGIFactory2(flags, riid, &factory)`.
pub fn create_factory2(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    factory(task, cx, a[1], a[2])
}

fn factory(task: &mut Task, cx: &mut Cx<'_>, iid: u64, out: u64) -> Reply {
    if task.k.options.gpu.is_none() {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(DXGI_ERROR_UNSUPPORTED);
    }
    match task.make_gfx(cx, "dxgi.dll", Kind::Factory, None) {
        Ok(base) => task.gfx_give_as(cx, base, iid, out),
        Err(reply) => reply,
    }
}

fn adapter(task: &mut Task, cx: &mut Cx<'_>, this: u64, index: u64, iid: Option<u64>, out: u64) -> Reply {
    let Some(g) = task.gfx(this) else { return Reply::Value(E_INVALIDARG) };
    if index != 0 || task.k.options.gpu.is_none() {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(DXGI_ERROR_NOT_FOUND);
    }
    match (task.make_gfx(cx, g.dll, Kind::Adapter, None), iid) {
        (Ok(base), Some(iid)) => task.gfx_give_as(cx, base, iid, out),
        (Ok(base), None) => task.gfx_give(cx, base, out),
        (Err(reply), _) => reply,
    }
}

/// `EnumAdapters(index, &adapter)` and `EnumAdapters1`: the one adapter.
pub fn factory_enum_adapters(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    adapter(task, cx, a[0], a[1] as u32 as u64, None, a[2])
}

/// `EnumWarpAdapter(riid, &adapter)`: the same adapter, which may be the CPU device anyway.
pub fn factory_enum_warp_adapter(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    adapter(task, cx, a[0], 0, Some(a[1]), a[2])
}

/// `EnumAdapterByGpuPreference(index, preference, riid, &adapter)`.
pub fn factory_enum_adapter_by_gpu_preference(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    adapter(task, cx, a[0], a[1] as u32 as u64, Some(a[3]), a[4])
}

/// `CheckFeatureSupport(feature, data, size)`: none of DXGI 1.5's features (tearing) are there.
pub fn factory_check_feature_support(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put(cx, a[2], &vec![0; (a[3] as u32 as usize).min(64)]);
    Reply::Value(S_OK)
}

fn read_desc(task: &mut Task, cx: &mut Cx<'_>, at: u64) -> Option<Vec<u8>> {
    let w = task.abi().width as usize;
    task.gfx_read(cx, at, if w == 8 { 72 } else { 60 })
}

/// `CreateSwapChain(device, &DXGI_SWAP_CHAIN_DESC, &swap_chain)`.
pub fn factory_create_swap_chain(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.gfx(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Some(desc) = read_desc(task, cx, a[2]) else { return Reply::Value(E_INVALIDARG) };
    swap_chain_from_desc(task, cx, g.dll, a[1], &desc, a[3])
}

fn swap_chain_from_desc(task: &mut Task, cx: &mut Cx<'_>, dll: &'static str, device: u64, desc: &[u8], out: u64) -> Reply {
    let w = task.abi().width as usize;
    let hwnd_at = if w == 8 { 48 } else { 44 };
    let hwnd = task.abi().ptr_at(desc, hwnd_at);
    let after = hwnd_at + w;
    let s = SwapDesc {
        width: u32_at(desc, 0),
        height: u32_at(desc, 4),
        format: u32_at(desc, 16),
        usage: u32_at(desc, 36),
        count: u32_at(desc, 40),
        effect: u32_at(desc, after + 4),
        flags: u32_at(desc, after + 8),
    };
    swap_chain(task, cx, dll, device, hwnd, s, out)
}

/// `CreateSwapChainForHwnd(device, hwnd, &DXGI_SWAP_CHAIN_DESC1, &fullscreen, output, &swap_chain)`.
pub fn factory_create_swap_chain_for_hwnd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.gfx(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Some(d) = task.gfx_read(cx, a[3], 48) else { return Reply::Value(E_INVALIDARG) };
    let s = SwapDesc { width: u32_at(&d, 0), height: u32_at(&d, 4), format: u32_at(&d, 8), usage: u32_at(&d, 24), count: u32_at(&d, 28), effect: u32_at(&d, 36), flags: u32_at(&d, 44) };
    swap_chain(task, cx, g.dll, a[1], a[2], s, a[6])
}

struct SwapDesc {
    width: u32,
    height: u32,
    format: u32,
    usage: u32,
    count: u32,
    effect: u32,
    flags: u32,
}

/// A window's client area's size.
fn client_size(task: &Task, hwnd: u64) -> (u32, u32) {
    task.win32k().windows.get(&hwnd).map_or((1, 1), |w| {
        let c = w.client();
        (c.width().max(1) as u32, c.height().max(1) as u32)
    })
}

/// A back buffer: a texture of the swap chain's size and format, drawn into and read by `Present`.
fn back_buffer(task: &mut Task, cx: &mut Cx<'_>, dev: &Shared, width: u32, height: u32, format: u32) -> Result<(u64, Id), u64> {
    let desc = d3d::Texture2dDesc {
        width,
        height,
        mip_levels: 1,
        array_size: 1,
        format,
        sample_count: 1,
        usage: d3d::usage::DEFAULT,
        bind: d3d::bind::RENDER_TARGET | d3d::bind::SHADER_RESOURCE,
        cpu_access: 0,
        misc: 0,
    };
    let id = lock(dev).d.create_texture2d(desc, &[]).map_err(hr)?;
    match task.make_gfx(cx, "d3d11.dll", Kind::Texture(id), Some(dev.clone())) {
        Ok(base) if base != 0 => Ok((base, id)),
        _ => Err(E_OUTOFMEMORY),
    }
}

fn swap_chain(task: &mut Task, cx: &mut Cx<'_>, dll: &'static str, device: u64, hwnd: u64, s: SwapDesc, out: u64) -> Reply {
    let Some((_, dev)) = dev_of(task, device) else { return Reply::Value(E_INVALIDARG) };
    if hwnd == 0 || out == 0 {
        return Reply::Value(E_INVALIDARG);
    }
    let (cw, ch) = client_size(task, hwnd);
    let (width, height) = (if s.width == 0 { cw } else { s.width }, if s.height == 0 { ch } else { s.height });
    let format = if s.format == 0 { d3d::format::B8G8R8A8_UNORM } else { s.format };
    let (back, texture) = match back_buffer(task, cx, &dev, width, height, format) {
        Ok(b) => b,
        Err(e) => return Reply::Value(e),
    };
    let swap = Swap { hwnd, width, height, format, count: s.count, usage: s.usage, effect: s.effect, flags: s.flags, back, texture, presents: 0, last: None };
    match task.make_gfx(cx, dll, Kind::SwapChain(Box::new(swap)), Some(dev)) {
        Ok(base) => task.gfx_give(cx, base, out),
        Err(reply) => reply,
    }
}

// ---- DXGI: the adapter and its output ----

/// DXGI_ADAPTER_DESC and its later versions, as far as `version` has fields: the name, IDs, memory and LUID, then
/// flags, then preemption granularities.
fn adapter_desc(task: &mut Task, version: u32) -> Vec<u8> {
    let w = task.abi().width as usize;
    let info = task.k.options.gpu.as_ref().map(|g| g.info().clone());
    let name = info.as_ref().map_or_else(|| "Carbon GPU".to_string(), |i| i.name.clone());
    let software = info.as_ref().is_some_and(|i| i.kind == gpu_api::Kind::Software);
    let mut b: Vec<u8> = name.encode_utf16().take(127).flat_map(u16::to_le_bytes).collect();
    b.resize(256, 0);
    for v in [gpu_api::VENDOR_ID, gpu_api::DEVICE_ID, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    let size = |v: u64| if w == 8 { v.to_le_bytes().to_vec() } else { (v.min(u32::MAX as u64) as u32).to_le_bytes().to_vec() };
    b.extend(size(1 << 30));
    b.extend(size(0));
    b.extend(size(4 << 30));
    b.extend(gpu_api::VENDOR_ID.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    if version >= 1 {
        b.extend(u32::from(software).wrapping_mul(2).to_le_bytes());
    }
    if version >= 2 {
        b.extend([0u8; 8]);
    }
    while !b.len().is_multiple_of(w) {
        b.push(0);
    }
    b
}

pub fn adapter_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut d = adapter_desc(task, 0);
    let w = task.abi().width as usize;
    d.truncate(if w == 8 { 304 } else { 292 });
    Reply::Value(if task.gfx_put(cx, a[1], &d) { S_OK } else { E_INVALIDARG })
}

pub fn adapter_get_desc1(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let d = adapter_desc(task, 1);
    Reply::Value(if task.gfx_put(cx, a[1], &d) { S_OK } else { E_INVALIDARG })
}

pub fn adapter_get_desc2(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let d = adapter_desc(task, 2);
    Reply::Value(if task.gfx_put(cx, a[1], &d) { S_OK } else { E_INVALIDARG })
}

pub fn adapter_get_desc3(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    adapter_get_desc2(task, cx, a)
}

/// `CheckInterfaceSupport(guid, &version)`: yes, with a driver version.
pub fn adapter_check_interface_support(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let version: u64 = (31 << 48) | (101 << 16);
    task.gfx_put(cx, a[2], &version.to_le_bytes());
    Reply::Value(S_OK)
}

/// `QueryVideoMemoryInfo(node, group, &info)`: a budget of 1 GiB, none used.
pub fn adapter_query_video_memory_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let mut b = (1u64 << 30).to_le_bytes().to_vec();
    b.extend([0u8; 24]);
    task.gfx_put(cx, a[3], &b);
    Reply::Value(S_OK)
}

/// `EnumOutputs(index, &output)`: the desktop's screen.
pub fn adapter_enum_outputs(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.gfx(a[0]) else { return Reply::Value(E_INVALIDARG) };
    if a[1] as u32 != 0 {
        let _ = task.write_ptr(cx, a[2], 0);
        return Reply::Value(DXGI_ERROR_NOT_FOUND);
    }
    match task.make_gfx(cx, g.dll, Kind::Output, None) {
        Ok(base) => task.gfx_give(cx, base, a[2]),
        Err(reply) => reply,
    }
}

fn screen(task: &Task) -> (i32, i32) {
    task.desktop(|d| (d.output().width, d.output().height))
}

/// A DXGI_MODE_DESC of the screen: its size, 60 Hz, and the format.
fn mode(task: &Task, format: u32) -> Vec<u8> {
    let (w, h) = screen(task);
    [w as u32, h as u32, 60, 1, format, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn output_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (w, h) = screen(task);
    let mut b: Vec<u8> = "\\\\.\\DISPLAY1".encode_utf16().flat_map(u16::to_le_bytes).collect();
    b.resize(64, 0);
    for v in [0, 0, w, h] {
        b.extend(v.to_le_bytes());
    }
    b.extend(1u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    let abi = task.abi();
    b.extend(abi.ptr(0x0001_0001));
    Reply::Value(if task.gfx_put(cx, a[1], &b) { S_OK } else { E_INVALIDARG })
}

/// `GetDisplayModeList(format, flags, &count, modes)`: the screen's one mode.
pub fn output_get_display_mode_list(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[4] != 0 {
        let m = mode(task, a[1] as u32);
        task.gfx_put(cx, a[4], &m);
    }
    task.gfx_put_u32(cx, a[3], 1);
    Reply::Value(S_OK)
}

/// `FindClosestMatchingMode(&mode, &closest, device)`: the screen's, in the format asked for.
pub fn output_find_closest_matching_mode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let format = task.gfx_read(cx, a[1], 28).map_or(0, |b| u32_at(&b, 16));
    let m = mode(task, if format == 0 { d3d::format::B8G8R8A8_UNORM } else { format });
    task.gfx_put(cx, a[2], &m);
    Reply::Value(S_OK)
}

pub fn output_wait_for_vblank(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    std::thread::sleep(Duration::from_micros(16_667));
    Reply::Value(S_OK)
}

/// IDXGIDevice2's `ReclaimResources(count, resources, &discarded)`: nothing offered is ever discarded.
pub fn dxgi_device_reclaim_resources(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[3] != 0 {
        task.gfx_put(cx, a[3], &vec![0; 4 * (a[1] as u32).min(4096) as usize]);
    }
    Reply::Value(S_OK)
}

/// IDXGIDevice2's `EnqueueSetEvent(event)`: the event set now, as the device's work is done when it's submitted.
pub fn dxgi_device_enqueue_set_event(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    super::synch::set_event(task, cx, &[a[1]]);
    Reply::Value(S_OK)
}

/// IDXGIDevice's `GetAdapter(&adapter)`.
pub fn dxgi_device_get_adapter(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    adapter(task, cx, a[0], 0, None, a[1])
}

// ---- DXGI: swap chains ----

fn swap_of(task: &Task, this: u64) -> Option<(Swap, Shared)> {
    let g = task.gfx(this)?;
    let Kind::SwapChain(s) = g.kind else { return None };
    Some((*s, g.dev?))
}

/// Pixels as the desktop has them (0x00RRGGBB), from a back buffer's texels.
fn to_rgb(format: u32, bytes: &[u8]) -> Option<Vec<display::Rgb>> {
    use d3d::format::*;
    let texels = bytes.as_chunks::<4>().0;
    Some(match format {
        B8G8R8A8_UNORM | B8G8R8A8_UNORM_SRGB | B8G8R8X8_UNORM | B8G8R8A8_TYPELESS => texels.iter().map(|t| u32::from_le_bytes([t[0], t[1], t[2], 0])).collect(),
        R8G8B8A8_UNORM | R8G8B8A8_UNORM_SRGB | R8G8B8A8_TYPELESS => texels.iter().map(|t| u32::from_le_bytes([t[2], t[1], t[0], 0])).collect(),
        R10G10B10A2_UNORM => texels
            .iter()
            .map(|t| {
                let w = u32::from_le_bytes(*t);
                let c = |s: u32| ((w >> s) & 0x3ff) >> 2;
                (c(0) << 16) | (c(10) << 8) | c(20)
            })
            .collect(),
        _ => return None,
    })
}

/// `Present(sync_interval, flags)` (and `Present1`): the back buffer into the window, after a 60 Hz frame since the
/// last if it's asked to wait for one.
pub fn swap_present(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    const TEST: u64 = 1;
    let Some((s, dev)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    if a[2] & TEST != 0 {
        return Reply::Value(S_OK);
    }
    let read = lock(&dev).d.read_texture(s.texture, 0);
    let (w, h, bytes) = match read {
        Ok(r) => r,
        Err(e) => return Reply::Value(hr(e)),
    };
    match to_rgb(s.format, &bytes) {
        Some(pixels) => {
            task.win32k().present(s.hwnd, w as i32, h as i32, &pixels);
        }
        None => task.trace_gfx(&format!("presenting DXGI format {} isn't written yet", s.format)),
    }
    if a[1] as u32 > 0
        && let Some(last) = s.last {
            let frame = Duration::from_micros(16_667) * a[1] as u32;
            if let Some(left) = frame.checked_sub(last.elapsed()) {
                std::thread::sleep(left);
            }
        }
    task.gfx_mut(a[0], |g| {
        if let Kind::SwapChain(s) = &mut g.kind {
            s.presents += 1;
            s.last = Some(Instant::now());
        }
    });
    Reply::Value(S_OK)
}

/// `GetBuffer(index, riid, &surface)`: the back buffer, whichever is asked for.
pub fn swap_get_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Some(iid) = task.read_guid(cx, a[2]) else { return Reply::Value(E_POINTER) };
    match task.query_interface(cx, s.back, &iid, a[3]) {
        Ok(hr) => Reply::Value(hr),
        Err(reply) => reply,
    }
}

/// `SetFullscreenState(fullscreen, output)`: windowed is all there is.
pub fn swap_set_fullscreen_state(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(if a[1] as u32 != 0 { DXGI_ERROR_NOT_CURRENTLY_AVAILABLE } else { S_OK })
}

pub fn swap_get_fullscreen_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[1], 0);
    if a[2] != 0 {
        let _ = task.write_ptr(cx, a[2], 0);
    }
    Reply::Value(S_OK)
}

pub fn swap_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let abi = task.abi();
    let mut b: Vec<u8> = [s.width, s.height, 60, 1, s.format, 0, 0, 1, 0, s.usage, s.count].iter().flat_map(|v| v.to_le_bytes()).collect();
    while !b.len().is_multiple_of(abi.width as usize) {
        b.push(0);
    }
    b.extend(abi.ptr(s.hwnd));
    for v in [1, s.effect, s.flags] {
        b.extend(v.to_le_bytes());
    }
    while !b.len().is_multiple_of(abi.width as usize) {
        b.push(0);
    }
    Reply::Value(if task.gfx_put(cx, a[1], &b) { S_OK } else { E_INVALIDARG })
}

pub fn swap_get_desc1(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let b: Vec<u8> = [s.width, s.height, s.format, 0, 1, 0, s.usage, s.count, 0, s.effect, 0, s.flags].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(if task.gfx_put(cx, a[1], &b) { S_OK } else { E_INVALIDARG })
}

/// DXGI_SWAP_CHAIN_FULLSCREEN_DESC: 60 Hz, windowed.
pub fn swap_get_fullscreen_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let b: Vec<u8> = [60u32, 1, 0, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    Reply::Value(if task.gfx_put(cx, a[1], &b) { S_OK } else { E_INVALIDARG })
}

pub fn swap_get_hwnd(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let _ = task.write_ptr(cx, a[1], s.hwnd);
    Reply::Value(S_OK)
}

pub fn swap_get_rotation(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[1], 1);
    Reply::Value(S_OK)
}

pub fn swap_get_source_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.gfx_put_u32(cx, a[1], s.width);
    task.gfx_put_u32(cx, a[2], s.height);
    Reply::Value(S_OK)
}

pub fn swap_get_last_present_count(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, _)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.gfx_put_u32(cx, a[1], s.presents);
    Reply::Value(S_OK)
}

/// `CheckColorSpaceSupport(space, &support)`: sRGB (space 0) can be presented.
pub fn swap_check_color_space_support(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[2], if a[1] as u32 == 0 { 1 } else { 0 });
    Reply::Value(S_OK)
}

pub fn swap_get_containing_output(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(g) = task.gfx(a[0]) else { return Reply::Value(E_INVALIDARG) };
    match task.make_gfx(cx, g.dll, Kind::Output, None) {
        Ok(base) => task.gfx_give(cx, base, a[1]),
        Err(reply) => reply,
    }
}

/// `ResizeBuffers(count, width, height, format, flags)`: a new back buffer (0 for the window's size, and the format
/// kept).
pub fn swap_resize_buffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((s, dev)) = swap_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let (cw, ch) = client_size(task, s.hwnd);
    let width = if a[2] as u32 == 0 { cw } else { a[2] as u32 };
    let height = if a[3] as u32 == 0 { ch } else { a[3] as u32 };
    let format = if a[4] as u32 == 0 { s.format } else { a[4] as u32 };
    let (back, texture) = match back_buffer(task, cx, &dev, width, height, format) {
        Ok(b) => b,
        Err(e) => return Reply::Value(e),
    };
    task.gfx_mut(a[0], |g| {
        if let Kind::SwapChain(sw) = &mut g.kind {
            sw.width = width;
            sw.height = height;
            sw.format = format;
            if a[1] as u32 != 0 {
                sw.count = a[1] as u32;
            }
            sw.back = back;
            sw.texture = texture;
        }
    });
    if let Err(reply) = task.release(cx, s.back) {
        return reply;
    }
    Reply::Value(S_OK)
}

// ---- Direct3D 11: making a device ----

/// `D3D11CreateDevice(adapter, driver, software, flags, levels, count, sdk, &device, &level, &context)`.
pub fn d3d11_create_device(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_device(task, cx, a[4], a[5], a[7], a[8], a[9], None)
}

/// `D3D11CreateDeviceAndSwapChain(..., sdk, &swap_desc, &swap_chain, &device, &level, &context)`.
pub fn d3d11_create_device_and_swap_chain(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    create_device(task, cx, a[4], a[5], a[9], a[10], a[11], Some((a[7], a[8])))
}

#[allow(clippy::too_many_arguments)]
fn create_device(task: &mut Task, cx: &mut Cx<'_>, levels: u64, count: u64, out_device: u64, out_level: u64, out_context: u64, swap: Option<(u64, u64)>) -> Reply {
    for out in [out_device, out_context] {
        if out != 0 {
            let _ = task.write_ptr(cx, out, 0);
        }
    }
    let Some(gpu) = task.k.options.gpu.clone() else { return Reply::Value(DXGI_ERROR_UNSUPPORTED) };
    // Feature levels 10_0 to 11_0: the first asked for that's one of them.
    let asked: Vec<u32> = if levels == 0 || count == 0 {
        vec![0xb000, 0xa100, 0xa000]
    } else {
        let n = (count as u32).min(16) as usize;
        task.gfx_read(cx, levels, 4 * n).map(|b| (0..n).map(|i| u32_at(&b, 4 * i)).collect()).unwrap_or_default()
    };
    let Some(level) = asked.into_iter().find(|l| (0xa000..=0xb000).contains(l)) else { return Reply::Value(E_INVALIDARG) };
    if out_level != 0 {
        task.gfx_put_u32(cx, out_level, level);
    }
    if out_device == 0 && out_context == 0 && swap.is_none() {
        return Reply::Value(S_FALSE);
    }
    let dev: Shared = Arc::new(Mutex::new(Dev { d: d3d::Device::new(gpu), level, device: 0, context: 0, maps: HashMap::new() }));
    let device = match task.make_gfx(cx, "d3d11.dll", Kind::Device, Some(dev.clone())) {
        Ok(0) => return Reply::Value(E_OUTOFMEMORY),
        Ok(b) => b,
        Err(reply) => return reply,
    };
    let context = match task.make_gfx(cx, "d3d11.dll", Kind::Context, Some(dev.clone())) {
        Ok(b) => b,
        Err(reply) => return reply,
    };
    {
        let mut d = lock(&dev);
        d.device = device;
        d.context = context;
    }
    if let Some((desc, out)) = swap
        && let Some(desc) = read_desc(task, cx, desc)
            && let Reply::Value(hr) = swap_chain_from_desc(task, cx, "d3d11.dll", device, &desc, out)
                && hr != S_OK {
                    return Reply::Value(hr);
                }
    if out_context != 0 {
        task.add_ref(context);
        let _ = task.write_ptr(cx, out_context, context);
    }
    if out_device != 0 {
        let _ = task.write_ptr(cx, out_device, device);
    }
    Reply::Value(S_OK)
}

// ---- Direct3D 11: the device's methods ----

/// Makes a child of the device's, from what `make` does with the device, and gives it to `out`.
fn child(task: &mut Task, cx: &mut Cx<'_>, this: u64, out: u64, make: impl FnOnce(&mut d3d::Device) -> Result<Kind, d3d::Error>) -> Reply {
    let (g, dev) = dev_or!(task, this, E_INVALIDARG);
    let kind = match make(&mut lock(&dev).d) {
        Ok(k) => k,
        Err(e) => {
            task.trace_gfx(&e.message);
            if out != 0 {
                let _ = task.write_ptr(cx, out, 0);
            }
            return Reply::Value(hr(e));
        }
    };
    if out == 0 {
        // Only checked: the object goes again.
        if let Some(id) = match kind {
            Kind::Buffer(id) | Kind::Texture(id) | Kind::Layout(id) => Some(id),
            Kind::View { id, .. } | Kind::State { id, .. } | Kind::Shader { id, .. } => Some(id),
            _ => None,
        } {
            lock(&dev).d.release(id);
        }
        return Reply::Value(S_FALSE);
    }
    match task.make_gfx(cx, g.dll, kind, Some(dev)) {
        Ok(base) => task.gfx_give(cx, base, out),
        Err(reply) => reply,
    }
}

/// `CreateBuffer(&desc, &initial, &buffer)`.
pub fn device_create_buffer(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 24) else { return Reply::Value(E_INVALIDARG) };
    let desc = d3d::BufferDesc { size: u32_at(&b, 0), usage: u32_at(&b, 4), bind: u32_at(&b, 8), cpu_access: u32_at(&b, 12), misc: u32_at(&b, 16), stride: u32_at(&b, 20) };
    let initial = if a[2] != 0 {
        let at = task.gfx_ptrs(cx, a[2], 1)[0];
        task.gfx_read(cx, at, desc.size as usize)
    } else {
        None
    };
    child(task, cx, a[0], a[3], |d| d.create_buffer(desc, initial.as_deref()).map(Kind::Buffer))
}

fn texture_desc(b: &[u8]) -> d3d::Texture2dDesc {
    d3d::Texture2dDesc {
        width: u32_at(b, 0),
        height: u32_at(b, 4),
        mip_levels: u32_at(b, 8),
        array_size: u32_at(b, 12),
        format: u32_at(b, 16),
        sample_count: u32_at(b, 20),
        usage: u32_at(b, 28),
        bind: u32_at(b, 32),
        cpu_access: u32_at(b, 36),
        misc: u32_at(b, 40),
    }
}

/// `CreateTexture2D(&desc, initial[], &texture)`: each subresource's data, rows its pitch apart.
pub fn device_create_texture2d(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 44) else { return Reply::Value(E_INVALIDARG) };
    let desc = texture_desc(&b);
    let format = match d3d::texture_format(desc.format, desc.bind) {
        Ok(f) => f,
        Err(e) => {
            task.trace_gfx(&e.message);
            return Reply::Value(hr(e));
        }
    };
    let mut subresources: Vec<(Vec<u8>, u32)> = Vec::new();
    if a[2] != 0 && desc.width > 0 && desc.height > 0 {
        let most = 32 - desc.width.max(desc.height).leading_zeros();
        let levels = if desc.mip_levels == 0 { most } else { desc.mip_levels.min(most) };
        let w = task.abi().width as usize;
        let size = if w == 8 { 16 } else { 12 };
        for i in 0..levels * desc.array_size.max(1) {
            let level = i % levels;
            let (lw, lh) = ((desc.width >> level).max(1), (desc.height >> level).max(1));
            let Some(entry) = task.gfx_read(cx, a[2] + (i as usize * size) as u64, size) else { break };
            let at = task.abi().ptr_at(&entry, 0);
            let row = lw * format.bytes();
            let pitch = match u32_at(&entry, w) {
                0 => row,
                p => p,
            };
            let len = pitch as usize * (lh as usize - 1) + row as usize;
            subresources.push((task.gfx_read(cx, at, len).unwrap_or_default(), pitch));
        }
    }
    child(task, cx, a[0], a[3], |d| {
        let data: Vec<d3d::Data<'_>> = subresources.iter().map(|(b, p)| d3d::Data { bytes: b, row_pitch: *p }).collect();
        d.create_texture2d(desc, &data).map(Kind::Texture)
    })
}

/// A view's resource, as its object's base (with a reference for the view) and its id.
fn resource_of(task: &mut Task, ptr: u64) -> Option<(u64, Id)> {
    let id = task.gfx_id(ptr)?;
    let base = task.object_base(ptr)?;
    task.add_ref(base);
    Some((base, id))
}

/// A view: `make` with the resource's id and the description (none for the resource's own).
fn view(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], iid: Guid, len: usize, make: impl FnOnce(&mut d3d::Device, Id, Option<&[u8]>) -> Result<Id, d3d::Error>) -> Reply {
    let Some((resource, id)) = resource_of(task, a[1]) else { return Reply::Value(E_INVALIDARG) };
    let desc = task.gfx_read(cx, a[2], len);
    let given = desc.clone().unwrap_or_else(|| vec![0; len]);
    let made = child(task, cx, a[0], a[3], |d| make(d, id, desc.as_deref()).map(|v| Kind::View { id: v, resource, iid, desc: given }));
    if !matches!(made, Reply::Value(S_OK))
        && let Err(reply) = task.release(cx, resource) {
            return reply;
        }
    made
}

/// `CreateShaderResourceView(resource, &desc, &view)`.
pub fn device_create_srv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    view(task, cx, a, IID_D3D11_SRV, 24, |d, id, b| {
        let desc = b.map(|b| {
            let dimension = u32_at(b, 4);
            let mut s = d3d::SrvDesc { format: u32_at(b, 0), dimension, ..d3d::SrvDesc::default() };
            match dimension {
                1 => (s.first_element, s.elements) = (u32_at(b, 8), u32_at(b, 12)),
                11 => (s.first_element, s.elements, s.flags) = (u32_at(b, 8), u32_at(b, 12), u32_at(b, 16)),
                4 => (s.most_detailed_mip, s.mip_levels) = (u32_at(b, 8), u32_at(b, 12)),
                5 => (s.most_detailed_mip, s.mip_levels, s.first_slice, s.slices) = (u32_at(b, 8), u32_at(b, 12), u32_at(b, 16), u32_at(b, 20)),
                _ => {}
            }
            s
        });
        d.create_srv(id, desc)
    })
}

/// `CreateRenderTargetView(resource, &desc, &view)`.
pub fn device_create_rtv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    view(task, cx, a, IID_D3D11_RTV, 20, |d, id, b| {
        let desc = b.map(|b| d3d::TargetDesc { format: u32_at(b, 0), dimension: u32_at(b, 4), mip_slice: u32_at(b, 8), first_slice: if u32_at(b, 4) == 5 { u32_at(b, 12) } else { 0 } });
        d.create_rtv(id, desc)
    })
}

/// `CreateDepthStencilView(resource, &desc, &view)`.
pub fn device_create_dsv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    view(task, cx, a, IID_D3D11_DSV, 24, |d, id, b| {
        let desc = b.map(|b| d3d::TargetDesc { format: u32_at(b, 0), dimension: u32_at(b, 4), mip_slice: u32_at(b, 12), first_slice: if u32_at(b, 4) == 4 { u32_at(b, 16) } else { 0 } });
        d.create_dsv(id, desc)
    })
}

/// `CreateUnorderedAccessView(resource, &desc, &view)`.
pub fn device_create_uav(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    view(task, cx, a, IID_D3D11_UAV, 20, |d, id, b| {
        let desc = b.map(|b| d3d::UavDesc { format: u32_at(b, 0), dimension: u32_at(b, 4), first_element: u32_at(b, 8), elements: u32_at(b, 12), flags: u32_at(b, 16) });
        d.create_uav(id, desc)
    })
}

/// `CreateInputLayout(elements, count, bytecode, length, &layout)`.
pub fn device_create_input_layout(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.abi().width as usize;
    let size = w + 24;
    let n = (a[2] as u32).min(32) as usize;
    let Some(table) = task.gfx_read(cx, a[1], n * size) else { return Reply::Value(E_INVALIDARG) };
    let mut elements = Vec::new();
    for i in 0..n {
        let e = &table[i * size..(i + 1) * size];
        let name_at = task.abi().ptr_at(e, 0);
        let semantic = task.text_arg(cx, name_at, false);
        elements.push(d3d::InputElement {
            semantic,
            index: u32_at(e, w),
            format: u32_at(e, w + 4),
            slot: u32_at(e, w + 8),
            offset: u32_at(e, w + 12),
            per_instance: u32_at(e, w + 16) == 1,
            step_rate: u32_at(e, w + 20),
        });
    }
    let Some(code) = task.gfx_read(cx, a[3], (a[4] as usize).min(1 << 24)) else { return Reply::Value(E_INVALIDARG) };
    child(task, cx, a[0], a[5], |d| d.create_input_layout(&elements, &code).map(Kind::Layout))
}

fn shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], stage: d3d::Stage, iid: Guid) -> Reply {
    let Some(code) = task.gfx_read(cx, a[1], (a[2] as usize).min(1 << 24)) else { return Reply::Value(E_INVALIDARG) };
    child(task, cx, a[0], a[4], |d| d.create_shader(stage, &code).map(|id| Kind::Shader { id, iid }))
}

/// `CreateVertexShader(bytecode, length, linkage, &shader)`, and the pixel and compute ones.
pub fn device_create_vertex_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    shader(task, cx, a, d3d::Stage::Vertex, IID_D3D11_VERTEX_SHADER)
}

pub fn device_create_pixel_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    shader(task, cx, a, d3d::Stage::Pixel, IID_D3D11_PIXEL_SHADER)
}

pub fn device_create_compute_shader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    shader(task, cx, a, d3d::Stage::Compute, IID_D3D11_COMPUTE_SHADER)
}

/// `CreateBlendState(&desc, &state)`: D3D11_BLEND_DESC, eight targets of 32 bytes after two BOOLs.
pub fn device_create_blend_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 264) else { return Reply::Value(E_INVALIDARG) };
    let mut desc = d3d::BlendDesc { alpha_to_coverage: u32_at(&b, 0) != 0, independent: u32_at(&b, 4) != 0, ..d3d::BlendDesc::default() };
    for (i, t) in desc.targets.iter_mut().enumerate() {
        let at = 8 + 32 * i;
        *t = d3d::TargetBlend {
            enable: u32_at(&b, at) != 0,
            src: u32_at(&b, at + 4),
            dest: u32_at(&b, at + 8),
            op: u32_at(&b, at + 12),
            src_alpha: u32_at(&b, at + 16),
            dest_alpha: u32_at(&b, at + 20),
            op_alpha: u32_at(&b, at + 24),
            write_mask: b[at + 28],
        };
    }
    child(task, cx, a[0], a[2], |d| d.create_blend_state(desc).map(|id| Kind::State { id, iid: IID_D3D11_BLEND_STATE, desc: b }))
}

/// `CreateDepthStencilState(&desc, &state)`.
pub fn device_create_depth_stencil_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 52) else { return Reply::Value(E_INVALIDARG) };
    let desc = d3d::DepthStencilDesc { depth_enable: u32_at(&b, 0) != 0, depth_write: u32_at(&b, 4), depth_func: u32_at(&b, 8), stencil_enable: u32_at(&b, 12) != 0 };
    if desc.stencil_enable {
        task.trace_gfx("a depth-stencil state's stencil test is ignored");
    }
    child(task, cx, a[0], a[2], |d| d.create_depth_stencil_state(desc).map(|id| Kind::State { id, iid: IID_D3D11_DEPTH_STENCIL_STATE, desc: b }))
}

/// `CreateRasterizerState(&desc, &state)`.
pub fn device_create_rasterizer_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 40) else { return Reply::Value(E_INVALIDARG) };
    let desc = d3d::RasterizerDesc { fill: u32_at(&b, 0), cull: u32_at(&b, 4), front_counter_clockwise: u32_at(&b, 8) != 0, depth_clip: u32_at(&b, 24) != 0, scissor_enable: u32_at(&b, 28) != 0 };
    child(task, cx, a[0], a[2], |d| d.create_rasterizer_state(desc).map(|id| Kind::State { id, iid: IID_D3D11_RASTERIZER_STATE, desc: b }))
}

/// `CreateSamplerState(&desc, &state)`.
pub fn device_create_sampler_state(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 52) else { return Reply::Value(E_INVALIDARG) };
    let desc = d3d::SamplerDesc {
        filter: u32_at(&b, 0),
        address: [u32_at(&b, 4), u32_at(&b, 8), u32_at(&b, 12)],
        mip_lod_bias: f32_at(&b, 16),
        max_anisotropy: u32_at(&b, 20),
        comparison: u32_at(&b, 24),
        border: [f32_at(&b, 28), f32_at(&b, 32), f32_at(&b, 36), f32_at(&b, 40)],
        min_lod: f32_at(&b, 44),
        max_lod: f32_at(&b, 48),
    };
    child(task, cx, a[0], a[2], |d| d.create_sampler(desc).map(|id| Kind::State { id, iid: IID_D3D11_SAMPLER_STATE, desc: b }))
}

/// `CreateQuery(&desc, &query)`: done as soon as it's ended, since the context waits for its work.
pub fn device_create_query(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(b) = task.gfx_read(cx, a[1], 8) else { return Reply::Value(E_INVALIDARG) };
    let kind = u32_at(&b, 0);
    child(task, cx, a[0], a[2], |_| Ok(Kind::Query { kind, desc: b }))
}

/// D3D11_FORMAT_SUPPORT for a format.
fn format_support(format: u32) -> u32 {
    use d3d::format::*;
    const BUFFER: u32 = 0x1;
    const VERTEX: u32 = 0x2;
    const INDEX: u32 = 0x4;
    const TEXTURE2D: u32 = 0x20;
    const TEXTURE3D: u32 = 0x40;
    const TEXTURECUBE: u32 = 0x80;
    const SHADER_LOAD: u32 = 0x100;
    const SAMPLE: u32 = 0x200;
    const SAMPLE_COMPARISON: u32 = 0x400;
    const MIP: u32 = 0x1000;
    const MIP_AUTOGEN: u32 = 0x2000;
    const RENDER_TARGET: u32 = 0x4000;
    const BLENDABLE: u32 = 0x8000;
    const DEPTH_STENCIL: u32 = 0x10000;
    const CPU_LOCKABLE: u32 = 0x20000;
    const MULTISAMPLE_RESOLVE: u32 = 0x40000;
    const DISPLAY: u32 = 0x80000;
    const CAST_WITHIN_BIT_LAYOUT: u32 = 0x100000;
    const MULTISAMPLE_RENDERTARGET: u32 = 0x200000;
    const MULTISAMPLE_LOAD: u32 = 0x400000;
    const SHADER_GATHER: u32 = 0x800000;
    const BACK_BUFFER_CAST: u32 = 0x1000000;
    let mut s = 0;
    if let Ok(f) = d3d::texture_format(format, 0) {
        s |= TEXTURE2D | TEXTURECUBE | SHADER_LOAD | MIP | CPU_LOCKABLE | CAST_WITHIN_BIT_LAYOUT;
        if f.is_depth() {
            s |= DEPTH_STENCIL | SAMPLE | SAMPLE_COMPARISON | MULTISAMPLE_RENDERTARGET;
        } else {
            s |= TEXTURE3D | SAMPLE | SHADER_GATHER;
            if f.is_renderable() {
                s |= RENDER_TARGET | BLENDABLE | MIP_AUTOGEN | MULTISAMPLE_RENDERTARGET | MULTISAMPLE_RESOLVE | MULTISAMPLE_LOAD;
            }
        }
    }
    if matches!(format, D24_UNORM_S8_UINT | D16_UNORM | D32_FLOAT) {
        s |= TEXTURE2D | TEXTURECUBE | SHADER_LOAD | SAMPLE | SAMPLE_COMPARISON | MIP | DEPTH_STENCIL | CAST_WITHIN_BIT_LAYOUT | MULTISAMPLE_RENDERTARGET;
    }
    if matches!(format, R8G8B8A8_UNORM | B8G8R8A8_UNORM | R8G8B8A8_UNORM_SRGB | B8G8R8A8_UNORM_SRGB | R10G10B10A2_UNORM) {
        s |= DISPLAY | BACK_BUFFER_CAST;
    }
    if matches!(format, R32G32B32A32_FLOAT | R32G32B32_FLOAT | R32G32_FLOAT | R32_FLOAT | R32G32B32A32_UINT | R32G32B32_UINT | R32G32_UINT | R32_UINT | R32G32B32A32_SINT | R32G32B32_SINT | R32G32_SINT | R32_SINT | R8G8B8A8_UNORM) {
        s |= BUFFER | VERTEX;
    }
    if matches!(format, R16_UINT | R32_UINT) {
        s |= BUFFER | INDEX;
    }
    s
}

/// `CheckFormatSupport(format, &support)`.
pub fn device_check_format_support(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let s = format_support(a[1] as u32);
    task.gfx_put_u32(cx, a[2], s);
    Reply::Value(if s == 0 { E_FAIL } else { S_OK })
}

/// `CheckMultisampleQualityLevels(format, count, &levels)`: one sample only.
pub fn device_check_multisample_quality_levels(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put_u32(cx, a[3], u32::from(a[2] as u32 == 1));
    Reply::Value(S_OK)
}

pub fn device_check_counter_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.gfx_put(cx, a[1], &[0; 12]);
    Reply::Value(0)
}

/// `CheckFeatureSupport(feature, data, size)`: threading (free-threaded creation, no command lists), no doubles, a
/// format's support, full non-power-of-two textures and instancing for 9_x, and nothing else.
pub fn device_check_feature_support(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (feature, at, size) = (a[1] as u32, a[2], (a[3] as u32 as usize).min(256));
    if at == 0 {
        return Reply::Value(E_INVALIDARG);
    }
    let mut b = vec![0u8; size];
    match feature {
        0 if size >= 8 => b[..4].copy_from_slice(&1u32.to_le_bytes()),
        2 if size >= 8 => {
            let format = task.gfx_read(cx, at, 4).map_or(0, |f| u32_at(&f, 0));
            b[..4].copy_from_slice(&format.to_le_bytes());
            b[4..8].copy_from_slice(&format_support(format).to_le_bytes());
        }
        3 if size >= 8 => {
            let format = task.gfx_read(cx, at, 4).map_or(0, |f| u32_at(&f, 0));
            b[..4].copy_from_slice(&format.to_le_bytes());
        }
        7 | 11 if size >= 4 => b[..4].copy_from_slice(&1u32.to_le_bytes()),
        _ => {}
    }
    task.gfx_put(cx, at, &b);
    Reply::Value(S_OK)
}

pub fn device_get_feature_level(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(dev_of(task, a[0]).map_or(0, |(_, d)| u64::from(lock(&d).level)))
}

pub fn device_get_immediate_context(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (_, dev) = dev_or!(task, a[0], 0);
    let context = lock(&dev).context;
    task.add_ref(context);
    let _ = task.write_ptr(cx, a[1], context);
    Reply::Value(0)
}

// ---- Direct3D 11: resources and views ----

/// `GetType(&dimension)`: a buffer (1) or a 2D texture (3).
pub fn resource_get_type(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let dimension = match task.gfx(a[0]).map(|g| g.kind) {
        Some(Kind::Buffer(_)) => 1,
        Some(Kind::Texture(_)) => 3,
        _ => 0,
    };
    task.gfx_put_u32(cx, a[1], dimension);
    Reply::Value(0)
}

pub fn resource_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((g, dev)) = dev_of(task, a[0]) else { return Reply::Value(0) };
    let words: Vec<u32> = match g.kind {
        Kind::Buffer(id) => lock(&dev).d.buffer_desc(id).map(|d| vec![d.size, d.usage, d.bind, d.cpu_access, d.misc, d.stride]).unwrap_or_default(),
        Kind::Texture(id) => lock(&dev)
            .d
            .texture_desc(id)
            .map(|d| vec![d.width, d.height, d.mip_levels, d.array_size, d.format, 1, 0, d.usage, d.bind, d.cpu_access, d.misc])
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let b: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
    task.gfx_put(cx, a[1], &b);
    Reply::Value(0)
}

/// IDXGISurface's `GetDesc(&desc)`: width, height, format, one sample.
pub fn surface_get_desc(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((g, dev)) = dev_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Kind::Texture(id) = g.kind else { return Reply::Value(E_INVALIDARG) };
    let Some(d) = lock(&dev).d.texture_desc(id) else { return Reply::Value(E_INVALIDARG) };
    let b: Vec<u8> = [d.width, d.height, d.format, 1, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
    task.gfx_put(cx, a[1], &b);
    Reply::Value(S_OK)
}

/// A view's `GetResource(&resource)`.
pub fn view_get_resource(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Kind::View { resource, .. }) = task.gfx(a[0]).map(|g| g.kind) else { return Reply::Value(0) };
    task.add_ref(resource);
    let _ = task.write_ptr(cx, a[1], resource);
    Reply::Value(0)
}

/// A query's `GetDataSize()`.
pub fn query_get_data_size(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Kind::Query { kind, .. }) = task.gfx(a[0]).map(|g| g.kind) else { return Reply::Value(0) };
    Reply::Value(match kind {
        0 | 5 => 4,
        1 | 2 => 8,
        3 => 16,
        4 => 88,
        _ => 16,
    })
}

// ---- Direct3D 11: the immediate context ----

/// Runs a context method on the device, tracing a failure (the methods give nothing back).
fn on(task: &mut Task, this: u64, f: impl FnOnce(&mut d3d::Device) -> Result<(), d3d::Error>) -> Reply {
    let Some((_, dev)) = dev_of(task, this) else { return Reply::Value(0) };
    if let Err(e) = f(&mut lock(&dev).d) {
        task.trace_gfx(&e.message);
    }
    Reply::Value(0)
}

fn set_table(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], f: fn(&mut d3d::Device, u32, &[Option<Id>])) -> Reply {
    let ids = task.gfx_ids(cx, a[3], a[2] as u32 as u64);
    on(task, a[0], |d| {
        f(d, a[1] as u32, &ids);
        Ok(())
    })
}

macro_rules! tables {
    ($($name:ident => $stage:ident $method:ident;)*) => {
        $(
            pub fn $name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
                set_table(task, cx, a, |d, first, ids| d.$method(d3d::Stage::$stage, first, ids))
            }
        )*
    };
}

tables! {
    ctx_vs_set_constant_buffers => Vertex set_constant_buffers;
    ctx_ps_set_constant_buffers => Pixel set_constant_buffers;
    ctx_cs_set_constant_buffers => Compute set_constant_buffers;
    ctx_vs_set_shader_resources => Vertex set_shader_resources;
    ctx_ps_set_shader_resources => Pixel set_shader_resources;
    ctx_cs_set_shader_resources => Compute set_shader_resources;
    ctx_vs_set_samplers => Vertex set_samplers;
    ctx_ps_set_samplers => Pixel set_samplers;
    ctx_cs_set_samplers => Compute set_samplers;
}

fn set_shader(task: &mut Task, a: &[u64], stage: d3d::Stage) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_shader(stage, id);
        Ok(())
    })
}

pub fn ctx_vs_set_shader(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_shader(task, a, d3d::Stage::Vertex)
}

pub fn ctx_ps_set_shader(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_shader(task, a, d3d::Stage::Pixel)
}

pub fn ctx_cs_set_shader(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_shader(task, a, d3d::Stage::Compute)
}

/// A geometry, hull or domain shader set: none of them are made, so only none can be.
pub fn ctx_gs_set_shader(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    if a[1] != 0 {
        task.trace_gfx("geometry and tessellation shaders aren't drawn");
    }
    Reply::Value(0)
}

/// `CSSetUnorderedAccessViews(first, count, views, counts)`.
pub fn ctx_cs_set_uavs(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ids = task.gfx_ids(cx, a[3], a[2] as u32 as u64);
    on(task, a[0], |d| {
        d.set_uavs(a[1] as u32, &ids);
        Ok(())
    })
}

pub fn ctx_ia_set_input_layout(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_input_layout(id);
        Ok(())
    })
}

/// `IASetVertexBuffers(first, count, buffers, strides, offsets)`.
pub fn ctx_ia_set_vertex_buffers(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let n = (a[2] as u32).min(32) as usize;
    let ids = task.gfx_ids(cx, a[3], n as u64);
    let strides = task.gfx_read(cx, a[4], 4 * n).unwrap_or_else(|| vec![0; 4 * n]);
    let offsets = task.gfx_read(cx, a[5], 4 * n).unwrap_or_else(|| vec![0; 4 * n]);
    let list: Vec<(Option<Id>, u32, u32)> = (0..n).map(|i| (ids[i], u32_at(&strides, 4 * i), u32_at(&offsets, 4 * i))).collect();
    on(task, a[0], |d| {
        d.set_vertex_buffers(a[1] as u32, &list);
        Ok(())
    })
}

pub fn ctx_ia_set_index_buffer(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_index_buffer(id, a[2] as u32, a[3] as u32);
        Ok(())
    })
}

pub fn ctx_ia_set_primitive_topology(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| {
        d.set_topology(a[1] as u32);
        Ok(())
    })
}

pub fn ctx_draw(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.draw(a[1] as u32, a[2] as u32))
}

pub fn ctx_draw_indexed(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.draw_indexed(a[1] as u32, a[2] as u32, a[3] as i32))
}

pub fn ctx_draw_instanced(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.draw_instanced(a[1] as u32, a[2] as u32, a[3] as u32, a[4] as u32))
}

pub fn ctx_draw_indexed_instanced(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.draw_indexed_instanced(a[1] as u32, a[2] as u32, a[3] as u32, a[4] as i32, a[5] as u32))
}

pub fn ctx_dispatch(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.dispatch(a[1] as u32, a[2] as u32, a[3] as u32))
}

/// `OMSetRenderTargets(count, views, depth_stencil)`.
pub fn ctx_om_set_render_targets(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ids = task.gfx_ids(cx, a[2], (a[1] as u32).min(8) as u64);
    let depth = task.gfx_id(a[3]);
    on(task, a[0], |d| {
        d.set_render_targets(&ids, depth);
        Ok(())
    })
}

/// `OMSetRenderTargetsAndUnorderedAccessViews(count, views, depth_stencil, first, uavs, ...)`: the targets, unless
/// they're to be kept; a pixel shader's UAVs aren't bound.
pub fn ctx_om_set_render_targets_and_uavs(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const KEEP: u32 = u32::MAX;
    if !matches!(a[5] as u32, 0 | KEEP) {
        task.trace_gfx("a pixel shader's unordered access views aren't bound");
    }
    if a[1] as u32 == KEEP {
        return Reply::Value(0);
    }
    ctx_om_set_render_targets(task, cx, a)
}

pub fn ctx_om_set_blend_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_blend_state(id);
        Ok(())
    })
}

pub fn ctx_om_set_depth_stencil_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_depth_stencil_state(id);
        Ok(())
    })
}

pub fn ctx_rs_set_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let id = task.gfx_id(a[1]);
    on(task, a[0], |d| {
        d.set_rasterizer_state(id);
        Ok(())
    })
}

/// `RSSetViewports(count, viewports)`.
pub fn ctx_rs_set_viewports(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let n = (a[1] as u32).min(16) as usize;
    let b = task.gfx_read(cx, a[2], 24 * n).unwrap_or_default();
    let list: Vec<d3d::Viewport> = b
        .as_chunks::<24>().0.iter()
        .map(|v| d3d::Viewport { x: f32_at(v, 0), y: f32_at(v, 4), width: f32_at(v, 8), height: f32_at(v, 12), min_depth: f32_at(v, 16), max_depth: f32_at(v, 20) })
        .collect();
    on(task, a[0], |d| {
        d.set_viewports(&list);
        Ok(())
    })
}

/// `RSSetScissorRects(count, rects)`.
pub fn ctx_rs_set_scissor_rects(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let n = (a[1] as u32).min(16) as usize;
    let b = task.gfx_read(cx, a[2], 16 * n).unwrap_or_default();
    let list: Vec<[i32; 4]> = b.as_chunks::<16>().0.iter().map(|r| [0, 4, 8, 12].map(|i| u32_at(r, i) as i32)).collect();
    on(task, a[0], |d| {
        d.set_scissor_rects(&list);
        Ok(())
    })
}

fn region(b: &[u8]) -> d3d::Region {
    d3d::Region { left: u32_at(b, 0), top: u32_at(b, 4), front: u32_at(b, 8), right: u32_at(b, 12), bottom: u32_at(b, 16), back: u32_at(b, 20) }
}

/// `CopySubresourceRegion(dest, sub, x, y, z, source, sub, &box)`.
pub fn ctx_copy_subresource_region(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (to, from) = (task.gfx_id(a[1]), task.gfx_id(a[6]));
    let b = task.gfx_read(cx, a[8], 24).map(|b| region(&b));
    let (Some(to), Some(from)) = (to, from) else { return Reply::Value(0) };
    on(task, a[0], |d| d.copy_subresource_region(to, a[2] as u32, a[3] as u32, a[4] as u32, from, a[7] as u32, b))
}

pub fn ctx_copy_resource(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (Some(to), Some(from)) = (task.gfx_id(a[1]), task.gfx_id(a[2])) else { return Reply::Value(0) };
    on(task, a[0], |d| d.copy_resource(to, from))
}

/// `UpdateSubresource(dest, sub, &box, data, row_pitch, depth_pitch)`: as many bytes as the region has.
pub fn ctx_update_subresource(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((_, dev)) = dev_of(task, a[0]) else { return Reply::Value(0) };
    let Some(id) = task.gfx_id(a[1]) else { return Reply::Value(0) };
    let b = task.gfx_read(cx, a[3], 24).map(|b| region(&b));
    let sub = a[2] as u32;
    let pitch = a[5] as u32;
    let len = {
        let d = lock(&dev);
        if let Some(desc) = d.d.buffer_desc(id) {
            b.map_or(desc.size, |r| r.right.saturating_sub(r.left)) as usize
        } else if let (Some(t), Some((row, size))) = (d.d.texture_desc(id), d.d.layout(id, sub)) {
            let bpp = row / (t.width >> (sub % t.mip_levels.max(1))).max(1);
            match b {
                Some(r) => {
                    let (w, h) = (r.right.saturating_sub(r.left), r.bottom.saturating_sub(r.top));
                    let p = if pitch == 0 { w * bpp } else { pitch };
                    (p * h.saturating_sub(1) + w * bpp) as usize
                }
                None => {
                    let rows = size / row.max(1);
                    let p = if pitch == 0 { row } else { pitch };
                    (p * rows.saturating_sub(1) + row) as usize
                }
            }
        } else {
            0
        }
    };
    let Some(bytes) = task.gfx_read(cx, a[4], len) else { return Reply::Value(0) };
    on(task, a[0], |d| d.update_subresource(id, sub, b, d3d::Data { bytes: &bytes, row_pitch: pitch }))
}

/// `ClearRenderTargetView(view, &rgba)`.
pub fn ctx_clear_render_target_view(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(view) = task.gfx_id(a[1]) else { return Reply::Value(0) };
    let c = task.gfx_read(cx, a[2], 16).unwrap_or_else(|| vec![0; 16]);
    let color = [f32_at(&c, 0), f32_at(&c, 4), f32_at(&c, 8), f32_at(&c, 12)];
    on(task, a[0], |d| d.clear_render_target(view, color))
}

/// `ClearDepthStencilView(view, flags, depth, stencil)`: the depth is a float, passed as floats are.
pub fn ctx_clear_depth_stencil_view(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(view) = task.gfx_id(a[1]) else { return Reply::Value(0) };
    let depth = task.f32_arg(cx, a, 3, 0);
    on(task, a[0], |d| d.clear_depth_stencil(view, a[2] as u32, depth))
}

pub fn ctx_generate_mips(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(view) = task.gfx_id(a[1]) else { return Reply::Value(0) };
    on(task, a[0], |d| d.generate_mips(view))
}

pub fn ctx_clear_state(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| {
        d.clear_state();
        Ok(())
    })
}

pub fn ctx_flush(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    on(task, a[0], |d| d.flush())
}

/// `GetData(query, data, size, flags)`: every query is done; an event's is TRUE, a timestamp's the host's time in
/// nanoseconds (its disjoint query's frequency 1 GHz), and the rest zeroes.
pub fn ctx_get_data(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Kind::Query { kind, .. }) = task.gfx(a[1]).map(|g| g.kind) else { return Reply::Value(E_INVALIDARG) };
    if let Reply::Value(_) = ctx_flush(task, cx, a) {}
    if a[2] == 0 {
        return Reply::Value(S_OK);
    }
    let mut b = vec![0u8; (a[3] as u32 as usize).min(256)];
    match kind {
        0 if b.len() >= 4 => b[..4].copy_from_slice(&1u32.to_le_bytes()),
        2 if b.len() >= 8 => b[..8].copy_from_slice(&task.k.host.monotonic().to_le_bytes()),
        3 if b.len() >= 8 => b[..8].copy_from_slice(&1_000_000_000u64.to_le_bytes()),
        _ => {}
    }
    task.gfx_put(cx, a[2], &b);
    Reply::Value(S_OK)
}

/// `Map(resource, sub, type, flags, &mapped)`: the subresource's bytes copied into the program's memory (kept there
/// for the next map), its pointer and pitches given.
pub fn ctx_map(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((_, dev)) = dev_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Some(id) = task.gfx_id(a[1]) else { return Reply::Value(E_INVALIDARG) };
    let (sub, kind) = (a[2] as u32, a[3] as u32);
    let (bytes, layout, kept) = {
        let mut d = lock(&dev);
        let bytes = match d.d.map(id, sub, kind) {
            Ok(b) => b,
            Err(e) => {
                drop(d);
                task.trace_gfx(&e.message);
                return Reply::Value(hr(e));
            }
        };
        let layout = d.d.layout(id, sub).unwrap_or((bytes.len() as u32, bytes.len() as u32));
        (bytes, layout, d.maps.get(&(id, sub)).copied())
    };
    let at = match kept {
        Some((at, size, _)) if size as usize >= bytes.len() => at,
        _ => match task.virtual_alloc(cx, 0, (bytes.len() as u64).max(1), 0x3000, 4) {
            Some(at) => at,
            None => return Reply::Value(E_OUTOFMEMORY),
        },
    };
    task.gfx_put(cx, at, &bytes);
    lock(&dev).maps.insert((id, sub), (at, bytes.len() as u32, kind));
    let abi = task.abi();
    let mut m = abi.ptr(at);
    m.extend(layout.0.to_le_bytes());
    m.extend(layout.1.to_le_bytes());
    task.gfx_put(cx, a[5], &m);
    Reply::Value(S_OK)
}

/// `Unmap(resource, sub)`: what the program wrote, back to the resource (unless it only read).
pub fn ctx_unmap(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some((_, dev)) = dev_of(task, a[0]) else { return Reply::Value(0) };
    let Some(id) = task.gfx_id(a[1]) else { return Reply::Value(0) };
    let sub = a[2] as u32;
    let Some((at, size, kind)) = lock(&dev).maps.get(&(id, sub)).copied() else { return Reply::Value(0) };
    let bytes = if kind == d3d::map::READ { None } else { task.gfx_read(cx, at, size as usize) };
    on(task, a[0], |d| d.unmap(id, sub, bytes.as_deref()))
}

// ---- The interfaces' vtables, and their methods as exports ----
// Written from a list of each interface's methods in order (d3d11.h's and dxgi1_6.h's); a slot is a method of
// gfx's, a no-op, E_NOTIMPL, or `missing` (which ends the process naming it), and `the_tables_name_declared_exports`
// checks every name is declared.

const FACTORY: [&str; 30] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxFactoryEnumAdapters",
    "AtlasGfxFactoryMakeWindowAssociation",
    "AtlasGfxFactoryGetWindowAssociation",
    "AtlasGfxFactoryCreateSwapChain",
    "AtlasGfxFactoryCreateSoftwareAdapter",
    "AtlasGfxFactoryEnumAdapters1",
    "AtlasGfxFactoryIsCurrent",
    "AtlasGfxFactoryIsWindowedStereoEnabled",
    "AtlasGfxFactoryCreateSwapChainForHwnd",
    "AtlasGfxFactoryCreateSwapChainForCoreWindow",
    "AtlasGfxFactoryGetSharedResourceAdapterLuid",
    "AtlasGfxFactoryRegisterStereoStatusWindow",
    "AtlasGfxFactoryRegisterStereoStatusEvent",
    "AtlasGfxFactoryUnregisterStereoStatus",
    "AtlasGfxFactoryRegisterOcclusionStatusWindow",
    "AtlasGfxFactoryRegisterOcclusionStatusEvent",
    "AtlasGfxFactoryUnregisterOcclusionStatus",
    "AtlasGfxFactoryCreateSwapChainForComposition",
    "AtlasGfxFactoryGetCreationFlags",
    "AtlasGfxFactoryEnumAdapterByLuid",
    "AtlasGfxFactoryEnumWarpAdapter",
    "AtlasGfxFactoryCheckFeatureSupport",
    "AtlasGfxFactoryEnumAdapterByGpuPreference",
];

const ADAPTER: [&str; 19] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxAdapterEnumOutputs",
    "AtlasGfxAdapterGetDesc",
    "AtlasGfxAdapterCheckInterfaceSupport",
    "AtlasGfxAdapterGetDesc1",
    "AtlasGfxAdapterGetDesc2",
    "AtlasGfxAdapterRegisterHardwareContentProtectionTeardownStatusEvent",
    "AtlasGfxAdapterUnregisterHardwareContentProtectionTeardownStatus",
    "AtlasGfxAdapterQueryVideoMemoryInfo",
    "AtlasGfxAdapterSetVideoMemoryReservation",
    "AtlasGfxAdapterRegisterVideoMemoryBudgetChangeNotificationEvent",
    "AtlasGfxAdapterUnregisterVideoMemoryBudgetChangeNotification",
    "AtlasGfxAdapterGetDesc3",
];

const OUTPUT: [&str; 19] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxOutputGetDesc",
    "AtlasGfxOutputGetDisplayModeList",
    "AtlasGfxOutputFindClosestMatchingMode",
    "AtlasGfxOutputWaitForVBlank",
    "AtlasGfxOutputTakeOwnership",
    "AtlasGfxOutputReleaseOwnership",
    "AtlasGfxOutputGetGammaControlCapabilities",
    "AtlasGfxOutputSetGammaControl",
    "AtlasGfxOutputGetGammaControl",
    "AtlasGfxOutputSetDisplaySurface",
    "AtlasGfxOutputGetDisplaySurfaceData",
    "AtlasGfxOutputGetFrameStatistics",
];

const SWAP_CHAIN: [&str; 40] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxSubGetDevice",
    "AtlasGfxSwapPresent",
    "AtlasGfxSwapGetBuffer",
    "AtlasGfxSwapSetFullscreenState",
    "AtlasGfxSwapGetFullscreenState",
    "AtlasGfxSwapGetDesc",
    "AtlasGfxSwapResizeBuffers",
    "AtlasGfxSwapResizeTarget",
    "AtlasGfxSwapGetContainingOutput",
    "AtlasGfxSwapGetFrameStatistics",
    "AtlasGfxSwapGetLastPresentCount",
    "AtlasGfxSwapGetDesc1",
    "AtlasGfxSwapGetFullscreenDesc",
    "AtlasGfxSwapGetHwnd",
    "AtlasGfxSwapGetCoreWindow",
    "AtlasGfxSwapPresent1",
    "AtlasGfxSwapIsTemporaryMonoSupported",
    "AtlasGfxSwapGetRestrictToOutput",
    "AtlasGfxSwapSetBackgroundColor",
    "AtlasGfxSwapGetBackgroundColor",
    "AtlasGfxSwapSetRotation",
    "AtlasGfxSwapGetRotation",
    "AtlasGfxSwapSetSourceSize",
    "AtlasGfxSwapGetSourceSize",
    "AtlasGfxSwapSetMaximumFrameLatency",
    "AtlasGfxSwapGetMaximumFrameLatency",
    "AtlasGfxSwapGetFrameLatencyWaitableObject",
    "AtlasGfxSwapSetMatrixTransform",
    "AtlasGfxSwapGetMatrixTransform",
    "AtlasGfxSwapGetCurrentBackBufferIndex",
    "AtlasGfxSwapCheckColorSpaceSupport",
    "AtlasGfxSwapSetColorSpace1",
    "AtlasGfxSwapResizeBuffers1",
];

const DXGI_DEVICE: [&str; 18] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxDxgiDeviceGetAdapter",
    "AtlasGfxDxgiDeviceCreateSurface",
    "AtlasGfxDxgiDeviceQueryResourceResidency",
    "AtlasGfxDxgiDeviceSetGPUThreadPriority",
    "AtlasGfxDxgiDeviceGetGPUThreadPriority",
    "AtlasGfxDxgiDeviceSetMaximumFrameLatency",
    "AtlasGfxDxgiDeviceGetMaximumFrameLatency",
    "AtlasGfxDxgiDeviceOfferResources",
    "AtlasGfxDxgiDeviceReclaimResources",
    "AtlasGfxDxgiDeviceEnqueueSetEvent",
    "AtlasGfxDxgiDeviceTrim",
];

const DEVICE: [&str; 43] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxDeviceCreateBuffer",
    "AtlasGfxDeviceCreateTexture1D",
    "AtlasGfxDeviceCreateTexture2D",
    "AtlasGfxDeviceCreateTexture3D",
    "AtlasGfxDeviceCreateShaderResourceView",
    "AtlasGfxDeviceCreateUnorderedAccessView",
    "AtlasGfxDeviceCreateRenderTargetView",
    "AtlasGfxDeviceCreateDepthStencilView",
    "AtlasGfxDeviceCreateInputLayout",
    "AtlasGfxDeviceCreateVertexShader",
    "AtlasGfxDeviceCreateGeometryShader",
    "AtlasGfxDeviceCreateGeometryShaderWithStreamOutput",
    "AtlasGfxDeviceCreatePixelShader",
    "AtlasGfxDeviceCreateHullShader",
    "AtlasGfxDeviceCreateDomainShader",
    "AtlasGfxDeviceCreateComputeShader",
    "AtlasGfxDeviceCreateClassLinkage",
    "AtlasGfxDeviceCreateBlendState",
    "AtlasGfxDeviceCreateDepthStencilState",
    "AtlasGfxDeviceCreateRasterizerState",
    "AtlasGfxDeviceCreateSamplerState",
    "AtlasGfxDeviceCreateQuery",
    "AtlasGfxDeviceCreatePredicate",
    "AtlasGfxDeviceCreateCounter",
    "AtlasGfxDeviceCreateDeferredContext",
    "AtlasGfxDeviceOpenSharedResource",
    "AtlasGfxDeviceCheckFormatSupport",
    "AtlasGfxDeviceCheckMultisampleQualityLevels",
    "AtlasGfxDeviceCheckCounterInfo",
    "AtlasGfxDeviceCheckCounter",
    "AtlasGfxDeviceCheckFeatureSupport",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxDeviceGetFeatureLevel",
    "AtlasGfxDeviceGetCreationFlags",
    "AtlasGfxDeviceGetDeviceRemovedReason",
    "AtlasGfxDeviceGetImmediateContext",
    "AtlasGfxDeviceSetExceptionMode",
    "AtlasGfxDeviceGetExceptionMode",
];

const CONTEXT: [&str; 115] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxContextVSSetConstantBuffers",
    "AtlasGfxContextPSSetShaderResources",
    "AtlasGfxContextPSSetShader",
    "AtlasGfxContextPSSetSamplers",
    "AtlasGfxContextVSSetShader",
    "AtlasGfxContextDrawIndexed",
    "AtlasGfxContextDraw",
    "AtlasGfxContextMap",
    "AtlasGfxContextUnmap",
    "AtlasGfxContextPSSetConstantBuffers",
    "AtlasGfxContextIASetInputLayout",
    "AtlasGfxContextIASetVertexBuffers",
    "AtlasGfxContextIASetIndexBuffer",
    "AtlasGfxContextDrawIndexedInstanced",
    "AtlasGfxContextDrawInstanced",
    "AtlasGfxContextGSSetConstantBuffers",
    "AtlasGfxContextGSSetShader",
    "AtlasGfxContextIASetPrimitiveTopology",
    "AtlasGfxContextVSSetShaderResources",
    "AtlasGfxContextVSSetSamplers",
    "AtlasGfxContextBegin",
    "AtlasGfxContextEnd",
    "AtlasGfxContextGetData",
    "AtlasGfxContextSetPredication",
    "AtlasGfxContextGSSetShaderResources",
    "AtlasGfxContextGSSetSamplers",
    "AtlasGfxContextOMSetRenderTargets",
    "AtlasGfxContextOMSetRenderTargetsAndUnorderedAccessViews",
    "AtlasGfxContextOMSetBlendState",
    "AtlasGfxContextOMSetDepthStencilState",
    "AtlasGfxContextSOSetTargets",
    "AtlasGfxContextDrawAuto",
    "AtlasGfxContextDrawIndexedInstancedIndirect",
    "AtlasGfxContextDrawInstancedIndirect",
    "AtlasGfxContextDispatch",
    "AtlasGfxContextDispatchIndirect",
    "AtlasGfxContextRSSetState",
    "AtlasGfxContextRSSetViewports",
    "AtlasGfxContextRSSetScissorRects",
    "AtlasGfxContextCopySubresourceRegion",
    "AtlasGfxContextCopyResource",
    "AtlasGfxContextUpdateSubresource",
    "AtlasGfxContextCopyStructureCount",
    "AtlasGfxContextClearRenderTargetView",
    "AtlasGfxContextClearUnorderedAccessViewUint",
    "AtlasGfxContextClearUnorderedAccessViewFloat",
    "AtlasGfxContextClearDepthStencilView",
    "AtlasGfxContextGenerateMips",
    "AtlasGfxContextSetResourceMinLOD",
    "AtlasGfxContextGetResourceMinLOD",
    "AtlasGfxContextResolveSubresource",
    "AtlasGfxContextExecuteCommandList",
    "AtlasGfxContextHSSetShaderResources",
    "AtlasGfxContextHSSetShader",
    "AtlasGfxContextHSSetSamplers",
    "AtlasGfxContextHSSetConstantBuffers",
    "AtlasGfxContextDSSetShaderResources",
    "AtlasGfxContextDSSetShader",
    "AtlasGfxContextDSSetSamplers",
    "AtlasGfxContextDSSetConstantBuffers",
    "AtlasGfxContextCSSetShaderResources",
    "AtlasGfxContextCSSetUnorderedAccessViews",
    "AtlasGfxContextCSSetShader",
    "AtlasGfxContextCSSetSamplers",
    "AtlasGfxContextCSSetConstantBuffers",
    "AtlasGfxContextVSGetConstantBuffers",
    "AtlasGfxContextPSGetShaderResources",
    "AtlasGfxContextPSGetShader",
    "AtlasGfxContextPSGetSamplers",
    "AtlasGfxContextVSGetShader",
    "AtlasGfxContextPSGetConstantBuffers",
    "AtlasGfxContextIAGetInputLayout",
    "AtlasGfxContextIAGetVertexBuffers",
    "AtlasGfxContextIAGetIndexBuffer",
    "AtlasGfxContextGSGetConstantBuffers",
    "AtlasGfxContextGSGetShader",
    "AtlasGfxContextIAGetPrimitiveTopology",
    "AtlasGfxContextVSGetShaderResources",
    "AtlasGfxContextVSGetSamplers",
    "AtlasGfxContextGetPredication",
    "AtlasGfxContextGSGetShaderResources",
    "AtlasGfxContextGSGetSamplers",
    "AtlasGfxContextOMGetRenderTargets",
    "AtlasGfxContextOMGetRenderTargetsAndUnorderedAccessViews",
    "AtlasGfxContextOMGetBlendState",
    "AtlasGfxContextOMGetDepthStencilState",
    "AtlasGfxContextSOGetTargets",
    "AtlasGfxContextRSGetState",
    "AtlasGfxContextRSGetViewports",
    "AtlasGfxContextRSGetScissorRects",
    "AtlasGfxContextHSGetShaderResources",
    "AtlasGfxContextHSGetShader",
    "AtlasGfxContextHSGetSamplers",
    "AtlasGfxContextHSGetConstantBuffers",
    "AtlasGfxContextDSGetShaderResources",
    "AtlasGfxContextDSGetShader",
    "AtlasGfxContextDSGetSamplers",
    "AtlasGfxContextDSGetConstantBuffers",
    "AtlasGfxContextCSGetShaderResources",
    "AtlasGfxContextCSGetUnorderedAccessViews",
    "AtlasGfxContextCSGetShader",
    "AtlasGfxContextCSGetSamplers",
    "AtlasGfxContextCSGetConstantBuffers",
    "AtlasGfxContextClearState",
    "AtlasGfxContextFlush",
    "AtlasGfxContextGetType",
    "AtlasGfxContextGetContextFlags",
    "AtlasGfxContextFinishCommandList",
];

const RESOURCE: [&str; 11] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxResourceGetType",
    "AtlasGfxResourceSetEvictionPriority",
    "AtlasGfxResourceGetEvictionPriority",
    "AtlasGfxResourceGetDesc",
];

const SURFACE: [&str; 11] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxGetPrivateData",
    "AtlasGfxGetParent",
    "AtlasGfxSubGetDevice",
    "AtlasGfxSurfaceGetDesc",
    "AtlasGfxSurfaceMap",
    "AtlasGfxSurfaceUnmap",
];

const VIEW: [&str; 9] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxViewGetResource",
    "AtlasGfxViewGetDesc",
];

const STATE: [&str; 8] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxStateGetDesc",
];

const CHILD: [&str; 7] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
];

const QUERY: [&str; 9] = [
    "AtlasComQueryInterface",
    "AtlasComAddRef",
    "AtlasComRelease",
    "AtlasGfxChildGetDevice",
    "AtlasGfxGetPrivateData",
    "AtlasGfxSetPrivateData",
    "AtlasGfxSetPrivateDataInterface",
    "AtlasGfxQueryGetDataSize",
    "AtlasGfxQueryGetDesc",
];

/// Declares a DLL with every graphics method among its exports: both DXGI and Direct3D 11 have them all, so an
/// object's vtables are always in a DLL its process has loaded (the one that made it, or its parent).
macro_rules! gfx_dll {
    ($file:literal; $($own:tt)*) => {
        $crate::os::dll! {
            $file;
            $($own)*
            host AtlasComQueryInterface(3) => com_query_interface;
            host AtlasComAddRef(1) => com_add_ref;
            host AtlasComRelease(1) => com_release;
            host AtlasGfxSetPrivateData(4) => set_private_data;
            host AtlasGfxSetPrivateDataInterface(3) => set_private_data_interface;
            host AtlasGfxGetPrivateData(4) => get_private_data;
            host AtlasGfxGetParent(3) => get_parent;
            host AtlasGfxFactoryEnumAdapters(3) => factory_enum_adapters;
            host AtlasGfxFactoryMakeWindowAssociation(3) => s_ok;
            host AtlasGfxFactoryGetWindowAssociation(2) => zero_out;
            host AtlasGfxFactoryCreateSwapChain(4) => factory_create_swap_chain;
            host AtlasGfxFactoryCreateSoftwareAdapter(3) => not_implemented;
            host AtlasGfxFactoryEnumAdapters1(3) => factory_enum_adapters;
            host AtlasGfxFactoryIsCurrent(1) => true_;
            host AtlasGfxFactoryIsWindowedStereoEnabled(1) => s_ok;
            host AtlasGfxFactoryCreateSwapChainForHwnd(7) => factory_create_swap_chain_for_hwnd;
            host AtlasGfxFactoryCreateSwapChainForCoreWindow(6) => not_implemented;
            host AtlasGfxFactoryGetSharedResourceAdapterLuid(3) => not_implemented;
            host AtlasGfxFactoryRegisterStereoStatusWindow(4) => not_implemented;
            host AtlasGfxFactoryRegisterStereoStatusEvent(3) => not_implemented;
            host AtlasGfxFactoryUnregisterStereoStatus(2) => s_ok;
            host AtlasGfxFactoryRegisterOcclusionStatusWindow(4) => not_implemented;
            host AtlasGfxFactoryRegisterOcclusionStatusEvent(3) => not_implemented;
            host AtlasGfxFactoryUnregisterOcclusionStatus(2) => s_ok;
            host AtlasGfxFactoryCreateSwapChainForComposition(5) => not_implemented;
            host AtlasGfxFactoryGetCreationFlags(1) => s_ok;
            missing AtlasGfxFactoryEnumAdapterByLuid;
            host AtlasGfxFactoryEnumWarpAdapter(3) => factory_enum_warp_adapter;
            host AtlasGfxFactoryCheckFeatureSupport(4) => factory_check_feature_support;
            host AtlasGfxFactoryEnumAdapterByGpuPreference(5) => factory_enum_adapter_by_gpu_preference;
            host AtlasGfxAdapterEnumOutputs(3) => adapter_enum_outputs;
            host AtlasGfxAdapterGetDesc(2) => adapter_get_desc;
            host AtlasGfxAdapterCheckInterfaceSupport(3) => adapter_check_interface_support;
            host AtlasGfxAdapterGetDesc1(2) => adapter_get_desc1;
            host AtlasGfxAdapterGetDesc2(2) => adapter_get_desc2;
            host AtlasGfxAdapterRegisterHardwareContentProtectionTeardownStatusEvent(3) => not_implemented;
            host AtlasGfxAdapterUnregisterHardwareContentProtectionTeardownStatus(2) => s_ok;
            host AtlasGfxAdapterQueryVideoMemoryInfo(4) => adapter_query_video_memory_info;
            missing AtlasGfxAdapterSetVideoMemoryReservation;
            host AtlasGfxAdapterRegisterVideoMemoryBudgetChangeNotificationEvent(3) => not_implemented;
            host AtlasGfxAdapterUnregisterVideoMemoryBudgetChangeNotification(2) => s_ok;
            host AtlasGfxAdapterGetDesc3(2) => adapter_get_desc3;
            host AtlasGfxOutputGetDesc(2) => output_get_desc;
            host AtlasGfxOutputGetDisplayModeList(5) => output_get_display_mode_list;
            host AtlasGfxOutputFindClosestMatchingMode(4) => output_find_closest_matching_mode;
            host AtlasGfxOutputWaitForVBlank(1) => output_wait_for_vblank;
            host AtlasGfxOutputTakeOwnership(3) => not_implemented;
            host AtlasGfxOutputReleaseOwnership(1) => s_ok;
            host AtlasGfxOutputGetGammaControlCapabilities(2) => not_implemented;
            host AtlasGfxOutputSetGammaControl(2) => not_implemented;
            host AtlasGfxOutputGetGammaControl(2) => not_implemented;
            host AtlasGfxOutputSetDisplaySurface(2) => not_implemented;
            host AtlasGfxOutputGetDisplaySurfaceData(2) => not_implemented;
            host AtlasGfxOutputGetFrameStatistics(2) => frame_statistics;
            host AtlasGfxSubGetDevice(3) => sub_get_device;
            host AtlasGfxSwapPresent(3) => swap_present;
            host AtlasGfxSwapGetBuffer(4) => swap_get_buffer;
            host AtlasGfxSwapSetFullscreenState(3) => swap_set_fullscreen_state;
            host AtlasGfxSwapGetFullscreenState(3) => swap_get_fullscreen_state;
            host AtlasGfxSwapGetDesc(2) => swap_get_desc;
            host AtlasGfxSwapResizeBuffers(6) => swap_resize_buffers;
            host AtlasGfxSwapResizeTarget(2) => s_ok;
            host AtlasGfxSwapGetContainingOutput(2) => swap_get_containing_output;
            host AtlasGfxSwapGetFrameStatistics(2) => frame_statistics;
            host AtlasGfxSwapGetLastPresentCount(2) => swap_get_last_present_count;
            host AtlasGfxSwapGetDesc1(2) => swap_get_desc1;
            host AtlasGfxSwapGetFullscreenDesc(2) => swap_get_fullscreen_desc;
            host AtlasGfxSwapGetHwnd(2) => swap_get_hwnd;
            host AtlasGfxSwapGetCoreWindow(3) => not_implemented;
            host AtlasGfxSwapPresent1(4) => swap_present;
            host AtlasGfxSwapIsTemporaryMonoSupported(1) => s_ok;
            host AtlasGfxSwapGetRestrictToOutput(2) => zero_out;
            host AtlasGfxSwapSetBackgroundColor(2) => s_ok;
            host AtlasGfxSwapGetBackgroundColor(2) => not_implemented;
            host AtlasGfxSwapSetRotation(2) => s_ok;
            host AtlasGfxSwapGetRotation(2) => swap_get_rotation;
            host AtlasGfxSwapSetSourceSize(3) => s_ok;
            host AtlasGfxSwapGetSourceSize(3) => swap_get_source_size;
            host AtlasGfxSwapSetMaximumFrameLatency(2) => s_ok;
            host AtlasGfxSwapGetMaximumFrameLatency(2) => get_maximum_frame_latency;
            missing AtlasGfxSwapGetFrameLatencyWaitableObject;
            host AtlasGfxSwapSetMatrixTransform(2) => not_implemented;
            host AtlasGfxSwapGetMatrixTransform(2) => not_implemented;
            host AtlasGfxSwapGetCurrentBackBufferIndex(1) => s_ok;
            host AtlasGfxSwapCheckColorSpaceSupport(3) => swap_check_color_space_support;
            host AtlasGfxSwapSetColorSpace1(2) => s_ok;
            missing AtlasGfxSwapResizeBuffers1;
            host AtlasGfxDxgiDeviceGetAdapter(2) => dxgi_device_get_adapter;
            host AtlasGfxDxgiDeviceCreateSurface(6) => not_implemented;
            host AtlasGfxDxgiDeviceQueryResourceResidency(4) => not_implemented;
            host AtlasGfxDxgiDeviceSetGPUThreadPriority(2) => s_ok;
            host AtlasGfxDxgiDeviceGetGPUThreadPriority(2) => zero_out32;
            host AtlasGfxDxgiDeviceSetMaximumFrameLatency(2) => s_ok;
            host AtlasGfxDxgiDeviceGetMaximumFrameLatency(2) => get_maximum_frame_latency;
            host AtlasGfxDxgiDeviceOfferResources(4) => s_ok;
            host AtlasGfxDxgiDeviceReclaimResources(4) => dxgi_device_reclaim_resources;
            host AtlasGfxDxgiDeviceEnqueueSetEvent(2) => dxgi_device_enqueue_set_event;
            host AtlasGfxDxgiDeviceTrim(1) => s_ok;
            host AtlasGfxDeviceCreateBuffer(4) => device_create_buffer;
            host AtlasGfxDeviceCreateTexture1D(4) => not_implemented;
            host AtlasGfxDeviceCreateTexture2D(4) => device_create_texture2d;
            host AtlasGfxDeviceCreateTexture3D(4) => not_implemented;
            host AtlasGfxDeviceCreateShaderResourceView(4) => device_create_srv;
            host AtlasGfxDeviceCreateUnorderedAccessView(4) => device_create_uav;
            host AtlasGfxDeviceCreateRenderTargetView(4) => device_create_rtv;
            host AtlasGfxDeviceCreateDepthStencilView(4) => device_create_dsv;
            host AtlasGfxDeviceCreateInputLayout(6) => device_create_input_layout;
            host AtlasGfxDeviceCreateVertexShader(5) => device_create_vertex_shader;
            host AtlasGfxDeviceCreateGeometryShader(5) => not_implemented;
            host AtlasGfxDeviceCreateGeometryShaderWithStreamOutput(10) => not_implemented;
            host AtlasGfxDeviceCreatePixelShader(5) => device_create_pixel_shader;
            host AtlasGfxDeviceCreateHullShader(5) => not_implemented;
            host AtlasGfxDeviceCreateDomainShader(5) => not_implemented;
            host AtlasGfxDeviceCreateComputeShader(5) => device_create_compute_shader;
            host AtlasGfxDeviceCreateClassLinkage(2) => not_implemented;
            host AtlasGfxDeviceCreateBlendState(3) => device_create_blend_state;
            host AtlasGfxDeviceCreateDepthStencilState(3) => device_create_depth_stencil_state;
            host AtlasGfxDeviceCreateRasterizerState(3) => device_create_rasterizer_state;
            host AtlasGfxDeviceCreateSamplerState(3) => device_create_sampler_state;
            host AtlasGfxDeviceCreateQuery(3) => device_create_query;
            host AtlasGfxDeviceCreatePredicate(3) => not_implemented;
            host AtlasGfxDeviceCreateCounter(3) => not_implemented;
            host AtlasGfxDeviceCreateDeferredContext(3) => not_implemented;
            host AtlasGfxDeviceOpenSharedResource(4) => not_implemented;
            host AtlasGfxDeviceCheckFormatSupport(3) => device_check_format_support;
            host AtlasGfxDeviceCheckMultisampleQualityLevels(4) => device_check_multisample_quality_levels;
            host AtlasGfxDeviceCheckCounterInfo(2) => device_check_counter_info;
            host AtlasGfxDeviceCheckCounter(10) => not_implemented;
            host AtlasGfxDeviceCheckFeatureSupport(4) => device_check_feature_support;
            host AtlasGfxDeviceGetFeatureLevel(1) => device_get_feature_level;
            host AtlasGfxDeviceGetCreationFlags(1) => s_ok;
            host AtlasGfxDeviceGetDeviceRemovedReason(1) => s_ok;
            host AtlasGfxDeviceGetImmediateContext(2) => device_get_immediate_context;
            host AtlasGfxDeviceSetExceptionMode(2) => s_ok;
            host AtlasGfxDeviceGetExceptionMode(1) => s_ok;
            host AtlasGfxChildGetDevice(2) => child_get_device;
            host AtlasGfxContextVSSetConstantBuffers(4) => ctx_vs_set_constant_buffers;
            host AtlasGfxContextPSSetShaderResources(4) => ctx_ps_set_shader_resources;
            host AtlasGfxContextPSSetShader(4) => ctx_ps_set_shader;
            host AtlasGfxContextPSSetSamplers(4) => ctx_ps_set_samplers;
            host AtlasGfxContextVSSetShader(4) => ctx_vs_set_shader;
            host AtlasGfxContextDrawIndexed(4) => ctx_draw_indexed;
            host AtlasGfxContextDraw(3) => ctx_draw;
            host AtlasGfxContextMap(6) => ctx_map;
            host AtlasGfxContextUnmap(3) => ctx_unmap;
            host AtlasGfxContextPSSetConstantBuffers(4) => ctx_ps_set_constant_buffers;
            host AtlasGfxContextIASetInputLayout(2) => ctx_ia_set_input_layout;
            host AtlasGfxContextIASetVertexBuffers(6) => ctx_ia_set_vertex_buffers;
            host AtlasGfxContextIASetIndexBuffer(4) => ctx_ia_set_index_buffer;
            host AtlasGfxContextDrawIndexedInstanced(6) => ctx_draw_indexed_instanced;
            host AtlasGfxContextDrawInstanced(5) => ctx_draw_instanced;
            host AtlasGfxContextGSSetConstantBuffers(4) => s_ok;
            host AtlasGfxContextGSSetShader(4) => ctx_gs_set_shader;
            host AtlasGfxContextIASetPrimitiveTopology(2) => ctx_ia_set_primitive_topology;
            host AtlasGfxContextVSSetShaderResources(4) => ctx_vs_set_shader_resources;
            host AtlasGfxContextVSSetSamplers(4) => ctx_vs_set_samplers;
            host AtlasGfxContextBegin(2) => s_ok;
            host AtlasGfxContextEnd(2) => s_ok;
            host AtlasGfxContextGetData(5) => ctx_get_data;
            host AtlasGfxContextSetPredication(3) => s_ok;
            host AtlasGfxContextGSSetShaderResources(4) => s_ok;
            host AtlasGfxContextGSSetSamplers(4) => s_ok;
            host AtlasGfxContextOMSetRenderTargets(4) => ctx_om_set_render_targets;
            host AtlasGfxContextOMSetRenderTargetsAndUnorderedAccessViews(8) => ctx_om_set_render_targets_and_uavs;
            host AtlasGfxContextOMSetBlendState(4) => ctx_om_set_blend_state;
            host AtlasGfxContextOMSetDepthStencilState(3) => ctx_om_set_depth_stencil_state;
            host AtlasGfxContextSOSetTargets(4) => s_ok;
            missing AtlasGfxContextDrawAuto;
            missing AtlasGfxContextDrawIndexedInstancedIndirect;
            missing AtlasGfxContextDrawInstancedIndirect;
            host AtlasGfxContextDispatch(4) => ctx_dispatch;
            missing AtlasGfxContextDispatchIndirect;
            host AtlasGfxContextRSSetState(2) => ctx_rs_set_state;
            host AtlasGfxContextRSSetViewports(3) => ctx_rs_set_viewports;
            host AtlasGfxContextRSSetScissorRects(3) => ctx_rs_set_scissor_rects;
            host AtlasGfxContextCopySubresourceRegion(9) => ctx_copy_subresource_region;
            host AtlasGfxContextCopyResource(3) => ctx_copy_resource;
            host AtlasGfxContextUpdateSubresource(7) => ctx_update_subresource;
            missing AtlasGfxContextCopyStructureCount;
            host AtlasGfxContextClearRenderTargetView(3) => ctx_clear_render_target_view;
            missing AtlasGfxContextClearUnorderedAccessViewUint;
            missing AtlasGfxContextClearUnorderedAccessViewFloat;
            host AtlasGfxContextClearDepthStencilView(5) => ctx_clear_depth_stencil_view;
            host AtlasGfxContextGenerateMips(2) => ctx_generate_mips;
            host AtlasGfxContextSetResourceMinLOD(3) => s_ok;
            missing AtlasGfxContextGetResourceMinLOD;
            missing AtlasGfxContextResolveSubresource;
            missing AtlasGfxContextExecuteCommandList;
            host AtlasGfxContextHSSetShaderResources(4) => s_ok;
            host AtlasGfxContextHSSetShader(4) => ctx_gs_set_shader;
            host AtlasGfxContextHSSetSamplers(4) => s_ok;
            host AtlasGfxContextHSSetConstantBuffers(4) => s_ok;
            host AtlasGfxContextDSSetShaderResources(4) => s_ok;
            host AtlasGfxContextDSSetShader(4) => ctx_gs_set_shader;
            host AtlasGfxContextDSSetSamplers(4) => s_ok;
            host AtlasGfxContextDSSetConstantBuffers(4) => s_ok;
            host AtlasGfxContextCSSetShaderResources(4) => ctx_cs_set_shader_resources;
            host AtlasGfxContextCSSetUnorderedAccessViews(5) => ctx_cs_set_uavs;
            host AtlasGfxContextCSSetShader(4) => ctx_cs_set_shader;
            host AtlasGfxContextCSSetSamplers(4) => ctx_cs_set_samplers;
            host AtlasGfxContextCSSetConstantBuffers(4) => ctx_cs_set_constant_buffers;
            missing AtlasGfxContextVSGetConstantBuffers;
            missing AtlasGfxContextPSGetShaderResources;
            missing AtlasGfxContextPSGetShader;
            missing AtlasGfxContextPSGetSamplers;
            missing AtlasGfxContextVSGetShader;
            missing AtlasGfxContextPSGetConstantBuffers;
            missing AtlasGfxContextIAGetInputLayout;
            missing AtlasGfxContextIAGetVertexBuffers;
            missing AtlasGfxContextIAGetIndexBuffer;
            missing AtlasGfxContextGSGetConstantBuffers;
            missing AtlasGfxContextGSGetShader;
            missing AtlasGfxContextIAGetPrimitiveTopology;
            missing AtlasGfxContextVSGetShaderResources;
            missing AtlasGfxContextVSGetSamplers;
            missing AtlasGfxContextGetPredication;
            missing AtlasGfxContextGSGetShaderResources;
            missing AtlasGfxContextGSGetSamplers;
            missing AtlasGfxContextOMGetRenderTargets;
            missing AtlasGfxContextOMGetRenderTargetsAndUnorderedAccessViews;
            missing AtlasGfxContextOMGetBlendState;
            missing AtlasGfxContextOMGetDepthStencilState;
            missing AtlasGfxContextSOGetTargets;
            missing AtlasGfxContextRSGetState;
            missing AtlasGfxContextRSGetViewports;
            missing AtlasGfxContextRSGetScissorRects;
            missing AtlasGfxContextHSGetShaderResources;
            missing AtlasGfxContextHSGetShader;
            missing AtlasGfxContextHSGetSamplers;
            missing AtlasGfxContextHSGetConstantBuffers;
            missing AtlasGfxContextDSGetShaderResources;
            missing AtlasGfxContextDSGetShader;
            missing AtlasGfxContextDSGetSamplers;
            missing AtlasGfxContextDSGetConstantBuffers;
            missing AtlasGfxContextCSGetShaderResources;
            missing AtlasGfxContextCSGetUnorderedAccessViews;
            missing AtlasGfxContextCSGetShader;
            missing AtlasGfxContextCSGetSamplers;
            missing AtlasGfxContextCSGetConstantBuffers;
            host AtlasGfxContextClearState(1) => ctx_clear_state;
            host AtlasGfxContextFlush(1) => ctx_flush;
            host AtlasGfxContextGetType(1) => s_ok;
            host AtlasGfxContextGetContextFlags(1) => s_ok;
            missing AtlasGfxContextFinishCommandList;
            host AtlasGfxResourceGetType(2) => resource_get_type;
            host AtlasGfxResourceSetEvictionPriority(2) => s_ok;
            host AtlasGfxResourceGetEvictionPriority(1) => s_ok;
            host AtlasGfxResourceGetDesc(2) => resource_get_desc;
            host AtlasGfxSurfaceGetDesc(2) => surface_get_desc;
            host AtlasGfxSurfaceMap(3) => not_implemented;
            host AtlasGfxSurfaceUnmap(1) => not_implemented;
            host AtlasGfxViewGetResource(2) => view_get_resource;
            host AtlasGfxViewGetDesc(2) => object_get_desc;
            host AtlasGfxStateGetDesc(2) => object_get_desc;
            host AtlasGfxQueryGetDataSize(1) => query_get_data_size;
            host AtlasGfxQueryGetDesc(2) => object_get_desc;
        }
    };
}
pub(crate) use gfx_dll;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_name_declared_exports() {
        for dll in [&super::super::dxgi::DLL, &super::super::d3d11::DLL] {
            let names: Vec<&str> = dll.exports.iter().map(|e| e.name).collect();
            for table in [&FACTORY[..], &ADAPTER, &OUTPUT, &SWAP_CHAIN, &DXGI_DEVICE, &DEVICE, &CONTEXT, &RESOURCE, &SURFACE, &VIEW, &STATE, &CHILD, &QUERY] {
                for name in table {
                    assert!(names.contains(name), "{} doesn't declare {name}", dll.name);
                }
            }
        }
    }

    #[test]
    fn interface_counts_are_d3d11s() {
        // IUnknown's 3, ID3D11DeviceChild's 4, and ID3D11DeviceContext's 108.
        assert_eq!(CONTEXT.len(), 115);
        assert_eq!(CONTEXT[13], "AtlasGfxContextDraw");
        assert_eq!(CONTEXT[41], "AtlasGfxContextDispatch");
        assert_eq!(CONTEXT[111], "AtlasGfxContextFlush");
        // ID3D11Device's 40 after IUnknown.
        assert_eq!(DEVICE.len(), 43);
        assert_eq!(DEVICE[5], "AtlasGfxDeviceCreateTexture2D");
        assert_eq!(DEVICE[40], "AtlasGfxDeviceGetImmediateContext");
        assert_eq!(SWAP_CHAIN[8], "AtlasGfxSwapPresent");
    }

    #[test]
    fn guids_as_laid_out() {
        assert_eq!(IID_D3D11_DEVICE[..4], [0xdb, 0x6d, 0x6f, 0xdb]);
        assert_eq!(IID_DXGI_FACTORY1[8..], [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
    }
}
