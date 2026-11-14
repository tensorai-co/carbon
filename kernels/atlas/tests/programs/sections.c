// Sections and waitable timers on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// Sections:
// - one the page file backs, seen by two views at once, and by a copy-on-write view that keeps its writes;
// - VirtualQuery of a view, private memory, the program's image and free space;
// - access refused, alignment refused; a read-only handle duplicated from a writable one, which maps only for
//   reading and can't be duplicated back to writing;
// - a file mapping read from its file, written back by FlushViewOfFile and by UnmapViewOfFile, growing its file,
//   and refused for an empty file;
// - a named section, and an inherited one, shared with another process.
//
// Waitable timers: notification and synchronization, relative and absolute, periodic and cancelled, an APC routine,
// and opened by name.

typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned long long u64;
typedef long long i64;

typedef struct {
    void *base, *allocation_base;
    DWORD allocation_protect;
#if __SIZEOF_POINTER__ == 8
    unsigned short partition;
#endif
    SIZE_T size;
    DWORD state, protect, type;
} MEMORY_BASIC_INFORMATION;

typedef struct {
    DWORD cb;
    WCHAR *reserved, *desktop, *title;
    DWORD x, y, x_size, y_size, x_chars, y_chars, fill, flags;
    unsigned short show, reserved2_size;
    void *reserved2;
    HANDLE std_input, std_output, std_error;
} STARTUPINFOW;

typedef struct {
    HANDLE process, thread;
    DWORD pid, tid;
} PROCESS_INFORMATION;

typedef struct {
    DWORD length;
    void *descriptor;
    int inherit;
} SECURITY_ATTRIBUTES;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K int __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K int __stdcall CloseHandle(HANDLE);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K DWORD __stdcall GetFileSize(HANDLE, DWORD *);
K DWORD __stdcall SetFilePointer(HANDLE, long, long *, DWORD);
K int __stdcall DeleteFileW(const WCHAR *);
K HANDLE __stdcall CreateFileMappingW(HANDLE, SECURITY_ATTRIBUTES *, DWORD, DWORD, DWORD, const WCHAR *);
K HANDLE __stdcall OpenFileMappingW(DWORD, int, const WCHAR *);
K void *__stdcall MapViewOfFile(HANDLE, DWORD, DWORD, DWORD, SIZE_T);
K void *__stdcall MapViewOfFileEx(HANDLE, DWORD, DWORD, DWORD, SIZE_T, void *);
K int __stdcall UnmapViewOfFile(void *);
K int __stdcall FlushViewOfFile(void *, SIZE_T);
K SIZE_T __stdcall VirtualQuery(const void *, MEMORY_BASIC_INFORMATION *, SIZE_T);
K void *__stdcall VirtualAlloc(void *, SIZE_T, DWORD, DWORD);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K int __stdcall CreateProcessW(const WCHAR *, WCHAR *, SECURITY_ATTRIBUTES *, SECURITY_ATTRIBUTES *, int, DWORD, void *,
                               const WCHAR *, STARTUPINFOW *, PROCESS_INFORMATION *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K int __stdcall GetExitCodeProcess(HANDLE, DWORD *);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K char *__stdcall GetCommandLineA(void);
K HANDLE __stdcall CreateWaitableTimerW(SECURITY_ATTRIBUTES *, int, const WCHAR *);
K HANDLE __stdcall CreateWaitableTimerExW(SECURITY_ATTRIBUTES *, const WCHAR *, DWORD, DWORD);
K HANDLE __stdcall OpenWaitableTimerW(DWORD, int, const WCHAR *);
K int __stdcall SetWaitableTimer(HANDLE, const i64 *, long, void (*)(void *, DWORD, DWORD), void *, int);
K int __stdcall CancelWaitableTimer(HANDLE);
K int __stdcall DuplicateHandle(HANDLE, HANDLE, HANDLE, HANDLE *, DWORD, int, DWORD);
K HANDLE __stdcall GetCurrentProcess(void);
K void __stdcall GetSystemTimeAsFileTime(u64 *);
K u64 __stdcall GetTickCount64(void);
K DWORD __stdcall SleepEx(DWORD, int);

enum { PAGE_READONLY = 2, PAGE_READWRITE = 4, FILE_MAP_COPY = 1, FILE_MAP_WRITE = 2, FILE_MAP_READ = 4,
       MEM_COMMIT = 0x1000, MEM_RESERVE = 0x2000, MEM_FREE = 0x10000, MEM_PRIVATE = 0x20000, MEM_MAPPED = 0x40000,
       MEM_IMAGE = 0x1000000, WAIT_TIMEOUT = 0x102, WAIT_IO_COMPLETION = 0xc0 };
#define INVALID ((HANDLE)(i64)-1)

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
static int same(const char *a, const char *b, unsigned n) { for (unsigned i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }
static void put(char *at, const char *s) { while (*s) *at++ = *s++; }
static u64 parse(const char *s) { u64 n = 0; while (*s >= '0' && *s <= '9') n = n * 10 + (u64)(*s++ - '0'); return n; }
static char *number(char *at, u64 n) {
    char digits[24];
    int i = 0;
    do { digits[i++] = (char)('0' + n % 10); n /= 10; } while (n);
    while (i) *at++ = digits[--i];
    *at = 0;
    return at;
}
static WCHAR *W(int slot, const char *s) {
    static WCHAR buf[4][256];
    int n = 0;
    for (; s[n]; n++) buf[slot][n] = (WCHAR)s[n];
    buf[slot][n] = 0;
    return buf[slot];
}
static const char *argument(void) {
    const char *s = GetCommandLineA();
    if (*s == '"') { s++; while (*s && *s != '"') s++; if (*s) s++; }
    else while (*s && *s != ' ') s++;
    while (*s == ' ') s++;
    return s;
}

/// A child: map a section, check what the parent left at its start, and leave "child" at 100.
static void child(const char *what) {
    HANDLE section = 0;
    if (what[0] == 'n') section = OpenFileMappingW(FILE_MAP_WRITE, 0, W(0, "atlas-section"));
    if (what[0] == 'i') section = (HANDLE)parse(what + 10);
    char *view = section ? MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0) : 0;
    if (!view || !same(view, "parent", 6)) ExitProcess(1);
    put(view + 100, "child");
    ExitProcess(0);
}

static WCHAR self[260], line[600];
static DWORD run_child(const char *rest, int inherit) {
    int n = 0;
    line[n++] = '"';
    for (int i = 0; self[i]; i++) line[n++] = self[i];
    line[n++] = '"';
    line[n++] = ' ';
    while (*rest) line[n++] = (WCHAR)*rest++;
    line[n] = 0;
    STARTUPINFOW si = {0};
    si.cb = sizeof si;
    PROCESS_INFORMATION pi;
    if (!CreateProcessW(0, line, 0, 0, inherit, 0, 0, 0, &si, &pi)) return 1000;
    DWORD code = 1001;
    WaitForSingleObject(pi.process, 10000);
    GetExitCodeProcess(pi.process, &code);
    CloseHandle(pi.process);
    CloseHandle(pi.thread);
    return code;
}

static volatile int apc_ran;
static void *apc_argument;
static void on_timer(void *argument, DWORD low, DWORD high) {
    apc_argument = argument;
    apc_ran = (low | high) != 0;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    const char *what = argument();
    if (*what) child(what);
#if defined(__aarch64__)
    say("sections and timers on atlas, arm64\n");
#elif defined(__i386__)
    say("sections and timers on atlas, x86 (WoW64)\n");
#else
    say("sections and timers on atlas, x86-64\n");
#endif
    GetModuleFileNameW(0, self, 260);
    MEMORY_BASIC_INFORMATION mbi;

    // The page file's: two views at once, and a copy.
    HANDLE section = CreateFileMappingW(INVALID, 0, PAGE_READWRITE, 0, 0x20000, 0);
    char *a = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0);
    char *b = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 0);
    check(section && a && b && a != b && a[0] == 0 && a[0x1ffff] == 0, "CreateFileMapping of the page file: two views, zeros");
    put(a, "shared");
    put(b + 0x10000, "both ways");
    check(same(b, "shared", 6) && same(a + 0x10000, "both ways", 9), "each view sees the other's writes");
    char *c = MapViewOfFile(section, FILE_MAP_COPY, 0, 0, 0x2000);
    put(c, "copied");
    check(c && same(a, "shared", 6) && same(c, "copied", 6), "FILE_MAP_COPY: its writes are its own");
    put(a + 0x1008, "later");
    check(same(c + 0x1008, "later", 5), "and sees the section's own writes on a page it hasn't written");
    char *high = MapViewOfFile(section, FILE_MAP_READ, 0, 0x10000, 0x10000);
    check(high && same(high, "both ways", 9), "a view from 64 KiB in");

    VirtualQuery(a + 5000, &mbi, sizeof mbi);
    check(mbi.base == a + 4096 && mbi.allocation_base == a && mbi.state == MEM_COMMIT && mbi.type == MEM_MAPPED && mbi.protect == PAGE_READWRITE && mbi.size == 0x20000 - 4096,
          "VirtualQuery of a view: MEM_MAPPED, its base, the rest of it");
    VirtualQuery(high, &mbi, sizeof mbi);
    check(mbi.protect == PAGE_READONLY && mbi.type == MEM_MAPPED, "a FILE_MAP_READ view: PAGE_READONLY");
    char *private = VirtualAlloc(0, 0x30000, MEM_RESERVE, PAGE_READWRITE);
    VirtualAlloc(private + 0x10000, 0x1000, MEM_COMMIT, PAGE_READWRITE);
    VirtualQuery(private + 0x10000, &mbi, sizeof mbi);
    int committed = mbi.state == MEM_COMMIT && mbi.type == MEM_PRIVATE && mbi.allocation_base == private && mbi.size == 0x1000;
    VirtualQuery(private + 0x20000, &mbi, sizeof mbi);
    check(committed && mbi.state == MEM_RESERVE && mbi.allocation_base == private, "VirtualQuery of private memory: committed, reserved, one allocation");
    VirtualQuery(GetModuleHandleW(0), &mbi, sizeof mbi);
    check(mbi.type == MEM_IMAGE && mbi.state == MEM_COMMIT, "the program's own image: MEM_IMAGE");
    check(UnmapViewOfFile(c) && VirtualQuery(c, &mbi, sizeof mbi) && mbi.state == MEM_FREE, "UnmapViewOfFile: free again");
    SetLastError(0);
    check(!UnmapViewOfFile(a + 4096) && GetLastError() == 487, "unmapping from inside a view: ERROR_INVALID_ADDRESS");
    SetLastError(0);
    check(!MapViewOfFile(section, FILE_MAP_WRITE, 0, 0x1000, 0) && GetLastError() == 1132, "an offset off 64 KiB: ERROR_MAPPED_ALIGNMENT");
    HANDLE readonly = CreateFileMappingW(INVALID, 0, PAGE_READONLY, 0, 0x1000, 0);
    SetLastError(0);
    check(readonly && !MapViewOfFile(readonly, FILE_MAP_WRITE, 0, 0, 0) && GetLastError() == 5, "FILE_MAP_WRITE of a PAGE_READONLY section: ERROR_ACCESS_DENIED");
    CloseHandle(readonly);

    // Shared with other processes: by name, and inherited.
    HANDLE named = CreateFileMappingW(INVALID, 0, PAGE_READWRITE, 0, 0x1000, W(0, "atlas-section"));
    char *n = MapViewOfFile(named, FILE_MAP_WRITE, 0, 0, 0);
    put(n, "parent");
    check(run_child("named", 0) == 0 && same(n + 100, "child", 5), "a named section, written by another process");
    SetLastError(0);
    HANDLE again = CreateFileMappingW(INVALID, 0, PAGE_READWRITE, 0, 0x1000, W(0, "atlas-section"));
    char *n2 = MapViewOfFile(again, FILE_MAP_READ, 0, 0, 0);
    check(again && GetLastError() == 183 && n2 && same(n2 + 100, "child", 5), "its name again: the same section, ERROR_ALREADY_EXISTS");
    SECURITY_ATTRIBUTES inheritable = {sizeof inheritable, 0, 1};
    HANDLE given = CreateFileMappingW(INVALID, &inheritable, PAGE_READWRITE, 0, 0x1000, 0);
    char *g = MapViewOfFile(given, FILE_MAP_WRITE, 0, 0, 0);
    put(g, "parent");
    char text[64] = "inherited ";
    number(text + 10, (u64)given);
    check(run_child(text, 1) == 0 && same(g + 100, "child", 5), "an inherited section, by its handle's value");
    SetLastError(0);
    check(!OpenFileMappingW(FILE_MAP_READ, 0, W(0, "atlas-none")) && GetLastError() == 2, "OpenFileMapping of no such name: ERROR_FILE_NOT_FOUND");

    // A file mapping.
    WCHAR *path = W(1, "C:\\Windows\\Temp\\atlas-section.bin");
    HANDLE file = CreateFileW(path, 0xc0000000, 0, 0, 2, 0x80, 0);
    DWORD done;
    WriteFile(file, "hello, file mapping", 19, &done, 0);
    HANDLE mapping = CreateFileMappingW(file, 0, PAGE_READWRITE, 0, 0, 0);
    char *f = MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, 0);
    check(mapping && f && same(f, "hello, file mapping", 19) && f[19] == 0, "a file mapping: the file's bytes, then zeros");
    put(f, "HELLO");
    FlushViewOfFile(f, 0);
    char buf[32] = {0};
    SetFilePointer(file, 0, 0, 0);
    ReadFile(file, buf, 19, &done, 0);
    check(same(buf, "HELLO, file mapping", 19), "FlushViewOfFile: written to the file");
    put(f + 7, "FILE");
    UnmapViewOfFile(f);
    SetFilePointer(file, 0, 0, 0);
    ReadFile(file, buf, 19, &done, 0);
    check(same(buf, "HELLO, FILE mapping", 19), "UnmapViewOfFile: written to the file too");
    CloseHandle(mapping);
    HANDLE bigger = CreateFileMappingW(file, 0, PAGE_READWRITE, 0, 8192, 0);
    check(bigger && GetFileSize(file, 0) == 8192, "a mapping bigger than its file: the file grows");
    CloseHandle(bigger);
    CloseHandle(file);
    file = CreateFileW(W(2, "C:\\Windows\\Temp\\atlas-empty.bin"), 0xc0000000, 0, 0, 2, 0x80, 0);
    SetLastError(0);
    check(!CreateFileMappingW(file, 0, PAGE_READWRITE, 0, 0, 0) && GetLastError() == 1006, "an empty file, no size: ERROR_FILE_INVALID");
    CloseHandle(file);
    file = CreateFileW(path, 0x80000000, 1, 0, 3, 0x80, 0);
    SetLastError(0);
    check(!CreateFileMappingW(file, 0, PAGE_READWRITE, 0, 0, 0) && GetLastError() == 5, "PAGE_READWRITE of a file opened to read: ERROR_ACCESS_DENIED");
    HANDLE ro = CreateFileMappingW(file, 0, PAGE_READONLY, 0, 0, 0);
    char *r = MapViewOfFile(ro, FILE_MAP_READ, 0, 0, 0);
    check(r && same(r, "HELLO, FILE mapping", 19), "PAGE_READONLY of it: the bytes");
    UnmapViewOfFile(r);
    CloseHandle(ro);
    CloseHandle(file);

    // Waitable timers.
    i64 due = -500000; // 50 ms from now
    HANDLE timer = CreateWaitableTimerW(0, 1, 0);
    u64 start = GetTickCount64();
    check(timer && SetWaitableTimer(timer, &due, 0, 0, 0, 0) && WaitForSingleObject(timer, 0) == WAIT_TIMEOUT, "a notification timer, set: not yet");
    check(WaitForSingleObject(timer, 5000) == 0 && GetTickCount64() - start >= 40, "it goes off after 50 ms");
    check(WaitForSingleObject(timer, 0) == 0, "and stays set");
    HANDLE sync = CreateWaitableTimerExW(0, 0, 0, 0x1f0003);
    SetWaitableTimer(sync, &due, 0, 0, 0, 0);
    check(WaitForSingleObject(sync, 5000) == 0 && WaitForSingleObject(sync, 0) == WAIT_TIMEOUT, "a synchronization timer lets one wait through");
    due = -100000;
    SetWaitableTimer(sync, &due, 20, 0, 0, 0);
    int periodic = 1;
    for (int i = 0; i < 3; i++) periodic = periodic && WaitForSingleObject(sync, 5000) == 0;
    check(periodic, "periodic, every 20 ms: three times");
    CancelWaitableTimer(sync);
    WaitForSingleObject(sync, 0);
    check(WaitForSingleObject(sync, 150) == WAIT_TIMEOUT, "cancelled: no more");
    u64 now;
    GetSystemTimeAsFileTime(&now);
    due = (i64)now + 300000;
    SetWaitableTimer(timer, &due, 0, 0, 0, 0);
    check(WaitForSingleObject(timer, 0) == WAIT_TIMEOUT && WaitForSingleObject(timer, 5000) == 0, "an absolute time, 30 ms on");
    due = -200000;
    SetWaitableTimer(timer, &due, 0, on_timer, (void *)0x5eed, 0);
    DWORD slept = SleepEx(5000, 1);
    check(slept == WAIT_IO_COMPLETION && apc_ran && apc_argument == (void *)0x5eed, "its APC routine, with its argument and the time, in an alertable wait");
    HANDLE named_timer = CreateWaitableTimerW(0, 1, W(0, "atlas-timer"));
    HANDLE opened = OpenWaitableTimerW(0x1f0003, 0, W(0, "atlas-timer"));
    due = -100000;
    SetWaitableTimer(opened, &due, 0, 0, 0, 0);
    check(opened && WaitForSingleObject(named_timer, 5000) == 0, "OpenWaitableTimer by name: the same timer");

    DeleteFileW(path);
    DeleteFileW(W(2, "C:\\Windows\\Temp\\atlas-empty.bin"));
    // ---- A read-only handle, as a browser engine makes one for shared memory ----
    HANDLE writable = CreateFileMappingW((HANDLE)-1, 0, 0x04, 0, 0x10000, 0); // PAGE_READWRITE
    HANDLE ro_dup = 0, ro_again = 0, ro_same = 0;
    check(writable && DuplicateHandle(GetCurrentProcess(), writable, GetCurrentProcess(), &ro_dup, 0x4, 0, 0), "a FILE_MAP_READ duplicate");
    SetLastError(0);
    check(MapViewOfFile(ro_dup, 0x2, 0, 0, 0) == 0 && GetLastError() == 5, "it can't map a view for writing: ERROR_ACCESS_DENIED");
    void *seen = MapViewOfFile(ro_dup, 0x4, 0, 0, 0);
    check(seen != 0, "it maps one for reading");
    SetLastError(0);
    check(!DuplicateHandle(GetCurrentProcess(), ro_dup, GetCurrentProcess(), &ro_again, 0x2, 0, 0) && GetLastError() == 5,
          "nor can it be duplicated with FILE_MAP_WRITE");
    check(DuplicateHandle(GetCurrentProcess(), ro_dup, GetCurrentProcess(), &ro_same, 0, 0, 2) && MapViewOfFile(ro_same, 0x2, 0, 0, 0) == 0,
          "DUPLICATE_SAME_ACCESS keeps it read-only");
    void *written = MapViewOfFile(writable, 0x2, 0, 0, 0);
    check(written != 0, "the writable handle still maps for writing");
    if (written && seen) {
        *(volatile DWORD *)written = 0x5eed;
        check(*(volatile DWORD *)seen == 0x5eed, "and the read-only view sees what it writes");
    }
    CloseHandle(ro_same);
    CloseHandle(ro_dup);
    CloseHandle(writable);

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
