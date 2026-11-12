// sysfs, from a program with no libc: the CPUs online, a class's link to its device (not followed by lstat, its
// target by readlink, followed by stat to a directory), an interface's address read through that link, a class
// listed, a device file's numbers leading to its sysfs entry the way libudev goes (/sys/dev/char/MAJOR:MINOR), sysfs
// in the mount table, and nothing where there's nothing; and /proc's names as they are: cpuinfo in the reader's ISA's
// own terms, /proc/self a directory, no file where /proc has none.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_write = 64, SYS_exit_group = 94, SYS_openat = 56, SYS_read = 63, SYS_close = 57, SYS_newfstatat = 79,
       SYS_readlinkat = 78, SYS_getdents64 = 61 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c, x3 __asm__("x3") = d;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#define CPU_FIELD "CPU implementer"
#else
enum { SYS_write = 1, SYS_exit_group = 231, SYS_openat = 257, SYS_read = 0, SYS_close = 3, SYS_newfstatat = 262,
       SYS_readlinkat = 267, SYS_getdents64 = 217 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d) {
    register i64 r10 __asm__("r10") = d;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
#define CPU_FIELD "vendor_id"
#endif

enum { AT_FDCWD = -100, AT_SYMLINK_NOFOLLOW = 0x100, O_DIRECTORY = 0x4000 | 0x10000 };

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int starts(const char *s, const char *with) { for (u64 i = 0; with[i]; i++) if (s[i] != with[i]) return 0; return 1; }
static int has(const char *s, i64 n, const char *word) {
    u64 w = len(word);
    for (i64 i = 0; i + (i64)w <= n; i++) if (starts(s + i, word)) return 1;
    return 0;
}

static char buf[16384];
// A file's text, NUL-ended; how long it is, or the error.
static i64 slurp(const char *path) {
    i64 fd = sys(SYS_openat, AT_FDCWD, (i64)path, 0, 0);
    if (fd < 0) return fd;
    i64 n = sys(SYS_read, fd, (i64)buf, sizeof buf - 1, 0);
    sys(SYS_close, fd, 0, 0, 0);
    buf[n > 0 ? n : 0] = 0;
    return n;
}
static u32 mode_of(const char *path, int follow) {
    u64 st[32];
    if (sys(SYS_newfstatat, AT_FDCWD, (i64)path, (i64)st, follow ? 0 : AT_SYMLINK_NOFOLLOW) != 0) return 0;
    return *(u32 *)((char *)st + ST_MODE) & 0170000;
}

void cmain(i64 *sp) {
    (void)sp;
    i64 n = slurp("/sys/devices/system/cpu/online");
    check(n >= 2 && buf[0] == '0' && (buf[1] == '\n' || buf[1] == '-'), "/sys/devices/system/cpu/online: a list from CPU 0");

    char target[256];
    i64 t = sys(SYS_readlinkat, AT_FDCWD, (i64)"/sys/class/net/lo", (i64)target, sizeof target - 1);
    target[t > 0 ? t : 0] = 0;
    check(mode_of("/sys/class/net/lo", 0) == 0120000 && mode_of("/sys/class/net/lo", 1) == 0040000 && t > 0 && starts(target, "../../devices/virtual/net/lo"),
          "/sys/class/net/lo: a link (lstat), to ../../devices/virtual/net/lo (readlink), a directory (stat)");

    n = slurp("/sys/class/net/lo/address");
    int zeroes = n == 18 && starts(buf, "00:00:00:00:00:00\n");
    n = slurp("/sys/class/net/lo/mtu");
    check(zeroes && n == 6 && starts(buf, "65536\n"), "the loopback's address (all zeroes) and MTU, read through its class link");

    i64 dir = sys(SYS_openat, AT_FDCWD, (i64)"/sys/class/net", O_DIRECTORY, 0);
    i64 got = dir >= 0 ? sys(SYS_getdents64, dir, (i64)buf, sizeof buf, 0) : -1;
    int lo = 0, eth = 0, links = 1;
    for (i64 at = 0; at < got; at += *(unsigned short *)(buf + at + 16)) {
        char *name = buf + at + 19;
        if (name[0] == '.') continue;
        if (starts(name, "lo") && name[2] == 0) lo = 1;
        if (starts(name, "eth0")) eth = 1;
        if (buf[at + 18] != 10 /* DT_LNK */) links = 0;
    }
    check(lo && eth && links, "/sys/class/net listed: lo and eth0, each a link");

    // /dev/null's numbers (1:3) to its sysfs entry.
    n = slurp("/sys/dev/char/1:3/uevent");
    check(n > 0 && has(buf, n, "MAJOR=1") && has(buf, n, "MINOR=3") && has(buf, n, "DEVNAME=null"), "/sys/dev/char/1:3/uevent: /dev/null's");
    n = slurp("/sys/dev/char/1:3/dev");
    check(n == 4 && starts(buf, "1:3\n"), "/sys/dev/char/1:3/dev: its numbers");

    n = slurp("/proc/self/mountinfo");
    check(n > 0 && has(buf, n, " /sys ") && has(buf, n, " sysfs sysfs "), "/proc/self/mountinfo: sysfs at /sys");

    check(slurp("/sys/no/such/thing") == -2 && mode_of("/sys/class/net/nope", 0) == 0 && mode_of("/sys/devices/system/cpu/cpu0/cache", 1) == 0,
          "a path that isn't there, under a link or not: ENOENT");
    n = slurp("/sys/devices/system/node/node0/cpumap");
    check(n >= 2 && ((buf[0] >= '0' && buf[0] <= '9') || (buf[0] >= 'a' && buf[0] <= 'f')), "node0's cpumap: a hexadecimal mask");

    n = slurp("/proc/cpuinfo");
    check(n > 0 && has(buf, n, "processor") && has(buf, n, CPU_FIELD), "/proc/cpuinfo: each processor, in this ISA's terms");
    check(mode_of("/proc/self", 1) == 0040000 && mode_of("/proc/iSeries", 1) == 0 && mode_of("/proc/self/nothing", 1) == 0,
          "/proc/self a directory; nothing where /proc has nothing");

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0, 0);
}
