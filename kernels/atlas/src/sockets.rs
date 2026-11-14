//! Winsock's sockets, over the machine's network (`net`). A socket is a file, as AFD makes it on Windows
//! (`Node::Socket`): so its handle closes, joins a completion port and takes `SetFileCompletionNotificationModes` as
//! any file's does.
//!
//! `net` never waits; atlas does. A blocking call tries, and if `net` would block, waits on its thread's waiter, which
//! a watch on the socket wakes whenever its readiness may have changed. An overlapped call tries once at once and, if
//! it would wait, is queued on its socket; the process's network thread (made the first time anything's queued, and
//! started at ws2_32's `AtlasNetPump`) wakes with the socket's readiness, finishes what it can, and writes the results
//! into the program's buffers and OVERLAPPED, as only a thread of the process with a CPU can, then tells the event,
//! the completion port or the routine. Windows has no such thread: its I/O finishes in the kernel. Programs see one
//! more thread than they made.

use std::collections::VecDeque;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::wait::{Waiter, Woke};

use crate::Kernel;
use crate::files::File;
use crate::os::Reply;
use crate::ports::Op;
use crate::process::{Process, Task};

pub const WSA_OPERATION_ABORTED: u32 = 995;
pub const WSA_IO_PENDING: u32 = 997;
pub const WSAEACCES: u32 = 10013;
pub const WSAEFAULT: u32 = 10014;
pub const WSAEINVAL: u32 = 10022;
pub const WSAEWOULDBLOCK: u32 = 10035;
pub const WSAEALREADY: u32 = 10037;
pub const WSAENOTSOCK: u32 = 10038;
pub const WSAEDESTADDRREQ: u32 = 10039;
pub const WSAEMSGSIZE: u32 = 10040;
pub const WSAEPROTONOSUPPORT: u32 = 10043;
pub const WSAESOCKTNOSUPPORT: u32 = 10044;
pub const WSAEOPNOTSUPP: u32 = 10045;
pub const WSAEAFNOSUPPORT: u32 = 10047;
pub const WSAEADDRINUSE: u32 = 10048;
pub const WSAEADDRNOTAVAIL: u32 = 10049;
pub const WSAENETDOWN: u32 = 10050;
pub const WSAENETUNREACH: u32 = 10051;
pub const WSAECONNABORTED: u32 = 10053;
pub const WSAECONNRESET: u32 = 10054;
pub const WSAEISCONN: u32 = 10056;
pub const WSAENOTCONN: u32 = 10057;
pub const WSAESHUTDOWN: u32 = 10058;
pub const WSAETIMEDOUT: u32 = 10060;
pub const WSAECONNREFUSED: u32 = 10061;
pub const WSAEHOSTUNREACH: u32 = 10065;
pub const WSANOTINITIALISED: u32 = 10093;
pub const WSAHOST_NOT_FOUND: u32 = 11001;

pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 23;
pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;

/// A `net` error as a WSA one.
pub fn wsa(e: &net::Error) -> u32 {
    use net::Error as E;
    match e {
        E::WouldBlock | E::InProgress => WSAEWOULDBLOCK,
        E::Already => WSAEALREADY,
        E::IsConnected => WSAEISCONN,
        E::NotConnected => WSAENOTCONN,
        E::ConnectionRefused => WSAECONNREFUSED,
        E::ConnectionReset => WSAECONNRESET,
        E::ConnectionAborted => WSAECONNABORTED,
        E::BrokenPipe => WSAESHUTDOWN,
        E::AddrInUse => WSAEADDRINUSE,
        E::AddrNotAvailable => WSAEADDRNOTAVAIL,
        E::NetworkUnreachable => WSAENETUNREACH,
        E::HostUnreachable => WSAEHOSTUNREACH,
        E::TimedOut => WSAETIMEDOUT,
        E::Denied => WSAEACCES,
        E::Invalid => WSAEINVAL,
        E::NotSupported => WSAEOPNOTSUPP,
        E::WrongFamily => WSAEAFNOSUPPORT,
        E::MessageTooBig => WSAEMSGSIZE,
        E::DestinationRequired => WSAEDESTADDRREQ,
        E::NotFound => WSAHOST_NOT_FOUND,
        E::Other(_) => WSAENETDOWN,
    }
}

/// A socket's address as a `sockaddr_in` (16 bytes) or `sockaddr_in6` (28).
pub fn sockaddr(addr: &SocketAddr) -> Vec<u8> {
    match addr {
        SocketAddr::V4(a) => {
            let mut b = AF_INET.to_le_bytes().to_vec();
            b.extend(a.port().to_be_bytes());
            b.extend(a.ip().octets());
            b.extend([0; 8]);
            b
        }
        SocketAddr::V6(a) => {
            let mut b = AF_INET6.to_le_bytes().to_vec();
            b.extend(a.port().to_be_bytes());
            b.extend(a.flowinfo().to_be_bytes());
            b.extend(a.ip().octets());
            b.extend(a.scope_id().to_le_bytes());
            b
        }
    }
}

/// A `net` address as a `sockaddr`: an IP one's, or a family alone for any other.
pub fn net_sockaddr(addr: &net::Addr) -> Vec<u8> {
    match addr {
        net::Addr::Inet(a) => sockaddr(a),
        net::Addr::Unix(_) => {
            let mut b = 1u16.to_le_bytes().to_vec();
            b.resize(110, 0);
            b
        }
    }
}

/// A `sockaddr` a program passed: IPv4's or IPv6's.
pub fn parse_sockaddr(b: &[u8]) -> Result<SocketAddr, u32> {
    if b.len() < 2 {
        return Err(WSAEFAULT);
    }
    let port = |b: &[u8]| u16::from_be_bytes([b[2], b[3]]);
    match u16::from_le_bytes([b[0], b[1]]) {
        AF_INET if b.len() >= 8 => Ok(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(b[4], b[5], b[6], b[7]), port(b)))),
        AF_INET6 if b.len() >= 24 => {
            let ip: [u8; 16] = b[8..24].try_into().map_err(|_| WSAEFAULT)?;
            let flow = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
            let scope = if b.len() >= 28 { u32::from_le_bytes([b[24], b[25], b[26], b[27]]) } else { 0 };
            Ok(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(ip), port(b), flow, scope)))
        }
        AF_INET | AF_INET6 => Err(WSAEFAULT),
        _ => Err(WSAEAFNOSUPPORT),
    }
}

/// One process's overlapped socket I/O: the sockets with some under way, and its network thread's waiter.
#[derive(Default)]
pub(crate) struct Pump {
    pub waiter: Option<Arc<Waiter>>,
    pub socks: Vec<Weak<Sock>>,
    pub started: bool,
}

/// What an overlapped operation still has to do.
pub(crate) enum Work {
    /// `WSARecv(From)`: into these buffers, and the sender's address (with its length's) for `WSARecvFrom`.
    Recv { bufs: Vec<(u64, u32)>, peek: bool, from: Option<(u64, u64)> },
    /// `WSASend(To)`: what's left to send, and how much has gone.
    Send { data: Vec<u8>, sent: usize, to: Option<net::Addr> },
    /// `ConnectEx`: connecting, then sending what it was given.
    Connect { then: Vec<u8> },
    /// `AcceptEx`: a connection for the socket `into`; its first bytes (up to `recv`) and both addresses into
    /// `buffer`, as `GetAcceptExSockaddrs` reads them.
    Accept { into: Arc<File>, buffer: u64, recv: u32, local_len: u32, addresses: Option<Vec<(u64, Vec<u8>)>> },
}

pub(crate) struct Pending {
    pub op: Arc<Op>,
    pub work: Work,
}

/// How an operation came out: what to write where in the program, how many bytes, and the WSA error.
pub(crate) struct Outcome {
    pub writes: Vec<(u64, Vec<u8>)>,
    pub n: u64,
    pub error: u32,
}

impl Outcome {
    fn done(n: usize) -> Outcome {
        Outcome { writes: Vec::new(), n: n as u64, error: 0 }
    }

    fn failed(error: u32) -> Outcome {
        Outcome { writes: Vec::new(), n: 0, error }
    }
}

struct Current {
    socket: Arc<net::Socket>,
    _watch: net::Watch,
}

/// A Winsock socket.
pub(crate) struct Sock {
    current: Mutex<Current>,
    pub family: u16,
    pub kind: u32,
    pub protocol: u32,
    /// `FIONBIO`'s.
    pub nonblocking: AtomicBool,
    /// SO_RCVTIMEO and SO_SNDTIMEO, in milliseconds (0: none).
    pub timeouts: Mutex<[u32; 2]>,
    /// Options atlas doesn't act on, by level and name, kept for `getsockopt`.
    pub kept: Mutex<HashMap<(u32, u32), Vec<u8>>>,
    /// Overlapped receives (and accepts) under way, and sends (and connects), each in order.
    reads: Mutex<VecDeque<Pending>>,
    writes: Mutex<VecDeque<Pending>>,
    process: Weak<Process>,
    /// Listening (`listen`), and a non-blocking connect under way: what FD_ACCEPT and FD_CONNECT wait for.
    pub listening: AtomicBool,
    pub connecting: AtomicBool,
    /// `WSAEventSelect`'s.
    pub selected: Mutex<Option<Selected>>,
}

/// FD_READ, FD_WRITE, FD_OOB, FD_ACCEPT, FD_CONNECT and FD_CLOSE: the network events `WSAEventSelect` records.
pub const FD_READ: u32 = 1;
pub const FD_WRITE: u32 = 2;
pub const FD_ACCEPT: u32 = 8;
pub const FD_CONNECT: u32 = 16;
pub const FD_CLOSE: u32 = 32;

/// What `WSAEventSelect` asked of a socket: its event, the network events wanted, those recorded since
/// `WSAEnumNetworkEvents` last took them (with an error code for each), and those that may be recorded again. Each is
/// recorded once, then waits to be re-enabled, as Winsock's are: FD_READ by a receive, FD_ACCEPT by an accept, FD_WRITE
/// by a send that would block; FD_CONNECT and FD_CLOSE come once.
pub(crate) struct Selected {
    pub event: Arc<crate::objects::Object>,
    pub mask: u32,
    pub recorded: u32,
    pub errors: [u32; 10],
    pub armed: u32,
}

/// A watch that wakes the process's network thread, which looks at whatever's queued.
fn waking(process: &Weak<Process>, socket: &net::Socket) -> net::Watch {
    let process = process.clone();
    socket.watch(Arc::new(move || {
        if let Some(p) = process.upgrade()
            && let Some(w) = lock(&p.net).waiter.clone()
        {
            w.wake();
        }
    }))
}

impl Sock {
    pub fn new(process: &Arc<Process>, socket: net::Socket, family: u16, kind: u32, protocol: u32) -> Arc<Sock> {
        let process = Arc::downgrade(process);
        let watch = waking(&process, &socket);
        Arc::new(Sock {
            current: Mutex::new(Current { socket: Arc::new(socket), _watch: watch }),
            family,
            kind,
            protocol,
            nonblocking: AtomicBool::new(false),
            timeouts: Mutex::default(),
            kept: Mutex::default(),
            reads: Mutex::default(),
            writes: Mutex::default(),
            process,
            listening: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            selected: Mutex::new(None),
        })
    }

    /// The `net` socket it is now (`AcceptEx` gives a socket another).
    pub fn net(&self) -> Arc<net::Socket> {
        lock(&self.current).socket.clone()
    }

    fn replace(&self, socket: net::Socket) {
        let watch = waking(&self.process, &socket);
        *lock(&self.current) = Current { socket: Arc::new(socket), _watch: watch };
    }

    pub fn is_stream(&self) -> bool {
        self.kind == SOCK_STREAM
    }

    /// Queues an overlapped operation, for the network thread.
    pub fn queue(&self, read: bool, pending: Pending) {
        lock(if read { &self.reads } else { &self.writes }).push_back(pending);
    }

    /// Whether anything's queued, or it has an event selected: what the network thread looks after.
    pub fn busy(&self) -> bool {
        !lock(&self.reads).is_empty() || !lock(&self.writes).is_empty() || lock(&self.selected).is_some()
    }

    /// Network events that may be recorded again (a receive re-enables FD_READ, and so on).
    pub fn rearm(&self, events: u32) {
        if let Some(s) = lock(&self.selected).as_mut() {
            s.armed |= events;
        }
    }

    /// Records the selected network events the socket's readiness now says, those that may be, and sets the event if
    /// any were.
    fn evaluate(&self, k: &Kernel) {
        use std::sync::atomic::Ordering::Relaxed;
        let socket = self.net();
        let ready = socket.ready();
        let (listening, connecting) = (self.listening.load(Relaxed), self.connecting.load(Relaxed));
        let mut now = Vec::new();
        if connecting && (ready.write || ready.error || ready.hangup) {
            self.connecting.store(false, Relaxed);
            let error = socket.take_error().map_or(0, |e| wsa(&e));
            now.push((FD_CONNECT, 4, error));
            if error != 0 {
                now.push((FD_CLOSE, 5, error));
            }
        }
        let connecting = self.connecting.load(Relaxed);
        if listening {
            if ready.read {
                now.push((FD_ACCEPT, 3, 0));
            }
        } else if !connecting {
            if ready.read {
                now.push((FD_READ, 0, 0));
            }
            // A socket that's neither connected nor connecting says it's hung up: it can't write, or close.
            if ready.write && !ready.hangup {
                now.push((FD_WRITE, 1, 0));
            }
            if self.is_stream() && ready.read && (ready.read_closed || ready.hangup) {
                now.push((FD_CLOSE, 5, if ready.error { WSAECONNRESET } else { 0 }));
            }
        }
        let event = {
            let mut selected = lock(&self.selected);
            let Some(s) = selected.as_mut() else { return };
            let mut any = false;
            for (bit, index, error) in now {
                if s.mask & s.armed & bit != 0 {
                    s.armed &= !bit;
                    s.recorded |= bit;
                    s.errors[index] = error;
                    any = true;
                }
            }
            any.then(|| s.event.clone())
        };
        if let Some(event) = event {
            k.objects.signal(&event, |s| s.set = true);
        }
    }

    /// Takes every queued operation (a socket closing), each to be finished as aborted.
    pub fn take_all(&self) -> Vec<Pending> {
        let mut all: Vec<Pending> = lock(&self.reads).drain(..).collect();
        all.extend(lock(&self.writes).drain(..));
        all
    }

    /// Tries `work`: None if it would wait.
    pub fn attempt(&self, work: &mut Work) -> Option<Outcome> {
        let socket = self.net();
        match work {
            Work::Recv { bufs, peek, from } => {
                let total: usize = bufs.iter().map(|b| b.1 as usize).sum();
                match socket.recv(total, *peek) {
                    Ok(r) => {
                        let mut writes = Vec::new();
                        let mut at = 0;
                        for &(buf, len) in bufs.iter() {
                            if at >= r.data.len() {
                                break;
                            }
                            let end = (at + len as usize).min(r.data.len());
                            writes.push((buf, r.data[at..end].to_vec()));
                            at = end;
                        }
                        if let (Some((addr_at, len_at)), Some(f)) = (from, &r.from)
                            && *addr_at != 0
                        {
                            let bytes = net_sockaddr(f);
                            if *len_at != 0 {
                                writes.push((*len_at, (bytes.len() as u32).to_le_bytes().to_vec()));
                            }
                            writes.push((*addr_at, bytes));
                        }
                        let error = if r.full > r.data.len() { WSAEMSGSIZE } else { 0 };
                        Some(Outcome { writes, n: r.data.len() as u64, error })
                    }
                    Err(net::Error::WouldBlock) => None,
                    Err(e) => Some(Outcome::failed(wsa(&e))),
                }
            }
            Work::Send { data, sent, to } => loop {
                if *sent >= data.len() && !(data.is_empty() && *sent == 0) {
                    return Some(Outcome::done(*sent));
                }
                match socket.send(&data[*sent..], to.as_ref(), &mut None) {
                    Ok(n) => {
                        *sent += n;
                        if data.is_empty() || !self.is_stream() {
                            return Some(Outcome::done(*sent));
                        }
                    }
                    Err(net::Error::WouldBlock) => return None,
                    Err(e) => return Some(Outcome { writes: Vec::new(), n: *sent as u64, error: wsa(&e) }),
                }
            },
            Work::Connect { then } => {
                let ready = socket.ready();
                if !(ready.write || ready.error || ready.hangup) {
                    return None;
                }
                if let Some(e) = socket.take_error() {
                    return Some(Outcome::failed(wsa(&e)));
                }
                if then.is_empty() {
                    return Some(Outcome::done(0));
                }
                match socket.send(then, None, &mut None) {
                    Ok(n) => Some(Outcome::done(n)),
                    Err(net::Error::WouldBlock) => None,
                    Err(e) => Some(Outcome::failed(wsa(&e))),
                }
            }
            Work::Accept { into, buffer, recv, local_len, addresses } => {
                let crate::files::Node::Socket(target) = &into.node else { return Some(Outcome::failed(WSAENOTSOCK)) };
                if addresses.is_none() {
                    match socket.accept() {
                        Ok((s, peer)) => {
                            let local = s.local();
                            target.replace(s);
                            let at = *buffer + u64::from(*recv);
                            *addresses = Some(vec![(at, net_sockaddr(&local)), (at + u64::from(*local_len), net_sockaddr(&peer))]);
                        }
                        Err(net::Error::WouldBlock) => return None,
                        Err(e) => return Some(Outcome::failed(wsa(&e))),
                    }
                }
                let mut writes = addresses.clone().unwrap_or_default();
                if *recv == 0 {
                    return Some(Outcome { writes, n: 0, error: 0 });
                }
                // With room for bytes, it's done once the first come, or the connection's over.
                match target.net().recv(*recv as usize, false) {
                    Ok(r) => {
                        let n = r.data.len() as u64;
                        writes.push((*buffer, r.data));
                        Some(Outcome { writes, n, error: 0 })
                    }
                    Err(net::Error::WouldBlock) => None,
                    Err(e) => Some(Outcome { writes, n: 0, error: wsa(&e) }),
                }
            }
        }
    }

    /// Finishes what it can of its queued operations. Whether it finished any.
    fn progress(&self, k: &Kernel, cx: &mut Cx<'_>) -> bool {
        let mut any = false;
        for queue in [&self.reads, &self.writes] {
            loop {
                let done = {
                    let mut q = lock(queue);
                    let Some(front) = q.front_mut() else { break };
                    match self.attempt(&mut front.work) {
                        Some(outcome) => (q.pop_front().map(|p| p.op), outcome),
                        None => break,
                    }
                };
                if let (Some(op), outcome) = done {
                    finish(k, cx, &op, &outcome);
                    any = true;
                }
            }
        }
        any
    }
}

/// Finishes an operation: its results into the program, its OVERLAPPED, then its event, port or routine.
pub(crate) fn finish(k: &Kernel, cx: &mut Cx<'_>, op: &Op, outcome: &Outcome) {
    let writes: Vec<(u64, &[u8])> = outcome.writes.iter().map(|(at, b)| (*at, &b[..])).collect();
    k.finish(cx, op, &writes, outcome.n, outcome.error);
}

/// Why a blocking socket call stopped without its answer.
pub(crate) enum Stopped {
    /// A WSA error.
    Error(u32),
    /// The thread or its process is ending.
    Ended,
}

impl Task {
    /// Waits for `attempt` to get its answer, waking whenever one of `sockets`' readiness may have changed: Ok(None)
    /// if `timeout` passes first.
    pub(crate) fn sock_wait<R>(&mut self, sockets: &[Arc<net::Socket>], timeout: Option<Duration>, mut attempt: impl FnMut(&mut Task) -> Result<Option<R>, u32>) -> Result<Option<R>, Stopped> {
        let deadline = timeout.map(|t| Instant::now() + t);
        let watches: Vec<net::Watch> = sockets
            .iter()
            .map(|s| {
                let w = self.waiter.clone();
                s.watch(Arc::new(move || w.wake()))
            })
            .collect();
        let result = loop {
            self.waiter.reset();
            match attempt(self) {
                Ok(Some(r)) => break Ok(Some(r)),
                Ok(None) => {}
                Err(e) => break Err(Stopped::Error(e)),
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                break Ok(None);
            }
            if self.waiter.wait(deadline) == Woke::Interrupted && self.ending() {
                break Err(Stopped::Ended);
            }
        };
        drop(watches);
        result
    }

    /// Hands a socket's queued operation to the process's network thread, starting it the first time.
    pub(crate) fn net_queue(&mut self, cx: &mut Cx<'_>, sock: &Arc<Sock>, read: bool, pending: Pending) {
        sock.queue(read, pending);
        self.net_watch(cx, sock);
    }

    /// Has the process's network thread look after a socket (starting it the first time), and wakes it.
    pub(crate) fn net_watch(&mut self, cx: &mut Cx<'_>, sock: &Arc<Sock>) {
        let start = {
            let mut pump = lock(&self.process.net);
            if !pump.socks.iter().any(|w| w.as_ptr() == Arc::as_ptr(sock)) {
                pump.socks.push(Arc::downgrade(sock));
            }
            if let Some(w) = &pump.waiter {
                w.wake();
            }
            !std::mem::replace(&mut pump.started, true)
        };
        if start && let Some(at) = self.system_export("ws2_32.dll", "AtlasNetPump") {
            let _ = self.create_thread(cx, 0, at, 0, 0);
        }
    }
}

/// The process's network thread, which ws2_32's `AtlasNetPump` is: it finishes queued overlapped socket I/O as each
/// socket can go on, and waits in between, until the process ends.
pub(crate) fn pump(task: &mut Task, cx: &mut Cx<'_>) -> Reply {
    lock(&task.process.net).waiter = Some(task.waiter.clone());
    loop {
        task.waiter.reset();
        let socks: Vec<Arc<Sock>> = {
            let mut pump = lock(&task.process.net);
            pump.socks.retain(|w| w.upgrade().is_some_and(|s| s.busy()));
            pump.socks.iter().filter_map(Weak::upgrade).collect()
        };
        let k = task.k.clone();
        let mut any = false;
        for s in &socks {
            any |= s.progress(&k, cx);
            s.evaluate(&k);
        }
        if any {
            continue;
        }
        if task.waiter.wait(None) == Woke::Interrupted && task.ending() {
            return Reply::Ended;
        }
    }
}

/// What `getaddrinfo` finds: the addresses of a name (or of none: the passive or loopback ones), as `family` asks.
pub(crate) fn resolve(k: &Kernel, node: Option<&str>, family: u16, passive: bool, numeric: bool) -> Result<Vec<IpAddr>, u32> {
    let found: Vec<IpAddr> = match node {
        None => {
            if passive {
                vec![IpAddr::V6(Ipv6Addr::UNSPECIFIED), IpAddr::V4(Ipv4Addr::UNSPECIFIED)]
            } else {
                vec![IpAddr::V6(Ipv6Addr::LOCALHOST), IpAddr::V4(Ipv4Addr::LOCALHOST)]
            }
        }
        Some(name) => match name.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) if numeric => return Err(WSAHOST_NOT_FOUND),
            Err(_) => k.net.lookup(name).map_err(|e| wsa(&e))?,
        },
    };
    let wanted: Vec<IpAddr> = found
        .into_iter()
        .filter(|ip| match family {
            AF_INET => ip.is_ipv4(),
            AF_INET6 => ip.is_ipv6(),
            _ => true,
        })
        .collect();
    if wanted.is_empty() { Err(WSAHOST_NOT_FOUND) } else { Ok(wanted) }
}

/// A service's port: a number, or a well-known name.
pub(crate) fn service_port(service: &str) -> Option<u16> {
    if let Ok(n) = service.parse() {
        return Some(n);
    }
    Some(match service.to_ascii_lowercase().as_str() {
        "http" | "www" => 80,
        "https" => 443,
        "ftp" => 21,
        "ssh" => 22,
        "telnet" => 23,
        "smtp" => 25,
        "domain" => 53,
        "pop3" => 110,
        "ntp" => 123,
        "imap" => 143,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockaddrs_both_ways() {
        let v4: SocketAddr = "93.184.216.34:443".parse().unwrap();
        let b = sockaddr(&v4);
        assert_eq!(b.len(), 16);
        assert_eq!(&b[..8], &[2, 0, 1, 0xbb, 93, 184, 216, 34]);
        assert_eq!(parse_sockaddr(&b), Ok(v4));
        let v6: SocketAddr = "[2001:db8::1]:8080".parse().unwrap();
        let b = sockaddr(&v6);
        assert_eq!(b.len(), 28);
        assert_eq!(parse_sockaddr(&b), Ok(v6));
        assert_eq!(parse_sockaddr(&[9, 0, 0, 0, 0, 0, 0, 0]), Err(WSAEAFNOSUPPORT));
        assert_eq!(service_port("https"), Some(443));
        assert_eq!(service_port("8080"), Some(8080));
        assert_eq!(service_port("nope"), None);
    }
}
