//! The desktop: the window manager both kernels share. It has a machine's top-level windows, in their places and
//! stacking order, draws their frames, decides which is active, routes input to them, and composites them into one
//! screen. What's inside a window (win32k's child windows and messages, Wayland's surfaces) is its kernel's.
//!
//! A kernel makes and places windows, shows its pixels in them as committed buffers ([`crate::buffer`]), and asks to
//! raise or activate them; it reads back [`Event`]s: windows moved or resized by their frames, activated or not, asked
//! to close, pointer and key input in their content's coordinates, buffers released, and frames to draw.
//!
//! Content is Wayland's model, which the other APIs map onto: a window's buffer, its damage and a frame request are
//! pending until `commit`, which makes them current at once. A window that asked for a frame gets `Frame` at the next
//! tick of the desktop's clock (the host's refresh, or [`crate::clock`]'s for a headless desktop), so nothing draws
//! faster than it can be seen. Input comes in as [`Input`], from the host, from `carbon display`, or from a
//! program (`SendInput`); [`send`] wakes the kernel to read what it made.
//!
//! This is the built-in backend, on a screen in memory. A host's windows are the other, behind the same calls.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};

use crate::buffer::{Buffer, BufferId, Format};
use crate::cursor::{Cursor, Shape};
use crate::keys;
use crate::observe::{Note, ObserverId, Observed, Observers, WindowInfo};
use crate::output::{Output, SCALE_1X};
use crate::selection::{MOST, Selection, Selections, Source, SourceId, Transfer, TransferId};
use crate::positioner::Positioner;
use crate::{Rect, Rgb, Surface, font};

pub type WindowId = u64;

/// Who a window belongs to: a process (win32k's), a connection (Wayland's, X11's). The focus policy's unit: a client
/// may move the focus between its own windows, and to another's only with the user's leave (`request_activation`).
pub type ClientId = u64;

/// An activation token (`activation_token`): xdg_activation_v1's, `XDG_ACTIVATION_TOKEN` and `_NET_STARTUP_ID` for a
/// program launched, Windows' `AllowSetForegroundWindow`. Good once, and only until the user's next press or key.
pub type Token = u32;

/// How fast a held key repeats: what Wayland sends clients (`wl_keyboard.repeat_info`) to repeat keys themselves, X11's
/// `XkbGetAutoRepeatRate`, Windows' `SPI_GETKEYBOARDSPEED` and `SPI_GETKEYBOARDDELAY`. The repeats themselves come from
/// the source (a host's keyboard, a viewer's), each marked `repeat` in its `Key` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRepeat {
    /// Repeats a second, 0 for none.
    pub rate: u32,
    /// Milliseconds before the first.
    pub delay: u32,
}

impl Default for KeyRepeat {
    fn default() -> KeyRepeat {
        KeyRepeat { rate: 30, delay: 500 }
    }
}

/// What a drag may do with what it carries, and what its target chose: one or more of these. Windows' `DROPEFFECT_*`
/// and X11's `XdndAction*` are the same three; Wayland's `ask` (choosing after the drop, from a menu) is `ASK`.
pub mod action {
    pub const COPY: u8 = 1;
    pub const MOVE: u8 = 2;
    pub const LINK: u8 = 4;
    pub const ASK: u8 = 8;
}

/// A drag as it is now, for whoever drives or watches it (`Desktop::drag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragInfo {
    /// The window it started from, or None for outside the machine (the host, `carbon display drop`).
    pub source: Option<WindowId>,
    pub mimes: Vec<String>,
    /// What the source lets it do.
    pub actions: u8,
    /// The window it's over, and what that would take it as and do with it (0: nothing; it won't take it there).
    pub target: Option<WindowId>,
    pub mime: Option<String>,
    pub action: u8,
    /// Dropped, and waiting for the target to say it's done (`finish_drop`).
    pub dropped: bool,
}

struct Drag {
    info: DragInfo,
    /// What the target takes, and which of those it prefers (`accept_drag`).
    accepts: u8,
    prefers: u8,
}

/// What a window asks of the pointer while it's the active window (`set_pointer_constraint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    /// It stays where it is, hidden, and only relative motion comes (`Pointer::Relative`), for games and remote
    /// desktops: Wayland's `zwp_locked_pointer_v1`. Taken when the pointer's in its content.
    Lock,
    /// It's kept within a rectangle, in the window's content coordinates (it may reach past the content): Wayland's
    /// `zwp_confined_pointer_v1`, Windows' `ClipCursor`. Taken at once, the pointer moved inside.
    Confine(Rect),
}

/// A frame's border, where it can be resized from.
pub const BORDER: i32 = 4;
/// A frame's title bar, below its top border.
pub const CAPTION: i32 = 20;
/// How far along an edge from a corner still resizes from that corner.
const CORNER: i32 = 16;
const MIN_WIDTH: i32 = 2 * BORDER + 4 * CAPTION;
const MIN_HEIGHT: i32 = 2 * BORDER + CAPTION;

const BACKGROUND: Rgb = 0x2b_3a4a;
const OUTLINE: Rgb = 0x1a_2129;
const ACTIVE: Rgb = 0x3d_5a80;
const INACTIVE: Rgb = 0x8a_96a3;
const TITLE: Rgb = 0xff_ffff;

/// How far a window's content is from its edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    /// A window's, with a frame or without.
    pub fn of(framed: bool) -> Insets {
        if framed { Insets { left: BORDER, top: BORDER + CAPTION, right: BORDER, bottom: BORDER } } else { Insets::default() }
    }

    /// The content within a window's rectangle.
    pub fn inside(&self, r: &Rect) -> Rect {
        Rect::new(r.left + self.left, r.top + self.top, (r.right - self.right).max(r.left + self.left), (r.bottom - self.bottom).max(r.top + self.top))
    }

    /// The window around some content.
    pub fn around(&self, r: &Rect) -> Rect {
        Rect::new(r.left - self.left, r.top - self.top, r.right + self.right, r.bottom + self.bottom)
    }
}

/// Windows always on top go above the rest, and popups above them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Layer {
    /// Below every other: a desktop's icons, X11's `_NET_WM_WINDOW_TYPE_DESKTOP` and `_NET_WM_STATE_BELOW`, a
    /// layer-shell background.
    Below,
    #[default]
    Normal,
    /// Always on top: Windows' `WS_EX_TOPMOST`, X11's `_NET_WM_STATE_ABOVE`, docks and notifications. Below a fullscreen
    /// window while that's active.
    Top,
    /// Popups' own: a `Spec` asks for `Normal` or `Top`.
    Popup,
}

/// What a window is, beside where: in a `Configure` event and from `states`.
pub mod state {
    pub const MAXIMIZED: u32 = 1;
    /// Kept, but not on the screen, until it's restored.
    pub const MINIMIZED: u32 = 2;
    /// The whole screen, with no frame, above the windows that aren't.
    pub const FULLSCREEN: u32 = 4;
    /// Being resized by its frame, until the button's up.
    pub const RESIZING: u32 = 8;
}

/// What a change to a window's frame keeps where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Content,
    Rect,
}

/// What a window is to the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Toplevel,
    /// Transient for `parent`: a dialog, or any window another owns (win32k's owned windows, X11's
    /// `WM_TRANSIENT_FOR`, `xdg_toplevel.set_parent`). Kept above it; it's active again when this one goes.
    Dialog { parent: WindowId },
    /// A menu, a tooltip, a list: placed against its parent by `positioner`, moved with it, never framed and never
    /// the active window (its parent stays active). With `grab` it takes the keyboard and pointer while it's shown,
    /// and a press outside it and the popups it opened from dismisses them all (`PopupDone`), as a menu closes.
    Popup { parent: WindowId, positioner: Positioner, grab: bool },
}

/// A new window.
#[derive(Debug, Clone, Default)]
pub struct Spec {
    /// The whole window, frame and all, on the screen.
    pub rect: Rect,
    pub title: String,
    /// With the desktop's frame, or drawing its own (or none).
    pub framed: bool,
    pub layer: Layer,
    pub role: Role,
    /// Its content's colour until it's drawn.
    pub fill: Rgb,
    /// Whose it is, for the focus policy.
    pub client: ClientId,
    /// A click neither activates nor raises it (Windows' `WS_EX_NOACTIVATE`, X11's docks and `WM_HINTS` input
    /// False, a layer-shell surface without keyboard interactivity), and it isn't made active when another goes.
    pub no_activate: bool,
}

/// Where `Desktop::stack` puts a window (with the windows it owns) among the others of its band: Windows'
/// `SetWindowPos` insert-after (`HWND_TOP`, `HWND_BOTTOM`, another window to go behind), X11's `ConfigureWindow` stack
/// mode with a sibling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stack {
    Top,
    Bottom,
    Above(WindowId),
    Below(WindowId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn bit(self) -> u8 {
        match self {
            Button::Left => 1,
            Button::Right => 2,
            Button::Middle => 4,
        }
    }
}

/// What came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The pointer, to a place on the screen.
    PointerTo(i32, i32),
    /// The pointer, moved by this much, as a mouse reports it (not accelerated, and not stopped by the screen's
    /// edge): what a host with its own pointer locked sends. Relative motion carries it as it came.
    PointerBy(i32, i32),
    Button(Button, bool),
    /// Scrolling, in 120ths of a wheel's notch (Windows' `WHEEL_DELTA` and Wayland's `axis_value120`'s unit), so a
    /// trackpad's finer steps go through: `y` up (away from the user) and `x` right are positive.
    Scroll { x: i32, y: i32 },
    /// A key (an evdev number), down or up. A down for a key that's down already is its repeat.
    Key(u16, bool),
    /// Text an input method committed (or that no key types): to the window that has the keyboard.
    Text(String),
    /// What an input method is composing, not yet committed, with its own cursor or selection (byte offsets); empty
    /// when it's done. Only to a window that takes text input (`set_text_input`).
    Preedit { text: String, cursor: Option<(usize, usize)> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Move,
    Down(Button),
    Up(Button),
    /// As `Input::Scroll`.
    Scroll { x: i32, y: i32 },
    /// The pointer came into its content (at the event's place), or left it: Wayland's `wl_pointer` enter and leave,
    /// X11's `EnterNotify` and `LeaveNotify`. Not while a button's held: the window it went down on keeps it.
    Enter,
    Leave,
    /// The pointer moved by this much, unaccelerated, to a window that asked (`set_relative_motion`): Wayland's
    /// `zwp_relative_pointer_v1`, Windows' raw input. While it's locked, the only motion that comes.
    Relative { dx: i32, dy: i32 },
}

/// What a kernel reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Where it is now and what it is, as the desktop decided (the user dragged its frame, it was maximized, made
    /// fullscreen, minimized or restored, a popup placed again): its rectangle, frame and all, its `state::*`, and a
    /// serial its kernel may acknowledge (`ack_configure`). Wayland's `xdg_toplevel` configure, X11's
    /// `ConfigureNotify` and `WM_SIZE` are each read from it.
    Configure { window: WindowId, rect: Rect, states: u32, serial: u32 },
    /// It became the active window, or stopped being.
    Activated { window: WindowId, active: bool },
    /// Its close button was clicked.
    Close { window: WindowId },
    /// The pointer, in its content's coordinates, with the buttons and modifiers held after it.
    Pointer { window: WindowId, x: i32, y: i32, what: Pointer, buttons: u8, modifiers: u8 },
    /// A key, to the active window (or to the popup that has the keyboard). `repeat`: it's held, and this is its
    /// repeat, which Windows programs get (`WM_KEYDOWN` again) and Wayland's clients don't (they repeat it themselves).
    Key { window: WindowId, code: u16, down: bool, repeat: bool, modifiers: u8 },
    /// A popup was dismissed (a press outside its grab, or its parent hidden or gone): it's hidden already, and its
    /// kernel tells its client. The topmost of a chain comes first.
    PopupDone { window: WindowId },
    /// Text committed to the window with the keyboard: `wl_keyboard`-free text for `text_input_v3`, `WM_CHAR`s.
    Text { window: WindowId, text: String },
    /// What an input method is composing for it (empty when it's done), and where its cursor is in that.
    Preedit { window: WindowId, text: String, cursor: Option<(usize, usize)> },
    /// Its offer of a selection was taken by another's: it may let go of what it offered.
    SelectionCancelled { window: WindowId, selection: Selection },
    /// A paste wants the data of its offer as `mime`: its kernel answers with `provide(transfer, ..)`.
    SendSelection { window: WindowId, selection: Selection, mime: String, transfer: TransferId },
    /// What a paste it asked for came to (None if the owner had nothing to give).
    SelectionData { window: WindowId, transfer: TransferId, data: Option<Vec<u8>> },
    /// The scale a window's buffers had best be drawn at (in 120ths, as `Output::scale`), as it changed: Wayland's
    /// preferred buffer scale and fractional scale, Windows' `WM_DPICHANGED`, X11's `Xft.dpi`.
    Scale { window: WindowId, scale: u32 },
    /// A buffer committed before is no longer shown: its kernel may reuse it.
    Released { window: WindowId, buffer: BufferId },
    /// Time to draw the next frame, as it asked at its last commit: the clock's time, in milliseconds.
    Frame { window: WindowId, time: u32 },
    /// A commit that asked (`request_presented`) reached the screen, at the clock's time in milliseconds, in the
    /// screen's `seq`th refresh; or with None, it never will (a newer commit replaced it first, or the window isn't on
    /// the screen): Wayland's presentation-time `presented` and `discarded`, DXGI's frame statistics.
    Presented { window: WindowId, time: Option<u32>, seq: u64 },
    /// Its pointer constraint was taken (it's the active window, and for a lock, the pointer's in its content), or let
    /// go (it isn't any more, or the constraint was taken away): Wayland's `locked` and `unlocked`, `confined` and
    /// `unconfined`. It stays asked for, and is taken again when it can be.
    Constrained { window: WindowId, on: bool },
    /// A drag came over its content, at this point, carrying `mimes` that the source lets it `actions` (`action::*`):
    /// Wayland's `wl_data_device.enter` with its offer, X11's `XdndEnter`, `IDropTarget::DragEnter`. Its kernel says what
    /// it would take with `accept_drag`, and may read the data (`request(Selection::Drag, ..)`) at any time until it ends.
    DragEnter { window: WindowId, x: i32, y: i32, mimes: Vec<String>, actions: u8 },
    /// The drag moved over it (merged while unread).
    DragMotion { window: WindowId, x: i32, y: i32 },
    /// The drag left it, or ended without being dropped on it.
    DragLeave { window: WindowId },
    /// It was dropped on it, at this point, to be taken as it last accepted with `action`: its kernel reads the data
    /// and says it's done with `finish_drop`. Wayland's `drop`, `XdndDrop`, `IDropTarget::Drop`.
    Drop { window: WindowId, x: i32, y: i32, action: u8 },
    /// To the drag's source: what the window it's over would take it as, and do with it (0 for nothing): Wayland's
    /// `wl_data_source.target` and `action`, X11's `XdndStatus`, `IDropSource::GiveFeedback`.
    DragAction { window: WindowId, mime: Option<String>, action: u8 },
    /// To the drag's source: it was dropped where it's taken, and the target's reading it (`dnd_drop_performed`).
    Dropped { window: WindowId },
    /// To the drag's source: it's over, done with `action`, or 0 if it was cancelled or dropped where nothing took it
    /// (Wayland's `dnd_finished` or `cancelled`, `XdndFinished`, what `DoDragDrop` returns). A `MOVE` is the source's to
    /// carry out, by deleting what it gave.
    DragEnd { window: WindowId, action: u8 },
}

/// Called when there are events to read, without the desktop's lock held.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

struct Window {
    rect: Rect,
    title: String,
    client: ClientId,
    no_activate: bool,
    /// It asked to be active and wasn't let, or its kernel says it wants the user: until it's active.
    attention: bool,
    /// What it asks of the pointer while it's active, and whether it takes relative motion.
    constraint: Option<Constraint>,
    relative: bool,
    framed: bool,
    layer: Layer,
    owner: Option<WindowId>,
    /// A popup's placement, and whether it grabs.
    popup: Option<(Positioner, bool)>,
    /// Shown, as its kernel has it; on the screen too unless it's minimized.
    visible: bool,
    /// `state::*`.
    states: u32,
    /// Where it was before it was maximized or made fullscreen, to go back to.
    restore: Option<Rect>,
    /// Its content's smallest and largest size, 0 for no limit.
    min_size: (i32, i32),
    max_size: (i32, i32),
    /// The last `Configure` its kernel acknowledged.
    acked: u32,
    /// The pointer's cursor over its content.
    cursor: Cursor,
    /// Takes text from an input method, with its text cursor here (in its content's coordinates).
    text_input: Option<Rect>,
    /// Where its content takes pointer input (in its content's coordinates), or None for all of it: Wayland's input
    /// region, X11's `ShapeInput`. Elsewhere, the pointer goes through to what's below (a client's shadow, an
    /// overlay that only draws).
    input: Option<Vec<Rect>>,
    /// What its content shows where no buffer covers it.
    fill: Rgb,
    /// What it shows now, as last committed, and how many of its pixels make a logical one.
    buffer: Option<Buffer>,
    buffer_scale: u32,
    /// How far its buffer's top left is left of and above its content's, in logical pixels, with all of its buffer
    /// shown: a client's own shadow around its window (Wayland's window geometry, X11's `_GTK_FRAME_EXTENTS`). None:
    /// at its content's top left, shown only within its content.
    offset: Option<(i32, i32)>,
    pending: Pending,
}

/// What's waiting for a window's next commit.
#[derive(Default)]
struct Pending {
    /// A buffer attached (or None, detached) since the last commit.
    buffer: Option<Option<Buffer>>,
    /// What changed, in the buffer's coordinates.
    damage: Rect,
    frame: bool,
    /// Wants a `Presented` for this commit.
    presented: bool,
    /// How many of the buffer's pixels to a logical one, from the next commit.
    scale: Option<u32>,
    /// Its buffer's offset, from the next commit.
    offset: Option<Option<(i32, i32)>>,
}

impl Window {
    /// Whether it has the desktop's frame now: not while it's fullscreen.
    fn has_frame(&self) -> bool {
        self.framed && self.states & state::FULLSCREEN == 0
    }

    /// On the screen: shown, and not minimized.
    fn on_screen(&self) -> bool {
        self.visible && self.states & state::MINIMIZED == 0
    }

    /// Maximized or fullscreen: its place is the desktop's, so its frame doesn't move or resize it.
    fn placed_by_desktop(&self) -> bool {
        self.states & (state::MAXIMIZED | state::FULLSCREEN) != 0
    }

    fn content_rect(&self) -> Rect {
        Insets::of(self.has_frame()).inside(&self.rect)
    }

    /// Where its buffer is on the screen: at its content's top left, or that less its offset.
    fn buffer_rect(&self) -> Option<Rect> {
        let b = self.buffer.as_ref()?;
        let (c, s, (bw, bh)) = (self.content_rect(), self.buffer_scale.max(1) as i32, b.pixels.size());
        let (ox, oy) = self.offset.unwrap_or((0, 0));
        Some(Rect::new(c.left - ox, c.top - oy, c.left - ox + bw / s, c.top - oy + bh / s))
    }

    /// All it covers on the screen: its rectangle, and with an offset, all of its buffer too (a client's shadow).
    fn extent(&self) -> Rect {
        match (self.offset, self.buffer_rect()) {
            (Some(_), Some(b)) => self.rect.union(&b),
            _ => self.rect,
        }
    }

    fn caption(&self) -> Rect {
        let r = &self.rect;
        Rect::new(r.left + BORDER, r.top + BORDER, r.right - BORDER, r.top + BORDER + CAPTION)
    }

    fn close_button(&self) -> Rect {
        let c = self.caption();
        Rect::new(c.right - CAPTION + 2, c.top + 2, c.right - 2, c.bottom - 2)
    }

    /// Beside the close button: maximizes it, or restores it.
    fn maximize_button(&self) -> Rect {
        self.close_button().offset(-CAPTION, 0)
    }

    /// What part of it is at a point on the screen.
    /// Whether the pointer at `(x, y)` is its: within it, and if it's in its content, within its input region.
    fn takes_pointer(&self, x: i32, y: i32) -> bool {
        if !self.rect.contains(x, y) {
            return false;
        }
        let c = self.content_rect();
        match &self.input {
            Some(region) if c.contains(x, y) => region.iter().any(|r| r.contains(x - c.left, y - c.top)),
            _ => true,
        }
    }

    fn part(&self, x: i32, y: i32) -> Option<Part> {
        if !self.rect.contains(x, y) {
            return None;
        }
        if !self.has_frame() || self.content_rect().contains(x, y) {
            return Some(Part::Content);
        }
        if self.close_button().contains(x, y) {
            return Some(Part::Close);
        }
        if self.maximize_button().contains(x, y) {
            return Some(Part::Maximize);
        }
        if self.caption().contains(x, y) {
            return Some(Part::Caption);
        }
        let r = &self.rect;
        let mut edges = 0;
        if x < r.left + BORDER || (x < r.left + CORNER && (y < r.top + BORDER || y >= r.bottom - BORDER)) {
            edges |= LEFT;
        }
        if x >= r.right - BORDER || (x >= r.right - CORNER && (y < r.top + BORDER || y >= r.bottom - BORDER)) {
            edges |= RIGHT;
        }
        if y < r.top + BORDER || (y < r.top + CORNER && (x < r.left + BORDER || x >= r.right - BORDER)) {
            edges |= TOP;
        }
        if y >= r.bottom - BORDER || (y >= r.bottom - CORNER && (x < r.left + BORDER || x >= r.right - BORDER)) {
            edges |= BOTTOM;
        }
        Some(Part::Edge(edges))
    }
}

/// A window's edges, for `begin_resize`: one, or two for a corner.
pub mod edge {
    pub const LEFT: u8 = 1;
    pub const TOP: u8 = 2;
    pub const RIGHT: u8 = 4;
    pub const BOTTOM: u8 = 8;
}
use edge::{BOTTOM, LEFT, RIGHT, TOP};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Content,
    Caption,
    Close,
    Maximize,
    Edge(u8),
}

/// What a held button is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grab {
    Content(WindowId),
    Move { window: WindowId, dx: i32, dy: i32 },
    Resize { window: WindowId, edges: u8, start: Rect, from: (i32, i32) },
    Close(WindowId),
    Maximize(WindowId),
    /// A press that dismissed popups: nothing more happens until the buttons are up.
    Dismissed,
    /// A drag: the pointer carries it over windows, and letting go drops it.
    Drag,
}

pub struct Desktop {
    screen: Surface,
    windows: BTreeMap<WindowId, Window>,
    /// Bottom to top.
    order: Vec<WindowId>,
    next: WindowId,
    /// The last `Configure`'s serial.
    serial: u32,
    active: Option<WindowId>,
    pointer: (i32, i32),
    buttons: u8,
    modifiers: u8,
    grab: Option<Grab>,
    /// The window pointer events go to, whatever's under it (`set_capture`).
    capture: Option<WindowId>,
    /// The window the pointer's in, as its last enter said.
    hover: Option<WindowId>,
    /// The cursor shown now, and the text input an input method serves now.
    cursor: Cursor,
    text_input: Option<(WindowId, Rect)>,
    events: VecDeque<Event>,
    /// What's changed on the screen since it was last composited.
    damage: Rect,
    waker: Option<Waker>,
    /// Others woken with it: each server of a kernel that has more than one (lambda's Wayland and X11).
    wakers: Vec<Waker>,
    /// Windows that asked for a frame at their last commit, and haven't had it yet.
    frames: BTreeSet<WindowId>,
    /// Told when a window starts waiting for a frame, so a clock that's idle starts ticking.
    clock: Option<Waker>,
    /// What shows or reads it, and what each hasn't taken yet.
    observers: Observers,
    /// What it's shown on: its size is the screen's.
    output: Output,
    /// The clipboard and primary selection, and pastes on their way.
    selections: Selections,
    /// The user's presses and key downs so far, and whose window the last went to: the focus policy's evidence.
    input_serial: u32,
    last_input: Option<ClientId>,
    /// Activation tokens issued and not yet used, with the input serial each was issued at.
    tokens: BTreeMap<Token, u32>,
    next_token: Token,
    /// Keys down now, so a down for one already is known as its repeat; and how fast they repeat.
    keys_down: BTreeSet<u16>,
    repeat: KeyRepeat,
    /// The window whose pointer constraint is taken now.
    constrained: Option<WindowId>,
    /// Windows whose last commit wants a `Presented`, and how many refreshes the screen's had.
    presenting: BTreeSet<WindowId>,
    refreshes: u64,
    /// The drag going on, if there's one; and what the last from outside the machine ended with, for it to read.
    drag: Option<Drag>,
    outside_drag_end: Option<u8>,
    /// The screen at the output's scale, in its own pixels (`screen_pixels`), and what's changed on it since it was
    /// last made, in logical pixels. None at 1×, where it's the logical screen.
    hires: Option<Surface>,
    hires_damage: Rect,
}

impl std::fmt::Debug for Desktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desktop").field("size", &(self.screen.width, self.screen.height)).field("windows", &self.order).field("active", &self.active).finish()
    }
}

/// A desktop shared between its kernel and whatever sends it input.
pub type Shared = Arc<Mutex<Desktop>>;

pub fn lock(desktop: &Mutex<Desktop>) -> std::sync::MutexGuard<'_, Desktop> {
    desktop.lock().unwrap_or_else(|p| p.into_inner())
}

/// Pastes a selection as `mime` for something outside the machine (the host's clipboard, `carbon display`): asks, wakes
/// the owner's kernel if it must answer, and waits for the answer up to `timeout`. None if it isn't offered as that, or
/// no answer came.
pub fn paste(desktop: &Mutex<Desktop>, which: Selection, mime: &str, timeout: Duration) -> Option<Vec<u8>> {
    let (transfer, waker) = {
        let mut d = lock(desktop);
        let t = d.request(which, mime, None)?;
        (t, if d.events.is_empty() { None } else { d.waker() })
    };
    if let Some(wake) = waker {
        wake();
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(answer) = lock(desktop).take_transfer(transfer) {
            return answer;
        }
        if Instant::now() >= deadline {
            lock(desktop).selections.transfers.remove(&transfer);
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Drags data in from outside the machine (the host, `carbon display drop`) and drops it at a point on the screen:
/// `data` is each MIME type with its bytes, and `actions` what it may be done with (`action::*`). The window under the
/// point hears it come, its kernel woken; this waits up to `timeout` for it to say what it would take, drops it there,
/// and waits up to `timeout` again for it to finish. What it was done with, or 0 if nothing there took it.
pub fn drop_from_outside(desktop: &Mutex<Desktop>, x: i32, y: i32, data: Vec<(String, Vec<u8>)>, actions: u8, timeout: Duration) -> Result<u8, &'static str> {
    let wake = |d: &Desktop| if d.events.is_empty() { None } else { d.waker() };
    let waker = {
        let mut d = lock(desktop);
        let mimes = data.iter().map(|(m, _)| m.clone()).collect();
        let source = d.start_drag(None, mimes, actions).ok_or("a drag is going on already")?;
        for (mime, bytes) in data {
            d.offer_data(source, &mime, bytes);
        }
        d.input(Input::PointerTo(x, y));
        wake(&d)
    };
    if let Some(w) = waker {
        w();
    }
    let deadline = Instant::now() + timeout;
    loop {
        let d = lock(desktop);
        let info = d.drag().ok_or("the drag ended before it was dropped")?;
        if info.target.is_none() || info.action != 0 || Instant::now() >= deadline {
            break;
        }
        drop(d);
        std::thread::sleep(Duration::from_millis(5));
    }
    let waker = {
        let mut d = lock(desktop);
        d.drop_drag();
        wake(&d)
    };
    if let Some(w) = waker {
        w();
    }
    let deadline = Instant::now() + timeout;
    loop {
        let mut d = lock(desktop);
        if let Some(action) = d.take_outside_drag_end() {
            return Ok(action);
        }
        if Instant::now() >= deadline {
            d.cancel_drag();
            d.take_outside_drag_end();
            return Err("the window it was dropped on never finished taking it");
        }
        drop(d);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Gives the desktop some input, and wakes its kernel if that made events.
pub fn send(desktop: &Mutex<Desktop>, input: Input) {
    let waker = {
        let mut d = lock(desktop);
        d.input(input);
        if d.events.is_empty() { None } else { d.waker() }
    };
    if let Some(wake) = waker {
        wake();
    }
}

impl Desktop {
    pub fn new(width: i32, height: i32) -> Desktop {
        let screen = Surface::new(width, height, BACKGROUND);
        Desktop {
            damage: screen.bounds(),
            screen,
            windows: BTreeMap::new(),
            order: Vec::new(),
            next: 1,
            serial: 0,
            active: None,
            pointer: (width / 2, height / 2),
            buttons: 0,
            modifiers: 0,
            grab: None,
            capture: None,
            hover: None,
            cursor: Cursor::default(),
            text_input: None,
            events: VecDeque::new(),
            waker: None,
            wakers: Vec::new(),
            frames: BTreeSet::new(),
            clock: None,
            observers: Observers::default(),
            output: Output::headless(width, height, SCALE_1X),
            selections: Selections::default(),
            input_serial: 0,
            last_input: None,
            tokens: BTreeMap::new(),
            next_token: 1,
            keys_down: BTreeSet::new(),
            repeat: KeyRepeat::default(),
            constrained: None,
            presenting: BTreeSet::new(),
            refreshes: 0,
            drag: None,
            outside_drag_end: None,
            hires: None,
            hires_damage: Rect::default(),
        }
    }

    pub fn shared(width: i32, height: i32) -> Shared {
        Arc::new(Mutex::new(Desktop::new(width, height)))
    }

    pub fn size(&self) -> (i32, i32) {
        (self.screen.width, self.screen.height)
    }

    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Another to wake as well: a second server of the same kernel, each taking its own clients' events
    /// (`take_events_where`).
    pub fn add_waker(&mut self, waker: Waker) {
        self.wakers.push(waker);
    }

    /// What wakes its kernel (each of its servers) when there are events to read.
    pub fn waker(&self) -> Option<Waker> {
        if self.wakers.is_empty() {
            return self.waker.clone();
        }
        let all: Vec<Waker> = self.waker.iter().chain(&self.wakers).cloned().collect();
        Some(Arc::new(move || {
            for w in &all {
                w();
            }
        }))
    }

    /// What starts the clock when a window waits for a frame. It's called with the desktop's lock held, so it may only
    /// signal (a condition variable, a host's display link).
    pub fn set_clock(&mut self, clock: Waker) {
        self.clock = Some(clock);
    }

    // ---- Windows, as their kernel asks ----

    /// A new window, hidden until it's shown.
    pub fn create(&mut self, spec: Spec) -> WindowId {
        let id = self.next;
        self.next += 1;
        let (parent, popup) = match spec.role {
            Role::Toplevel => (None, None),
            Role::Dialog { parent } => (Some(parent), None),
            Role::Popup { parent, positioner, grab } => (Some(parent), Some((positioner, grab))),
        };
        let owner = parent.filter(|o| self.windows.contains_key(o));
        let (rect, framed, layer) = match &popup {
            Some((positioner, _)) => (self.popup_rect(owner, positioner), false, Layer::Popup),
            None => (spec.rect, spec.framed, spec.layer.min(Layer::Top)),
        };
        let window = Window {
            rect,
            title: spec.title,
            client: spec.client,
            no_activate: spec.no_activate,
            attention: false,
            constraint: None,
            relative: false,
            framed,
            layer,
            owner,
            popup,
            states: 0,
            restore: None,
            min_size: (0, 0),
            max_size: (0, 0),
            acked: 0,
            cursor: Cursor::default(),
            text_input: None,
            input: None,
            visible: false,
            fill: spec.fill,
            buffer: None,
            buffer_scale: 1,
            offset: None,
            pending: Pending::default(),
        };
        self.windows.insert(id, window);
        self.order.push(id);
        self.restack();
        self.observers.note(Note::Window(id));
        id
    }

    pub fn destroy(&mut self, id: WindowId) {
        self.dismiss_popups_of(id);
        self.drag_window_gone(id);
        let Some(w) = self.windows.remove(&id) else { return };
        self.observers.note(Note::Gone(id));
        self.observers.note(Note::Stacking);
        self.selection_owner_gone(id);
        if self.capture == Some(id) {
            self.capture = None;
        }
        if self.hover == Some(id) {
            self.hover = None;
        }
        self.damage_rect(w.extent());
        self.order.retain(|&o| o != id);
        for other in self.windows.values_mut() {
            if other.owner == Some(id) {
                other.owner = None;
            }
        }
        if matches!(self.grab, Some(Grab::Content(g) | Grab::Close(g) | Grab::Maximize(g) | Grab::Move { window: g, .. } | Grab::Resize { window: g, .. }) if g == id) {
            self.grab = None;
        }
        self.events.retain(|e| event_window(e) != id);
        self.frames.remove(&id);
        self.presenting.remove(&id);
        if self.constrained == Some(id) {
            self.constrained = None;
        }
        if self.active == Some(id) {
            self.active = None;
            self.activate_next(w.owner);
        }
        self.update_constraint();
    }

    pub fn exists(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    pub fn rect(&self, id: WindowId) -> Option<Rect> {
        self.windows.get(&id).map(|w| w.rect)
    }

    /// All a window covers on the screen, its own shadow too: what's composited again when it changes.
    fn extent_of(&self, id: WindowId) -> Option<Rect> {
        self.windows.get(&id).map(Window::extent)
    }

    /// Where its content is on the screen.
    pub fn content_rect(&self, id: WindowId) -> Option<Rect> {
        self.windows.get(&id).map(Window::content_rect)
    }

    /// Moves or resizes it (frame and all). Its buffer stays at its content's top left; content it doesn't cover
    /// shows its fill until its kernel commits one of the new size.
    pub fn set_rect(&mut self, id: WindowId, rect: Rect) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        if w.rect != rect {
            self.observers.note(Note::Window(id));
        }
        let Some(w) = self.windows.get_mut(&id) else { return };
        let (old, before) = (w.rect, w.extent());
        w.rect = rect;
        let after = w.extent();
        // Its popups move with it.
        let (dx, dy) = (rect.left - old.left, rect.top - old.top);
        if (dx, dy) != (0, 0) {
            let popups: Vec<(WindowId, Rect)> = self.windows.iter().filter(|(_, p)| p.owner == Some(id) && p.popup.is_some()).map(|(&p, w)| (p, w.rect)).collect();
            for (p, r) in popups {
                self.set_rect(p, r.offset(dx, dy));
            }
        }
        let w = &self.windows[&id];
        if w.visible {
            self.damage_rect(before);
            self.damage_rect(after);
        }
        if self.constrained == Some(id) {
            self.confine_pointer();
        }
    }

    pub fn set_title(&mut self, id: WindowId, title: &str) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        w.title = title.to_string();
        self.observers.note(Note::Window(id));
        let Some(w) = self.windows.get_mut(&id) else { return };
        let caption = w.caption();
        if w.on_screen() && w.has_frame() {
            self.damage_rect(caption);
        }
    }

    pub fn set_fill(&mut self, id: WindowId, fill: Rgb) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.fill = fill;
            let r = w.content_rect();
            self.observers.note(Note::Damage(id, Rect::new(0, 0, r.width(), r.height())));
            let w = &self.windows[&id];
            if w.visible {
                self.damage_rect(r);
            }
        }
    }

    pub fn set_layer(&mut self, id: WindowId, layer: Layer) {
        if let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none()) {
            w.layer = layer.min(Layer::Top);
            let r = w.extent();
            self.restack();
            self.damage_rect(r);
        }
    }

    /// Shows or hides it. A hidden active window passes being active on.
    pub fn set_visible(&mut self, id: WindowId, visible: bool) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        if w.visible == visible {
            return;
        }
        w.visible = visible;
        let (r, owner) = (w.extent(), w.owner);
        self.damage_rect(r);
        self.observers.note(Note::Window(id));
        if !visible {
            self.dismiss_popups_of(id);
            if self.capture == Some(id) {
                self.capture = None;
            }
        }
        self.update_text_input();
        if !visible && self.active == Some(id) {
            self.set_active(None);
            self.activate_next(owner);
        }
        self.update_constraint();
        self.update_cursor();
    }

    pub fn is_visible(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|w| w.visible)
    }

    /// Puts it on top of its layer, with the windows it owns above it.
    pub fn raise(&mut self, id: WindowId) {
        if !self.windows.contains_key(&id) {
            return;
        }
        let family = self.owned_by(id);
        self.order.retain(|o| !family.contains(o));
        self.order.extend(family);
        self.restack();
        if let Some(r) = self.extent_of(id) {
            self.damage_rect(r);
        }
    }

    /// Puts it (with the windows it owns) at the top or bottom of its band, or just above or below another; the
    /// stacking rules then hold as ever (`restack`).
    pub fn stack(&mut self, id: WindowId, how: Stack) {
        if !self.windows.contains_key(&id) {
            return;
        }
        let family = self.owned_by(id);
        if let Stack::Above(s) | Stack::Below(s) = how
            && (family.contains(&s) || !self.windows.contains_key(&s))
        {
            return;
        }
        let rest: Vec<WindowId> = self.order.iter().copied().filter(|o| !family.contains(o)).collect();
        let at = match how {
            Stack::Top => rest.len(),
            Stack::Bottom => 0,
            Stack::Above(s) => rest.iter().position(|&o| o == s).map_or(rest.len(), |p| p + 1),
            Stack::Below(s) => rest.iter().position(|&o| o == s).unwrap_or(0),
        };
        let mut order = rest;
        order.splice(at..at, family.iter().copied());
        self.order = order;
        self.restack();
        for f in family {
            if let Some(r) = self.extent_of(f) {
                self.damage_rect(r);
            }
        }
    }

    /// Whether a click neither activates nor raises it.
    pub fn set_no_activate(&mut self, id: WindowId, on: bool) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.no_activate = on;
        }
    }

    /// Raises it and makes it the active window, if it's shown. A popup is never the active window.
    pub fn activate(&mut self, id: WindowId) {
        if self.windows.get(&id).is_some_and(|w| w.on_screen() && w.popup.is_none()) {
            self.set_active(Some(id));
        }
    }

    pub fn active(&self) -> Option<WindowId> {
        self.active
    }

    // ---- Focus policy ----

    /// Whose a window is.
    pub fn client(&self, id: WindowId) -> Option<ClientId> {
        self.windows.get(&id).map(|w| w.client)
    }

    /// A client (or with None, something outside the machine: the host, a launcher, `carbon display`) asks for a window
    /// to be made active: `SetForegroundWindow`, `xdg_activation_v1.activate`, `_NET_ACTIVE_WINDOW`. Granted if
    /// nothing's active, the active window is `by`'s, `by` had the user's last press or key, or with a token issued
    /// since the user's last one. Refused, it's marked as wanting attention instead (a taskbar button flashes, a dock
    /// icon bounces), and it doesn't take the focus from what the user's doing. Whether it's active now.
    pub fn request_activation(&mut self, id: WindowId, by: Option<ClientId>, token: Option<Token>) -> bool {
        if !self.windows.get(&id).is_some_and(|w| w.on_screen() && w.popup.is_none()) {
            return false;
        }
        if self.active == Some(id) {
            self.raise(id);
            return true;
        }
        let token = token.and_then(|t| self.tokens.remove(&t)).is_some_and(|serial| serial == self.input_serial);
        let active_client = self.active.and_then(|a| self.client(a));
        let granted = match by {
            None => true,
            Some(c) => self.active.is_none() || active_client == Some(c) || self.last_input == Some(c) || token,
        };
        if granted {
            self.set_active(Some(id));
        } else {
            self.set_attention(id, true);
        }
        granted
    }

    /// A token for `by` to pass on, to a program it launches or another client it hands the focus to, if `by` may
    /// move the focus now itself (as `request_activation` decides), or for something outside the machine (None). Good
    /// for one activation, until the user's next press or key.
    pub fn activation_token(&mut self, by: Option<ClientId>) -> Option<Token> {
        if let Some(c) = by {
            let active_client = self.active.and_then(|a| self.client(a));
            if !(self.active.is_none() || active_client == Some(c) || self.last_input == Some(c)) {
                return None;
            }
        }
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        self.tokens.insert(token, self.input_serial);
        // Tokens never used: only the latest few are kept.
        while self.tokens.len() > 64 {
            self.tokens.pop_first();
        }
        Some(token)
    }

    /// Marks a window as wanting the user's attention, or not: Windows' `FlashWindow`, X11's urgency hint and
    /// `_NET_WM_STATE_DEMANDS_ATTENTION`, an activation refused. It's cleared when it's active.
    pub fn set_attention(&mut self, id: WindowId, on: bool) {
        let on = on && self.active != Some(id);
        if let Some(w) = self.windows.get_mut(&id)
            && w.attention != on
        {
            w.attention = on;
            self.observers.note(Note::Window(id));
        }
    }

    pub fn attention(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|w| w.attention)
    }

    /// The user pressed a button or a key, on a window of `client`'s (or on none): the evidence `request_activation`
    /// goes by. Tokens issued before it are spent.
    fn user_input(&mut self, to: Option<WindowId>) {
        self.input_serial = self.input_serial.wrapping_add(1);
        self.last_input = to.and_then(|id| self.client(id));
    }

    // ---- Key repeat ----

    pub fn key_repeat(&self) -> KeyRepeat {
        self.repeat
    }

    /// How fast held keys repeat, as the host or the user has it.
    pub fn set_key_repeat(&mut self, repeat: KeyRepeat) {
        self.repeat = repeat;
    }

    // ---- Pointer constraints and relative motion ----

    /// What a window asks of the pointer while it's active (None: nothing). Taken now if it can be; `Constrained`
    /// says when it's taken and let go.
    pub fn set_pointer_constraint(&mut self, id: WindowId, constraint: Option<Constraint>) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        w.constraint = constraint;
        if self.constrained == Some(id) {
            // Changed while taken: let go, and taken again as it is now.
            self.constrained = None;
            self.events.push_back(Event::Constrained { window: id, on: false });
        }
        self.update_constraint();
        self.update_cursor();
    }

    pub fn pointer_constraint(&self, id: WindowId) -> Option<Constraint> {
        self.windows.get(&id).and_then(|w| w.constraint)
    }

    /// The window whose constraint is taken now, and what it is.
    pub fn constrained(&self) -> Option<(WindowId, Constraint)> {
        let id = self.constrained?;
        Some((id, self.windows.get(&id)?.constraint?))
    }

    /// Whether a window gets the pointer's relative motion (`Pointer::Relative`) as well as where it is.
    pub fn set_relative_motion(&mut self, id: WindowId, on: bool) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.relative = on;
        }
    }

    /// Takes the active window's constraint if it can be, letting go of one that can't be any more.
    fn update_constraint(&mut self) {
        let (x, y) = self.pointer;
        let want = self.active.filter(|id| {
            self.windows.get(id).is_some_and(|w| {
                w.on_screen()
                    && match w.constraint {
                        Some(Constraint::Lock) => w.content_rect().contains(x, y),
                        Some(Constraint::Confine(_)) => true,
                        None => false,
                    }
            })
        });
        if want == self.constrained {
            return;
        }
        if let Some(old) = self.constrained.take() {
            self.events.push_back(Event::Constrained { window: old, on: false });
        }
        if let Some(new) = want {
            self.constrained = Some(new);
            self.events.push_back(Event::Constrained { window: new, on: true });
            self.confine_pointer();
        }
        // A lock hides the cursor.
        self.update_cursor();
    }

    /// The rectangle on the screen a taken confinement keeps the pointer in.
    fn confinement(&self) -> Option<Rect> {
        let w = self.windows.get(&self.constrained?)?;
        let Some(Constraint::Confine(r)) = w.constraint else { return None };
        let c = w.content_rect();
        let r = if r.is_empty() { c } else { r.offset(c.left, c.top) };
        Some(r.intersect(&self.screen.bounds())).filter(|r| !r.is_empty())
    }

    /// Moves the pointer inside a taken confinement, if it's outside.
    fn confine_pointer(&mut self) {
        let Some(r) = self.confinement() else { return };
        let (x, y) = self.pointer;
        let to = (x.clamp(r.left, r.right - 1), y.clamp(r.top, r.bottom - 1));
        if to != self.pointer {
            self.pointer_to(to.0, to.1);
        }
    }

    fn locked(&self) -> Option<WindowId> {
        self.constrained().and_then(|(id, c)| (c == Constraint::Lock).then_some(id))
    }

    /// The pointer moved: to `to` (clamped to the screen, and to a confinement), by `raw` as the device saw it. A
    /// lock keeps it where it is, and only its relative motion goes to the window.
    fn motion(&mut self, to: (i32, i32), raw: (i32, i32)) {
        let target = match self.grab {
            Some(Grab::Content(id)) => Some(id),
            Some(_) => None,
            None => self.locked().or_else(|| self.pointer_target()),
        };
        if raw != (0, 0)
            && let Some(id) = target.filter(|id| self.windows.get(id).is_some_and(|w| w.relative))
        {
            self.pointer_event(id, Pointer::Relative { dx: raw.0, dy: raw.1 });
        }
        if self.locked().is_some() {
            return;
        }
        let (mut x, mut y) = to;
        if let Some(r) = self.confinement() {
            (x, y) = (x.clamp(r.left, r.right - 1), y.clamp(r.top, r.bottom - 1));
        }
        self.pointer_to(x, y);
        // A lock waiting for the pointer to come into its window.
        self.update_constraint();
    }

    // ---- Presentation ----

    /// Asks for a `Presented` event for the next commit: when it reached the screen, or that it never will.
    pub fn request_presented(&mut self, id: WindowId) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.pending.presented = true;
        }
    }

    /// Bottom to top.
    pub fn stacking(&self) -> &[WindowId] {
        &self.order
    }

    // ---- The clipboard and the primary selection ----

    /// Offers a selection, as `mimes`, from a window (or from outside the machine, with None), taking it from whoever
    /// had it. Data for a type can come with `offer_data`; a paste of one without is asked of the owner's kernel.
    pub fn offer(&mut self, which: Selection, owner: Option<WindowId>, mimes: Vec<String>) -> SourceId {
        let id = self.selections.next_id();
        let source = Source { id, owner: owner.filter(|o| self.windows.contains_key(o)), mimes, data: BTreeMap::new() };
        if let Some(old) = self.selections.sources.insert(which, source)
            && let Some(window) = old.owner
        {
            self.events.push_back(Event::SelectionCancelled { window, selection: which });
        }
        self.observers.note(Note::Selection(which));
        id
    }

    /// Gives the data for one type of an offer, at once (offering that type too, if it wasn't). False if the offer's
    /// been replaced, or the data's too big.
    pub fn offer_data(&mut self, source: SourceId, mime: &str, data: Vec<u8>) -> bool {
        let Some((&which, s)) = self.selections.sources.iter_mut().find(|(_, s)| s.id == source) else { return false };
        if data.len() > MOST {
            return false;
        }
        if !s.mimes.iter().any(|m| m == mime) {
            s.mimes.push(mime.to_string());
        }
        s.data.insert(mime.to_string(), data);
        self.observers.note(Note::Selection(which));
        true
    }

    /// Takes a selection away: nothing's offered.
    pub fn clear_selection(&mut self, which: Selection) {
        if let Some(old) = self.selections.sources.remove(&which) {
            if let Some(window) = old.owner {
                self.events.push_back(Event::SelectionCancelled { window, selection: which });
            }
            self.observers.note(Note::Selection(which));
        }
    }

    /// The offer a selection is now, to tell one's own from another's (a host's clipboard following it doesn't copy back
    /// what it offered itself).
    pub fn selection_source(&self, which: Selection) -> Option<SourceId> {
        self.selections.sources.get(&which).map(|s| s.id)
    }

    /// Who offers a selection now (None for outside the machine), and as what; None if nothing's offered.
    pub fn selection(&self, which: Selection) -> Option<(Option<WindowId>, Vec<String>)> {
        self.selections.sources.get(&which).map(|s| (s.owner, s.mimes.clone()))
    }

    /// Asks for a selection's data as `mime`, for a window (or for outside the machine, with None): the answer comes
    /// as `SelectionData` to the window, or from `take_transfer`. None if it isn't offered as that.
    pub fn request(&mut self, which: Selection, mime: &str, requester: Option<WindowId>) -> Option<TransferId> {
        let s = self.selections.sources.get(&which).filter(|s| s.mimes.iter().any(|m| m == mime))?;
        let (given, owner) = (s.data.get(mime).cloned(), s.owner);
        let id = self.selections.next_id();
        self.selections.transfers.insert(id, Transfer { requester, owner: None, result: None });
        match (given, owner) {
            (Some(data), _) => self.provide(id, Some(data)),
            (None, Some(window)) => {
                if let Some(t) = self.selections.transfers.get_mut(&id) {
                    t.owner = Some(window);
                }
                self.events.push_back(Event::SendSelection { window, selection: which, mime: mime.to_string(), transfer: id });
            }
            (None, None) => self.provide(id, None),
        }
        Some(id)
    }

    /// A kernel's answer to `SendSelection`: the data (None if it has none), passed to whoever asked.
    pub fn provide(&mut self, transfer: TransferId, data: Option<Vec<u8>>) {
        let data = data.filter(|d| d.len() <= MOST);
        let Some(t) = self.selections.transfers.get_mut(&transfer) else { return };
        match t.requester {
            Some(window) => {
                self.selections.transfers.remove(&transfer);
                if self.windows.contains_key(&window) {
                    self.events.push_back(Event::SelectionData { window, transfer, data });
                }
            }
            None => t.result = Some(data),
        }
    }

    /// For a requester outside the machine: the answer, once there is one (and then it's forgotten).
    pub fn take_transfer(&mut self, transfer: TransferId) -> Option<Option<Vec<u8>>> {
        let result = self.selections.transfers.get_mut(&transfer)?.result.take()?;
        self.selections.transfers.remove(&transfer);
        Some(result)
    }

    /// A window's gone: what it offered stays if all of it was given at once (as Windows' clipboard outlives its
    /// owner), else it's taken away; pastes waiting on it get nothing.
    fn selection_owner_gone(&mut self, id: WindowId) {
        let owned: Vec<Selection> = self.selections.sources.iter().filter(|(_, s)| s.owner == Some(id)).map(|(w, _)| *w).collect();
        for which in owned {
            let s = self.selections.sources.get_mut(&which).expect("just found");
            if s.mimes.iter().all(|m| s.data.contains_key(m)) {
                s.owner = None;
            } else {
                self.selections.sources.remove(&which);
            }
            self.observers.note(Note::Selection(which));
        }
        let waiting: Vec<TransferId> = self.selections.transfers.iter().filter(|(_, t)| t.owner == Some(id)).map(|(&t, _)| t).collect();
        for t in waiting {
            self.provide(t, None);
        }
        self.selections.transfers.retain(|_, t| t.requester != Some(id));
    }

    // ---- Drag and drop ----

    /// Starts a drag of `mimes`, which may be done `actions` (`action::*`) with, from a window whose content a button's
    /// held down on (`wl_data_device.start_drag` with its implicit grab, `DoDragDrop`, X11's `XdndSelection` owner on a
    /// press), or from outside the machine (None: the host's pointer bringing it in, `drop_from_outside`). It's offered
    /// as `Selection::Drag`, with data given at once (`offer_data`) or sent when a target asks. The pointer carries it
    /// over windows until the button's up (or `drop_drag`), and Escape cancels it. None if there's a drag already, or
    /// the window has no button held on it.
    pub fn start_drag(&mut self, source: Option<WindowId>, mimes: Vec<String>, actions: u8) -> Option<SourceId> {
        if self.drag.is_some() || actions == 0 {
            return None;
        }
        if let Some(w) = source
            && (self.buttons == 0 || self.grab != Some(Grab::Content(w)))
        {
            return None;
        }
        let id = self.offer(Selection::Drag, source, mimes.clone());
        if source.is_none() {
            self.outside_drag_end = None;
        }
        let info = DragInfo { source, mimes, actions, target: None, mime: None, action: 0, dropped: false };
        self.drag = Some(Drag { info, accepts: 0, prefers: 0 });
        self.grab = Some(Grab::Drag);
        // The pointer's the drag's now: the window it was in hears it leave.
        if let Some(h) = self.hover.take().filter(|h| self.windows.contains_key(h)) {
            self.pointer_event(h, Pointer::Leave);
        }
        self.drag_over();
        Some(id)
    }

    /// The drag going on, as it is now.
    pub fn drag(&self) -> Option<DragInfo> {
        self.drag.as_ref().map(|d| d.info.clone())
    }

    /// The drag's target says what it would take it as (None: nothing it carries), what it may do with it, and which
    /// of those it prefers: Wayland's `wl_data_offer.accept` and `set_actions`, `XdndStatus`, `IDropTarget`'s effect.
    /// The desktop chooses the action (the preferred one if the source allows it, else the first both allow), and
    /// the source hears (`DragAction`).
    pub fn accept_drag(&mut self, window: WindowId, mime: Option<&str>, accepts: u8, prefers: u8) {
        let Some(d) = self.drag.as_mut().filter(|d| d.info.target == Some(window) && !d.info.dropped) else { return };
        let mime = mime.filter(|m| d.info.mimes.iter().any(|x| x == m)).map(String::from);
        d.accepts = accepts;
        d.prefers = prefers;
        let common = d.info.actions & accepts;
        let pick = if common & prefers != 0 { common & prefers } else { common };
        let action = if mime.is_some() { pick.isolate_lowest_one() } else { 0 };
        if (&mime, action) != (&d.info.mime, d.info.action) {
            d.info.mime = mime.clone();
            d.info.action = action;
            if let Some(source) = d.info.source {
                self.events.push_back(Event::DragAction { window: source, mime, action });
            }
        }
        self.update_cursor();
    }

    /// Drops the drag where it is: on a window that takes it, which hears `Drop` and reads it, the source hearing
    /// `Dropped`; anywhere else, it's cancelled. A button coming up does this.
    pub fn drop_drag(&mut self) {
        let Some(d) = self.drag.as_mut().filter(|d| !d.info.dropped) else { return };
        if self.grab == Some(Grab::Drag) {
            self.grab = None;
        }
        let (target, action, source) = (d.info.target, d.info.action, d.info.source);
        match target.filter(|_| action != 0) {
            Some(t) => {
                d.info.dropped = true;
                let c = self.content_rect(t).unwrap_or_default();
                let (x, y) = (self.pointer.0 - c.left, self.pointer.1 - c.top);
                self.events.push_back(Event::Drop { window: t, x, y, action });
                if let Some(s) = source {
                    self.events.push_back(Event::Dropped { window: s });
                }
                self.update_hover();
                self.update_cursor();
            }
            None => self.cancel_drag(),
        }
    }

    /// Ends the drag with nothing done: its target hears it leave (if it wasn't dropped on it yet), and its source
    /// that it ended (`DragEnd` with 0).
    pub fn cancel_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };
        if self.grab == Some(Grab::Drag) {
            self.grab = None;
        }
        if let Some(t) = d.info.target.filter(|_| !d.info.dropped) {
            self.events.push_back(Event::DragLeave { window: t });
        }
        self.end_drag(d.info.source, 0);
    }

    /// The target of a drop says it's done reading it: Wayland's `wl_data_offer.finish`, `XdndFinished`, `Drop`
    /// returning. The source hears what was done (`DragEnd`).
    pub fn finish_drop(&mut self, window: WindowId) {
        if !self.drag.as_ref().is_some_and(|d| d.info.dropped && d.info.target == Some(window)) {
            return;
        }
        let d = self.drag.take().expect("just checked");
        self.end_drag(d.info.source, d.info.action);
    }

    /// What the last drag from outside the machine ended with, once (an `action::*`, or 0).
    pub fn take_outside_drag_end(&mut self) -> Option<u8> {
        self.outside_drag_end.take()
    }

    fn end_drag(&mut self, source: Option<WindowId>, action: u8) {
        if self.selections.sources.remove(&Selection::Drag).is_some() {
            self.observers.note(Note::Selection(Selection::Drag));
        }
        match source {
            Some(window) => self.events.push_back(Event::DragEnd { window, action }),
            None => self.outside_drag_end = Some(action),
        }
        self.update_hover();
        self.update_cursor();
    }

    /// The pointer moved with a drag: the window whose content it's over hears it come, move, or go.
    fn drag_over(&mut self) {
        let Some(d) = self.drag.as_ref().filter(|d| !d.info.dropped) else { return };
        let (x, y) = self.pointer;
        let target = self.window_at(x, y).filter(|id| self.windows[id].part(x, y) == Some(Part::Content));
        let (old, source) = (d.info.target, d.info.source);
        let at = |d: &Desktop, t: WindowId| {
            let c = d.content_rect(t).unwrap_or_default();
            (x - c.left, y - c.top)
        };
        if target != old {
            if let Some(o) = old {
                self.events.push_back(Event::DragLeave { window: o });
            }
            let d = self.drag.as_mut().expect("just found");
            d.info.target = target;
            (d.accepts, d.prefers) = (0, 0);
            let had = d.info.mime.take().is_some() || d.info.action != 0;
            d.info.action = 0;
            let (mimes, actions) = (d.info.mimes.clone(), d.info.actions);
            if had && let Some(s) = source {
                self.events.push_back(Event::DragAction { window: s, mime: None, action: 0 });
            }
            if let Some(t) = target {
                let (cx, cy) = at(self, t);
                self.events.push_back(Event::DragEnter { window: t, x: cx, y: cy, mimes, actions });
            }
        } else if let Some(t) = target {
            let (cx, cy) = at(self, t);
            if let Some(Event::DragMotion { window, .. }) = self.events.back()
                && *window == t
            {
                self.events.pop_back();
            }
            self.events.push_back(Event::DragMotion { window: t, x: cx, y: cy });
        }
        self.update_cursor();
    }

    /// A window's going: a drag from it is cancelled, and one over it (or dropped on it) has no target any more.
    fn drag_window_gone(&mut self, id: WindowId) {
        let Some(d) = self.drag.as_mut() else { return };
        if d.info.source == Some(id) {
            self.cancel_drag();
        } else if d.info.target == Some(id) {
            if d.info.dropped {
                let source = d.info.source;
                self.drag = None;
                self.end_drag(source, 0);
            } else {
                d.info.target = None;
                d.info.mime = None;
                d.info.action = 0;
                if let Some(s) = d.info.source {
                    self.events.push_back(Event::DragAction { window: s, mime: None, action: 0 });
                }
            }
        }
    }

    // ---- The output ----

    /// What the desktop's shown on.
    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Shows it on another output, or the same one changed: the screen takes its size, maximized and fullscreen
    /// windows are made its size again, and each window hears of a new scale (`Scale`).
    pub fn set_output(&mut self, output: Output) {
        let old = std::mem::replace(&mut self.output, output.clone());
        if (old.width, old.height) != (output.width, output.height) {
            self.screen.resize(output.width, output.height, BACKGROUND);
            let bounds = self.screen.bounds();
            let placed: Vec<WindowId> = self.order.iter().copied().filter(|id| self.windows[id].placed_by_desktop()).collect();
            for id in placed {
                self.set_rect(id, bounds);
                self.configure(id);
            }
            self.pointer = (self.pointer.0.min(output.width - 1), self.pointer.1.min(output.height - 1));
        }
        self.damage_rect(self.screen.bounds());
        if old.scale != output.scale {
            for &window in &self.order {
                self.events.push_back(Event::Scale { window, scale: output.scale });
            }
        }
        self.observers.note(Note::Output);
    }

    // ---- Observers ----

    /// A new observer of the desktop, whose first take is all of it.
    pub fn observe(&mut self) -> ObserverId {
        let all: Vec<(WindowId, Rect)> = self.order.iter().map(|&id| (id, self.windows[&id].content_rect())).collect();
        let screen = self.screen.bounds();
        self.observers.add(all.into_iter(), screen)
    }

    pub fn unobserve(&mut self, id: ObserverId) {
        self.observers.remove(id);
    }

    /// What wakes an observer when it has something to take, after it had nothing. It's called with the desktop's
    /// lock held, so it may only signal.
    pub fn set_observer_waker(&mut self, id: ObserverId, waker: Waker) {
        self.observers.set_waker(id, waker);
    }

    /// What's changed since this observer last took, as it is now.
    pub fn take_observed(&mut self, id: ObserverId) -> Observed {
        let Some(taken) = self.observers.take(id) else { return Observed::default() };
        Observed {
            windows: taken.changed.into_iter().filter_map(|w| self.window_info(w)).collect(),
            gone: taken.gone,
            stacking: taken.stacking.then(|| self.order.clone()),
            active: taken.active.then_some(self.active),
            damage: taken.damage.into_iter().filter(|(w, _)| self.windows.contains_key(w)).collect(),
            cursor: taken.cursor.then(|| self.cursor.clone()),
            text_input: taken.text_input.then_some(self.text_input),
            output: taken.output.then(|| self.output.clone()),
            selections: taken.selections,
            screen: taken.screen.intersect(&self.screen.bounds()),
        }
    }

    /// What a window is now.
    pub fn window_info(&self, id: WindowId) -> Option<WindowInfo> {
        let w = self.windows.get(&id)?;
        Some(WindowInfo {
            id,
            rect: w.rect,
            content: w.content_rect(),
            title: w.title.clone(),
            framed: w.has_frame(),
            layer: w.layer,
            parent: w.owner,
            popup: w.popup.is_some(),
            on_screen: w.on_screen(),
            states: w.states,
            attention: w.attention,
            client: w.client,
        })
    }

    /// Every window, bottom to top.
    pub fn windows(&self) -> Vec<WindowInfo> {
        self.order.iter().filter_map(|&id| self.window_info(id)).collect()
    }

    /// Part of a window's content as it shows now (its committed buffer, its fill where that doesn't reach), in its
    /// content's coordinates: what an observer sends or draws.
    pub fn read(&self, id: WindowId, rect: Rect) -> Option<Surface> {
        let w = self.windows.get(&id)?;
        let c = w.content_rect();
        let r = rect.intersect(&Rect::new(0, 0, c.width(), c.height()));
        let mut out = Surface::new(r.width(), r.height(), w.fill);
        if let Some(b) = &w.buffer {
            let all = out.bounds();
            let (ox, oy) = w.offset.unwrap_or((0, 0));
            show(&mut out, b, w.buffer_scale, -r.left - ox, -r.top - oy, &all);
        }
        Some(out)
    }

    /// A window's whole content in the output's own pixels: its fill, and its buffer over it, pixel for pixel if it
    /// was drawn at the output's scale, else magnified (or shrunk). What a host shows in a window of its own.
    pub fn read_pixels(&self, id: WindowId) -> Option<Surface> {
        let w = self.windows.get(&id)?;
        let c = w.content_rect();
        let o = &self.output;
        let (pw, ph) = (o.to_pixels(c.width()).max(1), o.to_pixels(c.height()).max(1));
        let mut out = Surface::new(pw, ph, w.fill);
        let Some(b) = &w.buffer else { return Some(out) };
        // Buffer pixels a screen pixel: the buffer's scale over the output's.
        let (num, den) = (i64::from(w.buffer_scale.max(1)) * i64::from(SCALE_1X), i64::from(o.scale.max(1)));
        // From its content's top left: past its offset (in the buffer's pixels), its shadow left out.
        let s = w.buffer_scale.max(1) as i32;
        let (ox, oy) = w.offset.map_or((0, 0), |(x, y)| (x * s, y * s));
        let (size_w, size_h) = b.pixels.size();
        let (bw, bh) = (size_w - ox, size_h - oy);
        if bw <= 0 || bh <= 0 {
            return Some(out);
        }
        let (ew, eh) = (((i64::from(bw) * den / num) as i32).min(pw), ((i64::from(bh) * den / num) as i32).min(ph));
        if ew <= 0 || eh <= 0 {
            return Some(out);
        }
        let last = ((i64::from(ew - 1) * num / den) as i32).min(bw - 1);
        let mut row = vec![0u32; (last + 1) as usize];
        let opaque = b.pixels.format() == Format::Xrgb8888;
        for y in 0..eh {
            let by = ((i64::from(y) * num / den) as i32).min(bh - 1);
            b.pixels.read_row(ox, by + oy, &mut row);
            let at = (y * pw) as usize;
            for (x, d) in out.pixels[at..at + ew as usize].iter_mut().enumerate() {
                let s = row[((x as i64 * num / den) as usize).min(row.len() - 1)];
                *d = if opaque { s & 0x00ff_ffff } else { over(s, *d) };
            }
        }
        Some(out)
    }

    /// The host's close button for it was clicked: its kernel's asked to close it (`Close`), as the desktop's own
    /// close button asks.
    pub fn request_close(&mut self, id: WindowId) {
        if self.windows.contains_key(&id) {
            self.events.push_back(Event::Close { window: id });
        }
    }

    /// The host's window manager placed it (the user moved or resized the host's window for it): moved there, frame
    /// and all, and its kernel told, as when the desktop's own frame is dragged.
    pub fn placed(&mut self, id: WindowId, rect: Rect) {
        if self.windows.get(&id).is_some_and(|w| w.rect != rect) {
            self.user_placed(id, rect);
        }
    }

    // ---- Frames, and moving and resizing for a window that draws its own ----

    /// Gives it the desktop's frame, or takes it away, keeping its content where it is (Wayland's decoration mode,
    /// X11's `_MOTIF_WM_HINTS`: the frame goes around the client's surface) or its rectangle (a Windows style
    /// change: the client area is what's left inside). Told with `Configure`.
    pub fn set_framed(&mut self, id: WindowId, framed: bool, keep: Keep) {
        let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none() && w.framed != framed) else { return };
        let (old, content) = (w.extent(), w.content_rect());
        w.framed = framed;
        if keep == Keep::Content {
            w.rect = Insets::of(w.has_frame()).around(&content);
        }
        let rect = w.extent();
        if w.on_screen() {
            self.damage_rect(old);
            self.damage_rect(rect);
        }
        self.configure(id);
    }

    /// Starts moving it by the pointer, as its title bar would, for a window that draws its own: Wayland's
    /// `xdg_toplevel.move`, X11's `_NET_WM_MOVERESIZE`, Windows' `HTCAPTION` and `SC_MOVE`. Only while a button that
    /// went down on its content is still held; false otherwise (a request that came too late does nothing).
    pub fn begin_move(&mut self, id: WindowId) -> bool {
        let Some(w) = self.windows.get(&id).filter(|w| self.grab == Some(Grab::Content(id)) && !w.placed_by_desktop()) else { return false };
        let (x, y) = self.pointer;
        self.grab = Some(Grab::Move { window: id, dx: x - w.rect.left, dy: y - w.rect.top });
        true
    }

    /// Starts resizing it from `edges` (`edge::*`) by the pointer, as its frame's edges would; as `begin_move`.
    pub fn begin_resize(&mut self, id: WindowId, edges: u8) -> bool {
        let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none() && !w.placed_by_desktop()) else { return false };
        if self.grab != Some(Grab::Content(id)) || edges & (LEFT | TOP | RIGHT | BOTTOM) == 0 {
            return false;
        }
        w.states |= state::RESIZING;
        self.grab = Some(Grab::Resize { window: id, edges, start: w.rect, from: self.pointer });
        true
    }

    // ---- States and sizes ----

    /// `state::*`.
    pub fn states(&self, id: WindowId) -> u32 {
        self.windows.get(&id).map_or(0, |w| w.states)
    }

    /// Maximizes it to the screen, frame and all, or puts it back where it was. Told with `Configure`.
    pub fn set_maximized(&mut self, id: WindowId, on: bool) {
        let screen = self.screen.bounds();
        let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none()) else { return };
        let was = w.states;
        let rect = if on {
            w.states |= state::MAXIMIZED;
            if was & state::FULLSCREEN == 0 {
                w.restore.get_or_insert(w.rect);
            }
            if was & state::FULLSCREEN != 0 { w.rect } else { screen }
        } else {
            w.states &= !state::MAXIMIZED;
            if was & state::FULLSCREEN != 0 { w.rect } else { w.restore.take().unwrap_or(w.rect) }
        };
        if w.states != was {
            self.set_rect(id, rect);
            self.redraw(id);
            self.configure(id);
        }
    }

    /// Makes it the whole screen with no frame, above the windows that aren't, or puts it back: maximized again if
    /// it was, else where it was. Told with `Configure`.
    pub fn set_fullscreen(&mut self, id: WindowId, on: bool) {
        let screen = self.screen.bounds();
        let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none()) else { return };
        let was = w.states;
        let rect = if on {
            w.states |= state::FULLSCREEN;
            w.restore.get_or_insert(w.rect);
            screen
        } else {
            w.states &= !state::FULLSCREEN;
            if w.states & state::MAXIMIZED != 0 { screen } else { w.restore.take().unwrap_or(w.rect) }
        };
        if w.states != was {
            self.set_rect(id, rect);
            self.restack();
            self.redraw(id);
            self.configure(id);
        }
    }

    /// Minimizes it (kept, off the screen, its popups dismissed, and not the active window), or restores it.
    pub fn set_minimized(&mut self, id: WindowId, on: bool) {
        let Some(w) = self.windows.get_mut(&id).filter(|w| w.popup.is_none()) else { return };
        let was = w.states;
        if on {
            w.states |= state::MINIMIZED;
        } else {
            w.states &= !state::MINIMIZED;
        }
        if w.states == was {
            return;
        }
        let owner = w.owner;
        self.redraw(id);
        if on {
            self.dismiss_popups_of(id);
            if self.active == Some(id) {
                self.set_active(None);
                self.activate_next(owner);
            }
        }
        self.configure(id);
    }

    /// Its content's smallest and largest size (0 for no limit), which resizing it by its frame keeps to.
    /// Its content's smallest and largest size, 0 for no limit.
    pub fn size_limits(&self, id: WindowId) -> ((i32, i32), (i32, i32)) {
        self.windows.get(&id).map_or(((0, 0), (0, 0)), |w| (w.min_size, w.max_size))
    }

    pub fn set_size_limits(&mut self, id: WindowId, min: (i32, i32), max: (i32, i32)) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.min_size = (min.0.max(0), min.1.max(0));
            w.max_size = (max.0.max(0), max.1.max(0));
        }
    }

    /// Its kernel has dealt with the `Configure` with this serial.
    pub fn ack_configure(&mut self, id: WindowId, serial: u32) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.acked = serial;
        }
    }

    /// The last `Configure` its kernel acknowledged.
    pub fn acked(&self, id: WindowId) -> u32 {
        self.windows.get(&id).map_or(0, |w| w.acked)
    }

    /// A `Configure` for it is waiting to be taken: what its client commits meanwhile was drawn before it, so its
    /// kernel doesn't size the window by it.
    pub fn configuring(&self, id: WindowId) -> bool {
        self.events.iter().any(|e| matches!(e, Event::Configure { window, .. } if *window == id))
    }

    /// All of it is to be composited again (its frame, its state).
    fn redraw(&mut self, id: WindowId) {
        if let Some(r) = self.extent_of(id) {
            self.damage_rect(r);
        }
    }

    /// Makes a window another's dialog (transient for it: kept above it, active again when it goes), or with None, a
    /// toplevel of its own again: Wayland's `xdg_toplevel.set_parent` and X11's `WM_TRANSIENT_FOR` change it at any
    /// time. Refused for a popup, and for an owner that's it or one of its own.
    pub fn set_owner(&mut self, id: WindowId, owner: Option<WindowId>) {
        if !self.windows.get(&id).is_some_and(|w| w.popup.is_none()) {
            return;
        }
        let owner = owner.filter(|o| self.windows.contains_key(o) && !self.owned_by(id).contains(o));
        if let Some(w) = self.windows.get_mut(&id) {
            w.owner = owner;
            let r = w.extent();
            self.observers.note(Note::Window(id));
            self.restack();
            self.damage_rect(r);
        }
    }

    // ---- Popups ----

    pub fn is_popup(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|w| w.popup.is_some())
    }

    /// Places a popup again by a new positioner (a menu that grew, an anchor that moved), and tells its kernel where
    /// with `Configure`.
    pub fn set_positioner(&mut self, id: WindowId, positioner: Positioner) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        let Some((p, _)) = &mut w.popup else { return };
        *p = positioner;
        let owner = w.owner;
        let rect = self.popup_rect(owner, &positioner);
        self.set_rect(id, rect);
        self.configure(id);
    }

    /// The popup that has the keyboard and the pointer: the topmost shown popup that grabs.
    pub fn popup_grab(&self) -> Option<WindowId> {
        self.order.iter().rev().copied().find(|id| self.windows.get(id).is_some_and(|w| w.on_screen() && w.popup.is_some_and(|(_, grab)| grab)))
    }

    /// Where a popup goes for `positioner`, against its parent's content, kept on the screen.
    fn popup_rect(&self, parent: Option<WindowId>, positioner: &Positioner) -> Rect {
        let content = parent.and_then(|p| self.content_rect(p)).unwrap_or_default();
        positioner.place(content, self.screen.bounds())
    }

    /// A grabbing popup, and the popups it opened from: the windows its grab lets the pointer reach.
    fn grab_chain(&self, top: WindowId) -> Vec<WindowId> {
        let mut chain = vec![top];
        let mut at = top;
        while let Some(owner) = self.windows.get(&at).and_then(|w| w.owner).filter(|&o| self.is_popup(o)) {
            chain.push(owner);
            at = owner;
        }
        chain
    }

    /// Dismisses a popup and those opened from it: each hidden, and `PopupDone`, topmost first.
    fn dismiss(&mut self, id: WindowId) {
        let family: Vec<WindowId> = self.owned_by(id).into_iter().rev().filter(|&p| self.is_popup(p) && self.is_visible(p)).collect();
        for p in family {
            if let Some(w) = self.windows.get_mut(&p) {
                w.visible = false;
                let r = w.extent();
                self.damage_rect(r);
            }
            self.observers.note(Note::Window(p));
            self.events.push_back(Event::PopupDone { window: p });
        }
    }

    /// Dismisses the popups opened from a window that's being hidden or destroyed.
    fn dismiss_popups_of(&mut self, id: WindowId) {
        let popups: Vec<WindowId> = self.windows.iter().filter(|(_, w)| w.owner == Some(id) && w.popup.is_some()).map(|(&p, _)| p).collect();
        for p in popups {
            self.dismiss(p);
        }
    }

    // ---- Content: buffers, commits and frames ----

    /// Attaches a buffer (or with None, takes it away) at the next commit.
    pub fn attach(&mut self, id: WindowId, buffer: Option<Buffer>) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.pending.buffer = Some(buffer);
        }
    }

    /// Marks part of its buffer, in the buffer's coordinates, as changed at the next commit.
    pub fn damage(&mut self, id: WindowId, r: Rect) {
        if let Some(w) = self.windows.get_mut(&id) {
            let d = &mut w.pending.damage;
            *d = if d.is_empty() { r } else { d.union(&r) };
        }
    }

    /// Where the next buffers' top left is, from its content's (`x`, `y` left and up, in logical pixels, never
    /// below 0), with all of each buffer shown, from the next commit; None, at its content's top left and only within
    /// it. Wayland's window geometry and X11's `_GTK_FRAME_EXTENTS`: a client that draws its own shadow keeps it out
    /// of its window, so it's placed, sized, maximized and clicked by its window alone, the shadow drawn around it.
    pub fn set_buffer_offset(&mut self, id: WindowId, offset: Option<(i32, i32)>) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.pending.offset = Some(offset.map(|(x, y)| (x.max(0), y.max(0))));
        }
    }

    /// How many of the next buffers' pixels make a logical one (Wayland's `set_buffer_scale`; a 2× buffer for a 2×
    /// output), from the next commit. The logical screen shows such a buffer scaled down; `screen_pixels`, at the
    /// output's scale, shows it as it is.
    pub fn set_buffer_scale(&mut self, id: WindowId, scale: u32) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.pending.scale = Some(scale.max(1));
        }
    }

    /// Asks for a `Frame` event after the next commit, at the clock's next tick.
    pub fn request_frame(&mut self, id: WindowId) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.pending.frame = true;
        }
    }

    /// Makes what's pending current, at once: the buffer attached (the one it replaces is released), its damage, and
    /// a frame request.
    pub fn commit(&mut self, id: WindowId) {
        let Some(w) = self.windows.get_mut(&id) else { return };
        let pending = std::mem::take(&mut w.pending);
        let before = w.extent();
        if let Some(scale) = pending.scale {
            w.buffer_scale = scale;
        }
        let moved = pending.offset.is_some_and(|o| o != w.offset);
        if let Some(offset) = pending.offset {
            w.offset = offset;
        }
        let content = w.content_rect();
        // Damage is in the buffer's pixels: in logical ones, everything it touches.
        let s = w.buffer_scale as i32;
        // Bounded first: a client damages "everything" as (0, 0, INT32_MAX, INT32_MAX).
        let d = pending.damage.intersect(&Rect::new(0, 0, 1 << 24, 1 << 24));
        let damage = if d.is_empty() { d } else { Rect::new(d.left / s, d.top / s, (d.right + s - 1) / s, (d.bottom + s - 1) / s) };
        let (ox, oy) = w.offset.unwrap_or((0, 0));
        let whole = Rect::new(0, 0, content.width(), content.height());
        let mut observed = damage.offset(-ox, -oy).intersect(&whole);
        if pending.buffer.is_some() || pending.scale.is_some() || moved {
            observed = whole;
        }
        self.observers.note(Note::Damage(id, observed));
        let Some(w) = self.windows.get_mut(&id) else { return };
        let replaced = pending.buffer.is_some();
        if let Some(buffer) = pending.buffer {
            let old = std::mem::replace(&mut w.buffer, buffer);
            if let Some(old) = old
                && w.buffer.as_ref().is_none_or(|b| b.id != old.id)
            {
                self.events.push_back(Event::Released { window: id, buffer: old.id });
            }
        }
        // On the screen: within its content, or with an offset, wherever its buffer is.
        let w = &self.windows[&id];
        let shown = if w.offset.is_some() { w.extent() } else { content };
        let mut changed = damage.offset(content.left - ox, content.top - oy).intersect(&shown);
        if replaced || moved {
            changed = if w.offset.is_none() && before == w.rect { content } else { shown.union(&before) };
        }
        if self.windows[&id].on_screen() {
            self.damage_rect(changed);
        }
        if pending.presented && !self.presenting.insert(id) {
            // The commit before was never shown: this one replaced it.
            self.events.push_back(Event::Presented { window: id, time: None, seq: self.refreshes });
        }
        if pending.frame || pending.presented {
            let idle = !self.wants_frame();
            if pending.frame {
                self.frames.insert(id);
            }
            if idle && let Some(clock) = &self.clock {
                clock();
            }
        }
    }

    /// What its content shows where no buffer covers it.
    pub fn fill(&self, id: WindowId) -> Option<Rgb> {
        self.windows.get(&id).map(|w| w.fill)
    }

    /// The buffer it shows now.
    pub fn buffer(&self, id: WindowId) -> Option<&Buffer> {
        self.windows.get(&id)?.buffer.as_ref()
    }

    /// Whether any window is waiting for a frame: a clock with nothing waiting can stop.
    pub fn wants_frame(&self) -> bool {
        !self.frames.is_empty() || !self.presenting.is_empty()
    }

    /// The clock ticked, at `time` milliseconds: what was committed is on the screen now, so each commit waiting to
    /// hear is told it was presented (or, if its window isn't on the screen, that it wasn't), and every window waiting
    /// for a frame gets one.
    pub fn frame(&mut self, time: u32) {
        self.refreshes += 1;
        for window in std::mem::take(&mut self.presenting) {
            let shown = self.windows.get(&window).is_some_and(Window::on_screen);
            self.events.push_back(Event::Presented { window, time: shown.then_some(time), seq: self.refreshes });
        }
        for window in std::mem::take(&mut self.frames) {
            self.events.push_back(Event::Frame { window, time });
        }
    }

    /// The top window shown at a point on the screen.
    /// The window the pointer at `(x, y)` would go to: the topmost on the screen there whose input region has it.
    pub fn window_at(&self, x: i32, y: i32) -> Option<WindowId> {
        self.order.iter().rev().copied().find(|id| self.windows.get(id).is_some_and(|w| w.on_screen() && w.takes_pointer(x, y)))
    }

    /// Where its content takes pointer input, in its content's coordinates (None: all of it). Elsewhere in it, the
    /// pointer goes to what's below, as if it weren't there: Wayland's `set_input_region`, X11's `ShapeInput`. A
    /// client's shadow, or a surface that only draws (Firefox's), is made with an empty one.
    pub fn set_input_region(&mut self, id: WindowId, region: Option<Vec<Rect>>) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.input = region;
            self.update_cursor();
        }
    }

    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }

    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    pub fn modifiers(&self) -> u8 {
        self.modifiers
    }

    /// What's happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    /// The events for windows whose client `mine` says are its (None for a window that's gone), the rest left for
    /// the kernel's other servers: lambda's Wayland and X11 servers share one desktop this way.
    pub fn take_events_where(&mut self, mine: impl Fn(Option<ClientId>) -> bool) -> Vec<Event> {
        let events = std::mem::take(&mut self.events);
        let mut taken = Vec::new();
        for e in events {
            if mine(self.windows.get(&event_window(&e)).map(|w| w.client)) {
                taken.push(e);
            } else {
                self.events.push_back(e);
            }
        }
        taken
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// The screen as it is now, every shown window composited.
    pub fn screen(&mut self) -> &Surface {
        self.composite();
        &self.screen
    }

    /// The screen in the output's own pixels: what a host with a high-resolution screen shows (a Mac's Retina screen at
    /// 2×). Frames, fills and buffers at 1× are the logical screen's, magnified, as Windows magnifies a program that
    /// isn't DPI aware; an opaque buffer drawn at a higher scale (a 2× buffer on a 2× output, `set_buffer_scale`) is
    /// shown pixel for pixel, sharp, where no window above covers it. At 1× it's the logical screen (`screen`) itself.
    /// Observers' `Observed::screen` is in logical pixels: `Output::rect_to_pixels` makes it this one's.
    pub fn screen_pixels(&mut self) -> &Surface {
        self.composite();
        if self.output.scale == SCALE_1X {
            self.hires = None;
            return &self.screen;
        }
        let (pw, ph) = (self.output.to_pixels(self.screen.width), self.output.to_pixels(self.screen.height));
        if self.hires.as_ref().is_none_or(|s| (s.width, s.height) != (pw, ph)) {
            self.hires = Some(Surface::new(pw, ph, BACKGROUND));
            self.hires_damage = self.screen.bounds();
        }
        let clip = std::mem::take(&mut self.hires_damage).intersect(&self.screen.bounds());
        if !clip.is_empty() {
            self.composite_pixels(clip);
        }
        self.hires.as_ref().expect("just made")
    }

    /// Makes part of the screen in its own pixels (`clip`, in logical ones): the logical screen magnified, then each
    /// high-resolution buffer drawn over it, pixel for pixel, where nothing above covers it.
    fn composite_pixels(&mut self, clip: Rect) {
        let Desktop { hires, screen, windows, order, output, .. } = self;
        let Some(hires) = hires.as_mut() else { return };
        let scale = i64::from(output.scale);
        let logical = |p: i32| (i64::from(p) * i64::from(SCALE_1X) / scale) as i32;
        let r = output.rect_to_pixels(clip).intersect(&hires.bounds());
        if r.is_empty() {
            return;
        }
        let columns: Vec<usize> = (r.left..r.right).map(|x| logical(x).clamp(0, screen.width - 1) as usize).collect();
        for y in r.top..r.bottom {
            let from = (logical(y).clamp(0, screen.height - 1) * screen.width) as usize;
            let at = (y * hires.width + r.left) as usize;
            for (d, &lx) in hires.pixels[at..at + columns.len()].iter_mut().zip(&columns) {
                *d = screen.pixels[from + lx];
            }
        }
        for (i, id) in order.iter().enumerate() {
            let w = &windows[id];
            let Some(buffer) = w.buffer.as_ref().filter(|b| w.on_screen() && w.buffer_scale > 1 && b.pixels.format() == Format::Xrgb8888) else { continue };
            let Some(at) = w.buffer_rect() else { continue };
            let mut parts = vec![if w.offset.is_some() { at } else { w.content_rect() }.intersect(&clip)];
            for above in &order[i + 1..] {
                if let Some(a) = windows.get(above).filter(|a| a.on_screen()) {
                    parts = subtract(parts, &a.extent());
                }
            }
            // Buffer pixels a screen pixel: the buffer's scale over the screen's (2 over 2: one).
            let (num, den) = (i64::from(w.buffer_scale) * i64::from(SCALE_1X), scale);
            let (bw, bh) = buffer.pixels.size();
            let (ox, oy) = (output.to_pixels(at.left), output.to_pixels(at.top));
            let extent = Rect::new(ox, oy, ox + (i64::from(bw) * den / num) as i32, oy + (i64::from(bh) * den / num) as i32);
            for part in parts.into_iter().filter(|p| !p.is_empty()) {
                let p = output.rect_to_pixels(part).intersect(&extent).intersect(&hires.bounds());
                if p.is_empty() {
                    continue;
                }
                let n = (p.right - p.left) as usize;
                let b0 = ((i64::from(p.left - ox)) * num / den) as i32;
                let b1 = ((i64::from(p.right - 1 - ox)) * num / den) as i32;
                let mut row = vec![0u32; (b1 - b0 + 1) as usize];
                for y in p.top..p.bottom {
                    let by = ((i64::from(y - oy)) * num / den) as i32;
                    buffer.pixels.read_row(b0, by.min(bh - 1), &mut row);
                    let at = (y * hires.width + p.left) as usize;
                    for (k, d) in hires.pixels[at..at + n].iter_mut().enumerate() {
                        let bx = ((i64::from(p.left - ox + k as i32)) * num / den) as i32 - b0;
                        *d = row[bx as usize] & 0x00ff_ffff;
                    }
                }
            }
        }
    }

    /// What's changed on the screen since the last call, composited: for a host to show.
    pub fn take_damage(&mut self) -> Rect {
        let r = self.damage.intersect(&self.screen.bounds());
        self.composite_rect(r);
        self.damage = Rect::default();
        r
    }

    // ---- Input ----

    pub fn input(&mut self, input: Input) {
        match input {
            Input::PointerTo(x, y) => self.motion((x, y), (x - self.pointer.0, y - self.pointer.1)),
            Input::PointerBy(dx, dy) => self.motion((self.pointer.0.saturating_add(dx), self.pointer.1.saturating_add(dy)), (dx, dy)),
            Input::Button(b, down) => self.button(b, down),
            Input::Scroll { x, y } => self.scroll(x, y),
            Input::Key(code, down) => self.key(code, down),
            Input::Text(text) => {
                if let Some(id) = self.keyboard_target() {
                    self.events.push_back(Event::Text { window: id, text });
                }
            }
            Input::Preedit { text, cursor } => {
                if let Some((id, _)) = self.text_input {
                    self.events.push_back(Event::Preedit { window: id, text, cursor });
                }
            }
        }
        self.update_cursor();
        self.update_text_input();
    }

    /// The window keys and text go to: a grabbing popup, else the active window.
    fn keyboard_target(&self) -> Option<WindowId> {
        self.popup_grab().or(self.active)
    }

    // ---- Cursors and text input ----

    /// What the pointer looks like over a window's content.
    pub fn set_cursor(&mut self, id: WindowId, cursor: Cursor) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.cursor = cursor;
            self.update_cursor();
        }
    }

    /// The cursor shown now, for whoever draws the desktop.
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// The window the pointer's in now, as its enter said.
    pub fn hovered(&self) -> Option<WindowId> {
        self.hover
    }

    /// Says a window takes text from an input method, with its text cursor at `at` (in its content's coordinates, for
    /// placing the input method's window), or with None, that it doesn't: Wayland's `text_input_v3` enable and
    /// `set_cursor_rectangle`, Windows' IMM.
    pub fn set_text_input(&mut self, id: WindowId, at: Option<Rect>) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.text_input = at;
            self.update_text_input();
        }
    }

    /// The window an input method serves now, if it takes text, and where its text cursor is.
    pub fn text_input(&self) -> Option<(WindowId, Rect)> {
        self.text_input
    }

    /// The cursor for where the pointer is: the window it's over (or that has it), or the desktop's own over a frame.
    fn update_cursor(&mut self) {
        let edges = |e: u8| {
            Cursor::Shape(match e {
                e if e & (LEFT | TOP) == LEFT | TOP || e & (RIGHT | BOTTOM) == RIGHT | BOTTOM => Shape::NwseResize,
                e if e & (RIGHT | TOP) == RIGHT | TOP || e & (LEFT | BOTTOM) == LEFT | BOTTOM => Shape::NeswResize,
                e if e & (LEFT | RIGHT) != 0 => Shape::EwResize,
                _ => Shape::NsResize,
            })
        };
        let (x, y) = self.pointer;
        let cursor = match self.grab {
            _ if self.locked().is_some() => Cursor::Hidden,
            Some(Grab::Move { .. }) => Cursor::Shape(Shape::Move),
            Some(Grab::Drag) => Cursor::Shape(match self.drag.as_ref().map_or(0, |d| d.info.action) {
                action::COPY => Shape::Copy,
                action::MOVE => Shape::Move,
                action::LINK => Shape::Alias,
                action::ASK => Shape::ContextMenu,
                _ => Shape::NoDrop,
            }),
            Some(Grab::Resize { edges: e, .. }) => edges(e),
            Some(Grab::Content(id)) => self.windows.get(&id).map(|w| w.cursor.clone()).unwrap_or_default(),
            _ => match self.capture.or_else(|| self.window_at(x, y)) {
                Some(id) => {
                    let w = &self.windows[&id];
                    match w.part(x, y) {
                        Some(Part::Edge(e)) if !w.placed_by_desktop() => edges(e),
                        Some(Part::Content) | None => w.cursor.clone(),
                        _ if self.capture == Some(id) => w.cursor.clone(),
                        _ => Cursor::default(),
                    }
                }
                None => Cursor::default(),
            },
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            self.observers.note(Note::Cursor);
        }
    }

    /// The text input an input method serves: the keyboard's window's, if it takes text.
    fn update_text_input(&mut self) {
        let now = self.keyboard_target().and_then(|id| self.windows.get(&id).and_then(|w| w.text_input).map(|r| (id, r)));
        if now != self.text_input {
            self.text_input = now;
            self.observers.note(Note::TextInput);
        }
    }

    fn pointer_to(&mut self, x: i32, y: i32) {
        let (w, h) = self.size();
        let (x, y) = (x.clamp(0, w - 1), y.clamp(0, h - 1));
        if (x, y) == self.pointer {
            return;
        }
        self.pointer = (x, y);
        match self.grab {
            Some(Grab::Content(id)) => self.pointer_event(id, Pointer::Move),
            Some(Grab::Move { window, dx, dy }) => {
                if let Some(r) = self.rect(window) {
                    let to = Rect::new(x - dx, y - dy, x - dx + r.width(), y - dy + r.height());
                    self.user_placed(window, to);
                }
            }
            Some(Grab::Resize { window, edges, start, from }) => {
                let (ddx, ddy) = (x - from.0, y - from.1);
                let ((min_w, min_h), (max_w, max_h)) = self.frame_limits(window);
                let mut r = start;
                if edges & LEFT != 0 {
                    r.left = (start.left + ddx).clamp(start.right - max_w, start.right - min_w);
                }
                if edges & RIGHT != 0 {
                    r.right = (start.right + ddx).clamp(start.left + min_w, start.left + max_w);
                }
                if edges & TOP != 0 {
                    r.top = (start.top + ddy).clamp(start.bottom - max_h, start.bottom - min_h);
                }
                if edges & BOTTOM != 0 {
                    r.bottom = (start.bottom + ddy).clamp(start.top + min_h, start.top + max_h);
                }
                self.user_placed(window, r);
            }
            Some(Grab::Drag) => self.drag_over(),
            Some(Grab::Close(_) | Grab::Maximize(_) | Grab::Dismissed) => {}
            None => {
                // Entering says where it came in; after that, moves.
                if !self.update_hover()
                    && let Some(id) = self.pointer_target()
                {
                    self.pointer_event(id, Pointer::Move);
                }
            }
        }
    }

    fn button(&mut self, b: Button, down: bool) {
        let (x, y) = self.pointer;
        if down {
            if self.buttons & b.bit() != 0 {
                return;
            }
            self.buttons |= b.bit();
            let pressed = match self.grab {
                Some(Grab::Content(id)) => Some(id),
                _ => self.capture.filter(|_| self.popup_grab().is_none()).or_else(|| self.window_at(x, y)),
            };
            self.user_input(pressed);
            if let Some(grab) = self.grab {
                if let Grab::Content(id) = grab {
                    self.pointer_event(id, Pointer::Down(b));
                }
                return;
            }
            // A capture: every press is its window's, wherever the pointer is.
            if let Some(c) = self.capture.filter(|_| self.popup_grab().is_none()) {
                self.grab = Some(Grab::Content(c));
                self.pointer_event(c, Pointer::Down(b));
                return;
            }
            // A popup's grab: a press inside its chain goes to it; one anywhere else dismisses them all, and goes
            // nowhere, as a menu closes.
            if let Some(top) = self.popup_grab() {
                let chain = self.grab_chain(top);
                match self.window_at(x, y) {
                    Some(id) if chain.contains(&id) => {
                        self.grab = Some(Grab::Content(id));
                        self.pointer_event(id, Pointer::Down(b));
                    }
                    _ => {
                        let root = *chain.last().unwrap_or(&top);
                        self.dismiss(root);
                        self.grab = Some(Grab::Dismissed);
                    }
                }
                return;
            }
            let Some(id) = self.window_at(x, y) else {
                self.set_active(None);
                return;
            };
            // A popup without a grab (a tooltip), or a window that's never activated by a click, takes the press, but
            // not being active (and isn't raised).
            if !self.is_popup(id) && !self.windows[&id].no_activate {
                self.set_active(Some(id));
            }
            let w = &self.windows[&id];
            match (w.part(x, y), b) {
                (Some(Part::Content), _) => {
                    self.grab = Some(Grab::Content(id));
                    self.pointer_event(id, Pointer::Down(b));
                }
                // A maximized window's frame doesn't move or resize it: its place is the desktop's.
                (Some(Part::Caption), Button::Left) if !w.placed_by_desktop() => {
                    self.grab = Some(Grab::Move { window: id, dx: x - w.rect.left, dy: y - w.rect.top })
                }
                (Some(Part::Close), Button::Left) => self.grab = Some(Grab::Close(id)),
                (Some(Part::Maximize), Button::Left) => self.grab = Some(Grab::Maximize(id)),
                (Some(Part::Edge(edges)), Button::Left) if !w.placed_by_desktop() => {
                    self.grab = Some(Grab::Resize { window: id, edges, start: w.rect, from: (x, y) });
                    if let Some(w) = self.windows.get_mut(&id) {
                        w.states |= state::RESIZING;
                    }
                }
                _ => {}
            }
        } else {
            if self.buttons & b.bit() == 0 {
                return;
            }
            self.buttons &= !b.bit();
            match self.grab {
                Some(Grab::Content(id)) => self.pointer_event(id, Pointer::Up(b)),
                Some(Grab::Close(id)) if b == Button::Left && self.windows.get(&id).is_some_and(|w| w.close_button().contains(x, y)) => {
                    self.events.push_back(Event::Close { window: id });
                }
                Some(Grab::Maximize(id)) if b == Button::Left && self.windows.get(&id).is_some_and(|w| w.maximize_button().contains(x, y)) => {
                    let maximized = self.states(id) & state::MAXIMIZED != 0;
                    self.set_maximized(id, !maximized);
                }
                Some(Grab::Drag) if self.buttons == 0 => self.drop_drag(),
                Some(Grab::Resize { window, .. }) if b == Button::Left => {
                    if let Some(w) = self.windows.get_mut(&window) {
                        w.states &= !state::RESIZING;
                    }
                    self.configure(window);
                }
                _ => {}
            }
            if self.buttons == 0 {
                self.grab = None;
                // What it's over now may not be what it went down on.
                self.update_hover();
            }
        }
    }

    fn scroll(&mut self, dx: i32, dy: i32) {
        let target = match self.grab {
            Some(Grab::Content(id)) => Some(id),
            _ => self.pointer_target().or_else(|| self.popup_grab()).or(self.active),
        };
        if let Some(id) = target {
            self.pointer_event(id, Pointer::Scroll { x: dx, y: dy });
        }
    }

    /// The window the pointer's events go to, with no button held: the capture's, else the content under it (within
    /// a popup's grab, if there's one).
    fn pointer_target(&self) -> Option<WindowId> {
        if let Some(c) = self.capture {
            return Some(c);
        }
        let (x, y) = self.pointer;
        let id = self.window_at(x, y).filter(|id| self.windows[id].part(x, y) == Some(Part::Content))?;
        match self.popup_grab() {
            Some(top) if !self.grab_chain(top).contains(&id) => None,
            _ => Some(id),
        }
    }

    /// Enter and leave, as the window the pointer's over changes (not while a button's held). True if it entered one.
    fn update_hover(&mut self) -> bool {
        if self.grab.is_some() {
            return false;
        }
        let target = self.pointer_target();
        if target == self.hover {
            return false;
        }
        if let Some(old) = self.hover.take().filter(|o| self.windows.contains_key(o)) {
            self.pointer_event(old, Pointer::Leave);
        }
        self.hover = target;
        if let Some(new) = target {
            self.pointer_event(new, Pointer::Enter);
        }
        target.is_some()
    }

    // ---- Capture ----

    /// Gives one window every pointer event, wherever the pointer is, until it's taken away: Windows' `SetCapture`,
    /// X11's `GrabPointer`. A press then doesn't activate another window or use a frame. The window that had it.
    pub fn set_capture(&mut self, id: Option<WindowId>) -> Option<WindowId> {
        let id = id.filter(|i| self.windows.contains_key(i));
        let old = std::mem::replace(&mut self.capture, id);
        self.update_hover();
        old
    }

    pub fn capture(&self) -> Option<WindowId> {
        self.capture
    }

    fn key(&mut self, code: u16, down: bool) {
        let m = keys::modifier(code);
        let repeat = down && !self.keys_down.insert(code);
        if !down {
            self.keys_down.remove(&code);
        }
        if down {
            self.modifiers |= m;
        } else {
            self.modifiers &= !m;
        }
        // Escape cancels a drag, and goes nowhere else.
        if down && code == keys::ESC && self.grab == Some(Grab::Drag) {
            self.cancel_drag();
            return;
        }
        let target = self.keyboard_target();
        if down && !repeat {
            self.user_input(target);
        }
        if let Some(id) = target {
            self.events.push_back(Event::Key { window: id, code, down, repeat, modifiers: self.modifiers });
        }
    }

    fn pointer_event(&mut self, id: WindowId, what: Pointer) {
        let Some(c) = self.content_rect(id) else { return };
        let (x, y) = (self.pointer.0 - c.left, self.pointer.1 - c.top);
        let mut what = what;
        // Relative motion the kernel hasn't read yet adds up.
        if let Pointer::Relative { dx, dy } = what
            && let Some(Event::Pointer { window, what: Pointer::Relative { dx: ox, dy: oy }, .. }) = self.events.back()
            && *window == id
        {
            what = Pointer::Relative { dx: dx.saturating_add(*ox), dy: dy.saturating_add(*oy) };
            self.events.pop_back();
        }
        let event = Event::Pointer { window: id, x, y, what, buttons: self.buttons, modifiers: self.modifiers };
        // Moves the kernel hasn't read yet are only where the pointer was on the way.
        if what == Pointer::Move
            && let Some(Event::Pointer { window, what: Pointer::Move, .. }) = self.events.back()
            && *window == id
        {
            self.events.pop_back();
        }
        self.events.push_back(event);
    }

    /// Placed by its frame: moved there, and its kernel told.
    fn user_placed(&mut self, id: WindowId, rect: Rect) {
        self.set_rect(id, rect);
        self.configure(id);
    }

    /// Tells its kernel where it is and what it is, with a new serial. One it hasn't read yet is replaced: only
    /// the latest matters.
    fn configure(&mut self, id: WindowId) {
        let Some(w) = self.windows.get(&id) else { return };
        let (rect, states) = (w.rect, w.states);
        if let Some(Event::Configure { window, .. }) = self.events.back()
            && *window == id
        {
            self.events.pop_back();
        }
        self.serial = self.serial.wrapping_add(1);
        self.events.push_back(Event::Configure { window: id, rect, states, serial: self.serial });
        self.observers.note(Note::Window(id));
    }

    /// Its frame's smallest and largest size, from its content's limits and the frame's own least.
    fn frame_limits(&self, id: WindowId) -> ((i32, i32), (i32, i32)) {
        let Some(w) = self.windows.get(&id) else { return ((MIN_WIDTH, MIN_HEIGHT), (i32::MAX / 2, i32::MAX / 2)) };
        let i = Insets::of(w.has_frame());
        let (fw, fh) = (i.left + i.right, i.top + i.bottom);
        // A frame is never smaller than its title bar and buttons need.
        let (least_w, least_h) = if w.has_frame() { (MIN_WIDTH, MIN_HEIGHT) } else { (1, 1) };
        let min = ((w.min_size.0 + fw).max(least_w), (w.min_size.1 + fh).max(least_h));
        let most = |content: i32, frame: i32, floor: i32| if content > 0 { (content + frame).max(floor) } else { i32::MAX / 2 };
        (min, (most(w.max_size.0, fw, min.0), most(w.max_size.1, fh, min.1)))
    }

    // ---- Stacking and activation ----

    /// It and every window it owns, through their owners, bottom to top as they are now.
    fn owned_by(&self, id: WindowId) -> Vec<WindowId> {
        let mut family = vec![id];
        let mut grew = true;
        while grew {
            grew = false;
            for (&o, w) in &self.windows {
                if !family.contains(&o) && w.owner.is_some_and(|owner| family.contains(&owner)) {
                    family.push(o);
                    grew = true;
                }
            }
        }
        self.order.iter().copied().filter(|o| family.contains(o)).collect()
    }

    /// The band a window's stacked in, bottom to top: `Layer::Below`, `Normal`, `Top`, a fullscreen window while it or a
    /// window it owns is active (above docks and always-on-top windows, as EWMH has it), and popups. A window another
    /// owns is in its owner's band at least: a topmost window's dialogs are topmost, as on Windows, and a fullscreen
    /// window's dialogs are above it.
    fn band(&self, id: WindowId) -> u8 {
        let Some(w) = self.windows.get(&id) else { return 0 };
        let own = match w.layer {
            Layer::Below => 0,
            Layer::Normal => 1,
            Layer::Top => 2,
            Layer::Popup => return 4,
        };
        let own = if w.states & state::FULLSCREEN != 0 && self.in_active_family(id) { 3 } else { own };
        match w.owner {
            Some(o) if self.windows.contains_key(&o) => own.max(self.band(o)),
            _ => own,
        }
    }

    /// Whether the active window is it, or one it owns (through their owners).
    fn in_active_family(&self, id: WindowId) -> bool {
        let mut at = self.active;
        while let Some(a) = at {
            if a == id {
                return true;
            }
            at = self.windows.get(&a).and_then(|w| w.owner);
        }
        false
    }

    /// Makes the stacking hold to its rules, keeping the order otherwise: bands in order (`band`), and each owned window
    /// above its owner. What moved is drawn again.
    fn restack(&mut self) {
        let bands: BTreeMap<WindowId, u8> = self.order.iter().map(|&id| (id, self.band(id))).collect();
        let mut sorted = self.order.clone();
        sorted.sort_by_key(|id| bands[id]);
        let mut order: Vec<WindowId> = Vec::with_capacity(sorted.len());
        let mut waiting: Vec<WindowId> = Vec::new();
        for id in sorted {
            let owner = self.windows[&id].owner.filter(|o| bands.contains_key(o));
            if let Some(o) = owner
                && bands[&o] == bands[&id]
                && !order.contains(&o)
            {
                waiting.push(id);
                continue;
            }
            order.push(id);
            // Owned windows waiting for this one go just above it, in the order they were.
            while let Some(i) = waiting.iter().position(|w| self.windows[w].owner.is_some_and(|o| order.contains(&o))) {
                order.push(waiting.remove(i));
            }
        }
        order.extend(waiting);
        if order != self.order {
            let moved: Vec<Rect> = order.iter().enumerate().filter(|&(i, id)| self.order.get(i) != Some(id)).filter_map(|(_, id)| self.windows.get(id).filter(|w| w.on_screen()).map(Window::extent)).collect();
            for r in moved {
                self.damage_rect(r);
            }
            self.order = order;
        }
        self.observers.note(Note::Stacking);
    }

    fn set_active(&mut self, id: Option<WindowId>) {
        if self.active == id {
            if let Some(id) = id {
                self.raise(id);
            }
            return;
        }
        self.observers.note(Note::Active);
        if let Some(old) = self.active.take() {
            self.events.push_back(Event::Activated { window: old, active: false });
            if let Some(r) = self.extent_of(old) {
                self.damage_rect(r);
            }
        }
        if let Some(new) = id {
            self.active = Some(new);
            self.raise(new);
            self.events.push_back(Event::Activated { window: new, active: true });
            self.set_attention(new, false);
        }
        // A fullscreen window that's no longer active goes back down among the others.
        self.restack();
        self.update_text_input();
        self.update_constraint();
    }

    /// After the active window goes: its owner if that's shown, else the top window shown.
    fn activate_next(&mut self, owner: Option<WindowId>) {
        let on_screen = |o: WindowId| self.windows.get(&o).is_some_and(|w| w.on_screen() && w.popup.is_none() && !w.no_activate);
        let next = owner.filter(|&o| on_screen(o)).or_else(|| self.order.iter().rev().copied().find(|&o| on_screen(o)));
        if let Some(next) = next {
            self.set_active(Some(next));
        }
    }

    // ---- Compositing ----

    /// Part of the screen to composite again; each observer hears of it, to show it again when it next takes.
    fn damage_rect(&mut self, r: Rect) {
        self.damage = if self.damage.is_empty() { r } else { self.damage.union(&r) };
        self.hires_damage = if self.hires_damage.is_empty() { r } else { self.hires_damage.union(&r) };
        self.observers.note(Note::Screen(r));
    }

    fn composite(&mut self) {
        let r = self.damage.intersect(&self.screen.bounds());
        self.composite_rect(r);
        self.damage = Rect::default();
    }

    fn composite_rect(&mut self, clip: Rect) {
        if clip.is_empty() {
            return;
        }
        self.screen.fill(&clip, BACKGROUND, &clip);
        for id in &self.order {
            let w = &self.windows[id];
            if !w.on_screen() || w.extent().intersect(&clip).is_empty() {
                continue;
            }
            if w.has_frame() {
                draw_frame(&mut self.screen, w, self.active == Some(*id), &clip);
            }
            let c = w.content_rect();
            let inside = c.intersect(&clip);
            // Its fill shows where its buffer doesn't reach; under a buffer with alpha, what's below shows through.
            let mut bare = vec![inside];
            if let Some(r) = w.buffer_rect().filter(|_| w.buffer.as_ref().is_some_and(|b| b.pixels.format() == Format::Argb8888)) {
                bare = subtract(bare, &r);
            }
            for part in bare.iter().filter(|p| !p.is_empty()) {
                self.screen.fill(part, w.fill, part);
            }
            if let (Some(b), Some(r)) = (&w.buffer, w.buffer_rect()) {
                // With an offset, all of it (its shadow past its content); else only what's within its content.
                let within = if w.offset.is_some() { r.intersect(&clip) } else { inside };
                show(&mut self.screen, b, w.buffer_scale, r.left, r.top, &within);
            }
        }
    }
}

/// Puts a buffer's pixels on the screen with its top left at `(x, y)`, within `clip`: copied if it's opaque, blended
/// over what's there if it has alpha. A buffer at `scale` has that many of its pixels to each of the screen's, on each
/// axis: every `scale`th is taken.
fn show(screen: &mut Surface, buffer: &Buffer, scale: u32, x: i32, y: i32, clip: &Rect) {
    let s = scale.max(1) as i32;
    let (bw, bh) = buffer.pixels.size();
    let (width, height) = (bw / s, bh / s);
    let r = Rect::new(x, y, x + width, y + height).intersect(clip).intersect(&screen.bounds());
    if r.is_empty() {
        return;
    }
    let format = buffer.pixels.format();
    let n = (r.right - r.left) as usize;
    let mut row = vec![0u32; n];
    let mut wide = vec![0u32; if s > 1 { (n - 1) * s as usize + 1 } else { 0 }];
    for sy in r.top..r.bottom {
        if s == 1 {
            buffer.pixels.read_row(r.left - x, sy - y, &mut row);
        } else {
            buffer.pixels.read_row((r.left - x) * s, (sy - y) * s, &mut wide);
            for (i, p) in row.iter_mut().enumerate() {
                *p = wide[i * s as usize];
            }
        }
        let at = (sy * screen.width + r.left) as usize;
        let dst = &mut screen.pixels[at..at + n];
        match format {
            Format::Xrgb8888 => {
                for (d, s) in dst.iter_mut().zip(&row) {
                    *d = s & 0x00ff_ffff;
                }
            }
            Format::Argb8888 => {
                for (d, s) in dst.iter_mut().zip(&row) {
                    *d = over(*s, *d);
                }
            }
        }
    }
}

/// What's left of some rectangles with another taken out of them.
fn subtract(parts: Vec<Rect>, cut: &Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    for p in parts {
        let i = p.intersect(cut);
        if i.is_empty() {
            out.push(p);
            continue;
        }
        for piece in [
            Rect::new(p.left, p.top, p.right, i.top),
            Rect::new(p.left, i.bottom, p.right, p.bottom),
            Rect::new(p.left, i.top, i.left, i.bottom),
            Rect::new(i.right, i.top, p.right, i.bottom),
        ] {
            if !piece.is_empty() {
                out.push(piece);
            }
        }
    }
    out
}

/// A premultiplied ARGB pixel over an opaque one.
fn over(src: u32, dst: u32) -> Rgb {
    let a = src >> 24;
    if a == 0xff {
        return src & 0x00ff_ffff;
    }
    let keep = 255 - a;
    let channel = |shift: u32| {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        (s + (d * keep + 127) / 255).min(255) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn event_window(e: &Event) -> WindowId {
    match e {
        Event::Configure { window, .. }
        | Event::Activated { window, .. }
        | Event::Close { window }
        | Event::Pointer { window, .. }
        | Event::Key { window, .. }
        | Event::Released { window, .. }
        | Event::Text { window, .. }
        | Event::Scale { window, .. }
        | Event::SelectionCancelled { window, .. }
        | Event::SendSelection { window, .. }
        | Event::SelectionData { window, .. }
        | Event::Preedit { window, .. }
        | Event::PopupDone { window }
        | Event::Presented { window, .. }
        | Event::Constrained { window, .. }
        | Event::DragEnter { window, .. }
        | Event::DragMotion { window, .. }
        | Event::DragLeave { window }
        | Event::Drop { window, .. }
        | Event::DragAction { window, .. }
        | Event::Dropped { window }
        | Event::DragEnd { window, .. }
        | Event::Frame { window, .. } => *window,
    }
}

/// A window's frame: an outline, its border and title bar in the active or inactive colour, its title, and a close
/// button.
fn draw_frame(s: &mut Surface, w: &Window, active: bool, clip: &Rect) {
    let colour = if active { ACTIVE } else { INACTIVE };
    let r = &w.rect;
    s.fill(r, colour, clip);
    s.frame(r, OUTLINE, clip);
    let caption = w.caption();
    let room = (caption.width() - 2 * CAPTION - 8) / font::CELL_WIDTH;
    let title: String = w.title.chars().take(room.max(0) as usize).collect();
    let text_clip = caption.intersect(clip);
    s.text(caption.left + 4, caption.top + (CAPTION - font::CELL_HEIGHT) / 2, &title, TITLE, None, &text_clip);
    let close = w.close_button();
    s.frame(&close, TITLE, clip);
    let (a, b) = ((close.left + 4, close.top + 4), (close.right - 5, close.bottom - 5));
    s.line(a, b, TITLE, clip);
    s.line((a.0, b.1), (b.0, a.1), TITLE, clip);
    // A square for maximize; two overlapping, to restore.
    let m = w.maximize_button();
    s.frame(&m, TITLE, clip);
    let inner = Rect::new(m.left + 4, m.top + 4, m.right - 4, m.bottom - 4);
    if w.states & state::MAXIMIZED != 0 {
        s.frame(&inner.offset(1, 1), TITLE, clip);
        s.frame(&inner.offset(-1, -1), TITLE, clip);
    } else {
        s.frame(&inner, TITLE, clip);
        s.line((inner.left, inner.top + 1), (inner.right - 1, inner.top + 1), TITLE, clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Buffer, Canvas, Format, Pixels};
    use crate::output::Output;
    use crate::positioner::Positioner;

    fn framed(left: i32, top: i32, w: i32, h: i32) -> Spec {
        Spec { rect: Rect::new(left, top, left + w, top + h), title: "w".into(), framed: true, fill: 0xffffff, ..Spec::default() }
    }

    /// The `Configure`s among some events, without their serials.
    fn configures(events: Vec<Event>) -> Vec<(WindowId, Rect, u32)> {
        events
            .into_iter()
            .filter_map(|e| match e {
                Event::Configure { window, rect, states, .. } => Some((window, rect, states)),
                _ => None,
            })
            .collect()
    }

    fn click(d: &mut Desktop, x: i32, y: i32) {
        d.input(Input::PointerTo(x, y));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
    }

    #[test]
    fn insets_go_both_ways() {
        let i = Insets::of(true);
        let r = Rect::new(10, 10, 210, 110);
        assert_eq!(i.around(&i.inside(&r)), r);
        assert_eq!(Insets::of(false).inside(&r), r);
    }

    #[test]
    fn clicking_activates_raises_and_reaches_content() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        let b = d.create(framed(200, 150, 300, 200));
        d.set_visible(a, true);
        d.set_visible(b, true);
        d.activate(b);
        d.take_events();
        assert_eq!(d.stacking(), &[a, b]);
        // a's content, where b doesn't cover it: entering, a move, then a click.
        d.input(Input::PointerTo(120, 140));
        assert!(matches!(d.take_events()[..], [Event::Pointer { window, what: Pointer::Enter, .. }] if window == a));
        d.input(Input::PointerTo(121, 140));
        assert!(matches!(d.take_events()[..], [Event::Pointer { window, what: Pointer::Move, .. }] if window == a));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.stacking(), &[b, a]);
        let content = d.content_rect(a).unwrap();
        let events = d.take_events();
        assert_eq!(events[0], Event::Activated { window: b, active: false });
        assert_eq!(events[1], Event::Activated { window: a, active: true });
        assert_eq!(events[2], Event::Pointer { window: a, x: 121 - content.left, y: 140 - content.top, what: Pointer::Down(Button::Left), buttons: 1, modifiers: 0 });
        assert!(matches!(events[3], Event::Pointer { window, what: Pointer::Up(Button::Left), buttons: 0, .. } if window == a));
        assert_eq!(d.window_at(250, 200), Some(a));
    }

    #[test]
    fn frames_move_resize_and_close() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.activate(a);
        d.take_events();
        // Drag the title bar by (50, 30).
        d.input(Input::PointerTo(150, 100 + BORDER + 5));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(180, 120 + BORDER));
        d.input(Input::PointerTo(200, 130 + BORDER + 5));
        d.input(Input::Button(Button::Left, false));
        let moved = Rect::new(150, 130, 450, 330);
        assert_eq!(d.rect(a), Some(moved));
        assert_eq!(configures(d.take_events()), vec![(a, moved, 0)]);
        // Drag the bottom right corner out by (40, 20).
        d.input(Input::PointerTo(449, 329));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(489, 349));
        d.input(Input::Button(Button::Left, false));
        let bigger = Rect::new(150, 130, 490, 350);
        assert_eq!(d.rect(a), Some(bigger));
        // Resizing while the button's down; told once more, as it's let go.
        assert_eq!(configures(d.take_events()), vec![(a, bigger, 0)]);
        // The close button, pressed and released on it.
        let (cx, cy) = (bigger.right - BORDER - CAPTION / 2, bigger.top + BORDER + CAPTION / 2);
        click(&mut d, cx, cy);
        assert_eq!(d.take_events(), vec![Event::Close { window: a }]);
        // Pressed on it, released off it: nothing.
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(cx - 100, cy));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.take_events(), vec![]);
    }

    #[test]
    fn a_drag_stays_with_its_window_and_keys_go_to_the_active_one() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(Spec { rect: Rect::new(0, 0, 100, 100), fill: 0, ..Spec::default() });
        d.set_visible(a, true);
        d.input(Input::PointerTo(50, 50));
        d.input(Input::Button(Button::Left, true));
        d.take_events();
        // Outside it, still to it; and only the last of the moves it hasn't read.
        d.input(Input::PointerTo(300, 300));
        d.input(Input::PointerTo(400, 350));
        assert_eq!(d.take_events(), vec![Event::Pointer { window: a, x: 400, y: 350, what: Pointer::Move, buttons: 1, modifiers: 0 }]);
        d.input(Input::Button(Button::Left, false));
        d.input(Input::Key(keys::LEFT_SHIFT, true));
        d.input(Input::Key(keys::LETTERS[0], true));
        let events = d.take_events();
        assert_eq!(events.last(), Some(&Event::Key { window: a, code: keys::LETTERS[0], down: true, repeat: false, modifiers: keys::SHIFT }));
        assert_eq!(keys::text(keys::LETTERS[0], keys::SHIFT), Some('A'));
    }

    #[test]
    fn owned_windows_stay_above_and_the_top_layer_above_all() {
        let mut d = Desktop::new(800, 600);
        let owner = d.create(framed(0, 0, 200, 200));
        let owned = d.create(Spec { role: Role::Dialog { parent: owner }, ..framed(50, 50, 100, 100) });
        let top = d.create(Spec { layer: Layer::Top, ..framed(300, 300, 100, 100) });
        let other = d.create(framed(400, 0, 200, 200));
        for id in [owner, owned, top, other] {
            d.set_visible(id, true);
        }
        assert_eq!(d.stacking(), &[owner, owned, other, top]);
        d.activate(owner);
        assert_eq!(d.stacking(), &[other, owner, owned, top]);
        d.set_visible(owner, false);
        assert_eq!(d.active(), Some(top));
        // Hiding an owner leaves what it owns shown: that's the top window now.
        d.destroy(top);
        assert_eq!(d.active(), Some(owned));
    }

    #[test]
    fn compositing_draws_frames_and_content() {
        let mut d = Desktop::new(400, 300);
        let a = d.create(framed(10, 10, 200, 100));
        d.set_visible(a, true);
        let c = d.content_rect(a).unwrap();
        let canvas = Canvas::new(c.width(), c.height(), 0xffffff);
        canvas.draw(|s| s.fill(&Rect::new(0, 0, 5, 5), 0xff0000, &Rect::new(0, 0, 1000, 1000)));
        d.attach(a, Some(canvas.buffer()));
        d.commit(a);
        d.activate(a);
        let c = d.content_rect(a).unwrap();
        let screen = d.screen();
        assert_eq!(screen.get(c.left, c.top), Some(0xff0000));
        assert_eq!(screen.get(c.left + 10, c.top + 10), Some(0xffffff));
        assert_eq!(screen.get(10, 10), Some(OUTLINE));
        assert_eq!(screen.get(c.left + 2, c.top - 2), Some(ACTIVE));
        assert_eq!(screen.get(300, 250), Some(BACKGROUND));
        d.destroy(a);
        assert_eq!(d.screen().get(c.left, c.top), Some(BACKGROUND));
    }

    #[test]
    fn send_wakes_the_kernel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let d = Desktop::shared(100, 100);
        let woken = Arc::new(AtomicUsize::new(0));
        let count = woken.clone();
        lock(&d).set_waker(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let a = lock(&d).create(Spec { rect: Rect::new(0, 0, 100, 100), ..Spec::default() });
        lock(&d).set_visible(a, true);
        send(&d, Input::PointerTo(10, 10));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
        lock(&d).take_events();
        send(&d, Input::PointerTo(10, 10));
        assert_eq!(woken.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn commits_attach_damage_and_release() {
        let mut d = Desktop::new(400, 300);
        let a = d.create(framed(10, 10, 200, 100));
        d.set_visible(a, true);
        d.screen();
        let c = d.content_rect(a).unwrap();
        let first = Canvas::new(c.width(), c.height(), 0x112233);
        d.attach(a, Some(first.buffer()));
        // Nothing shows until the commit.
        assert_eq!(d.screen().get(c.left, c.top), Some(0xffffff));
        d.commit(a);
        assert_eq!(d.screen().get(c.left, c.top), Some(0x112233));
        // Drawn into, and committed with damage: only that part counts as changed.
        first.draw(|s| s.fill(&Rect::new(3, 4, 6, 8), 0x00ff00, &Rect::new(0, 0, 1000, 1000)));
        d.damage(a, Rect::new(3, 4, 6, 8));
        d.commit(a);
        assert_eq!(d.take_damage(), Rect::new(c.left + 3, c.top + 4, c.left + 6, c.top + 8));
        assert_eq!(d.screen().get(c.left + 3, c.top + 4), Some(0x00ff00));
        // "Everything", as clients say it: the whole content, not an overflow.
        d.damage(a, Rect::new(0, 0, i32::MAX, i32::MAX));
        d.commit(a);
        assert_eq!(d.take_damage(), c);
        // Another buffer in its place: the first is released; the same one again isn't.
        let second = Canvas::new(c.width(), c.height(), 0x445566);
        d.attach(a, Some(second.buffer()));
        d.commit(a);
        assert_eq!(d.take_events(), vec![Event::Released { window: a, buffer: first.buffer().id }]);
        d.attach(a, Some(second.buffer()));
        d.commit(a);
        assert!(d.take_events().is_empty());
        // A buffer smaller than its content: the fill shows past it.
        let small = Canvas::new(5, 5, 0x0000ff);
        d.attach(a, Some(small.buffer()));
        d.commit(a);
        assert_eq!(d.screen().get(c.left + 4, c.top + 4), Some(0x0000ff));
        assert_eq!(d.screen().get(c.left + 5, c.top + 5), Some(0xffffff));
    }

    #[test]
    fn frames_come_at_the_tick_after_a_commit() {
        let mut d = Desktop::new(400, 300);
        let a = d.create(framed(10, 10, 200, 100));
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let s = started.clone();
        d.set_clock(Arc::new(move || {
            s.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        // Asked for, but not committed: nothing yet.
        d.request_frame(a);
        assert!(!d.wants_frame());
        d.commit(a);
        assert!(d.wants_frame());
        assert_eq!(started.load(std::sync::atomic::Ordering::Relaxed), 1);
        d.frame(16);
        assert_eq!(d.take_events(), vec![Event::Frame { window: a, time: 16 }]);
        // One request, one frame.
        d.frame(33);
        assert!(d.take_events().is_empty() && !d.wants_frame());
    }

    #[test]
    fn alpha_blends_over_what_is_below() {
        struct Half;
        impl Pixels for Half {
            fn size(&self) -> (i32, i32) {
                (4, 4)
            }
            fn format(&self) -> Format {
                Format::Argb8888
            }
            fn read_row(&self, _: i32, _: i32, out: &mut [u32]) {
                // Half-transparent red, premultiplied.
                out.fill(0x8080_0000);
            }
        }
        let mut d = Desktop::new(100, 100);
        let below = d.create(Spec { rect: Rect::new(0, 0, 50, 50), fill: 0x0000ff, ..Spec::default() });
        d.set_visible(below, true);
        let a = d.create(Spec { rect: Rect::new(10, 10, 30, 30), fill: 0x00ff00, ..Spec::default() });
        d.set_visible(a, true);
        d.attach(a, Some(Buffer::new(Arc::new(Half))));
        d.commit(a);
        // 0x80 red, and half of the blue window below it: its own fill isn't under its buffer (a shadow's see-through).
        assert_eq!(d.screen().get(11, 11), Some(0x80_007f));
        // Past its buffer, its fill.
        assert_eq!(d.screen().get(20, 20), Some(0x00ff00));
    }

    #[test]
    fn a_buffer_offset_keeps_a_shadow_outside_the_window() {
        // 70 by 70: a see-through shadow 10 wide around 50 by 50 of opaque red.
        struct Shadowed;
        impl Pixels for Shadowed {
            fn size(&self) -> (i32, i32) {
                (70, 70)
            }
            fn format(&self) -> Format {
                Format::Argb8888
            }
            fn read_row(&self, x: i32, y: i32, out: &mut [u32]) {
                for (i, p) in out.iter_mut().enumerate() {
                    let px = x + i as i32;
                    *p = if (10..60).contains(&px) && (10..60).contains(&y) { 0xffff_0000 } else { 0 };
                }
            }
        }
        let mut d = Desktop::new(200, 200);
        let below = d.create(Spec { rect: Rect::new(0, 0, 200, 200), fill: 0x0000ff, ..Spec::default() });
        d.set_visible(below, true);
        let a = d.create(Spec { rect: Rect::new(50, 50, 100, 100), fill: 0xffffff, ..Spec::default() });
        d.set_visible(a, true);
        d.set_buffer_offset(a, Some((10, 10)));
        d.attach(a, Some(Buffer::new(Arc::new(Shadowed))));
        d.commit(a);
        // Its window is the red; the shadow round it shows what's below, and isn't its to click.
        assert_eq!(d.screen().get(50, 50), Some(0xff0000));
        assert_eq!(d.screen().get(99, 99), Some(0xff0000));
        assert_eq!(d.screen().get(45, 45), Some(0x0000ff));
        assert_eq!(d.window_at(45, 45), Some(below));
        assert_eq!(d.window_at(50, 50), Some(a));
        // Its picture is its window's, from the red's corner.
        assert_eq!(d.read(a, Rect::new(0, 0, 50, 50)).and_then(|s| s.get(0, 0)), Some(0xff0000));
        assert_eq!(d.read_pixels(a).and_then(|s| s.get(0, 0)), Some(0xff0000));
        // Moved: where it and its shadow were is drawn again.
        d.take_damage();
        d.set_rect(a, Rect::new(120, 120, 170, 170));
        let damage = d.take_damage();
        assert!(damage.contains(40, 40) && damage.contains(179, 179), "{damage:?}");
        assert_eq!(d.screen().get(60, 60), Some(0x0000ff));
        assert_eq!(d.screen().get(120, 120), Some(0xff0000));
        // Taken away: its buffer at its content's corner again, only within it.
        d.set_buffer_offset(a, None);
        d.commit(a);
        assert_eq!(d.screen().get(120, 120), Some(0x0000ff));
        assert_eq!(d.screen().get(130, 130), Some(0xff0000));
        assert_eq!(d.screen().get(171, 171), Some(0x0000ff));
    }

    #[test]
    fn an_input_region_lets_the_pointer_through_elsewhere() {
        let mut d = Desktop::new(200, 200);
        let below = d.create(Spec { rect: Rect::new(0, 0, 200, 200), fill: 0x0000ff, ..Spec::default() });
        d.set_visible(below, true);
        // Frameless, over all of it, taking the pointer only in its top-left quarter.
        let over = d.create(Spec { rect: Rect::new(0, 0, 200, 200), fill: 0x00ff00, ..Spec::default() });
        d.set_visible(over, true);
        d.set_input_region(over, Some(vec![Rect::new(0, 0, 100, 100)]));
        assert_eq!(d.window_at(50, 50), Some(over));
        assert_eq!(d.window_at(150, 150), Some(below));
        // An empty one: an overlay that only draws.
        d.set_input_region(over, Some(Vec::new()));
        assert_eq!(d.window_at(50, 50), Some(below));
        d.input(Input::PointerTo(50, 50));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
        assert!(d.take_events().iter().any(|e| matches!(e, Event::Pointer { window, what: Pointer::Down(Button::Left), .. } if *window == below)));
        // Back to all of it (and on top again: the click activated and raised the one below); a framed window's
        // frame is its whatever its region.
        d.set_input_region(over, None);
        d.raise(over);
        assert_eq!(d.window_at(150, 150), Some(over));
        let framed = d.create(framed(20, 20, 100, 100));
        d.set_visible(framed, true);
        d.set_input_region(framed, Some(Vec::new()));
        assert_eq!(d.window_at(60, 22), Some(framed));
        assert_eq!(d.window_at(60, 60), Some(over));
    }

    #[test]
    fn the_clock_ticks_only_while_asked() {
        let d = Desktop::shared(100, 100);
        let a = lock(&d).create(framed(10, 10, 50, 50));
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        lock(&d).set_waker(Arc::new(move || {
            w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        let _clock = crate::clock::Clock::start(d.clone(), 200);
        {
            let mut desk = lock(&d);
            desk.request_frame(a);
            desk.commit(a);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while woken.load(std::sync::atomic::Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(lock(&d).take_events().as_slice(), [Event::Frame { window, .. }] if *window == a));
        // Nothing asked since: no more frames.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(lock(&d).take_events().is_empty());
    }

    /// A 100 by 80 menu, below a 40 by 20 item at the top left of its parent's content.
    fn menu(parent: WindowId, grab: bool) -> Spec {
        use crate::positioner::{Edge, adjust};
        let positioner = Positioner {
            size: (100, 80),
            anchor_rect: Rect::new(0, 0, 40, 20),
            anchor: Edge::BottomLeft,
            gravity: Edge::BottomRight,
            offset: (0, 0),
            adjust: adjust::FLIP_Y | adjust::SLIDE_X,
        };
        Spec { role: Role::Popup { parent, positioner, grab }, fill: 0xeeeeee, ..Spec::default() }
    }

    /// A parent window, active, with a menu open from it.
    fn with_menu() -> (Desktop, WindowId, WindowId) {
        let mut d = Desktop::new(800, 600);
        let top = d.create(Spec { layer: Layer::Top, ..framed(500, 400, 100, 100) });
        let parent = d.create(framed(100, 100, 300, 200));
        for id in [top, parent] {
            d.set_visible(id, true);
        }
        d.activate(parent);
        let m = d.create(menu(parent, true));
        d.set_visible(m, true);
        d.take_events();
        (d, parent, m)
    }

    #[test]
    fn a_popup_is_placed_against_its_parent_and_never_active() {
        let (mut d, parent, m) = with_menu();
        let c = d.content_rect(parent).unwrap();
        assert_eq!(d.rect(m), Some(Rect::new(c.left, c.top + 20, c.left + 100, c.top + 100)));
        // No frame: its content is all of it. Above even an always-on-top window.
        assert_eq!(d.content_rect(m), d.rect(m));
        assert_eq!(d.stacking().last(), Some(&m));
        // Its parent stays the active window, whatever's asked.
        d.activate(m);
        assert_eq!(d.active(), Some(parent));
        assert!(d.take_events().is_empty());
    }

    #[test]
    fn a_popups_grab_takes_keys_and_presses_and_a_press_outside_dismisses_it() {
        let (mut d, parent, m) = with_menu();
        let sub = d.create(menu(m, true));
        d.set_visible(sub, true);
        assert_eq!(d.popup_grab(), Some(sub));
        // Keys go to the topmost grab, not the active window.
        d.input(Input::Key(keys::TAB, true));
        assert!(matches!(d.take_events().as_slice(), [Event::Key { window, .. }] if *window == sub));
        // A press on a popup in the chain reaches it.
        let r = d.rect(m).unwrap();
        click(&mut d, r.left + 5, r.top + 10);
        let events = d.take_events();
        assert!(events.iter().any(|e| matches!(e, Event::Pointer { window, what: Pointer::Down(Button::Left), .. } if *window == m)));
        assert!(events.iter().all(|e| matches!(e, Event::Pointer { window, .. } if *window == m)));
        // A press anywhere else dismisses the chain, topmost first, and goes nowhere else.
        // Moving out of it first: it's left.
        click(&mut d, 700, 50);
        assert_eq!(
            d.take_events(),
            vec![
                Event::Pointer { window: m, x: 700 - r.left, y: 50 - r.top, what: Pointer::Leave, buttons: 0, modifiers: 0 },
                Event::PopupDone { window: sub },
                Event::PopupDone { window: m }
            ]
        );
        assert!(!d.is_visible(m) && !d.is_visible(sub) && d.popup_grab().is_none());
        assert_eq!(d.active(), Some(parent));
        // With no grab left, keys go to the active window again.
        d.input(Input::Key(keys::TAB, false));
        assert!(matches!(d.take_events().as_slice(), [Event::Key { window, .. }] if *window == parent));
    }

    #[test]
    fn popups_move_with_their_parent_and_go_with_it() {
        let (mut d, parent, m) = with_menu();
        let before = d.rect(m).unwrap();
        let p = d.rect(parent).unwrap();
        d.set_rect(parent, p.offset(30, 10));
        assert_eq!(d.rect(m), Some(before.offset(30, 10)));
        // Placed again by a new positioner: told where.
        let Role::Popup { mut positioner, .. } = menu(parent, true).role else { unreachable!() };
        positioner.anchor_rect = Rect::new(50, 0, 90, 20);
        d.set_positioner(m, positioner);
        let c = d.content_rect(parent).unwrap();
        let moved = Rect::new(c.left + 50, c.top + 20, c.left + 150, c.top + 100);
        assert_eq!(configures(d.take_events()), vec![(m, moved, 0)]);
        // Its parent hidden: dismissed.
        d.set_visible(parent, false);
        assert!(d.take_events().contains(&Event::PopupDone { window: m }));
        assert!(!d.is_visible(m));
    }

    #[test]
    fn a_popup_without_a_grab_takes_its_own_clicks_only() {
        let mut d = Desktop::new(800, 600);
        let parent = d.create(framed(100, 100, 300, 200));
        let other = d.create(framed(450, 100, 200, 200));
        for id in [parent, other] {
            d.set_visible(id, true);
        }
        d.activate(parent);
        let tip = d.create(menu(parent, false));
        d.set_visible(tip, true);
        d.take_events();
        assert_eq!(d.popup_grab(), None);
        // Clicked: the tooltip gets the press, and its parent stays active.
        let r = d.rect(tip).unwrap();
        click(&mut d, r.left + 5, r.top + 5);
        assert!(matches!(d.take_events().first(), Some(Event::Pointer { window, .. }) if *window == tip));
        assert_eq!(d.active(), Some(parent));
        // Elsewhere, input is as it always is.
        click(&mut d, 500, 200);
        assert_eq!(d.active(), Some(other));
        assert!(d.is_visible(tip));
    }

    #[test]
    fn maximized_fullscreen_and_back() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.activate(a);
        d.take_events();
        let normal = d.rect(a).unwrap();
        d.set_maximized(a, true);
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(0, 0, 800, 600), state::MAXIMIZED)]);
        // Its frame doesn't move it while it's maximized.
        let caption_y = BORDER + CAPTION / 2;
        d.input(Input::PointerTo(300, caption_y));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(350, caption_y + 40));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.rect(a), Some(Rect::new(0, 0, 800, 600)));
        // Fullscreen: no frame, so its content is all of the screen; then back to maximized, and to where it was.
        d.set_fullscreen(a, true);
        assert_eq!(d.content_rect(a), Some(Rect::new(0, 0, 800, 600)));
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(0, 0, 800, 600), state::MAXIMIZED | state::FULLSCREEN)]);
        d.set_fullscreen(a, false);
        assert_eq!(d.content_rect(a), Some(Insets::of(true).inside(&Rect::new(0, 0, 800, 600))));
        d.set_maximized(a, false);
        assert_eq!(configures(d.take_events()).last(), Some(&(a, normal, 0)));
    }

    #[test]
    fn the_maximize_button() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.take_events();
        let m = d.windows[&a].maximize_button();
        click(&mut d, m.left + 3, m.top + 3);
        assert_eq!(d.states(a), state::MAXIMIZED);
        // Where its button is now, maximized: pressed again, it's put back.
        let m = d.windows[&a].maximize_button();
        click(&mut d, m.left + 3, m.top + 3);
        assert_eq!((d.states(a), d.rect(a)), (0, Some(Rect::new(100, 100, 400, 300))));
    }

    #[test]
    fn minimized_windows_are_kept_off_the_screen() {
        let mut d = Desktop::new(800, 600);
        let back = d.create(framed(0, 0, 200, 200));
        let a = d.create(framed(100, 100, 300, 200));
        for id in [back, a] {
            d.set_visible(id, true);
        }
        d.activate(a);
        d.take_events();
        d.set_minimized(a, true);
        // Still shown as its kernel has it, but not on the screen, and not active.
        assert!(d.is_visible(a) && d.window_at(250, 250).is_none() && d.active() == Some(back));
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(100, 100, 400, 300), state::MINIMIZED)]);
        d.set_minimized(a, false);
        assert_eq!(d.window_at(250, 250), Some(a));
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(100, 100, 400, 300), 0)]);
    }

    #[test]
    fn resizing_keeps_to_its_size_limits_and_says_its_resizing() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.set_size_limits(a, (200, 100), (400, 0));
        d.take_events();
        let r = d.rect(a).unwrap();
        // The bottom right corner, dragged far out and held: only as wide as its largest, tall as asked.
        d.input(Input::PointerTo(r.right - 1, r.bottom - 1));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::PointerTo(r.right + 400, r.bottom + 50));
        let i = Insets::of(true);
        let widest = 400 + i.left + i.right;
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(100, 100, 100 + widest, 351), state::RESIZING)]);
        // Then in past its smallest.
        d.input(Input::PointerTo(r.left + 10, r.top + 10));
        d.input(Input::Button(Button::Left, false));
        let least = (200 + i.left + i.right, 100 + i.top + i.bottom);
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(100, 100, 100 + least.0, 100 + least.1), 0)]);
    }

    #[test]
    fn a_window_that_draws_its_own_title_bar_asks_to_be_moved_and_resized() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(Spec { rect: Rect::new(100, 100, 300, 250), fill: 0xffffff, ..Spec::default() });
        d.set_visible(a, true);
        // Too late: no button held.
        assert!(!d.begin_move(a));
        // Pressed on its content, which it says is its title bar: moved by the pointer until the button's up.
        d.input(Input::PointerTo(150, 110));
        d.input(Input::Button(Button::Left, true));
        assert!(d.begin_move(a));
        d.take_events();
        d.input(Input::PointerTo(180, 150));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.rect(a), Some(Rect::new(130, 140, 330, 290)));
        assert_eq!(configures(d.take_events()), vec![(a, Rect::new(130, 140, 330, 290), 0)]);
        // Its bottom right corner, the same way.
        d.input(Input::PointerTo(320, 280));
        d.input(Input::Button(Button::Left, true));
        assert!(d.begin_resize(a, edge::RIGHT | edge::BOTTOM));
        d.input(Input::PointerTo(340, 300));
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.rect(a), Some(Rect::new(130, 140, 350, 310)));
    }

    #[test]
    fn a_frame_given_and_taken_away_keeps_the_content_where_it_is() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.take_events();
        let content = d.content_rect(a).unwrap();
        d.set_framed(a, false, Keep::Content);
        assert_eq!((d.rect(a), d.content_rect(a)), (Some(content), Some(content)));
        assert_eq!(configures(d.take_events()), vec![(a, content, 0)]);
        d.set_framed(a, true, Keep::Content);
        assert_eq!((d.rect(a), d.content_rect(a)), (Some(Rect::new(100, 100, 400, 300)), Some(content)));
        // Keeping its rectangle instead: the content's what's inside the frame.
        d.set_framed(a, false, Keep::Rect);
        assert_eq!((d.rect(a), d.content_rect(a)), (Some(Rect::new(100, 100, 400, 300)), Some(Rect::new(100, 100, 400, 300))));
    }

    #[test]
    fn an_observers_first_take_is_everything_and_then_only_what_changed() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        let b = d.create(framed(200, 200, 100, 100));
        d.set_visible(a, true);
        let o = d.observe();
        let first = d.take_observed(o);
        assert_eq!(first.windows.iter().map(|w| w.id).collect::<Vec<_>>(), vec![a, b]);
        assert_eq!(first.stacking, Some(vec![a, b]));
        assert_eq!(first.damage.len(), 2);
        assert!(d.take_observed(o).is_empty());
        // Moved three times and retitled: one change, as it is now.
        for x in [110, 120, 130] {
            d.set_rect(a, Rect::new(x, 100, x + 300, 300));
        }
        d.set_title(a, "now");
        let next = d.take_observed(o);
        assert_eq!(next.windows.len(), 1);
        assert_eq!((next.windows[0].rect, next.windows[0].title.as_str()), (Rect::new(130, 100, 430, 300), "now"));
        assert!(next.stacking.is_none() && next.damage.is_empty());
        // Gone: in `gone`, nowhere else, and out of the stacking.
        d.set_title(b, "going");
        d.destroy(b);
        let next = d.take_observed(o);
        assert_eq!((next.gone, next.windows.len(), next.stacking), (vec![b], 0, Some(vec![a])));
    }

    #[test]
    fn content_damage_and_pixels_for_an_observer() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        let c = d.content_rect(a).unwrap();
        let canvas = Canvas::new(c.width(), c.height(), 0x101010);
        d.attach(a, Some(canvas.buffer()));
        d.commit(a);
        let o = d.observe();
        d.take_observed(o);
        // Two places drawn, apart: two rectangles. Then one overlapping the first: merged with it.
        for r in [Rect::new(0, 0, 10, 10), Rect::new(100, 100, 110, 110), Rect::new(5, 5, 20, 20)] {
            canvas.draw(|s| s.fill(&r, 0xff0000, &Rect::new(0, 0, 1000, 1000)));
            d.damage(a, r);
            d.commit(a);
        }
        let next = d.take_observed(o);
        let mut rects = next.damage[0].1.clone();
        rects.sort_by_key(|r| (r.left, r.top));
        assert_eq!(rects, vec![Rect::new(0, 0, 20, 20), Rect::new(100, 100, 110, 110)]);
        // Read back from the buffer, in place.
        let part = d.read(a, Rect::new(95, 95, 115, 115)).unwrap();
        assert_eq!((part.get(0, 0), part.get(5, 5)), (Some(0x101010), Some(0xff0000)));
    }

    #[test]
    fn observers_go_at_their_own_pace_and_are_woken_once() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        let (slow, fast) = (d.observe(), d.observe());
        d.take_observed(slow);
        d.take_observed(fast);
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let w = woken.clone();
        d.set_observer_waker(fast, Arc::new(move || {
            w.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        d.set_title(a, "one");
        d.set_title(a, "two");
        assert_eq!(woken.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(d.take_observed(fast).windows[0].title, "two");
        // The slow one still has it to take; the fast one, nothing.
        assert!(d.take_observed(fast).is_empty());
        assert_eq!(d.take_observed(slow).windows[0].title, "two");
        // Activation, as its own change.
        d.set_visible(a, true);
        d.activate(a);
        assert_eq!(d.take_observed(fast).active, Some(Some(a)));
    }

    #[test]
    fn enter_leave_and_capture() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 200, 200));
        let b = d.create(framed(400, 100, 200, 200));
        for id in [a, b] {
            d.set_visible(id, true);
        }
        let ca = d.content_rect(a).unwrap();
        d.input(Input::PointerTo(ca.left + 5, ca.top + 5));
        d.take_events();
        // Over to b: a's left, b's entered.
        let cb = d.content_rect(b).unwrap();
        d.input(Input::PointerTo(cb.left + 5, cb.top + 5));
        let events = d.take_events();
        assert!(matches!(events[..], [Event::Pointer { window: w1, what: Pointer::Leave, .. }, Event::Pointer { window: w2, what: Pointer::Enter, .. }] if w1 == a && w2 == b));
        // a captures: every pointer event is a's, a press over b neither activates b nor reaches it.
        assert_eq!(d.set_capture(Some(a)), None);
        d.take_events();
        d.input(Input::PointerTo(cb.left + 6, cb.top + 6));
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
        d.input(Input::Scroll { x: 0, y: -60 });
        let events = d.take_events();
        assert!(events.iter().all(|e| matches!(e, Event::Pointer { window, .. } if *window == a)));
        assert!(events.iter().any(|e| matches!(e, Event::Pointer { what: Pointer::Down(Button::Left), x, .. } if *x == cb.left + 6 - ca.left)));
        assert!(events.iter().any(|e| matches!(e, Event::Pointer { what: Pointer::Scroll { x: 0, y: -60 }, .. })));
        assert_ne!(d.active(), Some(b));
        // Let go: a's left, and b, under the pointer, entered; then it's b's.
        assert_eq!(d.set_capture(None), Some(a));
        let events = d.take_events();
        assert!(matches!(events[..], [Event::Pointer { window: w1, what: Pointer::Leave, .. }, Event::Pointer { window: w2, what: Pointer::Enter, .. }] if w1 == a && w2 == b));
        d.input(Input::PointerTo(cb.left + 7, cb.top + 7));
        assert!(d.take_events().iter().all(|e| matches!(e, Event::Pointer { window, .. } if *window == b)));
    }

    #[test]
    fn cursors_from_windows_and_frames() {
        use crate::cursor::{Cursor, Shape};
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        let o = d.observe();
        d.take_observed(o);
        d.set_cursor(a, Cursor::Shape(Shape::Text));
        let c = d.content_rect(a).unwrap();
        d.input(Input::PointerTo(c.left + 10, c.top + 10));
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::Text));
        assert_eq!(d.take_observed(o).cursor, Some(Cursor::Shape(Shape::Text)));
        // Its frame's right edge: the desktop's own; its corner, diagonal; off it all, the default.
        d.input(Input::PointerTo(399, 200));
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::EwResize));
        d.input(Input::PointerTo(399, 299));
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::NwseResize));
        d.input(Input::PointerTo(700, 500));
        assert_eq!(d.cursor(), &Cursor::default());
        // With the capture, its own wherever the pointer is.
        d.set_capture(Some(a));
        d.input(Input::PointerTo(701, 500));
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::Text));
    }

    #[test]
    fn text_and_an_input_methods_composing() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        d.set_visible(a, true);
        d.activate(a);
        let o = d.observe();
        d.take_observed(o);
        d.take_events();
        // Committed text goes to the active window, taking text input or not.
        d.input(Input::Text("café ☕".into()));
        assert_eq!(d.take_events(), vec![Event::Text { window: a, text: "café ☕".into() }]);
        // Composing only goes to a window that takes text input, which observers hear of, with where its cursor is.
        d.input(Input::Preedit { text: "ka".into(), cursor: Some((2, 2)) });
        assert!(d.take_events().is_empty());
        d.set_text_input(a, Some(Rect::new(40, 20, 41, 36)));
        assert_eq!(d.take_observed(o).text_input, Some(Some((a, Rect::new(40, 20, 41, 36)))));
        d.input(Input::Preedit { text: "か".into(), cursor: Some((3, 3)) });
        assert_eq!(d.take_events(), vec![Event::Preedit { window: a, text: "か".into(), cursor: Some((3, 3)) }]);
        // Not active any more: no window takes text.
        d.set_visible(a, false);
        assert_eq!(d.text_input(), None);
    }

    #[test]
    fn a_new_output_resizes_the_screen_and_what_fills_it() {
        let mut d = Desktop::new(800, 600);
        let a = d.create(framed(100, 100, 300, 200));
        let b = d.create(framed(50, 50, 200, 100));
        for id in [a, b] {
            d.set_visible(id, true);
        }
        d.set_maximized(a, true);
        let o = d.observe();
        d.take_observed(o);
        d.take_events();
        d.set_output(Output::headless(1024, 768, 240));
        assert_eq!(d.screen().width, 1024);
        let events = d.take_events();
        // The maximized one is the new size; each hears of the new scale.
        assert_eq!(configures(events.clone()), vec![(a, Rect::new(0, 0, 1024, 768), state::MAXIMIZED)]);
        assert!(events.contains(&Event::Scale { window: a, scale: 240 }) && events.contains(&Event::Scale { window: b, scale: 240 }));
        assert_eq!(d.rect(b), Some(Rect::new(50, 50, 250, 150)));
        assert_eq!(d.take_observed(o).output.map(|o| (o.width, o.scale)), Some((1024, 240)));
    }

    #[test]
    fn a_2x_buffer_is_shown_at_half_its_size() {
        let mut d = Desktop::new(400, 300);
        let a = d.create(Spec { rect: Rect::new(10, 10, 110, 60), fill: 0xffffff, ..Spec::default() });
        d.set_visible(a, true);
        // 200 by 100 pixels for a 100 by 50 window: its right half red.
        let canvas = Canvas::new(200, 100, 0x0000ff);
        canvas.draw(|s| s.fill(&Rect::new(100, 0, 200, 100), 0xff0000, &Rect::new(0, 0, 200, 100)));
        d.attach(a, Some(canvas.buffer()));
        d.set_buffer_scale(a, 2);
        d.commit(a);
        let screen = d.screen();
        assert_eq!((screen.get(10, 10), screen.get(59, 59), screen.get(60, 10), screen.get(109, 59)), (Some(0x0000ff), Some(0x0000ff), Some(0xff0000), Some(0xff0000)));
        // Damage in its pixels comes to the screen's.
        d.take_damage();
        d.damage(a, Rect::new(3, 3, 5, 5));
        d.commit(a);
        assert_eq!(d.take_damage(), Rect::new(11, 11, 13, 13));
    }

    #[test]
    fn the_clipboard_given_at_once_or_sent_when_asked() {
        use crate::selection::{Selection, TEXT};
        let mut d = Desktop::new(800, 600);
        let (a, b) = (d.create(framed(0, 0, 200, 200)), d.create(framed(300, 0, 200, 200)));
        let o = d.observe();
        d.take_observed(o);
        // a copies, giving text at once: a paste from outside has it straight away.
        let source = d.offer(Selection::Clipboard, Some(a), vec![]);
        assert!(d.offer_data(source, TEXT, b"hello".to_vec()));
        assert_eq!(d.selection(Selection::Clipboard), Some((Some(a), vec![TEXT.to_string()])));
        assert_eq!(d.take_observed(o).selections, vec![Selection::Clipboard]);
        let t = d.request(Selection::Clipboard, TEXT, None).unwrap();
        assert_eq!(d.take_transfer(t), Some(Some(b"hello".to_vec())));
        assert_eq!(d.request(Selection::Clipboard, "image/png", None), None);
        // b copies, as HTML it sends when asked: a hears its offer's gone.
        d.take_events();
        d.offer(Selection::Clipboard, Some(b), vec!["text/html".into()]);
        assert_eq!(d.take_events(), vec![Event::SelectionCancelled { window: a, selection: Selection::Clipboard }]);
        // a pastes: b's kernel is asked, and its answer comes to a.
        let t = d.request(Selection::Clipboard, "text/html", Some(a)).unwrap();
        assert_eq!(d.take_events(), vec![Event::SendSelection { window: b, selection: Selection::Clipboard, mime: "text/html".into(), transfer: t }]);
        d.provide(t, Some(b"<b>hi</b>".to_vec()));
        assert_eq!(d.take_events(), vec![Event::SelectionData { window: a, transfer: t, data: Some(b"<b>hi</b>".to_vec()) }]);
        // b goes, with what it sends when asked: the clipboard's empty.
        d.destroy(b);
        assert_eq!(d.selection(Selection::Clipboard), None);
    }

    #[test]
    fn what_was_given_at_once_outlives_its_owner_and_a_paste_waits_for_an_answer() {
        use crate::selection::{Selection, TEXT};
        let d = Desktop::shared(800, 600);
        let a = lock(&d).create(framed(0, 0, 200, 200));
        let source = lock(&d).offer(Selection::Primary, Some(a), vec![]);
        lock(&d).offer_data(source, TEXT, b"kept".to_vec());
        lock(&d).destroy(a);
        assert_eq!(lock(&d).selection(Selection::Primary), Some((None, vec![TEXT.to_string()])));
        // Another window offers what it sends when asked; a kernel answers on another thread.
        let b = lock(&d).create(framed(0, 0, 200, 200));
        lock(&d).offer(Selection::Clipboard, Some(b), vec![TEXT.into()]);
        let kernel = {
            let d = d.clone();
            std::thread::spawn(move || {
                for _ in 0..400 {
                    let events = lock(&d).take_events();
                    if let Some(Event::SendSelection { transfer, .. }) = events.into_iter().find(|e| matches!(e, Event::SendSelection { .. })) {
                        lock(&d).provide(transfer, Some(b"sent".to_vec()));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        assert_eq!(paste(&d, Selection::Clipboard, TEXT, Duration::from_secs(5)), Some(b"sent".to_vec()));
        kernel.join().unwrap();
    }

    fn shown(d: &mut Desktop, spec: Spec, client: ClientId) -> WindowId {
        let id = d.create(Spec { client, ..spec });
        d.set_visible(id, true);
        id
    }

    #[test]
    fn a_client_moves_the_focus_only_with_the_users_leave() {
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 200, 200), 1);
        d.activate(a);
        let b = shown(&mut d, framed(300, 0, 200, 200), 2);
        // Another client's window doesn't take the focus from what the user's in: it wants attention instead.
        assert!(!d.request_activation(b, Some(2), None));
        assert_eq!(d.active(), Some(a));
        assert!(d.attention(b) && d.window_info(b).unwrap().attention);
        assert_eq!(d.activation_token(Some(2)), None);
        // The active window's client hands it on with a token: granted, and good once.
        let t = d.activation_token(Some(1)).unwrap();
        assert!(d.request_activation(b, Some(2), Some(t)));
        assert_eq!(d.active(), Some(b));
        assert!(!d.attention(b));
        assert!(!d.request_activation(a, Some(1), Some(t)));
        assert!(d.attention(a));
        // A token the user's next key came after is spent.
        let late = d.activation_token(None).unwrap();
        d.input(Input::Key(keys::LETTERS[0], true));
        d.input(Input::Key(keys::LETTERS[0], false));
        assert!(!d.request_activation(a, Some(1), Some(late)));
        // The active window's client moves it between windows as it likes; then the one with the last key may too.
        assert!(d.request_activation(a, Some(2), None));
        assert_eq!(d.active(), Some(a));
        assert!(!d.attention(a));
        d.input(Input::Key(keys::LETTERS[1], true));
        d.input(Input::Key(keys::LETTERS[1], false));
        let c = shown(&mut d, framed(0, 300, 200, 200), 1);
        assert!(d.request_activation(c, Some(1), None));
        // From outside the machine (the host, a launcher), always.
        assert!(d.request_activation(b, None, None));
        assert_eq!(d.active(), Some(b));
    }

    #[test]
    fn a_held_key_repeats_without_being_new_input() {
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 200, 200), 1);
        d.activate(a);
        d.take_events();
        assert_eq!(d.key_repeat(), KeyRepeat { rate: 30, delay: 500 });
        d.set_key_repeat(KeyRepeat { rate: 25, delay: 600 });
        assert_eq!(d.key_repeat().delay, 600);
        let code = keys::LETTERS[2];
        d.input(Input::Key(code, true));
        let token = d.activation_token(None).unwrap();
        d.input(Input::Key(code, true));
        d.input(Input::Key(code, true));
        d.input(Input::Key(code, false));
        let repeats: Vec<(bool, bool)> = d.take_events().iter().filter_map(|e| if let Event::Key { down, repeat, .. } = e { Some((*down, *repeat)) } else { None }).collect();
        assert_eq!(repeats, vec![(true, false), (true, true), (true, true), (false, false)]);
        // Repeats aren't the user doing something new: a token from before them is still good.
        let b = shown(&mut d, framed(300, 0, 200, 200), 2);
        assert!(d.request_activation(b, Some(2), Some(token)));
    }

    #[test]
    fn a_locked_pointer_stays_put_and_a_confined_one_stays_inside() {
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(100, 100, 200, 200), 1);
        let b = shown(&mut d, framed(400, 100, 200, 200), 1);
        d.activate(a);
        d.input(Input::PointerTo(10, 10));
        d.set_relative_motion(a, true);
        // A lock is taken when the pointer comes into its content.
        d.set_pointer_constraint(a, Some(Constraint::Lock));
        assert_eq!(d.constrained(), None);
        d.take_events();
        let ca = d.content_rect(a).unwrap();
        d.input(Input::PointerTo(ca.left + 20, ca.top + 20));
        assert!(d.take_events().contains(&Event::Constrained { window: a, on: true }));
        assert_eq!(d.constrained(), Some((a, Constraint::Lock)));
        assert_eq!(d.cursor(), &Cursor::Hidden);
        // Then only relative motion: as the device saw it, added up until read, and the pointer doesn't move.
        let at = d.pointer();
        d.input(Input::PointerBy(5, -3));
        d.input(Input::PointerBy(1, 1));
        d.input(Input::PointerTo(700, 500));
        assert_eq!(d.pointer(), at);
        let events = d.take_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Pointer { window, what: Pointer::Relative { dx, dy }, .. } if window == a && dx == 6 + 700 - at.0 && dy == -2 + 500 - at.1));
        // Another window active: let go, and the cursor back.
        d.activate(b);
        assert!(d.take_events().contains(&Event::Constrained { window: a, on: false }));
        assert_ne!(d.cursor(), &Cursor::Hidden);
        // A confinement is taken at once, the pointer moved inside, and kept there.
        d.set_pointer_constraint(b, Some(Constraint::Confine(Rect::new(10, 10, 50, 50))));
        assert!(d.take_events().contains(&Event::Constrained { window: b, on: true }));
        let cb = d.content_rect(b).unwrap();
        let inside = Rect::new(cb.left + 10, cb.top + 10, cb.left + 50, cb.top + 50);
        assert!(inside.contains(d.pointer().0, d.pointer().1));
        d.input(Input::PointerTo(0, 0));
        assert_eq!(d.pointer(), (inside.left, inside.top));
        d.input(Input::PointerBy(1000, 1000));
        assert_eq!(d.pointer(), (inside.right - 1, inside.bottom - 1));
        // It moves with its window.
        let r = d.rect(b).unwrap();
        d.set_rect(b, r.offset(-100, 0));
        assert_eq!(d.pointer(), (inside.right - 101, inside.bottom - 1));
        d.take_events();
        d.set_pointer_constraint(b, None);
        assert_eq!(d.take_events(), vec![Event::Constrained { window: b, on: false }]);
        d.input(Input::PointerTo(0, 0));
        assert_eq!(d.pointer(), (0, 0));
        // Asked for still, and taken again when a is active and the pointer's in it.
        d.activate(a);
        d.input(Input::PointerTo(ca.left + 5, ca.top + 5));
        assert_eq!(d.constrained(), Some((a, Constraint::Lock)));
    }

    #[test]
    fn a_commit_hears_when_it_reached_the_screen() {
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 200, 200), 1);
        d.take_events();
        assert!(!d.wants_frame());
        d.request_presented(a);
        d.commit(a);
        assert!(d.wants_frame());
        d.frame(16);
        assert_eq!(d.take_events(), vec![Event::Presented { window: a, time: Some(16), seq: 1 }]);
        assert!(!d.wants_frame());
        // Replaced before a refresh: never shown.
        d.request_presented(a);
        d.commit(a);
        d.request_presented(a);
        d.commit(a);
        assert_eq!(d.take_events(), vec![Event::Presented { window: a, time: None, seq: 1 }]);
        d.frame(33);
        assert_eq!(d.take_events(), vec![Event::Presented { window: a, time: Some(33), seq: 2 }]);
        // Not on the screen: not presented.
        d.request_presented(a);
        d.commit(a);
        d.set_visible(a, false);
        d.take_events();
        d.frame(50);
        assert_eq!(d.take_events(), vec![Event::Presented { window: a, time: None, seq: 3 }]);
    }

    #[test]
    fn a_drag_carries_an_offer_to_the_window_that_takes_it() {
        use crate::selection::{Selection, TEXT};
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 300, 300), 1);
        let b = shown(&mut d, framed(400, 0, 300, 300), 2);
        let (ca, cb) = (d.content_rect(a).unwrap(), d.content_rect(b).unwrap());
        // No button held on it: no drag.
        assert_eq!(d.start_drag(Some(a), vec![TEXT.into()], action::COPY), None);
        d.input(Input::PointerTo(ca.left + 10, ca.top + 10));
        d.input(Input::Button(Button::Left, true));
        d.take_events();
        let source = d.start_drag(Some(a), vec![TEXT.into(), "text/html".into()], action::COPY | action::MOVE).unwrap();
        assert!(d.offer_data(source, "text/html", b"<b>hi</b>".to_vec()));
        let events = d.take_events();
        assert!(matches!(events[0], Event::Pointer { window, what: Pointer::Leave, .. } if window == a));
        assert!(matches!(&events[1], Event::DragEnter { window, x: 10, y: 10, actions: 3, .. } if *window == a));
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::NoDrop));
        // Over B: it hears it come, with what it carries.
        d.input(Input::PointerTo(cb.left + 20, cb.top + 30));
        let events = d.take_events();
        assert_eq!(events[0], Event::DragLeave { window: a });
        assert_eq!(events[1], Event::DragEnter { window: b, x: 20, y: 30, mimes: vec![TEXT.into(), "text/html".into()], actions: 3 });
        // B takes text, and would rather it moved: the source hears so, and the cursor says so. A isn't its target.
        d.accept_drag(a, Some(TEXT), action::COPY, 0);
        d.accept_drag(b, Some(TEXT), action::COPY | action::MOVE, action::MOVE);
        assert_eq!(d.take_events(), vec![Event::DragAction { window: a, mime: Some(TEXT.into()), action: action::MOVE }]);
        assert_eq!(d.cursor(), &Cursor::Shape(Shape::Move));
        // Something it doesn't carry isn't taken.
        d.accept_drag(b, Some("image/png"), action::COPY, 0);
        assert_eq!(d.drag().unwrap().action, 0);
        d.accept_drag(b, Some(TEXT), action::COPY | action::MOVE, action::LINK);
        assert_eq!(d.drag().unwrap().action, action::COPY);
        d.input(Input::PointerTo(cb.left + 21, cb.top + 30));
        d.input(Input::PointerTo(cb.left + 22, cb.top + 31));
        let events = d.take_events();
        assert_eq!(events.last(), Some(&Event::DragMotion { window: b, x: 22, y: 31 }));
        assert_eq!(events.iter().filter(|e| matches!(e, Event::DragMotion { .. })).count(), 1);
        // Dropped: B reads it from A's kernel, and says it's done; A hears it was copied. The pointer's B's again.
        d.input(Input::Button(Button::Left, false));
        let events = d.take_events();
        assert_eq!(events[..2], [Event::Drop { window: b, x: 22, y: 31, action: action::COPY }, Event::Dropped { window: a }]);
        assert!(matches!(events[2..], [Event::Pointer { window, what: Pointer::Enter, .. }] if window == b));
        let t = d.request(Selection::Drag, TEXT, Some(b)).unwrap();
        assert!(matches!(d.take_events()[..], [Event::SendSelection { window, selection: Selection::Drag, transfer, .. }] if window == a && transfer == t));
        d.provide(t, Some(b"hi".to_vec()));
        assert_eq!(d.take_events(), vec![Event::SelectionData { window: b, transfer: t, data: Some(b"hi".to_vec()) }]);
        let html = d.request(Selection::Drag, "text/html", Some(b)).unwrap();
        assert!(matches!(&d.take_events()[..], [Event::SelectionData { transfer, data: Some(h), .. }] if *transfer == html && h == b"<b>hi</b>"));
        d.finish_drop(b);
        assert_eq!(d.take_events(), vec![Event::DragEnd { window: a, action: action::COPY }]);
        assert_eq!(d.drag(), None);
        assert_eq!(d.selection(Selection::Drag), None);
    }

    #[test]
    fn a_drag_nothing_takes_or_escape_ends_with_nothing_done() {
        use crate::selection::TEXT;
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 300, 300), 1);
        let b = shown(&mut d, framed(400, 0, 300, 300), 2);
        let ca = d.content_rect(a).unwrap();
        let drag = |d: &mut Desktop| {
            d.input(Input::PointerTo(ca.left + 10, ca.top + 10));
            d.input(Input::Button(Button::Left, true));
            assert!(d.start_drag(Some(a), vec![TEXT.into()], action::COPY).is_some());
            assert_eq!(d.start_drag(None, vec![TEXT.into()], action::COPY), None);
        };
        // Dropped on the empty desktop.
        drag(&mut d);
        d.input(Input::PointerTo(780, 500));
        d.take_events();
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.take_events(), vec![Event::DragEnd { window: a, action: 0 }]);
        // Over B, which never takes it: dropped there, it leaves.
        drag(&mut d);
        d.input(Input::PointerTo(500, 100));
        d.take_events();
        d.input(Input::Button(Button::Left, false));
        assert_eq!(d.take_events()[..2], [Event::DragLeave { window: b }, Event::DragEnd { window: a, action: 0 }]);
        // Escape, with B taking it: cancelled, and the key goes nowhere.
        drag(&mut d);
        d.input(Input::PointerTo(500, 100));
        d.accept_drag(b, Some(TEXT), action::COPY, 0);
        d.take_events();
        d.input(Input::Key(keys::ESC, true));
        let events = d.take_events();
        assert_eq!(events[..2], [Event::DragLeave { window: b }, Event::DragEnd { window: a, action: 0 }]);
        assert!(!events.iter().any(|e| matches!(e, Event::Key { .. })));
        assert_eq!(d.drag(), None);
        d.input(Input::Button(Button::Left, false));
        // The source going cancels it; the target going leaves it with none.
        drag(&mut d);
        d.input(Input::PointerTo(500, 100));
        d.take_events();
        d.destroy(b);
        assert_eq!(d.drag().unwrap().target, None);
        d.destroy(a);
        assert_eq!(d.drag(), None);
    }

    #[test]
    fn files_dragged_in_from_outside_are_read_by_the_window_they_are_dropped_on() {
        use crate::selection::{Selection, URI_LIST, uri_list, uri_list_paths};
        let d = Desktop::shared(800, 600);
        let w = { let mut g = lock(&d); shown(&mut g, framed(100, 100, 300, 300), 1) };
        let c = lock(&d).content_rect(w).unwrap();
        // Its kernel, on another thread: takes files, reads them when they're dropped, and says it's done.
        let kernel = {
            let d = d.clone();
            std::thread::spawn(move || {
                let mut got = Vec::new();
                for _ in 0..800 {
                    let events = lock(&d).take_events();
                    for e in events {
                        let mut g = lock(&d);
                        match e {
                            Event::DragEnter { window, mimes, .. } if mimes.iter().any(|m| m == URI_LIST) => g.accept_drag(window, Some(URI_LIST), action::COPY, 0),
                            Event::Drop { window, .. } => {
                                g.request(Selection::Drag, URI_LIST, Some(window));
                            }
                            Event::SelectionData { window, data: Some(data), .. } => {
                                got = uri_list_paths(&data);
                                g.finish_drop(window);
                                return got;
                            }
                            _ => {}
                        }
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                got
            })
        };
        let files = uri_list(&["C:\\Users\\me\\a b.txt", "/home/me/ü.txt"]);
        let done = drop_from_outside(&d, c.left + 50, c.top + 50, vec![(URI_LIST.into(), files)], action::COPY, Duration::from_secs(5));
        assert_eq!(done, Ok(action::COPY));
        assert_eq!(kernel.join().unwrap(), vec!["C:/Users/me/a b.txt".to_string(), "/home/me/ü.txt".to_string()]);
        // Where no window is, nothing takes it.
        assert_eq!(drop_from_outside(&d, 700, 550, vec![(URI_LIST.into(), Vec::new())], action::COPY, Duration::from_secs(5)), Ok(0));
        assert_eq!(lock(&d).drag(), None);
    }

    #[test]
    fn file_uris_go_both_ways() {
        use crate::selection::{file_path, file_uri, uri_list_paths};
        assert_eq!(file_uri("C:\\a b\\c%.txt"), "file:///C:/a%20b/c%25.txt");
        assert_eq!(file_uri("/home/ü"), "file:///home/%C3%BC");
        assert_eq!(file_path("file:///C:/a%20b/c%25.txt").as_deref(), Some("C:/a b/c%.txt"));
        assert_eq!(file_path("file://localhost/etc/hosts").as_deref(), Some("/etc/hosts"));
        assert_eq!(file_path("file://elsewhere/x"), None);
        assert_eq!(file_path("https://example.com/"), None);
        assert_eq!(uri_list_paths(b"# a comment\r\nfile:///tmp/a\r\nhttps://x/\r\n\r\nfile:///tmp/b\n"), vec!["/tmp/a", "/tmp/b"]);
    }

    #[test]
    fn stacking_keeps_its_rules_whatever_is_asked() {
        let mut d = Desktop::new(800, 600);
        let below = shown(&mut d, Spec { layer: Layer::Below, ..framed(0, 0, 800, 600) }, 1);
        let top = shown(&mut d, Spec { layer: Layer::Top, ..framed(500, 0, 200, 200) }, 1);
        let a = shown(&mut d, framed(0, 0, 300, 300), 1);
        let dialog = shown(&mut d, Spec { role: Role::Dialog { parent: a }, ..framed(50, 50, 100, 100) }, 1);
        let b = shown(&mut d, framed(100, 100, 300, 300), 1);
        let pos = |d: &Desktop, id| d.stacking().iter().position(|&o| o == id).unwrap();
        // Bands in order; the desktop's own at the bottom, however it's raised.
        d.raise(below);
        assert_eq!(d.stacking(), &[below, a, dialog, b, top]);
        // Raising the owner brings what it owns along, above it.
        d.activate(a);
        assert_eq!(d.stacking(), &[below, b, a, dialog, top]);
        // Lowered, with its dialog; then just below B, and just above it.
        d.stack(a, Stack::Bottom);
        assert_eq!(d.stacking(), &[below, a, dialog, b, top]);
        d.stack(b, Stack::Below(a));
        assert_eq!(d.stacking(), &[below, b, a, dialog, top]);
        d.stack(b, Stack::Above(dialog));
        assert_eq!(d.stacking(), &[below, a, dialog, b, top]);
        // Its own dialog isn't a place to put it.
        d.stack(a, Stack::Above(dialog));
        assert_eq!(d.stacking(), &[below, a, dialog, b, top]);
        // A dialog can't go below its owner.
        d.stack(dialog, Stack::Bottom);
        assert!(pos(&d, dialog) > pos(&d, a));
        // A topmost window's dialog is topmost too.
        d.set_layer(a, Layer::Top);
        assert!(pos(&d, dialog) > pos(&d, a) && pos(&d, dialog) > pos(&d, b) && pos(&d, a) > pos(&d, b));
        d.set_layer(a, Layer::Normal);
        // A fullscreen window is above even the topmost while it's active, with its dialog above it; not when it isn't.
        d.set_fullscreen(a, true);
        d.activate(a);
        assert_eq!(&d.stacking()[2..], &[top, a, dialog]);
        d.activate(dialog);
        assert_eq!(&d.stacking()[2..], &[top, a, dialog]);
        d.activate(b);
        assert_eq!(d.stacking().last(), Some(&top));
        assert!(pos(&d, b) > pos(&d, a));
        d.set_fullscreen(a, false);
        // A window a click doesn't activate: the press is its, but B stays active, and above it.
        let n = shown(&mut d, Spec { no_activate: true, ..framed(600, 400, 150, 150) }, 1);
        d.stack(n, Stack::Bottom);
        d.activate(b);
        let cn = d.content_rect(n).unwrap();
        d.input(Input::PointerTo(cn.left + 5, cn.top + 5));
        d.take_events();
        d.input(Input::Button(Button::Left, true));
        d.input(Input::Button(Button::Left, false));
        assert!(d.take_events().iter().any(|e| matches!(e, Event::Pointer { window, what: Pointer::Down(Button::Left), .. } if *window == n)));
        assert_eq!(d.active(), Some(b));
        assert!(pos(&d, n) < pos(&d, b));
        // Nor is it made active when the active window goes.
        d.stack(n, Stack::Top);
        d.destroy(b);
        assert_ne!(d.active(), Some(n));
        assert!(d.active().is_some());
    }

    #[test]
    fn each_observer_hears_of_the_screen_changing_for_itself() {
        let mut d = Desktop::new(400, 300);
        let (host, shots) = (d.observe(), d.observe());
        // A new observer has the whole screen to show.
        assert_eq!(d.take_observed(host).screen, Rect::new(0, 0, 400, 300));
        let a = shown(&mut d, framed(10, 10, 100, 100), 1);
        // One taking it, and the screen composited, leaves the other's to take.
        assert_eq!(d.take_observed(shots).screen, Rect::new(0, 0, 400, 300));
        d.screen();
        assert_eq!(d.take_observed(host).screen, Rect::new(10, 10, 110, 110));
        assert!(d.take_observed(host).screen.is_empty());
        d.set_rect(a, Rect::new(200, 100, 300, 200));
        assert_eq!(d.take_observed(shots).screen, Rect::new(10, 10, 300, 200));
    }

    #[test]
    fn at_2x_a_2x_buffer_is_sharp_and_the_rest_is_magnified() {
        let mut d = Desktop::new(100, 80);
        // At 1×, the screen's own pixels are the logical screen's.
        assert_eq!(d.screen_pixels().width, 100);
        d.set_output(Output::headless(100, 80, 240));
        // A 2× window: 40 by 30, 80 by 60 pixels in stripes a pixel wide; and a 1× one above part of it.
        let sharp = d.create(Spec { rect: Rect::new(10, 10, 50, 40), fill: 0xffffff, ..Spec::default() });
        d.set_visible(sharp, true);
        let stripes = Canvas::new(80, 60, 0x0000ff);
        stripes.draw(|s| {
            for x in (0..80).step_by(2) {
                s.fill(&Rect::new(x, 0, x + 1, 60), 0xff0000, &Rect::new(0, 0, 80, 60));
            }
        });
        d.attach(sharp, Some(stripes.buffer()));
        d.set_buffer_scale(sharp, 2);
        d.commit(sharp);
        let blur = d.create(Spec { rect: Rect::new(40, 30, 60, 50), fill: 0x00ff00, ..Spec::default() });
        d.set_visible(blur, true);
        let px = |d: &mut Desktop, x: i32, y: i32| {
            let s = d.screen_pixels();
            s.pixels[(y * s.width + x) as usize]
        };
        assert_eq!((d.screen_pixels().width, d.screen_pixels().height), (200, 160));
        // The 2× window pixel for pixel: red, blue, red; where the logical screen has only every other one.
        assert_eq!([px(&mut d, 20, 21), px(&mut d, 21, 21), px(&mut d, 22, 21)], [0xff0000, 0x0000ff, 0xff0000]);
        let logical = d.screen().pixels[(10 * 100 + 10) as usize];
        assert_eq!([px(&mut d, 20, 20), px(&mut d, 21, 20)][0], logical);
        // The 1× window above it, magnified: two by two of its green; and the background, magnified.
        assert_eq!([px(&mut d, 80, 60), px(&mut d, 81, 61), px(&mut d, 98, 78)], [0x00ff00; 3]);
        assert_eq!(px(&mut d, 0, 0), BACKGROUND);
        // Where the 1× window covers the 2× one, it's the 1× window's.
        assert_eq!(px(&mut d, 85, 65), 0x00ff00);
        // Moved off it, the 2× window's sharp there again.
        d.set_rect(blur, Rect::new(70, 50, 90, 70));
        assert_eq!([px(&mut d, 84, 65), px(&mut d, 85, 65)], [0xff0000, 0x0000ff]);
    }

    #[test]
    fn a_window_s_own_pixels_are_at_the_output_s_scale() {
        let mut d = Desktop::new(200, 200);
        d.set_output(Output::headless(200, 200, 240));
        let a = d.create(Spec { rect: Rect::new(10, 10, 30, 20), fill: 0x00ff00, ..Spec::default() });
        // A 2× buffer, shown as it is; half of it covered, so the fill shows on the right.
        let canvas = Canvas::new(20, 20, 0x0000ff);
        canvas.draw(|s| s.fill(&Rect::new(1, 0, 2, 20), 0xff0000, &Rect::new(0, 0, 20, 20)));
        d.attach(a, Some(canvas.buffer()));
        d.set_buffer_scale(a, 2);
        d.commit(a);
        let p = d.read_pixels(a).unwrap();
        assert_eq!((p.width, p.height), (40, 20));
        assert_eq!(&p.pixels[0..3], &[0x0000ff, 0xff0000, 0x0000ff]);
        assert_eq!(p.pixels[25], 0x00ff00);
        // At 1×, a 1× buffer's pixels.
        d.set_output(Output::headless(200, 200, 120));
        d.set_buffer_scale(a, 1);
        d.commit(a);
        let p = d.read_pixels(a).unwrap();
        assert_eq!((p.width, p.height, p.pixels[1]), (20, 10, 0xff0000));
        // Placed by the host: moved, and told.
        d.take_events();
        d.placed(a, Rect::new(50, 60, 70, 70));
        assert_eq!(d.rect(a), Some(Rect::new(50, 60, 70, 70)));
        assert!(matches!(d.take_events()[..], [Event::Configure { window, rect, .. }] if window == a && rect == Rect::new(50, 60, 70, 70)));
        d.placed(a, Rect::new(50, 60, 70, 70));
        assert!(d.take_events().is_empty());
    }

    #[test]
    fn a_window_s_owner_changes_at_any_time() {
        let mut d = Desktop::new(800, 600);
        let a = shown(&mut d, framed(0, 0, 300, 300), 1);
        let b = shown(&mut d, framed(100, 100, 300, 300), 1);
        d.activate(b);
        d.activate(a);
        assert_eq!(d.stacking().last(), Some(&a));
        // B becomes A's dialog: above it, whatever's active.
        d.set_owner(b, Some(a));
        assert_eq!(d.stacking().last(), Some(&b));
        assert_eq!(d.window_info(b).unwrap().parent, Some(a));
        // Not its own owner's owner.
        d.set_owner(a, Some(b));
        assert_eq!(d.window_info(a).unwrap().parent, None);
        d.set_owner(b, None);
        d.activate(a);
        assert_eq!(d.stacking().last(), Some(&a));
    }
}
