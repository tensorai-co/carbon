//! The IR as text, for traces and test failures: `%3: u32 = add %1, %2`.

use core::fmt::{self, Write};

use crate::{Block, Entry, Lod, Op, Stage, Stmt, Type};

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [x, y, z] = self.workgroup_size;
        match self.stage {
            Stage::Compute => writeln!(f, "entry {} [{x}, {y}, {z}]", self.name)?,
            stage => writeln!(f, "entry {} {stage:?}", self.name)?,
        }
        for (i, b) in self.bindings.iter().enumerate() {
            writeln!(f, "  binding {i}: set {} binding {} {:?}", b.set, b.binding, b.kind)?;
        }
        for (i, io) in self.inputs.iter().enumerate() {
            writeln!(f, "  input {i}: {:?} {} {:?}", io.slot, TypeName(io.ty), io.interpolation)?;
        }
        for (i, io) in self.outputs.iter().enumerate() {
            writeln!(f, "  output {i}: {:?} {} {:?}", io.slot, TypeName(io.ty), io.interpolation)?;
        }
        for (i, r) in self.workgroup.iter().enumerate() {
            writeln!(f, "  workgroup {i}: {} bytes, align {}", r.size, r.align)?;
        }
        for (i, r) in self.private.iter().enumerate() {
            writeln!(f, "  private {i}: {} bytes, align {}", r.size, r.align)?;
        }
        for (i, ty) in self.vars.iter().enumerate() {
            writeln!(f, "  var $v{i}: {}", TypeName(*ty))?;
        }
        block(f, self, &self.body, 1)
    }
}

struct TypeName(Type);

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Type::Scalar(s) => write!(f, "{s:?}"),
            Type::Vector(s, n) => write!(f, "{s:?}x{n}"),
            Type::Ptr(space) => write!(f, "ptr<{space:?}>"),
        }
    }
}

fn indent(f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
    for _ in 0..depth {
        f.write_str("  ")?;
    }
    Ok(())
}

fn block(f: &mut fmt::Formatter<'_>, entry: &Entry, stmts: &Block, depth: usize) -> fmt::Result {
    for stmt in stmts {
        indent(f, depth)?;
        match stmt {
            Stmt::Let(v, op) => {
                write!(f, "{v}: {} = ", TypeName(entry.ty(*v)))?;
                op_text(f, op)?;
                f.write_char('\n')?;
            }
            Stmt::Store(p, v) => writeln!(f, "store {p}, {v}")?,
            Stmt::Write(var, v) => writeln!(f, "{var} = {v}")?,
            Stmt::If { cond, then, otherwise } => {
                writeln!(f, "if {cond} {{")?;
                block(f, entry, then, depth + 1)?;
                if !otherwise.is_empty() {
                    indent(f, depth)?;
                    writeln!(f, "}} else {{")?;
                    block(f, entry, otherwise, depth + 1)?;
                }
                indent(f, depth)?;
                writeln!(f, "}}")?;
            }
            Stmt::Loop { body, continuing, break_if } => {
                writeln!(f, "loop {{")?;
                block(f, entry, body, depth + 1)?;
                if !continuing.is_empty() || break_if.is_some() {
                    indent(f, depth)?;
                    writeln!(f, "}} continuing {{")?;
                    block(f, entry, continuing, depth + 1)?;
                    if let Some(c) = break_if {
                        indent(f, depth + 1)?;
                        writeln!(f, "break if {c}")?;
                    }
                }
                indent(f, depth)?;
                writeln!(f, "}}")?;
            }
            Stmt::Break => writeln!(f, "break")?,
            Stmt::Continue => writeln!(f, "continue")?,
            Stmt::Return => writeln!(f, "return")?,
            Stmt::Barrier(b) => writeln!(f, "barrier {b:?}")?,
            Stmt::Output(n, v) => writeln!(f, "output {n} = {v}")?,
            Stmt::Discard => writeln!(f, "discard")?,
        }
    }
    Ok(())
}

fn op_text(f: &mut fmt::Formatter<'_>, op: &Op) -> fmt::Result {
    match op {
        Op::Const(s, bits) => write!(f, "const {s:?} {bits:#x}"),
        Op::Builtin(b) => write!(f, "builtin {b:?}"),
        Op::Binding(n) => write!(f, "binding {n}"),
        Op::Workgroup(n) => write!(f, "workgroup {n}"),
        Op::Private(n) => write!(f, "private {n}"),
        Op::Offset(p, n) => write!(f, "offset {p}, {n}"),
        Op::Load(p, ty) => write!(f, "load {} {p}", TypeName(*ty)),
        Op::Read(var) => write!(f, "read {var}"),
        Op::Unary(op, a) => write!(f, "{op:?} {a}"),
        Op::Binary(op, a, b) => write!(f, "{op:?} {a}, {b}"),
        Op::Compare(cmp, a, b) => write!(f, "cmp {cmp:?} {a}, {b}"),
        Op::Select(c, a, b) => write!(f, "select {c}, {a}, {b}"),
        Op::Convert(a, s) => write!(f, "convert {a} to {s:?}"),
        Op::Bitcast(a, ty) => write!(f, "bitcast {a} to {}", TypeName(*ty)),
        Op::Construct(parts) => {
            f.write_str("construct")?;
            for (i, p) in parts.iter().enumerate() {
                write!(f, "{}{p}", if i == 0 { " " } else { ", " })?;
            }
            Ok(())
        }
        Op::Extract(v, lane) => write!(f, "extract {v}.{lane}"),
        Op::ExtractDynamic(v, lane) => write!(f, "extract {v}[{lane}]"),
        Op::Insert(v, lane, x) => write!(f, "insert {v}.{lane} = {x}"),
        Op::Atomic(op, p, x, s) => write!(f, "atomic {op:?} {s:?} {p}, {x}"),
        Op::CompareExchange(p, c, x, s) => write!(f, "cmpxchg {s:?} {p}, {c}, {x}"),
        Op::Subgroup(op, v, lane) => {
            write!(f, "subgroup {op:?}")?;
            if let Some(v) = v {
                write!(f, " {v}")?;
            }
            if let Some(lane) = lane {
                write!(f, ", {lane}")?;
            }
            Ok(())
        }
        Op::Input(n) => write!(f, "input {n}"),
        Op::Sample { image, sampler, coord, lod } => {
            write!(f, "sample binding {image}")?;
            if let Some(s) = sampler {
                write!(f, " with binding {s}")?;
            }
            write!(f, " at {coord}")?;
            match lod {
                Lod::Implicit => Ok(()),
                Lod::Bias(b) => write!(f, " bias {b}"),
                Lod::Level(l) => write!(f, " level {l}"),
            }
        }
        Op::Derivative(axis, v) => write!(f, "derivative {axis:?} {v}"),
    }
}
