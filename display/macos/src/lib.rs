//! The desktop on a Mac's screen. Desktop mode: the whole desktop in one `NSWindow`, as the composited screen any
//! observer reads (`Observed::screen` says what to show again), with the Mac's keyboard, mouse and trackpad going in
//! through `display::desktop::send`, the window's size as the output's, and the screen's refresh as the frame clock.
//!
//! AppKit is reached through the Objective-C runtime itself, as `exec/macos` reaches the hypervisor: no crates. It
//! wants the main thread, so [`run`] takes it, and the machine's kernel runs on another.

pub mod map;

#[cfg(target_os = "macos")]
mod clipboard;
#[cfg(target_os = "macos")]
mod desktop_mode;
#[cfg(target_os = "macos")]
mod ime;
#[cfg(target_os = "macos")]
mod input;
#[cfg(target_os = "macos")]
mod objc;
#[cfg(target_os = "macos")]
mod windows_mode;

#[cfg(target_os = "macos")]
pub use desktop_mode::run;
#[cfg(target_os = "macos")]
pub use windows_mode::run_windows;

/// What [`run`] shows the desktop as.
#[derive(Debug, Clone)]
pub struct Options {
    /// The window's title.
    pub title: String,
    /// Its content's size, in points: the output's, in logical pixels.
    pub width: i32,
    pub height: i32,
}

/// Whether this host can show the desktop in a window of its own.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}
