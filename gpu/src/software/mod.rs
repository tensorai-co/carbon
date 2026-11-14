//! The CPU device: device memory in host memory at addresses of its own, programs run by `gpu-backend`'s interpreter,
//! draws rasterized by `raster` with Vulkan's rules, images as tightly packed texels (`texel`).
//!
//! Its queue runs a submission as soon as everything it waits for has been signalled, in submission order: one that
//! waits for a fence nobody has raised yet holds back everything after it, as a real queue would.

mod raster;
mod texel;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;

use gpu_backend::cpu;
use gpu_ir::Entry;

use crate::{
    Backend, Command, Error, Fence, GraphicsDesc, Image, ImageDesc, Info, Kind, Load, Memory, Pipeline, Range, Rect,
    Sampler, SamplerDesc, Submit, Viewport, err,
};
use texel::Texels;

/// Where the CPU device's address space starts; nothing is at address 0, so a null pointer faults.
const BASE: u64 = 0x1_0000_0000;
/// Between allocations: an overrun of one doesn't land in the next.
const GAP: u64 = 64 << 10;
/// The CPU device's subgroups are as wide as Apple silicon's, so programs see the same width there and on the Mac.
const SUBGROUP: u32 = 32;

pub(crate) struct Software {
    info: Info,
    /// Allocations by their address.
    memory: BTreeMap<u64, Vec<u8>>,
    addresses: HashMap<Memory, u64>,
    next_address: u64,
    next_memory: u32,
    pipelines: Vec<Program>,
    images: Vec<Option<Texels>>,
    samplers: Vec<SamplerDesc>,
    fences: Vec<u64>,
    pending: VecDeque<Pending>,
}

enum Program {
    Compute(Box<Entry>),
    Graphics(Box<raster::Graphics>),
}

/// The CPU device's images and samplers, as the interpreter samples them.
pub(crate) struct Tex<'a> {
    images: &'a [Option<Texels>],
    samplers: &'a [SamplerDesc],
}

impl cpu::Textures for Tex<'_> {
    fn size(&self, image: u32) -> (u32, u32) {
        self.images.get(image as usize).and_then(|i| i.as_ref()).map_or((1, 1), |i| (i.desc.width, i.desc.height))
    }

    fn sample(&mut self, image: u32, sampler: u32, coord: [f32; 2], lod: f32) -> Result<[f32; 4], gpu_backend::Error> {
        let missing = || gpu_backend::Error { message: format!("image {image} or sampler {sampler} is gone") };
        let i = self.images.get(image as usize).and_then(|i| i.as_ref()).ok_or_else(missing)?;
        let s = self.samplers.get(sampler as usize).ok_or_else(missing)?;
        Ok(i.sample(s, coord, lod))
    }
}

struct Pending {
    commands: Vec<Command>,
    wait: Vec<(Fence, u64)>,
    signal: Vec<(Fence, u64)>,
}

pub(crate) struct Space<'a>(&'a mut BTreeMap<u64, Vec<u8>>);

impl Space<'_> {
    fn find(&mut self, addr: u64, len: usize) -> Result<&mut [u8], gpu_backend::Error> {
        let fault = || gpu_backend::Error { message: format!("{len} bytes at device address {addr:#x}, outside every allocation") };
        let (&base, bytes) = self.0.range_mut(..=addr).next_back().ok_or_else(fault)?;
        let at = (addr - base) as usize;
        match at.checked_add(len) {
            Some(end) if end <= bytes.len() => Ok(&mut bytes[at..end]),
            _ => Err(fault()),
        }
    }
}

impl cpu::Memory for Space<'_> {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), gpu_backend::Error> {
        let len = out.len();
        out.copy_from_slice(self.find(addr, len)?);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), gpu_backend::Error> {
        self.find(addr, data.len())?.copy_from_slice(data);
        Ok(())
    }
}

impl Software {
    pub(crate) fn new(why: Option<String>) -> Self {
        Software {
            info: Info {
                kind: Kind::Software,
                name: "software".into(),
                backend: "software",
                subgroup_size: SUBGROUP,
                max_workgroup_invocations: 1024,
                why,
            },
            memory: BTreeMap::new(),
            addresses: HashMap::new(),
            next_address: BASE,
            next_memory: 0,
            pipelines: Vec::new(),
            images: Vec::new(),
            samplers: Vec::new(),
            fences: Vec::new(),
            pending: VecDeque::new(),
        }
    }

    fn bytes(&mut self, m: Memory) -> Result<&mut Vec<u8>, Error> {
        let addr = self.addresses.get(&m).copied().unwrap_or(0);
        match self.memory.get_mut(&addr) {
            Some(b) => Ok(b),
            None => err(format!("{m:?} isn't allocated")),
        }
    }

    fn texels(&mut self, image: Image) -> Result<&mut Texels, Error> {
        match self.images.get_mut(image.0 as usize).and_then(|i| i.as_mut()) {
            Some(t) => Ok(t),
            None => err(format!("{image:?} isn't allocated")),
        }
    }

    /// Bytes between memory and an image's region, `row_pixels` to a row in memory.
    fn copy_image(&mut self, range: Range, row_pixels: u32, image: Image, region: Rect, to_image: bool) -> Result<(), Error> {
        let addr = self.addresses[&range.memory] + range.offset;
        let bpp = self.texels(image)?.desc.format.bytes() as u64;
        let row = row_pixels.max(region.width) as u64;
        let width = self.texels(image)?.desc.width as u64;
        for y in 0..region.height as u64 {
            let len = region.width as u64 * bpp;
            let mem = addr + y * row * bpp;
            let at = (((region.y as u64 + y) * width + region.x as u64) * bpp) as usize;
            let mut bytes = vec![0u8; len as usize];
            if to_image {
                cpu::Memory::read(&mut Space(&mut self.memory), mem, &mut bytes)?;
                self.texels(image)?.data[at..at + len as usize].copy_from_slice(&bytes);
            } else {
                bytes.copy_from_slice(&self.texels(image)?.data[at..at + len as usize]);
                cpu::Memory::write(&mut Space(&mut self.memory), mem, &bytes)?;
            }
        }
        Ok(())
    }

    fn run(&mut self, commands: &[Command]) -> Result<(), Error> {
        let mut pass: Option<raster::Target> = None;
        for c in commands {
            match c {
                Command::Dispatch { pipeline, bindings, groups } => {
                    let addrs: Vec<u64> = bindings.iter().map(|b| self.addresses[&b.memory] + b.offset).collect();
                    let Program::Compute(entry) = &self.pipelines[pipeline.0 as usize] else { return err("a draw pipeline dispatched") };
                    let d = cpu::Dispatch { bindings: &addrs, groups: *groups, subgroup_size: SUBGROUP };
                    cpu::dispatch(entry, &d, &mut Space(&mut self.memory))?;
                }
                Command::BeginRender { colors, depth } => {
                    for a in colors {
                        if let Load::Clear(c) = a.load {
                            let t = self.texels(a.image)?;
                            for y in 0..t.desc.height {
                                for x in 0..t.desc.width {
                                    t.write(x, y, c);
                                }
                            }
                        }
                    }
                    if let Some(d) = depth
                        && let Load::Clear(z) = d.load {
                            let t = self.texels(d.image)?;
                            for y in 0..t.desc.height {
                                for x in 0..t.desc.width {
                                    t.write(x, y, [z, 0.0, 0.0, 1.0]);
                                }
                            }
                        }
                    let first = colors.first().map(|a| a.image).or(depth.map(|d| d.image)).unwrap_or(Image(0));
                    let desc = self.texels(first)?.desc;
                    let (w, h) = (desc.width, desc.height);
                    pass = Some(raster::Target {
                        colors: colors.iter().map(|a| a.image.0).collect(),
                        depth: depth.map(|d| d.image.0),
                        viewport: Viewport { x: 0.0, y: 0.0, width: w as f32, height: h as f32, min_depth: 0.0, max_depth: 1.0 },
                        scissor: Rect { x: 0, y: 0, width: w, height: h },
                    });
                }
                Command::SetViewport(v) => {
                    if let Some(p) = &mut pass {
                        p.viewport = *v;
                    }
                }
                Command::SetScissor(r) => {
                    if let Some(p) = &mut pass {
                        p.scissor = *r;
                    }
                }
                Command::Draw(d) => {
                    let Some(target) = &pass else { return err("a draw outside a render pass") };
                    let Some(Program::Graphics(g)) = self.pipelines.get(d.pipeline.0 as usize) else { return err("a compute pipeline drawn") };
                    let mut dev = raster::Device {
                        memory: &mut self.memory,
                        addresses: &self.addresses,
                        images: &mut self.images,
                        samplers: &self.samplers,
                        subgroup: SUBGROUP,
                    };
                    dev.draw(g, d, target)?;
                }
                Command::EndRender => pass = None,
                Command::CopyBufferToImage { from, row_pixels, to, region } => self.copy_image(*from, *row_pixels, *to, *region, true)?,
                Command::CopyImageToBuffer { from, region, to, row_pixels } => self.copy_image(*to, *row_pixels, *from, *region, false)?,
                Command::Copy { from, to, at } => {
                    let (o, n) = (from.offset as usize, from.size as usize);
                    let data = self.bytes(from.memory)?[o..o + n].to_vec();
                    let a = *at as usize;
                    self.bytes(*to)?[a..a + n].copy_from_slice(&data);
                }
                Command::Fill { range, value } => {
                    let (o, n) = (range.offset as usize, range.size as usize);
                    for word in self.bytes(range.memory)?[o..o + n].as_chunks_mut::<4>().0 {
                        word.copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        Ok(())
    }

    /// Runs every submission at the queue's head whose waits are met.
    fn drain(&mut self) -> Result<(), Error> {
        while let Some(head) = self.pending.front() {
            if head.wait.iter().any(|(f, v)| self.fences[f.0 as usize] < *v) {
                break;
            }
            let Some(p) = self.pending.pop_front() else { break };
            self.run(&p.commands)?;
            for (f, v) in p.signal {
                let slot = &mut self.fences[f.0 as usize];
                *slot = (*slot).max(v);
            }
        }
        Ok(())
    }
}

impl Backend for Software {
    fn info(&self) -> &Info {
        &self.info
    }

    fn alloc(&mut self, size: u64) -> Result<Memory, Error> {
        let len = usize::try_from(size).map_err(|_| Error { message: format!("{size} bytes is more than this host can hold") })?;
        let addr = self.next_address;
        self.next_address = (addr + size + GAP).next_multiple_of(GAP);
        self.memory.insert(addr, vec![0; len]);
        let m = Memory(self.next_memory);
        self.next_memory += 1;
        self.addresses.insert(m, addr);
        Ok(m)
    }

    fn free(&mut self, memory: Memory) {
        if let Some(addr) = self.addresses.remove(&memory) {
            self.memory.remove(&addr);
        }
    }

    fn write(&mut self, memory: Memory, offset: u64, data: &[u8]) -> Result<(), Error> {
        let at = offset as usize;
        self.bytes(memory)?[at..at + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn read(&mut self, memory: Memory, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        let at = offset as usize;
        out.copy_from_slice(&self.bytes(memory)?[at..at + out.len()]);
        Ok(())
    }

    fn pipeline(&mut self, entry: &Entry) -> Result<Pipeline, Error> {
        self.pipelines.push(Program::Compute(Box::new(entry.clone())));
        Ok(Pipeline(self.pipelines.len() as u32 - 1))
    }

    fn image(&mut self, desc: &ImageDesc) -> Result<Image, Error> {
        self.images.push(Some(Texels::new(*desc)));
        Ok(Image(self.images.len() as u32 - 1))
    }

    fn free_image(&mut self, image: Image) {
        if let Some(slot) = self.images.get_mut(image.0 as usize) {
            *slot = None;
        }
    }

    fn sampler(&mut self, desc: &SamplerDesc) -> Result<Sampler, Error> {
        self.samplers.push(*desc);
        Ok(Sampler(self.samplers.len() as u32 - 1))
    }

    fn graphics(&mut self, d: &GraphicsDesc<'_>) -> Result<Pipeline, Error> {
        self.pipelines.push(Program::Graphics(Box::new(raster::Graphics {
            vertex: d.vertex.clone(),
            fragment: d.fragment.clone(),
            buffers: d.buffers.to_vec(),
            attributes: d.attributes.to_vec(),
            topology: d.topology,
            cull: d.cull,
            front: d.front,
            depth: d.depth,
            colors: d.colors.to_vec(),
        })));
        Ok(Pipeline(self.pipelines.len() as u32 - 1))
    }

    fn fence(&mut self) -> Result<Fence, Error> {
        self.fences.push(0);
        Ok(Fence(self.fences.len() as u32 - 1))
    }

    fn submit(&mut self, s: Submit<'_>) -> Result<(), Error> {
        self.pending.push_back(Pending { commands: s.commands.to_vec(), wait: s.wait.to_vec(), signal: s.signal.to_vec() });
        self.drain()
    }

    fn signal(&mut self, fence: Fence, value: u64) -> Result<(), Error> {
        let slot = &mut self.fences[fence.0 as usize];
        *slot = (*slot).max(value);
        self.drain()
    }

    fn fence_value(&mut self, fence: Fence) -> Result<u64, Error> {
        Ok(self.fences[fence.0 as usize])
    }

    fn wait(&mut self, fence: Fence, value: u64, _timeout: Duration) -> Result<bool, Error> {
        // Work runs when it's submitted, so waiting can't make it happen: a value not reached yet needs another
        // submission or a signal from the host first.
        Ok(self.fences[fence.0 as usize] >= value)
    }
}
