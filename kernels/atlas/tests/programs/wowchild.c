// A 64-bit program for atlas's WoW64 test: wow64.exe (32-bit) starts it, and it starts `wow64.exe child` (32-bit
// again). It exits 64 if it sees what a 64-bit program should (not WoW64, PROCESSOR_ARCHITECTURE AMD64 and no
// PROCESSOR_ARCHITEW6432, ProgramFiles the 64-bit one), and its 32-bit child exited 32, having seen what it should.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef int BOOL;
typedef unsigned short WCHAR;

__declspec(dllimport) BOOL __stdcall IsWow64Process(HANDLE, BOOL *);
__declspec(dllimport) HANDLE __stdcall GetCurrentProcess(void);
__declspec(dllimport) DWORD __stdcall GetEnvironmentVariableW(const WCHAR *, WCHAR *, DWORD);
__declspec(dllimport) BOOL __stdcall CreateProcessW(const WCHAR *, WCHAR *, void *, void *, BOOL, DWORD, void *, const WCHAR *, void *, void *);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
__declspec(dllimport) BOOL __stdcall GetExitCodeProcess(HANDLE, DWORD *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);

static int is(const WCHAR *name, const char *want) {
    WCHAR value[260];
    DWORD n = GetEnvironmentVariableW(name, value, 260);
    if (!want) return n == 0;
    DWORD i = 0;
    for (; i < n && want[i] && value[i] == (WCHAR)want[i]; i++) {}
    return i == n && want[i] == 0;
}

void mainCRTStartup(void) {
    BOOL wow = 1;
    IsWow64Process(GetCurrentProcess(), &wow);
    int ok = !wow && is(L"PROCESSOR_ARCHITECTURE", "AMD64") && is(L"PROCESSOR_ARCHITEW6432", 0) && is(L"ProgramFiles", "C:\\Program Files");
    WCHAR line[] = L"wow64.exe child";
    DWORD si[26] = {104};
    struct { HANDLE process, thread; DWORD pid, tid; } pi;
    DWORD code = 0;
    if (CreateProcessW(0, line, 0, 0, 0, 0, 0, 0, si, &pi)) {
        WaitForSingleObject(pi.process, 0xffffffff);
        GetExitCodeProcess(pi.process, &code);
    }
    ExitProcess(ok && code == 32 ? 64 : 2);
}
