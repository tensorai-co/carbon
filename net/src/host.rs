//! The host's part: what each host does for `net` with its own sockets. [`crate::System`] is the real host's;
//! [`crate::Loop`] is an internet in memory, for tests.

use std::net::{IpAddr, SocketAddr};

use crate::{Ready, Result, Shutdown, Waker};

pub trait Host: Send + Sync + 'static {
    /// Starts a TCP connection to `to`, without waiting for it. `wake` is called whenever the connection's readiness
    /// may have changed, from then until it's dropped.
    fn connect(&self, to: SocketAddr, wake: Waker) -> Box<dyn HostStream>;

    /// A UDP socket on any of the host's addresses (IPv6 if `v6`), with `wake` as for [`Host::connect`].
    fn udp(&self, v6: bool, wake: Waker) -> Result<Box<dyn HostDatagram>>;

    /// A name's addresses, looked up the host's way. It may wait: `net` calls it on a thread of its own.
    /// [`crate::Error::NotFound`] if there's no such name.
    fn lookup(&self, name: &str) -> Result<Vec<IpAddr>>;

    /// Asks a DNS server (`server`, or else the host's own) the query `packet` and gives its answer. It may wait, as
    /// [`Host::lookup`] may.
    fn query(&self, packet: &[u8], server: Option<SocketAddr>) -> Result<Vec<u8>>;

    /// The host's own addresses, so a machine's network can be on none of them.
    fn addresses(&self) -> Vec<IpAddr>;
}

/// A TCP connection out through the host. Every call returns at once.
pub trait HostStream: Send + Sync {
    /// Whether it's connected yet (`Ok(false)` while connecting), or why it couldn't be.
    fn connected(&self) -> Result<bool>;
    fn ready(&self) -> Ready;
    /// [`crate::Error::WouldBlock`] when there's nothing yet; 0 at the end.
    fn read(&self, buffer: &mut [u8]) -> Result<usize>;
    fn write(&self, data: &[u8]) -> Result<usize>;
    fn shutdown(&self, how: Shutdown);
    fn set_nodelay(&self, _on: bool) {}
    fn set_keepalive(&self, _on: bool) {}
}

/// A UDP socket on the host. Every call returns at once.
pub trait HostDatagram: Send + Sync {
    fn ready(&self) -> Ready;
    fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize>;
    /// [`crate::Error::WouldBlock`] when there's nothing yet.
    fn recv_from(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)>;
}
