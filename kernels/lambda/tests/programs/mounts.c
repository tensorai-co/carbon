// Mounts and roots, for lambda: /proc's mounts and mountinfo, statfs's types; a tmpfs with its options and mode,
// on a device of its own, EXDEV across it, busy to rmdir; a bind mount, `..` out of it, remounted read-only (EROFS for
// writes, makes, removes; the source still writable), unmounted; MS_MOVE; propagation; ENODEV, EBUSY, EINVAL, and
// EPERM without root; chroot (absolute paths, `..` and absolute links kept inside) and fchroot; and last, as it
// changes the machine's root, pivot_root, with the old root unmounted lazily. Freestanding, for ARM64 and x86-64
// alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_getcwd = 17, SYS_mkdirat = 34, SYS_unlinkat = 35, SYS_symlinkat = 36, SYS_linkat = 37, SYS_umount2 = 39,
       SYS_mount = 40, SYS_pivot_root = 41, SYS_statfs = 43, SYS_chdir = 49, SYS_chroot = 51, SYS_openat = 56,
       SYS_close = 57, SYS_read = 63, SYS_write = 64, SYS_newfstatat = 79, SYS_exit_group = 94, SYS_setresuid = 147,
       SYS_clone = 220, SYS_wait4 = 260, SYS_renameat2 = 276, SYS_fchroot = 472 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_clone = 56, SYS_wait4 = 61, SYS_getcwd = 79, SYS_chdir = 80,
       SYS_setresuid = 117, SYS_statfs = 137, SYS_pivot_root = 155, SYS_chroot = 161, SYS_mount = 165,
       SYS_umount2 = 166, SYS_exit_group = 231, SYS_openat = 257, SYS_mkdirat = 258, SYS_newfstatat = 262,
       SYS_unlinkat = 263, SYS_linkat = 265, SYS_symlinkat = 266, SYS_renameat2 = 316, SYS_fchroot = 472 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
#endif

#define AT_FDCWD (-100)
#define AT_REMOVEDIR 0x200
#define O_WRONLY 1
#define O_CREAT 0100
#define O_TRUNC 01000
#define EPERM 1
#define ENOENT 2
#define EBUSY 16
#define EXDEV 18
#define ENODEV 19
#define EINVAL 22
#define EROFS 30
#define SIGCHLD 17
#define MS_RDONLY 1
#define MS_REMOUNT 32
#define MS_BIND 4096
#define MS_MOVE 8192
#define MS_REC 16384
#define MS_PRIVATE (1 << 18)
#define MS_SHARED (1 << 20)
#define MNT_DETACH 2

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
static int contains(const char *hay, i64 n, const char *needle) {
    u64 k = len(needle);
    for (i64 i = 0; i + (i64)k <= n; i++) if (same(hay + i, needle, k)) return 1;
    return 0;
}
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return (status >> 8) & 0xff; }
static int open_at(const char *path, int flags) { return sys(SYS_openat, AT_FDCWD, (i64)path, flags, 0644, 0, 0); }
static i64 slurp(const char *path, char *buf, u64 size) {
    int fd = open_at(path, 0);
    if (fd < 0) return fd;
    i64 n = sys(SYS_read, fd, (i64)buf, size, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return n;
}
static int put(const char *path, const char *text) {
    int fd = open_at(path, O_WRONLY | O_CREAT | O_TRUNC);
    if (fd < 0) return fd;
    sys(SYS_write, fd, (i64)text, len(text), 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return 0;
}
static int mkdir(const char *path, int mode) { return sys(SYS_mkdirat, AT_FDCWD, (i64)path, mode, 0, 0, 0); }
static int stat_of(const char *path, u64 *st) { return sys(SYS_newfstatat, AT_FDCWD, (i64)path, (i64)st, 0, 0, 0); }
static u64 fs_type(const char *path, u64 *flags) {
    u64 b[16] = {0};
    if (sys(SYS_statfs, (i64)path, (i64)b, 0, 0, 0, 0) != 0) return 0;
    if (flags) *flags = b[10];
    return b[0];
}
static int mount(const char *src, const char *dst, const char *type, u64 flags, const char *data) {
    return sys(SYS_mount, (i64)src, (i64)dst, (i64)type, flags, (i64)data, 0);
}
static int umount(const char *path, int flags) { return sys(SYS_umount2, (i64)path, flags, 0, 0, 0, 0); }

void cmain(i64 *sp) {
    (void)sp;
    char buf[8192];
    u64 st[32], st2[32];

    i64 n = slurp("/proc/self/mounts", buf, sizeof buf);
    check(n > 0 && contains(buf, n, "/dev/root / ext4 rw,relatime 0 0\n") && contains(buf, n, "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n"),
          "/proc/self/mounts: the disk at /, proc at /proc");
    n = slurp("/proc/self/mountinfo", buf, sizeof buf);
    check(n > 0 && contains(buf, n, " / / rw,relatime - ext4 /dev/root rw\n"), "mountinfo: the root mount");
    check(fs_type("/", 0) == 0xef53 && fs_type("/proc/self", 0) == 0x9fa0, "statfs: ext4 at /, proc at /proc");

    // A tmpfs.
    mkdir("/tmp/m", 0755);
    check(mount("none", "/tmp/m", "tmpfs", 0, "size=1m,mode=700") == 0 && fs_type("/tmp/m", 0) == 0x01021994, "mount a tmpfs; statfs says so");
    put("/tmp/m/f", "in tmpfs");
    stat_of("/tmp/m/f", st);
    stat_of("/", st2);
    check(st[0] != st2[0], "its files are on a device of their own");
    stat_of("/tmp/m", st);
    n = slurp("/proc/self/mounts", buf, sizeof buf);
    check((*(u32 *)((char *)st + ST_MODE) & 0777) == 0700 && contains(buf, n, "none /tmp/m tmpfs rw,relatime,size=1m,mode=700 0 0\n"), "its mode and options");
    check(sys(SYS_renameat2, AT_FDCWD, (i64)"/tmp/m/f", AT_FDCWD, (i64)"/tmp/f", 0, 0) == -EXDEV &&
          sys(SYS_linkat, AT_FDCWD, (i64)"/tmp/m/f", AT_FDCWD, (i64)"/tmp/f", 0, 0) == -EXDEV &&
          sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/m", AT_REMOVEDIR, 0, 0, 0) == -EBUSY, "renaming or linking out of it EXDEV; removing its mount point EBUSY");

    // A bind mount.
    mkdir("/tmp/src", 0755);
    mkdir("/tmp/dst", 0755);
    put("/tmp/src/a", "data");
    check(mount("/tmp/src", "/tmp/dst", 0, MS_BIND, 0) == 0 && slurp("/tmp/dst/a", buf, sizeof buf) == 4 && same(buf, "data", 4), "bind mount: the source's file, through the target");
    stat_of("/tmp/dst/..", st);
    stat_of("/tmp", st2);
    check(st[1] == st2[1], "`..` out of a bind mount is where it's mounted");
    u64 flags = 0;
    check(mount(0, "/tmp/dst", 0, MS_BIND | MS_REMOUNT | MS_RDONLY, 0) == 0 && open_at("/tmp/dst/a", O_WRONLY) == -EROFS && put("/tmp/dst/new", "x") == -EROFS &&
          mkdir("/tmp/dst/d", 0755) == -EROFS && sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/dst/a", 0, 0, 0, 0) == -EROFS && put("/tmp/src/a", "more") == 0 &&
          fs_type("/tmp/dst", &flags) == 0xef53 && (flags & 1), "remounted read-only: writes, makes and removes EROFS; the source still writable; ST_RDONLY");
    n = slurp("/proc/self/mounts", buf, sizeof buf);
    check(contains(buf, n, " /tmp/dst ext4 ro,relatime 0 0\n"), "mounts shows it ro");
    check(umount("/tmp/dst", 0) == 0 && slurp("/tmp/dst/a", buf, sizeof buf) == -ENOENT && umount("/tmp/dst", 0) == -EINVAL, "umount2, and again: EINVAL");

    // Moving, propagation, what isn't.
    mkdir("/tmp/m2", 0755);
    check(mount("/tmp/m", "/tmp/m2", 0, MS_MOVE, 0) == 0 && slurp("/tmp/m2/f", buf, sizeof buf) == 8 && slurp("/tmp/m/f", buf, sizeof buf) == -ENOENT, "MS_MOVE");
    check(mount(0, "/", 0, MS_REC | MS_PRIVATE, 0) == 0 && mount(0, "/tmp/m2", 0, MS_SHARED, 0) == 0, "propagation: / private, recursively; a tmpfs shared");
    n = slurp("/proc/self/mountinfo", buf, sizeof buf);
    check(contains(buf, n, " /tmp/m2 rw,relatime shared:"), "mountinfo has its peer group");
    check(mount("sys", "/tmp/m2", "sysfs", 0, 0) == -ENODEV && umount("/", 0) == -EBUSY && umount("/tmp", 0) == -EINVAL && umount("/tmp/m2", 99) == -EINVAL,
          "sysfs ENODEV; / busy; not a mount point, a bad flag: EINVAL");
    check(umount("/tmp/m2", 0) == 0 && slurp("/tmp/m2/f", buf, sizeof buf) == -ENOENT, "the tmpfs unmounted, and its files gone");
    i64 child = fork();
    if (child == 0) {
        sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0);
        quit(mount("none", "/tmp/m2", "tmpfs", 0, 0) == -EPERM && umount("/proc", 0) == -EPERM && sys(SYS_chroot, (i64)"/tmp", 0, 0, 0, 0, 0) == -EPERM ? 0 : 1);
    }
    check(reap(child) == 0, "without root: EPERM for mount, umount2 and chroot");

    // chroot and fchroot.
    mkdir("/tmp/jail", 0755);
    mkdir("/tmp/jail/etc", 0755);
    put("/tmp/jail/etc/name", "jail");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/jail/escape", 0, 0, 0, 0);
    sys(SYS_symlinkat, (i64)"/../../../etc/name", AT_FDCWD, (i64)"/tmp/jail/escape", 0, 0, 0);
    child = fork();
    if (child == 0) {
        int bad = 0;
        if (sys(SYS_chroot, (i64)"/tmp/jail", 0, 0, 0, 0, 0) != 0 || sys(SYS_chdir, (i64)"/", 0, 0, 0, 0, 0) != 0) bad |= 1;
        char in[16];
        if (slurp("/etc/name", in, sizeof in) != 4 || !same(in, "jail", 4)) bad |= 2;
        if (slurp("/../../etc/name", in, sizeof in) != 4 || slurp("/escape", in, sizeof in) != 4 || !same(in, "jail", 4)) bad |= 4;
        u64 s[32];
        if (stat_of("/tmp", s) != -ENOENT) bad |= 8;
        char cwd[64];
        if (sys(SYS_getcwd, (i64)cwd, sizeof cwd, 0, 0, 0, 0) != 2 || cwd[0] != '/') bad |= 16;
        quit(bad);
    }
    check(reap(child) == 0, "chroot: absolute paths from it, `..` and an absolute link kept inside, nothing above it");
    child = fork();
    if (child == 0) {
        int dir = open_at("/tmp/jail", 0);
        char in[16];
        quit(sys(SYS_fchroot, dir, 0, 0, 0, 0, 0) == 0 && slurp("/etc/name", in, sizeof in) == 4 && same(in, "jail", 4) ? 0 : 1);
    }
    check(reap(child) == 0, "fchroot to an open directory");

    // pivot_root, last: it changes the machine's root.
    mkdir("/tmp/newroot", 0755);
    check(sys(SYS_pivot_root, (i64)"/tmp", (i64)"/tmp/newroot", 0, 0, 0, 0) == -EINVAL, "pivot_root to what isn't a mount: EINVAL");
    mount("none", "/tmp/newroot", "tmpfs", 0, 0);
    mkdir("/tmp/newroot/old", 0755);
    put("/tmp/newroot/hello", "new root");
    check(sys(SYS_pivot_root, (i64)"/tmp/newroot", (i64)"/tmp/newroot/old", 0, 0, 0, 0) == 0 && slurp("/hello", buf, sizeof buf) == 8 &&
          stat_of("/old/tmp/jail/etc/name", st) == 0, "pivot_root: the tmpfs is /, and the old root is at /old");
    sys(SYS_chdir, (i64)"/", 0, 0, 0, 0, 0);
    check(umount("/old", MNT_DETACH) == 0 && stat_of("/old/tmp", st) == -ENOENT, "the old root unmounted, lazily, with what's under it");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
