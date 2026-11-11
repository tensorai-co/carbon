// Files' names and metadata, for lambda: hard links, chmod, chown and utimensat as stat sees them, a file removed while
// it's open, flock and fcntl's F_GETLK, and close_range. What dpkg and apt need. Freestanding, for ARM64 and x86-64
// alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_dup = 23, SYS_fcntl = 25, SYS_flock = 32, SYS_unlinkat = 35, SYS_linkat = 37, SYS_fchmodat = 53,
       SYS_fchownat = 54, SYS_openat = 56, SYS_close = 57, SYS_lseek = 62, SYS_read = 63, SYS_write = 64,
       SYS_newfstatat = 79, SYS_utimensat = 88, SYS_exit_group = 94, SYS_close_range = 436 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#define ST_NLINK(st) (*(u32 *)((char *)(st) + 20))
#define ST_UID 24
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_lseek = 8, SYS_dup = 32, SYS_fcntl = 72, SYS_flock = 73,
       SYS_exit_group = 231, SYS_openat = 257, SYS_fchownat = 260, SYS_newfstatat = 262, SYS_unlinkat = 263,
       SYS_linkat = 265, SYS_fchmodat = 268, SYS_utimensat = 280, SYS_close_range = 436 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
#define ST_NLINK(st) (*(u64 *)((char *)(st) + 16))
#define ST_UID 28
#endif
#define ST_GID (ST_UID + 4)
#define ST_MTIME 88

#define AT_FDCWD (-100)
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define AT_SYMLINK_NOFOLLOW 0x100
#define LOCK_EX 2
#define LOCK_NB 4
#define LOCK_UN 8
#define F_GETLK 5
#define F_SETLK 6
#define F_WRLCK 1
#define F_UNLCK 2
#define EBADF 9
#define ENOENT 2

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
static u32 field(u64 *st, int at) { return *(u32 *)((char *)st + at); }
static i64 stat_of(const char *path, u64 *st) { return sys(SYS_newfstatat, AT_FDCWD, (i64)path, (i64)st, 0, 0, 0); }
static i64 make(const char *path, const char *text) {
    int fd = sys(SYS_openat, AT_FDCWD, (i64)path, O_RDWR | O_CREAT | O_TRUNC, 0644, 0, 0);
    sys(SYS_write, fd, (i64)text, len(text), 0, 0, 0);
    return fd;
}

void cmain(i64 *sp) {
    (void)sp;
    u64 st[32];
    char buf[64];

    // Hard links: two names for one file, either readable once the other's gone.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/a", 0, 0, 0, 0);
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/b", 0, 0, 0, 0);
    sys(SYS_close, make("/tmp/a", "linked"), 0, 0, 0, 0, 0);
    check(sys(SYS_linkat, AT_FDCWD, (i64)"/tmp/a", AT_FDCWD, (i64)"/tmp/b", 0, 0) == 0 && stat_of("/tmp/b", st) == 0 && ST_NLINK(st) == 2,
          "linkat: a second name, and st_nlink says two");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/a", 0, 0, 0, 0);
    int fd = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/b", O_RDWR, 0, 0, 0);
    i64 n = sys(SYS_read, fd, (i64)buf, sizeof buf, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    check(n == 6 && same(buf, "linked", 6) && stat_of("/tmp/b", st) == 0 && ST_NLINK(st) == 1, "the first name removed, the file's still there by the second");

    // Metadata, as stat sees it.
    check(sys(SYS_fchmodat, AT_FDCWD, (i64)"/tmp/b", 0750, 0, 0, 0) == 0 && stat_of("/tmp/b", st) == 0 && (field(st, ST_MODE) & 07777) == 0750 &&
          (field(st, ST_MODE) & 0170000) == 0100000, "chmod: the mode changes and it's still a regular file");
    check(sys(SYS_fchownat, AT_FDCWD, (i64)"/tmp/b", 42, 65534, 0, 0) == 0 && stat_of("/tmp/b", st) == 0 && field(st, ST_UID) == 42 && field(st, ST_GID) == 65534,
          "chown: owner and group");
    check(sys(SYS_fchownat, AT_FDCWD, (i64)"/tmp/b", -1, 7, 0, 0) == 0 && stat_of("/tmp/b", st) == 0 && field(st, ST_UID) == 42 && field(st, ST_GID) == 7,
          "chown with -1 leaves the owner as it was");
    i64 times[4] = {0, (1L << 30) - 2, 1234567890, 500};
    check(sys(SYS_utimensat, AT_FDCWD, (i64)"/tmp/b", (i64)times, 0, 0, 0) == 0 && stat_of("/tmp/b", st) == 0 &&
          *(i64 *)((char *)st + ST_MTIME) == 1234567890, "utimensat: the modification time");

    // A file removed while it's open is read and written until it's closed.
    fd = make("/tmp/scratch", "first ");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/scratch", 0, 0, 0, 0);
    i64 w = sys(SYS_write, fd, (i64)"second", 6, 0, 0, 0);
    sys(SYS_lseek, fd, 0, 0, 0, 0, 0);
    n = sys(SYS_read, fd, (i64)buf, sizeof buf, 0, 0, 0);
    check(stat_of("/tmp/scratch", st) == -ENOENT && w == 6 && n == 12 && same(buf, "first second", 12), "a file removed while open is still written and read");
    sys(SYS_close, fd, 0, 0, 0, 0, 0);

    // Locks: granted, and F_GETLK says nothing's in the way.
    fd = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/b", O_RDWR, 0, 0, 0);
    short lk[16] = {F_WRLCK, 0};
    check(sys(SYS_flock, fd, LOCK_EX | LOCK_NB, 0, 0, 0, 0) == 0 && sys(SYS_flock, fd, LOCK_UN, 0, 0, 0, 0) == 0, "flock");
    check(sys(SYS_fcntl, fd, F_SETLK, (i64)lk, 0, 0, 0) == 0 && sys(SYS_fcntl, fd, F_GETLK, (i64)lk, 0, 0, 0) == 0 && lk[0] == F_UNLCK,
          "fcntl: F_SETLK, and F_GETLK answers F_UNLCK");

    // close_range, and its CLOEXEC form.
    int first = sys(SYS_dup, fd, 0, 0, 0, 0, 0), second = sys(SYS_dup, fd, 0, 0, 0, 0, 0);
    check(sys(SYS_close_range, first, second, 4, 0, 0, 0) == 0 && sys(SYS_fcntl, first, 1, 0, 0, 0, 0) == 1, "close_range with CLOSE_RANGE_CLOEXEC marks them");
    check(sys(SYS_close_range, first, ~0u, 0, 0, 0, 0) == 0 && sys(SYS_close, second, 0, 0, 0, 0, 0) == -EBADF && sys(SYS_read, fd, (i64)buf, 1, 0, 0, 0) >= 0,
          "close_range closes them, and only them");

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
