// ANSI windows, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: a class registered with RegisterClassExA and a window made with CreateWindowExA, whose WndProc takes
// ANSI: its CREATESTRUCTA's names, WM_SETTEXT's text and typed characters in 1252, however they were sent, and
// DefWindowProcA keeping its text; IsWindowUnicode; a Unicode window driven by the *A functions (SetWindowTextA,
// GetWindowTextA, SendMessageA); GetMessageA and DispatchMessageA with a character; PostMessageA's made UTF-16; a
// WndProc set by SetWindowLongPtrA and SetWindowLongPtrW; FindWindowA; and UnregisterClassA.

typedef void *HANDLE;
typedef void *HWND;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef unsigned short WCHAR;
typedef long long LRESULT;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
typedef LRESULT (__stdcall *WNDPROC)(HWND, unsigned, u64, long long);
typedef struct { unsigned size, style; WNDPROC proc; int cls_extra, wnd_extra; HANDLE instance, icon, cursor, background; const void *menu, *name; HANDLE small; } WNDCLASSEX;
typedef struct { HWND hwnd; unsigned message; u64 wparam; long long lparam; DWORD time; long x, y; DWORD private_; } MSG;
typedef struct { void *params; HANDLE instance, menu; HWND parent; int cy, cx, y, x; long style; const void *name, *cls; DWORD ex; } CREATESTRUCT;
K unsigned short __stdcall RegisterClassExA(const WNDCLASSEX *);
K unsigned short __stdcall RegisterClassExW(const WNDCLASSEX *);
K HWND __stdcall CreateWindowExA(DWORD, const char *, const char *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcA(HWND, unsigned, u64, long long);
K LRESULT __stdcall DefWindowProcW(HWND, unsigned, u64, long long);
K int __stdcall GetMessageA(MSG *, HWND, unsigned, unsigned);
K int __stdcall GetMessageW(MSG *, HWND, unsigned, unsigned);
K LRESULT __stdcall DispatchMessageA(const MSG *);
K LRESULT __stdcall SendMessageA(HWND, unsigned, u64, long long);
K LRESULT __stdcall SendMessageW(HWND, unsigned, u64, long long);
K int __stdcall PostMessageA(HWND, unsigned, u64, long long);
K int __stdcall PostMessageW(HWND, unsigned, u64, long long);
K int __stdcall SetWindowTextA(HWND, const char *);
K int __stdcall SetWindowTextW(HWND, const WCHAR *);
K int __stdcall GetWindowTextA(HWND, char *, int);
K int __stdcall GetWindowTextW(HWND, WCHAR *, int);
K int __stdcall GetWindowTextLengthA(HWND);
K int __stdcall IsWindowUnicode(HWND);
K long long __stdcall SetWindowLongPtrA(HWND, int, long long);
K long long __stdcall SetWindowLongPtrW(HWND, int, long long);
K HWND __stdcall FindWindowA(const char *, const char *);
K int __stdcall DestroyWindow(HWND);
K int __stdcall UnregisterClassA(const char *, HANDLE);

#define WM_CREATE 1
#define WM_SETTEXT 0xc
#define WM_GETTEXT 0xd
#define WM_CHAR 0x102
#define WM_NCCREATE 0x81

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
static int same(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }
static int wsame(const WCHAR *a, const WCHAR *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }
static void copy(char *to, const char *from, int room) { int i = 0; for (; from[i] && i < room - 1; i++) to[i] = from[i]; to[i] = 0; }
static void wcopy(WCHAR *to, const WCHAR *from, int room) { int i = 0; for (; from[i] && i < room - 1; i++) to[i] = from[i]; to[i] = 0; }

// What the ANSI WndProc saw.
static char created_name[32], created_class[32], set_text[32];
static int nccreate_seen;
static unsigned chars[8];
static int char_count;
static LRESULT __stdcall ansi_proc(HWND h, unsigned m, u64 w, long long l) {
    if (m == WM_NCCREATE) {
        const CREATESTRUCT *cs = (const CREATESTRUCT *)l;
        copy(created_name, cs->name, 32);
        copy(created_class, cs->cls, 32);
        nccreate_seen = 1;
    }
    if (m == WM_SETTEXT) copy(set_text, (const char *)l, 32);
    if (m == WM_CHAR && char_count < 8) chars[char_count++] = (unsigned)w;
    return DefWindowProcA(h, m, w, l);
}

// What the Unicode WndProc saw.
static WCHAR wide_text[32];
static unsigned wide_char;
static LRESULT __stdcall wide_proc(HWND h, unsigned m, u64 w, long long l) {
    if (m == WM_SETTEXT) wcopy(wide_text, (const WCHAR *)l, 32);
    if (m == WM_CHAR) wide_char = (unsigned)w;
    return DefWindowProcW(h, m, w, l);
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("ANSI windows, arm64\n");
#else
    say("ANSI windows, x86-64\n");
#endif
    WNDCLASSEX ca = {sizeof(WNDCLASSEX), 0, ansi_proc, 0, 0, 0, 0, 0, 0, 0, "Caf\xe9" "Class", 0};
    check(RegisterClassExA(&ca) != 0, "RegisterClassExA, with é in its name");
    HWND a = CreateWindowExA(0, "Caf\xe9" "Class", "Fen\xeatre", 0x00cf0000, 10, 10, 200, 100, 0, 0, 0, 0);
    check(a && nccreate_seen && same(created_name, "Fen\xeatre") && same(created_class, "Caf\xe9" "Class"), "CreateWindowExA: its WndProc's CREATESTRUCTA has the names in ANSI");
    check(!IsWindowUnicode(a), "IsWindowUnicode: FALSE for it");
    WCHAR w[32];
    static const WCHAR fenetre[] = {'F', 'e', 'n', 0xea, 't', 'r', 'e', 0};
    check(GetWindowTextW(a, w, 32) == 7 && wsame(w, fenetre), "GetWindowTextW: its text, in UTF-16");
    static const WCHAR ete[] = {0xc9, 't', 0xe9, 0};
    SetWindowTextW(a, ete);
    check(same(set_text, "\xc9t\xe9"), "SetWindowTextW to it: its WndProc's WM_SETTEXT is ANSI");
    char t[32];
    check(GetWindowTextA(a, t, 32) == 3 && same(t, "\xc9t\xe9") && GetWindowTextLengthA(a) == 3, "GetWindowTextA and GetWindowTextLengthA");
    check(GetWindowTextW(a, w, 32) == 3 && wsame(w, ete), "  and DefWindowProcA kept it, as GetWindowTextW sees");
    SendMessageW(a, WM_CHAR, 0xe9, 0);
    SendMessageW(a, WM_CHAR, 0x2713, 0);
    check(char_count == 2 && chars[0] == 0xe9 && chars[1] == '?', "a character sent in UTF-16 comes as 1252, ? for what it hasn't");
    PostMessageW(a, WM_CHAR, 0x20ac, 0);
    MSG msg;
    check(GetMessageA(&msg, a, WM_CHAR, WM_CHAR) && msg.wparam == 0x80, "GetMessageA gives a character in ANSI");
    DispatchMessageA(&msg);
    check(char_count == 3 && chars[2] == 0x80, "  and DispatchMessageA takes it so");
    PostMessageA(a, WM_CHAR, 0x80, 0);
    check(GetMessageW(&msg, a, WM_CHAR, WM_CHAR) && msg.wparam == 0x20ac, "PostMessageA's character is UTF-16 to GetMessageW");
    check(FindWindowA("Caf\xe9" "Class", "\xc9t\xe9") == a, "FindWindowA");

    static const WCHAR wide_class[] = {'W', 'i', 'd', 'e', 0};
    WNDCLASSEX cw = {sizeof(WNDCLASSEX), 0, wide_proc, 0, 0, 0, 0, 0, 0, 0, wide_class, 0};
    RegisterClassExW(&cw);
    HWND u = CreateWindowExW(0, wide_class, wide_class, 0x00cf0000, 10, 10, 200, 100, 0, 0, 0, 0);
    check(u && IsWindowUnicode(u), "a Unicode window: IsWindowUnicode TRUE");
    static const WCHAR cafe[] = {'C', 'a', 'f', 0xe9, 0};
    SetWindowTextA(u, "Caf\xe9");
    check(wsame(wide_text, cafe), "SetWindowTextA to it: its WndProc's WM_SETTEXT is UTF-16");
    check(GetWindowTextA(u, t, 32) == 4 && same(t, "Caf\xe9"), "GetWindowTextA from it");
    check(SendMessageA(u, WM_GETTEXT, 3, (long long)t) == 2 && same(t, "Ca"), "SendMessageA's WM_GETTEXT, cut to fit");
    SendMessageA(u, WM_CHAR, 0xe9, 0);
    check(wide_char == 0xe9, "SendMessageA's character, in UTF-16 to it");
    SetWindowLongPtrA(u, -4, (long long)ansi_proc);
    check(!IsWindowUnicode(u), "SetWindowLongPtrA's WndProc takes ANSI");
    SetWindowLongPtrW(u, -4, (long long)wide_proc);
    check(IsWindowUnicode(u), "  and SetWindowLongPtrW's UTF-16");

    DestroyWindow(u);
    DestroyWindow(a);
    check(UnregisterClassA("Caf\xe9" "Class", 0), "UnregisterClassA");
    ExitProcess(failures);
}
