// Files and folders on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// It checks: the current directory; folders made, refused when there, and removed; CREATE_NEW, OPEN_EXISTING,
// OPEN_ALWAYS and CREATE_ALWAYS with their last errors; reads, seeks, a read at the end, sizes, SetEndOfFile; names in
// any case; GetFinalPathNameByHandleW in the disk's own case; attributes, and ERROR_FILE_NOT_FOUND against
// ERROR_PATH_NOT_FOUND; relative paths and GetFullPathNameW; FindFirstFileW/FindNextFileW with wildcards and .; copy,
// move, replace, delete; ERROR_DIR_NOT_EMPTY; delete-on-close; the temp folder; System32's kernel32.dll read as a PE
// from memory, not writable, and listed with ntdll; NUL and CONOUT$

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned long long u64;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef struct { DWORD attributes; DWORD times[6]; DWORD size_high, size_low, reserved[2]; WCHAR name[260]; WCHAR alternate[14]; } FIND_DATA;
typedef struct { DWORD attributes; DWORD times[6]; DWORD size_high, size_low; } ATTRIBUTE_DATA;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K BOOL __stdcall ReadFile(HANDLE, void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K HANDLE __stdcall CreateFileA(const char *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K BOOL __stdcall CloseHandle(HANDLE);
K DWORD __stdcall SetFilePointer(HANDLE, int, int *, DWORD);
K BOOL __stdcall GetFileSizeEx(HANDLE, u64 *);
K DWORD __stdcall GetFileType(HANDLE);
K BOOL __stdcall SetEndOfFile(HANDLE);
K DWORD __stdcall GetFileAttributesW(const WCHAR *);
K BOOL __stdcall GetFileAttributesExW(const WCHAR *, int, ATTRIBUTE_DATA *);
K BOOL __stdcall DeleteFileW(const WCHAR *);
K BOOL __stdcall CreateDirectoryW(const WCHAR *, void *);
K BOOL __stdcall RemoveDirectoryW(const WCHAR *);
K BOOL __stdcall MoveFileExW(const WCHAR *, const WCHAR *, DWORD);
K BOOL __stdcall CopyFileW(const WCHAR *, const WCHAR *, BOOL);
K HANDLE __stdcall FindFirstFileW(const WCHAR *, FIND_DATA *);
K BOOL __stdcall FindNextFileW(HANDLE, FIND_DATA *);
K BOOL __stdcall FindClose(HANDLE);
K DWORD __stdcall GetCurrentDirectoryW(DWORD, WCHAR *);
K BOOL __stdcall SetCurrentDirectoryW(const WCHAR *);
K DWORD __stdcall GetFullPathNameW(const WCHAR *, DWORD, WCHAR *, WCHAR **);
K DWORD __stdcall GetTempPathW(DWORD, WCHAR *);
K DWORD __stdcall GetFinalPathNameByHandleW(HANDLE, WCHAR *, DWORD, DWORD);

#define GENERIC_READ 0x80000000
#define GENERIC_WRITE 0x40000000
#define CREATE_NEW 1
#define CREATE_ALWAYS 2
#define OPEN_EXISTING 3
#define OPEN_ALWAYS 4
#define INVALID ((HANDLE)-1)
#define NO_ATTRIBUTES 0xffffffff

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
// UTF-16 literals, built from ASCII at run time: there's no wide string support without a C runtime here.
static WCHAR wbuf[8][128];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }
static int eq(const char *a, const char *b, unsigned n) { for (unsigned i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }
static HANDLE open_w(const char *path, DWORD access, DWORD disposition) { return CreateFileW(W(7, path), access, 3, 0, disposition, 0x80, 0); }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("files on atlas, arm64\n");
#else
    say("files on atlas, x86-64\n");
#endif
    WCHAR path[260];
    check(GetCurrentDirectoryW(260, path) == 3 && weq(path, "C:\\"), "the current directory is where the program is");
    check(GetFileType(out) == 2, "the console is a character device");

    check(CreateDirectoryW(W(0, "C:\\work"), 0), "CreateDirectoryW");
    check(!CreateDirectoryW(W(0, "C:\\work"), 0) && GetLastError() == 183, "again: ERROR_ALREADY_EXISTS");

    HANDLE f = open_w("C:\\work\\a.txt", GENERIC_WRITE, CREATE_NEW);
    DWORD n = 0;
    BOOL wrote = WriteFile(f, "hello world", 11, &n, 0);
    check(f != INVALID && wrote && n == 11 && GetFileType(f) == 1, "CreateFileW (CREATE_NEW) and WriteFile");
    CloseHandle(f);
    check(open_w("C:\\work\\a.txt", GENERIC_WRITE, CREATE_NEW) == INVALID && GetLastError() == 80, "CREATE_NEW on a file that's there: ERROR_FILE_EXISTS");

    char buf[64];
    f = open_w("c:\\WORK\\A.TXT", GENERIC_READ, OPEN_EXISTING);
    BOOL r1 = ReadFile(f, buf, 5, &n, 0);
    int first = r1 && n == 5 && eq(buf, "hello", 5);
    SetFilePointer(f, 1, 0, 1);
    BOOL r2 = ReadFile(f, buf, 64, &n, 0);
    int rest = r2 && n == 5 && eq(buf, "world", 5);
    BOOL r3 = ReadFile(f, buf, 64, &n, 0);
    check(f != INVALID && first && rest, "opened by another case, read, seeked, read on");
    check(r3 && n == 0, "a read at the end reads nothing, and succeeds");
    u64 size = 0;
    check(GetFileSizeEx(f, &size) && size == 11, "GetFileSizeEx");
    WCHAR final[260];
    check(GetFinalPathNameByHandleW(f, final, 260, 0) > 0 && weq(final, "\\\\?\\C:\\work\\a.txt"), "GetFinalPathNameByHandleW");
    CloseHandle(f);

    ATTRIBUTE_DATA data;
    check(GetFileAttributesW(W(0, "C:\\work")) == 0x10 && GetFileAttributesExW(W(1, "C:\\work\\a.txt"), 0, &data) && data.size_low == 11 && !(data.attributes & 0x10),
          "GetFileAttributesW and GetFileAttributesExW");
    check(GetFileAttributesW(W(0, "C:\\work\\nope.txt")) == NO_ATTRIBUTES && GetLastError() == 2, "a missing file: ERROR_FILE_NOT_FOUND");
    check(GetFileAttributesW(W(0, "C:\\nowhere\\nope.txt")) == NO_ATTRIBUTES && GetLastError() == 3, "a missing folder: ERROR_PATH_NOT_FOUND");

    f = open_w("C:\\work\\a.txt", GENERIC_WRITE, OPEN_ALWAYS);
    check(f != INVALID && GetLastError() == 183, "OPEN_ALWAYS on a file that's there says ERROR_ALREADY_EXISTS");
    SetFilePointer(f, 5, 0, 0);
    SetEndOfFile(f);
    GetFileSizeEx(f, &size);
    CloseHandle(f);
    check(size == 5, "SetEndOfFile");

    check(SetCurrentDirectoryW(W(0, "C:\\work")), "SetCurrentDirectoryW");
    f = CreateFileA("b.txt", GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0x80, 0);
    WriteFile(f, "bee", 3, &n, 0);
    CloseHandle(f);
    WCHAR *file_part = 0;
    DWORD full = GetFullPathNameW(W(0, "b.txt"), 260, path, &file_part);
    check(full == 13 && weq(path, "C:\\work\\b.txt") && file_part == path + 8, "relative paths, CreateFileA, GetFullPathNameW");

    FIND_DATA found;
    HANDLE find = FindFirstFileW(W(0, "*.txt"), &found);
    int seen = 0, a_seen = 0, b_seen = 0;
    if (find != INVALID) {
        do {
            seen++;
            a_seen |= weq(found.name, "a.txt") && found.size_low == 5;
            b_seen |= weq(found.name, "b.txt") && found.size_low == 3;
        } while (FindNextFileW(find, &found));
    }
    int ended = GetLastError() == 18;
    FindClose(find);
    check(seen == 2 && a_seen && b_seen && ended, "FindFirstFileW and FindNextFileW, to ERROR_NO_MORE_FILES");
    find = FindFirstFileW(W(0, "C:\\work\\*"), &found);
    int dot = weq(found.name, ".");
    FindClose(find);
    check(dot, "a folder's listing starts with .");

    check(CopyFileW(W(0, "a.txt"), W(1, "c.txt"), 1) && !CopyFileW(W(0, "a.txt"), W(1, "c.txt"), 1), "CopyFileW, failing if it's there");
    check(MoveFileExW(W(0, "c.txt"), W(1, "d.txt"), 0) && GetFileAttributesW(W(2, "c.txt")) == NO_ATTRIBUTES, "MoveFileExW");
    check(!MoveFileExW(W(0, "b.txt"), W(1, "d.txt"), 0) && MoveFileExW(W(0, "b.txt"), W(1, "d.txt"), 1), "replacing only with MOVEFILE_REPLACE_EXISTING");
    check(!RemoveDirectoryW(W(0, "C:\\work")) && GetLastError() == 145, "a folder with files in it: ERROR_DIR_NOT_EMPTY");
    BOOL gone = DeleteFileW(W(0, "a.txt")) && DeleteFileW(W(1, "d.txt"));
    SetCurrentDirectoryW(W(0, "C:\\"));
    check(gone && RemoveDirectoryW(W(0, "C:\\work")), "DeleteFileW, then RemoveDirectoryW");

    f = CreateFileW(W(0, "C:\\Windows\\Temp\\scratch"), GENERIC_WRITE, 0, 0, CREATE_ALWAYS, 0x04000000, 0);
    int was_there = GetFileAttributesW(W(0, "C:\\Windows\\Temp\\scratch")) != NO_ATTRIBUTES;
    CloseHandle(f);
    check(was_there && GetFileAttributesW(W(0, "C:\\Windows\\Temp\\scratch")) == NO_ATTRIBUTES, "FILE_FLAG_DELETE_ON_CLOSE");
    check(GetTempPathW(260, path) > 0 && GetFileAttributesW(path) == 0x10, "GetTempPathW names a folder that's there");

    f = open_w("C:\\Windows\\System32\\KERNEL32.DLL", GENERIC_READ, OPEN_EXISTING);
    BOOL rk = ReadFile(f, buf, 2, &n, 0);
    check(f != INVALID && rk && n == 2 && eq(buf, "MZ", 2), "the system layer: kernel32.dll reads as a PE, made in memory");
    CloseHandle(f);
    check(open_w("C:\\Windows\\System32\\kernel32.dll", GENERIC_WRITE, OPEN_EXISTING) == INVALID && GetLastError() == 5, "and can't be written");
    find = FindFirstFileW(W(0, "C:\\Windows\\System32\\*.dll"), &found);
    int k32 = 0, ntdll = 0;
    if (find != INVALID) {
        do { k32 |= weq(found.name, "KERNEL32.dll"); ntdll |= weq(found.name, "ntdll.dll"); } while (FindNextFileW(find, &found));
    }
    FindClose(find);
    check(k32 && ntdll, "System32 lists atlas's DLLs and ntdll");

    f = CreateFileA("NUL", GENERIC_WRITE, 0, 0, OPEN_EXISTING, 0, 0);
    BOOL nul = WriteFile(f, "gone", 4, &n, 0) && n == 4;
    CloseHandle(f);
    f = CreateFileA("CONOUT$", GENERIC_WRITE, 0, 0, OPEN_EXISTING, 0, 0);
    BOOL con = WriteFile(f, "(through CONOUT$) ", 18, &n, 0);
    CloseHandle(f);
    check(nul && con, "NUL and CONOUT$");

    ExitProcess(failures);
}
