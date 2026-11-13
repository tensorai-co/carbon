//! Runs a Windows program from the host's disk on atlas, in this host's `vm` backend. Its machine's disk has the
//! program at `C:\<name>`, with any DLLs and other programs from the same folder beside it. Set `CARBON_TRACE=1` to see every host
//! export, and `CARBON_ISOLATION=sandbox` (or `auto`) to use the sandbox. With `CARBON_SCREENSHOTS=<folder>`, the
//! desktop as the machine stopped is saved there as `screen.png`, and each top-level window's last picture beside it.
//!
//! ```sh
//! cargo build --example run-windows
//! codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run-windows
//! target/debug/examples/run-windows path/to/program.exe [args...]
//! ```

use std::sync::Arc;

use exec::{Exec, Isolation, Options};

fn main() {
    exec::run_if_stub();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(program) = args.first() else {
        eprintln!("usage: run-windows <program.exe> [args...]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(program).unwrap_or_else(|e| {
        eprintln!("reading {program}: {e}");
        std::process::exit(2)
    });
    let name = std::path::Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or("program.exe").to_string();
    // x86 programs on Apple silicon, 64-bit or 32-bit, want x86's memory ordering.
    let tso = bytes.windows(6).take(512).any(|w| w == b"PE\0\0\x64\x86" || w == b"PE\0\0\x4c\x01") && cfg!(target_arch = "aarch64");

    let mut disk = atlas::new_disk();
    disk.put(&format!("/{name}"), layers::New::File, &bytes, atlas::Meta::default(), atlas::Meta { attributes: 0x10 }).expect("putting the program on the disk");
    let folder = std::path::Path::new(program).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let lower = file.to_ascii_lowercase();
        if (lower.ends_with(".dll") || lower.ends_with(".exe") && file != name)
            && let Ok(dll) = std::fs::read(entry.path())
        {
            disk.put(&format!("/{file}"), layers::New::File, &dll, atlas::Meta::default(), atlas::Meta { attributes: 0x10 }).expect("putting a file beside the program on the disk");
        }
    }

    let isolation = match std::env::var("CARBON_ISOLATION").as_deref() {
        Ok("sandbox") => Isolation::Sandbox,
        Ok("auto") => Isolation::Auto,
        _ => Isolation::Vm,
    };
    let (exec, cpu) = Exec::open(Options { memory: 512 << 20, isolation, tso, tuning: tuning() }).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    eprintln!("[atlas on {}]", exec.backend());
    let path = format!("C:\\{name}");
    let mut argv: Vec<&str> = vec![&path];
    argv.extend(args.iter().skip(1).map(String::as_str));
    let options = atlas::Options { trace: std::env::var_os("CARBON_TRACE").is_some(), ..Default::default() };
    match atlas::boot(exec, cpu, disk, Arc::new(host::Std::new()), &argv, options) {
        Ok(ended) => {
            if let Ok(folder) = std::env::var("CARBON_SCREENSHOTS") {
                let screen = ("screen".to_string(), &ended.screen);
                let windows = ended.windows.iter().enumerate().map(|(i, (title, surface))| {
                    let name: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
                    (format!("{i}-{name}"), surface)
                });
                for (name, surface) in std::iter::once(screen).chain(windows) {
                    let path = format!("{folder}/{name}.png");
                    match std::fs::write(&path, surface.png()) {
                        Ok(()) => eprintln!("[saved {path}, {}x{}]", surface.width, surface.height),
                        Err(e) => eprintln!("[couldn't save {path}: {e}]"),
                    }
                }
            }
            eprintln!("[exit code {}]", ended.code);
            std::process::exit(ended.code as i32);
        }
        Err(why) => {
            eprintln!("atlas: {why}");
            std::process::exit(2);
        }
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
