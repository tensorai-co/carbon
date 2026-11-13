// Exceptions past the frame-based ones, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: vectored exception handlers, asked before any frame's, in the order `first` says, one carrying on, one
// passing it to the frames, one committing the memory an access violation touched so the program carries on; vectored
// continue handlers; removing both; guard pages (PAGE_GUARD) raising STATUS_GUARD_PAGE_VIOLATION once, as VirtualQuery
// and VirtualProtect see them; and a stack overflow caught with __try/__except, on the first thread and on another.

typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) void __stdcall RaiseException(DWORD, DWORD, DWORD, const ULONG_PTR *);
__declspec(dllimport) void *__stdcall AddVectoredExceptionHandler(DWORD, void *);
__declspec(dllimport) DWORD __stdcall RemoveVectoredExceptionHandler(void *);
__declspec(dllimport) void *__stdcall AddVectoredContinueHandler(DWORD, void *);
__declspec(dllimport) DWORD __stdcall RemoveVectoredContinueHandler(void *);
__declspec(dllimport) void *__stdcall VirtualAlloc(void *, SIZE_T, DWORD, DWORD);
__declspec(dllimport) int __stdcall VirtualProtect(void *, SIZE_T, DWORD, DWORD *);
__declspec(dllimport) SIZE_T __stdcall VirtualQuery(const void *, void *, SIZE_T);
__declspec(dllimport) HANDLE __stdcall CreateThread(void *, SIZE_T, DWORD (__stdcall *)(void *), void *, DWORD, DWORD *);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) int __stdcall GetExitCodeThread(HANDLE, DWORD *);

typedef struct { DWORD code, flags; u64 record, address; DWORD count, pad; u64 information[15]; } RECORD;
typedef struct { RECORD *record; void *context; } POINTERS;
typedef struct { u64 base, allocation_base; DWORD allocation_protect, pad; u64 size; DWORD state, protect, type, pad2; } MBI;

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
#define GetExceptionCode _exception_code
// clang's __try covers the calls in it, not instructions that fault in it: each touch is a call.
__declspec(noinline) static char peek(volatile char *p) { return *p; }
__declspec(noinline) static void poke(volatile char *p, char v) { *p = v; }

// The order handlers ran in, as digits.
static char trail[16];
static int trail_at;
static void mark(char c) { if (trail_at < 15) trail[trail_at++] = c; }
static int same(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }

static long __stdcall carry_on(POINTERS *p) { mark('1'); return p->record->code == 0xE0000010 ? -1 : 0; }
static long __stdcall look_first(POINTERS *p) { (void)p; mark('0'); return 0; }
static long __stdcall continued(POINTERS *p) { (void)p; mark('c'); return 0; }
static long __stdcall pass(POINTERS *p) { mark(p->record->code == 0xE0000011 ? 'p' : '?'); return 0; }

// Commits the page an access violation touched, if it's in `lazy`, and has the program try again.
static char *lazy;
static int committed;
static long __stdcall commit(POINTERS *p) {
    char *at = (char *)p->record->information[1];
    if (p->record->code != 0xC0000005 || at < lazy || at >= lazy + 0x10000) return 0;
    committed = committed + 1;
    VirtualAlloc((void *)((u64)at & ~0xfffull), 0x1000, 0x1000, 4);
    return -1;
}

// Deep enough to use every page of a stack: frames under a page, so no __chkstk is needed.
__declspec(noinline) static int recurse(int n) {
    volatile char frame[1500];
    frame[0] = (char)n;
    frame[1499] = (char)n;
    return recurse(n + 1) + frame[0] + frame[1499];
}
static DWORD overflowed(void) {
    DWORD code = 0;
    __try { recurse(0); } __except (1) { code = GetExceptionCode(); }
    return code;
}
static DWORD __stdcall overflow_thread(void *p) { (void)p; return overflowed(); }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("exceptions, arm64\n");
#else
    say("exceptions, x86-64\n");
#endif
    void *last = AddVectoredExceptionHandler(0, (void *)carry_on);
    void *first = AddVectoredExceptionHandler(1, (void *)look_first);
    void *cont = AddVectoredContinueHandler(0, (void *)continued);
    check(last && first && cont && last != first, "AddVectoredExceptionHandler and AddVectoredContinueHandler");
    RaiseException(0xE0000010, 0, 0, 0);
    check(same(trail, "01c"), "vectored handlers, first first; one carries on, and the continue handler hears it");

    trail_at = 0;
    for (int i = 0; i < 16; i++) trail[i] = 0;
    void *passing = AddVectoredExceptionHandler(0, (void *)pass);
    int caught = 0;
    __try { RaiseException(0xE0000011, 0, 0, 0); } __except (mark('f'), 1) { caught = 1; }
    check(caught && same(trail, "01pf"), "a vectored handler that passes: the frame's __except gets it, after every vectored one");

    check(RemoveVectoredExceptionHandler(first) && RemoveVectoredExceptionHandler(last) && RemoveVectoredExceptionHandler(passing) &&
              RemoveVectoredContinueHandler(cont),
          "removing them");
    check(!RemoveVectoredExceptionHandler(first) && !RemoveVectoredContinueHandler(last), "removing one twice, or from the wrong list, fails");
    trail_at = 0;
    for (int i = 0; i < 16; i++) trail[i] = 0;
    caught = 0;
    __try { RaiseException(0xE0000010, 0, 0, 0); } __except (1) { caught = 1; }
    check(caught && trail[0] == 0, "removed, they're asked no more");

    lazy = VirtualAlloc(0, 0x10000, 0x2000, 4);
    void *committer = AddVectoredExceptionHandler(1, (void *)commit);
    volatile char *touch = lazy + 0x3000;
    *touch = 42;
    lazy[0x8000] = 7;
    check(lazy && committed == 2 && *touch == 42 && lazy[0x8000] == 7, "a vectored handler commits reserved memory as it's touched");
    RemoveVectoredExceptionHandler(committer);

    volatile char *guarded = VirtualAlloc(0, 0x2000, 0x3000, 4 | 0x100);
    MBI mbi;
    check(guarded && VirtualQuery((void *)guarded, &mbi, sizeof mbi) && mbi.protect == 0x104 && mbi.state == 0x1000, "VirtualAlloc with PAGE_GUARD, as VirtualQuery sees it");
    DWORD code = 0;
    __try { peek(guarded + 10); } __except (1) { code = GetExceptionCode(); }
    check(code == 0x80000001, "its first touch: STATUS_GUARD_PAGE_VIOLATION");
    code = 0;
    __try { poke(guarded + 10, 5); } __except (1) { code = GetExceptionCode(); }
    check(code == 0 && guarded[10] == 5, "then the guard's gone, for that page");
    VirtualQuery((void *)guarded, &mbi, sizeof mbi);
    MBI next;
    VirtualQuery((void *)(guarded + 0x1000), &next, sizeof next);
    check(mbi.protect == 4 && mbi.size == 0x1000 && next.protect == 0x104, "only that page: the other is still guarded");
    DWORD old = 0;
    check(VirtualProtect((void *)guarded, 0x1000, 4 | 0x100, &old) && old == 4, "VirtualProtect puts a guard back");
    code = 0;
    __try { poke(guarded, 1); } __except (1) { code = GetExceptionCode(); }
    check(code == 0x80000001 && VirtualProtect((void *)(guarded + 0x1000), 0x1000, 4, &old) && old == 0x104, "and it guards again, and says it was guarded");

    check(overflowed() == 0xC00000FD, "a stack overflow, caught with __try/__except");
    int after = 0;
    __try { after = 1; } __except (1) { after = -1; }
    check(after == 1, "and the program carries on");
    HANDLE t = CreateThread(0, 0, overflow_thread, 0, 0, 0);
    DWORD thread_code = 0;
    WaitForSingleObject(t, 0xffffffff);
    GetExitCodeThread(t, &thread_code);
    check(thread_code == 0xC00000FD, "on another thread's stack too");

    ExitProcess(failures);
}
