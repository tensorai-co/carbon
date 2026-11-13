//! A machine's network. Every program's sockets, in either kernel, are `net`'s [`Socket`]s: it gives each machine
//! addresses, a loopback and Unix sockets of its own, a DNS server at its gateway, and carries what goes out over the
//! host's own sockets ([`Host`]). There's no TCP/IP stack here: a TCP connection out of the machine is a host TCP
//! connection, made when the program calls `connect`.
//!
//! `net` never blocks. A socket says what's ready ([`Socket::ready`]) and calls its watchers ([`Socket::watch`]) when
//! that may have changed; blocking, timeouts and signals are the kernel's.
//!
//! - **The machine's own network** is a `/16` (by default the first of `10.89.0.0/16`, `10.90.0.0/16`, … that no host
//!   interface is on): the machine is `.0.2`, and `.0.1` is its gateway and DNS server, both `net` itself.
//! - **Loopback** (`127.0.0.0/8`, `::1` and the machine's own address) and **Unix sockets** are joined in memory.
//! - **Out**: TCP and UDP to anywhere else go through the host. The program only ever sees the machine's address.
//! - **DNS**: the machine's names are answered here, addresses are looked up the host's way (so `/etc/hosts`, VPN split
//!   DNS and mDNS work as on the host), and other record types are asked of the host's DNS servers.

mod dns;
mod host;
mod loop_host;
mod socket;
mod stream;
mod sys;
mod watch;

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

pub use host::{Host, HostDatagram, HostStream};
pub use loop_host::{Loop, Peer};
pub use socket::{Received, Socket, SocketInfo, TcpState};
pub use sys::System;
pub use watch::Watch;

use socket::Inner;

/// Told when something may have changed. It mustn't call back into `net` while it's told.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// What a kernel attaches to a Unix socket's message (`SCM_RIGHTS`' files), handed over with its bytes untouched.
pub type Ancillary = Box<dyn std::any::Any + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Unix,
    Inet,
    Inet6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Stream,
    Datagram,
    /// A connection that keeps each message's bounds (Unix only).
    SeqPacket,
}

/// A Unix socket's name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnixName {
    Unnamed,
    /// A path, as the program gave it.
    Path(Vec<u8>),
    /// Linux's abstract namespace: a name with no file.
    Abstract(Vec<u8>),
}

/// A Unix socket's address: its name, and for a path, what identifies its file (the kernel's inode number), so that
/// two spellings of one path meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixAddr {
    pub name: UnixName,
    pub file: Option<u64>,
}

impl UnixAddr {
    pub const UNNAMED: UnixAddr = UnixAddr { name: UnixName::Unnamed, file: None };

    fn key(&self) -> Option<UnixKey> {
        match (&self.name, self.file) {
            (UnixName::Unnamed, _) => None,
            (UnixName::Path(_), Some(file)) => Some(UnixKey::File(file)),
            (UnixName::Path(path), None) => Some(UnixKey::Path(path.clone())),
            (UnixName::Abstract(name), _) => Some(UnixKey::Abstract(name.clone())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum UnixKey {
    File(u64),
    Path(Vec<u8>),
    Abstract(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Addr {
    Inet(SocketAddr),
    Unix(UnixAddr),
}

/// Who made a socket, as `SO_PEERCRED` and `SCM_CREDENTIALS` tell the other end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    WouldBlock,
    InProgress,
    /// A connect is under way already.
    Already,
    IsConnected,
    NotConnected,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    BrokenPipe,
    AddrInUse,
    AddrNotAvailable,
    NetworkUnreachable,
    HostUnreachable,
    TimedOut,
    /// The machine's policy doesn't allow it.
    Denied,
    Invalid,
    NotSupported,
    /// An address of a family this socket isn't.
    WrongFamily,
    MessageTooBig,
    DestinationRequired,
    /// A name DNS doesn't know.
    NotFound,
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Other(what) => f.write_str(what),
            other => write!(f, "{other:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        use std::io::ErrorKind as K;
        match e.kind() {
            K::WouldBlock => Error::WouldBlock,
            K::ConnectionRefused => Error::ConnectionRefused,
            K::ConnectionReset => Error::ConnectionReset,
            K::ConnectionAborted => Error::ConnectionAborted,
            K::BrokenPipe => Error::BrokenPipe,
            K::AddrInUse => Error::AddrInUse,
            K::AddrNotAvailable => Error::AddrNotAvailable,
            K::NetworkUnreachable => Error::NetworkUnreachable,
            K::HostUnreachable => Error::HostUnreachable,
            K::TimedOut => Error::TimedOut,
            K::NotConnected => Error::NotConnected,
            K::InvalidInput => Error::Invalid,
            K::PermissionDenied => Error::Denied,
            K::NotFound => Error::NotFound,
            _ => Error::Other(e.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// What's ready on a socket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ready {
    /// A read wouldn't wait (there's data, the end of it, or an error).
    pub read: bool,
    pub write: bool,
    /// The other end won't send again (`POLLRDHUP`).
    pub read_closed: bool,
    /// Neither way is open (`POLLHUP`).
    pub hangup: bool,
    /// An error is waiting (`POLLERR`).
    pub error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

/// How a machine's network is set up.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// What the machine calls itself, and the name its DNS answers with its address.
    pub hostname: String,
    /// Where the resolver asks what it doesn't answer itself, instead of the host's resolver (`--dns`).
    pub dns: Option<SocketAddr>,
}

/// A network interface, as programs see it.
#[derive(Debug, Clone)]
pub struct Interface {
    pub name: &'static str,
    pub index: u32,
    pub loopback: bool,
    pub mac: [u8; 6],
    pub mtu: u32,
    /// Addresses, each with its prefix length.
    pub addrs: Vec<(IpAddr, u8)>,
}

/// An IPv4 route, as programs see it.
#[derive(Debug, Clone)]
pub struct Route {
    pub dst: Ipv4Addr,
    pub prefix: u8,
    pub gateway: Option<Ipv4Addr>,
    pub index: u32,
    pub source: Option<Ipv4Addr>,
}

/// One machine's network.
#[derive(Clone)]
pub struct Network(pub(crate) Arc<Shared>);

impl fmt::Debug for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Network({}, {:?})", self.0.address, self.0.config.hostname)
    }
}

/// A bound address: whose, and how it may be shared.
pub(crate) struct Binding {
    pub socket: Weak<Inner>,
    pub ip: IpAddr,
    pub reuse_addr: bool,
    pub reuse_port: bool,
}

pub(crate) struct Shared {
    pub config: Config,
    pub host: Arc<dyn Host>,
    pub address: Ipv4Addr,
    pub gateway: Ipv4Addr,
    /// TCP's (true) and UDP's ports, with who has each.
    pub ports: Mutex<HashMap<(bool, u16), Vec<Binding>>>,
    pub unix: Mutex<HashMap<UnixKey, Weak<Inner>>>,
    /// Every socket, for `/proc/net`.
    pub all: Mutex<Vec<Weak<Inner>>>,
    pub next_id: AtomicU64,
    next_port: AtomicU16,
}

/// Where an address is, from inside the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// The machine itself: loopback, or its own address.
    Local,
    /// The gateway, which is `net`.
    Gateway,
    /// Another address on the machine's network, where nothing is yet.
    Subnet,
    /// Out through the host.
    Out,
}

const FIRST_EPHEMERAL: u16 = 32768;
const LAST_EPHEMERAL: u16 = 60999;

/// An IPv4 address an IPv6 one maps (`::ffff:a.b.c.d`), or the IPv4 address itself.
pub(crate) fn v4_of(ip: IpAddr) -> Option<Ipv4Addr> {
    match ip {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(v6) => v6.to_ipv4_mapped(),
    }
}

/// `ip` as an address of `domain`: an IPv4 one mapped into IPv6 for an IPv6 socket.
pub(crate) fn as_domain(domain: Domain, addr: SocketAddr) -> SocketAddr {
    match (domain, addr) {
        (Domain::Inet6, SocketAddr::V4(v4)) => SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port()),
        _ => addr,
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Network {
    /// A network on `host`. Its subnet is the first of `10.89.0.0/16`, `10.90.0.0/16`, … that none of the host's
    /// addresses is on.
    pub fn new(config: Config, host: Arc<dyn Host>) -> Network {
        let taken: Vec<Ipv4Addr> = host.addresses().into_iter().filter_map(v4_of).collect();
        let second = (89..=254u8).find(|n| !taken.iter().any(|a| a.octets()[0] == 10 && a.octets()[1] == *n)).unwrap_or(89);
        Network(Arc::new(Shared {
            config,
            host,
            address: Ipv4Addr::new(10, second, 0, 2),
            gateway: Ipv4Addr::new(10, second, 0, 1),
            ports: Mutex::new(HashMap::new()),
            unix: Mutex::new(HashMap::new()),
            all: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            next_port: AtomicU16::new(FIRST_EPHEMERAL),
        }))
    }

    /// A network on the host's own sockets.
    pub fn on_this_host(config: Config) -> Network {
        Network::new(config, Arc::new(System))
    }

    pub fn config(&self) -> &Config {
        &self.0.config
    }

    /// The machine's own address.
    pub fn address(&self) -> Ipv4Addr {
        self.0.address
    }

    /// A name's addresses, as the machine's resolver gives them: its own name and `localhost` it answers itself, and
    /// any other is looked up the host's way. It waits for the answer: kernels call it from a thread that may wait
    /// (atlas's `getaddrinfo`, which programs call to wait). [`Error::NotFound`] for a name that isn't there. A `--dns`
    /// server isn't asked here yet: the host's lookup is.
    pub fn lookup(&self, name: &str) -> Result<Vec<IpAddr>> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let hostname = self.0.config.hostname.to_ascii_lowercase();
        if !hostname.is_empty() && name == hostname {
            return Ok(vec![IpAddr::V4(self.0.address)]);
        }
        if name == "localhost" {
            return Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)]);
        }
        if name.is_empty() {
            return Err(Error::NotFound);
        }
        self.0.host.lookup(&name)
    }

    /// Its gateway and DNS server.
    pub fn gateway(&self) -> Ipv4Addr {
        self.0.gateway
    }

    pub fn socket(&self, domain: Domain, kind: Kind) -> Result<Socket> {
        if kind == Kind::SeqPacket && domain != Domain::Unix {
            return Err(Error::NotSupported);
        }
        Ok(Socket::new(&self.0, domain, kind))
    }

    /// `socketpair`: two Unix sockets, joined.
    pub fn pair(&self, kind: Kind) -> Result<(Socket, Socket)> {
        Socket::pair(&self.0, kind)
    }

    /// What programs see: `lo` and `eth0`.
    pub fn interfaces(&self) -> Vec<Interface> {
        let [_, b, c, d] = self.0.address.octets();
        vec![
            Interface {
                name: "lo",
                index: 1,
                loopback: true,
                mac: [0; 6],
                mtu: 65536,
                addrs: vec![(IpAddr::V4(Ipv4Addr::LOCALHOST), 8), (IpAddr::V6(Ipv6Addr::LOCALHOST), 128)],
            },
            Interface { name: "eth0", index: 2, loopback: false, mac: [0x02, 0x42, 10, b, c, d], mtu: 1500, addrs: vec![(IpAddr::V4(self.0.address), 16)] },
        ]
    }

    /// The IPv4 routes: the machine's network, and the default route through its gateway.
    pub fn routes(&self) -> Vec<Route> {
        let [a, b, ..] = self.0.address.octets();
        vec![
            Route { dst: Ipv4Addr::UNSPECIFIED, prefix: 0, gateway: Some(self.0.gateway), index: 2, source: None },
            Route { dst: Ipv4Addr::new(a, b, 0, 0), prefix: 16, gateway: None, index: 2, source: Some(self.0.address) },
        ]
    }

    /// Every socket there is, for `/proc/net`.
    pub fn sockets(&self) -> Vec<SocketInfo> {
        let live: Vec<Arc<Inner>> = {
            let mut all = lock(&self.0.all);
            all.retain(|s| s.strong_count() > 0);
            all.iter().filter_map(Weak::upgrade).collect()
        };
        live.iter().map(|s| s.info()).collect()
    }
}

impl Shared {
    pub(crate) fn place(&self, ip: IpAddr) -> Place {
        if ip.is_loopback() || ip.is_unspecified() {
            return Place::Local;
        }
        let Some(v4) = v4_of(ip) else { return Place::Out };
        if v4.is_loopback() || v4.is_unspecified() || v4 == self.address {
            Place::Local
        } else if v4 == self.gateway {
            Place::Gateway
        } else if v4.octets()[..2] == self.address.octets()[..2] {
            Place::Subnet
        } else {
            Place::Out
        }
    }

    /// Whether a socket in the machine may bind `ip`: any address, loopback, or the machine's own.
    pub(crate) fn bindable(&self, ip: IpAddr) -> bool {
        self.place(ip) == Place::Local
    }

    /// Takes `ip`'s `port` (or a free one, if it's 0) for `socket`, for TCP or UDP. The port taken.
    pub(crate) fn take_port(&self, tcp: bool, ip: IpAddr, port: u16, socket: &Arc<Inner>, reuse_addr: bool, reuse_port: bool) -> Result<u16> {
        let mut ports = lock(&self.ports);
        let ip = v4_of(ip).map_or(ip, IpAddr::V4);
        let clashes = |ports: &HashMap<(bool, u16), Vec<Binding>>, port: u16| {
            ports.get(&(tcp, port)).is_some_and(|bound| {
                bound.iter().any(|b| {
                    let Some(other) = b.socket.upgrade() else { return false };
                    let overlap = b.ip == ip || b.ip.is_unspecified() || ip.is_unspecified();
                    let shared = (reuse_port && b.reuse_port) || (reuse_addr && b.reuse_addr && !(tcp && other.is_listening()));
                    overlap && !shared
                })
            })
        };
        let port = if port != 0 {
            if clashes(&ports, port) {
                return Err(Error::AddrInUse);
            }
            port
        } else {
            let span = LAST_EPHEMERAL - FIRST_EPHEMERAL + 1;
            let mut found = None;
            for _ in 0..span {
                let n = self.next_port.fetch_add(1, Ordering::Relaxed);
                let candidate = FIRST_EPHEMERAL + n % span;
                if !clashes(&ports, candidate) && !ports.get(&(tcp, candidate)).is_some_and(|b| b.iter().any(|b| b.socket.strong_count() > 0)) {
                    found = Some(candidate);
                    break;
                }
            }
            found.ok_or(Error::AddrInUse)?
        };
        let entry = ports.entry((tcp, port)).or_default();
        entry.retain(|b| b.socket.strong_count() > 0);
        entry.push(Binding { socket: Arc::downgrade(socket), ip, reuse_addr, reuse_port });
        Ok(port)
    }

    /// The live sockets bound to `port` that `ip` reaches.
    pub(crate) fn bound(&self, tcp: bool, ip: IpAddr, port: u16) -> Vec<Arc<Inner>> {
        let ip = v4_of(ip).map_or(ip, IpAddr::V4);
        let ports = lock(&self.ports);
        let Some(bound) = ports.get(&(tcp, port)) else { return Vec::new() };
        // An exact address first, then any.
        let mut exact: Vec<Arc<Inner>> = bound.iter().filter(|b| b.ip == ip).filter_map(|b| b.socket.upgrade()).collect();
        exact.extend(bound.iter().filter(|b| b.ip != ip && b.ip.is_unspecified()).filter_map(|b| b.socket.upgrade()));
        exact
    }
}

#[cfg(test)]
mod tests;
