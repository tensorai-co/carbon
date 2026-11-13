// The shell's core, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it.
//
// It checks: special folders' paths (the profile's, Program Files as the program's ISA sees it, everyone's start
// menu, made on the disk), their item ID lists and the paths back, a virtual folder's list with no path, a CSIDL
// that isn't one, ILCreateFromPathW; SHFileOperationW copying a folder tree, copying what a wildcard matches into a
// folder it makes, renaming, moving and deleting a tree, and failing for what isn't there; ShellExecuteExW running a
// program (this one, which exits with 42) and giving its process, ShellExecuteW, and what it can't open (a folder, a
// URL, a file that isn't there); SHGetFileInfoW's display and type names, attributes, the system image list, a
// program's type, and a file named but not there; and SHBrowseForFolderW, its folder given by its callback
// (BFFM_SETSELECTION, then BFFM_SELCHANGED) and taken with Enter, cancelled with Escape, and a path that isn't a
// folder refused.

typedef void *HANDLE;
typedef void *HWND;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned int UINT;
typedef __UINTPTR_TYPE__ WPARAM;
typedef __INTPTR_TYPE__ LPARAM;
typedef __INTPTR_TYPE__ LRESULT;
typedef long HRESULT;
typedef void ITEMIDLIST;
typedef int (__stdcall *BFFCALLBACK)(HWND, UINT, LPARAM, LPARAM);

#if defined(__i386__)
#pragma pack(push, 1)
#endif
typedef struct { HWND hwnd; UINT wFunc; const WCHAR *pFrom, *pTo; WORD fFlags; int fAnyOperationsAborted; void *hNameMappings; const WCHAR *lpszProgressTitle; } SHFILEOPSTRUCTW;
#if defined(__i386__)
#pragma pack(pop)
#endif
typedef struct {
    DWORD cbSize, fMask;
    HWND hwnd;
    const WCHAR *lpVerb, *lpFile, *lpParameters, *lpDirectory;
    int nShow;
    HANDLE hInstApp;
    void *lpIDList;
    const WCHAR *lpClass;
    HANDLE hkeyClass;
    DWORD dwHotKey;
    HANDLE hIcon, hProcess;
} SHELLEXECUTEINFOW;
typedef struct { HANDLE hIcon; int iIcon; DWORD dwAttributes; WCHAR szDisplayName[260]; WCHAR szTypeName[80]; } SHFILEINFOW;
typedef struct { HWND hwndOwner; ITEMIDLIST *pidlRoot; WCHAR *pszDisplayName; const WCHAR *lpszTitle; UINT ulFlags; BFFCALLBACK lpfn; LPARAM lParam; int iImage; } BROWSEINFOW;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);
K WCHAR *__stdcall GetCommandLineW(void);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K int __stdcall GetExitCodeProcess(HANDLE, DWORD *);
K int __stdcall CloseHandle(HANDLE);
K int __stdcall CreateDirectoryW(const WCHAR *, void *);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K DWORD __stdcall GetFileAttributesW(const WCHAR *);
K DWORD __stdcall GetTempPathW(DWORD, WCHAR *);
K void __stdcall CoTaskMemFree(void *);
K LRESULT __stdcall SendMessageW(HWND, UINT, WPARAM, LPARAM);
K int __stdcall PostMessageW(HWND, UINT, WPARAM, LPARAM);
K HRESULT __stdcall SHGetFolderPathW(HWND, int, HANDLE, DWORD, WCHAR *);
K int __stdcall SHGetSpecialFolderPathW(HWND, WCHAR *, int, int);
K HRESULT __stdcall SHGetSpecialFolderLocation(HWND, int, ITEMIDLIST **);
K int __stdcall SHGetPathFromIDListW(const ITEMIDLIST *, WCHAR *);
K void __stdcall ILFree(ITEMIDLIST *);
K ITEMIDLIST *__stdcall ILCreateFromPathW(const WCHAR *);
K int __stdcall SHFileOperationW(SHFILEOPSTRUCTW *);
K HANDLE __stdcall ShellExecuteW(HWND, const WCHAR *, const WCHAR *, const WCHAR *, const WCHAR *, int);
K int __stdcall ShellExecuteExW(SHELLEXECUTEINFOW *);
K __UINTPTR_TYPE__ __stdcall SHGetFileInfoW(const WCHAR *, DWORD, SHFILEINFOW *, UINT, UINT);
K ITEMIDLIST *__stdcall SHBrowseForFolderW(BROWSEINFOW *);

#define S_OK 0
#define E_INVALIDARG ((HRESULT)0x80070057)
#define CSIDL_PROGRAMS 0x02
#define CSIDL_DRIVES 0x11
#define CSIDL_DESKTOPDIRECTORY 0x10
#define CSIDL_COMMON_PROGRAMS 0x17
#define CSIDL_APPDATA 0x1a
#define CSIDL_PROGRAM_FILES 0x26
#define FO_MOVE 1
#define FO_COPY 2
#define FO_DELETE 3
#define FO_RENAME 4
#define FOF_SILENT 0x4
#define FOF_NOCONFIRMATION 0x10
#define FOF_NOCONFIRMMKDIR 0x200
#define SHGFI_DISPLAYNAME 0x200
#define SHGFI_TYPENAME 0x400
#define SHGFI_ATTRIBUTES 0x800
#define SHGFI_EXETYPE 0x2000
#define SHGFI_SYSICONINDEX 0x4000
#define SHGFI_SMALLICON 0x1
#define SHGFI_USEFILEATTRIBUTES 0x10
#define WM_KEYDOWN 0x100
#define VK_RETURN 0x0d
#define VK_ESCAPE 0x1b
#define BFFM_INITIALIZED 1
#define BFFM_SELCHANGED 2
#define BFFM_SETSELECTIONW 0x467

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
static WCHAR wbuf[6][300];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int wweq(const WCHAR *a, const WCHAR *b) { while (*b && *a == *b) { a++; b++; } return *a == 0 && *b == 0; }
static int wcontains(const WCHAR *a, const char *b) {
    for (; *a; a++) {
        int i = 0;
        while (b[i] && a[i] == (WCHAR)b[i]) i++;
        if (!b[i]) return 1;
    }
    return 0;
}
static WCHAR *join(WCHAR *d, const WCHAR *a, const char *b) { int n = 0; for (; a[n]; n++) d[n] = a[n]; for (int i = 0; b[i]; i++) d[n++] = (WCHAR)b[i]; d[n] = 0; d[n + 1] = 0; return d; }
static int is_dir(const WCHAR *p) { DWORD a = GetFileAttributesW(p); return a != 0xffffffff && (a & 0x10); }
static int exists(const WCHAR *p) { return GetFileAttributesW(p) != 0xffffffff; }
static void touch(const WCHAR *p) { HANDLE h = CreateFileW(p, 0x40000000, 0, 0, 2, 0x80, 0); if (h != (HANDLE)-1) CloseHandle(h); }

static int op(UINT func, const WCHAR *from, const WCHAR *to) {
    SHFILEOPSTRUCTW s = {0};
    s.wFunc = func;
    s.pFrom = from;
    s.pTo = to;
    s.fFlags = FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR;
    s.fAnyOperationsAborted = 7;
    int r = SHFileOperationW(&s);
    return r == 0 && s.fAnyOperationsAborted == 0 ? 0 : (r ? r : -1);
}

// SHBrowseForFolderW's callback: lParam says what to do once it's up.
static WCHAR changed_to[300];
static int __stdcall browse_callback(HWND h, UINT m, LPARAM a, LPARAM data) {
    if (m == BFFM_INITIALIZED) {
        if (data == 1) {
            SendMessageW(h, BFFM_SETSELECTIONW, 1, (LPARAM)W(4, "C:\\Windows"));
            PostMessageW(h, WM_KEYDOWN, VK_RETURN, 1);
        } else if (data == 2) {
            PostMessageW(h, WM_KEYDOWN, VK_ESCAPE, 1);
        } else {
            // Not a folder: Enter leaves it open, then Escape.
            SendMessageW(h, BFFM_SETSELECTIONW, 1, (LPARAM)W(4, "C:\\no such folder"));
            PostMessageW(h, WM_KEYDOWN, VK_RETURN, 1);
            PostMessageW(h, WM_KEYDOWN, VK_ESCAPE, 1);
        }
    }
    if (m == BFFM_SELCHANGED) SHGetPathFromIDListW((const ITEMIDLIST *)a, changed_to);
    return 0;
}

static WCHAR path[300], other[300], tmp[260], a[300], b[300], c[300], self[260], name[260];
static SHFILEINFOW info;

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    // Run by ShellExecute: exit with 42.
    if (wcontains(GetCommandLineW(), " child")) ExitProcess(42);
#if defined(__aarch64__)
    say("the shell on atlas, arm64\n");
#elif defined(__i386__)
    say("the shell on atlas, x86 (WoW64)\n");
#else
    say("the shell on atlas, x86-64\n");
#endif

    // ---- Special folders ----
    check(SHGetFolderPathW(0, CSIDL_APPDATA, 0, 0, path) == S_OK && weq(path, "C:\\Users\\carbon\\AppData\\Roaming"), "SHGetFolderPathW: AppData");
    SHGetFolderPathW(0, CSIDL_PROGRAM_FILES, 0, 0, path);
#if defined(__i386__)
    check(weq(path, "C:\\Program Files (x86)"), "Program Files, a 32-bit program's");
#else
    check(weq(path, "C:\\Program Files"), "Program Files, a 64-bit program's");
#endif
    check(SHGetFolderPathW(0, CSIDL_COMMON_PROGRAMS, 0, 0, path) == S_OK && weq(path, "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs") && is_dir(path),
          "everyone's start menu, there on the disk");
    check(SHGetSpecialFolderPathW(0, path, CSIDL_DESKTOPDIRECTORY, 0) && weq(path, "C:\\Users\\carbon\\Desktop"), "SHGetSpecialFolderPathW: the desktop");
    ITEMIDLIST *pidl = 0;
    SHGetFolderPathW(0, CSIDL_PROGRAMS, 0, 0, other);
    check(SHGetSpecialFolderLocation(0, CSIDL_PROGRAMS, &pidl) == S_OK && pidl && SHGetPathFromIDListW(pidl, path) && wweq(path, other),
          "SHGetSpecialFolderLocation, and its path back");
    CoTaskMemFree(pidl);
    pidl = 0;
    path[0] = 'x';
    check(SHGetSpecialFolderLocation(0, CSIDL_DRIVES, &pidl) == S_OK && pidl && !SHGetPathFromIDListW(pidl, path) && path[0] == 0,
          "a virtual folder: a list, with no path");
    ILFree(pidl);
    check(SHGetSpecialFolderLocation(0, 0xfe, &pidl) == E_INVALIDARG, "a CSIDL that isn't one");
    pidl = ILCreateFromPathW(W(0, "C:\\Windows"));
    check(pidl && SHGetPathFromIDListW(pidl, path) && weq(path, "C:\\Windows") && !ILCreateFromPathW(W(1, "C:\\nothing here")), "ILCreateFromPathW");
    ILFree(pidl);

    // ---- File operations ----
    GetTempPathW(260, tmp);
    CreateDirectoryW(join(a, tmp, "src"), 0);
    CreateDirectoryW(join(a, tmp, "src\\sub"), 0);
    touch(join(a, tmp, "src\\a.txt"));
    touch(join(a, tmp, "src\\sub\\b.txt"));
    touch(join(a, tmp, "src\\c.dat"));
    check(op(FO_COPY, join(a, tmp, "src"), join(b, tmp, "dst")) == 0 && exists(join(c, tmp, "dst\\sub\\b.txt")) && exists(join(c, tmp, "src\\a.txt")),
          "SHFileOperationW: FO_COPY, a whole folder");
    check(op(FO_COPY, join(a, tmp, "src\\*.txt"), join(b, tmp, "flat")) == 0 && exists(join(c, tmp, "flat\\a.txt")) && !exists(join(c, tmp, "flat\\c.dat")),
          "FO_COPY of a wildcard, into a folder it makes");
    check(op(FO_RENAME, join(a, tmp, "dst"), join(b, tmp, "dst2")) == 0 && !exists(join(c, tmp, "dst")) && exists(join(c, tmp, "dst2\\a.txt")), "FO_RENAME");
    CreateDirectoryW(join(a, tmp, "into"), 0);
    check(op(FO_MOVE, join(a, tmp, "dst2"), join(b, tmp, "into")) == 0 && exists(join(c, tmp, "into\\dst2\\sub\\b.txt")) && !exists(join(c, tmp, "dst2")),
          "FO_MOVE into a folder that's there");
    check(op(FO_DELETE, join(a, tmp, "into"), 0) == 0 && !exists(join(c, tmp, "into")), "FO_DELETE, a whole tree");
    check(op(FO_DELETE, join(a, tmp, "not there"), 0) != 0, "FO_DELETE of what isn't there fails");
    op(FO_DELETE, join(a, tmp, "src"), 0);
    op(FO_DELETE, join(a, tmp, "flat"), 0);

    // ---- Running programs ----
    GetModuleFileNameW(0, self, 260);
    SHELLEXECUTEINFOW sei = {0};
    sei.cbSize = sizeof sei;
    sei.fMask = 0x40; // SEE_MASK_NOCLOSEPROCESS
    sei.lpFile = self;
    sei.lpParameters = W(0, "child");
    sei.nShow = 1;
    DWORD code = 0;
    int ran = ShellExecuteExW(&sei);
    check(ran && (__UINTPTR_TYPE__)sei.hInstApp > 32 && sei.hProcess, "ShellExecuteExW: a program, and its process");
    if (sei.hProcess) {
        WaitForSingleObject(sei.hProcess, 10000);
        GetExitCodeProcess(sei.hProcess, &code);
        CloseHandle(sei.hProcess);
    }
    check(code == 42, "which ran with its parameters");
    check((__UINTPTR_TYPE__)ShellExecuteW(0, W(0, "open"), self, W(1, "child"), 0, 1) > 32, "ShellExecuteW");
    check((__UINTPTR_TYPE__)ShellExecuteW(0, 0, W(0, "C:\\Windows"), 0, 0, 1) == 31, "a folder: no association (no Explorer)");
    check((__UINTPTR_TYPE__)ShellExecuteW(0, 0, W(0, "https://example.com/"), 0, 0, 1) == 31, "a URL: the same");
    check((__UINTPTR_TYPE__)ShellExecuteW(0, 0, W(0, "C:\\nowhere\\x.exe"), 0, 0, 1) == 2, "a file that isn't there: SE_ERR_FNF");

    // ---- What a file is ----
    check(SHGetFileInfoW(W(0, "C:\\Windows"), 0, &info, sizeof info, SHGFI_DISPLAYNAME | SHGFI_TYPENAME | SHGFI_ATTRIBUTES) == 1
              && weq(info.szDisplayName, "Windows") && weq(info.szTypeName, "File folder") && (info.dwAttributes & 0x20000000),
          "SHGetFileInfoW: a folder's name, type and attributes");
    check(SHGetFileInfoW(W(0, "notes.txt"), 0x80, &info, sizeof info, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES) == 1 && weq(info.szTypeName, "TXT File"),
          "a file by its name alone (SHGFI_USEFILEATTRIBUTES)");
    check(SHGetFileInfoW(W(0, "C:\\no file"), 0, &info, sizeof info, SHGFI_TYPENAME) == 0, "a file that isn't there: 0");
    check(SHGetFileInfoW(self, 0, &info, sizeof info, SHGFI_SYSICONINDEX | SHGFI_SMALLICON) != 0, "SHGFI_SYSICONINDEX: the system image list");
    check((SHGetFileInfoW(self, 0, 0, 0, SHGFI_EXETYPE) & 0xffff) == 0x4550, "SHGFI_EXETYPE: a PE program");

    // ---- Choosing a folder ----
    BROWSEINFOW bi = {0};
    bi.pszDisplayName = name;
    bi.lpszTitle = W(5, "Choose a folder");
    bi.lpfn = browse_callback;
    bi.lParam = 1;
    pidl = SHBrowseForFolderW(&bi);
    check(pidl && SHGetPathFromIDListW(pidl, path) && weq(path, "C:\\Windows") && weq(name, "Windows"), "SHBrowseForFolderW: the folder its callback gave, and its name");
    check(weq(changed_to, "C:\\Windows"), "BFFM_SELCHANGED as it was given");
    ILFree(pidl);
    bi.lParam = 2;
    check(SHBrowseForFolderW(&bi) == 0, "cancelled: NULL");
    bi.lParam = 3;
    check(SHBrowseForFolderW(&bi) == 0, "a path that isn't a folder isn't taken");

    ExitProcess(failures);
}
