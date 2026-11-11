//! Netlink's routing family (`NETLINK_ROUTE`), as far as programs ask it: the machine's links, addresses and routes,
//! dumped as `RTM_NEWLINK`, `RTM_NEWADDR` and `RTM_NEWROUTE` messages. That's what glibc's `getifaddrs` and
//! `getaddrinfo` (for `AI_ADDRCONFIG`), `ip` and `ifconfig` read. Each request's answer is one datagram, then a
//! `NLMSG_DONE` in another.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kernels_core::lock;
use kernels_core::wait::Watchers;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWLINK: u16 = 16;
const RTM_GETLINK: u16 = 18;
const RTM_NEWADDR: u16 = 20;
const RTM_GETADDR: u16 = 22;
const RTM_NEWROUTE: u16 = 24;
const RTM_GETROUTE: u16 = 26;
const NLM_F_MULTI: u16 = 2;
const NLM_F_ACK: u16 = 4;
const NLM_F_DUMP: u16 = 0x300;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;
const EOPNOTSUPP: i32 = 95;
const ENODEV: i32 = 19;

/// A netlink socket's numbers, for `/proc/self/fd`.
static NEXT_ID: AtomicU64 = AtomicU64::new(1 << 32);

pub(crate) struct Netlink {
    /// `SOCK_RAW` or `SOCK_DGRAM`, as it was made.
    pub kind: u32,
    pub id: u64,
    /// Its port: 0 until it's bound or first sends.
    port: AtomicU32,
    queue: Mutex<VecDeque<Vec<u8>>>,
    pub watchers: Arc<Watchers>,
}

/// One netlink message: its header and `body`.
fn message(kind: u16, flags: u16, seq: u32, port: u32, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(16 + body.len());
    m.extend(((16 + body.len()) as u32).to_le_bytes());
    m.extend(kind.to_le_bytes());
    m.extend(flags.to_le_bytes());
    m.extend(seq.to_le_bytes());
    m.extend(port.to_le_bytes());
    m.extend(body);
    m.resize(m.len().next_multiple_of(4), 0);
    m
}

/// A route attribute.
fn attr(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    out.extend(((4 + data.len()) as u16).to_le_bytes());
    out.extend(kind.to_le_bytes());
    out.extend(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

fn ip_bytes(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => v6.octets().to_vec(),
    }
}

fn link(iface: &net::Interface) -> Vec<u8> {
    // ifinfomsg: family, type (ARPHRD_LOOPBACK or ARPHRD_ETHER), index, flags (up, running, lower up, and loopback or
    // broadcast and multicast), change.
    let flags: u32 = if iface.loopback { 0x1 | 0x8 | 0x40 | 0x10000 } else { 0x1 | 0x2 | 0x40 | 0x1000 | 0x10000 };
    let mut body = vec![0, 0];
    body.extend((if iface.loopback { 772u16 } else { 1u16 }).to_le_bytes());
    body.extend((iface.index as i32).to_le_bytes());
    body.extend(flags.to_le_bytes());
    body.extend(0u32.to_le_bytes());
    let mut name = iface.name.as_bytes().to_vec();
    name.push(0);
    attr(&mut body, 3, &name);
    attr(&mut body, 4, &iface.mtu.to_le_bytes());
    attr(&mut body, 1, &iface.mac);
    attr(&mut body, 2, &if iface.loopback { [0; 6] } else { [0xff; 6] });
    attr(&mut body, 13, &1000u32.to_le_bytes());
    // IFLA_OPERSTATE: loopback's is unknown, as on Linux; eth0's is up.
    attr(&mut body, 16, &[if iface.loopback { 0 } else { 6 }]);
    body
}

fn address(iface: &net::Interface, ip: IpAddr, prefix: u8) -> Vec<u8> {
    let family = if ip.is_ipv4() { AF_INET } else { AF_INET6 };
    // ifaddrmsg: family, prefix length, flags (permanent), scope (host for loopback), index.
    let mut body = vec![family, prefix, 0x80, if iface.loopback { 254 } else { 0 }];
    body.extend(iface.index.to_le_bytes());
    attr(&mut body, 1, &ip_bytes(ip));
    if let IpAddr::V4(v4) = ip {
        attr(&mut body, 2, &v4.octets());
        if !iface.loopback {
            let broadcast = u32::from(v4) | u32::MAX.checked_shr(u32::from(prefix)).unwrap_or(0);
            attr(&mut body, 4, &broadcast.to_be_bytes());
        }
        let mut name = iface.name.as_bytes().to_vec();
        name.push(0);
        attr(&mut body, 3, &name);
    }
    body
}

fn route(r: &net::Route) -> Vec<u8> {
    // rtmsg: family, destination and source lengths, tos, table (main), protocol (boot, or kernel for the link's own),
    // scope (universe, or link), type (unicast), flags.
    let own = r.gateway.is_none();
    let mut body = vec![AF_INET, r.prefix, 0, 0, 254, if own { 2 } else { 3 }, if own { 253 } else { 0 }, 1];
    body.extend(0u32.to_le_bytes());
    attr(&mut body, 15, &254u32.to_le_bytes());
    if r.prefix > 0 {
        attr(&mut body, 1, &r.dst.octets());
    }
    if let Some(gateway) = r.gateway {
        attr(&mut body, 5, &gateway.octets());
    }
    if let Some(source) = r.source {
        attr(&mut body, 7, &source.octets());
    }
    attr(&mut body, 4, &r.index.to_le_bytes());
    body
}

impl Netlink {
    pub fn new(kind: u32) -> Arc<Netlink> {
        Arc::new(Netlink { kind, id: NEXT_ID.fetch_add(1, Ordering::Relaxed), port: AtomicU32::new(0), queue: Mutex::default(), watchers: Arc::default() })
    }

    pub fn port(&self) -> u32 {
        self.port.load(Ordering::Acquire)
    }

    pub fn bind(&self, port: u32) {
        self.port.store(port, Ordering::Release);
    }

    pub fn readable(&self) -> bool {
        !lock(&self.queue).is_empty()
    }

    pub fn available(&self) -> usize {
        lock(&self.queue).front().map_or(0, Vec::len)
    }

    /// The next datagram, cut to `len`, and its whole length; None if there's none yet.
    pub fn recv(&self, len: usize, peek: bool) -> Option<(Vec<u8>, usize)> {
        let mut queue = lock(&self.queue);
        let front = queue.front()?;
        let got = (front[..len.min(front.len())].to_vec(), front.len());
        if !peek {
            queue.pop_front();
        }
        Some(got)
    }

    /// Answers every request in `data`. A socket that's never been bound takes `default_port` (its process's ID).
    pub fn request(&self, net: &net::Network, data: &[u8], default_port: u32) {
        let _ = self.port.compare_exchange(0, default_port, Ordering::AcqRel, Ordering::Acquire);
        let port = self.port();
        let mut at = 0;
        let mut replies = Vec::new();
        while at + 16 <= data.len() {
            let len = u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
            if len < 16 || at + len > data.len() {
                break;
            }
            let kind = u16::from_le_bytes([data[at + 4], data[at + 5]]);
            let flags = u16::from_le_bytes([data[at + 6], data[at + 7]]);
            let seq = u32::from_le_bytes([data[at + 8], data[at + 9], data[at + 10], data[at + 11]]);
            let header = &data[at..at + 16];
            let body = &data[at + 16..at + len];
            // The family asked for, if any: the first byte of every request's body.
            let family = body.first().copied().unwrap_or(0);
            let dump = flags & NLM_F_DUMP == NLM_F_DUMP;
            let error = |code: i32| {
                let mut b = (-code).to_le_bytes().to_vec();
                b.extend(header);
                message(NLMSG_ERROR, 0, seq, port, &b)
            };
            let done = || message(NLMSG_DONE, NLM_F_MULTI, seq, port, &0i32.to_le_bytes());
            let interfaces = net.interfaces();
            match kind {
                RTM_GETLINK if dump => {
                    replies.push(interfaces.iter().flat_map(|i| message(RTM_NEWLINK, NLM_F_MULTI, seq, port, &link(i))).collect());
                    replies.push(done());
                }
                RTM_GETLINK => {
                    let index = body.get(4..8).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    match interfaces.iter().find(|i| i.index as i32 == index) {
                        Some(iface) => replies.push(message(RTM_NEWLINK, 0, seq, port, &link(iface))),
                        None => replies.push(error(ENODEV)),
                    }
                }
                RTM_GETADDR if dump => {
                    let mut out = Vec::new();
                    for iface in &interfaces {
                        for &(ip, prefix) in &iface.addrs {
                            let f = if ip.is_ipv4() { AF_INET } else { AF_INET6 };
                            if family == 0 || family == f {
                                out.extend(message(RTM_NEWADDR, NLM_F_MULTI, seq, port, &address(iface, ip, prefix)));
                            }
                        }
                    }
                    replies.push(out);
                    replies.push(done());
                }
                RTM_GETROUTE if dump => {
                    if family == 0 || family == AF_INET {
                        replies.push(net.routes().iter().flat_map(|r| message(RTM_NEWROUTE, NLM_F_MULTI, seq, port, &route(r))).collect());
                    }
                    replies.push(done());
                }
                // An error is its own acknowledgement.
                _ => replies.push(error(EOPNOTSUPP)),
            }
            if flags & NLM_F_ACK != 0 && !dump && kind == RTM_GETLINK {
                replies.push(error(0));
            }
            at += len.next_multiple_of(4);
        }
        if replies.is_empty() {
            return;
        }
        lock(&self.queue).extend(replies.into_iter().filter(|r| !r.is_empty()));
        self.watchers.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(kind: u16, family: u8) -> Vec<u8> {
        message(kind, 1 | NLM_F_DUMP, 9, 0, &[family, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    }

    /// The messages in a datagram: (type, body).
    fn split(datagram: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 16 <= datagram.len() {
            let len = u32::from_le_bytes(datagram[at..at + 4].try_into().unwrap()) as usize;
            out.push((u16::from_le_bytes([datagram[at + 4], datagram[at + 5]]), datagram[at + 16..at + len].to_vec()));
            at += len.next_multiple_of(4);
        }
        out
    }

    #[test]
    fn links_addresses_and_routes() {
        let net = net::Network::new(net::Config::default(), Arc::new(net::Loop::new()));
        let nl = Netlink::new(3);
        nl.request(&net, &dump(RTM_GETLINK, 0), 42);
        assert_eq!(nl.port(), 42);
        let (links, _) = nl.recv(1 << 16, false).unwrap();
        let links = split(&links);
        assert_eq!(links.len(), 2);
        assert!(links[1].1.windows(5).any(|w| w == b"eth0\0"));
        let (done, _) = nl.recv(1 << 16, false).unwrap();
        assert_eq!(split(&done)[0].0, NLMSG_DONE);
        nl.request(&net, &dump(RTM_GETADDR, AF_INET), 42);
        let (addrs, _) = nl.recv(1 << 16, false).unwrap();
        // lo's 127.0.0.1 and eth0's address; not ::1.
        assert_eq!(split(&addrs).len(), 2);
        nl.recv(1 << 16, false).unwrap();
        nl.request(&net, &dump(RTM_GETROUTE, 0), 42);
        let (routes, _) = nl.recv(1 << 16, false).unwrap();
        assert_eq!(split(&routes).len(), 2);
        nl.recv(1 << 16, false).unwrap();
        nl.request(&net, &message(99, 1 | NLM_F_ACK, 1, 0, &[]), 42);
        let (error, _) = nl.recv(1 << 16, false).unwrap();
        assert_eq!(split(&error)[0].0, NLMSG_ERROR);
        assert!(!nl.readable());
    }
}
