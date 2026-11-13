//! ws2_32.dll: Winsock, at Windows' own ordinals (programs import its oldest functions by number). Starting it up,
//! byte order and its last error are here; sockets themselves aren't written yet.

use kernels_core::thread::Cx;

use super::{Reply, dll};
use crate::process::Task;

dll! {
    "WS2_32.dll";
    missing accept;
    missing bind;
    missing closesocket;
    missing connect;
    missing getpeername;
    missing getsockname;
    missing getsockopt;
    host htonl(1) => byte_swap32;
    host htons(1) => byte_swap16;
    missing ioctlsocket;
    missing inet_addr;
    missing inet_ntoa;
    missing listen;
    host ntohl(1) => byte_swap32;
    host ntohs(1) => byte_swap16;
    missing recv;
    missing recvfrom;
    missing select;
    missing send;
    missing sendto;
    missing setsockopt;
    missing shutdown;
    missing socket;
    missing gethostbyaddr @ 51;
    missing gethostbyname;
    missing getprotobyname;
    missing getprotobynumber;
    missing getservbyname;
    missing getservbyport;
    missing gethostname;
    guest WSAGetLastError @ 111 => RtlGetLastWin32Error;
    guest WSASetLastError => RtlSetLastWin32Error;
    host WSAStartup(2) @ 115 => wsa_startup;
    host WSACleanup(0) => wsa_cleanup;
    host __WSAFDIsSet(2) @ 151 => wsa_fd_is_set;
    missing WSASocketA;
    missing WSASocketW;
    missing WSAIoctl;
    missing WSARecv;
    missing WSARecvFrom;
    missing WSASend;
    missing WSASendTo;
    missing getaddrinfo;
    missing freeaddrinfo;
    missing GetAddrInfoW;
    missing FreeAddrInfoW;
}

fn byte_swap32(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from((a[0] as u32).swap_bytes()))
}

fn byte_swap16(_: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from((a[0] as u16).swap_bytes()))
}

const WSAVERNOTSUPPORTED: u64 = 10092;
const WSAEFAULT: u64 = 10014;
const WSANOTINITIALISED: u32 = 10093;

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
    let wide = task.abi().width == 8;
    let mut data = Vec::new();
    data.extend(version.to_le_bytes());
    data.extend(0x0202u16.to_le_bytes());
    if wide {
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
        return Reply::Value(WSAEFAULT);
    }
    task.process_winsock(1);
    Reply::Value(0)
}

/// `WSACleanup()`: one `WSAStartup` undone; SOCKET_ERROR with WSANOTINITIALISED if there's none.
fn wsa_cleanup(task: &mut Task, cx: &mut Cx<'_>, _: &[u64]) -> Reply {
    if !task.process_winsock(-1) {
        task.set_last_error(cx, WSANOTINITIALISED);
        return Reply::Value(u64::from(u32::MAX));
    }
    Reply::Value(0)
}

/// `__WSAFDIsSet(socket, &set)`: whether an fd_set (a count, then that many SOCKETs) has the socket.
fn wsa_fd_is_set(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let abi = task.abi();
    let w = abi.width;
    let found = task.mem(cx, false, |t| {
        let count = u64::from(kernels_core::user::read_u32(&t.space(), a[1])?).min(64);
        // The count is a u_int, then the array, aligned as SOCKETs are.
        for i in 0..count {
            if kernels_core::user::read_ptr(&t.space(), a[1] + w + i * w, w)? == a[0] {
                return Ok(true);
            }
        }
        Ok(false)
    });
    Reply::Value(u64::from(found.unwrap_or(false)))
}

impl Task {
    /// Counts `WSAStartup`s less `WSACleanup`s for this process; false if a cleanup has none to undo.
    fn process_winsock(&self, by: i32) -> bool {
        use std::sync::atomic::{AtomicI64, Ordering};
        use std::collections::HashMap;
        use std::sync::Mutex;
        static STARTED: Mutex<Option<HashMap<u64, AtomicI64>>> = Mutex::new(None);
        let mut m = kernels_core::lock(&STARTED);
        let counts = m.get_or_insert_with(HashMap::new);
        let n = counts.entry(self.process.pid).or_insert_with(|| AtomicI64::new(0));
        if by < 0 && n.load(Ordering::Relaxed) <= 0 {
            return false;
        }
        n.fetch_add(i64::from(by), Ordering::Relaxed);
        true
    }
}
