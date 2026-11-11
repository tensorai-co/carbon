//! The real host: `System`, on the host's own sockets, the same on macOS, Linux and Windows. Its sockets are std's,
//! non-blocking, with one reactor thread for the whole process waiting on all of them with `poll` (`WSAPoll` on
//! Windows). A socket's readiness is edge-like: the reactor marks a socket readable or writable once and stops
//! watching that way, until an operation finds it would block and watches again. A connection is made on a thread of
//! its own, as std's `connect` waits.
//!
//! What differs by host is small: `poll` itself, the host's addresses, and where its DNS servers are.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown as StdShutdown, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use crate::{Error, Host, HostDatagram, HostStream, Ready, Result, Shutdown, Waker, lock};

/// How long a connection out may take to be made.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the host's DNS servers have to answer.
const QUERY_TIMEOUT: Duration = Duration::from_secs(4);

const READ: u8 = 1;
const WRITE: u8 = 2;

/// The host's own sockets.
pub struct System;

/// A host socket, as the reactor waits on it.
pub(crate) type Raw = u64;

/// One socket the reactor waits on, and what it's found.
struct Registration {
    raw: Raw,
    /// The ways it's watched for now.
    armed: AtomicU8,
    /// The ways it's been found ready since an operation last found it wasn't.
    ready: AtomicU8,
    wake: Waker,
}

impl Registration {
    fn new(raw: Raw, armed: u8, ready: u8, wake: Waker) -> Arc<Registration> {
        let registration = Arc::new(Registration { raw, armed: AtomicU8::new(armed), ready: AtomicU8::new(ready), wake });
        let reactor = reactor();
        lock(&reactor.list).push(Arc::downgrade(&registration));
        reactor.nudge();
        registration
    }

    fn is(&self, way: u8) -> bool {
        self.ready.load(Ordering::Acquire) & way != 0
    }

    /// An operation found `way` would block: watch for it again.
    fn rearm(&self, way: u8) {
        self.ready.fetch_and(!way, Ordering::AcqRel);
        self.armed.fetch_or(way, Ordering::AcqRel);
        reactor().nudge();
    }

    /// What an operation got: a would-block watches that way again.
    fn after<T>(&self, way: u8, result: std::io::Result<T>) -> Result<T> {
        match result {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.rearm(way);
                Err(Error::WouldBlock)
            }
            Err(e) => Err(e.into()),
        }
    }
}

struct Reactor {
    list: Mutex<Vec<Weak<Registration>>>,
    /// Sending to this wakes the reactor from its `poll`, to wait on a new set.
    nudge: UdpSocket,
}

impl Reactor {
    fn nudge(&self) {
        let _ = self.nudge.send(&[0]);
    }
}

/// The process's reactor, started the first time it's needed.
fn reactor() -> &'static Reactor {
    static REACTOR: OnceLock<Reactor> = OnceLock::new();
    REACTOR.get_or_init(|| {
        let inbox = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap_or_else(|e| panic!("net: the reactor's socket: {e}"));
        let nudge = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap_or_else(|e| panic!("net: the reactor's socket: {e}"));
        let _ = inbox.connect(nudge.local_addr().unwrap_or_else(|e| panic!("net: {e}")));
        let _ = nudge.connect(inbox.local_addr().unwrap_or_else(|e| panic!("net: {e}")));
        let _ = inbox.set_nonblocking(true);
        let _ = nudge.set_nonblocking(true);
        let _ = std::thread::Builder::new().name("net reactor".into()).spawn(move || run(inbox));
        Reactor { list: Mutex::new(Vec::new()), nudge }
    })
}

fn run(inbox: UdpSocket) {
    let inbox_raw = os::raw_udp(&inbox);
    loop {
        let watched: Vec<Arc<Registration>> = {
            let reactor = reactor();
            let mut list = lock(&reactor.list);
            list.retain(|r| r.strong_count() > 0);
            list.iter().filter_map(Weak::upgrade).filter(|r| r.armed.load(Ordering::Acquire) != 0).collect()
        };
        let mut entries: Vec<(Raw, u8)> = Vec::with_capacity(watched.len() + 1);
        entries.push((inbox_raw, READ));
        entries.extend(watched.iter().map(|r| (r.raw, r.armed.load(Ordering::Acquire))));
        let found = os::poll(&entries);
        let mut drain = [0u8; 64];
        while inbox.recv(&mut drain).is_ok() {}
        for (registration, &got) in watched.iter().zip(&found[1..]) {
            let fired = got & registration.armed.fetch_and(!got, Ordering::AcqRel);
            if fired != 0 {
                registration.ready.fetch_or(fired, Ordering::AcqRel);
                (registration.wake)();
            }
        }
    }
}

enum Connection {
    Connecting,
    Ready(TcpStream, Arc<Registration>),
    Failed(Error),
}

struct TcpOut {
    state: Arc<Mutex<Connection>>,
    options: Mutex<(bool, bool)>,
}

impl TcpOut {
    fn start(to: SocketAddr, wake: Waker) -> TcpOut {
        let state = Arc::new(Mutex::new(Connection::Connecting));
        let weak = Arc::downgrade(&state);
        let out = TcpOut { state, options: Mutex::new((false, false)) };
        let _ = std::thread::Builder::new().name("net connect".into()).spawn(move || {
            let result = TcpStream::connect_timeout(&to, CONNECT_TIMEOUT).and_then(|s| s.set_nonblocking(true).map(|()| s));
            // Nobody's waiting for it any more: it closes here.
            let Some(state) = weak.upgrade() else { return };
            *lock(&state) = match result {
                Ok(stream) => {
                    let raw = os::raw_tcp(&stream);
                    Connection::Ready(stream, Registration::new(raw, READ, WRITE, wake.clone()))
                }
                Err(e) => Connection::Failed(e.into()),
            };
            wake();
        });
        out
    }

    fn with<T>(&self, f: impl FnOnce(&TcpStream, &Registration) -> Result<T>) -> Result<T> {
        match &*lock(&self.state) {
            Connection::Ready(stream, registration) => f(stream, registration),
            Connection::Connecting => Err(Error::WouldBlock),
            Connection::Failed(e) => Err(e.clone()),
        }
    }
}

impl HostStream for TcpOut {
    fn connected(&self) -> Result<bool> {
        match &*lock(&self.state) {
            Connection::Connecting => Ok(false),
            Connection::Ready(stream, _) => {
                let (nodelay, keepalive) = *lock(&self.options);
                let _ = stream.set_nodelay(nodelay);
                os::set_keepalive(stream, keepalive);
                Ok(true)
            }
            Connection::Failed(e) => Err(e.clone()),
        }
    }

    fn ready(&self) -> Ready {
        match &*lock(&self.state) {
            Connection::Connecting => Ready::default(),
            Connection::Ready(_, r) => Ready { read: r.is(READ), write: r.is(WRITE), ..Ready::default() },
            Connection::Failed(_) => Ready { read: true, write: true, read_closed: true, hangup: true, error: true },
        }
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize> {
        self.with(|stream, r| r.after(READ, (&*stream).read(buffer)))
    }

    fn write(&self, data: &[u8]) -> Result<usize> {
        self.with(|stream, r| r.after(WRITE, (&*stream).write(data)))
    }

    fn shutdown(&self, how: Shutdown) {
        let _ = self.with(|stream, r| {
            let _ = stream.shutdown(match how {
                Shutdown::Read => StdShutdown::Read,
                Shutdown::Write => StdShutdown::Write,
                Shutdown::Both => StdShutdown::Both,
            });
            // What's ready may have changed: look again.
            r.rearm(READ);
            Ok(())
        });
    }

    fn set_nodelay(&self, on: bool) {
        lock(&self.options).0 = on;
        let _ = self.with(|stream, _| Ok(stream.set_nodelay(on)));
    }

    fn set_keepalive(&self, on: bool) {
        lock(&self.options).1 = on;
        let _ = self.with(|stream, _| {
            os::set_keepalive(stream, on);
            Ok(())
        });
    }
}

struct UdpOut {
    socket: UdpSocket,
    registration: Arc<Registration>,
}

impl HostDatagram for UdpOut {
    fn ready(&self) -> Ready {
        Ready { read: self.registration.is(READ), write: true, ..Ready::default() }
    }

    fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize> {
        self.registration.after(WRITE, self.socket.send_to(data, to))
    }

    fn recv_from(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        loop {
            match self.socket.recv_from(buffer) {
                // Windows reports an ICMP port-unreachable from an earlier send this way: not this datagram's.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
                other => return self.registration.after(READ, other),
            }
        }
    }
}

impl Host for System {
    fn connect(&self, to: SocketAddr, wake: Waker) -> Box<dyn HostStream> {
        Box::new(TcpOut::start(to, wake))
    }

    fn udp(&self, v6: bool, wake: Waker) -> Result<Box<dyn HostDatagram>> {
        let any: IpAddr = if v6 { Ipv6Addr::UNSPECIFIED.into() } else { Ipv4Addr::UNSPECIFIED.into() };
        let socket = UdpSocket::bind((any, 0))?;
        socket.set_nonblocking(true)?;
        let registration = Registration::new(os::raw_udp(&socket), READ, WRITE, wake);
        Ok(Box::new(UdpOut { socket, registration }))
    }

    fn lookup(&self, name: &str) -> Result<Vec<IpAddr>> {
        let found = (name, 0).to_socket_addrs().map_err(|_| Error::NotFound)?;
        let mut addrs: Vec<IpAddr> = Vec::new();
        for addr in found {
            if !addrs.contains(&addr.ip()) {
                addrs.push(addr.ip());
            }
        }
        Ok(addrs)
    }

    fn query(&self, packet: &[u8], server: Option<SocketAddr>) -> Result<Vec<u8>> {
        let servers = match server {
            Some(server) => vec![server],
            None => os::dns_servers(),
        };
        for server in servers {
            let any: IpAddr = if server.is_ipv6() { Ipv6Addr::UNSPECIFIED.into() } else { Ipv4Addr::UNSPECIFIED.into() };
            let Ok(socket) = UdpSocket::bind((any, 0)) else { continue };
            let _ = socket.set_read_timeout(Some(QUERY_TIMEOUT));
            if socket.connect(server).and_then(|()| socket.send(packet)).is_err() {
                continue;
            }
            let mut buffer = vec![0u8; 65536];
            while let Ok(n) = socket.recv(&mut buffer) {
                if n >= 12 && buffer[..2] == packet[..2] {
                    buffer.truncate(n);
                    return Ok(buffer);
                }
            }
        }
        Err(Error::TimedOut)
    }

    fn addresses(&self) -> Vec<IpAddr> {
        os::addresses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn counter() -> (Arc<AtomicUsize>, Waker) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        (count, Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }))
    }

    fn until(mut what: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn a_connection_out_is_made_and_woken() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let (woken, wake) = counter();
        let out = System.connect(listener.local_addr().unwrap(), wake);
        assert!(until(|| out.connected() == Ok(true)));
        let (mut server, _) = listener.accept().unwrap();
        assert!(out.ready().write);
        assert_eq!(out.read(&mut [0; 8]), Err(Error::WouldBlock));
        let before = woken.load(Ordering::SeqCst);
        server.write_all(b"hello").unwrap();
        assert!(until(|| woken.load(Ordering::SeqCst) > before && out.ready().read));
        let mut buffer = [0; 8];
        assert_eq!(out.read(&mut buffer), Ok(5));
        assert_eq!(&buffer[..5], b"hello");
        assert_eq!(out.write(b"back"), Ok(4));
        let mut got = [0; 4];
        server.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"back");
        drop(server);
        assert!(until(|| out.read(&mut buffer) == Ok(0)));
    }

    #[test]
    fn a_refused_connection_says_so() {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        let (_, wake) = counter();
        let out = System.connect(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port), wake);
        assert!(until(|| out.connected().is_err()));
        assert_eq!(out.connected(), Err(Error::ConnectionRefused));
        assert!(out.ready().error);
    }

    #[test]
    fn udp_goes_both_ways() {
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let (_, wake) = counter();
        let mine = System.udp(false, wake).unwrap();
        assert_eq!(mine.send_to(b"ping", peer.local_addr().unwrap()), Ok(4));
        let mut buffer = [0; 16];
        let (n, from) = peer.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..n], b"ping");
        peer.send_to(b"pong", (Ipv4Addr::LOCALHOST, from.port())).unwrap();
        assert!(until(|| mine.ready().read));
        let (n, from) = mine.recv_from(&mut buffer).unwrap();
        assert_eq!((&buffer[..n], from), (&b"pong"[..], peer.local_addr().unwrap()));
    }

    #[test]
    fn localhost_is_found() {
        assert!(System.lookup("localhost").unwrap().iter().any(|ip| ip.is_loopback()));
        assert_eq!(System.lookup("no-such-name.invalid"), Err(Error::NotFound));
        assert!(System.addresses().iter().any(|ip| ip.is_loopback()));
    }
}
