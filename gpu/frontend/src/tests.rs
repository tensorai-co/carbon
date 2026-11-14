//! Every conformance program reads and validates, and the IR has the shape its source promises.

use std::path::PathBuf;

/// A file of the conformance programs, by its whole name.
fn file(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/programs").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn program(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/programs").join(format!("{name}.spv"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const PROGRAMS: &[&str] = &[
    "copy", "reduce", "atomics", "subgroup", "stages", "branches", "cube.vert", "cube.frag", "colors.vert",
    "colors.frag", "textured.vert", "textured.frag",
];

#[test]
fn every_program_reads() {
    for name in PROGRAMS {
        let module = crate::spirv::read(&program(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(module.entries.len(), 1, "{name}");
        if std::env::var_os("GPU_PRINT_IR").is_some() {
            println!("{}", module.entries[0]);
        }
    }
}

#[test]
fn bindings_and_sizes() {
    let m = crate::spirv::read(&program("reduce")).unwrap();
    let e = &m.entries[0];
    assert_eq!(e.workgroup_size, [128, 1, 1]);
    assert_eq!(e.bindings.len(), 2);
    assert_eq!(e.bindings[0].kind, gpu_ir::BindingKind::Storage { writable: false });
    assert_eq!(e.bindings[1].kind, gpu_ir::BindingKind::Storage { writable: true });
    assert_eq!(e.workgroup, vec![gpu_ir::Region { size: 512, align: 4 }]);

    let m = crate::spirv::read(&program("stages")).unwrap();
    assert_eq!(m.entries[0].bindings[0].kind, gpu_ir::BindingKind::Uniform);
}

#[test]
fn malformed_modules_are_refused() {
    assert!(crate::spirv::read(&[1, 2, 3]).is_err());
    assert!(crate::spirv::read(&[0; 20]).is_err());
    let mut copy = program("copy");
    copy.truncate(copy.len() - 8);
    assert!(crate::spirv::read(&copy).is_err());
}

#[test]
fn shaders_have_their_stages_inputs_and_outputs() {
    use gpu_ir::{BindingKind, Interpolation, Io, Scalar, Slot, Stage, Type};
    let vec4 = Type::Vector(Scalar::F32, 4);

    let m = crate::spirv::read(&program("cube.vert")).unwrap();
    let e = &m.entries[0];
    assert_eq!(e.stage, Stage::Vertex);
    assert_eq!(e.bindings[0].kind, BindingKind::Uniform);
    let slots: Vec<Slot> = e.outputs.iter().map(|o| o.slot).collect();
    assert_eq!(slots, vec![Slot::Location(0), Slot::Position, Slot::Location(1)]);

    let m = crate::spirv::read(&program("cube.frag")).unwrap();
    let e = &m.entries[0];
    assert_eq!(e.stage, Stage::Fragment);
    assert_eq!(e.bindings[0].kind, BindingKind::Combined);
    assert_eq!(e.outputs, vec![Io { slot: Slot::Location(0), ty: vec4, interpolation: Interpolation::Smooth }]);

    let m = crate::spirv::read(&program("colors.frag")).unwrap();
    let e = &m.entries[0];
    assert_eq!(e.inputs[1].interpolation, Interpolation::Flat);
    assert!(format!("{e}").contains("discard"));

    let m = crate::spirv::read(&program("textured.frag")).unwrap();
    let kinds: Vec<BindingKind> = m.entries[0].bindings.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, vec![BindingKind::Image, BindingKind::Sampler]);
}

#[path = "../../examples/dxbc_asm.rs"]
mod dxbc_asm;

#[test]
fn dxbc_signatures_and_stages() {
    use dxbc_asm::*;
    use gpu_ir::{Interpolation, Slot, Stage};
    let mut ps = Asm::new(0);
    ps.op(op::DCL_INPUT_PS, 1 << 11, &[dst(INPUT, &[2], "x")], &[])
        .op(op::DCL_OUTPUT, 0, &[dst(OUTPUT, &[0], "xyzw")], &[])
        .i(op::UTOF, &[dst(OUTPUT, &[0], "xyzw"), src(INPUT, &[2], "xxxx")])
        .i(op::RET, &[]);
    let bytes = ps.finish(
        &[Sig { name: "BLENDINDICES", index: 3, system: 0, component: 1, register: 2, mask: 1 }],
        &[Sig { name: "SV_Target", index: 0, system: 64, component: 3, register: 0, mask: 15 }],
    );
    let s = crate::dxbc::read(&bytes).unwrap();
    assert_eq!(s.entry.stage, Stage::Fragment);
    assert_eq!((s.inputs[0].semantic.as_str(), s.inputs[0].index, s.inputs[0].register), ("BLENDINDICES", 3, 2));
    assert_eq!(s.inputs[0].component, crate::dxbc::Component::Uint);
    let input = s.entry.inputs[0];
    assert_eq!((input.slot, input.ty, input.interpolation), (Slot::Location(2), gpu_ir::Type::Vector(gpu_ir::Scalar::U32, 4), Interpolation::Flat));
    assert_eq!(s.entry.outputs[0].slot, Slot::Location(0));
}

#[test]
fn dxbc_refusals_name_what() {
    use dxbc_asm::*;
    assert!(crate::dxbc::read(b"DXBC").is_err());
    assert!(crate::dxbc::read(&[0; 64]).is_err());
    let mut cs = Asm::new(5);
    cs.op(op::DCL_THREAD_GROUP, 0, &[], &[1, 1, 1]).op(op::DCL_TEMPS, 0, &[], &[1]).i(45, &[r(0, "x"), src(TEMP, &[0], "xxxx"), src(RESOURCE, &[0], "xxxx")]);
    let why = crate::dxbc::read(&cs.finish(&[], &[])).unwrap_err().message;
    assert!(why.contains("texel fetches"), "{why}");
    let mut gs = Asm::new(2);
    gs.i(op::RET, &[]);
    let why = crate::dxbc::read(&gs.finish(&[], &[])).unwrap_err().message;
    assert!(why.contains("geometry"), "{why}");
    // A register used before it's declared.
    let mut cs = Asm::new(5);
    cs.op(op::DCL_THREAD_GROUP, 0, &[], &[1, 1, 1]).i(op::MOV, &[r(3, "x"), l1(1)]);
    let why = crate::dxbc::read(&cs.finish(&[], &[])).unwrap_err().message;
    assert!(why.contains("r3"), "{why}");
    // A truncated container.
    let mut cs = Asm::new(5);
    cs.op(op::DCL_THREAD_GROUP, 0, &[], &[1, 1, 1]).i(op::RET, &[]);
    let mut bytes = cs.finish(&[], &[]);
    bytes.truncate(bytes.len() - 8);
    assert!(crate::dxbc::read(&bytes).is_err());
}

/// Every DXBC program vkd3d's compiler wrote for the suite reads, with the signatures it declares.
#[test]
fn every_dxbc_program_reads() {
    use gpu_ir::{Slot, Stage};
    for name in ["arith", "flow", "shared", "quad.vs", "quad.ps"] {
        let bytes = file(&format!("{name}.dxbc"));
        let s = crate::dxbc::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        if std::env::var_os("GPU_PRINT_IR").is_some() {
            println!("{}", s.entry);
        }
    }
    let ps = crate::dxbc::read(&file("quad.ps.dxbc")).unwrap();
    assert_eq!(ps.entry.stage, Stage::Fragment);
    let names: Vec<(&str, u32, u32)> = ps.inputs.iter().map(|e| (e.semantic.as_str(), e.system, e.register)).collect();
    assert_eq!(names, vec![("SV_Position", 1, 0), ("TEXCOORD", 0, 1)]);
    assert_eq!(ps.entry.inputs.iter().map(|i| i.slot).collect::<Vec<_>>(), vec![Slot::Location(1)]);
    assert_eq!(ps.entry.outputs.iter().map(|o| o.slot).collect::<Vec<_>>(), vec![Slot::Location(0), Slot::Location(1)]);
    let vs = crate::dxbc::read(&file("quad.vs.dxbc")).unwrap();
    assert!(vs.entry.outputs.iter().any(|o| o.slot == Slot::Position));
}
