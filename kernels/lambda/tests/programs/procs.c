// Processes and scheduling, for lambda: resource limits (RLIMIT_NOFILE enforced on the descriptor table, inherited
// by a child, EINVAL and EPERM); nice values; scheduling policies and parameters, SCHED_RESET_ON_FORK, sched_setattr's
// sizes and SCHED_DEADLINE; CPU affinity and getcpu within it; personality and PER_LINUX32's uname; CPU time from
// times, getrusage, wait4's rusage and the CPU-time clocks; waitid's exits, stops, continues, WNOHANG and WNOWAIT;
// robust futexes released when their owner thread ends; kcmp; execveat by descriptor and by directory,
// AT_EXECVE_CHECK and AT_SYMLINK_NOFOLLOW; restart_syscall. Freestanding, for ARM64 and x86-64 alike. Built like
// hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_dup = 23, SYS_dup3 = 24, SYS_fcntl = 25, SYS_unlinkat = 35, SYS_symlinkat = 36, SYS_openat = 56,
       SYS_close = 57, SYS_write = 64, SYS_exit_group = 94, SYS_waitid = 95, SYS_futex = 98, SYS_set_robust_list = 99,
       SYS_get_robust_list = 100, SYS_clock_gettime = 113, SYS_sched_setparam = 118, SYS_sched_setscheduler = 119,
       SYS_sched_getscheduler = 120, SYS_sched_getparam = 121, SYS_sched_setaffinity = 122,
       SYS_sched_getaffinity = 123, SYS_sched_yield = 124, SYS_sched_get_priority_max = 125,
       SYS_sched_get_priority_min = 126, SYS_sched_rr_get_interval = 127, SYS_restart_syscall = 128, SYS_kill = 129,
       SYS_setpriority = 140, SYS_getpriority = 141, SYS_times = 153, SYS_uname = 160, SYS_getrlimit = 163,
       SYS_setrlimit = 164, SYS_getrusage = 165, SYS_getcpu = 168, SYS_getpid = 172, SYS_gettid = 178,
       SYS_clone = 220, SYS_wait4 = 260, SYS_prlimit64 = 261, SYS_kcmp = 272, SYS_sched_setattr = 274,
       SYS_sched_getattr = 275, SYS_execveat = 281, SYS_personality = 92 };
#define O_DIRECTORY 040000
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define MACHINE32 "armv8l"
#else
enum { SYS_write = 1, SYS_close = 3, SYS_sched_yield = 24, SYS_dup = 32, SYS_getpid = 39, SYS_clone = 56,
       SYS_wait4 = 61, SYS_kill = 62, SYS_uname = 63, SYS_fcntl = 72, SYS_getrlimit = 97, SYS_getrusage = 98,
       SYS_times = 100, SYS_personality = 135, SYS_getpriority = 140, SYS_setpriority = 141,
       SYS_sched_setparam = 142, SYS_sched_getparam = 143, SYS_sched_setscheduler = 144,
       SYS_sched_getscheduler = 145, SYS_sched_get_priority_max = 146, SYS_sched_get_priority_min = 147,
       SYS_sched_rr_get_interval = 148, SYS_setrlimit = 160, SYS_gettid = 186, SYS_futex = 202,
       SYS_sched_setaffinity = 203, SYS_sched_getaffinity = 204, SYS_restart_syscall = 219, SYS_clock_gettime = 228,
       SYS_exit_group = 231, SYS_waitid = 247, SYS_openat = 257, SYS_unlinkat = 263, SYS_symlinkat = 266,
       SYS_set_robust_list = 273, SYS_get_robust_list = 274, SYS_dup3 = 292, SYS_prlimit64 = 302, SYS_getcpu = 309,
       SYS_kcmp = 312, SYS_sched_setattr = 314, SYS_sched_getattr = 315, SYS_execveat = 322 };
#define O_DIRECTORY 0200000
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define MACHINE32 "i686"
#endif

#define AT_FDCWD (-100)
#define O_RDONLY 0
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define EPERM 1
#define ENOENT 2
#define ESRCH 3
#define EINTR 4
#define ENOEXEC 8
#define EBADF 9
#define ECHILD 10
#define E2BIG 7
#define EINVAL 22
#define EMFILE 24
#define ELOOP 40
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SIGKILL 9

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int same(const char *a, const char *b, u64 n) { for (u64 i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static i64 waitpid(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return status; }
static i64 now(int clock) { i64 ts[2]; return sys(SYS_clock_gettime, clock, (i64)ts, 0, 0, 0, 0) ? -1 : ts[0] * 1000000000 + ts[1]; }
// Keeps the CPU busy for `ms` of real time.
static void burn(int ms) {
    volatile u64 x = 1;
    i64 until = now(1) + (i64)ms * 1000000;
    while (now(1) < until) for (int i = 0; i < 10000; i++) x = x * 6364136223846793005UL + 1;
}

// Threads, as threads.c starts them, for the robust futex check.
#define CLONE_VM 0x100
#define CLONE_FS 0x200
#define CLONE_FILES 0x400
#define CLONE_SIGHAND 0x800
#define CLONE_THREAD 0x10000
#define CLONE_SYSVSEM 0x40000
#define CLONE_SETTLS 0x80000
#define CLONE_PARENT_SETTID 0x100000
#define CLONE_CHILD_CLEARTID 0x200000
static i64 spawn(void (*fn)(void *), void *arg, char *top, void *tls, volatile u32 *tid) {
    u64 flags = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM | CLONE_SETTLS |
                CLONE_PARENT_SETTID | CLONE_CHILD_CLEARTID;
    u64 *sp = (u64 *)top;
    *--sp = (u64)arg;
    *--sp = (u64)fn;
#if defined(__aarch64__)
    register i64 x8 __asm__("x8") = SYS_clone, x0 __asm__("x0") = flags, x1 __asm__("x1") = (i64)sp,
                 x2 __asm__("x2") = (i64)tid, x3 __asm__("x3") = (i64)tls, x4 __asm__("x4") = (i64)tid;
    __asm__ volatile("svc #0\n cbnz x0, 1f\n ldp x9, x0, [sp], #16\n blr x9\n mov x0, #0\n mov x8, #93\n svc #0\n 1:"
                     : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4) : "x9", "x30", "memory");
    return x0;
#else
    register i64 r10 __asm__("r10") = (i64)tid, r8 __asm__("r8") = (i64)tls;
    i64 ret;
    __asm__ volatile("syscall\n test %%rax, %%rax\n jnz 1f\n pop %%rax\n pop %%rdi\n call *%%rax\n"
                     " xor %%edi, %%edi\n mov $60, %%eax\n syscall\n hlt\n 1:"
                     : "=a"(ret) : "a"(SYS_clone), "D"(flags), "S"(sp), "d"(tid), "r"(r10), "r"(r8) : "rcx", "r11", "memory");
    return ret;
#endif
}
static void join(volatile u32 *tid) {
    for (;;) {
        u32 at = __atomic_load_n(tid, __ATOMIC_ACQUIRE);
        if (at == 0) return;
        sys(SYS_futex, (i64)tid, 0, at, 0, 0, 0);
    }
}

// A robust list of two entries: a futex the thread holds (with waiters), and one another thread holds.
struct node { u64 next; u32 futex; u32 pad; };
static struct node held, other;
static u64 head[3];
static char stack[65536] __attribute__((aligned(16)));
static void *tls[4] __attribute__((aligned(16)));
static void dies_holding(void *arg) {
    (void)arg;
    u32 tid = sys(SYS_gettid, 0, 0, 0, 0, 0, 0);
    held.futex = tid | 0x80000000u;
    other.futex = 12345;
    head[0] = (u64)&held;
    held.next = (u64)&other;
    other.next = (u64)head;
    head[1] = 8;
    head[2] = 0;
    sys(SYS_set_robust_list, (i64)head, 24, 0, 0, 0, 0);
}

void cmain(i64 *sp) {
    i64 argc = sp[0];
    char **argv = (char **)(sp + 1);
    // Run again by execveat: say so by the exit code.
    if (argc >= 2 && argv[1][0] == 'c') quit(42);
    u64 lim[2];
    int pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    // Resource limits.
    check(sys(SYS_getrlimit, 7, (i64)lim, 0, 0, 0, 0) == 0 && lim[0] == 1024 && lim[1] == 524288, "getrlimit: RLIMIT_NOFILE is 1024, 524288");
    u64 eight[2] = {8, 524288};
    check(sys(SYS_setrlimit, 7, (i64)eight, 0, 0, 0, 0) == 0, "setrlimit: RLIMIT_NOFILE down to 8");
    int opened[16], n = 0, last;
    while (n < 16 && (last = sys(SYS_openat, AT_FDCWD, (i64)"/dev/null", O_RDONLY, 0, 0, 0)) >= 0) opened[n++] = last;
    int highest = 0;
    for (int i = 0; i < n; i++) if (opened[i] > highest) highest = opened[i];
    check(last == -EMFILE && highest == 7, "with RLIMIT_NOFILE 8, descriptors stop at 7: EMFILE");
    check(sys(SYS_dup3, 0, 9, 0, 0, 0, 0) == -EBADF && sys(SYS_fcntl, 0, 0 /* F_DUPFD */, 9, 0, 0, 0) == -EINVAL, "dup3 and F_DUPFD past it: EBADF, EINVAL");
    for (int i = 0; i < n; i++) sys(SYS_close, opened[i], 0, 0, 0, 0, 0);
    u64 back[2] = {1024, 524288}, upside_down[2] = {10, 5}, huge[2] = {1024, 1UL << 30};
    sys(SYS_setrlimit, 7, (i64)back, 0, 0, 0, 0);
    check(sys(SYS_setrlimit, 7, (i64)upside_down, 0, 0, 0, 0) == -EINVAL && sys(SYS_setrlimit, 7, (i64)huge, 0, 0, 0, 0) == -EPERM &&
          sys(SYS_getrlimit, 99, (i64)lim, 0, 0, 0, 0) == -EINVAL, "setrlimit: soft over hard EINVAL, past nr_open EPERM; a bad resource EINVAL");
    u64 core[2] = {4096, ~0UL}, old[2];
    check(sys(SYS_prlimit64, pid, 4, (i64)core, (i64)old, 0, 0) == 0 && old[0] == 0 && sys(SYS_prlimit64, 0, 4, 0, (i64)lim, 0, 0) == 0 && lim[0] == 4096,
          "prlimit64: set RLIMIT_CORE by pid, the old one back, then read");
    i64 child = fork();
    if (child == 0) { sys(SYS_getrlimit, 4, (i64)lim, 0, 0, 0, 0); quit(lim[0] == 4096 ? 0 : 1); }
    check(waitpid(child) == 0, "a child inherits its limits");
    check(sys(SYS_prlimit64, 999999, 4, 0, (i64)lim, 0, 0) == -ESRCH, "prlimit64 on no such process: ESRCH");

    // Nice.
    check(sys(SYS_getpriority, 0, 0, 0, 0, 0, 0) == 20, "getpriority: nice 0, as 20");
    check(sys(SYS_setpriority, 0, 0, 5, 0, 0, 0) == 0 && sys(SYS_getpriority, 0, pid, 0, 0, 0, 0) == 15, "setpriority 5, read back as 15");
    child = fork();
    if (child == 0) quit(sys(SYS_getpriority, 0, 0, 0, 0, 0, 0));
    check(waitpid(child) >> 8 == 15, "a child inherits its nice");
    check(sys(SYS_setpriority, 0, 0, -30, 0, 0, 0) == 0 && sys(SYS_getpriority, 0, 0, 0, 0, 0, 0) == 40, "root may lower nice, clamped to -20");
    sys(SYS_setpriority, 0, 0, 0, 0, 0, 0);
    check(sys(SYS_getpriority, 0, 999999, 0, 0, 0, 0) == -ESRCH && sys(SYS_getpriority, 7, 0, 0, 0, 0, 0) == -EINVAL, "getpriority: no such process, a bad which");

    // Scheduling policies.
    int param = 10;
    check(sys(SYS_sched_getscheduler, 0, 0, 0, 0, 0, 0) == 0, "sched_getscheduler: SCHED_OTHER");
    check(sys(SYS_sched_setscheduler, 0, 1, (i64)&param, 0, 0, 0) == 0 && sys(SYS_sched_getscheduler, 0, 0, 0, 0, 0, 0) == 1 &&
          (param = 0, sys(SYS_sched_getparam, 0, (i64)&param, 0, 0, 0, 0)) == 0 && param == 10, "sched_setscheduler: SCHED_FIFO at 10, read back");
    param = 0;
    check(sys(SYS_sched_setscheduler, 0, 1, (i64)&param, 0, 0, 0) == -EINVAL && sys(SYS_sched_setscheduler, 0, 42, (i64)&param, 0, 0, 0) == -EINVAL,
          "sched_setscheduler: priority 0 for FIFO, or no such policy, EINVAL");
    param = 20;
    check(sys(SYS_sched_setparam, 0, (i64)&param, 0, 0, 0, 0) == 0 && sys(SYS_sched_getparam, 0, (i64)&param, 0, 0, 0, 0) == 0 && param == 20, "sched_setparam");
    check(sys(SYS_sched_get_priority_max, 1, 0, 0, 0, 0, 0) == 99 && sys(SYS_sched_get_priority_min, 2, 0, 0, 0, 0, 0) == 1 &&
          sys(SYS_sched_get_priority_max, 0, 0, 0, 0, 0, 0) == 0 && sys(SYS_sched_get_priority_max, 42, 0, 0, 0, 0, 0) == -EINVAL,
          "sched_get_priority_max and _min");
    param = 5;
    sys(SYS_sched_setscheduler, 0, 2 | 0x40000000, (i64)&param, 0, 0, 0);
    i64 ts[2] = {-1, -1};
    check(sys(SYS_sched_getscheduler, 0, 0, 0, 0, 0, 0) == (2 | 0x40000000) && sys(SYS_sched_rr_get_interval, 0, (i64)ts, 0, 0, 0, 0) == 0 &&
          ts[0] == 0 && ts[1] == 100000000, "SCHED_RR with SCHED_RESET_ON_FORK, a 100 ms slice");
    child = fork();
    if (child == 0) quit(sys(SYS_sched_getscheduler, 0, 0, 0, 0, 0, 0));
    check(waitpid(child) == 0, "SCHED_RESET_ON_FORK: the child has SCHED_OTHER");
    struct { u32 size, policy; u64 flags; int nice; u32 priority; u64 runtime, deadline, period; u32 util_min, util_max; } attr = {56, 0, 0, 3, 0, 0, 0, 0, 0, 1024};
    check(sys(SYS_sched_setattr, 0, (i64)&attr, 0, 0, 0, 0) == 0 && sys(SYS_sched_getscheduler, 0, 0, 0, 0, 0, 0) == 0 &&
          sys(SYS_getpriority, 0, 0, 0, 0, 0, 0) == 17, "sched_setattr: SCHED_OTHER at nice 3");
    attr.size = 8;
    check(sys(SYS_sched_setattr, 0, (i64)&attr, 0, 0, 0, 0) == -E2BIG && attr.size == 56, "sched_setattr too small: E2BIG, with the size it knows");
    attr = (typeof(attr)){56, 6, 0, 0, 0, 10000000, 30000000, 100000000, 0, 1024};
    check(sys(SYS_sched_setattr, 0, (i64)&attr, 0, 0, 0, 0) == 0, "sched_setattr: SCHED_DEADLINE");
    attr = (typeof(attr)){0};
    check(sys(SYS_sched_getattr, 0, (i64)&attr, 56, 0, 0, 0) == 0 && attr.size == 56 && attr.policy == 6 && attr.runtime == 10000000 &&
          attr.deadline == 30000000 && attr.period == 100000000, "sched_getattr: SCHED_DEADLINE's parameters");
    attr = (typeof(attr)){56, 6, 0, 0, 0, 50000000, 30000000, 100000000, 0, 1024};
    check(sys(SYS_sched_setattr, 0, (i64)&attr, 0, 0, 0, 0) == -EINVAL && sys(SYS_sched_getattr, 0, (i64)&attr, 8, 0, 0, 0) == -EINVAL,
          "SCHED_DEADLINE with more runtime than deadline, sched_getattr too small: EINVAL");
    param = 0;
    sys(SYS_sched_setscheduler, 0, 0, (i64)&param, 0, 0, 0);
    sys(SYS_setpriority, 0, 0, 0, 0, 0, 0);

    // CPU affinity.
    u64 mask[16] = {0};
    check(sys(SYS_sched_getaffinity, 0, sizeof mask, (i64)mask, 0, 0, 0) == 8 && mask[0] != 0, "sched_getaffinity: 8 bytes, some CPUs");
    u64 all = mask[0], one = 1;
    unsigned cpu = 99, node = 99;
    check(sys(SYS_sched_setaffinity, 0, 8, (i64)&one, 0, 0, 0) == 0 && sys(SYS_sched_getaffinity, 0, 8, (i64)mask, 0, 0, 0) == 8 && mask[0] == 1 &&
          sys(SYS_getcpu, (i64)&cpu, (i64)&node, 0, 0, 0, 0) == 0 && cpu == 0 && node == 0, "sched_setaffinity to CPU 0, and getcpu there");
    u64 none = 0;
    check(sys(SYS_sched_setaffinity, 0, 8, (i64)&none, 0, 0, 0) == -EINVAL && sys(SYS_sched_getaffinity, 0, 4, (i64)mask, 0, 0, 0) == -EINVAL,
          "no CPUs at all, or a buffer that's no whole long: EINVAL");
    sys(SYS_sched_setaffinity, 0, 8, (i64)&all, 0, 0, 0);

    // Personality.
    char uts[390];
    check(sys(SYS_personality, 0xffffffff, 0, 0, 0, 0, 0) == 0 && sys(SYS_personality, 8, 0, 0, 0, 0, 0) == 0 && sys(SYS_uname, (i64)uts, 0, 0, 0, 0, 0) == 0 &&
          same(uts + 65 * 4, MACHINE32, len(MACHINE32) + 1), "personality(PER_LINUX32): uname names the 32-bit machine");
    check(sys(SYS_personality, 0, 0, 0, 0, 0, 0) == 8 && sys(SYS_uname, (i64)uts, 0, 0, 0, 0, 0) == 0 && !same(uts + 65 * 4, MACHINE32, len(MACHINE32)),
          "personality back to PER_LINUX");

    // CPU time.
    burn(300);
    i64 tms[4] = {-1, -1, -1, -1};
    i64 ticks = sys(SYS_times, (i64)tms, 0, 0, 0, 0, 0);
    check(ticks > 0 && tms[0] + tms[1] >= 15, "times: 300 ms of work counted (at least 150 ms)");
    i64 cpu_process = now(2), cpu_thread = now(3);
    check(cpu_process >= 150000000 && cpu_thread >= 150000000 && cpu_thread <= cpu_process + 1000000, "the CPU-time clocks: this process's and this thread's");
    check(now((~pid << 3) | 2) >= cpu_process && now((~(int)sys(SYS_gettid, 0, 0, 0, 0, 0, 0) << 3) | 6) >= cpu_thread,
          "clock_getcpuclockid's clocks for this process and thread");
    i64 tai = now(11), real = now(0);
    check(now(10) == -1 && tai - real > 36000000000L && tai - real < 38000000000L, "clock 10 is EINVAL; TAI is 37 s ahead");
    i64 ru[18];
    check(sys(SYS_getrusage, 0, (i64)ru, 0, 0, 0, 0) == 0 && ru[0] * 1000000 + ru[1] + ru[2] * 1000000 + ru[3] >= 150000 &&
          sys(SYS_getrusage, 1, (i64)ru, 0, 0, 0, 0) == 0 && sys(SYS_getrusage, 5, (i64)ru, 0, 0, 0, 0) == -EINVAL, "getrusage: self, this thread, a bad who");
    child = fork();
    if (child == 0) { burn(200); quit(0); }
    for (int i = 0; i < 18; i++) ru[i] = -1;
    int status;
    sys(SYS_wait4, child, (i64)&status, 0, (i64)ru, 0, 0);
    check(ru[0] * 1000000 + ru[1] + ru[2] * 1000000 + ru[3] >= 100000, "wait4's rusage: the child's 200 ms");
    sys(SYS_times, (i64)tms, 0, 0, 0, 0, 0);
    // RUSAGE_CHILDREN as an int, the way a C library passes it: 32 bits of -1.
    check(tms[2] + tms[3] >= 10 && sys(SYS_getrusage, 0xffffffffL, (i64)ru, 0, 0, 0, 0) == 0 && ru[0] * 1000000 + ru[1] + ru[2] * 1000000 + ru[3] >= 100000,
          "times and getrusage(RUSAGE_CHILDREN): the waited-for child's time");

    // waitid: an exit, looked at with WNOWAIT and then reaped.
    int si[32];
    child = fork();
    if (child == 0) quit(7);
    check(sys(SYS_waitid, 1, child, (i64)si, 4 | 0x01000000, 0, 0) == 0 && si[0] == SIGCHLD && si[2] == 1 && si[4] == child && si[6] == 7,
          "waitid with WNOWAIT: CLD_EXITED, status 7, left to wait for");
    check(sys(SYS_waitid, 1, child, (i64)si, 4, 0, 0) == 0 && si[4] == child && si[6] == 7 && sys(SYS_waitid, 1, child, (i64)si, 4, 0, 0) == -ECHILD,
          "waitid again reaps it; then ECHILD");
    // A stop, a continue and a kill.
    child = fork();
    if (child == 0) { sys(SYS_kill, sys(SYS_getpid, 0, 0, 0, 0, 0, 0), SIGSTOP, 0, 0, 0, 0); for (;;) sys(SYS_sched_yield, 0, 0, 0, 0, 0, 0); }
    check(sys(SYS_waitid, 1, child, (i64)si, 2, 0, 0) == 0 && si[2] == 5 && si[6] == SIGSTOP, "waitid WSTOPPED: CLD_STOPPED by SIGSTOP");
    check(sys(SYS_kcmp, pid, child, 1, 0, 0, 0) != 0 && sys(SYS_kcmp, pid, child, 2, 0, 0, 0) != 0 && sys(SYS_kcmp, pid, pid, 1, 0, 0, 0) == 0,
          "kcmp: a child's memory and file table aren't ours; ours are");
    sys(SYS_kill, child, SIGCONT, 0, 0, 0, 0);
    check(sys(SYS_waitid, 1, child, (i64)si, 8, 0, 0) == 0 && si[2] == 6 && si[6] == SIGCONT, "waitid WCONTINUED: CLD_CONTINUED");
    for (int i = 0; i < 8; i++) si[i] = -1;
    check(sys(SYS_waitid, 1, child, (i64)si, 4 | 1, 0, 0) == 0 && si[0] == 0 && si[4] == 0, "waitid WNOHANG with a child running: 0, and si_pid 0");
    sys(SYS_kill, child, SIGKILL, 0, 0, 0, 0);
    check(sys(SYS_waitid, 2, 0, (i64)si, 4, 0, 0) == 0 && si[2] == 2 && si[6] == SIGKILL && si[4] == child, "waitid P_PGID: CLD_KILLED by SIGKILL");
    check(sys(SYS_waitid, 0, 0, (i64)si, 0, 0, 0) == -EINVAL && sys(SYS_waitid, 9, 0, (i64)si, 4, 0, 0) == -EINVAL, "waitid: no event asked for, or a bad idtype");

    // Robust futexes: a thread that dies holding one gives it up.
    volatile u32 tid = 0;
    tls[0] = &tls[0];
    spawn(dies_holding, 0, stack + sizeof stack, &tls[0], &tid);
    join(&tid);
    check(held.futex == 0xC0000000u && other.futex == 12345, "a thread ended holding a robust futex: FUTEX_OWNER_DIED, waiters kept; another's untouched");
    u64 mine[3] = {0, 0, 0}, got_head = 0, got_len = 0;
    mine[0] = (u64)mine;
    check(sys(SYS_set_robust_list, (i64)mine, 24, 0, 0, 0, 0) == 0 && sys(SYS_get_robust_list, 0, (i64)&got_head, (i64)&got_len, 0, 0, 0) == 0 &&
          got_head == (u64)mine && got_len == 24, "set_robust_list and get_robust_list");
    check(sys(SYS_set_robust_list, (i64)mine, 10, 0, 0, 0, 0) == -EINVAL && sys(SYS_get_robust_list, 999999, (i64)&got_head, (i64)&got_len, 0, 0, 0) == -ESRCH,
          "robust lists: a bad size, no such thread");

    // kcmp on files.
    int fd = sys(SYS_openat, AT_FDCWD, (i64)"/dev/null", O_RDONLY, 0, 0, 0), copy = sys(SYS_dup, fd, 0, 0, 0, 0, 0);
    int another = sys(SYS_openat, AT_FDCWD, (i64)"/dev/null", O_RDONLY, 0, 0, 0);
    i64 diff = sys(SYS_kcmp, pid, pid, 0, another, fd, 0);
    check(sys(SYS_kcmp, pid, pid, 0, fd, copy, 0) == 0 && (diff == 1 || diff == 2), "kcmp KCMP_FILE: a dup is the same file; another open isn't");
    check(sys(SYS_kcmp, pid, pid, 0, fd, 555, 0) == -EBADF && sys(SYS_kcmp, pid, pid, 99, 0, 0, 0) == -EINVAL && sys(SYS_kcmp, pid, 999999, 1, 0, 0, 0) == -ESRCH,
          "kcmp: a bad descriptor, a bad type, no such process");

    // execveat.
    char *cargv[] = {"procs", "child", 0}, *cenv[] = {0};
    int self = sys(SYS_openat, AT_FDCWD, (i64)argv[0], O_RDONLY, 0, 0, 0);
    child = fork();
    if (child == 0) { sys(SYS_execveat, self, (i64)"", (i64)cargv, (i64)cenv, 0x1000, 0); quit(99); }
    check(waitpid(child) >> 8 == 42, "execveat with AT_EMPTY_PATH: runs the file a descriptor is open on");
    int bin = sys(SYS_openat, AT_FDCWD, (i64)"/bin", O_RDONLY | O_DIRECTORY, 0, 0, 0);
    const char *name = argv[0];
    for (const char *c = argv[0]; *c; c++) if (*c == '/') name = c + 1;
    child = fork();
    if (child == 0) { sys(SYS_execveat, bin, (i64)name, (i64)cargv, (i64)cenv, 0, 0); quit(99); }
    check(waitpid(child) >> 8 == 42, "execveat from a directory");
    int text = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/text", O_RDWR | O_CREAT | O_TRUNC, 0755, 0, 0);
    sys(SYS_write, text, (i64)"just words\n", 11, 0, 0, 0);
    check(sys(SYS_execveat, AT_FDCWD, (i64)argv[0], 0, 0, 0x10000, 0) == 0 && sys(SYS_execveat, AT_FDCWD, (i64)"/tmp/text", 0, 0, 0x10000, 0) == -ENOEXEC,
          "AT_EXECVE_CHECK: a program can run, words can't");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/plink", 0, 0, 0, 0);
    sys(SYS_symlinkat, (i64)argv[0], AT_FDCWD, (i64)"/tmp/plink", 0, 0, 0);
    check(sys(SYS_execveat, AT_FDCWD, (i64)"/tmp/plink", (i64)cargv, (i64)cenv, 0x100, 0) == -ELOOP &&
          sys(SYS_execveat, AT_FDCWD, (i64)argv[0], (i64)cargv, (i64)cenv, 0x2, 0) == -EINVAL, "execveat: AT_SYMLINK_NOFOLLOW on a link ELOOP, a bad flag EINVAL");

    check(sys(SYS_restart_syscall, 0, 0, 0, 0, 0, 0) == -EINTR, "restart_syscall with nothing to restart: EINTR");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
