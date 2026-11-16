//! Framebuffer objects and renderbuffers (GL 3.0, `GL_ARB_framebuffer_object`): a program's own framebuffers, which
//! draws, clears and reads go to instead of the drawable's when one is bound.
//!
//! A framebuffer object keeps what's attached to it, a texture's level or a renderbuffer, by name; what it draws into
//! is worked out from them each time it's used ([`resolve`]), so a texture given a new image of another size is
//! simply what's drawn into next. Up to [`COLORS`] colour attachments and a depth attachment are drawn into, each
//! fragment output going to the attachment its draw buffer names; a stencil attachment is kept, and has no stencil
//! to draw into, as the default framebuffer hasn't.
//!
//! Depth renderbuffers and textures are `D32Float`, whatever depth format they're asked for in, as the default
//! framebuffer's depth is.

use gpu::{Format, ImageDesc, Rect, Sub};
use gpu_wire::gl as wire;
use gpu_wire::records;

use super::draw::not_yet;
use super::{Framebuffer, Gl, INVALID_ENUM, INVALID_OPERATION, INVALID_VALUE, Objects, R, Step, context, device_error, framebuffer_of};

const FRAMEBUFFER: u32 = 0x8d40;
const READ_FRAMEBUFFER: u32 = 0x8ca8;
const DRAW_FRAMEBUFFER: u32 = 0x8ca9;
const RENDERBUFFER: u32 = 0x8d41;
const TEXTURE_2D: u32 = 0x0de1;
const COLOR_ATTACHMENT0: u32 = 0x8ce0;
const DEPTH_ATTACHMENT: u32 = 0x8d00;
const STENCIL_ATTACHMENT: u32 = 0x8d20;
const DEPTH_STENCIL_ATTACHMENT: u32 = 0x821a;
const COLOR_BUFFER_BIT: u32 = 0x4000;
const DEPTH_BUFFER_BIT: u32 = 0x0100;
const STENCIL_BUFFER_BIT: u32 = 0x0400;
const NONE: u32 = 0;

/// `glCheckFramebufferStatus`'s answers.
const COMPLETE: u32 = 0x8cd5;
const INCOMPLETE_ATTACHMENT: u32 = 0x8cd6;
const INCOMPLETE_MISSING_ATTACHMENT: u32 = 0x8cd7;
const UNSUPPORTED: u32 = 0x8cdd;

/// What's attached at an attachment point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Attachment {
    Texture { name: u32, level: u32 },
    Renderbuffer(u32),
}

/// Colour attachments a framebuffer object has, and draw buffers (`GL_MAX_COLOR_ATTACHMENTS`, `GL_MAX_DRAW_BUFFERS`).
pub(super) const COLORS: usize = 4;

#[derive(Clone, Copy)]
pub(super) struct Fbo {
    colors: [Option<Attachment>; COLORS],
    depth: Option<Attachment>,
    stencil: Option<Attachment>,
    /// Each fragment output's colour attachment (`glDrawBuffers`; [`NO_BUFFER`] for none), and the one reads read
    /// (`glReadBuffer`).
    draw: [usize; COLORS],
    read: usize,
}

/// A draw buffer that's none.
const NO_BUFFER: usize = usize::MAX;

impl Default for Fbo {
    fn default() -> Fbo {
        // Output 0 to attachment 0 and the rest to none; reads from attachment 0.
        Fbo { colors: [None; COLORS], depth: None, stencil: None, draw: [0, NO_BUFFER, NO_BUFFER, NO_BUFFER], read: 0 }
    }
}

pub(super) struct Renderbuffer {
    image: Option<gpu::Image>,
    format: Format,
    width: u32,
    height: u32,
}

/// What a framebuffer object draws into now, or why it can't be drawn into (`glCheckFramebufferStatus`'s answer).
/// A read framebuffer's colour is read even when its draw buffers are none.
pub(super) fn resolve(o: &Objects, name: u32, read: bool) -> Result<Framebuffer, u32> {
    let fbo = o.fbos.get(&name).ok_or(INCOMPLETE_MISSING_ATTACHMENT)?;
    // An attachment's image, level, format and size.
    let image = |a: Attachment| -> Result<(gpu::Image, u32, Format, u32, u32), u32> {
        match a {
            Attachment::Texture { name, level } => {
                let (image, format, w, h) = o.textures.get(&name).and_then(|t| t.level(level)).ok_or(INCOMPLETE_ATTACHMENT)?;
                Ok((image, level, format, w, h))
            }
            Attachment::Renderbuffer(name) => {
                let r = o.renderbuffers.get(&name).ok_or(INCOMPLETE_ATTACHMENT)?;
                Ok((r.image.ok_or(INCOMPLETE_ATTACHMENT)?, 0, r.format, r.width, r.height))
            }
        }
    };
    // Every attachment there must be one that can be drawn into, whichever are drawn into now.
    let mut attached = [None; COLORS];
    for (i, a) in fbo.colors.iter().enumerate() {
        if let Some(a) = a {
            let c = image(*a)?;
            if c.2.is_depth() || !c.2.is_renderable() {
                return Err(INCOMPLETE_ATTACHMENT);
            }
            attached[i] = Some(c);
        }
    }
    // What each output goes to (reading: the read buffer's attachment, as output 0).
    let outputs: [Option<_>; COLORS] = if read {
        let mut o = [None; COLORS];
        o[0] = attached.get(fbo.read).copied().flatten();
        o
    } else {
        fbo.draw.map(|a| attached.get(a).copied().flatten())
    };
    let color = outputs[0];
    let depth = match fbo.depth {
        Some(a) => {
            let d = image(a)?;
            if !d.2.is_depth() {
                return Err(INCOMPLETE_ATTACHMENT);
            }
            Some(d)
        }
        None => None,
    };
    if fbo.colors.iter().all(Option::is_none) && fbo.depth.is_none() && fbo.stencil.is_none() {
        return Err(INCOMPLETE_MISSING_ATTACHMENT);
    }
    let size = outputs.iter().flatten().map(|c| (c.3, c.4)).next().or(depth.map(|d| (d.3, d.4)));
    // The device draws into attachments of one size only.
    let sizes = outputs.iter().flatten().map(|c| (c.3, c.4)).chain(depth.map(|d| (d.3, d.4)));
    if sizes.clone().any(|s| Some(s) != size) {
        return Err(UNSUPPORTED);
    }
    // Only a stencil attachment, or a colour one its draw buffers leave out: nothing to draw into.
    let Some((width, height)) = size else { return Err(if read { INCOMPLETE_ATTACHMENT } else { UNSUPPORTED }) };
    let sub = |level: u32| Sub { level, layer: 0 };
    Ok(Framebuffer {
        key: 1 << 63 | u64::from(name),
        width,
        height,
        color: color.map(|c| (c.0, sub(c.1))),
        format: color.map_or(Format::Rgba8Unorm, |c| c.2),
        extra: [1, 2, 3].map(|i| outputs[i].map(|c| (c.0, sub(c.1), c.2))),
        depth: depth.map(|d| (d.0, sub(d.1))),
        alpha: true,
        stencil_bits: if fbo.stencil.is_some() { 8 } else { 0 },
    })
}

/// A renderbuffer's internal format as the device's, or None for one GL hasn't.
fn renderbuffer_format(internal: u32) -> Option<Format> {
    Some(match internal {
        // DEPTH_COMPONENT, DEPTH_COMPONENT16/24/32, DEPTH_COMPONENT32F, DEPTH_STENCIL, DEPTH24_STENCIL8,
        // DEPTH32F_STENCIL8, STENCIL_INDEX8 (no stencil on the device: a depth image, never drawn into).
        0x1902 | 0x81a5 | 0x81a6 | 0x81a7 | 0x8cac | 0x84f9 | 0x88f0 | 0x8cad | 0x8d48 => Format::D32Float,
        // RGBA, RGBA8, RGB, RGB8, RGBA4, RGB5_A1, RGB565.
        0x1908 | 0x8058 | 0x1907 | 0x8051 | 0x8056 | 0x8057 | 0x8d62 => Format::Rgba8Unorm,
        // SRGB8_ALPHA8, SRGB8.
        0x8c43 | 0x8c41 => Format::Rgba8Srgb,
        0x8229 => Format::R8Unorm,
        0x822b => Format::Rg8Unorm,
        0x8059 => Format::Rgb10a2Unorm,
        0x8c3a => Format::Rg11b10Float,
        0x881a | 0x881b | 0x822d | 0x822f => Format::Rgba16Float,
        0x8814 | 0x8815 | 0x822e | 0x8230 => Format::Rgba32Float,
        _ => return None,
    })
}

impl Gl {
    /// A framebuffer object's or renderbuffer's record, if it's one: whether it was.
    pub(super) fn framebuffer_record(&self, o: &mut Objects, current: u64, kind: u32, p: &mut R<'_>, reply: &mut Vec<u8>) -> Result<bool, String> {
        match kind {
            wire::BIND_FRAMEBUFFER => {
                let (target, name) = (p.u32()?, p.u32()?);
                if name != 0 {
                    o.fbos.entry(name).or_default();
                }
                let c = context(o, current)?;
                match target {
                    FRAMEBUFFER => {
                        c.draw_fbo = name;
                        c.read_fbo = name;
                    }
                    DRAW_FRAMEBUFFER => c.draw_fbo = name,
                    READ_FRAMEBUFFER => c.read_fbo = name,
                    _ => c.fail(INVALID_ENUM),
                }
            }
            wire::DELETE_FRAMEBUFFER => {
                let name = p.u32()?;
                if o.fbos.remove(&name).is_some() {
                    self.flush(o)?;
                    // Deleting a bound framebuffer binds the default one in its place.
                    for c in o.contexts.values_mut() {
                        if c.draw_fbo == name {
                            c.draw_fbo = 0;
                        }
                        if c.read_fbo == name {
                            c.read_fbo = 0;
                        }
                    }
                }
            }
            wire::FRAMEBUFFER_TEXTURE | wire::FRAMEBUFFER_RENDERBUFFER => {
                let (target, attachment, textarget, object) = (p.u32()?, p.u32()?, p.u32()?, p.u32()?);
                let level = if kind == wire::FRAMEBUFFER_TEXTURE { p.u32()? } else { 0 };
                let Some(name) = self.bound_fbo(o, current, target)? else { return Ok(true) };
                let what = if object == 0 {
                    None
                } else if kind == wire::FRAMEBUFFER_TEXTURE {
                    if textarget != TEXTURE_2D {
                        not_yet("a framebuffer attachment of a texture other than GL_TEXTURE_2D", 32);
                        return Ok(true);
                    }
                    if !o.textures.contains_key(&object) {
                        context(o, current)?.fail(INVALID_OPERATION);
                        return Ok(true);
                    }
                    Some(Attachment::Texture { name: object, level })
                } else {
                    if textarget != RENDERBUFFER {
                        context(o, current)?.fail(INVALID_ENUM);
                        return Ok(true);
                    }
                    o.renderbuffers.entry(object).or_insert(Renderbuffer { image: None, format: Format::Rgba8Unorm, width: 0, height: 0 });
                    Some(Attachment::Renderbuffer(object))
                };
                // What's drawn so far went to the attachments as they were.
                self.flush(o)?;
                let fbo = o.fbos.entry(name).or_default();
                match attachment {
                    COLOR_ATTACHMENT0..=0x8ce3 => fbo.colors[(attachment - COLOR_ATTACHMENT0) as usize] = what,
                    DEPTH_ATTACHMENT => fbo.depth = what,
                    STENCIL_ATTACHMENT => fbo.stencil = what,
                    DEPTH_STENCIL_ATTACHMENT => {
                        fbo.depth = what;
                        fbo.stencil = what;
                    }
                    0x8ce4..=0x8cff => context(o, current)?.fail(INVALID_OPERATION),
                    _ => context(o, current)?.fail(INVALID_ENUM),
                }
            }
            wire::CHECK_FRAMEBUFFER => {
                let target = p.u32()?;
                let c = context(o, current)?;
                let status = match target {
                    FRAMEBUFFER | DRAW_FRAMEBUFFER => Some((c.draw_fbo, false)),
                    READ_FRAMEBUFFER => Some((c.read_fbo, true)),
                    _ => {
                        c.fail(INVALID_ENUM);
                        None
                    }
                }
                .map_or(0, |(name, read)| if name == 0 { COMPLETE } else { resolve(o, name, read).map_or_else(|why| why, |_| COMPLETE) });
                records::put_u32(reply, status);
            }
            wire::BIND_RENDERBUFFER => {
                let (target, name) = (p.u32()?, p.u32()?);
                if name != 0 {
                    o.renderbuffers.entry(name).or_insert(Renderbuffer { image: None, format: Format::Rgba8Unorm, width: 0, height: 0 });
                }
                let c = context(o, current)?;
                if target == RENDERBUFFER {
                    c.renderbuffer = name;
                } else {
                    c.fail(INVALID_ENUM);
                }
            }
            wire::RENDERBUFFER_STORAGE => {
                let [target, internal, width, height] = p.words::<4>()?;
                let c = context(o, current)?;
                let name = c.renderbuffer;
                if target != RENDERBUFFER {
                    c.fail(INVALID_ENUM);
                    return Ok(true);
                }
                let Some(format) = renderbuffer_format(internal) else {
                    c.fail(INVALID_ENUM);
                    return Ok(true);
                };
                if width > gpu::MAX_IMAGE || height > gpu::MAX_IMAGE {
                    c.fail(INVALID_VALUE);
                    return Ok(true);
                }
                if name == 0 {
                    c.fail(INVALID_OPERATION);
                    return Ok(true);
                }
                self.flush(o)?;
                let old = o.renderbuffers.get_mut(&name).and_then(|r| r.image.take());
                let image = self
                    .gpu
                    .run(move |d| -> Result<Option<gpu::Image>, gpu::Error> {
                        if let Some(i) = old {
                            d.free_image(i);
                        }
                        if width == 0 || height == 0 {
                            return Ok(None);
                        }
                        d.image(ImageDesc::new(format, width, height)).map(Some)
                    })
                    .map_err(device_error)?
                    .map_err(device_error)?;
                o.renderbuffers.insert(name, Renderbuffer { image, format, width, height });
            }
            wire::DELETE_RENDERBUFFER => {
                let name = p.u32()?;
                if let Some(r) = o.renderbuffers.remove(&name) {
                    self.flush(o)?;
                    if let Some(i) = r.image {
                        self.gpu.run(move |d| d.free_image(i)).map_err(device_error)?;
                    }
                    for c in o.contexts.values_mut() {
                        if c.renderbuffer == name {
                            c.renderbuffer = 0;
                        }
                    }
                }
            }
            wire::BLIT_FRAMEBUFFER => {
                let s = p.words::<4>()?.map(|w| w as i32);
                let d = p.words::<4>()?.map(|w| w as i32);
                let (mask, _filter) = (p.u32()?, p.u32()?);
                self.blit(o, current, s, d, mask)?;
            }
            wire::DRAW_BUFFERS => {
                let n = p.u32()? as usize;
                let mut list = [NO_BUFFER; COLORS];
                let mut ok = n <= COLORS;
                for i in 0..n {
                    let b = p.u32()?;
                    if let Some(slot) = list.get_mut(i) {
                        *slot = match b {
                            NONE => NO_BUFFER,
                            COLOR_ATTACHMENT0..=0x8ce3 => (b - COLOR_ATTACHMENT0) as usize,
                            // BACK, for the default framebuffer.
                            0x0405 | 0x0404 => 0,
                            _ => {
                                ok = false;
                                NO_BUFFER
                            }
                        };
                    }
                }
                let c = context(o, current)?;
                let name = c.draw_fbo;
                if !ok {
                    c.fail(INVALID_ENUM);
                } else if name != 0 {
                    self.flush(o)?;
                    if let Some(f) = o.fbos.get_mut(&name) {
                        f.draw = list;
                    }
                }
            }
            wire::READ_COLOR => {
                let mode = p.u32()?;
                let c = context(o, current)?;
                let name = c.read_fbo;
                if name != 0
                    && let Some(f) = o.fbos.get_mut(&name)
                {
                    f.read = match mode {
                        COLOR_ATTACHMENT0..=0x8ce3 => (mode - COLOR_ATTACHMENT0) as usize,
                        _ => NO_BUFFER,
                    };
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The framebuffer object bound at `target`, or the error that there's none (the default framebuffer's
    /// attachments can't be changed).
    fn bound_fbo(&self, o: &mut Objects, current: u64, target: u32) -> Result<Option<u32>, String> {
        let c = context(o, current)?;
        let name = match target {
            FRAMEBUFFER | DRAW_FRAMEBUFFER => c.draw_fbo,
            READ_FRAMEBUFFER => c.read_fbo,
            _ => {
                c.fail(INVALID_ENUM);
                return Ok(None);
            }
        };
        if name == 0 {
            c.fail(INVALID_OPERATION);
            return Ok(None);
        }
        Ok(Some(name))
    }

    /// `glBlitFramebuffer`, as far as copies go: the read framebuffer's colour or depth into the draw framebuffer's,
    /// a region of one size to the same size, in the same format.
    fn blit(&self, o: &mut Objects, current: u64, s: [i32; 4], d: [i32; 4], mask: u32) -> Result<(), String> {
        let (from, to) = (framebuffer_of(o, current, true), framebuffer_of(o, current, false));
        let c = context(o, current)?;
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            c.fail(INVALID_VALUE);
            return Ok(());
        }
        let (Some(from), Some(to)) = (from, to) else {
            c.fail(super::INVALID_FRAMEBUFFER_OPERATION);
            return Ok(());
        };
        let (sw, sh, dw, dh) = (s[2] - s[0], s[3] - s[1], d[2] - d[0], d[3] - d[1]);
        if (sw, sh) != (dw, dh) || sw <= 0 || sh <= 0 {
            not_yet("a framebuffer blit that scales or flips", 128);
            return Ok(());
        }
        // Clipped to both framebuffers, the same on each side.
        let x0 = 0.max(-s[0]).max(-d[0]);
        let y0 = 0.max(-s[1]).max(-d[1]);
        let x1 = sw.min(from.width as i32 - s[0]).min(to.width as i32 - d[0]);
        let y1 = sh.min(from.height as i32 - s[1]).min(to.height as i32 - d[1]);
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let (width, height) = ((x1 - x0) as u32, (y1 - y0) as u32);
        let src = Rect { x: (s[0] + x0) as u32, y: (s[1] + y0) as u32, width, height };
        let dst = Rect { x: (d[0] + x0) as u32, y: (d[1] + y0) as u32, width, height };
        let mut pairs = Vec::new();
        if mask & COLOR_BUFFER_BIT != 0
            && let (Some(a), Some(b)) = (from.color, to.color)
        {
            if from.format != to.format {
                not_yet("a framebuffer blit between colour formats", 256);
            } else {
                pairs.push((a, b, from.format.bytes()));
            }
        }
        if mask & DEPTH_BUFFER_BIT != 0
            && let (Some(a), Some(b)) = (from.depth, to.depth)
        {
            pairs.push((a, b, 4));
        }
        for ((image, sub), (to_image, to_sub), bytes) in pairs {
            let texels = self.steps(o, vec![Step::Download { from: image, sub, region: src, bytes }])?.pop().unwrap_or_default();
            self.steps(o, vec![Step::Upload { bytes: texels, to: to_image, level: to_sub.level, region: dst }])?;
        }
        Ok(())
    }
}
