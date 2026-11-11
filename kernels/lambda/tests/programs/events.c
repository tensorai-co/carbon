// Waiting without polling, for lambda: eventfd, timerfd, epoll, signalfd, and ppoll's signal mask. Freestanding, for
// ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;

#if defined(__aarch64__)
enum { SYS_eventfd2 = 19, SYS_epoll_create1 = 20, SYS_epoll_ctl = 21, SYS_epoll_pwait = 22, SYS_openat = 56,
       SYS_close = 57, SYS_pipe2 = 59, SYS_read = 63, SYS_write = 64, SYS_ppoll = 73, SYS_signalfd4 = 74,
       SYS_timerfd_create = 85, SYS_timerfd_settime = 86, SYS_timerfd_gettime = 87, SYS_exit_group = 94,
       SYS_nanosleep = 101, SYS_clock_gettime = 113, SYS_kill = 129, SYS_rt_sigaction = 134, SYS_rt_sigprocmask = 135,
       SYS_getpid = 172, SYS_getppid = 173, SYS_clone = 220, SYS_wait4 = 260, SYS_epoll_pwait2 = 441 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define RESTORER 0
#define SA_RESTORER_FLAG 0
// ARM64's struct epoll_event is padded.
struct event { unsigned events; u64 data; };
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_rt_sigaction = 13, SYS_rt_sigprocmask = 14, SYS_nanosleep = 35,
       SYS_getpid = 39, SYS_clone = 56, SYS_wait4 = 61, SYS_kill = 62, SYS_getppid = 110, SYS_epoll_create = 213,
       SYS_clock_gettime = 228, SYS_exit_group = 231, SYS_epoll_wait = 232, SYS_epoll_ctl = 233, SYS_openat = 257,
       SYS_ppoll = 271, SYS_epoll_pwait = 281, SYS_signalfd = 282, SYS_timerfd_create = 283, SYS_eventfd = 284,
       SYS_timerfd_settime = 286, SYS_timerfd_gettime = 287, SYS_signalfd4 = 289, SYS_eventfd2 = 290,
       SYS_epoll_create1 = 291, SYS_pipe2 = 293, SYS_epoll_pwait2 = 441 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
__asm__(".globl restorer\nrestorer:\n mov $15, %eax\n syscall\n");
void restorer(void);
#define RESTORER ((u64)restorer)
#define SA_RESTORER_FLAG 0x04000000
// x86-64's struct epoll_event is packed.
struct __attribute__((packed)) event { unsigned events; u64 data; };
#endif

#define AT_FDCWD (-100)
#define O_RDONLY 0
#define O_NONBLOCK 0x800
#define SIGUSR1 10
#define SIGCHLD 17
#define SA_SIGINFO 4
#define SIG_BLOCK 0
#define SIG_SETMASK 2
#define EINTR 4
#define EAGAIN 11
#define EEXIST 17
#define EINVAL 22
#define EPERM 1
#define ENOENT 2
#define POLLIN 1
#define EPOLLIN 1
#define EPOLLOUT 4
#define EPOLLHUP 0x10
#define EPOLLONESHOT (1u << 30)
#define EPOLLET (1u << 31)
#define EPOLL_CTL_ADD 1
#define EPOLL_CTL_DEL 2
#define EPOLL_CTL_MOD 3
#define EFD_SEMAPHORE 1
#define CLOCK_MONOTONIC 1

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static void sleep_ms(int ms) { i64 ts[2] = {0, ms * 1000000L}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static int wait_for(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return status; }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static i64 now_ms(void) { i64 ts[2]; sys(SYS_clock_gettime, CLOCK_MONOTONIC, (i64)ts, 0, 0, 0, 0); return ts[0] * 1000 + ts[1] / 1000000; }

static u64 rd64(int fd) { u64 v = 0; i64 n = sys(SYS_read, fd, (i64)&v, 8, 0, 0, 0); return n == 8 ? v : (u64)n; }
static i64 wr64(int fd, u64 v) { return sys(SYS_write, fd, (i64)&v, 8, 0, 0, 0); }
static i64 ctl(int ep, int op, int fd, unsigned events, u64 data) {
    struct event e = {events, data};
    return sys(SYS_epoll_ctl, ep, op, fd, (i64)&e, 0, 0);
}
static i64 ewait(int ep, struct event *out, int most, int ms) { return sys(SYS_epoll_pwait, ep, (i64)out, most, ms, 0, 8); }
static void arm(int tfd, i64 value_ms, i64 interval_ms) {
    i64 spec[4] = {interval_ms / 1000, interval_ms % 1000 * 1000000, value_ms / 1000, value_ms % 1000 * 1000000};
    sys(SYS_timerfd_settime, tfd, 0, (i64)spec, 0, 0, 0);
}
static i64 poll1(int fd, int ms, short *revents) {
    struct { int fd; short events, revents; } p = {fd, POLLIN, 0};
    i64 ts[2] = {ms / 1000, ms % 1000 * 1000000L};
    i64 n = sys(SYS_ppoll, (i64)&p, 1, ms < 0 ? 0 : (i64)ts, 0, 8, 0);
    *revents = p.revents;
    return n;
}

static volatile int got;
static void on_usr1(int sig, int *info, char *uc) { got = got + 1; (void)sig; (void)info; (void)uc; }

void cmain(i64 *sp) {
    (void)sp;
    u64 v;
    struct event ev[8];
    short revents;

    // eventfd: a counter.
    int efd = sys(SYS_eventfd2, 3, O_NONBLOCK, 0, 0, 0, 0);
    v = rd64(efd);
    check(efd >= 0 && v == 3 && rd64(efd) == (u64)-EAGAIN, "eventfd: a read takes the count; an empty one says EAGAIN");
    check(wr64(efd, 5) == 8 && wr64(efd, 2) == 8 && rd64(efd) == 7, "eventfd: writes add up");
    check(wr64(efd, (u64)-1) == -EINVAL, "eventfd: all ones can't be written");
    int sem = sys(SYS_eventfd2, 2, O_NONBLOCK | EFD_SEMAPHORE, 0, 0, 0, 0);
    check(rd64(sem) == 1 && rd64(sem) == 1 && rd64(sem) == (u64)-EAGAIN, "eventfd: EFD_SEMAPHORE takes one at a time");
    check(sys(SYS_eventfd2, 0, 0x40, 0, 0, 0, 0) == -EINVAL, "eventfd: unknown flags are EINVAL");

    // A blocking eventfd read, woken by another process's write.
    int bfd = sys(SYS_eventfd2, 0, 0, 0, 0, 0, 0);
    i64 child = fork();
    if (child == 0) {
        sleep_ms(30);
        wr64(bfd, 42);
        quit(0);
    }
    i64 t0 = now_ms();
    v = rd64(bfd);
    i64 took = now_ms() - t0;
    check(v == 42 && took >= 20 && wait_for(child) == 0, "eventfd: a blocking read waits for another process's write");

    // timerfd.
    int tfd = sys(SYS_timerfd_create, CLOCK_MONOTONIC, 0, 0, 0, 0, 0);
    check(tfd >= 0 && sys(SYS_timerfd_create, 3, 0, 0, 0, 0, 0) == -EINVAL, "timerfd: made on CLOCK_MONOTONIC; CLOCK_PROCESS_CPUTIME_ID refused");
    arm(tfd, 40, 0);
    t0 = now_ms();
    v = rd64(tfd);
    took = now_ms() - t0;
    check(v == 1 && took >= 30, "timerfd: a blocking read waits for a one-shot timer, which expired once");
    i64 cur[4] = {9, 9, 9, 9};
    sys(SYS_timerfd_gettime, tfd, (i64)cur, 0, 0, 0, 0);
    check(cur[0] == 0 && cur[1] == 0 && cur[2] == 0 && cur[3] == 0, "timerfd: an expired one-shot timer is disarmed");
    t0 = now_ms();
    arm(tfd, 10, 10);
    sleep_ms(55);
    v = rd64(tfd);
    took = now_ms() - t0;
    // One expiry for every 10 ms that passed (a sleep can take longer than asked on a coarse host clock), give or take one.
    check(v >= 4 && (i64)v <= took / 10 + 1 && (i64)v + 1 >= took / 10, "timerfd: an interval timer counts every expiry since the last read");
    sys(SYS_timerfd_gettime, tfd, (i64)cur, 0, 0, 0, 0);
    check(cur[0] == 0 && cur[1] == 10000000 && cur[2] == 0 && cur[3] > 0 && cur[3] <= 10000000, "timerfd: gettime says the interval and what's left");
    arm(tfd, 0, 0);
    int ntfd = sys(SYS_timerfd_create, CLOCK_MONOTONIC, O_NONBLOCK, 0, 0, 0, 0);
    sys(SYS_timerfd_gettime, tfd, (i64)cur, 0, 0, 0, 0);
    check(rd64(ntfd) == (u64)-EAGAIN && cur[2] == 0 && cur[3] == 0, "timerfd: a disarmed timer has nothing to read");

    // epoll.
    int ep = sys(SYS_epoll_create1, 0, 0, 0, 0, 0, 0);
    int pipe_fds[2];
    sys(SYS_pipe2, (i64)pipe_fds, 0, 0, 0, 0, 0);
    check(ep >= 0 && ctl(ep, EPOLL_CTL_ADD, pipe_fds[0], EPOLLIN, 111) == 0 && ctl(ep, EPOLL_CTL_ADD, efd, EPOLLIN, 222) == 0,
          "epoll: a pipe and an eventfd added");
    check(ctl(ep, EPOLL_CTL_ADD, efd, EPOLLIN, 222) == -EEXIST && ctl(ep, EPOLL_CTL_DEL, sem, 0, 0) == -ENOENT && ctl(ep, EPOLL_CTL_ADD, ep, EPOLLIN, 0) == -EINVAL,
          "epoll: adding twice, deleting what isn't there and adding itself are refused");
    t0 = now_ms();
    i64 n = ewait(ep, ev, 8, 30);
    took = now_ms() - t0;
    check(n == 0 && took >= 20, "epoll: nothing ready, so a wait times out");
    sys(SYS_write, pipe_fds[1], (i64)"x", 1, 0, 0, 0);
    n = ewait(ep, ev, 8, 0);
    check(n == 1 && ev[0].events == EPOLLIN && ev[0].data == 111, "epoll: the pipe is reported with its data");
    n = ewait(ep, ev, 8, 0);
    check(n == 1 && ev[0].data == 111, "epoll: level-triggered, it's reported again while it's ready");
    char c;
    sys(SYS_read, pipe_fds[0], (i64)&c, 1, 0, 0, 0);
    check(ewait(ep, ev, 8, 0) == 0, "epoll: read empty, it's not");

    check(ctl(ep, EPOLL_CTL_MOD, efd, EPOLLIN | EPOLLET, 333) == 0, "epoll: the eventfd made edge-triggered");
    wr64(efd, 1);
    n = ewait(ep, ev, 8, 0);
    check(n == 1 && ev[0].data == 333, "epoll: edge-triggered, a write is reported once");
    check(ewait(ep, ev, 8, 0) == 0, "epoll: and not again while nothing new happens");
    wr64(efd, 1);
    check(ewait(ep, ev, 8, 0) == 1, "epoll: another write is reported again");
    rd64(efd);

    ctl(ep, EPOLL_CTL_MOD, efd, EPOLLIN | EPOLLONESHOT, 444);
    wr64(efd, 1);
    n = ewait(ep, ev, 8, 0);
    int once = n == 1 && ev[0].data == 444 && ewait(ep, ev, 8, 0) == 0;
    ctl(ep, EPOLL_CTL_MOD, efd, EPOLLIN | EPOLLONESHOT, 555);
    n = ewait(ep, ev, 8, 0);
    check(once && n == 1 && ev[0].data == 555, "epoll: one-shot, reported once until EPOLL_CTL_MOD arms it again");
    rd64(efd);
    ctl(ep, EPOLL_CTL_DEL, efd, 0, 0);

    // An eventfd that's full isn't writable; an empty one is.
    int wfd = sys(SYS_eventfd2, 0, O_NONBLOCK, 0, 0, 0, 0);
    ctl(ep, EPOLL_CTL_ADD, wfd, EPOLLOUT, 666);
    n = ewait(ep, ev, 8, 0);
    int writable = n == 1 && ev[0].events == EPOLLOUT;
    wr64(wfd, 0xfffffffffffffffeUL);
    check(writable && ewait(ep, ev, 8, 0) == 0 && wr64(wfd, 1) == -EAGAIN, "epoll: EPOLLOUT on an eventfd until it's full");
    ctl(ep, EPOLL_CTL_DEL, wfd, 0, 0);

    // Another process writing wakes a wait with no timeout.
    child = fork();
    if (child == 0) {
        sleep_ms(30);
        sys(SYS_write, pipe_fds[1], (i64)"y", 1, 0, 0, 0);
        quit(0);
    }
    t0 = now_ms();
    n = ewait(ep, ev, 8, -1);
    took = now_ms() - t0;
    check(n == 1 && ev[0].data == 111 && took >= 20 && wait_for(child) == 0, "epoll: a wait with no timeout is woken by another process's write");
    sys(SYS_read, pipe_fds[0], (i64)&c, 1, 0, 0, 0);

    // A timer in an epoll, edge-triggered: its expiry has to say so itself.
    ctl(ep, EPOLL_CTL_ADD, tfd, EPOLLIN | EPOLLET, 777);
    arm(tfd, 30, 0);
    t0 = now_ms();
    n = ewait(ep, ev, 8, 1000);
    took = now_ms() - t0;
    check(n == 1 && ev[0].data == 777 && took >= 20 && took < 500, "epoll: an edge-triggered timerfd is reported when it expires");
    rd64(tfd);
    ctl(ep, EPOLL_CTL_DEL, tfd, 0, 0);

    // Closing a file's last descriptor takes it out.
    int pipe2_fds[2];
    sys(SYS_pipe2, (i64)pipe2_fds, 0, 0, 0, 0, 0);
    ctl(ep, EPOLL_CTL_ADD, pipe2_fds[0], EPOLLIN, 888);
    sys(SYS_close, pipe2_fds[1], 0, 0, 0, 0, 0);
    n = ewait(ep, ev, 8, 0);
    int hup = n == 1 && ev[0].data == 888 && (ev[0].events & EPOLLHUP);
    sys(SYS_close, pipe2_fds[0], 0, 0, 0, 0, 0);
    check(hup && ewait(ep, ev, 8, 0) == 0, "epoll: a pipe with no writer is EPOLLHUP; once closed, it's gone");

    // A directory can't be added.
    int dir = sys(SYS_openat, AT_FDCWD, (i64)"/", O_RDONLY, 0, 0, 0);
    check(dir >= 0 && ctl(ep, EPOLL_CTL_ADD, dir, EPOLLIN, 0) == -EPERM, "epoll: a directory is EPERM");

    // An epoll in an epoll, and polled.
    int outer = sys(SYS_epoll_create1, 0, 0, 0, 0, 0, 0);
    ctl(outer, EPOLL_CTL_ADD, ep, EPOLLIN, 999);
    int quiet = ewait(outer, ev, 8, 0) == 0 && poll1(ep, 0, &revents) == 0;
    sys(SYS_write, pipe_fds[1], (i64)"z", 1, 0, 0, 0);
    n = ewait(outer, ev, 8, 0);
    check(quiet && n == 1 && ev[0].data == 999 && poll1(ep, 0, &revents) == 1 && revents == POLLIN, "epoll: one inside another, and polled, is ready when its files are");
    sys(SYS_read, pipe_fds[0], (i64)&c, 1, 0, 0, 0);

    // epoll_pwait2's timeout is a timespec.
    i64 ts[2] = {0, 30000000};
    t0 = now_ms();
    n = sys(SYS_epoll_pwait2, ep, (i64)ev, 8, (i64)ts, 0, 8);
    took = now_ms() - t0;
    check(n == 0 && took >= 20, "epoll_pwait2: times out after its timespec");

#if !defined(__aarch64__)
    // x86-64's old forms.
    int old = sys(SYS_epoll_create, 1, 0, 0, 0, 0, 0);
    int oldfd = sys(SYS_eventfd, 7, 0, 0, 0, 0, 0);
    ctl(old, EPOLL_CTL_ADD, oldfd, EPOLLIN, 1234);
    n = sys(SYS_epoll_wait, old, (i64)ev, 8, 0, 0, 0);
    check(old >= 0 && n == 1 && ev[0].data == 1234 && rd64(oldfd) == 7 && sys(SYS_epoll_create, 0, 0, 0, 0, 0, 0) == -EINVAL,
          "x86-64's epoll_create, epoll_wait and eventfd");
#endif

    // signalfd: a blocked signal, read from a file, woken in poll.
    u64 usr1 = 1UL << (SIGUSR1 - 1);
    sys(SYS_rt_sigprocmask, SIG_BLOCK, (i64)&usr1, 0, 8, 0, 0);
    int sfd = sys(SYS_signalfd4, -1, (i64)&usr1, 8, 0, 0, 0);
    i64 parent = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    child = fork();
    if (child == 0) {
        sleep_ms(30);
        sys(SYS_kill, parent, SIGUSR1, 0, 0, 0, 0);
        quit(0);
    }
    t0 = now_ms();
    n = poll1(sfd, 2000, &revents);
    took = now_ms() - t0;
    int info[32];
    i64 got_bytes = sys(SYS_read, sfd, (i64)info, sizeof info, 0, 0, 0);
    check(n == 1 && revents == POLLIN && took >= 20 && took < 1000 && got_bytes == 128 && info[0] == SIGUSR1 && wait_for(child) == 0,
          "signalfd: poll is woken by a signal from another process, then it's read");
    int sep = sys(SYS_epoll_create1, 0, 0, 0, 0, 0, 0);
    ctl(sep, EPOLL_CTL_ADD, sfd, EPOLLIN, 4321);
    sys(SYS_kill, parent, SIGUSR1, 0, 0, 0, 0);
    n = ewait(sep, ev, 8, 1000);
    check(n == 1 && ev[0].data == 4321 && sys(SYS_read, sfd, (i64)info, sizeof info, 0, 0, 0) == 128, "signalfd: in an epoll");

    // ppoll's mask: SIGUSR1 is blocked and pending; a mask letting it through ends the wait with its handler run, and
    // the mask is back after.
    u64 action[4] = {(u64)on_usr1, SA_SIGINFO | SA_RESTORER_FLAG, RESTORER, 0};
    sys(SYS_rt_sigaction, SIGUSR1, (i64)action, 0, 8, 0, 0);
    sys(SYS_kill, parent, SIGUSR1, 0, 0, 0, 0);
    u64 open_mask = 0;
    struct { int fd; short events, revents; } p = {pipe_fds[0], POLLIN, 0};
    i64 five_s[2] = {5, 0};
    t0 = now_ms();
    n = sys(SYS_ppoll, (i64)&p, 1, (i64)five_s, (i64)&open_mask, 8, 0);
    took = now_ms() - t0;
    u64 mask_after = 0;
    sys(SYS_rt_sigprocmask, SIG_BLOCK, 0, (i64)&mask_after, 8, 0, 0);
    check(n == -EINTR && got == 1 && took < 1000 && (mask_after & usr1), "ppoll: its mask lets a pending signal in, and the old mask comes back");
    check(sys(SYS_ppoll, (i64)&p, 1, (i64)five_s, (i64)&open_mask, 16, 0) == -EINVAL, "ppoll: a mask of the wrong size is EINVAL");

    // epoll_pwait's mask too.
    sys(SYS_kill, parent, SIGUSR1, 0, 0, 0, 0);
    n = sys(SYS_epoll_pwait, ep, (i64)ev, 8, 5000, (i64)&open_mask, 8);
    mask_after = 0;
    sys(SYS_rt_sigprocmask, SIG_BLOCK, 0, (i64)&mask_after, 8, 0, 0);
    check(n == -EINTR && got == 2 && (mask_after & usr1), "epoll_pwait: its mask lets a pending signal in, and the old mask comes back");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
