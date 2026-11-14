//! The device checks every argument before a backend sees it.

use gpu::{Choice, Command, Device, Range, Submit};

const COPY: &[u8] = include_bytes!("../examples/programs/copy.spv");

fn device() -> Device {
    Device::open(Choice::Software).unwrap().unwrap()
}

#[test]
fn none_is_no_device() {
    assert!(Device::open(Choice::None).unwrap().is_none());
}

#[test]
fn auto_says_which() {
    let d = Device::open(Choice::Auto).unwrap().unwrap();
    assert!(!d.info().describe().is_empty());
}

#[test]
fn memory_is_bounded() {
    let mut d = device();
    assert!(d.alloc(0).is_err());
    let m = d.alloc(16).unwrap();
    assert!(d.write(m, 8, &[0; 8]).is_ok());
    assert!(d.write(m, 9, &[0; 8]).is_err());
    let mut out = [0u8; 4];
    assert!(d.read(m, u64::MAX - 1, &mut out).is_err());
    d.free(m);
    assert!(d.write(m, 0, &[0; 4]).is_err());
}

#[test]
fn dispatches_are_checked() {
    let mut d = device();
    let p = d.pipeline_spirv(COPY, "main").unwrap();
    assert!(d.pipeline_spirv(COPY, "other").is_err());
    let m = d.alloc(1024).unwrap();
    let whole = Range { memory: m, offset: 0, size: 1024 };
    let past = Range { memory: m, offset: 1000, size: 100 };
    let submit = |d: &mut Device, c: Command| d.submit(Submit { commands: &[c], wait: &[], signal: &[] });
    assert!(submit(&mut d, Command::Dispatch { pipeline: p, bindings: vec![whole], groups: [1, 1, 1] }).is_err());
    assert!(submit(&mut d, Command::Dispatch { pipeline: p, bindings: vec![whole, past], groups: [1, 1, 1] }).is_err());
    assert!(submit(&mut d, Command::Dispatch { pipeline: p, bindings: vec![whole, whole], groups: [0, 1, 1] }).is_err());
    assert!(submit(&mut d, Command::Fill { range: Range { memory: m, offset: 2, size: 4 }, value: 0 }).is_err());
    assert!(submit(&mut d, Command::Copy { from: whole, to: m, at: 1 }).is_err());
}

#[test]
fn a_program_past_its_buffer_faults_on_the_cpu_device() {
    let mut d = device();
    let p = d.pipeline_spirv(COPY, "main").unwrap();
    let small = d.alloc(64).unwrap();
    let r = Range { memory: small, offset: 0, size: 64 };
    let e = d.submit(Submit { commands: &[Command::Dispatch { pipeline: p, bindings: vec![r, r], groups: [1, 1, 1] }], wait: &[], signal: &[] });
    let message = e.unwrap_err().message;
    assert!(message.contains("outside every allocation"), "{message}");
}
