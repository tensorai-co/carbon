// Moving data, for lambda: preadv/pwritev and their 2 forms (RWF_APPEND, the -1 offset, unknown flags), ESPIPE on a
// pipe; sendfile, copy_file_range, splice, tee and vmsplice, with their offsets and errors, and a big copy into a pipe
// that moves only what fits; truncate and fallocate (growing, KEEP_SIZE, punching a hole, zeroing a range); sync,
// syncfs, sync_file_range and readahead; ioprio_set and ioprio_get; fchmodat2; file handles refused; and x86-64's
// utime, utimes, futimesat and getdents. Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_ioprio_set = 30, SYS_ioprio_get = 31, SYS_mkdirat = 34, SYS_unlinkat = 35, SYS_symlinkat = 36,
       SYS_truncate = 45, SYS_fallocate = 47, SYS_openat = 56, SYS_close = 57, SYS_pipe2 = 59, SYS_lseek = 62,
       SYS_read = 63, SYS_write = 64, SYS_pread64 = 67, SYS_pwrite64 = 68, SYS_preadv = 69, SYS_pwritev = 70,
       SYS_sendfile = 71, SYS_vmsplice = 75, SYS_splice = 76, SYS_tee = 77, SYS_newfstatat = 79, SYS_sync = 81,
       SYS_sync_file_range = 84, SYS_exit_group = 94, SYS_readahead = 213, SYS_name_to_handle_at = 264,
       SYS_open_by_handle_at = 265, SYS_syncfs = 267, SYS_copy_file_range = 285, SYS_preadv2 = 286,
       SYS_pwritev2 = 287, SYS_fchmodat2 = 452 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_lseek = 8, SYS_pread64 = 17, SYS_pwrite64 = 18,
       SYS_sendfile = 40, SYS_truncate = 76, SYS_getdents = 78, SYS_utime = 132, SYS_sync = 162,
       SYS_readahead = 187, SYS_exit_group = 231, SYS_utimes = 235, SYS_ioprio_set = 251, SYS_ioprio_get = 252,
       SYS_openat = 257, SYS_mkdirat = 258, SYS_futimesat = 261, SYS_newfstatat = 262, SYS_unlinkat = 263,
       SYS_symlinkat = 266, SYS_splice = 275, SYS_tee = 276, SYS_sync_file_range = 277, SYS_vmsplice = 278,
       SYS_fallocate = 285, SYS_pipe2 = 293, SYS_preadv = 295, SYS_pwritev = 296, SYS_name_to_handle_at = 303,
       SYS_open_by_handle_at = 304, SYS_syncfs = 306, SYS_copy_file_range = 326, SYS_preadv2 = 327,
       SYS_pwritev2 = 328, SYS_fchmodat2 = 452 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
#endif
#define ST_SIZE 48
#define ST_MTIME 88

#define AT_FDCWD (-100)
#define O_RDONLY 0
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define O_DIRECTORY_X86 0200000
#define AT_SYMLINK_NOFOLLOW 0x100
#define EPERM 1
#define ENOENT 2
#define EAGAIN 11
#define EISDIR 21
#define EINVAL 22
#define ESPIPE 29
#define EOPNOTSUPP 95
#define ESTALE 116
#define RWF_APPEND 0x10

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
static i64 stat_of(const char *path, u64 *st) { return sys(SYS_newfstatat, AT_FDCWD, (i64)path, (i64)st, 0, 0, 0); }
static i64 size_of(const char *path) { u64 st[32]; return stat_of(path, st) == 0 ? *(i64 *)((char *)st + ST_SIZE) : -1; }
static i64 make(const char *path, const char *text) {
    int fd = sys(SYS_openat, AT_FDCWD, (i64)path, O_RDWR | O_CREAT | O_TRUNC, 0644, 0, 0);
    sys(SYS_write, fd, (i64)text, len(text), 0, 0, 0);
    return fd;
}
static i64 here(int fd) { return sys(SYS_lseek, fd, 0, 1, 0, 0, 0); }

static char big[200000];

void cmain(i64 *sp) {
    (void)sp;
    u64 st[32];
    char buf[256], other[64];
    int p[2], q[2];

    // preadv and pwritev: at an offset, leaving the file's own alone.
    int fd = make("/tmp/v", "0123456789");
    struct { char *base; u64 len; } iov[2] = {{buf, 3}, {buf + 3, 4}};
    sys(SYS_lseek, fd, 2, 0, 0, 0, 0);
    i64 n = sys(SYS_preadv, fd, (i64)iov, 2, 1, 0, 0);
    check(n == 7 && same(buf, "1234567", 7) && here(fd) == 2, "preadv: two buffers from offset 1, the file's offset unmoved");
    iov[0].base = "ab"; iov[0].len = 2; iov[1].base = "cd"; iov[1].len = 2;
    n = sys(SYS_pwritev, fd, (i64)iov, 2, 6, 0, 0);
    check(n == 4 && sys(SYS_pread64, fd, (i64)buf, 10, 0, 0, 0) == 10 && same(buf, "012345abcd", 10) && here(fd) == 2, "pwritev: two buffers at offset 6");
    // preadv2 at -1 reads from, and moves, the file's own offset.
    iov[0].base = buf; iov[0].len = 3;
    n = sys(SYS_preadv2, fd, (i64)iov, 1, -1, 0, 0);
    check(n == 3 && same(buf, "234", 3) && here(fd) == 5, "preadv2 with -1: at the file's own offset, which moves");
    iov[0].base = "!"; iov[0].len = 1;
    n = sys(SYS_pwritev2, fd, (i64)iov, 1, 0, 0, RWF_APPEND);
    check(n == 1 && size_of("/tmp/v") == 11 && sys(SYS_pread64, fd, (i64)buf, 1, 10, 0, 0) == 1 && buf[0] == '!', "pwritev2 with RWF_APPEND: at the end, whatever the offset");
    check(sys(SYS_preadv2, fd, (i64)iov, 1, 0, 0, 0x10000) == -EOPNOTSUPP, "preadv2: an unknown flag is EOPNOTSUPP");
    check(sys(SYS_preadv, fd, (i64)iov, 1, -5, 0, 0) == -EINVAL, "preadv: a negative offset is EINVAL");
    sys(SYS_pipe2, (i64)p, 0, 0, 0, 0, 0);
    check(sys(SYS_preadv, p[0], (i64)iov, 1, 0, 0, 0) == -ESPIPE && sys(SYS_pread64, p[0], (i64)buf, 1, 0, 0, 0) == -ESPIPE &&
          sys(SYS_pwrite64, p[1], (i64)"x", 1, 0, 0, 0) == -ESPIPE, "preadv, pread64 and pwrite64 on a pipe: ESPIPE");
    iov[0].base = "via2"; iov[0].len = 4;
    check(sys(SYS_pwritev2, p[1], (i64)iov, 1, -1, 0, 0) == 4 && sys(SYS_read, p[0], (i64)buf, 4, 0, 0, 0) == 4 && same(buf, "via2", 4),
          "pwritev2 with -1 on a pipe: an ordinary write");
    iov[0].base = buf; iov[0].len = 4;
    check(sys(SYS_preadv2, p[0], (i64)iov, 1, -1, 0, 8 /* RWF_NOWAIT */) == -EAGAIN, "preadv2 with RWF_NOWAIT on an empty pipe: EAGAIN");

    // sendfile: from an offset given (and moved on), or from the file's own.
    i64 off = 2;
    n = sys(SYS_sendfile, p[1], fd, (i64)&off, 4, 0, 0);
    i64 got = sys(SYS_read, p[0], (i64)buf, sizeof buf, 0, 0, 0);
    check(n == 4 && off == 6 && here(fd) == 5 && got == 4 && same(buf, "2345", 4), "sendfile with an offset: moved on, the file's own unmoved");
    n = sys(SYS_sendfile, p[1], fd, 0, 3, 0, 0);
    got = sys(SYS_read, p[0], (i64)buf, sizeof buf, 0, 0, 0);
    check(n == 3 && here(fd) == 8 && got == 3 && same(buf, "5ab", 3), "sendfile without one: from the file's own offset, which moves");
    check(sys(SYS_sendfile, p[1], p[0], (i64)&off, 1, 0, 0) == -ESPIPE, "sendfile from a pipe at an offset: ESPIPE");

    // copy_file_range between files, and within one.
    int out = make("/tmp/c", "..........");
    i64 in_off = 0, out_off = 3;
    n = sys(SYS_copy_file_range, fd, (i64)&in_off, out, (i64)&out_off, 4, 0);
    check(n == 4 && in_off == 4 && out_off == 7 && sys(SYS_pread64, out, (i64)buf, 10, 0, 0, 0) == 10 && same(buf, "...0123...", 10),
          "copy_file_range: at both offsets, each moved on");
    in_off = 0; out_off = 2;
    check(sys(SYS_copy_file_range, out, (i64)&in_off, out, (i64)&out_off, 4, 0) == -EINVAL, "copy_file_range within a file, overlapping: EINVAL");
    check(sys(SYS_copy_file_range, fd, 0, out, 0, 1, 1) == -EINVAL && sys(SYS_copy_file_range, p[0], 0, out, 0, 1, 0) == -EINVAL,
          "copy_file_range: flags, or a pipe, EINVAL");
    in_off = 100;
    check(sys(SYS_copy_file_range, fd, (i64)&in_off, out, 0, 4, 0) == 0, "copy_file_range past the end: 0");

    // splice: file to pipe, pipe to file.
    in_off = 6;
    n = sys(SYS_splice, fd, (i64)&in_off, p[1], 0, 4, 0);
    check(n == 4 && in_off == 10 && here(fd) == 8, "splice from a file at an offset into a pipe");
    out_off = 0;
    n = sys(SYS_splice, p[0], 0, out, (i64)&out_off, 64, 0);
    check(n == 4 && out_off == 4 && sys(SYS_pread64, out, (i64)buf, 10, 0, 0, 0) == 10 && same(buf, "abcd123...", 10), "splice from the pipe into a file at an offset");
    check(sys(SYS_splice, p[0], (i64)&off, out, 0, 1, 0) == -ESPIPE, "splice: an offset for the pipe's end is ESPIPE");
    check(sys(SYS_splice, fd, 0, out, 0, 1, 0) == -EINVAL, "splice with no pipe: EINVAL");
    check(sys(SYS_splice, p[0], 0, out, 0, 1, 2 /* SPLICE_F_NONBLOCK */) == -EAGAIN, "splice from an empty pipe, SPLICE_F_NONBLOCK: EAGAIN");

    // A big copy into a pipe moves what fits, rather than waiting on a reader that's this very program.
    for (int i = 0; i < (int)sizeof big; i++) big[i] = (char)('a' + i % 26);
    int bigfd = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/big", O_RDWR | O_CREAT | O_TRUNC, 0644, 0, 0);
    sys(SYS_write, bigfd, (i64)big, sizeof big, 0, 0, 0);
    in_off = 0;
    n = sys(SYS_sendfile, p[1], bigfd, (i64)&in_off, sizeof big, 0, 0);
    i64 drained = 0;
    while (drained < n) drained += sys(SYS_read, p[0], (i64)buf, sizeof buf, 0, 0, 0);
    check(n > 0 && n <= 65536 && in_off == n && drained == n, "sendfile of 200 KB into a pipe: what fits, no more");
    // And a big copy between files goes all the way, a piece at a time.
    int copy = make("/tmp/big2", "");
    n = sys(SYS_copy_file_range, bigfd, (i64)&(i64){0}, copy, 0, sizeof big, 0);
    check(n == sizeof big && size_of("/tmp/big2") == sizeof big && sys(SYS_pread64, copy, (i64)buf, 26, 199974, 0, 0) == 26 && same(buf, big + 199974, 26),
          "copy_file_range of 200 KB: all of it");

    // tee leaves the data where it was; vmsplice moves it between a pipe and memory.
    sys(SYS_pipe2, (i64)q, 0, 0, 0, 0, 0);
    sys(SYS_write, p[1], (i64)"teed", 4, 0, 0, 0);
    n = sys(SYS_tee, p[0], q[1], 64, 0, 0, 0);
    i64 a = sys(SYS_read, q[0], (i64)buf, 64, 0, 0, 0), b = sys(SYS_read, p[0], (i64)other, 64, 0, 0, 0);
    check(n == 4 && a == 4 && b == 4 && same(buf, "teed", 4) && same(other, "teed", 4), "tee: copied, and still in the first pipe");
    check(sys(SYS_tee, p[0], p[1], 4, 0, 0, 0) == -EINVAL && sys(SYS_tee, fd, q[1], 4, 0, 0, 0) == -EINVAL, "tee: the same pipe, or a file, EINVAL");
    check(sys(SYS_tee, p[0], q[1], 4, 2, 0, 0) == -EAGAIN, "tee from an empty pipe, SPLICE_F_NONBLOCK: EAGAIN");
    iov[0].base = "vm"; iov[0].len = 2; iov[1].base = "splice"; iov[1].len = 6;
    n = sys(SYS_vmsplice, p[1], (i64)iov, 2, 0, 0, 0);
    iov[0].base = buf; iov[0].len = 3; iov[1].base = buf + 3; iov[1].len = 5;
    a = sys(SYS_vmsplice, p[0], (i64)iov, 2, 0, 0, 0);
    check(n == 8 && a == 8 && same(buf, "vmsplice", 8), "vmsplice: into a pipe, and out of it");
    check(sys(SYS_vmsplice, fd, (i64)iov, 1, 0, 0, 0) == -9, "vmsplice on a file: EBADF");

    // truncate by name.
    check(sys(SYS_truncate, (i64)"/tmp/v", 4, 0, 0, 0, 0) == 0 && size_of("/tmp/v") == 4, "truncate: shorter");
    check(sys(SYS_truncate, (i64)"/tmp/v", 9, 0, 0, 0, 0) == 0 && size_of("/tmp/v") == 9 && sys(SYS_pread64, fd, (i64)buf, 9, 0, 0, 0) == 9 &&
          same(buf, "0123\0\0\0\0\0", 9), "truncate: longer, with zeroes");
    check(sys(SYS_truncate, (i64)"/tmp", 0, 0, 0, 0, 0) == -EISDIR && sys(SYS_truncate, (i64)"/tmp/v", -1, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_truncate, (i64)"/tmp/none", 0, 0, 0, 0, 0) == -ENOENT, "truncate: a directory, a negative size, a missing file");

    // fallocate: room (the size), KEEP_SIZE, a hole punched, a range zeroed.
    int fa = make("/tmp/f", "ABCDEFGHIJ");
    check(sys(SYS_fallocate, fa, 0, 0, 20, 0, 0) == 0 && size_of("/tmp/f") == 20, "fallocate: the size grows to the range's end");
    check(sys(SYS_fallocate, fa, 1, 0, 100, 0, 0) == 0 && size_of("/tmp/f") == 20, "fallocate with KEEP_SIZE: the size stays");
    check(sys(SYS_fallocate, fa, 3, 2, 3, 0, 0) == 0 && size_of("/tmp/f") == 20 && sys(SYS_pread64, fa, (i64)buf, 10, 0, 0, 0) == 10 &&
          same(buf, "AB\0\0\0FGHIJ", 10), "fallocate PUNCH_HOLE|KEEP_SIZE: zeroes, same size");
    check(sys(SYS_fallocate, fa, 0x10, 8, 30, 0, 0) == 0 && size_of("/tmp/f") == 38 && sys(SYS_pread64, fa, (i64)buf, 10, 0, 0, 0) == 10 &&
          same(buf, "AB\0\0\0FGH\0\0", 10), "fallocate ZERO_RANGE: zeroes, and the size grows");
    check(sys(SYS_fallocate, fa, 2, 0, 1, 0, 0) == -EOPNOTSUPP && sys(SYS_fallocate, fa, 0, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_fallocate, p[1], 0, 0, 1, 0, 0) == -ESPIPE, "fallocate: PUNCH_HOLE alone, a zero length, a pipe");

    // Syncing, and advice.
    check(sys(SYS_sync, 0, 0, 0, 0, 0, 0) == 0 && sys(SYS_syncfs, fa, 0, 0, 0, 0, 0) == 0 && sys(SYS_syncfs, 999, 0, 0, 0, 0, 0) == -9, "sync and syncfs");
    check(sys(SYS_sync_file_range, fa, 0, 0, 7, 0, 0) == 0 && sys(SYS_sync_file_range, fa, 0, 0, 8, 0, 0) == -EINVAL &&
          sys(SYS_sync_file_range, p[1], 0, 0, 1, 0, 0) == -ESPIPE, "sync_file_range: every flag, a bad one, a pipe");
    check(sys(SYS_readahead, fa, 0, 4096, 0, 0, 0) == 0 && sys(SYS_readahead, p[0], 0, 1, 0, 0, 0) == -EINVAL, "readahead: a file, and a pipe");

    // I/O priorities: kept and read back.
    check(sys(SYS_ioprio_get, 1, 0, 0, 0, 0, 0) == 0, "ioprio_get: none set yet");
    check(sys(SYS_ioprio_set, 1, 0, (2 << 13) | 3, 0, 0, 0) == 0 && sys(SYS_ioprio_get, 1, 0, 0, 0, 0, 0) == ((2 << 13) | 3),
          "ioprio_set: best effort, level 3, read back");
    check(sys(SYS_ioprio_set, 1, 0, (2 << 13) | 9, 0, 0, 0) == -EINVAL && sys(SYS_ioprio_set, 9, 0, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_ioprio_get, 1, 99999, 0, 0, 0, 0) == -3, "ioprio: a level too high, a bad which, no such process");

    // fchmodat2: fchmodat with flags; a link's own mode can't change.
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/l", 0, 0, 0, 0);
    sys(SYS_symlinkat, (i64)"/tmp/f", AT_FDCWD, (i64)"/tmp/l", 0, 0, 0);
    check(sys(SYS_fchmodat2, AT_FDCWD, (i64)"/tmp/f", 0600, AT_SYMLINK_NOFOLLOW, 0, 0) == 0 && stat_of("/tmp/f", st) == 0 &&
          (*(u32 *)((char *)st + ST_MODE) & 07777) == 0600, "fchmodat2 with AT_SYMLINK_NOFOLLOW on a file");
    check(sys(SYS_fchmodat2, AT_FDCWD, (i64)"/tmp/l", 0600, AT_SYMLINK_NOFOLLOW, 0, 0) == -EOPNOTSUPP &&
          sys(SYS_fchmodat2, AT_FDCWD, (i64)"/tmp/f", 0600, 1, 0, 0) == -EINVAL, "fchmodat2: on a link EOPNOTSUPP, a bad flag EINVAL");

    // File handles: none to give, so none to open.
    u32 handle[34] = {128};
    int mount_id;
    check(sys(SYS_name_to_handle_at, AT_FDCWD, (i64)"/tmp/f", (i64)handle, (i64)&mount_id, 0, 0) == -EOPNOTSUPP &&
          sys(SYS_name_to_handle_at, AT_FDCWD, (i64)"/tmp/none", (i64)handle, (i64)&mount_id, 0, 0) == -ENOENT, "name_to_handle_at: EOPNOTSUPP, or ENOENT");
    handle[0] = 8;
    check(sys(SYS_open_by_handle_at, AT_FDCWD, (i64)handle, O_RDONLY, 0, 0, 0) == -ESTALE, "open_by_handle_at: ESTALE");

#if !defined(__aarch64__)
    // x86-64's old forms.
    i64 utimbuf[2] = {5, 1000000};
    check(sys(SYS_utime, (i64)"/tmp/f", (i64)utimbuf, 0, 0, 0, 0) == 0 && stat_of("/tmp/f", st) == 0 && *(i64 *)((char *)st + ST_MTIME) == 1000000,
          "utime: the modification time, in seconds");
    i64 tv[4] = {0, 0, 2000000, 250000};
    check(sys(SYS_utimes, (i64)"/tmp/f", (i64)tv, 0, 0, 0, 0) == 0 && stat_of("/tmp/f", st) == 0 && *(i64 *)((char *)st + ST_MTIME) == 2000000 &&
          *(i64 *)((char *)st + ST_MTIME + 8) == 250000000, "utimes: seconds and microseconds");
    tv[2] = 3000000; tv[3] = 0;
    int tmp = sys(SYS_openat, AT_FDCWD, (i64)"/tmp", O_RDONLY | O_DIRECTORY_X86, 0, 0, 0);
    check(sys(SYS_futimesat, tmp, (i64)"f", (i64)tv, 0, 0, 0) == 0 && stat_of("/tmp/f", st) == 0 && *(i64 *)((char *)st + ST_MTIME) == 3000000,
          "futimesat: from a directory");
    tv[3] = 1000000;
    check(sys(SYS_utimes, (i64)"/tmp/f", (i64)tv, 0, 0, 0, 0) == -EINVAL, "utimes: a million microseconds is EINVAL");
    // getdents: the name, then the type in the record's last byte.
    sys(SYS_mkdirat, AT_FDCWD, (i64)"/tmp/d", 0755, 0, 0, 0);
    sys(SYS_close, make("/tmp/d/entry", "x"), 0, 0, 0, 0, 0);
    int d = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/d", O_RDONLY | O_DIRECTORY_X86, 0, 0, 0);
    n = sys(SYS_getdents, d, (i64)buf, sizeof buf, 0, 0, 0);
    int found = 0;
    for (i64 at = 0; at < n;) {
        unsigned short reclen = *(unsigned short *)(buf + at + 16);
        if (same(buf + at + 18, "entry", 6) && buf[at + reclen - 1] == 8 /* DT_REG */) found = 1;
        if (reclen == 0) break;
        at += reclen;
    }
    check(n > 0 && found, "getdents: the old layout, its type last");
#endif

    say(failures ? "some checks failed\n" : "all passed\n");
    sys(SYS_exit_group, failures, 0, 0, 0, 0, 0);
}
