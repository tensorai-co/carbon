//! Every conformance program reads and validates, and the IR has the shape its source promises.

use std::path::PathBuf;

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
