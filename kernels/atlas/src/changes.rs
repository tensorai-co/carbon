//! Changes to folders, for `ReadDirectoryChangesW`: a machine-wide list of watches, each on a folder (and what's under
//! it, if asked) for the kinds of change its filter names. atlas's own file operations report what they change (a
//! name made, gone or renamed; a file written, cut, its attributes or times set), and each watch keeps the records
//! for its next read: one that's waiting is finished at once, others take them when they ask. A watch starts with its
//! folder handle's first read, as on Windows, and ends when the handle closes.

use std::sync::Arc;

use kernels_core::lock;
use kernels_core::wait::Waiter;

use crate::Kernel;
use crate::ports::{Completion, Op};

/// FILE_ACTION_*.
pub const ADDED: u32 = 1;
pub const REMOVED: u32 = 2;
pub const MODIFIED: u32 = 3;
pub const RENAMED_OLD: u32 = 4;
pub const RENAMED_NEW: u32 = 5;

/// FILE_NOTIFY_CHANGE_*: what kind of change each is.
pub const FILE_NAME: u32 = 0x1;
pub const DIR_NAME: u32 = 0x2;
pub const ATTRIBUTES: u32 = 0x4;
pub const SIZE: u32 = 0x8;
pub const LAST_WRITE: u32 = 0x10;
pub const LAST_ACCESS: u32 = 0x20;
pub const CREATION: u32 = 0x40;

/// STATUS_NOTIFY_ENUM_DIR's Win32 error: more changed than the buffer holds, so the program should look again.
pub const ERROR_NOTIFY_ENUM_DIR: u32 = 1022;

/// A read that's waiting for changes.
pub enum Waiting {
    /// Overlapped: its operation, buffer and length.
    Overlapped(Arc<Op>, u32),
    /// A thread blocked in it.
    Blocking(Arc<Waiter>),
}

pub struct Watch {
    /// The folder handle's file, by address, which the watch is the handle's.
    pub key: usize,
    /// The folder's full Windows path, lower case, without a trailing backslash.
    pub dir: String,
    pub subtree: bool,
    pub filter: u32,
    /// Each change: its action and its name from the folder, as the program spelled it.
    pub records: Vec<(u32, String)>,
    pub waiting: Option<Waiting>,
}

#[derive(Default)]
pub struct Watches {
    pub list: Vec<Watch>,
}

/// FILE_NOTIFY_INFORMATION records, each on four bytes: None if they don't fit in `room`.
pub fn encode(records: &[(u32, String)], room: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut last = None;
    for (action, name) in records {
        let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let start = out.len();
        if let Some(prev) = last {
            let next = (start - prev) as u32;
            out[prev..prev + 4].copy_from_slice(&next.to_le_bytes());
        }
        out.extend(0u32.to_le_bytes());
        out.extend(action.to_le_bytes());
        out.extend((units.len() as u32).to_le_bytes());
        out.extend(units);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        last = Some(start);
    }
    (out.len() <= room).then_some(out)
}

impl Kernel {
    /// A change atlas made at `path` (a full Windows path), of kind `what`, told to every watch it's in.
    pub(crate) fn changed(&self, path: &str, action: u32, what: u32) {
        let mut watches = lock(&self.changes);
        if watches.list.is_empty() {
            return;
        }
        let lower = path.to_lowercase();
        let mut done = Vec::new();
        for w in watches.list.iter_mut() {
            if w.filter & what == 0 {
                continue;
            }
            let Some(rest) = lower.strip_prefix(&w.dir).and_then(|r| r.strip_prefix('\\')) else { continue };
            if rest.is_empty() || (!w.subtree && rest.contains('\\')) {
                continue;
            }
            // The name as the path spells it, from where the folder's part ends.
            let name = path[path.len() - rest.len()..].to_string();
            w.records.push((action, name));
            match w.waiting.take() {
                Some(Waiting::Overlapped(op, room)) => {
                    let records = std::mem::take(&mut w.records);
                    let (data, error) = match encode(&records, room as usize) {
                        Some(d) => (d, 0),
                        None => (Vec::new(), ERROR_NOTIFY_ENUM_DIR),
                    };
                    done.push(Completion { op, data, error });
                }
                Some(Waiting::Blocking(waiter)) => waiter.wake(),
                None => {}
            }
        }
        drop(watches);
        for c in done {
            self.complete_later(c);
        }
    }

    /// A folder handle closed: its watch goes, and a read it had waiting is aborted.
    pub(crate) fn unwatch(&self, key: usize) {
        let gone: Vec<Watch> = {
            let mut watches = lock(&self.changes);
            let (gone, kept) = std::mem::take(&mut watches.list).into_iter().partition(|w| w.key == key);
            watches.list = kept;
            gone
        };
        for w in gone {
            match w.waiting {
                Some(Waiting::Overlapped(op, _)) => self.complete_later(Completion { op, data: Vec::new(), error: 995 }),
                Some(Waiting::Blocking(waiter)) => waiter.wake(),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_on_four_bytes_chained() {
        let b = encode(&[(ADDED, "a.txt".into()), (RENAMED_NEW, "sub\\bc".into())], 1024).unwrap();
        // The first: next at 24 (12 + 10 bytes of name, on four), added, 10 bytes; the second 12 + 12, so 48 in all.
        assert_eq!(&b[0..12], &[24, 0, 0, 0, 1, 0, 0, 0, 10, 0, 0, 0]);
        assert_eq!(&b[24..36], &[0, 0, 0, 0, 5, 0, 0, 0, 12, 0, 0, 0]);
        assert_eq!(b.len(), 48);
        assert!(encode(&[(ADDED, "a.txt".into())], 8).is_none());
    }
}
