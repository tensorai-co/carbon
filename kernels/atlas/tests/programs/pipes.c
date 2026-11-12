// Named pipes on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// - a byte pipe: a client before ConnectNamedPipe (ERROR_PIPE_CONNECTED), data both ways, each end's process,
//   ERROR_PIPE_BUSY and WaitNamedPipe's timeout, the client closing (ERROR_BROKEN_PIPE, then ERROR_NO_DATA to connect),
//   DisconnectNamedPipe and a new client, and names refused or not there;
// - pending I/O: ConnectNamedPipe completed by a client's CreateFile, a read completed by a write (with its event, its
//   port's packet, or ReadFileEx's routine), CancelIoEx, and a read ended by the client closing;
// - a message pipe: messages one at a time, ERROR_MORE_DATA and the rest, PeekNamedPipe's message left,
//   TransactNamedPipe and CallNamedPipe against a server thread;
// - another process: its pending read completed by this one's write, into its own memory, and its end closed as it
//   exits, ending this one's pending read.

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

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, OVERLAPPED *);
K int __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, OVERLAPPED *);
K int __stdcall ReadFileEx(HANDLE, void *, DWORD, OVERLAPPED *, void (*)(DWORD, DWORD, OVERLAPPED *));
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K int __stdcall CloseHandle(HANDLE);
K DWORD __stdcall GetFileType(HANDLE);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K HANDLE __stdcall CreateNamedPipeW(const WCHAR *, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, void *);
K int __stdcall ConnectNamedPipe(HANDLE, OVERLAPPED *);
K int __stdcall DisconnectNamedPipe(HANDLE);
K int __stdcall WaitNamedPipeW(const WCHAR *, DWORD);
K int __stdcall SetNamedPipeHandleState(HANDLE, DWORD *, DWORD *, DWORD *);
K int __stdcall GetNamedPipeInfo(HANDLE, DWORD *, DWORD *, DWORD *, DWORD *);
K int __stdcall GetNamedPipeClientProcessId(HANDLE, DWORD *);
K int __stdcall GetNamedPipeServerProcessId(HANDLE, DWORD *);
K int __stdcall TransactNamedPipe(HANDLE, void *, DWORD, void *, DWORD, DWORD *, OVERLAPPED *);
K int __stdcall CallNamedPipeW(const WCHAR *, void *, DWORD, void *, DWORD, DWORD *, DWORD);
K int __stdcall PeekNamedPipe(HANDLE, void *, DWORD, DWORD *, DWORD *, DWORD *);
K int __stdcall GetOverlappedResult(HANDLE, OVERLAPPED *, DWORD *, int);
K int __stdcall CancelIoEx(HANDLE, OVERLAPPED *);
K HANDLE __stdcall CreateIoCompletionPort(HANDLE, HANDLE, u64, DWORD);
K int __stdcall GetQueuedCompletionStatus(HANDLE, DWORD *, u64 *, OVERLAPPED **, DWORD);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K HANDLE __stdcall CreateThread(void *, u64, DWORD (*)(void *), void *, DWORD, DWORD *);
K DWORD __stdcall SleepEx(DWORD, int);
K void __stdcall Sleep(DWORD);
K DWORD __stdcall GetCurrentProcessId(void);
K int __stdcall CreateProcessW(const WCHAR *, WCHAR *, void *, void *, int, DWORD, void *, const WCHAR *, STARTUPINFOW *, PROCESS_INFORMATION *);
K int __stdcall GetExitCodeProcess(HANDLE, DWORD *);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K char *__stdcall GetCommandLineA(void);

enum { DUPLEX = 3, OVERLAPPED_FLAG = 0x40000000, FIRST_INSTANCE = 0x80000, TYPE_MESSAGE = 4, READMODE_MESSAGE = 2,
       GENERIC_RW = 0xc0000000, OPEN_EXISTING = 3, ERROR_FILE_NOT_FOUND = 2, ERROR_ACCESS_DENIED = 5,
       ERROR_BROKEN_PIPE = 109, ERROR_SEM_TIMEOUT = 121, ERROR_INVALID_NAME = 123, ERROR_PIPE_BUSY = 231,
       ERROR_NO_DATA = 232, ERROR_MORE_DATA = 234, ERROR_PIPE_CONNECTED = 535, ERROR_OPERATION_ABORTED = 995,
       ERROR_IO_PENDING = 997, ERROR_NOT_FOUND = 1168, WAIT_IO_COMPLETION = 0xc0, STATUS_PENDING = 0x103 };
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
static int same(const char *a, const char *b, unsigned n) { for (unsigned i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }
static void zero(void *p, unsigned n) { for (unsigned i = 0; i < n; i++) ((volatile char *)p)[i] = 0; }
static WCHAR *W(int slot, const char *s) {
    static WCHAR buf[4][128];
    int n = 0;
    for (; s[n]; n++) buf[slot][n] = (WCHAR)s[n];
    buf[slot][n] = 0;
    return buf[slot];
}
static HANDLE client(const char *name, DWORD flags) { return CreateFileW(W(3, name), GENERIC_RW, 0, 0, OPEN_EXISTING, flags, 0); }
static int put(HANDLE h, const char *s) { DWORD n; return WriteFile(h, s, len(s), &n, 0) && n == len(s); }

/// The other process: connects, says it's ready, waits on an overlapped read the parent completes, then exits with
/// its end still open.
static void child(void) {
    HANDLE pipe = client("\\\\.\\pipe\\atlas-across", OVERLAPPED_FLAG);
    if (pipe == INVALID) ExitProcess(1);
    OVERLAPPED o;
    zero(&o, sizeof o);
    WriteFile(pipe, "ready", 5, 0, &o);
    zero(&o, sizeof o);
    o.event = CreateEventW(0, 1, 0, 0);
    char buf[32] = {0};
    int pending = !ReadFile(pipe, buf, sizeof buf, 0, &o) && GetLastError() == ERROR_IO_PENDING;
    DWORD n = 0;
    int read = WaitForSingleObject(o.event, 10000) == 0 && GetOverlappedResult(pipe, &o, &n, 0) && n == 11 && same(buf, "from parent", 11);
    ExitProcess(pending && read ? 0 : 2);
}

// A server thread for a message pipe: answers each message with "echo:" and it, for each client in turn.
static HANDLE echo_pipe;
static DWORD echo(void *unused) {
    (void)unused;
    for (int clients = 0; clients < 2; clients++) {
        ConnectNamedPipe(echo_pipe, 0);
        for (;;) {
            char in[64], reply[80] = "echo:";
            DWORD n;
            if (!ReadFile(echo_pipe, in, sizeof in, &n, 0)) break;
            for (DWORD i = 0; i < n; i++) reply[5 + i] = in[i];
            DWORD w;
            WriteFile(echo_pipe, reply, 5 + n, &w, 0);
        }
        DisconnectNamedPipe(echo_pipe);
    }
    return 0;
}

static volatile DWORD routine_bytes, routine_error = 99;
static void routine(DWORD error, DWORD bytes, OVERLAPPED *ov) {
    (void)ov;
    routine_error = error;
    routine_bytes = bytes;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    const char *line = GetCommandLineA();
    for (const char *s = line; *s; s++)
        if (same(s, " child", 6)) child();
#if defined(__aarch64__)
    say("named pipes on atlas, arm64\n");
#else
    say("named pipes on atlas, x86-64\n");
#endif
    char buf[64];
    DWORD n, pid;

    // A byte pipe.
    HANDLE server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-bytes"), DUPLEX, 0, 1, 4096, 4096, 0, 0);
    HANDLE c = client("\\\\.\\pipe\\atlas-bytes", 0);
    SetLastError(0);
    check(server != INVALID && c != INVALID && GetFileType(server) == 3 && !ConnectNamedPipe(server, 0) && GetLastError() == ERROR_PIPE_CONNECTED,
          "a client before ConnectNamedPipe: connected already, ERROR_PIPE_CONNECTED");
    put(c, "hello");
    check(ReadFile(server, buf, sizeof buf, &n, 0) && n == 5 && same(buf, "hello", 5), "client to server");
    put(server, "back");
    check(ReadFile(c, buf, sizeof buf, &n, 0) && n == 4 && same(buf, "back", 4), "server to client");
    check(GetNamedPipeClientProcessId(server, &pid) && pid == GetCurrentProcessId() && GetNamedPipeServerProcessId(c, &pid) && pid == GetCurrentProcessId(),
          "each end's process");
    SetLastError(0);
    check(client("\\\\.\\pipe\\atlas-bytes", 0) == INVALID && GetLastError() == ERROR_PIPE_BUSY, "its one instance taken: ERROR_PIPE_BUSY");
    SetLastError(0);
    check(!WaitNamedPipeW(W(0, "\\\\.\\pipe\\atlas-bytes"), 50) && GetLastError() == ERROR_SEM_TIMEOUT, "WaitNamedPipe: ERROR_SEM_TIMEOUT after 50 ms");
    put(c, "last");
    CloseHandle(c);
    int rest = ReadFile(server, buf, sizeof buf, &n, 0) && n == 4;
    SetLastError(0);
    check(rest && !ReadFile(server, buf, sizeof buf, &n, 0) && GetLastError() == ERROR_BROKEN_PIPE, "the client closed: what it wrote, then ERROR_BROKEN_PIPE");
    SetLastError(0);
    check(!ConnectNamedPipe(server, 0) && GetLastError() == ERROR_NO_DATA, "ConnectNamedPipe before disconnecting: ERROR_NO_DATA");
    check(DisconnectNamedPipe(server) && client("\\\\.\\pipe\\atlas-bytes", 0) == INVALID, "DisconnectNamedPipe: no client until ConnectNamedPipe");
    CloseHandle(server);
    SetLastError(0);
    check(client("\\\\.\\pipe\\atlas-none", 0) == INVALID && GetLastError() == ERROR_FILE_NOT_FOUND, "no such pipe: ERROR_FILE_NOT_FOUND");
    SetLastError(0);
    check(CreateNamedPipeW(W(0, "atlas-bad"), DUPLEX, 0, 1, 0, 0, 0, 0) == INVALID && GetLastError() == ERROR_INVALID_NAME, "a name outside \\\\.\\pipe\\: ERROR_INVALID_NAME");
    HANDLE first = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-first"), DUPLEX, 0, 2, 0, 0, 0, 0);
    SetLastError(0);
    check(CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-first"), DUPLEX | FIRST_INSTANCE, 0, 2, 0, 0, 0, 0) == INVALID && GetLastError() == ERROR_ACCESS_DENIED,
          "FILE_FLAG_FIRST_PIPE_INSTANCE when there's one: ERROR_ACCESS_DENIED");
    CloseHandle(first);

    // Pending I/O.
    server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-async"), DUPLEX | OVERLAPPED_FLAG, 0, 1, 0, 0, 0, 0);
    OVERLAPPED o;
    zero(&o, sizeof o);
    o.event = CreateEventW(0, 1, 0, 0);
    SetLastError(0);
    int pending = !ConnectNamedPipe(server, &o) && GetLastError() == ERROR_IO_PENDING && o.internal == STATUS_PENDING;
    check(pending && WaitForSingleObject(o.event, 0) == 0x102 && WaitNamedPipeW(W(0, "\\\\.\\pipe\\atlas-async"), 1000), "an overlapped ConnectNamedPipe: pending, and WaitNamedPipe sees it listening");
    c = client("\\\\.\\pipe\\atlas-async", 0);
    check(c != INVALID && WaitForSingleObject(o.event, 1000) == 0 && GetOverlappedResult(server, &o, &n, 0), "a client's CreateFile completes it");
    zero(&o, 16);
    SetLastError(0);
    pending = !ReadFile(server, buf, sizeof buf, 0, &o) && GetLastError() == ERROR_IO_PENDING;
    check(pending && WaitForSingleObject(o.event, 50) == 0x102, "an overlapped read with nothing there: pending");
    put(c, "later");
    check(WaitForSingleObject(o.event, 1000) == 0 && GetOverlappedResult(server, &o, &n, 0) && n == 5 && same(buf, "later", 5) && o.internal_high == 5,
          "the client's write completes it: its event, the bytes, the OVERLAPPED");
    HANDLE port = CreateIoCompletionPort(server, 0, 42, 0);
    OVERLAPPED p;
    zero(&p, sizeof p);
    ReadFile(server, buf, sizeof buf, 0, &p);
    put(c, "packet");
    u64 key = 0;
    OVERLAPPED *got = 0;
    check(GetQueuedCompletionStatus(port, &n, &key, &got, 1000) && key == 42 && n == 6 && got == &p && same(buf, "packet", 6), "on a port: the write's completion as a packet");
    OVERLAPPED e;
    zero(&e, sizeof e);
    check(ReadFileEx(server, buf, sizeof buf, &e, routine) && routine_error == 99, "ReadFileEx with nothing there: pending, and TRUE");
    put(c, "routine");
    check(SleepEx(1000, 1) == WAIT_IO_COMPLETION && routine_error == 0 && routine_bytes == 7 && same(buf, "routine", 7), "the write queues its routine, which SleepEx runs");
    zero(&o, 16);
    ReadFile(server, buf, sizeof buf, 0, &o);
    check(CancelIoEx(server, &o) && !GetOverlappedResult(server, &o, &n, 1) && GetLastError() == ERROR_OPERATION_ABORTED, "CancelIoEx: ERROR_OPERATION_ABORTED");
    while (GetQueuedCompletionStatus(port, &n, &key, &got, 0)) {}
    SetLastError(0);
    check(!CancelIoEx(server, &o) && GetLastError() == ERROR_NOT_FOUND, "nothing more to cancel: ERROR_NOT_FOUND");
    zero(&o, 16);
    ReadFile(server, buf, sizeof buf, 0, &o);
    CloseHandle(c);
    check(WaitForSingleObject(o.event, 1000) == 0 && !GetOverlappedResult(server, &o, &n, 0) && GetLastError() == ERROR_BROKEN_PIPE, "a pending read when the client closes: ERROR_BROKEN_PIPE");
    CloseHandle(port);
    CloseHandle(server);

    // A message pipe.
    server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-messages"), DUPLEX, TYPE_MESSAGE | READMODE_MESSAGE, 1, 0, 0, 0, 0);
    c = client("\\\\.\\pipe\\atlas-messages", 0);
    DWORD mode = READMODE_MESSAGE, flags = 0;
    check(SetNamedPipeHandleState(c, &mode, 0, 0) && GetNamedPipeInfo(server, &flags, 0, 0, 0) && flags == 5, "a message pipe: the client in message mode, the server end's flags");
    put(server, "one");
    put(server, "two!");
    int first_message = ReadFile(c, buf, sizeof buf, &n, 0) && n == 3;
    check(first_message && ReadFile(c, buf, sizeof buf, &n, 0) && n == 4 && same(buf, "two!", 4), "one message a read");
    put(server, "a long message");
    DWORD left = 0, avail = 0;
    PeekNamedPipe(c, buf, 4, &n, &avail, &left);
    check(n == 4 && avail == 14 && left == 10, "PeekNamedPipe: four bytes, and ten left of the message");
    SetLastError(0);
    int partial = !ReadFile(c, buf, 4, &n, 0) && GetLastError() == ERROR_MORE_DATA && n == 4 && same(buf, "a lo", 4);
    check(partial && ReadFile(c, buf, sizeof buf, &n, 0) && n == 10 && same(buf, "ng message", 10), "a message bigger than the buffer: ERROR_MORE_DATA, then the rest");
    CloseHandle(c);
    CloseHandle(server);
    DWORD byte_mode = READMODE_MESSAGE;
    server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-plain"), DUPLEX, 0, 1, 0, 0, 0, 0);
    c = client("\\\\.\\pipe\\atlas-plain", 0);
    SetLastError(0);
    check(!SetNamedPipeHandleState(c, &byte_mode, 0, 0) && GetLastError() == 87, "message mode on a byte pipe: ERROR_INVALID_PARAMETER");
    CloseHandle(c);
    CloseHandle(server);

    echo_pipe = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-echo"), DUPLEX, TYPE_MESSAGE | READMODE_MESSAGE, 1, 0, 0, 0, 0);
    HANDLE thread = CreateThread(0, 0, echo, 0, 0, 0);
    WaitNamedPipeW(W(0, "\\\\.\\pipe\\atlas-echo"), 1000);
    c = client("\\\\.\\pipe\\atlas-echo", 0);
    SetNamedPipeHandleState(c, &mode, 0, 0);
    char reply[80];
    check(TransactNamedPipe(c, "ping", 4, reply, sizeof reply, &n, 0) && n == 9 && same(reply, "echo:ping", 9), "TransactNamedPipe: a message, and its answer");
    CloseHandle(c);
    WaitNamedPipeW(W(0, "\\\\.\\pipe\\atlas-echo"), 2000);
    check(CallNamedPipeW(W(0, "\\\\.\\pipe\\atlas-echo"), "call", 4, reply, sizeof reply, &n, 2000) && n == 9 && same(reply, "echo:call", 9), "CallNamedPipe: connect, ask, answer, close");
    check(WaitForSingleObject(thread, 5000) == 0, "the server thread saw both clients through");
    CloseHandle(echo_pipe);

    // Another process.
    server = CreateNamedPipeW(W(0, "\\\\.\\pipe\\atlas-across"), DUPLEX | OVERLAPPED_FLAG, 0, 1, 0, 0, 0, 0);
    WCHAR self[260], cmd[300];
    GetModuleFileNameW(0, self, 260);
    int k = 0;
    cmd[k++] = '"';
    for (int i = 0; self[i]; i++) cmd[k++] = self[i];
    cmd[k++] = '"';
    const char *arg = " child";
    while (*arg) cmd[k++] = (WCHAR)*arg++;
    cmd[k] = 0;
    STARTUPINFOW si = {0};
    si.cb = sizeof si;
    PROCESS_INFORMATION pi;
    zero(&o, 16);
    ConnectNamedPipe(server, &o);
    check(CreateProcessW(0, cmd, 0, 0, 0, 0, 0, 0, &si, &pi) && WaitForSingleObject(o.event, 10000) == 0, "another process connects");
    zero(&o, 16);
    int ready = (ReadFile(server, buf, sizeof buf, &n, &o) || (GetLastError() == ERROR_IO_PENDING && GetOverlappedResult(server, &o, &n, 1))) && n == 5;
    check(ready && GetNamedPipeClientProcessId(server, &pid) && pid == pi.pid, "it's ready, and it's the client");
    Sleep(100);
    zero(&o, 16);
    WriteFile(server, "from parent", 11, &n, &o);
    zero(&o, 16);
    ReadFile(server, buf, sizeof buf, 0, &o);
    DWORD code = 99;
    WaitForSingleObject(pi.process, 10000);
    GetExitCodeProcess(pi.process, &code);
    check(code == 0, "this write completed its pending read, in its memory");
    check(WaitForSingleObject(o.event, 5000) == 0 && !GetOverlappedResult(server, &o, &n, 0) && GetLastError() == ERROR_BROKEN_PIPE, "it exited with its end open: this pending read ends, ERROR_BROKEN_PIPE");
    CloseHandle(server);

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
