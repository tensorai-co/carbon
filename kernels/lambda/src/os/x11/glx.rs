//! GLX (1.4), the X server's half: what a client's GLX library asks the server before it renders on its own. The
//! screen's configs (one for each visual, and framebuffer configs beside them), the server's strings (among them the
//! GLVND vendor that renders for this screen), contexts, and GLX windows, pixmaps and pbuffers kept as names for X
//! drawables. Rendering is always direct: the client's driver draws and presents itself, so indirect contexts are
//! refused (BadValue, as an X server without `+iglx` does), and Render, MakeCurrent and SwapBuffers never come.

use super::wire::{Reply, Req, err};
use super::{Res, Server, VISUALS};

pub const GLX: u8 = 136;
/// GLX's 17 events (`GLX_PbufferClobber`, `GLX_BufferSwapComplete`, and room for the rest), from here.
pub const GLX_EVENT: u8 = 71;
/// GLX's 14 errors, from here.
pub const GLX_ERROR: u8 = 140;
const BAD_CONTEXT: u8 = GLX_ERROR;
const BAD_DRAWABLE: u8 = GLX_ERROR + 2;
const BAD_PIXMAP: u8 = GLX_ERROR + 3;
const BAD_CONTEXT_TAG: u8 = GLX_ERROR + 4;
const UNSUPPORTED_PRIVATE_REQUEST: u8 = GLX_ERROR + 8;
const BAD_FBCONFIG: u8 = GLX_ERROR + 9;
const BAD_PBUFFER: u8 = GLX_ERROR + 10;
const BAD_WINDOW: u8 = GLX_ERROR + 12;

/// The GLVND vendor library clients load to render on this screen (`libGLX_<name>.so.0`): the image's Mesa until
/// Carbon's own GL is there to name instead.
const VENDOR_NAMES: &str = "mesa";
const VENDOR: &str = "Carbon";
const VERSION: &str = "1.4";
/// What the server does for GLX: the extensions whose requests it answers, and those that are all the client's.
const EXTENSIONS: &str = "GLX_ARB_context_flush_control GLX_ARB_create_context GLX_ARB_create_context_no_error \
    GLX_ARB_create_context_profile GLX_ARB_framebuffer_sRGB GLX_ARB_multisample GLX_EXT_create_context_es2_profile \
    GLX_EXT_create_context_es_profile GLX_EXT_framebuffer_sRGB GLX_EXT_import_context GLX_EXT_libglvnd \
    GLX_EXT_no_config_context GLX_EXT_visual_info GLX_EXT_visual_rating GLX_SGIS_multisample GLX_SGIX_fbconfig \
    GLX_SGIX_pbuffer GLX_SGI_make_current_read";

// GLX's attribute names and values.
const USE_GL: u32 = 1;
const BUFFER_SIZE: u32 = 2;
const LEVEL: u32 = 3;
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
const ACCUM_GREEN_SIZE: u32 = 15;
const ACCUM_BLUE_SIZE: u32 = 16;
const ACCUM_ALPHA_SIZE: u32 = 17;
const CONFIG_CAVEAT: u32 = 0x20;
const X_VISUAL_TYPE: u32 = 0x22;
const TRANSPARENT_TYPE: u32 = 0x23;
const NONE: u32 = 0x8000;
const TRUE_COLOR: u32 = 0x8002;
const SHARE_CONTEXT: u32 = 0x800a;
const VISUAL_ID: u32 = 0x800b;
const SCREEN: u32 = 0x800c;
const DRAWABLE_TYPE: u32 = 0x8010;
const RENDER_TYPE: u32 = 0x8011;
const X_RENDERABLE: u32 = 0x8012;
const FBCONFIG_ID: u32 = 0x8013;
const MAX_PBUFFER_WIDTH: u32 = 0x8016;
const MAX_PBUFFER_HEIGHT: u32 = 0x8017;
const MAX_PBUFFER_PIXELS: u32 = 0x8018;
const PRESERVED_CONTENTS: u32 = 0x801b;
const LARGEST_PBUFFER: u32 = 0x801c;
const WIDTH: u32 = 0x801d;
const HEIGHT: u32 = 0x801e;
const EVENT_MASK: u32 = 0x801f;
const PBUFFER_HEIGHT: u32 = 0x8040;
const PBUFFER_WIDTH: u32 = 0x8041;
const SAMPLE_BUFFERS: u32 = 100000;
const SAMPLES: u32 = 100001;
const FRAMEBUFFER_SRGB_CAPABLE: u32 = 0x20b2;
const Y_INVERTED: u32 = 0x20d4;
const RGBA_BIT: u32 = 1;
/// A context's render type, as GLX_RENDER_TYPE names it for contexts (configs have bits instead).
const RGBA_TYPE: u32 = 0x8014;
const WINDOW_BIT: u32 = 1;
const PIXMAP_BIT: u32 = 2;
const PBUFFER_BIT: u32 = 4;
/// The largest pbuffer: the largest image the desktop composites.
const MAX_PBUFFER: u32 = 8192;

/// One of the screen's configs: 8 bits for each of red, green and blue, alpha with a 32-bit visual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub id: u32,
    pub visual: u32,
    /// The X depth of its visual: 24, or 32 with alpha.
    pub depth: u8,
    pub double: bool,
    pub depth_bits: u32,
    pub stencil_bits: u32,
}

impl Config {
    fn alpha(&self) -> u32 {
        if self.depth == 32 { 8 } else { 0 }
    }

    /// Its attributes, as GetFBConfigs sends them: the same names, in the same order, for every config.
    fn attributes(&self) -> [(u32, u32); 29] {
        [
            (VISUAL_ID, self.visual),
            (FBCONFIG_ID, self.id),
            (X_RENDERABLE, 1),
            (USE_GL, 1),
            (RENDER_TYPE, RGBA_BIT),
            (DOUBLEBUFFER, u32::from(self.double)),
            (STEREO, 0),
            (BUFFER_SIZE, 24 + self.alpha()),
            (LEVEL, 0),
            (AUX_BUFFERS, 0),
            (RED_SIZE, 8),
            (GREEN_SIZE, 8),
            (BLUE_SIZE, 8),
            (ALPHA_SIZE, self.alpha()),
            (ACCUM_RED_SIZE, 0),
            (ACCUM_GREEN_SIZE, 0),
            (ACCUM_BLUE_SIZE, 0),
            (ACCUM_ALPHA_SIZE, 0),
            (DEPTH_SIZE, self.depth_bits),
            (STENCIL_SIZE, self.stencil_bits),
            (X_VISUAL_TYPE, TRUE_COLOR),
            (CONFIG_CAVEAT, NONE),
            (TRANSPARENT_TYPE, NONE),
            (DRAWABLE_TYPE, WINDOW_BIT | PIXMAP_BIT | PBUFFER_BIT),
            (SAMPLE_BUFFERS, 0),
            (SAMPLES, 0),
            (MAX_PBUFFER_WIDTH, MAX_PBUFFER),
            (MAX_PBUFFER_HEIGHT, MAX_PBUFFER),
            (MAX_PBUFFER_PIXELS, MAX_PBUFFER * MAX_PBUFFER),
        ]
    }

    /// As GetVisualConfigs sends it: the 18 values every visual's config has, then names and values.
    fn visual_config(&self) -> Vec<u32> {
        let a = self.alpha();
        let mut v = vec![self.visual, 4, 1, 8, 8, 8, a, 0, 0, 0, 0, u32::from(self.double), 0, 24 + a, self.depth_bits, self.stencil_bits, 0, 0];
        for (name, value) in [(CONFIG_CAVEAT, NONE), (TRANSPARENT_TYPE, NONE), (SAMPLE_BUFFERS, 0), (SAMPLES, 0), (FBCONFIG_ID, self.id), (DRAWABLE_TYPE, WINDOW_BIT | PIXMAP_BIT | PBUFFER_BIT), (FRAMEBUFFER_SRGB_CAPABLE, 0)] {
            v.extend([name, value]);
        }
        v
    }
}

/// The screen's configs. Each visual's own comes first, numbered as the visuals are (24-bit depth and 8-bit stencil,
/// double-buffered or not as the visual is): what `glXChooseVisual` and a context made for a visual get. Then the
/// framebuffer configs that share those visuals with other depth and stencil buffers, for `glXChooseFBConfig`. The
/// client's driver keeps those it can draw.
pub fn configs() -> Vec<Config> {
    let mut out: Vec<Config> = VISUALS.iter().map(|&(visual, depth, double)| Config { id: 0, visual, depth, double, depth_bits: 24, stencil_bits: 8 }).collect();
    for &(visual, depth, double) in &VISUALS {
        for (depth_bits, stencil_bits) in [(0, 0), (16, 0), (24, 0), (32, 0)] {
            out.push(Config { id: 0, visual, depth, double, depth_bits, stencil_bits });
        }
    }
    for (i, c) in out.iter_mut().enumerate() {
        c.id = i as u32 + 1;
    }
    out
}

fn config(id: u32) -> Option<Config> {
    configs().into_iter().find(|c| c.id == id)
}

/// A visual's own config.
fn visual_config(visual: u32) -> Option<Config> {
    configs().into_iter().find(|c| c.visual == visual)
}

/// A GLX resource: a context, or a GLX drawable named for an X drawable.
pub enum Glx {
    /// Its config (0 with no config, `GLX_EXT_no_config_context`) and the context it shares objects with.
    Context { config: u32, share: u32, render_type: u32 },
    Window { window: u32, config: u32, event_mask: u32 },
    Pixmap { pixmap: u32, config: u32, event_mask: u32 },
    Pbuffer { config: u32, width: u32, height: u32, preserved: bool, event_mask: u32 },
}

impl Server {
    fn glx_res(&self, id: u32) -> Option<&Glx> {
        match self.res.get(&id) {
            Some((_, Res::Glx(g))) => Some(g),
            _ => None,
        }
    }

    fn put_glx(&mut self, ci: u32, id: u32, g: Glx) {
        if self.new_id(ci, id) {
            self.res.insert(id, (ci, Res::Glx(g)));
        }
    }

    /// Takes away a GLX resource of the kind `is` picks, or sends `error` for it.
    fn drop_glx(&mut self, ci: u32, id: u32, is: fn(&Glx) -> bool, error: u8) {
        if self.glx_res(id).is_some_and(is) {
            self.res.remove(&id);
        } else {
            self.error(ci, error, id);
        }
    }

    pub(super) fn glx(&mut self, ci: u32, r: Req) {
        // Requests for screens other than the one there is.
        let screen_ok = |at: usize| r.u32(at) == 0;
        match r.b1() {
            // Render, RenderLarge, SwapBuffers, CopyContext, UseXFont: an indirect context's, and none is current.
            1 | 2 | 10 | 11 | 12 => self.error(ci, BAD_CONTEXT_TAG, 0),
            // CreateContext: for a visual.
            3 => {
                let (id, visual, share, direct) = (r.u32(4), r.u32(8), r.u32(16), r.u8(20) != 0);
                let Some(c) = visual_config(visual).filter(|_| screen_ok(12)) else { return self.error(ci, err::VALUE, visual) };
                self.create_context(ci, id, c.id, share, RGBA_TYPE, direct);
            }
            4 => self.drop_glx(ci, r.u32(4), |g| matches!(g, Glx::Context { .. }), BAD_CONTEXT),
            // MakeCurrent and MakeContextCurrent: only an indirect context's come here.
            5 | 26 => self.error(ci, BAD_CONTEXT, r.u32(if r.b1() == 5 { 8 } else { 16 })),
            6 => {
                let id = r.u32(4);
                if !matches!(self.glx_res(id), Some(Glx::Context { .. })) {
                    return self.error(ci, BAD_CONTEXT, id);
                }
                let mut rep = Reply::new(0);
                rep.u8(1);
                self.reply(ci, rep);
            }
            7 => {
                let mut rep = Reply::new(0);
                rep.u32(1).u32(4);
                self.reply(ci, rep);
            }
            // WaitGL, WaitX, VendorPrivate (swap intervals and the like, the client's own), ClientInfo,
            // SetClientInfoARB, SetClientInfo2ARB: nothing to do.
            8 | 9 | 16 | 20 | 33 | 35 => {}
            13 => {
                let (visual, pixmap, id) = (r.u32(8), r.u32(12), r.u32(16));
                let Some(c) = visual_config(visual).filter(|_| screen_ok(4)) else { return self.error(ci, err::VALUE, visual) };
                self.create_glx_pixmap(ci, c, pixmap, id);
            }
            14 => {
                if !screen_ok(4) {
                    return self.error(ci, err::VALUE, r.u32(4));
                }
                let visuals: Vec<Vec<u32>> = configs().iter().filter(|c| visual_config(c.visual) == Some(**c)).map(Config::visual_config).collect();
                let mut rep = Reply::new(0);
                rep.u32(visuals.len() as u32).u32(visuals.first().map_or(0, Vec::len) as u32).skip(16);
                for v in visuals.iter().flatten() {
                    rep.u32(*v);
                }
                self.reply(ci, rep);
            }
            15 | 23 => self.drop_glx(ci, r.u32(4), |g| matches!(g, Glx::Pixmap { .. }), BAD_PIXMAP),
            17 => self.error(ci, UNSUPPORTED_PRIVATE_REQUEST, 0),
            // QueryExtensionsString, QueryServerString.
            18 | 19 => {
                if !screen_ok(4) {
                    return self.error(ci, err::VALUE, r.u32(4));
                }
                let s = match (r.b1(), r.u32(8)) {
                    (18, _) | (_, 3) => EXTENSIONS,
                    (_, 1) => VENDOR,
                    (_, 2) => VERSION,
                    (_, 0x20f6) => VENDOR_NAMES,
                    (_, name) => return self.error(ci, err::VALUE, name),
                };
                let mut rep = Reply::new(0);
                rep.u32(0).u32(s.len() as u32 + 1).skip(16).bytes(s.as_bytes()).u8(0);
                self.reply(ci, rep);
            }
            21 => {
                if !screen_ok(4) {
                    return self.error(ci, err::VALUE, r.u32(4));
                }
                let configs = configs();
                let mut rep = Reply::new(0);
                rep.u32(configs.len() as u32).u32(configs[0].attributes().len() as u32).skip(16);
                for (name, value) in configs.iter().flat_map(Config::attributes) {
                    rep.u32(name).u32(value);
                }
                self.reply(ci, rep);
            }
            22 => {
                let (fbconfig, pixmap, id) = (r.u32(8), r.u32(12), r.u32(16));
                let Some(c) = config(fbconfig).filter(|_| screen_ok(4)) else { return self.error(ci, BAD_FBCONFIG, fbconfig) };
                self.create_glx_pixmap(ci, c, pixmap, id);
            }
            // CreateNewContext: for a framebuffer config.
            24 => {
                let (id, fbconfig, render_type, share, direct) = (r.u32(4), r.u32(8), r.u32(16), r.u32(20), r.u8(24) != 0);
                let Some(c) = config(fbconfig).filter(|_| screen_ok(12)) else { return self.error(ci, BAD_FBCONFIG, fbconfig) };
                self.create_context(ci, id, c.id, share, render_type, direct);
            }
            25 => {
                let id = r.u32(4);
                let Some(&Glx::Context { config: c, share, render_type }) = self.glx_res(id) else { return self.error(ci, BAD_CONTEXT, id) };
                let visual = config(c).map_or(0, |c| c.visual);
                let pairs = [(SHARE_CONTEXT, share), (VISUAL_ID, visual), (SCREEN, 0), (FBCONFIG_ID, c), (RENDER_TYPE, render_type)];
                self.attribute_reply(ci, &pairs);
            }
            27 => {
                let (fbconfig, id, n) = (r.u32(8), r.u32(12), r.u32(16) as usize);
                let Some(c) = config(fbconfig).filter(|_| screen_ok(4)) else { return self.error(ci, BAD_FBCONFIG, fbconfig) };
                let (mut width, mut height, mut preserved) = (0, 0, true);
                for i in 0..n {
                    match (r.u32(20 + 8 * i), r.u32(24 + 8 * i)) {
                        (PBUFFER_WIDTH, v) => width = v,
                        (PBUFFER_HEIGHT, v) => height = v,
                        (PRESERVED_CONTENTS, v) => preserved = v != 0,
                        _ => {}
                    }
                }
                if width > MAX_PBUFFER || height > MAX_PBUFFER {
                    return self.error(ci, err::VALUE, width.max(height));
                }
                self.put_glx(ci, id, Glx::Pbuffer { config: c.id, width, height, preserved, event_mask: 0 });
            }
            28 => self.drop_glx(ci, r.u32(4), |g| matches!(g, Glx::Pbuffer { .. }), BAD_PBUFFER),
            29 => self.drawable_attributes(ci, r.u32(4)),
            30 => {
                let (id, n) = (r.u32(4), r.u32(8) as usize);
                let mask = (0..n).map(|i| (r.u32(12 + 8 * i), r.u32(16 + 8 * i))).find(|&(name, _)| name == EVENT_MASK).map(|(_, v)| v);
                match self.res.get_mut(&id) {
                    Some((_, Res::Glx(Glx::Window { event_mask, .. } | Glx::Pixmap { event_mask, .. } | Glx::Pbuffer { event_mask, .. }))) => {
                        if let Some(m) = mask {
                            *event_mask = m;
                        }
                    }
                    _ => self.error(ci, BAD_DRAWABLE, id),
                }
            }
            31 => {
                let (fbconfig, window, id) = (r.u32(8), r.u32(12), r.u32(16));
                let Some(c) = config(fbconfig).filter(|_| screen_ok(4)) else { return self.error(ci, BAD_FBCONFIG, fbconfig) };
                let Some(win) = self.windows.get(&window) else { return self.error(ci, err::WINDOW, window) };
                if win.depth != c.depth {
                    return self.error(ci, err::MATCH, window);
                }
                self.put_glx(ci, id, Glx::Window { window, config: c.id, event_mask: 0 });
            }
            32 => self.drop_glx(ci, r.u32(4), |g| matches!(g, Glx::Window { .. }), BAD_WINDOW),
            // CreateContextAttribsARB: for a framebuffer config, or none.
            34 => {
                let (id, fbconfig, share, direct, n) = (r.u32(4), r.u32(8), r.u32(16), r.u8(20) != 0, r.u32(24) as usize);
                if fbconfig != 0 && config(fbconfig).is_none() || !screen_ok(12) {
                    return self.error(ci, BAD_FBCONFIG, fbconfig);
                }
                // GLX_RENDER_TYPE among its attributes, else RGBA.
                let render_type = (0..n).map(|i| (r.u32(28 + 8 * i), r.u32(32 + 8 * i))).find(|&(name, _)| name == RENDER_TYPE).map_or(RGBA_TYPE, |(_, v)| v);
                self.create_context(ci, id, fbconfig, share, render_type, direct);
            }
            _ => self.error(ci, err::REQUEST, 0),
        }
    }

    fn create_context(&mut self, ci: u32, id: u32, config: u32, share: u32, render_type: u32, direct: bool) {
        if !direct {
            return self.error(ci, err::VALUE, 0);
        }
        if share != 0 && !matches!(self.glx_res(share), Some(Glx::Context { .. })) {
            return self.error(ci, BAD_CONTEXT, share);
        }
        self.put_glx(ci, id, Glx::Context { config, share, render_type });
    }

    fn create_glx_pixmap(&mut self, ci: u32, c: Config, pixmap: u32, id: u32) {
        match self.res.get(&pixmap) {
            Some((_, Res::Pixmap(p))) if p.depth == c.depth => self.put_glx(ci, id, Glx::Pixmap { pixmap, config: c.id, event_mask: 0 }),
            Some((_, Res::Pixmap(_))) => self.error(ci, err::MATCH, pixmap),
            _ => self.error(ci, err::PIXMAP, pixmap),
        }
    }

    /// GetDrawableAttributes: a GLX drawable's, or a plain window's (as its visual's config makes it).
    fn drawable_attributes(&mut self, ci: u32, id: u32) {
        let window_size = |s: &Server, w: u32| s.windows.get(&w).map(|w| (w.w as u32, w.h as u32));
        let (kind, config, size, mask) = match self.glx_res(id) {
            Some(&Glx::Window { window, config, event_mask }) => (WINDOW_BIT, config, window_size(self, window), event_mask),
            Some(&Glx::Pixmap { pixmap, config, event_mask }) => {
                let size = match self.res.get(&pixmap) {
                    Some((_, Res::Pixmap(p))) => {
                        let s = kernels_core::lock(&p.surface);
                        Some((s.width as u32, s.height as u32))
                    }
                    _ => None,
                };
                (PIXMAP_BIT, config, size, event_mask)
            }
            Some(&Glx::Pbuffer { config, width, height, event_mask, .. }) => (PBUFFER_BIT, config, Some((width, height)), event_mask),
            Some(Glx::Context { .. }) => return self.error(ci, BAD_DRAWABLE, id),
            None => match self.windows.get(&id) {
                Some(w) if !w.input_only => (WINDOW_BIT, visual_config(w.visual).map_or(0, |c| c.id), window_size(self, id), 0),
                _ => return self.error(ci, BAD_DRAWABLE, id),
            },
        };
        // A drawable whose X drawable is gone is gone too.
        let Some((width, height)) = size else { return self.error(ci, BAD_DRAWABLE, id) };
        let mut pairs = vec![(Y_INVERTED, 0), (WIDTH, width), (HEIGHT, height), (SCREEN, 0), (FBCONFIG_ID, config), (DRAWABLE_TYPE, kind), (EVENT_MASK, mask)];
        if let Some(&Glx::Pbuffer { preserved, .. }) = self.glx_res(id) {
            pairs.extend([(PRESERVED_CONTENTS, u32::from(preserved)), (LARGEST_PBUFFER, 0)]);
        }
        self.attribute_reply(ci, &pairs);
    }

    /// A reply of names and values, as GetDrawableAttributes' and QueryContext's are.
    fn attribute_reply(&mut self, ci: u32, pairs: &[(u32, u32)]) {
        let mut rep = Reply::new(0);
        rep.u32(pairs.len() as u32).skip(20);
        for &(name, value) in pairs {
            rep.u32(name).u32(value);
        }
        self.reply(ci, rep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_visual_has_its_own_config_first() {
        let configs = configs();
        for (i, &(visual, depth, double)) in VISUALS.iter().enumerate() {
            let c = visual_config(visual).unwrap();
            assert_eq!((c.id, c.depth, c.double, c.depth_bits, c.stencil_bits), (i as u32 + 1, depth, double, 24, 8));
        }
        let ids: std::collections::HashSet<u32> = configs.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), configs.len());
    }

    #[test]
    fn configs_say_the_same_names_in_the_same_order() {
        let names = |c: &Config| c.attributes().map(|(n, _)| n);
        let configs = configs();
        assert!(configs.iter().all(|c| names(c) == names(&configs[0])));
        assert!(configs.iter().all(|c| c.visual_config().len() == configs[0].visual_config().len()));
        // The 18 values every visual's config starts with, then pairs.
        assert_eq!((configs[0].visual_config().len() - 18) % 2, 0);
    }
}
