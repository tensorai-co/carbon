// A freestanding Wayland client for lambda's compositor: no libc, no libwayland, just system calls and the wire
// format. It finds the server from its environment, binds the globals, gets a pointer and a keyboard (its keymap a
// file passed with SCM_RIGHTS, mapped and read) and the output, draws into a memfd it passes with SCM_RIGHTS, maps an
// xdg_toplevel through its configure and ack, is told it's on the output and has the keyboard, waits for a frame,
// sees a replaced buffer released, has the desktop's frame by asking for server-side decorations, opens a popup that's
// placed by its positioner, moved by `reposition` and dismissed when its parent's hidden, shows a subsurface, copies
// text and pastes it back through the clipboard (its own source asked for it through a pipe, the paste read from
// another), and sees a protocol error end the connection. Run it with a display
// (CARBON_DISPLAY=640x480) to have the server at all. With --wait (or WAYLAND_TEST_WAIT=1), it stays instead of erring, and puts each input
// event it's sent in its window's title, for `carbon display` to drive it and `carbon display windows` to read.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef int i32;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit_group = 94, SYS_close = 57, SYS_ftruncate = 46, SYS_mmap = 222, SYS_socket = 198,
       SYS_connect = 203, SYS_sendmsg = 211, SYS_recvmsg = 212, SYS_memfd_create = 279, SYS_munmap = 215,
       SYS_read = 63, SYS_pipe2 = 59 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ARCH "aarch64"
#else
enum { SYS_write = 1, SYS_close = 3, SYS_mmap = 9, SYS_munmap = 11, SYS_socket = 41, SYS_connect = 42, SYS_sendmsg = 46,
       SYS_recvmsg = 47, SYS_ftruncate = 77, SYS_exit_group = 231, SYS_memfd_create = 319, SYS_read = 0, SYS_pipe2 = 293 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ARCH "x86_64"
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
static int same(const char *a, const char *b) { u64 i = 0; for (; a[i] && b[i]; i++) if (a[i] != b[i]) return 0; return a[i] == b[i]; }
static const char *env(char **envp, const char *name) {
    u64 n = len(name);
    for (; *envp; envp++) {
        u64 i = 0;
        while (i < n && (*envp)[i] == name[i]) i++;
        if (i == n && (*envp)[n] == '=') return *envp + n + 1;
    }
    return "";
}

// ---- The wire ----

static int sock;
static u32 out[4096];
static u32 nout, start;
static void begin(u32 object, u32 opcode) { start = nout; out[nout++] = object; out[nout++] = opcode; }
static void put(u32 v) { out[nout++] = v; }
static void put_str(const char *s) {
    u32 n = (u32)len(s) + 1;
    put(n);
    char *p = (char *)&out[nout];
    for (u32 i = 0; i < n; i++) p[i] = s[i];
    nout += (n + 3) / 4;
}
static void end(void) { out[start + 1] |= (nout - start) * 4 << 16; }

struct iovec { void *base; u64 len; };
struct msghdr { void *name; u32 namelen; u32 pad; struct iovec *iov; u64 iovlen; void *control; u64 controllen; i32 flags; i32 pad2; };

// Sends what's been put, with a file if `fd` isn't -1.
static int flush(int fd) {
    struct iovec iov = {out, nout * 4};
    struct msghdr m = {0, 0, 0, &iov, 1, 0, 0, 0, 0};
    u64 control[3];
    if (fd >= 0) {
        control[0] = 20; // its length: the header's 16 bytes and one int
        control[1] = 1 | (u64)1 << 32; // SOL_SOCKET, SCM_RIGHTS
        control[2] = (u64)(u32)fd;
        m.control = control;
        m.controllen = 24;
    }
    i64 n = sys(SYS_sendmsg, sock, (i64)&m, 0, 0, 0, 0);
    int ok = n == nout * 4;
    nout = 0;
    return ok;
}

static u32 in[4096];
static u32 nin;
static int eof;
// Files the server sent, in the order its messages take them.
static int fds[16];
static u32 nfds, fd_at;
// Reads more of what the server sent, and the files with it.
static void more(void) {
    struct iovec iov = {(char *)in + nin * 4, sizeof in - nin * 4};
    u64 control[16];
    struct msghdr m = {0, 0, 0, &iov, 1, control, sizeof control, 0, 0};
    i64 n = sys(SYS_recvmsg, sock, (i64)&m, 0, 0, 0, 0);
    if (n <= 0) { eof = 1; return; }
    nin += (u32)n / 4;
    for (u64 at = 0; at + 16 <= m.controllen;) {
        u64 cmsg_len = *(u64 *)((char *)control + at);
        u32 *head = (u32 *)((char *)control + at + 8);
        if (cmsg_len < 16) break;
        if (head[0] == 1 && head[1] == 1)
            for (u64 i = 16; i + 4 <= cmsg_len && nfds < 16; i += 4) fds[nfds++] = *(int *)((char *)control + at + i);
        at += (cmsg_len + 7) & ~7ul;
    }
}
static int take_fd(void) { return fd_at < nfds ? fds[fd_at++] : -1; }

// ---- What the server said ----

static u32 compositor, shm, wm_base, seat, output, formats, configures, xdg_serial, frame_time, released, error_object, error_code;
static int synced, framed, toplevel_configured, closed, waiting;
static u32 capabilities, keymap_format, keymap_size, repeat_rate, repeat_delay, mode_w, mode_h, output_scale, output_done;
static u32 entered_output, preferred_scale, keyboard_entered, modifiers_seen, decoration_mode, deco_manager;
static i32 popup_x, popup_y, popup_w, popup_h;
static u32 popup_configures, popup_xdg_configures, repositioned, popup_done, popup_framed;
static u32 data_manager, sub_compositor, offer_id, offer_has_text, selection_offer, source_sent, sub_framed;
static const char copied[] = "copied on lambda";
static int keymap_ok, seat_named;
enum { DISPLAY = 1, REGISTRY = 2, SYNC1 = 3, COMPOSITOR = 4, SHM = 5, WM_BASE = 6, SYNC2 = 7, POOL = 8, BUFFER = 9,
       SURFACE = 10, XDG_SURFACE = 11, TOPLEVEL = 12, FRAME = 13, BUFFER2 = 14, SYNC3 = 15, SEAT = 16, OUTPUT = 17,
       POINTER = 18, KEYBOARD = 19, SYNC4 = 20, DECO_MANAGER = 21, DECORATION = 22, POSITIONER = 23, POPUP_SURFACE = 24,
       POPUP_XDG = 25, POPUP = 26, POPUP_BUFFER = 27, POPUP_FRAME = 28, POSITIONER2 = 29, SYNC5 = 30, DATA_MANAGER = 31,
       SOURCE = 32, DEVICE = 33, SUBCOMPOSITOR = 34, SUB_SURFACE = 35, SUBSURFACE = 36, SUB_BUFFER = 37, SUB_FRAME = 38 };

// The title, for --wait: what the last input event was.
static char title[96];
static u32 ntitle;
static void t_str(const char *s) { while (*s && ntitle < 90) title[ntitle++] = *s++; }
static void t_num(i32 v) {
    char b[12]; int n = 0; u32 u = (u32)(v < 0 ? -v : v);
    if (v < 0) t_str("-");
    do { b[n++] = (char)('0' + u % 10); u /= 10; } while (u && n < 11);
    while (n && ntitle < 90) title[ntitle++] = b[--n];
}
static void show(void) {
    title[ntitle] = 0;
    begin(TOPLEVEL, 2); put_str(title); end();
    flush(-1);
    ntitle = 0;
}

static void event(u32 object, u32 opcode, u32 *args) {
    if (object == REGISTRY && opcode == 0) {
        const char *name = (const char *)&args[2];
        if (same(name, "wl_compositor")) compositor = args[0];
        if (same(name, "wl_shm")) shm = args[0];
        if (same(name, "xdg_wm_base")) wm_base = args[0];
        if (same(name, "wl_seat")) seat = args[0];
        if (same(name, "wl_output")) output = args[0];
        if (same(name, "zxdg_decoration_manager_v1")) deco_manager = args[0];
        if (same(name, "wl_data_device_manager")) data_manager = args[0];
        if (same(name, "wl_subcompositor")) sub_compositor = args[0];
    } else if (object == DEVICE && opcode == 0) {
        offer_id = args[0];
    } else if (offer_id && object == offer_id && opcode == 0) {
        if (same((const char *)&args[1], "text/plain;charset=utf-8")) offer_has_text = 1;
    } else if (object == DEVICE && opcode == 5) {
        selection_offer = args[0];
    } else if (object == SOURCE && opcode == 1) {
        // Asked for what it copied: written into the server's pipe, and its end closed.
        int fd = take_fd();
        if (fd >= 0) {
            sys(SYS_write, fd, (i64)copied, sizeof copied - 1, 0, 0, 0);
            sys(SYS_close, fd, 0, 0, 0, 0, 0);
            source_sent = 1;
        }
    } else if (object == SUB_FRAME && opcode == 0) {
        sub_framed = 1;
    } else if (object == DECORATION && opcode == 0) {
        decoration_mode = args[0];
    } else if (object == POPUP && opcode == 0) {
        popup_x = (i32)args[0]; popup_y = (i32)args[1]; popup_w = (i32)args[2]; popup_h = (i32)args[3];
        popup_configures++;
    } else if (object == POPUP && opcode == 1) {
        popup_done++;
    } else if (object == POPUP && opcode == 2) {
        repositioned = args[0];
    } else if (object == POPUP_XDG && opcode == 0) {
        popup_xdg_configures++;
        begin(POPUP_XDG, 4); put(args[0]); end();
        flush(-1);
    } else if (object == POPUP_FRAME && opcode == 0) {
        popup_framed = 1;
    } else if (object == SEAT && opcode == 0) {
        capabilities = args[0];
    } else if (object == SEAT && opcode == 1) {
        seat_named = same((const char *)&args[1], "seat0");
    } else if (object == KEYBOARD && opcode == 0) {
        // The keymap: its format, its file, its size with its NUL.
        keymap_format = args[0];
        keymap_size = args[1];
        int fd = take_fd();
        const char *map = fd >= 0 ? (const char *)sys(SYS_mmap, 0, keymap_size, 1, 2, fd, 0) : 0;
        if (map && (i64)map > 0) {
            const char *start = "xkb_keymap {";
            int ok = 1;
            for (int i = 0; start[i]; i++) if (map[i] != start[i]) ok = 0;
            keymap_ok = ok && map[keymap_size - 1] == 0;
            sys(SYS_munmap, (i64)map, keymap_size, 0, 0, 0, 0);
        }
        if (fd >= 0) sys(SYS_close, fd, 0, 0, 0, 0, 0);
    } else if (object == KEYBOARD && opcode == 5) {
        repeat_rate = args[0];
        repeat_delay = args[1];
    } else if (object == KEYBOARD && opcode == 1) {
        keyboard_entered = args[1];
    } else if (object == KEYBOARD && opcode == 4) {
        modifiers_seen++;
        if (waiting) { t_str("mods "); t_num((i32)args[1]); t_str(" locked "); t_num((i32)args[3]); show(); }
    } else if (object == KEYBOARD && opcode == 3 && waiting) {
        t_str("key "); t_num((i32)args[2]); t_str(args[3] ? " down" : " up"); show();
    } else if (object == POINTER && waiting && opcode <= 4) {
        static const char *what[] = {"enter ", "leave", "motion ", "button ", "axis "};
        t_str(what[opcode]);
        if (opcode == 0) { t_num((i32)args[2] >> 8); t_str(","); t_num((i32)args[3] >> 8); }
        if (opcode == 2) { t_num((i32)args[1] >> 8); t_str(","); t_num((i32)args[2] >> 8); }
        if (opcode == 3) { t_num((i32)args[2]); t_str(args[3] ? " down" : " up"); }
        if (opcode == 4) { t_num((i32)args[1]); t_str(" "); t_num((i32)args[2] >> 8); }
        show();
    } else if (object == POINTER && waiting && opcode == 9) {
        t_str("value120 "); t_num((i32)args[0]); t_str(" "); t_num((i32)args[1]); show();
    } else if (object == OUTPUT && opcode == 1) {
        mode_w = args[1];
        mode_h = args[2];
    } else if (object == OUTPUT && opcode == 3) {
        output_scale = args[0];
    } else if (object == OUTPUT && opcode == 2) {
        output_done++;
    } else if (object == SURFACE && opcode == 0) {
        entered_output = args[0];
    } else if (object == SURFACE && opcode == 2) {
        preferred_scale = args[0];
    } else if (object == TOPLEVEL && opcode == 1) {
        closed = 1;
    } else if (object == DISPLAY && opcode == 0) {
        error_object = args[0];
        error_code = args[1];
    } else if ((object == SYNC1 || object == SYNC2 || object == SYNC3 || object == SYNC4 || object == SYNC5) && opcode == 0) {
        synced = (int)object;
    } else if (object == SHM && opcode == 0) {
        formats |= 1u << args[0];
    } else if (object == TOPLEVEL && opcode == 0) {
        toplevel_configured++;
    } else if (object == XDG_SURFACE && opcode == 0) {
        xdg_serial = args[0];
        configures++;
        // Each configure acknowledged at once.
        begin(XDG_SURFACE, 4); put(xdg_serial); end();
        flush(-1);
    } else if (object == FRAME && opcode == 0) {
        framed = 1;
        frame_time = args[0];
    } else if (object == BUFFER && opcode == 0) {
        released++;
    }
}

// Reads events until `done` says so, or the connection ends.
static void until(int (*done)(void)) {
    while (!done() && !eof) {
        u32 at = 0;
        while (nin - at >= 2 && nin - at >= (in[at + 1] >> 16) / 4) {
            u32 size = (in[at + 1] >> 16) / 4;
            if (size < 2) { eof = 1; return; }
            event(in[at], in[at + 1] & 0xffff, &in[at + 2]);
            at += size;
        }
        for (u32 i = at; i < nin; i++) in[i - at] = in[i];
        nin -= at;
        if (!done()) more();
    }
}
static int synced1(void) { return synced == SYNC1; }
static int synced2(void) { return synced == SYNC2; }
static int synced3(void) { return synced == SYNC3; }
static int synced4(void) { return synced == SYNC4; }
static int got_close(void) { return closed; }
static int synced5(void) { return synced == SYNC5; }
static int popup_configured(void) { return popup_xdg_configures > 0; }
static int popup_shown(void) { return popup_framed; }
static int got_repositioned(void) { return repositioned != 0 && popup_xdg_configures > 1; }
static int got_popup_done(void) { return popup_done > 0; }
static int got_selection(void) { return selection_offer != 0 && selection_offer == offer_id; }
static int got_send(void) { return source_sent; }
static int sub_shown(void) { return sub_framed; }
static int configured(void) { return configures > 0; }
static int got_frame(void) { return framed; }
static int got_release(void) { return released > 0; }
static int never(void) { return 0; }

void cmain(i64 *sp) {
    char **argv = (char **)(sp + 1);
    char **envp = (char **)(sp + 1 + sp[0] + 1);
    waiting = (sp[0] > 1 && same(argv[1], "--wait")) || same(env(envp, "WAYLAND_TEST_WAIT"), "1");
    say("a Wayland client on " ARCH "\n");
    const char *dir = env(envp, "XDG_RUNTIME_DIR"), *display = env(envp, "WAYLAND_DISPLAY");
    check(same(dir, "/run/user/0") && same(display, "wayland-0"), "XDG_RUNTIME_DIR and WAYLAND_DISPLAY say where the server is");

    struct { unsigned short family; char path[108]; } addr = {1, {0}};
    u64 n = 0;
    for (const char *p = dir; *p; p++) addr.path[n++] = *p;
    addr.path[n++] = '/';
    for (const char *p = display; *p; p++) addr.path[n++] = *p;
    sock = (int)sys(SYS_socket, 1, 1 | 02000000, 0, 0, 0, 0);
    check(sock >= 0 && sys(SYS_connect, sock, (i64)&addr, 2 + n + 1, 0, 0, 0) == 0, "connect to $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY");

    begin(DISPLAY, 1); put(REGISTRY); end();
    begin(DISPLAY, 0); put(SYNC1); end();
    flush(-1);
    until(synced1);
    check(compositor && shm && wm_base && seat && output, "wl_registry lists wl_compositor, wl_shm, xdg_wm_base, wl_seat and wl_output");

    begin(REGISTRY, 0); put(compositor); put_str("wl_compositor"); put(6); put(COMPOSITOR); end();
    begin(REGISTRY, 0); put(shm); put_str("wl_shm"); put(1); put(SHM); end();
    begin(REGISTRY, 0); put(wm_base); put_str("xdg_wm_base"); put(3); put(WM_BASE); end();
    begin(DISPLAY, 0); put(SYNC2); end();
    flush(-1);
    until(synced2);
    check(formats == 3, "bound, and wl_shm says ARGB8888 and XRGB8888");

    begin(REGISTRY, 0); put(seat); put_str("wl_seat"); put(8); put(SEAT); end();
    begin(REGISTRY, 0); put(output); put_str("wl_output"); put(4); put(OUTPUT); end();
    begin(SEAT, 0); put(POINTER); end();
    begin(SEAT, 1); put(KEYBOARD); end();
    begin(DISPLAY, 0); put(SYNC4); end();
    flush(-1);
    until(synced4);
    check(capabilities == 3 && seat_named, "wl_seat: a pointer and a keyboard, called seat0");
    check(keymap_format == 1 && keymap_ok, "wl_keyboard.keymap: an XKB keymap in a file passed with it, mapped and read");
    check(repeat_rate > 0 && repeat_delay > 0, "wl_keyboard.repeat_info: the desktop's rate and delay");
    check(mode_w >= 64 && mode_h >= 64 && output_scale == 1 && output_done >= 1, "wl_output: its mode (the desktop's size), scale 1, done");

    // 200 by 120: red above, blue below, a green square at (10, 10). After it, the popup's 80 by 40, yellow.
    enum { W = 200, H = 120, SIZE = W * H * 4, PW = 80, PH = 40, SUB = 20, SUB_AT = SIZE + PW * PH * 4, POOL_SIZE = SUB_AT + SUB * SUB * 4 };
    int fd = (int)sys(SYS_memfd_create, (i64)"wl_shm", 1, 0, 0, 0, 0);
    check(fd >= 0 && sys(SYS_ftruncate, fd, POOL_SIZE, 0, 0, 0, 0) == 0, "a memfd for the pool");
    u32 *pixels = (u32 *)sys(SYS_mmap, 0, POOL_SIZE, 3, 1, fd, 0);
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++)
            pixels[y * W + x] = (x >= 10 && x < 30 && y >= 10 && y < 30) ? 0x00ff00 : y < H / 2 ? 0xff0000 : 0x0000ff;
    for (int i = 0; i < PW * PH; i++) pixels[W * H + i] = 0xffff00;
    for (int i = 0; i < SUB * SUB; i++) pixels[SUB_AT / 4 + i] = 0x00ffff;

    begin(SHM, 0); put(POOL); put(POOL_SIZE); end(); // create_pool: its file goes beside it
    begin(POOL, 0); put(BUFFER); put(0); put(W); put(H); put(W * 4); put(1); end();
    begin(COMPOSITOR, 0); put(SURFACE); end();
    begin(WM_BASE, 2); put(XDG_SURFACE); put(SURFACE); end();
    begin(XDG_SURFACE, 1); put(TOPLEVEL); end();
    begin(TOPLEVEL, 2); put_str("wayland test"); end();
    // The desktop's frame, asked for before the first commit.
    begin(REGISTRY, 0); put(deco_manager); put_str("zxdg_decoration_manager_v1"); put(1); put(DECO_MANAGER); end();
    begin(DECO_MANAGER, 1); put(DECORATION); put(TOPLEVEL); end();
    begin(DECORATION, 1); put(2); end();
    begin(SURFACE, 6); end(); // the first commit, with no buffer: for the first configure
    // The file goes with the first message, as libwayland sends it.
    check(flush(fd), "sendmsg with SCM_RIGHTS");
    until(configured);
    check(toplevel_configured >= 1 && configures >= 1, "xdg_toplevel.configure and xdg_surface.configure, acknowledged");
    check(deco_manager && decoration_mode == 2, "zxdg_toplevel_decoration_v1: server-side, as asked");

    begin(SURFACE, 1); put(BUFFER); put(0); put(0); end();
    begin(SURFACE, 2); put(0); put(0); put(W); put(H); end();
    begin(SURFACE, 3); put(FRAME); end();
    begin(SURFACE, 6); end();
    flush(-1);
    until(got_frame);
    check(framed, "attached, committed, and a frame callback done: the window's on the desktop");
    check(entered_output == OUTPUT && preferred_scale == 1, "wl_surface.enter for the output, and its preferred buffer scale");
    check(keyboard_entered == SURFACE && modifiers_seen >= 1, "shown and made active, it has the keyboard: enter and modifiers");

    // Another buffer in its place: the first is given back.
    begin(POOL, 0); put(BUFFER2); put(0); put(W); put(H); put(W * 4); put(1); end();
    begin(SURFACE, 1); put(BUFFER2); put(0); put(0); end();
    begin(SURFACE, 6); end();
    begin(DISPLAY, 0); put(SYNC3); end();
    flush(-1);
    until(got_release);
    check(released == 1, "the buffer it replaced, released");

    // A popup: 80 by 40, from the bottom right of (10, 10, 20, 20) in its parent, going right and down.
    begin(WM_BASE, 1); put(POSITIONER); end();
    begin(POSITIONER, 1); put(PW); put(PH); end();
    begin(POSITIONER, 2); put(10); put(10); put(20); put(20); end();
    begin(POSITIONER, 3); put(8); end();
    begin(POSITIONER, 4); put(8); end();
    begin(COMPOSITOR, 0); put(POPUP_SURFACE); end();
    begin(WM_BASE, 2); put(POPUP_XDG); put(POPUP_SURFACE); end();
    begin(POPUP_XDG, 2); put(POPUP); put(XDG_SURFACE); put(POSITIONER); end();
    begin(POPUP_SURFACE, 6); end();
    flush(-1);
    until(popup_configured);
    check(popup_configures == 1 && popup_x == 30 && popup_y == 30 && popup_w == PW && popup_h == PH,
          "xdg_popup.configure: placed by its positioner, at (30, 30) in its parent, 80 by 40");
    begin(POOL, 0); put(POPUP_BUFFER); put(SIZE); put(PW); put(PH); put(PW * 4); put(1); end();
    begin(POPUP_SURFACE, 1); put(POPUP_BUFFER); put(0); put(0); end();
    begin(POPUP_SURFACE, 3); put(POPUP_FRAME); end();
    begin(POPUP_SURFACE, 6); end();
    flush(-1);
    until(popup_shown);
    check(popup_framed, "the popup shown, with its frame done");
    // Moved by 5 and 5: told so with the token, then where.
    begin(WM_BASE, 1); put(POSITIONER2); end();
    begin(POSITIONER2, 1); put(PW); put(PH); end();
    begin(POSITIONER2, 2); put(10); put(10); put(20); put(20); end();
    begin(POSITIONER2, 3); put(8); end();
    begin(POSITIONER2, 4); put(8); end();
    begin(POSITIONER2, 6); put(5); put(5); end();
    begin(POPUP, 2); put(POSITIONER2); put(77); end();
    flush(-1);
    until(got_repositioned);
    check(repositioned == 77 && popup_x == 35 && popup_y == 35, "xdg_popup.reposition: repositioned with its token, at (35, 35)");

    // A subsurface: a cyan square at (150, 80) on the window.
    begin(REGISTRY, 0); put(sub_compositor); put_str("wl_subcompositor"); put(1); put(SUBCOMPOSITOR); end();
    begin(COMPOSITOR, 0); put(SUB_SURFACE); end();
    begin(SUBCOMPOSITOR, 1); put(SUBSURFACE); put(SUB_SURFACE); put(SURFACE); end();
    begin(SUBSURFACE, 1); put(150); put(80); end();
    begin(POOL, 0); put(SUB_BUFFER); put(SUB_AT); put(SUB); put(SUB); put(SUB * 4); put(1); end();
    begin(SUB_SURFACE, 1); put(SUB_BUFFER); put(0); put(0); end();
    begin(SUB_SURFACE, 3); put(SUB_FRAME); end();
    begin(SUB_SURFACE, 6); end();
    begin(SURFACE, 6); end();
    flush(-1);
    until(sub_shown);
    check(sub_compositor && sub_framed, "wl_subcompositor: a subsurface shown, with its frame done");

    // The clipboard: text copied, offered back as the selection, and pasted through a pipe.
    begin(REGISTRY, 0); put(data_manager); put_str("wl_data_device_manager"); put(3); put(DATA_MANAGER); end();
    begin(DATA_MANAGER, 1); put(DEVICE); put(SEAT); end();
    begin(DATA_MANAGER, 0); put(SOURCE); end();
    begin(SOURCE, 0); put_str("text/plain;charset=utf-8"); end();
    begin(DEVICE, 1); put(SOURCE); put(0); end();
    flush(-1);
    until(got_selection);
    check(data_manager && offer_has_text, "wl_data_device: its own copy offered back, as text, with selection");
    int ends[2];
    sys(SYS_pipe2, (i64)ends, 0, 0, 0, 0, 0);
    begin(offer_id, 1); put_str("text/plain;charset=utf-8"); end(); // receive: the pipe's write end goes beside it
    flush(ends[1]);
    sys(SYS_close, ends[1], 0, 0, 0, 0, 0);
    until(got_send);
    char pasted[64];
    i64 got = 0, part;
    while ((part = sys(SYS_read, ends[0], (i64)pasted + got, sizeof pasted - 1 - got, 0, 0, 0)) > 0) got += part;
    pasted[got] = 0;
    sys(SYS_close, ends[0], 0, 0, 0, 0, 0);
    check(source_sent && same(pasted, copied), "wl_data_offer.receive: what its wl_data_source sent, pasted through the desktop");

    if (waiting) {
        t_str("waiting for input"); show();
        until(got_close);
        sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
    }

    // Its parent hidden: the popup's dismissed.
    begin(SURFACE, 1); put(0); put(0); put(0); end();
    begin(SURFACE, 6); end();
    begin(DISPLAY, 0); put(SYNC5); end();
    flush(-1);
    until(got_popup_done);
    check(popup_done == 1, "its parent hidden, xdg_popup.popup_done");

    // A request to an object there isn't: a protocol error, and the connection ends.
    begin(999, 0); end();
    flush(-1);
    until(never);
    check(error_object == 1 && error_code == 0 && eof, "an object that isn't there: wl_display.error, and the end");

    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
