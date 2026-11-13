// What larger programs call as they start, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it.
//
// It checks: the heap's sizes and reallocation (in place, moved, zeroed) and its queries; encoded pointers; one-time
// initialization begun and completed by hand and run once; interlocked lists; the debugger's absence, DebugBreak to a
// vectored handler, and UnhandledExceptionFilter handing over to the program's filter; IsBad*Ptr; CompareString,
// LCMapString (case one character for one, and sort keys) and GetStringType; dates and times as text; disk space,
// drive types and FindFirstFileEx; the console's modes and SetStdHandle; OpenThread, priorities, affinity and the
// session; psapi's module paths and information and memory counters; the monitor, the window station, MapWindowPoints
// and MsgWaitForMultipleObjects; fonts from memory; the event log and RtlGenRandom; CommandLineToArgvW,
// SHGetKnownFolderPath and IsUserAnAdmin; oleaut32's BSTRs and VARIANTs; BCryptGenRandom; and Winsock's start, byte
// order and last error. oleaut32, ws2_32 and IsUserAnAdmin are imported by ordinal, as programs import them.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned char BYTE;
typedef __SIZE_TYPE__ SIZE_T;
typedef __UINTPTR_TYPE__ UPTR;
typedef struct { DWORD a; unsigned short b, c; unsigned char d[8]; } GUID;
typedef struct { unsigned short year, month, weekday, day, hour, minute, second, ms; } SYSTEMTIME;
typedef struct { int left, top, right, bottom; } RECT;
typedef struct { int x, y; } POINT;
typedef struct { DWORD size; RECT monitor, work; DWORD flags; } MONITORINFO;
typedef struct { void *base; DWORD size; void *entry; } MODULEINFO;
typedef struct { DWORD cb, faults; SIZE_T peak_ws, ws, qpp, qp, qpnp, qnp, pagefile, peak_pagefile; } PMC;
typedef struct { DWORD code, flags; void *record; void *address; DWORD count; UPTR info[15]; } EXCEPTION_RECORD;
typedef struct { EXCEPTION_RECORD *record; void *context; } EXCEPTION_POINTERS;
typedef struct { unsigned short vt, r1, r2, r3; void *data; void *extra; } VARIANT;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall SetStdHandle(DWORD, HANDLE);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K HANDLE __stdcall GetProcessHeap(void);
K void *__stdcall HeapAlloc(HANDLE, DWORD, SIZE_T);
K void *__stdcall HeapReAlloc(HANDLE, DWORD, void *, SIZE_T);
K int __stdcall HeapFree(HANDLE, DWORD, void *);
K SIZE_T __stdcall HeapSize(HANDLE, DWORD, const void *);
K int __stdcall HeapValidate(HANDLE, DWORD, const void *);
K int __stdcall HeapLock(HANDLE);
K int __stdcall HeapUnlock(HANDLE);
K int __stdcall HeapWalk(HANDLE, void *);
K DWORD __stdcall GetProcessHeaps(DWORD, HANDLE *);
K int __stdcall HeapQueryInformation(HANDLE, int, void *, SIZE_T, SIZE_T *);
K int __stdcall HeapSetInformation(HANDLE, int, void *, SIZE_T);
K void *__stdcall EncodePointer(void *);
K void *__stdcall DecodePointer(void *);
K int __stdcall InitOnceBeginInitialize(void **, DWORD, int *, void **);
K int __stdcall InitOnceComplete(void **, DWORD, void *);
K int __stdcall InitOnceExecuteOnce(void **, int(__stdcall *)(void **, void *, void **), void *, void **);
K void __stdcall InitializeSListHead(void *);
K void *__stdcall InterlockedPushEntrySList(void *, void *);
K void *__stdcall InterlockedPopEntrySList(void *);
K void *__stdcall InterlockedFlushSList(void *);
K unsigned short __stdcall QueryDepthSList(void *);
K int __stdcall IsDebuggerPresent(void);
K void __stdcall DebugBreak(void);
K void *__stdcall AddVectoredExceptionHandler(DWORD, void *);
K DWORD __stdcall RemoveVectoredExceptionHandler(void *);
K void *__stdcall SetUnhandledExceptionFilter(void *);
K long __stdcall UnhandledExceptionFilter(EXCEPTION_POINTERS *);
K int __stdcall IsBadWritePtr(void *, UPTR);
K int __stdcall IsBadReadPtr(const void *, UPTR);
K int __stdcall CompareStringW(DWORD, DWORD, const WCHAR *, int, const WCHAR *, int);
K int __stdcall CompareStringA(DWORD, DWORD, const char *, int, const char *, int);
K int __stdcall LCMapStringW(DWORD, DWORD, const WCHAR *, int, void *, int);
K int __stdcall GetStringTypeW(DWORD, const WCHAR *, int, unsigned short *);
K int __stdcall GetDateFormatW(DWORD, DWORD, const SYSTEMTIME *, const WCHAR *, WCHAR *, int);
K int __stdcall GetTimeFormatW(DWORD, DWORD, const SYSTEMTIME *, const WCHAR *, WCHAR *, int);
K int __stdcall GetDiskFreeSpaceExW(const WCHAR *, unsigned long long *, unsigned long long *, unsigned long long *);
K unsigned __stdcall GetDriveTypeW(const WCHAR *);
K HANDLE __stdcall FindFirstFileExW(const WCHAR *, int, void *, int, void *, DWORD);
K int __stdcall FindClose(HANDLE);
K int __stdcall GetConsoleMode(HANDLE, DWORD *);
K int __stdcall SetConsoleMode(HANDLE, DWORD);
K DWORD __stdcall GetCurrentThreadId(void);
K DWORD __stdcall GetCurrentProcessId(void);
K HANDLE __stdcall GetCurrentProcess(void);
K HANDLE __stdcall GetCurrentThread(void);
K HANDLE __stdcall OpenThread(DWORD, int, DWORD);
K int __stdcall CloseHandle(HANDLE);
K int __stdcall SetThreadPriority(HANDLE, int);
K int __stdcall GetThreadPriority(HANDLE);
K int __stdcall GetProcessAffinityMask(HANDLE, UPTR *, UPTR *);
K UPTR __stdcall SetThreadAffinityMask(HANDLE, UPTR);
K int __stdcall ProcessIdToSessionId(DWORD, DWORD *);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K int __stdcall SetEvent(HANDLE);
K void *__stdcall LocalFree(void *);
K DWORD __stdcall GetModuleFileNameExW(HANDLE, HANDLE, WCHAR *, DWORD);
K int __stdcall GetModuleInformation(HANDLE, HANDLE, MODULEINFO *, DWORD);
K int __stdcall GetProcessMemoryInfo(HANDLE, PMC *, DWORD);
K HANDLE __stdcall MonitorFromWindow(HANDLE, DWORD);
K HANDLE __stdcall MonitorFromPoint(POINT, DWORD);
K int __stdcall GetMonitorInfoW(HANDLE, MONITORINFO *);
K HANDLE __stdcall GetProcessWindowStation(void);
K int __stdcall GetUserObjectInformationW(HANDLE, int, void *, DWORD, DWORD *);
K int __stdcall MapWindowPoints(HANDLE, HANDLE, POINT *, unsigned);
K DWORD __stdcall MsgWaitForMultipleObjects(DWORD, const HANDLE *, int, DWORD, DWORD);
K int __stdcall PostThreadMessageW(DWORD, unsigned, UPTR, UPTR);
K int __stdcall PeekMessageW(void *, HANDLE, unsigned, unsigned, unsigned);
K HANDLE __stdcall AddFontMemResourceEx(void *, DWORD, void *, DWORD *);
K int __stdcall RemoveFontMemResourceEx(HANDLE);
K HANDLE __stdcall RegisterEventSourceW(const WCHAR *, const WCHAR *);
K int __stdcall ReportEventW(HANDLE, unsigned short, unsigned short, DWORD, void *, unsigned short, DWORD, const WCHAR **, void *);
K int __stdcall DeregisterEventSource(HANDLE);
K unsigned char __stdcall SystemFunction036(void *, DWORD);
K WCHAR **__stdcall CommandLineToArgvW(const WCHAR *, int *);
K long __stdcall SHGetKnownFolderPath(const GUID *, DWORD, HANDLE, WCHAR **);
K int __stdcall IsUserAnAdmin(void);
K void __stdcall CoTaskMemFree(void *);
K WCHAR *__stdcall SysAllocString(const WCHAR *);
K WCHAR *__stdcall SysAllocStringLen(const WCHAR *, unsigned);
K int __stdcall SysReAllocString(WCHAR **, const WCHAR *);
K void __stdcall SysFreeString(WCHAR *);
K unsigned __stdcall SysStringLen(const WCHAR *);
K unsigned __stdcall SysStringByteLen(const WCHAR *);
K void __stdcall VariantInit(VARIANT *);
K long __stdcall VariantClear(VARIANT *);
K long __stdcall VariantCopy(VARIANT *, const VARIANT *);
K long __stdcall BCryptGenRandom(void *, void *, DWORD, DWORD);
K int __stdcall WSAStartup(unsigned short, void *);
K int __stdcall WSACleanup(void);
K int __stdcall WSAGetLastError(void);
K void __stdcall WSASetLastError(int);
K unsigned short __stdcall htons(unsigned short);
K DWORD __stdcall ntohl(DWORD);
K int __stdcall __WSAFDIsSet(UPTR, void *);

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
static WCHAR wbuf[4][128];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)(BYTE)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)(BYTE)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int wends(const WCHAR *a, const char *b) {
    int n = 0, m = (int)len(b);
    while (a[n]) n++;
    return n >= m && weq(a + n - m, b);
}
static int bytes_less(const BYTE *a, const BYTE *b) { while (*a && *a == *b) { a++; b++; } return *a < *b; }

static int once_runs;
static int __stdcall once_function(void **once, void *parameter, void **context) {
    (void)once;
    once_runs++;
    *context = parameter;
    return 1;
}

static int breaks;
static long __stdcall on_break(EXCEPTION_POINTERS *p) {
    if (p->record->code != 0x80000003) return 0;
    breaks++;
    return -1;
}
static long __stdcall filter(EXCEPTION_POINTERS *p) { return p->record->code == 0xe0001234 ? -1 : 0; }

typedef struct entry { struct entry *next; int value; } __attribute__((aligned(16))) ENTRY;
static __attribute__((aligned(16))) BYTE slist[16];
static ENTRY entries[3];
static int local_word;
// Large buffers live here, zeroed, so nothing has to call memset to clear them.
static BYTE walk[64], data[600], msg[48], random[32], more[16], wsa[408];
static PMC pmc;
static MODULEINFO mi;

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("what larger programs call as they start, arm64\n");
#elif defined(__i386__)
    say("what larger programs call as they start, x86 (WoW64)\n");
#else
    say("what larger programs call as they start, x86-64\n");
#endif
    WCHAR s[260];

    // ---- The heap ----
    HANDLE heap = GetProcessHeap();
    BYTE *p = HeapAlloc(heap, 0, 10);
    for (int i = 0; i < 10; i++) p[i] = (BYTE)(i + 1);
    check(HeapSize(heap, 0, p) == 10, "HeapSize: the size asked for");
    BYTE *q = HeapReAlloc(heap, 8, p, 5000);
    int kept = q != 0 && HeapSize(heap, 0, q) == 5000;
    for (int i = 0; kept && i < 10; i++) kept = q[i] == i + 1;
    for (int i = 10; kept && i < 5000; i++) kept = q[i] == 0;
    check(kept, "HeapReAlloc: grown, what was there kept, the rest zeroed");
    check(HeapReAlloc(heap, 0x10, q, 100000) == 0, "HeapReAlloc: HEAP_REALLOC_IN_PLACE_ONLY that doesn't fit fails");
    check(HeapReAlloc(heap, 0, q, 8) == q && HeapSize(heap, 0, q) == 8, "HeapReAlloc: shrunk where it is");
    check(HeapFree(heap, 0, q) && HeapValidate(heap, 0, 0) && HeapLock(heap) && HeapUnlock(heap), "HeapValidate, HeapLock and HeapUnlock");
    HANDLE heaps[4] = {0};
    check(GetProcessHeaps(4, heaps) == 1 && heaps[0] == heap, "GetProcessHeaps: the process heap");
    DWORD compat = 0;
    SIZE_T got = 0;
    check(HeapQueryInformation(heap, 0, &compat, 4, &got) && compat == 2 && got == 4, "HeapQueryInformation: the low-fragmentation heap");
    check(HeapSetInformation(heap, 1, 0, 0), "HeapSetInformation: terminate on corruption");
    check(!HeapWalk(heap, walk) && GetLastError() == 259, "HeapWalk: no blocks listed, ERROR_NO_MORE_ITEMS");

    // ---- Encoded pointers, one-time initialization, lists ----
    void *encoded = EncodePointer(&local_word);
    check(encoded != (void *)&local_word && DecodePointer(encoded) == (void *)&local_word && DecodePointer(EncodePointer(0)) == 0, "EncodePointer and DecodePointer");
    void *once = 0, *context = 0;
    int pending = 0;
    check(!InitOnceBeginInitialize(&once, 1, &pending, &context), "InitOnceBeginInitialize: INIT_ONCE_CHECK_ONLY before it's done");
    check(InitOnceBeginInitialize(&once, 0, &pending, &context) && pending, "InitOnceBeginInitialize: this caller does it");
    check(InitOnceComplete(&once, 0, (void *)0x1000), "InitOnceComplete with a context");
    pending = 1;
    check(InitOnceBeginInitialize(&once, 0, &pending, &context) && !pending && context == (void *)0x1000, "InitOnceBeginInitialize: done, its context given");
    void *once2 = 0, *context2 = 0;
    InitOnceExecuteOnce(&once2, once_function, (void *)0x2000, &context2);
    context2 = 0;
    InitOnceExecuteOnce(&once2, once_function, (void *)0x3000, &context2);
    check(once_runs == 1 && context2 == (void *)0x2000, "InitOnceExecuteOnce: run once, its context kept");
    InitializeSListHead(slist);
    for (int i = 0; i < 3; i++) {
        entries[i].value = i;
        InterlockedPushEntrySList(slist, &entries[i]);
    }
    check(QueryDepthSList(slist) == 3, "InterlockedPushEntrySList, and QueryDepthSList");
    ENTRY *top = InterlockedPopEntrySList(slist);
    check(top == &entries[2] && QueryDepthSList(slist) == 2, "InterlockedPopEntrySList: last in, first out");
    top = InterlockedFlushSList(slist);
    check(top == &entries[1] && top->next == &entries[0] && QueryDepthSList(slist) == 0 && InterlockedPopEntrySList(slist) == 0, "InterlockedFlushSList: all of it, linked");

    // ---- The debugger, exceptions, and pointers ----
    check(!IsDebuggerPresent(), "IsDebuggerPresent: no debugger");
    void *handler = AddVectoredExceptionHandler(1, on_break);
    DebugBreak();
    RemoveVectoredExceptionHandler(handler);
    check(breaks == 1, "DebugBreak: a breakpoint exception, carried on from");
    SetUnhandledExceptionFilter(filter);
    EXCEPTION_RECORD record = {0xe0001234, 0, 0, 0, 0, {0}};
    EXCEPTION_POINTERS pointers = {&record, 0};
    check(UnhandledExceptionFilter(&pointers) == -1, "UnhandledExceptionFilter: the program's filter decides");
    SetUnhandledExceptionFilter(0);
    check(!IsBadWritePtr(&local_word, 4) && IsBadWritePtr(0, 4) && IsBadWritePtr((void *)mainCRTStartup, 4), "IsBadWritePtr: data, null, code");
    check(!IsBadReadPtr((void *)mainCRTStartup, 16) && IsBadReadPtr((void *)16, 4) && !IsBadReadPtr(0, 0), "IsBadReadPtr: code, the first page, nothing");

    // ---- Text ----
    check(CompareStringW(0x409, 0, W(0, "apple"), -1, W(1, "Banana"), -1) == 1, "CompareStringW: by letter before case");
    check(CompareStringW(0x409, 0, W(0, "a"), -1, W(1, "A"), -1) == 1, "CompareStringW: lowercase first");
    check(CompareStringW(0x409, 1, W(0, "Hello"), 5, W(1, "hELLO"), -1) == 2, "CompareStringW: NORM_IGNORECASE, a length given");
    check(CompareStringA(0x409, 0, "co-op", -1, "cop", -1) == 1 && CompareStringA(0x409, 0x1000, "co-op", -1, "coop", -1) == 1, "CompareStringA: word sort and string sort");
    WCHAR mapped[16];
    int n = LCMapStringW(0x409, 0x200, W(0, "stra\xdf" "e"), -1, mapped, 16);
    check(n == 7 && weq(mapped, "STRA\xdf" "E"), "LCMapStringW: uppercase, one character for one, with the NUL");
    check(LCMapStringW(0x409, 0x100, W(0, "ABC"), 3, 0, 0) == 3, "LCMapStringW: how much room");
    BYTE key_a[64], key_b[64];
    int ka = LCMapStringW(0x409, 0x400, W(0, "apple"), -1, key_a, 64);
    int kb = LCMapStringW(0x409, 0x400, W(0, "Banana"), -1, key_b, 64);
    check(ka > 0 && kb > 0 && key_a[ka - 1] == 0 && bytes_less(key_a, key_b), "LCMapStringW: sort keys in CompareString's order");
    unsigned short types[4];
    check(GetStringTypeW(1, W(0, "A1 ,"), 4, types) && (types[0] & 0x101) == 0x101 && (types[1] & 4) && (types[2] & 0x48) == 0x48 && (types[3] & 0x10),
          "GetStringTypeW: CT_CTYPE1's upper, alpha, digit, space, blank and punctuation");
    SYSTEMTIME st = {2026, 11, 0, 13, 14, 5, 9, 0};
    check(GetDateFormatW(0x409, 0, &st, 0, s, 64) == 11 && weq(s, "11/13/2026"), "GetDateFormatW: the short date, its length with the NUL");
    check(GetDateFormatW(0x409, 2, &st, 0, s, 64) && weq(s, "Friday, November 13, 2026"), "GetDateFormatW: the long date, its day of the week worked out");
    check(GetDateFormatW(0x409, 0, &st, W(0, "ddd MMM yy"), s, 64) && weq(s, "Fri Nov 26"), "GetDateFormatW: a picture");
    check(GetDateFormatW(0x409, 0, &st, 0, s, 5) == 0 && GetLastError() == 122 && GetDateFormatW(0x409, 0, &st, 0, 0, 0) == 11, "GetDateFormatW: too little room, and how much");
    check(GetTimeFormatW(0x409, 0, &st, 0, s, 64) && weq(s, "2:05:09 PM"), "GetTimeFormatW");
    check(GetTimeFormatW(0x409, 2, &st, 0, s, 64) && weq(s, "2:05 PM"), "GetTimeFormatW: TIME_NOSECONDS");
    check(GetTimeFormatW(0x409, 0xc, &st, 0, s, 64) && weq(s, "14:05:09"), "GetTimeFormatW: 24 hours, no marker");

    // ---- Disks and the console ----
    unsigned long long avail = 0, total = 0, free_bytes = 0;
    check(GetDiskFreeSpaceExW(W(0, "C:\\"), &avail, &total, &free_bytes) && avail && total >= free_bytes && free_bytes == avail, "GetDiskFreeSpaceExW");
    check(GetDriveTypeW(W(0, "C:\\")) == 3 && GetDriveTypeW(0) == 3 && GetDriveTypeW(W(0, "Q:\\")) == 1, "GetDriveTypeW: C: is fixed, Q: isn't there");
    HANDLE find = FindFirstFileExW(W(0, "C:\\Windows\\*"), 1, data, 0, 0, 0);
    check(find != (HANDLE)-1 && FindClose(find), "FindFirstFileExW, the basic info level");
    DWORD mode = 0;
    check(GetConsoleMode(out, &mode) && SetConsoleMode(out, mode), "GetConsoleMode and SetConsoleMode on the console");
    check(!GetConsoleMode((HANDLE)0x1234, &mode) && GetLastError() == 6, "GetConsoleMode: not the console's");
    HANDLE err = GetStdHandle((DWORD)-12);
    check(SetStdHandle((DWORD)-12, out) && GetStdHandle((DWORD)-12) == out && SetStdHandle((DWORD)-12, err), "SetStdHandle");

    // ---- Threads, processors and the session ----
    HANDLE thread = OpenThread(0x1fffff, 0, GetCurrentThreadId());
    check(thread != 0 && CloseHandle(thread) && OpenThread(0x1fffff, 0, 0x7ffffff0) == 0, "OpenThread: this thread, and one that isn't");
    check(SetThreadPriority(GetCurrentThread(), 1) && GetThreadPriority(GetCurrentThread()) == 0 && !SetThreadPriority(GetCurrentThread(), 7), "SetThreadPriority and GetThreadPriority");
    UPTR mine = 0, all = 0;
    check(GetProcessAffinityMask(GetCurrentProcess(), &mine, &all) && mine && mine == all, "GetProcessAffinityMask");
    check(SetThreadAffinityMask(GetCurrentThread(), all) == all, "SetThreadAffinityMask: the mask it had");
    DWORD session = 0;
    check(ProcessIdToSessionId(GetCurrentProcessId(), &session) && session == 1, "ProcessIdToSessionId: session 1");

    // ---- psapi ----
    check(GetModuleFileNameExW(GetCurrentProcess(), 0, s, 260) > 0 && wends(s, "\\startup.exe"), "GetModuleFileNameExW: the program");
    HANDLE kernel32 = GetModuleHandleW(W(0, "kernel32.dll"));
    check(GetModuleFileNameExW(GetCurrentProcess(), kernel32, s, 260) > 0 && wends(s, "KERNEL32.dll"), "GetModuleFileNameExW: a DLL");
    HANDLE self = GetModuleHandleW(0);
    check(GetModuleInformation(GetCurrentProcess(), self, &mi, sizeof mi) && mi.base == self && mi.size > 0x1000 && (BYTE *)mi.entry > (BYTE *)self, "GetModuleInformation: base, size and entry point");
    check(GetProcessMemoryInfo(GetCurrentProcess(), &pmc, sizeof pmc) && pmc.cb == sizeof pmc && pmc.ws > 0x10000, "GetProcessMemoryInfo");

    // ---- user32 ----
    HANDLE monitor = MonitorFromWindow(0, 1);
    POINT at = {10, 10};
    check(monitor != 0 && MonitorFromPoint(at, 1) == monitor, "MonitorFromWindow and MonitorFromPoint: the one monitor");
    MONITORINFO info = {sizeof info};
    check(GetMonitorInfoW(monitor, &info) && info.monitor.right > 0 && info.monitor.bottom > 0 && info.flags == 1, "GetMonitorInfoW: the primary");
    DWORD needed = 0;
    check(GetUserObjectInformationW(GetProcessWindowStation(), 2, s, sizeof s, &needed) && weq(s, "WinSta0") && needed == 16, "GetUserObjectInformationW: WinSta0");
    POINT pts[2] = {{1, 2}, {3, 4}};
    check(MapWindowPoints(0, 0, pts, 2) == 0 && pts[1].x == 3 && pts[1].y == 4, "MapWindowPoints: the screen to the screen");
    HANDLE event = CreateEventW(0, 1, 0, 0);
    check(MsgWaitForMultipleObjects(1, &event, 0, 20, 0x4ff) == 0x102, "MsgWaitForMultipleObjects: nothing, the time runs out");
    PostThreadMessageW(GetCurrentThreadId(), 0x400, 1, 2);
    check(MsgWaitForMultipleObjects(1, &event, 0, 1000, 0x4ff) == 1, "MsgWaitForMultipleObjects: a message");
    while (PeekMessageW(msg, 0, 0, 0, 1)) {}
    SetEvent(event);
    check(MsgWaitForMultipleObjects(1, &event, 0, 1000, 0x4ff) == 0, "MsgWaitForMultipleObjects: the object");
    CloseHandle(event);

    // ---- gdi32, advapi32 ----
    DWORD fonts = 0;
    HANDLE font = AddFontMemResourceEx(data, 100, 0, &fonts);
    check(font != 0 && fonts == 1 && RemoveFontMemResourceEx(font), "AddFontMemResourceEx and RemoveFontMemResourceEx");
    HANDLE source = RegisterEventSourceW(0, W(0, "startup"));
    const WCHAR *strings[1] = {W(1, "started")};
    check(source != 0 && ReportEventW(source, 4, 0, 1, 0, 1, 0, strings, 0) && DeregisterEventSource(source), "RegisterEventSourceW, ReportEventW, DeregisterEventSource");
    int any = 0;
    check(SystemFunction036(random, 32) != 0, "RtlGenRandom (SystemFunction036)");
    for (int i = 0; i < 32; i++) any |= random[i];
    check(any != 0, "RtlGenRandom's bytes aren't all zero");

    // ---- shell32 ----
    int argc = 0;
    WCHAR **argv = CommandLineToArgvW(W(0, "app.exe one \"two three\" fo\\\"ur"), &argc);
    check(argv && argc == 4 && weq(argv[0], "app.exe") && weq(argv[2], "two three") && weq(argv[3], "fo\"ur") && argv[4] == 0, "CommandLineToArgvW");
    check(LocalFree(argv) == 0, "CommandLineToArgvW's block, freed with LocalFree");
    GUID roaming = {0x3eb685db, 0x65f9, 0x4cf6, {0xa0, 0x3a, 0xe3, 0xef, 0x65, 0x72, 0x9f, 0x3d}};
    WCHAR *path = 0;
    check(SHGetKnownFolderPath(&roaming, 0, 0, &path) == 0 && path && weq(path, "C:\\Users\\carbon\\AppData\\Roaming"), "SHGetKnownFolderPath: RoamingAppData");
    CoTaskMemFree(path);
    GUID unknown = {1, 2, 3, {4, 5, 6, 7, 8, 9, 10, 11}};
    path = (WCHAR *)1;
    check(SHGetKnownFolderPath(&unknown, 0, 0, &path) == (long)0x80070002 && path == 0, "SHGetKnownFolderPath: a folder there isn't");
    check(IsUserAnAdmin(), "IsUserAnAdmin (#680)");

    // ---- oleaut32, by ordinal ----
    WCHAR *b = SysAllocString(W(0, "hello"));
    check(b && SysStringLen(b) == 5 && SysStringByteLen(b) == 10 && b[5] == 0 && ((DWORD *)b)[-1] == 10, "SysAllocString: its count before it, a NUL after");
    check(SysReAllocString(&b, W(0, "longer text")) && SysStringLen(b) == 11 && weq(b, "longer text"), "SysReAllocString");
    WCHAR *empty = SysAllocStringLen(0, 3);
    check(empty && SysStringLen(empty) == 3 && empty[0] == 0 && empty[3] == 0, "SysAllocStringLen with no text: zeroed");
    SysFreeString(empty);
    SysFreeString(0);
    check(SysStringLen(0) == 0, "SysStringLen of null");
    VARIANT v, copy;
    VariantInit(&v);
    VariantInit(&copy);
    v.vt = 8;
    v.data = b;
    check(VariantCopy(&copy, &v) == 0 && copy.vt == 8 && copy.data != b && weq(copy.data, "longer text"), "VariantCopy: its own BSTR");
    check(VariantClear(&v) == 0 && v.vt == 0 && VariantClear(&copy) == 0 && copy.vt == 0, "VariantClear: the BSTRs freed, VT_EMPTY");

    // ---- bcrypt ----
    any = 0;
    check(BCryptGenRandom(0, more, 16, 2) == 0, "BCryptGenRandom: the system's generator");
    for (int i = 0; i < 16; i++) any |= more[i];
    check(any && BCryptGenRandom(0, more, 16, 0) == (long)0xc0000008, "BCryptGenRandom: its bytes, and no algorithm handle");

    // ---- Winsock, by ordinal ----
    check(WSAStartup(0x0202, wsa) == 0 && wsa[0] == 2 && wsa[1] == 2 && wsa[2] == 2 && wsa[3] == 2, "WSAStartup: 2.2");
    check(htons(0x1234) == 0x3412 && ntohl(0x11223344) == 0x44332211, "htons and ntohl");
    WSASetLastError(10035);
    check(WSAGetLastError() == 10035 && GetLastError() == 10035, "WSASetLastError and WSAGetLastError: the thread's last error");
    UPTR set[3] = {2, 0x40, 0x48};
    check(__WSAFDIsSet(0x48, set) && !__WSAFDIsSet(0x50, set), "__WSAFDIsSet");
    check(WSACleanup() == 0 && WSACleanup() == -1 && WSAGetLastError() == 10093, "WSACleanup, and once too often: WSANOTINITIALISED");

    ExitProcess(failures);
}
