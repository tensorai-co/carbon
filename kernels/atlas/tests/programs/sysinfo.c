// System information on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it twice:
// sysinfo.exe, with no manifest, and sysinfo10.exe (WIN10 defined), whose manifest says it supports Windows 10.
//
// - KUSER_SHARED_DATA at 0x7ffe0000: the version, the processor, its features and count as the calls give them, the
//   clocks moving, and the page refusing VirtualProtect and VirtualFree;
// - the version: RtlGetVersion's truth, GetVersionEx's Windows 8 for a program without the manifest (and 10 with it),
//   GetVersion, VerifyVersionInfo and VerSetConditionMask;
// - the machine: GetSystemInfo, GlobalMemoryStatusEx, IsWow64Process(2), the processor counts, and
//   GetLogicalProcessorInformation(Ex)'s records;
// - its name, and the time: SYSTEMTIME and FILETIME both ways, the time zone, and a zone's daylight saving;
// - NtQuerySystemInformation: basic, processor, performance, processor times, the time of day, the process list (with
//   a suspended child, its parent and its threads), both handle lists (an event's handle and its duplicate, the same
//   object), the kernel's modules, the debugger, and the lengths it wants; and GetSystemTimes.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned char BYTE;
typedef unsigned long long u64;
typedef long long i64;

typedef struct {
    DWORD size, major, minor, build, platform;
    WCHAR csd[128];
    WORD sp_major, sp_minor, suite;
    BYTE product, reserved;
} OSVERSIONINFOEXW;

typedef struct {
    DWORD size, major, minor, build, platform;
    char csd[128];
    WORD sp_major, sp_minor, suite;
    BYTE product, reserved;
} OSVERSIONINFOEXA;

typedef struct {
    WORD architecture, reserved;
    DWORD page_size;
    void *minimum, *maximum;
    u64 active_mask;
    DWORD processors, processor_type, granularity;
    WORD level, revision;
} SYSTEM_INFO;

typedef struct {
    DWORD length, load;
    u64 total_phys, avail_phys, total_page, avail_page, total_virtual, avail_virtual, avail_extended;
} MEMORYSTATUSEX;

typedef struct { WORD year, month, weekday, day, hour, minute, second, ms; } SYSTEMTIME;

typedef struct {
    int bias;
    WCHAR standard_name[32];
    SYSTEMTIME standard_date;
    int standard_bias;
    WCHAR daylight_name[32];
    SYSTEMTIME daylight_date;
    int daylight_bias;
} TIME_ZONE_INFORMATION;

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

typedef struct { WORD length, maximum; DWORD pad; WCHAR *buffer; } UNICODE_STRING;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K void __stdcall Sleep(DWORD);
K void __stdcall GetSystemTimeAsFileTime(u64 *);
K u64 __stdcall GetTickCount64(void);
K int __stdcall VirtualProtect(void *, u64, DWORD, DWORD *);
K int __stdcall VirtualFree(void *, u64, DWORD);
K DWORD __stdcall GetVersion(void);
K int __stdcall GetVersionExW(OSVERSIONINFOEXW *);
K int __stdcall GetVersionExA(OSVERSIONINFOEXA *);
K int __stdcall VerifyVersionInfoW(OSVERSIONINFOEXW *, DWORD, u64);
K u64 __stdcall VerSetConditionMask(u64, DWORD, BYTE);
K int __stdcall GetProductInfo(DWORD, DWORD, DWORD, DWORD, DWORD *);
K void __stdcall GetSystemInfo(SYSTEM_INFO *);
K void __stdcall GetNativeSystemInfo(SYSTEM_INFO *);
K int __stdcall GlobalMemoryStatusEx(MEMORYSTATUSEX *);
K int __stdcall IsProcessorFeaturePresent(DWORD);
K int __stdcall IsWow64Process(HANDLE, int *);
K int __stdcall IsWow64Process2(HANDLE, WORD *, WORD *);
K HANDLE __stdcall GetCurrentProcess(void);
K DWORD __stdcall GetActiveProcessorCount(WORD);
K WORD __stdcall GetActiveProcessorGroupCount(void);
K int __stdcall GetLogicalProcessorInformation(void *, DWORD *);
K int __stdcall GetLogicalProcessorInformationEx(DWORD, void *, DWORD *);
K int __stdcall GetComputerNameW(WCHAR *, DWORD *);
K int __stdcall GetComputerNameA(char *, DWORD *);
K int __stdcall GetComputerNameExW(DWORD, WCHAR *, DWORD *);
K void __stdcall GetSystemTime(SYSTEMTIME *);
K void __stdcall GetLocalTime(SYSTEMTIME *);
K int __stdcall FileTimeToSystemTime(const u64 *, SYSTEMTIME *);
K int __stdcall SystemTimeToFileTime(const SYSTEMTIME *, u64 *);
K int __stdcall FileTimeToLocalFileTime(const u64 *, u64 *);
K DWORD __stdcall GetTimeZoneInformation(TIME_ZONE_INFORMATION *);
K int __stdcall SystemTimeToTzSpecificLocalTime(const TIME_ZONE_INFORMATION *, const SYSTEMTIME *, SYSTEMTIME *);
K int __stdcall TzSpecificLocalTimeToSystemTime(const TIME_ZONE_INFORMATION *, const SYSTEMTIME *, SYSTEMTIME *);
K int __stdcall GetSystemTimes(u64 *, u64 *, u64 *);
K int __stdcall CreateProcessW(const WCHAR *, WCHAR *, void *, void *, int, DWORD, void *, const WCHAR *, STARTUPINFOW *,
                               PROCESS_INFORMATION *);
K int __stdcall TerminateProcess(HANDLE, unsigned);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K DWORD __stdcall GetCurrentProcessId(void);
K DWORD __stdcall GetCurrentThreadId(void);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K int __stdcall DuplicateHandle(HANDLE, HANDLE, HANDLE, HANDLE *, DWORD, int, DWORD);
K int __stdcall CloseHandle(HANDLE);
__declspec(dllimport) unsigned __stdcall NtQuerySystemInformation(DWORD, void *, DWORD, DWORD *);
__declspec(dllimport) unsigned __stdcall RtlGetVersion(OSVERSIONINFOEXW *);
__declspec(dllimport) void __stdcall RtlGetNtVersionNumbers(DWORD *, DWORD *, DWORD *);

enum { ERROR_INVALID_PARAMETER = 87, ERROR_BUFFER_OVERFLOW = 111, ERROR_INSUFFICIENT_BUFFER = 122,
       ERROR_OLD_WIN_VERSION = 1150, PAGE_READWRITE = 4, MEM_RELEASE = 0x8000,
       VER_MINORVERSION = 1, VER_MAJORVERSION = 2, VER_BUILDNUMBER = 4, VER_GREATER_EQUAL = 3, VER_EQUAL = 1 };

#ifdef __aarch64__
enum { ARCHITECTURE = 12, MACHINE = 0xaa64 };
#else
enum { ARCHITECTURE = 9, MACHINE = 0x8664 };
#endif
#ifdef WIN10
#define WIN10_NAME "sysinfo10.exe"
#else
#define WIN10_NAME "sysinfo.exe"
#endif
#ifdef WIN10
enum { TOLD_MAJOR = 10, TOLD_MINOR = 0, TOLD_BUILD = 19045 };
#else
enum { TOLD_MAJOR = 6, TOLD_MINOR = 2, TOLD_BUILD = 9200 };
#endif

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
static int same_ascii(const char *a, const char *b) { while (*b) if (*a++ != *b++) return 0; return *a == 0; }
static int wsame(const WCHAR *a, const char *b) { while (*b) if (*a++ != (WCHAR)*b++) return 0; return *a == 0; }
static int wends(const WCHAR *s, unsigned n, const char *end) {
    unsigned e = len(end);
    if (n < e) return 0;
    for (unsigned i = 0; i < e; i++) {
        WCHAR a = s[n - e + i], b = (WCHAR)end[i];
        if (a >= 'A' && a <= 'Z') a += 32;
        if (b >= 'A' && b <= 'Z') b += 32;
        if (a != b) return 0;
    }
    return 1;
}
static void zero(void *p, unsigned n) { volatile char *c = p; while (n--) *c++ = 0; }

#define SHARED ((volatile unsigned char *)0x7ffe0000)
static DWORD shared_u32(unsigned at) { return *(volatile DWORD *)(SHARED + at); }
// A KSYSTEM_TIME, read as Windows' readers do: until High1Time and High2Time agree.
static u64 shared_time(unsigned at) {
    for (;;) {
        DWORD high1 = *(volatile DWORD *)(SHARED + at + 4);
        DWORD low = *(volatile DWORD *)(SHARED + at);
        DWORD high2 = *(volatile DWORD *)(SHARED + at + 8);
        if (high1 == high2) return (u64)high1 << 32 | low;
    }
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);

    // KUSER_SHARED_DATA.
    check(shared_u32(0x26c) == 10 && shared_u32(0x270) == 0 && shared_u32(0x260) == 19045,
          "the shared data page: Windows 10.0.19045, whatever the manifest");
    check(*(volatile WORD *)(SHARED + 0x26a) == ARCHITECTURE, "  and this ISA's processor architecture");
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    check(shared_u32(0x3c0) == si.processors && shared_u32(0x2e8) > 0, "  its processor count as GetSystemInfo's, and pages of memory");
    int features = 1;
    for (DWORD i = 0; i < 64; i++) features = features && (SHARED[0x274 + i] != 0) == (IsProcessorFeaturePresent(i) != 0);
    check(features, "  its processor features as IsProcessorFeaturePresent says, all 64");
#ifdef __aarch64__
    check(IsProcessorFeaturePresent(29) && IsProcessorFeaturePresent(34), "  ARM64: crypto and LSE atomics");
#else
    check(IsProcessorFeaturePresent(10) && !IsProcessorFeaturePresent(17), "  x86-64: SSE2, and no AVX");
#endif
    u64 now;
    GetSystemTimeAsFileTime(&now);
    i64 apart = (i64)(shared_time(0x14) - now);
    check(apart > -1000000 && apart < 1000000, "  its system time within 100 ms of GetSystemTimeAsFileTime");
    u64 interrupt = shared_time(0x08), ticks = shared_time(0x320);
    Sleep(100);
    u64 moved = shared_time(0x08) - interrupt;
    check(moved >= 500000 && moved < 50000000, "  its interrupt time moving on, 100 ms of sleep later");
    check(shared_time(0x320) > ticks, "  and its tick count");
    DWORD old;
    check(!VirtualProtect((void *)SHARED, 0x1000, PAGE_READWRITE, &old), "  VirtualProtect can't make it writable");
    check(!VirtualFree((void *)SHARED, 0, MEM_RELEASE), "  VirtualFree can't free it");

    // The version.
    OSVERSIONINFOEXW v;
    zero(&v, sizeof v);
    v.size = sizeof v;
    check(RtlGetVersion(&v) == 0 && v.major == 10 && v.minor == 0 && v.build == 19045 && v.platform == 2 && v.product == 1,
          "RtlGetVersion: Windows 10.0.19045, a workstation");
    DWORD major = 0, minor = 0, build = 0;
    RtlGetNtVersionNumbers(&major, &minor, &build);
    check(major == 10 && minor == 0 && build == (0xf0000000u | 19045), "RtlGetNtVersionNumbers: the same, as a free build");
    zero(&v, sizeof v);
    v.size = sizeof v;
    check(GetVersionExW(&v) && v.major == TOLD_MAJOR && v.minor == TOLD_MINOR && v.build == TOLD_BUILD,
#ifdef WIN10
          "GetVersionExW: Windows 10, as the manifest says it supports it"
#else
          "GetVersionExW: Windows 8, as there's no manifest"
#endif
    );
    OSVERSIONINFOEXA va;
    zero(&va, sizeof va);
    va.size = 148;
    check(GetVersionExA(&va) && va.major == TOLD_MAJOR && va.build == TOLD_BUILD, "GetVersionExA, with the shorter OSVERSIONINFOA");
    va.size = 100;
    SetLastError(0);
    check(!GetVersionExA(&va) && GetLastError() == ERROR_INSUFFICIENT_BUFFER, "  a size that's neither: ERROR_INSUFFICIENT_BUFFER");
    DWORD packed = GetVersion();
    check((packed & 0xff) == TOLD_MAJOR && (packed >> 8 & 0xff) == TOLD_MINOR && packed >> 16 == TOLD_BUILD, "GetVersion: the same, packed");

    u64 at_least = VerSetConditionMask(VerSetConditionMask(0, VER_MAJORVERSION, VER_GREATER_EQUAL), VER_MINORVERSION, VER_GREATER_EQUAL);
    check(at_least == (3ull << 3 | 3), "VerSetConditionMask: three bits for each type");
    zero(&v, sizeof v);
    v.size = sizeof v;
    v.major = 6;
    v.minor = 1;
    check(VerifyVersionInfoW(&v, VER_MAJORVERSION | VER_MINORVERSION, at_least), "VerifyVersionInfo: at least Windows 7");
    v.major = 10;
    v.minor = 0;
    SetLastError(0);
    int ten = VerifyVersionInfoW(&v, VER_MAJORVERSION | VER_MINORVERSION, at_least);
#ifdef WIN10
    check(ten, "  at least Windows 10, with the manifest");
#else
    check(!ten && GetLastError() == ERROR_OLD_WIN_VERSION, "  not Windows 10, without it: ERROR_OLD_WIN_VERSION");
#endif
    v.major = 6;
    v.minor = 3;
    check(VerifyVersionInfoW(&v, VER_MAJORVERSION | VER_MINORVERSION, at_least) == (TOLD_MAJOR == 10),
          "  8.1 compared as one number: 6.3 against 6.2 or 10.0");
    v.build = TOLD_BUILD;
    check(VerifyVersionInfoW(&v, VER_BUILDNUMBER, VerSetConditionMask(0, VER_BUILDNUMBER, VER_EQUAL)), "  its build, equal");
    DWORD product = 0;
    check(GetProductInfo(10, 0, 0, 0, &product) && product == 0x30, "GetProductInfo: Windows 10 Pro");

    // The machine.
    SYSTEM_INFO native;
    GetNativeSystemInfo(&native);
    check(si.architecture == ARCHITECTURE && si.page_size == 4096 && si.granularity == 0x10000, "GetSystemInfo: this ISA, 4 KiB pages, 64 KiB granules");
    check(si.processors >= 1 && si.active_mask == (si.processors == 64 ? ~0ull : (1ull << si.processors) - 1) &&
          (u64)si.minimum == 0x10000 && (u64)si.maximum > 0x7fffffffull,
          "  its processors' mask, and the program's addresses");
    check(native.architecture == si.architecture && native.processors == si.processors, "GetNativeSystemInfo: the same machine");
    MEMORYSTATUSEX m;
    zero(&m, sizeof m);
    m.length = 60;
    SetLastError(0);
    check(!GlobalMemoryStatusEx(&m) && GetLastError() == ERROR_INVALID_PARAMETER, "GlobalMemoryStatusEx: the wrong length is refused");
    m.length = sizeof m;
    check(GlobalMemoryStatusEx(&m) && m.total_phys > 0 && m.avail_phys <= m.total_phys && m.load <= 100 && m.total_virtual > 0x7fffffffull,
          "  and with the right one: physical memory, what's free, and the address space");
    int wow = 1;
    check(IsWow64Process(GetCurrentProcess(), &wow) && !wow, "IsWow64Process: no");
    WORD process_machine = 1, native_machine = 0;
    check(IsWow64Process2(GetCurrentProcess(), &process_machine, &native_machine) && process_machine == 0 && native_machine == MACHINE,
          "IsWow64Process2: not WOW64, on this ISA's machine");
    check(GetActiveProcessorCount(0xffff) == si.processors && GetActiveProcessorCount(1) == 0 && GetActiveProcessorGroupCount() == 1,
          "the processor counts: one group of them all");

    static unsigned char records[16384];
    DWORD size = 0;
    SetLastError(0);
    check(!GetLogicalProcessorInformationEx(0xffff, 0, &size) && GetLastError() == ERROR_INSUFFICIENT_BUFFER && size > 0,
          "GetLogicalProcessorInformationEx: how big, first");
    DWORD cores = 0, groups = 0, caches = 0, packages = 0, ok = size <= sizeof records;
    ok = ok && GetLogicalProcessorInformationEx(0xffff, records, &size);
    for (DWORD at = 0; ok && at < size;) {
        DWORD relation = *(DWORD *)(records + at), length = *(DWORD *)(records + at + 4);
        if (length < 8) { ok = 0; break; }
        cores += relation == 0;
        caches += relation == 2;
        packages += relation == 3;
        if (relation == 4) groups += *(WORD *)(records + at + 10) == 1 && records[at + 33] == si.processors;
        at += length;
    }
    check(ok && cores == si.processors && packages == 1 && groups == 1 && caches > 0,
          "  then its records: a core for each processor, caches, one package, one group");
    size = 0;
    GetLogicalProcessorInformationEx(0, 0, &size);
    check(size == si.processors * 48, "  just the cores, 48 bytes each");
    size = sizeof records;
    check(GetLogicalProcessorInformation(records, &size) && size % 32 == 0 && size >= (si.processors + 2) * 32,
          "GetLogicalProcessorInformation: 32-byte records");

    // The machine's name.
    WCHAR name[32];
    size = 2;
    SetLastError(0);
    check(!GetComputerNameW(name, &size) && GetLastError() == ERROR_BUFFER_OVERFLOW && size == 7, "GetComputerNameW: too small, and the size it needs");
    size = 32;
    check(GetComputerNameW(name, &size) && size == 6 && wsame(name, "CARBON"), "  then CARBON");
    char ansi[32];
    size = 32;
    check(GetComputerNameA(ansi, &size) && size == 6 && ansi[0] == 'C' && ansi[6] == 0, "GetComputerNameA: the same");
    size = 32;
    check(GetComputerNameExW(2, name, &size) && size == 0 && name[0] == 0, "GetComputerNameExW: no DNS domain");

    // Time.
    SYSTEMTIME st, from_file, local;
    GetSystemTimeAsFileTime(&now);
    GetSystemTime(&st);
    check(FileTimeToSystemTime(&now, &from_file) && from_file.year == st.year && from_file.month == st.month && from_file.day == st.day && st.year >= 2024,
          "GetSystemTime: today, as FileTimeToSystemTime has it");
    GetLocalTime(&local);
    check(local.hour == st.hour || local.minute == 0, "GetLocalTime: UTC's");
    u64 back = 0;
    check(SystemTimeToFileTime(&from_file, &back) && back == now / 10000 * 10000, "SystemTimeToFileTime: back again, to the millisecond");
    SYSTEMTIME leap = {2024, 2, 0, 29, 23, 59, 59, 999}, bad = {2023, 2, 0, 29, 0, 0, 0, 0};
    check(SystemTimeToFileTime(&leap, &back) && FileTimeToSystemTime(&back, &st) && st.day == 29 && st.weekday == 4,
          "  29 February 2024, a Thursday");
    SetLastError(0);
    check(!SystemTimeToFileTime(&bad, &back) && GetLastError() == ERROR_INVALID_PARAMETER, "  and no 29 February 2023");
    u64 shifted = 0;
    check(FileTimeToLocalFileTime(&now, &shifted) && shifted == now, "FileTimeToLocalFileTime: unchanged in UTC");
    TIME_ZONE_INFORMATION tz;
    zero(&tz, sizeof tz);
    tz.bias = 1;
    check(GetTimeZoneInformation(&tz) == 0 && tz.bias == 0 && wsame(tz.standard_name, "Coordinated Universal Time"),
          "GetTimeZoneInformation: UTC, without daylight saving");

    // US Eastern: five hours behind, an hour less from the second Sunday of March to the first of November.
    zero(&tz, sizeof tz);
    tz.bias = 300;
    tz.standard_date = (SYSTEMTIME){0, 11, 0, 1, 2, 0, 0, 0};
    tz.daylight_date = (SYSTEMTIME){0, 3, 0, 2, 2, 0, 0, 0};
    tz.daylight_bias = -60;
    SYSTEMTIME summer = {2024, 7, 0, 1, 16, 0, 0, 0}, winter = {2024, 1, 0, 15, 17, 0, 0, 0}, utc;
    check(SystemTimeToTzSpecificLocalTime(&tz, &summer, &local) && local.hour == 12 && local.day == 1,
          "SystemTimeToTzSpecificLocalTime: noon in July, in daylight saving");
    check(SystemTimeToTzSpecificLocalTime(&tz, &winter, &local) && local.hour == 12, "  and noon in January, without");
    check(TzSpecificLocalTimeToSystemTime(&tz, &local, &utc) && utc.hour == 17 && utc.day == 15, "TzSpecificLocalTimeToSystemTime: back");

    // NtQuerySystemInformation.
    enum { INFO_LENGTH_MISMATCH = 0xc0000004, INVALID_INFO_CLASS = 0xc0000003 };
    static unsigned char info[65536];
    DWORD got = 0;
    check(NtQuerySystemInformation(0, info, 64, &got) == 0 && got == 64 && *(DWORD *)(info + 8) == 4096 &&
          *(DWORD *)(info + 24) == 0x10000 && info[56] == si.processors && *(u64 *)(info + 48) == si.active_mask &&
          *(u64 *)(info + 40) == (u64)si.maximum,
          "NtQuerySystemInformation: basic information, as GetSystemInfo has it");
    got = 0;
    check(NtQuerySystemInformation(0, info, 63, &got) == INFO_LENGTH_MISMATCH && got == 64, "  a length short: the length it wants");
    check(NtQuerySystemInformation(1, info, 12, &got) == 0 && *(WORD *)info == ARCHITECTURE && *(WORD *)(info + 6) == si.processors,
          "  processor information");
    check(NtQuerySystemInformation(2, info, 0x158, &got) == 0 && got == 0x158 && *(DWORD *)(info + 0x34) > 0 &&
          *(DWORD *)(info + 0x2c) <= *(DWORD *)(info + 0x34),
          "  performance: memory in pages");
    check(NtQuerySystemInformation(2, info, 0x138, &got) == 0 && got == 0x138, "  and Windows Vista's shorter size");
    check(NtQuerySystemInformation(8, info, sizeof info, &got) == 0 && got == si.processors * 48 && *(u64 *)(info + 8) >= *(u64 *)info,
          "  each processor's times, kernel counting idle");
    u64 idle = 0, kernel = 0, user = 1;
    check(GetSystemTimes(&idle, &kernel, &user) && kernel >= idle && idle > 0 && user == 0, "GetSystemTimes: the same, for all of them");
    GetSystemTimeAsFileTime(&now);
    check(NtQuerySystemInformation(3, info, 48, &got) == 0 && got == 48 && *(u64 *)info < now && (u64)((i64)(*(u64 *)(info + 8) - now) + 10000000) < 20000000ull &&
          *(i64 *)(info + 16) == 0,
          "  the time of day: booted before now, now, no bias");
    check(NtQuerySystemInformation(3, info, 16, &got) == 0 && got == 16, "  or just its first part");
    check(NtQuerySystemInformation(35, info, 2, &got) == 0 && info[0] == 0 && info[1] == 1, "  no kernel debugger");
    check(NtQuerySystemInformation(50, info, 8, &got) == 0 && *(u64 *)info == 0xffff800000000000ull &&
          NtQuerySystemInformation(50, info, 16, &got) == INFO_LENGTH_MISMATCH,
          "  where the system's addresses start, in exactly 8 bytes");
    check(NtQuerySystemInformation(11, info, sizeof info, &got) == 0 && *(DWORD *)info == 2 &&
          same_ascii((char *)info + 8 + 0x28 + *(WORD *)(info + 8 + 0x26), "ntoskrnl.exe"),
          "  the kernel's modules: ntoskrnl.exe first");
    check(NtQuerySystemInformation(0x7fff, info, sizeof info, &got) == INVALID_INFO_CLASS, "  a class it doesn't know: STATUS_INVALID_INFO_CLASS");

    // The process list, with a child.
    WCHAR self[260];
    GetModuleFileNameW(0, self, 260);
    STARTUPINFOW start_info;
    zero(&start_info, sizeof start_info);
    start_info.cb = sizeof start_info;
    PROCESS_INFORMATION pi;
    zero(&pi, sizeof pi);
    int started = CreateProcessW(self, 0, 0, 0, 0, 4 /* CREATE_SUSPENDED */, 0, 0, &start_info, &pi);
    got = 0;
    NtQuerySystemInformation(5, info, 16, &got);
    unsigned status = NtQuerySystemInformation(5, info, sizeof info, &got);
    int idle_first = 0, found_self = 0, found_child = 0, threads_ok = 0, walked = 0;
    for (unsigned at = 0;; walked++) {
        unsigned char *p = info + at;
        u64 pid = *(u64 *)(p + 0x50);
        DWORD thread_count = *(DWORD *)(p + 4);
        UNICODE_STRING *name = (UNICODE_STRING *)(p + 0x38);
        if (at == 0) idle_first = pid == 0 && thread_count == si.processors && name->length == 0;
        if (pid == GetCurrentProcessId()) {
            found_self = name->buffer && wends(name->buffer, name->length / 2, WIN10_NAME) && *(u64 *)(p + 0x58) == 0 &&
                         *(DWORD *)(p + 0x60) > 0 && *(u64 *)(p + 0x78) > 0;
            for (DWORD i = 0; i < thread_count; i++)
                threads_ok |= *(u64 *)(p + 0x100 + i * 0x50 + 0x30) == GetCurrentThreadId() && *(u64 *)(p + 0x100 + i * 0x50 + 0x28) == pid;
        }
        if (pid == pi.pid) found_child = *(u64 *)(p + 0x58) == GetCurrentProcessId() && thread_count == 1 && *(u64 *)(p + 0x20) > 0;
        DWORD next = *(DWORD *)p;
        if (!next || walked > 64) break;
        at += next;
    }
    check(status == 0 && got > 0x100, "the process list, after asking how big it is");
    check(idle_first, "  the idle process first: ID 0, a thread a processor");
    check(found_self, "  this one, by its name, with no parent, handles and memory");
    check(threads_ok, "  and this thread among its threads");
    check(started && found_child, "  a suspended child: this one its parent, one thread, its start time");
    TerminateProcess(pi.process, 7);
    WaitForSingleObject(pi.process, 10000);

    // The handle lists.
    HANDLE event = CreateEventW(0, 1, 0, 0), copy = 0;
    DuplicateHandle(GetCurrentProcess(), event, GetCurrentProcess(), &copy, 0, 0, 2 /* DUPLICATE_SAME_ACCESS */);
    u64 object = 0, copied = 0;
    int typed = 0;
    status = NtQuerySystemInformation(16, info, sizeof info, &got);
    DWORD count = *(DWORD *)info;
    for (DWORD i = 0; status == 0 && i < count; i++) {
        unsigned char *e = info + 8 + i * 24;
        if (*(WORD *)e != (WORD)GetCurrentProcessId()) continue;
        if (*(WORD *)(e + 6) == (WORD)(u64)event) { object = *(u64 *)(e + 8); typed = e[4] == 16; }
        if (*(WORD *)(e + 6) == (WORD)(u64)copy) copied = *(u64 *)(e + 8);
    }
    check(status == 0 && got == 8 + count * 24 && typed, "the handle list: the event's handle, an Event");
    check(object && object == copied && object >= 0xffff800000000000ull, "  its duplicate, the same object, at a kernel address");
    status = NtQuerySystemInformation(64, info, sizeof info, &got);
    u64 ex_count = *(u64 *)info, ex_object = 0;
    for (u64 i = 0; status == 0 && i < ex_count; i++) {
        unsigned char *e = info + 16 + i * 40;
        if (*(u64 *)(e + 8) == GetCurrentProcessId() && *(u64 *)(e + 16) == (u64)copy) ex_object = *(u64 *)e;
    }
    check(status == 0 && got == 16 + ex_count * 40 && ex_count == count && ex_object == object, "  the extended list: the same");
    CloseHandle(copy);
    CloseHandle(event);

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
