// An app's own DLL, for the loader test: it imports kernel32, needs relocating (it's linked at the program's own base),
// counts its DllMain calls, and forwards one export to kernel32 (which forwards it on to ntdll).
typedef unsigned int DWORD;
static const char *words[] = {"relocated", "pointers"};
__declspec(dllexport) int attached;
__declspec(dllexport) int thread_attached;
__declspec(dllexport) int add(int a, int b) { return a + b; }
__declspec(dllexport) const char *word(int i) { return words[i]; }
int __stdcall DllMain(void *module, DWORD reason, void *reserved) {
    (void)module; (void)reserved;
    if (reason == 1) attached = attached + 1;
    if (reason == 2) thread_attached = thread_attached + 1;
    return 1;
}
