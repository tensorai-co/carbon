// A freestanding 32-bit Windows test program for atlas's WoW64: built for i686 by tests/programs/build.sh. Everything
// pointer-sized is 4 bytes here, every call is stdcall, and SEH is a chain of frames from fs:[0].
//
// It checks: the TEB through fs and the PEB through it; GetSystemInfo's 32-bit layout; VirtualAlloc below 2 GiB; the
// heap, the last error, TLS and critical sections in ntdll; GetModuleHandle and GetProcAddress; a thread and a wait;
// an upcall (EnumSystemLocalesW); fibers; and exceptions: RaiseException to a vectored handler that carries on, a
// frame of our own on the fs:[0] chain that takes an exception and unwinds with RtlUnwind, and an access violation
// delivered by atlas. Then WoW64 as a 32-bit program sees it: IsWow64Process(2), GetNativeSystemInfo, the
// environment's architecture and Program Files, System32 redirected to SysWOW64 (and not, while it's turned off, or
// through Sysnative), the registry's 32-bit view, and a 64-bit program started from here (wowchild.c), which starts
// this one again as a 32-bit child (`wow64.exe child`, which checks what it sees and exits 32).

typedef void *HANDLE;
typedef unsigned long DWORD;
typedef unsigned long ULONG_PTR;
typedef unsigned long SIZE_T;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef BOOL(__stdcall *LOCALE_ENUMPROCW)(WCHAR *);

typedef struct {
    unsigned short arch, reserved;
    DWORD page_size;
    void *min_address, *max_address;
    ULONG_PTR processor_mask;
    DWORD processors, processor_type, granularity;
    unsigned short level, revision;
} SYSTEM_INFO;

typedef struct {
    DWORD code, flags;
    void *record, *address;
    DWORD count;
    ULONG_PTR information[15];
} EXCEPTION_RECORD;

typedef struct {
    EXCEPTION_RECORD *record;
    void *context;
} EXCEPTION_POINTERS;

typedef struct {
    void *debug;
    long lock_count, recursion;
    HANDLE owner, semaphore;
    ULONG_PTR spin;
} CRITICAL_SECTION;

typedef struct REGISTRATION {
    struct REGISTRATION *next;
    void *handler;
} REGISTRATION;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) void __stdcall GetSystemInfo(SYSTEM_INFO *);
__declspec(dllimport) void *__stdcall VirtualAlloc(void *, SIZE_T, DWORD, DWORD);
__declspec(dllimport) HANDLE __stdcall GetProcessHeap(void);
__declspec(dllimport) void *__stdcall HeapAlloc(HANDLE, DWORD, SIZE_T);
__declspec(dllimport) BOOL __stdcall HeapFree(HANDLE, DWORD, void *);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) void __stdcall SetLastError(DWORD);
__declspec(dllimport) DWORD __stdcall TlsAlloc(void);
__declspec(dllimport) BOOL __stdcall TlsSetValue(DWORD, void *);
__declspec(dllimport) void *__stdcall TlsGetValue(DWORD);
__declspec(dllimport) void __stdcall InitializeCriticalSection(CRITICAL_SECTION *);
__declspec(dllimport) void __stdcall EnterCriticalSection(CRITICAL_SECTION *);
__declspec(dllimport) void __stdcall LeaveCriticalSection(CRITICAL_SECTION *);
__declspec(dllimport) HANDLE __stdcall GetModuleHandleW(const WCHAR *);
__declspec(dllimport) void *__stdcall GetProcAddress(HANDLE, const char *);
__declspec(dllimport) HANDLE __stdcall CreateThread(void *, SIZE_T, DWORD(__stdcall *)(void *), void *, DWORD, DWORD *);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) BOOL __stdcall GetExitCodeThread(HANDLE, DWORD *);
__declspec(dllimport) BOOL __stdcall EnumSystemLocalesW(LOCALE_ENUMPROCW, DWORD);
__declspec(dllimport) void *__stdcall ConvertThreadToFiber(void *);
__declspec(dllimport) void *__stdcall CreateFiber(SIZE_T, void(__stdcall *)(void *), void *);
__declspec(dllimport) void __stdcall SwitchToFiber(void *);
__declspec(dllimport) void *__stdcall AddVectoredExceptionHandler(DWORD, long(__stdcall *)(EXCEPTION_POINTERS *));
__declspec(dllimport) DWORD __stdcall RemoveVectoredExceptionHandler(void *);
__declspec(dllimport) void __stdcall RaiseException(DWORD, DWORD, DWORD, const ULONG_PTR *);
__declspec(dllimport) void __stdcall RtlUnwind(void *, void *, EXCEPTION_RECORD *, void *);
__declspec(dllimport) BOOL __stdcall IsWow64Process(HANDLE, BOOL *);
__declspec(dllimport) BOOL __stdcall IsWow64Process2(HANDLE, unsigned short *, unsigned short *);
__declspec(dllimport) HANDLE __stdcall GetCurrentProcess(void);
__declspec(dllimport) void __stdcall GetNativeSystemInfo(SYSTEM_INFO *);
__declspec(dllimport) DWORD __stdcall GetSystemWow64DirectoryW(WCHAR *, DWORD);
__declspec(dllimport) DWORD __stdcall GetEnvironmentVariableW(const WCHAR *, WCHAR *, DWORD);
__declspec(dllimport) BOOL __stdcall Wow64DisableWow64FsRedirection(void **);
__declspec(dllimport) BOOL __stdcall Wow64RevertWow64FsRedirection(void *);
__declspec(dllimport) HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
__declspec(dllimport) BOOL __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
__declspec(dllimport) BOOL __stdcall CloseHandle(HANDLE);
__declspec(dllimport) WCHAR *__stdcall GetCommandLineW(void);
__declspec(dllimport) BOOL __stdcall CreateProcessW(const WCHAR *, WCHAR *, void *, void *, BOOL, DWORD, void *, const WCHAR *, void *, void *);
__declspec(dllimport) BOOL __stdcall GetExitCodeProcess(HANDLE, DWORD *);
__declspec(dllimport) long __stdcall RegCreateKeyExW(HANDLE, const WCHAR *, DWORD, WCHAR *, DWORD, DWORD, void *, HANDLE *, DWORD *);
__declspec(dllimport) long __stdcall RegOpenKeyExW(HANDLE, const WCHAR *, DWORD, DWORD, HANDLE *);
__declspec(dllimport) long __stdcall RegCloseKey(HANDLE);
__declspec(dllimport) long __stdcall RegDeleteKeyExW(HANDLE, const WCHAR *, DWORD, DWORD);
#define HKLM ((HANDLE)0x80000002)
#define KEY_WOW64_64KEY 0x100

static HANDLE out;
static int failures;

static unsigned len(const char *s) {
    unsigned n = 0;
    while (s[n]) n++;
    return n;
}
static void say(const char *s) {
    DWORD n;
    WriteFile(out, s, len(s), &n, 0);
}
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) failures++;
}

static int wsame(const WCHAR *a, const char *b) {
    while (*b && *a == (WCHAR)*b) a++, b++;
    return *a == 0 && *b == 0;
}

/// The value of an environment variable, as ASCII, or "" if there's none.
static const char *env(const WCHAR *name) {
    static char text[260];
    WCHAR value[260];
    DWORD n = GetEnvironmentVariableW(name, value, 260);
    for (DWORD i = 0; i < n && i < 259; i++) text[i] = (char)value[i];
    text[n < 259 ? n : 259] = 0;
    return text;
}
static int same(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}

/// The machine a PE says it's for, read through the file system as this thread sees it.
static unsigned short machine_of(const WCHAR *path) {
    HANDLE f = CreateFileW(path, 0x80000000, 1, 0, 3, 0, 0);
    if (f == (HANDLE)-1) return 0;
    unsigned char head[0x200];
    DWORD n = 0;
    ReadFile(f, head, sizeof head, &n, 0);
    CloseHandle(f);
    if (n < 0x40) return 0;
    DWORD pe = *(DWORD *)(head + 0x3c);
    return pe + 6 <= n ? *(unsigned short *)(head + pe + 4) : 0;
}

/// Runs a program with a command line, giving its exit code, or -1.
static DWORD run(const WCHAR *line) {
    WCHAR buffer[260];
    int i = 0;
    for (; line[i] && i < 259; i++) buffer[i] = line[i];
    buffer[i] = 0;
    DWORD si[17] = {68};
    struct { HANDLE process, thread; DWORD pid, tid; } pi;
    if (!CreateProcessW(0, buffer, 0, 0, 0, 0, 0, 0, si, &pi)) return (DWORD)-1;
    DWORD code = (DWORD)-1;
    WaitForSingleObject(pi.process, 0xffffffff);
    GetExitCodeProcess(pi.process, &code);
    CloseHandle(pi.thread);
    CloseHandle(pi.process);
    return code;
}

static void *teb(void) {
    void *t;
    __asm__ volatile("movl %%fs:0x18, %0" : "=r"(t));
    return t;
}

static DWORD __stdcall thread_main(void *p) { return (DWORD)(ULONG_PTR)p + 1; }

static int locales;
static BOOL __stdcall on_locale(WCHAR *name) { return ++locales < 3; }

static void *main_fiber;
static int fiber_ran;
static void __stdcall fiber_main(void *p) {
    fiber_ran = (int)(ULONG_PTR)p;
    SwitchToFiber(main_fiber);
}

static int vectored_seen;
static long __stdcall vectored(EXCEPTION_POINTERS *e) {
    if (e->record->code == 0xE0000001) {
        vectored_seen = e->record->count == 2 && e->record->information[1] == 0x5678;
        return -1; // EXCEPTION_CONTINUE_EXECUTION
    }
    return 0;
}

// A frame of our own on the fs:[0] chain. Its handler takes 0xE0000002, unwinds to its frame, and goes back to the
// point `recover` saved (its stack and where to carry on), as _except_handler does after an __except.
static volatile int frame_handled, frame_on_top;
static void *recover_esp, *recover_ebp;
static void (*recover_at)(void);

static int __cdecl frame_handler(EXCEPTION_RECORD *r, REGISTRATION *frame, void *context, void *dispatcher) {
    if (r->flags & 6) return 1; // unwinding: nothing to clean up
    if (r->code != 0xE0000002) return 1;
    frame_handled = 1;
    RtlUnwind(frame, 0, r, 0);
    __asm__ volatile("movl %0, %%esp\n movl %1, %%ebp\n jmp *%2" ::"r"(recover_esp), "r"(recover_ebp), "r"(recover_at));
    return 0;
}

static int av_seen;
static long __stdcall av_handler(EXCEPTION_POINTERS *e) {
    if (e->record->code != 0xC0000005) return 0;
    av_seen = e->record->information[1] == 0x10;
    // Skip the faulting `mov eax, [0x10]` (5 bytes) in the CONTEXT: Eip is at 0xb8.
    *(DWORD *)((char *)e->context + 0xb8) += 5;
    return -1;
}

static __attribute__((noinline)) void through_frame(void) {
    REGISTRATION frame;
    frame.handler = (void *)frame_handler;
    __asm__ volatile("movl %%fs:0, %%eax\n movl %%eax, %0\n leal %1, %%eax\n movl %%eax, %%fs:0" : "=m"(frame.next) : "m"(frame) : "eax");
    __asm__ volatile(
        "movl %%esp, %0\n movl %%ebp, %1\n movl $1f, %2\n"
        "pushl $0\n pushl $0\n pushl $0\n pushl $0xE0000002\n call *%3\n"
        "1:\n"
        : "=m"(recover_esp), "=m"(recover_ebp), "=m"(recover_at)
        : "r"(RaiseException)
        : "eax", "ecx", "edx", "ebx", "esi", "edi", "memory");
    // RtlUnwind called and unlinked every frame above ours, and left ours: it comes off as the function leaves, as
    // compiled code takes its own frame off the chain.
    frame_on_top = *(REGISTRATION *volatile *)teb() == &frame;
    __asm__ volatile("movl %0, %%eax\n movl %%eax, %%fs:0" ::"m"(frame.next) : "eax");
}

/// `wow64.exe child`: started by a 64-bit program, it sees what a 32-bit one should, and says so with its exit code.
static void child(void) {
    BOOL wow = 0;
    IsWow64Process(GetCurrentProcess(), &wow);
    int ok = wow && same(env(L"PROCESSOR_ARCHITECTURE"), "x86") && same(env(L"PROCESSOR_ARCHITEW6432"), "AMD64") &&
             same(env(L"ProgramFiles"), "C:\\Program Files (x86)");
    ExitProcess(ok ? 32 : 1);
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    const WCHAR *line = GetCommandLineW();
    for (const WCHAR *p = line; *p; p++) {
        if (p[0] == ' ' && wsame(p + 1, "child")) child();
    }
    check(sizeof(void *) == 4, "pointers are 4 bytes");

    char *t = teb();
    check(*(void **)(t + 0x18) == t, "fs:[0x18] is the TEB, which points at itself");
    check(*(DWORD *)(t + 0) == 0xffffffff, "the SEH chain starts empty (-1)");
    char *peb = *(char **)(t + 0x30);
    check(peb != 0 && *(HANDLE *)(peb + 8) == GetModuleHandleW(0), "the PEB's image base is the program's module");
    check(*(DWORD *)(peb + 0xa4) == 10 && *(DWORD *)(peb + 0xa8) == 0, "the PEB says Windows 10");
    check((ULONG_PTR)t < 0x80000000 && (ULONG_PTR)peb < 0x80000000, "the TEB and PEB are below 2 GiB");

    SYSTEM_INFO si;
    GetSystemInfo(&si);
    check(si.arch == 0 && si.page_size == 4096 && si.granularity == 0x10000, "GetSystemInfo: x86, 4 KiB pages, 64 KiB granularity");
    check(si.processors >= 1 && (ULONG_PTR)si.max_address < 0x80000000, "GetSystemInfo: processors, and addresses below 2 GiB");

    char *mem = VirtualAlloc(0, 1 << 20, 0x3000, 4);
    check(mem && (ULONG_PTR)mem < 0x80000000 && ((ULONG_PTR)mem & 0xffff) == 0, "VirtualAlloc: 64 KiB aligned, below 2 GiB");
    mem[12345] = 7;
    check(mem[12345] == 7 && mem[0] == 0, "VirtualAlloc'd memory, committed on touch");

    HANDLE heap = GetProcessHeap();
    int *a = HeapAlloc(heap, 8, 100 * sizeof(int));
    int zero = 1;
    for (int i = 0; i < 100; i++) zero &= a[i] == 0;
    check(a && zero && HeapFree(heap, 0, a), "the heap: zeroed, freed");

    SetLastError(1234);
    check(GetLastError() == 1234 && *(DWORD *)(t + 0x34) == 1234, "the last error, at TEB+0x34");

    DWORD slot = TlsAlloc();
    TlsSetValue(slot, (void *)0x1234);
    check(TlsGetValue(slot) == (void *)0x1234 && *(void **)(t + 0xe10 + slot * 4) == (void *)0x1234, "TLS, in the TEB's slots at 0xe10");

    CRITICAL_SECTION cs;
    InitializeCriticalSection(&cs);
    EnterCriticalSection(&cs);
    EnterCriticalSection(&cs);
    check(cs.recursion == 2, "a critical section, entered twice");
    LeaveCriticalSection(&cs);
    LeaveCriticalSection(&cs);

    HANDLE k32 = GetModuleHandleW(L"kernel32.dll");
    check(k32 && GetProcAddress(k32, "WriteFile") == (void *)WriteFile, "GetModuleHandle and GetProcAddress");

    DWORD id, code = 0;
    HANDLE th = CreateThread(0, 0, thread_main, (void *)41, 0, &id);
    check(th && WaitForSingleObject(th, 0xffffffff) == 0 && GetExitCodeThread(th, &code) && code == 42, "a thread, waited for, with its exit code");

    EnumSystemLocalesW(on_locale, 2);
    check(locales == 3, "an upcall: EnumSystemLocalesW's callback, until it says stop");

    main_fiber = ConvertThreadToFiber(0);
    void *f = CreateFiber(0, fiber_main, (void *)5);
    SwitchToFiber(f);
    check(fiber_ran == 5, "a fiber, and back");

    void *h = AddVectoredExceptionHandler(1, vectored);
    ULONG_PTR args[2] = {0x1234, 0x5678};
    RaiseException(0xE0000001, 0, 2, args);
    RemoveVectoredExceptionHandler(h);
    check(vectored_seen, "RaiseException to a vectored handler, which carries on");

    through_frame();
    check(frame_handled && frame_on_top, "a frame on the fs:[0] chain takes an exception, and RtlUnwind unwinds to it");
    check(*(DWORD *)t == 0xffffffff, "the chain is empty again once the frame's function has left");

    h = AddVectoredExceptionHandler(1, av_handler);
    __asm__ volatile("movl 0x10, %%eax" ::: "eax");
    RemoveVectoredExceptionHandler(h);
    check(av_seen, "an access violation, delivered by atlas, carried on from past it");

    // WoW64, as a 32-bit program sees it.
    BOOL wow = 0;
    unsigned short process_machine = 0, native_machine = 0;
    check(IsWow64Process(GetCurrentProcess(), &wow) && wow, "IsWow64Process: yes");
    check(IsWow64Process2(GetCurrentProcess(), &process_machine, &native_machine) && process_machine == 0x14c && native_machine == 0x8664,
          "IsWow64Process2: I386 under AMD64");
    SYSTEM_INFO native;
    GetNativeSystemInfo(&native);
    check(native.arch == 9 && si.arch == 0, "GetNativeSystemInfo says x64; GetSystemInfo, x86");
    WCHAR dir[64];
    check(GetSystemWow64DirectoryW(dir, 64) == 19 && wsame(dir, "C:\\Windows\\SysWOW64"), "GetSystemWow64Directory: SysWOW64");
    check(same(env(L"PROCESSOR_ARCHITECTURE"), "x86") && same(env(L"PROCESSOR_ARCHITEW6432"), "AMD64"), "PROCESSOR_ARCHITECTURE x86, PROCESSOR_ARCHITEW6432 AMD64");
    check(same(env(L"ProgramFiles"), "C:\\Program Files (x86)") && same(env(L"ProgramW6432"), "C:\\Program Files") &&
              same(env(L"CommonProgramFiles"), "C:\\Program Files (x86)\\Common Files"),
          "ProgramFiles is the (x86) one; ProgramW6432 the other");
    check(machine_of(L"C:\\Windows\\System32\\kernel32.dll") == 0x14c, "System32's kernel32.dll is SysWOW64's: I386");
    void *old = 0;
    check(Wow64DisableWow64FsRedirection(&old) && machine_of(L"C:\\Windows\\System32\\kernel32.dll") == 0x8664, "  with redirection off, System32's own: AMD64");
    check(Wow64RevertWow64FsRedirection(old) && machine_of(L"C:\\Windows\\System32\\kernel32.dll") == 0x14c, "  and back on");
    check(machine_of(L"C:\\Windows\\Sysnative\\kernel32.dll") == 0x8664, "Sysnative is System32");
    HANDLE key = 0, seen = 0;
    DWORD made = 0;
    RegCreateKeyExW(HKLM, L"SOFTWARE\\CarbonWow64Test", 0, 0, 0, 0xf003f, 0, &key, &made);
    check(key && RegOpenKeyExW(HKLM, L"SOFTWARE\\WOW6432Node\\CarbonWow64Test", 0, 0x20019 | KEY_WOW64_64KEY, &seen) == 0,
          "the registry: HKLM\\SOFTWARE's keys are WOW6432Node's");
    RegCloseKey(seen);
    seen = 0;
    check(RegOpenKeyExW(HKLM, L"SOFTWARE\\CarbonWow64Test", 0, 0x20019 | KEY_WOW64_64KEY, &seen) != 0, "  not the 64-bit view's");
    RegCloseKey(key);
    RegDeleteKeyExW(HKLM, L"SOFTWARE\\CarbonWow64Test", 0, 0);
    check(run(L"wowchild.exe") == 64, "a 64-bit program started from here, which starts a 32-bit one");

    say(failures ? "some checks FAILED\n" : "all passed\n");
    ExitProcess(failures);
}
