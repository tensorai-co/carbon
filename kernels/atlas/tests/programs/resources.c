// Resources on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it with resources.rc's.
//
// - FindResource(Ex), LoadResource, LockResource, SizeofResource: by number, by name in any case, `#number`,
//   languages (neutral, one asked for, the default, one that isn't there), and what's not found;
// - EnumResourceTypes, EnumResourceNames and EnumResourceLanguages, and a callback that stops one;
// - LoadString(W/A): strings in two blocks, an empty one, a buffer too small, and the string's own address;
// - version information: this program's (GetFileVersionInfo(Size)W, VerQueryValueW's fixed information, strings and
//   translation), and a system DLL's, from its file and from the module loaded.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned long long u64;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K HANDLE __stdcall FindResourceW(HANDLE, const WCHAR *, const WCHAR *);
K HANDLE __stdcall FindResourceA(HANDLE, const char *, const char *);
K HANDLE __stdcall FindResourceExW(HANDLE, const WCHAR *, const WCHAR *, WORD);
K HANDLE __stdcall LoadResource(HANDLE, HANDLE);
K void *__stdcall LockResource(HANDLE);
K DWORD __stdcall SizeofResource(HANDLE, HANDLE);
typedef int(__stdcall *TypeProc)(HANDLE, WCHAR *, long long);
typedef int(__stdcall *NameProc)(HANDLE, WCHAR *, WCHAR *, long long);
typedef int(__stdcall *LangProc)(HANDLE, WCHAR *, WCHAR *, WORD, long long);
K int __stdcall EnumResourceTypesW(HANDLE, TypeProc, long long);
K int __stdcall EnumResourceNamesW(HANDLE, const WCHAR *, NameProc, long long);
K int __stdcall EnumResourceLanguagesW(HANDLE, const WCHAR *, const WCHAR *, LangProc, long long);
__declspec(dllimport) int __stdcall LoadStringW(HANDLE, DWORD, WCHAR *, int);
__declspec(dllimport) int __stdcall LoadStringA(HANDLE, DWORD, char *, int);
__declspec(dllimport) DWORD __stdcall GetFileVersionInfoSizeW(const WCHAR *, DWORD *);
__declspec(dllimport) int __stdcall GetFileVersionInfoW(const WCHAR *, DWORD, DWORD, void *);
__declspec(dllimport) int __stdcall VerQueryValueW(const void *, const WCHAR *, void **, DWORD *);

#define ID(n) ((const WCHAR *)(u64)(n))
enum { RT_STRING = 6, RT_RCDATA = 10, RT_VERSION = 16, ERROR_FILE_NOT_FOUND = 2, ERROR_RESOURCE_TYPE_NOT_FOUND = 1813,
       ERROR_RESOURCE_NAME_NOT_FOUND = 1814, ERROR_RESOURCE_LANG_NOT_FOUND = 1815, ERROR_RESOURCE_ENUM_USER_STOP = 15106 };

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
static int wsame(const WCHAR *a, const char *b) { if (!a) return 0; while (*b) if (*a++ != (WCHAR)*b++) return 0; return *a == 0; }
static int wsame_n(const WCHAR *a, const char *b, unsigned n) { for (unsigned i = 0; i < n; i++) if (a[i] != (WCHAR)b[i]) return 0; return 1; }
static WCHAR wbuf[4][300];
static const WCHAR *W(int i, const char *s) { int n = 0; while (s[n]) { wbuf[i][n] = (WCHAR)s[n]; n++; } wbuf[i][n] = 0; return wbuf[i]; }

/// What a resource holds, by type and name and language, or 0.
static const char *bytes_of(HANDLE found, DWORD *size) {
    if (!found) return 0;
    *size = SizeofResource(0, found);
    return LockResource(LoadResource(0, found));
}

static int types_seen, named_type_first, ids_in_order;
static u64 last_type;
static int __stdcall on_type(HANDLE module, WCHAR *type, long long param) {
    if (types_seen == 0) named_type_first = (u64)type >> 16 && wsame(type, "MYTYPE") && param == 7;
    else {
        if ((u64)type <= last_type) ids_in_order = 0;
        last_type = (u64)type;
    }
    types_seen++;
    return 1;
}
static int names_seen;
static u64 names[8];
static int __stdcall on_name(HANDLE module, WCHAR *type, WCHAR *name, long long param) {
    if (names_seen < 8 && (u64)type == RT_RCDATA) names[names_seen] = (u64)name;
    names_seen++;
    return param != 1;
}
static int langs_seen;
static WORD langs[8];
static int __stdcall on_lang(HANDLE module, WCHAR *type, WCHAR *name, WORD lang, long long param) {
    if (langs_seen < 8 && (u64)name == 42) langs[langs_seen] = lang;
    langs_seen++;
    return 1;
}

typedef struct { DWORD signature, struct_version, file_ms, file_ls, product_ms, product_ls, mask, flags, os, type, subtype,
                 date_ms, date_ls; } FIXED;

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    DWORD size = 0;

    // Finding them.
    const char *raw = bytes_of(FindResourceW(0, ID(1), ID(RT_RCDATA)), &size);
    check(raw && size == 11 && same(raw, "raw bytes\x34\x12", 11), "FindResourceW by number: RCDATA 1, its bytes and size");
    raw = bytes_of(FindResourceExW(0, ID(RT_RCDATA), ID(42), 0x407), &size);
    check(raw && size == 7 && same(raw, "deutsch", 7), "FindResourceExW: in German, asked for");
    raw = bytes_of(FindResourceExW(0, ID(RT_RCDATA), ID(42), 0x409), &size);
    check(raw && same(raw, "english", 7), "  in US English, asked for");
    raw = bytes_of(FindResourceW(0, ID(42), ID(RT_RCDATA)), &size);
    check(raw && same(raw, "english", 7), "  with none asked for: US English before German");
    SetLastError(0);
    check(!FindResourceExW(0, ID(RT_RCDATA), ID(42), 0x40c) && GetLastError() == ERROR_RESOURCE_LANG_NOT_FOUND, "  French, which it isn't in");
    raw = bytes_of(FindResourceW(0, W(0, "hello"), W(1, "MyType")), &size);
    check(raw && size == 11 && same(raw, "hello there", 11), "a type and name of its own, in any case");
    raw = bytes_of(FindResourceA(0, "HELLO", "MYTYPE"), &size);
    check(raw && same(raw, "hello there", 11), "  FindResourceA");
    raw = bytes_of(FindResourceW(0, W(0, "#42"), W(1, "#10")), &size);
    check(raw && same(raw, "english", 7), "  #42 and #10: numbers");
    SetLastError(0);
    check(!FindResourceW(0, ID(43), ID(RT_RCDATA)) && GetLastError() == ERROR_RESOURCE_NAME_NOT_FOUND, "a name that isn't there");
    SetLastError(0);
    check(!FindResourceW(0, ID(1), ID(9)) && GetLastError() == ERROR_RESOURCE_TYPE_NOT_FOUND, "a type that isn't there");

    // Enumerating them.
    ids_in_order = 1;
    check(EnumResourceTypesW(0, on_type, 7) && types_seen == 4 && named_type_first && ids_in_order,
          "EnumResourceTypesW: MYTYPE by name first, then STRING, RCDATA and VERSION in order");
    check(EnumResourceNamesW(0, ID(RT_RCDATA), on_name, 0) && names_seen == 2 && names[0] == 1 && names[1] == 42, "EnumResourceNamesW: RCDATA's 1 and 42");
    names_seen = 0;
    SetLastError(0);
    check(!EnumResourceNamesW(0, ID(RT_RCDATA), on_name, 1) && names_seen == 1 && GetLastError() == ERROR_RESOURCE_ENUM_USER_STOP,
          "  a callback that stops it: FALSE, ERROR_RESOURCE_ENUM_USER_STOP");
    check(EnumResourceLanguagesW(0, ID(RT_RCDATA), ID(42), on_lang, 0) && langs_seen == 2 && langs[0] == 0x407 && langs[1] == 0x409,
          "EnumResourceLanguagesW: 42's German and US English");

    // Strings.
    WCHAR text[64];
    check(LoadStringW(0, 2, text, 64) == 20 && wsame(text, "two, a longer string"), "LoadStringW: string 2");
    check(LoadStringW(0, 17, text, 64) == 9 && wsame(text, "seventeen"), "  string 17, in the second block");
    check(LoadStringW(0, 2, text, 4) == 3 && wsame(text, "two"), "  a buffer of 4: three characters and a NUL");
    WCHAR *own = 0;
    check(LoadStringW(0, 2, (WCHAR *)&own, 0) == 20 && own && wsame_n(own, "two, a longer string", 20), "  a size of 0: the string's own address");
    text[0] = 'x';
    check(LoadStringW(0, 3, text, 64) == 0 && text[0] == 0, "  string 3, empty");
    text[0] = 'x';
    check(LoadStringW(0, 999, text, 64) == 0 && text[0] == 0, "  string 999, in no block");
    char ansi[16];
    check(LoadStringA(0, 1, ansi, 16) == 3 && same(ansi, "one", 4), "LoadStringA: string 1");

    // Version information.
    WCHAR self[260];
    GetModuleFileNameW(0, self, 260);
    DWORD handle = 1;
    DWORD info_size = GetFileVersionInfoSizeW(self, &handle);
    static unsigned char info[4096];
    check(info_size > 92 && info_size < sizeof info && handle == 0 && GetFileVersionInfoW(self, 0, info_size, info),
          "GetFileVersionInfoSizeW and GetFileVersionInfoW: this program's");
    FIXED *fixed = 0;
    DWORD value_len = 0;
    check(VerQueryValueW(info, W(0, "\\"), (void **)&fixed, &value_len) && value_len == 52 && fixed->signature == 0xfeef04bd &&
          fixed->file_ms == 0x10002 && fixed->file_ls == 0x30004 && fixed->type == 1,
          "VerQueryValueW: its fixed information, 1.2.3.4, an application");
    WCHAR *description = 0;
    check(VerQueryValueW(info, W(0, "\\StringFileInfo\\040904b0\\FileDescription"), (void **)&description, &value_len) &&
          wsame(description, "atlas resources test") && (value_len == 20 || value_len == 21),
          "  its description, by a path in any case");
    DWORD *translation = 0;
    check(VerQueryValueW(info, W(0, "\\VarFileInfo\\Translation"), (void **)&translation, &value_len) && value_len == 4 &&
          *translation == 0x04b00409,
          "  its translation: US English, Unicode");
    SetLastError(0);
    check(!VerQueryValueW(info, W(0, "\\StringFileInfo\\040904b0\\Nothing"), (void **)&description, &value_len), "  a value that isn't there");
    SetLastError(0);
    check(!GetFileVersionInfoSizeW(W(1, "C:\\nowhere.dll"), &handle) && GetLastError() == ERROR_FILE_NOT_FOUND, "  a file that isn't there");

    info_size = GetFileVersionInfoSizeW(W(1, "C:\\Windows\\System32\\kernel32.dll"), &handle);
    check(info_size > 0 && GetFileVersionInfoW(W(1, "C:\\Windows\\System32\\kernel32.dll"), 0, info_size, info) &&
          VerQueryValueW(info, W(0, "\\"), (void **)&fixed, &value_len) && fixed->file_ms == 0xa0000 && (fixed->file_ls >> 16) == 19041 && fixed->type == 2,
          "kernel32.dll's file: version 10.0.19041, a DLL");
    check(VerQueryValueW(info, W(0, "\\StringFileInfo\\040904B0\\OriginalFilename"), (void **)&description, &value_len) &&
          wsame(description, "KERNEL32.dll"),
          "  its original file name");
    HANDLE kernel32 = GetModuleHandleW(W(2, "kernel32.dll"));
    HANDLE version = FindResourceW(kernel32, ID(1), ID(RT_VERSION));
    const unsigned char *loaded = LockResource(LoadResource(kernel32, version));
    check(kernel32 && version && loaded && SizeofResource(kernel32, version) > 92 &&
          VerQueryValueW(loaded, W(0, "\\"), (void **)&fixed, &value_len) && fixed->file_ms == 0xa0000,
          "  and the module loaded: its version resource, found and read in place");

    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
