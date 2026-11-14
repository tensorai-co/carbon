//! The validator refuses what a backend couldn't trust.

use crate::{BinOp, Entry, Op, Scalar, Stmt, Type, validate};

fn entry() -> Entry {
    Entry::new("t", [1, 1, 1])
}

#[test]
fn a_well_formed_entry_passes() {
    let mut e = entry();
    let a = e.value(Type::U32);
    let b = e.value(Type::U32);
    let v = e.var(Type::U32);
    e.body = vec![
        Stmt::Let(a, Op::Const(Scalar::U32, 1)),
        Stmt::Let(b, Op::Binary(BinOp::Add, a, a)),
        Stmt::Write(v, b),
        Stmt::Loop { body: vec![Stmt::Break], continuing: vec![], break_if: None },
    ];
    assert_eq!(validate(&e), Ok(()));
    assert!(format!("{e}").contains("Add %0, %0"));
}

fn refused(e: &Entry, says: &str) {
    match validate(e) {
        Ok(()) => panic!("accepted:\n{e}"),
        Err(err) => assert!(err.message.contains(says), "{:?} doesn't say {says:?}", err.message),
    }
}

#[test]
fn use_before_definition() {
    let mut e = entry();
    let a = e.value(Type::U32);
    let b = e.value(Type::U32);
    e.body = vec![Stmt::Let(b, Op::Binary(BinOp::Add, a, a)), Stmt::Let(a, Op::Const(Scalar::U32, 1))];
    refused(&e, "before it's defined");
}

#[test]
fn defined_twice() {
    let mut e = entry();
    let a = e.value(Type::U32);
    e.body = vec![Stmt::Let(a, Op::Const(Scalar::U32, 1)), Stmt::Let(a, Op::Const(Scalar::U32, 2))];
    refused(&e, "twice");
}

#[test]
fn mismatched_types() {
    let mut e = entry();
    let a = e.value(Type::U32);
    let f = e.value(Type::F32);
    let r = e.value(Type::U32);
    e.body = vec![
        Stmt::Let(a, Op::Const(Scalar::U32, 1)),
        Stmt::Let(f, Op::Const(Scalar::F32, 0)),
        Stmt::Let(r, Op::Binary(BinOp::Add, a, f)),
    ];
    refused(&e, "is Scalar(F32)");
    let mut e = entry();
    let a = e.value(Type::F32);
    e.body = vec![Stmt::Let(a, Op::Const(Scalar::U32, 1))];
    refused(&e, "declared");
}

#[test]
fn breaks_and_continues_outside_loops() {
    let mut e = entry();
    e.body = vec![Stmt::Break];
    refused(&e, "outside any loop");
    e.body = vec![Stmt::Loop { body: vec![], continuing: vec![Stmt::Continue], break_if: None }];
    refused(&e, "continue outside");
}

#[test]
fn booleans_stay_out_of_memory() {
    let mut e = entry();
    e.private.push(crate::Region { size: 4, align: 4 });
    let p = e.value(Type::Ptr(crate::Space::Private));
    let t = e.value(Type::BOOL);
    e.body = vec![Stmt::Let(p, Op::Private(0)), Stmt::Let(t, Op::Const(Scalar::Bool, 1)), Stmt::Store(p, t)];
    refused(&e, "booleans aren't stored");
}

#[test]
fn each_stage_keeps_to_its_own() {
    use crate::{Axis, Builtin, Interpolation, Io, Slot, Stage};
    let vec4 = Type::Vector(Scalar::F32, 4);

    // A derivative, a discard and a stage output belong to the graphics stages.
    let mut e = entry();
    let x = e.value(Type::F32);
    let d = e.value(Type::F32);
    e.body = vec![Stmt::Let(x, Op::Const(Scalar::F32, 0)), Stmt::Let(d, Op::Derivative(Axis::X, x))];
    refused(&e, "outside a fragment shader");
    e.body = vec![Stmt::Discard];
    refused(&e, "outside a fragment shader");

    // A vertex shader can't read a fragment's builtins, and its position is a vec4.
    let mut v = Entry::shader("v", Stage::Vertex);
    let c = v.value(Type::Vector(Scalar::F32, 4));
    v.body = vec![Stmt::Let(c, Op::Builtin(Builtin::FragCoord))];
    refused(&v, "FragCoord in a Vertex shader");
    let mut v = Entry::shader("v", Stage::Vertex);
    v.outputs.push(Io { slot: Slot::Position, ty: Type::F32, interpolation: Interpolation::Smooth });
    refused(&v, "Position");

    // What's written to an output has its type.
    let mut f = Entry::shader("f", Stage::Fragment);
    f.outputs.push(Io { slot: Slot::Location(0), ty: vec4, interpolation: Interpolation::Smooth });
    let one = f.value(Type::F32);
    f.body = vec![Stmt::Let(one, Op::Const(Scalar::F32, 0)), Stmt::Output(0, one)];
    refused(&f, "output 0 is");
}
