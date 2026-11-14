//! The conformance suite in `cargo test`: on the CPU device, and on the host's GPU where there is one.

#[path = "../examples/suite.rs"]
mod suite;

fn check(choice: gpu::Choice) {
    let Ok(Some(mut device)) = gpu::Device::open(choice) else {
        eprintln!("{choice:?}: no device here, skipped");
        return;
    };
    let failures: Vec<String> = suite::run(&mut device)
        .into_iter()
        .filter_map(|o| o.result.err().map(|why| format!("{}: {why}", o.name)))
        .collect();
    assert!(failures.is_empty(), "{} failed:\n{}", device.info().describe(), failures.join("\n"));
}

#[test]
fn software() {
    check(gpu::Choice::Software);
}

#[test]
fn host() {
    check(gpu::Choice::Host);
}
