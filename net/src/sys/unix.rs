//! macOS and Linux: `poll`, `getifaddrs`, and `/etc/resolv.conf`'s servers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::os::fd::AsRawFd;

use super::{READ, Raw, WRITE};

pub(super) fn raw_tcp(stream: &TcpStream) -> Raw {
    stream.as_raw_fd() as Raw
}

pub(super) fn raw_udp(socket: &UdpSocket) -> Raw {
    socket.as_raw_fd() as Raw
}

/// Waits until one of `entries` (a socket and the ways it's wanted) is ready; what each is ready for. Hung up or in
/// error counts as both ways.
pub(super) fn poll(entries: &[(Raw, u8)]) -> Vec<u8> {
    let mut fds: Vec<libc::pollfd> = entries
        .iter()
        .map(|&(raw, ways)| libc::pollfd {
            fd: raw as i32,
            events: (if ways & READ != 0 { libc::POLLIN } else { 0 }) | (if ways & WRITE != 0 { libc::POLLOUT } else { 0 }),
            revents: 0,
        })
        .collect();
    // SAFETY: `fds` is a live array of `fds.len()` pollfds.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
    if n < 0 {
        return vec![0; entries.len()];
    }
    fds.iter()
        .map(|fd| {
            let both = fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
            (if fd.revents & libc::POLLIN != 0 || both { READ } else { 0 }) | (if fd.revents & libc::POLLOUT != 0 || both { WRITE } else { 0 })
        })
        .collect()
}

pub(super) fn set_keepalive(stream: &TcpStream, on: bool) {
    let value: libc::c_int = on.into();
    // SAFETY: an int option, with its size.
    unsafe {
        libc::setsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_KEEPALIVE, (&raw const value).cast(), size_of::<libc::c_int>() as libc::socklen_t);
    }
}

/// Every address on the host's interfaces.
pub(super) fn addresses() -> Vec<IpAddr> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: it fills in `list`, which we free.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = list;
    while !at.is_null() {
        // SAFETY: a node of the list getifaddrs made.
        let entry = unsafe { &*at };
        if !entry.ifa_addr.is_null() {
            // SAFETY: a sockaddr of the family its first field says.
            unsafe {
                match i32::from((*entry.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let sin = &*entry.ifa_addr.cast::<libc::sockaddr_in>();
                        out.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))));
                    }
                    libc::AF_INET6 => {
                        let sin6 = &*entry.ifa_addr.cast::<libc::sockaddr_in6>();
                        out.push(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr)));
                    }
                    _ => {}
                }
            }
        }
        at = entry.ifa_next;
    }
    // SAFETY: what getifaddrs gave.
    unsafe { libc::freeifaddrs(list) };
    out
}

/// The DNS servers `/etc/resolv.conf` names.
pub(super) fn dns_servers() -> Vec<SocketAddr> {
    std::fs::read_to_string("/etc/resolv.conf").map(|text| parse_resolv_conf(&text)).unwrap_or_default()
}

pub(super) fn parse_resolv_conf(text: &str) -> Vec<SocketAddr> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("nameserver")).then(|| words.next()).flatten()
        })
        // A scope (`fe80::1%en0`) is dropped: it's the host's own link.
        .filter_map(|addr| addr.split('%').next()?.parse::<IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, 53))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolv_conf_is_read() {
        let servers = super::parse_resolv_conf("# a comment\nsearch example.com\nnameserver 1.1.1.1\nnameserver fe80::1%en0\nnameserver nonsense\n");
        assert_eq!(servers, vec!["1.1.1.1:53".parse().unwrap(), "[fe80::1]:53".parse().unwrap()]);
    }
}
