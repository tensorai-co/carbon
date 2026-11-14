//! Builds lambda's Vulkan driver (`gpu/guest/vulkan`) for each ISA a Linux program can be, for the system layer
//! (`os::system`) to embed: a shared library for `aarch64-` and `x86_64-unknown-linux-gnu`, linked with LLVM's
//! `ld.lld`. It's `no_std` Rust with `alloc` over the C library's `malloc`; the C library's functions are left for
//! the image's dynamic loader to bind, since the driver is loaded into programs that have it already.
//!
//! `core`, `alloc` and `compiler_builtins` are built from source (`-Zbuild-std`, under `RUSTC_BOOTSTRAP`), as atlas's
//! guest is. Where it can't be built, an empty file is embedded and the system layer leaves the driver out, saying
//! nothing to the machine: its programs see no Carbon GPU.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let gpu = manifest.join("../../gpu");
    let guest = gpu.join("guest/vulkan");
    for watched in [guest.join("src"), guest.join("Cargo.toml"), gpu.join("wire/src")] {
        println!("cargo:rerun-if-changed={}", watched.display());
    }
    println!("cargo:rerun-if-env-changed=LAMBDA_GUEST_LINKER");
    let linker = std::env::var("LAMBDA_GUEST_LINKER").ok().or_else(find_ld_lld);
    for arch in ["aarch64", "x86_64"] {
        let dest = out.join(format!("libvulkan_carbon-{arch}.so"));
        let built = linker.as_deref().ok_or("no ld.lld found (set LAMBDA_GUEST_LINKER)".to_string()).and_then(|linker| build(&guest, &out, arch, linker));
        match built {
            Ok(library) => {
                std::fs::copy(&library, &dest).expect("copying lambda's Vulkan driver");
            }
            Err(why) => {
                println!("cargo:warning=lambda's Vulkan driver isn't built for {arch}: {why}");
                std::fs::write(&dest, []).expect("writing an empty Vulkan driver");
            }
        }
    }
}

fn build(guest: &Path, out: &Path, arch: &str, linker: &str) -> Result<PathBuf, String> {
    let target = format!("{arch}-unknown-linux-gnu");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = out.join("guest-target");
    let output = Command::new(&cargo)
        .current_dir(guest)
        .args(["build", "--release", "--quiet", "--target", &target, "--target-dir"])
        .arg(&target_dir)
        .env(format!("CARGO_TARGET_{}_LINKER", target.to_uppercase().replace('-', "_")), linker)
        .env("RUSTFLAGS", "-C linker-flavor=ld -C link-arg=-soname=libvulkan_carbon.so")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["-Zbuild-std=core,alloc,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .output()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.lines().find(|l| l.contains("error")).unwrap_or("cargo failed").to_string());
    }
    Ok(target_dir.join(&target).join("release/libvulkan_carbon.so"))
}

fn find_ld_lld() -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into(), "/opt/homebrew/opt/llvm/bin".into(), "/opt/homebrew/opt/lld/bin".into()])
        .chain([r"C:\Program Files\LLVM\bin".into()])
        .map(|dir| dir.join(format!("ld.lld{}", std::env::consts::EXE_SUFFIX)))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}
