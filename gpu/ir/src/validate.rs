//! The validator: every program a frontend makes is checked before a backend sees it, so a backend can trust its
//! input and a frontend's mistake is reported where it happened, not as a wrong pixel.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::{BinOp, BindingKind, Block, Cmp, Entry, Lod, Op, Scalar, Slot, Space, Stage, Stmt, SubgroupOp, Type, UnOp, Value};

/// What's wrong with a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn err<T>(message: String) -> Result<T, Error> {
    Err(Error { message })
}

/// Checks an entry point: every value defined once and before its uses, every operand of the right type, every
/// `Break` and `Continue` inside a loop, every index in range, and each stage's inputs, outputs, builtins and
/// operations its own.
pub fn validate(entry: &Entry) -> Result<(), Error> {
    let named = |e: Error| Error { message: format!("{}: {}", entry.name, e.message) };
    stage_io(entry).map_err(named)?;
    let mut v = Validator { entry, defined: vec![false; entry.values.len()] };
    v.block(&entry.body, Loops::None).map_err(named)
}

/// What an entry point declares for its stage.
fn stage_io(entry: &Entry) -> Result<(), Error> {
    if entry.workgroup_size.contains(&0) {
        return err(format!("a workgroup size of {:?}", entry.workgroup_size));
    }
    if entry.stage == Stage::Compute && !(entry.inputs.is_empty() && entry.outputs.is_empty()) {
        return err("a compute kernel with stage inputs or outputs".into());
    }
    for (i, io) in entry.inputs.iter().enumerate() {
        if !matches!(io.slot, Slot::Location(_)) {
            return err(format!("input {i} is {:?}: inputs are locations", io.slot));
        }
        data_type(io.ty).map_err(|e| Error { message: format!("input {i}: {}", e.message) })?;
    }
    for (i, io) in entry.outputs.iter().enumerate() {
        let ok = match io.slot {
            Slot::Location(_) => data_type(io.ty).is_ok(),
            Slot::Position => entry.stage == Stage::Vertex && io.ty == Type::Vector(Scalar::F32, 4),
            Slot::FragDepth => entry.stage == Stage::Fragment && io.ty == Type::F32,
        };
        if !ok {
            return err(format!("output {i}: {:?} of {:?} in a {:?} shader", io.slot, io.ty, entry.stage));
        }
        if entry.outputs[..i].iter().any(|o| o.slot == io.slot) {
            return err(format!("output {i}: {:?} twice", io.slot));
        }
    }
    Ok(())
}

/// A type a stage input or output can have: numbers, scalars or vectors.
fn data_type(ty: Type) -> Result<(), Error> {
    match ty.scalar() {
        Some(Scalar::Bool) | None => err(format!("{ty:?} isn't passed between stages")),
        Some(_) => Ok(()),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Loops {
    None,
    Body,
    Continuing,
}

struct Validator<'a> {
    entry: &'a Entry,
    defined: Vec<bool>,
}

impl Validator<'_> {
    fn ty(&self, value: Value) -> Result<Type, Error> {
        match self.defined.get(value.0 as usize) {
            Some(true) => Ok(self.entry.values[value.0 as usize]),
            Some(false) => err(format!("{value} is used before it's defined")),
            None => err(format!("{value} doesn't exist")),
        }
    }

    fn data(&self, value: Value) -> Result<(Scalar, Type), Error> {
        let ty = self.ty(value)?;
        match ty.scalar() {
            Some(s) => Ok((s, ty)),
            None => err(format!("{value} is a pointer where a number or boolean is wanted")),
        }
    }

    fn ptr(&self, value: Value) -> Result<Space, Error> {
        match self.ty(value)? {
            Type::Ptr(space) => Ok(space),
            ty => err(format!("{value} is {ty:?}, not a pointer")),
        }
    }

    fn int_scalar(&self, value: Value) -> Result<(), Error> {
        match self.ty(value)? {
            Type::Scalar(s) if s.is_int() => Ok(()),
            ty => err(format!("{value} is {ty:?}, not an integer")),
        }
    }

    fn bool_scalar(&self, value: Value) -> Result<(), Error> {
        match self.ty(value)? {
            Type::BOOL => Ok(()),
            ty => err(format!("{value} is {ty:?}, not a boolean")),
        }
    }

    fn block(&mut self, block: &Block, loops: Loops) -> Result<(), Error> {
        for stmt in block {
            self.stmt(stmt, loops)?;
        }
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt, loops: Loops) -> Result<(), Error> {
        match stmt {
            Stmt::Let(value, op) => {
                let index = value.0 as usize;
                if index >= self.defined.len() {
                    return err(format!("{value} doesn't exist"));
                }
                if self.defined[index] {
                    return err(format!("{value} is defined twice"));
                }
                let ty = self.op(op).map_err(|e| Error { message: format!("{value} = {op:?}: {}", e.message) })?;
                if ty != self.entry.values[index] {
                    return err(format!("{value} is declared {:?} but {op:?} gives {ty:?}", self.entry.values[index]));
                }
                self.defined[index] = true;
            }
            Stmt::Store(ptr, value) => {
                self.ptr(*ptr)?;
                let (s, _) = self.data(*value)?;
                if s == Scalar::Bool {
                    return err(format!("{value}: booleans aren't stored in memory"));
                }
            }
            Stmt::Write(var, value) => {
                let Some(&ty) = self.entry.vars.get(var.0 as usize) else { return err(format!("{var} doesn't exist")) };
                if self.ty(*value)? != ty {
                    return err(format!("{var} is {ty:?} but {value} is {:?}", self.ty(*value)?));
                }
            }
            Stmt::If { cond, then, otherwise } => {
                self.bool_scalar(*cond)?;
                self.block(then, loops)?;
                self.block(otherwise, loops)?;
            }
            Stmt::Loop { body, continuing, break_if } => {
                self.block(body, Loops::Body)?;
                self.block(continuing, Loops::Continuing)?;
                if let Some(cond) = break_if {
                    self.bool_scalar(*cond)?;
                }
            }
            Stmt::Break if loops == Loops::None => return err("a break outside any loop".into()),
            Stmt::Continue if loops != Loops::Body => return err("a continue outside a loop's body".into()),
            Stmt::Break | Stmt::Continue | Stmt::Return => {}
            Stmt::Barrier(_) if self.entry.stage != Stage::Compute => return err("a barrier outside a compute kernel".into()),
            Stmt::Barrier(_) => {}
            Stmt::Output(n, value) => {
                let Some(io) = self.entry.outputs.get(*n as usize) else { return err(format!("no output {n}")) };
                if self.ty(*value)? != io.ty {
                    return err(format!("output {n} is {:?} but {value} is {:?}", io.ty, self.ty(*value)?));
                }
            }
            Stmt::Discard if self.entry.stage != Stage::Fragment => return err("a discard outside a fragment shader".into()),
            Stmt::Discard => {}
        }
        Ok(())
    }

    fn binding(&self, n: u32) -> Result<BindingKind, Error> {
        match self.entry.bindings.get(n as usize) {
            Some(b) => Ok(b.kind),
            None => err(format!("no binding {n}")),
        }
    }

    fn fragment_only(&self, what: &str) -> Result<(), Error> {
        if self.entry.stage == Stage::Fragment { Ok(()) } else { err(format!("{what} outside a fragment shader")) }
    }

    fn float(&self, value: Value) -> Result<Type, Error> {
        match self.data(value)? {
            (Scalar::F32, ty) => Ok(ty),
            (_, ty) => err(format!("{value} is {ty:?}, not f32")),
        }
    }

    fn op(&self, op: &Op) -> Result<Type, Error> {
        let entry = self.entry;
        Ok(match op {
            Op::Const(s, _) => Type::Scalar(*s),
            Op::Builtin(b) => match b.stage() {
                Some(stage) if stage != entry.stage => return err(format!("{b:?} in a {:?} shader", entry.stage)),
                _ => b.ty(),
            },
            Op::Binding(n) if (*n as usize) < entry.bindings.len() => {
                if !self.binding(*n)?.is_buffer() {
                    return err(format!("binding {n} is {:?}, not memory", self.binding(*n)?));
                }
                Type::Ptr(Space::Device)
            }
            Op::Workgroup(n) if (*n as usize) < entry.workgroup.len() => Type::Ptr(Space::Workgroup),
            Op::Private(n) if (*n as usize) < entry.private.len() => Type::Ptr(Space::Private),
            Op::Binding(n) | Op::Workgroup(n) | Op::Private(n) => return err(format!("no region {n}")),
            Op::Offset(ptr, bytes) => {
                let space = self.ptr(*ptr)?;
                self.int_scalar(*bytes)?;
                Type::Ptr(space)
            }
            Op::Load(ptr, ty) => {
                self.ptr(*ptr)?;
                match ty.scalar() {
                    Some(Scalar::Bool) | None => return err(format!("{ty:?} isn't loaded from memory")),
                    Some(_) => *ty,
                }
            }
            Op::Read(var) => match entry.vars.get(var.0 as usize) {
                Some(ty) => *ty,
                None => return err(format!("{var} doesn't exist")),
            },
            Op::Unary(op, a) => {
                let (s, ty) = self.data(*a)?;
                let ok = match op {
                    UnOp::Neg | UnOp::Abs => s.is_int() || s.is_float(),
                    UnOp::Not => s.is_int() || s == Scalar::Bool,
                    UnOp::Sqrt | UnOp::Floor | UnOp::Ceil | UnOp::Trunc | UnOp::RoundEven => s.is_float(),
                    UnOp::Sin | UnOp::Cos | UnOp::Exp2 | UnOp::Log2 => s.is_float(),
                    UnOp::Popcount => s.is_int(),
                };
                if !ok {
                    return err(format!("{op:?} on {ty:?}"));
                }
                ty
            }
            Op::Binary(op, a, b) => {
                let (s, ty) = self.data(*a)?;
                if self.ty(*b)? != ty {
                    return err(format!("{a} is {ty:?} but {b} is {:?}", self.ty(*b)?));
                }
                let ok = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Min | BinOp::Max => {
                        s.is_int() || s.is_float()
                    }
                    BinOp::And | BinOp::Or | BinOp::Xor => s.is_int() || s == Scalar::Bool,
                    BinOp::Shl | BinOp::Shr => s.is_int(),
                };
                if !ok {
                    return err(format!("{op:?} on {ty:?}"));
                }
                ty
            }
            Op::Compare(cmp, a, b) => {
                let (s, ty) = self.data(*a)?;
                if self.ty(*b)? != ty {
                    return err(format!("{a} is {ty:?} but {b} is {:?}", self.ty(*b)?));
                }
                if s == Scalar::Bool && !matches!(cmp, Cmp::Eq | Cmp::Ne) {
                    return err(format!("{cmp:?} on booleans"));
                }
                ty.with_scalar(Scalar::Bool)
            }
            Op::Select(cond, a, b) => {
                let (cs, cty) = self.data(*cond)?;
                let ty = self.ty(*a)?;
                if self.ty(*b)? != ty {
                    return err(format!("{a} is {ty:?} but {b} is {:?}", self.ty(*b)?));
                }
                if cs != Scalar::Bool || (cty.lanes() != 1 && cty.lanes() != ty.lanes()) {
                    return err(format!("selecting {ty:?} by {cty:?}"));
                }
                ty
            }
            Op::Convert(a, to) => {
                let (_, ty) = self.data(*a)?;
                ty.with_scalar(*to)
            }
            Op::Bitcast(a, to) => {
                let (_, ty) = self.data(*a)?;
                if to.scalar().is_none() || to.size() != ty.size() || to.scalar() == Some(Scalar::Bool) {
                    return err(format!("a bitcast from {ty:?} to {to:?}"));
                }
                *to
            }
            Op::Construct(parts) => {
                if !(2..=4).contains(&parts.len()) {
                    return err(format!("a vector of {} lanes", parts.len()));
                }
                let first = self.ty(parts[0])?;
                let Type::Scalar(s) = first else { return err(format!("a vector of {first:?}")) };
                for part in &parts[1..] {
                    if self.ty(*part)? != first {
                        return err(format!("a vector of {first:?} and {:?}", self.ty(*part)?));
                    }
                }
                Type::Vector(s, parts.len() as u8)
            }
            Op::Extract(v, lane) => match self.ty(*v)? {
                Type::Vector(s, n) if *lane < n => Type::Scalar(s),
                ty => return err(format!("lane {lane} of {ty:?}")),
            },
            Op::ExtractDynamic(v, lane) => {
                self.int_scalar(*lane)?;
                match self.ty(*v)? {
                    Type::Vector(s, _) => Type::Scalar(s),
                    ty => return err(format!("a lane of {ty:?}")),
                }
            }
            Op::Insert(v, lane, x) => match self.ty(*v)? {
                Type::Vector(s, n) if *lane < n && self.ty(*x)? == Type::Scalar(s) => Type::Vector(s, n),
                ty => return err(format!("{x} into lane {lane} of {ty:?}")),
            },
            Op::Atomic(_, ptr, x, s) | Op::CompareExchange(ptr, _, x, s) => {
                if self.ptr(*ptr)? == Space::Private {
                    return err("an atomic on private memory".into());
                }
                if !s.is_int() || self.ty(*x)? != Type::Scalar(*s) {
                    return err(format!("an atomic of {s:?} with {:?}", self.ty(*x)?));
                }
                if let Op::CompareExchange(_, compare, _, _) = op
                    && self.ty(*compare)? != Type::Scalar(*s) {
                        return err(format!("comparing {s:?} with {:?}", self.ty(*compare)?));
                    }
                Type::Scalar(*s)
            }
            Op::Subgroup(sop, value, lane) => {
                if let Some(lane) = lane {
                    match self.ty(*lane)? {
                        Type::U32 | Type::I32 => {}
                        ty => return err(format!("a lane of type {ty:?}")),
                    }
                }
                let needs_lane = matches!(
                    sop,
                    SubgroupOp::Shuffle | SubgroupOp::ShuffleXor | SubgroupOp::ShuffleUp | SubgroupOp::ShuffleDown
                );
                if needs_lane != lane.is_some() {
                    return err(format!("{sop:?} with a lane operand {}", if lane.is_some() { "given" } else { "missing" }));
                }
                match (sop, value) {
                    (SubgroupOp::Elect, None) => Type::BOOL,
                    (SubgroupOp::Elect, Some(_)) | (_, None) => return err(format!("{sop:?}'s operand")),
                    (SubgroupOp::All | SubgroupOp::Any, Some(v)) => {
                        self.bool_scalar(*v)?;
                        Type::BOOL
                    }
                    (SubgroupOp::Reduce(op) | SubgroupOp::InclusiveScan(op) | SubgroupOp::ExclusiveScan(op), Some(v)) => {
                        let (s, ty) = self.data(*v)?;
                        let ok = match op {
                            BinOp::Add | BinOp::Mul | BinOp::Min | BinOp::Max => s.is_int() || s.is_float(),
                            BinOp::And | BinOp::Or | BinOp::Xor => s.is_int() || s == Scalar::Bool,
                            _ => false,
                        };
                        if !ok {
                            return err(format!("a subgroup {op:?} of {ty:?}"));
                        }
                        ty
                    }
                    (_, Some(v)) => self.data(*v)?.1,
                }
            }
            Op::Input(n) => {
                if entry.stage == Stage::Compute {
                    return err("a stage input in a compute kernel".into());
                }
                match entry.inputs.get(*n as usize) {
                    Some(io) => io.ty,
                    None => return err(format!("no input {n}")),
                }
            }
            Op::Sample { image, sampler, coord, lod } => {
                match (self.binding(*image)?, sampler) {
                    (BindingKind::Combined, None) => {}
                    (BindingKind::Image, Some(s)) if self.binding(*s)? == BindingKind::Sampler => {}
                    (kind, s) => return err(format!("sampling binding {image} ({kind:?}) with sampler {s:?}")),
                }
                if self.ty(*coord)? != Type::Vector(Scalar::F32, 2) {
                    return err(format!("a sample at {coord}, a {:?}: 2D coordinates are vec2<f32>", self.ty(*coord)?));
                }
                match lod {
                    Lod::Implicit => self.fragment_only("an implicit level of detail")?,
                    Lod::Bias(v) => {
                        self.fragment_only("an implicit level of detail")?;
                        if self.ty(*v)? != Type::F32 {
                            return err(format!("a bias {v} that isn't f32"));
                        }
                    }
                    Lod::Level(v) if self.ty(*v)? != Type::F32 => return err(format!("a level {v} that isn't f32")),
                    Lod::Level(_) => {}
                }
                Type::Vector(Scalar::F32, 4)
            }
            Op::Derivative(_, v) => {
                self.fragment_only("a derivative")?;
                self.float(*v)?
            }
        })
    }
}
