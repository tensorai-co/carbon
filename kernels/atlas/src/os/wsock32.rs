//! wsock32.dll: Winsock 1's DLL, for what programs still import from it by ordinal.

use super::dll;

dll! {
    "WSOCK32.dll";
    missing TransmitFile @ 1140;
    missing AcceptEx;
    missing GetAcceptExSockaddrs;
}
