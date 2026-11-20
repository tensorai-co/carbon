//! A byte answer checked as it streams: what's read is held back by one read, and the last read is only given once
//! the whole matches what was expected. One that doesn't is an error instead, so the tool gets a short file.

use std::io::{self, Read};

use sha2::{Digest, Sha256, Sha512};

use crate::Expect;

enum Sum {
    None,
    Sha256(Sha256, Vec<u8>),
    Sha512(Sha512, Vec<u8>),
}

/// A reader that gives `inner`'s bytes, and an error instead of the last of them if they don't match.
pub struct Verified {
    inner: Box<dyn Read + Send>,
    sum: Sum,
    size: Option<u64>,
    read: u64,
    /// The newest read, not given yet.
    held: Vec<u8>,
    /// What's let go, and how much of it has been given.
    ready: Vec<u8>,
    at: usize,
    done: bool,
}

impl Verified {
    pub fn new(inner: Box<dyn Read + Send>, expect: &Expect) -> Verified {
        let unhex = |s: &str| (0..s.len() / 2).filter_map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Vec<u8>>();
        let sum = match (&expect.sha512, &expect.sha256) {
            (Some(h), _) => Sum::Sha512(Sha512::new(), unhex(h)),
            (None, Some(h)) => Sum::Sha256(Sha256::new(), unhex(h)),
            (None, None) => Sum::None,
        };
        Verified { inner, sum, size: expect.size, read: 0, held: Vec::new(), ready: Vec::new(), at: 0, done: false }
    }

    fn matches(&mut self) -> bool {
        let sum_ok = match std::mem::replace(&mut self.sum, Sum::None) {
            Sum::None => true,
            Sum::Sha256(h, want) => h.finalize().as_slice() == want.as_slice(),
            Sum::Sha512(h, want) => h.finalize().as_slice() == want.as_slice(),
        };
        sum_ok && self.size.is_none_or(|n| n == self.read)
    }
}

impl Read for Verified {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.at < self.ready.len() {
                let n = buf.len().min(self.ready.len() - self.at);
                buf[..n].copy_from_slice(&self.ready[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }
            let mut next = vec![0u8; 64 << 10];
            let n = self.inner.read(&mut next)?;
            next.truncate(n);
            if n == 0 {
                // The end: what's held goes only if the whole matched.
                self.done = true;
                if !self.matches() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "the file doesn't match what was expected of it"));
                }
                next = Vec::new();
            } else {
                self.read += n as u64;
                match &mut self.sum {
                    Sum::None => {}
                    Sum::Sha256(h, _) => h.update(&next),
                    Sum::Sha512(h, _) => h.update(&next),
                }
            }
            // What was held is let go; what was just read is held in its place.
            self.ready = std::mem::replace(&mut self.held, next);
            self.at = 0;
        }
    }
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Standard base64, with or without its padding; None if it isn't.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let text = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= value(c)? << (18 - 6 * i);
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8]),
            2 => out.push((acc >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that gives its bytes a few at a time, as a socket would.
    struct Dribble(Vec<u8>, usize);
    impl Read for Dribble {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(self.1).min(self.0.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0.drain(..n);
            Ok(n)
        }
    }

    fn read_all(r: &mut Verified, step: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; step];
        loop {
            let n = r.read(&mut buf)?;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    #[test]
    fn a_matching_file_comes_through_whole_and_a_bad_one_never_does() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let sum = hex(&Sha512::digest(&data));
        for (dribble, step) in [(7, 3), (100_000, 64 << 10), (64 << 10, 10)] {
            let expect = Expect { sha512: Some(sum.clone()), size: Some(data.len() as u64), ..Expect::default() };
            let mut ok = Verified::new(Box::new(Dribble(data.clone(), dribble)), &expect);
            assert_eq!(read_all(&mut ok, step).unwrap(), data);
            let mut bad_data = data.clone();
            bad_data[150_000] ^= 1;
            let mut bad = Verified::new(Box::new(Dribble(bad_data, dribble)), &expect);
            let mut got = Vec::new();
            let mut buf = vec![0u8; step];
            let ended = loop {
                match bad.read(&mut buf) {
                    Ok(0) => break Ok(()),
                    Ok(n) => got.extend_from_slice(&buf[..n]),
                    Err(e) => break Err(e),
                }
            };
            assert!(ended.is_err() && got.len() < data.len(), "{dribble}/{step}: {}", got.len());
        }
        let short = Expect { size: Some(10), ..Expect::default() };
        assert!(read_all(&mut Verified::new(Box::new(&b"123"[..]), &short), 8).is_err());
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
        assert!(base64_decode("a$==").is_none());
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
    }
}
