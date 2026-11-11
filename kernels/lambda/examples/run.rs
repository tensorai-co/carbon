//! Runs a Linux program on lambda, in this host's `vm` backend (or `sandbox`, with `CARBON_ISOLATION=sandbox`).
//!
//! With `--root image.tar`, the machine's disk is that root file system (a tar of an image's files, such as Debian's),
//! and the program is a path inside it. Without it, the program is a file on the host, put at `/bin/<name>` on an
//! otherwise empty disk with a file at `/etc/greeting`. Set `CARBON_TRACE=1` to see every system call.
//!
//! ```sh
//! cargo build --example run
//! codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run
//! target/debug/examples/run path/to/static-binary [args...]
//! target/debug/examples/run --root debian.tar /bin/bash -c 'echo hello'
//! ```

use std::sync::Arc;

use exec::{Exec, Isolation, Options};
use lambda::{Disk, Meta};

fn fail(what: String) -> ! {
    eprintln!("{what}");
    std::process::exit(2)
}

fn main() {
    exec::run_if_stub();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let root = if args.first().map(String::as_str) == Some("--root") {
        if args.len() < 3 {
            fail("usage: run --root <rootfs.tar> <path-in-it> [args...]".into());
        }
        args.remove(0);
        Some(args.remove(0))
    } else {
        None
    };
    let Some(program) = args.first().cloned() else { fail("usage: run [--root <rootfs.tar>] <linux-program> [args...]".into()) };
    let dir = Meta::dir(0o755);
    let mut disk = Disk::new(dir, false);
    let (path, is_x86) = match &root {
        Some(tar) => {
            let archive = std::fs::read(tar).unwrap_or_else(|e| fail(format!("reading {tar}: {e}")));
            let started = std::time::Instant::now();
            let n = layers::tar::import(&mut disk, &archive, Meta::from).unwrap_or_else(|e| fail(format!("{tar}: {e}")));
            eprintln!("[{n} files from {tar} in {:.0?}]", started.elapsed());
            let bytes = disk.resolve(layers::ROOT, &program, true).and_then(|ino| disk.contents(ino)).unwrap_or_default();
            (program.clone(), bytes.get(18) == Some(&62))
        }
        None => {
            let bytes = std::fs::read(&program).unwrap_or_else(|e| fail(format!("reading {program}: {e}")));
            let name = std::path::Path::new(&program).file_name().and_then(|n| n.to_str()).unwrap_or("program").to_string();
            let path = format!("/bin/{name}");
            disk.put(&path, layers::New::File, &bytes, Meta::file(0o755), dir).expect("putting the program on the disk");
            disk.put("/etc/greeting", layers::New::File, b"greetings from the machine's disk\n", Meta::file(0o644), dir).expect("putting /etc/greeting");
            (path, bytes.get(18) == Some(&62))
        }
    };
    let isolation = match std::env::var("CARBON_ISOLATION").as_deref() {
        Ok("sandbox") => Isolation::Sandbox,
        Ok("auto") => Isolation::Auto,
        _ => Isolation::Vm,
    };
    // x86 programs on Apple silicon want x86's memory ordering.
    let tso = is_x86 && cfg!(target_arch = "aarch64");

    let (exec, cpu) = Exec::open(Options { memory: 1 << 30, isolation, tso, tuning: tuning() }).unwrap_or_else(|e| fail(e.to_string()));
    eprintln!("[lambda on {}]", exec.backend());
    let mut argv: Vec<&str> = vec![&path];
    argv.extend(args.iter().skip(1).map(String::as_str));
    let mut options = lambda::Options::default();
    if let Ok(term) = std::env::var("TERM") {
        options.env.push(format!("TERM={term}"));
    }
    match lambda::boot(exec, cpu, disk, Arc::new(host::Std::new()), &argv, options) {
        Ok(status) => {
            eprintln!("[{status:?}]");
            std::process::exit(status.code());
        }
        Err(why) => fail(format!("lambda: {why}")),
    }
}

/// exec's tuning: the defaults, but for `CARBON_EXEC_CODE_CACHE=<bytes>`, which makes x86's code cache move between
/// its halves sooner, for testing.
fn tuning() -> exec::Tuning {
    let mut tuning = exec::Tuning::default();
    if let Some(bytes) = std::env::var("CARBON_EXEC_CODE_CACHE").ok().and_then(|v| v.parse().ok()) {
        tuning.code_cache = bytes;
    }
    tuning
}
