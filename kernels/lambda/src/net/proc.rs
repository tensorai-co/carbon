//! `/proc/net`'s files, from `net`'s sockets and interfaces, and the machine's own `/etc/resolv.conf`, `/etc/hosts`
//! and `/etc/hostname`, made in memory as Docker bind-mounts its own: the image's copies are hidden.

use std::net::IpAddr;

use net::{Addr, Domain, Kind, UnixName};

use crate::Kernel;

/// The machine's version of an `/etc` file, if it's one the machine makes.
pub(crate) fn etc_file(k: &Kernel, path: &str) -> Option<Vec<u8>> {
    let hostname = &k.options.hostname;
    let text = match path {
        "/etc/resolv.conf" => format!("# The machine's DNS server, which is Carbon's.\nnameserver {}\n", k.net.gateway()),
        "/etc/hosts" => format!(
            "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost ip6-loopback\nfe00::0\tip6-localnet\nff00::0\tip6-mcastprefix\nff02::1\tip6-allnodes\nff02::2\tip6-allrouters\n{}\t{hostname}\n",
            k.net.address()
        ),
        "/etc/hostname" => format!("{hostname}\n"),
        _ => return None,
    };
    Some(text.into_bytes())
}

/// An IPv4 address as `/proc/net` writes it: its four bytes as one hex number, in the host's order.
fn hex4(ip: [u8; 4]) -> String {
    format!("{:08X}", u32::from_le_bytes(ip))
}

fn hex_ip(ip: IpAddr, v6: bool) -> String {
    match (ip, v6) {
        (IpAddr::V4(v4), false) => hex4(v4.octets()),
        (ip, _) => {
            let octets = match ip {
                IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
                IpAddr::V6(v6) => v6.octets(),
            };
            octets.chunks(4).map(|c| hex4([c[0], c[1], c[2], c[3]])).collect()
        }
    }
}

fn hex_addr(addr: Option<&Addr>, v6: bool) -> String {
    let (ip, port) = match addr {
        Some(Addr::Inet(a)) => (a.ip(), a.port()),
        _ => (if v6 { IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED) } else { IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED) }, 0),
    };
    format!("{}:{port:04X}", hex_ip(ip, v6))
}

/// `/proc/net/<name>`, if it's one lambda makes.
pub(crate) fn proc_net(k: &Kernel, name: &str) -> Option<String> {
    let sockets = k.net.sockets();
    let inet = |kind: Kind, v6: bool| {
        let domain = if v6 { Domain::Inet6 } else { Domain::Inet };
        let mut out = String::from("  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n");
        for (i, s) in sockets.iter().filter(|s| s.kind == kind && s.domain == domain).enumerate() {
            // A datagram socket is "closed" (7) unless it's connected (1).
            let state = if kind == Kind::Datagram { if s.peer.is_some() { 1 } else { 7 } } else { s.state as u8 };
            out += &format!(
                "{i:4}: {} {} {state:02X} {:08X}:{:08X} 00:00000000 00000000     0        0 {} 1 0000000000000000 100 0 0 10 0\n",
                hex_addr(s.local.as_ref(), v6),
                hex_addr(s.peer.as_ref(), v6),
                s.queued.1,
                s.queued.0,
                s.id
            );
        }
        out
    };
    Some(match name {
        "tcp" => inet(Kind::Stream, false),
        "tcp6" => inet(Kind::Stream, true),
        "udp" => inet(Kind::Datagram, false),
        "udp6" => inet(Kind::Datagram, true),
        "unix" => {
            let mut out = String::from("Num       RefCount Protocol Flags    Type St Inode Path\n");
            for s in sockets.iter().filter(|s| s.domain == Domain::Unix) {
                let kind = match s.kind {
                    Kind::Stream => 1,
                    Kind::Datagram => 2,
                    Kind::SeqPacket => 5,
                };
                let listening = s.state == net::TcpState::Listen;
                let state = if s.peer.is_some() { 3 } else { 1 };
                let path = match &s.local {
                    Some(Addr::Unix(u)) => match &u.name {
                        UnixName::Path(p) => format!(" {}", String::from_utf8_lossy(p)),
                        UnixName::Abstract(n) => format!(" @{}", String::from_utf8_lossy(n)),
                        UnixName::Unnamed => String::new(),
                    },
                    _ => String::new(),
                };
                out += &format!("0000000000000000: 00000002 00000000 {:08X} {kind:04X} {state:02X} {}{path}\n", if listening { 0x10000 } else { 0 }, s.id);
            }
            out
        }
        "dev" => {
            let mut out = String::from(
                "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n",
            );
            for iface in k.net.interfaces() {
                out += &format!("{:>6}: {}\n", iface.name, ["0"; 16].join(" "));
            }
            out
        }
        "route" => {
            let mut out = String::from("Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n");
            for r in k.net.routes() {
                let mask = u32::MAX.checked_shl(32 - u32::from(r.prefix)).unwrap_or(0);
                let flags = 1 | if r.gateway.is_some() { 2 } else { 0 };
                out += &format!(
                    "eth0\t{}\t{}\t{flags:04X}\t0\t0\t0\t{}\t0\t0\t0\n",
                    hex4(r.dst.octets()),
                    hex4(r.gateway.map_or([0; 4], |g| g.octets())),
                    hex4(mask.to_be_bytes())
                );
            }
            out
        }
        "if_inet6" => "00000000000000000000000000000001 01 80 10 80       lo\n".to_string(),
        _ => return None,
    })
}

/// A socket address, for tests.
#[cfg(test)]
fn addr(s: &str) -> Addr {
    Addr::Inet(s.parse::<std::net::SocketAddr>().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_as_proc_net_has_them() {
        assert_eq!(hex_addr(Some(&addr("127.0.0.1:80")), false), "0100007F:0050");
        assert_eq!(hex_addr(Some(&addr("[::1]:22")), true), "00000000000000000000000001000000:0016");
        assert_eq!(hex_addr(None, false), "00000000:0000");
    }
}
