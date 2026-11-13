// Menus, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it, with menus.rc.
//
// It checks: a popup menu built with AppendMenuW (an accelerator, a separator, a greyed item, a submenu) and read back
// (GetMenuItemCount, GetMenuItemID, GetSubMenu, GetMenuState, GetMenuStringW, by position and by command, a command
// found in a submenu); CheckMenuItem, CheckMenuRadioItem, EnableMenuItem, the default item; InsertMenuItemW,
// GetMenuItemInfoW, SetMenuItemInfoW and RemoveMenu; a MENU and a MENUEX resource; SetMenu and GetMenu; and
// TrackPopupMenu on the desktop, driven by another thread's SendInput: the arrows past a separator and Enter, Escape,
// an item's & letter, a submenu opened by its letter, a click on an item, and WM_COMMAND when it isn't
// TPM_RETURNCMD; WM_INITMENUPOPUP before it's shown.

typedef void *HANDLE;
typedef void *HWND;
typedef void *HMENU;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned int UINT;
typedef __UINTPTR_TYPE__ WPARAM;
typedef __INTPTR_TYPE__ LPARAM;
typedef __INTPTR_TYPE__ LRESULT;
typedef struct { HWND hwnd; UINT message; WPARAM wParam; LPARAM lParam; DWORD time; int x, y; } MSG;
typedef LRESULT (__stdcall *WNDPROC)(HWND, UINT, WPARAM, LPARAM);
typedef struct {
    UINT cbSize, style;
    WNDPROC lpfnWndProc;
    int cbClsExtra, cbWndExtra;
    HANDLE hInstance, hIcon, hCursor, hbrBackground;
    const WCHAR *lpszMenuName, *lpszClassName;
    HANDLE hIconSm;
} WNDCLASSEXW;
typedef struct {
    UINT cbSize, fMask, fType, fState, wID;
    HMENU hSubMenu;
    HANDLE hbmpChecked, hbmpUnchecked;
    __UINTPTR_TYPE__ dwItemData;
    WCHAR *dwTypeData;
    UINT cch;
    HANDLE hbmpItem;
} MENUITEMINFOW;
typedef struct {
    DWORD type;
    union {
        struct { int dx, dy; DWORD data, flags, time; __UINTPTR_TYPE__ extra; } mi;
        struct { unsigned short vk, scan; DWORD flags, time; __UINTPTR_TYPE__ extra; } ki;
    };
} INPUT;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K HANDLE __stdcall CreateThread(void *, __SIZE_TYPE__, DWORD (__stdcall *)(void *), void *, DWORD, DWORD *);
K void __stdcall Sleep(DWORD);
K WORD __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, WPARAM, LPARAM);
K int __stdcall DestroyWindow(HWND);
K int __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K UINT __stdcall SendInput(UINT, INPUT *, int);
K int __stdcall SetCursorPos(int, int);
K HMENU __stdcall CreatePopupMenu(void);
K int __stdcall DestroyMenu(HMENU);
K int __stdcall AppendMenuW(HMENU, UINT, __UINTPTR_TYPE__, const WCHAR *);
K int __stdcall GetMenuItemCount(HMENU);
K UINT __stdcall GetMenuItemID(HMENU, int);
K HMENU __stdcall GetSubMenu(HMENU, int);
K UINT __stdcall GetMenuState(HMENU, UINT, UINT);
K int __stdcall GetMenuStringW(HMENU, UINT, WCHAR *, int, UINT);
K DWORD __stdcall CheckMenuItem(HMENU, UINT, UINT);
K int __stdcall CheckMenuRadioItem(HMENU, UINT, UINT, UINT, UINT);
K int __stdcall EnableMenuItem(HMENU, UINT, UINT);
K int __stdcall SetMenuDefaultItem(HMENU, UINT, UINT);
K UINT __stdcall GetMenuDefaultItem(HMENU, UINT, UINT);
K int __stdcall InsertMenuItemW(HMENU, UINT, int, const MENUITEMINFOW *);
K int __stdcall GetMenuItemInfoW(HMENU, UINT, int, MENUITEMINFOW *);
K int __stdcall SetMenuItemInfoW(HMENU, UINT, int, const MENUITEMINFOW *);
K int __stdcall RemoveMenu(HMENU, UINT, UINT);
K HMENU __stdcall LoadMenuW(HANDLE, const WCHAR *);
K int __stdcall SetMenu(HWND, HMENU);
K HMENU __stdcall GetMenu(HWND);
K int __stdcall TrackPopupMenu(HMENU, UINT, int, int, int, HWND, const void *);

#define MF_STRING 0
#define MF_GRAYED 1
#define MF_CHECKED 8
#define MF_POPUP 0x10
#define MF_BYPOSITION 0x400
#define MF_SEPARATOR 0x800
#define MIIM_ID 2
#define MIIM_STRING 0x40
#define TPM_RETURNCMD 0x100
#define WM_COMMAND 0x111
#define WM_INITMENUPOPUP 0x117
#define WS_POPUP 0x80000000u
#define WS_VISIBLE 0x10000000u

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
static WCHAR wbuf[8][64];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }

static int inits;
static LRESULT __stdcall wndproc(HWND h, UINT m, WPARAM w, LPARAM l) {
    if (m == WM_INITMENUPOPUP) inits++;
    return DefWindowProcW(h, m, w, l);
}

// ---- Input from another thread, while the menu loop runs ----

static INPUT inputs[16];
static int ninputs;
static void key(unsigned short vk) {
    for (int up = 0; up < 2; up++) {
        INPUT *in = &inputs[ninputs++];
        *in = (INPUT){0};
        in->type = 1;
        in->ki.vk = vk;
        in->ki.flags = up ? 2 : 0;
    }
}
static int script;
static int click_x, click_y;
static DWORD __stdcall driver(void *unused) {
    (void)unused;
    Sleep(300);
    ninputs = 0;
    switch (script) {
    case 0: key(0x28); key(0x28); key(0x0d); break; // down, down (past the separator), Enter
    case 1: key(0x1b); break;                        // Escape
    case 2: key('X'); break;                         // E&xit
    case 3: key('M'); key(0x28); key(0x0d); break;   // &More, then down to Beta, Enter
    case 4:                                          // a click on the first item
        SetCursorPos(click_x, click_y);
        Sleep(50);
        for (int up = 0; up < 2; up++) {
            INPUT *in = &inputs[ninputs++];
            *in = (INPUT){0};
            in->mi.flags = up ? 4 : 2;
        }
        break;
    case 5: key(0x28); key(0x0d); break;             // down, Enter: WM_COMMAND
    }
    SendInput((UINT)ninputs, inputs, sizeof(INPUT));
    return 0;
}
static int track(HMENU menu, UINT flags, HWND wnd, int which) {
    script = which;
    CreateThread(0, 0, driver, 0, 0, 0);
    return TrackPopupMenu(menu, flags, 300, 200, 0, wnd, 0);
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("menus on atlas, arm64\n");
#elif defined(__i386__)
    say("menus on atlas, x86 (WoW64)\n");
#else
    say("menus on atlas, x86-64\n");
#endif
    HANDLE instance = GetModuleHandleW(0);
    WCHAR text[64];

    // ---- Built, and read back ----
    HMENU menu = CreatePopupMenu(), sub = CreatePopupMenu();
    AppendMenuW(menu, MF_STRING, 10, W(0, "&Open\tCtrl+O"));
    AppendMenuW(menu, MF_SEPARATOR, 0, 0);
    AppendMenuW(menu, MF_STRING | MF_GRAYED, 11, W(0, "&Save"));
    AppendMenuW(sub, MF_STRING, 30, W(0, "&Alpha"));
    AppendMenuW(sub, MF_STRING, 31, W(0, "&Beta"));
    AppendMenuW(menu, MF_POPUP, (__UINTPTR_TYPE__)sub, W(0, "&More"));
    AppendMenuW(menu, MF_STRING, 12, W(0, "E&xit"));
    check(menu && GetMenuItemCount(menu) == 5 && GetMenuItemCount(sub) == 2, "CreatePopupMenu and AppendMenuW");
    check(GetMenuItemID(menu, 0) == 10 && GetMenuItemID(menu, 3) == 0xffffffffu && GetSubMenu(menu, 3) == sub, "GetMenuItemID, and GetSubMenu for a submenu");
    check((GetMenuState(menu, 11, 0) & MF_GRAYED) && (GetMenuState(menu, 1, MF_BYPOSITION) & MF_SEPARATOR) && GetMenuState(menu, 30, 0) != 0xffffffffu,
          "GetMenuState, by command (into a submenu too) and by position");
    check(GetMenuStringW(menu, 0, text, 64, MF_BYPOSITION) == 12 && weq(text, "&Open\tCtrl+O"), "GetMenuStringW");
    check(CheckMenuItem(menu, 10, MF_CHECKED) == 0 && (GetMenuState(menu, 10, 0) & MF_CHECKED), "CheckMenuItem");
    check(CheckMenuRadioItem(sub, 30, 31, 31, 0) && (GetMenuState(sub, 31, 0) & MF_CHECKED) && !(GetMenuState(sub, 30, 0) & MF_CHECKED), "CheckMenuRadioItem");
    check(EnableMenuItem(menu, 11, 0) == MF_GRAYED && !(GetMenuState(menu, 11, 0) & MF_GRAYED), "EnableMenuItem: the state before");
    check(SetMenuDefaultItem(menu, 12, 0) && GetMenuDefaultItem(menu, 0, 0) == 12, "SetMenuDefaultItem");
    MENUITEMINFOW mii = {0};
    mii.cbSize = sizeof mii;
    mii.fMask = MIIM_ID | MIIM_STRING;
    mii.wID = 5;
    mii.dwTypeData = (WCHAR *)W(1, "First");
    check(InsertMenuItemW(menu, 0, 1, &mii) && GetMenuItemCount(menu) == 6 && GetMenuItemID(menu, 0) == 5, "InsertMenuItemW, by position");
    MENUITEMINFOW got = {0};
    got.cbSize = sizeof got;
    got.fMask = MIIM_STRING;
    got.dwTypeData = text;
    got.cch = 64;
    check(GetMenuItemInfoW(menu, 5, 0, &got) && got.cch == 5 && weq(text, "First"), "GetMenuItemInfoW: its text");
    mii.fMask = MIIM_STRING;
    mii.dwTypeData = (WCHAR *)W(1, "Renamed");
    SetMenuItemInfoW(menu, 5, 0, &mii);
    GetMenuStringW(menu, 5, text, 64, 0);
    check(weq(text, "Renamed") && RemoveMenu(menu, 5, 0) && GetMenuItemCount(menu) == 5, "SetMenuItemInfoW and RemoveMenu");

    // ---- From resources ----
    HMENU loaded = LoadMenuW(instance, (const WCHAR *)1);
    HMENU file = loaded ? GetSubMenu(loaded, 0) : 0;
    check(loaded && GetMenuItemCount(loaded) == 2 && GetMenuItemID(loaded, 1) == 102 && GetMenuItemCount(file) == 3 && GetMenuItemID(file, 2) == 101,
          "LoadMenuW: a MENU, its popup and items");
    HMENU ex = LoadMenuW(instance, (const WCHAR *)2);
    HMENU edit = ex ? GetSubMenu(ex, 0) : 0;
    check(ex && GetMenuItemCount(edit) == 3 && (GetMenuState(edit, 1, MF_BYPOSITION) & MF_SEPARATOR) && (GetMenuState(edit, 202, 0) & MF_GRAYED),
          "a MENUEX: its separator and greyed item");
    check(LoadMenuW(instance, (const WCHAR *)9) == 0, "a menu that isn't there");

    // ---- A window's menu, and popups on the desktop ----
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    wc.lpfnWndProc = wndproc;
    wc.hInstance = instance;
    wc.lpszClassName = W(0, "AtlasMenus");
    RegisterClassExW(&wc);
    HWND wnd = CreateWindowExW(0, W(0, "AtlasMenus"), W(1, "menus"), WS_POPUP | WS_VISIBLE, 200, 100, 400, 300, 0, 0, instance, 0);
    check(SetMenu(wnd, loaded) && GetMenu(wnd) == loaded, "SetMenu and GetMenu");
    MSG msg;
    while (PeekMessageW(&msg, 0, 0, 0, 1)) DispatchMessageW(&msg);

    check(track(menu, TPM_RETURNCMD, wnd, 0) == 11 && inits == 1, "TrackPopupMenu: WM_INITMENUPOPUP, then down past the separator and Enter");
    check(track(menu, TPM_RETURNCMD, wnd, 1) == 0, "Escape: nothing");
    check(track(menu, TPM_RETURNCMD, wnd, 2) == 12, "an item's & letter");
    check(track(menu, TPM_RETURNCMD, wnd, 3) == 31, "a submenu opened by its letter, and an item in it");
    click_x = 300 + 30;
    click_y = 200 + 2 + 10;
    check(track(menu, TPM_RETURNCMD, wnd, 4) == 10, "a click on an item");
    int done = track(menu, 0, wnd, 5);
    int command = 0;
    while (PeekMessageW(&msg, wnd, WM_COMMAND, WM_COMMAND, 1)) command = (int)msg.wParam;
    check(done && command == 10, "without TPM_RETURNCMD: WM_COMMAND to the window");

    check(DestroyMenu(menu) && GetMenuItemCount(sub) == -1, "DestroyMenu, its submenu with it");
    DestroyWindow(wnd);
    ExitProcess(failures);
}
