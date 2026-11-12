// Extended attributes, mknod and openat2, for lambda. xattrs: set/get/list/remove in the user namespace, sizing a
// get and a list, ENODATA, XATTR_CREATE/XATTR_REPLACE, a value kept through a close, EOPNOTSUPP for an unknown
// namespace, EPERM for user. on a symlink, l* on the link itself, f* by descriptor, and the *at forms. mknod: a FIFO
// read and written across a fork, a character device node (EPERM without CAP_MKNOD, then its rdev in stat), EEXIST.
// openat2: a file opened, O_CREAT with mode, RESOLVE_BENEATH and RESOLVE_NO_SYMLINKS refused, RESOLVE_IN_ROOT, an
// unknown flag and a short/true struct. file_getattr reads zeros; file_setattr of nothing is accepted. Freestanding,
// for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_setxattr = 5, SYS_lsetxattr = 6, SYS_fsetxattr = 7, SYS_getxattr = 8, SYS_lgetxattr = 9, SYS_fgetxattr = 10,
       SYS_listxattr = 11, SYS_removexattr = 14, SYS_mknodat = 33, SYS_unlinkat = 35, SYS_symlinkat = 36,
       SYS_openat = 56, SYS_close = 57, SYS_read = 63, SYS_write = 64, SYS_newfstatat = 79, SYS_exit_group = 94,
       SYS_setresuid = 147, SYS_clone = 220, SYS_wait4 = 260, SYS_getxattrat = 464, SYS_openat2 = 437,
       SYS_file_getattr = 468, SYS_file_setattr = 469 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#define ST_RDEV 32
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_clone = 56, SYS_wait4 = 61, SYS_setresuid = 117,
       SYS_setxattr = 188, SYS_lsetxattr = 189, SYS_fsetxattr = 190, SYS_getxattr = 191, SYS_lgetxattr = 192,
       SYS_fgetxattr = 193, SYS_listxattr = 194, SYS_removexattr = 197, SYS_openat = 257, SYS_mknodat = 259,
       SYS_newfstatat = 262, SYS_unlinkat = 263, SYS_symlinkat = 266, SYS_exit_group = 231, SYS_openat2 = 437,
       SYS_getxattrat = 464, SYS_file_getattr = 468, SYS_file_setattr = 469 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
#define ST_RDEV 40
#endif

#define AT_FDCWD (-100)
#define AT_SYMLINK_NOFOLLOW 0x100
#define O_WRONLY 1
#define O_RDONLY 0
#define O_RDWR 2
#define O_CREAT 0100
#define EPERM 1
#define ENOENT 2
#define EEXIST 17
#define EINVAL 22
#define E2BIG 7
#define EXDEV 18
#define ELOOP 40
#define ENODATA 61
#define EOPNOTSUPP 95
#define SIGCHLD 17
#define XATTR_CREATE 1
#define XATTR_REPLACE 2
#define S_IFIFO 010000
#define S_IFCHR 020000
#define RESOLVE_NO_XDEV 0x01
#define RESOLVE_NO_SYMLINKS 0x04
#define RESOLVE_BENEATH 0x08
#define RESOLVE_IN_ROOT 0x10

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
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return (status >> 8) & 0xff; }
static int open_at(const char *path, int flags, int mode) { return sys(SYS_openat, AT_FDCWD, (i64)path, flags, mode, 0, 0); }
static int put(const char *path, const char *text) {
    int fd = open_at(path, O_WRONLY | O_CREAT, 0644);
    if (fd < 0) return fd;
    sys(SYS_write, fd, (i64)text, len(text), 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return 0;
}
static i64 setx(const char *path, const char *name, const char *val, u64 n, int flags) { return sys(SYS_setxattr, (i64)path, (i64)name, (i64)val, n, flags, 0); }
static i64 getx(const char *path, const char *name, char *val, u64 n) { return sys(SYS_getxattr, (i64)path, (i64)name, (i64)val, n, 0, 0); }

struct open_how { u64 flags, mode, resolve; };
static int openat2(int dfd, const char *path, u64 flags, u64 mode, u64 resolve, u64 size) {
    struct open_how how = {flags, mode, resolve};
    return sys(SYS_openat2, dfd, (i64)path, (i64)&how, size, 0, 0);
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[256];
    u64 st[20];

    // Extended attributes.
    put("/tmp/x", "data");
    check(setx("/tmp/x", "user.a", "one", 3, 0) == 0 && getx("/tmp/x", "user.a", buf, sizeof buf) == 3 && same(buf, "one", 3), "set and get user.a");
    check(getx("/tmp/x", "user.a", 0, 0) == 3, "getxattr size 0 gives the length");
    check(getx("/tmp/x", "user.a", buf, 2) == -34, "a buffer too small for the value: ERANGE");
    check(getx("/tmp/x", "user.none", buf, sizeof buf) == -ENODATA && setx("/tmp/x", "nodot", "x", 1, 0) == -EOPNOTSUPP &&
          setx("/tmp/x", "bogus.ns", "x", 1, 0) == -EOPNOTSUPP, "a missing attribute ENODATA; an unknown namespace EOPNOTSUPP");
    check(setx("/tmp/x", "user.a", "x", 1, XATTR_CREATE) == -EEXIST && setx("/tmp/x", "user.b", "x", 1, XATTR_REPLACE) == -ENODATA, "XATTR_CREATE on one there, XATTR_REPLACE on one not");
    setx("/tmp/x", "user.b", "two", 3, 0);
    setx("/tmp/x", "user.c", "three", 5, 0);
    i64 n = sys(SYS_listxattr, (i64)"/tmp/x", (i64)buf, sizeof buf, 0, 0, 0);
    check(n == (long)(7 + 7 + 7) && same(buf, "user.a\0user.b\0user.c\0", 21), "listxattr: the names, NUL-separated");
    check(sys(SYS_listxattr, (i64)"/tmp/x", 0, 0, 0, 0, 0) == n, "listxattr size 0 gives the length");
    check(sys(SYS_removexattr, (i64)"/tmp/x", (i64)"user.b", 0, 0, 0, 0) == 0 && getx("/tmp/x", "user.b", buf, sizeof buf) == -ENODATA &&
          sys(SYS_removexattr, (i64)"/tmp/x", (i64)"user.b", 0, 0, 0, 0) == -ENODATA, "removexattr, then ENODATA");
    // Kept through a close and reopen (it's on the file, not the open).
    int fd = open_at("/tmp/x", O_RDONLY, 0);
    check(sys(SYS_fgetxattr, fd, (i64)"user.a", (i64)buf, sizeof buf, 0, 0) == 3 && same(buf, "one", 3), "fgetxattr by descriptor");
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    // l* on a symlink: user. is refused on the link itself.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/link", 0, 0, 0, 0);
    sys(SYS_symlinkat, (i64)"/tmp/x", AT_FDCWD, (i64)"/tmp/link", 0, 0, 0);
    check(sys(SYS_lsetxattr, (i64)"/tmp/link", (i64)"user.a", (i64)"x", 1, 0, 0) == -EPERM && sys(SYS_lgetxattr, (i64)"/tmp/link", (i64)"user.a", (i64)buf, sizeof buf, 0, 0) == -EPERM,
          "user. on a symlink itself: EPERM for set and get");
    // The *at form.
    struct { u64 value; u32 size, flags; } args = {(u64)buf, sizeof buf, 0};
    i64 got = sys(SYS_getxattrat, AT_FDCWD, (i64)"/tmp/x", 0, (i64)"user.a", (i64)&args, 16);
    check(got == 3 && same(buf, "one", 3), "getxattrat");

    // mknod: a FIFO read and written across a fork.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/fifo", 0, 0, 0, 0);
    check(sys(SYS_mknodat, AT_FDCWD, (i64)"/tmp/fifo", S_IFIFO | 0644, 0, 0, 0) == 0 &&
          sys(SYS_newfstatat, AT_FDCWD, (i64)"/tmp/fifo", (i64)st, AT_SYMLINK_NOFOLLOW, 0, 0) == 0 &&
          (*(u32 *)((char *)st + ST_MODE) & 0170000) == S_IFIFO, "mknod a FIFO; stat says it's one");
    i64 child = fork();
    if (child == 0) {
        int w = open_at("/tmp/fifo", O_WRONLY, 0);
        sys(SYS_write, w, (i64)"through the pipe", 16, 0, 0, 0);
        sys(SYS_close, w, 0, 0, 0, 0, 0);
        quit(0);
    }
    int r = open_at("/tmp/fifo", O_RDONLY, 0);
    n = sys(SYS_read, r, (i64)buf, sizeof buf, 0, 0, 0);
    sys(SYS_close, r, 0, 0, 0, 0, 0);
    check(reap(child) == 0 && n == 16 && same(buf, "through the pipe", 16), "a FIFO carries bytes from a writer in another process to a reader");
    check(sys(SYS_mknodat, AT_FDCWD, (i64)"/tmp/fifo", S_IFIFO | 0644, 0, 0, 0) == -EEXIST, "mknod of one already there: EEXIST");

    // A character device node: EPERM without CAP_MKNOD, then (as root) its numbers in stat.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/dev", 0, 0, 0, 0);
    child = fork();
    if (child == 0) {
        sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0);
        quit(sys(SYS_mknodat, AT_FDCWD, (i64)"/tmp/dev", S_IFCHR | 0600, (1 << 8) | 7, 0, 0) == -EPERM ? 0 : 1);
    }
    check(reap(child) == 0, "a device node without CAP_MKNOD: EPERM");
    check(sys(SYS_mknodat, AT_FDCWD, (i64)"/tmp/dev", S_IFCHR | 0600, (1 << 8) | 7, 0, 0) == 0 &&
          sys(SYS_newfstatat, AT_FDCWD, (i64)"/tmp/dev", (i64)st, 0, 0, 0) == 0 && (*(u32 *)((char *)st + ST_MODE) & 0170000) == S_IFCHR &&
          *(u64 *)((char *)st + ST_RDEV) == ((1 << 8) | 7), "a character device node as root, with its rdev");

    // openat2.
    int o = openat2(AT_FDCWD, "/tmp/x", O_RDONLY, 0, 0, sizeof(struct open_how));
    check(o >= 0, "openat2 opens a file");
    sys(SYS_close, o, 0, 0, 0, 0, 0);
    o = openat2(AT_FDCWD, "/tmp/made", O_WRONLY | O_CREAT, 0644, 0, sizeof(struct open_how));
    check(o >= 0 && sys(SYS_write, o, (i64)"x", 1, 0, 0, 0) == 1, "openat2 with O_CREAT and a mode");
    sys(SYS_close, o, 0, 0, 0, 0, 0);
    check(openat2(AT_FDCWD, "/tmp/x", 1ul << 40, 0, 0, sizeof(struct open_how)) == -EINVAL &&
          openat2(AT_FDCWD, "/tmp/x", O_RDONLY, 0, 0x80, sizeof(struct open_how)) == -EINVAL &&
          openat2(AT_FDCWD, "/tmp/x", O_RDONLY, 0, 0, 8) == -EINVAL, "openat2: an unknown flag, an unknown resolve bit, a short struct");
    // RESOLVE_BENEATH keeps a relative path from escaping; a symlink is refused by RESOLVE_NO_SYMLINKS.
    int dir = open_at("/tmp", O_RDONLY, 0);
    check(openat2(dir, "../tmp/x", O_RDONLY, 0, RESOLVE_BENEATH, sizeof(struct open_how)) == -EXDEV, "RESOLVE_BENEATH: `..` out of the dirfd is EXDEV");
    check(openat2(dir, "link", O_RDONLY, 0, RESOLVE_NO_SYMLINKS, sizeof(struct open_how)) == -ELOOP, "RESOLVE_NO_SYMLINKS: a symlink is ELOOP");
    int in = openat2(dir, "/x", O_RDONLY, 0, RESOLVE_IN_ROOT, sizeof(struct open_how));
    check(in >= 0, "RESOLVE_IN_ROOT: an absolute path is taken from the dirfd");
    sys(SYS_close, in, 0, 0, 0, 0, 0);
    sys(SYS_close, dir, 0, 0, 0, 0, 0);

    // file_getattr and file_setattr.
    u64 fa[8];
    for (int i = 0; i < 8; i++) fa[i] = 0x1111;
    check(sys(SYS_file_getattr, AT_FDCWD, (i64)"/tmp/x", (i64)fa, 32, 0, 0) == 0 && fa[0] == 0 && fa[1] == 0, "file_getattr reads zeros");
    u64 zero[4] = {0, 0, 0, 0}, nonzero[4] = {1, 0, 0, 0};
    check(sys(SYS_file_setattr, AT_FDCWD, (i64)"/tmp/x", (i64)zero, 32, 0, 0) == 0 && sys(SYS_file_setattr, AT_FDCWD, (i64)"/tmp/x", (i64)nonzero, 32, 0, 0) == -EOPNOTSUPP,
          "file_setattr of nothing is fine; of something is EOPNOTSUPP");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
