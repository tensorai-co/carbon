// Loaded and freed by the loader test: it imports mylib2, so it holds a reference to it, and it tells the program
// when it's detached, and how (1: by FreeLibrary; 2: as the process ends).
typedef unsigned int DWORD;
__declspec(dllimport) int mul(int, int);
static int *log;
__declspec(dllexport) void watch(int *p) { log = p; }
__declspec(dllexport) int triple(int x) { return mul(3, x); }
int __stdcall DllMain(void *m, DWORD reason, void *reserved) {
    (void)m;
    if (reason == 0 && log) *log = reserved ? 2 : 1;
    return 1;
}
