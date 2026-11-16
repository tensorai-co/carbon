//! GLSL's built-in functions, as IR: the angle, exponential, common and geometric functions component by component
//! (arguments spread to the widest's lanes, integers converted to floats where GLSL converts them), the matrix and
//! vector-relational functions, bit casts, textures, derivatives, barriers and atomics. Where the IR has no
//! operation (the inverse trigonometric functions), they're polynomials accurate to about 1e-6.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu_ir::{AtomicOp, Axis, Barrier, BinOp, Block, Cmp, Lod, Op, Scalar, Stmt, Type, UnOp, Value};

use super::lower::{Base_, Lower, Place, Val};
use super::types::{self, Base, Ty};
use super::{Error, Stage};

/// Built-ins that change something, so are never evaluated speculatively.
pub fn has_effects(name: &str) -> bool {
    name.starts_with("atomic") || name.contains("arrier") || name == "modf"
}

/// Whether a built-in's argument is written (`modf`'s second) or names memory (an atomic's first).
pub fn wants_place(name: &str, index: usize) -> bool {
    (name == "modf" && index == 1) || (name.starts_with("atomic") && index == 0)
}

const PI: f32 = core::f32::consts::PI;

impl Lower {
    /// A built-in call's type and value, or None if there's no built-in of the name.
    pub fn builtin(&mut self, out: &mut Block, name: &str, args: Vec<(Ty, Val)>, places: Vec<Option<Place>>) -> Result<Option<(Ty, Val)>, Error> {
        let n = args.len();
        let want = |k: usize| -> Result<(), Error> { if n == k { Ok(()) } else { super::error(0, "") } };
        let arity = |me: &Self, k: usize| -> Result<(), Error> {
            if n == k { Ok(()) } else { me.err(&format!("{name} takes {k} arguments, given {n}")) }
        };
        let _ = want;
        let r = match name {
            // ---- Angles and trigonometry ----
            "radians" | "degrees" => {
                arity(self, 1)?;
                let k = if name == "radians" { PI / 180.0 } else { 180.0 / PI };
                self.float_map(out, &args, |me, out, x| {
                    let c = me.fsplat(out, k, x[0]);
                    me.fbin(out, BinOp::Mul, x[0], c)
                })?
            }
            "sin" | "cos" => {
                arity(self, 1)?;
                let op = if name == "sin" { UnOp::Sin } else { UnOp::Cos };
                self.float_map(out, &args, |me, out, x| me.fun(out, op, x[0]))?
            }
            "tan" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let (s, c) = (me.fun(out, UnOp::Sin, x[0]), me.fun(out, UnOp::Cos, x[0]));
                    me.fbin(out, BinOp::Div, s, c)
                })?
            }
            "asin" | "acos" => {
                arity(self, 1)?;
                let acos = name == "acos";
                self.float_map(out, &args, |me, out, x| me.asin_acos(out, x[0], acos))?
            }
            "atan" if n == 2 => self.float_map(out, &args, |me, out, x| me.atan2(out, x[0], x[1]))?,
            "atan" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let one = me.fsplat(out, 1.0, x[0]);
                    me.atan2(out, x[0], one)
                })?
            }
            "sinh" | "cosh" | "tanh" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let e = me.exp(out, x[0]);
                    let neg = me.fun(out, UnOp::Neg, x[0]);
                    let f = me.exp(out, neg);
                    let half = me.fsplat(out, 0.5, x[0]);
                    let d = me.fbin(out, BinOp::Sub, e, f);
                    let s = me.fbin(out, BinOp::Add, e, f);
                    match name {
                        "sinh" => me.fbin(out, BinOp::Mul, d, half),
                        "cosh" => me.fbin(out, BinOp::Mul, s, half),
                        _ => me.fbin(out, BinOp::Div, d, s),
                    }
                })?
            }
            "asinh" | "acosh" | "atanh" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let one = me.fsplat(out, 1.0, x[0]);
                    match name {
                        "atanh" => {
                            let a = me.fbin(out, BinOp::Add, one, x[0]);
                            let b = me.fbin(out, BinOp::Sub, one, x[0]);
                            let q = me.fbin(out, BinOp::Div, a, b);
                            let l = me.log(out, q);
                            let half = me.fsplat(out, 0.5, x[0]);
                            me.fbin(out, BinOp::Mul, l, half)
                        }
                        _ => {
                            let sq = me.fbin(out, BinOp::Mul, x[0], x[0]);
                            let inner = if name == "asinh" { me.fbin(out, BinOp::Add, sq, one) } else { me.fbin(out, BinOp::Sub, sq, one) };
                            let r = me.fun(out, UnOp::Sqrt, inner);
                            let s = me.fbin(out, BinOp::Add, x[0], r);
                            me.log(out, s)
                        }
                    }
                })?
            }
            // ---- Exponentials ----
            "pow" => {
                arity(self, 2)?;
                self.float_map(out, &args, |me, out, x| {
                    let l = me.fun(out, UnOp::Log2, x[0]);
                    let p = me.fbin(out, BinOp::Mul, x[1], l);
                    me.fun(out, UnOp::Exp2, p)
                })?
            }
            "exp" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| me.exp(out, x[0]))?
            }
            "log" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| me.log(out, x[0]))?
            }
            "exp2" | "log2" | "sqrt" => {
                arity(self, 1)?;
                let op = match name {
                    "exp2" => UnOp::Exp2,
                    "log2" => UnOp::Log2,
                    _ => UnOp::Sqrt,
                };
                self.float_map(out, &args, |me, out, x| me.fun(out, op, x[0]))?
            }
            "inversesqrt" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let r = me.fun(out, UnOp::Sqrt, x[0]);
                    let one = me.fsplat(out, 1.0, x[0]);
                    me.fbin(out, BinOp::Div, one, r)
                })?
            }
            // ---- Common ----
            "abs" => {
                arity(self, 1)?;
                self.num_map(out, &args, false, |me, out, x| me.fun(out, UnOp::Abs, x[0]))?
            }
            "sign" => {
                arity(self, 1)?;
                self.num_map(out, &args, false, |me, out, x| {
                    let ty = me.e.ty(x[0]);
                    let (zero, one, minus) = (me.konst_like(out, 0.0, ty), me.konst_like(out, 1.0, ty), me.konst_like(out, -1.0, ty));
                    let bools = ty.with_scalar(Scalar::Bool);
                    let pos = me.let_(out, bools, Op::Compare(Cmp::Gt, x[0], zero));
                    let neg = me.let_(out, bools, Op::Compare(Cmp::Lt, x[0], zero));
                    let low = me.let_(out, ty, Op::Select(neg, minus, zero));
                    me.let_(out, ty, Op::Select(pos, one, low))
                })?
            }
            "floor" | "ceil" | "trunc" | "round" | "roundEven" => {
                arity(self, 1)?;
                let op = match name {
                    "floor" => UnOp::Floor,
                    "ceil" => UnOp::Ceil,
                    "trunc" => UnOp::Trunc,
                    _ => UnOp::RoundEven,
                };
                self.float_map(out, &args, |me, out, x| me.fun(out, op, x[0]))?
            }
            "fract" => {
                arity(self, 1)?;
                self.float_map(out, &args, |me, out, x| {
                    let f = me.fun(out, UnOp::Floor, x[0]);
                    me.fbin(out, BinOp::Sub, x[0], f)
                })?
            }
            "mod" => {
                arity(self, 2)?;
                self.float_map(out, &args, |me, out, x| me.fmod(out, x[0], x[1]))?
            }
            "modf" => {
                arity(self, 2)?;
                let Some(Some(dst)) = places.get(1).cloned() else { return self.err("modf's second argument") };
                let (t, v) = self.floats(out, &args[0])?;
                if dst.ty() != &t {
                    return self.err(&format!("modf's out argument is {}, not {t}", dst.ty()));
                }
                let whole = self.fun(out, UnOp::Trunc, v);
                self.store(out, &dst, Val::V(whole))?;
                (t, Val::V(self.fbin(out, BinOp::Sub, v, whole)))
            }
            "min" | "max" => {
                arity(self, 2)?;
                let op = if name == "min" { BinOp::Min } else { BinOp::Max };
                self.num_map(out, &args, true, |me, out, x| me.fbin(out, op, x[0], x[1]))?
            }
            "clamp" => {
                arity(self, 3)?;
                self.num_map(out, &args, true, |me, out, x| {
                    let low = me.fbin(out, BinOp::Max, x[0], x[1]);
                    me.fbin(out, BinOp::Min, low, x[2])
                })?
            }
            "mix" if n == 3 && matches!(args[2].0.base(), Some(Base::Bool)) => {
                // mix(x, y, a) with booleans: each lane from y where a's is true.
                let (t, x) = (args[0].0.clone(), args[0].1.v());
                if args[1].0 != t || args[2].0.lanes() != t.lanes() {
                    return self.err(&format!("mix of {t}, {} and {}", args[1].0, args[2].0));
                }
                let ir = t.ir().expect("vector");
                (t, Val::V(self.let_(out, ir, Op::Select(args[2].1.v(), args[1].1.v(), x))))
            }
            "mix" => {
                arity(self, 3)?;
                self.float_map(out, &args, |me, out, x| {
                    let d = me.fbin(out, BinOp::Sub, x[1], x[0]);
                    let t = me.fbin(out, BinOp::Mul, d, x[2]);
                    me.fbin(out, BinOp::Add, x[0], t)
                })?
            }
            "step" => {
                arity(self, 2)?;
                self.float_map(out, &args, |me, out, x| {
                    let ty = me.e.ty(x[1]);
                    let (zero, one) = (me.fsplat(out, 0.0, x[1]), me.fsplat(out, 1.0, x[1]));
                    let below = me.let_(out, ty.with_scalar(Scalar::Bool), Op::Compare(Cmp::Lt, x[1], x[0]));
                    me.let_(out, ty, Op::Select(below, zero, one))
                })?
            }
            "smoothstep" => {
                arity(self, 3)?;
                self.float_map(out, &args, |me, out, x| {
                    let num = me.fbin(out, BinOp::Sub, x[2], x[0]);
                    let den = me.fbin(out, BinOp::Sub, x[1], x[0]);
                    let t = me.fbin(out, BinOp::Div, num, den);
                    let (zero, one) = (me.fsplat(out, 0.0, x[2]), me.fsplat(out, 1.0, x[2]));
                    let t = me.fbin(out, BinOp::Max, t, zero);
                    let t = me.fbin(out, BinOp::Min, t, one);
                    let (two, three) = (me.fsplat(out, 2.0, x[2]), me.fsplat(out, 3.0, x[2]));
                    let tt = me.fbin(out, BinOp::Mul, t, t);
                    let twice = me.fbin(out, BinOp::Mul, two, t);
                    let k = me.fbin(out, BinOp::Sub, three, twice);
                    me.fbin(out, BinOp::Mul, tt, k)
                })?
            }
            "fma" => {
                arity(self, 3)?;
                self.float_map(out, &args, |me, out, x| {
                    let m = me.fbin(out, BinOp::Mul, x[0], x[1]);
                    me.fbin(out, BinOp::Add, m, x[2])
                })?
            }
            "isnan" | "isinf" => {
                arity(self, 1)?;
                let (t, v) = self.floats(out, &args[0])?;
                let bools = t.with_base(Base::Bool);
                let ir = bools.ir().expect("bool");
                let r = if name == "isnan" {
                    self.let_(out, ir, Op::Compare(Cmp::Ne, v, v))
                } else {
                    let a = self.fun(out, UnOp::Abs, v);
                    let inf = self.fsplat(out, f32::INFINITY, v);
                    self.let_(out, ir, Op::Compare(Cmp::Eq, a, inf))
                };
                (bools, Val::V(r))
            }
            "floatBitsToInt" | "floatBitsToUint" | "intBitsToFloat" | "uintBitsToFloat" => {
                arity(self, 1)?;
                let (from, to) = match name {
                    "floatBitsToInt" => (Base::Float, Base::Int),
                    "floatBitsToUint" => (Base::Float, Base::Uint),
                    "intBitsToFloat" => (Base::Int, Base::Float),
                    _ => (Base::Uint, Base::Float),
                };
                let t = &args[0].0;
                if t.base() != Some(from) || !t.is_scalar_or_vector() {
                    return self.err(&format!("{name} of {t}"));
                }
                let rt = t.with_base(to);
                let ir = rt.ir().expect("scalar");
                (rt, Val::V(self.let_(out, ir, Op::Bitcast(args[0].1.v(), ir))))
            }
            "bitCount" => {
                arity(self, 1)?;
                let t = args[0].0.clone();
                if !matches!(t.base(), Some(Base::Int | Base::Uint)) || !t.is_scalar_or_vector() {
                    return self.err(&format!("bitCount of {t}"));
                }
                let ir = t.ir().expect("integer");
                let c = self.let_(out, ir, Op::Unary(UnOp::Popcount, args[0].1.v()));
                let rt = t.with_base(Base::Int);
                let rir = rt.ir().expect("int");
                (rt, Val::V(if ir == rir { c } else { self.let_(out, rir, Op::Bitcast(c, rir)) }))
            }
            // ---- Geometry ----
            "length" => {
                arity(self, 1)?;
                let (_, v) = self.floats(out, &args[0])?;
                (types::FLOAT, Val::V(self.length(out, v)))
            }
            "distance" => {
                arity(self, 2)?;
                let (_, a) = self.floats(out, &args[0])?;
                let (_, b) = self.floats(out, &args[1])?;
                let d = self.fbin(out, BinOp::Sub, a, b);
                (types::FLOAT, Val::V(self.length(out, d)))
            }
            "dot" => {
                arity(self, 2)?;
                let (ta, a) = self.floats(out, &args[0])?;
                let (tb, b) = self.floats(out, &args[1])?;
                if ta != tb {
                    return self.err(&format!("dot of {ta} and {tb}"));
                }
                (types::FLOAT, Val::V(self.dot(out, a, b)))
            }
            "cross" => {
                arity(self, 2)?;
                let vec3 = Ty::Vector(Base::Float, 3);
                let (ta, a) = self.floats(out, &args[0])?;
                let (tb, b) = self.floats(out, &args[1])?;
                if ta != vec3 || tb != vec3 {
                    return self.err("cross of what isn't vec3");
                }
                let p: Vec<Value> = (0..3).map(|i| self.let_(out, Type::F32, Op::Extract(a, i))).collect();
                let q: Vec<Value> = (0..3).map(|i| self.let_(out, Type::F32, Op::Extract(b, i))).collect();
                let mut lanes = Vec::new();
                for (i, j) in [(1, 2), (2, 0), (0, 1)] {
                    let l = self.fbin(out, BinOp::Mul, p[i], q[j]);
                    let r = self.fbin(out, BinOp::Mul, p[j], q[i]);
                    lanes.push(self.fbin(out, BinOp::Sub, l, r));
                }
                (vec3, Val::V(self.let_(out, Type::Vector(Scalar::F32, 3), Op::Construct(lanes))))
            }
            "normalize" => {
                arity(self, 1)?;
                let (t, v) = self.floats(out, &args[0])?;
                let len = self.length(out, v);
                let len = self.splat(out, len, t.ir().expect("float"));
                (t, Val::V(self.fbin(out, BinOp::Div, v, len)))
            }
            "faceforward" => {
                arity(self, 3)?;
                let (t, nv) = self.floats(out, &args[0])?;
                let (_, i) = self.floats(out, &args[1])?;
                let (_, nref) = self.floats(out, &args[2])?;
                let d = self.dot(out, nref, i);
                let zero = self.float_const(out, 0.0);
                let facing = self.let_(out, Type::BOOL, Op::Compare(Cmp::Lt, d, zero));
                let neg = self.fun(out, UnOp::Neg, nv);
                (t.clone(), Val::V(self.let_(out, t.ir().expect("float"), Op::Select(facing, nv, neg))))
            }
            "reflect" => {
                arity(self, 2)?;
                let (t, i) = self.floats(out, &args[0])?;
                let (_, nv) = self.floats(out, &args[1])?;
                let d = self.dot(out, nv, i);
                let two = self.float_const(out, 2.0);
                let d2 = self.fbin(out, BinOp::Mul, d, two);
                let k = self.splat(out, d2, t.ir().expect("float"));
                let scaled = self.fbin(out, BinOp::Mul, k, nv);
                (t, Val::V(self.fbin(out, BinOp::Sub, i, scaled)))
            }
            "refract" => {
                arity(self, 3)?;
                let (t, i) = self.floats(out, &args[0])?;
                let (_, nv) = self.floats(out, &args[1])?;
                let (_, eta) = self.floats(out, &args[2])?;
                let ir = t.ir().expect("float");
                let d = self.dot(out, nv, i);
                let one = self.float_const(out, 1.0);
                let dd = self.fbin(out, BinOp::Mul, d, d);
                let rest = self.fbin(out, BinOp::Sub, one, dd);
                let ee = self.fbin(out, BinOp::Mul, eta, eta);
                let m = self.fbin(out, BinOp::Mul, ee, rest);
                let k = self.fbin(out, BinOp::Sub, one, m);
                let zero = self.float_const(out, 0.0);
                let tir = self.let_(out, Type::BOOL, Op::Compare(Cmp::Lt, k, zero));
                let kk = self.fbin(out, BinOp::Max, k, zero);
                let root = self.fun(out, UnOp::Sqrt, kk);
                let ed = self.fbin(out, BinOp::Mul, eta, d);
                let c = self.fbin(out, BinOp::Add, ed, root);
                let es = self.splat(out, eta, ir);
                let cs = self.splat(out, c, ir);
                let a = self.fbin(out, BinOp::Mul, es, i);
                let b = self.fbin(out, BinOp::Mul, cs, nv);
                let r = self.fbin(out, BinOp::Sub, a, b);
                let zv = self.splat(out, zero, ir);
                (t, Val::V(self.let_(out, ir, Op::Select(tir, zv, r))))
            }
            // ---- Matrices ----
            "matrixCompMult" => {
                arity(self, 2)?;
                let (Ty::Matrix(..), Val::Parts(a), Val::Parts(b)) = (&args[0].0, &args[0].1, &args[1].1) else { return self.err("matrixCompMult of what aren't matrices") };
                if args[0].0 != args[1].0 {
                    return self.err("matrixCompMult of different matrices");
                }
                let cols = a.iter().zip(b).map(|(x, y)| Val::V(self.fbin(out, BinOp::Mul, x.v(), y.v()))).collect();
                (args[0].0.clone(), Val::Parts(cols))
            }
            "outerProduct" => {
                arity(self, 2)?;
                let (tc, c) = self.floats(out, &args[0])?;
                let (tr, r) = self.floats(out, &args[1])?;
                let (rows, cols) = (tc.lanes(), tr.lanes());
                let mut parts = Vec::new();
                for j in 0..cols {
                    let s = self.let_(out, Type::F32, Op::Extract(r, j));
                    let k = self.splat(out, s, Type::Vector(Scalar::F32, rows));
                    parts.push(Val::V(self.fbin(out, BinOp::Mul, c, k)));
                }
                (Ty::Matrix(cols, rows), Val::Parts(parts))
            }
            "transpose" => {
                arity(self, 1)?;
                let (Ty::Matrix(c, r), Val::Parts(cols)) = (&args[0].0, &args[0].1) else { return self.err("transpose of what isn't a matrix") };
                let (c, r) = (*c, *r);
                let m = self.matrix_scalars(out, cols, r);
                let mut parts = Vec::new();
                for i in 0..r {
                    let lanes = (0..c).map(|j| m[j as usize][i as usize]).collect();
                    parts.push(Val::V(self.let_(out, Type::Vector(Scalar::F32, c), Op::Construct(lanes))));
                }
                (Ty::Matrix(r, c), Val::Parts(parts))
            }
            "determinant" => {
                arity(self, 1)?;
                let (Ty::Matrix(c, r), Val::Parts(cols)) = (&args[0].0, &args[0].1) else { return self.err("determinant of what isn't a matrix") };
                if c != r {
                    return self.err("determinant of a matrix that isn't square");
                }
                let m = self.matrix_scalars(out, cols, *r);
                (types::FLOAT, Val::V(self.det(out, &m)))
            }
            "inverse" => {
                arity(self, 1)?;
                let (Ty::Matrix(c, r), Val::Parts(cols)) = (&args[0].0, &args[0].1) else { return self.err("inverse of what isn't a matrix") };
                if c != r {
                    return self.err("inverse of a matrix that isn't square");
                }
                let n = *c as usize;
                let m = self.matrix_scalars(out, cols, *r);
                let d = self.det(out, &m);
                let one = self.float_const(out, 1.0);
                let inv_d = self.fbin(out, BinOp::Div, one, d);
                // The adjugate over the determinant: entry (i, j) is cofactor (j, i).
                let mut parts = Vec::new();
                for col in 0..n {
                    let mut lanes = Vec::new();
                    for row in 0..n {
                        // Entry (row, col) of the inverse is cofactor (col, row): the minor without row `col` and column `row`.
                        let minor = minor(&m, row, col);
                        let cof = if minor.is_empty() { one } else { self.det(out, &minor) };
                        let cof = if (row + col) % 2 == 1 { self.fun(out, UnOp::Neg, cof) } else { cof };
                        lanes.push(self.fbin(out, BinOp::Mul, cof, inv_d));
                    }
                    parts.push(Val::V(self.let_(out, Type::Vector(Scalar::F32, n as u8), Op::Construct(lanes))));
                }
                (args[0].0.clone(), Val::Parts(parts))
            }
            // ---- Vector relations ----
            "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" | "equal" | "notEqual" => {
                arity(self, 2)?;
                let (a, b) = (&args[0], &args[1]);
                if a.0 != b.0 || !matches!(a.0, Ty::Vector(..)) || (a.0.base() == Some(Base::Bool) && !matches!(name, "equal" | "notEqual")) {
                    return self.err(&format!("{name} of {} and {}", a.0, b.0));
                }
                let cmp = match name {
                    "lessThan" => Cmp::Lt,
                    "lessThanEqual" => Cmp::Le,
                    "greaterThan" => Cmp::Gt,
                    "greaterThanEqual" => Cmp::Ge,
                    "equal" => Cmp::Eq,
                    _ => Cmp::Ne,
                };
                let rt = a.0.with_base(Base::Bool);
                (rt.clone(), Val::V(self.let_(out, rt.ir().expect("bvec"), Op::Compare(cmp, a.1.v(), b.1.v()))))
            }
            "any" | "all" | "not" => {
                arity(self, 1)?;
                let t = &args[0].0;
                if !matches!(t, Ty::Vector(Base::Bool, _)) {
                    return self.err(&format!("{name} of {t}"));
                }
                let v = args[0].1.v();
                match name {
                    "any" => (types::BOOL, Val::V(self.any(out, v))),
                    "all" => (types::BOOL, Val::V(self.all(out, v))),
                    _ => (t.clone(), Val::V(self.let_(out, t.ir().expect("bvec"), Op::Unary(UnOp::Not, v)))),
                }
            }
            // ---- The compatibility profile ----
            "ftransform" => {
                arity(self, 0)?;
                if !self.compat || self.stage != Stage::Vertex {
                    return self.err("ftransform is the compatibility profile's, in a vertex shader");
                }
                // gl_ModelViewProjectionMatrix * gl_Vertex, as fixed function transforms.
                let line = self.line;
                let named = |n: &str| super::ast::Expr { kind: super::ast::ExprKind::Ident(n.into()), line };
                let m = self.rvalue(out, &named("gl_ModelViewProjectionMatrix"))?;
                let v = self.rvalue(out, &named("gl_Vertex"))?;
                self.binary(out, super::ast::Binary::Mul, m, v)?
            }
            // ---- Textures ----
            "texture" | "texture2D" | "textureLod" | "texture2DLod" | "textureProj" | "texture2DProj" | "texture2DProjLod" | "textureProjLod" | "textureCube" | "textureCubeLod" => {
                let Some((sampler_ty, Val::Sampler(image))) = args.first().cloned() else { return self.err(&format!("{name} without a sampler")) };
                if args.len() < 2 {
                    return self.err(&format!("{name} without a coordinate"));
                }
                let cube = matches!(sampler_ty, Ty::Sampler(s) if s.dim == super::types::Dim::Cube);
                let (ct, coord) = self.floats(out, &args[1])?;
                let proj = name.contains("Proj");
                let coord = match (proj, ct.lanes()) {
                    (false, 3) if cube => self.cube_coord(out, image, coord)?,
                    _ if cube => return self.err(&format!("{name} on a samplerCube takes a vec3")),
                    (false, 2) => coord,
                    (true, 3 | 4) => {
                        let xy = vec![self.let_(out, Type::F32, Op::Extract(coord, 0)), self.let_(out, Type::F32, Op::Extract(coord, 1))];
                        let xy = self.let_(out, Type::Vector(Scalar::F32, 2), Op::Construct(xy));
                        let q = self.let_(out, Type::F32, Op::Extract(coord, ct.lanes() - 1));
                        let q = self.splat(out, q, Type::Vector(Scalar::F32, 2));
                        self.fbin(out, BinOp::Div, xy, q)
                    }
                    _ => return self.err(&format!("{name} with a {ct} coordinate")),
                };
                let lod_arg = if args.len() > 2 { Some(self.floats(out, &args[2])?.1) } else { None };
                let lod = match (name.contains("Lod"), lod_arg, self.stage) {
                    (true, Some(l), _) => Lod::Level(l),
                    (true, None, _) => return self.err(&format!("{name} without its level")),
                    // Outside fragment shaders there are no derivatives: the first level, as GLSL says.
                    (false, _, s) if s != Stage::Fragment => {
                        let zero = self.float_const(out, 0.0);
                        Lod::Level(zero)
                    }
                    (false, Some(b), _) => Lod::Bias(b),
                    (false, None, _) => Lod::Implicit,
                };
                let v = self.let_(out, Type::Vector(Scalar::F32, 4), Op::Sample { image, sampler: None, coord, lod });
                (Ty::Vector(Base::Float, 4), Val::V(v))
            }
            // A texel by its integer coordinates and level, and a level's size. The device samples only, so a texel is
            // sampled at its very centre at its level (exactly it, whatever the filter), the size coming from a
            // hidden uniform GL fills with the bound texture's.
            "texelFetch" | "textureSize" => {
                let Some((_, Val::Sampler(image))) = args.first().cloned() else { return self.err(&format!("{name} without a sampler")) };
                let cube = matches!(args[0].0, Ty::Sampler(s) if s.dim == super::types::Dim::Cube);
                if cube && name == "texelFetch" {
                    return self.err("texelFetch on a samplerCube");
                }
                let mut size = self.sampler_size(out, image)?;
                if cube {
                    // A cube's faces lie side by side: one face's size is a sixth of the width.
                    let sixth = self.float_const(out, 1.0 / 6.0);
                    let one = self.float_const(out, 1.0);
                    let scale = self.let_(out, Type::Vector(Scalar::F32, 2), Op::Construct(vec![sixth, one]));
                    size = self.fbin(out, BinOp::Mul, size, scale);
                }
                let lod_index = if name == "texelFetch" { 2 } else { 1 };
                let lod = match args.get(lod_index) {
                    Some(a) => self.as_floats(out, a)?,
                    None => self.float_const(out, 0.0),
                };
                // The level's size: halved for each level, at least 1.
                let scale = self.fun(out, UnOp::Exp2, lod);
                let scale = self.splat(out, scale, Type::Vector(Scalar::F32, 2));
                let level = self.fbin(out, BinOp::Div, size, scale);
                let level = self.fun(out, UnOp::Floor, level);
                let one = self.float_const(out, 1.0);
                let one = self.splat(out, one, Type::Vector(Scalar::F32, 2));
                let level = self.fbin(out, BinOp::Max, level, one);
                if name == "textureSize" {
                    let v = self.convert(out, level, &Ty::Vector(Base::Float, 2), Base::Int);
                    (Ty::Vector(Base::Int, 2), Val::V(v))
                } else {
                    if args.len() < 3 {
                        return self.err("texelFetch takes a sampler, a coordinate and a level");
                    }
                    let p = self.as_floats(out, &args[1])?;
                    let half = self.float_const(out, 0.5);
                    let half = self.splat(out, half, Type::Vector(Scalar::F32, 2));
                    let centre = self.fbin(out, BinOp::Add, p, half);
                    let coord = self.fbin(out, BinOp::Div, centre, level);
                    let v = self.let_(out, Type::Vector(Scalar::F32, 4), Op::Sample { image, sampler: None, coord, lod: Lod::Level(lod) });
                    (Ty::Vector(Base::Float, 4), Val::V(v))
                }
            }
            // ---- Derivatives ----
            "dFdx" | "dFdy" | "fwidth" | "dFdxFine" | "dFdyFine" | "dFdxCoarse" | "dFdyCoarse" | "fwidthFine" | "fwidthCoarse" => {
                arity(self, 1)?;
                if self.stage != Stage::Fragment {
                    return self.err(&format!("{name} outside a fragment shader"));
                }
                let (t, v) = self.floats(out, &args[0])?;
                let ir = t.ir().expect("float");
                let r = if name.starts_with("fwidth") {
                    let x = self.let_(out, ir, Op::Derivative(Axis::X, v));
                    let y = self.let_(out, ir, Op::Derivative(Axis::Y, v));
                    let (ax, ay) = (self.fun(out, UnOp::Abs, x), self.fun(out, UnOp::Abs, y));
                    self.fbin(out, BinOp::Add, ax, ay)
                } else {
                    let axis = if name.starts_with("dFdx") { Axis::X } else { Axis::Y };
                    self.let_(out, ir, Op::Derivative(axis, v))
                };
                (t, Val::V(r))
            }
            // ---- Compute ----
            "barrier" | "memoryBarrier" | "memoryBarrierShared" | "memoryBarrierBuffer" | "groupMemoryBarrier" => {
                arity(self, 0)?;
                if self.stage != Stage::Compute {
                    if name == "barrier" {
                        return self.err("barrier outside a compute kernel");
                    }
                    return Ok(Some((Ty::Void, Val::Void)));
                }
                let b = match name {
                    "barrier" => Barrier { execution: true, workgroup_memory: true, device_memory: false },
                    "memoryBarrierShared" => Barrier { execution: false, workgroup_memory: true, device_memory: false },
                    "memoryBarrierBuffer" => Barrier { execution: false, workgroup_memory: false, device_memory: true },
                    _ => Barrier { execution: false, workgroup_memory: true, device_memory: true },
                };
                out.push(Stmt::Barrier(b));
                (Ty::Void, Val::Void)
            }
            "atomicAdd" | "atomicMin" | "atomicMax" | "atomicAnd" | "atomicOr" | "atomicXor" | "atomicExchange" | "atomicCompSwap" => {
                let Some(Some(Place::Mem { base, offset, dynamic, ty, .. })) = places.first().cloned() else { return self.err(&format!("{name} of what isn't in a buffer or shared memory")) };
                let s = match ty {
                    Ty::Scalar(Base::Int) => Scalar::I32,
                    Ty::Scalar(Base::Uint) => Scalar::U32,
                    _ => return self.err(&format!("{name} of {ty}")),
                };
                let at = self.mem_address(out, base, offset, dynamic);
                let mut vals = Vec::new();
                for a in &args[1..] {
                    let v = self.coerce(out, a.1.clone(), &a.0, &ty)?;
                    vals.push(v.v());
                }
                let r = if name == "atomicCompSwap" {
                    if vals.len() != 2 {
                        return self.err("atomicCompSwap takes a compare and a value");
                    }
                    self.let_(out, Type::Scalar(s), Op::CompareExchange(at, vals[0], vals[1], s))
                } else {
                    if vals.len() != 1 {
                        return self.err(&format!("{name} takes a value"));
                    }
                    let op = match name {
                        "atomicAdd" => AtomicOp::Add,
                        "atomicMin" => AtomicOp::Min,
                        "atomicMax" => AtomicOp::Max,
                        "atomicAnd" => AtomicOp::And,
                        "atomicOr" => AtomicOp::Or,
                        "atomicXor" => AtomicOp::Xor,
                        _ => AtomicOp::Exchange,
                    };
                    self.let_(out, Type::Scalar(s), Op::Atomic(op, at, vals[0], s))
                };
                (ty, Val::V(r))
            }
            _ => return Ok(None),
        };
        Ok(Some(r))
    }

    fn mem_address(&mut self, out: &mut Block, base: Base_, offset: u32, dynamic: Option<Value>) -> Value {
        let (space, op) = match base {
            Base_::Binding(n) => (gpu_ir::Space::Device, Op::Binding(n)),
            Base_::Workgroup(n) => (gpu_ir::Space::Workgroup, Op::Workgroup(n)),
        };
        let mut at = self.let_(out, Type::Ptr(space), op);
        if offset != 0 {
            let o = self.uint_const(out, offset);
            at = self.let_(out, Type::Ptr(space), Op::Offset(at, o));
        }
        if let Some(d) = dynamic {
            at = self.let_(out, Type::Ptr(space), Op::Offset(at, d));
        }
        at
    }

    // ---- Helpers ----

    fn fun(&mut self, out: &mut Block, op: UnOp, x: Value) -> Value {
        let ty = self.e.ty(x);
        self.let_(out, ty, Op::Unary(op, x))
    }

    fn fbin(&mut self, out: &mut Block, op: BinOp, a: Value, b: Value) -> Value {
        let ty = self.e.ty(a);
        self.let_(out, ty, Op::Binary(op, a, b))
    }

    /// A float constant of another value's shape.
    fn fsplat(&mut self, out: &mut Block, f: f32, like: Value) -> Value {
        let ty = self.e.ty(like);
        let k = self.float_const(out, f);
        self.splat(out, k, ty)
    }

    /// A constant of a type (float, int or uint lanes).
    fn konst_like(&mut self, out: &mut Block, f: f64, ty: Type) -> Value {
        let k = match ty.scalar() {
            Some(Scalar::I32) => self.int_const(out, f as i64),
            Some(Scalar::U32) => self.uint_const(out, f as u32),
            _ => self.float_const(out, f as f32),
        };
        self.splat(out, k, ty)
    }

    fn exp(&mut self, out: &mut Block, x: Value) -> Value {
        let k = self.fsplat(out, core::f32::consts::LOG2_E, x);
        let y = self.fbin(out, BinOp::Mul, x, k);
        self.fun(out, UnOp::Exp2, y)
    }

    fn log(&mut self, out: &mut Block, x: Value) -> Value {
        let l = self.fun(out, UnOp::Log2, x);
        let k = self.fsplat(out, core::f32::consts::LN_2, x);
        self.fbin(out, BinOp::Mul, l, k)
    }

    /// `mod(x, y)`: the remainder with the divisor's sign.
    fn fmod(&mut self, out: &mut Block, p: Value, q: Value) -> Value {
        let ty = self.e.ty(p);
        let r = self.let_(out, ty, Op::Binary(BinOp::Rem, p, q));
        let zero = self.fsplat(out, 0.0, p);
        let bools = ty.with_scalar(Scalar::Bool);
        let r_neg = self.let_(out, bools, Op::Compare(Cmp::Lt, r, zero));
        let q_neg = self.let_(out, bools, Op::Compare(Cmp::Lt, q, zero));
        let differ = self.let_(out, bools, Op::Compare(Cmp::Ne, r_neg, q_neg));
        let nonzero = self.let_(out, bools, Op::Compare(Cmp::Ne, r, zero));
        let fix = self.let_(out, bools, Op::Binary(BinOp::And, differ, nonzero));
        let fixed = self.let_(out, ty, Op::Binary(BinOp::Add, r, q));
        self.let_(out, ty, Op::Select(fix, fixed, r))
    }

    pub fn length(&mut self, out: &mut Block, v: Value) -> Value {
        if self.e.ty(v).lanes() == 1 {
            return self.fun(out, UnOp::Abs, v);
        }
        let d = self.dot(out, v, v);
        self.fun(out, UnOp::Sqrt, d)
    }

    /// `asin` (or `acos`): Abramowitz and Stegun's polynomial in |x| times √(1 − |x|), its coefficients as published.
    #[allow(clippy::excessive_precision)]
    fn asin_acos(&mut self, out: &mut Block, x: Value, acos: bool) -> Value {
        let ty = self.e.ty(x);
        let bools = ty.with_scalar(Scalar::Bool);
        let zero = self.fsplat(out, 0.0, x);
        let neg = self.let_(out, bools, Op::Compare(Cmp::Lt, x, zero));
        let a = self.fun(out, UnOp::Abs, x);
        let mut p = self.fsplat(out, -0.0012624911, x);
        for c in [0.0066700901, -0.0170881256, 0.0308918810, -0.0501743046, 0.0889789874, -0.2145988016, 1.5707963050] {
            let m = self.fbin(out, BinOp::Mul, p, a);
            let k = self.fsplat(out, c, x);
            p = self.fbin(out, BinOp::Add, m, k);
        }
        let one = self.fsplat(out, 1.0, x);
        let rest = self.fbin(out, BinOp::Sub, one, a);
        let rest = self.fbin(out, BinOp::Max, rest, zero);
        let root = self.fun(out, UnOp::Sqrt, rest);
        // acos(|x|) = √(1 − |x|)·p(|x|).
        let acos_abs = self.fbin(out, BinOp::Mul, root, p);
        if acos {
            let pi = self.fsplat(out, PI, x);
            let other = self.fbin(out, BinOp::Sub, pi, acos_abs);
            return self.let_(out, ty, Op::Select(neg, other, acos_abs));
        }
        let half_pi = self.fsplat(out, PI / 2.0, x);
        let asin_abs = self.fbin(out, BinOp::Sub, half_pi, acos_abs);
        let negated = self.fun(out, UnOp::Neg, asin_abs);
        self.let_(out, ty, Op::Select(neg, negated, asin_abs))
    }

    /// `atan(y, x)`: a polynomial for the angle below π/4, then its octant.
    #[allow(clippy::excessive_precision)]
    fn atan2(&mut self, out: &mut Block, y: Value, x: Value) -> Value {
        let ty = self.e.ty(y);
        let bools = ty.with_scalar(Scalar::Bool);
        let (ax, ay) = (self.fun(out, UnOp::Abs, x), self.fun(out, UnOp::Abs, y));
        let big = self.fbin(out, BinOp::Max, ax, ay);
        let small = self.fbin(out, BinOp::Min, ax, ay);
        let t = self.fbin(out, BinOp::Div, small, big);
        // 0/0 is atan(0, 0) = 0.
        let zero = self.fsplat(out, 0.0, y);
        let nan = self.let_(out, bools, Op::Compare(Cmp::Ne, t, t));
        let t = self.let_(out, ty, Op::Select(nan, zero, t));
        let t2 = self.fbin(out, BinOp::Mul, t, t);
        // Abramowitz and Stegun 4.4.49: within 2e-8 on [0, 1].
        let mut p = self.fsplat(out, 0.0028662257, y);
        for c in [-0.0161657367, 0.0429096138, -0.0752896400, 0.1065626393, -0.1420889944, 0.1999355085, -0.3333314528, 1.0] {
            let m = self.fbin(out, BinOp::Mul, p, t2);
            let k = self.fsplat(out, c, y);
            p = self.fbin(out, BinOp::Add, m, k);
        }
        let mut r = self.fbin(out, BinOp::Mul, p, t);
        let steep = self.let_(out, bools, Op::Compare(Cmp::Gt, ay, ax));
        let half_pi = self.fsplat(out, PI / 2.0, y);
        let flipped = self.fbin(out, BinOp::Sub, half_pi, r);
        r = self.let_(out, ty, Op::Select(steep, flipped, r));
        let left = self.let_(out, bools, Op::Compare(Cmp::Lt, x, zero));
        let pi = self.fsplat(out, PI, y);
        let mirrored = self.fbin(out, BinOp::Sub, pi, r);
        r = self.let_(out, ty, Op::Select(left, mirrored, r));
        let below = self.let_(out, bools, Op::Compare(Cmp::Lt, y, zero));
        let negated = self.fun(out, UnOp::Neg, r);
        self.let_(out, ty, Op::Select(below, negated, r))
    }

    /// A cube map's direction as the coordinate of its texel in the image its six faces lie side by side in (+X, -X,
    /// +Y, -Y, +Z, -Z): the face of the direction's major axis, and s and t on it as GL's table has them, kept half a
    /// texel inside the face so its neighbour's edge isn't filtered in.
    fn cube_coord(&mut self, out: &mut Block, image: u32, dir: Value) -> Result<Value, Error> {
        let f = Type::F32;
        let lane = |me: &mut Self, out: &mut Block, i: u8| me.let_(out, f, Op::Extract(dir, i));
        let (x, y, z) = (lane(self, out, 0), lane(self, out, 1), lane(self, out, 2));
        let (ax, ay, az) = (self.fun(out, UnOp::Abs, x), self.fun(out, UnOp::Abs, y), self.fun(out, UnOp::Abs, z));
        let (nx, ny, nz) = (self.fun(out, UnOp::Neg, x), self.fun(out, UnOp::Neg, y), self.fun(out, UnOp::Neg, z));
        let zero = self.float_const(out, 0.0);
        let cmp = |me: &mut Self, out: &mut Block, c: Cmp, a: Value, b: Value| me.let_(out, Type::BOOL, Op::Compare(c, a, b));
        let (x_ge_y, x_ge_z, y_ge_z) = (cmp(self, out, Cmp::Ge, ax, ay), cmp(self, out, Cmp::Ge, ax, az), cmp(self, out, Cmp::Ge, ay, az));
        let x_major = self.let_(out, Type::BOOL, Op::Binary(BinOp::And, x_ge_y, x_ge_z));
        let (xpos, ypos, zpos) = (cmp(self, out, Cmp::Gt, x, zero), cmp(self, out, Cmp::Gt, y, zero), cmp(self, out, Cmp::Gt, z, zero));
        let sel = |me: &mut Self, out: &mut Block, c: Value, a: Value, b: Value| me.let_(out, f, Op::Select(c, a, b));
        let consts: Vec<Value> = (0..6).map(|i| self.float_const(out, i as f32)).collect();
        // The face, its major axis's length, and s and t before they're divided by it.
        let face_x = sel(self, out, xpos, consts[0], consts[1]);
        let face_y = sel(self, out, ypos, consts[2], consts[3]);
        let face_z = sel(self, out, zpos, consts[4], consts[5]);
        let face_yz = sel(self, out, y_ge_z, face_y, face_z);
        let face = sel(self, out, x_major, face_x, face_yz);
        let ma_yz = sel(self, out, y_ge_z, ay, az);
        let ma = sel(self, out, x_major, ax, ma_yz);
        let sc_x = sel(self, out, xpos, nz, z);
        let sc_z = sel(self, out, zpos, x, nx);
        let sc_yz = sel(self, out, y_ge_z, x, sc_z);
        let sc = sel(self, out, x_major, sc_x, sc_yz);
        let tc_y = sel(self, out, ypos, z, nz);
        let tc_yz = sel(self, out, y_ge_z, tc_y, ny);
        let tc = sel(self, out, x_major, ny, tc_yz);
        let half = self.float_const(out, 0.5);
        let one = self.float_const(out, 1.0);
        let st = |me: &mut Self, out: &mut Block, c: Value| {
            let d = me.fbin(out, BinOp::Div, c, ma);
            let d = me.fbin(out, BinOp::Add, d, one);
            me.fbin(out, BinOp::Mul, d, half)
        };
        let (s, t) = (st(self, out, sc), st(self, out, tc));
        // Half a texel inside the face: one face is a sixth of the image's width.
        let size = self.sampler_size(out, image)?;
        let w = self.let_(out, f, Op::Extract(size, 0));
        let h = self.let_(out, f, Op::Extract(size, 1));
        let six = self.float_const(out, 6.0);
        let face_w = self.fbin(out, BinOp::Div, w, six);
        let inside = |me: &mut Self, out: &mut Block, v: Value, n: Value| {
            let lo = me.fbin(out, BinOp::Div, half, n);
            let hi = me.fbin(out, BinOp::Sub, one, lo);
            let v = me.fbin(out, BinOp::Max, v, lo);
            me.fbin(out, BinOp::Min, v, hi)
        };
        let (s, t) = (inside(self, out, s, face_w), inside(self, out, t, h));
        let across = self.fbin(out, BinOp::Add, face, s);
        let u = self.fbin(out, BinOp::Div, across, six);
        Ok(self.let_(out, Type::Vector(Scalar::F32, 2), Op::Construct(vec![u, t])))
    }

    /// An integer or float argument as floats, converted as a constructor converts (texel coordinates and levels are
    /// integers, which GLSL ES doesn't convert by itself).
    fn as_floats(&mut self, out: &mut Block, (t, v): &(Ty, Val)) -> Result<Value, Error> {
        match t.base() {
            Some(Base::Float) => Ok(v.v()),
            Some(Base::Int | Base::Uint) if t.is_scalar_or_vector() => Ok(self.convert(out, v.v(), t, Base::Float)),
            _ => self.err(&format!("{t} where a number is wanted")),
        }
    }

    /// A sampler's level-0 size, as a `vec2`: the hidden uniform GL fills with its texture's, declared the first time.
    fn sampler_size(&mut self, out: &mut Block, image: u32) -> Result<Value, Error> {
        let place = match self.sizes.get(&image) {
            Some(p) => p.clone(),
            None => {
                let Some(name) = self.iface.uniforms.iter().find(|u| u.binding.map(|b| b.0) == Some(image)).map(|u| u.name.clone()) else {
                    return self.err("a sampler with no name");
                };
                let p = self.uniform_place(&format!("carbon_size_{name}"), Ty::Vector(Base::Float, 2), None, None)?;
                self.sizes.insert(image, p.clone());
                p
            }
        };
        Ok(self.load(out, &place)?.v())
    }

    /// A scalar or vector argument as floats: integers converted where GLSL converts them.
    fn floats(&mut self, out: &mut Block, (t, v): &(Ty, Val)) -> Result<(Ty, Value), Error> {
        match t.base() {
            Some(Base::Float) if t.is_scalar_or_vector() => Ok((t.clone(), v.v())),
            Some(Base::Int | Base::Uint) if t.is_scalar_or_vector() && self.implicit(t, &t.with_base(Base::Float)) => {
                let f = t.with_base(Base::Float);
                Ok((f, self.convert(out, v.v(), t, Base::Float)))
            }
            _ => self.err(&format!("{t} where a float is wanted")),
        }
    }

    /// A float function of each lane: its arguments made floats and spread to the widest one's lanes.
    fn float_map(&mut self, out: &mut Block, args: &[(Ty, Val)], f: impl FnOnce(&mut Self, &mut Block, &[Value]) -> Value) -> Result<(Ty, Val), Error> {
        let mut xs = Vec::new();
        for a in args {
            xs.push(self.floats(out, a)?);
        }
        let lanes = xs.iter().map(|x| x.0.lanes()).max().unwrap_or(1);
        let ty = Ty::vector(Base::Float, lanes);
        let ir = ty.ir().expect("float");
        let mut vals = Vec::new();
        for (t, v) in xs {
            if t.lanes() != lanes && t.lanes() != 1 {
                return self.err(&format!("{t} with {ty}"));
            }
            vals.push(if t.lanes() == 1 { self.splat(out, v, ir) } else { v });
        }
        let r = f(self, out, &vals);
        Ok((ty, Val::V(r)))
    }

    /// A numeric function of each lane over floats, ints or uints (the first argument's kind); `spread`: later
    /// scalar arguments are spread to its lanes.
    fn num_map(&mut self, out: &mut Block, args: &[(Ty, Val)], spread: bool, f: impl FnOnce(&mut Self, &mut Block, &[Value]) -> Value) -> Result<(Ty, Val), Error> {
        let first = args[0].0.clone();
        if first.base() == Some(Base::Float) || args.iter().any(|a| a.0.base() == Some(Base::Float)) {
            return self.float_map(out, args, f);
        }
        if !matches!(first.base(), Some(Base::Int | Base::Uint)) || !first.is_scalar_or_vector() {
            return self.err(&format!("{first} where a number is wanted"));
        }
        let ir = first.ir().expect("integer");
        let mut vals = Vec::new();
        for (t, v) in args {
            let v = if t.base() != first.base() {
                if !self.implicit(t, &t.with_base(first.base().expect("int"))) {
                    return self.err(&format!("{t} with {first}"));
                }
                self.convert(out, v.v(), t, first.base().expect("int"))
            } else {
                v.v()
            };
            vals.push(if t.lanes() == 1 && first.lanes() > 1 && spread {
                self.splat(out, v, ir)
            } else if t.lanes() != first.lanes() {
                return self.err(&format!("{t} with {first}"));
            } else {
                v
            });
        }
        let r = f(self, out, &vals);
        Ok((first, Val::V(r)))
    }

    /// A matrix's entries: `m[column][row]`.
    fn matrix_scalars(&mut self, out: &mut Block, cols: &[Val], rows: u8) -> Vec<Vec<Value>> {
        cols.iter().map(|c| (0..rows).map(|j| self.let_(out, Type::F32, Op::Extract(c.v(), j))).collect()).collect()
    }

    /// A square matrix's determinant, by cofactors along its first column.
    fn det(&mut self, out: &mut Block, m: &[Vec<Value>]) -> Value {
        let n = m.len();
        if n == 1 {
            return m[0][0];
        }
        if n == 2 {
            let a = self.fbin(out, BinOp::Mul, m[0][0], m[1][1]);
            let b = self.fbin(out, BinOp::Mul, m[1][0], m[0][1]);
            return self.fbin(out, BinOp::Sub, a, b);
        }
        let mut sum: Option<Value> = None;
        for row in 0..n {
            let sub = minor(m, 0, row);
            let d = self.det(out, &sub);
            let term = self.fbin(out, BinOp::Mul, m[0][row], d);
            sum = Some(match sum {
                None => term,
                Some(s) if row % 2 == 1 => self.fbin(out, BinOp::Sub, s, term),
                Some(s) => self.fbin(out, BinOp::Add, s, term),
            });
        }
        sum.expect("a row")
    }
}

/// A matrix without one column and one row.
fn minor(m: &[Vec<Value>], col: usize, row: usize) -> Vec<Vec<Value>> {
    m.iter().enumerate().filter(|(c, _)| *c != col).map(|(_, column)| column.iter().enumerate().filter(|(r, _)| *r != row).map(|(_, v)| *v).collect()).collect()
}
