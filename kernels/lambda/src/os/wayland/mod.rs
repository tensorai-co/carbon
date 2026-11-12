//! lambda's Wayland compositor: the server end of `$XDG_RUNTIME_DIR/wayland-0`, answered by the kernel itself, on the
//! desktop's contract (`display::Desktop`), as atlas's win32k is. A client's `xdg_toplevel` is a desktop window; its
//! `wl_shm` buffers are read in place from its own shared memory (a memfd's frames), never copied; what the desktop
//! decides (a configure, activation, a close, a frame, a buffer released) goes back as Wayland's events.
//!
//! Here so far: the core (`wl_display`, `wl_registry`, `wl_callback`, `wl_compositor`, `wl_surface`, `wl_region`),
//! `wl_shm` with its pools and buffers, xdg-shell's toplevels, `wl_seat` with its pointer (cursor surfaces too) and
//! keyboard (the desktop's XKB keymap, in a sealed memfd), `wl_output`, xdg-shell's popups on the desktop's (placed
//! by its positioner, grabbing, dismissed), server-side decorations (the desktop's frames), window geometry (a
//! client's own shadow drawn around its window, not part of it: the desktop's buffer offset), subsurfaces,
//! `wl_data_device`'s clipboard (a source taken told `cancelled` before the new selection's offered),
//! `wp_cursor_shape_v1` (the desktop's named cursors), and `zwp_text_input_v3` (the desktop's text and an input
//! method's preedit, to the window with the keyboard), relative pointers and pointer constraints (the desktop's
//! relative motion, lock and confinement), xdg-activation (the desktop's tokens), and xdg-output. A commit sizes its window only once the client's acknowledged
//! every configure (and never while the desktop places it, maximized or fullscreen). Real clients run on it: GTK 4
//! (`gtk4-demo`, `gnome-text-editor`), foot, and weston's shm and toytoolkit clients. Not yet: touch, primary
//! selection, fractional scale and viewporter, and the protocols made from their XML.

mod protocol;
mod wire;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use display::cursor::Cursor;
use display::desktop::{self, Button, Constraint, Event, Insets, Keep, Pointer, Role, Shared, Spec, WindowId, edge, state};
use display::positioner::{Edge, Positioner};
use display::observe::ObserverId;
use display::output::Output;
use display::selection::{Selection, SourceId, TransferId};
use kernels_core::wait::Waiter;
use display::{Buffer, Format, Pixels, Rect, Surface as Picture, keymap, keys};
use kernels_core::lock;
use layers::New;
use net::{Addr, Domain, Kind, UnixAddr, UnixName};

use self::protocol::{Interface, GLOBALS};
use self::wire::Arg;
use crate::abi::S_IFSOCK;
use crate::fs::{Node, Open, Pipe};
use crate::mm::shared::{F_SEAL_GROW, F_SEAL_SEAL, F_SEAL_SHRINK, F_SEAL_WRITE, SharedMem};
use crate::{Kernel, Meta};

/// Where the server's socket is: `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY`, root's runtime directory.
pub(crate) const RUNTIME_DIR: &str = "/run/user/0";
pub(crate) const DISPLAY: &str = "wayland-0";

/// Starts the server: its socket made on the disk and listening, and its thread serving every connection until the
/// machine ends.
pub(crate) fn start(k: &Arc<Kernel>, desktop: Shared) -> Result<(), String> {
    let path = format!("{RUNTIME_DIR}/{DISPLAY}");
    let ino = {
        let mut disk = lock(&k.disk);
        let _ = disk.put("/run/user", New::Dir, &[], Meta::dir(0o755), Meta::dir(0o755));
        let _ = disk.put(RUNTIME_DIR, New::Dir, &[], Meta::dir(0o700), Meta::dir(0o755));
        disk.put(&path, New::File, &[], Meta { mode: S_IFSOCK | 0o777, ..Meta::default() }, Meta::dir(0o755)).map_err(|e| format!("{path}: {e:?}"))?
    };
    let listener = k.net.socket(Domain::Unix, Kind::Stream).map_err(|e| format!("{path}: {e:?}"))?;
    listener.bind(&Addr::Unix(UnixAddr { name: UnixName::Path(path.clone().into_bytes()), file: Some(ino) })).map_err(|e| format!("{path}: {e:?}"))?;
    listener.listen(16).map_err(|e| format!("{path}: {e:?}"))?;
    let k = k.clone();
    std::thread::Builder::new()
        .name("lambda wayland".into())
        .spawn(move || Server::new(k, desktop, listener).run())
        .map_err(|e| e.to_string())?;
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

/// A client's `wl_shm` buffer: its pixels in its shared memory, read in place.
struct ShmPixels {
    mem: Arc<SharedMem>,
    offset: u64,
    width: i32,
    height: i32,
    stride: i32,
    format: Format,
}

impl Pixels for ShmPixels {
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    fn format(&self) -> Format {
        self.format
    }

    fn read_row(&self, x: i32, y: i32, out: &mut [u32]) {
        let mut bytes = vec![0u8; out.len() * 4];
        let at = self.offset + (y.max(0) as u64) * self.stride as u64 + (x.max(0) as u64) * 4;
        self.mem.read(at, &mut bytes);
        for (o, b) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *o = u32::from_le_bytes(*b);
        }
    }
}

/// A surface's state: what's pending for its next commit, and what it is.
#[derive(Default)]
struct Surface {
    /// The desktop window it's shown in, once it has a role that has one.
    window: Option<WindowId>,
    /// Its `xdg_surface`, and whether that's been sent its first configure.
    xdg: Option<u32>,
    configured: bool,
    pending_buffer: Option<Option<u32>>,
    pending_damage: Vec<Rect>,
    pending_frames: Vec<u32>,
    pending_scale: Option<i32>,
    scale: i32,
    /// Its input region set since the last commit (`Some(None)`: all of it again), and as committed, in its own
    /// coordinates: None for all of it.
    pending_input: Option<Option<Vec<Rect>>>,
    input: Option<Vec<Rect>>,
    /// Frame callbacks committed, waiting for the desktop's next frame.
    waiting: Vec<u32>,
    /// The buffer committed, and its size.
    buffer: Option<u32>,
    size: (i32, i32),
    /// It's a cursor (`wl_pointer.set_cursor`), with its hotspot.
    cursor: Option<(i32, i32)>,
    /// It's a subsurface: its `wl_subsurface`.
    subsurface: Option<u32>,
    /// Its window geometry (`xdg_surface.set_window_geometry`) set since the last commit, and as committed: the part
    /// of it that's its window, in its own coordinates, the rest a shadow it draws.
    pending_geometry: Option<Rect>,
    geometry: Option<Rect>,
    /// The serial of the last configure it was sent: until it's acknowledged, what it commits was drawn before it,
    /// so it doesn't size its window.
    sent: u32,
    /// Where its window's content is in it: its geometry's top left, as last committed (clamped to its buffer).
    offset: (i32, i32),
}

enum Data {
    Plain,
    /// A `wl_region`: the rectangles it's made of (`add`, less `subtract`).
    Region(Vec<Rect>),
    Pool { mem: Arc<SharedMem>, size: i64 },
    Buffer(Buffer),
    Surface(Box<Surface>),
    XdgSurface { surface: u32, role: Option<u32> },
    Toplevel { surface: u32 },
    Positioner(Positioner),
    /// A popup of a parent surface's (None: its parent's given another way, not yet), placed by `positioner`, grabbing
    /// if it asked; and the token a `reposition` it hasn't been told of yet gave.
    Popup { surface: u32, parent: Option<u32>, positioner: Positioner, grab: bool, token: Option<u32> },
    /// A toplevel's decoration, as the client asked.
    Decoration { toplevel: u32 },
    /// A subsurface: its surface, its parent surface, and where it is on that.
    Subsurface { surface: u32, parent: u32, at: (i32, i32) },
    /// A client's `wl_data_source`, and the types it offers.
    Source(Vec<String>),
    /// What the clipboard is, offered to a client (a `wl_data_offer` the server made).
    Offer,
    TextInput(Box<TextInput>),
    /// A `zwp_locked_pointer_v1` or `zwp_confined_pointer_v1` on a surface: confined to a region (its bounds, in the
    /// surface's coordinates; None for all of it), for one taking or every one, whether it's taken now, and whether a
    /// one-shot one's been let go for good.
    Constraint { surface: u32, region: Option<Rect>, oneshot: bool, taken: bool, dead: bool },
    /// An `xdg_activation_token_v1`: whether it's been committed (it may be only once).
    Token { committed: bool },
}

/// A `zwp_text_input_v3`: the surface it was told it's in (`enter`), its state as last committed and as pending
/// until the next commit, and how many commits it's made (`done`'s serial).
#[derive(Default)]
struct TextInput {
    focus: Option<u32>,
    enabled: bool,
    rect: Rect,
    pending_enabled: Option<bool>,
    pending_rect: Option<Rect>,
    commits: u32,
}

struct Object {
    interface: Interface,
    version: u32,
    data: Data,
}

struct Client {
    socket: net::Socket,
    _watch: net::Watch,
    input: Vec<u8>,
    fds: VecDeque<Arc<Open>>,
    out: Vec<u8>,
    out_fds: Vec<Arc<Open>>,
    objects: HashMap<u32, Object>,
    /// A protocol error was sent: it's disconnected once that's written.
    closing: bool,
    gone: bool,
    /// The window of its the pointer's in, the one with the keyboard, and the modifiers it was last told.
    pointer_focus: Option<WindowId>,
    keyboard_focus: Option<WindowId>,
    mods: (u32, u32),
    /// The next ID the server gives an object it makes for it (a `wl_data_offer`).
    next_id: u32,
}

struct Server {
    k: Arc<Kernel>,
    desktop: Shared,
    listener: net::Socket,
    signal: Arc<Signal>,
    _watch: net::Watch,
    clients: BTreeMap<u64, Client>,
    next_client: u64,
    /// Each desktop window's client and surface; each desktop buffer's client and `wl_buffer`.
    windows: HashMap<WindowId, (u64, u32)>,
    buffers: HashMap<display::buffer::BufferId, (u64, u32)>,
    serial: u32,
    placed: u32,
    /// Event times are milliseconds from here.
    start: Instant,
    /// The keymap every keyboard is sent, and its size with its NUL.
    keymap: Option<(Arc<Open>, u32)>,
    /// The output as every `wl_output` was last told it; Caps Lock, locked or not.
    output: Output,
    caps: bool,
    /// The server's own view of the desktop, for the clipboard's changes.
    observer: ObserverId,
    /// The clipboard as a client of ours offered it: the desktop's offer, the client, its `wl_data_source`.
    selection: Option<(SourceId, u64, u32)>,
    /// Pastes on their way: from a client's source, read from a pipe for a desktop transfer; from the desktop, for a
    /// client's pipe; and written into a client's pipe.
    reading: Vec<(Arc<Open>, TransferId, Vec<u8>)>,
    receiving: Vec<(TransferId, Arc<Open>)>,
    writing: Vec<(Arc<Open>, Vec<u8>)>,
    waiter: Arc<Waiter>,
}

impl Server {
    fn new(k: Arc<Kernel>, desktop: Shared, listener: net::Socket) -> Server {
        let signal = Arc::new(Signal::default());
        let s = signal.clone();
        let watch = listener.watch(Arc::new(move || s.wake()));
        let s = signal.clone();
        let (output, observer) = {
            let mut d = desktop::lock(&desktop);
            d.set_waker(Arc::new(move || s.wake()));
            let observer = d.observe();
            let s = signal.clone();
            d.set_observer_waker(observer, Arc::new(move || s.wake()));
            (d.output().clone(), observer)
        };
        let keymap = keymap_file(&k);
        Server {
            k,
            desktop,
            listener,
            signal,
            _watch: watch,
            clients: BTreeMap::new(),
            next_client: 1,
            windows: HashMap::new(),
            buffers: HashMap::new(),
            serial: 0,
            placed: 0,
            start: Instant::now(),
            keymap,
            output,
            caps: false,
            observer,
            selection: None,
            reading: Vec::new(),
            receiving: Vec::new(),
            writing: Vec::new(),
            waiter: Waiter::new(),
        }
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn run(mut self) {
        while !self.k.done.load(std::sync::atomic::Ordering::Acquire) {
            // Pastes through pipes are looked at often while there are any.
            let busy = !self.reading.is_empty() || !self.receiving.is_empty() || !self.writing.is_empty();
            self.signal.wait(Duration::from_millis(if busy { 10 } else { 200 }));
            self.accept();
            let ids: Vec<u64> = self.clients.keys().copied().collect();
            for id in ids {
                self.read(id);
            }
            self.desktop_events();
            self.pastes();
            self.flush();
        }
    }

    /// Pastes through pipes, as far as they'll go without waiting.
    fn pastes(&mut self) {
        // From a client's source: read until it closes its end, then given to the desktop.
        let mut done = Vec::new();
        self.reading.retain_mut(|(file, transfer, data)| loop {
            let Node::Pipe(pipe, false) = &file.node else { return false };
            match pipe.read(&self.waiter, 1 << 16, true) {
                Ok(bytes) if bytes.is_empty() => {
                    done.push((*transfer, std::mem::take(data)));
                    return false;
                }
                Ok(bytes) => data.extend(bytes),
                Err(_) => return true,
            }
        });
        if !done.is_empty() {
            let mut d = desktop::lock(&self.desktop);
            for (transfer, data) in done {
                d.provide(transfer, Some(data));
            }
        }
        // From the desktop, for a client's pipe: once it's come.
        let mut came = Vec::new();
        {
            let mut d = desktop::lock(&self.desktop);
            self.receiving.retain(|(transfer, file)| match d.take_transfer(*transfer) {
                Some(data) => {
                    came.push((file.clone(), data.unwrap_or_default()));
                    false
                }
                None => true,
            });
        }
        self.writing.extend(came);
        // Into a client's pipe, then its end closed (dropped), so it reads the end.
        self.writing.retain_mut(|(file, data)| {
            let Node::Pipe(pipe, true) = &file.node else { return false };
            while !data.is_empty() {
                match pipe.write(&self.waiter, data, true) {
                    Ok(n) => {
                        data.drain(..n);
                    }
                    Err(e) if e == crate::abi::EAGAIN => return true,
                    Err(_) => return false,
                }
            }
            false
        });
    }

    /// The clipboard as it is now, offered to a client through each of its `wl_data_device`s (or none).
    fn send_selection(&mut self, client: u64) {
        let offered = desktop::lock(&self.desktop).selection(Selection::Clipboard).map(|(_, mimes)| mimes);
        let Some(c) = self.clients.get_mut(&client) else { return };
        for (device, version) in objects_of(c, Interface::DataDevice) {
            let Some(mimes) = &offered else {
                send(c, device, 5, vec![Arg::Object(0)]);
                continue;
            };
            let offer = c.next_id;
            c.next_id += 1;
            c.objects.insert(offer, Object { interface: Interface::DataOffer, version, data: Data::Offer });
            send(c, device, 0, vec![Arg::NewId(offer)]);
            for mime in mimes {
                send(c, offer, 0, vec![Arg::Str(Some(mime.clone()))]);
            }
            send(c, device, 5, vec![Arg::Object(offer)]);
        }
    }

    fn accept(&mut self) {
        while let Ok((socket, _)) = self.listener.accept() {
            let s = self.signal.clone();
            let watch = socket.watch(Arc::new(move || s.wake()));
            let id = self.next_client;
            self.next_client += 1;
            let mut objects = HashMap::new();
            objects.insert(1, Object { interface: Interface::Display, version: 1, data: Data::Plain });
            self.clients.insert(
                id,
                Client {
                    socket,
                    _watch: watch,
                    input: Vec::new(),
                    fds: VecDeque::new(),
                    out: Vec::new(),
                    out_fds: Vec::new(),
                    objects,
                    closing: false,
                    gone: false,
                    pointer_focus: None,
                    keyboard_focus: None,
                    mods: (0, 0),
                    next_id: 0xff00_0000,
                },
            );
        }
    }

    /// Everything a client's sent, read and done.
    fn read(&mut self, id: u64) {
        let Some(mut c) = self.clients.remove(&id) else { return };
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
        while !c.closing
            && let Some((object, opcode, size)) = wire::header(&c.input)
        {
            if !(8..=wire::MOST).contains(&size) {
                self.error(&mut c, 1, protocol::INVALID_METHOD, "a message of an impossible size");
                break;
            }
            if c.input.len() < size {
                break;
            }
            let body: Vec<u8> = c.input[8..size].to_vec();
            c.input.drain(..size);
            self.dispatch(id, &mut c, object, opcode, &body);
        }
        if c.gone {
            self.disconnect(id, c);
        } else {
            self.clients.insert(id, c);
        }
    }

    fn error(&mut self, c: &mut Client, object: u32, code: u32, message: &str) {
        send(c, 1, 0, vec![Arg::Object(object), Arg::Uint(code), Arg::Str(Some(message.to_string()))]);
        c.closing = true;
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    fn dispatch(&mut self, client: u64, c: &mut Client, id: u32, opcode: u16, body: &[u8]) {
        let Some((interface, version)) = c.objects.get(&id).map(|o| (o.interface, o.version)) else {
            return self.error(c, 1, protocol::INVALID_OBJECT, &format!("there's no object {id}"));
        };
        let Some(signature) = interface.requests().get(opcode as usize) else {
            return self.error(c, id, protocol::INVALID_METHOD, &format!("{} has no request {opcode}", interface.name()));
        };
        let Some(args) = wire::decode(signature, body, &mut c.fds) else {
            return self.error(c, id, protocol::INVALID_METHOD, &format!("{} request {opcode}'s arguments", interface.name()));
        };
        let new = |interface, data| Object { interface, version, data };
        match (interface, opcode) {
            // wl_display.sync: done at once, everything before it having been done.
            (Interface::Display, 0) => {
                let cb = args[0].uint();
                send(c, cb, 0, vec![Arg::Uint(0)]);
                delete_id(c, cb);
            }
            (Interface::Display, 1) => {
                let registry = args[0].uint();
                c.objects.insert(registry, Object { interface: Interface::Registry, version: 1, data: Data::Plain });
                for (i, (global, v)) in GLOBALS.iter().enumerate() {
                    send(c, registry, 0, vec![Arg::Uint(i as u32 + 1), Arg::Str(Some(global.name().into())), Arg::Uint(*v)]);
                }
            }
            (Interface::Registry, 0) => {
                let (name, iface, v, new_id) = (args[0].uint(), args[1].str().to_string(), args[2].uint(), args[3].uint());
                let Some((global, most)) = GLOBALS.get((name as usize).wrapping_sub(1)).filter(|(g, _)| g.name() == iface) else {
                    return self.error(c, id, protocol::INVALID_OBJECT, &format!("there's no global {name} that's a {iface}"));
                };
                c.objects.insert(new_id, Object { interface: *global, version: v.clamp(1, *most), data: Data::Plain });
                let version = v.clamp(1, *most);
                match global {
                    Interface::Shm => {
                        for format in [protocol::SHM_ARGB8888, protocol::SHM_XRGB8888] {
                            send(c, new_id, 0, vec![Arg::Uint(format)]);
                        }
                    }
                    Interface::Seat => {
                        send(c, new_id, 0, vec![Arg::Uint(protocol::POINTER | protocol::KEYBOARD)]);
                        if version >= 2 {
                            send(c, new_id, 1, vec![Arg::Str(Some("seat0".into()))]);
                        }
                    }
                    Interface::Output => send_output(c, new_id, version, &self.output),
                    _ => {}
                }
            }
            (Interface::Compositor, 0) => {
                c.objects.insert(args[0].uint(), new(Interface::Surface, Data::Surface(Box::new(Surface { scale: 1, ..Surface::default() }))));
            }
            (Interface::Compositor, 1) => {
                c.objects.insert(args[0].uint(), new(Interface::Region, Data::Region(Vec::new())));
            }
            (Interface::Region, op @ (1 | 2)) => {
                let r = Rect::new(args[0].int(), args[1].int(), args[0].int().saturating_add(args[2].int()), args[1].int().saturating_add(args[3].int()));
                if let Some(Object { data: Data::Region(rects), .. }) = c.objects.get_mut(&id) {
                    if op == 1 {
                        rects.push(r);
                    } else {
                        *rects = subtract_rect(std::mem::take(rects), &r);
                    }
                }
            }
            // The opaque region is a hint the desktop doesn't need; the input region is where the pointer's its.
            (Interface::Surface, 4) => {}
            (Interface::Surface, 5) => {
                let region = match c.objects.get(&args[0].uint()).map(|o| &o.data) {
                    Some(Data::Region(rects)) => Some(rects.clone()),
                    _ => None,
                };
                self.surface(c, id, |s| s.pending_input = Some(region));
            }
            (Interface::Shm, 0) => {
                let (pool, size) = (args[0].uint(), i64::from(args[2].int()));
                let Arg::Fd(file) = &args[1] else { return };
                let Node::Memfd(mem, _) = &file.node else {
                    return self.error(c, id, protocol::IMPLEMENTATION, "only a memfd can be a wl_shm pool yet");
                };
                c.objects.insert(pool, new(Interface::ShmPool, Data::Pool { mem: mem.clone(), size }));
            }
            (Interface::ShmPool, 0) => {
                let Some(Data::Pool { mem, size }) = c.objects.get(&id).map(|o| &o.data) else { return };
                let (buffer, offset, width, height, stride, format) = (args[0].uint(), args[1].int(), args[2].int(), args[3].int(), args[4].int(), args[5].uint());
                let format = match format {
                    protocol::SHM_ARGB8888 => Format::Argb8888,
                    protocol::SHM_XRGB8888 => Format::Xrgb8888,
                    _ => return self.error(c, id, 0, &format!("format {format:#x} isn't one wl_shm offered")),
                };
                let end = i64::from(offset) + i64::from(stride) * i64::from(height);
                if offset < 0 || width <= 0 || height <= 0 || stride < width * 4 || end > *size {
                    return self.error(c, id, 2, "a buffer outside its pool, or of an impossible size");
                }
                let pixels = ShmPixels { mem: mem.clone(), offset: offset as u64, width, height, stride, format };
                let b = Buffer::new(Arc::new(pixels));
                self.buffers.insert(b.id, (client, buffer));
                c.objects.insert(buffer, new(Interface::Buffer, Data::Buffer(b)));
            }
            (Interface::ShmPool, 2) => {
                if let Some(Object { data: Data::Pool { size, .. }, .. }) = c.objects.get_mut(&id) {
                    *size = (*size).max(i64::from(args[0].int()));
                }
            }
            (Interface::Surface, 1) => self.surface(c, id, |s| s.pending_buffer = Some((args[0].uint() != 0).then(|| args[0].uint()))),
            (Interface::Surface, 2) => {
                // In the surface's coordinates: in the buffer's, at its scale.
                let s = c.objects.get(&id).and_then(|o| if let Data::Surface(s) = &o.data { Some(s.scale) } else { None }).unwrap_or(1);
                let r = Rect::new(args[0].int(), args[1].int(), args[0].int().saturating_add(args[2].int()), args[1].int().saturating_add(args[3].int()));
                let r = Rect::new(r.left.saturating_mul(s), r.top.saturating_mul(s), r.right.saturating_mul(s), r.bottom.saturating_mul(s));
                self.surface(c, id, |surface| surface.pending_damage.push(r));
            }
            (Interface::Surface, 3) => {
                let cb = args[0].uint();
                c.objects.insert(cb, new(Interface::Callback, Data::Plain));
                self.surface(c, id, |s| s.pending_frames.push(cb));
            }
            (Interface::Surface, 6) => self.commit(client, c, id),
            (Interface::Surface, 8) => {
                let scale = args[0].int();
                if scale < 1 {
                    return self.error(c, id, 0, "a buffer scale below 1");
                }
                self.surface(c, id, |s| s.pending_scale = Some(scale));
            }
            (Interface::Surface, 9) => {
                let r = Rect::new(args[0].int(), args[1].int(), args[0].int().saturating_add(args[2].int()), args[1].int().saturating_add(args[3].int()));
                self.surface(c, id, |s| s.pending_damage.push(r));
            }
            (Interface::XdgWmBase, 2) => {
                let (xdg, surface) = (args[0].uint(), args[1].uint());
                if !matches!(c.objects.get(&surface), Some(Object { interface: Interface::Surface, .. })) {
                    return self.error(c, id, protocol::INVALID_OBJECT, "get_xdg_surface of something that isn't a wl_surface");
                }
                c.objects.insert(xdg, new(Interface::XdgSurface, Data::XdgSurface { surface, role: None }));
                self.surface(c, surface, |s| s.xdg = Some(xdg));
            }
            (Interface::XdgWmBase, 1) => {
                c.objects.insert(args[0].uint(), new(Interface::XdgPositioner, Data::Positioner(Positioner::default())));
            }
            (Interface::XdgPositioner, op @ 1..=6) => {
                let Some(Object { data: Data::Positioner(p), .. }) = c.objects.get_mut(&id) else { return };
                let edge = |v: u32| match v {
                    1 => Edge::Top,
                    2 => Edge::Bottom,
                    3 => Edge::Left,
                    4 => Edge::Right,
                    5 => Edge::TopLeft,
                    6 => Edge::BottomLeft,
                    7 => Edge::TopRight,
                    8 => Edge::BottomRight,
                    _ => Edge::Centre,
                };
                match op {
                    1 => p.size = (args[0].int(), args[1].int()),
                    2 => p.anchor_rect = Rect::new(args[0].int(), args[1].int(), args[0].int().saturating_add(args[2].int()), args[1].int().saturating_add(args[3].int())),
                    3 => p.anchor = edge(args[0].uint()),
                    4 => p.gravity = edge(args[0].uint()),
                    // The same bits as the desktop's `adjust`.
                    5 => p.adjust = args[0].uint(),
                    _ => p.offset = (args[0].int(), args[1].int()),
                }
            }
            (Interface::XdgSurface, 1) => {
                let toplevel = args[0].uint();
                let Some(Data::XdgSurface { surface, .. }) = c.objects.get(&id).map(|o| &o.data) else { return };
                let surface = *surface;
                // A desktop window, hidden until its first buffer's committed.
                self.placed += 1;
                let at = 60 + 32 * (self.placed % 10) as i32;
                let window = desktop::lock(&self.desktop).create(Spec { rect: Rect::new(at, at, at + 1, at + 1), client, fill: 0xffffff, ..Spec::default() });
                self.windows.insert(window, (client, surface));
                if let Some(Object { data: Data::XdgSurface { role, .. }, .. }) = c.objects.get_mut(&id) {
                    *role = Some(toplevel);
                }
                c.objects.insert(toplevel, new(Interface::XdgToplevel, Data::Toplevel { surface }));
                self.surface(c, surface, |s| s.window = Some(window));
            }
            // A popup: made on the desktop at its first commit, with the grab it's asked for by then.
            (Interface::XdgSurface, 2) => {
                let (popup, parent, positioner) = (args[0].uint(), args[1].uint(), args[2].uint());
                let Some(Data::XdgSurface { surface, .. }) = c.objects.get(&id).map(|o| &o.data) else { return };
                let surface = *surface;
                let parent = match c.objects.get(&parent).map(|o| &o.data) {
                    Some(Data::XdgSurface { surface, .. }) => Some(*surface),
                    _ => None,
                };
                let Some(Data::Positioner(positioner)) = c.objects.get(&positioner).map(|o| &o.data) else {
                    return self.error(c, id, protocol::INVALID_OBJECT, "get_popup's positioner isn't one");
                };
                if positioner.size.0 <= 0 || positioner.size.1 <= 0 {
                    // xdg_wm_base.invalid_positioner
                    return self.error(c, id, 5, "a popup's positioner needs its size");
                }
                let positioner = *positioner;
                if let Some(Object { data: Data::XdgSurface { role, .. }, .. }) = c.objects.get_mut(&id) {
                    *role = Some(popup);
                }
                c.objects.insert(popup, new(Interface::XdgPopup, Data::Popup { surface, parent, positioner, grab: false, token: None }));
            }
            (Interface::XdgPopup, 1) => {
                if let Some(Object { data: Data::Popup { grab, .. }, .. }) = c.objects.get_mut(&id) {
                    *grab = true;
                }
            }
            // Placed again: the desktop's configure follows, with `repositioned` before it.
            (Interface::XdgPopup, 2) => {
                let Some(Data::Positioner(new_positioner)) = c.objects.get(&args[0].uint()).map(|o| &o.data) else { return };
                let new_positioner = *new_positioner;
                let token = args[1].uint();
                let window = self.window_of(c, id);
                if let Some(Object { data: Data::Popup { positioner, token: pending, .. }, .. }) = c.objects.get_mut(&id) {
                    *positioner = new_positioner;
                    *pending = Some(token);
                }
                if let Some(w) = window {
                    desktop::lock(&self.desktop).set_positioner(w, new_positioner);
                }
            }
            (Interface::DecorationManager, 1) => {
                let (decoration, toplevel) = (args[0].uint(), args[1].uint());
                c.objects.insert(decoration, new(Interface::ToplevelDecoration, Data::Decoration { toplevel }));
                // The desktop's frame, unless it asks otherwise.
                self.decorate(c, decoration, protocol::SERVER_SIDE);
            }
            (Interface::ToplevelDecoration, 1) => {
                let mode = if args[0].uint() == protocol::CLIENT_SIDE { protocol::CLIENT_SIDE } else { protocol::SERVER_SIDE };
                self.decorate(c, id, mode);
            }
            (Interface::ToplevelDecoration, 2) => self.decorate(c, id, protocol::SERVER_SIDE),
            (Interface::ToplevelDecoration, 0) => {
                // Back to drawing its own, from its next commit.
                self.decorate(c, id, protocol::CLIENT_SIDE);
                self.destroy(c, id);
            }
            (Interface::Subcompositor, 1) => {
                let (sub, surface, parent) = (args[0].uint(), args[1].uint(), args[2].uint());
                let is_surface = |s: u32| matches!(c.objects.get(&s), Some(Object { interface: Interface::Surface, .. }));
                if !is_surface(surface) || !is_surface(parent) || surface == parent {
                    // wl_subcompositor.bad_surface
                    return self.error(c, id, 0, "get_subsurface of something that isn't a wl_surface, or itself");
                }
                c.objects.insert(sub, new(Interface::Subsurface, Data::Subsurface { surface, parent, at: (0, 0) }));
                self.surface(c, surface, |s| s.subsurface = Some(sub));
            }
            (Interface::Subsurface, 1) => {
                let at = (args[0].int(), args[1].int());
                let Some(Object { data: Data::Subsurface { surface, parent, at: pos }, .. }) = c.objects.get_mut(&id) else { return };
                *pos = at;
                let (surface, parent) = (*surface, *parent);
                let (px, py) = offset_of(c, parent);
                let at = (at.0 - px, at.1 - py);
                let window = match c.objects.get(&surface).map(|o| &o.data) {
                    Some(Data::Surface(s)) => s.window,
                    _ => None,
                };
                // Moved at once (Wayland applies it with the parent's next commit).
                if let Some(w) = window {
                    let mut d = desktop::lock(&self.desktop);
                    let size = d.content_rect(w).map(|r| (r.width(), r.height())).unwrap_or((1, 1));
                    d.set_positioner(w, at_point(at, size));
                }
            }
            (Interface::DataDeviceManager, 0) => {
                c.objects.insert(args[0].uint(), new(Interface::DataSource, Data::Source(Vec::new())));
            }
            (Interface::DataDeviceManager, 1) => {
                c.objects.insert(args[0].uint(), new(Interface::DataDevice, Data::Plain));
            }
            (Interface::DataSource, 0) => {
                if let Some(Object { data: Data::Source(mimes), .. }) = c.objects.get_mut(&id) {
                    mimes.push(args[0].str().to_string());
                }
            }
            (Interface::DataSource, 1) => {
                // Its selection goes with it.
                if let Some((source, owner, obj)) = self.selection
                    && owner == client
                    && obj == id
                {
                    let mut d = desktop::lock(&self.desktop);
                    if d.selection_source(Selection::Clipboard) == Some(source) {
                        d.clear_selection(Selection::Clipboard);
                    }
                    self.selection = None;
                }
                self.destroy(c, id);
            }
            // The clipboard, from a client: its source's types offered on the desktop, its data asked for when pasted.
            (Interface::DataDevice, 1) => {
                let source = args[0].uint();
                if let Some((_, owner, old)) = self.selection.take()
                    && owner == client
                    && old != source
                {
                    send(c, old, 2, vec![]);
                }
                let mut d = desktop::lock(&self.desktop);
                match c.objects.get(&source).map(|o| &o.data) {
                    Some(Data::Source(mimes)) => {
                        let window = c.keyboard_focus.or_else(|| self.windows.iter().find(|(_, (cl, _))| *cl == client).map(|(&w, _)| w));
                        let offer = d.offer(Selection::Clipboard, window, mimes.clone());
                        self.selection = Some((offer, client, source));
                    }
                    _ => d.clear_selection(Selection::Clipboard),
                }
            }
            (Interface::DataOffer, 1) => {
                let Arg::Fd(file) = &args[1] else { return };
                let transfer = desktop::lock(&self.desktop).request(Selection::Clipboard, args[0].str(), None);
                if let Some(t) = transfer {
                    self.receiving.push((t, file.clone()));
                }
            }
            (Interface::DataDevice, 2) | (Interface::DataOffer, 2) | (Interface::DataDeviceManager, 2) => self.destroy(c, id),
            (Interface::XdgToplevel, 1) => {
                let Some(w) = self.window_of(c, id) else { return };
                let parent = self.window_of(c, args[0].uint());
                desktop::lock(&self.desktop).set_owner(w, parent);
            }
            (Interface::XdgSurface, 3) => {
                let (x, y, w, h) = (args[0].int(), args[1].int(), args[2].int(), args[3].int());
                if w <= 0 || h <= 0 {
                    return self.error(c, id, protocol::INVALID_SIZE, "a window geometry with no size");
                }
                let Some(Data::XdgSurface { surface, .. }) = c.objects.get(&id).map(|o| &o.data) else { return };
                let surface = *surface;
                self.surface(c, surface, |s| s.pending_geometry = Some(Rect::new(x, y, x.saturating_add(w), y.saturating_add(h))));
            }
            (Interface::XdgSurface, 4) => {
                if let Some(w) = self.window_of(c, id) {
                    desktop::lock(&self.desktop).ack_configure(w, args[0].uint());
                }
            }
            (Interface::XdgToplevel, op @ 2..=13) => {
                let Some(w) = self.window_of(c, id) else { return };
                let mut d = desktop::lock(&self.desktop);
                match op {
                    2 => d.set_title(w, args[0].str()),
                    5 => {
                        d.begin_move(w);
                    }
                    6 => {
                        let e = args[2].uint();
                        let edges = [(1, edge::TOP), (2, edge::BOTTOM), (4, edge::LEFT), (8, edge::RIGHT)].iter().filter(|(b, _)| e & b != 0).fold(0, |a, (_, d)| a | d);
                        d.begin_resize(w, edges);
                    }
                    7 => {
                        let min = d.size_limits(w).0;
                        d.set_size_limits(w, min, (args[0].int(), args[1].int()));
                    }
                    8 => {
                        let max = d.size_limits(w).1;
                        d.set_size_limits(w, (args[0].int(), args[1].int()), max);
                    }
                    9 | 10 => d.set_maximized(w, op == 9),
                    11 | 12 => d.set_fullscreen(w, op == 11),
                    13 => d.set_minimized(w, true),
                    _ => {}
                }
            }
            (Interface::Seat, 0) => {
                c.objects.insert(args[0].uint(), new(Interface::Pointer, Data::Plain));
            }
            (Interface::Seat, 1) => {
                let keyboard = args[0].uint();
                c.objects.insert(keyboard, new(Interface::Keyboard, Data::Plain));
                if let Some((file, size)) = &self.keymap {
                    send(c, keyboard, 0, vec![Arg::Uint(protocol::XKB_V1), Arg::Fd(file.clone()), Arg::Uint(*size)]);
                }
                if version >= 4 {
                    let repeat = desktop::lock(&self.desktop).key_repeat();
                    send(c, keyboard, 5, vec![Arg::Int(repeat.rate as i32), Arg::Int(repeat.delay as i32)]);
                }
                // It has the keyboard already: told so at once.
                if let Some(w) = c.keyboard_focus
                    && let Some(&(_, surface)) = self.windows.get(&w)
                {
                    let serial = self.next_serial();
                    send(c, keyboard, 1, vec![Arg::Uint(serial), Arg::Object(surface), Arg::Array(Vec::new())]);
                    let (depressed, locked) = c.mods;
                    send(c, keyboard, 4, vec![Arg::Uint(serial), Arg::Uint(depressed), Arg::Uint(0), Arg::Uint(locked), Arg::Uint(0)]);
                }
            }
            (Interface::Seat, 2) => {
                c.objects.insert(args[0].uint(), new(Interface::Touch, Data::Plain));
            }
            (Interface::Pointer, 0) => {
                let (surface, hotspot) = (args[1].uint(), (args[2].int(), args[3].int()));
                let Some(w) = c.pointer_focus else { return };
                if surface == 0 {
                    desktop::lock(&self.desktop).set_cursor(w, Cursor::Hidden);
                    return;
                }
                self.surface(c, surface, |s| s.cursor = Some(hotspot));
                if let Some(cursor) = cursor_of(c, surface) {
                    desktop::lock(&self.desktop).set_cursor(w, cursor);
                }
            }
            (Interface::CursorShapeManager, 1 | 2) => {
                c.objects.insert(args[0].uint(), new(Interface::CursorShapeDevice, Data::Plain));
            }
            (Interface::CursorShapeDevice, 1) => {
                let value = args[1].uint();
                let Some(shape) = (value as usize).checked_sub(1).and_then(|i| protocol::CURSOR_SHAPES.get(i)).and_then(|n| display::cursor::Shape::from_name(n)) else {
                    return self.error(c, id, protocol::INVALID_SHAPE, &format!("{value} isn't a cursor shape"));
                };
                if let Some(w) = c.pointer_focus {
                    desktop::lock(&self.desktop).set_cursor(w, Cursor::Shape(shape));
                }
            }
            (Interface::TextInputManager, 1) => {
                let input = args[0].uint();
                c.objects.insert(input, new(Interface::TextInput, Data::TextInput(Box::default())));
                // Made while one of its surfaces has the keyboard: told so at once.
                if let Some(&(_, surface)) = c.keyboard_focus.and_then(|w| self.windows.get(&w)) {
                    send(c, input, 0, vec![Arg::Object(surface)]);
                    if let Some(Data::TextInput(t)) = c.objects.get_mut(&input).map(|o| &mut o.data) {
                        t.focus = Some(surface);
                    }
                }
            }
            (Interface::TextInput, op @ 1..=7) => {
                let Some(Data::TextInput(t)) = c.objects.get_mut(&id).map(|o| &mut o.data) else { return };
                match op {
                    // Enabling starts afresh: what was set before it goes.
                    1 => {
                        t.pending_enabled = Some(true);
                        t.pending_rect = Some(Rect::default());
                    }
                    2 => t.pending_enabled = Some(false),
                    6 => t.pending_rect = Some(Rect::new(args[0].int(), args[1].int(), args[0].int().saturating_add(args[2].int()), args[1].int().saturating_add(args[3].int()))),
                    7 => {
                        t.commits = t.commits.wrapping_add(1);
                        t.enabled = t.pending_enabled.take().unwrap_or(t.enabled);
                        t.rect = t.pending_rect.take().unwrap_or(t.rect);
                        let (focus, enabled, rect) = (t.focus, t.enabled, t.rect);
                        self.tell_text_input(c, focus, enabled, rect);
                    }
                    // Surrounding text, its change's cause and the content type: an input method on the desktop
                    // doesn't ask for them yet.
                    _ => {}
                }
            }
            // Relative motion: every window of the client's gets it from now on.
            (Interface::RelativePointerManager, 1) => {
                c.objects.insert(args[0].uint(), new(Interface::RelativePointer, Data::Plain));
                let mut d = desktop::lock(&self.desktop);
                for (w, _) in self.windows.iter().filter(|(_, (cl, _))| *cl == client) {
                    d.set_relative_motion(*w, true);
                }
            }
            (Interface::PointerConstraints, op @ (1 | 2)) => {
                let (obj, surface, region, lifetime) = (args[0].uint(), args[1].uint(), args[3].uint(), args[4].uint());
                if !matches!(c.objects.get(&surface), Some(Object { interface: Interface::Surface, .. })) {
                    return self.error(c, id, protocol::INVALID_OBJECT, "a pointer constraint on something that isn't a wl_surface");
                }
                let taken = c.objects.values().any(|o| matches!(o.data, Data::Constraint { surface: s, dead: false, .. } if s == surface));
                if taken {
                    return self.error(c, id, protocol::ALREADY_CONSTRAINED, "that surface's pointer is constrained already");
                }
                let region = region_bounds(c, region);
                let interface = if op == 1 { Interface::LockedPointer } else { Interface::ConfinedPointer };
                c.objects.insert(obj, new(interface, Data::Constraint { surface, region, oneshot: lifetime == protocol::ONESHOT, taken: false, dead: false }));
                self.constrain(c, obj);
            }
            (Interface::LockedPointer, 2) | (Interface::ConfinedPointer, 1) => {
                let bounds = region_bounds(c, args[0].uint());
                if let Some(Object { data: Data::Constraint { region, .. }, .. }) = c.objects.get_mut(&id) {
                    *region = bounds;
                }
                // A lock's region only says where it may be taken; a confinement's is where it's kept.
                if matches!(interface, Interface::ConfinedPointer) {
                    self.constrain(c, id);
                }
            }
            (Interface::LockedPointer | Interface::ConfinedPointer, 0) => {
                if let Some(Data::Constraint { surface, dead: false, .. }) = c.objects.get(&id).map(|o| &o.data)
                    && let Some(w) = surface_window(c, *surface)
                {
                    desktop::lock(&self.desktop).set_pointer_constraint(w, None);
                }
                self.destroy(c, id);
            }
            (Interface::Activation, 1) => {
                c.objects.insert(args[0].uint(), new(Interface::ActivationToken, Data::Token { committed: false }));
            }
            // A token from the desktop, if the client may move the focus now (one that can't be used, if not).
            (Interface::ActivationToken, 3) => {
                let Some(Object { data: Data::Token { committed }, .. }) = c.objects.get_mut(&id) else { return };
                if std::mem::replace(committed, true) {
                    // xdg_activation_token_v1.already_used
                    return self.error(c, id, 0, "an activation token committed twice");
                }
                let token = desktop::lock(&self.desktop).activation_token(Some(client)).unwrap_or(0);
                send(c, id, 0, vec![Arg::Str(Some(format!("carbon-{token}")))]);
            }
            (Interface::Activation, 2) => {
                let token = args[0].str().strip_prefix("carbon-").and_then(|t| t.parse::<u32>().ok()).filter(|&t| t != 0);
                if let Some(w) = surface_window(c, args[1].uint()) {
                    desktop::lock(&self.desktop).request_activation(w, Some(client), token);
                }
            }
            (Interface::XdgOutputManager, 1) => {
                let xdg = args[0].uint();
                let output = args[1].uint();
                c.objects.insert(xdg, new(Interface::XdgOutput, Data::Plain));
                send_xdg_output(c, xdg, version, &self.output);
                if version >= 3 {
                    send(c, output, 2, vec![]);
                }
            }
            // Destroys.
            (Interface::RelativePointerManager | Interface::RelativePointer | Interface::PointerConstraints | Interface::Activation | Interface::XdgOutputManager | Interface::XdgOutput, 0)
            | (Interface::ActivationToken, 4) => {
                self.destroy(c, id);
            }
            (Interface::Seat, 3)
            | (Interface::Pointer, 1)
            | (Interface::Keyboard | Interface::Touch | Interface::Output | Interface::Subcompositor | Interface::Subsurface, 0)
            | (Interface::CursorShapeManager | Interface::CursorShapeDevice | Interface::TextInputManager | Interface::TextInput, 0) => {
                self.destroy(c, id);
            }
            (Interface::Surface | Interface::XdgSurface | Interface::XdgToplevel | Interface::Region | Interface::Buffer | Interface::XdgWmBase | Interface::XdgPositioner | Interface::XdgPopup, 0)
            | (Interface::ShmPool | Interface::Shm, 1) => {
                self.destroy(c, id);
            }
            _ => {}
        }
    }

    /// Changes a surface's state.
    fn surface(&mut self, c: &mut Client, id: u32, f: impl FnOnce(&mut Surface)) {
        if let Some(Object { data: Data::Surface(s), .. }) = c.objects.get_mut(&id) {
            f(s);
        }
    }

    /// A pointer constraint told to the desktop, for its surface's window (once it has one): a lock, or a confinement
    /// to its region (in its window's content coordinates, past the shadow it draws).
    fn constrain(&mut self, c: &Client, obj: u32) {
        let Some(Object { interface, data: Data::Constraint { surface, region, dead: false, .. }, .. }) = c.objects.get(&obj) else { return };
        let Some(w) = surface_window(c, *surface) else { return };
        let constraint = match interface {
            Interface::LockedPointer => Constraint::Lock,
            _ => {
                let (ox, oy) = offset_of(c, *surface);
                Constraint::Confine(region.map_or(Rect::default(), |r| r.offset(-ox, -oy)))
            }
        };
        let mut d = desktop::lock(&self.desktop);
        // The same again would let go and take it again.
        if d.pointer_constraint(w) != Some(constraint) {
            d.set_pointer_constraint(w, Some(constraint));
        }
    }

    /// A constraint taken or let go on a window: its `locked` or `confined`, `unlocked` or `unconfined`. A one-shot
    /// one let go is dead, and the desktop told it's gone.
    fn constrained(&mut self, window: WindowId, on: bool) {
        let Some((client, surface)) = self.windows.get(&window).copied() else { return };
        let Some(c) = self.clients.get_mut(&client) else { return };
        let mut ended = false;
        let objs: Vec<u32> = c.objects.iter().filter(|(_, o)| matches!(o.data, Data::Constraint { surface: s, dead: false, .. } if s == surface)).map(|(&id, _)| id).collect();
        for obj in objs {
            let Some(Object { data: Data::Constraint { taken, oneshot, dead, .. }, .. }) = c.objects.get_mut(&obj) else { continue };
            if *taken == on {
                continue;
            }
            *taken = on;
            if !on && *oneshot {
                *dead = true;
                ended = true;
            }
            send(c, obj, if on { 0 } else { 1 }, vec![]);
        }
        if ended {
            desktop::lock(&self.desktop).set_pointer_constraint(window, None);
        }
    }

    /// The desktop window an `xdg_surface` or `xdg_toplevel` shows its surface in.
    fn window_of(&self, c: &Client, id: u32) -> Option<WindowId> {
        let surface = match c.objects.get(&id).map(|o| &o.data) {
            Some(Data::XdgSurface { surface, .. } | Data::Toplevel { surface } | Data::Popup { surface, .. }) => *surface,
            _ => return None,
        };
        match c.objects.get(&surface).map(|o| &o.data) {
            Some(Data::Surface(s)) => s.window,
            _ => None,
        }
    }

    /// `wl_surface.commit`: what's pending made current on its desktop window; a toplevel's first commit, with no
    /// buffer, gets its first configure, and its first buffer shows it.
    fn commit(&mut self, client: u64, c: &mut Client, id: u32) {
        let Some(Object { data: Data::Surface(s), .. }) = c.objects.get_mut(&id) else { return };
        let pending_buffer = s.pending_buffer.take();
        let damage = std::mem::take(&mut s.pending_damage);
        let frames = std::mem::take(&mut s.pending_frames);
        if let Some(scale) = s.pending_scale.take() {
            s.scale = scale;
        }
        let geometry_set = s.pending_geometry.take();
        if geometry_set.is_some() {
            s.geometry = geometry_set;
        }
        let input_set = s.pending_input.take();
        if let Some(region) = &input_set {
            s.input = region.clone();
        }
        let input = s.input.clone();
        let had_window = s.window.is_some();
        let (window, xdg, configured, scale, geometry, old_size, sent) = (s.window, s.xdg, s.configured, s.scale, s.geometry, s.size, s.sent);
        let buffer = match pending_buffer {
            Some(Some(b)) => match c.objects.get(&b).map(|o| &o.data) {
                Some(Data::Buffer(buffer)) => Some(Some((b, buffer.clone()))),
                _ => return self.error(c, id, protocol::INVALID_OBJECT, "attach of something that isn't a wl_buffer"),
            },
            Some(None) => Some(None),
            None => None,
        };
        // A popup's first commit: made on the desktop now, placed and grabbing as it's asked.
        let window = match window {
            None if !configured && xdg.is_some() => self.make_popup(client, c, id, xdg),
            // A subsurface, with its first buffer: shown as a popup of its parent's window, at its place on it.
            None if matches!(buffer, Some(Some(_))) => self.make_subsurface(client, c, id, buffer.as_ref().and_then(|b| b.as_ref()).map(|(_, b)| b.pixels.size())),
            w => w,
        };
        let Some(window) = window else {
            if let Some(Data::Surface(s)) = c.objects.get_mut(&id).map(|o| &mut o.data)
                && let Some(b) = &buffer
            {
                s.buffer = b.as_ref().map(|(id, _)| *id);
            }
            // A cursor: shown as the pointer's, over whichever of its windows the pointer's in.
            if let Some(w) = c.pointer_focus
                && let Some(cursor) = cursor_of(c, id)
            {
                desktop::lock(&self.desktop).set_cursor(w, cursor);
            }
            // No role yet, so nothing shows it: its frames are done at once.
            for cb in frames {
                send(c, cb, 0, vec![Arg::Uint(0)]);
                delete_id(c, cb);
            }
            return;
        };
        if !configured && xdg.is_some() {
            // The first configure: the client picks its size.
            let serial = self.next_serial();
            if let Some(Data::Surface(s)) = c.objects.get_mut(&id).map(|o| &mut o.data) {
                s.configured = true;
            }
            match toplevel_of(c, xdg).map(|r| (r, c.objects.get(&r).map(|o| o.interface))) {
                Some((popup, Some(Interface::XdgPopup))) => {
                    let (x, y, w, h) = self.popup_geometry(window);
                    send(c, popup, 0, vec![Arg::Int(x), Arg::Int(y), Arg::Int(w), Arg::Int(h)]);
                }
                Some((top, _)) => send(c, top, 0, vec![Arg::Int(0), Arg::Int(0), Arg::Array(Vec::new())]),
                None => {}
            }
            configure_xdg(c, id, xdg, serial);
        }
        let mut d = desktop::lock(&self.desktop);
        let mut size = None;
        if let Some(b) = &buffer {
            match b {
                Some((_, buffer)) => {
                    let (w, h) = buffer.pixels.size();
                    size = Some((w / scale, h / scale));
                    d.attach(window, Some(buffer.clone()));
                }
                None => d.attach(window, None),
            }
        }
        d.set_buffer_scale(window, scale as u32);
        // Its window geometry, within its buffer (as the protocol clamps it): its content's place in it, the rest a
        // shadow drawn around its window. With none, all of its buffer is its window.
        let logical = size.unwrap_or(old_size);
        let window_part = geometry.map(|g| g.intersect(&Rect::new(0, 0, logical.0, logical.1))).filter(|g| !g.is_empty());
        if size.is_some() || geometry_set.is_some() {
            d.set_buffer_offset(window, window_part.map(|g| (g.left, g.top)));
        }
        for r in damage {
            d.damage(window, r);
        }
        // Its input region, in its own coordinates: in its content's, past its window geometry's corner. Set
        // again with the geometry, and given to a window just made (a subsurface's, at its first buffer).
        if input_set.is_some() || geometry_set.is_some() || !had_window {
            let (ox, oy) = window_part.map_or((0, 0), |g| (g.left, g.top));
            d.set_input_region(window, input.map(|rects| rects.into_iter().map(|r| r.offset(-ox, -oy)).collect()));
        }
        if !frames.is_empty() {
            d.request_frame(window);
        }
        d.commit(window);
        // Its size is its window geometry's, or its buffer's; shown with its first buffer, hidden with none. Not while
        // a configure it hasn't acknowledged is out (this was drawn before it), nor while the desktop places it.
        let current = xdg.is_none() || (d.acked(window) == sent && !d.configuring(window));
        let placed = d.states(window) & (state::MAXIMIZED | state::FULLSCREEN) != 0;
        if let Some((w, h)) = window_part.map(|g| (g.width(), g.height())).or(size).filter(|_| size.is_some() || geometry_set.is_some())
            && current
            && !placed
            && let Some(c) = d.content_rect(window)
            && (c.width(), c.height()) != (w, h)
        {
            let framed = d.window_info(window).is_some_and(|i| i.framed);
            d.set_rect(window, Insets::of(framed).around(&Rect::new(c.left, c.top, c.left + w, c.top + h)));
        }
        let mut shown = false;
        let whole_scale = d.output().whole_scale();
        match &buffer {
            Some(Some(_)) if !d.is_visible(window) => {
                d.set_visible(window, true);
                d.request_activation(window, Some(client), None);
                shown = true;
            }
            Some(None) if d.is_visible(window) => d.set_visible(window, false),
            _ => {}
        }
        drop(d);
        // Shown: its subsurfaces with it (hidden with it before), it's on the output, and told the scale to draw at.
        if shown {
            let subs: Vec<WindowId> = c
                .objects
                .values()
                .filter_map(|o| match &o.data {
                    Data::Subsurface { surface, parent, .. } if *parent == id => match c.objects.get(surface).map(|o| &o.data) {
                        Some(Data::Surface(s)) if s.buffer.is_some() => s.window,
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let mut d = desktop::lock(&self.desktop);
            for w in subs {
                d.set_visible(w, true);
            }
            if !objects_of(c, Interface::RelativePointer).is_empty() {
                d.set_relative_motion(window, true);
            }
            drop(d);
            // A constraint asked for before it was shown, taken now it can be.
            let pending: Vec<u32> = c.objects.iter().filter(|(_, o)| matches!(o.data, Data::Constraint { surface, dead: false, .. } if surface == id)).map(|(&o, _)| o).collect();
            for obj in pending {
                self.constrain(c, obj);
            }
            for output in objects_of(c, Interface::Output) {
                send(c, id, 0, vec![Arg::Object(output.0)]);
            }
            let surface_version = c.objects.get(&id).map_or(1, |o| o.version);
            if surface_version >= 6 {
                send(c, id, 2, vec![Arg::Int(whole_scale as i32)]);
                send(c, id, 3, vec![Arg::Uint(0)]);
            }
        }
        if let Some(Data::Surface(s)) = c.objects.get_mut(&id).map(|o| &mut o.data) {
            s.waiting.extend(frames);
            if let Some(b) = buffer {
                s.buffer = b.as_ref().map(|(id, _)| *id);
                s.size = size.unwrap_or(s.size);
            }
            let offset = window_part.map_or((0, 0), |g| (g.left, g.top));
            let moved = offset != s.offset;
            s.offset = offset;
            // Its window's corner moved within it (a window geometry committed after its subsurfaces were placed, as
            // GTK's shadow margins come with the first frame): its subsurfaces are placed again, from the new corner.
            if moved {
                let subs: Vec<(WindowId, (i32, i32))> = c
                    .objects
                    .values()
                    .filter_map(|o| match &o.data {
                        Data::Subsurface { surface, parent, at } if *parent == id => match c.objects.get(surface).map(|o| &o.data) {
                            Some(Data::Surface(s)) => s.window.map(|w| (w, *at)),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect();
                let mut d = desktop::lock(&self.desktop);
                for (w, at) in subs {
                    let size = d.content_rect(w).map(|r| (r.width(), r.height())).unwrap_or((1, 1));
                    d.set_positioner(w, at_point((at.0 - offset.0, at.1 - offset.1), size));
                }
            }
        }
    }

    /// An object destroyed: a toplevel's window goes with it, and the client's told its ID is free.
    fn destroy(&mut self, c: &mut Client, id: u32) {
        let Some(object) = c.objects.remove(&id) else { return };
        match object.data {
            Data::Subsurface { surface, .. } => {
                let window = match c.objects.get_mut(&surface).map(|o| &mut o.data) {
                    Some(Data::Surface(s)) => {
                        s.subsurface = None;
                        s.window.take()
                    }
                    _ => None,
                };
                if let Some(w) = window {
                    self.windows.remove(&w);
                    desktop::lock(&self.desktop).destroy(w);
                }
            }
            Data::Toplevel { surface } | Data::XdgSurface { surface, .. } | Data::Popup { surface, .. } => {
                let window = match c.objects.get_mut(&surface).map(|o| &mut o.data) {
                    Some(Data::Surface(s)) => {
                        s.configured = false;
                        if matches!(object.interface, Interface::XdgSurface) {
                            s.xdg = None;
                        }
                        s.window.take()
                    }
                    _ => None,
                };
                if let Some(w) = window {
                    self.windows.remove(&w);
                    desktop::lock(&self.desktop).destroy(w);
                }
            }
            Data::Surface(s) => {
                if let Some(w) = s.window {
                    self.windows.remove(&w);
                    desktop::lock(&self.desktop).destroy(w);
                }
            }
            Data::Buffer(b) => {
                self.buffers.remove(&b.id);
            }
            Data::TextInput(t) if t.enabled => self.tell_text_input(c, t.focus, false, Rect::default()),
            _ => {}
        }
        delete_id(c, id);
    }

    /// A client gone: its windows with it.
    fn disconnect(&mut self, id: u64, c: Client) {
        let gone: Vec<WindowId> = self.windows.iter().filter(|(_, (client, _))| *client == id).map(|(&w, _)| w).collect();
        let mut d = desktop::lock(&self.desktop);
        for w in gone {
            self.windows.remove(&w);
            d.destroy(w);
        }
        self.buffers.retain(|_, (client, _)| *client != id);
        drop(d);
        let _ = c.socket.shutdown(net::Shutdown::Both);
    }

    /// Another's offer took the clipboard: the client whose source had it is told it's `cancelled`.
    fn selection_cancelled(&mut self) {
        if let Some((source, client, obj)) = self.selection
            && desktop::lock(&self.desktop).selection_source(Selection::Clipboard) != Some(source)
        {
            self.selection = None;
            if let Some(c) = self.clients.get_mut(&client) {
                send(c, obj, 2, vec![]);
            }
        }
    }

    /// What the desktop decided, told to each window's client.
    fn desktop_events(&mut self) {
        let (events, output) = {
            let mut d = desktop::lock(&self.desktop);
            // Its own clients' events, leaving the X server's (and those of windows already gone, which it ignores).
            (d.take_events_where(|c| c.is_none_or(|c| c < super::x11::CLIENT_IDS)), d.output().clone())
        };
        // Another's offer took the clipboard: the client whose source had it is told first, as Wayland has it, since
        // a client that hears `cancelled` after the new selection empties its clipboard (GTK does).
        if events.iter().any(|e| matches!(e, Event::SelectionCancelled { selection: Selection::Clipboard, .. })) {
            self.selection_cancelled();
        }
        // The clipboard changed: the client with the keyboard is offered it.
        let changed = desktop::lock(&self.desktop).take_observed(self.observer).selections.contains(&Selection::Clipboard);
        if changed {
            let focused: Vec<u64> = self.clients.iter().filter(|(_, c)| c.keyboard_focus.is_some()).map(|(&id, _)| id).collect();
            for client in focused {
                self.send_selection(client);
            }
        }
        // The output changed: every `wl_output` is told it again.
        if output != self.output {
            self.output = output;
            for c in self.clients.values_mut() {
                for (id, version) in objects_of(c, Interface::XdgOutput) {
                    send_xdg_output(c, id, version, &self.output);
                }
                for (id, version) in objects_of(c, Interface::Output) {
                    send_output(c, id, version, &self.output);
                }
            }
        }
        for event in events {
            match event {
                Event::Configure { window, states, serial, .. } => self.configure(window, states, Some(serial)),
                Event::Activated { window, active } => {
                    let states = desktop::lock(&self.desktop).states(window);
                    self.configure(window, states, None);
                    self.keyboard_focus(window, active);
                }
                Event::Pointer { window, x, y, what, .. } => self.pointer(window, x, y, what),
                Event::Constrained { window, on } => self.constrained(window, on),
                Event::Key { window, code, down, repeat, modifiers } => {
                    if code == keys::CAPS_LOCK && down && !repeat {
                        self.caps = !self.caps;
                    }
                    // Wayland's clients repeat keys themselves (`repeat_info`).
                    if !repeat {
                        self.key(window, code, down, modifiers);
                    }
                }
                Event::Scale { window, scale } => {
                    if let Some((client, surface)) = self.windows.get(&window).copied()
                        && let Some(c) = self.clients.get_mut(&client)
                        && c.objects.get(&surface).is_some_and(|o| o.version >= 6)
                    {
                        send(c, surface, 2, vec![Arg::Int(scale.div_ceil(display::output::SCALE_1X).max(1) as i32)]);
                    }
                }
                // A client's source asked for its data: a pipe, its write end sent to it, its read end read.
                Event::SendSelection { selection: Selection::Clipboard, mime, transfer, .. } => match self.selection {
                    Some((_, client, source)) if self.clients.contains_key(&client) => {
                        let pipe = Pipe::new();
                        let write = Open::new(Node::Pipe(pipe.clone(), true), crate::abi::O_WRONLY, "pipe:");
                        let read = Open::new(Node::Pipe(pipe, false), 0, "pipe:");
                        if let Some(c) = self.clients.get_mut(&client) {
                            send(c, source, 1, vec![Arg::Str(Some(mime)), Arg::Fd(write)]);
                        }
                        // Its read end, kept open until it's read to the end, then closed.
                        self.reading.push((read, transfer, Vec::new()));
                    }
                    _ => desktop::lock(&self.desktop).provide(transfer, None),
                },
                Event::SendSelection { transfer, .. } => desktop::lock(&self.desktop).provide(transfer, None),
                Event::PopupDone { window } => {
                    if let Some((client, surface)) = self.windows.get(&window).copied()
                        && let Some(c) = self.clients.get_mut(&client)
                        && let Some(popup) = toplevel_of_surface(c, surface)
                    {
                        send(c, popup, 1, vec![]);
                    }
                }
                Event::Close { window } => {
                    if let Some((client, surface)) = self.windows.get(&window).copied()
                        && let Some(c) = self.clients.get_mut(&client)
                        && let Some(top) = toplevel_of_surface(c, surface)
                    {
                        send(c, top, 1, vec![]);
                    }
                }
                Event::Frame { window, time } => {
                    if let Some((client, surface)) = self.windows.get(&window).copied()
                        && let Some(c) = self.clients.get_mut(&client)
                    {
                        let waiting = match c.objects.get_mut(&surface).map(|o| &mut o.data) {
                            Some(Data::Surface(s)) => std::mem::take(&mut s.waiting),
                            _ => Vec::new(),
                        };
                        for cb in waiting {
                            send(c, cb, 0, vec![Arg::Uint(time)]);
                            c.objects.remove(&cb);
                            delete_id(c, cb);
                        }
                    }
                }
                // `commit_string`, and `preedit_string`.
                Event::Text { window, text } => self.text_input_event(window, 3, || vec![Arg::Str(Some(text.clone()))]),
                Event::Preedit { window, text, cursor } => {
                    let (begin, end) = cursor.map_or((-1, -1), |(b, e)| (b as i32, e as i32));
                    let text = (!text.is_empty()).then_some(text);
                    self.text_input_event(window, 2, || vec![Arg::Str(text.clone()), Arg::Int(begin), Arg::Int(end)]);
                }
                Event::Released { buffer, .. } => {
                    if let Some((client, b)) = self.buffers.get(&buffer).copied()
                        && let Some(c) = self.clients.get_mut(&client)
                    {
                        send(c, b, 0, vec![]);
                    }
                }
                _ => {}
            }
        }
    }

    /// A popup's desktop window, made at its first commit: a popup of its parent surface's window, placed by its
    /// positioner, grabbing if it asked; hidden until its first buffer.
    fn make_popup(&mut self, client: u64, c: &mut Client, surface: u32, xdg: Option<u32>) -> Option<WindowId> {
        let popup = toplevel_of(c, xdg)?;
        let Some(Data::Popup { parent, positioner, grab, .. }) = c.objects.get(&popup).map(|o| &o.data) else { return None };
        let (positioner, grab) = (*positioner, *grab);
        let parent = parent.and_then(|p| match c.objects.get(&p).map(|o| &o.data) {
            Some(Data::Surface(s)) => s.window,
            _ => None,
        })?;
        let window = desktop::lock(&self.desktop).create(Spec { role: Role::Popup { parent, positioner, grab }, client, fill: 0xffffff, ..Spec::default() });
        self.windows.insert(window, (client, surface));
        self.surface(c, surface, |s| s.window = Some(window));
        Some(window)
    }

    /// A subsurface's desktop window: a popup of its parent surface's window that doesn't grab, placed at its place on
    /// it, the size of its first buffer. None while its parent has no window.
    fn make_subsurface(&mut self, client: u64, c: &mut Client, surface: u32, size: Option<(i32, i32)>) -> Option<WindowId> {
        let Some(Data::Surface(s)) = c.objects.get(&surface).map(|o| &o.data) else { return None };
        let (scale, sub) = (s.scale.max(1), s.subsurface?);
        let Some(Data::Subsurface { parent, at, .. }) = c.objects.get(&sub).map(|o| &o.data) else { return None };
        let (parent, at) = (*parent, *at);
        let (parent, at) = match c.objects.get(&parent).map(|o| &o.data) {
            // Its place is on its parent's surface; the desktop places it on its parent's content.
            Some(Data::Surface(p)) => (p.window?, (at.0 - p.offset.0, at.1 - p.offset.1)),
            _ => return None,
        };
        let (w, h) = size.map_or((1, 1), |(w, h)| ((w / scale).max(1), (h / scale).max(1)));
        let window = desktop::lock(&self.desktop).create(Spec { role: Role::Popup { parent, positioner: at_point(at, (w, h)), grab: false }, client, fill: 0xffffff, ..Spec::default() });
        self.windows.insert(window, (client, surface));
        self.surface(c, surface, |s| s.window = Some(window));
        Some(window)
    }

    /// Where a popup is, relative to its parent's content, and its size: `xdg_popup.configure`'s.
    fn popup_geometry(&self, window: WindowId) -> (i32, i32, i32, i32) {
        let d = desktop::lock(&self.desktop);
        let Some(r) = d.rect(window) else { return (0, 0, 0, 0) };
        let parent = d.window_info(window).and_then(|i| i.parent).and_then(|p| d.content_rect(p)).unwrap_or_default();
        (r.left - parent.left, r.top - parent.top, r.width(), r.height())
    }

    /// A toplevel decorated as `mode` says: the desktop's frame for server-side, none for client-side; the decoration
    /// told, and the toplevel configured after (once it's had its first).
    fn decorate(&mut self, c: &mut Client, decoration: u32, mode: u32) {
        let Some(Data::Decoration { toplevel }) = c.objects.get(&decoration).map(|o| &o.data) else { return };
        let toplevel = *toplevel;
        send(c, decoration, 0, vec![Arg::Uint(mode)]);
        let Some(w) = self.window_of(c, toplevel) else { return };
        let states = {
            let mut d = desktop::lock(&self.desktop);
            d.set_framed(w, mode == protocol::SERVER_SIDE, Keep::Content);
            d.states(w)
        };
        // The desktop's own configure comes with its events; one now, so the decoration's is followed by one.
        let Some(&(_, surface)) = self.windows.get(&w) else { return };
        let configured = matches!(c.objects.get(&surface).map(|o| &o.data), Some(Data::Surface(s)) if s.configured);
        if configured {
            let serial = self.next_serial();
            let (r, shown) = {
                let d = desktop::lock(&self.desktop);
                (d.content_rect(w).unwrap_or_default(), d.is_visible(w))
            };
            // Until it's shown, its size is the client's to pick.
            let (width, height) = if shown { (r.width(), r.height()) } else { (0, 0) };
            let xdg = match c.objects.get(&surface).map(|o| &o.data) {
                Some(Data::Surface(s)) => s.xdg,
                _ => None,
            };
            let active = desktop::lock(&self.desktop).active() == Some(w);
            let mut list = Vec::new();
            for (bit, value) in [(state::MAXIMIZED, protocol::MAXIMIZED), (state::FULLSCREEN, protocol::FULLSCREEN)] {
                if states & bit != 0 {
                    list.extend(value.to_ne_bytes());
                }
            }
            if active {
                list.extend(protocol::ACTIVATED.to_ne_bytes());
            }
            send(c, toplevel, 0, vec![Arg::Int(width), Arg::Int(height), Arg::Array(list)]);
            configure_xdg(c, surface, xdg, serial);
        }
    }

    /// The pointer, to the client whose window it's in: each of its `wl_pointer`s told, each event ending a frame.
    fn pointer(&mut self, window: WindowId, x: i32, y: i32, what: Pointer) {
        let Some((client, surface)) = self.windows.get(&window).copied() else { return };
        let (serial, time) = (self.next_serial(), self.time());
        let Some(c) = self.clients.get_mut(&client) else { return };
        match what {
            Pointer::Enter => c.pointer_focus = Some(window),
            Pointer::Leave if c.pointer_focus == Some(window) => c.pointer_focus = None,
            _ => {}
        }
        if let Pointer::Relative { dx, dy } = what {
            let us = self.start.elapsed().as_micros() as u64;
            let (dx, dy) = (Arg::Fixed(dx.saturating_mul(256)), Arg::Fixed(dy.saturating_mul(256)));
            for (rel, _) in objects_of(c, Interface::RelativePointer) {
                send(c, rel, 0, vec![Arg::Uint((us >> 32) as u32), Arg::Uint(us as u32), dx.clone(), dy.clone(), dx.clone(), dy.clone()]);
            }
            for (pointer, version) in objects_of(c, Interface::Pointer) {
                if version >= 5 {
                    send(c, pointer, 5, vec![]);
                }
            }
            return;
        }
        // From its content's corner to its surface's: past the shadow it draws, if it has one.
        let (ox, oy) = offset_of(c, surface);
        let (x, y) = (x.saturating_add(ox), y.saturating_add(oy));
        let (fx, fy) = (Arg::Fixed(x.saturating_mul(256)), Arg::Fixed(y.saturating_mul(256)));
        for (pointer, version) in objects_of(c, Interface::Pointer) {
            match what {
                Pointer::Enter => send(c, pointer, 0, vec![Arg::Uint(serial), Arg::Object(surface), fx.clone(), fy.clone()]),
                Pointer::Leave => send(c, pointer, 1, vec![Arg::Uint(serial), Arg::Object(surface)]),
                Pointer::Move => send(c, pointer, 2, vec![Arg::Uint(time), fx.clone(), fy.clone()]),
                Pointer::Down(b) | Pointer::Up(b) => {
                    let code = match b {
                        Button::Left => protocol::BTN_LEFT,
                        Button::Right => protocol::BTN_RIGHT,
                        Button::Middle => protocol::BTN_MIDDLE,
                    };
                    send(c, pointer, 3, vec![Arg::Uint(serial), Arg::Uint(time), Arg::Uint(code), Arg::Uint(u32::from(matches!(what, Pointer::Down(_))))]);
                }
                Pointer::Scroll { x: sx, y: sy } => {
                    if version >= 5 {
                        // A wheel.
                        send(c, pointer, 6, vec![Arg::Uint(0)]);
                    }
                    // Wayland's axes go down and right; the desktop's y goes up. In 120ths, and as 15 pixels a notch.
                    for (axis, value) in [(0u32, -sy), (1, sx)] {
                        if value == 0 {
                            continue;
                        }
                        if version >= 8 {
                            send(c, pointer, 9, vec![Arg::Uint(axis), Arg::Int(value)]);
                        } else if version >= 5 && value / 120 != 0 {
                            send(c, pointer, 8, vec![Arg::Uint(axis), Arg::Int(value / 120)]);
                        }
                        send(c, pointer, 4, vec![Arg::Uint(time), Arg::Uint(axis), Arg::Fixed(value.saturating_mul(32))]);
                    }
                }
                Pointer::Relative { .. } => break,
            }
            if version >= 5 {
                send(c, pointer, 5, vec![]);
            }
        }
    }

    /// The keyboard came to one of a client's windows, or left it.
    fn keyboard_focus(&mut self, window: WindowId, active: bool) {
        let Some((client, surface)) = self.windows.get(&window).copied() else { return };
        let serial = self.next_serial();
        let held = desktop::lock(&self.desktop).modifiers();
        let mods = (keymap::xkb_mods(held), if self.caps { keymap::mods::LOCK } else { 0 });
        // The clipboard comes just before the keyboard, as Wayland has it.
        if active && self.clients.get(&client).is_some_and(|c| c.keyboard_focus.is_none()) {
            self.send_selection(client);
        }
        let Some(c) = self.clients.get_mut(&client) else { return };
        if active {
            c.keyboard_focus = Some(window);
            c.mods = mods;
        } else if c.keyboard_focus == Some(window) {
            c.keyboard_focus = None;
        }
        for (keyboard, _) in objects_of(c, Interface::Keyboard) {
            if active {
                send(c, keyboard, 1, vec![Arg::Uint(serial), Arg::Object(surface), Arg::Array(Vec::new())]);
                send(c, keyboard, 4, vec![Arg::Uint(serial), Arg::Uint(mods.0), Arg::Uint(0), Arg::Uint(mods.1), Arg::Uint(0)]);
            } else {
                send(c, keyboard, 2, vec![Arg::Uint(serial), Arg::Object(surface)]);
            }
        }
        // Its text inputs follow the keyboard: told it's in that surface, or that it left it (and so off, until it's
        // enabled again there).
        for (input, _) in objects_of(c, Interface::TextInput) {
            let Some(Data::TextInput(t)) = c.objects.get_mut(&input).map(|o| &mut o.data) else { continue };
            if active {
                if let Some(old) = t.focus.replace(surface).filter(|&old| old != surface) {
                    send(c, input, 1, vec![Arg::Object(old)]);
                }
                send(c, input, 0, vec![Arg::Object(surface)]);
            } else if t.focus == Some(surface) {
                t.focus = None;
                t.enabled = false;
                send(c, input, 1, vec![Arg::Object(surface)]);
                desktop::lock(&self.desktop).set_text_input(window, None);
            }
        }
    }

    /// A text input's committed state, told to the desktop for the window of the surface it's in: that window takes
    /// text, with its text cursor there (from the surface's coordinates to its content's), or it doesn't.
    fn tell_text_input(&self, c: &Client, focus: Option<u32>, enabled: bool, rect: Rect) {
        let Some(surface) = focus else { return };
        let Some(window) = (match c.objects.get(&surface).map(|o| &o.data) {
            Some(Data::Surface(s)) => s.window,
            _ => None,
        }) else {
            return;
        };
        let (ox, oy) = offset_of(c, surface);
        desktop::lock(&self.desktop).set_text_input(window, enabled.then(|| rect.offset(-ox, -oy)));
    }

    /// Text for a window, to each enabled text input of its client's that's in its surface: the event `opcode` with
    /// the arguments `args` makes, then `done` with how many commits it's made.
    fn text_input_event(&mut self, window: WindowId, opcode: u16, args: impl Fn() -> Vec<Arg<Arc<Open>>>) {
        let Some((client, surface)) = self.windows.get(&window).copied() else { return };
        let Some(c) = self.clients.get_mut(&client) else { return };
        for (input, _) in objects_of(c, Interface::TextInput) {
            let Some(Data::TextInput(t)) = c.objects.get(&input).map(|o| &o.data) else { continue };
            if !t.enabled || t.focus != Some(surface) {
                continue;
            }
            let commits = t.commits;
            send(c, input, opcode, args());
            send(c, input, 5, vec![Arg::Uint(commits)]);
        }
    }

    /// A key, to the client with the keyboard; then its modifiers, if they changed.
    fn key(&mut self, window: WindowId, code: u16, down: bool, modifiers: u8) {
        let Some((client, _)) = self.windows.get(&window).copied() else { return };
        let (serial, time) = (self.next_serial(), self.time());
        let mods = (keymap::xkb_mods(modifiers), if self.caps { keymap::mods::LOCK } else { 0 });
        let Some(c) = self.clients.get_mut(&client) else { return };
        let changed = c.mods != mods;
        c.mods = mods;
        for (keyboard, _) in objects_of(c, Interface::Keyboard) {
            send(c, keyboard, 3, vec![Arg::Uint(serial), Arg::Uint(time), Arg::Uint(u32::from(code)), Arg::Uint(u32::from(down))]);
            if changed {
                send(c, keyboard, 4, vec![Arg::Uint(serial), Arg::Uint(mods.0), Arg::Uint(0), Arg::Uint(mods.1), Arg::Uint(0)]);
            }
        }
    }

    /// An `xdg_toplevel.configure` and its `xdg_surface.configure`: the window's content size and states.
    fn configure(&mut self, window: WindowId, states: u32, serial: Option<u32>) {
        let Some((client, surface)) = self.windows.get(&window).copied() else { return };
        let serial = serial.unwrap_or_else(|| self.next_serial());
        // Until it's shown, its size is the client's to pick (0 by 0): its window's only a placeholder.
        let (size, active) = {
            let d = desktop::lock(&self.desktop);
            let size = d.content_rect(window).filter(|_| d.is_visible(window)).map_or((0, 0), |r| (r.width(), r.height()));
            (size, d.active() == Some(window))
        };
        let popup_at = self.popup_geometry(window);
        let Some(c) = self.clients.get_mut(&client) else { return };
        let (xdg, configured) = match c.objects.get(&surface).map(|o| &o.data) {
            Some(Data::Surface(s)) => (s.xdg, s.configured),
            _ => (None, false),
        };
        // Before its first commit it's told nothing: that commit's answered with its first configure.
        if !configured {
            return;
        }
        let Some(top) = toplevel_of(c, xdg) else { return };
        if let Some(Object { data: Data::Popup { token, .. }, version, .. }) = c.objects.get_mut(&top) {
            let (token, version) = (token.take(), *version);
            if let Some(token) = token.filter(|_| version >= 3) {
                send(c, top, 2, vec![Arg::Uint(token)]);
            }
            let (x, y, w, h) = popup_at;
            send(c, top, 0, vec![Arg::Int(x), Arg::Int(y), Arg::Int(w), Arg::Int(h)]);
            configure_xdg(c, surface, xdg, serial);
            return;
        }
        let mut list = Vec::new();
        for (bit, value) in [(state::MAXIMIZED, protocol::MAXIMIZED), (state::FULLSCREEN, protocol::FULLSCREEN), (state::RESIZING, protocol::RESIZING)] {
            if states & bit != 0 {
                list.extend(value.to_ne_bytes());
            }
        }
        if active {
            list.extend(protocol::ACTIVATED.to_ne_bytes());
        }
        send(c, top, 0, vec![Arg::Int(size.0), Arg::Int(size.1), Arg::Array(list)]);
        configure_xdg(c, surface, xdg, serial);
    }

    /// What's been sent to each client, written; one that's had a protocol error, disconnected after.
    fn flush(&mut self) {
        let ids: Vec<u64> = self.clients.keys().copied().collect();
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
            if c.gone || (c.closing && c.out.is_empty()) {
                let c = self.clients.remove(&id).expect("just found");
                self.disconnect(id, c);
            }
        }
    }
}

fn send(c: &mut Client, object: u32, opcode: u16, args: Vec<Arg<Arc<Open>>>) {
    wire::encode(object, opcode, args, &mut c.out, &mut c.out_fds);
}

fn delete_id(c: &mut Client, id: u32) {
    // Only the client's own IDs are given back.
    if id < 0xff00_0000 {
        send(c, 1, 1, vec![Arg::Uint(id)]);
    }
}

/// The `xdg_toplevel` of an `xdg_surface`.
fn toplevel_of(c: &Client, xdg: Option<u32>) -> Option<u32> {
    match c.objects.get(&xdg?).map(|o| &o.data) {
        Some(Data::XdgSurface { role, .. }) => *role,
        _ => None,
    }
}

fn toplevel_of_surface(c: &Client, surface: u32) -> Option<u32> {
    let xdg = match c.objects.get(&surface).map(|o| &o.data) {
        Some(Data::Surface(s)) => s.xdg,
        _ => None,
    };
    toplevel_of(c, xdg)
}

/// A client's objects of an interface, with their versions.
fn objects_of(c: &Client, interface: Interface) -> Vec<(u32, u32)> {
    let mut found: Vec<(u32, u32)> = c.objects.iter().filter(|(_, o)| o.interface == interface).map(|(&id, o)| (id, o.version)).collect();
    found.sort_unstable();
    found
}

/// Tells a `wl_output` what the desktop's output is: its geometry, its one mode (in its own pixels), its scale, name
/// and description, as its version has them, then that it's done.
fn send_output(c: &mut Client, id: u32, version: u32, output: &Output) {
    let (pw, ph) = output.pixels();
    let (mw, mh) = output.physical_mm;
    let model = Some("desktop".to_string());
    send(c, id, 0, vec![Arg::Int(0), Arg::Int(0), Arg::Int(mw), Arg::Int(mh), Arg::Int(0), Arg::Str(Some("carbon".into())), Arg::Str(model), Arg::Int(0)]);
    // Current and preferred.
    send(c, id, 1, vec![Arg::Uint(3), Arg::Int(pw), Arg::Int(ph), Arg::Int(output.refresh_mhz as i32)]);
    if version >= 2 {
        send(c, id, 3, vec![Arg::Int(output.whole_scale() as i32)]);
    }
    if version >= 4 {
        send(c, id, 4, vec![Arg::Str(Some(output.name.clone()))]);
        send(c, id, 5, vec![Arg::Str(Some(format!("carbon's {} output, {}x{}", output.name, output.width, output.height)))]);
    }
    if version >= 2 {
        send(c, id, 2, vec![]);
    }
}

/// Tells a `zxdg_output_v1` the output's place and size in logical pixels, its name and description; `done` up to
/// version 2 (from 3, `wl_output.done` says it).
fn send_xdg_output(c: &mut Client, id: u32, version: u32, output: &Output) {
    send(c, id, 0, vec![Arg::Int(0), Arg::Int(0)]);
    send(c, id, 1, vec![Arg::Int(output.width), Arg::Int(output.height)]);
    if version >= 2 {
        send(c, id, 3, vec![Arg::Str(Some(output.name.clone()))]);
        send(c, id, 4, vec![Arg::Str(Some(format!("carbon's {} output, {}x{}", output.name, output.width, output.height)))]);
    }
    if version < 3 {
        send(c, id, 2, vec![]);
    }
}

/// The bounds of a `wl_region` (None for a null one, or one with nothing in it).
fn region_bounds(c: &Client, region: u32) -> Option<Rect> {
    let Some(Data::Region(rects)) = c.objects.get(&region).map(|o| &o.data) else { return None };
    rects.iter().copied().reduce(|a, b| Rect::new(a.left.min(b.left), a.top.min(b.top), a.right.max(b.right), a.bottom.max(b.bottom)))
}

/// The desktop window a surface is shown in.
fn surface_window(c: &Client, surface: u32) -> Option<WindowId> {
    match c.objects.get(&surface).map(|o| &o.data) {
        Some(Data::Surface(s)) => s.window,
        _ => None,
    }
}

/// A cursor surface's picture as the desktop's cursor: its committed buffer's pixels (premultiplied, an opaque one
/// made so), one logical pixel to each of its buffer scale's, with its hotspot. None if it has no buffer, or isn't one.
fn cursor_of(c: &Client, surface: u32) -> Option<Cursor> {
    let Some(Data::Surface(s)) = c.objects.get(&surface).map(|o| &o.data) else { return None };
    let hotspot = s.cursor?;
    let Some(Data::Buffer(b)) = c.objects.get(&s.buffer?).map(|o| &o.data) else { return None };
    let step = s.scale.max(1);
    let (bw, bh) = b.pixels.size();
    let (w, h) = (bw / step, bh / step);
    if w <= 0 || h <= 0 {
        return None;
    }
    let opaque = b.pixels.format() == Format::Xrgb8888;
    let mut picture = Picture::new(w, h, 0);
    let mut row = vec![0u32; bw as usize];
    for y in 0..h {
        b.pixels.read_row(0, y * step, &mut row);
        for x in 0..w {
            let p = row[(x * step) as usize];
            picture.pixels[(y * w + x) as usize] = if opaque { p | 0xff00_0000 } else { p };
        }
    }
    Some(Cursor::Image { pixels: picture, hotspot })
}

/// The keymap every keyboard's sent: the desktop's, in a sealed memfd (a client maps it, privately, as v7 has it).
fn keymap_file(k: &Kernel) -> Option<(Arc<Open>, u32)> {
    let mut text = keymap::xkb_us().into_bytes();
    text.push(0);
    let mem = SharedMem::new(k.exec.clone(), text.len() as u64, 0);
    mem.write(0, &text).ok()?;
    mem.add_seals(F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE | F_SEAL_SEAL).ok()?;
    Some((Open::new(Node::Memfd(mem, false), 0, "/memfd:wayland-keymap (deleted)"), text.len() as u32))
}

/// A positioner that puts a window of `size` with its top left at `at` on its parent's content, and never moves it:
/// a subsurface's place.
/// `xdg_surface.configure`, its serial kept as the surface's last.
fn configure_xdg(c: &mut Client, surface: u32, xdg: Option<u32>, serial: u32) {
    send(c, xdg.unwrap_or(0), 0, vec![Arg::Uint(serial)]);
    if let Some(Data::Surface(s)) = c.objects.get_mut(&surface).map(|o| &mut o.data) {
        s.sent = serial;
    }
}

/// What's left of some rectangles with another taken out of them.
fn subtract_rect(parts: Vec<Rect>, cut: &Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    for p in parts {
        let i = p.intersect(cut);
        if i.is_empty() {
            out.push(p);
            continue;
        }
        for piece in [Rect::new(p.left, p.top, p.right, i.top), Rect::new(p.left, i.bottom, p.right, p.bottom), Rect::new(p.left, i.top, i.left, i.bottom), Rect::new(i.right, i.top, p.right, i.bottom)] {
            if !piece.is_empty() {
                out.push(piece);
            }
        }
    }
    out
}

/// Where a surface's window content is in it: past the shadow it draws (its window geometry's top left), or 0, 0.
fn offset_of(c: &Client, surface: u32) -> (i32, i32) {
    match c.objects.get(&surface).map(|o| &o.data) {
        Some(Data::Surface(s)) => s.offset,
        _ => (0, 0),
    }
}

fn at_point(at: (i32, i32), size: (i32, i32)) -> Positioner {
    Positioner { size, anchor_rect: Rect::new(at.0, at.1, at.0, at.1), anchor: Edge::TopLeft, gravity: Edge::BottomRight, offset: (0, 0), adjust: 0 }
}
