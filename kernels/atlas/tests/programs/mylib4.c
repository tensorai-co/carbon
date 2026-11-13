// Delay-loaded by the loader test: nothing maps it until the program first calls it.
__declspec(dllexport) int square(int x) { return x * x; }
int __stdcall DllMain(void *m, unsigned r, void *x) { (void)m; (void)r; (void)x; return 1; }
