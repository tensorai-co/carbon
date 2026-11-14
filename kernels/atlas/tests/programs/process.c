// Processes and pipes on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// The program starts copies of itself, each told what to do by its command line ("process.exe exit 42", …), and
// checks:
// - anonymous pipes in one process: reads, peeks, a broken pipe and one with no reader;
// - CreateProcessW finding the program (by path, and by name with `.exe` added), with ERROR_FILE_NOT_FOUND where
//   there's none;
// - exit codes and waiting on a process; a child's parent ID, as NtQueryInformationProcess gives it; an ended
//   process's times, by a handle to it;
// - a child's standard output redirected into a pipe, with an environment and a current directory of its own;
// - inherited handles at the same values, and a handle that isn't inheritable left behind;
// - a named event shared between processes;
// - a child started suspended and resumed;
// - TerminateProcess, by the handle CreateProcess gave and by one from OpenProcess;
// - a write much bigger than the pipe, read all the way to its end.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned long long u64;

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
K DWORD __stdcall GetFileType(HANDLE);
K int __stdcall CreatePipe(HANDLE *, HANDLE *, SECURITY_ATTRIBUTES *, DWORD);
K int __stdcall PeekNamedPipe(HANDLE, void *, DWORD, DWORD *, DWORD *, DWORD *);
K int __stdcall CreateProcessW(const WCHAR *, WCHAR *, SECURITY_ATTRIBUTES *, SECURITY_ATTRIBUTES *, int, DWORD, void *,
                               const WCHAR *, STARTUPINFOW *, PROCESS_INFORMATION *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K int __stdcall GetExitCodeProcess(HANDLE, DWORD *);
K int __stdcall GetProcessTimes(HANDLE, unsigned long long *, unsigned long long *, unsigned long long *, unsigned long long *);
K int __stdcall TerminateProcess(HANDLE, unsigned);
K HANDLE __stdcall OpenProcess(DWORD, int, DWORD);
K DWORD __stdcall GetProcessId(HANDLE);
K DWORD __stdcall GetCurrentProcessId(void);
K void *__stdcall GetModuleHandleA(const char *);
K void *__stdcall GetProcAddress(void *, const char *);
K DWORD __stdcall ResumeThread(HANDLE);
K HANDLE __stdcall CreateEventW(SECURITY_ATTRIBUTES *, int, int, const WCHAR *);
K HANDLE __stdcall OpenEventW(DWORD, int, const WCHAR *);
K int __stdcall SetEvent(HANDLE);
K void __stdcall Sleep(DWORD);
K int __stdcall GetHandleInformation(HANDLE, DWORD *);
K int __stdcall SetHandleInformation(HANDLE, DWORD, DWORD);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K char *__stdcall GetCommandLineA(void);
K DWORD __stdcall GetEnvironmentVariableA(const char *, char *, DWORD);
K DWORD __stdcall GetCurrentDirectoryA(DWORD, char *);

enum { CREATE_SUSPENDED = 4, CREATE_UNICODE_ENVIRONMENT = 0x400, STARTF_USESTDHANDLES = 0x100, STILL_ACTIVE = 259,
       WAIT_TIMEOUT = 0x102, ERROR_FILE_NOT_FOUND = 2, ERROR_BROKEN_PIPE = 109, ERROR_NO_DATA = 232 };

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
static int eq(const char *a, const char *b) { while (*b && *a == *b) { a++; b++; } return *a == 0 && *b == 0; }
static int starts(const char *a, const char *b) { while (*b && *a == *b) { a++; b++; } return *b == 0; }
static char *number(char *at, u64 n) {
    char digits[24];
    int i = 0;
    do { digits[i++] = (char)('0' + n % 10); n /= 10; } while (n);
    while (i) *at++ = digits[--i];
    *at = 0;
    return at;
}
static u64 parse(const char *s) { u64 n = 0; while (*s >= '0' && *s <= '9') n = n * 10 + (u64)(*s++ - '0'); return n; }
static char *append(char *at, const char *s) { while (*s) *at++ = *s++; *at = 0; return at; }
/// The command line's argument after the program's name, and after that.
static const char *argument(int which) {
    const char *s = GetCommandLineA();
    for (int i = 0; i <= which; i++) {
        if (*s == '"') { s++; while (*s && *s != '"') s++; if (*s) s++; }
        else while (*s && *s != ' ') s++;
        while (*s == ' ') s++;
    }
    return s;
}

/// What a child does, by its first argument.
static void child(const char *what) {
    if (starts(what, "exit ")) ExitProcess((unsigned)parse(what + 5));
    if (starts(what, "parent")) {
        // PROCESS_BASIC_INFORMATION's last field: the parent's ID, as the exit code.
        typedef long(__stdcall * QUERY)(void *, int, void *, unsigned, unsigned *);
        QUERY query = (QUERY)GetProcAddress(GetModuleHandleA("ntdll.dll"), "NtQueryInformationProcess");
        __UINTPTR_TYPE__ info[6] = {0};
        unsigned got = 0;
        long status = query ? query((void *)-1, 0, info, sizeof info, &got) : -1;
        ExitProcess(status == 0 && got == sizeof info ? (unsigned)info[5] : 0);
    }
    if (starts(what, "echo")) {
        // Its first argument, its ATLAS_TEST variable, and its current directory, to its standard output.
        char line[512], *at = line;
        at = append(at, "echo|");
        char value[64] = "";
        GetEnvironmentVariableA("ATLAS_TEST", value, sizeof value);
        at = append(at, value);
        at = append(at, "|");
        at += GetCurrentDirectoryA(256, at);
        DWORD n;
        WriteFile(GetStdHandle((DWORD)-11), line, (DWORD)(at - line), &n, 0);
        ExitProcess(0);
    }
    if (starts(what, "signal ")) {
        // An inherited event, by its value, and one that wasn't inheritable, which isn't here.
        HANDLE given = (HANDLE)parse(what + 7);
        const char *rest = what + 7;
        while (*rest && *rest != ' ') rest++;
        HANDLE left = (HANDLE)parse(rest + 1);
        DWORD flags = 0;
        int inherited = GetHandleInformation(given, &flags) && flags == 1;
        int absent = !GetHandleInformation(left, &flags);
        ExitProcess(inherited && absent && SetEvent(given) ? 0 : 1);
    }
    if (starts(what, "named")) {
        WCHAR name[] = {'a', 't', 'l', 'a', 's', '-', 'p', 'r', 'o', 'c', 'e', 's', 's', 0};
        HANDLE event = OpenEventW(2, 0, name);
        ExitProcess(event && SetEvent(event) ? 0 : 1);
    }
    if (starts(what, "sleep")) { Sleep(0xffffffff); ExitProcess(1); }
    if (starts(what, "write")) {
        // 100,000 bytes in one write, into a pipe that holds 4,096.
        static char big[100000];
        for (int i = 0; i < (int)sizeof big; i++) big[i] = (char)('a' + i % 26);
        DWORD n = 0;
        int ok = WriteFile(GetStdHandle((DWORD)-11), big, sizeof big, &n, 0);
        ExitProcess(ok && n == sizeof big ? 0 : 1);
    }
    ExitProcess(99);
}

static WCHAR self[260];
static WCHAR wide[1024];
/// A command line for a copy of this program: its own path, quoted, then `rest`.
static WCHAR *line_for(const char *rest) {
    int n = 0;
    wide[n++] = '"';
    for (int i = 0; self[i]; i++) wide[n++] = self[i];
    wide[n++] = '"';
    wide[n++] = ' ';
    while (*rest) wide[n++] = (WCHAR)*rest++;
    wide[n] = 0;
    return wide;
}
static WCHAR *W(const char *s) {
    static WCHAR buf[256];
    int n = 0;
    for (; s[n]; n++) buf[n] = (WCHAR)s[n];
    buf[n] = 0;
    return buf;
}
static PROCESS_INFORMATION pi;
static int start(WCHAR *line, int inherit, DWORD flags, void *env, const WCHAR *dir, HANDLE std_out) {
    STARTUPINFOW si = {0};
    si.cb = sizeof si;
    if (std_out) {
        si.flags = STARTF_USESTDHANDLES;
        si.std_input = GetStdHandle((DWORD)-10);
        si.std_output = std_out;
        si.std_error = std_out;
    }
    return CreateProcessW(0, line, 0, 0, inherit, flags, env, dir, &si, &pi);
}
static DWORD finish(void) {
    DWORD code = 12345;
    WaitForSingleObject(pi.process, 10000);
    GetExitCodeProcess(pi.process, &code);
    CloseHandle(pi.process);
    CloseHandle(pi.thread);
    return code;
}
/// Everything a pipe has until its write ends are all closed; ReadFile's last error then.
static DWORD read_all(HANDLE pipe, char *buffer, DWORD size, DWORD *error) {
    DWORD total = 0, n;
    while (ReadFile(pipe, buffer + (total < size ? total : 0), total < size ? size - total : size, &n, 0)) total += n;
    *error = GetLastError();
    return total;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    const char *what = argument(0);
    if (*what) child(what);
#if defined(__aarch64__)
    say("processes and pipes on atlas, arm64\n");
#elif defined(__i386__)
    say("processes and pipes on atlas, x86 (WoW64)\n");
#else
    say("processes and pipes on atlas, x86-64\n");
#endif
    GetModuleFileNameW(0, self, 260);
    SECURITY_ATTRIBUTES inheritable = {sizeof inheritable, 0, 1};

    // A pipe in this process.
    HANDLE r, w;
    DWORD n = 0, avail = 0, error;
    char buf[256];
    check(CreatePipe(&r, &w, 0, 0) && GetFileType(r) == 3, "CreatePipe; FILE_TYPE_PIPE");
    WriteFile(w, "abc", 3, &n, 0);
    check(PeekNamedPipe(r, buf, 2, &n, &avail, 0) && n == 2 && avail == 3 && buf[0] == 'a', "PeekNamedPipe: two of three bytes, none taken");
    check(ReadFile(r, buf, sizeof buf, &n, 0) && n == 3 && buf[2] == 'c', "ReadFile: all three");
    CloseHandle(w);
    SetLastError(0);
    check(!ReadFile(r, buf, sizeof buf, &n, 0) && GetLastError() == ERROR_BROKEN_PIPE && n == 0, "the writer closed: ERROR_BROKEN_PIPE");
    CloseHandle(r);
    CreatePipe(&r, &w, 0, 0);
    CloseHandle(r);
    SetLastError(0);
    check(!WriteFile(w, "x", 1, &n, 0) && GetLastError() == ERROR_NO_DATA, "the reader closed: ERROR_NO_DATA");
    CloseHandle(w);
    DWORD flags = 9;
    CreatePipe(&r, &w, 0, 0);
    int before = GetHandleInformation(r, &flags) && flags == 0;
    check(before && SetHandleInformation(r, 1, 1) && GetHandleInformation(r, &flags) && flags == 1, "Get/SetHandleInformation: HANDLE_FLAG_INHERIT");
    CloseHandle(r);
    CloseHandle(w);

    // Exit codes; the program found by its path, and by its name.
    check(start(line_for("exit 42"), 0, 0, 0, 0, 0) && pi.pid != GetCurrentProcessId() && GetProcessId(pi.process) == pi.pid, "CreateProcessW: a new process, its own ID");
    check(finish() == 42, "its exit code, once it's over");
    check(start(line_for("parent"), 0, 0, 0, 0, 0) && finish() == GetCurrentProcessId(), "its parent's ID is this process's");
    {
        unsigned long long created = 0, exited = 0, kernel = 0, user = 0;
        int started = start(line_for("exit 7"), 0, 0, 0, 0, 0);
        WaitForSingleObject(pi.process, 10000);
        int got = started && GetProcessTimes(pi.process, &created, &exited, &kernel, &user);
        check(got && created != 0 && exited >= created, "an ended process's times, by its handle: when it started and ended");
        finish();
    }
    WCHAR by_name[] = {'p', 'r', 'o', 'c', 'e', 's', 's', ' ', 'e', 'x', 'i', 't', ' ', '3', 0};
    check(start(by_name, 0, 0, 0, 0, 0) && finish() == 3, "found by name, .exe added, in the program's folder");
    WCHAR none[] = {'n', 'o', 's', 'u', 'c', 'h', '.', 'e', 'x', 'e', 0};
    SetLastError(0);
    check(!start(none, 0, 0, 0, 0, 0) && GetLastError() == ERROR_FILE_NOT_FOUND, "no such program: ERROR_FILE_NOT_FOUND");

    // Standard output into a pipe, with an environment and a current directory of its own.
    CreatePipe(&r, &w, &inheritable, 0);
    WCHAR env[] = {'A', 'T', 'L', 'A', 'S', '_', 'T', 'E', 'S', 'T', '=', 'y', 'e', 's', 0, 0};
    WCHAR dir[] = {'C', ':', '\\', 'W', 'i', 'n', 'd', 'o', 'w', 's', 0};
    int started = start(line_for("echo"), 1, CREATE_UNICODE_ENVIRONMENT, env, dir, w);
    CloseHandle(w);
    n = read_all(r, buf, sizeof buf - 1, &error);
    buf[n] = 0;
    check(started && eq(buf, "echo|yes|C:\\Windows") && error == ERROR_BROKEN_PIPE, "a child's output through a pipe: its environment and directory, then the pipe's end");
    check(finish() == 0, "and it exited 0");
    CloseHandle(r);

    // Inheritance: an inheritable event at the same value, and one that isn't, left behind.
    HANDLE given = CreateEventW(&inheritable, 1, 0, 0), kept = CreateEventW(0, 1, 0, 0);
    char text[64], *at = append(text, "signal ");
    at = number(at, (u64)given);
    at = append(at, " ");
    number(at, (u64)kept);
    started = start(line_for(text), 1, 0, 0, 0, 0);
    check(started && WaitForSingleObject(given, 10000) == 0 && finish() == 0, "an inherited event, set by the child; the other not inherited");
    CloseHandle(given);
    CloseHandle(kept);

    // A named event, shared.
    HANDLE named = CreateEventW(0, 1, 0, W("atlas-process"));
    check(start(line_for("named"), 0, 0, 0, 0, 0) && WaitForSingleObject(named, 10000) == 0 && finish() == 0, "a named event opened and set by another process");
    CloseHandle(named);

    // Suspended, then resumed.
    CreatePipe(&r, &w, &inheritable, 0);
    started = start(line_for("echo"), 1, CREATE_SUSPENDED, 0, 0, w);
    CloseHandle(w);
    Sleep(200);
    DWORD code = 0;
    int waiting = PeekNamedPipe(r, 0, 0, 0, &avail, 0) && avail == 0 && GetExitCodeProcess(pi.process, &code) && code == STILL_ACTIVE;
    check(started && waiting, "CREATE_SUSPENDED: nothing of it runs");
    check(ResumeThread(pi.thread) == 1, "ResumeThread: it was suspended once");
    n = read_all(r, buf, sizeof buf - 1, &error);
    buf[n] = 0;
    check(starts(buf, "echo||") && finish() == 0, "then it runs");
    CloseHandle(r);

    // TerminateProcess, by its handle and by OpenProcess's.
    check(start(line_for("sleep"), 0, 0, 0, 0, 0), "a child that sleeps");
    Sleep(50);
    check(WaitForSingleObject(pi.process, 0) == WAIT_TIMEOUT && TerminateProcess(pi.process, 7) && finish() == 7, "TerminateProcess: its exit code");
    start(line_for("sleep"), 0, 0, 0, 0, 0);
    HANDLE opened = OpenProcess(0x1fffff, 0, pi.pid);
    check(opened && GetProcessId(opened) == pi.pid && TerminateProcess(opened, 8) && finish() == 8, "OpenProcess by ID, and TerminateProcess through it");
    CloseHandle(opened);
    SetLastError(0);
    check(!OpenProcess(0x1fffff, 0, 0x7ffffff0) && GetLastError() == 87, "OpenProcess of no such process: ERROR_INVALID_PARAMETER");

    // A write much bigger than the pipe.
    CreatePipe(&r, &w, &inheritable, 0);
    started = start(line_for("write"), 1, 0, 0, 0, w);
    CloseHandle(w);
    static char big[100000];
    n = read_all(r, big, sizeof big, &error);
    int right = n == sizeof big && big[0] == 'a' && big[99999] == (char)('a' + 99999 % 26);
    check(started && right && error == ERROR_BROKEN_PIPE && finish() == 0, "100,000 bytes through a 4,096-byte pipe, in order, then its end");
    CloseHandle(r);

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
