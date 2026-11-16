//! GLX 1.4, the vendor's half: the screen's configs as its X server has them (and choosing among them, as GLX's
//! rules say), contexts and GLX drawables made on the server too, making a context current on a window, and swaps.
//! Pixmaps and pbuffers are made on the server, but GL doesn't draw into them yet: making a context current on one
//! fails.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::ptr;

use gpu_wire::gl as wire;
use gpu_wire::records;

use crate::sync::Spin;
use crate::x::{self, VisualInfo, X};
use crate::{current, gl, ring, set_current};

// GLX's names.
const USE_GL: u32 = 1;
const BUFFER_SIZE: u32 = 2;
const LEVEL: u32 = 3;
const RGBA: u32 = 4;
const DOUBLEBUFFER: u32 = 5;
const STEREO: u32 = 6;
const AUX_BUFFERS: u32 = 7;
const RED_SIZE: u32 = 8;
const GREEN_SIZE: u32 = 9;
const BLUE_SIZE: u32 = 10;
const ALPHA_SIZE: u32 = 11;
const DEPTH_SIZE: u32 = 12;
const STENCIL_SIZE: u32 = 13;
const ACCUM_RED_SIZE: u32 = 14;
const ACCUM_ALPHA_SIZE: u32 = 17;
const CONFIG_CAVEAT: u32 = 0x20;
const X_VISUAL_TYPE: u32 = 0x22;
const TRANSPARENT_TYPE: u32 = 0x23;
const NONE: u32 = 0x8000;
const SLOW_CONFIG: u32 = 0x8001;
const VISUAL_ID: u32 = 0x800b;
const SCREEN: u32 = 0x800c;
const DRAWABLE_TYPE: u32 = 0x8010;
const RENDER_TYPE: u32 = 0x8011;
const X_RENDERABLE: u32 = 0x8012;
const FBCONFIG_ID: u32 = 0x8013;
const RGBA_TYPE: u32 = 0x8014;
const WIDTH: u32 = 0x801d;
const HEIGHT: u32 = 0x801e;
const EVENT_MASK: u32 = 0x801f;
const SAMPLE_BUFFERS: u32 = 100000;
const SAMPLES: u32 = 100001;
const FRAMEBUFFER_SRGB_CAPABLE: u32 = 0x20b2;
const DONT_CARE: u32 = 0xffff_ffff;
const RGBA_BIT: u32 = 1;
const WINDOW_BIT: u32 = 1;
const VENDOR: c_int = 1;
const VERSION: c_int = 2;
const EXTENSIONS: c_int = 3;
// glXGetConfig's and glXGetFBConfigAttrib's errors.
const BAD_ATTRIBUTE: c_int = 2;
const BAD_VISUAL: c_int = 4;
// GLX_ARB_create_context's.
const CONTEXT_MAJOR_VERSION: u32 = 0x2091;
const CONTEXT_MINOR_VERSION: u32 = 0x2092;
const CONTEXT_PROFILE_MASK: u32 = 0x9126;
const ES2_PROFILE_BIT: u32 = 4;
// The protocol's requests.
const CREATE_CONTEXT: u8 = 3;
const DESTROY_CONTEXT: u8 = 4;
const QUERY_VERSION: u8 = 7;
const CREATE_GLX_PIXMAP: u8 = 13;
const GET_VISUAL_CONFIGS: u8 = 14;
const DESTROY_GLX_PIXMAP: u8 = 15;
const QUERY_SERVER_STRING: u8 = 19;
const GET_FB_CONFIGS: u8 = 21;
const CREATE_PIXMAP: u8 = 22;
const DESTROY_PIXMAP: u8 = 23;
const CREATE_NEW_CONTEXT: u8 = 24;
const CREATE_PBUFFER: u8 = 27;
const DESTROY_PBUFFER: u8 = 28;
const CHANGE_DRAWABLE_ATTRIBUTES: u8 = 30;
const CREATE_WINDOW: u8 = 31;
const DESTROY_WINDOW: u8 = 32;
const CREATE_CONTEXT_ATTRIBS: u8 = 34;

const CLIENT_VENDOR: &[u8] = b"Carbon\0";
const CLIENT_VERSION: &[u8] = b"1.4\0";
/// The GLX extensions this library has.
const CLIENT_EXTENSIONS: &[u8] = b"GLX_ARB_create_context GLX_ARB_create_context_profile GLX_EXT_create_context_es2_profile GLX_EXT_create_context_es_profile GLX_ARB_framebuffer_sRGB GLX_EXT_framebuffer_sRGB GLX_ARB_get_proc_address GLX_EXT_visual_info GLX_EXT_visual_rating\0";

/// A config, as the server describes it: its attributes, by name. A `GLXFBConfig` is the address of one.
pub(crate) struct Config {
    attrs: Vec<(u32, u32)>,
}

impl Config {
    pub(crate) fn get(&self, name: u32) -> Option<u32> {
        self.attrs.iter().find(|a| a.0 == name).map(|a| a.1)
    }

    fn value(&self, name: u32) -> u32 {
        self.get(name).unwrap_or(0)
    }
}

/// A screen with GLX, as this library found it first: its framebuffer configs, its visuals' configs, and the
/// server's strings, all kept for the program's life (GLX's pointers to them stay good).
pub(crate) struct Screen {
    dpy: usize,
    number: c_int,
    configs: Vec<Config>,
    visuals: Vec<Config>,
    strings: Spin<Vec<(c_int, Vec<u8>)>>,
}

static SCREENS: Spin<Vec<&'static Screen>> = Spin::new(Vec::new());

/// A display's screen, asked of its server the first time.
pub(crate) fn screen(dpy: *mut c_void, number: c_int) -> Option<&'static Screen> {
    if let Some(s) = SCREENS.lock().iter().find(|s| s.dpy == dpy as usize && s.number == number) {
        return Some(s);
    }
    let x = X::get()?;
    let c = x.connection(dpy);
    if c.is_null() || !x.has_glx(c) {
        return None;
    }
    let version = x.glx(c, QUERY_VERSION, &[1, 4], true)?;
    if (x::word(&version, 8), x::word(&version, 12)) < (1, 3) {
        return None;
    }
    let fb = x.glx(c, GET_FB_CONFIGS, &[number as u32], true)?;
    let (n, pairs) = (x::word(&fb, 8) as usize, x::word(&fb, 12) as usize);
    let configs = (0..n)
        .map(|i| Config { attrs: (0..pairs).map(|j| (x::word(&fb, 32 + (i * pairs + j) * 8), x::word(&fb, 36 + (i * pairs + j) * 8))).collect() })
        .collect();
    let vis = x.glx(c, GET_VISUAL_CONFIGS, &[number as u32], true)?;
    let (n, props) = (x::word(&vis, 8) as usize, x::word(&vis, 12) as usize);
    let visuals = (0..n).filter(|_| props >= 18).map(|i| visual_config(&vis[32 + i * props * 4..32 + (i + 1) * props * 4])).collect();
    let s: &'static Screen = Box::leak(Box::new(Screen { dpy: dpy as usize, number, configs, visuals, strings: Spin::new(Vec::new()) }));
    SCREENS.lock().push(s);
    Some(s)
}

/// A visual's config as GetVisualConfigs sends it: 18 values, then names and values.
fn visual_config(b: &[u8]) -> Config {
    let v = |i: usize| x::word(b, i * 4);
    let visual_type = match v(1) {
        4 => 0x8002,
        5 => 0x8003,
        other => 0x8002 + other.min(5),
    };
    let mut attrs = alloc::vec![
        (VISUAL_ID, v(0)),
        (X_VISUAL_TYPE, visual_type),
        (RENDER_TYPE, if v(2) != 0 { RGBA_BIT } else { 2 }),
        (RED_SIZE, v(3)),
        (GREEN_SIZE, v(4)),
        (BLUE_SIZE, v(5)),
        (ALPHA_SIZE, v(6)),
        (ACCUM_RED_SIZE, v(7)),
        (ACCUM_RED_SIZE + 1, v(8)),
        (ACCUM_RED_SIZE + 2, v(9)),
        (ACCUM_ALPHA_SIZE, v(10)),
        (DOUBLEBUFFER, v(11)),
        (STEREO, v(12)),
        (BUFFER_SIZE, v(13)),
        (DEPTH_SIZE, v(14)),
        (STENCIL_SIZE, v(15)),
        (AUX_BUFFERS, v(16)),
        (LEVEL, v(17)),
    ];
    for i in (18..b.len() / 4).step_by(2) {
        if v(i) != 0 && !attrs.iter().any(|a| a.0 == v(i)) {
            attrs.push((v(i), v(i + 1)));
        }
    }
    Config { attrs }
}

impl Screen {
    fn config_of_visual(&self, visual: u64) -> Option<&Config> {
        self.visuals.iter().find(|c| u64::from(c.value(VISUAL_ID)) == visual)
    }

    fn string(&self, dpy: *mut c_void, name: c_int) -> *const u8 {
        let mut strings = self.strings.lock();
        if let Some((_, s)) = strings.iter().find(|s| s.0 == name) {
            return s.as_ptr();
        }
        let Some(x) = X::get() else { return ptr::null() };
        let Some(r) = x.glx(x.connection(dpy), QUERY_SERVER_STRING, &[self.number as u32, name as u32], true) else { return ptr::null() };
        let n = x::word(&r, 12) as usize;
        let mut s: Vec<u8> = r.get(32..32 + n).unwrap_or_default().to_vec();
        if s.last() != Some(&0) {
            s.push(0);
        }
        // Kept, and never moved: the program holds the pointer.
        strings.push((name, s));
        strings.last().map_or(ptr::null(), |s| s.1.as_ptr())
    }
}

/// The best config for an attribute list, as GLX 1.3 matches and sorts them: `fbconfig` for `glXChooseFBConfig`'s
/// rules and list (names and values), else `glXChooseVisual`'s (booleans without values, single-buffered and RGBA only
/// when asked).
fn choose(configs: &[Config], list: *const c_int, fbconfig: bool) -> Vec<&Config> {
    // What's asked, by name: GLX's defaults first.
    let mut want: Vec<(u32, u32)> = if fbconfig {
        alloc::vec![(DOUBLEBUFFER, DONT_CARE), (RENDER_TYPE, RGBA_BIT), (DRAWABLE_TYPE, WINDOW_BIT), (X_RENDERABLE, DONT_CARE), (CONFIG_CAVEAT, DONT_CARE), (X_VISUAL_TYPE, DONT_CARE), (TRANSPARENT_TYPE, NONE), (FBCONFIG_ID, DONT_CARE)]
    } else {
        alloc::vec![(DOUBLEBUFFER, 0), (RENDER_TYPE, 2), (DRAWABLE_TYPE, WINDOW_BIT), (TRANSPARENT_TYPE, DONT_CARE)]
    };
    let mut set = |name: u32, value: u32| {
        want.retain(|w| w.0 != name);
        want.push((name, value));
    };
    if !list.is_null() {
        let mut i = 0;
        loop {
            let name = unsafe { *list.add(i) } as u32;
            if name == 0 {
                break;
            }
            match (fbconfig, name) {
                (false, RGBA) => set(RENDER_TYPE, RGBA_BIT),
                (false, DOUBLEBUFFER | STEREO) => set(name, 1),
                (false, USE_GL) => {}
                (true, RGBA) => {
                    i += 1;
                    set(RENDER_TYPE, RGBA_BIT);
                }
                _ => {
                    i += 1;
                    set(name, unsafe { *list.add(i) } as u32);
                }
            }
            i += 1;
        }
    }
    let asked = |name: u32| want.iter().find(|w| w.0 == name).map(|w| w.1);
    if let Some(id) = asked(FBCONFIG_ID).filter(|&v| v != DONT_CARE) {
        return configs.iter().filter(|c| c.value(FBCONFIG_ID) == id).collect();
    }
    let fits = |c: &Config| {
        want.iter().all(|&(name, w)| {
            if w == DONT_CARE {
                return true;
            }
            let have = c.value(name);
            match name {
                RENDER_TYPE | DRAWABLE_TYPE => have & w == w,
                // Asking for sRGB requires it; not asking takes either.
                BUFFER_SIZE | AUX_BUFFERS | RED_SIZE..=ACCUM_ALPHA_SIZE | SAMPLE_BUFFERS | SAMPLES | FRAMEBUFFER_SRGB_CAPABLE => have >= w,
                LEVEL | DOUBLEBUFFER | STEREO | X_VISUAL_TYPE | CONFIG_CAVEAT | TRANSPARENT_TYPE | X_RENDERABLE => have == w,
                _ => true,
            }
        })
    };
    let mut out: Vec<&Config> = configs.iter().filter(|c| fits(c)).collect();
    // The colour bits asked for, the more the better; then the smaller buffers, single before double, the least
    // multisampling, the deepest depth, the least stencil.
    let colour = |c: &Config| -> u32 { [RED_SIZE, GREEN_SIZE, BLUE_SIZE, ALPHA_SIZE].iter().filter(|&&n| asked(n).is_some_and(|v| v != 0 && v != DONT_CARE)).map(|&n| c.value(n)).sum() };
    let caveat = |c: &Config| match c.value(CONFIG_CAVEAT) {
        NONE | 0 => 0,
        SLOW_CONFIG => 1,
        _ => 2,
    };
    out.sort_by_key(|c| (caveat(c), u32::MAX - colour(c), c.value(BUFFER_SIZE), c.value(DOUBLEBUFFER), c.value(AUX_BUFFERS), c.value(SAMPLE_BUFFERS), c.value(SAMPLES), u32::MAX - c.value(DEPTH_SIZE), c.value(STENCIL_SIZE), c.value(FBCONFIG_ID)));
    out
}

/// What a program's drawable is, to this library.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Window,
    Pixmap,
    Pbuffer,
}

/// A GLX drawable this library made: its X drawable and config.
#[derive(Clone, Copy)]
struct Drawable {
    glx: u32,
    x: u32,
    kind: Kind,
    config: usize,
    event_mask: u32,
}

static DRAWABLES: Spin<Vec<Drawable>> = Spin::new(Vec::new());

/// A window frames are put in: its GC and the frame read back last.
struct Window {
    x: u32,
    connection: usize,
    gc: u32,
    width: u32,
    height: u32,
    depth: u8,
    pixels: Vec<u8>,
}

static WINDOWS: Spin<Vec<Window>> = Spin::new(Vec::new());

/// A GL context: its server ID, its host ID (its address), its config, the records it hasn't sent, and where it's
/// current.
pub(crate) struct Context {
    xid: u32,
    dpy: *mut c_void,
    connection: *mut c_void,
    screen: c_int,
    config: *const Config,
    render_type: u32,
    /// An OpenGL ES context (`GLX_CONTEXT_ES2_PROFILE_BIT_EXT`): ES 3.0, what a request for 2.0 or 3.0 gets.
    pub(crate) es: bool,
    records: Vec<u8>,
    /// `glPixelStore`'s state.
    pub(crate) store: gl::Store,
    /// `glClientActiveTexture`'s unit, which `glTexCoordPointer` sets the array of.
    pub(crate) client_unit: u32,
    /// Buffers mapped (`glMapBufferRange`), each by the target it was mapped at.
    pub(crate) maps: Vec<crate::objects::Map>,
    drawable: u32,
    /// The X window it draws into, and its size as last bound.
    window: u32,
    width: u32,
    height: u32,
    /// Current on a thread; destroyed while it was, and to be freed when it isn't.
    current: bool,
    destroyed: bool,
}

impl Context {
    fn id(&self) -> u64 {
        self as *const Context as u64
    }

    /// Adds a GL call's record to those to send.
    pub(crate) fn record(&mut self, kind: u32, payload: &[u8]) {
        records::record(&mut self.records, kind, payload);
    }

    /// Sends what's recorded (and `then`), for this context, and waits: the answers.
    pub(crate) fn sync(&mut self, then: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(16 + self.records.len() + then.len());
        records::record(&mut out, wire::USE, &self.id().to_le_bytes());
        out.append(&mut self.records);
        out.extend_from_slice(then);
        ring(&out)
    }

    /// Binds it to its window, at the window's size, as a BIND record.
    fn bind(&mut self, config: &Config) {
        let mut p = Vec::new();
        records::put_u64(&mut p, u64::from(self.window));
        for w in [self.width, self.height, config.value(DEPTH_SIZE), config.value(STENCIL_SIZE), u32::from(config.value(ALPHA_SIZE) > 0)] {
            records::put_u32(&mut p, w);
        }
        self.record(wire::BIND, &p);
    }
}

fn new_context(dpy: *mut c_void, screen: c_int, config: *const Config, render_type: u32, es: bool, share: *mut Context, request: impl FnOnce(&X, *mut c_void, u32)) -> *mut Context {
    let Some(x) = X::get() else { return ptr::null_mut() };
    let connection = x.connection(dpy);
    let xid = unsafe { (x.generate_id)(connection) };
    request(&x, connection, xid);
    let c = Box::into_raw(Box::new(Context { xid, dpy, connection, screen, config, render_type, es, records: Vec::new(), store: gl::Store::default(), client_unit: 0, maps: Vec::new(), drawable: 0, window: 0, width: 0, height: 0, current: false, destroyed: false }));
    let mut made = Vec::new();
    records::put_u64(&mut made, unsafe { (*c).id() });
    records::put_u64(&mut made, if share.is_null() { 0 } else { unsafe { (*share).id() } });
    let mut r = Vec::new();
    records::record(&mut r, wire::CONTEXT, &made);
    if ring(&r).is_none() {
        unsafe { drop(Box::from_raw(c)) };
        return ptr::null_mut();
    }
    c
}

fn free_context(c: *mut Context) {
    let mut r = Vec::new();
    records::record(&mut r, wire::FREE_CONTEXT, &unsafe { (*c).id() }.to_le_bytes());
    ring(&r);
    unsafe { drop(Box::from_raw(c)) };
}

/// A list the program frees with `XFree` (the C library's `free`).
fn c_list<T: Copy>(items: &[T], count: *mut c_int) -> *mut T {
    if !count.is_null() {
        unsafe { *count = items.len() as c_int };
    }
    if items.is_empty() {
        return ptr::null_mut();
    }
    let p = unsafe { crate::malloc_bytes(core::mem::size_of_val(items)) } as *mut T;
    if !p.is_null() {
        unsafe { ptr::copy_nonoverlapping(items.as_ptr(), p, items.len()) };
    }
    p
}

/// An `XVisualInfo` for a visual, from the program's Xlib (so `XFree` frees it).
fn visual_info(dpy: *mut c_void, screen: c_int, visual: u32) -> *mut VisualInfo {
    let Some(x) = X::get() else { return ptr::null_mut() };
    let mut template: VisualInfo = unsafe { core::mem::zeroed() };
    template.visualid = u64::from(visual);
    template.screen = screen;
    let mut n = 0;
    unsafe { (x.visual_info)(dpy, x::VISUAL_ID_MASK | x::VISUAL_SCREEN_MASK, &mut template, &mut n) }
}

// GLX's functions.

unsafe extern "C" fn glXChooseVisual(dpy: *mut c_void, screen: c_int, list: *const c_int) -> *mut VisualInfo {
    let Some(s) = self::screen(dpy, screen) else { return ptr::null_mut() };
    match choose(&s.visuals, list, false).first() {
        Some(c) => visual_info(dpy, screen, c.value(VISUAL_ID)),
        None => ptr::null_mut(),
    }
}

unsafe extern "C" fn glXChooseFBConfig(dpy: *mut c_void, screen: c_int, list: *const c_int, count: *mut c_int) -> *mut *const Config {
    let Some(s) = self::screen(dpy, screen) else { return c_list::<*const Config>(&[], count) };
    let chosen: Vec<*const Config> = choose(&s.configs, list, true).into_iter().map(|c| c as *const Config).collect();
    c_list(&chosen, count)
}

unsafe extern "C" fn glXGetFBConfigs(dpy: *mut c_void, screen: c_int, count: *mut c_int) -> *mut *const Config {
    let Some(s) = self::screen(dpy, screen) else { return c_list::<*const Config>(&[], count) };
    let all: Vec<*const Config> = s.configs.iter().map(|c| c as *const Config).collect();
    c_list(&all, count)
}

unsafe extern "C" fn glXGetFBConfigAttrib(_dpy: *mut c_void, config: *const Config, attribute: c_int, value: *mut c_int) -> c_int {
    let Some(c) = (unsafe { config.as_ref() }) else { return BAD_ATTRIBUTE };
    let v = match attribute as u32 {
        SCREEN => 0,
        // What a config has that the server didn't name is 0, as for any of GLX's names; others aren't GLX's.
        name if c.get(name).is_some() => c.value(name),
        1..=17 | 0x20..=0x28 | 0x8000..=0x8030 | SAMPLE_BUFFERS | SAMPLES | FRAMEBUFFER_SRGB_CAPABLE => 0,
        _ => return BAD_ATTRIBUTE,
    };
    unsafe { *value = v as c_int };
    0
}

unsafe extern "C" fn glXGetVisualFromFBConfig(dpy: *mut c_void, config: *const Config) -> *mut VisualInfo {
    let Some(c) = (unsafe { config.as_ref() }) else { return ptr::null_mut() };
    match c.value(VISUAL_ID) {
        0 => ptr::null_mut(),
        v => visual_info(dpy, screen_of(dpy, c), v),
    }
}

/// The screen a config is of.
fn screen_of(dpy: *mut c_void, config: &Config) -> c_int {
    SCREENS.lock().iter().find(|s| s.dpy == dpy as usize && s.configs.iter().chain(&s.visuals).any(|c| ptr::eq(c, config))).map_or(0, |s| s.number)
}

unsafe extern "C" fn glXGetConfig(dpy: *mut c_void, vis: *const VisualInfo, attribute: c_int, value: *mut c_int) -> c_int {
    let Some(vis) = (unsafe { vis.as_ref() }) else { return BAD_VISUAL };
    let Some(s) = screen(dpy, vis.screen) else { return BAD_VISUAL };
    let Some(c) = s.config_of_visual(vis.visualid) else { return BAD_VISUAL };
    let v = match attribute as u32 {
        USE_GL => 1,
        RGBA => u32::from(c.value(RENDER_TYPE) & RGBA_BIT != 0),
        name if c.get(name).is_some() => c.value(name),
        1..=17 | 0x20..=0x28 | 0x8000..=0x8030 | SAMPLE_BUFFERS | SAMPLES | FRAMEBUFFER_SRGB_CAPABLE => 0,
        _ => return BAD_ATTRIBUTE,
    };
    unsafe { *value = v as c_int };
    0
}

unsafe extern "C" fn glXCreateContext(dpy: *mut c_void, vis: *const VisualInfo, share: *mut Context, _direct: c_int) -> *mut Context {
    let Some(vis) = (unsafe { vis.as_ref() }) else { return ptr::null_mut() };
    let Some(s) = screen(dpy, vis.screen) else { return ptr::null_mut() };
    let Some(config) = s.config_of_visual(vis.visualid) else { return ptr::null_mut() };
    let share_xid = unsafe { share.as_ref() }.map_or(0, |c| c.xid);
    let (visual, number) = (vis.visualid as u32, vis.screen as u32);
    new_context(dpy, vis.screen, config, RGBA_TYPE, false, share, |x, c, xid| {
        x.glx(c, CREATE_CONTEXT, &[xid, visual, number, share_xid, 1], false);
    })
}

unsafe extern "C" fn glXCreateNewContext(dpy: *mut c_void, config: *const Config, render_type: c_int, share: *mut Context, _direct: c_int) -> *mut Context {
    let Some(c) = (unsafe { config.as_ref() }) else { return ptr::null_mut() };
    let screen = screen_of(dpy, c);
    let share_xid = unsafe { share.as_ref() }.map_or(0, |c| c.xid);
    let id = c.value(FBCONFIG_ID);
    new_context(dpy, screen, config, render_type as u32, false, share, |x, conn, xid| {
        x.glx(conn, CREATE_NEW_CONTEXT, &[xid, id, screen as u32, render_type as u32, share_xid, 1], false);
    })
}

unsafe extern "C" fn glXCreateContextAttribsARB(dpy: *mut c_void, config: *const Config, share: *mut Context, _direct: c_int, list: *const c_int) -> *mut Context {
    let mut attrs: Vec<u32> = Vec::new();
    let (mut major, mut minor, mut profile, mut render_type) = (1, 0, 0, RGBA_TYPE);
    if !list.is_null() {
        let mut i = 0;
        while unsafe { *list.add(i) } != 0 {
            let (name, value) = unsafe { (*list.add(i) as u32, *list.add(i + 1) as u32) };
            match name {
                CONTEXT_MAJOR_VERSION => major = value,
                CONTEXT_MINOR_VERSION => minor = value,
                CONTEXT_PROFILE_MASK => profile = value,
                RENDER_TYPE => render_type = value,
                _ => {}
            }
            attrs.extend([name, value]);
            i += 2;
        }
    }
    // Only the GL this library has made so far: desktop GL up to its version, OpenGL ES up to 3.0.
    let es = profile & ES2_PROFILE_BIT != 0;
    if (es && (major, minor) > gl::ES_VERSION) || (!es && (major, minor) > gl::VERSION) {
        return ptr::null_mut();
    }
    let config_ref = unsafe { config.as_ref() };
    let screen = config_ref.map_or(0, |c| screen_of(dpy, c));
    let id = config_ref.map_or(0, |c| c.value(FBCONFIG_ID));
    let share_xid = unsafe { share.as_ref() }.map_or(0, |c| c.xid);
    new_context(dpy, screen, config, render_type, es, share, |x, conn, xid| {
        let mut fields = alloc::vec![xid, id, screen as u32, share_xid, 1, (attrs.len() / 2) as u32];
        fields.extend(&attrs);
        x.glx(conn, CREATE_CONTEXT_ATTRIBS, &fields, false);
    })
}

unsafe extern "C" fn glXDestroyContext(dpy: *mut c_void, ctx: *mut Context) {
    let Some(c) = (unsafe { ctx.as_mut() }) else { return };
    if let Some(x) = X::get() {
        x.glx(x.connection(dpy), DESTROY_CONTEXT, &[c.xid], false);
    }
    if c.current {
        c.destroyed = true;
    } else {
        free_context(ctx);
    }
}

unsafe extern "C" fn glXIsDirect(_dpy: *mut c_void, _ctx: *mut Context) -> c_int {
    1
}

unsafe extern "C" fn glXCopyContext(_dpy: *mut c_void, _from: *mut Context, _to: *mut Context, _mask: u64) {}

unsafe extern "C" fn glXQueryContext(_dpy: *mut c_void, ctx: *mut Context, attribute: c_int, value: *mut c_int) -> c_int {
    let Some(c) = (unsafe { ctx.as_ref() }) else { return 5 };
    let config = unsafe { c.config.as_ref() };
    let v = match attribute as u32 {
        FBCONFIG_ID => config.map_or(0, |c| c.value(FBCONFIG_ID)),
        RENDER_TYPE => c.render_type,
        SCREEN => c.screen as u32,
        _ => return BAD_ATTRIBUTE,
    };
    unsafe { *value = v as c_int };
    0
}

unsafe extern "C" fn glXMakeCurrent(dpy: *mut c_void, drawable: u64, ctx: *mut Context) -> c_int {
    unsafe { glXMakeContextCurrent(dpy, drawable, drawable, ctx) }
}

unsafe extern "C" fn glXMakeContextCurrent(dpy: *mut c_void, draw: u64, _read: u64, ctx: *mut Context) -> c_int {
    let old = current();
    let draw = draw as u32;
    if !old.is_null() && old == ctx && unsafe { (*old).drawable } == draw {
        return 1;
    }
    // The context that was current: its calls sent, and freed if it was destroyed meanwhile.
    if let Some(o) = unsafe { old.as_mut() } {
        if !o.records.is_empty() {
            o.sync(&[]);
        }
        o.current = false;
        o.drawable = 0;
        set_current(ptr::null_mut());
        if o.destroyed {
            free_context(old);
        }
    }
    let Some(c) = (unsafe { ctx.as_mut() }) else { return 1 };
    if c.current {
        return 0;
    }
    // The X window behind the drawable, and the config it has.
    let made = DRAWABLES.lock().iter().find(|d| d.glx == draw).copied();
    let (window, drawable_config) = match made {
        Some(d) if d.kind != Kind::Window => return 0,
        Some(d) => (d.x, d.config as *const Config),
        None => (draw, ptr::null()),
    };
    let Some(x) = X::get() else { return 0 };
    let Some((width, height, depth)) = x.geometry(c.connection, window) else { return 0 };
    let config = unsafe { c.config.as_ref().or(drawable_config.as_ref()) }.or_else(|| {
        // A context with no config, on a window with none: the screen's first visual of the window's depth.
        let s = screen(dpy, c.screen)?;
        s.visuals.iter().find(|v| v.value(BUFFER_SIZE) == u32::from(depth))
    });
    let Some(config) = config else { return 0 };
    c.drawable = draw;
    c.window = window;
    c.width = width;
    c.height = height;
    c.bind(config);
    if c.sync(&[]).is_none() {
        return 0;
    }
    let mut windows = WINDOWS.lock();
    match windows.iter_mut().find(|w| w.x == window) {
        Some(w) => {
            w.width = width;
            w.height = height;
            w.depth = depth;
        }
        None => windows.push(Window { x: window, connection: c.connection as usize, gc: x.gc(c.connection, window), width, height, depth, pixels: Vec::new() }),
    }
    c.current = true;
    set_current(ctx);
    1
}

unsafe extern "C" fn glXSwapBuffers(_dpy: *mut c_void, drawable: u64) {
    let drawable = drawable as u32;
    let window = DRAWABLES.lock().iter().find(|d| d.glx == drawable).map_or(Some(drawable), |d| (d.kind == Kind::Window).then_some(d.x));
    let Some(window) = window else { return };
    let Some(x) = X::get() else { return };
    let mut windows = WINDOWS.lock();
    let Some(w) = windows.iter_mut().find(|w| w.x == window) else { return };
    w.pixels.resize(w.width as usize * w.height as usize * 4, 0);
    let mut swap = Vec::new();
    records::put_u64(&mut swap, u64::from(window));
    records::put_u64(&mut swap, w.pixels.as_mut_ptr() as u64);
    records::put_u32(&mut swap, w.width * 4);
    let mut then = Vec::new();
    records::record(&mut then, wire::SWAP, &swap);
    // The current context's calls first, if it draws here; GLX swaps a drawable whoever's current.
    let c = unsafe { current().as_mut() }.filter(|c| c.window == window);
    let done = match c {
        Some(c) => c.sync(&then),
        None => ring(&then),
    };
    if done.is_none() {
        return;
    }
    let connection = w.connection as *mut c_void;
    x.put(connection, window, w.gc, w.depth, w.width, w.height, &w.pixels);
    // A window resized since: the next frame is drawn at its new size.
    if let (Some((width, height, depth)), Some(c)) = (x.geometry(connection, window), unsafe { current().as_mut() }.filter(|c| c.window == window))
        && (width, height) != (w.width, w.height)
    {
        (w.width, w.height, w.depth) = (width, height, depth);
        (c.width, c.height) = (width, height);
        let config = unsafe { c.config.as_ref() }.or_else(|| screen(c.dpy, c.screen).and_then(|s| s.visuals.first()));
        if let Some(config) = config {
            c.bind(config);
        }
    }
}

unsafe extern "C" fn glXWaitGL() {
    if let Some(c) = unsafe { current().as_mut() } {
        c.sync(&[]);
    }
}

unsafe extern "C" fn glXWaitX() {}

unsafe extern "C" fn glXUseXFont(_font: u64, _first: c_int, _count: c_int, _list: c_int) {}

unsafe extern "C" fn glXCreateGLXPixmap(dpy: *mut c_void, vis: *const VisualInfo, pixmap: u64) -> u64 {
    let Some(vis) = (unsafe { vis.as_ref() }) else { return 0 };
    let config = screen(dpy, vis.screen).and_then(|s| s.config_of_visual(vis.visualid)).map_or(0, |c| c as *const Config as usize);
    let (visual, number) = (vis.visualid as u32, vis.screen as u32);
    made(dpy, pixmap as u32, Kind::Pixmap, config, |x, c, id| {
        x.glx(c, CREATE_GLX_PIXMAP, &[number, visual, pixmap as u32, id], false);
    })
}

unsafe extern "C" fn glXDestroyGLXPixmap(dpy: *mut c_void, pixmap: u64) {
    unmade(dpy, pixmap as u32, DESTROY_GLX_PIXMAP);
}

unsafe extern "C" fn glXCreatePixmap(dpy: *mut c_void, config: *const Config, pixmap: u64, _list: *const c_int) -> u64 {
    let Some(c) = (unsafe { config.as_ref() }) else { return 0 };
    let (screen, id) = (screen_of(dpy, c) as u32, c.value(FBCONFIG_ID));
    made(dpy, pixmap as u32, Kind::Pixmap, config as usize, |x, conn, glx| {
        x.glx(conn, CREATE_PIXMAP, &[screen, id, pixmap as u32, glx, 0], false);
    })
}

unsafe extern "C" fn glXDestroyPixmap(dpy: *mut c_void, pixmap: u64) {
    unmade(dpy, pixmap as u32, DESTROY_PIXMAP);
}

unsafe extern "C" fn glXCreateWindow(dpy: *mut c_void, config: *const Config, window: u64, _list: *const c_int) -> u64 {
    let Some(c) = (unsafe { config.as_ref() }) else { return 0 };
    let (screen, id) = (screen_of(dpy, c) as u32, c.value(FBCONFIG_ID));
    made(dpy, window as u32, Kind::Window, config as usize, |x, conn, glx| {
        x.glx(conn, CREATE_WINDOW, &[screen, id, window as u32, glx, 0], false);
    })
}

unsafe extern "C" fn glXDestroyWindow(dpy: *mut c_void, window: u64) {
    unmade(dpy, window as u32, DESTROY_WINDOW);
}

unsafe extern "C" fn glXCreatePbuffer(dpy: *mut c_void, config: *const Config, list: *const c_int) -> u64 {
    let Some(c) = (unsafe { config.as_ref() }) else { return 0 };
    let (screen, id) = (screen_of(dpy, c) as u32, c.value(FBCONFIG_ID));
    let mut attrs: Vec<u32> = Vec::new();
    if !list.is_null() {
        let mut i = 0;
        while unsafe { *list.add(i) } != 0 {
            attrs.extend(unsafe { [*list.add(i) as u32, *list.add(i + 1) as u32] });
            i += 2;
        }
    }
    made(dpy, 0, Kind::Pbuffer, config as usize, |x, conn, glx| {
        let mut fields = alloc::vec![screen, id, glx, (attrs.len() / 2) as u32];
        fields.extend(&attrs);
        x.glx(conn, CREATE_PBUFFER, &fields, false);
    })
}

unsafe extern "C" fn glXDestroyPbuffer(dpy: *mut c_void, pbuffer: u64) {
    unmade(dpy, pbuffer as u32, DESTROY_PBUFFER);
}

/// A GLX drawable made on the server, and kept: its ID.
fn made(dpy: *mut c_void, x_drawable: u32, kind: Kind, config: usize, request: impl FnOnce(&X, *mut c_void, u32)) -> u64 {
    let Some(x) = X::get() else { return 0 };
    let c = x.connection(dpy);
    let glx = unsafe { (x.generate_id)(c) };
    request(&x, c, glx);
    DRAWABLES.lock().push(Drawable { glx, x: x_drawable, kind, config, event_mask: 0 });
    u64::from(glx)
}

fn unmade(dpy: *mut c_void, glx: u32, request: u8) {
    if let Some(x) = X::get() {
        x.glx(x.connection(dpy), request, &[glx], false);
    }
    let gone = {
        let mut drawables = DRAWABLES.lock();
        let gone = drawables.iter().find(|d| d.glx == glx).copied();
        drawables.retain(|d| d.glx != glx);
        gone
    };
    // Its window's framebuffer too, unless another context is drawing there.
    if let Some(d) = gone.filter(|d| d.kind == Kind::Window) {
        WINDOWS.lock().retain(|w| w.x != d.x);
        let mut r = Vec::new();
        records::record(&mut r, wire::FREE_DRAWABLE, &u64::from(d.x).to_le_bytes());
        ring(&r);
    }
}

unsafe extern "C" fn glXQueryDrawable(dpy: *mut c_void, drawable: u64, attribute: c_int, value: *mut u32) {
    let made = DRAWABLES.lock().iter().find(|d| d.glx == drawable as u32).copied();
    let x_drawable = made.map_or(drawable as u32, |d| d.x);
    let v = match attribute as u32 {
        WIDTH | HEIGHT => {
            let size = X::get().and_then(|x| x.geometry(x.connection(dpy), x_drawable));
            size.map_or(0, |(w, h, _)| if attribute as u32 == WIDTH { w } else { h })
        }
        FBCONFIG_ID => made.and_then(|d| unsafe { (d.config as *const Config).as_ref() }).map_or(0, |c| c.value(FBCONFIG_ID)),
        EVENT_MASK => made.map_or(0, |d| d.event_mask),
        _ => 0,
    };
    if !value.is_null() {
        unsafe { *value = v };
    }
}

unsafe extern "C" fn glXSelectEvent(dpy: *mut c_void, drawable: u64, mask: u64) {
    if let Some(d) = DRAWABLES.lock().iter_mut().find(|d| d.glx == drawable as u32) {
        d.event_mask = mask as u32;
    }
    if let Some(x) = X::get() {
        x.glx(x.connection(dpy), CHANGE_DRAWABLE_ATTRIBUTES, &[drawable as u32, 1, EVENT_MASK, mask as u32], false);
    }
}

unsafe extern "C" fn glXGetSelectedEvent(_dpy: *mut c_void, drawable: u64, mask: *mut u64) {
    let m = DRAWABLES.lock().iter().find(|d| d.glx == drawable as u32).map_or(0, |d| d.event_mask);
    if !mask.is_null() {
        unsafe { *mask = u64::from(m) };
    }
}

unsafe extern "C" fn glXQueryServerString(dpy: *mut c_void, screen: c_int, name: c_int) -> *const u8 {
    match self::screen(dpy, screen) {
        Some(s) if (VENDOR..=EXTENSIONS).contains(&name) => s.string(dpy, name),
        _ => ptr::null(),
    }
}

unsafe extern "C" fn glXGetClientString(_dpy: *mut c_void, name: c_int) -> *const u8 {
    match name {
        VENDOR => CLIENT_VENDOR.as_ptr(),
        VERSION => CLIENT_VERSION.as_ptr(),
        EXTENSIONS => CLIENT_EXTENSIONS.as_ptr(),
        _ => ptr::null(),
    }
}

unsafe extern "C" fn glXQueryExtensionsString(_dpy: *mut c_void, _screen: c_int) -> *const u8 {
    CLIENT_EXTENSIONS.as_ptr()
}

macro_rules! entry {
    ($f:ident) => {
        (stringify!($f).as_bytes(), $f as *const c_void)
    };
}

/// GLX's functions, by name.
pub(crate) fn lookup(name: &[u8]) -> Option<*const c_void> {
    let table: &[(&[u8], *const c_void)] = &[
        entry!(glXChooseVisual),
        entry!(glXCopyContext),
        entry!(glXCreateContext),
        entry!(glXCreateGLXPixmap),
        entry!(glXDestroyContext),
        entry!(glXDestroyGLXPixmap),
        entry!(glXGetConfig),
        entry!(glXIsDirect),
        entry!(glXMakeCurrent),
        entry!(glXSwapBuffers),
        entry!(glXUseXFont),
        entry!(glXWaitGL),
        entry!(glXWaitX),
        entry!(glXQueryServerString),
        entry!(glXGetClientString),
        entry!(glXQueryExtensionsString),
        entry!(glXChooseFBConfig),
        entry!(glXCreateNewContext),
        entry!(glXCreatePbuffer),
        entry!(glXCreatePixmap),
        entry!(glXCreateWindow),
        entry!(glXDestroyPbuffer),
        entry!(glXDestroyPixmap),
        entry!(glXDestroyWindow),
        entry!(glXGetFBConfigAttrib),
        entry!(glXGetFBConfigs),
        entry!(glXGetSelectedEvent),
        entry!(glXGetVisualFromFBConfig),
        entry!(glXMakeContextCurrent),
        entry!(glXQueryContext),
        entry!(glXQueryDrawable),
        entry!(glXSelectEvent),
        entry!(glXCreateContextAttribsARB),
    ];
    table.iter().find(|e| e.0 == name).map(|e| e.1)
}
