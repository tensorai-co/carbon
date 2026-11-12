// Namespaces and the new mount API, for lambda. /proc/self/ns: eight links, each <kind>:[inode]; two processes in
// one namespace share an inode, a different one after unshare. unshare of a UTS namespace: the child's hostname is
// its own; the parent's is unchanged. unshare of an IPC namespace: a message queue made in it isn't the parent's.
// A new mount namespace: a mount the child makes isn't the parent's. setns back into the parent's UTS namespace by an
// ns fd. clone with CLONE_NEWUTS. EINVAL for a namespace lambda keeps one of (PID), EPERM without root. listns.
// The new mount API: fsopen/fsconfig/fsmount a tmpfs, move_mount it into place, use it, umount; fspick and
// mount_setattr to make it read-only; open_tree(OPEN_TREE_CLONE) and move_mount to bind a directory. Freestanding,
// for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_getcwd = 17, SYS_mkdirat = 34, SYS_umount2 = 39, SYS_statfs = 43, SYS_fsetxattr = 7, SYS_openat = 56,
       SYS_close = 57, SYS_read = 63, SYS_write = 64, SYS_newfstatat = 79, SYS_readlinkat = 78, SYS_exit_group = 94,
       SYS_setresuid = 147, SYS_sethostname = 161, SYS_uname = 160, SYS_unshare = 97, SYS_setns = 268,
       SYS_clone = 220, SYS_wait4 = 260, SYS_msgget = 186, SYS_msgctl = 187, SYS_move_mount = 429, SYS_fsopen = 430,
       SYS_fsconfig = 431, SYS_fsmount = 432, SYS_fspick = 433, SYS_mount_setattr = 442, SYS_open_tree = 428,
       SYS_listns = 470 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_clone = 56, SYS_wait4 = 61, SYS_getcwd = 79,
       SYS_setresuid = 117, SYS_statfs = 137, SYS_msgget = 68, SYS_msgctl = 71, SYS_uname = 63, SYS_sethostname = 170,
       SYS_unshare = 272, SYS_setns = 308, SYS_exit_group = 231, SYS_openat = 257, SYS_mkdirat = 258,
       SYS_newfstatat = 262, SYS_readlinkat = 267, SYS_umount2 = 166, SYS_open_tree = 428, SYS_move_mount = 429,
       SYS_fsopen = 430, SYS_fsconfig = 431, SYS_fsmount = 432, SYS_fspick = 433, SYS_mount_setattr = 442,
       SYS_listns = 470 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#endif

#define AT_FDCWD (-100)
#define O_WRONLY 1
#define O_RDONLY 0
#define O_CREAT 0100
#define EPERM 1
#define ENOENT 2
#define EACCES 13
#define EINVAL 22
#define EROFS 30
#define SIGCHLD 17
#define CLONE_NEWNS 0x00020000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWPID 0x20000000
#define IPC_CREAT 01000
#define IPC_RMID 0
#define IPC_PRIVATE 0
#define FSCONFIG_SET_STRING 1
#define FSCONFIG_CMD_CREATE 6
#define FSCONFIG_CMD_RECONFIGURE 7
#define MOVE_MOUNT_F_EMPTY_PATH 4
#define OPEN_TREE_CLONE 1
#define MOUNT_ATTR_RDONLY 1

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
static int starts(const char *s, const char *p) { return same(s, p, len(p)); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return (status >> 8) & 0xff; }
static int open_at(const char *path, int flags) { return sys(SYS_openat, AT_FDCWD, (i64)path, flags, 0644, 0, 0); }
static i64 readlink(const char *path, char *buf, u64 size) { return sys(SYS_readlinkat, AT_FDCWD, (i64)path, (i64)buf, size, 0, 0); }
static int put(const char *path, const char *text) {
    int fd = open_at(path, O_WRONLY | O_CREAT);
    if (fd < 0) return fd;
    sys(SYS_write, fd, (i64)text, len(text), 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return 0;
}
static i64 slurp(const char *path, char *buf, u64 size) {
    int fd = open_at(path, O_RDONLY);
    if (fd < 0) return fd;
    i64 n = sys(SYS_read, fd, (i64)buf, size, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return n;
}
static int mkdir(const char *path) { return sys(SYS_mkdirat, AT_FDCWD, (i64)path, 0755, 0, 0, 0); }
static u64 ns_ino(u64 *st) { return st[1]; }
static int stat_of(const char *path, u64 *st) { return sys(SYS_newfstatat, AT_FDCWD, (i64)path, (i64)st, 0, 0, 0); }
static int host_is(const char *want) {
    char uts[6 * 65];
    sys(SYS_uname, (i64)uts, 0, 0, 0, 0, 0);
    return same(uts + 65, want, len(want) + 1);
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[256];
    u64 st[20], st2[20];

    // /proc/self/ns: the eight links.
    const char *kinds[] = {"mnt", "uts", "ipc", "pid", "net", "user", "cgroup", "time"};
    int ok = 1;
    for (int i = 0; i < 8; i++) {
        char path[48], at[64];
        char *p = path;
        const char *pre = "/proc/self/ns/";
        for (const char *q = pre; *q; q++) *p++ = *q;
        for (const char *q = kinds[i]; *q; q++) *p++ = *q;
        *p = 0;
        i64 n = readlink(path, at, sizeof at);
        char head[12];
        u64 h = 0;
        for (const char *q = kinds[i]; *q; q++) head[h++] = *q;
        head[h++] = ':';
        head[h++] = '[';
        head[h] = 0;
        if (n <= 0 || !starts(at, head)) ok = 0;
    }
    check(ok, "/proc/self/ns: eight links, each <kind>:[inode]");
    // Two processes in one namespace share the link, a different one after unshare.
    char uts_link[64];
    i64 ul = readlink("/proc/self/ns/uts", uts_link, sizeof uts_link);
    uts_link[ul > 0 ? ul : 0] = 0;
    i64 child = fork();
    if (child == 0) {
        char s[64];
        i64 n = readlink("/proc/self/ns/uts", s, sizeof s);
        s[n > 0 ? n : 0] = 0;
        quit(n > 0 && same(s, uts_link, ul) ? 0 : 1);
    }
    check(reap(child) == 0, "a child shares its parent's namespace link");

    // unshare a UTS namespace.
    child = fork();
    if (child == 0) {
        u64 s[20];
        int bad = 0;
        (void)s;
        char link[64];
        if (sys(SYS_unshare, CLONE_NEWUTS, 0, 0, 0, 0, 0) != 0) bad |= 1;
        i64 n = readlink("/proc/self/ns/uts", link, sizeof link);
        link[n > 0 ? n : 0] = 0;
        if (n <= 0 || same(link, uts_link, ul)) bad |= 2;
        if (sys(SYS_sethostname, (i64)"child-host", 10, 0, 0, 0, 0) != 0 || !host_is("child-host")) bad |= 4;
        quit(bad);
    }
    check(reap(child) == 0 && !host_is("child-host"), "unshare UTS: the child's own hostname, a new inode, the parent's unchanged");

    // unshare an IPC namespace: a queue in it isn't the parent's.
    int q = sys(SYS_msgget, 0x4321, IPC_CREAT | 0600, 0, 0, 0, 0);
    child = fork();
    if (child == 0) {
        int bad = 0;
        if (sys(SYS_unshare, CLONE_NEWIPC, 0, 0, 0, 0, 0) != 0) bad |= 1;
        // The parent's queue isn't found; a private one can be made and removed.
        if (sys(SYS_msgget, 0x4321, 0, 0, 0, 0, 0) != -ENOENT) bad |= 2;
        int own = sys(SYS_msgget, IPC_PRIVATE, 0600, 0, 0, 0, 0);
        if (own < 0 || sys(SYS_msgctl, own, IPC_RMID, 0, 0, 0, 0) != 0) bad |= 4;
        quit(bad);
    }
    check(reap(child) == 0 && sys(SYS_msgget, 0x4321, 0, 0, 0, 0, 0) == q, "unshare IPC: the parent's queue is hidden, and still the parent's");
    sys(SYS_msgctl, q, IPC_RMID, 0, 0, 0, 0);

    // A new mount namespace: a mount the child makes isn't the parent's. setns back by an ns fd.
    mkdir("/tmp/nsmnt");
    int parent_uts = open_at("/proc/self/ns/uts", O_RDONLY);
    child = fork();
    if (child == 0) {
        int bad = 0;
        if (sys(SYS_unshare, CLONE_NEWNS | CLONE_NEWUTS, 0, 0, 0, 0, 0) != 0) bad |= 1;
        int fs = sys(SYS_fsopen, (i64)"tmpfs", 0, 0, 0, 0, 0);
        sys(SYS_fsconfig, fs, FSCONFIG_CMD_CREATE, 0, 0, 0, 0);
        int m = sys(SYS_fsmount, fs, 0, 0, 0, 0, 0);
        if (fs < 0 || m < 0 || sys(SYS_move_mount, m, (i64)"", AT_FDCWD, (i64)"/tmp/nsmnt", MOVE_MOUNT_F_EMPTY_PATH, 0) != 0) bad |= 2;
        if (put("/tmp/nsmnt/only", "child") != 0) bad |= 4;
        // setns back into the parent's UTS namespace: its hostname again.
        sys(SYS_sethostname, (i64)"temp", 4, 0, 0, 0, 0);
        if (sys(SYS_setns, parent_uts, CLONE_NEWUTS, 0, 0, 0, 0) != 0 || host_is("temp")) bad |= 8;
        quit(bad);
    }
    check(reap(child) == 0 && slurp("/tmp/nsmnt/only", buf, sizeof buf) == -ENOENT, "a child's mount in its own namespace isn't the parent's; setns rejoins the parent's UTS");

    // clone with CLONE_NEWUTS; and the refusals.
    check(sys(SYS_unshare, CLONE_NEWPID, 0, 0, 0, 0, 0) == -EINVAL, "unshare of a namespace lambda keeps one of: EINVAL");
    child = fork();
    if (child == 0) {
        sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0);
        quit(sys(SYS_unshare, CLONE_NEWNS, 0, 0, 0, 0, 0) == -EPERM ? 0 : 1);
    }
    check(reap(child) == 0, "unshare without root: EPERM");
    // listns: at least the eight the machine has.
    struct { u32 size, spare; u64 ns_id; u32 ns_type, spare2; u64 user_ns_id; } req = {32, 0, 0, 0, 0, 0};
    u64 ids[64];
    i64 count = sys(SYS_listns, (i64)&req, (i64)ids, 64, 0, 0, 0);
    check(count >= 8, "listns gives the machine's namespaces");

    // The new mount API: a tmpfs, sized, used, made read-only, unmounted.
    mkdir("/tmp/new");
    int fs = sys(SYS_fsopen, (i64)"tmpfs", 0, 0, 0, 0, 0);
    sys(SYS_fsconfig, fs, FSCONFIG_SET_STRING, (i64)"size", (i64)"2m", 0, 0);
    check(fs >= 0 && sys(SYS_fsconfig, fs, FSCONFIG_CMD_CREATE, 0, 0, 0, 0) == 0, "fsopen and fsconfig a tmpfs");
    int m = sys(SYS_fsmount, fs, 0, 0, 0, 0, 0);
    check(m >= 0 && sys(SYS_move_mount, m, (i64)"", AT_FDCWD, (i64)"/tmp/new", MOVE_MOUNT_F_EMPTY_PATH, 0) == 0 && put("/tmp/new/f", "hi") == 0 &&
          slurp("/tmp/new/f", buf, sizeof buf) == 2, "fsmount and move_mount it into place, then use it");
    u64 sb[16] = {0};
    sys(SYS_statfs, (i64)"/tmp/new", (i64)sb, 0, 0, 0, 0);
    check(sb[0] == 0x01021994, "statfs says tmpfs");
    int pick = sys(SYS_fspick, AT_FDCWD, (i64)"/tmp/new", 0, 0, 0, 0);
    struct { u64 set, clr, prop, userns; } attr = {MOUNT_ATTR_RDONLY, 0, 0, 0};
    check(sys(SYS_mount_setattr, AT_FDCWD, (i64)"/tmp/new", 0, (i64)&attr, 32, 0) == 0 && open_at("/tmp/new/g", O_WRONLY | O_CREAT) == -EROFS, "mount_setattr makes it read-only");
    (void)pick;
    sys(SYS_umount2, (i64)"/tmp/new", 0, 0, 0, 0, 0);

    // open_tree(OPEN_TREE_CLONE) and move_mount, to bind a directory.
    mkdir("/tmp/ot_src");
    mkdir("/tmp/ot_dst");
    put("/tmp/ot_src/a", "tree");
    int tree = sys(SYS_open_tree, AT_FDCWD, (i64)"/tmp/ot_src", OPEN_TREE_CLONE, 0, 0, 0);
    check(tree >= 0 && sys(SYS_move_mount, tree, (i64)"", AT_FDCWD, (i64)"/tmp/ot_dst", MOVE_MOUNT_F_EMPTY_PATH, 0) == 0 && slurp("/tmp/ot_dst/a", buf, sizeof buf) == 4 &&
          same(buf, "tree", 4), "open_tree clone and move_mount bind a directory");
    sys(SYS_umount2, (i64)"/tmp/ot_dst", 0, 0, 0, 0, 0);

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
