//! Reading ELF files: just enough to load a program and its interpreter.

use exec::{Isa, Prot};

use crate::abi::{EM_AARCH64, EM_X86_64, ENOEXEC, Errno};

const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
pub const ET_DYN: u16 = 3;

#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub vaddr: u64,
    pub memsz: u64,
    pub filesz: u64,
    pub offset: u64,
    pub prot: Prot,
}

#[derive(Debug)]
pub struct Elf<'a> {
    pub bytes: &'a [u8],
    pub isa: Isa,
    pub kind: u16,
    pub entry: u64,
    pub phoff: u64,
    pub phnum: u64,
    pub segments: Vec<Segment>,
    pub interp: Option<String>,
    /// Where the program headers are, if a segment says.
    pub phdr: Option<u64>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, Errno> {
    Ok(u16::from_le_bytes(b.get(at..at + 2).ok_or(ENOEXEC)?.try_into().map_err(|_| ENOEXEC)?))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, Errno> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).ok_or(ENOEXEC)?.try_into().map_err(|_| ENOEXEC)?))
}

fn u64_at(b: &[u8], at: usize) -> Result<u64, Errno> {
    Ok(u64::from_le_bytes(b.get(at..at + 8).ok_or(ENOEXEC)?.try_into().map_err(|_| ENOEXEC)?))
}

pub fn parse(bytes: &[u8]) -> Result<Elf<'_>, Errno> {
    // 64-bit, little-endian ELF only.
    if bytes.get(..6) != Some(b"\x7fELF\x02\x01") {
        return Err(ENOEXEC);
    }
    let kind = u16_at(bytes, 16)?;
    let isa = match u16_at(bytes, 18)? {
        EM_AARCH64 => Isa::Arm64,
        EM_X86_64 => Isa::X86_64,
        _ => return Err(ENOEXEC),
    };
    let entry = u64_at(bytes, 24)?;
    let phoff = u64_at(bytes, 32)?;
    let phentsize = u64::from(u16_at(bytes, 54)?);
    let phnum = u64::from(u16_at(bytes, 56)?);
    if phentsize != 56 {
        return Err(ENOEXEC);
    }
    let mut segments = Vec::new();
    let (mut interp, mut phdr) = (None, None);
    for i in 0..phnum {
        let at = (phoff + i * phentsize) as usize;
        let (kind, flags) = (u32_at(bytes, at)?, u32_at(bytes, at + 4)?);
        let offset = u64_at(bytes, at + 8)?;
        let vaddr = u64_at(bytes, at + 16)?;
        let filesz = u64_at(bytes, at + 32)?;
        let memsz = u64_at(bytes, at + 40)?;
        match kind {
            PT_LOAD => {
                let mut prot = Prot::NONE;
                for (bit, p) in [(4, Prot::R), (2, Prot::W), (1, Prot::X)] {
                    if flags & bit != 0 {
                        prot = prot | p;
                    }
                }
                if filesz > memsz || offset.checked_add(filesz).is_none_or(|end| end > bytes.len() as u64) {
                    return Err(ENOEXEC);
                }
                segments.push(Segment { vaddr, memsz, filesz, offset, prot });
            }
            PT_INTERP => {
                let raw = bytes.get(offset as usize..(offset + filesz) as usize).ok_or(ENOEXEC)?;
                interp = Some(String::from_utf8_lossy(raw.split(|&b| b == 0).next().unwrap_or(raw)).into_owned());
            }
            PT_PHDR => phdr = Some(vaddr),
            _ => {}
        }
    }
    if segments.is_empty() {
        return Err(ENOEXEC);
    }
    // Without PT_PHDR, the headers are wherever the segment holding them puts them.
    if phdr.is_none() {
        phdr = segments.iter().find(|s| s.offset <= phoff && phoff < s.offset + s.filesz).map(|s| s.vaddr + phoff - s.offset);
    }
    Ok(Elf { bytes, isa, kind, entry, phoff, phnum, segments, interp, phdr })
}

impl Elf<'_> {
    /// The lowest and highest addresses its segments cover, before any base is added.
    pub fn span(&self) -> (u64, u64) {
        let lo = self.segments.iter().map(|s| s.vaddr).min().unwrap_or(0);
        let hi = self.segments.iter().map(|s| s.vaddr + s.memsz).max().unwrap_or(0);
        (lo, hi)
    }
}
