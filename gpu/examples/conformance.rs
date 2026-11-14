//! gpu's conformance suite on every device this host has: the CPU device always, the host's GPU if there is one.
//!
//! ```sh
//! cargo run -p gpu --example gpu-conformance
//! ```

mod suite;

fn main() {
    let mut failed = false;
    let mut devices = Vec::new();
    for choice in [gpu::Choice::Software, gpu::Choice::Host] {
        match gpu::Device::open(choice) {
            Ok(Some(d)) => devices.push(d),
            Ok(None) => {}
            Err(e) => println!("{choice:?}: none ({e})"),
        }
    }
    for device in &mut devices {
        println!("{}", device.info().describe());
        for outcome in suite::run(device) {
            match outcome.result {
                Ok(suite::Ran::Passed) => println!("  ok    {}", outcome.name),
                Ok(suite::Ran::Unsupported(why)) => println!("  --    {}: not on this device yet ({why})", outcome.name),
                Err(why) => {
                    failed = true;
                    println!("  FAIL  {}: {why}", outcome.name);
                }
            }
        }
    }
    std::process::exit(if failed { 1 } else { 0 });
}
