//! Cursors: what the pointer looks like. A window sets one for its content: a shape by name, an image of its own with
//! a hotspot, or none. The names are CSS's, as Wayland's `wp_cursor_shape_v1` has them, which cover Windows' `IDC_*`
//! cursors and X11's cursor font too. The desktop picks its own over its frames (the resize arrows on their edges),
//! and whoever shows the desktop (a host, a viewer) draws it.

use crate::Surface;

/// A cursor shape, by its CSS name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Default,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

const NAMES: [(Shape, &str); 34] = [
    (Shape::Default, "default"),
    (Shape::ContextMenu, "context-menu"),
    (Shape::Help, "help"),
    (Shape::Pointer, "pointer"),
    (Shape::Progress, "progress"),
    (Shape::Wait, "wait"),
    (Shape::Cell, "cell"),
    (Shape::Crosshair, "crosshair"),
    (Shape::Text, "text"),
    (Shape::VerticalText, "vertical-text"),
    (Shape::Alias, "alias"),
    (Shape::Copy, "copy"),
    (Shape::Move, "move"),
    (Shape::NoDrop, "no-drop"),
    (Shape::NotAllowed, "not-allowed"),
    (Shape::Grab, "grab"),
    (Shape::Grabbing, "grabbing"),
    (Shape::EResize, "e-resize"),
    (Shape::NResize, "n-resize"),
    (Shape::NeResize, "ne-resize"),
    (Shape::NwResize, "nw-resize"),
    (Shape::SResize, "s-resize"),
    (Shape::SeResize, "se-resize"),
    (Shape::SwResize, "sw-resize"),
    (Shape::WResize, "w-resize"),
    (Shape::EwResize, "ew-resize"),
    (Shape::NsResize, "ns-resize"),
    (Shape::NeswResize, "nesw-resize"),
    (Shape::NwseResize, "nwse-resize"),
    (Shape::ColResize, "col-resize"),
    (Shape::RowResize, "row-resize"),
    (Shape::AllScroll, "all-scroll"),
    (Shape::ZoomIn, "zoom-in"),
    (Shape::ZoomOut, "zoom-out"),
];

impl Shape {
    /// Its CSS name.
    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(s, _)| *s == self).map_or("default", |(_, n)| n)
    }

    pub fn from_name(name: &str) -> Option<Shape> {
        NAMES.iter().find(|(_, n)| *n == name).map(|(s, _)| *s)
    }
}

/// What the pointer looks like over a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    Shape(Shape),
    /// A picture of its own, premultiplied `0xAARRGGBB`, with the point that's the pointer's place.
    Image { pixels: Surface, hotspot: (i32, i32) },
    Hidden,
}

impl Default for Cursor {
    fn default() -> Cursor {
        Cursor::Shape(Shape::Default)
    }
}
