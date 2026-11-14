//! The IR run on the CPU: the software device's engine, and the reference every other backend is checked against.
//!
//! A workgroup runs as one group of lanes in lockstep, statement by statement, with a mask of the lanes still
//! running, as a GPU runs a subgroup. That makes barriers trivially right (every lane that reaches one reaches it
//! together) and subgroup operations exact (they see the lanes that are active at that point). An `If` splits the
//! mask; a `Loop` runs until no lane is left in it; `Break`, `Continue` and `Return` take lanes out.
//!
//! Every value is kept per lane as up to four 64-bit components; 32-bit types use the low half. Device memory is the
//! caller's, through [`Memory`]; workgroup and private memory are the interpreter's, zeroed for each workgroup.
//!
//! The graphics stages run through [`shade`]: a group of lanes (vertices, or fragments in 2×2 quads), each with its
//! inputs and builtins, giving back each lane's outputs. A fragment group's lanes come in quads, lane `4q + 2y + x`
//! being pixel `(x, y)` of quad `q`, so derivatives are differences between a lane and its neighbour; lanes the
//! primitive doesn't cover are helpers, run for their values but never writing memory. Images are the caller's too,
//! through [`Textures`].
//!
//! This is the first form of the CPU backend: interpreted, one workgroup at a time. Compiling to host code comes
//! later; the semantics here are what it must keep.

use gpu_ir::{
    AtomicOp, Axis, BinOp, Block, Builtin, Cmp, Entry, Lod, Op, Scalar, Space, Stmt, SubgroupOp, Type, UnOp, Value,
};

use crate::{Error, err};

/// A device's memory, by address.
pub trait Memory {
    fn read(&mut self, addr: u64, out: &mut [u8]) -> Result<(), Error>;
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), Error>;
}

/// A device's images, by the numbers [`Bound`] gives them.
pub trait Textures {
    /// An image's width and height.
    fn size(&self, image: u32) -> (u32, u32);
    /// A filtered sample at a normalized coordinate and level of detail (log2 of the texels a pixel covers).
    fn sample(&mut self, image: u32, sampler: u32, coord: [f32; 2], lod: f32) -> Result<[f32; 4], Error>;
}

/// What one of an entry point's bindings is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// Memory, at an address.
    Buffer(u64),
    Image(u32),
    Sampler(u32),
    /// An image and a sampler.
    Combined(u32, u32),
}

/// What a dispatch hands its program.
#[derive(Debug, Clone, Copy)]
pub struct Dispatch<'a> {
    /// Each binding's address in device memory, by the entry point's binding index.
    pub bindings: &'a [u64],
    pub groups: [u32; 3],
    /// The device's subgroup width, unless the program names its own.
    pub subgroup_size: u32,
}

/// Runs every workgroup of a dispatch, in order.
pub fn dispatch(entry: &Entry, d: &Dispatch<'_>, memory: &mut dyn Memory) -> Result<(), Error> {
    if d.bindings.len() < entry.bindings.len() {
        return err(format!("{} needs {} bindings, the dispatch gave {}", entry.name, entry.bindings.len(), d.bindings.len()));
    }
    let bound: Vec<Bound> = d.bindings.iter().map(|&a| Bound::Buffer(a)).collect();
    let width = entry.subgroup_size.unwrap_or(d.subgroup_size).max(1);
    let lanes = entry.invocations_per_group() as usize;
    let [gx, gy, gz] = d.groups;
    for z in 0..gz {
        for y in 0..gy {
            for x in 0..gx {
                let mut g = Group::new(entry, &bound, lanes, width, memory, None);
                g.group = [x, y, z];
                g.groups = d.groups;
                let mut active = vec![true; g.lanes];
                g.block(&entry.body, &mut active)?;
            }
        }
    }
    Ok(())
}

/// One lane's value: up to four components, each in the low bits of a word (an `f32` as its bits).
pub type Lanes = [u64; 4];

/// The values a graphics stage's invocations are given besides their inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Builtins {
    pub vertex_index: u32,
    pub instance_index: u32,
    pub frag_coord: [f32; 4],
    pub front_facing: bool,
}

/// A group of a graphics stage's invocations, run together.
#[derive(Debug, Clone, Copy)]
pub struct Shade<'a> {
    /// What each of the entry point's bindings is bound to.
    pub bindings: &'a [Bound],
    pub subgroup_size: u32,
    /// Each lane's inputs, `inputs[lane * entry.inputs.len() + i]`.
    pub inputs: &'a [Lanes],
    /// Each lane's builtins; there are as many lanes as these.
    pub builtins: &'a [Builtins],
    /// Lanes run only so their quad's derivatives are right: they write no memory, and their outputs are nobody's.
    pub helpers: &'a [bool],
}

/// What a group of invocations gave back.
#[derive(Debug, Clone, PartialEq)]
pub struct Shaded {
    /// Each lane's outputs, `outputs[lane * entry.outputs.len() + i]`, zero where never written.
    pub outputs: Vec<Lanes>,
    /// The lanes that discarded themselves.
    pub discarded: Vec<bool>,
}

/// Runs a vertex or fragment shader's invocations, a group at a time.
pub fn shade(entry: &Entry, s: &Shade<'_>, memory: &mut dyn Memory, textures: &mut dyn Textures) -> Result<Shaded, Error> {
    let lanes = s.builtins.len();
    if s.bindings.len() < entry.bindings.len() {
        return err(format!("{} needs {} bindings, the draw gave {}", entry.name, entry.bindings.len(), s.bindings.len()));
    }
    if s.inputs.len() != lanes * entry.inputs.len() || s.helpers.len() != lanes {
        return err(format!("{}: inputs for {} lanes, not {lanes}", entry.name, s.inputs.len() / entry.inputs.len().max(1)));
    }
    let width = entry.subgroup_size.unwrap_or(s.subgroup_size).max(1);
    let mut g = Group::new(entry, s.bindings, lanes, width, memory, Some(textures));
    g.inputs = s.inputs;
    g.builtins = s.builtins;
    g.helpers = s.helpers;
    g.outputs = vec![[0; 4]; lanes * entry.outputs.len()];
    g.discarded = vec![false; lanes];
    let mut active = vec![true; lanes];
    g.block(&entry.body, &mut active)?;
    Ok(Shaded { outputs: g.outputs, discarded: g.discarded })
}

struct Group<'a> {
    entry: &'a Entry,
    bindings: &'a [Bound],
    memory: &'a mut dyn Memory,
    textures: Option<&'a mut dyn Textures>,
    lanes: usize,
    width: usize,
    group: [u32; 3],
    groups: [u32; 3],
    inputs: &'a [Lanes],
    builtins: &'a [Builtins],
    helpers: &'a [bool],
    outputs: Vec<Lanes>,
    discarded: Vec<bool>,
    /// `values[value * lanes + lane]`.
    values: Vec<Lanes>,
    vars: Vec<Lanes>,
    workgroup: Vec<u8>,
    workgroup_base: Vec<u64>,
    /// Each lane's private memory, `private_size` bytes apiece.
    private: Vec<u8>,
    private_size: usize,
    private_base: Vec<u64>,
    /// For each loop being run, innermost last: the lanes that broke out of it, and the lanes that continued.
    breaks: Vec<Vec<bool>>,
    continues: Vec<Vec<bool>>,
}

fn regions(regions: &[gpu_ir::Region]) -> (Vec<u64>, usize) {
    let mut bases = Vec::new();
    let mut at = 0u64;
    for r in regions {
        at = at.next_multiple_of(r.align.max(1) as u64);
        bases.push(at);
        at += r.size as u64;
    }
    (bases, at as usize)
}

impl<'a> Group<'a> {
    fn new(
        entry: &'a Entry,
        bindings: &'a [Bound],
        lanes: usize,
        width: u32,
        memory: &'a mut dyn Memory,
        textures: Option<&'a mut dyn Textures>,
    ) -> Self {
        let (workgroup_base, workgroup_size) = regions(&entry.workgroup);
        let (private_base, private_size) = regions(&entry.private);
        Group {
            entry,
            bindings,
            memory,
            textures,
            lanes,
            width: width as usize,
            group: [0; 3],
            groups: [1; 3],
            inputs: &[],
            builtins: &[],
            helpers: &[],
            outputs: Vec::new(),
            discarded: Vec::new(),
            values: vec![[0; 4]; entry.values.len() * lanes],
            vars: vec![[0; 4]; entry.vars.len() * lanes],
            workgroup: vec![0; workgroup_size],
            workgroup_base,
            private: vec![0; private_size * lanes],
            private_size,
            private_base,
            breaks: Vec::new(),
            continues: Vec::new(),
        }
    }

    fn get(&self, v: Value, lane: usize) -> Lanes {
        self.values[v.0 as usize * self.lanes + lane]
    }

    fn put(&mut self, v: Value, lane: usize, x: Lanes) {
        self.values[v.0 as usize * self.lanes + lane] = x;
    }

    fn ty(&self, v: Value) -> Type {
        self.entry.ty(v)
    }

    fn block(&mut self, stmts: &Block, active: &mut Vec<bool>) -> Result<(), Error> {
        for stmt in stmts {
            if !active.iter().any(|&a| a) {
                return Ok(());
            }
            self.stmt(stmt, active)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt, active: &mut Vec<bool>) -> Result<(), Error> {
        match stmt {
            Stmt::Let(v, op) => self.op(*v, op, active)?,
            Stmt::Store(ptr, value) => {
                let ty = self.ty(*value);
                for lane in lanes_of(active) {
                    if self.is_helper(lane) {
                        continue;
                    }
                    let bytes = to_bytes(self.get(*value, lane), ty);
                    self.write(*ptr, lane, &bytes)?;
                }
            }
            Stmt::Write(var, value) => {
                for lane in lanes_of(active) {
                    self.vars[var.0 as usize * self.lanes + lane] = self.get(*value, lane);
                }
            }
            Stmt::If { cond, then, otherwise } => {
                let mut t = vec![false; self.lanes];
                let mut f = vec![false; self.lanes];
                for lane in lanes_of(active) {
                    if self.get(*cond, lane)[0] != 0 {
                        t[lane] = true;
                    } else {
                        f[lane] = true;
                    }
                }
                self.block(then, &mut t)?;
                self.block(otherwise, &mut f)?;
                for lane in 0..self.lanes {
                    active[lane] = t[lane] || f[lane];
                }
            }
            Stmt::Loop { body, continuing, break_if } => {
                self.breaks.push(vec![false; self.lanes]);
                self.continues.push(vec![false; self.lanes]);
                let mut running = active.clone();
                let mut result = Ok(());
                while running.iter().any(|&a| a) {
                    if let Err(e) = self.block(body, &mut running) {
                        result = Err(e);
                        break;
                    }
                    let continued = std::mem::replace(self.continues.last_mut().unwrap_or(&mut Vec::new()), vec![false; self.lanes]);
                    for (r, c) in running.iter_mut().zip(continued) {
                        *r |= c;
                    }
                    if let Err(e) = self.block(continuing, &mut running) {
                        result = Err(e);
                        break;
                    }
                    if let Some(cond) = break_if {
                        for lane in lanes_of(&running.clone()) {
                            if self.get(*cond, lane)[0] != 0 {
                                running[lane] = false;
                                if let Some(b) = self.breaks.last_mut() {
                                    b[lane] = true;
                                }
                            }
                        }
                    }
                }
                self.continues.pop();
                let broke = self.breaks.pop().unwrap_or_default();
                result?;
                *active = broke;
            }
            Stmt::Break => {
                let Some(b) = self.breaks.last_mut() else { return err("a break outside a loop") };
                for (lane, a) in active.iter_mut().enumerate() {
                    if *a {
                        b[lane] = true;
                        *a = false;
                    }
                }
            }
            Stmt::Continue => {
                let Some(c) = self.continues.last_mut() else { return err("a continue outside a loop") };
                for (lane, a) in active.iter_mut().enumerate() {
                    if *a {
                        c[lane] = true;
                        *a = false;
                    }
                }
            }
            Stmt::Return => active.iter_mut().for_each(|a| *a = false),
            // Every active lane is here together, and memory is already in order: nothing to wait for.
            Stmt::Barrier(_) => {}
            Stmt::Output(n, value) => {
                let count = self.entry.outputs.len();
                for lane in lanes_of(active) {
                    let x = self.get(*value, lane);
                    if let Some(slot) = self.outputs.get_mut(lane * count + *n as usize) {
                        *slot = x;
                    }
                }
            }
            Stmt::Discard => {
                for (lane, a) in active.iter_mut().enumerate() {
                    if *a {
                        *a = false;
                        if let Some(d) = self.discarded.get_mut(lane) {
                            *d = true;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn is_helper(&self, lane: usize) -> bool {
        self.helpers.get(lane).copied().unwrap_or(false)
    }

    // ---- memory ---------------------------------------------------------------------------------------------

    fn local_slice(&mut self, space: Space, addr: u64, lane: usize, len: usize) -> Result<&mut [u8], Error> {
        let (base, size, what) = match space {
            Space::Workgroup => (0, self.workgroup.len(), "workgroup"),
            Space::Private => (lane * self.private_size, self.private_size, "private"),
            Space::Device => return err("device memory isn't local"),
        };
        let at = addr as usize;
        if addr > usize::MAX as u64 || at.checked_add(len).is_none_or(|end| end > size) {
            return err(format!("{} bytes at {what} address {addr:#x}, past its {size} bytes", len));
        }
        let mem = if space == Space::Workgroup { &mut self.workgroup } else { &mut self.private };
        Ok(&mut mem[base + at..base + at + len])
    }

    fn read(&mut self, ptr: Value, lane: usize, out: &mut [u8]) -> Result<(), Error> {
        let addr = self.get(ptr, lane)[0];
        match self.ty(ptr) {
            Type::Ptr(Space::Device) => self.memory.read(addr, out),
            Type::Ptr(space) => {
                let len = out.len();
                out.copy_from_slice(self.local_slice(space, addr, lane, len)?);
                Ok(())
            }
            _ => err("a read through a value that isn't a pointer"),
        }
    }

    fn write(&mut self, ptr: Value, lane: usize, data: &[u8]) -> Result<(), Error> {
        let addr = self.get(ptr, lane)[0];
        match self.ty(ptr) {
            Type::Ptr(Space::Device) => self.memory.write(addr, data),
            Type::Ptr(space) => {
                self.local_slice(space, addr, lane, data.len())?.copy_from_slice(data);
                Ok(())
            }
            _ => err("a write through a value that isn't a pointer"),
        }
    }

    // ---- operations -----------------------------------------------------------------------------------------

    fn builtin(&self, b: Builtin, lane: usize) -> Lanes {
        let [wx, wy, _] = self.entry.workgroup_size;
        let (wx, wy) = (wx as usize, wy as usize);
        let local = [(lane % wx) as u64, ((lane / wx) % wy) as u64, (lane / (wx * wy)) as u64];
        let g = self.group.map(|x| x as u64);
        let size = self.entry.workgroup_size.map(|x| x as u64);
        match b {
            Builtin::GlobalId => [g[0] * size[0] + local[0], g[1] * size[1] + local[1], g[2] * size[2] + local[2], 0],
            Builtin::LocalId => [local[0], local[1], local[2], 0],
            Builtin::GroupId => [g[0], g[1], g[2], 0],
            Builtin::GroupCount => {
                let [x, y, z] = self.groups.map(|x| x as u64);
                [x, y, z, 0]
            }
            Builtin::LocalIndex => [lane as u64, 0, 0, 0],
            Builtin::SubgroupLane => [(lane % self.width) as u64, 0, 0, 0],
            Builtin::SubgroupSize => [self.width as u64, 0, 0, 0],
            Builtin::SubgroupId => [(lane / self.width) as u64, 0, 0, 0],
            Builtin::SubgroupCount => [self.lanes.div_ceil(self.width) as u64, 0, 0, 0],
            Builtin::VertexIndex => [self.builtins.get(lane).map_or(0, |b| b.vertex_index) as u64, 0, 0, 0],
            Builtin::InstanceIndex => [self.builtins.get(lane).map_or(0, |b| b.instance_index) as u64, 0, 0, 0],
            Builtin::FragCoord => self.builtins.get(lane).map_or([0; 4], |b| b.frag_coord.map(fb)),
            Builtin::FrontFacing => [self.builtins.get(lane).is_some_and(|b| b.front_facing) as u64, 0, 0, 0],
        }
    }

    fn op(&mut self, v: Value, op: &Op, active: &[bool]) -> Result<(), Error> {
        match op {
            Op::Subgroup(sop, value, lane) => return self.subgroup(v, *sop, *value, *lane, active),
            Op::Derivative(axis, x) => {
                let ty = self.ty(*x);
                for lane in lanes_of(active) {
                    let d = self.derivative(*axis, *x, lane, ty.lanes() as usize);
                    self.put(v, lane, d);
                }
                return Ok(());
            }
            Op::Sample { image, sampler, coord, lod } => return self.sample(v, *image, *sampler, *coord, *lod, active),
            _ => {}
        }
        let ty = self.ty(v);
        for lane in lanes_of(active) {
            let r = self.eval(op, ty, lane)?;
            self.put(v, lane, r);
        }
        Ok(())
    }

    fn eval(&mut self, op: &Op, ty: Type, lane: usize) -> Result<Lanes, Error> {
        let n = ty.lanes() as usize;
        Ok(match op {
            Op::Const(_, bits) => [*bits, 0, 0, 0],
            Op::Builtin(b) => self.builtin(*b, lane),
            Op::Binding(i) => match self.bindings.get(*i as usize) {
                Some(Bound::Buffer(addr)) => [*addr, 0, 0, 0],
                other => return err(format!("binding {i} is {other:?}, not memory")),
            },
            Op::Input(i) => {
                let count = self.entry.inputs.len();
                self.inputs.get(lane * count + *i as usize).copied().unwrap_or([0; 4])
            }
            Op::Workgroup(i) => [self.workgroup_base[*i as usize], 0, 0, 0],
            Op::Private(i) => [self.private_base[*i as usize], 0, 0, 0],
            Op::Offset(p, bytes) => {
                let s = self.ty(*bytes).scalar().unwrap_or(Scalar::U64);
                let delta = sign_extend(self.get(*bytes, lane)[0], s);
                [self.get(*p, lane)[0].wrapping_add(delta), 0, 0, 0]
            }
            Op::Load(p, t) => {
                let mut bytes = [0u8; 32];
                let len = t.size() as usize;
                self.read(*p, lane, &mut bytes[..len])?;
                from_bytes(&bytes[..len], *t)
            }
            Op::Read(var) => self.vars[var.0 as usize * self.lanes + lane],
            Op::Unary(u, a) => {
                let s = self.ty(*a).scalar().unwrap_or(Scalar::U32);
                let x = self.get(*a, lane);
                let mut r = [0; 4];
                for i in 0..n {
                    r[i] = unary(*u, s, x[i]);
                }
                r
            }
            Op::Binary(b, a, c) => {
                let s = self.ty(*a).scalar().unwrap_or(Scalar::U32);
                let (x, y) = (self.get(*a, lane), self.get(*c, lane));
                let mut r = [0; 4];
                for i in 0..n {
                    r[i] = binary(*b, s, x[i], y[i]);
                }
                r
            }
            Op::Compare(c, a, b) => {
                let s = self.ty(*a).scalar().unwrap_or(Scalar::U32);
                let (x, y) = (self.get(*a, lane), self.get(*b, lane));
                let mut r = [0; 4];
                for i in 0..n {
                    r[i] = compare(*c, s, x[i], y[i]) as u64;
                }
                r
            }
            Op::Select(c, a, b) => {
                let cond = self.get(*c, lane);
                let (x, y) = (self.get(*a, lane), self.get(*b, lane));
                let wide = self.ty(*c).lanes() > 1;
                let mut r = [0; 4];
                for i in 0..n {
                    r[i] = if cond[if wide { i } else { 0 }] != 0 { x[i] } else { y[i] };
                }
                r
            }
            Op::Convert(a, to) => {
                let from = self.ty(*a).scalar().unwrap_or(Scalar::U32);
                let x = self.get(*a, lane);
                let mut r = [0; 4];
                for i in 0..n {
                    r[i] = convert(x[i], from, *to);
                }
                r
            }
            Op::Bitcast(a, to) => {
                let from = self.ty(*a);
                from_bytes(&to_bytes(self.get(*a, lane), from), *to)
            }
            Op::Construct(parts) => {
                let mut r = [0; 4];
                for (i, p) in parts.iter().enumerate() {
                    r[i] = self.get(*p, lane)[0];
                }
                r
            }
            Op::Extract(v, i) => [self.get(*v, lane)[*i as usize], 0, 0, 0],
            Op::ExtractDynamic(v, i) => {
                let lanes = self.ty(*v).lanes() as u64;
                let index = self.get(*i, lane)[0];
                let index = if index < lanes { index } else { 0 };
                [self.get(*v, lane)[index as usize], 0, 0, 0]
            }
            Op::Insert(v, i, x) => {
                let mut r = self.get(*v, lane);
                r[*i as usize] = self.get(*x, lane)[0];
                r
            }
            Op::Atomic(aop, p, x, s) => {
                let size = s.size() as usize;
                let mut bytes = [0u8; 8];
                self.read(*p, lane, &mut bytes[..size])?;
                let old = from_bytes(&bytes[..size], Type::Scalar(*s))[0];
                let operand = self.get(*x, lane)[0];
                let new = match aop {
                    AtomicOp::Add => binary(BinOp::Add, *s, old, operand),
                    AtomicOp::Sub => binary(BinOp::Sub, *s, old, operand),
                    AtomicOp::Min => binary(BinOp::Min, *s, old, operand),
                    AtomicOp::Max => binary(BinOp::Max, *s, old, operand),
                    AtomicOp::And => binary(BinOp::And, *s, old, operand),
                    AtomicOp::Or => binary(BinOp::Or, *s, old, operand),
                    AtomicOp::Xor => binary(BinOp::Xor, *s, old, operand),
                    AtomicOp::Exchange => operand,
                };
                if !self.is_helper(lane) {
                    self.write(*p, lane, &to_bytes([new, 0, 0, 0], Type::Scalar(*s)))?;
                }
                [old, 0, 0, 0]
            }
            Op::CompareExchange(p, c, x, s) => {
                let size = s.size() as usize;
                let mut bytes = [0u8; 8];
                self.read(*p, lane, &mut bytes[..size])?;
                let old = from_bytes(&bytes[..size], Type::Scalar(*s))[0];
                if old == self.get(*c, lane)[0] && !self.is_helper(lane) {
                    let new = self.get(*x, lane)[0];
                    self.write(*p, lane, &to_bytes([new, 0, 0, 0], Type::Scalar(*s)))?;
                }
                [old, 0, 0, 0]
            }
            Op::Subgroup(..) | Op::Derivative(..) | Op::Sample { .. } => {
                return err("an operation across lanes evaluated lane by lane");
            }
        })
    }

    /// How a value changes from this lane's pixel to its neighbour's along the axis, in its 2×2 quad: the right
    /// (or lower) pixel's value less the left (or upper) one's, the same for both pixels of the pair.
    fn derivative(&self, axis: Axis, x: Value, lane: usize, n: usize) -> Lanes {
        let bit = if axis == Axis::X { 1 } else { 2 };
        let low = lane & !bit;
        let high = (low | bit).min(self.lanes - 1);
        let (a, b) = (self.get(x, low), self.get(x, high));
        let mut r = [0; 4];
        for i in 0..n {
            r[i] = fb(f(b[i]) - f(a[i]));
        }
        r
    }

    fn sample(&mut self, v: Value, image: u32, sampler: Option<u32>, coord: Value, lod: Lod, active: &[bool]) -> Result<(), Error> {
        let bound = |i: u32| self.bindings.get(i as usize).copied();
        let (img, smp) = match (bound(image), sampler.map(bound)) {
            (Some(Bound::Combined(i, s)), None) => (i, s),
            (Some(Bound::Image(i)), Some(Some(Bound::Sampler(s)))) => (i, s),
            (b, s) => return err(format!("sampling {b:?} with {s:?}")),
        };
        let Some(textures) = self.textures.as_deref() else { return err("a sample with no images bound") };
        let (w, h) = textures.size(img);
        for lane in lanes_of(active) {
            let c = self.get(coord, lane);
            let level = match lod {
                Lod::Level(l) => f(self.get(l, lane)[0]),
                Lod::Implicit | Lod::Bias(_) => {
                    // log2 of the texels one pixel step covers, the longer of the two axes' (Vulkan's ρ).
                    let dx = self.derivative(Axis::X, coord, lane, 2);
                    let dy = self.derivative(Axis::Y, coord, lane, 2);
                    let len = |d: Lanes| ((f(d[0]) * w as f32).powi(2) + (f(d[1]) * h as f32).powi(2)).sqrt();
                    let rho = len(dx).max(len(dy));
                    let bias = if let Lod::Bias(b) = lod { f(self.get(b, lane)[0]) } else { 0.0 };
                    rho.log2() + bias
                }
            };
            let textures = self.textures.as_deref_mut().ok_or_else(|| Error { message: "no images".into() })?;
            let texel = textures.sample(img, smp, [f(c[0]), f(c[1])], level)?;
            self.put(v, lane, texel.map(fb));
        }
        Ok(())
    }

    fn subgroup(&mut self, v: Value, sop: SubgroupOp, value: Option<Value>, lane_op: Option<Value>, active: &[bool]) -> Result<(), Error> {
        let ty = self.ty(v);
        let s = ty.scalar().unwrap_or(Scalar::U32);
        let n = ty.lanes() as usize;
        for start in (0..self.lanes).step_by(self.width) {
            let end = (start + self.width).min(self.lanes);
            let members: Vec<usize> = (start..end).filter(|&l| active[l]).collect();
            let Some(&first) = members.first() else { continue };
            let get = |g: &Self, l: usize| value.map(|x| g.get(x, l)).unwrap_or([0; 4]);
            match sop {
                SubgroupOp::Shuffle | SubgroupOp::ShuffleXor | SubgroupOp::ShuffleUp | SubgroupOp::ShuffleDown => {
                    for &l in &members {
                        let me = l - start;
                        let k = lane_op.map(|x| self.get(x, l)[0] as usize).unwrap_or(0);
                        let from = match sop {
                            SubgroupOp::Shuffle => Some(k),
                            SubgroupOp::ShuffleXor => Some(me ^ k),
                            SubgroupOp::ShuffleUp => me.checked_sub(k),
                            _ => me.checked_add(k),
                        };
                        // A lane past the subgroup reads its own value, as Metal's shuffles do.
                        let from = from.filter(|&f| start + f < end).map_or(l, |f| start + f);
                        let x = get(self, from);
                        self.put(v, l, x);
                    }
                }
                SubgroupOp::BroadcastFirst => {
                    let x = get(self, first);
                    for &l in &members {
                        self.put(v, l, x);
                    }
                }
                SubgroupOp::Elect => {
                    for &l in &members {
                        self.put(v, l, [(l == first) as u64, 0, 0, 0]);
                    }
                }
                SubgroupOp::All | SubgroupOp::Any => {
                    let all = sop == SubgroupOp::All;
                    let r = if all {
                        members.iter().all(|&l| get(self, l)[0] != 0)
                    } else {
                        members.iter().any(|&l| get(self, l)[0] != 0)
                    };
                    for &l in &members {
                        self.put(v, l, [r as u64, 0, 0, 0]);
                    }
                }
                SubgroupOp::Reduce(op) | SubgroupOp::InclusiveScan(op) | SubgroupOp::ExclusiveScan(op) => {
                    let mut acc = [identity(op, s); 4];
                    let mut results = Vec::with_capacity(members.len());
                    for &l in &members {
                        let x = get(self, l);
                        let before = acc;
                        for i in 0..n {
                            acc[i] = binary(op, s, acc[i], x[i]);
                        }
                        results.push(if matches!(sop, SubgroupOp::ExclusiveScan(_)) { before } else { acc });
                    }
                    for (k, &l) in members.iter().enumerate() {
                        let r = if matches!(sop, SubgroupOp::Reduce(_)) { acc } else { results[k] };
                        self.put(v, l, r);
                    }
                }
            }
        }
        Ok(())
    }
}

fn lanes_of(active: &[bool]) -> impl Iterator<Item = usize> + use<'_> {
    active.iter().enumerate().filter(|(_, a)| **a).map(|(i, _)| i)
}

fn sign_extend(bits: u64, s: Scalar) -> u64 {
    match s {
        Scalar::I32 => bits as u32 as i32 as i64 as u64,
        Scalar::U32 => bits as u32 as u64,
        _ => bits,
    }
}

fn to_bytes(x: Lanes, ty: Type) -> Vec<u8> {
    let size = ty.scalar().map_or(8, |s| s.size()) as usize;
    let mut out = Vec::with_capacity(size * ty.lanes() as usize);
    for c in x.iter().take(ty.lanes() as usize) {
        out.extend_from_slice(&c.to_le_bytes()[..size]);
    }
    out
}

fn from_bytes(bytes: &[u8], ty: Type) -> Lanes {
    let size = ty.scalar().map_or(8, |s| s.size()) as usize;
    let mut r = [0u64; 4];
    for (i, chunk) in bytes.chunks(size).take(4).enumerate() {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        r[i] = u64::from_le_bytes(word);
    }
    r
}

fn f(bits: u64) -> f32 {
    f32::from_bits(bits as u32)
}

fn fb(x: f32) -> u64 {
    x.to_bits() as u64
}

fn unary(op: UnOp, s: Scalar, x: u64) -> u64 {
    match s {
        Scalar::F32 => {
            let a = f(x);
            fb(match op {
                UnOp::Neg => -a,
                UnOp::Abs => a.abs(),
                UnOp::Sqrt => a.sqrt(),
                UnOp::Floor => a.floor(),
                UnOp::Ceil => a.ceil(),
                UnOp::Trunc => a.trunc(),
                UnOp::RoundEven => a.round_ties_even(),
                UnOp::Sin => a.sin(),
                UnOp::Cos => a.cos(),
                UnOp::Exp2 => a.exp2(),
                UnOp::Log2 => a.log2(),
                UnOp::Not | UnOp::Popcount => a,
            })
        }
        Scalar::Bool => match op {
            UnOp::Not => (x == 0) as u64,
            _ => x,
        },
        Scalar::U32 | Scalar::I32 => {
            let a = x as u32;
            (match op {
                UnOp::Neg => a.wrapping_neg(),
                UnOp::Not => !a,
                UnOp::Abs => (a as i32).unsigned_abs(),
                UnOp::Popcount => a.count_ones(),
                _ => a,
            }) as u64
        }
        Scalar::U64 | Scalar::I64 => match op {
            UnOp::Neg => x.wrapping_neg(),
            UnOp::Not => !x,
            UnOp::Abs => (x as i64).unsigned_abs(),
            UnOp::Popcount => x.count_ones() as u64,
            _ => x,
        },
    }
}

/// One operation on one component, by the IR's rules (division and remainder by zero give 0, shifts take their
/// count modulo the width).
fn binary(op: BinOp, s: Scalar, x: u64, y: u64) -> u64 {
    match s {
        Scalar::F32 => {
            let (a, b) = (f(x), f(y));
            fb(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                BinOp::Rem => a % b,
                BinOp::Min => a.min(b),
                BinOp::Max => a.max(b),
                _ => a,
            })
        }
        Scalar::Bool => match op {
            BinOp::And | BinOp::Min | BinOp::Mul => x & y,
            BinOp::Or | BinOp::Max | BinOp::Add => x | y,
            BinOp::Xor => x ^ y,
            _ => x,
        },
        Scalar::U32 => {
            let (a, b) = (x as u32, y as u32);
            (match op {
                BinOp::Add => a.wrapping_add(b),
                BinOp::Sub => a.wrapping_sub(b),
                BinOp::Mul => a.wrapping_mul(b),
                BinOp::Div => a.checked_div(b).unwrap_or(0),
                BinOp::Rem => a.checked_rem(b).unwrap_or(0),
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Shl => a.wrapping_shl(b),
                BinOp::Shr => a.wrapping_shr(b),
                BinOp::Min => a.min(b),
                BinOp::Max => a.max(b),
            }) as u64
        }
        Scalar::I32 => {
            let (a, b) = (x as u32 as i32, y as u32 as i32);
            (match op {
                BinOp::Add => a.wrapping_add(b),
                BinOp::Sub => a.wrapping_sub(b),
                BinOp::Mul => a.wrapping_mul(b),
                BinOp::Div => if b == 0 { 0 } else { a.wrapping_div(b) },
                BinOp::Rem => if b == 0 { 0 } else { a.wrapping_rem(b) },
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Shl => a.wrapping_shl(b as u32),
                BinOp::Shr => a.wrapping_shr(b as u32),
                BinOp::Min => a.min(b),
                BinOp::Max => a.max(b),
            }) as u32 as u64
        }
        Scalar::U64 => match op {
            BinOp::Add => x.wrapping_add(y),
            BinOp::Sub => x.wrapping_sub(y),
            BinOp::Mul => x.wrapping_mul(y),
            BinOp::Div => x.checked_div(y).unwrap_or(0),
            BinOp::Rem => x.checked_rem(y).unwrap_or(0),
            BinOp::And => x & y,
            BinOp::Or => x | y,
            BinOp::Xor => x ^ y,
            BinOp::Shl => x.wrapping_shl(y as u32),
            BinOp::Shr => x.wrapping_shr(y as u32),
            BinOp::Min => x.min(y),
            BinOp::Max => x.max(y),
        },
        Scalar::I64 => {
            let (a, b) = (x as i64, y as i64);
            (match op {
                BinOp::Add => a.wrapping_add(b),
                BinOp::Sub => a.wrapping_sub(b),
                BinOp::Mul => a.wrapping_mul(b),
                BinOp::Div => if b == 0 { 0 } else { a.wrapping_div(b) },
                BinOp::Rem => if b == 0 { 0 } else { a.wrapping_rem(b) },
                BinOp::And => a & b,
                BinOp::Or => a | b,
                BinOp::Xor => a ^ b,
                BinOp::Shl => a.wrapping_shl(b as u32),
                BinOp::Shr => a.wrapping_shr(b as u32),
                BinOp::Min => a.min(b),
                BinOp::Max => a.max(b),
            }) as u64
        }
    }
}

fn compare(c: Cmp, s: Scalar, x: u64, y: u64) -> bool {
    use core::cmp::Ordering;
    let ord = match s {
        Scalar::F32 => f(x).partial_cmp(&f(y)),
        Scalar::I32 => Some((x as u32 as i32).cmp(&(y as u32 as i32))),
        Scalar::I64 => Some((x as i64).cmp(&(y as i64))),
        Scalar::U32 => Some((x as u32).cmp(&(y as u32))),
        Scalar::U64 | Scalar::Bool => Some(x.cmp(&y)),
    };
    match (c, ord) {
        (Cmp::Ne, None) => true,
        (_, None) => false,
        (Cmp::Eq, Some(o)) => o == Ordering::Equal,
        (Cmp::Ne, Some(o)) => o != Ordering::Equal,
        (Cmp::Lt, Some(o)) => o == Ordering::Less,
        (Cmp::Le, Some(o)) => o != Ordering::Greater,
        (Cmp::Gt, Some(o)) => o == Ordering::Greater,
        (Cmp::Ge, Some(o)) => o != Ordering::Less,
    }
}

fn convert(x: u64, from: Scalar, to: Scalar) -> u64 {
    // Through the widest form of each kind: integers as i128 (sign- or zero-extended), floats as f64.
    let as_int: i128 = match from {
        Scalar::Bool => (x != 0) as i128,
        Scalar::U32 => x as u32 as i128,
        Scalar::I32 => x as u32 as i32 as i128,
        Scalar::U64 => x as i128,
        Scalar::I64 => x as i64 as i128,
        Scalar::F32 => 0,
    };
    let float = from == Scalar::F32;
    match to {
        Scalar::Bool => if float { (f(x) != 0.0) as u64 } else { (as_int != 0) as u64 },
        Scalar::F32 => if float { x } else { fb(as_int as f32) },
        // Float to int rounds toward zero and saturates (NaN gives 0), as Rust's `as` does.
        Scalar::U32 => if float { f(x) as u32 as u64 } else { as_int as u32 as u64 },
        Scalar::I32 => if float { f(x) as i32 as u32 as u64 } else { as_int as i32 as u32 as u64 },
        Scalar::U64 => if float { f(x) as u64 } else { as_int as u64 },
        Scalar::I64 => if float { f(x) as i64 as u64 } else { as_int as i64 as u64 },
    }
}

fn identity(op: BinOp, s: Scalar) -> u64 {
    match (op, s) {
        (BinOp::Mul, Scalar::F32) => fb(1.0),
        (BinOp::Mul, _) => 1,
        (BinOp::And, Scalar::Bool) => 1,
        (BinOp::And, Scalar::U32 | Scalar::I32) => u32::MAX as u64,
        (BinOp::And, _) => u64::MAX,
        (BinOp::Min, Scalar::F32) => fb(f32::INFINITY),
        (BinOp::Min, Scalar::U32) => u32::MAX as u64,
        (BinOp::Min, Scalar::I32) => i32::MAX as u32 as u64,
        (BinOp::Min, Scalar::U64) => u64::MAX,
        (BinOp::Min, Scalar::I64) => i64::MAX as u64,
        (BinOp::Max, Scalar::F32) => fb(f32::NEG_INFINITY),
        (BinOp::Max, Scalar::I32) => i32::MIN as u32 as u64,
        (BinOp::Max, Scalar::I64) => i64::MIN as u64,
        _ => 0,
    }
}
