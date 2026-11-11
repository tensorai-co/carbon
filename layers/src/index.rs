//! The index: a whole tree's names and metadata, in tables of fixed-width records. An image's layers,
//! merged, are one index; a frozen machine is another. Files of `INLINE_MAX` bytes or less are kept in it; bigger ones
//! are lists of chunks, by name, in the store.
//!
//! ```text
//! header   64 bytes: magic, the five table sizes, then 32 bytes kept for a parent
//! inodes   64 bytes each, numbered from 1 (the root): meta[0..3], links, size, mtime s, mtime ns, meta[3], first,
//!          count, two words kept for extended attributes, flags
//! entries  16 bytes each: a name (offset and length in the strings), an inode, a word kept
//! chunks   32 bytes each: a chunk's name, or zeroes for a hole
//! strings  names and link targets
//! inline   small files' contents
//! ```
//!
//! An index could come from anywhere, so nothing in one is trusted: every offset is checked as it's used, and a bad one
//! is `Error::Io`, never a panic.

use alloc::string::String;
use alloc::vec::Vec;

use crate::store::Hash;
use crate::{Error, Kind, Result};

pub const MAGIC: &[u8; 8] = b"MVKIDX\0\x02";
/// Files this small live in the index itself.
pub const INLINE_MAX: usize = 2048;
const HEADER: usize = 64;
const INODE: usize = 64;
const ENTRY: usize = 16;
const CHUNK: usize = 32;
const NO_HASH: Hash = [0; 32];

const FLAG_INLINE: u32 = 1;
const KIND_SHIFT: u32 = 1;
const KIND_MASK: u32 = 0b11 << KIND_SHIFT;
const FLAG_FOLDS_CASE: u32 = 1 << 3;

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// One inode's fields.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inode {
    pub kind: Kind,
    pub meta: [u32; 4],
    pub links: u32,
    pub size: u64,
    pub modified: u64,
    pub folds_case: bool,
    first: u64,
    count: u32,
    inline: bool,
}

/// An index's bytes, its tables checked to fit.
pub(crate) struct Index<'a> {
    data: &'a [u8],
    inodes: u32,
    entries: u32,
    chunks: u32,
    strings: u32,
    inline: u32,
}

impl<'a> Index<'a> {
    pub fn open(data: &'a [u8]) -> Result<Index<'a>> {
        if data.get(..8) != Some(MAGIC.as_slice()) {
            return Err(Error::Io);
        }
        let field = |at| u32_at(data, at).ok_or(Error::Io);
        let (inodes, entries, chunks, strings, inline) = (field(8)?, field(12)?, field(16)?, field(20)?, field(24)?);
        let needed = HEADER as u64
            + u64::from(inodes) * INODE as u64
            + u64::from(entries) * ENTRY as u64
            + u64::from(chunks) * CHUNK as u64
            + u64::from(strings)
            + u64::from(inline);
        if needed != data.len() as u64 || inodes == 0 {
            return Err(Error::Io);
        }
        Ok(Index { data, inodes, entries, chunks, strings, inline })
    }

    pub fn inode_count(&self) -> u64 {
        u64::from(self.inodes)
    }

    fn entries_at(&self) -> usize {
        HEADER + self.inodes as usize * INODE
    }

    fn chunks_at(&self) -> usize {
        self.entries_at() + self.entries as usize * ENTRY
    }

    fn strings_at(&self) -> usize {
        self.chunks_at() + self.chunks as usize * CHUNK
    }

    fn inline_at(&self) -> usize {
        self.strings_at() + self.strings as usize
    }

    /// An inode by its number, from 1.
    pub fn inode(&self, ino: u64) -> Result<Inode> {
        if ino == 0 || ino > u64::from(self.inodes) {
            return Err(Error::Io);
        }
        let at = HEADER + (ino as usize - 1) * INODE;
        let d = self.data;
        let get = |off| u32_at(d, at + off).ok_or(Error::Io);
        let flags = get(60)?;
        let kind = match (flags & KIND_MASK) >> KIND_SHIFT {
            0 => Kind::File,
            1 => Kind::Dir,
            2 => Kind::Symlink,
            _ => return Err(Error::Io),
        };
        let seconds = u64_at(d, at + 24).ok_or(Error::Io)?;
        let nanos = u64::from(get(32)?);
        Ok(Inode {
            kind,
            meta: [get(0)?, get(4)?, get(8)?, get(36)?],
            links: get(12)?,
            size: u64_at(d, at + 16).ok_or(Error::Io)?,
            modified: seconds.saturating_mul(1_000_000_000).saturating_add(nanos),
            folds_case: kind == Kind::Dir && flags & FLAG_FOLDS_CASE != 0,
            first: u64_at(d, at + 40).ok_or(Error::Io)?,
            count: get(48)?,
            inline: flags & FLAG_INLINE != 0,
        })
    }

    fn string(&self, offset: u64, len: u64) -> Result<&'a [u8]> {
        if offset.checked_add(len).is_none_or(|end| end > u64::from(self.strings)) {
            return Err(Error::Io);
        }
        let at = self.strings_at() + offset as usize;
        self.data.get(at..at + len as usize).ok_or(Error::Io)
    }

    pub fn link_target(&self, inode: &Inode) -> Result<String> {
        String::from_utf8(self.string(inode.first, u64::from(inode.count))?.to_vec()).map_err(|_| Error::Io)
    }

    /// A small file's contents.
    pub fn inline_data(&self, inode: &Inode) -> Result<Option<&'a [u8]>> {
        if !inode.inline {
            return Ok(None);
        }
        let len = u64::from(inode.count);
        if inode.first.checked_add(len).is_none_or(|end| end > u64::from(self.inline)) {
            return Err(Error::Io);
        }
        let at = self.inline_at() + inode.first as usize;
        self.data.get(at..at + len as usize).map(Some).ok_or(Error::Io)
    }

    /// A big file's chunks, in order: each a name, or None for a hole.
    pub fn chunks(&self, inode: &Inode) -> Result<Vec<Option<Hash>>> {
        if inode.inline || inode.kind != Kind::File {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(inode.count as usize);
        for i in 0..u64::from(inode.count) {
            let number = inode.first.checked_add(i).ok_or(Error::Io)?;
            if number >= u64::from(self.chunks) {
                return Err(Error::Io);
            }
            let at = self.chunks_at() + number as usize * CHUNK;
            let hash: Hash = self.data.get(at..at + CHUNK).ok_or(Error::Io)?.try_into().map_err(|_| Error::Io)?;
            out.push((hash != NO_HASH).then_some(hash));
        }
        Ok(out)
    }

    /// A directory's entries: each name and inode.
    pub fn entries(&self, inode: &Inode) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::with_capacity(inode.count as usize);
        for i in 0..u64::from(inode.count) {
            let number = inode.first.checked_add(i).ok_or(Error::Io)?;
            if number >= u64::from(self.entries) {
                return Err(Error::Io);
            }
            let at = self.entries_at() + number as usize * ENTRY;
            let get = |off| u32_at(self.data, at + off).ok_or(Error::Io);
            let name = self.string(u64::from(get(0)?), u64::from(get(4)?))?;
            let name = String::from_utf8(name.to_vec()).map_err(|_| Error::Io)?;
            out.push((name, u64::from(get(8)?)));
        }
        Ok(out)
    }
}

/// What a node is, for writing an index.
pub(crate) enum Content {
    /// Entries, in the order they go in, by inode number in this index.
    Dir(Vec<(String, u32)>),
    Inline(Vec<u8>),
    Chunks(Vec<Option<Hash>>),
    Symlink(String),
}

/// A node, for writing an index.
pub(crate) struct Out {
    pub kind: Kind,
    pub meta: [u32; 4],
    pub links: u32,
    pub size: u64,
    pub modified: u64,
    pub folds_case: bool,
    pub content: Content,
}

/// A table of strings, each kept once.
#[derive(Default)]
struct Strings {
    bytes: Vec<u8>,
    seen: alloc::collections::BTreeMap<Vec<u8>, u32>,
}

impl Strings {
    fn add(&mut self, text: &[u8]) -> u32 {
        if let Some(&offset) = self.seen.get(text) {
            return offset;
        }
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(text);
        self.seen.insert(text.to_vec(), offset);
        offset
    }
}

/// Writes nodes as an index: the first is inode 1, the root, and so on.
pub(crate) fn build(nodes: &[Out]) -> Vec<u8> {
    let mut strings = Strings::default();
    let (mut entries, mut chunks, mut inline) = (Vec::new(), Vec::new(), Vec::new());
    let mut inodes = Vec::with_capacity(nodes.len() * INODE);
    for node in nodes {
        let (first, count, flags) = match &node.content {
            Content::Dir(children) => {
                let first = (entries.len() / ENTRY) as u64;
                for (name, ino) in children {
                    entries.extend_from_slice(&strings.add(name.as_bytes()).to_le_bytes());
                    entries.extend_from_slice(&(name.len() as u32).to_le_bytes());
                    entries.extend_from_slice(&ino.to_le_bytes());
                    entries.extend_from_slice(&[0; 4]);
                }
                (first, children.len() as u32, if node.folds_case { FLAG_FOLDS_CASE } else { 0 })
            }
            Content::Inline(data) => {
                let first = inline.len() as u64;
                inline.extend_from_slice(data);
                (first, data.len() as u32, FLAG_INLINE)
            }
            Content::Chunks(list) => {
                let first = (chunks.len() / CHUNK) as u64;
                for hash in list {
                    chunks.extend_from_slice(hash.as_ref().unwrap_or(&NO_HASH));
                }
                (first, list.len() as u32, 0)
            }
            Content::Symlink(target) => (u64::from(strings.add(target.as_bytes())), target.len() as u32, 0),
        };
        let kind = match node.kind {
            Kind::File => 0,
            Kind::Dir => 1,
            Kind::Symlink => 2,
        };
        inodes.extend_from_slice(&node.meta[0].to_le_bytes());
        inodes.extend_from_slice(&node.meta[1].to_le_bytes());
        inodes.extend_from_slice(&node.meta[2].to_le_bytes());
        inodes.extend_from_slice(&node.links.to_le_bytes());
        inodes.extend_from_slice(&node.size.to_le_bytes());
        inodes.extend_from_slice(&(node.modified / 1_000_000_000).to_le_bytes());
        inodes.extend_from_slice(&((node.modified % 1_000_000_000) as u32).to_le_bytes());
        inodes.extend_from_slice(&node.meta[3].to_le_bytes());
        inodes.extend_from_slice(&first.to_le_bytes());
        inodes.extend_from_slice(&count.to_le_bytes());
        inodes.extend_from_slice(&[0; 8]);
        inodes.extend_from_slice(&(flags | kind << KIND_SHIFT).to_le_bytes());
    }
    let mut out = Vec::with_capacity(HEADER + inodes.len() + entries.len() + chunks.len() + strings.bytes.len() + inline.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    out.extend_from_slice(&((entries.len() / ENTRY) as u32).to_le_bytes());
    out.extend_from_slice(&((chunks.len() / CHUNK) as u32).to_le_bytes());
    out.extend_from_slice(&(strings.bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(inline.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 36]);
    out.extend_from_slice(&inodes);
    out.extend_from_slice(&entries);
    out.extend_from_slice(&chunks);
    out.extend_from_slice(&strings.bytes);
    out.extend_from_slice(&inline);
    out
}
