//! Where a popup goes: placed against a rectangle on its parent, kept on the screen. These are Wayland's
//! `xdg_positioner` rules, which cover the others too: Windows' `TrackPopupMenu` (a point, flipped to stay on the
//! screen) and X11's override-redirect menus and tooltips (placed by the client, then kept within the screen).
//!
//! The anchor is a point on the anchor rectangle (an edge's middle, a corner, or its centre); the gravity is which way
//! the popup goes from it. If the popup doesn't fit, it's adjusted on each axis on its own, as asked, in order: flipped
//! to the other side (only if that fits), slid along until it does, then shrunk.

use crate::Rect;

/// A point on a rectangle, or a direction from one: an edge's middle, a corner, or (`Centre`) none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    #[default]
    Centre,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

/// Which way on each axis: -1 (left, up), 0, or 1 (right, down).
impl Edge {
    fn x(self) -> i32 {
        match self {
            Edge::Left | Edge::TopLeft | Edge::BottomLeft => -1,
            Edge::Right | Edge::TopRight | Edge::BottomRight => 1,
            _ => 0,
        }
    }

    fn y(self) -> i32 {
        match self {
            Edge::Top | Edge::TopLeft | Edge::TopRight => -1,
            Edge::Bottom | Edge::BottomLeft | Edge::BottomRight => 1,
            _ => 0,
        }
    }

    fn from(x: i32, y: i32) -> Edge {
        match (x, y) {
            (-1, -1) => Edge::TopLeft,
            (0, -1) => Edge::Top,
            (1, -1) => Edge::TopRight,
            (-1, 0) => Edge::Left,
            (1, 0) => Edge::Right,
            (-1, 1) => Edge::BottomLeft,
            (0, 1) => Edge::Bottom,
            (1, 1) => Edge::BottomRight,
            _ => Edge::Centre,
        }
    }
}

/// How a popup that doesn't fit may be adjusted: `xdg_positioner`'s constraint adjustments.
pub mod adjust {
    pub const SLIDE_X: u32 = 1;
    pub const SLIDE_Y: u32 = 2;
    pub const FLIP_X: u32 = 4;
    pub const FLIP_Y: u32 = 8;
    pub const RESIZE_X: u32 = 16;
    pub const RESIZE_Y: u32 = 32;
}

/// Where a popup goes, relative to its parent's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Positioner {
    /// The popup's size.
    pub size: (i32, i32),
    /// The rectangle it's placed against, in its parent's content coordinates (a menu's item, a button).
    pub anchor_rect: Rect,
    pub anchor: Edge,
    pub gravity: Edge,
    /// Moved by this, after it's placed (before it's adjusted).
    pub offset: (i32, i32),
    /// `adjust::*`: what may be done if it doesn't fit.
    pub adjust: u32,
}

impl Positioner {
    /// Its place on the screen, for a parent whose content is at `parent` (screen coordinates), kept within `bounds`.
    pub fn place(&self, parent: Rect, bounds: Rect) -> Rect {
        let at = |anchor: Edge, gravity: Edge| {
            let a = self.anchor_rect.offset(parent.left, parent.top);
            let point = |lo: i32, hi: i32, dir: i32| match dir {
                -1 => lo,
                1 => hi,
                _ => lo + (hi - lo) / 2,
            };
            let (px, py) = (point(a.left, a.right, anchor.x()), point(a.top, a.bottom, anchor.y()));
            let (w, h) = self.size;
            // The gravity says which way it extends from the point: right puts its left edge there.
            let left = match gravity.x() {
                -1 => px - w,
                1 => px,
                _ => px - w / 2,
            } + self.offset.0;
            let top = match gravity.y() {
                -1 => py - h,
                1 => py,
                _ => py - h / 2,
            } + self.offset.1;
            Rect::new(left, top, left + w, top + h)
        };
        let fits_x = |r: &Rect| r.left >= bounds.left && r.right <= bounds.right;
        let fits_y = |r: &Rect| r.top >= bounds.top && r.bottom <= bounds.bottom;
        let mut r = at(self.anchor, self.gravity);
        // Flipped, on an axis it doesn't fit, if that fits there.
        if !fits_x(&r) && self.adjust & adjust::FLIP_X != 0 {
            let flipped = at(Edge::from(-self.anchor.x(), self.anchor.y()), Edge::from(-self.gravity.x(), self.gravity.y()));
            if fits_x(&flipped) {
                r = Rect::new(flipped.left, r.top, flipped.right, r.bottom);
            }
        }
        if !fits_y(&r) && self.adjust & adjust::FLIP_Y != 0 {
            let flipped = at(Edge::from(self.anchor.x(), -self.anchor.y()), Edge::from(self.gravity.x(), -self.gravity.y()));
            if fits_y(&flipped) {
                r = Rect::new(r.left, flipped.top, r.right, flipped.bottom);
            }
        }
        // Slid along until it fits; one bigger than the bounds lines up with their start.
        if !fits_x(&r) && self.adjust & adjust::SLIDE_X != 0 {
            let dx = if r.right > bounds.right { bounds.right - r.right } else { 0 };
            let dx = if r.left + dx < bounds.left { bounds.left - r.left } else { dx };
            r = r.offset(dx, 0);
        }
        if !fits_y(&r) && self.adjust & adjust::SLIDE_Y != 0 {
            let dy = if r.bottom > bounds.bottom { bounds.bottom - r.bottom } else { 0 };
            let dy = if r.top + dy < bounds.top { bounds.top - r.top } else { dy };
            r = r.offset(0, dy);
        }
        // Shrunk to what's within them.
        if !fits_x(&r) && self.adjust & adjust::RESIZE_X != 0 {
            r = Rect::new(r.left.max(bounds.left), r.top, r.right.min(bounds.right), r.bottom);
        }
        if !fits_y(&r) && self.adjust & adjust::RESIZE_Y != 0 {
            r = Rect::new(r.left, r.top.max(bounds.top), r.right, r.bottom.min(bounds.bottom));
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { left: 0, top: 0, right: 800, bottom: 600 };

    /// A 100 by 150 menu below an item, as a menu bar opens one.
    fn menu(item: Rect, adjust: u32) -> Positioner {
        Positioner { size: (100, 150), anchor_rect: item, anchor: Edge::BottomLeft, gravity: Edge::BottomRight, offset: (0, 0), adjust }
    }

    #[test]
    fn anchor_and_gravity() {
        let parent = Rect::new(100, 50, 500, 400);
        assert_eq!(menu(Rect::new(10, 0, 60, 20), 0).place(parent, SCREEN), Rect::new(110, 70, 210, 220));
        // Centred under its anchor's middle, moved by its offset.
        let p = Positioner { size: (40, 10), anchor_rect: Rect::new(0, 0, 100, 20), anchor: Edge::Bottom, gravity: Edge::Bottom, offset: (0, 2), adjust: 0 };
        assert_eq!(p.place(parent, SCREEN), Rect::new(130, 72, 170, 82));
    }

    #[test]
    fn flipped_when_it_would_go_off_the_screen() {
        // Near the bottom: above the item instead.
        let parent = Rect::new(0, 400, 400, 600);
        let r = menu(Rect::new(10, 150, 60, 170), adjust::FLIP_Y | adjust::SLIDE_Y).place(parent, SCREEN);
        assert_eq!(r, Rect::new(10, 400, 110, 550));
        // Not flipped if that doesn't fit either: slid instead.
        let tall = Positioner { size: (100, 560), ..menu(Rect::new(10, 150, 60, 170), adjust::FLIP_Y | adjust::SLIDE_Y) };
        assert_eq!(tall.place(parent, SCREEN), Rect::new(10, 40, 110, 600));
    }

    #[test]
    fn slid_then_resized() {
        let parent = Rect::new(700, 0, 800, 100);
        // Off the right edge: slid back on.
        assert_eq!(menu(Rect::new(50, 0, 90, 20), adjust::SLIDE_X).place(parent, SCREEN), Rect::new(700, 20, 800, 170));
        // Not allowed to move: left as it is.
        assert_eq!(menu(Rect::new(50, 0, 90, 20), 0).place(parent, SCREEN), Rect::new(750, 20, 850, 170));
        // Wider than the screen: slid to its left edge, then shrunk.
        let wide = Positioner { size: (900, 50), ..menu(Rect::new(50, 0, 90, 20), adjust::SLIDE_X | adjust::RESIZE_X) };
        assert_eq!(wide.place(parent, SCREEN), Rect::new(0, 20, 800, 70));
    }
}
