# remote

**Not started.** A machine's desktop and sound, shown and driven from another computer: `carbon run --remote
0.0.0.0:4433 …` on a server (a Linux VM in the cloud, say) and `carbon connect box/app` on a laptop. A native viewer,
a protocol of Carbon's own, QUIC underneath, and keys as SSH uses them for who may connect. No web client for now.

It builds on [`display`](../display/README.md)'s `Desktop`, the window manager both kernels share, and on
[`audio`](../audio/README.md)'s mixer. From RDP it takes the features a remote desktop needs, and leaves out the
layers and history around them.

```
 Server (any host, usually Linux)                              Viewer (a laptop)
┌──────────────────────────────────────────────┐            ┌──────────────────────────────────┐
│ lambda / atlas                               │            │ carbon connect                   │
│   win32k, Wayland, X11 ─► display::Desktop   │            │                                  │
│                           windows, frames,   │   QUIC     │  remote (viewer side)            │
│                           stacking, input    │  TLS 1.3   │   windows, their pixels, tiles   │
│                              │      ▲        │  ◄──────►  │        │              ▲          │
│            remote (server side)              │  UDP 4433  │        ▼              │ input    │
│            one session per viewer: diff,     │  (or 443)  │  display/macos: the desktop in   │
│            encode, flow control, input       │            │  one window, or each window in   │
│                              │               │            │  its own native window           │
│            remote/quic: listener, keys       │            │  remote/quic: dialer, keys       │
└──────────────────────────────────────────────┘            └──────────────────────────────────┘
```

- **The desktop and the mixer are the source.** The server reads the desktop (windows, their pixels, their stacking,
  the active one) and the mixer, and feeds input and the microphone back into them. It never sees win32k, Wayland,
  X11 or PulseAudio, so it serves both kernels and every program in them, and a machine looks and sounds the same
  headless, on the host or remotely. No kernel knows it's there.
- **Per window, not per screen.** Each top-level window's pixels go on their own; where windows are, which is on top
  and how their frames look go as a few bytes of state. Moving a window or bringing another to the front sends no
  pixels.
- **Two ways to show it.** *Desktop mode* draws the whole desktop in one viewer window, with the desktop's own
  frames. *Windows mode* gives each remote window a native window of the viewer's: the viewer's window manager moves
  and stacks them, and tells the server.
- **Sans-IO.** `remote` is messages, codecs and sessions with no sockets; `remote/quic` carries them. Tests join a
  server session and a viewer in memory, and another transport needs no protocol changes.

## Principles

| Principle | What it means |
| --- | --- |
| **Damage only** | Every layer passes on just the rectangles that changed. An idle desktop costs nothing, and a ticking clock a few hundred bytes a second |
| **A shadow** | The server keeps, per viewer and window, exactly the pixels the viewer holds, and sends only 64×64 tiles that differ from it |
| **Exact codecs** | Every codec is integer-only, so the viewer decodes bit for bit what the server predicts, and the shadow stays exact even after lossy updates |
| **The right encoding per region** | Lossless for UI and text, moved pixels for scrolling, references for tiles the viewer already has, lossy only for what keeps changing |
| **Fast while moving, exact when still** | Busy tiles go lossy; once they've been still for 300 ms they're sent again losslessly |
| **The latest pixels, never a backlog** | Frames are acknowledged. While the link is slow, damage merges and waits rather than queueing, and busy areas get lossier |
| **The viewer remembers** | Tiles the viewer has, by hash, kept on its disk: a reconnect redraws from references, not pixels |
| **Local where latency shows** | The viewer draws the cursor from images it has kept, and the frames from each window's state |
| **Small, direct messages** | One length-prefixed framing, capability bits in the handshake, unknown messages skipped by length |

### What RDP has, and what's kept

| RDP | Kept | As |
| --- | --- | --- |
| Persistent bitmap cache | ✓ | Kept tiles, by hash, on the viewer's disk |
| RemoteFX's progressive codec (lossy first, refined to lossless) | ✓ | Lossy busy tiles, refined once still |
| EGFX frame acknowledgements; network auto-detect | ✓ | `FrameAck` and flow control; QUIC measures the round trip itself |
| RemoteApp (RAIL): programs' windows as the client's own | ✓ | Windows mode |
| Multitransport (RDP-UDP) | ✓ | QUIC: UDP, with no head-of-line blocking between streams |
| Auto-reconnect cookie | ✓ | QUIC session resumption, and kept tiles |
| Fast-path input | ✓ | Small input messages; pointer motion as datagrams |
| Pointer caching | ✓ | `Cursor`, by hash, drawn by the viewer |
| Display Control channel | ✓ | `ViewerSize`, with `--display headless=auto` |
| Clipboard, drive redirection, audio out and in | ✓, later | `Clipboard`, files, `Audio` and `Mic` (Opus), on streams and datagrams of their own |
| Virtual channels | ✓, simply | A QUIC stream per extension, by name |
| X.224, T.125 MCS and GCC; BER and PER; the sprawl of capability sets | ✗ | One `Hello` and `Welcome`, with capability bits |
| CredSSP/NLA, licensing, RDP's own security layer | ✗ | TLS 1.3 with keys, and nothing else |
| GDI drawing orders; glyph, brush and palette caches | ✗ | Pixels and tiles: simpler, and as fast with a shadow and a cache |
| MPPC, NCRUSH and XCRUSH bulk compression | ✗ | Codecs that compress what they send |
| Printer, smart card, serial and port redirection; server redirection | ✗ | Not needed |
| Channel chunking, slow-path PDUs, heartbeats | ✗ | QUIC's streams, flow control, keepalives and migration |

## Modes

| | Desktop mode (the default) | Windows mode (`--windows`) |
| --- | --- | --- |
| The viewer shows | One window: the remote screen, frames drawn by the viewer with the desktop's own `display` code | Each remote top-level window as a native window, framed by the viewer's OS |
| Moving, resizing, stacking | The server's desktop, as on the machine: pointer input in screen coordinates | The viewer's window manager; the viewer sends `Place`, `Raise`, `Activate` and `Close`, and the desktop follows |
| Popups and menus | Placed by the desktop | Borderless native windows where the desktop puts them |
| Screen size | Fixed (`--display headless=1280x800`) or following the viewer's window (`headless=auto`) | The viewer's screen; windows keep their own sizes |

A window dragged in desktop mode costs one small `Window` message per frame. Once that works, the viewer can show a
drag before the server's answer arrives.

## Transport: QUIC

QUIC (RFC 9000) over UDP, with TLS 1.3 inside it (RFC 9001), ALPN `carbon/1`, and unreliable datagrams (RFC 9221).

| What | Where | Why |
| --- | --- | --- |
| Control | One bidirectional stream, opened by the viewer | Handshake, window state, acknowledgements, cursor, clipboard text: small, ordered, reliable |
| A window's pixels | One server-to-viewer stream per window | Ordered within a window (`Delta` and `Copy` build on what came before); independent between windows, so a large update to one never holds up another |
| Input | One viewer-to-server stream | Keys, buttons, text and the wheel: never lost, never reordered |
| Pointer motion | Datagrams, numbered | Only the latest matters: a lost one is replaced by the next, never sent again late |
| Bulk | A stream each, at low priority | Files and large clipboard items never delay pixels or input |
| Sound (later) | Datagrams: 20 ms Opus packets | Late sound is worse than lost sound |

QUIC brings what would otherwise have to be written: congestion control and round-trip estimates (flow control reads
them), loss recovery, keepalives, **connection migration** (a laptop moving from Wi-Fi to a phone's network keeps its
session), and **session resumption**: a reconnect takes one round trip, or none for `Hello`. 0-RTT data can be
replayed, so only `Hello` may ride in it: no input, no files.

The port is **4433/udp** by default. `--remote 0.0.0.0:443` uses 443/udp, which most networks let through for HTTP/3;
on Linux that needs root, or `setcap cap_net_bind_service=+ep` on the binary.

## Security: keys, as with SSH

TLS 1.3 with keys on both sides (mutual TLS). No passwords, no tokens.

- **The server's key.** By default the host's own Ed25519 key (`~/.carbon/remote/host_key`, made on first use), in a
  self-signed certificate; its fingerprint is printed when serving starts. Or a real certificate (`--remote-cert`,
  `--remote-key`, from Let's Encrypt, say), checked against the viewer's system roots.
- **The viewer checks it** as SSH does: the first time it shows the fingerprint and asks, unless given `--host-key
  SHA256:…`; it then keeps it in `~/.carbon/known_hosts` and refuses a changed key loudly. A certificate for the
  host's name from a trusted CA needs no prompt.
- **The viewer's key.** `~/.carbon/id_ed25519` by default, or an OpenSSH Ed25519 key (`--identity
  ~/.ssh/id_ed25519`), presented as a TLS client certificate, or a raw public key (RFC 7250).
- **Who may connect.** `~/.carbon/remote/authorized_keys`, in OpenSSH's format, so `cat id_ed25519.pub >>
  authorized_keys` works. Options per line: `view-only`, `machine=NAME`. An unknown key is refused in the TLS
  handshake, before any Carbon message is read.
- **Roles.** A view-only viewer is told so (`VIEW_ONLY`), and its input is dropped. Input from several controlling
  viewers goes to the desktop's one pointer and keyboard.
- **Parsers** are sans-IO and reject bad input without panicking. Limits on sizes (windows, files, clipboard) are
  protocol constants.

## The protocol

Messages are little-endian `u32 length | u16 opcode | payload` on streams; a datagram carries one message, without
the length. Opcodes: `0x00xx` session, `0x01xx` windows, `0x02xx` input, `0x03xx` sound, `0x04xx` files and
clipboard. Unknown opcodes are skipped by their length, so either side can be newer.

**Handshake** (control stream):

| Message | Fields |
| --- | --- |
| `Hello` (viewer) | magic `CRBN`, version, capabilities, the machine's name, the mode, the viewer's size and scale, how many tiles it kept for this server and machine |
| `Welcome` (server) | version, the capabilities both have, the role (control or view-only), the screen's size, the desktop's identity (what kept tiles belong to), the windows there are now |
| `Tiles` (viewer) | the hashes of the tiles it kept, in pieces, so the server can send references at once |
| `Bye` (either) | why: the machine stopped (with its exit code), the key was refused, the server is shutting down |

Capabilities: `QOI`, `DELTA`, `COPY`, `TILES`, `JPEG`, `H264`, `CURSOR`, `CLIPBOARD`, `FILES`, `AUDIO`, `OPUS`, `MIC`,
`SCREEN_FOLLOWS_VIEWER`, `VIEW_ONLY`.

**Window state** (control stream, server to viewer): `Window` (id, its rectangle, its content's rectangle, title,
framed, layer, owner, shown, kind: normal, popup, tool), `WindowGone`, `Stacking` (ids, bottom to top), `Active`,
`Cursor` (hidden, a named shape, a QOI image, or a kept one by hash; its hot spot), `PointerAt` (where a program moved
the pointer: `SetCursorPos`, `XWarpPointer`), `ScreenSize`.

**A window's stream** (server to viewer): `Size` (the content's new size, before anything drawn at it), `Pixels` (a
rectangle, its encoding, its bytes), `Copy` (moves a block the viewer already has: scrolling), `TileRef` (puts a kept
64×64 tile somewhere, by hash), `Keep` (these tiles have settled: keep them), `Video` (one H.264 frame of a stream
covering a rectangle), `Commit` (frame `seq` is complete: show it).

**Viewer to server:**

| Message | Where | Fields |
| --- | --- | --- |
| `FrameAck` | control | window, the last `seq` shown |
| `Motion` | datagram | a sequence number, window (0 in desktop mode), x, y |
| `Button` | input | window, x, y, which, down. It carries its own place, so a click lands where it was made even if motion was lost |
| `Wheel` | input | window, x, y, notches in 1/120ths |
| `Key` | input | evdev code, down |
| `Text` | input | UTF-8 that the US layout has no key for |
| `Locks` | input | Caps Lock and Num Lock, before any keys once the viewer has focus again |
| `ReleaseAll` | input | the viewer lost focus: every held key and button up |
| `Place`, `Raise`, `Activate`, `Close` | control | windows mode: what the viewer's window manager did |
| `ViewerSize`, `Scale` | control | the viewer's window or screen changed |

Later: `Clipboard` (text inline on the control stream; images and files on bulk streams), `FileUpload` and
`FileDownload`, `Audio` (server datagrams), `Mic` (viewer datagrams) and `MicWanted`.

## Pixels

A session keeps, per window, the **shadow** and the viewer's kept tiles. Each frame:

1. **Take the damage** since this session's last frame from the desktop, cut it into 64×64 tiles, and drop tiles
   whose pixels equal the shadow's.
2. **Find scrolls.** Rows (or columns) vote, by hash, for the offset they moved by; the winner is checked exactly and
   sent as `Copy`, so only the strip it exposed goes as pixels.
3. **Choose each region's encoding.** A tile the viewer kept: `TileRef`. A tile that changes every frame (busy): JPEG,
   at the quality flow control allows. A large busy area that stays put: H.264. Anything else: `Delta` (XOR against
   the shadow, then QOI) when that's smaller, else QOI. A tile last sent lossy that stays still for 300 ms is sent
   again losslessly; one held exactly and still for longer is `Keep`.
4. **Encode** from a snapshot, off the desktop's lock, on as many cores as the frame needs. Viewers in the same state
   share the bytes.
5. **Send** on the window's stream, then `Commit`, and update the shadow with exactly what the viewer will decode.

**Flow control.** Frames in flight cover the round trip (one per 16 ms of it, plus one), so a long link isn't capped
at a few frames a second. Once acknowledgements show a queue building, bytes in flight are held near what the link
carries in a round trip, measured only from frames that had others waiting behind them (a frame with nothing behind
it measures the program, not the link). While that budget holds frames back, lossy quality steps down once a second,
and back up after three quiet seconds. A frame that can't go isn't encoded: its damage merges into the next. A viewer
that stops acknowledging (minimized) stops its stream.

**The codecs** are Carbon's own, integer-only and dependency-free: QOI, `Delta`, JPEG, H.264, Opus, and deflate where it
pays for itself (a capability, chosen per message).

## Input

Keys are evdev numbers, the desktop's own (`display::keys`), so each viewer maps its keyboard to them once. Characters
the US layout can't type go as `Text`. Pointer positions are the screen's (desktop mode) or a window's content's
(windows mode); the server turns the second into the first from the window's place. Everything goes into the desktop
as `display::desktop::Input`, as local input and `SendInput` do, so the kernels can't tell them apart.

## What the desktop needs for it

`display::Desktop` today composites one screen and keeps one damage rectangle. Remote viewers need (and `display`'s
to-do list has):

- **Observers:** `observe() -> ObserverId`, and for each, every window's damage as a set of rectangles (merged as it
  piles up) and a log of changes (made, placed, titled, stacked, activated, gone, cursor), taken once a frame.
- **A window manager outside it** (windows mode): `place`, `close_request` and `raise`, doing what a drag or click on
  the desktop's own frame does, so the kernel gets its usual events.
- **Cursor shapes**, set by the kernels (`SetCursor`, `wl_pointer.set_cursor`), and **text input**.
- **A screen size that changes**, for `headless=auto`.

The desktop's lock stays a leaf: sessions copy what they encode out of it and encode with it released.

## On the command line

```sh
# Serving one machine's desktop, on every interface, UDP 4433. --remote implies --display headless.
carbon run --name app --remote 0.0.0.0:4433 setup.exe

# A set size, on 443/udp, with a real certificate.
carbon run --name dev --display headless=1920x1080 --remote 0.0.0.0:443 \
    --remote-cert /etc/letsencrypt/live/box.example.com/fullchain.pem \
    --remote-key /etc/letsencrypt/live/box.example.com/privkey.pem \
    debian:13 -- xfce4-session

# The screen follows the first viewer's window.
carbon run --name app --display headless=auto --remote [::]:4433 app.exe

# Who may connect.
carbon remote allow ~/.ssh/id_ed25519.pub --name laptop
carbon remote allow phone.pub --view-only --machine app
carbon remote keys                        # NAME    KEY                  ACCESS
carbon remote revoke laptop

# Serving a machine that's already running.
carbon remote start app --listen 0.0.0.0:4433
carbon remote stop app

# Connecting.
carbon connect box.example.com/app            # port 4433, machine "app"
carbon connect 34.118.2.7:443/dev --windows   # each window a native window
carbon connect box/app --view-only --fullscreen
carbon connect box/app --headless out/        # no windows: each frame's screen as a PNG
```

| Option | Meaning |
| --- | --- |
| `--remote ADDR` | Serve this machine's desktop on `ADDR`: `HOST:PORT`, `[::]:PORT`, or `:PORT` for every interface. There's no default address: serving is always asked for |
| `--remote-cert FILE`, `--remote-key FILE` | A PEM certificate chain and key; by default the host's own key, self-signed |
| `--remote-allow FILE` | The authorized keys; `~/.carbon/remote/authorized_keys` by default |
| `--remote-view-only` | Every viewer may watch; none may control |
| `--display headless=WxH\|auto` | The desktop's size; `auto` follows the first controlling viewer |
| `carbon connect HOST[:PORT][/MACHINE]` | Where. The machine can be left out when the server serves only one |
| `--windows` | Windows mode; desktop mode by default |
| `--identity FILE` | The viewer's key; `~/.carbon/id_ed25519` by default, made on first use |
| `--host-key SHA256:…` | Expect this server key, without asking |
| `--view-only`, `--fullscreen`, `--scale N` | How the viewer shows it |
| `--headless DIR` | Write the screen as PNGs instead of showing it |

When serving starts: `carbon: serving app on 0.0.0.0:4433/udp, host key SHA256:9f2c…`. The `carbon display`
commands take a remote machine too, which is how an agent drives one over the network: `carbon display screenshot
box/app -o s.png`, `carbon display click box/app 300 200`, `carbon display type box/app "hello"`.

## Layout

```
remote/        the protocol and sessions, sans-IO, no dependencies:
                 messages, framing, capabilities
                 codecs: QOI, Delta, Copy, tiles; later JPEG, H.264, Opus, deflate
                 server: a session per viewer (shadow, damage, scrolls, encodings, flow control, sound)
                 viewer: windows and their pixels, applying messages, sending input
  quic/        remote-quic: the transport, listener and dialer, TLS keys, known_hosts and authorized_keys
```

Dependencies: `cli` → `machine` → `remote-quic` → `remote` → `display`, `audio`. `machine` attaches a listener to a
machine's desktop and mixer; the viewer shows what it gets with `display/macos` (and later the other hosts').

## Testing

- **Sessions in memory:** a server session and a viewer joined without sockets. After every frame the viewer's pixels
  equal the shadow, and the shadow equals the desktop wherever nothing is pending.
- **A simulated link** with a round trip, a bandwidth and loss, for flow control: frames per second and bytes in
  flight against known answers.
- **QUIC on loopback**, with keys made for the test: refused keys, a changed host key, view-only input dropped,
  resumption.
- **End to end:** the kernels' window tests through a remote viewer, with input from the viewer's side.

## To do

1. **A first picture.** Desktop observers. `remote` with QOI, `Copy` and kept tiles; window state, input and
   `FrameAck`. `remote-quic` with keys, `known_hosts` and `authorized_keys`. `carbon connect --headless`, then desktop
   mode on macOS. *Done means:* atlas's `desktop.c` runs on one computer and is driven from another, and the
   viewer's PNG equals the server's screenshot pixel for pixel.
2. **Fast on real links.** Flow control, `Delta`, kept tiles on disk, session resumption and migration, windows mode.
3. **Busy content.** Lossy busy tiles with refinement, scroll detection, encoding on several cores, encodings shared
   between viewers.
4. **The rest.** H.264, cursors from the kernels, clipboard, files, sound and microphone (Opus, from and into
   `audio`'s mixer), `headless=auto`, `carbon remote start` and `stop`, a host-wide gateway, and the `carbon display`
   commands against remote machines.

## Open questions

1. **QUIC and TLS: ours, or crates?** Carbon depends on almost nothing beyond `libc` and each host's system libraries.
   QUIC with TLS 1.3 and its cryptography (X25519, Ed25519, AES-GCM, ChaCha20-Poly1305, SHA-256, HKDF) is a project of
   its own, perhaps 15–25k lines. *Recommendation:* `quinn` and `rustls` for now, kept inside `remote-quic`, the only
   crate allowed them. The protocol is sans-IO, so writing our own later changes that one crate. (`images` already
   uses `rustls` for pulling.)
2. **One port per machine, or one for the host?** `--remote` on each `carbon run` is simplest. A host serving many
   machines behind one firewall rule wants a gateway (`carbon remote serve --listen 0.0.0.0:443`) routing by the
   machine named in `Hello` to each machine's worker process. *Recommendation:* per machine first, the gateway in
   step 4.
3. **4433 or 443 by default?** 4433 needs no privileges; 443 gets through more networks. *Recommendation:* 4433 by
   default, with 443 documented for servers.
4. **Keys.** *Recommendation:* Carbon's own `~/.carbon/id_ed25519`, accepting OpenSSH Ed25519 keys and
   `authorized_keys` lines as they are, so existing SSH keys just work.
5. **A browser, later?** WebTransport is QUIC too, so a browser viewer could speak this protocol unchanged, with the
   viewer compiled to WebAssembly. Not now, but nothing here should rule it out.
