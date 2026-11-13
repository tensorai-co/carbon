// Sharing modes, links and file times, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: opens refused where another open doesn't share what they're for, or they don't share its; opens for
// attributes only sharing with anything; deleting and moving an open file only if it shares deleting, and a deleted
// file's data kept while it's open; hard links, as two names of one file; symbolic links to files and folders,
// absolute and relative, as GetFileAttributes, FindFirstFile and RemoveDirectory see them, and one that leads nowhere;
// and SetFileTime's times, as GetFileTime, GetFileAttributesEx and FindFirstFile give them back.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef unsigned short WCHAR;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) int __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) DWORD __stdcall GetLastError(void);
__declspec(dllimport) HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
__declspec(dllimport) int __stdcall CloseHandle(HANDLE);
__declspec(dllimport) DWORD __stdcall SetFilePointer(HANDLE, long, long *, DWORD);
__declspec(dllimport) int __stdcall DeleteFileW(const WCHAR *);
__declspec(dllimport) int __stdcall MoveFileExW(const WCHAR *, const WCHAR *, DWORD);
__declspec(dllimport) int __stdcall CreateDirectoryW(const WCHAR *, void *);
__declspec(dllimport) int __stdcall RemoveDirectoryW(const WCHAR *);
__declspec(dllimport) DWORD __stdcall GetFileAttributesW(const WCHAR *);
__declspec(dllimport) int __stdcall GetFileAttributesExW(const WCHAR *, int, void *);
__declspec(dllimport) int __stdcall GetFileInformationByHandle(HANDLE, void *);
__declspec(dllimport) int __stdcall CreateHardLinkW(const WCHAR *, const WCHAR *, void *);
__declspec(dllimport) unsigned char __stdcall CreateSymbolicLinkW(const WCHAR *, const WCHAR *, DWORD);
__declspec(dllimport) int __stdcall GetFileTime(HANDLE, u64 *, u64 *, u64 *);
__declspec(dllimport) int __stdcall SetFileTime(HANDLE, const u64 *, const u64 *, const u64 *);
__declspec(dllimport) HANDLE __stdcall FindFirstFileW(const WCHAR *, void *);
__declspec(dllimport) int __stdcall FindNextFileW(HANDLE, void *);
__declspec(dllimport) int __stdcall FindClose(HANDLE);

#define INVALID ((HANDLE)(long long)-1)
#define GENERIC_READ 0x80000000u
#define GENERIC_WRITE 0x40000000u
#define SHARE_READ 1
#define SHARE_WRITE 2
#define SHARE_DELETE 4
#define CREATE_ALWAYS 2
#define OPEN_EXISTING 3
#define BACKUP 0x02000000

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

// Wide strings, without a C runtime: W("...") into a buffer.
static WCHAR bufs[8][128];
static int next_buf;
static const WCHAR *W(const char *s) {
    WCHAR *b = bufs[next_buf++ & 7];
    int i = 0;
    for (; s[i]; i++) b[i] = (WCHAR)s[i];
    b[i] = 0;
    return b;
}
static HANDLE open_as(const char *path, DWORD access, DWORD share, DWORD disposition) { return CreateFileW(W(path), access, share, 0, disposition, 0, 0); }
static int put(HANDLE h, const char *s) { DWORD n; return WriteFile(h, s, len(s), &n, 0) && n == len(s); }
static int got(HANDLE h, const char *want) {
    char b[64];
    DWORD n = 0;
    if (!ReadFile(h, b, sizeof b, &n, 0) || n != len(want)) return 0;
    for (DWORD i = 0; i < n; i++) if (b[i] != want[i]) return 0;
    return 1;
}
static int read_file(const char *path, const char *want) {
    HANDLE h = open_as(path, GENERIC_READ, SHARE_READ | SHARE_WRITE | SHARE_DELETE, OPEN_EXISTING);
    int ok = h != INVALID && got(h, want);
    if (h != INVALID) CloseHandle(h);
    return ok;
}
static void make(const char *path, const char *text) {
    HANDLE h = open_as(path, GENERIC_WRITE, 0, CREATE_ALWAYS);
    put(h, text);
    CloseHandle(h);
}

// FILETIMEs are two DWORDs, so Windows' structures have them 4-aligned.
typedef struct { DWORD low, high; } FT;
static u64 ft(FT t) { return (u64)t.high << 32 | t.low; }
typedef struct { DWORD attributes; FT created, accessed, written; DWORD high, low; } ATTRS;
typedef struct { DWORD attributes; FT created, accessed, written; DWORD high, low, reserved0, reserved1; WCHAR name[260]; WCHAR alt[14]; } FIND;
typedef struct { DWORD attributes; FT created, accessed, written; DWORD serial, high, low, links, index_high, index_low; } BYHANDLE;

static int wsame(const WCHAR *a, const char *b) { while (*a && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int find_one(const char *pattern, const char *name, FIND *f) {
    HANDLE h = FindFirstFileW(W(pattern), f);
    if (h == INVALID) return 0;
    int found = 0;
    do { if (wsame(f->name, name)) { found = 1; break; } } while (FindNextFileW(h, f));
    FindClose(h);
    return found;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("sharing, links and times, arm64\n");
#elif defined(__i386__)
    say("sharing, links and times, x86 (WoW64)\n");
#else
    say("sharing, links and times, x86-64\n");
#endif
    CreateDirectoryW(W("C:\\links"), 0);

    // Sharing.
    HANDLE a = open_as("C:\\links\\shared.txt", GENERIC_READ | GENERIC_WRITE, SHARE_READ, CREATE_ALWAYS);
    put(a, "shared");
    HANDLE b = open_as("C:\\links\\shared.txt", GENERIC_READ, SHARE_READ | SHARE_WRITE, OPEN_EXISTING);
    check(a != INVALID && b != INVALID, "two opens for reading, sharing it");
    HANDLE c = open_as("C:\\links\\shared.txt", GENERIC_WRITE, SHARE_READ | SHARE_WRITE, OPEN_EXISTING);
    check(c == INVALID && GetLastError() == 32, "one for writing, which the first doesn't share: ERROR_SHARING_VIOLATION");
    c = open_as("C:\\links\\shared.txt", GENERIC_READ, 0, OPEN_EXISTING);
    check(c == INVALID && GetLastError() == 32, "one that won't share reading, while it's open for reading");
    c = open_as("C:\\links\\shared.txt", 0, 0, OPEN_EXISTING);
    check(c != INVALID, "one for its attributes only shares with anything");
    CloseHandle(c);
    check(!DeleteFileW(W("C:\\links\\shared.txt")) && GetLastError() == 32, "DeleteFile, while it's open without sharing deleting");
    check(!MoveFileExW(W("C:\\links\\shared.txt"), W("C:\\links\\moved.txt"), 0) && GetLastError() == 32, "and MoveFileEx");
    CloseHandle(a);
    CloseHandle(b);
    a = open_as("C:\\links\\shared.txt", GENERIC_WRITE, SHARE_READ | SHARE_WRITE, OPEN_EXISTING);
    check(a != INVALID, "once they're closed, it opens as it likes");
    CloseHandle(a);

    a = open_as("C:\\links\\doomed.txt", GENERIC_READ | GENERIC_WRITE, SHARE_READ | SHARE_DELETE, CREATE_ALWAYS);
    put(a, "kept");
    check(DeleteFileW(W("C:\\links\\doomed.txt")) && GetFileAttributesW(W("C:\\links\\doomed.txt")) == 0xffffffff && GetLastError() == 2,
          "DeleteFile of a file open sharing deleting: its name goes at once");
    b = open_as("C:\\links\\doomed.txt", GENERIC_WRITE, 0, CREATE_ALWAYS);
    check(b != INVALID, "and another can be made in its place");
    CloseHandle(b);
    char back[8];
    DWORD n = 0;
    SetFilePointer(a, 0, 0, 0);
    check(ReadFile(a, back, 8, &n, 0) && n == 4 && back[0] == 'k' && back[3] == 't', "while what had it open still reads what it wrote");
    CloseHandle(a);

    // Hard links.
    make("C:\\links\\one.txt", "linked");
    check(CreateHardLinkW(W("C:\\links\\two.txt"), W("C:\\links\\one.txt"), 0) && read_file("C:\\links\\two.txt", "linked"), "CreateHardLink: another name for it");
    a = open_as("C:\\links\\one.txt", GENERIC_READ, SHARE_READ, OPEN_EXISTING);
    b = open_as("C:\\links\\two.txt", GENERIC_READ, SHARE_READ, OPEN_EXISTING);
    BYHANDLE ia, ib;
    check(GetFileInformationByHandle(a, &ia) && GetFileInformationByHandle(b, &ib) && ia.links == 2 && ia.index_low == ib.index_low && ia.index_high == ib.index_high,
          "two links, one file number");
    CloseHandle(a);
    CloseHandle(b);
    check(DeleteFileW(W("C:\\links\\one.txt")) && read_file("C:\\links\\two.txt", "linked"), "one name deleted, the file's there by the other");
    check(!CreateHardLinkW(W("C:\\links\\two.txt"), W("C:\\links\\shared.txt"), 0) && GetLastError() == 183, "a link where there's a file: ERROR_ALREADY_EXISTS");
    check(!CreateHardLinkW(W("C:\\links\\dir2"), W("C:\\links"), 0) && GetLastError() == 5, "a link to a folder: ERROR_ACCESS_DENIED");

    // Symbolic links.
    make("C:\\links\\target.txt", "target");
    check(CreateSymbolicLinkW(W("C:\\links\\abs.lnk"), W("C:\\links\\target.txt"), 0) && read_file("C:\\links\\abs.lnk", "target"), "CreateSymbolicLink to a path: opening it reaches its target");
    check(CreateSymbolicLinkW(W("C:\\links\\rel.lnk"), W("target.txt"), 0) && read_file("C:\\links\\rel.lnk", "target"), "and from its own folder");
    DWORD attributes = GetFileAttributesW(W("C:\\links\\abs.lnk"));
    check(attributes != 0xffffffff && (attributes & 0x400) && !(attributes & 0x10), "GetFileAttributes sees the link: a reparse point");
    CreateDirectoryW(W("C:\\links\\folder"), 0);
    make("C:\\links\\folder\\inner.txt", "inner");
    check(CreateSymbolicLinkW(W("C:\\links\\dirlink"), W("C:\\links\\folder"), 1) && read_file("C:\\links\\dirlink\\inner.txt", "inner"), "a folder's link, and a file through it");
    attributes = GetFileAttributesW(W("C:\\links\\dirlink"));
    check((attributes & 0x410) == 0x410, "  a folder and a reparse point");
    FIND f;
    check(find_one("C:\\links\\*", "dirlink", &f) && (f.attributes & 0x410) == 0x410 && f.reserved0 == 0xA000000C, "FindFirstFile: its attributes and IO_REPARSE_TAG_SYMLINK");
    check(RemoveDirectoryW(W("C:\\links\\dirlink")) && GetFileAttributesW(W("C:\\links\\dirlink")) == 0xffffffff && read_file("C:\\links\\folder\\inner.txt", "inner"),
          "RemoveDirectory takes the link away, and leaves the folder");
    check(DeleteFileW(W("C:\\links\\abs.lnk")) && read_file("C:\\links\\target.txt", "target"), "DeleteFile of a link leaves its target");
    CreateSymbolicLinkW(W("C:\\links\\nowhere.lnk"), W("C:\\links\\missing.txt"), 0);
    a = open_as("C:\\links\\nowhere.lnk", GENERIC_READ, SHARE_READ, OPEN_EXISTING);
    check(a == INVALID && GetLastError() == 2 && GetFileAttributesW(W("C:\\links\\nowhere.lnk")) != 0xffffffff, "a link to nothing: there, but opening it finds nothing");

    // Times.
    a = open_as("C:\\links\\times.txt", GENERIC_READ | GENERIC_WRITE, SHARE_READ, CREATE_ALWAYS);
    u64 made = 0, read_at = 0, written = 0;
    check(GetFileTime(a, &made, &read_at, &written) && made > 0x01d0000000000000ull && made <= written, "a new file's times: when it was made");
    u64 c1 = 0x01d9000000000000ull, a1 = 0x01d9100000000000ull, w1 = 0x01d9200000000000ull;
    check(SetFileTime(a, &c1, &a1, &w1), "SetFileTime");
    check(GetFileTime(a, &made, &read_at, &written) && made == c1 && read_at == a1 && written == w1, "GetFileTime gives them back");
    u64 w2 = 0x01d9300000000000ull;
    check(SetFileTime(a, 0, 0, &w2) && GetFileTime(a, &made, &read_at, &written) && made == c1 && read_at == a1 && written == w2, "one at a time, the others as they were");
    CloseHandle(a);
    ATTRS at;
    check(GetFileAttributesExW(W("C:\\links\\times.txt"), 0, &at) && ft(at.created) == c1 && ft(at.accessed) == a1 && ft(at.written) == w2, "GetFileAttributesEx has them");
    check(find_one("C:\\links\\times*", "times.txt", &f) && ft(f.created) == c1 && ft(f.accessed) == a1 && ft(f.written) == w2, "and FindFirstFile");

    ExitProcess(failures);
}
