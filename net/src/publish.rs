//! Ports in: a host port that reaches a machine's port, as `--publish 2222:22` asks, and what `--network host` does
//! for every port a program listens on.
//!
//! `net` listens on the host's port and hands each connection to the socket in the machine that listens on the port
//! it's published to. The connection isn't made again inside: the host's accepted connection becomes the machine's
//! accepted socket, so the program reads and writes the host's TCP connection directly, and `accept` gives the client's
//! own address. A client on the host's loopback shows as the machine's gateway, as it would through Docker's proxy, so
//! a program trusting `127.0.0.1` doesn't trust the host's every process. With nothing listening in the machine, the
//! connection is closed at once.
//!
//! A UDP port works the same way, datagram by datagram, with each one's real source; what the machine sends back to
//! a client from the published port goes out through the host socket the client sent to, so the client sees the
//! answer come from the port it asked.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, Weak};

use crate::socket::{Datagram, Inner};
use crate::watch::waker_for;
use crate::{Addr, Domain, Error, HostDatagram, HostListener, Kind, Mode, Result, Shared, Socket, as_domain, lock, v4_of};

/// The most UDP clients a published port remembers, to send their answers back.
const MOST_CLIENTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        })
    }
}

/// A port to publish, as `--publish` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Publish {
    /// The host address to listen on; None for all of them, IPv4 and IPv6.
    pub host_ip: Option<IpAddr>,
    /// The host's port; 0 for a free one.
    pub host_port: u16,
    pub machine_port: u16,
    pub protocol: Protocol,
}

impl Publish {
    /// `--publish`'s forms, as Docker's: `[HOST_IP:][HOST_PORT:]MACHINE_PORT[/tcp|/udp]`. An IPv6 address is in
    /// brackets (`[::1]:8080:80`); a port may be a range (`8000-8009:8000-8009`, or `8000-8009` alone for free host
    /// ports); leaving out the host port, or leaving it empty (`127.0.0.1::80`), takes a free one.
    pub fn parse(text: &str) -> std::result::Result<Vec<Publish>, String> {
        let bad = |why: &str| format!("{text}: {why} (as [HOST_IP:][HOST_PORT:]PORT[/tcp|udp], e.g. 2222:22 or 127.0.0.1:8080:80/udp)");
        let (rest, protocol) = match text.rsplit_once('/') {
            Some((rest, "tcp")) => (rest, Protocol::Tcp),
            Some((rest, "udp")) => (rest, Protocol::Udp),
            Some((_, other)) => return Err(bad(&format!("{other} isn't tcp or udp"))),
            None => (text, Protocol::Tcp),
        };
        // An IPv6 address in brackets, then the rest as for IPv4.
        let (host_ip, ports) = if let Some(inner) = rest.strip_prefix('[') {
            let (ip, after) = inner.split_once(']').ok_or_else(|| bad("an IPv6 address's ] is missing"))?;
            let ip: Ipv6Addr = ip.parse().map_err(|_| bad(&format!("{ip} isn't an IPv6 address")))?;
            let after = after.strip_prefix(':').ok_or_else(|| bad("a port goes after the address"))?;
            (Some(IpAddr::V6(ip)), after)
        } else {
            let parts: Vec<&str> = rest.split(':').collect();
            match parts.len() {
                1 | 2 => (None, rest),
                3 => {
                    let ip: Ipv4Addr = parts[0].parse().map_err(|_| bad(&format!("{} isn't an IPv4 address", parts[0])))?;
                    (Some(IpAddr::V4(ip)), &rest[parts[0].len() + 1..])
                }
                _ => return Err(bad("too many parts")),
            }
        };
        let (host, machine) = match ports.split_once(':') {
            Some((host, machine)) => (host, machine),
            None => ("", ports),
        };
        let range = |text: &str, zero_ok: bool| -> std::result::Result<(u16, u16), String> {
            let (first, last) = text.split_once('-').unwrap_or((text, text));
            let port = |p: &str| p.parse::<u16>().ok().filter(|&p| p != 0 || zero_ok).ok_or_else(|| bad(&format!("{p} isn't a port")));
            let (first, last) = (port(first)?, port(last)?);
            if last < first {
                return Err(bad("a range goes up"));
            }
            Ok((first, last))
        };
        let (machine_first, machine_last) = range(machine, false)?;
        let count = machine_last - machine_first;
        let host_first = if host.is_empty() {
            None
        } else {
            let (first, last) = range(host, false)?;
            if last - first != count {
                return Err(bad("the host's range and the machine's are different lengths"));
            }
            Some(first)
        };
        Ok((0..=count).map(|i| Publish { host_ip, host_port: host_first.map_or(0, |h| h + i), machine_port: machine_first + i, protocol }).collect())
    }
}

/// A host address a machine's port is published on, as `carbon port` and `carbon ps` show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    pub host: SocketAddr,
    pub machine_port: u16,
    pub protocol: Protocol,
}

impl fmt::Display for Bound {
    /// Docker's way: `0.0.0.0:2222->22/tcp`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}->{}/{}", self.host, self.machine_port, self.protocol)
    }
}

/// Who gets what comes in on a published port.
enum Target {
    /// Whatever listens on (or is bound to) the port on the machine's address: `--publish`.
    Port,
    /// This socket: `--network host`'s, for the port it listens on.
    Socket(Weak<Inner>),
}

/// One published port: its host sockets, and for UDP, the clients that sent to it.
pub(crate) struct Entry {
    net: Weak<Shared>,
    protocol: Protocol,
    pub machine_port: u16,
    target: Target,
    listeners: Mutex<Vec<Box<dyn HostListener>>>,
    sockets: Mutex<Vec<Box<dyn HostDatagram>>>,
    /// A client as the machine sees it, and which host socket it sent to and its real address.
    clients: Mutex<HashMap<SocketAddr, (usize, SocketAddr)>>,
    bound: Mutex<Vec<SocketAddr>>,
}

/// Published ports, listening on the host until this is dropped.
pub struct Published {
    entries: Vec<Arc<Entry>>,
}

impl fmt::Debug for Published {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.bound()).finish()
    }
}

impl Published {
    /// What it listens on, each host address it's bound to.
    pub fn bound(&self) -> Vec<Bound> {
        self.entries
            .iter()
            .flat_map(|e| lock(&e.bound).iter().map(|&host| Bound { host, machine_port: e.machine_port, protocol: e.protocol }).collect::<Vec<_>>())
            .collect()
    }

    pub(crate) fn none() -> Published {
        Published { entries: Vec::new() }
    }

    pub(crate) fn join(&mut self, other: Published) {
        self.entries.extend(other.entries);
    }
}

/// An IP address as one family: IPv4, or IPv4 again if it's mapped.
fn plain(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(v4_of(addr.ip()).map_or(addr.ip(), IpAddr::V4), addr.port())
}

impl Entry {
    /// Listens on the host for `publish`, for `target`.
    fn open(net: &Arc<Shared>, publish: Publish, target: Target) -> Result<Arc<Entry>> {
        let entry = Arc::new(Entry {
            net: Arc::downgrade(net),
            protocol: publish.protocol,
            machine_port: publish.machine_port,
            target,
            listeners: Mutex::new(Vec::new()),
            sockets: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            bound: Mutex::new(Vec::new()),
        });
        let weak = Arc::downgrade(&entry);
        let wake: crate::Waker = Arc::new(move || {
            if let Some(entry) = weak.upgrade() {
                entry.pump();
            }
        });
        // Every address means IPv6's and IPv4's: [::] first, then 0.0.0.0 on the port it got, which a host whose [::]
        // takes IPv4 too (Linux's) refuses as in use, and that's fine. A host with no IPv6 has 0.0.0.0 alone.
        let wanted: Vec<IpAddr> = match publish.host_ip {
            Some(ip) => vec![ip],
            None => vec![IpAddr::V6(Ipv6Addr::UNSPECIFIED), IpAddr::V4(Ipv4Addr::UNSPECIFIED)],
        };
        let mut port = publish.host_port;
        let (mut listeners, mut sockets, mut bound) = (Vec::new(), Vec::new(), Vec::new());
        let mut first_error = None;
        for (i, ip) in wanted.iter().enumerate() {
            let at = SocketAddr::new(*ip, port);
            let opened = match publish.protocol {
                Protocol::Tcp => net.host.listen(at, wake.clone()).map(|l| {
                    let local = l.local();
                    listeners.push(l);
                    local
                }),
                Protocol::Udp => net.host.udp_at(at, wake.clone()).map(|s| {
                    let local = s.local().unwrap_or(at);
                    sockets.push(s);
                    local
                }),
            };
            match opened {
                Ok(local) => {
                    port = local.port();
                    bound.push(local);
                }
                // IPv4's, after IPv6's took every address.
                Err(Error::AddrInUse) if i > 0 && !bound.is_empty() => {}
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        if bound.is_empty() {
            return Err(first_error.unwrap_or(Error::AddrNotAvailable));
        }
        *lock(&entry.listeners) = listeners;
        *lock(&entry.sockets) = sockets;
        *lock(&entry.bound) = bound;
        if publish.protocol == Protocol::Udp {
            let mut published = lock(&net.published_udp);
            published.retain(|e| e.strong_count() > 0);
            published.push(Arc::downgrade(&entry));
        }
        // Anything that came before the lists were filled, its wake-up gone to an empty one.
        entry.pump();
        Ok(entry)
    }

    /// What a client is, as the machine sees it: its own address, but the gateway for the host's loopback (unless the
    /// machine has the host's network) or for an IPv6 client of an IPv4 socket.
    fn shown(&self, net: &Shared, from: SocketAddr, domain: Domain) -> SocketAddr {
        let from = plain(from);
        let gateway = SocketAddr::new(IpAddr::V4(net.gateway), from.port());
        let loopback = from.ip().is_loopback();
        if loopback && net.config.mode == Mode::Host {
            return if domain == Domain::Inet { SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), from.port()) } else { from };
        }
        if loopback || from.ip().is_unspecified() || (from.is_ipv6() && domain == Domain::Inet) { gateway } else { from }
    }

    /// The socket that takes what comes in: a listener for TCP, a bound socket for UDP.
    fn target(&self, net: &Shared) -> Option<Arc<Inner>> {
        match &self.target {
            Target::Socket(socket) => socket.upgrade(),
            Target::Port => {
                let tcp = self.protocol == Protocol::Tcp;
                net.bound(tcp, IpAddr::V4(net.address), self.machine_port).into_iter().find(|s| if tcp { s.is_listening() && s.kind == Kind::Stream } else { s.kind == Kind::Datagram })
            }
        }
    }

    /// Takes what's waiting on the host's sockets into the machine.
    fn pump(&self) {
        let Some(net) = self.net.upgrade() else { return };
        match self.protocol {
            Protocol::Tcp => self.pump_tcp(&net),
            Protocol::Udp => self.pump_udp(&net),
        }
    }

    fn pump_tcp(&self, net: &Arc<Shared>) {
        let listeners = lock(&self.listeners);
        for listener in listeners.iter() {
            loop {
                let target = self.target(net);
                let domain = target.as_ref().map_or(Domain::Inet, |t| t.domain);
                let server = Socket::new(net, domain, Kind::Stream);
                let Ok((stream, from)) = listener.accept(waker_for(&server.0.watchers)) else { break };
                // Nothing listening: the connection closes at once, as a refused one would.
                let Some(target) = target else { continue };
                let local = match target.local_inet() {
                    Some(local) if !local.ip().is_unspecified() => local,
                    _ => SocketAddr::new(IpAddr::V4(net.address), self.machine_port),
                };
                let shown = self.shown(net, from, domain);
                let _ = Socket::arrive(&target, server, stream, local, shown);
            }
        }
    }

    fn pump_udp(&self, net: &Arc<Shared>) {
        let sockets = lock(&self.sockets);
        for (i, socket) in sockets.iter().enumerate() {
            loop {
                let mut buffer = vec![0u8; 65536];
                let Ok((n, from)) = socket.recv_from(&mut buffer) else { break };
                buffer.truncate(n);
                // Nobody bound to the port: dropped, as UDP's own would be.
                let Some(target) = self.target(net) else { continue };
                let shown = self.shown(net, from, target.domain);
                {
                    let mut clients = lock(&self.clients);
                    if clients.len() >= MOST_CLIENTS && !clients.contains_key(&shown) {
                        clients.clear();
                    }
                    clients.insert(shown, (i, from));
                }
                let _ = target.deliver(Datagram { from: Addr::Inet(as_domain(target.domain, shown)), data: buffer, extra: None, cred: None }, &mut None);
            }
        }
    }

    /// Sends a datagram back to a client that sent to this port, through the host socket it sent to. None if `to`
    /// isn't one of its clients.
    fn answer(&self, to: SocketAddr, data: &[u8]) -> Option<Result<usize>> {
        let (i, real) = *lock(&self.clients).get(&plain(to))?;
        let sockets = lock(&self.sockets);
        Some(sockets.get(i)?.send_to(data, real))
    }
}

impl Shared {
    /// Publishes a port for whatever listens on (or is bound to) it on the machine's address.
    pub(crate) fn publish(self: &Arc<Self>, publish: Publish) -> Result<Published> {
        Ok(Published { entries: vec![Entry::open(self, publish, Target::Port)?] })
    }

    /// `--network host`'s: the host's port `port` on `ip` (any address for the machine's own or none), for `socket`.
    pub(crate) fn publish_socket(self: &Arc<Self>, socket: &Arc<Inner>, ip: IpAddr, port: u16, protocol: Protocol) -> Result<Published> {
        let host_ip = if ip.is_unspecified() || v4_of(ip) == Some(self.address) { None } else { Some(ip) };
        let publish = Publish { host_ip, host_port: port, machine_port: port, protocol };
        Ok(Published { entries: vec![Entry::open(self, publish, Target::Socket(Arc::downgrade(socket)))?] })
    }

    /// A datagram from the machine's port `port` to `to`: sent back through a published port's host socket if `to` is
    /// one of its clients. None if it isn't.
    pub(crate) fn answer(&self, port: u16, to: SocketAddr, data: &[u8]) -> Option<Result<usize>> {
        let entries: Vec<Arc<Entry>> = lock(&self.published_udp).iter().filter_map(Weak::upgrade).filter(|e| e.machine_port == port).collect();
        entries.iter().find_map(|e| e.answer(to, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Publish {
        let mut all = Publish::parse(text).unwrap();
        assert_eq!(all.len(), 1, "{text}");
        all.remove(0)
    }

    #[test]
    fn publish_is_parsed_as_dockers() {
        let p = |ip: Option<&str>, host, machine, protocol| Publish { host_ip: ip.map(|i| i.parse().unwrap()), host_port: host, machine_port: machine, protocol };
        assert_eq!(one("2222:22"), p(None, 2222, 22, Protocol::Tcp));
        assert_eq!(one("80"), p(None, 0, 80, Protocol::Tcp));
        assert_eq!(one("5353:53/udp"), p(None, 5353, 53, Protocol::Udp));
        assert_eq!(one("127.0.0.1:8080:80"), p(Some("127.0.0.1"), 8080, 80, Protocol::Tcp));
        assert_eq!(one("127.0.0.1::80/tcp"), p(Some("127.0.0.1"), 0, 80, Protocol::Tcp));
        assert_eq!(one("[::1]:8080:80"), p(Some("::1"), 8080, 80, Protocol::Tcp));
        assert_eq!(one("[::]::80/udp"), p(Some("::"), 0, 80, Protocol::Udp));
        let range = Publish::parse("8000-8002:9000-9002").unwrap();
        assert_eq!(range.iter().map(|p| (p.host_port, p.machine_port)).collect::<Vec<_>>(), [(8000, 9000), (8001, 9001), (8002, 9002)]);
        assert_eq!(Publish::parse("7000-7001").unwrap().iter().map(|p| (p.host_port, p.machine_port)).collect::<Vec<_>>(), [(0, 7000), (0, 7001)]);
        for wrong in ["", "x", "0", "80/sctp", "1:2:3:4", "nonsense:80:80", "8000-8001:80", "[::1:80", "70000:80", "9-8"] {
            assert!(Publish::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn bound_shows_as_docker_does() {
        let b = Bound { host: "0.0.0.0:2222".parse().unwrap(), machine_port: 22, protocol: Protocol::Tcp };
        assert_eq!(b.to_string(), "0.0.0.0:2222->22/tcp");
        let b = Bound { host: "[::]:5353".parse().unwrap(), machine_port: 53, protocol: Protocol::Udp };
        assert_eq!(b.to_string(), "[::]:5353->53/udp");
    }
}
