//! DXBC's instructions as IR.
//!
//! Registers are typeless four-component words, so each `r#` is a [`Var`] of `vec4<u32>`, and every instruction reads
//! the components it needs, bitcasts them to its own type, and writes the components of its destination's mask back.
//! The work is done one component at a time: what a destination's lane `c` gets is the instruction applied to each
//! source's lane chosen by its swizzle for `c`, so swizzles, masks and dot products need nothing special.
//!
//! Inputs are read once at the start, outputs kept in variables and written out where the program ends (each `ret`,
//! and the end). Control flow is structured already; a `switch` becomes a loop run once, its cases tests in order with
//! fall-through, and a `continue` inside it a flag the loop's end turns into the real one.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use gpu_ir::{
    AtomicOp, Axis, Barrier, BinOp, Binding, BindingKind, Block, Builtin, Cmp, Entry, Interpolation, Io, Lod, Op, Region,
    Scalar, Slot, Space, Stage, Stmt, Type, UnOp, Value, Var,
};

use super::container::{Component, Element};
use super::tokens::{Instruction, Operand, Program, op, ty};
use super::{Shader, fail};
use crate::Error;

const U4: Type = Type::Vector(Scalar::U32, 4);
const F4: Type = Type::Vector(Scalar::F32, 4);

/// Where a texture's, sampler's or buffer's binding is in each stage's tables (D3D11 has one per stage): a constant
/// buffer `b#` in set 0, a shader resource `t#` in set 1, a sampler `s#` in set 2, an unordered access view `u#` in
/// set 3, each at its slot.
pub const SET_CONSTANTS: u32 = 0;
pub const SET_RESOURCES: u32 = 1;
pub const SET_SAMPLERS: u32 = 2;
pub const SET_UAVS: u32 = 3;

/// An output register, kept in a variable until the program ends.
struct Out {
    var: Var,
    io: u32,
    /// `oDepth`: only its x, as an `f32`.
    depth: bool,
    ty: Type,
}

enum Frame {
    If { cond: Value, then: Option<Block> },
    Loop,
    Switch { selector: Value, groups: Vec<Group>, cont: Var, continued: bool },
}

#[derive(Default)]
struct Group {
    labels: Vec<u32>,
    default: bool,
    body: Block,
}

/// A buffer's base and its structure's stride (0 for raw buffers).
#[derive(Clone, Copy)]
enum Buffer {
    Binding(u32, u32),
    Workgroup(u32, u32),
}

struct T<'a> {
    e: Entry,
    blocks: Vec<Block>,
    frames: Vec<Frame>,
    inputs: &'a [Element],
    outputs_sig: &'a [Element],
    temps: Vec<Var>,
    indexable: BTreeMap<u32, u32>,
    icb: Option<u32>,
    regs_in: BTreeMap<u32, Value>,
    builtins: BTreeMap<u32, Value>,
    outs: BTreeMap<u32, Out>,
    depth: Option<Out>,
    constants: BTreeMap<u32, u32>,
    textures: BTreeMap<u32, (u32, u32)>,
    samplers: BTreeMap<u32, u32>,
    buffers: BTreeMap<(u32, u32), Buffer>,
}

fn scalar_of(c: Component) -> Scalar {
    match c {
        Component::Uint => Scalar::U32,
        Component::Sint => Scalar::I32,
        Component::Float => Scalar::F32,
    }
}

pub(super) fn translate(p: &Program, inputs: &[Element], outputs: &[Element]) -> Result<Shader, Error> {
    let stage = match p.kind {
        0 => Stage::Fragment,
        1 => Stage::Vertex,
        5 => Stage::Compute,
        2 => return fail("geometry shaders aren't read yet"),
        3 | 4 => return fail("tessellation shaders aren't read yet"),
        k => return fail(alloc::format!("a program of kind {k}")),
    };
    if p.major < 4 || p.major > 5 || (p.major == 5 && p.minor > 0) {
        return fail(alloc::format!("shader model {}.{} isn't read (4.0 to 5.0 are)", p.major, p.minor));
    }
    let mut e = Entry::shader("main", stage);
    if stage == Stage::Compute {
        e = Entry::new("main", [1, 1, 1]);
    }
    let mut t = T {
        e,
        blocks: vec![Vec::new()],
        frames: Vec::new(),
        inputs,
        outputs_sig: outputs,
        temps: Vec::new(),
        indexable: BTreeMap::new(),
        icb: None,
        regs_in: BTreeMap::new(),
        builtins: BTreeMap::new(),
        outs: BTreeMap::new(),
        depth: None,
        constants: BTreeMap::new(),
        textures: BTreeMap::new(),
        samplers: BTreeMap::new(),
        buffers: BTreeMap::new(),
    };
    for (n, i) in p.instructions.iter().enumerate() {
        t.instruction(i).map_err(|e| Error { message: alloc::format!("instruction {n} (opcode {}): {}", i.op, e.message) })?;
    }
    if !t.frames.is_empty() {
        return fail("the program ends inside a block");
    }
    if !matches!(t.blocks[0].last(), Some(Stmt::Return)) {
        t.write_outputs();
    }
    let body = t.blocks.pop().unwrap_or_default();
    t.e.body = body;
    gpu_ir::validate(&t.e)?;
    Ok(Shader { entry: t.e, inputs: inputs.to_vec(), outputs: outputs.to_vec() })
}

impl T<'_> {
    fn emit(&mut self, s: Stmt) {
        if let Some(b) = self.blocks.last_mut() {
            b.push(s);
        }
    }

    fn let_(&mut self, ty: Type, op: Op) -> Value {
        let v = self.e.value(ty);
        self.emit(Stmt::Let(v, op));
        v
    }

    fn u32c(&mut self, x: u32) -> Value {
        self.let_(Type::U32, Op::Const(Scalar::U32, x as u64))
    }

    fn f32c(&mut self, x: f32) -> Value {
        self.let_(Type::F32, Op::Const(Scalar::F32, x.to_bits() as u64))
    }

    fn cast(&mut self, v: Value, to: Scalar) -> Value {
        let from = self.e.ty(v);
        if from.scalar() == Some(to) {
            return v;
        }
        self.let_(from.with_scalar(to), Op::Bitcast(v, from.with_scalar(to)))
    }

    fn bin(&mut self, op: BinOp, a: Value, b: Value) -> Value {
        let ty = self.e.ty(a);
        self.let_(ty, Op::Binary(op, a, b))
    }

    fn un(&mut self, op: UnOp, a: Value) -> Value {
        let ty = self.e.ty(a);
        self.let_(ty, Op::Unary(op, a))
    }

    fn cmp(&mut self, c: Cmp, a: Value, b: Value) -> Value {
        self.let_(Type::BOOL, Op::Compare(c, a, b))
    }

    fn select(&mut self, cond: Value, a: Value, b: Value) -> Value {
        let ty = self.e.ty(a);
        self.let_(ty, Op::Select(cond, a, b))
    }

    /// D3D's boolean: all ones or zero.
    fn mask_of(&mut self, cond: Value) -> Value {
        let (ones, zero) = (self.u32c(u32::MAX), self.u32c(0));
        self.select(cond, ones, zero)
    }

    fn zero4(&mut self) -> Value {
        let z = self.u32c(0);
        self.let_(U4, Op::Construct(vec![z; 4]))
    }

    fn extract(&mut self, v: Value, lane: u8) -> Value {
        let ty = self.e.ty(v);
        if ty.lanes() == 1 {
            return v;
        }
        self.let_(Type::Scalar(ty.scalar().unwrap_or(Scalar::U32)), Op::Extract(v, lane))
    }

    fn element(sig: &[Element], register: u32) -> Option<&Element> {
        sig.iter().find(|el| el.register == register)
    }

    // Sources.

    /// An index of an operand: its number, plus a register's component if it's relative.
    fn index(&mut self, o: &Operand, d: usize) -> Result<Value, Error> {
        let Some(i) = o.index.get(d) else { return fail("an operand missing an index") };
        let base = self.u32c(i.imm);
        match &i.rel {
            None => Ok(base),
            Some(r) => {
                let r = r.as_ref().clone();
                let x = self.raw(&r, 0)?;
                Ok(self.bin(BinOp::Add, base, x))
            }
        }
    }

    /// A word at `elem * 16 + comp * 4` of memory.
    fn load_word(&mut self, base: Op, space: Space, elem: Value, comp: u8) -> Result<Value, Error> {
        let ptr = self.let_(Type::Ptr(space), base);
        let sixteen = self.u32c(16);
        let at = self.bin(BinOp::Mul, elem, sixteen);
        let c = self.u32c(comp as u32 * 4);
        let at = self.bin(BinOp::Add, at, c);
        let p = self.let_(Type::Ptr(space), Op::Offset(ptr, at));
        Ok(self.let_(Type::U32, Op::Load(p, Type::U32)))
    }

    /// A source's component `c` as its bits, before modifiers.
    fn raw(&mut self, o: &Operand, c: usize) -> Result<Value, Error> {
        let comp = o.component(c);
        match o.ty {
            ty::TEMP => {
                let Some(&var) = self.temps.get(o.reg() as usize) else { return fail(alloc::format!("r{} isn't declared", o.reg())) };
                let v = self.let_(U4, Op::Read(var));
                Ok(self.extract(v, comp))
            }
            ty::INPUT => {
                if o.index.first().is_some_and(|i| i.rel.is_some()) {
                    return fail("inputs indexed by a register aren't read yet");
                }
                let Some(&v) = self.regs_in.get(&o.reg()) else { return fail(alloc::format!("v{} isn't declared", o.reg())) };
                Ok(self.extract(v, comp))
            }
            ty::IMMEDIATE32 => Ok(self.u32c(o.imm[comp as usize])),
            ty::CONSTANT_BUFFER => {
                let slot = o.reg();
                let Some(&b) = self.constants.get(&slot) else { return fail(alloc::format!("cb{slot} isn't declared")) };
                let elem = self.index(o, 1)?;
                self.load_word(Op::Binding(b), Space::Device, elem, comp)
            }
            ty::IMMEDIATE_CONSTANT_BUFFER => {
                let Some(r) = self.icb else { return fail("icb isn't declared") };
                let elem = self.index(o, 0)?;
                self.load_word(Op::Private(r), Space::Private, elem, comp)
            }
            ty::INDEXABLE_TEMP => {
                let Some(&r) = self.indexable.get(&o.reg()) else { return fail(alloc::format!("x{} isn't declared", o.reg())) };
                let elem = self.index(o, 1)?;
                self.load_word(Op::Private(r), Space::Private, elem, comp)
            }
            ty::THREAD_ID | ty::THREAD_GROUP_ID | ty::THREAD_ID_IN_GROUP | ty::THREAD_ID_IN_GROUP_FLATTENED => {
                let Some(&v) = self.builtins.get(&o.ty) else { return fail(alloc::format!("input {} isn't declared", o.ty)) };
                Ok(self.extract(v, comp))
            }
            ty::OUTPUT => {
                let Some(out) = self.outs.get(&o.reg()) else { return fail(alloc::format!("o{} isn't declared", o.reg())) };
                let var = out.var;
                let v = self.let_(U4, Op::Read(var));
                Ok(self.extract(v, comp))
            }
            t => fail(alloc::format!("a source of operand type {t}")),
        }
    }

    /// A float source: its bits as an `f32`, with its modifier.
    fn src_f(&mut self, o: &Operand, c: usize) -> Result<Value, Error> {
        let v = self.raw(o, c)?;
        let mut v = self.cast(v, Scalar::F32);
        if o.modifier & 2 != 0 {
            v = self.un(UnOp::Abs, v);
        }
        if o.modifier & 1 != 0 {
            v = self.un(UnOp::Neg, v);
        }
        Ok(v)
    }

    /// An integer source, as the scalar given, its modifier as integer negation.
    fn src_int(&mut self, o: &Operand, c: usize, s: Scalar) -> Result<Value, Error> {
        let v = self.raw(o, c)?;
        let mut v = self.cast(v, s);
        if o.modifier & 2 != 0 {
            let i = self.cast(v, Scalar::I32);
            let a = self.un(UnOp::Abs, i);
            v = self.cast(a, s);
        }
        if o.modifier & 1 != 0 {
            v = self.un(UnOp::Neg, v);
        }
        Ok(v)
    }

    /// A source's bits; through the float modifiers if it has any.
    fn src_bits(&mut self, o: &Operand, c: usize) -> Result<Value, Error> {
        if o.modifier == 0 {
            return self.raw(o, c);
        }
        let f = self.src_f(o, c)?;
        Ok(self.cast(f, Scalar::U32))
    }

    // Destinations.

    fn write(&mut self, d: &Operand, lanes: [Option<Value>; 4]) -> Result<(), Error> {
        let var = match d.ty {
            ty::NULL => return Ok(()),
            ty::TEMP => *self.temps.get(d.reg() as usize).ok_or_else(|| Error { message: alloc::format!("r{} isn't declared", d.reg()) })?,
            ty::OUTPUT => self.outs.get(&d.reg()).map(|o| o.var).ok_or_else(|| Error { message: alloc::format!("o{} isn't declared", d.reg()) })?,
            ty::OUTPUT_DEPTH => self.depth.as_ref().map(|o| o.var).ok_or_else(|| Error { message: "oDepth isn't declared".into() })?,
            ty::INDEXABLE_TEMP => {
                let Some(&r) = self.indexable.get(&d.reg()) else { return fail(alloc::format!("x{} isn't declared", d.reg())) };
                let elem = self.index(d, 1)?;
                for (c, v) in lanes.iter().enumerate() {
                    let Some(v) = *v else { continue };
                    let ptr = self.let_(Type::Ptr(Space::Private), Op::Private(r));
                    let sixteen = self.u32c(16);
                    let at = self.bin(BinOp::Mul, elem, sixteen);
                    let k = self.u32c(c as u32 * 4);
                    let at = self.bin(BinOp::Add, at, k);
                    let p = self.let_(Type::Ptr(Space::Private), Op::Offset(ptr, at));
                    self.emit(Stmt::Store(p, v));
                }
                return Ok(());
            }
            t => return fail(alloc::format!("a destination of operand type {t}")),
        };
        let mut value = self.let_(U4, Op::Read(var));
        for (c, v) in lanes.iter().enumerate() {
            if let Some(v) = *v {
                value = self.let_(U4, Op::Insert(value, c as u8, v));
            }
        }
        self.emit(Stmt::Write(var, value));
        Ok(())
    }

    /// Each of the destination's components from a function of the component, giving bits.
    fn each(&mut self, d: &Operand, mut f: impl FnMut(&mut Self, usize) -> Result<Value, Error>) -> Result<(), Error> {
        let mut lanes = [None; 4];
        let mask = if d.components == 1 { 1 } else { d.mask() };
        for (c, lane) in lanes.iter_mut().enumerate() {
            if mask & (1 << c) != 0 {
                *lane = Some(f(self, c)?);
            }
        }
        self.write(d, lanes)
    }

    fn saturate(&mut self, v: Value) -> Value {
        let (zero, one) = (self.f32c(0.0), self.f32c(1.0));
        let v = self.bin(BinOp::Max, v, zero);
        self.bin(BinOp::Min, v, one)
    }

    /// A float instruction's components: the function's `f32`, saturated if the instruction says so.
    fn each_f(&mut self, i: &Instruction, mut f: impl FnMut(&mut Self, usize) -> Result<Value, Error>) -> Result<(), Error> {
        let sat = i.saturate;
        self.each(&i.operands[0], |t, c| {
            let mut v = f(t, c)?;
            if sat {
                v = t.saturate(v);
            }
            Ok(t.cast(v, Scalar::U32))
        })
    }

    fn operand(i: &Instruction, n: usize) -> Result<&Operand, Error> {
        i.operands.get(n).ok_or_else(|| Error { message: alloc::format!("operand {n} is missing") })
    }

    /// The test of `if`, `breakc` and the like: the source's first selected component, nonzero or zero.
    fn test(&mut self, i: &Instruction) -> Result<Value, Error> {
        let o = Self::operand(i, 0)?.clone();
        let x = self.raw(&o, 0)?;
        let zero = self.u32c(0);
        Ok(self.cmp(if i.nonzero { Cmp::Ne } else { Cmp::Eq }, x, zero))
    }

    fn write_outputs(&mut self) {
        let outs: Vec<(Var, u32, bool, Type)> = self.outs.values().chain(self.depth.as_ref()).map(|o| (o.var, o.io, o.depth, o.ty)).collect();
        for (var, io, depth, ty) in outs {
            let v = self.let_(U4, Op::Read(var));
            let typed = if depth {
                let x = self.extract(v, 0);
                self.cast(x, Scalar::F32)
            } else {
                self.let_(ty, Op::Bitcast(v, ty))
            };
            self.emit(Stmt::Output(io, typed));
        }
    }

    // Declarations.

    fn binding(&mut self, set: u32, slot: u32, kind: BindingKind) -> u32 {
        self.e.bindings.push(Binding { set, binding: slot, kind });
        self.e.bindings.len() as u32 - 1
    }

    fn io_input(&mut self, register: u32, interpolation: Interpolation) -> Result<(), Error> {
        if self.regs_in.contains_key(&register) {
            return Ok(());
        }
        let s = Self::element(self.inputs, register).map_or(Scalar::F32, |el| scalar_of(el.component));
        let interpolation = if s == Scalar::F32 { interpolation } else { Interpolation::Flat };
        let ty = Type::Vector(s, 4);
        self.e.inputs.push(Io { slot: Slot::Location(register), ty, interpolation });
        let n = self.e.inputs.len() as u32 - 1;
        let v = self.let_(ty, Op::Input(n));
        let bits = self.cast(v, Scalar::U32);
        self.regs_in.insert(register, bits);
        Ok(())
    }

    /// A builtin value into a register's lanes: in the lanes of the mask, from its own lanes in order.
    fn builtin_input(&mut self, register: u32, mask: u8, value: Value) {
        let lanes = self.e.ty(value).lanes();
        let mut parts = Vec::new();
        let mut next = 0u8;
        let zero = self.u32c(0);
        let value = self.cast(value, Scalar::U32);
        for c in 0..4 {
            if mask & (1 << c) != 0 && next < lanes {
                parts.push(self.extract(value, next));
                next += 1;
            } else {
                parts.push(zero);
            }
        }
        let existing = self.regs_in.get(&register).copied();
        let v = match existing {
            None => self.let_(U4, Op::Construct(parts)),
            Some(old) => {
                let mut v = old;
                for c in 0..4u8 {
                    if mask & (1 << c) != 0 {
                        v = self.let_(U4, Op::Insert(v, c, parts[c as usize]));
                    }
                }
                v
            }
        };
        self.regs_in.insert(register, v);
    }

    fn system_input(&mut self, o: &Operand, name: u32) -> Result<(), Error> {
        let mask = o.mask();
        let stage = self.e.stage;
        let value = match (stage, name) {
            (Stage::Vertex, 6) => self.let_(Type::U32, Op::Builtin(Builtin::VertexIndex)),
            (Stage::Vertex, 8) => self.let_(Type::U32, Op::Builtin(Builtin::InstanceIndex)),
            (Stage::Fragment, 1) => {
                // D3D's w is 1/w where Vulkan's (and the IR's) is w's reciprocal already: d3d.w = 1 / ir.w.
                let p = self.let_(F4, Op::Builtin(Builtin::FragCoord));
                let w = self.extract(p, 3);
                let one = self.f32c(1.0);
                let rw = self.bin(BinOp::Div, one, w);
                self.let_(F4, Op::Insert(p, 3, rw))
            }
            (Stage::Fragment, 9) => {
                let f = self.let_(Type::BOOL, Op::Builtin(Builtin::FrontFacing));
                self.mask_of(f)
            }
            (Stage::Vertex, 0) | (Stage::Fragment, 0) => return self.io_input(o.reg(), Interpolation::Smooth),
            (_, n) => return fail(alloc::format!("the {stage:?} input system value {n} isn't read yet")),
        };
        self.builtin_input(o.reg(), mask, value);
        Ok(())
    }

    fn output(&mut self, o: &Operand, name: u32) -> Result<(), Error> {
        if o.ty == ty::OUTPUT_DEPTH {
            let var = self.e.var(U4);
            let z = self.zero4();
            self.emit(Stmt::Write(var, z));
            self.e.outputs.push(Io { slot: Slot::FragDepth, ty: Type::F32, interpolation: Interpolation::Smooth });
            self.depth = Some(Out { var, io: self.e.outputs.len() as u32 - 1, depth: true, ty: Type::F32 });
            return Ok(());
        }
        if o.ty != ty::OUTPUT {
            return fail(alloc::format!("an output of operand type {}", o.ty));
        }
        let register = o.reg();
        if self.outs.contains_key(&register) {
            return Ok(());
        }
        let slot = match (self.e.stage, name) {
            (Stage::Vertex, 1) => Slot::Position,
            (_, 0) => Slot::Location(register),
            (stage, n) => return fail(alloc::format!("the {stage:?} output system value {n} isn't written yet")),
        };
        let s = if slot == Slot::Position { Scalar::F32 } else { Self::element(self.outputs_sig, register).map_or(Scalar::F32, |el| scalar_of(el.component)) };
        let ty = Type::Vector(s, 4);
        self.e.outputs.push(Io { slot, ty, interpolation: Interpolation::Smooth });
        let var = self.e.var(U4);
        let z = self.zero4();
        self.emit(Stmt::Write(var, z));
        self.outs.insert(register, Out { var, io: self.e.outputs.len() as u32 - 1, depth: false, ty });
        Ok(())
    }

    fn compute_input(&mut self, o: &Operand) -> Result<(), Error> {
        let builtin = match o.ty {
            ty::THREAD_ID => Builtin::GlobalId,
            ty::THREAD_GROUP_ID => Builtin::GroupId,
            ty::THREAD_ID_IN_GROUP => Builtin::LocalId,
            ty::THREAD_ID_IN_GROUP_FLATTENED => Builtin::LocalIndex,
            t => return fail(alloc::format!("an input of operand type {t}")),
        };
        let v = self.let_(builtin.ty(), Op::Builtin(builtin));
        let zero = self.u32c(0);
        let mut parts = Vec::new();
        for c in 0..4 {
            parts.push(if c < builtin.ty().lanes() { self.extract(v, c) } else { zero });
        }
        let v = self.let_(U4, Op::Construct(parts));
        self.builtins.insert(o.ty, v);
        Ok(())
    }

    fn declaration(&mut self, i: &Instruction) -> Result<bool, Error> {
        let word = |n: usize| i.words.get(n).copied().ok_or_else(|| Error { message: alloc::format!("a declaration missing word {n}") });
        match i.op {
            op::DCL_GLOBAL_FLAGS | op::DCL_INDEX_RANGE => {}
            op::DCL_TEMPS => {
                for _ in 0..word(0)? {
                    let var = self.e.var(U4);
                    let z = self.zero4();
                    self.emit(Stmt::Write(var, z));
                    self.temps.push(var);
                }
            }
            op::DCL_INDEXABLE_TEMP => {
                let (reg, size) = (word(0)?, word(1)?);
                self.e.private.push(Region { size: size * 16, align: 16 });
                self.indexable.insert(reg, self.e.private.len() as u32 - 1);
            }
            op::CUSTOMDATA if i.token >> 11 == 3 => {
                self.e.private.push(Region { size: i.words.len() as u32 * 4, align: 16 });
                let r = self.e.private.len() as u32 - 1;
                self.icb = Some(r);
                for (n, w) in i.words.iter().enumerate() {
                    let ptr = self.let_(Type::Ptr(Space::Private), Op::Private(r));
                    let at = self.u32c(n as u32 * 4);
                    let p = self.let_(Type::Ptr(Space::Private), Op::Offset(ptr, at));
                    let v = self.u32c(*w);
                    self.emit(Stmt::Store(p, v));
                }
            }
            op::CUSTOMDATA => {}
            op::DCL_CONSTANT_BUFFER => {
                let slot = Self::operand(i, 0)?.reg();
                let b = self.binding(SET_CONSTANTS, slot, BindingKind::Uniform);
                self.constants.insert(slot, b);
            }
            op::DCL_SAMPLER => {
                let slot = Self::operand(i, 0)?.reg();
                let b = self.binding(SET_SAMPLERS, slot, BindingKind::Sampler);
                self.samplers.insert(slot, b);
            }
            op::DCL_RESOURCE => {
                let slot = Self::operand(i, 0)?.reg();
                let dim = (i.token >> 11) & 0x1f;
                let b = if dim == 3 { self.binding(SET_RESOURCES, slot, BindingKind::Image) } else { u32::MAX };
                self.textures.insert(slot, (b, dim));
            }
            op::DCL_RESOURCE_RAW | op::DCL_RESOURCE_STRUCTURED => {
                let slot = Self::operand(i, 0)?.reg();
                let stride = if i.op == op::DCL_RESOURCE_RAW { 0 } else { word(0)? };
                let b = self.binding(SET_RESOURCES, slot, BindingKind::Storage { writable: false });
                self.buffers.insert((ty::RESOURCE, slot), Buffer::Binding(b, stride));
            }
            op::DCL_UAV_RAW | op::DCL_UAV_STRUCTURED => {
                let slot = Self::operand(i, 0)?.reg();
                let stride = if i.op == op::DCL_UAV_RAW { 0 } else { word(0)? };
                let b = self.binding(SET_UAVS, slot, BindingKind::Storage { writable: true });
                self.buffers.insert((ty::UAV, slot), Buffer::Binding(b, stride));
            }
            op::DCL_UAV_TYPED => return fail("typed unordered access views aren't read yet"),
            op::DCL_TGSM_RAW | op::DCL_TGSM_STRUCTURED => {
                let reg = Self::operand(i, 0)?.reg();
                let (stride, size) = if i.op == op::DCL_TGSM_RAW { (0, word(0)?) } else { (word(0)?, word(0)? * word(1)?) };
                self.e.workgroup.push(Region { size, align: 4 });
                self.buffers.insert((ty::TGSM, reg), Buffer::Workgroup(self.e.workgroup.len() as u32 - 1, stride));
            }
            op::DCL_THREAD_GROUP => self.e.workgroup_size = [word(0)?, word(1)?, word(2)?],
            op::DCL_INPUT | op::DCL_INPUT_SIV => {
                let o = Self::operand(i, 0)?.clone();
                if o.ty == ty::INPUT {
                    let interpolation = if self.e.stage == Stage::Fragment { Interpolation::Flat } else { Interpolation::Smooth };
                    if i.op == op::DCL_INPUT_SIV && word(0)? != 0 {
                        return self.system_input(&o, word(0)?).map(|_| true);
                    }
                    self.io_input(o.reg(), interpolation)?;
                } else {
                    self.compute_input(&o)?;
                }
            }
            op::DCL_INPUT_SGV | op::DCL_INPUT_PS_SGV | op::DCL_INPUT_PS_SIV => {
                let o = Self::operand(i, 0)?.clone();
                self.system_input(&o, word(0)?)?;
            }
            op::DCL_INPUT_PS => {
                let o = Self::operand(i, 0)?.clone();
                let interpolation = match (i.token >> 11) & 0xf {
                    1 => Interpolation::Flat,
                    4 | 5 | 7 => Interpolation::NoPerspective,
                    _ => Interpolation::Smooth,
                };
                self.io_input(o.reg(), interpolation)?;
            }
            op::DCL_OUTPUT => {
                let o = Self::operand(i, 0)?.clone();
                self.output(&o, 0)?;
            }
            op::DCL_OUTPUT_SIV | op::DCL_OUTPUT_SGV => {
                let o = Self::operand(i, 0)?.clone();
                self.output(&o, word(0)?)?;
            }
            92..=94 | 143..=154 | 206 => return fail(alloc::format!("declaration {} is a geometry or tessellation stage's", i.op)),
            _ => return Ok(false),
        }
        Ok(true)
    }

    // Control flow.

    fn breakable(&self) -> Option<usize> {
        self.frames.iter().rposition(|f| !matches!(f, Frame::If { .. }))
    }

    fn brk(&mut self) {
        self.emit(Stmt::Break);
    }

    fn cont(&mut self) -> Result<(), Error> {
        match self.breakable().map(|n| &mut self.frames[n]) {
            Some(Frame::Loop) => self.emit(Stmt::Continue),
            Some(Frame::Switch { cont, continued, .. }) => {
                *continued = true;
                let var = *cont;
                let t = self.let_(Type::BOOL, Op::Const(Scalar::Bool, 1));
                self.emit(Stmt::Write(var, t));
                self.emit(Stmt::Break);
            }
            _ => return fail("a continue outside a loop"),
        }
        Ok(())
    }

    /// Statements in a block of their own, as one statement in the current one.
    fn conditional(&mut self, cond: Value, then: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        self.blocks.push(Vec::new());
        then(self)?;
        let then = self.blocks.pop().unwrap_or_default();
        self.emit(Stmt::If { cond, then, otherwise: Vec::new() });
        Ok(())
    }

    fn end_switch(&mut self) -> Result<(), Error> {
        let body = self.blocks.pop().unwrap_or_default();
        let Some(Frame::Switch { selector, mut groups, cont, continued }) = self.frames.pop() else { return fail("endswitch outside a switch") };
        if let Some(g) = groups.last_mut() {
            g.body.extend(body);
        }
        let matched = self.e.var(Type::BOOL);
        let no = self.let_(Type::BOOL, Op::Const(Scalar::Bool, 0));
        self.emit(Stmt::Write(matched, no));
        // The default runs when no case's label is the selector.
        let mut any = no;
        for label in groups.iter().flat_map(|g| g.labels.iter().copied()).collect::<Vec<_>>() {
            let l = self.u32c(label);
            let eq = self.cmp(Cmp::Eq, selector, l);
            any = self.let_(Type::BOOL, Op::Binary(BinOp::Or, any, eq));
        }
        let none = self.let_(Type::BOOL, Op::Unary(UnOp::Not, any));
        self.blocks.push(Vec::new());
        for g in groups {
            let mut cond = self.let_(Type::BOOL, Op::Read(matched));
            for l in g.labels {
                let l = self.u32c(l);
                let eq = self.cmp(Cmp::Eq, selector, l);
                cond = self.let_(Type::BOOL, Op::Binary(BinOp::Or, cond, eq));
            }
            if g.default {
                cond = self.let_(Type::BOOL, Op::Binary(BinOp::Or, cond, none));
            }
            let yes = self.e.value(Type::BOOL);
            let mut then = vec![Stmt::Let(yes, Op::Const(Scalar::Bool, 1)), Stmt::Write(matched, yes)];
            then.extend(g.body);
            self.emit(Stmt::If { cond, then, otherwise: Vec::new() });
        }
        self.emit(Stmt::Break);
        let body = self.blocks.pop().unwrap_or_default();
        self.emit(Stmt::Loop { body, continuing: Vec::new(), break_if: None });
        if continued {
            let c = self.let_(Type::BOOL, Op::Read(cont));
            self.conditional(c, |t| t.cont())?;
        }
        Ok(())
    }

    fn flow(&mut self, i: &Instruction) -> Result<bool, Error> {
        match i.op {
            op::IF => {
                let cond = self.test(i)?;
                self.frames.push(Frame::If { cond, then: None });
                self.blocks.push(Vec::new());
            }
            op::ELSE => {
                let b = self.blocks.pop().unwrap_or_default();
                match self.frames.last_mut() {
                    Some(Frame::If { then, .. }) if then.is_none() => *then = Some(b),
                    _ => return fail("else outside an if"),
                }
                self.blocks.push(Vec::new());
            }
            op::ENDIF => {
                let b = self.blocks.pop().unwrap_or_default();
                let Some(Frame::If { cond, then }) = self.frames.pop() else { return fail("endif outside an if") };
                let (then, otherwise) = match then {
                    Some(t) => (t, b),
                    None => (b, Vec::new()),
                };
                self.emit(Stmt::If { cond, then, otherwise });
            }
            op::LOOP => {
                self.frames.push(Frame::Loop);
                self.blocks.push(Vec::new());
            }
            op::ENDLOOP => {
                let body = self.blocks.pop().unwrap_or_default();
                if !matches!(self.frames.pop(), Some(Frame::Loop)) {
                    return fail("endloop outside a loop");
                }
                self.emit(Stmt::Loop { body, continuing: Vec::new(), break_if: None });
            }
            op::BREAK => {
                if self.breakable().is_none() {
                    return fail("a break outside a loop or switch");
                }
                self.brk();
            }
            op::BREAKC => {
                if self.breakable().is_none() {
                    return fail("a break outside a loop or switch");
                }
                let cond = self.test(i)?;
                self.conditional(cond, |t| {
                    t.brk();
                    Ok(())
                })?;
            }
            op::CONTINUE => self.cont()?,
            op::CONTINUEC => {
                let cond = self.test(i)?;
                self.conditional(cond, |t| t.cont())?;
            }
            op::SWITCH => {
                let o = Self::operand(i, 0)?.clone();
                let selector = self.raw(&o, 0)?;
                let cont = self.e.var(Type::BOOL);
                let no = self.let_(Type::BOOL, Op::Const(Scalar::Bool, 0));
                self.emit(Stmt::Write(cont, no));
                self.frames.push(Frame::Switch { selector, groups: Vec::new(), cont, continued: false });
                self.blocks.push(Vec::new());
            }
            op::CASE | op::DEFAULT => {
                let label = if i.op == op::CASE { Some(Self::operand(i, 0)?.imm[0]) } else { None };
                let b = self.blocks.pop().unwrap_or_default();
                let Some(Frame::Switch { groups, .. }) = self.frames.last_mut() else { return fail("a case outside a switch") };
                if let Some(g) = groups.last_mut() {
                    g.body.extend(b);
                }
                if groups.last().is_none_or(|g| !g.body.is_empty()) {
                    groups.push(Group::default());
                }
                if let Some(g) = groups.last_mut() {
                    match label {
                        Some(l) => g.labels.push(l),
                        None => g.default = true,
                    }
                }
                self.blocks.push(Vec::new());
            }
            op::ENDSWITCH => self.end_switch()?,
            op::RET => {
                self.write_outputs();
                self.emit(Stmt::Return);
            }
            op::RETC => {
                let cond = self.test(i)?;
                self.conditional(cond, |t| {
                    t.write_outputs();
                    t.emit(Stmt::Return);
                    Ok(())
                })?;
            }
            op::DISCARD => {
                if self.e.stage != Stage::Fragment {
                    return fail("discard outside a pixel shader");
                }
                let cond = self.test(i)?;
                self.conditional(cond, |t| {
                    t.emit(Stmt::Discard);
                    Ok(())
                })?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    // Resources.

    fn buffer(&mut self, o: &Operand) -> Result<(Value, u32), Error> {
        let Some(&b) = self.buffers.get(&(o.ty, o.reg())) else { return fail(alloc::format!("buffer {} of type {} isn't declared", o.reg(), o.ty)) };
        Ok(match b {
            Buffer::Binding(n, stride) => (self.let_(Type::Ptr(Space::Device), Op::Binding(n)), stride),
            Buffer::Workgroup(n, stride) => (self.let_(Type::Ptr(Space::Workgroup), Op::Workgroup(n)), stride),
        })
    }

    fn at(&mut self, base: Value, byte: Value, plus: u32) -> Value {
        let space = match self.e.ty(base) {
            Type::Ptr(s) => s,
            _ => Space::Device,
        };
        let k = self.u32c(plus);
        let off = self.bin(BinOp::Add, byte, k);
        self.let_(Type::Ptr(space), Op::Offset(base, off))
    }

    /// A structured buffer's byte: `index * stride + offset`.
    fn structured(&mut self, index: Value, stride: u32, offset: Value) -> Value {
        let s = self.u32c(stride);
        let m = self.bin(BinOp::Mul, index, s);
        self.bin(BinOp::Add, m, offset)
    }

    fn sample(&mut self, i: &Instruction) -> Result<(), Error> {
        if i.offsets != [0; 3] {
            return fail("samples with texel offsets aren't read yet");
        }
        let (d, coord, t, s) = (Self::operand(i, 0)?.clone(), Self::operand(i, 1)?.clone(), Self::operand(i, 2)?.clone(), Self::operand(i, 3)?.clone());
        let Some(&(image, dim)) = self.textures.get(&t.reg()) else { return fail(alloc::format!("t{} isn't declared", t.reg())) };
        if dim != 3 {
            return fail(alloc::format!("sampling a texture of dimension {dim} (only 2D textures are read yet)"));
        }
        let Some(&sampler) = self.samplers.get(&s.reg()) else { return fail(alloc::format!("s{} isn't declared", s.reg())) };
        let lod = match i.op {
            op::SAMPLE if self.e.stage == Stage::Fragment => Lod::Implicit,
            op::SAMPLE => return fail("an implicit level of detail outside a pixel shader"),
            _ => {
                let x = Self::operand(i, 4)?.clone();
                let v = self.src_f(&x, 0)?;
                if i.op == op::SAMPLE_L { Lod::Level(v) } else { Lod::Bias(v) }
            }
        };
        let (u, v) = (self.src_f(&coord, 0)?, self.src_f(&coord, 1)?);
        let uv = self.let_(Type::Vector(Scalar::F32, 2), Op::Construct(vec![u, v]));
        let r = self.let_(F4, Op::Sample { image, sampler: Some(sampler), coord: uv, lod });
        let sat = i.saturate;
        self.each(&d, |me, c| {
            let mut x = me.extract(r, t.component(c));
            if sat {
                x = me.saturate(x);
            }
            Ok(me.cast(x, Scalar::U32))
        })
    }

    fn memory(&mut self, i: &Instruction) -> Result<(), Error> {
        let o = |n| Self::operand(i, n).cloned();
        match i.op {
            op::LD_RAW | op::LD_STRUCTURED => {
                let d = o(0)?;
                let res = o(if i.op == op::LD_RAW { 2 } else { 3 })?;
                let (base, stride) = self.buffer(&res)?;
                let byte = if i.op == op::LD_RAW {
                    self.raw(&o(1)?, 0)?
                } else {
                    let (index, offset) = (self.raw(&o(1)?, 0)?, self.raw(&o(2)?, 0)?);
                    self.structured(index, stride, offset)
                };
                self.each(&d, |me, c| {
                    let p = me.at(base, byte, res.component(c) as u32 * 4);
                    Ok(me.let_(Type::U32, Op::Load(p, Type::U32)))
                })
            }
            op::STORE_RAW | op::STORE_STRUCTURED => {
                let d = o(0)?;
                let (base, stride) = self.buffer(&d)?;
                let (byte, value) = if i.op == op::STORE_RAW {
                    (self.raw(&o(1)?, 0)?, o(2)?)
                } else {
                    let (index, offset) = (self.raw(&o(1)?, 0)?, self.raw(&o(2)?, 0)?);
                    (self.structured(index, stride, offset), o(3)?)
                };
                let mut values = Vec::new();
                for c in 0..4 {
                    if d.mask() & (1 << c) != 0 {
                        values.push((c, self.raw(&value, c)?));
                    }
                }
                for (c, v) in values {
                    let p = self.at(base, byte, c as u32 * 4);
                    self.emit(Stmt::Store(p, v));
                }
                Ok(())
            }
            _ => fail("not a memory instruction"),
        }
    }

    fn atomic(&mut self, i: &Instruction) -> Result<(), Error> {
        let imm = i.op >= op::IMM_ATOMIC_IADD;
        let first = if imm { 1 } else { 0 };
        let res = Self::operand(i, first)?.clone();
        let addr = Self::operand(i, first + 1)?.clone();
        let (base, stride) = self.buffer(&res)?;
        let byte = if stride == 0 {
            self.raw(&addr, 0)?
        } else {
            let (index, offset) = (self.raw(&addr, 0)?, self.raw(&addr, 1)?);
            self.structured(index, stride, offset)
        };
        let ptr = self.at(base, byte, 0);
        let (aop, s) = match i.op {
            op::ATOMIC_IADD | op::IMM_ATOMIC_IADD => (Some(AtomicOp::Add), Scalar::U32),
            op::ATOMIC_AND | op::IMM_ATOMIC_AND => (Some(AtomicOp::And), Scalar::U32),
            op::ATOMIC_OR | op::IMM_ATOMIC_OR => (Some(AtomicOp::Or), Scalar::U32),
            op::ATOMIC_XOR | op::IMM_ATOMIC_XOR => (Some(AtomicOp::Xor), Scalar::U32),
            op::ATOMIC_IMAX | op::IMM_ATOMIC_IMAX => (Some(AtomicOp::Max), Scalar::I32),
            op::ATOMIC_IMIN | op::IMM_ATOMIC_IMIN => (Some(AtomicOp::Min), Scalar::I32),
            op::ATOMIC_UMAX | op::IMM_ATOMIC_UMAX => (Some(AtomicOp::Max), Scalar::U32),
            op::ATOMIC_UMIN | op::IMM_ATOMIC_UMIN => (Some(AtomicOp::Min), Scalar::U32),
            op::IMM_ATOMIC_EXCH => (Some(AtomicOp::Exchange), Scalar::U32),
            op::ATOMIC_CMP_STORE | op::IMM_ATOMIC_CMP_EXCH => (None, Scalar::U32),
            n => return fail(alloc::format!("atomic opcode {n} isn't read yet")),
        };
        let old = match aop {
            Some(aop) => {
                let v = self.raw(&Self::operand(i, first + 2)?.clone(), 0)?;
                let v = self.cast(v, s);
                self.let_(Type::Scalar(s), Op::Atomic(aop, ptr, v, s))
            }
            None => {
                let cmp = self.raw(&Self::operand(i, first + 2)?.clone(), 0)?;
                let v = self.raw(&Self::operand(i, first + 3)?.clone(), 0)?;
                self.let_(Type::U32, Op::CompareExchange(ptr, cmp, v, Scalar::U32))
            }
        };
        if imm {
            let d = Self::operand(i, 0)?.clone();
            let bits = self.cast(old, Scalar::U32);
            self.each(&d, |_, _| Ok(bits))?;
        }
        Ok(())
    }

    // Everything.

    fn instruction(&mut self, i: &Instruction) -> Result<(), Error> {
        if self.declaration(i)? || self.flow(i)? {
            return Ok(());
        }
        let o = |n| Self::operand(i, n).cloned();
        match i.op {
            op::NOP => Ok(()),
            op::MOV if !i.saturate => {
                let s = o(1)?;
                self.each(&o(0)?, |t, c| t.src_bits(&s, c))
            }
            op::MOV => {
                let s = o(1)?;
                self.each_f(i, |t, c| t.src_f(&s, c))
            }
            op::ADD | op::MUL | op::DIV | op::MIN | op::MAX => {
                let bop = match i.op {
                    op::ADD => BinOp::Add,
                    op::MUL => BinOp::Mul,
                    op::DIV => BinOp::Div,
                    op::MIN => BinOp::Min,
                    _ => BinOp::Max,
                };
                let (a, b) = (o(1)?, o(2)?);
                self.each_f(i, |t, c| {
                    let (x, y) = (t.src_f(&a, c)?, t.src_f(&b, c)?);
                    Ok(t.bin(bop, x, y))
                })
            }
            op::MAD => {
                let (a, b, s) = (o(1)?, o(2)?, o(3)?);
                self.each_f(i, |t, c| {
                    let (x, y, z) = (t.src_f(&a, c)?, t.src_f(&b, c)?, t.src_f(&s, c)?);
                    let m = t.bin(BinOp::Mul, x, y);
                    Ok(t.bin(BinOp::Add, m, z))
                })
            }
            op::DP2 | op::DP3 | op::DP4 => {
                let n = (i.op - op::DP2 + 2) as usize;
                let (a, b) = (o(1)?, o(2)?);
                let mut acc = None;
                for c in 0..n {
                    let (x, y) = (self.src_f(&a, c)?, self.src_f(&b, c)?);
                    let m = self.bin(BinOp::Mul, x, y);
                    acc = Some(match acc {
                        None => m,
                        Some(s) => self.bin(BinOp::Add, s, m),
                    });
                }
                let dot = acc.unwrap_or_else(|| self.f32c(0.0));
                self.each_f(i, |_, _| Ok(dot))
            }
            op::FRC | op::ROUND_NE | op::ROUND_NI | op::ROUND_PI | op::ROUND_Z | op::RSQ | op::SQRT | op::EXP | op::LOG | op::RCP => {
                let s = o(1)?;
                let code = i.op;
                self.each_f(i, |t, c| {
                    let x = t.src_f(&s, c)?;
                    Ok(match code {
                        op::FRC => {
                            let f = t.un(UnOp::Floor, x);
                            t.bin(BinOp::Sub, x, f)
                        }
                        op::ROUND_NE => t.un(UnOp::RoundEven, x),
                        op::ROUND_NI => t.un(UnOp::Floor, x),
                        op::ROUND_PI => t.un(UnOp::Ceil, x),
                        op::ROUND_Z => t.un(UnOp::Trunc, x),
                        op::SQRT => t.un(UnOp::Sqrt, x),
                        op::EXP => t.un(UnOp::Exp2, x),
                        op::LOG => t.un(UnOp::Log2, x),
                        op::RSQ => {
                            let r = t.un(UnOp::Sqrt, x);
                            let one = t.f32c(1.0);
                            t.bin(BinOp::Div, one, r)
                        }
                        _ => {
                            let one = t.f32c(1.0);
                            t.bin(BinOp::Div, one, x)
                        }
                    })
                })
            }
            op::SINCOS => {
                let (ds, dc, s) = (o(0)?, o(1)?, o(2)?);
                let mut sin = [None; 4];
                let mut cos = [None; 4];
                for c in 0..4 {
                    let want = (ds.ty != ty::NULL && ds.mask() & (1 << c) != 0, dc.ty != ty::NULL && dc.mask() & (1 << c) != 0);
                    if !want.0 && !want.1 {
                        continue;
                    }
                    let x = self.src_f(&s, c)?;
                    for (on, out, f) in [(want.0, &mut sin, UnOp::Sin), (want.1, &mut cos, UnOp::Cos)] {
                        if on {
                            let mut v = self.un(f, x);
                            if i.saturate {
                                v = self.saturate(v);
                            }
                            out[c] = Some(self.cast(v, Scalar::U32));
                        }
                    }
                }
                self.write(&ds, sin)?;
                self.write(&dc, cos)
            }
            op::EQ | op::NE | op::LT | op::GE => {
                let c_op = match i.op {
                    op::EQ => Cmp::Eq,
                    op::NE => Cmp::Ne,
                    op::LT => Cmp::Lt,
                    _ => Cmp::Ge,
                };
                let (a, b) = (o(1)?, o(2)?);
                self.each(&o(0)?, |t, c| {
                    let (x, y) = (t.src_f(&a, c)?, t.src_f(&b, c)?);
                    let r = t.cmp(c_op, x, y);
                    Ok(t.mask_of(r))
                })
            }
            op::IEQ | op::INE | op::ILT | op::IGE | op::ULT | op::UGE => {
                let (c_op, s) = match i.op {
                    op::IEQ => (Cmp::Eq, Scalar::U32),
                    op::INE => (Cmp::Ne, Scalar::U32),
                    op::ILT => (Cmp::Lt, Scalar::I32),
                    op::IGE => (Cmp::Ge, Scalar::I32),
                    op::ULT => (Cmp::Lt, Scalar::U32),
                    _ => (Cmp::Ge, Scalar::U32),
                };
                let (a, b) = (o(1)?, o(2)?);
                self.each(&o(0)?, |t, c| {
                    let (x, y) = (t.src_int(&a, c, s)?, t.src_int(&b, c, s)?);
                    let r = t.cmp(c_op, x, y);
                    Ok(t.mask_of(r))
                })
            }
            op::MOVC | op::SWAPC => {
                let swap = i.op == op::SWAPC;
                let first = if swap { 2 } else { 1 };
                let (cnd, a, b) = (o(first)?, o(first + 1)?, o(first + 2)?);
                let (d0, d1) = (o(0)?, if swap { Some(o(1)?) } else { None });
                let (mut first_lanes, mut second_lanes) = ([None; 4], [None; 4]);
                let mask = d0.mask() | d1.as_ref().map_or(0, |d| d.mask());
                for (c, (first_lane, second_lane)) in first_lanes.iter_mut().zip(second_lanes.iter_mut()).enumerate() {
                    if mask & (1 << c) == 0 {
                        continue;
                    }
                    let x = self.raw(&cnd, c)?;
                    let zero = self.u32c(0);
                    let cond = self.cmp(Cmp::Ne, x, zero);
                    let (va, vb) = (self.src_bits(&a, c)?, self.src_bits(&b, c)?);
                    if swap {
                        *first_lane = Some(self.select(cond, vb, va));
                        *second_lane = Some(self.select(cond, va, vb));
                    } else {
                        let mut v = self.select(cond, va, vb);
                        if i.saturate {
                            let f = self.cast(v, Scalar::F32);
                            let f = self.saturate(f);
                            v = self.cast(f, Scalar::U32);
                        }
                        *first_lane = Some(v);
                    }
                }
                let only = |m: u8, l: [Option<Value>; 4]| core::array::from_fn(|c| if m & (1 << c) != 0 { l[c] } else { None });
                self.write(&d0, only(d0.mask(), first_lanes))?;
                if let Some(d1) = d1 {
                    self.write(&d1, only(d1.mask(), second_lanes))?;
                }
                Ok(())
            }
            op::IADD | op::AND | op::OR | op::XOR | op::ISHL | op::ISHR | op::USHR | op::IMIN | op::IMAX | op::UMIN | op::UMAX => {
                let (bop, s) = match i.op {
                    op::IADD => (BinOp::Add, Scalar::U32),
                    op::AND => (BinOp::And, Scalar::U32),
                    op::OR => (BinOp::Or, Scalar::U32),
                    op::XOR => (BinOp::Xor, Scalar::U32),
                    op::ISHL => (BinOp::Shl, Scalar::U32),
                    op::ISHR => (BinOp::Shr, Scalar::I32),
                    op::USHR => (BinOp::Shr, Scalar::U32),
                    op::IMIN => (BinOp::Min, Scalar::I32),
                    op::IMAX => (BinOp::Max, Scalar::I32),
                    op::UMIN => (BinOp::Min, Scalar::U32),
                    _ => (BinOp::Max, Scalar::U32),
                };
                let (a, b) = (o(1)?, o(2)?);
                self.each(&o(0)?, |t, c| {
                    let x = t.src_int(&a, c, s)?;
                    // A shift's count is an unsigned word, whatever the shifted value's type.
                    let y = if matches!(bop, BinOp::Shl | BinOp::Shr) { t.src_int(&b, c, Scalar::U32)? } else { t.src_int(&b, c, s)? };
                    let y = t.cast(y, s);
                    let r = t.bin(bop, x, y);
                    Ok(t.cast(r, Scalar::U32))
                })
            }
            op::NOT | op::INEG | op::COUNTBITS => {
                let uop = match i.op {
                    op::NOT => UnOp::Not,
                    op::INEG => UnOp::Neg,
                    _ => UnOp::Popcount,
                };
                let s = o(1)?;
                self.each(&o(0)?, |t, c| {
                    let x = t.src_int(&s, c, Scalar::U32)?;
                    Ok(t.un(uop, x))
                })
            }
            op::IMAD | op::UMAD => {
                let (a, b, s) = (o(1)?, o(2)?, o(3)?);
                self.each(&o(0)?, |t, c| {
                    let (x, y, z) = (t.src_int(&a, c, Scalar::U32)?, t.src_int(&b, c, Scalar::U32)?, t.src_int(&s, c, Scalar::U32)?);
                    let m = t.bin(BinOp::Mul, x, y);
                    Ok(t.bin(BinOp::Add, m, z))
                })
            }
            op::IMUL | op::UMUL => {
                let (wide, narrow) = if i.op == op::IMUL { (Scalar::I64, Scalar::I32) } else { (Scalar::U64, Scalar::U32) };
                let (hi, lo, a, b) = (o(0)?, o(1)?, o(2)?, o(3)?);
                let mut his = [None; 4];
                let mut los = [None; 4];
                for c in 0..4 {
                    let (want_hi, want_lo) = (hi.ty != ty::NULL && hi.mask() & (1 << c) != 0, lo.ty != ty::NULL && lo.mask() & (1 << c) != 0);
                    if !want_hi && !want_lo {
                        continue;
                    }
                    let (x, y) = (self.src_int(&a, c, narrow)?, self.src_int(&b, c, narrow)?);
                    if want_lo {
                        let (xu, yu) = (self.cast(x, Scalar::U32), self.cast(y, Scalar::U32));
                        los[c] = Some(self.bin(BinOp::Mul, xu, yu));
                    }
                    if want_hi {
                        let xw = self.let_(Type::Scalar(wide), Op::Convert(x, wide));
                        let yw = self.let_(Type::Scalar(wide), Op::Convert(y, wide));
                        let m = self.bin(BinOp::Mul, xw, yw);
                        let k = self.let_(Type::Scalar(wide), Op::Const(wide, 32));
                        let h = self.bin(BinOp::Shr, m, k);
                        let h = self.let_(Type::Scalar(narrow), Op::Convert(h, narrow));
                        his[c] = Some(self.cast(h, Scalar::U32));
                    }
                }
                self.write(&hi, his)?;
                self.write(&lo, los)
            }
            op::UDIV => {
                let (dq, dr, a, b) = (o(0)?, o(1)?, o(2)?, o(3)?);
                let mut qs = [None; 4];
                let mut rs = [None; 4];
                for c in 0..4 {
                    let (want_q, want_r) = (dq.ty != ty::NULL && dq.mask() & (1 << c) != 0, dr.ty != ty::NULL && dr.mask() & (1 << c) != 0);
                    if !want_q && !want_r {
                        continue;
                    }
                    let (x, y) = (self.src_int(&a, c, Scalar::U32)?, self.src_int(&b, c, Scalar::U32)?);
                    let zero = self.u32c(0);
                    let by_zero = self.cmp(Cmp::Eq, y, zero);
                    let ones = self.u32c(u32::MAX);
                    if want_q {
                        let q = self.bin(BinOp::Div, x, y);
                        qs[c] = Some(self.select(by_zero, ones, q));
                    }
                    if want_r {
                        let r = self.bin(BinOp::Rem, x, y);
                        rs[c] = Some(self.select(by_zero, ones, r));
                    }
                }
                self.write(&dq, qs)?;
                self.write(&dr, rs)
            }
            op::FTOI | op::FTOU => {
                let to = if i.op == op::FTOI { Scalar::I32 } else { Scalar::U32 };
                let s = o(1)?;
                self.each(&o(0)?, |t, c| {
                    let x = t.src_f(&s, c)?;
                    let v = t.let_(Type::Scalar(to), Op::Convert(x, to));
                    Ok(t.cast(v, Scalar::U32))
                })
            }
            op::ITOF | op::UTOF => {
                let from = if i.op == op::ITOF { Scalar::I32 } else { Scalar::U32 };
                let s = o(1)?;
                self.each_f(i, |t, c| {
                    let x = t.src_int(&s, c, from)?;
                    Ok(t.let_(Type::F32, Op::Convert(x, Scalar::F32)))
                })
            }
            op::DERIV_RTX | op::DERIV_RTY | op::DERIV_RTX_COARSE | op::DERIV_RTX_FINE | op::DERIV_RTY_COARSE | op::DERIV_RTY_FINE => {
                if self.e.stage != Stage::Fragment {
                    return fail("a derivative outside a pixel shader");
                }
                let axis = if matches!(i.op, op::DERIV_RTX | op::DERIV_RTX_COARSE | op::DERIV_RTX_FINE) { Axis::X } else { Axis::Y };
                let s = o(1)?;
                self.each_f(i, |t, c| {
                    let x = t.src_f(&s, c)?;
                    Ok(t.let_(Type::F32, Op::Derivative(axis, x)))
                })
            }
            op::SAMPLE | op::SAMPLE_L | op::SAMPLE_B => self.sample(i),
            op::LD_RAW | op::LD_STRUCTURED | op::STORE_RAW | op::STORE_STRUCTURED => self.memory(i),
            op::ATOMIC_AND..=op::ATOMIC_UMIN | op::IMM_ATOMIC_IADD..=op::IMM_ATOMIC_UMIN => self.atomic(i),
            op::SYNC => {
                let f = (i.token >> 11) & 0xf;
                self.emit(Stmt::Barrier(Barrier { execution: f & 1 != 0, workgroup_memory: f & 2 != 0, device_memory: f & 0xc != 0 }));
                Ok(())
            }
            op::LD => fail("ld (texel fetches) isn't read yet"),
            70 | 71 => fail("comparison samples (sample_c) aren't read yet"),
            73 => fail("samples with gradients (sample_d) aren't read yet"),
            n => fail(alloc::format!("opcode {n} isn't read yet")),
        }
    }
}
