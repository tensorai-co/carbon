//! lambda's X server: the server end of `/tmp/.X11-unix/X0` (and its abstract twin, which libxcb tries first),
//! answered by the kernel itself, on the desktop's contract (`display::Desktop`), as the Wayland compositor is. There's
//! no window manager to run: the desktop is it, so a child of the root is a desktop window (framed, unless it asks not
//! to be or is override-redirect, which becomes a popup or an unframed window above the rest), and this server plays
//! the window manager's part of ICCCM and EWMH itself (`WM_PROTOCOLS`, `_NET_WM_STATE`, `_NET_ACTIVE_WINDOW`,
//! `_NET_WM_MOVERESIZE` and the rest). Subwindows are this server's own: each top-level has one backing of pixels the
//! desktop reads in place, and its subwindows are drawn into it, clipped, with their backgrounds and exposures.
//!
//! Here so far: the core protocol as programs use it (windows and the tree, atoms and properties, events and masks,
//! the pointer and keyboard with grabs and focus, selections (bridged to the desktop's clipboard and primary
//! selection as text), cursors (the cursor font as the desktop's named shapes, and pictures), pixmaps, GCs, images,
//! fills, lines, arcs, polygons, and the core font: the display's own `fixed`), and the extensions BIG-REQUESTS,
//! XC-MISC, SHAPE and MIT-SHM (SysV segments and memfds). Not yet: RENDER, XKEYBOARD, RANDR, XFIXES, XInput 2,
//! Present, DRI3 and GLX; fonts from the image's own files; drag and drop (XDND).

mod draw;
mod ext;
mod wire;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use display::cursor::Cursor;
use display::desktop::{self, Button, Event, Insets, Keep, Layer, Pointer, Role, Shared, Spec, Stack, WindowId, edge, state};
use display::output::Output;
use display::positioner::{Edge, Positioner};
use display::selection::{Selection, SourceId, TransferId};
use display::{Format, Pixels, Rect, Surface, keymap, keys};
use kernels_core::lock;
use layers::New;
use net::{Addr, Domain, Kind, UnixAddr, UnixName};

use self::draw::{Clip, Fill, Gc, Paint, Region, Span};
use self::wire::{Ev, Reply, Req, atom, err, ev, mask};
use crate::abi::S_IFSOCK;
use crate::fs::Open;
use crate::mm::shared::SharedMem;
use crate::{Kernel, Meta};

/// The display programs are told: `DISPLAY=:0`.
pub(crate) const DISPLAY: &str = ":0";
const DIR: &str = "/tmp/.X11-unix";
const SOCKET: &str = "/tmp/.X11-unix/X0";

/// The server's own resources, below every client's base.
const ROOT: u32 = 0x100;
/// The window `_NET_SUPPORTING_WM_CHECK` names, and that asks owners for a selection the desktop wants.
const WM_CHECK: u32 = 0x101;
const COLORMAP: u32 = 0x20;
const VISUAL: u32 = 0x21;
const VISUAL_ARGB: u32 = 0x22;
/// Each client's resource IDs: its index above these bits, its own below.
const SHIFT: u32 = 21;
const ID_MASK: u32 = (1 << SHIFT) - 1;
/// `SetInputFocus`'s PointerRoot.
const POINTER_ROOT: u32 = 1;
/// X clients' IDs on the desktop, apart from Wayland's.
pub(crate) const CLIENT_IDS: u64 = 1 << 32;
/// The most a request may be with BIG-REQUESTS, in words.
const MAX_REQUEST: u32 = 1 << 22;

/// Starts the server: its sockets made and listening, and its thread serving every connection until the machine ends.
pub(crate) fn start(k: &Arc<Kernel>, desktop: Shared) -> Result<(), String> {
    let ino = {
        let mut disk = lock(&k.disk);
        let _ = disk.put(DIR, New::Dir, &[], Meta::dir(0o1777), Meta::dir(0o1777));
        disk.put(SOCKET, New::File, &[], Meta { mode: S_IFSOCK | 0o777, ..Meta::default() }, Meta::dir(0o1777)).map_err(|e| format!("{SOCKET}: {e:?}"))?
    };
    let file = k.net.socket(Domain::Unix, Kind::Stream).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    file.bind(&Addr::Unix(UnixAddr { name: UnixName::Path(SOCKET.as_bytes().to_vec()), file: Some(ino) })).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    file.listen(64).map_err(|e| format!("{SOCKET}: {e:?}"))?;
    let mut listeners = vec![file];
    // The abstract name too: libxcb tries it first.
    if let Ok(abs) = k.net.socket(Domain::Unix, Kind::Stream)
        && abs.bind(&Addr::Unix(UnixAddr { name: UnixName::Abstract(SOCKET.as_bytes().to_vec()), file: None })).is_ok()
        && abs.listen(64).is_ok()
    {
        listeners.push(abs);
    }
    let k = k.clone();
    std::thread::Builder::new().name("lambda x11".into()).spawn(move || Server::new(k, desktop, listeners).run()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Wakes the server's thread: a client wrote, one connected, or the desktop has events.
#[derive(Default)]
struct Signal {
    woken: Mutex<bool>,
    ready: Condvar,
}

impl Signal {
    fn wake(&self) {
        *lock(&self.woken) = true;
        self.ready.notify_one();
    }

    fn wait(&self, most: Duration) {
        let mut woken = lock(&self.woken);
        if !*woken {
            woken = self.ready.wait_timeout(woken, most).map(|(g, _)| g).unwrap_or_else(|p| p.into_inner().0);
        }
        *woken = false;
    }
}

/// A top-level window's pixels, its subwindows drawn into them: read in place by the desktop. A bounding shape (SHAPE)
/// makes what's outside it transparent.
struct Backing {
    surface: Mutex<Surface>,
    shape: Mutex<Option<Region>>,
    /// A 32-bit visual's: premultiplied ARGB, as the desktop's `Argb8888` is.
    argb: bool,
}

impl Pixels for Backing {
    fn size(&self) -> (i32, i32) {
        let s = lock(&self.surface);
        (s.width, s.height)
    }

    fn format(&self) -> Format {
        if self.argb || lock(&self.shape).is_some() { Format::Argb8888 } else { Format::Xrgb8888 }
    }

    fn read_row(&self, x: i32, y: i32, out: &mut [u32]) {
        {
            let s = lock(&self.surface);
            let start = (y * s.width + x) as usize;
            match s.pixels.get(start..start + out.len()) {
                Some(row) if y < s.height => out.copy_from_slice(row),
                _ => out.fill(0),
            }
        }
        if let Some(rects) = lock(&self.shape).as_ref() {
            let rows: Vec<&Rect> = rects.iter().filter(|r| y >= r.top && y < r.bottom).collect();
            for (i, o) in out.iter_mut().enumerate() {
                let px = x + i as i32;
                *o = if rows.iter().any(|r| px >= r.left && px < r.right) {
                    if self.argb { *o } else { *o | 0xff00_0000 }
                } else {
                    0
                };
            }
        }
    }
}

/// A top-level window as it's shown on the desktop.
struct Top {
    window: WindowId,
    backing: Arc<Backing>,
    framed: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Bg {
    None,
    Pixel(u32),
    Pixmap(u32),
    ParentRelative,
}

struct Prop {
    kind: u32,
    format: u8,
    data: Vec<u8>,
}

struct Win {
    /// Whose it is (0: the server's).
    client: u32,
    parent: u32,
    /// Its children, bottom first.
    children: Vec<u32>,
    /// Its outer corner, relative to its parent's inside; its inside's size; its border's width.
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bw: i32,
    input_only: bool,
    depth: u8,
    visual: u32,
    mapped: bool,
    override_redirect: bool,
    bg: Bg,
    /// Each client's event mask on it.
    masks: Vec<(u32, u32)>,
    dont_propagate: u32,
    cursor: u32,
    colormap: u32,
    props: BTreeMap<u32, Prop>,
    top: Option<Top>,
    /// SHAPE's bounding and input regions, in its own coordinates (None: all of it).
    bounding: Option<Region>,
    input: Option<Region>,
}

impl Win {
    fn mask_of(&self, client: u32) -> u32 {
        self.masks.iter().find(|(c, _)| *c == client).map_or(0, |(_, m)| *m)
    }

    fn all_masks(&self) -> u32 {
        self.masks.iter().fold(0, |a, (_, m)| a | m)
    }
}

struct Pix {
    surface: Arc<Mutex<Surface>>,
    depth: u8,
}

enum Res {
    Pixmap(Pix),
    Gc(Box<Gc>),
    /// A font: the core font, or the cursor font (true).
    Font(bool),
    Cursor(Cursor),
    Colormap,
    /// An MIT-SHM segment.
    Shm(Arc<SharedMem>),
}

/// A pointer grab, the client's own (`GrabPointer`) or the one a press makes.
#[derive(Clone, Copy)]
struct Grab {
    client: u32,
    window: u32,
    mask: u32,
    owner_events: bool,
}

struct Client {
    socket: net::Socket,
    _watch: net::Watch,
    input: Vec<u8>,
    fds: VecDeque<Arc<Open>>,
    out: Vec<u8>,
    out_fds: Vec<Arc<Open>>,
    setup: bool,
    seq: u16,
    gone: bool,
    big: bool,
}

/// A paste for an X client, waiting for the desktop's data.
struct Paste {
    transfer: TransferId,
    client: u32,
    requestor: u32,
    selection: u32,
    target: u32,
    property: u32,
    time: u32,
}

struct Server {
    k: Arc<Kernel>,
    desktop: Shared,
    listeners: Vec<net::Socket>,
    _watches: Vec<net::Watch>,
    signal: Arc<Signal>,
    clients: BTreeMap<u32, Client>,
    next_client: u32,
    windows: HashMap<u32, Win>,
    res: HashMap<u32, (u32, Res)>,
    atoms: Vec<String>,
    atom_ids: HashMap<String, u32>,
    /// Each desktop window's top-level.
    tops: HashMap<WindowId, u32>,
    /// Each selection's owner: its window, client and time.
    selections: HashMap<u32, (u32, u32, u32)>,
    focus: u32,
    revert_to: u8,
    /// The deepest window the pointer's in, and where it is on the screen.
    under: u32,
    root_pointer: (i32, i32),
    grab: Option<Grab>,
    implicit: Option<Grab>,
    keyboard_grab: Option<(u32, u32)>,
    /// The modifiers and buttons as X's state bits, before the next event; Caps Lock; keys held.
    mods: u16,
    buttons: u8,
    caps: bool,
    keys_down: HashSet<u8>,
    scroll: (i32, i32),
    start: Instant,
    output: Output,
    placed: u32,
    /// What's been drawn in each top-level since the desktop was last told, in its backing's coordinates.
    dirty: HashMap<u32, Rect>,
    colors: Option<HashMap<String, (u8, u8, u8)>>,
    keysyms: HashMap<u8, [u32; 2]>,
    /// The request being done, for its errors: its major and minor opcodes.
    cur: (u8, u16),
    /// What the desktop's selections are offered as from an X owner's: each X selection's offer.
    offers: HashMap<u32, SourceId>,
    /// Desktop pastes from an X owner, by the property they're asked into on `WM_CHECK`.
    exports: HashMap<u32, TransferId>,
    next_export: u32,
    pastes: Vec<Paste>,
}

impl Server {
    fn new(k: Arc<Kernel>, desktop: Shared, listeners: Vec<net::Socket>) -> Server {
        let signal = Arc::new(Signal::default());
        let watches = listeners.iter().map(|l| {
            let s = signal.clone();
            l.watch(Arc::new(move || s.wake()))
        }).collect();
        let output = {
            let mut d = desktop::lock(&desktop);
            let s = signal.clone();
            // Woken beside the Wayland server, each taking its own clients' events.
            d.add_waker(Arc::new(move || s.wake()));
            d.output().clone()
        };
        let mut keysyms = HashMap::new();
        for (code, syms) in keymap::levels() {
            let first = keymap::keysym(syms[0]);
            let second = syms.get(1).map_or(0, |s| keymap::keysym(s));
            if let Ok(kc) = u8::try_from(code + 8) {
                keysyms.insert(kc, [first, second]);
            }
        }
        let mut s = Server {
            k,
            desktop,
            listeners,
            _watches: watches,
            signal,
            clients: BTreeMap::new(),
            next_client: 1,
            windows: HashMap::new(),
            res: HashMap::new(),
            atoms: wire::PREDEFINED.iter().map(|s| s.to_string()).collect(),
            atom_ids: wire::PREDEFINED.iter().enumerate().map(|(i, s)| (s.to_string(), i as u32 + 1)).collect(),
            tops: HashMap::new(),
            selections: HashMap::new(),
            focus: POINTER_ROOT,
            revert_to: 1,
            under: ROOT,
            root_pointer: (0, 0),
            grab: None,
            implicit: None,
            keyboard_grab: None,
            mods: 0,
            buttons: 0,
            caps: false,
            keys_down: HashSet::new(),
            scroll: (0, 0),
            start: Instant::now(),
            output,
            placed: 0,
            dirty: HashMap::new(),
            colors: None,
            keysyms,
            cur: (0, 0),
            offers: HashMap::new(),
            exports: HashMap::new(),
            next_export: 0,
            pastes: Vec::new(),
        };
        s.make_root();
        s
    }

    /// The root window, the window manager's check window, and the root's properties a window manager sets.
    fn make_root(&mut self) {
        let (w, h) = (self.output.width, self.output.height);
        self.windows.insert(ROOT, Win::new(0, 0, 0, 0, w, h, 0, false, 24, VISUAL));
        self.windows.get_mut(&ROOT).expect("made").mapped = true;
        self.windows.get_mut(&ROOT).expect("made").bg = Bg::Pixel(0x2b3a4a);
        let mut check = Win::new(0, ROOT, -1, -1, 1, 1, 0, true, 0, VISUAL);
        check.override_redirect = true;
        self.windows.insert(WM_CHECK, check);
        self.windows.get_mut(&ROOT).expect("made").children.push(WM_CHECK);
        let supported: Vec<u32> = [
            "_NET_SUPPORTED", "_NET_SUPPORTING_WM_CHECK", "_NET_ACTIVE_WINDOW", "_NET_CLIENT_LIST", "_NET_WM_NAME", "_NET_WM_STATE",
            "_NET_WM_STATE_FULLSCREEN", "_NET_WM_STATE_MAXIMIZED_VERT", "_NET_WM_STATE_MAXIMIZED_HORZ", "_NET_WM_STATE_HIDDEN",
            "_NET_WM_STATE_ABOVE", "_NET_WM_STATE_DEMANDS_ATTENTION", "_NET_WM_MOVERESIZE", "_NET_CLOSE_WINDOW", "_NET_WM_WINDOW_TYPE",
            "_NET_WM_WINDOW_TYPE_NORMAL", "_NET_WM_WINDOW_TYPE_DIALOG", "_NET_WM_WINDOW_TYPE_DOCK", "_NET_WM_WINDOW_TYPE_MENU",
            "_NET_WM_WINDOW_TYPE_POPUP_MENU", "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU", "_NET_WM_WINDOW_TYPE_TOOLTIP", "_NET_FRAME_EXTENTS",
            "_NET_REQUEST_FRAME_EXTENTS", "_NET_WORKAREA", "_NET_NUMBER_OF_DESKTOPS", "_NET_CURRENT_DESKTOP", "_NET_DESKTOP_GEOMETRY",
        ]
        .iter()
        .map(|n| self.atom(n))
        .collect();
        let a = |s: &mut Server, n: &str| s.atom(n);
        let (net_supported, check_atom, net_name, utf8) = (a(self, "_NET_SUPPORTED"), a(self, "_NET_SUPPORTING_WM_CHECK"), a(self, "_NET_WM_NAME"), a(self, "UTF8_STRING"));
        self.set_prop(ROOT, net_supported, atom::ATOM, 32, words(&supported));
        self.set_prop(ROOT, check_atom, atom::WINDOW, 32, words(&[WM_CHECK]));
        self.set_prop(WM_CHECK, check_atom, atom::WINDOW, 32, words(&[WM_CHECK]));
        self.set_prop(WM_CHECK, net_name, utf8, 8, b"carbon".to_vec());
        let n = a(self, "_NET_NUMBER_OF_DESKTOPS");
        self.set_prop(ROOT, n, atom::CARDINAL, 32, words(&[1]));
        let n = a(self, "_NET_CURRENT_DESKTOP");
        self.set_prop(ROOT, n, atom::CARDINAL, 32, words(&[0]));
        self.root_size_props();
        let active = a(self, "_NET_ACTIVE_WINDOW");
        self.set_prop(ROOT, active, atom::WINDOW, 32, words(&[0]));
        let list = a(self, "_NET_CLIENT_LIST");
        self.set_prop(ROOT, list, atom::WINDOW, 32, Vec::new());
    }

    /// The root's properties that follow the output: the work area, the desktop's size, and Xft's DPI.
    fn root_size_props(&mut self) {
        let (w, h) = (self.output.width as u32, self.output.height as u32);
        let n = self.atom("_NET_DESKTOP_GEOMETRY");
        self.set_prop(ROOT, n, atom::CARDINAL, 32, words(&[w, h]));
        let n = self.atom("_NET_WORKAREA");
        self.set_prop(ROOT, n, atom::CARDINAL, 32, words(&[0, 0, w, h]));
        let dpi = 96 * self.output.scale / display::output::SCALE_1X;
        self.set_prop(ROOT, atom::RESOURCE_MANAGER, atom::STRING, 8, format!("Xft.dpi:\t{dpi}\nXcursor.size:\t24\n").into_bytes());
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn run(mut self) {
        while !self.k.done.load(std::sync::atomic::Ordering::Acquire) {
            self.signal.wait(Duration::from_millis(if self.pastes.is_empty() { 200 } else { 10 }));
            self.accept();
            let ids: Vec<u32> = self.clients.keys().copied().collect();
            for id in ids {
                self.read(id);
            }
            self.desktop_events();
            self.pastes();
            self.flush_damage();
            self.flush();
        }
    }

    fn accept(&mut self) {
        for i in 0..self.listeners.len() {
            while let Ok((socket, _)) = self.listeners[i].accept() {
                let s = self.signal.clone();
                let watch = socket.watch(Arc::new(move || s.wake()));
                let id = self.next_client;
                self.next_client += 1;
                if id >= 1 << (32 - SHIFT) {
                    let _ = socket.shutdown(net::Shutdown::Both);
                    continue;
                }
                self.clients.insert(id, Client { socket, _watch: watch, input: Vec::new(), fds: VecDeque::new(), out: Vec::new(), out_fds: Vec::new(), setup: false, seq: 0, gone: false, big: false });
            }
        }
    }

    /// Everything a client's sent, read and done.
    fn read(&mut self, ci: u32) {
        let Some(c) = self.clients.get_mut(&ci) else { return };
        loop {
            match c.socket.recv(1 << 16, false) {
                Ok(got) if got.data.is_empty() && got.extra.is_none() => {
                    c.gone = true;
                    break;
                }
                Ok(got) => {
                    c.input.extend(got.data);
                    if let Some(extra) = got.extra
                        && let Ok(files) = extra.downcast::<Vec<Arc<Open>>>()
                    {
                        c.fds.extend(*files);
                    }
                }
                Err(net::Error::WouldBlock) => break,
                Err(_) => {
                    c.gone = true;
                    break;
                }
            }
        }
        loop {
            let Some(c) = self.clients.get_mut(&ci) else { return };
            if c.gone {
                break;
            }
            if !c.setup {
                if c.input.len() < 12 {
                    break;
                }
                let (n, d) = (u16::from_le_bytes([c.input[6], c.input[7]]) as usize, u16::from_le_bytes([c.input[8], c.input[9]]) as usize);
                let total = 12 + n + wire::pad(n) + d + wire::pad(d);
                if c.input.len() < total {
                    break;
                }
                let data: Vec<u8> = c.input.drain(..total).collect();
                self.setup(ci, &data);
                continue;
            }
            if c.input.len() < 4 {
                break;
            }
            let len = u16::from_le_bytes([c.input[2], c.input[3]]) as usize * 4;
            let data: Vec<u8> = if len == 0 {
                if !c.big || c.input.len() < 8 {
                    if !c.big {
                        c.gone = true;
                    }
                    break;
                }
                let len = u32::from_le_bytes([c.input[4], c.input[5], c.input[6], c.input[7]]) as usize * 4;
                if len < 8 || len > MAX_REQUEST as usize * 4 {
                    c.gone = true;
                    break;
                }
                if c.input.len() < len {
                    break;
                }
                let mut d: Vec<u8> = c.input[..4].to_vec();
                d.extend_from_slice(&c.input[8..len]);
                c.input.drain(..len);
                d
            } else {
                if c.input.len() < len {
                    break;
                }
                c.input.drain(..len).collect()
            };
            c.seq = c.seq.wrapping_add(1);
            self.request(ci, Req { data: &data });
        }
        if self.clients.get(&ci).is_some_and(|c| c.gone) {
            self.disconnect(ci);
        }
    }

    /// The connection's setup: refused for a big-endian client, or answered with the one screen.
    fn setup(&mut self, ci: u32, data: &[u8]) {
        if data[0] != b'l' {
            let reason = b"lambda's X server speaks little-endian only";
            let mut out = vec![0, reason.len() as u8, 11, 0, 0, 0];
            let words = (reason.len() + wire::pad(reason.len())) / 4;
            out.extend((words as u16).to_le_bytes());
            out.extend_from_slice(reason);
            out.resize(8 + words * 4, 0);
            self.send(ci, &out);
            if let Some(c) = self.clients.get_mut(&ci) {
                c.gone = true;
            }
            return;
        }
        let vendor = b"carbon";
        let mut r: Vec<u8> = Vec::new();
        let put32 = |r: &mut Vec<u8>, v: u32| r.extend(v.to_le_bytes());
        let put16 = |r: &mut Vec<u8>, v: u16| r.extend(v.to_le_bytes());
        put32(&mut r, 1);
        put32(&mut r, ci << SHIFT);
        put32(&mut r, ID_MASK);
        put32(&mut r, 256);
        put16(&mut r, vendor.len() as u16);
        put16(&mut r, 0xffff);
        r.extend([1, draw::DEPTHS.len() as u8, 0, 0, 32, 32, 8, 255, 0, 0, 0, 0]);
        r.extend_from_slice(vendor);
        r.resize(r.len() + wire::pad(vendor.len()), 0);
        for d in draw::DEPTHS {
            r.extend([d, draw::bits_per_pixel(d) as u8, 32, 0, 0, 0, 0, 0]);
        }
        let (w, h) = (self.output.width, self.output.height);
        let (mw, mh) = if self.output.physical_mm.0 > 0 { self.output.physical_mm } else { (w * 254 / 960, h * 254 / 960) };
        put32(&mut r, ROOT);
        put32(&mut r, COLORMAP);
        put32(&mut r, 0xffffff);
        put32(&mut r, 0);
        put32(&mut r, 0);
        put16(&mut r, w as u16);
        put16(&mut r, h as u16);
        put16(&mut r, mw as u16);
        put16(&mut r, mh as u16);
        put16(&mut r, 1);
        put16(&mut r, 1);
        put32(&mut r, VISUAL);
        r.extend([0, 0, 24, draw::DEPTHS.len() as u8]);
        for d in draw::DEPTHS {
            let visual = match d {
                24 => Some(VISUAL),
                32 => Some(VISUAL_ARGB),
                _ => None,
            };
            r.extend([d, 0]);
            put16(&mut r, u16::from(visual.is_some()));
            put32(&mut r, 0);
            if let Some(v) = visual {
                put32(&mut r, v);
                r.extend([4, 8]);
                put16(&mut r, 256);
                put32(&mut r, 0xff0000);
                put32(&mut r, 0x00ff00);
                put32(&mut r, 0x0000ff);
                put32(&mut r, 0);
            }
        }
        let mut out = vec![1, 0, 11, 0, 0, 0];
        out.extend(((r.len() / 4) as u16).to_le_bytes());
        out.extend(r);
        self.send(ci, &out);
        if let Some(c) = self.clients.get_mut(&ci) {
            c.setup = true;
        }
    }

    // ---- Output ----

    fn send(&mut self, ci: u32, bytes: &[u8]) {
        if let Some(c) = self.clients.get_mut(&ci) {
            c.out.extend_from_slice(bytes);
        }
    }

    fn seq(&self, ci: u32) -> u16 {
        self.clients.get(&ci).map_or(0, |c| c.seq)
    }

    fn reply(&mut self, ci: u32, r: Reply) {
        let bytes = r.finish(self.seq(ci));
        self.send(ci, &bytes);
    }

    fn error(&mut self, ci: u32, code: u8, value: u32) {
        let e = wire::error(code, self.seq(ci), value, self.cur.0, self.cur.1);
        self.send(ci, &e);
    }

    fn event(&mut self, ci: u32, mut e: Ev) {
        let seq = self.seq(ci);
        e.0[2..4].copy_from_slice(&seq.to_le_bytes());
        self.send(ci, &e.0);
    }

    /// An event to every client that selected any of `m` on a window. Whether any had.
    fn deliver(&mut self, w: u32, m: u32, e: Ev) -> bool {
        let targets: Vec<u32> = self.windows.get(&w).map(|win| win.masks.iter().filter(|(_, mk)| mk & m != 0).map(|(c, _)| *c).collect()).unwrap_or_default();
        for c in &targets {
            self.event(*c, e);
        }
        !targets.is_empty()
    }

    /// A structure event, to the window's own `StructureNotify` and its parent's `SubstructureNotify`: `make` builds it
    /// for the window it's reported on.
    fn structure(&mut self, w: u32, make: impl Fn(u32) -> Ev) {
        self.deliver(w, mask::STRUCTURE_NOTIFY, make(w));
        if let Some(p) = self.windows.get(&w).map(|win| win.parent)
            && w != ROOT
        {
            self.deliver(p, mask::SUBSTRUCTURE_NOTIFY, make(p));
        }
    }

    fn flush(&mut self) {
        let ids: Vec<u32> = self.clients.keys().copied().collect();
        for id in ids {
            let Some(c) = self.clients.get_mut(&id) else { continue };
            while !c.out.is_empty() {
                let mut extra = (!c.out_fds.is_empty()).then(|| Box::new(std::mem::take(&mut c.out_fds)) as net::Ancillary);
                match c.socket.send(&c.out, None, &mut extra) {
                    Ok(n) => {
                        c.out.drain(..n);
                    }
                    Err(net::Error::WouldBlock) => {
                        if let Some(extra) = extra
                            && let Ok(files) = extra.downcast::<Vec<Arc<Open>>>()
                        {
                            c.out_fds = *files;
                        }
                        break;
                    }
                    Err(_) => {
                        c.gone = true;
                        break;
                    }
                }
            }
            if c.gone {
                self.disconnect(id);
            }
        }
    }

    /// What's been drawn, told to the desktop.
    fn flush_damage(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let dirty = std::mem::take(&mut self.dirty);
        let mut d = desktop::lock(&self.desktop);
        for (top, r) in dirty {
            if let Some(t) = self.windows.get(&top).and_then(|w| w.top.as_ref()) {
                d.damage(t.window, r);
                d.commit(t.window);
            }
        }
    }

    fn mark(&mut self, top: u32, r: Rect) {
        if r.is_empty() {
            return;
        }
        let e = self.dirty.entry(top).or_default();
        *e = e.union(&r);
    }

    // ---- Atoms and properties ----

    fn atom(&mut self, name: &str) -> u32 {
        if let Some(&a) = self.atom_ids.get(name) {
            return a;
        }
        self.atoms.push(name.to_string());
        let a = self.atoms.len() as u32;
        self.atom_ids.insert(name.to_string(), a);
        a
    }

    fn atom_name(&self, a: u32) -> Option<&str> {
        self.atoms.get((a as usize).wrapping_sub(1)).map(|s| s.as_str())
    }

    /// Sets a property, telling whoever watches it.
    fn set_prop(&mut self, w: u32, property: u32, kind: u32, format: u8, data: Vec<u8>) {
        let Some(win) = self.windows.get_mut(&w) else { return };
        win.props.insert(property, Prop { kind, format, data });
        let e = Ev::new(ev::PROPERTY_NOTIFY).u32(4, w).u32(8, property).u32(12, self.start.elapsed().as_millis() as u32).u8(16, 0);
        self.deliver(w, mask::PROPERTY_CHANGE, e);
    }

    fn delete_prop(&mut self, w: u32, property: u32) {
        let Some(win) = self.windows.get_mut(&w) else { return };
        if win.props.remove(&property).is_some() {
            let e = Ev::new(ev::PROPERTY_NOTIFY).u32(4, w).u32(8, property).u32(12, self.time()).u8(16, 1);
            self.deliver(w, mask::PROPERTY_CHANGE, e);
        }
    }

    /// A property's 32-bit values.
    fn prop_words(&self, w: u32, name: &str) -> Vec<u32> {
        let Some(&a) = self.atom_ids.get(name) else { return Vec::new() };
        self.windows.get(&w).and_then(|win| win.props.get(&a)).filter(|p| p.format == 32).map(|p| p.data.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()).unwrap_or_default()
    }

    fn prop_text(&self, w: u32, a: u32) -> Option<String> {
        self.windows.get(&w).and_then(|win| win.props.get(&a)).filter(|p| p.format == 8).map(|p| {
            // STRING is Latin-1; UTF8_STRING (and the rest) UTF-8.
            if p.kind == atom::STRING { p.data.iter().map(|&b| b as char).collect() } else { String::from_utf8_lossy(&p.data).into_owned() }
        })
    }

    fn has_atom(&self, w: u32, list: &str, name: &str) -> bool {
        let Some(&a) = self.atom_ids.get(name) else { return false };
        self.prop_words(w, list).contains(&a)
    }

    // ---- The tree ----

    fn viewable(&self, mut w: u32) -> bool {
        loop {
            let Some(win) = self.windows.get(&w) else { return false };
            if !win.mapped {
                return false;
            }
            if w == ROOT {
                return true;
            }
            w = win.parent;
        }
    }

    /// Its inside's top left on the screen.
    fn root_origin(&self, w: u32) -> (i32, i32) {
        let mut at = (0, 0);
        let mut w = w;
        while w != ROOT {
            let Some(win) = self.windows.get(&w) else { break };
            at = (at.0 + win.x + win.bw, at.1 + win.y + win.bw);
            w = win.parent;
        }
        at
    }

    /// The top-level it's in (itself, if it's one).
    fn top_of(&self, mut w: u32) -> Option<u32> {
        loop {
            let win = self.windows.get(&w)?;
            if win.parent == ROOT && w != ROOT {
                return Some(w);
            }
            if w == ROOT {
                return None;
            }
            w = win.parent;
        }
    }

    fn is_inferior(&self, mut w: u32, of: u32) -> bool {
        while w != ROOT {
            let Some(win) = self.windows.get(&w) else { return false };
            if win.parent == of {
                return true;
            }
            w = win.parent;
        }
        false
    }

    /// The deepest viewable window at a point in a window's inside (the window itself if none of its children has it).
    fn window_at(&self, w: u32, x: i32, y: i32) -> u32 {
        let Some(win) = self.windows.get(&w) else { return w };
        for &c in win.children.iter().rev() {
            let Some(cw) = self.windows.get(&c) else { continue };
            if !cw.mapped || c == WM_CHECK {
                continue;
            }
            let (ix, iy) = (x - cw.x - cw.bw, y - cw.y - cw.bw);
            if x >= cw.x && x < cw.x + cw.w + 2 * cw.bw && y >= cw.y && y < cw.y + cw.h + 2 * cw.bw && cw.input.as_ref().is_none_or(|r| r.iter().any(|r| r.contains(ix, iy))) {
                return self.window_at(c, ix, iy);
            }
        }
        w
    }

    /// A viewable window's drawable part in its top-level's backing: its top-level, its inside's corner there, and the
    /// region (less what its siblings above it cover, and less its children unless `inferiors`).
    fn window_clip(&self, w: u32, inferiors: bool) -> Option<(u32, i32, i32, Region)> {
        if !self.viewable(w) {
            return None;
        }
        let top = self.top_of(w)?;
        let t = self.windows.get(&top)?;
        t.top.as_ref()?;
        let (tx, ty) = self.root_origin(top);
        let (wx, wy) = self.root_origin(w);
        let (ox, oy) = (wx - tx, wy - ty);
        let win = self.windows.get(&w)?;
        let canvas = Rect::new(0, 0, t.w, t.h);
        let mut region = vec![Rect::new(ox, oy, ox + win.w, oy + win.h).intersect(&canvas)];
        let mut a = w;
        while a != top {
            let aw = self.windows.get(&a)?;
            let p = aw.parent;
            let pw = self.windows.get(&p)?;
            let (px, py) = self.root_origin(p);
            let (px, py) = (px - tx, py - ty);
            let above = pw.children.iter().skip_while(|&&s| s != a).skip(1);
            for &s in above {
                if let Some(sw) = self.windows.get(&s)
                    && sw.mapped
                    && !sw.input_only
                {
                    region = draw::subtract(&region, &Rect::new(px + sw.x, py + sw.y, px + sw.x + sw.w + 2 * sw.bw, py + sw.y + sw.h + 2 * sw.bw));
                }
            }
            if p != top {
                region = draw::intersect(&region, &[Rect::new(px, py, px + pw.w, py + pw.h)]);
            }
            a = p;
        }
        if !inferiors {
            for &c in &win.children {
                if let Some(cw) = self.windows.get(&c)
                    && cw.mapped
                    && !cw.input_only
                {
                    region = draw::subtract(&region, &Rect::new(ox + cw.x, oy + cw.y, ox + cw.x + cw.w + 2 * cw.bw, oy + cw.y + cw.h + 2 * cw.bw));
                }
            }
        }
        region.retain(|r| !r.is_empty());
        Some((top, ox, oy, region))
    }

    // ---- Requests ----

    fn request(&mut self, ci: u32, r: Req) {
        self.cur = (r.major(), 0);
        if r.major() >= 128 {
            self.cur.1 = u16::from(r.b1());
            return self.extension(ci, r);
        }
        match r.major() {
            1 => self.create_window(ci, r),
            2 => {
                let w = r.u32(4);
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                let values = r.values(r.u32(8), 12);
                self.set_attributes(ci, w, &values);
            }
            3 => self.get_window_attributes(ci, r.u32(4)),
            4 => {
                let w = r.u32(4);
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                if w != ROOT {
                    self.destroy_window(w);
                }
            }
            5 => {
                let children = self.windows.get(&r.u32(4)).map(|w| w.children.clone()).unwrap_or_default();
                for c in children.into_iter().rev() {
                    if c != WM_CHECK {
                        self.destroy_window(c);
                    }
                }
            }
            6 => {}
            7 => self.reparent(ci, r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32),
            8 => self.map_window(ci, r.u32(4)),
            9 => {
                let children = self.windows.get(&r.u32(4)).map(|w| w.children.clone()).unwrap_or_default();
                for c in children {
                    if c != WM_CHECK {
                        self.map_window(ci, c);
                    }
                }
            }
            10 => self.unmap_window(r.u32(4)),
            11 => {
                let children = self.windows.get(&r.u32(4)).map(|w| w.children.clone()).unwrap_or_default();
                for c in children.into_iter().rev() {
                    self.unmap_window(c);
                }
            }
            12 => self.configure_window(ci, r),
            13 => {
                // CirculateWindow: the lowest mapped child raised, or the highest lowered.
                let w = r.u32(4);
                let Some(children) = self.windows.get(&w).map(|win| win.children.clone()) else { return self.error(ci, err::WINDOW, w) };
                let pick = if r.b1() == 0 { children.first() } else { children.last() };
                if let Some(&c) = pick {
                    self.restack(c, if r.b1() == 0 { 0 } else { 1 }, 0);
                    if let Some(top) = self.window_clip(w, true).map(|c| c.0) {
                        self.expose_top(top);
                    }
                }
            }
            14 => self.get_geometry(ci, r.u32(4)),
            15 => {
                let w = r.u32(4);
                let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
                let children: Vec<u32> = win.children.clone();
                let parent = if w == ROOT { 0 } else { win.parent };
                let mut rep = Reply::new(0);
                rep.u32(ROOT).u32(parent).u16(children.len() as u16).skip(14);
                for c in children {
                    rep.u32(c);
                }
                self.reply(ci, rep);
            }
            16 => {
                let name = r.str(8, r.u16(4) as usize);
                let a = if r.b1() != 0 { self.atom_ids.get(&name).copied().unwrap_or(0) } else { self.atom(&name) };
                let mut rep = Reply::new(0);
                rep.u32(a);
                self.reply(ci, rep);
            }
            17 => {
                let a = r.u32(4);
                let Some(name) = self.atom_name(a).map(|s| s.to_string()) else { return self.error(ci, err::ATOM, a) };
                let mut rep = Reply::new(0);
                rep.u16(name.len() as u16).skip(22).bytes(name.as_bytes());
                self.reply(ci, rep);
            }
            18 => self.change_property(ci, r),
            19 => {
                let w = r.u32(4);
                if !self.windows.contains_key(&w) {
                    return self.error(ci, err::WINDOW, w);
                }
                self.delete_prop(w, r.u32(8));
            }
            20 => self.get_property(ci, r),
            21 => {
                let w = r.u32(4);
                let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
                let atoms: Vec<u32> = win.props.keys().copied().collect();
                let mut rep = Reply::new(0);
                rep.u16(atoms.len() as u16).skip(22);
                for a in atoms {
                    rep.u32(a);
                }
                self.reply(ci, rep);
            }
            22 => self.set_selection_owner(ci, r.u32(4), r.u32(8), r.u32(12)),
            23 => {
                let owner = self.selections.get(&r.u32(4)).map_or(0, |o| o.0);
                let mut rep = Reply::new(0);
                rep.u32(owner);
                self.reply(ci, rep);
            }
            24 => self.convert_selection(ci, r.u32(4), r.u32(8), r.u32(12), r.u32(16), r.u32(20)),
            25 => self.send_event(ci, r),
            26 => self.grab_pointer(ci, r),
            27 => self.ungrab_pointer(ci),
            28 | 29 | 33 | 34 | 35 | 36 | 37 | 41 => {}
            30 => {
                if let Some(g) = self.grab.as_mut().filter(|g| g.client == ci) {
                    g.mask = u32::from(r.u16(12));
                }
            }
            31 => {
                let w = r.u32(4);
                self.keyboard_grab = Some((ci, w));
                let rep = Reply::new(0);
                self.reply(ci, rep);
            }
            32 => {
                if self.keyboard_grab.is_some_and(|(c, _)| c == ci) {
                    self.keyboard_grab = None;
                }
            }
            38 => self.query_pointer(ci, r.u32(4)),
            39 => {
                let mut rep = Reply::new(0);
                rep.u32(0);
                self.reply(ci, rep);
            }
            40 => self.translate(ci, r),
            42 => self.set_input_focus(ci, r.u32(4), r.b1()),
            43 => {
                let mut rep = Reply::new(self.revert_to);
                rep.u32(self.focus);
                self.reply(ci, rep);
            }
            44 => {
                let mut bits = [0u8; 32];
                for &k in &self.keys_down {
                    bits[k as usize / 8] |= 1 << (k % 8);
                }
                let mut rep = Reply::new(0);
                rep.bytes(&bits);
                self.reply(ci, rep);
            }
            45 => {
                let (fid, name) = (r.u32(4), r.str(12, r.u16(8) as usize));
                if !self.new_id(ci, fid) {
                    return;
                }
                self.res.insert(fid, (ci, Res::Font(name.eq_ignore_ascii_case("cursor"))));
            }
            46 => {
                self.res.remove(&r.u32(4));
            }
            47 => self.query_font(ci, r.u32(4)),
            48 => {
                let n = (r.data.len().saturating_sub(8)) / 2 - usize::from(r.b1() & 1 != 0);
                let width = n as i32 * draw::core_font::WIDTH;
                let mut rep = Reply::new(0);
                rep.i16(draw::core_font::ASCENT as i16).i16(draw::core_font::DESCENT as i16).i16(draw::core_font::ASCENT as i16).i16(draw::core_font::DESCENT as i16).i32(width).i32(0).i32(width);
                self.reply(ci, rep);
            }
            49 => {
                let (most, pattern) = (r.u16(4) as usize, r.str(8, r.u16(6) as usize));
                let names = font_names(&pattern, most);
                let mut rep = Reply::new(0);
                rep.u16(names.len() as u16).skip(22);
                for n in names {
                    rep.u8(n.len() as u8).bytes(n.as_bytes());
                }
                self.reply(ci, rep);
            }
            50 => {
                let (most, pattern) = (r.u16(4) as usize, r.str(8, r.u16(6) as usize));
                let names = font_names(&pattern, most);
                let count = names.len();
                for (i, n) in names.into_iter().enumerate() {
                    let mut rep = Reply::new(n.len() as u8);
                    font_info(&mut rep, false);
                    rep.u32((count - i - 1) as u32).bytes(n.as_bytes());
                    self.reply(ci, rep);
                }
                let mut last = Reply::new(0);
                last.skip(52);
                self.reply(ci, last);
            }
            51 => {}
            52 => {
                let mut rep = Reply::new(0);
                rep.u16(0);
                self.reply(ci, rep);
            }
            53 => {
                let (depth, pid, w, h) = (r.b1(), r.u32(4), r.u16(12) as i32, r.u16(14) as i32);
                if !draw::DEPTHS.contains(&depth) {
                    return self.error(ci, err::VALUE, u32::from(depth));
                }
                if w == 0 || h == 0 {
                    return self.error(ci, err::VALUE, 0);
                }
                if !self.new_id(ci, pid) {
                    return;
                }
                self.res.insert(pid, (ci, Res::Pixmap(Pix { surface: Arc::new(Mutex::new(Surface::new(w, h, 0))), depth })));
            }
            54 | 60 | 79 | 95 => {
                self.res.remove(&r.u32(4));
            }
            55 => {
                let cid = r.u32(4);
                if !self.new_id(ci, cid) {
                    return;
                }
                let mut gc = Gc::default();
                gc.set(&r.values(r.u32(12), 16));
                self.res.insert(cid, (ci, Res::Gc(Box::new(gc))));
            }
            56 => {
                let values = r.values(r.u32(8), 12);
                match self.res.get_mut(&r.u32(4)) {
                    Some((_, Res::Gc(gc))) => {
                        gc.set(&values);
                    }
                    _ => self.error(ci, err::GCONTEXT, r.u32(4)),
                }
            }
            57 => {
                let Some(from) = self.gc(r.u32(4)) else { return self.error(ci, err::GCONTEXT, r.u32(4)) };
                if let Some((_, Res::Gc(gc))) = self.res.get_mut(&r.u32(8)) {
                    gc.copy_from(&from, r.u32(12));
                }
            }
            58 => {}
            59 => {
                let rects: Vec<Rect> = r.rects(12).into_iter().map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h)).collect();
                if let Some((_, Res::Gc(gc))) = self.res.get_mut(&r.u32(4)) {
                    gc.clip_origin = (r.i16(8) as i32, r.i16(10) as i32);
                    gc.clip = Clip::Rects(rects);
                }
            }
            61 => self.clear_area(ci, r.b1() != 0, r.u32(4), r.i16(8) as i32, r.i16(10) as i32, r.u16(12) as i32, r.u16(14) as i32),
            62 | 63 => self.copy_area(ci, r),
            64 => {
                let mut pts = r.points(12);
                relative(&mut pts, r.b1());
                let spans = pts.iter().map(|&(x, y)| (y, x, x + 1)).collect();
                self.draw_spans(ci, r.u32(4), r.u32(8), spans);
            }
            65 => {
                let mut pts = r.points(12);
                relative(&mut pts, r.b1());
                self.poly_line(ci, r.u32(4), r.u32(8), &pts, false);
            }
            66 => {
                for seg in r.rest(12).as_chunks::<8>().0 {
                    let s = Req { data: seg };
                    let pts = [(s.i16(0) as i32, s.i16(2) as i32), (s.i16(4) as i32, s.i16(6) as i32)];
                    self.poly_line(ci, r.u32(4), r.u32(8), &pts, false);
                }
            }
            67 => {
                for (x, y, w, h) in r.rects(12) {
                    self.poly_line(ci, r.u32(4), r.u32(8), &[(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)], true);
                }
            }
            68 | 71 => self.arcs(ci, r),
            69 => {
                let mut pts = r.points(16);
                relative(&mut pts, r.u8(13));
                let Some(gc) = self.gc(r.u32(8)) else { return self.error(ci, err::GCONTEXT, r.u32(8)) };
                let mut spans = Vec::new();
                let fp: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
                draw::polygon_spans(&fp, gc.fill_rule == 1, &mut spans);
                self.draw_spans(ci, r.u32(4), r.u32(8), spans);
            }
            70 => {
                let mut spans = Vec::new();
                for (x, y, w, h) in r.rects(12) {
                    draw::rect_spans(x, y, w, h, &mut spans);
                }
                self.draw_spans(ci, r.u32(4), r.u32(8), spans);
            }
            72 => self.put_image(ci, r),
            73 => self.get_image(ci, r),
            74 | 75 => self.poly_text(ci, r, r.major() == 75),
            76 | 77 => self.image_text(ci, r, r.major() == 77),
            78 => {
                if self.new_id(ci, r.u32(4)) {
                    self.res.insert(r.u32(4), (ci, Res::Colormap));
                }
            }
            80 => {
                if self.new_id(ci, r.u32(4)) {
                    self.res.insert(r.u32(4), (ci, Res::Colormap));
                }
            }
            81 | 82 => {}
            83 => {
                let mut rep = Reply::new(0);
                rep.u16(1).skip(22).u32(COLORMAP);
                self.reply(ci, rep);
            }
            84 => {
                let (red, green, blue) = (r.u16(8), r.u16(10), r.u16(12));
                let pixel = (u32::from(red >> 8) << 16) | (u32::from(green >> 8) << 8) | u32::from(blue >> 8);
                let mut rep = Reply::new(0);
                rep.u16((red >> 8) * 257).u16((green >> 8) * 257).u16((blue >> 8) * 257).skip(2).u32(pixel);
                self.reply(ci, rep);
            }
            85 | 92 => {
                let name = r.str(12, r.u16(8) as usize);
                let Some((red, green, blue)) = self.color(&name) else { return self.error(ci, err::NAME, 0) };
                let (r16, g16, b16) = (u16::from(red) * 257, u16::from(green) * 257, u16::from(blue) * 257);
                let mut rep = Reply::new(0);
                if r.major() == 85 {
                    rep.u32((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue));
                }
                rep.u16(r16).u16(g16).u16(b16).u16(r16).u16(g16).u16(b16);
                self.reply(ci, rep);
            }
            86 | 87 | 89 | 90 => self.error(ci, err::ACCESS, r.u32(4)),
            88 => {}
            91 => {
                let pixels: Vec<u32> = r.rest(8).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
                let mut rep = Reply::new(0);
                rep.u16(pixels.len() as u16).skip(22);
                for p in pixels {
                    rep.u16(((p >> 16) & 0xff) as u16 * 257).u16(((p >> 8) & 0xff) as u16 * 257).u16((p & 0xff) as u16 * 257).skip(2);
                }
                self.reply(ci, rep);
            }
            93 => self.create_cursor(ci, r),
            94 => {
                let (cid, font, ch) = (r.u32(4), r.u32(8), r.u16(16));
                if !self.new_id(ci, cid) {
                    return;
                }
                let cursor = match self.res.get(&font) {
                    Some((_, Res::Font(true))) => Cursor::Shape(draw::cursor_glyph(ch)),
                    _ => Cursor::default(),
                };
                self.res.insert(cid, (ci, Res::Cursor(cursor)));
            }
            96 => {}
            97 => {
                let mut rep = Reply::new(0);
                rep.u16(r.u16(8).max(1)).u16(r.u16(10).max(1));
                self.reply(ci, rep);
            }
            98 => {
                let name = r.str(8, r.u16(4) as usize);
                let found = ext::EXTENSIONS.iter().find(|e| e.0 == name);
                let mut rep = Reply::new(0);
                match found {
                    Some(&(_, major, event, error)) => rep.u8(1).u8(major).u8(event).u8(error),
                    None => rep.u8(0).u8(0).u8(0).u8(0),
                };
                self.reply(ci, rep);
            }
            99 => {
                let mut rep = Reply::new(ext::EXTENSIONS.len() as u8);
                rep.skip(24);
                for e in ext::EXTENSIONS {
                    rep.u8(e.0.len() as u8).bytes(e.0.as_bytes());
                }
                self.reply(ci, rep);
            }
            100 | 102 | 104 | 105 | 107 | 109 | 111 | 112 | 114 | 115 | 127 => {}
            101 => {
                let (first, count) = (r.u8(4), r.u8(5));
                let mut rep = Reply::new(2);
                rep.skip(24);
                for kc in first as u16..first as u16 + count as u16 {
                    let syms = self.keysyms.get(&(kc as u8)).copied().unwrap_or([0, 0]);
                    rep.u32(syms[0]).u32(syms[1]);
                }
                self.reply(ci, rep);
            }
            103 => {
                let mut rep = Reply::new(1);
                rep.u32(0).u8(0).u8(50).u16(400).u16(100).skip(2).bytes(&[0xff; 32]);
                self.reply(ci, rep);
            }
            106 => {
                let mut rep = Reply::new(0);
                rep.u16(2).u16(1).u16(4);
                self.reply(ci, rep);
            }
            108 => {
                let mut rep = Reply::new(0);
                rep.u16(0).u16(0).u8(1).u8(1);
                self.reply(ci, rep);
            }
            110 => {
                let mut rep = Reply::new(0);
                rep.u16(0);
                self.reply(ci, rep);
            }
            113 => {
                let id = r.u32(4);
                let owner = id >> SHIFT;
                if id != 0 && owner != 0 && self.clients.contains_key(&owner) {
                    self.disconnect(owner);
                }
            }
            116 | 118 => {
                let rep = Reply::new(0);
                self.reply(ci, rep);
            }
            117 => {
                let map: Vec<u8> = (1..=10).collect();
                let mut rep = Reply::new(map.len() as u8);
                rep.skip(24).bytes(&map);
                self.reply(ci, rep);
            }
            119 => {
                let map: [u8; 16] = [50, 62, 66, 0, 37, 105, 64, 108, 77, 0, 0, 0, 133, 134, 0, 0];
                let mut rep = Reply::new(2);
                rep.skip(24).bytes(&map);
                self.reply(ci, rep);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    /// A new resource ID of a client's: its own, and not in use. An `IDChoice` error if not.
    fn new_id(&mut self, ci: u32, id: u32) -> bool {
        if id >> SHIFT != ci || self.res.contains_key(&id) || self.windows.contains_key(&id) {
            self.error(ci, err::IDCHOICE, id);
            return false;
        }
        true
    }

    fn gc(&self, id: u32) -> Option<Gc> {
        match self.res.get(&id) {
            Some((_, Res::Gc(gc))) => Some((**gc).clone()),
            _ => None,
        }
    }

    // ---- Windows ----

    fn create_window(&mut self, ci: u32, r: Req) {
        let (wid, parent) = (r.u32(4), r.u32(8));
        let Some(p) = self.windows.get(&parent) else { return self.error(ci, err::WINDOW, parent) };
        let (x, y, w, h, bw) = (r.i16(12) as i32, r.i16(14) as i32, r.u16(16) as i32, r.u16(18) as i32, r.u16(20) as i32);
        let class = r.u16(22);
        let input_only = match class {
            0 => p.input_only,
            2 => true,
            _ => false,
        };
        let depth = if input_only { 0 } else if r.b1() == 0 { p.depth } else { r.b1() };
        let visual = match r.u32(24) {
            0 if depth == 32 => VISUAL_ARGB,
            0 => p.visual,
            v => v,
        };
        if w == 0 || h == 0 {
            return self.error(ci, err::VALUE, 0);
        }
        if !self.new_id(ci, wid) {
            return;
        }
        let mut win = Win::new(ci, parent, x, y, w, h, bw, input_only, depth, visual);
        win.colormap = COLORMAP;
        self.windows.insert(wid, win);
        if let Some(p) = self.windows.get_mut(&parent) {
            p.children.push(wid);
        }
        let values = r.values(r.u32(28), 32);
        self.set_attributes(ci, wid, &values);
        let or = self.windows.get(&wid).is_some_and(|w| w.override_redirect);
        let e = Ev::new(ev::CREATE_NOTIFY).u32(4, parent).u32(8, wid).i16(12, x).i16(14, y).u16(16, w as u16).u16(18, h as u16).u16(20, bw as u16).u8(22, u8::from(or));
        self.deliver(parent, mask::SUBSTRUCTURE_NOTIFY, e);
    }

    fn set_attributes(&mut self, ci: u32, w: u32, values: &[Option<u32>; 32]) {
        if let Some(m) = values[11] {
            // Only one client may redirect the root's structure (a window manager's), or take presses on a window: the
            // root's redirect is the desktop's.
            let exclusive = mask::SUBSTRUCTURE_REDIRECT | 0x40000 | mask::BUTTON_PRESS;
            let taken = self.windows.get(&w).map_or(0, |win| win.masks.iter().filter(|(c, _)| *c != ci).fold(0, |a, (_, m)| a | m));
            if m & exclusive & taken != 0 || (w == ROOT && m & mask::SUBSTRUCTURE_REDIRECT != 0) {
                return self.error(ci, err::ACCESS, w);
            }
        }
        let mut cursor_changed = false;
        let Some(win) = self.windows.get_mut(&w) else { return };
        for (bit, v) in values.iter().enumerate() {
            let Some(v) = *v else { continue };
            match bit {
                0 => win.bg = match v {
                    0 => Bg::None,
                    1 => Bg::ParentRelative,
                    p => Bg::Pixmap(p),
                },
                1 => win.bg = Bg::Pixel(v),
                9 => win.override_redirect = v != 0,
                11 => {
                    win.masks.retain(|(c, _)| *c != ci);
                    if v != 0 {
                        win.masks.push((ci, v));
                    }
                }
                12 => win.dont_propagate = v,
                13 => win.colormap = v,
                14 => {
                    win.cursor = v;
                    cursor_changed = true;
                }
                _ => {}
            }
        }
        if cursor_changed {
            self.update_cursor();
        }
    }

    fn get_window_attributes(&mut self, ci: u32, w: u32) {
        let viewable = self.viewable(w);
        let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
        let map_state = if !win.mapped { 0 } else if viewable { 2 } else { 1 };
        let mut rep = Reply::new(0);
        rep.u32(win.visual).u16(if win.input_only { 2 } else { 1 }).u8(0).u8(1).u32(!0).u32(0).u8(0).u8(1).u8(map_state).u8(u8::from(win.override_redirect));
        rep.u32(win.colormap).u32(win.all_masks()).u32(win.mask_of(ci)).u16(win.dont_propagate as u16).skip(2);
        self.reply(ci, rep);
    }

    fn get_geometry(&mut self, ci: u32, d: u32) {
        let mut rep;
        if let Some(win) = self.windows.get(&d) {
            rep = Reply::new(win.depth);
            rep.u32(ROOT).i16(win.x as i16).i16(win.y as i16).u16(win.w as u16).u16(win.h as u16).u16(win.bw as u16);
        } else if let Some((_, Res::Pixmap(p))) = self.res.get(&d) {
            let (w, h) = { let s = lock(&p.surface); (s.width, s.height) };
            rep = Reply::new(p.depth);
            rep.u32(ROOT).i16(0).i16(0).u16(w as u16).u16(h as u16).u16(0);
        } else {
            return self.error(ci, err::DRAWABLE, d);
        }
        self.reply(ci, rep);
    }

    fn destroy_window(&mut self, w: u32) {
        let Some(win) = self.windows.get(&w) else { return };
        let children = win.children.clone();
        for c in children.into_iter().rev() {
            self.destroy_window(c);
        }
        let viewable = self.viewable(w);
        let exposed = if viewable { self.outer_in_top(w) } else { None };
        if self.windows.get(&w).is_some_and(|win| win.mapped) {
            self.hide_top(w);
        }
        self.structure(w, |e| Ev::new(ev::DESTROY_NOTIFY).u32(4, e).u32(8, w));
        let Some(win) = self.windows.remove(&w) else { return };
        if let Some(p) = self.windows.get_mut(&win.parent) {
            p.children.retain(|&c| c != w);
        }
        if self.focus == w {
            self.focus = match self.revert_to {
                2 => win.parent,
                1 => POINTER_ROOT,
                _ => 0,
            };
        }
        if self.under == w || !self.windows.contains_key(&self.under) {
            self.under = ROOT;
        }
        if self.grab.is_some_and(|g| g.window == w) {
            self.grab = None;
            desktop::lock(&self.desktop).set_capture(None);
        }
        if self.implicit.is_some_and(|g| g.window == w) {
            self.implicit = None;
        }
        if self.keyboard_grab.is_some_and(|(_, g)| g == w) {
            self.keyboard_grab = None;
        }
        self.selections.retain(|_, o| o.0 != w);
        if let Some((top, r)) = exposed {
            self.expose_area(top, r);
        }
    }

    /// A window's outer rectangle in its top-level's backing, and the top-level.
    fn outer_in_top(&self, w: u32) -> Option<(u32, Rect)> {
        let top = self.top_of(w)?;
        if top == w {
            return None;
        }
        let win = self.windows.get(&w)?;
        let (tx, ty) = self.root_origin(top);
        let (px, py) = self.root_origin(win.parent);
        let (x, y) = (px - tx + win.x, py - ty + win.y);
        Some((top, Rect::new(x, y, x + win.w + 2 * win.bw, y + win.h + 2 * win.bw)))
    }

    fn map_window(&mut self, ci: u32, w: u32) {
        let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
        if win.mapped || w == ROOT {
            return;
        }
        let (parent, or) = (win.parent, win.override_redirect);
        self.windows.get_mut(&w).expect("found").mapped = true;
        if parent == ROOT {
            self.show_top(w);
        }
        self.structure(w, |e| Ev::new(ev::MAP_NOTIFY).u32(4, e).u32(8, w).u8(12, u8::from(or)));
        if self.viewable(w) {
            if let Some((top, ox, oy, _)) = self.window_clip(w, true) {
                let r = self.windows.get(&w).map(|win| Rect::new(ox, oy, ox + win.w, oy + win.h)).unwrap_or_default();
                self.expose_area(top, r);
            }
            if parent == ROOT {
                // Where the window manager put it, as ICCCM has a window told (a synthetic ConfigureNotify).
                self.configure_notify(w, true);
                self.update_cursor();
            }
        }
    }

    fn unmap_window(&mut self, w: u32) {
        let Some(win) = self.windows.get(&w) else { return };
        if !win.mapped || w == ROOT {
            return;
        }
        let exposed = if self.viewable(w) { self.outer_in_top(w) } else { None };
        self.hide_top(w);
        if let Some(win) = self.windows.get_mut(&w) {
            win.mapped = false;
        }
        self.structure(w, |e| Ev::new(ev::UNMAP_NOTIFY).u32(4, e).u32(8, w).u8(12, 0));
        if let Some((top, r)) = exposed {
            self.expose_area(top, r);
        }
        if self.under == w || self.is_inferior(self.under, w) {
            self.under = ROOT;
        }
    }

    /// A top-level mapped: a desktop window for it, placed, framed and stacked as its properties ask, its backing
    /// attached, shown and (unless it's override-redirect) made active as the desktop allows.
    fn show_top(&mut self, w: u32) {
        let Some(win) = self.windows.get(&w) else { return };
        if win.top.is_some() {
            return;
        }
        let (client, or, bw, depth) = (win.client, win.override_redirect, win.bw, win.depth);
        let (mut x, mut y, width, height) = (win.x + bw, win.y + bw, win.w, win.h);
        let fill = match win.bg {
            Bg::Pixel(p) => p & 0xffffff,
            _ => 0xffffff,
        };
        let cid = CLIENT_IDS + u64::from(client);
        let title = self.title(w);
        let hints = self.prop_words(w, "WM_NORMAL_HINTS");
        let flags = hints.first().copied().unwrap_or(0);
        let motif = self.prop_words(w, "_MOTIF_WM_HINTS");
        let undecorated = motif.len() >= 3 && motif[0] & 2 != 0 && motif[2] == 0;
        let types = self.prop_words(w, "_NET_WM_WINDOW_TYPE");
        let type_name = types.first().and_then(|&t| self.atom_name(t)).unwrap_or("").to_string();
        let (layer, no_activate, plain) = match type_name.as_str() {
            "_NET_WM_WINDOW_TYPE_DOCK" => (Layer::Top, true, true),
            "_NET_WM_WINDOW_TYPE_DESKTOP" => (Layer::Below, true, true),
            "_NET_WM_WINDOW_TYPE_SPLASH" => (Layer::Normal, false, true),
            "_NET_WM_WINDOW_TYPE_NOTIFICATION" | "_NET_WM_WINDOW_TYPE_TOOLTIP" | "_NET_WM_WINDOW_TYPE_MENU" | "_NET_WM_WINDOW_TYPE_POPUP_MENU"
            | "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU" | "_NET_WM_WINDOW_TYPE_COMBO" | "_NET_WM_WINDOW_TYPE_DND" => (Layer::Top, true, true),
            _ => (Layer::Normal, false, false),
        };
        let transient = self.prop_words(w, "WM_TRANSIENT_FOR").first().copied().and_then(|t| self.top_of(t)).and_then(|t| self.windows.get(&t)).and_then(|t| t.top.as_ref()).map(|t| t.window);
        let framed = !or && !undecorated && !plain;
        // Placed by the desktop unless it says where it wants to be (USPosition or PPosition), or it's override-redirect.
        if !or && flags & (1 | 4) == 0 && x <= bw && y <= bw {
            self.placed += 1;
            let at = 60 + 32 * (self.placed % 10) as i32;
            (x, y) = (at, at + if framed { desktop::CAPTION + desktop::BORDER } else { 0 });
        }
        let content = Rect::new(x, y, x + width, y + height);
        let mut d = desktop::lock(&self.desktop);
        let role = if or {
            // A menu or tooltip: a popup of the client's active window (or its last), so it stays above it and goes
            // with it; on its own (above everything, never active) if the client has none shown.
            let parent = transient.or_else(|| {
                let active = d.active().filter(|a| d.client(*a) == Some(cid));
                active.or_else(|| self.tops.keys().copied().find(|t| d.client(*t) == Some(cid) && !d.is_popup(*t)))
            });
            match parent.and_then(|p| d.content_rect(p).map(|r| (p, r))) {
                Some((p, pr)) => Role::Popup { parent: p, positioner: at_point((x - pr.left, y - pr.top), (width, height)), grab: false },
                None => Role::Toplevel,
            }
        } else {
            match transient {
                Some(p) => Role::Dialog { parent: p },
                None => Role::Toplevel,
            }
        };
        let window = d.create(Spec {
            rect: Insets::of(framed).around(&content),
            title,
            framed,
            layer: if or && matches!(role, Role::Toplevel) { Layer::Top } else { layer },
            role,
            fill,
            client: cid,
            no_activate: no_activate || or,
        });
        let backing = Arc::new(Backing { surface: Mutex::new(Surface::new(width, height, fill)), shape: Mutex::new(None), argb: depth == 32 });
        let bounding = self.windows.get(&w).and_then(|win| win.bounding.clone());
        *lock(&backing.shape) = bounding;
        d.attach(window, Some(display::Buffer::new(backing.clone())));
        if hints.len() >= 9 {
            let min = if flags & 16 != 0 { (hints[5] as i32, hints[6] as i32) } else { (0, 0) };
            let max = if flags & 32 != 0 { (hints[7] as i32, hints[8] as i32) } else { (0, 0) };
            if min != (0, 0) || max != (0, 0) {
                d.set_size_limits(window, min, max);
            }
        }
        d.set_visible(window, true);
        if !or && !no_activate {
            d.request_activation(window, Some(cid), None);
        }
        let states = self.prop_words(w, "_NET_WM_STATE");
        let has = |s: &Server, n: &str| s.atom_ids.get(n).is_some_and(|a| states.contains(a));
        if has(self, "_NET_WM_STATE_FULLSCREEN") {
            d.set_fullscreen(window, true);
        } else if has(self, "_NET_WM_STATE_MAXIMIZED_VERT") && has(self, "_NET_WM_STATE_MAXIMIZED_HORZ") {
            d.set_maximized(window, true);
        }
        if has(self, "_NET_WM_STATE_ABOVE") {
            d.set_layer(window, Layer::Top);
        }
        let shown = d.content_rect(window).unwrap_or(content);
        let input = self.windows.get(&w).and_then(top_input);
        if input.is_some() {
            d.set_input_region(window, input);
        }
        d.commit(window);
        drop(d);
        self.tops.insert(window, w);
        if let Some(win) = self.windows.get_mut(&w) {
            win.top = Some(Top { window, backing, framed });
            win.x = shown.left - bw;
            win.y = shown.top - bw;
            win.w = shown.width().max(1);
            win.h = shown.height().max(1);
        }
        self.resize_backing(w);
        if !or {
            let wm_state = self.atom("WM_STATE");
            self.set_prop(w, wm_state, wm_state, 32, words(&[1, 0]));
            let i = Insets::of(framed);
            let extents = self.atom("_NET_FRAME_EXTENTS");
            self.set_prop(w, extents, atom::CARDINAL, 32, words(&[i.left as u32, i.right as u32, i.top as u32, i.bottom as u32]));
            self.client_list();
        }
    }

    /// A top-level unmapped or gone: its desktop window with it.
    fn hide_top(&mut self, w: u32) {
        let Some(top) = self.windows.get_mut(&w).and_then(|win| win.top.take()) else { return };
        self.tops.remove(&top.window);
        self.dirty.remove(&w);
        desktop::lock(&self.desktop).destroy(top.window);
        if self.windows.get(&w).is_some_and(|win| !win.override_redirect) {
            let wm_state = self.atom("WM_STATE");
            self.delete_prop(w, wm_state);
            self.client_list();
        }
        if self.focus == w || self.is_inferior(self.focus, w) {
            self.set_focus(0);
        }
    }

    /// `_NET_CLIENT_LIST`: every managed top-level shown.
    fn client_list(&mut self) {
        let mut list: Vec<u32> = self.tops.values().copied().filter(|t| self.windows.get(t).is_some_and(|w| !w.override_redirect)).collect();
        list.sort_unstable();
        let a = self.atom("_NET_CLIENT_LIST");
        self.set_prop(ROOT, a, atom::WINDOW, 32, words(&list));
    }

    fn title(&self, w: u32) -> String {
        self.atom_ids.get("_NET_WM_NAME").and_then(|&a| self.prop_text(w, a)).or_else(|| self.prop_text(w, atom::WM_NAME)).unwrap_or_default()
    }

    /// A top-level's backing made its size again (what's still inside kept).
    fn resize_backing(&mut self, w: u32) {
        let Some(win) = self.windows.get(&w) else { return };
        let Some(t) = &win.top else { return };
        let (ww, wh) = (win.w, win.h);
        let fill = match win.bg {
            Bg::Pixel(p) => p,
            _ => 0xffffff,
        };
        let mut s = lock(&t.backing.surface);
        if (s.width, s.height) != (ww, wh) {
            s.resize(ww, wh, fill);
        }
    }

    /// A `ConfigureNotify` for a window as it is (synthetic, for a top-level the window manager moved).
    fn configure_notify(&mut self, w: u32, synthetic: bool) {
        let Some(win) = self.windows.get(&w) else { return };
        let (x, y, ww, h, bw, or) = (win.x, win.y, win.w, win.h, win.bw, win.override_redirect);
        let above = self.windows.get(&win.parent).and_then(|p| {
            let at = p.children.iter().position(|&c| c == w)?;
            at.checked_sub(1).map(|i| p.children[i])
        }).unwrap_or(0);
        let code = if synthetic { ev::CONFIGURE_NOTIFY | 0x80 } else { ev::CONFIGURE_NOTIFY };
        let make = |e: u32| Ev::new(code).u32(4, e).u32(8, w).u32(12, above).i16(16, x).i16(18, y).u16(20, ww as u16).u16(22, h as u16).u16(24, bw as u16).u8(26, u8::from(or));
        if synthetic {
            self.deliver(w, mask::STRUCTURE_NOTIFY, make(w));
        } else {
            self.structure(w, make);
        }
    }

    fn configure_window(&mut self, ci: u32, r: Req) {
        let w = r.u32(4);
        let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
        let m = u32::from(r.u16(8));
        let values = r.values(m, 12);
        let (mut x, mut y, mut ww, mut h, mut bw) = (win.x, win.y, win.w, win.h, win.bw);
        let before = self.outer_in_top(w);
        if let Some(v) = values[0] { x = v as i16 as i32; }
        if let Some(v) = values[1] { y = v as i16 as i32; }
        if let Some(v) = values[2] { ww = (v as u16 as i32).max(1); }
        if let Some(v) = values[3] { h = (v as u16 as i32).max(1); }
        if let Some(v) = values[4] { bw = v as u16 as i32; }
        let sized = (ww, h) != (win.w, win.h);
        let (parent, top, or) = (win.parent, win.top.as_ref().map(|t| (t.window, t.framed)), win.override_redirect);
        if let Some(stack) = values[6] {
            self.restack(w, stack, values[5].unwrap_or(0));
        }
        if parent == ROOT
            && let Some((window, framed)) = top
        {
            // The desktop places it, as the window manager would grant it.
            let content = Rect::new(x + bw, y + bw, x + bw + ww, y + bw + h);
            let mut d = desktop::lock(&self.desktop);
            if d.is_popup(window) {
                let p = d.window_info(window).and_then(|i| i.parent).and_then(|p| d.content_rect(p)).unwrap_or_default();
                d.set_positioner(window, at_point((content.left - p.left, content.top - p.top), (ww, h)));
            } else {
                d.set_rect(window, Insets::of(framed).around(&content));
            }
            if values[6].is_some() && !or {
                d.stack(window, if values[6] == Some(1) { Stack::Bottom } else { Stack::Top });
            }
            let shown = d.content_rect(window).unwrap_or(content);
            drop(d);
            (x, y, ww, h) = (shown.left - bw, shown.top - bw, shown.width().max(1), shown.height().max(1));
        }
        let changed = {
            let win = self.windows.get_mut(&w).expect("found");
            let changed = (win.x, win.y, win.w, win.h, win.bw) != (x, y, ww, h, bw);
            (win.x, win.y, win.w, win.h, win.bw) = (x, y, ww, h, bw);
            changed
        };
        if parent == ROOT {
            self.resize_backing(w);
        }
        self.configure_notify(w, false);
        if !changed && values[6].is_none() {
            return;
        }
        if parent == ROOT {
            if sized && let Some(top) = self.top_of(w) {
                self.expose_top(top);
            }
        } else if self.viewable(w)
            && let Some((top, after)) = self.outer_in_top(w)
        {
            let area = before.map_or(after, |(_, b)| b.union(&after));
            self.expose_area(top, area);
        }
    }

    /// A window moved in its parent's stack: above or below a sibling, or all of them (`mode` 0 Above, 1 Below; the
    /// others as those).
    fn restack(&mut self, w: u32, mode: u32, sibling: u32) {
        let Some(parent) = self.windows.get(&w).map(|win| win.parent) else { return };
        let Some(p) = self.windows.get_mut(&parent) else { return };
        p.children.retain(|&c| c != w);
        let at = match (mode, p.children.iter().position(|&c| c == sibling)) {
            (0 | 2, Some(i)) => i + 1,
            (1 | 3, Some(i)) => i,
            (1 | 3, None) => 0,
            _ => p.children.len(),
        };
        p.children.insert(at.min(p.children.len()), w);
    }

    fn reparent(&mut self, ci: u32, w: u32, parent: u32, x: i32, y: i32) {
        if !self.windows.contains_key(&w) || w == ROOT {
            return self.error(ci, err::WINDOW, w);
        }
        if !self.windows.contains_key(&parent) || parent == w || self.is_inferior(parent, w) {
            return self.error(ci, err::MATCH, parent);
        }
        let was_mapped = self.windows.get(&w).is_some_and(|win| win.mapped);
        if was_mapped {
            self.unmap_window(w);
        }
        let old = self.windows.get(&w).map(|win| win.parent).unwrap_or(ROOT);
        if let Some(p) = self.windows.get_mut(&old) {
            p.children.retain(|&c| c != w);
        }
        if let Some(p) = self.windows.get_mut(&parent) {
            p.children.push(w);
        }
        let or = {
            let win = self.windows.get_mut(&w).expect("found");
            win.parent = parent;
            win.x = x;
            win.y = y;
            win.override_redirect
        };
        let e = |ev_w: u32| Ev::new(ev::REPARENT_NOTIFY).u32(4, ev_w).u32(8, w).u32(12, parent).i16(16, x).i16(18, y).u8(20, u8::from(or));
        self.deliver(w, mask::STRUCTURE_NOTIFY, e(w));
        self.deliver(old, mask::SUBSTRUCTURE_NOTIFY, e(old));
        self.deliver(parent, mask::SUBSTRUCTURE_NOTIFY, e(parent));
        if was_mapped {
            self.map_window(ci, w);
        }
    }

    // ---- Exposure ----

    /// The parts of every viewable window in a top-level's tree within `area` (backing coordinates), each painted
    /// with its background and sent an `Expose` for what it lost.
    fn expose_area(&mut self, w: u32, area: Rect) {
        let Some(win) = self.windows.get(&w) else { return };
        if !win.mapped || win.input_only {
            return;
        }
        let children = win.children.clone();
        if let Some((bbox, ox, oy)) = self.paint_background(w, Some(area)) {
            let e = Ev::new(ev::EXPOSE).u32(4, w).u16(8, (bbox.left - ox) as u16).u16(10, (bbox.top - oy) as u16).u16(12, bbox.width() as u16).u16(14, bbox.height() as u16).u16(16, 0);
            self.deliver(w, mask::EXPOSURE, e);
        }
        for c in children {
            self.expose_area(c, area);
        }
    }

    /// The whole of a top-level exposed.
    fn expose_top(&mut self, top: u32) {
        let Some(win) = self.windows.get(&top) else { return };
        let r = Rect::new(0, 0, win.w, win.h);
        self.expose_area(top, r);
    }

    /// A window's background painted where it shows (within `area`, if given): what it painted (backing
    /// coordinates), and its inside's corner there. None where none of it shows.
    fn paint_background(&mut self, w: u32, area: Option<Rect>) -> Option<(Rect, i32, i32)> {
        let (top, ox, oy, mut clip) = self.window_clip(w, false)?;
        if let Some(a) = area {
            clip = draw::intersect(&clip, &[a]);
        }
        let bbox = clip.iter().fold(Rect::default(), |a, r| a.union(r));
        if bbox.is_empty() {
            return None;
        }
        // ParentRelative: the parent's background, from the parent's corner.
        let (mut bg, mut origin, mut at) = (self.windows.get(&w)?.bg, (ox, oy), w);
        while bg == Bg::ParentRelative {
            at = self.windows.get(&at)?.parent;
            bg = self.windows.get(&at)?.bg;
            let (tx, ty) = self.root_origin(top);
            let (px, py) = self.root_origin(at);
            origin = (px - tx, py - ty);
        }
        let fill = match bg {
            Bg::Pixel(p) => Fill::Solid.with(p),
            Bg::Pixmap(id) => match self.res.get(&id) {
                Some((_, Res::Pixmap(p))) => (Fill::Tiled(lock(&p.surface).clone(), origin.0, origin.1), 0),
                _ => return Some((bbox, ox, oy)),
            },
            _ => return Some((bbox, ox, oy)),
        };
        let backing = self.windows.get(&top)?.top.as_ref()?.backing.clone();
        let mut spans = Vec::new();
        for r in &clip {
            draw::rect_spans(r.left, r.top, r.width(), r.height(), &mut spans);
        }
        let paint = Paint { function: draw::GX_COPY, fg: fill.1, bg: 0, depth: 32, fill: fill.0, mask: None };
        draw::fill_spans(&mut lock(&backing.surface), &spans, &paint, &clip);
        self.mark(top, bbox);
        Some((bbox, ox, oy))
    }

    #[allow(clippy::too_many_arguments)]
    fn clear_area(&mut self, ci: u32, exposures: bool, w: u32, x: i32, y: i32, width: i32, height: i32) {
        let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
        let (width, height) = (if width == 0 { win.w - x } else { width }, if height == 0 { win.h - y } else { height });
        let Some((_, ox, oy, _)) = self.window_clip(w, false) else { return };
        let area = Rect::new(ox + x, oy + y, ox + x + width, oy + y + height);
        let painted = self.paint_background(w, Some(area));
        if exposures && let Some((bbox, ox, oy)) = painted {
            let e = Ev::new(ev::EXPOSE).u32(4, w).u16(8, (bbox.left - ox) as u16).u16(10, (bbox.top - oy) as u16).u16(12, bbox.width() as u16).u16(14, bbox.height() as u16);
            self.deliver(w, mask::EXPOSURE, e);
        }
    }

    // ---- Properties ----

    fn change_property(&mut self, ci: u32, r: Req) {
        let (mode, w, property, kind, format, n) = (r.b1(), r.u32(4), r.u32(8), r.u32(12), r.u8(16), r.u32(20) as usize);
        if !self.windows.contains_key(&w) {
            return self.error(ci, err::WINDOW, w);
        }
        if self.atom_name(property).is_none() {
            return self.error(ci, err::ATOM, property);
        }
        if !matches!(format, 8 | 16 | 32) {
            return self.error(ci, err::VALUE, u32::from(format));
        }
        let data = r.bytes(24, n * usize::from(format / 8)).to_vec();
        let win = self.windows.get_mut(&w).expect("found");
        let new = match (mode, win.props.get(&property)) {
            (1 | 2, Some(old)) if old.kind != kind || old.format != format => return self.error(ci, err::MATCH, property),
            (1, Some(old)) => [data, old.data.clone()].concat(),
            (2, Some(old)) => [old.data.clone(), data].concat(),
            _ => data,
        };
        self.set_prop(w, property, kind, format, new);
        self.property_changed(w, property);
    }

    /// A top-level's property the window manager reads, changed: told to the desktop.
    fn property_changed(&mut self, w: u32, property: u32) {
        let Some(window) = self.windows.get(&w).and_then(|win| win.top.as_ref()).map(|t| t.window) else { return };
        let name = self.atom_name(property).unwrap_or("").to_string();
        match name.as_str() {
            "WM_NAME" | "_NET_WM_NAME" => {
                let title = self.title(w);
                desktop::lock(&self.desktop).set_title(window, &title);
            }
            "WM_NORMAL_HINTS" => {
                let hints = self.prop_words(w, "WM_NORMAL_HINTS");
                if hints.len() >= 9 {
                    let min = if hints[0] & 16 != 0 { (hints[5] as i32, hints[6] as i32) } else { (0, 0) };
                    let max = if hints[0] & 32 != 0 { (hints[7] as i32, hints[8] as i32) } else { (0, 0) };
                    desktop::lock(&self.desktop).set_size_limits(window, min, max);
                }
            }
            "_MOTIF_WM_HINTS" => {
                let motif = self.prop_words(w, "_MOTIF_WM_HINTS");
                let framed = !(motif.len() >= 3 && motif[0] & 2 != 0 && motif[2] == 0) && !self.windows.get(&w).is_some_and(|win| win.override_redirect);
                desktop::lock(&self.desktop).set_framed(window, framed, Keep::Content);
                if let Some(t) = self.windows.get_mut(&w).and_then(|win| win.top.as_mut()) {
                    t.framed = framed;
                }
            }
            "WM_HINTS" => {
                let hints = self.prop_words(w, "WM_HINTS");
                let urgent = hints.first().is_some_and(|f| f & 256 != 0);
                desktop::lock(&self.desktop).set_attention(window, urgent);
            }
            "WM_TRANSIENT_FOR" => {
                let owner = self.prop_words(w, "WM_TRANSIENT_FOR").first().copied().and_then(|t| self.top_of(t)).and_then(|t| self.windows.get(&t)).and_then(|t| t.top.as_ref()).map(|t| t.window);
                desktop::lock(&self.desktop).set_owner(window, owner);
            }
            _ => {}
        }
    }

    fn get_property(&mut self, ci: u32, r: Req) {
        let (delete, w, property, kind, offset, length) = (r.b1() != 0, r.u32(4), r.u32(8), r.u32(12), r.u32(16) as usize, r.u32(20) as usize);
        let Some(win) = self.windows.get(&w) else { return self.error(ci, err::WINDOW, w) };
        let Some(p) = win.props.get(&property) else {
            let mut rep = Reply::new(0);
            rep.u32(0).u32(0).u32(0);
            return self.reply(ci, rep);
        };
        if kind != 0 && kind != p.kind {
            let mut rep = Reply::new(p.format);
            rep.u32(p.kind).u32(p.data.len() as u32).u32(0);
            return self.reply(ci, rep);
        }
        let n = p.data.len();
        let start = offset.saturating_mul(4);
        if start > n {
            return self.error(ci, err::VALUE, offset as u32);
        }
        let take = (n - start).min(length.saturating_mul(4));
        let after = n - start - take;
        let unit = usize::from(p.format / 8).max(1);
        let mut rep = Reply::new(p.format);
        rep.u32(p.kind).u32(after as u32).u32((take / unit) as u32).skip(12).bytes(&p.data[start..start + take]);
        self.reply(ci, rep);
        if delete && after == 0 {
            self.delete_prop(w, property);
        }
    }

    // ---- Selections ----

    fn set_selection_owner(&mut self, ci: u32, owner: u32, selection: u32, time: u32) {
        if owner != 0 && !self.windows.contains_key(&owner) {
            return self.error(ci, err::WINDOW, owner);
        }
        let time = if time == 0 { self.time() } else { time };
        let old = if owner == 0 { self.selections.remove(&selection) } else { self.selections.insert(selection, (owner, ci, time)) };
        if let Some((ow, oc, _)) = old
            && (ow != owner || oc != ci)
        {
            let e = Ev::new(ev::SELECTION_CLEAR).u32(4, time).u32(8, ow).u32(12, selection);
            self.event(oc, e);
        }
        // The desktop's own selections follow X's CLIPBOARD and PRIMARY, offered as text: what an X owner has is asked
        // of it when the desktop pastes.
        let Some(which) = self.desktop_selection(selection) else { return };
        let mut d = desktop::lock(&self.desktop);
        if owner == 0 {
            if self.offers.remove(&selection).is_some_and(|s| d.selection_source(which) == Some(s)) {
                d.clear_selection(which);
            }
            return;
        }
        // Offered from a desktop window of the owner's, so a paste is asked of it (toolkits own selections with a
        // window that's never shown: then any of the client's that is). With none shown, it stays X's own.
        let shown = |t: u32| self.windows.get(&t).and_then(|t| t.top.as_ref()).map(|t| t.window);
        let window = self.top_of(owner).and_then(shown).or_else(|| {
            let mut mine: Vec<u32> = self.tops.values().copied().filter(|t| self.windows.get(t).is_some_and(|w| w.client == ci)).collect();
            mine.sort_unstable();
            mine.into_iter().find_map(shown)
        });
        let Some(window) = window else { return };
        let source = d.offer(which, Some(window), TEXT_MIMES.iter().map(|s| s.to_string()).collect());
        self.offers.insert(selection, source);
    }

    fn desktop_selection(&mut self, selection: u32) -> Option<Selection> {
        match self.atom_name(selection) {
            Some("CLIPBOARD") => Some(Selection::Clipboard),
            Some("PRIMARY") => Some(Selection::Primary),
            _ => None,
        }
    }

    fn convert_selection(&mut self, ci: u32, requestor: u32, selection: u32, target: u32, property: u32, time: u32) {
        if !self.windows.contains_key(&requestor) {
            return self.error(ci, err::WINDOW, requestor);
        }
        if let Some(&(ow, oc, _)) = self.selections.get(&selection) {
            let e = Ev::new(ev::SELECTION_REQUEST).u32(4, time).u32(8, ow).u32(12, requestor).u32(16, selection).u32(20, target).u32(24, property);
            return self.event(oc, e);
        }
        let property = if property == 0 { target } else { property };
        // Owned outside X (a Wayland client, the host): answered from the desktop's offer.
        if let Some(which) = self.desktop_selection(selection) {
            let offered = desktop::lock(&self.desktop).selection(which).map(|(_, mimes)| mimes);
            if let Some(mimes) = offered {
                let target_name = self.atom_name(target).unwrap_or("").to_string();
                if target_name == "TARGETS" {
                    let targets: Vec<u32> = ["TARGETS", "UTF8_STRING", "STRING", "TEXT", "text/plain;charset=utf-8", "text/plain"].iter().map(|n| self.atom(n)).collect();
                    self.set_prop(requestor, property, atom::ATOM, 32, words(&targets));
                    return self.selection_notify(ci, requestor, selection, target, property, time);
                }
                if TEXT_MIMES.contains(&target_name.as_str())
                    && let Some(mime) = TEXT_MIMES.iter().find(|m| mimes.iter().any(|o| o == *m)).or(mimes.first().map(|s| s.as_str()).as_ref().and(None))
                {
                    let transfer = desktop::lock(&self.desktop).request(which, mime, None);
                    if let Some(transfer) = transfer {
                        self.pastes.push(Paste { transfer, client: ci, requestor, selection, target, property, time });
                        return;
                    }
                }
            }
        }
        self.selection_notify(ci, requestor, selection, target, 0, time);
    }

    fn selection_notify(&mut self, ci: u32, requestor: u32, selection: u32, target: u32, property: u32, time: u32) {
        let e = Ev::new(ev::SELECTION_NOTIFY).u32(4, time).u32(8, requestor).u32(12, selection).u32(16, target).u32(20, property);
        self.event(ci, e);
    }

    /// Pastes from the desktop that have come, written into their requestors' properties.
    fn pastes(&mut self) {
        if self.pastes.is_empty() {
            return;
        }
        let mut came = Vec::new();
        {
            let mut d = desktop::lock(&self.desktop);
            self.pastes.retain(|p| match d.take_transfer(p.transfer) {
                Some(data) => {
                    came.push((p.client, p.requestor, p.selection, p.target, p.property, p.time, data));
                    false
                }
                None => true,
            });
        }
        for (ci, requestor, selection, target, property, time, data) in came {
            match data {
                Some(data) if self.windows.contains_key(&requestor) => {
                    let kind = if self.atom_name(target) == Some("STRING") { atom::STRING } else { self.atom("UTF8_STRING") };
                    self.set_prop(requestor, property, kind, 8, data);
                    self.selection_notify(ci, requestor, selection, target, property, time);
                }
                _ => self.selection_notify(ci, requestor, selection, target, 0, time),
            }
        }
    }

    // ---- Events between clients, and the window manager's client messages ----

    fn send_event(&mut self, ci: u32, r: Req) {
        let (propagate, dest, m) = (r.b1() != 0, r.u32(4), r.u32(8));
        let mut e = Ev([0; 32]);
        e.0.copy_from_slice(r.bytes(12, 32).iter().chain(std::iter::repeat(&0)).take(32).copied().collect::<Vec<u8>>().as_slice());
        e.0[0] |= 0x80;
        let dest = match dest {
            0 => self.under,
            1 => if self.focus > POINTER_ROOT { self.focus } else { self.under },
            w => w,
        };
        if !self.windows.contains_key(&dest) {
            return self.error(ci, err::WINDOW, dest);
        }
        // An owner's answer to a selection the desktop asked of it.
        if dest == WM_CHECK && e.code() == ev::SELECTION_NOTIFY {
            let property = u32::from_le_bytes([e.0[20], e.0[21], e.0[22], e.0[23]]);
            return self.export_done(property);
        }
        if dest == ROOT && e.code() == ev::CLIENT_MESSAGE && m & (mask::SUBSTRUCTURE_REDIRECT | mask::SUBSTRUCTURE_NOTIFY) != 0 {
            self.client_message(&e);
        }
        if m == 0 {
            let owner = self.windows.get(&dest).map_or(0, |w| w.client);
            if owner != 0 {
                self.event(owner, e);
            }
            return;
        }
        let mut w = dest;
        loop {
            if self.deliver(w, m, e) || !propagate || w == ROOT {
                return;
            }
            let Some(win) = self.windows.get(&w) else { return };
            if win.dont_propagate & m != 0 {
                return;
            }
            w = win.parent;
            e.0[4..8].copy_from_slice(&w.to_le_bytes());
        }
    }

    /// EWMH's and ICCCM's requests of the window manager, sent to the root.
    fn client_message(&mut self, e: &Ev) {
        let w = u32::from_le_bytes([e.0[4], e.0[5], e.0[6], e.0[7]]);
        let kind = u32::from_le_bytes([e.0[8], e.0[9], e.0[10], e.0[11]]);
        let data: Vec<u32> = (0..5).map(|i| u32::from_le_bytes([e.0[12 + 4 * i], e.0[13 + 4 * i], e.0[14 + 4 * i], e.0[15 + 4 * i]])).collect();
        let Some(window) = self.windows.get(&w).and_then(|win| win.top.as_ref()).map(|t| t.window) else { return };
        let client = self.windows.get(&w).map_or(0, |win| win.client);
        let name = self.atom_name(kind).unwrap_or("").to_string();
        let mut d = desktop::lock(&self.desktop);
        match name.as_str() {
            "_NET_ACTIVE_WINDOW" => {
                let by = if data[0] == 2 { None } else { Some(CLIENT_IDS + u64::from(client)) };
                d.request_activation(window, by, None);
            }
            "_NET_WM_STATE" => {
                let states = d.states(window);
                for &p in &data[1..3] {
                    let on = |current: bool| match data[0] {
                        0 => false,
                        1 => true,
                        _ => !current,
                    };
                    match self.atom_name(p).unwrap_or("") {
                        "_NET_WM_STATE_FULLSCREEN" => d.set_fullscreen(window, on(states & state::FULLSCREEN != 0)),
                        "_NET_WM_STATE_MAXIMIZED_VERT" | "_NET_WM_STATE_MAXIMIZED_HORZ" => d.set_maximized(window, on(states & state::MAXIMIZED != 0)),
                        "_NET_WM_STATE_HIDDEN" => d.set_minimized(window, on(states & state::MINIMIZED != 0)),
                        "_NET_WM_STATE_ABOVE" => {
                            let above = d.window_info(window).is_some_and(|i| i.layer == Layer::Top);
                            d.set_layer(window, if on(above) { Layer::Top } else { Layer::Normal });
                        }
                        "_NET_WM_STATE_DEMANDS_ATTENTION" => {
                            let now = d.attention(window);
                            d.set_attention(window, on(now));
                        }
                        _ => {}
                    }
                }
            }
            "_NET_WM_MOVERESIZE" => match data[2] {
                8 | 10 => {
                    d.begin_move(window);
                }
                dir @ 0..=7 => {
                    let edges = [edge::TOP | edge::LEFT, edge::TOP, edge::TOP | edge::RIGHT, edge::RIGHT, edge::BOTTOM | edge::RIGHT, edge::BOTTOM, edge::BOTTOM | edge::LEFT, edge::LEFT][dir as usize];
                    d.begin_resize(window, edges);
                }
                _ => {}
            },
            "_NET_CLOSE_WINDOW" => d.request_close(window),
            "WM_CHANGE_STATE" if data[0] == 3 => d.set_minimized(window, true),
            _ => {}
        }
    }

    // ---- The pointer and keyboard ----

    fn grab_pointer(&mut self, ci: u32, r: Req) {
        let (owner_events, w, m, cursor) = (r.b1() != 0, r.u32(4), u32::from(r.u16(8)), r.u32(16));
        if !self.viewable(w) {
            let rep = Reply::new(3);
            return self.reply(ci, rep);
        }
        if self.grab.is_some_and(|g| g.client != ci) {
            let rep = Reply::new(1);
            return self.reply(ci, rep);
        }
        self.grab = Some(Grab { client: ci, window: w, mask: m, owner_events });
        if let Some(window) = self.top_of(w).and_then(|t| self.windows.get(&t)).and_then(|t| t.top.as_ref()).map(|t| t.window) {
            let mut d = desktop::lock(&self.desktop);
            d.set_capture(Some(window));
            if let Some((_, Res::Cursor(c))) = self.res.get(&cursor) {
                d.set_cursor(window, c.clone());
            }
        }
        let rep = Reply::new(0);
        self.reply(ci, rep);
    }

    fn ungrab_pointer(&mut self, ci: u32) {
        if self.grab.is_some_and(|g| g.client == ci) {
            self.grab = None;
            desktop::lock(&self.desktop).set_capture(None);
            self.update_cursor();
        }
    }

    fn query_pointer(&mut self, ci: u32, w: u32) {
        if !self.windows.contains_key(&w) {
            return self.error(ci, err::WINDOW, w);
        }
        let (rx, ry) = self.root_pointer;
        let (ox, oy) = self.root_origin(w);
        let child = self.child_at(w, rx - ox, ry - oy);
        let mut rep = Reply::new(1);
        rep.u32(ROOT).u32(child).i16(rx as i16).i16(ry as i16).i16((rx - ox) as i16).i16((ry - oy) as i16).u16(self.state());
        self.reply(ci, rep);
    }

    /// The child of a window that has a point in its inside (0 for none).
    fn child_at(&self, w: u32, x: i32, y: i32) -> u32 {
        let Some(win) = self.windows.get(&w) else { return 0 };
        win.children.iter().rev().copied().find(|c| {
            self.windows.get(c).is_some_and(|cw| cw.mapped && *c != WM_CHECK && x >= cw.x && x < cw.x + cw.w + 2 * cw.bw && y >= cw.y && y < cw.y + cw.h + 2 * cw.bw)
        }).unwrap_or(0)
    }

    fn translate(&mut self, ci: u32, r: Req) {
        let (src, dst, x, y) = (r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
        if !self.windows.contains_key(&src) {
            return self.error(ci, err::WINDOW, src);
        }
        if !self.windows.contains_key(&dst) {
            return self.error(ci, err::WINDOW, dst);
        }
        let (sx, sy) = self.root_origin(src);
        let (dx, dy) = self.root_origin(dst);
        let (tx, ty) = (x + sx - dx, y + sy - dy);
        let child = self.child_at(dst, tx, ty);
        let mut rep = Reply::new(1);
        rep.u32(child).i16(tx as i16).i16(ty as i16);
        self.reply(ci, rep);
    }

    fn set_input_focus(&mut self, ci: u32, focus: u32, revert_to: u8) {
        if focus > POINTER_ROOT && !self.windows.contains_key(&focus) {
            return self.error(ci, err::WINDOW, focus);
        }
        if focus > POINTER_ROOT && !self.viewable(focus) {
            return self.error(ci, err::MATCH, focus);
        }
        self.revert_to = revert_to;
        self.set_focus(focus);
        // Focusing another top-level asks the desktop to make it active, as its focus policy allows.
        if let Some(window) = self.top_of(focus).and_then(|t| self.windows.get(&t)).and_then(|t| t.top.as_ref()).map(|t| t.window) {
            let mut d = desktop::lock(&self.desktop);
            if d.active() != Some(window) && !d.is_popup(window) {
                d.request_activation(window, Some(CLIENT_IDS + u64::from(ci)), None);
            }
        }
    }

    /// The input focus moved: `FocusOut` to where it was and its ancestors it's left, `FocusIn` to where it is and
    /// its ancestors it's come into.
    fn set_focus(&mut self, new: u32) {
        let old = self.focus;
        if old == new {
            return;
        }
        self.focus = new;
        let chain = |s: &Server, mut w: u32| -> Vec<u32> {
            let mut out = Vec::new();
            while w > POINTER_ROOT && w != ROOT {
                let Some(win) = s.windows.get(&w) else { break };
                out.push(w);
                w = win.parent;
            }
            out
        };
        let (from, to) = (chain(self, old), chain(self, new));
        for (i, &w) in from.iter().enumerate() {
            if !to.contains(&w) {
                let e = Ev::new(ev::FOCUS_OUT).u8(1, if i == 0 { 3 } else { 4 }).u32(4, w).u8(8, 0);
                self.deliver(w, mask::FOCUS_CHANGE, e);
            }
        }
        for (i, &w) in to.iter().enumerate().rev() {
            if !from.contains(&w) {
                let e = Ev::new(ev::FOCUS_IN).u8(1, if i == 0 { 3 } else { 4 }).u32(4, w).u8(8, 0);
                self.deliver(w, mask::FOCUS_CHANGE, e);
            }
        }
    }

    /// X's state bits: the modifiers and buttons held.
    fn state(&self) -> u16 {
        self.mods | button_state(self.buttons)
    }

    /// A device event from a window up its ancestors to the first where a client (or `only` that one) selected `want`:
    /// to every such client there (only the first, for a press). Who had it, and on which window.
    #[allow(clippy::too_many_arguments)]
    fn device(&mut self, code: u8, detail: u8, source: u32, want: u32, state: u16, only: Option<u32>) -> Option<(u32, u32)> {
        let mut w = source;
        let mut child = 0;
        loop {
            let win = self.windows.get(&w)?;
            let targets: Vec<u32> = win.masks.iter().filter(|(c, m)| m & want != 0 && only.is_none_or(|o| o == *c)).map(|(c, _)| *c).collect();
            if !targets.is_empty() {
                let e = self.device_ev(code, detail, w, child, state);
                let n = if code == ev::BUTTON_PRESS { 1 } else { targets.len() };
                for &c in &targets[..n] {
                    self.event(c, e);
                }
                return Some((targets[0], w));
            }
            if win.dont_propagate & want != 0 || w == ROOT {
                return None;
            }
            child = w;
            w = win.parent;
        }
    }

    fn device_ev(&self, code: u8, detail: u8, w: u32, child: u32, state: u16) -> Ev {
        let (rx, ry) = self.root_pointer;
        let (ox, oy) = self.root_origin(w);
        Ev::new(code).u8(1, detail).u32(4, self.time()).u32(8, ROOT).u32(12, w).u32(16, child).i16(20, rx).i16(22, ry).i16(24, rx - ox).i16(26, ry - oy).u16(28, state).u8(30, 1)
    }

    /// A pointer event, through the grab if there is one.
    fn pointer_event(&mut self, code: u8, detail: u8, target: u32, want: u32, state: u16) -> Option<(u32, u32)> {
        match self.grab.or(self.implicit) {
            Some(g) => {
                if g.owner_events
                    && let Some(r) = self.device(code, detail, target, want, state, Some(g.client))
                {
                    return Some(r);
                }
                if g.mask & want != 0 {
                    let e = self.device_ev(code, detail, g.window, 0, state);
                    self.event(g.client, e);
                    return Some((g.client, g.window));
                }
                None
            }
            None => self.device(code, detail, target, want, state, None),
        }
    }

    /// The pointer crossed from one window to another: `LeaveNotify` and `EnterNotify`, with their details.
    fn cross(&mut self, from: u32, to: u32) {
        if from == to {
            return;
        }
        let (leave, enter) = if self.is_inferior(to, from) { (2, 0) } else if self.is_inferior(from, to) { (0, 2) } else { (3, 3) };
        let state = self.state();
        if from != 0 && self.windows.contains_key(&from) {
            let e = self.device_ev(ev::LEAVE_NOTIFY, leave, from, 0, state).u8(30, 0).u8(31, 2);
            self.deliver(from, mask::LEAVE_WINDOW, e);
        }
        if to != 0 && self.windows.contains_key(&to) {
            let e = self.device_ev(ev::ENTER_NOTIFY, enter, to, 0, state).u8(30, 0).u8(31, 2 | u8::from(self.focus == to));
            self.deliver(to, mask::ENTER_WINDOW, e);
        }
        self.under = to;
        self.update_cursor();
    }

    /// The desktop's cursor for the window the pointer's in: its own, or its nearest ancestor's.
    fn update_cursor(&mut self) {
        let Some(top) = self.top_of(self.under) else { return };
        let Some(window) = self.windows.get(&top).and_then(|t| t.top.as_ref()).map(|t| t.window) else { return };
        let mut w = self.under;
        let cursor = loop {
            let Some(win) = self.windows.get(&w) else { break Cursor::default() };
            if win.cursor != 0 {
                break match self.res.get(&win.cursor) {
                    Some((_, Res::Cursor(c))) => c.clone(),
                    _ => Cursor::default(),
                };
            }
            if w == top {
                break Cursor::default();
            }
            w = win.parent;
        };
        desktop::lock(&self.desktop).set_cursor(window, cursor);
    }

    fn create_cursor(&mut self, ci: u32, r: Req) {
        let (cid, source, mask_id) = (r.u32(4), r.u32(8), r.u32(12));
        let rgb = |at: usize| ((u32::from(r.u16(at) >> 8)) << 16) | ((u32::from(r.u16(at + 2) >> 8)) << 8) | u32::from(r.u16(at + 4) >> 8);
        let (fg, bg, hotspot) = (rgb(16), rgb(22), (r.u16(28) as i32, r.u16(30) as i32));
        if !self.new_id(ci, cid) {
            return;
        }
        let pixmap = |id: u32| match self.res.get(&id) {
            Some((_, Res::Pixmap(p))) => Some(lock(&p.surface).clone()),
            _ => None,
        };
        let Some(src) = pixmap(source) else { return self.error(ci, err::PIXMAP, source) };
        let m = pixmap(mask_id);
        let mut pixels = Surface::new(src.width, src.height, 0);
        for (i, p) in src.pixels.iter().enumerate() {
            let shown = m.as_ref().is_none_or(|m| m.pixels.get(i).is_some_and(|v| v & 1 != 0));
            pixels.pixels[i] = if shown { 0xff00_0000 | if p & 1 != 0 { fg } else { bg } } else { 0 };
        }
        self.res.insert(cid, (ci, Res::Cursor(Cursor::Image { pixels, hotspot })));
    }

    // ---- Drawing ----

    /// Where a drawable's pixels are: a viewable window's top-level backing (its corner there, and its region), or a
    /// pixmap. None for a window that isn't viewable (drawing on it goes nowhere); the error for something else.
    fn target(&self, d: u32, inferiors: bool) -> Result<Option<Target>, u8> {
        if let Some(win) = self.windows.get(&d) {
            if win.input_only {
                return Err(err::MATCH);
            }
            let Some((top, ox, oy, clip)) = self.window_clip(d, inferiors) else { return Ok(None) };
            let Some(t) = self.windows.get(&top).and_then(|t| t.top.as_ref()) else { return Ok(None) };
            return Ok(Some(Target { surface: TargetSurface::Window(t.backing.clone()), top: Some(top), ox, oy, clip, depth: win.depth }));
        }
        match self.res.get(&d) {
            Some((_, Res::Pixmap(p))) => {
                let (w, h) = { let s = lock(&p.surface); (s.width, s.height) };
                Ok(Some(Target { surface: TargetSurface::Pixmap(p.surface.clone()), top: None, ox: 0, oy: 0, clip: vec![Rect::new(0, 0, w, h)], depth: p.depth }))
            }
            _ => Err(err::DRAWABLE),
        }
    }

    /// A GC's paint for a target, and the target's region with the GC's clip.
    fn paint_for(&self, gc: &Gc, t: &Target) -> (Paint, Region) {
        let pixmap = |id: Option<u32>| match id.and_then(|id| self.res.get(&id)) {
            Some((_, Res::Pixmap(p))) => Some(lock(&p.surface).clone()),
            _ => None,
        };
        let (tx, ty) = (gc.ts_origin.0 + t.ox, gc.ts_origin.1 + t.oy);
        let fill = match gc.fill_style {
            1 => pixmap(gc.tile).map_or(Fill::Solid, |s| Fill::Tiled(s, tx, ty)),
            2 | 3 => pixmap(gc.stipple).map_or(Fill::Solid, |s| Fill::Stippled(s, tx, ty, gc.fill_style == 3)),
            _ => Fill::Solid,
        };
        let (cx, cy) = (gc.clip_origin.0 + t.ox, gc.clip_origin.1 + t.oy);
        let mut clip = t.clip.clone();
        let mut mask = None;
        match &gc.clip {
            Clip::Rects(rects) => clip = draw::intersect(&clip, &rects.iter().map(|r| r.offset(cx, cy)).collect::<Vec<_>>()),
            Clip::Mask(id) => mask = pixmap(Some(*id)).map(|s| (s, cx, cy)),
            Clip::None => {}
        }
        (Paint { function: gc.function, fg: gc.fg, bg: gc.bg, depth: if t.depth == 0 { 24 } else { t.depth }, fill, mask }, clip)
    }

    /// Spans (in the drawable's coordinates) painted with a GC.
    fn draw_spans(&mut self, ci: u32, d: u32, gcid: u32, spans: Vec<Span>) {
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        let t = match self.target(d, gc.subwindow_mode == 1) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => return self.error(ci, e, d),
        };
        let (paint, clip) = self.paint_for(&gc, &t);
        let spans: Vec<Span> = draw::merge(spans).into_iter().map(|(y, a, b)| (y + t.oy, a + t.ox, b + t.ox)).collect();
        t.with(|s| draw::fill_spans(s, &spans, &paint, &clip));
        if let Some(top) = t.top {
            let bbox = spans.iter().fold(Rect::default(), |a, &(y, x0, x1)| a.union(&Rect::new(x0, y, x1, y + 1)));
            let clipped = clip.iter().fold(Rect::default(), |a, r| a.union(&r.intersect(&bbox)));
            self.mark(top, clipped);
        }
    }

    fn poly_line(&mut self, ci: u32, d: u32, gcid: u32, pts: &[(i32, i32)], closed: bool) {
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        let mut spans = Vec::new();
        if gc.line_width <= 1 {
            for (i, pair) in pts.windows(2).enumerate() {
                let last = i + 2 == pts.len() && !closed && gc.cap_style != 0;
                draw::thin_line(pair[0], pair[1], last || closed, &mut spans);
            }
            if pts.len() == 1 {
                spans.push((pts[0].1, pts[0].0, pts[0].0 + 1));
            }
        } else {
            let wdt = f64::from(gc.line_width);
            let f = |p: (i32, i32)| (p.0 as f64 + 0.5, p.1 as f64 + 0.5);
            for (i, pair) in pts.windows(2).enumerate() {
                let (first, last) = (i == 0, i + 2 == pts.len());
                // Caps only at the ends; round joins between.
                let cap = if (first || last) && !closed { gc.cap_style } else { 0 };
                draw::wide_line(f(pair[0]), f(pair[1]), wdt, cap, &mut spans);
                if !last || closed {
                    draw::disc(f(pair[1]), wdt / 2.0, &mut spans);
                }
            }
        }
        self.draw_spans(ci, d, gcid, spans);
    }

    fn arcs(&mut self, ci: u32, r: Req) {
        let (d, gcid, fill) = (r.u32(4), r.u32(8), r.major() == 71);
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        let mut spans = Vec::new();
        for a in r.rest(12).as_chunks::<12>().0 {
            let a = Req { data: a };
            let (x, y, w, h, start, extent) = (a.i16(0) as i32, a.i16(2) as i32, a.u16(4) as i32, a.u16(6) as i32, a.i16(8) as i32, a.i16(10) as i32);
            let pts = draw::arc_points(x, y, w, h, start, extent);
            if fill {
                let mut poly = pts.clone();
                if gc.arc_mode == 1 && extent.abs() < 360 * 64 {
                    poly.push((x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0));
                }
                draw::polygon_spans(&poly, false, &mut spans);
            } else if gc.line_width <= 1 {
                let ip: Vec<(i32, i32)> = pts.iter().map(|p| (p.0.round() as i32, p.1.round() as i32)).collect();
                for pair in ip.windows(2) {
                    draw::thin_line(pair[0], pair[1], true, &mut spans);
                }
            } else {
                for pair in pts.windows(2) {
                    draw::wide_line(pair[0], pair[1], f64::from(gc.line_width), 2, &mut spans);
                }
            }
        }
        self.draw_spans(ci, d, gcid, spans);
    }

    fn poly_text(&mut self, ci: u32, r: Req, wide: bool) {
        let (d, gcid, mut x, y) = (r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
        let mut spans = Vec::new();
        let mut at = 16;
        while at + 2 <= r.data.len() {
            let n = r.u8(at) as usize;
            if n == 255 {
                at += 5;
                continue;
            }
            x += r.u8(at + 1) as i8 as i32;
            let chars: Vec<u16> = if wide { r.bytes(at + 2, 2 * n).as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect() } else { r.bytes(at + 2, n).iter().map(|&b| u16::from(b)).collect() };
            draw::core_font::spans(x, y, &chars, &mut spans);
            x += chars.len() as i32 * draw::core_font::WIDTH;
            at += 2 + if wide { 2 * n } else { n };
        }
        self.draw_spans(ci, d, gcid, spans);
    }

    fn image_text(&mut self, ci: u32, r: Req, wide: bool) {
        let (n, d, gcid, x, y) = (r.b1() as usize, r.u32(4), r.u32(8), r.i16(12) as i32, r.i16(14) as i32);
        let chars: Vec<u16> = if wide { r.bytes(16, 2 * n).as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect() } else { r.bytes(16, n).iter().map(|&b| u16::from(b)).collect() };
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        // The background first, as a plain fill in the background colour, then the glyphs in the foreground.
        let mut back = gc.clone();
        back.fg = gc.bg;
        back.function = draw::GX_COPY;
        back.fill_style = 0;
        let mut text = gc.clone();
        text.function = draw::GX_COPY;
        text.fill_style = 0;
        let t = match self.target(d, gc.subwindow_mode == 1) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => return self.error(ci, e, d),
        };
        let mut bg_spans = Vec::new();
        draw::rect_spans(x, y - draw::core_font::ASCENT, chars.len() as i32 * draw::core_font::WIDTH, draw::core_font::ASCENT + draw::core_font::DESCENT, &mut bg_spans);
        let mut fg_spans = Vec::new();
        draw::core_font::spans(x, y, &chars, &mut fg_spans);
        let shift = |s: Vec<Span>| -> Vec<Span> { s.into_iter().map(|(sy, a, b)| (sy + t.oy, a + t.ox, b + t.ox)).collect() };
        let (bg_spans, fg_spans) = (shift(bg_spans), shift(draw::merge(fg_spans)));
        let (back_paint, clip) = self.paint_for(&back, &t);
        let (text_paint, _) = self.paint_for(&text, &t);
        t.with(|s| {
            draw::fill_spans(s, &bg_spans, &back_paint, &clip);
            draw::fill_spans(s, &fg_spans, &text_paint, &clip);
        });
        if let Some(top) = t.top {
            let bbox = Rect::new(x + t.ox, y + t.oy - draw::core_font::ASCENT, x + t.ox + chars.len() as i32 * draw::core_font::WIDTH, y + t.oy + draw::core_font::DESCENT);
            self.mark(top, bbox);
        }
    }

    /// Pixels from a drawable: a rectangle in its coordinates.
    fn read_drawable(&self, d: u32, x: i32, y: i32, w: i32, h: i32) -> Result<(Vec<u32>, u8, u32), u8> {
        if let Some(win) = self.windows.get(&d) {
            let (depth, visual) = (win.depth, win.visual);
            let Some((top, ox, oy, _)) = self.window_clip(d, true) else { return Ok((vec![0; (w.max(0) * h.max(0)) as usize], depth, visual)) };
            let Some(t) = self.windows.get(&top).and_then(|t| t.top.as_ref()) else { return Err(err::MATCH) };
            let px = draw::read(&lock(&t.backing.surface), ox + x, oy + y, w, h);
            return Ok((px, depth, visual));
        }
        match self.res.get(&d) {
            Some((_, Res::Pixmap(p))) => Ok((draw::read(&lock(&p.surface), x, y, w, h), p.depth, 0)),
            _ => Err(err::DRAWABLE),
        }
    }

    fn copy_area(&mut self, ci: u32, r: Req) {
        let (src, dst, gcid) = (r.u32(4), r.u32(8), r.u32(12));
        let (sx, sy, dx, dy, w, h) = (r.i16(16) as i32, r.i16(18) as i32, r.i16(20) as i32, r.i16(22) as i32, r.u16(24) as i32, r.u16(26) as i32);
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        let (mut px, sdepth, _) = match self.read_drawable(src, sx, sy, w, h) {
            Ok(v) => v,
            Err(e) => return self.error(ci, e, src),
        };
        let t = match self.target(dst, gc.subwindow_mode == 1) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => return self.error(ci, e, dst),
        };
        if r.major() == 63 {
            // CopyPlane: one bit plane as the foreground where it's set, the background where it's not.
            let plane = r.u32(28);
            for p in px.iter_mut() {
                *p = if *p & plane != 0 { gc.fg } else { gc.bg };
            }
        } else if sdepth != t.depth && !(sdepth >= 24 && t.depth >= 24) {
            return self.error(ci, err::MATCH, dst);
        }
        let (paint, clip) = self.paint_for(&gc, &t);
        let (x, y) = (dx + t.ox, dy + t.oy);
        let transparent: Option<Vec<bool>> = paint.mask.as_ref().map(|(m, mx, my)| {
            (0..h).flat_map(|row| (0..w).map(move |col| (col, row))).map(|(col, row)| m.get(x + col - mx, y + row - my).unwrap_or(0) & 1 == 0).collect()
        });
        let depth = paint.depth;
        t.with(|s| draw::blit(s, x, y, w, h, &px, gc.function, depth, &clip, transparent.as_deref()));
        if let Some(top) = t.top {
            self.mark(top, Rect::new(x, y, x + w, y + h));
        }
        if gc.exposures {
            let e = Ev::new(ev::NO_EXPOSURE).u32(4, dst).u16(8, 0).u8(10, r.major());
            self.event(ci, e);
        }
    }

    fn put_image(&mut self, ci: u32, r: Req) {
        let (format, d, gcid, w, h, x, y, left_pad, depth) = (r.b1(), r.u32(4), r.u32(8), r.u16(12) as i32, r.u16(14) as i32, r.i16(16) as i32, r.i16(18) as i32, r.u8(20), r.u8(21));
        let Some(gc) = self.gc(gcid) else { return self.error(ci, err::GCONTEXT, gcid) };
        let Some(px) = draw::decode_image(format, depth, w, h, left_pad, r.rest(24), gc.fg, gc.bg) else { return self.error(ci, err::LENGTH, 0) };
        self.blit_image(ci, d, &gc, x, y, w, h, &px);
    }

    /// Pixels drawn with a GC's function and clip at a place in a drawable.
    #[allow(clippy::too_many_arguments)]
    fn blit_image(&mut self, ci: u32, d: u32, gc: &Gc, x: i32, y: i32, w: i32, h: i32, px: &[u32]) {
        let t = match self.target(d, gc.subwindow_mode == 1) {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => return self.error(ci, e, d),
        };
        let (paint, clip) = self.paint_for(gc, &t);
        let (x, y) = (x + t.ox, y + t.oy);
        let depth = paint.depth;
        t.with(|s| draw::blit(s, x, y, w, h, px, gc.function, depth, &clip, None));
        if let Some(top) = t.top {
            self.mark(top, Rect::new(x, y, x + w, y + h));
        }
    }

    fn get_image(&mut self, ci: u32, r: Req) {
        let (d, x, y, w, h) = (r.u32(4), r.i16(8) as i32, r.i16(10) as i32, r.u16(12) as i32, r.u16(14) as i32);
        let (px, depth, visual) = match self.read_drawable(d, x, y, w, h) {
            Ok(v) => v,
            Err(e) => return self.error(ci, e, d),
        };
        let px: Vec<u32> = if depth == 24 { px.iter().map(|p| p & 0xffffff).collect() } else { px };
        let mut rep = Reply::new(depth);
        rep.u32(visual).skip(20).bytes(&draw::encode_image(depth, w, h, &px));
        self.reply(ci, rep);
    }

    // ---- Fonts and colours ----

    fn query_font(&mut self, ci: u32, f: u32) {
        let ok = matches!(self.res.get(&f), Some((_, Res::Font(_) | Res::Gc(_))));
        if !ok {
            return self.error(ci, err::FONT, f);
        }
        let mut rep = Reply::new(0);
        font_info(&mut rep, true);
        self.reply(ci, rep);
    }

    /// A colour by its name, from the image's own `rgb.txt` (or a few every server knows, without one).
    fn color(&mut self, name: &str) -> Option<(u8, u8, u8)> {
        let key: String = name.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        if let Some(hex) = key.strip_prefix('#') {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return match hex.len() {
                6 => Some(((v >> 16) as u8, (v >> 8) as u8, v as u8)),
                3 => Some((((v >> 8) & 15) as u8 * 17, ((v >> 4) & 15) as u8 * 17, (v & 15) as u8 * 17)),
                _ => None,
            };
        }
        if self.colors.is_none() {
            let mut table = HashMap::new();
            for (n, c) in [("black", (0, 0, 0)), ("white", (255, 255, 255)), ("red", (255, 0, 0)), ("green", (0, 255, 0)), ("blue", (0, 0, 255)), ("gray", (190, 190, 190)), ("grey", (190, 190, 190)), ("yellow", (255, 255, 0))] {
                table.insert(n.to_string(), c);
            }
            for path in ["/etc/X11/rgb.txt", "/usr/share/X11/rgb.txt"] {
                let Some(text) = read_file(&self.k, path) else { continue };
                for line in String::from_utf8_lossy(&text).lines() {
                    let mut parts = line.split_whitespace();
                    let (Some(r), Some(g), Some(b)) = (parts.next(), parts.next(), parts.next()) else { continue };
                    let (Ok(r), Ok(g), Ok(b)) = (r.parse::<u8>(), g.parse::<u8>(), b.parse::<u8>()) else { continue };
                    let n: String = parts.collect::<String>().to_ascii_lowercase();
                    table.insert(n, (r, g, b));
                }
                break;
            }
            self.colors = Some(table);
        }
        self.colors.as_ref()?.get(&key).copied()
    }

    // ---- The desktop's events ----

    fn desktop_events(&mut self) {
        let (events, output) = {
            let mut d = desktop::lock(&self.desktop);
            (d.take_events_where(|c| c.is_some_and(|c| c >= CLIENT_IDS)), d.output().clone())
        };
        if output != self.output {
            self.output = output;
            if let Some(root) = self.windows.get_mut(&ROOT) {
                root.w = self.output.width;
                root.h = self.output.height;
            }
            self.root_size_props();
            self.configure_notify(ROOT, false);
        }
        for event in events {
            match event {
                Event::Configure { window, states, .. } => self.desktop_configure(window, states),
                Event::Activated { window, active } => self.activated(window, active),
                Event::Close { window } => self.close(window),
                Event::Pointer { window, x, y, what, buttons, .. } => self.pointer(window, x, y, what, buttons),
                Event::Key { window, code, down, repeat, modifiers } => self.key(window, code, down, repeat, modifiers),
                Event::SendSelection { selection, mime, transfer, .. } => self.export(selection, &mime, transfer),
                Event::SelectionCancelled { selection, .. } => self.selection_taken(selection),
                _ => {}
            }
        }
    }

    /// The desktop moved or sized a top-level (the user, maximizing, fullscreen): its geometry follows, its backing
    /// with it, and it's told (a real `ConfigureNotify` for a new size, a synthetic one for a move).
    fn desktop_configure(&mut self, window: WindowId, states: u32) {
        let Some(&w) = self.tops.get(&window) else { return };
        let Some(content) = desktop::lock(&self.desktop).content_rect(window) else { return };
        let Some(win) = self.windows.get_mut(&w) else { return };
        let bw = win.bw;
        let (x, y, ww, h) = (content.left - bw, content.top - bw, content.width().max(1), content.height().max(1));
        let sized = (win.w, win.h) != (ww, h);
        let moved = (win.x, win.y) != (x, y);
        (win.x, win.y, win.w, win.h) = (x, y, ww, h);
        if sized {
            self.resize_backing(w);
            self.configure_notify(w, false);
            self.expose_top(w);
        } else if moved {
            self.configure_notify(w, true);
        }
        let mut list = Vec::new();
        for (bit, names) in [(state::MAXIMIZED, &["_NET_WM_STATE_MAXIMIZED_VERT", "_NET_WM_STATE_MAXIMIZED_HORZ"][..]), (state::FULLSCREEN, &["_NET_WM_STATE_FULLSCREEN"][..]), (state::MINIMIZED, &["_NET_WM_STATE_HIDDEN"][..])] {
            if states & bit != 0 {
                for n in names {
                    list.push(self.atom(n));
                }
            }
        }
        let a = self.atom("_NET_WM_STATE");
        if self.prop_words(w, "_NET_WM_STATE") != list {
            self.set_prop(w, a, atom::ATOM, 32, words(&list));
        }
    }

    fn activated(&mut self, window: WindowId, active: bool) {
        let Some(&w) = self.tops.get(&window) else { return };
        let a = self.atom("_NET_ACTIVE_WINDOW");
        if active {
            self.set_prop(ROOT, a, atom::WINDOW, 32, words(&[w]));
            let hints = self.prop_words(w, "WM_HINTS");
            let takes_input = !(hints.len() >= 2 && hints[0] & 1 != 0 && hints[1] == 0);
            if takes_input && !(self.focus == w || self.is_inferior(self.focus, w)) {
                self.revert_to = 2;
                self.set_focus(w);
            }
            if self.has_atom(w, "WM_PROTOCOLS", "WM_TAKE_FOCUS") {
                let (protocols, take) = (self.atom("WM_PROTOCOLS"), self.atom("WM_TAKE_FOCUS"));
                let client = self.windows.get(&w).map_or(0, |win| win.client);
                let e = Ev::new(ev::CLIENT_MESSAGE).u8(1, 32).u32(4, w).u32(8, protocols).u32(12, take).u32(16, self.time());
                self.event(client, e);
            }
        } else {
            if self.focus == w || self.is_inferior(self.focus, w) {
                self.set_focus(0);
            }
            self.set_prop(ROOT, a, atom::WINDOW, 32, words(&[0]));
        }
    }

    /// The close button: `WM_DELETE_WINDOW` if the client takes it, or the client disconnected (as a window manager
    /// kills one that doesn't).
    fn close(&mut self, window: WindowId) {
        let Some(&w) = self.tops.get(&window) else { return };
        let client = self.windows.get(&w).map_or(0, |win| win.client);
        if self.has_atom(w, "WM_PROTOCOLS", "WM_DELETE_WINDOW") {
            let (protocols, delete) = (self.atom("WM_PROTOCOLS"), self.atom("WM_DELETE_WINDOW"));
            let e = Ev::new(ev::CLIENT_MESSAGE).u8(1, 32).u32(4, w).u32(8, protocols).u32(12, delete).u32(16, self.time());
            self.event(client, e);
        } else if client != 0 {
            self.disconnect(client);
        }
    }

    fn pointer(&mut self, window: WindowId, x: i32, y: i32, what: Pointer, buttons: u8) {
        let Some(&top) = self.tops.get(&window) else { return };
        let (ox, oy) = self.root_origin(top);
        self.root_pointer = (ox + x, oy + y);
        let target = self.window_at(top, x, y);
        match what {
            Pointer::Enter | Pointer::Move => {
                if target != self.under {
                    let from = self.under;
                    self.cross(from, target);
                }
                if matches!(what, Pointer::Move) {
                    let mut want = mask::POINTER_MOTION;
                    if buttons != 0 {
                        want |= mask::BUTTON_MOTION;
                        want |= button_state(buttons) as u32 & 0x700;
                    }
                    self.buttons = buttons;
                    let state = self.state();
                    self.pointer_event(ev::MOTION_NOTIFY, 0, target, want, state);
                }
            }
            Pointer::Leave => {
                if self.top_of(self.under) == Some(top) {
                    let from = self.under;
                    self.cross(from, ROOT);
                }
            }
            Pointer::Down(b) | Pointer::Up(b) => {
                let down = matches!(what, Pointer::Down(_));
                let n = button_number(b);
                let before = if down { buttons & !button_bit(b) } else { buttons | button_bit(b) };
                self.buttons = before;
                let state = self.state();
                self.press(target, n, down, state);
                self.buttons = buttons;
                if !down && buttons == 0 {
                    self.implicit = None;
                }
            }
            Pointer::Scroll { x: sx, y: sy } => {
                self.scroll.0 += sx;
                self.scroll.1 += sy;
                let state = self.state();
                while self.scroll.1.abs() >= 120 {
                    let up = self.scroll.1 > 0;
                    self.scroll.1 -= if up { 120 } else { -120 };
                    self.press(target, if up { 4 } else { 5 }, true, state);
                    self.press(target, if up { 4 } else { 5 }, false, state | (0x800 << if up { 0 } else { 1 }));
                    self.implicit = None;
                }
                while self.scroll.0.abs() >= 120 {
                    let right = self.scroll.0 > 0;
                    self.scroll.0 -= if right { 120 } else { -120 };
                    self.press(target, if right { 7 } else { 6 }, true, state);
                    self.press(target, if right { 7 } else { 6 }, false, state);
                    self.implicit = None;
                }
            }
            Pointer::Relative { .. } => {}
        }
    }

    /// A button pressed or let go: a press starts the grab a press makes, which its release ends.
    fn press(&mut self, target: u32, n: u8, down: bool, state: u16) {
        let (code, want) = if down { (ev::BUTTON_PRESS, mask::BUTTON_PRESS) } else { (ev::BUTTON_RELEASE, mask::BUTTON_RELEASE) };
        let had = self.pointer_event(code, n, target, want, state);
        if down
            && self.grab.is_none()
            && self.implicit.is_none()
            && let Some((client, w)) = had
        {
            let m = self.windows.get(&w).map_or(0, |win| win.mask_of(client));
            self.implicit = Some(Grab { client, window: w, mask: m, owner_events: m & mask::OWNER_GRAB_BUTTON != 0 });
        }
    }

    fn key(&mut self, window: WindowId, code: u16, down: bool, repeat: bool, modifiers: u8) {
        let Some(&top) = self.tops.get(&window) else { return };
        let keycode = (code + 8).min(255) as u8;
        if code == keys::CAPS_LOCK && down && !repeat {
            self.caps = !self.caps;
        }
        let state = self.state();
        let send = |s: &mut Server, code: u8, want: u32| {
            if let Some((c, g)) = s.keyboard_grab {
                let e = s.device_ev(code, keycode, g, 0, state);
                s.event(c, e);
                return;
            }
            // To the focus, or the window the pointer's in within it; or to the top-level if the focus isn't in it.
            let focus = if s.focus == top || s.is_inferior(s.focus, top) { s.focus } else { top };
            let source = if s.under == focus || s.is_inferior(s.under, focus) { s.under } else { focus };
            s.device(code, keycode, source, want, state, None);
        };
        if down {
            if repeat {
                send(self, ev::KEY_RELEASE, mask::KEY_RELEASE);
            }
            send(self, ev::KEY_PRESS, mask::KEY_PRESS);
            self.keys_down.insert(keycode);
        } else {
            send(self, ev::KEY_RELEASE, mask::KEY_RELEASE);
            self.keys_down.remove(&keycode);
        }
        self.mods = keymap::xkb_mods(modifiers) as u16 | if self.caps { keymap::mods::LOCK as u16 } else { 0 };
    }

    /// The desktop asks for an X owner's selection: asked of the owner, into a property of the server's own window.
    fn export(&mut self, which: Selection, mime: &str, transfer: TransferId) {
        let name = match which {
            Selection::Clipboard => "CLIPBOARD",
            Selection::Primary => "PRIMARY",
            Selection::Drag => return desktop::lock(&self.desktop).provide(transfer, None),
        };
        let selection = self.atom(name);
        let Some(&(owner, client, _)) = self.selections.get(&selection) else { return desktop::lock(&self.desktop).provide(transfer, None) };
        self.next_export += 1;
        let property = self.atom(&format!("_CARBON_SELECTION_{}", self.next_export % 64));
        let target = self.atom(if mime == "STRING" || mime == "text/plain" { "STRING" } else { "UTF8_STRING" });
        self.exports.insert(property, transfer);
        let e = Ev::new(ev::SELECTION_REQUEST).u32(4, self.time()).u32(8, owner).u32(12, WM_CHECK).u32(16, selection).u32(20, target).u32(24, property);
        self.event(client, e);
    }

    /// An owner answered: what it put in the property given to the desktop.
    fn export_done(&mut self, property: u32) {
        // A refusal names no property: the oldest still waiting is the one refused.
        let property = if property == 0 { self.exports.keys().copied().min().unwrap_or(0) } else { property };
        let Some(transfer) = self.exports.remove(&property) else { return };
        let data = self.windows.get_mut(&WM_CHECK).and_then(|w| w.props.remove(&property)).map(|p| p.data);
        desktop::lock(&self.desktop).provide(transfer, data);
    }

    /// Another's offer took a desktop selection an X client owned: that owner's told it's lost it.
    fn selection_taken(&mut self, which: Selection) {
        let name = match which {
            Selection::Clipboard => "CLIPBOARD",
            Selection::Primary => "PRIMARY",
            Selection::Drag => return,
        };
        let selection = self.atom(name);
        let Some(&source) = self.offers.get(&selection) else { return };
        if desktop::lock(&self.desktop).selection_source(which) == Some(source) {
            return;
        }
        self.offers.remove(&selection);
        if let Some((ow, oc, _)) = self.selections.remove(&selection) {
            let e = Ev::new(ev::SELECTION_CLEAR).u32(4, self.time()).u32(8, ow).u32(12, selection);
            self.event(oc, e);
        }
    }

    /// A client gone: its windows, resources, selections and grabs with it.
    fn disconnect(&mut self, ci: u32) {
        let Some(c) = self.clients.remove(&ci) else { return };
        let _ = c.socket.shutdown(net::Shutdown::Both);
        let mine: Vec<u32> = self.windows.iter().filter(|(_, w)| w.client == ci).map(|(&id, _)| id).collect();
        for w in mine {
            // Its windows that are children of others' go first; the rest with their parents.
            if self.windows.get(&w).is_some_and(|win| self.windows.get(&win.parent).is_none_or(|p| p.client != ci)) {
                self.destroy_window(w);
            }
        }
        self.res.retain(|_, (owner, _)| *owner != ci);
        for win in self.windows.values_mut() {
            win.masks.retain(|(c, _)| *c != ci);
        }
        let gone: Vec<u32> = self.selections.iter().filter(|(_, o)| o.1 == ci).map(|(&s, _)| s).collect();
        for s in gone {
            self.selections.remove(&s);
            if let Some(which) = self.desktop_selection(s)
                && let Some(source) = self.offers.remove(&s)
            {
                let mut d = desktop::lock(&self.desktop);
                if d.selection_source(which) == Some(source) {
                    d.clear_selection(which);
                }
            }
        }
        if self.grab.is_some_and(|g| g.client == ci) {
            self.grab = None;
            desktop::lock(&self.desktop).set_capture(None);
        }
        if self.implicit.is_some_and(|g| g.client == ci) {
            self.implicit = None;
        }
        if self.keyboard_grab.is_some_and(|(c, _)| c == ci) {
            self.keyboard_grab = None;
        }
        self.pastes.retain(|p| p.client != ci);
    }
}

impl Win {
    #[allow(clippy::too_many_arguments)]
    fn new(client: u32, parent: u32, x: i32, y: i32, w: i32, h: i32, bw: i32, input_only: bool, depth: u8, visual: u32) -> Win {
        Win {
            client,
            parent,
            children: Vec::new(),
            x,
            y,
            w,
            h,
            bw,
            input_only,
            depth,
            visual,
            mapped: false,
            override_redirect: false,
            bg: Bg::None,
            masks: Vec::new(),
            dont_propagate: 0,
            cursor: 0,
            colormap: COLORMAP,
            props: BTreeMap::new(),
            top: None,
            bounding: None,
            input: None,
        }
    }
}

/// Where drawing goes.
struct Target {
    surface: TargetSurface,
    /// The top-level whose backing it is, for its damage.
    top: Option<u32>,
    ox: i32,
    oy: i32,
    clip: Region,
    depth: u8,
}

enum TargetSurface {
    Window(Arc<Backing>),
    Pixmap(Arc<Mutex<Surface>>),
}

impl Target {
    fn with<R>(&self, f: impl FnOnce(&mut Surface) -> R) -> R {
        match &self.surface {
            TargetSurface::Window(b) => f(&mut lock(&b.surface)),
            TargetSurface::Pixmap(s) => f(&mut lock(s)),
        }
    }
}

impl Fill {
    /// A solid fill in a colour, as `paint_background` takes it.
    fn with(self, color: u32) -> (Fill, u32) {
        (self, color)
    }
}

/// The text types a selection is offered to the desktop as, and taken from it as, best first.
const TEXT_MIMES: [&str; 5] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"];

/// 32-bit values as a property's bytes.
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Points made absolute, for `CoordModePrevious` (each relative to the one before).
fn relative(pts: &mut [(i32, i32)], mode: u8) {
    if mode == 1 {
        for i in 1..pts.len() {
            pts[i] = (pts[i].0 + pts[i - 1].0, pts[i].1 + pts[i - 1].1);
        }
    }
}

fn button_number(b: Button) -> u8 {
    match b {
        Button::Left => 1,
        Button::Middle => 2,
        Button::Right => 3,
    }
}

fn button_bit(b: Button) -> u8 {
    match b {
        Button::Left => 1,
        Button::Right => 2,
        Button::Middle => 4,
    }
}

/// The desktop's buttons held as X's: Button1Mask for the left, Button2Mask the middle, Button3Mask the right.
fn button_state(buttons: u8) -> u16 {
    (if buttons & 1 != 0 { 0x100 } else { 0 }) | (if buttons & 4 != 0 { 0x200 } else { 0 }) | (if buttons & 2 != 0 { 0x400 } else { 0 })
}

/// A top-level's input region for the desktop: its input shape within its bounding shape (None: all of it).
fn top_input(win: &Win) -> Option<Vec<Rect>> {
    match (&win.bounding, &win.input) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(i)) => Some(i.clone()),
        (Some(b), Some(i)) => Some(draw::intersect(b, i)),
    }
}

/// The font names `ListFonts` gives for a pattern: the core font's, and the cursor font's.
fn font_names(pattern: &str, most: usize) -> Vec<String> {
    draw::core_font::NAMES.iter().chain(["cursor"].iter()).filter(|n| draw::matches(pattern, n)).take(most).map(|s| s.to_string()).collect()
}

/// `QueryFont`'s (and `ListFontsWithInfo`'s) font information for the core font: one cell for every character, so
/// no character's own (`with_count` adds the count of them, none, after the properties).
fn font_info(rep: &mut Reply, with_count: bool) {
    use draw::core_font::{ASCENT, DESCENT, FIRST, LAST, WIDTH};
    let charinfo = |rep: &mut Reply| {
        rep.i16(0).i16(WIDTH as i16).i16(WIDTH as i16).i16(ASCENT as i16).i16(DESCENT as i16).u16(0);
    };
    charinfo(rep);
    rep.skip(4);
    charinfo(rep);
    rep.skip(4);
    rep.u16(FIRST).u16(LAST).u16(32).u16(0).u8(0).u8(0).u8(0).u8(1).i16(ASCENT as i16).i16(DESCENT as i16);
    if with_count {
        rep.u32(0);
    }
}

/// A positioner that puts a window of `size` with its top left at `at` on its parent's content, and never moves it:
/// an override-redirect window's place, as a popup.
fn at_point(at: (i32, i32), size: (i32, i32)) -> Positioner {
    Positioner { size, anchor_rect: Rect::new(at.0, at.1, at.0, at.1), anchor: Edge::TopLeft, gravity: Edge::BottomRight, offset: (0, 0), adjust: 0 }
}

/// A file from the machine's disk, whole.
fn read_file(k: &Kernel, path: &str) -> Option<Vec<u8>> {
    let disk = lock(&k.disk);
    let ino = disk.resolve(layers::ROOT, path, true).ok()?;
    let size = disk.stat(ino).ok()?.size as usize;
    let mut buf = vec![0u8; size.min(1 << 22)];
    let n = disk.read(ino, 0, &mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

