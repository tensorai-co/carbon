//! The interfaces the server speaks, as far as it speaks them: each one's name, the version it offers, and its
//! requests' signatures by opcode, from `wayland.xml`, `xdg-shell.xml`, `cursor-shape-v1.xml` and
//! `text-input-unstable-v3.xml`, `relative-pointer-unstable-v1.xml`, `pointer-constraints-unstable-v1.xml`,
//! `xdg-activation-v1.xml` and `xdg-output-unstable-v1.xml`. Written here by hand for now; the to-do
//! has them made from the XML at build time.

/// An interface the server has objects of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    Display,
    Registry,
    Callback,
    Compositor,
    Region,
    Surface,
    Shm,
    ShmPool,
    Buffer,
    XdgWmBase,
    XdgPositioner,
    XdgSurface,
    XdgToplevel,
    XdgPopup,
    Seat,
    Pointer,
    Keyboard,
    Touch,
    Output,
    DecorationManager,
    ToplevelDecoration,
    Subcompositor,
    Subsurface,
    DataDeviceManager,
    DataSource,
    DataDevice,
    DataOffer,
    CursorShapeManager,
    CursorShapeDevice,
    TextInputManager,
    TextInput,
    RelativePointerManager,
    RelativePointer,
    PointerConstraints,
    LockedPointer,
    ConfinedPointer,
    Activation,
    ActivationToken,
    XdgOutputManager,
    XdgOutput,
}

impl Interface {
    pub fn name(self) -> &'static str {
        match self {
            Interface::Display => "wl_display",
            Interface::Registry => "wl_registry",
            Interface::Callback => "wl_callback",
            Interface::Compositor => "wl_compositor",
            Interface::Region => "wl_region",
            Interface::Surface => "wl_surface",
            Interface::Shm => "wl_shm",
            Interface::ShmPool => "wl_shm_pool",
            Interface::Buffer => "wl_buffer",
            Interface::XdgWmBase => "xdg_wm_base",
            Interface::XdgPositioner => "xdg_positioner",
            Interface::XdgSurface => "xdg_surface",
            Interface::XdgToplevel => "xdg_toplevel",
            Interface::XdgPopup => "xdg_popup",
            Interface::Seat => "wl_seat",
            Interface::Pointer => "wl_pointer",
            Interface::Keyboard => "wl_keyboard",
            Interface::Touch => "wl_touch",
            Interface::Output => "wl_output",
            Interface::DecorationManager => "zxdg_decoration_manager_v1",
            Interface::ToplevelDecoration => "zxdg_toplevel_decoration_v1",
            Interface::Subcompositor => "wl_subcompositor",
            Interface::Subsurface => "wl_subsurface",
            Interface::DataDeviceManager => "wl_data_device_manager",
            Interface::DataSource => "wl_data_source",
            Interface::DataDevice => "wl_data_device",
            Interface::DataOffer => "wl_data_offer",
            Interface::CursorShapeManager => "wp_cursor_shape_manager_v1",
            Interface::CursorShapeDevice => "wp_cursor_shape_device_v1",
            Interface::TextInputManager => "zwp_text_input_manager_v3",
            Interface::TextInput => "zwp_text_input_v3",
            Interface::RelativePointerManager => "zwp_relative_pointer_manager_v1",
            Interface::RelativePointer => "zwp_relative_pointer_v1",
            Interface::PointerConstraints => "zwp_pointer_constraints_v1",
            Interface::LockedPointer => "zwp_locked_pointer_v1",
            Interface::ConfinedPointer => "zwp_confined_pointer_v1",
            Interface::Activation => "xdg_activation_v1",
            Interface::ActivationToken => "xdg_activation_token_v1",
            Interface::XdgOutputManager => "zxdg_output_manager_v1",
            Interface::XdgOutput => "zxdg_output_v1",
        }
    }

    /// Each request's signature, by opcode.
    pub fn requests(self) -> &'static [&'static str] {
        match self {
            // sync, get_registry
            Interface::Display => &["n", "n"],
            // bind: a global's name, and an untyped new ID (its interface, its version, its ID)
            Interface::Registry => &["usun"],
            Interface::Callback => &[],
            // create_surface, create_region
            Interface::Compositor => &["n", "n"],
            // destroy, add, subtract
            Interface::Region => &["", "iiii", "iiii"],
            // destroy, attach, damage, frame, set_opaque_region, set_input_region, commit, set_buffer_transform,
            // set_buffer_scale, damage_buffer, offset
            Interface::Surface => &["", "?oii", "iiii", "n", "?o", "?o", "", "i", "i", "iiii", "ii"],
            // create_pool, release
            Interface::Shm => &["nhi", ""],
            // create_buffer, destroy, resize
            Interface::ShmPool => &["niiiiu", "", "i"],
            // destroy
            Interface::Buffer => &[""],
            // destroy, create_positioner, get_xdg_surface, pong
            Interface::XdgWmBase => &["", "n", "no", "u"],
            // destroy, set_size, set_anchor_rect, set_anchor, set_gravity, set_constraint_adjustment, set_offset,
            // set_reactive, set_parent_size, set_parent_configure
            Interface::XdgPositioner => &["", "ii", "iiii", "u", "u", "u", "ii", "", "ii", "u"],
            // destroy, get_toplevel, get_popup, set_window_geometry, ack_configure
            Interface::XdgSurface => &["", "n", "n?oo", "iiii", "u"],
            // destroy, set_parent, set_title, set_app_id, show_window_menu, move, resize, set_max_size, set_min_size,
            // set_maximized, unset_maximized, set_fullscreen, unset_fullscreen, set_minimized
            Interface::XdgToplevel => &["", "?o", "s", "s", "ouii", "ou", "ouu", "ii", "ii", "", "", "?o", "", ""],
            // destroy, grab, reposition
            Interface::XdgPopup => &["", "ou", "ou"],
            // get_pointer, get_keyboard, get_touch, release
            Interface::Seat => &["n", "n", "n", ""],
            // set_cursor, release
            Interface::Pointer => &["u?oii", ""],
            // release
            Interface::Keyboard | Interface::Touch | Interface::Output => &[""],
            // destroy, get_toplevel_decoration
            Interface::DecorationManager => &["", "no"],
            // destroy, set_mode, unset_mode
            Interface::ToplevelDecoration => &["", "u", ""],
            // destroy, get_subsurface
            Interface::Subcompositor => &["", "noo"],
            // destroy, set_position, place_above, place_below, set_sync, set_desync
            Interface::Subsurface => &["", "ii", "o", "o", "", ""],
            // create_data_source, get_data_device, release
            Interface::DataDeviceManager => &["n", "no", ""],
            // offer, destroy, set_actions
            Interface::DataSource => &["s", "", "u"],
            // start_drag, set_selection, release
            Interface::DataDevice => &["?oo?ou", "?ou", ""],
            // accept, receive, destroy, finish, set_actions
            Interface::DataOffer => &["u?s", "sh", "", "", "uu"],
            // destroy, get_pointer, get_tablet_tool_v2
            Interface::CursorShapeManager => &["", "no", "no"],
            // destroy, set_shape
            Interface::CursorShapeDevice => &["", "uu"],
            // destroy, get_text_input
            Interface::TextInputManager => &["", "no"],
            // destroy, enable, disable, set_surrounding_text, set_text_change_cause, set_content_type,
            // set_cursor_rectangle, commit
            Interface::TextInput => &["", "", "", "sii", "u", "uu", "iiii", ""],
            // destroy, get_relative_pointer
            Interface::RelativePointerManager => &["", "no"],
            // destroy
            Interface::RelativePointer | Interface::XdgOutput => &[""],
            // destroy, lock_pointer, confine_pointer
            Interface::PointerConstraints => &["", "noo?ou", "noo?ou"],
            // destroy, set_cursor_position_hint, set_region
            Interface::LockedPointer => &["", "ff", "?o"],
            // destroy, set_region
            Interface::ConfinedPointer => &["", "?o"],
            // destroy, get_activation_token, activate
            Interface::Activation => &["", "n", "so"],
            // set_serial, set_app_id, set_surface, commit, destroy
            Interface::ActivationToken => &["uo", "s", "o", "", ""],
            // destroy, get_xdg_output
            Interface::XdgOutputManager => &["", "no"],
        }
    }
}

/// The globals every client's registry lists: each one's interface and the version offered.
pub const GLOBALS: &[(Interface, u32)] =
    &[(Interface::Compositor, 6), (Interface::Shm, 1), (Interface::XdgWmBase, 6), (Interface::Seat, 8), (Interface::Output, 4), (Interface::DecorationManager, 1), (Interface::Subcompositor, 1), (Interface::DataDeviceManager, 3), (Interface::CursorShapeManager, 1), (Interface::TextInputManager, 1), (Interface::RelativePointerManager, 1), (Interface::PointerConstraints, 1), (Interface::Activation, 1), (Interface::XdgOutputManager, 3)];

/// `zwp_pointer_constraints_v1`'s lifetimes: let go for good once it's let go, or taken again each time it can be.
pub const ONESHOT: u32 = 1;
/// `zwp_pointer_constraints_v1`'s error for a surface already constrained by that pointer.
pub const ALREADY_CONSTRAINED: u32 = 1;

/// `wp_cursor_shape_device_v1`'s shapes, from 1: CSS's names, as the desktop's `Shape`s are named. (Version 2's
/// `dnd_ask` and `all_resize` aren't the desktop's, so version 1 is offered.)
pub const CURSOR_SHAPES: [&str; 34] = [
    "default", "context-menu", "help", "pointer", "progress", "wait", "cell", "crosshair", "text", "vertical-text", "alias", "copy", "move",
    "no-drop", "not-allowed", "grab", "grabbing", "e-resize", "n-resize", "ne-resize", "nw-resize", "s-resize", "se-resize", "sw-resize",
    "w-resize", "ew-resize", "ns-resize", "nesw-resize", "nwse-resize", "col-resize", "row-resize", "all-scroll", "zoom-in", "zoom-out",
];
/// `wp_cursor_shape_device_v1`'s error for a shape that isn't one.
pub const INVALID_SHAPE: u32 = 1;

/// `zxdg_toplevel_decoration_v1`'s modes.
pub const CLIENT_SIDE: u32 = 1;
pub const SERVER_SIDE: u32 = 2;

/// `wl_seat`'s capabilities: a pointer and a keyboard.
pub const POINTER: u32 = 1;
pub const KEYBOARD: u32 = 2;

/// `wl_keyboard.keymap`'s format: an XKB text keymap.
pub const XKB_V1: u32 = 1;

/// Linux's button codes, as `wl_pointer.button` gives them.
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// `wl_shm`'s formats: the two every compositor has, and the desktop's two.
pub const SHM_ARGB8888: u32 = 0;
pub const SHM_XRGB8888: u32 = 1;

/// `wl_display.error`'s codes.
pub const INVALID_OBJECT: u32 = 0;
pub const INVALID_METHOD: u32 = 1;
pub const IMPLEMENTATION: u32 = 3;
/// `xdg_surface`'s error for a window geometry with no size.
pub const INVALID_SIZE: u32 = 5;

/// `xdg_toplevel`'s states.
pub const MAXIMIZED: u32 = 1;
pub const FULLSCREEN: u32 = 2;
pub const RESIZING: u32 = 3;
pub const ACTIVATED: u32 = 4;
