//! The machine's DNS server, at its gateway's port 53 (UDP). It answers the machine's own name itself, looks up
//! addresses (A and AAAA) the host's way, and asks the host's DNS servers everything else, passing their answers
//! through. Each question is answered on a thread of its own, since the host's lookups wait.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Weak};

use crate::socket::{Datagram, Inner};
use crate::{Addr, Error, Shared};

const A: u16 = 1;
const AAAA: u16 = 28;
const IN: u16 = 1;
const NOERROR: u8 = 0;
const SERVFAIL: u8 = 2;
const NXDOMAIN: u8 = 3;
const NOTIMP: u8 = 4;
/// What a reply over UDP fits in, without EDNS.
const MOST_UDP: usize = 512;
/// How long an answer may be kept.
const TTL: u32 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Question {
    pub opcode: u8,
    /// The name, lowercased, without its final dot.
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
    /// Where the question ends in the packet.
    pub end: usize,
}

impl Question {
    pub fn parse(packet: &[u8]) -> Option<Question> {
        if packet.len() < 12 || packet[2] & 0x80 != 0 || u16::from_be_bytes([packet[4], packet[5]]) != 1 {
            return None;
        }
        let opcode = (packet[2] >> 3) & 0xF;
        let mut at = 12;
        let mut labels = Vec::new();
        loop {
            let len = *packet.get(at)? as usize;
            at += 1;
            if len == 0 {
                break;
            }
            // A question's name is never compressed.
            if len > 63 {
                return None;
            }
            labels.push(String::from_utf8_lossy(packet.get(at..at + len)?).to_ascii_lowercase());
            at += len;
        }
        let qtype = u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]);
        let qclass = u16::from_be_bytes([*packet.get(at + 2)?, *packet.get(at + 3)?]);
        Some(Question { opcode, name: labels.join("."), qtype, qclass, end: at + 4 })
    }
}

/// Answers `query`, which `reply_to` sent to the gateway, in time: the reply comes to it from `from`.
pub(crate) fn answer(net: &Arc<Shared>, query: Vec<u8>, reply_to: Weak<Inner>, from: SocketAddr) {
    let Some(question) = Question::parse(&query) else { return };
    let net = net.clone();
    let _ = std::thread::Builder::new().name("net dns".into()).spawn(move || {
        let reply = respond(&net, &query, &question);
        if let Some(socket) = reply_to.upgrade() {
            let _ = socket.deliver(Datagram { from: Addr::Inet(from), data: reply, extra: None, cred: None }, &mut None);
        }
    });
}

/// The reply to `query`.
pub(crate) fn respond(net: &Shared, query: &[u8], question: &Question) -> Vec<u8> {
    if question.opcode != 0 {
        return build(query, question, NOTIMP, &[]);
    }
    if let Some(server) = net.config.dns {
        return net.host.query(query, Some(server)).ok().filter(|r| r.get(..2) == query.get(..2)).unwrap_or_else(|| build(query, question, SERVFAIL, &[]));
    }
    let addresses = question.qclass == IN && matches!(question.qtype, A | AAAA);
    let hostname = net.config.hostname.to_ascii_lowercase();
    let own: Option<Vec<IpAddr>> = if !hostname.is_empty() && question.name == hostname {
        Some(vec![IpAddr::V4(net.address)])
    } else if question.name == "localhost" {
        Some(vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)])
    } else {
        None
    };
    if let Some(own) = own {
        return build(query, question, NOERROR, if addresses { &own } else { &[] });
    }
    if addresses {
        return match net.host.lookup(&question.name) {
            Ok(found) => build(query, question, NOERROR, &found),
            Err(Error::NotFound) => build(query, question, NXDOMAIN, &[]),
            Err(_) => build(query, question, SERVFAIL, &[]),
        };
    }
    net.host.query(query, None).ok().filter(|r| r.len() >= 12 && r.get(..2) == query.get(..2)).unwrap_or_else(|| build(query, question, SERVFAIL, &[]))
}

/// A reply to `query` with `rcode` and, of `found`, the addresses of the type asked for.
pub(crate) fn build(query: &[u8], question: &Question, rcode: u8, found: &[IpAddr]) -> Vec<u8> {
    let mut answers: Vec<Vec<u8>> = Vec::new();
    for ip in found {
        let rdata: Vec<u8> = match (question.qtype, ip) {
            (A, IpAddr::V4(v4)) => v4.octets().to_vec(),
            (AAAA, IpAddr::V6(v6)) if v6.to_ipv4_mapped().is_none() => v6.octets().to_vec(),
            _ => continue,
        };
        let mut record = vec![0xC0, 0x0C];
        record.extend(question.qtype.to_be_bytes());
        record.extend(IN.to_be_bytes());
        record.extend(TTL.to_be_bytes());
        record.extend((rdata.len() as u16).to_be_bytes());
        record.extend(rdata);
        if !answers.contains(&record) {
            answers.push(record);
        }
    }
    let mut reply = Vec::with_capacity(MOST_UDP);
    reply.extend(&query[..2]);
    // A response, its opcode, recursion desired as asked, recursion available.
    reply.push(0x80 | (question.opcode << 3) | (query[2] & 0x01));
    reply.push(0x80 | rcode);
    reply.extend(1u16.to_be_bytes());
    let fits = answers.iter().scan(question.end, |len, a| {
        *len += a.len();
        Some(*len <= MOST_UDP)
    });
    let count = fits.take_while(|f| *f).count();
    reply.extend((count as u16).to_be_bytes());
    reply.extend([0, 0, 0, 0]);
    reply.extend(&query[12..question.end]);
    for record in &answers[..count] {
        reply.extend(record);
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query for `name` of `qtype`, as a resolver sends it.
    pub(crate) fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut q = id.to_be_bytes().to_vec();
        q.extend([0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend(label.as_bytes());
        }
        q.push(0);
        q.extend(qtype.to_be_bytes());
        q.extend(IN.to_be_bytes());
        q
    }

    #[test]
    fn a_question_is_read_and_answered() {
        let q = query(0x1234, "Example.COM", A);
        let question = Question::parse(&q).unwrap();
        assert_eq!((question.name.as_str(), question.qtype, question.end), ("example.com", A, q.len()));
        let reply = build(&q, &question, NOERROR, &[IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), IpAddr::V6(Ipv6Addr::LOCALHOST)]);
        assert_eq!(&reply[..2], &[0x12, 0x34]);
        assert_eq!(reply[2] & 0x80, 0x80);
        assert_eq!(u16::from_be_bytes([reply[6], reply[7]]), 1);
        assert_eq!(&reply[reply.len() - 4..], &[1, 2, 3, 4]);
        let nx = build(&q, &question, NXDOMAIN, &[]);
        assert_eq!(nx[3] & 0xF, NXDOMAIN);
        assert_eq!(nx.len(), q.len());
    }

    #[test]
    fn answers_stop_at_512_bytes() {
        let q = query(1, "many.example", A);
        let question = Question::parse(&q).unwrap();
        let found: Vec<IpAddr> = (0..100).map(|i| IpAddr::V4(Ipv4Addr::new(10, 0, 0, i))).collect();
        let reply = build(&q, &question, NOERROR, &found);
        assert!(reply.len() <= MOST_UDP);
        assert_eq!(u16::from_be_bytes([reply[6], reply[7]]) as usize, (MOST_UDP - q.len()) / 16);
    }
}
