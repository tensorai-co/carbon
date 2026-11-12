//! X11's wire format, little-endian only (every client on ARM64 and x86-64 speaks it; a big-endian one is refused at
//! setup): a request's fields read where the protocol puts them, replies written field by field, events as 32 bytes,
//! errors, and the 68 atoms every server has from the start.

/// A request, header and all: its fields read at their byte offsets, 0 past its end (a short request reads zeroes,
/// as a careless client's would be refused by a stricter server).
#[derive(Clone, Copy)]
pub struct Req<'a> {
    pub data: &'a [u8],
}

impl<'a> Req<'a> {
    pub fn major(&self) -> u8 {
        self.u8(0)
    }

    /// The header's second byte: a minor opcode for an extension, a small argument for the core.
    pub fn b1(&self) -> u8 {
        self.u8(1)
    }

    pub fn u8(&self, at: usize) -> u8 {
        self.data.get(at).copied().unwrap_or(0)
    }

    pub fn u16(&self, at: usize) -> u16 {
        self.data.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&self, at: usize) -> i16 {
        self.u16(at) as i16
    }

    pub fn u32(&self, at: usize) -> u32 {
        self.data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Bytes from `at`, as many as there are up to `n`.
    pub fn bytes(&self, at: usize, n: usize) -> &'a [u8] {
        let start = at.min(self.data.len());
        &self.data[start..(at.saturating_add(n)).min(self.data.len())]
    }

    /// Everything from `at` to its end.
    pub fn rest(&self, at: usize) -> &'a [u8] {
        &self.data[at.min(self.data.len())..]
    }

    pub fn str(&self, at: usize, n: usize) -> String {
        String::from_utf8_lossy(self.bytes(at, n)).into_owned()
    }

    /// A value list: one word for each bit of `mask`, lowest bit first, from `at`. Each bit's value, or None.
    pub fn values(&self, mask: u32, at: usize) -> [Option<u32>; 32] {
        let mut out = [None; 32];
        let mut i = 0;
        for (bit, v) in out.iter_mut().enumerate() {
            if mask & (1 << bit) != 0 {
                *v = Some(self.u32(at + 4 * i));
                i += 1;
            }
        }
        out
    }

    /// Its rectangles from `at`: x and y (16 bits signed), width and height (16 bits).
    pub fn rects(&self, at: usize) -> Vec<(i32, i32, i32, i32)> {
        self.rest(at).as_chunks::<8>().0.iter().map(|c| (le16(c, 0) as i16 as i32, le16(c, 2) as i16 as i32, le16(c, 4) as i32, le16(c, 6) as i32)).collect()
    }

    /// Its points from `at`, each an x and y (16 bits signed).
    pub fn points(&self, at: usize) -> Vec<(i32, i32)> {
        self.rest(at).as_chunks::<4>().0.iter().map(|c| (le16(c, 0) as i16 as i32, le16(c, 2) as i16 as i32)).collect()
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// Bytes to pad `n` to a multiple of four.
pub fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

/// A reply being written: its first 8 bytes are the header (filled in when it's sent), its fields follow.
pub struct Reply {
    pub data: Vec<u8>,
}

impl Reply {
    /// A reply whose header's second byte is `b1`.
    pub fn new(b1: u8) -> Reply {
        let mut data = vec![0u8; 8];
        data[0] = 1;
        data[1] = b1;
        Reply { data }
    }

    pub fn u8(&mut self, v: u8) -> &mut Reply {
        self.data.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Reply {
        self.data.extend(v.to_le_bytes());
        self
    }

    pub fn i16(&mut self, v: i16) -> &mut Reply {
        self.u16(v as u16)
    }

    pub fn u32(&mut self, v: u32) -> &mut Reply {
        self.data.extend(v.to_le_bytes());
        self
    }

    pub fn i32(&mut self, v: i32) -> &mut Reply {
        self.u32(v as u32)
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Reply {
        self.data.extend_from_slice(b);
        self
    }

    /// Zeroes, `n` of them.
    pub fn skip(&mut self, n: usize) -> &mut Reply {
        self.data.resize(self.data.len() + n, 0);
        self
    }

    /// Its bytes to send, with `seq` and its length in its header: at least 32 bytes, a whole number of words.
    pub fn finish(mut self, seq: u16) -> Vec<u8> {
        if self.data.len() < 32 {
            self.data.resize(32, 0);
        }
        let n = pad(self.data.len());
        self.data.resize(self.data.len() + n, 0);
        self.data[2..4].copy_from_slice(&seq.to_le_bytes());
        let len = ((self.data.len() - 32) / 4) as u32;
        self.data[4..8].copy_from_slice(&len.to_le_bytes());
        self.data
    }
}

/// An event's 32 bytes: its code first, its fields written at their offsets (its sequence number is the client's,
/// filled in when it's sent).
#[derive(Clone, Copy)]
pub struct Ev(pub [u8; 32]);

impl Ev {
    pub fn new(code: u8) -> Ev {
        let mut e = [0u8; 32];
        e[0] = code;
        Ev(e)
    }

    pub fn u8(mut self, at: usize, v: u8) -> Ev {
        self.0[at] = v;
        self
    }

    pub fn u16(mut self, at: usize, v: u16) -> Ev {
        self.0[at..at + 2].copy_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i16(self, at: usize, v: i32) -> Ev {
        self.u16(at, v as i16 as u16)
    }

    pub fn u32(mut self, at: usize, v: u32) -> Ev {
        self.0[at..at + 4].copy_from_slice(&v.to_le_bytes());
        self
    }

    pub fn code(&self) -> u8 {
        self.0[0] & 0x7f
    }
}

/// An error's 32 bytes.
pub fn error(code: u8, seq: u16, value: u32, major: u8, minor: u16) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[1] = code;
    e[2..4].copy_from_slice(&seq.to_le_bytes());
    e[4..8].copy_from_slice(&value.to_le_bytes());
    e[8..10].copy_from_slice(&minor.to_le_bytes());
    e[10] = major;
    e
}

/// The core protocol's errors.
pub mod err {
    pub const REQUEST: u8 = 1;
    pub const VALUE: u8 = 2;
    pub const WINDOW: u8 = 3;
    pub const PIXMAP: u8 = 4;
    pub const ATOM: u8 = 5;
    pub const FONT: u8 = 7;
    pub const MATCH: u8 = 8;
    pub const DRAWABLE: u8 = 9;
    pub const ACCESS: u8 = 10;
    pub const GCONTEXT: u8 = 13;
    pub const IDCHOICE: u8 = 14;
    pub const NAME: u8 = 15;
    pub const LENGTH: u8 = 16;
}

/// The core protocol's events.
pub mod ev {
    pub const KEY_PRESS: u8 = 2;
    pub const KEY_RELEASE: u8 = 3;
    pub const BUTTON_PRESS: u8 = 4;
    pub const BUTTON_RELEASE: u8 = 5;
    pub const MOTION_NOTIFY: u8 = 6;
    pub const ENTER_NOTIFY: u8 = 7;
    pub const LEAVE_NOTIFY: u8 = 8;
    pub const FOCUS_IN: u8 = 9;
    pub const FOCUS_OUT: u8 = 10;
    pub const EXPOSE: u8 = 12;
    pub const NO_EXPOSURE: u8 = 14;
    pub const CREATE_NOTIFY: u8 = 16;
    pub const DESTROY_NOTIFY: u8 = 17;
    pub const UNMAP_NOTIFY: u8 = 18;
    pub const MAP_NOTIFY: u8 = 19;
    pub const REPARENT_NOTIFY: u8 = 21;
    pub const CONFIGURE_NOTIFY: u8 = 22;
    pub const PROPERTY_NOTIFY: u8 = 28;
    pub const SELECTION_CLEAR: u8 = 29;
    pub const SELECTION_REQUEST: u8 = 30;
    pub const SELECTION_NOTIFY: u8 = 31;
    pub const CLIENT_MESSAGE: u8 = 33;
}

/// Event masks.
pub mod mask {
    pub const KEY_PRESS: u32 = 1;
    pub const KEY_RELEASE: u32 = 2;
    pub const BUTTON_PRESS: u32 = 4;
    pub const BUTTON_RELEASE: u32 = 8;
    pub const ENTER_WINDOW: u32 = 0x10;
    pub const LEAVE_WINDOW: u32 = 0x20;
    pub const POINTER_MOTION: u32 = 0x40;
    pub const BUTTON_MOTION: u32 = 0x2000;
    pub const EXPOSURE: u32 = 0x8000;
    pub const STRUCTURE_NOTIFY: u32 = 0x20000;
    pub const SUBSTRUCTURE_NOTIFY: u32 = 0x80000;
    pub const SUBSTRUCTURE_REDIRECT: u32 = 0x100000;
    pub const FOCUS_CHANGE: u32 = 0x200000;
    pub const PROPERTY_CHANGE: u32 = 0x400000;
    pub const OWNER_GRAB_BUTTON: u32 = 0x1000000;
}

/// The atoms every server has, from 1.
pub const PREDEFINED: [&str; 68] = [
    "PRIMARY", "SECONDARY", "ARC", "ATOM", "BITMAP", "CARDINAL", "COLORMAP", "CURSOR", "CUT_BUFFER0", "CUT_BUFFER1", "CUT_BUFFER2",
    "CUT_BUFFER3", "CUT_BUFFER4", "CUT_BUFFER5", "CUT_BUFFER6", "CUT_BUFFER7", "DRAWABLE", "FONT", "INTEGER", "PIXMAP", "POINT",
    "RECTANGLE", "RESOURCE_MANAGER", "RGB_COLOR_MAP", "RGB_BEST_MAP", "RGB_BLUE_MAP", "RGB_DEFAULT_MAP", "RGB_GRAY_MAP", "RGB_GREEN_MAP",
    "RGB_RED_MAP", "STRING", "VISUALID", "WINDOW", "WM_COMMAND", "WM_HINTS", "WM_CLIENT_MACHINE", "WM_ICON_NAME", "WM_ICON_SIZE",
    "WM_NAME", "WM_NORMAL_HINTS", "WM_SIZE_HINTS", "WM_ZOOM_HINTS", "MIN_SPACE", "NORM_SPACE", "MAX_SPACE", "END_SPACE",
    "SUPERSCRIPT_X", "SUPERSCRIPT_Y", "SUBSCRIPT_X", "SUBSCRIPT_Y", "UNDERLINE_POSITION", "UNDERLINE_THICKNESS", "STRIKEOUT_ASCENT",
    "STRIKEOUT_DESCENT", "ITALIC_ANGLE", "X_HEIGHT", "QUAD_WIDTH", "WEIGHT", "POINT_SIZE", "RESOLUTION", "COPYRIGHT", "NOTICE",
    "FONT_NAME", "FAMILY_NAME", "FULL_NAME", "CAP_HEIGHT", "WM_CLASS", "WM_TRANSIENT_FOR",
];

/// Predefined atoms the server itself uses.
pub mod atom {
    pub const ATOM: u32 = 4;
    pub const CARDINAL: u32 = 6;
    pub const RESOURCE_MANAGER: u32 = 23;
    pub const STRING: u32 = 31;
    pub const WINDOW: u32 = 33;
    pub const WM_NAME: u32 = 39;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_whole_words_and_say_their_length() {
        let mut r = Reply::new(7);
        r.u32(1).u16(2).bytes(&[0; 30]);
        let bytes = r.finish(9);
        assert_eq!(bytes.len(), 44);
        assert_eq!((bytes[0], bytes[1], u16::from_le_bytes([bytes[2], bytes[3]])), (1, 7, 9));
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 3);
    }

    #[test]
    fn value_lists_follow_their_mask() {
        let mut data = vec![0u8; 8];
        for v in [10u32, 20, 30] {
            data.extend(v.to_le_bytes());
        }
        let v = Req { data: &data }.values(0b1010_0001, 8);
        assert_eq!((v[0], v[1], v[5], v[7]), (Some(10), None, Some(20), Some(30)));
    }
}
