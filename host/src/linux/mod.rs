//! Linux: memory from `/proc/meminfo`, and certificates checked against the system's CA bundle.

pub(crate) mod trust;

pub use crate::unix::{disk_space, random, set_raw, terminal_size, thread_cpu};
pub use trust::check as check_certificates;

/// `MemAvailable`: what the kernel says it could give without swapping.
pub fn memory_available() -> u64 {
    meminfo("MemAvailable:").or_else(|| meminfo("MemTotal:")).unwrap_or(0)
}

/// The most memory this process may use: its cgroup's limit (a container's, as on Cloud Run or Kubernetes), or the
/// host's RAM, whichever is less.
pub fn memory_limit() -> Option<u64> {
    let total = meminfo("MemTotal:");
    let cgroup = cgroup_limit();
    match (total, cgroup) {
        (Some(total), Some(cgroup)) => Some(total.min(cgroup)),
        (total, cgroup) => total.or(cgroup),
    }
}

/// The memory limit of this process's cgroup, v2's `memory.max` or v1's `memory.limit_in_bytes`, if it has one.
fn cgroup_limit() -> Option<u64> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok().unwrap_or_default();
    let mut candidates = Vec::new();
    for line in own.lines() {
        // v2: "0::/path"; v1: "N:memory:/path" (or with other controllers alongside).
        let mut parts = line.splitn(3, ':');
        let (_, controllers, path) = (parts.next(), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if controllers.is_empty() {
            candidates.push(format!("/sys/fs/cgroup{path}/memory.max"));
        } else if controllers.split(',').any(|c| c == "memory") {
            candidates.push(format!("/sys/fs/cgroup/memory{path}/memory.limit_in_bytes"));
        }
    }
    // Inside a container the cgroup's own directory is usually the root of what's mounted.
    candidates.push("/sys/fs/cgroup/memory.max".to_string());
    candidates.push("/sys/fs/cgroup/memory/memory.limit_in_bytes".to_string());
    candidates.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let limit: u64 = text.trim().parse().ok()?;
        // v1 says "no limit" with a huge number.
        (limit < 1 << 60).then_some(limit)
    })
}

/// A line of `/proc/meminfo`, in bytes.
fn meminfo(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with(key))?;
    let kib: u64 = line[key.len()..].trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kib * 1024)
}

/// The host's fonts: not listed here yet, so programs get none (the to-do list's fonts on Linux and Windows hosts).
pub fn fonts() -> Vec<crate::FontFace> {
    Vec::new()
}

pub fn open_font(_id: &str) -> Option<std::sync::Arc<dyn crate::HostFont>> {
    None
}

pub fn open_font_bytes(_bytes: &[u8]) -> Option<std::sync::Arc<dyn crate::HostFont>> {
    None
}

/// A file's extended attribute, if it has it (not following a link).
pub fn xattr_get(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0u8; 256];
    // SAFETY: both strings are NUL-terminated, and the buffer is as long as said.
    let n = unsafe { libc::lgetxattr(c_path.as_ptr(), c_name.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    (n >= 0).then(|| {
        buf.truncate(n as usize);
        buf
    })
}

/// Sets a file's extended attribute (not following a link).
pub fn xattr_set(path: &std::path::Path, name: &str, value: &[u8]) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let c_name = std::ffi::CString::new(name)?;
    // SAFETY: both strings are NUL-terminated, and the value is as long as said.
    let r = unsafe { libc::lsetxattr(c_path.as_ptr(), c_name.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}
