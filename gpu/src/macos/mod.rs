//! The Mac's GPU through Metal: programs loaded as AIR libraries (`gpu_backend::air`, written here, never compiled from
//! source), memory in shared buffers (on Apple silicon the GPU and CPU share memory, so writes and reads are copies
//! into and out of the buffer's own pages), one command queue, and fences as `MTLSharedEvent`s.
//!
//! Each submission is one command buffer: its waits encoded first, then a compute or blit encoder per command (a
//! render encoder for each render pass), then its signals. Metal tracks the hazards between commands on the same
//! buffers and textures, so commands run in order without barriers of ours. Draws are in `draw`.

mod draw;
pub(crate) mod objc;

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use gpu_backend::air;
use gpu_ir::{BinOp, Binding, BindingKind, Builtin, Entry, Op, Scalar, Space, Stmt, Type};

use crate::{Backend, Command, Error, Fence, GraphicsDesc, Image, ImageDesc, Info, Kind, Memory, Pipeline, Sampler, SamplerDesc, Submit, err};
use objc::{Id, error_text, msg, ns_string, pooled, release, retain, rust_string};

#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    fn MTLCreateSystemDefaultDevice() -> Id;
}

// libdispatch, part of libSystem: a library's bytes are handed to Metal as dispatch data.
unsafe extern "C" {
    fn dispatch_data_create(buffer: *const c_void, size: usize, queue: *mut c_void, destructor: *const c_void) -> Id;
    fn dispatch_release(object: Id);
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Size {
    width: usize,
    height: usize,
    depth: usize,
}

/// `MTLResourceStorageModeShared`: one copy, seen by both the CPU and the GPU.
const STORAGE_SHARED: usize = 0;
/// `MTLCommandBufferStatusCompleted` and `...Error`.
const STATUS_COMPLETED: usize = 4;
const STATUS_ERROR: usize = 5;
/// Apple GPUs' subgroup (SIMD group) width and workgroup limit.
const SIMD_WIDTH: u32 = 32;
const MAX_THREADS: u32 = 1024;

/// The threads in each of a fill's workgroups.
const FILL_GROUP: u32 = 64;

/// Fills a buffer with a 32-bit pattern (Metal's own fill takes a byte): `b0[gid.x * 4] = *b1`, the pattern being
/// binding 1. It's dispatched as exactly one invocation per word.
fn fill_program() -> Entry {
    let mut e = Entry::new("carbon_fill", [FILL_GROUP, 1, 1]);
    e.bindings = vec![
        Binding { set: 0, binding: 0, kind: BindingKind::Storage { writable: true } },
        Binding { set: 0, binding: 1, kind: BindingKind::Uniform },
    ];
    let [gid, x, four, off, words, at, value, pattern] = [
        Type::UVEC3,
        Type::U32,
        Type::U32,
        Type::U32,
        Type::Ptr(Space::Device),
        Type::Ptr(Space::Device),
        Type::Ptr(Space::Device),
        Type::U32,
    ]
    .map(|t| e.value(t));
    e.body = vec![
        Stmt::Let(gid, Op::Builtin(Builtin::GlobalId)),
        Stmt::Let(x, Op::Extract(gid, 0)),
        Stmt::Let(four, Op::Const(Scalar::U32, 4)),
        Stmt::Let(off, Op::Binary(BinOp::Mul, x, four)),
        Stmt::Let(words, Op::Binding(0)),
        Stmt::Let(at, Op::Offset(words, off)),
        Stmt::Let(value, Op::Binding(1)),
        Stmt::Let(pattern, Op::Load(value, Type::U32)),
        Stmt::Store(at, pattern),
    ];
    e
}

/// A compute pipeline state and its workgroup, or a graphics pipeline.
struct Program {
    state: Id,
    workgroup: [u32; 3],
    graphics: Option<draw::Graphics>,
}

pub(crate) struct Metal {
    info: Info,
    device: Id,
    queue: Id,
    buffers: HashMap<Memory, (Id, u64)>,
    next_memory: u32,
    pipelines: Vec<Program>,
    textures: HashMap<Image, draw::Texture>,
    next_image: u32,
    samplers: Vec<Id>,
    fill: Option<Id>,
    fences: Vec<Id>,
    /// Command buffers committed and not yet seen finished, kept to report a failure.
    inflight: Vec<Id>,
}

impl Drop for Metal {
    fn drop(&mut self) {
        for cb in self.inflight.drain(..) {
            msg!((); cb, "waitUntilCompleted");
            release(cb);
        }
        for (b, _) in self.buffers.values() {
            release(*b);
        }
        for p in &self.pipelines {
            release(p.state);
        }
        self.pipelines.clear();
        for t in self.textures.values() {
            release(t.texture);
        }
        for s in &self.samplers {
            release(*s);
        }
        if let Some(f) = self.fill {
            release(f);
        }
        for e in &self.fences {
            release(*e);
        }
        release(self.queue);
        release(self.device);
    }
}

impl Metal {
    pub(crate) fn open() -> Result<Metal, String> {
        // SAFETY: Metal's own constructor; nil when the Mac has no Metal device.
        let device = unsafe { MTLCreateSystemDefaultDevice() };
        if device.is_null() {
            return Err("no Metal device".into());
        }
        let name = pooled(|| rust_string(msg!(Id; device, "name")));
        let queue = msg!(Id; device, "newCommandQueue");
        if queue.is_null() {
            release(device);
            return Err("Metal gave no command queue".into());
        }
        Ok(Metal {
            info: Info {
                kind: Kind::Host,
                name,
                backend: "metal",
                subgroup_size: SIMD_WIDTH,
                max_workgroup_invocations: MAX_THREADS,
                why: None,
            },
            device,
            queue,
            buffers: HashMap::new(),
            next_memory: 0,
            pipelines: Vec::new(),
            textures: HashMap::new(),
            next_image: 0,
            samplers: Vec::new(),
            fill: None,
            fences: Vec::new(),
            inflight: Vec::new(),
        })
    }

    /// A compute pipeline state for an entry point: its AIR library, loaded. The library says its math is IEEE's
    /// (`air.compile.fast_math_disable`), so there are no compile options to give.
    fn compile(&self, entry: &Entry) -> Result<Id, Error> {
        let lib = air::write(entry).map_err(|e| Error { message: format!("{}: {e}", entry.name) })?;
        let f = self.load(&entry.name, &lib)?;
        pooled(|| {
            let mut error: Id = std::ptr::null_mut();
            let state = msg!(Id; self.device, "newComputePipelineStateWithFunction:error:", f => Id, &mut error => *mut Id);
            release(f);
            if state.is_null() {
                return err(format!("Metal couldn't make a pipeline: {}", error_text(error)));
            }
            Ok(state)
        })
    }

    /// The function in an AIR library, loaded: retained, for the caller to release.
    fn load(&self, name: &str, lib: &air::Library) -> Result<Id, Error> {
        let function = lib.function.as_str();
        pooled(|| {
            // SAFETY: the bytes are valid for the call; the default destructor (null) has dispatch copy them.
            let data = unsafe {
                dispatch_data_create(lib.metallib.as_ptr().cast(), lib.metallib.len(), std::ptr::null_mut(), std::ptr::null())
            };
            if data.is_null() {
                return err("dispatch gave no data for the library");
            }
            let mut error: Id = std::ptr::null_mut();
            let library = msg!(Id; self.device, "newLibraryWithData:error:", data => Id, &mut error => *mut Id);
            // SAFETY: made above, and Metal keeps what it needs of it.
            unsafe { dispatch_release(data) };
            if library.is_null() {
                return err(format!("Metal couldn't load {name}'s library: {}", error_text(error)));
            }
            let f = msg!(Id; library, "newFunctionWithName:", ns_string(function) => Id);
            release(library);
            if f.is_null() {
                return err(format!("the compiled library has no function {function}"));
            }
            Ok(f)
        })
    }

    fn buffer(&self, m: Memory) -> Result<(Id, u64), Error> {
        match self.buffers.get(&m) {
            Some(b) => Ok(*b),
            None => err(format!("{m:?} isn't allocated")),
        }
    }

    fn contents(&self, m: Memory, offset: u64, len: usize) -> Result<*mut u8, Error> {
        let (buffer, size) = self.buffer(m)?;
        if offset.checked_add(len as u64).is_none_or(|end| end > size) {
            return err("past the buffer's end");
        }
        let base = msg!(*mut c_void; buffer, "contents") as *mut u8;
        if base.is_null() {
            return err("the buffer has no CPU-visible contents");
        }
        // SAFETY: inside the buffer, which is `size` bytes long, as just checked.
        Ok(unsafe { base.add(offset as usize) })
    }

    /// Forgets command buffers that finished; the first one that failed is an error.
    fn reap(&mut self) -> Result<(), Error> {
        let mut failure = None;
        self.inflight.retain(|&cb| {
            let status = msg!(usize; cb, "status");
            if status == STATUS_ERROR && failure.is_none() {
                failure = Some(error_text(msg!(Id; cb, "error")));
            }
            if status >= STATUS_COMPLETED {
                release(cb);
                false
            } else {
                true
            }
        });
        match failure {
            Some(why) => err(format!("work on the GPU failed: {why}")),
            None => Ok(()),
        }
    }

    fn fill_state(&mut self) -> Result<Id, Error> {
        if let Some(f) = self.fill {
            return Ok(f);
        }
        let f = self.compile(&fill_program())?;
        self.fill = Some(f);
        Ok(f)
    }
}

impl Backend for Metal {
    fn info(&self) -> &Info {
        &self.info
    }

    fn alloc(&mut self, size: u64) -> Result<Memory, Error> {
        let buffer = msg!(Id; self.device, "newBufferWithLength:options:", size as usize => usize, STORAGE_SHARED => usize);
        if buffer.is_null() {
            return err(format!("Metal couldn't allocate {size} bytes"));
        }
        let m = Memory(self.next_memory);
        self.next_memory += 1;
        self.buffers.insert(m, (buffer, size));
        Ok(m)
    }

    fn free(&mut self, memory: Memory) {
        if let Some((b, _)) = self.buffers.remove(&memory) {
            release(b);
        }
    }

    fn write(&mut self, memory: Memory, offset: u64, data: &[u8]) -> Result<(), Error> {
        let at = self.contents(memory, offset, data.len())?;
        // SAFETY: `at` has `data.len()` bytes of the buffer behind it, and no submission in flight uses them (the
        // caller's promise); the two don't overlap.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), at, data.len()) };
        Ok(())
    }

    fn read(&mut self, memory: Memory, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        let at = self.contents(memory, offset, out.len())?;
        // SAFETY: as for `write`, the other way.
        unsafe { std::ptr::copy_nonoverlapping(at, out.as_mut_ptr(), out.len()) };
        Ok(())
    }

    fn pipeline(&mut self, entry: &Entry) -> Result<Pipeline, Error> {
        let state = self.compile(entry).inspect_err(|_e| {
            if std::env::var_os("CARBON_TRACE").is_some() {
                eprintln!("gpu: {}'s IR:\n{entry}", entry.name);
            }
        })?;
        let max = msg!(usize; state, "maxTotalThreadsPerThreadgroup");
        if (entry.invocations_per_group() as usize) > max {
            release(state);
            return err(format!("{}: {} invocations a workgroup, past this pipeline's {max}", entry.name, entry.invocations_per_group()));
        }
        let width = msg!(usize; state, "threadExecutionWidth");
        if let Some(want) = entry.subgroup_size
            && want as usize != width {
                release(state);
                return err(format!("{} wants {want}-wide subgroups; this GPU's are {width}", entry.name));
            }
        self.pipelines.push(Program { state, workgroup: entry.workgroup_size, graphics: None });
        Ok(Pipeline(self.pipelines.len() as u32 - 1))
    }

    fn image(&mut self, desc: &ImageDesc) -> Result<Image, Error> {
        self.make_image(desc)
    }

    fn free_image(&mut self, image: Image) {
        self.drop_image(image);
    }

    fn sampler(&mut self, desc: &SamplerDesc) -> Result<Sampler, Error> {
        self.make_sampler(desc)
    }

    fn graphics(&mut self, desc: &GraphicsDesc<'_>) -> Result<Pipeline, Error> {
        let g = self.make_graphics(desc).inspect_err(|_| {
            if std::env::var_os("CARBON_TRACE").is_some() {
                eprintln!("gpu: {}'s IR:\n{}\ngpu: {}'s IR:\n{}", desc.vertex.name, desc.vertex, desc.fragment.name, desc.fragment);
            }
        })?;
        self.pipelines.push(Program { state: std::ptr::null_mut(), workgroup: [1, 1, 1], graphics: Some(g) });
        Ok(Pipeline(self.pipelines.len() as u32 - 1))
    }

    fn fence(&mut self) -> Result<Fence, Error> {
        let event = msg!(Id; self.device, "newSharedEvent");
        if event.is_null() {
            return err("Metal gave no shared event");
        }
        self.fences.push(event);
        Ok(Fence(self.fences.len() as u32 - 1))
    }

    fn submit(&mut self, s: Submit<'_>) -> Result<(), Error> {
        self.reap()?;
        let fill = if s.commands.iter().any(|c| matches!(c, Command::Fill { .. })) { Some(self.fill_state()?) } else { None };
        // Buffers made for this submission's draws (a fan's indices), let go once it's committed.
        let mut made: Vec<Id> = Vec::new();
        let mut render: Option<Id> = None;
        let cb = pooled(|| -> Result<Id, Error> {
            let cb = msg!(Id; self.queue, "commandBuffer");
            if cb.is_null() {
                return err("Metal gave no command buffer");
            }
            for (f, v) in s.wait {
                msg!((); cb, "encodeWaitForEvent:value:", self.fences[f.0 as usize] => Id, *v => u64);
            }
            for c in s.commands {
                match c {
                    Command::Dispatch { pipeline, bindings, groups } => {
                        let enc = msg!(Id; cb, "computeCommandEncoder");
                        let state = self.pipelines[pipeline.0 as usize].state;
                        msg!((); enc, "setComputePipelineState:", state => Id);
                        for (i, b) in bindings.iter().enumerate() {
                            let (buffer, _) = self.buffer(b.memory)?;
                            msg!((); enc, "setBuffer:offset:atIndex:", buffer => Id, b.offset as usize => usize, i => usize);
                        }
                        let [x, y, z] = self.pipelines[pipeline.0 as usize].workgroup;
                        let threads = Size { width: x as usize, height: y as usize, depth: z as usize };
                        let g = Size { width: groups[0] as usize, height: groups[1] as usize, depth: groups[2] as usize };
                        msg!((); enc, "dispatchThreadgroups:threadsPerThreadgroup:", g => Size, threads => Size);
                        msg!((); enc, "endEncoding");
                    }
                    Command::Copy { from, to, at } => {
                        let (src, _) = self.buffer(from.memory)?;
                        let (dst, _) = self.buffer(*to)?;
                        let enc = msg!(Id; cb, "blitCommandEncoder");
                        msg!((); enc, "copyFromBuffer:sourceOffset:toBuffer:destinationOffset:size:",
                            src => Id, from.offset as usize => usize, dst => Id, *at as usize => usize, from.size as usize => usize);
                        msg!((); enc, "endEncoding");
                    }
                    Command::Fill { range, value } => {
                        if range.size == 0 {
                            continue;
                        }
                        let (buffer, _) = self.buffer(range.memory)?;
                        let enc = msg!(Id; cb, "computeCommandEncoder");
                        msg!((); enc, "setComputePipelineState:", fill.unwrap_or(std::ptr::null_mut()) => Id);
                        msg!((); enc, "setBuffer:offset:atIndex:", buffer => Id, range.offset as usize => usize, 0usize => usize);
                        msg!((); enc, "setBytes:length:atIndex:", value as *const u32 as *const c_void => *const c_void, 4usize => usize, 1usize => usize);
                        let words = Size { width: (range.size / 4) as usize, height: 1, depth: 1 };
                        let group = Size { width: FILL_GROUP as usize, height: 1, depth: 1 };
                        msg!((); enc, "dispatchThreads:threadsPerThreadgroup:", words => Size, group => Size);
                        msg!((); enc, "endEncoding");
                    }
                    Command::BeginRender { colors, depth } => render = Some(self.begin_render(cb, colors, *depth)?),
                    Command::SetViewport(v) => Metal::viewport(render.unwrap_or(std::ptr::null_mut()), v),
                    Command::SetScissor(r) => Metal::scissor(render.unwrap_or(std::ptr::null_mut()), r),
                    Command::Draw(d) => {
                        let Some(g) = self.pipelines.get(d.pipeline.0 as usize).and_then(|p| p.graphics.as_ref()) else {
                            return err(format!("{:?} isn't a graphics pipeline", d.pipeline));
                        };
                        self.draw(render.unwrap_or(std::ptr::null_mut()), g, d, &mut made)?;
                    }
                    Command::EndRender => {
                        if let Some(enc) = render.take() {
                            msg!((); enc, "endEncoding");
                        }
                    }
                    Command::CopyBufferToImage { from, row_pixels, to, region } => self.copy_to_texture(cb, *from, *row_pixels, *to, *region)?,
                    Command::CopyImageToBuffer { from, region, to, row_pixels } => self.copy_from_texture(cb, *from, *region, *to, *row_pixels)?,
                }
            }
            for (f, v) in s.signal {
                msg!((); cb, "encodeSignalEvent:value:", self.fences[f.0 as usize] => Id, *v => u64);
            }
            msg!((); cb, "commit");
            Ok(retain(cb))
        })?;
        // Metal retains what a committed command buffer uses.
        for b in made {
            release(b);
        }
        self.inflight.push(cb);
        Ok(())
    }

    fn signal(&mut self, fence: Fence, value: u64) -> Result<(), Error> {
        msg!((); self.fences[fence.0 as usize], "setSignaledValue:", value => u64);
        Ok(())
    }

    fn fence_value(&mut self, fence: Fence) -> Result<u64, Error> {
        Ok(msg!(u64; self.fences[fence.0 as usize], "signaledValue"))
    }

    fn wait(&mut self, fence: Fence, value: u64, timeout: Duration) -> Result<bool, Error> {
        let event = self.fences[fence.0 as usize];
        let start = Instant::now();
        let mut nap = Duration::from_micros(20);
        loop {
            if msg!(u64; event, "signaledValue") >= value {
                self.reap()?;
                return Ok(true);
            }
            self.reap()?;
            if start.elapsed() >= timeout {
                return Ok(false);
            }
            std::thread::sleep(nap);
            nap = (nap * 2).min(Duration::from_millis(1));
        }
    }
}
