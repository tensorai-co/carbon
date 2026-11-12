//! Sockets: lambda's socket system calls, a thin layer over `net` with Linux's argument and `sockaddr` layouts.
//! A socket is an open file ([`SockFile`]), so `read`, `write`, `poll`, `select` and `epoll` work on it: `net` tells
//! the file's watchers when the socket's readiness may have changed. `net` never waits; blocking, `SO_RCVTIMEO` and
//! `SO_SNDTIMEO`, and signals are lambda's, as for every other file.
//!
//! Netlink (`netlink.rs`) answers what programs ask about the machine's interfaces, addresses and routes, and
//! `proc.rs` makes `/proc/net` and the machine's `/etc/resolv.conf`, `/etc/hosts` and `/etc/hostname`.

mod netlink;
mod proc;

pub(crate) use netlink::Netlink;
pub(crate) use proc::{etc_file, proc_net};

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::Watchers;
use layers::New;
use net::{Addr, Domain, Kind, UnixAddr, UnixName};

use crate::Meta;
use crate::abi::*;
use crate::fs::{Node, Open};
use crate::process::Task;
use crate::signal::SIGPIPE;

pub const AF_UNSPEC: u16 = 0;
pub const AF_UNIX: u16 = 1;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;
pub const AF_NETLINK: u16 = 16;
const AF_PACKET: u16 = 17;

const SOCK_STREAM: u32 = 1;
const SOCK_DGRAM: u32 = 2;
const SOCK_RAW: u32 = 3;
const SOCK_SEQPACKET: u32 = 5;
const SOCK_NONBLOCK: u32 = 0o4000;
const SOCK_CLOEXEC: u32 = 0o2000000;

const MSG_PEEK: u64 = 0x2;
const MSG_CTRUNC: u32 = 0x8;
const MSG_TRUNC: u64 = 0x20;
const MSG_DONTWAIT: u64 = 0x40;
const MSG_WAITALL: u64 = 0x100;
const MSG_ERRQUEUE: u64 = 0x2000;
const MSG_NOSIGNAL: u64 = 0x4000;
const MSG_CMSG_CLOEXEC: u64 = 0x4000_0000;

const SOL_SOCKET: i32 = 1;
const SCM_RIGHTS: i32 = 1;
const SCM_CREDENTIALS: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_UDP: i32 = 17;

const SO_REUSEADDR: i32 = 2;
const SO_TYPE: i32 = 3;
const SO_ERROR: i32 = 4;
const SO_BROADCAST: i32 = 6;
const SO_SNDBUF: i32 = 7;
const SO_RCVBUF: i32 = 8;
const SO_KEEPALIVE: i32 = 9;
const SO_REUSEPORT: i32 = 15;
const SO_PASSCRED: i32 = 16;
const SO_PEERCRED: i32 = 17;
const SO_RCVTIMEO: i32 = 20;
const SO_SNDTIMEO: i32 = 21;
const SO_ACCEPTCONN: i32 = 30;
const SO_PEERSEC: i32 = 31;
const SO_PROTOCOL: i32 = 38;
const SO_DOMAIN: i32 = 39;
const TCP_NODELAY: i32 = 1;
const TCP_MAXSEG: i32 = 2;

/// Linux's default socket buffer size, as `SO_SNDBUF` and `SO_RCVBUF` say it (doubled, as Linux does).
const BUFFER_SIZE: u32 = 212_992;
/// The most one send or receive moves.
const MOST_AT_ONCE: usize = 1 << 22;

/// Files passed with `SCM_RIGHTS`.
type Files = Vec<Arc<Open>>;

/// A socket, as an open file.
pub(crate) struct SockFile {
    pub socket: net::Socket,
    pub watchers: Arc<Watchers>,
    _watch: net::Watch,
    protocol: i32,
    /// Options lambda keeps for the asking, by level and name.
    kept: Mutex<HashMap<(i32, i32), Vec<u8>>>,
    /// `SO_RCVTIMEO` and `SO_SNDTIMEO`.
    timeouts: Mutex<[Option<Duration>; 2]>,
}

impl SockFile {
    fn new(socket: net::Socket, protocol: i32) -> Arc<SockFile> {
        let watchers = Arc::new(Watchers::default());
        let weak = Arc::downgrade(&watchers);
        let watch = socket.watch(Arc::new(move || {
            if let Some(w) = weak.upgrade() {
                w.notify();
            }
        }));
        Arc::new(SockFile { socket, watchers, _watch: watch, protocol, kept: Mutex::default(), timeouts: Mutex::default() })
    }

    pub fn open(socket: net::Socket, protocol: i32, nonblock: bool) -> Arc<Open> {
        let path = format!("socket:[{}]", socket.id());
        Open::new(Node::Socket(SockFile::new(socket, protocol)), O_RDWR | if nonblock { O_NONBLOCK } else { 0 }, path)
    }
}

/// What a program's socket call got from `net`, as an errno.
pub(crate) fn errno(e: net::Error) -> Errno {
    use net::Error as E;
    match e {
        E::WouldBlock => EAGAIN,
        E::InProgress => EINPROGRESS,
        E::Already => EALREADY,
        E::IsConnected => EISCONN,
        E::NotConnected => ENOTCONN,
        E::ConnectionRefused => ECONNREFUSED,
        E::ConnectionReset => ECONNRESET,
        E::ConnectionAborted => ECONNABORTED,
        E::BrokenPipe => EPIPE,
        E::AddrInUse => EADDRINUSE,
        E::AddrNotAvailable => EADDRNOTAVAIL,
        E::NetworkUnreachable => ENETUNREACH,
        E::HostUnreachable => EHOSTUNREACH,
        E::TimedOut => ETIMEDOUT,
        E::Denied => EACCES,
        E::Invalid => EINVAL,
        E::NotSupported => EOPNOTSUPP,
        E::WrongFamily => EAFNOSUPPORT,
        E::MessageTooBig => EMSGSIZE,
        E::DestinationRequired => EDESTADDRREQ,
        E::NotFound => ENOENT,
        E::Other(_) => EIO,
    }
}

/// `net`'s answer as a wait's: nothing yet, or what it got.
fn attempt<T>(result: net::Result<T>) -> Result<Option<T>, Errno> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(net::Error::WouldBlock) => Ok(None),
        Err(e) => Err(errno(e)),
    }
}

/// A `sockaddr` as a program gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Given {
    Unspec,
    Inet(SocketAddr),
    Unix(UnixName),
    Netlink(u32),
}

fn parse_sockaddr(bytes: &[u8]) -> Result<Given, Errno> {
    let family = u16::from_le_bytes([*bytes.first().ok_or(EINVAL)?, *bytes.get(1).ok_or(EINVAL)?]);
    let port = || u16::from_be_bytes([bytes[2], bytes[3]]);
    match family {
        AF_UNSPEC => Ok(Given::Unspec),
        AF_INET if bytes.len() >= 16 => Ok(Given::Inet(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]), port())))),
        AF_INET6 if bytes.len() >= 24 => {
            let ip: [u8; 16] = bytes[8..24].try_into().map_err(|_| EINVAL)?;
            let scope = bytes.get(24..28).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
            Ok(Given::Inet(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(ip), port(), 0, scope))))
        }
        AF_INET | AF_INET6 => Err(EINVAL),
        AF_UNIX => {
            let path = &bytes[2..];
            Ok(Given::Unix(match path.first() {
                None => UnixName::Unnamed,
                Some(0) => UnixName::Abstract(path[1..].to_vec()),
                Some(_) => UnixName::Path(path[..path.iter().position(|&b| b == 0).unwrap_or(path.len())].to_vec()),
            }))
        }
        AF_NETLINK if bytes.len() >= 12 => Ok(Given::Netlink(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]))),
        _ => Err(EAFNOSUPPORT),
    }
}

/// An address as a `sockaddr`.
fn encode(addr: &Addr) -> Vec<u8> {
    match addr {
        Addr::Inet(SocketAddr::V4(v4)) => {
            let mut b = AF_INET.to_le_bytes().to_vec();
            b.extend(v4.port().to_be_bytes());
            b.extend(v4.ip().octets());
            b.extend([0; 8]);
            b
        }
        Addr::Inet(SocketAddr::V6(v6)) => {
            let mut b = AF_INET6.to_le_bytes().to_vec();
            b.extend(v6.port().to_be_bytes());
            b.extend(v6.flowinfo().to_be_bytes());
            b.extend(v6.ip().octets());
            b.extend(v6.scope_id().to_le_bytes());
            b
        }
        Addr::Unix(unix) => {
            let mut b = AF_UNIX.to_le_bytes().to_vec();
            match &unix.name {
                UnixName::Unnamed => {}
                UnixName::Path(path) => {
                    b.extend(path);
                    b.push(0);
                }
                UnixName::Abstract(name) => {
                    b.push(0);
                    b.extend(name);
                }
            }
            b
        }
    }
}

fn encode_netlink(port: u32) -> Vec<u8> {
    let mut b = AF_NETLINK.to_le_bytes().to_vec();
    b.extend([0, 0]);
    b.extend(port.to_le_bytes());
    b.extend([0; 4]);
    b
}

/// An `int` option's bytes.
fn int(v: impl Into<i64>) -> Vec<u8> {
    (v.into() as i32).to_le_bytes().to_vec()
}

fn timeval(t: Option<Duration>) -> Vec<u8> {
    let t = t.unwrap_or_default();
    let mut b = t.as_secs().to_le_bytes().to_vec();
    b.extend(u64::from(t.subsec_micros()).to_le_bytes());
    b
}

/// A socket file, of either kind.
enum Sock {
    Net(Arc<Open>, Arc<SockFile>),
    Netlink(Arc<Open>, Arc<Netlink>),
}

impl Task {
    /// Who this process is, for the other end of a Unix socket.
    fn cred(&self) -> net::Cred {
        let creds = lock(&self.process.state).creds.clone();
        net::Cred { pid: self.process.pid, uid: creds.uid[1], gid: creds.gid[1] }
    }

    fn sock(&self, fd: i32) -> Result<Sock, Errno> {
        let open = self.file(fd)?;
        match &open.node {
            Node::Socket(s) => {
                let s = s.clone();
                Ok(Sock::Net(open, s))
            }
            Node::Netlink(n) => {
                let n = n.clone();
                Ok(Sock::Netlink(open, n))
            }
            _ => Err(ENOTSOCK),
        }
    }

    fn net_sock(&self, fd: i32) -> Result<(Arc<Open>, Arc<SockFile>), Errno> {
        match self.sock(fd)? {
            Sock::Net(open, s) => Ok((open, s)),
            Sock::Netlink(..) => Err(EOPNOTSUPP),
        }
    }

    fn read_sockaddr(&mut self, cx: &mut Cx<'_>, at: u64, len: u64) -> Result<Given, Errno> {
        if !(2..=128).contains(&len) {
            return Err(EINVAL);
        }
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize))?;
        parse_sockaddr(&bytes)
    }

    /// Writes an address where a program asked for one: as much as its length says there's room for, and the
    /// address's own length back.
    fn write_sockaddr(&mut self, cx: &mut Cx<'_>, at: u64, len_at: u64, bytes: &[u8]) -> Result<(), Errno> {
        if at == 0 || len_at == 0 {
            return Ok(());
        }
        let room = self.mem(cx, false, |t| user::read_u32(&t.space(), len_at))? as usize;
        if (room as i32) < 0 {
            return Err(EINVAL);
        }
        self.mem(cx, true, |t| {
            user::write(&mut t.space(), at, &bytes[..room.min(bytes.len())])?;
            user::write_u32(&mut t.space(), len_at, bytes.len() as u32)
        })
    }

    /// A Unix socket's path, made on the disk as a socket file for `bind`.
    fn unix_bind_path(&self, path: &[u8]) -> Result<UnixAddr, Errno> {
        let text = std::str::from_utf8(path).map_err(|_| EINVAL)?;
        let ((dir, dir_path), umask) = {
            let state = lock(&self.process.state);
            (state.cwd.clone(), state.umask)
        };
        self.writable(&crate::process::exe_path(&dir_path, text))?;
        let view = self.view();
        let mut disk = lock(&self.k.disk);
        match view.resolve(&disk, dir, text, false) {
            Ok(_) => return Err(EADDRINUSE),
            Err(layers::Error::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        let (parent, name) = view.resolve_parent(&disk, dir, text)?;
        let ino = disk.create(parent, name, New::File, Meta { mode: S_IFSOCK | (0o777 & !umask), ..Meta::default() }, self.k.now())?;
        Ok(UnixAddr { name: UnixName::Path(path.to_vec()), file: Some(ino) })
    }

    /// A Unix socket's path, found on the disk, for `connect` and `sendto`.
    fn unix_path(&self, path: &[u8]) -> Result<UnixAddr, Errno> {
        let text = std::str::from_utf8(path).map_err(|_| EINVAL)?;
        let dir = lock(&self.process.state).cwd.0;
        let view = self.view();
        let disk = lock(&self.k.disk);
        let ino = view.resolve(&disk, dir, text, true)?;
        if disk.meta(ino)?.mode & S_IFMT != S_IFSOCK {
            return Err(ECONNREFUSED);
        }
        Ok(UnixAddr { name: UnixName::Path(path.to_vec()), file: Some(ino) })
    }

    /// What a program gave, as an address for `socket`: for `bind`, a Unix path is made.
    fn net_addr(&self, socket: &net::Socket, given: Given, binding: bool) -> Result<Addr, Errno> {
        match (socket.domain(), given) {
            (Domain::Inet, Given::Inet(a @ SocketAddr::V4(_))) | (Domain::Inet6, Given::Inet(a @ SocketAddr::V6(_))) => Ok(Addr::Inet(a)),
            (Domain::Unix, Given::Unix(UnixName::Path(path))) if binding => Ok(Addr::Unix(self.unix_bind_path(&path)?)),
            (Domain::Unix, Given::Unix(UnixName::Path(path))) => Ok(Addr::Unix(self.unix_path(&path)?)),
            (Domain::Unix, Given::Unix(name)) => Ok(Addr::Unix(UnixAddr { name, file: None })),
            (Domain::Inet, Given::Inet(_)) | (Domain::Inet6, Given::Inet(_)) => Err(EAFNOSUPPORT),
            _ => Err(EINVAL),
        }
    }

    /// Waits for `attempt` to get something, as a socket call does: not at all if `nonblock` (EAGAIN), else until it
    /// does, `timeout` passes (EAGAIN, as for `SO_RCVTIMEO`), or a signal comes (EINTR).
    fn sock_wait<R>(&self, open: &Open, nonblock: bool, timeout: Option<Duration>, mut attempt: impl FnMut(&Task) -> Result<Option<R>, Errno>) -> Result<R, Errno> {
        if nonblock {
            return attempt(self)?.ok_or(EAGAIN);
        }
        self.wait_ready(timeout, &[open], attempt)?.ok_or(EAGAIN)
    }

    pub(crate) fn socket(&mut self, domain: u64, kind: u64, protocol: u64) -> Result<u64, Errno> {
        let kind = kind as u32;
        if kind & !(0xf | SOCK_NONBLOCK | SOCK_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let open = self.new_socket(domain as u16, kind & 0xf, protocol as i32, kind & SOCK_NONBLOCK != 0)?;
        lock(&self.process.state).files.add(open, kind & SOCK_CLOEXEC != 0, 0).map(|fd| fd as u64)
    }

    fn new_socket(&self, family: u16, kind: u32, protocol: i32, nonblock: bool) -> Result<Arc<Open>, Errno> {
        let (domain, kind, protocol) = match family {
            AF_NETLINK => {
                if kind != SOCK_RAW && kind != SOCK_DGRAM {
                    return Err(ESOCKTNOSUPPORT);
                }
                // NETLINK_ROUTE only: the others (audit, uevents, …) aren't the machine's to give.
                if protocol != 0 {
                    return Err(EPROTONOSUPPORT);
                }
                let netlink = Netlink::new(kind);
                let path = format!("socket:[{}]", netlink.id);
                return Ok(Open::new(Node::Netlink(netlink), O_RDWR | if nonblock { O_NONBLOCK } else { 0 }, path));
            }
            AF_UNIX => {
                let kind = match kind {
                    SOCK_STREAM => Kind::Stream,
                    SOCK_DGRAM => Kind::Datagram,
                    SOCK_SEQPACKET => Kind::SeqPacket,
                    _ => return Err(ESOCKTNOSUPPORT),
                };
                if protocol != 0 && protocol != 1 {
                    return Err(EPROTONOSUPPORT);
                }
                (Domain::Unix, kind, 0)
            }
            AF_INET | AF_INET6 => {
                let domain = if family == AF_INET { Domain::Inet } else { Domain::Inet6 };
                match (kind, protocol) {
                    (SOCK_STREAM, 0 | IPPROTO_TCP) => (domain, Kind::Stream, IPPROTO_TCP),
                    (SOCK_DGRAM, 0 | IPPROTO_UDP) => (domain, Kind::Datagram, IPPROTO_UDP),
                    // ICMP echo sockets, as `ping` uses unprivileged: not yet, and `ping` says so.
                    (SOCK_DGRAM, 1 | 58) => return Err(EACCES),
                    // Raw sockets aren't a machine's, as in a rootless container.
                    (SOCK_RAW, _) => return Err(EPERM),
                    (SOCK_STREAM | SOCK_DGRAM, _) => return Err(EPROTONOSUPPORT),
                    _ => return Err(ESOCKTNOSUPPORT),
                }
            }
            AF_PACKET => return Err(EPERM),
            _ => return Err(EAFNOSUPPORT),
        };
        let socket = self.k.net.socket(domain, kind).map_err(errno)?;
        socket.set_cred(self.cred());
        Ok(SockFile::open(socket, protocol, nonblock))
    }

    pub(crate) fn socketpair(&mut self, cx: &mut Cx<'_>, family: u64, kind: u64, protocol: u64, at: u64) -> Result<u64, Errno> {
        let flags = kind as u32;
        let kind = match flags & 0xf {
            SOCK_STREAM => Kind::Stream,
            SOCK_DGRAM => Kind::Datagram,
            SOCK_SEQPACKET => Kind::SeqPacket,
            _ => return Err(ESOCKTNOSUPPORT),
        };
        if family as u16 != AF_UNIX {
            return Err(if matches!(family as u16, AF_INET | AF_INET6) { EOPNOTSUPP } else { EAFNOSUPPORT });
        }
        if protocol != 0 && protocol != 1 {
            return Err(EPROTONOSUPPORT);
        }
        let (a, b) = self.k.net.pair(kind).map_err(errno)?;
        let nonblock = flags & SOCK_NONBLOCK != 0;
        let mut fds = [0i32; 2];
        for (i, socket) in [a, b].into_iter().enumerate() {
            socket.set_cred(self.cred());
            fds[i] = lock(&self.process.state).files.add(SockFile::open(socket, 0, nonblock), flags & SOCK_CLOEXEC != 0, 0)?;
        }
        let mut bytes = fds[0].to_le_bytes().to_vec();
        bytes.extend(fds[1].to_le_bytes());
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes))?;
        Ok(0)
    }

    pub(crate) fn bind(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len: u64) -> Result<u64, Errno> {
        let given = self.read_sockaddr(cx, at, len)?;
        match self.sock(fd)? {
            Sock::Netlink(_, netlink) => {
                let Given::Netlink(port) = given else { return Err(EINVAL) };
                netlink.bind(if port == 0 { self.process.pid as u32 } else { port });
                Ok(0)
            }
            Sock::Net(_, s) => {
                let addr = self.net_addr(&s.socket, given, true)?;
                s.socket.bind(&addr).map_err(errno)?;
                Ok(0)
            }
        }
    }

    pub(crate) fn listen(&mut self, fd: i32, backlog: u64) -> Result<u64, Errno> {
        let (_, s) = self.net_sock(fd)?;
        s.socket.listen(backlog as u32).map_err(errno)?;
        Ok(0)
    }

    pub(crate) fn connect(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len: u64) -> Result<u64, Errno> {
        let given = self.read_sockaddr(cx, at, len)?;
        let (open, s) = match self.sock(fd)? {
            Sock::Netlink(..) => return Ok(0),
            Sock::Net(open, s) => (open, s),
        };
        if given == Given::Unspec {
            s.socket.disconnect();
            return Ok(0);
        }
        let addr = self.net_addr(&s.socket, given, false)?;
        match s.socket.connect(&addr) {
            Ok(()) => Ok(0),
            Err(net::Error::InProgress) if open.nonblock() => Err(EINPROGRESS),
            Err(net::Error::InProgress) => {
                let timeout = lock(&s.timeouts)[1];
                let done = self.wait_ready(timeout, &[&open], |_| {
                    let ready = s.socket.ready();
                    Ok((ready.write || ready.error || ready.hangup).then_some(()))
                })?;
                if done.is_none() {
                    return Err(EINPROGRESS);
                }
                match s.socket.take_error() {
                    Some(e) => Err(errno(e)),
                    None => Ok(0),
                }
            }
            Err(e) => Err(errno(e)),
        }
    }

    pub(crate) fn accept4(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len_at: u64, flags: u32) -> Result<u64, Errno> {
        if flags & !(SOCK_NONBLOCK | SOCK_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let (open, s) = self.net_sock(fd)?;
        if s.socket.kind() == Kind::Datagram {
            return Err(EOPNOTSUPP);
        }
        if !s.socket.is_listening() {
            return Err(EINVAL);
        }
        let (socket, from) = self.sock_wait(&open, open.nonblock(), lock(&s.timeouts)[0], |_| attempt(s.socket.accept()))?;
        let accepted = SockFile::open(socket, s.protocol, flags & SOCK_NONBLOCK != 0);
        let fd = lock(&self.process.state).files.add(accepted, flags & SOCK_CLOEXEC != 0, 0)?;
        self.write_sockaddr(cx, at, len_at, &encode(&from))?;
        Ok(fd as u64)
    }

    pub(crate) fn getsockname(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, len_at: u64, peer: bool) -> Result<u64, Errno> {
        let bytes = match self.sock(fd)? {
            Sock::Netlink(_, netlink) => encode_netlink(if peer { 0 } else { netlink.port() }),
            Sock::Net(_, s) if peer => encode(&s.socket.peer().map_err(errno)?),
            Sock::Net(_, s) => encode(&s.socket.local()),
        };
        self.write_sockaddr(cx, at, len_at, &bytes)?;
        Ok(0)
    }

    pub(crate) fn shutdown(&mut self, fd: i32, how: u64) -> Result<u64, Errno> {
        let how = match how {
            0 => net::Shutdown::Read,
            1 => net::Shutdown::Write,
            2 => net::Shutdown::Both,
            _ => return Err(EINVAL),
        };
        match self.sock(fd)? {
            Sock::Netlink(..) => Ok(0),
            Sock::Net(_, s) => s.socket.shutdown(how).map(|()| 0).map_err(errno),
        }
    }

    /// Sends `data` on a socket file, waiting as the file and `flags` say: a blocking stream sends it all.
    pub(crate) fn sock_send(&self, open: &Open, s: &SockFile, data: &[u8], to: Option<Addr>, mut extra: Option<net::Ancillary>, flags: u64) -> Result<usize, Errno> {
        let nonblock = open.nonblock() || flags & MSG_DONTWAIT != 0;
        let timeout = lock(&s.timeouts)[1];
        let stream = s.socket.kind() == Kind::Stream;
        let mut sent = 0;
        let result = loop {
            match self.sock_wait(open, nonblock, timeout, |_| attempt(s.socket.send(&data[sent..], to.as_ref(), &mut extra))) {
                Ok(n) => {
                    sent += n;
                    if !stream || sent >= data.len() || nonblock {
                        break Ok(sent);
                    }
                }
                Err(_) if sent > 0 => break Ok(sent),
                Err(e) => break Err(e),
            }
        };
        if result == Err(EPIPE) && flags & MSG_NOSIGNAL == 0 {
            self.k.signal_thread(&self.process, self.tid, SIGPIPE);
        }
        result
    }

    /// Receives from a socket file, waiting as the file and `flags` say.
    pub(crate) fn sock_recv(&self, open: &Open, s: &SockFile, len: usize, flags: u64) -> Result<net::Received, Errno> {
        if flags & MSG_ERRQUEUE != 0 {
            return Err(EAGAIN);
        }
        let nonblock = open.nonblock() || flags & MSG_DONTWAIT != 0;
        let peek = flags & MSG_PEEK != 0;
        let timeout = lock(&s.timeouts)[0];
        let len = len.min(MOST_AT_ONCE);
        let mut got = self.sock_wait(open, nonblock, timeout, |_| attempt(s.socket.recv(len, peek)))?;
        if flags & MSG_WAITALL != 0 && !peek && s.socket.kind() == Kind::Stream {
            while got.data.len() < len && !got.data.is_empty() {
                match self.sock_wait(open, nonblock, timeout, |_| attempt(s.socket.recv(len - got.data.len(), false))) {
                    Ok(more) if !more.data.is_empty() => got.data.extend(more.data),
                    _ => break,
                }
            }
            got.full = got.data.len();
        }
        Ok(got)
    }

    /// `sendto`, with Linux's six arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sendto(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64, flags: u64, to_at: u64, to_len: u64) -> Result<u64, Errno> {
        let data = self.mem(cx, false, |t| user::read_vec(&t.space(), buf, (len as usize).min(MOST_AT_ONCE)))?;
        let to = if to_at != 0 { Some(self.read_sockaddr(cx, to_at, to_len)?) } else { None };
        match self.sock(fd)? {
            Sock::Netlink(_, netlink) => {
                netlink.request(&self.k.net, &data, self.process.pid as u32);
                Ok(data.len() as u64)
            }
            Sock::Net(open, s) => {
                let to = match to {
                    Some(given) if s.socket.kind() != Kind::Stream => Some(self.net_addr(&s.socket, given, false)?),
                    _ => None,
                };
                self.sock_send(&open, &s, &data, to, None, flags).map(|n| n as u64)
            }
        }
    }

    /// `recvfrom`, with Linux's six arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn recvfrom(&mut self, cx: &mut Cx<'_>, fd: i32, buf: u64, len: u64, flags: u64, from_at: u64, from_len: u64) -> Result<u64, Errno> {
        let (data, full, from) = match self.sock(fd)? {
            Sock::Netlink(open, netlink) => {
                let (data, full) = self.sock_wait(&open, open.nonblock() || flags & MSG_DONTWAIT != 0, None, |_| Ok(netlink.recv(len as usize, flags & MSG_PEEK != 0)))?;
                (data, full, Some(encode_netlink(0)))
            }
            Sock::Net(open, s) => {
                let got = self.sock_recv(&open, &s, len as usize, flags)?;
                (got.data, got.full, got.from.as_ref().map(encode))
            }
        };
        self.mem(cx, true, |t| user::write(&mut t.space(), buf, &data))?;
        match from {
            Some(from) => self.write_sockaddr(cx, from_at, from_len, &from)?,
            None if from_len != 0 => self.mem(cx, true, |t| user::write_u32(&mut t.space(), from_len, 0))?,
            None => {}
        }
        Ok(if flags & MSG_TRUNC != 0 { full } else { data.len() } as u64)
    }

    /// A `msghdr`: its name, its iovecs, and its control buffer.
    fn read_msghdr(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<[u64; 6], Errno> {
        self.mem(cx, false, |t| {
            let s = t.space();
            Ok([
                user::read_u64(&s, at)?,
                u64::from(user::read_u32(&s, at + 8)?),
                user::read_u64(&s, at + 16)?,
                user::read_u64(&s, at + 24)?,
                user::read_u64(&s, at + 32)?,
                user::read_u64(&s, at + 40)?,
            ])
        })
    }

    pub(crate) fn sendmsg(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, flags: u64) -> Result<u64, Errno> {
        let [name, name_len, iov, iov_len, control, control_len] = self.read_msghdr(cx, at)?;
        let iovecs = self.iovecs(cx, iov, iov_len)?;
        let data = self.mem(cx, false, |t| {
            let space = t.space();
            let mut data = Vec::new();
            for &(base, len) in &iovecs {
                data.extend(user::read_vec(&space, base, (len as usize).min(MOST_AT_ONCE))?);
            }
            Ok(data)
        })?;
        let to = if name != 0 && name_len > 0 { Some(self.read_sockaddr(cx, name, name_len)?) } else { None };
        let (open, s) = match self.sock(fd)? {
            Sock::Netlink(_, netlink) => {
                netlink.request(&self.k.net, &data, self.process.pid as u32);
                return Ok(data.len() as u64);
            }
            Sock::Net(open, s) => (open, s),
        };
        let extra = if control != 0 && control_len >= 16 { self.read_rights(cx, control, control_len.min(1 << 16))? } else { None };
        let to = match to {
            Some(given) if s.socket.kind() != Kind::Stream => Some(self.net_addr(&s.socket, given, false)?),
            _ => None,
        };
        self.sock_send(&open, &s, &data, to, extra, flags).map(|n| n as u64)
    }

    /// The files an `SCM_RIGHTS` message in a control buffer passes, if it has one.
    fn read_rights(&mut self, cx: &mut Cx<'_>, at: u64, len: u64) -> Result<Option<net::Ancillary>, Errno> {
        let control = self.mem(cx, false, |t| user::read_vec(&t.space(), at, len as usize))?;
        let mut files: Files = Vec::new();
        let mut i = 0;
        while i + 16 <= control.len() {
            let cmsg_len = u64::from_le_bytes(control[i..i + 8].try_into().map_err(|_| EINVAL)?) as usize;
            let level = i32::from_le_bytes(control[i + 8..i + 12].try_into().map_err(|_| EINVAL)?);
            let kind = i32::from_le_bytes(control[i + 12..i + 16].try_into().map_err(|_| EINVAL)?);
            if cmsg_len < 16 || i + cmsg_len > control.len() {
                return Err(EINVAL);
            }
            if level == SOL_SOCKET && kind == SCM_RIGHTS {
                for fd in control[i + 16..i + cmsg_len].as_chunks::<4>().0 {
                    files.push(self.file(i32::from_le_bytes(*fd))?);
                }
            }
            i += cmsg_len.next_multiple_of(8);
        }
        Ok((!files.is_empty()).then(|| Box::new(files) as net::Ancillary))
    }

    pub(crate) fn recvmsg(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, flags: u64) -> Result<u64, Errno> {
        let [name, name_len, iov, iov_len, control, control_len] = self.read_msghdr(cx, at)?;
        let iovecs = self.iovecs(cx, iov, iov_len)?;
        let total: u64 = iovecs.iter().map(|(_, len)| len).sum();
        let (data, full, from, extra, cred, pass_cred) = match self.sock(fd)? {
            Sock::Netlink(open, netlink) => {
                let (data, full) = self.sock_wait(&open, open.nonblock() || flags & MSG_DONTWAIT != 0, None, |_| Ok(netlink.recv(total as usize, flags & MSG_PEEK != 0)))?;
                (data, full, Some(encode_netlink(0)), None, None, false)
            }
            Sock::Net(open, s) => {
                let got = self.sock_recv(&open, &s, total as usize, flags)?;
                (got.data, got.full, got.from.as_ref().map(encode), got.extra, got.cred, s.socket.options().pass_cred)
            }
        };
        self.mem(cx, true, |t| {
            let mut done = 0;
            for &(base, len) in &iovecs {
                let n = (len as usize).min(data.len() - done);
                user::write(&mut t.space(), base, &data[done..done + n])?;
                done += n;
            }
            Ok(())
        })?;
        // The sender's address, for a datagram.
        match from {
            Some(from) if name != 0 => {
                let n = from.len().min(name_len as usize);
                self.mem(cx, true, |t| {
                    user::write(&mut t.space(), name, &from[..n])?;
                    user::write_u32(&mut t.space(), at + 8, from.len() as u32)
                })?;
            }
            _ => self.mem(cx, true, |t| user::write_u32(&mut t.space(), at + 8, 0))?,
        }
        // Control messages: files passed, and who sent it if the program asked.
        let mut out = Vec::new();
        let mut out_flags = 0u32;
        if let Some(extra) = extra
            && let Ok(files) = extra.downcast::<Files>()
        {
            let need = 16 + (files.len() * 4).next_multiple_of(8);
            if control == 0 || (out.len() + need) as u64 > control_len {
                // No room: the files close, as on Linux.
                out_flags |= MSG_CTRUNC;
            } else {
                let mut fds = Vec::new();
                for file in files.iter() {
                    fds.push(lock(&self.process.state).files.add(file.clone(), flags & MSG_CMSG_CLOEXEC != 0, 0)?);
                }
                out.extend(((16 + fds.len() * 4) as u64).to_le_bytes());
                out.extend(SOL_SOCKET.to_le_bytes());
                out.extend(SCM_RIGHTS.to_le_bytes());
                for fd in fds {
                    out.extend(fd.to_le_bytes());
                }
                out.resize(out.len().next_multiple_of(8), 0);
            }
        }
        if pass_cred && let Some(cred) = cred {
            if control != 0 && (out.len() + 32) as u64 <= control_len {
                out.extend(28u64.to_le_bytes());
                out.extend(SOL_SOCKET.to_le_bytes());
                out.extend(SCM_CREDENTIALS.to_le_bytes());
                out.extend(cred.pid.to_le_bytes());
                out.extend(cred.uid.to_le_bytes());
                out.extend(cred.gid.to_le_bytes());
                out.extend([0; 4]);
            } else {
                out_flags |= MSG_CTRUNC;
            }
        }
        if full > data.len() {
            out_flags |= MSG_TRUNC as u32;
        }
        self.mem(cx, true, |t| {
            if !out.is_empty() {
                user::write(&mut t.space(), control, &out)?;
            }
            user::write_u64(&mut t.space(), at + 40, out.len() as u64)?;
            user::write_u32(&mut t.space(), at + 48, out_flags)
        })?;
        Ok(if flags & MSG_TRUNC != 0 { full } else { data.len() } as u64)
    }

    /// `sendmmsg`: each `mmsghdr` in turn, its length written back, until one fails.
    pub(crate) fn sendmmsg(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        let mut done = 0;
        for i in 0..count.min(1024) {
            let header = at + i * 64;
            match self.sendmsg(cx, fd, header, flags) {
                Ok(n) => self.mem(cx, true, |t| user::write_u32(&mut t.space(), header + 56, n as u32))?,
                Err(e) if done == 0 => return Err(e),
                Err(_) => break,
            }
            done += 1;
        }
        Ok(done)
    }

    /// `recvmmsg`: the first message as asked, then as many more as are there without waiting.
    pub(crate) fn recvmmsg(&mut self, cx: &mut Cx<'_>, fd: i32, at: u64, count: u64, flags: u64) -> Result<u64, Errno> {
        let mut done = 0;
        for i in 0..count.min(1024) {
            let header = at + i * 64;
            let flags = if done == 0 { flags } else { flags | MSG_DONTWAIT };
            match self.recvmsg(cx, fd, header, flags) {
                Ok(n) => self.mem(cx, true, |t| user::write_u32(&mut t.space(), header + 56, n as u32))?,
                Err(e) if done == 0 => return Err(e),
                Err(_) => break,
            }
            done += 1;
        }
        Ok(done)
    }

    pub(crate) fn setsockopt(&mut self, cx: &mut Cx<'_>, fd: i32, level: i32, name: i32, at: u64, len: u64) -> Result<u64, Errno> {
        let value = self.mem(cx, false, |t| user::read_vec(&t.space(), at, (len as usize).min(256)))?;
        let s = match self.sock(fd)? {
            Sock::Netlink(..) => return Ok(0),
            Sock::Net(_, s) => s,
        };
        let flag = || value.get(..4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) != 0).ok_or(EINVAL);
        match (level, name) {
            (SOL_SOCKET, SO_REUSEADDR) => {
                let on = flag()?;
                s.socket.set_options(|o| o.reuse_addr = on);
            }
            (SOL_SOCKET, SO_REUSEPORT) => {
                let on = flag()?;
                s.socket.set_options(|o| o.reuse_port = on);
            }
            (SOL_SOCKET, SO_KEEPALIVE) => {
                let on = flag()?;
                s.socket.set_options(|o| o.keepalive = on);
            }
            (SOL_SOCKET, SO_BROADCAST) => {
                let on = flag()?;
                s.socket.set_options(|o| o.broadcast = on);
            }
            (SOL_SOCKET, SO_PASSCRED) => {
                let on = flag()?;
                s.socket.set_options(|o| o.pass_cred = on);
            }
            (IPPROTO_TCP, TCP_NODELAY) => {
                let on = flag()?;
                s.socket.set_options(|o| o.nodelay = on);
            }
            (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => {
                if value.len() < 16 {
                    return Err(EINVAL);
                }
                let sec = i64::from_le_bytes(value[..8].try_into().map_err(|_| EINVAL)?);
                let usec = i64::from_le_bytes(value[8..16].try_into().map_err(|_| EINVAL)?);
                if !(0..1_000_000).contains(&usec) {
                    return Err(EDOM);
                }
                let t = Duration::new(sec.max(0) as u64, usec as u32 * 1000);
                lock(&s.timeouts)[usize::from(name == SO_SNDTIMEO)] = (!t.is_zero()).then_some(t);
            }
            (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => {
                // Linux doubles what it's given, for its own bookkeeping.
                let n = i32::from_le_bytes(value.get(..4).ok_or(EINVAL)?.try_into().map_err(|_| EINVAL)?);
                lock(&s.kept).insert((level, name), int(n.clamp(2304, 1 << 24) * 2));
            }
            _ => {
                lock(&s.kept).insert((level, name), value);
            }
        }
        Ok(0)
    }

    pub(crate) fn getsockopt(&mut self, cx: &mut Cx<'_>, fd: i32, level: i32, name: i32, at: u64, len_at: u64) -> Result<u64, Errno> {
        let value = match self.sock(fd)? {
            Sock::Netlink(_, netlink) => match (level, name) {
                (SOL_SOCKET, SO_TYPE) => int(netlink.kind),
                (SOL_SOCKET, SO_DOMAIN) => int(AF_NETLINK),
                (SOL_SOCKET, SO_PROTOCOL | SO_ERROR) => int(0),
                (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => int(BUFFER_SIZE),
                _ => int(0),
            },
            Sock::Net(_, s) => {
                let socket = &s.socket;
                let options = socket.options();
                match (level, name) {
                    (SOL_SOCKET, SO_TYPE) => int(match socket.kind() {
                        Kind::Stream => SOCK_STREAM,
                        Kind::Datagram => SOCK_DGRAM,
                        Kind::SeqPacket => SOCK_SEQPACKET,
                    }),
                    (SOL_SOCKET, SO_ERROR) => int(socket.take_error().map_or(0, |e| errno(e).0)),
                    (SOL_SOCKET, SO_DOMAIN) => int(match socket.domain() {
                        Domain::Unix => AF_UNIX,
                        Domain::Inet => AF_INET,
                        Domain::Inet6 => AF_INET6,
                    }),
                    (SOL_SOCKET, SO_PROTOCOL) => int(s.protocol),
                    (SOL_SOCKET, SO_ACCEPTCONN) => int(socket.is_listening()),
                    (SOL_SOCKET, SO_REUSEADDR) => int(options.reuse_addr),
                    (SOL_SOCKET, SO_REUSEPORT) => int(options.reuse_port),
                    (SOL_SOCKET, SO_KEEPALIVE) => int(options.keepalive),
                    (SOL_SOCKET, SO_BROADCAST) => int(options.broadcast),
                    (SOL_SOCKET, SO_PASSCRED) => int(options.pass_cred),
                    (IPPROTO_TCP, TCP_NODELAY) => int(options.nodelay),
                    (IPPROTO_TCP, TCP_MAXSEG) => int(1448),
                    (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => timeval(lock(&s.timeouts)[usize::from(name == SO_SNDTIMEO)]),
                    (SOL_SOCKET, SO_PEERCRED) => {
                        let cred = socket.peer_cred().unwrap_or(net::Cred { pid: 0, uid: u32::MAX, gid: u32::MAX });
                        let mut b = cred.pid.to_le_bytes().to_vec();
                        b.extend(cred.uid.to_le_bytes());
                        b.extend(cred.gid.to_le_bytes());
                        b
                    }
                    (SOL_SOCKET, SO_PEERSEC) => return Err(ENOPROTOOPT),
                    _ => match lock(&s.kept).get(&(level, name)) {
                        Some(kept) => kept.clone(),
                        None if level == SOL_SOCKET && matches!(name, SO_SNDBUF | SO_RCVBUF) => int(BUFFER_SIZE),
                        None => int(0),
                    },
                }
            }
        };
        let room = self.mem(cx, false, |t| user::read_u32(&t.space(), len_at))? as usize;
        if (room as i32) < 0 {
            return Err(EINVAL);
        }
        let n = room.min(value.len());
        self.mem(cx, true, |t| {
            user::write(&mut t.space(), at, &value[..n])?;
            user::write_u32(&mut t.space(), len_at, n as u32)
        })?;
        Ok(0)
    }

    /// The interface ioctls (`SIOCGIFCONF`, `SIOCGIFFLAGS`, …) and `FIONREAD`, on any socket.
    pub(crate) fn socket_ioctl(&mut self, cx: &mut Cx<'_>, open: &Open, request: u64, arg: u64) -> Result<u64, Errno> {
        const FIONREAD: u64 = 0x541b;
        const SIOCATMARK: u64 = 0x8905;
        const SIOCGIFNAME: u64 = 0x8910;
        const SIOCGIFCONF: u64 = 0x8912;
        const SIOCGIFFLAGS: u64 = 0x8913;
        const SIOCGIFADDR: u64 = 0x8915;
        const SIOCGIFBRDADDR: u64 = 0x8919;
        const SIOCGIFNETMASK: u64 = 0x891b;
        const SIOCGIFMTU: u64 = 0x8921;
        const SIOCGIFHWADDR: u64 = 0x8927;
        const SIOCGIFINDEX: u64 = 0x8933;
        let interfaces = self.k.net.interfaces();
        let v4 = |iface: &net::Interface| {
            iface.addrs.iter().find_map(|&(ip, prefix)| match ip {
                IpAddr::V4(v4) => Some((v4, prefix)),
                IpAddr::V6(_) => None,
            })
        };
        let sockaddr_in = |ip: Ipv4Addr| encode(&Addr::Inet(SocketAddr::new(IpAddr::V4(ip), 0)));
        match request {
            FIONREAD => {
                let n = match &open.node {
                    Node::Socket(s) => s.socket.available(),
                    Node::Netlink(n) => n.available(),
                    _ => 0,
                };
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, n as u32))?;
                Ok(0)
            }
            SIOCATMARK => {
                self.mem(cx, true, |t| user::write_u32(&mut t.space(), arg, 0))?;
                Ok(0)
            }
            SIOCGIFCONF => {
                let (len, buf) = self.mem(cx, false, |t| Ok((user::read_u32(&t.space(), arg)?, user::read_u64(&t.space(), arg + 8)?)))?;
                let mut out = Vec::new();
                for iface in &interfaces {
                    if let Some((ip, _)) = v4(iface) {
                        let mut entry = [0u8; 40];
                        entry[..iface.name.len()].copy_from_slice(iface.name.as_bytes());
                        entry[16..32].copy_from_slice(&sockaddr_in(ip));
                        out.push(entry);
                    }
                }
                let fits = if buf == 0 { 0 } else { (len as usize / 40).min(out.len()) };
                let bytes: Vec<u8> = out[..fits].concat();
                let written = if buf == 0 { out.len() * 40 } else { bytes.len() };
                self.mem(cx, true, |t| {
                    if !bytes.is_empty() {
                        user::write(&mut t.space(), buf, &bytes)?;
                    }
                    user::write_u32(&mut t.space(), arg, written as u32)
                })?;
                Ok(0)
            }
            SIOCGIFNAME | SIOCGIFFLAGS | SIOCGIFADDR | SIOCGIFBRDADDR | SIOCGIFNETMASK | SIOCGIFMTU | SIOCGIFHWADDR | SIOCGIFINDEX => {
                let mut ifreq = self.mem(cx, false, |t| user::read_vec(&t.space(), arg, 40))?;
                let iface = if request == SIOCGIFNAME {
                    let index = u32::from_le_bytes([ifreq[16], ifreq[17], ifreq[18], ifreq[19]]);
                    interfaces.iter().find(|i| i.index == index)
                } else {
                    let name = &ifreq[..16];
                    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(16)];
                    interfaces.iter().find(|i| i.name.as_bytes() == name)
                }
                .ok_or(ENODEV)?;
                let field: Vec<u8> = match request {
                    SIOCGIFNAME => {
                        ifreq[..16].fill(0);
                        ifreq[..iface.name.len()].copy_from_slice(iface.name.as_bytes());
                        Vec::new()
                    }
                    SIOCGIFFLAGS => {
                        let flags: u16 = if iface.loopback { 0x1 | 0x8 | 0x40 } else { 0x1 | 0x2 | 0x40 | 0x1000 };
                        flags.to_le_bytes().to_vec()
                    }
                    SIOCGIFADDR => sockaddr_in(v4(iface).ok_or(EADDRNOTAVAIL)?.0),
                    SIOCGIFNETMASK => {
                        let prefix = v4(iface).ok_or(EADDRNOTAVAIL)?.1;
                        sockaddr_in(Ipv4Addr::from(u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)))
                    }
                    SIOCGIFBRDADDR => {
                        let (ip, prefix) = v4(iface).ok_or(EADDRNOTAVAIL)?;
                        sockaddr_in(Ipv4Addr::from(u32::from(ip) | u32::MAX.checked_shr(u32::from(prefix)).unwrap_or(0)))
                    }
                    SIOCGIFMTU => iface.mtu.to_le_bytes().to_vec(),
                    SIOCGIFINDEX => iface.index.to_le_bytes().to_vec(),
                    _ => {
                        // SIOCGIFHWADDR: a sockaddr with ARPHRD_ETHER (or loopback's) and the address.
                        let mut b = (if iface.loopback { 772u16 } else { 1u16 }).to_le_bytes().to_vec();
                        b.extend(iface.mac);
                        b.resize(16, 0);
                        b
                    }
                };
                ifreq[16..16 + field.len()].copy_from_slice(&field);
                self.mem(cx, true, |t| user::write(&mut t.space(), arg, &ifreq))?;
                Ok(0)
            }
            _ => Err(ENOTTY),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockaddrs_both_ways() {
        let v4 = Addr::Inet("10.89.0.2:443".parse().unwrap());
        assert_eq!(parse_sockaddr(&encode(&v4)), Ok(Given::Inet("10.89.0.2:443".parse().unwrap())));
        let v6 = Addr::Inet("[::1]:8080".parse().unwrap());
        assert_eq!(encode(&v6).len(), 28);
        assert_eq!(parse_sockaddr(&encode(&v6)), Ok(Given::Inet("[::1]:8080".parse().unwrap())));
        let path = Addr::Unix(UnixAddr { name: UnixName::Path(b"/run/x.sock".to_vec()), file: Some(3) });
        assert_eq!(parse_sockaddr(&encode(&path)), Ok(Given::Unix(UnixName::Path(b"/run/x.sock".to_vec()))));
        let abs = Addr::Unix(UnixAddr { name: UnixName::Abstract(b"bus\0x".to_vec()), file: None });
        assert_eq!(parse_sockaddr(&encode(&abs)), Ok(Given::Unix(UnixName::Abstract(b"bus\0x".to_vec()))));
        assert_eq!(parse_sockaddr(&[1, 0]), Ok(Given::Unix(UnixName::Unnamed)));
        assert_eq!(parse_sockaddr(&encode_netlink(7)), Ok(Given::Netlink(7)));
        assert_eq!(parse_sockaddr(&[2, 0, 0, 80]), Err(EINVAL));
        assert_eq!(parse_sockaddr(&[99, 0, 0, 0]), Err(EAFNOSUPPORT));
    }
}
