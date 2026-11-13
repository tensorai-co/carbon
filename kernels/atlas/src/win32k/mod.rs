//! win32k: windows and messages, as Windows has them in its kernel. Window classes and windows, each thread's message
//! queue, sending (a `WndProc` called by upcall, on the thread that owns the window) and posting, timers, painting,
//! input, and GDI (`gdi.rs`), which draws host-side into each top-level window's pixels.
//!
//! Top-level windows are on the machine's desktop (`display::Desktop`), the window manager both kernels share: it has
//! their pixels, draws their frames, stacks them, decides which is active, and routes the pointer and keys to them.
//! win32k keeps what Windows says about them (window and client rectangles, child windows, focus) and turns the
//! desktop's events into messages (`pump`).

pub mod gdi;
pub mod keys;
mod paint;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;
use kernels_core::wait::{Waiter, Woke};
use display::desktop::edge::{BOTTOM, LEFT, RIGHT, TOP};
use display::desktop::{self as dt, Button, Event, Insets, Keep, Layer, Pointer, Role, Spec};
use display::{Rect, Surface};

use crate::os::Reply;
use crate::process::Task;

pub use gdi::Gdi;

pub const WM_CREATE: u32 = 0x1;
pub const WM_DESTROY: u32 = 0x2;
pub const WM_MOVE: u32 = 0x3;
pub const WM_SIZE: u32 = 0x5;
pub const WM_ACTIVATE: u32 = 0x6;
pub const WM_SETFOCUS: u32 = 0x7;
pub const WM_KILLFOCUS: u32 = 0x8;
pub const WM_SETTEXT: u32 = 0xc;
pub const WM_GETTEXT: u32 = 0xd;
pub const WM_GETTEXTLENGTH: u32 = 0xe;
pub const WM_PAINT: u32 = 0xf;
pub const WM_CLOSE: u32 = 0x10;
pub const WM_QUIT: u32 = 0x12;
pub const WM_ERASEBKGND: u32 = 0x14;
pub const WM_SHOWWINDOW: u32 = 0x18;
pub const WM_NCCREATE: u32 = 0x81;
pub const WM_NCDESTROY: u32 = 0x82;
pub const WM_KEYDOWN: u32 = 0x100;
pub const WM_KEYUP: u32 = 0x101;
pub const WM_CHAR: u32 = 0x102;
pub const WM_SYSKEYDOWN: u32 = 0x104;
pub const WM_SYSKEYUP: u32 = 0x105;
pub const WM_SYSCHAR: u32 = 0x106;
pub const WM_DEADCHAR: u32 = 0x103;
pub const WM_SYSDEADCHAR: u32 = 0x107;
pub const WM_SYSCOMMAND: u32 = 0x112;
pub const WM_NCLBUTTONDOWN: u32 = 0xa1;
pub const WM_TIMER: u32 = 0x113;
pub const WM_MOUSEMOVE: u32 = 0x200;
pub const WM_MOUSEWHEEL: u32 = 0x20a;
pub const SC_CLOSE: u64 = 0xf060;
const WA_INACTIVE: u64 = 0;
const WA_ACTIVE: u64 = 1;

/// The desktop: every top-level window's parent.
pub const DESKTOP: u64 = 0x10010;
pub const SCREEN: (i32, i32) = (1280, 800);
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_CAPTION: u32 = 0x00c0_0000;
pub const WS_EX_TOPMOST: u32 = 0x8;
/// The DPI a program's told: 96, as Windows tells a program that isn't DPI aware, whatever the output's scale. Its
/// windows are drawn at 1× and magnified to the output's (`Desktop::screen_pixels`), as Windows virtualizes them. A
/// program that says it's aware (`SetProcessDPIAware`, its manifest) would be told the output's, and draw at it: not
/// yet.
pub const PROGRAM_DPI: u32 = 96;

/// A click neither activates nor raises it, and it isn't made active when another goes.
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const CW_USEDEFAULT: i32 = 0x8000_0000_u32 as i32;
pub const SW_SHOW: u32 = 5;
const SW_SHOWNA: u32 = 8;
pub const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
pub const ERROR_CANNOT_FIND_WND_CLASS: u32 = 1407;
pub const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

#[derive(Debug, Clone)]
pub struct Class {
    pub name: String,
    pub atom: u16,
    pub wndproc: u64,
    pub style: u32,
    pub extra: usize,
    /// `hbrBackground`: a brush, or a system colour's index plus one.
    pub background: u64,
    pub instance: u64,
    /// `hCursor`: what the pointer looks like over its windows' client areas, or 0 for what the program sets.
    pub cursor: u64,
    /// Registered with `RegisterClass(Ex)A`: its windows' `WndProc` takes text in the ANSI code page.
    pub ansi: bool,
}

pub struct Window {
    pub class: String,
    pub wndproc: u64,
    pub parent: u64,
    pub tid: u64,
    pub pid: u64,
    pub style: u32,
    pub ex_style: u32,
    /// Relative to its parent's client area; a top-level window's is on the screen, frame and all.
    pub rect: Rect,
    pub text: String,
    pub visible: bool,
    /// What needs painting, in client coordinates, and whether its background needs erasing.
    pub invalid: Rect,
    pub erase: bool,
    pub userdata: u64,
    pub id: u64,
    pub instance: u64,
    pub extra: Vec<u8>,
    /// A top-level window's place on the desktop, which has its pixels: 0 for a child window.
    pub shown: u64,
    /// The top-level window this one belongs to, if it has one.
    pub owner: u64,
    /// Whether the desktop draws it a frame.
    pub framed: bool,
    pub destroying: bool,
    /// WM_SIZE's kind as it was last sent: `SIZE_RESTORED`, `SIZE_MINIMIZED` or `SIZE_MAXIMIZED`.
    pub size_kind: u64,
    /// Its `WndProc` takes ANSI text (`IsWindowUnicode` is FALSE): its class's was registered with the `*A`
    /// functions, or `SetWindowLongPtrA` gave it its `WndProc`. Messages with text are converted on the way in, and
    /// what it gives back on the way out.
    pub ansi: bool,
}

/// The clipboard, as win32k keeps it.
#[derive(Default)]
pub struct Clipboard {
    /// The window that has it open (`OPENED_BY_NONE` if opened with none), or None.
    pub open: Option<u64>,
    /// The offer it made last, and the window that emptied it then.
    pub source: Option<display::selection::SourceId>,
    pub owner: u64,
    /// The global blocks `GetClipboardData` gave, which are the clipboard's: freed when it's emptied.
    pub handles: Vec<u64>,
    /// Registered formats, by name (in lower case), from 0xC000.
    pub formats: BTreeMap<String, u32>,
    /// The window that emptied it, by its top-level window's place on the desktop: who WM_DESTROYCLIPBOARD goes to
    /// when the desktop says that window's offer was taken.
    pub owners: BTreeMap<u64, u64>,
}

/// `OpenClipboard(NULL)`'s opener.
pub const OPENED_BY_NONE: u64 = u64::MAX;

/// The desktop's cursor for a cursor handle: `LoadCursorW`'s, whose low word is its `IDC_*`; 0 hides it.
pub fn cursor_of(handle: u64) -> display::cursor::Cursor {
    use display::cursor::{Cursor, Shape};
    if handle == 0 {
        return Cursor::Hidden;
    }
    Cursor::Shape(match handle & 0xffff {
        32513 => Shape::Text,
        32514 => Shape::Wait,
        32515 => Shape::Crosshair,
        32642 => Shape::NwseResize,
        32643 => Shape::NeswResize,
        32644 => Shape::EwResize,
        32645 => Shape::NsResize,
        32646 => Shape::Move,
        32648 => Shape::NotAllowed,
        32649 => Shape::Pointer,
        32650 => Shape::Progress,
        32651 => Shape::Help,
        _ => Shape::Default,
    })
}

/// Whether a window with this style has a frame: a top-level window with a caption. Windows gives every overlapped
/// window (not a child, not a popup) one.
pub fn framed(style: u32) -> bool {
    style & WS_CHILD == 0 && (style & WS_POPUP == 0 || style & WS_CAPTION == WS_CAPTION)
}

/// Two 16-bit words in an LPARAM: `x` low, `y` high.
pub fn words(x: i32, y: i32) -> u64 {
    (y as u64 & 0xffff) << 16 | (x as u64 & 0xffff)
}

impl Window {
    pub fn insets(&self) -> Insets {
        Insets::of(self.framed)
    }

    /// Its client area, in its own coordinates.
    pub fn client(&self) -> Rect {
        let c = self.insets().inside(&self.rect);
        Rect::new(0, 0, c.width(), c.height())
    }

    pub fn top_level(&self) -> bool {
        self.parent == DESKTOP
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Msg {
    pub hwnd: u64,
    pub message: u32,
    pub wparam: u64,
    pub lparam: u64,
    pub time: u32,
}

impl Msg {
    /// MSG, 64-bit: 48 bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; 48];
        b[0..8].copy_from_slice(&self.hwnd.to_le_bytes());
        b[8..12].copy_from_slice(&self.message.to_le_bytes());
        b[16..24].copy_from_slice(&self.wparam.to_le_bytes());
        b[24..32].copy_from_slice(&self.lparam.to_le_bytes());
        b[32..36].copy_from_slice(&self.time.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Msg {
        let at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default());
        Msg { hwnd: at(0), message: at(8) as u32, wparam: at(16), lparam: at(24), time: at(32) as u32 }
    }
}

/// A message sent from another thread, and where its result goes.
pub struct Sent {
    pub msg: Msg,
    pub reply: Arc<SentReply>,
}

pub struct SentReply {
    value: Mutex<Option<u64>>,
    sender: Arc<Waiter>,
}

struct Timer {
    hwnd: u64,
    id: u64,
    period: Duration,
    next: Instant,
    function: u64,
}

/// One thread's messages.
pub struct Queue {
    posted: VecDeque<Msg>,
    sent: VecDeque<Sent>,
    quit: Option<u64>,
    timers: Vec<Timer>,
    waiter: Arc<Waiter>,
    /// The keys and buttons as of the input messages this thread has taken: what `GetKeyState` says.
    pub keys: [u8; 256],
}

/// Which messages `GetMessage` and `PeekMessage` want.
#[derive(Debug, Clone, Copy)]
pub struct Filter {
    pub hwnd: u64,
    pub min: u32,
    pub max: u32,
}

impl Filter {
    fn wants(&self, m: &Msg, windows: &BTreeMap<u64, Window>) -> bool {
        let hwnd_ok = match self.hwnd {
            0 => true,
            // -1: only thread messages.
            u64::MAX => m.hwnd == 0,
            h => m.hwnd == h || is_descendant(windows, m.hwnd, h),
        };
        hwnd_ok && self.message(m.message)
    }

    fn message(&self, message: u32) -> bool {
        (self.min == 0 && self.max == 0) || (self.min..=self.max).contains(&message)
    }
}

/// A key in a key state, down or up: each press toggles it too.
fn set_key(keys: &mut [u8; 256], vk: u8, down: bool) {
    let state = &mut keys[vk as usize];
    if down {
        if *state & 0x80 == 0 {
            *state ^= 1;
        }
        *state |= 0x80;
    } else {
        *state &= !0x80;
    }
}

/// What taking an input message says of the keys and buttons, for the thread's key state.
fn took(keys: &mut [u8; 256], m: &Msg) {
    match m.message {
        WM_KEYDOWN | WM_SYSKEYDOWN => set_key(keys, m.wparam as u8, true),
        WM_KEYUP | WM_SYSKEYUP => set_key(keys, m.wparam as u8, false),
        0x200..=0x20e => {
            for (mk, vk) in [(1, keys::VK_LBUTTON), (2, keys::VK_RBUTTON), (0x10, keys::VK_MBUTTON)] {
                set_key(keys, vk, m.wparam & mk != 0);
            }
            set_key(keys, keys::VK_SHIFT, m.wparam & 4 != 0);
            set_key(keys, keys::VK_CONTROL, m.wparam & 8 != 0);
        }
        _ => {}
    }
}

fn is_descendant(windows: &BTreeMap<u64, Window>, mut hwnd: u64, ancestor: u64) -> bool {
    while let Some(w) = windows.get(&hwnd) {
        if w.parent == ancestor {
            return true;
        }
        hwnd = w.parent;
    }
    false
}

/// Windows and messages, for the whole machine.
pub struct Win32k {
    pub windows: BTreeMap<u64, Window>,
    classes: HashMap<String, Class>,
    next_hwnd: u64,
    next_atom: u16,
    queues: HashMap<u64, Queue>,
    pub gdi: Gdi,
    /// Top-level windows' last pictures, as they were destroyed, for screenshots.
    pub shots: Vec<(String, Surface)>,
    /// The desktop the machine's top-level windows are on.
    pub display: dt::Shared,
    /// Each top-level window's pixels, by its place on the desktop: GDI draws into them, and the desktop shows them
    /// in place, as each drawing commits.
    pub canvases: BTreeMap<u64, display::Canvas>,
    /// The active window, as the desktop last said, and the window keys go to.
    pub active: u64,
    pub focus: u64,
    /// The window that has the mouse (`SetCapture`): every mouse message is its, wherever the pointer is.
    pub capture: u64,
    /// The cursor `SetCursor` (or a class's) set last.
    pub cursor: u64,
    /// Files dropped on its windows: the drops whose files it's reading (by the desktop's transfer: the window to tell,
    /// and where in its client area), and each `HDROP` given in a WM_DROPFILES until `DragFinish`.
    pub drops: BTreeMap<u64, (u64, i32, i32)>,
    pub hdrops: BTreeMap<u64, DropFiles>,
    /// The window a drag carrying files is over now.
    pub drag_files: Option<u64>,
    /// `ClipCursor`'s rectangle on the screen, and the window on the desktop it confines the pointer for: the active
    /// one when it was set.
    pub clip: Option<(u64, Rect)>,
    /// The clipboard as Windows has it: the window that opened it (or `OPENED_BY_NONE` for none), the offer this
    /// machine's windows made of it last and its owner, the handles `GetClipboardData` gave (freed when it's emptied),
    /// and the formats `RegisterClipboardFormatW` named.
    pub clipboard: Clipboard,
    /// Each virtual key's state: 0x80 while it's down, 0x01 toggled by each press.
    pub keys: [u8; 256],
}

/// What an `HDROP` holds: the files dropped (Windows paths), and where in the window's client area. It's win32k's own
/// handle, read by shell32's `DragQueryFileW` and `DragQueryPoint`; not global memory a program can lock.
#[derive(Debug, Clone)]
pub struct DropFiles {
    pub files: Vec<String>,
    pub point: (i32, i32),
}

/// `WS_EX_ACCEPTFILES`: the window takes files dropped on it, as WM_DROPFILES (`DragAcceptFiles`).
pub const WS_EX_ACCEPTFILES: u32 = 0x10;
const WM_DROPFILES: u32 = 0x233;

impl Win32k {
    /// A window's children, or (for `DESKTOP`) the top-level windows, top first: the top-level windows as the desktop
    /// stacks them, children with the later made above (as `child_at` finds them).
    pub fn z_order(&self, parent: u64) -> Vec<u64> {
        if parent == DESKTOP {
            let stacking = dt::lock(&self.display).stacking().to_vec();
            let mut top: Vec<u64> = stacking.iter().rev().filter_map(|&s| self.hwnd_of(s)).collect();
            // Any the desktop doesn't know (none, today), last.
            let unknown: Vec<u64> = self.windows.iter().filter(|(h, w)| w.top_level() && !top.contains(h)).map(|(&h, _)| h).collect();
            top.extend(unknown);
            top
        } else {
            self.windows.iter().rev().filter(|(_, w)| w.parent == parent).map(|(&h, _)| h).collect()
        }
    }

    /// The window files dropped at a point in a top-level window's client area would go to: the deepest window there
    /// that takes them, or its nearest ancestor that does; with the point in its own client coordinates.
    fn drop_target(&self, top: u64, x: i32, y: i32) -> Option<(u64, i32, i32)> {
        let (sx, sy) = self.screen_origin(top)?;
        let (mut hwnd, _, _) = self.child_at(top, x, y);
        loop {
            let w = self.windows.get(&hwnd)?;
            if w.ex_style & WS_EX_ACCEPTFILES != 0 {
                let (ox, oy) = self.screen_origin(hwnd)?;
                return Some((hwnd, sx + x - ox, sy + y - oy));
            }
            if w.top_level() {
                return None;
            }
            hwnd = w.parent;
        }
    }

    /// Takes a drag over one of its windows as files, if it carries them and there's a window there that takes them.
    fn take_drag(&mut self, window: u64, x: i32, y: i32, files: bool) {
        use display::desktop::action;
        let target = self.hwnd_of(window).filter(|_| files).and_then(|top| self.drop_target(top, x, y));
        let mut d = dt::lock(&self.display);
        match target {
            Some(_) => d.accept_drag(window, Some(display::selection::URI_LIST), action::COPY, action::COPY),
            None => d.accept_drag(window, None, 0, 0),
        }
    }

    /// A top-level window's canvas, made the size of its content again after it moved or was resized, and shown.
    pub fn fit(&mut self, shown: u64) {
        let Some(canvas) = self.canvases.get(&shown) else { return };
        let mut d = dt::lock(&self.display);
        let (Some(content), Some(fill)) = (d.content_rect(shown), d.fill(shown)) else { return };
        // The canvas's lock under the desktop's is the order the desktop composites in.
        if canvas.resize(content.width(), content.height(), fill) {
            d.attach(shown, Some(canvas.buffer()));
            d.commit(shown);
        }
    }

    pub fn new(display: dt::Shared) -> Win32k {
        Win32k {
            windows: BTreeMap::new(),
            classes: HashMap::new(),
            next_hwnd: 0,
            next_atom: 0,
            queues: HashMap::new(),
            gdi: Gdi::default(),
            shots: Vec::new(),
            display,
            canvases: BTreeMap::new(),
            active: 0,
            focus: 0,
            capture: 0,
            cursor: 0,
            clip: None,
            drops: BTreeMap::new(),
            hdrops: BTreeMap::new(),
            drag_files: None,
            clipboard: Clipboard::default(),
            keys: [0; 256],
        }
    }

    /// Wakes every thread with a message queue, to look at the desktop's events.
    pub fn wake_all(&self) {
        for q in self.queues.values() {
            q.waiter.wake();
        }
    }

    /// The window at a place on the desktop.
    pub fn hwnd_of(&self, shown: u64) -> Option<u64> {
        self.windows.iter().find(|(_, w)| w.shown == shown).map(|(&h, _)| h)
    }

    /// The top-level window a window is in.
    pub fn top_level_of(&self, mut hwnd: u64) -> Option<u64> {
        loop {
            let w = self.windows.get(&hwnd)?;
            if w.top_level() {
                return Some(hwnd);
            }
            hwnd = w.parent;
        }
    }

    /// Where a window's client area is on the screen.
    pub fn screen_origin(&self, hwnd: u64) -> Option<(i32, i32)> {
        let w = self.windows.get(&hwnd)?;
        if w.top_level() {
            let c = w.insets().inside(&w.rect);
            return Some((c.left, c.top));
        }
        let (x, y) = self.screen_origin(w.parent)?;
        Some((x + w.rect.left, y + w.rect.top))
    }

    /// What WM_MOVE says of a window: its client area on the screen (a top-level window), or in its parent's.
    fn move_origin(&self, hwnd: u64) -> (i32, i32) {
        match self.windows.get(&hwnd) {
            Some(w) if !w.top_level() => (w.rect.left, w.rect.top),
            _ => self.screen_origin(hwnd).unwrap_or_default(),
        }
    }

    /// The deepest shown child window at a point in a window's client area, and the point in that child's.
    pub fn child_at(&self, mut hwnd: u64, mut x: i32, mut y: i32) -> (u64, i32, i32) {
        loop {
            // Later windows are above earlier ones.
            let next = self.windows.iter().rev().find(|(_, w)| w.parent == hwnd && w.visible && w.rect.contains(x, y)).map(|(&h, _)| h);
            let Some(child) = next else { return (hwnd, x, y) };
            let r = self.windows[&child].rect;
            (x, y, hwnd) = (x - r.left, y - r.top, child);
        }
    }

    /// A message from the system, sent to its window's thread: dealt with the next time that thread looks at its
    /// messages, before anything posted, with no one waiting for its result.
    fn deliver(&mut self, hwnd: u64, message: u32, wparam: u64, lparam: u64) {
        let Some(tid) = self.windows.get(&hwnd).map(|w| w.tid) else { return };
        let Some(q) = self.queues.get_mut(&tid) else { return };
        let reply = Arc::new(SentReply { value: Mutex::new(None), sender: Waiter::new() });
        q.sent.push_back(Sent { msg: Msg { hwnd, message, wparam, lparam, time: 0 }, reply });
        q.waiter.wake();
    }

    /// Input, posted to its window's thread. A move the thread hasn't taken yet is replaced by the next, as Windows
    /// does.
    fn post_input(&mut self, hwnd: u64, message: u32, wparam: u64, lparam: u64, time: u32) {
        let Some(tid) = self.windows.get(&hwnd).map(|w| w.tid) else { return };
        let Some(q) = self.queues.get_mut(&tid) else { return };
        let msg = Msg { hwnd, message, wparam, lparam, time };
        match q.posted.back_mut() {
            Some(last) if message == WM_MOUSEMOVE && last.message == WM_MOUSEMOVE && last.hwnd == hwnd => *last = msg,
            _ => q.posted.push_back(msg),
        }
        q.waiter.wake();
    }

    fn set_key(&mut self, vk: u8, down: bool) {
        set_key(&mut self.keys, vk, down);
    }

    /// Posts a message ahead of everything else its window's thread has: `TranslateMessage`'s characters come next.
    pub fn post_front(&mut self, hwnd: u64, message: u32, wparam: u64, lparam: u64, time: u32) {
        let Some(tid) = self.windows.get(&hwnd).map(|w| w.tid) else { return };
        if let Some(q) = self.queues.get_mut(&tid) {
            q.posted.push_front(Msg { hwnd, message, wparam, lparam, time });
        }
    }

    /// A thread's key state, as of the input messages it's taken.
    pub fn thread_keys(&self, tid: u64) -> [u8; 256] {
        self.queues.get(&tid).map_or([0; 256], |q| q.keys)
    }

    /// One of the desktop's events, as the messages Windows has for it.
    fn route(&mut self, event: Event, time: u32) {
        match event {
            Event::Configure { window, rect, states, .. } => {
                const SIZE_MINIMIZED: u64 = 1;
                const SIZE_MAXIMIZED: u64 = 2;
                let Some(hwnd) = self.hwnd_of(window) else { return };
                self.fit(window);
                let Some(win) = self.windows.get_mut(&hwnd) else { return };
                let before = win.client();
                let moved = (rect.left, rect.top) != (win.rect.left, win.rect.top);
                win.rect = rect;
                let after = win.client();
                let resized = (before.width(), before.height()) != (after.width(), after.height());
                if resized {
                    win.invalid = after;
                    win.erase = true;
                }
                let kind = if states & dt::state::MINIMIZED != 0 {
                    SIZE_MINIMIZED
                } else if states & dt::state::MAXIMIZED != 0 {
                    SIZE_MAXIMIZED
                } else {
                    0
                };
                let kind_changed = std::mem::replace(&mut win.size_kind, kind) != kind;
                if moved || resized {
                    let (x, y) = self.move_origin(hwnd);
                    self.deliver(hwnd, WM_MOVE, 0, words(x, y));
                }
                if resized || kind_changed {
                    self.deliver(hwnd, WM_SIZE, kind, words(after.width(), after.height()));
                }
            }
            Event::Activated { window, active } => {
                let Some(hwnd) = self.hwnd_of(window) else { return };
                if active {
                    self.active = hwnd;
                    self.deliver(hwnd, WM_ACTIVATE, WA_ACTIVE, 0);
                } else {
                    if self.active == hwnd {
                        self.active = 0;
                    }
                    let focus = self.focus;
                    if focus != 0 && (focus == hwnd || is_descendant(&self.windows, focus, hwnd)) {
                        self.focus = 0;
                        self.deliver(focus, WM_KILLFOCUS, 0, 0);
                    }
                    self.deliver(hwnd, WM_ACTIVATE, WA_INACTIVE, 0);
                }
            }
            Event::Close { window } => {
                if let Some(hwnd) = self.hwnd_of(window) {
                    self.post_input(hwnd, WM_SYSCOMMAND, SC_CLOSE, 0, time);
                }
            }
            Event::Pointer { window, x, y, what, buttons, modifiers } => {
                let Some(top) = self.hwnd_of(window) else { return };
                for (bit, vk) in [(1, keys::VK_LBUTTON), (2, keys::VK_RBUTTON), (4, keys::VK_MBUTTON)] {
                    self.set_key(vk, buttons & bit != 0);
                }
                // MK_LBUTTON, MK_RBUTTON, MK_SHIFT, MK_CONTROL, MK_MBUTTON.
                let mut flags = u64::from(buttons & 3) | if buttons & 4 != 0 { 0x10 } else { 0 };
                if modifiers & display::keys::SHIFT != 0 {
                    flags |= 4;
                }
                if modifiers & display::keys::CTRL != 0 {
                    flags |= 8;
                }
                let (target, message, wparam, lparam) = match what {
                    // WM_MOUSELEAVE only comes to a window that asked with TrackMouseEvent, which isn't there yet;
                    // relative motion is raw input's (WM_INPUT), which isn't either.
                    Pointer::Leave | Pointer::Relative { .. } => return,
                    Pointer::Scroll { x: dx, y: dy } => {
                        const WM_MOUSEHWHEEL: u32 = 0x20e;
                        let (sx, sy) = self.screen_origin(top).unwrap_or_default();
                        let target = if self.focus != 0 { self.focus } else { top };
                        for (delta, message) in [(dy, WM_MOUSEWHEEL), (dx, WM_MOUSEHWHEEL)] {
                            if delta != 0 {
                                self.post_input(target, message, flags | (delta as i16 as u16 as u64) << 16, words(sx + x, sy + y), time);
                            }
                        }
                        return;
                    }
                    _ => {
                        let client = self.windows.get(&top).map(Window::client).unwrap_or_default();
                        // The window with the capture has it wherever it is, in its own client coordinates; outside
                        // the client area (a drag that's left it), it's the top-level window's.
                        let (target, cx, cy) = if self.capture != 0 && self.windows.contains_key(&self.capture) {
                            let (sx, sy) = self.screen_origin(top).unwrap_or_default();
                            let (ox, oy) = self.screen_origin(self.capture).unwrap_or_default();
                            (self.capture, sx + x - ox, sy + y - oy)
                        } else if client.contains(x, y) {
                            self.child_at(top, x, y)
                        } else {
                            (top, x, y)
                        };
                        let message = match what {
                            Pointer::Down(Button::Left) => 0x201,
                            Pointer::Up(Button::Left) => 0x202,
                            Pointer::Down(Button::Right) => 0x204,
                            Pointer::Up(Button::Right) => 0x205,
                            Pointer::Down(Button::Middle) => 0x207,
                            Pointer::Up(Button::Middle) => 0x208,
                            _ => WM_MOUSEMOVE,
                        };
                        // As DefWindowProc's WM_SETCURSOR does: over a client area, its class's cursor, if it has one.
                        if matches!(what, Pointer::Enter | Pointer::Move) && self.capture == 0 {
                            let class = self.windows.get(&target).and_then(|w| self.class_by_name(&w.class)).map_or(0, |c| c.cursor);
                            if class != 0 {
                                self.cursor = class;
                                dt::lock(&self.display).set_cursor(window, cursor_of(class));
                            }
                        }
                        (target, message, flags, words(cx, cy))
                    }
                };
                self.post_input(target, message, wparam, lparam, time);
            }
            // win32k's canvases are its own, reused in place; GDI paints when it's asked to, not by frames.
            // win32k makes no popups yet (menus come with its user interface).
            // DXGI's frame statistics will read Presented; ClipCursor's confinement stays set while another window's
            // active, as the desktop keeps it, and GetClipCursor still says it.
            Event::Released { .. } | Event::Frame { .. } | Event::PopupDone { .. } | Event::Presented { .. } | Event::Constrained { .. } => {}
            // Another's offer took the clipboard from the window that emptied it: WM_DESTROYCLIPBOARD.
            Event::SelectionCancelled { window, selection: display::selection::Selection::Clipboard } => {
                let Some(&hwnd) = self.clipboard.owners.get(&window) else { return };
                // Not the owner any more, unless it's emptied it again since.
                let still = dt::lock(&self.display).selection(display::selection::Selection::Clipboard).is_some_and(|(o, _)| o == Some(window));
                if self.clipboard.owner == hwnd && !still {
                    self.clipboard.owner = 0;
                    self.clipboard.source = None;
                }
                if self.windows.contains_key(&hwnd) {
                    self.deliver(hwnd, 0x307, 0, 0);
                }
            }
            // Files dropped: WM_DROPFILES, with an HDROP of their paths, to the window that takes them.
            Event::SelectionData { window, transfer, data } => {
                let Some((hwnd, x, y)) = self.drops.remove(&transfer) else { return };
                let files: Vec<String> = data.map(|d| display::selection::uri_list_paths(&d)).unwrap_or_default().into_iter().map(|p| p.replace('/', "\\")).collect();
                if !files.is_empty() && self.windows.contains_key(&hwnd) {
                    let hdrop = 0xd_0000 + 4 * (self.hdrops.keys().next_back().map_or(0, |h| (h - 0xd_0000) / 4) + 1);
                    self.hdrops.insert(hdrop, DropFiles { files, point: (x, y) });
                    self.post_input(hwnd, WM_DROPFILES, hdrop, 0, time);
                }
                dt::lock(&self.display).finish_drop(window);
            }
            Event::SelectionCancelled { .. } => {}
            // A drag over its windows: taken as files where a window takes them (DragAcceptFiles). OLE's drag and drop
            // (RegisterDragDrop, DoDragDrop), which carries anything, isn't there yet, so win32k is never a source.
            Event::DragEnter { window, x, y, mimes, .. } => {
                let files = mimes.iter().any(|m| m == display::selection::URI_LIST);
                self.drag_files = files.then_some(window);
                self.take_drag(window, x, y, files);
            }
            Event::DragMotion { window, x, y } => {
                let files = self.drag_files == Some(window);
                self.take_drag(window, x, y, files);
            }
            Event::DragLeave { .. } => self.drag_files = None,
            Event::Drop { window, x, y, .. } => {
                self.drag_files = None;
                let target = self.hwnd_of(window).and_then(|top| self.drop_target(top, x, y));
                let mut d = dt::lock(&self.display);
                match target.and_then(|t| d.request(display::selection::Selection::Drag, display::selection::URI_LIST, Some(window)).map(|id| (id, t))) {
                    Some((transfer, t)) => {
                        drop(d);
                        self.drops.insert(transfer, t);
                    }
                    None => d.finish_drop(window),
                }
            }
            Event::DragAction { .. } | Event::Dropped { .. } | Event::DragEnd { .. } => {}
            // win32k's offers give all their data at once (delayed rendering, WM_RENDERFORMAT, isn't there yet).
            Event::SendSelection { transfer, .. } => dt::lock(&self.display).provide(transfer, None),
            // Windows sends WM_DPICHANGED only to a window that's per-monitor DPI aware, which none says it is yet;
            // GetDpiForWindow reads the output as it is.
            Event::Scale { .. } => {}
            // Text an input method committed (or no key types): WM_CHAR for each UTF-16 unit, to the focus, as Windows
            // sends a VK_PACKET's.
            Event::Text { window, text } => {
                let Some(top) = self.hwnd_of(window) else { return };
                let focus = self.focus;
                let target = if focus != 0 && (focus == top || is_descendant(&self.windows, focus, top)) { focus } else { top };
                for unit in text.encode_utf16() {
                    self.post_input(target, WM_CHAR, u64::from(unit), 1, time);
                }
            }
            // The composition itself is the input method's, shown by the host; IMM's messages aren't there yet.
            Event::Preedit { .. } => {}
            Event::Key { window, code, down, repeat, modifiers } => {
                let Some(top) = self.hwnd_of(window) else { return };
                let vk = keys::vk(code);
                if vk == 0 {
                    return;
                }
                let generic = keys::generic(vk);
                // A repeat as the desktop marked it, or a down for a key this side has down already (SendInput's).
                let repeat = repeat || (down && self.keys[generic as usize] & 0x80 != 0);
                self.set_key(vk, down);
                if generic != vk {
                    self.set_key(generic, down);
                }
                let focus = self.focus;
                let target = if focus != 0 && (focus == top || is_descendant(&self.windows, focus, top)) { focus } else { top };
                let alt = modifiers & display::keys::ALT != 0;
                // With Alt held, and Alt itself coming up, they're system keys.
                let system = alt || (!down && generic == keys::VK_MENU);
                let (scan, extended) = keys::scan(code);
                let mut lparam = 1 | u64::from(scan) << 16 | u64::from(extended) << 24 | u64::from(alt) << 29;
                if repeat || !down {
                    lparam |= 1 << 30;
                }
                if !down {
                    lparam |= 1 << 31;
                }
                let message = match (down, system) {
                    (true, false) => WM_KEYDOWN,
                    (false, false) => WM_KEYUP,
                    (true, true) => WM_SYSKEYDOWN,
                    (false, true) => WM_SYSKEYUP,
                };
                self.post_input(target, message, u64::from(generic), lparam, time);
            }
        }
    }

    fn queue(&mut self, tid: u64, waiter: &Arc<Waiter>) -> &mut Queue {
        self.queues.entry(tid).or_insert_with(|| Queue { posted: VecDeque::new(), sent: VecDeque::new(), quit: None, timers: Vec::new(), waiter: waiter.clone(), keys: [0; 256] })
    }

    /// Where a window's client area is in its top-level window's surface, and that window.
    pub fn origin(&self, mut hwnd: u64) -> Option<(u64, i32, i32)> {
        let (mut x, mut y) = (0, 0);
        loop {
            let w = self.windows.get(&hwnd)?;
            if w.top_level() {
                return Some((hwnd, x, y));
            }
            x += w.rect.left;
            y += w.rect.top;
            hwnd = w.parent;
        }
    }

    pub fn class_by_name(&self, name: &str) -> Option<&Class> {
        self.classes.get(&name.to_lowercase())
    }

    /// Every window that's `hwnd` or under it, or that it owns, children before their parents.
    fn family(&self, hwnd: u64) -> Vec<u64> {
        let mut out = Vec::new();
        for (&child, w) in &self.windows {
            if w.parent == hwnd || w.owner == hwnd {
                out.extend(self.family(child));
            }
        }
        out.push(hwnd);
        out
    }

    /// Every top-level window's picture now, and those destroyed already.
    pub fn screenshots(&self) -> Vec<(String, Surface)> {
        let mut shots = self.shots.clone();
        let d = dt::lock(&self.display);
        for w in self.windows.values() {
            if let Some(c) = self.canvases.get(&w.shown).filter(|_| w.shown != 0 && d.exists(w.shown)) {
                shots.push((w.text.clone(), c.snapshot()));
            }
        }
        shots
    }
}

/// A RECT in the program's memory.
pub fn rect_bytes(r: &Rect) -> Vec<u8> {
    [r.left, r.top, r.right, r.bottom].iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn rect_from(b: &[u8]) -> Rect {
    let at = |i: usize| i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or_default());
    Rect::new(at(0), at(1), at(2), at(3))
}

impl Task {
    pub fn win32k(&self) -> std::sync::MutexGuard<'_, Win32k> {
        lock(&self.k.win32k)
    }

    pub fn read_rect(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<Rect> {
        (at != 0).then(|| self.mem(cx, false, |t| user::read_vec(&t.space(), at, 16)).ok().map(|b| rect_from(&b))).flatten()
    }

    pub fn write_rect(&mut self, cx: &mut Cx<'_>, at: u64, r: &Rect) -> bool {
        self.mem(cx, true, |t| user::write(&mut t.space(), at, &rect_bytes(r))).is_ok()
    }

    pub fn wstr(&mut self, cx: &mut Cx<'_>, at: u64) -> String {
        if at == 0 { String::new() } else { self.mem(cx, false, |t| user::read_wstr(&t.space(), at, 32767)).unwrap_or_default() }
    }

    /// A string the program passed: UTF-16, or (not `wide`) in the ANSI code page.
    pub fn text_arg(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> String {
        if wide || at == 0 {
            return self.wstr(cx, at);
        }
        self.mem(cx, false, |t| user::read_cstr(&t.space(), at, 32767)).map(|b| crate::ansi::text(&b)).unwrap_or_default()
    }

    fn now_ms(&self) -> u32 {
        (self.k.host.monotonic() / 1_000_000) as u32
    }

    /// Works on the desktop, without win32k's lock.
    pub fn desktop<R>(&self, f: impl FnOnce(&mut dt::Desktop) -> R) -> R {
        let shared = self.win32k().display.clone();
        let mut d = dt::lock(&shared);
        f(&mut d)
    }

    /// Turns what's happened on the desktop into messages, for whichever threads' windows they're for.
    pub fn pump(&mut self) {
        let time = self.now_ms();
        let mut w = self.win32k();
        let shared = w.display.clone();
        let events = dt::lock(&shared).take_events();
        for e in events {
            w.route(e, time);
        }
    }

    /// Activates a window's top-level window, and deals with what that sends this thread's windows: the previous
    /// active window.
    pub fn activate(&mut self, cx: &mut Cx<'_>, hwnd: u64) -> Result<u64, Reply> {
        let (shown, previous) = {
            let w = self.win32k();
            (w.top_level_of(hwnd).and_then(|t| w.windows.get(&t)).map_or(0, |t| t.shown), w.active)
        };
        // As the foreground rules have it: granted to the process whose window is active or that had the last
        // input, else the window flashes instead.
        if shown != 0 {
            let pid = self.process.pid;
            self.desktop(|d| d.request_activation(shown, Some(pid), None));
        }
        self.pump();
        self.take_sent(cx)?;
        Ok(previous)
    }

    /// `SetFocus`: WM_KILLFOCUS to the window that had it, WM_SETFOCUS to this one. The window that had it.
    pub fn set_focus(&mut self, cx: &mut Cx<'_>, hwnd: u64) -> Result<u64, Reply> {
        let old = {
            let mut w = self.win32k();
            if hwnd != 0 && !w.windows.contains_key(&hwnd) {
                return Ok(0);
            }
            let old = w.focus;
            if old == hwnd {
                return Ok(old);
            }
            w.focus = hwnd;
            old
        };
        if old != 0 && self.window_exists(old) {
            self.send_message(cx, old, WM_KILLFOCUS, hwnd, 0)?;
        }
        if hwnd != 0 {
            self.send_message(cx, hwnd, WM_SETFOCUS, old, 0)?;
        }
        Ok(old)
    }

    // ---- Classes ----

    pub fn register_class(&mut self, mut class: Class) -> Result<u16, u32> {
        let mut w = self.win32k();
        let key = class.name.to_lowercase();
        if w.classes.contains_key(&key) {
            return Err(ERROR_CLASS_ALREADY_EXISTS);
        }
        w.next_atom = w.next_atom.max(0xc000) + 1;
        class.atom = w.next_atom;
        let atom = class.atom;
        w.classes.insert(key, class);
        Ok(atom)
    }

    pub fn unregister_class(&mut self, name: &str) -> bool {
        let mut w = self.win32k();
        let key = name.to_lowercase();
        let used = w.windows.values().any(|win| win.class == key);
        !used && w.classes.remove(&key).is_some()
    }

    /// A class named by a string, or by its atom (a "pointer" below 64 KiB).
    pub fn class_named(&mut self, cx: &mut Cx<'_>, at: u64) -> Option<String> {
        self.class_named_as(cx, at, true)
    }

    /// A class by its atom, or its name as UTF-16 or ANSI.
    pub fn class_named_as(&mut self, cx: &mut Cx<'_>, at: u64, wide: bool) -> Option<String> {
        if at < 0x1_0000 {
            let w = self.win32k();
            return w.classes.values().find(|c| u64::from(c.atom) == at).map(|c| c.name.to_lowercase());
        }
        Some(self.text_arg(cx, at, wide).to_lowercase())
    }

    // ---- Windows ----

    /// `CreateWindowExW` (or, with `ansi`, `CreateWindowExA`): a window, its WM_NCCREATE and WM_CREATE, and showing
    /// it if it's WS_VISIBLE.
    pub fn create_window(&mut self, cx: &mut Cx<'_>, a: &[u64], ansi: bool) -> Result<u64, Reply> {
        let (ex_style, class_at, name_at, style) = (a[0] as u32, a[1], a[2], a[3] as u32);
        let mut rect = [a[4] as i32, a[5] as i32, a[6] as i32, a[7] as i32];
        let (parent, menu, instance, param) = (a[8], a[9], a[10], a[11]);
        let Some(class_name) = self.class_named_as(cx, class_at, !ansi) else {
            self.set_last_error(cx, ERROR_CANNOT_FIND_WND_CLASS);
            return Ok(0);
        };
        let text = self.text_arg(cx, name_at, !ansi);
        let child = style & WS_CHILD != 0;
        let style = if !child && style & WS_POPUP == 0 { style | WS_CAPTION | WS_CLIPSIBLINGS } else { style };
        if rect[0] == CW_USEDEFAULT {
            rect[0] = 100;
            rect[1] = 100;
        }
        if rect[2] == CW_USEDEFAULT {
            rect[2] = 640;
            rect[3] = 480;
        }
        // A top-level window's parent, as CreateWindowExW has it, is its owner.
        let (parent, owner) = if child { (parent, 0) } else { (DESKTOP, parent) };
        let hwnd = {
            let mut w = self.win32k();
            let Some(class) = w.class_by_name(&class_name).cloned() else {
                drop(w);
                self.set_last_error(cx, ERROR_CANNOT_FIND_WND_CLASS);
                return Ok(0);
            };
            if (parent != DESKTOP && !w.windows.contains_key(&parent)) || (owner != 0 && !w.windows.contains_key(&owner)) {
                drop(w);
                self.set_last_error(cx, ERROR_INVALID_WINDOW_HANDLE);
                return Ok(0);
            }
            w.next_hwnd = w.next_hwnd.max(0x10020) + 4;
            let hwnd = w.next_hwnd;
            let r = Rect::new(rect[0], rect[1], rect[0] + rect[2].max(0), rect[1] + rect[3].max(0));
            let top_level = parent == DESKTOP;
            let background = self.background_of(&w, &class);
            let owner = w.top_level_of(owner).unwrap_or(0);
            let framed = top_level && framed(style);
            let shown = if top_level {
                let owner_shown = w.windows.get(&owner).map_or(0, |o| o.shown);
                let layer = if ex_style & WS_EX_TOPMOST != 0 { Layer::Top } else { Layer::Normal };
                // An owned window is the desktop's dialog: kept above its owner, which is active again when it goes.
                let role = if owner_shown != 0 { Role::Dialog { parent: owner_shown } } else { Role::Toplevel };
                let spec = Spec { rect: r, title: text.clone(), framed, layer, role, fill: background, client: self.process.pid, no_activate: ex_style & WS_EX_NOACTIVATE != 0 };
                let mut d = dt::lock(&w.display);
                let shown = d.create(spec);
                let content = d.content_rect(shown).unwrap_or_default();
                let canvas = display::Canvas::new(content.width(), content.height(), background);
                d.attach(shown, Some(canvas.buffer()));
                d.commit(shown);
                drop(d);
                w.canvases.insert(shown, canvas);
                shown
            } else {
                0
            };
            let window = Window {
                class: class_name.clone(),
                wndproc: class.wndproc,
                parent,
                tid: self.tid,
                pid: self.process.pid,
                style: style & !WS_VISIBLE,
                ex_style,
                rect: r,
                text: text.clone(),
                visible: false,
                invalid: Rect::default(),
                erase: false,
                userdata: 0,
                id: if child { menu } else { 0 },
                instance,
                extra: vec![0; class.extra],
                shown,
                owner,
                framed,
                destroying: false,
                size_kind: 0,
                ansi: class.ansi,
            };
            w.windows.insert(hwnd, window);
            w.queue(self.tid, &self.waiter.clone());
            hwnd
        };
        // CREATESTRUCTW: lpCreateParams, hInstance, hMenu, hwndParent, cy, cx, y, x, style, lpszName, lpszClass,
        // dwExStyle. From CreateWindowExA, its strings are UTF-16 copies after it, as every message's are inside
        // win32k; a window that takes ANSI gets them back as ANSI.
        let wide = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() };
        // The class's name as the program wrote it, not as win32k keys it.
        let class_text = if ansi && class_at >= 0x1_0000 { self.text_arg(cx, class_at, false) } else { String::new() };
        let (name_w, class_w) = if ansi { (wide(&text), if class_at < 0x1_0000 { Vec::new() } else { wide(&class_text) }) } else { (Vec::new(), Vec::new()) };
        let len = 80 + name_w.len() + class_w.len();
        let cs = |at: u64| -> Vec<u8> {
            let mut cs = vec![0u8; 80];
            let name_ptr = if ansi && name_at != 0 { at + 80 } else { name_at };
            let class_ptr = if ansi && class_at >= 0x1_0000 { at + 80 + name_w.len() as u64 } else { class_at };
            for (o, v) in [(0, param), (8, instance), (16, menu), (24, if parent == DESKTOP { 0 } else { parent }), (56, name_ptr), (64, class_ptr)] {
                cs[o..o + 8].copy_from_slice(&v.to_le_bytes());
            }
            for (o, v) in [(32, rect[3]), (36, rect[2]), (40, rect[1]), (44, rect[0]), (48, style as i32), (72, ex_style as i32)] {
                cs[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
            cs.extend(&name_w);
            cs.extend(&class_w);
            cs
        };
        let created = self.with_scratch(cx, len, cs, |task, cx, at| {
            let ok = task.call_wndproc(cx, hwnd, WM_NCCREATE, 0, at)?;
            Ok(ok != 0 && task.call_wndproc(cx, hwnd, WM_CREATE, 0, at)? as i64 != -1)
        })?;
        if !created {
            self.destroy_window(cx, hwnd)?;
            return Ok(0);
        }
        let (client, (x, y)) = {
            let w = self.win32k();
            (w.windows.get(&hwnd).map(Window::client).unwrap_or_default(), w.move_origin(hwnd))
        };
        self.send_message(cx, hwnd, WM_SIZE, 0, words(client.width(), client.height()))?;
        self.send_message(cx, hwnd, WM_MOVE, 0, words(x, y))?;
        if style & WS_VISIBLE != 0 {
            self.show_window(cx, hwnd, SW_SHOW)?;
        }
        Ok(hwnd)
    }

    /// The colour a class's background brush fills with.
    fn background_of(&self, w: &Win32k, class: &Class) -> u32 {
        match class.background {
            0 => 0xffffff,
            b if b < 0x100 => gdi::sys_color(b as u32 - 1),
            b => w.gdi.brush_color(b).unwrap_or(0xffffff),
        }
    }

    pub fn window_exists(&self, hwnd: u64) -> bool {
        self.win32k().windows.contains_key(&hwnd)
    }

    /// `DestroyWindow`: WM_DESTROY to it and then its children, then WM_NCDESTROY to them, children first.
    pub fn destroy_window(&mut self, cx: &mut Cx<'_>, hwnd: u64) -> Result<bool, Reply> {
        let family = {
            let mut w = self.win32k();
            match w.windows.get_mut(&hwnd) {
                Some(win) if win.destroying => return Ok(true),
                Some(win) if win.tid != self.tid => return Ok(false),
                Some(win) => win.destroying = true,
                None => return Ok(false),
            }
            let family = w.family(hwnd);
            for h in &family {
                if let Some(win) = w.windows.get_mut(h) {
                    win.destroying = true;
                }
            }
            family
        };
        for h in family.iter().rev() {
            self.call_wndproc(cx, *h, WM_DESTROY, 0, 0)?;
        }
        for h in &family {
            self.call_wndproc(cx, *h, WM_NCDESTROY, 0, 0)?;
            let mut w = self.win32k();
            if let Some(win) = w.windows.remove(h) {
                if win.shown != 0 {
                    let shared = w.display.clone();
                    if let Some(c) = w.canvases.remove(&win.shown) {
                        w.shots.push((win.text.clone(), c.snapshot()));
                    }
                    dt::lock(&shared).destroy(win.shown);
                }
                if w.active == *h {
                    w.active = 0;
                }
                if w.focus == *h {
                    w.focus = 0;
                }
                if w.capture == *h {
                    w.capture = 0;
                    let shared = w.display.clone();
                    dt::lock(&shared).set_capture(None);
                }
            }
            for q in w.queues.values_mut() {
                q.posted.retain(|m| m.hwnd != *h);
                q.timers.retain(|t| t.hwnd != *h);
            }
        }
        // Another window may be active now.
        self.pump();
        self.take_sent(cx)?;
        Ok(true)
    }

    /// `ShowWindow`: whether it was visible before. Most ways of showing a top-level window activate it.
    pub fn show_window(&mut self, cx: &mut Cx<'_>, hwnd: u64, command: u32) -> Result<bool, Reply> {
        let show = command != 0;
        let (was, shown) = {
            let mut w = self.win32k();
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(false) };
            let was = win.visible;
            win.visible = show;
            if show && !was {
                win.invalid = win.client();
                win.erase = true;
            }
            (was, win.shown)
        };
        if shown != 0 {
            self.desktop(|d| {
                d.set_visible(shown, show);
                // SW_SHOWMINIMIZED, SW_MINIMIZE, SW_SHOWMINNOACTIVE and SW_FORCEMINIMIZE minimize it; SW_SHOWMAXIMIZED
                // maximizes it; SW_SHOWNORMAL and SW_RESTORE restore a minimized window, else a maximized one.
                let states = d.states(shown);
                match command {
                    2 | 6 | 7 | 11 => d.set_minimized(shown, true),
                    3 => {
                        d.set_minimized(shown, false);
                        d.set_maximized(shown, true);
                    }
                    1 | 9 if states & dt::state::MINIMIZED != 0 => d.set_minimized(shown, false),
                    1 | 9 if states & dt::state::MAXIMIZED != 0 => d.set_maximized(shown, false),
                    _ => {}
                }
            });
        }
        if was != show {
            self.send_message(cx, hwnd, WM_SHOWWINDOW, u64::from(show), 0)?;
        }
        // SW_SHOWNORMAL, SW_SHOWMAXIMIZED, SW_SHOW, SW_RESTORE and SW_SHOWDEFAULT activate; SW_SHOWNOACTIVATE and
        // SW_SHOWNA don't.
        if show && shown != 0 && matches!(command, 1 | 3 | 5 | 9 | 10) {
            self.activate(cx, hwnd)?;
        } else {
            self.pump();
            self.take_sent(cx)?;
        }
        Ok(was)
    }

    /// Moves or sizes a window (frame and all), telling it with WM_MOVE and WM_SIZE (its client area).
    #[allow(clippy::too_many_arguments)]
    pub fn place_window(&mut self, cx: &mut Cx<'_>, hwnd: u64, x: i32, y: i32, width: i32, height: i32, flags: u32) -> Result<bool, Reply> {
        const NOSIZE: u32 = 1;
        const NOMOVE: u32 = 2;
        const NOACTIVATE: u32 = 0x10;
        const FRAMECHANGED: u32 = 0x20;
        const SHOWWINDOW: u32 = 0x40;
        const HIDEWINDOW: u32 = 0x80;
        let (moved, sized, (cx_, cy_), client) = {
            let mut w = self.win32k();
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(false) };
            let old = win.rect;
            // A style changed with SetWindowLong takes effect here: a top-level window's frame, given or taken
            // away, keeps its rectangle, and its client area is what's inside.
            let old_client = win.client();
            let reframed = flags & FRAMECHANGED != 0 && win.top_level() && win.framed != framed(win.style);
            if reframed {
                win.framed = framed(win.style);
                let (shown, framed_now) = (win.shown, win.framed);
                if shown != 0 {
                    dt::lock(&w.display).set_framed(shown, framed_now, Keep::Rect);
                }
            }
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(false) };
            let (x, y) = if flags & NOMOVE != 0 { (old.left, old.top) } else { (x, y) };
            let (width, height) = if flags & NOSIZE != 0 { (old.width(), old.height()) } else { (width.max(0), height.max(0)) };
            win.rect = Rect::new(x, y, x + width, y + height);
            let sized = (width, height) != (old.width(), old.height()) || win.client() != old_client;
            if sized {
                win.invalid = win.client();
                win.erase = true;
            }
            let (rect, shown, client) = (win.rect, win.shown, win.client());
            if shown != 0 {
                dt::lock(&w.display).set_rect(shown, rect);
                w.fit(shown);
            }
            ((x, y) != (old.left, old.top), sized, w.move_origin(hwnd), client)
        };
        if moved {
            self.send_message(cx, hwnd, WM_MOVE, 0, words(cx_, cy_))?;
        }
        if sized {
            self.send_message(cx, hwnd, WM_SIZE, 0, words(client.width(), client.height()))?;
        }
        if flags & SHOWWINDOW != 0 {
            self.show_window(cx, hwnd, if flags & NOACTIVATE != 0 { SW_SHOWNA } else { SW_SHOW })?;
        } else if flags & HIDEWINDOW != 0 {
            self.show_window(cx, hwnd, 0)?;
        }
        Ok(true)
    }

    // ---- Messages ----

    /// Calls a window's `WndProc` on this thread, with the message as win32k has every message (text in UTF-16):
    /// converted to ANSI first for a window that takes it.
    pub fn call_wndproc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let wndproc = self.win32k().windows.get(&hwnd).map(|w| (w.wndproc, w.ansi));
        match wndproc {
            Some((f, true)) if f != 0 => self.call_ansi(cx, f, hwnd, message, wparam, lparam),
            Some((f, false)) if f != 0 => self.upcall(cx, f, &[hwnd, u64::from(message), wparam, lparam]),
            _ => Ok(0),
        }
    }

    /// Calls a `WndProc` that takes ANSI with a message in UTF-16: its text converted on the way in (WM_SETTEXT,
    /// CREATESTRUCT's names, a typed character) and what it writes converted on the way out (WM_GETTEXT).
    fn call_ansi(&mut self, cx: &mut Cx<'_>, f: u64, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let msg = u64::from(message);
        match message {
            WM_SETTEXT if lparam != 0 => {
                let text = crate::ansi::bytes(&self.wstr(cx, lparam)).into_iter().chain([0]).collect::<Vec<u8>>();
                self.with_scratch(cx, text.len(), |_| text.clone(), |task, cx, at| task.upcall(cx, f, &[hwnd, msg, wparam, at]))
            }
            WM_GETTEXT if lparam != 0 && wparam != 0 => {
                let room = (wparam as u32 as usize).min(1 << 16);
                self.with_scratch(cx, room, |_| vec![0; room], |task, cx, at| {
                    task.upcall(cx, f, &[hwnd, msg, room as u64, at])?;
                    let ansi = task.mem(cx, false, |t| user::read_vec(&t.space(), at, room)).unwrap_or_default();
                    let end = ansi.iter().position(|&b| b == 0).unwrap_or(room.saturating_sub(1));
                    let units: Vec<u16> = crate::ansi::decode(crate::ansi::ACP, &ansi[..end], false).unwrap_or_default().into_iter().take(room - 1).collect();
                    let bytes: Vec<u8> = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
                    let _ = task.mem(cx, true, |t| user::write(&mut t.space(), lparam, &bytes));
                    Ok(units.len() as u64)
                })
            }
            WM_NCCREATE | WM_CREATE if lparam != 0 => {
                let Ok(cs) = self.mem(cx, false, |t| user::read_vec(&t.space(), lparam, 80)) else { return Ok(0) };
                let ptr = |o: usize| u64::from_le_bytes(cs[o..o + 8].try_into().unwrap_or_default());
                let to_ansi = |task: &mut Task, cx: &mut Cx<'_>, at: u64| -> Option<Vec<u8>> {
                    (at >= 0x1_0000).then(|| crate::ansi::bytes(&task.wstr(cx, at)).into_iter().chain([0]).collect())
                };
                let (name, class) = (to_ansi(self, cx, ptr(56)), to_ansi(self, cx, ptr(64)));
                let len = 80 + name.as_ref().map_or(0, Vec::len) + class.as_ref().map_or(0, Vec::len);
                let build = |at: u64| {
                    let mut out = cs.clone();
                    let mut tail: Vec<u8> = Vec::new();
                    for (o, s) in [(56, &name), (64, &class)] {
                        if let Some(s) = s {
                            out[o..o + 8].copy_from_slice(&(at + 80 + tail.len() as u64).to_le_bytes());
                            tail.extend(s);
                        }
                    }
                    out.extend(tail);
                    out
                };
                self.with_scratch(cx, len, build, |task, cx, at| task.upcall(cx, f, &[hwnd, msg, wparam, at]))
            }
            WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => {
                let c = atlas_abi::ansi::to_1252(wparam as u16).unwrap_or(b'?');
                self.upcall(cx, f, &[hwnd, msg, u64::from(c), lparam])
            }
            _ => self.upcall(cx, f, &[hwnd, msg, wparam, lparam]),
        }
    }

    /// `SendMessageW`: the `WndProc`'s result. From another thread, it waits for the window's thread to take it,
    /// dealing with messages sent to its own windows meanwhile, as Windows does.
    pub fn send_message(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let owner = self.win32k().windows.get(&hwnd).map(|w| w.tid);
        let Some(owner) = owner else { return Ok(0) };
        if owner == self.tid {
            return self.call_wndproc(cx, hwnd, message, wparam, lparam);
        }
        let reply = Arc::new(SentReply { value: Mutex::new(None), sender: self.waiter.clone() });
        {
            let mut w = self.win32k();
            let Some(q) = w.queues.get_mut(&owner) else { return Ok(0) };
            q.sent.push_back(Sent { msg: Msg { hwnd, message, wparam, lparam, time: 0 }, reply: reply.clone() });
            q.waiter.wake();
        }
        loop {
            self.waiter.reset();
            if let Some(value) = *lock(&reply.value) {
                return Ok(value);
            }
            self.take_sent(cx)?;
            if let Some(value) = *lock(&reply.value) {
                return Ok(value);
            }
            if !self.window_exists(hwnd) {
                return Ok(0);
            }
            if self.waiter.wait(Some(Instant::now() + Duration::from_millis(100))) == Woke::Interrupted && self.ending() {
                return Err(Reply::Ended);
            }
        }
    }

    /// Calls the `WndProc` for each message other threads have sent to this thread's windows.
    pub fn take_sent(&mut self, cx: &mut Cx<'_>) -> Result<(), Reply> {
        loop {
            let next = {
                let tid = self.tid;
                let mut w = self.win32k();
                w.queues.get_mut(&tid).and_then(|q| q.sent.pop_front())
            };
            let Some(sent) = next else { return Ok(()) };
            let value = self.call_wndproc(cx, sent.msg.hwnd, sent.msg.message, sent.msg.wparam, sent.msg.lparam)?;
            *lock(&sent.reply.value) = Some(value);
            sent.reply.sender.wake();
        }
    }

    /// `PostMessageW` (and `PostThreadMessageW`, with `hwnd` 0 and a thread): false if there's nowhere for it.
    pub fn post_message(&mut self, hwnd: u64, thread: u64, message: u32, wparam: u64, lparam: u64) -> bool {
        let time = self.now_ms();
        let mut w = self.win32k();
        let tid = if hwnd == 0 { thread } else { w.windows.get(&hwnd).map_or(0, |win| win.tid) };
        let Some(q) = w.queues.get_mut(&tid) else { return false };
        q.posted.push_back(Msg { hwnd, message, wparam, lparam, time });
        q.waiter.wake();
        true
    }

    pub fn post_quit(&mut self, code: u64) {
        let waiter = self.waiter.clone();
        let mut w = self.win32k();
        let q = w.queue(self.tid, &waiter);
        q.quit = Some(code);
    }

    /// The next message for `GetMessage` (`wait`) or `PeekMessage`, taking it off the queue if `remove`. Sent messages
    /// are dealt with first; then posted ones, then WM_QUIT, then WM_PAINT for windows that need it, then timers.
    pub fn next_message(&mut self, cx: &mut Cx<'_>, filter: Filter, remove: bool, wait: bool) -> Result<Option<Msg>, Reply> {
        let waiter = self.waiter.clone();
        loop {
            self.waiter.reset();
            self.pump();
            self.take_sent(cx)?;
            let time = self.now_ms();
            let deadline = {
                let tid = self.tid;
                let mut w = self.win32k();
                w.queue(tid, &waiter);
                let windows = &w.windows;
                let q = w.queues.get(&tid).expect("the queue was just made");
                if let Some(i) = q.posted.iter().position(|m| filter.wants(m, windows)) {
                    let m = q.posted[i];
                    if remove {
                        let q = w.queues.get_mut(&tid).expect("there");
                        q.posted.remove(i);
                        took(&mut q.keys, &m);
                    }
                    return Ok(Some(m));
                }
                if let Some(code) = q.quit.filter(|_| filter.message(WM_QUIT)) {
                    if remove {
                        w.queues.get_mut(&tid).expect("there").quit = None;
                    }
                    return Ok(Some(Msg { hwnd: 0, message: WM_QUIT, wparam: code, lparam: 0, time }));
                }
                if filter.message(WM_PAINT) {
                    let painting = windows.iter().find(|&(&h, win)| {
                        win.tid == tid && win.visible && !win.invalid.is_empty() && (filter.hwnd == 0 || filter.hwnd == h || is_descendant(windows, h, filter.hwnd))
                    });
                    if let Some((&h, _)) = painting {
                        return Ok(Some(Msg { hwnd: h, message: WM_PAINT, wparam: 0, lparam: 0, time }));
                    }
                }
                let now = Instant::now();
                let q = w.queues.get_mut(&tid).expect("there");
                if filter.message(WM_TIMER)
                    && let Some(t) = q.timers.iter_mut().find(|t| t.next <= now && (filter.hwnd == 0 || filter.hwnd == t.hwnd))
                {
                    let m = Msg { hwnd: t.hwnd, message: WM_TIMER, wparam: t.id, lparam: t.function, time };
                    if remove {
                        t.next = now + t.period;
                    }
                    return Ok(Some(m));
                }
                q.timers.iter().map(|t| t.next).min()
            };
            if !wait {
                return Ok(None);
            }
            if self.waiter.wait(deadline) == Woke::Interrupted && self.ending() {
                return Err(Reply::Ended);
            }
        }
    }

    /// `DispatchMessageW`: the message's `WndProc`, or a timer's function.
    pub fn dispatch(&mut self, cx: &mut Cx<'_>, m: Msg) -> Result<u64, Reply> {
        if m.message == WM_TIMER && m.lparam != 0 {
            return self.upcall(cx, m.lparam, &[m.hwnd, u64::from(WM_TIMER), m.wparam, u64::from(m.time)]);
        }
        if m.hwnd == 0 {
            return Ok(0);
        }
        self.call_wndproc(cx, m.hwnd, m.message, m.wparam, m.lparam)
    }

    pub fn set_timer(&mut self, hwnd: u64, id: u64, ms: u32, function: u64) -> u64 {
        let waiter = self.waiter.clone();
        let tid = self.tid;
        let mut w = self.win32k();
        if hwnd != 0 && !w.windows.contains_key(&hwnd) {
            return 0;
        }
        let period = Duration::from_millis(u64::from(ms.max(10)));
        let q = w.queue(tid, &waiter);
        // A timer with no window gets an ID of its own.
        let id = if hwnd == 0 { q.timers.iter().map(|t| t.id).max().unwrap_or(0x100) + 1 } else { id };
        q.timers.retain(|t| !(t.hwnd == hwnd && t.id == id));
        q.timers.push(Timer { hwnd, id, period, next: Instant::now() + period, function });
        id
    }

    pub fn kill_timer(&mut self, hwnd: u64, id: u64) -> bool {
        let tid = self.tid;
        let mut w = self.win32k();
        let Some(q) = w.queues.get_mut(&tid) else { return false };
        let before = q.timers.len();
        q.timers.retain(|t| !(t.hwnd == hwnd && t.id == id));
        q.timers.len() != before
    }

    /// Marks part of a window (all of it, for None) as needing painting, and wakes its thread to paint it.
    pub fn invalidate(&mut self, hwnd: u64, rect: Option<Rect>, erase: bool) -> bool {
        let mut w = self.win32k();
        let Some(win) = w.windows.get_mut(&hwnd) else { return false };
        let r = rect.unwrap_or(win.client()).intersect(&win.client());
        win.invalid = win.invalid.union(&r);
        win.erase |= erase;
        let tid = win.tid;
        if let Some(q) = w.queues.get(&tid) {
            q.waiter.wake();
        }
        true
    }

    /// Starts moving a top-level window by the pointer (or with `edges`, resizing it), as its frame would, while the
    /// button that went down on it is held. Windows enters a loop of its own until the button's up; here the
    /// desktop moves it as the pointer does, and WM_MOVE and WM_SIZE come as they do from its frame.
    fn drag_frame(&mut self, top: u64, edges: Option<u8>) {
        let shown = self.win32k().windows.get(&top).map_or(0, |w| w.shown);
        if shown != 0 {
            self.desktop(|d| match edges {
                Some(edges) => d.begin_resize(shown, edges),
                None => d.begin_move(shown),
            });
        }
    }

    /// `DefWindowProcW`: what a window does with a message its `WndProc` doesn't.
    pub fn def_window_proc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        match message {
            WM_NCCREATE => Ok(1),
            WM_CLOSE => {
                self.destroy_window(cx, hwnd)?;
                Ok(0)
            }
            WM_SETTEXT => {
                let text = self.wstr(cx, lparam);
                let shown = match self.win32k().windows.get_mut(&hwnd) {
                    Some(win) => {
                        win.text = text.clone();
                        win.shown
                    }
                    None => 0,
                };
                if shown != 0 {
                    self.desktop(|d| d.set_title(shown, &text));
                }
                Ok(1)
            }
            WM_ACTIVATE => {
                if wparam & 0xffff != WA_INACTIVE {
                    self.set_focus(cx, hwnd)?;
                }
                Ok(0)
            }
            WM_SYSCOMMAND => {
                const SC_SIZE: u64 = 0xf000;
                const SC_MOVE: u64 = 0xf010;
                const SC_MINIMIZE: u64 = 0xf020;
                const SC_MAXIMIZE: u64 = 0xf030;
                const SC_RESTORE: u64 = 0xf120;
                // SC_SIZE's low bits are which edge (WMSZ_LEFT, RIGHT, TOP, TOPLEFT, TOPRIGHT, BOTTOM, BOTTOMLEFT,
                // BOTTOMRIGHT); SC_MOVE's are HTCAPTION from a click, as programs that draw their own send it.
                let top = self.win32k().top_level_of(hwnd).unwrap_or(hwnd);
                match wparam & 0xfff0 {
                    SC_CLOSE => {
                        self.send_message(cx, hwnd, WM_CLOSE, 0, 0)?;
                    }
                    SC_MOVE => self.drag_frame(top, None),
                    SC_SIZE => {
                        const WMSZ: [u8; 9] = [0, LEFT, RIGHT, TOP, TOP | LEFT, TOP | RIGHT, BOTTOM, BOTTOM | LEFT, BOTTOM | RIGHT];
                        if let Some(&edges) = WMSZ.get((wparam & 0xf) as usize).filter(|&&e| e != 0) {
                            self.drag_frame(top, Some(edges));
                        }
                    }
                    SC_MAXIMIZE => {
                        self.show_window(cx, top, 3)?;
                    }
                    SC_MINIMIZE => {
                        self.show_window(cx, top, 6)?;
                    }
                    SC_RESTORE => {
                        self.show_window(cx, top, 9)?;
                    }
                    _ => {}
                }
                Ok(0)
            }
            // A press on a part of the frame (a program's own title bar, from its WM_NCHITTEST, or sent itself, as
            // programs that draw their own do): its caption moves it, its edges resize it.
            WM_NCLBUTTONDOWN => {
                const HTCAPTION: u64 = 2;
                let edges = match wparam {
                    10 => Some(LEFT),
                    11 => Some(RIGHT),
                    12 => Some(TOP),
                    13 => Some(TOP | LEFT),
                    14 => Some(TOP | RIGHT),
                    15 => Some(BOTTOM),
                    16 => Some(BOTTOM | LEFT),
                    17 => Some(BOTTOM | RIGHT),
                    _ => None,
                };
                let top = self.win32k().top_level_of(hwnd).unwrap_or(hwnd);
                if wparam == HTCAPTION || edges.is_some() {
                    self.drag_frame(top, edges);
                }
                Ok(0)
            }
            // Alt+F4 closes the window.
            WM_SYSKEYDOWN if wparam == u64::from(keys::VK_F4) && lparam & 1 << 29 != 0 => {
                let top = self.win32k().top_level_of(hwnd).unwrap_or(hwnd);
                self.send_message(cx, top, WM_SYSCOMMAND, SC_CLOSE, 0)?;
                Ok(0)
            }
            WM_GETTEXTLENGTH => Ok(self.win32k().windows.get(&hwnd).map_or(0, |w| w.text.encode_utf16().count() as u64)),
            WM_GETTEXT => {
                let text = self.win32k().windows.get(&hwnd).map(|w| w.text.clone()).unwrap_or_default();
                if wparam == 0 {
                    return Ok(0);
                }
                let units: Vec<u16> = text.encode_utf16().take(wparam as usize - 1).collect();
                let bytes: Vec<u8> = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
                let _ = self.mem(cx, true, |t| user::write(&mut t.space(), lparam, &bytes));
                Ok(units.len() as u64)
            }
            WM_ERASEBKGND => Ok(u64::from(self.erase_background(hwnd, wparam))),
            WM_PAINT => {
                // Painting nothing still validates, or WM_PAINT would come forever.
                if let Some(win) = self.win32k().windows.get_mut(&hwnd) {
                    win.invalid = Rect::default();
                    win.erase = false;
                }
                Ok(0)
            }
            _ => Ok(0),
        }
    }
}
