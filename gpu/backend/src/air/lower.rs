//! gpu's IR lowered onto AIR: one function per entry point, a kernel, a vertex function or a fragment function.
//!
//! - **Arguments**: each binding at its index n: a buffer is a `device uchar*` at `[[buffer(n)]]`, an image a
//!   `texture2d<float>` at `[[texture(n)]]`, a sampler at `[[sampler(n)]]`, a combined one both; then the builtins the
//!   program reads, with their descriptors.
//! - **Vertex functions** fetch their own attributes (vertex pulling): the vertex buffers are buffers after the
//!   bindings (`[[buffer(bindings + k)]]`), read at `vertex_id` or `instance_id` times their stride by the pipeline's
//!   layout ([`VertexInput`]), each format widened to what the shader reads as Vulkan does (missing lanes 0, alpha 1).
//!   Their outputs are returned as a packed struct: the position (its y negated, Vulkan's clip space having y down and
//!   Metal's up) and each location's `user(locnN)`.
//! - **Fragment functions** take each input as an interpolated argument (`air.perspective`, `air.no_perspective`,
//!   `air.flat`), the position and facing as builtins, and return their colour attachments' values and depth; they
//!   sample with `air.sample_texture_2d`, take derivatives with `air.dfdx` and `air.dfdy`, and `air.discard_fragment`.
//! - **Memory is bytes**: a pointer is `i8 addrspace(n)*`, moved with a byte `getelementptr`, and cast to the type
//!   each load or store reads. A 3-lane vector is read and written a lane at a time, as xcrun does for `packed_`
//!   vectors, so it touches its 12 bytes and no more. Workgroup regions are threadgroup globals; private regions,
//!   variables, and values used outside the block that defines them are allocas, which Metal's compiler turns back
//!   into registers.
//! - **Control flow**: each `If` and `Loop` becomes branches between blocks; `Break`, `Continue` and `Return` are
//!   branches to the loop's exit or continuing block, and `ret`.
//! - **The IR's rules**: integer division and remainder by zero give 0 (and the signed overflow case its wrapped
//!   answer), shift counts are taken modulo the width, float to integer conversions are AIR's saturating
//!   `air.convert`, and floats round as IEEE says: no fast-math flags, and `air.compile.fast_math_disable`.
//! - **Barriers** are `air.wg.barrier`; a barrier that only orders memory is `air.atomic.fence`, which needs MSL 3.2
//!   and macOS 15, so a module that uses one says so.

use gpu_ir::{AtomicOp, BinOp, Block, Builtin, Cmp, Entry, Op as IrOp, Scalar, Space, Stmt, SubgroupOp, Type, UnOp};

use super::module::{Arg, CONSTANT, Const, DEVICE, Inst, Module, Op, Output, Param, Stage as AirStage, THREAD, THREADGROUP, Ty, TypeId, Value, attr};
use crate::{Error, err};

/// The kernel function's name in every library this writes.
pub const FUNCTION: &str = "carbon_entry";

// LLVM's opcodes, as bitcode numbers them.
const ADD: u64 = 0;
const SUB: u64 = 1;
const MUL: u64 = 2;
const UDIV: u64 = 3;
const SDIV: u64 = 4;
const UREM: u64 = 5;
const SREM: u64 = 6;
const SHL: u64 = 7;
const LSHR: u64 = 8;
const ASHR: u64 = 9;
const AND: u64 = 10;
const OR: u64 = 11;
const XOR: u64 = 12;
const FDIV: u64 = 4;

const TRUNC: u64 = 0;
const ZEXT: u64 = 1;
const SEXT: u64 = 2;
const BITCAST: u64 = 11;

const FCMP_OEQ: u64 = 1;
const FCMP_OGT: u64 = 2;
const FCMP_OGE: u64 = 3;
const FCMP_OLT: u64 = 4;
const FCMP_OLE: u64 = 5;
const FCMP_UNE: u64 = 14;
const ICMP_EQ: u64 = 32;
const ICMP_NE: u64 = 33;
const ICMP_UGT: u64 = 34;
const ICMP_UGE: u64 = 35;
const ICMP_ULT: u64 = 36;
const ICMP_ULE: u64 = 37;
const ICMP_SGT: u64 = 38;
const ICMP_SGE: u64 = 39;
const ICMP_SLT: u64 = 40;
const ICMP_SLE: u64 = 41;

// Barrier flags, memory orders and scopes, as the intrinsics take them.
const MEM_DEVICE: u32 = 1;
const MEM_THREADGROUP: u32 = 2;
const SEQ_CST: u32 = 5;
const SCOPE_THREADGROUP: u32 = 1;
const SCOPE_DEVICE: u32 = 2;

/// An IEEE single's 1.0.
const ONE_F32: u64 = 0x3F80_0000;

fn builtin_arg(b: Builtin) -> (&'static str, &'static str, &'static str) {
    match b {
        Builtin::GlobalId => ("gid", "air.thread_position_in_grid", "uint3"),
        Builtin::LocalId => ("lid", "air.thread_position_in_threadgroup", "uint3"),
        Builtin::GroupId => ("wid", "air.threadgroup_position_in_grid", "uint3"),
        Builtin::GroupCount => ("nwg", "air.threadgroups_per_grid", "uint3"),
        Builtin::LocalIndex => ("lidx", "air.thread_index_in_threadgroup", "uint"),
        Builtin::SubgroupLane => ("sglane", "air.thread_index_in_simdgroup", "uint"),
        Builtin::SubgroupSize => ("sgsize", "air.threads_per_simdgroup", "uint"),
        Builtin::SubgroupId => ("sgid", "air.simdgroup_index_in_threadgroup", "uint"),
        Builtin::SubgroupCount => ("nsg", "air.simdgroups_per_threadgroup", "uint"),
        // The graphics stages' builtins: refused with their stages, in `lower`.
        Builtin::VertexIndex => ("vid", "air.vertex_id", "uint"),
        Builtin::InstanceIndex => ("iid", "air.instance_id", "uint"),
        Builtin::FragCoord => ("pos", "air.position", "float4"),
        Builtin::FrontFacing => ("front", "air.front_facing", "bool"),
    }
}

/// A vertex function's attributes: where each location's data is in which vertex buffer, and in what format.
#[derive(Debug, Clone, Default)]
pub struct VertexInput {
    pub buffers: Vec<VertexBuffer>,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, Copy)]
pub struct VertexBuffer {
    pub stride: u32,
    pub per_instance: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Attribute {
    pub location: u32,
    pub buffer: u32,
    pub offset: u32,
    pub format: Format,
}

/// An attribute's format in its buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    F32(u8),
    U32(u8),
    I32(u8),
    /// Four bytes, each `n / 255`.
    Unorm8x4,
}

/// A value's MSL type name, for a descriptor: `float4`, `uint`.
fn type_name(t: Type) -> &'static str {
    const F: [&str; 5] = ["float", "float", "float2", "float3", "float4"];
    const U: [&str; 5] = ["uint", "uint", "uint2", "uint3", "uint4"];
    const I: [&str; 5] = ["int", "int", "int2", "int3", "int4"];
    let (s, n) = match t {
        Type::Scalar(s) => (s, 1),
        Type::Vector(s, n) => (s, n as usize),
        Type::Ptr(_) => return "uchar",
    };
    match s {
        Scalar::F32 => F[n],
        Scalar::I32 | Scalar::I64 => I[n],
        Scalar::Bool => "bool",
        _ => U[n],
    }
}

fn space_number(s: Space) -> u32 {
    match s {
        Space::Device => DEVICE,
        Space::Workgroup => THREADGROUP,
        Space::Private => THREAD,
    }
}

/// The type part of an overloaded intrinsic's name: `f32`, `i32`, `v4f32`.
fn suffix(t: Type) -> String {
    let s = match t.scalar() {
        Some(Scalar::F32) => "f32",
        Some(Scalar::U32 | Scalar::I32) => "i32",
        Some(Scalar::U64 | Scalar::I64) => "i64",
        _ => "i1",
    };
    match t {
        Type::Vector(_, n) => format!("v{n}{s}"),
        _ => s.into(),
    }
}

/// The sign an intrinsic on integers says: `s.` or `u.`; nothing for floats.
fn sign(s: Scalar) -> &'static str {
    match s {
        Scalar::I32 | Scalar::I64 => "s.",
        Scalar::U32 | Scalar::U64 | Scalar::Bool => "u.",
        Scalar::F32 => "",
    }
}

/// The IR values an operation reads.
fn operands(op: &IrOp) -> Vec<gpu_ir::Value> {
    match op {
        IrOp::Const(..) | IrOp::Builtin(_) | IrOp::Binding(_) | IrOp::Workgroup(_) | IrOp::Private(_) | IrOp::Read(_) => {
            Vec::new()
        }
        IrOp::Load(p, _) | IrOp::Unary(_, p) | IrOp::Convert(p, _) | IrOp::Bitcast(p, _) | IrOp::Extract(p, _) => vec![*p],
        IrOp::Offset(a, b) | IrOp::Binary(_, a, b) | IrOp::Compare(_, a, b) | IrOp::ExtractDynamic(a, b) => vec![*a, *b],
        IrOp::Insert(a, _, b) | IrOp::Atomic(_, a, b, _) => vec![*a, *b],
        IrOp::Select(a, b, c) | IrOp::CompareExchange(a, b, c, _) => vec![*a, *b, *c],
        IrOp::Construct(parts) => parts.clone(),
        IrOp::Subgroup(_, x, lane) => x.iter().chain(lane.iter()).copied().collect(),
        IrOp::Input(_) => Vec::new(),
        IrOp::Derivative(_, x) => vec![*x],
        IrOp::Sample { coord, lod, .. } => match lod {
            gpu_ir::Lod::Bias(v) | gpu_ir::Lod::Level(v) => vec![*coord, *v],
            gpu_ir::Lod::Implicit => vec![*coord],
        },
    }
}

/// Which values are used outside the block that defines them (a value defined in a loop's body and read after the
/// loop, or in its continuing block): those live in allocas, since their definitions needn't dominate those uses.
fn escaping(e: &Entry) -> Vec<bool> {
    struct Walk {
        def: Vec<u32>,
        escapes: Vec<bool>,
        stack: Vec<u32>,
        next: u32,
    }
    impl Walk {
        fn uses(&mut self, v: gpu_ir::Value) {
            let i = v.0 as usize;
            if i < self.def.len() && !self.stack.contains(&self.def[i]) {
                self.escapes[i] = true;
            }
        }
        fn scope(&mut self, block: &Block, then: Option<gpu_ir::Value>) {
            self.stack.push(self.next);
            self.next += 1;
            self.block(block);
            if let Some(v) = then {
                self.uses(v);
            }
            self.stack.pop();
        }
        fn block(&mut self, block: &Block) {
            for stmt in block {
                match stmt {
                    Stmt::Let(v, op) => {
                        for u in operands(op) {
                            self.uses(u);
                        }
                        if let Some(d) = self.def.get_mut(v.0 as usize) {
                            *d = *self.stack.last().unwrap_or(&0);
                        }
                    }
                    Stmt::Store(p, x) => {
                        self.uses(*p);
                        self.uses(*x);
                    }
                    Stmt::Write(_, x) | Stmt::Output(_, x) => self.uses(*x),
                    Stmt::If { cond, then, otherwise } => {
                        self.uses(*cond);
                        self.scope(then, None);
                        self.scope(otherwise, None);
                    }
                    Stmt::Loop { body, continuing, break_if } => {
                        self.scope(body, None);
                        self.scope(continuing, *break_if);
                    }
                    Stmt::Break | Stmt::Continue | Stmt::Return | Stmt::Barrier(_) | Stmt::Discard => {}
                }
            }
        }
    }
    let n = e.values.len();
    let mut w = Walk { def: vec![u32::MAX; n], escapes: vec![false; n], stack: vec![0], next: 1 };
    w.block(&e.body);
    w.escapes
}

fn builtins_used(block: &Block, out: &mut Vec<Builtin>) {
    for stmt in block {
        match stmt {
            Stmt::Let(_, IrOp::Builtin(b)) if !out.contains(b) => out.push(*b),
            Stmt::If { then, otherwise, .. } => {
                builtins_used(then, out);
                builtins_used(otherwise, out);
            }
            Stmt::Loop { body, continuing, .. } => {
                builtins_used(body, out);
                builtins_used(continuing, out);
            }
            _ => {}
        }
    }
}

fn uses_compare_exchange(block: &Block) -> bool {
    block.iter().any(|stmt| match stmt {
        Stmt::Let(_, IrOp::CompareExchange(..)) => true,
        Stmt::If { then, otherwise, .. } => uses_compare_exchange(then) || uses_compare_exchange(otherwise),
        Stmt::Loop { body, continuing, .. } => uses_compare_exchange(body) || uses_compare_exchange(continuing),
        _ => false,
    })
}

struct Lower<'a> {
    e: &'a Entry,
    m: Module,
    /// The block instructions go into.
    cur: u32,
    /// Each IR value's AIR value, once defined.
    vals: Vec<Option<Value>>,
    /// The alloca of each value used outside its block.
    slots: Vec<Option<Value>>,
    vars: Vec<Value>,
    workgroup: Vec<Value>,
    private: Vec<Value>,
    builtins: Vec<(Builtin, Value)>,
    /// The innermost loop's continuing and exit blocks.
    loops: Vec<(u32, u32)>,
    /// An `i32` for compare-exchange's expected value.
    expected: Option<Value>,
    /// Each binding's argument: a buffer's or an image's, and a sampler's.
    bindings: Vec<(Option<Value>, Option<Value>)>,
    /// A vertex function's attributes and vertex buffers, and its vertex and instance index.
    vertex: Option<&'a VertexInput>,
    vertex_buffers: Vec<Value>,
    /// A fragment function's input arguments, by input.
    inputs: Vec<Value>,
    /// Where each output is kept until the function returns, and the struct it returns (None: nothing).
    outputs: Vec<Value>,
    ret: Option<TypeId>,
}

/// The entry point as an AIR module; a vertex shader's with its attributes' layout.
pub(crate) fn lower(e: &Entry, vertex: Option<&VertexInput>) -> Result<Module, Error> {
    if e.stage == gpu_ir::Stage::Vertex && vertex.is_none() {
        return err(format!("{}: a vertex shader is written with its pipeline's vertex input", e.name));
    }
    let mut l = Lower {
        e,
        m: Module::new(FUNCTION),
        cur: 0,
        vals: vec![None; e.values.len()],
        slots: vec![None; e.values.len()],
        vars: Vec::new(),
        workgroup: Vec::new(),
        private: Vec::new(),
        builtins: Vec::new(),
        loops: Vec::new(),
        expected: None,
        bindings: Vec::new(),
        vertex,
        vertex_buffers: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        ret: None,
    };
    l.m.stage = match e.stage {
        gpu_ir::Stage::Compute => AirStage::Kernel,
        gpu_ir::Stage::Vertex => AirStage::Vertex,
        gpu_ir::Stage::Fragment => AirStage::Fragment,
    };
    l.prologue()?;
    l.block(&e.body)?;
    if !l.terminated() {
        l.finish();
    }
    Ok(l.m)
}

impl Lower<'_> {
    // ---- Types and building -------------------------------------------------------------------------------------

    fn scalar_ty(&mut self, s: Scalar) -> TypeId {
        match s {
            Scalar::Bool => self.m.int(1),
            Scalar::U32 | Scalar::I32 => self.m.int(32),
            Scalar::U64 | Scalar::I64 => self.m.int(64),
            Scalar::F32 => self.m.ty(Ty::Float),
        }
    }

    fn ty(&mut self, t: Type) -> TypeId {
        match t {
            Type::Scalar(s) => self.scalar_ty(s),
            Type::Vector(s, n) => {
                let e = self.scalar_ty(s);
                self.m.ty(Ty::Vector(e, n as u32))
            }
            Type::Ptr(space) => {
                let i8 = self.m.int(8);
                self.m.ptr(i8, space_number(space))
            }
        }
    }

    /// The natural alignment of a value of `t` in an alloca.
    fn align_of(&self, t: TypeId) -> u32 {
        match self.m.get(t) {
            Ty::Int(b) => (b / 8).max(1),
            Ty::Float => 4,
            Ty::Ptr(..) => 8,
            Ty::Vector(e, n) => self.align_of(*e) * n.next_power_of_two(),
            Ty::Array(e, _) => self.align_of(*e),
            Ty::Struct(elems, _) => elems.iter().map(|e| self.align_of(*e)).max().unwrap_or(1),
            Ty::Void | Ty::Func(..) | Ty::Opaque(_) => 1,
        }
    }

    fn terminated(&self) -> bool {
        let body = self.m.body();
        body.blocks[self.cur as usize].last().is_some_and(|&i| body.insts[i as usize].terminates())
    }

    fn new_block(&mut self) -> u32 {
        let body = self.m.body_mut();
        body.blocks.push(Vec::new());
        body.blocks.len() as u32 - 1
    }

    /// Adds an instruction to the current block; after a terminator, to a new block nothing branches to.
    fn emit(&mut self, ty: TypeId, op: Op) -> Value {
        if self.terminated() {
            self.cur = self.new_block();
        }
        let cur = self.cur as usize;
        let body = self.m.body_mut();
        let i = body.insts.len() as u32;
        body.insts.push(Inst { ty, op });
        body.blocks[cur].push(i);
        Value::Inst(i)
    }

    /// Branches to `to` unless the block already ended.
    fn jump(&mut self, to: u32) {
        if !self.terminated() {
            let void = self.m.void();
            self.emit(void, Op::Br(to));
        }
    }

    fn type_of(&self, v: Value) -> TypeId {
        self.m.type_of(v)
    }

    fn bin(&mut self, code: u64, a: Value, b: Value) -> Value {
        let t = self.type_of(a);
        self.emit(t, Op::Binary(code, a, b))
    }

    fn cast(&mut self, code: u64, v: Value, to: TypeId) -> Value {
        if self.type_of(v) == to {
            return v;
        }
        self.emit(to, Op::Cast(code, v))
    }

    fn cmp(&mut self, pred: u64, a: Value, b: Value) -> Value {
        let i1 = self.m.int(1);
        let t = self.m.shaped(self.type_of(a), i1);
        self.emit(t, Op::Cmp(pred, a, b))
    }

    fn select(&mut self, c: Value, a: Value, b: Value) -> Value {
        let t = self.type_of(a);
        self.emit(t, Op::Select(c, a, b))
    }

    fn load(&mut self, ptr: Value, ty: TypeId, align: u32) -> Value {
        self.emit(ty, Op::Load { ptr, align })
    }

    fn store(&mut self, ptr: Value, value: Value, align: u32) {
        let void = self.m.void();
        self.emit(void, Op::Store { ptr, value, align });
    }

    fn alloca(&mut self, elem: TypeId, align: u32) -> Value {
        let ty = self.m.ptr(elem, THREAD);
        let count = self.m.i32(1);
        self.emit(ty, Op::Alloca { elem, count, align })
    }

    /// `p` moved by `n` bytes (an `i64`).
    fn gep(&mut self, p: Value, n: Value) -> Value {
        let t = self.type_of(p);
        let elem = self.m.int(8);
        self.emit(t, Op::Gep { elem, ptr: p, index: n })
    }

    fn i64(&mut self, v: u64) -> Value {
        let t = self.m.int(64);
        self.m.konst(Const::Int(t, v))
    }

    fn call(&mut self, name: &str, ret: TypeId, args: Vec<Value>, attrs: &'static [u64]) -> Value {
        let params = args.iter().map(|a| self.type_of(*a)).collect();
        let func = self.m.declare(name, ret, params, attrs);
        self.emit(ret, Op::Call { func, args })
    }

    fn space_of(&self, p: Value) -> u32 {
        match self.m.get(self.type_of(p)) {
            Ty::Ptr(_, s) => *s,
            _ => THREAD,
        }
    }

    /// `p` as a pointer to `elem` in its own address space.
    fn retype(&mut self, p: Value, elem: TypeId) -> Value {
        let space = self.space_of(p);
        let t = self.m.ptr(elem, space);
        self.cast(BITCAST, p, t)
    }

    // ---- The kernel's arguments and its allocas -----------------------------------------------------------------

    fn prologue(&mut self) -> Result<(), Error> {
        use gpu_ir::BindingKind;
        let e = self.e;
        let i8 = self.m.int(8);
        let device_ptr = self.m.ptr(i8, DEVICE);
        let texture = self.m.ty(Ty::Opaque("struct._texture_2d_t".into()));
        let texture = self.m.ptr(texture, DEVICE);
        let sampler = self.m.ty(Ty::Opaque("struct._sampler_t".into()));
        let sampler = self.m.ptr(sampler, CONSTANT);
        let mut params = Vec::new();
        let next = |params: &Vec<Param>| Value::Param(params.len() as u32);
        for (i, b) in e.bindings.iter().enumerate() {
            let location = i as u32;
            let (mut main, mut samp) = (None, None);
            if b.kind.is_buffer() {
                let writable = matches!(b.kind, BindingKind::Storage { writable: true });
                main = Some(next(&params));
                params.push(Param { ty: device_ptr, name: format!("b{i}"), arg: Arg::Buffer { location, writable } });
            }
            if matches!(b.kind, BindingKind::Image | BindingKind::Combined) {
                main = Some(next(&params));
                params.push(Param { ty: texture, name: format!("t{i}"), arg: Arg::Texture { location } });
            }
            if matches!(b.kind, BindingKind::Sampler | BindingKind::Combined) {
                samp = Some(next(&params));
                params.push(Param { ty: sampler, name: format!("s{i}"), arg: Arg::Sampler { location } });
            }
            self.bindings.push((main, samp));
        }
        let mut used = Vec::new();
        builtins_used(&e.body, &mut used);
        if let Some(input) = self.vertex {
            // Fetching attributes takes the vertex and instance index, and the vertex buffers.
            if !e.inputs.is_empty() {
                for b in [Builtin::VertexIndex, Builtin::InstanceIndex] {
                    if !used.contains(&b) {
                        used.push(b);
                    }
                }
            }
            for k in 0..input.buffers.len() {
                let location = (e.bindings.len() + k) as u32;
                self.vertex_buffers.push(next(&params));
                params.push(Param { ty: device_ptr, name: format!("vb{k}"), arg: Arg::Buffer { location, writable: false } });
            }
            if e.bindings.len() + input.buffers.len() > 31 {
                return err(format!("{}: more bindings and vertex buffers than Metal's 31 buffers", e.name));
            }
        }
        if e.stage == gpu_ir::Stage::Fragment {
            for io in &e.inputs {
                let gpu_ir::Slot::Location(location) = io.slot else { return err(format!("{}: a fragment input that isn't at a location", e.name)) };
                let how: &'static [&'static str] = match io.interpolation {
                    gpu_ir::Interpolation::Smooth => &["air.center", "air.perspective"],
                    gpu_ir::Interpolation::NoPerspective => &["air.center", "air.no_perspective"],
                    gpu_ir::Interpolation::Flat => &["air.flat"],
                };
                let ty = self.ty(io.ty);
                self.inputs.push(next(&params));
                params.push(Param { ty, name: format!("in{location}"), arg: Arg::Interpolated { location, how, type_name: type_name(io.ty) } });
            }
        }
        for b in used {
            let ty = self.ty(b.ty());
            let (name, desc, type_name) = builtin_arg(b);
            self.builtins.push((b, next(&params)));
            let arg = if b == Builtin::FragCoord { Arg::FragmentPosition } else { Arg::Builtin { name: desc, type_name } };
            params.push(Param { ty, name: name.into(), arg });
        }
        // A vertex or fragment function returns its outputs, in order, as a packed struct.
        let mut members = Vec::new();
        for io in &e.outputs {
            members.push(self.ty(io.ty));
            self.m.outputs.push(match (e.stage, io.slot) {
                (_, gpu_ir::Slot::Position) => Output::Position,
                (gpu_ir::Stage::Vertex, gpu_ir::Slot::Location(location)) => Output::Vertex { location, type_name: type_name(io.ty) },
                (_, gpu_ir::Slot::Location(index)) => Output::Color { index, type_name: type_name(io.ty) },
                (_, gpu_ir::Slot::FragDepth) => Output::Depth,
            });
        }
        if members.is_empty() {
            self.m.set_params(params);
        } else {
            let ret = self.m.ty(Ty::Struct(members, true));
            self.ret = Some(ret);
            self.m.set_signature(ret, params);
        }
        self.cur = self.new_block();
        for io in &e.outputs {
            let ty = self.ty(io.ty);
            let align = self.align_of(ty);
            let a = self.alloca(ty, align);
            let zero = self.m.null(ty);
            self.store(a, zero, align);
            self.outputs.push(a);
        }

        for (i, r) in e.workgroup.iter().enumerate() {
            let elem = self.m.ty(Ty::Array(i8, r.size.max(1) as u64));
            let g = self.m.global(format!("carbon_workgroup{i}"), elem, THREADGROUP, r.align.max(16));
            let p = self.m.ptr(i8, THREADGROUP);
            let p = self.cast(BITCAST, g, p);
            self.workgroup.push(p);
        }
        for r in &e.private {
            let elem = self.m.ty(Ty::Array(i8, r.size.max(1) as u64));
            let a = self.alloca(elem, r.align.max(16));
            let p = self.m.ptr(i8, THREAD);
            let p = self.cast(BITCAST, a, p);
            // Zeroed, as the CPU device zeroes it.
            let (void, i1, i64) = (self.m.void(), self.m.int(1), self.m.int(64));
            let zero = self.m.konst(Const::Int(i8, 0));
            let size = self.m.konst(Const::Int(i64, r.size as u64));
            let no = self.m.konst(Const::Int(i1, 0));
            self.call("llvm.memset.p0i8.i64", void, vec![p, zero, size, no], attr::MEMSET);
            self.private.push(p);
        }
        for t in e.vars.clone() {
            let ty = self.ty(t);
            let align = self.align_of(ty);
            let a = self.alloca(ty, align);
            let zero = self.m.null(ty);
            self.store(a, zero, align);
            self.vars.push(a);
        }
        for (i, escapes) in escaping(e).into_iter().enumerate() {
            if escapes {
                let ty = self.ty(e.values[i]);
                let align = self.align_of(ty);
                self.slots[i] = Some(self.alloca(ty, align));
            }
        }
        if uses_compare_exchange(&e.body) {
            let i32 = self.m.int(32);
            self.expected = Some(self.alloca(i32, 4));
        }
        Ok(())
    }

    /// Returns: nothing from a kernel; a vertex or fragment function's outputs, as its struct.
    fn finish(&mut self) {
        let Some(ret) = self.ret else {
            let void = self.m.void();
            self.emit(void, Op::Ret);
            return;
        };
        let mut agg = self.m.undef(ret);
        for (i, io) in self.e.outputs.clone().iter().enumerate() {
            let ty = self.ty(io.ty);
            let align = self.align_of(ty);
            let mut x = self.load(self.outputs[i], ty, align);
            if io.slot == gpu_ir::Slot::Position && self.e.stage == gpu_ir::Stage::Vertex {
                let f = self.m.ty(Ty::Float);
                let one = self.m.i32(1);
                let y = self.emit(f, Op::ExtractElement(x, one));
                let y = self.emit(f, Op::FNeg(y));
                x = self.emit(ty, Op::InsertElement(x, y, one));
            }
            agg = self.emit(ret, Op::InsertValue(agg, x, i as u32));
        }
        let void = self.m.void();
        self.emit(void, Op::RetValue(agg));
    }

    /// A vertex attribute, fetched from its vertex buffer at this vertex (or instance), widened to `want` as Vulkan
    /// does: lanes the format hasn't are 0, and the fourth 1.
    fn fetch(&mut self, input: usize, want: Type) -> Result<Value, Error> {
        let e = self.e;
        let gpu_ir::Slot::Location(location) = e.inputs[input].slot else { return err("a vertex input that isn't at a location") };
        let layout = self.vertex.ok_or_else(|| Error { message: "a vertex input with no layout".into() })?;
        let a = *layout.attributes.iter().find(|a| a.location == location).ok_or_else(|| Error { message: format!("{}: no vertex attribute at location {location}", e.name) })?;
        let vb = *layout.buffers.get(a.buffer as usize).ok_or_else(|| Error { message: format!("vertex buffer {} isn't in the layout", a.buffer) })?;
        let which = if vb.per_instance { Builtin::InstanceIndex } else { Builtin::VertexIndex };
        let index = self.builtins.iter().find(|(b, _)| *b == which).map(|(_, v)| *v).ok_or_else(|| Error { message: "no vertex index".into() })?;
        let i64 = self.m.int(64);
        let index = self.cast(ZEXT, index, i64);
        let stride = self.i64(vb.stride as u64);
        let at = self.bin(MUL, index, stride);
        let offset = self.i64(a.offset as u64);
        let at = self.bin(ADD, at, offset);
        let p = self.gep(self.vertex_buffers[a.buffer as usize], at);
        let (lanes, got) = match a.format {
            Format::F32(n) => (n as u32, self.mem_load(p, if n == 1 { Type::F32 } else { Type::Vector(Scalar::F32, n) })),
            Format::U32(n) => (n as u32, self.mem_load(p, if n == 1 { Type::U32 } else { Type::Vector(Scalar::U32, n) })),
            Format::I32(n) => (n as u32, self.mem_load(p, if n == 1 { Type::I32 } else { Type::Vector(Scalar::I32, n) })),
            Format::Unorm8x4 => {
                let word = self.mem_load(p, Type::U32);
                let (f, v4) = (self.m.ty(Ty::Float), self.ty(Type::Vector(Scalar::F32, 4)));
                let i32 = self.m.int(32);
                let mut v = self.m.undef(v4);
                for lane in 0..4u32 {
                    let shift = self.m.i32(lane * 8);
                    let byte = self.bin(LSHR, word, shift);
                    let mask = self.m.i32(0xff);
                    let byte = self.bin(AND, byte, mask);
                    let x = self.call("air.convert.f.f32.u.i32", f, vec![byte], attr::PURE);
                    let k = self.m.konst(Const::Float(f, 255.0f32.to_bits()));
                    let x = self.bin(FDIV, x, k);
                    let l = self.m.i32(lane);
                    v = self.emit(v4, Op::InsertElement(v, x, l));
                }
                let _ = i32;
                (4, v)
            }
        };
        let want_lanes = match want {
            Type::Vector(_, n) => n as u32,
            _ => 1,
        };
        if want_lanes == lanes {
            return Ok(got);
        }
        let scalar = want.scalar().unwrap_or(Scalar::F32);
        let st = self.scalar_ty(scalar);
        let one = if scalar == Scalar::F32 { ONE_F32 } else { 1 };
        let lane_of = |l: &mut Self, i: u32| -> Value {
            if i < lanes {
                if lanes == 1 {
                    return got;
                }
                let idx = l.m.i32(i);
                l.emit(st, Op::ExtractElement(got, idx))
            } else {
                l.m.splat(st, if i == 3 { one } else { 0 })
            }
        };
        if want_lanes == 1 {
            return Ok(lane_of(self, 0));
        }
        let wt = self.ty(want);
        let mut v = self.m.undef(wt);
        for i in 0..want_lanes {
            let x = lane_of(self, i);
            let idx = self.m.i32(i);
            v = self.emit(wt, Op::InsertElement(v, x, idx));
        }
        Ok(v)
    }

    // ---- Values -------------------------------------------------------------------------------------------------

    fn val(&mut self, v: gpu_ir::Value) -> Result<Value, Error> {
        let i = v.0 as usize;
        if let Some(Some(slot)) = self.slots.get(i).copied() {
            let ty = self.ty(self.e.ty(v));
            let align = self.align_of(ty);
            return Ok(self.load(slot, ty, align));
        }
        match self.vals.get(i).copied().flatten() {
            Some(x) => Ok(x),
            None => err(format!("{v} is used before it's defined")),
        }
    }

    fn define(&mut self, v: gpu_ir::Value, x: Value) {
        let i = v.0 as usize;
        self.vals[i] = Some(x);
        if let Some(slot) = self.slots[i] {
            let align = self.align_of(self.type_of(x));
            self.store(slot, x, align);
        }
    }

    // ---- Statements ---------------------------------------------------------------------------------------------

    fn block(&mut self, stmts: &Block) -> Result<(), Error> {
        for stmt in stmts {
            self.stmt(stmt)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), Error> {
        let void = self.m.void();
        match stmt {
            Stmt::Let(v, op) => {
                let x = self.op(*v, op)?;
                self.define(*v, x);
            }
            Stmt::Store(p, x) => {
                let (p, x, t) = (self.val(*p)?, self.val(*x)?, self.e.ty(*x));
                self.mem_store(p, x, t);
            }
            Stmt::Write(var, x) => {
                let x = self.val(*x)?;
                let a = self.vars[var.0 as usize];
                let align = self.align_of(self.type_of(x));
                self.store(a, x, align);
            }
            Stmt::If { cond, then, otherwise } => {
                let c = self.val(*cond)?;
                let then_b = self.new_block();
                let else_b = if otherwise.is_empty() { None } else { Some(self.new_block()) };
                let merge = self.new_block();
                self.emit(void, Op::CondBr(c, then_b, else_b.unwrap_or(merge)));
                self.cur = then_b;
                self.block(then)?;
                self.jump(merge);
                if let Some(b) = else_b {
                    self.cur = b;
                    self.block(otherwise)?;
                    self.jump(merge);
                }
                self.cur = merge;
            }
            Stmt::Loop { body, continuing, break_if } => {
                let body_b = self.new_block();
                let cont = self.new_block();
                let exit = self.new_block();
                self.jump(body_b);
                self.cur = body_b;
                self.loops.push((cont, exit));
                self.block(body)?;
                self.jump(cont);
                self.cur = cont;
                self.block(continuing)?;
                match break_if {
                    Some(c) => {
                        let c = self.val(*c)?;
                        self.emit(void, Op::CondBr(c, exit, body_b));
                    }
                    None => self.jump(body_b),
                }
                self.loops.pop();
                self.cur = exit;
            }
            Stmt::Break | Stmt::Continue => {
                let Some(&(cont, exit)) = self.loops.last() else {
                    return err(format!("{stmt:?} outside a loop"));
                };
                let to = if matches!(stmt, Stmt::Break) { exit } else { cont };
                self.emit(void, Op::Br(to));
            }
            Stmt::Return => self.finish(),
            Stmt::Output(i, x) => {
                let x = self.val(*x)?;
                let align = self.align_of(self.type_of(x));
                let slot = *self.outputs.get(*i as usize).ok_or_else(|| Error { message: format!("output {i} isn't one of the entry point's") })?;
                self.store(slot, x, align);
            }
            Stmt::Discard => {
                self.call("air.discard_fragment", void, Vec::new(), attr::DISCARD);
            }
            Stmt::Barrier(b) => {
                let mut flags = 0;
                if b.device_memory {
                    flags |= MEM_DEVICE;
                }
                if b.workgroup_memory {
                    flags |= MEM_THREADGROUP;
                }
                if b.execution {
                    let (f, one) = (self.m.i32(flags), self.m.i32(1));
                    self.call("air.wg.barrier", void, vec![f, one], attr::CONVERGENT_SET);
                } else if flags != 0 {
                    // MSL 3.2's atomic_thread_fence: seq_cst, for the whole device.
                    self.m.language = self.m.language.max((3, 2));
                    self.m.os = self.m.os.max((15, 0));
                    let args = vec![self.m.i32(flags), self.m.i32(SEQ_CST), self.m.i32(SCOPE_DEVICE)];
                    self.call("air.atomic.fence", void, args, attr::ATOMIC);
                }
            }
        }
        Ok(())
    }

    // ---- Memory -------------------------------------------------------------------------------------------------

    /// The alignment a load or store of `t` assumes: its scalar's, since memory is bytes.
    fn mem_align(t: Type) -> u32 {
        match t {
            Type::Ptr(_) => 8,
            _ => t.scalar().map_or(1, |s| s.size()),
        }
    }

    fn mem_load(&mut self, p: Value, t: Type) -> Value {
        let ty = self.ty(t);
        let align = Self::mem_align(t);
        if let Type::Vector(s, 3) = t {
            let st = self.scalar_ty(s);
            let mut v = self.m.undef(ty);
            for lane in 0..3u32 {
                let off = self.i64(lane as u64 * s.size() as u64);
                let q = self.gep(p, off);
                let q = self.retype(q, st);
                let x = self.load(q, st, align);
                let i = self.m.i32(lane);
                v = self.emit(ty, Op::InsertElement(v, x, i));
            }
            return v;
        }
        let q = self.retype(p, ty);
        self.load(q, ty, align)
    }

    fn mem_store(&mut self, p: Value, x: Value, t: Type) {
        let align = Self::mem_align(t);
        if let Type::Vector(s, 3) = t {
            let st = self.scalar_ty(s);
            for lane in 0..3u32 {
                let i = self.m.i32(lane);
                let lx = self.emit(st, Op::ExtractElement(x, i));
                let off = self.i64(lane as u64 * s.size() as u64);
                let q = self.gep(p, off);
                let q = self.retype(q, st);
                self.store(q, lx, align);
            }
            return;
        }
        let ty = self.type_of(x);
        let q = self.retype(p, ty);
        self.store(q, x, align);
    }

    // ---- Operations ---------------------------------------------------------------------------------------------

    fn op(&mut self, value: gpu_ir::Value, op: &IrOp) -> Result<Value, Error> {
        let e = self.e;
        let rt = e.ty(value);
        let ty = self.ty(rt);
        Ok(match op {
            IrOp::Const(s, bits) => {
                let t = self.scalar_ty(*s);
                let bits = if *s == Scalar::Bool { u64::from(*bits != 0) } else { *bits };
                self.m.splat(t, bits)
            }
            IrOp::Builtin(b) => match self.builtins.iter().find(|(x, _)| x == b) {
                Some((_, v)) => *v,
                None => return err(format!("builtin {b:?} has no argument")),
            },
            IrOp::Binding(n) => self.bindings.get(*n as usize).and_then(|b| b.0).ok_or_else(|| Error { message: format!("binding {n} isn't a buffer") })?,
            IrOp::Workgroup(n) => self.workgroup[*n as usize],
            IrOp::Private(n) => self.private[*n as usize],
            IrOp::Offset(p, n) => {
                let (pv, nv) = (self.val(*p)?, self.val(*n)?);
                let s = e.ty(*n).scalar().unwrap_or(Scalar::U32);
                let i64 = self.m.int(64);
                let n = self.cast(if s.is_signed() { SEXT } else { ZEXT }, nv, i64);
                self.gep(pv, n)
            }
            IrOp::Load(p, t) => {
                let p = self.val(*p)?;
                self.mem_load(p, *t)
            }
            IrOp::Read(var) => {
                let a = self.vars[var.0 as usize];
                let align = self.align_of(ty);
                self.load(a, ty, align)
            }
            IrOp::Unary(u, a) => self.unary(*u, *a, rt)?,
            IrOp::Binary(b, a, c) => self.binary(*b, *a, *c)?,
            IrOp::Compare(c, a, b) => {
                let s = e.ty(*a).scalar().unwrap_or(Scalar::U64);
                let pred = match (s.is_float(), c) {
                    (true, Cmp::Eq) => FCMP_OEQ,
                    (true, Cmp::Ne) => FCMP_UNE,
                    (true, Cmp::Lt) => FCMP_OLT,
                    (true, Cmp::Le) => FCMP_OLE,
                    (true, Cmp::Gt) => FCMP_OGT,
                    (true, Cmp::Ge) => FCMP_OGE,
                    (false, Cmp::Eq) => ICMP_EQ,
                    (false, Cmp::Ne) => ICMP_NE,
                    (false, c) => match (s.is_signed(), c) {
                        (true, Cmp::Lt) => ICMP_SLT,
                        (true, Cmp::Le) => ICMP_SLE,
                        (true, Cmp::Gt) => ICMP_SGT,
                        (true, _) => ICMP_SGE,
                        (false, Cmp::Lt) => ICMP_ULT,
                        (false, Cmp::Le) => ICMP_ULE,
                        (false, Cmp::Gt) => ICMP_UGT,
                        (false, _) => ICMP_UGE,
                    },
                };
                let (a, b) = (self.val(*a)?, self.val(*b)?);
                self.cmp(pred, a, b)
            }
            IrOp::Select(c, a, b) => {
                let (c, a, b) = (self.val(*c)?, self.val(*a)?, self.val(*b)?);
                self.select(c, a, b)
            }
            IrOp::Convert(a, to) => self.convert(*a, *to)?,
            IrOp::Bitcast(a, t) => {
                // Between numbers of one size; the IR has no casts of pointers yet.
                let x = self.val(*a)?;
                let to = self.ty(*t);
                self.cast(BITCAST, x, to)
            }
            IrOp::Construct(parts) => {
                let mut v = self.m.undef(ty);
                for (i, p) in parts.iter().enumerate() {
                    let x = self.val(*p)?;
                    let i = self.m.i32(i as u32);
                    v = self.emit(ty, Op::InsertElement(v, x, i));
                }
                v
            }
            IrOp::Extract(x, i) => {
                let x = self.val(*x)?;
                let i = self.m.i32(*i as u32);
                self.emit(ty, Op::ExtractElement(x, i))
            }
            IrOp::ExtractDynamic(x, i) => {
                // Out of range (negative included) reads lane 0.
                let n = e.ty(*x).lanes() as u64;
                let (x, i) = (self.val(*x)?, self.val(*i)?);
                let it = self.type_of(i);
                let (limit, zero) = (self.m.splat(it, n), self.m.splat(it, 0));
                let inside = self.cmp(ICMP_ULT, i, limit);
                let i = self.select(inside, i, zero);
                self.emit(ty, Op::ExtractElement(x, i))
            }
            IrOp::Insert(x, i, y) => {
                let (x, y) = (self.val(*x)?, self.val(*y)?);
                let i = self.m.i32(*i as u32);
                self.emit(ty, Op::InsertElement(x, y, i))
            }
            IrOp::Atomic(aop, p, x, s) => {
                let (p, x) = (self.val(*p)?, self.val(*x)?);
                let (p, space, scope) = self.atomic_ptr(p, *s)?;
                let name = match aop {
                    AtomicOp::Exchange => format!("air.atomic.{space}.xchg.i32"),
                    op => {
                        let op = match op {
                            AtomicOp::Add => "add",
                            AtomicOp::Sub => "sub",
                            AtomicOp::Min => "min",
                            AtomicOp::Max => "max",
                            AtomicOp::And => "and",
                            AtomicOp::Or => "or",
                            _ => "xor",
                        };
                        format!("air.atomic.{space}.{op}.{}i32", sign(*s))
                    }
                };
                let (relaxed, scope, volatile) = (self.m.i32(0), self.m.i32(scope), self.bool(true));
                self.call(&name, ty, vec![p, x, relaxed, scope, volatile], attr::ATOMIC)
            }
            IrOp::CompareExchange(p, c, x, s) => {
                let (p, c, x) = (self.val(*p)?, self.val(*c)?, self.val(*x)?);
                let (p, space, scope) = self.atomic_ptr(p, *s)?;
                let Some(expected) = self.expected else { return err("no slot for compare-exchange") };
                self.store(expected, c, 4);
                // It gives the value it found, which equals `c` exactly when it stored `x`.
                let (relaxed, scope, volatile) = (self.m.i32(0), self.m.i32(scope), self.bool(true));
                let name = format!("air.atomic.{space}.cmpxchg.weak.i32");
                self.call(&name, ty, vec![p, expected, x, relaxed, relaxed, scope, volatile], attr::ATOMIC)
            }
            IrOp::Subgroup(sop, x, lane) => self.subgroup(*sop, *x, *lane)?,
            IrOp::Input(i) => match e.stage {
                gpu_ir::Stage::Vertex => self.fetch(*i as usize, rt)?,
                gpu_ir::Stage::Fragment => *self.inputs.get(*i as usize).ok_or_else(|| Error { message: format!("input {i} isn't one of the entry point's") })?,
                gpu_ir::Stage::Compute => return err(format!("{op:?} in a compute kernel")),
            },
            IrOp::Derivative(axis, x) => {
                let x = self.val(*x)?;
                let name = format!("air.{}.{}", if *axis == gpu_ir::Axis::X { "dfdx" } else { "dfdy" }, suffix(rt));
                self.call(&name, ty, vec![x], attr::CONVERGENT_SET)
            }
            IrOp::Sample { image, sampler, coord, lod } => {
                let texture = self.bindings.get(*image as usize).and_then(|b| b.0).ok_or_else(|| Error { message: format!("binding {image} isn't an image") })?;
                let from = sampler.unwrap_or(*image);
                let samp = self.bindings.get(from as usize).and_then(|b| b.1).ok_or_else(|| Error { message: format!("binding {from} isn't a sampler") })?;
                let coord = self.val(*coord)?;
                let f = self.m.ty(Ty::Float);
                let (level, amount) = match lod {
                    gpu_ir::Lod::Implicit => (false, self.m.konst(Const::Float(f, 0))),
                    gpu_ir::Lod::Bias(v) => (false, self.val(*v)?),
                    gpu_ir::Lod::Level(v) => (true, self.val(*v)?),
                };
                let (i1, i8, i32) = (self.m.int(1), self.m.int(8), self.m.int(32));
                let v2i32 = self.m.ty(Ty::Vector(i32, 2));
                let result = self.m.ty(Ty::Struct(vec![ty, i8], false));
                let yes = self.m.konst(Const::Int(i1, 1));
                let is_level = self.m.konst(Const::Int(i1, level as u64));
                let no_offset = self.m.null(v2i32);
                let no_clamp = self.m.konst(Const::Float(f, 0));
                let zero = self.m.i32(0);
                let r = self.call("air.sample_texture_2d.v4f32", result, vec![texture, samp, coord, yes, no_offset, is_level, amount, no_clamp, zero], attr::SAMPLE);
                self.emit(ty, Op::ExtractValue(r, 0))
            }
        })
    }

    fn bool(&mut self, b: bool) -> Value {
        let i1 = self.m.int(1);
        self.m.konst(Const::Int(i1, u64::from(b)))
    }

    /// An atomic's pointer as an `i32` pointer, its intrinsics' middle (`global` or `local`) and their scope.
    fn atomic_ptr(&mut self, p: Value, s: Scalar) -> Result<(Value, &'static str, u32), Error> {
        if !matches!(s, Scalar::U32 | Scalar::I32) {
            return err(format!("{s:?} atomics aren't in AIR yet"));
        }
        let (space, scope) = match self.space_of(p) {
            DEVICE => ("global", SCOPE_DEVICE),
            THREADGROUP => ("local", SCOPE_THREADGROUP),
            _ => return err("atomics are on device or workgroup memory"),
        };
        let i32 = self.m.int(32);
        Ok((self.retype(p, i32), space, scope))
    }

    fn unary(&mut self, u: UnOp, a: gpu_ir::Value, rt: Type) -> Result<Value, Error> {
        let s = self.e.ty(a).scalar().unwrap_or(Scalar::U32);
        let x = self.val(a)?;
        let t = self.type_of(x);
        let math = |l: &mut Self, f: &str| {
            let name = format!("air.{f}.{}", suffix(rt));
            l.call(&name, t, vec![x], attr::PURE)
        };
        Ok(match u {
            UnOp::Neg if s.is_float() => self.emit(t, Op::FNeg(x)),
            UnOp::Neg => {
                let zero = self.m.splat(t, 0);
                self.bin(SUB, zero, x)
            }
            UnOp::Not => {
                let ones = self.m.splat(t, u64::MAX);
                self.bin(XOR, x, ones)
            }
            UnOp::Abs if s.is_float() => math(self, "fabs"),
            UnOp::Abs if s.is_signed() => {
                let zero = self.m.splat(t, 0);
                let negative = self.cmp(ICMP_SLT, x, zero);
                let negated = self.bin(SUB, zero, x);
                self.select(negative, negated, x)
            }
            UnOp::Abs => x,
            UnOp::Sqrt => math(self, "sqrt"),
            UnOp::Floor => math(self, "floor"),
            UnOp::Ceil => math(self, "ceil"),
            UnOp::Trunc => math(self, "trunc"),
            UnOp::RoundEven => math(self, "rint"),
            UnOp::Sin => math(self, "sin"),
            UnOp::Cos => math(self, "cos"),
            UnOp::Exp2 => math(self, "exp2"),
            UnOp::Log2 => math(self, "log2"),
            UnOp::Popcount => math(self, "popcount"),
        })
    }

    fn binary(&mut self, b: BinOp, a: gpu_ir::Value, c: gpu_ir::Value) -> Result<Value, Error> {
        let at = self.e.ty(a);
        let s = at.scalar().unwrap_or(Scalar::U32);
        let (x, y) = (self.val(a)?, self.val(c)?);
        let t = self.type_of(x);
        let bits = s.bits();
        Ok(match b {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let code = match b {
                    BinOp::Add => ADD,
                    BinOp::Sub => SUB,
                    _ => MUL,
                };
                self.bin(code, x, y)
            }
            BinOp::Div | BinOp::Rem if s.is_int() => {
                // By zero gives 0; the signed overflow case (MIN / -1) divides by 1, giving the wrapped answer.
                let (zero, one) = (self.m.splat(t, 0), self.m.splat(t, 1));
                let by_zero = self.cmp(ICMP_EQ, y, zero);
                let mut bad = by_zero;
                if s.is_signed() {
                    let (min, minus_one) = (self.m.splat(t, 1u64 << (bits - 1)), self.m.splat(t, u64::MAX));
                    let (is_min, is_minus_one) = (self.cmp(ICMP_EQ, x, min), self.cmp(ICMP_EQ, y, minus_one));
                    let overflow = self.bin(AND, is_min, is_minus_one);
                    bad = self.bin(OR, by_zero, overflow);
                }
                let safe = self.select(bad, one, y);
                let code = match (b, s.is_signed()) {
                    (BinOp::Div, true) => SDIV,
                    (BinOp::Div, false) => UDIV,
                    (_, true) => SREM,
                    (_, false) => UREM,
                };
                let q = self.bin(code, x, safe);
                self.select(by_zero, zero, q)
            }
            BinOp::Div => self.bin(FDIV, x, y),
            BinOp::Rem => self.call(&format!("air.fmod.{}", suffix(at)), t, vec![x, y], attr::PURE),
            BinOp::And => self.bin(AND, x, y),
            BinOp::Or => self.bin(OR, x, y),
            BinOp::Xor => self.bin(XOR, x, y),
            BinOp::Shl | BinOp::Shr => {
                let mask = self.m.splat(t, bits as u64 - 1);
                let n = self.bin(AND, y, mask);
                let code = match (b, s.is_signed()) {
                    (BinOp::Shl, _) => SHL,
                    (_, true) => ASHR,
                    (_, false) => LSHR,
                };
                self.bin(code, x, n)
            }
            BinOp::Min | BinOp::Max if s.is_float() => {
                let f = if b == BinOp::Min { "fmin" } else { "fmax" };
                self.call(&format!("air.{f}.{}", suffix(at)), t, vec![x, y], attr::PURE)
            }
            BinOp::Min | BinOp::Max => {
                let pred = match (b, s.is_signed()) {
                    (BinOp::Min, true) => ICMP_SLT,
                    (BinOp::Min, false) => ICMP_ULT,
                    (_, true) => ICMP_SGT,
                    (_, false) => ICMP_UGT,
                };
                let keep = self.cmp(pred, x, y);
                self.select(keep, x, y)
            }
        })
    }

    fn convert(&mut self, a: gpu_ir::Value, to: Scalar) -> Result<Value, Error> {
        let at = self.e.ty(a);
        let from = at.scalar().unwrap_or(Scalar::U32);
        let x = self.val(a)?;
        let dst_scalar = self.scalar_ty(to);
        let dst = self.m.shaped(self.type_of(x), dst_scalar);
        let rt = at.with_scalar(to);
        Ok(match (from, to) {
            _ if from == to => x,
            (Scalar::Bool, _) if to.is_float() => {
                let (one, zero) = (self.m.splat(dst, ONE_F32), self.m.splat(dst, 0));
                self.select(x, one, zero)
            }
            (Scalar::Bool, _) => self.cast(ZEXT, x, dst),
            (_, Scalar::Bool) => {
                let zero = self.m.splat(self.type_of(x), 0);
                self.cmp(if from.is_float() { FCMP_UNE } else { ICMP_NE }, x, zero)
            }
            (Scalar::F32, _) => {
                let name = format!("air.convert.{}{}.f.{}", sign(to), suffix(rt), suffix(at));
                self.call(&name, dst, vec![x], attr::PURE)
            }
            (_, Scalar::F32) => {
                let name = format!("air.convert.f.{}.{}{}", suffix(rt), sign(from), suffix(at));
                self.call(&name, dst, vec![x], attr::PURE)
            }
            _ if from.bits() == to.bits() => x,
            _ if from.bits() > to.bits() => self.cast(TRUNC, x, dst),
            _ => self.cast(if from.is_signed() { SEXT } else { ZEXT }, x, dst),
        })
    }

    fn subgroup(&mut self, sop: SubgroupOp, x: Option<gpu_ir::Value>, lane: Option<gpu_ir::Value>) -> Result<Value, Error> {
        let i1 = self.m.int(1);
        if sop == SubgroupOp::Elect {
            return Ok(self.call("air.simd_is_first", i1, Vec::new(), attr::CONVERGENT_SET));
        }
        let Some(xv) = x else { return err(format!("subgroup {sop:?} without a value")) };
        let xt = self.e.ty(xv);
        let s = xt.scalar().unwrap_or(Scalar::U32);
        let x = self.val(xv)?;
        let t = self.type_of(x);
        let lane = match lane {
            Some(l) => {
                let l = self.val(l)?;
                let i16 = self.m.int(16);
                Some(self.cast(TRUNC, l, i16))
            }
            None => None,
        };
        let simd = |l: &mut Self, name: &str, x: Value, xt: Type| {
            let s = xt.scalar().unwrap_or(Scalar::U32);
            let name = format!("air.simd_{name}.{}{}", sign(s), suffix(xt));
            let t = l.type_of(x);
            let mut args = vec![x];
            args.extend(lane);
            l.call(&name, t, args, attr::CONVERGENT_SET)
        };
        let moving = |sop: SubgroupOp| match sop {
            SubgroupOp::Shuffle => Some("shuffle"),
            SubgroupOp::ShuffleXor => Some("shuffle_xor"),
            SubgroupOp::ShuffleUp => Some("shuffle_up"),
            SubgroupOp::ShuffleDown => Some("shuffle_down"),
            SubgroupOp::BroadcastFirst => Some("broadcast_first"),
            _ => None,
        };
        if let Some(name) = moving(sop) {
            // Values that only move between lanes: booleans as 32-bit words, 64-bit values as pairs of them.
            return Ok(match s {
                Scalar::Bool => {
                    let i32 = self.m.int(32);
                    let wt = self.m.shaped(t, i32);
                    let w = self.cast(ZEXT, x, wt);
                    let moved = simd(self, name, w, xt.with_scalar(Scalar::U32));
                    let zero = self.m.splat(wt, 0);
                    self.cmp(ICMP_NE, moved, zero)
                }
                Scalar::U64 | Scalar::I64 => {
                    let n = xt.lanes() * 2;
                    if n > 4 {
                        return err(format!("subgroup {sop:?} of {xt:?} in AIR"));
                    }
                    let i32 = self.m.int(32);
                    let wt = self.m.ty(Ty::Vector(i32, n as u32));
                    let w = self.cast(BITCAST, x, wt);
                    let moved = simd(self, name, w, Type::Vector(Scalar::U32, n));
                    self.cast(BITCAST, moved, t)
                }
                _ => simd(self, name, x, xt),
            });
        }
        if matches!(s, Scalar::U64 | Scalar::I64) {
            return err(format!("subgroup {sop:?} of {xt:?}: Metal has no 64-bit SIMD-group reductions"));
        }
        Ok(match (sop, s) {
            (SubgroupOp::All | SubgroupOp::Reduce(BinOp::And), Scalar::Bool) if xt.lanes() == 1 => {
                self.call("air.simd_all", i1, vec![x], attr::CONVERGENT_SET)
            }
            (SubgroupOp::Any | SubgroupOp::Reduce(BinOp::Or), Scalar::Bool) if xt.lanes() == 1 => {
                self.call("air.simd_any", i1, vec![x], attr::CONVERGENT_SET)
            }
            (SubgroupOp::Reduce(BinOp::Xor), Scalar::Bool) => {
                let i32 = self.m.int(32);
                let wt = self.m.shaped(t, i32);
                let w = self.cast(ZEXT, x, wt);
                let parity = simd(self, "xor", w, xt.with_scalar(Scalar::U32));
                let zero = self.m.splat(wt, 0);
                self.cmp(ICMP_NE, parity, zero)
            }
            (SubgroupOp::Reduce(op), _) if s != Scalar::Bool => {
                let name = match op {
                    BinOp::Add => "sum",
                    BinOp::Mul => "product",
                    BinOp::Min => "min",
                    BinOp::Max => "max",
                    BinOp::And if s.is_int() => "and",
                    BinOp::Or if s.is_int() => "or",
                    BinOp::Xor if s.is_int() => "xor",
                    _ => return err(format!("a subgroup {op:?} of {xt:?} in AIR")),
                };
                simd(self, name, x, xt)
            }
            (SubgroupOp::InclusiveScan(op) | SubgroupOp::ExclusiveScan(op), _) if s != Scalar::Bool => {
                let kind = if matches!(sop, SubgroupOp::InclusiveScan(_)) { "inclusive" } else { "exclusive" };
                let what = match op {
                    BinOp::Add => "sum",
                    BinOp::Mul => "product",
                    _ => return err(format!("subgroup {sop:?} isn't in AIR yet: Metal's scans are sums and products")),
                };
                simd(self, &format!("prefix_{kind}_{what}"), x, xt)
            }
            _ => return err(format!("subgroup {sop:?} of {xt:?} in AIR")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpu_ir::{Binding, BindingKind};

    /// `b0[gid.x] = b0[gid.x] * 2`, with the loop and branch shapes the escape analysis cares about.
    fn program() -> Entry {
        let mut e = Entry::new("t", [64, 1, 1]);
        e.bindings.push(Binding { set: 0, binding: 0, kind: BindingKind::Storage { writable: true } });
        let gid = e.value(Type::UVEC3);
        let x = e.value(Type::U32);
        let four = e.value(Type::U32);
        let off = e.value(Type::U32);
        let b = e.value(Type::Ptr(Space::Device));
        let p = e.value(Type::Ptr(Space::Device));
        let inner = e.value(Type::U32);
        e.body = vec![
            Stmt::Let(gid, IrOp::Builtin(Builtin::GlobalId)),
            Stmt::Let(x, IrOp::Extract(gid, 0)),
            Stmt::Let(four, IrOp::Const(Scalar::U32, 4)),
            Stmt::Let(off, IrOp::Binary(BinOp::Mul, x, four)),
            Stmt::Let(b, IrOp::Binding(0)),
            Stmt::Let(p, IrOp::Offset(b, off)),
            Stmt::Loop { body: vec![Stmt::Let(inner, IrOp::Load(p, Type::U32)), Stmt::Break], continuing: vec![], break_if: None },
            Stmt::Store(p, inner),
        ];
        e
    }

    #[test]
    fn a_value_read_after_its_loop_lives_in_an_alloca() {
        let e = program();
        let escapes = escaping(&e);
        assert!(escapes[6], "the loaded value is read after the loop");
        assert!(!escapes[5], "the pointer is read inside the loop, which its block encloses");
    }

    #[test]
    fn every_block_ends_in_a_terminator() {
        let m = lower(&program(), None).unwrap();
        let body = m.body();
        for (i, block) in body.blocks.iter().enumerate() {
            let last = block.last().map(|&i| body.insts[i as usize].terminates());
            assert_eq!(last, Some(true), "block {i} doesn't end in a terminator");
        }
    }
}
