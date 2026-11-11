//! Building what goes on a program's stack, from the top down: a new program's arguments and auxiliary vector, a
//! signal frame, an exception record, an upcall's arguments. The layouts are each kernel's; the arithmetic is here.

/// Bytes to go just below `top`, built downwards, then written with one copy.
#[derive(Debug, Clone)]
pub struct StackWriter {
    top: u64,
    /// What's below `top`, lowest address first.
    bytes: Vec<u8>,
}

impl StackWriter {
    pub fn new(top: u64) -> StackWriter {
        StackWriter { top, bytes: Vec::new() }
    }

    /// The lowest address written so far: the stack pointer, if it ended here.
    pub fn sp(&self) -> u64 {
        self.top - self.bytes.len() as u64
    }

    /// Puts `data` below what's there, giving its address.
    pub fn push(&mut self, data: &[u8]) -> u64 {
        self.bytes.splice(0..0, data.iter().copied());
        self.sp()
    }

    pub fn push_u64(&mut self, value: u64) -> u64 {
        self.push(&value.to_le_bytes())
    }

    pub fn push_u32(&mut self, value: u32) -> u64 {
        self.push(&value.to_le_bytes())
    }

    /// A NUL-terminated string.
    pub fn push_cstr(&mut self, s: &[u8]) -> u64 {
        self.push(&[0]);
        self.push(s)
    }

    /// Pads down to a multiple of `align`.
    pub fn align(&mut self, align: u64) {
        let pad = self.sp() % align;
        self.push(&vec![0; pad as usize]);
    }

    /// Pads so that after `then` more bytes are pushed, the stack pointer is a multiple of `align`.
    pub fn align_for(&mut self, then: u64, align: u64) {
        let pad = (self.sp() - then) % align;
        self.push(&vec![0; pad as usize]);
    }

    /// The final stack pointer, and the bytes to write there.
    pub fn finish(self) -> (u64, Vec<u8>) {
        (self.sp(), self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_downwards() {
        let mut stack = StackWriter::new(0x1000);
        let s = stack.push_cstr(b"hi");
        assert_eq!(s, 0x1000 - 3);
        stack.align(16);
        assert_eq!(stack.sp(), 0xff0);
        stack.align_for(24, 16);
        stack.push_u64(1);
        stack.push_u64(2);
        stack.push_u64(3);
        assert_eq!(stack.sp() % 16, 0);
        let (sp, bytes) = stack.finish();
        assert_eq!(&bytes[..8], &3u64.to_le_bytes());
        assert_eq!(sp + bytes.len() as u64, 0x1000);
    }
}
