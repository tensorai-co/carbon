// ntdll's loader and structured exception handling, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: an app's own DLL, imported and relocated off a base the program has; its DllMain; a forwarder chain
// (mylib!heap_alloc → kernel32!HeapAlloc → ntdll!RtlAllocateHeap); LoadLibraryA and GetProcAddress by name and by
// ordinal; ERROR_MOD_NOT_FOUND and ERROR_PROC_NOT_FOUND; GetModuleFileNameW; static TLS and a TLS callback;
// __try/__except catching an access violation, from three calls down too; RaiseException with a filter; __finally
// during an unwind and on a normal exit; EXCEPTION_CONTINUE_EXECUTION; integer divide by zero (x86-64)

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef unsigned short WCHAR;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) HANDLE __stdcall LoadLibraryA(const char *);
__declspec(dllimport) void *__stdcall GetProcAddress(HANDLE, const char *);
__declspec(dllimport) HANDLE __stdcall GetModuleHandleA(const char *);
__declspec(dllimport) DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) void __stdcall RaiseException(DWORD, DWORD, DWORD, const u64 *);
__declspec(dllimport) HANDLE __stdcall GetProcessHeap(void);

__declspec(dllimport) int add(int, int);
__declspec(dllimport) const char *word(int);
__declspec(dllimport) int attached;
__declspec(dllimport) void *heap_alloc(HANDLE, DWORD, u64);

// Static TLS, as the C runtime's tlssup sets it up: a template between .tls and .tls$ZZZ, and callbacks in .CRT$XL*.
typedef void (__stdcall *TLS_CALLBACK)(void *, DWORD, void *);
unsigned _tls_index;
__attribute__((section(".tls"), used)) char _tls_start = 0;
__attribute__((section(".tls$ZZZ"), used)) char _tls_end = 0;
static int callback_ran;
static void __stdcall on_tls(void *m, DWORD reason, void *r) { (void)m; (void)r; if (reason == 1) callback_ran = 1; }
__attribute__((section(".CRT$XLA"), used)) TLS_CALLBACK __xl_a = 0;
__attribute__((section(".CRT$XLB"), used)) TLS_CALLBACK __xl_b = on_tls;
__attribute__((section(".CRT$XLZ"), used)) TLS_CALLBACK __xl_z = 0;
struct { u64 start, end, index, callbacks; DWORD zero_fill, flags; } const _tls_used = {
    (u64)&_tls_start, (u64)&_tls_end, (u64)&_tls_index, (u64)(&__xl_a + 1), 0, 0};
__declspec(thread) int tls_counter = 41;

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

__declspec(noinline) static void poke(volatile int *p) { *p = 1; }
__declspec(noinline) static int depth1(volatile int *p) { volatile int pad[16]; pad[0] = 1; poke(p); return pad[0]; }
__declspec(noinline) static int depth2(volatile int *p) { volatile int pad[32]; pad[3] = depth1(p); return pad[3] + 1; }
__declspec(noinline) static int depth3(volatile int *p) { int v = depth2(p); return v * 2; }
#if defined(__x86_64__)
// In assembly: dividing by zero is undefined in C, and the compiler would drop it.
__declspec(noinline) static int divide(int a, int b) { int rem; __asm__ volatile("cltd; idivl %2" : "+a"(a), "=&d"(rem) : "r"(b)); return a + rem; }
#endif
#define GetExceptionCode _exception_code
static DWORD seen;
static int pick(DWORD code) { seen = code; return code == 0xE0000001 ? 1 : 0; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("the loader and SEH, arm64\n");
#else
    say("the loader and SEH, x86-64\n");
#endif
    HANDLE self = GetModuleHandleA(0), lib = GetModuleHandleA("MyLib.DLL");
    check(add(2, 3) == 5 && same(word(1), "pointers"), "a DLL of the program's own, imported, with relocations");
    check(lib != 0 && lib != self, "it was moved off the base the program has");
    check(attached == 1, "its DllMain ran, once");
    check(heap_alloc(GetProcessHeap(), 0, 32) != 0, "a forwarder to kernel32, which forwards to ntdll");

    HANDLE two = LoadLibraryA("mylib2");
    int (*mul)(int, int) = (int (*)(int, int))GetProcAddress(two, "mul");
    int (*by_ordinal)(int, int) = (int (*)(int, int))GetProcAddress(two, (const char *)5);
    check(two && mul && mul(6, 7) == 42 && by_ordinal == mul, "LoadLibraryA, and GetProcAddress by name and by ordinal");
    check(LoadLibraryA("mylib2.dll") == two, "loading it again gives the same module");
    check(GetProcAddress(two, "nope") == 0 && GetLastError() == 127, "a missing export is ERROR_PROC_NOT_FOUND");
    check(LoadLibraryA("missing.dll") == 0 && GetLastError() == 126, "a missing DLL is ERROR_MOD_NOT_FOUND");
    WCHAR path[260];
    DWORD n = GetModuleFileNameW(two, path, 260);
    check(n > 10 && path[n - 1] == 'l' && path[n - 10] == 'm', "GetModuleFileNameW");

    check(callback_ran && tls_counter == 41, "static TLS: its template, and its callback ran first");
    tls_counter = tls_counter + 1;
    check(tls_counter == 42, "and it's this thread's to change");

    DWORD code = 0;
    __try { poke(0); } __except (1) { code = GetExceptionCode(); }
    check(code == 0xC0000005, "__try/__except catches an access violation");

    code = 0;
    __try { depth3(0); } __except (1) { code = GetExceptionCode(); }
    check(code == 0xC0000005, "from three calls down, unwinding through their frames");

    code = 0;
    __try { RaiseException(0xE0000001, 0, 0, 0); } __except (pick(GetExceptionCode())) { code = 1; }
    check(code == 1 && seen == 0xE0000001, "RaiseException, and a filter that looks at the code");

    int finally_ran = 0, order = -1;
    __try {
        __try { poke(0); } __finally { finally_ran = 1; }
    } __except (1) { order = finally_ran; }
    check(order == 1, "a __finally runs as the exception unwinds past it, before the __except");

    int normal = 0;
    __try { normal = 1; } __finally { normal = normal + 1; }
    check(normal == 2, "a __finally runs when its __try just ends");

    int resumed = 0;
    __try { RaiseException(0xE0000002, 0, 0, 0); resumed = 1; } __except (-1) { resumed = -1; }
    check(resumed == 1, "EXCEPTION_CONTINUE_EXECUTION: RaiseException returns");

#if defined(__x86_64__)
    code = 0;
    __try { divide(1, 0); } __except (1) { code = GetExceptionCode(); }
    check(code == 0xC0000094, "integer divide by zero");
#endif

    ExitProcess(failures);
}
