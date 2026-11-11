// Loaded at run time by the loader test, and found by name and by ordinal.
__declspec(dllexport) int mul(int a, int b) { return a * b; }
int __stdcall DllMain(void *m, unsigned r, void *x) { (void)m; (void)r; (void)x; return 1; }
