//! A module as the `.air` file Metal reads: LLVM bitcode in the typed-pointer format of the LLVM Apple's toolchain is
//! built on, inside Apple's wrapper header.
//!
//! The bitstream is LLVM's documented container: fields least significant bit first in 32-bit little-endian words,
//! blocks nested and word-aligned with their length up front. The records are laid out as LLVM's own writer lays
//! them out, with module format version 2 (names in a string table, function bodies found through the module's
//! symbol table). No abbreviations are used beyond the two the format needs, so the same module always gives the
//! same bytes.

use std::collections::HashMap;

use super::module::{Arg, Const, Func, KERNEL, Module, Op, Output, Stage, Ty, Value};

/// Apple's wrapper: magic, version 0, the bitcode's offset (20) and size, and CPU type -1, then the bitcode padded
/// with zeros to a multiple of 16 bytes, as LLVM's Darwin writer pads it (`air-validate` refuses an unpadded one).
pub(crate) fn wrap(bc: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + bc.len() + 15);
    for word in [0x0B17_C0DE, 0, 20, bc.len() as u32, 0xFFFF_FFFF] {
        out.extend_from_slice(&u32::to_le_bytes(word));
    }
    out.extend_from_slice(bc);
    while out.len() % 16 != 0 {
        out.push(0);
    }
    out
}

/// The module as a `.air` file.
pub(crate) fn encode(m: &mut Module) -> Vec<u8> {
    let named = metadata(m);
    let mut e = Encoder { m, w: Writer::default(), strtab: Vec::new(), attr_sets: Vec::new(), n_module: 0 };
    e.collect();
    let bc = e.encode(&named);
    wrap(&bc)
}

// ---- The bitstream ---------------------------------------------------------------------------------------------

const END_BLOCK: u64 = 0;
const ENTER_BLOCK: u64 = 1;
const DEFINE_ABBREV: u64 = 2;
const UNABBREV_RECORD: u64 = 3;
const FIRST_USER_ABBREV: u64 = 4;

const ENC_FIXED: u64 = 1;
const ENC_BLOB: u64 = 5;

/// One operand of an abbreviation: a literal, or an encoding and its width.
#[derive(Clone, Copy)]
enum AbbrevOp {
    Literal(u64),
    Fixed(u64),
    Blob,
}

#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
    cur: u64,
    nbits: u32,
    /// The current block's abbreviation ID width.
    width: u32,
    /// The enclosing blocks': their width, where their length word is, and their abbreviations.
    blocks: Vec<(u32, usize, Vec<Vec<AbbrevOp>>)>,
    abbrevs: Vec<Vec<AbbrevOp>>,
}

impl Writer {
    fn emit(&mut self, mut v: u64, mut n: u32) {
        while n > 0 {
            let take = n.min(32);
            self.cur |= (v & ((1u64 << take) - 1)) << self.nbits;
            self.nbits += take;
            v = if take == 64 { 0 } else { v >> take };
            n -= take;
            while self.nbits >= 32 {
                self.buf.extend_from_slice(&(self.cur as u32).to_le_bytes());
                self.cur >>= 32;
                self.nbits -= 32;
            }
        }
    }

    fn vbr(&mut self, mut v: u64, n: u32) {
        let hi = 1u64 << (n - 1);
        while v >= hi {
            self.emit((v & (hi - 1)) | hi, n);
            v >>= n - 1;
        }
        self.emit(v, n);
    }

    fn bit_pos(&self) -> u64 {
        self.buf.len() as u64 * 8 + self.nbits as u64
    }

    fn align32(&mut self) {
        if self.nbits > 0 {
            self.emit(0, 32 - self.nbits);
        }
    }

    fn enter(&mut self, id: u64, width: u32) {
        self.emit(ENTER_BLOCK, self.width);
        self.vbr(id, 8);
        self.vbr(width as u64, 4);
        self.align32();
        let abbrevs = std::mem::take(&mut self.abbrevs);
        self.blocks.push((self.width, self.buf.len(), abbrevs));
        self.emit(0, 32); // the length in words, patched on exit
        self.width = width;
    }

    fn exit(&mut self) {
        self.emit(END_BLOCK, self.width);
        self.align32();
        let (width, size_at, abbrevs) = self.blocks.pop().expect("a block is open");
        let words = ((self.buf.len() - size_at - 4) / 4) as u32;
        self.buf[size_at..size_at + 4].copy_from_slice(&words.to_le_bytes());
        self.width = width;
        self.abbrevs = abbrevs;
    }

    /// Defines an abbreviation in the current block, giving its ID.
    fn define(&mut self, ops: &[AbbrevOp]) -> u64 {
        self.emit(DEFINE_ABBREV, self.width);
        self.vbr(ops.len() as u64, 5);
        for op in ops {
            match *op {
                AbbrevOp::Literal(v) => {
                    self.emit(1, 1);
                    self.vbr(v, 8);
                }
                AbbrevOp::Fixed(width) => {
                    self.emit(0, 1);
                    self.emit(ENC_FIXED, 3);
                    self.vbr(width, 5);
                }
                AbbrevOp::Blob => {
                    self.emit(0, 1);
                    self.emit(ENC_BLOB, 3);
                }
            }
        }
        self.abbrevs.push(ops.to_vec());
        FIRST_USER_ABBREV + self.abbrevs.len() as u64 - 1
    }

    fn record(&mut self, code: u64, ops: &[u64]) {
        self.emit(UNABBREV_RECORD, self.width);
        self.vbr(code, 6);
        self.vbr(ops.len() as u64, 6);
        for op in ops {
            self.vbr(*op, 6);
        }
    }

    /// A record whose abbreviation is a literal code and one 32-bit field; gives the field's bit position, to patch.
    fn fixed32_record(&mut self, id: u64, value: u32) -> u64 {
        self.emit(id, self.width);
        let at = self.bit_pos();
        self.emit(value as u64, 32);
        at
    }

    /// A record whose abbreviation is a literal code and a blob.
    fn blob_record(&mut self, id: u64, blob: &[u8]) {
        self.emit(id, self.width);
        self.vbr(blob.len() as u64, 6);
        self.align32();
        for b in blob {
            self.emit(*b as u64, 8);
        }
        self.align32();
    }

    /// Overwrites a 32-bit field written at bit position `at`.
    fn patch32(&mut self, at: u64, v: u32) {
        for i in 0..32 {
            let bit = at + i;
            let mask = 1u8 << (bit % 8);
            let byte = &mut self.buf[(bit / 8) as usize];
            if (v >> i) & 1 == 1 {
                *byte |= mask;
            } else {
                *byte &= !mask;
            }
        }
    }
}

fn chars(s: &str) -> Vec<u64> {
    s.bytes().map(u64::from).collect()
}

/// LLVM's signed VBR operand: the magnitude shifted up, the sign in the low bit.
fn signed(v: i64) -> u64 {
    if v >= 0 { (v as u64) << 1 } else { ((v.unsigned_abs()) << 1) | 1 }
}

/// An alignment as LLVM stores it: log2 plus one, 0 for none.
fn log2_align(a: u32) -> u64 {
    if a == 0 { 0 } else { a.trailing_zeros() as u64 + 1 }
}

// ---- Block IDs and record codes ---------------------------------------------------------------------------------

const MODULE_BLOCK: u64 = 8;
const PARAMATTR_BLOCK: u64 = 9;
const PARAMATTR_GROUP_BLOCK: u64 = 10;
const CONSTANTS_BLOCK: u64 = 11;
const FUNCTION_BLOCK: u64 = 12;
const IDENTIFICATION_BLOCK: u64 = 13;
const VALUE_SYMTAB_BLOCK: u64 = 14;
const METADATA_BLOCK: u64 = 15;
const TYPE_BLOCK: u64 = 17;
const OPERAND_BUNDLE_TAGS_BLOCK: u64 = 21;
const METADATA_KIND_BLOCK: u64 = 22;
const STRTAB_BLOCK: u64 = 23;
const SYNC_SCOPE_NAMES_BLOCK: u64 = 26;

/// The tables LLVM writes into every module, as Apple's toolchain writes them. They're the LLVM context's, not the
/// module's, so they're fixed.
const METADATA_KINDS: &[&str] = &[
    "dbg", "tbaa", "prof", "fpmath", "range", "tbaa.struct", "invariant.load", "alias.scope", "noalias",
    "nontemporal", "llvm.mem.parallel_loop_access", "nonnull", "dereferenceable", "dereferenceable_or_null",
    "make.implicit", "unpredictable", "invariant.group", "align", "llvm.loop", "type", "section_prefix",
    "absolute_symbol", "associated", "callees", "irr_loop", "llvm.access.group", "callback",
    "llvm.preserve.access.index", "vcall_visibility", "noundef", "annotation",
];
const BUNDLE_TAGS: &[&str] =
    &["deopt", "funclet", "gc-transition", "cfguardtarget", "preallocated", "gc-live", "clang.arc.attachedcall", "ptrauth"];
const SYNC_SCOPES: &[&str] = &["singlethread", ""];

/// AIR's data layout, the same for every target.
const DATA_LAYOUT: &str = "e-p:64:64:64-i1:8:8-i8:8:8-i16:16:16-i32:32:32-i64:64:64-f32:32:32-f64:64:64-v16:16:16-v24:32:32-v32:32:32-v48:64:64-v64:64:64-v96:128:128-v128:128:128-v192:256:256-v256:256:256-v512:512:512-v1024:1024:1024-n8:16:32";

/// The AIR version a deployment target is written in: it follows the OS, not the MSL version (measured with xcrun).
pub(crate) fn air_version(os: (u32, u32)) -> (u32, u32) {
    match os.0 {
        26.. => (2, 8),
        15.. => (2, 7),
        14 => (2, 6),
        _ => (2, 5),
    }
}

fn triple(os: (u32, u32)) -> String {
    let (a, b) = air_version(os);
    format!("air64_v{a}{b}-apple-macosx{}.{}.0", os.0, os.1)
}

// ---- Metadata ---------------------------------------------------------------------------------------------------

/// A metadata operand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Md {
    Str(String),
    Val(Value),
    Node(Vec<Md>),
}

struct Named {
    name: &'static str,
    nodes: Vec<Vec<Md>>,
}

/// The limits xcrun records as module flags (behaviour 7, "max"), which Apple's tools want.
const MODULE_FLAGS: &[(&str, u32)] = &[
    ("air.max_device_buffers", 31),
    ("air.max_constant_buffers", 31),
    ("air.max_threadgroup_buffers", 31),
    ("air.max_textures", 128),
    ("air.max_read_write_textures", 8),
    ("air.max_samplers", 16),
];

/// What the module says about itself besides its code: the kernel and its argument descriptors, the versions, the
/// module flags and the compile options. Interns the constants it names.
fn metadata(m: &mut Module) -> Vec<Named> {
    fn int(m: &mut Module, v: u32) -> Md {
        Md::Val(m.i32(v))
    }
    let mut out = Vec::new();

    let flags = MODULE_FLAGS
        .iter()
        .map(|(name, v)| vec![int(m, 7), Md::Str((*name).into()), int(m, *v)])
        .collect();
    out.push(Named { name: "llvm.module.flags", nodes: flags });

    let params = m.funcs[KERNEL as usize].params.clone();
    let mut args = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let mut n = vec![int(m, i as u32)];
        let s = |t: &str| Md::Str(t.into());
        match p.arg {
            Arg::Buffer { location, writable } => {
                n.extend([s("air.buffer"), s("air.location_index"), int(m, location), int(m, 1)]);
                n.push(s(if writable { "air.read_write" } else { "air.read" }));
                n.extend([s("air.address_space"), int(m, 1), s("air.arg_type_size"), int(m, 1)]);
                n.extend([s("air.arg_type_align_size"), int(m, 1), s("air.arg_type_name"), s("uchar")]);
            }
            Arg::Builtin { name, type_name } => n.extend([s(name), s("air.arg_type_name"), s(type_name)]),
            Arg::Texture { location } => {
                n.extend([s("air.texture"), s("air.location_index"), int(m, location), int(m, 1), s("air.sample")]);
                n.extend([s("air.arg_type_name"), s("texture2d<float, sample>")]);
            }
            Arg::Sampler { location } => {
                n.extend([s("air.sampler"), s("air.location_index"), int(m, location), int(m, 1), s("air.arg_type_name"), s("sampler")]);
            }
            Arg::Interpolated { location, how, type_name } => {
                n.extend([s("air.fragment_input"), s(&format!("user(locn{location})"))]);
                n.extend(how.iter().map(|h| s(h)));
                n.extend([s("air.arg_type_name"), s(type_name)]);
            }
            Arg::FragmentPosition => n.extend([s("air.position"), s("air.center"), s("air.no_perspective"), s("air.arg_type_name"), s("float4")]),
        }
        n.extend([s("air.arg_name"), s(&p.name)]);
        args.push(Md::Node(n));
    }
    let s = |t: &str| Md::Str(t.into());
    let mut results = Vec::new();
    for (i, o) in m.outputs.clone().iter().enumerate() {
        let mut n = match o {
            Output::Position => vec![s("air.position"), s("air.arg_type_name"), s("float4")],
            Output::Vertex { location, type_name } => vec![s("air.vertex_output"), s(&format!("user(locn{location})")), s("air.arg_type_name"), s(type_name)],
            Output::Color { index, type_name } => vec![s("air.render_target"), int(m, *index), int(m, 0), s("air.arg_type_name"), s(type_name)],
            Output::Depth => vec![s("air.depth"), s("air.depth_qualifier"), s("air.any"), s("air.arg_type_name"), s("float")],
        };
        n.extend([s("air.arg_name"), s(&format!("out{i}"))]);
        results.push(Md::Node(n));
    }
    let entry = vec![Md::Val(Value::Func(KERNEL)), Md::Node(results), Md::Node(args)];
    let list = match m.stage {
        Stage::Kernel => "air.kernel",
        Stage::Vertex => "air.vertex",
        Stage::Fragment => "air.fragment",
    };
    out.push(Named { name: list, nodes: vec![entry] });

    let options = ["air.compile.denorms_disable", "air.compile.fast_math_disable", "air.compile.framebuffer_fetch_enable"];
    out.push(Named { name: "air.compile_options", nodes: options.iter().map(|o| vec![Md::Str((*o).into())]).collect() });

    let (major, minor) = air_version(m.os);
    let version = vec![int(m, major), int(m, minor), int(m, 0)];
    out.push(Named { name: "air.version", nodes: vec![version] });
    let (lmajor, lminor) = m.language;
    let language = vec![Md::Str("Metal".into()), int(m, lmajor), int(m, lminor), int(m, 0)];
    out.push(Named { name: "air.language_version", nodes: vec![language] });
    out
}

// ---- The module -------------------------------------------------------------------------------------------------

struct Encoder<'m> {
    m: &'m Module,
    w: Writer,
    strtab: Vec<u8>,
    /// Distinct function attribute sets; a function's attribute list is its set's index plus one.
    attr_sets: Vec<&'static [u64]>,
    /// Globals, functions and constants: the module-level values, numbered in that order.
    n_module: u32,
}

impl Encoder<'_> {
    fn collect(&mut self) {
        for f in &self.m.funcs {
            if !f.attrs.is_empty() && !self.attr_sets.contains(&f.attrs) {
                self.attr_sets.push(f.attrs);
            }
        }
        self.n_module = (self.m.globals.len() + self.m.funcs.len() + self.m.consts.len()) as u32;
    }

    /// A value's ID: module-level values first, then the function's parameters, then its instructions' results
    /// (`inst_ids`, by instruction).
    fn value_id(&self, v: Value, inst_ids: &[u32]) -> u32 {
        let (g, f) = (self.m.globals.len() as u32, self.m.funcs.len() as u32);
        match v {
            Value::Global(i) => i,
            Value::Func(i) => g + i,
            Value::Const(c) => g + f + c.0,
            Value::Param(p) => self.n_module + p,
            Value::Inst(i) => inst_ids[i as usize],
        }
    }

    fn is_void(&self, t: super::module::TypeId) -> bool {
        *self.m.get(t) == Ty::Void
    }

    fn name(&mut self, s: &str) -> (u64, u64) {
        let at = self.strtab.len() as u64;
        self.strtab.extend_from_slice(s.as_bytes());
        (at, s.len() as u64)
    }

    fn encode(&mut self, named: &[Named]) -> Vec<u8> {
        self.w.width = 2;
        for b in *b"BC" {
            self.w.emit(b as u64, 8);
        }
        for nibble in [0x0, 0xC, 0xE, 0xD] {
            self.w.emit(nibble, 4);
        }
        // Offsets into the stream (the symbol table's, each function block's) count words from one word before the
        // identification block, which is where the magic starts.
        let start = self.w.bit_pos() - 32;

        self.w.enter(IDENTIFICATION_BLOCK, 5);
        self.w.record(1, &chars("carbon gpu"));
        self.w.record(2, &[0]); // epoch
        self.w.exit();

        self.w.enter(MODULE_BLOCK, 3);
        self.w.record(1, &[2]); // version
        self.attributes();
        self.type_table();
        self.w.record(2, &chars(&triple(self.m.os)));
        self.w.record(3, &chars(DATA_LAYOUT));
        self.globals();
        self.function_records();
        let vst_abbrev = self.w.define(&[AbbrevOp::Literal(13), AbbrevOp::Fixed(32)]);
        let vst_patch = self.w.fixed32_record(vst_abbrev, 0);
        self.constants();
        self.fixed_table(METADATA_KIND_BLOCK, 6, METADATA_KINDS, true);
        self.metadata(named);
        self.fixed_table(OPERAND_BUNDLE_TAGS_BLOCK, 1, BUNDLE_TAGS, false);
        self.fixed_table(SYNC_SCOPE_NAMES_BLOCK, 1, SYNC_SCOPES, false);

        let mut offsets = Vec::new();
        for (i, f) in self.m.funcs.iter().enumerate() {
            if let Some(body) = &f.body {
                offsets.push((i as u32, (self.w.bit_pos() - start) / 32));
                self.function(f, body);
            }
        }

        let at = ((self.w.bit_pos() - start) / 32) as u32;
        self.w.patch32(vst_patch, at);
        self.w.enter(VALUE_SYMTAB_BLOCK, 4);
        let globals = self.m.globals.len() as u64;
        for (f, off) in offsets {
            self.w.record(3, &[globals + f as u64, off]); // VST_CODE_FNENTRY
        }
        self.w.exit();
        self.w.exit(); // the module

        self.w.enter(STRTAB_BLOCK, 3);
        let blob = self.w.define(&[AbbrevOp::Literal(1), AbbrevOp::Blob]);
        let strtab = std::mem::take(&mut self.strtab);
        self.w.blob_record(blob, &strtab);
        self.w.exit();

        assert!(self.w.nbits == 0 && self.w.blocks.is_empty(), "bitcode: unfinished stream");
        std::mem::take(&mut self.w.buf)
    }

    fn fixed_table(&mut self, block: u64, code: u64, names: &[&str], numbered: bool) {
        self.w.enter(block, 3);
        for (i, n) in names.iter().enumerate() {
            let mut ops = if numbered { vec![i as u64] } else { Vec::new() };
            ops.extend(chars(n));
            self.w.record(code, &ops);
        }
        self.w.exit();
    }

    fn attributes(&mut self) {
        if self.attr_sets.is_empty() {
            return;
        }
        self.w.enter(PARAMATTR_GROUP_BLOCK, 3);
        for (i, set) in self.attr_sets.clone().iter().enumerate() {
            let mut ops = vec![i as u64 + 1, 0xFFFF_FFFF]; // the group, on the function itself
            let mut codes = set.to_vec();
            codes.sort_unstable();
            for c in codes {
                ops.extend([0, c]); // an enum attribute
            }
            self.w.record(3, &ops);
        }
        self.w.exit();
        self.w.enter(PARAMATTR_BLOCK, 3);
        for i in 0..self.attr_sets.len() {
            self.w.record(2, &[i as u64 + 1]);
        }
        self.w.exit();
    }

    fn type_table(&mut self) {
        self.w.enter(TYPE_BLOCK, 4);
        self.w.record(1, &[self.m.types.len() as u64]);
        for t in &self.m.types {
            match t {
                Ty::Void => self.w.record(2, &[]),
                Ty::Float => self.w.record(3, &[]),
                Ty::Int(bits) => self.w.record(7, &[*bits as u64]),
                Ty::Ptr(elem, space) => self.w.record(8, &[elem.0 as u64, *space as u64]),
                Ty::Array(elem, n) => self.w.record(11, &[*n, elem.0 as u64]),
                Ty::Vector(elem, n) => self.w.record(12, &[*n as u64, elem.0 as u64]),
                Ty::Func(ret, params) => {
                    let mut ops = vec![0, ret.0 as u64]; // not vararg
                    ops.extend(params.iter().map(|p| p.0 as u64));
                    self.w.record(21, &ops);
                }
                Ty::Struct(elems, packed) => {
                    let mut ops = vec![*packed as u64];
                    ops.extend(elems.iter().map(|e| e.0 as u64));
                    self.w.record(18, &ops); // STRUCT_ANON
                }
                Ty::Opaque(name) => {
                    self.w.record(19, &chars(name)); // STRUCT_NAME, for the next named struct
                    self.w.record(6, &[0]); // OPAQUE
                }
            }
        }
        self.w.exit();
    }

    fn globals(&mut self) {
        for g in &self.m.globals {
            let (off, size) = self.name(&g.name);
            let flags = (1 << 1) | ((g.space as u64) << 2); // an explicit type, and the address space
            let ops =
                [off, size, g.elem.0 as u64, flags, g.init.0 as u64 + 1 + self.const_base() as u64, 3, log2_align(g.align), 0];
            self.w.record(7, &ops); // internal linkage (3)
        }
    }

    /// The first constant's value ID.
    fn const_base(&self) -> u32 {
        (self.m.globals.len() + self.m.funcs.len()) as u32
    }

    fn function_records(&mut self) {
        for f in &self.m.funcs {
            let (off, size) = self.name(&f.name);
            let proto = u64::from(f.body.is_none());
            let attrs = self.attr_sets.iter().position(|s| *s == f.attrs).map_or(0, |i| i as u64 + 1);
            // [strtab offset, size, type, cc, isproto, linkage, paramattrs, alignment, section, visibility, gc,
            //  unnamed_addr, prologue, dllstorage, comdat, prefix, personality, dso_local]
            let mut ops = vec![off, size, f.sig.0 as u64, 0, proto, 0, attrs];
            ops.extend([0; 11]);
            self.w.record(8, &ops);
        }
    }

    fn constants(&mut self) {
        if self.m.consts.is_empty() {
            return;
        }
        let base = self.const_base() as u64;
        self.w.enter(CONSTANTS_BLOCK, 4);
        let mut cur = None;
        for c in &self.m.consts {
            let t = c.ty();
            if cur != Some(t) {
                self.w.record(1, &[t.0 as u64]); // SETTYPE
                cur = Some(t);
            }
            match c {
                Const::Undef(_) => self.w.record(3, &[]),
                Const::Null(_) => self.w.record(2, &[]),
                Const::Int(t, bits) => {
                    let w = match self.m.get(*t) {
                        Ty::Int(w) => *w,
                        _ => 64,
                    };
                    let v = if w >= 64 { *bits as i64 } else { ((*bits << (64 - w)) as i64) >> (64 - w) };
                    self.w.record(4, &[signed(v)]);
                }
                Const::Float(_, bits) => self.w.record(6, &[*bits as u64]),
                Const::Vector(_, elems) => {
                    let ids: Vec<u64> = elems.iter().map(|e| base + e.0 as u64).collect();
                    self.w.record(7, &ids); // AGGREGATE
                }
            }
        }
        self.w.exit();
    }

    /// Strings first, then values, then nodes after their operands, then the named metadata. A node's operands are
    /// their IDs plus one.
    fn metadata(&mut self, named: &[Named]) {
        let mut strs: Vec<String> = Vec::new();
        let mut str_id: HashMap<String, u64> = HashMap::new();
        let mut vals: Vec<Value> = Vec::new();
        let mut val_id: HashMap<Value, u64> = HashMap::new();
        let mut nodes: Vec<Vec<Md>> = Vec::new();
        let mut node_id: HashMap<Vec<Md>, usize> = HashMap::new();

        fn walk(
            n: &[Md],
            strs: &mut Vec<String>,
            str_id: &mut HashMap<String, u64>,
            vals: &mut Vec<Value>,
            val_id: &mut HashMap<Value, u64>,
            nodes: &mut Vec<Vec<Md>>,
            node_id: &mut HashMap<Vec<Md>, usize>,
        ) {
            if node_id.contains_key(n) {
                return;
            }
            for op in n {
                match op {
                    Md::Str(s) if !str_id.contains_key(s) => {
                        str_id.insert(s.clone(), strs.len() as u64);
                        strs.push(s.clone());
                    }
                    Md::Val(v) if !val_id.contains_key(v) => {
                        val_id.insert(*v, vals.len() as u64);
                        vals.push(*v);
                    }
                    Md::Node(c) => walk(c, strs, str_id, vals, val_id, nodes, node_id),
                    _ => {}
                }
            }
            node_id.insert(n.to_vec(), nodes.len());
            nodes.push(n.to_vec());
        }
        for nm in named {
            for n in &nm.nodes {
                walk(n, &mut strs, &mut str_id, &mut vals, &mut val_id, &mut nodes, &mut node_id);
            }
        }
        let first_node = (strs.len() + vals.len()) as u64;
        let id_of = |n: &[Md]| first_node + node_id[n] as u64;

        self.w.enter(METADATA_BLOCK, 4);
        for s in &strs {
            self.w.record(1, &chars(s)); // METADATA_STRING_OLD
        }
        for v in &vals {
            let ty = self.m.type_of(*v);
            let id = self.value_id(*v, &[]);
            self.w.record(2, &[ty.0 as u64, id as u64]); // METADATA_VALUE
        }
        for n in &nodes {
            let ops: Vec<u64> = n
                .iter()
                .map(|op| {
                    1 + match op {
                        Md::Str(s) => str_id[s],
                        Md::Val(v) => strs.len() as u64 + val_id[v],
                        Md::Node(c) => id_of(c),
                    }
                })
                .collect();
            self.w.record(3, &ops); // METADATA_NODE
        }
        for nm in named {
            self.w.record(4, &chars(nm.name)); // METADATA_NAME
            let ids: Vec<u64> = nm.nodes.iter().map(|n| id_of(n)).collect();
            self.w.record(10, &ids); // METADATA_NAMED_NODE
        }
        self.w.exit();
    }

    // ---- Function bodies ----------------------------------------------------------------------------------------

    fn function(&mut self, f: &Func, body: &super::module::Body) {
        let mut ids = vec![u32::MAX; body.insts.len()];
        let mut n = self.n_module + f.params.len() as u32;
        for block in &body.blocks {
            for &i in block {
                if !self.is_void(body.insts[i as usize].ty) {
                    ids[i as usize] = n;
                    n += 1;
                }
            }
        }
        let mut next = self.n_module + f.params.len() as u32;

        self.w.enter(FUNCTION_BLOCK, 4);
        self.w.record(1, &[body.blocks.len() as u64]); // DECLAREBLOCKS
        for block in &body.blocks {
            for &i in block {
                let inst = &body.insts[i as usize];
                self.inst(inst, next, &ids);
                if !self.is_void(inst.ty) {
                    next += 1;
                }
            }
        }
        self.w.exit();
    }

    fn inst(&mut self, inst: &super::module::Inst, next: u32, ids: &[u32]) {
        let id = |v: Value| self.value_id(v, ids);
        // An operand as the distance back from this instruction, wrapping for a forward reference.
        let rel = |v: Value| (next.wrapping_sub(id(v))) as u64;
        // `rel`, followed by the type for a forward reference.
        let rel_ty = |v: Value| -> Vec<u64> {
            if id(v) >= next { vec![rel(v), self.m.type_of(v).0 as u64] } else { vec![rel(v)] }
        };
        let mut ops: Vec<u64> = Vec::new();
        let (code, ops) = match &inst.op {
            Op::Binary(opcode, a, b) => {
                ops.extend(rel_ty(*a));
                ops.extend([rel(*b), *opcode]);
                (2, ops)
            }
            Op::Cast(opcode, v) => {
                ops.extend(rel_ty(*v));
                ops.extend([inst.ty.0 as u64, *opcode]);
                (3, ops)
            }
            Op::FNeg(v) => {
                ops.extend(rel_ty(*v));
                ops.push(0);
                (56, ops)
            }
            Op::Cmp(pred, a, b) => {
                ops.extend(rel_ty(*a));
                ops.extend([rel(*b), *pred]);
                (28, ops)
            }
            Op::Select(c, a, b) => {
                ops.extend(rel_ty(*a));
                ops.push(rel(*b));
                ops.extend(rel_ty(*c));
                (29, ops) // VSELECT
            }
            Op::Alloca { elem, count, align } => {
                // The count is an absolute value ID, with its type; bit 6 says the type is explicit.
                let ty = self.m.type_of(*count).0 as u64;
                (19, vec![elem.0 as u64, ty, id(*count) as u64, log2_align(*align) | (1 << 6)])
            }
            Op::Load { ptr, align } => {
                ops.extend(rel_ty(*ptr));
                ops.extend([inst.ty.0 as u64, log2_align(*align), 0]);
                (20, ops)
            }
            Op::Store { ptr, value, align } => {
                ops.extend(rel_ty(*ptr));
                ops.extend(rel_ty(*value));
                ops.extend([log2_align(*align), 0]);
                (44, ops)
            }
            Op::Gep { elem, ptr, index } => {
                ops.extend([0, elem.0 as u64]); // not inbounds
                ops.extend(rel_ty(*ptr));
                ops.extend(rel_ty(*index));
                (43, ops)
            }
            Op::ExtractElement(v, i) => {
                ops.extend(rel_ty(*v));
                ops.extend(rel_ty(*i));
                (6, ops)
            }
            Op::InsertElement(v, x, i) => {
                ops.extend(rel_ty(*v));
                ops.push(rel(*x));
                ops.extend(rel_ty(*i));
                (7, ops)
            }
            Op::ExtractValue(v, i) => {
                ops.extend(rel_ty(*v));
                ops.push(*i as u64);
                (26, ops)
            }
            Op::InsertValue(v, x, i) => {
                ops.extend(rel_ty(*v));
                ops.extend(rel_ty(*x));
                ops.push(*i as u64);
                (27, ops)
            }
            Op::Call { func, args } => {
                let callee = Value::Func(*func);
                ops.extend([0, 1 << 15, self.m.funcs[*func as usize].sig.0 as u64, rel(callee)]); // explicit type
                ops.extend(args.iter().map(|a| rel(*a)));
                (34, ops)
            }
            Op::Br(b) => (11, vec![*b as u64]),
            Op::CondBr(c, t, f) => (11, vec![*t as u64, *f as u64, rel(*c)]),
            Op::Ret => (10, Vec::new()),
            Op::RetValue(v) => (10, rel_ty(*v)),
        };
        self.w.record(code, &ops);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vbr_and_fields_pack_least_significant_bit_first() {
        let mut w = Writer::default();
        w.emit(0b101, 3);
        w.vbr(100, 6); // 100 = 0b1100100: chunks 00100|1, 00011|0
        w.align32();
        let word = u32::from_le_bytes(w.buf[..4].try_into().unwrap());
        assert_eq!(word & 0b111, 0b101);
        assert_eq!((word >> 3) & 0x3F, 0b100100);
        assert_eq!((word >> 9) & 0x3F, 0b000011);
    }

    #[test]
    fn the_wrapper_is_padded_to_16_bytes() {
        let out = wrap(&[1, 2, 3, 4]);
        assert_eq!(out.len(), 32);
        assert_eq!(&out[..4], &0x0B17_C0DEu32.to_le_bytes());
        assert_eq!(u32::from_le_bytes(out[12..16].try_into().unwrap()), 4);
    }

    #[test]
    fn signed_operands() {
        assert_eq!(signed(0), 0);
        assert_eq!(signed(3), 6);
        assert_eq!(signed(-1), 3);
    }
}
