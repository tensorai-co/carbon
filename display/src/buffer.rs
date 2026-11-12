//! What a window shows: a buffer of pixels a kernel lends the desktop. The desktop reads it in place as it composites
//! and never writes it, so a client's shared memory (a Wayland `wl_shm` pool, X11's MIT-SHM, a DIB section) is shown
//! without a copy. A buffer is attached to a window and committed with its damage; when another is committed in its
//! place, the desktop says the old one is released, and its kernel may reuse it.
//!
//! A kernel that draws itself, as win32k's GDI does, keeps a [`Canvas`]: a buffer in memory it draws into, attached
//! once, committed after each drawing with what it drew.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Rgb, Surface};

/// How a buffer's pixels are laid out, one 32-bit word each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `0x00RRGGBB` (the top byte ignored): opaque.
    Xrgb8888,
    /// `0xAARRGGBB`, with its colour already multiplied by its alpha, as Wayland and X11's 32-bit visuals have it.
    Argb8888,
}

/// A source of pixels the desktop reads in place. Reads may come from another thread at any time while the buffer is
/// attached; a source that changes meanwhile shows what it has when it's read.
pub trait Pixels: Send + Sync {
    /// Its width and height, in pixels.
    fn size(&self) -> (i32, i32);
    fn format(&self) -> Format;
    /// Row `y`'s pixels from column `x`, as many as `out` holds (which the desktop keeps within its size).
    fn read_row(&self, x: i32, y: i32, out: &mut [u32]);
}

/// A buffer's identity, for the `Released` event that gives it back.
pub type BufferId = u64;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Pixels to attach to a window.
#[derive(Clone)]
pub struct Buffer {
    pub id: BufferId,
    pub pixels: Arc<dyn Pixels>,
}

impl Buffer {
    pub fn new(pixels: Arc<dyn Pixels>) -> Buffer {
        Buffer { id: NEXT.fetch_add(1, Ordering::Relaxed), pixels }
    }
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buffer").field("id", &self.id).field("size", &self.pixels.size()).finish()
    }
}

impl Pixels for Mutex<Surface> {
    fn size(&self) -> (i32, i32) {
        let s = lock(self);
        (s.width, s.height)
    }

    fn format(&self) -> Format {
        Format::Xrgb8888
    }

    fn read_row(&self, x: i32, y: i32, out: &mut [u32]) {
        let s = lock(self);
        let start = (y * s.width + x) as usize;
        out.copy_from_slice(&s.pixels[start..start + out.len()]);
    }
}

fn lock(s: &Mutex<Surface>) -> MutexGuard<'_, Surface> {
    s.lock().unwrap_or_else(|p| p.into_inner())
}

/// A buffer in memory that its kernel draws into, shown in place: win32k's for each top-level window.
#[derive(Clone)]
pub struct Canvas {
    surface: Arc<Mutex<Surface>>,
    buffer: Buffer,
}

impl Canvas {
    pub fn new(width: i32, height: i32, fill: Rgb) -> Canvas {
        let surface = Arc::new(Mutex::new(Surface::new(width.max(0), height.max(0), fill)));
        let buffer = Buffer::new(surface.clone());
        Canvas { surface, buffer }
    }

    /// The buffer to attach: the same one for as long as the canvas lives.
    pub fn buffer(&self) -> Buffer {
        self.buffer.clone()
    }

    /// Draws into it. The desktop's lock must not be held: the desktop takes this one under its own.
    pub fn draw<R>(&self, f: impl FnOnce(&mut Surface) -> R) -> R {
        f(&mut lock(&self.surface))
    }

    pub fn size(&self) -> (i32, i32) {
        self.surface.size()
    }

    /// Makes it `width` by `height`, keeping what's still inside and filling what's new. True if it changed.
    pub fn resize(&self, width: i32, height: i32, fill: Rgb) -> bool {
        let mut s = lock(&self.surface);
        if (s.width, s.height) == (width.max(0), height.max(0)) {
            return false;
        }
        s.resize(width.max(0), height.max(0), fill);
        true
    }

    /// A copy of what it shows now.
    pub fn snapshot(&self) -> Surface {
        lock(&self.surface).clone()
    }
}
