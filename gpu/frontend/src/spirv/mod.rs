//! SPIR-V to gpu's IR: Vulkan's compute kernels and vertex and fragment shaders, as glslang and other compilers write
//! them.
//!
//! Reading is two passes. The first ([`Spirv::parse`]) collects the module's types, constants, decorations, global
//! variables, entry points and each function's blocks. The second (`translate`) walks one entry point's blocks
//! from its first, rebuilding structured control flow from SPIR-V's merge annotations (`OpSelectionMerge`,
//! `OpLoopMerge`), turning phis into variables, and lowering access chains to byte offsets with each storage class's
//! layout: explicit (`Offset`, `ArrayStride`) for buffers, our own for workgroup and private memory.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

mod op;
mod translate;

pub(crate) fn err<T>(message: String) -> Result<T, Error> {
    Err(Error { message })
}

/// Reads a SPIR-V module (as its file's bytes) and translates every compute, vertex and fragment entry point in it.
pub fn read(bytes: &[u8]) -> Result<gpu_ir::Module, Error> {
    if !bytes.len().is_multiple_of(4) {
        return err(format!("{} bytes isn't a whole number of SPIR-V words", bytes.len()));
    }
    // SPIR-V files are little-endian, as every host Carbon runs on is.
    let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
    read_words(&words)
}

/// Reads a SPIR-V module given as words, as Vulkan's `VkShaderModuleCreateInfo` gives it.
pub fn read_words(words: &[u32]) -> Result<gpu_ir::Module, Error> {
    let spirv = Spirv::parse(words)?;
    let mut module = gpu_ir::Module::default();
    for entry in &spirv.entries {
        if !matches!(entry.model, op::MODEL_GL_COMPUTE | op::MODEL_VERTEX | op::MODEL_FRAGMENT) {
            continue;
        }
        let ir = translate::entry(&spirv, entry)?;
        gpu_ir::validate(&ir)?;
        module.entries.push(ir);
    }
    if module.entries.is_empty() {
        return err("the module has no compute, vertex or fragment entry point".into());
    }
    Ok(module)
}

/// One instruction: its opcode and its operand words.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inst<'a> {
    pub op: u16,
    pub args: &'a [u32],
}

impl Inst<'_> {
    pub fn arg(&self, i: usize) -> Result<u32, Error> {
        match self.args.get(i) {
            Some(&w) => Ok(w),
            None => err(format!("{} has no operand {i}", op::name(self.op))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Type {
    Void,
    Bool,
    Int { width: u32, signed: bool },
    Float { width: u32 },
    Vector { component: u32, count: u32 },
    /// The length is a constant's id.
    Array { element: u32, length: u32 },
    RuntimeArray { element: u32 },
    Struct { members: Vec<u32> },
    Pointer { class: u32, pointee: u32 },
    Function,
    /// `count` columns of the vector type `column`.
    Matrix { column: u32, count: u32 },
    /// A sampled 2D image of floats; other images are [`Type::Other`].
    Image,
    Sampler,
    SampledImage,
    /// Everything else (storage images, other dimensions, opaque types): an error if a program uses one.
    Other(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Constant {
    /// Up to 64 bits, as SPIR-V gives them (low word first).
    Scalar(u64),
    Composite(Vec<u32>),
    /// `OpConstantNull`: zero of its type.
    Null,
}

#[derive(Debug, Clone)]
pub(crate) struct Global {
    pub ty: u32,
    pub class: u32,
    pub init: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct EntryPoint {
    pub model: u32,
    pub function: u32,
    pub name: String,
}

/// What `OpTypeImage`'s operands make, given its sampled type is a 32-bit float: [`Type::Image`] for a 2D image
/// that isn't depth, arrayed or multisampled and is sampled; anything else isn't translated yet.
fn image_type(a: &[u32]) -> Type {
    match a {
        [_, _, op::DIM_2D, 0 | 2, 0, 0, 1, ..] => Type::Image,
        _ => Type::Other(op::TYPE_IMAGE),
    }
}

/// A block: its instructions after `OpLabel`, up to and including its terminator.
#[derive(Debug, Clone)]
pub(crate) struct Block<'a> {
    pub insts: Vec<Inst<'a>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Function<'a> {
    /// Its blocks, the first one first.
    pub order: Vec<u32>,
    pub blocks: BTreeMap<u32, Block<'a>>,
}

/// Decorations on one id or member: each one's number and its operands.
pub(crate) type Decorations<'a> = Vec<(u32, &'a [u32])>;

/// What the first pass collected.
pub(crate) struct Spirv<'a> {
    pub types: BTreeMap<u32, Type>,
    pub constants: BTreeMap<u32, (u32, Constant)>,
    /// Every decoration on an id: (decoration, its operands).
    pub decorations: BTreeMap<u32, Decorations<'a>>,
    pub member_decorations: BTreeMap<(u32, u32), Decorations<'a>>,
    pub globals: BTreeMap<u32, Global>,
    pub entries: Vec<EntryPoint>,
    /// Each function's local size from `OpExecutionMode LocalSize`.
    pub local_size: BTreeMap<u32, [u32; 3]>,
    /// Each function's other execution modes, by number.
    pub modes: BTreeMap<u32, Vec<u32>>,
    pub functions: BTreeMap<u32, Function<'a>>,
    /// Ids of `OpExtInstImport "GLSL.std.450"`.
    pub glsl: Vec<u32>,
    /// Constants and globals in the order they were declared.
    pub declared: Vec<u32>,
}

/// A literal string's text and how many words it takes.
pub(crate) fn string(words: &[u32]) -> (String, usize) {
    let mut bytes = Vec::new();
    let mut used = 0;
    'words: for w in words {
        used += 1;
        for b in w.to_le_bytes() {
            if b == 0 {
                break 'words;
            }
            bytes.push(b);
        }
    }
    (String::from_utf8_lossy(&bytes).into_owned(), used)
}

impl<'a> Spirv<'a> {
    pub fn parse(words: &'a [u32]) -> Result<Spirv<'a>, Error> {
        if words.len() < 5 {
            return err(format!("{} words isn't a SPIR-V module", words.len()));
        }
        if words[0] != 0x0723_0203 {
            return err(format!("magic {:#010x} isn't SPIR-V's", words[0]));
        }
        let mut s = Spirv {
            types: BTreeMap::new(),
            constants: BTreeMap::new(),
            decorations: BTreeMap::new(),
            member_decorations: BTreeMap::new(),
            globals: BTreeMap::new(),
            entries: Vec::new(),
            local_size: BTreeMap::new(),
            modes: BTreeMap::new(),
            functions: BTreeMap::new(),
            glsl: Vec::new(),
            declared: Vec::new(),
        };
        let mut at = 5;
        let mut function: Option<(u32, Function<'a>)> = None;
        let mut block: Option<(u32, Vec<Inst<'a>>)> = None;
        while at < words.len() {
            let count = (words[at] >> 16) as usize;
            let op = words[at] as u16;
            if count == 0 || at + count > words.len() {
                return err(format!("an instruction at word {at} runs past the module's end"));
            }
            let inst = Inst { op, args: &words[at + 1..at + count] };
            at += count;
            if let Some((_, insts)) = &mut block {
                insts.push(inst);
                if op::is_terminator(op) {
                    let (label, insts) = block.take().unwrap_or_default();
                    let Some((_, f)) = &mut function else { return err("a block outside a function".into()) };
                    f.order.push(label);
                    f.blocks.insert(label, Block { insts });
                }
                continue;
            }
            s.first_pass(inst, &mut function, &mut block)?;
        }
        Ok(s)
    }

    fn first_pass(
        &mut self,
        inst: Inst<'a>,
        function: &mut Option<(u32, Function<'a>)>,
        block: &mut Option<(u32, Vec<Inst<'a>>)>,
    ) -> Result<(), Error> {
        let a = inst.args;
        match inst.op {
            op::EXT_INST_IMPORT => {
                if string(&a[1..]).0 == "GLSL.std.450" {
                    self.glsl.push(inst.arg(0)?);
                }
            }
            op::ENTRY_POINT => {
                let (name, _) = string(&a[2..]);
                self.entries.push(EntryPoint { model: inst.arg(0)?, function: inst.arg(1)?, name });
            }
            op::EXECUTION_MODE => {
                if inst.arg(1)? == op::MODE_LOCAL_SIZE {
                    self.local_size.insert(inst.arg(0)?, [inst.arg(2)?, inst.arg(3)?, inst.arg(4)?]);
                } else {
                    self.modes.entry(inst.arg(0)?).or_default().push(inst.arg(1)?);
                }
            }
            op::DECORATE => self.decorations.entry(inst.arg(0)?).or_default().push((inst.arg(1)?, &a[2..])),
            op::MEMBER_DECORATE => {
                self.member_decorations.entry((inst.arg(0)?, inst.arg(1)?)).or_default().push((inst.arg(2)?, &a[3..]))
            }
            op::TYPE_VOID => self.ty(inst, Type::Void)?,
            op::TYPE_BOOL => self.ty(inst, Type::Bool)?,
            op::TYPE_INT => self.ty(inst, Type::Int { width: inst.arg(1)?, signed: inst.arg(2)? != 0 })?,
            op::TYPE_FLOAT => self.ty(inst, Type::Float { width: inst.arg(1)? })?,
            op::TYPE_VECTOR => self.ty(inst, Type::Vector { component: inst.arg(1)?, count: inst.arg(2)? })?,
            op::TYPE_ARRAY => self.ty(inst, Type::Array { element: inst.arg(1)?, length: inst.arg(2)? })?,
            op::TYPE_RUNTIME_ARRAY => self.ty(inst, Type::RuntimeArray { element: inst.arg(1)? })?,
            op::TYPE_STRUCT => self.ty(inst, Type::Struct { members: a[1..].to_vec() })?,
            op::TYPE_POINTER => self.ty(inst, Type::Pointer { class: inst.arg(1)?, pointee: inst.arg(2)? })?,
            op::TYPE_FUNCTION => self.ty(inst, Type::Function)?,
            op::TYPE_MATRIX => self.ty(inst, Type::Matrix { column: inst.arg(1)?, count: inst.arg(2)? })?,
            op::TYPE_IMAGE => {
                let float = matches!(self.types.get(&inst.arg(1)?), Some(Type::Float { width: 32 }));
                self.ty(inst, if float { image_type(a) } else { Type::Other(inst.op) })?
            }
            op::TYPE_SAMPLER => self.ty(inst, Type::Sampler)?,
            op::TYPE_SAMPLED_IMAGE => {
                let image = self.types.get(&inst.arg(1)?);
                let ty = if image == Some(&Type::Image) { Type::SampledImage } else { Type::Other(inst.op) };
                self.ty(inst, ty)?
            }
            op::TYPE_OPAQUE => self.ty(inst, Type::Other(inst.op))?,
            op::CONSTANT_TRUE | op::SPEC_CONSTANT_TRUE => self.constant(inst, Constant::Scalar(1))?,
            op::CONSTANT_FALSE | op::SPEC_CONSTANT_FALSE => self.constant(inst, Constant::Scalar(0))?,
            op::CONSTANT | op::SPEC_CONSTANT => {
                let low = inst.arg(2)? as u64;
                let high = a.get(3).copied().unwrap_or(0) as u64;
                self.constant(inst, Constant::Scalar(low | (high << 32)))?
            }
            op::CONSTANT_COMPOSITE | op::SPEC_CONSTANT_COMPOSITE => {
                self.constant(inst, Constant::Composite(a[2..].to_vec()))?
            }
            op::CONSTANT_NULL => self.constant(inst, Constant::Null)?,
            op::SPEC_CONSTANT_OP => return err("OpSpecConstantOp isn't translated yet".into()),
            op::VARIABLE if function.is_none() => {
                let id = inst.arg(1)?;
                self.globals.insert(id, Global { ty: inst.arg(0)?, class: inst.arg(2)?, init: a.get(3).copied() });
                self.declared.push(id);
            }
            op::FUNCTION => {
                *function = Some((inst.arg(1)?, Function { order: Vec::new(), blocks: BTreeMap::new() }));
            }
            op::FUNCTION_END => {
                if let Some((id, f)) = function.take() {
                    self.functions.insert(id, f);
                }
            }
            op::LABEL => *block = Some((inst.arg(0)?, Vec::new())),
            op::FUNCTION_PARAMETER => return err("functions with parameters aren't translated yet".into()),
            op::UNDEF if function.is_none() => self.constant(inst, Constant::Null)?,
            _ => {}
        }
        Ok(())
    }

    fn ty(&mut self, inst: Inst<'a>, ty: Type) -> Result<(), Error> {
        self.types.insert(inst.arg(0)?, ty);
        Ok(())
    }

    fn constant(&mut self, inst: Inst<'a>, c: Constant) -> Result<(), Error> {
        let id = inst.arg(1)?;
        self.constants.insert(id, (inst.arg(0)?, c));
        self.declared.push(id);
        Ok(())
    }

    pub fn ty_of(&self, id: u32) -> Result<&Type, Error> {
        match self.types.get(&id) {
            Some(t) => Ok(t),
            None => err(format!("%{id} isn't a type")),
        }
    }

    /// The first operand of decoration `which` on `id`, if it has it.
    pub fn decoration(&self, id: u32, which: u32) -> Option<u32> {
        let decs = self.decorations.get(&id)?;
        decs.iter().find(|(d, _)| *d == which).map(|(_, args)| args.first().copied().unwrap_or(0))
    }

    pub fn has_decoration(&self, id: u32, which: u32) -> bool {
        self.decorations.get(&id).is_some_and(|d| d.iter().any(|(w, _)| *w == which))
    }

    pub fn member_decoration(&self, id: u32, member: u32, which: u32) -> Option<u32> {
        let decs = self.member_decorations.get(&(id, member))?;
        decs.iter().find(|(d, _)| *d == which).map(|(_, args)| args.first().copied().unwrap_or(0))
    }

    /// A constant's value as an integer, for array lengths, struct indices and lanes.
    pub fn const_int(&self, id: u32) -> Option<u64> {
        match self.constants.get(&id)? {
            (_, Constant::Scalar(v)) => Some(*v),
            (_, Constant::Null) => Some(0),
            _ => None,
        }
    }
}
