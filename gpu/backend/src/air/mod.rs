//! The IR as AIR, the IR Apple's GPU drivers compile, written as the `.metallib` Metal loads with
//! `newLibraryWithData:`, so nothing compiles source at pipeline creation.
//!
//! AIR is LLVM IR in Apple's dialect: an SSA control-flow graph with typed pointers in numbered address spaces,
//! kernels listed in `!air.kernel` with a descriptor per argument, and barriers, SIMD-group operations, atomics and
//! math as calls to `air.*` intrinsics. Everything here was read from what `xcrun metal -S -emit-llvm` makes of the
//! matching MSL, and is checked against Apple's toolchain where it's installed (`gpu/tests/air.rs`).
//!
//! - [`lower`]: gpu's IR onto an AIR module (`module`).
//! - `bitcode`: the module as LLVM bitcode in Apple's wrapper, the `.air` file.
//! - `metallib`: that `.air` as a library of one kernel.
//!
//! A module is written for macOS 13 (AIR 2.5) in MSL 3.0, which every later macOS loads; one that needs a newer
//! language (a memory-only barrier is MSL 3.2's) says so, and its deployment target follows.

mod bitcode;
mod lower;
mod metallib;
mod module;
mod sha256;

use gpu_ir::Entry;

use crate::Error;

pub use lower::{Attribute, FUNCTION, Format, VertexBuffer, VertexInput};

/// One entry point, compiled.
#[derive(Debug, Clone)]
pub struct Library {
    /// The `.metallib`, for `newLibraryWithData:`.
    pub metallib: Vec<u8>,
    /// The kernel's module alone, as a `.air` file, for Apple's tools (`air-opt`, `metal-objdump`).
    pub air: Vec<u8>,
    /// The function's name in the library.
    pub function: String,
    /// A vertex function's first vertex buffer's `[[buffer(n)]]`: the one after its bindings.
    pub vertex_buffers_at: u32,
}

/// A compute kernel or fragment function.
pub fn write(entry: &Entry) -> Result<Library, Error> {
    write_with(entry, None)
}

/// Any entry point; a vertex function with its pipeline's vertex input, which it fetches itself.
pub fn write_with(entry: &Entry, vertex: Option<&VertexInput>) -> Result<Library, Error> {
    let mut m = lower::lower(entry, vertex)?;
    let air = bitcode::encode(&mut m);
    let metallib = metallib::build(FUNCTION, &air, m.os, m.language, m.stage);
    Ok(Library { metallib, air, function: FUNCTION.into(), vertex_buffers_at: entry.bindings.len() as u32 })
}
