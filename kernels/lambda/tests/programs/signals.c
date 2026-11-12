// Signals, job control and ptys, for lambda: freestanding, for ARM64 and x86-64 alike. Built like hello.c.
//
// It checks: a handler with siginfo and rt_sigreturn, blocked and pending signals, a SIGSEGV handler that moves the
// program on, SIGTERM to a child, SIGSTOP/SIGCONT/SIGKILL as wait4 reports them, SA_RESTART and EINTR, SIGPIPE and
// EPIPE, a pty's line editing and echo, ^C on a pty

typedef unsigned long u64;
typedef long i64;

#if defined(__aarch64__)
enum { SYS_ioctl = 29, SYS_openat = 56, SYS_close = 57, SYS_pipe2 = 59, SYS_read = 63, SYS_write = 64, SYS_exit_group = 94,
       SYS_nanosleep = 101, SYS_kill = 129, SYS_rt_sigaction = 134, SYS_rt_sigprocmask = 135, SYS_rt_sigpending = 136,
       SYS_setsid = 157, SYS_getpid = 172, SYS_getppid = 173, SYS_clone = 220, SYS_wait4 = 260 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
// Where the program counter is in the ucontext a handler gets, and how long the faulting load is.
#define UC_PC 440
#define LOAD_LEN 4
static int load(volatile int *p) { int v; __asm__ volatile("ldr %w0, [%1]" : "=r"(v) : "r"(p) : "memory"); return v; }
#define RESTORER 0
#define SA_RESTORER_FLAG 0
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_rt_sigaction = 13, SYS_rt_sigprocmask = 14, SYS_ioctl = 16,
       SYS_nanosleep = 35, SYS_getpid = 39, SYS_clone = 56, SYS_wait4 = 61, SYS_kill = 62, SYS_getppid = 110,
       SYS_setsid = 112, SYS_rt_sigpending = 127, SYS_exit_group = 231, SYS_openat = 257, SYS_pipe2 = 293 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
__asm__(".globl restorer\nrestorer:\n mov $15, %eax\n syscall\n");
void restorer(void);
#define UC_PC 168
#define LOAD_LEN 2
static int load(volatile int *p) { int v; __asm__ volatile(".byte 0x8b, 0x07" : "=a"(v) : "D"(p) : "memory"); return v; }
#define RESTORER ((u64)restorer)
#define SA_RESTORER_FLAG 0x04000000
#endif

#define AT_FDCWD (-100)
#define SIGINT 2
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGPIPE 13
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SA_SIGINFO 4
#define SA_RESTART 0x10000000

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
static void sleep_ms(int ms) { i64 ts[2] = {0, ms * 1000000L}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static int wait_for(i64 pid, int options) { int status = -1; sys(SYS_wait4, pid, (i64)&status, options, 0, 0, 0); return status; }

static void handle(int sig, void (*handler)(int, int *, char *), u64 flags) {
    u64 action[4] = {(u64)handler, flags | SA_SIGINFO | SA_RESTORER_FLAG, RESTORER, 0};
    sys(SYS_rt_sigaction, sig, (i64)action, 0, 8, 0, 0);
}
static void ignore(int sig) { u64 action[4] = {1, SA_RESTORER_FLAG, RESTORER, 0}; sys(SYS_rt_sigaction, sig, (i64)action, 0, 8, 0, 0); }

static volatile int got, got_signo;
static void on_usr1(int sig, int *info, char *uc) { got = got + 1; got_signo = info[0]; (void)sig; (void)uc; }

static volatile u64 fault_addr;
static void on_segv(int sig, int *info, char *uc) {
    fault_addr = *(u64 *)(info + 4);
    *(u64 *)(uc + UC_PC) += LOAD_LEN; // skip the load that faulted
    (void)sig;
}

void cmain(i64 *sp) {
    (void)sp;
#if defined(__aarch64__)
    say("signals on lambda, aarch64\n");
#else
    say("signals on lambda, x86_64\n");
#endif
    i64 me = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    handle(SIGUSR1, on_usr1, 0);
    sys(SYS_kill, me, SIGUSR1, 0, 0, 0, 0);
    check(got == 1 && got_signo == SIGUSR1, "a handler runs, with its siginfo, and returns through rt_sigreturn");

    u64 block = 1UL << (SIGUSR1 - 1), pending = 0;
    sys(SYS_rt_sigprocmask, 0, (i64)&block, 0, 8, 0, 0);
    sys(SYS_kill, me, SIGUSR1, 0, 0, 0, 0);
    sys(SYS_rt_sigpending, (i64)&pending, 8, 0, 0, 0, 0);
    int held = got == 1 && pending == block;
    sys(SYS_rt_sigprocmask, 1, (i64)&block, 0, 8, 0, 0);
    check(held && got == 2, "a blocked signal waits, pending, until it's unblocked");

    handle(SIGSEGV, on_segv, 0);
    volatile int *bad = (volatile int *)0x1000;
    load(bad);
    check(fault_addr == 0x1000, "SIGSEGV's handler sees the address, and moves the program on");

    i64 child = fork();
    if (child == 0) { for (;;) sleep_ms(1000); }
    sleep_ms(20);
    sys(SYS_kill, child, SIGTERM, 0, 0, 0, 0);
    check(wait_for(child, 0) == SIGTERM, "SIGTERM ends a sleeping child, and wait4 says so");

    child = fork();
    if (child == 0) { for (;;) sleep_ms(1000); }
    sys(SYS_kill, child, SIGSTOP, 0, 0, 0, 0);
    int stopped = wait_for(child, 2);
    sys(SYS_kill, child, SIGCONT, 0, 0, 0, 0);
    int continued = wait_for(child, 8);
    sys(SYS_kill, child, SIGKILL, 0, 0, 0, 0);
    int killed = wait_for(child, 0);
    check(stopped == (0x7f | SIGSTOP << 8) && continued == 0xffff && killed == SIGKILL, "SIGSTOP, SIGCONT and SIGKILL, as wait4 reports them");

    int fds[2];
    char buf[64];
    for (int restart = 1; restart >= 0; restart--) {
        sys(SYS_pipe2, (i64)fds, 0, 0, 0, 0, 0);
        handle(SIGUSR1, on_usr1, restart ? SA_RESTART : 0);
        got = 0;
        child = fork();
        if (child == 0) {
            sleep_ms(30);
            sys(SYS_kill, sys(SYS_getppid, 0, 0, 0, 0, 0, 0), SIGUSR1, 0, 0, 0, 0);
            sleep_ms(30);
            sys(SYS_write, fds[1], (i64)"late", 4, 0, 0, 0);
            sys(SYS_exit_group, 0, 0, 0, 0, 0, 0);
        }
        i64 n = sys(SYS_read, fds[0], (i64)buf, sizeof buf, 0, 0, 0);
        wait_for(child, 0);
        sys(SYS_close, fds[0], 0, 0, 0, 0, 0);
        sys(SYS_close, fds[1], 0, 0, 0, 0, 0);
        if (restart) check(got == 1 && n == 4, "SA_RESTART: a read the signal interrupted carries on");
        else check(got == 1 && n == -4, "without SA_RESTART: the read fails with EINTR");
    }

    child = fork();
    if (child == 0) {
        sys(SYS_pipe2, (i64)fds, 0, 0, 0, 0, 0);
        sys(SYS_close, fds[0], 0, 0, 0, 0, 0);
        sys(SYS_write, fds[1], (i64)"x", 1, 0, 0, 0);
        sys(SYS_exit_group, 0, 0, 0, 0, 0, 0);
    }
    check(wait_for(child, 0) == SIGPIPE, "writing to a pipe no one reads sends SIGPIPE");
    ignore(SIGPIPE);
    sys(SYS_pipe2, (i64)fds, 0, 0, 0, 0, 0);
    sys(SYS_close, fds[0], 0, 0, 0, 0, 0);
    check(sys(SYS_write, fds[1], (i64)"x", 1, 0, 0, 0) == -32, "with SIGPIPE ignored, it's EPIPE");
    sys(SYS_close, fds[1], 0, 0, 0, 0, 0);

    int master = sys(SYS_openat, AT_FDCWD, (i64)"/dev/ptmx", 2, 0, 0, 0);
    unsigned number = 99, unlock = 0;
    sys(SYS_ioctl, master, 0x40045431, (i64)&unlock, 0, 0, 0);
    sys(SYS_ioctl, master, 0x80045430, (i64)&number, 0, 0, 0);
    char path[] = "/dev/pts/0";
    path[9] = '0' + number;
    int slave = sys(SYS_openat, AT_FDCWD, (i64)path, 2 | 0400, 0, 0, 0);
    sys(SYS_write, master, (i64)"typed\r", 6, 0, 0, 0);
    i64 n = sys(SYS_read, slave, (i64)buf, sizeof buf, 0, 0, 0);
    int line_ok = n == 6 && same(buf, "typed\n", 6);
    n = sys(SYS_read, master, (i64)buf, sizeof buf, 0, 0, 0); // the echo
    int echo_ok = n == 7 && same(buf, "typed\r\n", 7);
    sys(SYS_write, slave, (i64)"out\n", 4, 0, 0, 0);
    n = sys(SYS_read, master, (i64)buf, sizeof buf, 0, 0, 0);
    check(master >= 0 && slave >= 0 && line_ok && echo_ok && n == 5 && same(buf, "out\r\n", 5), "a pty: a line typed in, echoed, and output with \\r\\n");
    sys(SYS_close, slave, 0, 0, 0, 0, 0);

    child = fork();
    if (child == 0) {
        sys(SYS_setsid, 0, 0, 0, 0, 0, 0);
        int tty = sys(SYS_openat, AT_FDCWD, (i64)path, 2, 0, 0, 0); // becomes its controlling terminal
        sys(SYS_read, tty, (i64)buf, sizeof buf, 0, 0, 0);
        sys(SYS_exit_group, 0, 0, 0, 0, 0, 0);
    }
    sleep_ms(30);
    sys(SYS_write, master, (i64)"\x03", 1, 0, 0, 0);
    check(wait_for(child, 0) == SIGINT, "^C on a pty sends SIGINT to its foreground process group");

    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
