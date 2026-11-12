# display

A machine's screen, whichever kernel draws on it: the desktop both kernels' windows go on, and the pixels they're
drawn in. Neither kernel writes a window manager of its own, and neither knows where its windows end up: on a screen
in memory, in the host's own windows, or (through [`remote`](../remote/README.md)) on another computer.

**First cut:** the shared part is built, with the desktop on a screen in memory. Showing it on the host is to do.

```
 atlas: win32k (windows, messages, GDI)        lambda: os/wayland, os/x11
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
- **Buffers** (`buffer.rs`): what a window shows. A `Pixels` source (XRGB8888, or premultiplied ARGB8888) the desktop
  reads in place as it composites and never writes, so a client's shared memory is shown without a copy; a `Buffer`
  is one attached to a window. A `Canvas` is a buffer in memory a kernel draws into itself: win32k keeps one per
  top-level window for GDI.
- **`Desktop`** (`desktop.rs`): the window manager. A kernel makes and destroys windows, sets their place, title,
  layer, owner and visibility, asks to raise or activate them, and shows its pixels in them Wayland's way: a buffer
  `attach`ed, `damage`d and `commit`ted at once, with a frame asked for (`request_frame`). It reads back `Event`s
  (`Configure`: where a window is and what it is, after its frame moved or resized it or it changed state, with a
  serial to acknowledge; activated or not; asked to close; pointer and key input in content
  coordinates, a buffer `Released` when another is committed in its place, `Frame` at the clock's tick, and
  `Presented` when a commit that asked (`request_presented`) reached the screen, or `None` if it never will).
  Content its buffer doesn't cover shows the window's fill; buffers with alpha blend over what's below. A buffer can
  sit at an offset from its content's corner (`set_buffer_offset`: Wayland's window geometry, X11's
  `_GTK_FRAME_EXTENTS`): all of it is shown, the part past the window being a shadow its client draws, while the
  window alone is what's placed, sized, maximized and clicked. `configuring` says a `Configure` is still waiting to be
  taken, so a kernel doesn't size a window by a buffer drawn before it.
- **Roles** (`desktop.rs`, `positioner.rs`): a window is a toplevel, a dialog (transient for a parent: kept above
  it, which is active again when it goes; win32k's owned windows), or a popup: a menu, tooltip or list placed against
  its parent by a `Positioner` (Wayland's `xdg_positioner` rules: an anchor rectangle, anchor and gravity, an offset,
  and flip, slide and resize to stay on the screen, which also cover `TrackPopupMenu` and X11's override-redirect
  menus). A popup has no frame, sits above every other layer, moves with its parent, and is never the active window.
  One that grabs takes the keyboard and pointer while shown; a press outside it and the popups it opened from
  dismisses them all (`PopupDone`, topmost first) and goes nowhere, as a menu closes. Hiding or destroying a parent
  dismisses its popups.
- **States** (`desktop.rs`): maximized (the screen, frame and all), fullscreen (the screen with no frame, above
  the windows that aren't), minimized (kept, off the screen, its popups dismissed, not active), and resizing (while
  its frame's edge is dragged); where it was, kept to restore. A content size's minimum and maximum, which resizing
  keeps to. A maximized or fullscreen window's frame doesn't move or resize it. Each change comes to its kernel as
  one `Configure`, which `xdg_toplevel`'s configure, X11's `ConfigureNotify` and `WM_SIZE` (with `SIZE_MAXIMIZED`
  and `SIZE_MINIMIZED`) are all read from; win32k's `ShowWindow`, `IsZoomed` and `IsIconic` are on it.
- **Frames and windows that draw their own** (`desktop.rs`): a window's frame given or taken away at any time
  (`set_framed`), keeping its content where it is (Wayland's decoration mode, X11's `_MOTIF_WM_HINTS`) or its
  rectangle (a Windows style change). A window that draws its own title bar asks the desktop to move or resize it
  (`begin_move`, `begin_resize`) while the press on it is held: Wayland's `xdg_toplevel.move` and `resize`, X11's
  `_NET_WM_MOVERESIZE`, Windows' `WM_NCLBUTTONDOWN` and `WM_SYSCOMMAND`. The desktop then moves it exactly as its own
  frame would; a request that comes after the button's up does nothing.
- **The pointer** (`desktop.rs`): enter and leave as it crosses windows' content (not while a button's held: the
  window it went down on keeps it), scrolling in 120ths of a notch on both axes (Windows' `WHEEL_DELTA`, Wayland's
  `axis_value120`), and a capture (`set_capture`: Windows' `SetCapture`, X11's `GrabPointer`) that gives one window
  every pointer event wherever it is, a press neither activating another window nor using a frame. win32k's
  `SetCapture`, `GetCapture`, `ReleaseCapture` and `WM_MOUSEHWHEEL` are on it.
- **Pointer constraints and relative motion** (`desktop.rs`): a window asks for the pointer to be locked
  (`Constraint::Lock`: it stays put and hidden, Wayland's `zwp_locked_pointer_v1`, taken once the pointer's in its
  content) or confined to a rectangle (`Constraint::Confine`: `zwp_confined_pointer_v1`, Windows' `ClipCursor`, taken at
  once, moving with the window). Either holds only while it's the active window, and `Constrained` says when it's
  taken and let go. A window that asks (`set_relative_motion`) gets `Pointer::Relative`, the motion as the device saw
  it (`Input::PointerBy`, a host's raw deltas), unaccelerated and past the screen's edge: Wayland's
  `zwp_relative_pointer_v1`. win32k's `ClipCursor` and `GetClipCursor` are on it.
- **Stacking** (`desktop.rs`): every window's in a band, bottom to top: `Layer::Below` (a desktop's icons), `Normal`,
  `Top` (always on top), a fullscreen window while it or a window it owns is active (above docks and topmost windows,
  as EWMH has it; back among the others when it isn't), and popups. An owned window is in its owner's band at least
  and always above it: a topmost window's dialogs are topmost, a fullscreen window's are above it, and moving an owner
  moves what it owns. Within those rules a kernel puts a window where its API says (`stack`: top, bottom, just above
  or below another: `SetWindowPos`'s insert-after, X11's `ConfigureWindow` with a sibling). A window a click doesn't
  activate (`Spec::no_activate`: `WS_EX_NOACTIVATE`, a dock, `WM_HINTS` input False) takes the press without being
  activated or raised, and isn't made active when another goes. win32k's `SetWindowPos` (`HWND_TOP`, `HWND_BOTTOM`,
  `HWND_TOPMOST`, `HWND_NOTOPMOST`, a window to go behind), `WS_EX_NOACTIVATE`, `GetWindow`, `GetTopWindow` and
  `EnumWindows` (top first) are on it. X11's window types map onto the same few rules:

  | `_NET_WM_WINDOW_TYPE` | On the desktop |
  |---|---|
  | `NORMAL` | a toplevel, framed |
  | `DIALOG`, `UTILITY`, `TOOLBAR`, `MENU` (torn off), and any with `WM_TRANSIENT_FOR` | a dialog of the window it's transient for (a toplevel without one), framed except a toolbar or menu |
  | `SPLASH` | a toplevel, unframed, `Top` while it's shown |
  | `DOCK` | unframed, `Top`, `no_activate` (its struts, when outputs have work areas) |
  | `DESKTOP` | unframed, `Below`, `no_activate` |
  | `NOTIFICATION` | unframed, `Top`, `no_activate` |
  | `DROPDOWN_MENU`, `POPUP_MENU`, `COMBO`, `TOOLTIP` (override-redirect) | a popup on the window it's for, a grab for menus and combos |
  | `DND` | the drag's icon (not drawn yet) |

  `_NET_WM_STATE_ABOVE` and `_BELOW` are `Top` and `Below`; `_NET_WM_STATE_FULLSCREEN` is the fullscreen state.
- **lambda's Wayland server** (`kernels/lambda/src/os/wayland`) is on the same contract: an `xdg_toplevel` is a
  window, its `wl_shm` buffers `Pixels` read in place from the client's memfd, a frame callback the desktop's `Frame`,
  a buffer replaced `Released`, a configure, activation or close Wayland's own events; the pointer's events are
  `wl_pointer`'s (scrolling in 120ths, a cursor surface the window's `Cursor::Image`), keys `wl_keyboard`'s (the
  keymap above, focus following activation, repeats left to the client), and the output `wl_output`'s; an
  `xdg_popup` is a desktop popup placed by its `xdg_positioner` (the desktop's `Positioner`), a server-side
  decoration the desktop's frame, `xdg_toplevel.set_parent` a dialog (`set_owner`).
- **Focus policy** (`desktop.rs`): each window is a client's (a process, a connection: `Spec::client`). A kernel asks
  for activation (`request_activation`: `SetForegroundWindow`, `xdg_activation_v1`, `_NET_ACTIVE_WINDOW`) and the
  desktop grants it if nothing's active, the active window is the same client's, that client had the user's last press
  or key, or with a token (`activation_token`) issued since the user's last one. Refused, the window wants attention
  instead (`set_attention`: Windows' `FlashWindow`, X11's urgency), which observers and `carbon display windows` show,
  until it's active. win32k's activations all go through it, `FlashWindow` and `FlashWindowEx` too.
- **Key repeat** (`desktop.rs`): a key down for a key that's down already is its repeat, marked in its `Key` event
  (win32k's `WM_KEYDOWN` with the previous state set; Wayland's server drops them, since its clients repeat keys
  themselves from `key_repeat()`'s rate and delay). The repeats come from the source, as hosts and viewers send them,
  and aren't new input to the focus policy.
- **The output** (`output.rs`): what the desktop's shown on: its size in logical pixels (the screen's, and what
  windows are placed in), its scale in 120ths (Wayland's fractional unit: 120 is 1×, 180 is 1.5×, 240 is 2×), refresh
  and physical size, changed at any time (`set_output`): the screen takes the new size, maximized and fullscreen
  windows follow, each window hears of a new scale (`Scale`), and observers of the output. A window commits buffers at
  a whole buffer scale (`set_buffer_scale`: a 2× buffer for a 2× output), shown scaled down on the logical screen.
  win32k tells programs 96 DPI (`GetDpiForWindow`, `GetDpiForSystem`, `GetDeviceCaps`), as Windows tells one that isn't DPI aware, and its windows are magnified to the output's scale.
- **The clipboard and the primary selection** (`selection.rs`): an owner (a window, or outside the machine: the
  host, `carbon display clipboard`) offers a selection as MIME types, with data given at once (Windows'
  `SetClipboardData`) or sent when a paste asks (Wayland's and X11's way: `SendSelection`, answered with `provide`).
  A new offer takes it from the owner before (`SelectionCancelled`: `WM_DESTROYCLIPBOARD`, `SelectionClear`,
  `cancelled`). A paste goes to a window (`SelectionData`) or outside (`paste`, waiting for the owner's kernel). What
  was all given at once outlives its owner's window, as Windows' clipboard does. Observers hear of each new offer.
  win32k's clipboard is on it, text in any of its text formats as one UTF-8 type.
- **Drag and drop** (`desktop.rs`, on the same offers): a window with a button held on its content starts a drag
  (`start_drag`: Wayland's `start_drag`, `DoDragDrop`, XDND), or something outside the machine does (the host,
  `drop_from_outside`, `carbon display drop`). What it carries is offered as `Selection::Drag`, read with `request` as a
  paste is. The window whose content it's over hears `DragEnter` (with the types, and the actions the source allows:
  `action::COPY`, `MOVE`, `LINK`, `ASK`), `DragMotion` and `DragLeave`, and says what it would take
  (`accept_drag`); the desktop chooses the action, the source hears it (`DragAction`) and the cursor shows it. Letting
  go over a window that takes it sends `Drop`, the target reads it and says it's done (`finish_drop`), and the source
  hears `DragEnd` with what was done; anywhere else, or with Escape, it's cancelled. Files travel as `text/uri-list`
  (`selection::file_uri`, `file_path`, `uri_list`). win32k takes files dropped on a window that asked
  (`DragAcceptFiles`) as WM_DROPFILES, through shell32's `DragQueryFileW`, `DragQueryPoint` and `DragFinish`.
- **Cursors** (`cursor.rs`): a window's cursor over its content, a shape by its CSS name (`wp_cursor_shape_v1`'s,
  which cover Windows' `IDC_*` and X11's cursor font), an image with a hotspot, or none; the desktop's own resize
  arrows over its frames' edges, and a move cursor while a frame's dragged. `cursor()` is the one to draw now, and
  observers hear when it changes. win32k's class cursors, `SetCursor` and `GetCursor` are on it.
- **Text input** (`desktop.rs`): text an input method committed (or that no key types) goes to the window with the
  keyboard (`Event::Text`; win32k's `WM_CHAR`s), and what it's composing (`Event::Preedit`) to one that takes text
  (`set_text_input`, with where its text cursor is, which observers hear of, for placing the input method's window).
- **The keymap** (`keymap.rs`): one description of the keyboard the servers derive theirs from: a US layout over the
  desktop's evdev numbers, as a fully compiled XKB text keymap (in the form libxkbcommon writes one), with the
  desktop's modifiers as XKB's (`xkb_mods`). lambda's Wayland server sends it to every `wl_keyboard`; its X
  server makes its core keyboard mapping and its XKEYBOARD map from the same keys (`levels`, `keys`, `keysym`).
- **Several servers on one desktop**: a kernel with two (lambda's Wayland and X11) wakes both (`add_waker`) and each
  takes only its own clients' events (`take_events_where`), by the windows' `ClientId`s.
- **Keys by name** (`keys.rs`): `by_name`, for `carbon display key` (`enter`, `ctrl`, `f4`, `pageup`, ...).
- **Observers** (`observe.rs`): whatever shows or reads the desktop (the headless screen, a host's windows,
  `remote`'s viewers, `carbon display`) takes what changed since it last looked, at its own pace: the latest state of
  each window that changed (a `WindowInfo`: rect, content, title, frame, layer, parent, popup, on the screen, states),
  never every step on the way; the windows gone; the stacking and the active window if they changed; and each
  window's content damage as a few rectangles. A new observer's first take is everything. Its waker is called when
  it goes from nothing to take to something. Pixels are read from a window's committed buffer when they're wanted
  (`read`), in place.
- **The clock** (`clock.rs`): a headless desktop's 60 Hz tick, which sleeps until a commit asks for a frame, so an
  idle desktop costs nothing. A host backend ticks the desktop with its own display's refresh instead (`clock::tick`). Input comes in as `Input`, from the host, from `carbon display`, or from a program (`SendInput`), and
  `send` wakes the kernel to read what it made.
- **The composited screen** (`screen()`): every shown window, frames and all, composited once and read by whoever
  shows the whole desktop. Each observer hears for itself what changed on it (`Observed::screen`), so the host's window
  and a screenshot taken now and then never take each other's damage. It's in logical pixels, as screenshots and
  `carbon display`'s coordinates are. **`screen_pixels()`** is the same screen in the output's own pixels (a Retina
  screen's at 2×): the logical screen magnified, except that an opaque buffer drawn at the output's scale (a 2× buffer,
  `set_buffer_scale`) is shown pixel for pixel, sharp, where no window above covers it; `Output::rect_to_pixels` maps
  an observer's damage onto it. At 1× it's the logical screen.
- **The Mac's screen** (`display/macos`, the `display-macos` crate): desktop mode, the whole desktop in one
  `NSWindow`, through the Objective-C runtime with no crates. The window's content is the output (resizing it resizes
  the screen, and `carbon display resize` resizes the window), and its backing scale the output's scale (2× on a Retina
  screen, followed as the window moves between screens); `screen_pixels()` is its layer's contents, at the backing
  scale, what changed copied in at each refresh; the Mac's keys (as evdev numbers, Command as Meta), buttons, wheel and trackpad
  scrolling (in 120ths, trackpad points and wheel lines alike, the user's natural scrolling kept) go in through
  `send`, taken from the event queue before AppKit sees them, so Command's key-ups aren't lost; the desktop's cursor
  is AppKit's nearest over the content; keys and buttons held are let go when the window loses the keyboard; the
  screen's refresh (`maximumFramesPerSecond`) is the frame clock. AppKit wants the main thread: `carbon run` boots the
  kernel on another for it. Closing the window stops the machine, as Ctrl-C does.
- **The Mac's windows** (`display/macos`, windows mode, `--display host-windows`): each top-level window in an
  `NSWindow` of its own, among the Mac's other windows. The desktop is laid over the main screen (below the menu bar,
  at its backing scale) and stays the truth, frames and all, so no kernel's geometry changes: a framed window gets the
  Mac's title bar where the desktop's frame is, an unframed one is borderless (and still takes the keyboard), a popup
  is a child window of its owner's, always-on-top windows float. Each Mac window follows its window (place, size,
  title, frame, layer, shown, minimized, stacking, the active one) and shows its content (`read_pixels`: at the
  output's scale, a 2× buffer as it is); what the user does to a Mac window goes back as the desktop's own frames would
  tell it (`placed`, `set_minimized`, `request_close`, `activate`). Pointer events are placed on the desktop from
  where they were on the screen, so a window moving under a drag doesn't throw them off.
- **The Mac's clipboard, input method and cursors**, in both modes: the general pasteboard and the machine's
  clipboard follow each other (text copied on the Mac is offered to the machine; what a window offers is pasted, off
  the main thread, onto the pasteboard; `selection_source` and the change count keep either from copying back its
  own). Keys go through the Mac's input method in a view of ours that's an `NSTextInputClient`: what a key types as a
  US keyboard would goes in as the key, an IME's commit, a dead key's accent or another layout's letter as `Text`, a
  composition as `Preedit`, its candidate window opened at the guest's text cursor (`text_input`); Command, Control
  and Option keep their keys as they are (shortcuts, and Option as Alt). A window's own cursor picture is an
  `NSCursor` with its hotspot.

### What the desktop does, and what each kernel does

| The desktop does | Each kernel does |
| --- | --- |
| Top-level windows only: where they are, their stacking (a normal and an always-on-top layer, owned windows above their owners), and which one is active | Everything inside a top-level window: win32k's child windows, messages and `WM_NCHITTEST`; Wayland's subsurfaces and X11's subwindows |
| Frames: a title bar with the window's title, maximize and close buttons, and edges to resize by (within the window's size limits). A window can go without (`WS_POPUP`, a Wayland client that decorates itself, X11's override-redirect for menus and tooltips) | What its API says a window's frame is: win32k's window and client rectangles, from the frame's insets |
| Input: a click goes to the window under it, in that window's coordinates, and activates it; keys go to the active window; a drag stays with the window it started on. Moving and resizing by the frame happen here, whole | Turning the desktop's events into its own: `WM_LBUTTONDOWN`, `WM_KEYDOWN`, `WM_ACTIVATE`, `WM_SIZE`; `wl_pointer` and `wl_keyboard`; X11's events |
| Compositing every window's pixels into one screen, with what changed since last time; screenshots and `carbon display`'s clicks and keys | Drawing into its windows' pixels: GDI in win32k, the client's buffers for Wayland and X11 |
| Policy the kernel asks for and the desktop carries out: raise, activate, keep on top, show and hide | Policy its API defines: Windows' foreground rules, `SetWindowPos`'s flags, `xdg_toplevel`'s states |

The desktop has no taskbar or shell: those are programs (Explorer, a Linux panel). Its frames have a plain look of
their own, not Windows' or GNOME's. Its lock is a leaf but for one: a buffer's own (a `Canvas`'s), which the desktop
takes as it composites. So a kernel draws into a canvas without the desktop's lock, and commits after.

## Two backends, one API

- **Built in**: the window manager above, on a screen in memory. For headless machines, and for showing a whole
  machine's desktop in one host window (desktop mode, built on macOS).
- **Host windows**: each top-level window gets a native window of the host's (an `NSWindow` on macOS), and the host's
  own window manager places, stacks, frames and activates them; the desktop passes its events through.

Like `audio`, `net` and `host`, it's a shared part with a folder per host, and every host-specific line inside it:

| Host | Shows the desktop with |
| --- | --- |
| macOS (`display/macos`) | AppKit: one window for the whole desktop (built), or an `NSWindow` per top-level window |
| Linux (`display/linux`) | the host's own display server |
| Windows (`display/windows`) | Win32 windows |

## On the command line

```sh
carbon run --display headless=1280x800 setup.exe   # a screen in memory (the default today)
carbon run --display headless=1440x900@2 setup.exe # the same at 2×, as a Mac's screen
carbon display resize app 1600x900                 # a running machine's screen changed, as a viewer's resize
carbon display clipboard app                       # its clipboard's text; with text after, set, as the host's
carbon run --display none debian:13 -- bash        # no screen
carbon run --display host setup.exe                # the whole desktop in a window on this Mac (host=WxH to size it)
carbon run --display host-windows setup.exe        # each of its windows in a Mac window of its own
carbon display screenshot app -o screen.png        # a running machine's screen, as a PNG
carbon display windows app                         # its windows, top first: ID, place, size, state, title
carbon display click app 640 400                   # input, as a user's: move, click (right, double), drag,
carbon display type app 'Hello, world!'            #   scroll, type text, and keys held together
carbon display key app ctrl+s
```

`--display host` may become the default where the host has a screen; today it's asked for, so a machine an agent
starts stays headless. `carbon display`'s input (`move`, `click`, `drag`,
`scroll`, `type`, `key`) goes in through `display::desktop::send`, exactly as a host's or a viewer's would: it's how
an agent drives a machine, locally or (with `remote`) over the network, beside `windows` and `screenshot`.

## Testing

`cargo test -p display`: the desktop's window management and input (stacking, activation, frames, dragging, routing),
its content (commits, damage, release, alpha, buffer offsets and shadows, frame requests, the clock), and popups (placement, flipping and
sliding, grabs and dismissal, moving with their parent), states (maximize, fullscreen, minimize, the maximize
button, size limits while resizing), frames (given and taken away, moving and resizing a window that draws its
own), and observers (first takes, coalescing, damage rectangles, reading pixels, their own pace and waking) as unit
tests. The kernels' window programs (atlas's `windows.c` and `desktop.c`) are checked pixel by pixel through
screenshots; with `CARBON_SCREENSHOTS=<folder>`, `run-windows` saves the screen and each window's last picture.

## To do

- [x] **`display/macos`, desktop mode**: the whole desktop in one window, with keyboard, mouse and trackpad, the
      window's size as the output's, the screen's refresh as the clock; `--display host`.
- [x] **The backing scale**: the output says the Mac's (2× on Retina), the window shows the screen at it, and a 2×
      buffer is sharp; 1× content (win32k's GDI, told 96 DPI as a program that isn't DPI aware is) is magnified.
- [x] **Windows mode**: each top-level window in an `NSWindow` of its own (`--display host-windows`).
- [x] **The host's clipboard, input method and image cursors** on the Mac, in both modes.
- [ ] **`display/macos`, the rest**: images and files on the clipboard; Option as a dead-key prefix, as a setting;
      dragging to and from the Mac; in windows mode, popups
      dismissed when another app's clicked, windows on screens besides the main one, the desktop's own move and
      resize handed to the Mac's (`performWindowDragWithEvent:`). Then `display/linux` and `display/windows`.
- [x] **`carbon display` commands**: `click`, `type` and `key` beside `screenshot`, against a running machine.
- [ ] **The desktop, the rest**: cursor shapes drawn by a viewer; WM_NCHITTEST and frames a program draws itself
      (today a frameless window shown on the host would still get a host title bar); keyboard layouts besides US.
- [ ] **What `remote` needs** (its README has the details): observers, each with every window's damage as a set of
      rectangles and a log of changes, taken once a frame; a window manager outside it (`place`, `close_request`,
      `raise`) for windows mode; cursor shapes set by the kernels; text input; a screen size that changes, for
      `headless=auto`.
- [ ] **The contract, as the servers ask for it.** Frames and states: a reader of `Presented` with the host's
      scan-out time; tiled states; a size the window asks for, answered; win32k's menus on the desktop's popups;
      activation tokens passed on at launch (`XDG_ACTIVATION_TOKEN`, `_NET_STARTUP_ID`, `AllowSetForegroundWindow`),
      so a second client's first window is made active; docks' struts, a work area, `_NET_RESTACK_WINDOW`, a taskbar
      for minimized windows; a DIB section's pixels lent in place. Input: serials from the desktop; the scroll's
      source, and touch; key repeat for a source that doesn't; win32k's scan codes from the keymap; cursors on the
      headless screen; win32k's IMM messages; raw input and pointer lock. Outputs: several, with windows moved
      between them; win32k's per-monitor DPI; scaled and translucent buffers composited at full resolution. Data:
      delayed clipboard rendering and images; a drag icon; OLE drag and drop in win32k; paths between kernels.
      Observers: each window's app ID and kernel in `carbon display windows`; compositing outside the desktop's lock;
      locking that scales to a 4K screen.
- [ ] `--display host` as the default where the host has a screen, once it's decided how an agent's machines stay
      headless.
- [ ] Real fonts, shared with atlas's GDI, in place of the one bitmap font.
- [ ] Not tried yet on a host window: typing, frameless popups, how scrolling feels.
