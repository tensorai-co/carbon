// fanotify, statmount/listmount, remap_file_pages and x86-64's old calls, for lambda. fanotify: a group (EINVAL for
// FAN_REPORT_FID, EPERM without CAP_SYS_ADMIN), a mark on a file (EINVAL for an event that needs FID), EAGAIN with
// nothing yet; an open, a write and a close merged into one event, with the right version, length and pid, and a
// descriptor that /proc/self/fd names as the file; FIONREAD; an ignore mask; a directory's mark with
// FAN_EVENT_ON_CHILD; flushing; a whole file system's mark, then flushed. statmount/listmount: the mounts by ID, the
// root's type, magic and mount point, EINVAL and ENOENT, a tmpfs mounted showing up with its parent, after `param`,
// and in reverse. remap_file_pages: a shared mapping's first page remapped to its second, EINVAL for prot and for a
// private mapping. x86-64 only: ustat, sysfs, modify_ldt, uprobe (ENXIO) and uretprobe (SIGILL). Freestanding, for
// ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_ioctl = 29, SYS_mkdirat = 34, SYS_mount = 40, SYS_ftruncate = 46, SYS_openat = 56, SYS_close = 57,
       SYS_read = 63, SYS_write = 64, SYS_pwrite64 = 68, SYS_readlinkat = 78, SYS_exit_group = 94,
       SYS_setresuid = 147, SYS_getpid = 172, SYS_clone = 220, SYS_mmap = 222, SYS_remap_file_pages = 234,
       SYS_wait4 = 260, SYS_fanotify_init = 262, SYS_fanotify_mark = 263, SYS_memfd_create = 279,
       SYS_statmount = 457, SYS_listmount = 458 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define X86 0
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_mmap = 9, SYS_ioctl = 16, SYS_pwrite64 = 18, SYS_getpid = 39,
       SYS_clone = 56, SYS_wait4 = 61, SYS_ftruncate = 77, SYS_setresuid = 117, SYS_ustat = 136, SYS_sysfs = 139,
       SYS_modify_ldt = 154, SYS_mount = 165, SYS_remap_file_pages = 216, SYS_exit_group = 231, SYS_openat = 257,
       SYS_mkdirat = 258, SYS_readlinkat = 267, SYS_fanotify_init = 300, SYS_fanotify_mark = 301,
       SYS_memfd_create = 319, SYS_uretprobe = 335, SYS_uprobe = 336, SYS_statmount = 457, SYS_listmount = 458 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define X86 1
#endif

#define AT_FDCWD (-100)
#define O_RDONLY 0
#define O_WRONLY 1
#define O_RDWR 2
#define O_CREAT 0100
#define EPERM 1
#define ENOENT 2
#define ENXIO 6
#define EAGAIN 11
#define EINVAL 22
#define SIGCHLD 17
#define FIONREAD 0x541b

#define FAN_ACCESS 0x1
#define FAN_MODIFY 0x2
#define FAN_CLOSE_WRITE 0x8
#define FAN_CLOSE_NOWRITE 0x10
#define FAN_OPEN 0x20
#define FAN_CREATE 0x100
#define FAN_EVENT_ON_CHILD 0x08000000
#define FAN_NONBLOCK 0x2
#define FAN_REPORT_FID 0x200
#define FAN_MARK_ADD 0x1
#define FAN_MARK_IGNORED_MASK 0x20
#define FAN_MARK_FLUSH 0x80
#define FAN_MARK_FILESYSTEM 0x100

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
static int streq(const char *a, const char *b) { return len(a) == len(b) && same(a, b, len(a)); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static int wait_status(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return status; }
static int open_at(const char *path, int flags, int mode) { return sys(SYS_openat, AT_FDCWD, (i64)path, flags, mode, 0, 0); }
static void put(const char *path) {
    int fd = open_at(path, O_WRONLY | O_CREAT, 0644);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
}
static void touch(const char *path) { sys(SYS_close, open_at(path, O_RDONLY, 0), 0, 0, 0, 0, 0); }
static char *num(char *at, i64 n) {
    char d[24]; int k = 0;
    do { d[k++] = '0' + n % 10; n /= 10; } while (n);
    while (k) *at++ = d[--k];
    return at;
}
static i64 mark(int g, u32 flags, u64 mask, const char *path) { return sys(SYS_fanotify_mark, g, flags, mask, AT_FDCWD, (i64)path, 0); }

// One read of a group: its first event, and how many there were.
struct event { u32 len; u8 vers, reserved; u16 mlen; u64 mask; int fd; int pid; };
static struct event events[16];
static i64 take(int g) {
    i64 n = sys(SYS_read, g, (i64)events, sizeof events, 0, 0, 0);
    return n < 0 ? n : n / (i64)sizeof(struct event);
}
// What /proc/self/fd says a descriptor is; and closes it.
static char linked[128];
static const char *named(int fd) {
    char path[48], *at = path;
    for (const char *s = "/proc/self/fd/"; *s; s++) *at++ = *s;
    at = num(at, fd);
    *at = 0;
    i64 n = sys(SYS_readlinkat, AT_FDCWD, (i64)path, (i64)linked, sizeof linked - 1, 0, 0);
    linked[n < 0 ? 0 : n] = 0;
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return linked;
}

struct mnt_id_req { u32 size, spare; u64 mnt_id, param; };
static u64 sm[1024];
static i64 statmount(u64 id, u64 mask, u64 size) {
    struct mnt_id_req req = {24, 0, id, mask};
    return sys(SYS_statmount, (i64)&req, (i64)sm, size, 0, 0, 0);
}
static const char *sm_str(int field) { return (const char *)sm + 512 + ((u32 *)sm)[field / 4]; }
#define SM_U64(off) (sm[(off) / 8])
static u64 ids[64];
static i64 listmount(u64 parent, u64 after, u64 flags) {
    struct mnt_id_req req = {24, 0, parent, after};
    return sys(SYS_listmount, (i64)&req, (i64)ids, 64, flags, 0, 0);
}
#define LSMT_ROOT (~0UL)

void cmain(i64 *sp) {
    (void)sp;
    i64 me = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    // fanotify.
    put("/tmp/fa");
    put("/tmp/fb");
    sys(SYS_mkdirat, AT_FDCWD, (i64)"/tmp/fd", 0755, 0, 0, 0);
    put("/tmp/fd/c");
    int g = sys(SYS_fanotify_init, FAN_NONBLOCK, O_RDONLY, 0, 0, 0, 0);
    check(g >= 0 && sys(SYS_fanotify_init, FAN_REPORT_FID, O_RDONLY, 0, 0, 0, 0) == -EINVAL, "fanotify_init, and EINVAL for FAN_REPORT_FID (not yet)");
    i64 child = fork();
    if (child == 0) {
        sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0);
        quit(sys(SYS_fanotify_init, 0, O_RDONLY, 0, 0, 0, 0) == -EPERM ? 0 : 1);
    }
    check(wait_status(child) == 0, "fanotify_init without CAP_SYS_ADMIN: EPERM");
    check(mark(g, FAN_MARK_ADD, FAN_OPEN | FAN_MODIFY | FAN_CLOSE_WRITE | FAN_CLOSE_NOWRITE | FAN_ACCESS, "/tmp/fa") == 0 &&
          mark(g, FAN_MARK_ADD, FAN_CREATE, "/tmp/fa") == -EINVAL, "fanotify_mark on a file; EINVAL for an event that needs FID");
    check(take(g) == -EAGAIN, "nothing yet: EAGAIN");
    int fd = open_at("/tmp/fa", O_RDWR, 0);
    sys(SYS_write, fd, (i64)"x", 1, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    int queued = -1;
    sys(SYS_ioctl, g, FIONREAD, (i64)&queued, 0, 0, 0);
    check(queued == 24, "FIONREAD: one event's 24 bytes");
    i64 n = take(g);
    struct event e = events[0];
    check(n == 1 && e.mask == (FAN_OPEN | FAN_MODIFY | FAN_CLOSE_WRITE) && e.len == 24 && e.vers == 3 && e.mlen == 24 && e.pid == me,
          "an open, a write and a close: one event, merged, with the pid");
    check(e.fd >= 0 && streq(named(e.fd), "/tmp/fa"), "its descriptor is open on the file");
    check(take(g) == -EAGAIN, "closing that descriptor makes no event of its own");
    mark(g, FAN_MARK_ADD | FAN_MARK_IGNORED_MASK, FAN_OPEN, "/tmp/fa");
    touch("/tmp/fa");
    n = take(g);
    check(n == 1 && events[0].mask == FAN_CLOSE_NOWRITE, "an ignore mask leaves the open out");
    named(events[0].fd);
    check(mark(g, FAN_MARK_ADD, FAN_OPEN | FAN_EVENT_ON_CHILD, "/tmp/fd") == 0, "a directory's mark with FAN_EVENT_ON_CHILD");
    touch("/tmp/fd/c");
    n = take(g);
    check(n == 1 && events[0].mask == FAN_OPEN && streq(named(events[0].fd), "/tmp/fd/c"), "a child's open, with its own descriptor");
    check(sys(SYS_fanotify_mark, g, FAN_MARK_FLUSH, 0, 0, 0, 0) == 0, "FAN_MARK_FLUSH");
    touch("/tmp/fa");
    check(take(g) == -EAGAIN, "and no more events");
    check(mark(g, FAN_MARK_ADD | FAN_MARK_FILESYSTEM, FAN_CLOSE_NOWRITE, "/") == 0, "a whole file system's mark");
    touch("/tmp/fb");
    n = take(g);
    check(n == 1 && events[0].mask == FAN_CLOSE_NOWRITE && streq(named(events[0].fd), "/tmp/fb"), "any file's close on it");
    sys(SYS_fanotify_mark, g, FAN_MARK_FLUSH | FAN_MARK_FILESYSTEM, 0, 0, 0, 0);
    touch("/tmp/fb");
    check(take(g) == -EAGAIN, "flushed too");
    sys(SYS_close, g, 0, 0, 0, 0, 0);

    // statmount and listmount.
    i64 count = listmount(LSMT_ROOT, 0, 0);
    u64 root = 0;
    for (i64 i = 0; i < count; i++)
        if (statmount(ids[i], 0x10, sizeof sm) == 0 && streq(sm_str(108), "/")) root = ids[i];
    check(count >= 1 && root != 0 && ids[0] < ids[count - 1], "listmount: the mounts, in order of ID; one at /");
    check(statmount(root, 0x1 | 0x2 | 0x20 | 0x10, sizeof sm) == 0 && SM_U64(8) == (0x1 | 0x2 | 0x20 | 0x10) && SM_U64(40) == root &&
          SM_U64(24) == 0xef53 && streq(sm_str(36), "ext4"), "statmount of the root: its ID, ext4's magic and type");
    check(statmount(root, 0x2, 100) == -EINVAL && statmount(12345, 0x2, sizeof sm) == -ENOENT, "statmount: EINVAL for a small buffer, ENOENT for no such mount");
    sys(SYS_mkdirat, AT_FDCWD, (i64)"/tmp/m", 0755, 0, 0, 0);
    check(sys(SYS_mount, (i64)"none", (i64)"/tmp/m", (i64)"tmpfs", 0, 0, 0) == 0 && listmount(LSMT_ROOT, 0, 0) == count + 1, "a tmpfs mounted: one more");
    u64 last = ids[count];
    check(statmount(last, 0x1 | 0x2 | 0x10 | 0x20, sizeof sm) == 0 && streq(sm_str(36), "tmpfs") && streq(sm_str(108), "/tmp/m") &&
          SM_U64(24) == 0x01021994 && SM_U64(48) == root, "statmount of it: tmpfs, its mount point, its parent the root");
    check(listmount(LSMT_ROOT, last, 0) == 0 && listmount(LSMT_ROOT, 0, 1) == count + 1 && ids[0] == last, "listmount after the last, and in reverse");

    // remap_file_pages.
    int m = sys(SYS_memfd_create, (i64)"r", 0, 0, 0, 0, 0);
    sys(SYS_ftruncate, m, 8192, 0, 0, 0, 0);
    sys(SYS_pwrite64, m, (i64)"AAAA", 4, 0, 0, 0);
    sys(SYS_pwrite64, m, (i64)"BBBB", 4, 4096, 0, 0);
    volatile char *p = (volatile char *)sys(SYS_mmap, 0, 8192, 3, 1, m, 0);
    check(p[0] == 'A' && sys(SYS_remap_file_pages, (i64)p, 4096, 0, 1, 0, 0) == 0 && p[0] == 'B' && p[4096] == 'B', "remap_file_pages: its first page now the memfd's second");
    volatile char *q = (volatile char *)sys(SYS_mmap, 0, 4096, 3, 0x22, -1, 0);
    check(sys(SYS_remap_file_pages, (i64)p, 4096, 1, 0, 0, 0) == -EINVAL && sys(SYS_remap_file_pages, (i64)q, 4096, 0, 0, 0, 0) == -EINVAL,
          "EINVAL for a prot, and for a private mapping");

#if X86
    char buf[64];
    check(sys(SYS_ustat, 0x803, (i64)buf, 0, 0, 0, 0) == 0 && sys(SYS_ustat, 0xfff0, (i64)buf, 0, 0, 0, 0) == -EINVAL, "ustat of the disk's device; EINVAL for none");
    check(sys(SYS_sysfs, 3, 0, 0, 0, 0, 0) == 5 && sys(SYS_sysfs, 1, (i64)"tmpfs", 0, 0, 0, 0) == 1 &&
          sys(SYS_sysfs, 2, 2, (i64)buf, 0, 0, 0) == 0 && streq(buf, "proc"), "sysfs: how many types, an index, a name");
    check(sys(SYS_modify_ldt, 0, (i64)buf, 64, 0, 0, 0) == 0 && sys(SYS_modify_ldt, 2, (i64)buf, 64, 0, 0, 0) == 40 &&
          sys(SYS_modify_ldt, 1, (i64)buf, 16, 0, 0, 0) == -EINVAL, "modify_ldt: no LDT, the default's zeros, and EINVAL to set one");
    check(sys(SYS_uprobe, 0, 0, 0, 0, 0, 0) == -ENXIO, "uprobe from no trampoline: ENXIO");
    child = fork();
    if (child == 0) {
        sys(SYS_uretprobe, 0, 0, 0, 0, 0, 0);
        quit(0);
    }
    check((wait_status(child) & 0x7f) == 4, "uretprobe from no trampoline: SIGILL");
#endif

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures ? 2 : 0);
}
