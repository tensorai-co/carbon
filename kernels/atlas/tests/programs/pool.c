// The thread pool, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: work submitted many times, run on the pool's threads, several at once, and waited for; queued callbacks
// cancelled; a pool of its own with one thread at most, and a minimum; TrySubmitThreadpoolCallback and
// QueueUserWorkItem; an event set, a semaphore released and a critical section left as a callback returns; timers,
// once and every so often, and stopped; waits on a handle, signalled and timed out; RegisterWaitForSingleObject, every
// time and once; a cleanup group closing its members; CallbackMayRunLong and DisassociateCurrentThreadFromCallback;
// I/O objects, completing a file's read at once and a pipe's later, and BindIoCompletionCallback; and timer queues'
// timers, once and every so often, changed and deleted.

typedef __INTPTR_TYPE__ LONG_PTR;
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef __SIZE_TYPE__ SIZE_T;
typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) HANDLE __stdcall CreateEventW(void *, int, int, const void *);
__declspec(dllimport) int __stdcall SetEvent(HANDLE);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) HANDLE __stdcall CreateSemaphoreA(void *, long, long, const char *);
__declspec(dllimport) void __stdcall Sleep(DWORD);
__declspec(dllimport) u64 __stdcall GetTickCount64(void);
__declspec(dllimport) DWORD __stdcall GetCurrentThreadId(void);
typedef struct { void *debug; long lock, recursion; void *owner, *semaphore; u64 spin; } CS;
__declspec(dllimport) void __stdcall InitializeCriticalSection(CS *);
__declspec(dllimport) void __stdcall EnterCriticalSection(CS *);
__declspec(dllimport) void __stdcall LeaveCriticalSection(CS *);
__declspec(dllimport) int __stdcall TryEnterCriticalSection(CS *);

typedef void *PTP;
typedef struct { DWORD version; void *pool, *group, *cancel, *race, *activation, *finalization; DWORD flags, priority, size; } ENV;
__declspec(dllimport) PTP __stdcall CreateThreadpool(void *);
__declspec(dllimport) void __stdcall CloseThreadpool(PTP);
__declspec(dllimport) void __stdcall SetThreadpoolThreadMaximum(PTP, DWORD);
__declspec(dllimport) int __stdcall SetThreadpoolThreadMinimum(PTP, DWORD);
__declspec(dllimport) PTP __stdcall CreateThreadpoolWork(void *, void *, ENV *);
__declspec(dllimport) void __stdcall SubmitThreadpoolWork(PTP);
__declspec(dllimport) void __stdcall WaitForThreadpoolWorkCallbacks(PTP, int);
__declspec(dllimport) void __stdcall CloseThreadpoolWork(PTP);
__declspec(dllimport) int __stdcall TrySubmitThreadpoolCallback(void *, void *, ENV *);
__declspec(dllimport) int __stdcall QueueUserWorkItem(void *, void *, DWORD);
__declspec(dllimport) PTP __stdcall CreateThreadpoolTimer(void *, void *, ENV *);
__declspec(dllimport) void __stdcall SetThreadpoolTimer(PTP, const long long *, DWORD, DWORD);
__declspec(dllimport) int __stdcall IsThreadpoolTimerSet(PTP);
__declspec(dllimport) void __stdcall WaitForThreadpoolTimerCallbacks(PTP, int);
__declspec(dllimport) void __stdcall CloseThreadpoolTimer(PTP);
__declspec(dllimport) PTP __stdcall CreateThreadpoolWait(void *, void *, ENV *);
__declspec(dllimport) void __stdcall SetThreadpoolWait(PTP, HANDLE, const long long *);
__declspec(dllimport) void __stdcall WaitForThreadpoolWaitCallbacks(PTP, int);
__declspec(dllimport) void __stdcall CloseThreadpoolWait(PTP);
__declspec(dllimport) int __stdcall RegisterWaitForSingleObject(HANDLE *, HANDLE, void *, void *, DWORD, DWORD);
__declspec(dllimport) int __stdcall UnregisterWaitEx(HANDLE, HANDLE);
__declspec(dllimport) PTP __stdcall CreateThreadpoolCleanupGroup(void);
__declspec(dllimport) void __stdcall CloseThreadpoolCleanupGroupMembers(PTP, int, void *);
__declspec(dllimport) void __stdcall CloseThreadpoolCleanupGroup(PTP);
__declspec(dllimport) void __stdcall SetEventWhenCallbackReturns(void *, HANDLE);
__declspec(dllimport) void __stdcall ReleaseSemaphoreWhenCallbackReturns(void *, HANDLE, DWORD);
__declspec(dllimport) void __stdcall LeaveCriticalSectionWhenCallbackReturns(void *, CS *);
__declspec(dllimport) int __stdcall CallbackMayRunLong(void *);
__declspec(dllimport) void __stdcall DisassociateCurrentThreadFromCallback(void *);
typedef unsigned short WCHAR;
typedef struct { u64 internal, internal_high; DWORD offset, offset_high; HANDLE event; } OVERLAPPED;
__declspec(dllimport) HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
__declspec(dllimport) HANDLE __stdcall CreateNamedPipeW(const WCHAR *, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, void *);
__declspec(dllimport) int __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, OVERLAPPED *);
__declspec(dllimport) int __stdcall CloseHandle(HANDLE);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) int __stdcall DeleteFileW(const WCHAR *);
__declspec(dllimport) PTP __stdcall CreateThreadpoolIo(HANDLE, void *, void *, ENV *);
__declspec(dllimport) void __stdcall StartThreadpoolIo(PTP);
__declspec(dllimport) void __stdcall CancelThreadpoolIo(PTP);
__declspec(dllimport) void __stdcall WaitForThreadpoolIoCallbacks(PTP, int);
__declspec(dllimport) void __stdcall CloseThreadpoolIo(PTP);
__declspec(dllimport) int __stdcall BindIoCompletionCallback(HANDLE, void *, DWORD);
__declspec(dllimport) HANDLE __stdcall CreateTimerQueue(void);
__declspec(dllimport) int __stdcall CreateTimerQueueTimer(HANDLE *, HANDLE, void *, void *, DWORD, DWORD, DWORD);
__declspec(dllimport) int __stdcall ChangeTimerQueueTimer(HANDLE, HANDLE, DWORD, DWORD);
__declspec(dllimport) int __stdcall DeleteTimerQueueTimer(HANDLE, HANDLE, HANDLE);
__declspec(dllimport) int __stdcall DeleteTimerQueueEx(HANDLE, HANDLE);

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
static int bump(volatile int *p) { return __atomic_add_fetch(p, 1, __ATOMIC_SEQ_CST); }
static int get(volatile int *p) { return __atomic_load_n(p, __ATOMIC_SEQ_CST); }
/// Waits for `*p` to reach `n`, up to `ms`.
static int until(volatile int *p, int n, DWORD ms) {
    u64 end = GetTickCount64() + ms;
    while (get(p) < n) {
        if (GetTickCount64() > end) return 0;
        Sleep(1);
    }
    return 1;
}

static DWORD main_tid;
static volatile int counted, elsewhere, wrong_context;
static PTP the_work;
static void __stdcall count(void *inst, void *ctx, PTP work) {
    (void)inst;
    if (ctx != (void *)0x77 || work != the_work) bump(&wrong_context);
    if (GetCurrentThreadId() != main_tid) bump(&elsewhere);
    bump(&counted);
}

static volatile int gathered;
static void __stdcall gather(void *inst, void *ctx, PTP work) {
    (void)inst; (void)ctx; (void)work;
    bump(&gathered);
    until(&gathered, 4, 5000);
}

static HANDLE gate;
static volatile int blocker_started, cancelled_ran;
static void __stdcall blocker(void *inst, void *ctx, PTP work) { (void)inst; (void)ctx; (void)work; bump(&blocker_started); WaitForSingleObject(gate, 5000); }
static void __stdcall never(void *inst, void *ctx, PTP work) { (void)inst; (void)ctx; (void)work; bump(&cancelled_ran); }

static volatile int simple_ran;
static void __stdcall simple(void *inst, void *ctx) { (void)inst; if (ctx == (void *)5) bump(&simple_ran); }
static HANDLE item_done;
static DWORD __stdcall user_item(void *ctx) { SetEvent(ctx); return 0; }

static HANDLE on_return, sem;
static CS section;
static volatile int long_ok;
static void __stdcall actions(void *inst, void *ctx, PTP work) {
    (void)ctx; (void)work;
    EnterCriticalSection(&section);
    SetEventWhenCallbackReturns(inst, on_return);
    ReleaseSemaphoreWhenCallbackReturns(inst, sem, 2);
    LeaveCriticalSectionWhenCallbackReturns(inst, &section);
    if (CallbackMayRunLong(inst)) bump(&long_ok);
}

static volatile int ticks;
static void __stdcall tick(void *inst, void *ctx, PTP timer) { (void)inst; (void)ctx; (void)timer; bump(&ticks); }

static volatile int waited;
static volatile DWORD wait_result;
static void __stdcall on_wait(void *inst, void *ctx, PTP wait, DWORD result) { (void)inst; (void)ctx; (void)wait; wait_result = result; bump(&waited); }

static volatile int registered, timed_out;
static void __stdcall on_registered(void *ctx, unsigned char timeout) { (void)ctx; if (timeout) bump(&timed_out); else bump(&registered); }

static volatile int grouped;
static void __stdcall in_group(void *inst, void *ctx, PTP work) { (void)inst; (void)ctx; (void)work; Sleep(5); bump(&grouped); }

static HANDLE let_go;
static volatile int let_go_done;
static void __stdcall leaves(void *inst, void *ctx, PTP work) {
    (void)ctx; (void)work;
    DisassociateCurrentThreadFromCallback(inst);
    WaitForSingleObject(let_go, 5000);
    bump(&let_go_done);
}

static WCHAR wbuf[4][64];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }

static volatile int io_done;
static volatile DWORD io_result;
static volatile u64 io_bytes;
static void *volatile io_overlapped;
static void __stdcall on_io(void *inst, void *ctx, void *overlapped, DWORD result, ULONG_PTR bytes, PTP io) {
    (void)inst; (void)io;
    if (ctx != (void *)0x10) return;
    io_result = result;
    io_bytes = bytes;
    io_overlapped = overlapped;
    bump(&io_done);
}
static volatile int bound_done;
static volatile DWORD bound_bytes;
static void __stdcall on_bound(DWORD error, DWORD bytes, void *overlapped) { (void)overlapped; if (error == 0) bound_bytes = bytes; bump(&bound_done); }

static volatile int queued_once, queued_often;
static void *volatile queued_param;
static unsigned char queued_flag;
static void __stdcall once_timer(void *param, unsigned char fired) { queued_param = param; queued_flag = fired; bump(&queued_once); }
static void __stdcall often_timer(void *param, unsigned char fired) { (void)param; (void)fired; bump(&queued_often); }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("the thread pool, arm64\n");
#elif defined(__i386__)
    say("the thread pool, x86 (WoW64)\n");
#else
    say("the thread pool, x86-64\n");
#endif
    main_tid = GetCurrentThreadId();

    the_work = CreateThreadpoolWork((void *)count, (void *)0x77, 0);
    for (int i = 0; i < 10; i++) SubmitThreadpoolWork(the_work);
    WaitForThreadpoolWorkCallbacks(the_work, 0);
    check(the_work && get(&counted) == 10 && get(&wrong_context) == 0, "work submitted ten times runs ten times, with its context, and is waited for");
    check(get(&elsewhere) == 10, "on the pool's threads");
    CloseThreadpoolWork(the_work);

    PTP gatherer = CreateThreadpoolWork((void *)gather, 0, 0);
    for (int i = 0; i < 4; i++) SubmitThreadpoolWork(gatherer);
    WaitForThreadpoolWorkCallbacks(gatherer, 0);
    check(get(&gathered) == 4, "four at once, each waiting for the others");
    CloseThreadpoolWork(gatherer);

    PTP pool = CreateThreadpool(0);
    SetThreadpoolThreadMaximum(pool, 1);
    ENV env = {3, pool, 0, 0, 0, 0, 0, 0, 1, sizeof(ENV)};
    gate = CreateEventW(0, 1, 0, 0);
    PTP first = CreateThreadpoolWork((void *)blocker, 0, &env), second = CreateThreadpoolWork((void *)never, 0, &env);
    SubmitThreadpoolWork(first);
    until(&blocker_started, 1, 5000);
    for (int i = 0; i < 5; i++) SubmitThreadpoolWork(second);
    WaitForThreadpoolWorkCallbacks(second, 1);
    SetEvent(gate);
    WaitForThreadpoolWorkCallbacks(first, 0);
    check(get(&blocker_started) == 1 && get(&cancelled_ran) == 0, "a pool of one thread: what's queued behind a callback, cancelled");
    check(SetThreadpoolThreadMinimum(pool, 2), "SetThreadpoolThreadMinimum");
    CloseThreadpoolWork(first);
    CloseThreadpoolWork(second);
    CloseThreadpool(pool);

    check(TrySubmitThreadpoolCallback((void *)simple, (void *)5, 0) && until(&simple_ran, 1, 5000), "TrySubmitThreadpoolCallback");
    item_done = CreateEventW(0, 0, 0, 0);
    check(QueueUserWorkItem((void *)user_item, item_done, 0) && WaitForSingleObject(item_done, 5000) == 0, "QueueUserWorkItem");

    on_return = CreateEventW(0, 0, 0, 0);
    sem = CreateSemaphoreA(0, 0, 10, 0);
    InitializeCriticalSection(&section);
    PTP acting = CreateThreadpoolWork((void *)actions, 0, 0);
    SubmitThreadpoolWork(acting);
    check(WaitForSingleObject(on_return, 5000) == 0, "SetEventWhenCallbackReturns");
    check(WaitForSingleObject(sem, 5000) == 0 && WaitForSingleObject(sem, 5000) == 0 && WaitForSingleObject(sem, 0) == 0x102, "ReleaseSemaphoreWhenCallbackReturns, by two");
    int entered = TryEnterCriticalSection(&section);
    if (entered) LeaveCriticalSection(&section);
    check(entered, "LeaveCriticalSectionWhenCallbackReturns");
    check(get(&long_ok) == 1, "CallbackMayRunLong");
    WaitForThreadpoolWorkCallbacks(acting, 0);
    CloseThreadpoolWork(acting);

    PTP timer = CreateThreadpoolTimer((void *)tick, 0, 0);
    long long due = -50 * 10000;
    u64 start = GetTickCount64();
    SetThreadpoolTimer(timer, &due, 0, 0);
    check(IsThreadpoolTimerSet(timer), "SetThreadpoolTimer: it's set");
    check(until(&ticks, 1, 5000) && GetTickCount64() - start >= 40, "it comes due, once, after its time");
    Sleep(100);
    check(get(&ticks) == 1 && !IsThreadpoolTimerSet(timer), "and isn't set any more");
    ticks = 0;
    due = -10 * 10000;
    SetThreadpoolTimer(timer, &due, 20, 0);
    check(until(&ticks, 3, 5000), "every 20 ms");
    SetThreadpoolTimer(timer, 0, 0, 0);
    WaitForThreadpoolTimerCallbacks(timer, 1);
    int stopped = get(&ticks);
    Sleep(100);
    check(get(&ticks) == stopped && !IsThreadpoolTimerSet(timer), "stopped, it comes no more");
    CloseThreadpoolTimer(timer);

    HANDLE event = CreateEventW(0, 0, 0, 0);
    PTP wait = CreateThreadpoolWait((void *)on_wait, 0, 0);
    SetThreadpoolWait(wait, event, 0);
    Sleep(20);
    check(get(&waited) == 0, "a wait on an event that isn't set: nothing yet");
    SetEvent(event);
    check(until(&waited, 1, 5000) && wait_result == 0, "set: its callback, with WAIT_OBJECT_0");
    long long timeout = -30 * 10000;
    SetThreadpoolWait(wait, event, &timeout);
    check(until(&waited, 2, 5000) && wait_result == 0x102, "a wait that times out: WAIT_TIMEOUT");
    SetEvent(event);
    Sleep(50);
    check(get(&waited) == 2, "once it's had its callback, it waits no more until it's set again");
    WaitForThreadpoolWaitCallbacks(wait, 1);
    CloseThreadpoolWait(wait);

    HANDLE again = CreateEventW(0, 0, 0, 0), registration = 0;
    check(RegisterWaitForSingleObject(&registration, again, (void *)on_registered, 0, 0xffffffff, 0), "RegisterWaitForSingleObject");
    for (int i = 0; i < 3; i++) {
        SetEvent(again);
        until(&registered, i + 1, 5000);
    }
    check(get(&registered) == 3, "its callback each time the event's set");
    check(UnregisterWaitEx(registration, (HANDLE)(long long)-1), "UnregisterWaitEx, waiting for it");
    SetEvent(again);
    Sleep(30);
    check(get(&registered) == 3, "and no more after");
    HANDLE never_set = CreateEventW(0, 0, 0, 0);
    RegisterWaitForSingleObject(&registration, never_set, (void *)on_registered, 0, 30, 8);
    check(until(&timed_out, 1, 5000), "WT_EXECUTEONLYONCE with a timeout: called, timed out");
    Sleep(100);
    check(get(&timed_out) == 1, "  once");
    UnregisterWaitEx(registration, 0);

    PTP group = CreateThreadpoolCleanupGroup();
    ENV grouped_env = {3, 0, group, 0, 0, 0, 0, 0, 1, sizeof(ENV)};
    for (int i = 0; i < 3; i++) SubmitThreadpoolWork(CreateThreadpoolWork((void *)in_group, 0, &grouped_env));
    CloseThreadpoolCleanupGroupMembers(group, 0, 0);
    check(get(&grouped) == 3, "CloseThreadpoolCleanupGroupMembers: its members' callbacks run, then they're closed");
    CloseThreadpoolCleanupGroup(group);

    let_go = CreateEventW(0, 1, 0, 0);
    PTP leaving = CreateThreadpoolWork((void *)leaves, 0, 0);
    SubmitThreadpoolWork(leaving);
    Sleep(20);
    WaitForThreadpoolWorkCallbacks(leaving, 0);
    check(get(&let_go_done) == 0, "DisassociateCurrentThreadFromCallback: waiting for callbacks doesn't wait for it");
    SetEvent(let_go);
    check(until(&let_go_done, 1, 5000), "  and it goes on running");
    CloseThreadpoolWork(leaving);

    // I/O objects.
    HANDLE f = CreateFileW(W(0, "C:\\pool.txt"), 0xc0000000, 0, 0, 2, 0, 0);
    DWORD wrote = 0;
    WriteFile(f, "hello", 5, &wrote, 0);
    CloseHandle(f);
    f = CreateFileW(W(0, "C:\\pool.txt"), 0x80000000, 1, 0, 3, 0x40000000, 0);
    PTP io = CreateThreadpoolIo(f, (void *)on_io, (void *)0x10, 0);
    check(io != 0, "CreateThreadpoolIo");
    char buffer[16];
    OVERLAPPED ov = {0, 0, 0, 0, 0};
    StartThreadpoolIo(io);
    int read_now = ReadFile(f, buffer, 16, 0, &ov);
    check(read_now && until(&io_done, 1, 5000) && io_result == 0 && io_bytes == 5 && io_overlapped == &ov, "a file's overlapped read: its callback, with the OVERLAPPED and the bytes");
    WaitForThreadpoolIoCallbacks(io, 0);
    CloseThreadpoolIo(io);
    CloseHandle(f);

    HANDLE server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-pool"), 3 | 0x40000000, 0, 1, 512, 512, 0, 0);
    HANDLE client = CreateFileW(W(1, "\\\\.\\pipe\\atlas-pool"), 0xc0000000, 0, 0, 3, 0, 0);
    io = CreateThreadpoolIo(server, (void *)on_io, (void *)0x10, 0);
    OVERLAPPED pending = {0, 0, 0, 0, 0};
    StartThreadpoolIo(io);
    int now_read = ReadFile(server, buffer, 16, 0, &pending);
    DWORD why = GetLastError();
    Sleep(20);
    check(!now_read && why == 997 && get(&io_done) == 1, "a pipe's read, pending: no callback yet");
    WriteFile(client, "abcdefg", 7, &wrote, 0);
    check(until(&io_done, 2, 5000) && io_bytes == 7 && io_overlapped == &pending && buffer[0] == 'a', "  the client writes: its callback");
    StartThreadpoolIo(io);
    CancelThreadpoolIo(io);
    WaitForThreadpoolIoCallbacks(io, 0);
    CloseThreadpoolIo(io);
    CloseHandle(client);
    CloseHandle(server);

    f = CreateFileW(W(0, "C:\\pool.txt"), 0x40000000, 0, 0, 3, 0x40000000, 0);
    check(BindIoCompletionCallback(f, (void *)on_bound, 0), "BindIoCompletionCallback");
    OVERLAPPED at_end = {0, 0, 5, 0, 0};
    WriteFile(f, " there", 6, 0, &at_end);
    check(until(&bound_done, 1, 5000) && bound_bytes == 6, "  a write's completion: function(error, bytes, overlapped)");
    CloseHandle(f);
    DeleteFileW(W(0, "C:\\pool.txt"));

    // Timer queues.
    HANDLE queue = CreateTimerQueue(), once = 0, often = 0, plain = 0;
    u64 begun = GetTickCount64();
    check(queue && CreateTimerQueueTimer(&once, queue, (void *)once_timer, (void *)0x99, 30, 500, 8), "CreateTimerQueue and CreateTimerQueueTimer");
    check(until(&queued_once, 1, 5000) && GetTickCount64() - begun >= 20 && queued_param == (void *)0x99 && queued_flag == 1, "  it's due: callback(parameter, TRUE)");
    Sleep(600);
    check(get(&queued_once) == 1, "  WT_EXECUTEONLYONCE: once, whatever its period");
    CreateTimerQueueTimer(&often, queue, (void *)often_timer, 0, 5, 15, 0);
    check(until(&queued_often, 3, 5000), "every 15 ms");
    ChangeTimerQueueTimer(queue, often, 10000, 0);
    Sleep(30);
    int was = get(&queued_often);
    Sleep(60);
    check(get(&queued_often) == was, "ChangeTimerQueueTimer: put off, it comes no more for now");
    check(DeleteTimerQueueTimer(queue, often, (HANDLE)(long long)-1), "DeleteTimerQueueTimer, waiting for its callbacks");
    int before = get(&queued_once);
    check(CreateTimerQueueTimer(&plain, 0, (void *)once_timer, (void *)0x98, 10, 0, 0) && until(&queued_once, before + 1, 5000) && queued_param == (void *)0x98,
          "a timer on the default queue");
    DeleteTimerQueueTimer(0, plain, (HANDLE)(long long)-1);
    check(DeleteTimerQueueEx(queue, (HANDLE)(long long)-1), "DeleteTimerQueueEx");

    ExitProcess(failures);
}
