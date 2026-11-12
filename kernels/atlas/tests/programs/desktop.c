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
K LRESULT __stdcall SendMessageW(HWND, UINT, u64, long long);
K BOOL __stdcall IsZoomed(HWND);
K HWND __stdcall SetCapture(HWND);
K HANDLE __stdcall GlobalAlloc(UINT, unsigned long long);
K void *__stdcall GlobalLock(HANDLE);
K BOOL __stdcall GlobalUnlock(HANDLE);
K unsigned long long __stdcall GlobalSize(HANDLE);
K BOOL __stdcall OpenClipboard(HWND);
K BOOL __stdcall CloseClipboard(void);
K BOOL __stdcall EmptyClipboard(void);
K HANDLE __stdcall SetClipboardData(UINT, HANDLE);
K HANDLE __stdcall GetClipboardData(UINT);
K BOOL __stdcall IsClipboardFormatAvailable(UINT);
K int __stdcall CountClipboardFormats(void);
K UINT __stdcall EnumClipboardFormats(UINT);
K HWND __stdcall GetClipboardOwner(void);
K HANDLE __stdcall LoadCursorW(HANDLE, const WCHAR *);
K HANDLE __stdcall SetCursor(HANDLE);
K HANDLE __stdcall GetCursor(void);
K HWND __stdcall GetCapture(void);
K BOOL __stdcall ReleaseCapture(void);
K BOOL __stdcall FlashWindow(HWND, BOOL);
typedef struct { UINT cbSize; HWND hwnd; DWORD dwFlags; UINT uCount; DWORD dwTimeout; } FLASHWINFO;
K BOOL __stdcall FlashWindowEx(FLASHWINFO *);
K BOOL __stdcall ClipCursor(const RECT *);
K BOOL __stdcall GetClipCursor(RECT *);
K long long __stdcall GetWindowLongPtrW(HWND, int);
K BOOL __stdcall DestroyWindow(HWND);
K BOOL __stdcall SetWindowPos(HWND, HWND, int, int, int, int, UINT);
K HWND __stdcall GetWindow(HWND, UINT);
K HWND __stdcall GetTopWindow(HWND);
K BOOL __stdcall EnumWindows(BOOL(__stdcall *)(HWND, long long), long long);
K void __stdcall DragAcceptFiles(HWND, BOOL);
K UINT __stdcall DragQueryFileW(HANDLE, UINT, WCHAR *, UINT);

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
#define WM_NCLBUTTONDOWN 0xa1
#define WM_MOUSEHWHEEL 0x20e
#define MOUSEEVENTF_HWHEEL 0x1000
#define WM_SYSCOMMAND 0x112
#define HTCAPTION 2
#define SC_MAXIMIZE 0xf030
#define SC_RESTORE 0xf120
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
static long long down_at, down_wparam, wheel, hwheel;
static int clipboard_lost;
static int r_downs, r_first_repeat, r_then_repeat;
static HWND enumerated_first;
static BOOL __stdcall first_window(HWND h, long long lp) { (void)lp; enumerated_first = h; return 0; }
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
    // The popup draws its own title bar: a press on it is one, so it asks to be moved, as such programs do.
    case WM_LBUTTONDOWN:
        if (hwnd == c) { SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 0); return 0; }
        down_at = lp; down_wparam = (long long)wp; down_window_ok = hwnd == a; break;
    case WM_KEYDOWN:
        if (wp == 'A') shift_seen_down = GetKeyState(0x10) < 0;
        // Bit 30: the key was down already, so this is its repeat.
        if (wp == 'R') { if (r_downs++ == 0) r_first_repeat = (int)((lp >> 30) & 1); else r_then_repeat = (int)((lp >> 30) & 1); }
        break;
    case WM_CHAR: if (ntyped < 15) typed[ntyped++] = (char)wp; return 0;
    case WM_MOUSEWHEEL: wheel = (long long)wp; return 0;
    case WM_MOUSEHWHEEL: hwheel = (long long)wp; return 0;
    case 0x307: if (hwnd == a) clipboard_lost++; return 0;
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
    // A character no key on the keyboard types: WM_CHAR all the same.
    key(0, 0xe9, KEYEVENTF_UNICODE);
    key(0, 0xe9, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
    send();
    check(ntyped == 4 && (unsigned char)typed[3] == 0xe9, "KEYEVENTF_UNICODE with a character no key types: its WM_CHAR");
    HANDLE ibeam = LoadCursorW(0, (const WCHAR *)32513);
    SetCursor(ibeam);
    check(ibeam != 0 && GetCursor() == ibeam, "LoadCursorW(IDC_IBEAM), SetCursor and GetCursor");

    mouse(MOUSEEVENTF_WHEEL, 0, 0, (DWORD)-120);
    send();
    check((short)(wheel >> 16) == -120, "WM_MOUSEWHEEL, a notch towards the user");
    mouse(MOUSEEVENTF_HWHEEL, 0, 0, 60);
    send();
    check((short)(hwheel >> 16) == 60, "WM_MOUSEHWHEEL, half a notch to the right, as it came");

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
    // Pressed (its WM_LBUTTONDOWN sends WM_NCLBUTTONDOWN with HTCAPTION), then dragged by (50, -40) and let go.
    RECT pr;
    SetCursorPos(610, 410);
    mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0);
    send();
    mouse(MOUSEEVENTF_MOVE, 30, 0, 0);
    mouse(MOUSEEVENTF_MOVE, 20, -40, 0);
    mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0);
    send();
    GetWindowRect(c, &pr);
    check(pr.left == 650 && pr.top == 360 && pr.right == 770 && pr.bottom == 440, "a window that draws its own title bar moves by WM_NCLBUTTONDOWN with HTCAPTION");
    SendMessageW(c, WM_SYSCOMMAND, SC_MAXIMIZE, 0);
    pump();
    GetWindowRect(c, &pr);
    check(IsZoomed(c) && pr.left == 0 && pr.top == 0 && pr.right == GetSystemMetrics(0) && pr.bottom == GetSystemMetrics(1), "WM_SYSCOMMAND's SC_MAXIMIZE: the whole screen, and IsZoomed");
    SendMessageW(c, WM_SYSCOMMAND, SC_RESTORE, 0);
    pump();
    GetWindowRect(c, &pr);
    check(!IsZoomed(c) && pr.left == 650 && pr.top == 360 && pr.right == 770, "SC_RESTORE: back where it was");
    key(0x12, 0, 0);
    key(0x73, 0, 0);
    key(0x73, 0, KEYEVENTF_KEYUP);
    key(0x12, 0, KEYEVENTF_KEYUP);
    send();
    check(closed[2] == 1 && !IsWindow(c) && GetActiveWindow() == a, "Alt+F4 closes it, and A is active again");

    // The clipboard: A copies "h\u00e9llo" as CF_UNICODETEXT, in global memory.
    static const WCHAR hello[] = {'h', 0xe9, 'l', 'l', 'o', 0};
    HANDLE g = GlobalAlloc(2, sizeof hello);
    WCHAR *gp = GlobalLock(g);
    for (int i = 0; i < 6; i++) gp[i] = hello[i];
    GlobalUnlock(g);
    check(g && GlobalSize(g) == sizeof hello, "GlobalAlloc, GlobalLock and GlobalSize");
    check(OpenClipboard(a) && EmptyClipboard() && SetClipboardData(13, g) == g && CloseClipboard(), "OpenClipboard, EmptyClipboard, SetClipboardData(CF_UNICODETEXT), CloseClipboard");
    check(GetClipboardOwner() == a && IsClipboardFormatAvailable(13) && IsClipboardFormatAvailable(1) && CountClipboardFormats() == 3 &&
          EnumClipboardFormats(0) == 13 && EnumClipboardFormats(13) == 1, "its owner, and its text as CF_UNICODETEXT, CF_TEXT and CF_OEMTEXT");
    check(OpenClipboard(0), "OpenClipboard(NULL) to paste");
    const WCHAR *wide = GlobalLock(GetClipboardData(13));
    const unsigned char *narrow = GlobalLock(GetClipboardData(1));
    check(wide && wide[1] == 0xe9 && wide[4] == 'o' && wide[5] == 0, "pasted as CF_UNICODETEXT");
    check(narrow && narrow[0] == 'h' && narrow[1] == 0xe9 && narrow[4] == 'o' && narrow[5] == 0, "and as CF_TEXT, made from it");
    CloseClipboard();
    // Another window takes it: A hears so.
    HWND other = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "other"), WS_OVERLAPPEDWINDOW, 10, 10, 100, 100, 0, 0, 0, 0);
    check(OpenClipboard(other) && EmptyClipboard() && CloseClipboard(), "another window empties it");
    pump();
    check(clipboard_lost == 1 && GetClipboardOwner() == other && CountClipboardFormats() == 0, "A's WM_DESTROYCLIPBOARD; the other owns it, empty");

    // A has the capture: a click on the empty desktop is its, in its own client coordinates.
    POINT origin = {0, 0};
    ClientToScreen(a, &origin);
    check(SetCapture(a) == 0 && GetCapture() == a, "SetCapture, and GetCapture");
    down_window_ok = 0;
    click(1000, 700);
    check(down_window_ok && (short)(down_at & 0xffff) == 1000 - origin.x && (short)((down_at >> 16) & 0xffff) == 700 - origin.y,
          "with the capture, a click anywhere is A's WM_LBUTTONDOWN, in its client coordinates");
    check(ReleaseCapture() && GetCapture() == 0, "ReleaseCapture");

    // A key held down: its repeat is WM_KEYDOWN again, with the previous state set.
    key('R', 0, 0);
    key('R', 0, 0);
    key('R', 0, KEYEVENTF_KEYUP);
    send();
    check(r_downs == 2 && !r_first_repeat && r_then_repeat, "a held key's repeat: WM_KEYDOWN with lParam's bit 30");

    // The active window's process may make its own windows active; a flash says whether it was.
    FLASHWINFO flash = {sizeof flash, other, 3, 2, 0};
    check(FlashWindow(a, 1) && !FlashWindowEx(&flash), "FlashWindow and FlashWindowEx: whether it was active");

    // ClipCursor keeps the pointer in a rectangle, until it's let go.
    RECT clip = {origin.x + 10, origin.y + 10, origin.x + 60, origin.y + 40}, got;
    POINT pt;
    check(ClipCursor(&clip) && GetClipCursor(&got) && got.left == clip.left && got.top == clip.top && got.right == clip.right && got.bottom == clip.bottom,
          "ClipCursor, and GetClipCursor says it");
    SetCursorPos(0, 0);
    GetCursorPos(&pt);
    int top_left = pt.x == clip.left && pt.y == clip.top;
    SetCursorPos(2000, 2000);
    GetCursorPos(&pt);
    check(top_left && pt.x == clip.right - 1 && pt.y == clip.bottom - 1, "the pointer stays inside it");
    check(ClipCursor(0) && GetClipCursor(&got) && got.left == 0 && got.top == 0 && got.right == GetSystemMetrics(0) && got.bottom == GetSystemMetrics(1),
          "ClipCursor(NULL) lets it go: GetClipCursor says the whole screen");
    SetCursorPos(5, 5);
    GetCursorPos(&pt);
    check(pt.x == 5 && pt.y == 5, "and it goes anywhere again");

    // A window that takes files dropped on it says so with WS_EX_ACCEPTFILES; an HDROP that isn't one holds nothing.
    DragAcceptFiles(a, 1);
    int accepts = (GetWindowLongPtrW(a, -20) & 0x10) != 0;
    DragAcceptFiles(a, 0);
    check(accepts && !(GetWindowLongPtrW(a, -20) & 0x10) && DragQueryFileW((HANDLE)0x1234, 0xffffffff, 0, 0) == 0,
          "DragAcceptFiles sets and clears WS_EX_ACCEPTFILES; DragQueryFileW on no HDROP");

    // Stacking: P and Q overlap at (900, 600), shown without being activated.
    const UINT keep = 0x1 | 0x2 | 0x10; // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
    HWND active = GetActiveWindow();
    HWND wp = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "P"), WS_OVERLAPPEDWINDOW, 850, 550, 200, 150, 0, 0, 0, 0);
    HWND wq = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "Q"), WS_OVERLAPPEDWINDOW, 870, 570, 200, 150, 0, 0, 0, 0);
    ShowWindow(wp, 4);
    ShowWindow(wq, 4);
    EnumWindows(first_window, 0);
    check(WindowFromPoint((POINT){900, 600}) == wq && GetTopWindow(0) == wq && GetWindow(wq, 2) == wp && GetWindow(wp, 3) == wq && enumerated_first == wq,
          "shown last, Q is on top: GetTopWindow, GetWindow's next and previous, EnumWindows top first");
    SetWindowPos(wq, (HWND)1, 0, 0, 0, 0, keep);
    check(WindowFromPoint((POINT){900, 600}) == wp && GetWindow(GetTopWindow(0), 1) == wq, "SetWindowPos(HWND_BOTTOM): below every other");
    SetWindowPos(wq, wp, 0, 0, 0, 0, keep);
    check(GetWindow(wp, 2) == wq && WindowFromPoint((POINT){900, 600}) == wp, "SetWindowPos after P: just behind it");
    // A window P owns is kept above it, and is topmost when P is.
    HWND owned = CreateWindowExW(0, W(0, "CarbonDesktop"), W(1, "owned"), WS_OVERLAPPEDWINDOW, 860, 560, 100, 100, wp, 0, 0, 0);
    ShowWindow(owned, 4);
    SetWindowPos(wq, 0, 0, 0, 0, 0, keep);
    SetWindowPos(wp, (HWND)-1, 0, 0, 0, 0, keep);
    check(GetTopWindow(0) == owned && GetWindow(owned, 2) == wp && GetWindow(owned, 4) == wp, "HWND_TOPMOST: P's owned window above it, topmost too; GW_OWNER");
    SetWindowPos(wp, (HWND)1, 0, 0, 0, 0, keep);
    check(GetWindow(GetTopWindow(0), 1) == wp && GetWindow(wp, 3) == owned, "P to the bottom takes its owned window along, still above it");
    // WS_EX_NOACTIVATE: a click is the window's, but it's neither activated nor raised.
    HWND n = CreateWindowExW(0x08000000, W(0, "CarbonDesktop"), W(1, "N"), WS_OVERLAPPEDWINDOW, 20, 600, 150, 100, 0, 0, 0, 0);
    ShowWindow(n, 4);
    SetWindowPos(n, (HWND)1, 0, 0, 0, 0, keep);
    click(60, 660);
    check(GetActiveWindow() == active && GetWindow(GetTopWindow(0), 1) == n, "WS_EX_NOACTIVATE: clicked, not activated, not raised");
    DestroyWindow(n);
    DestroyWindow(owned);
    DestroyWindow(wq);
    DestroyWindow(wp);
    pump();

    check(SetForegroundWindow(a) && GetActiveWindow() == a && IsWindow(a), "A stays open, for the screenshot");
    ExitProcess(failures);
}
