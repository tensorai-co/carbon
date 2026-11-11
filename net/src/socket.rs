//! Sockets: one state machine for every kind. A stream socket is idle, listening, joined to another in memory
//! (Unix, or TCP over loopback), or connected out through the host; a datagram socket keeps a queue that other sockets
//! in the machine, the machine's DNS server and the host's UDP sockets fill.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::stream::{self, End};
use crate::watch::{Watch, Watchers, waker_for};
use crate::{Addr, Ancillary, Cred, Domain, Error, HostDatagram, HostStream, Kind, Place, Ready, Result, Shared, Shutdown, UnixAddr, UnixName, Waker, as_domain, dns, lock, v4_of};

/// The most datagrams a socket holds unread, and the most bytes.
const MOST_DATAGRAMS: usize = 512;
const DATAGRAM_BYTES: usize = 256 << 10;
/// The largest UDP datagram.
const MOST_UDP: usize = 65507;

/// Socket options `net` acts on, or keeps for the asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    pub reuse_addr: bool,
    pub reuse_port: bool,
    pub nodelay: bool,
    pub keepalive: bool,
    pub broadcast: bool,
    pub pass_cred: bool,
}

pub(crate) struct Datagram {
    pub from: Addr,
    pub data: Vec<u8>,
    pub extra: Option<Ancillary>,
    pub cred: Option<Cred>,
}

#[derive(Default)]
struct Datagrams {
    queue: VecDeque<Datagram>,
    bytes: usize,
    /// The host's UDP sockets behind this one, IPv4's and IPv6's, opened as they're first needed.
    host: [Option<Box<dyn HostDatagram>>; 2],
    /// The socket a Unix datagram socket sends to by default: `socketpair`'s other end, or what it connected to.
    target: Option<Weak<Inner>>,
}

enum Conn {
    Idle,
    Listening { queue: VecDeque<(Socket, Addr)> },
    Joined(End),
    Connecting(Box<dyn HostStream>),
    Out { stream: Box<dyn HostStream>, peeked: Vec<u8> },
    /// A connect that failed: its error waits in `State::error`.
    Failed,
    Datagrams(Datagrams),
}

struct State {
    local: Option<Addr>,
    peer: Option<Addr>,
    conn: Conn,
    options: Options,
    cred: Cred,
    peer_cred: Option<Cred>,
    /// What `SO_ERROR` says next.
    error: Option<Error>,
    /// A datagram socket's shutdown: no more reading, no more writing.
    shut: (bool, bool),
}

pub(crate) struct Inner {
    net: Arc<Shared>,
    pub domain: Domain,
    pub kind: Kind,
    pub id: u64,
    pub watchers: Arc<Watchers>,
    listening: AtomicBool,
    state: Mutex<State>,
}

/// What a read got.
pub struct Received {
    pub data: Vec<u8>,
    /// Who sent it, for a datagram.
    pub from: Option<Addr>,
    pub extra: Option<Ancillary>,
    /// Who wrote it, for a Unix socket.
    pub cred: Option<Cred>,
    /// How long the message was, if it was longer than what was asked for.
    pub full: usize,
}

/// TCP's states as `/proc/net/tcp` numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TcpState {
    Established = 1,
    SynSent = 2,
    Close = 7,
    Listen = 10,
}

/// A socket, for `/proc/net`.
#[derive(Debug, Clone)]
pub struct SocketInfo {
    pub id: u64,
    pub domain: Domain,
    pub kind: Kind,
    pub local: Option<Addr>,
    pub peer: Option<Addr>,
    pub state: TcpState,
    /// Bytes waiting to be read, and written but not yet taken.
    pub queued: (usize, usize),
}

/// A socket. Dropping it closes it.
pub struct Socket(pub(crate) Arc<Inner>);

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Socket({}, {:?} {:?})", self.0.id, self.0.domain, self.0.kind)
    }
}

fn unspecified(domain: Domain) -> IpAddr {
    if domain == Domain::Inet6 { IpAddr::V6(Ipv6Addr::UNSPECIFIED) } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback() || v4_of(ip).is_some_and(|v4| v4.is_loopback())
}

/// Two IP socket addresses the same, an IPv4 one and its IPv6 mapping alike.
fn same_inet(a: SocketAddr, b: SocketAddr) -> bool {
    let norm = |ip: IpAddr| v4_of(ip).map_or(ip, IpAddr::V4);
    a.port() == b.port() && norm(a.ip()) == norm(b.ip())
}

impl Inner {
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }

    /// Takes a datagram from another socket in the machine (or the DNS server). It's dropped if this one shut its
    /// reading, or is connected elsewhere.
    pub(crate) fn deliver(&self, mut datagram: Datagram, extra: &mut Option<Ancillary>) -> Result<()> {
        {
            let mut state = lock(&self.state);
            let (shut, peer) = (state.shut.0, state.peer.clone());
            let Conn::Datagrams(d) = &mut state.conn else { return Err(Error::ConnectionRefused) };
            if shut {
                return Ok(());
            }
            if let (Some(Addr::Inet(peer)), Addr::Inet(from)) = (peer, &datagram.from)
                && !same_inet(peer, *from)
            {
                return Ok(());
            }
            if d.queue.len() >= MOST_DATAGRAMS || d.bytes + datagram.data.len() > DATAGRAM_BYTES {
                return Err(Error::WouldBlock);
            }
            d.bytes += datagram.data.len();
            datagram.extra = extra.take();
            d.queue.push_back(datagram);
        }
        self.watchers.notify();
        Ok(())
    }

    pub fn info(&self) -> SocketInfo {
        let state = lock(&self.state);
        let (tcp, queued) = match &state.conn {
            Conn::Listening { queue } => (TcpState::Listen, (queue.len(), 0)),
            Conn::Joined(end) => (TcpState::Established, end.queued()),
            Conn::Out { peeked, .. } => (TcpState::Established, (peeked.len(), 0)),
            Conn::Connecting(_) => (TcpState::SynSent, (0, 0)),
            Conn::Datagrams(d) => (TcpState::Close, (d.bytes, 0)),
            Conn::Idle | Conn::Failed => (TcpState::Close, (0, 0)),
        };
        SocketInfo { id: self.id, domain: self.domain, kind: self.kind, local: state.local.clone(), peer: state.peer.clone(), state: tcp, queued }
    }
}

impl Socket {
    pub(crate) fn new(net: &Arc<Shared>, domain: Domain, kind: Kind) -> Socket {
        let conn = if kind == Kind::Datagram { Conn::Datagrams(Datagrams::default()) } else { Conn::Idle };
        let inner = Arc::new(Inner {
            net: net.clone(),
            domain,
            kind,
            id: net.next_id.fetch_add(1, Ordering::Relaxed),
            watchers: Arc::default(),
            listening: AtomicBool::new(false),
            state: Mutex::new(State { local: None, peer: None, conn, options: Options::default(), cred: Cred::default(), peer_cred: None, error: None, shut: (false, false) }),
        });
        lock(&net.all).push(Arc::downgrade(&inner));
        Socket(inner)
    }

    pub(crate) fn pair(net: &Arc<Shared>, kind: Kind) -> Result<(Socket, Socket)> {
        let (a, b) = (Socket::new(net, Domain::Unix, kind), Socket::new(net, Domain::Unix, kind));
        let unnamed = Some(Addr::Unix(UnixAddr::UNNAMED));
        if kind == Kind::Datagram {
            for (me, other) in [(&a, &b), (&b, &a)] {
                let mut state = lock(&me.0.state);
                if let Conn::Datagrams(d) = &mut state.conn {
                    d.target = Some(Arc::downgrade(&other.0));
                }
                state.peer = unnamed.clone();
            }
        } else {
            let (ea, eb) = stream::join([a.0.watchers.clone(), b.0.watchers.clone()], kind == Kind::SeqPacket, false);
            for (me, end) in [(&a, ea), (&b, eb)] {
                let mut state = lock(&me.0.state);
                state.conn = Conn::Joined(end);
                state.peer = unnamed.clone();
            }
        }
        for s in [&a, &b] {
            lock(&s.0.state).local = unnamed.clone();
        }
        Ok((a, b))
    }

    pub fn domain(&self) -> Domain {
        self.0.domain
    }

    pub fn kind(&self) -> Kind {
        self.0.kind
    }

    /// A number for this socket, unique in its network: its inode, as programs see it.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    fn net(&self) -> &Arc<Shared> {
        &self.0.net
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.0.state)
    }

    /// Whom it was made by, for the other end's `SO_PEERCRED`.
    pub fn set_cred(&self, cred: Cred) {
        self.state().cred = cred;
    }

    pub fn peer_cred(&self) -> Option<Cred> {
        self.state().peer_cred
    }

    pub fn options(&self) -> Options {
        self.state().options
    }

    pub fn set_options(&self, change: impl FnOnce(&mut Options)) {
        let mut state = self.state();
        change(&mut state.options);
        let options = state.options;
        if let Conn::Out { stream, .. } | Conn::Connecting(stream) = &state.conn {
            stream.set_nodelay(options.nodelay);
            stream.set_keepalive(options.keepalive);
        }
    }

    /// Calls `waker` whenever [`Socket::ready`] may have changed, until the [`Watch`] is dropped.
    pub fn watch(&self, waker: Waker) -> Watch {
        self.0.watchers.add(waker)
    }

    /// The error waiting (`SO_ERROR`), which reading it clears.
    pub fn take_error(&self) -> Option<Error> {
        let mut state = self.state();
        settle(&mut state);
        state.error.take()
    }

    fn check_family(&self, addr: SocketAddr) -> Result<()> {
        match (self.0.domain, addr) {
            (Domain::Inet, SocketAddr::V4(_)) | (Domain::Inet6, SocketAddr::V6(_)) => Ok(()),
            _ => Err(Error::WrongFamily),
        }
    }

    pub fn bind(&self, addr: &Addr) -> Result<()> {
        match (self.0.domain, addr) {
            (Domain::Unix, Addr::Unix(unix)) => {
                if self.state().local.as_ref().is_some_and(|a| *a != Addr::Unix(UnixAddr::UNNAMED)) {
                    return Err(Error::Invalid);
                }
                let addr = if unix.name == UnixName::Unnamed { self.autobind_name() } else { unix.clone() };
                let key = addr.key().ok_or(Error::Invalid)?;
                {
                    let mut names = lock(&self.net().unix);
                    if names.get(&key).is_some_and(|w| w.strong_count() > 0) {
                        return Err(Error::AddrInUse);
                    }
                    names.insert(key, Arc::downgrade(&self.0));
                }
                self.state().local = Some(Addr::Unix(addr));
                Ok(())
            }
            (Domain::Inet | Domain::Inet6, Addr::Inet(to)) => {
                self.check_family(*to)?;
                if !self.net().bindable(to.ip()) {
                    return Err(Error::AddrNotAvailable);
                }
                let options = {
                    let state = self.state();
                    if state.local.is_some() {
                        return Err(Error::Invalid);
                    }
                    state.options
                };
                let port = self.net().take_port(self.0.kind == Kind::Stream, to.ip(), to.port(), &self.0, options.reuse_addr, options.reuse_port)?;
                self.state().local = Some(Addr::Inet(SocketAddr::new(to.ip(), port)));
                Ok(())
            }
            _ => Err(Error::WrongFamily),
        }
    }

    /// A free name in the abstract namespace, as Linux gives a Unix socket bound with no name.
    fn autobind_name(&self) -> UnixAddr {
        let names = lock(&self.net().unix);
        let mut n = self.0.id;
        loop {
            let name = format!("{:05x}", n & 0xFFFFF).into_bytes();
            let addr = UnixAddr { name: UnixName::Abstract(name), file: None };
            if !names.get(&addr.key().unwrap_or_else(|| unreachable!())).is_some_and(|w| w.strong_count() > 0) {
                return addr;
            }
            n += 1;
        }
    }

    /// The address it's bound to, binding any address and a free port first if it isn't.
    fn ensure_bound(&self) -> Result<SocketAddr> {
        let options = {
            let state = self.state();
            if let Some(Addr::Inet(addr)) = state.local {
                return Ok(addr);
            }
            state.options
        };
        let ip = unspecified(self.0.domain);
        let port = self.net().take_port(self.0.kind == Kind::Stream, ip, 0, &self.0, options.reuse_addr, options.reuse_port)?;
        let addr = SocketAddr::new(ip, port);
        self.state().local = Some(Addr::Inet(addr));
        Ok(addr)
    }

    pub fn listen(&self, _backlog: u32) -> Result<()> {
        if self.0.kind == Kind::Datagram {
            return Err(Error::NotSupported);
        }
        match self.0.domain {
            Domain::Unix if self.state().local.as_ref().is_none_or(|a| *a == Addr::Unix(UnixAddr::UNNAMED)) => return Err(Error::Invalid),
            Domain::Unix => {}
            Domain::Inet | Domain::Inet6 => {
                self.ensure_bound()?;
            }
        }
        let mut state = self.state();
        match state.conn {
            Conn::Idle => state.conn = Conn::Listening { queue: VecDeque::new() },
            Conn::Listening { .. } => {}
            _ => return Err(Error::Invalid),
        }
        self.0.listening.store(true, Ordering::Release);
        Ok(())
    }

    /// Connects: at once inside the machine, or [`Error::InProgress`] while a connection out is made.
    pub fn connect(&self, addr: &Addr) -> Result<()> {
        match (self.0.domain, addr) {
            (Domain::Unix, Addr::Unix(unix)) if self.0.kind == Kind::Datagram => {
                let key = unix.key().ok_or(Error::Invalid)?;
                let target = lock(&self.net().unix).get(&key).and_then(Weak::upgrade).filter(|t| t.kind == Kind::Datagram).ok_or(Error::ConnectionRefused)?;
                let mut state = self.state();
                if let Conn::Datagrams(d) = &mut state.conn {
                    d.target = Some(Arc::downgrade(&target));
                }
                state.peer = Some(addr.clone());
                Ok(())
            }
            (Domain::Unix, Addr::Unix(unix)) => self.join_unix(unix),
            (Domain::Inet | Domain::Inet6, Addr::Inet(to)) => {
                self.check_family(*to)?;
                if self.0.kind == Kind::Datagram {
                    self.ensure_bound()?;
                    self.state().peer = Some(addr.clone());
                    return Ok(());
                }
                {
                    let mut state = self.state();
                    settle(&mut state);
                    match state.conn {
                        Conn::Idle | Conn::Failed => {}
                        Conn::Connecting(_) => return Err(Error::Already),
                        Conn::Joined(_) | Conn::Out { .. } => return Err(Error::IsConnected),
                        Conn::Listening { .. } | Conn::Datagrams(_) => return Err(Error::Invalid),
                    }
                }
                match self.net().place(to.ip()) {
                    Place::Local => self.join_tcp(*to),
                    Place::Gateway => Err(Error::ConnectionRefused),
                    Place::Subnet => Err(Error::HostUnreachable),
                    Place::Out => self.connect_out(*to),
                }
            }
            _ => Err(Error::WrongFamily),
        }
    }

    /// A datagram socket forgets its peer (`connect` with `AF_UNSPEC`).
    pub fn disconnect(&self) {
        let mut state = self.state();
        state.peer = None;
        if let Conn::Datagrams(d) = &mut state.conn {
            d.target = None;
        }
    }

    fn join_unix(&self, to: &UnixAddr) -> Result<()> {
        let key = to.key().ok_or(Error::Invalid)?;
        let listener = lock(&self.net().unix).get(&key).and_then(Weak::upgrade).ok_or(Error::ConnectionRefused)?;
        if listener.kind != self.0.kind || !listener.is_listening() {
            return Err(Error::ConnectionRefused);
        }
        let (my_addr, my_cred) = {
            let state = self.state();
            match state.conn {
                Conn::Idle => {}
                Conn::Joined(_) => return Err(Error::IsConnected),
                _ => return Err(Error::Invalid),
            }
            (state.local.clone().unwrap_or(Addr::Unix(UnixAddr::UNNAMED)), state.cred)
        };
        let (listener_addr, listener_cred) = {
            let state = lock(&listener.state);
            (state.local.clone().unwrap_or(Addr::Unix(UnixAddr::UNNAMED)), state.cred)
        };
        let server = Socket::new(self.net(), Domain::Unix, self.0.kind);
        let (mine, theirs) = stream::join([self.0.watchers.clone(), server.0.watchers.clone()], self.0.kind == Kind::SeqPacket, false);
        {
            let mut state = server.state();
            state.local = Some(listener_addr.clone());
            state.peer = Some(my_addr.clone());
            state.conn = Conn::Joined(theirs);
            state.cred = listener_cred;
            state.peer_cred = Some(my_cred);
        }
        self.queue_on(&listener, server, my_addr)?;
        let mut state = self.state();
        state.peer = Some(listener_addr);
        state.conn = Conn::Joined(mine);
        state.peer_cred = Some(listener_cred);
        Ok(())
    }

    /// Puts a new connection's server end on `listener`'s queue, for `accept`.
    fn queue_on(&self, listener: &Arc<Inner>, server: Socket, from: Addr) -> Result<()> {
        {
            let mut state = lock(&listener.state);
            let Conn::Listening { queue } = &mut state.conn else { return Err(Error::ConnectionRefused) };
            queue.push_back((server, from));
        }
        listener.watchers.notify();
        Ok(())
    }

    fn join_tcp(&self, to: SocketAddr) -> Result<()> {
        let bound = self.ensure_bound()?;
        let pure_v6 = v4_of(to.ip()).is_none();
        let listener = self
            .net()
            .bound(true, to.ip(), to.port())
            .into_iter()
            .find(|s| s.is_listening() && s.kind == Kind::Stream && (!pure_v6 || s.domain == Domain::Inet6))
            .ok_or(Error::ConnectionRefused)?;
        // This end's address: the loopback address dialed, or else the machine's own.
        let ip = if !bound.ip().is_unspecified() {
            bound.ip()
        } else if is_loopback(to.ip()) {
            to.ip()
        } else {
            IpAddr::V4(self.net().address)
        };
        let local = SocketAddr::new(ip, bound.port());
        let (my_cred, options) = {
            let state = self.state();
            (state.cred, state.options)
        };
        let (listener_cred, listener_options) = {
            let state = lock(&listener.state);
            (state.cred, state.options)
        };
        let server = Socket::new(self.net(), listener.domain, Kind::Stream);
        let (mine, theirs) = stream::join([self.0.watchers.clone(), server.0.watchers.clone()], false, true);
        let seen_from = Addr::Inet(as_domain(listener.domain, local));
        {
            let mut state = server.state();
            state.local = Some(Addr::Inet(as_domain(listener.domain, to)));
            state.peer = Some(seen_from.clone());
            state.conn = Conn::Joined(theirs);
            state.cred = listener_cred;
            state.peer_cred = Some(my_cred);
            state.options = Options { reuse_addr: false, reuse_port: false, ..listener_options };
        }
        self.queue_on(&listener, server, seen_from)?;
        let mut state = self.state();
        state.local = Some(Addr::Inet(as_domain(self.0.domain, local)));
        state.peer = Some(Addr::Inet(as_domain(self.0.domain, to)));
        state.conn = Conn::Joined(mine);
        state.peer_cred = Some(listener_cred);
        state.options = options;
        Ok(())
    }

    fn connect_out(&self, to: SocketAddr) -> Result<()> {
        let bound = self.ensure_bound()?;
        // The host connects to an IPv4 address as itself, mapped or not.
        let target = v4_of(to.ip()).map_or(to, |v4| SocketAddr::new(IpAddr::V4(v4), to.port()));
        let shown = match target {
            SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(self.net().address), bound.port()),
            SocketAddr::V6(_) => SocketAddr::new(unspecified(Domain::Inet6), bound.port()),
        };
        let stream = self.net().host.connect(target, waker_for(&self.0.watchers));
        let mut state = self.state();
        stream.set_nodelay(state.options.nodelay);
        stream.set_keepalive(state.options.keepalive);
        state.local = Some(Addr::Inet(as_domain(self.0.domain, shown)));
        state.peer = Some(Addr::Inet(as_domain(self.0.domain, to)));
        state.error = None;
        state.conn = Conn::Connecting(stream);
        settle(&mut state);
        match state.conn {
            Conn::Out { .. } => Ok(()),
            Conn::Failed => Err(state.error.take().unwrap_or(Error::ConnectionRefused)),
            _ => Err(Error::InProgress),
        }
    }

    pub fn accept(&self) -> Result<(Socket, Addr)> {
        let mut state = self.state();
        let Conn::Listening { queue } = &mut state.conn else { return Err(Error::Invalid) };
        queue.pop_front().ok_or(Error::WouldBlock)
    }

    /// Sends `data`, to `to` if it's a datagram socket that isn't connected. All or nothing for a datagram; for a
    /// stream, as much as there's room for. `extra` goes with it (and is taken) only if something's sent.
    pub fn send(&self, data: &[u8], to: Option<&Addr>, extra: &mut Option<Ancillary>) -> Result<usize> {
        if self.0.kind == Kind::Datagram {
            return self.send_datagram(data, to, extra);
        }
        let mut state = self.state();
        settle(&mut state);
        let cred = state.cred;
        match &state.conn {
            Conn::Joined(end) => end.send(data, extra, (self.0.domain == Domain::Unix).then_some(cred)),
            Conn::Out { stream, .. } => stream.write(data),
            Conn::Connecting(_) => Err(Error::WouldBlock),
            Conn::Failed => Err(state.error.take().unwrap_or(Error::NotConnected)),
            Conn::Idle | Conn::Listening { .. } | Conn::Datagrams(_) => Err(if self.0.domain == Domain::Unix { Error::NotConnected } else { Error::BrokenPipe }),
        }
    }

    fn send_datagram(&self, data: &[u8], to: Option<&Addr>, extra: &mut Option<Ancillary>) -> Result<usize> {
        let (peer, cred, target) = {
            let state = self.state();
            if state.shut.1 {
                return Err(Error::BrokenPipe);
            }
            let target = match &state.conn {
                Conn::Datagrams(d) => d.target.clone(),
                _ => None,
            };
            (state.peer.clone(), state.cred, target)
        };
        let dest = to.cloned().or(peer).ok_or(Error::DestinationRequired)?;
        match (self.0.domain, &dest) {
            (Domain::Unix, Addr::Unix(unix)) => {
                let target = match (to, target.and_then(|t| t.upgrade())) {
                    (None, Some(target)) => target,
                    _ => {
                        let key = unix.key().ok_or(Error::DestinationRequired)?;
                        lock(&self.net().unix).get(&key).and_then(Weak::upgrade).ok_or(Error::ConnectionRefused)?
                    }
                };
                if target.kind != Kind::Datagram {
                    return Err(Error::ConnectionRefused);
                }
                let from = self.state().local.clone().unwrap_or(Addr::Unix(UnixAddr::UNNAMED));
                target.deliver(Datagram { from, data: data.to_vec(), extra: None, cred: Some(cred) }, extra)?;
                Ok(data.len())
            }
            (Domain::Inet | Domain::Inet6, Addr::Inet(to)) => {
                let to = *to;
                self.check_family(to)?;
                if data.len() > MOST_UDP {
                    return Err(Error::MessageTooBig);
                }
                let bound = self.ensure_bound()?;
                match self.net().place(to.ip()) {
                    Place::Local => {
                        let ip = if !bound.ip().is_unspecified() {
                            bound.ip()
                        } else if is_loopback(to.ip()) {
                            to.ip()
                        } else {
                            IpAddr::V4(self.net().address)
                        };
                        let from = SocketAddr::new(ip, bound.port());
                        let pure_v6 = v4_of(to.ip()).is_none();
                        let targets = self.net().bound(false, to.ip(), to.port());
                        // Like UDP's own, a datagram nobody's waiting for is dropped.
                        if let Some(target) = targets.into_iter().find(|t| !pure_v6 || t.domain == Domain::Inet6) {
                            let _ = target.deliver(Datagram { from: Addr::Inet(as_domain(target.domain, from)), data: data.to_vec(), extra: None, cred: None }, &mut None);
                        }
                        Ok(data.len())
                    }
                    Place::Gateway if to.port() == 53 => {
                        let gateway = SocketAddr::new(IpAddr::V4(self.net().gateway), 53);
                        dns::answer(self.net(), data.to_vec(), Arc::downgrade(&self.0), as_domain(self.0.domain, gateway));
                        Ok(data.len())
                    }
                    Place::Gateway | Place::Subnet => Ok(data.len()),
                    Place::Out => {
                        let target = v4_of(to.ip()).map_or(to, |v4| SocketAddr::new(IpAddr::V4(v4), to.port()));
                        let v6 = target.is_ipv6();
                        let mut state = self.state();
                        let Conn::Datagrams(d) = &mut state.conn else { return Err(Error::Invalid) };
                        let slot = &mut d.host[usize::from(v6)];
                        if slot.is_none() {
                            *slot = Some(self.0.net.host.udp(v6, waker_for(&self.0.watchers))?);
                        }
                        slot.as_ref().unwrap_or_else(|| unreachable!()).send_to(data, target)
                    }
                }
            }
            _ => Err(Error::WrongFamily),
        }
    }

    /// Up to `len` bytes, or one datagram (cut to `len`). `peek` leaves it to be read again.
    pub fn recv(&self, len: usize, peek: bool) -> Result<Received> {
        let mut state = self.state();
        settle(&mut state);
        let domain = self.0.domain;
        let shut_read = state.shut.0;
        let peer = state.peer.clone();
        match &mut state.conn {
            Conn::Joined(end) => end.recv(len, peek).map(|got| Received { data: got.data, from: None, extra: got.extra, cred: got.cred, full: got.full }),
            Conn::Out { stream, peeked } => {
                if peeked.is_empty() {
                    let mut buffer = vec![0; len.clamp(1, 1 << 20)];
                    let n = stream.read(&mut buffer)?;
                    buffer.truncate(n);
                    if !peek {
                        return Ok(Received { full: n, data: buffer, from: None, extra: None, cred: None });
                    }
                    *peeked = buffer;
                }
                let n = len.min(peeked.len());
                let data = if peek { peeked[..n].to_vec() } else { peeked.drain(..n).collect() };
                Ok(Received { full: data.len(), data, from: None, extra: None, cred: None })
            }
            Conn::Connecting(_) => Err(Error::WouldBlock),
            Conn::Failed => Err(state.error.take().unwrap_or(Error::NotConnected)),
            Conn::Idle | Conn::Listening { .. } => Err(Error::NotConnected),
            Conn::Datagrams(d) => {
                // What the host has, first.
                for host in d.host.iter().flatten() {
                    for _ in 0..64 {
                        let mut buffer = vec![0; 65536];
                        let Ok((n, from)) = host.recv_from(&mut buffer) else { break };
                        if let Some(Addr::Inet(peer)) = &peer
                            && !same_inet(*peer, from)
                        {
                            continue;
                        }
                        buffer.truncate(n);
                        d.bytes += n;
                        d.queue.push_back(Datagram { from: Addr::Inet(as_domain(domain, from)), data: buffer, extra: None, cred: None });
                    }
                }
                let Some(front) = d.queue.front_mut() else {
                    return if shut_read { Ok(Received { data: Vec::new(), from: None, extra: None, cred: None, full: 0 }) } else { Err(Error::WouldBlock) };
                };
                let full = front.data.len();
                let data = front.data[..len.min(full)].to_vec();
                let (from, cred) = (front.from.clone(), front.cred);
                let extra = if peek {
                    None
                } else {
                    d.bytes -= full;
                    d.queue.pop_front().and_then(|f| f.extra)
                };
                Ok(Received { data, from: Some(from), extra, cred, full })
            }
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        let mut state = self.state();
        settle(&mut state);
        match &state.conn {
            Conn::Joined(end) => end.shutdown(how),
            Conn::Out { stream, .. } => stream.shutdown(how),
            Conn::Datagrams(_) => {
                state.shut.0 |= matches!(how, Shutdown::Read | Shutdown::Both);
                state.shut.1 |= matches!(how, Shutdown::Write | Shutdown::Both);
                drop(state);
                self.0.watchers.notify();
            }
            Conn::Idle | Conn::Listening { .. } | Conn::Connecting(_) | Conn::Failed => return Err(Error::NotConnected),
        }
        Ok(())
    }

    /// Its address (`getsockname`): any address and port 0 (or no name) if it isn't bound.
    pub fn local(&self) -> Addr {
        self.state().local.clone().unwrap_or(match self.0.domain {
            Domain::Unix => Addr::Unix(UnixAddr::UNNAMED),
            domain => Addr::Inet(SocketAddr::new(unspecified(domain), 0)),
        })
    }

    /// Who it's connected to (`getpeername`).
    pub fn peer(&self) -> Result<Addr> {
        let mut state = self.state();
        settle(&mut state);
        match state.conn {
            Conn::Joined(_) | Conn::Out { .. } => state.peer.clone().ok_or(Error::NotConnected),
            Conn::Datagrams(_) => state.peer.clone().ok_or(Error::NotConnected),
            _ => Err(Error::NotConnected),
        }
    }

    pub fn ready(&self) -> Ready {
        let mut state = self.state();
        settle(&mut state);
        let (shut, error) = (state.shut, state.error.is_some());
        match &state.conn {
            Conn::Idle => Ready { write: true, hangup: self.0.domain != Domain::Unix, ..Ready::default() },
            Conn::Listening { queue } => Ready { read: !queue.is_empty(), ..Ready::default() },
            Conn::Joined(end) => end.ready(),
            Conn::Connecting(_) => Ready::default(),
            Conn::Out { stream, peeked } => {
                let mut ready = stream.ready();
                ready.read |= !peeked.is_empty();
                ready
            }
            Conn::Failed => Ready { read: true, write: true, read_closed: true, hangup: true, error },
            Conn::Datagrams(d) => Ready {
                read: !d.queue.is_empty() || shut.0 || d.host.iter().flatten().any(|h| h.ready().read),
                write: !shut.1,
                read_closed: shut.0,
                hangup: shut.0 && shut.1,
                error,
            },
        }
    }

    /// What a read would get now (`FIONREAD`): bytes waiting, or the next datagram's length.
    pub fn available(&self) -> usize {
        let state = self.state();
        match &state.conn {
            Conn::Joined(end) => end.available(),
            Conn::Out { peeked, .. } => peeked.len(),
            Conn::Datagrams(d) => d.queue.front().map_or(0, |f| f.data.len()),
            _ => 0,
        }
    }

    /// Whether it's listening (`SO_ACCEPTCONN`).
    pub fn is_listening(&self) -> bool {
        self.0.is_listening()
    }
}

/// A connection out that has finished connecting, one way or the other, becomes connected or failed.
fn settle(state: &mut State) {
    let Conn::Connecting(stream) = &state.conn else { return };
    match stream.connected() {
        Ok(false) => {}
        Ok(true) => {
            let Conn::Connecting(stream) = std::mem::replace(&mut state.conn, Conn::Idle) else { unreachable!() };
            state.conn = Conn::Out { stream, peeked: Vec::new() };
        }
        Err(e) => {
            state.error = Some(e);
            state.conn = Conn::Failed;
        }
    }
}
