//! macOS: memory from the Mach VM statistics, certificates by the system's own trust settings, and fonts from Core
//! Text.

mod fonts;
mod trust;

pub use crate::unix::{random, set_raw, terminal_size, thread_cpu};
pub use fonts::{fonts, open_font};
pub use trust::check as check_certificates;

unsafe extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
}

/// Free, inactive and purgeable pages: what the system could hand out without paging.
pub fn memory_available() -> u64 {
    // SAFETY: an all-zero block of numbers is one of these.
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let mut page = 0u64;
    let mut size = size_of::<u64>();
    // SAFETY: each is given room for what it's said to fill in.
    let (result, _) = unsafe {
        (
            libc::host_statistics64(mach_host_self(), libc::HOST_VM_INFO64, (&raw mut stats).cast(), &raw mut count),
            libc::sysctlbyname(c"hw.pagesize".as_ptr(), (&raw mut page).cast(), &raw mut size, std::ptr::null_mut(), 0),
        )
    };
    if result != 0 || page == 0 {
        return installed_memory();
    }
    (u64::from(stats.free_count) + u64::from(stats.inactive_count) + u64::from(stats.purgeable_count)) * page
}

/// The most memory this process may use: the Mac's RAM.
pub fn memory_limit() -> Option<u64> {
    Some(installed_memory()).filter(|&bytes| bytes > 0)
}

fn installed_memory() -> u64 {
    let mut bytes = 0u64;
    let mut size = size_of::<u64>();
    // SAFETY: room for a u64, as `hw.memsize` is.
    unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &raw mut size, std::ptr::null_mut(), 0) };
    bytes
}

/// A file's extended attribute, if it has it (not following a link).
pub fn xattr_get(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0u8; 256];
    // SAFETY: both strings are NUL-terminated, and the buffer is as long as said.
    let n = unsafe { libc::getxattr(c_path.as_ptr(), c_name.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, libc::XATTR_NOFOLLOW) };
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
    let r = unsafe { libc::setxattr(c_path.as_ptr(), c_name.as_ptr(), value.as_ptr().cast(), value.len(), 0, libc::XATTR_NOFOLLOW) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}
