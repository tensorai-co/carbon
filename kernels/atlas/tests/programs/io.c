// Asynchronous I/O and completion ports on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds
// it.
//
// - a completion port as a work queue for four threads;
// - a timeout, and GetQueuedCompletionStatusEx taking several packets, or running an APC when it's alertable;
// - overlapped I/O on a file at an offset, with its event, OVERLAPPED status and GetOverlappedResult;
// - a file associated with a port: a packet for each I/O with its key, none for a failed read past the end, none with
//   FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, and no second association;
// - ReadFileEx and WriteFileEx's routines in an alertable wait;
// - CancelIo and CancelIoEx with nothing under way.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned long long u64;

typedef struct {
    u64 internal, internal_high;
    DWORD offset, offset_high;
    HANDLE event;
} OVERLAPPED;

typedef struct {
    u64 key;
    OVERLAPPED *overlapped;
    u64 internal;
    DWORD bytes;
} OVERLAPPED_ENTRY;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, OVERLAPPED *);
K int __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, OVERLAPPED *);
K int __stdcall ReadFileEx(HANDLE, void *, DWORD, OVERLAPPED *, void (*)(DWORD, DWORD, OVERLAPPED *));
K int __stdcall WriteFileEx(HANDLE, const void *, DWORD, OVERLAPPED *, void (*)(DWORD, DWORD, OVERLAPPED *));
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K int __stdcall CloseHandle(HANDLE);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K DWORD __stdcall GetFileSize(HANDLE, DWORD *);
K int __stdcall DeleteFileW(const WCHAR *);
K HANDLE __stdcall CreateIoCompletionPort(HANDLE, HANDLE, u64, DWORD);
K int __stdcall PostQueuedCompletionStatus(HANDLE, DWORD, u64, OVERLAPPED *);
K int __stdcall GetQueuedCompletionStatus(HANDLE, DWORD *, u64 *, OVERLAPPED **, DWORD);
K int __stdcall GetQueuedCompletionStatusEx(HANDLE, OVERLAPPED_ENTRY *, DWORD, DWORD *, DWORD, int);
K int __stdcall GetOverlappedResult(HANDLE, OVERLAPPED *, DWORD *, int);
K int __stdcall CancelIo(HANDLE);
K int __stdcall CancelIoEx(HANDLE, OVERLAPPED *);
K int __stdcall SetFileCompletionNotificationModes(HANDLE, unsigned char);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K DWORD __stdcall WaitForMultipleObjects(DWORD, const HANDLE *, int, DWORD);
K HANDLE __stdcall CreateThread(void *, u64, DWORD (*)(void *), void *, DWORD, DWORD *);
K DWORD __stdcall QueueUserAPC(void (*)(u64), HANDLE, u64);
K HANDLE __stdcall GetCurrentThread(void);
K DWORD __stdcall SleepEx(DWORD, int);

enum { GENERIC_RW = 0xc0000000, CREATE_ALWAYS = 2, FILE_FLAG_OVERLAPPED = 0x40000000, ERROR_HANDLE_EOF = 38,
       WAIT_TIMEOUT = 258, WAIT_IO_COMPLETION = 0xc0, ERROR_NOT_FOUND = 1168 };
#define INVALID ((HANDLE)(long long)-1)

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
static WCHAR *W(const char *s) {
    static WCHAR buf[256];
    int n = 0;
    for (; s[n]; n++) buf[n] = (WCHAR)s[n];
    buf[n] = 0;
    return buf;
}
static void zero(void *p, unsigned n) { for (unsigned i = 0; i < n; i++) ((volatile char *)p)[i] = 0; }

// Four workers take numbers from a port and add them up, until a packet with key 0.
static HANDLE queue;
static volatile u64 sums[4];
static DWORD worker(void *which) {
    for (;;) {
        DWORD bytes;
        u64 key;
        OVERLAPPED *ov;
        if (!GetQueuedCompletionStatus(queue, &bytes, &key, &ov, 10000) || key == 0) return 0;
        sums[(u64)which] += bytes;
    }
}

static volatile DWORD routine_error = 99, routine_bytes;
static OVERLAPPED *volatile routine_ov;
static volatile int routines;
static void routine(DWORD error, DWORD bytes, OVERLAPPED *ov) {
    routine_error = error;
    routine_bytes = bytes;
    routine_ov = ov;
    routines = routines + 1;
}
static volatile int apcs;
static void apc(u64 x) { apcs = apcs + (int)x; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("asynchronous I/O on atlas, arm64\n");
#else
    say("asynchronous I/O on atlas, x86-64\n");
#endif
    DWORD bytes = 0;
    u64 key = 0;
    OVERLAPPED *ov = (OVERLAPPED *)1;

    // A work queue.
    queue = CreateIoCompletionPort(INVALID, 0, 0, 0);
    HANDLE threads[4];
    for (u64 i = 0; i < 4; i++) threads[i] = CreateThread(0, 0, worker, (void *)i, 0, 0);
    u64 expected = 0;
    for (DWORD i = 1; i <= 200; i++) {
        PostQueuedCompletionStatus(queue, i, 1, 0);
        expected += i;
    }
    for (int i = 0; i < 4; i++) PostQueuedCompletionStatus(queue, 0, 0, 0);
    WaitForMultipleObjects(4, threads, 1, 10000);
    u64 total = sums[0] + sums[1] + sums[2] + sums[3];
    check(queue && total == expected, "a completion port as a work queue: 200 packets among four threads, each taken once");
    for (int i = 0; i < 4; i++) CloseHandle(threads[i]);

    SetLastError(0);
    check(!GetQueuedCompletionStatus(queue, &bytes, &key, &ov, 50) && GetLastError() == WAIT_TIMEOUT && ov == 0, "nothing queued: WAIT_TIMEOUT after 50 ms, and no OVERLAPPED");
    OVERLAPPED marker;
    PostQueuedCompletionStatus(queue, 11, 21, &marker);
    PostQueuedCompletionStatus(queue, 12, 22, 0);
    PostQueuedCompletionStatus(queue, 13, 23, 0);
    OVERLAPPED_ENTRY entries[8];
    DWORD removed = 0;
    check(GetQueuedCompletionStatusEx(queue, entries, 8, &removed, 1000, 0) && removed == 3 && entries[0].key == 21 && entries[0].overlapped == &marker &&
              entries[0].bytes == 11 && entries[2].key == 23,
          "GetQueuedCompletionStatusEx: three packets at once, in order");
    QueueUserAPC(apc, GetCurrentThread(), 5);
    SetLastError(0);
    check(!GetQueuedCompletionStatusEx(queue, entries, 8, &removed, 5000, 1) && GetLastError() == WAIT_IO_COMPLETION && apcs == 5 && removed == 0,
          "alertable, with an APC queued: the APC runs, WAIT_IO_COMPLETION");

    // Overlapped I/O on a file, at an offset.
    WCHAR *path = W("C:\\Windows\\Temp\\atlas-io.bin");
    HANDLE file = CreateFileW(path, GENERIC_RW, 0, 0, CREATE_ALWAYS, FILE_FLAG_OVERLAPPED, 0);
    static char data[4096], back[4096];
    for (int i = 0; i < 4096; i++) data[i] = (char)(i * 7);
    OVERLAPPED o;
    zero(&o, sizeof o);
    o.offset = 8192;
    o.event = CreateEventW(0, 1, 0, 0);
    int wrote = WriteFile(file, data, 4096, 0, &o) || GetLastError() == 997;
    check(wrote && GetOverlappedResult(file, &o, &bytes, 1) && bytes == 4096 && o.internal == 0 && o.internal_high == 4096, "an overlapped write at 8 KiB: GetOverlappedResult, the OVERLAPPED's status and count");
    check(WaitForSingleObject(o.event, 0) == 0 && GetFileSize(file, 0) == 12288, "its event set, the file 12 KiB");
    zero(&o, 16);
    o.offset = 8192;
    int read = ReadFile(file, back, 4096, 0, &o) || GetLastError() == 997;
    int same = 1;
    for (int i = 0; i < 4096; i++) same = same && back[i] == data[i];
    check(read && GetOverlappedResult(file, &o, &bytes, 1) && bytes == 4096 && same, "an overlapped read back, the same bytes");
    zero(&o, 16);
    o.offset = 20000;
    SetLastError(0);
    check(!ReadFile(file, back, 16, 0, &o) && GetLastError() == ERROR_HANDLE_EOF && o.internal == 0xc0000011, "a read past the end: ERROR_HANDLE_EOF, STATUS_END_OF_FILE in the OVERLAPPED");
    SetLastError(0);
    check(!GetOverlappedResult(file, &o, &bytes, 0) && GetLastError() == ERROR_HANDLE_EOF, "and GetOverlappedResult says so too");

    // Associated with a port.
    HANDLE port = CreateIoCompletionPort(file, 0, 7, 0);
    zero(&o, 16);
    o.offset = 100;
    o.event = 0;
    WriteFile(file, "port", 4, 0, &o);
    ov = 0;
    check(port && GetQueuedCompletionStatus(port, &bytes, &key, &ov, 1000) && key == 7 && bytes == 4 && ov == &o, "a file on a port: a packet with its key, count and OVERLAPPED");
    zero(&o, 16);
    o.offset = 50000;
    ReadFile(file, back, 16, 0, &o);
    check(!GetQueuedCompletionStatus(port, &bytes, &key, &ov, 0), "a read that failed at once: no packet");
    HANDLE second = CreateIoCompletionPort(INVALID, 0, 0, 0);
    SetLastError(0);
    check(!CreateIoCompletionPort(file, second, 8, 0) && GetLastError() == 87, "a second association: ERROR_INVALID_PARAMETER");
    check(CreateIoCompletionPort(file, 0, 0, 0) == 0, "nor a new port for it");
    SetFileCompletionNotificationModes(file, 1);
    zero(&o, 16);
    o.offset = 0;
    check(WriteFile(file, "skip", 4, &bytes, &o) && bytes == 4 && !GetQueuedCompletionStatus(port, &bytes, &key, &ov, 0), "FILE_SKIP_COMPLETION_PORT_ON_SUCCESS: done at once, no packet");

    // ReadFileEx and WriteFileEx.
    OVERLAPPED e;
    zero(&e, sizeof e);
    e.offset = 200;
    check(WriteFileEx(file, "routine", 7, &e, routine) && routines == 0, "WriteFileEx: its routine waits for an alertable wait");
    check(SleepEx(5000, 1) == WAIT_IO_COMPLETION && routines == 1 && routine_error == 0 && routine_bytes == 7 && routine_ov == &e, "SleepEx runs it: no error, 7 bytes, its OVERLAPPED");
    zero(&e, sizeof e);
    e.offset = 200;
    char seven[8] = {0};
    ReadFileEx(file, seven, 7, &e, routine);
    check(SleepEx(5000, 1) == WAIT_IO_COMPLETION && routines == 2 && routine_bytes == 7 && seven[0] == 'r' && seven[6] == 'e', "ReadFileEx: the bytes, then its routine");

    check(CancelIo(file), "CancelIo: nothing under way");
    SetLastError(0);
    check(!CancelIoEx(file, &e) && GetLastError() == ERROR_NOT_FOUND, "CancelIoEx: ERROR_NOT_FOUND");

    CloseHandle(file);
    CloseHandle(port);
    CloseHandle(second);
    CloseHandle(queue);
    DeleteFileW(path);
    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
