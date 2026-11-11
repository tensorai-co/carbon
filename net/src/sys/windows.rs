//! Windows: `WSAPoll`, the host's IPv4 addresses from `GetIpAddrTable`, and its DNS servers from `GetNetworkParams`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::os::windows::io::AsRawSocket;

use super::{READ, Raw, WRITE};

#[repr(C)]
struct WsaPollFd {
    fd: usize,
    events: i16,
    revents: i16,
}

const POLLRDNORM: i16 = 0x100;
const POLLWRNORM: i16 = 0x10;
const POLLERR: i16 = 0x1;
const POLLHUP: i16 = 0x2;
const POLLNVAL: i16 = 0x4;
const SOL_SOCKET: i32 = 0xffff;
const SO_KEEPALIVE: i32 = 8;

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAPoll(fds: *mut WsaPollFd, count: u32, timeout: i32) -> i32;
    fn setsockopt(socket: usize, level: i32, name: i32, value: *const u8, len: i32) -> i32;
}

#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetIpAddrTable(table: *mut u8, size: *mut u32, order: i32) -> u32;
    fn GetNetworkParams(info: *mut u8, size: *mut u32) -> u32;
}

const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

pub(super) fn raw_tcp(stream: &TcpStream) -> Raw {
    stream.as_raw_socket()
}

pub(super) fn raw_udp(socket: &UdpSocket) -> Raw {
    socket.as_raw_socket()
}

/// Waits until one of `entries` is ready, as `unix::poll` does.
pub(super) fn poll(entries: &[(Raw, u8)]) -> Vec<u8> {
    let mut fds: Vec<WsaPollFd> = entries
        .iter()
        .map(|&(raw, ways)| WsaPollFd {
            fd: raw as usize,
            events: (if ways & READ != 0 { POLLRDNORM } else { 0 }) | (if ways & WRITE != 0 { POLLWRNORM } else { 0 }),
            revents: 0,
        })
        .collect();
    // SAFETY: `fds` is a live array of `fds.len()` WSAPOLLFDs.
    let n = unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, -1) };
    if n < 0 {
        return vec![0; entries.len()];
    }
    fds.iter()
        .map(|fd| {
            let both = fd.revents & (POLLHUP | POLLERR | POLLNVAL) != 0;
            (if fd.revents & POLLRDNORM != 0 || both { READ } else { 0 }) | (if fd.revents & POLLWRNORM != 0 || both { WRITE } else { 0 })
        })
        .collect()
}

pub(super) fn set_keepalive(stream: &TcpStream, on: bool) {
    let value: u32 = on.into();
    // SAFETY: a DWORD option, with its size.
    unsafe {
        setsockopt(stream.as_raw_socket() as usize, SOL_SOCKET, SO_KEEPALIVE, (&raw const value).cast(), 4);
    }
}

/// Calls `get` with a buffer until it's big enough: the buffer, or None.
fn grow(mut get: impl FnMut(*mut u8, *mut u32) -> u32) -> Option<Vec<u64>> {
    let mut size: u32 = 4096;
    for _ in 0..4 {
        // u64s, so what it fills in is aligned for its pointers.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        match get(buffer.as_mut_ptr().cast(), &mut size) {
            0 => return Some(buffer),
            ERROR_BUFFER_OVERFLOW | ERROR_INSUFFICIENT_BUFFER => continue,
            _ => return None,
        }
    }
    None
}

/// The host's IPv4 addresses (enough to choose a subnet none of them is on).
pub(super) fn addresses() -> Vec<IpAddr> {
    // SAFETY: room for `size` bytes, which it says if it needs more.
    let Some(table) = grow(|buffer, size| unsafe { GetIpAddrTable(buffer, size, 0) }) else { return vec![IpAddr::V4(Ipv4Addr::LOCALHOST)] };
    // SAFETY: a MIB_IPADDRTABLE: a count, then rows of 24 bytes with the address first.
    let bytes = unsafe { std::slice::from_raw_parts(table.as_ptr().cast::<u8>(), table.len() * 8) };
    let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    (0..count)
        .filter_map(|i| bytes.get(4 + i * 24..8 + i * 24))
        .map(|a| IpAddr::V4(Ipv4Addr::new(a[0], a[1], a[2], a[3])))
        .collect()
}

/// The DNS servers Windows uses.
pub(super) fn dns_servers() -> Vec<SocketAddr> {
    // SAFETY: room for `size` bytes, which it says if it needs more.
    let Some(info) = grow(|buffer, size| unsafe { GetNetworkParams(buffer, size) }) else { return Vec::new() };
    let base = info.as_ptr().cast::<u8>();
    let mut out = Vec::new();
    // FIXED_INFO: the host and domain names (132 bytes each), the current server's pointer, then the first
    // IP_ADDR_STRING of the server list: a pointer to the next, and the address as text (16 bytes).
    // SAFETY: inside the buffer GetNetworkParams filled; `next` pointers point into it too.
    let mut entry = unsafe { base.add(272) };
    for _ in 0..16 {
        // SAFETY: as above.
        let (next, text) = unsafe { (entry.cast::<*const u8>().read_unaligned(), std::slice::from_raw_parts(entry.add(8), 16)) };
        let text = String::from_utf8_lossy(&text[..text.iter().position(|&b| b == 0).unwrap_or(16)]).into_owned();
        if let Ok(ip) = text.parse::<IpAddr>() {
            out.push(SocketAddr::new(ip, 53));
        }
        if next.is_null() {
            break;
        }
        entry = next.cast_mut();
    }
    out
}
