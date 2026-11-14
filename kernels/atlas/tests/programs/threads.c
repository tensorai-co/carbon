// Threads and synchronization on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: four threads and a contended critical section with wait-all; GetExitCodeThread; TlsAlloc slots and
// __declspec(thread), each thread's own; DLL_THREAD_ATTACH; auto- and manual-reset events; a wait that times out;
// semaphores and their maximum; a mutex owned, waited for, released, not owned, and abandoned; an SRW lock and a
// condition variable; WaitOnAddress; InitOnceExecuteOnce from four threads; CREATE_SUSPENDED and ResumeThread;
// QueueUserAPC and an alertable SleepEx; named events; TerminateThread; ExitThread; DuplicateHandle of a pseudo-handle;
// TLS's expansion slots past the first 64, each thread's own, and a freed slot cleared in every thread

// x86's thread-local variables are found through fs:[__tls_array], which the C runtime defines as 0x2c, the TEB's
// ThreadLocalStoragePointer; there's none here.
#if defined(__i386__)
__asm__(".globl __tls_array\n.set __tls_array, 0x2c");
#endif
typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef int BOOL;
typedef struct { void *debug; int lock_count, recursion; HANDLE owner, semaphore; u64 spin; } CRITICAL_SECTION;
typedef DWORD (__stdcall *THREAD_START)(void *);

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall CreateThread(void *, SIZE_T, THREAD_START, void *, DWORD, DWORD *);
K void __stdcall ExitThread(DWORD);
K BOOL __stdcall GetExitCodeThread(HANDLE, DWORD *);
K DWORD __stdcall GetThreadId(HANDLE);
K DWORD __stdcall GetCurrentThreadId(void);
K HANDLE __stdcall GetCurrentThread(void);
K HANDLE __stdcall GetCurrentProcess(void);
K DWORD __stdcall ResumeThread(HANDLE);
K BOOL __stdcall TerminateThread(HANDLE, DWORD);
K HANDLE __stdcall CreateEventA(void *, BOOL, BOOL, const char *);
K HANDLE __stdcall OpenEventW(DWORD, BOOL, const unsigned short *);
K BOOL __stdcall SetEvent(HANDLE);
K BOOL __stdcall ResetEvent(HANDLE);
K HANDLE __stdcall CreateMutexA(void *, BOOL, const char *);
K BOOL __stdcall ReleaseMutex(HANDLE);
K HANDLE __stdcall CreateSemaphoreA(void *, int, int, const char *);
K BOOL __stdcall ReleaseSemaphore(HANDLE, int, int *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K DWORD __stdcall WaitForMultipleObjects(DWORD, const HANDLE *, BOOL, DWORD);
K void __stdcall Sleep(DWORD);
K DWORD __stdcall SleepEx(DWORD, BOOL);
K DWORD __stdcall QueueUserAPC(void (__stdcall *)(ULONG_PTR), HANDLE, ULONG_PTR);
K BOOL __stdcall DuplicateHandle(HANDLE, HANDLE, HANDLE, HANDLE *, DWORD, BOOL, DWORD);
K BOOL __stdcall CloseHandle(HANDLE);
K DWORD __stdcall GetLastError(void);
K void __stdcall InitializeCriticalSection(CRITICAL_SECTION *);
K void __stdcall EnterCriticalSection(CRITICAL_SECTION *);
K void __stdcall LeaveCriticalSection(CRITICAL_SECTION *);
K DWORD __stdcall TlsAlloc(void);
K void *__stdcall TlsGetValue(DWORD);
K BOOL __stdcall TlsSetValue(DWORD, void *);
K BOOL __stdcall TlsFree(DWORD);
K void __stdcall InitializeSRWLock(void **);
K void __stdcall AcquireSRWLockExclusive(void **);
K void __stdcall ReleaseSRWLockExclusive(void **);
K void __stdcall InitializeConditionVariable(void **);
K BOOL __stdcall SleepConditionVariableSRW(void **, void **, DWORD, DWORD);
K void __stdcall WakeConditionVariable(void **);
K void __stdcall WakeAllConditionVariable(void **);
K BOOL __stdcall WaitOnAddress(volatile void *, void *, SIZE_T, DWORD);
K void __stdcall WakeByAddressSingle(void *);
K BOOL __stdcall InitOnceExecuteOnce(void **, BOOL (__stdcall *)(void **, void *, void **), void *, void **);
K int thread_attached;

#define INFINITE 0xffffffff
#define WAIT_TIMEOUT 0x102
#define WAIT_ABANDONED 0x80
#define WAIT_IO_COMPLETION 0xc0
#define CREATE_SUSPENDED 4

// Static TLS, as tlssup sets it up.
unsigned _tls_index;
__attribute__((section(".tls"), used)) char _tls_start = 0;
__attribute__((section(".tls$ZZZ"), used)) char _tls_end = 0;
struct { ULONG_PTR start, end, index, callbacks; DWORD zero_fill, flags; } const _tls_used = {(ULONG_PTR)&_tls_start, (ULONG_PTR)&_tls_end, (ULONG_PTR)&_tls_index, 0, 0, 0};
__declspec(thread) int per_thread = 41;

static HANDLE out;
static int failures;
static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { DWORD n; WriteFile(out, s, len(s), &n, 0); }
static DWORD expansion_slot;
static volatile int expansion_seen;
static DWORD __stdcall expansion_thread(void *unused) {
    (void)unused;
    int fresh = TlsGetValue(expansion_slot) == 0;
    TlsSetValue(expansion_slot, (void *)7);
    expansion_seen = fresh && TlsGetValue(expansion_slot) == (void *)7;
    return 0;
}

static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

// Four threads, each adding to a shared count under a critical section, and checking their own TLS.
static CRITICAL_SECTION section;
static volatile int count;
#define ROUNDS 20000
static DWORD slot;
static volatile int tls_ok = 1;
static DWORD __stdcall adder(void *arg) {
    u64 me = (u64)arg;
    TlsSetValue(slot, (void *)(me * 10));
    per_thread = per_thread + (int)me;
    for (int i = 0; i < ROUNDS; i++) {
        EnterCriticalSection(&section);
        count = count + 1;
        LeaveCriticalSection(&section);
    }
    if ((u64)TlsGetValue(slot) != me * 10 || per_thread != 41 + (int)me) tls_ok = 0;
    return 100 + (DWORD)me;
}

static HANDLE event, auto_event;
static volatile int woke;
static DWORD __stdcall event_waiter(void *arg) {
    (void)arg;
    WaitForSingleObject(auto_event, INFINITE);
    EnterCriticalSection(&section);
    woke = woke + 1;
    LeaveCriticalSection(&section);
    return 0;
}

static HANDLE mutex;
static volatile DWORD mutex_wait;
static DWORD __stdcall mutex_taker(void *arg) {
    mutex_wait = WaitForSingleObject(mutex, (DWORD)(u64)arg);
    return 0;
}
static DWORD __stdcall abandoner(void *arg) {
    (void)arg;
    WaitForSingleObject(mutex, INFINITE);
    return 7; // ends holding it
}

static void *srw, *cv;
static volatile int queue[100], queued, taken;
static DWORD __stdcall consumer(void *arg) {
    (void)arg;
    int sum = 0;
    while (taken < 100) {
        AcquireSRWLockExclusive(&srw);
        while (taken == queued) SleepConditionVariableSRW(&cv, &srw, INFINITE, 0);
        sum += queue[taken];
        taken = taken + 1;
        ReleaseSRWLockExclusive(&srw);
    }
    return (DWORD)sum;
}

static volatile DWORD flag;
static DWORD __stdcall setter(void *arg) {
    (void)arg;
    Sleep(20);
    flag = 1;
    WakeByAddressSingle((void *)&flag);
    return 0;
}

static void *once;
static volatile int once_ran;
static BOOL __stdcall initialize(void **o, void *p, void **c) { (void)o; (void)p; (void)c; Sleep(10); once_ran = once_ran + 1; return 1; }
static DWORD __stdcall once_user(void *arg) { (void)arg; InitOnceExecuteOnce(&once, initialize, 0, 0); return (DWORD)once_ran; }

static volatile int started;
static DWORD __stdcall marker(void *arg) { (void)arg; started = 1; return 0; }

static volatile ULONG_PTR apc_value;
static void __stdcall apc(ULONG_PTR value) { apc_value = value; }

static DWORD __stdcall spinner(void *arg) { (void)arg; for (;;) { __asm__ volatile("" ::: "memory"); } }
static DWORD __stdcall exiter(void *arg) { (void)arg; ExitThread(33); return 0; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("threads on atlas, arm64\n");
#elif defined(__i386__)
    say("threads on atlas, x86 (WoW64)\n");
#else
    say("threads on atlas, x86-64\n");
#endif
    InitializeCriticalSection(&section);
    slot = TlsAlloc();
    TlsSetValue(slot, (void *)1234);
    HANDLE threads[4];
    for (u64 i = 0; i < 4; i++) threads[i] = CreateThread(0, 0, adder, (void *)(i + 1), 0, 0);
    DWORD all = WaitForMultipleObjects(4, threads, 1, INFINITE);
    DWORD code = 0;
    GetExitCodeThread(threads[2], &code);
    check(all == 0 && count == 4 * ROUNDS, "four threads, a contended critical section, and wait-all");
    check(code == 103, "GetExitCodeThread");
    check(tls_ok && (u64)TlsGetValue(slot) == 1234 && per_thread == 41, "TlsAlloc slots and __declspec(thread), each thread's own");
    check(thread_attached == 4, "a DLL's DllMain heard of each thread (DLL_THREAD_ATTACH)");

    auto_event = CreateEventA(0, 0, 0, 0);
    HANDLE waiters[2] = {CreateThread(0, 0, event_waiter, 0, 0, 0), CreateThread(0, 0, event_waiter, 0, 0, 0)};
    Sleep(30);
    SetEvent(auto_event);
    Sleep(50);
    int after_one = woke;
    SetEvent(auto_event);
    WaitForMultipleObjects(2, waiters, 1, INFINITE);
    check(after_one == 1 && woke == 2, "an auto-reset event lets one waiter through each time");
    event = CreateEventA(0, 1, 0, 0);
    check(WaitForSingleObject(event, 30) == WAIT_TIMEOUT, "a wait that times out");
    SetEvent(event);
    check(WaitForSingleObject(event, 0) == 0 && WaitForSingleObject(event, 0) == 0, "a manual-reset event stays set");
    ResetEvent(event);
    check(WaitForSingleObject(event, 0) == WAIT_TIMEOUT, "until it's reset");

    HANDLE sem = CreateSemaphoreA(0, 0, 2, 0);
    int previous = -1;
    ReleaseSemaphore(sem, 2, &previous);
    DWORD s1 = WaitForSingleObject(sem, 0), s2 = WaitForSingleObject(sem, 0), s3 = WaitForSingleObject(sem, 10);
    check(previous == 0 && s1 == 0 && s2 == 0 && s3 == WAIT_TIMEOUT, "a semaphore counts");
    check(!ReleaseSemaphore(sem, 3, 0) && GetLastError() == 298, "past its maximum is ERROR_TOO_MANY_POSTS");

    mutex = CreateMutexA(0, 1, 0);
    HANDLE taker = CreateThread(0, 0, mutex_taker, (void *)30, 0, 0);
    WaitForSingleObject(taker, INFINITE);
    int timed_out = mutex_wait == WAIT_TIMEOUT;
    taker = CreateThread(0, 0, mutex_taker, (void *)INFINITE, 0, 0);
    Sleep(20);
    ReleaseMutex(mutex);
    WaitForSingleObject(taker, INFINITE);
    check(timed_out && mutex_wait == 0, "a mutex: owned, waited for, released");
    check(!ReleaseMutex(mutex) && GetLastError() == 288, "releasing one this thread doesn't own: ERROR_NOT_OWNER");
    mutex = CreateMutexA(0, 0, 0);
    HANDLE ab = CreateThread(0, 0, abandoner, 0, 0, 0);
    WaitForSingleObject(ab, INFINITE);
    check(WaitForSingleObject(mutex, 0) == WAIT_ABANDONED, "a mutex its owner ended holding: WAIT_ABANDONED");
    ReleaseMutex(mutex);

    InitializeSRWLock(&srw);
    InitializeConditionVariable(&cv);
    HANDLE c = CreateThread(0, 0, consumer, 0, 0, 0);
    int expected = 0;
    for (int i = 0; i < 100; i++) {
        AcquireSRWLockExclusive(&srw);
        queue[queued] = i;
        queued = queued + 1;
        expected += i;
        ReleaseSRWLockExclusive(&srw);
        WakeConditionVariable(&cv);
    }
    WaitForSingleObject(c, INFINITE);
    GetExitCodeThread(c, &code);
    check(code == (DWORD)expected, "an SRW lock and a condition variable: producer and consumer");

    CreateThread(0, 0, setter, 0, 0, 0);
    DWORD zero = 0;
    while (flag == 0) WaitOnAddress(&flag, &zero, 4, INFINITE);
    check(flag == 1, "WaitOnAddress and WakeByAddressSingle");

    HANDLE users[4];
    for (int i = 0; i < 4; i++) users[i] = CreateThread(0, 0, once_user, 0, 0, 0);
    WaitForMultipleObjects(4, users, 1, INFINITE);
    int all_saw_it = 1;
    for (int i = 0; i < 4; i++) { GetExitCodeThread(users[i], &code); all_saw_it &= code == 1; }
    check(once_ran == 1 && all_saw_it, "InitOnceExecuteOnce from four threads: once, and all wait for it");

    DWORD tid = 0;
    HANDLE suspended = CreateThread(0, 0, marker, 0, CREATE_SUSPENDED, &tid);
    Sleep(30);
    int before = started;
    ResumeThread(suspended);
    WaitForSingleObject(suspended, INFINITE);
    check(before == 0 && started == 1 && GetThreadId(suspended) == tid, "CREATE_SUSPENDED, then ResumeThread");

    QueueUserAPC(apc, GetCurrentThread(), 77);
    DWORD slept = SleepEx(1000, 1);
    check(slept == WAIT_IO_COMPLETION && apc_value == 77, "QueueUserAPC, run by an alertable SleepEx");

    HANDLE named = CreateEventA(0, 1, 0, "carbon-event");
    HANDLE again = CreateEventA(0, 1, 0, "carbon-event");
    int existed = GetLastError() == 183;
    static const unsigned short wname[] = {'c', 'a', 'r', 'b', 'o', 'n', '-', 'e', 'v', 'e', 'n', 't', 0};
    HANDLE opened = OpenEventW(0x1f0003, 0, wname);
    SetEvent(opened);
    check(named && again && existed && opened && WaitForSingleObject(named, 0) == 0, "named events: created twice, opened, the same event");

    HANDLE spin = CreateThread(0, 0, spinner, 0, 0, 0);
    Sleep(20);
    TerminateThread(spin, 55);
    WaitForSingleObject(spin, INFINITE);
    GetExitCodeThread(spin, &code);
    check(code == 55, "TerminateThread on a thread that's running");

    HANDLE ex = CreateThread(0, 0, exiter, 0, 0, 0);
    WaitForSingleObject(ex, INFINITE);
    GetExitCodeThread(ex, &code);
    check(code == 33, "ExitThread");

    HANDLE real = 0;
    DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &real, 0, 0, 2);
    check(real && GetThreadId(real) == GetCurrentThreadId(), "DuplicateHandle of the current thread's pseudo-handle");
    CloseHandle(real);

    // ---- TLS's expansion slots ----
    static DWORD many[100];
    int got_all = 1;
    for (int i = 0; i < 100; i++) got_all &= (many[i] = TlsAlloc()) != 0xffffffff;
    DWORD high = many[99];
    check(got_all && high >= 64 && high < 1088, "a hundred TlsAlloc slots, past the TEB's 64");
    check(TlsGetValue(high) == 0 && TlsSetValue(high, (void *)0x5eed) && TlsGetValue(high) == (void *)0x5eed, "an expansion slot keeps its value");
    expansion_slot = high;
    HANDLE other = CreateThread(0, 0, expansion_thread, 0, 0, 0);
    WaitForSingleObject(other, 10000);
    CloseHandle(other);
    check(expansion_seen == 1 && TlsGetValue(high) == (void *)0x5eed, "another thread's is its own, 0 at first");
    TlsFree(high);
    DWORD reissued = TlsAlloc();
    check(reissued == high && TlsGetValue(reissued) == 0, "a freed slot, given out again, is 0 in every thread");
    for (int i = 0; i < 99; i++) TlsFree(many[i]);
    TlsFree(reissued);

    ExitProcess(failures);
}
