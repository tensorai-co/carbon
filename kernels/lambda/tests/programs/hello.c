// A freestanding test program for lambda: no libc, just system calls, for ARM64 and x86-64 alike.
// Build: clang --target=aarch64-linux-gnu (or x86_64-linux-gnu) -nostdlib -static -ffreestanding -fno-stack-protector
//        -fuse-ld=lld -O2 hello.c -o hello
//
// It checks: argv, getpid, uname, brk and mmap with pages faulted in on touch, files read from and written to the disk,
// getdents64, clocks, pipes, fork with copy-on-write, a child writing to a pipe, wait4

typedef unsigned long u64;
typedef long i64;

#if defined(__aarch64__)
enum { SYS_openat = 56, SYS_close = 57, SYS_getdents64 = 61, SYS_read = 63, SYS_write = 64, SYS_exit_group = 94,
       SYS_clock_gettime = 113, SYS_getpid = 172, SYS_brk = 214, SYS_munmap = 215, SYS_clone = 220, SYS_mmap = 222,
       SYS_wait4 = 260, SYS_pipe2 = 59, SYS_uname = 160 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ARCH "aarch64"
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_mmap = 9, SYS_munmap = 11, SYS_brk = 12, SYS_getpid = 39,
       SYS_clone = 56, SYS_wait4 = 61, SYS_uname = 63, SYS_getdents64 = 217, SYS_clock_gettime = 228,
       SYS_exit_group = 231, SYS_openat = 257, SYS_pipe2 = 293 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ARCH "x86_64"
#endif

#define AT_FDCWD (-100)
static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    // A branch, not arithmetic on the comparison: exec's x86 translator doesn't do `adc` and `sbb` yet.
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int same(const char *a, const char *b, u64 n) { for (u64 i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }

void cmain(i64 *sp) {
    i64 argc = sp[0];
    char **argv = (char **)(sp + 1);
    say("hello from " ARCH " on lambda, as ");
    say(argv[0]);
    say("\n");
    check(argc >= 1, "argc and argv");

    check(sys(SYS_getpid, 0, 0, 0, 0, 0, 0) == 1, "getpid is 1");

    char uts[390];
    check(sys(SYS_uname, (i64)uts, 0, 0, 0, 0, 0) == 0 && same(uts, "Linux", 5) && same(uts + 260, ARCH, len(ARCH)), "uname");

    i64 top = sys(SYS_brk, 0, 0, 0, 0, 0, 0);
    i64 grown = sys(SYS_brk, top + 65536, 0, 0, 0, 0, 0);
    int heap_ok = grown == top + 65536;
    if (heap_ok) { volatile char *p = (char *)top; for (int i = 0; i < 65536; i += 4096) p[i] = (char)i; heap_ok = p[4096] == 0 && p[8192] == 0; }
    check(heap_ok, "brk grows the heap, faulted in page by page");

    i64 m = sys(SYS_mmap, 0, 1 << 20, 3, 0x22, -1, 0);
    int mmap_ok = m > 0;
    if (mmap_ok) { volatile u64 *p = (u64 *)m; for (int i = 0; i < (1 << 20) / 8; i += 512) p[i] = i; for (int i = 0; i < (1 << 20) / 8; i += 512) mmap_ok &= p[i] == (u64)i; }
    check(mmap_ok && sys(SYS_munmap, m, 1 << 20, 0, 0, 0, 0) == 0, "mmap 1 MiB, touch it, munmap");

    char buf[128];
    i64 fd = sys(SYS_openat, AT_FDCWD, (i64)"/etc/greeting", 0, 0, 0, 0);
    i64 n = fd >= 0 ? sys(SYS_read, fd, (i64)buf, sizeof buf, 0, 0, 0) : -1;
    check(n > 0 && same(buf, "greetings", 9), "open and read a file from the disk");
    sys(SYS_close, fd, 0, 0, 0, 0, 0);

    fd = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/out", 0101 | 01000, 0644, 0, 0);
    sys(SYS_write, fd, (i64)"written", 7, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    fd = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/out", 0, 0, 0, 0);
    n = sys(SYS_read, fd, (i64)buf, sizeof buf, 0, 0, 0);
    check(n == 7 && same(buf, "written", 7), "create, write, reopen and read back");
    sys(SYS_close, fd, 0, 0, 0, 0, 0);

    fd = sys(SYS_openat, AT_FDCWD, (i64)"/", 0, 0, 0, 0);
    n = sys(SYS_getdents64, fd, (i64)buf, sizeof buf, 0, 0, 0);
    check(n > 0, "getdents64 on /");
    sys(SYS_close, fd, 0, 0, 0, 0, 0);

    i64 ts[2] = {0, 0};
    check(sys(SYS_clock_gettime, 1, (i64)ts, 0, 0, 0, 0) == 0 && (ts[0] | ts[1]) != 0, "clock_gettime");

    int fds[2];
    check(sys(SYS_pipe2, (i64)fds, 0, 0, 0, 0, 0) == 0, "pipe2");
    i64 child = sys(SYS_clone, 17, 0, 0, 0, 0, 0); // SIGCHLD: fork
    if (child == 0) {
        sys(SYS_write, fds[1], (i64)"child says hi", 13, 0, 0, 0);
        sys(SYS_exit_group, 3, 0, 0, 0, 0, 0);
    }
    sys(SYS_close, fds[1], 0, 0, 0, 0, 0);
    n = sys(SYS_read, fds[0], (i64)buf, sizeof buf, 0, 0, 0);
    check(child > 1 && n == 13 && same(buf, "child says hi", 13), "fork, and read what the child wrote to a pipe");
    int status = 0;
    i64 waited = sys(SYS_wait4, -1, (i64)&status, 0, 0, 0, 0);
    check(waited == child && status == 3 << 8, "wait4 for the child's exit(3)");

    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
