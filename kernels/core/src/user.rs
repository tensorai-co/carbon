//! Copying in and out of a program's memory. Anything a program points at may be unmapped or not allowed: a bad
//! pointer is a [`Fault`], which a kernel turns into `EFAULT` or `STATUS_ACCESS_VIOLATION`, never a crash.
//!
//! exec counts a page that's shared copy-on-write, or not yet faulted in, as unmapped too. A [`Fault`] says which
//! page, so the kernel can fill it the way it would for the program's own touch, and copy again.

use exec::{PAGE, Prot, Space, Turn};

use crate::thread::Cx;

/// A program gave an address it can't use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault(pub u64);

pub type Result<T> = std::result::Result<T, Fault>;

/// The most a string read stops at, if it has no end before.
pub const MOST_STRING: usize = 1 << 16;

pub fn read(space: &Space<'_>, addr: u64, buffer: &mut [u8]) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    space.read(addr, buffer).map_err(|e| at(e, addr))
}

/// Where a copy failed: the page exec couldn't reach, or else where it started.
fn at(e: exec::Error, addr: u64) -> Fault {
    match e {
        exec::Error::Unmapped(page) => Fault(page),
        _ => Fault(addr),
    }
}

pub fn read_vec(space: &Space<'_>, addr: u64, len: usize) -> Result<Vec<u8>> {
    let mut buffer = vec![0; len];
    read(space, addr, &mut buffer)?;
    Ok(buffer)
}

pub fn write(space: &mut Space<'_>, addr: u64, data: &[u8]) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    space.write(addr, data).map_err(|e| at(e, addr))
}

pub fn read_u32(space: &Space<'_>, addr: u64) -> Result<u32> {
    let mut b = [0; 4];
    read(space, addr, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

pub fn read_u64(space: &Space<'_>, addr: u64) -> Result<u64> {
    let mut b = [0; 8];
    read(space, addr, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

/// A pointer-sized value: 4 bytes for a 32-bit program, 8 otherwise.
pub fn read_ptr(space: &Space<'_>, addr: u64, width: u64) -> Result<u64> {
    if width == 4 { read_u32(space, addr).map(u64::from) } else { read_u64(space, addr) }
}

pub fn write_u32(space: &mut Space<'_>, addr: u64, value: u32) -> Result<()> {
    write(space, addr, &value.to_le_bytes())
}

pub fn write_u64(space: &mut Space<'_>, addr: u64, value: u64) -> Result<()> {
    write(space, addr, &value.to_le_bytes())
}

/// A NUL-terminated string, without its NUL, read a page at a time so it never reads past its end into what's
/// unmapped.
pub fn read_cstr(space: &Space<'_>, addr: u64, most: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = addr;
    while out.len() < most {
        let chunk = (exec::PAGE - at % exec::PAGE) as usize;
        let mut buffer = vec![0; chunk.min(most - out.len())];
        read(space, at, &mut buffer)?;
        if let Some(end) = buffer.iter().position(|&b| b == 0) {
            out.extend_from_slice(&buffer[..end]);
            return Ok(out);
        }
        out.extend_from_slice(&buffer);
        at += buffer.len() as u64;
    }
    Err(Fault(addr))
}

/// A NUL-terminated UTF-16 string, as Windows passes them.
pub fn read_wstr(space: &Space<'_>, addr: u64, most: usize) -> Result<String> {
    let mut units = Vec::new();
    let mut at = addr;
    while units.len() < most {
        let mut b = [0; 2];
        read(space, at, &mut b)?;
        let unit = u16::from_le_bytes(b);
        if unit == 0 {
            return Ok(String::from_utf16_lossy(&units));
        }
        units.push(unit);
        at += 2;
    }
    Err(Fault(addr))
}

/// A list of pointers ending in a null one, such as `argv`.
pub fn read_ptr_list(space: &Space<'_>, addr: u64, width: u64, most: usize) -> Result<Vec<u64>> {
    let mut out = Vec::new();
    if addr == 0 {
        return Ok(out);
    }
    loop {
        let ptr = read_ptr(space, addr + out.len() as u64 * width, width)?;
        if ptr == 0 {
            return Ok(out);
        }
        if out.len() == most {
            return Err(Fault(addr));
        }
        out.push(ptr);
    }
}

/// Runs `copy` in a turn. Where it faults on a page, asks `fill` to fill that page as the program's own touch would (a
/// page shared copy-on-write, or one not yet faulted in), and copies again. Err if `fill` can't.
pub fn with_faults<R>(
    cx: &mut Cx<'_>,
    write: bool,
    mut fill: impl FnMut(&mut Cx<'_>, u64, Prot) -> bool,
    mut copy: impl FnMut(&mut Turn<'_>) -> Result<R>,
) -> Result<R> {
    let access = if write { Prot::W } else { Prot::R };
    let mut last = None;
    loop {
        match cx.turn(&mut copy) {
            Ok(result) => return Ok(result),
            Err(Fault(addr)) => {
                let page = addr & !(PAGE - 1);
                if last == Some(page) || !fill(cx, addr, access) {
                    return Err(Fault(addr));
                }
                last = Some(page);
            }
        }
    }
}
