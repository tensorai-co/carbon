//! One SPIR-V entry point to one IR [`Entry`](gpu_ir::Entry).
//!
//! **Control flow.** SPIR-V's blocks form a graph, but a shader's graph is structured: every selection and loop
//! header names its merge block (and a loop its continue target). [`Translator::region`] walks blocks from a start,
//! emitting each one's instructions, until it reaches the merge of the construct it's inside (it stops), the
//! innermost loop's merge (`Break`) or continue target (`Continue`), or a return. A selection header becomes an `If`
//! whose branches are regions stopping at its merge; a loop header becomes a `Loop` whose body is the header and what
//! follows it, and whose continuing is the region from the continue target back to the header.
//!
//! **Phis** become variables: each predecessor writes the incoming value before its branch, and the phi's block
//! reads it at its start.
//!
//! **Memory.** A pointer is an address and the SPIR-V type it points at. Access chains add byte offsets, with the
//! storage class's layout: buffers' explicit `Offset`, `ArrayStride` and `MatrixStride` decorations, and our own
//! natural layout for workgroup and private memory. Function variables of scalar or vector type that are only loaded
//! and stored become IR variables (registers); the rest get private memory. A matrix is its columns.
//!
//! **Stages.** A vertex or fragment shader's inputs are read once, at the start; its outputs are registers (a shader
//! may read back what it wrote, as `gl_Position`), written out as the entry's outputs wherever it returns. An image,
//! sampler, or combined image and sampler is its binding's number, which `OpImageSample*` names.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu_ir::{
    AtomicOp, Axis, Barrier, BinOp, Binding, BindingKind, Block, Builtin, Cmp, Entry, Interpolation, Io, Lod, Op,
    Region, Scalar, Slot, Stage, Stmt, SubgroupOp, Type, UnOp, Value, Var,
};

use super::op;
use super::{Constant, EntryPoint, Function, Inst, Spirv, Type as SpvType, err};
use crate::Error;

/// Blocks emitted before a translation gives up: a guard against graphs that aren't structured.
const MAX_BLOCKS: usize = 1 << 20;
/// Selections and loops nested deeper than this are refused.
const MAX_DEPTH: usize = 256;
/// SPIR-V's `Subgroup` scope.
const SCOPE_SUBGROUP: u64 = 3;
/// SPIR-V's `Workgroup` scope.
const SCOPE_WORKGROUP: u64 = 2;

/// A SPIR-V id's value, as the translation holds it.
#[derive(Debug, Clone)]
enum Val {
    Ir(Value),
    /// A struct's or array's parts.
    Composite(Vec<Val>),
    Ptr(Ptr),
    /// An image, a sampler, or the two together: bindings (a combined binding has no separate sampler).
    Image(u32),
    Sampler(u32),
    SampledImage { image: u32, sampler: Option<u32> },
    /// Something declared but not translated yet; an error if it's used.
    Unsupported(&'static str),
}

/// One lane of a register variable's vector.
#[derive(Debug, Clone, Copy)]
enum Lane {
    Const(u8),
    Dynamic(Value),
}

/// How a matrix is laid out in memory: the bytes from one column (or row) to the next, and which it is.
#[derive(Debug, Clone, Copy)]
struct Mat {
    stride: u32,
    row_major: bool,
}

#[derive(Debug, Clone)]
enum Ptr {
    /// An address in memory, the type it points at, whether that memory's layout is explicit (buffers), and the
    /// layout of the matrices it holds, as the struct member they're in says.
    Mem { addr: Value, ty: u32, explicit: bool, mat: Option<Mat> },
    /// A register variable, or one lane of it.
    Reg { var: Var, ty: u32, lane: Option<Lane> },
    /// A value that can only be read: an input, a built-in input, an image binding, or part of one.
    Const { val: Box<Val>, ty: u32 },
    /// A struct's members, each its own pointer: an output block such as `gl_PerVertex`.
    Fields(Vec<Ptr>),
}

#[derive(Debug, Clone, Copy)]
enum Merge {
    Selection(u32),
    Loop { merge: u32, cont: u32 },
}

/// How a block ends.
enum End {
    Goto(u32),
    Cond { cond: Value, t: u32, f: u32 },
    Switch { selector: Value, default: u32, cases: Vec<(u64, u32)> },
    Return,
    /// `OpKill`, `OpTerminateInvocation`: the fragment is thrown away.
    Kill,
}

/// Where a region is in the constructs around it.
#[derive(Debug, Clone, Copy)]
struct Ctx {
    /// The merge of the selection being translated: the region ends there.
    stop: Option<u32>,
    /// The innermost loop: its header, merge and continue target.
    header: Option<u32>,
    merge: Option<u32>,
    cont: Option<u32>,
    /// Translating that loop's continue construct, which ends at the back edge to the header.
    continuing: bool,
}

const TOP: Ctx = Ctx { stop: None, header: None, merge: None, cont: None, continuing: false };

pub(super) fn entry(s: &Spirv<'_>, ep: &EntryPoint) -> Result<Entry, Error> {
    let Some(function) = s.functions.get(&ep.function) else {
        return err(format!("{}'s function %{} has no body", ep.name, ep.function));
    };
    let e = match ep.model {
        op::MODEL_VERTEX => Entry::shader(ep.name.clone(), Stage::Vertex),
        op::MODEL_FRAGMENT => {
            if s.modes.get(&ep.function).is_some_and(|m| m.contains(&op::MODE_ORIGIN_LOWER_LEFT)) {
                return err(format!("{}: a lower-left origin isn't Vulkan's (OriginUpperLeft)", ep.name));
            }
            Entry::shader(ep.name.clone(), Stage::Fragment)
        }
        _ => Entry::new(ep.name.clone(), s.local_size.get(&ep.function).copied().unwrap_or([1, 1, 1])),
    };
    let mut t = Translator { s, f: function, e, vals: BTreeMap::new(), phis: BTreeMap::new(), stops: Vec::new(), blocks: 0, outputs: Vec::new() };
    let mut body = Vec::new();
    t.prologue(&mut body).map_err(|e| t.context(e, "declarations"))?;
    let Some(&first) = function.order.first() else { return err(format!("{} has no blocks", ep.name)) };
    t.region(first, TOP, &mut body, 0)?;
    t.e.body = body;
    Ok(t.e)
}

struct Translator<'s, 'a> {
    s: &'s Spirv<'a>,
    f: &'s Function<'a>,
    e: Entry,
    vals: BTreeMap<u32, Val>,
    phis: BTreeMap<u32, Var>,
    /// The merges of the selections being translated, outermost first.
    stops: Vec<u32>,
    blocks: usize,
    /// Each stage output's register: written out as output `n` wherever the shader returns.
    outputs: Vec<(u32, Var)>,
}

impl<'s, 'a> Translator<'s, 'a> {
    fn context(&self, e: Error, what: &str) -> Error {
        Error { message: format!("{} ({}): {}", self.e.name, what, e.message) }
    }

    // ---- values ---------------------------------------------------------------------------------------------

    fn let_(&mut self, out: &mut Block, ty: Type, op: Op) -> Value {
        let v = self.e.value(ty);
        out.push(Stmt::Let(v, op));
        v
    }

    fn konst(&mut self, out: &mut Block, scalar: Scalar, bits: u64) -> Value {
        self.let_(out, Type::Scalar(scalar), Op::Const(scalar, bits))
    }

    fn val(&self, id: u32) -> Result<Val, Error> {
        match self.vals.get(&id) {
            Some(Val::Unsupported(what)) => err(format!("%{id} is {what}, which isn't translated yet")),
            Some(v) => Ok(v.clone()),
            None => err(format!("%{id} is used before it's defined")),
        }
    }

    fn ir(&self, id: u32) -> Result<Value, Error> {
        match self.val(id)? {
            Val::Ir(v) => Ok(v),
            other => err(format!("%{id} is {}, where a number or boolean is wanted", kind(&other))),
        }
    }

    fn ptr(&self, id: u32) -> Result<Ptr, Error> {
        match self.val(id)? {
            Val::Ptr(p) => Ok(p),
            other => err(format!("%{id} is {}, where a pointer is wanted", kind(&other))),
        }
    }

    fn set(&mut self, id: u32, val: Val) {
        self.vals.insert(id, val);
    }

    fn scalar_of(&self, ty: u32) -> Result<Scalar, Error> {
        Ok(match self.s.ty_of(ty)? {
            SpvType::Bool => Scalar::Bool,
            SpvType::Int { width: 32, signed: false } => Scalar::U32,
            SpvType::Int { width: 32, signed: true } => Scalar::I32,
            SpvType::Int { width: 64, signed: false } => Scalar::U64,
            SpvType::Int { width: 64, signed: true } => Scalar::I64,
            SpvType::Float { width: 32 } => Scalar::F32,
            other => return err(format!("%{ty} ({other:?}) isn't a scalar type translated yet")),
        })
    }

    /// The IR type of a SPIR-V scalar or vector type.
    fn ir_ty(&self, ty: u32) -> Result<Type, Error> {
        match self.s.ty_of(ty)? {
            SpvType::Vector { component, count: count @ 2..=4 } => {
                Ok(Type::Vector(self.scalar_of(*component)?, *count as u8))
            }
            SpvType::Vector { count, .. } => err(format!("a vector of {count} components")),
            _ => Ok(Type::Scalar(self.scalar_of(ty)?)),
        }
    }

    fn is_value_type(&self, ty: u32) -> bool {
        matches!(
            self.s.types.get(&ty),
            Some(SpvType::Bool | SpvType::Int { .. } | SpvType::Float { .. } | SpvType::Vector { .. })
        )
    }

    /// `v` as `want`: the same bits if the sizes match (signedness), converted otherwise (widths).
    fn as_ty(&mut self, out: &mut Block, v: Value, want: Type) -> Value {
        let have = self.e.ty(v);
        if have == want {
            return v;
        }
        let bools = have.scalar() == Some(Scalar::Bool) || want.scalar() == Some(Scalar::Bool);
        if have.size() == want.size() && !bools {
            self.let_(out, want, Op::Bitcast(v, want))
        } else {
            let to = want.scalar().unwrap_or(Scalar::U32);
            self.let_(out, have.with_scalar(to), Op::Convert(v, to))
        }
    }

    fn zero(&mut self, out: &mut Block, ty: u32) -> Result<Val, Error> {
        Ok(match self.s.ty_of(ty)?.clone() {
            SpvType::Vector { component, count } => {
                let s = self.scalar_of(component)?;
                let parts: Vec<Value> = (0..count).map(|_| self.konst(out, s, 0)).collect();
                Val::Ir(self.let_(out, Type::Vector(s, count as u8), Op::Construct(parts)))
            }
            SpvType::Struct { members } => {
                let mut parts = Vec::new();
                for m in members {
                    parts.push(self.zero(out, m)?);
                }
                Val::Composite(parts)
            }
            SpvType::Array { element, length } => {
                let n = self.array_len(length)?;
                let mut parts = Vec::new();
                for _ in 0..n {
                    parts.push(self.zero(out, element)?);
                }
                Val::Composite(parts)
            }
            SpvType::Matrix { column, count } => {
                let mut parts = Vec::new();
                for _ in 0..count {
                    parts.push(self.zero(out, column)?);
                }
                Val::Composite(parts)
            }
            _ => {
                let s = self.scalar_of(ty)?;
                Val::Ir(self.konst(out, s, 0))
            }
        })
    }

    fn array_len(&self, length: u32) -> Result<u64, Error> {
        match self.s.const_int(length) {
            Some(n) => Ok(n),
            None => err(format!("an array length %{length} that isn't a constant")),
        }
    }

    // ---- declarations ---------------------------------------------------------------------------------------

    fn prologue(&mut self, out: &mut Block) -> Result<(), Error> {
        // Bindings get indices in (set, binding) order, so a dispatch's list doesn't depend on declaration order.
        let mut buffers: Vec<(u32, u32, u32)> = Vec::new();
        for (&id, g) in &self.s.globals {
            if matches!(g.class, op::CLASS_STORAGE_BUFFER | op::CLASS_UNIFORM | op::CLASS_UNIFORM_CONSTANT) {
                let set = self.s.decoration(id, op::DECORATION_DESCRIPTOR_SET).unwrap_or(0);
                let binding = self.s.decoration(id, op::DECORATION_BINDING).unwrap_or(0);
                buffers.push((set, binding, id));
            } else if g.class == op::CLASS_PUSH_CONSTANT {
                // A uniform buffer at a set of its own, filled with what's pushed.
                buffers.push((gpu_ir::PUSH_CONSTANTS, 0, id));
            }
        }
        buffers.sort();
        let mut binding_of = BTreeMap::new();
        for (set, binding, id) in buffers {
            let g = &self.s.globals[&id];
            let SpvType::Pointer { pointee, .. } = self.s.ty_of(g.ty)? else { return err(format!("%{id}'s type")) };
            let storage = g.class == op::CLASS_STORAGE_BUFFER || self.s.has_decoration(*pointee, op::DECORATION_BUFFER_BLOCK);
            let kind = if g.class == op::CLASS_UNIFORM_CONSTANT {
                match self.s.ty_of(*pointee)? {
                    SpvType::Image => BindingKind::Image,
                    SpvType::Sampler => BindingKind::Sampler,
                    SpvType::SampledImage => BindingKind::Combined,
                    other => return err(format!("%{id}: a uniform constant of type {other:?} isn't translated yet")),
                }
            } else if storage {
                let read_only = self.s.has_decoration(id, op::DECORATION_NON_WRITABLE) || self.all_members_read_only(*pointee);
                BindingKind::Storage { writable: !read_only }
            } else {
                BindingKind::Uniform
            };
            binding_of.insert(id, self.e.bindings.len() as u32);
            self.e.bindings.push(Binding { set, binding, kind });
        }
        for &id in &self.s.declared.clone() {
            if let Some((ty, c)) = self.s.constants.get(&id).cloned() {
                let val = self.constant(out, ty, &c).map_err(|e| Error { message: format!("constant %{id}: {}", e.message) })?;
                self.set(id, val);
                continue;
            }
            let Some(g) = self.s.globals.get(&id).cloned() else { continue };
            let SpvType::Pointer { pointee, .. } = self.s.ty_of(g.ty)?.clone() else { return err(format!("%{id}'s type")) };
            let val = match g.class {
                op::CLASS_INPUT => match self.s.decoration(id, op::DECORATION_BUILT_IN) {
                    Some(b) => self.builtin(out, b, pointee)?,
                    None => self.input(out, id, pointee)?,
                },
                op::CLASS_OUTPUT => self.output(out, id, pointee)?,
                op::CLASS_STORAGE_BUFFER | op::CLASS_UNIFORM | op::CLASS_PUSH_CONSTANT => {
                    let n = binding_of[&id];
                    let addr = self.let_(out, Type::Ptr(gpu_ir::Space::Device), Op::Binding(n));
                    Val::Ptr(Ptr::Mem { addr, ty: pointee, explicit: true, mat: None })
                }
                op::CLASS_UNIFORM_CONSTANT => {
                    let n = binding_of[&id];
                    let val = match self.e.bindings[n as usize].kind {
                        BindingKind::Image => Val::Image(n),
                        BindingKind::Sampler => Val::Sampler(n),
                        _ => Val::SampledImage { image: n, sampler: None },
                    };
                    Val::Ptr(Ptr::Const { val: Box::new(val), ty: pointee })
                }
                op::CLASS_WORKGROUP => {
                    let (size, align) = self.layout(pointee, false)?;
                    self.e.workgroup.push(Region { size: size.max(1), align });
                    let n = self.e.workgroup.len() as u32 - 1;
                    let addr = self.let_(out, Type::Ptr(gpu_ir::Space::Workgroup), Op::Workgroup(n));
                    Val::Ptr(Ptr::Mem { addr, ty: pointee, explicit: false, mat: None })
                }
                op::CLASS_PRIVATE | op::CLASS_FUNCTION => self.local(out, pointee, g.init)?,
                _ => Val::Unsupported("a variable of this storage class"),
            };
            self.set(id, val);
        }
        // Phis become variables, read at their block's start and written by each predecessor.
        for block in self.f.blocks.values() {
            for inst in &block.insts {
                if inst.op == op::PHI {
                    let ty = self.ir_ty(inst.arg(0)?)?;
                    let var = self.e.var(ty);
                    self.phis.insert(inst.arg(1)?, var);
                }
            }
        }
        Ok(())
    }

    fn all_members_read_only(&self, ty: u32) -> bool {
        match self.s.types.get(&ty) {
            Some(SpvType::Struct { members }) => {
                !members.is_empty()
                    && (0..members.len() as u32).all(|m| self.s.member_decoration(ty, m, op::DECORATION_NON_WRITABLE).is_some())
            }
            _ => false,
        }
    }

    fn constant(&mut self, out: &mut Block, ty: u32, c: &Constant) -> Result<Val, Error> {
        match c {
            Constant::Null => self.zero(out, ty),
            Constant::Scalar(bits) => {
                let s = self.scalar_of(ty)?;
                let bits = match s {
                    Scalar::I32 => *bits as u32 as i32 as i64 as u64,
                    Scalar::U32 | Scalar::F32 => *bits & 0xffff_ffff,
                    Scalar::Bool => (*bits != 0) as u64,
                    Scalar::U64 | Scalar::I64 => *bits,
                };
                Ok(Val::Ir(self.konst(out, s, bits)))
            }
            Constant::Composite(parts) => {
                if let SpvType::Vector { .. } = self.s.ty_of(ty)? {
                    let ir_ty = self.ir_ty(ty)?;
                    let mut lanes = Vec::new();
                    for p in parts {
                        lanes.push(self.ir(*p)?);
                    }
                    Ok(Val::Ir(self.let_(out, ir_ty, Op::Construct(lanes))))
                } else {
                    let mut vals = Vec::new();
                    for p in parts {
                        vals.push(self.val(*p)?);
                    }
                    Ok(Val::Composite(vals))
                }
            }
        }
    }

    fn builtin(&mut self, out: &mut Block, which: u32, ty: u32) -> Result<Val, Error> {
        let b = match which {
            op::BUILTIN_GLOBAL_INVOCATION_ID => Builtin::GlobalId,
            op::BUILTIN_LOCAL_INVOCATION_ID => Builtin::LocalId,
            op::BUILTIN_WORKGROUP_ID => Builtin::GroupId,
            op::BUILTIN_NUM_WORKGROUPS => Builtin::GroupCount,
            op::BUILTIN_LOCAL_INVOCATION_INDEX => Builtin::LocalIndex,
            op::BUILTIN_SUBGROUP_SIZE => Builtin::SubgroupSize,
            op::BUILTIN_SUBGROUP_LOCAL_INVOCATION_ID => Builtin::SubgroupLane,
            op::BUILTIN_SUBGROUP_ID => Builtin::SubgroupId,
            op::BUILTIN_NUM_SUBGROUPS => Builtin::SubgroupCount,
            op::BUILTIN_VERTEX_INDEX => Builtin::VertexIndex,
            op::BUILTIN_INSTANCE_INDEX => Builtin::InstanceIndex,
            op::BUILTIN_FRAG_COORD => Builtin::FragCoord,
            op::BUILTIN_FRONT_FACING => Builtin::FrontFacing,
            _ => return Ok(Val::Unsupported("a built-in input")),
        };
        let v = self.let_(out, b.ty(), Op::Builtin(b));
        let want = self.ir_ty(ty)?;
        let v = self.as_ty(out, v, want);
        Ok(Val::Ptr(Ptr::Const { val: Box::new(Val::Ir(v)), ty }))
    }

    fn interpolation(&self, id: u32) -> Interpolation {
        if self.s.has_decoration(id, op::DECORATION_FLAT) {
            Interpolation::Flat
        } else if self.s.has_decoration(id, op::DECORATION_NO_PERSPECTIVE) {
            Interpolation::NoPerspective
        } else {
            Interpolation::Smooth
        }
    }

    /// A stage input at a location: read once, at the start.
    fn input(&mut self, out: &mut Block, id: u32, ty: u32) -> Result<Val, Error> {
        let Some(location) = self.s.decoration(id, op::DECORATION_LOCATION) else {
            return err(format!("the stage input %{id} has no Location"));
        };
        if !self.is_value_type(ty) {
            return Ok(Val::Unsupported("a stage input that isn't a scalar or vector"));
        }
        let ir_ty = self.ir_ty(ty)?;
        let interpolation = self.interpolation(id);
        self.e.inputs.push(Io { slot: Slot::Location(location), ty: ir_ty, interpolation });
        let v = self.let_(out, ir_ty, Op::Input(self.e.inputs.len() as u32 - 1));
        Ok(Val::Ptr(Ptr::Const { val: Box::new(Val::Ir(v)), ty }))
    }

    /// A stage output: a register, written out wherever the shader returns. A block of built-ins (`gl_PerVertex`) is
    /// one register a member; members the IR has no slot for (`gl_PointSize`) are registers nothing reads.
    fn output(&mut self, out: &mut Block, id: u32, ty: u32) -> Result<Val, Error> {
        let interpolation = self.interpolation(id);
        if let SpvType::Struct { members } = self.s.ty_of(ty)?.clone() {
            let mut fields = Vec::new();
            for (i, m) in members.iter().enumerate() {
                let slot = match self.s.member_decoration(ty, i as u32, op::DECORATION_BUILT_IN) {
                    Some(op::BUILTIN_POSITION) => Some(Slot::Position),
                    Some(op::BUILTIN_POINT_SIZE) => None,
                    Some(op::BUILTIN_CLIP_DISTANCE | op::BUILTIN_CULL_DISTANCE) => {
                        fields.push(Ptr::Const { val: Box::new(Val::Unsupported("a clip or cull distance")), ty: *m });
                        continue;
                    }
                    _ => return err(format!("member {i} of the output block %{id} isn't a built-in the IR has")),
                };
                fields.push(self.output_reg(out, slot, *m, interpolation)?);
            }
            return Ok(Val::Ptr(Ptr::Fields(fields)));
        }
        let slot = match self.s.decoration(id, op::DECORATION_BUILT_IN) {
            Some(op::BUILTIN_POSITION) => Some(Slot::Position),
            Some(op::BUILTIN_FRAG_DEPTH) => Some(Slot::FragDepth),
            Some(op::BUILTIN_POINT_SIZE) => None,
            Some(_) => return Ok(Val::Unsupported("a built-in output")),
            None => match self.s.decoration(id, op::DECORATION_LOCATION) {
                Some(location) => Some(Slot::Location(location)),
                None => return err(format!("the stage output %{id} has no Location")),
            },
        };
        Ok(Val::Ptr(self.output_reg(out, slot, ty, interpolation)?))
    }

    fn output_reg(&mut self, out: &mut Block, slot: Option<Slot>, ty: u32, interpolation: Interpolation) -> Result<Ptr, Error> {
        if !self.is_value_type(ty) {
            return Ok(Ptr::Const { val: Box::new(Val::Unsupported("a stage output that isn't a scalar or vector")), ty });
        }
        let Val::Ptr(reg @ Ptr::Reg { var, .. }) = self.local(out, ty, None)? else { return err("an output's register".into()) };
        if let Some(slot) = slot {
            let ir_ty = self.ir_ty(ty)?;
            self.e.outputs.push(Io { slot, ty: ir_ty, interpolation });
            self.outputs.push((self.e.outputs.len() as u32 - 1, var));
        }
        Ok(reg)
    }

    /// The outputs' registers, written out: before every return.
    fn write_outputs(&mut self, out: &mut Block) {
        for (n, var) in self.outputs.clone() {
            let ty = self.e.vars[var.0 as usize];
            let v = self.let_(out, ty, Op::Read(var));
            out.push(Stmt::Output(n, v));
        }
    }

    /// A function or private variable: a register if it's a scalar or vector, private memory otherwise.
    fn local(&mut self, out: &mut Block, ty: u32, init: Option<u32>) -> Result<Val, Error> {
        if self.is_value_type(ty) {
            let ir_ty = self.ir_ty(ty)?;
            let var = self.e.var(ir_ty);
            // Every variable starts as something, so no backend reads an undefined register.
            let first = match init {
                Some(id) => self.ir(id)?,
                None => match self.zero(out, ty)? {
                    Val::Ir(v) => v,
                    _ => return err("a register variable's zero".into()),
                },
            };
            out.push(Stmt::Write(var, first));
            return Ok(Val::Ptr(Ptr::Reg { var, ty, lane: None }));
        }
        let (size, align) = self.layout(ty, false)?;
        self.e.private.push(Region { size: size.max(1), align });
        let n = self.e.private.len() as u32 - 1;
        let addr = self.let_(out, Type::Ptr(gpu_ir::Space::Private), Op::Private(n));
        let ptr = Ptr::Mem { addr, ty, explicit: false, mat: None };
        if let Some(id) = init {
            let val = self.val(id)?;
            self.store(out, &ptr, val)?;
        }
        Ok(Val::Ptr(ptr))
    }

    // ---- layout ---------------------------------------------------------------------------------------------

    /// Size and alignment: explicit (buffers) from decorations, natural otherwise.
    fn layout(&self, ty: u32, explicit: bool) -> Result<(u32, u32), Error> {
        Ok(match self.s.ty_of(ty)?.clone() {
            SpvType::Bool => (4, 4),
            SpvType::Int { width, .. } | SpvType::Float { width } => (width / 8, width / 8),
            SpvType::Vector { component, count } => {
                let (s, a) = self.layout(component, explicit)?;
                (s * count, a)
            }
            SpvType::Array { element, length } => {
                let stride = self.stride(ty, element, explicit)?;
                let (_, a) = self.layout(element, explicit)?;
                (stride * self.array_len(length)? as u32, a)
            }
            SpvType::RuntimeArray { element } => (0, self.layout(element, explicit)?.1),
            SpvType::Matrix { column, count } => {
                let stride = self.column_stride(column, explicit)?;
                (stride * count, self.layout(column, explicit)?.1.max(if explicit { 16 } else { 1 }))
            }
            SpvType::Struct { members } => {
                let mut size = 0u32;
                let mut align = 1u32;
                for (i, m) in members.iter().enumerate() {
                    let (s, a) = self.layout(*m, explicit)?;
                    let at = self.member_offset(ty, i as u32, explicit)?;
                    size = size.max(at + s);
                    align = align.max(a);
                }
                (size.next_multiple_of(align), align)
            }
            other => return err(format!("%{ty} ({other:?}) has no memory layout translated yet")),
        })
    }

    /// A matrix's natural column stride: the column rounded up to its alignment, to 16 bytes in a buffer (std140's
    /// and std430's, when no `MatrixStride` says otherwise).
    fn column_stride(&self, column: u32, explicit: bool) -> Result<u32, Error> {
        let (s, a) = self.layout(column, explicit)?;
        Ok(if explicit { s.next_multiple_of(16) } else { s.next_multiple_of(a) })
    }

    /// The layout a struct member's matrices have, from its decorations.
    fn member_mat(&self, ty: u32, member: u32) -> Option<Mat> {
        let stride = self.s.member_decoration(ty, member, op::DECORATION_MATRIX_STRIDE)?;
        let row_major = self.s.member_decorations.get(&(ty, member)).is_some_and(|d| d.iter().any(|(w, _)| *w == op::DECORATION_ROW_MAJOR));
        Some(Mat { stride, row_major })
    }

    fn stride(&self, array: u32, element: u32, explicit: bool) -> Result<u32, Error> {
        if explicit
            && let Some(stride) = self.s.decoration(array, op::DECORATION_ARRAY_STRIDE) {
                return Ok(stride);
            }
        let (s, a) = self.layout(element, explicit)?;
        Ok(s.next_multiple_of(a))
    }

    fn member_offset(&self, ty: u32, member: u32, explicit: bool) -> Result<u32, Error> {
        if explicit {
            return match self.s.member_decoration(ty, member, op::DECORATION_OFFSET) {
                Some(at) => Ok(at),
                None => err(format!("member {member} of %{ty} has no Offset")),
            };
        }
        let SpvType::Struct { members } = self.s.ty_of(ty)?.clone() else { return err(format!("%{ty} isn't a struct")) };
        let mut at = 0u32;
        for (i, m) in members.iter().enumerate() {
            let (s, a) = self.layout(*m, false)?;
            at = at.next_multiple_of(a);
            if i as u32 == member {
                return Ok(at);
            }
            at += s;
        }
        err(format!("%{ty} has no member {member}"))
    }

    // ---- memory ---------------------------------------------------------------------------------------------

    fn offset(&mut self, out: &mut Block, addr: Value, bytes: u64) -> Value {
        if bytes == 0 {
            return addr;
        }
        let n = self.konst(out, Scalar::U64, bytes);
        let ty = self.e.ty(addr);
        self.let_(out, ty, Op::Offset(addr, n))
    }

    fn access(&mut self, out: &mut Block, base: Ptr, indices: &[u32]) -> Result<Ptr, Error> {
        match base {
            Ptr::Mem { mut addr, mut ty, explicit, mut mat } => {
                let mut fixed = 0u64;
                for &index in indices {
                    let (element, stride) = match self.s.ty_of(ty)?.clone() {
                        SpvType::Struct { members } => {
                            let Some(i) = self.s.const_int(index) else { return err("a struct indexed at run time".into()) };
                            fixed += self.member_offset(ty, i as u32, explicit)? as u64;
                            mat = if explicit { self.member_mat(ty, i as u32) } else { None };
                            ty = *members.get(i as usize).ok_or_else(|| Error { message: format!("no member {i}") })?;
                            continue;
                        }
                        SpvType::Array { element, .. } | SpvType::RuntimeArray { element } => {
                            (element, self.stride(ty, element, explicit)?)
                        }
                        SpvType::Matrix { column, .. } => {
                            if mat.is_some_and(|m| m.row_major) {
                                return err("a pointer to one column of a row-major matrix".into());
                            }
                            let stride = match mat {
                                Some(m) => m.stride,
                                None => self.column_stride(column, explicit)?,
                            };
                            mat = None;
                            (column, stride)
                        }
                        SpvType::Vector { component, .. } => (component, self.layout(component, explicit)?.0),
                        other => return err(format!("an access chain into {other:?}")),
                    };
                    match self.s.const_int(index) {
                        Some(i) => fixed += i * stride as u64,
                        None => {
                            let i = self.ir(index)?;
                            let wide = if self.e.ty(i).scalar().is_some_and(|s| s.is_signed()) { Scalar::I64 } else { Scalar::U64 };
                            let i = self.as_ty(out, i, Type::Scalar(wide));
                            let stride = self.konst(out, wide, stride as u64);
                            let bytes = self.let_(out, Type::Scalar(wide), Op::Binary(BinOp::Mul, i, stride));
                            let ptr_ty = self.e.ty(addr);
                            addr = self.let_(out, ptr_ty, Op::Offset(addr, bytes));
                        }
                    }
                    ty = element;
                }
                let addr = self.offset(out, addr, fixed);
                Ok(Ptr::Mem { addr, ty, explicit, mat })
            }
            Ptr::Fields(fields) => {
                let Some((&first, rest)) = indices.split_first() else { return Ok(Ptr::Fields(fields)) };
                let Some(i) = self.s.const_int(first) else { return err("an output block indexed at run time".into()) };
                let Some(field) = fields.get(i as usize).cloned() else { return err(format!("no member {i}")) };
                self.access(out, field, rest)
            }
            Ptr::Reg { var, ty, lane: None } => match (self.s.ty_of(ty)?.clone(), indices) {
                (_, []) => Ok(Ptr::Reg { var, ty, lane: None }),
                (SpvType::Vector { component, .. }, [index]) => {
                    let lane = match self.s.const_int(*index) {
                        Some(i) => Lane::Const(i as u8),
                        None => Lane::Dynamic(self.ir(*index)?),
                    };
                    Ok(Ptr::Reg { var, ty: component, lane: Some(lane) })
                }
                _ => err("an access chain into a register variable".into()),
            },
            Ptr::Reg { .. } => err("an access chain into one lane of a vector".into()),
            Ptr::Const { mut val, mut ty } => {
                for &index in indices {
                    let next = match self.s.ty_of(ty)?.clone() {
                        SpvType::Struct { members } => {
                            let i = self.s.const_int(index).unwrap_or(0) as usize;
                            ty = members[i];
                            part(&val, i)?
                        }
                        SpvType::Array { element, .. } => {
                            let Some(i) = self.s.const_int(index) else { return err("a built-in array indexed at run time".into()) };
                            ty = element;
                            part(&val, i as usize)?
                        }
                        SpvType::Vector { component, .. } => {
                            let Val::Ir(v) = *val else { return err("a vector built-in".into()) };
                            let s = Type::Scalar(self.scalar_of(component)?);
                            ty = component;
                            match self.s.const_int(index) {
                                Some(i) => Val::Ir(self.let_(out, s, Op::Extract(v, i as u8))),
                                None => {
                                    let i = self.ir(index)?;
                                    Val::Ir(self.let_(out, s, Op::ExtractDynamic(v, i)))
                                }
                            }
                        }
                        other => return err(format!("an access chain into {other:?}")),
                    };
                    val = Box::new(next);
                }
                Ok(Ptr::Const { val, ty })
            }
        }
    }

    fn load(&mut self, out: &mut Block, ptr: &Ptr) -> Result<Val, Error> {
        match ptr {
            Ptr::Mem { addr, ty, explicit, mat } => self.load_mem(out, *addr, *ty, *explicit, *mat),
            Ptr::Fields(fields) => {
                let mut parts = Vec::new();
                for f in fields {
                    parts.push(self.load(out, f)?);
                }
                Ok(Val::Composite(parts))
            }
            Ptr::Reg { var, ty, lane } => {
                let whole_ty = self.e.vars[var.0 as usize];
                let whole = self.let_(out, whole_ty, Op::Read(*var));
                Ok(Val::Ir(match lane {
                    None => whole,
                    Some(Lane::Const(i)) => {
                        let s = self.ir_ty(*ty)?;
                        self.let_(out, s, Op::Extract(whole, *i))
                    }
                    Some(Lane::Dynamic(i)) => {
                        let s = self.ir_ty(*ty)?;
                        self.let_(out, s, Op::ExtractDynamic(whole, *i))
                    }
                }))
            }
            Ptr::Const { val, .. } => match &**val {
                Val::Unsupported(what) => err(format!("a read of {what}, which isn't translated yet")),
                v => Ok(v.clone()),
            },
        }
    }

    fn load_mem(&mut self, out: &mut Block, addr: Value, ty: u32, explicit: bool, mat: Option<Mat>) -> Result<Val, Error> {
        match self.s.ty_of(ty)?.clone() {
            SpvType::Struct { members } => {
                let mut parts = Vec::new();
                for (i, m) in members.iter().enumerate() {
                    let at = self.member_offset(ty, i as u32, explicit)? as u64;
                    let a = self.offset(out, addr, at);
                    let mat = if explicit { self.member_mat(ty, i as u32) } else { None };
                    parts.push(self.load_mem(out, a, *m, explicit, mat)?);
                }
                Ok(Val::Composite(parts))
            }
            SpvType::Array { element, length } => {
                let stride = self.stride(ty, element, explicit)? as u64;
                let mut parts = Vec::new();
                for i in 0..self.array_len(length)? {
                    let a = self.offset(out, addr, i * stride);
                    parts.push(self.load_mem(out, a, element, explicit, mat)?);
                }
                Ok(Val::Composite(parts))
            }
            SpvType::Matrix { column, count } => {
                let stride = match mat {
                    Some(m) => m.stride,
                    None => self.column_stride(column, explicit)?,
                } as u64;
                let SpvType::Vector { component, count: rows } = self.s.ty_of(column)?.clone() else {
                    return err("a matrix whose columns aren't vectors".into());
                };
                let mut columns = Vec::new();
                if mat.is_some_and(|m| m.row_major) {
                    // Row r is at r * stride, its elements one column apiece.
                    let (size, _) = self.layout(component, explicit)?;
                    let s = Type::Scalar(self.scalar_of(component)?);
                    let col_ty = self.ir_ty(column)?;
                    for c in 0..count as u64 {
                        let mut lanes = Vec::new();
                        for r in 0..rows as u64 {
                            let a = self.offset(out, addr, r * stride + c * size as u64);
                            lanes.push(self.let_(out, s, Op::Load(a, s)));
                        }
                        columns.push(Val::Ir(self.let_(out, col_ty, Op::Construct(lanes))));
                    }
                } else {
                    for c in 0..count as u64 {
                        let a = self.offset(out, addr, c * stride);
                        columns.push(self.load_mem(out, a, column, explicit, None)?);
                    }
                }
                Ok(Val::Composite(columns))
            }
            SpvType::RuntimeArray { .. } => err("a load of a whole run-time array".into()),
            _ => {
                let want = self.ir_ty(ty)?;
                if want.scalar() == Some(Scalar::Bool) {
                    // Booleans in memory are 32-bit words, nonzero for true.
                    let word = want.with_scalar(Scalar::U32);
                    let w = self.let_(out, word, Op::Load(addr, word));
                    let zero = self.zero_like(out, word);
                    return Ok(Val::Ir(self.let_(out, want, Op::Compare(Cmp::Ne, w, zero))));
                }
                Ok(Val::Ir(self.let_(out, want, Op::Load(addr, want))))
            }
        }
    }

    fn zero_like(&mut self, out: &mut Block, ty: Type) -> Value {
        let s = ty.scalar().unwrap_or(Scalar::U32);
        let z = self.konst(out, s, 0);
        self.splat(out, z, ty)
    }

    fn splat(&mut self, out: &mut Block, scalar: Value, ty: Type) -> Value {
        match ty {
            Type::Vector(_, n) => self.let_(out, ty, Op::Construct(vec![scalar; n as usize])),
            _ => scalar,
        }
    }

    fn store(&mut self, out: &mut Block, ptr: &Ptr, val: Val) -> Result<(), Error> {
        match ptr {
            Ptr::Mem { addr, ty, explicit, mat } => self.store_mem(out, *addr, *ty, *explicit, *mat, val),
            Ptr::Fields(fields) => {
                let Val::Composite(parts) = val else { return err("a store of a value that isn't a struct to a block".into()) };
                for (f, p) in fields.iter().zip(parts) {
                    self.store(out, f, p)?;
                }
                Ok(())
            }
            Ptr::Reg { var, lane, .. } => {
                let Val::Ir(v) = val else { return err("a composite stored to a register variable".into()) };
                let whole_ty = self.e.vars[var.0 as usize];
                let new = match lane {
                    None => v,
                    Some(Lane::Const(i)) => {
                        let old = self.let_(out, whole_ty, Op::Read(*var));
                        self.let_(out, whole_ty, Op::Insert(old, *i, v))
                    }
                    Some(Lane::Dynamic(i)) => {
                        // A lane chosen at run time: each lane takes the new value if it's the one.
                        let old = self.let_(out, whole_ty, Op::Read(*var));
                        let index_ty = self.e.ty(*i);
                        let mut lanes = Vec::new();
                        for k in 0..whole_ty.lanes() {
                            let s = Type::Scalar(whole_ty.scalar().unwrap_or(Scalar::U32));
                            let current = self.let_(out, s, Op::Extract(old, k));
                            let kk = self.konst(out, index_ty.scalar().unwrap_or(Scalar::U32), k as u64);
                            let is = self.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, *i, kk));
                            lanes.push(self.let_(out, s, Op::Select(is, v, current)));
                        }
                        self.let_(out, whole_ty, Op::Construct(lanes))
                    }
                };
                out.push(Stmt::Write(*var, new));
                Ok(())
            }
            Ptr::Const { val, .. } => match &**val {
                Val::Unsupported(what) => err(format!("a write to {what}, which isn't translated yet")),
                _ => err("a store to an input".into()),
            },
        }
    }

    fn store_mem(&mut self, out: &mut Block, addr: Value, ty: u32, explicit: bool, mat: Option<Mat>, val: Val) -> Result<(), Error> {
        match (self.s.ty_of(ty)?.clone(), val) {
            (SpvType::Struct { members }, Val::Composite(parts)) => {
                for (i, (m, p)) in members.iter().zip(parts).enumerate() {
                    let at = self.member_offset(ty, i as u32, explicit)? as u64;
                    let a = self.offset(out, addr, at);
                    let mat = if explicit { self.member_mat(ty, i as u32) } else { None };
                    self.store_mem(out, a, *m, explicit, mat, p)?;
                }
                Ok(())
            }
            (SpvType::Array { element, .. }, Val::Composite(parts)) => {
                let stride = self.stride(ty, element, explicit)? as u64;
                for (i, p) in parts.into_iter().enumerate() {
                    let a = self.offset(out, addr, i as u64 * stride);
                    self.store_mem(out, a, element, explicit, mat, p)?;
                }
                Ok(())
            }
            (SpvType::Matrix { column, .. }, Val::Composite(columns)) => {
                let stride = match mat {
                    Some(m) => m.stride,
                    None => self.column_stride(column, explicit)?,
                } as u64;
                let SpvType::Vector { component, .. } = self.s.ty_of(column)?.clone() else {
                    return err("a matrix whose columns aren't vectors".into());
                };
                let (size, _) = self.layout(component, explicit)?;
                for (c, col) in columns.into_iter().enumerate() {
                    if mat.is_some_and(|m| m.row_major) {
                        let Val::Ir(v) = col else { return err("a matrix column that isn't a vector".into()) };
                        for (r, lane) in self.lanes_of(out, v).into_iter().enumerate() {
                            let a = self.offset(out, addr, r as u64 * stride + c as u64 * size as u64);
                            out.push(Stmt::Store(a, lane));
                        }
                    } else {
                        let a = self.offset(out, addr, c as u64 * stride);
                        self.store_mem(out, a, column, explicit, None, col)?;
                    }
                }
                Ok(())
            }
            (_, Val::Ir(v)) => {
                let have = self.e.ty(v);
                let v = if have.scalar() == Some(Scalar::Bool) {
                    let word = have.with_scalar(Scalar::U32);
                    let one = self.konst(out, Scalar::U32, 1);
                    let one = self.splat(out, one, word);
                    let zero = self.zero_like(out, word);
                    self.let_(out, word, Op::Select(v, one, zero))
                } else {
                    v
                };
                out.push(Stmt::Store(addr, v));
                Ok(())
            }
            (t, v) => err(format!("a store of {} to {t:?}", kind(&v))),
        }
    }

    // ---- control flow ---------------------------------------------------------------------------------------

    fn merge_of(&self, label: u32) -> Result<Option<Merge>, Error> {
        let block = self.block(label)?;
        for inst in &block.insts {
            match inst.op {
                op::SELECTION_MERGE => return Ok(Some(Merge::Selection(inst.arg(0)?))),
                op::LOOP_MERGE => return Ok(Some(Merge::Loop { merge: inst.arg(0)?, cont: inst.arg(1)? })),
                _ => {}
            }
        }
        Ok(None)
    }

    fn block(&self, label: u32) -> Result<&'s super::Block<'a>, Error> {
        match self.f.blocks.get(&label) {
            Some(b) => Ok(b),
            None => err(format!("a branch to %{label}, which isn't a block")),
        }
    }

    fn region(&mut self, mut label: u32, ctx: Ctx, out: &mut Block, depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return err(format!("control flow nested more than {MAX_DEPTH} deep"));
        }
        loop {
            if Some(label) == ctx.stop {
                return Ok(());
            }
            if Some(label) == ctx.merge {
                out.push(Stmt::Break);
                return Ok(());
            }
            if Some(label) == ctx.cont && !ctx.continuing {
                out.push(Stmt::Continue);
                return Ok(());
            }
            if Some(label) == ctx.header && ctx.continuing {
                return Ok(());
            }
            if self.stops.contains(&label) {
                return err(format!("a branch from a nested selection out to %{label} isn't translated yet"));
            }
            self.blocks += 1;
            if self.blocks > MAX_BLOCKS {
                return err("control flow that isn't structured (a cycle with no loop merge)".into());
            }
            let merge = self.merge_of(label)?;
            if let Some(Merge::Loop { merge, cont }) = merge {
                let inner = Ctx { stop: None, header: Some(label), merge: Some(merge), cont: Some(cont), continuing: false };
                let saved = core::mem::take(&mut self.stops);
                let mut body = Vec::new();
                let end = self.emit_block(label, &mut body)?;
                if let Some(next) = self.next(end, None, inner, &mut body, depth)? {
                    self.region(next, inner, &mut body, depth + 1)?;
                }
                let mut continuing = Vec::new();
                if cont != label {
                    self.region(cont, Ctx { continuing: true, ..inner }, &mut continuing, depth + 1)?;
                }
                self.stops = saved;
                out.push(Stmt::Loop { body, continuing, break_if: None });
                label = merge;
                continue;
            }
            let end = self.emit_block(label, out)?;
            match self.next(end, merge, ctx, out, depth)? {
                Some(next) => label = next,
                None => return Ok(()),
            }
        }
    }

    /// After a block's instructions: what its terminator does, and the label the region goes on with, if any.
    fn next(&mut self, end: End, merge: Option<Merge>, ctx: Ctx, out: &mut Block, depth: usize) -> Result<Option<u32>, Error> {
        match end {
            End::Return => {
                self.write_outputs(out);
                out.push(Stmt::Return);
                Ok(None)
            }
            End::Kill => {
                out.push(Stmt::Discard);
                Ok(None)
            }
            End::Goto(t) => Ok(Some(t)),
            End::Cond { cond, t, f } => {
                let selection = match merge {
                    Some(Merge::Selection(m)) => Some(m),
                    _ => None,
                };
                let inner = match selection {
                    Some(m) => Ctx { stop: Some(m), ..ctx },
                    None => ctx,
                };
                if let Some(m) = selection {
                    self.stops.push(m);
                }
                let mut then = Vec::new();
                let mut otherwise = Vec::new();
                let a = self.region(t, inner, &mut then, depth + 1);
                let b = a.and_then(|()| self.region(f, inner, &mut otherwise, depth + 1));
                if selection.is_some() {
                    self.stops.pop();
                }
                b?;
                out.push(Stmt::If { cond, then, otherwise });
                // With a merge, the region goes on there; without one, both sides left (or fell to the enclosing
                // construct's merge, which its own header goes on from).
                Ok(selection)
            }
            End::Switch { selector, default, cases } => {
                let Some(Merge::Selection(m)) = merge else { return err("an OpSwitch with no merge".into()) };
                let inner = Ctx { stop: Some(m), ..ctx };
                // Cases sharing a target become one test; each case is a region stopping at the merge (a case
                // that falls through into another just translates the other's blocks again).
                let mut targets: Vec<(u32, Vec<u64>)> = Vec::new();
                for (value, target) in cases {
                    if target == default {
                        continue;
                    }
                    match targets.iter_mut().find(|(t, _)| *t == target) {
                        Some((_, values)) => values.push(value),
                        None => targets.push((target, vec![value])),
                    }
                }
                self.stops.push(m);
                let mut chain = Vec::new();
                let r = self.region(default, inner, &mut chain, depth + 1);
                let mut result = r;
                for (target, values) in targets.into_iter().rev() {
                    if result.is_err() {
                        break;
                    }
                    let mut test = Vec::new();
                    let ty = self.e.ty(selector);
                    let s = ty.scalar().unwrap_or(Scalar::U32);
                    let mut cond: Option<Value> = None;
                    for v in values {
                        let k = self.konst(&mut test, s, v);
                        let eq = self.let_(&mut test, Type::BOOL, Op::Compare(Cmp::Eq, selector, k));
                        cond = Some(match cond {
                            Some(c) => self.let_(&mut test, Type::BOOL, Op::Binary(BinOp::Or, c, eq)),
                            None => eq,
                        });
                    }
                    let mut then = Vec::new();
                    result = self.region(target, inner, &mut then, depth + 1);
                    let otherwise = core::mem::take(&mut chain);
                    test.push(Stmt::If { cond: cond.unwrap_or(selector), then, otherwise });
                    chain = test;
                }
                self.stops.pop();
                result?;
                out.extend(chain);
                Ok(Some(m))
            }
        }
    }

    /// A block's instructions (its phis read, each successor's phis written before the branch), and how it ends.
    fn emit_block(&mut self, label: u32, out: &mut Block) -> Result<End, Error> {
        let insts = self.block(label)?.insts.clone();
        let Some((terminator, body)) = insts.split_last() else { return err(format!("%{label} is empty")) };
        for inst in body {
            match inst.op {
                op::PHI => {
                    let id = inst.arg(1)?;
                    let var = self.phis[&id];
                    let ty = self.e.vars[var.0 as usize];
                    let v = self.let_(out, ty, Op::Read(var));
                    self.set(id, Val::Ir(v));
                }
                op::SELECTION_MERGE | op::LOOP_MERGE => {}
                _ => self.inst(*inst, out).map_err(|e| Error {
                    message: format!("{}: {} in %{label}: {}", self.e.name, op::name(inst.op), e.message),
                })?,
            }
        }
        let end = match terminator.op {
            op::BRANCH => End::Goto(terminator.arg(0)?),
            op::BRANCH_CONDITIONAL => {
                let cond = self.ir(terminator.arg(0)?)?;
                End::Cond { cond, t: terminator.arg(1)?, f: terminator.arg(2)? }
            }
            op::SWITCH => {
                let selector = self.ir(terminator.arg(0)?)?;
                let wide = self.e.ty(selector).size() == 8;
                let step = if wide { 3 } else { 2 };
                let mut cases = Vec::new();
                for case in terminator.args[2..].chunks(step) {
                    let value = if wide { case[0] as u64 | (case[1] as u64) << 32 } else { case[0] as u64 };
                    cases.push((value, case[step - 1]));
                }
                End::Switch { selector, default: terminator.arg(1)?, cases }
            }
            op::RETURN | op::UNREACHABLE => End::Return,
            op::KILL | op::TERMINATE_INVOCATION => End::Kill,
            other => return err(format!("{} ends %{label}, which isn't translated yet", op::name(other))),
        };
        for succ in successors(&end) {
            self.write_phis(label, succ, out)?;
        }
        Ok(end)
    }

    fn write_phis(&mut self, from: u32, to: u32, out: &mut Block) -> Result<(), Error> {
        let insts = self.block(to)?.insts.clone();
        for inst in insts.iter().take_while(|i| i.op == op::PHI) {
            let id = inst.arg(1)?;
            for pair in inst.args[2..].chunks(2) {
                if pair.len() == 2 && pair[1] == from {
                    let v = self.ir(pair[0])?;
                    out.push(Stmt::Write(self.phis[&id], v));
                }
            }
        }
        Ok(())
    }

    // ---- instructions ---------------------------------------------------------------------------------------

    fn unsigned(ty: Type) -> Type {
        match ty.scalar() {
            Some(Scalar::I32) => ty.with_scalar(Scalar::U32),
            Some(Scalar::I64) => ty.with_scalar(Scalar::U64),
            _ => ty,
        }
    }

    fn signed(ty: Type) -> Type {
        match ty.scalar() {
            Some(Scalar::U32) => ty.with_scalar(Scalar::I32),
            Some(Scalar::U64) => ty.with_scalar(Scalar::I64),
            _ => ty,
        }
    }

    /// `op` on `a` and `b` read as `semantic` (signed or unsigned), the result given back as `result`.
    fn binary(&mut self, out: &mut Block, op: BinOp, a: Value, b: Value, semantic: Type, result: Type) -> Value {
        let a = self.as_ty(out, a, semantic);
        let b = self.as_ty(out, b, semantic);
        let r = self.let_(out, semantic, Op::Binary(op, a, b));
        self.as_ty(out, r, result)
    }

    fn compare(&mut self, out: &mut Block, cmp: Cmp, a: Value, b: Value, semantic: Type) -> Value {
        let a = self.as_ty(out, a, semantic);
        let b = self.as_ty(out, b, semantic);
        self.let_(out, semantic.with_scalar(Scalar::Bool), Op::Compare(cmp, a, b))
    }

    fn lanes_of(&mut self, out: &mut Block, v: Value) -> Vec<Value> {
        match self.e.ty(v) {
            Type::Vector(s, n) => (0..n).map(|i| self.let_(out, Type::Scalar(s), Op::Extract(v, i))).collect(),
            _ => vec![v],
        }
    }

    fn inst(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let a = |i: usize| inst.arg(i);
        match inst.op {
            op::VARIABLE => {
                let SpvType::Pointer { pointee, .. } = self.s.ty_of(a(0)?)?.clone() else { return err("a variable's type".into()) };
                let val = self.local(out, pointee, inst.args.get(3).copied())?;
                self.set(a(1)?, val);
            }
            op::LOAD => {
                let ptr = self.ptr(a(2)?)?;
                let val = self.load(out, &ptr)?;
                self.set(a(1)?, val);
            }
            op::STORE => {
                let ptr = self.ptr(a(0)?)?;
                let val = self.val(a(1)?)?;
                self.store(out, &ptr, val)?;
            }
            op::COPY_MEMORY => {
                let to = self.ptr(a(0)?)?;
                let from = self.ptr(a(1)?)?;
                let val = self.load(out, &from)?;
                self.store(out, &to, val)?;
            }
            op::ACCESS_CHAIN | op::IN_BOUNDS_ACCESS_CHAIN => {
                let base = self.ptr(a(2)?)?;
                let p = self.access(out, base, &inst.args[3..])?;
                self.set(a(1)?, Val::Ptr(p));
            }
            op::COPY_OBJECT => {
                let v = self.val(a(2)?)?;
                self.set(a(1)?, v);
            }
            op::UNDEF => {
                let z = self.zero(out, a(0)?)?;
                self.set(a(1)?, z);
            }
            op::FUNCTION_CALL => {
                return err("function calls aren't translated yet (spirv-opt --inline-entry-points-exhaustive inlines them)".into());
            }
            op::NOP | op::LINE | op::NO_LINE => {}
            op::COMPOSITE_CONSTRUCT => {
                let rt = a(0)?;
                let mut parts = Vec::new();
                for &id in &inst.args[2..] {
                    parts.push(self.val(id)?);
                }
                let val = if let SpvType::Vector { .. } = self.s.ty_of(rt)? {
                    let ty = self.ir_ty(rt)?;
                    let mut lanes = Vec::new();
                    for p in parts {
                        let Val::Ir(v) = p else { return err("a vector of composites".into()) };
                        lanes.extend(self.lanes_of(out, v));
                    }
                    Val::Ir(self.let_(out, ty, Op::Construct(lanes)))
                } else {
                    Val::Composite(parts)
                };
                self.set(a(1)?, val);
            }
            op::COMPOSITE_EXTRACT => {
                let mut val = self.val(a(2)?)?;
                for &i in &inst.args[3..] {
                    val = match val {
                        Val::Composite(parts) => parts.get(i as usize).cloned().ok_or_else(|| Error { message: format!("no part {i}") })?,
                        Val::Ir(v) => {
                            let s = self.e.ty(v).scalar().unwrap_or(Scalar::U32);
                            Val::Ir(self.let_(out, Type::Scalar(s), Op::Extract(v, i as u8)))
                        }
                        other => return err(format!("extracting from {}", kind(&other))),
                    };
                }
                self.set(a(1)?, val);
            }
            op::COMPOSITE_INSERT => {
                let object = self.val(a(2)?)?;
                let composite = self.val(a(3)?)?;
                let val = self.insert(out, composite, &inst.args[4..], object)?;
                self.set(a(1)?, val);
            }
            op::VECTOR_SHUFFLE => {
                let ty = self.ir_ty(a(0)?)?;
                let v1 = self.ir(a(2)?)?;
                let v2 = self.ir(a(3)?)?;
                let mut all = self.lanes_of(out, v1);
                all.extend(self.lanes_of(out, v2));
                let lanes: Vec<Value> = inst.args[4..].iter().map(|&c| all.get(c as usize).copied().unwrap_or(all[0])).collect();
                let v = self.let_(out, ty, Op::Construct(lanes));
                self.set(a(1)?, Val::Ir(v));
            }
            op::VECTOR_EXTRACT_DYNAMIC => {
                let ty = self.ir_ty(a(0)?)?;
                let (v, i) = (self.ir(a(2)?)?, self.ir(a(3)?)?);
                let r = self.let_(out, ty, Op::ExtractDynamic(v, i));
                self.set(a(1)?, Val::Ir(r));
            }
            op::VECTOR_INSERT_DYNAMIC => {
                let ty = self.ir_ty(a(0)?)?;
                let (v, x, i) = (self.ir(a(2)?)?, self.ir(a(3)?)?, self.ir(a(4)?)?);
                let index_ty = self.e.ty(i).scalar().unwrap_or(Scalar::U32);
                let mut lanes = Vec::new();
                for k in 0..ty.lanes() {
                    let s = Type::Scalar(ty.scalar().unwrap_or(Scalar::U32));
                    let current = self.let_(out, s, Op::Extract(v, k));
                    let kk = self.konst(out, index_ty, k as u64);
                    let is = self.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, i, kk));
                    lanes.push(self.let_(out, s, Op::Select(is, x, current)));
                }
                let r = self.let_(out, ty, Op::Construct(lanes));
                self.set(a(1)?, Val::Ir(r));
            }
            op::SELECT => {
                let cond = self.ir(a(2)?)?;
                let (t, f) = (self.val(a(3)?)?, self.val(a(4)?)?);
                let val = self.select(out, cond, t, f)?;
                self.set(a(1)?, val);
            }
            op::EXT_INST => self.ext_inst(inst, out)?,
            op::CONTROL_BARRIER | op::MEMORY_BARRIER => {
                let (scope, semantics) = if inst.op == op::CONTROL_BARRIER { (a(0)?, a(2)?) } else { (a(0)?, a(1)?) };
                let semantics = self.s.const_int(semantics).unwrap_or(0) as u32;
                let execution = inst.op == op::CONTROL_BARRIER && self.s.const_int(scope) == Some(SCOPE_WORKGROUP);
                out.push(Stmt::Barrier(Barrier {
                    execution,
                    workgroup_memory: semantics & op::SEMANTICS_WORKGROUP_MEMORY != 0,
                    device_memory: semantics & (op::SEMANTICS_UNIFORM_MEMORY | op::SEMANTICS_IMAGE_MEMORY) != 0,
                }));
            }
            op::ATOMIC_LOAD..=op::ATOMIC_XOR => self.atomic(inst, out)?,
            op::GROUP_NON_UNIFORM_ELECT..=op::GROUP_NON_UNIFORM_LOGICAL_XOR => self.group(inst, out)?,
            op::SAMPLED_IMAGE => {
                let val = match (self.val(a(2)?)?, self.val(a(3)?)?) {
                    (Val::Image(image), Val::Sampler(sampler)) => Val::SampledImage { image, sampler: Some(sampler) },
                    (i, s) => return err(format!("OpSampledImage of {} and {}", kind(&i), kind(&s))),
                };
                self.set(a(1)?, val);
            }
            op::IMAGE => {
                let val = match self.val(a(2)?)? {
                    Val::SampledImage { image, sampler: Some(_) } => Val::Image(image),
                    Val::SampledImage { sampler: None, .. } => Val::Unsupported("the image of a combined image and sampler"),
                    other => return err(format!("OpImage of {}", kind(&other))),
                };
                self.set(a(1)?, val);
            }
            op::IMAGE_SAMPLE_IMPLICIT_LOD | op::IMAGE_SAMPLE_EXPLICIT_LOD => self.sample(inst, out)?,
            op::DPDX..=op::FWIDTH_COARSE => {
                let v = self.ir(a(2)?)?;
                let ty = self.e.ty(v);
                let r = match inst.op {
                    op::DPDX | op::DPDX_FINE | op::DPDX_COARSE => self.let_(out, ty, Op::Derivative(Axis::X, v)),
                    op::DPDY | op::DPDY_FINE | op::DPDY_COARSE => self.let_(out, ty, Op::Derivative(Axis::Y, v)),
                    _ => {
                        let dx = self.let_(out, ty, Op::Derivative(Axis::X, v));
                        let dy = self.let_(out, ty, Op::Derivative(Axis::Y, v));
                        let dx = self.let_(out, ty, Op::Unary(UnOp::Abs, dx));
                        let dy = self.let_(out, ty, Op::Unary(UnOp::Abs, dy));
                        self.let_(out, ty, Op::Binary(BinOp::Add, dx, dy))
                    }
                };
                self.set(a(1)?, Val::Ir(r));
            }
            op::TRANSPOSE | op::MATRIX_TIMES_SCALAR..=op::OUTER_PRODUCT => self.matrix(inst, out)?,
            _ => self.arith(inst, out)?,
        }
        Ok(())
    }

    /// `OpImageSample*Lod`: result type, result, sampled image, coordinate, then the image operands' mask and
    /// operands (a bias or a level; others aren't translated yet).
    fn sample(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let a = |i: usize| inst.arg(i);
        let rt = self.ir_ty(a(0)?)?;
        if rt != Type::Vector(Scalar::F32, 4) {
            return err(format!("a sample giving {rt:?}: only float images are translated yet"));
        }
        let Val::SampledImage { image, sampler } = self.val(a(2)?)? else { return err("a sample of something that isn't a sampled image".into()) };
        let c = self.ir(a(3)?)?;
        let coord = match self.e.ty(c) {
            Type::Vector(Scalar::F32, 2) => c,
            Type::Vector(Scalar::F32, n) if n > 2 => {
                let (x, y) = (self.let_(out, Type::F32, Op::Extract(c, 0)), self.let_(out, Type::F32, Op::Extract(c, 1)));
                self.let_(out, Type::Vector(Scalar::F32, 2), Op::Construct(vec![x, y]))
            }
            ty => return err(format!("a sample at a {ty:?}: 2D coordinates are float vectors")),
        };
        let mask = inst.args.get(4).copied().unwrap_or(0);
        if mask & !(op::IMAGE_OPERAND_BIAS | op::IMAGE_OPERAND_LOD) != 0 {
            return err(format!("image operands {mask:#x} (gradients, offsets) aren't translated yet"));
        }
        let mut next = 5;
        let mut lod = Lod::Implicit;
        if mask & op::IMAGE_OPERAND_BIAS != 0 {
            lod = Lod::Bias(self.ir(a(next)?)?);
            next += 1;
        }
        if mask & op::IMAGE_OPERAND_LOD != 0 {
            lod = Lod::Level(self.ir(a(next)?)?);
        }
        if inst.op == op::IMAGE_SAMPLE_EXPLICIT_LOD && !matches!(lod, Lod::Level(_)) {
            return err("an explicit-level sample without a level".into());
        }
        let v = self.let_(out, rt, Op::Sample { image, sampler, coord, lod });
        self.set(a(1)?, Val::Ir(v));
        Ok(())
    }

    /// A matrix's columns, as vectors.
    fn columns(&self, id: u32) -> Result<Vec<Value>, Error> {
        match self.val(id)? {
            Val::Composite(parts) => parts
                .into_iter()
                .map(|p| match p {
                    Val::Ir(v) => Ok(v),
                    other => err(format!("a matrix column that's {}", kind(&other))),
                })
                .collect(),
            other => err(format!("%{id} is {}, where a matrix is wanted", kind(&other))),
        }
    }

    fn dot(&mut self, out: &mut Block, p: Value, q: Value) -> Value {
        let ty = self.e.ty(p);
        let s = Type::Scalar(ty.scalar().unwrap_or(Scalar::F32));
        let m = self.let_(out, ty, Op::Binary(BinOp::Mul, p, q));
        let lanes = self.lanes_of(out, m);
        let mut sum = lanes[0];
        for l in &lanes[1..] {
            sum = self.let_(out, s, Op::Binary(BinOp::Add, sum, *l));
        }
        sum
    }

    /// `m` (columns) times the vector `v`: each column scaled by `v`'s lane and summed.
    fn mat_vec(&mut self, out: &mut Block, m: &[Value], v: Value) -> Value {
        let lanes = self.lanes_of(out, v);
        let ty = self.e.ty(m[0]);
        let mut sum: Option<Value> = None;
        for (c, l) in m.iter().zip(lanes) {
            let k = self.splat(out, l, ty);
            let term = self.let_(out, ty, Op::Binary(BinOp::Mul, *c, k));
            sum = Some(match sum {
                Some(s) => self.let_(out, ty, Op::Binary(BinOp::Add, s, term)),
                None => term,
            });
        }
        sum.unwrap_or(m[0])
    }

    /// The matrix instructions, as vector arithmetic on columns.
    fn matrix(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let a = |i: usize| inst.arg(i);
        let val = match inst.op {
            op::MATRIX_TIMES_SCALAR => {
                let m = self.columns(a(2)?)?;
                let k = self.ir(a(3)?)?;
                let mut cols = Vec::new();
                for c in m {
                    let ty = self.e.ty(c);
                    let kk = self.splat(out, k, ty);
                    cols.push(Val::Ir(self.let_(out, ty, Op::Binary(BinOp::Mul, c, kk))));
                }
                Val::Composite(cols)
            }
            op::VECTOR_TIMES_MATRIX => {
                let v = self.ir(a(2)?)?;
                let m = self.columns(a(3)?)?;
                let lanes: Vec<Value> = m.iter().map(|c| self.dot(out, v, *c)).collect();
                let rt = self.ir_ty(a(0)?)?;
                Val::Ir(if lanes.len() == 1 { lanes[0] } else { self.let_(out, rt, Op::Construct(lanes)) })
            }
            op::MATRIX_TIMES_VECTOR => {
                let m = self.columns(a(2)?)?;
                let v = self.ir(a(3)?)?;
                Val::Ir(self.mat_vec(out, &m, v))
            }
            op::MATRIX_TIMES_MATRIX => {
                let l = self.columns(a(2)?)?;
                let r = self.columns(a(3)?)?;
                Val::Composite(r.iter().map(|c| Val::Ir(self.mat_vec(out, &l, *c))).collect())
            }
            op::OUTER_PRODUCT => {
                let p = self.ir(a(2)?)?;
                let q = self.ir(a(3)?)?;
                let ty = self.e.ty(p);
                let mut cols = Vec::new();
                for l in self.lanes_of(out, q) {
                    let k = self.splat(out, l, ty);
                    cols.push(Val::Ir(self.let_(out, ty, Op::Binary(BinOp::Mul, p, k))));
                }
                Val::Composite(cols)
            }
            _ => {
                // Transpose: column j of the result is lane j of every column.
                let m = self.columns(a(2)?)?;
                let SpvType::Matrix { column, .. } = self.s.ty_of(a(0)?)?.clone() else { return err("a transpose's type".into()) };
                let col_ty = self.ir_ty(column)?;
                let s = Type::Scalar(col_ty.scalar().unwrap_or(Scalar::F32));
                let rows = self.e.ty(m[0]).lanes();
                let mut cols = Vec::new();
                for j in 0..rows {
                    let lanes: Vec<Value> = m.iter().map(|c| self.let_(out, s, Op::Extract(*c, j))).collect();
                    cols.push(Val::Ir(self.let_(out, col_ty, Op::Construct(lanes))));
                }
                Val::Composite(cols)
            }
        };
        self.set(a(1)?, val);
        Ok(())
    }

    /// An `f32` constant, splatted to `ty`'s shape.
    fn fconst(&mut self, out: &mut Block, x: f32, ty: Type) -> Value {
        let k = self.konst(out, Scalar::F32, x.to_bits() as u64);
        self.splat(out, k, ty)
    }

    fn fbin(&mut self, out: &mut Block, op: BinOp, p: Value, q: Value) -> Value {
        let ty = self.e.ty(p);
        self.let_(out, ty, Op::Binary(op, p, q))
    }

    fn funary(&mut self, out: &mut Block, op: UnOp, p: Value) -> Value {
        let ty = self.e.ty(p);
        self.let_(out, ty, Op::Unary(op, p))
    }

    fn length(&mut self, out: &mut Block, v: Value) -> Value {
        match self.e.ty(v) {
            Type::Vector(..) => {
                let d = self.dot(out, v, v);
                self.funary(out, UnOp::Sqrt, d)
            }
            _ => self.funary(out, UnOp::Abs, v),
        }
    }

    fn insert(&mut self, out: &mut Block, composite: Val, path: &[u32], object: Val) -> Result<Val, Error> {
        let Some((&first, rest)) = path.split_first() else { return Ok(object) };
        match composite {
            Val::Composite(mut parts) => {
                let i = first as usize;
                let inner = parts.get(i).cloned().ok_or_else(|| Error { message: format!("no part {i}") })?;
                parts[i] = self.insert(out, inner, rest, object)?;
                Ok(Val::Composite(parts))
            }
            Val::Ir(v) => {
                let Val::Ir(x) = object else { return err("a composite into a vector lane".into()) };
                let ty = self.e.ty(v);
                Ok(Val::Ir(self.let_(out, ty, Op::Insert(v, first as u8, x))))
            }
            other => err(format!("inserting into {}", kind(&other))),
        }
    }

    fn select(&mut self, out: &mut Block, cond: Value, t: Val, f: Val) -> Result<Val, Error> {
        match (t, f) {
            (Val::Ir(t), Val::Ir(f)) => {
                let ty = self.e.ty(t);
                Ok(Val::Ir(self.let_(out, ty, Op::Select(cond, t, f))))
            }
            (Val::Composite(ts), Val::Composite(fs)) => {
                let mut parts = Vec::new();
                for (t, f) in ts.into_iter().zip(fs) {
                    parts.push(self.select(out, cond, t, f)?);
                }
                Ok(Val::Composite(parts))
            }
            _ => err("selecting between pointers".into()),
        }
    }

    fn ext_inst(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        if !self.s.glsl.contains(&inst.arg(2)?) {
            return err("an extended instruction set other than GLSL.std.450".into());
        }
        let rt = self.ir_ty(inst.arg(0)?)?;
        let which = inst.arg(3)?;
        let mut x = Vec::new();
        for &id in &inst.args[4..] {
            x.push(self.ir(id)?);
        }
        let arg = |i: usize| x.get(i).copied().ok_or_else(|| Error { message: format!("GLSL.std.450 {which}'s operand {i}") });
        let (u, s) = (Self::unsigned(rt), Self::signed(rt));
        let v = match which {
            op::GLSL_F_ABS => self.let_(out, rt, Op::Unary(UnOp::Abs, arg(0)?)),
            op::GLSL_S_ABS => {
                let v = self.as_ty(out, arg(0)?, s);
                let r = self.let_(out, s, Op::Unary(UnOp::Abs, v));
                self.as_ty(out, r, rt)
            }
            op::GLSL_FLOOR => self.let_(out, rt, Op::Unary(UnOp::Floor, arg(0)?)),
            op::GLSL_CEIL => self.let_(out, rt, Op::Unary(UnOp::Ceil, arg(0)?)),
            op::GLSL_TRUNC => self.let_(out, rt, Op::Unary(UnOp::Trunc, arg(0)?)),
            op::GLSL_SQRT => self.let_(out, rt, Op::Unary(UnOp::Sqrt, arg(0)?)),
            op::GLSL_F_MIN => self.binary(out, BinOp::Min, arg(0)?, arg(1)?, rt, rt),
            op::GLSL_F_MAX => self.binary(out, BinOp::Max, arg(0)?, arg(1)?, rt, rt),
            op::GLSL_U_MIN => self.binary(out, BinOp::Min, arg(0)?, arg(1)?, u, rt),
            op::GLSL_U_MAX => self.binary(out, BinOp::Max, arg(0)?, arg(1)?, u, rt),
            op::GLSL_S_MIN => self.binary(out, BinOp::Min, arg(0)?, arg(1)?, s, rt),
            op::GLSL_S_MAX => self.binary(out, BinOp::Max, arg(0)?, arg(1)?, s, rt),
            op::GLSL_F_CLAMP | op::GLSL_U_CLAMP | op::GLSL_S_CLAMP => {
                let semantic = match which {
                    op::GLSL_U_CLAMP => u,
                    op::GLSL_S_CLAMP => s,
                    _ => rt,
                };
                let low = self.binary(out, BinOp::Max, arg(0)?, arg(1)?, semantic, semantic);
                self.binary(out, BinOp::Min, low, arg(2)?, semantic, rt)
            }
            // Not fused: the same as a multiply then an add on every backend.
            op::GLSL_FMA => {
                let m = self.let_(out, rt, Op::Binary(BinOp::Mul, arg(0)?, arg(1)?));
                self.let_(out, rt, Op::Binary(BinOp::Add, m, arg(2)?))
            }
            op::GLSL_N_MIN => self.binary(out, BinOp::Min, arg(0)?, arg(1)?, rt, rt),
            op::GLSL_N_MAX => self.binary(out, BinOp::Max, arg(0)?, arg(1)?, rt, rt),
            op::GLSL_N_CLAMP => {
                let low = self.binary(out, BinOp::Max, arg(0)?, arg(1)?, rt, rt);
                self.binary(out, BinOp::Min, low, arg(2)?, rt, rt)
            }
            op::GLSL_ROUND | op::GLSL_ROUND_EVEN => self.funary(out, UnOp::RoundEven, arg(0)?),
            op::GLSL_F_SIGN => {
                let x = arg(0)?;
                let (zero, one, minus) = (self.fconst(out, 0.0, rt), self.fconst(out, 1.0, rt), self.fconst(out, -1.0, rt));
                let bools = rt.with_scalar(Scalar::Bool);
                let positive = self.let_(out, bools, Op::Compare(Cmp::Gt, x, zero));
                let negative = self.let_(out, bools, Op::Compare(Cmp::Lt, x, zero));
                let low = self.let_(out, rt, Op::Select(negative, minus, x));
                self.let_(out, rt, Op::Select(positive, one, low))
            }
            op::GLSL_FRACT => {
                let fl = self.funary(out, UnOp::Floor, arg(0)?);
                self.fbin(out, BinOp::Sub, arg(0)?, fl)
            }
            op::GLSL_RADIANS | op::GLSL_DEGREES => {
                let k = if which == op::GLSL_RADIANS { core::f32::consts::PI / 180.0 } else { 180.0 / core::f32::consts::PI };
                let k = self.fconst(out, k, rt);
                self.fbin(out, BinOp::Mul, arg(0)?, k)
            }
            op::GLSL_SIN => self.funary(out, UnOp::Sin, arg(0)?),
            op::GLSL_COS => self.funary(out, UnOp::Cos, arg(0)?),
            op::GLSL_TAN => {
                let (sn, cs) = (self.funary(out, UnOp::Sin, arg(0)?), self.funary(out, UnOp::Cos, arg(0)?));
                self.fbin(out, BinOp::Div, sn, cs)
            }
            op::GLSL_EXP2 => self.funary(out, UnOp::Exp2, arg(0)?),
            op::GLSL_LOG2 => self.funary(out, UnOp::Log2, arg(0)?),
            op::GLSL_EXP => {
                let k = self.fconst(out, core::f32::consts::LOG2_E, rt);
                let x = self.fbin(out, BinOp::Mul, arg(0)?, k);
                self.funary(out, UnOp::Exp2, x)
            }
            op::GLSL_LOG => {
                let l = self.funary(out, UnOp::Log2, arg(0)?);
                let k = self.fconst(out, core::f32::consts::LN_2, rt);
                self.fbin(out, BinOp::Mul, l, k)
            }
            op::GLSL_POW => {
                let l = self.funary(out, UnOp::Log2, arg(0)?);
                let p = self.fbin(out, BinOp::Mul, arg(1)?, l);
                self.funary(out, UnOp::Exp2, p)
            }
            op::GLSL_INVERSE_SQRT => {
                let r = self.funary(out, UnOp::Sqrt, arg(0)?);
                let one = self.fconst(out, 1.0, rt);
                self.fbin(out, BinOp::Div, one, r)
            }
            op::GLSL_F_MIX => {
                let d = self.fbin(out, BinOp::Sub, arg(1)?, arg(0)?);
                let t = self.fbin(out, BinOp::Mul, d, arg(2)?);
                self.fbin(out, BinOp::Add, arg(0)?, t)
            }
            op::GLSL_STEP => {
                let (zero, one) = (self.fconst(out, 0.0, rt), self.fconst(out, 1.0, rt));
                let below = self.let_(out, rt.with_scalar(Scalar::Bool), Op::Compare(Cmp::Lt, arg(1)?, arg(0)?));
                self.let_(out, rt, Op::Select(below, zero, one))
            }
            op::GLSL_SMOOTH_STEP => {
                let num = self.fbin(out, BinOp::Sub, arg(2)?, arg(0)?);
                let den = self.fbin(out, BinOp::Sub, arg(1)?, arg(0)?);
                let t = self.fbin(out, BinOp::Div, num, den);
                let (zero, one) = (self.fconst(out, 0.0, rt), self.fconst(out, 1.0, rt));
                let t = self.fbin(out, BinOp::Max, t, zero);
                let t = self.fbin(out, BinOp::Min, t, one);
                let (two, three) = (self.fconst(out, 2.0, rt), self.fconst(out, 3.0, rt));
                let tt = self.fbin(out, BinOp::Mul, t, t);
                let twice = self.fbin(out, BinOp::Mul, two, t);
                let k = self.fbin(out, BinOp::Sub, three, twice);
                self.fbin(out, BinOp::Mul, tt, k)
            }
            op::GLSL_LENGTH => self.length(out, arg(0)?),
            op::GLSL_DISTANCE => {
                let d = self.fbin(out, BinOp::Sub, arg(0)?, arg(1)?);
                self.length(out, d)
            }
            op::GLSL_CROSS => {
                let (p, q) = (self.lanes_of(out, arg(0)?), self.lanes_of(out, arg(1)?));
                if p.len() != 3 || q.len() != 3 {
                    return err("a cross product of vectors that aren't vec3".into());
                }
                let mut lanes = Vec::new();
                for (i, j) in [(1, 2), (2, 0), (0, 1)] {
                    let l = self.fbin(out, BinOp::Mul, p[i], q[j]);
                    let r = self.fbin(out, BinOp::Mul, p[j], q[i]);
                    lanes.push(self.fbin(out, BinOp::Sub, l, r));
                }
                self.let_(out, rt, Op::Construct(lanes))
            }
            op::GLSL_NORMALIZE => {
                let len = self.length(out, arg(0)?);
                let len = self.splat(out, len, rt);
                self.fbin(out, BinOp::Div, arg(0)?, len)
            }
            op::GLSL_FACE_FORWARD => {
                let d = self.dot(out, arg(2)?, arg(1)?);
                let zero = self.fconst(out, 0.0, Type::F32);
                let facing = self.let_(out, Type::BOOL, Op::Compare(Cmp::Lt, d, zero));
                let neg = self.funary(out, UnOp::Neg, arg(0)?);
                self.let_(out, rt, Op::Select(facing, arg(0)?, neg))
            }
            op::GLSL_REFLECT => {
                let d = self.dot(out, arg(1)?, arg(0)?);
                let two = self.fconst(out, 2.0, Type::F32);
                let d2 = self.fbin(out, BinOp::Mul, d, two);
                let k = self.splat(out, d2, rt);
                let scaled = self.fbin(out, BinOp::Mul, k, arg(1)?);
                self.fbin(out, BinOp::Sub, arg(0)?, scaled)
            }
            _ => return err(format!("GLSL.std.450 instruction {which} isn't translated yet")),
        };
        self.set(inst.arg(1)?, Val::Ir(v));
        Ok(())
    }

    fn atomic(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let a = |i: usize| inst.arg(i);
        if inst.op == op::ATOMIC_STORE {
            let Ptr::Mem { addr, ty, .. } = self.ptr(a(0)?)? else { return err("an atomic on a register".into()) };
            let s = self.scalar_of(ty)?;
            let v = self.ir(a(3)?)?;
            let v = self.as_ty(out, v, Type::Scalar(s));
            self.let_(out, Type::Scalar(s), Op::Atomic(AtomicOp::Exchange, addr, v, s));
            return Ok(());
        }
        let rt = self.ir_ty(a(0)?)?;
        let Ptr::Mem { addr, .. } = self.ptr(a(2)?)? else { return err("an atomic on a register".into()) };
        let r = rt.scalar().unwrap_or(Scalar::U32);
        let (semantic, aop) = match inst.op {
            op::ATOMIC_LOAD => (rt, AtomicOp::Add),
            op::ATOMIC_I_INCREMENT | op::ATOMIC_I_ADD => (rt, AtomicOp::Add),
            op::ATOMIC_I_DECREMENT | op::ATOMIC_I_SUB => (rt, AtomicOp::Sub),
            op::ATOMIC_EXCHANGE => (rt, AtomicOp::Exchange),
            op::ATOMIC_S_MIN => (Self::signed(rt), AtomicOp::Min),
            op::ATOMIC_U_MIN => (Self::unsigned(rt), AtomicOp::Min),
            op::ATOMIC_S_MAX => (Self::signed(rt), AtomicOp::Max),
            op::ATOMIC_U_MAX => (Self::unsigned(rt), AtomicOp::Max),
            op::ATOMIC_AND => (rt, AtomicOp::And),
            op::ATOMIC_OR => (rt, AtomicOp::Or),
            op::ATOMIC_XOR => (rt, AtomicOp::Xor),
            op::ATOMIC_COMPARE_EXCHANGE => {
                let v = self.ir(a(6)?)?;
                let c = self.ir(a(7)?)?;
                let old = self.let_(out, rt, Op::CompareExchange(addr, c, v, r));
                self.set(a(1)?, Val::Ir(old));
                return Ok(());
            }
            _ => return err("this atomic isn't translated yet".into()),
        };
        let s = semantic.scalar().unwrap_or(r);
        let operand = match inst.op {
            op::ATOMIC_LOAD => self.konst(out, s, 0),
            op::ATOMIC_I_INCREMENT | op::ATOMIC_I_DECREMENT => self.konst(out, s, 1),
            _ => {
                let v = self.ir(a(5)?)?;
                self.as_ty(out, v, semantic)
            }
        };
        let old = self.let_(out, semantic, Op::Atomic(aop, addr, operand, s));
        let old = self.as_ty(out, old, rt);
        self.set(a(1)?, Val::Ir(old));
        Ok(())
    }

    fn group(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let a = |i: usize| inst.arg(i);
        let rt = self.ir_ty(a(0)?)?;
        if self.s.const_int(a(2)?) != Some(SCOPE_SUBGROUP) {
            return err("a group operation across more than a subgroup".into());
        }
        let shuffle = |op| -> (SubgroupOp, bool) { (op, true) };
        let (sop, lane) = match inst.op {
            op::GROUP_NON_UNIFORM_ELECT => {
                let v = self.let_(out, Type::BOOL, Op::Subgroup(SubgroupOp::Elect, None, None));
                self.set(a(1)?, Val::Ir(v));
                return Ok(());
            }
            op::GROUP_NON_UNIFORM_ALL => (SubgroupOp::All, false),
            op::GROUP_NON_UNIFORM_ANY => (SubgroupOp::Any, false),
            op::GROUP_NON_UNIFORM_BROADCAST | op::GROUP_NON_UNIFORM_SHUFFLE => shuffle(SubgroupOp::Shuffle),
            op::GROUP_NON_UNIFORM_SHUFFLE_XOR => shuffle(SubgroupOp::ShuffleXor),
            op::GROUP_NON_UNIFORM_SHUFFLE_UP => shuffle(SubgroupOp::ShuffleUp),
            op::GROUP_NON_UNIFORM_SHUFFLE_DOWN => shuffle(SubgroupOp::ShuffleDown),
            op::GROUP_NON_UNIFORM_BROADCAST_FIRST => (SubgroupOp::BroadcastFirst, false),
            op::GROUP_NON_UNIFORM_I_ADD..=op::GROUP_NON_UNIFORM_LOGICAL_XOR => {
                let (op, semantic) = match inst.op {
                    op::GROUP_NON_UNIFORM_I_ADD | op::GROUP_NON_UNIFORM_F_ADD => (BinOp::Add, rt),
                    op::GROUP_NON_UNIFORM_I_MUL | op::GROUP_NON_UNIFORM_F_MUL => (BinOp::Mul, rt),
                    op::GROUP_NON_UNIFORM_S_MIN => (BinOp::Min, Self::signed(rt)),
                    op::GROUP_NON_UNIFORM_U_MIN => (BinOp::Min, Self::unsigned(rt)),
                    op::GROUP_NON_UNIFORM_F_MIN => (BinOp::Min, rt),
                    op::GROUP_NON_UNIFORM_S_MAX => (BinOp::Max, Self::signed(rt)),
                    op::GROUP_NON_UNIFORM_U_MAX => (BinOp::Max, Self::unsigned(rt)),
                    op::GROUP_NON_UNIFORM_F_MAX => (BinOp::Max, rt),
                    op::GROUP_NON_UNIFORM_BITWISE_AND | op::GROUP_NON_UNIFORM_LOGICAL_AND => (BinOp::And, rt),
                    op::GROUP_NON_UNIFORM_BITWISE_OR | op::GROUP_NON_UNIFORM_LOGICAL_OR => (BinOp::Or, rt),
                    op::GROUP_NON_UNIFORM_BITWISE_XOR | op::GROUP_NON_UNIFORM_LOGICAL_XOR => (BinOp::Xor, rt),
                    _ => return err("this subgroup operation isn't translated yet".into()),
                };
                let sop = match a(3)? {
                    op::GROUP_REDUCE => SubgroupOp::Reduce(op),
                    op::GROUP_INCLUSIVE_SCAN => SubgroupOp::InclusiveScan(op),
                    op::GROUP_EXCLUSIVE_SCAN => SubgroupOp::ExclusiveScan(op),
                    other => return err(format!("group operation {other} isn't translated yet")),
                };
                let v = self.ir(a(4)?)?;
                let v = self.as_ty(out, v, semantic);
                let r = self.let_(out, semantic, Op::Subgroup(sop, Some(v), None));
                let r = self.as_ty(out, r, rt);
                self.set(a(1)?, Val::Ir(r));
                return Ok(());
            }
            _ => return err("this subgroup operation isn't translated yet".into()),
        };
        let v = self.ir(a(3)?)?;
        let lane = if lane {
            let l = self.ir(a(4)?)?;
            Some(self.as_ty(out, l, Type::U32))
        } else {
            None
        };
        let r = self.let_(out, rt, Op::Subgroup(sop, Some(v), lane));
        self.set(a(1)?, Val::Ir(r));
        Ok(())
    }

    /// Arithmetic, logic, comparisons and conversions: `result type, result, operands...`.
    fn arith(&mut self, inst: Inst<'_>, out: &mut Block) -> Result<(), Error> {
        let rt = self.ir_ty(inst.arg(0)?)?;
        let id = inst.arg(1)?;
        let x = |me: &Self, i: usize| -> Result<Value, Error> { me.ir(inst.arg(2 + i)?) };
        let (u, s) = (Self::unsigned(rt), Self::signed(rt));
        let bin = |me: &mut Self, out: &mut Block, op, semantic| -> Result<Value, Error> {
            let (p, q) = (me.ir(inst.arg(2)?)?, me.ir(inst.arg(3)?)?);
            Ok(me.binary(out, op, p, q, semantic, rt))
        };
        let cmp = |me: &mut Self, out: &mut Block, c, sign: Option<bool>| -> Result<Value, Error> {
            let (p, q) = (me.ir(inst.arg(2)?)?, me.ir(inst.arg(3)?)?);
            let have = me.e.ty(p);
            let semantic = match sign {
                Some(true) => Self::signed(have),
                Some(false) => Self::unsigned(have),
                None => have,
            };
            Ok(me.compare(out, c, p, q, semantic))
        };
        let v = match inst.op {
            op::I_ADD | op::F_ADD => bin(self, out, BinOp::Add, rt)?,
            op::I_SUB | op::F_SUB => bin(self, out, BinOp::Sub, rt)?,
            op::I_MUL | op::F_MUL => bin(self, out, BinOp::Mul, rt)?,
            op::U_DIV => bin(self, out, BinOp::Div, u)?,
            op::S_DIV => bin(self, out, BinOp::Div, s)?,
            op::F_DIV => bin(self, out, BinOp::Div, rt)?,
            op::U_MOD => bin(self, out, BinOp::Rem, u)?,
            op::S_REM | op::F_REM => bin(self, out, BinOp::Rem, if inst.op == op::S_REM { s } else { rt })?,
            op::S_MOD | op::F_MOD => {
                // The remainder with the divisor's sign: the dividend-signed one, plus the divisor when the signs
                // differ and it isn't zero.
                let semantic = if inst.op == op::S_MOD { s } else { rt };
                let (p, q) = (x(self, 0)?, x(self, 1)?);
                let p = self.as_ty(out, p, semantic);
                let q = self.as_ty(out, q, semantic);
                let r = self.let_(out, semantic, Op::Binary(BinOp::Rem, p, q));
                let zero = self.zero_like(out, semantic);
                let bools = semantic.with_scalar(Scalar::Bool);
                let r_neg = self.let_(out, bools, Op::Compare(Cmp::Lt, r, zero));
                let q_neg = self.let_(out, bools, Op::Compare(Cmp::Lt, q, zero));
                let differ = self.let_(out, bools, Op::Compare(Cmp::Ne, r_neg, q_neg));
                let nonzero = self.let_(out, bools, Op::Compare(Cmp::Ne, r, zero));
                let fix = self.let_(out, bools, Op::Binary(BinOp::And, differ, nonzero));
                let fixed = self.let_(out, semantic, Op::Binary(BinOp::Add, r, q));
                let m = self.let_(out, semantic, Op::Select(fix, fixed, r));
                self.as_ty(out, m, rt)
            }
            op::S_NEGATE => {
                let v = x(self, 0)?;
                let v = self.as_ty(out, v, s);
                let r = self.let_(out, s, Op::Unary(UnOp::Neg, v));
                self.as_ty(out, r, rt)
            }
            op::F_NEGATE => {
                let v = x(self, 0)?;
                self.let_(out, rt, Op::Unary(UnOp::Neg, v))
            }
            op::NOT | op::LOGICAL_NOT => {
                let v = x(self, 0)?;
                let v = self.as_ty(out, v, rt);
                self.let_(out, rt, Op::Unary(UnOp::Not, v))
            }
            op::BIT_COUNT => {
                let v = x(self, 0)?;
                let have = self.e.ty(v);
                let r = self.let_(out, have, Op::Unary(UnOp::Popcount, v));
                self.as_ty(out, r, rt)
            }
            op::BITWISE_AND | op::LOGICAL_AND => bin(self, out, BinOp::And, rt)?,
            op::BITWISE_OR | op::LOGICAL_OR => bin(self, out, BinOp::Or, rt)?,
            op::BITWISE_XOR => bin(self, out, BinOp::Xor, rt)?,
            op::LOGICAL_EQUAL => cmp(self, out, Cmp::Eq, None)?,
            op::LOGICAL_NOT_EQUAL => cmp(self, out, Cmp::Ne, None)?,
            op::SHIFT_LEFT_LOGICAL | op::SHIFT_RIGHT_LOGICAL | op::SHIFT_RIGHT_ARITHMETIC => {
                let semantic = match inst.op {
                    op::SHIFT_RIGHT_ARITHMETIC => s,
                    op::SHIFT_RIGHT_LOGICAL => u,
                    _ => rt,
                };
                let op = if inst.op == op::SHIFT_LEFT_LOGICAL { BinOp::Shl } else { BinOp::Shr };
                bin(self, out, op, semantic)?
            }
            op::I_EQUAL => cmp(self, out, Cmp::Eq, None)?,
            op::I_NOT_EQUAL => cmp(self, out, Cmp::Ne, None)?,
            op::U_GREATER_THAN => cmp(self, out, Cmp::Gt, Some(false))?,
            op::S_GREATER_THAN => cmp(self, out, Cmp::Gt, Some(true))?,
            op::U_GREATER_THAN_EQUAL => cmp(self, out, Cmp::Ge, Some(false))?,
            op::S_GREATER_THAN_EQUAL => cmp(self, out, Cmp::Ge, Some(true))?,
            op::U_LESS_THAN => cmp(self, out, Cmp::Lt, Some(false))?,
            op::S_LESS_THAN => cmp(self, out, Cmp::Lt, Some(true))?,
            op::U_LESS_THAN_EQUAL => cmp(self, out, Cmp::Le, Some(false))?,
            op::S_LESS_THAN_EQUAL => cmp(self, out, Cmp::Le, Some(true))?,
            // Ordered and unordered differ only for NaN, which every backend compares as IEEE does (false, except
            // for not-equal); the unordered forms are taken as the ordered ones.
            op::F_ORD_EQUAL | op::F_UNORD_EQUAL => cmp(self, out, Cmp::Eq, None)?,
            op::F_ORD_NOT_EQUAL | op::F_UNORD_NOT_EQUAL => cmp(self, out, Cmp::Ne, None)?,
            op::F_ORD_LESS_THAN | op::F_UNORD_LESS_THAN => cmp(self, out, Cmp::Lt, None)?,
            op::F_ORD_GREATER_THAN | op::F_UNORD_GREATER_THAN => cmp(self, out, Cmp::Gt, None)?,
            op::F_ORD_LESS_THAN_EQUAL | op::F_UNORD_LESS_THAN_EQUAL => cmp(self, out, Cmp::Le, None)?,
            op::F_ORD_GREATER_THAN_EQUAL | op::F_UNORD_GREATER_THAN_EQUAL => cmp(self, out, Cmp::Ge, None)?,
            op::CONVERT_F_TO_U | op::CONVERT_F_TO_S => {
                let target = if inst.op == op::CONVERT_F_TO_U { u } else { s };
                let v = x(self, 0)?;
                let r = self.let_(out, target, Op::Convert(v, target.scalar().unwrap_or(Scalar::U32)));
                self.as_ty(out, r, rt)
            }
            op::CONVERT_S_TO_F | op::CONVERT_U_TO_F => {
                let v = x(self, 0)?;
                let have = self.e.ty(v);
                let read = if inst.op == op::CONVERT_S_TO_F { Self::signed(have) } else { Self::unsigned(have) };
                let v = self.as_ty(out, v, read);
                self.let_(out, rt, Op::Convert(v, rt.scalar().unwrap_or(Scalar::F32)))
            }
            op::U_CONVERT | op::S_CONVERT => {
                let v = x(self, 0)?;
                let have = self.e.ty(v);
                let (read, target) = if inst.op == op::S_CONVERT { (Self::signed(have), s) } else { (Self::unsigned(have), u) };
                let v = self.as_ty(out, v, read);
                let r = self.let_(out, target, Op::Convert(v, target.scalar().unwrap_or(Scalar::U32)));
                self.as_ty(out, r, rt)
            }
            op::F_CONVERT => {
                let v = x(self, 0)?;
                if self.e.ty(v) != rt {
                    return err("a float conversion between widths isn't translated yet".into());
                }
                v
            }
            op::BITCAST => {
                let v = x(self, 0)?;
                self.let_(out, rt, Op::Bitcast(v, rt))
            }
            op::VECTOR_TIMES_SCALAR => {
                let (v, k) = (x(self, 0)?, x(self, 1)?);
                let k = self.splat(out, k, rt);
                self.let_(out, rt, Op::Binary(BinOp::Mul, v, k))
            }
            op::DOT => {
                let (p, q) = (x(self, 0)?, x(self, 1)?);
                self.dot(out, p, q)
            }
            op::ANY | op::ALL => {
                let v = x(self, 0)?;
                let lanes = self.lanes_of(out, v);
                let op = if inst.op == op::ANY { BinOp::Or } else { BinOp::And };
                let mut acc = lanes[0];
                for l in &lanes[1..] {
                    acc = self.let_(out, Type::BOOL, Op::Binary(op, acc, *l));
                }
                acc
            }
            other => return err(format!("{} isn't translated yet", op::name(other))),
        };
        self.set(id, Val::Ir(v));
        Ok(())
    }
}

fn part(val: &Val, i: usize) -> Result<Val, Error> {
    match val {
        Val::Composite(parts) => parts.get(i).cloned().ok_or_else(|| Error { message: format!("no part {i}") }),
        other => err(format!("part {i} of {}", kind(other))),
    }
}

fn kind(val: &Val) -> &'static str {
    match val {
        Val::Ir(_) => "a number",
        Val::Composite(_) => "a struct, array or matrix",
        Val::Ptr(_) => "a pointer",
        Val::Image(_) => "an image",
        Val::Sampler(_) => "a sampler",
        Val::SampledImage { .. } => "a sampled image",
        Val::Unsupported(what) => what,
    }
}

fn successors(end: &End) -> Vec<u32> {
    match end {
        End::Goto(t) => vec![*t],
        End::Cond { t, f, .. } if t == f => vec![*t],
        End::Cond { t, f, .. } => vec![*t, *f],
        End::Switch { default, cases, .. } => {
            let mut all = vec![*default];
            for (_, t) in cases {
                if !all.contains(t) {
                    all.push(*t);
                }
            }
            all
        }
        End::Return | End::Kill => Vec::new(),
    }
}
