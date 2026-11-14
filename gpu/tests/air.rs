//! The AIR backend held to Apple's toolchain, where it's installed: every suite program's library passes
//! `air-validate`, and its module disassembles with `air-opt`: the compute kernels, and the draw tests' vertex and
//! fragment shaders (a vertex shader with a layout giving each of its inputs an attribute). Skipped, saying why, on a machine without the Metal
//! toolchain (`xcodebuild -downloadComponent MetalToolchain`).
//!
//! `GPU_AIR_DUMP=dir` keeps each program's `.air` and `.metallib` there, for a closer look.

#[path = "../examples/suite.rs"]
#[allow(dead_code)]
mod suite;

use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> Option<PathBuf> {
    let out = Command::new("xcrun").args(["-f", name]).output().ok()?;
    out.status.success().then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

fn dir() -> PathBuf {
    match std::env::var_os("GPU_AIR_DUMP") {
        Some(d) => PathBuf::from(d),
        None => std::env::temp_dir().join(format!("gpu-air-{}", std::process::id())),
    }
}

/// What Apple's tools say is wrong with a library and its module, if anything.
fn check(validate: &Path, opt: &Path, at: &Path, name: &str, lib: &gpu_backend::air::Library) -> Option<String> {
    let metallib = at.join(format!("{name}.metallib"));
    let air = at.join(format!("{name}.air"));
    std::fs::write(&metallib, &lib.metallib).ok()?;
    std::fs::write(&air, &lib.air).ok()?;
    // air-validate exits 0 even on a file it can't read, so anything it prints is a failure.
    let v = Command::new(validate).arg(&metallib).output().ok()?;
    let said = format!("{}{}", String::from_utf8_lossy(&v.stdout), String::from_utf8_lossy(&v.stderr));
    if !v.status.success() || !said.trim().is_empty() {
        return Some(format!("air-validate: {}", said.trim()));
    }
    let d = Command::new(opt).arg("-S").arg(&air).arg("-o").arg("-").output().ok()?;
    if !d.status.success() {
        return Some(format!("air-opt: {}", String::from_utf8_lossy(&d.stderr).trim()));
    }
    None
}

/// A vertex layout for an entry point's inputs: each in a buffer of its own, in the format its type reads.
fn layout(entry: &gpu_ir::Entry) -> gpu_backend::air::VertexInput {
    use gpu_backend::air::{Attribute, Format, VertexBuffer, VertexInput};
    let mut input = VertexInput::default();
    for (i, io) in entry.inputs.iter().enumerate() {
        let gpu_ir::Slot::Location(location) = io.slot else { continue };
        let lanes = match io.ty {
            gpu_ir::Type::Vector(_, n) => n,
            _ => 1,
        };
        let format = match io.ty.scalar() {
            Some(gpu_ir::Scalar::U32) => Format::U32(lanes),
            Some(gpu_ir::Scalar::I32) => Format::I32(lanes),
            _ => Format::F32(lanes),
        };
        input.buffers.push(VertexBuffer { stride: 4 * lanes as u32, per_instance: false });
        input.attributes.push(Attribute { location, buffer: i as u32, offset: 0, format });
    }
    input
}

#[test]
fn apples_tools_accept_every_program() {
    let (Some(validate), Some(opt)) = (tool("air-validate"), tool("air-opt")) else {
        eprintln!("no Metal toolchain here (air-validate, air-opt): skipped");
        return;
    };
    let at = dir();
    std::fs::create_dir_all(&at).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (program, spirv) in suite::PROGRAMS.iter().chain(suite::SHADERS).copied() {
        let module = gpu_frontend::spirv::read(spirv).unwrap_or_else(|e| panic!("{program}: {e}"));
        for entry in &module.entries {
            let name = format!("{program}-{}", entry.name);
            match gpu_backend::air::write_with(entry, Some(&layout(entry))) {
                Ok(lib) => {
                    if let Some(why) = check(&validate, &opt, &at, &name, &lib) {
                        failures.push(format!("{name}: {why}"));
                    }
                    checked += 1;
                }
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
    }
    if std::env::var_os("GPU_AIR_DUMP").is_none() {
        let _ = std::fs::remove_dir_all(&at);
    }
    assert!(checked > 0);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
