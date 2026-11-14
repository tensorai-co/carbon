//! Window-system integration: surfaces and swapchains, presented the way Mesa's software drivers present. The driver
//! speaks the program's own window system over the program's own connection: on Wayland, `wl_shm` buffers attached to
//! its surface (through the program's `libwayland-client`, on an event queue of the driver's, paced by frame
//! callbacks); on X11, `PutImage` into its window (through its `libxcb`; Xlib's window reaches it through
//! `XGetXCBConnection`). A present has the host read the swapchain image back into the buffer's memory first.
//!
//! The loader makes the surfaces (`VkIcdSurfaceWayland`, `Xcb`, `Xlib`), and the driver reads them. The window
//! system's libraries are the program's: `dlopen` by name gives the copy it has loaded (vkcube loads libwayland itself,
//! privately, so it isn't among the program's global symbols), and the driver links none of them.

// The window systems' functions are found by name and given the type of the field they're put in.
#![allow(clippy::missing_transmute_annotations)]

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use gpu_wire::records;
use gpu_wire::vk::*;

use crate::device::{Device, Image, device, signal_fence};
use crate::{mmap, munmap, write_list};

unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn memfd_create(name: *const c_char, flags: u32) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn free(p: *mut c_void);
}

const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const MAP_PRIVATE: c_int = 2;
const MAP_ANONYMOUS: c_int = 0x20;
const MFD_CLOEXEC: u32 = 1;
const RTLD_NOW: c_int = 2;
const RTLD_NOLOAD: c_int = 4;

const WAYLAND: &CStr = c"libwayland-client.so.0";
const XCB: &CStr = c"libxcb.so.1";
const X11_XCB: &CStr = c"libX11-xcb.so.1";

/// `VkIcdWsiPlatform`'s values, as `vk_icd.h` has them.
const PLATFORM_WAYLAND: c_int = 1;
const PLATFORM_XCB: c_int = 3;
const PLATFORM_XLIB: c_int = 4;

#[repr(C)]
struct SurfaceWayland {
    platform: c_int,
    display: *mut c_void,
    surface: *mut c_void,
}

#[repr(C)]
struct SurfaceXcb {
    platform: c_int,
    connection: *mut c_void,
    window: u32,
}

#[repr(C)]
struct SurfaceXlib {
    platform: c_int,
    display: *mut c_void,
    window: c_ulong,
}

/// A symbol of a library, the copy the program has loaded if it has (or else loaded now).
fn sym(library: &CStr, name: &CStr) -> *mut c_void {
    unsafe {
        let mut handle = dlopen(library.as_ptr(), RTLD_NOW | RTLD_NOLOAD);
        if handle.is_null() {
            handle = dlopen(library.as_ptr(), RTLD_NOW);
        }
        if handle.is_null() {
            return ptr::null_mut();
        }
        dlsym(handle, name.as_ptr())
    }
}

macro_rules! function {
    ($library:expr, $name:literal) => {{
        let f = sym($library, $name);
        if f.is_null() {
            return None;
        }
        unsafe { core::mem::transmute::<*mut c_void, _>(f) }
    }};
}

/// Where a surface's frames go.
#[derive(Clone, Copy)]
enum Window {
    Wayland { display: *mut c_void, surface: *mut c_void },
    Xcb { connection: *mut c_void, window: u32 },
}

fn window(surface: VkSurfaceKHR) -> Option<Window> {
    let platform = unsafe { *(surface as *const c_int) };
    match platform {
        PLATFORM_WAYLAND => {
            let s = unsafe { &*(surface as *const SurfaceWayland) };
            Some(Window::Wayland { display: s.display, surface: s.surface })
        }
        PLATFORM_XCB => {
            let s = unsafe { &*(surface as *const SurfaceXcb) };
            Some(Window::Xcb { connection: s.connection, window: s.window })
        }
        PLATFORM_XLIB => {
            let s = unsafe { &*(surface as *const SurfaceXlib) };
            let xcb: unsafe extern "C" fn(*mut c_void) -> *mut c_void = function!(X11_XCB, c"XGetXCBConnection");
            Some(Window::Xcb { connection: unsafe { xcb(s.display) }, window: s.window as u32 })
        }
        _ => None,
    }
}

// X11, through libxcb.

struct Xcb {
    generate_id: unsafe extern "C" fn(*mut c_void) -> u32,
    create_gc: unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const u32) -> u32,
    put_image: unsafe extern "C" fn(*mut c_void, u8, u32, u32, u16, u16, i16, i16, u8, u8, u32, *const u8) -> u32,
    flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    max_request: unsafe extern "C" fn(*mut c_void) -> u32,
}

impl Xcb {
    fn load() -> Option<Xcb> {
        Some(Xcb {
            generate_id: function!(XCB, c"xcb_generate_id"),
            create_gc: function!(XCB, c"xcb_create_gc"),
            put_image: function!(XCB, c"xcb_put_image"),
            flush: function!(XCB, c"xcb_flush"),
            max_request: function!(XCB, c"xcb_get_maximum_request_length"),
        })
    }
}

/// A window's size and depth: `xcb_get_geometry`'s reply.
fn xcb_geometry(connection: *mut c_void, window: u32) -> Option<(u32, u32, u8)> {
    let get: unsafe extern "C" fn(*mut c_void, u32) -> u32 = function!(XCB, c"xcb_get_geometry");
    let reply: unsafe extern "C" fn(*mut c_void, u32, *mut *mut c_void) -> *mut u8 = function!(XCB, c"xcb_get_geometry_reply");
    let r = unsafe { reply(connection, get(connection, window), ptr::null_mut()) };
    if r.is_null() {
        return None;
    }
    // response_type, depth, sequence, length, root, x, y, width, height, border_width.
    let (depth, width, height) = unsafe { (*r.add(1), u16::from_le_bytes([*r.add(16), *r.add(17)]), u16::from_le_bytes([*r.add(18), *r.add(19)])) };
    unsafe { free(r as *mut c_void) };
    Some((u32::from(width), u32::from(height), depth))
}

// Wayland, through libwayland-client.

struct Wl {
    create_queue: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    queue_destroy: unsafe extern "C" fn(*mut c_void),
    create_wrapper: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    wrapper_destroy: unsafe extern "C" fn(*mut c_void),
    set_queue: unsafe extern "C" fn(*mut c_void, *mut c_void),
    marshal: unsafe extern "C" fn(*mut c_void, u32, *const c_void, u32, u32, ...) -> *mut c_void,
    add_listener: unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int,
    destroy: unsafe extern "C" fn(*mut c_void),
    version: unsafe extern "C" fn(*mut c_void) -> u32,
    roundtrip_queue: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    dispatch_queue: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    registry: *const c_void,
    shm: *const c_void,
    pool: *const c_void,
    buffer: *const c_void,
    callback: *const c_void,
}

const MARSHAL_DESTROY: u32 = 1;
const DISPLAY_GET_REGISTRY: u32 = 1;
const REGISTRY_BIND: u32 = 0;
const SHM_CREATE_POOL: u32 = 0;
const POOL_CREATE_BUFFER: u32 = 0;
const POOL_DESTROY: u32 = 1;
const BUFFER_DESTROY: u32 = 0;
const SURFACE_ATTACH: u32 = 1;
const SURFACE_DAMAGE: u32 = 2;
const SURFACE_FRAME: u32 = 3;
const SURFACE_COMMIT: u32 = 6;
/// `wl_shm`'s XRGB8888: B, G, R and an unused byte in memory, Vulkan's B8G8R8A8.
const SHM_XRGB8888: u32 = 1;

impl Wl {
    fn load() -> Option<Wl> {
        let data = |name: &CStr| {
            let p = sym(WAYLAND, name);
            if p.is_null() { None } else { Some(p as *const c_void) }
        };
        Some(Wl {
            create_queue: function!(WAYLAND, c"wl_display_create_queue"),
            queue_destroy: function!(WAYLAND, c"wl_event_queue_destroy"),
            create_wrapper: function!(WAYLAND, c"wl_proxy_create_wrapper"),
            wrapper_destroy: function!(WAYLAND, c"wl_proxy_wrapper_destroy"),
            set_queue: function!(WAYLAND, c"wl_proxy_set_queue"),
            marshal: function!(WAYLAND, c"wl_proxy_marshal_flags"),
            add_listener: function!(WAYLAND, c"wl_proxy_add_listener"),
            destroy: function!(WAYLAND, c"wl_proxy_destroy"),
            version: function!(WAYLAND, c"wl_proxy_get_version"),
            roundtrip_queue: function!(WAYLAND, c"wl_display_roundtrip_queue"),
            dispatch_queue: function!(WAYLAND, c"wl_display_dispatch_queue"),
            flush: function!(WAYLAND, c"wl_display_flush"),
            registry: data(c"wl_registry_interface")?,
            shm: data(c"wl_shm_interface")?,
            pool: data(c"wl_shm_pool_interface")?,
            buffer: data(c"wl_buffer_interface")?,
            callback: data(c"wl_callback_interface")?,
        })
    }
}

#[repr(C)]
struct RegistryListener {
    global: unsafe extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32),
    global_remove: unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
}

#[repr(C)]
struct CallbackListener {
    done: unsafe extern "C" fn(*mut c_void, *mut c_void, u32),
}

/// What the registry's globals are looked through for.
struct Binding<'a> {
    wl: &'a Wl,
    shm: *mut c_void,
}

unsafe extern "C" fn global(data: *mut c_void, registry: *mut c_void, name: u32, interface: *const c_char, _version: u32) {
    let b = unsafe { &mut *(data as *mut Binding<'_>) };
    if b.shm.is_null() && unsafe { CStr::from_ptr(interface) }.to_bytes() == b"wl_shm" {
        // A wl_interface's first member is its name.
        let iface_name = unsafe { *(b.wl.shm as *const *const c_char) };
        b.shm = unsafe { (b.wl.marshal)(registry, REGISTRY_BIND, b.wl.shm, 1, 0, name, iface_name, 1u32, ptr::null_mut::<c_void>()) };
    }
}

unsafe extern "C" fn global_remove(_data: *mut c_void, _registry: *mut c_void, _name: u32) {}

static REGISTRY_LISTENER: RegistryListener = RegistryListener { global, global_remove };

/// A frame callback's `done`: the compositor's shown the last frame, so another may go. It's told the swapchain's
/// flag alone (boxed, and atomic), which the present waiting in `wl_display_dispatch_queue` reads again each time,
/// and the callback's `wl_proxy_destroy` as the flag's neighbour.
unsafe extern "C" fn frame_done(data: *mut c_void, callback: *mut c_void, _serial: u32) {
    let pending = unsafe { &*(data as *const Pending) };
    pending.flag.store(false, Ordering::Release);
    pending.callback.store(ptr::null_mut(), Ordering::Release);
    unsafe { (pending.destroy)(callback) };
}

/// Whether a frame callback is still to come, the callback (destroyed with the swapchain if it hasn't come: its queue
/// goes too), and how to destroy one.
struct Pending {
    flag: AtomicBool,
    callback: AtomicPtr<c_void>,
    destroy: unsafe extern "C" fn(*mut c_void),
}

static CALLBACK_LISTENER: CallbackListener = CallbackListener { done: frame_done };

// Surfaces, as a physical device sees them.

/// The formats a swapchain's images can be: B, G, R, A in memory, as both window systems take 32-bit pixels.
const FORMATS: [VkFormat; 2] = [VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_B8G8R8A8_SRGB];

pub unsafe extern "C" fn vkGetPhysicalDeviceSurfaceSupportKHR(_p: VkPhysicalDevice, family: u32, surface: VkSurfaceKHR, out: *mut VkBool32) -> VkResult {
    unsafe { *out = (family == 0 && window(surface).is_some()) as VkBool32 };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkGetPhysicalDeviceSurfaceCapabilitiesKHR(_p: VkPhysicalDevice, surface: VkSurfaceKHR, out: *mut VkSurfaceCapabilitiesKHR) -> VkResult {
    // A Wayland surface is the size the program makes its buffers; an X11 window's size is its own.
    let current = match window(surface) {
        Some(Window::Xcb { connection, window }) => xcb_geometry(connection, window).map_or(VkExtent2D { width: u32::MAX, height: u32::MAX }, |(w, h, _)| VkExtent2D { width: w, height: h }),
        Some(Window::Wayland { .. }) => VkExtent2D { width: u32::MAX, height: u32::MAX },
        None => return VK_ERROR_SURFACE_LOST_KHR,
    };
    unsafe {
        *out = VkSurfaceCapabilitiesKHR {
            minImageCount: 2,
            maxImageCount: 8,
            currentExtent: current,
            minImageExtent: VkExtent2D { width: 1, height: 1 },
            maxImageExtent: VkExtent2D { width: 8192, height: 8192 },
            maxImageArrayLayers: 1,
            supportedTransforms: VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR,
            currentTransform: VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR,
            supportedCompositeAlpha: VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR | VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR,
            supportedUsageFlags: VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_SAMPLED_BIT,
        }
    };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkGetPhysicalDeviceSurfaceFormatsKHR(_p: VkPhysicalDevice, _surface: VkSurfaceKHR, count: *mut u32, out: *mut VkSurfaceFormatKHR) -> VkResult {
    let list = FORMATS.map(|format| VkSurfaceFormatKHR { format, colorSpace: VK_COLOR_SPACE_SRGB_NONLINEAR_KHR });
    unsafe { write_list(&list, count, out) }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceSurfacePresentModesKHR(_p: VkPhysicalDevice, _surface: VkSurfaceKHR, count: *mut u32, out: *mut VkPresentModeKHR) -> VkResult {
    unsafe { write_list(&[VK_PRESENT_MODE_FIFO_KHR], count, out) }
}

pub unsafe extern "C" fn vkGetPhysicalDeviceWaylandPresentationSupportKHR(_p: VkPhysicalDevice, family: u32, _display: *mut c_void) -> VkBool32 {
    (family == 0) as VkBool32
}

pub unsafe extern "C" fn vkGetPhysicalDeviceXcbPresentationSupportKHR(_p: VkPhysicalDevice, family: u32, _connection: *mut c_void, _visual: u32) -> VkBool32 {
    (family == 0) as VkBool32
}

pub unsafe extern "C" fn vkGetPhysicalDeviceXlibPresentationSupportKHR(_p: VkPhysicalDevice, family: u32, _display: *mut c_void, _visual: c_ulong) -> VkBool32 {
    (family == 0) as VkBool32
}

// Swapchains.

/// Where one image is read back to for presenting: memory the host knows (its handle), mapped here, and on Wayland
/// the `wl_buffer` over it.
struct Target {
    memory: u64,
    at: *mut u8,
    size: usize,
    buffer: *mut c_void,
}

struct Swapchain {
    device: *const Device,
    window: Window,
    width: u32,
    height: u32,
    images: Vec<u64>,
    targets: Vec<Target>,
    next: usize,
    wl: Option<Wl>,
    /// Wayland: the driver's event queue, its wrapper of the surface on that queue, and whether a frame callback is
    /// still to come.
    queue: *mut c_void,
    surface: *mut c_void,
    shm: *mut c_void,
    frame_pending: Option<Box<Pending>>,
    xcb: Option<Xcb>,
    gc: u32,
    depth: u8,
}

/// Memory the host reads an image back into, mapped here: a memfd's, shared, for Wayland's `wl_shm`; anonymous
/// memory for X11.
fn target_memory(device: &Device, size: usize, shared: bool) -> Option<(u64, *mut u8, c_int)> {
    let (fd, flags) = if shared {
        let fd = unsafe { memfd_create(c"carbon-vulkan-present".as_ptr(), MFD_CLOEXEC) };
        if fd < 0 || unsafe { ftruncate(fd, size as i64) } != 0 {
            return None;
        }
        (fd, MAP_SHARED)
    } else {
        (-1, MAP_PRIVATE | MAP_ANONYMOUS)
    };
    let at = unsafe { mmap(ptr::null_mut(), size, PROT_READ | PROT_WRITE, flags, fd, 0) };
    if at as isize == -1 {
        return None;
    }
    let id = Box::into_raw(Box::new(0u8)) as u64;
    let mut p = Vec::new();
    records::put_u64(&mut p, id);
    records::put_u64(&mut p, size as u64);
    records::put_u64(&mut p, at as u64);
    device.record(records::MEMORY, &p);
    Some((id, at as *mut u8, fd))
}

impl Swapchain {
    /// The driver's way onto a Wayland surface: a queue of its own, the surface on it, `wl_shm` from the registry.
    fn wayland(&mut self, display: *mut c_void, surface: *mut c_void) -> Option<*mut c_void> {
        let wl = Wl::load()?;
        unsafe {
            self.queue = (wl.create_queue)(display);
            let wrapper = (wl.create_wrapper)(display);
            (wl.set_queue)(wrapper, self.queue);
            let registry = (wl.marshal)(wrapper, DISPLAY_GET_REGISTRY, wl.registry, (wl.version)(wrapper), 0, ptr::null_mut::<c_void>());
            (wl.wrapper_destroy)(wrapper);
            let mut binding = Binding { wl: &wl, shm: ptr::null_mut() };
            (wl.add_listener)(registry, &REGISTRY_LISTENER as *const RegistryListener as *const c_void, &mut binding as *mut Binding<'_> as *mut c_void);
            (wl.roundtrip_queue)(display, self.queue);
            (wl.destroy)(registry);
            let shm = binding.shm;
            self.shm = shm;
            self.surface = (wl.create_wrapper)(surface);
            (wl.set_queue)(self.surface, self.queue);
            self.frame_pending = Some(Box::new(Pending { flag: AtomicBool::new(false), callback: AtomicPtr::new(ptr::null_mut()), destroy: wl.destroy }));
            self.wl = Some(wl);
            (!shm.is_null()).then_some(shm)
        }
    }

    fn destroy(&mut self) {
        let device = unsafe { &*self.device };
        for t in self.targets.drain(..) {
            if let (Some(wl), false) = (&self.wl, t.buffer.is_null()) {
                unsafe { (wl.marshal)(t.buffer, BUFFER_DESTROY, ptr::null(), (wl.version)(t.buffer), MARSHAL_DESTROY) };
            }
            let mut p = Vec::new();
            records::put_u64(&mut p, t.memory);
            device.record(records::FREE_MEMORY, &p);
            unsafe { munmap(t.at as *mut c_void, t.size) };
            drop(unsafe { Box::from_raw(t.memory as *mut u8) });
        }
        for image in self.images.drain(..) {
            let mut p = Vec::new();
            records::put_u64(&mut p, image);
            device.record(records::FREE_IMAGE, &p);
            drop(unsafe { Box::from_raw(image as *mut Image) });
        }
        if let Some(wl) = &self.wl {
            unsafe {
                if let Some(pending) = &self.frame_pending {
                    let callback = pending.callback.swap(ptr::null_mut(), Ordering::AcqRel);
                    if !callback.is_null() {
                        (wl.destroy)(callback);
                    }
                }
                if !self.surface.is_null() {
                    (wl.wrapper_destroy)(self.surface);
                }
                if !self.shm.is_null() {
                    (wl.destroy)(self.shm);
                }
                if !self.queue.is_null() {
                    (wl.queue_destroy)(self.queue);
                }
            }
        }
    }
}

pub unsafe extern "C" fn vkCreateSwapchainKHR(d: VkDevice, info: *const VkSwapchainCreateInfoKHR, _a: *const VkAllocationCallbacks, out: *mut VkSwapchainKHR) -> VkResult {
    let info = unsafe { &*info };
    let device = unsafe { device(d) };
    let Some(window) = window(info.surface) else { return VK_ERROR_SURFACE_LOST_KHR };
    if !FORMATS.contains(&info.imageFormat) {
        return VK_ERROR_FORMAT_NOT_SUPPORTED;
    }
    let (width, height) = (info.imageExtent.width.max(1), info.imageExtent.height.max(1));
    let count = info.minImageCount.clamp(2, 8) as usize;
    let mut sc = Box::new(Swapchain {
        device,
        window,
        width,
        height,
        images: Vec::new(),
        targets: Vec::new(),
        next: 0,
        wl: None,
        queue: ptr::null_mut(),
        surface: ptr::null_mut(),
        shm: ptr::null_mut(),
        frame_pending: None,
        xcb: None,
        gc: 0,
        depth: 24,
    });
    let size = width as usize * height as usize * 4;
    let shm = match window {
        Window::Wayland { display, surface } => match sc.wayland(display, surface) {
            Some(shm) => Some(shm),
            None => return VK_ERROR_SURFACE_LOST_KHR,
        },
        Window::Xcb { connection, window } => {
            let Some(xcb) = Xcb::load() else { return VK_ERROR_SURFACE_LOST_KHR };
            sc.depth = xcb_geometry(connection, window).map_or(24, |g| g.2);
            sc.gc = unsafe { (xcb.generate_id)(connection) };
            unsafe { (xcb.create_gc)(connection, sc.gc, window, 0, ptr::null()) };
            sc.xcb = Some(xcb);
            None
        }
    };
    for _ in 0..count {
        let mut image = Box::new(Image::swapchain(info.imageFormat, width, height));
        let id = &*image as *const Image as u64;
        image.make(device, id, 0, 0);
        sc.images.push(Box::into_raw(image) as u64);
        let Some((memory, at, fd)) = target_memory(device, size, shm.is_some()) else {
            sc.destroy();
            return VK_ERROR_OUT_OF_HOST_MEMORY;
        };
        let mut buffer = ptr::null_mut();
        if let (Some(shm), Some(wl)) = (shm, &sc.wl) {
            unsafe {
                let pool = (wl.marshal)(shm, SHM_CREATE_POOL, wl.pool, (wl.version)(shm), 0, ptr::null_mut::<c_void>(), fd, size as i32);
                buffer = (wl.marshal)(pool, POOL_CREATE_BUFFER, wl.buffer, (wl.version)(pool), 0, ptr::null_mut::<c_void>(), 0i32, width as i32, height as i32, (width * 4) as i32, SHM_XRGB8888);
                (wl.marshal)(pool, POOL_DESTROY, ptr::null(), (wl.version)(pool), MARSHAL_DESTROY);
            }
        }
        if fd >= 0 {
            unsafe { close(fd) };
        }
        sc.targets.push(Target { memory, at, size, buffer });
    }
    unsafe { *out = Box::into_raw(sc) as VkSwapchainKHR };
    VK_SUCCESS
}

pub unsafe extern "C" fn vkDestroySwapchainKHR(_d: VkDevice, swapchain: VkSwapchainKHR, _a: *const VkAllocationCallbacks) {
    if swapchain != 0 {
        let mut sc = unsafe { Box::from_raw(swapchain as *mut Swapchain) };
        sc.destroy();
    }
}

pub unsafe extern "C" fn vkGetSwapchainImagesKHR(_d: VkDevice, swapchain: VkSwapchainKHR, count: *mut u32, out: *mut VkImage) -> VkResult {
    let sc = unsafe { &*(swapchain as *const Swapchain) };
    unsafe { write_list(&sc.images, count, out) }
}

/// The next image, in turn: work is finished when it's submitted, so any image is free by the time it's asked for.
pub unsafe extern "C" fn vkAcquireNextImageKHR(_d: VkDevice, swapchain: VkSwapchainKHR, _timeout: u64, _semaphore: VkSemaphore, fence: VkFence, index: *mut u32) -> VkResult {
    let sc = unsafe { &mut *(swapchain as *mut Swapchain) };
    unsafe { *index = sc.next as u32 };
    sc.next = (sc.next + 1) % sc.images.len();
    if fence != 0 {
        signal_fence(fence);
    }
    VK_SUCCESS
}

/// Reads one image back into its target and shows it in the window.
fn present(sc: &mut Swapchain, index: usize) -> VkResult {
    let device = unsafe { &*sc.device };
    let (Some(&image), Some(target)) = (sc.images.get(index), sc.targets.get(index)) else { return VK_ERROR_OUT_OF_DATE_KHR };
    let (memory, buffer, at) = (target.memory, target.buffer, target.at);
    let mut copy = Vec::new();
    records::put_u64(&mut copy, image);
    for v in [0, 0, sc.width, sc.height] {
        records::put_u32(&mut copy, v);
    }
    records::put_u64(&mut copy, memory);
    records::put_u64(&mut copy, 0);
    records::put_u32(&mut copy, sc.width);
    let mut work = Vec::new();
    records::record(&mut work, records::COPY_FROM_IMAGE, &copy);
    if device.ring(records::SUBMIT, &work).is_err() {
        return VK_ERROR_DEVICE_LOST;
    }
    match sc.window {
        Window::Wayland { display, surface } => {
            let (Some(wl), Some(pending)) = (&sc.wl, &sc.frame_pending) else { return VK_ERROR_SURFACE_LOST_KHR };
            unsafe {
                // FIFO: one frame to the compositor at a time, at its pace.
                while pending.flag.load(Ordering::Acquire) {
                    if (wl.dispatch_queue)(display, sc.queue) < 0 {
                        pending.flag.store(false, Ordering::Release);
                        return VK_ERROR_SURFACE_LOST_KHR;
                    }
                }
                let v = (wl.version)(surface);
                let callback = (wl.marshal)(sc.surface, SURFACE_FRAME, wl.callback, v, 0, ptr::null_mut::<c_void>());
                (wl.add_listener)(callback, &CALLBACK_LISTENER as *const CallbackListener as *const c_void, &**pending as *const Pending as *mut c_void);
                pending.callback.store(callback, Ordering::Release);
                pending.flag.store(true, Ordering::Release);
                (wl.marshal)(surface, SURFACE_ATTACH, ptr::null(), v, 0, buffer, 0i32, 0i32);
                (wl.marshal)(surface, SURFACE_DAMAGE, ptr::null(), v, 0, 0i32, 0i32, sc.width as i32, sc.height as i32);
                (wl.marshal)(surface, SURFACE_COMMIT, ptr::null(), v, 0);
                (wl.flush)(display);
            }
        }
        Window::Xcb { connection, window } => {
            let Some(xcb) = &sc.xcb else { return VK_ERROR_SURFACE_LOST_KHR };
            let row = sc.width as usize * 4;
            // As many rows a request as its largest size allows (in words, less PutImage's header).
            let most = (unsafe { (xcb.max_request)(connection) } as usize * 4).saturating_sub(32);
            let rows = (most / row).clamp(1, sc.height as usize);
            let mut y = 0;
            while y < sc.height as usize {
                let n = rows.min(sc.height as usize - y);
                unsafe {
                    (xcb.put_image)(connection, 2, window, sc.gc, sc.width as u16, n as u16, 0, y as i16, 0, sc.depth, (n * row) as u32, at.add(y * row));
                }
                y += n;
            }
            unsafe { (xcb.flush)(connection) };
        }
    }
    VK_SUCCESS
}

pub unsafe extern "C" fn vkQueuePresentKHR(_queue: VkQueue, info: *const VkPresentInfoKHR) -> VkResult {
    let info = unsafe { &*info };
    let mut result = VK_SUCCESS;
    for i in 0..info.swapchainCount as usize {
        let sc = unsafe { &mut *(*info.pSwapchains.add(i) as *mut Swapchain) };
        let r = present(sc, unsafe { *info.pImageIndices.add(i) } as usize);
        if !info.pResults.is_null() {
            unsafe { *info.pResults.add(i) = r };
        }
        if r != VK_SUCCESS {
            result = r;
        }
    }
    result
}

/// The swapchain's commands, by name (`VK_KHR_swapchain`, a device extension).
pub(crate) fn lookup(name: &[u8]) -> Option<*const ()> {
    Some(match name {
        b"vkCreateSwapchainKHR" => vkCreateSwapchainKHR as PFN_vkCreateSwapchainKHR as *const (),
        b"vkDestroySwapchainKHR" => vkDestroySwapchainKHR as PFN_vkDestroySwapchainKHR as *const (),
        b"vkGetSwapchainImagesKHR" => vkGetSwapchainImagesKHR as PFN_vkGetSwapchainImagesKHR as *const (),
        b"vkAcquireNextImageKHR" => vkAcquireNextImageKHR as PFN_vkAcquireNextImageKHR as *const (),
        b"vkQueuePresentKHR" => vkQueuePresentKHR as PFN_vkQueuePresentKHR as *const (),
        _ => return None,
    })
}
