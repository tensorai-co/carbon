// A freestanding Windows test program for atlas: no C runtime, only kernel32 and user32, for ARM64 and x86-64.
// tests/programs/build.sh builds it.
//
// It checks: console handles, WriteConsoleW, the command line, VirtualAlloc committed on touch, an upcall into the
// program (EnumSystemLocalesW, twice), MessageBoxW, and the guest exports: the heap, the last error and critical
// sections, running inside the program in the embedded ntdll

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef BOOL (__stdcall *LOCALE_ENUMPROCW)(WCHAR *);

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) BOOL __stdcall WriteConsoleW(HANDLE, const WCHAR *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) WCHAR *__stdcall GetCommandLineW(void);
__declspec(dllimport) void *__stdcall VirtualAlloc(void *, unsigned long long, DWORD, DWORD);
__declspec(dllimport) BOOL __stdcall EnumSystemLocalesW(LOCALE_ENUMPROCW, DWORD);
__declspec(dllimport) unsigned long long __stdcall GetTickCount64(void);
__declspec(dllimport) int __stdcall MessageBoxW(void *, const WCHAR *, const WCHAR *, unsigned);
// Guest exports: these run in the program, in atlas-guest's ntdll.
__declspec(dllimport) HANDLE __stdcall GetProcessHeap(void);
__declspec(dllimport) void *__stdcall HeapAlloc(HANDLE, DWORD, unsigned long long);
__declspec(dllimport) BOOL __stdcall HeapFree(HANDLE, DWORD, void *);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) void __stdcall SetLastError(DWORD);
typedef struct { void *debug; int lock_count, recursion; HANDLE owner, semaphore; unsigned long long spin; } CRITICAL_SECTION;
__declspec(dllimport) void __stdcall InitializeCriticalSection(CRITICAL_SECTION *);
__declspec(dllimport) void __stdcall EnterCriticalSection(CRITICAL_SECTION *);
__declspec(dllimport) void __stdcall LeaveCriticalSection(CRITICAL_SECTION *);

static HANDLE out;
static int failures;

static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { DWORD n; WriteFile(out, s, len(s), &n, 0); }
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    // A branch, not arithmetic on the comparison: exec's x86 translator doesn't do `adc` and `sbb` yet.
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

static int locales;
static WCHAR first[16];
static BOOL __stdcall on_locale(WCHAR *name) {
    if (locales == 0) for (int i = 0; i < 15 && name[i]; i++) first[i] = name[i];
    locales = locales + 1;
    return locales < 2; // stop after the second
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("hello from arm64 Windows code on atlas\n");
#else
    say("hello from x86-64 Windows code on atlas\n");
#endif
    check(out != 0 && out != (HANDLE)-1, "GetStdHandle");

    DWORD n = 0;
    static const WCHAR wide[] = {'w', 'i', 'd', 'e', ' ', 0x263a, '\n', 0};
    check(WriteConsoleW(out, wide, 7, &n, 0) && n == 7, "WriteConsoleW, as UTF-16");

    WCHAR *line = GetCommandLineW();
    check(line && line[0] == 'C' && line[1] == ':' && line[2] == '\\', "GetCommandLineW");

    volatile unsigned long long *p = VirtualAlloc(0, 1 << 20, 0x3000, 4); // MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE
    int memory_ok = p != 0;
    if (memory_ok) { for (int i = 0; i < (1 << 20) / 8; i += 512) p[i] = i; for (int i = 0; i < (1 << 20) / 8; i += 512) memory_ok &= p[i] == (unsigned long long)i; }
    check(memory_ok, "VirtualAlloc 1 MiB, committed page by page as it's touched");

    check(EnumSystemLocalesW(on_locale, 2) && locales == 2 && first[0] == '0' && first[4] == '0' && first[5] == '4', "EnumSystemLocalesW: an upcall into the program, twice");

    check(GetTickCount64() > 0, "GetTickCount64");

    static const WCHAR text[] = {'f', 'r', 'o', 'm', ' ', 'u', 's', 'e', 'r', '3', '2', 0};
    static const WCHAR title[] = {'a', 't', 'l', 'a', 's', 0};
    check(MessageBoxW(0, text, title, 0) == 1, "MessageBoxW");

    HANDLE heap = GetProcessHeap();
    char *a = HeapAlloc(heap, 8, 100), *b = HeapAlloc(heap, 0, 5000);
    int heap_ok = heap && a && b && a != b && a[0] == 0 && a[99] == 0;
    if (heap_ok) { a[0] = 'x'; b[4999] = 'y'; heap_ok = HeapFree(heap, 0, a) != 0; }
    char *c = HeapAlloc(heap, 0, 90);
    check(heap_ok && c == a, "HeapAlloc and HeapFree, in the program (a freed block is reused)");

    SetLastError(1234);
    check(GetLastError() == 1234, "SetLastError and GetLastError, in the program's TEB");

    static CRITICAL_SECTION section;
    InitializeCriticalSection(&section);
    EnterCriticalSection(&section);
    EnterCriticalSection(&section);
    int owned = section.recursion == 2;
    LeaveCriticalSection(&section);
    LeaveCriticalSection(&section);
    check(owned && section.recursion == 0 && section.lock_count == -1, "critical sections, entered twice and left");

    ExitProcess(failures);
}
