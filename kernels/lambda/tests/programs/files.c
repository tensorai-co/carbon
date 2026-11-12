// Files' names and metadata, for lambda: hard links, chmod, chown and utimensat as stat sees them, a file removed while
// it's open, flock and fcntl's F_GETLK, and close_range. What dpkg and apt need. Freestanding, for ARM64 and x86-64
// alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_symlinkat = 36, SYS_faccessat = 48, SYS_pipe2 = 59, SYS_readlinkat = 78, SYS_dup = 23, SYS_fcntl = 25, SYS_flock = 32, SYS_unlinkat = 35, SYS_linkat = 37, SYS_fchmodat = 53,
       SYS_fchownat = 54, SYS_openat = 56, SYS_close = 57, SYS_lseek = 62, SYS_read = 63, SYS_write = 64,
       SYS_newfstatat = 79, SYS_utimensat = 88, SYS_exit_group = 94, SYS_close_range = 436, SYS_getdents64 = 61, SYS_getpid = 172 };
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
#define O_DIRECTORY 040000
#else
enum { SYS_symlinkat = 266, SYS_faccessat = 269, SYS_pipe2 = 293, SYS_readlinkat = 267, SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_lseek = 8, SYS_dup = 32, SYS_fcntl = 72, SYS_flock = 73,
       SYS_exit_group = 231, SYS_openat = 257, SYS_fchownat = 260, SYS_newfstatat = 262, SYS_unlinkat = 263,
       SYS_linkat = 265, SYS_fchmodat = 268, SYS_utimensat = 280, SYS_close_range = 436, SYS_getdents64 = 217, SYS_getpid = 39 };
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
#define O_DIRECTORY 0200000
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

// `prefix` and `fd`'s digits, in `out`.
static char *fd_path(char *out, const char *prefix, int fd) {
    u64 n = 0;
    while (prefix[n]) { out[n] = prefix[n]; n++; }
    char digits[12];
    int d = 0;
    do { digits[d++] = (char)('0' + fd % 10); fd /= 10; } while (fd);
    while (d) out[n++] = digits[--d];
    out[n] = 0;
    return out;
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

    // Paths through the disk's links to what lambda makes: /dev/fd (bash's process substitution), /dev/stdin, and a
    // link of the program's own into /proc.
    int p[2];
    char path[40];
    sys(SYS_pipe2, (i64)p, 0, 0, 0, 0, 0);
    sys(SYS_write, p[1], (i64)"through", 7, 0, 0, 0);
    int via = sys(SYS_openat, AT_FDCWD, (i64)fd_path(path, "/dev/fd/", p[0]), 0, 0, 0, 0);
    n = sys(SYS_read, via, (i64)buf, sizeof buf, 0, 0, 0);
    check(via >= 0 && n == 7 && same(buf, "through", 7), "/dev/fd/N opens the pipe that N is");
    int back = sys(SYS_openat, AT_FDCWD, (i64)fd_path(path, "/dev/fd/", p[1]), 01 | O_CREAT | O_TRUNC, 0644, 0, 0);
    sys(SYS_write, back, (i64)"back", 4, 0, 0, 0);
    check(back >= 0 && sys(SYS_read, p[0], (i64)buf, sizeof buf, 0, 0, 0) == 4 && same(buf, "back", 4), "and its write end, with O_CREAT and O_TRUNC, as >(...) opens it");
    // Two pipes are two files to stat, as diff <(a) <(b) needs them to be.
    int q[2];
    u64 st2[32];
    sys(SYS_pipe2, (i64)q, 0, 0, 0, 0, 0);
    check(stat_of(fd_path(path, "/dev/fd/", p[0]), st) == 0 && stat_of(fd_path(path, "/dev/fd/", q[0]), st2) == 0 && (st[0] != st2[0] || st[1] != st2[1]) &&
          stat_of(fd_path(path, "/dev/fd/", p[1]), st2) == 0 && st[1] == st2[1], "two pipes have different inodes; a pipe's two ends, the same");
    check(stat_of("/dev/stdin", st) == 0 && sys(SYS_faccessat, AT_FDCWD, (i64)"/dev/fd/1", 0, 0, 0, 0) == 0 &&
          sys(SYS_readlinkat, AT_FDCWD, (i64)"/dev/fd/0", (i64)buf, sizeof buf, 0, 0) > 0, "stat /dev/stdin, access /dev/fd/1, readlink /dev/fd/0");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/me", 0, 0, 0, 0);
    sys(SYS_symlinkat, (i64)"../proc/self", AT_FDCWD, (i64)"/tmp/me", 0, 0, 0);
    sys(SYS_write, p[1], (i64)"mine", 4, 0, 0, 0);
    int mine = sys(SYS_openat, AT_FDCWD, (i64)fd_path(path, "/tmp/me/fd/", p[0]), 0, 0, 0, 0);
    check(mine >= 0 && sys(SYS_read, mine, (i64)buf, sizeof buf, 0, 0, 0) == 4 && same(buf, "mine", 4), "a link of its own, relative, into /proc/self");
    check(sys(SYS_openat, AT_FDCWD, (i64)"/dev/fd/900", 0, 0, 0, 0) == -ENOENT && stat_of("/dev/nothing/here", st) == -ENOENT, "a descriptor that isn't open, and a path that isn't there: ENOENT");

    // /proc/self/fd and /proc/self/task: directories, listed as Linux lists them (what closefrom and sandboxes read).
    int marker = sys(SYS_openat, AT_FDCWD, (i64)"/dev/null", 0, 0, 0, 0);
    int dir = sys(SYS_openat, AT_FDCWD, (i64)"/proc/self/fd", O_DIRECTORY, 0, 0, 0);
    char dents[4096], name[16];
    i64 got = dir >= 0 ? sys(SYS_getdents64, dir, (i64)dents, sizeof dents, 0, 0, 0) : -1;
    int saw0 = 0, saw_marker = 0, links = 1;
    for (i64 at = 0; at < got; at += *(unsigned short *)(dents + at + 16)) {
        char *entry = dents + at + 19;
        if (entry[0] == '.') continue;
        i64 n = 0;
        for (char *c = entry; *c; c++) n = n * 10 + (*c - '0');
        if (dents[at + 18] != 10 /* DT_LNK */) links = 0;
        if (n == 0) saw0 = 1;
        if (n == marker) saw_marker = 1;
    }
    char *digits = fd_path(name, "", marker);
    i64 target = dir >= 0 ? sys(SYS_readlinkat, dir, (i64)digits, (i64)buf, sizeof buf, 0, 0) : -1;
    check(stat_of("/proc/self/fd", st) == 0 && (field(st, ST_MODE) & 0170000) == 0040000 && got > 0 && saw0 && saw_marker && links &&
          target == 9 && same(buf, "/dev/null", 9), "/proc/self/fd: a directory listing each open descriptor as a link, read relative to it");
    sys(SYS_close, dir, 0, 0, 0, 0, 0);

    // A device opened is the file it was opened from: fstat says what stat of its path says (st_dev and st_ino, at 0
    // and 8 on both ISAs), as `cp` and `install` check before copying from it.
    u64 opened[32];
    check(stat_of("/dev/null", st) == 0 && sys(SYS_newfstatat, marker, (i64)"", (i64)opened, 0x1000 /* AT_EMPTY_PATH */, 0, 0) == 0 &&
          st[0] == opened[0] && st[1] == opened[1] && st[1] != 0 && (field(opened, ST_MODE) & 0170000) == 0020000,
          "fstat of an open /dev/null: the same device and inode as stat of its path, a character device");
    i64 pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    int tasks = sys(SYS_openat, AT_FDCWD, (i64)"/proc/self/task", O_DIRECTORY, 0, 0, 0);
    got = tasks >= 0 ? sys(SYS_getdents64, tasks, (i64)dents, sizeof dents, 0, 0, 0) : -1;
    int saw_me = 0;
    for (i64 at = 0; at < got; at += *(unsigned short *)(dents + at + 16)) {
        char *entry = dents + at + 19;
        i64 n = 0;
        for (char *c = entry; *c >= '0' && *c <= '9'; c++) n = n * 10 + (*c - '0');
        if (n == pid && dents[at + 18] == 4 /* DT_DIR */) saw_me = 1;
    }
    check(saw_me && stat_of("/proc/self/task", st) == 0 && ST_NLINK(st) == 3 && sys(SYS_openat, AT_FDCWD, (i64)"/proc/self/maps", O_DIRECTORY, 0, 0, 0) < 0,
          "/proc/self/task: its one thread, and 3 links (single-threaded, as sandboxes check); a file isn't a directory");
    sys(SYS_close, tasks, 0, 0, 0, 0, 0);

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
