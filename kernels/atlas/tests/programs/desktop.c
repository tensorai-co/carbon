// The desktop on atlas, for ARM64 and x86-64: frames, activation, focus, and input through SendInput, as a user's
// clicks, typing and drags. No C runtime. tests/programs/build.sh builds it. It ends with a window still open, so
// the desktop's screenshot has it in its frame.

typedef void *HANDLE;
typedef void *HWND;
typedef unsigned int DWORD;
typedef unsigned int UINT;
typedef unsigned long long u64;
typedef long long LRESULT;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef LRESULT (__stdcall *WNDPROC)(HWND, UINT, u64, long long);
typedef struct { int left, top, right, bottom; } RECT;
typedef struct { int x, y; } POINT;
typedef struct { HWND hwnd; UINT message; u64 wParam; long long lParam; DWORD time; int x, y; DWORD priv; } MSG;
typedef struct { UINT size, style; WNDPROC proc; int cls, wnd; HANDLE instance, icon, cursor, background; const WCHAR *menu, *name; HANDLE small; } WNDCLASSEXW;
typedef struct { DWORD type; DWORD pad; union { struct { int dx, dy; DWORD data, flags, time; u64 extra; } mi; struct { unsigned short vk, scan; DWORD flags, time; u64 extra; } ki; }; } INPUT;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K unsigned short __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, u64, long long);
K BOOL __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K BOOL __stdcall TranslateMessage(const MSG *);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K BOOL __stdcall IsWindow(HWND);
K BOOL __stdcall ShowWindow(HWND, int);
K BOOL __stdcall GetClientRect(HWND, RECT *);
K BOOL __stdcall GetWindowRect(HWND, RECT *);
K BOOL __stdcall AdjustWindowRectEx(RECT *, DWORD, BOOL, DWORD);
K HWND __stdcall GetActiveWindow(void);
K HWND __stdcall GetForegroundWindow(void);
K BOOL __stdcall SetForegroundWindow(HWND);
K HWND __stdcall GetFocus(void);
K short __stdcall GetKeyState(int);
K UINT __stdcall SendInput(UINT, INPUT *, int);
K BOOL __stdcall SetCursorPos(int, int);
K BOOL __stdcall GetCursorPos(POINT *);
K BOOL __stdcall ClientToScreen(HWND, POINT *);
K BOOL __stdcall ScreenToClient(HWND, POINT *);
K HWND __stdcall WindowFromPoint(POINT);
K int __stdcall GetSystemMetrics(int);
K HANDLE __stdcall GetSysColorBrush(int);

#define WS_OVERLAPPEDWINDOW 0x00cf0000
#define WS_POPUP 0x80000000
#define WS_CHILD 0x40000000
#define WS_VISIBLE 0x10000000
#define WM_MOVE 3
#define WM_SIZE 5
#define WM_ACTIVATE 6
#define WM_SETFOCUS 7
#define WM_KILLFOCUS 8
#define WM_CLOSE 0x10
#define WM_KEYDOWN 0x100
#define WM_CHAR 0x102
#define WM_SYSKEYDOWN 0x104
#define WM_MOUSEMOVE 0x200
#define WM_LBUTTONDOWN 0x201
#define WM_LBUTTONUP 0x202
#define WM_MOUSEWHEEL 0x20a
#define MOUSEEVENTF_MOVE 1
#define MOUSEEVENTF_LEFTDOWN 2
#define MOUSEEVENTF_LEFTUP 4
#define MOUSEEVENTF_WHEEL 0x800
#define KEYEVENTF_KEYUP 2
#define KEYEVENTF_UNICODE 4

static HANDLE out;
static int failures;
static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { DWORD n; WriteFile(out, s, len(s), &n, 0); }
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static WCHAR wbuf[4][64];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }

static HWND a, b, c, child;
// What each window saw.
static int activated[3], deactivated[3], focused[3], unfocused[3], closed[3], moves, sizes;
static long long down_at, down_wparam, wheel;
static int down_window_ok, shift_seen_down, moved_x, moved_y, size_w, size_h;
static char typed[16];
static int ntyped;
static int which(HWND h) { return h == a ? 0 : h == b ? 1 : 2; }

static LRESULT __stdcall wndproc(HWND hwnd, UINT msg, u64 wp, long long lp) {
    int i = which(hwnd);
    switch (msg) {
    case WM_ACTIVATE: if (wp & 0xffff) activated[i]++; else deactivated[i]++; break;
    case WM_SETFOCUS: focused[i]++; break;
    case WM_KILLFOCUS: unfocused[i]++; break;
    case WM_CLOSE: closed[i]++; break;
    case WM_MOVE: if (hwnd == a) { moves++; moved_x = (short)(lp & 0xffff); moved_y = (short)((lp >> 16) & 0xffff); } break;
    case WM_SIZE: if (hwnd == a) { sizes++; size_w = (int)(lp & 0xffff); size_h = (int)((lp >> 16) & 0xffff); } break;
    case WM_LBUTTONDOWN: down_at = lp; down_wparam = (long long)wp; down_window_ok = hwnd == a; break;
    case WM_KEYDOWN: if (wp == 'A') shift_seen_down = GetKeyState(0x10) < 0; break;
    case WM_CHAR: if (ntyped < 15) typed[ntyped++] = (char)wp; return 0;
    case WM_MOUSEWHEEL: wheel = (long long)wp; return 0;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

/// Takes and dispatches every message there is, as a program's loop does.
static void pump(void) {
    MSG msg;
    while (PeekMessageW(&msg, 0, 0, 0, 1)) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

static INPUT inputs[8];
static int ninputs;
static void mouse(DWORD flags, int dx, int dy, DWORD data) {
    INPUT *in = &inputs[ninputs++];
    *in = (INPUT){0};
    in->type = 0;
    in->mi.dx = dx;
    in->mi.dy = dy;
    in->mi.data = data;
    in->mi.flags = flags;
}
static void key(unsigned short vk, unsigned short scan, DWORD flags) {
    INPUT *in = &inputs[ninputs++];
    *in = (INPUT){0};
    in->type = 1;
    in->ki.vk = vk;
    in->ki.scan = scan;
    in->ki.flags = flags;
}
static UINT send(void) {
    UINT n = SendInput((UINT)ninputs, inputs, sizeof(INPUT));
    UINT want = (UINT)ninputs;
    ninputs = 0;
    pump();
    return n == want;
}
/// A left click at a point on the screen.
static int click(int x, int y) {
    SetCursorPos(x, y);
    mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0);
    mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0);
    return send();
}
/// A left-button drag from a point on the screen, by (dx, dy).
static int drag(int x, int y, int dx, int dy) {
    SetCursorPos(x, y);
    mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0);
    mouse(MOUSEEVENTF_MOVE, dx / 2, dy / 2, 0);
    mouse(MOUSEEVENTF_MOVE, dx - dx / 2, dy - dy / 2, 0);
    mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0);
    return send();
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("the desktop on atlas, arm64\n");
#else
    say("the desktop on atlas, x86-64\n");
#endif
    WNDCLASSEXW wc = {sizeof wc, 0, wndproc, 0, 0, 0, 0, 0, GetSysColorBrush(5), 0, W(0, "CarbonDesktop"), 0};
    RegisterClassExW(&wc);
    RECT frame = {0, 0, 0, 0};
    AdjustWindowRectEx(&frame, WS_OVERLAPPEDWINDOW, 0, 0);
    int fl = -frame.left, ft = -frame.top, fr = frame.right, fb = frame.bottom;

    a = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "Window A"), WS_OVERLAPPEDWINDOW, 100, 100, 300, 200, 0, 0, 0, 0);
    b = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "Window B"), WS_OVERLAPPEDWINDOW, 250, 150, 300, 200, 0, 0, 0, 0);
    check(GetActiveWindow() == 0 && WindowFromPoint((POINT){200, 200}) == 0, "made hidden: nothing's active, and nothing's there");
    ShowWindow(a, 5);
    ShowWindow(b, 5);
    pump();
    check(GetActiveWindow() == b && GetForegroundWindow() == b && GetFocus() == b, "shown last, B is active and has the focus");
    check(activated[0] == 1 && deactivated[0] == 1 && focused[0] == 1 && unfocused[0] == 1 && activated[1] == 1 && focused[1] == 1,
          "A was activated and focused, then deactivated as B was shown");

    RECT r, cr;
    GetWindowRect(a, &r);
    GetClientRect(a, &cr);
    check(r.left == 100 && r.top == 100 && r.right == 400 && r.bottom == 300 && cr.right == 300 - fl - fr && cr.bottom == 200 - ft - fb,
          "GetWindowRect has the frame; GetClientRect is inside it");
    POINT p = {0, 0};
    ClientToScreen(a, &p);
    POINT q = {p.x + 5, p.y + 6};
    ScreenToClient(a, &q);
    check(p.x == 100 + fl && p.y == 100 + ft && q.x == 5 && q.y == 6, "ClientToScreen and ScreenToClient");
    check(WindowFromPoint((POINT){120, 150}) == a && WindowFromPoint((POINT){500, 300}) == b && WindowFromPoint((POINT){5, 5}) == 0,
          "WindowFromPoint: A where B doesn't cover it, B on top, nothing on the desktop");
    check(GetSystemMetrics(0) > 0 && GetSystemMetrics(4) > 0 && GetSystemMetrics(4) <= ft, "GetSystemMetrics: the screen and the caption");

    // A click in A's client area, where B doesn't cover it.
    check(click(p.x + 20, p.y + 30), "SendInput takes a click");
    POINT at;
    GetCursorPos(&at);
    check(at.x == p.x + 20 && at.y == p.y + 30, "SetCursorPos and GetCursorPos");
    check(down_window_ok && down_at == (30 << 16 | 20) && (down_wparam & 1), "WM_LBUTTONDOWN in A's client coordinates, with MK_LBUTTON");
    check(GetActiveWindow() == a && GetFocus() == a && deactivated[1] == 1 && unfocused[1] == 1 && activated[0] == 2,
          "clicking A activated it and gave it the focus, and B lost both");
    check(WindowFromPoint((POINT){300, 200}) == a, "A is on top of B now");

    // Typing: Shift+A, then b, then '!' as a character of its own.
    key(0x10, 0, 0);
    key('A', 0, 0);
    key('A', 0, KEYEVENTF_KEYUP);
    key(0x10, 0, KEYEVENTF_KEYUP);
    key('B', 0, 0);
    key('B', 0, KEYEVENTF_KEYUP);
    key(0, '!', KEYEVENTF_UNICODE);
    send();
    typed[ntyped] = 0;
    check(shift_seen_down, "GetKeyState: Shift down as of WM_KEYDOWN for A, though it's up by now");
    check(ntyped == 3 && typed[0] == 'A' && typed[1] == 'b' && typed[2] == '!', "TranslateMessage: WM_CHAR \"Ab!\" to the focused window");
    check(GetKeyState(0x10) >= 0, "and Shift is up once its WM_KEYUP is taken");

    mouse(MOUSEEVENTF_WHEEL, 0, 0, (DWORD)-120);
    send();
    check((short)(wheel >> 16) == -120, "WM_MOUSEWHEEL, a notch towards the user");

    // Dragging A's title bar by (40, 25).
    moves = 0;
    drag(r.left + 60, r.top + ft / 2, 40, 25);
    GetWindowRect(a, &r);
    check(r.left == 140 && r.top == 125 && r.right == 440 && r.bottom == 325, "dragging the title bar moves A");
    check(moves >= 1 && moved_x == 140 + fl && moved_y == 125 + ft, "with WM_MOVE where its client area is now");

    // Dragging its bottom right corner by (30, 20).
    sizes = 0;
    drag(r.right - 1, r.bottom - 1, 30, 20);
    GetWindowRect(a, &r);
    GetClientRect(a, &cr);
    check(r.right == 470 && r.bottom == 345 && cr.right == 330 - fl - fr, "dragging a corner resizes it");
    check(sizes >= 1 && size_w == cr.right && size_h == cr.bottom, "with WM_SIZE");

    // B's close button: B comes to the top first, then closes, and A is active again.
    RECT br;
    GetWindowRect(b, &br);
    click(br.right - fr - ft / 2 + 2, br.top + ft / 2 + 1);
    check(closed[1] == 1 && !IsWindow(b), "B's close button: WM_CLOSE, and DefWindowProc destroys it");
    check(GetActiveWindow() == a && GetFocus() == a, "A is active again, with the focus");

    // A popup has no frame; Alt+F4 closes it.
    c = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "popup"), WS_POPUP, 600, 400, 120, 80, 0, 0, 0, 0);
    ShowWindow(c, 5);
    pump();
    GetClientRect(c, &cr);
    check(cr.right == 120 && cr.bottom == 80 && GetActiveWindow() == c, "a popup has no frame, and is active when shown");
    key(0x12, 0, 0);
    key(0x73, 0, 0);
    key(0x73, 0, KEYEVENTF_KEYUP);
    key(0x12, 0, KEYEVENTF_KEYUP);
    send();
    check(closed[2] == 1 && !IsWindow(c) && GetActiveWindow() == a, "Alt+F4 closes it, and A is active again");

    check(SetForegroundWindow(a) && GetActiveWindow() == a && IsWindow(a), "A stays open, for the screenshot");
    ExitProcess(failures);
}
