// What a browser engine's processes call as they set up their sandbox and start, for ARM64, x86-64 and x86: no C
// runtime. tests/programs/build.sh builds it.
//
// It checks: window stations and desktops made, named, opened, made current and closed, and their security read and
// set; DACLs built with SetEntriesInAcl (granting, denying and revoking, in canonical order) from trustees named and by
// SID; message-only windows, which get messages and aren't found as top-level windows; the registry by NT path
// (NtCreateKey, NtOpenKeyEx, NtSetValueKey, NtQueryValueKey in its three forms and when the buffer is short,
// NtDeleteKey) and the user's key path; GetThreadContext of the calling thread, of a suspended thread running, and of
// one suspended while it waits; the Windows Runtime's start and strings (HSTRINGs made, by reference, duplicated,
// compared and taken apart) and its activation finding no class; event tracing's provider calls; ProcessPrng;
// restricted tokens (deny-only SIDs and the user, privileges taken away, restricting SIDs); the process's handles
// listed (ProcessHandleInformation) and one's type and name (NtQueryObject); and on x86-64, ntdll's
// system-call stubs as Windows 10 lays them out, called as they are and through a copy elsewhere, as a sandbox's
// interceptions call them.

typedef void *HANDLE;
typedef void *HWND;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned short USHORT;
typedef unsigned char BYTE;
typedef unsigned long long ULONGLONG;
typedef __UINTPTR_TYPE__ UPTR;
typedef __INTPTR_TYPE__ IPTR;
typedef struct { DWORD a; unsigned short b, c; unsigned char d[8]; } GUID;
typedef struct { USHORT length, maximum; WCHAR *buffer; } UNICODE_STRING;
typedef struct { DWORD length; HANDLE root; UNICODE_STRING *name; DWORD attributes; void *security, *quality; } OBJECT_ATTRIBUTES;
typedef struct { void *multiple; int operation, form, type; WCHAR *name; } TRUSTEE_W;
typedef struct { DWORD permissions, mode, inheritance; TRUSTEE_W trustee; } EXPLICIT_ACCESS_W;
typedef struct { USHORT id; BYTE version, channel, level, opcode; USHORT task; ULONGLONG keyword; } EVENT_DESCRIPTOR;
typedef struct {
    DWORD size, style;
    IPTR(__stdcall *proc)(HWND, DWORD, UPTR, IPTR);
    int class_extra, window_extra;
    HANDLE instance, icon, cursor, background;
    const WCHAR *menu, *name;
    HANDLE small_icon;
} WNDCLASSEXW;
typedef struct { HWND hwnd; DWORD message; UPTR wparam; IPTR lparam; DWORD time; int x, y; } MSG;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K int __stdcall CloseHandle(HANDLE);
K HANDLE __stdcall GetCurrentThread(void);
K HANDLE __stdcall CreateThread(void *, UPTR, DWORD(__stdcall *)(void *), void *, DWORD, DWORD *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K int __stdcall SetEvent(HANDLE);
K DWORD __stdcall SuspendThread(HANDLE);
K DWORD __stdcall ResumeThread(HANDLE);
K int __stdcall GetThreadContext(HANDLE, void *);
K void __stdcall Sleep(DWORD);
K void *__stdcall LocalFree(void *);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K void *__stdcall GetProcAddress(HANDLE, const char *);
K void *__stdcall VirtualAlloc(void *, UPTR, DWORD, DWORD);
K HANDLE __stdcall GetCurrentProcess(void);
K int __stdcall OpenProcessToken(HANDLE, DWORD, HANDLE *);
K int __stdcall GetTokenInformation(HANDLE, int, void *, DWORD, DWORD *);
K int __stdcall CreateRestrictedToken(HANDLE, DWORD, DWORD, void *, DWORD, void *, DWORD, void *, HANDLE *);
K int __stdcall AllocateAndInitializeSid(void *, BYTE, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, void **);
K void *__stdcall FreeSid(void *);

K HANDLE __stdcall GetProcessWindowStation(void);
K HANDLE __stdcall GetThreadDesktop(DWORD);
K int __stdcall GetUserObjectInformationW(HANDLE, int, void *, DWORD, DWORD *);
K HANDLE __stdcall CreateWindowStationW(const WCHAR *, DWORD, DWORD, void *);
K HANDLE __stdcall OpenWindowStationW(const WCHAR *, int, DWORD);
K int __stdcall CloseWindowStation(HANDLE);
K int __stdcall SetProcessWindowStation(HANDLE);
K HANDLE __stdcall CreateDesktopW(const WCHAR *, const WCHAR *, void *, DWORD, DWORD, void *);
K HANDLE __stdcall OpenDesktopW(const WCHAR *, DWORD, int, DWORD);
K int __stdcall CloseDesktop(HANDLE);
K int __stdcall SetThreadDesktop(HANDLE);
K DWORD __stdcall GetCurrentThreadId(void);
K unsigned short __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K int __stdcall DestroyWindow(HWND);
K IPTR __stdcall DefWindowProcW(HWND, DWORD, UPTR, IPTR);
K HWND __stdcall FindWindowW(const WCHAR *, const WCHAR *);
K HWND __stdcall FindWindowExW(HWND, HWND, const WCHAR *, const WCHAR *);
K HWND __stdcall GetParent(HWND);
K HWND __stdcall GetAncestor(HWND, DWORD);
K int __stdcall ShowWindow(HWND, int);
K int __stdcall IsWindowVisible(HWND);
K int __stdcall PostMessageW(HWND, DWORD, UPTR, IPTR);
K int __stdcall PeekMessageW(MSG *, HWND, DWORD, DWORD, DWORD);
K IPTR __stdcall SendMessageW(HWND, DWORD, UPTR, IPTR);

K DWORD __stdcall GetSecurityInfo(HANDLE, int, DWORD, void **, void **, void **, void **, void **);
K DWORD __stdcall SetSecurityInfo(HANDLE, int, DWORD, void *, void *, void *, void *);
K DWORD __stdcall SetEntriesInAclW(DWORD, EXPLICIT_ACCESS_W *, void *, void **);
K void __stdcall BuildExplicitAccessWithNameW(EXPLICIT_ACCESS_W *, WCHAR *, DWORD, DWORD, DWORD);
K void __stdcall BuildTrusteeWithSidW(TRUSTEE_W *, void *);
K DWORD __stdcall EventRegister(const GUID *, void *, void *, ULONGLONG *);
K DWORD __stdcall EventWrite(ULONGLONG, const EVENT_DESCRIPTOR *, DWORD, void *);
K DWORD __stdcall EventUnregister(ULONGLONG);

K DWORD __stdcall RtlFormatCurrentUserKeyPath(UNICODE_STRING *);
K void __stdcall RtlFreeUnicodeString(UNICODE_STRING *);
K void __stdcall RtlInitUnicodeString(UNICODE_STRING *, const WCHAR *);
K DWORD __stdcall NtCreateKey(HANDLE *, DWORD, OBJECT_ATTRIBUTES *, DWORD, UNICODE_STRING *, DWORD, DWORD *);
K DWORD __stdcall NtOpenKeyEx(HANDLE *, DWORD, OBJECT_ATTRIBUTES *, DWORD);
K DWORD __stdcall NtSetValueKey(HANDLE, UNICODE_STRING *, DWORD, DWORD, void *, DWORD);
K DWORD __stdcall NtQueryValueKey(HANDLE, UNICODE_STRING *, int, void *, DWORD, DWORD *);
K DWORD __stdcall NtDeleteKey(HANDLE);
K DWORD __stdcall NtClose(HANDLE);

K DWORD __stdcall RoInitialize(int);
K void __stdcall RoUninitialize(void);
K DWORD __stdcall RoGetActivationFactory(void *, const GUID *, void **);
K DWORD __stdcall WindowsCreateString(const WCHAR *, DWORD, void **);
K DWORD __stdcall WindowsCreateStringReference(const WCHAR *, DWORD, void *, void **);
K DWORD __stdcall WindowsDeleteString(void *);
K DWORD __stdcall WindowsDuplicateString(void *, void **);
K DWORD __stdcall WindowsGetStringLen(void *);
K const WCHAR *__stdcall WindowsGetStringRawBuffer(void *, DWORD *);
K int __stdcall WindowsIsStringEmpty(void *);
K DWORD __stdcall WindowsStringHasEmbeddedNull(void *, int *);
K DWORD __stdcall WindowsCompareStringOrdinal(void *, void *, int *);
K int __stdcall ProcessPrng(BYTE *, UPTR);

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
static WCHAR wbuf[8][96];
static WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)(BYTE)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)(BYTE)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int wstarts(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)(BYTE)*b) { a++; b++; } return *b == 0; }
static void ustr(UNICODE_STRING *u, int i, const char *s) { RtlInitUnicodeString(u, W(i, s)); }

/// A raw ACL's ACEs: whether one of `type` for the SID whose last sub-authority is `rid` (and has `count` of them)
/// has `mask`, and the position of the first such.
static int ace_at(const BYTE *acl, int type, BYTE count, DWORD rid, DWORD mask) {
    int aces = acl[4] | acl[5] << 8;
    const BYTE *p = acl + 8;
    for (int i = 0; i < aces; i++) {
        int size = p[2] | p[3] << 8;
        DWORD m = *(const DWORD *)(p + 4);
        const BYTE *sid = p + 8;
        DWORD last = *(const DWORD *)(sid + 8 + 4 * (sid[1] - 1));
        if (p[0] == type && sid[1] == count && last == rid && m == mask) return i;
        p += size;
    }
    return -1;
}

static int messages;
static IPTR __stdcall wndproc(HWND h, DWORD m, UPTR w, IPTR l) {
    if (m == 0x8001) { messages++; return 42; }
    return DefWindowProcW(h, m, w, l);
}

static volatile int stop;
static volatile UPTR spins;
static DWORD __stdcall spinner(void *p) { (void)p; while (!stop) spins = spins + 1; return 0; }
static HANDLE wake;
static DWORD __stdcall waiter(void *p) { (void)p; WaitForSingleObject(wake, 0xffffffff); return 7; }

// A CONTEXT for any ISA (x64's is the largest, 1232 bytes), aligned as x64's must be.
static __attribute__((aligned(16))) BYTE context[1280];
static void want_full(void) {
#if defined(__x86_64__)
    *(DWORD *)(context + 0x30) = 0x10000b;  // CONTEXT_AMD64 | CONTROL | INTEGER | FLOATING_POINT
#elif defined(__aarch64__)
    *(DWORD *)context = 0x400007;           // CONTEXT_ARM64 | CONTROL | INTEGER | FLOATING_POINT
#else
    *(DWORD *)context = 0x1000b;            // CONTEXT_i386 | CONTROL | INTEGER | FLOATING_POINT
#endif
}
static UPTR context_pc(void) {
#if defined(__x86_64__)
    return *(UPTR *)(context + 0xf8);
#elif defined(__aarch64__)
    return *(UPTR *)(context + 0x108);
#else
    return *(DWORD *)(context + 0xb8);
#endif
}
static UPTR context_sp(void) {
#if defined(__x86_64__)
    return *(UPTR *)(context + 0x98);
#elif defined(__aarch64__)
    return *(UPTR *)(context + 0x100);
#else
    return *(DWORD *)(context + 0xc4);
#endif
}

static BYTE info[64], name_buf[96], value_buf[64];

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    say("a sandbox's setup\n");

    // ---- Window stations and desktops ----
    HANDLE winsta0 = GetProcessWindowStation(), desk0 = GetThreadDesktop(GetCurrentThreadId());
    DWORD needed = 0;
    int ok = GetUserObjectInformationW(winsta0, 2, name_buf, sizeof name_buf, &needed);
    check(ok && weq((WCHAR *)name_buf, "WinSta0") && needed == 16, "the process's window station: WinSta0");
    HANDLE station = CreateWindowStationW(0, 0, 0x37f, 0);
    ok = station && GetUserObjectInformationW(station, 2, name_buf, sizeof name_buf, &needed);
    check(ok && wstarts((WCHAR *)name_buf, "Service-0x0-") && station != winsta0, "a new window station, with no name: Service-0x0-...$");
    int flags[3] = {0, 0, 7};
    GetUserObjectInformationW(station, 1, flags, sizeof flags, 0);
    check(flags[2] == 0, "a made window station isn't visible (WSF_VISIBLE is WinSta0's)");
    check(SetProcessWindowStation(station), "SetProcessWindowStation: the new one");
    HANDLE desk = CreateDesktopW(W(0, "alternate"), 0, 0, 0, 0x10000000, 0);
    ok = desk && GetUserObjectInformationW(desk, 2, name_buf, sizeof name_buf, 0);
    check(ok && weq((WCHAR *)name_buf, "alternate"), "a desktop made in it, by name");
    check(OpenDesktopW(W(0, "alternate"), 0, 0, 0x10000000) == desk, "OpenDesktop finds it in the current station");
    GetUserObjectInformationW(desk, 3, name_buf, sizeof name_buf, 0);
    check(weq((WCHAR *)name_buf, "Desktop"), "its type: Desktop");
    check(SetProcessWindowStation(winsta0), "SetProcessWindowStation: back to WinSta0");
    SetLastError(0);
    check(!OpenDesktopW(W(0, "alternate"), 0, 0, 0x10000000) && GetLastError() != 0, "not found in WinSta0, which has Default");
    check(OpenDesktopW(W(0, "Default"), 0, 0, 0x10000000) == desk0, "Default is the thread's desktop");
    GetUserObjectInformationW(station, 2, name_buf, sizeof name_buf, 0);
    check(OpenWindowStationW((WCHAR *)name_buf, 0, 0x37f) == station, "OpenWindowStation by its name");
    check(!CreateWindowStationW((WCHAR *)name_buf, 1, 0x37f, 0) && CreateWindowStationW((WCHAR *)name_buf, 0, 0x37f, 0) == station,
          "CWF_CREATE_ONLY refuses a name there is; without it, the one there is");
    check(SetThreadDesktop(desk) && !SetThreadDesktop(station) && SetThreadDesktop(desk0), "SetThreadDesktop: a desktop, not a station");

    // Their security: read, a grant added with SetEntriesInAcl, set, and read back.
    void *dacl = 0, *sd = 0;
    DWORD e = GetSecurityInfo(station, 7, 4, 0, 0, &dacl, 0, &sd);
    check(e == 0 && dacl && sd, "GetSecurityInfo of a window station: its DACL");
    static EXPLICIT_ACCESS_W entry;
    BuildExplicitAccessWithNameW(&entry, W(1, "Everyone"), 0x37f, 1, 0);
    check(entry.trustee.form == 1 && entry.mode == 1 && entry.permissions == 0x37f, "BuildExplicitAccessWithName: a named trustee, GRANT_ACCESS");
    void *granted = 0;
    e = SetEntriesInAclW(1, &entry, dacl, &granted);
    check(e == 0 && granted && ace_at(granted, 0, 1, 0, 0x37f) >= 0, "SetEntriesInAcl: Everyone allowed, on the DACL there was");
    check(SetSecurityInfo(station, 7, 4, 0, 0, granted, 0) == 0, "SetSecurityInfo on the window station");
    LocalFree(sd);
    dacl = 0, sd = 0;
    e = GetSecurityInfo(station, 7, 4, 0, 0, &dacl, 0, &sd);
    check(e == 0 && dacl && ace_at(dacl, 0, 1, 0, 0x37f) >= 0, "read back: Everyone's ACE is there");
    LocalFree(sd);
    LocalFree(granted);

    // Entries from nothing: a grant by SID, a deny by name (the caller's user) that goes first, then a revoke.
    static BYTE everyone[12] = {1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0};
    static EXPLICIT_ACCESS_W two[2];
    two[0].permissions = 1, two[0].mode = 1;
    BuildTrusteeWithSidW(&two[0].trustee, everyone);
    BuildExplicitAccessWithNameW(&two[1], W(2, "CURRENT_USER"), 2, 3, 0);
    void *built = 0;
    e = SetEntriesInAclW(2, two, 0, &built);
    check(e == 0 && built && ace_at(built, 1, 5, 1000, 2) == 0 && ace_at(built, 0, 1, 0, 1) == 1,
          "SetEntriesInAcl from no ACL: the deny first, then the grant");
    EXPLICIT_ACCESS_W revoke = two[0];
    revoke.mode = 4;
    void *revoked = 0;
    e = SetEntriesInAclW(1, &revoke, built, &revoked);
    check(e == 0 && ace_at(revoked, 0, 1, 0, 1) < 0 && ace_at(revoked, 1, 5, 1000, 2) == 0, "REVOKE_ACCESS takes the trustee's ACEs away");
    two[0].mode = 9;
    check(SetEntriesInAclW(1, two, 0, &built) == 87, "an ACCESS_MODE there isn't: ERROR_INVALID_PARAMETER");
    LocalFree(revoked);

    check(CloseDesktop(desk) && CloseWindowStation(station), "CloseDesktop and CloseWindowStation");
    check(!GetUserObjectInformationW(station, 2, name_buf, sizeof name_buf, 0) && GetLastError() == 6, "a closed station is gone");
    check(CloseWindowStation(winsta0) && GetUserObjectInformationW(winsta0, 2, name_buf, sizeof name_buf, 0), "WinSta0 stays");

    // ---- Message-only windows ----
    static WNDCLASSEXW wc;
    wc.size = sizeof wc;
    wc.proc = wndproc;
    wc.instance = GetModuleHandleW(0);
    wc.name = W(3, "AtlasMessageOnly");
    check(RegisterClassExW(&wc) != 0, "a class for a message-only window");
    HWND hwnd = CreateWindowExW(0, W(3, "AtlasMessageOnly"), W(4, "messages"), 0, 0, 0, 0, 0, (HWND)(IPTR)-3, 0, wc.instance, 0);
    check(hwnd != 0, "CreateWindowEx with HWND_MESSAGE");
    check(!FindWindowW(W(3, "AtlasMessageOnly"), 0) && FindWindowExW((HWND)(IPTR)-3, 0, W(3, "AtlasMessageOnly"), 0) == hwnd,
          "FindWindow passes it by; FindWindowEx(HWND_MESSAGE) finds it");
    check(GetParent(hwnd) == 0 && GetAncestor(hwnd, 1) == (HWND)(IPTR)-3, "GetParent: none; GetAncestor(GA_PARENT): HWND_MESSAGE");
    ShowWindow(hwnd, 5);
    check(!IsWindowVisible(hwnd), "ShowWindow doesn't show it");
    check(SendMessageW(hwnd, 0x8001, 0, 0) == 42 && messages == 1, "a message sent reaches its WndProc");
    MSG msg;
    PostMessageW(hwnd, 0x8002, 5, 6);
    check(PeekMessageW(&msg, hwnd, 0, 0, 1) && msg.message == 0x8002 && msg.wparam == 5 && msg.lparam == 6, "a message posted is in its queue");
    check(DestroyWindow(hwnd), "DestroyWindow");

    // ---- The registry by NT path ----
    UNICODE_STRING user;
    check(RtlFormatCurrentUserKeyPath(&user) == 0 && wstarts(user.buffer, "\\REGISTRY\\USER\\S-1-5-21-") && user.length == 2 * 34,
          "RtlFormatCurrentUserKeyPath: \\REGISTRY\\USER\\ and the user's SID");
    RtlFreeUnicodeString(&user);
    check(user.buffer == 0 && user.length == 0, "RtlFreeUnicodeString");
    UNICODE_STRING path, child_name, value, empty;
    ustr(&path, 0, "\\Registry\\Machine\\Software\\AtlasNtTest");
    OBJECT_ATTRIBUTES oa = {sizeof oa, 0, &path, 0x40, 0, 0};
    HANDLE key = 0, again = 0, child = 0, opened = 0;
    DWORD disposition = 0;
    check(NtCreateKey(&key, 0xf003f, &oa, 0, 0, 0, &disposition) == 0 && key && disposition == 1, "NtCreateKey: made (REG_CREATED_NEW_KEY)");
    check(NtCreateKey(&again, 0xf003f, &oa, 0, 0, 0, &disposition) == 0 && disposition == 2, "again: REG_OPENED_EXISTING_KEY");
    NtClose(again);
    ustr(&child_name, 1, "child");
    OBJECT_ATTRIBUTES relative = {sizeof relative, key, &child_name, 0x40, 0, 0};
    check(NtCreateKey(&child, 0xf003f, &relative, 0, 0, 0, &disposition) == 0 && disposition == 1, "a subkey, relative to a key's handle");
    ustr(&path, 0, "\\REGISTRY\\MACHINE\\SOFTWARE\\atlasnttest\\CHILD");
    check(NtOpenKeyEx(&opened, 0x20019, &oa, 0) == 0 && opened, "NtOpenKeyEx by its full path, in any case");
    NtClose(opened);
    ustr(&path, 0, "\\Registry\\Machine\\Software\\AtlasNtTest\\none");
    check(NtOpenKeyEx(&opened, 0x20019, &oa, 0) == 0xc0000034, "one there isn't: STATUS_OBJECT_NAME_NOT_FOUND");
    ustr(&path, 0, "\\Device\\Nowhere");
    check(NtOpenKeyEx(&opened, 0x20019, &oa, 0) == 0xc0000034, "a path outside \\Registry: not found");
    DWORD number = 1234;
    ustr(&value, 2, "v");
    check(NtSetValueKey(key, &value, 0, 4, &number, 4) == 0, "NtSetValueKey: a REG_DWORD");
    DWORD got = 0;
    DWORD st = NtQueryValueKey(key, &value, 2, info, sizeof info, &got);
    check(st == 0 && got == 16 && ((DWORD *)info)[1] == 4 && ((DWORD *)info)[2] == 4 && ((DWORD *)info)[3] == 1234,
          "KeyValuePartialInformation: its type, length and data");
    st = NtQueryValueKey(key, &value, 2, info, 13, &got);
    check(st == 0x80000005 && got == 16, "with room for less than the data: STATUS_BUFFER_OVERFLOW, and the size");
    st = NtQueryValueKey(key, &value, 2, info, 4, &got);
    check(st == 0xc0000023 && got == 16, "with less room than its fixed part: STATUS_BUFFER_TOO_SMALL");
    st = NtQueryValueKey(key, &value, 0, info, sizeof info, &got);
    check(st == 0 && got == 14 && ((DWORD *)info)[2] == 2 && *(WCHAR *)(info + 12) == 'v', "KeyValueBasicInformation: the name");
    st = NtQueryValueKey(key, &value, 1, value_buf, sizeof value_buf, &got);
    DWORD offset = ((DWORD *)value_buf)[2];
    check(st == 0 && offset == 24 && ((DWORD *)value_buf)[3] == 4 && *(DWORD *)(value_buf + offset) == 1234 && got == 28,
          "KeyValueFullInformation: the name, and the data at its offset");
    ustr(&value, 2, "nothing");
    check(NtQueryValueKey(key, &value, 2, info, sizeof info, &got) == 0xc0000034, "a value there isn't");
    ustr(&empty, 3, "");
    check(NtDeleteKey(key) != 0, "NtDeleteKey refuses a key with a subkey");
    check(NtDeleteKey(child) == 0 && NtDeleteKey(key) == 0, "NtDeleteKey: the subkey, then the key");
    NtClose(child);
    NtClose(key);
    ustr(&path, 0, "\\Registry\\Machine\\Software\\AtlasNtTest");
    check(NtOpenKeyEx(&opened, 0x20019, &oa, 0) == 0xc0000034, "gone");

    // ---- GetThreadContext ----
    want_full();
    check(GetThreadContext(GetCurrentThread(), context) && context_sp() != 0, "GetThreadContext of the calling thread");
    HANDLE spinning = CreateThread(0, 0, spinner, 0, 0, 0);
    while (spins < 1000) Sleep(1);
    check(SuspendThread(spinning) == 0, "SuspendThread on a thread that's running");
    want_full();
    ok = GetThreadContext(spinning, context);
    // GetThreadContext waits for it to have stopped, as SuspendThread itself needn't.
    UPTR before = spins;
    UPTR pc = context_pc();
    check(ok && pc >= (UPTR)spinner && pc < (UPTR)spinner + 0x100, "its context: the pc inside the loop it runs");
    Sleep(20);
    check(spins == before, "suspended, it doesn't run");
    ResumeThread(spinning);
    stop = 1;
    check(WaitForSingleObject(spinning, 5000) == 0, "resumed, it runs to its end");
    CloseHandle(spinning);
    wake = CreateEventW(0, 1, 0, 0);
    HANDLE waiting = CreateThread(0, 0, waiter, 0, 0, 0);
    Sleep(50);
    SuspendThread(waiting);
    want_full();
    ok = GetThreadContext(waiting, context);
    check(ok && context_pc() != 0 && context_sp() != 0, "the context of a thread suspended while it waits");
    ResumeThread(waiting);
    SetEvent(wake);
    check(WaitForSingleObject(waiting, 5000) == 0, "and it carries on waking");
    CloseHandle(waiting);

    // ---- The Windows Runtime ----
    check(RoInitialize(1) == 0, "RoInitialize");
    void *factory = (void *)1;
    check(RoGetActivationFactory(0, 0, &factory) == 0x80040154 && factory == 0, "RoGetActivationFactory: no class registered");
    void *made = 0, *reference = 0, *copy = 0;
    static BYTE header[24];
    static const WCHAR text[] = {'a', 't', 'l', 'a', 's', 0};
    static const WCHAR inner[] = {'a', 0, 'b', 0};
    check(WindowsCreateString(text, 5, &made) == 0 && WindowsGetStringLen(made) == 5, "WindowsCreateString: a copy, 5 long");
    DWORD length = 0;
    const WCHAR *raw = WindowsGetStringRawBuffer(made, &length);
    check(raw && raw != text && length == 5 && weq(raw, "atlas"), "WindowsGetStringRawBuffer: its own characters, ending in NUL");
    check(WindowsCreateStringReference(text, 5, header, &reference) == 0 && WindowsGetStringRawBuffer(reference, 0) == text,
          "WindowsCreateStringReference: the caller's characters");
    int order = 9;
    check(WindowsCompareStringOrdinal(made, reference, &order) == 0 && order == 0, "the copy and the reference compare equal");
    check(WindowsDuplicateString(reference, &copy) == 0 && WindowsGetStringRawBuffer(copy, 0) != text && WindowsGetStringLen(copy) == 5,
          "WindowsDuplicateString of a reference: a copy of its own");
    void *a = 0, *b = 0;
    WindowsCreateString(text, 1, &a);
    WindowsCreateString(text + 1, 1, &b);
    check(WindowsCompareStringOrdinal(a, b, &order) == 0 && order == -1, "'a' before 't'");
    raw = WindowsGetStringRawBuffer(0, &length);
    check(raw && *raw == 0 && length == 0 && WindowsIsStringEmpty(0) && WindowsGetStringLen(0) == 0, "a null HSTRING is the empty string");
    void *nul = 0;
    int has = 0;
    WindowsCreateString(inner, 3, &nul);
    check(WindowsStringHasEmbeddedNull(nul, &has) == 0 && has == 1, "WindowsStringHasEmbeddedNull");
    check(WindowsCreateString(text, 0, &made) == 0 && made == 0, "an empty string is a null HSTRING");
    WindowsDeleteString(copy);
    WindowsDeleteString(reference);
    WindowsDeleteString(a);
    WindowsDeleteString(b);
    WindowsDeleteString(nul);
    check(WindowsDeleteString(0) == 0, "WindowsDeleteString, of a null one too");
    RoUninitialize();

    // ---- Event tracing, and ProcessPrng ----
    static const GUID provider = {0x12345678, 1, 2, {3, 4, 5, 6, 7, 8, 9, 10}};
    ULONGLONG handle = 0;
    EVENT_DESCRIPTOR event = {1, 0, 0, 4, 0, 0, 0};
    check(EventRegister(&provider, 0, 0, &handle) == 0 && handle != 0, "EventRegister: a handle");
    check(EventWrite(handle, &event, 0, 0) == 0 && EventUnregister(handle) == 0, "EventWrite and EventUnregister, with 64-bit handles");
    static BYTE random[32];
    int nonzero = 0;
    ok = ProcessPrng(random, sizeof random);
    for (int i = 0; i < 32; i++) nonzero += random[i] != 0;
    check(ok == 1 && nonzero > 8, "ProcessPrng: random bytes");

    // ---- Restricted tokens ----
    HANDLE token = 0, restricted = 0;
    static BYTE user_buf[128], groups_buf[512];
    check(OpenProcessToken(GetCurrentProcess(), 0xf01ff, &token) && GetTokenInformation(token, 1, user_buf, sizeof user_buf, &needed),
        "the process's token and its user");
    void *user_sid = *(void **)user_buf;
    static BYTE nt_authority[6] = {0, 0, 0, 0, 0, 5};
    void *restricting = 0;
    check(AllocateAndInitializeSid(nt_authority, 1, 12, 0, 0, 0, 0, 0, 0, 0, &restricting), "the RESTRICTED SID");
    struct { void *sid; DWORD attributes; } deny = {user_sid, 0}, restricts = {restricting, 0};
    // DISABLE_MAX_PRIVILEGE: every privilege but SeChangeNotifyPrivilege goes.
    ok = CreateRestrictedToken(token, 1, 1, &deny, 0, 0, 1, &restricts, &restricted);
    check(ok && restricted != 0, "CreateRestrictedToken");
    check(GetTokenInformation(restricted, 1, user_buf, sizeof user_buf, &needed) && *(DWORD *)(user_buf + sizeof(void *)) == 0x10,
        "its user is deny-only");
    check(GetTokenInformation(restricted, 11, groups_buf, sizeof groups_buf, &needed) && *(DWORD *)groups_buf == 1,
        "TokenRestrictedSids: the one restricting SID");
    check(GetTokenInformation(restricted, 3, groups_buf, sizeof groups_buf, &needed) && *(DWORD *)groups_buf == 1
            && *(DWORD *)(groups_buf + 4) == 23, "TokenPrivileges: only SeChangeNotifyPrivilege");
    check(GetTokenInformation(token, 11, groups_buf, sizeof groups_buf, &needed) && *(DWORD *)groups_buf == 0,
        "the process's own token isn't restricted");
    CloseHandle(restricted);
    CloseHandle(token);
    FreeSid(restricting);

    // ---- The process's own handles, as a sandbox's handle closer lists them ----
    {
        typedef DWORD(__stdcall * INFO)(HANDLE, DWORD, void *, DWORD, DWORD *);
        INFO query_process = (INFO)GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtQueryInformationProcess");
        INFO query_object = (INFO)GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtQueryObject");
        HANDLE named = CreateEventW(0, 1, 0, L"AtlasSandboxNamedEvent");
        static BYTE snapshot[16384], object[512];
        DWORD need = 0, status = query_process((HANDLE)-1, 51, snapshot, sizeof snapshot, &need);
        UPTR count = *(UPTR *)snapshot, each = 3 * sizeof(void *) + 16;
        int listed = 0;
        for (UPTR i = 0; status == 0 && i < count; i++) {
            BYTE *entry = snapshot + 2 * sizeof(void *) + i * each;
            if (*(HANDLE *)entry == named && *(DWORD *)(entry + 3 * sizeof(void *) + 4) == 16) listed = 1;
        }
        check(status == 0 && count > 3 && listed, "ProcessHandleInformation: the event is listed, as an Event (16)");
        status = query_process((HANDLE)-1, 58, snapshot, sizeof snapshot, &need);
        int in_table = 0;
        for (DWORD i = 0; status == 0 && i < need / 4; i++) in_table |= ((DWORD *)snapshot)[i] == (DWORD)(UPTR)named;
        check(status == 0 && in_table, "ProcessHandleTable: its value among the process's handles");
        status = query_process((HANDLE)-1, 51, snapshot, 8, &need);
        check(status == 0xc0000004 && need > 8, "with too little room: STATUS_INFO_LENGTH_MISMATCH and the length needed");
        status = query_object(named, 2, object, sizeof object, &need);
        check(status == 0 && weq(*(WCHAR **)(object + sizeof(void *)), "Event"), "NtQueryObject: its type, Event");
        status = query_object(named, 1, object, sizeof object, &need);
        const WCHAR *name = *(WCHAR **)(object + sizeof(void *));
        int ends = 0;
        if (status == 0 && name) {
            int n = 0;
            while (name[n]) n++;
            ends = n > 22 && weq(name + n - 22, "AtlasSandboxNamedEvent");
        }
        check(ends, "and its name, in BaseNamedObjects");
        CloseHandle(named);
    }

#if defined(__x86_64__)
    // ---- ntdll's system-call stubs ----
    HANDLE ntdll = GetModuleHandleW(L"ntdll.dll");
    const BYTE *stub = GetProcAddress(ntdll, "NtQueryVirtualMemory");
    const BYTE *other = GetProcAddress(ntdll, "NtOpenProcess");
    int shaped = stub && stub[0] == 0x4c && stub[1] == 0x8b && stub[2] == 0xd1 && stub[3] == 0xb8 && stub[8] == 0xf6
        && stub[9] == 0x04 && stub[10] == 0x25 && *(DWORD *)(stub + 11) == 0x7ffe0308 && stub[16] == 0x75 && stub[17] == 0x03
        && stub[18] == 0x0f && stub[19] == 0x05 && stub[20] == 0xc3 && stub[21] == 0xcd && stub[22] == 0x2e && stub[23] == 0xc3;
    check(shaped, "NtQueryVirtualMemory is Windows 10's stub: mov r10, rcx; mov eax; test; jne; syscall; ret; int 2e; ret");
    check(other && other[0] == 0x4c && *(DWORD *)(other + 4) != *(DWORD *)(stub + 4) && ((UPTR)stub & 31) == 0 && ((UPTR)other & 31) == 0,
        "each stub has a number of its own, 32 bytes apart");
    typedef DWORD(__stdcall * QUERY)(HANDLE, void *, int, void *, UPTR, UPTR *);
    static BYTE mbi[48];
    UPTR returned = 0;
    DWORD status = ((QUERY)stub)((HANDLE)-1, (void *)stub, 0, mbi, sizeof mbi, &returned);
    check(status == 0 && returned == 48 && *(UPTR *)mbi == ((UPTR)stub & ~(UPTR)0xfff), "NtQueryVirtualMemory through its stub");
    // A copy of the stub, elsewhere, as a sandbox keeps the original when it patches the export.
    BYTE *stub_copy = VirtualAlloc(0, 4096, 0x3000, 0x40);
    for (int i = 0; i < 32; i++) stub_copy[i] = stub[i];
    for (int i = 0; i < 48; i++) mbi[i] = 0;
    returned = 0;
    status = ((QUERY)stub_copy)((HANDLE)-1, (void *)stub_copy, 0, mbi, sizeof mbi, &returned);
    check(status == 0 && returned == 48 && *(UPTR *)mbi == (UPTR)stub_copy, "NtQueryVirtualMemory through a copy of its stub");
#endif

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
