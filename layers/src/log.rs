//! The machine's log: every change to the disk since its index, as records appended to the store, replayed when the disk
//! is opened again. A record is its length and a checksum, then what it says; a crash can leave the last one cut short
//! or garbled, and replay stops there, so the disk comes back as it was at the last record that was whole.
//!
//! Changes to names and metadata are logged as they happen. A file's contents are logged when the disk syncs: its new
//! size and the chunks that changed, which are in the store by then.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Ino;
use crate::store::Hash;

/// What a record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Record {
    /// `name` made in `dir` as `ino`: a file (0), directory (1) or symbolic link (2) to `target`.
    Create { dir: Ino, ino: Ino, kind: u8, meta: [u32; 4], now: u64, name: String, target: String },
    Link { dir: Ino, ino: Ino, now: u64, name: String },
    Remove { dir: Ino, now: u64, name: String },
    Rename { from_dir: Ino, to_dir: Ino, now: u64, from: String, to: String },
    Meta { ino: Ino, meta: [u32; 4] },
    Modified { ino: Ino, now: u64 },
    /// A file's contents since the last of these: cut to `cut` (the smallest it's been since), then `size`, with each
    /// chunk that changed, by its number, with its name or none for a hole; and its time.
    File { ino: Ino, cut: u64, size: u64, modified: u64, chunks: Vec<(u32, Option<Hash>)> },
}

/// FNV-1a: enough to tell a record that's whole from one a crash cut short.
fn check(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for byte in bytes {
        h = (h ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
    }
    h
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn meta(&mut self, meta: &[u32; 4]) {
        for word in meta {
            self.u32(*word);
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let (head, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
    fn meta(&mut self) -> Option<[u32; 4]> {
        Some([self.u32()?, self.u32()?, self.u32()?, self.u32()?])
    }
}

impl Record {
    /// The record as it goes in the log, framed.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        match self {
            Record::Create { dir, ino, kind, meta, now, name, target } => {
                w.u8(1);
                w.u64(*dir);
                w.u64(*ino);
                w.u8(*kind);
                w.meta(meta);
                w.u64(*now);
                w.str(name);
                w.str(target);
            }
            Record::Link { dir, ino, now, name } => {
                w.u8(2);
                w.u64(*dir);
                w.u64(*ino);
                w.u64(*now);
                w.str(name);
            }
            Record::Remove { dir, now, name } => {
                w.u8(3);
                w.u64(*dir);
                w.u64(*now);
                w.str(name);
            }
            Record::Rename { from_dir, to_dir, now, from, to } => {
                w.u8(4);
                w.u64(*from_dir);
                w.u64(*to_dir);
                w.u64(*now);
                w.str(from);
                w.str(to);
            }
            Record::Meta { ino, meta } => {
                w.u8(5);
                w.u64(*ino);
                w.meta(meta);
            }
            Record::Modified { ino, now } => {
                w.u8(6);
                w.u64(*ino);
                w.u64(*now);
            }
            Record::File { ino, cut, size, modified, chunks } => {
                w.u8(7);
                w.u64(*ino);
                w.u64(*cut);
                w.u64(*size);
                w.u64(*modified);
                w.u32(chunks.len() as u32);
                for (number, hash) in chunks {
                    w.u32(*number);
                    w.u8(u8::from(hash.is_some()));
                    if let Some(hash) = hash {
                        w.0.extend_from_slice(hash);
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(w.0.len() + 8);
        out.extend_from_slice(&(w.0.len() as u32).to_le_bytes());
        out.extend_from_slice(&check(&w.0).to_le_bytes());
        out.extend_from_slice(&w.0);
        out
    }

    fn decode(payload: &[u8]) -> Option<Record> {
        let mut r = Reader(payload);
        let record = match r.u8()? {
            1 => Record::Create { dir: r.u64()?, ino: r.u64()?, kind: r.u8()?, meta: r.meta()?, now: r.u64()?, name: r.str()?, target: r.str()? },
            2 => Record::Link { dir: r.u64()?, ino: r.u64()?, now: r.u64()?, name: r.str()? },
            3 => Record::Remove { dir: r.u64()?, now: r.u64()?, name: r.str()? },
            4 => Record::Rename { from_dir: r.u64()?, to_dir: r.u64()?, now: r.u64()?, from: r.str()?, to: r.str()? },
            5 => Record::Meta { ino: r.u64()?, meta: r.meta()? },
            6 => Record::Modified { ino: r.u64()?, now: r.u64()? },
            7 => {
                let (ino, cut, size, modified) = (r.u64()?, r.u64()?, r.u64()?, r.u64()?);
                let count = r.u32()? as usize;
                let mut chunks = Vec::with_capacity(count.min(1 << 16));
                for _ in 0..count {
                    let number = r.u32()?;
                    let hash = if r.u8()? != 0 { Some(r.take(32)?.try_into().ok()?) } else { None };
                    chunks.push((number, hash));
                }
                Record::File { ino, cut, size, modified, chunks }
            }
            _ => return None,
        };
        r.0.is_empty().then_some(record)
    }
}

/// The records in a log, as far as they're whole, and how many bytes that is: what's past it is cut off.
pub(crate) fn read(log: &[u8]) -> (Vec<Record>, usize) {
    let mut records = Vec::new();
    let mut at = 0;
    while let Some(header) = log.get(at..at + 8) {
        let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let sum = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let Some(payload) = log.get(at + 8..at + 8 + len) else { break };
        if check(payload) != sum {
            break;
        }
        let Some(record) = Record::decode(payload) else { break };
        records.push(record);
        at += 8 + len;
    }
    (records, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn records_round_trip_and_a_torn_one_is_dropped() {
        let records = vec![
            Record::Create { dir: 1, ino: 2, kind: 0, meta: [1, 2, 3, 4], now: 5, name: "a".to_string(), target: String::new() },
            Record::Rename { from_dir: 1, to_dir: 1, now: 6, from: "a".to_string(), to: "b".to_string() },
            Record::File { ino: 2, cut: 0, size: 70_000, modified: 7, chunks: vec![(0, Some([9; 32])), (1, None)] },
        ];
        let mut log: Vec<u8> = records.iter().flat_map(Record::encode).collect();
        let whole = log.len();
        assert_eq!(read(&log), (records.clone(), whole));
        // Half of another record, as a crash leaves it.
        let extra = Record::Remove { dir: 1, now: 8, name: "b".to_string() }.encode();
        log.extend_from_slice(&extra[..extra.len() / 2]);
        assert_eq!(read(&log), (records.clone(), whole));
        // A byte gone wrong in the last one.
        log.truncate(whole);
        log[whole - 1] ^= 1;
        let (got, len) = read(&log);
        assert_eq!(got, records[..2].to_vec());
        assert!(len < whole);
    }
}
