//! Runs a Linux program on lambda, in this host's `vm` backend (or `sandbox`, with `CARBON_ISOLATION=sandbox`).
//!
//! With `--root image.tar`, the machine's disk is that root file system (a tar of an image's files, such as Debian's),
//! and the program is a path inside it. Without it, the program is a file on the host, put at `/bin/<name>` on an
//! otherwise empty disk with a file at `/etc/greeting`. Set `CARBON_TRACE=1` to see every system call.
//!
//! With `CARBON_DISPLAY=WxH[@SCALE]`, the machine has a desktop that big, and its Wayland server; with
//! `CARBON_SCREENSHOTS=<folder>` too, the desktop as the machine stopped is saved there as `screen.png`, and each
//! window's last picture while it was shown beside it.
//!
//! ```sh
//! cargo build --example run
//! codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run
//! target/debug/examples/run path/to/static-binary [args...]
//! target/debug/examples/run --root debian.tar /bin/bash -c 'echo hello'
//! ```

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use display::desktop::{self, WindowId};

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
    let desktop = std::env::var("CARBON_DISPLAY").ok().map(|size| {
        let (w, h, scale) = display::output::parse_size(&size).unwrap_or_else(|| fail(format!("CARBON_DISPLAY is WxH or WxH@SCALE, not {size}")));
        let d = display::Desktop::shared(w, h);
        desktop::lock(&d).set_output(display::output::Output::headless(w, h, scale));
        d
    });
    options.display = desktop.clone();
    let _clock = desktop.clone().map(|d| display::clock::Clock::start(d, 60));
    let done = Arc::new(AtomicBool::new(false));
    let pictures = desktop.clone().map(|d| record(d, done.clone()));
    let ended = lambda::boot(exec, cpu, disk, Arc::new(host::Std::new()), &argv, options);
    done.store(true, Ordering::Release);
    if let (Some(d), Ok(folder)) = (&desktop, std::env::var("CARBON_SCREENSHOTS")) {
        let mut saved = vec![("screen".to_string(), desktop::lock(d).screen().clone())];
        if let Some(pictures) = pictures {
            saved.extend(pictures.lock().unwrap_or_else(|p| p.into_inner()).values().enumerate().map(|(i, (title, s))| {
                let name: String = title.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
                (format!("{i}-{name}"), s.clone())
            }));
        }
        for (name, surface) in saved {
            let path = format!("{folder}/{name}.png");
            match std::fs::write(&path, surface.png()) {
                Ok(()) => eprintln!("[saved {path}, {}x{}]", surface.width, surface.height),
                Err(e) => eprintln!("[couldn't save {path}: {e}]"),
            }
        }
    }
    match ended {
        Ok(status) => {
            eprintln!("[{status:?}]");
            std::process::exit(status.code());
        }
        Err(why) => fail(format!("lambda: {why}")),
    }
}

/// Each window's picture as it was last shown, with its title, kept as the desktop changes until `done`.
fn record(d: desktop::Shared, done: Arc<AtomicBool>) -> Arc<Mutex<BTreeMap<WindowId, (String, display::Surface)>>> {
    let pictures: Arc<Mutex<BTreeMap<WindowId, (String, display::Surface)>>> = Arc::default();
    let kept = pictures.clone();
    let observer = desktop::lock(&d).observe();
    std::thread::spawn(move || {
        while !done.load(Ordering::Acquire) {
            {
                let mut d = desktop::lock(&d);
                let observed = d.take_observed(observer);
                let changed = observed.damage.iter().map(|(id, _)| *id).chain(observed.windows.iter().map(|w| w.id));
                let ids: Vec<WindowId> = changed.collect();
                for id in ids {
                    let Some(info) = d.window_info(id).filter(|w| w.on_screen) else { continue };
                    if let Some(s) = d.read(id, display::Rect::new(0, 0, info.content.width(), info.content.height())) {
                        kept.lock().unwrap_or_else(|p| p.into_inner()).insert(id, (info.title, s));
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    pictures
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
