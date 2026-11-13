// ntdll's loader and structured exception handling, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: an app's own DLL, imported and relocated off a base the program has; its DllMain; a forwarder chain
// (mylib!heap_alloc → kernel32!HeapAlloc → ntdll!RtlAllocateHeap); LoadLibraryA and GetProcAddress by name and by
// ordinal; ERROR_MOD_NOT_FOUND and ERROR_PROC_NOT_FOUND; GetModuleFileNameW; static TLS and a TLS callback;
// the PEB's three module lists; GetModuleHandleExA; FreeLibrary counting references and unloading, with DllMain
// told and a DLL notification, and what only it held unloaded too; DLL_THREAD_DETACH; an API set; LdrGetDllHandle
// and LdrGetProcedureAddress; a delay-loaded DLL; __try/__except catching an access violation, from three calls down
// too; RaiseException with a filter; __finally during an unwind and on a normal exit; EXCEPTION_CONTINUE_EXECUTION;
// integer divide by zero (x86-64); and, as it ends, DLL_PROCESS_DETACH

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
__declspec(dllimport) int __stdcall FreeLibrary(HANDLE);
__declspec(dllimport) int __stdcall GetModuleHandleExA(DWORD, const char *, HANDLE *);
__declspec(dllimport) HANDLE __stdcall CreateThread(void *, u64, DWORD (__stdcall *)(void *), void *, DWORD, DWORD *);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) int __stdcall CloseHandle(HANDLE);
__declspec(dllimport) void *__stdcall ResolveDelayLoadedAPI(void *, const void *, void *, void *, void **, DWORD);

typedef struct { unsigned short length, max; unsigned pad; const WCHAR *buffer; } USTR;
typedef struct { unsigned short length, max; unsigned pad; const char *buffer; } ASTR;
__declspec(dllimport) int __stdcall LdrGetDllHandle(void *, void *, const USTR *, HANDLE *);
__declspec(dllimport) int __stdcall LdrGetProcedureAddress(HANDLE, const ASTR *, DWORD, void **);
__declspec(dllimport) int __stdcall LdrRegisterDllNotification(DWORD, void *, void *, void **);
__declspec(dllimport) int __stdcall LdrUnregisterDllNotification(void *);

__declspec(dllimport) int add(int, int);
__declspec(dllimport) const char *word(int);
__declspec(dllimport) int attached;
__declspec(dllimport) void *heap_alloc(HANDLE, DWORD, u64);
__declspec(dllimport) int thread_attached, thread_detached, say_at_exit;
__declspec(dllimport) int square(int);

// The delay-load helper MSVC's delayimp.lib has, as Windows' own DLLs have it: everything is ResolveDelayLoadedAPI.
extern char __ImageBase;
void *__delayLoadHelper2(const void *descriptor, void **thunk) { return ResolveDelayLoadedAPI(&__ImageBase, descriptor, 0, 0, thunk, 0); }

// The PEB's lists: an LDR_DATA_TABLE_ENTRY's three links, then its base, entry point, size and names.
typedef struct LIST { struct LIST *next, *prev; } LIST;
typedef struct { LIST load, memory, init; void *base, *entry; DWORD size, pad; USTR full, name; } ENTRY;
static LIST *ldr_list(int which) {
    char *teb;
#if defined(__aarch64__)
    __asm__ volatile("mov %0, x18" : "=r"(teb));
#else
    __asm__ volatile("mov %%gs:0x30, %0" : "=r"(teb));
#endif
    char *peb = *(char **)(teb + 0x60), *ldr = *(char **)(peb + 0x18);
    return (LIST *)(ldr + 0x10 + which * 0x10);
}
static ENTRY *entry_of(LIST *links, int which) { return (ENTRY *)((char *)links - which * 0x10); }
static int named(const USTR *s, const char *want) {
    unsigned n = s->length / 2, i = 0;
    for (; i < n && want[i]; i++) {
        WCHAR c = s->buffer[i];
        if (c >= 'A' && c <= 'Z') c += 32;
        if (c != (WCHAR)want[i]) return 0;
    }
    return i == n && !want[i];
}
static ENTRY *nth(int which, int index) {
    LIST *head = ldr_list(which), *at = head->next;
    for (int i = 0; at != head; at = at->next, i++)
        if (i == index) return entry_of(at, which);
    return 0;
}
static int listed(int which, HANDLE base) {
    LIST *head = ldr_list(which);
    for (LIST *at = head->next; at != head; at = at->next)
        if (entry_of(at, which)->base == base) return 1;
    return 0;
}
static int count_of(int which) { int n = 0; LIST *head = ldr_list(which); for (LIST *at = head->next; at != head; at = at->next) n++; return n; }

static int loads, unloads;
typedef struct { DWORD flags; const USTR *full, *name; void *base; DWORD size; } NOTIFIED;
static void __stdcall on_dll(DWORD reason, const NOTIFIED *data, void *context) {
    if (context != (void *)&loads || !named(data->name, "mylib3.dll")) return;
    if (reason == 1) loads = loads + 1;
    if (reason == 2) unloads = unloads + 1;
}
static DWORD __stdcall nothing(void *p) { (void)p; return 7; }

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

    // The PEB's lists: load order is the program, ntdll and kernel32 first; initialization order starts at ntdll.
    ENTRY *first = nth(0, 0), *second = nth(0, 1), *third = nth(0, 2), *init = nth(2, 0);
    check(first && first->base == self && second && named(&second->name, "ntdll.dll") && third && named(&third->name, "kernel32.dll"),
          "the PEB's load order list: the program, ntdll, kernel32");
    check(init && named(&init->name, "ntdll.dll") && listed(2, lib) && !listed(2, self), "its initialization order list: ntdll first, DLLs only");
    check(count_of(0) == count_of(1) && listed(1, two) && first->size > 0, "its memory order list has the same modules");

    HANDLE found = 0;
    check(GetModuleHandleExA(4 | 2, (const char *)(void *)add, &found) && found == lib, "GetModuleHandleExA, from an address in the module");

    void *cookie = 0;
    check(LdrRegisterDllNotification(0, (void *)on_dll, &loads, &cookie) == 0 && cookie, "LdrRegisterDllNotification");
    HANDLE three = LoadLibraryA("mylib3.dll");
    void (*watch)(int *) = (void (*)(int *))GetProcAddress(three, "watch");
    int (*triple)(int) = (int (*)(int))GetProcAddress(three, "triple");
    int detached = 0;
    if (watch) watch(&detached);
    check(three && triple && triple(5) == 15 && loads == 1, "a DLL that imports another, loaded, and its notification");
    check(FreeLibrary(three) && detached == 1 && unloads == 1, "FreeLibrary of its last reference: DLL_PROCESS_DETACH, then a notification");
    check(GetModuleHandleA("mylib3.dll") == 0 && !listed(0, three) && !listed(2, three), "and it's gone, from the lists too");
    LdrUnregisterDllNotification(cookie);
    // mylib2 was loaded twice, and mylib3 held it a third time.
    check(GetModuleHandleA("mylib2.dll") == two && FreeLibrary(two), "what it imported stays while others hold it");
    check(GetModuleHandleA("mylib2.dll") == two && FreeLibrary(two) && GetModuleHandleA("mylib2.dll") == 0, "and goes with its last reference");
    two = LoadLibraryA("mylib2.dll");
    mul = (int (*)(int, int))GetProcAddress(two, "mul");
    check(two && mul && mul(4, 5) == 20, "a DLL unloaded loads again");
    check(FreeLibrary((HANDLE)0x1234) == 0 && GetLastError() == 126, "FreeLibrary of something that isn't a module fails");

    int before = thread_detached;
    HANDLE t = CreateThread(0, 0, nothing, 0, 0, 0);
    WaitForSingleObject(t, 0xffffffff);
    CloseHandle(t);
    check(thread_attached >= 1 && thread_detached == before + 1, "a thread ending: DLL_THREAD_DETACH");

    HANDLE set = LoadLibraryA("api-ms-win-core-synch-l1-2-0.dll");
    check(set && set == GetModuleHandleA("kernel32.dll") && GetProcAddress(set, "Sleep"), "an API set is its DLL");

    static const WCHAR wide[] = {'M', 'y', 'L', 'i', 'b', '.', 'd', 'l', 'l'};
    USTR lib_name = {sizeof wide, sizeof wide, 0, wide};
    ASTR add_name = {3, 3, 0, "addXX"};
    HANDLE by_ldr = 0;
    void *add_at = 0;
    check(LdrGetDllHandle(0, 0, &lib_name, &by_ldr) == 0 && by_ldr == lib, "LdrGetDllHandle");
    check(LdrGetProcedureAddress(lib, &add_name, 0, &add_at) == 0 && add_at == GetProcAddress(lib, "add"), "LdrGetProcedureAddress, by a counted name");
    check(LdrGetProcedureAddress(two, 0, 5, &add_at) == 0 && add_at == (void *)mul, "and by ordinal");

    check(GetModuleHandleA("mylib4.dll") == 0, "a delay-loaded DLL isn't loaded at first");
    check(square(9) == 81 && GetModuleHandleA("mylib4.dll") != 0 && square(4) == 16, "and is the first time it's called");

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

    say_at_exit = 1;
    ExitProcess(failures);
}
