// The registry on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.

typedef void *HANDLE;
typedef void *HKEY;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef long LSTATUS;
typedef unsigned short WCHAR;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetFileAttributesW(const WCHAR *);
K LSTATUS __stdcall RegOpenKeyExW(HKEY, const WCHAR *, DWORD, DWORD, HKEY *);
K LSTATUS __stdcall RegCreateKeyExW(HKEY, const WCHAR *, DWORD, WCHAR *, DWORD, DWORD, void *, HKEY *, DWORD *);
K LSTATUS __stdcall RegCloseKey(HKEY);
K LSTATUS __stdcall RegQueryValueExW(HKEY, const WCHAR *, DWORD *, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegSetValueExW(HKEY, const WCHAR *, DWORD, DWORD, const void *, DWORD);
K LSTATUS __stdcall RegSetValueExA(HKEY, const char *, DWORD, DWORD, const void *, DWORD);
K LSTATUS __stdcall RegGetValueW(HKEY, const WCHAR *, const WCHAR *, DWORD, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegDeleteValueW(HKEY, const WCHAR *);
K LSTATUS __stdcall RegDeleteKeyW(HKEY, const WCHAR *);
K LSTATUS __stdcall RegDeleteTreeW(HKEY, const WCHAR *);
K LSTATUS __stdcall RegEnumKeyExW(HKEY, DWORD, WCHAR *, DWORD *, DWORD *, WCHAR *, DWORD *, void *);
K LSTATUS __stdcall RegEnumValueW(HKEY, DWORD, WCHAR *, DWORD *, DWORD *, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegQueryInfoKeyW(HKEY, WCHAR *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, void *);
K LSTATUS __stdcall RegFlushKey(HKEY);

#define HKCR ((HKEY)(long long)(int)0x80000000)
#define HKCU ((HKEY)(long long)(int)0x80000001)
#define HKLM ((HKEY)(long long)(int)0x80000002)
#define REG_SZ 1
#define REG_EXPAND_SZ 2
#define REG_BINARY 3
#define REG_DWORD 4
#define REG_MULTI_SZ 7
#define REG_QWORD 11
#define RRF_RT_ANY 0xffff
#define RRF_RT_REG_SZ 2
#define RRF_RT_REG_DWORD 0x10

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
static WCHAR wbuf[8][128];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("the registry on atlas, arm64\n");
#else
    say("the registry on atlas, x86-64\n");
#endif
    HKEY version = 0;
    WCHAR text[128];
    DWORD size = sizeof text, type = 0, dword = 0;
    LSTATUS opened = RegOpenKeyExW(HKLM, W(0, "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"), 0, 0x20019, &version);
    LSTATUS got = RegQueryValueExW(version, W(1, "ProductName"), 0, &type, text, &size);
    check(opened == 0 && got == 0 && type == REG_SZ && weq(text, "Windows 10 Pro") && size == 30, "a fresh machine's HKLM: ProductName");
    size = sizeof dword;
    check(RegGetValueW(version, 0, W(1, "CurrentMajorVersionNumber"), RRF_RT_REG_DWORD, &type, &dword, &size) == 0 && dword == 10, "RegGetValueW, a DWORD");
    check(RegGetValueW(version, 0, W(1, "CurrentMajorVersionNumber"), RRF_RT_REG_SZ, 0, 0, &size) == 1630, "of a type it wasn't asked for: ERROR_UNSUPPORTED_TYPE");
    RegCloseKey(version);
    size = sizeof text;
    check(RegGetValueW(HKLM, W(0, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion"), W(1, "ProgramFilesPath"), RRF_RT_REG_SZ, &type, text, &size) == 0
              && type == REG_SZ && weq(text, "C:\\Program Files"),
          "REG_EXPAND_SZ, expanded by RegGetValueW");

    HKEY key = 0, again = 0;
    DWORD disposition = 0, again_disposition = 0;
    LSTATUS made = RegCreateKeyExW(HKCU, W(0, "Software\\Carbon\\Test"), 0, 0, 0, 0xf003f, 0, &key, &disposition);
    RegCreateKeyExW(HKCU, W(0, "software\\CARBON\\test"), 0, 0, 0, 0xf003f, 0, &again, &again_disposition);
    check(made == 0 && disposition == 1 && again_disposition == 2, "RegCreateKeyExW: made, then opened (in another case)");
    RegCloseKey(again);

    static const WCHAR name[] = {'a', 't', 'l', 'a', 's', 0};
    static const WCHAR multi[] = {'o', 'n', 'e', 0, 't', 'w', 'o', 0, 0};
    DWORD seven = 7;
    u64 big = 0x123456789abcdef0ULL;
    unsigned char blob[3] = {1, 2, 3};
    LSTATUS s1 = RegSetValueExW(key, W(0, "Name"), 0, REG_SZ, name, sizeof name);
    LSTATUS s2 = RegSetValueExW(key, W(0, "Count"), 0, REG_DWORD, &seven, 4);
    LSTATUS s3 = RegSetValueExW(key, W(0, "Blob"), 0, REG_BINARY, blob, 3);
    LSTATUS s4 = RegSetValueExW(key, W(0, "List"), 0, REG_MULTI_SZ, multi, sizeof multi);
    LSTATUS s5 = RegSetValueExW(key, W(0, "Big"), 0, REG_QWORD, &big, 8);
    LSTATUS s6 = RegSetValueExW(key, 0, 0, REG_SZ, name, sizeof name);
    check(!s1 && !s2 && !s3 && !s4 && !s5 && !s6, "RegSetValueExW: strings, numbers, binary, lists, and the default value");

    size = 0;
    LSTATUS just_size = RegQueryValueExW(key, W(0, "NAME"), 0, &type, 0, &size);
    DWORD needed = size;
    size = 4;
    LSTATUS small = RegQueryValueExW(key, W(0, "name"), 0, &type, text, &size);
    check(just_size == 0 && needed == sizeof name && small == 234 && size == sizeof name, "asking its size, and ERROR_MORE_DATA");
    u64 got_big = 0;
    size = 8;
    RegQueryValueExW(key, W(0, "Big"), 0, &type, &got_big, &size);
    size = sizeof text;
    RegQueryValueExW(key, 0, 0, &type, text, &size);
    check(got_big == big && weq(text, "atlas"), "a QWORD and the default value, read back");
    RegSetValueExA(key, "Ansi", 0, REG_SZ, "ansi", 5);
    size = sizeof text;
    check(RegQueryValueExW(key, W(0, "Ansi"), 0, &type, text, &size) == 0 && weq(text, "ansi") && size == 10, "RegSetValueExA stores UTF-16");

    DWORD subkeys = 99, values = 0, longest = 0;
    RegQueryInfoKeyW(key, 0, 0, 0, &subkeys, 0, 0, &values, &longest, 0, 0, 0);
    check(subkeys == 0 && values == 7 && longest == 5, "RegQueryInfoKeyW");
    int listed = 0, saw_blob = 0;
    for (DWORD i = 0;; i++) {
        WCHAR value_name[64];
        DWORD name_size = 64;
        if (RegEnumValueW(key, i, value_name, &name_size, 0, &type, 0, 0) != 0) break;
        listed++;
        saw_blob |= weq(value_name, "Blob") && type == REG_BINARY;
    }
    check(listed == 7 && saw_blob, "RegEnumValueW, to the end");

    HKEY carbon = 0;
    RegOpenKeyExW(HKCU, W(0, "Software\\Carbon"), 0, 0x20019, &carbon);
    WCHAR sub[64];
    DWORD sub_size = 64;
    LSTATUS e0 = RegEnumKeyExW(carbon, 0, sub, &sub_size, 0, 0, 0, 0);
    sub_size = 64;
    LSTATUS e1 = RegEnumKeyExW(carbon, 1, sub, &sub_size, 0, 0, 0, 0);
    check(e0 == 0 && e1 == 259, "RegEnumKeyExW, to ERROR_NO_MORE_ITEMS");
    sub_size = 64;
    RegEnumKeyExW(carbon, 0, sub, &sub_size, 0, 0, 0, 0);
    check(weq(sub, "Test") && sub_size == 4, "a key keeps the case it was made with");

    check(RegDeleteValueW(key, W(0, "Count")) == 0 && RegQueryValueExW(key, W(0, "Count"), 0, 0, 0, 0) == 2, "RegDeleteValueW");
    check(RegDeleteKeyW(HKCU, W(0, "Software\\Carbon")) == 5, "RegDeleteKeyW on a key with subkeys: ERROR_ACCESS_DENIED");
    check(RegDeleteTreeW(HKCU, W(0, "Software\\Carbon")) == 0, "RegDeleteTreeW");
    HKEY gone = 0;
    check(RegOpenKeyExW(HKCU, W(0, "Software\\Carbon\\Test"), 0, 0x20019, &gone) == 2, "and it's gone: ERROR_FILE_NOT_FOUND");
    check(RegSetValueExW(key, W(0, "Late"), 0, REG_DWORD, &seven, 4) == 1018, "a handle to a deleted key: ERROR_KEY_DELETED");
    RegCloseKey(key);
    RegCloseKey(carbon);

    HKEY ext = 0, seen = 0;
    RegCreateKeyExW(HKCR, W(0, ".carbon"), 0, 0, 0, 0xf003f, 0, &ext, 0);
    check(RegOpenKeyExW(HKLM, W(0, "SOFTWARE\\Classes\\.carbon"), 0, 0x20019, &seen) == 0, "HKEY_CLASSES_ROOT is HKLM\\SOFTWARE\\Classes");
    RegCloseKey(ext);
    RegCloseKey(seen);

    check(RegFlushKey(HKLM) == 0 && GetFileAttributesW(W(0, "C:\\Windows\\System32\\config\\REGISTRY")) != 0xffffffff, "RegFlushKey saves it, where Windows keeps its hives");

    ExitProcess(failures);
}
