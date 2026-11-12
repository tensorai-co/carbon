//! The clipboard, and the primary selection (what's selected, for a middle click to paste), between every window of
//! every kernel and the host. An owner offers a selection as a list of MIME types, with the data for some of them at
//! once (Windows' `SetClipboardData` gives it so) and the rest sent when it's asked for (Wayland's and X11's way). A new
//! offer takes the selection from the owner before, which hears so (`SelectionCancelled`: Windows'
//! `WM_DESTROYCLIPBOARD`, X11's `SelectionClear`, Wayland's `cancelled`). A paste asks for one type: the desktop gives
//! what it has, or asks the owner's kernel (`SendSelection`) and passes on its answer, to a window
//! (`SelectionData`) or to something outside the machine (`take_transfer`).
//!
//! An owner outside the machine (the host's clipboard, `carbon display clipboard`) has no window; its data is always
//! given at once.

use std::collections::BTreeMap;

use crate::desktop::WindowId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Selection {
    Clipboard,
    /// What's selected, for a middle click: X11's PRIMARY, Wayland's primary selection.
    Primary,
    /// What's being dragged, while a drag lasts (`Desktop::start_drag`): read by its target as the others are, and
    /// gone when the drag ends.
    Drag,
}

/// Files, as drag and drop and the clipboard carry them: one `file://` URI a line (RFC 2483), each in the namespace of
/// the kernel that made it.
pub const URI_LIST: &str = "text/uri-list";

/// An offer's identity: a newer one replaces it.
pub type SourceId = u64;
/// A paste's identity, from its request to its answer.
pub type TransferId = u64;

/// Text, as every API's text maps to it.
pub const TEXT: &str = "text/plain;charset=utf-8";
/// The most a selection's data may be, for one type.
pub const MOST: usize = 64 << 20;

/// What a selection is now: who offered it, and as what.
#[derive(Debug, Clone, Default)]
pub(crate) struct Source {
    pub id: SourceId,
    pub owner: Option<WindowId>,
    pub mimes: Vec<String>,
    /// The data given at once, by type.
    pub data: BTreeMap<String, Vec<u8>>,
}

/// A paste on its way.
pub(crate) struct Transfer {
    /// Who asked: a window, or (None) something outside the machine.
    pub requester: Option<WindowId>,
    /// The owner asked, whose answer it's waiting for.
    pub owner: Option<WindowId>,
    /// The answer, once there is one, for a requester outside.
    pub result: Option<Option<Vec<u8>>>,
}

#[derive(Default)]
pub(crate) struct Selections {
    pub sources: BTreeMap<Selection, Source>,
    pub transfers: BTreeMap<TransferId, Transfer>,
    pub next: u64,
}

impl Selections {
    pub fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

/// A `file://` URI for a path in a kernel's namespace: `/home/a b` is `file:///home/a%20b`, and a Windows path
/// `C:\a b` is `file:///C:/a%20b`. Bytes outside RFC 3986's unreserved and path characters are percent-encoded.
pub fn file_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = if path.starts_with('/') { path } else { format!("/{path}") };
    let mut uri = String::from("file://");
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:!$&'()*+,;=@".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The path a `file://` URI names (`file_uri`'s inverse), with `/` between its parts: `/home/a b`, or `C:/a b` for a
/// Windows path. None for another scheme, a host other than this machine, or a path that isn't UTF-8.
pub fn file_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let raw = rest.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && i + 2 < raw.len()
            && let Some(b) = std::str::from_utf8(&raw[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            bytes.push(b);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `/C:/...`: a drive letter's.
    let b = path.as_bytes();
    Some(if b.len() >= 3 && b[1].is_ascii_alphabetic() && b[2] == b':' { path[1..].to_string() } else { path })
}

/// The paths of a `text/uri-list`'s `file://` URIs, in order, skipping comments and other schemes.
pub fn uri_list_paths(list: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(list).lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(file_path).collect()
}

/// A `text/uri-list` of paths, each line ending in CRLF as RFC 2483 has it.
pub fn uri_list(paths: &[&str]) -> Vec<u8> {
    paths.iter().map(|p| file_uri(p) + "\r\n").collect::<String>().into_bytes()
}
