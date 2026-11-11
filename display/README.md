# display

A machine's screen, whichever kernel draws on it: the desktop both kernels' windows go on, and the pixels they're
drawn in. Neither kernel writes a window manager of its own, and neither knows where its windows end up: on a screen
in memory, in the host's own windows, or (through [`remote`](../remote/README.md)) on another computer.

**First cut:** the shared part is built, with the desktop on a screen in memory. Showing it on the host is to do.

```
 atlas: win32k (windows, messages, GDI)        lambda: os/wayland, os/x11 (to do)
                     │                                       │
                     ▼                                       ▼
 display::Desktop   top-level windows: places, stacking, frames, which is active, input routing,
                    compositing into one screen with what changed since last time
                     │
     ┌───────────────┼───────────────────────┬──────────────────────┐
     ▼               ▼                       ▼                      ▼
 a screen in      the host's windows       remote's viewers       carbon display
 memory           (display/macos, …)       (observers)            screenshot, click, type
```

## What it has

- **`Surface`** (`lib.rs`): pixels, with drawing (rectangles, lines, pixels), text in its one font (5 by 7 in a 6 by 13
  cell, in `font.rs`), and PNG out. `Rect` and `Rgb` with it.
- **Keys** (`keys.rs`): Linux's evdev numbers, so events mean the same to both kernels, and to every host and viewer
  that maps its keyboard onto them once.
- **`Desktop`** (`desktop.rs`): the window manager. A kernel makes and destroys windows, sets their place, title,
  layer, owner and visibility, asks to raise or activate them, and draws into their content; it reads back `Event`s
  (moved or resized by their frames, activated or not, asked to close, and pointer and key input in content
  coordinates). Input comes in as `Input`, from the host, from `carbon display`, or from a program (`SendInput`), and
  `send` wakes the kernel to read what it made. `screen()` composites, `take_damage()` says what changed.

### What the desktop does, and what each kernel does

| The desktop does | Each kernel does |
| --- | --- |
| Top-level windows only: where they are, their stacking (a normal and an always-on-top layer, owned windows above their owners), and which one is active | Everything inside a top-level window: win32k's child windows, messages and `WM_NCHITTEST`; Wayland's subsurfaces and X11's subwindows |
| Frames: a title bar with the window's title, a close button, and edges to resize by. A window can go without (`WS_POPUP`, a Wayland client that decorates itself, X11's override-redirect for menus and tooltips) | What its API says a window's frame is: win32k's window and client rectangles, from the frame's insets |
| Input: a click goes to the window under it, in that window's coordinates, and activates it; keys go to the active window; a drag stays with the window it started on. Moving and resizing by the frame happen here, whole | Turning the desktop's events into its own: `WM_LBUTTONDOWN`, `WM_KEYDOWN`, `WM_ACTIVATE`, `WM_SIZE`; `wl_pointer` and `wl_keyboard`; X11's events |
| Compositing every window's pixels into one screen, with what changed since last time; screenshots and `carbon display`'s clicks and keys | Drawing into its windows' pixels: GDI in win32k, the client's buffers for Wayland and X11 |
| Policy the kernel asks for and the desktop carries out: raise, activate, keep on top, show and hide | Policy its API defines: Windows' foreground rules, `SetWindowPos`'s flags, `xdg_toplevel`'s states |

The desktop has no taskbar or shell: those are programs (Explorer, a Linux panel). Its frames have a plain look of
their own, not Windows' or GNOME's. Its lock is a leaf: nothing is taken after it.

## Two backends, one API

- **Built in**: the window manager above, on a screen in memory. For headless machines, and for showing a whole
  machine's desktop in one host window. This is what's built.
- **Host windows**: each top-level window gets a native window of the host's (an `NSWindow` on macOS), and the host's
  own window manager places, stacks, frames and activates them; the desktop passes its events through.

Like `audio`, `net` and `host`, it's a shared part with a folder per host, and every host-specific line inside it:

| Host | Shows the desktop with |
| --- | --- |
| macOS (`display/macos`) | AppKit: one window for the whole desktop, or an `NSWindow` per top-level window |
| Linux (`display/linux`) | the host's own display server |
| Windows (`display/windows`) | Win32 windows |

## On the command line

```sh
carbon run --display headless=1280x800 setup.exe   # a screen in memory (the default today)
carbon run --display none debian:13 -- bash        # no screen
carbon run --display host setup.exe                # the host's windows: to do
carbon display screenshot app -o screen.png        # a running machine's screen, as a PNG
```

`--display host` will be the default where the host has a screen. `carbon display click`, `type` and `key` are to do;
they're how an agent drives a machine, locally or (with `remote`) over the network.

## Testing

`cargo test -p display`: the desktop's window management and input (stacking, activation, frames, dragging, routing)
as unit tests. The kernels' window programs (atlas's `windows.c` and `desktop.c`) are checked pixel by pixel through
screenshots; with `CARBON_SCREENSHOTS=<folder>`, `run-windows` saves the screen and each window's last picture.

## To do

- [ ] **`display/macos`**: the desktop on the host, either whole in one window or each top-level window in a native
      one, with keyboard and mouse. Then `display/linux` and `display/windows`. `--display host`.
- [ ] **`carbon display` commands**: `click`, `type` and `key` beside `screenshot`, against a running machine.
- [ ] **The desktop, the rest**: minimizing and maximizing; cursor shapes, drawn by the host or a viewer;
      `SetCapture`; WM_NCHITTEST and frames a program draws itself (today a frameless window shown on the host would
      still get a host title bar); keyboard layouts besides US; the clipboard, and the host's (Cmd+V).
- [ ] **What `remote` needs** (its README has the details): observers, each with every window's damage as a set of
      rectangles and a log of changes, taken once a frame; a window manager outside it (`place`, `close_request`,
      `raise`) for windows mode; cursor shapes set by the kernels; text input; a screen size that changes, for
      `headless=auto`.
- [ ] Real fonts, shared with atlas's GDI, in place of the one bitmap font.
- [ ] Not tried yet on a host window: typing, frameless popups, how scrolling feels.
