//! The Mac's text input: a view of our own that's an `NSTextInputClient`, so keys go through the input method (an
//! IME's composition, a dead key's accent, another layout's letters) before they reach the machine. What it gives
//! back is gathered here for the key's handler to decide on: a key, text, or a composition (`Input::Preedit`).

use std::cell::RefCell;
use std::ffi::c_void;

use crate::objc::{BOOL, Frame, Id, NIL, Point, Sel, class, define_class, msg, rust_string};

/// An `NSRange`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Range {
    location: u64,
    length: u64,
}

const NOT_FOUND: u64 = i64::MAX as u64;

/// What the input method did with a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Got {
    /// Text committed.
    Insert(String),
    /// What it's composing now (empty when it's done), with its selection in it (UTF-16 units).
    Marked(String, usize, usize),
    /// A command (a Return, an arrow, a Delete): the key itself.
    Command,
}

#[derive(Default)]
struct Ime {
    marked: String,
    got: Vec<Got>,
    /// Where the text cursor is on the screen (Cocoa's coordinates), for the input method's window.
    caret: Option<Box<dyn Fn() -> Option<Frame>>>,
}

thread_local! {
    static IME: RefCell<Ime> = RefCell::new(Ime::default());
}

/// Whether an input method is composing now.
pub(crate) fn composing() -> bool {
    IME.with(|i| !i.borrow().marked.is_empty())
}

/// Says where the text cursor is on the screen, when an input method asks.
pub(crate) fn set_caret(caret: Box<dyn Fn() -> Option<Frame>>) {
    IME.with(|i| i.borrow_mut().caret = Some(caret));
}

/// Gives a key event to the input method through `view`, and what it did with it.
pub(crate) fn interpret(view: Id, event: Id) -> Vec<Got> {
    IME.with(|i| i.borrow_mut().got.clear());
    let events = msg!(Id; class("NSArray"), "arrayWithObject:", event => Id);
    msg!((); view, "interpretKeyEvents:", events => Id);
    IME.with(|i| std::mem::take(&mut i.borrow_mut().got))
}

extern "C" fn yes(_: Id, _: Sel) -> bool {
    true
}

extern "C" fn insert_text(_: Id, _: Sel, text: Id, _: Range) {
    let text = rust_string(text);
    IME.with(|i| {
        let mut i = i.borrow_mut();
        if !i.marked.is_empty() {
            i.marked.clear();
            i.got.push(Got::Marked(String::new(), 0, 0));
        }
        i.got.push(Got::Insert(text));
    });
}

extern "C" fn set_marked_text(_: Id, _: Sel, text: Id, selected: Range, _: Range) {
    let text = rust_string(text);
    IME.with(|i| {
        let mut i = i.borrow_mut();
        i.marked = text.clone();
        i.got.push(Got::Marked(text, selected.location as usize, selected.length as usize));
    });
}

/// The input method's done: what it was composing is committed as it is.
extern "C" fn unmark_text(_: Id, _: Sel) {
    IME.with(|i| {
        let mut i = i.borrow_mut();
        let marked = std::mem::take(&mut i.marked);
        if !marked.is_empty() {
            i.got.push(Got::Marked(String::new(), 0, 0));
            i.got.push(Got::Insert(marked));
        }
    });
}

extern "C" fn has_marked_text(_: Id, _: Sel) -> bool {
    composing()
}

extern "C" fn marked_range(_: Id, _: Sel) -> Range {
    IME.with(|i| {
        let units = i.borrow().marked.encode_utf16().count() as u64;
        if units == 0 { Range { location: NOT_FOUND, length: 0 } } else { Range { location: 0, length: units } }
    })
}

extern "C" fn selected_range(_: Id, _: Sel) -> Range {
    Range { location: 0, length: 0 }
}

extern "C" fn valid_attributes(_: Id, _: Sel) -> Id {
    msg!(Id; class("NSArray"), "array")
}

extern "C" fn attributed_substring(_: Id, _: Sel, _: Range, _: *mut Range) -> Id {
    NIL
}

extern "C" fn character_index(_: Id, _: Sel, _: Point) -> u64 {
    NOT_FOUND
}

/// Where the text the input method is working on is, on the screen: the guest's text cursor, so the candidate
/// window opens beside it.
extern "C" fn first_rect(_: Id, _: Sel, _: Range, _: *mut Range) -> Frame {
    IME.with(|i| i.borrow().caret.as_ref().and_then(|caret| caret())).unwrap_or_default()
}

extern "C" fn do_command(_: Id, _: Sel, _: Sel) {
    IME.with(|i| i.borrow_mut().got.push(Got::Command));
}

/// The view class every window of ours shows the desktop in: an `NSView` that takes the keyboard and is a text
/// input client. Nil if it can't be made.
pub(crate) fn view_class() -> Id {
    let range = "{_NSRange=QQ}";
    let rect = "{CGRect={CGPoint=dd}{CGSize=dd}}";
    let methods: [(&str, *const c_void, String); 13] = [
        ("acceptsFirstResponder", yes as *const c_void, format!("{BOOL}@:")),
        ("insertText:replacementRange:", insert_text as *const c_void, format!("v@:@{range}")),
        ("setMarkedText:selectedRange:replacementRange:", set_marked_text as *const c_void, format!("v@:@{range}{range}")),
        ("unmarkText", unmark_text as *const c_void, "v@:".into()),
        ("hasMarkedText", has_marked_text as *const c_void, format!("{BOOL}@:")),
        ("markedRange", marked_range as *const c_void, format!("{range}@:")),
        ("selectedRange", selected_range as *const c_void, format!("{range}@:")),
        ("validAttributesForMarkedText", valid_attributes as *const c_void, "@@:".into()),
        ("attributedSubstringForProposedRange:actualRange:", attributed_substring as *const c_void, format!("@@:{range}^{range}")),
        ("characterIndexForPoint:", character_index as *const c_void, "Q@:{CGPoint=dd}".into()),
        ("firstRectForCharacterRange:actualRange:", first_rect as *const c_void, format!("{rect}@:{range}^{range}")),
        ("doCommandBySelector:", do_command as *const c_void, "v@::".into()),
        ("isOpaque", yes as *const c_void, format!("{BOOL}@:")),
    ];
    let methods: Vec<(&str, *const c_void, &str)> = methods.iter().map(|(s, f, t)| (*s, *f, t.as_str())).collect();
    define_class("CarbonView", "NSView", &methods, &["NSTextInputClient"])
}

/// A new view of that class, `width` by `height` points; nil if it can't be made.
pub(crate) fn new_view(width: f64, height: f64) -> Id {
    let class = view_class();
    if class.is_null() {
        return NIL;
    }
    let frame = Frame { origin: Point::default(), size: crate::objc::Size { width, height } };
    msg!(Id; msg!(Id; class, "alloc"), "initWithFrame:", frame => Frame)
}
