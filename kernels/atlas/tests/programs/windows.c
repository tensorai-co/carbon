// Windows, messages and painting on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: RegisterClassExW; CreateWindowExW with WM_NCCREATE and WM_CREATE (and its parameter) and WM_SIZE (its
// client area, inside the frame AdjustWindowRectEx says); window text through DefWindowProc's WM_GETTEXT and
// WM_SETTEXT; FindWindowW; SendMessageW on the window's thread; window and class longs; subclassing with
// CallWindowProcW; a child window, EnumChildWindows, EnumWindows; ShowWindow, UpdateWindow, WM_ERASEBKGND and WM_PAINT;
// FillRect, the class background, a pen's Rectangle and LineTo read back with GetPixel; MoveWindow and WM_SIZE;
// SendMessageW and PostMessageW from another thread into a GetMessageW loop; timers; PeekMessageW; WM_CLOSE destroying
// a window and its child; PostQuitMessage

typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef void *HWND;
typedef void *HDC;
typedef unsigned int DWORD;
typedef unsigned int UINT;
typedef ULONG_PTR u64; // pointer-sized everywhere here
typedef LONG_PTR LRESULT;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef LRESULT (__stdcall *WNDPROC)(HWND, UINT, u64, LONG_PTR);
typedef struct { int left, top, right, bottom; } RECT;
typedef struct { HWND hwnd; UINT message; u64 wParam; LONG_PTR lParam; DWORD time; int x, y; DWORD priv; } MSG;
typedef struct { HDC hdc; BOOL erase; RECT paint; BOOL restore, update; unsigned char reserved[32]; } PAINTSTRUCT;
typedef struct { UINT size, style; WNDPROC proc; int cls, wnd; HANDLE instance, icon, cursor, background; const WCHAR *menu, *name; HANDLE small; } WNDCLASSEXW;
typedef struct { void *params; HANDLE instance, menu; HWND parent; int cy, cx, y, x; long style; const WCHAR *name, *cls; DWORD ex; } CREATESTRUCTW;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall CreateThread(void *, u64, DWORD (__stdcall *)(void *), void *, DWORD, DWORD *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K void __stdcall Sleep(DWORD);
K unsigned short __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K BOOL __stdcall DestroyWindow(HWND);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, u64, LONG_PTR);
K LRESULT __stdcall CallWindowProcW(WNDPROC, HWND, UINT, u64, LONG_PTR);
K BOOL __stdcall ShowWindow(HWND, int);
K BOOL __stdcall UpdateWindow(HWND);
K BOOL __stdcall GetMessageW(MSG *, HWND, UINT, UINT);
K BOOL __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K BOOL __stdcall TranslateMessage(const MSG *);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K LRESULT __stdcall SendMessageW(HWND, UINT, u64, LONG_PTR);
K BOOL __stdcall PostMessageW(HWND, UINT, u64, LONG_PTR);
K void __stdcall PostQuitMessage(int);
K BOOL __stdcall SetWindowTextW(HWND, const WCHAR *);
K int __stdcall GetWindowTextW(HWND, WCHAR *, int);
K LONG_PTR __stdcall GetWindowLongPtrW(HWND, int);
K LONG_PTR __stdcall SetWindowLongPtrW(HWND, int, LONG_PTR);
K u64 __stdcall GetClassLongPtrW(HWND, int);
K BOOL __stdcall GetClientRect(HWND, RECT *);
K BOOL __stdcall AdjustWindowRectEx(RECT *, DWORD, BOOL, DWORD);
K BOOL __stdcall MoveWindow(HWND, int, int, int, int, BOOL);
K BOOL __stdcall IsWindow(HWND);
K BOOL __stdcall IsWindowVisible(HWND);
K HWND __stdcall GetParent(HWND);
K HWND __stdcall FindWindowW(const WCHAR *, const WCHAR *);
K BOOL __stdcall EnumWindows(BOOL (__stdcall *)(HWND, LONG_PTR), LONG_PTR);
K BOOL __stdcall EnumChildWindows(HWND, BOOL (__stdcall *)(HWND, LONG_PTR), LONG_PTR);
K u64 __stdcall SetTimer(HWND, u64, UINT, void *);
K BOOL __stdcall KillTimer(HWND, u64);
K BOOL __stdcall InvalidateRect(HWND, const RECT *, BOOL);
K HDC __stdcall BeginPaint(HWND, PAINTSTRUCT *);
K BOOL __stdcall EndPaint(HWND, const PAINTSTRUCT *);
K HDC __stdcall GetDC(HWND);
K int __stdcall ReleaseDC(HWND, HDC);
K int __stdcall FillRect(HDC, const RECT *, HANDLE);
K int __stdcall DrawTextW(HDC, const WCHAR *, int, RECT *, UINT);
K HANDLE __stdcall GetSysColorBrush(int);
K HANDLE __stdcall GetStockObject(int);
K HANDLE __stdcall CreateSolidBrush(DWORD);
K HANDLE __stdcall CreatePen(int, int, DWORD);
K HANDLE __stdcall SelectObject(HDC, HANDLE);
K BOOL __stdcall DeleteObject(HANDLE);
K DWORD __stdcall SetTextColor(HDC, DWORD);
K int __stdcall SetBkMode(HDC, int);
K BOOL __stdcall TextOutW(HDC, int, int, const WCHAR *, int);
K BOOL __stdcall Rectangle(HDC, int, int, int, int);
K BOOL __stdcall MoveToEx(HDC, int, int, void *);
K BOOL __stdcall LineTo(HDC, int, int);
K DWORD __stdcall GetPixel(HDC, int, int);
K BOOL __stdcall GetTextExtentPoint32W(HDC, const WCHAR *, int, int *);

#define WM_CREATE 1
#define WM_DESTROY 2
#define WM_SIZE 5
#define WM_PAINT 0xf
#define WM_CLOSE 0x10
#define WM_ERASEBKGND 0x14
#define WM_NCCREATE 0x81
#define WM_NCDESTROY 0x82
#define WM_TIMER 0x113
#define WM_USER 0x400
#define WS_OVERLAPPEDWINDOW 0xcf0000
#define WS_CHILD 0x40000000
#define WS_VISIBLE 0x10000000
#define RGB(r, g, b) ((DWORD)((r) | ((g) << 8) | ((b) << 16)))

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
static WCHAR wbuf[8][128];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }

// What the window procedure saw, in order.
static UINT seen[64];
static int nseen;
static void *create_param;
static int sized_w, sized_h, paints, erases, timers, destroyed, nc_destroyed, user_sent, closed;
static DWORD sender_result;

static LRESULT __stdcall wndproc(HWND hwnd, UINT msg, u64 wp, LONG_PTR lp) {
    if (nseen < 64) seen[nseen++] = msg;
    switch (msg) {
    case WM_CREATE: create_param = ((CREATESTRUCTW *)lp)->params; return 0;
    case WM_SIZE: sized_w = (int)(lp & 0xffff); sized_h = (int)((lp >> 16) & 0xffff); return 0;
    case WM_ERASEBKGND: erases++; break;
    case WM_PAINT: {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(hwnd, &ps);
        paints++;
        RECT band = {10, 10, 110, 40};
        HANDLE red = CreateSolidBrush(RGB(255, 0, 0));
        FillRect(dc, &band, red);
        DeleteObject(red);
        SetBkMode(dc, 1);
        SetTextColor(dc, RGB(0, 0, 128));
        TextOutW(dc, 12, 50, W(4, "Hello from atlas"), 16);
        HANDLE pen = CreatePen(0, 1, RGB(0, 128, 0));
        HANDLE old_pen = SelectObject(dc, pen);
        HANDLE old_brush = SelectObject(dc, GetStockObject(5)); // NULL_BRUSH
        Rectangle(dc, 120, 10, 200, 60);
        MoveToEx(dc, 120, 70, 0);
        LineTo(dc, 200, 70);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        RECT box = {0, 80, 300, 100};
        DrawTextW(dc, W(5, "centered"), -1, &box, 0x1 | 0x20); // DT_CENTER | DT_SINGLELINE
        EndPaint(hwnd, &ps);
        return 0;
    }
    case WM_TIMER: timers++; if (timers == 3) KillTimer(hwnd, wp); return 0;
    case WM_USER: return (LRESULT)(wp + lp);
    case WM_USER + 1: user_sent++; return 77;
    case WM_CLOSE: closed++; break;
    case WM_DESTROY: destroyed++; return 0;
    case WM_NCDESTROY: nc_destroyed++; break;
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
}

static WNDPROC original;
static int subclassed;
static LRESULT __stdcall subclass(HWND hwnd, UINT msg, u64 wp, LONG_PTR lp) {
    if (msg == WM_USER) { subclassed++; return CallWindowProcW(original, hwnd, msg, wp * 10, lp); }
    return CallWindowProcW(original, hwnd, msg, wp, lp);
}

static HWND main_window;
static DWORD __stdcall sender(void *arg) {
    (void)arg;
    sender_result = (DWORD)SendMessageW(main_window, WM_USER + 1, 0, 0);
    PostMessageW(main_window, WM_USER + 2, 5, 6);
    return 0;
}

static int counted;
static BOOL __stdcall count(HWND hwnd, LONG_PTR lp) { (void)hwnd; counted += (int)lp; return 1; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("windows on atlas, arm64\n");
#elif defined(__i386__)
    say("windows on atlas, x86 (WoW64)\n");
#else
    say("windows on atlas, x86-64\n");
#endif
    WNDCLASSEXW wc = {sizeof wc, 0, wndproc, 0, 16, 0, 0, 0, GetSysColorBrush(5), 0, W(0, "CarbonWindow"), 0};
    unsigned short atom = RegisterClassExW(&wc);
    check(atom != 0 && RegisterClassExW(&wc) == 0, "RegisterClassExW, and not twice");

    int param = 42;
    main_window = CreateWindowExW(0, W(0, "CarbonWindow"), W(1, "atlas window"), WS_OVERLAPPEDWINDOW, 100, 100, 320, 240, 0, 0, 0, &param);
    check(main_window != 0 && seen[0] == WM_NCCREATE && seen[1] == WM_CREATE && create_param == &param, "CreateWindowExW: WM_NCCREATE, then WM_CREATE with its parameter");
    // The frame, as AdjustWindowRectEx has it: the window's size less that is its client area's.
    RECT outer = {0, 0, 0, 0};
    AdjustWindowRectEx(&outer, WS_OVERLAPPEDWINDOW, 0, 0);
    int fw = outer.right - outer.left, fh = outer.bottom - outer.top;
    check(fw > 0 && fh > fw && sized_w == 320 - fw && sized_h == 240 - fh, "WM_SIZE with its client area's size, inside its frame");
    WCHAR title[64];
    check(GetWindowTextW(main_window, title, 64) == 12 && weq(title, "atlas window"), "GetWindowTextW, through DefWindowProc's WM_GETTEXT");
    SetWindowTextW(main_window, W(1, "Carbon v2"));
    GetWindowTextW(main_window, title, 64);
    check(weq(title, "Carbon v2") && FindWindowW(W(0, "CarbonWindow"), W(1, "Carbon v2")) == main_window, "SetWindowTextW, and FindWindowW by class and title");
    check(!IsWindowVisible(main_window), "not visible until it's shown");

    check(SendMessageW(main_window, WM_USER, 2, 3) == 5, "SendMessageW on its own thread: the WndProc's result");
    SetWindowLongPtrW(main_window, -21, 1234);
    SetWindowLongPtrW(main_window, 8, 5678);
    check(GetWindowLongPtrW(main_window, -21) == 1234 && GetWindowLongPtrW(main_window, 8) == 5678 && GetClassLongPtrW(main_window, -32) == atom, "window and class longs: user data, extra bytes, the atom");
    original = (WNDPROC)SetWindowLongPtrW(main_window, -4, (LONG_PTR)subclass);
    check(original == wndproc && SendMessageW(main_window, WM_USER, 2, 3) == 23 && subclassed == 1, "subclassing with GWLP_WNDPROC and CallWindowProcW");
    SetWindowLongPtrW(main_window, -4, (LONG_PTR)original);

    HWND child = CreateWindowExW(0, W(0, "CarbonWindow"), W(2, "child"), WS_CHILD | WS_VISIBLE, 10, 150, 100, 50, main_window, (HANDLE)7, 0, 0);
    counted = 0;
    EnumChildWindows(main_window, count, 1);
    int children = counted;
    counted = 0;
    EnumWindows(count, 1);
    check(child && GetParent(child) == main_window && GetWindowLongPtrW(child, -12) == 7 && children == 1 && counted == 1, "a child window, its ID, EnumChildWindows and EnumWindows");

    ShowWindow(main_window, 1);
    UpdateWindow(main_window);
    check(IsWindowVisible(main_window) && paints == 1 && erases >= 1, "ShowWindow and UpdateWindow: erased, then WM_PAINT");

    HDC dc = GetDC(main_window);
    DWORD band = GetPixel(dc, 20, 20), background = GetPixel(dc, 5, 5), line = GetPixel(dc, 150, 70), frame = GetPixel(dc, 120, 30), inside = GetPixel(dc, 150, 30);
    int size[2];
    GetTextExtentPoint32W(dc, W(3, "abcd"), 4, size);
    ReleaseDC(main_window, dc);
    check(band == RGB(255, 0, 0) && background == RGB(255, 255, 255), "FillRect and the class's background, read back with GetPixel");
    check(line == RGB(0, 128, 0) && frame == RGB(0, 128, 0) && inside == RGB(255, 255, 255), "a pen's Rectangle with NULL_BRUSH, and LineTo");
    check(size[0] > 0 && size[1] > 0, "GetTextExtentPoint32W");

    MoveWindow(main_window, 50, 50, 400, 300, 1);
    RECT client;
    GetClientRect(main_window, &client);
    check(client.right == 400 - fw && client.bottom == 300 - fh && sized_w == 400 - fw, "MoveWindow: a new size, and WM_SIZE");

    SetTimer(main_window, 1, 20, 0);
    HANDLE t = CreateThread(0, 0, sender, 0, 0, 0);
    MSG msg;
    int got_posted = 0;
    while (!got_posted || timers < 3 || paints < 2) {
        if (!GetMessageW(&msg, 0, 0, 0)) break;
        if (msg.message == WM_USER + 2 && msg.wParam == 5 && msg.lParam == 6) got_posted = 1;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    WaitForSingleObject(t, 0xffffffff);
    check(sender_result == 77 && user_sent == 1, "SendMessageW from another thread, taken while this one waits in GetMessageW");
    check(got_posted, "PostMessageW from another thread, through GetMessageW");
    check(timers == 3, "SetTimer: WM_TIMER through the message loop, until KillTimer");
    check(paints >= 2, "WM_PAINT again after the resize");

    PostMessageW(main_window, WM_USER + 3, 0, 0);
    BOOL peeked = PeekMessageW(&msg, 0, WM_USER + 3, WM_USER + 3, 0);
    BOOL removed = PeekMessageW(&msg, 0, WM_USER + 3, WM_USER + 3, 1);
    BOOL empty = !PeekMessageW(&msg, 0, WM_USER + 3, WM_USER + 3, 1);
    check(peeked && removed && empty, "PeekMessageW, leaving and then taking a message");

    SendMessageW(main_window, WM_CLOSE, 0, 0);
    check(closed == 1 && destroyed == 2 && nc_destroyed == 2 && !IsWindow(main_window) && !IsWindow(child), "WM_CLOSE: DefWindowProc destroys it and its child");

    PostQuitMessage(9);
    BOOL got = GetMessageW(&msg, 0, 0, 0);
    check(got == 0 && msg.message == 0x12 && msg.wParam == 9, "PostQuitMessage: GetMessageW returns 0, with WM_QUIT's code");

    ExitProcess(failures);
}
