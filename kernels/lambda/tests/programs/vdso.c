// The vDSO, from a program with no libc: found through AT_SYSINFO_EHDR, its clock_gettime looked up the way a C
// library does (its dynamic section, hash table, symbols and version definitions), called for the monotonic and wall
// clocks once its clock page is ready and checked against the system call, never going backwards, falling through to
// the system call for a clock it doesn't keep itself, and quicker than the system call; `[vvar]` and `[vdso]` in
// /proc/self/maps.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit_group = 94, SYS_clock_gettime = 113, SYS_openat = 56, SYS_read = 63 };
static i64 sys(i64 n, i64 a, i64 b, i64 c) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define CLOCK_FN "__kernel_clock_gettime"
#define TIMEVAL_FN "__kernel_gettimeofday"
#define VERSION "LINUX_2.6.39"
#else
enum { SYS_write = 1, SYS_exit_group = 231, SYS_clock_gettime = 228, SYS_openat = 257, SYS_read = 0 };
static i64 sys(i64 n, i64 a, i64 b, i64 c) {
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define CLOCK_FN "__vdso_clock_gettime"
#define TIMEVAL_FN "__vdso_gettimeofday"
#define VERSION "LINUX_2.6"
#endif

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s)); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int same(const char *a, const char *b) { u64 i = 0; for (; a[i] && b[i]; i++) if (a[i] != b[i]) return 0; return a[i] == b[i]; }
static void say_num(u64 n) {
    char buf[24];
    int i = 23;
    buf[i] = 0;
    do { buf[--i] = (char)('0' + n % 10); n /= 10; } while (n);
    say(buf + i);
}

struct timespec { i64 sec, nsec; };
typedef i64 (*clock_fn)(i64, struct timespec *);
struct timeval { i64 sec, usec; };
typedef i64 (*timeval_fn)(struct timeval *, void *);

static u64 ns(struct timespec t) { return (u64)t.sec * 1000000000ul + (u64)t.nsec; }
static u64 sys_now(i64 clock) {
    struct timespec t;
    sys(SYS_clock_gettime, clock, (i64)&t, 0);
    return ns(t);
}

static u64 elf_hash(const char *name) {
    u64 h = 0, g;
    for (; *name; name++) {
        h = (h << 4) + (u8)*name;
        g = h & 0xf0000000;
        if (g) h ^= g >> 24;
        h &= ~g;
    }
    return h;
}

// A symbol of the vDSO's, of a version, as a C library finds it.
static void *lookup(u8 *base, const char *name, const char *version) {
    u64 phoff = *(u64 *)(base + 32);
    u16 phnum = *(u16 *)(base + 56);
    u64 *dyn = 0;
    for (u16 i = 0; i < phnum; i++) {
        u8 *ph = base + phoff + 56 * i;
        if (*(u32 *)ph == 2) dyn = (u64 *)(base + *(u64 *)(ph + 16));
    }
    if (!dyn) return 0;
    u32 *hash = 0;
    u8 *symtab = 0, *verdef = 0;
    const char *strtab = 0;
    u16 *versym = 0;
    for (; dyn[0]; dyn += 2) {
        if (dyn[0] == 4) hash = (u32 *)(base + dyn[1]);
        if (dyn[0] == 5) strtab = (const char *)(base + dyn[1]);
        if (dyn[0] == 6) symtab = base + dyn[1];
        if (dyn[0] == 0x6ffffff0) versym = (u16 *)(base + dyn[1]);
        if (dyn[0] == 0x6ffffffc) verdef = base + dyn[1];
    }
    if (!hash || !strtab || !symtab || !versym || !verdef) return 0;
    u64 want = elf_hash(version);
    for (u32 i = hash[2 + elf_hash(name) % hash[0]]; i; i = hash[2 + hash[0] + i]) {
        u8 *sym = symtab + 24 * i;
        if (!same(strtab + *(u32 *)sym, name) || *(u16 *)(sym + 6) == 0) continue;
        // Its version: the definition its versym names, by hash and name.
        u8 *d = verdef;
        for (;;) {
            if (*(u16 *)(d + 4) == (versym[i] & 0x7fff)) {
                u8 *aux = d + *(u32 *)(d + 12);
                if (*(u32 *)(d + 8) == want && same(strtab + *(u32 *)aux, version)) return base + *(u64 *)(sym + 8);
                break;
            }
            if (!*(u32 *)(d + 16)) break;
            d += *(u32 *)(d + 16);
        }
    }
    return 0;
}

// How long `n` calls of a clock take, in its own nanoseconds.
static u64 time_calls(clock_fn f, int n) {
    struct timespec t;
    u64 start = sys_now(1);
    for (int i = 0; i < n; i++) {
        if (f) f(1, &t);
        else sys(SYS_clock_gettime, 1, (i64)&t, 0);
    }
    return sys_now(1) - start;
}

void cmain(i64 *sp) {
    i64 argc = sp[0];
    i64 *p = sp + 1 + argc + 1;
    while (*p) p++;
    u64 *aux = (u64 *)(p + 1);
    u8 *base = 0;
    for (; aux[0]; aux += 2)
        if (aux[0] == 33) base = (u8 *)aux[1];
    check(base && base[0] == 0x7f && base[1] == 'E' && base[2] == 'L' && base[3] == 'F', "AT_SYSINFO_EHDR: the vDSO, an ELF image");

    clock_fn f = (clock_fn)lookup(base, CLOCK_FN, VERSION);
    check(f != 0, "its clock_gettime, found by its hash table and version as a C library looks it up");
    if (!f) {
        say("some checks failed\n");
        sys(SYS_exit_group, 1, 0, 0);
    }

    // Its first readings teach the kernel its counter (through the system call); then the clock page is ready.
    struct timespec t;
    for (int i = 0; i < 16; i++) f(1, &t);
    u64 *vvar_page = (u64 *)(base - 4096);
    check((vvar_page[1] & 0xffffffff) == 1 && vvar_page[5] != 0, "the clock page ([vvar], before it): ready after its first readings, with nanoseconds per tick");

    // Each clock it keeps itself agrees with the system call (within 5 ms).
    int agree = 1;
    for (i64 clock = 0; clock < 8; clock++) {
        if (clock == 2 || clock == 3) continue;
        u64 before = sys_now(clock);
        i64 r = f(clock, &t);
        u64 after = sys_now(clock);
        u64 v = ns(t);
        if (r != 0 || v + 5000000 < before || v > after + 5000000) agree = 0;
    }
    check(agree, "the wall and monotonic clocks (and their coarse, raw and boot forms) agree with the system call");

    // gettimeofday: the wall clock in microseconds, and with a time zone asked for, through the system call.
    timeval_fn g = (timeval_fn)lookup(base, TIMEVAL_FN, VERSION);
    struct timeval tv = {0, 0};
    i64 zone[2] = {-1, -1};
    u64 before = sys_now(0);
    i64 gr = g ? g(&tv, 0) : -1;
    u64 after = sys_now(0);
    u64 us = (u64)tv.sec * 1000000ul + (u64)tv.usec;
    int timeval_ok = gr == 0 && tv.usec >= 0 && tv.usec < 1000000 && us + 5000 >= before / 1000 && us <= after / 1000 + 5000;
    check(timeval_ok && g(&tv, zone) == 0 && zone[0] != -1, "gettimeofday: the wall clock in microseconds, and a time zone through the system call");

    // Never backwards, over many calls.
    u64 last = 0;
    int forward = 1;
    for (int i = 0; i < 200000; i++) {
        f(1, &t);
        if (ns(t) < last) forward = 0;
        last = ns(t);
    }
    check(forward, "the monotonic clock never goes backwards, over 200,000 readings");

    // A clock it doesn't keep: through to the system call (the process's CPU time), and one that isn't: EINVAL.
    struct timespec cpu = {0, 0};
    i64 r = f(2, &cpu);
    check(r == 0 && ns(cpu) > 0 && f(99, &t) == -22, "the CPU-time clock through to the system call, a clock that isn't one EINVAL");

    // Quicker than the system call.
    u64 slow = time_calls(0, 20000), fast = time_calls(f, 20000);
    say("        20,000 readings: ");
    say_num(slow / 1000);
    say(" us by system call, ");
    say_num(fast / 1000);
    say(" us through the vDSO\n");
    check(fast * 4 < slow, "at least four times quicker than the system call");

    // /proc/self/maps names both pages.
    char maps[16384];
    i64 fd = sys(SYS_openat, -100, (i64)"/proc/self/maps", 0);
    i64 n = fd >= 0 ? sys(SYS_read, fd, (i64)maps, sizeof maps - 1) : 0;
    maps[n > 0 ? n : 0] = 0;
    int vvar = 0, vdso = 0;
    for (i64 i = 0; i + 6 <= n; i++) {
        if (maps[i] == '[' && maps[i + 1] == 'v' && maps[i + 2] == 'v' && maps[i + 3] == 'a' && maps[i + 4] == 'r') vvar = 1;
        if (maps[i] == '[' && maps[i + 1] == 'v' && maps[i + 2] == 'd' && maps[i + 3] == 's' && maps[i + 4] == 'o') vdso = 1;
    }
    check(vvar && vdso, "/proc/self/maps: [vvar] and [vdso]");

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0);
}
