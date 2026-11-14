//! The DXBC container: a header, then chunks by four-character code. The program is `SHDR` (shader model 4) or
//! `SHEX` (5); the stage's inputs and outputs are described by the signature chunks.

use alloc::string::String;
use alloc::vec::Vec;

use super::fail;
use crate::Error;

/// A container's chunks, in order.
pub(super) struct Container<'a> {
    pub chunks: Vec<([u8; 4], &'a [u8])>,
}

fn word(b: &[u8], at: usize) -> Result<u32, Error> {
    match b.get(at..at + 4) {
        Some(w) => Ok(u32::from_le_bytes([w[0], w[1], w[2], w[3]])),
        None => fail(alloc::format!("the container ends at byte {}, before {at}", b.len())),
    }
}

impl<'a> Container<'a> {
    /// The header (`DXBC`, a 16-byte checksum, 1, the total size, the chunk count) and each chunk's code and bytes.
    /// The checksum isn't checked: drivers don't need it, and programs that patch their shaders don't keep it.
    pub fn read(b: &'a [u8]) -> Result<Container<'a>, Error> {
        if b.get(..4) != Some(b"DXBC") {
            return fail("not a DXBC container");
        }
        let size = word(b, 24)? as usize;
        if size > b.len() {
            return fail(alloc::format!("the container says it's {size} bytes, and there are {}", b.len()));
        }
        let b = &b[..size];
        let count = word(b, 28)? as usize;
        let mut chunks = Vec::new();
        for i in 0..count {
            let at = word(b, 32 + 4 * i)? as usize;
            let code = b.get(at..at + 4).ok_or_else(|| Error { message: alloc::format!("chunk {i} is past the end") })?;
            let len = word(b, at + 4)? as usize;
            let data = b.get(at + 8..at + 8 + len).ok_or_else(|| Error { message: alloc::format!("chunk {i}'s {len} bytes are past the end") })?;
            chunks.push(([code[0], code[1], code[2], code[3]], data));
        }
        Ok(Container { chunks })
    }

    pub fn chunk(&self, codes: &[&[u8; 4]]) -> Option<(&[u8; 4], &'a [u8])> {
        self.chunks.iter().find(|(c, _)| codes.contains(&c)).map(|(c, d)| (c, *d))
    }
}

/// What a signature element's components are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Uint,
    Sint,
    Float,
}

/// One element of an input or output signature: a semantic, the register it's in and which of its components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The semantic's name as the program wrote it (`TEXCOORD`, `SV_Position`); compared without case.
    pub semantic: String,
    pub index: u32,
    /// `D3D_NAME`: 0 for none, 1 position, 6 vertex id, 8 instance id, 9 is-front-face, 64 target, 65 depth, ...
    pub system: u32,
    pub component: Component,
    pub register: u32,
    /// The components it has, x lowest.
    pub mask: u8,
    /// For an input, the components the program reads; for an output, those it doesn't always write.
    pub used: u8,
}

/// A signature chunk's elements: `ISGN`/`OSGN` (24 bytes each), `OSG5` (a stream first, 28) or `ISG1`/`OSG1` (a
/// stream first and a minimum precision last, 32).
pub(super) fn signature(code: &[u8; 4], data: &[u8]) -> Result<Vec<Element>, Error> {
    let count = word(data, 0)? as usize;
    let (size, skip) = match code {
        b"ISGN" | b"OSGN" | b"PCSG" => (24, 0),
        b"OSG5" => (28, 4),
        _ => (32, 4),
    };
    let mut out = Vec::new();
    for i in 0..count {
        let at = 8 + i * size + skip;
        let name_at = word(data, at)? as usize;
        let name = data.get(name_at..).unwrap_or(&[]);
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let component = match word(data, at + 12)? {
            1 => Component::Uint,
            2 => Component::Sint,
            3 => Component::Float,
            other => return fail(alloc::format!("signature element {i} has components of type {other}")),
        };
        let masks = word(data, at + 20)?;
        out.push(Element {
            semantic: String::from_utf8_lossy(&name[..end]).into_owned(),
            index: word(data, at + 4)?,
            system: word(data, at + 8)?,
            component,
            register: word(data, at + 16)?,
            mask: masks as u8,
            used: (masks >> 8) as u8,
        });
    }
    Ok(out)
}
