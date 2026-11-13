// The environment block on atlas, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it.
//
// It checks: SystemRoot, PROCESSOR_ARCHITECTURE, Path from the registry expanded, USERNAME through the A version,
// NUMBER_OF_PROCESSORS, TEMP; ERROR_ENVVAR_NOT_FOUND; set and get in another case; sizes when the buffer's short,
// missing or just enough; replacing; SetEnvironmentVariableA; an empty value; = in a name refused;
// ExpandEnvironmentStrings (W and A) with unknown names and a short buffer; GetEnvironmentStringsW in order with the
// new variables; the ANSI block; deleting with NULL

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K DWORD __stdcall GetEnvironmentVariableW(const WCHAR *, WCHAR *, DWORD);
K DWORD __stdcall GetEnvironmentVariableA(const char *, char *, DWORD);
K int __stdcall SetEnvironmentVariableW(const WCHAR *, const WCHAR *);
K int __stdcall SetEnvironmentVariableA(const char *, const char *);
K DWORD __stdcall ExpandEnvironmentStringsW(const WCHAR *, WCHAR *, DWORD);
K DWORD __stdcall ExpandEnvironmentStringsA(const char *, char *, DWORD);
K WCHAR *__stdcall GetEnvironmentStringsW(void);
K char *__stdcall GetEnvironmentStrings(void);
K int __stdcall FreeEnvironmentStringsW(WCHAR *);
K int __stdcall FreeEnvironmentStringsA(char *);

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
static int eq(const char *a, const char *b) { while (*b && *a == *b) { a++; b++; } return *a == 0 && *b == 0; }
static int wstarts(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *b == 0; }
static int upper(int c) { return c >= 'a' && c <= 'z' ? c - 32 : c; }
/// Compares two entries' names (up to '='), without case.
static int before(const WCHAR *a, const WCHAR *b) {
    for (;; a++, b++) {
        int x = *a == '=' ? 0 : upper(*a), y = *b == '=' ? 0 : upper(*b);
        if (x != y) return x < y;
        if (!x) return 1;
    }
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("the environment on atlas, arm64\n");
    const char *arch = "ARM64";
#elif defined(__i386__)
    say("the environment on atlas, x86 (WoW64)\n");
    const char *arch = "x86";
#else
    say("the environment on atlas, x86-64\n");
    const char *arch = "AMD64";
#endif
    WCHAR value[512];
    char text[512];
    DWORD n = GetEnvironmentVariableW(W(0, "SystemRoot"), value, 512);
    check(n == 10 && weq(value, "C:\\Windows"), "SystemRoot, from the session");
    n = GetEnvironmentVariableW(W(0, "PROCESSOR_ARCHITECTURE"), value, 512);
    check(weq(value, arch) && n == len(arch), "PROCESSOR_ARCHITECTURE, the program's");
    GetEnvironmentVariableW(W(0, "Path"), value, 512);
    check(wstarts(value, "C:\\Windows\\system32;C:\\Windows;"), "Path, from the registry, expanded");
    n = GetEnvironmentVariableA("username", text, sizeof text);
    check(n == 6 && eq(text, "carbon"), "GetEnvironmentVariableA: USERNAME, any case");
    n = GetEnvironmentVariableW(W(0, "NUMBER_OF_PROCESSORS"), value, 512);
    check(n >= 1 && value[0] >= '1' && value[0] <= '9', "NUMBER_OF_PROCESSORS");
    n = GetEnvironmentVariableW(W(0, "TEMP"), value, 512);
    check(n > 0, "TEMP, from the user's key");

    SetLastError(0);
    check(GetEnvironmentVariableW(W(0, "CARBON_NONE"), value, 512) == 0 && GetLastError() == 203, "not set: 0 and ERROR_ENVVAR_NOT_FOUND");

    check(SetEnvironmentVariableW(W(0, "Carbon_Test"), W(1, "hello")) == 1, "SetEnvironmentVariableW");
    n = GetEnvironmentVariableW(W(0, "CARBON_TEST"), value, 512);
    check(n == 5 && weq(value, "hello"), "and back, by another case");
    check(GetEnvironmentVariableW(W(0, "carbon_test"), value, 5) == 6, "too small a buffer: the size it needs, with the NUL");
    check(GetEnvironmentVariableW(W(0, "carbon_test"), 0, 0) == 6, "no buffer: the same");
    check(GetEnvironmentVariableW(W(0, "carbon_test"), value, 6) == 5, "just enough: the length");
    SetEnvironmentVariableW(W(0, "CARBON_TEST"), W(1, "a longer value than before"));
    GetEnvironmentVariableW(W(0, "Carbon_Test"), value, 512);
    check(weq(value, "a longer value than before"), "set again: replaced");
    check(SetEnvironmentVariableA("CARBON_A", "ansi") == 1 && GetEnvironmentVariableW(W(0, "carbon_a"), value, 512) == 4 && weq(value, "ansi"),
          "SetEnvironmentVariableA, read as UTF-16");
    check(SetEnvironmentVariableW(W(0, "CARBON_EMPTY"), W(1, "")) == 1 && GetEnvironmentVariableW(W(0, "CARBON_EMPTY"), value, 512) == 0
              && value[0] == 0,
          "an empty value is still set");
    SetLastError(0);
    check(SetEnvironmentVariableW(W(0, "A=B"), W(1, "x")) == 0 && GetLastError() == 87, "a name with '=': ERROR_INVALID_PARAMETER");

    n = ExpandEnvironmentStringsW(W(0, "%SystemRoot%\\x;%carbon_a%;%NOPE%;100%"), value, 512);
    check(weq(value, "C:\\Windows\\x;ansi;%NOPE%;100%") && n == 30, "ExpandEnvironmentStringsW: the chars with the NUL; unknown ones stay");
    check(ExpandEnvironmentStringsW(W(0, "%SystemRoot%"), value, 4) == 11, "too small: the size it needs");
    n = ExpandEnvironmentStringsA("%USERPROFILE%\\Desktop", text, sizeof text);
    check(eq(text, "C:\\Users\\carbon\\Desktop") && n == 24, "ExpandEnvironmentStringsA");

    WCHAR *block = GetEnvironmentStringsW();
    int count = 0, sorted = 1, seen_test = 0, seen_a = 0;
    const WCHAR *last = 0;
    for (const WCHAR *e = block; *e; ) {
        const WCHAR *start = e;
        if (last && !before(last, start)) sorted = 0;
        if (wstarts(start, "CARBON_TEST=a longer")) seen_test = 1;
        if (wstarts(start, "CARBON_A=ansi")) seen_a = 1;
        last = start;
        while (*e) e++;
        e++;
        count++;
    }
    check(count > 20 && seen_test && seen_a, "GetEnvironmentStringsW: every variable, set ones too");
    check(sorted, "in order, by name");
    check(FreeEnvironmentStringsW(block) == 1, "FreeEnvironmentStringsW");
    char *ansi = GetEnvironmentStrings();
    int found = 0;
    for (const char *e = ansi; *e; ) {
        if (eq(e, "USERNAME=carbon")) found = 1;
        while (*e) e++;
        e++;
    }
    check(found && FreeEnvironmentStringsA(ansi) == 1, "GetEnvironmentStrings (ANSI)");

    check(SetEnvironmentVariableW(W(0, "carbon_test"), 0) == 1 && GetEnvironmentVariableW(W(0, "Carbon_Test"), value, 512) == 0
              && GetLastError() == 203,
          "a NULL value deletes it");
    check(GetEnvironmentVariableW(W(0, "CARBON_A"), value, 512) == 4, "and only it");

    ExitProcess(failures);
}
