//! Builds `atlas-guest` (ntdll) for each ISA a Windows program can use, for `images::guest` to embed: ARM64, x86-64,
//! and x86 for WoW64. It's `no_std` Rust for the `*-pc-windows-msvc` targets, linked with LLVM's `lld-link`.
//!
//! `core` and `compiler_builtins` are always built from source (`-Zbuild-std`, under `RUSTC_BOOTSTRAP`), with
//! `compiler-builtins-mem`: ntdll has no C runtime under it, so `memcpy`, `memset`, `memcmp` and `strlen`, which the
//! compiler and `core` call, come from there. rustup's prebuilt `core` for these targets has no such functions. With
//! rustup, that needs `rustup component add rust-src`; Homebrew's Rust has the source already. Where it can't be built,
//! an empty image is embedded and guest exports say they need it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let guest = manifest.join("guest");
    println!("cargo:rerun-if-changed={}", guest.join("src").display());
    println!("cargo:rerun-if-changed={}", guest.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", manifest.join("abi/src").display());
    println!("cargo:rerun-if-env-changed=ATLAS_GUEST_LINKER");
    let linker = std::env::var("ATLAS_GUEST_LINKER").ok().or_else(find_lld_link);
    for arch in ["aarch64", "x86_64", "i686"] {
        let dest = out.join(format!("ntdll-{arch}.dll"));
        let built = linker.as_deref().ok_or("no lld-link found (set ATLAS_GUEST_LINKER)".to_string()).and_then(|linker| build(&guest, &out, arch, linker));
        match built {
            Ok(dll) => {
                std::fs::copy(&dll, &dest).expect("copying atlas-guest");
            }
            Err(why) => {
                println!("cargo:warning=atlas-guest isn't built for {arch}: {why}");
                std::fs::write(&dest, []).expect("writing an empty atlas-guest");
            }
        }
    }
}

fn build(guest: &Path, out: &Path, arch: &str, linker: &str) -> Result<PathBuf, String> {
    let target = format!("{arch}-pc-windows-msvc");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = out.join("guest-target");
    let mut command = Command::new(&cargo);
    command
        .current_dir(guest)
        .args(["build", "--release", "--quiet", "--target", &target, "--target-dir"])
        .arg(&target_dir)
        .env(format!("CARGO_TARGET_{}_LINKER", target.to_uppercase().replace('-', "_")), linker)
        // Exports written in assembly, which rustc doesn't export by itself.
        .env("RUSTFLAGS", rustflags(arch))
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER");
    command.env("RUSTC_BOOTSTRAP", "1").args(["-Zbuild-std=core,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"]);
    let output = command.output().map_err(|e| format!("running cargo: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.lines().find(|l| l.contains("error")).unwrap_or("cargo failed").to_string());
    }
    Ok(target_dir.join(&target).join("release/atlas_guest.dll"))
}

/// What the link needs: no entry point or C runtime, and the exports written in assembly (on x86, raising and unwinding
/// too, which capture their caller's registers).
fn rustflags(arch: &str) -> String {
    let mut exports = vec!["RtlCaptureContext", "AtlasSwitchToFiber"];
    if arch == "i686" {
        exports.extend(["RtlRaiseException", "RtlUnwind"]);
    }
    let mut flags = String::from("-C link-arg=/NOENTRY -C link-arg=/NODEFAULTLIB");
    for e in exports {
        flags.push_str(&format!(" -C link-arg=/EXPORT:{e}"));
    }
    flags
}

fn find_lld_link() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into(), "/opt/homebrew/opt/llvm/bin".into(), "/opt/homebrew/opt/lld/bin".into()])
        .chain([r"C:\Program Files\LLVM\bin".into()])
        // `lld-link.exe` where the build runs on Windows.
        .map(|dir| dir.join(format!("lld-link{}", std::env::consts::EXE_SUFFIX)))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}
