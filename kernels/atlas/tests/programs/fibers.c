// Fibers and FLS, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: a thread made a fiber, and its data; fibers switched to and back, each on its own stack, with locals
// kept across the switches; an exception caught on a fiber; a stack overflow on a fiber, caught; FLS that's each
// fiber's own, its callbacks as a fiber's deleted, as FlsFree frees a slot, and as a thread ends; a fiber whose
// routine returns, ending its thread; and a fiber back to a thread.

typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) void __stdcall RaiseException(DWORD, DWORD, DWORD, const ULONG_PTR *);
__declspec(dllimport) void *__stdcall CreateFiber(SIZE_T, void (__stdcall *)(void *), void *);
__declspec(dllimport) void __stdcall DeleteFiber(void *);
__declspec(dllimport) void __stdcall SwitchToFiber(void *);
__declspec(dllimport) void *__stdcall ConvertThreadToFiber(void *);
__declspec(dllimport) int __stdcall ConvertFiberToThread(void);
__declspec(dllimport) int __stdcall IsThreadAFiber(void);
__declspec(dllimport) DWORD __stdcall FlsAlloc(void (__stdcall *)(void *));
__declspec(dllimport) int __stdcall FlsFree(DWORD);
__declspec(dllimport) void *__stdcall FlsGetValue(DWORD);
__declspec(dllimport) int __stdcall FlsSetValue(DWORD, void *);
__declspec(dllimport) HANDLE __stdcall CreateThread(void *, SIZE_T, DWORD (__stdcall *)(void *), void *, DWORD, DWORD *);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) int __stdcall GetExitCodeThread(HANDLE, DWORD *);

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

// GetCurrentFiber and GetFiberData, as winnt.h has them: the TEB's FiberData, and the first word of what it points at.
static void *current_fiber(void) {
    char *teb;
#if defined(__aarch64__)
    __asm__ volatile("mov %0, x18" : "=r"(teb));
#else
    __asm__ volatile("mov %%gs:0x30, %0" : "=r"(teb));
#endif
    return *(void **)(teb + 0x20);
}
static void *fiber_data(void) { return *(void **)current_fiber(); }

static void *main_fiber, *worker;
static char trail[32];
static int trail_at;
static void mark(char c) { if (trail_at < 31) trail[trail_at++] = c; }
static int same(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }

static DWORD slot;
static int freed, freed_sum;
static void __stdcall on_free(void *v) { freed = freed + 1; freed_sum = freed_sum + (int)(u64)v; }

static void *seen_data, *worker_local;
static void *worker_fls_before;
static DWORD caught;
__declspec(noinline) static void poke(volatile int *p) { *p = 1; }
__declspec(noinline) static int recurse(int n) {
    volatile char frame[1500];
    frame[0] = (char)n;
    frame[1499] = (char)n;
    return recurse(n + 1) + frame[0] + frame[1499];
}

static void __stdcall work(void *p) {
    int local = 0;
    seen_data = fiber_data();
    worker_local = &local;
    (void)p;
    for (int round = 0; round < 3; round++) {
        mark('w');
        local = local + 1;
        SwitchToFiber(main_fiber);
    }
    worker_fls_before = FlsGetValue(slot);
    FlsSetValue(slot, (void *)20);
    __try { poke(0); } __except (1) { caught = GetExceptionCode(); }
    DWORD overflow = 0;
    __try { recurse(0); } __except (1) { overflow = GetExceptionCode(); }
    if (overflow == 0xC00000FD && local == 3) mark('o');
    SwitchToFiber(main_fiber);
    // Never switched to again: it's deleted.
    mark('!');
    SwitchToFiber(main_fiber);
}

static DWORD __stdcall thread_fls(void *p) {
    (void)p;
    FlsSetValue(slot, (void *)300);
    return 0;
}

static void __stdcall returns(void *p) { (void)p; }
static DWORD __stdcall fiber_thread(void *p) {
    (void)p;
    ConvertThreadToFiber(0);
    SwitchToFiber(CreateFiber(0, returns, 0));
    // A fiber that returns ends the thread: this never runs.
    return 99;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("fibers, arm64\n");
#elif defined(__i386__)
    say("fibers, x86 (WoW64)\n");
#else
    say("fibers, x86-64\n");
#endif
    check(!IsThreadAFiber() && current_fiber() == 0, "a thread isn't a fiber at first");
    main_fiber = ConvertThreadToFiber((void *)0x1234);
    check(main_fiber && IsThreadAFiber() && current_fiber() == main_fiber && fiber_data() == (void *)0x1234, "ConvertThreadToFiber, and its data");
    check(ConvertThreadToFiber(0) == 0 && GetLastError() == 1280, "and again: ERROR_ALREADY_FIBER");

    slot = FlsAlloc(on_free);
    check(slot != 0xffffffff && FlsSetValue(slot, (void *)1) && FlsGetValue(slot) == (void *)1, "FlsAlloc, FlsSetValue and FlsGetValue");

    worker = CreateFiber(0, work, (void *)0x5678);
    check(worker != 0, "CreateFiber");
    int here = 0;
    for (int round = 0; round < 3; round++) {
        mark('m');
        SwitchToFiber(worker);
        here = here + 10;
    }
    check(same(trail, "mwmwmw") && here == 30 && current_fiber() == main_fiber, "switching back and forth, with locals kept on both sides");
    check(seen_data == (void *)0x5678, "a fiber's data is its own");
    u64 apart = (u64)worker_local > (u64)&here ? (u64)worker_local - (u64)&here : (u64)&here - (u64)worker_local;
    check(apart > 0x10000, "and so is its stack");
    SwitchToFiber(worker);
    check(caught == 0xC0000005, "an exception, caught on a fiber");
    check(trail[6] == 'o', "a stack overflow on a fiber, caught, with the fiber carrying on");
    check(worker_fls_before == 0 && FlsGetValue(slot) == (void *)1, "FLS is each fiber's own");

    DeleteFiber(worker);
    check(freed == 1 && freed_sum == 20 && trail[7] == 0, "DeleteFiber: its FLS's callback, and it never runs again");

    HANDLE t = CreateThread(0, 0, thread_fls, 0, 0, 0);
    WaitForSingleObject(t, 0xffffffff);
    check(freed == 2 && freed_sum == 320, "a thread ending: its FLS's callback");

    check(FlsFree(slot) && freed == 3 && freed_sum == 321, "FlsFree: the callback for what's left, this fiber's");
    check(!FlsSetValue(slot, (void *)5) && GetLastError() == 87 && !FlsFree(slot), "a freed slot can't be used, or freed again");

    t = CreateThread(0, 0, fiber_thread, 0, 0, 0);
    DWORD code = 1;
    WaitForSingleObject(t, 0xffffffff);
    GetExitCodeThread(t, &code);
    check(code == 0, "a fiber whose routine returns ends its thread");

    check(ConvertFiberToThread() && !IsThreadAFiber() && current_fiber() == 0, "ConvertFiberToThread");
    check(!ConvertFiberToThread() && GetLastError() == 1281, "and again: ERROR_ALREADY_THREAD");

    ExitProcess(failures);
}
