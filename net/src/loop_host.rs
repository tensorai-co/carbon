//! `Loop`: a host with no sockets, an internet in memory with scripted peers, so `net` and the kernels can be tested
//! with no network.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use crate::{Error, Host, HostDatagram, HostStream, Ready, Result, Shutdown, Waker, lock};

/// What's at an address on a [`Loop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// Accepts, and sends back what it's sent (TCP), or each datagram (UDP); ends when the machine's end does.
    Echo,
    /// Refuses connections.
    Refuse,
    /// Accepts, then resets the connection at the first thing it's sent.
    Reset,
    /// Never answers: the connection stays connecting.
    Stall,
}

type Answer = Box<dyn Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync>;

/// An internet in memory.
#[derive(Default)]
pub struct Loop {
    peers: Mutex<HashMap<SocketAddr, Peer>>,
    names: Mutex<HashMap<String, Vec<IpAddr>>>,
    dns: Mutex<Option<Answer>>,
}

impl Loop {
    pub fn new() -> Arc<Loop> {
        Arc::new(Loop::default())
    }

    pub fn peer(&self, at: SocketAddr, peer: Peer) {
        lock(&self.peers).insert(at, peer);
    }

    /// What looking `name` up finds.
    pub fn name(&self, name: &str, addrs: &[IpAddr]) {
        lock(&self.names).insert(name.to_ascii_lowercase(), addrs.to_vec());
    }

    /// How the host's DNS servers answer a query.
    pub fn dns(&self, answer: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync + 'static) {
        *lock(&self.dns) = Some(Box::new(answer));
    }
}

struct LoopStream {
    peer: Option<Peer>,
    state: Mutex<StreamState>,
    wake: Waker,
}

#[derive(Default)]
struct StreamState {
    /// What the peer has sent, unread.
    incoming: VecDeque<u8>,
    /// The peer won't send again.
    ended: bool,
    reset: bool,
    shut_write: bool,
}

impl HostStream for LoopStream {
    fn connected(&self) -> Result<bool> {
        match self.peer {
            Some(Peer::Echo | Peer::Reset) => Ok(true),
            Some(Peer::Refuse) => Err(Error::ConnectionRefused),
            Some(Peer::Stall) => Ok(false),
            None => Err(Error::HostUnreachable),
        }
    }

    fn ready(&self) -> Ready {
        if self.connected() != Ok(true) {
            return Ready::default();
        }
        let state = lock(&self.state);
        Ready {
            read: !state.incoming.is_empty() || state.ended || state.reset,
            write: !state.shut_write && !state.reset,
            read_closed: state.ended,
            hangup: state.ended && state.shut_write,
            error: state.reset,
        }
    }

    fn read(&self, buffer: &mut [u8]) -> Result<usize> {
        let mut state = lock(&self.state);
        if state.reset {
            return Err(Error::ConnectionReset);
        }
        if state.incoming.is_empty() {
            return if state.ended { Ok(0) } else { Err(Error::WouldBlock) };
        }
        let n = buffer.len().min(state.incoming.len());
        for (b, byte) in buffer.iter_mut().zip(state.incoming.drain(..n)) {
            *b = byte;
        }
        Ok(n)
    }

    fn write(&self, data: &[u8]) -> Result<usize> {
        {
            let mut state = lock(&self.state);
            if state.reset {
                return Err(Error::ConnectionReset);
            }
            if state.shut_write {
                return Err(Error::BrokenPipe);
            }
            match self.peer {
                Some(Peer::Reset) => state.reset = true,
                _ => state.incoming.extend(data),
            }
        }
        (self.wake)();
        Ok(data.len())
    }

    fn shutdown(&self, how: Shutdown) {
        {
            let mut state = lock(&self.state);
            if matches!(how, Shutdown::Write | Shutdown::Both) {
                state.shut_write = true;
                // An echo ends when the machine's end does.
                state.ended = true;
            }
        }
        (self.wake)();
    }
}

struct LoopDatagram {
    host: Arc<Loop>,
    incoming: Mutex<VecDeque<(Vec<u8>, SocketAddr)>>,
    wake: Waker,
}

impl HostDatagram for LoopDatagram {
    fn ready(&self) -> Ready {
        Ready { read: !lock(&self.incoming).is_empty(), write: true, ..Ready::default() }
    }

    fn send_to(&self, data: &[u8], to: SocketAddr) -> Result<usize> {
        if lock(&self.host.peers).get(&to) == Some(&Peer::Echo) {
            lock(&self.incoming).push_back((data.to_vec(), to));
            (self.wake)();
        }
        Ok(data.len())
    }

    fn recv_from(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let (data, from) = lock(&self.incoming).pop_front().ok_or(Error::WouldBlock)?;
        let n = data.len().min(buffer.len());
        buffer[..n].copy_from_slice(&data[..n]);
        Ok((n, from))
    }
}

/// [`Loop`]'s `Host`. It's implemented on `Arc<Loop>`, so its UDP sockets can find its peers.
impl Host for Arc<Loop> {
    fn connect(&self, to: SocketAddr, wake: Waker) -> Box<dyn HostStream> {
        let peer = lock(&self.peers).get(&to).copied();
        Box::new(LoopStream { peer, state: Mutex::default(), wake })
    }

    fn udp(&self, _v6: bool, wake: Waker) -> Result<Box<dyn HostDatagram>> {
        Ok(Box::new(LoopDatagram { host: self.clone(), incoming: Mutex::default(), wake }))
    }

    fn lookup(&self, name: &str) -> Result<Vec<IpAddr>> {
        lock(&self.names).get(&name.to_ascii_lowercase()).cloned().ok_or(Error::NotFound)
    }

    fn query(&self, packet: &[u8], _server: Option<SocketAddr>) -> Result<Vec<u8>> {
        lock(&self.dns).as_ref().and_then(|answer| answer(packet)).ok_or(Error::TimedOut)
    }

    fn addresses(&self) -> Vec<IpAddr> {
        // As if the host were on 10.89.0.0/16 itself, so a machine's network has to be the next one.
        vec![IpAddr::V4(Ipv4Addr::new(10, 89, 4, 7)), IpAddr::V4(Ipv4Addr::LOCALHOST)]
    }
}
