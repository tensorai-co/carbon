// File watching and async I/O, for lambda. inotify: a directory's create, open, modify (merged), close-write,
// attrib, moves with a shared cookie, delete, and a directory made and removed; a file's own attrib, move-self,
// delete-self and ignored; a one-shot watch; the checks of inotify_add_watch and rm_watch; EINVAL for a buffer too
// small, FIONREAD, poll, a blocking read woken from another process. Linux AIO: io_setup's checks, writes and reads
// (plain and vectored) completed with their data and iocb, fsync, an eventfd told, a bad descriptor and opcode, a
// request's own error in its result, a full context, a timeout, contexts not inherited, io_pgetevents, io_cancel and
// io_destroy. Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;

#if defined(__aarch64__)
enum { SYS_io_setup = 0, SYS_io_destroy = 1, SYS_io_submit = 2, SYS_io_cancel = 3, SYS_io_getevents = 4,
       SYS_eventfd2 = 19, SYS_inotify_init1 = 26, SYS_inotify_add_watch = 27, SYS_inotify_rm_watch = 28,
       SYS_ioctl = 29, SYS_mkdirat = 34, SYS_unlinkat = 35, SYS_fchmodat = 53, SYS_openat = 56, SYS_close = 57,
       SYS_getdents64 = 61, SYS_read = 63, SYS_write = 64, SYS_ppoll = 73, SYS_exit_group = 94, SYS_nanosleep = 101,
       SYS_clock_gettime = 113, SYS_clone = 220, SYS_wait4 = 260, SYS_renameat2 = 276, SYS_io_pgetevents = 292 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_ioctl = 16, SYS_nanosleep = 35, SYS_clone = 56,
       SYS_wait4 = 61, SYS_io_setup = 206, SYS_io_destroy = 207, SYS_io_getevents = 208, SYS_io_submit = 209,
       SYS_io_cancel = 210, SYS_getdents64 = 217, SYS_clock_gettime = 228, SYS_exit_group = 231,
       SYS_inotify_init = 253, SYS_inotify_add_watch = 254, SYS_inotify_rm_watch = 255, SYS_openat = 257,
       SYS_mkdirat = 258, SYS_unlinkat = 263, SYS_fchmodat = 268, SYS_ppoll = 271, SYS_eventfd2 = 290,
       SYS_inotify_init1 = 294, SYS_renameat2 = 316, SYS_io_pgetevents = 333 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#endif

#define AT_FDCWD (-100)
#define AT_REMOVEDIR 0x200
#define O_WRONLY 1
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define O_NONBLOCK 04000
#define O_CLOEXEC 02000000
#define ENOENT 2
#define EBADF 9
#define EAGAIN 11
#define EEXIST 17
#define ENOTDIR 20
#define EINVAL 22
#define SIGCHLD 17
#define POLLIN 1
#define FIONREAD 0x541b

#define IN_ACCESS 0x1
#define IN_MODIFY 0x2
#define IN_ATTRIB 0x4
#define IN_CLOSE_WRITE 0x8
#define IN_CLOSE_NOWRITE 0x10
#define IN_OPEN 0x20
#define IN_MOVED_FROM 0x40
#define IN_MOVED_TO 0x80
#define IN_CREATE 0x100
#define IN_DELETE 0x200
#define IN_DELETE_SELF 0x400
#define IN_MOVE_SELF 0x800
#define IN_ALL_EVENTS 0xfff
#define IN_IGNORED 0x8000
#define IN_ONLYDIR 0x01000000
#define IN_MASK_CREATE 0x10000000
#define IN_ISDIR 0x40000000
#define IN_ONESHOT 0x80000000u

#define IOCB_CMD_PREAD 0
#define IOCB_CMD_PWRITE 1
#define IOCB_CMD_FSYNC 2
#define IOCB_CMD_PREADV 7
#define IOCB_CMD_PWRITEV 8
#define IOCB_FLAG_RESFD 1

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
static int named(const char *got, const char *want) { return got && same(got, want, len(want) + 1); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return (status >> 8) & 0xff; }
static void nap(u64 ms) { u64 ts[2] = {0, ms * 1000000}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static u64 now_ns(void) { u64 ts[2]; sys(SYS_clock_gettime, 1, (i64)ts, 0, 0, 0, 0); return ts[0] * 1000000000 + ts[1]; }
static int open_at(const char *path, int flags) { return sys(SYS_openat, AT_FDCWD, (i64)path, flags, 0644, 0, 0); }
static int add_watch(int fd, const char *path, u32 mask) { return sys(SYS_inotify_add_watch, fd, (i64)path, mask, 0, 0, 0); }

// The events read, at most 32 of them.
struct ev { int wd; u32 mask, cookie; const char *name; };
static char evbuf[4096];
static struct ev evs[32];
static int nevs;
static void take_events(int fd) {
    nevs = 0;
    i64 n = sys(SYS_read, fd, (i64)evbuf, sizeof evbuf, 0, 0, 0);
    for (i64 at = 0; n > 0 && at < n && nevs < 32; ) {
        int *head = (int *)(evbuf + at);
        u32 namelen = (u32)head[3];
        evs[nevs].wd = head[0];
        evs[nevs].mask = (u32)head[1];
        evs[nevs].cookie = (u32)head[2];
        evs[nevs].name = namelen ? evbuf + at + 16 : 0;
        nevs++;
        at += 16 + namelen;
    }
}
// Whether the events read are, in order, these: each wd, mask and name (0 for none).
static int events_are(int count, const int *wds, const u32 *masks, const char *const *names) {
    if (nevs != count) return 0;
    for (int i = 0; i < count; i++) {
        if (evs[i].wd != wds[i] || evs[i].mask != masks[i]) return 0;
        if (names[i] ? !named(evs[i].name, names[i]) : evs[i].name != 0) return 0;
    }
    return 1;
}

struct iocb { u64 data; u32 key, rw_flags; u16 opcode; short prio; u32 fd; u64 buf, nbytes; i64 offset; u64 reserved2; u32 flags, resfd; };
struct io_event { u64 data, obj; i64 res, res2; };
static void prep(struct iocb *cb, u16 op, int fd, void *buf, u64 n, i64 off, u64 data) {
    char *p = (char *)cb;
    for (u64 i = 0; i < sizeof *cb; i++) p[i] = 0;
    cb->opcode = op; cb->fd = fd; cb->buf = (u64)buf; cb->nbytes = n; cb->offset = off; cb->data = data;
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[256];

    // inotify.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w/a", 0, 0, 0, 0);
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w/b", 0, 0, 0, 0);
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w", AT_REMOVEDIR, 0, 0, 0);
    sys(SYS_mkdirat, AT_FDCWD, (i64)"/tmp/w", 0755, 0, 0, 0);
    int in = sys(SYS_inotify_init1, O_NONBLOCK | O_CLOEXEC, 0, 0, 0, 0, 0);
    check(in >= 0 && sys(SYS_read, in, (i64)buf, sizeof buf, 0, 0, 0) == -EAGAIN && sys(SYS_inotify_init1, 1, 0, 0, 0, 0, 0) == -EINVAL,
          "inotify_init1: nothing to read yet, EAGAIN; a bad flag EINVAL");
    int wd = add_watch(in, "/tmp/w", IN_ALL_EVENTS);
    int f = open_at("/tmp/w/plain", O_WRONLY | O_CREAT | O_TRUNC);
    sys(SYS_close, f, 0, 0, 0, 0, 0);
    check(wd == 1 && add_watch(in, "/tmp/w", IN_ALL_EVENTS) == wd && add_watch(in, "/tmp/w", IN_ALL_EVENTS | IN_MASK_CREATE) == -EEXIST &&
          add_watch(in, "/tmp/w", 0) == -EINVAL && add_watch(in, "/tmp/w/nothing", IN_ALL_EVENTS) == -ENOENT &&
          add_watch(in, "/tmp/w/plain", IN_ALL_EVENTS | IN_ONLYDIR) == -ENOTDIR && add_watch(1, "/tmp/w", IN_ALL_EVENTS) == -EINVAL,
          "inotify_add_watch: a watch, the same again, IN_MASK_CREATE, no events, no file, IN_ONLYDIR, not an inotify");
    take_events(in);
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w/plain", 0, 0, 0, 0);
    take_events(in);
    check(nevs == 1 && evs[0].mask == IN_DELETE && named(evs[0].name, "plain"), "unlink: IN_DELETE with the name");

    f = open_at("/tmp/w/a", O_WRONLY | O_CREAT | O_TRUNC);
    sys(SYS_write, f, (i64)"hi", 2, 0, 0, 0);
    sys(SYS_write, f, (i64)"!!", 2, 0, 0, 0);
    sys(SYS_close, f, 0, 0, 0, 0, 0);
    i64 waiting = 0;
    sys(SYS_ioctl, in, FIONREAD, (i64)&waiting, 0, 0, 0);
    struct { int fd; short events, revents; } pfd = {in, POLLIN, 0};
    u64 zero[2] = {0, 0};
    check(waiting == 4 * 32 && sys(SYS_ppoll, (i64)&pfd, 1, (i64)zero, 0, 8, 0) == 1 && pfd.revents == POLLIN, "FIONREAD: four events, 32 bytes each; poll: readable");
    check(sys(SYS_read, in, (i64)buf, 20, 0, 0, 0) == -EINVAL, "a buffer too small for the first event: EINVAL");
    take_events(in);
    {
        int w[] = {wd, wd, wd, wd};
        u32 m[] = {IN_CREATE, IN_OPEN, IN_MODIFY, IN_CLOSE_WRITE};
        const char *n[] = {"a", "a", "a", "a"};
        check(events_are(4, w, m, n), "create, open, write twice (one IN_MODIFY, merged), close: each with its name");
    }
    int wf = add_watch(in, "/tmp/w/a", IN_ATTRIB | IN_MOVE_SELF | IN_DELETE_SELF | IN_CLOSE_NOWRITE | IN_ACCESS);
    sys(SYS_fchmodat, AT_FDCWD, (i64)"/tmp/w/a", 0600, 0, 0, 0);
    take_events(in);
    {
        int w[] = {wf, wd};
        u32 m[] = {IN_ATTRIB, IN_ATTRIB};
        const char *n[] = {0, "a"};
        check(wf == 2 && events_are(2, w, m, n), "chmod: IN_ATTRIB to the file's own watch, and to its directory's with its name");
    }
    f = open_at("/tmp/w/a", 0);
    sys(SYS_read, f, (i64)buf, sizeof buf, 0, 0, 0);
    sys(SYS_close, f, 0, 0, 0, 0, 0);
    take_events(in);
    {
        int w[] = {wd, wf, wd, wf, wd};
        u32 m[] = {IN_OPEN, IN_ACCESS, IN_ACCESS, IN_CLOSE_NOWRITE, IN_CLOSE_NOWRITE};
        const char *n[] = {"a", 0, "a", 0, "a"};
        check(events_are(5, w, m, n), "open, read and close for reading, to each watch that wants them");
    }
    sys(SYS_renameat2, AT_FDCWD, (i64)"/tmp/w/a", AT_FDCWD, (i64)"/tmp/w/b", 0, 0);
    take_events(in);
    check(nevs == 3 && evs[0].mask == IN_MOVED_FROM && named(evs[0].name, "a") && evs[1].mask == IN_MOVED_TO && named(evs[1].name, "b") &&
          evs[0].cookie != 0 && evs[0].cookie == evs[1].cookie && evs[2].wd == wf && evs[2].mask == IN_MOVE_SELF,
          "rename: IN_MOVED_FROM and IN_MOVED_TO sharing a cookie, IN_MOVE_SELF");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w/b", 0, 0, 0, 0);
    take_events(in);
    {
        int w[] = {wf, wd, wf, wf};
        u32 m[] = {IN_ATTRIB, IN_DELETE, IN_DELETE_SELF, IN_IGNORED};
        const char *n[] = {0, "b", 0, 0};
        check(events_are(4, w, m, n), "unlink: IN_ATTRIB, IN_DELETE, IN_DELETE_SELF, and the file's watch IN_IGNORED");
    }
    sys(SYS_mkdirat, AT_FDCWD, (i64)"/tmp/w/sub", 0755, 0, 0, 0);
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/w/sub", AT_REMOVEDIR, 0, 0, 0);
    take_events(in);
    check(nevs == 2 && evs[0].mask == (IN_CREATE | IN_ISDIR) && named(evs[0].name, "sub") && evs[1].mask == (IN_DELETE | IN_ISDIR),
          "mkdir and rmdir: IN_ISDIR");
    f = open_at("/tmp/w/once", O_RDWR | O_CREAT | O_TRUNC);
    int w1 = add_watch(in, "/tmp/w/once", IN_MODIFY | IN_ONESHOT);
    take_events(in);
    sys(SYS_write, f, (i64)"1", 1, 0, 0, 0);
    sys(SYS_write, f, (i64)"2", 1, 0, 0, 0);
    sys(SYS_close, f, 0, 0, 0, 0, 0);
    take_events(in);
    int one_shot = 0, ignored = 0;
    for (int i = 0; i < nevs; i++) {
        if (evs[i].wd == w1 && evs[i].mask == IN_MODIFY) one_shot++;
        if (evs[i].wd == w1 && evs[i].mask == IN_IGNORED) ignored++;
    }
    check(one_shot == 1 && ignored == 1, "IN_ONESHOT: one event, then IN_IGNORED");
    check(sys(SYS_inotify_rm_watch, in, wd, 0, 0, 0, 0) == 0 && sys(SYS_inotify_rm_watch, in, wd, 0, 0, 0, 0) == -EINVAL, "inotify_rm_watch, and again: EINVAL");
    take_events(in);
    check(nevs == 1 && evs[0].wd == wd && evs[0].mask == IN_IGNORED, "a removed watch: IN_IGNORED");
    int blocking = sys(SYS_inotify_init1, 0, 0, 0, 0, 0, 0);
    add_watch(blocking, "/tmp/w", IN_CREATE);
    i64 child = fork();
    if (child == 0) {
        take_events(blocking);
        quit(nevs == 1 && evs[0].mask == IN_CREATE && named(evs[0].name, "late") ? 0 : 1);
    }
    nap(30);
    sys(SYS_close, open_at("/tmp/w/late", O_WRONLY | O_CREAT), 0, 0, 0, 0, 0);
    check(reap(child) == 0, "a blocking read in another process, woken by a create");
#if !defined(__aarch64__)
    int old = sys(SYS_inotify_init, 0, 0, 0, 0, 0, 0);
    check(old >= 0 && add_watch(old, "/tmp", IN_CREATE) == 1, "x86-64's inotify_init");
#endif

    // Linux AIO.
    u64 ctx = 0, other = 5;
    check(sys(SYS_io_setup, 4, (i64)&ctx, 0, 0, 0, 0) == 0 && ctx != 0 && sys(SYS_io_setup, 4, (i64)&other, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_io_setup, 0, (i64)&other, 0, 0, 0, 0) == -EINVAL, "io_setup; a context that isn't 0, or no events: EINVAL");
    int file = open_at("/tmp/aio", O_RDWR | O_CREAT | O_TRUNC);
    char got[8] = {0};
    struct iocb w, r, *two[2] = {&w, &r};
    prep(&w, IOCB_CMD_PWRITE, file, "hello aio", 9, 0, 11);
    prep(&r, IOCB_CMD_PREAD, file, got, 3, 6, 22);
    struct io_event events[8];
    check(sys(SYS_io_submit, ctx, 2, (i64)two, 0, 0, 0) == 2 && sys(SYS_io_getevents, ctx, 2, 8, (i64)events, 0, 0) == 2 &&
          events[0].data == 11 && events[0].obj == (u64)&w && events[0].res == 9 && events[1].data == 22 && events[1].obj == (u64)&r &&
          events[1].res == 3 && same(got, "aio", 3), "a write and a read: done in order, each event with its data, iocb and count");
    char a1[4] = {0}, a2[4] = {0};
    struct { void *base; u64 len; } out[2] = {{"ab", 2}, {"cd", 2}}, back[2] = {{a1, 3}, {a2, 3}};
    struct iocb wv, rv, *vs[2] = {&wv, &rv};
    prep(&wv, IOCB_CMD_PWRITEV, file, out, 2, 20, 1);
    prep(&rv, IOCB_CMD_PREADV, file, back, 2, 19, 2);
    check(sys(SYS_io_submit, ctx, 2, (i64)vs, 0, 0, 0) == 2 && sys(SYS_io_getevents, ctx, 2, 2, (i64)events, 0, 0) == 2 &&
          events[0].res == 4 && events[1].res == 5 && a1[0] == 0 && same(a1 + 1, "ab", 2) && same(a2, "cd", 2), "vectored: PWRITEV and PREADV");
    int efd = sys(SYS_eventfd2, 0, 0, 0, 0, 0, 0);
    struct iocb s, *one[1] = {&s};
    prep(&s, IOCB_CMD_FSYNC, file, 0, 0, 0, 3);
    s.flags = IOCB_FLAG_RESFD;
    s.resfd = efd;
    u64 count = 0;
    check(sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0) == 1 && sys(SYS_read, efd, (i64)&count, 8, 0, 0, 0) == 8 && count == 1 &&
          sys(SYS_io_getevents, ctx, 1, 1, (i64)events, 0, 0) == 1 && events[0].res == 0, "fsync, with an eventfd told");
    int wronly = open_at("/tmp/aio", O_WRONLY);
    struct iocb bad, *pair[2] = {&r, &bad};
    prep(&r, IOCB_CMD_PREAD, wronly, got, 3, 0, 4);
    prep(&bad, IOCB_CMD_PREAD, 999, got, 3, 0, 5);
    check(sys(SYS_io_submit, ctx, 2, (i64)pair, 0, 0, 0) == 1 && sys(SYS_io_getevents, ctx, 1, 8, (i64)events, 0, 0) == 1 && events[0].res == -EBADF,
          "a request's own error is its result; a bad descriptor after it: only the first submitted");
    one[0] = &bad;
    check(sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0) == -EBADF, "a bad descriptor first: io_submit's EBADF");
    prep(&bad, 6, file, 0, 0, 0, 0);
    check(sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0) == -EINVAL, "an opcode that isn't: EINVAL");
    struct iocb fill[5], *fills[5];
    for (int i = 0; i < 5; i++) { prep(&fill[i], IOCB_CMD_PREAD, file, got, 1, 0, 100 + i); fills[i] = &fill[i]; }
    check(sys(SYS_io_submit, ctx, 5, (i64)fills, 0, 0, 0) == 4 && sys(SYS_io_submit, ctx, 1, (i64)fills, 0, 0, 0) == -EAGAIN,
          "a context holds as many as io_setup said: then EAGAIN");
    check(sys(SYS_io_getevents, ctx, 3, 2, (i64)events, 0, 0) == -EINVAL && sys(SYS_io_getevents, ctx, 0, 8, (i64)events, 0, 0) == 4 &&
          events[3].data == 103, "io_getevents: fewer wanted than the least, EINVAL; all four taken");
    u64 ts[2] = {0, 20000000};
    u64 before = now_ns();
    check(sys(SYS_io_getevents, ctx, 1, 1, (i64)events, (i64)ts, 0) == 0 && now_ns() - before >= 15000000, "nothing done: 0 after its 20 ms");
    struct { u64 mask, size; } sig = {0, 8};
    prep(&r, IOCB_CMD_PREAD, file, got, 1, 0, 7);
    one[0] = &r;
    sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0);
    check(sys(SYS_io_pgetevents, ctx, 1, 1, (i64)events, 0, (i64)&sig) == 1 && events[0].data == 7 && sys(SYS_io_cancel, ctx, (i64)&r, (i64)events, 0, 0, 0) == -EINVAL,
          "io_pgetevents; io_cancel finds nothing in flight");
    child = fork();
    if (child == 0) quit(sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0) == -EINVAL ? 0 : 1);
    check(reap(child) == 0, "a child doesn't have its parent's context");
    check(sys(SYS_io_destroy, ctx, 0, 0, 0, 0, 0) == 0 && sys(SYS_io_destroy, ctx, 0, 0, 0, 0, 0) == -EINVAL && sys(SYS_io_submit, ctx, 1, (i64)one, 0, 0, 0) == -EINVAL,
          "io_destroy, and the context's gone");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
