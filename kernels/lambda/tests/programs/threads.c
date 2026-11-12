// lambda's threads: clone'd threads sharing memory on several CPUs, futexes, thread-local storage, and joining by
// CLONE_CHILD_CLEARTID. Freestanding, for ARM64 and x86-64 alike.
// Build: clang --target=aarch64-linux-gnu (or x86_64-linux-gnu) -nostdlib -static -ffreestanding -fno-stack-protector
//        -fno-builtin -fuse-ld=lld -O2 threads.c -o threads

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit = 93, SYS_exit_group = 94, SYS_futex = 98, SYS_sched_yield = 124, SYS_getpid = 172,
       SYS_gettid = 178, SYS_clone = 220, SYS_mmap = 222, SYS_wait4 = 260 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ARCH "aarch64"
// The thread pointer: the first word of each thread's TLS block points at the block itself.
static void *tls_self(void) { void **tp; __asm__ volatile("mrs %0, tpidr_el0" : "=r"(tp)); return *tp; }
#else
enum { SYS_write = 1, SYS_sched_yield = 24, SYS_getpid = 39, SYS_clone = 56, SYS_exit = 60, SYS_mmap = 9,
       SYS_gettid = 186, SYS_futex = 202, SYS_exit_group = 231, SYS_wait4 = 61 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ARCH "x86_64"
static void *tls_self(void) { void *self; __asm__ volatile("mov %%fs:0, %0" : "=r"(self)); return self; }
#endif

#define CLONE_VM 0x100
#define CLONE_FS 0x200
#define CLONE_FILES 0x400
#define CLONE_SIGHAND 0x800
#define CLONE_THREAD 0x10000
#define CLONE_SYSVSEM 0x40000
#define CLONE_SETTLS 0x80000
#define CLONE_PARENT_SETTID 0x100000
#define CLONE_CHILD_CLEARTID 0x200000
#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_PRIVATE 128

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

static i64 futex(volatile u32 *at, int op, u32 value) { return sys(SYS_futex, (i64)at, op | FUTEX_PRIVATE, value, 0, 0, 0); }

// A new thread running fn(arg) on the stack whose top is `top`, with `tls` as its thread pointer. Its tid goes in
// *tid, which becomes 0 (with a futex wake) when it ends.
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

// Waits for a thread spawned with `tid` to end.
static void join(volatile u32 *tid) {
    for (;;) {
        u32 now = __atomic_load_n(tid, __ATOMIC_ACQUIRE);
        if (now == 0) return;
        futex(tid, FUTEX_WAIT, now);
    }
}

// A futex mutex (Drepper's): 0 free, 1 taken, 2 taken with waiters.
static void lock(volatile u32 *m) {
    u32 c = 0;
    if (__atomic_compare_exchange_n(m, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return;
    if (c != 2) c = __atomic_exchange_n(m, 2, __ATOMIC_ACQUIRE);
    while (c != 0) {
        futex(m, FUTEX_WAIT, 2);
        c = __atomic_exchange_n(m, 2, __ATOMIC_ACQUIRE);
    }
}
static void unlock(volatile u32 *m) {
    if (__atomic_exchange_n(m, 0, __ATOMIC_RELEASE) == 2) futex(m, FUTEX_WAKE, 1);
}

#define THREADS 4
#define ROUNDS 20000
#define STACK 65536

static char stacks[THREADS][STACK] __attribute__((aligned(16)));
static void *tls[THREADS][4] __attribute__((aligned(16)));
static volatile u32 tids[THREADS];

struct seen { i64 tid; void *tls; };
static struct seen seen[THREADS];
static volatile u32 mutex;
static u64 counter;
static volatile u64 atomic_counter;

static void who(void *arg) {
    i64 i = (i64)arg;
    seen[i].tid = sys(SYS_gettid, 0, 0, 0, 0, 0, 0);
    seen[i].tls = tls_self();
}

static void contend(void *arg) {
    (void)arg;
    for (int i = 0; i < ROUNDS; i++) {
        lock(&mutex);
        counter = counter + 1;
        unlock(&mutex);
        __atomic_fetch_add(&atomic_counter, 1, __ATOMIC_RELAXED);
    }
}

// Two threads hand a turn back and forth through one futex word.
#define PINGS 2000
static volatile u32 turn;
static volatile u32 pings;
static void pong(void *arg) {
    (void)arg;
    for (int i = 0; i < PINGS; i++) {
        while (__atomic_load_n(&turn, __ATOMIC_ACQUIRE) != 1) futex(&turn, FUTEX_WAIT, 0);
        pings = pings + 1;
        __atomic_store_n(&turn, 0, __ATOMIC_RELEASE);
        futex(&turn, FUTEX_WAKE, 1);
    }
}

// More threads and processes over a program's life than a machine has CPUs at once: each ended before the next.
#define LIVES 300
static volatile u32 lived;
static void live(void *arg) {
    (void)arg;
    __atomic_fetch_add(&lived, 1, __ATOMIC_RELAXED);
}

// More threads alive at once than a VM has CPUs (64 on an M1): all waiting together, so most have given their CPU up
// by the time they're let go. Each keeps values in registers across its wait (callee-saved ones, and a SIMD one the
// system call doesn't touch) and checks them after, with its thread pointer and its stack.
#define CROWD 160
#define CROWD_STACK 16384
static char crowd_stacks[CROWD][CROWD_STACK] __attribute__((aligned(16)));
static void *crowd_tls[CROWD][4] __attribute__((aligned(16)));
static volatile u32 crowd_tids[CROWD];
static volatile u32 arrived, gate, kept;
static void crowd(void *arg) {
    i64 i = (i64)arg;
    volatile u64 on_stack = (u64)i * 7 + 3;
#if defined(__aarch64__)
    register u64 a __asm__("x20") = (u64)i * 0x9e3779b97f4a7c15ull;
    register u64 b __asm__("x21") = ~(u64)i;
    register double d __asm__("d8") = (double)i * 1.5;
    __asm__ volatile("" : "+r"(a), "+r"(b), "+w"(d));
#else
    register u64 a __asm__("rbx") = (u64)i * 0x9e3779b97f4a7c15ull;
    register u64 b __asm__("r12") = ~(u64)i;
    register double d __asm__("xmm6") = (double)i * 1.5;
    __asm__ volatile("" : "+r"(a), "+r"(b), "+x"(d));
#endif
    __atomic_fetch_add(&arrived, 1, __ATOMIC_RELEASE);
    futex(&arrived, FUTEX_WAKE, 1);
    while (__atomic_load_n(&gate, __ATOMIC_ACQUIRE) == 0) futex(&gate, FUTEX_WAIT, 0);
#if defined(__aarch64__)
    __asm__ volatile("" : "+r"(a), "+r"(b), "+w"(d));
#else
    __asm__ volatile("" : "+r"(a), "+r"(b), "+x"(d));
#endif
    int ok = a == (u64)i * 0x9e3779b97f4a7c15ull && b == ~(u64)i && d == (double)i * 1.5 && on_stack == (u64)i * 7 + 3 &&
             tls_self() == &crowd_tls[i][0];
    if (ok) __atomic_fetch_add(&kept, 1, __ATOMIC_RELAXED);
}

static void run(void (*fn)(void *), int n) {
    for (int i = 0; i < n; i++) {
        tls[i][0] = &tls[i][0];
        tids[i] = 0;
        spawn(fn, (void *)(i64)i, stacks[i] + STACK, &tls[i][0], &tids[i]);
    }
    for (int i = 0; i < n; i++) join(&tids[i]);
}

void cmain(i64 *sp) {
    (void)sp;
    say("threads on " ARCH ", lambda\n");
    i64 pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    run(who, THREADS);
    int distinct = 1, own_tls = 1;
    for (int i = 0; i < THREADS; i++) {
        if (seen[i].tid <= 0 || seen[i].tid == pid) distinct = 0;
        for (int j = 0; j < i; j++) if (seen[i].tid == seen[j].tid) distinct = 0;
        if (seen[i].tls != &tls[i][0]) own_tls = 0;
    }
    check(distinct, "four threads, each with a tid of its own, none the process's");
    check(own_tls, "each thread's own thread pointer (CLONE_SETTLS)");
    int cleared = 1;
    for (int i = 0; i < THREADS; i++) if (tids[i] != 0) cleared = 0;
    check(cleared, "joined: each tid word cleared and woken (CLONE_CHILD_CLEARTID)");

    run(contend, THREADS);
    check(counter == (u64)THREADS * ROUNDS, "a futex mutex contended by four threads, 20,000 rounds each");
    check(atomic_counter == (u64)THREADS * ROUNDS, "an atomic add from four threads");

    tls[0][0] = &tls[0][0];
    tids[0] = 0;
    spawn(pong, 0, stacks[0] + STACK, &tls[0][0], &tids[0]);
    for (int i = 0; i < PINGS; i++) {
        __atomic_store_n(&turn, 1, __ATOMIC_RELEASE);
        futex(&turn, FUTEX_WAKE, 1);
        while (__atomic_load_n(&turn, __ATOMIC_ACQUIRE) != 0) futex(&turn, FUTEX_WAIT, 1);
    }
    join(&tids[0]);
    check(pings == PINGS, "futex wait and wake, 2,000 turns between two threads");

    for (int i = 0; i < LIVES; i++) {
        tls[0][0] = &tls[0][0];
        tids[0] = 0;
        if (spawn(live, 0, stacks[0] + STACK, &tls[0][0], &tids[0]) <= 0) break;
        join(&tids[0]);
    }
    check(lived == LIVES, "300 threads, one after another: more than there are CPUs at once");
    int made = 0;
    for (int i = 0; i < CROWD; i++) {
        crowd_tls[i][0] = &crowd_tls[i][0];
        if (spawn(crowd, (void *)(i64)i, crowd_stacks[i] + CROWD_STACK, &crowd_tls[i][0], &crowd_tids[i]) <= 0) break;
        made++;
    }
    for (;;) {
        u32 now = __atomic_load_n(&arrived, __ATOMIC_ACQUIRE);
        if (now >= (u32)made) break;
        futex(&arrived, FUTEX_WAIT, now);
    }
    __atomic_store_n(&gate, 1, __ATOMIC_RELEASE);
    futex(&gate, FUTEX_WAKE, 0x7fffffff);
    for (int i = 0; i < made; i++) join(&crowd_tids[i]);
    check(made == CROWD, "160 threads alive at once, all waiting: more than a VM has CPUs");
    check(kept == CROWD, "each with its registers, SIMD, stack and thread pointer as it left them, after its wait");
    int forked = 0;
    for (int i = 0; i < LIVES; i++) {
        i64 child = sys(SYS_clone, 17 /* SIGCHLD */, 0, 0, 0, 0, 0);
        if (child == 0) sys(SYS_exit_group, i & 0x7f, 0, 0, 0, 0, 0);
        if (child < 0) break;
        int status = -1;
        if (sys(SYS_wait4, child, (i64)&status, 0, 0, 0, 0) == child && status == (i & 0x7f) << 8) forked++;
    }
    check(forked == LIVES, "300 processes forked and waited for, one after another");

    check(sys(SYS_getpid, 0, 0, 0, 0, 0, 0) == pid, "still one process after its threads ended");
    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
