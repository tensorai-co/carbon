//! GLSL's types as the compiler resolves them, how they become the IR's, and how they're laid out in memory: std140
//! (uniform blocks), std430 (buffer blocks, shared memory, and the default uniform block GL fills), column- or
//! row-major.

use alloc::boxed::Box;
use alloc::format;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use gpu_ir::{Scalar, Type};

/// A scalar's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Base {
    Bool,
    Int,
    Uint,
    Float,
}

impl Base {
    pub fn scalar(self) -> Scalar {
        match self {
            Base::Bool => Scalar::Bool,
            Base::Int => Scalar::I32,
            Base::Uint => Scalar::U32,
            Base::Float => Scalar::F32,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Base::Bool => "b",
            Base::Int => "i",
            Base::Uint => "u",
            Base::Float => "",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Base::Bool => "bool",
            Base::Int => "int",
            Base::Uint => "uint",
            Base::Float => "float",
        }
    }
}

/// What a sampler reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dim {
    D2,
    D3,
    Cube,
    Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerTy {
    pub dim: Dim,
    pub base: Base,
    pub shadow: bool,
    pub array: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructTy {
    pub name: String,
    pub members: Vec<(String, Ty)>,
}

/// A GLSL type.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Void,
    Scalar(Base),
    Vector(Base, u8),
    /// Columns, then rows: `mat3x2` has 3 columns of 2.
    Matrix(u8, u8),
    /// The element and its count (`None` for a buffer block's last member, sized by the buffer).
    Array(Box<Ty>, Option<u32>),
    Struct(Arc<StructTy>),
    Sampler(SamplerTy),
}

pub const FLOAT: Ty = Ty::Scalar(Base::Float);
pub const INT: Ty = Ty::Scalar(Base::Int);
pub const UINT: Ty = Ty::Scalar(Base::Uint);
pub const BOOL: Ty = Ty::Scalar(Base::Bool);

impl Ty {
    pub fn vector(base: Base, n: u8) -> Ty {
        if n == 1 { Ty::Scalar(base) } else { Ty::Vector(base, n) }
    }

    /// A scalar's or vector's kind.
    pub fn base(&self) -> Option<Base> {
        match self {
            Ty::Scalar(b) | Ty::Vector(b, _) => Some(*b),
            Ty::Matrix(..) => Some(Base::Float),
            _ => None,
        }
    }

    /// A scalar's or vector's lanes.
    pub fn lanes(&self) -> u8 {
        match self {
            Ty::Vector(_, n) => *n,
            _ => 1,
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Scalar(b) | Ty::Vector(b, _) if *b != Base::Bool) || matches!(self, Ty::Matrix(..))
    }

    pub fn is_scalar_or_vector(&self) -> bool {
        matches!(self, Ty::Scalar(_) | Ty::Vector(..))
    }

    pub fn with_base(&self, base: Base) -> Ty {
        match self {
            Ty::Vector(_, n) => Ty::Vector(base, *n),
            _ => Ty::Scalar(base),
        }
    }

    /// The IR's type for a scalar or vector.
    pub fn ir(&self) -> Option<Type> {
        match self {
            Ty::Scalar(b) => Some(Type::Scalar(b.scalar())),
            Ty::Vector(b, n) => Some(Type::Vector(b.scalar(), *n)),
            _ => None,
        }
    }

    /// Whether a value of this type is made of samplers anywhere (they're bindings, not values).
    pub fn has_sampler(&self) -> bool {
        match self {
            Ty::Sampler(_) => true,
            Ty::Array(e, _) => e.has_sampler(),
            Ty::Struct(s) => s.members.iter().any(|m| m.1.has_sampler()),
            _ => false,
        }
    }

    /// Scalars in it, as a constructor counts them.
    pub fn components(&self) -> u32 {
        match self {
            Ty::Scalar(_) => 1,
            Ty::Vector(_, n) => u32::from(*n),
            Ty::Matrix(c, r) => u32::from(*c) * u32::from(*r),
            Ty::Array(e, Some(n)) => e.components() * n,
            Ty::Struct(s) => s.members.iter().map(|m| m.1.components()).sum(),
            _ => 0,
        }
    }

    /// A matrix's column.
    pub fn column(&self) -> Option<Ty> {
        match self {
            Ty::Matrix(_, r) => Some(Ty::Vector(Base::Float, *r)),
            _ => None,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Void => f.write_str("void"),
            Ty::Scalar(b) => f.write_str(b.name()),
            Ty::Vector(b, n) => write!(f, "{}vec{n}", b.prefix()),
            Ty::Matrix(c, r) if c == r => write!(f, "mat{c}"),
            Ty::Matrix(c, r) => write!(f, "mat{c}x{r}"),
            Ty::Array(e, Some(n)) => write!(f, "{e}[{n}]"),
            Ty::Array(e, None) => write!(f, "{e}[]"),
            Ty::Struct(s) => f.write_str(&s.name),
            Ty::Sampler(s) => {
                let dim = match s.dim {
                    Dim::D2 => "2D",
                    Dim::D3 => "3D",
                    Dim::Cube => "Cube",
                    Dim::Rect => "2DRect",
                };
                write!(f, "{}sampler{dim}{}{}", s.base.prefix(), if s.array { "Array" } else { "" }, if s.shadow { "Shadow" } else { "" })
            }
        }
    }
}

/// A built-in type by name.
pub fn named(name: &str) -> Option<Ty> {
    let vec = |b, n| Some(Ty::Vector(b, n));
    let sampler = |dim, base, shadow, array| Some(Ty::Sampler(SamplerTy { dim, base, shadow, array }));
    match name {
        "void" => Some(Ty::Void),
        "bool" => Some(BOOL),
        "int" => Some(INT),
        "uint" => Some(UINT),
        "float" => Some(FLOAT),
        "vec2" => vec(Base::Float, 2),
        "vec3" => vec(Base::Float, 3),
        "vec4" => vec(Base::Float, 4),
        "ivec2" => vec(Base::Int, 2),
        "ivec3" => vec(Base::Int, 3),
        "ivec4" => vec(Base::Int, 4),
        "uvec2" => vec(Base::Uint, 2),
        "uvec3" => vec(Base::Uint, 3),
        "uvec4" => vec(Base::Uint, 4),
        "bvec2" => vec(Base::Bool, 2),
        "bvec3" => vec(Base::Bool, 3),
        "bvec4" => vec(Base::Bool, 4),
        "mat2" | "mat2x2" => Some(Ty::Matrix(2, 2)),
        "mat3" | "mat3x3" => Some(Ty::Matrix(3, 3)),
        "mat4" | "mat4x4" => Some(Ty::Matrix(4, 4)),
        "mat2x3" => Some(Ty::Matrix(2, 3)),
        "mat2x4" => Some(Ty::Matrix(2, 4)),
        "mat3x2" => Some(Ty::Matrix(3, 2)),
        "mat3x4" => Some(Ty::Matrix(3, 4)),
        "mat4x2" => Some(Ty::Matrix(4, 2)),
        "mat4x3" => Some(Ty::Matrix(4, 3)),
        "sampler2D" => sampler(Dim::D2, Base::Float, false, false),
        "sampler3D" => sampler(Dim::D3, Base::Float, false, false),
        "samplerCube" => sampler(Dim::Cube, Base::Float, false, false),
        "sampler2DShadow" => sampler(Dim::D2, Base::Float, true, false),
        "samplerCubeShadow" => sampler(Dim::Cube, Base::Float, true, false),
        "sampler2DArray" => sampler(Dim::D2, Base::Float, false, true),
        "sampler2DArrayShadow" => sampler(Dim::D2, Base::Float, true, true),
        "sampler2DRect" => sampler(Dim::Rect, Base::Float, false, false),
        "isampler2D" => sampler(Dim::D2, Base::Int, false, false),
        "isampler3D" => sampler(Dim::D3, Base::Int, false, false),
        "isamplerCube" => sampler(Dim::Cube, Base::Int, false, false),
        "isampler2DArray" => sampler(Dim::D2, Base::Int, false, true),
        "usampler2D" => sampler(Dim::D2, Base::Uint, false, false),
        "usampler3D" => sampler(Dim::D3, Base::Uint, false, false),
        "usamplerCube" => sampler(Dim::Cube, Base::Uint, false, false),
        "usampler2DArray" => sampler(Dim::D2, Base::Uint, false, true),
        _ => None,
    }
}

/// How a block's members are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packing {
    Std140,
    Std430,
}

/// A type in memory: its size and alignment in bytes, under a packing, its matrices column- or row-major.
pub fn layout(ty: &Ty, packing: Packing, row_major: bool) -> Result<(u32, u32), String> {
    Ok(match ty {
        Ty::Scalar(_) => (4, 4),
        Ty::Vector(_, n) => {
            let size = 4 * u32::from(*n);
            (size, if *n == 2 { 8 } else { 16 })
        }
        Ty::Matrix(c, r) => {
            // A matrix is an array of its columns (or rows, row-major).
            let (count, vector) = if row_major { (*r, *c) } else { (*c, *r) };
            let stride = matrix_stride(vector, packing);
            (stride * u32::from(count), stride)
        }
        Ty::Array(e, n) => {
            let stride = array_stride(e, packing, row_major)?;
            let (_, align) = layout(e, packing, row_major)?;
            let align = if packing == Packing::Std140 { align.max(16) } else { align };
            (stride * n.unwrap_or(0), align)
        }
        Ty::Struct(s) => {
            let mut offset = 0u32;
            let mut align = 4;
            for (_, m) in &s.members {
                let (size, a) = layout(m, packing, row_major)?;
                offset = offset.next_multiple_of(a) + size;
                align = align.max(a);
            }
            if packing == Packing::Std140 {
                align = align.max(16);
            }
            (offset.next_multiple_of(align), align)
        }
        Ty::Void | Ty::Sampler(_) => return Err(format!("{ty} isn't laid out in memory")),
    })
}

/// The bytes from one of a matrix's columns (or rows) to the next.
pub fn matrix_stride(vector: u8, packing: Packing) -> u32 {
    let (size, align) = (4 * u32::from(vector), if vector == 2 { 8 } else { 16 });
    let s = size.next_multiple_of(align);
    if packing == Packing::Std140 { s.next_multiple_of(16) } else { s }
}

/// The bytes from one of an array's elements to the next.
pub fn array_stride(element: &Ty, packing: Packing, row_major: bool) -> Result<u32, String> {
    let (size, align) = layout(element, packing, row_major)?;
    let align = if packing == Packing::Std140 { align.max(16) } else { align };
    Ok(size.next_multiple_of(align))
}

/// A struct member's offset.
pub fn member_offset(s: &StructTy, member: usize, packing: Packing, row_major: bool) -> Result<u32, String> {
    let mut offset = 0u32;
    for (i, (_, m)) in s.members.iter().enumerate() {
        let (size, a) = layout(m, packing, row_major)?;
        offset = offset.next_multiple_of(a);
        if i == member {
            return Ok(offset);
        }
        offset += size;
    }
    Err("no such member".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std140_and_std430() {
        let v3 = Ty::Vector(Base::Float, 3);
        assert_eq!(layout(&v3, Packing::Std140, false).unwrap(), (12, 16));
        let floats = Ty::Array(Box::new(FLOAT), Some(4));
        assert_eq!(layout(&floats, Packing::Std140, false).unwrap(), (64, 16));
        assert_eq!(layout(&floats, Packing::Std430, false).unwrap(), (16, 4));
        assert_eq!(layout(&Ty::Matrix(3, 3), Packing::Std140, false).unwrap(), (48, 16));
        assert_eq!(layout(&Ty::Matrix(2, 2), Packing::Std430, false).unwrap(), (16, 8));
        assert_eq!(layout(&Ty::Matrix(2, 2), Packing::Std140, false).unwrap(), (32, 16));
        let s = StructTy { name: "S".into(), members: alloc::vec![("a".into(), FLOAT), ("b".into(), v3.clone()), ("c".into(), FLOAT)] };
        assert_eq!(member_offset(&s, 1, Packing::Std430, false).unwrap(), 16);
        assert_eq!(member_offset(&s, 2, Packing::Std430, false).unwrap(), 28);
        assert_eq!(layout(&Ty::Struct(Arc::new(s)), Packing::Std140, false).unwrap(), (32, 16));
    }
}
