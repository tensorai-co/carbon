//! OpenGL's records: what Carbon's GL vendor library (`libGLX_carbon`, in programs) sends `gpu_api::gl` through the
//! render node's [`GL_SUBMIT_IOCTL`]. They're GL's calls, encoded as they are and run by the host in order: the guest
//! records them and rings at the points it has to wait (a call that answers, a swap, `glFinish`, a context made
//! current), with a [`USE`] first naming the context they're for.
//!
//! Records are [`records`](crate::records)'s (two words, kind and length, then the payload). The doorbell's reply is a
//! status word, [`OK`] or [`FAILED`], then the answers of the records that answer, in order (or what failed, as text).
//! A GL error isn't a failure: it's the context's state, which [`GET_ERROR`] reads.

use crate::{DRM_COMMAND_BASE, DRM_TYPE, IOC_READ, IOC_WRITE, ioc};

/// The GL doorbell: `struct { records, length, reply, reply_capacity }`, as [`SUBMIT_IOCTL`](crate::SUBMIT_IOCTL)'s.
/// It returns how many bytes of reply it wrote.
pub const GL_SUBMIT_IOCTL: u64 = ioc(IOC_READ | IOC_WRITE, DRM_TYPE, DRM_COMMAND_BASE + 2, crate::SUBMIT_SIZE);

/// The reply's first word.
pub const OK: u32 = 0;
pub const FAILED: u32 = 1;

/// A context made: `id`, and the context it shares objects with (`share`, 0 for none).
pub const CONTEXT: u32 = 1;
/// `id`.
pub const FREE_CONTEXT: u32 = 2;
/// The context the records after it are for: `id`.
pub const USE: u32 = 3;
/// The used context bound to a drawable (`id`, 0 to unbind it): the drawable's `width` and `height`, and its
/// config's depth and stencil bits and whether it has alpha. The drawable's default framebuffer is made, or resized,
/// to fit; the first time a context is bound, its viewport and scissor are set to the drawable's size.
pub const BIND: u32 = 4;
/// A drawable gone: `id`.
pub const FREE_DRAWABLE: u32 = 5;
/// Answers the context's error (a word), and clears it.
pub const GET_ERROR: u32 = 6;
/// `pname`: answers how many values, then each as a kind ([`INT`], [`FLOAT`], [`BOOL`]) and 64 bits (an `i64`, an
/// `f64`'s bits, 0 or 1). A name the context doesn't know answers none, and sets `GL_INVALID_ENUM`.
pub const GET: u32 = 7;
/// `cap`: answers 1 or 0 (`glIsEnabled`).
pub const IS_ENABLED: u32 = 8;
/// The drawable's frame, as a window shows it, written to the program: `drawable`, the guest's address, the row's
/// bytes. Its rows go top first, each pixel `B, G, R, A` (or X).
pub const SWAP: u32 = 9;
/// An error the guest found itself (a name `glGetString` hasn't, say): `code`, kept as the context's own.
pub const ERROR: u32 = 10;

/// Four `f32`s' bits.
pub const CLEAR_COLOR: u32 = 16;
/// An `f64`'s bits.
pub const CLEAR_DEPTH: u32 = 17;
/// A word.
pub const CLEAR_STENCIL: u32 = 18;
/// The mask of buffers.
pub const CLEAR: u32 = 19;
/// `x`, `y`, `width`, `height`, as words (`i32`s).
pub const VIEWPORT: u32 = 20;
pub const SCISSOR: u32 = 21;
/// `cap`.
pub const ENABLE: u32 = 22;
pub const DISABLE: u32 = 23;
/// Red, green, blue and alpha, each 0 or 1.
pub const COLOR_MASK: u32 = 24;
/// 0 or 1.
pub const DEPTH_MASK: u32 = 25;
/// `x`, `y`, `width`, `height`, `format`, `type`, then where in the program (64 bits) and the row's bytes there: GL's
/// rows, lowest first.
pub const READ_PIXELS: u32 = 26;

// Objects. Names are the guest's: it hands them out, as `glGen*` and `glCreate*` return them, and the host keeps
// what each names. Shaders and programs share one namespace, as GL has them.

/// A shader made: `name`, its kind (`GL_VERTEX_SHADER`, `GL_FRAGMENT_SHADER`, `GL_COMPUTE_SHADER`).
pub const SHADER: u32 = 32;
/// `name`, then its source (length, bytes).
pub const SHADER_SOURCE: u32 = 33;
/// `name`.
pub const COMPILE: u32 = 34;
/// `name`: a shader's or program's (deleted once nothing uses it, as GL deletes them).
pub const DELETE_SHADER: u32 = 35;
/// `name`, `pname`: answers an `i32`.
pub const GET_SHADER: u32 = 36;
/// `name`: answers its info log (length, bytes).
pub const SHADER_LOG: u32 = 37;
pub const PROGRAM: u32 = 38;
/// `program`, `shader`.
pub const ATTACH: u32 = 39;
pub const DETACH: u32 = 40;
/// `program`, `index`, then the attribute's name (length, bytes).
pub const BIND_ATTRIB: u32 = 41;
pub const LINK: u32 = 42;
/// `program` (0 for none).
pub const USE_PROGRAM: u32 = 43;
pub const DELETE_PROGRAM: u32 = 44;
/// `name`, `pname`: answers an `i32`.
pub const GET_PROGRAM: u32 = 45;
/// `name`: answers its info log (length, bytes).
pub const PROGRAM_LOG: u32 = 46;
/// `program`, then a name (length, bytes): answers the location (`i32`, -1 for none).
pub const UNIFORM_LOCATION: u32 = 47;
pub const ATTRIB_LOCATION: u32 = 48;
/// `program`, `index`: answers its array size, its GL type, and its name (length, bytes); a size of 0 for none.
pub const ACTIVE_UNIFORM: u32 = 49;
pub const ACTIVE_ATTRIB: u32 = 50;
/// The used program's uniform at `location` set: the values' kind ([`FLOAT`], [`INT`] or the unsigned [`UINT`]), the
/// columns and rows of each (1 row for a scalar or vector), how many, whether a matrix is given transposed, and the
/// values' words (length, bytes).
pub const UNIFORM: u32 = 51;
/// `glValidateProgram`: `name`.
pub const VALIDATE: u32 = 52;

/// `target`, `name` (0 to unbind).
pub const BIND_BUFFER: u32 = 64;
/// `target`, `size` (64 bits), `usage`, whether data is given, then the data (length, bytes).
pub const BUFFER_DATA: u32 = 65;
/// `target`, `offset` (64 bits), then the data (length, bytes).
pub const BUFFER_SUB_DATA: u32 = 66;
pub const DELETE_BUFFER: u32 = 67;
/// `name` (0 for the default vertex array).
pub const BIND_VERTEX_ARRAY: u32 = 68;
pub const DELETE_VERTEX_ARRAY: u32 = 69;
/// `index`, `size` (1 to 4, or `GL_BGRA`), `type`, whether it's normalized, whether it's read as integers
/// (`glVertexAttribIPointer`), `stride`, then where (64 bits): an offset in the bound array buffer, or with none bound,
/// the address of the program's own array.
pub const ATTRIB_POINTER: u32 = 70;
/// `index`, on (1) or off (0).
pub const ENABLE_ATTRIB: u32 = 71;
/// `index`, four `f32`s' bits: a generic attribute's value when its array is off.
pub const ATTRIB_VALUE: u32 = 72;
/// `index`, `divisor`.
pub const ATTRIB_DIVISOR: u32 = 73;

/// `unit` (0 for `GL_TEXTURE0`).
pub const ACTIVE_TEXTURE: u32 = 80;
/// `target`, `name`.
pub const BIND_TEXTURE: u32 = 81;
/// `target`, `level`, `internalformat`, `width`, `height`, `format`, `type`, whether texels are given, then the texels,
/// rows tightly packed, lowest first (length, bytes).
pub const TEX_IMAGE: u32 = 82;
/// `target`, `level`, `x`, `y`, `width`, `height`, `format`, `type`, then the texels (length, bytes).
pub const TEX_SUB_IMAGE: u32 = 83;
/// `target`, `pname`, the value's bits, and whether it's a float.
pub const TEX_PARAMETER: u32 = 84;
pub const GENERATE_MIPMAP: u32 = 85;
pub const DELETE_TEXTURE: u32 = 86;

/// `src_rgb`, `dst_rgb`, `src_alpha`, `dst_alpha`.
pub const BLEND_FUNC: u32 = 96;
/// `rgb`, `alpha`.
pub const BLEND_EQUATION: u32 = 97;
pub const DEPTH_FUNC: u32 = 98;
pub const CULL_FACE: u32 = 99;
pub const FRONT_FACE: u32 = 100;

/// `mode`, `first`, `count`, `instances`.
pub const DRAW_ARRAYS: u32 = 112;
/// `mode`, `count`, `type`, `instances`, `base_vertex`, then where the indices are (64 bits): an offset in the bound
/// element buffer, or with none bound, the address of the program's own.
pub const DRAW_ELEMENTS: u32 = 113;

// The compatibility profile's state. Its arrays and current values are generic attributes' (`gl_Vertex` 0,
// `gl_Normal` 2, `gl_Color` 3, `gl_SecondaryColor` 4, `gl_FogCoord` 5, `gl_MultiTexCoord<n>` 8 + n), so the
// guest sends them as those.

/// `mode` (`GL_MODELVIEW`, `GL_PROJECTION`, `GL_TEXTURE`).
pub const MATRIX_MODE: u32 = 128;
/// An operation on the current matrix ([`MATRIX_IDENTITY`] and the rest), then its floats (length, bytes).
pub const MATRIX: u32 = 129;
/// `pname`, four words of its value, and whether they're floats: the active unit's texture environment.
pub const TEX_ENV: u32 = 130;
/// `func`, the reference's bits.
pub const ALPHA_FUNC: u32 = 131;
/// `pname`, four words of its value, and whether they're floats.
pub const FOG: u32 = 132;
pub const SHADE_MODEL: u32 = 133;
/// `glRectf`: x1, y1, x2, y2's bits. Immediate mode: every attribute but the position is its current value, whatever
/// arrays are on.
pub const RECT: u32 = 134;

/// [`MATRIX`]'s operations, and the floats each takes.
pub const MATRIX_IDENTITY: u32 = 0;
/// 16, column-major.
pub const MATRIX_LOAD: u32 = 1;
pub const MATRIX_MULTIPLY: u32 = 2;
pub const MATRIX_PUSH: u32 = 3;
pub const MATRIX_POP: u32 = 4;
/// x, y, z.
pub const MATRIX_TRANSLATE: u32 = 5;
/// The angle in degrees, then the axis.
pub const MATRIX_ROTATE: u32 = 6;
pub const MATRIX_SCALE: u32 = 7;
/// left, right, bottom, top, near, far.
pub const MATRIX_ORTHO: u32 = 8;
pub const MATRIX_FRUSTUM: u32 = 9;

/// The unsigned kind of [`UNIFORM`]'s values.
pub const UINT: u32 = 3;

/// [`GET`]'s kinds of value.
pub const INT: u32 = 0;
pub const FLOAT: u32 = 1;
pub const BOOL: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gl_doorbell_is_the_next_command() {
        assert_eq!(GL_SUBMIT_IOCTL, 0xc020_6442);
    }
}
