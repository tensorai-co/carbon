//! The Mac's general pasteboard and the machine's clipboard, following each other: text copied on the Mac is offered
//! to the machine (from outside it, given at once), and what a window of the machine offers is put on the pasteboard.
//! Each side knows its own offers (the pasteboard's change count, the desktop's `selection_source`), so neither copies
//! back what came from the other.

use std::sync::mpsc;
use std::time::Duration;

use display::desktop::{self, Shared};
use display::selection::{SourceId, Selection, TEXT};

use crate::objc::{Id, class, msg, ns_string, pasteboard_string_type, rust_string};

/// How long a paste from the machine may take, its kernel answering: on another thread, so the loop never waits.
const PASTE_WAIT: Duration = Duration::from_secs(2);

pub(crate) struct Pasteboard {
    board: Id,
    /// The pasteboard's change count as last seen (or set by us), and the offer we last made the machine.
    change: isize,
    ours: Option<SourceId>,
    /// A paste from the machine on its way, for the pasteboard.
    pending: Option<mpsc::Receiver<Option<Vec<u8>>>>,
}

impl Pasteboard {
    /// Following the general pasteboard: what's on it now is offered to the machine at the first `follow`.
    pub fn new() -> Pasteboard {
        Pasteboard { board: msg!(Id; class("NSPasteboard"), "generalPasteboard"), change: -1, ours: None, pending: None }
    }

    /// Text copied on the Mac since last time, offered to the machine; a paste from the machine that's come, put on
    /// the pasteboard.
    pub fn follow(&mut self, desktop: &Shared) {
        if self.board.is_null() {
            return;
        }
        if let Some(rx) = &self.pending
            && let Ok(answer) = rx.try_recv()
        {
            self.pending = None;
            if let Some(text) = answer.and_then(|b| String::from_utf8(b).ok()) {
                msg!(isize; self.board, "clearContents");
                msg!(bool; self.board, "setString:forType:", ns_string(&text) => Id, pasteboard_string_type() => Id);
                self.change = msg!(isize; self.board, "changeCount");
            }
        }
        let change = msg!(isize; self.board, "changeCount");
        if change == self.change {
            return;
        }
        self.change = change;
        let text = msg!(Id; self.board, "stringForType:", pasteboard_string_type() => Id);
        if text.is_null() {
            return;
        }
        let text = rust_string(text);
        let waker = {
            let mut d = desktop::lock(desktop);
            let source = d.offer(Selection::Clipboard, None, Vec::new());
            d.offer_data(source, TEXT, text.into_bytes());
            self.ours = Some(source);
            if d.has_events() { d.waker() } else { None }
        };
        // Whoever had the clipboard hears it's gone (WM_DESTROYCLIPBOARD).
        if let Some(wake) = waker {
            wake();
        }
    }

    /// The machine's clipboard was offered anew: unless it's ours, its text is pasted (on another thread, its kernel
    /// may have to answer) and put on the pasteboard when it comes.
    pub fn offered(&mut self, desktop: &Shared) {
        let (source, has_text) = {
            let d = desktop::lock(desktop);
            let source = d.selection_source(Selection::Clipboard);
            let has_text = d.selection(Selection::Clipboard).is_some_and(|(_, mimes)| mimes.iter().any(|m| m == TEXT));
            (source, has_text)
        };
        if source.is_none() || source == self.ours || !has_text {
            return;
        }
        self.ours = source;
        let (tx, rx) = mpsc::channel();
        let desktop = desktop.clone();
        let _ = std::thread::Builder::new().name("carbon pasteboard".into()).spawn(move || {
            let _ = tx.send(desktop::paste(&desktop, Selection::Clipboard, TEXT, PASTE_WAIT));
        });
        self.pending = Some(rx);
    }
}
