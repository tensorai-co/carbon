//! wsock32.dll: Winsock 1's DLL, for what programs still import from it by ordinal: mswsock's `AcceptEx` and
//! `GetAcceptExSockaddrs`, as ws2_32 has them (`os/ws2_32.rs`).

use super::dll;
use super::ws2_32::{accept_ex, get_accept_ex_sockaddrs};

dll! {
    "WSOCK32.dll";
    missing TransmitFile @ 1140;
    host AcceptEx(8) => accept_ex;
    host GetAcceptExSockaddrs(8) => get_accept_ex_sockaddrs;
}
