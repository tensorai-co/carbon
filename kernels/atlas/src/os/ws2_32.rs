//! ws2_32.dll: Winsock, over the machine's network (`sockets`, on `net`), at Windows' own ordinals (programs import its
//! oldest functions by number). Sockets block, don't (`FIONBIO`), or are overlapped, finishing through an event, a
//! completion port or a routine; `select` and `WSAPoll` wait on several; `getaddrinfo` asks the machine's resolver.
//! The extension functions `WSAIoctl` gives (`ConnectEx`, `AcceptEx`, `GetAcceptExSockaddrs`, `DisconnectEx`) are
//! exports of atlas's own here, as they're mswsock's on Windows.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::fileapi::close_handle;
use super::synch::{create_event_w, reset_event, set_event, wait_for_multiple_objects_ex};
use super::{Reply, dll};
use crate::files::{File, Node};
use crate::handles::Handle;
use crate::ports::{self, Op};
use crate::process::Task;
use crate::sockets::*;

dll! {
    "WS2_32.dll";
    host accept(3) => accept;
    host bind(3) => bind;
    host closesocket(1) => closesocket;
    host connect(3) => connect;
    host getpeername(3) => getpeername;
    host getsockname(3) => getsockname;
    host getsockopt(5) => getsockopt;
    host htonl(1) => byte_swap32;
    host htons(1) => byte_swap16;
    host ioctlsocket(3) => ioctlsocket;
    host inet_addr(1) => inet_addr;
    missing inet_ntoa;
    host listen(2) => listen;
    host ntohl(1) => byte_swap32;
    host ntohs(1) => byte_swap16;
    host recv(4) => recv;
    host recvfrom(6) => recvfrom;
    host select(5) => select;
    host send(4) => send;
    host sendto(6) => sendto;
    host setsockopt(5) => setsockopt;
    host shutdown(2) => shutdown;
    host socket(3) => socket;
    missing gethostbyaddr @ 51;
    missing gethostbyname;
    missing getprotobyname;
    missing getprotobynumber;
    missing getservbyname;
    missing getservbyport;
    host gethostname(2) => gethostname;
    guest WSAGetLastError @ 111 => RtlGetLastWin32Error;
    guest WSASetLastError => RtlSetLastWin32Error;
    host WSAStartup(2) @ 115 => wsa_startup;
    host WSACleanup(0) => wsa_cleanup;
    host __WSAFDIsSet(2) @ 151 => wsa_fd_is_set;
    host WSASocketA(6) => wsa_socket;
    host WSASocketW(6) => wsa_socket;
    host WSAIoctl(9) => wsa_ioctl;
    host WSARecv(7) => wsa_recv;
    host WSARecvFrom(9) => wsa_recv_from;
    host WSASend(7) => wsa_send;
    host WSASendTo(9) => wsa_send_to;
    host WSAGetOverlappedResult(5) => wsa_get_overlapped_result;
    host WSAPoll(3) => wsa_poll;
    host getaddrinfo(4) => getaddrinfo;
    host freeaddrinfo(1) => freeaddrinfo;
    host GetAddrInfoW(4) => get_addr_info_w;
    host FreeAddrInfoW(1) => freeaddrinfo;
    // mswsock's, which WSAIoctl hands out.
    host AtlasConnectEx(7) => connect_ex;
    host AtlasAcceptEx(8) => accept_ex;
    host AtlasGetAcceptExSockaddrs(8) => get_accept_ex_sockaddrs;
    host AtlasDisconnectEx(4) => disconnect_ex;
    // The process's network thread starts here.
    host AtlasNetPump(1) => net_pump;
    // Not written yet: what larger programs import.
    host WSACloseEvent(1) => close_handle;
    host WSACreateEvent(0) => wsa_create_event;
    host WSAEnumNetworkEvents(3) => wsa_enum_network_events;
    host WSAEventSelect(3) => wsa_event_select;
    host WSAResetEvent(1) => reset_event;
    missing WSASendMsg;
    host WSASetEvent(1) => set_event;
    host WSAWaitForMultipleEvents(5) => wait_for_multiple_objects_ex;
    missing WSCEnumProtocols;
    missing getnameinfo;
    missing inet_ntop;
    missing inet_pton;
    missing WSADuplicateSocketW;
    missing WSAEnumNameSpaceProvidersW;
    host WSALookupServiceBeginW(3) => wsa_lookup_service;
    host WSALookupServiceEnd(1) => wsa_lookup_service;
    host WSALookupServiceNextW(4) => wsa_lookup_service;
    missing WSASetServiceW;
    missing WSCGetProviderPath;
    host WSALookupServiceBeginA(3) => wsa_lookup_service;
    host WSALookupServiceNextA(4) => wsa_lookup_service;
}

/// SOCKET_ERROR, as an int.
const SOCKET_ERROR: u64 = 0xffff_ffff;
/// INVALID_SOCKET, all ones at either width.
const INVALID_SOCKET: u64 = u64::MAX;
const MSG_PEEK: u64 = 0x2;
const FIONBIO: u32 = 0x8004_667e;
const FIONREAD: u32 = 0x4004_667f;
const SIOCATMARK: u32 = 0x4004_7307;
const SOL_SOCKET: u32 = 0xffff;
const IPPROTO_TCP: u32 = 6;

fn byte_swap32(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from((a[0] as u32).swap_bytes()))
}

fn byte_swap16(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from((a[0] as u16).swap_bytes()))
}

/// SOCKET_ERROR (or another failure value) with the WSA error as the thread's last.
fn fail_with(task: &mut Task, cx: &mut Cx<'_>, error: u32, value: u64) -> Reply {
    task.set_last_error(cx, error);
    Reply::Value(value)
}

fn fail(task: &mut Task, cx: &mut Cx<'_>, error: u32) -> Reply {
    fail_with(task, cx, error, SOCKET_ERROR)
}

/// A call's result: its value, or its error (SOCKET_ERROR, last error set), or how it ended otherwise.
enum Failed {
    Error(u32),
    Reply(Reply),
}

impl From<u32> for Failed {
    fn from(e: u32) -> Failed {
        Failed::Error(e)
    }
}

impl From<Stopped> for Failed {
    fn from(s: Stopped) -> Failed {
        match s {
            Stopped::Error(e) => Failed::Error(e),
            Stopped::Ended => Failed::Reply(Reply::Ended),
        }
    }
}

fn answer(task: &mut Task, cx: &mut Cx<'_>, r: Result<u64, Failed>) -> Reply {
    match r {
        Ok(v) => Reply::Value(v),
        Err(Failed::Error(e)) => fail(task, cx, e),
        Err(Failed::Reply(reply)) => reply,
    }
}

impl Task {
    /// The socket a SOCKET is, and its file.
    fn sock(&self, s: u64) -> Result<(Arc<File>, Arc<Sock>), u32> {
        match lock(&self.process.state).handles.get(s) {
            Some(Handle::File(file)) => match &file.node {
                Node::Socket(sock) => {
                    let sock = sock.clone();
                    Ok((file, sock))
                }
                _ => Err(WSAENOTSOCK),
            },
            _ => Err(WSAENOTSOCK),
        }
    }

    fn bytes_at(&mut self, cx: &mut Cx<'_>, at: u64, len: usize) -> Result<Vec<u8>, u32> {
        if len == 0 {
            return Ok(Vec::new());
        }
        self.mem(cx, false, |t| user::read_vec(&t.space(), at, len)).map_err(|_| WSAEFAULT)
    }

    fn u32_at(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<u32, u32> {
        self.mem(cx, false, |t| user::read_u32(&t.space(), at)).map_err(|_| WSAEFAULT)
    }

    fn write_at(&mut self, cx: &mut Cx<'_>, at: u64, bytes: &[u8]) -> Result<(), u32> {
        self.mem(cx, true, |t| user::write(&mut t.space(), at, bytes)).map_err(|_| WSAEFAULT)
    }

    /// A `sockaddr` the program passed, `len` bytes of it.
    fn addr_arg(&mut self, cx: &mut Cx<'_>, at: u64, len: u64) -> Result<SocketAddr, u32> {
        if at == 0 {
            return Err(WSAEFAULT);
        }
        let b = self.bytes_at(cx, at, (len as u32 as usize).min(128))?;
        parse_sockaddr(&b)
    }

    /// Writes an address where the program asked, with its length where `len_at` says how much room there is.
    fn put_addr(&mut self, cx: &mut Cx<'_>, at: u64, len_at: u64, bytes: &[u8]) -> Result<(), u32> {
        if at == 0 {
            return Ok(());
        }
        let room = if len_at != 0 { self.u32_at(cx, len_at)? as usize } else { bytes.len() };
        if room < bytes.len() {
            return Err(WSAEFAULT);
        }
        self.write_at(cx, at, bytes)?;
        if len_at != 0 {
            self.write_at(cx, len_at, &(bytes.len() as u32).to_le_bytes())?;
        }
        Ok(())
    }

    /// WSABUFs: each a length and a pointer.
    fn bufs_arg(&mut self, cx: &mut Cx<'_>, at: u64, count: u64) -> Result<Vec<(u64, u32)>, u32> {
        let w = self.abi().width;
        let count = (count as u32).min(1024) as usize;
        let raw = self.bytes_at(cx, at, count * 2 * w as usize)?;
        Ok(raw
            .chunks_exact(2 * w as usize)
            .map(|c| {
                let len = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                let mut p = [0u8; 8];
                p[..w as usize].copy_from_slice(&c[w as usize..2 * w as usize]);
                (u64::from_le_bytes(p), len)
            })
            .collect())
    }

    /// What WSABUFs hold, together.
    fn gather(&mut self, cx: &mut Cx<'_>, bufs: &[(u64, u32)]) -> Result<Vec<u8>, u32> {
        let mut data = Vec::new();
        for &(at, len) in bufs {
            data.extend(self.bytes_at(cx, at, len as usize)?);
        }
        Ok(data)
    }

    /// An overlapped call's operation: its OVERLAPPED's event (unset until it's done) and the socket's port, or the
    /// routine to run instead.
    fn socket_op(&mut self, cx: &mut Cx<'_>, file: &File, ov: u64, routine: u64, buffer: u64) -> Arc<Op> {
        let abi = self.abi();
        let event = self.mem(cx, false, |t| user::read_ptr(&t.space(), ov + abi.overlapped_event(), abi.width)).unwrap_or(0);
        let (event, port, routine) = if routine != 0 {
            (None, None, Some(routine))
        } else {
            let event = (event != 0).then(|| self.object(event)).flatten();
            if let Some(e) = &event {
                self.k.objects.signal(e, |s| s.set = false);
            }
            (event, lock(&file.port).clone(), None)
        };
        Arc::new(Op { process: Arc::downgrade(&self.process), thread: Arc::downgrade(&self.control), overlapped: ov, buffer, event, port, routine })
    }

    /// Runs a socket operation as the call asks: overlapped (done now, or pending for the network thread), or
    /// blocking (waiting up to `timeout`), or not (WSAEWOULDBLOCK). Its byte count.
    #[allow(clippy::too_many_arguments)]
    fn socket_io(&mut self, cx: &mut Cx<'_>, file: &Arc<File>, sock: &Arc<Sock>, read: bool, work: Work, ov: u64, routine: u64) -> Result<u64, Failed> {
        // A receive re-enables FD_READ (and a send that would block, FD_WRITE, below).
        let selected = lock(&sock.selected).is_some();
        if read && selected {
            sock.rearm(FD_READ);
            self.wake_net();
        }
        let result = self.socket_io_now(cx, file, sock, read, work, ov, routine);
        if selected && !read && matches!(result, Err(Failed::Error(WSAEWOULDBLOCK))) {
            sock.rearm(FD_WRITE);
            self.wake_net();
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn socket_io_now(&mut self, cx: &mut Cx<'_>, file: &Arc<File>, sock: &Arc<Sock>, read: bool, mut work: Work, ov: u64, routine: u64) -> Result<u64, Failed> {
        if ov != 0 {
            let op = self.socket_op(cx, file, ov, routine, 0);
            return match sock.attempt(&mut work) {
                Some(outcome) if outcome.error == 0 || outcome.error == WSAEMSGSIZE => {
                    // Done at once: told as if it had waited, but for the port, with FILE_SKIP_COMPLETION_PORT_ON_SUCCESS.
                    let skip = outcome.error == 0 && file.modes.load(Ordering::Relaxed) & 1 != 0;
                    let op = if skip { Op { port: None, ..clone_op(&op) } } else { clone_op(&op) };
                    let k = self.k.clone();
                    finish(&k, cx, &op, &outcome);
                    if outcome.error != 0 { Err(Failed::Error(outcome.error)) } else { Ok(outcome.n) }
                }
                Some(outcome) => Err(Failed::Error(outcome.error)),
                None => {
                    let pending = self.abi().overlapped_result(ports::STATUS_PENDING, 0);
                    self.write_at(cx, ov, &pending)?;
                    self.net_queue(cx, sock, read, Pending { op, work });
                    Err(Failed::Error(WSA_IO_PENDING))
                }
            };
        }
        let outcome = if sock.nonblocking.load(Ordering::Relaxed) {
            sock.attempt(&mut work).ok_or(WSAEWOULDBLOCK)?
        } else {
            let ms = lock(&sock.timeouts)[usize::from(!read)];
            let timeout = (ms != 0).then(|| Duration::from_millis(u64::from(ms)));
            let socket = sock.net();
            self.sock_wait(&[socket], timeout, |_| Ok(sock.attempt(&mut work)))?.ok_or(WSAETIMEDOUT)?
        };
        for (at, bytes) in &outcome.writes {
            self.write_at(cx, *at, bytes)?;
        }
        if outcome.error != 0 {
            return Err(Failed::Error(outcome.error));
        }
        Ok(outcome.n)
    }

    /// A new socket's handle.
    fn new_socket(&mut self, af: u16, kind: u32, protocol: u32, overlapped: bool) -> Result<u64, u32> {
        if !self.process_winsock(0) {
            return Err(WSANOTINITIALISED);
        }
        let domain = match af {
            AF_INET => net::Domain::Inet,
            AF_INET6 => net::Domain::Inet6,
            _ => return Err(WSAEAFNOSUPPORT),
        };
        let net_kind = match kind {
            SOCK_STREAM => net::Kind::Stream,
            SOCK_DGRAM => net::Kind::Datagram,
            _ => return Err(WSAESOCKTNOSUPPORT),
        };
        let protocol = match (kind, protocol) {
            (SOCK_STREAM, 0 | 6) => 6,
            (SOCK_DGRAM, 0 | 17) => 17,
            _ => return Err(WSAEPROTONOSUPPORT),
        };
        let socket = self.k.net.socket(domain, net_kind).map_err(|e| wsa(&e))?;
        Ok(self.socket_handle(Sock::new(&self.process, socket, af, kind, protocol), overlapped))
    }

    fn socket_handle(&mut self, sock: Arc<Sock>, overlapped: bool) -> u64 {
        let file = File::new(Node::Socket(sock), true, true, false, "\\Device\\Afd".into(), String::new(), false, overlapped);
        lock(&self.process.state).handles.add(Handle::File(Arc::new(file)))
    }

    /// Counts `WSAStartup`s less `WSACleanup`s for this process: false if a cleanup has none to undo; with 0, whether
    /// it's started.
    fn process_winsock(&self, by: i32) -> bool {
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STARTED: Mutex<Option<HashMap<u64, i64>>> = Mutex::new(None);
        let mut m = lock(&STARTED);
        let n = m.get_or_insert_with(HashMap::new).entry(self.process.pid).or_insert(0);
        if by <= 0 && *n <= 0 {
            return false;
        }
        *n += i64::from(by);
        true
    }
}

fn clone_op(op: &Op) -> Op {
    Op { process: op.process.clone(), thread: op.thread.clone(), overlapped: op.overlapped, buffer: op.buffer, event: op.event.clone(), port: op.port.clone(), routine: op.routine }
}

// ---- Starting up ----

const WSAVERNOTSUPPORTED: u64 = 10092;

/// `WSAStartup(version, &data)`: Winsock 2.2, or the version asked for if it's older (from 1.0); WSADATA says which,
/// as each ISA lays it out.
fn wsa_startup(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (major, minor) = (a[0] as u8, (a[0] >> 8) as u8);
    if major == 0 {
        return Reply::Value(WSAVERNOTSUPPORTED);
    }
    let version = if (major, minor) >= (2, 2) { 0x0202u16 } else { u16::from(major) | u16::from(minor) << 8 };
    let text = |s: &str, n: usize| {
        let mut b = s.as_bytes().to_vec();
        b.resize(n, 0);
        b
    };
    let mut data = Vec::new();
    data.extend(version.to_le_bytes());
    data.extend(0x0202u16.to_le_bytes());
    if task.abi().width == 8 {
        // iMaxSockets and iMaxUdpDg (0 from 2.0 on), lpVendorInfo, then the two strings.
        data.extend([0u8; 12]);
        data.extend(text("WinSock 2.0", 257));
        data.extend(text("Running", 129));
        data.resize(408, 0);
    } else {
        data.extend(text("WinSock 2.0", 257));
        data.extend(text("Running", 129));
        data.resize(400, 0);
    }
    if a[1] == 0 || task.put(cx, a[1], &data).is_err() {
        return Reply::Value(u64::from(WSAEFAULT));
    }
    task.process_winsock(1);
    Reply::Value(0)
}

/// `WSACleanup()`: one `WSAStartup` undone; SOCKET_ERROR with WSANOTINITIALISED if there's none.
fn wsa_cleanup(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    if !task.process_winsock(-1) {
        return fail(task, cx, WSANOTINITIALISED);
    }
    Reply::Value(0)
}

/// `__WSAFDIsSet(socket, &set)`: whether an fd_set (a count, then that many SOCKETs) has the socket.
fn wsa_fd_is_set(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let w = task.abi().width;
    let found = task.mem(cx, false, |t| {
        let count = u64::from(user::read_u32(&t.space(), a[1])?).min(64);
        // The count is a u_int, then the array, aligned as SOCKETs are.
        for i in 0..count {
            if user::read_ptr(&t.space(), a[1] + w + i * w, w)? == a[0] {
                return Ok(true);
            }
        }
        Ok(false)
    });
    Reply::Value(u64::from(found.unwrap_or(false)))
}

// ---- Making, naming and connecting sockets ----

/// `socket(af, type, protocol)`: an overlapped socket, as Winsock's are.
fn socket(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.new_socket(a[0] as u16, a[1] as u32, a[2] as u32, true) {
        Ok(h) => Reply::Value(h),
        Err(e) => fail_with(task, cx, e, INVALID_SOCKET),
    }
}

/// `WSASocketA/W(af, type, protocol, &protocol info, group, flags)`: overlapped with WSA_FLAG_OVERLAPPED.
fn wsa_socket(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.new_socket(a[0] as u16, a[1] as u32, a[2] as u32, a[5] & 1 != 0) {
        Ok(h) => Reply::Value(h),
        Err(e) => fail_with(task, cx, e, INVALID_SOCKET),
    }
}

/// `closesocket(s)`: what's under way on it ends with WSA_OPERATION_ABORTED, and its handle goes.
fn closesocket(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (_, sock) = match task.sock(a[0]) {
        Ok(s) => s,
        Err(e) => return fail(task, cx, e),
    };
    let k = task.k.clone();
    for p in sock.take_all() {
        finish(&k, cx, &p.op, &Outcome { writes: Vec::new(), n: 0, error: WSA_OPERATION_ABORTED });
    }
    lock(&task.process.state).handles.close(a[0]);
    Reply::Value(0)
}

fn bind(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let addr = task.addr_arg(cx, a[1], a[2])?;
        sock.net().bind(&net::Addr::Inet(addr)).map_err(|e| wsa(&e))?;
        Ok(0)
    })();
    answer(task, cx, r)
}

fn listen(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        if !sock.is_stream() {
            return Err(WSAEOPNOTSUPP.into());
        }
        sock.net().listen(a[1] as u32).map_err(|e| wsa(&e))?;
        sock.listening.store(true, Ordering::Relaxed);
        Ok(0)
    })();
    answer(task, cx, r)
}

/// `connect(s, &addr, len)`: blocking until it's connected or refused; a non-blocking socket's gives
/// WSAEWOULDBLOCK while it's under way, and `select` says how it went.
fn connect(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let addr = task.addr_arg(cx, a[1], a[2])?;
        let socket = sock.net();
        match socket.connect(&net::Addr::Inet(addr)) {
            Ok(()) => Ok(0),
            Err(net::Error::InProgress) if sock.nonblocking.load(Ordering::Relaxed) => {
                // FD_CONNECT says how it went.
                sock.connecting.store(true, Ordering::Relaxed);
                sock.rearm(FD_CONNECT);
                task.wake_net();
                Err(WSAEWOULDBLOCK.into())
            }
            Err(net::Error::InProgress) => {
                let s = socket.clone();
                task.sock_wait(&[socket], None, |_| {
                    let ready = s.ready();
                    if !(ready.write || ready.error || ready.hangup) {
                        return Ok(None);
                    }
                    match s.take_error() {
                        Some(e) => Err(wsa(&e)),
                        None => Ok(Some(0)),
                    }
                })?
                .ok_or(Failed::Error(WSAETIMEDOUT))
            }
            Err(e) => Err(wsa(&e).into()),
        }
    })();
    answer(task, cx, r)
}

/// `accept(s, &addr, &len)`: the next connection, as a new socket, waiting for one unless it's non-blocking.
fn accept(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        sock.rearm(FD_ACCEPT);
        task.wake_net();
        let socket = sock.net();
        let take = |_: &mut Task| match socket.accept() {
            Ok(got) => Ok(Some(got)),
            Err(net::Error::WouldBlock) => Ok(None),
            Err(e) => Err(wsa(&e)),
        };
        let (new, peer) = if sock.nonblocking.load(Ordering::Relaxed) {
            take(task)?.ok_or(WSAEWOULDBLOCK)?
        } else {
            task.sock_wait(std::slice::from_ref(&socket), None, take)?.ok_or(WSAETIMEDOUT)?
        };
        let accepted = Sock::new(&task.process, new, sock.family, sock.kind, sock.protocol);
        accepted.nonblocking.store(sock.nonblocking.load(Ordering::Relaxed), Ordering::Relaxed);
        task.put_addr(cx, a[1], a[2], &net_sockaddr(&peer))?;
        Ok(task.socket_handle(accepted, file.overlapped))
    })();
    match r {
        Ok(h) => Reply::Value(h),
        Err(Failed::Error(e)) => fail_with(task, cx, e, INVALID_SOCKET),
        Err(Failed::Reply(reply)) => reply,
    }
}

fn names(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], peer: bool) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let socket = sock.net();
        let addr = if peer { socket.peer().map_err(|e| wsa(&e))? } else { socket.local() };
        task.put_addr(cx, a[1], a[2], &net_sockaddr(&addr))?;
        Ok(0)
    })();
    answer(task, cx, r)
}

fn getsockname(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    names(task, cx, a, false)
}

fn getpeername(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    names(task, cx, a, true)
}

/// `shutdown(s, how)`: SD_RECEIVE, SD_SEND or SD_BOTH.
fn shutdown(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let how = match a[1] as u32 {
            0 => net::Shutdown::Read,
            1 => net::Shutdown::Write,
            2 => net::Shutdown::Both,
            _ => return Err(WSAEINVAL.into()),
        };
        sock.net().shutdown(how).map_err(|e| wsa(&e))?;
        Ok(0)
    })();
    answer(task, cx, r)
}

// ---- Sending and receiving ----

fn recv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        task.socket_io(cx, &file, &sock, true, Work::Recv { bufs: vec![(a[1], a[2] as u32)], peek: a[3] & MSG_PEEK != 0, from: None }, 0, 0)
    })();
    answer(task, cx, r)
}

/// `recvfrom(s, buffer, len, flags, &from, &from len)`.
fn recvfrom(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        task.socket_io(cx, &file, &sock, true, Work::Recv { bufs: vec![(a[1], a[2] as u32)], peek: a[3] & MSG_PEEK != 0, from: Some((a[4], a[5])) }, 0, 0)
    })();
    answer(task, cx, r)
}

fn send(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let data = task.bytes_at(cx, a[1], a[2] as u32 as usize)?;
        task.socket_io(cx, &file, &sock, false, Work::Send { data, sent: 0, to: None }, 0, 0)
    })();
    answer(task, cx, r)
}

/// `sendto(s, buffer, len, flags, &to, to len)`.
fn sendto(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let data = task.bytes_at(cx, a[1], a[2] as u32 as usize)?;
        let to = if a[4] != 0 { Some(net::Addr::Inet(task.addr_arg(cx, a[4], a[5])?)) } else { None };
        task.socket_io(cx, &file, &sock, false, Work::Send { data, sent: 0, to }, 0, 0)
    })();
    answer(task, cx, r)
}

/// The WSA calls' common end: their count (and flags, for a receive) where the program asked, and 0.
fn wsa_done(task: &mut Task, cx: &mut Cx<'_>, r: Result<u64, Failed>, count_at: u64, flags_at: u64) -> Reply {
    match r {
        Ok(n) => {
            if count_at != 0 {
                let _ = task.write_at(cx, count_at, &(n as u32).to_le_bytes());
            }
            if flags_at != 0 {
                let _ = task.write_at(cx, flags_at, &0u32.to_le_bytes());
            }
            Reply::Value(0)
        }
        other => answer(task, cx, other),
    }
}

/// `WSARecv(s, buffers, count, &received, &flags, overlapped, routine)`.
fn wsa_recv(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let bufs = task.bufs_arg(cx, a[1], a[2])?;
        let flags = if a[4] != 0 { task.u32_at(cx, a[4])? } else { 0 };
        task.socket_io(cx, &file, &sock, true, Work::Recv { bufs, peek: u64::from(flags) & MSG_PEEK != 0, from: None }, a[5], a[6])
    })();
    wsa_done(task, cx, r, a[3], a[4])
}

/// `WSARecvFrom(s, buffers, count, &received, &flags, &from, &from len, overlapped, routine)`.
fn wsa_recv_from(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let bufs = task.bufs_arg(cx, a[1], a[2])?;
        let flags = if a[4] != 0 { task.u32_at(cx, a[4])? } else { 0 };
        task.socket_io(cx, &file, &sock, true, Work::Recv { bufs, peek: u64::from(flags) & MSG_PEEK != 0, from: Some((a[5], a[6])) }, a[7], a[8])
    })();
    wsa_done(task, cx, r, a[3], a[4])
}

/// `WSASend(s, buffers, count, &sent, flags, overlapped, routine)`.
fn wsa_send(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let bufs = task.bufs_arg(cx, a[1], a[2])?;
        let data = task.gather(cx, &bufs)?;
        task.socket_io(cx, &file, &sock, false, Work::Send { data, sent: 0, to: None }, a[5], a[6])
    })();
    wsa_done(task, cx, r, a[3], 0)
}

/// `WSASendTo(s, buffers, count, &sent, flags, &to, to len, overlapped, routine)`.
fn wsa_send_to(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let bufs = task.bufs_arg(cx, a[1], a[2])?;
        let data = task.gather(cx, &bufs)?;
        let to = if a[5] != 0 { Some(net::Addr::Inet(task.addr_arg(cx, a[5], a[6])?)) } else { None };
        task.socket_io(cx, &file, &sock, false, Work::Send { data, sent: 0, to }, a[7], a[8])
    })();
    wsa_done(task, cx, r, a[3], 0)
}

/// `WSAGetOverlappedResult(s, overlapped, &bytes, wait, &flags)`: how an overlapped call went, from its OVERLAPPED;
/// with `wait`, once it's done.
fn wsa_get_overlapped_result(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let abi = task.abi();
    if let Err(e) = task.sock(a[0]) {
        return fail_with(task, cx, e, 0);
    }
    loop {
        let Ok((status, n)) = task.mem(cx, false, |t| Ok((user::read_ptr(&t.space(), a[1], abi.width)? as u32, user::read_ptr(&t.space(), a[1] + abi.width, abi.width)?))) else {
            task.set_last_error(cx, WSAEFAULT);
            return Reply::Value(0);
        };
        if status == ports::STATUS_PENDING {
            if a[3] as u32 == 0 {
                task.set_last_error(cx, ports::ERROR_IO_INCOMPLETE);
                return Reply::Value(0);
            }
            // The network thread writes it: look again soon.
            if let Err(reply) = task.wait_for(cx, &[], false, 5, false) {
                return reply;
            }
            continue;
        }
        let _ = task.write_at(cx, a[2], &(n as u32).to_le_bytes());
        if a[4] != 0 {
            let _ = task.write_at(cx, a[4], &0u32.to_le_bytes());
        }
        if status != 0 {
            task.set_last_error(cx, ports::error(status));
            return Reply::Value(0);
        }
        return Reply::Value(1);
    }
}

// ---- Waiting on several ----

/// What's ready on a socket, as `select` and `WSAPoll` ask: (readable, writable, an error).
fn readiness(socket: &net::Socket) -> (bool, bool, bool) {
    let r = socket.ready();
    (readable(socket, &r), r.write && !r.error, r.error)
}

/// Whether a read wouldn't wait. `net` says a host socket is readable from when data comes until a read finds none,
/// so a socket read dry still says so: a byte peeked says for sure (and, finding none, has `net` watch again).
fn readable(socket: &net::Socket, r: &net::Ready) -> bool {
    if socket.is_listening() {
        return r.read;
    }
    (r.read || r.read_closed || r.hangup) && !matches!(socket.recv(1, true), Err(net::Error::WouldBlock))
}

/// `select(nfds, &read, &write, &except, &timeout)`: the sockets of each set that are ready (readable: data, the
/// end, or a connection to accept; writable: connected; except: a connect that failed), waiting until one is or the
/// timeout passes (none: forever). How many there are; the sets keep only them.
fn select(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let w = task.abi().width;
        let mut sets: Vec<(u64, Vec<u64>)> = Vec::new();
        for &at in &a[1..4] {
            if at == 0 {
                sets.push((0, Vec::new()));
                continue;
            }
            let count = u64::from(task.u32_at(cx, at)?).min(64);
            let raw = task.bytes_at(cx, at + w, (count * w) as usize)?;
            let list = raw
                .chunks_exact(w as usize)
                .map(|c| {
                    let mut p = [0u8; 8];
                    p[..w as usize].copy_from_slice(c);
                    u64::from_le_bytes(p)
                })
                .collect();
            sets.push((at, list));
        }
        let mut sockets = Vec::new();
        for (_, list) in &sets {
            for &s in list {
                let (_, sock) = task.sock(s)?;
                sockets.push((s, sock.net()));
            }
        }
        if sockets.is_empty() {
            return Err(WSAEINVAL.into());
        }
        let timeout = if a[4] != 0 {
            let t = task.bytes_at(cx, a[4], 8)?;
            let (sec, usec) = (u32::from_le_bytes([t[0], t[1], t[2], t[3]]), u32::from_le_bytes([t[4], t[5], t[6], t[7]]));
            Some(Duration::from_secs(u64::from(sec)) + Duration::from_micros(u64::from(usec)))
        } else {
            None
        };
        let check = |_: &mut Task| {
            let ready: Vec<Vec<u64>> = sets
                .iter()
                .enumerate()
                .map(|(i, (_, list))| {
                    list.iter()
                        .copied()
                        .filter(|s| {
                            let socket = &sockets.iter().find(|(h, _)| h == s).expect("every socket is listed").1;
                            let (r, w, e) = readiness(socket);
                            [r, w, e][i]
                        })
                        .collect()
                })
                .collect();
            Ok(ready.iter().any(|l| !l.is_empty()).then_some(ready))
        };
        let all: Vec<Arc<net::Socket>> = sockets.iter().map(|(_, s)| s.clone()).collect();
        let ready = task.sock_wait(&all, timeout, check)?.unwrap_or_else(|| vec![Vec::new(); 3]);
        let mut total = 0;
        for ((at, _), list) in sets.iter().zip(&ready) {
            if *at == 0 {
                continue;
            }
            let mut b = (list.len() as u32).to_le_bytes().to_vec();
            b.resize(w as usize, 0);
            for s in list {
                b.extend(&s.to_le_bytes()[..w as usize]);
            }
            task.write_at(cx, *at, &b)?;
            total += list.len() as u64;
        }
        Ok(total)
    })();
    answer(task, cx, r)
}

/// `WSAPoll(&fds, count, timeout)`: each WSAPOLLFD's revents (POLLRDNORM, POLLWRNORM, POLLERR, POLLHUP, POLLNVAL
/// for what isn't a socket), waiting up to `timeout` ms (negative: forever) for one. How many have any.
fn wsa_poll(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const POLLRDNORM: u16 = 0x100;
    const POLLWRNORM: u16 = 0x10;
    const POLLERR: u16 = 0x1;
    const POLLHUP: u16 = 0x2;
    const POLLNVAL: u16 = 0x4;
    let r = (|| -> Result<u64, Failed> {
        let w = task.abi().width;
        let size = w + if w == 8 { 8 } else { 4 };
        let count = (a[1] as u32).min(4096) as u64;
        let raw = task.bytes_at(cx, a[0], (count * size) as usize)?;
        let fds: Vec<(u64, u16)> = raw
            .chunks_exact(size as usize)
            .map(|c| {
                let mut p = [0u8; 8];
                p[..w as usize].copy_from_slice(&c[..w as usize]);
                (u64::from_le_bytes(p), u16::from_le_bytes([c[w as usize], c[w as usize + 1]]))
            })
            .collect();
        let socks: Vec<Option<Arc<net::Socket>>> = fds.iter().map(|(s, _)| task.sock(*s).ok().map(|(_, k)| k.net())).collect();
        let timeout = match a[2] as u32 as i32 {
            t if t < 0 => None,
            t => Some(Duration::from_millis(t as u64)),
        };
        let check = |_: &mut Task| {
            let revents: Vec<u16> = fds
                .iter()
                .zip(&socks)
                .map(|((_, events), s)| match s {
                    None => POLLNVAL,
                    Some(s) => {
                        let ready = s.ready();
                        let mut r = 0;
                        if events & 0x300 != 0 && readable(s, &ready) {
                            r |= POLLRDNORM;
                        }
                        if ready.write && events & POLLWRNORM != 0 {
                            r |= POLLWRNORM;
                        }
                        if ready.error {
                            r |= POLLERR;
                        }
                        if ready.hangup {
                            r |= POLLHUP;
                        }
                        r
                    }
                })
                .collect();
            Ok(revents.iter().any(|&r| r != 0).then_some(revents))
        };
        let all: Vec<Arc<net::Socket>> = socks.iter().flatten().cloned().collect();
        let revents = task.sock_wait(&all, timeout, check)?.unwrap_or_else(|| vec![0; fds.len()]);
        for (i, r) in revents.iter().enumerate() {
            task.write_at(cx, a[0] + i as u64 * size + w + 2, &r.to_le_bytes())?;
        }
        Ok(revents.iter().filter(|&&r| r != 0).count() as u64)
    })();
    answer(task, cx, r)
}

// ---- Options and controls ----

/// `ioctlsocket(s, command, &argument)`: FIONBIO, FIONREAD and SIOCATMARK.
fn ioctlsocket(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        control(task, cx, &sock, a[1] as u32, a[2], 4, a[2], 4).map(|_| 0)
    })();
    answer(task, cx, r)
}

/// A control's work, for `ioctlsocket` and `WSAIoctl`: how many bytes of output it wrote.
#[allow(clippy::too_many_arguments)]
fn control(task: &mut Task, cx: &mut Cx<'_>, sock: &Arc<Sock>, code: u32, input: u64, in_len: u32, out: u64, out_len: u32) -> Result<u32, Failed> {
    match code {
        FIONBIO => {
            if in_len < 4 {
                return Err(WSAEFAULT.into());
            }
            let on = task.u32_at(cx, input)? != 0;
            sock.nonblocking.store(on, Ordering::Relaxed);
            Ok(0)
        }
        FIONREAD => {
            if out_len < 4 {
                return Err(WSAEFAULT.into());
            }
            task.write_at(cx, out, &(sock.net().available() as u32).to_le_bytes())?;
            Ok(4)
        }
        SIOCATMARK => {
            task.write_at(cx, out, &1u32.to_le_bytes())?;
            Ok(4)
        }
        _ => Err(WSAEINVAL.into()),
    }
}

/// The extension functions' GUIDs, and the export that is each.
const EXTENSIONS: [(&str, [u8; 16]); 4] = [
    ("AtlasConnectEx", [0xb9, 0x07, 0xa2, 0x25, 0xf3, 0xdd, 0x60, 0x46, 0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e]),
    ("AtlasAcceptEx", [0xf1, 0x7d, 0x36, 0xb5, 0xac, 0xcb, 0xcf, 0x11, 0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92]),
    ("AtlasGetAcceptExSockaddrs", [0xf2, 0x7d, 0x36, 0xb5, 0xac, 0xcb, 0xcf, 0x11, 0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92]),
    ("AtlasDisconnectEx", [0x11, 0x2e, 0xda, 0x7f, 0x30, 0x86, 0x6f, 0x43, 0xa0, 0x31, 0xf5, 0x36, 0xa6, 0xee, 0xc1, 0x57]),
];

/// `WSAIoctl(s, code, in, in len, out, out len, &returned, overlapped, routine)`: the extension functions
/// (SIO_GET_EXTENSION_FUNCTION_POINTER), the base handle, the address list, keep-alives, and what `ioctlsocket`
/// takes. Overlapped, it's done at once, and told so.
fn wsa_ioctl(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xc800_0006;
    const SIO_BASE_HANDLE: u32 = 0x4800_0022;
    const SIO_BSP_HANDLE_SELECT: u32 = 0x4800_001c;
    const SIO_ADDRESS_LIST_QUERY: u32 = 0x4800_0016;
    const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
    const SIO_UDP_CONNRESET: u32 = 0x9800_000c;
    const SIO_LOOPBACK_FAST_PATH: u32 = 0x9800_0010;
    let (code, input, in_len, out, out_len) = (a[1] as u32, a[2], a[3] as u32, a[4], a[5] as u32);
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let w = task.abi().width;
        let written: u32 = match code {
            SIO_GET_EXTENSION_FUNCTION_POINTER => {
                let guid = task.bytes_at(cx, input, 16.min(in_len as usize))?;
                let Some((name, _)) = EXTENSIONS.iter().find(|(_, g)| g[..] == guid[..]) else { return Err(WSAEINVAL.into()) };
                let at = task.system_export("ws2_32.dll", name).ok_or(Failed::Error(WSAEINVAL))?;
                if (out_len as u64) < w {
                    return Err(WSAEFAULT.into());
                }
                let ptr = task.abi().ptr(at);
                task.write_at(cx, out, &ptr)?;
                w as u32
            }
            SIO_BASE_HANDLE | SIO_BSP_HANDLE_SELECT => {
                let ptr = task.abi().ptr(a[0]);
                task.write_at(cx, out, &ptr)?;
                w as u32
            }
            SIO_ADDRESS_LIST_QUERY => {
                // SOCKET_ADDRESS_LIST: a count, then (pointer, length) for each: the machine's own address.
                let ip = if sock.family == AF_INET6 { std::net::IpAddr::V6(task.k.net.address().to_ipv6_mapped()) } else { std::net::IpAddr::V4(task.k.net.address()) };
                let addr = sockaddr(&SocketAddr::new(ip, 0));
                let header = w + 2 * w;
                let need = header as u32 + addr.len() as u32;
                if out_len < need {
                    task.write_at(cx, a[6], &need.to_le_bytes())?;
                    return Err(WSAEFAULT.into());
                }
                let mut b = task.abi().ptr(1);
                b.extend(task.abi().ptr(out + header));
                b.extend(task.abi().ptr(addr.len() as u64));
                b.extend(&addr);
                task.write_at(cx, out, &b)?;
                need
            }
            SIO_KEEPALIVE_VALS => {
                let on = task.u32_at(cx, input)? != 0;
                sock.net().set_options(|o| o.keepalive = on);
                0
            }
            SIO_UDP_CONNRESET | SIO_LOOPBACK_FAST_PATH => 0,
            other => control(task, cx, &sock, other, input, in_len, out, out_len)?,
        };
        if a[6] != 0 {
            task.write_at(cx, a[6], &written.to_le_bytes())?;
        }
        if a[7] != 0 {
            let op = task.socket_op(cx, &file, a[7], a[8], 0);
            let k = task.k.clone();
            finish(&k, cx, &op, &Outcome { writes: Vec::new(), n: u64::from(written), error: 0 });
        }
        Ok(0)
    })();
    answer(task, cx, r)
}

/// An int option's value.
fn int(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

/// `setsockopt(s, level, name, &value, len)`: what `net` acts on (SO_REUSEADDR, SO_EXCLUSIVEADDRUSE, SO_KEEPALIVE,
/// SO_BROADCAST, TCP_NODELAY), the timeouts, and the rest kept for `getsockopt`.
fn setsockopt(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    const SO_REUSEADDR: u32 = 0x4;
    const SO_EXCLUSIVEADDRUSE: u32 = !SO_REUSEADDR;
    const SO_KEEPALIVE: u32 = 0x8;
    const SO_BROADCAST: u32 = 0x20;
    const SO_SNDTIMEO: u32 = 0x1005;
    const SO_RCVTIMEO: u32 = 0x1006;
    const TCP_NODELAY: u32 = 1;
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let (level, name) = (a[1] as u32, a[2] as u32);
        let value = task.bytes_at(cx, a[3], (a[4] as u32 as usize).min(256))?;
        let flag = value.first().is_some_and(|&b| b != 0) || value.iter().any(|&b| b != 0);
        let ms = || value.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let socket = sock.net();
        match (level, name) {
            (SOL_SOCKET, SO_REUSEADDR) => socket.set_options(|o| o.reuse_addr = flag),
            (SOL_SOCKET, SO_EXCLUSIVEADDRUSE) => socket.set_options(|o| o.reuse_addr = !flag && o.reuse_addr),
            (SOL_SOCKET, SO_KEEPALIVE) => socket.set_options(|o| o.keepalive = flag),
            (SOL_SOCKET, SO_BROADCAST) => socket.set_options(|o| o.broadcast = flag),
            (SOL_SOCKET, SO_RCVTIMEO) => lock(&sock.timeouts)[0] = ms(),
            (SOL_SOCKET, SO_SNDTIMEO) => lock(&sock.timeouts)[1] = ms(),
            (IPPROTO_TCP, TCP_NODELAY) => socket.set_options(|o| o.nodelay = flag),
            _ => {}
        }
        lock(&sock.kept).insert((level, name), value);
        Ok(0)
    })();
    answer(task, cx, r)
}

/// `getsockopt(s, level, name, &value, &len)`: the type, the pending error, whether it's listening, the options
/// `net` has, the timeouts and buffer sizes, and what was set; anything else is 0.
fn getsockopt(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let (level, name) = (a[1] as u32, a[2] as u32);
        let socket = sock.net();
        let options = socket.options();
        let kept = lock(&sock.kept).get(&(level, name)).cloned();
        let value = match (level, name) {
            (SOL_SOCKET, 0x1008) => int(sock.kind),
            (SOL_SOCKET, 0x1007) => int(socket.take_error().map_or(0, |e| wsa(&e))),
            (SOL_SOCKET, 0x2) => int(u32::from(socket.is_listening())),
            (SOL_SOCKET, 0x4) => int(u32::from(options.reuse_addr)),
            (SOL_SOCKET, 0x8) => int(u32::from(options.keepalive)),
            (SOL_SOCKET, 0x20) => int(u32::from(options.broadcast)),
            (SOL_SOCKET, 0x1006) => int(lock(&sock.timeouts)[0]),
            (SOL_SOCKET, 0x1005) => int(lock(&sock.timeouts)[1]),
            (SOL_SOCKET, 0x1001 | 0x1002) => kept.unwrap_or_else(|| int(64 << 10)),
            (SOL_SOCKET, 0x80) => kept.unwrap_or_else(|| vec![0; 4]),
            // SO_CONNECT_TIME: seconds connected, or -1 for not.
            (SOL_SOCKET, 0x700c) => int(if socket.peer().is_ok() { 0 } else { u32::MAX }),
            (IPPROTO_TCP, 1) => int(u32::from(options.nodelay)),
            _ => kept.unwrap_or_else(|| vec![0; 4]),
        };
        let room = task.u32_at(cx, a[4])? as usize;
        let n = value.len().min(room);
        if n < value.len() && n < 4 {
            return Err(WSAEFAULT.into());
        }
        task.write_at(cx, a[3], &value[..n])?;
        task.write_at(cx, a[4], &(n as u32).to_le_bytes())?;
        Ok(0)
    })();
    answer(task, cx, r)
}

// ---- Names ----

/// `gethostname(buffer, len)`: the machine's name.
fn gethostname(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = super::sysapi::computer_name();
    let r = (|| -> Result<u64, Failed> {
        if (a[1] as u32 as usize) <= name.len() {
            return Err(WSAEFAULT.into());
        }
        task.write_at(cx, a[0], &[name.as_bytes(), &[0]].concat())?;
        Ok(0)
    })();
    answer(task, cx, r)
}

/// `inet_addr(text)`: a dotted IPv4 address as its four bytes in order; INADDR_NONE for what isn't one.
fn inet_addr(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let text = task.mem(cx, false, |t| user::read_cstr(&t.space(), a[0], 64)).unwrap_or_default();
    match std::str::from_utf8(&text).ok().and_then(|s| s.trim().parse::<std::net::Ipv4Addr>().ok()) {
        Some(ip) => Reply::Value(u64::from(u32::from_le_bytes(ip.octets()))),
        None => Reply::Value(0xffff_ffff),
    }
}

/// `getaddrinfo(node, service, &hints, &result)` and `GetAddrInfoW`: the machine's resolver's addresses for a name
/// (a numeric address as it is; none, the passive or loopback ones), as the hints' family, type and protocol ask,
/// each an ADDRINFO in one block from the heap, which `freeaddrinfo` frees. 0, or the error (also the last error).
fn addr_info(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], wide: bool) -> Reply {
    const AI_PASSIVE: u32 = 0x1;
    const AI_CANONNAME: u32 = 0x2;
    const AI_NUMERICHOST: u32 = 0x4;
    const WSATYPE_NOT_FOUND: u32 = 10109;
    let r = (|| -> Result<u64, Failed> {
        let node = (a[0] != 0).then(|| task.text_arg(cx, a[0], wide));
        let service = (a[1] != 0).then(|| task.text_arg(cx, a[1], wide));
        if node.is_none() && service.is_none() {
            return Err(WSAHOST_NOT_FOUND.into());
        }
        let hints = if a[2] != 0 { task.bytes_at(cx, a[2], 16)? } else { vec![0; 16] };
        let field = |i: usize| u32::from_le_bytes([hints[i], hints[i + 1], hints[i + 2], hints[i + 3]]);
        let (flags, family, kind, protocol) = (field(0), field(4) as u16, field(8), field(12));
        let port = match &service {
            Some(s) => service_port(s).ok_or(Failed::Error(WSATYPE_NOT_FOUND))?,
            None => 0,
        };
        let ips = resolve(&task.k, node.as_deref(), family, flags & AI_PASSIVE != 0, flags & AI_NUMERICHOST != 0)?;
        let protocol = match (kind, protocol) {
            (SOCK_STREAM, 0) => 6,
            (SOCK_DGRAM, 0) => 17,
            (_, p) => p,
        };
        // The block: every ADDRINFO, then each one's sockaddr (28 bytes' room), then the canonical name.
        let w = task.abi().width;
        let entry = if w == 8 { 48 } else { 32 };
        let n = ips.len() as u64;
        let addrs_at = n * entry;
        let name_at = addrs_at + n * 28;
        let canon = (flags & AI_CANONNAME != 0).then(|| node.clone().unwrap_or_default());
        let name: Vec<u8> = match &canon {
            Some(c) if wide => c.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect(),
            Some(c) => c.bytes().chain([0]).collect(),
            None => Vec::new(),
        };
        let size = (name_at + name.len() as u64).max(1);
        let base = task.task_alloc(cx, &vec![0; size as usize]).map_err(Failed::Reply)?;
        if base == 0 {
            return Err(10055.into()); // WSA_NOT_ENOUGH_MEMORY's WSAENOBUFS
        }
        let abi = task.abi();
        let mut block = Vec::with_capacity(size as usize);
        for (i, ip) in ips.iter().enumerate() {
            let i = i as u64;
            let addr = sockaddr(&SocketAddr::new(*ip, port));
            let fam = if ip.is_ipv4() { AF_INET } else { AF_INET6 };
            for v in [flags & AI_CANONNAME, u32::from(fam), kind, protocol] {
                block.extend(v.to_le_bytes());
            }
            block.extend(abi.ptr(addr.len() as u64));
            block.extend(abi.ptr(if i == 0 && canon.is_some() { base + name_at } else { 0 }));
            block.extend(abi.ptr(base + addrs_at + i * 28));
            block.extend(abi.ptr(if i + 1 < n { base + (i + 1) * entry } else { 0 }));
        }
        for ip in &ips {
            let mut addr = sockaddr(&SocketAddr::new(*ip, port));
            addr.resize(28, 0);
            block.extend(addr);
        }
        block.extend(name);
        task.write_at(cx, base, &block)?;
        task.write_at(cx, a[3], &abi.ptr(base))?;
        Ok(0)
    })();
    match r {
        Ok(_) => Reply::Value(0),
        Err(Failed::Error(e)) => {
            task.set_last_error(cx, e);
            Reply::Value(u64::from(e))
        }
        Err(Failed::Reply(reply)) => reply,
    }
}

fn getaddrinfo(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    addr_info(task, cx, a, false)
}

fn get_addr_info_w(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    addr_info(task, cx, a, true)
}

/// `freeaddrinfo(list)` and `FreeAddrInfoW`: the block `getaddrinfo` gave.
fn freeaddrinfo(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.task_free(cx, a[0]) {
        Ok(()) => Reply::Value(0),
        Err(reply) => reply,
    }
}

// ---- mswsock's extensions ----

/// `ConnectEx(s, &addr, len, send buffer, send len, &sent, overlapped)`: a connect that finishes through the
/// OVERLAPPED, then sends what it's given.
fn connect_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        if a[6] == 0 {
            return Err(WSAEINVAL.into());
        }
        let addr = task.addr_arg(cx, a[1], a[2])?;
        let then = task.bytes_at(cx, a[3], a[4] as u32 as usize)?;
        match sock.net().connect(&net::Addr::Inet(addr)) {
            Ok(()) | Err(net::Error::InProgress) => {}
            Err(e) => return Err(wsa(&e).into()),
        }
        task.socket_io(cx, &file, &sock, false, Work::Connect { then }, a[6], 0)
    })();
    match r {
        Ok(n) => {
            if a[5] != 0 {
                let _ = task.write_at(cx, a[5], &(n as u32).to_le_bytes());
            }
            Reply::Value(1)
        }
        Err(Failed::Error(e)) => fail_with(task, cx, e, 0),
        Err(Failed::Reply(reply)) => reply,
    }
}

/// `AcceptEx(listening, accepting, buffer, receive len, local len, remote len, &received, overlapped)`: the next
/// connection, made the accepting socket's, with its first bytes (if there's room for any) and both addresses in the
/// buffer, after the bytes.
pub(crate) fn accept_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let into = match lock(&task.process.state).handles.get(a[1]) {
            Some(Handle::File(f)) if matches!(f.node, Node::Socket(_)) => f,
            _ => return Err(WSAENOTSOCK.into()),
        };
        if a[7] == 0 {
            return Err(WSAEINVAL.into());
        }
        let work = Work::Accept { into, buffer: a[2], recv: a[3] as u32, local_len: a[4] as u32, addresses: None };
        task.socket_io(cx, &file, &sock, true, work, a[7], 0)
    })();
    match r {
        Ok(n) => {
            if a[6] != 0 {
                let _ = task.write_at(cx, a[6], &(n as u32).to_le_bytes());
            }
            Reply::Value(1)
        }
        Err(Failed::Error(e)) => fail_with(task, cx, e, 0),
        Err(Failed::Reply(reply)) => reply,
    }
}

/// `GetAcceptExSockaddrs(buffer, receive len, local len, remote len, &local, &local len, &remote, &remote len)`:
/// where `AcceptEx` put the two addresses in the buffer.
pub(crate) fn get_accept_ex_sockaddrs(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let abi = task.abi();
    let local = a[0] + (a[1] as u32 as u64);
    let remote = local + (a[2] as u32 as u64);
    for (addr, ptr_at, len_at) in [(local, a[4], a[5]), (remote, a[6], a[7])] {
        let family = task.bytes_at(cx, addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
        let len: u32 = if family == AF_INET6 { 28 } else { 16 };
        if ptr_at != 0 {
            let _ = task.write_at(cx, ptr_at, &abi.ptr(addr));
        }
        if len_at != 0 {
            let _ = task.write_at(cx, len_at, &len.to_le_bytes());
        }
    }
    Reply::Value(0)
}

/// `DisconnectEx(s, overlapped, flags, reserved)`: both ways shut, at once.
fn disconnect_ex(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (file, sock) = task.sock(a[0])?;
        let _ = sock.net().shutdown(net::Shutdown::Both);
        if a[1] != 0 {
            let op = task.socket_op(cx, &file, a[1], 0, 0);
            let k = task.k.clone();
            finish(&k, cx, &op, &Outcome { writes: Vec::new(), n: 0, error: 0 });
        }
        Ok(1)
    })();
    match r {
        Ok(v) => Reply::Value(v),
        Err(Failed::Error(e)) => fail_with(task, cx, e, 0),
        Err(Failed::Reply(reply)) => reply,
    }
}

fn net_pump(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    crate::sockets::pump(task, cx)
}

/// `WSALookupServiceBegin(&query, flags, &lookup)`, `Next` and `End`: no name-space provider is installed (names are
/// looked up through `getaddrinfo`, and network location awareness isn't there), so WSASERVICE_NOT_FOUND, as Windows
/// says for a provider it hasn't.
fn wsa_lookup_service(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    task.set_last_error(cx, 10108);
    Reply::Value(u64::from(u32::MAX))
}

// ---- Event selection ----

/// `WSACreateEvent()`: a manual-reset event, not set.
fn wsa_create_event(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    create_event_w(task, cx, &[0, 1, 0, 0])
}

impl Task {
    /// Wakes the process's network thread, if it has one, to look at its sockets again.
    fn wake_net(&self) {
        if let Some(w) = &lock(&self.process.net).waiter {
            w.wake();
        }
    }
}

/// `WSAEventSelect(s, event, events)`: the network events to record and set the event for, as they happen (the
/// socket becomes non-blocking); no event and none asked stops it.
fn wsa_event_select(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let mask = a[2] as u32;
        if a[1] == 0 && mask == 0 {
            *lock(&sock.selected) = None;
            return Ok(0);
        }
        let event = task.object(a[1]).ok_or(Failed::Error(WSAEINVAL))?;
        sock.nonblocking.store(true, Ordering::Relaxed);
        *lock(&sock.selected) = Some(Selected { event, mask, recorded: 0, errors: [0; 10], armed: u32::MAX });
        task.net_watch(cx, &sock);
        Ok(0)
    })();
    answer(task, cx, r)
}

/// `WSAEnumNetworkEvents(s, event, &events)`: the network events recorded and their errors (WSANETWORKEVENTS), which
/// it clears, and the event reset if it's given.
fn wsa_enum_network_events(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let r = (|| -> Result<u64, Failed> {
        let (_, sock) = task.sock(a[0])?;
        let (recorded, errors) = match lock(&sock.selected).as_mut() {
            Some(s) => {
                let taken = (s.recorded, s.errors);
                s.recorded = 0;
                s.errors = [0; 10];
                taken
            }
            None => (0, [0; 10]),
        };
        if a[1] != 0 {
            let event = task.object(a[1]).ok_or(Failed::Error(WSAEINVAL))?;
            task.k.objects.signal(&event, |s| s.set = false);
        }
        let mut b = recorded.to_le_bytes().to_vec();
        for e in errors {
            b.extend(e.to_le_bytes());
        }
        task.write_at(cx, a[2], &b)?;
        Ok(0)
    })();
    answer(task, cx, r)
}
