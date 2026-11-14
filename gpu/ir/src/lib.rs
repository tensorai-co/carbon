//! gpu's IR: what every program a device runs looks like between a frontend (SPIR-V, later DXBC, DXIL, PTX) and a
//! backend (AIR for Metal, the CPU, later SPIR-V).
//!
//! - **SSA values with structured control flow.** A value is defined once, by a [`Stmt::Let`], and used anywhere after
//!   it in program order where its definition dominates the use (frontends guarantee that; the validator checks the
//!   order). Control flow is a tree: [`Stmt::If`], [`Stmt::Loop`], `Break`, `Continue`, `Return`.
//! - **Variables for what changes**: a [`Var`] is a typed register written with [`Stmt::Write`] and read with
//!   [`Op::Read`] (SPIR-V's phis and its scalar locals become these).
//! - **Memory is bytes, and pointers are addresses.** A pointer is a [`Space`] and a byte address in it; loads and
//!   stores give their type, and arithmetic on pointers is [`Op::Offset`]. That's CUDA's model and Vulkan's buffer
//!   device addresses', and a frontend lowers typed access chains to it with its own layout rules.
//! - **Subgroups are first-class**, with the width part of the program ([`Entry::subgroup_size`]).
//! - **Stages**: an entry point is a compute kernel, a vertex shader or a fragment shader ([`Stage`]). The graphics
//!   stages read their inputs with [`Op::Input`] and write their outputs with [`Stmt::Output`], by slot (a location,
//!   or the vertex's position); a fragment shader can sample images ([`Op::Sample`]), take derivatives across its
//!   2×2 quad ([`Op::Derivative`]) and [`Stmt::Discard`] itself.
//!
//! Values have scalar, vector (2 to 4 lanes) or pointer types; composites (structs, arrays, matrices) don't exist as
//! values, so frontends split them into their parts. Images and samplers aren't values either: [`Op::Sample`] names
//! the bindings it reads.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

mod print;
#[cfg(test)]
mod tests;
mod validate;

pub use validate::{Error, validate};

/// A scalar type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scalar {
    Bool,
    U32,
    I32,
    F32,
    U64,
    I64,
}

impl Scalar {
    /// Bytes in memory. A `Bool` is only ever in a [`Var`], never stored, but is a byte if a backend needs one.
    pub const fn size(self) -> u32 {
        match self {
            Scalar::Bool => 1,
            Scalar::U32 | Scalar::I32 | Scalar::F32 => 4,
            Scalar::U64 | Scalar::I64 => 8,
        }
    }

    pub const fn is_int(self) -> bool {
        matches!(self, Scalar::U32 | Scalar::I32 | Scalar::U64 | Scalar::I64)
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, Scalar::I32 | Scalar::I64)
    }

    pub const fn is_float(self) -> bool {
        matches!(self, Scalar::F32)
    }

    /// Bits in a value of this type.
    pub const fn bits(self) -> u32 {
        match self {
            Scalar::Bool => 1,
            Scalar::U32 | Scalar::I32 | Scalar::F32 => 32,
            Scalar::U64 | Scalar::I64 => 64,
        }
    }
}

/// Where a pointer points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Space {
    /// The device's memory: buffers, reached through bindings or 64-bit device addresses.
    Device,
    /// One workgroup's memory, shared by its invocations (SPIR-V's `Workgroup`, CUDA's `__shared__`, MSL's
    /// `threadgroup`).
    Workgroup,
    /// One invocation's own memory, for locals that need an address (arrays indexed at run time).
    Private,
}

/// A value's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Scalar(Scalar),
    /// 2, 3 or 4 lanes.
    Vector(Scalar, u8),
    Ptr(Space),
}

impl Type {
    pub const U32: Type = Type::Scalar(Scalar::U32);
    pub const I32: Type = Type::Scalar(Scalar::I32);
    pub const F32: Type = Type::Scalar(Scalar::F32);
    pub const BOOL: Type = Type::Scalar(Scalar::Bool);
    pub const UVEC3: Type = Type::Vector(Scalar::U32, 3);

    /// The scalar of a scalar or vector; `None` for a pointer.
    pub const fn scalar(self) -> Option<Scalar> {
        match self {
            Type::Scalar(s) | Type::Vector(s, _) => Some(s),
            Type::Ptr(_) => None,
        }
    }

    /// Lanes: 1 for a scalar or pointer.
    pub const fn lanes(self) -> u8 {
        match self {
            Type::Vector(_, n) => n,
            _ => 1,
        }
    }

    /// The same shape with another scalar.
    pub const fn with_scalar(self, scalar: Scalar) -> Type {
        match self {
            Type::Vector(_, n) => Type::Vector(scalar, n),
            _ => Type::Scalar(scalar),
        }
    }

    /// Bytes in memory, tightly packed (a 3-lane vector is 12).
    pub const fn size(self) -> u32 {
        match self {
            Type::Scalar(s) => s.size(),
            Type::Vector(s, n) => s.size() * n as u32,
            Type::Ptr(_) => 8,
        }
    }
}

/// An SSA value: its index in [`Entry::values`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Value(pub u32);

/// A variable: its index in [`Entry::vars`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Var(pub u32);

/// Values the invocation is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `uvec3`: the invocation's position in the whole dispatch.
    GlobalId,
    /// `uvec3`: its position in its workgroup.
    LocalId,
    /// `uvec3`: its workgroup's position in the dispatch.
    GroupId,
    /// `uvec3`: workgroups in the dispatch.
    GroupCount,
    /// `u32`: its position in its workgroup, flattened.
    LocalIndex,
    /// `u32`: its lane in its subgroup.
    SubgroupLane,
    /// `u32`: lanes in a subgroup.
    SubgroupSize,
    /// `u32`: its subgroup's index in the workgroup.
    SubgroupId,
    /// `u32`: subgroups in the workgroup.
    SubgroupCount,
    /// `u32`, vertex shaders: the vertex's index (its index buffer's value plus the draw's vertex offset, or its
    /// place in a draw without one).
    VertexIndex,
    /// `u32`, vertex shaders: the instance's index, counting from the draw's first instance.
    InstanceIndex,
    /// `vec4<f32>`, fragment shaders: the pixel's centre in framebuffer coordinates, its depth, and 1/w.
    FragCoord,
    /// `bool`, fragment shaders: whether the primitive faces the viewer.
    FrontFacing,
}

impl Builtin {
    pub const fn ty(self) -> Type {
        match self {
            Builtin::GlobalId | Builtin::LocalId | Builtin::GroupId | Builtin::GroupCount => Type::UVEC3,
            Builtin::FragCoord => Type::Vector(Scalar::F32, 4),
            Builtin::FrontFacing => Type::BOOL,
            _ => Type::U32,
        }
    }

    /// The stage a builtin belongs to, if it isn't every stage's (the subgroup ones are).
    pub const fn stage(self) -> Option<Stage> {
        match self {
            Builtin::GlobalId | Builtin::LocalId | Builtin::GroupId | Builtin::GroupCount | Builtin::LocalIndex => {
                Some(Stage::Compute)
            }
            Builtin::VertexIndex | Builtin::InstanceIndex => Some(Stage::Vertex),
            Builtin::FragCoord | Builtin::FrontFacing => Some(Stage::Fragment),
            Builtin::SubgroupLane | Builtin::SubgroupSize | Builtin::SubgroupId | Builtin::SubgroupCount => None,
        }
    }
}

/// What an entry point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// A kernel run over a grid of workgroups ([`Entry::workgroup_size`]).
    Compute,
    /// Run once a vertex: reads its attributes, writes its position and what the fragment shader interpolates.
    Vertex,
    /// Run once a pixel a primitive covers, in 2×2 quads: reads what the vertices wrote, interpolated, and writes
    /// colours (and optionally depth).
    Fragment,
}

/// Where a stage input or output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A location: a vertex attribute (vertex inputs), a value passed from the vertex to the fragment shader, or a
    /// colour attachment (fragment outputs).
    Location(u32),
    /// The vertex's clip-space position, `vec4<f32>` (a vertex output).
    Position,
    /// The fragment's depth, `f32`, replacing the interpolated one (a fragment output).
    FragDepth,
}

/// How a value passed from vertices to fragments is interpolated across a primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interpolation {
    /// Perspective-correct.
    Smooth,
    /// Linear in screen space.
    NoPerspective,
    /// The first (provoking) vertex's value, for every fragment.
    Flat,
}

/// One of a stage's inputs or outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Io {
    pub slot: Slot,
    pub ty: Type,
    /// For values passed from vertices to fragments: how; ignored elsewhere.
    pub interpolation: Interpolation,
}

/// Which axis a derivative is taken along: across the quad's columns (x) or rows (y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
}

/// Which mip level a sample reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lod {
    /// From the coordinate's derivatives across the quad (fragment shaders only).
    Implicit,
    /// The derivatives' level plus a bias, an `f32` (fragment shaders only).
    Bias(Value),
    /// The level given, an `f32`.
    Level(Value),
}

/// An operation with one operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// Two's-complement negation for integers, sign flip for floats.
    Neg,
    /// Bitwise not for integers, logical not for booleans.
    Not,
    Abs,
    Sqrt,
    Floor,
    Ceil,
    Trunc,
    /// To the nearest integer, halves to even.
    RoundEven,
    /// Count of set bits.
    Popcount,
    /// Radians.
    Sin,
    Cos,
    /// Two to the power of the operand.
    Exp2,
    /// Base-2 logarithm.
    Log2,
}

/// An operation with two operands of the same type, giving that type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Rounds toward zero for integers; division by zero gives 0 on every backend (no trap).
    Div,
    /// The remainder with the dividend's sign (C's `%`, SPIR-V's `SRem`); by zero gives 0.
    Rem,
    /// Bitwise for integers, logical for booleans.
    And,
    Or,
    Xor,
    /// Shifts take the shift count from the second operand, modulo the width.
    Shl,
    /// Arithmetic for signed types, logical for unsigned.
    Shr,
    Min,
    Max,
}

/// A comparison, giving booleans of the operands' shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A read-modify-write on memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomicOp {
    Add,
    Sub,
    Min,
    Max,
    And,
    Or,
    Xor,
    Exchange,
}

/// An operation across a subgroup's active lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubgroupOp {
    /// The value from the lane named by the second operand.
    Shuffle,
    /// The value from this lane's index xor the second operand.
    ShuffleXor,
    /// The value from this lane's index minus the second operand.
    ShuffleUp,
    /// The value from this lane's index plus the second operand.
    ShuffleDown,
    /// The value from the lowest active lane.
    BroadcastFirst,
    /// `true` in the lowest active lane only; takes no operand.
    Elect,
    /// Booleans: whether every active lane's is true.
    All,
    /// Booleans: whether any active lane's is true.
    Any,
    /// Combined across the active lanes, every lane getting the result.
    Reduce(BinOp),
    /// Combined over the active lanes up to and including this one.
    InclusiveScan(BinOp),
    /// Combined over the active lanes before this one (the operation's identity for the first).
    ExclusiveScan(BinOp),
}

/// What defines a value.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A scalar constant, as bits: integers as their two's-complement (sign-extended for `I32`), floats as their IEEE
    /// encoding, booleans as 0 or 1.
    Const(Scalar, u64),
    Builtin(Builtin),
    /// The address of binding `n`'s memory ([`Entry::bindings`]).
    Binding(u32),
    /// The address of workgroup region `n` ([`Entry::workgroup`]).
    Workgroup(u32),
    /// The address of private region `n` ([`Entry::private`]).
    Private(u32),
    /// A pointer moved by a signed or unsigned integer count of bytes.
    Offset(Value, Value),
    /// A value of the type given, read from memory.
    Load(Value, Type),
    Read(Var),
    Unary(UnOp, Value),
    Binary(BinOp, Value, Value),
    Compare(Cmp, Value, Value),
    /// `cond ? a : b`, lane by lane if `cond` is a vector.
    Select(Value, Value, Value),
    /// A numeric conversion to the scalar given, lane by lane: float to int rounds toward zero and saturates,
    /// int to int wraps or extends by the source's signedness, bool to number is 0 or 1.
    Convert(Value, Scalar),
    /// The same bits as another type of the same size.
    Bitcast(Value, Type),
    /// A vector from scalars, one per lane.
    Construct(Vec<Value>),
    /// One lane of a vector.
    Extract(Value, u8),
    /// The lane of a vector named by an integer value (out of range gives lane 0).
    ExtractDynamic(Value, Value),
    /// A vector with one lane replaced.
    Insert(Value, u8, Value),
    /// `ptr`, `value`, of the scalar type given; gives the old value.
    Atomic(AtomicOp, Value, Value, Scalar),
    /// `ptr`, `compare`, `value`; gives the old value.
    CompareExchange(Value, Value, Value, Scalar),
    /// The operation, the value, and a lane operand for the shuffles.
    Subgroup(SubgroupOp, Option<Value>, Option<Value>),
    /// Stage input `n` ([`Entry::inputs`]): a vertex's attribute, or what the vertices wrote, interpolated.
    Input(u32),
    /// A sample of an image, as `vec4<f32>`: the image binding, the sampler binding (`None` when the image's binding
    /// is a combined image and sampler), a `vec2<f32>` coordinate normalized to the image, and the level.
    Sample { image: u32, sampler: Option<u32>, coord: Value, lod: Lod },
    /// How much an `f32` scalar or vector changes to the next pixel in the quad along an axis (fragment shaders).
    Derivative(Axis, Value),
}

/// Which memory a barrier orders, besides making the workgroup wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Barrier {
    /// Wait for every invocation in the workgroup (`false`: only order memory).
    pub execution: bool,
    pub workgroup_memory: bool,
    pub device_memory: bool,
}

/// One statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let(Value, Op),
    /// `ptr`, `value`.
    Store(Value, Value),
    Write(Var, Value),
    If {
        cond: Value,
        then: Block,
        otherwise: Block,
    },
    /// `body` runs, then `continuing` (also reached by `Continue`), then `break_if` is tested, then it starts again.
    Loop {
        body: Block,
        continuing: Block,
        break_if: Option<Value>,
    },
    /// Leaves the innermost loop.
    Break,
    /// Goes to the innermost loop's `continuing`.
    Continue,
    Return,
    Barrier(Barrier),
    /// Writes stage output `n` ([`Entry::outputs`]); the last write before the invocation ends counts.
    Output(u32, Value),
    /// Ends a fragment shader's invocation and throws its fragment away: nothing it outputs is written.
    Discard,
}

pub type Block = Vec<Stmt>;

/// Memory a program declares for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub size: u32,
    pub align: u32,
}

/// What a binding is: memory a program reaches through [`Op::Binding`]'s pointer, or an image or sampler
/// [`Op::Sample`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// Read and written (SPIR-V's storage buffers).
    Storage { writable: bool },
    /// Read only, small, the same for every invocation (uniform and constant buffers).
    Uniform,
    /// A 2D image a sampler reads (SPIR-V's sampled `OpTypeImage`).
    Image,
    /// How an image is read: filtering and addressing (`OpTypeSampler`).
    Sampler,
    /// An image and its sampler bound together (`OpTypeSampledImage`, GLSL's `sampler2D`).
    Combined,
}

impl BindingKind {
    /// Whether it's memory, reached with [`Op::Binding`].
    pub const fn is_buffer(self) -> bool {
        matches!(self, BindingKind::Storage { .. } | BindingKind::Uniform)
    }
}

/// The descriptor set a program's push constants are bound at: Vulkan's push constant block is a [`BindingKind::Uniform`]
/// binding at this set (binding 0), which the API fills with the bytes pushed, as Metal's `setBytes` would.
pub const PUSH_CONSTANTS: u32 = u32::MAX;

/// A binding: memory the dispatch hands the program, by its index in [`Entry::bindings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    /// Where the API put it: Vulkan's descriptor set and binding.
    pub set: u32,
    pub binding: u32,
    pub kind: BindingKind,
}

/// A program a dispatch or draw runs: one entry point, with everything it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The API's name for it.
    pub name: String,
    pub stage: Stage,
    /// A compute kernel's; `[1, 1, 1]` for the graphics stages.
    pub workgroup_size: [u32; 3],
    /// The subgroup width the program was written for, if it depends on one (CUDA's 32); `None` takes the device's.
    pub subgroup_size: Option<u32>,
    pub bindings: Vec<Binding>,
    /// The graphics stages' inputs and outputs, by the index [`Op::Input`] and [`Stmt::Output`] name.
    pub inputs: Vec<Io>,
    pub outputs: Vec<Io>,
    pub workgroup: Vec<Region>,
    pub private: Vec<Region>,
    pub vars: Vec<Type>,
    /// Each value's type.
    pub values: Vec<Type>,
    pub body: Block,
}

impl Entry {
    /// A compute kernel.
    pub fn new(name: impl Into<String>, workgroup_size: [u32; 3]) -> Self {
        Entry {
            name: name.into(),
            stage: Stage::Compute,
            workgroup_size,
            subgroup_size: None,
            bindings: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            workgroup: Vec::new(),
            private: Vec::new(),
            vars: Vec::new(),
            values: Vec::new(),
            body: Vec::new(),
        }
    }

    /// A vertex or fragment shader.
    pub fn shader(name: impl Into<String>, stage: Stage) -> Self {
        Entry { stage, ..Entry::new(name, [1, 1, 1]) }
    }

    /// A new value of the type given; the caller defines it with a `Let`.
    pub fn value(&mut self, ty: Type) -> Value {
        self.values.push(ty);
        Value(self.values.len() as u32 - 1)
    }

    pub fn var(&mut self, ty: Type) -> Var {
        self.vars.push(ty);
        Var(self.vars.len() as u32 - 1)
    }

    pub fn ty(&self, value: Value) -> Type {
        self.values[value.0 as usize]
    }

    pub fn invocations_per_group(&self) -> u32 {
        self.workgroup_size.iter().product()
    }
}

/// A program: its entry points.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Module {
    pub entries: Vec<Entry>,
}

impl Module {
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

impl fmt::Display for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "$v{}", self.0)
    }
}
