// Delay-loaded by the loader test: nothing maps it until the program first calls it. Its thread-local counter is
// static TLS each thread gets a copy of, the threads already running too, and its TLS callback reads it as each
// thread ends (where a thread with no copy would fault).
typedef __UINTPTR_TYPE__ ULONG_PTR;
typedef unsigned int DWORD;

#if defined(__i386__)
__asm__(".globl __tls_array\n.set __tls_array, 0x2c");
#endif

// Static TLS, as the C runtime's tlssup sets it up (see loader.c).
typedef void(__stdcall *TLS_CALLBACK)(void *, DWORD, void *);
unsigned _tls_index;
__attribute__((section(".tls"), used)) char _tls_start = 0;
__attribute__((section(".tls$ZZZ"), used)) char _tls_end = 0;
__declspec(thread) int counted = 40;
static volatile int detached_seen;
static void __stdcall on_tls(void *m, DWORD reason, void *r) {
    (void)m;
    (void)r;
    if (reason == 3) detached_seen += counted;  // DLL_THREAD_DETACH
}
__attribute__((section(".CRT$XLA"), used)) TLS_CALLBACK __xl_a = 0;
__attribute__((section(".CRT$XLB"), used)) TLS_CALLBACK __xl_b = on_tls;
__attribute__((section(".CRT$XLZ"), used)) TLS_CALLBACK __xl_z = 0;
struct { ULONG_PTR start, end, index, callbacks; DWORD zero_fill, flags; } const _tls_used = {
    (ULONG_PTR)&_tls_start, (ULONG_PTR)&_tls_end, (ULONG_PTR)&_tls_index, (ULONG_PTR)(&__xl_a + 1), 0, 0};

__declspec(dllexport) int square(int x) { return x * x; }
__declspec(dllexport) int bump(void) { return ++counted; }
__declspec(dllexport) int tls_detached(void) { return detached_seen; }
int __stdcall DllMain(void *m, unsigned r, void *x) { (void)m; (void)r; (void)x; return 1; }
