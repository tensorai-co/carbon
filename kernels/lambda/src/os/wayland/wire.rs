//! Wayland's wire format: each message a header (the object's ID; its size in bytes and its opcode, in one word) and
//! its arguments, in the host's byte order, each padded to 32 bits. Files go beside the bytes (`SCM_RIGHTS`), in the
//! order the messages' `fd` arguments take them. A signature says what a message carries, a letter an argument, as
//! libwayland's interface tables have it: `i` int, `u` uint, `f` fixed (24.8), `s` string, `o` object, `n` new ID,
//! `a` array, `h` file; `?` before `s` or `o` lets it be null.

use std::collections::VecDeque;

/// One argument, with `F` a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg<F> {
    Int(i32),
    Uint(u32),
    Fixed(i32),
    Str(Option<String>),
    /// An object's ID, 0 for null.
    Object(u32),
    NewId(u32),
    Array(Vec<u8>),
    Fd(F),
}

impl<F> Arg<F> {
    pub fn int(&self) -> i32 {
        match self {
            Arg::Int(v) | Arg::Fixed(v) => *v,
            Arg::Uint(v) | Arg::Object(v) | Arg::NewId(v) => *v as i32,
            _ => 0,
        }
    }

    pub fn uint(&self) -> u32 {
        self.int() as u32
    }

    pub fn str(&self) -> &str {
        match self {
            Arg::Str(Some(s)) => s,
            _ => "",
        }
    }
}

/// The most a message may be: its size is 16 bits.
pub const MOST: usize = 0xffff;

/// A message's header, if `buf` starts with a whole message: its object, opcode and size.
pub fn header(buf: &[u8]) -> Option<(u32, u16, usize)> {
    if buf.len() < 8 {
        return None;
    }
    let object = u32::from_ne_bytes(buf[0..4].try_into().ok()?);
    let word = u32::from_ne_bytes(buf[4..8].try_into().ok()?);
    let size = (word >> 16) as usize;
    Some((object, (word & 0xffff) as u16, size))
}

/// A message's arguments, from its body (what follows the header) as `signature` says, taking its files from `fds`.
/// None if the body doesn't hold them, or a file's missing.
pub fn decode<F>(signature: &str, body: &[u8], fds: &mut VecDeque<F>) -> Option<Vec<Arg<F>>> {
    let mut args = Vec::new();
    let mut at = 0;
    let word = |at: &mut usize| -> Option<u32> {
        let w = u32::from_ne_bytes(body.get(*at..*at + 4)?.try_into().ok()?);
        *at += 4;
        Some(w)
    };
    let mut nullable = false;
    for c in signature.chars() {
        let arg = match c {
            '?' => {
                nullable = true;
                continue;
            }
            'i' => Arg::Int(word(&mut at)? as i32),
            'u' => Arg::Uint(word(&mut at)?),
            'f' => Arg::Fixed(word(&mut at)? as i32),
            'o' => {
                let id = word(&mut at)?;
                if id == 0 && !nullable {
                    return None;
                }
                Arg::Object(id)
            }
            'n' => Arg::NewId(word(&mut at)?),
            's' | 'a' => {
                let len = word(&mut at)? as usize;
                let bytes = body.get(at..at.checked_add(len)?)?;
                at += len.next_multiple_of(4);
                if c == 'a' {
                    Arg::Array(bytes.to_vec())
                } else if len == 0 {
                    if !nullable {
                        return None;
                    }
                    Arg::Str(None)
                } else {
                    // Its length counts its NUL.
                    let text = bytes.strip_suffix(&[0])?;
                    Arg::Str(Some(String::from_utf8_lossy(text).into_owned()))
                }
            }
            'h' => Arg::Fd(fds.pop_front()?),
            _ => return None,
        };
        nullable = false;
        args.push(arg);
    }
    Some(args)
}

/// A message to `object`, with `opcode` and `args`: its bytes, appended to `out`, and its files, to `files`.
pub fn encode<F>(object: u32, opcode: u16, args: Vec<Arg<F>>, out: &mut Vec<u8>, files: &mut Vec<F>) {
    let start = out.len();
    out.extend(object.to_ne_bytes());
    out.extend(0u32.to_ne_bytes());
    for arg in args {
        match arg {
            Arg::Int(v) | Arg::Fixed(v) => out.extend(v.to_ne_bytes()),
            Arg::Uint(v) | Arg::Object(v) | Arg::NewId(v) => out.extend(v.to_ne_bytes()),
            Arg::Str(None) => out.extend(0u32.to_ne_bytes()),
            Arg::Str(Some(s)) => {
                out.extend(((s.len() + 1) as u32).to_ne_bytes());
                out.extend(s.as_bytes());
                out.push(0);
                out.resize(out.len().next_multiple_of(4), 0);
            }
            Arg::Array(bytes) => {
                out.extend((bytes.len() as u32).to_ne_bytes());
                out.extend(&bytes);
                out.resize(out.len().next_multiple_of(4), 0);
            }
            Arg::Fd(f) => files.push(f),
        }
    }
    let size = (out.len() - start) as u32;
    out[start + 4..start + 8].copy_from_slice(&((size << 16) | u32::from(opcode)).to_ne_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_go_both_ways() {
        let mut out = Vec::new();
        let mut files = Vec::new();
        let args = vec![Arg::Uint(7), Arg::Str(Some("wl_compositor".into())), Arg::Uint(6), Arg::NewId(3), Arg::Fd("a file")];
        encode(2, 0, args.clone(), &mut out, &mut files);
        // A string's length counts its NUL, and it's padded: 14 bytes in 16.
        assert_eq!(out.len(), 8 + 4 + 4 + 16 + 4 + 4);
        let (object, opcode, size) = header(&out).unwrap();
        assert_eq!((object, opcode, size), (2, 0, out.len()));
        let mut fds: VecDeque<&str> = files.into();
        assert_eq!(decode("usunh", &out[8..], &mut fds), Some(args));
    }

    #[test]
    fn nulls_and_shortfalls() {
        let mut out = Vec::new();
        let mut files: Vec<()> = Vec::new();
        encode(1, 1, vec![Arg::Object(0), Arg::Str(None), Arg::Array(vec![1, 2, 3, 4, 5])], &mut out, &mut files);
        let mut none = VecDeque::<()>::new();
        assert_eq!(decode("?o?sa", &out[8..], &mut none), Some(vec![Arg::Object(0), Arg::Str(None), Arg::Array(vec![1, 2, 3, 4, 5])]));
        // A null where none may be, a missing file, a body too short.
        assert_eq!(decode("o", &out[8..], &mut none), None);
        assert_eq!(decode("h", &[], &mut none), None);
        assert_eq!(decode("ii", &[0; 4], &mut none), None);
        assert_eq!(header(&[0; 7]), None);
    }
}
