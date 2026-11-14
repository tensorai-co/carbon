//! Expressions: what each operator does to each type GLSL allows it on (matrices' linear algebra among them),
//! implicit conversions as each version has them, constructors, swizzles, and calls to the program's own functions,
//! chosen among their overloads and inlined.

use alloc::boxed::Box;
use alloc::format;
use alloc::sync::Arc;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use gpu_ir::{BinOp, Block, Cmp, Op, Scalar, Stmt, Type, UnOp, Value};

use super::Error;
use super::ast::{Binary, Callee, Expr, ExprKind, Storage, TypeBase, Unary};
use super::lower::{Arg, Func, Lower, Place, Sym, Val, element_type, part_types, scalar_of};
use super::types::{self, Base, Ty};

impl Lower {
    /// An expression's type and value.
    pub fn rvalue(&mut self, out: &mut Block, e: &Expr) -> Result<(Ty, Val), Error> {
        self.line = e.line;
        Ok(match &e.kind {
            ExprKind::Int(v, true) => (types::UINT, Val::V(self.uint_const(out, *v))),
            ExprKind::Int(v, false) => (types::INT, Val::V(self.int_const(out, i64::from(*v as i32)))),
            ExprKind::Float(f) => (types::FLOAT, Val::V(self.float_const(out, *f as f32))),
            ExprKind::Bool(b) => (types::BOOL, Val::V(self.bool_const(out, *b))),
            ExprKind::Ident(_) | ExprKind::Index(..) | ExprKind::Field(..) => {
                let (place, _) = self.place(out, e)?;
                let ty = place.ty().clone();
                let v = self.load(out, &place)?;
                (ty, v)
            }
            ExprKind::Length(x) => {
                let (place, _) = self.place(out, x)?;
                let n = match place.ty() {
                    Ty::Array(_, Some(n)) => *n,
                    Ty::Vector(_, n) => u32::from(*n),
                    Ty::Matrix(c, _) => u32::from(*c),
                    Ty::Array(_, None) => return self.err("the length of a buffer's last array isn't here yet"),
                    other => return self.err(&format!("{other} has no length")),
                };
                (types::INT, Val::V(self.int_const(out, i64::from(n))))
            }
            ExprKind::Unary(op, x) => self.unary(out, *op, x)?,
            ExprKind::Binary(op, a, b) => self.binary_expr(out, *op, a, b)?,
            ExprKind::Assign(op, l, r) => self.assign(out, *op, l, r)?,
            ExprKind::Ternary(c, a, b) => self.ternary(out, c, a, b)?,
            ExprKind::Comma(a, b) => {
                self.rvalue(out, a)?;
                self.rvalue(out, b)?
            }
            ExprKind::Call(callee, args) => self.call(out, callee, args)?,
        })
    }

    /// An expression for what it does, its value unused.
    pub fn effect(&mut self, out: &mut Block, e: &Expr) -> Result<(), Error> {
        self.rvalue(out, e).map(|_| ())
    }

    /// Where an expression's value is: a variable's place (and whether it can't be assigned), or its value in a
    /// place of its own.
    pub fn place(&mut self, out: &mut Block, e: &Expr) -> Result<(Place, bool), Error> {
        self.line = e.line;
        match &e.kind {
            ExprKind::Ident(n) => match self.lookup(n)? {
                Some(Sym::Var { place: Place::Value(Val::Void, ty), constant: Some(k), .. }) => {
                    // A built-in constant: its value made where it's used.
                    let v = match k {
                        super::lower::Konst::Int(i) => self.int_const(out, i),
                        super::lower::Konst::Float(f) => self.float_const(out, f as f32),
                        super::lower::Konst::Bool(b) => self.bool_const(out, b),
                    };
                    Ok((Place::Value(Val::V(v), ty), true))
                }
                Some(Sym::Var { place, readonly, .. }) => Ok((place, readonly)),
                Some(Sym::Type(_)) => self.err(&format!("{n} is a type")),
                None => self.err(&format!("{n} isn't declared")),
            },
            ExprKind::Index(base, index) => {
                let (p, ro) = self.place(out, base)?;
                if element_type(p.ty()).is_none() {
                    return self.err(&format!("{} indexed", p.ty()));
                }
                if let Some(super::lower::Konst::Int(i)) = self.try_const(index) {
                    if i < 0 {
                        return self.err(&format!("index {i}"));
                    }
                    return Ok((self.element(out, p, i as u32)?, ro));
                }
                let (t, i) = self.rvalue(out, index)?;
                if !matches!(t, Ty::Scalar(Base::Int | Base::Uint)) {
                    return self.err(&format!("an index of {t}"));
                }
                Ok((self.element_dyn(out, p, i.v())?, ro))
            }
            ExprKind::Field(base, name) => {
                let (p, ro) = self.place(out, base)?;
                match p.ty().clone() {
                    Ty::Struct(s) => {
                        let Some(i) = s.members.iter().position(|m| &m.0 == name) else { return self.err(&format!("{} has no member {name}", s.name)) };
                        Ok((self.member(p, i)?, ro))
                    }
                    t @ (Ty::Vector(..) | Ty::Scalar(_)) => {
                        let lanes = swizzle(name, t.lanes()).ok_or_else(|| Error { message: format!("0:{}: error: {name} isn't a swizzle of {t}", self.line) })?;
                        let ty = Ty::vector(t.base().expect("vector"), lanes.len() as u8);
                        let dup = (1..lanes.len()).any(|i| lanes[..i].contains(&lanes[i]));
                        let place = match p {
                            Place::Value(Val::V(v), _) => {
                                let x = if t.lanes() == 1 { v } else { self.lanes(out, v, &lanes, &ty) };
                                Place::Value(Val::V(x), ty)
                            }
                            _ if t.lanes() == 1 => {
                                // A scalar's swizzle: its value, as many times as named.
                                let v = self.load(out, &p)?.v();
                                let x = if lanes.len() == 1 { v } else { self.splat(out, v, ty.ir().expect("vector")) };
                                return Ok((Place::Value(Val::V(x), ty), true));
                            }
                            p => Place::Lanes(Box::new(p), lanes, ty),
                        };
                        Ok((place, ro || dup))
                    }
                    other => self.err(&format!("{other} has no member {name}")),
                }
            }
            _ => {
                let (t, v) = self.rvalue(out, e)?;
                Ok((Place::Value(v, t), true))
            }
        }
    }

    fn lanes(&mut self, out: &mut Block, v: Value, lanes: &[u8], ty: &Ty) -> Value {
        let ir = ty.ir().expect("vector");
        if lanes.len() == 1 {
            return self.let_(out, ir, Op::Extract(v, lanes[0]));
        }
        let parts = lanes.iter().map(|&l| self.let_(out, scalar_of(ir), Op::Extract(v, l))).collect();
        self.let_(out, ir, Op::Construct(parts))
    }

    // ---- Conversions ----

    /// Whether GLSL converts `from` to `to` by itself in this version.
    pub fn implicit(&self, from: &Ty, to: &Ty) -> bool {
        if from == to {
            return true;
        }
        if self.version.es || self.version.number < 120 {
            return false;
        }
        match (from, to) {
            (Ty::Scalar(a) | Ty::Vector(a, _), Ty::Scalar(b) | Ty::Vector(b, _)) if from.lanes() == to.lanes() => {
                matches!((a, b), (Base::Int | Base::Uint, Base::Float)) || (*a == Base::Int && *b == Base::Uint && self.version.number >= 400)
            }
            _ => false,
        }
    }

    /// A value converted implicitly to a type, or an error saying it can't be.
    pub fn coerce(&mut self, out: &mut Block, v: Val, from: &Ty, to: &Ty) -> Result<Val, Error> {
        if from == to {
            return Ok(v);
        }
        if !self.implicit(from, to) {
            return self.err(&format!("{from} where {to} is wanted"));
        }
        Ok(Val::V(self.convert(out, v.v(), from, to.base().expect("numeric"))))
    }

    /// A scalar or vector converted to another kind, as a constructor converts: floats to integers toward zero,
    /// numbers to booleans by whether they're not zero.
    pub fn convert(&mut self, out: &mut Block, v: Value, from: &Ty, to: Base) -> Value {
        let have = from.base().expect("numeric");
        if have == to {
            return v;
        }
        let to_ty = from.with_base(to).ir().expect("scalar or vector");
        match (have, to) {
            (_, Base::Bool) => {
                let zero = self.number(out, from, 0.0);
                self.let_(out, to_ty, Op::Compare(Cmp::Ne, v, zero))
            }
            (Base::Int, Base::Uint) | (Base::Uint, Base::Int) => self.let_(out, to_ty, Op::Bitcast(v, to_ty)),
            _ => self.let_(out, to_ty, Op::Convert(v, to.scalar())),
        }
    }

    /// The scalars of a value, in order (a matrix's column by column).
    pub fn scalars(&mut self, out: &mut Block, ty: &Ty, v: &Val) -> Result<Vec<(Base, Value)>, Error> {
        Ok(match (ty, v) {
            (Ty::Scalar(b), Val::V(x)) => vec![(*b, *x)],
            (Ty::Vector(b, n), Val::V(x)) => (0..*n).map(|i| (*b, self.let_(out, Type::Scalar(b.scalar()), Op::Extract(*x, i)))).collect(),
            (Ty::Matrix(..), Val::Parts(cols)) => {
                let col = ty.column().expect("matrix");
                let mut all = Vec::new();
                for c in cols {
                    all.extend(self.scalars(out, &col, c)?);
                }
                all
            }
            _ => return self.err(&format!("{ty} in a constructor of scalars")),
        })
    }

    fn scalar_to(&mut self, out: &mut Block, (b, v): (Base, Value), to: Base) -> Value {
        self.convert(out, v, &Ty::Scalar(b), to)
    }

    // ---- Operators ----

    fn unary(&mut self, out: &mut Block, op: Unary, x: &Expr) -> Result<(Ty, Val), Error> {
        match op {
            Unary::PreInc | Unary::PreDec | Unary::PostInc | Unary::PostDec => {
                let (place, ro) = self.place(out, x)?;
                if ro {
                    return self.err("that can't be incremented");
                }
                let ty = place.ty().clone();
                if !ty.is_numeric() {
                    return self.err(&format!("{ty} incremented"));
                }
                let old = self.load(out, &place)?;
                let one = self.number(out, &ty.column().unwrap_or(ty.clone()), 1.0);
                let one_ty = ty.column().unwrap_or(ty.clone());
                let bop = if matches!(op, Unary::PreInc | Unary::PostInc) { BinOp::Add } else { BinOp::Sub };
                let new = self.map(out, &ty, &old, |me, out, v| me.let_(out, me.e.ty(v), Op::Binary(bop, v, one)))?;
                let _ = one_ty;
                self.store(out, &place, new.clone())?;
                Ok((ty, if matches!(op, Unary::PreInc | Unary::PreDec) { new } else { old }))
            }
            Unary::Plus => {
                let (t, v) = self.rvalue(out, x)?;
                if !t.is_numeric() {
                    return self.err(&format!("+ on {t}"));
                }
                Ok((t, v))
            }
            Unary::Neg => {
                let (t, v) = self.rvalue(out, x)?;
                if !t.is_numeric() {
                    return self.err(&format!("- on {t}"));
                }
                let r = self.map(out, &t, &v, |me, out, v| me.let_(out, me.e.ty(v), Op::Unary(UnOp::Neg, v)))?;
                Ok((t, r))
            }
            Unary::Not => {
                let (t, v) = self.rvalue(out, x)?;
                if t != types::BOOL {
                    return self.err(&format!("! on {t}"));
                }
                Ok((t, Val::V(self.let_(out, Type::BOOL, Op::Unary(UnOp::Not, v.v())))))
            }
            Unary::BitNot => {
                let (t, v) = self.rvalue(out, x)?;
                if !matches!(t.base(), Some(Base::Int | Base::Uint)) || matches!(t, Ty::Matrix(..)) {
                    return self.err(&format!("~ on {t}"));
                }
                let ir = t.ir().expect("integer");
                Ok((t, Val::V(self.let_(out, ir, Op::Unary(UnOp::Not, v.v())))))
            }
        }
    }

    /// An operation on each scalar or vector of a value (a matrix's columns).
    fn map(&mut self, out: &mut Block, ty: &Ty, v: &Val, mut f: impl FnMut(&mut Self, &mut Block, Value) -> Value) -> Result<Val, Error> {
        Ok(match (ty, v) {
            (Ty::Matrix(..), Val::Parts(cols)) => Val::Parts(cols.iter().map(|c| Val::V(f(self, out, c.v()))).collect()),
            (_, Val::V(x)) => Val::V(f(self, out, *x)),
            _ => return self.err(&format!("{ty} in arithmetic")),
        })
    }

    /// Whether evaluating an expression can't change anything, so it may be evaluated when it otherwise wouldn't be.
    pub fn pure(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Int(..) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Ident(_) => true,
            ExprKind::Assign(..) | ExprKind::Unary(Unary::PreInc | Unary::PreDec | Unary::PostInc | Unary::PostDec, _) => false,
            ExprKind::Unary(_, x) | ExprKind::Field(x, _) | ExprKind::Length(x) => self.pure(x),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) | ExprKind::Comma(a, b) => self.pure(a) && self.pure(b),
            ExprKind::Ternary(c, a, b) => self.pure(c) && self.pure(a) && self.pure(b),
            ExprKind::Call(Callee::Type(_), args) => args.iter().all(|a| self.pure(a)),
            ExprKind::Call(Callee::Name(n), args) => !self.functions(n).iter().any(|_| true) && !super::builtins::has_effects(n) && args.iter().all(|a| self.pure(a)),
        }
    }

    fn binary_expr(&mut self, out: &mut Block, op: Binary, a: &Expr, b: &Expr) -> Result<(Ty, Val), Error> {
        if matches!(op, Binary::And | Binary::Or) {
            let (ta, va) = self.rvalue(out, a)?;
            if ta != types::BOOL {
                return self.err(&format!("{op:?} on {ta}"));
            }
            let va = va.v();
            if self.pure(b) {
                let (tb, vb) = self.rvalue(out, b)?;
                if tb != types::BOOL {
                    return self.err(&format!("{op:?} on {tb}"));
                }
                let bop = if op == Binary::And { BinOp::And } else { BinOp::Or };
                return Ok((types::BOOL, Val::V(self.let_(out, Type::BOOL, Op::Binary(bop, va, vb.v())))));
            }
            // The right side only when it decides: an `if` around it.
            let r = self.e.var(Type::BOOL);
            out.push(Stmt::Write(r, va));
            let mut then = Vec::new();
            let (tb, vb) = self.rvalue(&mut then, b)?;
            if tb != types::BOOL {
                return self.err(&format!("{op:?} on {tb}"));
            }
            then.push(Stmt::Write(r, vb.v()));
            let cond = if op == Binary::And { va } else { self.let_(out, Type::BOOL, Op::Unary(UnOp::Not, va)) };
            out.push(Stmt::If { cond, then, otherwise: Vec::new() });
            return Ok((types::BOOL, Val::V(self.let_(out, Type::BOOL, Op::Read(r)))));
        }
        let left = self.rvalue(out, a)?;
        let right = self.rvalue(out, b)?;
        self.binary(out, op, left, right)
    }

    /// Two operands' kinds made the same, as implicit conversion allows: the kind they share.
    fn unify(&mut self, out: &mut Block, a: &mut (Ty, Val), b: &mut (Ty, Val)) -> Result<Base, Error> {
        let (ba, bb) = (a.0.base(), b.0.base());
        let (Some(ba), Some(bb)) = (ba, bb) else { return self.err(&format!("{} and {}", a.0, b.0)) };
        if ba == bb {
            return Ok(ba);
        }
        let to_a = a.0.with_base(bb);
        let to_b = b.0.with_base(ba);
        if !matches!(a.0, Ty::Matrix(..)) && self.implicit(&a.0, &to_a) {
            a.1 = Val::V(self.convert(out, a.1.v(), &a.0, bb));
            a.0 = to_a;
            return Ok(bb);
        }
        if !matches!(b.0, Ty::Matrix(..)) && self.implicit(&b.0, &to_b) {
            b.1 = Val::V(self.convert(out, b.1.v(), &b.0, ba));
            b.0 = to_b;
            return Ok(ba);
        }
        self.err(&format!("{} and {} together", a.0, b.0))
    }

    pub fn binary(&mut self, out: &mut Block, op: Binary, mut a: (Ty, Val), mut b: (Ty, Val)) -> Result<(Ty, Val), Error> {
        use Binary as B;
        match op {
            B::Eq | B::Ne => {
                if a.0 != b.0 {
                    self.unify(out, &mut a, &mut b)?;
                }
                if a.0 != b.0 || a.0.has_sampler() || matches!(a.0, Ty::Array(_, None)) {
                    return self.err(&format!("{} {op:?} {}", a.0, b.0));
                }
                let eq = self.equal(out, &a.0, &a.1, &b.1)?;
                let v = if op == B::Eq { eq } else { self.let_(out, Type::BOOL, Op::Unary(UnOp::Not, eq)) };
                Ok((types::BOOL, Val::V(v)))
            }
            B::Lt | B::Gt | B::Le | B::Ge => {
                self.unify(out, &mut a, &mut b)?;
                if !matches!(a.0, Ty::Scalar(Base::Int | Base::Uint | Base::Float)) || a.0 != b.0 {
                    return self.err(&format!("{} {op:?} {}", a.0, b.0));
                }
                let cmp = match op {
                    B::Lt => Cmp::Lt,
                    B::Gt => Cmp::Gt,
                    B::Le => Cmp::Le,
                    _ => Cmp::Ge,
                };
                Ok((types::BOOL, Val::V(self.let_(out, Type::BOOL, Op::Compare(cmp, a.1.v(), b.1.v())))))
            }
            B::Xor => {
                if a.0 != types::BOOL || b.0 != types::BOOL {
                    return self.err("^^ on what isn't bool");
                }
                Ok((types::BOOL, Val::V(self.let_(out, Type::BOOL, Op::Compare(Cmp::Ne, a.1.v(), b.1.v())))))
            }
            B::And | B::Or => unreachable!("short-circuited"),
            B::Shl | B::Shr => {
                let ok = |t: &Ty| matches!(t.base(), Some(Base::Int | Base::Uint)) && t.is_scalar_or_vector();
                if !ok(&a.0) || !ok(&b.0) || (b.0.lanes() != 1 && b.0.lanes() != a.0.lanes()) {
                    return self.err(&format!("{} {op:?} {}", a.0, b.0));
                }
                let ir = a.0.ir().expect("integer");
                let mut count = b.1.v();
                if b.0.base() != a.0.base() {
                    let t = b.0.with_base(a.0.base().expect("integer")).ir().expect("integer");
                    count = self.let_(out, t, Op::Bitcast(count, t));
                }
                if b.0.lanes() == 1 && a.0.lanes() > 1 {
                    count = self.splat(out, count, ir);
                }
                let bop = if op == B::Shl { BinOp::Shl } else { BinOp::Shr };
                Ok((a.0.clone(), Val::V(self.let_(out, ir, Op::Binary(bop, a.1.v(), count)))))
            }
            B::BitAnd | B::BitOr | B::BitXor | B::Mod => {
                self.unify(out, &mut a, &mut b)?;
                let ok = |t: &Ty| matches!(t.base(), Some(Base::Int | Base::Uint)) && t.is_scalar_or_vector();
                if !ok(&a.0) || !ok(&b.0) {
                    return self.err(&format!("{} {op:?} {}", a.0, b.0));
                }
                let (ty, x, y) = self.shape(out, a, b)?;
                let bop = match op {
                    B::BitAnd => BinOp::And,
                    B::BitOr => BinOp::Or,
                    B::BitXor => BinOp::Xor,
                    _ => BinOp::Rem,
                };
                let ir = ty.ir().expect("integer");
                Ok((ty, Val::V(self.let_(out, ir, Op::Binary(bop, x, y)))))
            }
            B::Add | B::Sub | B::Mul | B::Div => {
                if !a.0.is_numeric() || !b.0.is_numeric() {
                    return self.err(&format!("{} {op:?} {}", a.0, b.0));
                }
                self.unify(out, &mut a, &mut b)?;
                let bop = match op {
                    B::Add => BinOp::Add,
                    B::Sub => BinOp::Sub,
                    B::Mul => BinOp::Mul,
                    _ => BinOp::Div,
                };
                match (&a.0, &b.0) {
                    (Ty::Matrix(..), Ty::Matrix(..)) if op == B::Mul => self.mat_mul(out, &a, &b),
                    (Ty::Matrix(c, r), Ty::Vector(_, n)) if op == B::Mul => {
                        if c != n {
                            return self.err(&format!("{} * {}", a.0, b.0));
                        }
                        let Val::Parts(cols) = &a.1 else { unreachable!() };
                        let cols: Vec<Value> = cols.iter().map(Val::v).collect();
                        let v = self.mat_vec(out, &cols, b.1.v(), *r);
                        Ok((Ty::Vector(Base::Float, *r), Val::V(v)))
                    }
                    (Ty::Vector(_, n), Ty::Matrix(c, r)) if op == B::Mul => {
                        if r != n {
                            return self.err(&format!("{} * {}", a.0, b.0));
                        }
                        // Each column dotted with the vector.
                        let Val::Parts(cols) = &b.1 else { unreachable!() };
                        let lanes = cols.iter().map(|col| self.dot(out, a.1.v(), col.v())).collect::<Vec<_>>();
                        let ty = Ty::Vector(Base::Float, *c);
                        Ok((ty.clone(), Val::V(self.let_(out, ty.ir().expect("vector"), Op::Construct(lanes)))))
                    }
                    (Ty::Matrix(..), Ty::Matrix(..)) => {
                        if a.0 != b.0 {
                            return self.err(&format!("{} {op:?} {}", a.0, b.0));
                        }
                        let (Val::Parts(x), Val::Parts(y)) = (&a.1, &b.1) else { unreachable!() };
                        let ir = a.0.column().and_then(|c| c.ir()).expect("column");
                        let cols = x.iter().zip(y).map(|(p, q)| Val::V(self.let_(out, ir, Op::Binary(bop, p.v(), q.v())))).collect();
                        Ok((a.0.clone(), Val::Parts(cols)))
                    }
                    (Ty::Matrix(..), Ty::Scalar(_)) | (Ty::Scalar(_), Ty::Matrix(..)) => {
                        let (m, s, matrix_first) = if matches!(a.0, Ty::Matrix(..)) { (&a, &b, true) } else { (&b, &a, false) };
                        let ir = m.0.column().and_then(|c| c.ir()).expect("column");
                        let k = self.splat(out, s.1.v(), ir);
                        let Val::Parts(cols) = &m.1 else { unreachable!() };
                        let cols = cols.iter().map(|c| Val::V(if matrix_first { self.let_(out, ir, Op::Binary(bop, c.v(), k)) } else { self.let_(out, ir, Op::Binary(bop, k, c.v())) })).collect();
                        Ok((m.0.clone(), Val::Parts(cols)))
                    }
                    (Ty::Matrix(..), _) | (_, Ty::Matrix(..)) => self.err(&format!("{} {op:?} {}", a.0, b.0)),
                    _ => {
                        let (ty, x, y) = self.shape(out, a, b)?;
                        let ir = ty.ir().expect("numeric");
                        Ok((ty, Val::V(self.let_(out, ir, Op::Binary(bop, x, y)))))
                    }
                }
            }
        }
    }

    /// Two scalars or vectors of one kind made the same shape (a scalar spread to the vector's lanes).
    fn shape(&mut self, out: &mut Block, a: (Ty, Val), b: (Ty, Val)) -> Result<(Ty, Value, Value), Error> {
        let (la, lb) = (a.0.lanes(), b.0.lanes());
        if la == lb {
            return Ok((a.0, a.1.v(), b.1.v()));
        }
        if la == 1 {
            let ir = b.0.ir().expect("vector");
            let x = self.splat(out, a.1.v(), ir);
            return Ok((b.0, x, b.1.v()));
        }
        if lb == 1 {
            let ir = a.0.ir().expect("vector");
            let y = self.splat(out, b.1.v(), ir);
            return Ok((a.0.clone(), a.1.v(), y));
        }
        self.err(&format!("{} and {} of different sizes", a.0, b.0))
    }

    pub fn dot(&mut self, out: &mut Block, p: Value, q: Value) -> Value {
        let ty = self.e.ty(p);
        let m = self.let_(out, ty, Op::Binary(BinOp::Mul, p, q));
        if ty.lanes() == 1 {
            return m;
        }
        let mut sum = self.let_(out, Type::F32, Op::Extract(m, 0));
        for i in 1..ty.lanes() {
            let l = self.let_(out, Type::F32, Op::Extract(m, i));
            sum = self.let_(out, Type::F32, Op::Binary(BinOp::Add, sum, l));
        }
        sum
    }

    /// A matrix (its columns) times a vector: the columns, each scaled by the vector's lane, added up.
    pub fn mat_vec(&mut self, out: &mut Block, cols: &[Value], v: Value, rows: u8) -> Value {
        let ir = Type::Vector(Scalar::F32, rows);
        let mut sum = None;
        for (i, &c) in cols.iter().enumerate() {
            let s = self.let_(out, Type::F32, Op::Extract(v, i as u8));
            let k = self.splat(out, s, ir);
            let term = self.let_(out, ir, Op::Binary(BinOp::Mul, c, k));
            sum = Some(match sum {
                None => term,
                Some(acc) => self.let_(out, ir, Op::Binary(BinOp::Add, acc, term)),
            });
        }
        sum.expect("a column")
    }

    fn mat_mul(&mut self, out: &mut Block, a: &(Ty, Val), b: &(Ty, Val)) -> Result<(Ty, Val), Error> {
        let (Ty::Matrix(ac, ar), Ty::Matrix(bc, br)) = (&a.0, &b.0) else { unreachable!() };
        if ac != br {
            return self.err(&format!("{} * {}", a.0, b.0));
        }
        let (Val::Parts(x), Val::Parts(y)) = (&a.1, &b.1) else { unreachable!() };
        let left: Vec<Value> = x.iter().map(Val::v).collect();
        let cols = y.iter().map(|col| Val::V(self.mat_vec(out, &left, col.v(), *ar))).collect();
        Ok((Ty::Matrix(*bc, *ar), Val::Parts(cols)))
    }

    /// Whether two values of a type are equal, every part of them.
    fn equal(&mut self, out: &mut Block, ty: &Ty, a: &Val, b: &Val) -> Result<Value, Error> {
        Ok(match (a, b) {
            (Val::V(x), Val::V(y)) => {
                let bools = ty.with_base(Base::Bool).ir().expect("scalar");
                let e = self.let_(out, bools, Op::Compare(Cmp::Eq, *x, *y));
                self.all(out, e)
            }
            (Val::Parts(x), Val::Parts(y)) => {
                let parts = part_types(ty);
                let mut acc = self.bool_const(out, true);
                for (k, (p, q)) in x.iter().zip(y).enumerate() {
                    let e = self.equal(out, &parts[k], p, q)?;
                    acc = self.let_(out, Type::BOOL, Op::Binary(BinOp::And, acc, e));
                }
                acc
            }
            _ => return self.err(&format!("{ty} compared")),
        })
    }

    /// Whether every lane of a boolean vector is true.
    pub fn all(&mut self, out: &mut Block, v: Value) -> Value {
        let n = self.e.ty(v).lanes();
        if n == 1 {
            return v;
        }
        let mut acc = self.let_(out, Type::BOOL, Op::Extract(v, 0));
        for i in 1..n {
            let l = self.let_(out, Type::BOOL, Op::Extract(v, i));
            acc = self.let_(out, Type::BOOL, Op::Binary(BinOp::And, acc, l));
        }
        acc
    }

    pub fn any(&mut self, out: &mut Block, v: Value) -> Value {
        let n = self.e.ty(v).lanes();
        if n == 1 {
            return v;
        }
        let mut acc = self.let_(out, Type::BOOL, Op::Extract(v, 0));
        for i in 1..n {
            let l = self.let_(out, Type::BOOL, Op::Extract(v, i));
            acc = self.let_(out, Type::BOOL, Op::Binary(BinOp::Or, acc, l));
        }
        acc
    }

    fn assign(&mut self, out: &mut Block, op: Option<Binary>, l: &Expr, r: &Expr) -> Result<(Ty, Val), Error> {
        let right = self.rvalue(out, r)?;
        let (place, ro) = self.place(out, l)?;
        if ro {
            return self.err("that can't be assigned");
        }
        let ty = place.ty().clone();
        if ty.has_sampler() {
            return self.err("a sampler assigned");
        }
        let value = match op {
            None => self.coerce(out, right.1, &right.0, &ty)?,
            Some(op) => {
                let old = self.load(out, &place)?;
                let (t, v) = self.binary(out, op, (ty.clone(), old), right)?;
                self.coerce(out, v, &t, &ty)?
            }
        };
        self.store(out, &place, value.clone())?;
        Ok((ty, value))
    }

    fn ternary(&mut self, out: &mut Block, c: &Expr, a: &Expr, b: &Expr) -> Result<(Ty, Val), Error> {
        let cond = self.condition_value(out, c)?;
        if self.pure(a) && self.pure(b) {
            let mut x = self.rvalue(out, a)?;
            let mut y = self.rvalue(out, b)?;
            if x.0 != y.0 {
                self.unify(out, &mut x, &mut y)?;
            }
            if x.0 != y.0 {
                return self.err(&format!("?: of {} and {}", x.0, y.0));
            }
            let v = self.select(out, cond, x.1, y.1, &x.0)?;
            return Ok((x.0, v));
        }
        // Only the side chosen is evaluated: each in an `if`, into variables.
        let mut then = Vec::new();
        let x = self.rvalue(&mut then, a)?;
        let mut otherwise = Vec::new();
        let y = self.rvalue(&mut otherwise, b)?;
        let ty = if x.0 == y.0 {
            x.0.clone()
        } else if self.implicit(&x.0, &y.0) {
            y.0.clone()
        } else if self.implicit(&y.0, &x.0) {
            x.0.clone()
        } else {
            return self.err(&format!("?: of {} and {}", x.0, y.0));
        };
        if ty == Ty::Void {
            out.push(Stmt::If { cond, then, otherwise });
            return Ok((Ty::Void, Val::Void));
        }
        let r = self.alloc(&ty)?;
        let xv = self.coerce(&mut then, x.1, &x.0, &ty)?;
        self.store(&mut then, &r, xv)?;
        let yv = self.coerce(&mut otherwise, y.1, &y.0, &ty)?;
        self.store(&mut otherwise, &r, yv)?;
        out.push(Stmt::If { cond, then, otherwise });
        let v = self.load(out, &r)?;
        Ok((ty, v))
    }

    // ---- Calls ----

    fn call(&mut self, out: &mut Block, callee: &Callee, args: &[Expr]) -> Result<(Ty, Val), Error> {
        match callee {
            Callee::Type(spec) => {
                let mut ty = match (&spec.base, spec.dims.first()) {
                    (_, Some(None)) => {
                        // `float[](...)`: as many as there are.
                        let mut inner = spec.clone();
                        inner.dims.remove(0);
                        let e = self.resolve(&inner)?;
                        Ty::Array(Box::new(e), Some(args.len() as u32))
                    }
                    _ => self.resolve(spec)?,
                };
                if let (TypeBase::Named(n), true) = (&spec.base, spec.dims.is_empty())
                    && types::named(n).is_none()
                {
                    ty = self.resolve(spec)?;
                }
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.rvalue(out, a)?);
                }
                let v = self.construct(out, &ty, vals)?;
                Ok((ty, v))
            }
            Callee::Name(name) => {
                let overloads = self.functions(name);
                if overloads.is_empty() {
                    let mut vals = Vec::new();
                    let mut places = Vec::new();
                    for a in args {
                        // Built-ins with `out` parameters (`modf`, `frexp`) take places; the rest values.
                        if super::builtins::wants_place(name, places.len()) {
                            let (p, ro) = self.place(out, a)?;
                            if ro {
                                return self.err(&format!("{name}'s out argument can't be assigned"));
                            }
                            vals.push((p.ty().clone(), Val::Void));
                            places.push(Some(p));
                        } else {
                            vals.push(self.rvalue(out, a)?);
                            places.push(None);
                        }
                    }
                    return match self.builtin(out, name, vals, places)? {
                        Some(r) => Ok(r),
                        None => self.err(&format!("{name} isn't a function")),
                    };
                }
                let mut given = Vec::new();
                for a in args {
                    given.push(self.place(out, a)?);
                }
                let f = self.choose(&overloads, &given, name)?;
                let mut call_args = Vec::new();
                for ((storage, ty, _), (place, ro)) in f.params.iter().zip(&given) {
                    match storage {
                        Storage::Out | Storage::InOut => {
                            if *ro {
                                return self.err(&format!("{name}'s out argument can't be assigned"));
                            }
                            if place.ty() != ty {
                                return self.err(&format!("{name}'s out argument is {}, not {ty}", place.ty()));
                            }
                            call_args.push(Arg::Place(place.clone()));
                        }
                        _ => {
                            let v = self.load(out, place)?;
                            let v = self.coerce(out, v, place.ty(), ty)?;
                            call_args.push(Arg::Value(v));
                        }
                    }
                }
                let ret = if f.ret == Ty::Void { None } else { Some(self.alloc(&f.ret)?) };
                self.call_body(out, &f, &call_args, ret.clone())?;
                match ret {
                    Some(r) => {
                        let v = self.load(out, &r)?;
                        Ok((f.ret.clone(), v))
                    }
                    None => Ok((Ty::Void, Val::Void)),
                }
            }
        }
    }

    /// The overload a call means: one whose parameters match exactly, else the only one its arguments convert to.
    fn choose(&self, overloads: &[Arc<Func>], given: &[(Place, bool)], name: &str) -> Result<Arc<Func>, Error> {
        let fits = |f: &Func, exact: bool| {
            f.params.len() == given.len() && f.params.iter().zip(given).all(|((s, t, _), (p, _))| if exact || matches!(s, Storage::Out | Storage::InOut) { p.ty() == t } else { self.implicit(p.ty(), t) })
        };
        if let Some(f) = overloads.iter().find(|f| fits(f, true)) {
            return Ok(f.clone());
        }
        let near: Vec<&Arc<Func>> = overloads.iter().filter(|f| fits(f, false)).collect();
        match near.as_slice() {
            [f] => Ok((*f).clone()),
            [] => {
                let tys: Vec<_> = given.iter().map(|g| g.0.ty().to_string()).collect();
                self.err(&format!("no {name} takes ({})", tys.join(", ")))
            }
            _ => self.err(&format!("the call to {name} could mean more than one of its overloads")),
        }
    }

    /// A constructor: GLSL's rules for each kind of type.
    pub fn construct(&mut self, out: &mut Block, ty: &Ty, args: Vec<(Ty, Val)>) -> Result<Val, Error> {
        if args.is_empty() {
            return self.err(&format!("{ty}() with nothing"));
        }
        match ty {
            Ty::Scalar(b) => {
                if args.len() != 1 {
                    return self.err(&format!("{ty} from {} arguments", args.len()));
                }
                let first = self.scalars(out, &args[0].0, &args[0].1)?[0];
                Ok(Val::V(self.scalar_to(out, first, *b)))
            }
            Ty::Vector(b, n) => {
                if args.len() == 1 && matches!(args[0].0, Ty::Scalar(_)) {
                    let s = self.scalars(out, &args[0].0, &args[0].1)?[0];
                    let s = self.scalar_to(out, s, *b);
                    return Ok(Val::V(self.splat(out, s, ty.ir().expect("vector"))));
                }
                let lanes = self.components(out, &args, u32::from(*n), *b, true)?;
                Ok(Val::V(self.let_(out, ty.ir().expect("vector"), Op::Construct(lanes))))
            }
            Ty::Matrix(c, r) => {
                let (c, r) = (*c, *r);
                let col_ir = Type::Vector(Scalar::F32, r);
                if args.len() == 1 && matches!(args[0].0, Ty::Scalar(_)) {
                    // A diagonal of the value.
                    let s = self.scalars(out, &args[0].0, &args[0].1)?[0];
                    let s = self.scalar_to(out, s, Base::Float);
                    let zero = self.float_const(out, 0.0);
                    let cols = (0..c).map(|i| Val::V(self.let_(out, col_ir, Op::Construct((0..r).map(|j| if i == j { s } else { zero }).collect())))).collect();
                    return Ok(Val::Parts(cols));
                }
                if args.len() == 1 && let (Ty::Matrix(sc, sr), Val::Parts(src)) = (&args[0].0, &args[0].1) {
                    // Another matrix: what overlaps, and the identity's elsewhere.
                    let (zero, one) = (self.float_const(out, 0.0), self.float_const(out, 1.0));
                    let mut cols = Vec::new();
                    for i in 0..c {
                        let mut lanes = Vec::new();
                        for j in 0..r {
                            lanes.push(if i < *sc && j < *sr {
                                self.let_(out, Type::F32, Op::Extract(src[i as usize].v(), j))
                            } else if i == j {
                                one
                            } else {
                                zero
                            });
                        }
                        cols.push(Val::V(self.let_(out, col_ir, Op::Construct(lanes))));
                    }
                    return Ok(Val::Parts(cols));
                }
                let all = self.components(out, &args, u32::from(c) * u32::from(r), Base::Float, false)?;
                let cols = all.chunks(r as usize).map(|lanes| Val::V(self.let_(out, col_ir, Op::Construct(lanes.to_vec())))).collect();
                Ok(Val::Parts(cols))
            }
            Ty::Struct(s) => {
                if args.len() != s.members.len() {
                    return self.err(&format!("{} from {} arguments: it has {} members", s.name, args.len(), s.members.len()));
                }
                let mut parts = Vec::new();
                for ((_, mt), (t, v)) in s.members.iter().zip(args) {
                    parts.push(self.coerce(out, v, &t, mt)?);
                }
                Ok(Val::Parts(parts))
            }
            Ty::Array(e, Some(n)) => {
                if args.len() != *n as usize {
                    return self.err(&format!("{ty} from {} arguments", args.len()));
                }
                let mut parts = Vec::new();
                for (t, v) in args {
                    parts.push(self.coerce(out, v, &t, e)?);
                }
                Ok(Val::Parts(parts))
            }
            _ => self.err(&format!("{ty} can't be constructed")),
        }
    }

    /// The first `n` scalars of the arguments, converted; every argument must be used (`extra`: the last may be cut).
    fn components(&mut self, out: &mut Block, args: &[(Ty, Val)], n: u32, to: Base, extra: bool) -> Result<Vec<Value>, Error> {
        let mut lanes = Vec::new();
        for (i, (t, v)) in args.iter().enumerate() {
            if lanes.len() as u32 >= n {
                return self.err(&format!("too many arguments for {n} components"));
            }
            if matches!(t, Ty::Array(..) | Ty::Struct(_) | Ty::Sampler(_) | Ty::Void) {
                return self.err(&format!("{t} in a constructor of scalars"));
            }
            let scalars = self.scalars(out, t, v)?;
            let take = (n as usize - lanes.len()).min(scalars.len());
            if take < scalars.len() && (!extra || i + 1 != args.len()) {
                return self.err(&format!("too many arguments for {n} components"));
            }
            for s in scalars.into_iter().take(take) {
                lanes.push(self.scalar_to(out, s, to));
            }
        }
        if (lanes.len() as u32) < n {
            return self.err(&format!("{} components given, {n} wanted", lanes.len()));
        }
        Ok(lanes)
    }
}

/// A swizzle's lanes (`xyzw`, `rgba` or `stpq`, not mixed), each below `lanes`.
fn swizzle(name: &str, lanes: u8) -> Option<Vec<u8>> {
    if name.is_empty() || name.len() > 4 {
        return None;
    }
    let sets = ["xyzw", "rgba", "stpq"];
    let set = sets.iter().find(|s| name.chars().all(|c| s.contains(c)))?;
    name.chars().map(|c| set.find(c).map(|i| i as u8).filter(|&i| i < lanes)).collect()
}
