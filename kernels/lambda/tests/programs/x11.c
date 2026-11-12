// A freestanding X11 client for lambda's X server: no libc, no Xlib or xcb, just system calls and the wire protocol.
// It connects to $DISPLAY's socket and reads the setup (the root, its visual, its resource base), interns an atom and
// reads its name back, makes a window with a background and maps it (MapNotify, then Expose, the background painted),
// sets and reads a property, fills a rectangle and reads it back with GetImage, puts an image into a pixmap and reads
// it back, copies it onto the window, finds the extensions toolkits ask for, owns a selection, reads the keyboard
// mapping (evdev's numbers plus 8, a US layout's keysyms), sees a BIG-REQUESTS request done, and gets the errors a
// careless client earns (a drawable that isn't one, an ID that isn't its own). Run it with a display
// (CARBON_DISPLAY=640x480) to have the server at all.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef int i32;
typedef unsigned short u16;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit_group = 94, SYS_socket = 198, SYS_connect = 203, SYS_read = 63 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#else
enum { SYS_write = 1, SYS_exit_group = 231, SYS_socket = 41, SYS_connect = 42, SYS_read = 0 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#endif

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

// ---- The wire ----

static int sock;
static u8 out[1 << 16];
static u32 nout;
static u16 seq;
static void b8(u32 v) { out[nout++] = (u8)v; }
static void b16(u32 v) { b8(v); b8(v >> 8); }
static void b32(u32 v) { b16(v); b16(v >> 16); }
static void bytes(const char *s, u32 n) { for (u32 i = 0; i < n; i++) b8((u8)s[i]); while (nout % 4) b8(0); }
static u32 start;
// A request: its opcode and header byte; its length's filled in by `send`.
static void req(u32 major, u32 b1) { start = nout; b8(major); b8(b1); b16(0); }
static u16 send(void) {
    u32 words = (nout - start) / 4;
    out[start + 2] = (u8)words;
    out[start + 3] = (u8)(words >> 8);
    sys(SYS_write, sock, (i64)out, nout, 0, 0, 0);
    nout = 0;
    return ++seq;
}

static u8 in[1 << 17];
static u32 nin;
static int eof;
static void more(void) {
    i64 n = sys(SYS_read, sock, (i64)in + nin, sizeof in - nin, 0, 0, 0);
    if (n <= 0) { eof = 1; return; }
    nin += (u32)n;
}
static u16 r16(const u8 *p) { return (u16)(p[0] | p[1] << 8); }
static u32 r32(const u8 *p) { return p[0] | p[1] << 8 | p[2] << 16 | (u32)p[3] << 24; }

// What came besides replies.
static int mapped, exposed, configured;
static u32 error_code, error_seq;
// The next message: an error, a reply (into `msg`, whole), or an event (noted, and skipped).
static u8 msg[1 << 17];
static u32 msg_len;
static int next(void) {
    for (;;) {
        while (nin < 32 && !eof) more();
        if (eof) return -1;
        u32 n = 32;
        if (in[0] == 1) {
            n += r32(in + 4) * 4;
            while (nin < n && !eof) more();
            if (eof) return -1;
        }
        for (u32 i = 0; i < n; i++) msg[i] = in[i];
        msg_len = n;
        for (u32 i = n; i < nin; i++) in[i - n] = in[i];
        nin -= n;
        u32 code = msg[0] & 0x7f;
        if (code == 19) mapped = 1;
        if (code == 12) exposed = 1;
        if (code == 22) configured = 1;
        if (code == 0) { error_code = msg[1]; error_seq = r16(msg + 2); }
        if (code <= 1) return code;
    }
}
// The reply (1) or error (0) to the request with sequence number `s`.
static int answer(u16 s) {
    for (;;) {
        int k = next();
        if (k < 0) return -1;
        if (r16(msg + 2) == s) return k;
    }
}

struct sockaddr_un { u16 family; char path[108]; };

void cmain(i64 *sp) {
    (void)sp;
    sock = (int)sys(SYS_socket, 1, 1, 0, 0, 0, 0);
    struct sockaddr_un addr = {1, "/tmp/.X11-unix/X0"};
    check(sock >= 0 && sys(SYS_connect, sock, (i64)&addr, sizeof addr, 0, 0, 0) == 0, "connect: /tmp/.X11-unix/X0");

    // Setup: little-endian, 11.0, no authorization.
    b8('l'); b8(0); b16(11); b16(0); b16(0); b16(0); b16(0);
    sys(SYS_write, sock, (i64)out, nout, 0, 0, 0);
    nout = 0;
    while (nin < 8 && !eof) more();
    u32 total = 8 + r16(in + 6) * 4;
    while (nin < total && !eof) more();
    u32 base = r32(in + 12), vendor_len = r16(in + 24), nformats = in[29];
    u32 screen = 40 + ((vendor_len + 3) & ~3u) + 8 * nformats;
    u32 root = r32(in + screen), white = r32(in + screen + 8), width = r16(in + screen + 20), height = r16(in + screen + 22);
    u32 root_visual = r32(in + screen + 32), root_depth = in[screen + 38];
    check(in[0] == 1 && r16(in + 2) == 11 && root != 0 && root_visual != 0 && root_depth == 24 && white == 0xffffff && width > 0 && height > 0,
          "setup: one screen, its root and 24-bit visual, its size");
    nin = 0;

    // An atom made, and its name read back.
    static const char name[] = "CARBON_TEST_ATOM";
    req(16, 0); b16(len(name)); b16(0); bytes(name, len(name));
    u16 s = send();
    u32 atom = answer(s) == 1 ? r32(msg + 8) : 0;
    req(17, 0); b32(atom);
    s = send();
    int named = answer(s) == 1 && r16(msg + 8) == len(name);
    for (u32 i = 0; named && i < len(name); i++) named = msg[32 + i] == (u8)name[i];
    check(atom > 68 && named, "InternAtom and GetAtomName: a new atom, past the predefined ones");

    // A window, with a background, mapped: MapNotify, then Expose with the background painted.
    u32 win = base | 1, gc = base | 2, pix = base | 3;
    req(1, 0); b32(win); b32(root); b16(20); b16(20); b16(200); b16(100); b16(0); b16(1); b32(0);
    b32(0x2 | 0x800); b32(0x336699); b32(0x8000 | 0x20000);
    send();
    static const char title[] = "x11.c on lambda";
    req(18, 0); b32(win); b32(39); b32(31); b8(8); b8(0); b16(0); b32(len(title)); bytes(title, len(title));
    send();
    req(8, 0); b32(win);
    send();
    req(43, 0); // GetInputFocus: a round trip, so everything before it's done
    s = send();
    answer(s);
    while (!(mapped && exposed) && next() >= 0) {}
    check(mapped && exposed, "CreateWindow and MapWindow: MapNotify and Expose");

    req(20, 0); b32(win); b32(39); b32(0); b32(0); b32(100);
    s = send();
    int titled = answer(s) == 1 && msg[1] == 8 && r32(msg + 8) == 31 && r32(msg + 16) == len(title);
    for (u32 i = 0; titled && i < len(title); i++) titled = msg[32 + i] == (u8)title[i];
    check(titled, "ChangeProperty and GetProperty: WM_NAME as it was set");

    // Drawn on: a red rectangle, read back; the background around it.
    req(55, 0); b32(gc); b32(win); b32(0x4 | 0x10000); b32(0xff0000); b32(0);
    send();
    req(70, 0); b32(win); b32(gc); b16(10); b16(10); b16(20); b16(20);
    send();
    req(73, 2); b32(win); b16(15); b16(15); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 inside = answer(s) == 1 ? r32(msg + 32) : 0;
    req(73, 2); b32(win); b16(100); b16(50); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 outside = answer(s) == 1 ? r32(msg + 32) : 0;
    check(inside == 0xff0000 && outside == 0x336699, "PolyFillRectangle and GetImage: the rectangle, and the background around it");

    // A pixmap: an image put into it and read back, then copied onto the window.
    req(53, 24); b32(pix); b32(win); b16(2); b16(2);
    send();
    u32 image[4] = {0x112233, 0x445566, 0x778899, 0xaabbcc};
    req(72, 2); b32(pix); b32(gc); b16(2); b16(2); b16(0); b16(0); b8(0); b8(24); b16(0);
    for (int i = 0; i < 4; i++) b32(image[i]);
    send();
    req(73, 2); b32(pix); b16(0); b16(0); b16(2); b16(2); b32(0xffffffff);
    s = send();
    int round = answer(s) == 1 && msg[1] == 24;
    for (int i = 0; round && i < 4; i++) round = (r32(msg + 32 + 4 * i) & 0xffffff) == image[i];
    req(62, 0); b32(pix); b32(win); b32(gc); b16(0); b16(0); b16(150); b16(60); b16(2); b16(2);
    send();
    req(73, 2); b32(win); b16(151); b16(61); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 copied = answer(s) == 1 ? r32(msg + 32) : 0;
    check(round && copied == 0xaabbcc, "PutImage, GetImage and CopyArea: a pixmap's pixels as put, and copied onto the window");

    // The extensions toolkits look for.
    static const char *exts[] = {"BIG-REQUESTS", "MIT-SHM", "SHAPE", "XC-MISC", "RENDER", "XKEYBOARD"};
    int found = 1;
    u32 big = 0, render = 0, xkb = 0;
    for (int i = 0; i < 6; i++) {
        req(98, 0); b16(len(exts[i])); b16(0); bytes(exts[i], len(exts[i]));
        s = send();
        found = found && answer(s) == 1 && msg[8] == 1;
        if (i == 0) big = msg[9];
        if (i == 4) render = msg[9];
        if (i == 5) xkb = msg[9];
    }
    req(98, 0); b16(7); b16(0); bytes("NOTHERE", 7);
    s = send();
    check(found && answer(s) == 1 && msg[8] == 0, "QueryExtension: BIG-REQUESTS, MIT-SHM, SHAPE, XC-MISC, RENDER and XKEYBOARD, and not one that isn't there");

    // BIG-REQUESTS: enabled, then a request with a length of 0 and the real one after (in words, itself included).
    req(big, 0);
    s = send();
    u32 most = answer(s) == 1 ? r32(msg + 8) : 0;
    start = nout; b8(70); b8(0); b16(0); b32(6); b32(win); b32(gc); b16(30); b16(70); b16(4); b16(4);
    sys(SYS_write, sock, (i64)out, nout, 0, 0, 0);
    nout = 0;
    ++seq;
    req(73, 2); b32(win); b16(31); b16(71); b16(1); b16(1); b32(0xffffffff);
    s = send();
    check(most >= 65536 && answer(s) == 1 && r32(msg + 32) == 0xff0000, "BIG-REQUESTS: enabled, and a request given its length past the header");

    // RENDER: its version, the formats for depths 24 and 8, a picture of the window.
    req(render, 0); b32(0); b32(11);
    s = send();
    int version = answer(s) == 1 && r32(msg + 8) == 0 && r32(msg + 12) == 11;
    req(render, 1);
    s = send();
    u32 fmt24 = 0, fmt8 = 0;
    if (answer(s) == 1)
        for (u32 i = 0; i < r32(msg + 8); i++) {
            const u8 *f = msg + 32 + 28 * i;
            if (f[5] == 24) fmt24 = r32(f);
            if (f[5] == 8) fmt8 = r32(f);
        }
    u32 pict = base | 10, green = base | 11, glyphs = base | 12;
    req(render, 4); b32(pict); b32(win); b32(fmt24); b32(0);
    send();
    check(version && fmt24 && fmt8, "RENDER: version 0.11, a format for depth 24 and an alpha one for 8");

    // Half-transparent red, premultiplied, over the window's background.
    req(render, 26); b8(3); b8(0); b16(0); b32(pict); b16(0x8000); b16(0); b16(0); b16(0x8000);
    b16(120); b16(20); b16(10); b16(10);
    send();
    req(73, 2); b32(win); b16(125); b16(25); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 blended = answer(s) == 1 ? r32(msg + 32) : 0;
    int near = 1;
    u32 want = 0x99334c;
    for (int sh = 0; sh < 24; sh += 8) {
        int d = (int)((blended >> sh) & 0xff) - (int)((want >> sh) & 0xff);
        if (d < -2 || d > 2) near = 0;
    }
    check(near, "RENDER FillRectangles: half-transparent red Over the background, blended");

    // A glyph (2 by 2, all covered) composited from a solid green fill.
    req(render, 17); b32(glyphs); b32(fmt8);
    send();
    req(render, 20); b32(glyphs); b32(1); b32(7); b16(2); b16(2); b16(0); b16(0); b16(2); b16(0);
    b32(0x0000ffff); b32(0x0000ffff);
    send();
    req(render, 33); b32(green); b16(0); b16(0xffff); b16(0); b16(0xffff);
    send();
    req(render, 23); b8(3); b8(0); b16(0); b32(green); b32(pict); b32(0); b32(glyphs); b16(0); b16(0);
    b8(1); b8(0); b8(0); b8(0); b16(140); b16(20); b8(7); b8(0); b8(0); b8(0);
    send();
    req(73, 2); b32(win); b16(141); b16(21); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 glyph = answer(s) == 1 ? r32(msg + 32) : 0;
    req(73, 2); b32(win); b16(142); b16(21); b16(1); b16(1); b32(0xffffffff);
    s = send();
    u32 beside = answer(s) == 1 ? r32(msg + 32) : 0;
    check(glyph == 0x00ff00 && beside == 0x336699, "RENDER glyphs: a glyph set, a glyph added and composited from a solid fill, and nothing past it");

    // XKEYBOARD: 1.0, the map's four key types and the keysyms of keycode 38 (a, A), its name, the state.
    req(xkb, 0); b16(1); b16(0);
    s = send();
    int using = answer(s) == 1 && msg[1] == 1 && r16(msg + 8) == 1;
    req(xkb, 8); b16(0x100); b16(1 | 2); b16(0); b8(0); b8(0); b8(0); b8(0); b8(0); b8(0); b8(0); b8(0);
    b16(0); b8(0); b8(0); b8(0); b8(0); b8(0); b8(0); b16(0);
    s = send();
    int mapped_keys = 0;
    if (answer(s) == 1 && msg[15] == 4 && msg[17] == 8) {
        u32 at = 40;
        for (int t = 0; t < 4; t++) {
            u32 entries = msg[at + 5], preserve = msg[at + 6];
            at += 8 + 8 * entries + (preserve ? 4 * entries : 0);
        }
        for (u32 key = 8; key < 38; key++) at += 8 + 4 * r16(msg + at + 6);
        mapped_keys = msg[at] == 2 && msg[at + 5] == 2 && r32(msg + at + 8) == 'a' && r32(msg + at + 12) == 'A';
    }
    req(xkb, 17); b16(0x100); b16(0); b32(512);
    s = send();
    int key_named = answer(s) == 1 && msg[18] == 8;
    const u8 *n = msg + 32 + (38 - 8) * 4;
    key_named = key_named && n[0] == 'A' && n[1] == 'C' && n[2] == '0' && n[3] == '1';
    req(xkb, 4); b16(0x100); b16(0);
    s = send();
    check(using && mapped_keys && key_named && answer(s) == 1 && msg[1] == 3 && msg[8] == 0,
          "XKEYBOARD: 1.0, four key types, keycode 38 alphabetic with a and A, named AC01, nothing held");

    // A selection owned.
    req(16, 0); b16(9); b16(0); bytes("CLIPBOARD", 9);
    s = send();
    u32 clipboard = answer(s) == 1 ? r32(msg + 8) : 0;
    req(22, 0); b32(win); b32(clipboard); b32(0);
    send();
    req(23, 0); b32(clipboard);
    s = send();
    check(answer(s) == 1 && r32(msg + 8) == win, "SetSelectionOwner and GetSelectionOwner: CLIPBOARD is the window's");

    // The keyboard: evdev's KEY_A (30) is keycode 38, a and A.
    req(101, 0); b8(38); b8(1); b16(0);
    s = send();
    check(answer(s) == 1 && msg[1] >= 2 && r32(msg + 32) == 'a' && r32(msg + 36) == 'A', "GetKeyboardMapping: keycode 38 is a, A");

    // Errors: a drawable that isn't one, and an ID that isn't the client's.
    req(14, 0); b32(0x1234);
    s = send();
    int bad_drawable = answer(s) == 0 && error_code == 9;
    req(53, 24); b32(0x1235); b32(win); b16(1); b16(1);
    s = send();
    req(43, 0);
    u16 sync = send();
    int k = answer(s);
    int bad_id = k == 0 && error_code == 14 && error_seq == s;
    answer(sync);
    check(bad_drawable && bad_id, "errors: Drawable for GetGeometry of nothing, IDChoice for another's ID");

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
