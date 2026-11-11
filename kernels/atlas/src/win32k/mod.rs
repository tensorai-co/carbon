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
use display::desktop::{self as dt, Button, Event, Insets, Layer, Pointer, Spec};
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
pub const WM_SYSCOMMAND: u32 = 0x112;
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
    /// The active window, as the desktop last said, and the window keys go to.
    pub active: u64,
    pub focus: u64,
    /// Each virtual key's state: 0x80 while it's down, 0x01 toggled by each press.
    pub keys: [u8; 256],
}

impl Win32k {
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
            active: 0,
            focus: 0,
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
            Event::Placed { window, rect } => {
                let Some(hwnd) = self.hwnd_of(window) else { return };
                let Some(win) = self.windows.get_mut(&hwnd) else { return };
                let before = win.client();
                win.rect = rect;
                let after = win.client();
                let resized = (before.width(), before.height()) != (after.width(), after.height());
                if resized {
                    win.invalid = after;
                    win.erase = true;
                }
                let (x, y) = self.move_origin(hwnd);
                self.deliver(hwnd, WM_MOVE, 0, words(x, y));
                if resized {
                    self.deliver(hwnd, WM_SIZE, 0, words(after.width(), after.height()));
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
                    Pointer::Wheel(notches) => {
                        let (sx, sy) = self.screen_origin(top).unwrap_or_default();
                        let target = if self.focus != 0 { self.focus } else { top };
                        (target, WM_MOUSEWHEEL, flags | ((notches * 120) as i16 as u16 as u64) << 16, words(sx + x, sy + y))
                    }
                    _ => {
                        let client = self.windows.get(&top).map(Window::client).unwrap_or_default();
                        // Outside its client area (a drag that's left it), it's the top-level window's.
                        let (target, cx, cy) = if client.contains(x, y) { self.child_at(top, x, y) } else { (top, x, y) };
                        let message = match what {
                            Pointer::Down(Button::Left) => 0x201,
                            Pointer::Up(Button::Left) => 0x202,
                            Pointer::Down(Button::Right) => 0x204,
                            Pointer::Up(Button::Right) => 0x205,
                            Pointer::Down(Button::Middle) => 0x207,
                            Pointer::Up(Button::Middle) => 0x208,
                            _ => WM_MOUSEMOVE,
                        };
                        (target, message, flags, words(cx, cy))
                    }
                };
                self.post_input(target, message, wparam, lparam, time);
            }
            Event::Key { window, code, down, modifiers } => {
                let Some(top) = self.hwnd_of(window) else { return };
                let vk = keys::vk(code);
                if vk == 0 {
                    return;
                }
                let generic = keys::generic(vk);
                let repeat = self.keys[generic as usize] & 0x80 != 0;
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
            if let Some(s) = d.content(w.shown).filter(|_| w.shown != 0) {
                shots.push((w.text.clone(), s.clone()));
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
        if shown != 0 {
            self.desktop(|d| d.activate(shown));
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
        if at < 0x1_0000 {
            let w = self.win32k();
            return w.classes.values().find(|c| u64::from(c.atom) == at).map(|c| c.name.to_lowercase());
        }
        Some(self.wstr(cx, at).to_lowercase())
    }

    // ---- Windows ----

    /// `CreateWindowExW`: a window, its WM_NCCREATE and WM_CREATE, and showing it if it's WS_VISIBLE.
    pub fn create_window(&mut self, cx: &mut Cx<'_>, a: &[u64]) -> Result<u64, Reply> {
        let (ex_style, class_at, name_at, style) = (a[0] as u32, a[1], a[2], a[3] as u32);
        let mut rect = [a[4] as i32, a[5] as i32, a[6] as i32, a[7] as i32];
        let (parent, menu, instance, param) = (a[8], a[9], a[10], a[11]);
        let Some(class_name) = self.class_named(cx, class_at) else {
            self.set_last_error(cx, ERROR_CANNOT_FIND_WND_CLASS);
            return Ok(0);
        };
        let text = self.wstr(cx, name_at);
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
                let spec = Spec { rect: r, title: text.clone(), framed, layer, owner: (owner_shown != 0).then_some(owner_shown), fill: background };
                dt::lock(&w.display).create(spec)
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
            };
            w.windows.insert(hwnd, window);
            w.queue(self.tid, &self.waiter.clone());
            hwnd
        };
        // CREATESTRUCTW: lpCreateParams, hInstance, hMenu, hwndParent, cy, cx, y, x, style, lpszName, lpszClass,
        // dwExStyle.
        let mut cs = vec![0u8; 80];
        for (at, v) in [(0, param), (8, instance), (16, menu), (24, if parent == DESKTOP { 0 } else { parent }), (56, name_at), (64, class_at)] {
            cs[at..at + 8].copy_from_slice(&v.to_le_bytes());
        }
        for (at, v) in [(32, rect[3]), (36, rect[2]), (40, rect[1]), (44, rect[0]), (48, style as i32), (72, ex_style as i32)] {
            cs[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let ok = self.call_wndproc_data(cx, hwnd, WM_NCCREATE, 0, &cs)?;
        if ok == 0 || self.call_wndproc_data(cx, hwnd, WM_CREATE, 0, &cs)? as i64 == -1 {
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
                    let mut d = dt::lock(&shared);
                    if let Some(s) = d.content(win.shown) {
                        w.shots.push((win.text.clone(), s.clone()));
                    }
                    d.destroy(win.shown);
                }
                if w.active == *h {
                    w.active = 0;
                }
                if w.focus == *h {
                    w.focus = 0;
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
            self.desktop(|d| d.set_visible(shown, show));
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
        const SHOWWINDOW: u32 = 0x40;
        const HIDEWINDOW: u32 = 0x80;
        let (moved, sized, (cx_, cy_), client) = {
            let mut w = self.win32k();
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(false) };
            let old = win.rect;
            let (x, y) = if flags & NOMOVE != 0 { (old.left, old.top) } else { (x, y) };
            let (width, height) = if flags & NOSIZE != 0 { (old.width(), old.height()) } else { (width.max(0), height.max(0)) };
            win.rect = Rect::new(x, y, x + width, y + height);
            let sized = (width, height) != (old.width(), old.height());
            if sized {
                win.invalid = win.client();
                win.erase = true;
            }
            let (rect, shown, client) = (win.rect, win.shown, win.client());
            if shown != 0 {
                dt::lock(&w.display).set_rect(shown, rect);
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

    /// Calls a window's `WndProc` on this thread.
    pub fn call_wndproc(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<u64, Reply> {
        let wndproc = self.win32k().windows.get(&hwnd).map(|w| w.wndproc);
        match wndproc {
            Some(f) if f != 0 => self.upcall(cx, f, &[hwnd, u64::from(message), wparam, lparam]),
            _ => Ok(0),
        }
    }

    /// Calls a window's `WndProc` with `data` on the stack as `lParam`.
    fn call_wndproc_data(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, data: &[u8]) -> Result<u64, Reply> {
        let wndproc = self.win32k().windows.get(&hwnd).map(|w| w.wndproc).unwrap_or(0);
        if wndproc == 0 {
            return Ok(0);
        }
        self.upcall_data(cx, wndproc, data, |at| vec![hwnd, u64::from(message), wparam, at])
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
                if wparam & 0xfff0 == SC_CLOSE {
                    self.send_message(cx, hwnd, WM_CLOSE, 0, 0)?;
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
