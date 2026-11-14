//! The records a guest driver sends the host: what it made, and the work it submits.
//!
//! The driver keeps what it can itself (buffers, descriptor sets, command buffers being recorded), and names memory by
//! the guest's own handle, so the host only learns of memory, pipelines and work. Records collect in the driver and go
//! to the host in one [`SUBMIT_IOCTL`](crate::SUBMIT_IOCTL) at the points it has to wait for the host: a queue
//! submission, and a pipeline's compile (whose errors Vulkan reports when it's made).
//!
//! Each record is a header of two little-endian words, its kind and its payload's length in bytes (a multiple of 4),
//! then the payload. Handles, sizes and offsets are 64-bit, the rest 32-bit; text and bytes are padded to a word.

/// Device memory the guest allocated: `id`, `size`, and where the guest's copy is in the program (`guest`), which the
/// host reads before work that uses it and writes after.
pub const MEMORY: u32 = 1;
/// `id`.
pub const FREE_MEMORY: u32 = 2;
/// A compute pipeline: `id`, the entry point's name (length, bytes) and the SPIR-V module (length in bytes, words).
pub const PIPELINE: u32 = 3;
/// `id`.
pub const FREE_PIPELINE: u32 = 4;
/// Work for the queue: command records, run in order before the ioctl returns.
pub const SUBMIT: u32 = 5;
/// An image bound to memory (or, `memory` 0, one the driver keeps, a swapchain's): `id`, `memory`, `offset`, then
/// Vulkan's `format`, `width`, `height`, and whether it's linear (its texels in its memory, tightly packed, which a
/// submission that samples it reads first).
pub const IMAGE: u32 = 6;
/// `id`.
pub const FREE_IMAGE: u32 = 7;
/// A sampler: `id`, then Vulkan's magnification and minification filters and u and v address modes.
pub const SAMPLER: u32 = 8;
/// `id`.
pub const FREE_SAMPLER: u32 = 9;
/// A graphics pipeline: `id`; the vertex and fragment entry points (name, SPIR-V, as [`PIPELINE`]'s); the vertex
/// buffers (count, then each stride and whether it steps per instance) and attributes (count, then each location,
/// buffer, offset and Vulkan format); Vulkan's topology, cull mode and front face; depth (test on, compare op, write
/// on, attachment format or 0); and the colour attachments (count, then each format, blend on, the colour's and
/// alpha's source factor, destination factor and op, and the write mask).
pub const GRAPHICS: u32 = 10;

/// A dispatch: `pipeline`, three group counts, the number of buffers bound and of push-constant bytes, then each
/// buffer (`set`, `binding`, `memory`, `offset`, `size`) and the push constants' bytes.
pub const DISPATCH: u32 = 16;
/// `from` memory and offset, `to` memory and offset, `size`.
pub const COPY: u32 = 17;
/// `memory`, `offset`, `size`, the 32-bit pattern.
pub const FILL: u32 = 18;
/// `memory`, `offset`, then the bytes (length, bytes): `vkCmdUpdateBuffer`'s, written in the queue's order.
pub const UPDATE: u32 = 19;
/// A render pass begun: the colour attachments (count, then each image, Vulkan's load op and four `f32` clear
/// values as bits), then the depth attachment's image (0 for none), load op and clear value.
pub const BEGIN_RENDER: u32 = 20;
/// Four `f32`s as bits, `x`, `y`, `width`, `height`, then the depth range's two.
pub const VIEWPORT: u32 = 21;
/// `x`, `y`, `width`, `height`, as 32-bit words.
pub const SCISSOR: u32 = 22;
/// A draw: `pipeline`; `first`, `count`, `vertex_offset`, `first_instance`, `instances`; the buffers bound and the
/// push constants as [`DISPATCH`]'s; the images bound (count, then each `set`, `binding`, `image` and `sampler`, either
/// 0 where it's a sampler or image alone); the vertex buffers (count, then each memory, offset, size); and the index
/// buffer (memory, 0 for none, offset, size, Vulkan's index type).
pub const DRAW: u32 = 23;
pub const END_RENDER: u32 = 24;
/// `memory`, `offset`, the row's texels (0: the region's width), `image`, the region's `x`, `y`, `width`, `height`.
pub const COPY_TO_IMAGE: u32 = 25;
/// `image`, the region's `x`, `y`, `width`, `height`, `memory`, `offset`, the row's texels: what a present reads.
pub const COPY_FROM_IMAGE: u32 = 26;

/// An image or sampler bound to a draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundImage {
    pub set: u32,
    pub binding: u32,
    pub image: u64,
    pub sampler: u64,
}

pub fn put_bound_image(out: &mut impl Extend<u8>, b: &BoundImage) {
    put_u32(out, b.set);
    put_u32(out, b.binding);
    put_u64(out, b.image);
    put_u64(out, b.sampler);
}

/// A buffer bound to a dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    pub set: u32,
    pub binding: u32,
    pub memory: u64,
    pub offset: u64,
    pub size: u64,
}

/// Writes records into anything bytes can be added to (a `Vec<u8>`).
pub fn put_u32(out: &mut impl Extend<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}

pub fn put_u64(out: &mut impl Extend<u8>, v: u64) {
    out.extend(v.to_le_bytes());
}

/// Bytes with their length first, padded to a word.
pub fn put_bytes(out: &mut impl Extend<u8>, bytes: &[u8]) {
    put_u32(out, bytes.len() as u32);
    out.extend(bytes.iter().copied());
    out.extend(core::iter::repeat_n(0, bytes.len().next_multiple_of(4) - bytes.len()));
}

/// A record's header; its payload's length is known once it's written, so `record` takes the payload's words.
pub fn record(out: &mut impl Extend<u8>, kind: u32, payload: &[u8]) {
    put_u32(out, kind);
    put_u32(out, payload.len().next_multiple_of(4) as u32);
    out.extend(payload.iter().copied());
    out.extend(core::iter::repeat_n(0, payload.len().next_multiple_of(4) - payload.len()));
}

/// Reads records, and the fields in one.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

/// What's wrong with records that don't read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if n > self.bytes.len() {
            return Err(Malformed);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    pub fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| Malformed)?))
    }

    pub fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| Malformed)?))
    }

    /// Bytes [`put_bytes`] wrote.
    pub fn bytes(&mut self) -> Result<&'a [u8], Malformed> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        self.take(n.next_multiple_of(4) - n)?;
        Ok(b)
    }

    /// The next record: its kind, and a reader of its payload.
    pub fn record(&mut self) -> Result<(u32, Reader<'a>), Malformed> {
        let kind = self.u32()?;
        let n = self.u32()? as usize;
        Ok((kind, Reader { bytes: self.take(n)? }))
    }

    pub fn f32(&mut self) -> Result<f32, Malformed> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn bound_image(&mut self) -> Result<BoundImage, Malformed> {
        Ok(BoundImage { set: self.u32()?, binding: self.u32()?, image: self.u64()?, sampler: self.u64()? })
    }

    pub fn bound(&mut self) -> Result<Bound, Malformed> {
        Ok(Bound { set: self.u32()?, binding: self.u32()?, memory: self.u64()?, offset: self.u64()?, size: self.u64()? })
    }
}

pub fn put_bound(out: &mut impl Extend<u8>, b: &Bound) {
    put_u32(out, b.set);
    put_u32(out, b.binding);
    put_u64(out, b.memory);
    put_u64(out, b.offset);
    put_u64(out, b.size);
}
