//! Observers: everything that shows or reads the desktop, each at its own pace. The headless screen, a host's windows,
//! `remote`'s viewers and `carbon display` all take the same thing: what changed since they last looked. That's the
//! latest state of each window that changed (never a backlog of every step on the way), the windows gone, the
//! stacking and the active window if they changed, and each window's content damage as a few rectangles. Pixels are
//! read from the committed buffers when they're wanted, in place.

use std::collections::{BTreeMap, BTreeSet};

use crate::Rect;
use crate::cursor::Cursor;
use crate::desktop::{Layer, Waker, WindowId};
use crate::output::Output;
use crate::selection::Selection;

/// An observer's handle.
pub type ObserverId = u64;

/// What a window is, as an observer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    /// The whole window, frame and all, on the screen; and its content.
    pub rect: Rect,
    pub content: Rect,
    pub title: String,
    /// With the desktop's frame now.
    pub framed: bool,
    pub layer: Layer,
    /// The window it's a dialog or popup of.
    pub parent: Option<WindowId>,
    pub popup: bool,
    /// On the screen: shown, and not minimized.
    pub on_screen: bool,
    /// `state::*`.
    pub states: u32,
    /// It wants the user (`set_attention`): an activation refused, a flash.
    pub attention: bool,
    /// Whose it is.
    pub client: crate::desktop::ClientId,
}

/// What changed since an observer last took.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    /// Each window that's new or changed, as it is now.
    pub windows: Vec<WindowInfo>,
    pub gone: Vec<WindowId>,
    /// The stacking, bottom to top, if it changed.
    pub stacking: Option<Vec<WindowId>>,
    /// The active window, if that changed.
    pub active: Option<Option<WindowId>>,
    /// What changed in each window's content, in its content's coordinates.
    pub damage: Vec<(WindowId, Vec<Rect>)>,
    /// The pointer's cursor, if that changed.
    pub cursor: Option<Cursor>,
    /// The window taking text from an input method, and where its text cursor is (in its content's coordinates),
    /// if that changed: where a host puts its input method's window.
    pub text_input: Option<Option<(WindowId, Rect)>>,
    /// The output, if it changed: its size, scale and the rest.
    pub output: Option<Output>,
    /// The selections offered anew (read them with `Desktop::selection`): the host's clipboard follows.
    pub selections: Vec<Selection>,
    /// What changed on the composited screen (`Desktop::screen`), frames and all: the part to show again, for an
    /// observer that shows the whole desktop (the host's desktop mode, a viewer, a screenshot taken now and then).
    pub screen: Rect,
}

impl Observed {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
            && self.gone.is_empty()
            && self.stacking.is_none()
            && self.active.is_none()
            && self.damage.is_empty()
            && self.cursor.is_none()
            && self.text_input.is_none()
            && self.output.is_none()
            && self.selections.is_empty()
            && self.screen.is_empty()
    }
}

/// Past this many separate rectangles a window's damage becomes the one around them all.
const MOST_RECTS: usize = 8;

#[derive(Default)]
struct Observer {
    changed: BTreeSet<WindowId>,
    gone: Vec<WindowId>,
    stacking: bool,
    active: bool,
    damage: BTreeMap<WindowId, Vec<Rect>>,
    cursor: bool,
    text_input: bool,
    output: bool,
    selections: Vec<Selection>,
    screen: Rect,
    waker: Option<Waker>,
}

impl Observer {
    fn pending(&self) -> bool {
        !self.changed.is_empty() || !self.gone.is_empty() || self.stacking || self.active || !self.damage.is_empty() || self.cursor || self.text_input || self.output || !self.selections.is_empty() || !self.screen.is_empty()
    }
}

/// Every observer's account of what it hasn't taken yet.
#[derive(Default)]
pub(crate) struct Observers {
    each: BTreeMap<ObserverId, Observer>,
    next: ObserverId,
}

/// What an observer took: which windows changed or went, whether the stacking and active window did, and damage.
pub(crate) struct Taken {
    pub changed: Vec<WindowId>,
    pub gone: Vec<WindowId>,
    pub stacking: bool,
    pub active: bool,
    pub damage: Vec<(WindowId, Vec<Rect>)>,
    pub cursor: bool,
    pub text_input: bool,
    pub output: bool,
    pub selections: Vec<Selection>,
    pub screen: Rect,
}

/// One change, to note for every observer.
pub(crate) enum Note {
    Window(WindowId),
    Gone(WindowId),
    Stacking,
    Active,
    Damage(WindowId, Rect),
    Cursor,
    TextInput,
    Output,
    Selection(Selection),
    /// Part of the composited screen changed.
    Screen(Rect),
}

impl Observers {
    /// A new observer, which has everything still to take: `all` windows, each of whose content is `content`, and the
    /// whole `screen`.
    pub fn add(&mut self, all: impl Iterator<Item = (WindowId, Rect)>, screen: Rect) -> ObserverId {
        self.next += 1;
        let mut o = Observer { stacking: true, active: true, cursor: true, text_input: true, output: true, screen, ..Observer::default() };
        for (id, content) in all {
            o.changed.insert(id);
            o.damage.insert(id, vec![Rect::new(0, 0, content.width(), content.height())]);
        }
        self.each.insert(self.next, o);
        self.next
    }

    pub fn remove(&mut self, id: ObserverId) {
        self.each.remove(&id);
    }

    pub fn set_waker(&mut self, id: ObserverId, waker: Waker) {
        if let Some(o) = self.each.get_mut(&id) {
            o.waker = Some(waker);
        }
    }

    /// Notes a change for every observer, waking each that had nothing pending. Called with the desktop's lock held.
    pub fn note(&mut self, note: Note) {
        for o in self.each.values_mut() {
            let was = o.pending();
            match &note {
                Note::Window(id) => {
                    o.changed.insert(*id);
                }
                Note::Gone(id) => {
                    o.changed.remove(id);
                    o.damage.remove(id);
                    o.gone.push(*id);
                }
                Note::Stacking => o.stacking = true,
                Note::Cursor => o.cursor = true,
                Note::TextInput => o.text_input = true,
                Note::Output => o.output = true,
                Note::Selection(which) => {
                    if !o.selections.contains(which) {
                        o.selections.push(*which);
                    }
                }
                Note::Active => o.active = true,
                Note::Damage(id, r) if !r.is_empty() => add_damage(o.damage.entry(*id).or_default(), *r),
                Note::Damage(..) => {}
                Note::Screen(r) if !r.is_empty() => o.screen = if o.screen.is_empty() { *r } else { o.screen.union(r) },
                Note::Screen(_) => {}
            }
            if !was
                && o.pending()
                && let Some(wake) = &o.waker
            {
                wake();
            }
        }
    }

    /// What `id` hasn't taken yet, and now has. The desktop fills in the windows' states as they are now.
    pub fn take(&mut self, id: ObserverId) -> Option<Taken> {
        let o = self.each.get_mut(&id)?;
        Some(Taken {
            changed: std::mem::take(&mut o.changed).into_iter().collect(),
            gone: std::mem::take(&mut o.gone),
            stacking: std::mem::take(&mut o.stacking),
            active: std::mem::take(&mut o.active),
            damage: std::mem::take(&mut o.damage).into_iter().collect(),
            cursor: std::mem::take(&mut o.cursor),
            text_input: std::mem::take(&mut o.text_input),
            output: std::mem::take(&mut o.output),
            selections: std::mem::take(&mut o.selections),
            screen: std::mem::take(&mut o.screen),
        })
    }
}

/// Adds a rectangle to a window's damage: merged with one it overlaps, else kept apart, up to a few.
fn add_damage(rects: &mut Vec<Rect>, r: Rect) {
    let mut r = r;
    while let Some(i) = rects.iter().position(|o| !o.intersect(&r).is_empty()) {
        r = r.union(&rects.swap_remove(i));
    }
    rects.push(r);
    if rects.len() > MOST_RECTS {
        let all = rects.iter().fold(Rect::default(), |acc, r| if acc.is_empty() { *r } else { acc.union(r) });
        *rects = vec![all];
    }
}
