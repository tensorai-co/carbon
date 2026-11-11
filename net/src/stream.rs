//! A connection inside the machine: two ends joined in memory, for Unix stream and seqpacket sockets and for TCP over
//! loopback. Each way has its own buffer; each end tells the other's watchers when it writes, reads or goes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::watch::Watchers;
use crate::{Ancillary, Cred, Error, Ready, Result, Shutdown, lock};

/// How much one way holds before a writer waits.
pub(crate) const CAPACITY: usize = 256 << 10;

pub(crate) struct Segment {
    pub data: Vec<u8>,
    pub extra: Option<Ancillary>,
    pub cred: Option<Cred>,
}

#[derive(Default)]
struct Way {
    segments: VecDeque<Segment>,
    bytes: usize,
    /// The writer won't write again: what's here is all there is.
    eof: bool,
    /// The writer went with data of its own unread: a reset, for TCP.
    reset: bool,
    /// The reader won't read again: writing fails.
    reader_gone: bool,
}

pub(crate) struct Duplex {
    /// `ways[s]` is what end `s` reads.
    ways: Mutex<[Way; 2]>,
    watchers: [Arc<Watchers>; 2],
    /// Seqpacket: each write is a message, read whole or cut short.
    messages: bool,
    /// TCP: a reset when an end goes with data unread.
    tcp: bool,
}

/// One end of a [`Duplex`].
pub(crate) struct End {
    duplex: Arc<Duplex>,
    side: usize,
}

/// What a read got.
pub(crate) struct Got {
    pub data: Vec<u8>,
    pub extra: Option<Ancillary>,
    pub cred: Option<Cred>,
    /// The message was longer than what was asked for (seqpacket), and how long.
    pub full: usize,
}

/// Two ends joined, each telling its own watchers.
pub(crate) fn join(watchers: [Arc<Watchers>; 2], messages: bool, tcp: bool) -> (End, End) {
    let duplex = Arc::new(Duplex { ways: Mutex::new([Way::default(), Way::default()]), watchers, messages, tcp });
    (End { duplex: duplex.clone(), side: 0 }, End { duplex, side: 1 })
}

impl End {
    fn other(&self) -> usize {
        1 - self.side
    }

    fn tell_other(&self) {
        self.duplex.watchers[self.other()].notify();
    }

    /// Writes what fits of `data`, taking `extra` with it only if something's written.
    pub fn send(&self, data: &[u8], extra: &mut Option<Ancillary>, cred: Option<Cred>) -> Result<usize> {
        let n = {
            let mut ways = lock(&self.duplex.ways);
            let reset = ways[self.side].reset;
            let out = &mut ways[self.other()];
            if out.reader_gone || out.eof {
                return Err(if reset && self.duplex.tcp { Error::ConnectionReset } else { Error::BrokenPipe });
            }
            let room = CAPACITY.saturating_sub(out.bytes);
            let n = if self.duplex.messages {
                if data.len() > CAPACITY {
                    return Err(Error::MessageTooBig);
                }
                if room < data.len() {
                    return Err(Error::WouldBlock);
                }
                data.len()
            } else {
                if room == 0 {
                    return Err(Error::WouldBlock);
                }
                data.len().min(room)
            };
            // Bytes with nothing attached join the last write's, so a stream stays a few segments long.
            let joins = !self.duplex.messages && extra.is_none() && out.segments.back().is_some_and(|s| s.extra.is_none() && s.cred == cred);
            match out.segments.back_mut() {
                Some(last) if joins => last.data.extend_from_slice(&data[..n]),
                _ => out.segments.push_back(Segment { data: data[..n].to_vec(), extra: extra.take(), cred }),
            }
            out.bytes += n;
            n
        };
        self.tell_other();
        Ok(n)
    }

    pub fn recv(&self, len: usize, peek: bool) -> Result<Got> {
        let got = {
            let mut ways = lock(&self.duplex.ways);
            let way = &mut ways[self.side];
            if way.segments.is_empty() {
                if way.reset && self.duplex.tcp {
                    // Said once; after it, the connection reads as ended.
                    way.reset = false;
                    way.eof = true;
                    return Err(Error::ConnectionReset);
                }
                if way.eof {
                    return Ok(Got { data: Vec::new(), extra: None, cred: None, full: 0 });
                }
                return Err(Error::WouldBlock);
            }
            if self.duplex.messages {
                let first = &way.segments[0];
                let full = first.data.len();
                let data = first.data[..len.min(full)].to_vec();
                let cred = first.cred;
                if peek {
                    Got { data, extra: None, cred, full }
                } else {
                    let first = way.segments.pop_front().unwrap_or_else(|| unreachable!());
                    way.bytes -= full;
                    Got { data, extra: first.extra, cred, full }
                }
            } else {
                let mut data = Vec::new();
                let mut extra = None;
                let mut cred = None;
                let mut i = 0;
                while data.len() < len && i < way.segments.len() {
                    let segment = &way.segments[i];
                    // What's attached to a later write is read with that write's first byte, not before.
                    if i > 0 && (segment.extra.is_some() || segment.cred != cred) {
                        break;
                    }
                    if i == 0 {
                        cred = segment.cred;
                    }
                    let n = (len - data.len()).min(segment.data.len());
                    data.extend_from_slice(&segment.data[..n]);
                    i += 1;
                }
                if !peek {
                    extra = way.segments[0].extra.take();
                    let mut left = data.len();
                    while left > 0 {
                        let front = &mut way.segments[0];
                        if front.data.len() <= left {
                            left -= front.data.len();
                            way.segments.pop_front();
                        } else {
                            front.data.drain(..left);
                            left = 0;
                        }
                    }
                    way.bytes -= data.len();
                }
                let full = data.len();
                Got { data, extra, cred, full }
            }
        };
        if !peek {
            self.tell_other();
        }
        Ok(got)
    }

    pub fn shutdown(&self, how: Shutdown) {
        {
            let mut ways = lock(&self.duplex.ways);
            if matches!(how, Shutdown::Write | Shutdown::Both) {
                ways[self.other()].eof = true;
            }
            if matches!(how, Shutdown::Read | Shutdown::Both) {
                let way = &mut ways[self.side];
                way.eof = true;
                way.reader_gone = true;
                way.segments.clear();
                way.bytes = 0;
            }
        }
        self.tell_other();
        self.duplex.watchers[self.side].notify();
    }

    pub fn ready(&self) -> Ready {
        let ways = lock(&self.duplex.ways);
        let (inn, out) = (&ways[self.side], &ways[self.other()]);
        let write_open = !out.reader_gone && !out.eof;
        Ready {
            read: !inn.segments.is_empty() || inn.eof || inn.reset,
            write: write_open && out.bytes < CAPACITY,
            read_closed: inn.eof,
            hangup: inn.eof && !write_open,
            error: inn.reset && self.duplex.tcp,
        }
    }

    /// What a read would get now: the bytes waiting, or a message's length (`FIONREAD`).
    pub fn available(&self) -> usize {
        let ways = lock(&self.duplex.ways);
        let way = &ways[self.side];
        if self.duplex.messages { way.segments.front().map_or(0, |s| s.data.len()) } else { way.bytes }
    }

    /// Bytes waiting to be read, and bytes written but not yet read by the other end.
    pub fn queued(&self) -> (usize, usize) {
        let ways = lock(&self.duplex.ways);
        (ways[self.side].bytes, ways[self.other()].bytes)
    }
}

impl Drop for End {
    fn drop(&mut self) {
        {
            let mut ways = lock(&self.duplex.ways);
            let unread = ways[self.side].bytes > 0;
            let way = &mut ways[self.side];
            way.reader_gone = true;
            way.segments.clear();
            way.bytes = 0;
            let out = &mut ways[self.other()];
            out.eof = true;
            if unread && self.duplex.tcp {
                out.reset = true;
            }
        }
        self.tell_other();
    }
}
