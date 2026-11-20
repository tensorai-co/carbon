//! A machine's network on the host's sockets, with its hooks asked first: every connection out (TCP, and each new
//! UDP destination) and every name looked up. It's a `net::Host` around `net::System`, so `net` itself needn't know
//! about hooks, and both kernels' sockets go through it.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};

use hooks::{Connect, Decision, Hub, Kind, Lookup, Proto};
use net::{Host, HostDatagram, HostListener, HostStream, Ready, Result, Shutdown, Waker};

/// A line a machine's `/etc/hosts` gets (`--add-host`, Docker's `ExtraHosts`): a name, and its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraHost {
    pub name: String,
    pub ip: IpAddr,
}

impl ExtraHost {
    pub fn new(name: impl Into<String>, ip: IpAddr) -> crate::Result<ExtraHost> {
        let name = name.into();
        if !crate::spec::valid_hostname(&name) {
            return Err(crate::Error::invalid(format!("--add-host {name}: a host name is letters, digits and '-', in labels separated by '.'")));
        }
        Ok(ExtraHost { name, ip })
    }

    /// `NAME:IP` or `NAME=IP`, as `--add-host` takes it; an IPv6 address as it is, or in brackets
    /// (`db:fd00::5`, `db=[fd00::5]`).
    pub fn parse(text: &str) -> crate::Result<ExtraHost> {
        let Some(at) = text.find([':', '=']) else {
            return Err(crate::Error::invalid(format!("--add-host {text}: NAME:IP")));
        };
        let (name, ip) = (&text[..at], text[at + 1..].trim_start_matches('[').trim_end_matches(']'));
        if ip == "host-gateway" {
            return Err(crate::Error::unsupported(format!(
                "--add-host {text}: a machine's gateway doesn't reach this host's own services, so there's no host-gateway yet"
            )));
        }
        let ip: IpAddr = ip.parse().map_err(|_| crate::Error::invalid(format!("--add-host {text}: {ip:?} isn't an IP address")))?;
        ExtraHost::new(name, ip)
    }
}

impl std::fmt::Display for ExtraHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.name, self.ip)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The names lookups gave addresses, so a connection's event can say which name it's to.
#[derive(Default)]
struct Names(Mutex<HashMap<IpAddr, String>>);

impl Names {
    fn learn(&self, name: &str, ips: &[IpAddr]) {
        let mut names = lock(&self.0);
        // Kept small: a machine that looks up a great many names forgets the oldest-learned at random.
        if names.len() > 4096 {
            names.clear();
        }
        for ip in ips {
            names.insert(*ip, name.to_string());
        }
    }

    fn of(&self, ip: IpAddr) -> Option<String> {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 => v4,
        };
        lock(&self.0).get(&ip).cloned()
    }
}

pub(crate) struct HookedNet {
    inner: net::System,
    hub: Hub,
    names: Arc<Names>,
    /// Its registries, answering at the gateway, if it has any.
    registries: Option<Arc<registries_core::Server>>,
}

impl HookedNet {
    pub fn new(hub: Hub) -> HookedNet {
        HookedNet { inner: net::System, hub, names: Arc::default(), registries: None }
    }

    /// With `registries` answering the gateway's port 80.
    pub fn with_registries(mut self, registries: Option<Arc<registries_core::Server>>) -> HookedNet {
        self.registries = registries;
        self
    }
}

/// A connection that was refused before it was tried.
struct Refused;

impl HostStream for Refused {
    fn connected(&self) -> Result<bool> {
        Err(net::Error::ConnectionRefused)
    }
    fn ready(&self) -> Ready {
        Ready { read: true, write: true, read_closed: true, hangup: true, error: true }
    }
    fn read(&self, _buffer: &mut [u8]) -> Result<usize> {
        Err(net::Error::ConnectionRefused)
    }
    fn write(&self, _data: &[u8]) -> Result<usize> {
        Err(net::Error::ConnectionRefused)
    }
    fn shutdown(&self, _how: Shutdown) {}
}

impl Host for HookedNet {
    fn connect(&self, to: SocketAddr, wake: Waker) -> Box<dyn HostStream> {
        if !self.hub.wants(Kind::Connect) {
            return self.inner.connect(to, wake);
        }
        match self.hub.connect(Connect { proto: Proto::Tcp, to, name: self.names.of(to.ip()) }) {
            Decision::Deny | Decision::Answer(_) => {
                // Woken once, so a program waiting for the connection sees it fail.
                wake();
                Box::new(Refused)
            }
            Decision::Rewrite(c) => self.inner.connect(c.to, wake),
            Decision::Pass | Decision::Allow => self.inner.connect(to, wake),
        }
    }

    fn udp(&self, v6: bool, wake: Waker) -> Result<Box<dyn HostDatagram>> {
        let inner = self.inner.udp(v6, wake)?;
        Ok(Box::new(HookedDatagram { inner, hub: self.hub.clone(), names: self.names.clone(), decided: Mutex::default() }))
    }

    fn lookup(&self, name: &str) -> Result<Vec<IpAddr>> {
        let found = if self.hub.wants(Kind::Lookup) {
            match self.hub.lookup(Lookup::new(name)) {
                Decision::Deny => return Err(net::Error::NotFound),
                Decision::Answer(ips) => Ok(ips),
                Decision::Rewrite(l) => self.inner.lookup(&l.name),
                Decision::Pass | Decision::Allow => self.inner.lookup(name),
            }
        } else {
            self.inner.lookup(name)
        };
        if let Ok(ips) = &found
            && self.hub.wants(Kind::Connect)
        {
            self.names.learn(name, ips);
        }
        found
    }

    fn query(&self, packet: &[u8], server: Option<SocketAddr>) -> Result<Vec<u8>> {
        self.inner.query(packet, server)
    }

    fn addresses(&self) -> Vec<IpAddr> {
        self.inner.addresses()
    }

    fn listen(&self, at: SocketAddr, wake: Waker) -> Result<Box<dyn HostListener>> {
        self.inner.listen(at, wake)
    }

    fn udp_at(&self, at: SocketAddr, wake: Waker) -> Result<Box<dyn HostDatagram>> {
        self.inner.udp_at(at, wake)
    }

    fn gateway(&self, port: u16, wake: Waker) -> Option<Box<dyn HostStream>> {
        match &self.registries {
            Some(server) if port == crate::registries::PORT => Some(crate::registries::connect(server, wake)),
            _ => None,
        }
    }
}

/// What's been decided for each destination a UDP socket sends to: allowed, refused, or sent somewhere else.
#[derive(Default)]
struct Decided {
    allowed: HashSet<SocketAddr>,
    refused: HashSet<SocketAddr>,
    /// Where each rewritten destination really goes, and back.
    sent_to: HashMap<SocketAddr, SocketAddr>,
    came_from: HashMap<SocketAddr, SocketAddr>,
}

/// A UDP socket whose first datagram to each destination is asked about, as a connection is.
struct HookedDatagram {
    inner: Box<dyn HostDatagram>,
    hub: Hub,
    names: Arc<Names>,
    decided: Mutex<Decided>,
}

impl HostDatagram for HookedDatagram {
    fn ready(&self) -> Ready {
        self.inner.ready()
    }

    fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize> {
        if !self.hub.wants(Kind::Connect) {
            return self.inner.send_to(data, to);
        }
        let known = {
            let d = lock(&self.decided);
            if d.refused.contains(&to) {
                return Err(net::Error::Denied);
            }
            d.sent_to.get(&to).copied().or_else(|| d.allowed.contains(&to).then_some(to))
        };
        let target = match known {
            Some(target) => target,
            None => match self.hub.connect(Connect { proto: Proto::Udp, to, name: self.names.of(to.ip()) }) {
                Decision::Deny | Decision::Answer(_) => {
                    lock(&self.decided).refused.insert(to);
                    return Err(net::Error::Denied);
                }
                Decision::Rewrite(c) => {
                    let mut d = lock(&self.decided);
                    d.sent_to.insert(to, c.to);
                    d.came_from.insert(c.to, to);
                    c.to
                }
                Decision::Pass | Decision::Allow => {
                    lock(&self.decided).allowed.insert(to);
                    to
                }
            },
        };
        self.inner.send_to(data, target)
    }

    fn recv_from(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let (n, from) = self.inner.recv_from(buffer)?;
        // An answer from where a rewrite sent it comes from where the program sent it.
        let shown = lock(&self.decided).came_from.get(&from).copied().unwrap_or(from);
        Ok((n, shown))
    }

    fn local(&self) -> Result<SocketAddr> {
        self.inner.local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_hosts_as_add_host_takes_them() {
        for (text, shown) in [("db:10.0.0.5", "db:10.0.0.5"), ("db=10.0.0.5", "db:10.0.0.5"), ("v6:fd00::5", "v6:fd00::5"), ("v6=[fd00::5]", "v6:fd00::5"), ("git.internal:10.8.0.2", "git.internal:10.8.0.2")] {
            let host = ExtraHost::parse(text).unwrap();
            assert_eq!(host.to_string(), shown);
            assert_eq!(ExtraHost::parse(&host.to_string()).unwrap(), host);
        }
        for bad in ["db", "db:", "db:nope", ":10.0.0.5", "bad name:10.0.0.5", "-db:10.0.0.5"] {
            assert!(ExtraHost::parse(bad).is_err(), "{bad}");
        }
        assert!(ExtraHost::parse("host.docker.internal:host-gateway").unwrap_err().to_string().contains("host-gateway"));
    }

    #[test]
    fn refused_connections_and_unknown_names() {
        let hub = Hub::new();
        hub.on_connect(hooks::Filter::port(9), |_| Decision::refuse());
        hub.on_lookup(hooks::Filter::host("blocked.test"), |_| Decision::nxdomain());
        hub.on_lookup(hooks::Filter::host("api.internal"), |_| Decision::answer(vec![IpAddr::from([10, 89, 0, 1])]));
        let seen = hub.observe(hooks::Interest::connect(), 8);
        let net = HookedNet::new(hub);
        let stream = net.connect(SocketAddr::from(([127, 0, 0, 1], 9)), Arc::new(|| {}));
        assert!(matches!(stream.connected(), Err(net::Error::ConnectionRefused)));
        assert!(matches!(net.lookup("x.blocked.test"), Err(net::Error::NotFound)));
        assert_eq!(net.lookup("api.internal").unwrap(), vec![IpAddr::from([10, 89, 0, 1])]);
        // The name a lookup gave goes with the connection's event.
        let _ = net.connect(SocketAddr::from(([10, 89, 0, 1], 9)), Arc::new(|| {}));
        let events: Vec<_> = std::iter::from_fn(|| seen.try_next()).collect();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[1].event, hooks::Event::Connect(c) if c.name() == Some("api.internal")));
    }
}
