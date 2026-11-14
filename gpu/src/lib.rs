//! A machine's GPU: one device model both kernels' graphics and compute APIs lower onto, backed by the host's GPU or
//! the CPU.
//!
//! The device is small and explicit, the shape Vulkan, Direct3D 12, CUDA and Metal share underneath: memory, images
//! and samplers, programs (pipelines: a compute entry point, or a vertex and a fragment one with their fixed-function
//! state), command lists (dispatches, render passes and draws, copies, fills) submitted in order to one queue, and
//! 64-bit timeline fences that submissions wait on and signal. Programs come in as gpu's IR from any frontend
//! (`Device::pipeline_spirv` reads SPIR-V), and each backend compiles them its own way: Metal loads them as AIR
//! libraries on the Mac, the CPU device interprets them.
//!
//! Built so far: compute on both devices; draws on the CPU device (Metal's are still to come). Presenting is the
//! caller's: an image copied to memory and read.
//!
//! ```no_run
//! let mut device = gpu::Device::open(gpu::Choice::Auto).unwrap().unwrap();
//! println!("gpu: {}", device.info().describe());
//! ```

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

pub use gpu_ir as ir;

mod graphics;
#[cfg(target_os = "macos")]
mod macos;
mod software;

pub use graphics::*;

/// Why the device refused a call, or a program couldn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error { message: message.into() })
}

/// A SPIR-V module's compute, vertex and fragment entry points, as IR: for [`Device::pipeline`] and
/// [`Device::graphics`].
pub fn spirv(bytes: &[u8]) -> Result<ir::Module, Error> {
    Ok(gpu_frontend::spirv::read(bytes)?)
}

/// An image's largest side.
pub const MAX_IMAGE: u32 = 16384;

/// Whether two stages' declarations of one binding agree: the same kind (a buffer's writability may differ).
fn same_kind(a: ir::BindingKind, b: ir::BindingKind) -> bool {
    a == b || (a.is_buffer() && b.is_buffer() && a != ir::BindingKind::Uniform && b != ir::BindingKind::Uniform)
}

impl From<gpu_frontend::Error> for Error {
    fn from(e: gpu_frontend::Error) -> Self {
        Error { message: e.message }
    }
}

impl From<gpu_backend::Error> for Error {
    fn from(e: gpu_backend::Error) -> Self {
        Error { message: e.message }
    }
}

impl From<gpu_ir::Error> for Error {
    fn from(e: gpu_ir::Error) -> Self {
        Error { message: e.message }
    }
}

/// Which device a machine gets (`--gpu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The host's GPU if it has one, else the CPU device, and [`Info::why`] says why.
    Auto,
    /// The host's GPU, or an error.
    Host,
    /// The CPU device, the same on every host.
    Software,
    /// No device: programs take their own fallback.
    None,
}

/// What kind of device it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Host,
    Software,
}

/// What a device is and can do.
#[derive(Debug, Clone)]
pub struct Info {
    pub kind: Kind,
    /// The host's name for it (`Apple M3 Pro`), or `software`.
    pub name: String,
    /// The backend driving it: `metal`, `software`.
    pub backend: &'static str,
    /// The subgroup width programs see unless they name their own.
    pub subgroup_size: u32,
    pub max_workgroup_invocations: u32,
    /// Why `Auto` got the CPU device, if it did.
    pub why: Option<String>,
}

impl Info {
    /// One line for the machine to say at start: `software (no Metal device)`, `Apple M3 Pro (metal)`.
    pub fn describe(&self) -> String {
        match (&self.why, self.kind) {
            (Some(why), _) => format!("{} ({why})", self.name),
            (None, Kind::Host) => format!("{} ({})", self.name, self.backend),
            (None, Kind::Software) => self.name.clone(),
        }
    }
}

/// An allocation of device memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Memory(pub(crate) u32);

/// A compiled program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pipeline(pub(crate) u32);

/// A 64-bit timeline: submissions wait for it to reach a value and raise it when they finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fence(pub(crate) u32);

/// Bytes of one allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub memory: Memory,
    pub offset: u64,
    pub size: u64,
}

/// One command in a submission. Commands run in order: each sees everything before it finished.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// The pipeline's program over `groups` workgroups, each binding the range at its index.
    Dispatch { pipeline: Pipeline, bindings: Vec<Range>, groups: [u32; 3] },
    Copy { from: Range, to: Memory, at: u64 },
    /// The range filled with a 32-bit pattern; offset and size are multiples of 4.
    Fill { range: Range, value: u32 },
    /// Starts a render pass: its attachments, all the same size, loaded or cleared. The viewport and scissor start
    /// as the whole attachment.
    BeginRender { colors: Vec<ColorAttachment>, depth: Option<DepthAttachment> },
    SetViewport(Viewport),
    SetScissor(Rect),
    /// Triangles, in a render pass, with a graphics pipeline made for its attachments' formats.
    Draw(Draw),
    EndRender,
    /// Texels from memory, `row_pixels` to a row (0: the region's width), tightly packed in the image's format.
    CopyBufferToImage { from: Range, row_pixels: u32, to: Image, region: Rect },
    CopyImageToBuffer { from: Image, region: Rect, to: Range, row_pixels: u32 },
}

/// A list of commands for the queue, with the fences it waits for first and signals when it's done.
#[derive(Debug, Clone, Copy)]
pub struct Submit<'a> {
    pub commands: &'a [Command],
    pub wait: &'a [(Fence, u64)],
    pub signal: &'a [(Fence, u64)],
}

/// What a backend implements: the device model, with every argument already checked by [`Device`].
pub(crate) trait Backend {
    fn info(&self) -> &Info;
    fn alloc(&mut self, size: u64) -> Result<Memory, Error>;
    fn free(&mut self, memory: Memory);
    fn write(&mut self, memory: Memory, offset: u64, data: &[u8]) -> Result<(), Error>;
    fn read(&mut self, memory: Memory, offset: u64, out: &mut [u8]) -> Result<(), Error>;
    fn pipeline(&mut self, entry: &ir::Entry) -> Result<Pipeline, Error>;
    fn image(&mut self, _desc: &ImageDesc) -> Result<Image, Error> {
        err(format!("images aren't on {} yet", self.info().backend))
    }
    fn free_image(&mut self, _image: Image) {}
    fn sampler(&mut self, _desc: &SamplerDesc) -> Result<Sampler, Error> {
        err(format!("samplers aren't on {} yet", self.info().backend))
    }
    fn graphics(&mut self, _desc: &GraphicsDesc<'_>) -> Result<Pipeline, Error> {
        err(format!("draws aren't on {} yet", self.info().backend))
    }
    fn fence(&mut self) -> Result<Fence, Error>;
    fn submit(&mut self, submit: Submit<'_>) -> Result<(), Error>;
    /// Raises a fence from the host.
    fn signal(&mut self, fence: Fence, value: u64) -> Result<(), Error>;
    fn fence_value(&mut self, fence: Fence) -> Result<u64, Error>;
    /// Whether the fence reached the value in time; an error if work on the queue failed.
    fn wait(&mut self, fence: Fence, value: u64, timeout: Duration) -> Result<bool, Error>;
}

/// A device, and what the facade checks before its backend sees a call.
pub struct Device {
    backend: Box<dyn Backend>,
    sizes: HashMap<Memory, u64>,
    programs: HashMap<Pipeline, Program>,
    images: HashMap<Image, ImageDesc>,
    samplers: Vec<Sampler>,
    fences: Vec<Fence>,
}

/// What the facade keeps of a pipeline to check the commands that use it.
enum Program {
    Compute { bindings: usize },
    Graphics {
        /// Every (set, binding) either stage uses, and what it is.
        bindings: Vec<(u32, u32, ir::BindingKind)>,
        buffers: usize,
        colors: Vec<Format>,
        depth: Option<Format>,
    },
}

/// What a submission's commands are inside, as the facade walks them.
struct Pass {
    colors: Vec<Format>,
    depth: Option<Format>,
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device").field("info", self.backend.info()).finish()
    }
}

/// The host's own GPU backend, or why there isn't one.
fn host() -> Result<Box<dyn Backend>, String> {
    #[cfg(target_os = "macos")]
    {
        macos::Metal::open().map(|m| Box::new(m) as Box<dyn Backend>)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("no host GPU backend on this host yet".into())
    }
}

impl Device {
    /// The device a machine gets; `None` for [`Choice::None`].
    pub fn open(choice: Choice) -> Result<Option<Device>, Error> {
        let backend: Box<dyn Backend> = match choice {
            Choice::None => return Ok(None),
            Choice::Software => Box::new(software::Software::new(None)),
            Choice::Host => host().map_err(|why| Error { message: format!("no host GPU: {why}") })?,
            Choice::Auto => match host() {
                Ok(b) => b,
                Err(why) => Box::new(software::Software::new(Some(why))),
            },
        };
        Ok(Some(Device {
            backend,
            sizes: HashMap::new(),
            programs: HashMap::new(),
            images: HashMap::new(),
            samplers: Vec::new(),
            fences: Vec::new(),
        }))
    }

    pub fn info(&self) -> &Info {
        self.backend.info()
    }

    pub fn alloc(&mut self, size: u64) -> Result<Memory, Error> {
        if size == 0 {
            return err("an allocation of 0 bytes");
        }
        let m = self.backend.alloc(size)?;
        self.sizes.insert(m, size);
        Ok(m)
    }

    pub fn free(&mut self, memory: Memory) {
        if self.sizes.remove(&memory).is_some() {
            self.backend.free(memory);
        }
    }

    fn check(&self, r: Range) -> Result<(), Error> {
        let Some(&size) = self.sizes.get(&r.memory) else { return err(format!("{:?} isn't allocated", r.memory)) };
        if r.offset.checked_add(r.size).is_none_or(|end| end > size) {
            return err(format!("{} bytes at {} are past {:?}'s {size}", r.size, r.offset, r.memory));
        }
        Ok(())
    }

    /// Copies bytes into memory no submission in flight is using.
    pub fn write(&mut self, memory: Memory, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.check(Range { memory, offset, size: data.len() as u64 })?;
        self.backend.write(memory, offset, data)
    }

    pub fn read(&mut self, memory: Memory, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        self.check(Range { memory, offset, size: out.len() as u64 })?;
        self.backend.read(memory, offset, out)
    }

    /// A compute pipeline from an IR entry point, validated first.
    pub fn pipeline(&mut self, entry: &ir::Entry) -> Result<Pipeline, Error> {
        ir::validate(entry)?;
        if entry.stage != ir::Stage::Compute {
            return err(format!("{} is a {:?} shader, not a compute kernel", entry.name, entry.stage));
        }
        let n = entry.invocations_per_group();
        let max = self.info().max_workgroup_invocations;
        if n > max {
            return err(format!("{}: {n} invocations a workgroup, past this device's {max}", entry.name));
        }
        let p = self.backend.pipeline(entry)?;
        self.programs.insert(p, Program::Compute { bindings: entry.bindings.len() });
        Ok(p)
    }

    /// A compute pipeline from a SPIR-V module's entry point.
    pub fn pipeline_spirv(&mut self, spirv: &[u8], name: &str) -> Result<Pipeline, Error> {
        let module = gpu_frontend::spirv::read(spirv)?;
        let Some(entry) = module.entries.iter().find(|e| e.name == name && e.stage == ir::Stage::Compute) else {
            return err(format!("the module has no compute entry point {name:?}"));
        };
        self.pipeline(entry)
    }

    pub fn image(&mut self, desc: ImageDesc) -> Result<Image, Error> {
        if desc.width == 0 || desc.height == 0 || desc.width > MAX_IMAGE || desc.height > MAX_IMAGE {
            return err(format!("an image of {}×{}: each side is 1 to {MAX_IMAGE}", desc.width, desc.height));
        }
        let image = self.backend.image(&desc)?;
        self.images.insert(image, desc);
        Ok(image)
    }

    pub fn free_image(&mut self, image: Image) {
        if self.images.remove(&image).is_some() {
            self.backend.free_image(image);
        }
    }

    pub fn image_desc(&self, image: Image) -> Option<ImageDesc> {
        self.images.get(&image).copied()
    }

    pub fn sampler(&mut self, desc: SamplerDesc) -> Result<Sampler, Error> {
        let s = self.backend.sampler(&desc)?;
        self.samplers.push(s);
        Ok(s)
    }

    /// A graphics pipeline: both stages validated, each output the fragment shader reads written by the vertex
    /// shader with the same type, each vertex input given by an attribute of its type, each colour output an
    /// attachment's.
    pub fn graphics(&mut self, desc: &GraphicsDesc<'_>) -> Result<Pipeline, Error> {
        let (v, f) = (desc.vertex, desc.fragment);
        ir::validate(v)?;
        ir::validate(f)?;
        if v.stage != ir::Stage::Vertex || f.stage != ir::Stage::Fragment {
            return err(format!("a pipeline of a {:?} and a {:?} shader, not a vertex and a fragment one", v.stage, f.stage));
        }
        for (i, input) in f.inputs.iter().enumerate() {
            let written = v.outputs.iter().find(|o| o.slot == input.slot);
            if written.map(|o| o.ty) != Some(input.ty) {
                return err(format!("{}'s input {i} at {:?} ({:?}) isn't {}'s output", f.name, input.slot, input.ty, v.name));
            }
        }
        if !v.outputs.iter().any(|o| o.slot == ir::Slot::Position) {
            return err(format!("{} writes no position", v.name));
        }
        for input in &v.inputs {
            let ir::Slot::Location(location) = input.slot else { continue };
            let Some(a) = desc.attributes.iter().find(|a| a.location == location) else {
                return err(format!("{}'s input at location {location} has no attribute", v.name));
            };
            if a.format.scalar() != input.ty.scalar().unwrap_or(ir::Scalar::F32) {
                return err(format!("location {location}: an attribute of {:?} for a {:?} input", a.format, input.ty));
            }
            if a.buffer as usize >= desc.buffers.len() {
                return err(format!("location {location}'s attribute is in vertex buffer {}, of {}", a.buffer, desc.buffers.len()));
            }
        }
        for output in &f.outputs {
            if let ir::Slot::Location(n) = output.slot
                && n as usize >= desc.colors.len() {
                    return err(format!("{} writes location {n}, and there are {} colour attachments", f.name, desc.colors.len()));
                }
        }
        if desc.colors.iter().any(|c| c.format.is_depth()) || desc.depth_format.is_some_and(|d| !d.is_depth()) {
            return err("a colour attachment of a depth format, or the reverse");
        }
        if desc.depth.is_some() && desc.depth_format.is_none() {
            return err("a depth test with no depth attachment");
        }
        let mut bindings: Vec<(u32, u32, ir::BindingKind)> = Vec::new();
        for b in v.bindings.iter().chain(&f.bindings) {
            match bindings.iter().find(|(s, n, _)| (*s, *n) == (b.set, b.binding)) {
                Some((_, _, kind)) if !same_kind(*kind, b.kind) => {
                    return err(format!("set {} binding {} is {kind:?} in one stage and {:?} in the other", b.set, b.binding, b.kind));
                }
                Some(_) => {}
                None => bindings.push((b.set, b.binding, b.kind)),
            }
        }
        let p = self.backend.graphics(desc)?;
        let colors = desc.colors.iter().map(|c| c.format).collect();
        self.programs.insert(p, Program::Graphics { bindings, buffers: desc.buffers.len(), colors, depth: desc.depth_format });
        Ok(p)
    }

    fn check_image(&self, image: Image) -> Result<ImageDesc, Error> {
        match self.images.get(&image) {
            Some(d) => Ok(*d),
            None => err(format!("{image:?} isn't this device's")),
        }
    }

    /// A copy between memory and an image: the region inside the image, and the bytes inside the range.
    fn check_copy(&self, range: Range, row_pixels: u32, image: Image, region: Rect) -> Result<(), Error> {
        self.check(range)?;
        let desc = self.check_image(image)?;
        if region.width == 0 || region.height == 0 || region.x + region.width > desc.width || region.y + region.height > desc.height {
            return err(format!("{region:?} isn't inside {image:?}'s {}×{}", desc.width, desc.height));
        }
        let row = row_pixels.max(region.width) as u64;
        let needed = (row * (region.height as u64 - 1) + region.width as u64) * desc.format.bytes() as u64;
        if needed > range.size {
            return err(format!("{needed} bytes of texels, and the range has {}", range.size));
        }
        Ok(())
    }

    fn check_draw(&self, d: &Draw, pass: &Pass) -> Result<(), Error> {
        let Some(Program::Graphics { bindings, buffers, colors, depth }) = self.programs.get(&d.pipeline) else {
            return err(format!("{:?} isn't one of this device's graphics pipelines", d.pipeline));
        };
        if *colors != pass.colors || *depth != pass.depth {
            return err(format!("{:?} renders to {colors:?} and {depth:?}, the pass is {:?} and {:?}", d.pipeline, pass.colors, pass.depth));
        }
        if d.vertex_buffers.len() != *buffers {
            return err(format!("{:?} reads {buffers} vertex buffers, the draw gives {}", d.pipeline, d.vertex_buffers.len()));
        }
        for r in &d.vertex_buffers {
            self.check(*r)?;
        }
        for (set, binding, kind) in bindings {
            let Some((_, _, bind)) = d.bindings.iter().find(|(s, b, _)| (*s, *b) == (*set, *binding)) else {
                return err(format!("set {set} binding {binding} isn't bound"));
            };
            let ok = match (kind, bind) {
                (k, Bind::Buffer(r)) if k.is_buffer() => self.check(*r).is_ok(),
                (ir::BindingKind::Image, Bind::Image(i)) => self.check_image(*i).is_ok(),
                (ir::BindingKind::Sampler, Bind::Sampler(s)) => self.samplers.contains(s),
                (ir::BindingKind::Combined, Bind::Combined(i, s)) => self.check_image(*i).is_ok() && self.samplers.contains(s),
                _ => false,
            };
            if !ok {
                return err(format!("set {set} binding {binding} is {kind:?}, and {bind:?} isn't one of this device's"));
            }
        }
        if let Some((range, ty)) = d.index {
            self.check(range)?;
            let size = if ty == IndexType::U16 { 2 } else { 4 };
            let end = (d.first as u64 + d.count as u64) * size;
            if end > range.size {
                return err(format!("indices {} to {} are past the index buffer's {} bytes", d.first, d.first + d.count, range.size));
            }
        }
        Ok(())
    }

    pub fn fence(&mut self) -> Result<Fence, Error> {
        let f = self.backend.fence()?;
        self.fences.push(f);
        Ok(f)
    }

    fn check_fence(&self, f: Fence) -> Result<(), Error> {
        if self.fences.contains(&f) { Ok(()) } else { err(format!("{f:?} isn't this device's")) }
    }

    pub fn submit(&mut self, submit: Submit<'_>) -> Result<(), Error> {
        for (f, _) in submit.wait.iter().chain(submit.signal) {
            self.check_fence(*f)?;
        }
        let mut pass: Option<Pass> = None;
        for c in submit.commands {
            let rendering = matches!(c, Command::SetViewport(_) | Command::SetScissor(_) | Command::Draw(_) | Command::EndRender);
            if rendering != pass.is_some() && !matches!(c, Command::BeginRender { .. }) {
                return err(format!("{} a render pass: {c:?}", if rendering { "outside" } else { "inside" }));
            }
            match c {
                Command::BeginRender { colors, depth } => {
                    if pass.is_some() {
                        return err("a render pass begun inside another");
                    }
                    let mut size = None;
                    let mut formats = Vec::new();
                    for image in colors.iter().map(|a| a.image).chain(depth.map(|d| d.image)) {
                        let d = self.check_image(image)?;
                        if size.is_some_and(|s| s != (d.width, d.height)) {
                            return err("a render pass's attachments of different sizes");
                        }
                        size = Some((d.width, d.height));
                        formats.push(d.format);
                    }
                    if size.is_none() {
                        return err("a render pass with no attachments");
                    }
                    let depth_format = depth.map(|d| self.images[&d.image].format);
                    let color_formats: Vec<Format> = formats[..colors.len()].to_vec();
                    if color_formats.iter().any(|f| f.is_depth()) || depth_format.is_some_and(|f| !f.is_depth()) {
                        return err("a colour attachment of a depth format, or the reverse");
                    }
                    pass = Some(Pass { colors: color_formats, depth: depth_format });
                }
                Command::EndRender => pass = None,
                Command::SetViewport(v) => {
                    if v.width <= 0.0 || v.height == 0.0 || !(0.0..=1.0).contains(&v.min_depth) || !(0.0..=1.0).contains(&v.max_depth) {
                        return err(format!("a viewport of {v:?}"));
                    }
                }
                Command::SetScissor(_) => {}
                Command::Draw(d) => self.check_draw(d, pass.as_ref().ok_or_else(|| Error { message: "a draw outside a pass".into() })?)?,
                Command::CopyBufferToImage { from, row_pixels, to, region } => self.check_copy(*from, *row_pixels, *to, *region)?,
                Command::CopyImageToBuffer { from, region, to, row_pixels } => self.check_copy(*to, *row_pixels, *from, *region)?,
                Command::Dispatch { pipeline, bindings, groups } => {
                    let Some(Program::Compute { bindings: needs }) = self.programs.get(pipeline) else {
                        return err(format!("{pipeline:?} isn't one of this device's compute pipelines"));
                    };
                    let needs = *needs;
                    if bindings.len() != needs {
                        return err(format!("{pipeline:?} needs {needs} bindings, the dispatch gives {}", bindings.len()));
                    }
                    for b in bindings {
                        self.check(*b)?;
                    }
                    if groups.contains(&0) {
                        return err(format!("a dispatch of {groups:?} workgroups"));
                    }
                }
                Command::Copy { from, to, at } => {
                    self.check(*from)?;
                    self.check(Range { memory: *to, offset: *at, size: from.size })?;
                }
                Command::Fill { range, .. } => {
                    self.check(*range)?;
                    if range.offset % 4 != 0 || range.size % 4 != 0 {
                        return err("a fill's offset and size must be multiples of 4");
                    }
                }
            }
        }
        if pass.is_some() {
            return err("a render pass not ended by the submission's end");
        }
        self.backend.submit(submit)
    }

    pub fn signal(&mut self, fence: Fence, value: u64) -> Result<(), Error> {
        self.check_fence(fence)?;
        self.backend.signal(fence, value)
    }

    pub fn fence_value(&mut self, fence: Fence) -> Result<u64, Error> {
        self.check_fence(fence)?;
        self.backend.fence_value(fence)
    }

    /// Waits for a fence to reach a value: `Ok(false)` if the time ran out first.
    pub fn wait(&mut self, fence: Fence, value: u64, timeout: Duration) -> Result<bool, Error> {
        self.check_fence(fence)?;
        self.backend.wait(fence, value, timeout)
    }
}
