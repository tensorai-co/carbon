// Programs that write their own code, as JITs do: code written to pages and run, then rewritten, re-protected,
// remapped, rewritten by another thread while it runs, and a JIT's worth of code churned through. Freestanding, for
// ARM64 and x86-64 alike. On Apple silicon the x86-64 build checks that exec's translator forgets what it translated
// when the code changes; run it with CARBON_EXEC_CODE_CACHE=65536 to make the code cache move between its halves too.
// Build: clang --target=aarch64-linux-gnu (or x86_64-linux-gnu) -nostdlib -static -ffreestanding -fno-stack-protector
//        -fno-builtin -fuse-ld=lld -O2 code.c -o code

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit = 93, SYS_exit_group = 94, SYS_futex = 98, SYS_clone = 220, SYS_munmap = 215,
       SYS_mmap = 222, SYS_mprotect = 226 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ARCH "aarch64"
#define SLOT 8
// `movz w0, #k; ret`
static void emit_const(u8 *at, u32 k) {
    u32 *w = (u32 *)at;
    w[0] = 0x52800000 | (k & 0xFFFF) << 5;
    w[1] = 0xd65f03c0;
}
// `stp x29, x30, [sp, #-16]!; bl g; add w0, w0, #1; ldp x29, x30, [sp], #16; ret`
static void emit_call_plus_one(u8 *at, u8 *g) {
    u32 *w = (u32 *)at;
    w[0] = 0xa9bf7bfd;
    w[1] = 0x94000000 | (u32)(((i64)g - (i64)(at + 4)) >> 2 & 0x03FFFFFF);
    w[2] = 0x11000400;
    w[3] = 0xa8c17bfd;
    w[4] = 0xd65f03c0;
}
// Code written as data reaches instruction fetch: clean to where both meet, then invalidate the instruction cache.
static void sync_code(u8 *start, u64 n) {
    for (u64 at = (u64)start & ~63ul; at < (u64)start + n; at += 64) __asm__ volatile("dc cvau, %0" ::"r"(at) : "memory");
    __asm__ volatile("dsb ish" ::: "memory");
    for (u64 at = (u64)start & ~63ul; at < (u64)start + n; at += 64) __asm__ volatile("ic ivau, %0" ::"r"(at) : "memory");
    __asm__ volatile("dsb ish\n isb" ::: "memory");
}
static void fetch_fence(void) { __asm__ volatile("isb" ::: "memory"); }
#else
enum { SYS_write = 1, SYS_mmap = 9, SYS_mprotect = 10, SYS_munmap = 11, SYS_clone = 56, SYS_exit = 60, SYS_futex = 202,
       SYS_exit_group = 231 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ARCH "x86_64"
#define SLOT 8
// `mov eax, k; ret`
static void emit_const(u8 *at, u32 k) {
    at[0] = 0xb8;
    at[1] = (u8)k; at[2] = (u8)(k >> 8); at[3] = (u8)(k >> 16); at[4] = (u8)(k >> 24);
    at[5] = 0xc3;
}
// `call g; add eax, 1; ret`
static void emit_call_plus_one(u8 *at, u8 *g) {
    i64 rel = (i64)g - (i64)(at + 5);
    at[0] = 0xe8;
    at[1] = (u8)rel; at[2] = (u8)(rel >> 8); at[3] = (u8)(rel >> 16); at[4] = (u8)(rel >> 24);
    at[5] = 0x83; at[6] = 0xc0; at[7] = 0x01;
    at[8] = 0xc3;
}
// x86 sees its own stores to code; nothing to do.
static void sync_code(u8 *start, u64 n) { (void)start; (void)n; __asm__ volatile("" ::: "memory"); }
static void fetch_fence(void) { __asm__ volatile("" ::: "memory"); }
#endif

#define PROT_RW 3
#define PROT_RX 5
#define PROT_RWX 7
#define MAP_PRIVATE_ANON 0x22
#define MAP_FIXED 0x10
#define PAGE 4096

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}

typedef u32 (*fn)(void);
static u32 call(u8 *at) { fetch_fence(); return ((fn)(void *)at)(); }
static u8 *map(u64 size, int prot) { return (u8 *)sys(SYS_mmap, 0, size, prot, MAP_PRIVATE_ANON, -1, 0); }

// Threads, as threads.c makes them, without thread-local storage.
#define CLONE_THREADS (0x100 | 0x200 | 0x400 | 0x800 | 0x10000 | 0x40000 | 0x100000 | 0x200000)
static i64 spawn(void (*f)(void *), void *arg, char *top, volatile u32 *tid) {
    u64 *sp = (u64 *)top;
    *--sp = (u64)arg;
    *--sp = (u64)f;
#if defined(__aarch64__)
    register i64 x8 __asm__("x8") = SYS_clone, x0 __asm__("x0") = CLONE_THREADS, x1 __asm__("x1") = (i64)sp,
                 x2 __asm__("x2") = (i64)tid, x3 __asm__("x3") = 0, x4 __asm__("x4") = (i64)tid;
    __asm__ volatile("svc #0\n cbnz x0, 1f\n ldp x9, x0, [sp], #16\n blr x9\n mov x0, #0\n mov x8, #93\n svc #0\n 1:"
                     : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4) : "x9", "x30", "memory");
    return x0;
#else
    register i64 r10 __asm__("r10") = (i64)tid, r8 __asm__("r8") = 0;
    i64 ret;
    __asm__ volatile("syscall\n test %%rax, %%rax\n jnz 1f\n pop %%rax\n pop %%rdi\n call *%%rax\n"
                     " xor %%edi, %%edi\n mov $60, %%eax\n syscall\n hlt\n 1:"
                     : "=a"(ret) : "a"(SYS_clone), "D"((u64)CLONE_THREADS), "S"(sp), "d"(tid), "r"(r10), "r"(r8) : "rcx", "r11", "memory");
    return ret;
#endif
}
static void join(volatile u32 *tid) {
    for (;;) {
        u32 now = __atomic_load_n(tid, __ATOMIC_ACQUIRE);
        if (now == 0) return;
        sys(SYS_futex, (i64)tid, 128, now, 0, 0, 0);
    }
}

#define STACK 65536
static char stacks[2][STACK] __attribute__((aligned(16)));
static volatile u32 tids[2];

// Another thread rewrites a function this thread keeps calling.
static u8 *shared_fn;
static volatile u32 go;
static void rewriter(void *arg) {
    (void)arg;
    while (__atomic_load_n(&go, __ATOMIC_ACQUIRE) == 0) {}
    emit_const(shared_fn, 2222);
    sync_code(shared_fn, SLOT);
    __atomic_store_n(&go, 2, __ATOMIC_RELEASE);
}

// A thread that keeps running a function of its own while the main thread churns through code.
static u8 *hot_fn;
static volatile u32 stop_hot, hot_wrong;
static volatile u64 hot_calls;
static void hot(void *arg) {
    (void)arg;
    while (__atomic_load_n(&stop_hot, __ATOMIC_ACQUIRE) == 0) {
        if (call(hot_fn) != 7777) hot_wrong = hot_wrong + 1;
        hot_calls = hot_calls + 1;
    }
}

void cmain(i64 *sp) {
    (void)sp;
    say("code written at run time on " ARCH ", lambda\n");

    // 1. A function written to a page the program may write and run, then rewritten again and again.
    u8 *page = map(PAGE, PROT_RWX);
    emit_const(page, 41);
    sync_code(page, SLOT);
    check(call(page) == 41, "a function written to a writable, runnable page runs");
    int wrong = 0;
    for (u32 i = 0; i < 1000; i++) {
        emit_const(page, i);
        sync_code(page, SLOT);
        if (call(page) != i) wrong++;
    }
    check(wrong == 0, "rewritten 1,000 times, it returns each new value");

    // 2. A direct call into code that's rewritten: f calls g, g changes, f's result follows.
    u8 *f = page + 64, *g = page + 128;
    emit_const(g, 10);
    emit_call_plus_one(f, g);
    sync_code(page, PAGE);
    u32 sum = 0;
    for (int i = 0; i < 1000; i++) sum += call(f);
    emit_const(g, 20);
    sync_code(g, SLOT);
    check(sum == 11000 && call(f) == 21, "a direct call, run 1,000 times, into code that's then rewritten");

    // 3. W^X: written while writable, run while runnable, then rewritten the same way.
    u8 *wx = map(PAGE, PROT_RW);
    emit_const(wx, 5);
    sync_code(wx, SLOT);
    sys(SYS_mprotect, (i64)wx, PAGE, PROT_RX, 0, 0, 0);
    u32 first = call(wx);
    sys(SYS_mprotect, (i64)wx, PAGE, PROT_RW, 0, 0, 0);
    emit_const(wx, 6);
    sync_code(wx, SLOT);
    sys(SYS_mprotect, (i64)wx, PAGE, PROT_RX, 0, 0, 0);
    check(first == 5 && call(wx) == 6, "W^X: written, made runnable, run; made writable, rewritten, run again");

    // 4. New code mapped where old code ran.
    sys(SYS_munmap, (i64)wx, PAGE, 0, 0, 0, 0);
    u8 *again = (u8 *)sys(SYS_mmap, (i64)wx, PAGE, PROT_RWX, MAP_PRIVATE_ANON | MAP_FIXED, -1, 0);
    emit_const(again, 8);
    sync_code(again, SLOT);
    check(again == wx && call(again) == 8, "munmap, then new code mapped at the same address");

    // 5. Another thread rewrites a function while this one keeps calling it.
    shared_fn = map(PAGE, PROT_RWX);
    emit_const(shared_fn, 1111);
    sync_code(shared_fn, SLOT);
    spawn(rewriter, 0, stacks[0] + STACK, &tids[0]);
    u64 calls = 0, before = 0;
    u32 got = 0;
    for (;;) {
        got = call(shared_fn);
        calls++;
        if (got != 1111) break;
        if (calls == 1000) __atomic_store_n(&go, 1, __ATOMIC_RELEASE);
        if (__atomic_load_n(&go, __ATOMIC_ACQUIRE) == 2 && before++ > 10000000) break;
    }
    join(&tids[0]);
    check(got == 2222, "another thread rewrites a function this one keeps calling: the new code runs");

    // 6. A JIT's worth of code: 4,096 functions written, run and rewritten 16 times over, while another thread keeps
    //    running a function of its own.
    enum { FUNCTIONS = 4096, ROUNDS = 16 };
    u8 *jit = map(FUNCTIONS * SLOT, PROT_RWX);
    hot_fn = map(PAGE, PROT_RWX);
    emit_const(hot_fn, 7777);
    sync_code(hot_fn, SLOT);
    spawn(hot, 0, stacks[1] + STACK, &tids[1]);
    wrong = 0;
    for (u32 round = 0; round < ROUNDS; round++) {
        for (u32 i = 0; i < FUNCTIONS; i++) emit_const(jit + i * SLOT, round * 4096 + i);
        sync_code(jit, FUNCTIONS * SLOT);
        for (u32 i = 0; i < FUNCTIONS; i++) if (call(jit + i * SLOT) != round * 4096 + i) wrong++;
    }
    __atomic_store_n(&stop_hot, 1, __ATOMIC_RELEASE);
    join(&tids[1]);
    check(wrong == 0, "4,096 functions written and run, 16 times over, each giving its own value");
    check(hot_wrong == 0 && hot_calls > 0, "another thread running its own function meanwhile, right every time");

    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
