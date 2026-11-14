//! The system layer: files lambda lends every image, because the image's own software looks for them as files though
//! they're the OS's. Today that's lambda's Vulkan driver, on a machine with a GPU: the driver for each ISA
//! (`/usr/lib/<triple>/libvulkan_carbon.so`, built by `build.rs` from `gpu/guest/vulkan`) and the manifest the
//! image's Vulkan loader finds it by (`/usr/share/vulkan/icd.d/carbon_icd.json`, naming the driver for the ISA of the
//! process reading it). NVIDIA's container toolkit puts its driver into containers the same way.
//!
//! They're made in memory and never on the disk: never logged in the machine's layer, frozen, or copied out, and
//! sealed, so no program can change them. A directory the image has too lists both its files and these; one the image
//! hasn't (`/usr/share/vulkan` before Mesa's drivers are installed) is made.

use std::borrow::Cow;
use std::sync::Arc;

use exec::Isa;
use kernels_core::lock;
use layers::New;

use crate::abi::*;
use crate::mm::shared::{self, SharedMem};
use crate::process::Task;
use crate::Meta;

/// The driver, for each ISA a Linux program can be: empty where `build.rs` couldn't build it.
static DRIVER_AARCH64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libvulkan_carbon-aarch64.so"));
static DRIVER_X86_64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libvulkan_carbon-x86_64.so"));

const DRIVER: &str = "libvulkan_carbon.so";
const MANIFEST: &str = "/usr/share/vulkan/icd.d/carbon_icd.json";

/// What's at a path of the system layer.
pub(crate) enum Entry {
    File(Arc<SharedMem>),
    /// A directory: the system layer's names in it, each with its `DT_*` type.
    Dir(Vec<(String, u8)>),
}

fn triple(isa: Isa) -> &'static str {
    match isa {
        Isa::Arm64 => "aarch64-linux-gnu",
        _ => "x86_64-linux-gnu",
    }
}

fn driver_path(isa: Isa) -> String {
    format!("/usr/lib/{}/{DRIVER}", triple(isa))
}

/// The layer's files, for a process of `isa`: none on a machine without a GPU, or where the driver wasn't built.
fn files(task: &Task) -> Vec<(String, Cow<'static, [u8]>)> {
    if task.k.options.gpu.is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (isa, bytes) in [(Isa::Arm64, DRIVER_AARCH64), (Isa::X86_64, DRIVER_X86_64)] {
        if !bytes.is_empty() {
            out.push((driver_path(isa), Cow::Borrowed(bytes)));
        }
    }
    let own = if task.isa == Isa::Arm64 { DRIVER_AARCH64 } else { DRIVER_X86_64 };
    if !own.is_empty() {
        let manifest = format!("{{\n    \"file_format_version\": \"1.0.1\",\n    \"ICD\": {{\n        \"library_path\": \"{}\",\n        \"api_version\": \"1.0.0\"\n    }}\n}}\n", driver_path(task.isa));
        out.push((MANIFEST.to_string(), Cow::Owned(manifest.into_bytes())));
    }
    out
}

/// Whether a path might be in the layer, before anything's made to look.
fn maybe(path: &str) -> bool {
    path == "/usr" || path.starts_with("/usr/")
}

/// What `path` (absolute, with no links in it) is in the layer, if anything.
pub(crate) fn lookup(task: &Task, path: &str) -> Option<Entry> {
    if !maybe(path) || task.k.options.gpu.is_none() {
        return None;
    }
    let path = path.trim_end_matches('/');
    let files = files(task);
    if let Some((_, bytes)) = files.iter().find(|(p, _)| p == path) {
        return Some(Entry::File(sealed(task, path, bytes)));
    }
    let mut names: Vec<(String, u8)> = Vec::new();
    for (file, _) in &files {
        let Some(rest) = file.strip_prefix(path).and_then(|r| r.strip_prefix('/')) else { continue };
        let (name, kind) = match rest.split_once('/') {
            Some((dir, _)) => (dir, DT_DIR),
            None => (rest, DT_REG),
        };
        if !names.iter().any(|(n, _)| n == name) {
            names.push((name.to_string(), kind));
        }
    }
    (!names.is_empty()).then_some(Entry::Dir(names))
}

/// The memory a file of the layer is read and mapped from: made once a machine (for each ISA's manifest too), filled,
/// and sealed.
fn sealed(task: &Task, path: &str, bytes: &[u8]) -> Arc<SharedMem> {
    let key = format!("{path}\0{}", triple(task.isa));
    let mut made = lock(&task.k.system);
    if let Some(mem) = made.get(&key) {
        return mem.clone();
    }
    let mem = SharedMem::new(task.k.exec.clone(), 0, 0);
    let _ = mem.write(0, bytes);
    let _ = mem.add_seals(shared::F_SEAL_SEAL | shared::F_SEAL_SHRINK | shared::F_SEAL_GROW | shared::F_SEAL_WRITE);
    made.insert(key, mem.clone());
    mem
}

/// Whether the machine's disk has `path`, which then wins over the layer for a directory.
fn on_disk(task: &Task, path: &str) -> bool {
    lock(&task.k.disk).resolve(layers::ROOT, path, true).is_ok()
}

/// A layer file opened, or a directory only the layer has.
pub(crate) fn open(task: &Task, path: &str) -> Option<Entry> {
    match lookup(task, path)? {
        Entry::Dir(_) if on_disk(task, path) => None,
        entry => Some(entry),
    }
}

/// `stat` of a layer file, or of a directory only the layer has.
pub(crate) fn stat(task: &Task, path: &str) -> Option<Stat> {
    match open(task, path)? {
        Entry::File(mem) => Some(Stat { dev: 0x5, ino: mem.ino, mode: S_IFREG | 0o755, nlink: 1, size: mem.size() as i64, blksize: 4096, blocks: mem.size().div_ceil(512) as i64, ..Stat::default() }),
        Entry::Dir(names) => {
            let (dev, ino) = super::made_identity(path);
            let dirs = names.iter().filter(|(_, kind)| *kind == DT_DIR).count();
            Some(Stat { dev, ino, mode: S_IFDIR | 0o755, nlink: 2 + dirs as u32, blksize: 4096, ..Stat::default() })
        }
    }
}

/// The layer's names in a directory the disk has too, for `getdents` to list after the disk's.
pub(crate) fn listing(task: &Task, path: &str) -> Vec<(String, u8)> {
    match lookup(task, path) {
        Some(Entry::Dir(names)) => names,
        _ => Vec::new(),
    }
}

/// Puts the directory `path` is made in onto the disk, if only the layer has it, as overlayfs copies a lower directory
/// up: so a program can make files beside the layer's (dpkg unpacking Mesa's drivers into `/usr/share/vulkan`).
pub(crate) fn copy_up(task: &Task, path: &str) {
    let Some((parent, _)) = path.trim_end_matches('/').rsplit_once('/') else { return };
    if parent.is_empty() || !matches!(open(task, parent), Some(Entry::Dir(_))) {
        return;
    }
    let _ = lock(&task.k.disk).put(parent, New::Dir, &[], Meta::dir(0o755), Meta::dir(0o755));
}
