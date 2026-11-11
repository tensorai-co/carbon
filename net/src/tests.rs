//! `Network` on the `Loop` host: everything inside the machine, and out to scripted peers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;

fn network() -> (Network, Arc<Loop>) {
    let host = Loop::new();
    (Network::new(Config { hostname: "box".into(), dns: None }, Arc::new(host.clone())), host)
}

fn inet(s: &str) -> Addr {
    Addr::Inet(s.parse().unwrap())
}

fn path(p: &str, file: u64) -> Addr {
    Addr::Unix(UnixAddr { name: UnixName::Path(p.as_bytes().to_vec()), file: Some(file) })
}

fn recv_all(s: &Socket) -> Vec<u8> {
    s.recv(1 << 20, false).unwrap().data
}

fn until(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn the_subnet_avoids_the_hosts() {
    let (net, _) = network();
    // The loop host is on 10.89/16.
    assert_eq!(net.address(), Ipv4Addr::new(10, 90, 0, 2));
    assert_eq!(net.gateway(), Ipv4Addr::new(10, 90, 0, 1));
    let names: Vec<&str> = net.interfaces().iter().map(|i| i.name).collect();
    assert_eq!(names, ["lo", "eth0"]);
    assert_eq!(net.routes()[0].gateway, Some(net.gateway()));
}

#[test]
fn tcp_over_loopback() {
    let (net, _) = network();
    let listener = net.socket(Domain::Inet, Kind::Stream).unwrap();
    listener.bind(&inet("127.0.0.1:8080")).unwrap();
    listener.listen(16).unwrap();
    assert!(!listener.ready().read);
    let woken = Arc::new(AtomicUsize::new(0));
    let w = woken.clone();
    let _watch = listener.watch(Arc::new(move || {
        w.fetch_add(1, Ordering::SeqCst);
    }));
    let client = net.socket(Domain::Inet, Kind::Stream).unwrap();
    client.connect(&inet("127.0.0.1:8080")).unwrap();
    assert!(woken.load(Ordering::SeqCst) > 0 && listener.ready().read);
    let (server, from) = listener.accept().unwrap();
    assert_eq!(from, client.local());
    assert_eq!(server.local(), inet("127.0.0.1:8080"));
    assert_eq!(client.peer().unwrap(), inet("127.0.0.1:8080"));
    let Addr::Inet(local) = client.local() else { panic!() };
    assert!(local.port() >= 32768 && local.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST));

    assert_eq!(client.send(b"hello", None, &mut None), Ok(5));
    assert!(server.ready().read);
    assert_eq!(server.available(), 5);
    assert_eq!(server.recv(2, true).unwrap().data, b"he");
    assert_eq!(recv_all(&server), b"hello");
    assert_eq!(server.recv(16, false).err(), Some(Error::WouldBlock));

    // Half-close: the client's end reads the server's last words, then the end.
    client.shutdown(Shutdown::Write).unwrap();
    assert_eq!(recv_all(&server), b"");
    assert!(server.ready().read_closed);
    server.send(b"bye", None, &mut None).unwrap();
    assert_eq!(recv_all(&client), b"bye");
    drop(server);
    assert_eq!(recv_all(&client), b"");
    assert!(client.ready().hangup);
}

#[test]
fn closing_with_data_unread_resets() {
    let (net, _) = network();
    let listener = net.socket(Domain::Inet, Kind::Stream).unwrap();
    listener.bind(&inet("0.0.0.0:9000")).unwrap();
    listener.listen(1).unwrap();
    let client = net.socket(Domain::Inet, Kind::Stream).unwrap();
    // The machine's own address reaches a listener on any address.
    client.connect(&Addr::Inet(SocketAddr::new(IpAddr::V4(net.address()), 9000))).unwrap();
    let (server, _) = listener.accept().unwrap();
    client.send(b"unread", None, &mut None).unwrap();
    drop(server);
    assert!(client.ready().error);
    assert_eq!(client.recv(16, false).err(), Some(Error::ConnectionReset));
    assert_eq!(recv_all(&client), b"");
}

#[test]
fn ports_are_the_machines() {
    let (net, _) = network();
    let a = net.socket(Domain::Inet, Kind::Stream).unwrap();
    a.bind(&inet("0.0.0.0:80")).unwrap();
    let b = net.socket(Domain::Inet, Kind::Stream).unwrap();
    assert_eq!(b.bind(&inet("127.0.0.1:80")), Err(Error::AddrInUse));
    assert_eq!(b.bind(&inet("8.8.8.8:80")), Err(Error::AddrNotAvailable));
    // UDP's ports are its own.
    let u = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    u.bind(&inet("0.0.0.0:80")).unwrap();
    // Nothing listens on 81; the gateway takes no TCP.
    let c = net.socket(Domain::Inet, Kind::Stream).unwrap();
    assert_eq!(c.connect(&inet("127.0.0.1:81")), Err(Error::ConnectionRefused));
    let gw = Addr::Inet(SocketAddr::new(IpAddr::V4(net.gateway()), 53));
    assert_eq!(c.connect(&gw), Err(Error::ConnectionRefused));
    // A port that's closed is free again; one with SO_REUSEADDR on both, too.
    drop(a);
    b.bind(&inet("127.0.0.1:80")).unwrap();
    let r1 = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    let r2 = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    for r in [&r1, &r2] {
        r.set_options(|o| o.reuse_addr = true);
    }
    r1.bind(&inet("0.0.0.0:5353")).unwrap();
    r2.bind(&inet("0.0.0.0:5353")).unwrap();
}

#[test]
fn ipv6_sockets_reach_ipv4_listeners_through_mapping() {
    let (net, _) = network();
    let listener = net.socket(Domain::Inet6, Kind::Stream).unwrap();
    listener.bind(&inet("[::]:7000")).unwrap();
    listener.listen(4).unwrap();
    let v4 = net.socket(Domain::Inet, Kind::Stream).unwrap();
    v4.connect(&inet("127.0.0.1:7000")).unwrap();
    let (_, from) = listener.accept().unwrap();
    let Addr::Inet(from) = from else { panic!() };
    assert_eq!(from.ip(), IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()));
    let v6 = net.socket(Domain::Inet6, Kind::Stream).unwrap();
    v6.connect(&inet("[::1]:7000")).unwrap();
    // An IPv4 socket can't reach ::1.
    let only4 = net.socket(Domain::Inet, Kind::Stream).unwrap();
    only4.bind(&inet("0.0.0.0:7001")).unwrap();
    only4.listen(1).unwrap();
    let v6b = net.socket(Domain::Inet6, Kind::Stream).unwrap();
    assert_eq!(v6b.connect(&Addr::Inet(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 7001))), Err(Error::ConnectionRefused));
}

#[test]
fn udp_inside_the_machine() {
    let (net, _) = network();
    let a = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    a.bind(&inet("127.0.0.1:5000")).unwrap();
    let b = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    assert_eq!(b.send(b"one", Some(&inet("127.0.0.1:5000")), &mut None), Ok(3));
    assert_eq!(b.send(b"two", Some(&inet("127.0.0.1:5000")), &mut None), Ok(3));
    let got = a.recv(2, false).unwrap();
    let Addr::Inet(b_local) = b.local() else { panic!() };
    // b is bound to any address; what it sent came from loopback.
    assert_eq!((got.data.as_slice(), got.full, got.from.clone()), (&b"on"[..], 3, Some(inet(&format!("127.0.0.1:{}", b_local.port())))));
    assert_eq!(recv_all(&a), b"two");
    // A connected socket takes only from its peer.
    let c = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    c.bind(&inet("127.0.0.1:5001")).unwrap();
    a.connect(&inet("127.0.0.1:5001")).unwrap();
    b.send(b"not you", Some(&inet("127.0.0.1:5000")), &mut None).unwrap();
    c.send(b"you", Some(&inet("127.0.0.1:5000")), &mut None).unwrap();
    assert_eq!(recv_all(&a), b"you");
    assert_eq!(a.recv(16, false).err(), Some(Error::WouldBlock));
}

#[test]
fn unix_streams_and_their_names() {
    let (net, _) = network();
    let listener = net.socket(Domain::Unix, Kind::Stream).unwrap();
    assert_eq!(listener.listen(1), Err(Error::Invalid));
    listener.bind(&path("/run/app.sock", 41)).unwrap();
    listener.listen(8).unwrap();
    let other = net.socket(Domain::Unix, Kind::Stream).unwrap();
    assert_eq!(other.bind(&path("/run/app.sock", 41)), Err(Error::AddrInUse));
    let client = net.socket(Domain::Unix, Kind::Stream).unwrap();
    client.set_cred(Cred { pid: 7, uid: 0, gid: 0 });
    listener.set_cred(Cred { pid: 3, uid: 0, gid: 0 });
    // Another spelling of the same file meets it.
    client.connect(&path("/run/../run/app.sock", 41)).unwrap();
    let (server, from) = listener.accept().unwrap();
    assert_eq!(from, Addr::Unix(UnixAddr::UNNAMED));
    assert_eq!(client.peer().unwrap(), path("/run/app.sock", 41));
    assert_eq!(server.peer_cred().unwrap().pid, 7);
    assert_eq!(client.peer_cred().unwrap().pid, 3);
    // What's attached comes with the first byte of its write, and splits the stream there.
    client.send(b"ab", None, &mut None).unwrap();
    client.send(b"cd", None, &mut Some(Box::new(42u32))).unwrap();
    let first = server.recv(16, false).unwrap();
    assert_eq!((first.data.as_slice(), first.extra.is_none()), (&b"ab"[..], true));
    let second = server.recv(16, false).unwrap();
    assert_eq!(second.data, b"cd");
    assert_eq!(*second.extra.unwrap().downcast::<u32>().unwrap(), 42);
    assert_eq!(second.cred.unwrap().pid, 7);
    // Nothing bound there: refused. An abstract name works the same.
    let lost = net.socket(Domain::Unix, Kind::Stream).unwrap();
    assert_eq!(lost.connect(&path("/run/none.sock", 99)), Err(Error::ConnectionRefused));
    let abstract_name = Addr::Unix(UnixAddr { name: UnixName::Abstract(b"bus".to_vec()), file: None });
    let l2 = net.socket(Domain::Unix, Kind::Stream).unwrap();
    l2.bind(&abstract_name).unwrap();
    l2.listen(1).unwrap();
    lost.connect(&abstract_name).unwrap();
    // Writing to an end that's gone is a broken pipe, not a reset.
    drop(server);
    assert_eq!(client.send(b"x", None, &mut None), Err(Error::BrokenPipe));
}

#[test]
fn seqpacket_keeps_messages() {
    let (net, _) = network();
    let (a, b) = net.pair(Kind::SeqPacket).unwrap();
    a.send(b"first message", None, &mut None).unwrap();
    a.send(b"second", None, &mut None).unwrap();
    let got = b.recv(5, false).unwrap();
    assert_eq!((got.data.as_slice(), got.full), (&b"first"[..], 13));
    assert_eq!(recv_all(&b), b"second");
}

#[test]
fn unix_datagrams() {
    let (net, _) = network();
    let (a, b) = net.pair(Kind::Datagram).unwrap();
    a.send(b"to b", None, &mut None).unwrap();
    assert_eq!(recv_all(&b), b"to b");
    let named = net.socket(Domain::Unix, Kind::Datagram).unwrap();
    named.bind(&path("/dev/log", 7)).unwrap();
    let sender = net.socket(Domain::Unix, Kind::Datagram).unwrap();
    sender.bind(&Addr::Unix(UnixAddr::UNNAMED)).unwrap();
    let Addr::Unix(UnixAddr { name: UnixName::Abstract(auto), .. }) = sender.local() else { panic!("autobind") };
    assert_eq!(auto.len(), 5);
    sender.send(b"log line", Some(&path("/dev/log", 7)), &mut None).unwrap();
    let got = named.recv(64, false).unwrap();
    assert_eq!((got.data.as_slice(), got.from), (&b"log line"[..], Some(sender.local())));
    assert_eq!(sender.send(b"x", Some(&path("/dev/nothing", 8)), &mut None), Err(Error::ConnectionRefused));
    assert_eq!(sender.send(b"x", None, &mut None), Err(Error::DestinationRequired));
}

#[test]
fn out_through_the_host() {
    let (net, host) = network();
    let echo: SocketAddr = "93.184.216.34:80".parse().unwrap();
    let refuse: SocketAddr = "93.184.216.34:81".parse().unwrap();
    let stall: SocketAddr = "93.184.216.34:82".parse().unwrap();
    host.peer(echo, Peer::Echo);
    host.peer(refuse, Peer::Refuse);
    host.peer(stall, Peer::Stall);

    let s = net.socket(Domain::Inet, Kind::Stream).unwrap();
    s.connect(&Addr::Inet(echo)).unwrap();
    let Addr::Inet(local) = s.local() else { panic!() };
    assert_eq!(local.ip(), IpAddr::V4(net.address()));
    assert_eq!(s.peer().unwrap(), Addr::Inet(echo));
    s.send(b"echo me", None, &mut None).unwrap();
    assert!(s.ready().read);
    assert_eq!(recv_all(&s), b"echo me");
    s.shutdown(Shutdown::Write).unwrap();
    assert_eq!(recv_all(&s), b"");

    let r = net.socket(Domain::Inet, Kind::Stream).unwrap();
    assert_eq!(r.connect(&Addr::Inet(refuse)), Err(Error::ConnectionRefused));
    let st = net.socket(Domain::Inet, Kind::Stream).unwrap();
    assert_eq!(st.connect(&Addr::Inet(stall)), Err(Error::InProgress));
    assert_eq!(st.connect(&Addr::Inet(stall)), Err(Error::Already));
    assert!(!st.ready().write);
    assert_eq!(st.send(b"x", None, &mut None), Err(Error::WouldBlock));
    // Another machine on the network: nothing's there.
    let other = Addr::Inet(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 90, 0, 9)), 80));
    assert_eq!(net.socket(Domain::Inet, Kind::Stream).unwrap().connect(&other), Err(Error::HostUnreachable));

    // UDP out, and back.
    let dns_peer: SocketAddr = "9.9.9.9:53".parse().unwrap();
    host.peer(dns_peer, Peer::Echo);
    let u = net.socket(Domain::Inet6, Kind::Datagram).unwrap();
    let mapped = SocketAddr::new(IpAddr::V6(Ipv4Addr::new(9, 9, 9, 9).to_ipv6_mapped()), 53);
    u.send(b"q", Some(&Addr::Inet(mapped)), &mut None).unwrap();
    assert!(u.ready().read);
    let got = u.recv(16, false).unwrap();
    assert_eq!((got.data.as_slice(), got.from), (&b"q"[..], Some(Addr::Inet(mapped))));
}

#[test]
fn the_machines_dns() {
    let (net, host) = network();
    host.name("deb.debian.org", &[IpAddr::V4(Ipv4Addr::new(151, 101, 2, 132)), IpAddr::V6("2a04:4e42::644".parse().unwrap())]);
    host.dns(|q| {
        let mut r = q.to_vec();
        r[2] |= 0x80;
        Some(r)
    });
    let s = net.socket(Domain::Inet, Kind::Datagram).unwrap();
    let gateway = Addr::Inet(SocketAddr::new(IpAddr::V4(net.gateway()), 53));
    let ask = |id: u16, name: &str, qtype: u16| {
        s.send(&query(id, name, qtype), Some(&gateway), &mut None).unwrap();
        assert!(until(|| s.ready().read), "an answer for {name}");
        let got = s.recv(512, false).unwrap();
        assert_eq!(got.from, Some(gateway.clone()));
        got.data
    };
    let a = ask(1, "deb.debian.org", 1);
    assert_eq!((u16::from_be_bytes([a[6], a[7]]), &a[a.len() - 4..]), (1, &[151, 101, 2, 132][..]));
    let aaaa = ask(2, "deb.debian.org", 28);
    assert_eq!(u16::from_be_bytes([aaaa[6], aaaa[7]]), 1);
    let own = ask(3, "BOX", 1);
    assert_eq!(&own[own.len() - 4..], &net.address().octets());
    let nx = ask(4, "nowhere.example", 1);
    assert_eq!(nx[3] & 0xF, 3);
    // Another type goes to the host's servers, and their answer comes back as it was.
    let srv = ask(5, "_http._tcp.deb.debian.org", 33);
    assert_eq!(&srv[..2], &[0, 5]);
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

#[test]
fn sockets_are_listed() {
    let (net, _) = network();
    let l = net.socket(Domain::Inet, Kind::Stream).unwrap();
    l.bind(&inet("0.0.0.0:22")).unwrap();
    l.listen(1).unwrap();
    let infos = net.sockets();
    assert!(infos.iter().any(|i| i.id == l.id() && i.state == TcpState::Listen && i.local == Some(inet("0.0.0.0:22"))));
    drop(l);
    assert!(net.sockets().is_empty());
}
