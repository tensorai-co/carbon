// IPC and synchronisation, for lambda: SysV message queues (types, MSG_EXCEPT, MSG_NOERROR, E2BIG, IPC_STAT, a
// receiver waiting in a child, EIDRM when the queue goes), SysV semaphores (SETVAL/GETALL/SETALL, IPC_NOWAIT and
// semtimedop's timeout, a waiter counted by GETNCNT and let go, SEM_UNDO undone when a process ends, IPC_STAT),
// POSIX message queues (priorities, sizes, O_NONBLOCK, a timeout, poll, the status read, mq_notify's signal, a
// receiver waiting in a child, mq_unlink) and futex2 (futex_wait's checks and timeout, futex_waitv across a fork
// giving which futex woke it, futex_requeue). Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef short i16;

#if defined(__aarch64__)
enum { SYS_close = 57, SYS_read = 63, SYS_write = 64, SYS_ppoll = 73, SYS_exit_group = 94, SYS_nanosleep = 101,
       SYS_clock_gettime = 113, SYS_rt_sigprocmask = 135, SYS_rt_sigtimedwait = 137, SYS_getpid = 172,
       SYS_mq_open = 180, SYS_mq_unlink = 181, SYS_mq_timedsend = 182, SYS_mq_timedreceive = 183, SYS_mq_notify = 184,
       SYS_mq_getsetattr = 185, SYS_msgget = 186, SYS_msgctl = 187, SYS_msgrcv = 188, SYS_msgsnd = 189,
       SYS_semget = 190, SYS_semctl = 191, SYS_semtimedop = 192, SYS_semop = 193, SYS_clone = 220, SYS_mmap = 222,
       SYS_wait4 = 260, SYS_futex_waitv = 449, SYS_futex_wake = 454, SYS_futex_wait = 455, SYS_futex_requeue = 456 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
// Where semid64_ds has sem_nsems: x86-64 has a word after each time.
#define SEM_NSEMS 64
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_mmap = 9, SYS_rt_sigprocmask = 14, SYS_nanosleep = 35,
       SYS_getpid = 39, SYS_clone = 56, SYS_wait4 = 61, SYS_semget = 64, SYS_semop = 65, SYS_semctl = 66,
       SYS_msgget = 68, SYS_msgsnd = 69, SYS_msgrcv = 70, SYS_msgctl = 71, SYS_rt_sigtimedwait = 128,
       SYS_semtimedop = 220, SYS_clock_gettime = 228, SYS_exit_group = 231, SYS_mq_open = 240, SYS_mq_unlink = 241,
       SYS_mq_timedsend = 242, SYS_mq_timedreceive = 243, SYS_mq_notify = 244, SYS_mq_getsetattr = 245,
       SYS_ppoll = 271, SYS_futex_waitv = 449, SYS_futex_wake = 454, SYS_futex_wait = 455, SYS_futex_requeue = 456 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define SEM_NSEMS 80
#endif

#define O_RDWR 2
#define O_CREAT 0100
#define O_EXCL 0200
#define O_NONBLOCK 04000
#define PROT_READ 1
#define PROT_WRITE 2
#define MAP_SHARED 1
#define MAP_ANONYMOUS 0x20
#define ENOENT 2
#define E2BIG 7
#define EAGAIN 11
#define EACCES 13
#define EBUSY 16
#define EEXIST 17
#define EINVAL 22
#define EFBIG 27
#define ERANGE 34
#define ENOMSG 42
#define EIDRM 43
#define EMSGSIZE 90
#define ETIMEDOUT 110
#define SIGCHLD 17
#define SIGUSR1 10
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_STAT 2
#define MSG_NOERROR 010000
#define MSG_EXCEPT 020000
#define SEM_UNDO 0x1000
#define GETPID 11
#define GETVAL 12
#define GETALL 13
#define GETNCNT 14
#define SETVAL 16
#define SETALL 17
#define POLLIN 1
#define POLLOUT 4
#define SIGEV_SIGNAL 0
#define SI_MESGQ (-3)
#define FUTEX2_SIZE_U32 2
#define FUTEX2_PRIVATE 128

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
// Waits for a child, giving its exit code.
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return (status >> 8) & 0xff; }
static void nap(u64 ms) { u64 ts[2] = {0, ms * 1000000}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static u64 now_ns(int clock) { u64 ts[2]; sys(SYS_clock_gettime, clock, (i64)ts, 0, 0, 0, 0); return ts[0] * 1000000000 + ts[1]; }
static void at_ns(u64 *ts, u64 ns) { ts[0] = ns / 1000000000; ts[1] = ns % 1000000000; }

struct msg { i64 type; char text[32]; };
struct sembuf { u16 num; i16 op; i16 flags; };
struct waitv { u64 val; u64 addr; u32 flags; u32 reserved; };

static int msgsend(int q, i64 type, const char *text) {
    struct msg m = {type, {0}};
    u64 n = len(text);
    for (u64 i = 0; i < n; i++) m.text[i] = text[i];
    return sys(SYS_msgsnd, q, (i64)&m, n, IPC_NOWAIT, 0, 0);
}
static int semop1(int set, u16 num, i16 op, i16 flags) {
    struct sembuf b = {num, op, flags};
    return sys(SYS_semop, set, (i64)&b, 1, 0, 0, 0);
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[128];
    int pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    // A page shared with children: flags they raise, and futexes.
    volatile u32 *shared = (volatile u32 *)sys(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);

    // SysV message queues.
    int q = sys(SYS_msgget, IPC_PRIVATE, 0600, 0, 0, 0, 0);
    int named = sys(SYS_msgget, 0x1234, IPC_CREAT | IPC_EXCL | 0600, 0, 0, 0, 0);
    check(q >= 0 && named >= 0 && named != q && sys(SYS_msgget, 0x1234, IPC_CREAT | IPC_EXCL | 0600, 0, 0, 0, 0) == -EEXIST &&
          sys(SYS_msgget, 0x1234, 0, 0, 0, 0, 0) == named && sys(SYS_msgget, 0x4321, 0, 0, 0, 0, 0) == -ENOENT,
          "msgget: private, by key, IPC_EXCL, found again, ENOENT");
    msgsend(q, 1, "one");
    msgsend(q, 2, "two");
    msgsend(q, 1, "three");
    msgsend(q, 3, "four");
    struct msg m;
    i64 n = sys(SYS_msgrcv, q, (i64)&m, 32, 2, IPC_NOWAIT, 0);
    check(n == 3 && m.type == 2 && same(m.text, "two", 3), "msgrcv by type");
    n = sys(SYS_msgrcv, q, (i64)&m, 32, 0, IPC_NOWAIT, 0);
    check(n == 3 && m.type == 1 && same(m.text, "one", 3), "msgrcv 0: the oldest");
    n = sys(SYS_msgrcv, q, (i64)&m, 32, 1, IPC_NOWAIT | MSG_EXCEPT, 0);
    check(n == 4 && m.type == 3 && same(m.text, "four", 4), "MSG_EXCEPT: the first not of a type");
    check(sys(SYS_msgrcv, q, (i64)&m, 2, -5, IPC_NOWAIT, 0) == -E2BIG, "too long for the buffer: E2BIG, and left there");
    n = sys(SYS_msgrcv, q, (i64)&m, 2, -5, IPC_NOWAIT | MSG_NOERROR, 0);
    check(n == 2 && m.type == 1 && same(m.text, "th", 2), "MSG_NOERROR cuts it; a negative type takes the lowest up to it");
    check(sys(SYS_msgrcv, q, (i64)&m, 32, 0, IPC_NOWAIT, 0) == -ENOMSG && msgsend(q, 0, "x") == -EINVAL, "an empty queue: ENOMSG; type 0: EINVAL");
    msgsend(q, 7, "stat");
    u64 ds[15];
    check(sys(SYS_msgctl, q, IPC_STAT, (i64)ds, 0, 0, 0) == 0 && ds[9] == 4 && ds[10] == 1 && ds[11] == 16384 && (int)ds[12] == pid &&
          (((u32 *)ds)[5] & 0777) == 0600, "IPC_STAT: bytes, messages, limit, last sender, mode");
    sys(SYS_msgrcv, q, (i64)&m, 32, 0, IPC_NOWAIT, 0);
    shared[0] = 0;
    i64 child = fork();
    if (child == 0) {
        struct msg got;
        shared[0] = 1;
        i64 r = sys(SYS_msgrcv, q, (i64)&got, 32, 9, 0, 0);
        quit(r == 5 && got.type == 9 && same(got.text, "later", 5) ? 0 : 1);
    }
    while (!shared[0]) nap(1);
    nap(30);
    msgsend(q, 9, "later");
    check(reap(child) == 0, "a child waits in msgrcv for a message of its type");
    shared[0] = 0;
    child = fork();
    if (child == 0) {
        struct msg got;
        shared[0] = 1;
        quit(sys(SYS_msgrcv, q, (i64)&got, 32, 0, 0, 0) == -EIDRM ? 0 : 1);
    }
    while (!shared[0]) nap(1);
    nap(30);
    check(sys(SYS_msgctl, q, IPC_RMID, 0, 0, 0, 0) == 0 && reap(child) == 0 && msgsend(q, 1, "x") == -EINVAL,
          "IPC_RMID: a waiting receiver gets EIDRM; the queue's gone");
    sys(SYS_msgctl, named, IPC_RMID, 0, 0, 0, 0);

    // SysV semaphores.
    int set = sys(SYS_semget, IPC_PRIVATE, 2, 0600, 0, 0, 0);
    check(set >= 0 && sys(SYS_semget, IPC_PRIVATE, 0, 0600, 0, 0, 0) == -EINVAL, "semget; a set of none is EINVAL");
    check(sys(SYS_semctl, set, 0, SETVAL, 1, 0, 0) == 0 && sys(SYS_semctl, set, 0, GETVAL, 0, 0, 0) == 1 && semop1(set, 0, -1, 0) == 0 &&
          sys(SYS_semctl, set, 0, GETVAL, 0, 0, 0) == 0 && (int)sys(SYS_semctl, set, 0, GETPID, 0, 0, 0) == pid,
          "SETVAL, semop takes it, GETVAL, GETPID");
    check(semop1(set, 0, -1, IPC_NOWAIT) == -EAGAIN && semop1(set, 5, 1, 0) == -EFBIG && sys(SYS_semctl, set, 0, SETVAL, 40000, 0, 0) == -ERANGE,
          "IPC_NOWAIT: EAGAIN; no such semaphore: EFBIG; past SEMVMX: ERANGE");
    u64 ts[2] = {0, 30000000};
    struct sembuf take = {0, -1, 0};
    u64 before = now_ns(1);
    check(sys(SYS_semtimedop, set, (i64)&take, 1, (i64)ts, 0, 0) == -EAGAIN && now_ns(1) - before >= 25000000, "semtimedop times out after its 30 ms");
    u16 all[2] = {3, 4}, back[2] = {0, 0};
    check(sys(SYS_semctl, set, 0, SETALL, (i64)all, 0, 0) == 0 && sys(SYS_semctl, set, 0, GETALL, (i64)back, 0, 0) == 0 && back[0] == 3 && back[1] == 4,
          "SETALL and GETALL");
    struct sembuf both[2] = {{0, -3, 0}, {1, -5, IPC_NOWAIT}};
    check(sys(SYS_semop, set, (i64)both, 2, 0, 0, 0) == -EAGAIN && sys(SYS_semctl, set, 0, GETVAL, 0, 0, 0) == 3, "all or nothing: one can't, so neither happens");
    sys(SYS_semctl, set, 0, SETVAL, 0, 0, 0);
    child = fork();
    if (child == 0) quit(semop1(set, 0, -2, 0) == 0 ? 0 : 1);
    while (sys(SYS_semctl, set, 0, GETNCNT, 0, 0, 0) != 1) nap(1);
    semop1(set, 0, 2, 0);
    check(reap(child) == 0 && (int)sys(SYS_semctl, set, 0, GETPID, 0, 0, 0) == child && sys(SYS_semctl, set, 0, GETNCNT, 0, 0, 0) == 0,
          "a child waits, GETNCNT counts it, and it's let go");
    sys(SYS_semctl, set, 1, SETVAL, 0, 0, 0);
    child = fork();
    if (child == 0) quit(semop1(set, 1, 5, SEM_UNDO) == 0 && semop1(set, 1, -2, SEM_UNDO) == 0 ? 0 : 1);
    check(reap(child) == 0 && sys(SYS_semctl, set, 1, GETVAL, 0, 0, 0) == 0, "SEM_UNDO: what a process did is undone when it ends");
    u64 sds[13];
    check(sys(SYS_semctl, set, 0, IPC_STAT, (i64)sds, 0, 0) == 0 && *(u64 *)((char *)sds + SEM_NSEMS) == 2, "IPC_STAT: the set's size");
    check(sys(SYS_semctl, set, 0, IPC_RMID, 0, 0, 0) == 0 && sys(SYS_semctl, set, 0, GETVAL, 0, 0, 0) == -EINVAL, "IPC_RMID");

    // POSIX message queues.
    i64 attr[8] = {0, 4, 64, 0, 0, 0, 0, 0};
    int mq = sys(SYS_mq_open, (i64)"lambda-test", O_RDWR | O_CREAT | O_EXCL, 0600, (i64)attr, 0, 0);
    check(mq >= 0 && sys(SYS_mq_open, (i64)"lambda-test", O_RDWR | O_CREAT | O_EXCL, 0600, 0, 0, 0) == -EEXIST &&
          sys(SYS_mq_open, (i64)"not-there", O_RDWR, 0, 0, 0, 0) == -ENOENT && sys(SYS_mq_open, (i64)"a/b", O_RDWR | O_CREAT, 0600, 0, 0, 0) == -EACCES,
          "mq_open: made, EEXIST, ENOENT, a slash EACCES");
    sys(SYS_mq_timedsend, mq, (i64)"low", 3, 1, 0, 0);
    sys(SYS_mq_timedsend, mq, (i64)"high", 4, 5, 0, 0);
    sys(SYS_mq_timedsend, mq, (i64)"low2", 4, 1, 0, 0);
    u32 prio = 0;
    i64 got = sys(SYS_mq_timedreceive, mq, (i64)buf, 64, (i64)&prio, 0, 0);
    check(got == 4 && prio == 5 && same(buf, "high", 4), "highest priority first");
    got = sys(SYS_mq_timedreceive, mq, (i64)buf, 64, (i64)&prio, 0, 0);
    check(got == 3 && prio == 1 && same(buf, "low", 3), "then the oldest of a priority");
    check(sys(SYS_mq_timedreceive, mq, (i64)buf, 10, 0, 0, 0) == -EMSGSIZE && sys(SYS_mq_timedsend, mq, (i64)buf, 65, 0, 0, 0) == -EMSGSIZE,
          "a buffer smaller than the queue's messages, a message bigger: EMSGSIZE");
    i64 old[8], nb[8] = {O_NONBLOCK, 0, 0, 0, 0, 0, 0, 0};
    check(sys(SYS_mq_getsetattr, mq, (i64)nb, (i64)old, 0, 0, 0) == 0 && old[0] == 0 && old[1] == 4 && old[2] == 64 && old[3] == 1,
          "mq_getsetattr: sizes and how many it holds");
    sys(SYS_mq_timedreceive, mq, (i64)buf, 64, 0, 0, 0);
    check(sys(SYS_mq_timedreceive, mq, (i64)buf, 64, 0, 0, 0) == -EAGAIN, "O_NONBLOCK and empty: EAGAIN");
    for (int i = 0; i < 4; i++) sys(SYS_mq_timedsend, mq, (i64)"x", 1, 0, 0, 0);
    check(sys(SYS_mq_timedsend, mq, (i64)"x", 1, 0, 0, 0) == -EAGAIN, "O_NONBLOCK and full: EAGAIN");
    nb[0] = 0;
    sys(SYS_mq_getsetattr, mq, (i64)nb, 0, 0, 0, 0);
    u64 when[2];
    at_ns(when, now_ns(0) + 20000000);
    check(sys(SYS_mq_timedsend, mq, (i64)"x", 1, 0, (i64)when, 0) == -ETIMEDOUT, "full, with a deadline: ETIMEDOUT");
    struct { int fd; short events, revents; } pfd = {mq, POLLIN | POLLOUT, 0};
    u64 zero[2] = {0, 0};
    check(sys(SYS_ppoll, (i64)&pfd, 1, (i64)zero, 0, 8, 0) == 1 && pfd.revents == POLLIN, "poll: readable, and full so not writable");
    n = sys(SYS_read, mq, (i64)buf, sizeof buf, 0, 0, 0);
    check(n > 6 && same(buf, "QSIZE:4 ", 8), "read gives the queue's status");
    for (int i = 0; i < 4; i++) sys(SYS_mq_timedreceive, mq, (i64)buf, 64, 0, 0, 0);
    u64 mask = 1ul << (SIGUSR1 - 1);
    sys(SYS_rt_sigprocmask, 0, (i64)&mask, 0, 8, 0, 0);
    struct { u64 value; int signo, notify; char pad[48]; } ev = {0x5eed, SIGUSR1, SIGEV_SIGNAL, {0}};
    check(sys(SYS_mq_notify, mq, (i64)&ev, 0, 0, 0, 0) == 0 && sys(SYS_mq_notify, mq, (i64)&ev, 0, 0, 0, 0) == -EBUSY, "mq_notify; a second: EBUSY");
    sys(SYS_mq_timedsend, mq, (i64)"ping", 4, 0, 0, 0);
    int info[32];
    int signal = sys(SYS_rt_sigtimedwait, (i64)&mask, (i64)info, (i64)zero, 8, 0, 0);
    check(signal == SIGUSR1 && info[2] == SI_MESGQ && info[4] == pid && *(u64 *)&info[6] == 0x5eed, "a message to an empty queue: the signal, SI_MESGQ, its value");
    check(sys(SYS_mq_notify, mq, (i64)&ev, 0, 0, 0, 0) == 0 && sys(SYS_mq_notify, mq, 0, 0, 0, 0, 0) == 0, "told once, then free; unregistered");
    sys(SYS_mq_timedreceive, mq, (i64)buf, 64, 0, 0, 0);
    child = fork();
    if (child == 0) {
        char in[64];
        u32 p = 0;
        quit(sys(SYS_mq_timedreceive, mq, (i64)in, 64, (i64)&p, 0, 0) == 5 && p == 3 && same(in, "hello", 5) ? 0 : 1);
    }
    nap(30);
    sys(SYS_mq_timedsend, mq, (i64)"hello", 5, 3, 0, 0);
    check(reap(child) == 0, "a child waits in mq_timedreceive");
    sys(SYS_close, mq, 0, 0, 0, 0, 0);
    check(sys(SYS_mq_unlink, (i64)"lambda-test", 0, 0, 0, 0, 0) == 0 && sys(SYS_mq_open, (i64)"lambda-test", O_RDWR, 0, 0, 0, 0) == -ENOENT &&
          sys(SYS_mq_unlink, (i64)"lambda-test", 0, 0, 0, 0, 0) == -ENOENT, "mq_unlink");

    // futex2.
    volatile u32 *a = &shared[4], *b = &shared[8];
    *a = 0;
    *b = 0;
    check(sys(SYS_futex_wait, (i64)a, 1, ~0u, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0, 0) == -EAGAIN &&
          sys(SYS_futex_wait, (i64)a, 0, ~0u, 3, 0, 0) == -EINVAL && sys(SYS_futex_wait, (i64)a, 0, 0, FUTEX2_SIZE_U32, 0, 0) == -EINVAL &&
          sys(SYS_futex_wake, (i64)a, ~0u, 1, FUTEX2_SIZE_U32, 0, 0) == 0, "futex_wait: not what's there EAGAIN, bad size or mask EINVAL; futex_wake of none");
    at_ns(when, now_ns(1) + 20000000);
    check(sys(SYS_futex_wait, (i64)a, 0, ~0u, FUTEX2_SIZE_U32, (i64)when, 1) == -ETIMEDOUT, "futex_wait until a monotonic deadline");
    struct waitv wv[2] = {{0, (u64)a, FUTEX2_SIZE_U32, 0}, {0, (u64)b, FUTEX2_SIZE_U32, 0}};
    *b = 1;
    check(sys(SYS_futex_waitv, (i64)wv, 2, 0, 0, 0, 0) == -EAGAIN && sys(SYS_futex_waitv, (i64)wv, 0, 0, 0, 0, 0) == -EINVAL, "futex_waitv: one changed EAGAIN; none EINVAL");
    *b = 0;
    child = fork();
    if (child == 0) quit((int)sys(SYS_futex_waitv, (i64)wv, 2, 0, 0, 0, 0));
    // Woken on the second until it's waiting there; a wake needs no change of value.
    i64 woken = 0;
    for (int i = 0; i < 2000 && woken == 0; i++) { woken = sys(SYS_futex_wake, (i64)b, ~0u, 1, FUTEX2_SIZE_U32, 0, 0); if (!woken) nap(1); }
    check(woken == 1 && reap(child) == 1, "futex_waitv across a fork: woken by the second, which it gives");
    *a = 0;
    *b = 0;
    child = fork();
    if (child == 0) quit(sys(SYS_futex_wait, (i64)a, 0, ~0u, FUTEX2_SIZE_U32, 0, 0) == 0 ? 0 : 1);
    i64 moved = 0;
    for (int i = 0; i < 2000 && moved == 0; i++) { moved = sys(SYS_futex_requeue, (i64)wv, 0, 0, 1, 0, 0); if (!moved) nap(1); }
    check(moved == 1 && sys(SYS_futex_wake, (i64)a, ~0u, 1, FUTEX2_SIZE_U32, 0, 0) == 0 && sys(SYS_futex_wake, (i64)b, ~0u, 1, FUTEX2_SIZE_U32, 0, 0) == 1 &&
          reap(child) == 0, "futex_requeue moves a waiter from one futex to the other");
    wv[0].val = 1;
    check(sys(SYS_futex_requeue, (i64)wv, 0, 1, 1, 0, 0) == -EAGAIN, "futex_requeue: the first not as expected, EAGAIN");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
