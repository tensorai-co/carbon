//! An AIR module in memory: LLVM IR's types, constants, functions and instructions, as much of them as an entry point
//! lowered from gpu's IR uses, and the metadata that makes a function a kernel, a vertex function or a fragment
//! function.
//!
//! Types and constants are interned, so each is one ID and anything that refers to one comes after it; the bitcode
//! writer numbers them in that order. A function's body is a list of blocks, each a list of instructions, built in
//! the order they run.

use std::collections::HashMap;

/// An interned type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Ty {
    Void,
    /// An integer of that many bits.
    Int(u32),
    Float,
    Vector(TypeId, u32),
    Array(TypeId, u64),
    /// The pointee and the address space.
    Ptr(TypeId, u32),
    /// The result and the parameters.
    Func(TypeId, Vec<TypeId>),
    /// A literal struct, and whether it's packed: what a vertex or fragment function returns, and what sampling gives.
    Struct(Vec<TypeId>, bool),
    /// A named opaque struct: Metal's texture and sampler handles (`struct._texture_2d_t`, `struct._sampler_t`).
    Opaque(String),
}

/// Address spaces, as AIR numbers them.
pub(crate) const THREAD: u32 = 0;
pub(crate) const DEVICE: u32 = 1;
/// Where samplers are.
pub(crate) const CONSTANT: u32 = 2;
pub(crate) const THREADGROUP: u32 = 3;

/// An interned constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ConstId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Const {
    /// The bits, truncated to the type's width.
    Int(TypeId, u64),
    /// An IEEE single's bits.
    Float(TypeId, u32),
    Undef(TypeId),
    /// Zero, or a null pointer.
    Null(TypeId),
    Vector(TypeId, Vec<ConstId>),
}

impl Const {
    pub(crate) fn ty(&self) -> TypeId {
        match self {
            Const::Int(t, _) | Const::Float(t, _) | Const::Undef(t) | Const::Null(t) | Const::Vector(t, _) => *t,
        }
    }
}

/// An operand: a module-level value or one of the function's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Value {
    Const(ConstId),
    Global(u32),
    Func(u32),
    Param(u32),
    Inst(u32),
}

/// LLVM's attribute kinds, by their bitcode codes, as xcrun gives them to the `air.*` intrinsics.
pub(crate) mod attr {
    const NOUNWIND: u64 = 18;
    const READNONE: u64 = 20;
    const READONLY: u64 = 21;
    const CONVERGENT: u64 = 43;
    const ARGMEMONLY: u64 = 45;
    const WRITEONLY: u64 = 52;
    const WILLRETURN: u64 = 61;
    const NOFREE: u64 = 62;
    const NOSYNC: u64 = 63;
    const MUSTPROGRESS: u64 = 70;

    /// Math, conversions, integer functions: no memory, no side effects.
    pub(crate) const PURE: &[u64] = &[NOUNWIND, READNONE, WILLRETURN, NOFREE, NOSYNC, MUSTPROGRESS];
    /// SIMD-group operations and barriers: convergent, so no optimization moves them across control flow.
    pub(crate) const CONVERGENT_SET: &[u64] = &[NOUNWIND, CONVERGENT, WILLRETURN, MUSTPROGRESS];
    pub(crate) const ATOMIC: &[u64] = &[NOUNWIND, WILLRETURN, MUSTPROGRESS];
    pub(crate) const MEMSET: &[u64] = &[NOUNWIND, ARGMEMONLY, WRITEONLY, WILLRETURN, NOFREE];
    /// Sampling a texture: it reads the texture and sampler it's given, and is convergent (it takes derivatives).
    pub(crate) const SAMPLE: &[u64] = &[NOUNWIND, ARGMEMONLY, CONVERGENT, READONLY, WILLRETURN];
    /// `air.discard_fragment`: it has an effect, so it's kept.
    pub(crate) const DISCARD: &[u64] = &[NOUNWIND, WILLRETURN, MUSTPROGRESS];
}

/// A module-level variable: threadgroup memory.
#[derive(Debug, Clone)]
pub(crate) struct Global {
    pub name: String,
    pub elem: TypeId,
    /// The pointer type the global's value has.
    pub ty: TypeId,
    pub space: u32,
    pub align: u32,
    pub init: ConstId,
}

/// What Metal binds an argument to.
#[derive(Debug, Clone)]
pub(crate) enum Arg {
    /// `device uchar* name [[buffer(location)]]`.
    Buffer { location: u32, writable: bool },
    /// A value Metal passes: `uint3 gid [[thread_position_in_grid]]`, its descriptor name and its MSL type's.
    Builtin { name: &'static str, type_name: &'static str },
    /// `texture2d<float> name [[texture(location)]]`, read through a sampler.
    Texture { location: u32 },
    /// `sampler name [[sampler(location)]]`.
    Sampler { location: u32 },
    /// A value interpolated from the vertices: `user(locnN)`, how it's interpolated (`air.center`, `air.perspective`;
    /// `air.flat`), and its MSL type's name.
    Interpolated { location: u32, how: &'static [&'static str], type_name: &'static str },
    /// The fragment's position, `float4 [[position]]`.
    FragmentPosition,
}

/// One of a vertex or fragment function's results, a member of the struct it returns, in order.
#[derive(Debug, Clone)]
pub(crate) enum Output {
    /// The vertex's clip-space position.
    Position,
    /// A value passed to the fragment function, at `user(locnN)`.
    Vertex { location: u32, type_name: &'static str },
    /// A colour attachment's value.
    Color { index: u32, type_name: &'static str },
    /// The fragment's depth.
    Depth,
}

/// Which kind of function the module's entry point is, which its metadata lists it as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Kernel,
    Vertex,
    Fragment,
}

/// A kernel argument.
#[derive(Debug, Clone)]
pub(crate) struct Param {
    pub ty: TypeId,
    pub name: String,
    pub arg: Arg,
}

/// A function: the kernel, with a body, or an intrinsic it calls, declared.
#[derive(Debug, Clone)]
pub(crate) struct Func {
    pub name: String,
    /// The function type, and the pointer to it the function's value has.
    pub sig: TypeId,
    pub ptr: TypeId,
    pub attrs: &'static [u64],
    pub params: Vec<Param>,
    pub body: Option<Body>,
}

/// What an instruction does. Its result's type is the [`Inst`]'s.
#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// LLVM's binary opcode (add 0, sub 1, mul 2, udiv 3, sdiv 4, urem 5, srem 6, shl 7, lshr 8, ashr 9, and 10,
    /// or 11, xor 12; the float forms share add, sub, mul, div and rem's numbers).
    Binary(u64, Value, Value),
    /// LLVM's cast opcode, to the instruction's type.
    Cast(u64, Value),
    FNeg(Value),
    /// A predicate: fcmp's below 32, icmp's from 32.
    Cmp(u64, Value, Value),
    Select(Value, Value, Value),
    /// `alloca elem, i32 count`.
    Alloca { elem: TypeId, count: Value, align: u32 },
    Load { ptr: Value, align: u32 },
    Store { ptr: Value, value: Value, align: u32 },
    /// `getelementptr elem, ptr, index`.
    Gep { elem: TypeId, ptr: Value, index: Value },
    ExtractElement(Value, Value),
    InsertElement(Value, Value, Value),
    /// A member of a struct, by index.
    ExtractValue(Value, u32),
    /// The struct with a member replaced.
    InsertValue(Value, Value, u32),
    Call { func: u32, args: Vec<Value> },
    Br(u32),
    CondBr(Value, u32, u32),
    Ret,
    /// `ret` with the function's result.
    RetValue(Value),
}

#[derive(Debug, Clone)]
pub(crate) struct Inst {
    pub ty: TypeId,
    pub op: Op,
}

impl Inst {
    pub(crate) fn terminates(&self) -> bool {
        matches!(self.op, Op::Br(_) | Op::CondBr(..) | Op::Ret | Op::RetValue(_))
    }
}

/// A function's body: its instructions, and its blocks as lists of them in order.
#[derive(Debug, Clone, Default)]
pub(crate) struct Body {
    pub insts: Vec<Inst>,
    pub blocks: Vec<Vec<u32>>,
}

/// The kernel's index in [`Module::funcs`]: it's made first.
pub(crate) const KERNEL: u32 = 0;

/// A module: one kernel and the intrinsics it calls, for a deployment target and an MSL version.
#[derive(Debug, Clone)]
pub(crate) struct Module {
    pub types: Vec<Ty>,
    type_ids: HashMap<Ty, TypeId>,
    pub consts: Vec<Const>,
    const_ids: HashMap<Const, ConstId>,
    pub globals: Vec<Global>,
    pub funcs: Vec<Func>,
    func_ids: HashMap<String, u32>,
    /// The oldest macOS the module runs on, and the MSL version it says it was written in.
    pub os: (u32, u32),
    pub language: (u32, u32),
    pub stage: Stage,
    /// A vertex or fragment function's results, as the members of the struct it returns.
    pub outputs: Vec<Output>,
}

impl Module {
    /// A module whose kernel, `name`, takes `params` and has no body yet.
    pub(crate) fn new(name: &str) -> Module {
        let mut m = Module {
            types: Vec::new(),
            type_ids: HashMap::new(),
            consts: Vec::new(),
            const_ids: HashMap::new(),
            globals: Vec::new(),
            funcs: Vec::new(),
            func_ids: HashMap::new(),
            os: (13, 0),
            language: (3, 0),
            stage: Stage::Kernel,
            outputs: Vec::new(),
        };
        let void = m.ty(Ty::Void);
        let sig = m.ty(Ty::Func(void, Vec::new()));
        let ptr = m.ty(Ty::Ptr(sig, THREAD));
        m.funcs.push(Func { name: name.into(), sig, ptr, attrs: &[], params: Vec::new(), body: Some(Body::default()) });
        m.func_ids.insert(name.into(), KERNEL);
        m
    }

    pub(crate) fn ty(&mut self, ty: Ty) -> TypeId {
        if let Some(id) = self.type_ids.get(&ty) {
            return *id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(ty.clone());
        self.type_ids.insert(ty, id);
        id
    }

    pub(crate) fn get(&self, t: TypeId) -> &Ty {
        &self.types[t.0 as usize]
    }

    pub(crate) fn void(&mut self) -> TypeId {
        self.ty(Ty::Void)
    }

    pub(crate) fn int(&mut self, bits: u32) -> TypeId {
        self.ty(Ty::Int(bits))
    }

    pub(crate) fn ptr(&mut self, elem: TypeId, space: u32) -> TypeId {
        self.ty(Ty::Ptr(elem, space))
    }

    /// The element of a vector, or the type itself.
    pub(crate) fn scalar_of(&self, t: TypeId) -> TypeId {
        match self.get(t) {
            Ty::Vector(e, _) => *e,
            _ => t,
        }
    }

    pub(crate) fn lanes(&self, t: TypeId) -> u32 {
        match self.get(t) {
            Ty::Vector(_, n) => *n,
            _ => 1,
        }
    }

    /// The same shape as `like` (a scalar or a vector of its lanes), of `scalar`.
    pub(crate) fn shaped(&mut self, like: TypeId, scalar: TypeId) -> TypeId {
        match *self.get(like) {
            Ty::Vector(_, n) => self.ty(Ty::Vector(scalar, n)),
            _ => scalar,
        }
    }

    pub(crate) fn konst(&mut self, c: Const) -> Value {
        if let Some(id) = self.const_ids.get(&c) {
            return Value::Const(*id);
        }
        let id = ConstId(self.consts.len() as u32);
        self.consts.push(c.clone());
        self.const_ids.insert(c, id);
        Value::Const(id)
    }

    pub(crate) fn const_id(&mut self, c: Const) -> ConstId {
        match self.konst(c) {
            Value::Const(id) => id,
            _ => unreachable!("konst gives constants"),
        }
    }

    /// An integer or float constant of `ty`'s scalar, splatted across its lanes if it's a vector. Integers are their
    /// bits; floats are an IEEE single's.
    pub(crate) fn splat(&mut self, ty: TypeId, bits: u64) -> Value {
        let s = self.scalar_of(ty);
        let one = match *self.get(s) {
            Ty::Float => Const::Float(s, bits as u32),
            Ty::Int(w) => Const::Int(s, if w >= 64 { bits } else { bits & ((1u64 << w) - 1) }),
            _ => Const::Null(s),
        };
        if s == ty {
            return self.konst(one);
        }
        let n = self.lanes(ty);
        let lane = self.const_id(one);
        self.konst(Const::Vector(ty, vec![lane; n as usize]))
    }

    /// An `i32` constant.
    pub(crate) fn i32(&mut self, v: u32) -> Value {
        let t = self.int(32);
        self.konst(Const::Int(t, v as u64))
    }

    pub(crate) fn undef(&mut self, ty: TypeId) -> Value {
        self.konst(Const::Undef(ty))
    }

    pub(crate) fn null(&mut self, ty: TypeId) -> Value {
        self.konst(Const::Null(ty))
    }

    pub(crate) fn global(&mut self, name: String, elem: TypeId, space: u32, align: u32) -> Value {
        let ty = self.ptr(elem, space);
        let init = self.const_id(Const::Undef(elem));
        self.globals.push(Global { name, elem, ty, space, align, init });
        Value::Global(self.globals.len() as u32 - 1)
    }

    /// The intrinsic `name`, declared on first use with the attributes given.
    pub(crate) fn declare(&mut self, name: &str, ret: TypeId, params: Vec<TypeId>, attrs: &'static [u64]) -> u32 {
        if let Some(f) = self.func_ids.get(name) {
            return *f;
        }
        let sig = self.ty(Ty::Func(ret, params.clone()));
        let ptr = self.ptr(sig, THREAD);
        let params = params
            .into_iter()
            .map(|ty| Param { ty, name: String::new(), arg: Arg::Buffer { location: 0, writable: false } })
            .collect();
        let id = self.funcs.len() as u32;
        self.funcs.push(Func { name: name.into(), sig, ptr, attrs, params, body: None });
        self.func_ids.insert(name.into(), id);
        id
    }

    /// Gives the entry point its arguments, returning nothing; its type follows from them.
    pub(crate) fn set_params(&mut self, params: Vec<Param>) {
        let void = self.void();
        self.set_signature(void, params);
    }

    /// Gives the entry point its arguments and its result's type.
    pub(crate) fn set_signature(&mut self, ret: TypeId, params: Vec<Param>) {
        let sig = self.ty(Ty::Func(ret, params.iter().map(|p| p.ty).collect()));
        let ptr = self.ptr(sig, THREAD);
        let k = &mut self.funcs[KERNEL as usize];
        k.sig = sig;
        k.ptr = ptr;
        k.params = params;
    }

    /// A value's type, in the kernel.
    pub(crate) fn type_of(&self, v: Value) -> TypeId {
        match v {
            Value::Const(c) => self.consts[c.0 as usize].ty(),
            Value::Global(g) => self.globals[g as usize].ty,
            Value::Func(f) => self.funcs[f as usize].ptr,
            Value::Param(p) => self.funcs[KERNEL as usize].params[p as usize].ty,
            Value::Inst(i) => self.body().insts[i as usize].ty,
        }
    }

    pub(crate) fn body(&self) -> &Body {
        self.funcs[KERNEL as usize].body.as_ref().expect("the kernel has a body")
    }

    pub(crate) fn body_mut(&mut self) -> &mut Body {
        self.funcs[KERNEL as usize].body.as_mut().expect("the kernel has a body")
    }
}
