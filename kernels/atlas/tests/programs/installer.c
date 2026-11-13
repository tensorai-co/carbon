// What installers call, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it.
//
// It checks: kernel32's and user32's string functions (lstrlen, lstrcpy(n), lstrcat, lstrcmp(i), CharNext/Prev),
// MulDiv and CompareFileTime; wsprintf, C-variadic, so its arguments are found as each ISA passes them, past the
// registers too, and cut at 1024 characters; wvsprintf with a va_list; temporary names, short names, disk space and
// SearchPath; .ini files; the error mode; COM's start, its task allocator and CoCreateInstance with no class
// registered; and SHGetFileInfo as installers first call it.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef struct { DWORD lo, hi; } FILETIME;
typedef struct { DWORD a; unsigned short b, c; unsigned char d[8]; } GUID;
typedef __builtin_va_list va_list;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K int __stdcall lstrlenW(const WCHAR *);
K int __stdcall lstrlenA(const char *);
K WCHAR *__stdcall lstrcpyW(WCHAR *, const WCHAR *);
K WCHAR *__stdcall lstrcatW(WCHAR *, const WCHAR *);
K WCHAR *__stdcall lstrcpynW(WCHAR *, const WCHAR *, int);
K int __stdcall lstrcmpW(const WCHAR *, const WCHAR *);
K int __stdcall lstrcmpiW(const WCHAR *, const WCHAR *);
K int __stdcall lstrcmpA(const char *, const char *);
K int __stdcall MulDiv(int, int, int);
K int __stdcall CompareFileTime(const FILETIME *, const FILETIME *);
K DWORD __stdcall GetTempPathW(DWORD, WCHAR *);
K unsigned __stdcall GetTempFileNameW(const WCHAR *, const WCHAR *, unsigned, WCHAR *);
K DWORD __stdcall GetShortPathNameW(const WCHAR *, WCHAR *, DWORD);
K int __stdcall GetDiskFreeSpaceW(const WCHAR *, DWORD *, DWORD *, DWORD *, DWORD *);
K DWORD __stdcall SearchPathW(const WCHAR *, const WCHAR *, const WCHAR *, DWORD, WCHAR *, WCHAR **);
K DWORD __stdcall GetPrivateProfileStringW(const WCHAR *, const WCHAR *, const WCHAR *, WCHAR *, DWORD, const WCHAR *);
K int __stdcall WritePrivateProfileStringW(const WCHAR *, const WCHAR *, const WCHAR *, const WCHAR *);
K unsigned __stdcall SetErrorMode(unsigned);
K DWORD __stdcall GetFileAttributesW(const WCHAR *);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K int __stdcall CloseHandle(HANDLE);
K int __stdcall DeleteFileW(const WCHAR *);
K WCHAR *__stdcall CharNextW(const WCHAR *);
K WCHAR *__stdcall CharPrevW(const WCHAR *, const WCHAR *);
K int __cdecl wsprintfW(WCHAR *, const WCHAR *, ...);
K int __cdecl wsprintfA(char *, const char *, ...);
K int __stdcall wvsprintfW(WCHAR *, const WCHAR *, va_list);
K long __stdcall CoInitialize(void *);
K void __stdcall CoUninitialize(void);
K long __stdcall OleInitialize(void *);
K void __stdcall OleUninitialize(void);
K void *__stdcall CoTaskMemAlloc(__SIZE_TYPE__);
K void *__stdcall CoTaskMemRealloc(void *, __SIZE_TYPE__);
K void __stdcall CoTaskMemFree(void *);
K long __stdcall CoCreateInstance(const GUID *, void *, DWORD, const GUID *, void **);
K __UINTPTR_TYPE__ __stdcall SHGetFileInfoW(const WCHAR *, DWORD, void *, unsigned, unsigned);

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
static WCHAR wbuf[4][128];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int wweq(const WCHAR *a, const WCHAR *b) { while (*b && *a == *b) { a++; b++; } return *a == 0 && *b == 0; }
static int eq(const char *a, const char *b) { while (*b && *a == *b) { a++; b++; } return *a == 0 && *b == 0; }
static int wends(const WCHAR *a, const char *b) {
    int n = 0, m = (int)len(b);
    while (a[n]) n++;
    if (n < m) return 0;
    return weq(a + n - m, b);
}
static int wstarts(const WCHAR *a, const WCHAR *b) { while (*b && *a == *b) { a++; b++; } return *b == 0; }
static void wjoin(WCHAR *d, const WCHAR *a, const char *b) { int n = 0; for (; a[n]; n++) d[n] = a[n]; for (int i = 0; b[i]; i++) d[n++] = (WCHAR)b[i]; d[n] = 0; }

static int via_va_list(WCHAR *b, const WCHAR *f, ...) {
    va_list ap;
    __builtin_va_start(ap, f);
    int n = wvsprintfW(b, f, ap);
    __builtin_va_end(ap);
    return n;
}

static WCHAR big[1100];
static WCHAR long_text[1500];
static char nbuf[64];

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("what installers call, arm64\n");
#elif defined(__i386__)
    say("what installers call, x86 (WoW64)\n");
#else
    say("what installers call, x86-64\n");
#endif
    WCHAR s[260], t[260];

    // ---- Strings ----
    check(lstrlenW(W(0, "hello")) == 5 && lstrlenA("hey") == 3 && lstrlenA(0) == 0, "lstrlen, and null is 0");
    lstrcpyW(s, W(0, "hello"));
    lstrcatW(s, W(0, " world"));
    check(weq(s, "hello world"), "lstrcpyW and lstrcatW");
    check(lstrcpynW(s, W(0, "abcdef"), 4) == s && weq(s, "abc"), "lstrcpynW: n - 1 units and the NUL");
    check(lstrcmpiW(W(0, "ABC"), W(1, "abc")) == 0, "lstrcmpiW: case doesn't matter");
    check(lstrcmpW(W(0, "abc"), W(1, "ABC")) < 0 && lstrcmpW(W(0, "b"), W(1, "A")) > 0, "lstrcmpW: letters first, then lower-case first");
    check(lstrcmpA("a", "b") < 0 && lstrcmpA("same", "same") == 0, "lstrcmpA");
    const WCHAR *ab = W(0, "ab");
    check(CharNextW(ab) == ab + 1 && CharNextW(ab + 2) == ab + 2, "CharNextW, staying at the NUL");
    check(CharPrevW(ab, ab + 1) == ab && CharPrevW(ab, ab) == ab, "CharPrevW, staying at the start");
    check(MulDiv(10, 3, 4) == 8 && MulDiv(-10, 3, 4) == -8 && MulDiv(1, 1, 0) == -1, "MulDiv: rounded half away from zero; / 0 is -1");
    check(MulDiv(0x40000000, 4, 2) == (int)0x80000000 || MulDiv(0x40000000, 4, 2) == -1, "MulDiv: 64 bits in between");
    FILETIME a = {5, 1}, b = {4, 2};
    check(CompareFileTime(&a, &b) == -1 && CompareFileTime(&b, &a) == 1 && CompareFileTime(&a, &a) == 0, "CompareFileTime, high part first");

    // ---- wsprintf ----
    int n = wsprintfW(s, W(0, "%d-%s-%x-%c|%5d|%-3d|"), -42, W(1, "str"), 0xbeef, (WCHAR)'Z', 7, 1);
    check(n == 25 && weq(s, "-42-str-beef-Z|    7|1  |"), "wsprintfW: conversions, widths and the length");
    n = wsprintfW(s, W(0, "%d%d%d%d%d%d%d%d%d%d%d"), 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1);
    check(n == 11 && weq(s, "12345678901"), "wsprintfW: eleven arguments, on the stack past the registers");
    n = wsprintfW(s, W(0, "%08X %u %s"), 0xabcu, 4000000000u, W(1, "end"));
    check(n == 23 && weq(s, "00000ABC 4000000000 end"), "wsprintfW: zero padding, unsigned, a string after them");
    n = wsprintfA(nbuf, "%s=%u, %ls", "n", 5u, W(0, "wide"));
    check(n == 9 && eq(nbuf, "n=5, wide"), "wsprintfA, with a wide %ls");
    for (int i = 0; i < 1499; i++) long_text[i] = 'x';
    long_text[1499] = 0;
    big[1024] = 'q';
    n = wsprintfW(big, W(0, "%s"), long_text);
    check(n == 1024 && big[1023] == 'x' && big[1024] == 0, "wsprintfW: cut at 1024 characters and the NUL");
    n = via_va_list(s, W(0, "%s/%d/%s"), W(1, "a"), 99, W(2, "b"));
    check(n == 6 && weq(s, "a/99/b"), "wvsprintfW, with a va_list");

    // ---- Files ----
    WCHAR tmp[260];
    DWORD tn = GetTempPathW(260, tmp);
    check(tn > 0 && tmp[tn - 1] == '\\', "GetTempPathW, with its backslash");
    unsigned u = GetTempFileNameW(tmp, W(0, "nsiX"), 0, s);
    check(u != 0 && wstarts(s, tmp) && wends(s, ".tmp") && GetFileAttributesW(s) != 0xffffffff,
          "GetTempFileNameW: a new number, its empty file made");
    unsigned v = GetTempFileNameW(tmp, W(0, "nsi"), 0x12ab, t);
    wjoin(big, tmp, "nsi12AB.tmp");
    check(v == 0x12ab && wweq(t, big) && GetFileAttributesW(t) == 0xffffffff, "GetTempFileNameW: a number given, in hex, nothing made");
    DWORD sn = GetShortPathNameW(s, t, 260);
    check(sn == (DWORD)lstrlenW(s) && wweq(t, s), "GetShortPathNameW: the long name, with no 8.3 names");
    wjoin(big, tmp, "not-there.txt");
    check(GetShortPathNameW(big, t, 260) == 0 && GetLastError() == 2, "GetShortPathNameW: not there, ERROR_FILE_NOT_FOUND");
    DeleteFileW(s);
    DWORD spc = 0, bps = 0, free = 0, total = 0;
    check(GetDiskFreeSpaceW(W(0, "C:\\"), &spc, &bps, &free, &total) && spc * bps == 4096 && free > 0 && free <= total,
          "GetDiskFreeSpaceW: clusters, their size, and free ones");
    wjoin(big, tmp, "found.txt");
    HANDLE h = CreateFileW(big, 0x40000000, 0, 0, 2, 0x80, 0);
    check(h != (HANDLE)-1 && CloseHandle(h), "a file to search for");
    WCHAR *part = 0;
    n = (int)SearchPathW(tmp, W(0, "found"), W(1, ".txt"), 260, s, &part);
    check(n == lstrlenW(big) && wweq(s, big) && part && weq(part, "found.txt"), "SearchPathW: in a folder, the extension added, its file part");
    check(SearchPathW(tmp, W(0, "found"), W(1, ".txt"), 4, s, &part) == (DWORD)lstrlenW(big) + 1, "SearchPathW: too small, the size it needs");
    check(SearchPathW(tmp, W(0, "nowhere.txt"), 0, 260, s, &part) == 0 && GetLastError() == 2, "SearchPathW: not found");
    DeleteFileW(big);

    // ---- .ini files ----
    WCHAR ini[260];
    wjoin(ini, tmp, "carbon-test.ini");
    DeleteFileW(ini);
    check(WritePrivateProfileStringW(W(0, "Sec"), W(1, "key"), W(2, "value"), ini) == 1, "WritePrivateProfileStringW makes the file");
    check(WritePrivateProfileStringW(W(0, "Sec"), W(1, "two"), W(2, "2"), ini) == 1, "and a second key");
    check(WritePrivateProfileStringW(W(0, "Other"), W(1, "k"), W(2, "v"), ini) == 1, "and a second section");
    n = (int)GetPrivateProfileStringW(W(0, "sec"), W(1, "KEY"), W(2, "def"), s, 260, ini);
    check(n == 5 && weq(s, "value"), "GetPrivateProfileStringW: a value, any case");
    n = (int)GetPrivateProfileStringW(W(0, "Sec"), W(1, "none"), W(2, "def"), s, 260, ini);
    check(n == 3 && weq(s, "def"), "a key that isn't there: the default");
    n = (int)GetPrivateProfileStringW(W(0, "Sec"), 0, W(2, "def"), s, 260, ini);
    check(n == 8 && weq(s, "key") && weq(s + 4, "two") && s[8] == 0, "no key: the section's keys, each with a NUL");
    n = (int)GetPrivateProfileStringW(0, 0, 0, s, 260, ini);
    check(n == 10 && weq(s, "Sec") && weq(s + 4, "Other"), "no section: the sections' names");
    n = (int)GetPrivateProfileStringW(W(0, "Sec"), W(1, "key"), W(2, "def"), s, 3, ini);
    check(n == 2 && weq(s, "va"), "too small: cut, size - 1");
    check(WritePrivateProfileStringW(W(0, "Sec"), W(1, "key"), 0, ini) == 1, "no value deletes the key");
    n = (int)GetPrivateProfileStringW(W(0, "Sec"), W(1, "key"), W(2, "gone"), s, 260, ini);
    check(n == 4 && weq(s, "gone"), "and it's gone");
    check(WritePrivateProfileStringW(W(0, "Other"), 0, 0, ini) == 1, "no key deletes the section");
    n = (int)GetPrivateProfileStringW(0, 0, 0, s, 260, ini);
    check(n == 4 && weq(s, "Sec") && s[4] == 0, "and only Sec is left");
    DeleteFileW(ini);

    // ---- The error mode ----
    unsigned before = SetErrorMode(0x8001);
    check(SetErrorMode(before) == 0x8001, "SetErrorMode gives the mode it had");

    // ---- COM ----
    check(CoInitialize(0) == 0, "CoInitialize");
    check(OleInitialize(0) == 0, "OleInitialize");
    char *p = CoTaskMemAlloc(16);
    check(p != 0, "CoTaskMemAlloc");
    for (int i = 0; i < 16; i++) p[i] = (char)i;
    p = CoTaskMemRealloc(p, 4096);
    int kept = p != 0;
    for (int i = 0; kept && i < 16; i++) kept = p[i] == (char)i;
    check(kept, "CoTaskMemRealloc keeps what was there");
    CoTaskMemFree(p);
    CoTaskMemFree(0);
    GUID clsid = {0x00021401, 0, 0, {0xc0, 0, 0, 0, 0, 0, 0, 0x46}}, iid = {0x000214f9, 0, 0, {0xc0, 0, 0, 0, 0, 0, 0, 0x46}};
    void *object = (void *)1;
    check(CoCreateInstance(&clsid, 0, 1, &iid, &object) == (long)0x80040154 && object == 0, "CoCreateInstance: no class registered, and no object");
    OleUninitialize();
    CoUninitialize();

    // ---- The shell ----
    unsigned char info[700];
    for (int i = 0; i < 700; i++) info[i] = 0xff;
    unsigned size = sizeof(void *) == 8 ? 696 : 692;
    check(SHGetFileInfoW(W(0, ""), 0, info, size, 0) == 1 && info[0] == 0 && info[size - 1] == 0 && info[size] == 0xff,
          "SHGetFileInfoW with no flags: TRUE, the structure cleared");

    ExitProcess(failures);
}
