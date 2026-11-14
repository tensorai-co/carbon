//! GLSL's syntax tree to the IR, type-checking as it goes: one entry point for `main`, with every function it calls
//! inlined (GLSL has no recursion), every composite split into scalars and vectors, and every resource a binding.
//!
//! - **Variables** live in places ([`Place`]): IR variables for locals and globals (a tree of them for a matrix,
//!   struct or array), memory for blocks and shared variables, and read-only values for inputs and built-ins, made
//!   in the prologue so they dominate every use.
//! - **Control flow** maps onto the IR's: `if`, loops (`for`'s step is the loop's continuing block, `do`'s condition
//!   its `break_if`), and `switch` as a loop run once with fallthrough guards. An inlined function's body is a loop run
//!   once too: `return` writes its result, sets the frame's flag and breaks, and each loop inside it that a return
//!   left is followed by a check of the flag that breaks again.
//! - **Indexing** with a value that isn't constant reads every element and selects, and writes every element with a
//!   select, for what's in variables; memory is addressed.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::sync::Arc;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use gpu_ir::{Binding, BindingKind, Block, Builtin, Cmp, Entry, Interpolation, Io, Op, Region, Scalar, Slot, Space, Stage as IrStage, Stmt, Type, Value, Var};

use super::ast::{self, Cond, Declarator, ExprKind, External, Init, Qualifiers, StmtKind, Storage, TypeBase, TypeSpec};
use super::types::{self, Base, Packing, StructTy, Ty};
use super::{BlockInfo, Error, Interface, Stage, Uniform, Variable, Version, error};

/// Where a value of a type lives, as an assignment or a read reaches it.
#[derive(Debug, Clone)]
pub(super) enum Place {
    /// A scalar or vector in an IR variable.
    Reg(Var, Ty),
    /// A composite's parts, each a place: a matrix's columns, a struct's members, an array's elements.
    Parts(Vec<Place>, Ty),
    /// Lanes of a vector place, by a swizzle.
    Lanes(Box<Place>, Vec<u8>, Ty),
    /// One lane of a vector place, by a value.
    Lane(Box<Place>, Value, Ty),
    /// One of a composite's parts, by a value.
    Part(Vec<Place>, Value, Ty),
    /// Memory: a binding (or workgroup region), a byte offset and an offset in bytes by value, the type there, and how
    /// it's laid out.
    Mem { base: Base_, offset: u32, dynamic: Option<Value>, ty: Ty, packing: Packing, row_major: bool },
    /// A value that can't be assigned: an input, a built-in, a constant's.
    Value(Val, Ty),
}

/// What memory a place is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Base_ {
    Binding(u32),
    Workgroup(u32),
}

/// A value of any type: a scalar or vector as one IR value, a composite as its parts, a sampler as its binding.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Val {
    V(Value),
    Parts(Vec<Val>),
    Sampler(u32),
    Void,
}

impl Val {
    pub(super) fn v(&self) -> Value {
        match self {
            Val::V(v) => *v,
            _ => panic!("a composite where a scalar or vector is"),
        }
    }
}

impl Place {
    pub(super) fn ty(&self) -> &Ty {
        match self {
            Place::Reg(_, t) | Place::Parts(_, t) | Place::Lanes(_, _, t) | Place::Lane(_, _, t) | Place::Part(_, _, t) | Place::Value(_, t) => t,
            Place::Mem { ty, .. } => ty,
        }
    }
}

/// A symbol in scope.
#[derive(Debug, Clone)]
pub(super) enum Sym {
    Var { place: Place, constant: Option<Konst>, readonly: bool },
    Type(Ty),
}

/// A constant scalar, for array sizes, case labels and layouts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Konst {
    Int(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub(super) struct Func {
    pub name: String,
    pub params: Vec<(Storage, Ty, Option<String>)>,
    pub ret: Ty,
    pub body: Option<Arc<Vec<ast::Stmt>>>,
    pub line: u32,
}

/// An inlined function's call: where its result goes, and its returned flag.
struct Frame {
    ret: Option<Place>,
    done: Var,
    /// The depth of `ctxs` at its body's loop.
    base: usize,
}

/// What a `break` or `continue` leaves: a loop, a `switch` (its `continue` flag), or a function's body.
struct Ctx {
    kind: CtxKind,
    /// A `return` happened inside it.
    returned: bool,
}

enum CtxKind {
    Loop,
    Switch { cont: Var, used: bool },
    Function,
}

/// A program output, written as the entry ends.
struct Out {
    place: Place,
    /// Its outputs, one for each location it takes.
    io: Vec<u32>,
}

pub(super) struct Lower {
    pub e: Entry,
    pub stage: Stage,
    pub version: Version,
    pub iface: Interface,
    pub line: u32,
    scopes: Vec<BTreeMap<String, Sym>>,
    functions: BTreeMap<String, Vec<Arc<Func>>>,
    frames: Vec<Frame>,
    ctxs: Vec<Ctx>,
    outputs: Vec<Out>,
    /// Lets that must come before everything: inputs and built-ins.
    prologue: Block,
    /// The default uniform block's members, as declared.
    defaults: Vec<(String, Ty, Option<u32>)>,
    /// Its binding, made when the first plain uniform is.
    default_binding: Option<u32>,
    /// Inlining depth, to find recursion.
    depth: usize,
    /// Blocks' default packing and matrices (`layout(std430) buffer;`).
    uniform_packing: Packing,
    buffer_packing: Packing,
    row_major: bool,
    /// Fragment shaders: `gl_FragColor`'s or `gl_FragData`'s outputs were made.
    frag_outputs: bool,
    /// The compatibility profile's built-ins are there (GLSL before 1.40, or `compatibility`).
    pub compat: bool,
}

/// Locations GLSL's `gl_FragData` has here (`GL_MAX_DRAW_BUFFERS`).
pub const DRAW_BUFFERS: u32 = 4;

impl Lower {
    pub fn new(stage: Stage, version: Version) -> Lower {
        let ir_stage = match stage {
            Stage::Vertex => IrStage::Vertex,
            Stage::Fragment => IrStage::Fragment,
            Stage::Compute => IrStage::Compute,
        };
        let e = if ir_stage == IrStage::Compute { Entry::new("main", [1, 1, 1]) } else { Entry::shader("main", ir_stage) };
        Lower {
            e,
            stage,
            version,
            iface: Interface::default(),
            line: 0,
            scopes: vec![BTreeMap::new()],
            functions: BTreeMap::new(),
            frames: Vec::new(),
            ctxs: Vec::new(),
            outputs: Vec::new(),
            prologue: Vec::new(),
            defaults: Vec::new(),
            default_binding: None,
            depth: 0,
            uniform_packing: Packing::Std140,
            buffer_packing: Packing::Std430,
            row_major: false,
            frag_outputs: false,
            compat: false,
        }
    }

    pub fn err<T>(&self, message: &str) -> Result<T, Error> {
        error(self.line, message)
    }

    // ---- The IR ----

    pub fn let_(&mut self, out: &mut Block, ty: Type, op: Op) -> Value {
        let v = self.e.value(ty);
        out.push(Stmt::Let(v, op));
        v
    }

    pub fn konst(&mut self, out: &mut Block, scalar: Scalar, bits: u64) -> Value {
        self.let_(out, Type::Scalar(scalar), Op::Const(scalar, bits))
    }

    pub fn int_const(&mut self, out: &mut Block, v: i64) -> Value {
        self.konst(out, Scalar::I32, v as u64)
    }

    pub fn uint_const(&mut self, out: &mut Block, v: u32) -> Value {
        self.konst(out, Scalar::U32, u64::from(v))
    }

    pub fn bool_const(&mut self, out: &mut Block, b: bool) -> Value {
        self.konst(out, Scalar::Bool, u64::from(b))
    }

    pub fn float_const(&mut self, out: &mut Block, f: f32) -> Value {
        self.konst(out, Scalar::F32, u64::from(f.to_bits()))
    }

    /// A scalar spread over a vector's lanes (itself, for a scalar type).
    pub fn splat(&mut self, out: &mut Block, scalar: Value, ty: Type) -> Value {
        match ty {
            Type::Vector(_, n) => self.let_(out, ty, Op::Construct(vec![scalar; n as usize])),
            _ => scalar,
        }
    }

    /// A constant of a scalar or vector type: `f` for floats, the integer for integers, nonzero for booleans.
    pub fn number(&mut self, out: &mut Block, ty: &Ty, f: f64) -> Value {
        let base = ty.base().unwrap_or(Base::Float);
        let s = match base {
            Base::Float => self.float_const(out, f as f32),
            Base::Int => self.int_const(out, f as i64),
            Base::Uint => self.uint_const(out, f as u32),
            Base::Bool => self.bool_const(out, f != 0.0),
        };
        let ir = ty.ir().unwrap_or(Type::F32);
        self.splat(out, s, ir)
    }

    // ---- Scopes ----

    pub fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn declare(&mut self, name: &str, sym: Sym) -> Result<(), Error> {
        if name.starts_with("gl_") {
            return self.err(&format!("{name} is reserved"));
        }
        let scope = self.scopes.last_mut().expect("a scope");
        if scope.contains_key(name) {
            return error(self.line, &format!("{name} is declared twice"));
        }
        scope.insert(name.to_string(), sym);
        Ok(())
    }

    pub fn lookup(&mut self, name: &str) -> Result<Option<Sym>, Error> {
        for s in self.scopes.iter().rev() {
            if let Some(sym) = s.get(name) {
                return Ok(Some(sym.clone()));
            }
        }
        if name.starts_with("gl_") {
            return self.builtin_var(name);
        }
        Ok(None)
    }

    // ---- Types ----

    pub fn resolve(&mut self, spec: &TypeSpec) -> Result<Ty, Error> {
        let base = match &spec.base {
            TypeBase::Named(n) => match types::named(n) {
                Some(t) => {
                    if matches!(t, Ty::Scalar(Base::Uint) | Ty::Vector(Base::Uint, _)) && self.version.number < if self.version.es { 300 } else { 130 } {
                        return self.err(&format!("{n} isn't in GLSL {}", self.version));
                    }
                    t
                }
                None => match self.lookup(n)? {
                    Some(Sym::Type(t)) => t,
                    _ => return self.err(&format!("{n} isn't a type")),
                },
            },
            TypeBase::Struct(def) => self.struct_type(def)?,
        };
        self.with_dims(base, &spec.dims)
    }

    /// A type with array sizes: outermost first in the list, so the innermost is applied first.
    pub fn with_dims(&mut self, mut ty: Ty, dims: &[Option<ast::Expr>]) -> Result<Ty, Error> {
        for d in dims.iter().rev() {
            let n = match d {
                Some(e) => {
                    let n = self.const_int(e)?;
                    if n <= 0 {
                        return self.err(&format!("an array of {n} elements"));
                    }
                    Some(n as u32)
                }
                None => None,
            };
            ty = Ty::Array(Box::new(ty), n);
        }
        Ok(ty)
    }

    fn struct_type(&mut self, def: &ast::StructDef) -> Result<Ty, Error> {
        let mut members = Vec::new();
        for m in &def.members {
            let base = self.resolve(&m.ty)?;
            for (name, dims) in &m.names {
                if members.iter().any(|(n, _)| n == name) {
                    return self.err(&format!("member {name} twice"));
                }
                let ty = self.with_dims(base.clone(), dims)?;
                if matches!(ty, Ty::Array(_, None)) {
                    return self.err(&format!("{name} is an array without a size"));
                }
                members.push((name.clone(), ty));
            }
        }
        let name = def.name.clone().unwrap_or_else(|| format!("struct@{}", def.line));
        let ty = Ty::Struct(Arc::new(StructTy { name: name.clone(), members }));
        if def.name.is_some() {
            self.declare(&name, Sym::Type(ty.clone()))?;
        }
        Ok(ty)
    }

    // ---- Places ----

    /// Fresh variables for a value of a type.
    pub fn alloc(&mut self, ty: &Ty) -> Result<Place, Error> {
        Ok(match ty {
            Ty::Scalar(_) | Ty::Vector(..) => {
                let v = self.e.var(ty.ir().expect("a scalar or vector"));
                Place::Reg(v, ty.clone())
            }
            Ty::Matrix(c, _) => {
                let col = ty.column().expect("a matrix");
                let parts = (0..*c).map(|_| self.alloc(&col)).collect::<Result<_, _>>()?;
                Place::Parts(parts, ty.clone())
            }
            Ty::Array(e, Some(n)) => {
                if *n > 4096 {
                    return self.err(&format!("an array of {n} elements in registers"));
                }
                let parts = (0..*n).map(|_| self.alloc(e)).collect::<Result<_, _>>()?;
                Place::Parts(parts, ty.clone())
            }
            Ty::Struct(s) => {
                let parts = s.members.iter().map(|(_, m)| self.alloc(m)).collect::<Result<_, _>>()?;
                Place::Parts(parts, ty.clone())
            }
            Ty::Array(_, None) => return self.err("an array without a size"),
            Ty::Sampler(_) => return self.err("a sampler is only a uniform or a parameter"),
            Ty::Void => return self.err("a void variable"),
        })
    }

    pub fn load(&mut self, out: &mut Block, place: &Place) -> Result<Val, Error> {
        Ok(match place {
            Place::Reg(v, ty) => Val::V(self.let_(out, ty.ir().expect("scalar"), Op::Read(*v))),
            Place::Parts(parts, _) => Val::Parts(parts.iter().map(|p| self.load(out, p)).collect::<Result<_, _>>()?),
            Place::Value(v, _) => v.clone(),
            Place::Lanes(p, lanes, ty) => {
                let v = self.load(out, p)?.v();
                self.swizzle(out, v, lanes, ty)
            }
            Place::Lane(p, i, ty) => {
                let v = self.load(out, p)?.v();
                Val::V(self.let_(out, ty.ir().expect("scalar"), Op::ExtractDynamic(v, *i)))
            }
            Place::Part(parts, i, ty) => {
                let mut result = self.load(out, &parts[0])?;
                for (k, p) in parts.iter().enumerate().skip(1) {
                    let here = self.load(out, p)?;
                    let k = self.int_const(out, k as i64);
                    let at = self.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, *i, k));
                    result = self.select(out, at, here, result, ty)?;
                }
                result
            }
            Place::Mem { base, offset, dynamic, ty, packing, row_major } => self.load_mem(out, *base, *offset, *dynamic, ty, *packing, *row_major)?,
        })
    }

    pub fn store(&mut self, out: &mut Block, place: &Place, val: Val) -> Result<(), Error> {
        match place {
            Place::Reg(v, _) => out.push(Stmt::Write(*v, val.v())),
            Place::Parts(parts, _) => {
                let Val::Parts(vals) = val else { return self.err("a composite assigned a scalar") };
                for (p, v) in parts.iter().zip(vals) {
                    self.store(out, p, v)?;
                }
            }
            Place::Value(..) => return self.err("that can't be assigned"),
            Place::Lanes(p, lanes, _) => {
                let old = self.load(out, p)?.v();
                let ty = p.ty().clone();
                let mut v = old;
                let new = val.v();
                for (k, lane) in lanes.iter().enumerate() {
                    let part = if lanes.len() == 1 { new } else { self.let_(out, ty.with_base(ty.base().expect("vector")).ir().map(scalar_of).expect("lane"), Op::Extract(new, k as u8)) };
                    v = self.let_(out, ty.ir().expect("vector"), Op::Insert(v, *lane, part));
                }
                self.store(out, p, Val::V(v))?;
            }
            Place::Lane(p, i, _) => {
                let ty = p.ty().clone();
                let ir = ty.ir().expect("vector");
                let old = self.load(out, p)?.v();
                let mut v = old;
                for lane in 0..ty.lanes() {
                    let k = self.int_const(out, i64::from(lane));
                    let at = self.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, *i, k));
                    let was = self.let_(out, scalar_of(ir), Op::Extract(old, lane));
                    let now = self.let_(out, scalar_of(ir), Op::Select(at, val.v(), was));
                    v = self.let_(out, ir, Op::Insert(v, lane, now));
                }
                self.store(out, p, Val::V(v))?;
            }
            Place::Part(parts, i, ty) => {
                for (k, p) in parts.iter().enumerate() {
                    let was = self.load(out, p)?;
                    let kk = self.int_const(out, k as i64);
                    let at = self.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, *i, kk));
                    let now = self.select(out, at, val.clone(), was, ty)?;
                    self.store(out, p, now)?;
                }
            }
            Place::Mem { base, offset, dynamic, ty, packing, row_major } => self.store_mem(out, *base, *offset, *dynamic, ty, *packing, *row_major, val)?,
        }
        Ok(())
    }

    /// `cond ? a : b` of any type, part by part.
    pub fn select(&mut self, out: &mut Block, cond: Value, a: Val, b: Val, ty: &Ty) -> Result<Val, Error> {
        Ok(match (a, b) {
            (Val::V(a), Val::V(b)) => {
                let ir = self.e.ty(a);
                Val::V(self.let_(out, ir, Op::Select(cond, a, b)))
            }
            (Val::Parts(a), Val::Parts(b)) => {
                let parts = part_types(ty);
                let mut v = Vec::new();
                for (k, (x, y)) in a.into_iter().zip(b).enumerate() {
                    v.push(self.select(out, cond, x, y, parts.get(k).unwrap_or(ty))?);
                }
                Val::Parts(v)
            }
            (Val::Sampler(a), Val::Sampler(b)) if a == b => Val::Sampler(a),
            _ => return self.err("samplers chosen between at run time"),
        })
    }

    fn swizzle(&mut self, out: &mut Block, v: Value, lanes: &[u8], ty: &Ty) -> Val {
        let ir = ty.ir().expect("scalar or vector");
        if lanes.len() == 1 {
            return Val::V(self.let_(out, ir, Op::Extract(v, lanes[0])));
        }
        let s = scalar_of(ir);
        let parts = lanes.iter().map(|&l| self.let_(out, s, Op::Extract(v, l))).collect();
        Val::V(self.let_(out, ir, Op::Construct(parts)))
    }

    /// The address of a place in memory.
    fn address(&mut self, out: &mut Block, base: Base_, offset: u32, dynamic: Option<Value>) -> Value {
        let space = match base {
            Base_::Binding(_) => Space::Device,
            Base_::Workgroup(_) => Space::Workgroup,
        };
        let p = match base {
            Base_::Binding(n) => self.let_(out, Type::Ptr(space), Op::Binding(n)),
            Base_::Workgroup(n) => self.let_(out, Type::Ptr(space), Op::Workgroup(n)),
        };
        let mut at = p;
        if offset != 0 {
            let o = self.uint_const(out, offset);
            at = self.let_(out, Type::Ptr(space), Op::Offset(at, o));
        }
        if let Some(d) = dynamic {
            at = self.let_(out, Type::Ptr(space), Op::Offset(at, d));
        }
        at
    }

    #[allow(clippy::too_many_arguments)]
    fn load_mem(&mut self, out: &mut Block, base: Base_, offset: u32, dynamic: Option<Value>, ty: &Ty, packing: Packing, row_major: bool) -> Result<Val, Error> {
        Ok(match ty {
            Ty::Scalar(_) | Ty::Vector(..) => {
                let at = self.address(out, base, offset, dynamic);
                let ir = ty.ir().expect("scalar");
                if ty.base() == Some(Base::Bool) {
                    // Booleans are words in memory: anything but 0 is true.
                    let w = ir.with_scalar(Scalar::U32);
                    let word = self.let_(out, w, Op::Load(at, w));
                    let zero = self.uint_const(out, 0);
                    let zero = self.splat(out, zero, w);
                    Val::V(self.let_(out, ir, Op::Compare(Cmp::Ne, word, zero)))
                } else {
                    Val::V(self.let_(out, ir, Op::Load(at, ir)))
                }
            }
            Ty::Matrix(c, r) if row_major => {
                // Rows in memory: each column gathered from them.
                let stride = types::matrix_stride(*c, packing);
                let rows: Vec<Value> = (0..*r)
                    .map(|i| {
                        let at = self.address(out, base, offset + stride * u32::from(i), dynamic);
                        let rt = Type::Vector(Scalar::F32, *c);
                        self.let_(out, rt, Op::Load(at, rt))
                    })
                    .collect();
                let mut cols = Vec::new();
                for j in 0..*c {
                    let lanes = rows.iter().map(|&row| self.let_(out, Type::F32, Op::Extract(row, j))).collect();
                    cols.push(Val::V(self.let_(out, Type::Vector(Scalar::F32, *r), Op::Construct(lanes))));
                }
                Val::Parts(cols)
            }
            Ty::Matrix(c, r) => {
                let stride = types::matrix_stride(*r, packing);
                let col = ty.column().expect("matrix");
                let mut cols = Vec::new();
                for i in 0..*c {
                    cols.push(self.load_mem(out, base, offset + stride * u32::from(i), dynamic, &col, packing, row_major)?);
                }
                Val::Parts(cols)
            }
            Ty::Array(e, Some(n)) => {
                let stride = types::array_stride(e, packing, row_major).map_err(|m| Error { message: m })?;
                let mut parts = Vec::new();
                for i in 0..*n {
                    parts.push(self.load_mem(out, base, offset + stride * i, dynamic, e, packing, row_major)?);
                }
                Val::Parts(parts)
            }
            Ty::Struct(s) => {
                let mut parts = Vec::new();
                for (i, (_, m)) in s.members.iter().enumerate() {
                    let o = types::member_offset(s, i, packing, row_major).map_err(|m| Error { message: m })?;
                    parts.push(self.load_mem(out, base, offset + o, dynamic, m, packing, row_major)?);
                }
                Val::Parts(parts)
            }
            _ => return self.err(&format!("{ty} read whole from memory")),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn store_mem(&mut self, out: &mut Block, base: Base_, offset: u32, dynamic: Option<Value>, ty: &Ty, packing: Packing, row_major: bool, val: Val) -> Result<(), Error> {
        match (ty, val) {
            (Ty::Scalar(_) | Ty::Vector(..), Val::V(v)) => {
                let at = self.address(out, base, offset, dynamic);
                let v = if ty.base() == Some(Base::Bool) {
                    let w = ty.ir().expect("scalar").with_scalar(Scalar::U32);
                    self.let_(out, w, Op::Convert(v, Scalar::U32))
                } else {
                    v
                };
                out.push(Stmt::Store(at, v));
            }
            (Ty::Matrix(c, r), Val::Parts(cols)) if row_major => {
                let stride = types::matrix_stride(*c, packing);
                for i in 0..*r {
                    let lanes = cols.iter().map(|col| self.let_(out, Type::F32, Op::Extract(col.v(), i))).collect();
                    let row = self.let_(out, Type::Vector(Scalar::F32, *c), Op::Construct(lanes));
                    let at = self.address(out, base, offset + stride * u32::from(i), dynamic);
                    out.push(Stmt::Store(at, row));
                }
            }
            (Ty::Matrix(_, r), Val::Parts(cols)) => {
                let stride = types::matrix_stride(*r, packing);
                let col = ty.column().expect("matrix");
                for (i, v) in cols.into_iter().enumerate() {
                    self.store_mem(out, base, offset + stride * i as u32, dynamic, &col, packing, row_major, v)?;
                }
            }
            (Ty::Array(e, _), Val::Parts(parts)) => {
                let stride = types::array_stride(e, packing, row_major).map_err(|m| Error { message: m })?;
                for (i, v) in parts.into_iter().enumerate() {
                    self.store_mem(out, base, offset + stride * i as u32, dynamic, e, packing, row_major, v)?;
                }
            }
            (Ty::Struct(s), Val::Parts(parts)) => {
                for (i, ((_, m), v)) in s.members.iter().zip(parts).enumerate() {
                    let o = types::member_offset(s, i, packing, row_major).map_err(|m| Error { message: m })?;
                    self.store_mem(out, base, offset + o, dynamic, m, packing, row_major, v)?;
                }
            }
            _ => return self.err(&format!("{ty} written to memory")),
        }
        Ok(())
    }

    /// A place's part: a struct's member, by index.
    pub fn member(&mut self, place: Place, index: usize) -> Result<Place, Error> {
        Ok(match place {
            Place::Parts(mut parts, _) => parts.swap_remove(index),
            Place::Value(Val::Parts(mut v), Ty::Struct(s)) => Place::Value(v.swap_remove(index), s.members[index].1.clone()),
            Place::Mem { base, offset, dynamic, ty: Ty::Struct(s), packing, row_major } => {
                let o = types::member_offset(&s, index, packing, row_major).map_err(|m| Error { message: m })?;
                Place::Mem { base, offset: offset + o, dynamic, ty: s.members[index].1.clone(), packing, row_major }
            }
            Place::Part(parts, i, Ty::Struct(s)) => {
                let ty = s.members[index].1.clone();
                let parts = parts.into_iter().map(|p| self.member(p, index)).collect::<Result<_, _>>()?;
                Place::Part(parts, i, ty)
            }
            _ => return self.err("a member of something that isn't a struct"),
        })
    }

    /// A place's element by a constant: an array's, a matrix's column, a vector's lane.
    pub fn element(&mut self, out: &mut Block, place: Place, index: u32) -> Result<Place, Error> {
        let ty = place.ty().clone();
        let count = match &ty {
            Ty::Array(_, Some(n)) => *n,
            Ty::Array(_, None) => u32::MAX,
            Ty::Matrix(c, _) => u32::from(*c),
            Ty::Vector(_, n) => u32::from(*n),
            _ => return self.err(&format!("{ty} indexed")),
        };
        if index >= count {
            return self.err(&format!("index {index} is past {ty}'s end"));
        }
        let elem = element_type(&ty).expect("indexable");
        Ok(match place {
            Place::Parts(mut parts, _) => parts.swap_remove(index as usize),
            Place::Value(Val::Parts(mut v), _) => Place::Value(v.swap_remove(index as usize), elem),
            Place::Value(Val::V(v), _) => {
                let ir = elem.ir().expect("lane");
                let x = self.let_(out, ir, Op::Extract(v, index as u8));
                Place::Value(Val::V(x), elem)
            }
            Place::Mem { base, offset, dynamic, packing, row_major, .. } => {
                let o = self.element_stride(&ty, packing, row_major)? * index;
                if let Ty::Vector(..) = ty {
                    return Ok(Place::Mem { base, offset: offset + 4 * index, dynamic, ty: elem, packing, row_major });
                }
                Place::Mem { base, offset: offset + o, dynamic, ty: elem, packing, row_major }
            }
            p @ (Place::Reg(..) | Place::Lanes(..) | Place::Lane(..)) => Place::Lanes(Box::new(p), vec![index as u8], elem),
            Place::Value(..) => return self.err(&format!("{ty} indexed")),
            Place::Part(parts, i, _) => {
                let parts = parts.into_iter().map(|p| self.element(out, p, index)).collect::<Result<_, _>>()?;
                Place::Part(parts, i, elem)
            }
        })
    }

    /// A place's element by a value (an `int` or `uint`).
    pub fn element_dyn(&mut self, out: &mut Block, place: Place, index: Value) -> Result<Place, Error> {
        let ty = place.ty().clone();
        let elem = element_type(&ty).ok_or_else(|| Error { message: format!("0:{}: error: {ty} indexed", self.line) })?;
        let index = self.as_int(out, index);
        Ok(match place {
            Place::Parts(parts, _) => Place::Part(parts, index, elem),
            Place::Value(Val::Parts(v), t) => {
                let parts = v.into_iter().map(|x| Place::Value(x, element_type(&t).expect("indexable"))).collect();
                Place::Part(parts, index, elem)
            }
            Place::Value(Val::V(v), _) => {
                let x = self.let_(out, elem.ir().expect("lane"), Op::ExtractDynamic(v, index));
                Place::Value(Val::V(x), elem)
            }
            Place::Mem { base, offset, dynamic, packing, row_major, .. } => {
                let stride = if matches!(ty, Ty::Vector(..)) { 4 } else { self.element_stride(&ty, packing, row_major)? };
                // An index past a sized array's end is clamped to it, so memory past the array is never touched.
                let index = match &ty {
                    Ty::Array(_, Some(n)) if *n > 0 => {
                        let zero = self.int_const(out, 0);
                        let last = self.int_const(out, i64::from(*n) - 1);
                        let low = self.let_(out, Type::I32, Op::Binary(gpu_ir::BinOp::Max, index, zero));
                        self.let_(out, Type::I32, Op::Binary(gpu_ir::BinOp::Min, low, last))
                    }
                    _ => index,
                };
                let u = self.let_(out, Type::U32, Op::Bitcast(index, Type::U32));
                let s = self.uint_const(out, stride);
                let bytes = self.let_(out, Type::U32, Op::Binary(gpu_ir::BinOp::Mul, u, s));
                let dynamic = match dynamic {
                    Some(d) => Some(self.let_(out, Type::U32, Op::Binary(gpu_ir::BinOp::Add, d, bytes))),
                    None => Some(bytes),
                };
                Place::Mem { base, offset, dynamic, ty: elem, packing, row_major }
            }
            p @ (Place::Reg(..) | Place::Lanes(..) | Place::Lane(..)) => Place::Lane(Box::new(p), index, elem),
            Place::Value(..) => return self.err(&format!("{ty} indexed")),
            Place::Part(parts, i, _) => {
                // An element of each candidate, chosen by the outer index later.
                let parts = parts.into_iter().map(|p| self.element_dyn(out, p, index)).collect::<Result<_, _>>()?;
                Place::Part(parts, i, elem)
            }
        })
    }

    fn element_stride(&self, ty: &Ty, packing: Packing, row_major: bool) -> Result<u32, Error> {
        match ty {
            Ty::Array(e, _) => types::array_stride(e, packing, row_major).map_err(|m| Error { message: format!("0:{}: error: {m}", self.line) }),
            Ty::Matrix(_, r) if !row_major => Ok(types::matrix_stride(*r, packing)),
            Ty::Matrix(..) => self.err("a column of a row-major matrix in memory, by itself"),
            _ => Ok(4),
        }
    }

    /// An integer value as `int`.
    pub fn as_int(&mut self, out: &mut Block, v: Value) -> Value {
        match self.e.ty(v) {
            Type::Scalar(Scalar::I32) => v,
            Type::Scalar(Scalar::U32) => self.let_(out, Type::I32, Op::Bitcast(v, Type::I32)),
            _ => self.let_(out, Type::I32, Op::Convert(v, Scalar::I32)),
        }
    }

    // ---- The translation unit ----

    pub fn unit(&mut self, unit: &[External]) -> Result<(), Error> {
        let mut main = None;
        for ext in unit {
            match ext {
                External::Function(f) => {
                    self.line = f.line;
                    let func = self.function(f)?;
                    if f.name == "main" && f.body.is_some() {
                        if !func.params.is_empty() || func.ret != Ty::Void {
                            return self.err("main takes nothing and returns nothing");
                        }
                        main = Some(func);
                    }
                }
                External::Decl { quals, ty, vars, line } => {
                    self.line = *line;
                    self.global(quals, ty, vars)?;
                }
                External::Block(b) => {
                    self.line = b.line;
                    self.block(b)?;
                }
                External::Default { quals, line } => {
                    self.line = *line;
                    self.defaults(quals)?;
                }
                External::Requalify { quals, names, line } => {
                    self.line = *line;
                    for n in names {
                        if self.lookup(n)?.is_none() {
                            return self.err(&format!("{n} isn't declared"));
                        }
                    }
                    let _ = quals;
                }
            }
        }
        let Some(main) = main else { return error(0, "no main function") };
        // Every function is checked, called or not: each has its body lowered once, somewhere nothing runs it.
        let all: Vec<Arc<Func>> = self.functions.values().flatten().cloned().collect();
        let real_body = core::mem::take(&mut self.e.body);
        for f in all.iter().filter(|f| f.name != "main" && f.body.is_some()) {
            self.check_function(f)?;
        }
        self.e.body = real_body;
        let mut body = Vec::new();
        self.call_body(&mut body, &main, &[], None)?;
        // The outputs, as `main` left them.
        let outputs = core::mem::take(&mut self.outputs);
        for o in &outputs {
            let v = self.load(&mut body, &o.place)?;
            let mut leaves = Vec::new();
            flatten(v, &mut leaves);
            for (&io, v) in o.io.iter().zip(leaves) {
                body.push(Stmt::Output(io, v));
            }
        }
        let mut whole = core::mem::take(&mut self.prologue);
        whole.append(&mut self.e.body);
        whole.append(&mut body);
        self.e.body = whole;
        self.finish_defaults()?;
        Ok(())
    }

    /// A function's body lowered with made-up arguments, to report its errors; the IR is thrown away.
    fn check_function(&mut self, f: &Arc<Func>) -> Result<(), Error> {
        let mut scratch = Vec::new();
        let mut args = Vec::new();
        for (_, ty, _) in &f.params {
            let place = match ty {
                Ty::Sampler(_) => Place::Value(Val::Sampler(u32::MAX), ty.clone()),
                _ => self.alloc(ty)?,
            };
            args.push(Arg::Place(place));
        }
        let saved_outputs = self.outputs.len();
        self.call_body(&mut scratch, f, &args, None)?;
        self.outputs.truncate(saved_outputs);
        Ok(())
    }

    fn function(&mut self, f: &ast::Function) -> Result<Arc<Func>, Error> {
        let ret = self.resolve(&f.ret)?;
        let mut params = Vec::new();
        for p in &f.params {
            let ty = self.resolve(&p.ty)?;
            if ty == Ty::Void {
                return self.err("a void parameter");
            }
            let storage = match p.quals.storage {
                Storage::None | Storage::In | Storage::Const => Storage::In,
                Storage::Out => Storage::Out,
                Storage::InOut => Storage::InOut,
                other => return self.err(&format!("a parameter can't be {other:?}")),
            };
            params.push((storage, ty, p.name.clone()));
        }
        let func = Arc::new(Func { name: f.name.clone(), params, ret, body: f.body.clone().map(Arc::new), line: f.line });
        let overloads = self.functions.entry(f.name.clone()).or_default();
        if let Some(i) = overloads.iter().position(|g| g.params.iter().map(|p| &p.1).eq(func.params.iter().map(|p| &p.1))) {
            if overloads[i].ret != func.ret {
                return error(f.line, &format!("{} is declared again with another return type", f.name));
            }
            if overloads[i].body.is_some() && func.body.is_some() {
                return error(f.line, &format!("{} is defined twice", f.name));
            }
            if func.body.is_some() {
                overloads[i] = func.clone();
            }
            return Ok(overloads[i].clone());
        }
        overloads.push(func.clone());
        Ok(func)
    }

    pub fn functions(&self, name: &str) -> Vec<Arc<Func>> {
        self.functions.get(name).cloned().unwrap_or_default()
    }

    /// A call inlined: parameters as variables, the body in a loop run once, the result in `ret`.
    pub fn call_body(&mut self, out: &mut Block, f: &Arc<Func>, args: &[Arg], ret: Option<Place>) -> Result<(), Error> {
        let Some(body) = f.body.clone() else { return error(f.line, &format!("{} is declared but never defined", f.name)) };
        if self.depth > 64 || self.frames.len() > 32 {
            return self.err(&format!("{} calls itself, which GLSL doesn't allow", f.name));
        }
        // Function bodies see only globals: the scopes between are hidden while it's inlined.
        let saved_scopes = self.scopes.split_off(1);
        let saved_ctxs = core::mem::take(&mut self.ctxs);
        self.depth += 1;
        self.push_scope();
        let mut copy_back = Vec::new();
        let result = (|| -> Result<(), Error> {
            for ((storage, ty, name), arg) in f.params.iter().zip(args) {
                let place = match (ty, arg) {
                    (Ty::Sampler(_), Arg::Value(v)) => Place::Value(v.clone(), ty.clone()),
                    (Ty::Sampler(_), Arg::Place(p)) => p.clone(),
                    _ => {
                        let p = self.alloc(ty)?;
                        match (storage, arg) {
                            (Storage::Out, _) => {}
                            (_, Arg::Value(v)) => self.store(out, &p, v.clone())?,
                            (_, Arg::Place(src)) => {
                                let v = self.load(out, src)?;
                                self.store(out, &p, v)?;
                            }
                        }
                        if let (Storage::Out | Storage::InOut, Arg::Place(dst)) = (storage, arg) {
                            copy_back.push((p.clone(), dst.clone()));
                        }
                        p
                    }
                };
                if let Some(n) = name {
                    self.declare(n, Sym::Var { place, constant: None, readonly: false })?;
                }
            }
            let done = self.e.var(Type::BOOL);
            let no = self.bool_const(out, false);
            out.push(Stmt::Write(done, no));
            self.frames.push(Frame { ret: ret.clone(), done, base: self.ctxs.len() });
            self.ctxs.push(Ctx { kind: CtxKind::Function, returned: false });
            let mut inner = Vec::new();
            let r = self.stmts(&mut inner, &body);
            self.ctxs.pop();
            self.frames.pop();
            r?;
            inner.push(Stmt::Break);
            out.push(Stmt::Loop { body: inner, continuing: Vec::new(), break_if: None });
            for (from, to) in copy_back {
                let v = self.load(out, &from)?;
                self.store(out, &to, v)?;
            }
            Ok(())
        })();
        self.pop_scope();
        self.depth -= 1;
        self.ctxs = saved_ctxs;
        self.scopes.extend(saved_scopes);
        result
    }

    // ---- Globals ----

    fn defaults(&mut self, quals: &Qualifiers) -> Result<(), Error> {
        for (name, value) in &quals.layout {
            match (quals.storage, name.as_str()) {
                (Storage::In, "local_size_x" | "local_size_y" | "local_size_z") if self.stage == Stage::Compute => {
                    let n = match value {
                        Some(v) => self.const_int(v)?,
                        None => return self.err(&format!("{name} without a value")),
                    };
                    if n <= 0 {
                        return self.err(&format!("{name} of {n}"));
                    }
                    let axis = (name.as_bytes()[11] - b'x') as usize;
                    self.e.workgroup_size[axis] = n as u32;
                }
                (Storage::Uniform, "std140" | "shared" | "packed") => self.uniform_packing = Packing::Std140,
                (Storage::Uniform, "std430") => self.uniform_packing = Packing::Std430,
                (Storage::Buffer, "std140" | "shared" | "packed") => self.buffer_packing = Packing::Std140,
                (Storage::Buffer, "std430") => self.buffer_packing = Packing::Std430,
                (_, "row_major") => self.row_major = true,
                (_, "column_major") => self.row_major = false,
                (Storage::Out, "early_fragment_tests") | (Storage::In, "early_fragment_tests") => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn layout_int(&mut self, quals: &Qualifiers, name: &str) -> Result<Option<u32>, Error> {
        for (n, v) in &quals.layout {
            if n == name {
                let Some(v) = v else { return self.err(&format!("layout {name} without a value")) };
                let x = self.const_int(v)?;
                if x < 0 {
                    return self.err(&format!("layout {name} of {x}"));
                }
                return Ok(Some(x as u32));
            }
        }
        Ok(None)
    }

    fn global(&mut self, quals: &Qualifiers, spec: &TypeSpec, vars: &[Declarator]) -> Result<(), Error> {
        let base = self.resolve(spec)?;
        for d in vars {
            self.line = d.line;
            let mut ty = self.with_dims(base.clone(), &d.dims)?;
            if let (Ty::Array(e, None), Some(Init::Expr(init))) = (&ty, &d.init) {
                // `float a[] = float[](...)`: the size from the initializer.
                let n = match &init.kind {
                    ExprKind::Call(_, args) => args.len() as u32,
                    _ => 0,
                };
                ty = Ty::Array(e.clone(), Some(n));
            }
            match quals.storage {
                Storage::Uniform => {
                    if d.init.is_some() && self.version.es {
                        return self.err("a uniform's initializer, in GLSL ES");
                    }
                    let location = self.layout_int(quals, "location")?;
                    let binding = self.layout_int(quals, "binding")?;
                    self.uniform(&d.name, ty, location, binding)?;
                }
                Storage::In | Storage::Attribute | Storage::Varying if self.is_input(quals.storage) => {
                    let location = self.layout_int(quals, "location")?;
                    self.input(&d.name, &ty, quals, location)?;
                }
                Storage::Out | Storage::Varying => {
                    if self.stage == Stage::Compute {
                        return self.err("a compute kernel has no outputs");
                    }
                    let location = self.layout_int(quals, "location")?;
                    let place = self.output(&d.name, &ty, quals, location)?;
                    self.declare(&d.name, Sym::Var { place, constant: None, readonly: false })?;
                }
                Storage::Shared => {
                    if self.stage != Stage::Compute {
                        return self.err("shared variables are a compute kernel's");
                    }
                    let (size, align) = types::layout(&ty, Packing::Std430, false).map_err(|m| Error { message: m })?;
                    self.e.workgroup.push(Region { size: size.max(4), align: align.max(4) });
                    let n = self.e.workgroup.len() as u32 - 1;
                    let place = Place::Mem { base: Base_::Workgroup(n), offset: 0, dynamic: None, ty, packing: Packing::Std430, row_major: false };
                    self.declare(&d.name, Sym::Var { place, constant: None, readonly: false })?;
                }
                Storage::Buffer => return self.err("a buffer variable outside a block"),
                Storage::Const | Storage::None => {
                    let constant = quals.storage == Storage::Const;
                    if constant && d.init.is_none() {
                        return self.err(&format!("const {} without a value", d.name));
                    }
                    let mut body = core::mem::take(&mut self.e.body);
                    let r = self.local(&mut body, &d.name, ty, d.init.as_ref(), constant);
                    self.e.body = body;
                    r?;
                }
                other => return self.err(&format!("a global can't be {other:?} in a {:?} shader", self.stage)),
            }
        }
        if vars.is_empty() && quals.storage != Storage::None && !matches!(spec.base, TypeBase::Struct(_)) {
            return self.err("a declaration of no variable");
        }
        Ok(())
    }

    fn is_input(&self, s: Storage) -> bool {
        match s {
            Storage::In => true,
            Storage::Attribute => self.stage == Stage::Vertex,
            Storage::Varying => self.stage == Stage::Fragment,
            _ => false,
        }
    }

    /// A local (or plain global): its variables, and its initializer stored in them.
    pub fn local(&mut self, out: &mut Block, name: &str, ty: Ty, init: Option<&Init>, constant: bool) -> Result<(), Error> {
        let ty = match (&ty, init) {
            (Ty::Array(e, None), Some(Init::List(items))) => Ty::Array(e.clone(), Some(items.len() as u32)),
            (Ty::Array(e, None), Some(Init::Expr(ast::Expr { kind: ExprKind::Call(_, args), .. }))) => Ty::Array(e.clone(), Some(args.len() as u32)),
            _ => ty,
        };
        let value = match init {
            Some(i) => Some(self.initializer(out, i, &ty)?),
            None => None,
        };
        let konst = match (constant, init) {
            (true, Some(Init::Expr(e))) if ty.is_scalar_or_vector() && ty.lanes() == 1 => self.try_const(e),
            _ => None,
        };
        let place = if constant {
            let Some(v) = value else { return self.err(&format!("const {name} without a value")) };
            Place::Value(v, ty)
        } else {
            let p = self.alloc(&ty)?;
            if let Some(v) = value {
                self.store(out, &p, v)?;
            }
            p
        };
        self.declare(name, Sym::Var { place, constant: konst, readonly: constant })
    }

    fn initializer(&mut self, out: &mut Block, init: &Init, ty: &Ty) -> Result<Val, Error> {
        match init {
            Init::Expr(e) => {
                let (t, v) = self.rvalue(out, e)?;
                self.coerce(out, v, &t, ty)
            }
            Init::List(items) => {
                let parts: Vec<Ty> = match ty {
                    Ty::Array(e, Some(n)) => vec![(**e).clone(); *n as usize],
                    Ty::Struct(s) => s.members.iter().map(|m| m.1.clone()).collect(),
                    Ty::Matrix(c, _) => vec![ty.column().expect("matrix"); *c as usize],
                    Ty::Vector(b, n) => vec![Ty::Scalar(*b); *n as usize],
                    _ => return self.err(&format!("braces for {ty}")),
                };
                if parts.len() != items.len() {
                    return self.err(&format!("{} initializers for {ty}", items.len()));
                }
                let mut vals = Vec::new();
                for (item, t) in items.iter().zip(&parts) {
                    vals.push(self.initializer(out, item, t)?);
                }
                if let Ty::Vector(..) = ty {
                    let lanes = vals.iter().map(Val::v).collect();
                    return Ok(Val::V(self.let_(out, ty.ir().expect("vector"), Op::Construct(lanes))));
                }
                Ok(Val::Parts(vals))
            }
        }
    }

    // ---- Interface ----

    /// A plain uniform: a member of the default block (or a sampler, a binding of its own).
    fn uniform(&mut self, name: &str, ty: Ty, location: Option<u32>, binding: Option<u32>) -> Result<(), Error> {
        let place = self.uniform_place(name, ty, location, binding)?;
        self.declare(name, Sym::Var { place, constant: None, readonly: true })
    }

    /// A plain uniform's place: in the default block (or a sampler's binding), reflected for GL.
    fn uniform_place(&mut self, name: &str, ty: Ty, location: Option<u32>, binding: Option<u32>) -> Result<Place, Error> {
        if ty.has_sampler() {
            return self.sampler_place(name, &ty, location, binding);
        }
        if matches!(ty, Ty::Array(_, None)) {
            return self.err(&format!("uniform {name} without a size"));
        }
        let binding = match self.default_binding {
            Some(b) => b,
            None => {
                // Each stage's has a number of its own, so a program binds both at once.
                let number = if self.stage == Stage::Fragment { 1 } else { 0 };
                self.e.bindings.push(Binding { set: super::SET_DEFAULT, binding: number, kind: BindingKind::Uniform });
                let b = self.e.bindings.len() as u32 - 1;
                self.default_binding = Some(b);
                b
            }
        };
        // Members are laid out std430, one after another, in declaration order.
        let mut offset = 0u32;
        for (_, t, _) in &self.defaults {
            let (size, align) = types::layout(t, Packing::Std430, false).map_err(|m| Error { message: m })?;
            offset = offset.next_multiple_of(align) + size;
        }
        let (_, align) = types::layout(&ty, Packing::Std430, false).map_err(|m| Error { message: m })?;
        let offset = offset.next_multiple_of(align);
        self.defaults.push((name.to_string(), ty.clone(), location));
        let place = Place::Mem { base: Base_::Binding(binding), offset, dynamic: None, ty: ty.clone(), packing: Packing::Std430, row_major: false };
        self.reflect_uniform(name, &ty, offset, location)?;
        Ok(place)
    }

    /// The default block's uniforms as GL names them: structs' members and arrays of structs spelled out.
    fn reflect_uniform(&mut self, name: &str, ty: &Ty, offset: u32, location: Option<u32>) -> Result<(), Error> {
        let err = |m: String| Error { message: m };
        match ty {
            Ty::Struct(s) => {
                for (i, (m, t)) in s.members.iter().enumerate() {
                    let o = types::member_offset(s, i, Packing::Std430, false).map_err(err)?;
                    self.reflect_uniform(&format!("{name}.{m}"), t, offset + o, None)?;
                }
            }
            Ty::Array(e, Some(n)) if matches!(**e, Ty::Struct(_) | Ty::Array(..)) => {
                let stride = types::array_stride(e, Packing::Std430, false).map_err(err)?;
                for i in 0..*n {
                    self.reflect_uniform(&format!("{name}[{i}]"), e, offset + stride * i, None)?;
                }
            }
            _ => {
                let (elem, count, stride) = match ty {
                    Ty::Array(e, Some(n)) => ((**e).clone(), Some(*n), types::array_stride(e, Packing::Std430, false).map_err(err)?),
                    _ => (ty.clone(), None, 0),
                };
                let matrix_stride = match elem {
                    Ty::Matrix(_, r) => types::matrix_stride(r, Packing::Std430),
                    _ => 0,
                };
                self.iface.uniforms.push(Uniform { name: name.to_string(), ty: elem, count, offset, stride, matrix_stride, location, binding: None });
            }
        }
        Ok(())
    }

    /// A sampler uniform (or an array of them): a combined image and sampler binding each.
    fn sampler_place(&mut self, name: &str, ty: &Ty, location: Option<u32>, binding: Option<u32>) -> Result<Place, Error> {
        match ty {
            Ty::Sampler(s) => {
                if s.dim != types::Dim::D2 || s.shadow || s.array || s.base != Base::Float {
                    return self.err(&format!("{ty} isn't sampled here yet: only sampler2D is"));
                }
                let index = self.iface.uniforms.iter().filter(|u| u.binding.is_some()).count() as u32;
                self.e.bindings.push(Binding { set: super::SET_SAMPLER, binding: index, kind: BindingKind::Combined });
                let b = self.e.bindings.len() as u32 - 1;
                self.iface.uniforms.push(Uniform { name: name.to_string(), ty: ty.clone(), count: None, offset: 0, stride: 0, matrix_stride: 0, location, binding: Some((b, binding)) });
                Ok(Place::Value(Val::Sampler(b), ty.clone()))
            }
            Ty::Array(e, Some(n)) => {
                let mut parts = Vec::new();
                for i in 0..*n {
                    let p = self.sampler_place(&format!("{name}[{i}]"), e, location.map(|l| l + i), binding.map(|b| b + i))?;
                    let Place::Value(v, _) = p else { unreachable!() };
                    parts.push(v);
                }
                Ok(Place::Value(Val::Parts(parts), ty.clone()))
            }
            _ => self.err(&format!("{ty}: samplers in structs aren't here yet")),
        }
    }

    fn finish_defaults(&mut self) -> Result<(), Error> {
        let mut size = 0u32;
        for (_, t, _) in &self.defaults {
            let (s, a) = types::layout(t, Packing::Std430, false).map_err(|m| Error { message: m })?;
            size = size.next_multiple_of(a) + s;
        }
        self.iface.default_block = self.default_binding.map(|b| (b, size.next_multiple_of(16)));
        Ok(())
    }

    /// A uniform or buffer block (or an `in`/`out` block, which this compiler hasn't yet).
    fn block(&mut self, b: &ast::Block) -> Result<(), Error> {
        let buffer = match b.quals.storage {
            Storage::Uniform => false,
            Storage::Buffer => true,
            other => return self.err(&format!("{other:?} blocks aren't here yet")),
        };
        let mut packing = if buffer { self.buffer_packing } else { self.uniform_packing };
        let mut row_major = self.row_major;
        for (n, _) in &b.quals.layout {
            match n.as_str() {
                "std140" | "shared" | "packed" => packing = Packing::Std140,
                "std430" => packing = Packing::Std430,
                "row_major" => row_major = true,
                "column_major" => row_major = false,
                _ => {}
            }
        }
        let explicit = self.layout_int(&b.quals, "binding")?;
        let mut members = Vec::new();
        for m in &b.members {
            let base = self.resolve(&m.ty)?;
            for (name, dims) in &m.names {
                let ty = self.with_dims(base.clone(), dims)?;
                if ty.has_sampler() {
                    return self.err("a sampler in a block");
                }
                members.push((name.clone(), ty));
            }
        }
        if let Some(i) = members.iter().position(|m| matches!(m.1, Ty::Array(_, None)))
            && (!buffer || i + 1 != members.len())
        {
            return self.err("only a buffer block's last member can be an array without a size");
        }
        let block_ty = Ty::Struct(Arc::new(StructTy { name: b.name.clone(), members: members.clone() }));
        let instance_dims = b.instance.as_ref().map(|i| i.1.clone()).unwrap_or_default();
        if !instance_dims.is_empty() {
            return self.err("arrays of blocks aren't here yet");
        }
        let (size, _) = types::layout(&block_ty, packing, row_major).map_err(|m| Error { message: m })?;
        let set = if buffer { super::SET_BUFFER } else { super::SET_UNIFORM_BLOCK };
        let index = self.iface.blocks.iter().filter(|k| k.buffer == buffer).count() as u32;
        let readonly = b.members.iter().all(|m| m.quals.readonly) || b.quals.readonly;
        let kind = if buffer { BindingKind::Storage { writable: !readonly } } else { BindingKind::Uniform };
        self.e.bindings.push(Binding { set, binding: index, kind });
        let binding = self.e.bindings.len() as u32 - 1;
        let Ty::Struct(s) = &block_ty else { unreachable!() };
        let mut reflected = Vec::new();
        for i in 0..s.members.len() {
            let o = types::member_offset(s, i, packing, row_major).map_err(|m| Error { message: m })?;
            reflected.push((s.members[i].0.clone(), s.members[i].1.clone(), o));
        }
        self.iface.blocks.push(BlockInfo { name: b.name.clone(), buffer, binding, explicit, size, members: reflected });
        let place = Place::Mem { base: Base_::Binding(binding), offset: 0, dynamic: None, ty: block_ty.clone(), packing, row_major };
        let ro = !buffer || readonly;
        match &b.instance {
            Some((name, _)) => self.declare(name, Sym::Var { place, constant: None, readonly: ro })?,
            None => {
                for i in 0..s.members.len() {
                    let p = self.member(place.clone(), i)?;
                    self.declare(&s.members[i].0, Sym::Var { place: p, constant: None, readonly: ro })?;
                }
            }
        }
        Ok(())
    }

    fn interpolation(&self, quals: &Qualifiers, ty: &Ty) -> Result<Interpolation, Error> {
        let integer = matches!(ty.base(), Some(Base::Int | Base::Uint)) || matches!(ty, Ty::Array(e, _) if matches!(e.base(), Some(Base::Int | Base::Uint)));
        Ok(match quals.interp {
            Some(ast::Interp::Flat) => Interpolation::Flat,
            Some(ast::Interp::NoPerspective) => Interpolation::NoPerspective,
            _ if integer && (self.stage == Stage::Fragment) => return self.err("an integer input of a fragment shader must be flat"),
            _ if integer => Interpolation::Flat,
            _ => Interpolation::Smooth,
        })
    }

    /// The IR types a value of `ty` takes between stages, one for each location.
    fn io_types(&self, ty: &Ty) -> Result<Vec<Type>, Error> {
        Ok(match ty {
            Ty::Scalar(Base::Bool) | Ty::Vector(Base::Bool, _) => return self.err("booleans aren't passed between stages"),
            Ty::Scalar(_) | Ty::Vector(..) => vec![ty.ir().expect("scalar")],
            Ty::Matrix(c, r) => vec![Type::Vector(Scalar::F32, *r); *c as usize],
            Ty::Array(e, Some(n)) => {
                let one = self.io_types(e)?;
                (0..*n).flat_map(|_| one.clone()).collect()
            }
            Ty::Struct(s) => {
                let mut all = Vec::new();
                for (_, m) in &s.members {
                    all.extend(self.io_types(m)?);
                }
                all
            }
            _ => return self.err(&format!("{ty} isn't passed between stages")),
        })
    }

    fn input(&mut self, name: &str, ty: &Ty, quals: &Qualifiers, location: Option<u32>) -> Result<(), Error> {
        let place = self.input_place(name, ty, quals, location)?;
        self.declare(name, Sym::Var { place, constant: None, readonly: true })
    }

    /// An input's place: its values, read in the prologue.
    fn input_place(&mut self, name: &str, ty: &Ty, quals: &Qualifiers, location: Option<u32>) -> Result<Place, Error> {
        if self.stage == Stage::Compute {
            return self.err("a compute kernel has no inputs");
        }
        let interpolation = self.interpolation(quals, ty)?;
        let tys = self.io_types(ty)?;
        let first = self.e.inputs.len() as u32;
        // Locations are the interface's to settle; linking renumbers them.
        for (i, t) in tys.iter().enumerate() {
            let slot = Slot::Location(location.map_or(first + i as u32, |l| l + i as u32));
            self.e.inputs.push(Io { slot, ty: *t, interpolation });
        }
        let mut values = Vec::new();
        for i in 0..tys.len() as u32 {
            let t = tys[i as usize];
            let v = self.e.value(t);
            self.prologue.push(Stmt::Let(v, Op::Input(first + i)));
            values.push(v);
        }
        let val = rebuild(ty, &mut values.into_iter());
        self.iface.inputs.push(Variable { name: name.to_string(), ty: ty.clone(), location, io: (first..first + tys.len() as u32).collect() });
        Ok(Place::Value(val, ty.clone()))
    }

    fn output(&mut self, name: &str, ty: &Ty, quals: &Qualifiers, location: Option<u32>) -> Result<Place, Error> {
        let interpolation = self.interpolation(quals, ty).unwrap_or(Interpolation::Smooth);
        let tys = self.io_types(ty)?;
        let first = self.e.outputs.len() as u32;
        for (i, t) in tys.iter().enumerate() {
            let slot = Slot::Location(location.map_or(first + i as u32, |l| l + i as u32));
            self.e.outputs.push(Io { slot, ty: *t, interpolation });
        }
        let place = self.alloc(ty)?;
        let io: Vec<u32> = (first..first + tys.len() as u32).collect();
        self.iface.outputs.push(Variable { name: name.to_string(), ty: ty.clone(), location, io: io.clone() });
        self.outputs.push(Out { place: place.clone(), io });
        Ok(place)
    }

    /// GLSL's built-in variables, made the first time they're named.
    fn builtin_var(&mut self, name: &str) -> Result<Option<Sym>, Error> {
        let vec4 = Ty::Vector(Base::Float, 4);
        let uvec3 = Ty::Vector(Base::Uint, 3);
        let sym = match (self.stage, name) {
            (Stage::Vertex, "gl_Position") => {
                self.e.outputs.push(Io { slot: Slot::Position, ty: Type::Vector(Scalar::F32, 4), interpolation: Interpolation::Smooth });
                let io = self.e.outputs.len() as u32 - 1;
                let place = self.alloc(&vec4)?;
                // Defined even if the shader never writes it.
                let mut init = Vec::new();
                let zero = self.number(&mut init, &vec4, 0.0);
                if let Place::Reg(v, _) = &place {
                    init.push(Stmt::Write(*v, zero));
                }
                self.prologue.extend(init);
                self.outputs.push(Out { place: place.clone(), io: vec![io] });
                Sym::Var { place, constant: None, readonly: false }
            }
            (Stage::Vertex, "gl_PointSize") => Sym::Var { place: self.alloc(&types::FLOAT)?, constant: None, readonly: false },
            (Stage::Vertex, "gl_VertexID" | "gl_InstanceID" | "gl_VertexIndex" | "gl_InstanceIndex") => {
                let b = if name.contains("Vertex") { Builtin::VertexIndex } else { Builtin::InstanceIndex };
                let mut p = Vec::new();
                let raw = self.let_(&mut p, Type::U32, Op::Builtin(b));
                let v = self.let_(&mut p, Type::I32, Op::Bitcast(raw, Type::I32));
                self.prologue.extend(p);
                Sym::Var { place: Place::Value(Val::V(v), types::INT), constant: None, readonly: true }
            }
            (Stage::Fragment, "gl_FragCoord") => {
                let v = self.e.value(Type::Vector(Scalar::F32, 4));
                self.prologue.push(Stmt::Let(v, Op::Builtin(Builtin::FragCoord)));
                Sym::Var { place: Place::Value(Val::V(v), vec4), constant: None, readonly: true }
            }
            (Stage::Fragment, "gl_FrontFacing") => {
                let v = self.e.value(Type::BOOL);
                self.prologue.push(Stmt::Let(v, Op::Builtin(Builtin::FrontFacing)));
                Sym::Var { place: Place::Value(Val::V(v), types::BOOL), constant: None, readonly: true }
            }
            (Stage::Fragment, "gl_FragColor" | "gl_FragData") => {
                if self.frag_outputs || !self.iface.outputs.is_empty() {
                    return self.err("gl_FragColor, gl_FragData and out variables can't be mixed");
                }
                self.frag_outputs = true;
                let ty = if name == "gl_FragColor" { vec4.clone() } else { Ty::Array(Box::new(vec4.clone()), Some(DRAW_BUFFERS)) };
                let place = self.output(name, &ty, &Qualifiers::none(), Some(0))?;
                // Black until written.
                let mut init = Vec::new();
                let zero = self.number(&mut init, &vec4, 0.0);
                self.store(&mut init, &place, if name == "gl_FragColor" { Val::V(zero) } else { Val::Parts(vec![Val::V(zero); DRAW_BUFFERS as usize]) })?;
                self.prologue.extend(init);
                // Declared so the next use finds it.
                self.scopes[0].insert(name.to_string(), Sym::Var { place: place.clone(), constant: None, readonly: false });
                return Ok(Some(Sym::Var { place, constant: None, readonly: false }));
            }
            (Stage::Fragment, "gl_FragDepth") => {
                self.e.outputs.push(Io { slot: Slot::FragDepth, ty: Type::F32, interpolation: Interpolation::Smooth });
                let io = self.e.outputs.len() as u32 - 1;
                let place = self.alloc(&types::FLOAT)?;
                // Until written, the fragment's own depth.
                let mut p = Vec::new();
                let coord = self.let_(&mut p, Type::Vector(Scalar::F32, 4), Op::Builtin(Builtin::FragCoord));
                let z = self.let_(&mut p, Type::F32, Op::Extract(coord, 2));
                if let Place::Reg(v, _) = &place {
                    p.push(Stmt::Write(*v, z));
                }
                self.prologue.extend(p);
                self.outputs.push(Out { place: place.clone(), io: vec![io] });
                Sym::Var { place, constant: None, readonly: false }
            }
            (Stage::Compute, "gl_GlobalInvocationID" | "gl_LocalInvocationID" | "gl_WorkGroupID" | "gl_NumWorkGroups" | "gl_LocalInvocationIndex") => {
                let (b, ty) = match name {
                    "gl_GlobalInvocationID" => (Builtin::GlobalId, uvec3.clone()),
                    "gl_LocalInvocationID" => (Builtin::LocalId, uvec3.clone()),
                    "gl_WorkGroupID" => (Builtin::GroupId, uvec3.clone()),
                    "gl_NumWorkGroups" => (Builtin::GroupCount, uvec3.clone()),
                    _ => (Builtin::LocalIndex, types::UINT),
                };
                let v = self.e.value(ty.ir().expect("vector"));
                self.prologue.push(Stmt::Let(v, Op::Builtin(b)));
                Sym::Var { place: Place::Value(Val::V(v), ty), constant: None, readonly: true }
            }
            (Stage::Compute, "gl_WorkGroupSize") => {
                let mut p = Vec::new();
                let size = self.e.workgroup_size;
                let lanes = size.iter().map(|&n| self.uint_const(&mut p, n)).collect();
                let v = self.let_(&mut p, Type::UVEC3, Op::Construct(lanes));
                self.prologue.extend(p);
                Sym::Var { place: Place::Value(Val::V(v), uvec3), constant: None, readonly: true }
            }
            _ if self.compat && let Some(sym) = self.compat_var(name)? => sym,
            (_, "gl_MaxDrawBuffers") => Sym::Var { place: Place::Value(Val::Void, types::INT), constant: Some(Konst::Int(i64::from(DRAW_BUFFERS))), readonly: true },
            _ => return self.err(&format!("{name} isn't here in a {:?} shader yet", self.stage)),
        };
        self.scopes[0].insert(name.to_string(), sym.clone());
        Ok(Some(sym))
    }

    // ---- Statements ----

    pub fn stmts(&mut self, out: &mut Block, stmts: &[ast::Stmt]) -> Result<(), Error> {
        for s in stmts {
            self.stmt(out, s)?;
        }
        Ok(())
    }

    fn scoped(&mut self, out: &mut Block, s: &ast::Stmt) -> Result<(), Error> {
        self.push_scope();
        let r = self.stmt(out, s);
        self.pop_scope();
        r
    }

    pub fn stmt(&mut self, out: &mut Block, s: &ast::Stmt) -> Result<(), Error> {
        self.line = s.line;
        match &s.kind {
            StmtKind::Empty => {}
            StmtKind::Block(body) => {
                self.push_scope();
                let r = self.stmts(out, body);
                self.pop_scope();
                r?;
            }
            StmtKind::Decl { quals, ty, vars } => {
                if !matches!(quals.storage, Storage::None | Storage::Const) {
                    return self.err(&format!("a local can't be {:?}", quals.storage));
                }
                let base = self.resolve(ty)?;
                for d in vars {
                    self.line = d.line;
                    let ty = self.with_dims(base.clone(), &d.dims)?;
                    self.local(out, &d.name, ty, d.init.as_ref(), quals.storage == Storage::Const)?;
                }
            }
            StmtKind::Expr(e) => {
                self.effect(out, e)?;
            }
            StmtKind::If(cond, then, otherwise) => {
                let c = self.condition_value(out, cond)?;
                let mut t = Vec::new();
                self.scoped(&mut t, then)?;
                let mut f = Vec::new();
                if let Some(o) = otherwise {
                    self.scoped(&mut f, o)?;
                }
                out.push(Stmt::If { cond: c, then: t, otherwise: f });
            }
            StmtKind::While(cond, body) => {
                self.push_scope();
                let r = (|| {
                    let mut inner = Vec::new();
                    self.loop_condition(&mut inner, cond)?;
                    self.enter(CtxKind::Loop);
                    let r = self.scoped(&mut inner, body);
                    let ctx = self.ctxs.pop().expect("the loop");
                    r?;
                    out.push(Stmt::Loop { body: inner, continuing: Vec::new(), break_if: None });
                    self.after(out, ctx)
                })();
                self.pop_scope();
                r?;
            }
            StmtKind::DoWhile(body, cond) => {
                let mut inner = Vec::new();
                self.enter(CtxKind::Loop);
                let r = self.scoped(&mut inner, body);
                let ctx = self.ctxs.pop().expect("the loop");
                r?;
                let mut continuing = Vec::new();
                let c = self.condition_value(&mut continuing, cond)?;
                let stop = self.let_(&mut continuing, Type::BOOL, Op::Unary(gpu_ir::UnOp::Not, c));
                out.push(Stmt::Loop { body: inner, continuing, break_if: Some(stop) });
                self.after(out, ctx)?;
            }
            StmtKind::For { init, cond, step, body } => {
                self.push_scope();
                let r = (|| {
                    if let Some(i) = init {
                        self.stmt(out, i)?;
                    }
                    let mut inner = Vec::new();
                    if let Some(c) = cond {
                        self.loop_condition(&mut inner, c)?;
                    }
                    self.enter(CtxKind::Loop);
                    let r = self.scoped(&mut inner, body);
                    let ctx = self.ctxs.pop().expect("the loop");
                    r?;
                    let mut continuing = Vec::new();
                    if let Some(st) = step {
                        self.effect(&mut continuing, st)?;
                    }
                    out.push(Stmt::Loop { body: inner, continuing, break_if: None });
                    self.after(out, ctx)
                })();
                self.pop_scope();
                r?;
            }
            StmtKind::Switch(selector, body) => self.switch(out, selector, body)?,
            StmtKind::Case(_) | StmtKind::Default => return self.err("a case label outside a switch"),
            StmtKind::Break => {
                if !self.ctxs.iter().rev().take_while(|c| !matches!(c.kind, CtxKind::Function)).any(|c| matches!(c.kind, CtxKind::Loop | CtxKind::Switch { .. })) {
                    return self.err("a break outside a loop or switch");
                }
                out.push(Stmt::Break);
            }
            StmtKind::Continue => {
                if !self.ctxs.iter().rev().take_while(|c| !matches!(c.kind, CtxKind::Function)).any(|c| matches!(c.kind, CtxKind::Loop)) {
                    return self.err("a continue outside a loop");
                }
                self.continue_(out);
            }
            StmtKind::Discard => {
                if self.stage != Stage::Fragment {
                    return self.err("discard outside a fragment shader");
                }
                out.push(Stmt::Discard);
            }
            StmtKind::Return(e) => {
                let frame = self.frames.last().ok_or_else(|| Error { message: format!("0:{}: error: a return outside a function", self.line) })?;
                let (ret, done, base) = (frame.ret.clone(), frame.done, frame.base);
                match (e, &ret) {
                    (Some(e), Some(place)) => {
                        let (t, v) = self.rvalue(out, e)?;
                        let want = place.ty().clone();
                        let v = self.coerce(out, v, &t, &want)?;
                        self.store(out, place, v)?;
                    }
                    (Some(e), None) => {
                        // Lowered for its errors, in a function that's only being checked.
                        let (t, _) = self.rvalue(out, e)?;
                        if t == Ty::Void {
                            return self.err("a void value returned");
                        }
                    }
                    (None, Some(_)) => return self.err("a return without a value, in a function that returns one"),
                    (None, None) => {}
                }
                let yes = self.bool_const(out, true);
                out.push(Stmt::Write(done, yes));
                for c in &mut self.ctxs[base..] {
                    c.returned = true;
                }
                out.push(Stmt::Break);
            }
        }
        Ok(())
    }

    fn enter(&mut self, kind: CtxKind) {
        self.ctxs.push(Ctx { kind, returned: false });
    }

    /// After a loop or switch: a `return` inside it breaks out of what holds it too.
    fn after(&mut self, out: &mut Block, ctx: Ctx) -> Result<(), Error> {
        if ctx.returned {
            let done = self.frames.last().expect("a frame").done;
            let d = self.let_(out, Type::BOOL, Op::Read(done));
            out.push(Stmt::If { cond: d, then: vec![Stmt::Break], otherwise: Vec::new() });
        }
        if let CtxKind::Switch { cont, used: true } = ctx.kind {
            let c = self.let_(out, Type::BOOL, Op::Read(cont));
            let mut then = Vec::new();
            self.continue_(&mut then);
            out.push(Stmt::If { cond: c, then, otherwise: Vec::new() });
        }
        Ok(())
    }

    /// `continue`: the loop's, through any `switch` between (each left with its flag set).
    fn continue_(&mut self, out: &mut Block) {
        if let Some(Ctx { kind: CtxKind::Switch { cont, used }, .. }) = self.ctxs.last_mut() {
            *used = true;
            let cont = *cont;
            let yes = self.bool_const(out, true);
            out.push(Stmt::Write(cont, yes));
            out.push(Stmt::Break);
        } else {
            out.push(Stmt::Continue);
        }
    }

    /// A loop's condition at the top of its body: false breaks.
    fn loop_condition(&mut self, out: &mut Block, cond: &Cond) -> Result<(), Error> {
        let c = match cond {
            Cond::Expr(e) => self.condition_value(out, e)?,
            Cond::Decl { ty, name, init } => {
                let ty = self.resolve(ty)?;
                self.local(out, name, ty, Some(&Init::Expr(init.clone())), false)?;
                let Some(Sym::Var { place, .. }) = self.lookup(name)? else { unreachable!() };
                self.load(out, &place)?.v()
            }
        };
        let stop = self.let_(out, Type::BOOL, Op::Unary(gpu_ir::UnOp::Not, c));
        out.push(Stmt::If { cond: stop, then: vec![Stmt::Break], otherwise: Vec::new() });
        Ok(())
    }

    /// A `bool` condition.
    pub fn condition_value(&mut self, out: &mut Block, e: &ast::Expr) -> Result<Value, Error> {
        let (t, v) = self.rvalue(out, e)?;
        if t != types::BOOL {
            return self.err(&format!("a condition of {t}, not bool"));
        }
        Ok(v.v())
    }

    /// `switch`: a loop run once, each label's statements guarded by whether it or one before matched.
    fn switch(&mut self, out: &mut Block, selector: &ast::Expr, body: &[ast::Stmt]) -> Result<(), Error> {
        let (t, v) = self.rvalue(out, selector)?;
        if !matches!(t, Ty::Scalar(Base::Int | Base::Uint)) {
            return self.err(&format!("a switch on {t}"));
        }
        let s = v.v();
        // The statements under each label (several labels may share them).
        let mut groups: Vec<(Vec<Option<i64>>, Vec<ast::Stmt>)> = Vec::new();
        for st in body {
            match &st.kind {
                StmtKind::Case(e) => {
                    let k = self.const_int(e)?;
                    if groups.last().is_none_or(|g| !g.1.is_empty()) {
                        groups.push((Vec::new(), Vec::new()));
                    }
                    groups.last_mut().expect("a group").0.push(Some(k));
                }
                StmtKind::Default => {
                    if groups.last().is_none_or(|g| !g.1.is_empty()) {
                        groups.push((Vec::new(), Vec::new()));
                    }
                    groups.last_mut().expect("a group").0.push(None);
                }
                _ => match groups.last_mut() {
                    Some(g) => g.1.push(st.clone()),
                    None => return self.err("a statement before a switch's first label"),
                },
            }
        }
        let all: Vec<i64> = groups.iter().flat_map(|g| g.0.iter().flatten().copied()).collect();
        if (1..all.len()).any(|i| all[..i].contains(&all[i])) {
            return self.err("a case label twice");
        }
        if groups.iter().flat_map(|g| &g.0).filter(|l| l.is_none()).count() > 1 {
            return self.err("two default labels");
        }
        let cont = self.e.var(Type::BOOL);
        let no = self.bool_const(out, false);
        out.push(Stmt::Write(cont, no));
        let mut inner = Vec::new();
        let ir = t.ir().expect("scalar");
        let is = |me: &mut Self, out: &mut Block, k: i64| {
            let kk = me.konst(out, ir.scalar().expect("integer"), k as u32 as u64);
            me.let_(out, Type::BOOL, Op::Compare(Cmp::Eq, s, kk))
        };
        // Whether any case matches, for `default`.
        let mut any = self.bool_const(&mut inner, false);
        for &k in &all {
            let m = is(self, &mut inner, k);
            any = self.let_(&mut inner, Type::BOOL, Op::Binary(gpu_ir::BinOp::Or, any, m));
        }
        let none = self.let_(&mut inner, Type::BOOL, Op::Unary(gpu_ir::UnOp::Not, any));
        let fall = self.e.var(Type::BOOL);
        let no = self.bool_const(&mut inner, false);
        inner.push(Stmt::Write(fall, no));
        self.enter(CtxKind::Switch { cont, used: false });
        self.push_scope();
        let r = (|| -> Result<(), Error> {
            for (labels, stmts) in &groups {
                let mut c = self.let_(&mut inner, Type::BOOL, Op::Read(fall));
                for l in labels {
                    let m = match l {
                        Some(k) => is(self, &mut inner, *k),
                        None => none,
                    };
                    c = self.let_(&mut inner, Type::BOOL, Op::Binary(gpu_ir::BinOp::Or, c, m));
                }
                let mut then = Vec::new();
                let yes = self.bool_const(&mut then, true);
                then.push(Stmt::Write(fall, yes));
                self.stmts(&mut then, stmts)?;
                inner.push(Stmt::If { cond: c, then, otherwise: Vec::new() });
            }
            Ok(())
        })();
        self.pop_scope();
        let ctx = self.ctxs.pop().expect("the switch");
        r?;
        inner.push(Stmt::Break);
        out.push(Stmt::Loop { body: inner, continuing: Vec::new(), break_if: None });
        self.after(out, ctx)
    }

    // ---- Constants ----

    pub fn const_int(&mut self, e: &ast::Expr) -> Result<i64, Error> {
        match self.const_eval(e)? {
            Konst::Int(i) => Ok(i),
            _ => self.err("an integer constant expected"),
        }
    }

    /// A constant's value, if the expression is one this evaluator knows.
    pub fn try_const(&mut self, e: &ast::Expr) -> Option<Konst> {
        self.const_eval(e).ok()
    }

    fn const_eval(&mut self, e: &ast::Expr) -> Result<Konst, Error> {
        use ast::{Binary as B, Unary as U};
        let line = e.line;
        let bad = || error(line, "not a constant expression");
        Ok(match &e.kind {
            ExprKind::Int(v, u) => Konst::Int(if *u { i64::from(*v) } else { i64::from(*v as i32) }),
            ExprKind::Float(f) => Konst::Float(*f),
            ExprKind::Bool(b) => Konst::Bool(*b),
            ExprKind::Ident(n) => match self.lookup(n)? {
                Some(Sym::Var { constant: Some(k), .. }) => k,
                _ => return bad(),
            },
            ExprKind::Unary(op, x) => match (op, self.const_eval(x)?) {
                (U::Neg, Konst::Int(i)) => Konst::Int(i.wrapping_neg()),
                (U::Neg, Konst::Float(f)) => Konst::Float(-f),
                (U::Plus, k) => k,
                (U::Not, Konst::Bool(b)) => Konst::Bool(!b),
                (U::BitNot, Konst::Int(i)) => Konst::Int(!i),
                _ => return bad(),
            },
            ExprKind::Binary(op, a, b) => {
                let (a, b) = (self.const_eval(a)?, self.const_eval(b)?);
                match (a, b) {
                    (Konst::Int(a), Konst::Int(b)) => match op {
                        B::Add => Konst::Int(a.wrapping_add(b)),
                        B::Sub => Konst::Int(a.wrapping_sub(b)),
                        B::Mul => Konst::Int(a.wrapping_mul(b)),
                        B::Div if b != 0 => Konst::Int(a / b),
                        B::Mod if b != 0 => Konst::Int(a % b),
                        B::Shl => Konst::Int(a << (b & 31)),
                        B::Shr => Konst::Int(a >> (b & 31)),
                        B::BitAnd => Konst::Int(a & b),
                        B::BitOr => Konst::Int(a | b),
                        B::BitXor => Konst::Int(a ^ b),
                        B::Lt => Konst::Bool(a < b),
                        B::Gt => Konst::Bool(a > b),
                        B::Le => Konst::Bool(a <= b),
                        B::Ge => Konst::Bool(a >= b),
                        B::Eq => Konst::Bool(a == b),
                        B::Ne => Konst::Bool(a != b),
                        _ => return bad(),
                    },
                    (Konst::Float(a), Konst::Float(b)) => match op {
                        B::Add => Konst::Float(a + b),
                        B::Sub => Konst::Float(a - b),
                        B::Mul => Konst::Float(a * b),
                        B::Div => Konst::Float(a / b),
                        B::Lt => Konst::Bool(a < b),
                        B::Gt => Konst::Bool(a > b),
                        _ => return bad(),
                    },
                    (Konst::Bool(a), Konst::Bool(b)) => match op {
                        B::And => Konst::Bool(a && b),
                        B::Or => Konst::Bool(a || b),
                        B::Xor => Konst::Bool(a ^ b),
                        B::Eq => Konst::Bool(a == b),
                        B::Ne => Konst::Bool(a != b),
                        _ => return bad(),
                    },
                    _ => return bad(),
                }
            }
            ExprKind::Ternary(c, a, b) => match self.const_eval(c)? {
                Konst::Bool(true) => self.const_eval(a)?,
                Konst::Bool(false) => self.const_eval(b)?,
                _ => return bad(),
            },
            ExprKind::Call(ast::Callee::Type(t), args) if args.len() == 1 && t.dims.is_empty() => {
                let k = self.const_eval(&args[0])?;
                let TypeBase::Named(n) = &t.base else { return bad() };
                match (n.as_str(), k) {
                    ("int" | "uint", Konst::Int(i)) => Konst::Int(i),
                    ("int" | "uint", Konst::Float(f)) => Konst::Int(f as i64),
                    ("int" | "uint", Konst::Bool(b)) => Konst::Int(i64::from(b)),
                    ("float", Konst::Int(i)) => Konst::Float(i as f64),
                    ("float", Konst::Float(f)) => Konst::Float(f),
                    ("bool", Konst::Int(i)) => Konst::Bool(i != 0),
                    _ => return bad(),
                }
            }
            ExprKind::Length(x) => {
                let ExprKind::Ident(n) = &x.kind else { return bad() };
                match self.lookup(n)? {
                    Some(Sym::Var { place, .. }) => match place.ty() {
                        Ty::Array(_, Some(n)) => Konst::Int(i64::from(*n)),
                        _ => return bad(),
                    },
                    _ => return bad(),
                }
            }
            _ => return bad(),
        })
    }
}

/// A call's argument: a value, or a place an `out` or `inout` parameter writes back to.
#[derive(Debug, Clone)]
pub(super) enum Arg {
    Value(Val),
    Place(Place),
}

pub(super) fn scalar_of(t: Type) -> Type {
    match t {
        Type::Vector(s, _) => Type::Scalar(s),
        other => other,
    }
}

/// What indexing a type gives.
pub(super) fn element_type(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::Array(e, _) => Some((**e).clone()),
        Ty::Matrix(..) => ty.column(),
        Ty::Vector(b, _) => Some(Ty::Scalar(*b)),
        _ => None,
    }
}

/// A composite's parts' types.
pub(super) fn part_types(ty: &Ty) -> Vec<Ty> {
    match ty {
        Ty::Array(e, Some(n)) => vec![(**e).clone(); *n as usize],
        Ty::Matrix(c, _) => vec![ty.column().expect("matrix"); *c as usize],
        Ty::Struct(s) => s.members.iter().map(|m| m.1.clone()).collect(),
        _ => Vec::new(),
    }
}

/// A value's scalars and vectors, in order.
pub(super) fn flatten(v: Val, out: &mut Vec<Value>) {
    match v {
        Val::V(x) => out.push(x),
        Val::Parts(p) => p.into_iter().for_each(|x| flatten(x, out)),
        _ => {}
    }
}

/// A value of a type from its scalars and vectors, in order.
fn rebuild(ty: &Ty, values: &mut impl Iterator<Item = Value>) -> Val {
    match ty {
        Ty::Scalar(_) | Ty::Vector(..) => Val::V(values.next().expect("enough")),
        _ => Val::Parts(part_types(ty).iter().map(|t| rebuild(t, values)).collect()),
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}{}", self.number, if self.es { " es" } else { "" })
    }
}

/// The compatibility profile's attributes: each at the generic attribute it aliases, as NVIDIA's drivers alias them
/// (GL's conventional arrays and current values are those attributes').
pub const COMPAT_ATTRIBS: &[(&str, u32)] = &[
    ("gl_Vertex", 0),
    ("gl_Normal", 2),
    ("gl_Color", 3),
    ("gl_SecondaryColor", 4),
    ("gl_FogCoord", 5),
    ("gl_MultiTexCoord0", 8),
    ("gl_MultiTexCoord1", 9),
    ("gl_MultiTexCoord2", 10),
    ("gl_MultiTexCoord3", 11),
    ("gl_MultiTexCoord4", 12),
    ("gl_MultiTexCoord5", 13),
    ("gl_MultiTexCoord6", 14),
    ("gl_MultiTexCoord7", 15),
];

/// Texture coordinate sets the compatibility profile has (`gl_MaxTextureCoords`).
pub const TEXTURE_COORDS: u32 = 8;

impl Lower {
    /// The compatibility profile's built-in variables: attributes, the state's matrices (uniforms GL fills), and the
    /// colours, texture coordinates and fog coordinate passed from vertex to fragment shader.
    fn compat_var(&mut self, name: &str) -> Result<Option<Sym>, Error> {
        let vec4 = Ty::Vector(Base::Float, 4);
        let mat4 = Ty::Matrix(4, 4);
        let coords = Ty::Array(Box::new(vec4.clone()), Some(TEXTURE_COORDS));
        let matrices = Ty::Array(Box::new(mat4.clone()), Some(TEXTURE_COORDS));
        let none = Qualifiers::none();
        // The matrices, and what's made of them.
        let uniform = match name {
            "gl_NormalMatrix" => Some(Ty::Matrix(3, 3)),
            "gl_NormalScale" => Some(types::FLOAT),
            n if n.starts_with("gl_TextureMatrix") => Some(matrices.clone()),
            n if n.starts_with("gl_ModelViewMatrix") || n.starts_with("gl_ProjectionMatrix") || n.starts_with("gl_ModelViewProjectionMatrix") => {
                let rest = n.trim_start_matches("gl_ModelViewProjectionMatrix").trim_start_matches("gl_ModelViewMatrix").trim_start_matches("gl_ProjectionMatrix");
                matches!(rest, "" | "Inverse" | "Transpose" | "InverseTranspose").then(|| mat4.clone())
            }
            _ => None,
        };
        if let Some(ty) = uniform {
            let place = self.uniform_place(name, ty, None, None)?;
            return Ok(Some(Sym::Var { place, constant: None, readonly: true }));
        }
        let sym = match (self.stage, name) {
            (Stage::Vertex, _) if COMPAT_ATTRIBS.iter().any(|a| a.0 == name) => {
                let location = COMPAT_ATTRIBS.iter().find(|a| a.0 == name).map(|a| a.1);
                let ty = match name {
                    "gl_Normal" => Ty::Vector(Base::Float, 3),
                    "gl_FogCoord" => types::FLOAT,
                    _ => vec4.clone(),
                };
                let place = self.input_place(name, &ty, &none, location)?;
                Sym::Var { place, constant: None, readonly: true }
            }
            (Stage::Vertex, "gl_FrontColor" | "gl_BackColor" | "gl_FrontSecondaryColor" | "gl_BackSecondaryColor" | "gl_TexCoord" | "gl_FogFragCoord") => {
                let ty = match name {
                    "gl_TexCoord" => coords.clone(),
                    "gl_FogFragCoord" => types::FLOAT,
                    _ => vec4.clone(),
                };
                let place = self.output(name, &ty, &none, None)?;
                // Defined even if the shader never writes it: a colour white, coordinates (0, 0, 0, 1).
                let mut init = Vec::new();
                let v = match &ty {
                    Ty::Array(..) => {
                        let one = self.number(&mut init, &types::FLOAT, 1.0);
                        let zero = self.number(&mut init, &types::FLOAT, 0.0);
                        let c = self.let_(&mut init, Type::Vector(Scalar::F32, 4), Op::Construct(vec![zero, zero, zero, one]));
                        Val::Parts(vec![Val::V(c); TEXTURE_COORDS as usize])
                    }
                    t => Val::V(self.number(&mut init, t, if name.ends_with("Color") { 1.0 } else { 0.0 })),
                };
                self.store(&mut init, &place, v)?;
                self.prologue.extend(init);
                Sym::Var { place, constant: None, readonly: false }
            }
            (Stage::Vertex, "gl_ClipVertex") => Sym::Var { place: self.alloc(&vec4)?, constant: None, readonly: false },
            (Stage::Fragment, "gl_Color" | "gl_SecondaryColor" | "gl_TexCoord" | "gl_FogFragCoord") => {
                let ty = match name {
                    "gl_TexCoord" => coords,
                    "gl_FogFragCoord" => types::FLOAT,
                    _ => vec4,
                };
                let place = self.input_place(name, &ty, &none, None)?;
                Sym::Var { place, constant: None, readonly: true }
            }
            (_, "gl_MaxTextureCoords" | "gl_MaxTextureUnits") => {
                Sym::Var { place: Place::Value(Val::Void, types::INT), constant: Some(Konst::Int(i64::from(TEXTURE_COORDS))), readonly: true }
            }
            _ => return Ok(None),
        };
        Ok(Some(sym))
    }
}
