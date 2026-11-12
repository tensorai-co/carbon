// Signals, timers and pidfds, for lambda: real-time signals queued with their values and in order, standard ones
// merged; rt_sigtimedwait (a timeout, nothing waiting, woken by another process, interrupted by a handler);
// rt_sigqueueinfo's and rt_tgsigqueueinfo's checks; pause; setitimer and getitimer (real, virtual and profiling time),
// alarm; POSIX timers with their signals, values, overruns, absolute times, SIGEV_NONE, a thread's own signal, a
// CPU-time clock, and deletion taking a queued signal back; clock_settime, settimeofday and adjtimex; pidfds by
// pidfd_open and CLONE_PIDFD, polled, signalled, waited for (non-blocking too) and copied from with pidfd_getfd.
// Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_openat = 56, SYS_close = 57, SYS_write = 64, SYS_ppoll = 73, SYS_exit_group = 94, SYS_waitid = 95,
       SYS_nanosleep = 101, SYS_getitimer = 102, SYS_setitimer = 103, SYS_timer_create = 107,
       SYS_timer_gettime = 108, SYS_timer_getoverrun = 109, SYS_timer_settime = 110, SYS_timer_delete = 111,
       SYS_clock_settime = 112, SYS_clock_gettime = 113, SYS_sched_yield = 124, SYS_kill = 129,
       SYS_rt_sigaction = 134, SYS_rt_sigprocmask = 135, SYS_rt_sigtimedwait = 137, SYS_rt_sigqueueinfo = 138,
       SYS_settimeofday = 170, SYS_adjtimex = 171, SYS_getpid = 172, SYS_gettid = 178, SYS_clone = 220,
       SYS_wait4 = 260, SYS_rt_tgsigqueueinfo = 240, SYS_clock_adjtime = 266, SYS_kcmp = 272,
       SYS_pidfd_send_signal = 424, SYS_pidfd_open = 434, SYS_pidfd_getfd = 438 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#else
enum { SYS_write = 1, SYS_close = 3, SYS_rt_sigaction = 13, SYS_rt_sigprocmask = 14, SYS_sched_yield = 24,
       SYS_pause = 34, SYS_nanosleep = 35, SYS_getitimer = 36, SYS_alarm = 37, SYS_setitimer = 38, SYS_getpid = 39,
       SYS_clone = 56, SYS_wait4 = 61, SYS_kill = 62, SYS_rt_sigtimedwait = 128, SYS_rt_sigqueueinfo = 129,
       SYS_adjtimex = 159, SYS_settimeofday = 164, SYS_gettid = 186, SYS_timer_create = 222,
       SYS_timer_settime = 223, SYS_timer_gettime = 224, SYS_timer_getoverrun = 225, SYS_timer_delete = 226,
       SYS_clock_settime = 227, SYS_clock_gettime = 228, SYS_exit_group = 231, SYS_waitid = 247, SYS_openat = 257,
       SYS_ppoll = 271, SYS_rt_tgsigqueueinfo = 297, SYS_clock_adjtime = 305, SYS_kcmp = 312,
       SYS_pidfd_send_signal = 424, SYS_pidfd_open = 434, SYS_pidfd_getfd = 438 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#endif

#define AT_FDCWD (-100)
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define O_NONBLOCK 04000
#define EPERM 1
#define ESRCH 3
#define EINTR 4
#define EBADF 9
#define EAGAIN 11
#define EINVAL 22
#define SIGKILL 9
#define SIGUSR1 10
#define SIGUSR2 12
#define SIGALRM 14
#define SIGCHLD 17
#define SIGVTALRM 26
#define SIGPROF 27
#define SI_QUEUE (-1)
#define SI_TIMER (-2)
#define CLONE_PIDFD 0x1000
#define P_PIDFD 3
#define WEXITED 4

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static u64 bit(int s) { return 1UL << (s - 1); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static i64 now(int clock) { i64 ts[2]; sys(SYS_clock_gettime, clock, (i64)ts, 0, 0, 0, 0); return ts[0] * 1000000000 + ts[1]; }
static void sleep_ms(int ms) { i64 ts[2] = {ms / 1000, (i64)(ms % 1000) * 1000000}; while (sys(SYS_nanosleep, (i64)ts, (i64)ts, 0, 0, 0, 0) == -EINTR) {} }
static void burn(int ms) {
    volatile u64 x = 1;
    i64 until = now(1) + (i64)ms * 1000000;
    while (now(1) < until) for (int i = 0; i < 10000; i++) x = x * 6364136223846793005UL + 1;
}
static void block(u64 set) { sys(SYS_rt_sigprocmask, 0, (i64)&set, 0, 8, 0, 0); }
// Takes a signal in `set`, waiting `ms` (-1: for ever). `si` gets its siginfo.
static i64 take(u64 set, int *si, int ms) {
    i64 ts[2] = {ms / 1000, (i64)(ms % 1000) * 1000000};
    return sys(SYS_rt_sigtimedwait, (i64)&set, (i64)si, ms < 0 ? 0 : (i64)ts, 8, 0, 0);
}
static i64 queue(int pid, int signal, i64 value) {
    int si[32] = {0};
    si[0] = signal; si[2] = SI_QUEUE; si[4] = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    *(i64 *)&si[6] = value;
    return sys(SYS_rt_sigqueueinfo, pid, signal, (i64)si, 0, 0, 0);
}
static i64 value_of(int *si) { return *(i64 *)&si[6]; }

static volatile int usr2;
static void on_usr2(int s) { (void)s; usr2 = usr2 + 1; }

void cmain(i64 *sp) {
    (void)sp;
    int pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0), tid = sys(SYS_gettid, 0, 0, 0, 0, 0, 0);
    int si[32];
    block(bit(32 + 2) | bit(32 + 3) | bit(32 + 4) | bit(SIGUSR1) | bit(SIGALRM) | bit(SIGVTALRM) | bit(SIGPROF) | bit(40) | bit(41) | bit(42) | bit(43));

    // Real-time signals queue, with their values, in order; standard ones merge.
    queue(pid, 34, 11); queue(pid, 34, 22); queue(pid, 34, 33);
    i64 a = take(bit(34), si, 0); i64 va = value_of(si); int code = si[2], from = si[4];
    i64 b = take(bit(34), si, 0); i64 vb = value_of(si);
    i64 c = take(bit(34), si, 0); i64 vc = value_of(si);
    check(a == 34 && b == 34 && c == 34 && va == 11 && vb == 22 && vc == 33 && code == SI_QUEUE && from == pid, "a real-time signal sent three times is taken three times, values in order");
    check(take(bit(34), si, 0) == -EAGAIN, "then there are none: EAGAIN");
    sys(SYS_kill, pid, SIGUSR1, 0, 0, 0, 0); sys(SYS_kill, pid, SIGUSR1, 0, 0, 0, 0);
    check(take(bit(SIGUSR1), si, 0) == SIGUSR1 && si[4] == pid && take(bit(SIGUSR1), si, 0) == -EAGAIN, "a standard signal sent twice is pending once");
    queue(pid, 36, 1); queue(pid, 34, 2);
    check(take(bit(34) | bit(36), si, 0) == 34 && take(bit(34) | bit(36), si, 0) == 36, "the lowest signal comes first");
    int ti[32] = {0};
    ti[0] = 35; ti[2] = SI_QUEUE; *(i64 *)&ti[6] = 5150;
    check(sys(SYS_rt_tgsigqueueinfo, pid, tid, 35, (i64)ti, 0, 0) == 0 && take(bit(35), si, 0) == 35 && value_of(si) == 5150, "rt_tgsigqueueinfo to this thread");
    i64 t0 = now(1);
    check(take(bit(35), si, 60) == -EAGAIN && now(1) - t0 >= 50000000, "rt_sigtimedwait times out with EAGAIN");
    i64 set = bit(34);
    check(sys(SYS_rt_sigtimedwait, (i64)&set, 0, 0, 4, 0, 0) == -EINVAL, "rt_sigtimedwait: a bad set size");

    // Woken by another process's signal, blocked here.
    i64 child = fork();
    if (child == 0) { sleep_ms(50); queue(pid, 35, 77); quit(0); }
    check(take(bit(35), si, 3000) == 35 && value_of(si) == 77 && si[4] == child, "rt_sigtimedwait woken by another process's sigqueue");
    sys(SYS_wait4, child, 0, 0, 0, 0, 0);
    // A siginfo with a code of the kernel's (SI_USER) may only go to the sender itself.
    int kernel_code[32] = {0};
    kernel_code[0] = 35;
    child = fork();
    if (child == 0) quit(sys(SYS_rt_sigqueueinfo, pid, 35, (i64)kernel_code, 0, 0, 0) == -EPERM ? 0 : 1);
    int status = -1;
    sys(SYS_wait4, child, (i64)&status, 0, 0, 0, 0);
    check(status == 0 && sys(SYS_rt_sigqueueinfo, pid, 35, (i64)kernel_code, 0, 0, 0) == 0 && take(bit(35), si, 0) == 35 && take(bit(35), si, 0) == -EAGAIN,
          "rt_sigqueueinfo: a kernel's code to another process EPERM, to itself allowed");

    // Interrupted by a handled signal it doesn't wait for.
    u64 act[4] = {(u64)on_usr2, 0, 0, 0};
    sys(SYS_rt_sigaction, SIGUSR2, (i64)act, 0, 8, 0, 0);
    child = fork();
    if (child == 0) { sleep_ms(50); sys(SYS_kill, pid, SIGUSR2, 0, 0, 0, 0); quit(0); }
    check(take(bit(35), si, 3000) == -EINTR && usr2 == 1, "rt_sigtimedwait interrupted by a handled signal: EINTR, the handler ran");
    sys(SYS_wait4, child, 0, 0, 0, 0, 0);
#if !defined(__aarch64__)
    child = fork();
    if (child == 0) { sleep_ms(50); sys(SYS_kill, pid, SIGUSR2, 0, 0, 0, 0); quit(0); }
    check(sys(SYS_pause, 0, 0, 0, 0, 0, 0) == -EINTR && usr2 == 2, "pause: until a handler runs");
    sys(SYS_wait4, child, 0, 0, 0, 0, 0);
#endif

    // Interval timers.
    i64 itv[4] = {0, 20000, 0, 20000}, old[4] = {-1, -1, -1, -1}, cur[4];
    t0 = now(1);
    check(sys(SYS_setitimer, 0, (i64)itv, (i64)old, 0, 0, 0) == 0 && old[0] == 0 && old[2] == 0 && old[3] == 0, "setitimer ITIMER_REAL every 20 ms; none before");
    check(take(bit(SIGALRM), si, 1000) == SIGALRM && take(bit(SIGALRM), si, 1000) == SIGALRM && now(1) - t0 >= 30000000, "SIGALRM, and again 20 ms later");
    check(sys(SYS_getitimer, 0, (i64)cur, 0, 0, 0, 0) == 0 && cur[1] == 20000 && cur[2] == 0 && cur[3] > 0 && cur[3] <= 20000, "getitimer: its interval, and what's left");
    i64 zero[4] = {0, 0, 0, 0};
    sys(SYS_setitimer, 0, (i64)zero, (i64)old, 0, 0, 0);
    take(bit(SIGALRM), si, 0);
    check(old[1] == 20000 && take(bit(SIGALRM), si, 60) == -EAGAIN, "disarmed: the old one back, and no more SIGALRM");
    check(sys(SYS_setitimer, 5, (i64)itv, 0, 0, 0, 0) == -EINVAL && (itv[3] = 1000000, sys(SYS_setitimer, 0, (i64)itv, 0, 0, 0, 0)) == -EINVAL, "setitimer: a bad which, a million microseconds");
#if !defined(__aarch64__)
    check(sys(SYS_alarm, 5, 0, 0, 0, 0, 0) == 0 && sys(SYS_alarm, 0, 0, 0, 0, 0, 0) == 5, "alarm: none before; then the 5 s left, rounded");
#endif
    i64 virt[4] = {0, 0, 0, 50000};
    sys(SYS_setitimer, 1, (i64)virt, 0, 0, 0, 0);
    sys(SYS_setitimer, 2, (i64)virt, 0, 0, 0, 0);
    burn(300);
    check(take(bit(SIGVTALRM), si, 0) == SIGVTALRM && take(bit(SIGPROF), si, 0) == SIGPROF, "ITIMER_VIRTUAL and ITIMER_PROF: SIGVTALRM and SIGPROF after 50 ms of CPU");

    // POSIX timers.
    struct { i64 value; int signo, notify, tid, pad[11]; } ev = {1234, 40, 0, 0};
    int id = -1;
    check(sys(SYS_timer_create, 1, (i64)&ev, (i64)&id, 0, 0, 0) == 0 && id >= 0, "timer_create on CLOCK_MONOTONIC");
    i64 its[4] = {0, 0, 0, 30000000};
    check(sys(SYS_timer_settime, id, 0, (i64)its, 0, 0, 0) == 0 && take(bit(40), si, 1000) == 40 && si[2] == SI_TIMER && si[4] == id && value_of(si) == 1234,
          "its signal: SI_TIMER, its ID and value");
    i64 periodic[4] = {0, 5000000, 0, 5000000};
    sys(SYS_timer_settime, id, 0, (i64)periodic, 0, 0, 0);
    sleep_ms(100);
    check(take(bit(40), si, 0) == 40 && si[5] >= 5 && sys(SYS_timer_getoverrun, id, 0, 0, 0, 0, 0) == si[5] && take(bit(40), si, 0) == -EAGAIN,
          "every 5 ms for 100 ms, blocked: one signal, the rest its overrun");
    i64 one_s[4] = {0, 7000000, 1, 0};
    sys(SYS_timer_settime, id, 0, (i64)one_s, 0, 0, 0);
    check(sys(SYS_timer_gettime, id, (i64)cur, 0, 0, 0, 0) == 0 && cur[0] == 0 && cur[1] == 7000000 && cur[2] * 1000000000 + cur[3] > 500000000, "timer_gettime: interval and what's left");
    sys(SYS_timer_settime, id, 0, (i64)periodic, 0, 0, 0);
    sleep_ms(20);
    check(sys(SYS_timer_delete, id, 0, 0, 0, 0, 0) == 0 && take(bit(40), si, 0) == -EAGAIN && sys(SYS_timer_delete, id, 0, 0, 0, 0, 0) == -EINVAL,
          "timer_delete takes its queued signal with it; twice is EINVAL");
    ev.signo = 41;
    sys(SYS_timer_create, 0, (i64)&ev, (i64)&id, 0, 0, 0);
    i64 at = now(0) + 30000000, abs[4] = {0, 0, at / 1000000000, at % 1000000000};
    check(sys(SYS_timer_settime, id, 1, (i64)abs, 0, 0, 0) == 0 && take(bit(41), si, 1000) == 41 && now(0) >= at, "TIMER_ABSTIME on CLOCK_REALTIME");
    sys(SYS_timer_delete, id, 0, 0, 0, 0, 0);
    ev.notify = 1;
    sys(SYS_timer_create, 1, (i64)&ev, (i64)&id, 0, 0, 0);
    its[3] = 20000000;
    sys(SYS_timer_settime, id, 0, (i64)its, 0, 0, 0);
    sleep_ms(50);
    check(sys(SYS_timer_gettime, id, (i64)cur, 0, 0, 0, 0) == 0 && cur[2] == 0 && cur[3] == 0 && take(bit(41), si, 0) == -EAGAIN, "SIGEV_NONE: expired, and no signal");
    sys(SYS_timer_delete, id, 0, 0, 0, 0, 0);
    ev.notify = 4; ev.tid = tid; ev.signo = 43; ev.value = 4343;
    sys(SYS_timer_create, 1, (i64)&ev, (i64)&id, 0, 0, 0);
    sys(SYS_timer_settime, id, 0, (i64)its, 0, 0, 0);
    check(take(bit(43), si, 1000) == 43 && value_of(si) == 4343, "SIGEV_THREAD_ID: to this thread");
    sys(SYS_timer_delete, id, 0, 0, 0, 0, 0);
    ev.notify = 0; ev.signo = 42;
    its[3] = 50000000;
    check(sys(SYS_timer_create, 2, (i64)&ev, (i64)&id, 0, 0, 0) == 0 && sys(SYS_timer_settime, id, 0, (i64)its, 0, 0, 0) == 0, "a timer on CLOCK_PROCESS_CPUTIME_ID");
    burn(300);
    check(take(bit(42), si, 0) == 42, "it fires after 50 ms of CPU");
    sys(SYS_timer_delete, id, 0, 0, 0, 0, 0);
    i64 bad[4] = {0, 0, 0, 1000000000};
    check(sys(SYS_timer_create, 4, (i64)&ev, (i64)&id, 0, 0, 0) == -EINVAL && sys(SYS_timer_settime, 999, 0, (i64)its, 0, 0, 0) == -EINVAL &&
          (sys(SYS_timer_create, 1, 0, (i64)&id, 0, 0, 0), sys(SYS_timer_settime, id, 0, (i64)bad, 0, 0, 0)) == -EINVAL,
          "timers: CLOCK_MONOTONIC_RAW, no such timer, a billion nanoseconds");

    // Setting the clock.
    i64 ts[2] = {1700000000, 0};
    check(sys(SYS_clock_settime, 0, (i64)ts, 0, 0, 0, 0) == -EPERM && sys(SYS_clock_settime, 1, (i64)ts, 0, 0, 0, 0) == -EINVAL, "clock_settime: real time EPERM, monotonic EINVAL");
    check(sys(SYS_settimeofday, 0, 0, 0, 0, 0, 0) == 0 && sys(SYS_settimeofday, (i64)ts, 0, 0, 0, 0, 0) == -EPERM, "settimeofday: nothing to set, or EPERM");
    i64 tx[26] = {0};
    check(sys(SYS_adjtimex, (i64)tx, 0, 0, 0, 0, 0) == 0 && (((int *)tx)[10] & 0x40) == 0 && tx[3] < 16000000 && ((int *)tx)[40] == 37 && tx[9] > 1700000000,
          "adjtimex: TIME_OK, synchronized, TAI 37 s, the time");
    tx[0] = 2;
    check(sys(SYS_adjtimex, (i64)tx, 0, 0, 0, 0, 0) == -EPERM && (tx[0] = 0, sys(SYS_clock_adjtime, 1, (i64)tx, 0, 0, 0, 0)) == -EINVAL,
          "adjtimex adjusting: EPERM; clock_adjtime on monotonic: EINVAL");

    // pidfds.
    int keep = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/pidfile", O_RDWR | O_CREAT | O_TRUNC, 0644, 0, 0);
    child = fork();
    if (child == 0) { sleep_ms(10000); quit(0); }
    int pfd = sys(SYS_pidfd_open, child, 0, 0, 0, 0, 0), nb = sys(SYS_pidfd_open, child, O_NONBLOCK, 0, 0, 0, 0);
    struct { int fd; short events, revents; } pfds = {pfd, 1, 0};
    i64 zero_ts[2] = {0, 0}, second[2] = {1, 0};
    check(pfd >= 0 && sys(SYS_ppoll, (i64)&pfds, 1, (i64)zero_ts, 0, 8, 0) == 0, "pidfd_open: not readable while it runs");
    check(sys(SYS_waitid, P_PIDFD, nb, (i64)si, WEXITED, 0, 0) == -EAGAIN, "waitid on a non-blocking pidfd of a running child: EAGAIN");
    int copied = sys(SYS_pidfd_getfd, pfd, keep, 0, 0, 0, 0);
    check(copied >= 0 && sys(SYS_kcmp, pid, pid, 0, copied, keep, 0) == 0, "pidfd_getfd: the child's file is ours");
    check(sys(SYS_pidfd_getfd, pfd, 900, 0, 0, 0, 0) == -EBADF && sys(SYS_pidfd_getfd, pfd, keep, 1, 0, 0, 0) == -EINVAL && sys(SYS_pidfd_getfd, keep, 0, 0, 0, 0, 0) == -EBADF,
          "pidfd_getfd: no such file, a flag, not a pidfd");
    check(sys(SYS_pidfd_send_signal, pfd, SIGKILL, 0, 0, 0, 0) == 0 && sys(SYS_ppoll, (i64)&pfds, 1, (i64)second, 0, 8, 0) == 1 && (pfds.revents & 1),
          "pidfd_send_signal SIGKILL; the pidfd becomes readable");
    check(sys(SYS_waitid, P_PIDFD, pfd, (i64)si, WEXITED, 0, 0) == 0 && si[2] == 2 && si[6] == SIGKILL && si[4] == child, "waitid P_PIDFD: killed by SIGKILL");
    check(sys(SYS_pidfd_send_signal, pfd, SIGUSR1, 0, 0, 0, 0) == -ESRCH, "a pidfd outlives its process: ESRCH");
    check(sys(SYS_pidfd_open, 0, 0, 0, 0, 0, 0) == -EINVAL && sys(SYS_pidfd_open, 999999, 0, 0, 0, 0, 0) == -ESRCH && sys(SYS_pidfd_open, pid, 1, 0, 0, 0, 0) == -EINVAL,
          "pidfd_open: pid 0, no such process, a bad flag");
    int self = sys(SYS_pidfd_open, pid, 0, 0, 0, 0, 0);
    int qi[32] = {0};
    qi[0] = 43; qi[2] = SI_QUEUE; *(i64 *)&qi[6] = 99;
    check(sys(SYS_pidfd_send_signal, self, 43, (i64)qi, 0, 0, 0) == 0 && take(bit(43), si, 0) == 43 && value_of(si) == 99, "pidfd_send_signal with a siginfo, to itself");
    int cpidfd = -1;
    child = sys(SYS_clone, SIGCHLD | CLONE_PIDFD, 0, (i64)&cpidfd, 0, 0, 0);
    if (child == 0) quit(3);
    check(child > 0 && cpidfd >= 0 && sys(SYS_waitid, P_PIDFD, cpidfd, (i64)si, WEXITED, 0, 0) == 0 && si[6] == 3 && si[4] == child, "clone with CLONE_PIDFD, waited for by it");
    check(sys(SYS_clone, SIGCHLD | CLONE_PIDFD | 0x100000, 0, (i64)&cpidfd, 0, 0, 0) == -EINVAL, "CLONE_PIDFD with CLONE_PARENT_SETTID: EINVAL");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
