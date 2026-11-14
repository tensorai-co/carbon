//! GLSL's syntax tree, as the parser makes it: declarations, statements and expressions, each with its line. Types are
//! still names here; `lower` resolves them.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A type as written: a name (`vec3`, a struct's), or a struct defined in place, and any array sizes after it.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeSpec {
    pub base: TypeBase,
    /// Sizes outermost first; `None` for `[]`.
    pub dims: Vec<Option<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeBase {
    Named(String),
    Struct(StructDef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: Option<String>,
    pub members: Vec<Member>,
    pub line: u32,
}

/// A struct's or block's member declaration: one type, one or more names.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub quals: Qualifiers,
    pub ty: TypeSpec,
    pub names: Vec<(String, Vec<Option<Expr>>)>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    None,
    Const,
    In,
    Out,
    InOut,
    Uniform,
    Buffer,
    Shared,
    /// GLSL 1.x's vertex inputs.
    Attribute,
    /// GLSL 1.x's values between stages: an output of a vertex shader, an input of a fragment shader.
    Varying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Smooth,
    Flat,
    NoPerspective,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Qualifiers {
    pub storage: Storage,
    pub interp: Option<Interp>,
    pub invariant: bool,
    pub centroid: bool,
    /// `layout(...)`'s names, each with its value if it has one.
    pub layout: Vec<(String, Option<Expr>)>,
    pub readonly: bool,
    pub writeonly: bool,
}

impl Qualifiers {
    pub fn none() -> Qualifiers {
        Qualifiers { storage: Storage::None, interp: None, invariant: false, centroid: false, layout: Vec::new(), readonly: false, writeonly: false }
    }
}

/// A variable declared: its name, its own array sizes, and its initializer.
#[derive(Debug, Clone, PartialEq)]
pub struct Declarator {
    pub name: String,
    pub dims: Vec<Option<Expr>>,
    pub init: Option<Init>,
    pub line: u32,
}

/// An initializer: an expression, or GLSL 4.2's braces.
#[derive(Debug, Clone, PartialEq)]
pub enum Init {
    Expr(Expr),
    List(Vec<Init>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub quals: Qualifiers,
    pub ty: TypeSpec,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub ret: TypeSpec,
    pub name: String,
    pub params: Vec<Param>,
    /// `None` for a prototype.
    pub body: Option<Vec<Stmt>>,
    pub line: u32,
}

/// A uniform, buffer, `in` or `out` block.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub quals: Qualifiers,
    pub name: String,
    pub members: Vec<Member>,
    /// The instance's name and array sizes, if it has one.
    pub instance: Option<(String, Vec<Option<Expr>>)>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum External {
    /// Variables (or only a struct, with none).
    Decl { quals: Qualifiers, ty: TypeSpec, vars: Vec<Declarator>, line: u32 },
    Block(Block),
    Function(Function),
    /// A qualifier alone: `layout(local_size_x = 64) in;`, `layout(std140) uniform;`.
    Default { quals: Qualifiers, line: u32 },
    /// `invariant gl_Position;` and the like.
    Requalify { quals: Qualifiers, names: Vec<String>, line: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Block(Vec<Stmt>),
    Decl { quals: Qualifiers, ty: TypeSpec, vars: Vec<Declarator> },
    Expr(Expr),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    /// A condition that may declare a variable (`while (bool b = f())`) is an expression here, or a declaration.
    While(Cond, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For { init: Option<Box<Stmt>>, cond: Option<Cond>, step: Option<Expr>, body: Box<Stmt> },
    Switch(Expr, Vec<Stmt>),
    Case(Expr),
    Default,
    Break,
    Continue,
    Return(Option<Expr>),
    Discard,
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Expr(Expr),
    Decl { ty: TypeSpec, name: String, init: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unary {
    Neg,
    Plus,
    Not,
    BitNot,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binary {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Xor,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Ident(String),
    Int(u32, bool),
    Float(f64),
    Bool(bool),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    /// `=` (no operator) or `op=`.
    Assign(Option<Binary>, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
    /// A function's call by name (a user's or a built-in), or a constructor of a type.
    Call(Callee, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    /// A member, or a swizzle.
    Field(Box<Expr>, String),
    /// `.length()`.
    Length(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    Name(String),
    Type(TypeSpec),
}
