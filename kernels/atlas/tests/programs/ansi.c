// Code pages and the *A functions, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: the ANSI and OEM code pages and what they are; MultiByteToWideChar and WideCharToMultiByte in 1252, 437
// and UTF-8, sizes, buffers too small, invalid characters and default characters; and *A functions reading and giving
// back strings in 1252, so a name with é in it is the same name to the *W functions: files and folders, paths, the
// module's name, the registry, the environment, named objects, accounts and SIDs, locales and version information.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef unsigned short WCHAR;
typedef long LSTATUS;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K DWORD __stdcall GetACP(void);
K DWORD __stdcall GetOEMCP(void);
K int __stdcall IsValidCodePage(DWORD);
K int __stdcall GetCPInfo(DWORD, void *);
K int __stdcall MultiByteToWideChar(DWORD, DWORD, const char *, int, WCHAR *, int);
K int __stdcall WideCharToMultiByte(DWORD, DWORD, const WCHAR *, int, char *, int, const char *, int *);
K HANDLE __stdcall CreateFileA(const char *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K int __stdcall CloseHandle(HANDLE);
K HANDLE __stdcall FindFirstFileW(const WCHAR *, void *);
K HANDLE __stdcall FindFirstFileA(const char *, void *);
K int __stdcall FindClose(HANDLE);
K DWORD __stdcall GetFullPathNameA(const char *, DWORD, char *, char **);
K DWORD __stdcall GetSystemDirectoryA(char *, DWORD);
K DWORD __stdcall GetWindowsDirectoryA(char *, DWORD);
K DWORD __stdcall GetTempPathA(DWORD, char *);
K DWORD __stdcall GetModuleFileNameA(HANDLE, char *, DWORD);
K int __stdcall SetCurrentDirectoryA(const char *);
K DWORD __stdcall GetCurrentDirectoryA(DWORD, char *);
K int __stdcall MoveFileA(const char *, const char *);
K int __stdcall CopyFileA(const char *, const char *, int);
K int __stdcall GetFileAttributesExA(const char *, int, void *);
K int __stdcall SetFileAttributesA(const char *, DWORD);
K DWORD __stdcall GetFileAttributesA(const char *);
K int __stdcall CreateDirectoryA(const char *, void *);
K int __stdcall RemoveDirectoryA(const char *);
K int __stdcall DeleteFileA(const char *);
K HANDLE __stdcall CreateEventA(void *, int, int, const char *);
K HANDLE __stdcall OpenEventA(DWORD, int, const char *);
K int __stdcall SetEnvironmentVariableA(const char *, const char *);
K DWORD __stdcall GetEnvironmentVariableW(const WCHAR *, WCHAR *, DWORD);
K int __stdcall EnumSystemLocalesA(int (__stdcall *)(char *), DWORD);
K LSTATUS __stdcall RegCreateKeyA(HANDLE, const char *, HANDLE *);
K LSTATUS __stdcall RegSetValueExA(HANDLE, const char *, DWORD, DWORD, const void *, DWORD);
K LSTATUS __stdcall RegQueryValueExW(HANDLE, const WCHAR *, DWORD *, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegGetValueA(HANDLE, const char *, const char *, DWORD, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegEnumValueA(HANDLE, DWORD, char *, DWORD *, DWORD *, DWORD *, void *, DWORD *);
K LSTATUS __stdcall RegEnumKeyExA(HANDLE, DWORD, char *, DWORD *, DWORD *, char *, DWORD *, void *);
K LSTATUS __stdcall RegQueryInfoKeyA(HANDLE, char *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, DWORD *, void *);
K LSTATUS __stdcall RegDeleteTreeA(HANDLE, const char *);
K LSTATUS __stdcall RegCloseKey(HANDLE);
K int __stdcall GetUserNameA(char *, DWORD *);
K int __stdcall ConvertSidToStringSidA(void *, char **);
K int __stdcall ConvertStringSidToSidA(const char *, void **);
K int __stdcall LookupAccountSidA(const char *, void *, char *, DWORD *, char *, DWORD *, DWORD *);
K DWORD __stdcall GetFileVersionInfoSizeA(const char *, DWORD *);
K int __stdcall GetFileVersionInfoA(const char *, DWORD, DWORD, void *);
K int __stdcall VerQueryValueA(const void *, const char *, void **, DWORD *);

#define HKCU ((HANDLE)(long long)(int)0x80000001)
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
static int same(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }
static int wsame(const WCHAR *a, const WCHAR *b) { while (*a && *a == *b) { a++; b++; } return *a == *b; }
static int ends(const char *s, const char *tail) { unsigned a = len(s), b = len(tail); return a >= b && same(s + a - b, tail); }

static const WCHAR cafe_w[] = {'c', 'a', 'f', 0xe9, 0};
static int saw_us;
static int __stdcall locale(char *id) { if (same(id, "00000409")) saw_us = 1; return 1; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("code pages and *A functions, arm64\n");
#else
    say("code pages and *A functions, x86-64\n");
#endif
    check(GetACP() == 1252 && GetOEMCP() == 437, "GetACP and GetOEMCP: 1252 and 437");
    check(IsValidCodePage(1252) && IsValidCodePage(437) && IsValidCodePage(65001) && !IsValidCodePage(932) && !IsValidCodePage(0), "IsValidCodePage");
    struct { DWORD max; unsigned char def[2], lead[12]; } cpi;
    check(GetCPInfo(1252, &cpi) && cpi.max == 1 && cpi.def[0] == '?' && GetCPInfo(65001, &cpi) && cpi.max == 4, "GetCPInfo");

    WCHAR w[32];
    int n = MultiByteToWideChar(0, 0, "caf\xe9 \x80", -1, 0, 0);
    check(n == 7, "MultiByteToWideChar: how many, with the NUL");
    n = MultiByteToWideChar(0, 0, "caf\xe9 \x80", -1, w, 32);
    check(n == 7 && w[3] == 0xe9 && w[5] == 0x20ac && w[6] == 0, "  1252: é and the euro sign");
    n = MultiByteToWideChar(437, 0, "\x82\xc9", 2, w, 32);
    check(n == 2 && w[0] == 0xe9 && w[1] == 0x2554, "  437: é and a box corner");
    n = MultiByteToWideChar(65001, 0, "\xc3\xa9\xe2\x9c\x93", 5, w, 32);
    check(n == 2 && w[0] == 0xe9 && w[1] == 0x2713, "  UTF-8");
    n = MultiByteToWideChar(65001, 8, "a\xff", 2, w, 32);
    check(n == 0 && GetLastError() == 1113, "  MB_ERR_INVALID_CHARS: ERROR_NO_UNICODE_TRANSLATION");
    n = MultiByteToWideChar(0, 0, "abcdef", -1, w, 3);
    check(n == 0 && GetLastError() == 122, "  too small a buffer: ERROR_INSUFFICIENT_BUFFER");

    static const WCHAR mixed[] = {'c', 'a', 'f', 0xe9, ' ', 0x20ac, ' ', 0x2713, 0};
    char a[32];
    int used = 0;
    n = WideCharToMultiByte(0, 0, mixed, -1, a, 32, 0, &used);
    check(n == 9 && same(a, "caf\xe9 \x80 ?") && used, "WideCharToMultiByte: 1252, with ? for what it hasn't, and says so");
    n = WideCharToMultiByte(0, 0, mixed, -1, a, 32, "#", 0);
    check(n == 9 && a[7] == '#', "  a default character of the program's");
    n = WideCharToMultiByte(65001, 0, mixed, -1, a, 32, 0, 0);
    check(n == 14 && (unsigned char)a[3] == 0xc3 && (unsigned char)a[4] == 0xa9, "  UTF-8");
    n = WideCharToMultiByte(65001, 0, mixed, -1, a, 32, 0, &used);
    check(n == 0 && GetLastError() == 87, "  UTF-8 with a default character asked about: ERROR_INVALID_PARAMETER");
    n = WideCharToMultiByte(437, 0, cafe_w, -1, a, 32, 0, 0);
    check(n == 5 && (unsigned char)a[3] == 0x82, "  437");

    // Files, by a name with é.
    CreateDirectoryA("C:\\ansi", 0);
    HANDLE f = CreateFileA("C:\\ansi\\caf\xe9.txt", 0x40000000, 0, 0, 2, 0, 0);
    CloseHandle(f);
    struct { DWORD attributes, t[6], high, low, r0, r1; WCHAR name[260], alt[14]; } fw;
    static const WCHAR pattern[] = {'C', ':', '\\', 'a', 'n', 's', 'i', '\\', '*', '.', 't', 'x', 't', 0};
    HANDLE h = FindFirstFileW(pattern, &fw);
    static const WCHAR cafe_txt[] = {'c', 'a', 'f', 0xe9, '.', 't', 'x', 't', 0};
    check(f != INVALID && h != INVALID && wsame(fw.name, cafe_txt), "CreateFileA with é in its name: FindFirstFileW sees café.txt");
    FindClose(h);
    struct { DWORD attributes, t[6], high, low, r0, r1; char name[260], alt[14]; } fa;
    h = FindFirstFileA("C:\\ansi\\*.txt", &fa);
    check(h != INVALID && same(fa.name, "caf\xe9.txt"), "FindFirstFileA gives it back in 1252");
    FindClose(h);
    char path[260], *file = 0;
    DWORD got = GetFullPathNameA("C:\\ansi\\caf\xe9.txt", 260, path, &file);
    check(got == len("C:\\ansi\\caf\xe9.txt") && same(path, "C:\\ansi\\caf\xe9.txt") && file && same(file, "caf\xe9.txt"), "GetFullPathNameA, and its file part");
    check(GetSystemDirectoryA(path, 260) && same(path, "C:\\Windows\\System32") && GetWindowsDirectoryA(path, 260) && same(path, "C:\\Windows") &&
              GetTempPathA(260, path) && same(path, "C:\\Windows\\Temp\\"),
          "GetSystemDirectoryA, GetWindowsDirectoryA, GetTempPathA");
    check(GetModuleFileNameA(0, path, 260) && ends(path, "ansi.exe"), "GetModuleFileNameA");
    check(SetCurrentDirectoryA("C:\\ansi") && GetCurrentDirectoryA(260, path) && same(path, "C:\\ansi"), "SetCurrentDirectoryA");
    check(CopyFileA("caf\xe9.txt", "copy.txt", 1) && MoveFileA("copy.txt", "moved\xe9.txt") && GetFileAttributesA("C:\\ansi\\moved\xe9.txt") != 0xffffffff, "CopyFileA and MoveFileA");
    struct { DWORD attributes; DWORD t[6]; DWORD high, low; } ad;
    check(SetFileAttributesA("moved\xe9.txt", 1) && GetFileAttributesExA("moved\xe9.txt", 0, &ad) && (ad.attributes & 1), "SetFileAttributesA and GetFileAttributesExA");
    SetFileAttributesA("moved\xe9.txt", 0x80);
    DeleteFileA("moved\xe9.txt");
    DeleteFileA("caf\xe9.txt");
    SetCurrentDirectoryA("C:\\");
    check(RemoveDirectoryA("C:\\ansi") && GetFileAttributesA("C:\\ansi") == 0xffffffff, "RemoveDirectoryA");

    // The registry.
    HANDLE key = 0;
    check(RegCreateKeyA(HKCU, "Software\\Ansi", &key) == 0 && RegSetValueExA(key, "Caf\xe9", 0, 1, "th\xe9", 4) == 0, "RegCreateKeyA, and RegSetValueExA with é");
    static const WCHAR cafe_cap[] = {'C', 'a', 'f', 0xe9, 0};
    WCHAR wvalue[16];
    DWORD size = sizeof wvalue, type = 0;
    check(RegQueryValueExW(key, cafe_cap, 0, &type, wvalue, &size) == 0 && wvalue[2] == 0xe9 && wvalue[3] == 0, "  as UTF-16, it's thé");
    char value[16];
    size = sizeof value;
    check(RegGetValueA(HKCU, "Software\\Ansi", "Caf\xe9", 2, 0, value, &size) == 0 && same(value, "th\xe9") && size == 4, "RegGetValueA");
    char name[32];
    DWORD name_size = 32;
    size = sizeof value;
    check(RegEnumValueA(key, 0, name, &name_size, 0, &type, (void *)value, &size) == 0 && same(name, "Caf\xe9") && name_size == 4 && same(value, "th\xe9"), "RegEnumValueA");
    HANDLE sub = 0;
    RegCreateKeyA(key, "Sub\xe9", &sub);
    RegCloseKey(sub);
    name_size = 32;
    check(RegEnumKeyExA(key, 0, name, &name_size, 0, 0, 0, 0) == 0 && same(name, "Sub\xe9"), "RegEnumKeyExA");
    DWORD subkeys = 0, values = 0;
    check(RegQueryInfoKeyA(key, 0, 0, 0, &subkeys, 0, 0, &values, 0, 0, 0, 0) == 0 && subkeys == 1 && values == 1, "RegQueryInfoKeyA");
    RegCloseKey(key);
    check(RegDeleteTreeA(HKCU, "Software\\Ansi") == 0, "RegDeleteTreeA");

    // Names, accounts and the rest.
    char user[16];
    DWORD user_size = 16;
    check(GetUserNameA(user, &user_size) && same(user, "carbon") && user_size == 7, "GetUserNameA");
    void *sid = 0;
    char *sid_text = 0;
    check(ConvertStringSidToSidA("S-1-5-32-544", &sid) && ConvertSidToStringSidA(sid, &sid_text) && same(sid_text, "S-1-5-32-544"), "ConvertStringSidToSidA and back");
    char account[32], domain[32];
    DWORD account_size = 32, domain_size = 32, use = 0;
    check(LookupAccountSidA(0, sid, account, &account_size, domain, &domain_size, &use) && same(account, "Administrators") && same(domain, "BUILTIN"), "LookupAccountSidA");
    HANDLE event = CreateEventA(0, 1, 0, "Caf\xe9" "Event");
    HANDLE opened = OpenEventA(0x1f0003, 0, "Caf\xe9" "Event");
    check(event && opened, "CreateEventA and OpenEventA, named with é");
    static const WCHAR var_w[] = {'A', 'N', 'S', 'I', 'V', 'A', 'R', 0};
    check(SetEnvironmentVariableA("ANSIVAR", "caf\xe9") && GetEnvironmentVariableW(var_w, w, 32) == 4 && wsame(w, cafe_w), "SetEnvironmentVariableA in 1252, GetEnvironmentVariableW sees café");
    check(EnumSystemLocalesA(locale, 1) && saw_us, "EnumSystemLocalesA");

    DWORD info_size = GetFileVersionInfoSizeA("C:\\Windows\\System32\\kernel32.dll", 0);
    static char block[8192];
    void *at = 0;
    DWORD at_len = 0;
    int have = info_size && info_size <= sizeof block && GetFileVersionInfoA("C:\\Windows\\System32\\kernel32.dll", 0, info_size, block);
    check(have, "GetFileVersionInfoSizeA and GetFileVersionInfoA");
    unsigned short *translation = 0;
    DWORD tlen = 0;
    if (have && VerQueryValueA(block, "\\VarFileInfo\\Translation", (void **)&translation, &tlen) && tlen >= 4) {
        static const char hex[] = "0123456789ABCDEF";
        char q[64] = "\\StringFileInfo\\00000000\\FileVersion";
        unsigned v = (unsigned)translation[0] << 16 | translation[1];
        for (int i = 0; i < 8; i++) q[16 + i] = hex[(v >> (28 - 4 * i)) & 15];
        have = VerQueryValueA(block, q, &at, &at_len);
    } else {
        have = 0;
    }
    check(have && at && ((char *)at)[0] == '1' && ((char *)at)[1] == '0' && ((char *)at)[2] == '.', "VerQueryValueA: a string value, in ANSI");

    ExitProcess(failures);
}
