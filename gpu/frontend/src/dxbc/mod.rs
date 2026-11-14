//! DXBC, Direct3D 10 and 11's shaders (shader models 4.0 to 5.0), as fxc and vkd3d's compiler write them: the
//! container (`container`), the program's tokens (`tokens`) and their translation (`translate`).
//!
//! Read so far: vertex, pixel and compute shaders; their inputs and outputs by register (a location is the register
//! number, so a vertex shader's outputs and a pixel shader's inputs meet where D3D11's linkage puts them, and an input
//! layout's elements are attributes at their register); `SV_Position` (and a pixel shader's, with D3D's `1/w` in w),
//! `SV_VertexID`, `SV_InstanceID`, `SV_IsFrontFace`, `SV_Target#`, `SV_Depth` and the compute thread ids; constant
//! buffers (dynamically indexed too), the immediate constant buffer, indexable temps and group-shared memory; the
//! arithmetic, integer, bit, conversion and comparison instructions; structured control flow, `switch` included;
//! `sample`, `sample_l` and `sample_b` on 2D textures; raw and structured buffers, their atomics, and `sync`.
//!
//! Each stage has its own tables of slots, so a binding's set is the kind of slot and its binding the slot number:
//! [`SET_CONSTANTS`], [`SET_RESOURCES`], [`SET_SAMPLERS`] and [`SET_UAVS`].

mod container;
mod tokens;
mod translate;

use alloc::string::String;
use alloc::vec::Vec;

use gpu_ir::Entry;

pub use container::{Component, Element};
pub use translate::{SET_CONSTANTS, SET_RESOURCES, SET_SAMPLERS, SET_UAVS};

use crate::Error;

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error { message: message.into() })
}

/// A program read: its entry point, and its signatures, by which the API links stages and input layouts.
#[derive(Debug, Clone, PartialEq)]
pub struct Shader {
    pub entry: Entry,
    pub inputs: Vec<Element>,
    pub outputs: Vec<Element>,
}

/// A DXBC container's shader, as IR.
pub fn read(bytes: &[u8]) -> Result<Shader, Error> {
    let c = container::Container::read(bytes)?;
    let Some((_, program)) = c.chunk(&[b"SHEX", b"SHDR"]) else { return fail("the container has no program (SHEX or SHDR)") };
    let inputs = match c.chunk(&[b"ISGN", b"ISG1"]) {
        Some((code, data)) => container::signature(code, data)?,
        None => Vec::new(),
    };
    let outputs = match c.chunk(&[b"OSGN", b"OSG5", b"OSG1"]) {
        Some((code, data)) => container::signature(code, data)?,
        None => Vec::new(),
    };
    let program = tokens::read(program)?;
    translate::translate(&program, &inputs, &outputs)
}
