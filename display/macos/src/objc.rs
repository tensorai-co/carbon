//! The Objective-C runtime, AppKit's, CoreGraphics' and Core Animation's entry points, and the few helpers both modes
//! build on: sending messages, classes of our own, images for layers, the event loop's waking.

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) type Id = *mut c_void;
pub(crate) type Sel = *const c_void;

pub(crate) const NIL: Id = std::ptr::null_mut();

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Size {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Frame {
    pub origin: Point,
    pub size: Size,
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    pub(crate) fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(class: Id);
    fn class_addMethod(class: Id, name: Sel, imp: *const c_void, types: *const c_char) -> bool;
    fn objc_getProtocol(name: *const c_char) -> Id;
    fn class_addProtocol(class: Id, protocol: Id) -> bool;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSPasteboardTypeString: Id;
}

/// The pasteboard's type for plain text.
pub(crate) fn pasteboard_string_type() -> Id {
    // SAFETY: AppKit's constant string.
    unsafe { NSPasteboardTypeString }
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSDefaultRunLoopMode: Id;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAGravityResize: Id;
    static kCAFilterNearest: Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *mut c_void;
    fn CFRelease(object: *mut c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGDataProviderCreateWithCFData(data: *mut c_void) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        info: u32,
        provider: *mut c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
    fn CGEventGetLocation(event: *mut c_void) -> Point;
}

unsafe extern "C" {
    fn pthread_main_np() -> i32;
}

/// `0x00RRGGBB` words, little-endian: `kCGImageAlphaNoneSkipFirst | kCGBitmapByteOrder32Little`.
const XRGB_LITTLE: u32 = 6 | (2 << 12);
/// Premultiplied `0xAARRGGBB` words, little-endian: `kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little`.
const ARGB_LITTLE: u32 = 2 | (2 << 12);

pub(crate) fn sel(name: &str) -> Sel {
    let name = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated name; selectors live as long as the process.
    unsafe { sel_registerName(name.as_ptr()) }
}

pub(crate) fn class(name: &str) -> Id {
    let name = CString::new(name).unwrap_or_default();
    // SAFETY: a NUL-terminated name; nil if there's no such class.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// Sends a message: `msg!(Ret; receiver, "selector:with:", arg => Type, ...)`, called as the method's own C signature.
macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:expr $(, $arg:expr => $t:ty)* $(,)?) => {{
        let receiver: $crate::objc::Id = $obj;
        let selector: $crate::objc::Sel = $crate::objc::sel($sel);
        // SAFETY: objc_msgSend is called through a pointer of the method's own signature, which the selector names,
        // on an object (or nil, which takes any message) that has it.
        unsafe {
            let f: unsafe extern "C" fn($crate::objc::Id, $crate::objc::Sel $(, $t)*) -> $ret =
                std::mem::transmute($crate::objc::objc_msgSend as unsafe extern "C" fn());
            f(receiver, selector $(, $arg)*)
        }
    }};
}
pub(crate) use msg;

/// A method that returns a rectangle (`bounds`, `frame`, `visibleFrame`): returned in memory on x86-64, in
/// registers on ARM64.
pub(crate) fn frame_of(obj: Id, name: &str) -> Frame {
    #[cfg(target_arch = "aarch64")]
    {
        msg!(Frame; obj, name)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut frame = Frame::default();
        // SAFETY: objc_msgSend_stret as a method returning an NSRect is called on x86-64, the result written to it.
        unsafe {
            let f: unsafe extern "C" fn(*mut Frame, Id, Sel) = std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
            f(&mut frame, obj, sel(name));
        }
        frame
    }
}

/// A method that takes a rectangle and returns one (`contentRectForFrameRect:`, `frameRectForContentRect:`).
pub(crate) fn frame_for(obj: Id, name: &str, arg: Frame) -> Frame {
    #[cfg(target_arch = "aarch64")]
    {
        msg!(Frame; obj, name, arg => Frame)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let mut frame = Frame::default();
        // SAFETY: objc_msgSend_stret as a method taking and returning an NSRect is called on x86-64.
        unsafe {
            let f: unsafe extern "C" fn(*mut Frame, Id, Sel, Frame) = std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
            f(&mut frame, obj, sel(name), arg);
        }
        frame
    }
}

/// An `NSString`'s text (or an `NSAttributedString`'s), as Rust's; empty for nil.
pub(crate) fn rust_string(obj: Id) -> String {
    if obj.is_null() {
        return String::new();
    }
    let obj = if msg!(bool; obj, "isKindOfClass:", class("NSAttributedString") => Id) { msg!(Id; obj, "string") } else { obj };
    let utf8 = msg!(*const c_char; obj, "UTF8String");
    if utf8.is_null() {
        return String::new();
    }
    // SAFETY: a NUL-terminated string the object owns, copied before anything can let it go.
    unsafe { std::ffi::CStr::from_ptr(utf8) }.to_string_lossy().into_owned()
}

pub(crate) fn ns_string(text: &str) -> Id {
    let text = CString::new(text.replace('\0', "")).unwrap_or_default();
    msg!(Id; class("NSString"), "stringWithUTF8String:", text.as_ptr() => *const c_char)
}

/// Runs a block with an autorelease pool of its own.
pub(crate) fn pooled<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: a pool pushed and popped around the block, on one thread.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let r = f();
    // SAFETY: the pool just pushed.
    unsafe { objc_autoreleasePoolPop(pool) };
    r
}

pub(crate) fn is_main_thread() -> bool {
    // SAFETY: asks whether this is the process's main thread.
    unsafe { pthread_main_np() != 0 }
}

pub(crate) fn app() -> Id {
    msg!(Id; class("NSApplication"), "sharedApplication")
}

/// The app made a regular one (a Dock icon, windows that take the keyboard), and launched.
pub(crate) fn start_app() -> Result<Id, String> {
    let app = app();
    if app.is_null() {
        return Err("AppKit isn't there".into());
    }
    msg!(bool; app, "setActivationPolicy:", 0isize => isize);
    msg!((); app, "finishLaunching");
    Ok(app)
}

/// Wakes the event loop from any thread, once until it next takes an event: an application-defined event posted to
/// the queue, as GLFW's `glfwPostEmptyEvent` does.
pub(crate) fn wake_loop(posted: &AtomicBool) {
    if posted.swap(true, Ordering::AcqRel) {
        return;
    }
    pooled(|| {
        let event = msg!(Id; class("NSEvent"), "otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:",
            15usize => usize, Point::default() => Point, 0usize => usize, 0.0f64 => f64, 0isize => isize, NIL => Id, 0i16 => i16, 0isize => isize, 0isize => isize);
        msg!((); app(), "postEvent:atStart:", event => Id, false => bool);
    });
}

/// Takes events for up to `wait`, then every one queued: each given to `take` first, and to AppKit if `take` leaves it.
pub(crate) fn pump(app: Id, wait: Duration, posted: &AtomicBool, mut take: impl FnMut(Id) -> bool) {
    // SAFETY: Foundation's constant string.
    let mode = unsafe { NSDefaultRunLoopMode };
    let mut until = msg!(Id; class("NSDate"), "dateWithTimeIntervalSinceNow:", wait.as_secs_f64() => f64);
    loop {
        let event = msg!(Id; app, "nextEventMatchingMask:untilDate:inMode:dequeue:", u64::MAX => u64, until => Id, mode => Id, true => bool);
        if event.is_null() {
            break;
        }
        posted.store(false, Ordering::Release);
        if !take(event) {
            msg!((); app, "sendEvent:", event => Id);
        }
        until = msg!(Id; class("NSDate"), "distantPast");
    }
    msg!((); app, "updateWindows");
}

/// Refreshes a second of a screen, as it says (`maximumFramesPerSecond`, macOS 12 on), else 60.
pub(crate) fn refresh_of(screen: Id) -> u32 {
    if !screen.is_null() && msg!(bool; screen, "respondsToSelector:", sel("maximumFramesPerSecond") => Sel) {
        msg!(isize; screen, "maximumFramesPerSecond").clamp(24, 240) as u32
    } else {
        60
    }
}

/// Where a mouse event was, on the screen: the main display's top left the origin, in points, whichever window it
/// was for and wherever that's moved since.
pub(crate) fn global_location(event: Id) -> Point {
    let cg = msg!(*mut c_void; event, "CGEvent");
    if cg.is_null() {
        return Point::default();
    }
    // SAFETY: the event's own CGEvent, alive as long as it is.
    unsafe { CGEventGetLocation(cg) }
}

/// A view made layer-backed, its layer showing images we give it, magnified pixel for pixel. The layer.
pub(crate) fn layer_for(view: Id) -> Id {
    msg!((); view, "setWantsLayer:", true => bool);
    // NSViewLayerContentsRedrawNever: the layer's contents are ours.
    msg!((); view, "setLayerContentsRedrawPolicy:", 0isize => isize);
    let layer = msg!(Id; view, "layer");
    // SAFETY: Core Animation's constant strings.
    let (gravity, nearest) = unsafe { (kCAGravityResize, kCAFilterNearest) };
    msg!((); layer, "setContentsGravity:", gravity => Id);
    msg!((); layer, "setMagnificationFilter:", nearest => Id);
    layer
}

/// Makes images of pixels for layers.
pub(crate) struct Images {
    space: *mut c_void,
}

impl Images {
    pub fn new() -> Images {
        // SAFETY: a colour space, released when this is dropped.
        Images { space: unsafe { CGColorSpaceCreateDeviceRGB() } }
    }

    /// An image of a copy of `pixels` (`width` by `height`, opaque `0x00RRGGBB` or premultiplied `0xAARRGGBB`), for
    /// the caller to release (`release_image`); null if they're not that many.
    pub fn image(&self, pixels: &[u32], width: i32, height: i32, alpha: bool) -> *mut c_void {
        if width <= 0 || height <= 0 || pixels.len() < (width * height) as usize {
            return std::ptr::null_mut();
        }
        let info = if alpha { ARGB_LITTLE } else { XRGB_LITTLE };
        // SAFETY: the pixels copied into a CFData and an image made over it; the image holds what it needs.
        unsafe {
            let data = CFDataCreate(std::ptr::null(), pixels.as_ptr().cast(), (width * height * 4) as isize);
            let provider = CGDataProviderCreateWithCFData(data);
            let image = CGImageCreate(width as usize, height as usize, 8, 32, width as usize * 4, self.space, info, provider, std::ptr::null(), false, 0);
            CGDataProviderRelease(provider);
            CFRelease(data);
            image
        }
    }

    /// `pixels` (`0x00RRGGBB`, `width` by `height`) as a layer's contents: an image of a copy of them, so they never
    /// change under Core Animation.
    pub fn show(&self, layer: Id, pixels: &[u32], width: i32, height: i32) {
        let image = self.image(pixels, width, height, false);
        if image.is_null() {
            return;
        }
        msg!((); class("CATransaction"), "begin");
        msg!((); class("CATransaction"), "setDisableActions:", true => bool);
        msg!((); layer, "setContents:", image => Id);
        msg!((); class("CATransaction"), "commit");
        release_image(image);
    }
}

/// Lets go of an image `Images::image` made.
pub(crate) fn release_image(image: *mut c_void) {
    if !image.is_null() {
        // SAFETY: an image made by `Images::image`, released once; whatever holds it holds its own reference.
        unsafe { CGImageRelease(image) };
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        // SAFETY: the colour space made in `new`, once.
        unsafe { CGColorSpaceRelease(self.space) };
    }
}

/// A class of our own, made once: `methods` are each a selector, a function, and its type encoding; `protocols` it says
/// it conforms to. Nil if it can't be.
pub(crate) fn define_class(name: &str, superclass: &str, methods: &[(&str, *const c_void, &str)], protocols: &[&str]) -> Id {
    let existing = class(name);
    if !existing.is_null() {
        return existing;
    }
    let cname = CString::new(name).unwrap_or_default();
    // SAFETY: a new class under an existing one, given its methods, then registered, all before it's used.
    unsafe {
        let cls = objc_allocateClassPair(class(superclass), cname.as_ptr(), 0);
        if cls.is_null() {
            return NIL;
        }
        for (selector, imp, types) in methods {
            let types = CString::new(*types).unwrap_or_default();
            class_addMethod(cls, sel(selector), *imp, types.as_ptr());
        }
        for protocol in protocols {
            let pname = CString::new(*protocol).unwrap_or_default();
            let p = objc_getProtocol(pname.as_ptr());
            if !p.is_null() {
                class_addProtocol(cls, p);
            }
        }
        objc_registerClassPair(cls);
        cls
    }
}

/// `BOOL`'s type encoding: `bool` on ARM64, `signed char` on x86-64.
#[cfg(target_arch = "aarch64")]
pub(crate) const BOOL: &str = "B";
#[cfg(target_arch = "x86_64")]
pub(crate) const BOOL: &str = "c";
