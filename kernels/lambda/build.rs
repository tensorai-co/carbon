//! Builds lambda's GPU drivers for each ISA a Linux program can be, for the system layer (`os::system`) to embed: the
//! Vulkan driver (`gpu/guest/vulkan`, `libvulkan_carbon.so`) and the GL vendor library (`gpu/guest/gl`,
//! `libGLX_carbon.so.0`), each a shared library for `aarch64-` and `x86_64-unknown-linux-gnu`, linked with LLVM's
//! `ld.lld`. They're `no_std` Rust with `alloc` over the C library's `malloc`; the C library's functions are left for
//! the image's dynamic loader to bind, since the drivers are loaded into programs that have it already.
//!
//! `core`, `alloc` and `compiler_builtins` are built from source (`-Zbuild-std`, under `RUSTC_BOOTSTRAP`), as atlas's
//! guest is. Where one can't be built, an empty file is embedded and the system layer leaves it out, saying nothing to
//! the machine: its programs see no Carbon GPU, or no Carbon GL.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Each driver: its folder in `gpu/guest`, its crate's library, the soname it's loaded by, and the file it's
/// embedded from (with the ISA after it).
const DRIVERS: [(&str, &str, &str, &str); 2] = [
    ("vulkan", "libvulkan_carbon.so", "libvulkan_carbon.so", "libvulkan_carbon"),
    ("gl", "libglx_carbon.so", "libGLX_carbon.so.0", "libGLX_carbon"),
];

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let gpu = manifest.join("../../gpu");
    println!("cargo:rerun-if-changed={}", gpu.join("wire/src").display());
    println!("cargo:rerun-if-env-changed=LAMBDA_GUEST_LINKER");
    let linker = std::env::var("LAMBDA_GUEST_LINKER").ok().or_else(find_ld_lld);
    for (folder, library, soname, file) in DRIVERS {
        let guest = gpu.join("guest").join(folder);
        for watched in [guest.join("src"), guest.join("Cargo.toml")] {
            println!("cargo:rerun-if-changed={}", watched.display());
        }
        for arch in ["aarch64", "x86_64"] {
            let dest = out.join(format!("{file}-{arch}.so"));
            let built = linker.as_deref().ok_or("no ld.lld found (set LAMBDA_GUEST_LINKER)".to_string()).and_then(|linker| build(&guest, &out, arch, linker, library, soname));
            match built {
                Ok(built) => {
                    std::fs::copy(&built, &dest).expect("copying a GPU driver");
                }
                Err(why) => {
                    println!("cargo:warning=lambda's {folder} driver isn't built for {arch}: {why}");
                    std::fs::write(&dest, []).expect("writing an empty GPU driver");
                }
            }
        }
    }
}

fn build(guest: &Path, out: &Path, arch: &str, linker: &str, library: &str, soname: &str) -> Result<PathBuf, String> {
    let target = format!("{arch}-unknown-linux-gnu");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = out.join("guest-target");
    let output = Command::new(&cargo)
        .current_dir(guest)
        .args(["build", "--release", "--quiet", "--target", &target, "--target-dir"])
        .arg(&target_dir)
        .env(format!("CARGO_TARGET_{}_LINKER", target.to_uppercase().replace('-', "_")), linker)
        .env("RUSTFLAGS", format!("-C linker-flavor=ld -C link-arg=-soname={soname}"))
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
    Ok(target_dir.join(&target).join("release").join(library))
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
