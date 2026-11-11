//! Small strings without an allocator: module names, and messages for atlas's log.

/// A short ASCII name, lowercased: how the loader compares module names.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; 64],
    len: usize,
}

impl Name {
    pub const EMPTY: Name = Name { bytes: [0; 64], len: 0 };

    /// A module name from a path or a bare name, lowercased, with `.dll` added if it has no extension.
    pub fn module(raw: &[u8]) -> Name {
        let file = raw.rsplit(|&b| b == b'\\' || b == b'/').next().unwrap_or(raw);
        let mut name = Name::EMPTY;
        for &b in file.iter().take(59) {
            name.bytes[name.len] = b.to_ascii_lowercase();
            name.len += 1;
        }
        if !name.as_bytes().contains(&b'.') {
            for &b in b".dll" {
                name.bytes[name.len] = b;
                name.len += 1;
            }
        }
        name
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// A message being built, for `DEBUG_PRINT`.
pub struct Message {
    bytes: [u8; 256],
    len: usize,
}

impl Message {
    pub fn new() -> Message {
        Message { bytes: [0; 256], len: 0 }
    }

    pub fn text(&mut self, s: &[u8]) -> &mut Message {
        for &b in s {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
        self
    }

    pub fn hex(&mut self, v: u64) -> &mut Message {
        self.text(b"0x");
        let mut started = false;
        for shift in (0..16).rev() {
            let digit = (v >> (shift * 4)) & 0xf;
            if digit != 0 || started || shift == 0 {
                started = true;
                self.text(&[b"0123456789abcdef"[digit as usize]]);
            }
        }
        self
    }

    pub fn print(&self) {
        crate::sys::debug_print(&self.bytes[..self.len]);
    }
}
