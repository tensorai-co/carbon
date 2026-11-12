//! Linux: memory from `/proc/meminfo`, and certificates checked against the system's CA bundle.

pub(crate) mod trust;

pub use crate::unix::{random, set_raw, terminal_size, thread_cpu};
pub use trust::check as check_certificates;

/// `MemAvailable`: what the kernel says it could give without swapping.
pub fn memory_available() -> u64 {
    meminfo("MemAvailable:").or_else(|| meminfo("MemTotal:")).unwrap_or(0)
}

/// A line of `/proc/meminfo`, in bytes.
fn meminfo(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|line| line.starts_with(key))?;
    let kib: u64 = line[key.len()..].trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kib * 1024)
}
