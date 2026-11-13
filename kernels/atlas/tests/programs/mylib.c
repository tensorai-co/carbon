// An app's own DLL, for the loader test: it imports kernel32, needs relocating (it's linked at the program's own base),
// counts its DllMain calls, and forwards one export to kernel32 (which forwards it on to ntdll). When the program asks
// (`say_at_exit`), it says so as it hears the process end.
typedef unsigned int DWORD;
typedef void *HANDLE;
__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
static const char *words[] = {"relocated", "pointers"};
__declspec(dllexport) int attached;
__declspec(dllexport) int thread_attached;
__declspec(dllexport) int thread_detached;
__declspec(dllexport) int say_at_exit;
__declspec(dllexport) int add(int a, int b) { return a + b; }
__declspec(dllexport) const char *word(int i) { return words[i]; }
int __stdcall DllMain(void *module, DWORD reason, void *reserved) {
    (void)module;
    if (reason == 1) attached = attached + 1;
    if (reason == 2) thread_attached = thread_attached + 1;
    if (reason == 3) thread_detached = thread_detached + 1;
    if (reason == 0 && reserved && say_at_exit) {
        static const char text[] = "ok      mylib's DllMain hears DLL_PROCESS_DETACH as the process ends\n";
        DWORD n;
        WriteFile(GetStdHandle((DWORD)-11), text, sizeof text - 1, &n, 0);
    }
    return 1;
}
