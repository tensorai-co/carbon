//! net's conformance suite: the same checks on every host, through a machine's network on the host's own sockets.
//! A machine reaches the host the way it reaches anywhere out, so the peers here listen on one of the host's own
//! (non-loopback) addresses, which the machine sees as just another address out.
//!
//! ```sh
//! cargo run -p net --example net-conformance
//! ```

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use net::{Addr, Config, Domain, Error, Kind, Network, Shutdown, Socket, System};

static FAILURES: AtomicUsize = AtomicUsize::new(0);

fn check(ok: bool, what: &str) {
    println!("{}{what}", if ok { "ok      " } else { "FAILED  " });
    if !ok {
        FAILURES.fetch_add(1, Ordering::SeqCst);
    }
}

/// Waits up to 10 s for `what`.
fn until(mut what: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// Reads to the end, waiting as a program would.
fn read_to_end(s: &Socket) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    loop {
        if !until(|| s.ready().read) {
            return Err(Error::TimedOut);
        }
        match s.recv(1 << 16, false) {
            Ok(got) if got.data.is_empty() => return Ok(out),
            Ok(got) => out.extend(got.data),
            Err(Error::WouldBlock) => {}
            Err(e) => return Err(e),
        }
    }
}

/// One of the host's own addresses that isn't loopback, for the peers to listen on.
fn host_address() -> Option<Ipv4Addr> {
    // The address the host would use to go out; nothing is sent.
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    probe.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match probe.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_unspecified() => Some(v4),
        _ => None,
    }
}

fn connected(net: &Network, to: SocketAddr) -> Result<Socket, Error> {
    let s = net.socket(Domain::Inet, Kind::Stream)?;
    match s.connect(&Addr::Inet(to)) {
        Ok(()) => return Ok(s),
        Err(Error::InProgress) => {}
        Err(e) => return Err(e),
    }
    if !until(|| s.ready().write || s.ready().error) {
        return Err(Error::TimedOut);
    }
    match s.take_error() {
        Some(e) => Err(e),
        None => Ok(s),
    }
}

/// A DNS server on the host's loopback that answers every question with one TXT record.
fn txt_server() -> SocketAddr {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap_or_else(|e| panic!("a test DNS server: {e}"));
    let addr = server.local_addr().unwrap_or_else(|e| panic!("{e}"));
    std::thread::spawn(move || {
        let mut buffer = [0u8; 512];
        while let Ok((n, from)) = server.recv_from(&mut buffer) {
            let mut reply = buffer[..n].to_vec();
            reply[2] |= 0x80;
            reply[3] = 0x80;
            reply[7] = 1;
            // An answer to the question's name: TXT, IN, a TTL, "carbon".
            reply.extend([0xC0, 0x0C, 0, 16, 0, 1, 0, 0, 0, 60, 0, 7, 6]);
            reply.extend(b"carbon");
            let _ = server.send_to(&reply, from);
        }
    });
    addr
}

fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend([0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend(label.as_bytes());
    }
    q.extend([0, (qtype >> 8) as u8, qtype as u8, 0, 1]);
    q
}

fn main() {
    let Some(host_ip) = host_address() else {
        println!("FAILED  the host has no address other than loopback to test with");
        std::process::exit(1);
    };
    println!("[net conformance, peers on {host_ip}]");
    let net = Network::new(Config { hostname: "conformance".into(), dns: None }, Arc::new(System));
    check(!net.address().octets()[..2].eq(&host_ip.octets()[..2]) || host_ip.octets()[0] != 10, "the machine's network is on none of the host's addresses");

    // TCP: connect, accept, echo; the machine's own address as the local one.
    let listener = TcpListener::bind((host_ip, 0)).unwrap_or_else(|e| panic!("a listener: {e}"));
    let to = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    let echo = std::thread::spawn(move || {
        let (mut c, _) = listener.accept().ok()?;
        let mut got = Vec::new();
        c.read_to_end(&mut got).ok()?;
        c.write_all(&got).ok()?;
        Some((got, listener))
    });
    let s = connected(&net, to);
    check(s.is_ok(), "tcp: connected out through the host");
    let Ok(s) = s else { std::process::exit(1) };
    let Addr::Inet(local) = s.local() else { unreachable!() };
    check(local.ip() == IpAddr::V4(net.address()) && s.peer() == Ok(Addr::Inet(to)), "tcp: the machine's address is its own; the peer is where it connected");
    let woken = Arc::new(AtomicUsize::new(0));
    let w = woken.clone();
    let _watch = s.watch(Arc::new(move || {
        w.fetch_add(1, Ordering::SeqCst);
    }));
    check(s.send(b"there and back", None, &mut None) == Ok(14), "tcp: sent");
    // Half-close: the peer reads to the end, then answers.
    check(s.shutdown(Shutdown::Write).is_ok(), "tcp: half-closed");
    let back = read_to_end(&s);
    check(back.as_deref() == Ok(&b"there and back"[..]) && woken.load(Ordering::SeqCst) > 0, "tcp: the echo came back after the half-close, and the socket's watchers were told");
    let listener = echo.join().ok().flatten().map(|(got, l)| {
        check(got == b"there and back", "tcp: the peer read it all");
        l
    });
    drop(s);

    // A reset: the peer closes with what the machine sent unread.
    if let Some(listener) = listener {
        let peer = std::thread::spawn(move || {
            let (c, _) = listener.accept().ok()?;
            std::thread::sleep(Duration::from_millis(200));
            drop(c);
            Some(())
        });
        let s = connected(&net, to);
        if let Ok(s) = s {
            let _ = s.send(b"never read", None, &mut None);
            let _ = peer.join();
            let mut result = Ok(Vec::new());
            until(|| {
                result = match s.recv(64, false) {
                    Err(Error::WouldBlock) => return false,
                    other => other.map(|g| g.data),
                };
                true
            });
            check(result == Err(Error::ConnectionReset), "tcp: a peer that closes with data unread resets the connection");
        } else {
            check(false, "tcp: connected again");
        }
    }

    // Refused: a port nothing listens on.
    let closed = TcpListener::bind((host_ip, 0)).and_then(|l| l.local_addr()).unwrap_or_else(|e| panic!("{e}"));
    let s = net.socket(Domain::Inet, Kind::Stream).unwrap_or_else(|e| panic!("{e}"));
    let first = s.connect(&Addr::Inet(closed));
    check(first == Err(Error::InProgress) || first == Err(Error::ConnectionRefused), "tcp: a connect out starts without waiting");
    check(until(|| s.ready().error) && s.take_error() == Some(Error::ConnectionRefused) || first == Err(Error::ConnectionRefused), "tcp: refused, as SO_ERROR says");

    // UDP both ways.
    let peer = UdpSocket::bind((host_ip, 0)).unwrap_or_else(|e| panic!("{e}"));
    let peer_addr = peer.local_addr().unwrap_or_else(|e| panic!("{e}"));
    let u = net.socket(Domain::Inet, Kind::Datagram).unwrap_or_else(|e| panic!("{e}"));
    check(u.send(b"ping", Some(&Addr::Inet(peer_addr)), &mut None) == Ok(4), "udp: sent out");
    let _ = peer.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = [0u8; 64];
    let got = peer.recv_from(&mut buffer);
    check(got.as_ref().is_ok_and(|(n, _)| &buffer[..*n] == b"ping"), "udp: the peer got it");
    if let Ok((_, from)) = got {
        let _ = peer.send_to(b"pong", from);
        check(until(|| u.ready().read), "udp: the answer made the socket readable");
        let back = u.recv(64, false);
        check(back.is_ok_and(|g| g.data == b"pong" && g.from == Some(Addr::Inet(peer_addr))), "udp: the answer, from the peer");
    }

    // DNS: an address the host's way, and another type from a server of the machine's own (`--dns`).
    let gateway = Addr::Inet(SocketAddr::new(IpAddr::V4(net.gateway()), 53));
    let ask = |net: &Network, q: Vec<u8>| -> Option<Vec<u8>> {
        let d = net.socket(Domain::Inet, Kind::Datagram).ok()?;
        d.send(&q, Some(&gateway), &mut None).ok()?;
        until(|| d.ready().read).then(|| d.recv(512, false).ok().map(|g| g.data)).flatten()
    };
    let a = ask(&net, query(1, "localhost", 1));
    check(a.is_some_and(|a| a[7] >= 1 && a[a.len() - 4..] == [127, 0, 0, 1]), "dns: localhost's address");
    let own = ask(&net, query(2, "conformance", 1));
    check(own.is_some_and(|a| a[a.len() - 4..] == net.address().octets()), "dns: the machine's own name");
    let with_server = Network::new(Config { hostname: "conformance".into(), dns: Some(txt_server()) }, Arc::new(System));
    let txt = ask(&with_server, query(3, "example.test", 16));
    check(txt.is_some_and(|t| t.ends_with(b"\x06carbon") && t[..2] == [0, 3]), "dns: a TXT record from the machine's own DNS server");

    let failures = FAILURES.load(Ordering::SeqCst);
    println!("{}", if failures == 0 { "all passed" } else { "some checks failed" });
    std::process::exit(i32::from(failures != 0));
}
